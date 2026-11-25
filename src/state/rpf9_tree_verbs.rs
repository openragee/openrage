//! A GTA VI archive opened as the tree takes the tree's own verbs: a row extracts in the loose form
//! OpenRage reopens, and the tree's edits save through the GTA VI writer to a new file, which the game
//! refuses because only its maker can sign the table.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::File;
use std::io::{BufWriter, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use or_io::container::rpf::gta6_container_write::Rpf9NewBody;
use or_io::container::rpf::{
    read_rpf9_body, write_rpf9_resource, BodyForm, Gta6Class, Gta6Mount, GtaViKeys,
    Rpf9ArchiveWriter, Rpf9Sections, RpfEntry, RpfVersion, RPF9_MAGIC, RPF9_SLOT,
};
use or_io::container::RpfArchive;

use crate::state::document::{ArchiveId, DocumentId, RpfNodeId};
use crate::state::extract_job::{plan, ExtractItem, TypeFilter};
use crate::state::rpf_node::{ArchiveSource, RpfNodeBacking};
use crate::state::rpf_session::RpfEditSession;
use crate::state::{AppState, DocPayload};

/// Where a save finds the keys the archive's tables open with.
type Keys = fn() -> Result<Arc<GtaViKeys>, String>;

/// How a save of a tree ended: its document, the edit it was at, the new file, and whether a table
/// and whether a body changed in it, or why it was refused.
type Ended = (DocumentId, u64, PathBuf, Result<(bool, bool), String>);

// ponytail: one list for the process, emptied by the next frame's poll.
static ENDED: Mutex<Vec<Ended>> = Mutex::new(Vec::new());

fn ended() -> MutexGuard<'static, Vec<Ended>> {
    ENDED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A clear headerless resource of a GTA VI tree, decoded, in the loose form OpenRage reopens; `None`
/// for any other entry or form, which reads as every archive's does.
pub(in crate::state) fn loose_body(
    archive: &RpfArchive,
    entry_index: u32,
    form: BodyForm,
) -> Option<Vec<u8>> {
    let place =
        (archive.rpf9_place(entry_index)).filter(|p| form == BodyForm::Decoded && !p.ciphered)?;
    let row = archive.binary_entry(entry_index)?.file_offset;
    // Only a headerless resource's body follows a zero slot past its row's start.
    if place.offset != place.base + row + RPF9_SLOT {
        return None;
    }
    let file = File::open(&place.file).ok()?;
    let (find, body) = read_rpf9_body(file, place.offset, u64::MAX).ok()?;
    write_rpf9_resource(&Rpf9Sections::of(&find, &body)?).ok()
}

/// The top archive of a tree opened from a GTA VI archive on its own.
fn top_rpf9(session: &RpfEditSession) -> Option<&Arc<RpfArchive>> {
    let RpfNodeBacking::ArchiveDir { archive_id } = &session.node(session.root_id())?.backing
    else {
        return None;
    };
    let archive = &session.archive(*archive_id)?.archive;
    (archive.version == RpfVersion::Rpf9).then_some(archive)
}

fn named(archive: &RpfArchive) -> String {
    (archive.path()).map_or_else(String::new, crate::state::jobs::file_name)
}

/// Why a GTA VI tree is not written into the archive it was read from; `None` for any other tree.
pub(in crate::state) fn refused_tree(session: &RpfEditSession) -> Option<String> {
    let file = named(top_rpf9(session)?);
    Some(tr!(
        "{file} is a GTA VI archive, which is never written in place: Save as… writes its edits to a new file, which the game refuses because its signature covers the table.",
        file = file
    ))
}

/// Refuse writing a GTA VI archive of a folder's tree in place, saying why.
pub(in crate::state) fn refuse_in_place(archive: &RpfArchive) -> Result<(), String> {
    if archive.version != RpfVersion::Rpf9 {
        return Ok(());
    }
    Err(tr!(
        "{file} is a GTA VI archive, which is never written in place: open it on its own and Save as… writes its edits to a new file, which the game refuses because its signature covers the table.",
        file = named(archive)
    ))
}

/// One change a tree holds to a GTA VI archive, by the absolute offset its entry's body starts at.
enum Edit {
    Replace {
        at: u64,
        body: Arc<Vec<u8>>,
    },
    Rename {
        at: u64,
        name: String,
    },
    Remove {
        at: u64,
    },
    /// An entry `name` added to the archive whose header is at `archive`.
    Add {
        archive: u64,
        name: String,
        body: ExtractItem,
    },
}

/// The edits the tree `session` holds to its top GTA VI archive and every archive opened in it.
fn tree_edits(session: &RpfEditSession) -> Result<Vec<Edit>, String> {
    let root = session.root_id();
    let Some(RpfNodeBacking::ArchiveDir { archive_id }) = session.node(root).map(|n| &n.backing)
    else {
        return Err(tr!("the tree is no GTA VI archive opened on its own").to_string());
    };
    let mut edits = Vec::new();
    archive_edits(session, *archive_id, root, 0, &mut edits)?;
    Ok(edits)
}

/// The edits of the archive `archive_id` whose header is at `base`, read under the row `root`.
fn archive_edits(
    session: &RpfEditSession,
    archive_id: ArchiveId,
    root: RpfNodeId,
    base: u64,
    edits: &mut Vec<Edit>,
) -> Result<(), String> {
    let archive = &(session.archive(archive_id))
        .ok_or_else(|| tr!("an archive of the tree is no longer open").to_string())?
        .archive;
    let prefix = (session.path_of(root))
        .filter(|p| !p.is_empty())
        .map(|p| p + "/")
        .unwrap_or_default();
    let start_of = |index: u32| {
        (archive.binary_entry(index))
            .filter(|e| e.file_offset != 0)
            .map(|e| base + e.file_offset)
            .ok_or_else(|| tr!("entry {index} states no start", index = index).to_string())
    };
    let renamed = |index: u32, name: &str| archive.entry_path(index).as_deref() != Some(name);
    let mut kept: HashSet<u32> = HashSet::new();
    let mut stack = session.children_of(root);
    while let Some(id) = stack.pop() {
        let Some(node) = session.node(id) else {
            continue;
        };
        let path = session.path_of(id).unwrap_or_default();
        let name = path.strip_prefix(&prefix).unwrap_or(&path).to_owned();
        // A second row of one entry is a copy of it, which is added.
        let own = match &node.backing {
            RpfNodeBacking::ArchiveFile {
                archive_id: from,
                entry_index,
                ..
            } if *from == archive_id => kept.insert(*entry_index),
            _ => false,
        };
        match &node.backing {
            RpfNodeBacking::ArchiveFile {
                entry_index,
                override_bytes,
                ..
            } if own => {
                // An entry with no row has no body to place, so only an edited one is refused.
                let is_renamed = renamed(*entry_index, &name);
                if override_bytes.is_none() && !is_renamed {
                    continue;
                }
                let at = (archive.rpf9_place(*entry_index))
                    .map(|place| place.offset)
                    .ok_or_else(|| tr!("{name} has no body of its own", name = name).to_string())?;
                if let Some(body) = override_bytes {
                    edits.push(Edit::Replace {
                        at,
                        body: Arc::clone(body),
                    });
                }
                if is_renamed {
                    edits.push(Edit::Rename { at, name });
                }
            }
            RpfNodeBacking::Archive {
                archive_id: opened,
                source:
                    ArchiveSource::Nested {
                        parent_archive_id,
                        parent_entry_index,
                        ..
                    },
                scanned,
                ..
            } if *parent_archive_id == archive_id => {
                kept.insert(*parent_entry_index);
                let at = start_of(*parent_entry_index)?;
                if renamed(*parent_entry_index, &name) {
                    edits.push(Edit::Rename { at, name });
                }
                // An archive the tree has not opened holds no edit.
                if let (Some(nested), true) = (opened, scanned) {
                    archive_edits(session, *nested, id, at, edits)?;
                }
            }
            RpfNodeBacking::Archive { .. } => {
                let why = tr!(
                    "{name} is an archive moved in from another, which the GTA VI writer does not add",
                    name = name
                );
                return Err(why);
            }
            RpfNodeBacking::ArchiveFile { .. }
            | RpfNodeBacking::InMemoryFile { .. }
            | RpfNodeBacking::DiskFile { .. } => {
                let body = (plan(session, id, &TypeFilter::All).pop())
                    .ok_or_else(|| tr!("{name} holds no bytes", name = name).to_string())?;
                edits.push(Edit::Add {
                    archive: base,
                    name,
                    body,
                });
            }
            _ => stack.extend(session.children_of(id)),
        }
    }
    for (index, entry) in (0u32..).zip(&archive.entries) {
        if !matches!(entry, RpfEntry::Binary(_)) || kept.contains(&index) {
            continue;
        }
        let at = match archive.rpf9_place(index) {
            Some(place) => place.offset,
            None => start_of(index)?,
        };
        edits.push(Edit::Remove { at });
    }
    Ok(())
}

/// The body an added entry is stored as: an archive as it is, anything else in chunks.
fn new_body(bytes: Vec<u8>) -> Rpf9NewBody {
    // ponytail: a loose resource added is stored chunked under a 24-byte row; a 20-byte row needs its
    // type code, which no added file states.
    if bytes.starts_with(&RPF9_MAGIC.to_le_bytes()) {
        Rpf9NewBody::Archive(bytes)
    } else {
        Rpf9NewBody::Chunked(bytes)
    }
}

/// Where each body offset of `wanted` is in the archive `src`: the entry indices from the top-level
/// table to the entry whose body starts there (a headerless resource's past its slot), and whether
/// that entry's row is a headerless resource's.
pub(crate) fn chains<R: Read + Seek>(
    src: &mut R,
    wanted: &BTreeSet<u64>,
    keys: &GtaViKeys,
) -> Result<HashMap<u64, (Vec<u32>, bool)>, String> {
    let mut found: HashMap<u64, (Vec<u32>, bool)> = HashMap::new();
    let root = Gta6Mount::open_root(&mut *src, keys).map_err(|e| e.to_string())?;
    let mut stack = vec![(root, Vec::new())];
    while let Some((mount, above)) = stack.pop() {
        let entries = &mount.archive.entries;
        let extras = mount.archive.entry_extras();
        let headerless = |at: usize| extras.get(at).is_some_and(|x| x.nested.is_some());
        let mut at_offset: HashMap<u64, Vec<usize>> = HashMap::new();
        for (at, entry) in entries.iter().enumerate() {
            let Some(start) = mount.absolute_start(entry) else {
                continue;
            };
            at_offset.entry(start).or_default().push(at);
            // Only a headerless resource's body opens past its slot.
            if headerless(at) {
                at_offset.entry(start + RPF9_SLOT).or_default().push(at);
            }
        }
        let left: Vec<u64> = (wanted.iter().copied())
            .filter(|o| !found.contains_key(o))
            .collect();
        for offset in left {
            match at_offset.get(&offset).map(Vec::as_slice) {
                Some(&[at]) => {
                    let mut chain = above.clone();
                    chain.push(entries[at].index);
                    found.insert(offset, (chain, headerless(at)));
                }
                Some([_, _, ..]) => {
                    return Err(tr!(
                        "more than one entry holds the body at {offset}, so which one to replace is not known",
                        offset = format!("{offset:#x}")
                    ))
                }
                _ => {}
            }
        }
        for entry in entries {
            let (Some(start), Some(len), Some(Gta6Class::NestedArchive)) =
                (mount.absolute_start(entry), entry.stored, entry.kind())
            else {
                continue;
            };
            let inside =
                len > 1 && (wanted.range(start + 1..start + len)).any(|o| !found.contains_key(o));
            if inside {
                let nested = mount.open_nested(&mut *src, entry, keys);
                let mut chain = above.clone();
                chain.push(entry.index);
                stack.push((nested.map_err(|e| e.to_string())?, chain));
            }
        }
    }
    match wanted.iter().find(|o| !found.contains_key(o)) {
        Some(offset) => Err(tr!(
            "no entry of the archive holds the body at {offset}",
            offset = format!("{offset:#x}")
        )),
        None => Ok(found),
    }
}

/// Write the archive `writer` plans from the one at `archive` into the new file `out`; whether a
/// table and whether a body changed. `out` must not exist, and a refused write leaves nothing there.
pub(crate) fn write_plan(
    writer: &Rpf9ArchiveWriter,
    archive: &Path,
    out: &Path,
    keys: &GtaViKeys,
) -> Result<(bool, bool), String> {
    if crate::state::reload_open_file::same_file(out, archive) {
        return Err(tr!(
            "the copy is a new file, and the archive it is read from is never written"
        )
        .to_string());
    }
    let mut file = File::open(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let plan = writer.plan(&mut file, keys).map_err(|e| e.to_string())?;
    // `create_new` refuses any file that exists, so the cleanup below only removes one made here.
    let created = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out)
        .map_err(|e| format!("{}: {e}", out.display()))?;
    let mut sink = BufWriter::new(created);
    let written = (plan
        .write_to(&mut file, &mut sink)
        .map_err(|e| e.to_string()))
    .and_then(|_| sink.flush().map_err(|e| e.to_string()));
    drop(sink);
    if let Err(e) = written {
        let _ = std::fs::remove_file(out);
        return Err(format!("{}: {e}", out.display()));
    }
    Ok((plan.tables_changed > 0, plan.bodies_changed > 0))
}

/// Write the archive at `source` with `edits` made into the new file `out`; whether a table and
/// whether a body changed.
fn write_tree(
    source: &Path,
    edits: &[Edit],
    out: &Path,
    keys: &GtaViKeys,
) -> Result<(bool, bool), String> {
    let wanted: BTreeSet<u64> = (edits.iter())
        .filter_map(|edit| match edit {
            Edit::Replace { at, .. } | Edit::Rename { at, .. } | Edit::Remove { at } => Some(*at),
            // The top archive's header is at 0, and its chain is empty.
            Edit::Add { archive, .. } => Some(*archive).filter(|&at| at != 0),
        })
        .collect();
    let mut file = File::open(source).map_err(|e| format!("{}: {e}", source.display()))?;
    let found = chains(&mut file, &wanted, keys)?;
    drop(file);
    let chain = |at: &u64| found.get(at).cloned().unwrap_or_default();
    let mut writer = Rpf9ArchiveWriter::new();
    for edit in edits {
        match edit {
            Edit::Replace { at, body } => match chain(at) {
                (path, true) => writer.replace_resource(&path, (**body).clone(), None),
                (path, false) => writer.replace(&path, (**body).clone()),
            },
            Edit::Rename { at, name } => writer.rename(&chain(at).0, name),
            Edit::Remove { at } => writer.remove(&chain(at).0),
            Edit::Add {
                archive,
                name,
                body,
            } => {
                let bytes = body.read(BodyForm::Decoded)?;
                writer.add(&chain(archive).0, name, new_body(bytes))
            }
        };
    }
    write_plan(&writer, source, out, keys)
}

impl AppState {
    /// The name Save as… offers a GTA VI archive opened on its own; `None` for any other document.
    pub(crate) fn gta6_tree_copy_name(&self, id: DocumentId) -> Option<String> {
        let DocPayload::Rpf(rpf) = &self.document(id)?.payload else {
            return None;
        };
        let stem = top_rpf9(&rpf.session)?
            .path()?
            .file_stem()?
            .to_string_lossy();
        Some(format!("{stem}_edited.rpf"))
    }

    /// Refuse a save of a GTA VI tree into the archive it was read from, saying why; true when refused.
    pub(in crate::state) fn refused_gta6_tree_in_place(&mut self, doc_id: DocumentId) -> bool {
        let why = match self.document(doc_id).map(|d| &d.payload) {
            Some(DocPayload::Rpf(rpf)) => refused_tree(&rpf.session),
            _ => None,
        };
        let refused = why.is_some();
        if let Some(why) = why {
            self.show_error_toast(why);
        }
        refused
    }

    /// Write a GTA VI tree's edits to the new archive `out` on a worker; `None` for any other document.
    pub(in crate::state) fn save_gta6_tree_as(
        &mut self,
        doc_id: DocumentId,
        out: &Path,
    ) -> Option<bool> {
        self.save_gta6_tree_with(doc_id, out, crate::state::rpf9_scan::rpf9_table::keys)
    }

    fn save_gta6_tree_with(&mut self, doc_id: DocumentId, out: &Path, keys: Keys) -> Option<bool> {
        let doc = self.document(doc_id)?;
        let DocPayload::Rpf(rpf) = &doc.payload else {
            return None;
        };
        let source = top_rpf9(&rpf.session)?.path()?.to_path_buf();
        let at = doc.history.at();
        let refused = if crate::state::reload_open_file::same_file(out, &source) {
            Err(refused_tree(&rpf.session).unwrap_or_default())
        } else {
            (self.no_job_reads(out, "save")).and_then(|()| tree_edits(&rpf.session))
        };
        let edits = match refused {
            Ok(edits) => edits,
            Err(why) => {
                self.show_error_toast(why);
                return Some(false);
            }
        };
        let out = out.to_path_buf();
        std::thread::spawn(move || {
            let said = keys().and_then(|keys| write_tree(&source, &edits, &out, &keys));
            ended().push((doc_id, at, out, said));
        });
        Some(true)
    }

    /// Say how each save of a GTA VI tree ended.
    pub(in crate::state) fn poll_tree_saves(&mut self) {
        let done = std::mem::take(&mut *ended());
        for (doc_id, at, out, said) in done {
            let file = crate::state::jobs::file_name(&out);
            let (tables, bodies) = match said {
                Ok(changed) => changed,
                Err(why) => {
                    self.show_error_toast(tr!(
                        "Cannot save into a copy of the archive: {reason}",
                        reason = why
                    ));
                    continue;
                }
            };
            // The edits are in the new file, so closing the tree loses nothing.
            if let Some(doc) = self.document_mut(doc_id) {
                doc.saved_as_of(at);
            }
            self.file_replaced(&out);
            self.show_toast(match (tables, bodies) {
                (true, _) => tr!(
                    "{file} is a new archive with the tree's edits, which the game refuses because only its maker can sign the table they changed.",
                    file = file
                ),
                (false, true) => tr!(
                    "{file} is a new archive with the edit in its entry's place and its table as it was read.",
                    file = file
                ),
                (false, false) => tr!(
                    "{file} is a new archive the same as the one it was read from: the edit writes the bytes the entry holds.",
                    file = file
                ),
            });
        }
    }
}

#[cfg(test)]
#[path = "rpf9_tree_verbs_tests.rs"]
pub(crate) mod tests;
