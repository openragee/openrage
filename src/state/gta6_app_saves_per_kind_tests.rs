//! Every kind the app edits, taken from a row of the build through the tree's own open: the
//! smallest edit its editor makes, a save into its row and the plan of the tree's save, printed per
//! kind as a patched copy, a rebuilt archive and why, no edit function, or no row found. A kind is
//! tried on up to three rows spread over its sizes before a rebuilt archive or a refusal is what it
//! says; one the small archives leave unsettled is looked for in the archives whose tables hold
//! most rows of its kind.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use or_io::container::rpf::{read_rpf9_body, GtaViKeys, Rpf9BodyKind, Rpf9Role, Rpf9Sections};
use or_io::container::RpfArchive;
use or_io::gta6::headerless::{read_dictionary, Entry, Resource};
use or_io::gta6::map::MapBody;
use or_io::gta6::mission_tables::Body;

use super::super::tests::reading_with;
use super::super::{plan_tree, tree_edits, Edit, Writes};
use crate::state::document::{DocumentId, RpfNodeId};
use crate::state::gta6_edit::is_scanned_dictionary;
use crate::state::rpf_node::RpfNodeBacking;
use crate::state::rpf_session::{RpfDocument, RpfEditSession};
use crate::state::{AppState, DocPayload, Document, ToastType};
use edits::{
    edit_bound, edit_cell, edit_expression, edit_filter, edit_key, edit_map, edit_material,
    edit_meta, edit_particles, edit_text, edit_texture, edit_types_added, edit_types_member,
    last_error, NO_ENTITY, NO_SCALAR,
};
use read_back::copied;
pub(crate) use read_back::overlaid;
use rows::{ranked, spread, Row};

#[path = "gta6_app_saves_per_kind_edits.rs"]
mod edits;
#[path = "gta6_app_saves_per_kind_read_back.rs"]
mod read_back;
#[path = "gta6_app_saves_per_kind_rows.rs"]
mod rows;

const WHAT: &str = "every kind the app edits saved through its tree row of the build";
const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
const DOC: DocumentId = 9_700;
/// No row is opened past this: an attempt starts only while its load's wait ends by then.
const BUDGET: Duration = Duration::from_secs(200);
/// No copy is written, or read back, to end past this; nextest terminates an app test at 300 s.
const END: Duration = Duration::from_secs(260);
/// An archive this size or smaller is a small one, whose rows are tried as its walk finds them.
const SMALL_CAP: u64 = 1 << 30;
/// No row of a small archive is opened past this.
const SMALL_END: Duration = Duration::from_secs(70);
/// The other archives' tables are walked only before this; the rest of [`BUDGET`] is shared among
/// the kinds still unsettled.
const CENSUS_END: Duration = Duration::from_secs(120);
/// The longest one archive's tree is walked.
const WALK: Duration = Duration::from_secs(15);
/// The longest a row's load, and its save into its row, is waited on.
const STEP: Duration = Duration::from_secs(30);
/// The most nested archives one archive's walk opens.
const NESTED_PER_ARCHIVE: usize = 256;
const MAX_DEPTH: usize = 3;
/// The most a sampled row holds inflated.
const ROW_CAP: u64 = 512 << 10;
/// A patched copy is written, and read back, only of an archive this size or smaller; a larger
/// one's edit is read back from the plan's own bytes laid over the archive.
// ponytail: copies of the 0.5 to 0.8 GB archives took most of the run; the overlay reads the same
// bytes, and the small archives still prove the copy's own write.
const COPY_CAP: u64 = 64 << 20;
/// The least time left before [`END`] to write and read back a copy.
const COPY_MARGIN: Duration = Duration::from_secs(30);
/// The rows tried of a label that holds several kinds, as its walk files them.
const PER_MIXED_LABEL: usize = 24;
/// The rows of a kind tried before its last rebuilt archive or refusal is what the table says, and
/// the rows of a label of one kind tried.
const TRIES: usize = 3;
/// The archives an unsettled kind is looked for in.
const PER_KIND: usize = 2;
/// The values of a metadata tree looked at for a scalar to edit.
const VISITS: usize = 100_000;

/// The table kinds whose rows hold several of the kinds below: PSO, file, resource and headerless
/// metadata.
const MULTI: [Row; 4] = [
    Row::Class(0x5D),
    Row::Class(0xFF),
    Row::Class(0x0B),
    Row::Type(0x0B),
];
const MIXED: &[Row] = &MULTI;
/// A table kind only some of whose rows hold what its edit changes (a map data row may hold only
/// instance lists), so its rows are taken as a mixed kind's.
const MAP_DATA: Row = Row::Type(0x49);
/// The labels of rows none of the kinds below is, split at `|`.
const SKIPPED: &str = "archive|script program|acceleration structure|acceleration dictionary|\
    keyed blob|keyed-blob descriptor|asset index|neighbour table|navigation octree|\
    navigation metadata|cloth|morph target|span grid|object trees|GFX|MoVE network|\
    fragment";
