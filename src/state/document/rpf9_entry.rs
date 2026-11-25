//! Opening and extracting an entry through its table, with a reloadable archive/index address.

use super::*;
use crate::state::{
    AppState, DocPayload, DocumentKind, LoadResult, LoadingState, RpfEntryBinding, RpfNodeBacking,
};
use or_io::container::rpf::{Gta6EntryContent, Gta6EntryExtra, Gta6EntryRead};

const MARKER: &str = ".rpf9-table";
const LIMIT: u64 = 1 << 30;
pub const READ_ONLY: &str = tr_key!(
    "This file was opened from an RPF9 table and is read-only. Extract it to save a separate copy."
);

#[derive(Clone)]
pub struct EntrySource {
    pub path: PathBuf,
    pub name: String,
    archive: PathBuf,
    metadata: Metadata,
    keys: Arc<GtaViKeys>,
    read: Gta6EntryRead,
    entry: Gta6Entry,
    extra: Gta6EntryExtra,
    tree: Arc<Mutex<Tree>>,
    key: (u64, u32),
}

impl Rpf9TableDocument {
    pub fn entry(&self, base: u64, index: u32) -> Result<EntrySource, String> {
        let tree = self.tree();
        let node = tree
            .nodes
            .get(&base)
            .ok_or_else(|| tr!("The archive table could not be loaded.").to_string())?;
        let read = Gta6EntryRead::from_table(&node.archive, node.extent.clone(), index, u64::MAX)
            .map_err(|e| e.to_string())?;
        let entry = &node.archive.entries[index as usize];
        let extra = node
            .archive
            .entry_extra(index as usize)
            .ok_or_else(|| tr!("The archive table could not be loaded.").to_string())?;
        let name = crate::state::files_view::rpf9_display_name(
            &file_name(entry),
            tree.entry_type(base, index).0,
        )
        .into_owned();
        let indices = node
            .chain
            .iter()
            .chain(std::iter::once(&index))
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join("_");
        Ok(EntrySource {
            path: self.path.join(MARKER).join(indices).join(&name),
            name,
            archive: self.path.clone(),
            metadata: self.metadata.clone(),
            keys: self.keys.clone(),
            read,
            entry: entry.clone(),
            extra,
            tree: Arc::clone(&self.tree),
            key: (base, index),
        })
    }
}

// A recovered name is descriptive data, never a destination path.
fn file_name(entry: &Gta6Entry) -> String {
    let name = entry.display_name();
    let leaf = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let stem = leaf
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix)
                .is_some_and(|n| n.len() == 1 && matches!(n.as_bytes()[0], b'1'..=b'9'))
        });
    if leaf.is_empty()
        || leaf == "."
        || leaf == ".."
        || leaf.ends_with(['.', ' '])
        || leaf
            .chars()
            .any(|c| c.is_control() || "<>:\"|?*".contains(c))
        || reserved
    {
        format!("hash_{:016x}", entry.name64)
    } else {
        leaf.to_string()
    }
}

pub fn is_entry(path: &Path) -> bool {
    path.components().any(|part| part.as_os_str() == MARKER)
}

pub fn archive_path(path: &Path) -> Option<&Path> {
    path.parent()?
        .parent()
        .filter(|p| p.file_name().is_some_and(|n| n == MARKER))?
        .parent()
}

fn address(path: &Path) -> Result<(PathBuf, Vec<u32>), String> {
    let invalid = || tr!("The archive table could not be loaded.").to_string();
    let indices = path.parent().ok_or_else(invalid)?;
    let marker = indices
        .parent()
        .filter(|p| p.file_name().is_some_and(|n| n == MARKER))
        .ok_or_else(invalid)?;
    let archive = marker.parent().ok_or_else(invalid)?.to_path_buf();
    let chain: Vec<u32> = indices
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(invalid)?
        .split('_')
        .map(|n| n.parse().map_err(|_| invalid()))
        .collect::<Result<_, _>>()?;
    if chain.is_empty() || chain.len() > 33 {
        return Err(invalid());
    }
    Ok((archive, chain))
}

pub fn read_path(path: &Path) -> Result<DocPayload, String> {
    let (archive, _) = address(path)?;
    let doc = Rpf9TableDocument::open(&archive)?
        .ok_or_else(|| tr!("The archive table could not be loaded.").to_string())?;
    read_path_with_table(path, &doc)
}

