//! A map and a type file made from nothing become sources of a pack and break none of its rules, an empty map takes its first
//! entity with no entity to copy, an entity that is no copy is the record a copy with the same fields would be, and the first
//! record an operation builds in an empty file is to the byte the one a copy of another file's gives.
//!
//! ⚠ The files are made here, by `or_world_gta5::create` and the crate's samples, so the clean checkout runs this. No schema snapshot
//! is installed in this process, which is what a user's machine has.

use std::path::{Path, PathBuf};

use glam::{Quat, Vec3};
use or_io::decl::File;
use or_io::meta::instance::HashedString;
use or_io::meta::typed::{CMapData, CMapTypes};
use or_io::meta::{detect_binary_format, BinaryFormat, MetaDocument, SchemaLibrary};
use or_maps::edit::{
    add_archetype, add_entity, add_new_archetype, add_new_entity, edit_map, edit_types,
    grow_extents, pose_of, read_archetype, read_entity,
};
use or_maps::facts::{cooked, Name};
use or_maps::import::{add_files, Incoming};
use or_maps::model::Maps;
use or_maps::operation::{apply, At, Operation};
use or_maps::sample::{map_file, types_file, Entity};
use or_maps::{check, inspect, Depth, Severity};
use or_world::transform::Pose;
use or_world_gta5::archetypes::ArchetypeIndex;
use or_world_gta5::create::{self, Placed};
use or_world_gta5::generate::{AssetKind, Authoring, DrawableFacts};
use or_world_gta5::map_types::MapTypes;

const CRATE: &str = "prop_crate_a";

/// The archetype of the crate, a cube two units on a side, as `or_world_gta5::create` makes it from nothing.
fn crate_archetype() -> or_io::meta::typed::CBaseArchetypeDefAny {
    create::archetype(
        &DrawableFacts {
            name: CRATE.into(),
            kind: AssetKind::Drawable,
            dictionary: None,
            bb_min: Vec3::splat(-1.0),
            bb_max: Vec3::splat(1.0),
            bs_centre: Vec3::ZERO,
            bs_radius: 3.0_f32.sqrt(),
        },
        &Authoring::default(),
    )
}

/// A type file made from nothing that declares the crate.
fn crate_types() -> Vec<u8> {
    let mut types = create::types("harbour_types");
    types.archetypes.push(crate_archetype());
    create::types_file(&types, BinaryFormat::Pso).expect("a type file made from nothing saves")
}

fn index_of(types: &[u8]) -> ArchetypeIndex {
    let tree = MetaDocument::parse(&SchemaLibrary::default(), types.to_vec())
        .expect("the type file reads")
        .instance;
    let mut index = ArchetypeIndex::new();
    index.add(MapTypes::new(&tree).expect("a CMapTypes"));
    index
}

fn crate_at(position: Vec3) -> Placed {
    Placed {
        flags: 32,
        ..Placed::at(HashedString::new(CRATE), position, 100.0)
    }
}

/// A map made from nothing, called `harbour`, that places `entities`.
fn harbour(entities: &[Placed], types: &[u8]) -> Vec<u8> {
    let mut map = create::map("harbour");
    for placed in entities {
        create::place(&mut map, placed).expect("placed");
    }
    create::map_file(&map, &index_of(types), BinaryFormat::Pso)
        .expect("a map made from nothing saves")
}

/// A module holding `map` as `harbour.ymap` and `types` as `harbour_types.ytyp`; its folder and the folder of the map's source.
fn module(dir: &Path, map: &[u8], types: &[u8]) -> (PathBuf, PathBuf) {
    let folder = dir.join("maps");
    let added = add_files(
        &folder,
        &[
            Incoming {
                name: "harbour.ymap",
                bytes: map,
                from_game: false,
                virtual_path: None,
            },
            Incoming {
                name: "harbour_types.ytyp",
                bytes: types,
                from_game: false,
                virtual_path: None,
            },
        ],
        None,
    )
    .expect("the files are added");
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    (folder.clone(), folder.join("harbour.metadata.src"))
}

