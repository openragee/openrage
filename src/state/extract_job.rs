//! Extracting entries out of an archive: planned here, run on a worker.
//!
//! The job model — the cancellation flag, the counters and the rule that
//! a per-item failure is counted rather than propagated — is [`or_job`]'s, and
//! `openrage-cli`'s `rpf extract` and `bulk` run through the same one.
//!
//! ⚠ [`RpfEditSession`](super::RpfEditSession) lives inside `AppState`, and
//! nothing on a worker may touch app state. So the plan — which entries, in
//! which archive, at which relative path — is built on the UI thread out of
//! `Arc`s and plain data, and the worker never sees the session. Reading the
//! bytes up front instead would put a whole archive in memory before writing
//! the first file.

use std::collections::{BTreeMap, HashSet};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use or_io::container::rpf::{BodyForm, RpfArchive};
use or_io::container::RpfEntryKind;
use or_job::{Cancel, Progress, Report, Snapshot, Step};
use or_title::vfs::ArchiveDevice;

use super::document::{DocumentId, RpfNodeId};
use super::document_io::{canonical_form, canonical_form_repairing, has_canonical_form};
use super::extract_planning::{OpenUnder, Planning};
use super::jobs::{JobKind, Outcome, RunningJob, Then};
use super::prefs::ExportFixes;
use super::rpf_node::RpfNodeBacking;
use super::rpf_session::RpfEditSession;
use super::{AppState, DocPayload, GateOutcome, PendingRetailEdit, PendingRetailEditPrompt};

/// Which extensions an extraction is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeFilter {
    All,
    /// Lowercase extensions without the dot. An entry with no extension is
    /// named by the empty string, so "everything without one" is expressible.
    Only(Vec<String>),
}

impl TypeFilter {
    pub fn admits(&self, name: &str) -> bool {
        match self {
            Self::All => true,
            Self::Only(exts) => exts.iter().any(|e| e == &extension_of(name)),
        }
    }
}

