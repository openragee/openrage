//! Opening and extracting an entry through its table, with a reloadable archive/index address.

use super::*;
use crate::state::{AppState, DocPayload, DocumentKind, LoadResult, LoadingState};
use or_io::container::rpf::Gta6EntryRead;

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
        let name = file_name(entry);
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
    let (archive, chain) = address(path)?;
    let doc = Rpf9TableDocument::open(&archive)?
        .ok_or_else(|| tr!("The archive table could not be loaded.").to_string())?;
    let mut source = read_chain(&doc, &chain)?;
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
        let mut archive =
            open_at_bounded(&mut file, extent.clone(), &doc.keys).map_err(|e| e.to_string())?;
        archive.resolve_names(&doc.resolver);
        let mut chain = parent.chain.clone();
        chain.push(index);
        base = extent.start;
        tree.nodes.insert(
            base,
            Node {
                archive,
                extent,
                depth: chain.len(),
                chain,
            },
        );
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
        let mut file = checked_file(&self.archive, &self.metadata)?;
        let bytes = self
            .read
            .read_limited(&mut file, &self.keys, LIMIT)
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

    fn payload(&self) -> Result<DocPayload, String> {
        let bytes = self.bytes()?;
        if DocumentKind::from_path(&self.path) == Some(DocumentKind::Text) {
            return super::super::TextDocument::open(&self.path, &bytes)
                .map(DocPayload::Text)
                .map_err(|e| e.to_string());
        }
        Ok(DocPayload::Hex(super::super::HexDocument {
            bytes,
            banner: crate::i18n::tr(READ_ONLY).to_string(),
        }))
    }
}

impl AppState {
    pub fn open_rpf9_entry(&mut self, table: &Rpf9TableDocument, base: u64, index: u32) {
        let source = match table.entry(base, index) {
            Ok(source) => source,
            Err(error) => {
                self.set_error(tr!("Could not open the file"), error);
                return;
            }
        };
        if let Some(open) = self.document_holding(&source.path, None) {
            self.set_active_document(open);
            return;
        }
        if self.load_receiver.is_some() {
            self.load_file(source.path);
            return;
        }
        self.loading = LoadingState::Loading;
        let (tx, rx) = mpsc::channel();
        self.load_receiver = Some((source.path.clone(), rx));
        std::thread::spawn(move || {
            let result = match source.payload() {
                Ok(payload) => LoadResult::Success {
                    path: source.path,
                    payload,
                    binding: None,
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
