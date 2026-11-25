//! An archive session's edits as undo holds them: rows taken out and put back, renamed and moved.
//!
//! ⚠ Only what the session holds until a save: a row on disk is renamed,
//! moved or sent to the Recycle Bin at once, and of that only a paste's cut
//! is undone here (`cut`).

use std::path::{Path, PathBuf};

use super::edit_history::{Change, Swap, GONE};
use super::memory::vec_bytes;
use super::rpf_node::{ArchiveSlot, RpfNodeBacking, RpfSessionNode};
use super::rpf_session::RpfEditSession;
use super::{ArchiveId, DocPayload, DocumentId, RpfNodeId};

/// Rows taken out of the tree, with every archive opened under them.
pub struct Rows {
    parent: RpfNodeId,
    at: usize,
    nodes: Vec<RpfSessionNode>,
    archives: Vec<(ArchiveId, ArchiveSlot)>,
}

impl Rows {
    /// The rows, the bytes they hold in memory and the tables of the archives opened under them.
    fn held_bytes(&self) -> usize {
        let rows: usize = (self.nodes.iter())
            .map(|node| node.row_bytes() + node.backing.memory_bytes())
            .sum();
        let archives: usize = (self.archives.iter())
            .map(|(_, slot)| slot.archive.table_bytes() + slot.memory_bytes())
            .sum();
        vec_bytes(&self.nodes) + rows + vec_bytes(&self.archives) + archives
    }
}

impl RpfEditSession {
    /// Take the row `id` and every row under it out of the tree, as a delete does, to put back later.
    pub(super) fn take_rows(&mut self, id: RpfNodeId) -> Option<Rows> {
        if self.is_rpf9(id) {
            return None;
        }
        let (parent, at) = self.place_of(id)?;
        self.nodes.get_mut(&parent)?.children.remove(at);
        let (mut nodes, mut archives, mut stack) = (Vec::new(), Vec::new(), vec![id]);
        while let Some(next) = stack.pop() {
            let Some(node) = self.nodes.remove(&next) else {
                continue;
            };
            if let RpfNodeBacking::Archive {
                archive_id: Some(archive),
                ..
            } = node.backing
            {
                archives.extend(self.archives.remove(&archive).map(|slot| (archive, slot)));
            }
            if let Ok(mut cache) = self.cache.lock() {
                cache.pop(&next);
            }
            stack.extend(&node.children);
            nodes.push(node);
        }
        self.mark_archive_dirty(parent);
        Some(Rows {
            parent,
            at,
            nodes,
            archives,
        })
    }

    fn put_rows(&mut self, rows: Rows) -> Result<(), Rows> {
        let first = rows.nodes.first().map(|node| node.id);
        let (Some(first), Some(parent)) = (first, self.nodes.get_mut(&rows.parent)) else {
            return Err(rows);
        };
        parent
            .children
            .insert(rows.at.min(parent.children.len()), first);
        let (parent, nodes, archives) = (rows.parent, rows.nodes, rows.archives);
        self.nodes
            .extend(nodes.into_iter().map(|node| (node.id, node)));
        self.archives.extend(archives);
        self.mark_archive_dirty(parent);
        Ok(())
    }

    /// The row `id`'s parent, and its place among the parent's rows.
    pub(super) fn place_of(&self, id: RpfNodeId) -> Option<(RpfNodeId, usize)> {
        let parent = self.nodes.get(&id)?.parent?;
        let at = (self.nodes.get(&parent)?.children.iter()).position(|c| *c == id)?;
        Some((parent, at))
    }

    fn put_at(&mut self, id: RpfNodeId, parent: RpfNodeId, at: usize) -> bool {
        let Some((old, _)) = self.place_of(id) else {
            return false;
        };
        if !self.nodes.get(&parent).is_some_and(|p| p.is_dir()) {
            return false;
        }
        if let Some(old) = self.nodes.get_mut(&old) {
            old.children.retain(|c| *c != id);
        }
        if let Some(siblings) = self.nodes.get_mut(&parent).map(|p| &mut p.children) {
            siblings.insert(at.min(siblings.len()), id);
        }
        if let Some(node) = self.nodes.get_mut(&id) {
            node.parent = Some(parent);
        }
        self.mark_archive_dirty(old);
        self.mark_archive_dirty(parent);
        true
    }