/// The type codes no kind is named by, whose rows the app opens by their head, as the fit census
/// names them.
const BY_HEAD: [Row; 5] = [
    Row::Type(0x12),
    Row::Type(0x45),
    Row::Type(0x4D),
    Row::Type(0x56),
    Row::Type(0x63),
];
/// The line of frame filter dictionaries, and what the walk labels a [`BY_HEAD`] row whose head
/// holds frame filters.
const FILTERS: &str = "frame filter dictionary";

/// What a kind is called in the table, which [`kind_of`] names it by.
type Kind = &'static str;

type Editor = fn(&mut AppState, DocumentId) -> Result<(), String>;

/// A line of the table: its name, the kind of document its rows open as, the edit its own editor
/// makes (`None` where no view edits it), and the table kinds of the rows it is found in.
type Line = (Kind, Kind, Option<Editor>, &'static [Row]);

/// The line whose edit adds an archetype, so its save lays the body out anew and is never patched.
const ADDED: Kind = "map types, archetype added";

const KINDS: [Line; 21] = [
    (
        "text table",
        "text table",
        Some(edit_text),
        &[Row::Type(0x6F)],
    ),
    ("PSO table form", "PSO table form", Some(edit_meta), MIXED),
    // A P-form is the body of a resource row.
    (
        "PSO P-form",
        "PSO P-form",
        Some(edit_meta),
        &[Row::Class(0x0B)],
    ),
    (
        "headerless metadata",
        "headerless metadata",
        Some(edit_meta),
        MIXED,
    ),
    ("cutscene", "cutscene", Some(edit_meta), MIXED),
    ("RBF", "RBF", Some(edit_meta), MIXED),
    ("data XML", "data XML", Some(edit_meta), MIXED),
    (
        "drawable",
        "drawable",
        Some(edit_material),
        &[Row::Type(0x02)],
    ),
    (
        "drawable dictionary",
        "drawable dictionary",
        Some(edit_material),
        &[Row::Type(0x04)],
    ),
    (
        "texture dictionary",
        "texture dictionary",
        Some(edit_texture),
        &[Row::Type(0x05)],
    ),
    ("bound", "bound", Some(edit_bound), &[Row::Type(0x06)]),
    (
        "navigation mesh",
        "navigation mesh",
        Some(edit_cell),
        &[Row::Type(0x4F)],
    ),
    (
        "clip dictionary",
        "clip dictionary",
        Some(edit_key),
        &[Row::Type(0x43)],
    ),
    (FILTERS, FILTERS, Some(edit_filter), &[Row::Type(0x56)]),
    (
        "expression set",
        "expression set",
        Some(edit_expression),
        &[Row::Type(0x63)],
    ),
    ("map data", "map data", Some(edit_map), &[MAP_DATA]),
    (
        "map types",
        "map types",
        Some(edit_types_member),
        &[Row::Type(0x4A)],
    ),
    (
        ADDED,
        "map types",
        Some(edit_types_added),
        &[Row::Type(0x4A)],
    ),
    (
        "particles",
        "particles",
        Some(edit_particles),
        &[Row::Type(0x48)],
    ),
    (
        "height map",
        "height map",
        Some(edits::edit_height),
        &[Row::Class(0x69)],
    ),
    ("mission tables", "mission tables", Some(edit_meta), MIXED),
];

/// The first line of the document kind `kind` with an edit that is not yet settled, and its edit; a
/// kind no line opens as panics, so a misnamed kind fails the test.
fn line_for(kind: Kind, settled: &HashSet<Kind>) -> Option<(Kind, Editor)> {
    assert!(
        KINDS.iter().any(|l| l.1 == kind),
        "{kind} is no kind of the table"
    );
    let open = |&(line, of, edit, _): &Line| (of == kind).then_some((line, edit?));
    (KINDS.iter())
        .filter_map(open)
        .find(|(line, _)| !settled.contains(line))
}

/// Whether `label` is the label of one of `rows`.
fn is_one_of(label: &str, rows: &[Row]) -> bool {
    rows.iter().any(|r| r.label() == label)
}

/// The labels of the rows `line` is found in, as the walk files them.
fn labels(line: &Line) -> Vec<Cow<'static, str>> {
    let mut out: Vec<Cow<'static, str>> = line.3.iter().map(|r| r.label()).collect();
    if line.0 == FILTERS && !out.iter().any(|l| l == FILTERS) {
        out.push(FILTERS.into());
    }
    out
}

