//! Opening, off the UI thread, what an extraction names before it is planned.
//!
//! ⚠ A row the tree has not expanded holds no rows, so a plan of it would hold
//! nothing. Opening an install's archives is minutes of reading, so the rows
//! open on the expansion workers a few at a time, and the plan is built once
//! the last has.

use std::collections::HashSet;
use std::path::PathBuf;

use or_io::container::rpf::BodyForm;

use super::document::{DocumentId, RpfNodeId};
use super::extract_job::{extension_census, extraction_label, ExtractStart, TypeFilter};
use super::jobs::{JobKind, RunningJob};
use super::rpf_node::RpfNodeBacking;
use super::rpf_session::RpfEditSession;
use super::{AppState, DocPayload};

/// ⚠ The cycle guard as well: a nested archive's entry can name the bytes of the one holding it.
const DEEPEST: usize = 8;

/// Rows opening at once: each is a thread reading an archive's table of contents.
const AT_ONCE: usize = 4;

/// The rows under some roots that the tree has not opened, and how far opening them has got.
pub(super) struct OpenUnder {
    doc_id: DocumentId,
    /// Rows waiting to open, each with how many archives deep it sits.
    pending: Vec<(RpfNodeId, usize)>,
    /// Rows this has asked a worker to open, so one that fails is not asked again.
    asked: HashSet<RpfNodeId>,
    failed: Vec<String>,
    /// Rows that would not open, here or in an earlier walk, left alone and not told again.
    skip: HashSet<RpfNodeId>,
    opened: usize,
    /// The files on disk the archives it has asked for are read from.
    reading: Vec<PathBuf>,
}

enum Look {
    /// The row is open: these folders under it are next.
    Open(Vec<(RpfNodeId, usize)>),
    /// A worker is opening it.
    Waiting,
    /// Left for a later frame, because enough rows are opening already.
    Held,
    Failed(String),
}

impl OpenUnder {
    pub(super) fn new(doc_id: DocumentId, roots: &[RpfNodeId]) -> Self {
        Self {
            doc_id,
            pending: roots.iter().map(|&root| (root, 0)).collect(),
            asked: HashSet::new(),
            failed: Vec::new(),
            skip: HashSet::new(),
            opened: 0,
            reading: Vec::new(),
        }
    }

    /// How far the opening has got, for the jobs list.
    pub(super) fn progress_line(&self) -> String {
        format!("{} opened, {} to open", self.opened, self.left())
    }

    /// How many rows are waiting to open.
    pub(super) fn left(&self) -> usize {
        self.pending.len()
    }

    /// One frame's work: look under every row that is open, and ask for the ones that are not; `true` once nothing under the roots waits.
    pub(super) fn step(&mut self, state: &mut AppState) -> bool {
        state.poll_expansions();
        let mut stack = std::mem::take(&mut self.pending);
        let mut busy = 0;
        while let Some((id, depth)) = stack.pop() {
            match self.look(state, id, depth, busy) {
                Look::Open(folders) => stack.extend(folders),
                Look::Waiting => {
                    busy += 1;
                    self.pending.push((id, depth));
                }
                Look::Held => self.pending.push((id, depth)),
                Look::Failed(why) => {
                    self.failed.push(why);
                    self.skip.insert(id);
                }
            }
        }
        self.pending.is_empty()
    }

