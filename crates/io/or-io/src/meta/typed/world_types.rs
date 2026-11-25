//! The world family's classes, declared here so a map or a type file built from nothing saves where
//! no schema snapshot is installed, which is every user's machine.
//!
//! Each class states the members its typed record names (`CMapData::MEMBERS` and its siblings), in
//! the engine's order and with the engine's types, and
//! `every_declared_world_class_states_the_members_its_record_types` holds the two together. A class
//! the engine derives from another (`CMloInstanceDef`, `CTimeArchetypeDef`, `CMloArchetypeDef`) is
//! not declared: the members that make one are not typed yet.
//!
//! ⚠ **A document these types lay out is a PSO, never an RBM.** An RBM class carries a `signature`
//! word that nothing in this tree derives, and a PSO type table states none, so a root built here
//! is written as PSO. A record built in an RBM file whose own table does not state its class is
//! refused with that reason where no snapshot declares the class either; where one does, the class
//! is laid out from it with a zero signature.
//!
//! ⚠ **The declaration is the last source and the first.** A class the document's own table states
//! is laid out as that table states it and never as declared here. Where the table states none, the
//! declaration wins over an installed snapshot: a project's source compiles through its base's type
//! table with no snapshot, so a save has to write the same members on every machine.

use std::sync::OnceLock;

use crate::meta::binary::synth::name_hash;
use crate::meta::binary::type_tables;
use crate::meta::instance::{MetaInstance, SourceFormat, Value};
use crate::meta::schema::{parse_psc, SchemaItem, SchemaLibrary};

use super::{ped_types, Arrival};

pub const WORLD_TYPES: &str = r#"<ParserSchema>
  <structdef type="CEntityDef">
    <string name="archetypeName" type="atHashString"/>
    <u32 name="flags"/>
    <u32 name="guid"/>
    <Vector3 name="position"/>
    <Vector4 name="rotation"/>
    <float name="scaleXY"/>
    <float name="scaleZ"/>
    <s32 name="parentIndex"/>
    <float name="lodDist"/>
    <float name="childLodDist"/>
    <u32 name="numChildren"/>
    <s32 name="ambientOcclusionMultiplier" init="255"/>
    <s32 name="artificialAmbientOcclusion" init="255"/>
    <u32 name="tintValue" init="0"/>
  </structdef>
  <structdef type="CMapData">
    <string name="name" type="atHashString"/>
    <string name="parent" type="atHashString"/>
    <u32 name="flags"/>
    <u32 name="contentFlags"/>
    <Vector3 name="streamingExtentsMin"/>
    <Vector3 name="streamingExtentsMax"/>
    <Vector3 name="entitiesExtentsMin"/>
    <Vector3 name="entitiesExtentsMax"/>
    <array name="entities" type="atArray"><pointer type="rage::fwEntityDef" policy="owner"/></array>
    <array name="physicsDictionaries" type="atArray"><string type="atHashString"/></array>
  </structdef>
  <structdef type="CBaseArchetypeDef">
    <float name="lodDist"/>
    <u32 name="flags"/>
    <u32 name="specialAttribute"/>
    <Vector3 name="bbMin"/>
    <Vector3 name="bbMax"/>
    <Vector3 name="bsCentre"/>
    <float name="bsRadius"/>
    <float name="hdTextureDist" init="5.0"/>
    <string name="name" type="atHashString"/>
    <string name="textureDictionary" type="atHashString"/>
    <string name="clipDictionary" type="atHashString"/>
    <string name="drawableDictionary" type="atHashString"/>
    <string name="physicsDictionary" type="atHashString"/>
    <string name="assetName" type="atHashString"/>
  </structdef>
  <structdef type="CMapTypes">
    <array name="archetypes" type="atArray"><pointer type="rage::fwArchetypeDef" policy="owner"/></array>
    <string name="name" type="atHashString"/>
    <array name="dependencies" type="atArray"><string type="atHashString"/></array>
  </structdef>
</ParserSchema>"#;

