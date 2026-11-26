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

    /// The parked vehicles, each with every member the installs state for one.
    pub fn car_generators(&self) -> impl Iterator<Item = CarGenerator<'a>> + 'a {
        field::structs_at(self.root, keys::CAR_GENERATORS).map(|inst| CarGenerator { inst })
    }

    /// The time cycle modifiers, each applied inside its box between two hours.
    pub fn time_cycle_modifiers(&self) -> impl Iterator<Item = TimeCycleModifier<'a>> + 'a {
        field::structs_at(self.root, keys::TIME_CYCLE_MODIFIERS)
            .map(|inst| TimeCycleModifier { inst })
    }

    /// The far lights this map describes: all of each but where it stands and its colour.
    pub fn lod_lights(&self) -> Option<LodLights<'a>> {
        // ⚠ A far light is split across two maps: light `i` has its position and colour at `i` of the
        // `distant_lod_lights` of the map this one names as its parent, and no shipped map holds both families.
        field::struct_at(self.root, keys::LOD_LIGHTS_SOA).map(|inst| LodLights { inst })
    }

    /// Where the far lights of a child map stand and their colours, under the child's subscript.
    pub fn distant_lod_lights(&self) -> Option<DistantLodLights<'a>> {
        field::struct_at(self.root, keys::DISTANT_LOD_LIGHTS_SOA)
            .map(|inst| DistantLodLights { inst })
    }
}

/// One parked vehicle of a map.
#[derive(Debug, Clone, Copy)]
pub struct CarGenerator<'a> {
    inst: &'a MetaInstance,
}

impl CarGenerator<'_> {
    pub fn position(&self) -> Option<Vec3> {
        field::vec3_at(self.inst, keys::POSITION).map(Vec3::from)
    }

    /// The heading, as the two numbers the file states.
    pub fn orient(&self) -> (f32, f32) {
        (
            field::f32_at(self.inst, keys::ORIENT_X).unwrap_or(0.0),
            field::f32_at(self.inst, keys::ORIENT_Y).unwrap_or(0.0),
        )
    }

    /// The length that sizes the slot.
    pub fn perpendicular_length(&self) -> f32 {
        field::f32_at(self.inst, keys::PERPENDICULAR_LENGTH).unwrap_or(0.0)
    }

    pub fn car_model(&self) -> u32 {
        field::hash_at(self.inst, keys::CAR_MODEL).unwrap_or(0)
    }

    /// The flags, as the one word the file states.
    pub fn flags(&self) -> u32 {
        field::u32_at(self.inst, keys::FLAGS).unwrap_or(0)
    }

    pub fn body_color_remaps(&self) -> [i32; 4] {
        [
            keys::BODY_COLOR_REMAP_1,
            keys::BODY_COLOR_REMAP_2,
            keys::BODY_COLOR_REMAP_3,
            keys::BODY_COLOR_REMAP_4,
        ]
        .map(|key| field::i32_at(self.inst, key).unwrap_or(0))
    }

    pub fn pop_group(&self) -> u32 {
        field::hash_at(self.inst, keys::POP_GROUP).unwrap_or(0)
    }

    /// The livery, `-1` where the file states none.
    pub fn livery(&self) -> i32 {
        field::i32_at(self.inst, keys::LIVERY).unwrap_or(-1)
    }
}

/// One time cycle modifier of a map.
#[derive(Debug, Clone, Copy)]
pub struct TimeCycleModifier<'a> {
    inst: &'a MetaInstance,
}

impl TimeCycleModifier<'_> {
    pub fn name(&self) -> u32 {
        field::hash_at(self.inst, keys::NAME).unwrap_or(0)
    }

    /// The box the modifier is applied in.
    pub fn extents(&self) -> Option<(Vec3, Vec3)> {
        Some((
            field::vec3_at(self.inst, keys::MIN_EXTENTS)?.into(),
            field::vec3_at(self.inst, keys::MAX_EXTENTS)?.into(),
        ))
    }

    pub fn percentage(&self) -> f32 {
        field::f32_at(self.inst, keys::PERCENTAGE).unwrap_or(0.0)
    }

    pub fn range(&self) -> f32 {
        field::f32_at(self.inst, keys::RANGE).unwrap_or(0.0)
    }

    /// The hours it starts and ends at.
    pub fn hours(&self) -> (u32, u32) {
        (
            field::u32_at(self.inst, keys::START_HOUR).unwrap_or(0),
            field::u32_at(self.inst, keys::END_HOUR).unwrap_or(0),
        )
    }
}

fn float_of(value: &Value) -> Option<f32> {
    match value {
        Value::Float(v) => Some(*v as f32),
        Value::Int(v) => Some(*v as f32),
        Value::UInt(v) => Some(*v as f32),
        _ => None,
    }
}

