//! Asking a mount built by hand where something is: a pasted place, a hash in each spelling, a model and a map by name, a
//! model's placements read from the maps that place it, and each copied as a script takes it.
//!
//! ⚠ Every expected value is computed from the fixture's own lists: a hash by the hash function, a spelling by formatting it, a
//! heading by the function a script's heading is read with, a height from the bound the fixture built.

mod world_fixture;

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use glam::{Quat, Vec3};
use or_depends::find::{
    find, models_asked, names_of, place_maps, placements_of, Asked, Budget, Detail, Found,
    GroundFound, Install, Named, Rank, SearchNames, Unread,
};
use or_depends::world_index::Archive;
use or_depends::{
    Catalogue, Kind, MountedWorld, Placed as PlacedAs, PlacementIndex, WorldCollision, WorldIndex,
    WorldSource,
};
use or_io::NameIndex;
use or_maps::edit::Pose;
use or_maps::place::{read_pasted, Pasted, PastedPlace};
use or_maps::script_text::{heading_of, ScriptForms};
use or_title::vfs::EntryFacts;
use world_fixture::{
    cube, hash, level_square, manifest_bytes, mesh_bound_bytes, models_held_to_their_maps, mount,
    placing_map_bytes, types_bytes, write_archive, MapSpec, ModelsHeld, Placed, Span,
};

/// More than every map of a fixture comes to.
const ROOMY: Budget = Budget {
    placements: usize::MAX,
    bytes: u64::MAX,
};

/// Every index a question is answered from, built from one source.
struct Built {
    world: WorldIndex,
    catalogue: Catalogue,
    names: NameIndex,
    search: SearchNames,
    placements: PlacementIndex,
}

impl Built {
    fn of(source: &dyn WorldSource) -> Self {
        let (world, _) = WorldIndex::build(source, None);
        let (catalogue, names) = names_of(source);
        let search = SearchNames::new(&world, &names);
        let mut placements = PlacementIndex::new();
        assert_eq!(place_maps(&world, source, &mut placements), 0);
        Self {
            world,
            catalogue,
            names,
            search,
            placements,
        }
    }

    fn install(&self) -> Install<'_> {
        Install {
            world: &self.world,
            catalogue: &self.catalogue,
            names: &self.names,
            search: &self.search,
            placements: &self.placements,
        }
    }

    fn find(&self, text: &str, near: Option<Vec3>, limit: usize) -> Found {
        let asked = Asked { near, limit };
        find(text, &self.install(), &asked, None).expect("the text is read")
    }
}

/// A source that records each file opened whole, by where the mount lists it.
struct Opened<'a> {
    source: &'a dyn WorldSource,
    opened: RefCell<Vec<String>>,
}

impl WorldSource for Opened<'_> {
    fn identity(&self) -> String {
        self.source.identity()
    }

    fn archives(&self) -> Vec<Archive> {
        self.source.archives()
    }

    fn entries(&self, archive: usize, visit: &mut dyn FnMut(&EntryFacts)) {
        self.source.entries(archive, visit);
    }

    fn open(&self, archive: usize, logical: &str) -> Option<Vec<u8>> {
        self.opened.borrow_mut().push(logical.to_string());
        self.source.open(archive, logical)
    }

    fn open_head(&self, archive: usize, logical: &str, limit: usize) -> Option<Vec<u8>> {
        self.source.open_head(archive, logical, limit)
    }

    fn patch_of(&self, archive: usize) -> Option<String> {
        self.source.patch_of(archive)
    }
}

const TOWN: [f32; 3] = [100.0, 100.0, 0.0];
const DOCK: [f32; 3] = [400.0, 100.0, 0.0];
const HILL: [f32; 3] = [100.0, 400.0, 0.0];

/// The height of the town's floor as the fixture asks for it; the file's own is read back from the bound.
const FLOOR: f32 = 10.0;

/// Every archetype the type file declares. `prop_nameless` has no model file, so no file of the install states its name.
const MODELS: &[&str] = &[
    "bench",
    "bench_press",
    "prop_bench",
    "park_bench_long",
    "workbench",
    "prop_lamp",
    "prop_unplaced",
    "prop_nameless",
];

/// What each map places, in the order of its entities.
fn placed() -> Vec<(&'static str, [f32; 3], Vec<Placed>)> {
    vec![
        (
            "m_town",
            TOWN,
            vec![
                Placed::new("prop_lamp", [120.0, 120.0, 10.0], 0.0, 10),
                Placed::new("prop_bench", [110.0, 110.0, 10.0], 90.0, 11),
                Placed::new("prop_nameless", [130.0, 130.0, 10.0], 0.0, 12),
            ],
        ),
        (
            "m_dock",
            DOCK,
            vec![
                Placed::new("prop_bench", [410.0, 110.0, 5.0], 45.0, 21),
                Placed::new("workbench", [420.0, 120.0, 5.0], 0.0, 22),
                // Two of one model at one place are two entities.
                Placed::new("prop_bench", [410.0, 110.0, 5.0], 135.0, 23),
            ],
        ),
        (
            "m_hill",
            HILL,
            vec![
                // Another model at the very place of the bench, before it: a placement is paired with an entity of its
                // own model.
                Placed::new("prop_lamp", [150.0, 450.0, 30.0], 180.0, 30),
                Placed::new("prop_bench", [150.0, 450.0, 30.0], 270.0, 31),
            ],
        ),
    ]
}