fn map_of(dir: &Path) -> CMapData {
    CMapData::from_bytes(&cooked(dir).expect("the source cooks")).expect("to a map")
}

fn vec3(v: or_io::decl::Vec3) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

/// The box and the stream distance of each archetype the pack in `folder` declares, by the hash of its name.
fn boxes(folder: &Path) -> std::collections::BTreeMap<u32, (Vec3, Vec3, f32)> {
    let model = Maps::read(folder).expect("the pack reads");
    let facts = inspect(folder, &model, Depth::Open);
    let mut boxes = std::collections::BTreeMap::new();
    for path in &model.maps[0].sources {
        let Some(types) = facts.source(path).and_then(|s| s.types()) else {
            continue;
        };
        for archetype in &types.archetypes {
            boxes.entry(archetype.name.hash).or_insert((
                Vec3::from(archetype.bb_min),
                Vec3::from(archetype.bb_max),
                archetype.lod_dist,
            ));
        }
    }
    boxes
}

#[test]
fn a_map_and_a_type_file_made_from_nothing_enter_a_pack_and_break_none_of_its_rules() {
    let scratch = tempfile::tempdir().unwrap();
    let types = crate_types();
    let map = harbour(
        &[
            crate_at(Vec3::new(2000.0, -3000.0, 50.0)),
            crate_at(Vec3::new(2004.0, -3000.0, 50.0)),
        ],
        &types,
    );
    let (folder, dir) = module(scratch.path(), &map, &types);

    assert_eq!(
        cooked(&dir).expect("the source cooks"),
        map,
        "the source cooks to the file it was made of"
    );
    let model = Maps::read(&folder).expect("the pack reads");
    let facts = inspect(&folder, &model, Depth::Open);
    let found = check(&model, &facts, None);
    let blocking: Vec<_> = found
        .iter()
        .filter(|f| f.severity != Severity::Info)
        .map(|f| (f.code, f.message.as_str()))
        .collect();
    assert_eq!(blocking, Vec::new(), "no rule says more than a note");
    // The archetype names a model the pack does not hold, which every rule run of a pack with no model says as a note.
    assert!(found.iter().any(|f| f.code == "archetype-without-model"));
}

#[test]
fn an_empty_map_made_from_nothing_takes_its_first_entity_with_none_to_copy() {
    let scratch = tempfile::tempdir().unwrap();
    let types = crate_types();
    let (folder, dir) = module(scratch.path(), &harbour(&[], &types), &types);
    assert!(map_of(&dir).entities.is_empty());

    let at = Vec3::new(2000.0, -3000.0, 50.0);
    let known = boxes(&folder);
    let mut index = usize::MAX;
    let edited = edit_map(&dir, |map| {
        index = add_new_entity(map, &crate_at(at))?;
        grow_extents(map, |hash| known.get(&hash).copied());
        Ok(())
    })
    .expect("the first entity is written to the source");
    assert!(edited.changed());
    assert_eq!(index, 0);

    let cooked_now = cooked(&dir).expect("the source cooks");
    assert_eq!(detect_binary_format(&cooked_now), Some(BinaryFormat::Pso));
    let map = map_of(&dir);
    assert_eq!(map.entities.len(), 1);
    let entity = &map.entities[0];
    assert!(entity.archetype_name == CRATE);
    assert_eq!(vec3(entity.position), at);
    assert_eq!(
        (entity.guid, entity.parent_index, entity.num_children),
        (1, -1, 0)
    );
    assert_eq!((entity.flags, entity.lod_dist), (32, 100.0));

    // The extents hold the crate and not the origin of the world, which extents grown from none would.
    assert_eq!(vec3(map.entities_extents_min), at - Vec3::ONE);
    assert_eq!(vec3(map.entities_extents_max), at + Vec3::ONE);
    assert_eq!(vec3(map.streaming_extents_min), at - Vec3::splat(101.0));
    assert_eq!(vec3(map.streaming_extents_max), at + Vec3::splat(101.0));

    // A second one is added the same way, and the XML as it was puts the map back as it was.
    edit_map(&dir, |map| {
        add_new_entity(map, &crate_at(at + Vec3::new(4.0, 0.0, 0.0))).map(drop)
    })
    .expect("the second entity is written");
    assert_eq!(map_of(&dir).entities.len(), 2);
    assert_eq!(map_of(&dir).entities[1].guid, 2);
    std::fs::write(&edited.file, &edited.before).unwrap();
    assert!(map_of(&dir).entities.is_empty());
}

