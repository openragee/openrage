//! What the Files view in the centre says about a row and its folder, and the
//! moves only it makes: going to a typed path, reading a folder again and
//! exporting a row under a name of the user's.

pub mod pictures;
#[path = "../rpf9_tree_row_kind.rs"]
mod rpf9_tree_row_kind;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};

use or_io::container::resource::{ResourceEnvelope, Rsc7};
use or_io::container::rpf::gta6_kinds::kind_label;
use or_io::container::rpf::gta6_rpf::row::Rpf9Row;
use or_io::container::rpf::{read_rpf9_body, Rpf9Sections, RpfEncryption, RpfEntry, RpfVersion};
use or_io::container::RpfArchive;

use super::document::{DocumentId, RpfNodeId};
use super::explorer::{ColumnValue, SortKey};
use super::navigation::{resolve_path, Location, Resolved};
use super::rpf_node::{ArchiveSource, RpfNodeBacking, RpfSessionNode};
use super::rpf_session::RpfEditSession;
use super::{AppState, DocPayload, DocumentKind};
use crate::ui::panels::selection_ids::DOC_ROOT_SEL;

/// One row's Details columns; `None` is a cell that does not apply.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowFacts {
    /// The type in words: "Folder", "Archive", "Drawable", "XML file".
    pub kind: String,
    /// The bytes it holds once extracted.
    pub size: Option<u64>,
    /// The bytes it takes inside its archive.
    pub stored: Option<u64>,
    pub compression: Option<&'static str>,
    pub encryption: Option<&'static str>,
    /// The edition its RSC7 version names, for a resource.
    pub edition: Option<&'static str>,
    pub version: Option<u32>,
}

/// What a row is, in words: a GTA VI row's kind as its table states it, else [`type_in_words`].
pub fn kind_in_words(session: &RpfEditSession, node: &RpfSessionNode) -> String {
    if let Some(kind) = super::explorer::gta6_kind(session, node) {
        return kind.label.to_owned();
    }
    let RpfNodeBacking::ArchiveFile {
        archive_id,
        entry_index,
        ..
    } = &node.backing
    else {
        return type_in_words(node);
    };
    let row = (session.archive(*archive_id)).and_then(|slot| slot.archive.rpf9_row(*entry_index));
    stated_kind(row).unwrap_or_else(|| type_in_words(node))
}

/// The class byte of a 24-byte row that holds a nested archive.
const ARCHIVE_CLASS: u8 = 0x00;

/// The kind a GTA VI row no kind list names is shown by, as Details states it: a class or type
/// code in hex, or no row; `None` for a row whose class says archive over a body that begins
/// none, which is a file as the tree lists it.
fn stated_kind(row: Option<Rpf9Row>) -> Option<String> {
    let row = row?;
    (row.kind != kind_label(Some(ARCHIVE_CLASS), None)).then(|| row.kind.into_owned())
}

/// What a row is, in words.
pub fn type_in_words(node: &RpfSessionNode) -> String {
    if matches!(node.backing, RpfNodeBacking::Archive { .. }) {
        return DocumentKind::Rpf.info().noun.to_string();
    }
    if node.is_dir() {
        return tr!("Folder").to_string();
    }
    match DocumentKind::from_path(Path::new(&node.name)) {
        Some(kind) => kind.info().noun.to_string(),
        None => match super::extract_job::extension_of(&node.name) {
            ext if ext.is_empty() => tr!("File").to_string(),
            ext => format!("{} file", ext.to_ascii_uppercase()),
        },
    }
}

/// A GTA VI plain file's kind as its body's head names it, which only a read gives.
pub fn kind_by_head(kind: String, head: &[u8]) -> String {
    if kind != or_io::container::rpf::gta6_kinds::PLAIN_FILE {
        return kind;
    }
    if head.starts_with(b"PSIN") {
        return "PSO".into();
    }
    if head.starts_with(b"RBF0") {
        return "RBF".into();
    }
    if !super::document_io::is_text(head) {
        return kind;
    }
    let text = head.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(head);
    match text.trim_ascii_start().starts_with(b"<") {
        true => tr!("XML").to_string(),
        false => tr!("text").to_string(),
    }
}

/// Deflate or None: what or-io says of an RPF9 body, else what the entry's compressor names.
fn compression_label(rpf9_deflated: Option<bool>, compressed: bool) -> &'static str {
    match rpf9_deflated.unwrap_or(compressed) {
        true => tr!("Deflate"),
        false => tr!("None"),
    }
}

/// The bytes row `id` adds to its folder's total: the Size it shows for a file or an archive, a folder's none.
pub fn listed_bytes(session: &RpfEditSession, id: RpfNodeId) -> u64 {
    measured_bytes(session, id).unwrap_or(0)
}