/// The extension an entry is filed under, lowercased and without the dot.
pub fn extension_of(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Where one item's bytes come from, in a form a worker can hold.
#[derive(Clone)]
enum ExtractSource {
    /// ⚠ The only source [`BodyForm`] means anything for. A disk file or an
    /// unsaved edit has exactly one form: the bytes themselves.
    Archive {
        device: ArchiveDevice,
        entry_index: u32,
        kind: RpfEntryKind,
    },
    Rpf9Table(Arc<super::document::rpf9::entry::EntrySource>),
    Disk(PathBuf),
    Bytes(Arc<Vec<u8>>),
    /// A body a scan of the GTA VI archive at `archive` found at `offset`.
    Scanned {
        archive: PathBuf,
        offset: u64,
    },
    /// A GTA VI resource's sections, written as a loose resource.
    Sections(Arc<or_io::container::rpf::Rpf9Sections>),
    /// The body at `offset` of `archive` read again and edited, written as a loose resource.
    Edited {
        archive: PathBuf,
        offset: u64,
        edit: Arc<super::gta6_edit::EditSections>,
    },
    /// Bytes a writer makes on the worker.
    Written(Arc<super::gta6_other_bodies_save::Write>),
}

/// One file to write, and where its bytes are.
#[derive(Clone)]
pub struct ExtractItem {
    rel_path: String,
    source: ExtractSource,
}

impl ExtractItem {
    /// The body a scan of the GTA VI archive at `archive` found at `offset`, written as `rel_path`.
    pub(super) fn scanned(rel_path: String, archive: PathBuf, offset: u64) -> Self {
        let source = ExtractSource::Scanned { archive, offset };
        Self { rel_path, source }
    }

    /// `sections`, written as `rel_path` as a loose resource.
    pub(super) fn sections(
        rel_path: String,
        sections: or_io::container::rpf::Rpf9Sections,
    ) -> Self {
        let source = ExtractSource::Sections(Arc::new(sections));
        Self { rel_path, source }
    }

    /// The body at `offset` of `archive` read again, `edit` made to its sections, written as
    /// `rel_path` as a loose resource.
    pub(super) fn edited(
        rel_path: String,
        archive: PathBuf,
        offset: u64,
        edit: Arc<super::gta6_edit::EditSections>,
    ) -> Self {
        let source = ExtractSource::Edited {
            archive,
            offset,
            edit,
        };
        Self { rel_path, source }
    }

    /// The bytes `write` makes on the worker, written as `rel_path`.
    pub(super) fn written(
        rel_path: String,
        write: Arc<super::gta6_other_bodies_save::Write>,
    ) -> Self {
        let source = ExtractSource::Written(write);
        Self { rel_path, source }
    }

    /// An RPF9 table entry decoded progressively on the worker.
    pub(super) fn rpf9(source: super::document::rpf9::entry::EntrySource) -> Self {
        Self {
            rel_path: source.name.clone(),
            source: ExtractSource::Rpf9Table(Arc::new(source)),
        }
    }

    /// `bytes`, written as `rel_path`.
    pub(super) fn bytes(rel_path: String, bytes: Vec<u8>) -> Self {
        let source = ExtractSource::Bytes(Arc::new(bytes));
        Self { rel_path, source }
    }

    /// The file or folder this item lands in directly under the destination.
    pub fn top_component(&self) -> &str {
        self.rel_path.split('/').next().unwrap_or(&self.rel_path)
    }

    /// Would this item be openable if written in `form`?
    fn loadable_in(&self, form: BodyForm) -> bool {
        match &self.source {
            ExtractSource::Archive { kind, .. } => form.is_loadable_for(*kind),
            _ => true,
        }
    }

    pub(super) fn read(&self, form: BodyForm) -> Result<Vec<u8>, String> {
        match &self.source {
            ExtractSource::Bytes(bytes) => Ok((**bytes).clone()),
            ExtractSource::Disk(path) => std::fs::read(path).map_err(|e| e.to_string()),
            ExtractSource::Scanned { archive, offset } => {
                super::rpf9_scan::extracted_body(archive, *offset)
            }
            ExtractSource::Sections(sections) => {
                or_io::container::rpf::write_rpf9_resource(sections).map_err(|e| e.to_string())
            }
            ExtractSource::Edited {
                archive,
                offset,
                edit,
            } => super::gta6_edit::written(archive, *offset, &**edit),
            ExtractSource::Written(write) => write(),
            ExtractSource::Rpf9Table(source) => source.bytes(),
            ExtractSource::Archive {
                device,
                entry_index,
                ..
            } => device
                .read_entry(*entry_index, form)
                .map_err(|e| e.to_string()),
        }
    }

    /// The archive file on disk this item is read out of, if it is in one.
    pub(super) fn archive_read(&self) -> Option<PathBuf> {
        match &self.source {
            ExtractSource::Archive { device, .. } => {
                archive_file(device.archive()).map(Path::to_path_buf)
            }
            ExtractSource::Rpf9Table(source) => {
                super::document::rpf9::entry::archive_path(&source.path).map(Path::to_path_buf)
            }
            _ => None,
        }
    }

    /// What an export writes: OpenRage's canonical form when `fixes` and the form is decoded, else the bytes as they are.
    pub(super) fn export_bytes(&self, form: BodyForm, fixes: bool) -> Result<Vec<u8>, String> {
        let bytes = self.read(form)?;
        if !fixes || form != BodyForm::Decoded || matches!(self.source, ExtractSource::Rpf9Table(_))
        {
            return Ok(bytes);
        }
        let Some((canonical, repairs)) = canonical_form_repairing(&self.rel_path, &bytes) else {
            return Ok(bytes);
        };
        for repair in repairs {
            tracing::info!("Repaired {}: {repair}", self.rel_path);
        }
        Ok(canonical)
    }
}

/// Whether writing any decoded item in OpenRage's canonical form changes its bytes.
///
/// ⚠ Stops at the first that does: a game file usually does, and reading the
/// rest only to ask the same question would double the export's reads.
/// ⚠ Runs on the UI thread, so an export of files already canonical reads
/// each one once before it starts.
fn canonicalising_alters(items: &[ExtractItem], form: BodyForm) -> bool {
    form == BodyForm::Decoded
        && items
            .iter()
            .filter(|item| {
                !matches!(item.source, ExtractSource::Rpf9Table(_))
                    && has_canonical_form(&item.rel_path)
            })
            .any(|item| {
                item.read(form).is_ok_and(|bytes| {
                    canonical_form(&item.rel_path, &bytes).is_some_and(|c| c != bytes)
                })
            })
}

/// Every file at or under `node_id` that the filter admits.
///
/// A file node plans to one item; a directory plans to its whole subtree.
pub fn plan(session: &RpfEditSession, node_id: RpfNodeId, filter: &TypeFilter) -> Vec<ExtractItem> {
    let mut out = Vec::new();
    let Some(node) = session.node(node_id) else {
        return out;
    };
    // A single file lands at its own name rather than under a directory the
    // user did not name.
    if !node.is_dir() {
        return match source_for(session, node_id) {
            Some(source) if filter.admits(&node.name) => vec![ExtractItem {
                rel_path: sanitise(&node.name),
                source,
            }],
            _ => out,
        };
    }
    collect(session, node_id, "", filter, &mut out);
    out
}

/// Every file under a multi-row selection, and how many were left out because
/// another selected row had already claimed their path.
///
/// One row plans exactly as [`plan`] does. Several rows each land under their
/// own name, since there is no single folder whose contents they are.
///
/// ⚠ **A row inside another selected row is dropped**, or a folder and a file
/// in it would both be written and the second write would be a duplicate the
/// report counts as done. ⚠ **Two rows can still claim one path** — `a.ytd`
/// from two folders — and the first keeps it: silently overwriting one with
/// the other would report two files written and leave one.
pub fn plan_selection(
    session: &RpfEditSession,
    nodes: &[RpfNodeId],
    filter: &TypeFilter,
) -> (Vec<ExtractItem>, usize) {
    if let [only] = nodes {
        return (plan(session, *only, filter), 0);
    }
    plan_named_rows(session, nodes, filter)
}

/// As [`plan_selection`], but one row lands under its own name too.
pub fn plan_named_rows(
    session: &RpfEditSession,
    nodes: &[RpfNodeId],
    filter: &TypeFilter,
) -> (Vec<ExtractItem>, usize) {
    let selected: std::collections::HashSet<RpfNodeId> = nodes.iter().copied().collect();
    let inside_another = |id: RpfNodeId| {
        let mut at = session.node(id).and_then(|n| n.parent);
        while let Some(parent) = at {
            if selected.contains(&parent) {
                return true;
            }
            at = session.node(parent).and_then(|n| n.parent);
        }
        false
    };

    let mut planned = Vec::new();
    for &id in nodes {
        let Some(node) = session.node(id) else {
            continue;
        };
        if inside_another(id) {
            continue;
        }
        let name = sanitise(&node.name);
        if node.is_dir() {
            collect(session, id, &name, filter, &mut planned);
        } else if filter.admits(&node.name) {
            if let Some(source) = source_for(session, id) {
                planned.push(ExtractItem {
                    rel_path: name,
                    source,
                });
            }
        }
    }

    let mut claimed = std::collections::HashSet::new();
    let before = planned.len();
    planned.retain(|item| claimed.insert(item.rel_path.to_ascii_lowercase()));
    let collided = before - planned.len();
    (planned, collided)
}

fn collect(
    session: &RpfEditSession,
    node_id: RpfNodeId,
    prefix: &str,
    filter: &TypeFilter,
    out: &mut Vec<ExtractItem>,
) {
    for child_id in session.children_of(node_id) {
        let Some(child) = session.node(child_id) else {
            continue;
        };
        let name = sanitise(&child.name);
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if child.is_dir() {
            collect(session, child_id, &path, filter, out);
            continue;
        }
        if !filter.admits(&child.name) {
            continue;
        }
        if let Some(source) = source_for(session, child_id) {
            out.push(ExtractItem {
                rel_path: path,
                source,
            });
        }
    }
}

/// ⚠ A name out of an archive decides a path on disk, so a separator cannot
/// survive it. One entry called `../../gta5.exe` would otherwise put the
/// extraction outside the directory the gate was asked about.
pub(super) fn sanitise(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            ':' | '<' | '>' | '"' | '|' | '?' | '*' | '/' | '\\' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    // ⚠ Trailing only. A leading dot is a legitimate name (`.gitignore`);
    // Windows is the one that refuses a trailing dot or space, and `..`
    // trimmed from the end is what stops it staying a parent reference.
    let trimmed = cleaned.trim_end_matches(['.', ' ']);
    if trimmed.is_empty() {
        "_".to_string()
    } else {
        trimmed.to_string()
    }
}

fn source_for(session: &RpfEditSession, node_id: RpfNodeId) -> Option<ExtractSource> {
    let node = session.node(node_id)?;
    match &node.backing {
        RpfNodeBacking::InMemoryFile { bytes } => Some(ExtractSource::Bytes(Arc::clone(bytes))),
        RpfNodeBacking::DiskFile {
            fs_path,
            override_bytes,
            ..
        } => Some(match override_bytes {
            // An unsaved edit is what the user can see, so it is what they get.
            Some(bytes) => ExtractSource::Bytes(Arc::clone(bytes)),
            None => ExtractSource::Disk(fs_path.clone()),
        }),
        RpfNodeBacking::ArchiveFile {
            archive_id,
            entry_index,
            override_bytes,
        } => {
            if let Some(bytes) = override_bytes {
                return Some(ExtractSource::Bytes(Arc::clone(bytes)));
            }
            let device = session.device(*archive_id)?;
            let kind = device.archive().entries.get(*entry_index as usize)?.kind();
            Some(ExtractSource::Archive {
                device,
                entry_index: *entry_index,
                kind,
            })
        }
        _ => None,
    }
}

/// The file on disk `archive` is read from, through every archive it is nested in.
pub(super) fn archive_file(archive: &RpfArchive) -> Option<&Path> {
    archive
        .path()
        .or_else(|| archive.parent().and_then(|parent| archive_file(parent)))
}

/// What an extraction of `nodes` is called: the row's own name, or how many rows.
pub(super) fn extraction_label(session: &RpfEditSession, nodes: &[RpfNodeId]) -> String {
    match nodes {
        [only] => session
            .node(*only)
            .filter(|n| !n.name.is_empty())
            .map(|n| n.name.clone())
            .unwrap_or_else(|| "archive".to_string()),
        many => tr!("{count} selected rows", count = many.len()),
    }
}

/// The bytes every file at or under `nodes` holds, counted once each.
pub fn total_size(session: &RpfEditSession, nodes: &[RpfNodeId]) -> u64 {
    let mut seen = std::collections::HashSet::new();
    let mut stack: Vec<RpfNodeId> = nodes.to_vec();
    let mut total = 0u64;
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        match session.node(id) {
            Some(node) if node.is_dir() => stack.extend(session.children_of(id)),
            Some(_) => total += session.size_for(id).unwrap_or(0),
            None => {}
        }
    }
    total
}