    fn look(&mut self, state: &mut AppState, id: RpfNodeId, depth: usize, busy: usize) -> Look {
        let doc_id = self.doc_id;
        let in_flight = state.expander.is_in_flight(doc_id, id);
        let Some(DocPayload::Rpf(rpf)) = state.document(doc_id).map(|d| &d.payload) else {
            return Look::Open(Vec::new());
        };
        let Some(node) = rpf.session.node(id) else {
            return Look::Open(Vec::new());
        };
        let (is_archive, open_error) = match &node.backing {
            RpfNodeBacking::Archive { open_error, .. }
            | RpfNodeBacking::Rpf9Archive { open_error, .. } => (true, open_error.clone()),
            _ => (false, None),
        };
        let unopened = rpf.session.needs_expansion(id);
        let named = || rpf.session.path_of(id).unwrap_or_default();
        let below = depth + usize::from(is_archive);
        let folders = || -> Vec<(RpfNodeId, usize)> {
            (rpf.session.children_of(id).into_iter())
                .filter(|&child| rpf.session.node(child).is_some_and(|n| n.is_dir()))
                .map(|child| (child, below))
                .collect()
        };
        if in_flight {
            return Look::Waiting;
        }
        if self.asked.contains(&id) {
            if unopened {
                return Look::Failed(format!("`{}` could not be read", named()));
            }
            if let Some(error) = open_error {
                return Look::Failed(format!("`{}`: {error}", named()));
            }
            self.opened += 1;
            self.asked.remove(&id);
            return Look::Open(folders());
        }
        if let Some(error) = open_error {
            if self.skip.contains(&id) {
                return Look::Open(Vec::new());
            }
            return Look::Failed(format!("`{}`: {error}", named()));
        }
        if !unopened {
            return Look::Open(folders());
        }
        if self.skip.contains(&id) {
            return Look::Open(Vec::new());
        }
        if is_archive && depth >= DEEPEST {
            return Look::Failed(format!(
                "`{}` is more than {DEEPEST} archives deep",
                named()
            ));
        }
        if busy >= AT_ONCE {
            return Look::Held;
        }
        // ⚠ Archives only: asking a folder to list reads no file, and its path would hold every file under it.
        if let Some(file) = state
            .affected_disk_path(doc_id, id)
            .filter(|file| is_archive && !self.reading.contains(file))
        {
            self.reading.push(file);
        }
        state.request_expand(doc_id, id);
        self.asked.insert(id);
        Look::Waiting
    }
}

/// How far the count of "Only one type" under a row has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Counting {
    /// Nothing is opening for it: this many rows under the row are unread, and `count_types_under` reads them.
    Idle { unread: usize },
    /// Rows are opening for it: this many wait to open.
    Opening { left: usize },
}

/// How many rows at or under `node` the tree has not read: folders on disk it has not listed, and archives it has not opened.
fn unread_under(session: &RpfEditSession, node: RpfNodeId, told: &HashSet<RpfNodeId>) -> usize {
    let mut stack = vec![node];
    let mut unread = 0;
    while let Some(id) = stack.pop() {
        unread += usize::from(session.needs_expansion(id) && !told.contains(&id));
        stack.extend(session.children_of(id));
    }
    unread
}

/// An extraction whose rows are opening, before its plan is built.
pub(super) struct Planning {
    walk: OpenUnder,
    nodes: Vec<RpfNodeId>,
    dest: PathBuf,
    form: BodyForm,
    filter: TypeFilter,
    label: String,
    /// The files on disk the rows named sit in or are, kept from being deleted, renamed or moved while they open.
    roots: Vec<PathBuf>,
}

impl Planning {
    /// What a refusal calls it.
    pub(super) fn name(&self) -> String {
        format!("the extraction of {}", self.label)
    }

    /// The archives on disk opened for it so far: all a save can replace under it.
    pub(super) fn reads(&self) -> &[PathBuf] {
        &self.walk.reading
    }

    pub(super) fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    pub(super) fn job(&self) -> RunningJob {
        RunningJob {
            kind: JobKind::Extraction,
            doing: format!("Opening what {} holds", self.label),
            fraction: None,
            detail: self.walk.progress_line(),
            stopping: false,
        }
    }

    fn start(self, state: &mut AppState) -> ExtractStart {
        state.tell_unopened(&self.walk.failed, "extracted");
        state.extract_rpf_nodes_inner(
            self.walk.doc_id,
            &self.nodes,
            self.dest,
            self.form,
            self.filter,
            None,
            None,
        )
    }
}