/// The lines found in rows labelled `label`.
fn lines_of(label: &str) -> Vec<Kind> {
    (KINDS.iter())
        .filter(|l| labels(l).iter().any(|x| x == label))
        .map(|l| l.0)
        .collect()
}

/// Whether every line found in rows labelled `label` is settled, so none of them is opened; never
/// so of a label whose rows hold several kinds or are read by their head.
fn label_done(label: &str, settled: &HashSet<Kind>) -> bool {
    let lines = lines_of(label);
    !is_one_of(label, &MULTI)
        && !is_one_of(label, &BY_HEAD)
        && !lines.is_empty()
        && lines.iter().all(|line| settled.contains(line))
}

/// The kind the document `doc`, opened from a row labelled `label`, is; `None` for any other.
fn kind_of(doc: &Document, label: &str) -> Option<Kind> {
    let path = doc.path.as_deref()?;
    Some(match &doc.payload {
        DocPayload::Gta6Text(_) => "text table",
        DocPayload::Gta6Expressions(_) => "expression set",
        DocPayload::Gta6Anim(_) if Row::Type(0x43).label() == label => "clip dictionary",
        DocPayload::HeightMap(_) => "height map",
        DocPayload::Gta6Drawable(resource) => match &**resource {
            Resource::Bound(_) => "bound",
            Resource::Drawable(_) => "drawable",
            Resource::Dictionary(dict) => {
                let holds = |f: fn(&Entry) -> bool| {
                    dict.entries.iter().any(|(_, e)| e.as_ref().is_ok_and(f))
                };
                if holds(|e| matches!(e, Entry::FrameFilter { .. })) {
                    FILTERS
                } else if holds(|e| matches!(e, Entry::Drawable(_))) {
                    "drawable dictionary"
                } else {
                    return None;
                }
            }
            _ => return None,
        },
        DocPayload::Resource(or_io::Asset::TextureDict(_)) if is_scanned_dictionary(doc) => {
            "texture dictionary"
        }
        DocPayload::Resource(or_io::Asset::NavMesh(_))
            if crate::state::gta6_navmesh::scanned_cell(path).is_some() =>
        {
            "navigation mesh"
        }
        DocPayload::Resource(or_io::Asset::Particles(_))
            if crate::state::gta6_particles::scanned_lines(path).is_some() =>
        {
            "particles"
        }
        DocPayload::Meta(meta) => {
            if let Some(Ok(body)) = crate::state::gta6_map::opened_map(path) {
                return Some(match &*body {
                    MapBody::Data(_) => "map data",
                    MapBody::Types(_) => "map types",
                });
            }
            let raw: &[u8] = &meta.raw_bytes;
            if Body::open(raw).is_ok_and(|b| b.holds_tables()) {
                "mission tables"
            } else if crate::ui::panels::gta6_anim_scene::holds_one(doc) {
                // A headerless RBM body whose root is a scene class, as the app shows its timeline.
                "cutscene"
            } else if meta.is_headerless() {
                "headerless metadata"
            } else if raw.starts_with(b"RBF0") {
                "RBF"
            } else if raw.starts_with(b"PSIN") {
                // `PMAP` after the `PSIN` section is the P-form, else the table form.
                let psin = raw
                    .get(4..8)
                    .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
                match raw.get(psin as usize..psin as usize + 4) {
                    Some(b"PMAP") => "PSO P-form",
                    _ => "PSO table form",
                }
            } else if (raw.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(raw))
                .trim_ascii_start()
                .starts_with(b"<")
            {
                "data XML"
            } else {
                return None;
            }
        }
        _ => return None,
    })
}

/// Whether the headerless row `entry_index` of `archive` is a dictionary holding a frame filter
/// with a track, read at its body as the tree reads a resource: its head's role a dictionary's. A
/// ciphered row is passed over, since its stored bytes are read as they are.
fn holds_filters(archive: &RpfArchive, entry_index: u32) -> bool {
    let Some(place) = archive.rpf9_place(entry_index).filter(|p| !p.ciphered) else {
        return false;
    };
    let read = File::open(&place.file)
        .ok()
        .and_then(|f| read_rpf9_body(f, place.offset, u64::MAX).ok());
    let Some((find, bytes)) = read else {
        return false;
    };
    let role = match &find.kind {
        Rpf9BodyKind::Zlib(run) => run.signature.and_then(|s| s.role()),
        _ => None,
    };
    if role != Some(Rpf9Role::Dictionary) {
        return false;
    }
    let dictionary = Rpf9Sections::of(&find, &bytes)
        .and_then(|s| read_dictionary(&s.virtual_section, Some(s.physical.as_slice())).ok());
    dictionary.is_some_and(|d| {
        (d.entries.iter())
            .any(|(_, e)| matches!(e, Ok(Entry::FrameFilter { tracks, .. }) if !tracks.is_empty()))
    })
}

