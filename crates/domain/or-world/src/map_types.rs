//! `CMapTypes`: the archetypes a `.ytyp` declares, interiors included.

use glam::Vec3;
use or_io::meta::MetaInstance;

use crate::extension::Extension;
use crate::field;
use crate::keys;
use crate::map_data::Entity;

/// A `.ytyp`'s root, borrowed.
#[derive(Debug, Clone, Copy)]
pub struct MapTypes<'a> {
    root: &'a MetaInstance,
}

impl<'a> MapTypes<'a> {
    pub fn new(root: &'a MetaInstance) -> Option<Self> {
        field::is_type(root, keys::types::C_MAP_TYPES).then_some(Self { root })
    }

    pub fn instance(&self) -> &'a MetaInstance {
        self.root
    }

    pub fn name(&self) -> u32 {
        field::hash_at(self.root, keys::NAME).unwrap_or(0)
    }

    pub fn archetypes(&self) -> impl Iterator<Item = Archetype<'a>> + 'a {
        field::structs_at(self.root, keys::ARCHETYPES).map(Archetype::new)
    }

    pub fn archetype_count(&self) -> usize {
        field::array_at(self.root, keys::ARCHETYPES).len()
    }

    pub fn dependency_count(&self) -> usize {
        field::array_at(self.root, keys::DEPENDENCIES).len()
    }

    pub fn composite_entity_type_count(&self) -> usize {
        field::array_at(self.root, keys::COMPOSITE_ENTITY_TYPES).len()
    }

    /// Extensions declared on the type file itself rather than on an archetype.
    pub fn extensions(&self) -> impl Iterator<Item = Extension<'a>> + 'a {
        crate::extension::extensions_of(self.root)
    }
}

/// Which of the three archetype declarations this is.
///
/// ⚠ All three share `CBaseArchetypeDef`'s fields and **append** to them, so
/// bounds, name and asset read the same way for each; only the extra fields
/// differ. Dispatching before reading the shared half is what makes a reader
/// miss interiors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchetypeKind {
    Base,
    Time,
    Mlo,
}

/// One archetype: a model's bounds, its assets and, for an interior, its rooms.
#[derive(Debug, Clone, Copy)]
pub struct Archetype<'a> {
    inst: &'a MetaInstance,
    kind: ArchetypeKind,
}

impl<'a> Archetype<'a> {
    pub fn new(inst: &'a MetaInstance) -> Self {
        let kind = if field::is_type(inst, keys::types::C_MLO_ARCHETYPE_DEF) {
            ArchetypeKind::Mlo
        } else if field::is_type(inst, keys::types::C_TIME_ARCHETYPE_DEF) {
            ArchetypeKind::Time
        } else {
            ArchetypeKind::Base
        };
        Self { inst, kind }
    }

    pub fn instance(&self) -> &'a MetaInstance {
        self.inst
    }

    pub fn kind(&self) -> ArchetypeKind {
        self.kind
    }

    pub fn name(&self) -> u32 {
        field::hash_at(self.inst, keys::NAME).unwrap_or(0)
    }

    pub fn asset_name(&self) -> u32 {
        field::hash_at(self.inst, keys::ASSET_NAME).unwrap_or(0)
    }

    pub fn texture_dictionary(&self) -> u32 {
        field::hash_at(self.inst, keys::TEXTURE_DICTIONARY).unwrap_or(0)
    }

    pub fn drawable_dictionary(&self) -> u32 {
        field::hash_at(self.inst, keys::DRAWABLE_DICTIONARY).unwrap_or(0)
    }

    pub fn physics_dictionary(&self) -> u32 {
        field::hash_at(self.inst, keys::PHYSICS_DICTIONARY).unwrap_or(0)
    }

    pub fn clip_dictionary(&self) -> u32 {
        field::hash_at(self.inst, keys::CLIP_DICTIONARY).unwrap_or(0)
    }

    /// What kind of file [`Self::asset_name`] names.
    pub fn asset_type(&self) -> AssetType {
        match field::get(self.inst, keys::ASSET_TYPE) {
            Some(or_io::meta::Value::Enum { variant, .. }) => AssetType::of_variant(variant),
            Some(or_io::meta::Value::UInt(n)) => AssetType::Ordinal(*n as u32),
            Some(or_io::meta::Value::Int(n)) => AssetType::Ordinal(*n as u32),
            _ => AssetType::Absent,
        }
    }

    pub fn lod_dist(&self) -> f32 {
        field::f32_at(self.inst, keys::LOD_DIST).unwrap_or(0.0)
    }

    pub fn flags(&self) -> u32 {
        field::u32_at(self.inst, keys::FLAGS).unwrap_or(0)
    }

    pub fn bounds(&self) -> Option<Bounds> {
        Some(Bounds {
            min: field::vec3_at(self.inst, keys::BB_MIN)?.into(),
            max: field::vec3_at(self.inst, keys::BB_MAX)?.into(),
            centre: field::vec3_at(self.inst, keys::BS_CENTRE)
                .map(Vec3::from)
                .unwrap_or(Vec3::ZERO),
            radius: field::f32_at(self.inst, keys::BS_RADIUS).unwrap_or(0.0),
        })
    }

    pub fn extensions(&self) -> impl Iterator<Item = Extension<'a>> + 'a {
        crate::extension::extensions_of(self.inst)
    }

    /// The interior this archetype describes, if it is one.
    pub fn interior(&self) -> Option<Interior<'a>> {
        (self.kind == ArchetypeKind::Mlo).then_some(Interior { inst: self.inst })
    }
}