/// `listed_bytes`, or `None` while a worker still measures the row's held body.
pub fn measured_bytes(session: &RpfEditSession, id: RpfNodeId) -> Option<u64> {
    let Some(node) = session.node(id) else {
        return Some(0);
    };
    match &node.backing {
        RpfNodeBacking::Archive { .. } => {
            Some(row_facts(session, id).and_then(|f| f.size).unwrap_or(0))
        }
        RpfNodeBacking::ArchiveFile {
            archive_id,
            entry_index,
            override_bytes: Some(held),
        } => match session.archive(*archive_id) {
            Some(slot) => held_size(&slot.archive, *entry_index, held),
            None => Some(held.len() as u64),
        },
        // ⚠ A GTA VI file's size is its row's: its entry's is 32-bit.
        RpfNodeBacking::ArchiveFile {
            archive_id,
            entry_index,
            override_bytes: None,
        } => {
            let row = (session.archive(*archive_id)).and_then(|s| s.archive.rpf9_row(*entry_index));
            Some(row.map_or_else(|| session.size_for(id).unwrap_or(0), |row| row.real))
        }
        _ if node.is_dir() => Some(0),
        _ => Some(session.size_for(id).unwrap_or(0)),
    }
}

/// A held body and the size of the sections it stands for, `None` until measured.
type Measured = (Weak<Vec<u8>>, Option<u64>);

/// The bytes a row's held body stands for: a headerless resource's sections, which the loose form it holds deflates, else its length; `None` while a worker measures those sections.
fn held_size(archive: &RpfArchive, entry_index: u32, held: &Arc<Vec<u8>>) -> Option<u64> {
    let headerless = (archive.rpf9_row(entry_index)).is_some_and(|row| row.row_bytes == Some(20));
    if !headerless {
        return Some(held.len() as u64);
    }
    // ponytail: a loose form states no section size, so a worker inflates each held body once and the
    // row shows no size until it ends; a view showing `None` asks for a repaint to pick it up.
    static MEMO: Mutex<Vec<Measured>> = Mutex::new(Vec::new());
    let mut memo = MEMO.lock().unwrap_or_else(PoisonError::into_inner);
    memo.retain(|(body, _)| body.strong_count() > 0);
    let held_at = Arc::as_ptr(held);
    if let Some((_, len)) = memo.iter().find(|(body, _)| body.as_ptr() == held_at) {
        return *len;
    }
    memo.push((Arc::downgrade(held), None));
    let body = Arc::clone(held);
    std::thread::spawn(move || {
        let len = sections_len(&body).unwrap_or(body.len() as u64);
        let mut memo = MEMO.lock().unwrap_or_else(PoisonError::into_inner);
        let at = Arc::as_ptr(&body);
        if let Some((_, measured)) = memo.iter_mut().find(|(b, _)| b.as_ptr() == at) {
            *measured = Some(len);
        }
    });
    None
}

/// The inflated length of the sections a loose GTA VI resource holds; `None` for bytes that are no such file.
fn sections_len(loose: &[u8]) -> Option<u64> {
    let (find, body) = read_rpf9_body(std::io::Cursor::new(loose), 0, u64::MAX).ok()?;
    let sections = Rpf9Sections::of(&find, &body)?;
    Some((sections.virtual_section.len() + sections.physical.len()) as u64)
}

/// A GTA VI file's sizes and cipher as its table's row states them: its entry's sizes are 32-bit
/// and its archive's header names no cipher.
fn take_row(facts: &mut RowFacts, row: &Rpf9Row) {
    facts.size = Some(row.real);
    facts.stored = Some(row.stored);
    facts.encryption = Some(match row.ciphered {
        true => tr!("Enciphered"),
        false => tr!("None"),
    });
}

/// An archive's own encryption, its table's: a GTA VI table is always enciphered, with its own cipher.
fn archive_encryption(archive: &RpfArchive) -> &'static str {
    match archive.version {
        RpfVersion::Rpf9 => tr!("Enciphered"),
        _ => encryption_label(archive.header.encryption),
    }
}

fn encryption_label(encryption: RpfEncryption) -> &'static str {
    match encryption {
        RpfEncryption::None | RpfEncryption::Open => tr!("None"),
        RpfEncryption::Aes => tr!("AES"),
        RpfEncryption::Ng => tr!("NG"),
        RpfEncryption::Other(_) | RpfEncryption::Tagged(_) => tr!("Unknown"),
    }
}

/// The edition a resource called `name` at RSC7 `version` is laid out for.
fn edition_of(name: &str, version: u32) -> Option<&'static str> {
    let ext = super::extract_job::extension_of(name);
    let target = or_io::gta5::formats::format(&ext)?
        .target_of_version(version)
        .ok()?;
    Some(target.edition_label())
}

fn rsc7_version(bytes: &[u8]) -> Option<u32> {
    Rsc7::detect(bytes)
        .then(|| Rsc7::peek(bytes).ok())
        .flatten()
        .map(|seal| seal.version())
}

/// A file on disk's length, and its RSC7 version when it has one.
type DiskFacts = (Option<u64>, Option<u32>);

/// ponytail: one memo for the process, keyed by path, so a row's facts cost one
/// read and not one a frame; Refresh forgets a folder's, and a file rewritten
/// behind the app's back keeps its old facts until then.
fn disk_memo() -> &'static Mutex<HashMap<PathBuf, DiskFacts>> {
    static MEMO: OnceLock<Mutex<HashMap<PathBuf, DiskFacts>>> = OnceLock::new();
    MEMO.get_or_init(Default::default)
}

