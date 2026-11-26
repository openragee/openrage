//! Every kind the app edits, taken from a row of the build through the tree's own open: the
//! smallest edit its editor makes, a save into its row and the plan of the tree's save, printed per
//! kind as a patched copy, a rebuilt archive and why, no edit function, or no row found.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::fs::File;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::{Quat, Vec3};
use or_io::authoring::{is_encodable, FormatChoice, TextureBuildOptions};
use or_io::codec::texture::{decode_level, TextureSource as _};
use or_io::container::rpf::{Gta6Mount, GtaViKeys};
use or_io::container::{NoOpCryptoProvider, RpfArchive};
use or_io::gta5::particle::PtxEffectRule;
use or_io::gta6::anim::{AnimAt, AnimEdit, ExpressionGroup};
use or_io::gta6::headerless::{Entry, ParameterValue, Resource};
use or_io::gta6::map::MapBody;
use or_io::gta6::mission_tables::Body;
use or_io::meta::{MetaInstance, Value};
use or_maps::edit::Pose;

use super::super::tests::reading_with;
use super::super::{
    chains, plan_tree, stored_body, tree_edits, write_planned, Edit, Planned, Saved, Writes,
};
use crate::state::document::{DocumentId, RpfNodeId};
use crate::state::gta6_edit::{is_scanned_dictionary, MaterialEdit};
use crate::state::gta6_filter_edit::FilterEdit;
use crate::state::gta6_physics_edit::BoundEdit;
use crate::state::meta_edit::value_at;
use crate::state::presets::MetaStep;
use crate::state::rpf_node::RpfNodeBacking;
use crate::state::rpf_session::{RpfDocument, RpfEditSession};
use crate::state::{AppState, Change, DocPayload, Document, MetaEdit, ToastType};

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
const NESTED_PER_ARCHIVE: usize = 64;
const MAX_DEPTH: usize = 3;
/// The most a sampled row holds inflated.
const ROW_CAP: u64 = 512 << 10;
/// A patched copy is written, and read back, only of an archive this size or smaller.
const COPY_CAP: u64 = 1 << 30;
/// The time kept for a plan and a copy past an attempt's two waits, and the least left to write one.
const COPY_MARGIN: Duration = Duration::from_secs(30);
/// The rows of a label tried, the smallest first.
const PER_LABEL: usize = 3;
/// The rows tried of a label that holds several kinds, spread over its sizes.
const PER_MIXED_LABEL: usize = 24;
/// The rows of a kind tried before its last refusal is what the table says.
const TRIES: usize = 3;
/// The values of a metadata tree looked at for a scalar to edit.
const VISITS: usize = 100_000;

/// The labels whose rows hold several of the kinds below.
const MIXED: [&str; 3] = ["PSO", "file", "resource"];
/// The labels of rows none of the kinds below is, split at `|`.
const SKIPPED: &str = "archive|script program|acceleration structure|acceleration dictionary|\
    keyed blob|keyed-blob descriptor|asset index|neighbour table|navigation octree|\
    navigation metadata|cloth|morph target|span grid|object trees|GFX|MoVE network|height map|\
    fragment";

/// What a kind is called in the table, which [`kind_of`] names it by.
type Kind = &'static str;

type Editor = fn(&mut AppState, DocumentId) -> Result<(), String>;

/// Each kind the app edits and the edit its own editor makes; `None` where no view edits it, as the
/// inspector shows a height map's facts and edits none.
const KINDS: [(Kind, Option<Editor>); 19] = [
    ("text table", Some(edit_text)),
    ("PSO table form", Some(edit_meta)),
    ("PSO P-form", Some(edit_meta)),
    ("headerless metadata", Some(edit_meta)),
    ("RBF", Some(edit_meta)),
    ("data XML", Some(edit_meta)),
    ("drawable", Some(edit_material)),
    ("drawable dictionary", Some(edit_material)),
    ("texture dictionary", Some(edit_texture)),
    ("bound", Some(edit_bound)),
    ("navigation mesh", Some(edit_cell)),
    ("clip dictionary", Some(edit_key)),
    ("frame filter dictionary", Some(edit_filter)),
    ("expression set", Some(edit_expression)),
    ("map data", Some(edit_map)),
    ("map types", Some(edit_types)),
    ("particles", Some(edit_particles)),
    ("height map", None),
    ("mission tables", Some(edit_meta)),
];