/// Which file family an archetype's `assetName` belongs to.
///
/// ⚠ `Ordinal` is not a defect: the field is an enum whose variant table lives
/// in the schema snapshot, so a build without one reads the bare stored number.
/// A caller needing the answer there corroborates it against the install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetType {
    Uninitialized,
    /// A `.yft`.
    Fragment,
    /// A `.ydr`.
    Drawable,
    /// An entry inside the archetype's `drawableDictionary`.
    DrawableDictionary,
    /// Declares no asset at all.
    Assetless,
    /// A variant the snapshot named and this list does not cover.
    Named(&'static str),
    /// Stored as a number, with no variant table to name it.
    Ordinal(u32),
    Absent,
}

impl AssetType {
    fn of_variant(variant: &str) -> Self {
        match variant.rsplit("ASSET_TYPE_").next().unwrap_or(variant) {
            "UNINITIALIZED" => Self::Uninitialized,
            "FRAGMENT" => Self::Fragment,
            "DRAWABLE" => Self::Drawable,
            "DRAWABLEDICTIONARY" => Self::DrawableDictionary,
            "ASSETLESS" => Self::Assetless,
            _ => Self::Named("other"),
        }
    }

    /// Whether the archetype names an asset a streamer would go and load.
    pub fn names_an_asset(&self) -> bool {
        matches!(
            self,
            Self::Fragment | Self::Drawable | Self::DrawableDictionary
        )
    }
}

/// A model's bounding box and sphere, in its own space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min: Vec3,
    pub max: Vec3,
    pub centre: Vec3,
    pub radius: f32,
}

/// The rooms, portals and entity sets of a `CMloArchetypeDef`.
#[derive(Debug, Clone, Copy)]
pub struct Interior<'a> {
    inst: &'a MetaInstance,
}

impl<'a> Interior<'a> {
    pub fn mlo_flags(&self) -> u32 {
        field::u32_at(self.inst, keys::MLO_FLAGS).unwrap_or(0)
    }

    pub fn rooms(&self) -> impl Iterator<Item = Room<'a>> + 'a {
        field::structs_at(self.inst, keys::ROOMS).map(|inst| Room { inst })
    }

    pub fn room_count(&self) -> usize {
        field::array_at(self.inst, keys::ROOMS).len()
    }

    pub fn portals(&self) -> impl Iterator<Item = Portal<'a>> + 'a {
        field::structs_at(self.inst, keys::PORTALS).map(|inst| Portal { inst })
    }

    pub fn portal_count(&self) -> usize {
        field::array_at(self.inst, keys::PORTALS).len()
    }

    pub fn entity_sets(&self) -> impl Iterator<Item = EntitySet<'a>> + 'a {
        field::structs_at(self.inst, keys::ENTITY_SETS).map(|inst| EntitySet { inst })
    }

    pub fn entity_set_count(&self) -> usize {
        field::array_at(self.inst, keys::ENTITY_SETS).len()
    }

    /// The interior's always-present entities, in the order rooms index them.
    ///
    /// ⚠ This order is load-bearing: a room's `attachedObjects` and a
    /// portal's are **indices into this array**, so reordering or compacting
    /// it silently re-attaches every object in the interior.
    pub fn entities(&self) -> impl Iterator<Item = Entity<'a>> + 'a {
        field::structs_at(self.inst, keys::ENTITIES).map(Entity::in_interior)
    }