/// The archive a mount is not made without, holding a manifest that names nothing.
fn write_common(root: &Path) {
    let manifest = manifest_bytes(&[], &[]);
    write_archive(root, "common.rpf", &[("data/_manifest.ymf", manifest)]);
}

/// An install of three maps a hundred metres square, a type file, a model file for each archetype but one, and a floor under
/// the town.
fn install() -> (tempfile::TempDir, PathBuf) {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let root = scratch.path().join("install");
    std::fs::create_dir(&root).expect("mkdir");
    let unit: Span = cube([-1.0; 3], 2.0);
    let declared: Vec<(&str, Span, f32)> = MODELS.iter().map(|name| (*name, unit, 80.0)).collect();
    let mut files: Vec<(String, Vec<u8>)> = vec![
        ("types/props.ytyp".into(), types_bytes("props", &declared)),
        (
            "levels/m_town_0.ybn".into(),
            mesh_bound_bytes(level_square([TOWN[0], TOWN[1], FLOOR], 100.0, 0x21)),
        ),
    ];
    for (map, corner, entities) in placed() {
        let spec = MapSpec::around(cube(corner, 100.0));
        files.push((
            format!("levels/{map}.ymap"),
            placing_map_bytes(map, &spec, &entities),
        ));
    }
    for model in MODELS.iter().filter(|model| **model != "prop_nameless") {
        files.push((format!("models/{model}.ydr"), b"a model".to_vec()));
    }
    let files: Vec<(&str, Vec<u8>)> = (files.iter())
        .map(|(path, bytes)| (path.as_str(), bytes.clone()))
        .collect();
    write_archive(&root, "x64a.rpf", &files);
    write_common(&root);
    (scratch, root)
}

/// The heading a script reads of an entity the fixture turned `degrees` about the vertical axis.
fn heading(degrees: f32) -> f32 {
    heading_of(&Pose {
        position: Vec3::ZERO,
        rotation: Quat::from_rotation_z(degrees.to_radians()),
        scale_xy: 1.0,
        scale_z: 1.0,
    })
}

fn names(found: &Found) -> Vec<(&str, Rank)> {
    (found.models.iter())
        .map(|model| (model.name.as_deref().unwrap_or(""), model.rank))
        .collect()
}

#[test]
fn a_pasted_position_is_a_place_with_the_map_that_holds_it_and_its_ground() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let built = Built::of(&source);
    let asked = Asked {
        near: None,
        limit: 8,
    };
    let floor = level_square([TOWN[0], TOWN[1], FLOOR], 100.0, 0x21);
    let height = floor.vertex_position(0).expect("a vertex")[2];

    // Three numbers as a script writes them, with a heading: the map at that height, and the ground under it.
    let mut collision = WorldCollision::new(&built.world, &source, u64::MAX);
    let text = format!("vector4(150.0, 150.0, {}, 90.0)", height + 0.5);
    let found = find(&text, &built.install(), &asked, Some(&mut collision)).expect("a place");
    assert!(found.hash.is_none() && found.models.is_empty() && found.maps.is_empty());
    let place = found.place.expect("the text reads as a place");
    assert_eq!(
        (place.x, place.y, place.z),
        (150.0, 150.0, Some(height + 0.5))
    );
    assert_eq!(place.heading, Some(90.0));
    let held: Vec<&str> = place.maps.iter().map(|map| map.name.as_str()).collect();
    assert_eq!(held, ["m_town"]);
    assert_eq!(
        place.maps[0].logical,
        built.world.maps_at(Vec3::new(150.0, 150.0, 5.0))[0].logical
    );
    assert_eq!(place.maps_held, 1);
    // The bounds are those whose box holds the place at its height, as the index answers them.
    let at = Vec3::new(150.0, 150.0, height + 0.5);
    let bounds: Vec<&str> = place.bounds.iter().map(|b| b.name.as_str()).collect();
    let boxed = built.world.bounds_at(at);
    let boxed: Vec<&str> = boxed
        .iter()
        .map(|found| found.bound.name.as_str())
        .collect();
    assert_eq!(bounds, boxed);
    assert_eq!(
        place.ground,
        GroundFound::At {
            height,
            bound: Some("m_town_0".to_string())
        }
    );
    assert_eq!(
        place.position(),
        Some(Vec3::new(150.0, 150.0, height + 0.5))
    );

    // A height is asked for the ground at it and not for the highest of its column: under the floor by more than the rise
    // allowed there is none, and the place stays where it was typed.
    let under = height - 5.0 * or_depends::find::RISE;
    let text = format!("150, 150, {under}");
    let found = find(&text, &built.install(), &asked, Some(&mut collision)).expect("a place");
    let place = found.place.expect("a place");
    assert_eq!(place.ground, GroundFound::Nothing);
    assert_eq!(place.position(), Some(Vec3::new(150.0, 150.0, under)));
    // And for the maps that hold it at that height: above the box the town streams from, none does.
    let town = built.world.map(hash("m_town")).expect("the map");
    let above = town.streaming.expect("a streaming box").1.z + 1.0;
    let place = built.find(&format!("150, 150, {above}"), None, 8).place;
    let place = place.expect("a place");
    assert_eq!((place.maps_held, place.maps.len()), (0, 0));
    assert_eq!(built.world.maps_in_column(150.0, 150.0).len(), 1);

    // Two numbers have no height, so the place goes to the ground of its column.
    let found = find("150 150", &built.install(), &asked, Some(&mut collision)).expect("a place");
    let place = found.place.expect("two numbers are a place");
    assert_eq!(place.z, None);
    assert_eq!(place.position(), Some(Vec3::new(150.0, 150.0, height)));
    assert_eq!(place.maps_held, 1);
    let bounds: Vec<&str> = place.bounds.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(bounds, ["m_town_0"]);

    // With no collision the ground is not asked, and two numbers have nowhere to go.
    let place = built.find("150, 150", None, 8).place.expect("a place");
    assert_eq!(place.ground, GroundFound::Unasked);
    assert_eq!(place.position(), None);

    // Outside every map: a place all the same, which names none, over no ground.
    let found = find(
        "-5000, -5000, 0",
        &built.install(),
        &asked,
        Some(&mut collision),
    )
    .expect("a place");
    let place = found.place.expect("a place");
    assert!(place.maps.is_empty() && place.bounds.is_empty());
    assert_eq!(place.ground, GroundFound::Nothing);
}