/// How one row's attempt ended.
enum Ended {
    /// The tree's save planned a patched copy, and how its edit was read back.
    Patched(String),
    /// The tree's save planned a rebuilt archive, and why.
    Rebuilt(String),
    /// A patched copy that does not read back as the edit.
    Differs(String),
    /// Refused before the save planned.
    Failed(String),
    /// The row holds nothing the edit changes, so it is no row to judge the kind by.
    Unfit(String),
}

fn session(state: &AppState) -> Option<&RpfEditSession> {
    match &state.document(DOC)?.payload {
        DocPayload::Rpf(rpf) => Some(&rpf.session),
        _ => None,
    }
}

/// `poll` run until `ended` holds, for at most [`STEP`]; whether it held.
fn waited(state: &mut AppState, poll: fn(&mut AppState), ended: fn(&AppState) -> bool) -> bool {
    let deadline = Instant::now() + STEP;
    while !ended(state) {
        if Instant::now() > deadline {
            return false;
        }
        poll(state);
        std::thread::sleep(Duration::from_millis(5));
    }
    true
}

fn loaded(state: &AppState) -> bool {
    state.load_receiver.is_none() && state.pending_loads.is_empty()
}

fn said_how_it_ended(state: &AppState) -> bool {
    (state.toasts.iter()).any(|t| matches!(t.toast_type, ToastType::Success | ToastType::Error))
}

/// A row to try: its node, its label and what it holds inflated.
#[derive(Clone)]
struct Candidate {
    node: RpfNodeId,
    label: String,
    size: u64,
    /// Whether its head was read and holds frame filters.
    by_head: bool,
}

/// An archive's tree as its walk left it, and the walk's rows of it by label.
struct Walked {
    disk: PathBuf,
    file: String,
    tree: RpfDocument,
    rows: HashMap<String, Vec<Candidate>>,
}

impl Walked {
    /// The archive `disk` opened and walked until `until`, as [`walk`] walks it with `heads`.
    fn open(disk: &Path, until: Instant, heads: bool) -> Option<Self> {
        let crypto = crate::state::rpf_archive_open::gta6_session::crypto();
        let tree = (RpfArchive::open_with(disk, crypto).map_err(|e| e.to_string()))
            .and_then(RpfDocument::new)
            .ok()?;
        let mut base = AppState::new();
        base.documents.push(Document::new(
            DOC,
            Some(disk.to_path_buf()),
            DocPayload::Rpf(tree),
        ));
        let rows = walk(&mut base, until, heads);
        let held = std::mem::take(&mut base.documents)
            .into_iter()
            .find(|d| d.id == DOC);
        let Some(DocPayload::Rpf(tree)) = held.map(|d| d.payload) else {
            return None;
        };
        let file = disk
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Some(Self {
            disk: disk.to_path_buf(),
            file,
            tree,
            rows,
        })
    }

    /// How many of its rows are labelled as `line` is found.
    fn held(&self, line: &Line) -> usize {
        (labels(line).iter())
            .filter_map(|l| self.rows.get(&**l))
            .map(Vec::len)
            .sum()
    }

    /// Its rows labelled as `line` is found that `keep` keeps, asked of the smallest first, all of
    /// them spread over their sizes.
    fn rows_for(&self, line: &Line, mut keep: impl FnMut(&Candidate) -> bool) -> Vec<Candidate> {
        let mut rows: Vec<&Candidate> = (labels(line).iter())
            .filter_map(|l| self.rows.get(&**l))
            .flatten()
            .collect();
        rows.sort_by_key(|c| c.size);
        rows.retain(|c| keep(*c));
        let picked = spread(rows.len(), rows.len()).into_iter();
        picked.map(|i| rows[i].clone()).collect()
    }

    /// Whether the row `node` holds frame filters, as [`holds_filters`] reads its head.
    fn holds_filters(&self, node: RpfNodeId) -> bool {
        let session = &self.tree.session;
        let Some(RpfNodeBacking::ArchiveFile {
            archive_id,
            entry_index,
            ..
        }) = session.node(node).map(|n| &n.backing)
        else {
            return false;
        };
        (session.archive(*archive_id)).is_some_and(|a| holds_filters(&a.archive, *entry_index))
    }
}

