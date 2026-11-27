//! Every kind the app edits, taken from a row of the build through the tree's own open: the
//! smallest edit its editor makes, a save into its row and the plan of the tree's save, printed per
//! kind as a patched copy, a rebuilt archive and why, no edit function, or no row found. A kind is
//! tried on up to three rows before a rebuilt archive or a refusal is what it says.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use or_io::container::rpf::{
    read_rpf9_body, ChunkPatch, Gta6Mount, GtaViKeys, PatchPlan, Rpf9BodyKind, Rpf9Role,
    Rpf9Sections,
};
use or_io::container::{NoOpCryptoProvider, RpfArchive};
use or_io::gta6::headerless::{read_dictionary, Entry, Resource};
use or_io::gta6::map::MapBody;
use or_io::gta6::mission_tables::Body;

use super::super::tests::reading_with;
use super::super::{
    chains, plan_tree, stored_body, tree_edits, write_planned, Edit, Planned, Saved, Writes,
};
use crate::state::document::{DocumentId, RpfNodeId};
use crate::state::gta6_edit::is_scanned_dictionary;
use crate::state::rpf_node::RpfNodeBacking;
use crate::state::rpf_session::{RpfDocument, RpfEditSession};
use crate::state::{AppState, DocPayload, Document, ToastType};
use edits::{
    edit_bound, edit_cell, edit_expression, edit_filter, edit_key, edit_map, edit_material,
    edit_meta, edit_particles, edit_text, edit_texture, edit_types_added, edit_types_member,
    last_error, NO_ENTITY,
};

#[path = "gta6_app_saves_per_kind_edits.rs"]
mod edits;

const WHAT: &str = "every kind the app edits saved through its tree row of the build";
const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
const DOC: DocumentId = 9_700;
/// No row is opened past this; nextest terminates an app test at 300 s.
const BUDGET: Duration = Duration::from_secs(200);
/// An archive is opened only while this much of [`BUDGET`] is left, as its open cannot be stopped.
const ARCHIVE_LEFT: Duration = Duration::from_secs(40);
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
const COPY_CAP: u64 = 1 << 30;
/// The time kept for a plan and a copy past an attempt's two waits, and the least left to write one.
const COPY_MARGIN: Duration = Duration::from_secs(30);
/// The rows of a label tried, the smallest first.
const PER_LABEL: usize = 3;
/// The rows tried of a label that holds several kinds, spread over its sizes.
const PER_MIXED_LABEL: usize = 24;
/// The rows of a kind tried before its last rebuilt archive or refusal is what the table says.
const TRIES: usize = 3;
/// The values of a metadata tree looked at for a scalar to edit.
const VISITS: usize = 100_000;

/// The labels whose rows hold several of the kinds below.
const MULTI: [&str; 4] = ["PSO", "file", "resource", "headerless metadata"];
/// The labels whose rows are spread over their sizes: [`MULTI`]'s, and one only some of whose rows
/// hold what its edit changes (a map data row may hold only instance lists).
const SPREAD: [&str; 5] = ["PSO", "file", "resource", "headerless metadata", "map data"];
/// The labels of rows none of the kinds below is, split at `|`.
const SKIPPED: &str = "archive|script program|acceleration structure|acceleration dictionary|\
    keyed blob|keyed-blob descriptor|asset index|neighbour table|navigation octree|\
    navigation metadata|cloth|morph target|span grid|object trees|GFX|MoVE network|\
    fragment";
/// The labels of the type codes no kind is named by, whose rows the app opens by their head, as the
/// fit census names them; split at `|`.
const BY_HEAD: &str = "resource 0x12|resource 0x45|resource 0x4d|animation dictionary|dictionary";
const FILTERS: &str = "frame filter dictionary";

/// What a kind is called in the table, which [`kind_of`] names it by.
type Kind = &'static str;

type Editor = fn(&mut AppState, DocumentId) -> Result<(), String>;

/// A line of the table: its name, the kind of document its rows open as, and the edit its own
/// editor makes; `None` where no view edits it.
type Line = (Kind, Kind, Option<Editor>);

/// The line whose edit adds an archetype, so its save lays the body out anew and is never patched.
const ADDED: Kind = "map types, archetype added";