pub fn read_path_with_table(path: &Path, doc: &Rpf9TableDocument) -> Result<DocPayload, String> {
    let (archive, chain) = address(path)?;
    if archive != doc.path {
        return Err(tr!("The archive table could not be loaded.").to_string());
    }
    let base = {
        let tree = doc.tree();
        let parent = &chain[..chain.len() - 1];
        tree.nodes
            .iter()
            .find_map(|(base, node)| (node.chain.as_slice() == parent).then_some(*base))
    };
    let mut source = match base {
        Some(base) => doc.entry(base, chain[chain.len() - 1])?,
        None => read_chain(doc, &chain)?,
    };
    source.path = path.to_path_buf();
    source.payload()
}

fn read_chain(doc: &Rpf9TableDocument, chain: &[u32]) -> Result<EntrySource, String> {
    let mut base = 0;
    let mut file = checked_file(&doc.path, &doc.metadata)?;
    for &index in &chain[..chain.len() - 1] {
        let mut tree = doc.tree();
        let parent = &tree.nodes[&base];
        let extent = child_extent(parent, index)?;
        let mut chain = parent.chain.clone();
        chain.push(index);
        base = extent.start;
        let node = Node::open(
            &mut file,
            extent,
            chain.len(),
            chain,
            &doc.keys,
            &doc.resolver,
        )?;
        tree.nodes.insert(base, node);
    }
    doc.entry(base, chain[chain.len() - 1])
}

fn checked_file(path: &Path, metadata: &Metadata) -> Result<File, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let now = file.metadata().map_err(|e| e.to_string())?;
    if now.len() != metadata.len() || now.modified().ok() != metadata.modified().ok() {
        return Err(tr!("The archive changed on disk. Reopen it to read its table.").to_string());
    }
    Ok(file)
}

impl EntrySource {
    pub fn bytes(&self) -> Result<Vec<u8>, String> {
        self.bytes_limited(LIMIT)
    }

    fn bytes_limited(&self, limit: u64) -> Result<Vec<u8>, String> {
        let mut file = checked_file(&self.archive, &self.metadata)?;
        let bytes = self
            .read
            .read_limited(&mut file, &self.keys, limit)
            .map_err(|e| e.to_string())?;
        checked_file(&self.archive, &self.metadata)?;
        Ok(bytes)
    }

    pub fn extract(&self, output: &mut dyn std::io::Write) -> Result<u64, String> {
        let mut file = checked_file(&self.archive, &self.metadata)?;
        let count = self
            .read
            .extract(&mut file, &self.keys, output)
            .map_err(|e| e.to_string())?;
        checked_file(&self.archive, &self.metadata)?;
        Ok(count)
    }

    pub(crate) fn payload(&self) -> Result<DocPayload, String> {
        self.payload_limited(LIMIT)
    }

    pub(crate) fn payload_limited(&self, limit: u64) -> Result<DocPayload, String> {
        let result = (|| {
            let bytes = self.bytes_limited(limit)?;
            let content = Gta6EntryContent::detect(&self.entry, &self.extra, &bytes)
                .map_err(|e| e.to_string())?;
            let payload = if let Some(textures) = content.textures {
                crate::state::ttpf_document::table_textures(textures)?
            } else if let Some(payload) =
                table_preview(&self.path, &self.entry, &self.extra, &bytes)?
            {
                payload
            } else if DocumentKind::from_path(&self.path) == Some(DocumentKind::Text) {
                crate::state::document_io::text_payload(&self.path, bytes)
            } else {
                DocPayload::Hex(super::super::HexDocument {
                    bytes,
                    banner: crate::i18n::tr(READ_ONLY).to_string(),
                })
            };
            Ok((content.kind, payload))
        })();
        let mut tree = self.tree.lock().unwrap_or_else(|p| p.into_inner());
        match &result {
            Ok((kind, _)) => {
                tree.detected.insert(self.key, *kind);
                tree.errors.remove(&self.key);
            }
            Err(error) => {
                tree.detected.remove(&self.key);
                tree.errors.insert(self.key, String::clone(error));
            }
        }
        result.map(|(_, payload)| payload)
    }
}

fn preview_error(error: impl std::fmt::Display) -> String {
    tr!(
        "GTA VI's headerless resource reader does not read this body: {e}",
        e = error
    )
}

