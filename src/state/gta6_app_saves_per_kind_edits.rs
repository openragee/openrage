//! The smallest edit each kind's own editor makes, as the per-kind table makes it on a row of the
//! build.

use glam::{Quat, Vec3};
use or_io::authoring::{is_encodable, FormatChoice, TextureBuildOptions};
use or_io::codec::texture::{decode_level, TextureSource as _};
use or_io::gta5::particle::PtxEffectRule;
use or_io::gta6::anim::{AnimAt, AnimEdit, ExpressionGroup};
use or_io::gta6::headerless::{Entry, ParameterValue, Resource};
use or_io::meta::{MetaInstance, Value};
use or_world::transform::Pose;

use crate::state::document::DocumentId;
use crate::state::gta6_edit::MaterialEdit;
use crate::state::gta6_filter_edit::FilterEdit;
use crate::state::gta6_physics_edit::BoundEdit;
use crate::state::meta_edit::value_at;
use crate::state::presets::MetaStep;
use crate::state::{AppState, Change, DocPayload, MetaEdit, ToastType};

/// What the map edit says of a row with no entity, which is no row of the kind to judge it by.
pub(super) const NO_ENTITY: &str = "the map shows no entity";

pub(super) fn last_error(state: &AppState) -> Option<String> {
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
pub(super) fn edit_text(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
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
    if *seen > super::VISITS {
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
pub(super) fn edit_meta(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
    edit_meta_by(state, doc, [0, 1, 2])
}

/// A member value of a map types body changed through its metadata form, a float first: an integer
/// of one is often a string's hash, which its writer keeps to the strings the body holds.
pub(super) fn edit_types_member(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
    edit_meta_by(state, doc, [1, 0, 2])
}

/// The first scalar of the form `passes` takes in turn, as [`nudged`] numbers them, changed.
fn edit_meta_by(state: &mut AppState, doc: DocumentId, passes: [u8; 3]) -> Result<(), String> {
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
    let found = (passes.into_iter())
        .find_map(|pass| scalar_in(&parsed.instance, &mut Vec::new(), pass, &mut 0));
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
pub(super) fn edit_material(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
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
pub(super) fn edit_texture(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
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
pub(super) fn edit_bound(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
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
pub(super) fn edit_cell(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
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
pub(super) fn edit_key(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
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
pub(super) fn edit_filter(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
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
pub(super) fn edit_expression(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
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
pub(super) fn edit_map(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
    state.active_document = Some(doc);
    state.refresh_scanned_map();
    let e = (state.scanned_map().and_then(|s| s.data()))
        .and_then(|m| m.entities.first())
        .cloned()
        .ok_or(NO_ENTITY)?;
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
pub(super) fn edit_types_added(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
    state.active_document = Some(doc);
    state.refresh_scanned_map();
    state.toasts.clear();
    state.add_scanned_archetype();
    match last_error(state) {
        Some(why) => Err(why),
        None => Ok(()),
    }
}

/// The first cell not at the grid's commonest height, its low bit changed through the inspector's
/// field: a one-array grid's spans cover such a cell, so the body keeps its length.
pub(super) fn edit_height(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
    let Some(DocPayload::HeightMap(map)) = state.document(doc).map(|d| &d.payload) else {
        return Err("not a height map".into());
    };
    let mut seen = [0usize; 256];
    map.max.iter().for_each(|h| seen[usize::from(*h)] += 1);
    let commonest = (0..=u8::MAX).max_by_key(|h| seen[usize::from(*h)]);
    let at = (map.max.iter())
        .position(|h| Some(*h) != commonest)
        .ok_or("every cell holds one height")?;
    let cols = usize::from(map.num_cols.max(1));
    // Both are below the grid's u16 column and row counts.
    let (col, row, height) = ((at % cols) as u16, (at / cols) as u16, map.max[at] ^ 1);
    state.set_height_map_cell(doc, col, row, height)
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
pub(super) fn edit_particles(state: &mut AppState, doc: DocumentId) -> Result<(), String> {
    let d = state.document_mut(doc).ok_or("its document is closed")?;
    let effect = first_effect(&mut d.payload).ok_or("the list holds no effect")?;
    let before = effect.clone();
    effect.duration_max += 0.01;
    crate::ui::panels::inspector::ypt_steps::effect(state, doc, 0, before);
    Ok(())
}
