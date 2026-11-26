//! `CMapData`: what a `.ymap` places, as a view over the value tree.
//!
//! ⚠ Nothing here copies the tree: a model holding its own copy of every field
//! drops, when written back, the fields it forgot — the mistake `or-particle`
//! cost ~329,000 mismatches to learn.

use glam::{Quat, Vec3};
use or_io::meta::{MetaInstance, Value};

use crate::extension::Extension;
use crate::field;
use crate::keys;
use crate::streaming::Level;
use crate::transform::{self, Placement};

/// A `.ymap`'s root, borrowed.
#[derive(Debug, Clone, Copy)]
pub struct MapData<'a> {
    root: &'a MetaInstance,
}

impl<'a> MapData<'a> {
    /// A view over `root`, if it is a `CMapData`.
    pub fn new(root: &'a MetaInstance) -> Option<Self> {
        field::is_type(root, keys::types::C_MAP_DATA).then_some(Self { root })
    }

    pub fn instance(&self) -> &'a MetaInstance {
        self.root
    }

    pub fn name(&self) -> u32 {
        field::hash_at(self.root, keys::NAME).unwrap_or(0)
    }

    pub fn parent(&self) -> u32 {
        field::hash_at(self.root, keys::PARENT).unwrap_or(0)
    }

    pub fn flags(&self) -> u32 {
        field::u32_at(self.root, keys::FLAGS).unwrap_or(0)
    }

    pub fn content_flags(&self) -> u32 {
        field::u32_at(self.root, keys::CONTENT_FLAGS).unwrap_or(0)
    }

    pub fn stored_entities_extents(&self) -> Option<(Vec3, Vec3)> {
        Some((
            field::vec3_at(self.root, keys::ENTITIES_EXTENTS_MIN)?.into(),
            field::vec3_at(self.root, keys::ENTITIES_EXTENTS_MAX)?.into(),
        ))
    }

    pub fn stored_streaming_extents(&self) -> Option<(Vec3, Vec3)> {
        Some((
            field::vec3_at(self.root, keys::STREAMING_EXTENTS_MIN)?.into(),
            field::vec3_at(self.root, keys::STREAMING_EXTENTS_MAX)?.into(),
        ))
    }

    /// Every placement, plain entities and interior instances alike.
    ///
    /// ⚠ The two are in **one** array and are told apart by their own type,
    /// not by position or by a flag. They also read their rotation
    /// differently — see [`Placement`].
    pub fn entities(&self) -> impl Iterator<Item = Entity<'a>> + 'a {
        field::structs_at(self.root, keys::ENTITIES).map(Entity::new)
    }

    pub fn entity_count(&self) -> usize {
        field::array_at(self.root, keys::ENTITIES).len()
    }

    /// Whether every entity read, so a place in [`Self::entities`] is the place a `parentIndex` names.
    pub fn entities_are_whole(&self) -> bool {
        field::structs_fill(self.root, keys::ENTITIES)
    }

    pub fn car_generator_count(&self) -> usize {
        field::array_at(self.root, keys::CAR_GENERATORS).len()
    }

    pub fn time_cycle_modifier_count(&self) -> usize {
        field::array_at(self.root, keys::TIME_CYCLE_MODIFIERS).len()
    }

    pub fn box_occluder_count(&self) -> usize {
        field::array_at(self.root, keys::BOX_OCCLUDERS).len()
    }

    pub fn occlude_model_count(&self) -> usize {
        field::array_at(self.root, keys::OCCLUDE_MODELS).len()
    }

    pub fn physics_dictionary_count(&self) -> usize {
        field::array_at(self.root, keys::PHYSICS_DICTIONARIES).len()
    }

    pub fn container_lod_count(&self) -> usize {
        field::array_at(self.root, keys::CONTAINER_LODS).len()
    }

    /// The grass batches, each with its own precomputed bounds.
    pub fn grass_batches(&self) -> impl Iterator<Item = GrassBatch<'a>> + 'a {
        self.root
            .fields
            .iter()
            .find(|(n, _)| field::name_hash(n) == keys::INSTANCED_DATA)
            .and_then(|(_, v)| field::as_struct(v))
            .into_iter()
            .flat_map(|inst| field::structs_at(inst, keys::GRASS_INSTANCE_LIST))
            .map(|inst| GrassBatch { inst })
    }

    pub fn prop_batch_count(&self) -> usize {
        field::struct_at(self.root, keys::INSTANCED_DATA)
            .map(|inst| field::array_at(inst, keys::PROP_INSTANCE_LIST).len())
            .unwrap_or(0)
    }

    /// Whether the map carries the light arrays a LOD-light `.ymap` holds.
    ///
    /// ⚠ The structs are present and empty on almost every map, so their
    /// presence answers nothing; the arrays inside them are the signal.
    pub fn has_lod_lights(&self) -> bool {
        field::struct_at(self.root, keys::LOD_LIGHTS_SOA)
            .map(|inst| inst.fields.iter().any(|(_, v)| array_len(v) > 0))
            .unwrap_or(false)
    }

    pub fn has_distant_lod_lights(&self) -> bool {
        field::struct_at(self.root, keys::DISTANT_LOD_LIGHTS_SOA)
            .map(|inst| inst.fields.iter().any(|(_, v)| array_len(v) > 0))
            .unwrap_or(false)
    }

    /// The box occluders, as centre and half-extent in world units.
    ///
    /// ⚠ Every field is a **signed 16-bit count of quarter-metres**, which is
    /// why they are integers in the file and why a box further than about
    /// 8 km from the origin cannot be expressed.
    pub fn box_occluders(&self) -> impl Iterator<Item = BoxOccluder> + 'a {
        field::structs_at(self.root, keys::BOX_OCCLUDERS).map(|inst| BoxOccluder {
            centre: Vec3::new(
                field::i32_at(inst, keys::I_CENTER_X).unwrap_or(0) as f32 * 0.25,
                field::i32_at(inst, keys::I_CENTER_Y).unwrap_or(0) as f32 * 0.25,
                field::i32_at(inst, keys::I_CENTER_Z).unwrap_or(0) as f32 * 0.25,
            ),
            size: Vec3::new(
                field::i32_at(inst, keys::I_LENGTH).unwrap_or(0) as f32 * 0.25,
                field::i32_at(inst, keys::I_WIDTH).unwrap_or(0) as f32 * 0.25,
                field::i32_at(inst, keys::I_HEIGHT).unwrap_or(0) as f32 * 0.25,
            ),
        })
    }

    /// The occlusion meshes' stored bounds.
    pub fn occlude_model_bounds(&self) -> impl Iterator<Item = (Vec3, Vec3)> + 'a {
        field::structs_at(self.root, keys::OCCLUDE_MODELS).filter_map(|inst| {
            Some((
                field::vec3_at(inst, keys::BMIN)?.into(),
                field::vec3_at(inst, keys::BMAX)?.into(),
            ))
        })
    }

    /// Car generator positions, with the length that sizes the slot.
    pub fn car_generators(&self) -> impl Iterator<Item = (Vec3, f32)> + 'a {
        field::structs_at(self.root, keys::CAR_GENERATORS).filter_map(|inst| {
            Some((
                field::vec3_at(inst, keys::POSITION)?.into(),
                field::f32_at(inst, keys::PERPENDICULAR_LENGTH).unwrap_or(0.0),
            ))
        })
    }
}