    pub fn entity_count(&self) -> usize {
        field::array_at(self.inst, keys::ENTITIES).len()
    }

    /// Whether every entity read, so a place in [`Self::entities`] is the place a room or a `parentIndex` names.
    pub fn entities_are_whole(&self) -> bool {
        field::structs_fill(self.inst, keys::ENTITIES)
    }

    pub fn time_cycle_modifier_count(&self) -> usize {
        field::array_at(self.inst, keys::TIME_CYCLE_MODIFIERS).len()
    }
}

/// One room of an interior.
#[derive(Debug, Clone, Copy)]
pub struct Room<'a> {
    inst: &'a MetaInstance,
}

impl<'a> Room<'a> {
    /// ⚠ A room's name is a **string**, not a hash — the one place in this
    /// family where reading it as a hash yields nothing.
    pub fn name(&self) -> Option<String> {
        field::string_at(self.inst, keys::NAME)
    }

    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        Some((
            field::vec3_at(self.inst, keys::BB_MIN)?.into(),
            field::vec3_at(self.inst, keys::BB_MAX)?.into(),
        ))
    }

    pub fn flags(&self) -> u32 {
        field::u32_at(self.inst, keys::FLAGS).unwrap_or(0)
    }

    pub fn floor_id(&self) -> i32 {
        field::i32_at(self.inst, keys::FLOOR_ID).unwrap_or(0)
    }

    pub fn blend(&self) -> f32 {
        field::f32_at(self.inst, keys::BLEND).unwrap_or(0.0)
    }

    pub fn timecycle(&self) -> u32 {
        field::hash_at(self.inst, keys::TIMECYCLE_NAME).unwrap_or(0)
    }

    /// How many portals the room states lead into or out of it, which the file states apart from its portals.
    pub fn portal_count(&self) -> u32 {
        field::u32_at(self.inst, keys::PORTAL_COUNT).unwrap_or(0)
    }

    pub fn attached_objects(&self) -> impl Iterator<Item = u32> + 'a {
        indices(self.inst, keys::ATTACHED_OBJECTS)
    }
}

/// One portal between two rooms.
#[derive(Debug, Clone, Copy)]
pub struct Portal<'a> {
    inst: &'a MetaInstance,
}

impl<'a> Portal<'a> {
    pub fn room_from(&self) -> u32 {
        field::u32_at(self.inst, keys::ROOM_FROM).unwrap_or(0)
    }

    pub fn room_to(&self) -> u32 {
        field::u32_at(self.inst, keys::ROOM_TO).unwrap_or(0)
    }

    pub fn flags(&self) -> u32 {
        field::u32_at(self.inst, keys::FLAGS).unwrap_or(0)
    }

    pub fn opacity(&self) -> u32 {
        field::u32_at(self.inst, keys::OPACITY).unwrap_or(0)
    }

    /// The portal's quad, in interior space.
    ///
    /// ⚠ Four corners is the shape the engine expects; anything else is a
    /// portal it will not render through.
    pub fn corners(&self) -> Vec<Vec3> {
        field::array_at(self.inst, keys::CORNERS)
            .iter()
            .filter_map(|v| match v {
                or_io::meta::Value::Vec3(a) => Some(Vec3::from(*a)),
                or_io::meta::Value::Vec4(a) => Some(Vec3::new(a[0], a[1], a[2])),
                _ => None,
            })
            .collect()
    }

    pub fn attached_objects(&self) -> impl Iterator<Item = u32> + 'a {
        indices(self.inst, keys::ATTACHED_OBJECTS)
    }
}

/// A set of entities a room can switch on.
#[derive(Debug, Clone, Copy)]
pub struct EntitySet<'a> {
    inst: &'a MetaInstance,
}

impl<'a> EntitySet<'a> {
    pub fn name(&self) -> u32 {
        field::hash_at(self.inst, keys::NAME).unwrap_or(0)
    }

    /// One room index per entity, parallel to [`Self::entities`].
    ///
    /// ⚠ **Paired arrays sharing a subscript.** The engine reads
    /// `locations[i]` for `entities[i]`; a set whose two arrays differ in
    /// length puts entities in the wrong room or reads past the end.
    pub fn locations(&self) -> impl Iterator<Item = u32> + 'a {
        indices(self.inst, keys::LOCATIONS)
    }

    pub fn location_count(&self) -> usize {
        field::array_at(self.inst, keys::LOCATIONS).len()
    }

    pub fn entities(&self) -> impl Iterator<Item = Entity<'a>> + 'a {
        field::structs_at(self.inst, keys::ENTITIES).map(Entity::in_interior)
    }

    pub fn entity_count(&self) -> usize {
        field::array_at(self.inst, keys::ENTITIES).len()
    }
}

