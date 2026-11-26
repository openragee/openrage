//! Editing a world file's derived fields and putting it back.
//!
//! Two derived things can be recomputed and written without moving anything:
//! the four extents and the two flag words. Both are scalars in place, so the
//! file's own type table and allocation are reused and the rest of the
//! document is untouched.
//!
//! ⚠ The refusal runs before the encode: a structural problem — an index past
//! the end of the array it subscripts, two parallel arrays of different
//! lengths — is a file the game misreads and OpenRage reads back identically,
//! so no round trip catches it.

use or_io::meta::binary::synth::{pso_layout_for_instance, rbm_layout_for_instance};
use or_io::meta::binary::{write_meta_binary, ContainerLayout, MetaLayout};
use or_io::meta::{MetaInstance, SchemaLibrary, Value};

use crate::archetypes::ArchetypeIndex;
use crate::content_flags::ContentFlags;
use crate::extents;
use crate::field;
use crate::keys;
use crate::map_data::MapData;
use crate::validate::{self, Problem};

/// Why a world file would not be written.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("{0} structural problem(s) the game would misread: {}", .1.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
    Structural(usize, Vec<Problem>),
    #[error("not a CMapData")]
    NotAMap,
    #[error("{0} entity archetype(s) unresolved, so the extents would be a lower bound")]
    IncompleteExtents(usize),
    #[error("the box the map streams by comes out holding nothing or not finite, as it does where an entity states a scale that is not above nought, so no extents are written")]
    NoStreamingExtents,
    #[error("`{0}` is not a CMapData or a CMapTypes")]
    NotAWorldFile(String),
    #[error("{0}")]
    NoSchema(String),
    #[error(transparent)]
    Meta(#[from] or_io::meta::MetaError),
}

/// Refuse a document the game would misread.
///
/// A tree that is not a `CMapData` passes: this says nothing about documents
/// it does not understand.
pub fn refuse_if_broken(root: &MetaInstance) -> Result<(), WriteError> {
    let Some(map) = MapData::new(root) else {
        return Ok(());
    };
    let problems = validate::check(&map);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(WriteError::Structural(problems.len(), problems))
    }
}

/// Encode a world document, refusing one the game would misread.
///
/// The layout is the one the document was read with, so an untouched
/// document re-encodes to the bytes it arrived as.
///
/// ⚠ A synthesised layout is the fallback only when the writer refuses with
/// [`or_io::meta::MetaError::OutgrewItsLayout`], never the default: it
/// reproduces none of the staged corpus's 3,527 PSO and 2,338 RBM documents
/// byte for byte, where the arrival layout reproduces all of them, so an
/// unconditional synthesis path would change every file.
pub fn encode(root: &MetaInstance, layout: &MetaLayout) -> Result<Vec<u8>, WriteError> {
    encode_with(or_io::meta::default_library(), root, layout)
}

/// [`encode`], synthesising any fallback layout from a named schema library.
pub fn encode_with(
    library: &SchemaLibrary,
    root: &MetaInstance,
    layout: &MetaLayout,
) -> Result<Vec<u8>, WriteError> {
    refuse_if_broken(root)?;
    match write_meta_binary(layout.format, Some(layout), root) {
        Err(e) if e.outgrew_its_layout() => {
            tracing::info!(
                reason = %e,
                "the document outgrew its own allocation; encoding against a synthesised layout"
            );
            Ok(write_meta_binary(
                layout.format,
                Some(&synthesise(library, root, layout)?),
                root,
            )?)
        }
        other => Ok(other?),
    }
}

/// A layout for a tree that no longer fits the one it arrived with.
///
/// ⚠ The donor supplies a class's `signature` word and nothing else: no tool
/// here or in CodeWalker derives one, so it is taken from the document rather
/// than left at zero. Every offset, ordering and padding byte comes from the
/// value tree, which keeps a second save byte-identical to the first.
fn synthesise(
    library: &SchemaLibrary,
    root: &MetaInstance,
    layout: &MetaLayout,
) -> Result<MetaLayout, WriteError> {
    let container = match &layout.container {
        ContainerLayout::Rbm(donor) => ContainerLayout::Rbm(Box::new(rbm_layout_for_instance(
            library,
            root,
            Some(&donor.model),
        )?)),
        ContainerLayout::Pso(_) => ContainerLayout::Pso(pso_layout_for_instance(library, root)?),
        // RBF needs no layout at all, so it has no allocation to outgrow.
        ContainerLayout::Rbf(rbf) => ContainerLayout::Rbf(rbf.clone()),
    };
    Ok(MetaLayout {
        format: layout.format,
        container,
    })
}