fn array_len(value: &Value) -> usize {
    match value {
        Value::Array(items) => items.len(),
        Value::ByteArray(bytes) => bytes.len(),
        _ => 0,
    }
}

/// A box occluder in world units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxOccluder {
    pub centre: Vec3,
    pub size: Vec3,
}

/// One grass batch, which carries its own bounds rather than deriving them.
#[derive(Debug, Clone, Copy)]
pub struct GrassBatch<'a> {
    inst: &'a MetaInstance,
}

impl GrassBatch<'_> {
    pub fn archetype(&self) -> u32 {
        field::hash_at(self.inst, keys::ARCHETYPE_NAME).unwrap_or(0)
    }

    pub fn lod_dist(&self) -> f32 {
        field::f32_at(self.inst, keys::LOD_DIST).unwrap_or(0.0)
    }

    pub fn instance_count(&self) -> usize {
        field::array_at(self.inst, keys::INSTANCE_LIST).len()
    }

    /// The batch's own axis-aligned bounds.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let aabb = field::struct_at(self.inst, keys::BATCH_AABB)?;
        let min = field::vec4_at(aabb, keys::MIN)?;
        let max = field::vec4_at(aabb, keys::MAX)?;
        Some((
            Vec3::new(min[0], min[1], min[2]),
            Vec3::new(max[0], max[1], max[2]),
        ))
    }
}

/// The variants of `lodLevel`, each with the level it names.
const LOD_LEVELS: [(&str, Level); Level::COUNT] = [
    ("LODTYPES_DEPTH_HD", Level::Hd),
    ("LODTYPES_DEPTH_LOD", Level::Lod),
    ("LODTYPES_DEPTH_SLOD1", Level::Slod1),
    ("LODTYPES_DEPTH_SLOD2", Level::Slod2),
    ("LODTYPES_DEPTH_SLOD3", Level::Slod3),
    ("LODTYPES_DEPTH_ORPHANHD", Level::OrphanHd),
    ("LODTYPES_DEPTH_SLOD4", Level::Slod4),
];