/// The rows of `rows` tried as they are walked: each label's spread over its sizes, the labels in
/// turns, so each label's first rows come before any label's later ones. A [`BY_HEAD`] label's
/// rows are its smallest, since a large one, often an animation, can take long to open as nothing
/// the table edits.
fn in_turns(rows: &HashMap<String, Vec<Candidate>>) -> Vec<Candidate> {
    let mut lists: Vec<VecDeque<Candidate>> = Vec::new();
    for (label, rows) in rows {
        let many = is_one_of(label, &MULTI) || MAP_DATA.label() == label.as_str();
        // A label whose rows open as two lines (map types) takes rows enough for both.
        let want = match many {
            true => PER_MIXED_LABEL,
            false => TRIES * lines_of(label).len().max(1),
        };
        let picked = match is_one_of(label, &BY_HEAD) {
            true => (0..want.min(rows.len())).collect(),
            false => spread(rows.len(), want),
        };
        lists.push(picked.into_iter().map(|i| rows[i].clone()).collect());
    }
    let mut out = Vec::new();
    while lists.iter().any(|l| !l.is_empty()) {
        out.extend(lists.iter_mut().filter_map(VecDeque::pop_front));
    }
    out
}

/// The file rows of the tree `DOC` of `state` by label, each label's smallest first, nested
/// archives opened as the tree expands them. With `heads`, a row of a [`BY_HEAD`] kind whose head
/// holds frame filters is labelled [`FILTERS`]; without, no body is read.
fn walk(state: &mut AppState, until: Instant, heads: bool) -> HashMap<String, Vec<Candidate>> {
    let mut by_label: HashMap<String, Vec<Candidate>> = HashMap::new();
    let Some(root) = session(state).map(RpfEditSession::root_id) else {
        return by_label;
    };
    // Nested archives are opened only in the walk's first half, so the second lists what they hold.
    let open_until = Instant::now() + until.saturating_duration_since(Instant::now()) / 2;
    let mut queue = VecDeque::from([(root, 0usize)]);
    let mut opened = 0;
    while let Some((id, depth)) = queue.pop_front() {
        if Instant::now() > until {
            break;
        }
        for child in session(state)
            .map(|s| s.children_of(id))
            .unwrap_or_default()
        {
            let backing = session(state)
                .and_then(|s| s.node(child))
                .map(|n| n.backing.clone());
            match backing {
                Some(RpfNodeBacking::ArchiveFile {
                    archive_id,
                    entry_index,
                    ..
                }) => {
                    let archive = (session(state).and_then(|s| s.archive(archive_id)))
                        .map(|a| Arc::clone(&a.archive));
                    let Some((row, archive)) =
                        archive.and_then(|a| Some((a.rpf9_row(entry_index)?, a)))
                    else {
                        continue;
                    };
                    let size = if row.real > 0 { row.real } else { row.stored };
                    let mut label = row.kind.to_string();
                    let skipped = SKIPPED.split('|').any(|s| s == label)
                        || label.starts_with("type ")
                        || label.starts_with("class ");
                    if size > 0 && size <= ROW_CAP && !skipped {
                        // Each head read inflates its row, so none is read past the walk's end. A
                        // row labelled as filters already is left for the kind's own turn to read.
                        let by_head = heads
                            && label != FILTERS
                            && is_one_of(&label, &BY_HEAD)
                            && Instant::now() < until
                            && holds_filters(&archive, entry_index);
                        if by_head {
                            label = FILTERS.to_owned();
                        }
                        by_label.entry(label.clone()).or_default().push(Candidate {
                            node: child,
                            label,
                            size,
                            by_head,
                        });
                    }
                }
                Some(RpfNodeBacking::Archive { .. })
                    if depth < MAX_DEPTH
                        && opened < NESTED_PER_ARCHIVE
                        && Instant::now() < open_until =>
                {
                    opened += 1;
                    if state.open_archive_row(DOC, child).is_ok() {
                        queue.push_back((child, depth + 1));
                    }
                }
                Some(RpfNodeBacking::ArchiveDir { .. } | RpfNodeBacking::InMemoryDir) => {
                    queue.push_back((child, depth));
                }
                _ => {}
            }
        }
    }
    for rows in by_label.values_mut() {
        rows.sort_by_key(|c| c.size);
    }
    by_label
}