fn disk_facts(path: &Path) -> DiskFacts {
    if let Some(known) = disk_memo().lock().ok().and_then(|m| m.get(path).copied()) {
        return known;
    }
    let len = std::fs::metadata(path).ok().map(|m| m.len());
    let version = DocumentKind::from_path(path)
        .and_then(DocumentKind::resource_type)
        .and_then(|_| {
            use std::io::Read;
            let mut header = [0u8; 16];
            let read = std::fs::File::open(path).ok()?.read(&mut header).ok()?;
            rsc7_version(&header[..read])
        });
    if let Ok(mut m) = disk_memo().lock() {
        m.insert(path.to_path_buf(), (len, version));
    }
    (len, version)
}

/// Forget what [`disk_facts`] read under `folder`, so the next look reads it again.
fn forget_disk_facts(folder: &Path) {
    if let Ok(mut m) = disk_memo().lock() {
        m.retain(|path, _| !path.starts_with(folder));
    }
}

/// The Details columns of row `id`.
pub fn row_facts(session: &RpfEditSession, id: RpfNodeId) -> Option<RowFacts> {
    let node = session.node(id)?;
    let mut facts = RowFacts {
        kind: kind_in_words(session, node),
        ..Default::default()
    };
    match &node.backing {
        RpfNodeBacking::ArchiveFile {
            archive_id,
            entry_index,
            override_bytes,
        } => {
            let archive = &session.archive(*archive_id)?.archive;
            match archive.entries.get(*entry_index as usize)? {
                RpfEntry::Binary(b) => {
                    facts.size = Some(u64::from(b.uncompressed_size));
                    facts.stored = Some(u64::from(b.read_size()));
                    facts.compression = Some(compression_label(
                        archive.rpf9_deflated(*entry_index),
                        b.is_compressed(),
                    ));
                    facts.encryption = Some(if b.encrypted {
                        encryption_label(archive.header.encryption)
                    } else {
                        tr!("None")
                    });
                    if let Some(row) = archive.rpf9_row(*entry_index) {
                        take_row(&mut facts, &row);
                    }
                }
                RpfEntry::Resource(r) => {
                    let version =
                        Rsc7::header_from_entry_flags(r.system_info, r.graphics_info).version();
                    facts.size = Some(u64::from(r.consumed_size));
                    facts.stored = Some(u64::from(r.consumed_size));
                    // ⚠ Its pages are deflated inside the RSC7, and the archive stores it as it is.
                    facts.compression = Some(tr!("Resource"));
                    let script = node.name.to_ascii_lowercase().ends_with(".ysc");
                    facts.encryption =
                        Some(if script && archive.header.encryption.needs_crypto() {
                            encryption_label(archive.header.encryption)
                        } else {
                            tr!("None")
                        });
                    facts.version = Some(version);
                    facts.edition = edition_of(&node.name, version);
                }
                RpfEntry::Directory(_) => {}
            }
            if let Some(edited) = override_bytes {
                facts.size = held_size(archive, *entry_index, edited);
                facts.version = rsc7_version(edited);
                facts.edition = facts.version.and_then(|v| edition_of(&node.name, v));
            }
        }
        RpfNodeBacking::InMemoryFile { bytes } => {
            facts.size = Some(bytes.len() as u64);
            facts.version = rsc7_version(bytes);
            facts.edition = facts.version.and_then(|v| edition_of(&node.name, v));
        }
        RpfNodeBacking::DiskFile {
            fs_path,
            override_bytes,
            ..
        } => {
            facts.size = session.size_for(id);
            facts.version = match override_bytes {
                Some(edited) => rsc7_version(edited),
                None => disk_facts(fs_path).1,
            };
            facts.edition = facts.version.and_then(|v| edition_of(&node.name, v));
        }
        RpfNodeBacking::Archive {
            archive_id, source, ..
        } => {
            match source {
                ArchiveSource::Disk(Some(path)) => facts.size = disk_facts(path).0,
                ArchiveSource::Nested {
                    parent_archive_id,
                    parent_entry_index,
                    ..
                } => {
                    let parent = &session.archive(*parent_archive_id)?.archive;
                    if let Some(RpfEntry::Binary(b)) =
                        parent.entries.get(*parent_entry_index as usize)
                    {
                        // ⚠ An entry's sizes are 32-bit, so a GTA VI archive past 4 GiB takes its row's.
                        let (size, stored) = match parent.rpf9_row(*parent_entry_index) {
                            Some(row) => (row.stored, row.stored),
                            None => (u64::from(b.uncompressed_size), u64::from(b.read_size())),
                        };
                        facts.size = Some(size);
                        facts.stored = Some(stored);
                        facts.compression = Some(compression_label(
                            parent.rpf9_deflated(*parent_entry_index),
                            b.is_compressed(),
                        ));
                    }
                }
                ArchiveSource::Disk(None) => {}
            }
            // ⚠ An archive's own encryption is its table's, known once it is open.
            facts.encryption = archive_id
                .and_then(|a| session.archive(a))
                .map(|slot| archive_encryption(&slot.archive));
        }
        _ => {}
    }
    Some(facts)
}

