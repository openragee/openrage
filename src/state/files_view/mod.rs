//! What the Files view in the centre says about a row and its folder, and the
//! moves only it makes: going to a typed path, reading a folder again and
//! exporting a row under a name of the user's.

pub mod pictures;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use or_io::container::resource::{ResourceEnvelope, Rsc7};
use or_io::container::rpf::{RpfEncryption, RpfEntry};

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
    /// The index and original 64-bit name hash carried by an RPF9 table row.
    pub rpf9_entry: Option<(u32, u64)>,
}

/// What a row is, in words.
pub fn type_in_words(node: &RpfSessionNode) -> String {
    if matches!(node.backing, RpfNodeBacking::Rpf9Dir { .. }) {
        return tr!("Folder").to_string();
    }
    if let Some(kind) = node.rpf9_type() {
        return rpf9_type_name(kind).to_string();
    }
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

/// The content family identified by an RPF9 table or the bytes of its entry.
pub(crate) fn rpf9_type_name(kind: or_io::container::rpf::Gta6EntryType) -> &'static str {
    use or_io::container::rpf::Gta6EntryType;
    match kind {
        Gta6EntryType::Unknown => tr!("Unknown"),
        Gta6EntryType::Archive => tr!("Archive"),
        Gta6EntryType::TextureDictionary => tr!("Texture dictionary"),
        Gta6EntryType::Awc => DocumentKind::Awc.info().noun,
        Gta6EntryType::Pso => DocumentKind::Meta.info().noun,
        Gta6EntryType::Resource(role) => super::gta6_resource::role_name(Some(role)),
        Gta6EntryType::Body(body) => super::gta6_other_bodies::label(body),
        Gta6EntryType::Rbf => tr!("RBF metadata"),
        Gta6EntryType::File(extension) => {
            rpf9_file_kind(extension).map_or(extension, |kind| kind.info().noun)
        }
    }
}

pub(crate) fn rpf9_file_kind(extension: &str) -> Option<DocumentKind> {
    match extension {
        "bk2" => Some(DocumentKind::Bik),
        "yad" | "ypm" | "ypmd" | "ymmd" | "yrd" => Some(DocumentKind::Gta6Anim),
        "yscript" | "yascript" => Some(DocumentKind::Registered(
            or_io::container::resource::ResourceType::Ysc,
        )),
        "ysp" => Some(DocumentKind::Ynv),
        "yascd" => Some(DocumentKind::Meta),
        _ => DocumentKind::from_path(Path::new(&format!("entry.{extension}"))),
    }
}

/// Add a type suffix for display without changing a recovered name or a hash.
pub(crate) fn rpf9_display_name(
    name: &str,
    kind: or_io::container::rpf::Gta6EntryType,
) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;

    let extension = kind.extension();
    let existing = Path::new(name).extension().and_then(|s| s.to_str());
    if existing.is_some_and(|ext| extension == "bin" || ext.eq_ignore_ascii_case(extension)) {
        Cow::Borrowed(name)
    } else {
        Cow::Owned(format!("{name}.{extension}"))
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
        kind: type_in_words(node),
        ..Default::default()
    };
    if session.is_rpf9(id) {
        facts.edition = Some(or_title::Game::GtaVI.profile().label());
        facts.size = session.size_for(id).or_else(|| session.archive_size(id));
        facts.rpf9_entry = node.rpf9_entry();
        let source = match &node.backing {
            RpfNodeBacking::Rpf9File { table, base, index } => Some((table, *base, *index)),
            RpfNodeBacking::Rpf9Archive {
                table,
                owner: Some((base, index)),
                ..
            } => Some((table, *base, *index)),
            _ => None,
        };
        if let Some((table, base, index)) = source {
            let tree = table.tree();
            if let Some(archive) = tree.nodes.get(&base).map(|node| &node.archive) {
                if let Some(entry) = archive.entries.get(index as usize) {
                    facts.stored = entry.stored.or_else(|| {
                        archive
                            .entry_extra(index as usize)?
                            .nested
                            .map(|row| row.stored_size())
                    });
                    facts.encryption = Some(if entry.cipher.is_some() {
                        tr!("Encrypted")
                    } else {
                        tr!("None")
                    });
                }
            }
        }
        return Some(facts);
    }
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
                    facts.compression = Some(if b.is_compressed() {
                        tr!("Deflate")
                    } else {
                        tr!("None")
                    });
                    facts.encryption = Some(if b.encrypted {
                        encryption_label(archive.header.encryption)
                    } else {
                        tr!("None")
                    });
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
                facts.size = Some(edited.len() as u64);
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
                        facts.size = Some(u64::from(b.uncompressed_size));
                        facts.stored = Some(u64::from(b.read_size()));
                        facts.compression = Some(if b.is_compressed() {
                            tr!("Deflate")
                        } else {
                            tr!("None")
                        });
                    }
                }
                ArchiveSource::Disk(None) => {}
            }
            // ⚠ An archive's own encryption is its table's, known once it is open.
            facts.encryption = archive_id
                .and_then(|a| session.archive(a))
                .map(|slot| encryption_label(slot.archive.header.encryption));
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
            if rpf.session.is_rpf9(child) {
                rpf.session.unload_archive(child);
            }
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