/// Open `c` of the tree `tree` of the archive `disk` in an app of its own, and when it is a kind
/// with a line not yet `settled`, edit it, save it into its row and plan the tree's save; the line,
/// and how it ended.
fn attempt(
    tree: &RpfDocument,
    disk: &Path,
    c: &Candidate,
    keys: &GtaViKeys,
    settled: &HashSet<Kind>,
    until: Instant,
) -> Option<(Kind, Ended)> {
    let mut state = AppState::new();
    state.documents.push(Document::new(
        DOC,
        Some(disk.to_path_buf()),
        DocPayload::Rpf(tree.clone()),
    ));
    state.active_document = Some(DOC);
    state.toasts.clear();
    state.open_rpf_entry(DOC, c.node);
    if !waited(&mut state, AppState::poll_load_result, loaded) {
        return None;
    }
    let row_of = crate::state::rpf_archive_open::tree_open::row_of;
    let doc = (state.documents.iter()).find(|d| {
        d.path
            .as_deref()
            .and_then(|p| row_of(&state, p))
            .map(|r| r.entry_id)
            == Some(c.node)
    })?;
    let (doc, kind) = (doc.id, kind_of(doc, &c.label)?);
    let (line, edit) = line_for(kind, settled)?;
    let failed = |why: String| Some((line, Ended::Failed(why)));
    state.toasts.clear();
    match edit(&mut state, doc) {
        Err(why) if why == NO_ENTITY || why == NO_SCALAR => return Some((line, Ended::Unfit(why))),
        Err(why) => return failed(format!("the edit is refused: {why}")),
        Ok(()) => {}
    }
    if !state
        .document(doc)
        .is_some_and(|d| d.dirty || d.history.can_undo())
    {
        return failed("the edit function changed nothing".into());
    }
    state.active_document = Some(doc);
    state.toasts.clear();
    let asked = state.save_active_document();
    if !waited(&mut state, AppState::poll_save_result, said_how_it_ended) && !asked {
        return failed("the save into its row did not start".into());
    }
    if let Some(why) = last_error(&state) {
        return failed(format!("the save into its row is refused: {why}"));
    }
    let edits = match session(&state).map(tree_edits) {
        Some(Ok(edits)) => edits,
        Some(Err(why)) => return failed(format!("the tree's edits do not list: {why}")),
        None => return failed("the tree is gone".into()),
    };
    let replaced = edits.iter().find_map(|e| match e {
        Edit::Replace { at, body, .. } => Some((*at, Arc::clone(body))),
        _ => None,
    });
    let Some((at, body)) = replaced else {
        return failed("the save left no body in its row".into());
    };
    let planned = match plan_tree(disk, &edits, keys) {
        Ok(planned) => planned,
        Err(why) => return failed(format!("the save does not plan: {why}")),
    };
    let large = std::fs::metadata(disk).map_or(u64::MAX, |m| m.len()) > COPY_CAP;
    let ended = match planned.writes() {
        Writes::Rebuilt => Ended::Rebuilt(format!("Rebuilt: {}", planned.kept.join("; "))),
        Writes::AsRead => {
            Ended::Failed("the save plans the archive as read: the body did not change".into())
        }
        Writes::Patched if Instant::now() + COPY_MARGIN > until => {
            Ended::Patched("Patched (not read back: out of time)".into())
        }
        Writes::Patched => {
            let (read, how) = match large {
                true => (
                    overlaid(disk, keys, &planned, at, &body),
                    "Patched, and the plan's own bytes over the archive read the edit back (no \
                     copy written: the archive is over 64 MiB)",
                ),
                false => (
                    copied(disk, keys, &planned, at, &body),
                    "Patched, and a copy reads the edit back",
                ),
            };
            match read {
                Ok(()) => Ended::Patched(how.into()),
                Err(why) => Ended::Differs(format!("Patched, but {why}")),
            }
        }
    };
    Some((line, ended))
}

fn panic_said(payload: &(dyn std::any::Any + Send)) -> String {
    (payload.downcast_ref::<String>().cloned())
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_default()
}

/// Every `*.rpf` of the build root and of `pack/*/`, the smallest first.
fn archives(root: &Path) -> Vec<(u64, PathBuf)> {
    let listed =
        |d: &PathBuf| (std::fs::read_dir(d).into_iter().flatten().flatten()).map(|e| e.path());
    let mut dirs = vec![root.to_path_buf()];
    dirs.extend(listed(&root.join("pack")));
    let mut out: Vec<(u64, PathBuf)> = (dirs.iter())
        .flat_map(listed)
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("rpf"))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.len(), p)))
        .collect();
    out.sort();
    out
}

/// What a run's attempts found, line by line.
struct Run {
    began: Instant,
    said: HashMap<Kind, Vec<String>>,
    unfit: HashMap<Kind, (usize, String)>,
    settled: HashSet<Kind>,
    /// The rows tried, by archive and node, so none is tried twice.
    tried: HashSet<(PathBuf, RpfNodeId)>,
    panics: Vec<String>,
    differs: Vec<String>,
    wrong: Vec<String>,
}