/// The editor of `kind`; a name [`KINDS`] does not hold panics, so a misnamed kind fails the test.
fn editor(kind: Kind) -> Option<Editor> {
    let row = KINDS.iter().find(|(k, _)| *k == kind);
    row.unwrap_or_else(|| panic!("{kind} is no kind of the table"))
        .1
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
                    "frame filter dictionary"
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

fn last_error(state: &AppState) -> Option<String> {
    (state.toasts.iter())
        .filter(|t| matches!(t.toast_type, ToastType::Error))
        .last()
        .map(|t| t.message.clone())
}

fn refused(state: &AppState, made: bool, what: &str) -> Result<(), String> {
    match made {
        true => Ok(()),
        false => Err(last_error(state).unwrap_or_else(|| format!("the {what} edit was not made"))),
    }
}

fn text_at(p: &mut DocPayload, row: usize) -> Option<&mut String> {
    match p {
        DocPayload::Gta6Text(t) => t.text_mut(row),
        _ => None,
    }
}

/// The first text holding a letter, that letter's case changed, as the table's cell records it.
fn edit_text(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
    let Some(DocPayload::Gta6Text(table)) = state.document(doc).map(|d| &d.payload) else {
        return Err("not a text table".into());
    };
    let found = (0usize..)
        .map_while(|row| table.row(row).map(|(_, _, text)| (row, text)))
        .find_map(|(row, text)| Some((row, text.bytes().position(|b| b.is_ascii_alphabetic())?)));
    let (row, at) = found.ok_or("no text holds a letter")?;
    let text = (state.document_mut(doc))
        .and_then(|d| text_at(&mut d.payload, row))
        .ok_or("the text is gone")?;
    let before = text.clone();
    // A letter's case is its 0x20 bit, so the text keeps its byte length.
    let mut bytes = std::mem::take(text).into_bytes();
    bytes[at] ^= 0x20;
    *text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    let change = Change::part(format!("entry {row}"), before, move |p| text_at(p, row));
    state.record(doc, format!("Edit entry {row}"), change);
    Ok(())
}

/// `value` changed by the least the form's field changes it: an integer's low bit on pass 0, a
/// float by one step of an `f32` on pass 1, a flag on pass 2.
fn nudged(value: &Value, pass: u8) -> Option<Value> {
    Some(match (value, pass) {
        (Value::Int(i), 0) => Value::Int(i ^ 1),
        (Value::UInt(u), 0) => Value::UInt(u ^ 1),
        (Value::Color(c), 0) => Value::Color(c ^ 1),
        (Value::Float(f), 1) if f.is_finite() => Value::Float(f64::from((*f as f32).next_up())),
        (Value::Bool(b), 2) => Value::Bool(!b),
        _ => return None,
    })
}

fn scalar_in(
    inst: &MetaInstance,
    path: &mut Vec<MetaStep>,
    pass: u8,
    seen: &mut usize,
) -> Option<(Vec<MetaStep>, Value)> {
    for (at, (name, value)) in inst.fields.iter().enumerate() {
        let nth = inst.fields[..at].iter().filter(|(n, _)| n == name).count();
        path.push(match nth {
            0 => MetaStep::Field(name.clone()),
            nth => MetaStep::Nth(name.clone(), nth),
        });
        let found = scalar_at(value, path, pass, seen);
        path.pop();
        if found.is_some() {
            return found;
        }
    }
    None
}

fn scalar_at(
    value: &Value,
    path: &mut Vec<MetaStep>,
    pass: u8,
    seen: &mut usize,
) -> Option<(Vec<MetaStep>, Value)> {
    *seen += 1;
    if *seen > VISITS {
        return None;
    }
    if let Some(nudged) = nudged(value, pass) {
        return Some((path.clone(), nudged));
    }
    match value {
        Value::Struct(inner) | Value::Pointer(Some(inner)) => scalar_in(inner, path, pass, seen),
        Value::Array(items) => items.iter().enumerate().find_map(|(i, item)| {
            path.push(MetaStep::Index(i));
            let found = scalar_at(item, path, pass, seen);
            path.pop();
            found
        }),
        _ => None,
    }
}

/// The first integer of the form, else float, else flag, changed as the form's field changes it and
/// recorded as the form records it.
fn edit_meta(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
    let d = state.document_mut(doc).ok_or("its document is closed")?;
    // Every body opened from a GTA VI row is read only on disk, yet its form edits it, as the meta
    // editor lets it.
    let refused = (d.read_only).filter(|w| *w != crate::state::rpf9_scan::READ_ONLY_BODY);
    if let Some(why) = refused {
        return Err(format!("it opens read only: {why}"));
    }
    let DocPayload::Meta(meta) = &mut d.payload else {
        return Err("not a metadata document".into());
    };
    let parsed = meta.parsed.as_mut().ok_or("its form did not parse")?;
    let found = (0..3).find_map(|pass| scalar_in(&parsed.instance, &mut Vec::new(), pass, &mut 0));
    let (path, value) = found.ok_or("its form holds no number or flag")?;
    let held = value_at(&mut parsed.instance, &path).ok_or("the value is gone")?;
    let before = std::mem::replace(held, value);
    meta.after_visual_edit_in_place();
    let edit = MetaEdit::Set {
        path,
        other: before,
    };
    state.record(doc, edit.label(), Change::Meta(vec![edit]));
    Ok(())
}

/// The first constant of the first shader holding one, its first word's low bit changed.
fn edit_material(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
    let Some(DocPayload::Gta6Drawable(resource)) = state.document(doc).map(|d| &d.payload) else {
        return Err("not a drawable".into());
    };
    let drawables: Vec<_> = match &**resource {
        Resource::Drawable(d) => vec![(None, d)],
        Resource::Dictionary(_) => crate::state::gta6_resource::dictionary_drawables(resource)
            .map(|(code, d)| (Some(code), d))
            .collect(),
        _ => Vec::new(),
    };
    let found = drawables.iter().find_map(|(code, d)| {
        d.shaders.iter().enumerate().find_map(|(s, shader)| {
            shader
                .parameters
                .iter()
                .enumerate()
                .find_map(|(p, param)| match &param.value {
                    ParameterValue::Constant { words, .. } if !words.is_empty() => {
                        let mut words = words.clone();
                        words[0] ^= 1;
                        Some((*code, s, p, words))
                    }
                    _ => None,
                })
        })
    });
    let (code, shader, parameter, words) = found.ok_or("no shader holds a constant")?;
    state.edit_gta6_material(
        doc,
        code,
        shader,
        MaterialEdit::Constant { parameter, words },
    )
}

/// The smallest texture OpenRage encodes, replaced by its own pixels with one low bit changed.
fn edit_texture(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
    let Some(DocPayload::Resource(or_io::Asset::TextureDict(dict))) =
        state.document(doc).map(|d| &d.payload)
    else {
        return Err("not a texture dictionary".into());
    };
    let texture = (dict.entries.iter().flatten())
        .filter(|t| !t.backing_store.is_empty() && is_encodable(t.pixel_format()))
        .min_by_key(|t| t.backing_store.len())
        .ok_or("no texture reads in a format OpenRage encodes")?;
    let name = texture.name.clone();
    let mut image = decode_level(&**texture, 0).map_err(|e| e.to_string())?;
    if let Some(byte) = image.data.first_mut() {
        *byte ^= 1;
    }
    let held = crate::state::textures::TextureSource::Document;
    let options = TextureBuildOptions::default();
    state
        .import_texture_over(doc, held, &name, &image, FormatChoice::Automatic, &options)
        .map(drop)
}

/// A primitive child's material key, its low bit changed; else the radius one `f32` step larger.
fn edit_bound(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
    let Some(DocPayload::Gta6Drawable(resource)) = state.document(doc).map(|d| &d.payload) else {
        return Err("not a bound".into());
    };
    let Resource::Bound(b) = &**resource else {
        return Err("not a bound".into());
    };
    let keyed = b
        .children
        .iter()
        .enumerate()
        .find_map(|(i, c)| Some((i, c.material_id?)));
    let edit = match keyed {
        // A material key is the id's fifteen low bits.
        Some((child, id)) => BoundEdit::ChildKey {
            child,
            key: (id & 0x7FFF) as u16 ^ 1,
        },
        None => BoundEdit::Root {
            radius: b.radius.next_up(),
            box_min: b.box_min,
            box_max: b.box_max,
            centroid: b.centroid,
        },
    };
    state.edit_gta6_bound(doc, edit)
}

/// Polygon 0's flags with the lowest bit the flags view writes changed.
fn edit_cell(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
    let Some(DocPayload::Resource(or_io::Asset::NavMesh(mesh))) =
        state.document(doc).map(|d| &d.payload)
    else {
        return Err("not a navigation mesh".into());
    };
    let poly = (mesh.polys_array.as_deref())
        .and_then(|l| l.iter().next())
        .ok_or("the cell holds no polygon")?
        .clone();
    let flags = poly.as_integer1();
    let bit = (0..32).map(|b| 1u32 << b).find(|bit| {
        let mut p = poly.clone();
        p.set_as_integer1(flags ^ bit);
        p.as_integer1() != flags
    });
    let bit = bit.ok_or("no bit of the flags is written")?;
    state.set_gta6_polygon_flags(doc, 0, flags ^ bit)
}

/// The first key at frame 0 of the first animation holding a channel, raised by the 0.01 the key
/// editor drags by.
fn edit_key(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
    let Some(DocPayload::Gta6Anim(resource)) = state.document(doc).map(|d| &d.payload) else {
        return Err("not an animation resource".into());
    };
    let found = resource.sets.iter().enumerate().find_map(|(set, s)| {
        s.animations.iter().enumerate().find_map(|(animation, a)| {
            let (dof, component) = *a.key_components().first()?;
            let value = a.key_value(dof, component, 0)?;
            Some((AnimAt { set, animation }, dof, component, value))
        })
    });
    let (animation, dof, component, value) =
        found.ok_or("no animation holds a channel a key is written to")?;
    let edit = AnimEdit::Key {
        animation,
        dof,
        component,
        frame: 0,
        value: value + 0.01,
    };
    let made = state.edit_gta6_key(doc, edit);
    refused(state, made, "key")
}

/// The first frame filter's first weight one `f32` step larger.
fn edit_filter(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
    let Some(DocPayload::Gta6Drawable(resource)) = state.document(doc).map(|d| &d.payload) else {
        return Err("not a dictionary".into());
    };
    let Resource::Dictionary(dict) = &**resource else {
        return Err("not a dictionary".into());
    };
    let found = dict.entries.iter().find_map(|(code, e)| match e {
        Ok(Entry::FrameFilter { tracks, .. }) => tracks.first().map(|t| (*code, t.weight)),
        _ => None,
    });
    let (code, weight) = found.ok_or("no frame filter holds a track")?;
    state.edit_gta6_filter(
        doc,
        code,
        FilterEdit::Weight {
            track: 0,
            weight: weight.next_up(),
        },
    )
}

/// The first track's id of the first group holding one, the next id its group does not state.
fn edit_expression(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
    let Some(DocPayload::Gta6Expressions(set)) = state.document(doc).map(|d| &d.payload) else {
        return Err("not an expression set".into());
    };
    let found = ExpressionGroup::ALL.into_iter().find_map(|group| {
        let tracks = &group.of(set)?.tracks;
        let &(old, word) = tracks.first()?;
        let id = (1..=u32::MAX)
            .map(|d| old.wrapping_add(d))
            .find(|id| !tracks.contains(&(*id, word)))?;
        Some((group, id))
    });
    let (group, id) = found.ok_or("no group holds a track")?;
    let made = state.edit_gta6_expression_id(doc, group, 0, id);
    refused(state, made, "track id")
}

/// Entity 0 of the map shown stood one `f32` step further along x.
fn edit_map(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
    state.active_document = Some(doc);
    state.refresh_scanned_map();
    let e = (state.scanned_map().and_then(|s| s.data()))
        .and_then(|m| m.entities.first())
        .cloned()
        .ok_or("the map shows no entity")?;
    let [x, y, z] = e.position;
    let pose = Pose {
        position: Vec3::new(x.next_up(), y, z),
        rotation: Quat::from_array(e.model_rotation()),
        scale_xy: e.scale_xy,
        scale_z: e.scale_z,
    };
    state.place_scanned_entity(0, pose, "Moved entity 0".into())
}

/// An archetype added as the map screen's Add does, with no map shown.
fn edit_types(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
    state.active_document = Some(doc);
    state.refresh_scanned_map();
    state.toasts.clear();
    state.add_scanned_archetype();
    match last_error(state) {
        Some(why) => Err(why),
        None => Ok(()),
    }
}

fn first_effect(p: &mut DocPayload) -> Option<&mut PtxEffectRule<or_io::decl::Gen8>> {
    match p {
        DocPayload::Resource(or_io::Asset::Particles(list)) => {
            or_particle::list::effect_rules_mut(list).next()
        }
        _ => None,
    }
}

/// The first effect's longest duration raised by the 0.01 the effect inspector drags by, recorded
/// by the inspector's own step.
fn edit_particles(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
    let d = state.document_mut(doc).ok_or("its document is closed")?;
    let effect = first_effect(&mut d.payload).ok_or("the list holds no effect")?;
    let before = effect.clone();
    effect.duration_max += 0.01;
    crate::ui::panels::inspector::ypt_steps::effect(state, doc, 0, before);
    Ok(())
}

/// How one row's attempt ended.
enum Ended {
    /// The tree's save planned, and what it plans.
    Planned(String),
    /// A patched copy that does not read back as the edit.
    Differs(String),
    /// Refused before the save planned.
    Failed(String),
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
/// smallest of each label; a mixed label's spread over its sizes.
fn walk(state: &mut AppState, until: Instant) -> Vec<Candidate> {
    let mut by_label: HashMap<String, Vec<Candidate>> = HashMap::new();
    let Some(root) = session(state).map(RpfEditSession::root_id) else {
        return Vec::new();
    };
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
                    let row = (session(state).and_then(|s| s.archive(archive_id)))
                        .and_then(|a| a.archive.rpf9_row(entry_index));
                    let Some(row) = row else {
                        continue;
                    };
                    let size = if row.real > 0 { row.real } else { row.stored };
                    let label = row.kind.to_string();
                    let skipped = SKIPPED.split('|').any(|s| s == label)
                        || label.starts_with("type ")
                        || label.starts_with("class ");
                    if size > 0 && size <= ROW_CAP && !skipped {
                        by_label.entry(label.clone()).or_default().push(Candidate {
                            node: child,
                            label,
                            size,
                        });
                    }
                }
                Some(RpfNodeBacking::Archive { .. })
                    if depth < MAX_DEPTH && opened < NESTED_PER_ARCHIVE =>
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
        let spread = MIXED.contains(&label.as_str());
        let want = if spread { PER_MIXED_LABEL } else { PER_LABEL };
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

/// The body of the entry whose body starts at `at` in the archive `copy`, and whether its row is a
/// headerless resource's.
fn read_back(copy: &Path, at: u64, keys: &GtaViKeys) -> Result<(Vec<u8>, bool), String> {
    let mut file = File::open(copy).map_err(|e| e.to_string())?;
    let found = chains(&mut file, &BTreeSet::from([at]), keys)?
        .remove(&at)
        .ok_or("the copy holds no entry there")?;
    let (last, above) = found.chain.split_last().ok_or("the row has no entry")?;
    let mut mount = Gta6Mount::open_root(&mut file, keys).map_err(|e| e.to_string())?;
    for index in above {
        let next = {
            let e = (mount.archive.entries.iter().find(|e| e.index == *index))
                .ok_or("a nested archive is gone")?;
            mount
                .open_nested(&mut file, e, keys)
                .map_err(|e| e.to_string())?
        };
        mount = next;
    }
    let e = (mount.archive.entries.iter().find(|e| e.index == *last)).ok_or("the entry is gone")?;
    let body = mount
        .read_entry_decrypted(&mut file, e, keys)
        .map_err(|e| e.to_string())?;
    Ok((body, found.headerless))
}

/// Open `c` of the tree `tree` of the archive `disk` in an app of its own, and when it is a kind
/// not yet `settled`, edit it, save it into its row and plan the tree's save.
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
    let edit = editor(kind).filter(|_| !settled.contains(&kind))?;
    let failed = |why: String| Some((kind, Ended::Failed(why)));
    state.toasts.clear();
    if let Err(why) = edit(&mut state, doc) {
        return failed(format!("the edit is refused: {why}"));
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
    let ended = match planned.writes() {
        Writes::Rebuilt => Ended::Planned(format!("Rebuilt: {}", planned.kept.join("; "))),
        Writes::AsRead => {
            Ended::Failed("the save plans the archive as read: the body did not change".into())
        }
        Writes::Patched if std::fs::metadata(disk).map_or(u64::MAX, |m| m.len()) > COPY_CAP => {
            Ended::Planned("Patched (no copy written: the archive is over 1 GiB)".into())
        }
        Writes::Patched if Instant::now() + COPY_MARGIN > until => {
            Ended::Planned("Patched (no copy written: out of time)".into())
        }
        Writes::Patched => match copied(disk, keys, &planned, at, &body) {
            Ok(()) => Ended::Planned("Patched, and a copy reads the edit back".into()),
            Err(why) => Ended::Differs(format!("Patched, but {why}")),
        },
    };
    Some((kind, ended))
}

/// `planned` written to a copy under a temp dir, refused unless it is a patched copy whose entry at
/// `at` reads back as `body`, a headerless row's loose form read as its stored sections.
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
    let (read, headerless) = read_back(&out, at, keys)?;
    let (source, _) = read_back(disk, at, keys)?;
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
/// left and walked for at most 15 s through at most 64 nested archives three deep; no row is opened
/// once less than its two 30 s waits and 30 s for a plan and a copy are left of 200 s. A patched
/// copy is written under a temp dir only of an archive of at most 1 GiB. Install-gated: it
/// needs the key snapshot and the build, and skips without them.
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
    let mut said: HashMap<Kind, String> = HashMap::new();
    let mut settled: HashSet<Kind> = (KINDS.iter())
        .filter_map(|(k, edit)| edit.is_none().then_some(*k))
        .collect();
    let mut tries: HashMap<Kind, usize> = HashMap::new();
    let (mut panics, mut differs) = (Vec::new(), Vec::new());
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
            let tried = catch_unwind(AssertUnwindSafe(|| {
                attempt(&tree, &disk, c, &keys, &settled, began + BUDGET)
            }));
            let where_ = format!("{file}, {} bytes", c.size);
            let (kind, ended) = match tried {
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
            let (s, done) = match ended {
                Ended::Planned(s) => (s, true),
                Ended::Differs(s) => {
                    differs.push(format!("{kind}: {s} ({where_})"));
                    (s, true)
                }
                Ended::Failed(s) => {
                    let n = tries.entry(kind).or_default();
                    *n += 1;
                    (s, *n >= TRIES)
                }
            };
            said.insert(kind, format!("{s} ({where_})"));
            if done {
                settled.insert(kind);
            }
        }
    }
    let table: Vec<(Kind, String)> = (KINDS.iter())
        .map(|(kind, edit)| {
            let line = match edit {
                None => "no edit function".to_owned(),
                Some(_) => said
                    .remove(kind)
                    .unwrap_or_else(|| "no row found".to_owned()),
            };
            (*kind, line)
        })
        .collect();
    println!("{WHAT}, in {:.0} s:", began.elapsed().as_secs_f64());
    for (name, line) in &table {
        println!("  {name:<24} {line}");
    }
    for p in &panics {
        println!("  {p}");
    }
    assert!(panics.is_empty(), "a kind panicked: {panics:?}");
    assert!(
        differs.is_empty(),
        "a patched copy does not read back its edit: {differs:?}"
    );
    let listed: Vec<&str> = table.iter().map(|(name, _)| *name).collect();
    assert_eq!(
        listed,
        KINDS.map(|(name, _)| name),
        "the table lists every kind"
    );
}