/// Add the flag bits a map's contents imply, leaving the rest alone.
///
/// Returns whether anything changed.
///
/// ⚠ **Additive, and clearing would be a defect.** `contentFlags` bit 9
/// marks a `_critical_` map, an authoring decision no derivation produces,
/// and a map group's members can carry a neighbour's claim. Over a mounted
/// retail Legacy install 1,476 of 11,076 maps set a `contentFlags` bit their
/// own contents do not imply — 1,338 of them bit 9 — so writing the derived
/// word would stop 1,338 maps loading with their area. A bit the contents
/// imply and the file does not set is content the streamer is never told
/// about.
pub fn apply_flags(root: &mut MetaInstance) -> Result<bool, WriteError> {
    let implied = {
        let map = MapData::new(root).ok_or(WriteError::NotAMap)?;
        ContentFlags::of(&map)
    };
    let stored_flags = field::u32_at(root, keys::FLAGS).unwrap_or(0);
    let stored_content = field::u32_at(root, keys::CONTENT_FLAGS).unwrap_or(0);
    let mut changed = set_u32(root, keys::FLAGS, stored_flags | implied.flags);
    changed |= set_u32(root, keys::CONTENT_FLAGS, stored_content | implied.content);
    Ok(changed)
}

/// Recompute a map's four extents and write them into the tree.
///
/// ⚠ Refuses when any entity's archetype is unresolved: extents computed
/// from a subset are smaller than the truth, and a map whose extents are too
/// small is one the streamer skips. A lower bound written into the file is
/// worse than the stale value it replaces.
pub fn apply_extents(root: &mut MetaInstance, index: &ArchetypeIndex) -> Result<bool, WriteError> {
    let computed = {
        let map = MapData::new(root).ok_or(WriteError::NotAMap)?;
        extents::compute(&map, index)
    };
    if !computed.is_complete() {
        return Err(WriteError::IncompleteExtents(computed.unresolved));
    }
    if computed.entities.is_empty() {
        return Ok(false);
    }
    // ⚠ A scale of nought or below leaves an entity out of the streaming box with no word, and a box that holds nothing streams nothing.
    let streaming = computed.streaming;
    if streaming.is_empty() || !(streaming.min.is_finite() && streaming.max.is_finite()) {
        return Err(WriteError::NoStreamingExtents);
    }

    let mut changed = set_vec3(root, keys::ENTITIES_EXTENTS_MIN, computed.entities.min);
    changed |= set_vec3(root, keys::ENTITIES_EXTENTS_MAX, computed.entities.max);
    changed |= set_vec3(root, keys::STREAMING_EXTENTS_MIN, computed.streaming.min);
    changed |= set_vec3(root, keys::STREAMING_EXTENTS_MAX, computed.streaming.max);
    Ok(changed)
}

/// ⚠ Writes through the field's existing slot and never adds one: a name the
/// type table does not declare makes a document the binary writer refuses,
/// so a missing field returns `false`.
fn set_u32(root: &mut MetaInstance, key: u32, value: u32) -> bool {
    match field::get_mut(root, key) {
        Some(slot @ Value::UInt(_)) => {
            let new = Value::UInt(value as u64);
            let changed = *slot != new;
            *slot = new;
            changed
        }
        Some(slot @ Value::Int(_)) => {
            let new = Value::Int(value as i64);
            let changed = *slot != new;
            *slot = new;
            changed
        }
        _ => false,
    }
}