impl Run {
    fn all_settled(&self) -> bool {
        self.settled.len() == KINDS.len()
    }

    /// `c` of `w` tried once, while its load's wait ends within [`BUDGET`], and how it ended kept.
    fn try_row(&mut self, w: &Walked, c: &Candidate, keys: &GtaViKeys) {
        if self.began.elapsed() + STEP > BUDGET || !self.tried.insert((w.disk.clone(), c.node)) {
            return;
        }
        let (settled, until) = (&self.settled, self.began + END);
        let tried = catch_unwind(AssertUnwindSafe(|| {
            attempt(&w.tree, &w.disk, c, keys, settled, until)
        }));
        let where_ = format!("{}, {} bytes", w.file, c.size);
        let (line, ended) = match tried {
            Ok(Some(ended)) => ended,
            Ok(None) => return,
            Err(payload) => {
                let said = panic_said(&*payload);
                self.panics
                    .push(format!("a {} row ({where_}) panicked: {said}", c.label));
                return;
            }
        };
        let (s, settles) = match ended {
            Ended::Unfit(s) => {
                let n = self.unfit.entry(line).or_default();
                *n = (n.0 + 1, format!("{s} ({where_})"));
                return;
            }
            Ended::Patched(s) if line == ADDED => {
                self.wrong.push(format!("{line}: {s} ({where_})"));
                (s, true)
            }
            Ended::Patched(s) => (s, true),
            Ended::Differs(s) => {
                self.differs.push(format!("{line}: {s} ({where_})"));
                (s, true)
            }
            // The add is expected to rebuild, so its first plan settles it.
            Ended::Rebuilt(s) if line == ADDED => (s, true),
            Ended::Rebuilt(s) | Ended::Failed(s) => (s, false),
        };
        let tries = self.said.entry(line).or_default();
        tries.push(format!("{s} ({where_})"));
        if settles || tries.len() >= TRIES {
            self.settled.insert(line);
        }
    }
}

/// The archives of `census` a line not yet `settled` is looked for in: for each, the
/// [`PER_KIND`] holding most rows of its kind.
fn kept(census: &[Walked], settled: &HashSet<Kind>) -> HashSet<usize> {
    (KINDS.iter())
        .filter(|l| !settled.contains(l.0))
        .flat_map(|l| {
            let counts: Vec<usize> = census.iter().map(|w| w.held(l)).collect();
            ranked(&counts, PER_KIND)
        })
        .collect()
}