/// The two sources of one map: one takes a copy, the other an entity that is no copy.
fn twice(scratch: &Path, map: &[u8], types: &[u8]) -> (PathBuf, PathBuf) {
    let (_, copied) = module(&scratch.join("copied"), map, types);
    let (_, built) = module(&scratch.join("built"), map, types);
    (copied, built)
}

#[test]
fn an_entity_that_is_no_copy_is_the_record_a_copy_with_the_same_fields_is() {
    let scratch = tempfile::tempdir().unwrap();
    let types = crate_types();
    let pose = Pose {
        position: Vec3::new(50.0, 60.0, 7.0),
        rotation: Quat::from_rotation_z(0.7),
        scale_xy: 2.0,
        scale_z: 1.5,
    };
    let donors = [
        // The crate's sample, an RBM file with a class of its own, and a map made from nothing, a PSO.
        (
            "sample",
            map_file(
                "harbour",
                &[Entity::at("prop_crate_b", [10.0, 20.0, 0.5], 1)],
                ([0.0; 3], [100.0; 3]),
            ),
        ),
        (
            "made",
            harbour(&[crate_at(Vec3::new(10.0, 20.0, 0.5))], &types),
        ),
    ];
    for (which, map) in donors {
        let (copied, built) = twice(&scratch.path().join(which), &map, &types);
        let donor = map_of(&copied).entities[0].clone();

        edit_map(&copied, |map| {
            add_entity(map, &donor, Name::from_text(CRATE), pose).map(drop)
        })
        .expect("the copy is written");
        edit_map(&built, |map| {
            let placed = Placed {
                archetype: HashedString::new(CRATE),
                position: pose.position,
                rotation: pose.rotation,
                scale_xy: pose.scale_xy,
                scale_z: pose.scale_z,
                lod_dist: donor.lod_dist,
                flags: donor.flags,
            };
            add_new_entity(map, &placed).map(drop)
        })
        .expect("the entity that is no copy is written");

        let (a, b) = (map_of(&copied), map_of(&built));
        assert_eq!(a.entities.len(), 2, "{which}");
        assert_eq!(
            a.entities[1], b.entities[1],
            "{which}: every member the record types"
        );
        assert_eq!(pose_of(&b.entities[1]).position, pose.position);
        assert_eq!(
            std::fs::read(copied.join("metadata.xml")).unwrap(),
            std::fs::read(built.join("metadata.xml")).unwrap(),
            "{which}: every member the source states"
        );
        assert_eq!(
            cooked(&copied).unwrap(),
            cooked(&built).unwrap(),
            "{which}: and every byte the source cooks to"
        );
    }
}

#[test]
fn a_position_that_is_no_number_is_refused_and_the_source_is_as_it_was() {
    let scratch = tempfile::tempdir().unwrap();
    let types = crate_types();
    let (_, dir) = module(scratch.path(), &harbour(&[], &types), &types);
    let before = std::fs::read(dir.join("metadata.xml")).unwrap();
    for position in [
        Vec3::new(f32::NAN, 0.0, 0.0),
        Vec3::new(0.0, f32::INFINITY, 0.0),
        Vec3::new(0.0, 0.0, f32::NEG_INFINITY),
    ] {
        let refused = edit_map(&dir, |map| {
            add_new_entity(map, &crate_at(position)).map(drop)
        })
        .expect_err("a position that is no number");
        assert!(refused.contains("not a finite number"), "{refused}");
    }
    assert_eq!(std::fs::read(dir.join("metadata.xml")).unwrap(), before);
    assert!(map_of(&dir).entities.is_empty());
}