#[test]
fn a_hash_in_each_of_its_three_spellings_finds_one_model_and_says_its_name() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let built = Built::of(&source);
    // ⚠ A hash under 2^31 is spelled alike signed and unsigned, so a model whose three spellings differ is asked for.
    let wide: Vec<&str> = (MODELS.iter().copied())
        .filter(|model| *model != "prop_nameless" && hash(model) > i32::MAX as u32)
        .collect();
    assert!(
        !wide.is_empty(),
        "no model of the fixture has a hash with three spellings"
    );
    for model in wide {
        let wanted = hash(model);
        let spellings = [
            format!("0x{wanted:08X}"),
            wanted.to_string(),
            (wanted as i32).to_string(),
        ];
        assert_ne!(spellings[1], spellings[2]);
        for spelled in &spellings {
            let found = built.find(spelled, None, 8);
            assert!(found.place.is_none() && found.maps.is_empty(), "{spelled}");
            let said = found.hash.as_ref().expect("the text reads as a hash");
            assert_eq!(said.hash, wanted);
            assert_eq!(said.spellings.name.as_deref(), Some(model), "{spelled}");
            assert_eq!(
                [
                    &said.spellings.hexadecimal,
                    &said.spellings.unsigned,
                    &said.spellings.signed
                ],
                [&spellings[0], &spellings[1], &spellings[2]]
            );
            let file = built.world.type_file(hash("props")).expect("the type file");
            assert!(matches!(
                &said.names[0],
                Named::Archetype { type_file: Some(declares) }
                    if declares.name == "props" && declares.logical == file.logical
            ));
            let model_file = built
                .catalogue
                .find(wanted, Kind::Drawable)
                .expect("a model file");
            assert!(said.names.contains(&Named::File {
                kind: Kind::Drawable,
                logical: model_file.logical.clone()
            }));
            assert_eq!(names(&found), [(model, Rank::Exact)], "{spelled}");
            assert_eq!(found.models[0].hash, wanted);
        }
    }

    // A hash no file names is an archetype all the same, with no name; a map's is a map; one that names nothing says nothing.
    let nameless = built.find(&format!("0x{:08X}", hash("prop_nameless")), None, 8);
    let said = nameless.hash.expect("a hash");
    assert_eq!(said.spellings.name, None);
    assert!(matches!(said.names[..], [Named::Archetype { .. }]));
    assert_eq!(nameless.models[0].name, None);
    assert_eq!(nameless.models[0].placements, 1);
    let map = built.find(&hash("m_dock").to_string(), None, 8);
    let dock = built.world.map(hash("m_dock")).expect("the map");
    assert_eq!(
        map.hash.expect("a hash").names,
        [Named::Map {
            logical: dock.logical.clone()
        }]
    );
    assert_eq!(map.maps[0].name, "m_dock");
    assert!(map.models.is_empty());
    // The hash of a file's whole name, which an archive lists it by, names that file; the hash of its stem is the model's.
    for (leaf, kind) in [
        ("prop_bench.ydr", Kind::Drawable),
        ("M_Dock.ymap", Kind::MapData),
        ("props.ytyp", Kind::MapTypes),
    ] {
        let (stem, _) = leaf.rsplit_once('.').expect("a leaf");
        let file = built.catalogue.find_named(&stem.to_ascii_lowercase(), kind);
        let logical = file.expect("a file of the fixture").logical.clone();
        let whole = built.find(&hash(leaf).to_string(), None, 8);
        let said = whole.hash.expect("a hash");
        let lower = leaf.to_ascii_lowercase();
        assert_eq!(said.spellings.name.as_deref(), Some(lower.as_str()));
        assert_eq!(said.names, [Named::File { kind, logical }], "{leaf}");
        assert!(whole.models.is_empty() && whole.maps.is_empty(), "{leaf}");
    }
    let nothing = built.find(&format!("0x{:08X}", hash("no_such_thing")), None, 8);
    assert!(nothing.hash.expect("a hash").names.is_empty());
    assert!(nothing.models.is_empty() && nothing.maps.is_empty());
}

