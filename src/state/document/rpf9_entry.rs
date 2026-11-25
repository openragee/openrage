//! Opening and extracting an entry through its table, with a reloadable archive/index address.

use super::*;
use crate::state::{
    AppState, DocPayload, DocumentKind, LoadResult, LoadingState, RpfEntryBinding, RpfNodeBacking,
};
use or_io::container::rpf::{Gta6EntryContent, Gta6EntryExtra, Gta6EntryRead};

#[path = "rpf9_media_preview.rs"]
mod media_preview;
#[path = "rpf9_resource_preview.rs"]
mod resource_preview;
#[path = "../gta6_script_view.rs"]
mod script_view;

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
        let read = node
            .mount
            .read_entry_plan(index, node.extent.end, u64::MAX)
            .map_err(|e| e.to_string())?;
        let entry = &node.mount.archive.entries[index as usize];
        let extra = node
            .mount
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
            .copied()
            .chain([index])
            .collect::<Vec<_>>();
        Ok(EntrySource {
            path: indexed_path(&self.path, &indices, &name),
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

pub(crate) fn indexed_path(archive: &Path, indices: &[u32], name: &str) -> PathBuf {
    let indices = indices
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join("_");
    archive.join(MARKER).join(indices).join(name)
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

pub(crate) fn address(path: &Path) -> Result<(PathBuf, Vec<u32>), String> {
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
            doc.read_headers,
        )?;
        tree.nodes.insert(base, node);
        tree.revision = tree.revision.wrapping_add(1);
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

    pub(crate) fn stored_len(&self) -> u64 {
        self.read.stored_len()
    }

    pub(crate) fn decoded_len(&self) -> u64 {
        self.read.decoded_len()
    }

    pub(crate) fn section_lengths(&self) -> Option<(u32, u32)> {
        self.extra
            .nested
            .map(|row| (row.virtual_size & !15, row.flags & !15))
    }

    pub(crate) fn bytes_limited(&self, limit: u64) -> Result<Vec<u8>, String> {
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

    pub(crate) fn texture_keys_limited(&self, limit: u64) -> Result<Vec<u64>, String> {
        use or_io::container::rpf::{Rpf9Role, Rpf9Signature};
        use or_io::gta6::{hash::name_hash64, headerless};

        let row = self
            .extra
            .nested
            .filter(|row| row.resource_type == 5)
            .ok_or_else(|| preview_error("the table entry is not a texture dictionary"))?;
        if Gta6EntryType::hint(&self.entry, &self.extra) != Gta6EntryType::TextureDictionary
            || row.virtual_size & 15 != 0
            || row.flags & 15 != 3
        {
            return Err(preview_error(
                "the texture dictionary has conflicting rows or unsupported section flags",
            ));
        }
        let virtual_len = u64::from(row.virtual_size & !15);
        let physical_len = u64::from(row.flags & !15);
        if virtual_len + physical_len != self.decoded_len() {
            return Err(preview_error(
                "the texture dictionary's section sizes do not match its decoded bytes",
            ));
        }
        if virtual_len > limit {
            return Err(preview_error(
                "the texture dictionary's virtual section exceeds its byte budget",
            ));
        }
        let count = usize::try_from(virtual_len)
            .map_err(|_| preview_error("the virtual section exceeds addressable memory"))?;
        let mut file = checked_file(&self.archive, &self.metadata)?;
        let bytes = self
            .read
            .read_prefix(&mut file, &self.keys, count, 64 << 20)
            .map_err(preview_error)?;
        checked_file(&self.archive, &self.metadata)?;
        if Rpf9Signature::of(&bytes, virtual_len).and_then(Rpf9Signature::role)
            != Some(Rpf9Role::Dictionary)
        {
            return Err(preview_error(
                "the texture dictionary's root is not a dictionary",
            ));
        }
        let dictionary = headerless::read_dictionary(&bytes, None).map_err(preview_error)?;
        dictionary
            .entries
            .into_iter()
            .map(|(_, entry)| match entry.map_err(preview_error)? {
                headerless::Entry::Texture(texture) => Ok(name_hash64(texture.name.as_bytes())),
                _ => Err(preview_error("a texture dictionary entry is not a texture")),
            })
            .collect()
    }

    pub(crate) fn payload(&self) -> Result<DocPayload, String> {
        self.payload_limited(LIMIT)
    }

    pub(crate) fn payload_limited(&self, limit: u64) -> Result<DocPayload, String> {
        self.payload_from_read_at(&self.path, self.bytes_limited(limit))
    }

    pub(crate) fn payload_from_bytes_at(
        &self,
        path: &Path,
        bytes: Vec<u8>,
    ) -> Result<DocPayload, String> {
        self.payload_from_read_at(path, Ok(bytes))
    }

    fn payload_from_read_at(
        &self,
        path: &Path,
        bytes: Result<Vec<u8>, String>,
    ) -> Result<DocPayload, String> {
        let result = (|| {
            let bytes = bytes?;
            checked_file(&self.archive, &self.metadata)?;
            if bytes.len() as u64 != self.decoded_len() {
                return Err(preview_error(
                    "the entry's decoded length does not match its bytes",
                ));
            }
            let content = Gta6EntryContent::detect(&self.entry, &self.extra, &bytes)
                .map_err(|e| e.to_string())?;
            let payload = if let Some(textures) = content.textures {
                crate::state::ttpf_document::table_textures(textures)?
            } else if let Some(payload) = table_preview(path, &self.entry, &self.extra, &bytes)? {
                payload
            } else if let Some(payload) = script_view::script_payload(&bytes) {
                payload
            } else if DocumentKind::from_path(path) == Some(DocumentKind::Text) {
                crate::state::document_io::text_payload(path, bytes)
            } else {
                DocPayload::Hex(crate::state::document::HexDocument {
                    bytes,
                    banner: crate::i18n::tr(READ_ONLY).to_string(),
                })
            };
            checked_file(&self.archive, &self.metadata)?;
            Ok((content.kind, payload))
        })();
        let mut tree = self.tree.lock().unwrap_or_else(|p| p.into_inner());
        match &result {
            Ok((kind, _)) => {
                tree.detected.insert(self.key, *kind);
                tree.revision = tree.revision.wrapping_add(1);
                tree.errors.remove(&self.key);
            }
            Err(error) => {
                tree.detected.remove(&self.key);
                tree.revision = tree.revision.wrapping_add(1);
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

    if let Some(payload) = media_preview::payload(path, bytes)? {
        return Ok(Some(payload));
    }
    let hint = Gta6EntryType::hint(entry, extra);
    let supported = matches!(
        hint,
        Gta6EntryType::Resource(
            Rpf9Role::Drawable
                | Rpf9Role::DrawableWithBound
                | Rpf9Role::Dictionary
                | Rpf9Role::Bound
                | Rpf9Role::NavMesh
                | Rpf9Role::ParticleEffects
        ) | Gta6EntryType::File("ymap" | "ytyp" | "ymt" | "ycd" | "yed")
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
    if hint == Gta6EntryType::File("ycd") {
        return resource_preview::animation(path, virtual_section, physical).map(Some);
    }
    if hint == Gta6EntryType::File("yed") {
        return resource_preview::expressions(path, virtual_section, physical).map(Some);
    }
    if let Gta6EntryType::File(extension) = hint {
        if extension != "ymt" {
            let expected = if extension == "ymap" {
                map::C_MAP_DATA
            } else {
                map::C_MAP_TYPES
            };
            if map::root_type(virtual_section) != Some(expected) {
                return Err(preview_error("the map root conflicts with its table type"));
            }
        }
        return resource_preview::metadata(path, virtual_section, physical).map(Some);
    }
    let signature = Rpf9Signature::of(virtual_section, virtual_len as u64)
        .ok_or_else(|| preview_error("the resource has no complete root"))?;
    if signature.role().map(Gta6EntryType::Resource) != Some(hint) {
        return Err(preview_error(
            "the resource root conflicts with its table type",
        ));
    }
    if hint == Gta6EntryType::Resource(Rpf9Role::NavMesh) {
        return resource_preview::navmesh(path, virtual_section, physical).map(Some);
    }
    if hint == Gta6EntryType::Resource(Rpf9Role::ParticleEffects) {
        return resource_preview::particles(path, virtual_section, physical).map(Some);
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
        self.mounted_rpf9_table(&archive).map(|table| (table, None))
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
#[path = "rpf9_ciphered_texture_tests.rs"]
mod ciphered_texture_tests;

#[cfg(test)]
#[path = "rpf9_texture_read_only_tests.rs"]
mod texture_read_only_tests;

#[cfg(test)]
#[path = "rpf9_preview_tests.rs"]
mod preview_tests;