fn table_preview(
    path: &Path,
    entry: &Gta6Entry,
    extra: &Gta6EntryExtra,
    bytes: &[u8],
) -> Result<Option<DocPayload>, String> {
    use or_io::container::rpf::{Gta6EntryType, Rpf9Role, Rpf9Signature};
    use or_io::gta6::{headerless, map};

    let hint = Gta6EntryType::hint(entry, extra);
    let supported = matches!(
        hint,
        Gta6EntryType::Resource(
            Rpf9Role::Drawable
                | Rpf9Role::DrawableWithBound
                | Rpf9Role::Dictionary
                | Rpf9Role::Bound
        ) | Gta6EntryType::File("ymap" | "ytyp")
    );
    if !supported {
        return Ok(None);
    }
    let Some(row) = extra.nested else {
        return Ok(None);
    };
    // The low nibbles are not part of the section sizes; only an exact sum permits this flat section split.
    let virtual_len = (row.virtual_size & !15) as usize;
    let physical_len = (row.flags & !15) as usize;
    if virtual_len.checked_add(physical_len) != Some(bytes.len()) {
        return Err(preview_error(
            "the table's section sizes do not match its decoded bytes",
        ));
    }
    let (virtual_section, physical) = bytes.split_at(virtual_len);
    if let Gta6EntryType::File(extension) = hint {
        let expected = if extension == "ymap" {
            map::C_MAP_DATA
        } else {
            map::C_MAP_TYPES
        };
        if map::root_type(virtual_section) != Some(expected) {
            return Err(preview_error("the map root conflicts with its table type"));
        }
        let map = crate::state::gta6_map::keep(path, virtual_section, physical);
        crate::state::gta6_world::keep(path, virtual_section, physical);
        let document =
            match super::super::MetaDocument::from_gta_vi_headerless(virtual_section, physical) {
                Ok(document) => document,
                Err(error) if map => {
                    let mut document = super::super::MetaDocument::empty();
                    document.parsed = None;
                    document.parse_error = Some(preview_error(error));
                    document.source_format = or_io::meta::SourceFormat::Rbm;
                    document
                }
                Err(error) => return Err(preview_error(error)),
            };
        return Ok(Some(DocPayload::Meta(document)));
    }
    let signature = Rpf9Signature::of(virtual_section, virtual_len as u64)
        .ok_or_else(|| preview_error("the resource has no complete root"))?;
    if signature.role().map(Gta6EntryType::Resource) != Some(hint) {
        return Err(preview_error(
            "the resource root conflicts with its table type",
        ));
    }
    let resource =
        headerless::read(signature, virtual_section, Some(physical)).map_err(preview_error)?;
    match &resource {
        headerless::Resource::Drawable(drawable) => check_drawable(drawable)?,
        headerless::Resource::DrawableWithBound(fragment) => {
            check_drawable(&fragment.drawable)?;
        }
        headerless::Resource::Dictionary(dictionary) => {
            for (_, entry) in &dictionary.entries {
                match entry.as_ref().map_err(preview_error)? {
                    headerless::Entry::Drawable(drawable) => check_drawable(drawable)?,
                    _ => {
                        return Err(preview_error(
                            "a drawable dictionary entry is not a drawable",
                        ))
                    }
                }
            }
        }
        _ => {}
    }
    let resource = Arc::new(resource);
    let fragment = match signature.role() {
        Some(Rpf9Role::Drawable) => Some(false),
        Some(Rpf9Role::DrawableWithBound) => Some(true),
        _ => None,
    };
    if let Some(fragment) = fragment {
        crate::state::gta6_head::keep(path, &resource, virtual_section, physical, fragment);
    }
    Ok(Some(DocPayload::Gta6Drawable(resource)))
}

fn check_drawable(drawable: &or_io::gta6::headerless::Drawable) -> Result<(), String> {
    for model in drawable.levels.iter().flatten().flatten() {
        model.palette.as_ref().map_err(preview_error)?;
        for geometry in &model.geometries {
            geometry
                .as_ref()
                .map_err(preview_error)?
                .positions
                .as_ref()
                .map_err(preview_error)?;
        }
    }
    if let Some(embedded) = &drawable.embedded {
        embedded.as_ref().as_ref().map_err(preview_error)?;
    }
    Ok(())
}