    /// Mark unsaved exactly the archives `marked` names, as they stood before a step that changed nothing.
    pub(super) fn mark_unsaved_only(&mut self, marked: &[ArchiveId]) {
        for (id, slot) in &mut self.archives {
            slot.dirty = marked.contains(id);
        }
    }
}

fn session(payload: &mut DocPayload) -> Option<&mut RpfEditSession> {
    match payload {
        DocPayload::Rpf(rpf) => Some(&mut rpf.session),
        _ => None,
    }
}

fn step(part: impl Swap + 'static) -> Change {
    Change::Part {
        key: None,
        parts: vec![Box::new(part)],
    }
}

/// The row `id`, which the edit added when `held` is `None` and took out as `held` otherwise.
struct Row {
    id: RpfNodeId,
    held: Option<Rows>,
}

impl Swap for Row {
    fn held_bytes(&self) -> usize {
        self.held.as_ref().map_or(0, Rows::held_bytes)
    }

    fn reads_an_archive(&self) -> bool {
        let mut rows = self.held.iter().flat_map(|rows| &rows.nodes);
        rows.any(|node| {
            matches!(
                node.backing,
                RpfNodeBacking::ArchiveFile { .. }
                    | RpfNodeBacking::ArchiveDir { .. }
                    | RpfNodeBacking::Archive { .. }
            )
        })
    }

    fn swap(&mut self, payload: &mut DocPayload) -> bool {
        let Some(session) = session(payload) else {
            return false;
        };
        match self.held.take() {
            Some(rows) => session
                .put_rows(rows)
                .map_err(|rows| self.held = Some(rows))
                .is_ok(),
            None => {
                self.held = session.take_rows(self.id);
                self.held.is_some()
            }
        }
    }

    fn why_not(&self) -> String {
        match self.held {
            Some(_) => tr!("the folder its rows go back into is no longer in the tree"),
            None => tr!("a row it changed is no longer in the tree"),
        }
        .to_string()
    }
}

/// The step that takes out the row `id` an edit added.
pub(super) fn added(id: RpfNodeId) -> Change {
    step(Row { id, held: None })
}

/// The step that puts back `rows`, which a delete took out.
pub(super) fn deleted(id: RpfNodeId, rows: Rows) -> Change {
    step(Row {
        id,
        held: Some(rows),
    })
}

/// The row `id`, named `other` before the rename.
struct Renamed {
    id: RpfNodeId,
    other: String,
}

impl Swap for Renamed {
    fn held_bytes(&self) -> usize {
        self.other.capacity()
    }

    fn swap(&mut self, payload: &mut DocPayload) -> bool {
        let Some(session) = session(payload) else {
            return false;
        };
        let Some(now) = session.node(self.id).map(|node| node.name.clone()) else {
            return false;
        };
        let renamed = session.rename_node(self.id, &self.other).is_ok();
        if renamed {
            self.other = now;
        }
        renamed
    }

    fn why_not(&self) -> String {
        let other = &self.other;
        tr!(
            "a row it renamed is no longer in the tree, or another row there is called {other}",
            other = other
        )
    }
}

pub(super) fn renamed(id: RpfNodeId, other: String) -> Change {
    step(Renamed { id, other })
}

/// The row `id`, which sat at `at` among `parent`'s rows before the move.
struct Moved {
    id: RpfNodeId,
    parent: RpfNodeId,
    at: usize,
}

impl Swap for Moved {
    fn swap(&mut self, payload: &mut DocPayload) -> bool {
        let Some(session) = session(payload) else {
            return false;
        };
        let Some(now) = session.place_of(self.id) else {
            return false;
        };
        let moved = session.put_at(self.id, self.parent, self.at);
        if moved {
            (self.parent, self.at) = now;
        }
        moved
    }

    fn why_not(&self) -> String {
        tr!("a row it moved, or the folder it goes back into, is no longer in the tree").to_string()
    }
}

pub(super) fn moved(id: RpfNodeId, (parent, at): (RpfNodeId, usize)) -> Change {
    step(Moved { id, parent, at })
}

/// Each file and folder on disk a cut row was and where its paste put the copy.
type Copies = Vec<(PathBuf, Option<RpfNodeId>)>;