/// How many files of each extension sit at or under a node, commonest first.
pub fn extension_census(session: &RpfEditSession, node_id: RpfNodeId) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut stack = vec![node_id];
    while let Some(id) = stack.pop() {
        let Some(node) = session.node(id) else {
            continue;
        };
        if node.is_dir() {
            stack.extend(session.children_of(id));
            continue;
        }
        *counts.entry(extension_of(&node.name)).or_default() += 1;
    }
    let mut out: Vec<(String, usize)> = counts.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

/// What an extraction does with a file its destination already holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnClash {
    Replace,
    Skip,
    /// The extracted copy lands beside the old one as `name (2).ext`, or the first number free.
    KeepBoth,
    /// Replaced as Save replaces a file: through a temporary file, with a backup made the first time.
    Save,
}

/// `name (2).ext` beside `target`, or the first number after it that nothing holds.
pub(super) fn kept_copy(target: &Path) -> PathBuf {
    let stem = target.file_stem().unwrap_or_default().to_string_lossy();
    let ext = target
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (2u32..)
        .map(|n| target.with_file_name(format!("{stem} ({n}){ext}")))
        .find(|candidate| std::fs::symlink_metadata(candidate).is_err())
        .unwrap_or_else(|| target.to_path_buf())
}

struct Rpf9Output<'a, W> {
    writer: &'a mut W,
    cancel: &'a Cancel,
}

impl<W: Write> Rpf9Output<'_, W> {
    fn check_cancelled(&self) -> io::Result<()> {
        if self.cancel.is_set() {
            // Interrupted is retried by write_all, so cancellation must use a terminal error.
            return Err(io::Error::other("stopped"));
        }
        Ok(())
    }
}

impl<W: Write> Write for Rpf9Output<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.check_cancelled()?;
        self.writer.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.check_cancelled()?;
        self.writer.flush()
    }
}

/// Write every item under `dest`, counting what fails and stopping when asked.
///
/// ⚠ Runs on a worker and touches no app state, which is why it is a free
/// function taking everything it needs.
pub fn run_extraction(
    items: Vec<ExtractItem>,
    dest: PathBuf,
    form: BodyForm,
    fixes: bool,
    on_clash: OnClash,
    cancel: Cancel,
    progress: Progress,
) -> Report {
    // ⚠ A save of several files, such as a cell and the neighbours whose links it renumbered,
    // writes none of them unless every one is made: a part written names records the rest lack.
    let (items, fixes) = match on_clash {
        OnClash::Save if items.len() > 1 => {
            let made: Result<Vec<ExtractItem>, String> = (items.into_iter())
                .map(|i| match i.export_bytes(form, fixes) {
                    Ok(bytes) => Ok(ExtractItem::bytes(i.rel_path, bytes)),
                    Err(e) => Err(format!("{}: {e}", i.rel_path)),
                })
                .collect();
            match made {
                Ok(made) => (made, false),
                Err(e) => return or_job::run([e], &cancel, &progress, Step::Failed),
            }
        }
        _ => (items, fixes),
    };
    let mut rpf9_cancelled = false;
    let mut report = or_job::run(items, &cancel, &progress, |item| {
        progress.now_on(item.rel_path.clone());
        let mut target = dest.join(&item.rel_path);
        if std::fs::symlink_metadata(&target).is_ok() {
            match on_clash {
                OnClash::Replace | OnClash::Save => {}
                OnClash::Skip => return Step::Skipped,
                OnClash::KeepBoth => target = kept_copy(&target),
            }
        }
        if let Some(parent) = target.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return Step::Failed(format!("{}: {e}", parent.display()));
            }
        }
        // A scanned body its list gave an end is copied as a range, never held whole.
        if let (ExtractSource::Scanned { archive, offset }, false) =
            (&item.source, on_clash == OnClash::Save)
        {
            let copy = super::gta6_first_entry::copied;
            if let Some(step) = copy(archive, *offset, &target, &item.rel_path, &cancel) {
                return step;
            }
        }
        if let (ExtractSource::Rpf9Table(source), false) = (&item.source, on_clash == OnClash::Save)
        {
            let write = || -> Result<u64, String> {
                let mut temp = tempfile::NamedTempFile::new_in(&dest).map_err(|e| e.to_string())?;
                let mut output = Rpf9Output {
                    writer: temp.as_file_mut(),
                    cancel: &cancel,
                };
                let count = source.extract(&mut output)?;
                output.check_cancelled().map_err(|e| e.to_string())?;
                if matches!(on_clash, OnClash::KeepBoth | OnClash::Skip) {
                    temp.persist_noclobber(&target).map_err(|e| e.to_string())?;
                } else {
                    temp.persist(&target).map_err(|e| e.to_string())?;
                }
                Ok(count)
            };
            return match write() {
                Ok(count) => Step::Done(count),
                Err(_) if cancel.is_set() => {
                    rpf9_cancelled = true;
                    Step::Skipped
                }
                Err(e) => Step::Failed(format!("{}: {e}", item.rel_path)),
            };
        }
        let write = |bytes: &[u8]| match on_clash {
            OnClash::Save => super::document_io::atomic_write_with_backup(&target, bytes),
            _ => std::fs::write(&target, bytes),
        };
        match item.export_bytes(form, fixes) {
            Ok(bytes) => match write(&bytes) {
                Ok(()) => Step::Done(bytes.len() as u64),
                Err(e) => Step::Failed(format!("{}: {e}", item.rel_path)),
            },
            Err(e) => Step::Failed(format!("{}: {e}", item.rel_path)),
        }
    });
    // A stop inside the final entry leaves no following iteration for or_job to observe it.
    report.cancelled |= rpf9_cancelled;
    report
}