impl AppState {
    pub(crate) fn rpf9_tree_context(
        &self,
        path: &Path,
    ) -> Option<(Arc<Rpf9TableDocument>, Option<RpfEntryBinding>)> {
        let (archive, chain) = address(path).ok()?;
        let (&index, parents) = chain.split_last()?;
        for doc in &self.documents {
            let DocPayload::Rpf(rpf) = &doc.payload else {
                continue;
            };
            let table = rpf.session.rpf9_table(&archive).cloned();
            let Some(table) = table else { continue };
            let base = table
                .tree()
                .nodes
                .iter()
                .find_map(|(&base, node)| (node.chain == parents).then_some(base));
            let binding = base.and_then(|base| {
                rpf.session
                    .nodes
                    .values()
                    .find_map(|node| match &node.backing {
                        RpfNodeBacking::Rpf9File {
                            table: owner,
                            base: at,
                            index: row,
                        } if owner.path == archive && *at == base && *row == index => {
                            Some(RpfEntryBinding {
                                parent_doc_id: doc.id,
                                entry_id: node.id,
                            })
                        }
                        _ => None,
                    })
            });
            return Some((table, binding));
        }
        self.documents.iter().find_map(|doc| match &doc.payload {
            DocPayload::Rpf9(super::super::Rpf9Document::Table(table)) if table.path == archive => {
                Some((Arc::clone(table), None))
            }
            _ => None,
        })
    }
    pub fn open_rpf9_entry(&mut self, table: &Rpf9TableDocument, base: u64, index: u32) {
        self.open_rpf9_row(table, base, index, None);
    }

    pub(crate) fn open_rpf9_row(
        &mut self,
        table: &Rpf9TableDocument,
        base: u64,
        index: u32,
        binding: Option<RpfEntryBinding>,
    ) {
        let source = match table.entry(base, index) {
            Ok(source) => source,
            Err(error) => {
                self.set_error(tr!("Could not open the file"), error);
                return;
            }
        };
        if let Some(open) = self.document_holding(&source.path, binding) {
            self.bind_rpf9_document(open, binding);
            self.set_active_document(open);
            return;
        }
        if self.load_receiver.is_some() {
            if let Some(binding) = binding {
                self.tabs
                    .queued_rpf9
                    .push_back((source.path.clone(), binding));
            }
            self.load_file(source.path);
            return;
        }
        self.loading = LoadingState::Loading;
        let (tx, rx) = mpsc::channel();
        self.load_receiver = Some((source.path.clone(), rx));
        self.tabs.arriving = binding;
        std::thread::spawn(move || {
            let result = match source.payload() {
                Ok(payload) => LoadResult::Success {
                    path: source.path,
                    payload,
                    binding,
                    resource: None,
                },
                Err(message) => LoadResult::Error {
                    path: source.path,
                    message,
                },
            };
            let _ = tx.send(result);
        });
    }

    pub(crate) fn bind_rpf9_document(
        &mut self,
        open: crate::state::DocumentId,
        binding: Option<RpfEntryBinding>,
    ) {
        let Some(binding) = binding else { return };
        if let Some(doc) = self.document_mut(open) {
            doc.rpf_entry_binding = Some(binding);
        }
        if let Some(DocPayload::Rpf(rpf)) = self
            .document_mut(binding.parent_doc_id)
            .map(|doc| &mut doc.payload)
        {
            rpf.entry_documents.insert(binding.entry_id, open);
        }
    }

    pub fn extract_rpf9_entry(
        &mut self,
        table: &Rpf9TableDocument,
        base: u64,
        index: u32,
        dest: PathBuf,
    ) -> bool {
        use crate::state::extract_job::{ExtractItem, OnClash};
        let source = match table.entry(base, index) {
            Ok(source) => source,
            Err(error) => {
                self.show_error_toast(error);
                return false;
            }
        };
        match self.gate_permits(Some(&dest)) {
            crate::state::GateOutcome::Proceed => {}
            crate::state::GateOutcome::Refused => return false,
            crate::state::GateOutcome::Confirm(_) => {
                self.show_warning_toast(tr!("Extract into a folder outside the game's install."));
                return false;
            }
        }
        let name = source.name.clone();
        let archive = source.archive.clone();
        let item = ExtractItem::rpf9(source);
        self.start_archive_extraction(name, dest, archive, vec![item], OnClash::KeepBoth)
    }
}

#[cfg(test)]
#[path = "rpf9_entry_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "rpf9_texture_tests.rs"]
mod texture_tests;

#[cfg(test)]
#[path = "rpf9_texture_read_only_tests.rs"]
mod texture_read_only_tests;

#[cfg(test)]
#[path = "rpf9_preview_tests.rs"]
mod preview_tests;