const KINDS: [Line; 21] = [
    ("text table", "text table", Some(edit_text)),
    ("PSO table form", "PSO table form", Some(edit_meta)),
    ("PSO P-form", "PSO P-form", Some(edit_meta)),
    (
        "headerless metadata",
        "headerless metadata",
        Some(edit_meta),
    ),
    ("cutscene", "cutscene", Some(edit_meta)),
    ("RBF", "RBF", Some(edit_meta)),
    ("data XML", "data XML", Some(edit_meta)),
    ("drawable", "drawable", Some(edit_material)),
    (
        "drawable dictionary",
        "drawable dictionary",
        Some(edit_material),
    ),
    (
        "texture dictionary",
        "texture dictionary",
        Some(edit_texture),
    ),
    ("bound", "bound", Some(edit_bound)),
    ("navigation mesh", "navigation mesh", Some(edit_cell)),
    ("clip dictionary", "clip dictionary", Some(edit_key)),
    (FILTERS, FILTERS, Some(edit_filter)),
    ("expression set", "expression set", Some(edit_expression)),
    ("map data", "map data", Some(edit_map)),
    ("map types", "map types", Some(edit_types_member)),
    (ADDED, "map types", Some(edit_types_added)),
    ("particles", "particles", Some(edit_particles)),
    ("height map", "height map", Some(edits::edit_height)),
    ("mission tables", "mission tables", Some(edit_meta)),
];

/// The first line of the document kind `kind` with an edit that is not yet settled, and its edit; a
/// kind no line opens as panics, so a misnamed kind fails the test.
fn line_for(kind: Kind, settled: &HashSet<Kind>) -> Option<(Kind, Editor)> {
    assert!(
        KINDS.iter().any(|(_, of, _)| *of == kind),
        "{kind} is no kind of the table"
    );
    let open = |&(line, of, edit): &Line| (of == kind).then_some((line, edit?));
    (KINDS.iter())
        .filter_map(open)
        .find(|(line, _)| !settled.contains(line))
}

/// Whether every line of the kind a row labelled `label` opens as is settled, so none is opened.
fn label_done(label: &str, settled: &HashSet<Kind>) -> bool {
    let mut lines = KINDS.iter().filter(|(_, of, _)| *of == label).peekable();
    !MULTI.contains(&label)
        && lines.peek().is_some()
        && lines.all(|(line, ..)| settled.contains(line))
}