/// The value row `id` sorts by under `key`, for the keys that read a Details column.
pub fn column_value(session: &RpfEditSession, id: RpfNodeId, key: SortKey) -> Option<ColumnValue> {
    let facts = row_facts(session, id)?;
    match key {
        SortKey::StoredSize => facts.stored.map(ColumnValue::Number),
        SortKey::Compression => facts.compression.map(ColumnValue::Text),
        SortKey::Encryption => facts.encryption.map(ColumnValue::Text),
        SortKey::Edition => facts.edition.map(ColumnValue::Text),
        SortKey::Version => facts.version.map(|v| ColumnValue::Number(u64::from(v))),
        _ => None,
    }
}

/// The hash the game knows a file by: its name's without the extension, as RAGE hashes names.
pub fn name_hash(name: &str) -> u32 {
    let stem = Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name);
    or_io::hash::string_hash(stem.as_bytes())
}

/// The path inside a document a typed address names, `/`-separated and relative.
///
/// ⚠ A path on disk must be under `root`, the document's own file or folder;
/// one with a leading `/` and no drive is a path inside it, as Copy path writes one.
/// A drive letter and a slash, or a UNC prefix, is on disk on every platform: `Path::has_root`
/// is false for `C:\x` on Linux.
pub fn inner_of_address(root: Option<&Path>, typed: &str) -> Result<String, String> {
    let typed = typed.trim().trim_matches('"');
    let slashed = typed.replace('\\', "/");
    let drive =
        matches!(slashed.as_bytes(), [letter, b':', b'/', ..] if letter.is_ascii_alphabetic());
    let on_disk = drive || slashed.starts_with("//");
    if !on_disk {
        return Ok(slashed.trim_matches('/').to_string());
    }
    let Some(root) = root else {
        return Err(tr!(
            "`{typed}` is on disk, and this has no file there",
            typed = typed
        ));
    };
    let root = root.to_string_lossy().replace('\\', "/");
    let root = root.trim_end_matches('/');
    // ⚠ `get`, not a slice: the root's length can fall inside a letter of a typed address.
    let inside = slashed
        .get(..root.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(root))
        && slashed[root.len()..]
            .chars()
            .next()
            .is_none_or(|c| c == '/');
    if !inside {
        return Err(tr!(
            "`{typed}` is not inside `{root}`",
            typed = typed,
            root = root
        ));
    }
    Ok(slashed[root.len()..].trim_matches('/').to_string())
}

impl AppState {
    /// The folder the Files view shows for `doc`: where the user last went in it, a file's being its folder, else `kept`, else its root.
    pub fn files_view_folder(&self, doc: DocumentId, kept: Option<RpfNodeId>) -> Option<RpfNodeId> {
        let DocPayload::Rpf(rpf) = &self.document(doc)?.payload else {
            return None;
        };
        let session = &rpf.session;
        let folder_of = |id: RpfNodeId| {
            let node = session.node(id)?;
            if node.is_dir() {
                Some(id)
            } else {
                node.parent
            }
        };
        let here = self
            .navigation
            .current()
            .filter(|at| at.doc == doc)
            .and_then(|at| match at.node {
                DOC_ROOT_SEL => Some(session.root_id()),
                node => folder_of(node),
            });
        here.or_else(|| kept.filter(|&k| session.node(k).is_some_and(|n| n.is_dir())))
            .or(Some(session.root_id()))
    }

    /// Go where a path typed into the address bar points in `doc`, opening the archives on the way.
    pub fn go_to_address(&mut self, doc: DocumentId, typed: &str) -> Result<(), String> {
        let document = self
            .document(doc)
            .ok_or_else(|| "That document is closed".to_string())?;
        let root = document.path.clone();
        let title = document.title();
        let DocPayload::Rpf(rpf) = &document.payload else {
            return Err(tr!("{title} has no folders", title = title));
        };
        let inner = inner_of_address(root.as_deref(), typed)?;
        match resolve_path(&rpf.session, &inner) {
            Resolved::Found(node) => {
                let node = if inner.is_empty() { DOC_ROOT_SEL } else { node };
                self.navigate_to(Location { doc, node });
                Ok(())
            }
            Resolved::Unloaded(_) => match root {
                Some(root) => {
                    self.walk_to(&root, vec![inner], super::navigation::Arrival::Reveal);
                    Ok(())
                }
                None => Err(tr!(
                    "Nothing at `{typed}` in {title}",
                    typed = typed,
                    title = title
                )),
            },
            Resolved::Missing => Err(tr!(
                "Nothing at `{typed}` in {title}",
                typed = typed,
                title = title
            )),
        }
    }

