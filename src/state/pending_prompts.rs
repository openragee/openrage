//! The edit gate's verdict, and the prompts a gated write or the open of an Enhanced file waits on.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{save_destination, DocumentId, DocumentKind, Game, RpfNodeId, TypeFilter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetailEditDecision {
    YesSession,
    No,
}

#[derive(Debug, Clone)]
pub enum PendingRetailEdit {
    /// An undo or a redo whose step writes inside an install.
    Step {
        doc_id: DocumentId,
        direction: super::EditDirection,
    },
    /// `Ctrl+S` on a document whose own file is inside an install.
    SaveLooseFile {
        doc_id: DocumentId,
    },
    /// `Save As` to a path inside an install.
    SaveAs {
        doc_id: DocumentId,
        path: PathBuf,
    },
    AddFolder {
        doc_id: DocumentId,
        parent_id: RpfNodeId,
        name: String,
    },
    AddEmptyFile {
        doc_id: DocumentId,
        parent_folder: RpfNodeId,
        kind: DocumentKind,
    },
    RenameNode {
        doc_id: DocumentId,
        node_id: RpfNodeId,
        name: String,
    },
    DeleteNode {
        doc_id: DocumentId,
        node_id: RpfNodeId,
    },
    MoveNode {
        doc_id: DocumentId,
        node_ids: Vec<RpfNodeId>,
        new_parent: RpfNodeId,
    },
    ImportFileToFolder {
        doc_id: DocumentId,
        folder_id: RpfNodeId,
        file_path: PathBuf,
    },
    /// Files and folders dropped from Explorer onto an archive folder, or rows copied there, whose files are `staged`.
    ImportPaths {
        doc_id: DocumentId,
        folder_id: RpfNodeId,
        paths: Vec<PathBuf>,
        staged: Option<Arc<Staged>>,
    },
    /// What Copy or Cut put on the clipboard, pasted into a folder or an archive.
    PasteRows {
        doc_id: DocumentId,
        folder_id: RpfNodeId,
    },
    SaveDirtyArchives {
        doc_id: DocumentId,
    },
    /// ⚠ The only prompt whose target a user picked freely: a destination
    /// directory, which a file picker will happily aim inside an install.
    ExtractInto {
        doc_id: DocumentId,
        nodes: Vec<RpfNodeId>,
        dest: PathBuf,
        form: or_io::container::rpf::BodyForm,
        filter: TypeFilter,
    },
    /// Saving a document opened from an archive into the destination the user chose.
    SaveEntry {
        doc_id: DocumentId,
        dest: save_destination::SaveDestination,
    },
    /// Working out a defragment's size; the write asks again once the preview is answered.
    Defragment {
        doc_id: DocumentId,
        node_id: RpfNodeId,
    },
}

#[derive(Debug, Clone)]
pub struct PendingRetailEditPrompt {
    pub action: PendingRetailEdit,
    pub target_path: PathBuf,
    pub verb: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnhancedOpenDecision {
    Switch,
    Cancel,
}

#[derive(Debug, Clone)]
pub struct PendingEnhancedOpen {
    pub path: PathBuf,
    pub game: Game,
    pub version: u32,
    pub enhanced_root_known: bool,
}

#[derive(Debug, Clone)]
pub(super) enum RetailGuard {
    Allowed,
    NeedsConfirmation(PathBuf),
    /// The current [`EditMode`](prefs::EditMode) does not permit this write at
    /// all. Carries the whole sentence, path included, because a refusal a user
    /// cannot read is a save that appears to have silently failed.
    Refused(String),
}

/// What a caller should do after asking the gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateOutcome {
    Proceed,
    Refused,
    Confirm(PathBuf),
}

/// A gated edit's `Err` once the gate has prompted or refused; the gate already told the user, so it is never toasted.
pub(crate) fn retail_pending_message() -> String {
    tr!("Awaiting retail-edit confirmation").to_string()
}

/// A folder a copy's files are staged in, taken away when the last hold on it goes: the question
/// that waits on them answered, refused or replaced by another.
#[derive(Debug)]
pub struct Staged(PathBuf);

impl Staged {
    pub fn new(folder: PathBuf) -> Arc<Self> {
        Arc::new(Self(folder))
    }

    pub fn folder(&self) -> &Path {
        &self.0
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
