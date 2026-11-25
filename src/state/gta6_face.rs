//! A GTA VI head's face skeleton, found by the head's name among the face-skeleton dictionaries
//! opened from a scan, and the expression sets opened from one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use or_io::container::rpf::{Rpf9Role, Rpf9Signature};
use or_io::gta6::anim::{
    read_expressions, track_label, ExpressionGroup, Expressions, EXPRESSIONS_SIGNATURE,
};
use or_io::gta6::hash::name_hash32;
use or_io::gta6::headerless::{
    read_dictionary_skeletons, Drawable, Entry, Palette, Resource, Skeleton,
};

use super::{AppState, DocPayload, DocumentId};

/// The first bone of every face skeleton.
const FACIAL_ROOT: u32 = name_hash32(b"FACIAL_C_FacialRoot");
/// How far a face skeleton's bind rotation may be from unit length.
const UNIT_SLACK: f32 = 2e-3;

/// The face skeletons of each dictionary opened from a scan, by entry code.
// ponytail: one map for the whole app, which no close empties; a face skeleton is a few kilobytes.
static FACES: Mutex<BTreeMap<PathBuf, Vec<(u32, Skeleton)>>> = Mutex::new(BTreeMap::new());
/// Each expression set opened from a scan, by the path it opened under.
static EXPRESSIONS: Mutex<BTreeMap<PathBuf, Expressions>> = Mutex::new(BTreeMap::new());