fn word_of(value: &Value) -> Option<u32> {
    match value {
        Value::UInt(v) => Some(*v as u32),
        Value::Int(v) => Some(*v as u32),
        _ => None,
    }
}

/// The three floats of a `FloatXYZ`.
fn point_of(value: &Value) -> Option<Vec3> {
    let inst = field::as_struct(value)?;
    Some(Vec3::new(
        field::f32_at(inst, keys::X)?,
        field::f32_at(inst, keys::Y)?,
        field::f32_at(inst, keys::Z)?,
    ))
}

/// What a map of far lights states of one light.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LodLight {
    pub direction: Vec3,
    pub falloff: f32,
    pub falloff_exponent: f32,
    /// `timeAndStateFlags`, as the one word the file states.
    pub time_and_state_flags: u32,
    pub hash: u32,
    pub cone_inner_angle: u8,
    pub cone_outer_angle_or_cap_ext: u8,
    pub corona_intensity: u8,
}

/// The far lights of a map: eight arrays under one subscript.
#[derive(Debug, Clone, Copy)]
pub struct LodLights<'a> {
    inst: &'a MetaInstance,
}

impl LodLights<'_> {
    /// How many lights the first array holds; [`crate::validate`] says where another holds fewer.
    pub fn len(&self) -> usize {
        field::array_at(self.inst, keys::DIRECTION).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The light at `index`, where every array holds one there.
    pub fn get(&self, index: usize) -> Option<LodLight> {
        let at = |key| field::array_at(self.inst, key).get(index);
        let byte = |key| at(key).and_then(word_of).map(|word| word as u8);
        Some(LodLight {
            direction: at(keys::DIRECTION).and_then(point_of)?,
            falloff: at(keys::FALLOFF).and_then(float_of)?,
            falloff_exponent: at(keys::FALLOFF_EXPONENT).and_then(float_of)?,
            time_and_state_flags: at(keys::TIME_AND_STATE_FLAGS).and_then(word_of)?,
            hash: at(keys::HASH).and_then(word_of)?,
            cone_inner_angle: byte(keys::CONE_INNER_ANGLE)?,
            cone_outer_angle_or_cap_ext: byte(keys::CONE_OUTER_ANGLE_OR_CAP_EXT)?,
            corona_intensity: byte(keys::CORONA_INTENSITY)?,
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = LodLight> + '_ {
        (0..self.len()).map_while(|index| self.get(index))
    }
}

/// Where a far light stands and its colour, which the parent of its map states.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DistantLodLight {
    pub position: Vec3,
    /// `RGBI`, as the one word the file states.
    pub rgbi: u32,
}

/// The positions and colours of the far lights a child map describes: two arrays under its subscript.
#[derive(Debug, Clone, Copy)]
pub struct DistantLodLights<'a> {
    inst: &'a MetaInstance,
}