#[test]
fn a_models_exact_name_ranks_first_then_a_prefix_then_a_part_after_an_underscore_then_contained() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let built = Built::of(&source);

    let found = built.find("Bench", None, 8);
    assert_eq!(
        names(&found),
        [
            ("bench", Rank::Exact),
            ("bench_press", Rank::Prefix),
            // By the part that answers: `bench` before `bench_long`.
            ("prop_bench", Rank::PartPrefix),
            ("park_bench_long", Rank::PartPrefix),
            ("workbench", Rank::Contained),
        ]
    );
    assert!(!found.more_models && found.place.is_none() && found.hash.is_none());
    // Text that neither begins nor ends a name or a part of one is contained all the same.
    assert_eq!(
        names(&built.find("orkb", None, 8)),
        [("workbench", Rank::Contained)]
    );
    // Each says the type file that declares it and how many placements the index holds.
    let file = built.world.type_file(hash("props")).expect("the type file");
    for model in &found.models {
        let declares = model.type_file.as_ref().expect("a type file");
        assert_eq!(
            (&declares.name, &declares.logical),
            (&file.name, &file.logical)
        );
        let name = model.name.as_deref().expect("a name");
        let count = placed()
            .iter()
            .flat_map(|(_, _, entities)| entities)
            .filter(|entity| entity.archetype == name)
            .count();
        assert_eq!(model.placements, count, "{name}");
        assert_eq!(model.hash, hash(name));
    }
    assert_eq!(found.models[2].placements, 4);

    // An index that states a name in capitals, as a name table may, names the model in lower case, which is what its hash
    // folds it to: by a part of it and by its hash alike.
    let mut capitals = NameIndex::new("test");
    capitals.insert("Prop_LAMP");
    assert_eq!(
        capitals
            .resolve(hash("prop_lamp"))
            .map(|stated| stated.name),
        Some("Prop_LAMP")
    );
    let search = SearchNames::new(&built.world, &capitals);
    let install = Install {
        names: &capitals,
        search: &search,
        ..built.install()
    };
    let asked = Asked {
        near: None,
        limit: 8,
    };
    let by_part = find("LAMP", &install, &asked, None).expect("text");
    assert_eq!(names(&by_part), [("prop_lamp", Rank::PartPrefix)]);
    let by_hash = find(&hash("prop_lamp").to_string(), &install, &asked, None).expect("a hash");
    assert_eq!(names(&by_hash), [("prop_lamp", Rank::Exact)]);
    let said = by_hash.hash.expect("a hash");
    assert_eq!(said.spellings.name.as_deref(), Some("prop_lamp"));

    // A name no file states is found typed in full, and by no part of it.
    assert_eq!(built.search.unnamed(), 1);
    assert_eq!(built.search.models(), MODELS.len() - 1);
    let whole = built.find("prop_nameless", None, 8);
    assert_eq!(names(&whole), [("prop_nameless", Rank::Exact)]);
    assert!(built.find("nameless", None, 8).models.is_empty());
}