    /// Write row `node` to `path` as other tools open it: decompressed, and a resource with its header put back.
    ///
    /// ⚠ A path inside the install is refused rather than asked about: the
    /// question's answer would come back to Extract…, which names a folder
    /// and not a file.
    pub fn export_rpf_node_as(
        &mut self,
        doc: DocumentId,
        node: RpfNodeId,
        path: PathBuf,
        fixes: Option<bool>,
    ) {
        use or_io::container::rpf::BodyForm;

        if let Some(what) = self.extract_jobs.running_noun() {
            self.show_error_toast(tr!("{what} is already running", what = what));
            return;
        }
        let Some(folder) = path.parent().map(Path::to_path_buf) else {
            return;
        };
        match self.gate_permits(Some(&path)) {
            super::GateOutcome::Proceed => {}
            super::GateOutcome::Refused => return,
            super::GateOutcome::Confirm(_) => {
                self.show_warning_toast(tr!(
                    "Export as… writes outside the install; Extract… asks before writing into it"
                ));
                return;
            }
        }
        if let Err(why) = self.no_job_reads(&path, "export") {
            self.show_error_toast(why);
            return;
        }
        let Some(DocPayload::Rpf(rpf)) = self.document(doc).map(|d| &d.payload) else {
            return;
        };
        let item =
            super::extract_job::plan(&rpf.session, node, &super::extract_job::TypeFilter::All)
                .into_iter()
                .next();
        let Some(item) = item else {
            self.show_warning_toast(tr!("There is nothing to export there"));
            return;
        };
        let Some(fixes) =
            fixes.or_else(|| self.export_fixes_for(std::slice::from_ref(&item), BodyForm::Decoded))
        else {
            self.pending_export_fixes = Some(super::extract_job::PendingExportFixes::ExportAs {
                doc_id: doc,
                node,
                path,
            });
            return;
        };
        let label = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let reads = item.archive_read().into_iter().collect();
        let now_on = label.clone();
        self.extract_jobs.start(
            super::jobs::JobKind::Export,
            label,
            folder,
            reads,
            move |cancel, progress| {
                or_job::run(vec![item], &cancel, &progress, |item| {
                    progress.now_on(now_on.clone());
                    match item.export_bytes(BodyForm::Decoded, fixes) {
                        Ok(bytes) => match std::fs::write(&path, &bytes) {
                            Ok(()) => or_job::Step::Done(bytes.len() as u64),
                            Err(e) => or_job::Step::Failed(format!("{}: {e}", path.display())),
                        },
                        Err(e) => or_job::Step::Failed(e),
                    }
                })
            },
        );
    }