/// A row on disk that a paste cut into an archive, sent to the Recycle Bin once its copy landed.
///
/// ⚠ Undone, its files are written back from that copy rather than taken out
/// of the bin, which no call restores from and which outside Windows deletes
/// for good. The copy holds the bytes, in memory until its archive is saved
/// and in the archive after, so this part holds none.
struct Cut {
    /// Folders before what they hold, each file with its copy.
    files: Copies,
    /// The row with the rows under it, held while the paste stands; `None` when it is another document's.
    row: Option<Row>,
    /// Whether the paste stands: the files sent to the bin and the row out of the tree.
    sent: bool,
    /// The other document the row was cut from, whose tree cannot take the row back.
    from: Option<DocumentId>,
    /// Whether the bin left the files where they were, as a self-check's stand-in does.
    left: bool,
    why: String,
}

impl Swap for Cut {
    fn held_bytes(&self) -> usize {
        let files: usize = self.files.iter().map(|(path, _)| path.capacity()).sum();
        let row = self.row.as_ref().map_or(0, |row| row.held_bytes());
        vec_bytes(&self.files) + files + row + self.why.capacity()
    }

    fn swap(&mut self, payload: &mut DocPayload) -> bool {
        let done = match self.sent {
            true => self.write_back(payload),
            false => self.send_away(payload),
        };
        match done {
            Ok(()) => {
                self.sent = !self.sent;
                true
            }
            Err(why) => {
                self.why = why;
                false
            }
        }
    }

    fn why_not(&self) -> String {
        self.why.clone()
    }

    fn on_disk(&self) -> Option<&Path> {
        self.files.first().map(|(top, _)| top.as_path())
    }

    fn leaves_a_tree_stale(&self) -> Option<DocumentId> {
        self.from
    }
}

impl Cut {
    /// Write the files back where they were, unless the bin left them there, and put the row back.
    fn write_back(&mut self, payload: &mut DocPayload) -> Result<(), String> {
        let mut made = 0;
        let mut done = match self.left {
            true => Ok(()),
            false => self.write(payload, &mut made),
        };
        if done.is_ok() && self.row.as_mut().is_some_and(|row| !row.swap(payload)) {
            done = Err(tr!("the folder it was cut from is no longer in the tree").to_string());
        }
        if done.is_err() {
            for (path, copy) in self.files[..made].iter().rev() {
                let _ = match copy {
                    Some(_) => std::fs::remove_file(path),
                    None => std::fs::remove_dir(path),
                };
            }
        }
        done
    }

    /// Write each folder and file from its copy, counting in `made` those written.
    fn write(&self, payload: &DocPayload, made: &mut usize) -> Result<(), String> {
        let DocPayload::Rpf(rpf) = payload else {
            return Err(crate::i18n::tr(GONE).to_string());
        };
        if let Some((top, _)) =
            (self.files.first()).filter(|(top, _)| top.symlink_metadata().is_ok())
        {
            return Err(tr!("`{top}` already exists", top = top.display()));
        }
        for (path, copy) in &self.files {
            let written = match copy {
                None => std::fs::create_dir(path),
                Some(copy) => {
                    let bytes = (rpf.session.bytes_for(*copy)).ok_or_else(|| {
                        tr!(
                            "the copy of `{path}` in the archive no longer reads",
                            path = path.display()
                        )
                    })?;
                    std::fs::write(path, &*bytes)
                }
            };
            written.map_err(|e| format!("`{}`: {e}", path.display()))?;
            *made += 1;
        }
        Ok(())
    }

    /// Take the row out of the tree and send its files to the Recycle Bin, as the paste did.
    fn send_away(&mut self, payload: &mut DocPayload) -> Result<(), String> {
        let Some((top, _)) = self.files.first() else {
            return Err(crate::i18n::tr(GONE).to_string());
        };
        if self.row.as_mut().is_some_and(|row| !row.swap(payload)) {
            return Err(tr!("its row is no longer in the tree").to_string());
        }
        if let Err(e) = crate::platform::recycle_bin::recycle(top) {
            if let Some(row) = self.row.as_mut() {
                row.swap(payload);
            }
            return Err(tr!(
                "`{top}` could not go to the Recycle Bin: {e}",
                e = e,
                top = top.display()
            ));
        }
        self.left = top.symlink_metadata().is_ok();
        Ok(())
    }
}