#[test]
fn a_model_placed_in_three_maps_lists_its_placements_nearest_the_view_first() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let mounted = MountedWorld::new(&game, &root);
    let built = Built::of(&mounted);
    let source = Opened {
        source: &mounted,
        opened: RefCell::new(Vec::new()),
    };
    let model = hash("prop_bench");
    let view = Vec3::new(400.0, 100.0, 5.0);

    // The fixture's own entities of the model, nearest the view first, with their place in their map.
    let mut expected: Vec<(&str, usize, Placed)> = Vec::new();
    for (map, _, entities) in placed() {
        for (index, entity) in entities.into_iter().enumerate() {
            if entity.archetype == "prop_bench" {
                expected.push((map, index, entity));
            }
        }
    }
    expected.sort_by(|a, b| {
        let distance = |entity: &Placed| Vec3::from(entity.position).distance(view);
        distance(&a.2).total_cmp(&distance(&b.2))
    });
    let maps: Vec<&str> = expected.iter().map(|(map, ..)| *map).collect();
    assert_eq!(maps, ["m_dock", "m_dock", "m_town", "m_hill"]);

    let row = &built.find("prop_bench", Some(view), 8).models[0];
    assert_eq!(row.placements, expected.len());
    let nearest = row.nearest.expect("a nearest placement");
    assert_eq!(nearest.position, Vec3::from(expected[0].2.position));
    assert_eq!(nearest.map, hash("m_dock"));
    assert_eq!(nearest.distance, nearest.position.distance(view));
    assert!(built.find("prop_bench", None, 8).models[0]
        .nearest
        .is_none());

    let found = placements_of(model, &built.install(), &source, Some(view), &ROOMY);
    assert_eq!(found.known, expected.len());
    assert_eq!(found.listed.len(), expected.len());
    for (listed, (map, index, entity)) in found.listed.iter().zip(&expected) {
        let file = built.world.map(hash(map)).expect("the map");
        assert_eq!(listed.map, hash(map));
        assert_eq!(listed.map_name.as_deref(), Some(*map));
        assert_eq!(listed.logical.as_deref(), Some(file.logical.as_str()));
        assert_eq!(listed.placed, PlacedAs::Entity);
        let position = Vec3::from(entity.position);
        assert_eq!(listed.position, Some(position));
        assert_eq!(listed.distance, Some(position.distance(view)));
        let Detail::Entity {
            index: read,
            guid,
            heading: read_heading,
        } = listed.detail
        else {
            panic!("{map} entity {index} was not read: {:?}", listed.detail);
        };
        assert_eq!((read, guid), (*index, entity.guid), "{map}");
        let turn = heading(entity.heading);
        assert!(
            (read_heading - turn).abs() < 1e-3,
            "{read_heading} is not {turn}"
        );
    }
    // The two of one place are the two entities of the map, each once.
    let (first, second) = (&found.listed[0], &found.listed[1]);
    assert_eq!(first.position, second.position);
    assert_ne!(first.detail, second.detail);

    // Each map that places the model was opened once, and the fixture's other files not at all.
    let mut opened = source.opened.borrow().clone();
    opened.sort();
    let mut wanted: Vec<String> = ["m_dock", "m_hill", "m_town"]
        .iter()
        .map(|map| built.world.map(hash(map)).expect("the map").logical.clone())
        .collect();
    wanted.sort();
    assert_eq!(opened, wanted);
    assert_eq!(found.maps_read, 3);
    let sizes = wanted.iter().map(|logical| {
        let archive = built.world.maps().iter().find(|m| &m.logical == logical);
        mounted
            .open(archive.expect("the map").archive, logical)
            .expect("the file")
            .len() as u64
    });
    assert_eq!(found.bytes_read, sizes.sum::<u64>());

    // With no view the placements come in the order the maps were folded in, which is the mount's.
    let unordered = placements_of(model, &built.install(), &source, None, &ROOMY);
    let order: Vec<Option<f32>> = unordered.listed.iter().map(|p| p.distance).collect();
    assert_eq!(order, [None; 4]);
    let index_order: Vec<Vec3> = built
        .placements
        .of(model)
        .iter()
        .map(|p| p.position)
        .collect();
    let listed: Vec<Vec3> = unordered.listed.iter().filter_map(|p| p.position).collect();
    assert_eq!(listed, index_order);
}

#[test]
fn a_models_placements_are_read_under_a_budget_and_a_model_nothing_places_says_so() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let mounted = MountedWorld::new(&game, &root);
    let built = Built::of(&mounted);
    let source = Opened {
        source: &mounted,
        opened: RefCell::new(Vec::new()),
    };
    let model = hash("prop_bench");
    let view = Vec3::new(400.0, 100.0, 5.0);

    // One byte of budget opens the nearest map and no other: the rest are listed where the index holds them, unread.
    let tight = Budget {
        placements: usize::MAX,
        bytes: 1,
    };
    let found = placements_of(model, &built.install(), &source, Some(view), &tight);
    assert_eq!(found.maps_read, 1);
    assert_eq!(source.opened.borrow().len(), 1);
    let read: Vec<bool> = (found.listed.iter())
        .map(|p| matches!(p.detail, Detail::Entity { .. }))
        .collect();
    assert_eq!(read, [true, true, false, false]);
    assert_eq!(found.listed[2].detail, Detail::Unread(Unread::Budget));
    assert_eq!(
        found.listed[2].position,
        Some(Vec3::new(110.0, 110.0, 10.0))
    );
    assert_eq!(found.listed[2].heading(), None);

    // No budget of bytes reads nothing, and a count lists the nearest only.
    source.opened.borrow_mut().clear();
    let none = Budget {
        placements: 3,
        bytes: 0,
    };
    let found = placements_of(model, &built.install(), &source, Some(view), &none);
    assert_eq!(
        (found.known, found.listed.len(), found.maps_read),
        (4, 3, 0)
    );
    assert!(source.opened.borrow().is_empty());
    assert_eq!(found.listed[2].map, hash("m_town"));

    // A model nothing places: the row says none, and there is nothing to list or to open.
    let row = &built.find("prop_unplaced", Some(view), 8).models[0];
    assert_eq!((row.placements, row.nearest), (0, None));
    let found = placements_of(
        hash("prop_unplaced"),
        &built.install(),
        &source,
        Some(view),
        &ROOMY,
    );
    assert_eq!(
        (found.known, found.listed.len(), found.maps_read),
        (0, 0, 0)
    );
    assert!(source.opened.borrow().is_empty());
}