/// [`WORLD_TYPES`] as a library.
pub fn declared() -> &'static SchemaLibrary {
    static LIBRARY: OnceLock<SchemaLibrary> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        let mut library = SchemaLibrary::new();
        let items = parse_psc(WORLD_TYPES.as_bytes(), "world")
            .expect("the world types are well-formed PSC");
        for item in items {
            match item {
                SchemaItem::Struct(def) => library.insert_struct(def),
                SchemaItem::Enum(def) => library.insert_enum(def),
                SchemaItem::Skipped => {}
            }
        }
        library
    })
}

/// Why a `type_name` built in an RBM file whose table states no such class is not saved.
fn no_signature(type_name: &str) -> String {
    format!(
        "a `{type_name}` built here cannot be saved into this file: it is an RBM file whose own type table states no `{type_name}`, and an RBM class carries a signature word that nothing derives, so there is none to write; add the first as a copy of one another file holds, or build the file from nothing, which is written as PSO"
    )
}

/// The types a world document is saved against, and which of them this declaration gave.
pub(super) struct Saving {
    pub(super) library: SchemaLibrary,
    /// The name hash of each class `library` takes from [`declared`].
    declared: Vec<u32>,
    /// Whether the document arrived as RBM, whose classes each carry a signature.
    rbm: bool,
}

impl Saving {
    /// The types of a root that arrived as `arrival`, or was built here when there is none.
    pub(super) fn of(arrival: Option<&Arrival>) -> Self {
        let family = ped_types::library(crate::meta::default_library());
        let own = arrival
            .and_then(|a| a.layout.as_ref())
            .map(type_tables::of_layout);
        let mut library = match &own {
            Some(tables) => type_tables::extend(&family, tables).into_owned(),
            None => family.into_owned(),
        };
        let stated = |name: &str| {
            let key = name_hash(name);
            own.as_ref()
                .is_some_and(|tables| tables.structs.keys().any(|k| name_hash(k) == key))
        };
        let mut given = Vec::new();
        if arrival.is_none_or(|a| a.format == SourceFormat::Pso) {
            for (name, def) in &declared().structs {
                if !stated(name) {
                    library.insert_struct(def.clone());
                    given.push(name_hash(name));
                }
            }
        }
        Self {
            library,
            declared: given,
            rbm: arrival.is_some_and(|a| a.format == SourceFormat::Rbm),
        }
    }

    /// Gives `inst` the members its class has in this document.
    pub(super) fn conform(&self, inst: &mut MetaInstance) -> Result<(), String> {
        if self.rbm && self.library.resolve_any(&inst.type_name).is_none() {
            return Err(no_signature(&inst.type_name));
        }
        super::conform(&self.library, inst)
    }