fn lock<T>(map: &'static Mutex<T>) -> MutexGuard<'static, T> {
    map.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What a headerless body opened from a scan adds to what the Animation workspace plays with.
pub(super) enum Learned {
    /// A dictionary's lone face skeleton, which picking the dictionary as a model poses.
    Skeleton(Skeleton),
    /// An expression set, which opens as a document of its own.
    Expressions(Box<Expressions>),
    /// The lines a body the expression reader refuses opens as.
    Facts(String),
}

/// Keep the face skeletons or the expression set of the body of `signature` opened at `path`.
pub(super) fn learn(
    path: &Path,
    signature: Rpf9Signature,
    virtual_section: &[u8],
) -> Option<Learned> {
    if signature.to_string() == EXPRESSIONS_SIGNATURE {
        let learned = match read_expressions(virtual_section) {
            Ok(set) => {
                lock(&EXPRESSIONS).insert(path.to_path_buf(), set.clone());
                Learned::Expressions(Box::new(set))
            }
            Err(e) => {
                let text = tr!("The expression reader does not read this body: {e}", e = e);
                Learned::Facts(text)
            }
        };
        return Some(learned);
    }
    if signature.role() != Some(Rpf9Role::Dictionary) {
        return None;
    }
    let faces: Vec<(u32, Skeleton)> = (read_dictionary_skeletons(virtual_section).ok()?)
        .into_iter()
        .filter(|(_, s)| is_face(s))
        .collect();
    let lone = match faces.as_slice() {
        [(_, skeleton)] => Some(Learned::Skeleton(skeleton.clone())),
        _ => None,
    };
    if !faces.is_empty() {
        lock(&FACES).insert(path.to_path_buf(), faces);
    }
    lone
}

/// Whether bone 0 of `skeleton` is the face root and every bind rotation is a unit quaternion.
pub(crate) fn is_face(skeleton: &Skeleton) -> bool {
    let unit = |q: &[f32; 4]| (q.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() <= UNIT_SLACK;
    skeleton
        .bones
        .first()
        .is_some_and(|b| b.hash == FACIAL_ROOT)
        && skeleton.bones.iter().all(|b| unit(&b.rotation))
}

/// An expression set's key and its tracks, as its body opens.
pub(crate) fn facts(set: &Expressions) -> String {
    let mut lines = vec![tr!(
        "A GTA VI expression set of key {key}.",
        key = super::gta6_names::said32(set.key)
    )];
    lines.push(tr_n!(
        "It reads {count} track: the facial controls, or bones, a clip carries.",
        "It reads {count} tracks: the facial controls, or bones, a clip carries.",
        set.inputs.tracks.len()
    ));
    lines.push(match &set.outputs {
        Some(outputs) => tr_n!(
            "It drives {count} track.",
            "It drives {count} tracks.",
            outputs.tracks.len()
        ),
        None => tr!("It drives no track.").to_string(),
    });
    lines.push(tr!(
        "Bone tracks it reads: {bones}; tracks of a word whose name is not known it reads: {others}; facial controls it writes: {facial}.",
        bones = set.bones.tracks.len(),
        others = set.controls.tracks.len(),
        facial = set.facial_outputs.tracks.len()
    ));
    lines.push(tr!("It holds no program, so how what it reads becomes what it drives is not known. Pick it under Expression set where a clip plays in the Animation workspace to see the controls of the clip it reads.").to_string());
    lines.extend(tables(set));
    lines.join("\n")
}

/// The tracks a set reads and those it writes, each group under its name, a line a track.
fn tables(set: &Expressions) -> Vec<String> {
    let mut lines = Vec::new();
    for reads in [true, false] {
        let slots: usize = (ExpressionGroup::ALL.iter())
            .filter(|g| g.reads() == reads)
            .filter_map(|g| g.of(set))
            .map(|g| g.slots.len())
            .sum();
        let slots = slots + if reads { set.control_slots.len() } else { 0 };
        lines.push(String::new());
        lines.push(match reads {
            true => tr_n!(
                "What it reads, in {count} slot",
                "What it reads, in {count} slots",
                slots
            ),
            false => tr_n!(
                "What it writes, in {count} slot",
                "What it writes, in {count} slots",
                slots
            ),
        });
        for group in ExpressionGroup::ALL
            .into_iter()
            .filter(|g| g.reads() == reads)
        {
            let Some(tracks) = group.of(set).filter(|t| !t.tracks.is_empty()) else {
                continue;
            };
            lines.push(format!("{} ({})", group_label(group), tracks.tracks.len()));
            for (k, (id, word)) in tracks.tracks.iter().enumerate() {
                let detail = track_detail(set, group, k);
                let id = super::gta6_names::said32(*id);
                lines.push(format!("  {id} {}: {detail}", track_label(*word)));
            }
        }
        if reads && !set.control_slots.is_empty() {
            let slots: Vec<String> = set.control_slots.iter().map(u16::to_string).collect();
            let words: Vec<String> = (set.control_words.iter())
                .map(|w| format!("{w:#010X}"))
                .collect();
            lines.push(tr!(
                "Further input slots of the unnamed word: {slots}; their words: {words}",
                slots = slots.join(", "),
                words = words.join(", ")
            ));
        }
    }
    lines
}

/// What a group of an expression set's tracks is called.
pub(crate) fn group_label(group: ExpressionGroup) -> &'static str {
    match group {
        ExpressionGroup::Inputs => tr!("Facial controls it reads"),
        ExpressionGroup::Bones => tr!("Bones it reads"),
        ExpressionGroup::Controls => tr!("Tracks of an unnamed word it reads"),
        ExpressionGroup::FacialOutputs => tr!("Facial controls it writes"),
        ExpressionGroup::Outputs => tr!("Bones it drives"),
    }
}

/// What track `track` of `group` states beside its id and word: components, a byte or a mask, and
/// its slots.
pub(crate) fn track_detail(set: &Expressions, group: ExpressionGroup, track: usize) -> String {
    let slots: Vec<String> = (group.slots(set, track).unwrap_or_default().iter())
        .map(u16::to_string)
        .collect();
    let slots = slots.join(", ");
    let component = (group.of(set))
        .and_then(|g| g.components.get(track))
        .copied()
        .unwrap_or_default();
    match group {
        ExpressionGroup::Inputs | ExpressionGroup::FacialOutputs => tr!(
            "{components} components in slots {slots}",
            components = component,
            slots = slots
        ),
        ExpressionGroup::Controls => {
            tr!("byte {byte}, slot {slots}", byte = component, slots = slots)
        }
        ExpressionGroup::Bones | ExpressionGroup::Outputs => tr!(
            "mask {mask}, slots {slots}",
            mask = format!("{:#04X}", group.mask(set, track).unwrap_or_default()),
            slots = slots
        ),
    }
}

/// The expression set opened at `path`, if one did.
pub(crate) fn expression_set(path: &Path) -> Option<Expressions> {
    lock(&EXPRESSIONS).get(path).cloned()
}

/// Whether an expression set opened at `path`.
pub(crate) fn is_expression_set(path: &Path) -> bool {
    lock(&EXPRESSIONS).contains_key(path)
}

/// The face skeleton the head named `head` is posed by, else the entry names looked for.
// `find` gives the face skeleton of an open dictionary's entry by its code.
pub(crate) fn face_skeleton(
    head: &str,
    own: Option<&Skeleton>,
    find: impl Fn(u32) -> Option<Skeleton>,
) -> Result<Skeleton, Vec<String>> {
    let name = head.to_ascii_lowercase();
    let base = name.strip_suffix(".#dr").unwrap_or(&name);
    let stem = base.strip_suffix("_full").unwrap_or(base);
    let eyes_full = format!("{stem}_eyes_full");
    if let Some(found) = find(name_hash32(eyes_full.as_bytes())) {
        return Ok(found);
    }
    if let Some(own) = own.filter(|s| is_face(s)) {
        return Ok(own.clone());
    }
    let mut looked_for = vec![eyes_full];
    // A head with no `_eyes_full` entry may name its face skeleton `_eyes` alone.
    let eyes = format!("{base}_eyes");
    if let Some(found) = find(name_hash32(eyes.as_bytes())) {
        return Ok(found);
    }
    looked_for.push(eyes);
    Err(looked_for)
}

/// The bone name hashes the palettes of `drawable`'s models name, rising, each once.
pub(crate) fn palette_bones(drawable: &Drawable) -> Vec<u32> {
    let mut bones: Vec<u32> = (drawable.levels.iter().flatten().flatten())
        .filter_map(|model| match &model.palette {
            Ok(Palette::Hashes(hashes)) => Some(hashes.iter().copied()),
            _ => None,
        })
        .flatten()
        .collect();
    bones.sort_unstable();
    bones.dedup();
    bones
}

/// The face skeleton of an open dictionary's entry of code `code`, read whole.
fn open_face(state: &AppState, code: u32) -> Option<Skeleton> {
    let faces = lock(&FACES);
    state.documents.iter().find_map(|d| {
        let DocPayload::Gta6Drawable(resource) = &d.payload else {
            return None;
        };
        let Resource::Dictionary(dict) = &**resource else {
            return None;
        };
        // An entry read whole is one whose code is the hash of its name.
        let read =
            (dict.entries.iter()).any(|(c, e)| *c == code && matches!(e, Ok(Entry::Drawable(_))));
        if !read {
            return None;
        }
        let held = faces.get(d.path.as_deref()?)?;
        held.iter()
            .find(|(c, _)| *c == code)
            .map(|(_, s)| s.clone())
    })
}

/// The skeleton the scanned body `doc` at `path` is posed by: its named face skeleton, else its own.
// A drawable with neither says which bones its palette names that no skeleton read holds.
pub(crate) fn model_skeleton(
    state: &AppState,
    doc: DocumentId,
    path: &Path,
) -> Result<Skeleton, String> {
    let own = super::rpf9_scan::scanned_skeleton(path);
    let drawable = match state.document(doc).map(|d| &d.payload) {
        Some(DocPayload::Gta6Drawable(resource)) => match &**resource {
            Resource::Drawable(d) => d,
            Resource::DrawableWithBound(r) => &r.drawable,
            // A dictionary's drawable is posed by the skeleton the entry selected holds.
            Resource::Dictionary(_) => {
                let selected = state.selection.selected(doc);
                let entry = (state.document(doc))
                    .and_then(|d| super::gta6_entry_skeletons::shown(d, selected));
                return entry.ok_or_else(|| {
                    tr!("Select a drawable of this dictionary that holds a skeleton, then pick the model again.")
                        .to_string()
                });
            }
            _ => return own,
        },
        _ => return own,
    };
    let found = face_skeleton(&drawable.name, own.as_ref().ok(), |code| {
        open_face(state, code)
    });
    let looked_for = match found {
        Ok(face) => return Ok(face),
        Err(looked_for) => looked_for,
    };
    own.map_err(|why| {
        let bones = palette_bones(drawable);
        if bones.is_empty() {
            return why;
        }
        let bones: Vec<String> = (bones.iter())
            .map(|h| super::gta6_names::said32(*h))
            .collect();
        tr!(
            "No open dictionary holds {entries}, the face skeleton this head's name names, and the head has no skeleton of its own, so no skeleton holds the bones its palette names: {bones}. Open that dictionary from a scan of its archive, then pick the head again.",
            entries = looked_for.join(", "),
            bones = bones.join(", ")
        )
    })
}

#[cfg(test)]
#[path = "gta6_face_tests.rs"]
mod tests;