#[test]
fn a_maps_name_finds_the_map_with_the_centre_of_its_extents() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let built = Built::of(&source);

    let found = built.find("m_dock", None, 8);
    assert!(found.models.is_empty() && found.place.is_none() && found.hash.is_none());
    let [map] = &found.maps[..] else {
        panic!("{:?}", found.maps);
    };
    let file = built.world.map(hash("m_dock")).expect("the map");
    assert_eq!(
        (map.name.as_str(), map.hash, map.rank),
        ("m_dock", hash("m_dock"), Rank::Exact)
    );
    assert_eq!(map.logical, file.logical);
    // The fixture's map states its entities in a box a hundred metres square from its corner.
    let corner = Vec3::from(DOCK);
    assert_eq!(map.extents, Some((corner, corner + 100.0)));
    assert_eq!(map.centre, Some(corner + 50.0));

    // A part of the name after an underscore finds it, and a prefix every map.
    let part = built.find("dock", None, 8);
    assert_eq!(part.maps.len(), 1);
    assert_eq!(
        (part.maps[0].name.as_str(), part.maps[0].rank),
        ("m_dock", Rank::PartPrefix)
    );
    let all = built.find("m_", None, 8);
    let all: Vec<&str> = all.maps.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(all, ["m_dock", "m_hill", "m_town"]);
}

#[test]
fn text_that_matches_nothing_returns_empty_groups() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let built = Built::of(&source);
    for text in ["no_such_thing", "", "   ", "zzz"] {
        let found = built.find(text, None, 8);
        assert!(found.place.is_none() && found.hash.is_none(), "{text:?}");
        assert!(found.models.is_empty() && found.maps.is_empty(), "{text:?}");
        assert!(!found.more_models && !found.more_maps, "{text:?}");
        assert!(models_asked(text, &built.world, &built.search, 8)
            .expect("text")
            .is_empty());
    }
    // What has the shape of a hash and is none is refused, and says why.
    let asked = Asked {
        near: None,
        limit: 8,
    };
    let refused = find("0x123456789", &built.install(), &asked, None).expect_err("too wide");
    assert!(refused.to_string().contains("0x123456789"), "{refused}");
    assert!(models_asked("0x123456789", &built.world, &built.search, 8).is_err());
}

#[test]
fn the_count_asked_for_is_the_most_of_each_group() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let built = Built::of(&source);

    // Five models answer `bench`; the best of them are kept, in order, and the rest are said to be there.
    let all = built.find("bench", None, 8);
    assert_eq!(all.models.len(), 5);
    for limit in 0..=5 {
        let found = built.find("bench", None, limit);
        assert_eq!(found.models, all.models[..limit], "{limit}");
        assert_eq!(found.more_models, limit < 5, "{limit}");
        let watched = models_asked("bench", &built.world, &built.search, limit).expect("text");
        let listed: Vec<u32> = found.models.iter().map(|model| model.hash).collect();
        assert_eq!(watched, listed, "{limit}");
    }
    // Three maps answer `m_`.
    for limit in 0..=3 {
        let found = built.find("m_", None, limit);
        assert_eq!(found.maps.len(), limit);
        assert_eq!(found.more_maps, limit < 3);
    }
    // A hash's one model and a map's one map are rows too.
    let model = format!("0x{:08X}", hash("prop_bench"));
    assert_eq!(built.find(&model, None, 1).models.len(), 1);
    assert!(built.find(&model, None, 0).models.is_empty());
    assert!(models_asked(&model, &built.world, &built.search, 0)
        .expect("a hash")
        .is_empty());
    assert!(built
        .find(&hash("m_dock").to_string(), None, 0)
        .maps
        .is_empty());
    // And the files named under a place, whose count is said whole.
    let place = built.find("150, 150", None, 0).place.expect("a place");
    assert!(place.maps.is_empty() && place.bounds.is_empty());
    assert_eq!((place.maps_held, place.bounds_held), (1, 1));
}