/// The one extraction the editor has in flight, if any.
#[derive(Default)]
pub struct ExtractJobs {
    active: Option<ActiveJob>,
    /// An extraction still opening the rows it names, before it is planned.
    pub(super) planning: Option<Planning>,
    /// The rows being opened for the counts of "Only one type", and under which row.
    pub(super) census: Option<(DocumentId, RpfNodeId, OpenUnder)>,
    /// The rows a drag out asked to open, and under which rows, until it has them all.
    pub(super) drag_opening: Option<(DocumentId, Vec<RpfNodeId>, OpenUnder)>,
    /// The rows a drag out told would not open, and under which rows: a drag out of the same rows goes ahead without them.
    pub(super) drag_told: Option<(DocumentId, Vec<RpfNodeId>, HashSet<RpfNodeId>)>,
    /// The rows a count of "Only one type" told would not open: left out of what is unread, and not asked for again, until they list.
    pub(super) census_failed: HashSet<(DocumentId, RpfNodeId)>,
    /// An extraction waiting on what to do with the files its destination already holds.
    pub clash: Option<ExtractClash>,
}

/// An extraction asked once, for the whole job, what to do with the files already there.
#[derive(Debug, Clone)]
pub struct ExtractClash {
    pub doc_id: DocumentId,
    pub nodes: Vec<RpfNodeId>,
    pub dest: PathBuf,
    pub form: BodyForm,
    pub filter: TypeFilter,
    pub fixes: bool,
    pub clashes: usize,
    pub planned: usize,
    /// The first file already there, relative to the destination.
    pub first: String,
}

impl ExtractClash {
    pub fn question(&self) -> String {
        clash_question(self.clashes, self.planned, &self.first)
    }
}

/// What the "Files already there" question asks, for `clashes` of `planned` files, `first` among them.
pub(super) fn clash_question(clashes: usize, planned: usize, first: &str) -> String {
    match clashes {
        1 => tr!("{first} is already in", first = first),
        n => tr!(
            "{n} of the {planned} files, {first} among them, are already in",
            n = n,
            planned = planned,
            first = first
        ),
    }
}

struct ActiveJob {
    /// Which job the list names it as: an extraction, a conversion's write or an export's.
    kind: JobKind,
    cancel: Cancel,
    progress: Progress,
    receiver: Receiver<Report>,
    label: String,
    dest: PathBuf,
    /// The files on disk its items read: archives at the offsets they were planned with, and loose files.
    reads: Vec<PathBuf>,
    /// The document whose own file it writes, and the edit it was at: saved once the write lands.
    saves: Option<(DocumentId, u64)>,
}

impl ExtractJobs {
    /// Whether an extraction, an export's write or a conversion's holds the slot.
    #[cfg(test)]
    pub fn is_running(&self) -> bool {
        self.running_noun().is_some()
    }

    /// What holds the one slot, for a sentence, as the jobs list names it: "An extraction", "An export" or "A conversion".
    pub fn running_noun(&self) -> Option<&'static str> {
        match &self.active {
            Some(job) => Some(match job.kind {
                JobKind::Conversion => tr!("A conversion"),
                JobKind::Export => tr!("An export"),
                _ => tr!("An extraction"),
            }),
            None => self.planning.as_ref().map(|_| tr!("An extraction")),
        }
    }

    /// The extraction in flight, named, and the files on disk it reads: while it opens its rows, the archives it has asked for.
    pub fn reads(&self) -> Option<(String, &[PathBuf])> {
        let Some(job) = &self.active else {
            let planning = self.planning.as_ref()?;
            return Some((planning.name(), planning.reads()));
        };
        let noun = job.kind.label().to_lowercase();
        Some((
            tr!("the {noun} of {job}", noun = noun, job = job.label),
            &job.reads,
        ))
    }

    /// As [`reads`](Self::reads), but while it opens its rows, the files its rows sit in or are: what is deleted, renamed or moved under them is read when it plans.
    pub fn holds(&self) -> Option<(String, &[PathBuf])> {
        match &self.planning {
            Some(planning) => Some((planning.name(), planning.roots())),
            None => self.reads(),
        }
    }

    /// The extraction in flight, as the job list draws it.
    pub fn job(&self) -> Option<RunningJob> {
        let Some(job) = &self.active else {
            return (self.planning.as_ref().map(Planning::job)).or_else(|| {
                let (_, _, walk) = self.drag_opening.as_ref()?;
                Some(RunningJob {
                    kind: JobKind::Extraction,
                    doing: tr!("Opening what the drag out holds").to_string(),
                    fraction: None,
                    detail: walk.progress_line(),
                    stopping: false,
                })
            });
        };
        let snap: Snapshot = job.progress.snapshot();
        let verb = match job.kind {
            JobKind::Conversion => tr!("Converting"),
            JobKind::Export => tr!("Exporting"),
            _ => tr!("Extracting"),
        };
        Some(RunningJob {
            kind: job.kind,
            doing: format!("{verb} {}", job.label),
            fraction: (snap.total > 0).then(|| snap.fraction()),
            detail: tr!(
                "{attempted} of {total} files  {item}",
                attempted = snap.attempted,
                total = snap.total,
                item = snap.item
            ),
            stopping: job.cancel.is_set(),
        })
    }

    /// Stop the job in flight if it is the `kind` the list named: at once while an extraction is still opening its rows, which returns `true`; else once the file in hand is written.
    ///
    /// ⚠ Sets a flag rather than killing the worker, so the file in hand is finished and counted.
    pub fn request_cancel(&mut self, kind: JobKind) -> bool {
        // ⚠ The rows a drag out is opening are dropped with it: what has opened stays open
        let dragging = kind == JobKind::Extraction && self.drag_opening.take().is_some();
        if kind == JobKind::Extraction && (self.planning.take().is_some() || dragging) {
            return true;
        }
        if let Some(job) = self.active.as_ref().filter(|job| job.kind == kind) {
            job.cancel.set();
        }
        false
    }

    /// Run `work` as the one extraction, conversion's write or export's, under `dest`, reading the files in `reads`.
    pub(super) fn start(
        &mut self,
        kind: JobKind,
        label: String,
        dest: PathBuf,
        reads: Vec<PathBuf>,
        work: impl FnOnce(Cancel, Progress) -> Report + Send + 'static,
    ) {
        let cancel = Cancel::new();
        let progress = Progress::new();
        let (tx, rx) = channel::<Report>();
        let (worker_cancel, worker_progress) = (cancel.clone(), progress.clone());
        std::thread::spawn(move || {
            let _ = tx.send(work(worker_cancel, worker_progress));
        });
        self.active = Some(ActiveJob {
            kind,
            cancel,
            progress,
            receiver: rx,
            label,
            dest,
            reads,
            saves: None,
        });
    }

    /// The job just started writes document `doc`'s own file as it is at edit `at`.
    pub(super) fn saving(&mut self, doc: DocumentId, at: u64) {
        if let Some(job) = &mut self.active {
            job.saves = Some((doc, at));
        }
    }
}