/// One placement in a `.ymap`.
#[derive(Debug, Clone, Copy)]
pub struct Entity<'a> {
    inst: &'a MetaInstance,
    placement: Placement,
}

impl<'a> Entity<'a> {
    pub fn new(inst: &'a MetaInstance) -> Self {
        let placement = if field::is_type(inst, keys::types::C_MLO_INSTANCE_DEF) {
            Placement::InteriorInstance
        } else {
            Placement::Entity
        };
        Self { inst, placement }
    }

    /// Read an entity that lives inside an interior archetype.
    ///
    /// ⚠ These are `CEntityDef`s and take the conjugating convention, even
    /// though the interior placing them does not.
    pub fn in_interior(inst: &'a MetaInstance) -> Self {
        Self {
            inst,
            placement: Placement::Entity,
        }
    }

    pub fn instance(&self) -> &'a MetaInstance {
        self.inst
    }

    pub fn placement(&self) -> Placement {
        self.placement
    }

    pub fn is_interior_instance(&self) -> bool {
        self.placement == Placement::InteriorInstance
    }

    pub fn archetype(&self) -> u32 {
        field::hash_at(self.inst, keys::ARCHETYPE_NAME).unwrap_or(0)
    }

    pub fn guid(&self) -> u32 {
        field::u32_at(self.inst, keys::GUID).unwrap_or(0)
    }

    pub fn position(&self) -> Vec3 {
        field::vec3_at(self.inst, keys::POSITION)
            .map(Vec3::from)
            .unwrap_or(Vec3::ZERO)
    }

    pub fn rotation(&self) -> Quat {
        self.placement
            .rotation_of(field::vec4_at(self.inst, keys::ROTATION).unwrap_or([0.0, 0.0, 0.0, 1.0]))
    }

    pub fn scale(&self) -> Vec3 {
        transform::scale_of(
            field::f32_at(self.inst, keys::SCALE_XY).unwrap_or(1.0),
            field::f32_at(self.inst, keys::SCALE_Z).unwrap_or(1.0),
        )
    }

    pub fn flags(&self) -> u32 {
        field::u32_at(self.inst, keys::FLAGS).unwrap_or(0)
    }

    /// The raw LOD parent index, or `None` when there is none.
    ///
    /// ⚠ **This number indexes one of two different arrays** and the field
    /// itself does not say which — see [`Self::lod_in_parent_map`]. Prefer
    /// [`Self::local_parent_index`], which returns a value only where it is
    /// a position in the same map.
    ///
    /// ⚠ `-1` is the absent value and it is signed for that reason. Reading
    /// the field unsigned turns "no parent" into index 4,294,967,295.
    pub fn parent_index(&self) -> Option<usize> {
        match field::i32_at(self.inst, keys::PARENT_INDEX) {
            Some(i) if i >= 0 => Some(i as usize),
            _ => None,
        }
    }

    /// Whether this entity's LOD parent lives in the map's `parent` file.
    ///
    /// ⚠ **Bit 3 of `flags` is the whole discriminator.** Over a retail
    /// Legacy install, checking every index against the local array reports
    /// 478,261 entities pointing past the end of maps the game ships and
    /// loads. They are not broken; they index the parent map.
    pub fn lod_in_parent_map(&self) -> bool {
        self.flags() & (1 << 3) != 0
    }

    /// The parent's position in *this* map's entity array.
    ///
    /// `None` when there is no parent, or when the index belongs to another
    /// file and this one cannot check it.
    pub fn local_parent_index(&self) -> Option<usize> {
        (!self.lod_in_parent_map())
            .then(|| self.parent_index())
            .flatten()
    }

    pub fn num_children(&self) -> u32 {
        field::u32_at(self.inst, keys::NUM_CHILDREN).unwrap_or(0)
    }

    pub fn lod_dist(&self) -> f32 {
        field::f32_at(self.inst, keys::LOD_DIST).unwrap_or(0.0)
    }

    pub fn child_lod_dist(&self) -> f32 {
        field::f32_at(self.inst, keys::CHILD_LOD_DIST).unwrap_or(0.0)
    }

    pub fn lod_level(&self) -> Option<&'a str> {
        match field::get(self.inst, keys::LOD_LEVEL) {
            Some(Value::Enum { variant, .. }) => Some(variant),
            _ => None,
        }
    }

    /// The level its `lodLevel` names; `None` where it states none, or one that is no level.
    // ⚠ A variant reads three ways: by its name where the names are known, as `hash_HHHHHHHH`
    // of its name where they are not, and as `0xHHHHHHHH` where the file's own table lacks the value.
    pub fn level(&self) -> Option<Level> {
        let variant = self.lod_level()?;
        if let Some(value) = or_io::meta::binary::enum_text::value_literal(variant) {
            return u8::try_from(value)
                .ok()
                .and_then(|raw| LOD_LEVELS.iter().find(|(_, level)| *level as u8 == raw))
                .map(|(_, level)| *level);
        }
        let named = |name: &str| {
            let hash = or_io::hash::literal_string_hash(name.as_bytes());
            variant == name || variant == format!("hash_{hash:08X}")
        };
        (LOD_LEVELS.iter())
            .find(|(name, _)| named(name))
            .map(|(_, level)| *level)
    }

    pub fn extension_count(&self) -> usize {
        field::array_at(self.inst, keys::EXTENSIONS).len()
    }

    pub fn extensions(&self) -> impl Iterator<Item = Extension<'a>> + 'a {
        crate::extension::extensions_of(self.inst)
    }

    /// The world-space corners of a local box placed by this entity.
    pub fn place(&self, bb_min: Vec3, bb_max: Vec3) -> [Vec3; 8] {
        let scale = self.scale();
        transform::placed_corners(
            bb_min * scale,
            bb_max * scale,
            self.rotation(),
            self.position(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::meta::instance::value::HashedString;

    fn entity(type_name: &str, rotation: [f32; 4]) -> MetaInstance {
        let mut inst = MetaInstance::new(type_name);
        inst.set("position", Value::Vec3([1.0, 2.0, 3.0]));
        inst.set("rotation", Value::Vec4(rotation));
        inst.set("scaleXY", Value::Float(2.0));
        inst.set("scaleZ", Value::Float(3.0));
        inst.set("parentIndex", Value::Int(-1));
        inst.set(
            "archetypeName",
            Value::Hash(HashedString::from_hash(0xABCD_1234)),
        );
        inst
    }

    #[test]
    fn a_map_view_refuses_a_tree_that_is_not_one() {
        assert!(MapData::new(&MetaInstance::new("CMapTypes")).is_none());
        assert!(MapData::new(&MetaInstance::new("CMapData")).is_some());
        // And the numbered spelling of the same type is the same type.
        assert!(MapData::new(&MetaInstance::new("0xD3593FA6")).is_some());
    }

    #[test]
    fn an_interior_instance_is_recognised_in_the_same_array() {
        let plain = entity("CEntityDef", [0.0, 0.0, 0.0, 1.0]);
        let mlo = entity("CMloInstanceDef", [0.0, 0.0, 0.0, 1.0]);
        assert!(!Entity::new(&plain).is_interior_instance());
        assert!(Entity::new(&mlo).is_interior_instance());
        assert_eq!(Entity::new(&mlo).placement(), Placement::InteriorInstance);
    }

    /// An interior's contents go back to the entity convention, so the two
    /// constructors must disagree on the same bytes.
    #[test]
    fn an_entity_inside_an_interior_uses_the_entity_convention() {
        let quarter = glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let inst = entity("CEntityDef", [quarter.x, quarter.y, quarter.z, quarter.w]);
        assert!(Entity::in_interior(&inst)
            .rotation()
            .abs_diff_eq(Entity::new(&inst).rotation(), 1e-6));

        let as_mlo = entity(
            "CMloInstanceDef",
            [quarter.x, quarter.y, quarter.z, quarter.w],
        );
        assert!(!Entity::new(&as_mlo)
            .rotation()
            .abs_diff_eq(Entity::in_interior(&inst).rotation(), 1e-3));
    }

    #[test]
    fn a_negative_parent_index_is_no_parent() {
        let inst = entity("CEntityDef", [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(Entity::new(&inst).parent_index(), None);

        let mut with_parent = inst.clone();
        with_parent.set("parentIndex", Value::Int(7));
        assert_eq!(Entity::new(&with_parent).parent_index(), Some(7));
    }

    #[test]
    fn an_entity_names_its_level_by_each_spelling_a_file_gives_a_variant() {
        let stating = |variant: String| {
            let mut inst = entity("CEntityDef", [0.0, 0.0, 0.0, 1.0]);
            inst.set(
                "lodLevel",
                Value::Enum {
                    type_name: "rage__eLodType".into(),
                    variant,
                },
            );
            inst
        };
        for (name, level) in LOD_LEVELS {
            let hash = or_io::hash::literal_string_hash(name.as_bytes());
            for spelling in [
                name.to_string(),
                format!("hash_{hash:08X}"),
                format!("0x{:08X}", level as u8),
            ] {
                let inst = stating(spelling.clone());
                assert_eq!(Entity::new(&inst).level(), Some(level), "{spelling}");
            }
        }
        // The seven levels are numbered 0 to 6, each once.
        let mut numbers: Vec<u8> = LOD_LEVELS.iter().map(|(_, level)| *level as u8).collect();
        numbers.sort_unstable();
        assert_eq!(numbers, (0..Level::COUNT as u8).collect::<Vec<_>>());
        // The two levels a file numbers out of the order of their depth, and the first, each said outright.
        for (name, number, level) in [
            ("LODTYPES_DEPTH_HD", 0, Level::Hd),
            ("LODTYPES_DEPTH_ORPHANHD", 5, Level::OrphanHd),
            ("LODTYPES_DEPTH_SLOD4", 6, Level::Slod4),
        ] {
            assert_eq!(Entity::new(&stating(name.to_string())).level(), Some(level));
            let numbered = stating(format!("0x{number:08X}"));
            assert_eq!(Entity::new(&numbered).level(), Some(level), "{number}");
        }
        // No level stated, a number past the last and a name that is none read as no level.
        let none = entity("CEntityDef", [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(Entity::new(&none).level(), None);
        for other in [
            "0x00000007",
            "0xFFFFFFFF",
            "hash_00000000",
            "LODTYPES_DEPTH",
        ] {
            let inst = stating(other.to_string());
            assert_eq!(Entity::new(&inst).level(), None, "{other}");
        }
    }

    #[test]
    fn a_map_one_of_whose_entities_did_not_read_does_not_say_its_entities_are_whole() {
        let placed = |inst: MetaInstance| Value::Pointer(Some(Box::new(inst)));
        let of = |entities: Value| {
            let mut root = MetaInstance::new("CMapData");
            root.set("entities", entities);
            root
        };
        let one = || placed(entity("CEntityDef", [0.0, 0.0, 0.0, 1.0]));
        let whole = of(Value::Array(vec![one(), one()]));
        assert!(MapData::new(&whole).unwrap().entities_are_whole());
        // A pointer a lenient read could not follow is null, and the entities after it move up a place.
        let holed = of(Value::Array(vec![one(), Value::Pointer(None), one()]));
        let map = MapData::new(&holed).unwrap();
        assert_eq!((map.entity_count(), map.entities().count()), (3, 2));
        assert!(!map.entities_are_whole());
        // A member that did not decode is raw, and reads as no entity at all.
        let raw = of(Value::Raw("(decode error)".into()));
        assert!(!MapData::new(&raw).unwrap().entities_are_whole());
        // A map that places nothing, or states no entities, is whole.
        assert!(MapData::new(&of(Value::Array(Vec::new())))
            .unwrap()
            .entities_are_whole());
        assert!(MapData::new(&MetaInstance::new("CMapData"))
            .unwrap()
            .entities_are_whole());
    }

    #[test]
    fn scale_applies_xy_to_two_axes() {
        let inst = entity("CEntityDef", [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(Entity::new(&inst).scale(), Vec3::new(2.0, 2.0, 3.0));
    }

    #[test]
    fn a_box_occluder_is_stored_in_quarter_metres() {
        let mut occ = MetaInstance::new("BoxOccluder");
        occ.set("iCenterX", Value::Int(4));
        occ.set("iCenterY", Value::Int(-8));
        occ.set("iCenterZ", Value::Int(0));
        occ.set("iLength", Value::Int(40));
        occ.set("iWidth", Value::Int(20));
        occ.set("iHeight", Value::Int(12));
        let mut root = MetaInstance::new("CMapData");
        root.set(
            "boxOccluders",
            Value::Array(vec![Value::Struct(Box::new(occ))]),
        );

        let map = MapData::new(&root).unwrap();
        let box_ = map.box_occluders().next().unwrap();
        assert_eq!(box_.centre, Vec3::new(1.0, -2.0, 0.0));
        assert_eq!(box_.size, Vec3::new(10.0, 5.0, 3.0));
    }

    #[test]
    fn counts_read_zero_from_an_empty_map() {
        let root = MetaInstance::new("CMapData");
        let map = MapData::new(&root).unwrap();
        assert_eq!(map.entity_count(), 0);
        assert_eq!(map.car_generator_count(), 0);
        assert_eq!(map.box_occluder_count(), 0);
        assert!(!map.has_lod_lights());
        assert_eq!(map.grass_batches().count(), 0);
    }
}