impl DistantLodLights<'_> {
    /// How many lights the first array holds; [`crate::validate`] says where the other holds fewer.
    pub fn len(&self) -> usize {
        field::array_at(self.inst, keys::POSITION).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The light at `index`, where both arrays hold one there.
    pub fn get(&self, index: usize) -> Option<DistantLodLight> {
        let at = |key| field::array_at(self.inst, key).get(index);
        Some(DistantLodLight {
            position: at(keys::POSITION).and_then(point_of)?,
            rgbi: at(keys::RGBI).and_then(word_of)?,
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = DistantLodLight> + '_ {
        (0..self.len()).map_while(|index| self.get(index))
    }

    pub fn num_street_lights(&self) -> u32 {
        field::u32_at(self.inst, keys::NUM_STREET_LIGHTS).unwrap_or(0)
    }

    pub fn category(&self) -> u32 {
        field::u32_at(self.inst, keys::CATEGORY).unwrap_or(0)
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
pub(crate) mod tests {
    use super::*;
    use or_io::decl::File;
    use or_io::meta::instance::value::HashedString;
    use or_io::meta::{CMapData, MetaDocument};

    fn items(of: impl IntoIterator<Item = MetaInstance>) -> Value {
        Value::Array(of.into_iter().map(|i| Value::Struct(Box::new(i))).collect())
    }

    fn floats(of: &[f64]) -> Value {
        Value::Array(of.iter().map(|&f| Value::Float(f)).collect())
    }

    fn words(of: &[u64]) -> Value {
        Value::Array(of.iter().map(|&w| Value::UInt(w)).collect())
    }

    fn points(of: &[[f64; 3]]) -> Value {
        items(of.iter().map(|[x, y, z]| {
            let mut point = MetaInstance::new("Item");
            point.set("x", Value::Float(*x));
            point.set("y", Value::Float(*y));
            point.set("z", Value::Float(*z));
            point
        }))
    }

    fn parked(model: &str, x: f32, livery: i64) -> MetaInstance {
        let mut car = MetaInstance::new("Item");
        car.set("position", Value::Vec3([x, -40.25, 30.0]));
        car.set("orientX", Value::Float(0.5));
        car.set("orientY", Value::Float(-0.75));
        car.set("perpendicularLength", Value::Float(2.5));
        car.set("carModel", Value::String(model.into()));
        car.set("flags", Value::UInt(1664));
        car.set("bodyColorRemap1", Value::Int(12));
        car.set("bodyColorRemap2", Value::Int(-1));
        car.set("bodyColorRemap3", Value::Int(3));
        car.set("bodyColorRemap4", Value::Int(-1));
        car.set("popGroup", Value::String("a_pop_group".into()));
        car.set("livery", Value::Int(livery));
        car
    }

    /// A map as the text a document with no schema is: two parked vehicles, a time cycle modifier and three far
    /// lights in both families, which no shipped map holds together; `edit` changes the map before it is written.
    pub(crate) fn map_file(edit: impl FnOnce(&mut MetaInstance)) -> Vec<u8> {
        let mut modifier = MetaInstance::new("Item");
        modifier.set("name", Value::String("a_warm_evening".into()));
        modifier.set("minExtents", Value::Vec3([-100.0, -50.0, 0.0]));
        modifier.set("maxExtents", Value::Vec3([100.0, 50.0, 80.0]));
        modifier.set("percentage", Value::Float(80.0));
        modifier.set("range", Value::Float(45.0));
        modifier.set("startHour", Value::UInt(20));
        modifier.set("endHour", Value::UInt(6));

        let mut lights = MetaInstance::new("LODLightsSOA");
        lights.set(
            "direction",
            points(&[[0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]]),
        );
        lights.set("falloff", floats(&[25.0, 30.5, 35.0]));
        lights.set("falloffExponent", floats(&[8.0, 16.0, 32.5]));
        lights.set("timeAndStateFlags", words(&[0x00FF_FFFF, 0x0100_0FFF, 7]));
        lights.set("hash", words(&[0xAAAA_0001, 0xAAAA_0002, 0xAAAA_0003]));
        lights.set("coneInnerAngle", words(&[10, 20, 30]));
        lights.set("coneOuterAngleOrCapExt", words(&[40, 50, 60]));
        lights.set("coronaIntensity", words(&[70, 80, 255]));

        let mut distant = MetaInstance::new("DistantLODLightsSOA");
        distant.set(
            "position",
            points(&[[10.0, 20.0, 30.0], [11.0, 21.0, 31.0], [12.0, 22.0, 32.5]]),
        );
        distant.set("RGBI", words(&[0xFF10_2030, 0x8040_5060, 0x4070_8090]));
        distant.set("numStreetLights", Value::UInt(2));
        distant.set("category", Value::UInt(1));

        let mut root = MetaInstance::new("CMapData");
        root.set("name", Value::String("town".into()));
        root.set("timeCycleModifiers", items([modifier]));
        root.set(
            "carGenerators",
            items([parked("a_saloon", 120.5, -1), parked("a_van", 124.5, 3)]),
        );
        root.set("LODLightsSOA", Value::Struct(Box::new(lights)));
        root.set("DistantLODLightsSOA", Value::Struct(Box::new(distant)));
        edit(&mut root);
        or_io::meta::write_meta(&root)
            .expect("the map is written as text")
            .into_bytes()
    }

    /// The tree `bytes` reads as, which the read views borrow.
    pub(crate) fn tree(bytes: &[u8]) -> MetaInstance {
        MetaDocument::parse(or_io::meta::default_library(), bytes.to_vec())
            .expect("the map reads")
            .instance
    }

    #[test]
    fn the_read_views_of_a_maps_records_read_what_the_typed_records_read() {
        let bytes = map_file(|_| {});
        let root = tree(&bytes);
        let viewed = MapData::new(&root).expect("a map");
        let typed = CMapData::from_bytes(&bytes).expect("opens typed");
        let at = |v: or_io::decl::Vec3| Vec3::new(v.x, v.y, v.z);

        assert_eq!(viewed.car_generator_count(), typed.car_generators.len());
        assert_eq!(typed.car_generators.len(), 2);
        for (car, typed) in viewed.car_generators().zip(&typed.car_generators) {
            assert_eq!(car.position(), Some(at(typed.position)));
            assert_eq!(car.orient(), (typed.orient_x, typed.orient_y));
            assert_eq!(car.perpendicular_length(), typed.perpendicular_length);
            assert_eq!(car.car_model(), typed.car_model.hash);
            assert_eq!(car.flags(), typed.flags);
            let remaps = [
                typed.body_color_remap1,
                typed.body_color_remap2,
                typed.body_color_remap3,
                typed.body_color_remap4,
            ];
            assert_eq!(car.body_color_remaps(), remaps);
            assert_eq!(car.pop_group(), typed.pop_group.hash);
            assert_eq!(car.livery(), i32::from(typed.livery));
        }
        let van = viewed.car_generators().nth(1).expect("the second");
        assert_eq!(van.car_model(), HashedString::new("a_van").hash);
        assert_eq!(van.position(), Some(Vec3::new(124.5, -40.25, 30.0)));
        assert_eq!((van.orient(), van.livery()), ((0.5, -0.75), 3));
        assert_eq!(van.body_color_remaps(), [12, -1, 3, -1]);

        assert_eq!(
            viewed.time_cycle_modifier_count(),
            typed.time_cycle_modifiers.len()
        );
        assert_eq!(typed.time_cycle_modifiers.len(), 1);
        for (modifier, typed) in (viewed.time_cycle_modifiers()).zip(&typed.time_cycle_modifiers) {
            assert_eq!(modifier.name(), typed.name.hash);
            assert_eq!(
                modifier.extents(),
                Some((at(typed.min_extents), at(typed.max_extents)))
            );
            assert_eq!(modifier.percentage(), typed.percentage);
            assert_eq!(modifier.range(), typed.range);
            assert_eq!(modifier.hours(), (typed.start_hour, typed.end_hour));
            assert_eq!(modifier.hours(), (20, 6));
            assert_eq!(modifier.name(), HashedString::new("a_warm_evening").hash);
        }

        let lights = viewed.lod_lights().expect("the map states the family");
        let family = &typed.lod_lights_soa;
        assert_eq!((lights.len(), family.len()), (3, 3));
        let read: Vec<LodLight> = lights.iter().collect();
        assert_eq!(read.len(), 3);
        for (index, light) in read.iter().enumerate() {
            assert_eq!(lights.get(index), Some(*light));
            let direction = &family.direction[index];
            assert_eq!(
                light.direction,
                Vec3::new(direction.x, direction.y, direction.z)
            );
            assert_eq!(light.falloff, family.falloff[index]);
            assert_eq!(light.falloff_exponent, family.falloff_exponent[index]);
            assert_eq!(
                light.time_and_state_flags,
                family.time_and_state_flags[index]
            );
            assert_eq!(light.hash, family.hash[index]);
            assert_eq!(light.cone_inner_angle, family.cone_inner_angle[index]);
            assert_eq!(
                light.cone_outer_angle_or_cap_ext,
                family.cone_outer_angle_or_cap_ext[index]
            );
            assert_eq!(light.corona_intensity, family.corona_intensity[index]);
        }
        assert_eq!(read[2].falloff_exponent, 32.5);
        assert_eq!((read[1].hash, read[2].corona_intensity), (0xAAAA_0002, 255));
        assert_eq!(lights.get(3), None);

        let distant = viewed
            .distant_lod_lights()
            .expect("the map states the family");
        let family = &typed.distant_lod_lights_soa;
        assert_eq!((distant.len(), family.len()), (3, 3));
        let read: Vec<DistantLodLight> = distant.iter().collect();
        assert_eq!(read.len(), 3);
        for (index, light) in read.iter().enumerate() {
            let position = &family.position[index];
            assert_eq!(
                light.position,
                Vec3::new(position.x, position.y, position.z)
            );
            assert_eq!(light.rgbi, family.rgbi[index]);
        }
        assert_eq!(read[2].position, Vec3::new(12.0, 22.0, 32.5));
        assert_eq!(read[0].rgbi, 0xFF10_2030);
        assert_eq!(
            distant.num_street_lights(),
            u32::from(family.num_street_lights)
        );
        assert_eq!(distant.category(), u32::from(family.category));
        assert_eq!((distant.num_street_lights(), distant.category()), (2, 1));
    }

    #[test]
    fn a_far_light_one_array_lacks_is_no_light_and_a_map_with_no_family_has_none() {
        let bytes = map_file(|root| {
            let Some(Value::Struct(lights)) = root.get_mut("LODLightsSOA") else {
                panic!("the map holds the family");
            };
            lights.set("hash", words(&[0xAAAA_0001, 0xAAAA_0002]));
        });
        let root = tree(&bytes);
        let lights = MapData::new(&root).and_then(|map| map.lod_lights());
        let lights = lights.expect("the map states the family");
        assert_eq!(lights.len(), 3);
        assert!(lights.get(1).is_some());
        assert_eq!(lights.get(2), None);
        assert_eq!(lights.iter().count(), 2);

        let bare = MetaInstance::new("CMapData");
        let map = MapData::new(&bare).expect("a map");
        assert!(map.lod_lights().is_none() && map.distant_lod_lights().is_none());
        assert_eq!(map.car_generators().count(), 0);
        assert_eq!(map.time_cycle_modifiers().count(), 0);
    }

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