/// An export waiting on whether to write OpenRage's canonical form.
#[derive(Debug, Clone)]
pub enum PendingExportFixes {
    Extract {
        doc_id: DocumentId,
        nodes: Vec<RpfNodeId>,
        dest: PathBuf,
        form: BodyForm,
        filter: TypeFilter,
    },
    /// ⚠ The drag has ended by the time it is answered, so the answer is
    /// held for the next drag out rather than applied.
    DragOut,
    /// A loose save of an archive entry, the gate already passed.
    SaveEntry { doc_id: DocumentId, path: PathBuf },
    /// One row exported to a file the user named.
    ExportAs {
        doc_id: DocumentId,
        node: RpfNodeId,
        path: PathBuf,
    },
}

/// What asking for an extraction did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtractStart {
    Started {
        planned: usize,
    },
    /// The edit gate said no, and has already said why.
    Refused,
    /// The edit gate, or a question about the export, wants a click first.
    AwaitingConfirmation,
    /// Rows it names are opening on workers, and its plan follows in a later frame, which says its own outcome.
    Planning,
    /// Nothing under the node matched, so nothing was spawned.
    NothingToDo,
    /// One extraction at a time; a second would race the first on disk.
    AlreadyRunning,
}

impl AppState {
    /// Extract a node's subtree into `dest`, if the edit gate permits it.
    ///
    /// ⚠ **The gate is asked about the destination, not about the archive.**
    /// Extraction is the one operation in the editor whose target a user picks
    /// freely, so a picker pointed at `<install>/x64/` is how an extraction
    /// overwrites shipped content — and that is the case
    /// `the_gate_stands_in_front_of_every_write` names this function for.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn extract_rpf_node(
        &mut self,
        doc_id: DocumentId,
        node_id: RpfNodeId,
        dest: PathBuf,
        form: BodyForm,
        filter: TypeFilter,
    ) -> ExtractStart {
        self.extract_rpf_nodes(doc_id, vec![node_id], dest, form, filter)
    }

    /// Extract every selected row into `dest`, if the edit gate permits it.
    pub fn extract_rpf_nodes(
        &mut self,
        doc_id: DocumentId,
        nodes: Vec<RpfNodeId>,
        dest: PathBuf,
        form: BodyForm,
        filter: TypeFilter,
    ) -> ExtractStart {
        if let Some(what) = self.extract_jobs.running_noun() {
            self.show_error_toast(tr!("{what} is already running", what = what));
            return ExtractStart::AlreadyRunning;
        }
        match self.gate_permits(Some(&dest)) {
            GateOutcome::Refused => return ExtractStart::Refused,
            GateOutcome::Confirm(target) => {
                self.pending_retail_edit = Some(PendingRetailEditPrompt {
                    action: PendingRetailEdit::ExtractInto {
                        doc_id,
                        nodes,
                        dest,
                        form,
                        filter,
                    },
                    target_path: target,
                    verb: tr!("extract into"),
                });
                return ExtractStart::AwaitingConfirmation;
            }
            GateOutcome::Proceed => {}
        }
        self.begin_extraction(doc_id, &nodes, dest, form, filter)
    }

    /// Answer the question about files already in an extraction's destination; `None` drops the extraction.
    pub fn answer_extract_clash(&mut self, answer: Option<OnClash>) {
        let Some(clash) = self.extract_jobs.clash.take() else {
            return;
        };
        if let Some(on_clash) = answer {
            self.extract_rpf_nodes_inner(
                clash.doc_id,
                &clash.nodes,
                clash.dest,
                clash.form,
                clash.filter,
                Some(clash.fixes),
                Some(on_clash),
            );
        }
    }

    /// Whether an export of `items` writes OpenRage's canonical form; `None` while the user has to be asked.
    pub(super) fn export_fixes_for(&self, items: &[ExtractItem], form: BodyForm) -> Option<bool> {
        match self.prefs.export_fixes {
            ExportFixes::Apply => Some(true),
            ExportFixes::Keep => Some(false),
            ExportFixes::Ask => (!canonicalising_alters(items, form)).then_some(false),
        }
    }

    /// Answer the pending export-fixes question, and remember the answer when asked to.
    pub fn apply_export_fixes(&mut self, apply: bool, remember: bool) {
        let Some(pending) = self.pending_export_fixes.take() else {
            return;
        };
        if remember {
            self.prefs.export_fixes = if apply {
                ExportFixes::Apply
            } else {
                ExportFixes::Keep
            };
            super::save_prefs(&self.prefs);
        }
        match pending {
            PendingExportFixes::Extract {
                doc_id,
                nodes,
                dest,
                form,
                filter,
            } => {
                self.extract_rpf_nodes_inner(doc_id, &nodes, dest, form, filter, Some(apply), None);
            }
            PendingExportFixes::DragOut => self.drag_out_fixes = Some(apply),
            PendingExportFixes::SaveEntry { doc_id, path } => self.save_entry_to_inner(
                doc_id,
                super::save_destination::SaveDestination::LooseFile(path),
                Some(apply),
            ),
            PendingExportFixes::ExportAs { doc_id, node, path } => {
                self.export_rpf_node_as(doc_id, node, path, Some(apply));
            }
        }
    }

    /// The tail of [`extract_rpf_nodes`](Self::extract_rpf_nodes), once the
    /// gate is satisfied; `fixes` and `on_clash` are `None` until the user has answered.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn extract_rpf_nodes_inner(
        &mut self,
        doc_id: DocumentId,
        nodes: &[RpfNodeId],
        dest: PathBuf,
        form: BodyForm,
        filter: TypeFilter,
        fixes: Option<bool>,
        on_clash: Option<OnClash>,
    ) -> ExtractStart {
        let Some(doc) = self.document(doc_id) else {
            return ExtractStart::NothingToDo;
        };
        let DocPayload::Rpf(rpf) = &doc.payload else {
            return ExtractStart::NothingToDo;
        };
        let label = extraction_label(&rpf.session, nodes);
        let (items, collided) = plan_selection(&rpf.session, nodes, &filter);
        let reads: Vec<PathBuf> = items
            .iter()
            .filter_map(|item| match &item.source {
                ExtractSource::Archive { device, .. } => archive_file(device.archive()),
                ExtractSource::Rpf9Table(source) => {
                    super::document::rpf9::entry::archive_path(&source.path)
                }
                ExtractSource::Disk(path) => Some(path.as_path()),
                ExtractSource::Scanned { archive, .. } => Some(archive.as_path()),
                ExtractSource::Edited { archive, .. } => Some(archive.as_path()),
                ExtractSource::Bytes(_) | ExtractSource::Sections(_) => None,
                ExtractSource::Written(_) => None,
            })
            .collect::<std::collections::HashSet<&Path>>()
            .into_iter()
            .map(Path::to_path_buf)
            .collect();
        // ⚠ A defragment rewrites its archive under any offset planned here.
        if let Some(held) = reads.iter().find(|path| self.defrag.holds(path)) {
            self.show_error_toast(tr!(
                "`{held}` is being defragmented; extract once that is done",
                held = held.display()
            ));
            return ExtractStart::Refused;
        }
        // ⚠ Each file by its own path, so a job reading one file of the folder does not hold back the rest; the folder is asked first, as most hold nothing a job reads.
        if self.no_job_reads(&dest, "extract").is_err() {
            let refused = items
                .iter()
                .find_map(|item| (self.no_job_reads(&dest.join(&item.rel_path), "extract")).err());
            if let Some(why) = refused {
                self.show_error_toast(why);
                return ExtractStart::Refused;
            }
        }
        let Some(fixes) = fixes.or_else(|| self.export_fixes_for(&items, form)) else {
            self.pending_export_fixes = Some(PendingExportFixes::Extract {
                doc_id,
                nodes: nodes.to_vec(),
                dest,
                form,
                filter,
            });
            return ExtractStart::AwaitingConfirmation;
        };
        if collided > 0 {
            self.show_warning_toast(tr!(
                "{collided} selected file(s) share a name with another and were \
                 left out rather than overwrite it",
                collided = collided
            ));
        }
        if items.is_empty() {
            self.show_warning_toast(tr!("Nothing to extract from {label}", label = label));
            return ExtractStart::NothingToDo;
        }
        let not_openable = items.iter().filter(|i| !i.loadable_in(form)).count();
        let planned = items.len();
        // ⚠ Each file is asked about on the UI thread, so a folder that holds
        // anything costs a look per planned file before the job starts.
        let holds_any = std::fs::read_dir(&dest).is_ok_and(|mut entries| entries.next().is_some());
        let clashing: Vec<&str> = items
            .iter()
            .map(|item| item.rel_path.as_str())
            .filter(|rel| holds_any && std::fs::symlink_metadata(dest.join(rel)).is_ok())
            .collect();
        let on_clash = match (on_clash, clashing.first()) {
            (Some(answer), _) => answer,
            (None, Some(first)) => {
                self.extract_jobs.clash = Some(ExtractClash {
                    doc_id,
                    nodes: nodes.to_vec(),
                    first: first.to_string(),
                    clashes: clashing.len(),
                    planned,
                    dest,
                    form,
                    filter,
                    fixes,
                });
                return ExtractStart::AwaitingConfirmation;
            }
            // Nothing was there when asked, so a file that turns up before its
            // write is somebody else's and is not this job's to replace.
            (None, None) => OnClash::Skip,
        };

        if let Err(e) = std::fs::create_dir_all(&dest) {
            self.show_error_toast(format!("{}: {e}", dest.display()));
            return ExtractStart::NothingToDo;
        }

        let worker_dest = dest.clone();
        self.extract_jobs.start(
            JobKind::Extraction,
            label,
            dest,
            reads,
            move |cancel, progress| {
                run_extraction(items, worker_dest, form, fixes, on_clash, cancel, progress)
            },
        );
        if not_openable > 0 {
            self.show_warning_toast(tr!(
                "{not_openable} of {planned} will not open in `{name}` form — a \
                 resource's page flags live in the entry table, not in its body",
                not_openable = not_openable,
                planned = planned,
                name = form.name()
            ));
        }
        ExtractStart::Started { planned }
    }

    /// Where a finished extraction lands: the per-frame tick, like every other
    /// worker result.
    pub fn poll_extract_job(&mut self) {
        self.advance_planning();
        self.advance_drag_opening();
        self.advance_census();
        let Some(job) = &self.extract_jobs.active else {
            return;
        };
        let Ok(report) = job.receiver.try_recv() else {
            return;
        };
        let (kind, dest, saves) = (job.kind, job.dest.clone(), job.saves);
        self.extract_jobs.active = None;
        if let Some((doc, at)) = saves.filter(|_| !report.cancelled && report.counts.failed == 0) {
            // An edit made while it wrote is not in the file, so it stays unsaved.
            if let Some(doc) = self.document_mut(doc).filter(|d| d.history.at() == at) {
                doc.mark_saved();
                super::gta6_physics_edit::bound_reshape::saved_over(doc);
            }
        }
        // ⚠ Where an extraction's, an Export as…'s, a conversion's and a glTF export's writes all end, so Uses reads the install again once one landed under it.
        if report.counts.attempted > report.counts.skipped {
            self.file_replaced(&dest);
        }

        for failure in report.failures.iter().take(3) {
            tracing::warn!(%failure, "extraction item failed");
        }
        let mut summary = format!("{} into {}", report.summary(), dest.display());
        // A save is one item, so its failure is the reason it was refused; a job of many items
        // keeps its count alone, and the log holds each reason.
        if let Some(why) = report
            .failures
            .first()
            .filter(|_| report.counts.attempted == 1)
        {
            summary = format!("{summary}: {why}");
        }
        if report.cancelled || report.counts.failed > 0 {
            self.show_warning_toast(summary.clone());
        } else {
            self.show_toast(summary.clone());
        }
        let then = Then::OpenFolder(dest);
        let outcome = if report.cancelled {
            Outcome::Stopped(then)
        } else {
            Outcome::Done(then)
        };
        self.jobs.record(kind, outcome, summary);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn rpf9_output_stops_after_cancellation_during_a_write() {
        struct StopAfterWrite {
            bytes: Vec<u8>,
            cancel: super::Cancel,
        }
        impl std::io::Write for StopAfterWrite {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.bytes.extend_from_slice(bytes);
                self.cancel.set();
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        use std::io::Write;
        let cancel = super::Cancel::new();
        let mut writer = StopAfterWrite {
            bytes: Vec::new(),
            cancel: cancel.clone(),
        };
        let mut output = super::Rpf9Output {
            writer: &mut writer,
            cancel: &cancel,
        };
        assert!(!cancel.is_set());
        output.write_all(b"first block").unwrap();
        assert!(cancel.is_set());
        let error = output.write_all(b"must not be written").unwrap_err();
        assert_ne!(error.kind(), std::io::ErrorKind::Interrupted);
        assert!(
            output.check_cancelled().is_err(),
            "the last block must not be published either"
        );
        assert_eq!(writer.bytes, b"first block");
    }

    use super::*;

    fn session_with(files: &[(&str, &[u8])]) -> RpfEditSession {
        let mut session = RpfEditSession::empty();
        let root = session.root_id();
        let mut dirs: BTreeMap<String, RpfNodeId> = BTreeMap::new();
        for (path, bytes) in files {
            let mut parent = root;
            let mut walked = String::new();
            let parts: Vec<&str> = path.split('/').collect();
            for segment in &parts[..parts.len() - 1] {
                walked.push('/');
                walked.push_str(segment);
                parent = *dirs.entry(walked.clone()).or_insert_with(|| {
                    session
                        .add_folder(parent, segment)
                        .expect("a folder the test just named")
                });
            }
            session
                .add_file_copy(parent, parts[parts.len() - 1], bytes.to_vec())
                .expect("a file the test just named");
        }
        session
    }

    #[test]
    fn a_directory_plans_its_whole_subtree_and_keeps_the_shape() {
        let session = session_with(&[
            ("a.ytd", b"one"),
            ("models/b.ydr", b"two"),
            ("models/deep/c.ydr", b"three"),
        ]);
        let items = plan(&session, session.root_id(), &TypeFilter::All);
        let mut paths: Vec<&str> = items.iter().map(|i| i.rel_path.as_str()).collect();
        paths.sort_unstable();
        assert_eq!(paths, ["a.ytd", "models/b.ydr", "models/deep/c.ydr"]);
    }

    #[test]
    fn a_type_filter_keeps_only_what_it_names() {
        let session = session_with(&[
            ("a.ytd", b"one"),
            ("models/b.ydr", b"two"),
            ("readme", b"three"),
        ]);
        let items = plan(
            &session,
            session.root_id(),
            &TypeFilter::Only(vec!["ydr".to_string()]),
        );
        assert_eq!(items.len(), 1, "one `.ydr` and nothing else");
        assert_eq!(items[0].rel_path, "models/b.ydr");

        // ⚠ An entry with no extension is a real case in an archive and must be
        // reachable, or "everything else" is unexpressible.
        let none = plan(
            &session,
            session.root_id(),
            &TypeFilter::Only(vec![String::new()]),
        );
        assert_eq!(none.len(), 1);
        assert_eq!(none[0].rel_path, "readme");
    }

    #[test]
    fn the_census_counts_what_is_actually_there() {
        let session = session_with(&[
            ("a.ytd", b"1"),
            ("b.YTD", b"2"),
            ("models/c.ydr", b"3"),
            ("plain", b"4"),
        ]);
        let census = extension_census(&session, session.root_id());
        assert_eq!(
            census,
            vec![
                ("ytd".to_string(), 2),
                (String::new(), 1),
                ("ydr".to_string(), 1),
            ],
            "commonest first, and `.YTD` is the same type as `.ytd`"
        );
    }

    /// ⚠ A name out of an archive must not choose a path outside the directory
    /// the gate was asked about.
    ///
    /// Every segment of a planned path has to be one ordinary component: a
    /// `ParentDir`, a root or a drive prefix all reach somewhere the gate never
    /// saw, and `Path::join` honours an absolute right-hand side by replacing
    /// the left.
    #[test]
    fn a_traversing_name_cannot_escape_the_destination() {
        let mut session = session_with(&[("ordinary.bin", b"x")]);
        let root = session.root_id();
        for hostile in [
            "../../escaped.exe",
            "..",
            "C:\\windows\\evil.dll",
            "/etc/passwd",
        ] {
            let node = session
                .add_file_copy(root, "placeholder", b"y".to_vec())
                .expect("add");
            session.node_mut(node).expect("the node just added").name = hostile.to_string();
        }

        let items = plan(&session, root, &TypeFilter::All);
        assert_eq!(items.len(), 5, "every entry is still planned, not dropped");

        let dest = Path::new("C:/tmp/out");
        for item in &items {
            let rel = Path::new(&item.rel_path);
            let odd: Vec<_> = rel
                .components()
                .filter(|c| !matches!(c, std::path::Component::Normal(_)))
                .collect();
            assert!(
                odd.is_empty(),
                "`{}` planned `{}`, whose components include {odd:?}",
                item.rel_path,
                rel.display()
            );
            assert!(
                dest.join(rel).starts_with(dest),
                "`{}` joins to somewhere outside the destination",
                item.rel_path
            );
        }
    }

    #[test]
    fn a_single_file_lands_at_its_own_name() {
        let mut session = RpfEditSession::empty();
        let root = session.root_id();
        let folder = session.add_folder(root, "models").expect("folder");
        let file = session
            .add_file_copy(folder, "b.ydr", b"two".to_vec())
            .expect("file");
        let items = plan(&session, file, &TypeFilter::All);
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].rel_path, "b.ydr",
            "extracting one file must not recreate the folders above it"
        );
    }

    #[test]
    fn what_a_run_writes_is_what_was_planned() {
        let session = session_with(&[("a.ytd", b"one"), ("models/b.ydr", b"two")]);
        let items = plan(&session, session.root_id(), &TypeFilter::All);
        assert_eq!(
            items.len(),
            2,
            "the run below is only worth anything if it has work"
        );

        let dir = tempfile::tempdir().expect("tempdir");
        let report = run_extraction(
            items,
            dir.path().to_path_buf(),
            BodyForm::Decoded,
            false,
            OnClash::Replace,
            Cancel::new(),
            Progress::new(),
        );

        assert_eq!(report.counts.succeeded, 2);
        assert_eq!(report.counts.failed, 0);
        assert_eq!(
            std::fs::read(dir.path().join("a.ytd")).expect("a.ytd"),
            b"one"
        );
        assert_eq!(
            std::fs::read(dir.path().join("models/b.ydr")).expect("b.ydr"),
            b"two"
        );
    }

    /// An archive entry extracts through the VFS in the form asked for, and an
    /// edited one extracts as edited.
    #[test]
    fn an_archive_entry_extracts_through_the_vfs_and_an_edit_wins() {
        use or_io::container::rpf::RpfBuilder;

        let resource = [
            or_io::container::resource::Rsc7::header_from_entry_flags(0x10, 0x20).header_bytes(),
            vec![0u8; 32],
        ]
        .concat();
        let archive = RpfBuilder::default()
            .insert_with("models/a.ydr", resource, false)
            .expect("add")
            .insert_with("b.txt", b"shipped".to_vec(), false)
            .expect("add")
            .build()
            .expect("opens");
        let mut session = RpfEditSession::from_archive(archive).expect("session");
        session
            .set_bytes_for(id_of(&session, "b.txt"), b"edited".to_vec())
            .expect("edit");

        let items = plan(&session, session.root_id(), &TypeFilter::All);
        assert!(items
            .iter()
            .any(|i| matches!(i.source, ExtractSource::Archive { .. })));
        let dir = tempfile::tempdir().expect("tempdir");
        let report = run_extraction(
            items,
            dir.path().to_path_buf(),
            BodyForm::Decoded,
            false,
            OnClash::Replace,
            Cancel::new(),
            Progress::new(),
        );

        assert_eq!(report.counts.failed, 0, "{:?}", report.failures);
        let ydr = std::fs::read(dir.path().join("models/a.ydr")).expect("a.ydr");
        assert_eq!(&ydr[..4], b"RSC7", "a decoded resource carries its header");
        assert_eq!(
            std::fs::read(dir.path().join("b.txt")).expect("b.txt"),
            b"edited"
        );
    }

    #[test]
    fn a_save_of_several_files_writes_none_where_one_is_not_made() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("no such archive.rpf");
        let items = vec![
            ExtractItem::bytes("cell.rsc".into(), b"renumbered".to_vec()),
            ExtractItem::scanned("0x10.rsc".into(), missing, 0x10),
        ];
        let report = run_extraction(
            items,
            dir.path().to_path_buf(),
            BodyForm::Decoded,
            false,
            OnClash::Save,
            Cancel::new(),
            Progress::new(),
        );
        assert_eq!(report.counts.failed, 1, "{:?}", report.failures);
        assert!(
            !dir.path().join("cell.rsc").exists(),
            "the cell is not written alone"
        );
    }

    fn id_of(session: &RpfEditSession, path: &str) -> RpfNodeId {
        match crate::state::navigation::resolve_path(session, path) {
            crate::state::navigation::Resolved::Found(id) => id,
            other => panic!("{path}: {other:?}"),
        }
    }

    fn planned_paths(items: &[ExtractItem]) -> Vec<&str> {
        let mut paths: Vec<&str> = items.iter().map(|i| i.rel_path.as_str()).collect();
        paths.sort_unstable();
        paths
    }

    #[test]
    fn a_selection_extracts_exactly_its_rows_each_under_its_own_name() {
        let session = session_with(&[
            ("a.ytd", b"1"),
            ("b.ytd", b"2"),
            ("models/c.ydr", b"3"),
            ("models/deep/d.ydr", b"4"),
            ("props/e.ydr", b"5"),
        ]);
        let rows = [
            id_of(&session, "a.ytd"),
            id_of(&session, "models"),
            id_of(&session, "models/deep/d.ydr"),
        ];
        let (items, collided) = plan_selection(&session, &rows, &TypeFilter::All);
        assert_eq!(collided, 0);
        assert_eq!(
            planned_paths(&items),
            ["a.ytd", "models/c.ydr", "models/deep/d.ydr"],
            "b.ytd and props/ were not selected; d.ydr is inside a selected \
             folder and is written once"
        );
    }

    #[test]
    fn two_selected_files_with_one_name_do_not_overwrite_each_other() {
        let session = session_with(&[("x/a.ytd", b"first"), ("y/A.ytd", b"second")]);
        let rows = [id_of(&session, "x/a.ytd"), id_of(&session, "y/A.ytd")];
        let (items, collided) = plan_selection(&session, &rows, &TypeFilter::All);
        assert_eq!(collided, 1);
        assert_eq!(planned_paths(&items), ["a.ytd"]);
    }

    #[test]
    fn a_selection_of_one_plans_what_the_single_row_always_did() {
        let session = session_with(&[("models/c.ydr", b"3")]);
        let models = id_of(&session, "models");
        let (items, _) = plan_selection(&session, &[models], &TypeFilter::All);
        assert_eq!(
            planned_paths(&items),
            ["c.ydr"],
            "one folder extracts its contents into the destination, as before"
        );
    }

    /// The warnings of a job of `items` edited saves the worker refuses, and the reason each gives.
    fn refused_saves(items: usize) -> (Vec<String>, String) {
        use or_io::container::rpf::{Rpf9Role, Rpf9Sections};
        use std::time::{Duration, Instant};
        let _serial = crate::ui::theme::serial();
        let dir = tempfile::tempdir().expect("tempdir");
        let archive = dir.path().join("gone.rpf");
        let edit: Arc<super::super::gta6_edit::EditSections> =
            Arc::new(|_: Option<Rpf9Role>, _: &mut Rpf9Sections| Ok::<(), String>(()));
        let items: Vec<_> = (0..items)
            .map(|i| {
                ExtractItem::edited(format!("saved{i}.rsc"), archive.clone(), 0x30, edit.clone())
            })
            .collect();
        let mut state = AppState::new();
        let dest = dir.path().to_path_buf();
        let worker_dest = dest.clone();
        state.extract_jobs.start(
            JobKind::Extraction,
            "saved.rsc".into(),
            dest,
            Vec::new(),
            move |cancel, progress| {
                run_extraction(
                    items,
                    worker_dest,
                    BodyForm::Decoded,
                    false,
                    OnClash::Replace,
                    cancel,
                    progress,
                )
            },
        );
        let deadline = Instant::now() + Duration::from_secs(30);
        while state.extract_jobs.is_running() {
            state.poll_extract_job();
            assert!(Instant::now() < deadline, "the save never ended");
            std::thread::sleep(Duration::from_millis(5));
        }
        let warnings = (state.toasts.iter())
            .filter(|t| t.toast_type == crate::state::ToastType::Warning)
            .map(|t| t.message.clone())
            .collect();
        (warnings, archive.display().to_string())
    }

    /// A save the worker refuses says why in its toast, not only that an item failed.
    #[test]
    fn a_refused_save_says_why() {
        let (warnings, reason) = refused_saves(1);
        assert!(
            (warnings.iter()).any(|m| m.contains("1 failed") && m.contains(&reason)),
            "{warnings:?}"
        );
    }

    /// A job of many items keeps its toast to the counts; the log holds each reason.
    #[test]
    fn a_job_of_many_items_says_its_counts_alone() {
        let (warnings, reason) = refused_saves(2);
        assert!(
            (warnings.iter()).any(|m| m.contains("2 failed") && !m.contains(&reason)),
            "{warnings:?}"
        );
    }
}