/// Bounded: archives are taken smallest first. A small one (at most 1 GiB) is walked for at most
/// 15 s and its rows tried until 70 s; every other archive's table is walked, for an equal share of
/// what is left until 120 s, through at most 256 nested archives three deep, with no body read;
/// then each kind still unsettled is looked for, for an equal share of what is left, in the two
/// archives whose tables hold most rows of its kind. No row is opened once its 30 s load would end
/// past 200 s, and no copy is begun with less than 30 s left of 260 s. A patched copy is written
/// under a temp dir only of an archive of at most 64 MiB; a larger one's edit is read back through
/// the plan's bytes laid over it. Install-gated: it needs the key snapshot and the build, and skips
/// without them.
#[test]
fn every_kind_the_app_edits_saved_through_its_tree_row_of_the_build_plans_patched_or_says_why() {
    let began = Instant::now();
    let build = (std::env::var_os(BUILD_ENV).map(PathBuf::from)).filter(|p| p.is_dir());
    let keys = (build.as_ref()).and_then(|_| crate::state::rpf9_scan::rpf9_table::keys().ok());
    let (Some(build), Some(keys)) = (build, keys) else {
        or_test_support::skip_install(WHAT, format_args!("set OPENRAGE_GTA6_KEYS and {BUILD_ENV}"));
        return;
    };
    let _serial = crate::ui::theme::serial();
    let _reading = reading_with(Arc::clone(&keys));
    let mut run = Run {
        began,
        said: HashMap::new(),
        unfit: HashMap::new(),
        settled: (KINDS.iter())
            .filter_map(|(line, _, edit, _)| edit.is_none().then_some(*line))
            .collect(),
        tried: HashSet::new(),
        panics: Vec::new(),
        differs: Vec::new(),
        wrong: Vec::new(),
    };
    // The small archives, each label's rows tried as its walk files them; one left part way is
    // looked in again for a kind still unsettled.
    let (mut census, mut later) = (Vec::new(), Vec::new());
    for (size, disk) in archives(&build) {
        if size > SMALL_CAP || began.elapsed() + WALK > SMALL_END {
            later.push(disk);
            continue;
        }
        let until = (Instant::now() + WALK).min(began + SMALL_END);
        let Some(walked) = Walked::open(&disk, until, true) else {
            continue;
        };
        let mut cut = false;
        for c in in_turns(&walked.rows) {
            if run.all_settled() {
                break;
            }
            if began.elapsed() > SMALL_END {
                cut = true;
                break;
            }
            if !label_done(&c.label, &run.settled) {
                run.try_row(&walked, &c, &keys);
            }
        }
        if cut {
            census.push(walked);
        }
    }
    // Every other archive's table, kept while a kind still unsettled is looked for in it.
    for (at, disk) in later.iter().enumerate() {
        let left = (began + CENSUS_END).saturating_duration_since(Instant::now());
        if left.is_zero() || run.all_settled() {
            break;
        }
        let share = (left / (later.len() - at) as u32).min(WALK);
        census.extend(Walked::open(disk, Instant::now() + share, false));
        let keep = kept(&census, &run.settled);
        let mut i = 0;
        census.retain(|_| {
            i += 1;
            keep.contains(&(i - 1))
        });
    }
    // Each kind still unsettled, those of one table kind first, for an equal share of what is left.
    let mut open: Vec<Line> = (KINDS.into_iter())
        .filter(|l| !run.settled.contains(l.0))
        .collect();
    open.sort_by_key(|l| l.3.iter().any(|r| MULTI.contains(r)));
    let last = began + BUDGET - STEP;
    for (n, line) in open.iter().enumerate() {
        if run.settled.contains(line.0) {
            continue;
        }
        let now = Instant::now();
        let until = now + last.saturating_duration_since(now) / (open.len() - n) as u32;
        // An animation row can take long to open, so one is opened only once its head, read in the
        // first half of the kind's time, holds filters.
        let (filters, heads_until) = (line.0 == FILTERS, now + (until - now) / 2);
        let counts: Vec<usize> = census.iter().map(|w| w.held(line)).collect();
        'kind: for i in ranked(&counts, PER_KIND) {
            let w = &census[i];
            let keep = |c: &Candidate| {
                !filters || c.by_head || (Instant::now() < heads_until && w.holds_filters(c.node))
            };
            for c in w.rows_for(line, keep) {
                if run.settled.contains(line.0) || Instant::now() > until {
                    break 'kind;
                }
                run.try_row(w, &c, &keys);
            }
        }
    }
    let Run {
        mut said,
        mut unfit,
        panics,
        differs,
        wrong,
        ..
    } = run;
    // A run that opened nothing, or patched nothing, found nothing to judge the kinds by; and every
    // kind of the table was driven to a patched copy on the build, so the build holds rows of each.
    let opened_any = !said.is_empty();
    let patched_any = (said.values().flatten()).any(|s| s.starts_with("Patched"));
    let unfound: Vec<Kind> = (KINDS.iter())
        .filter(|l| l.2.is_some() && !said.contains_key(l.0) && !unfit.contains_key(l.0))
        .map(|l| l.0)
        .collect();
    let table: Vec<(Kind, Vec<String>)> = (KINDS.iter())
        .map(|(line, _, edit, _)| {
            let lines = match (edit, said.remove(line), unfit.remove(line)) {
                (None, ..) => vec!["no edit function".to_owned()],
                (Some(_), Some(tries), _) => tries,
                (Some(_), None, Some((n, last))) => {
                    vec![format!(
                        "no row of {n} tried holds what the edit changes: {last}"
                    )]
                }
                (Some(_), None, None) => vec!["no row found".to_owned()],
            };
            (*line, lines)
        })
        .collect();
    println!("{WHAT}, in {:.0} s:", began.elapsed().as_secs_f64());
    for (name, lines) in &table {
        for (i, said) in lines.iter().enumerate() {
            let name = if i == 0 { *name } else { "" };
            println!("  {name:<28} {said}");
        }
    }
    for p in &panics {
        println!("  {p}");
    }
    assert!(panics.is_empty(), "a kind panicked: {panics:?}");
    assert!(
        differs.is_empty(),
        "a patched copy does not read back its edit: {differs:?}"
    );
    assert!(
        wrong.is_empty(),
        "an archetype added lays the body out anew, so it never plans a patched copy: {wrong:?}"
    );
    let listed: Vec<&str> = table.iter().map(|(name, _)| *name).collect();
    assert_eq!(
        listed,
        KINDS.map(|(name, ..)| name),
        "the table lists every kind"
    );
    assert!(
        opened_any,
        "no row of the build opened as a kind the app edits: {table:?}"
    );
    assert!(patched_any, "no kind planned a patched copy: {table:?}");
    assert!(
        unfound.is_empty(),
        "the build holds rows of these kinds, yet none was found: {unfound:?}"
    );
}