#[test]
fn an_rbm_map_that_holds_no_entity_refuses_one_that_is_no_copy_and_names_the_signature() {
    let scratch = tempfile::tempdir().unwrap();
    let types = crate_types();
    let empty = map_file("harbour", &[], ([0.0; 3], [0.0; 3]));
    assert_eq!(detect_binary_format(&empty), Some(BinaryFormat::Rbm));
    let (_, dir) = module(scratch.path(), &empty, &types);
    let before = std::fs::read(dir.join("metadata.xml")).unwrap();

    let refused = edit_map(&dir, |map| {
        add_new_entity(map, &crate_at(Vec3::new(1.0, 2.0, 3.0))).map(drop)
    })
    .expect_err("nothing states the signature of the entity's class");
    assert!(
        refused.contains("signature") && refused.contains("CEntityDef"),
        "{refused}"
    );
    assert_eq!(std::fs::read(dir.join("metadata.xml")).unwrap(), before);
}

fn types_of(dir: &Path) -> CMapTypes {
    CMapTypes::from_bytes(&cooked(dir).expect("the source cooks")).expect("to a type file")
}

#[test]
fn an_empty_type_file_made_from_nothing_takes_its_first_archetype_with_none_to_copy() {
    let scratch = tempfile::tempdir().unwrap();
    let empty = create::types_file(&create::types("harbour_types"), BinaryFormat::Pso)
        .expect("an empty type file saves");
    let (folder, _) = module(scratch.path(), &harbour(&[], &crate_types()), &empty);
    let dir = folder.join("harbour_types.metadata.src");
    assert!(types_of(&dir).archetypes.is_empty());
    assert!(boxes(&folder).is_empty());

    let mut index = usize::MAX;
    let edited = edit_types(&dir, |types| {
        index = add_new_archetype(types, crate_archetype())?;
        Ok(())
    })
    .expect("the first archetype is written to the source");
    assert!(edited.changed());
    assert_eq!(index, 0);
    assert_eq!(
        detect_binary_format(&cooked(&dir).expect("cooks")),
        Some(BinaryFormat::Pso)
    );
    assert_eq!(
        cooked(&dir).expect("cooks"),
        crate_types(),
        "the file a type file made whole with the archetype is"
    );
    let declared = &types_of(&dir).archetypes[0];
    assert!(declared.name == CRATE && declared.asset_name == CRATE);
    let known = boxes(&folder);
    let hash = or_io::string_hash(CRATE.as_bytes());
    assert_eq!(
        known.get(&hash).map(|(lo, hi, _)| (*lo, *hi)),
        Some((Vec3::splat(-1.0), Vec3::splat(1.0))),
        "the pack finds the box an entity's extents are held to"
    );

    // The game resolves a name once, so a second archetype of the name is refused and the source is as it was.
    let before = std::fs::read(dir.join("metadata.xml")).unwrap();
    let refused = edit_types(&dir, |types| {
        add_new_archetype(types, crate_archetype()).map(drop)
    })
    .expect_err("the name is declared");
    assert!(refused.contains("already declares"), "{refused}");
    assert_eq!(std::fs::read(dir.join("metadata.xml")).unwrap(), before);
}

#[test]
fn an_rbm_type_file_that_holds_no_archetype_refuses_one_that_is_no_copy_and_names_the_signature() {
    let scratch = tempfile::tempdir().unwrap();
    let empty = types_file("harbour_types", &[]);
    assert_eq!(detect_binary_format(&empty), Some(BinaryFormat::Rbm));
    let (folder, _) = module(scratch.path(), &harbour(&[], &crate_types()), &empty);
    let dir = folder.join("harbour_types.metadata.src");
    let before = std::fs::read(dir.join("metadata.xml")).unwrap();

    let refused = edit_types(&dir, |types| {
        add_new_archetype(types, crate_archetype()).map(drop)
    })
    .expect_err("nothing states the signature of the archetype's class");
    assert!(
        refused.contains("signature") && refused.contains("CBaseArchetypeDef"),
        "{refused}"
    );
    assert_eq!(std::fs::read(dir.join("metadata.xml")).unwrap(), before);
}