    /// Refuses a record laid out by a class declared here, or by one derived from it, that holds a member the layout would drop.
    pub(super) fn refuse_what_would_be_dropped(&self, root: &MetaInstance) -> Result<(), String> {
        fn dropped<'a>(saving: &Saving, inst: &'a MetaInstance) -> Option<&'a str> {
            let resolved = saving.library.resolve_any(&inst.type_name)?;
            let declared_here = |class: &str| saving.declared.contains(&name_hash(class));
            if !resolved.chain.iter().any(|def| declared_here(&def.name)) {
                return None;
            }
            let held = |member: &str| {
                let key = name_hash(member);
                resolved.fields.iter().any(|f| name_hash(&f.name) == key)
            };
            let lost = inst.fields.iter().find(|(member, _)| !held(member));
            lost.map(|(member, _)| member.as_str())
        }
        fn first<'a>(saving: &Saving, inst: &'a MetaInstance) -> Option<(&'a str, &'a str)> {
            if let Some(member) = dropped(saving, inst) {
                return Some((inst.type_name.as_str(), member));
            }
            inst.fields.iter().find_map(|(_, v)| in_value(saving, v))
        }
        fn in_value<'a>(saving: &Saving, value: &'a Value) -> Option<(&'a str, &'a str)> {
            match value {
                Value::Struct(inner) | Value::Pointer(Some(inner)) => first(saving, inner),
                Value::Array(items) => items.iter().find_map(|v| in_value(saving, v)),
                _ => None,
            }
        }
        match first(self, root) {
            Some((class, member)) => Err(format!(
                "a `{class}` here holds `{member}`, a member of the file it came from: this file's own type table does not state the class it is laid out as, and the class as it is built from nothing has no such member, so the save would drop it; build the record here instead of copying it"
            )),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decl::{File, Vec3, Vec4};
    use crate::meta::binary::hash_index::with_public_names_only;
    use crate::meta::instance::HashedString;
    use crate::meta::typed::{
        CBaseArchetypeDef, CBaseArchetypeDefAny, CEntityDef, CEntityDefAny, CMapData, CMapTypes,
    };
    use crate::meta::{
        build_library, canonical, detect_binary_format, write_meta_binary, BinaryFormat,
        MetaDocument,
    };

    fn members(class: &str) -> Vec<String> {
        declared()
            .resolve(class)
            .unwrap_or_else(|| panic!("`{class}` is declared"))
            .fields
            .iter()
            .map(|f| f.name.clone())
            .collect()
    }

    fn sorted(names: impl IntoIterator<Item = impl Into<String>>) -> Vec<String> {
        let mut names: Vec<String> = names.into_iter().map(Into::into).collect();
        names.sort();
        names
    }

    #[test]
    fn every_declared_world_class_states_the_members_its_record_types() {
        for (class, typed) in [
            ("CEntityDef", CEntityDef::MEMBERS),
            ("CMapData", CMapData::MEMBERS),
            ("CBaseArchetypeDef", CBaseArchetypeDef::MEMBERS),
            ("CMapTypes", CMapTypes::MEMBERS),
        ] {
            assert_eq!(
                sorted(members(class)),
                sorted(typed.iter().copied()),
                "`{class}` declares what its record types, no member more and none fewer"
            );
        }
        assert_eq!(declared().structs.len(), 4, "and no class besides");
    }

    fn bench(guid: u32) -> CEntityDefAny {
        CEntityDefAny::Base(CEntityDef {
            archetype_name: HashedString::new("prop_bench_01a"),
            flags: 32,
            guid,
            position: Vec3 {
                x: 120.5,
                y: -40.25,
                z: 30.0,
            },
            rotation: Vec4 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            },
            scale_xy: 1.0,
            scale_z: 1.0,
            parent_index: -1,
            lod_dist: 60.0,
            ..Default::default()
        })
    }

    fn hills() -> CMapData {
        CMapData {
            name: HashedString::new("hills"),
            entities: vec![bench(1)],
            ..Default::default()
        }
    }

    /// The members `inst` states, by name hash, in the order it states them.
    fn stated(inst: &MetaInstance) -> Vec<u32> {
        inst.fields.iter().map(|(n, _)| name_hash(n)).collect()
    }

    fn hashes(names: &[&str]) -> Vec<u32> {
        names.iter().map(|n| name_hash(n)).collect()
    }

    fn tree(bytes: &[u8]) -> MetaInstance {
        MetaDocument::parse(&SchemaLibrary::default(), bytes.to_vec())
            .expect("the save reads")
            .instance
    }

    fn elements<'a>(root: &'a MetaInstance, array: &str) -> Vec<&'a MetaInstance> {
        let key = name_hash(array);
        let found = root.fields.iter().find(|(n, _)| name_hash(n) == key);
        match found {
            Some((_, Value::Array(items))) => items
                .iter()
                .filter_map(|v| match v {
                    Value::Struct(inst) | Value::Pointer(Some(inst)) => Some(&**inst),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn a_map_built_from_nothing_saves_as_pso_and_reads_back_as_built() {
        let built = hills();
        let saved = built.to_bytes().expect("a map built from nothing saves");
        assert_eq!(detect_binary_format(&saved), Some(BinaryFormat::Pso));

        let back = CMapData::from_bytes(&saved).expect("and reopens");
        assert!(back.name == "hills");
        assert_eq!(back.entities.len(), 1);
        let entity = &back.entities[0];
        assert!(matches!(entity, CEntityDefAny::Base(_)));
        assert!(entity.archetype_name == "prop_bench_01a");
        assert_eq!(
            (entity.flags, entity.guid, entity.parent_index),
            (32, 1, -1)
        );
        assert_eq!(entity.position, built.entities[0].position);
        assert_eq!(entity.rotation, built.entities[0].rotation);
        assert_eq!(
            (entity.scale_xy, entity.scale_z, entity.lod_dist),
            (1.0, 1.0, 60.0)
        );
        assert_eq!(
            (
                entity.ambient_occlusion_multiplier,
                entity.artificial_ambient_occlusion
            ),
            (255, 255),
            "the engine's inits are stated"
        );
        assert_eq!(back.to_bytes().expect("saves again"), saved);

        let root = tree(&saved);
        assert_eq!(
            sorted_hashes(stated(&root)),
            sorted_hashes(hashes(CMapData::MEMBERS))
        );
        let entities = elements(&root, "entities");
        assert_eq!(entities.len(), 1);
        assert_eq!(
            sorted_hashes(stated(entities[0])),
            sorted_hashes(hashes(CEntityDef::MEMBERS))
        );
    }

    fn sorted_hashes(mut hashes: Vec<u32>) -> Vec<u32> {
        hashes.sort_unstable();
        hashes
    }

    #[test]
    fn a_type_file_built_from_nothing_saves_as_pso_and_reads_back_as_built() {
        let built = CMapTypes {
            name: HashedString::new("hills_types"),
            archetypes: vec![CBaseArchetypeDefAny::Base(CBaseArchetypeDef {
                lod_dist: 80.0,
                bb_min: Vec3 {
                    x: -1.0,
                    y: -2.0,
                    z: 0.0,
                },
                bb_max: Vec3 {
                    x: 1.0,
                    y: 2.0,
                    z: 3.0,
                },
                bs_radius: 2.5,
                name: HashedString::new("prop_bench_01a"),
                asset_name: HashedString::new("prop_bench_01a"),
                ..Default::default()
            })],
            ..Default::default()
        };
        let saved = built
            .to_bytes()
            .expect("a type file built from nothing saves");
        assert_eq!(detect_binary_format(&saved), Some(BinaryFormat::Pso));

        let back = CMapTypes::from_bytes(&saved).expect("and reopens");
        assert!(back.name == "hills_types");
        assert_eq!(back.archetypes.len(), 1);
        let archetype = &back.archetypes[0];
        assert!(matches!(archetype, CBaseArchetypeDefAny::Base(_)));
        assert!(archetype.name == "prop_bench_01a" && archetype.asset_name == "prop_bench_01a");
        assert_eq!(archetype.bb_max, built.archetypes[0].bb_max);
        assert_eq!((archetype.lod_dist, archetype.bs_radius), (80.0, 2.5));
        assert_eq!(
            archetype.hd_texture_dist, 5.0,
            "the engine's init is stated"
        );
        assert_eq!(back.to_bytes().expect("saves again"), saved);
    }

    #[test]
    fn a_map_built_from_nothing_is_the_fixed_point_of_its_own_xml() {
        for saved in [
            hills().to_bytes().expect("saves"),
            CMapData::default().to_bytes().expect("an empty map saves"),
            CMapTypes::default()
                .to_bytes()
                .expect("an empty type file saves"),
        ] {
            let converge = || canonical::converge(&SchemaLibrary::default(), &saved);
            for converged in [converge(), with_public_names_only(converge)] {
                let converged = converged.expect("its XML compiles back");
                assert!(!converged.values_moved, "no value is lost on the way");
                assert_eq!(
                    converged.bytes, saved,
                    "the file its XML compiles to is the file itself"
                );
            }
        }
    }

    #[test]
    fn an_entity_built_into_an_empty_map_saves_what_building_the_map_whole_saves() {
        let empty = CMapData {
            name: HashedString::new("hills"),
            ..Default::default()
        };
        let mut reopened =
            CMapData::from_bytes(&empty.to_bytes().expect("saves")).expect("reopens");
        reopened.entities.push(bench(1));
        assert_eq!(
            reopened.to_bytes().expect("the first entity saves"),
            hills().to_bytes().expect("saves")
        );
    }

    /// The world classes with a member of `CEntityDef` that no record types.
    fn with_a_member_no_record_types() -> SchemaLibrary {
        let tint = r#"<u32 name="tintValue" init="0"/>"#;
        assert!(WORLD_TYPES.contains(tint));
        let psc = WORLD_TYPES.replace(
            tint,
            &format!(r#"{tint}<u32 name="aMemberNoRecordTypes"/>"#),
        );
        build_library([("wider.psc", psc.as_bytes())]).expect("the wider classes parse")
    }

    /// A map of `library`'s classes holding one bench, as `format`.
    fn wider_map(format: BinaryFormat, entities: usize) -> Vec<u8> {
        let library = with_a_member_no_record_types();
        let mut root = MetaInstance::new("CMapData");
        crate::meta::typed::conform(&library, &mut root).expect("conforms");
        let mut entity = MetaInstance::new("CEntityDef");
        crate::meta::typed::conform(&library, &mut entity).expect("conforms");
        // ⚠ By hash: `conform` spells a member as the index of names does, which has no name for this one.
        let mut put = |member: &str, value: Value| {
            let key = name_hash(member);
            let slot = entity.fields.iter_mut().find(|(n, _)| name_hash(n) == key);
            *slot.map(|(_, v)| v).expect("the class has the member") = value;
        };
        put("parentIndex", Value::Int(-1));
        put("aMemberNoRecordTypes", Value::UInt(9));
        root.set(
            "entities",
            Value::Array(
                (0..entities)
                    .map(|_| Value::Pointer(Some(Box::new(entity.clone()))))
                    .collect(),
            ),
        );
        let layout = canonical::layout_for(&library, &root, format, None).expect("lays out");
        write_meta_binary(format, Some(&layout), &root).expect("encodes")
    }

    #[test]
    fn an_entity_built_into_a_file_states_the_members_that_file_states() {
        for format in [BinaryFormat::Pso, BinaryFormat::Rbm] {
            let mut map = CMapData::from_bytes(&wider_map(format, 1)).expect("opens");
            map.entities.push(bench(2));
            let saved = map.to_bytes().expect("an entity built here saves");
            assert_eq!(detect_binary_format(&saved), Some(format));

            let root = tree(&saved);
            let entities = elements(&root, "entities");
            assert_eq!(entities.len(), 2);
            assert_eq!(
                stated(entities[1]),
                stated(entities[0]),
                "the entity built here states what the file's own class states"
            );
            let extra = name_hash("aMemberNoRecordTypes");
            let held = |inst: &MetaInstance| {
                inst.fields
                    .iter()
                    .find(|(n, _)| name_hash(n) == extra)
                    .map(|(_, v)| v.clone())
            };
            assert_eq!(held(entities[0]), Some(Value::UInt(9)));
            assert_eq!(
                held(entities[1]),
                Some(Value::UInt(0)),
                "at its empty value"
            );
            let back = CMapData::from_bytes(&saved).expect("reopens");
            assert_eq!(back.entities[1].guid, 2);
        }
    }

    #[test]
    fn an_entity_built_into_an_rbm_file_that_states_no_entity_class_is_refused_for_its_signature() {
        let mut map = CMapData::from_bytes(&wider_map(BinaryFormat::Rbm, 0)).expect("opens");
        map.entities.push(bench(1));
        let refused = map
            .to_bytes()
            .expect_err("nothing states the signature of the class");
        let why = refused.to_string();
        assert!(
            why.contains("signature") && why.contains("CEntityDef"),
            "{why}"
        );

        // The same file as PSO states no signature, so the class is laid out as it is declared here.
        let mut map = CMapData::from_bytes(&wider_map(BinaryFormat::Pso, 0)).expect("opens");
        map.entities.push(bench(1));
        let saved = map.to_bytes().expect("a PSO takes the declared class");
        assert_eq!(
            CMapData::from_bytes(&saved)
                .expect("reopens")
                .entities
                .len(),
            1
        );
    }

    /// The file `xml` compiles to over `base`, typed as a project's source is: by the base's own table and no snapshot.
    fn compiled(base: &[u8], xml: &str) -> crate::meta::error::Result<Vec<u8>> {
        let empty = SchemaLibrary::default();
        let layout = MetaDocument::parse(&empty, base.to_vec())
            .expect("the base reads")
            .layout
            .expect("a binary document has a layout");
        let format = detect_binary_format(base).expect("the base is binary");
        let typed = canonical::typed_by(&empty, Some(&layout));
        canonical::import(&typed, xml.as_bytes(), format, None)
    }

    #[test]
    fn an_xml_that_adds_the_first_entity_compiles_over_a_pso_base_and_never_over_an_rbm_one() {
        let saved = hills().to_bytes().expect("saves");
        let document = MetaDocument::parse(&SchemaLibrary::default(), saved).expect("reads");
        let xml = canonical::export(
            &document.instance,
            document.layout.as_ref().expect("a layout"),
        )
        .expect("exports");

        let over_pso = compiled(&wider_map(BinaryFormat::Pso, 0), &xml)
            .expect("a PSO base takes the class as it is declared here");
        assert_eq!(
            CMapData::from_bytes(&over_pso)
                .expect("reopens")
                .entities
                .len(),
            1
        );
        // ⚠ An RBM class laid out from the declaration would be written with a zero signature, which no shipped file carries.
        compiled(&wider_map(BinaryFormat::Rbm, 0), &xml)
            .expect_err("an RBM base that states no entity class has no signature for one");
    }

    #[test]
    fn an_entity_built_here_that_is_its_own_parent_is_refused_and_not_saved() {
        let mut map = hills();
        map.entities[0].parent_index = 0;
        let refused = map.to_bytes().expect_err("an index the engine trusts");
        assert!(matches!(refused, crate::Error::Refused { .. }), "{refused}");
        assert!(
            refused.to_string().contains("its own LOD parent"),
            "{refused}"
        );
    }

    #[test]
    fn a_world_save_leaves_its_types_to_no_later_save() {
        let none_left = || super::super::SAVING.with(|saving| saving.borrow().is_none());
        hills().to_bytes().expect("saves");
        assert!(none_left(), "after a save that wrote");
        let mut map = CMapData::from_bytes(&wider_map(BinaryFormat::Rbm, 0)).expect("opens");
        map.entities.push(bench(1));
        map.to_bytes().expect_err("no signature to write");
        assert!(none_left(), "after a save that was refused");
    }

    #[test]
    fn a_member_set_on_an_entity_built_into_a_file_whose_class_lacks_it_is_refused() {
        let tint = r#"<u32 name="tintValue" init="0"/>"#;
        assert!(WORLD_TYPES.contains(tint));
        let narrower = build_library([("narrower.psc", WORLD_TYPES.replace(tint, "").as_bytes())])
            .expect("the narrower classes parse");
        let mut root = MetaInstance::new("CMapData");
        crate::meta::typed::conform(&narrower, &mut root).expect("conforms");
        let mut held = MetaInstance::new("CEntityDef");
        crate::meta::typed::conform(&narrower, &mut held).expect("conforms");
        let parent = name_hash("parentIndex");
        for (name, value) in &mut held.fields {
            if name_hash(name) == parent {
                *value = Value::Int(-1);
            }
        }
        root.set(
            "entities",
            Value::Array(vec![Value::Pointer(Some(Box::new(held)))]),
        );
        let layout =
            canonical::layout_for(&narrower, &root, BinaryFormat::Pso, None).expect("lays out");
        let file = write_meta_binary(BinaryFormat::Pso, Some(&layout), &root).expect("encodes");

        let mut map = CMapData::from_bytes(&file).expect("opens");
        map.entities.push(bench(2));
        map.to_bytes()
            .expect("a tint at the engine's init needs no member to hold it");
        map.entities[1].tint_value = 5;
        let refused = map
            .to_bytes()
            .expect_err("the file's own class has no tint to hold 5");
        let why = refused.to_string();
        assert!(
            why.contains("tintValue") && why.contains("no such member"),
            "{why}"
        );
    }

    #[test]
    fn a_copy_whose_member_the_declared_class_lacks_is_refused_rather_than_dropped() {
        let donor = CMapData::from_bytes(&wider_map(BinaryFormat::Pso, 1)).expect("opens");
        let mut map = hills();
        map.entities.push(donor.entities[0].clone());
        let refused = map.to_bytes().expect_err("the copy holds a member to drop");
        let why = refused.to_string();
        assert!(
            why.contains("would drop") && why.contains("CEntityDef"),
            "{why}"
        );
    }
}