    /// Read a folder on disk again: what was added shows, and what is gone leaves.
    ///
    /// ⚠ A row holding a document open from it, or an archive with unsaved
    /// changes, stays even when its file is gone: dropping it would lose work.
    pub fn refresh_folder(&mut self, doc: DocumentId, folder: RpfNodeId) -> Result<(), String> {
        let Some(DocPayload::Rpf(rpf)) = self.document_mut(doc).map(|d| &mut d.payload) else {
            return Err(tr!("That document is closed").to_string());
        };
        let Some(RpfNodeBacking::DiskFolder { fs_path, .. }) =
            rpf.session.node(folder).map(|n| n.backing.clone())
        else {
            return Err(tr!("Only a folder on disk can be read again").to_string());
        };
        forget_disk_facts(&fs_path);
        let holds_work = |row: RpfNodeId| {
            let mut stack = vec![row];
            while let Some(at) = stack.pop() {
                let Some(node) = rpf.session.node(at) else {
                    continue;
                };
                if rpf.session.archive_unsaved(at) || rpf.entry_documents.contains_key(&at) {
                    return true;
                }
                stack.extend(&node.children);
            }
            false
        };
        let gone: Vec<RpfNodeId> = rpf
            .session
            .children_of(folder)
            .into_iter()
            .filter(|&child| {
                let missing = rpf
                    .session
                    .node(child)
                    .and_then(|n| n.backing.fs_path())
                    .is_some_and(|p| !p.exists());
                missing && !holds_work(child)
            })
            .collect();
        for child in gone {
            rpf.session.delete_node(child)?;
        }
        if let Some(RpfNodeBacking::DiskFolder { scanned, .. }) =
            rpf.session.node_mut(folder).map(|n| &mut n.backing)
        {
            *scanned = false;
        }
        self.request_expand(doc, folder);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use or_io::container::rpf::RpfBuilder;

    use super::*;

    #[test]
    fn the_folder_total_counts_a_nested_archive() {
        let mut inner = RpfBuilder::default();
        inner
            .insert_with("deep.txt", b"deep".to_vec(), false)
            .unwrap();
        let inner = inner.to_bytes().unwrap();
        let mut outer = RpfBuilder::default();
        outer
            .insert_with("inner.rpf", inner.clone(), false)
            .unwrap();
        outer
            .insert_with("top.txt", b"top".to_vec(), false)
            .unwrap();
        let session = RpfEditSession::from_archive(outer.build().unwrap()).unwrap();
        let rows = session.children_of(session.root_id());
        let total: u64 = rows.iter().map(|&id| listed_bytes(&session, id)).sum();
        assert_eq!(total, inner.len() as u64 + 3);
    }

    mod gta6 {
        use std::io::Cursor;
        use std::sync::Arc;

        use or_io::container::rpf::gta6_container_write::{build, Rpf9IndexKey, Rpf9NewBody};
        use or_io::container::rpf::{
            write_rpf9_resource, Gta6Context, NameResolver, Rpf9Form, Rpf9Sections,
        };
        use or_io::container::RpfArchive;
        use or_io::gta6::hash::name_hash64;

        use super::super::*;
        use crate::state::rpf_archive_open::gta6_session::{self, Reading};
        use crate::state::rpf_archive_open::tree_verbs::tests::{rbm_section, synthetic_keys};

        /// Archives opened through [`tree`] while this is held are read with the synthetic keys and `names`.
        fn reading(names: &[&str]) -> Reading {
            let mut resolver = NameResolver::new();
            for &name in names {
                resolver.insert(name_hash64(name.as_bytes()), name);
            }
            gta6_session::hold(Gta6Context {
                keys: synthetic_keys().expect("the synthetic snapshot parses"),
                names: Arc::new(resolver),
            })
        }

        fn built(entries: &[(String, Rpf9NewBody)]) -> Vec<u8> {
            let key = Rpf9IndexKey {
                key_id: 0xDEAD_BEEF,
                iv: [0x33; 16],
            };
            let keys = synthetic_keys().expect("the synthetic snapshot parses");
            let plan = build(entries, or_io::platform::Platform::XboxSeries, &key, &keys)
                .expect("the archive builds");
            let mut bytes = Vec::new();
            (plan.write_to(&mut Cursor::new(Vec::<u8>::new()), &mut bytes))
                .expect("the archive is laid out");
            bytes
        }

        fn chunked(name: &str) -> (String, Rpf9NewBody) {
            let body = format!("the body of {name}, which deflates. ").repeat(40);
            (name.to_owned(), Rpf9NewBody::Chunked(body.into_bytes()))
        }

        fn loose(virtual_section: Vec<u8>) -> Vec<u8> {
            write_rpf9_resource(&Rpf9Sections {
                form: Rpf9Form::BlockMapped,
                virtual_section,
                physical: Vec::new(),
            })
            .expect("a loose resource")
        }

        /// The tree of the GTA VI archive `bytes`, read from a file in `dir`.
        fn tree(dir: &Path, bytes: Vec<u8>) -> RpfEditSession {
            let path = dir.join("source.rpf");
            std::fs::write(&path, bytes).expect("the archive is written");
            let archive =
                RpfArchive::open_with(&path, gta6_session::crypto()).expect("the archive opens");
            RpfEditSession::from_archive(archive).expect("the tree lists")
        }

        fn row(session: &RpfEditSession, name: &str) -> RpfNodeId {
            (session.all_nodes().into_iter())
                .find(|&id| session.node(id).is_some_and(|n| n.name == name))
                .unwrap_or_else(|| panic!("no row {name}"))
        }

        fn size(session: &RpfEditSession, id: RpfNodeId) -> Option<u64> {
            row_facts(session, id).and_then(|facts| facts.size)
        }

        #[test]
        fn a_headerless_row_holding_its_loose_form_shows_the_sections_it_inflates_to() {
            let _serial = crate::ui::theme::serial();
            let _reading = reading(&["data/model.ydr", "data/named.bin"]);
            let dir = tempfile::tempdir().expect("a scratch directory");
            let resource = Rpf9NewBody::Resource {
                type_code: 0x02,
                loose: loose(rbm_section()),
            };
            let entries = [
                ("data/model.ydr".to_owned(), resource),
                chunked("data/named.bin"),
            ];
            let mut session = tree(dir.path(), built(&entries));
            let model = row(&session, "model.ydr");
            assert_eq!(size(&session, model), Some(rbm_section().len() as u64));

            // An edit that grows the virtual section with zeros, which deflate to little.
            let mut grown = rbm_section();
            grown.resize(0x200, 0);
            let held = loose(grown);
            assert!(
                held.len() < 0x200,
                "the loose form is shorter than its sections"
            );
            session.set_bytes_for(model, held).expect("held");
            // The first look inflates nothing on the drawing thread: a worker measures the sections.
            assert_eq!(size(&session, model), None);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while size(&session, model).is_none() {
                assert!(std::time::Instant::now() < deadline, "never measured");
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert_eq!(size(&session, model), Some(0x200));
            assert_eq!(listed_bytes(&session, model), 0x200);

            // A 24-byte row's held body is its own length.
            let named = row(&session, "named.bin");
            session
                .set_bytes_for(named, b"plain".to_vec())
                .expect("held");
            assert_eq!(size(&session, named), Some(5));
            assert_eq!(listed_bytes(&session, named), 5);
        }

        #[test]
        fn a_folder_of_two_nested_gta_vi_archives_totals_their_two_sizes_once() {
            let _serial = crate::ui::theme::serial();
            let _reading = reading(&["a.rpf", "b.rpf"]);
            let dir = tempfile::tempdir().expect("a scratch directory");
            let (a, b) = (
                built(&[chunked("x/one.bin")]),
                built(&[chunked("x/two.bin")]),
            );
            let stored = |len: usize| len.next_multiple_of(16) as u64;
            let expected = stored(a.len()) + stored(b.len());
            let entries = [
                ("a.rpf".to_owned(), Rpf9NewBody::Archive(a)),
                ("b.rpf".to_owned(), Rpf9NewBody::Archive(b)),
            ];
            let session = tree(dir.path(), built(&entries));
            let rows = session.children_of(session.root_id());
            assert_eq!(rows.len(), 2, "two rows, each a nested archive");
            let shown: u64 = rows.iter().filter_map(|&id| size(&session, id)).sum();
            let total: u64 = rows.iter().map(|&id| listed_bytes(&session, id)).sum();
            assert_eq!((shown, total), (expected, expected));
        }

        #[test]
        fn a_gta_vi_rows_columns_say_what_its_row_states_and_its_table_is_enciphered() {
            let _serial = crate::ui::theme::serial();
            let _reading = reading(&["data/named.bin"]);
            let dir = tempfile::tempdir().expect("a scratch directory");
            let session = tree(dir.path(), built(&[chunked("data/named.bin")]));
            let named = row(&session, "named.bin");
            let RpfNodeBacking::ArchiveFile {
                archive_id,
                entry_index,
                ..
            } = &session.node(named).expect("a row").backing
            else {
                panic!("a file of the archive");
            };
            let archive = &session.archive(*archive_id).expect("its archive").archive;
            let stated = archive.rpf9_row(*entry_index).expect("its row");
            let facts = row_facts(&session, named).expect("its columns");
            assert_eq!(facts.kind, stated.kind);
            assert_eq!(
                (facts.size, facts.stored),
                (Some(stated.real), Some(stated.stored))
            );
            assert_eq!(listed_bytes(&session, named), stated.real);
            assert!(!stated.ciphered, "a new body is stored clear");
            assert_eq!(facts.encryption, Some(tr!("None")));
            assert_eq!(archive_encryption(archive), tr!("Enciphered"));
            let mut gta5 = or_io::container::rpf::RpfBuilder::default();
            gta5.insert_with("a.txt", b"a".to_vec(), false)
                .expect("a file");
            let gta5 = gta5.build().expect("a GTA V archive");
            assert_eq!(archive_encryption(&gta5), tr!("None"));
        }

        /// Every file row of the build's smallest archives shows the kind, sizes and cipher its
        /// table's row states, as Details does, a ciphered row among them.
        #[test]
        fn every_row_of_the_builds_smallest_archives_shows_what_its_row_states() {
            const WHAT: &str = "a GTA VI row's columns say what its row states";
            const ARCHIVES: usize = 16;
            let _serial = crate::ui::theme::serial();
            let build_root = std::env::var_os("OPENRAGE_GTA6_BUILD").map(PathBuf::from);
            let (Some(build_root), Ok(keys)) =
                (build_root, crate::state::rpf9_scan::rpf9_table::keys())
            else {
                or_test_support::skip_install(
                    WHAT,
                    "set OPENRAGE_GTA6_BUILD and OPENRAGE_GTA6_KEYS",
                );
                return;
            };
            let _reading = gta6_session::hold(Gta6Context {
                keys,
                names: crate::state::rpf9_scan::rpf9_table::resolver(),
            });
            let mut disks: Vec<(u64, PathBuf)> = std::fs::read_dir(&build_root)
                .expect("the build root lists")
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("rpf")))
                .filter_map(|p| Some((std::fs::metadata(&p).ok()?.len(), p)))
                .collect();
            disks.sort();
            let archive_kind = kind_label(Some(ARCHIVE_CLASS), None);
            let (mut rows, mut ciphered) = (0usize, 0usize);
            for (_, disk) in disks.into_iter().take(ARCHIVES) {
                let archive = RpfArchive::open_with(&disk, gta6_session::crypto())
                    .unwrap_or_else(|e| panic!("{} opens: {e}", disk.display()));
                let session = RpfEditSession::from_archive(archive).expect("the tree lists");
                for id in session.all_nodes() {
                    let Some(RpfNodeBacking::ArchiveFile {
                        archive_id,
                        entry_index,
                        ..
                    }) = session.node(id).map(|n| &n.backing)
                    else {
                        continue;
                    };
                    let archive = &session.archive(*archive_id).expect("its archive").archive;
                    let Some(stated) = archive.rpf9_row(*entry_index) else {
                        continue;
                    };
                    let facts = row_facts(&session, id).expect("its columns");
                    let at = format!("{} row {entry_index}", disk.display());
                    if stated.kind != archive_kind {
                        assert_eq!(facts.kind, stated.kind, "{at}");
                    }
                    assert_eq!(
                        (facts.size, facts.stored),
                        (Some(stated.real), Some(stated.stored)),
                        "{at}"
                    );
                    assert_eq!(measured_bytes(&session, id), Some(stated.real), "{at}");
                    let shown = facts.encryption == Some(tr!("Enciphered"));
                    assert_eq!(shown, stated.ciphered, "{at}");
                    rows += 1;
                    ciphered += usize::from(stated.ciphered);
                }
            }
            assert!(rows > 0, "the smallest archives list file rows");
            if ciphered == 0 {
                or_test_support::skip_install(
                    WHAT,
                    format_args!(
                        "no ciphered row among the {rows} rows of the {ARCHIVES} smallest archives"
                    ),
                );
            }
        }

        /// A build root's top-level archive whose root holds a nested archive past 4 GiB: the
        /// largest under 64 GiB, so its root's nested archives are each opened once.
        #[test]
        fn a_nested_gta_vi_archive_past_4_gib_counts_its_whole_size() {
            const WHAT: &str = "a GTA VI root's total counts a nested archive past 4 GiB whole";
            let _serial = crate::ui::theme::serial();
            let build_root = std::env::var_os("OPENRAGE_GTA6_BUILD").map(PathBuf::from);
            let (Some(build_root), Ok(keys)) =
                (build_root, crate::state::rpf9_scan::rpf9_table::keys())
            else {
                or_test_support::skip_install(
                    WHAT,
                    "set OPENRAGE_GTA6_BUILD and OPENRAGE_GTA6_KEYS",
                );
                return;
            };
            let _reading = gta6_session::hold(Gta6Context {
                keys,
                names: crate::state::rpf9_scan::rpf9_table::resolver(),
            });
            let largest = std::fs::read_dir(&build_root)
                .expect("the build root lists")
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("rpf")))
                .filter_map(|p| Some((std::fs::metadata(&p).ok()?.len(), p)))
                .filter(|(len, _)| *len < 64 << 30)
                .max()
                .map(|(_, p)| p)
                .expect("a top-level archive");
            let archive =
                RpfArchive::open_with(&largest, gta6_session::crypto()).expect("the archive opens");
            let session = RpfEditSession::from_archive(archive).expect("the tree lists");
            let rows = session.children_of(session.root_id());
            let sizes: Vec<u64> = rows.iter().filter_map(|&id| size(&session, id)).collect();
            assert!(
                sizes.iter().any(|&s| s > u64::from(u32::MAX)),
                "a row past 4 GiB"
            );
            let total: u64 = rows.iter().map(|&id| listed_bytes(&session, id)).sum();
            assert_eq!(total, sizes.iter().sum::<u64>());
            // Measured on the build 1.0.2.1: 103 rows, whose tables state these stored sizes.
            assert_eq!((rows.len(), total), (103, 28_543_219_104));
        }
    }

    #[test]
    fn a_plain_gta_vi_file_is_named_by_its_head_and_another_kind_keeps_its_name() {
        let plain = || or_io::container::rpf::gta6_kinds::PLAIN_FILE.to_string();
        assert_eq!(kind_by_head(plain(), b"PSIN\0\0\0\x10"), "PSO");
        assert_eq!(kind_by_head(plain(), b"RBF0\x01"), "RBF");
        assert_eq!(kind_by_head(plain(), b"\xEF\xBB\xBF  <CData>"), tr!("XML"));
        assert_eq!(kind_by_head(plain(), b"a = 1\n"), tr!("text"));
        assert_eq!(kind_by_head(plain(), b"\x00\x01\x02"), plain());
        assert_eq!(kind_by_head("drawable".into(), b"PSIN"), "drawable");
    }

    fn stated(kind: Cow<'static, str>, ciphered: bool) -> Rpf9Row {
        Rpf9Row {
            kind,
            stored: 0x1_2345_6780,
            real: 0x2_0000_0000,
            ciphered,
            row_bytes: Some(20),
            hash: None,
            name64: 1,
        }
    }

    #[test]
    fn a_gta_vi_rows_sizes_past_4_gib_and_its_cipher_are_its_rows() {
        let mut facts = RowFacts {
            size: Some(u64::from(u32::MAX)),
            stored: Some(7),
            encryption: Some(tr!("None")),
            ..Default::default()
        };
        take_row(&mut facts, &stated("drawable".into(), true));
        assert_eq!(
            (facts.size, facts.stored, facts.encryption),
            (
                Some(0x2_0000_0000),
                Some(0x1_2345_6780),
                Some(tr!("Enciphered"))
            )
        );
        take_row(&mut facts, &stated("drawable".into(), false));
        assert_eq!(facts.encryption, Some(tr!("None")));
    }

    #[test]
    fn a_gta_vi_row_no_list_names_is_shown_as_details_states_it() {
        for kind in [
            kind_label(Some(0x7F), None),
            kind_label(None, Some(0x99)),
            kind_label(None, None),
        ] {
            let shown = stated_kind(Some(stated(kind.clone(), false)));
            assert_eq!(shown.as_deref(), Some(&*kind));
        }
        let archive = stated(kind_label(Some(ARCHIVE_CLASS), None), false);
        assert_eq!(stated_kind(Some(archive)), None, "no archive, so a file");
        assert_eq!(stated_kind(None), None);
    }

    #[test]
    fn an_rpf9_body_takes_or_io_s_word_over_its_compressor() {
        assert_eq!(compression_label(Some(false), true), tr!("None"));
        assert_eq!(compression_label(Some(true), false), tr!("Deflate"));
        assert_eq!(compression_label(None, true), tr!("Deflate"));
        assert_eq!(compression_label(None, false), tr!("None"));
    }
}