/// A pack of an empty map and an empty type file made from nothing, beside a map that places the crate and the type file that
/// declares it, each made from nothing too.
fn pack_with_donors(dir: &Path) -> PathBuf {
    let types = crate_types();
    let mut yard = create::map("yard");
    create::place(&mut yard, &crate_at(Vec3::new(10.0, 20.0, 0.5))).expect("placed");
    let yard = create::map_file(&yard, &index_of(&types), BinaryFormat::Pso).expect("saves");
    let empty_types = create::types_file(&create::types("quay_types"), BinaryFormat::Pso)
        .expect("an empty type file saves");
    let folder = dir.join("maps");
    let files = [
        ("harbour.ymap", harbour(&[], &types)),
        ("yard.ymap", yard),
        ("harbour_types.ytyp", types),
        ("quay_types.ytyp", empty_types),
    ];
    let incoming: Vec<Incoming<'_>> = (files.iter())
        .map(|(name, bytes)| Incoming {
            name,
            bytes,
            from_game: false,
            virtual_path: None,
        })
        .collect();
    let added = add_files(&folder, &incoming, None).expect("the files are added");
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    folder
}

/// ⚠ An operation builds the first entity of a map and the first archetype of a type file and does not copy the donor, whose tree
/// was another file's. A file that states no class for a record takes a copy too, laid out as the class is declared, so on donors
/// made from nothing the two are one record: this fails when either path writes a member the other does not.
#[test]
fn the_first_record_an_operation_builds_is_to_the_byte_the_one_a_copy_of_another_files_gives() {
    const MAP: &str = "harbour.metadata.src";
    const YARD: &str = "yard.metadata.src";
    const TYPES: &str = "harbour_types.metadata.src";
    const QUAY: &str = "quay_types.metadata.src";
    let scratch = tempfile::tempdir().unwrap();
    let built = pack_with_donors(&scratch.path().join("built"));
    let copied = pack_with_donors(&scratch.path().join("copied"));
    let pose = Pose {
        position: Vec3::new(60.0, 70.0, 0.5),
        rotation: Quat::from_rotation_z(0.7),
        scale_xy: 2.0,
        scale_z: 1.5,
    };

    apply(
        &built,
        &Operation::AddEntity {
            source: MAP.into(),
            expects: None,
            archetype: CRATE.into(),
            pose: pose.into(),
            donor: Some(At {
                source: YARD.into(),
                index: 0,
            }),
        },
    )
    .unwrap_or_else(|e| panic!("the first entity, built here: {e}"));
    let donor = read_entity(&copied.join(YARD), 0).expect("the donor reads");
    let known = boxes(&copied);
    edit_map(&copied.join(MAP), |map| {
        add_entity(map, &donor, Name::from_text(CRATE), pose)?;
        grow_extents(map, |hash| known.get(&hash).copied());
        Ok(())
    })
    .expect("the first entity, as a copy of one of another file");

    apply(
        &built,
        &Operation::AddArchetype {
            source: QUAY.into(),
            expects: None,
            name: "prop_quay_crate".into(),
            asset: None,
            donor: Some(At {
                source: TYPES.into(),
                index: 0,
            }),
        },
    )
    .unwrap_or_else(|e| panic!("the first archetype, built here: {e}"));
    let donor = read_archetype(&copied.join(TYPES), 0).expect("the donor reads");
    edit_types(&copied.join(QUAY), |types| {
        add_archetype(types, Some(&donor), "prop_quay_crate", None).map(drop)
    })
    .expect("the first archetype, as a copy of one of another file");

    for source in [MAP, QUAY] {
        assert_eq!(
            std::fs::read(built.join(source).join("metadata.xml")).unwrap(),
            std::fs::read(copied.join(source).join("metadata.xml")).unwrap(),
            "{source}: every member the source states"
        );
        assert_eq!(
            cooked(&built.join(source)).unwrap(),
            cooked(&copied.join(source)).unwrap(),
            "{source}: and every byte the source cooks to"
        );
    }
    assert_eq!(map_of(&built.join(MAP)).entities.len(), 1);
    assert_eq!(types_of(&built.join(QUAY)).archetypes.len(), 1);
}