impl AppState {
    /// Extract `nodes` into `dest`, opening what they hold first: here at once when nothing waits to open, else over the frames that follow.
    pub(super) fn begin_extraction(
        &mut self,
        doc_id: DocumentId,
        nodes: &[RpfNodeId],
        dest: PathBuf,
        form: BodyForm,
        filter: TypeFilter,
    ) -> ExtractStart {
        let Some(DocPayload::Rpf(rpf)) = self.document(doc_id).map(|d| &d.payload) else {
            return ExtractStart::NothingToDo;
        };
        let label = extraction_label(&rpf.session, nodes);
        let roots = (nodes.iter())
            .filter_map(|&node| self.affected_disk_path(doc_id, node))
            .collect();
        let mut planning = Planning {
            walk: OpenUnder::new(doc_id, nodes),
            nodes: nodes.to_vec(),
            dest,
            form,
            filter,
            label,
            roots,
        };
        if planning.walk.step(self) {
            return planning.start(self);
        }
        self.extract_jobs.planning = Some(planning);
        ExtractStart::Planning
    }

    /// The per-frame tick of an extraction still opening its rows.
    pub(super) fn advance_planning(&mut self) {
        let Some(mut planning) = self.extract_jobs.planning.take() else {
            return;
        };
        if self.document(planning.walk.doc_id).is_none() {
            return;
        }
        if planning.walk.step(self) {
            planning.start(self);
        } else {
            self.extract_jobs.planning = Some(planning);
        }
    }

    /// How many files of each extension the tree holds at or under `node`, and how the read of the rest has got.
    ///
    /// ⚠ Reads nothing unless [`count_types_under`](Self::count_types_under)
    /// started it: an install's root is minutes of reading, and a menu being
    /// open is not a request for that. A started count opens a few more rows
    /// each call, which the menu makes every frame it is open.
    pub fn census_of(
        &mut self,
        doc_id: DocumentId,
        node: RpfNodeId,
    ) -> (Vec<(String, usize)>, Counting) {
        let told = self.told_not_to_list(doc_id);
        let opening = match self.extract_jobs.census.take() {
            Some((doc, root, mut walk)) if (doc, root) == (doc_id, node) => {
                if walk.step(self) {
                    self.finish_census_walk(doc, &walk);
                    None
                } else {
                    let left = walk.left();
                    self.extract_jobs.census = Some((doc, root, walk));
                    Some(Counting::Opening { left })
                }
            }
            other => {
                self.extract_jobs.census = other;
                None
            }
        };
        let Some(DocPayload::Rpf(rpf)) = self.document(doc_id).map(|d| &d.payload) else {
            return (Vec::new(), Counting::Idle { unread: 0 });
        };
        let unread = unread_under(&rpf.session, node, &told);
        (
            extension_census(&rpf.session, node),
            opening.unwrap_or(Counting::Idle { unread }),
        )
    }

    /// The per-frame tick of a count of "Only one type", so that closing the menu that shows it does not park it.
    pub(super) fn advance_census(&mut self) {
        let Some((doc, root, mut walk)) = self.extract_jobs.census.take() else {
            return;
        };
        if self.document(doc).is_none() {
            return;
        }
        if walk.step(self) {
            self.finish_census_walk(doc, &walk);
        } else {
            self.extract_jobs.census = Some((doc, root, walk));
        }
    }

    /// A count that has opened what it can: what would not open is told, and not asked for again.
    fn finish_census_walk(&mut self, doc: DocumentId, walk: &OpenUnder) {
        let failed = walk.skip.iter().map(|&row| (doc, row));
        self.extract_jobs.census_failed.extend(failed);
        self.tell_unopened(&walk.failed, "counted");
    }

    /// Start opening every folder and archive under `node` the tree has not, for the counts of "Only one type".
    pub fn count_types_under(&mut self, doc_id: DocumentId, node: RpfNodeId) {
        let mut walk = OpenUnder::new(doc_id, &[node]);
        walk.skip = self.told_not_to_list(doc_id);
        self.extract_jobs.census = Some((doc_id, node, walk));
    }

    /// The rows of `doc_id` a count told would not open, less the folders that list again since, which are asked for afresh.
    fn told_not_to_list(&mut self, doc_id: DocumentId) -> HashSet<RpfNodeId> {
        let mended: Vec<_> = (self.extract_jobs.census_failed.iter())
            .filter(|(doc, row)| *doc == doc_id && self.may_read_again(doc_id, *row))
            .copied()
            .collect();
        for row in mended {
            self.extract_jobs.census_failed.remove(&row);
        }
        (self.extract_jobs.census_failed.iter())
            .filter(|(doc, _)| *doc == doc_id)
            .map(|(_, row)| *row)
            .collect()
    }