fn indices(inst: &MetaInstance, key: u32) -> impl Iterator<Item = u32> + '_ {
    field::array_at(inst, key).iter().filter_map(|v| match v {
        or_io::meta::Value::UInt(n) => Some(*n as u32),
        or_io::meta::Value::Int(n) => Some(*n as u32),
        or_io::meta::Value::Hash(h) => Some(h.hash),
        _ => None,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use or_io::decl::File;
    use or_io::meta::instance::value::HashedString;
    use or_io::meta::{CBaseArchetypeDefAny, CMapTypes, MetaDocument, Value};

    fn items(of: impl IntoIterator<Item = MetaInstance>) -> Value {
        Value::Array(of.into_iter().map(|i| Value::Struct(Box::new(i))).collect())
    }

    fn pointers(of: impl IntoIterator<Item = MetaInstance>) -> Value {
        let behind = |i| Value::Pointer(Some(Box::new(i)));
        Value::Array(of.into_iter().map(behind).collect())
    }

    fn indexes(of: &[u32]) -> Value {
        Value::Array(of.iter().map(|&i| Value::UInt(u64::from(i))).collect())
    }

    fn placed(guid: u32) -> MetaInstance {
        let mut entity = MetaInstance::new("CEntityDef");
        entity.set("archetypeName", Value::String("prop_bench_01a".into()));
        entity.set("guid", Value::UInt(u64::from(guid)));
        entity.set("parentIndex", Value::Int(-1));
        entity
    }

    fn a_room(name: &str, floor: i64, attached: &[u32]) -> MetaInstance {
        let mut room = MetaInstance::new("Item");
        room.set("name", Value::String(name.into()));
        room.set("bbMin", Value::Vec3([-4.0, -3.0, floor as f32]));
        room.set("bbMax", Value::Vec3([4.0, 3.0, floor as f32 + 2.5]));
        room.set("blend", Value::Float(0.75));
        room.set("timecycleName", Value::String("int_hall".into()));
        room.set("flags", Value::UInt(96));
        room.set("portalCount", Value::UInt(1));
        room.set("floorId", Value::Int(floor));
        room.set("attachedObjects", indexes(attached));
        room
    }

    /// A type file with one interior, as the text a document with no schema is: two rooms, a portal between them with a
    /// door attached, and an entity set of two lamps in the second room; `edit` changes the interior before it is written.
    pub(crate) fn interior_file(edit: impl FnOnce(&mut MetaInstance)) -> Vec<u8> {
        let mut portal = MetaInstance::new("Item");
        portal.set("roomFrom", Value::UInt(0));
        portal.set("roomTo", Value::UInt(1));
        portal.set("flags", Value::UInt(8));
        portal.set("opacity", Value::UInt(40));
        let corners = [
            [1.0, 3.0, 0.0],
            [-1.0, 3.0, 0.0],
            [-1.0, 3.0, 2.0],
            [1.0, 3.0, 2.0],
        ];
        portal.set("corners", Value::Array(corners.map(Value::Vec3).to_vec()));
        portal.set("attachedObjects", indexes(&[2]));

        let mut set = MetaInstance::new("Item");
        set.set("name", Value::String("set_party".into()));
        set.set("locations", indexes(&[1, 1]));
        set.set("entities", pointers([placed(10), placed(11)]));

        let mut interior = MetaInstance::new("CMloArchetypeDef");
        interior.set("name", Value::String("v_int_hills".into()));
        interior.set("mloFlags", Value::UInt(1));
        interior.set("entities", pointers([placed(1), placed(2), placed(3)]));
        let rooms = [a_room("limbo", 0, &[]), a_room("hall", 1, &[0, 1])];
        interior.set("rooms", items(rooms));
        interior.set("portals", items([portal]));
        interior.set("entitySets", items([set]));
        edit(&mut interior);

        let mut root = MetaInstance::new("CMapTypes");
        root.set(
            "archetypes",
            Value::Array(vec![Value::Pointer(Some(Box::new(interior)))]),
        );
        or_io::meta::write_meta(&root)
            .expect("the type file is written as text")
            .into_bytes()
    }

    /// The tree `bytes` reads as, which the read views borrow.
    pub(crate) fn tree(bytes: &[u8]) -> MetaInstance {
        MetaDocument::parse(or_io::meta::default_library(), bytes.to_vec())
            .expect("the type file reads")
            .instance
    }

    #[test]
    fn the_read_views_of_an_interior_read_what_the_typed_records_read() {
        let bytes = interior_file(|_| {});
        let root = tree(&bytes);
        let types = MapTypes::new(&root).expect("a type file");
        let viewed = types.archetypes().next().and_then(|a| a.interior());
        let viewed = viewed.expect("the archetype is an interior");
        let typed = CMapTypes::from_bytes(&bytes).expect("opens typed");
        let CBaseArchetypeDefAny::CMloArchetypeDef(typed) = &typed.archetypes[0] else {
            panic!("the archetype is an interior");
        };

        assert_eq!(viewed.mlo_flags(), typed.mlo_flags);
        assert_eq!(viewed.entity_count(), typed.entities.len());
        assert_eq!(viewed.room_count(), typed.rooms.len());
        assert_eq!(typed.rooms.len(), 2);
        for (room, typed) in viewed.rooms().zip(&typed.rooms) {
            assert_eq!(room.name().as_deref(), Some(typed.name.as_str()));
            let corner = |v: or_io::decl::Vec3| Vec3::new(v.x, v.y, v.z);
            assert_eq!(
                room.bounds(),
                Some((corner(typed.bb_min), corner(typed.bb_max)))
            );
            assert_eq!(room.flags(), typed.flags);
            assert_eq!(room.floor_id(), typed.floor_id);
            assert_eq!((room.portal_count(), typed.portal_count), (1, 1));
            assert_eq!(room.blend(), typed.blend);
            assert_eq!(room.timecycle(), typed.timecycle_name.hash);
            assert_eq!(
                room.attached_objects().collect::<Vec<_>>(),
                typed.attached_objects
            );
        }
        assert_eq!(typed.rooms[1].name, "hall");
        assert_eq!(
            typed.rooms[1].timecycle_name.hash,
            HashedString::new("int_hall").hash
        );

        assert_eq!(viewed.portal_count(), typed.portals.len());
        assert_eq!(typed.portals.len(), 1);
        for (portal, typed) in viewed.portals().zip(&typed.portals) {
            assert_eq!(
                (portal.room_from(), portal.room_to()),
                (typed.room_from, typed.room_to)
            );
            assert_eq!(
                (portal.flags(), portal.opacity()),
                (typed.flags, typed.opacity)
            );
            let corners: Vec<Vec3> = (typed.corners.iter())
                .map(|c| Vec3::new(c.x, c.y, c.z))
                .collect();
            assert_eq!(portal.corners(), corners);
            assert_eq!(corners.len(), 4);
            assert_eq!(
                portal.attached_objects().collect::<Vec<_>>(),
                typed.attached_objects
            );
        }

        assert_eq!(viewed.entity_set_count(), typed.entity_sets.len());
        assert_eq!(typed.entity_sets.len(), 1);
        for (set, typed) in viewed.entity_sets().zip(&typed.entity_sets) {
            assert_eq!(set.name(), typed.name.hash);
            assert_eq!(set.locations().collect::<Vec<_>>(), typed.locations);
            assert_eq!(set.entity_count(), typed.entities.len());
            assert_eq!(typed.locations, [1, 1]);
        }
        assert_eq!(
            viewed.time_cycle_modifier_count(),
            typed.time_cycle_modifiers.len()
        );
    }

    fn archetype(type_name: &str) -> MetaInstance {
        let mut inst = MetaInstance::new(type_name);
        inst.set("bbMin", Value::Vec3([-1.0, -2.0, -3.0]));
        inst.set("bbMax", Value::Vec3([1.0, 2.0, 3.0]));
        inst.set("bsRadius", Value::Float(4.0));
        inst.set("lodDist", Value::Float(100.0));
        inst
    }

    #[test]
    fn every_archetype_kind_reads_the_shared_half() {
        for (type_name, kind) in [
            ("CBaseArchetypeDef", ArchetypeKind::Base),
            ("CTimeArchetypeDef", ArchetypeKind::Time),
            ("CMloArchetypeDef", ArchetypeKind::Mlo),
        ] {
            let inst = archetype(type_name);
            let arch = Archetype::new(&inst);
            assert_eq!(arch.kind(), kind, "{type_name}");
            let bounds = arch.bounds().expect(type_name);
            assert_eq!(bounds.min, Vec3::new(-1.0, -2.0, -3.0));
            assert_eq!(bounds.radius, 4.0);
            assert_eq!(arch.lod_dist(), 100.0);
        }
    }

    #[test]
    fn only_an_mlo_has_an_interior() {
        assert!(Archetype::new(&archetype("CBaseArchetypeDef"))
            .interior()
            .is_none());
        assert!(Archetype::new(&archetype("CMloArchetypeDef"))
            .interior()
            .is_some());
    }

    #[test]
    fn a_room_name_is_a_string() {
        let mut room = MetaInstance::new("CMloRoomDef");
        room.set("name", Value::String("limbo".into()));
        let room = Room { inst: &room };
        assert_eq!(room.name().as_deref(), Some("limbo"));
    }

    #[test]
    fn an_entity_set_pairs_its_two_arrays_by_index() {
        let mut set = MetaInstance::new("CMloEntitySet");
        set.set(
            "locations",
            Value::Array(vec![Value::UInt(0), Value::UInt(3)]),
        );
        set.set(
            "entities",
            Value::Array(vec![
                Value::Struct(Box::new(MetaInstance::new("CEntityDef"))),
                Value::Struct(Box::new(MetaInstance::new("CEntityDef"))),
            ]),
        );
        let set = EntitySet { inst: &set };
        assert_eq!(set.location_count(), set.entity_count());
        assert_eq!(set.locations().collect::<Vec<_>>(), vec![0, 3]);
    }

    #[test]
    fn an_asset_type_reads_as_a_variant_or_as_the_number_it_is_stored_as() {
        let mut named = MetaInstance::new("CBaseArchetypeDef");
        named.set(
            "assetType",
            Value::Enum {
                type_name: "assetType".into(),
                variant: "ASSET_TYPE_DRAWABLEDICTIONARY".into(),
            },
        );
        assert_eq!(
            Archetype::new(&named).asset_type(),
            AssetType::DrawableDictionary
        );

        let mut numbered = MetaInstance::new("CBaseArchetypeDef");
        numbered.set("assetType", Value::UInt(3));
        assert_eq!(
            Archetype::new(&numbered).asset_type(),
            AssetType::Ordinal(3)
        );

        assert_eq!(
            Archetype::new(&MetaInstance::new("CBaseArchetypeDef")).asset_type(),
            AssetType::Absent
        );
    }

    #[test]
    fn assetless_and_uninitialized_name_no_asset() {
        for (variant, names) in [
            ("ASSET_TYPE_FRAGMENT", true),
            ("ASSET_TYPE_DRAWABLE", true),
            ("ASSET_TYPE_DRAWABLEDICTIONARY", true),
            ("ASSET_TYPE_ASSETLESS", false),
            ("ASSET_TYPE_UNINITIALIZED", false),
        ] {
            let mut inst = MetaInstance::new("CBaseArchetypeDef");
            inst.set(
                "assetType",
                Value::Enum {
                    type_name: "assetType".into(),
                    variant: variant.into(),
                },
            );
            assert_eq!(
                Archetype::new(&inst).asset_type().names_an_asset(),
                names,
                "{variant}"
            );
        }
    }

    #[test]
    fn the_four_dictionary_fields_read_as_hashes() {
        let mut inst = MetaInstance::new("CBaseArchetypeDef");
        for (field, name) in [
            ("textureDictionary", "txd_a"),
            ("drawableDictionary", "ydd_a"),
            ("physicsDictionary", "ybn_a"),
            ("clipDictionary", "ycd_a"),
        ] {
            inst.set(
                field,
                Value::Hash(or_io::meta::instance::value::HashedString::new(name)),
            );
        }
        let arch = Archetype::new(&inst);
        let hash = |n: &str| or_io::string_hash(n.as_bytes());
        assert_eq!(arch.texture_dictionary(), hash("txd_a"));
        assert_eq!(arch.drawable_dictionary(), hash("ydd_a"));
        assert_eq!(arch.physics_dictionary(), hash("ybn_a"));
        assert_eq!(arch.clip_dictionary(), hash("ycd_a"));
    }

    #[test]
    fn a_map_types_view_refuses_a_map_data_tree() {
        assert!(MapTypes::new(&MetaInstance::new("CMapData")).is_none());
        assert!(MapTypes::new(&MetaInstance::new("0xD98BB561")).is_some());
    }
}