/// Each file and folder at `path` on disk, folders first, with the row under `folder` its paste made the copy; `None` where one has no copy there.
pub(super) fn copies(session: &RpfEditSession, folder: RpfNodeId, path: &Path) -> Option<Copies> {
    let name = path.file_name()?.to_str()?.trim();
    let copy = (session.children_of(folder).into_iter()).find(|&row| {
        session
            .node(row)
            .is_some_and(|node| node.name.eq_ignore_ascii_case(name))
    })?;
    if !path.is_dir() {
        return Some(vec![(path.to_path_buf(), Some(copy))]);
    }
    let mut out = vec![(path.to_path_buf(), None)];
    for entry in std::fs::read_dir(path).ok()? {
        out.extend(copies(session, copy, &entry.ok()?.path())?);
    }
    Some(out)
}

/// The part of a paste's step that brings back a row cut from disk where `files` were, `taken` from the tree when it is the paste's own; `None` where the row held an opened archive, whose delete empties the history.
pub(super) fn cut(
    files: Copies,
    taken: Option<(RpfNodeId, Rows)>,
    from: Option<DocumentId>,
) -> Option<Change> {
    if taken
        .as_ref()
        .is_some_and(|(_, rows)| !rows.archives.is_empty())
    {
        return None;
    }
    let left = (files.first()).is_some_and(|(top, _)| top.symlink_metadata().is_ok());
    let row = taken.map(|(id, rows)| Row {
        id,
        held: Some(rows),
    });
    Some(step(Cut {
        files,
        row,
        sent: true,
        from,
        left,
        why: String::new(),
    }))
}

/// A file an edit wrote into a folder on disk, with the row that shows it, while the edit stands.
///
/// ⚠ Taken back, the file is deleted only if it still holds what was written, so a change made to it since is not lost with it: a part that cannot finish changes nothing. Its bytes stay here for a redo to write again.
struct Written {
    path: PathBuf,
    bytes: Vec<u8>,
    /// The row that shows the file, held while the file is out.
    row: Row,
    /// Whether the file is on disk and in the tree.
    here: bool,
    why: String,
}

impl Swap for Written {
    fn held_bytes(&self) -> usize {
        self.path.capacity() + self.bytes.capacity() + self.row.held_bytes() + self.why.capacity()
    }

    fn swap(&mut self, payload: &mut DocPayload) -> bool {
        let done = match self.here {
            true => self.take_back(payload),
            false => self.write_again(payload),
        };
        match done {
            Ok(()) => {
                self.here = !self.here;
                true
            }
            Err(why) => {
                self.why = why;
                false
            }
        }
    }

    fn why_not(&self) -> String {
        self.why.clone()
    }

    fn on_disk(&self) -> Option<&Path> {
        Some(&self.path)
    }
}

impl Written {
    /// Delete the file when it is as it was written, and take its row out of the tree.
    fn take_back(&mut self, payload: &mut DocPayload) -> Result<(), String> {
        match std::fs::read(&self.path) {
            Ok(now) if now == self.bytes => {}
            Ok(_) => {
                return Err(tr!(
                    "`{path}` has changed since it was written",
                    path = self.path.display()
                ))
            }
            Err(e) => return Err(format!("`{}`: {e}", self.path.display())),
        }
        if !self.row.swap(payload) {
            return Err(tr!("its row is no longer in the tree").to_string());
        }
        if let Err(e) = std::fs::remove_file(&self.path) {
            self.row.swap(payload);
            return Err(format!("`{}`: {e}", self.path.display()));
        }
        Ok(())
    }

    /// Write the file again, unless something is there now, and put its row back.
    fn write_again(&mut self, payload: &mut DocPayload) -> Result<(), String> {
        if self.path.symlink_metadata().is_ok() {
            return Err(tr!("`{path}` already exists", path = self.path.display()));
        }
        std::fs::write(&self.path, &self.bytes)
            .map_err(|e| format!("`{}`: {e}", self.path.display()))?;
        if !self.row.swap(payload) {
            let _ = std::fs::remove_file(&self.path);
            return Err(tr!("the folder it was written into is no longer in the tree").to_string());
        }
        Ok(())
    }
}

/// The step that takes back the file `bytes` written at `path`, shown by the row `id`.
pub(super) fn wrote(path: PathBuf, bytes: Vec<u8>, id: RpfNodeId) -> Change {
    step(Written {
        path,
        bytes,
        row: Row { id, held: None },
        here: true,
        why: String::new(),
    })
}