    /// Ask the workers to open every row under `nodes` the tree has not, and answer at once: `Ok` once none waits.
    ///
    /// ⚠ Never waits: it runs on the frame that starts a drag, with the mouse
    /// button held. The rows go on opening over the frames that follow
    /// ([`advance_drag_opening`](Self::advance_drag_opening)), which say when the
    /// drag can be tried again.
    ///
    /// ⚠ A row that would not list or open stays unopened, so asking for it
    /// afresh on every drag would refuse every drag: once the failure is told,
    /// every drag of the same rows goes ahead without it, until other rows are
    /// dragged. Rows the tree has regained unopened since are still asked for.
    pub(super) fn open_rows_for_drag(
        &mut self,
        doc_id: DocumentId,
        nodes: &[RpfNodeId],
    ) -> Result<(), String> {
        let mut walk = match self.extract_jobs.drag_opening.take() {
            Some((doc, rows, walk)) if doc == doc_id && rows == nodes => walk,
            _ => {
                let mut walk = OpenUnder::new(doc_id, nodes);
                let told = (self.extract_jobs.drag_told.as_ref())
                    .filter(|(doc, rows, _)| *doc == doc_id && rows == nodes)
                    .map(|(_, _, told)| told.clone());
                // ⚠ A folder that lists again was mended since, and is asked for afresh: an archive cannot be told so without opening it
                walk.skip = (told.into_iter().flatten())
                    .filter(|&row| !self.folder_lists_again(doc_id, row))
                    .collect();
                walk
            }
        };
        if walk.step(self) {
            self.finish_drag_walk(doc_id, nodes, walk);
            return Ok(());
        }
        let left = walk.left();
        self.extract_jobs.drag_opening = Some((doc_id, nodes.to_vec(), walk));
        Err(format!(
            "{left} folder(s) or archive(s) under the selection are opening: \
             drag it again once they have, or use Extract… instead"
        ))
    }

    /// Whether a row a count told would not open may be asked for again: a folder that lists now, or an archive unloaded since, which the tree reads afresh.
    fn may_read_again(&self, doc_id: DocumentId, row: RpfNodeId) -> bool {
        let unloaded = !self.is_disk_folder(doc_id, row)
            && matches!(
                self.document(doc_id).map(|d| &d.payload),
                Some(DocPayload::Rpf(rpf)) if rpf.session.needs_expansion(row)
            );
        unloaded || self.folder_lists_again(doc_id, row)
    }

    /// Whether the row is a folder on disk that can be listed now.
    fn folder_lists_again(&self, doc_id: DocumentId, row: RpfNodeId) -> bool {
        self.disk_path_for_node(doc_id, row)
            .filter(|_| self.is_disk_folder(doc_id, row))
            .is_some_and(|path| std::fs::read_dir(path).is_ok())
    }

    /// The per-frame tick of the rows a drag out asked to open.
    pub(super) fn advance_drag_opening(&mut self) {
        let Some((doc, nodes, mut walk)) = self.extract_jobs.drag_opening.take() else {
            return;
        };
        if self.document(doc).is_none() {
            return;
        }
        if !walk.step(self) {
            self.extract_jobs.drag_opening = Some((doc, nodes, walk));
            return;
        }
        self.finish_drag_walk(doc, &nodes, walk);
        self.show_info_toast("What you dragged is open: drag it again");
    }

    /// Tell what would not open, and remember it for the next drag of the same rows.
    fn finish_drag_walk(&mut self, doc: DocumentId, nodes: &[RpfNodeId], walk: OpenUnder) {
        self.tell_unopened(&walk.failed, "dragged out");
        self.extract_jobs.drag_told =
            (!walk.skip.is_empty()).then(|| (doc, nodes.to_vec(), walk.skip));
    }

    fn tell_unopened(&mut self, failed: &[String], then: &str) {
        if let Some(first) = failed.first() {
            self.show_error_toast(format!(
                "{} folder(s) or archive(s) did not open, so nothing in them is {then}: {first}",
                failed.len()
            ));
        }
    }
}