fn set_vec3(root: &mut MetaInstance, key: u32, value: glam::Vec3) -> bool {
    match field::get_mut(root, key) {
        Some(slot @ Value::Vec3(_)) => {
            let new = Value::Vec3(value.into());
            let changed = *slot != new;
            *slot = new;
            changed
        }
        Some(slot @ Value::Vec4(_)) => {
            let old = match slot {
                Value::Vec4(a) => *a,
                _ => unreachable!(),
            };
            let new = Value::Vec4([value.x, value.y, value.z, old[3]]);
            let changed = *slot != new;
            *slot = new;
            changed
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_flags::content;
    use or_io::meta::instance::value::HashedString;

    fn map() -> MetaInstance {
        let mut ent = MetaInstance::new("CEntityDef");
        ent.set("archetypeName", Value::Hash(HashedString::new("prop_x")));
        ent.set("position", Value::Vec3([0.0, 0.0, 0.0]));
        ent.set("rotation", Value::Vec4([0.0, 0.0, 0.0, 1.0]));
        ent.set("scaleXY", Value::Float(1.0));
        ent.set("scaleZ", Value::Float(1.0));
        ent.set("parentIndex", Value::Int(-1));
        ent.set(
            "lodLevel",
            Value::Enum {
                type_name: "rage__eLodType".into(),
                variant: "LODTYPES_DEPTH_HD".into(),
            },
        );

        let mut root = MetaInstance::new("CMapData");
        root.set("flags", Value::UInt(0));
        root.set("contentFlags", Value::UInt(0));
        root.set("entitiesExtentsMin", Value::Vec3([0.0; 3]));
        root.set("entitiesExtentsMax", Value::Vec3([0.0; 3]));
        root.set("streamingExtentsMin", Value::Vec3([0.0; 3]));
        root.set("streamingExtentsMax", Value::Vec3([0.0; 3]));
        root.set(
            "entities",
            Value::Array(vec![Value::Pointer(Some(Box::new(ent)))]),
        );
        root
    }

    fn index() -> ArchetypeIndex {
        let mut arch = MetaInstance::new("CBaseArchetypeDef");
        arch.set("name", Value::Hash(HashedString::new("prop_x")));
        arch.set("bbMin", Value::Vec3([-1.0; 3]));
        arch.set("bbMax", Value::Vec3([1.0; 3]));
        arch.set("lodDist", Value::Float(0.0));
        let mut types = MetaInstance::new("CMapTypes");
        types.set(
            "archetypes",
            Value::Array(vec![Value::Pointer(Some(Box::new(arch)))]),
        );
        let mut index = ArchetypeIndex::new();
        index.add(crate::map_types::MapTypes::new(&types).unwrap());
        index
    }

    #[test]
    fn recomputing_flags_writes_what_the_contents_imply() {
        let mut root = map();
        assert!(apply_flags(&mut root).unwrap());
        assert_eq!(field::u32_at(&root, keys::CONTENT_FLAGS), Some(content::HD));
        assert!(!apply_flags(&mut root).unwrap());
    }

    #[test]
    fn recomputing_flags_keeps_an_authored_bit() {
        let mut root = map();
        set_u32(&mut root, keys::CONTENT_FLAGS, content::CRITICAL);

        // The derivation alone does not produce it, so this test can fail.
        let implied = ContentFlags::of(&MapData::new(&root).unwrap());
        assert_eq!(implied.content & content::CRITICAL, 0);

        assert!(apply_flags(&mut root).unwrap());
        let after = field::u32_at(&root, keys::CONTENT_FLAGS).unwrap();
        assert_eq!(after & content::CRITICAL, content::CRITICAL, "preserved");
        assert_eq!(after & content::HD, content::HD, "and HD was added");
    }

    #[test]
    fn recomputing_extents_writes_the_placed_box() {
        let mut root = map();
        assert!(apply_extents(&mut root, &index()).unwrap());
        assert_eq!(
            field::vec3_at(&root, keys::ENTITIES_EXTENTS_MIN),
            Some([-1.0; 3])
        );
        assert!(!apply_extents(&mut root, &index()).unwrap());
    }

    #[test]
    fn extents_refuse_when_an_archetype_is_unresolved() {
        let mut root = map();
        let err = apply_extents(&mut root, &ArchetypeIndex::new()).unwrap_err();
        assert!(matches!(err, WriteError::IncompleteExtents(1)));
        assert_eq!(
            field::vec3_at(&root, keys::ENTITIES_EXTENTS_MIN),
            Some([0.0; 3])
        );
    }

    #[test]
    fn writing_a_field_the_document_does_not_have_changes_nothing() {
        let mut root = MetaInstance::new("CMapData");
        assert!(!set_u32(&mut root, keys::CONTENT_FLAGS, 7));
        assert!(root.fields.is_empty());
    }

    #[test]
    fn a_broken_hierarchy_is_refused_before_anything_is_encoded() {
        let mut root = map();
        let entities = field::get_mut(&mut root, keys::ENTITIES).unwrap();
        if let Value::Array(items) = entities {
            if let Some(Value::Pointer(Some(ent))) = items.first_mut() {
                ent.set("parentIndex", Value::Int(9));
            }
        }
        let err = refuse_if_broken(&root).unwrap_err();
        assert!(matches!(err, WriteError::Structural(1, _)), "{err}");
        assert!(err.to_string().contains("entity 0"), "{err}");
    }

    #[test]
    fn a_tree_that_is_not_a_map_passes_the_refusal() {
        assert!(refuse_if_broken(&MetaInstance::new("CPedVariationInfo")).is_ok());
    }
}