/// The check an install suite holds a real install to, run here on the mount built by hand: its own models found by name, and
/// each placement held to the entity of the map file it names.
#[test]
fn the_models_of_a_mount_are_found_by_name_and_each_placement_stands_in_the_map_it_names() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    // The models a file names and a map places, counted from the fixture's own lists, and their entities.
    let placed = placed();
    let entities = || placed.iter().flat_map(|(_, _, entities)| entities);
    let named =
        |model: &&str| *model != "prop_nameless" && entities().any(|e| e.archetype == **model);
    let models: Vec<&str> = MODELS
        .iter()
        .copied()
        .filter(|model| named(model))
        .collect();
    let stood = entities()
        .filter(|e| models.contains(&e.archetype.as_str()))
        .count();
    assert!(models.len() >= 3 && stood > models.len());

    let held = models_held_to_their_maps(&source, models.len());
    assert_eq!(
        held,
        ModelsHeld {
            named: models.len(),
            asked: models.len(),
            placements: stood,
            // Every entity of the fixture stands inside the box its map streams from.
            inside: stood,
        }
    );
}

/// Whether `text` reads back as a place within half of the last of `decimals` decimals of `position`, with `heading`.
fn reads_back(text: &str, position: Vec3, heading: Option<f32>, decimals: i32) {
    let Ok(Pasted::Place(PastedPlace {
        x,
        y,
        z: Some(z),
        heading: read,
    })) = read_pasted(text)
    else {
        panic!("{text:?} does not read as a place with a height");
    };
    let half = 0.5 * 10f32.powi(-decimals) + 1e-4;
    for (read, written) in [(x, position.x), (y, position.y), (z, position.z)] {
        assert!(
            (read - written).abs() <= half,
            "{text:?}: {read} is not {written}"
        );
    }
    match (read, heading) {
        (Some(read), Some(written)) => {
            assert!(
                (read - written).abs() <= half,
                "{text:?}: heading {read} is not {written}"
            )
        }
        (None, None) => {}
        other => panic!("{text:?}: the heading read and the one written are {other:?}"),
    }
}

#[test]
fn a_result_copied_for_a_script_reads_back_as_the_place_it_was_copied_from() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let built = Built::of(&source);
    let forms = ScriptForms::built_in().expect("the forms of the tree");
    assert_eq!(forms.forms.len(), 3);
    let view = Vec3::new(400.0, 100.0, 5.0);
    let found = placements_of(
        hash("prop_bench"),
        &built.install(),
        &source,
        Some(view),
        &ROOMY,
    );
    let unread = placements_of(
        hash("prop_bench"),
        &built.install(),
        &source,
        Some(view),
        &Budget {
            placements: usize::MAX,
            bytes: 0,
        },
    );
    let place = built.find("vector4(12.5, -340.25, 28, 271.5)", None, 8);
    let place = place.place.expect("a place");
    let flat = built.find("12.5, -340.25", None, 8).place.expect("a place");

    for form in &forms.forms {
        for decimals in [0usize, 2, 4] {
            // A placement: where it stands and the heading its map states.
            for placement in &found.listed {
                let text = placement.script(form, decimals).expect("written");
                let text = text.expect("an entity has a position");
                let position = placement.position.expect("a position");
                assert!(placement.heading().is_some());
                reads_back(&text, position, placement.heading(), decimals as i32);
            }
            // One whose map was not read has no heading to write.
            let text = unread.listed[0].script(form, decimals).expect("written");
            let position = unread.listed[0].position.expect("a position");
            reads_back(&text.expect("a position"), position, None, decimals as i32);

            // The place typed, with the heading typed; one of two numbers and no ground has nothing to copy.
            let text = place
                .script(form, decimals)
                .expect("written")
                .expect("a height");
            let typed = Vec3::new(12.5, -340.25, 28.0);
            reads_back(&text, typed, Some(271.5), decimals as i32);
            assert_eq!(flat.script(form, decimals).expect("nothing to write"), None);

            // The list of a model's placements holds each placement as the form writes it, in the list's order, under its name.
            let list = found.script("prop_bench", form, decimals).expect("written");
            let mut rest = list.as_str();
            for placement in &found.listed {
                let one = placement.script(form, decimals).expect("written");
                let one = one.expect("a position");
                let at = rest
                    .find(&one)
                    .unwrap_or_else(|| panic!("{list} lacks {one}"));
                rest = &rest[at + one.len()..];
            }
            assert_eq!(
                list.matches("\"prop_bench\"").count(),
                found.listed.len(),
                "{list}"
            );
        }
    }
}

/// An install of `models` archetypes, each with its model file and placed three times, over maps of ten entities.
fn sized(root: &Path, models: usize) {
    std::fs::create_dir(root).expect("mkdir");
    let name = |at: usize| format!("prop_{at:05}_crate");
    let unit: Span = cube([-1.0; 3], 2.0);
    let all: Vec<String> = (0..models).map(name).collect();
    let declared: Vec<(&str, Span, f32)> =
        all.iter().map(|name| (name.as_str(), unit, 80.0)).collect();
    let mut files: Vec<(String, Vec<u8>)> =
        vec![("types/props.ytyp".into(), types_bytes("props", &declared))];
    let entities: Vec<Placed> = (0..models * 3)
        .map(|at| {
            Placed::new(
                &all[at % models],
                [at as f32, (at % 7) as f32, 0.0],
                0.0,
                at as u32,
            )
        })
        .collect();
    for (at, entities) in entities.chunks(10).enumerate() {
        let spec = MapSpec::around(cube([at as f32 * 10.0, 0.0, 0.0], 10.0));
        let map = format!("m_{at:05}");
        files.push((
            format!("levels/{map}.ymap"),
            placing_map_bytes(&map, &spec, entities),
        ));
    }
    files.extend(
        all.iter()
            .map(|name| (format!("models/{name}.ydr"), b"a model".to_vec())),
    );
    let files: Vec<(&str, Vec<u8>)> = (files.iter())
        .map(|(path, bytes)| (path.as_str(), bytes.clone()))
        .collect();
    write_archive(root, "x64a.rpf", &files);
    write_common(root);
}