/// The kind the document `doc`, opened from a row labelled `label`, is; `None` for any other.
fn kind_of(doc: &Document, label: &str) -> Option<Kind> {
    let path = doc.path.as_deref()?;
    Some(match &doc.payload {
        DocPayload::Gta6Text(_) => "text table",
        DocPayload::Gta6Expressions(_) => "expression set",
        DocPayload::Gta6Anim(_) if label == "clip dictionary" => "clip dictionary",
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
struct Candidate {
    node: RpfNodeId,
    label: String,
    size: u64,
}

/// The file rows of the tree `DOC` of `state`, nested archives opened as the tree expands them, the
/// smallest of each label; a mixed label's spread over its sizes. A row of a [`BY_HEAD`] label that
/// holds frame filters is labelled [`FILTERS`].
fn walk(state: &mut AppState, until: Instant) -> Vec<Candidate> {
    let mut by_label: HashMap<String, Vec<Candidate>> = HashMap::new();
    let Some(root) = session(state).map(RpfEditSession::root_id) else {
        return Vec::new();
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
                        // Each head read inflates its row, so none is read past the walk's end.
                        if BY_HEAD.split('|').any(|l| l == label)
                            && Instant::now() < until
                            && holds_filters(&archive, entry_index)
                        {
                            label = FILTERS.to_owned();
                        }
                        by_label.entry(label.clone()).or_default().push(Candidate {
                            node: child,
                            label,
                            size,
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
    let mut lists: Vec<VecDeque<Candidate>> = Vec::new();
    for (label, mut rows) in by_label {
        rows.sort_by_key(|c| c.size);
        let n = rows.len();
        let spread = SPREAD.contains(&label.as_str());
        // A label whose rows open as two lines (map types) takes rows enough for both.
        let lines = (KINDS.iter().filter(|(_, of, _)| *of == label)).count();
        let want = if spread {
            PER_MIXED_LABEL
        } else {
            PER_LABEL * lines.max(1)
        };
        let picked: HashSet<usize> = if spread {
            (0..want).map(|i| i * n / want).collect()
        } else {
            (0..want).collect()
        };
        let kept = rows
            .into_iter()
            .enumerate()
            .filter(|(i, _)| picked.contains(i));
        lists.push(kept.map(|(_, c)| c).collect());
    }
    // The labels in turns, so each label's smallest rows come before any label's larger ones.
    let mut out = Vec::new();
    while lists.iter().any(|l| !l.is_empty()) {
        out.extend(lists.iter_mut().filter_map(VecDeque::pop_front));
    }
    out
}

/// The body of the entry whose body starts at `at` in the archive `src`, and whether its row is a
/// headerless resource's.
fn read_back<R: Read + Seek>(
    src: &mut R,
    at: u64,
    keys: &GtaViKeys,
) -> Result<(Vec<u8>, bool), String> {
    let found = chains(&mut *src, &BTreeSet::from([at]), keys)?
        .remove(&at)
        .ok_or("the copy holds no entry there")?;
    let (last, above) = found.chain.split_last().ok_or("the row has no entry")?;
    let mut mount = Gta6Mount::open_root(&mut *src, keys).map_err(|e| e.to_string())?;
    for index in above {
        let next = {
            let e = (mount.archive.entries.iter().find(|e| e.index == *index))
                .ok_or("a nested archive is gone")?;
            mount
                .open_nested(&mut *src, e, keys)
                .map_err(|e| e.to_string())?
        };
        mount = next;
    }
    let e = (mount.archive.entries.iter().find(|e| e.index == *last)).ok_or("the entry is gone")?;
    let body = mount
        .read_entry_decrypted(&mut *src, e, keys)
        .map_err(|e| e.to_string())?;
    Ok((body, found.headerless))
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
        Err(why) if why == NO_ENTITY => return Some((line, Ended::Unfit(why))),
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
                     copy written: the archive is over 1 GiB)",
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

/// `planned` written to a copy under a temp dir, refused unless it is a patched copy whose entry at
/// `at` reads back as `body`.
fn copied(
    disk: &Path,
    keys: &GtaViKeys,
    planned: &Planned,
    at: u64,
    body: &[u8],
) -> Result<(), String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let out = dir.path().join("copy.rpf");
    let saved = write_planned(disk, &out, keys, planned)?;
    if saved != Saved::Patched {
        return Err(format!("the copy is written as {saved:?}"));
    }
    let mut copy = File::open(&out).map_err(|e| e.to_string())?;
    reads_as_edit(&mut copy, disk, at, keys, body)
}

/// The archive at `disk` read with every byte `planned` writes laid over it, as its copy would hold
/// it, refused unless its entry at `at` reads back as `body`; no copy is written.
pub(crate) fn overlaid(
    disk: &Path,
    keys: &GtaViKeys,
    planned: &Planned,
    at: u64,
    body: &[u8],
) -> Result<(), String> {
    let mut writes = Vec::new();
    for plan in &planned.patches {
        writes.extend(plan_writes(plan)?);
    }
    let file = File::open(disk).map_err(|e| e.to_string())?;
    let mut copy = Overlay {
        file,
        writes,
        pos: 0,
    };
    reads_as_edit(&mut copy, disk, at, keys, body)
}

/// Refused unless the entry at `at` of the archive `copy` reads as `body` and not as the one at
/// `disk` does, a headerless row's loose form read as its stored sections.
fn reads_as_edit<R: Read + Seek>(
    copy: &mut R,
    disk: &Path,
    at: u64,
    keys: &GtaViKeys,
    body: &[u8],
) -> Result<(), String> {
    let (read, headerless) = read_back(copy, at, keys)?;
    let mut source = File::open(disk).map_err(|e| e.to_string())?;
    let (source, _) = read_back(&mut source, at, keys)?;
    if read == source {
        return Err("the copy holds the entry as the source does: the edit is lost".into());
    }
    let want = if headerless {
        stored_body(body)
    } else {
        Some(body.to_vec())
    };
    match want.as_deref() == Some(read.as_slice()) {
        true => Ok(()),
        false => Err("the copy does not read the edit back".into()),
    }
}

/// The bytes `plan` lays at absolute offsets of a copy.
// ponytail: `PatchPlan` keeps its writes private to or-io, so they are read from its derived Debug
// form; an accessor there replaces this.
fn plan_writes(plan: &PatchPlan) -> Result<Vec<(u64, Vec<u8>)>, String> {
    writes_said(&format!("{plan:?}"))
}

/// The writes a plan's Debug form `said` states, its last field `writes: [(at, [bytes]), …]`.
fn writes_said(said: &str) -> Result<Vec<(u64, Vec<u8>)>, String> {
    let bad = || format!("the plan's writes do not read from {said:.80}");
    let (_, rest) = said.split_once("writes: [").ok_or_else(bad)?;
    let rest = rest.strip_suffix("] }").ok_or_else(bad)?;
    if rest.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for tuple in rest.split("), (") {
        let tuple = tuple.trim_start_matches('(').trim_end_matches(')');
        let (at, bytes) = tuple.split_once(", [").ok_or_else(bad)?;
        let bytes = bytes.strip_suffix(']').ok_or_else(bad)?;
        let at = at.parse::<u64>().map_err(|_| bad())?;
        let bytes = (bytes.split(", ").filter(|b| !b.is_empty()))
            .map(|b| b.parse::<u8>().map_err(|_| bad()))
            .collect::<Result<Vec<u8>, String>>()?;
        out.push((at, bytes));
    }
    Ok(out)
}

/// A file read with `writes` laid over its bytes at their absolute offsets.
struct Overlay {
    file: File,
    writes: Vec<(u64, Vec<u8>)>,
    pos: u64,
}

impl Read for Overlay {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.seek(SeekFrom::Start(self.pos))?;
        let n = self.file.read(buf)?;
        let (start, end) = (self.pos, self.pos + n as u64);
        for (at, bytes) in &self.writes {
            let (from, to) = ((*at).max(start), (at + bytes.len() as u64).min(end));
            if from < to {
                buf[(from - start) as usize..(to - start) as usize]
                    .copy_from_slice(&bytes[(from - at) as usize..(to - at) as usize]);
            }
        }
        self.pos = end;
        Ok(n)
    }
}

impl Seek for Overlay {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        self.pos = match to {
            SeekFrom::Start(at) => at,
            SeekFrom::End(_) => self.file.seek(to)?,
            SeekFrom::Current(by) => (self.pos.checked_add_signed(by))
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?,
        };
        Ok(self.pos)
    }
}

#[test]
fn a_file_read_through_an_overlay_holds_each_write_at_its_offset_and_the_file_elsewhere() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("file");
    std::fs::write(&path, (0u8..100).collect::<Vec<u8>>()).expect("the file is written");
    let writes = vec![(10, vec![0xA0, 0xA1, 0xA2]), (98, vec![0xB0, 0xB1])];
    let file = File::open(&path).expect("the file opens");
    let mut overlay = Overlay {
        file,
        writes,
        pos: 0,
    };
    let mut whole = Vec::new();
    overlay.read_to_end(&mut whole).expect("it reads");
    let mut want: Vec<u8> = (0u8..100).collect();
    want[10..13].copy_from_slice(&[0xA0, 0xA1, 0xA2]);
    want[98..].copy_from_slice(&[0xB0, 0xB1]);
    assert_eq!(whole, want);
    // A read that starts inside a write takes only the write's bytes from there.
    let mut two = [0u8; 2];
    overlay.seek(SeekFrom::Start(11)).expect("it seeks");
    overlay.read_exact(&mut two).expect("it reads");
    assert_eq!(two, [0xA1, 0xA2]);
    assert_eq!(overlay.seek(SeekFrom::Current(-3)).expect("it seeks"), 10);
    assert_eq!(overlay.seek(SeekFrom::End(0)).expect("it seeks"), 100);
}

#[test]
fn a_plans_writes_read_from_its_debug_form_are_the_writes_it_holds() {
    // The same fields as `PatchPlan`, so its derived Debug form is the one `writes_said` reads.
    #[derive(Debug)]
    #[allow(dead_code)]
    struct PatchPlan {
        base: u64,
        entry: u32,
        ciphered: bool,
        chunks: Vec<ChunkPatch>,
        writes: Vec<(u64, Vec<u8>)>,
    }
    let chunk = ChunkPatch {
        inflated: 4,
        ceiling: 3,
        new_stored: 2,
        zlib: true,
        changed: true,
        optimal: false,
    };
    let writes = vec![
        (4096, vec![0, 255, 7]),
        (8192, Vec::new()),
        (12288, vec![1]),
    ];
    let plan = PatchPlan {
        base: 4096,
        entry: 3,
        ciphered: true,
        chunks: vec![chunk, chunk],
        writes: writes.clone(),
    };
    assert_eq!(writes_said(&format!("{plan:?}")), Ok(writes));
    let none = PatchPlan {
        writes: Vec::new(),
        ..plan
    };
    assert_eq!(writes_said(&format!("{none:?}")), Ok(Vec::new()));
    assert!(writes_said("PatchPlan { base: 0 }").is_err());
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

/// Bounded: archives are taken smallest first, each opened only while 40 s of a 200 s budget are
/// left and walked for at most 15 s through at most 256 nested archives three deep; no row is
/// opened once less than its two 30 s waits and 30 s for a plan and a copy are left of 200 s. A
/// patched copy is written under a temp dir only of an archive of at most 1 GiB; a larger one's
/// edit is read back through the plan's bytes laid over it. Install-gated: it needs the key
/// snapshot and the build, and skips without them.
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
    let mut said: HashMap<Kind, Vec<String>> = HashMap::new();
    let mut unfit: HashMap<Kind, (usize, String)> = HashMap::new();
    let mut settled: HashSet<Kind> = (KINDS.iter())
        .filter_map(|(line, _, edit)| edit.is_none().then_some(*line))
        .collect();
    let (mut panics, mut differs, mut wrong) = (Vec::new(), Vec::new(), Vec::new());
    for (_, disk) in archives(&build) {
        if began.elapsed() + ARCHIVE_LEFT > BUDGET || settled.len() == KINDS.len() {
            break;
        }
        let file = disk
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let crypto = Arc::new(NoOpCryptoProvider::new());
        let Ok(tree) = (RpfArchive::open_with(&disk, crypto).map_err(|e| e.to_string()))
            .and_then(RpfDocument::new)
        else {
            continue;
        };
        let mut base = AppState::new();
        base.documents.push(Document::new(
            DOC,
            Some(disk.clone()),
            DocPayload::Rpf(tree),
        ));
        let until = (Instant::now() + WALK).min(began + BUDGET);
        let candidates = walk(&mut base, until);
        let held = std::mem::take(&mut base.documents)
            .into_iter()
            .find(|d| d.id == DOC);
        drop(base);
        let Some(DocPayload::Rpf(tree)) = held.map(|d| d.payload) else {
            continue;
        };
        for c in &candidates {
            // An attempt waits twice, then plans and copies, so it starts only while all of that fits.
            if began.elapsed() + 2 * STEP + COPY_MARGIN > BUDGET || settled.len() == KINDS.len() {
                break;
            }
            if label_done(&c.label, &settled) {
                continue;
            }
            let tried = catch_unwind(AssertUnwindSafe(|| {
                attempt(&tree, &disk, c, &keys, &settled, began + BUDGET)
            }));
            let where_ = format!("{file}, {} bytes", c.size);
            let (line, ended) = match tried {
                Ok(Some(ended)) => ended,
                Ok(None) => continue,
                Err(payload) => {
                    panics.push(format!(
                        "a {} row ({where_}) panicked: {}",
                        c.label,
                        panic_said(&*payload)
                    ));
                    continue;
                }
            };
            let (s, settles) = match ended {
                Ended::Unfit(s) => {
                    let n = unfit.entry(line).or_default();
                    *n = (n.0 + 1, format!("{s} ({where_})"));
                    continue;
                }
                Ended::Patched(s) if line == ADDED => {
                    wrong.push(format!("{line}: {s} ({where_})"));
                    (s, true)
                }
                Ended::Patched(s) => (s, true),
                Ended::Differs(s) => {
                    differs.push(format!("{line}: {s} ({where_})"));
                    (s, true)
                }
                // The add is expected to rebuild, so its first plan settles it.
                Ended::Rebuilt(s) if line == ADDED => (s, true),
                Ended::Rebuilt(s) | Ended::Failed(s) => (s, false),
            };
            let tries = said.entry(line).or_default();
            tries.push(format!("{s} ({where_})"));
            if settles || tries.len() >= TRIES {
                settled.insert(line);
            }
        }
    }
    // A run that opened nothing, or patched nothing, found nothing to judge the kinds by.
    let opened_any = !said.is_empty();
    let patched_any = (said.values().flatten()).any(|s| s.starts_with("Patched"));
    let table: Vec<(Kind, Vec<String>)> = (KINDS.iter())
        .map(|(line, _, edit)| {
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
}