/// What five questions cost a mount of `models` models: the names compared, the placements visited, the map files opened to
/// list one model's placements, and the seconds the questions took.
fn cost(scratch: &Path, models: usize) -> (usize, usize, usize, f64) {
    let root = scratch.join(format!("install_{models}"));
    sized(&root, models);
    let game = mount(&root);
    let mounted = MountedWorld::new(&game, &root);
    let built = Built::of(&mounted);
    assert_eq!(built.search.models(), models);
    assert_eq!(built.placements.held(), models as u64 * 3);
    let source = Opened {
        source: &mounted,
        opened: RefCell::new(Vec::new()),
    };
    let view = Some(Vec3::ZERO);
    let started = std::time::Instant::now();
    let (mut names, mut placements) = (0, 0);
    let exact = "prop_00007_crate";
    let questions = [
        (exact, 1),
        ("prop_0000", 8),
        ("crate", 8),
        ("7_cr", 8),
        ("no_such_thing", 0),
    ];
    let maps = built.world.maps().len();
    // The names two binary searches compare, over the names and over their parts, of the models and of the maps, and the
    // rows each of the three passes may reach before it stops at one past the count asked for.
    let searched = 4 * (2 * models).ilog2() as usize + 4 * (8 + 2);
    for (text, rows) in questions {
        let found = built.find(text, view, 8);
        assert_eq!(found.models.len(), rows, "{text}");
        // ⚠ At most one pass over the names: a second would be as many names again.
        assert!(
            found.work.names <= models + maps + searched,
            "{text}: {} name(s) compared over {models} models and {maps} maps",
            found.work.names
        );
        // ⚠ The placements visited are those of the rows listed, three a model, whatever the mount holds.
        assert_eq!(found.work.placements, rows * 3, "{text}");
        names += found.work.names;
        placements += found.work.placements;
    }
    let by_hash = built.find(&format!("0x{:08X}", hash(exact)), view, 8);
    assert_eq!(
        (
            by_hash.models.len(),
            by_hash.work.names,
            by_hash.work.placements
        ),
        (1, 0, 3)
    );
    let listed = placements_of(hash(exact), &built.install(), &source, view, &ROOMY);
    assert_eq!((listed.known, listed.listed.len()), (3, 3));
    let seconds = started.elapsed().as_secs_f64();
    let opened = source.opened.borrow().len();
    (names, placements, opened, seconds)
}

/// The profile of a question over a mount of `n` names, counted by [`or_depends::find::Work`]: two binary searches of about
/// log2 n names each, for the name and its prefix and for a prefix of a part after an underscore, then at most one pass over
/// the n names for text only contained, which ends at the count asked for; once over the models and once over the maps. A
/// hash compares no name. The placements visited are those of the rows listed, and a model's placements open the maps that
/// place it and no other. So a mount four times the size costs under four times the names, where a question that looked at
/// every placement for every name would cost sixteen, and the placements visited and the maps opened do not grow at all. The
/// test prints what it measured at 200 and at 800 models.
#[test]
fn a_question_costs_no_more_than_the_names_of_a_mount_and_never_their_square() {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let (small, large) = (200, 800);
    let (names_small, placements_small, opened_small, seconds_small) = cost(scratch.path(), small);
    let (names_large, placements_large, opened_large, seconds_large) = cost(scratch.path(), large);
    eprintln!(
        "\n=== five questions and a model's placements: {small} models, {names_small} name(s) compared, \
         {placements_small} placement(s) visited, {opened_small} map(s) opened, {seconds_small:.6} s; {large} models, \
         {names_large} name(s) compared, {placements_large} placement(s) visited, {opened_large} map(s) opened, \
         {seconds_large:.6} s ==="
    );
    // What does not grow at all: the placements visited and the maps opened are the answer's, three of each.
    assert_eq!(placements_small, placements_large);
    assert_eq!((opened_small, opened_large), (3, 3));
    // What grows with the names: the two questions that pass over every name, and no faster than the names do.
    assert!(
        names_small >= 2 * small && names_large >= 2 * large,
        "{names_small} {names_large}"
    );
    let grew = names_large as f64 / names_small as f64;
    let size = large as f64 / small as f64;
    assert!(
        grew <= size,
        "the names compared grew {grew:.2} times over a mount {size} times the size"
    );
}
