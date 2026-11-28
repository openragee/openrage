//! The world index of a mount built by hand: what it names, what it answers at a position, and
//! what it reads again of an install it has kept.
//!
//! ⚠ Every expected value is computed from the fixture's own lists, and each test says how many
//! things it compared: an index that named nothing would agree with a mount that listed nothing.

#[path = "../../or-world-gta5/tests/world_fixture/mod.rs"]
mod world_fixture;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use glam::Vec3;
use or_depends::Resolver;
use or_title::MountedGame;
use or_world_gta5::world_index::{
    refresh, Archive, Kept, KeptError, KeptState, Stamp, RULES, VERSION,
};
use or_world_gta5::{
    ArchetypeAssetIndex, Catalogue, InstallTables, Kind, ModelContents, MountedWorld, Via,
    WorldIndex, WorldSource,
};
use world_fixture::{
    bound_bytes, cube, hash, held_to_its_mount, manifest_bytes, map_bytes, mount, mount_all,
    stated_by_the_files, types_bytes, write_archive, Counting, Held, MapSpec, Span, Stated,
};

/// One file of the fixture: where it sits in its archive, and what it is.
#[derive(Clone)]
enum What {
    Map(MapSpec),
    Types(Vec<(&'static str, Span, f32)>),
    Manifest,
    /// A manifest naming these bounds for these map groups, and none for an interior.
    Groups(Vec<(&'static str, Vec<&'static str>)>),
    Bound(Span),
    /// Bytes as they are, whatever the file's name says they are.
    Raw(Vec<u8>),
}

type Files = Vec<(&'static str, What)>;

/// The bounds the manifest names for a map group, one of them a file the install lacks.
const GROUP_BOUNDS: &[&str] = &["m_group_0", "hi@m_group_0", "m_group_9"];
const INTERIOR_BOUNDS: &[&str] = &["v_int_shop_0"];

fn north() -> Span {
    cube([100.0, 100.0, 0.0], 100.0)
}

fn group() -> Span {
    cube([1000.0, 0.0, 0.0], 100.0)
}

fn south() -> Span {
    cube([100.0, -300.0, 0.0], 100.0)
}

/// The three archives of the fixture, in mount order.
///
/// `x64b.rpf` holds a second `m_north.ymap`, which shadows `x64a.rpf`'s, a type file that
/// declares `prop_a` again, and an archive nested in it that holds a map and a bound.
fn archives() -> Vec<(&'static str, Files)> {
    let child = MapSpec {
        parent: "m_north",
        content_flags: 1,
        ..MapSpec::around(cube([120.0, 120.0, 0.0], 20.0))
    };
    let shadowed = MapSpec {
        content_flags: 2,
        ..MapSpec::around(cube([-900.0, -900.0, 0.0], 50.0))
    };
    let later = MapSpec {
        content_flags: 4,
        flags: 1,
        ..MapSpec::around(north())
    };
    let south = MapSpec {
        interiors: vec!["v_int_shop"],
        ..MapSpec::around(south())
    };
    let unit = cube([-1.0, -1.0, -1.0], 2.0);
    vec![
        (
            "common.rpf",
            vec![("data/levels/gta5/_manifest.ymf", What::Manifest)],
        ),
        (
            "x64a.rpf",
            vec![
                ("levels/m_north.ymap", What::Map(shadowed)),
                ("levels/m_north_child.ymap", What::Map(child)),
                ("levels/m_group.ymap", What::Map(MapSpec::around(group()))),
                (
                    "levels/m_north_0.ybn",
                    What::Bound(cube([110.0, 110.0, 0.0], 30.0)),
                ),
                (
                    "levels/hi@m_north_0.ybn",
                    What::Bound(cube([110.0, 110.0, 0.0], 30.0)),
                ),
                // Named after the map and lying outside its streaming extents.
                (
                    "levels/ma@m_north_1.ybn",
                    What::Bound(cube([500.0, 500.0, 0.0], 10.0)),
                ),
                (
                    "levels/m_group_0.ybn",
                    What::Bound(cube([1010.0, 10.0, 0.0], 30.0)),
                ),
                (
                    "levels/hi@m_group_0.ybn",
                    What::Bound(cube([1010.0, 10.0, 0.0], 30.0)),
                ),
                // Named after the group's map, which the manifest names other bounds for.
                (
                    "levels/m_group_7.ybn",
                    What::Bound(cube([1050.0, 50.0, 0.0], 30.0)),
                ),
                // An archetype's own collision, about its origin, twice: one stem no map could
                // name, and one that reads as `<map>_N` of a map the install lacks.
                ("props/prop_col.ybn", What::Bound(unit)),
                ("props/prop_bench_01.ybn", What::Bound(unit)),
                (
                    "types/types_a.ytyp",
                    What::Types(vec![("prop_a", unit, 100.0), ("prop_b", unit, 60.0)]),
                ),
            ],
        ),
        (
            "x64b.rpf",
            vec![
                ("patch/m_north.ymap", What::Map(later)),
                ("levels/pack.rpf/m_south.ymap", What::Map(south)),
                (
                    "levels/pack.rpf/v_int_shop_0.ybn",
                    What::Bound(cube([120.0, -280.0, 0.0], 20.0)),
                ),
                (
                    "types/types_b.ytyp",
                    What::Types(vec![
                        ("prop_a", cube([-2.0, -2.0, -2.0], 4.0), 250.0),
                        ("prop_c", unit, 30.0),
                    ]),
                ),
            ],
        ),
    ]
}

fn bytes_of(path: &str, what: &What) -> Vec<u8> {
    let stem = path.rsplit('/').next().expect("a leaf");
    let stem = stem.rsplit_once('.').expect("an extension").0;
    match what {
        What::Map(spec) => map_bytes(stem, spec),
        What::Types(archetypes) => types_bytes(stem, archetypes),
        What::Manifest => manifest_bytes(
            &[("m_group", GROUP_BOUNDS)],
            &[("v_int_shop", INTERIOR_BOUNDS)],
        ),
        What::Groups(groups) => {
            let groups: Vec<(&str, &[&str])> = (groups.iter())
                .map(|(name, bounds)| (*name, bounds.as_slice()))
                .collect();
            manifest_bytes(&groups, &[])
        }
        What::Bound(span) => bound_bytes(*span),
        What::Raw(bytes) => bytes.clone(),
    }
}

fn write(root: &Path, name: &str, files: &Files) {
    let files: Vec<(&str, Vec<u8>)> = (files.iter())
        .map(|(path, what)| (*path, bytes_of(path, what)))
        .collect();
    write_archive(root, name, &files);
}

/// An install holding `archives`, and the folder's guard.
fn install_of(archives: &[(&'static str, Files)]) -> (tempfile::TempDir, PathBuf) {
    let scratch = tempfile::Builder::new()
        .prefix("or-depends-world-")
        .tempdir()
        .expect("mkdir");
    let root = scratch.path().join("install");
    std::fs::create_dir(&root).expect("mkdir");
    for (name, files) in archives {
        write(&root, name, files);
    }
    (scratch, root)
}

fn install() -> (tempfile::TempDir, PathBuf) {
    install_of(&archives())
}

fn built(game: &MountedGame, root: &Path) -> WorldIndex {
    WorldIndex::build(&MountedWorld::new(game, root), None).0
}

/// The copy the mount would load of each file of `extension`: its lower-case leaf, and the last
/// logical path the mount lists it under.
fn winners(game: &MountedGame, extension: &str) -> BTreeMap<String, String> {
    let mut winners = BTreeMap::new();
    game.for_each_file(|logical, _container, leaf| {
        let leaf = leaf.to_ascii_lowercase();
        if leaf.ends_with(extension) {
            winners.insert(leaf, logical.to_string());
        }
    });
    winners
}

/// The fixture's files of one kind, last copy of a leaf winning, by stem.
fn fixture_winners(pick: impl Fn(&What) -> bool) -> BTreeMap<String, What> {
    let mut out = BTreeMap::new();
    for (_, files) in archives() {
        for (path, what) in files {
            if pick(&what) {
                let leaf = path.rsplit('/').next().expect("a leaf");
                let stem = leaf.rsplit_once('.').expect("an extension").0;
                out.insert(stem.to_string(), what);
            }
        }
    }
    out
}

fn vec3(v: [f32; 3]) -> Vec3 {
    Vec3::from(v)
}

fn span(s: Span) -> (Vec3, Vec3) {
    (vec3(s.0), vec3(s.1))
}

fn centre(s: Span) -> Vec3 {
    (vec3(s.0) + vec3(s.1)) * 0.5
}

fn names<'a>(
    maps: impl IntoIterator<Item = &'a or_world_gta5::world_index::Map>,
) -> BTreeSet<String> {
    maps.into_iter().map(|map| map.name.clone()).collect()
}

#[test]
fn the_index_names_every_map_and_type_file_the_mount_lists() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let index = built(&game, &root);

    let listed_maps = winners(&game, ".ymap");
    let listed_types = winners(&game, ".ytyp");
    assert_eq!(listed_maps.len(), 4, "the fixture lists four maps by name");
    assert_eq!(listed_types.len(), 2);

    let named: BTreeMap<String, String> = (index.maps().iter())
        .map(|map| (format!("{}.ymap", map.name), map.logical.clone()))
        .collect();
    assert_eq!(named, listed_maps, "each map, at the copy the mount loads");
    let named: BTreeMap<String, String> = (index.type_files().iter())
        .map(|file| (format!("{}.ytyp", file.name), file.logical.clone()))
        .collect();
    assert_eq!(named, listed_types);
    assert!(index.maps().iter().all(|map| map.read));
    assert!(index.type_files().iter().all(|file| file.read));

    // What each map states, from the copy that wins.
    let specs = fixture_winners(|what| matches!(what, What::Map(_)));
    assert_eq!(specs.len(), index.maps().len());
    for (name, what) in &specs {
        let What::Map(spec) = what else {
            unreachable!()
        };
        let map = index.map(hash(name)).expect(name);
        assert_eq!(map.logical, listed_maps[&format!("{name}.ymap")]);
        assert_eq!(map.streaming, Some(span(spec.streaming)), "{name}");
        assert_eq!(map.entities, Some(span(spec.entities)), "{name}");
        assert_eq!(map.content_flags, spec.content_flags, "{name}");
        assert_eq!(map.flags, spec.flags, "{name}");
        let interiors: Vec<u32> = spec.interiors.iter().map(|i| hash(i)).collect();
        assert_eq!(map.interiors, interiors, "{name}");
    }
    assert_eq!(index.shadowed(), 1, "one copy of `m_north` is shadowed");

    // Each archetype, from the last type file to declare it.
    let mut declared: BTreeMap<&str, (&str, Span, f32)> = BTreeMap::new();
    let mut declarations = 0;
    for (stem, what) in fixture_winners(|what| matches!(what, What::Types(_))) {
        let What::Types(archetypes) = what else {
            unreachable!()
        };
        let stem: &'static str = Box::leak(stem.into_boxed_str());
        assert_eq!(
            index.type_file(hash(stem)).map(|f| f.archetypes),
            Some(archetypes.len())
        );
        for (name, bounds, lod_dist) in archetypes {
            declarations += 1;
            declared.insert(name, (stem, bounds, lod_dist));
        }
    }
    assert_eq!(index.archetype_count(), declared.len());
    assert_eq!(index.redefined(), declarations - declared.len());
    assert_eq!(index.redefined(), 1, "`prop_a` is declared twice");
    for (name, (file, bounds, lod_dist)) in declared {
        let archetype = index.archetype(hash(name)).expect(name);
        assert_eq!(
            archetype.type_file,
            hash(file),
            "{name} is declared in {file}"
        );
        assert_eq!(archetype.span, Some(span(bounds)), "{name}");
        assert_eq!(archetype.lod_dist, lod_dist, "{name}");
    }
    assert_eq!(
        index.archetype(hash("prop_a")).map(|a| a.type_file),
        Some(hash("types_b")),
        "the later archive's type file is the one `prop_a` is taken from"
    );
}

#[test]
fn a_point_inside_a_maps_extents_answers_it_and_one_outside_every_map_answers_none() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let index = built(&game, &root);
    let specs = fixture_winners(|what| matches!(what, What::Map(_)));
    // What a scan of every map answers, which the tree has to agree with.
    let by_scanning = |at: Vec3| -> BTreeSet<String> {
        let holds = |what: &What| {
            let What::Map(spec) = what else {
                unreachable!()
            };
            let (min, max) = span(spec.streaming);
            at.cmpge(min).all() && at.cmple(max).all()
        };
        let held = specs.iter().filter(|(_, what)| holds(what));
        held.map(|(name, _)| name.clone()).collect()
    };

    let mut asked = 0;
    for (name, what) in &specs {
        let What::Map(spec) = what else {
            unreachable!()
        };
        let at = centre(spec.entities);
        let answered = names(index.maps_at(at));
        assert!(answered.contains(name), "{name} at {at}: {answered:?}");
        assert_eq!(answered, by_scanning(at), "{name} at {at}");
        asked += 1;
    }
    assert_eq!(asked, 4);

    // The child lies inside its parent, so one point answers both.
    let both = names(index.maps_at(centre(cube([120.0, 120.0, 0.0], 20.0))));
    let expected: BTreeSet<String> = ["m_north", "m_north_child"].map(String::from).into();
    assert_eq!(both, expected);

    // Five metres outside the entities and inside the ten the map streams from: the streaming
    // extents answer, not the entities'.
    let margin = vec3(south().0) - Vec3::splat(5.0);
    assert_eq!(names(index.maps_at(margin)), ["m_south".to_string()].into());
    let beyond = vec3(south().0) - Vec3::splat(15.0);
    assert_eq!(names(index.maps_at(beyond)), BTreeSet::new());

    for outside in [
        Vec3::ZERO,
        Vec3::splat(5000.0),
        Vec3::new(150.0, 150.0, 900.0),
    ] {
        assert_eq!(
            by_scanning(outside),
            BTreeSet::new(),
            "{outside} is in no map"
        );
        assert_eq!(index.maps_at(outside).len(), 0, "{outside}");
    }
    // Where the shadowed copy of `m_north` said it was, nothing is.
    assert_eq!(index.maps_at(Vec3::new(-875.0, -875.0, 25.0)).len(), 0);
}

#[test]
fn a_map_with_a_parent_names_it() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let index = built(&game, &root);

    let child = index.map(hash("m_north_child")).expect("listed");
    assert_eq!(child.parent, hash("m_north"));
    let south = index.map(hash("m_south")).expect("listed");
    assert_eq!(south.logical, "platform:/levels/pack.rpf/m_south.ymap");
    assert!(south.read, "a map in a nested archive is read from it");
    let parent = index.parent_of(child).expect("the mount holds the parent");
    assert_eq!(parent.name, "m_north");
    assert_eq!(parent.logical, "platform:/patch/m_north.ymap");

    let orphans: Vec<&str> = (index.maps().iter())
        .filter(|map| index.parent_of(map).is_none())
        .map(|map| map.name.as_str())
        .collect();
    assert_eq!(orphans.len(), 3, "{orphans:?}");
    assert!(index
        .maps()
        .iter()
        .all(|map| (map.parent != 0) == (map.name == "m_north_child")));
}

/// The catalogue, the tables and the boxes the dependency resolver answers a map's bounds from.
fn resolver_inputs(game: &MountedGame) -> (Catalogue, InstallTables, ModelContents) {
    let mut catalogue = Catalogue::new();
    game.for_each_entry(|facts| catalogue.index(facts));
    let read = |logical: &str| {
        let bytes = game.vfs.open(logical).expect("installed");
        or_io::meta::binary::parse_meta_binary(or_io::meta::default_library(), &bytes)
            .expect("parses")
    };
    let mut tables = InstallTables::new();
    let manifest = read("common:/data/levels/gta5/_manifest.ymf");
    tables.add_manifest(or_world_gta5::Manifest::new(&manifest).expect("a CPackFileMetaData"));
    let mut contents = ModelContents::new();
    for located in catalogue.of_kind(Kind::MapData) {
        let map = read(&located.logical);
        contents.add_map(
            hash(&located.stem),
            or_world_gta5::MapData::new(&map).expect("a CMapData"),
        );
    }
    let bounds: Vec<u32> = (catalogue.of_kind(Kind::Bounds))
        .map(|located| hash(&located.stem))
        .collect();
    for bound in bounds {
        assert!(contents.read_bound(game, &catalogue, bound));
    }
    (catalogue, tables, contents)
}

#[test]
fn a_maps_bounds_are_the_ones_the_tables_name_today() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let index = built(&game, &root);
    let (catalogue, tables, contents) = resolver_inputs(&game);
    let archetypes = ArchetypeAssetIndex::new();
    let resolver = Resolver::new(&catalogue, &archetypes, &tables).reading(&contents);

    let mut compared = 0;
    for map in index.maps() {
        let set = resolver.of_map(map.hash).expect("the install holds it");
        let mut today: Vec<(u32, Via, Option<String>)> = (set.edges.iter())
            .map(|edge| (edge.name, edge.via, edge.logical().map(str::to_string)))
            .collect();
        let mut indexed: Vec<(u32, Via, Option<String>)> = (index.bounds_of_map(map.hash).iter())
            .filter(|carried| carried.via != Via::ManifestInterior)
            .map(|c| (c.name, c.via, c.bound.map(|b| b.logical.clone())))
            .collect();
        let by_name = |edge: &(u32, Via, Option<String>)| edge.0;
        today.sort_by_key(by_name);
        indexed.sort_by_key(by_name);
        assert_eq!(indexed, today, "{}", map.name);
        compared += today.len();
    }
    assert_eq!(
        compared,
        GROUP_BOUNDS.len() + 3,
        "the group's three stated bounds and the three `m_north`'s stem names"
    );

    // The group's bounds are the manifest's, in its order, the missing file reported.
    let group: Vec<(u32, bool)> = (index.bounds_of_map(hash("m_group")).iter())
        .map(|carried| (carried.name, carried.bound.is_some()))
        .collect();
    let stated: Vec<(u32, bool)> = (GROUP_BOUNDS.iter())
        .map(|bound| (hash(bound), *bound != "m_group_9"))
        .collect();
    assert_eq!(group, stated);

    // A stem bound outside the map's streaming extents is carried and flagged.
    let flagged: BTreeMap<u32, Via> = (index.bounds_of_map(hash("m_north")).iter())
        .map(|carried| (carried.name, carried.via))
        .collect();
    let stem = |inside| Via::MapStemConvention {
        inside_streaming_box: Some(inside),
    };
    let expected: BTreeMap<u32, Via> = [
        (hash("m_north_0"), stem(true)),
        (hash("hi@m_north_0"), stem(true)),
        (hash("ma@m_north_1"), stem(false)),
    ]
    .into();
    assert_eq!(flagged, expected);

    // An interior's bounds reach the map that places it.
    let interior: Vec<(u32, Via)> = (index.bounds_of_map(hash("m_south")).iter())
        .map(|carried| (carried.name, carried.via))
        .collect();
    let stated: Vec<(u32, Via)> = (tables.interior_bounds_of(hash("v_int_shop")).iter())
        .map(|bound| (*bound, Via::ManifestInterior))
        .collect();
    assert_eq!(stated.len(), INTERIOR_BOUNDS.len());
    assert_eq!(interior, stated);
}

#[test]
fn a_point_answers_the_bounds_something_carries_whose_box_holds_it() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let index = built(&game, &root);
    let at = |point: Vec3| -> BTreeMap<String, Vec<(u32, Via)>> {
        (index.bounds_at(point).iter())
            .map(|found| {
                let carriers = found.carriers.iter().map(|c| (c.name, c.via));
                (found.bound.name.clone(), carriers.collect())
            })
            .collect()
    };
    let stem = |inside| Via::MapStemConvention {
        inside_streaming_box: Some(inside),
    };

    let north = at(centre(cube([110.0, 110.0, 0.0], 30.0)));
    let expected: BTreeMap<String, Vec<(u32, Via)>> = [
        ("m_north_0".to_string(), vec![(hash("m_north"), stem(true))]),
        (
            "hi@m_north_0".to_string(),
            vec![(hash("m_north"), stem(true))],
        ),
    ]
    .into();
    assert_eq!(north, expected);

    let group = at(centre(cube([1010.0, 10.0, 0.0], 30.0)));
    let by_group = vec![(hash("m_group"), Via::ManifestMapDataGroup)];
    let expected: BTreeMap<String, Vec<(u32, Via)>> = [
        ("m_group_0".to_string(), by_group.clone()),
        ("hi@m_group_0".to_string(), by_group),
    ]
    .into();
    assert_eq!(group, expected);

    // An interior carries this one alone, so its box is about the interior and holds no place.
    let shop = index.bound(hash("v_int_shop_0")).and_then(|b| b.span);
    assert_eq!(
        shop.map(|(min, max)| (min + max) * 0.5).map(at),
        Some(BTreeMap::new())
    );

    // Its box holds this point, and nothing carries it: the manifest names the group's bounds.
    let uncarried = Vec3::new(1075.0, 75.0, 25.0);
    let span = index
        .bound(hash("m_group_7"))
        .and_then(|b| b.span)
        .expect("read");
    assert!(uncarried.cmpge(span.0).all() && uncarried.cmple(span.1).all());
    assert_eq!(at(uncarried), BTreeMap::new());

    // Two archetypes' own collision hold the origin in their boxes, and are no place in the world.
    for prop in ["prop_col", "prop_bench_01"] {
        let span = index.bound(hash(prop)).and_then(|b| b.span).expect("read");
        assert!(Vec3::ZERO.cmpge(span.0).all() && Vec3::ZERO.cmple(span.1).all());
    }
    assert_eq!(at(Vec3::ZERO), BTreeMap::new());
    assert_eq!(at(Vec3::splat(5000.0)), BTreeMap::new());

    let carried: usize = 3 + GROUP_BOUNDS.len() + INTERIOR_BOUNDS.len();
    assert_eq!(index.carried_bounds(), carried);
    assert_eq!(
        index.bounds().len(),
        9,
        "every `.ybn` is named, carried or not"
    );
}

/// An interior stands far from the origin and its bound states a box about the interior's own
/// origin, as every such bound of both installs does: that box holds no place of the world.
#[test]
fn an_interiors_bound_whose_box_is_about_the_interior_is_answered_at_no_position_and_named_by_its_map(
) {
    let far = cube([2000.0, 2000.0, 0.0], 100.0);
    let town = cube([-50.0, -50.0, 0.0], 100.0);
    let room = cube([-8.0, -8.0, -1.0], 16.0);
    let manifest = manifest_bytes(&[], &[("int_far", &["int_far_col"])]);
    let archives: Vec<(&'static str, Files)> = vec![
        (
            "common.rpf",
            vec![("data/levels/gta5/_manifest.ymf", What::Raw(manifest))],
        ),
        (
            "x64a.rpf",
            vec![
                (
                    "levels/m_far.ymap",
                    What::Map(MapSpec {
                        interiors: vec!["int_far"],
                        ..MapSpec::around(far)
                    }),
                ),
                ("levels/m_town.ymap", What::Map(MapSpec::around(town))),
                ("levels/m_town_0.ybn", What::Bound(town)),
                ("levels/int_far_col.ybn", What::Bound(room)),
            ],
        ),
    ];
    let (_scratch, root) = install_of(&archives);
    let game = mount(&root);
    let index = built(&game, &root);
    let names = |found: Vec<or_world_gta5::world_index::BoundAt<'_>>| -> Vec<String> {
        found.iter().map(|at| at.bound.name.clone()).collect()
    };

    // The origin is in the interior's box and in the town's: the town alone is there.
    let origin = centre(room);
    assert!(origin.cmpge(Vec3::from(town.0)).all() && origin.cmple(Vec3::from(town.1)).all());
    assert_eq!(names(index.bounds_at(origin)), ["m_town_0"]);
    assert_eq!(
        names(index.bounds_in_column(origin.x, origin.y)),
        ["m_town_0"]
    );
    assert_eq!(
        names(index.bounds_near_column(origin.x, origin.y, 500.0)),
        ["m_town_0"]
    );

    // It is carried, and named where the map that places its interior streams.
    let carried = index.bounds_of_map(hash("m_far"));
    let by_interior: Vec<&str> = (carried.iter())
        .filter(|c| matches!(c.via, Via::ManifestInterior))
        .filter_map(|c| Some(c.bound?.name.as_str()))
        .collect();
    assert_eq!(by_interior, ["int_far_col"]);
    assert_eq!(index.carried_bounds(), 2);
    let there = centre(far);
    assert_eq!(
        names(index.interior_bounds_near_column(there.x, there.y, 0.0)),
        ["int_far_col"]
    );
    assert!(index.bounds_at(there).is_empty());
    assert!(index
        .interior_bounds_near_column(origin.x, origin.y, 0.0)
        .is_empty());
}

/// How many files of the fixture's archive `name` an index opens: each map, type file, manifest
/// and bound once.
fn files_of(archives: &[(&'static str, Files)], name: &str) -> u32 {
    let (_, files) = archives.iter().find(|(n, _)| *n == name).expect(name);
    files.len() as u32
}

fn kept_file(scratch: &tempfile::TempDir) -> PathBuf {
    scratch.path().join("kept").join("world.index")
}

/// What an index answers, as text: every map, type file and bound, and what lies at a few points.
fn answers(index: &WorldIndex) -> String {
    let mut out = format!(
        "{:#?}\n{:#?}\n{:#?}\n{:#?}\n",
        index.maps(),
        index.type_files(),
        archetypes(index),
        index.bounds()
    );
    for point in [
        centre(north()),
        centre(group()),
        centre(south()),
        Vec3::ZERO,
    ] {
        out += &format!("{point}: {:?}\n", names(index.maps_at(point)));
        out += &format!("{point}: {:?}\n", index.bounds_at(point));
    }
    for map in index.maps() {
        out += &format!("{}: {:?}\n", map.name, index.bounds_of_map(map.hash));
    }
    out
}

#[test]
fn a_second_build_reads_the_kept_file_and_opens_no_archive() {
    let (scratch, root) = install();
    let file = kept_file(&scratch);
    let all = archives();
    let per_archive: Vec<u32> = all.iter().map(|(name, _)| files_of(&all, name)).collect();

    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let counting = Counting::new(&source);
    let (first, report) = refresh(&counting, &file);
    assert!(
        matches!(report.kept, KeptState::Absent),
        "{:?}",
        report.kept
    );
    assert_eq!(report.read, ["common.rpf", "x64a.rpf", "x64b.rpf"]);
    assert_eq!(counting.counts(), (vec![1, 1, 1], per_archive.clone()));
    assert_eq!(report.opened, u64::from(per_archive.iter().sum::<u32>()));
    assert!(report.not_kept.is_none(), "{:?}", report.not_kept);
    assert!(file.is_file(), "the first build keeps the index");

    // A file written again is a new file with a new time, whatever its bytes.
    set_modified(&file, SystemTime::now() - Duration::from_secs(3600));
    let stood = modified(&file);

    // A mount made anew, as a second start makes one.
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let counting = Counting::new(&source);
    let (second, report) = refresh(&counting, &file);
    assert!(
        matches!(report.kept, KeptState::Current),
        "{:?}",
        report.kept
    );
    assert_eq!(
        modified(&file),
        stood,
        "a build that read nothing wrote the index again"
    );
    assert_eq!(report.read, Vec::<String>::new());
    assert_eq!(report.opened, 0);
    assert_eq!(
        counting.counts(),
        (vec![0, 0, 0], vec![0, 0, 0]),
        "no archive's entries were listed and no file was opened"
    );
    assert_eq!(answers(&second), answers(&first));
    assert_eq!(second.key(), first.key());
    assert!(
        answers(&second).contains("m_south"),
        "an index that answers nothing agrees with itself"
    );
}

fn modified(path: &Path) -> SystemTime {
    let meta = std::fs::metadata(path).expect("there");
    meta.modified().expect("a modification time")
}

/// Write the archive `name` again and date it ten seconds after it stood.
///
/// ⚠ Some file systems keep a time to the tick of a coarse clock, and an archive is whole blocks:
/// two writes a few milliseconds apart can bear one time and one size.
fn write_later(root: &Path, name: &str, files: &Files) {
    let path = root.join(name);
    let stood = modified(&path);
    write(root, name, files);
    set_modified(&path, stood + Duration::from_secs(10));
}

fn set_modified(path: &Path, time: SystemTime) {
    let file = std::fs::File::options()
        .write(true)
        .open(path)
        .expect("opened");
    file.set_modified(time).expect("timed");
}

#[test]
fn after_one_archive_changes_only_its_entries_are_read_again() {
    let (scratch, root) = install();
    let file = kept_file(&scratch);
    let game = mount(&root);
    let (first, _) = refresh(&MountedWorld::new(&game, &root), &file);
    assert_eq!(
        names(first.maps_at(centre(south()))),
        ["m_south".to_string()].into()
    );
    drop(game);

    // `x64b.rpf` is written again: `m_south` has moved and a map is new.
    let moved = cube([3000.0, 3000.0, 0.0], 100.0);
    let mut changed = archives();
    let (_, files) = changed
        .iter_mut()
        .find(|(n, _)| *n == "x64b.rpf")
        .expect("x64b");
    for (path, what) in files.iter_mut() {
        if *path == "levels/pack.rpf/m_south.ymap" {
            *what = What::Map(MapSpec::around(moved));
        }
    }
    files.push(("levels/m_new.ymap", What::Map(MapSpec::around(south()))));
    write_later(&root, "x64b.rpf", files);

    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let counting = Counting::new(&source);
    let (second, report) = refresh(&counting, &file);
    assert!(
        matches!(report.kept, KeptState::Changed),
        "{:?}",
        report.kept
    );
    assert_eq!(report.read, ["x64b.rpf"]);
    let opened = files_of(&changed, "x64b.rpf");
    assert_eq!(counting.counts(), (vec![0, 0, 1], vec![0, 0, opened]));
    assert_eq!(report.opened, u64::from(opened));

    assert_eq!(
        names(second.maps_at(centre(south()))),
        ["m_new".to_string()].into()
    );
    assert_eq!(
        names(second.maps_at(centre(moved))),
        ["m_south".to_string()].into()
    );
    // What the unchanged archive holds is answered as before, from the kept file.
    assert_eq!(
        names(second.maps_at(centre(group()))),
        ["m_group".to_string()].into()
    );
    assert_eq!(
        second.bounds_of_map(hash("m_group")),
        first.bounds_of_map(hash("m_group"))
    );
    assert_eq!(second.maps().len(), first.maps().len() + 1);

    // The same bytes written ten seconds later are another archive to the key: read again.
    let x64a = root.join("x64a.rpf");
    set_modified(&x64a, modified(&x64a) + Duration::from_secs(10));
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let counting = Counting::new(&source);
    let (third, report) = refresh(&counting, &file);
    assert_eq!(report.read, ["x64a.rpf"]);
    assert_eq!(counting.counts().0, vec![0, 1, 0]);
    assert_eq!(answers(&third), answers(&second));

    // And then nothing has changed.
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let counting = Counting::new(&source);
    let (_, report) = refresh(&counting, &file);
    assert!(
        matches!(report.kept, KeptState::Current),
        "{:?}",
        report.kept
    );
    assert_eq!(counting.counts(), (vec![0, 0, 0], vec![0, 0, 0]));
}

/// ⚠ Two archives of one device can hold one path, and the device answers it with the later
/// archive's copy: a row read through the device would keep, under the earlier archive, what the
/// later one held, and show it once the later archive no longer shadowed it.
#[test]
fn an_archive_is_read_from_itself_where_a_later_one_holds_the_same_path() {
    let earlier = MapSpec {
        content_flags: 1,
        ..MapSpec::around(north())
    };
    let later = MapSpec {
        content_flags: 2,
        ..MapSpec::around(south())
    };
    let twice = "shared/m_twice.ymap";
    let mut archives: Vec<(&'static str, Files)> = vec![
        ("common.rpf", vec![("data/_manifest.ymf", What::Manifest)]),
        ("x64a.rpf", vec![(twice, What::Map(earlier))]),
        ("x64b.rpf", vec![(twice, What::Map(later))]),
    ];
    let (scratch, root) = install_of(&archives);
    let file = kept_file(&scratch);
    let (first, ..) = read_again(&root, &file);
    let flags = |index: &WorldIndex| index.map(hash("m_twice")).map(|map| map.content_flags);
    assert_eq!(
        flags(&first),
        Some(2),
        "the later archive's copy is the one loaded"
    );
    assert_eq!(first.shadowed(), 1);

    // The later archive stops holding the map: the earlier one's copy is loaded, and it is
    // answered from the kept file, which has to hold what the earlier archive holds.
    archives[2].1 = vec![("levels/m_other.ymap", What::Map(MapSpec::around(group())))];
    write_later(&root, "x64b.rpf", &archives[2].1);
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let counting = Counting::new(&source);
    let (second, report) = refresh(&counting, &file);
    assert_eq!(report.read, ["x64b.rpf"]);
    assert_eq!(counting.counts(), (vec![0, 0, 1], vec![0, 0, 1]));
    assert_eq!(flags(&second), Some(1), "the earlier archive's own copy");
    assert_eq!(
        names(second.maps_at(centre(north()))),
        ["m_twice".to_string()].into()
    );
    assert_eq!(second.maps_at(centre(south())).len(), 0);
}

/// A build over `root` from the file as it is, and the number of archives it read again.
fn read_again(root: &Path, file: &Path) -> (WorldIndex, KeptState, usize) {
    let game = mount(root);
    let source = MountedWorld::new(&game, root);
    let counting = Counting::new(&source);
    let (index, report) = refresh(&counting, file);
    let listed: u32 = counting.counts().0.iter().sum();
    assert_eq!(listed as usize, report.read.len());
    (index, report.kept, report.read.len())
}

#[test]
fn a_kept_file_of_another_version_or_another_key_is_rebuilt_never_trusted() {
    let (scratch, root) = install();
    let file = kept_file(&scratch);
    let (first, ..) = read_again(&root, &file);
    let sealed = std::fs::read(&file).expect("kept");
    assert!(Kept::unseal(&sealed).is_ok());

    // The version is the second word of the file and the rules the third.
    assert_eq!(sealed[4..8], VERSION.to_le_bytes());
    assert_eq!(sealed[8..12], RULES.to_le_bytes());
    let mut another_version = sealed.clone();
    another_version[4] += 1;
    let mut other_rules = sealed.clone();
    other_rules[8] += 1;
    let refused = Kept::unseal(&another_version).expect_err("another version");
    assert!(
        matches!(refused, KeptError::Version { found } if found == VERSION + 1),
        "{refused}"
    );
    let refused = Kept::unseal(&other_rules).expect_err("other rules");
    assert!(
        matches!(refused, KeptError::Rules { found } if found == RULES + 1),
        "{refused}"
    );
    for other in [&another_version, &other_rules] {
        std::fs::write(&file, other).expect("written");
        let (index, kept, read) = read_again(&root, &file);
        assert!(matches!(kept, KeptState::Refused(_)), "{kept:?}");
        assert_eq!(read, 3, "every archive is read again");
        assert_eq!(answers(&index), answers(&first));
        let (_, kept, read) = read_again(&root, &file);
        assert!(
            matches!(kept, KeptState::Current) && read == 0,
            "not kept anew: {kept:?}"
        );
    }

    // The key is the first word of what the file seals: one that is not its archives' is refused.
    let stream = &sealed[20..];
    let mut plain = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(stream), &mut plain)
        .expect("inflates");
    assert_eq!(plain[..8], first.key().to_le_bytes());
    plain[0] ^= 1;
    let mut other_key = sealed[..20].to_vec();
    let mut stream = flate2::write::ZlibEncoder::new(&mut other_key, flate2::Compression::fast());
    std::io::Write::write_all(&mut stream, &plain).expect("deflates");
    stream.finish().expect("deflates");
    let refused = Kept::unseal(&other_key).expect_err("a key that is not its archives'");
    assert!(matches!(refused, KeptError::Malformed(_)), "{refused}");
    std::fs::write(&file, &other_key).expect("written");
    let (_, kept, read) = read_again(&root, &file);
    assert!(
        matches!(kept, KeptState::Refused(KeptError::Malformed(_))),
        "{kept:?}"
    );
    assert_eq!(read, 3);

    // Another install whose archives are the same bytes, sizes and times: its key is another,
    // and nothing of the kept file answers for it.
    let other = scratch.path().join("other-install");
    std::fs::create_dir(&other).expect("mkdir");
    let mut shifted = archives();
    for (name, _) in &shifted {
        std::fs::copy(root.join(name), other.join(name)).expect("copied");
        set_modified(&other.join(name), modified(&root.join(name)));
    }
    let same_stamps = |a: &Path, b: &Path| {
        let game = (mount(a), mount(b));
        let stamps = |game, root| {
            let source = MountedWorld::new(game, root);
            let archives = or_world_gta5::WorldSource::archives(&source);
            assert!(archives.iter().all(|archive| archive.stamp.is_some()));
            archives
        };
        stamps(&game.0, a) == stamps(&game.1, b)
    };
    assert!(
        same_stamps(&root, &other),
        "the copy is the same archives to a stamp"
    );
    let (index, kept, read) = read_again(&other, &file);
    assert!(matches!(kept, KeptState::Foreign), "{kept:?}");
    assert_eq!(read, 3, "every archive of the other install is read");
    assert_ne!(index.key(), first.key());

    // Its archives then change under the names the first install's had: the kept file of the
    // first, put back, answers nothing for them.
    std::fs::write(&file, &sealed).expect("written");
    let (_, files) = shifted
        .iter_mut()
        .find(|(n, _)| *n == "x64b.rpf")
        .expect("x64b");
    files.retain(|(path, _)| *path != "levels/pack.rpf/m_south.ymap");
    write_later(&other, "x64b.rpf", files);
    let (index, kept, read) = read_again(&other, &file);
    assert!(matches!(kept, KeptState::Foreign), "{kept:?}");
    assert_eq!(read, 3);
    assert!(
        index.map(hash("m_south")).is_none(),
        "the kept file's map answered"
    );
    assert!(first.map(hash("m_south")).is_some());
}

#[test]
fn a_truncated_or_corrupt_kept_file_is_refused_with_an_error_and_rebuilt() {
    let (scratch, root) = install();
    let file = kept_file(&scratch);
    let (first, ..) = read_again(&root, &file);
    let sealed = std::fs::read(&file).expect("kept");
    assert!(sealed.len() > 200, "{} bytes", sealed.len());

    // Cut at every length: none reads as an index.
    for len in 0..sealed.len() {
        let refused = Kept::unseal(&sealed[..len]);
        assert!(refused.is_err(), "the first {len} bytes read as an index");
    }
    // With every byte in turn inverted: the file is refused, or it says what it said. A byte of
    // the stream can spell the same bytes another way (a run of zeros copied from one byte back
    // or from two), and the checksum then holds because nothing it sums has changed.
    let whole = Kept::unseal(&sealed).expect("whole");
    let (head, checksum) = (20, sealed.len() - 4);
    let mut refused = 0;
    for at in 0..sealed.len() {
        let mut altered = sealed.clone();
        altered[at] ^= 0xFF;
        match Kept::unseal(&altered) {
            Err(_) => refused += 1,
            Ok(same) => {
                assert!(
                    (head..checksum).contains(&at),
                    "byte {at} of the head or the checksum inverted reads as an index"
                );
                assert_eq!(same, whole, "byte {at} inverted reads as another index");
            }
        }
    }
    assert!(
        refused >= sealed.len() * 9 / 10,
        "{refused} of {} refused",
        sealed.len()
    );
    let mut longer = sealed.clone();
    longer.push(0);
    assert!(
        Kept::unseal(&longer).is_err(),
        "a byte after the stream's end reads as an index"
    );

    // Cut in half, and with the last byte of its checksum inverted.
    let mut altered = sealed.clone();
    altered[sealed.len() - 1] ^= 0xFF;
    for damaged in [&sealed[..sealed.len() / 2], &altered[..]] {
        let refused = Kept::unseal(damaged).expect_err("damaged");
        assert!(matches!(refused, KeptError::Damaged), "{refused}");
        std::fs::write(&file, damaged).expect("written");
        let (index, kept, read) = read_again(&root, &file);
        assert!(
            matches!(kept, KeptState::Refused(KeptError::Damaged)),
            "{kept:?}"
        );
        assert_eq!(read, 3, "every archive is read again");
        assert_eq!(answers(&index), answers(&first));
        assert_eq!(std::fs::read(&file).expect("kept anew"), sealed);
        let (_, kept, read) = read_again(&root, &file);
        assert!(
            matches!(kept, KeptState::Current) && read == 0,
            "not kept anew: {kept:?}"
        );
    }

    // A file that is not an index at all.
    std::fs::write(&file, b"not an index").expect("written");
    let (_, kept, read) = read_again(&root, &file);
    assert!(
        matches!(kept, KeptState::Refused(KeptError::NotAnIndex)),
        "{kept:?}"
    );
    assert_eq!(read, 3);
}

/// What an index says of every archetype, lowest name first.
fn archetypes(index: &WorldIndex) -> Vec<or_world_gta5::world_index::Archetype> {
    let mut all: Vec<_> = index.archetypes().copied().collect();
    all.sort_by_key(|archetype| archetype.name);
    all
}

#[test]
fn a_later_copy_of_a_type_file_or_a_bound_is_the_one_named() {
    let early = cube([0.0, 0.0, 0.0], 10.0);
    let late = cube([700.0, 700.0, 0.0], 10.0);
    let unit = cube([-1.0, -1.0, -1.0], 2.0);
    // The interiors a map places, one of them twice and one with no name.
    let placed = MapSpec {
        interiors: vec!["v_int_b", "", "v_int_a", "v_int_b"],
        ..MapSpec::around(late)
    };
    let archives: Vec<(&'static str, Files)> = vec![
        ("common.rpf", vec![("data/_manifest.ymf", What::Manifest)]),
        (
            "x64a.rpf",
            vec![
                (
                    "types/t_twice.ytyp",
                    What::Types(vec![("prop_x", unit, 10.0), ("prop_gone", unit, 5.0)]),
                ),
                ("levels/m_twice_0.ybn", What::Bound(early)),
                ("levels/m_twice.ymap", What::Map(placed)),
            ],
        ),
        (
            "x64b.rpf",
            vec![
                (
                    "patch/t_twice.ytyp",
                    What::Types(vec![("prop_x", unit, 20.0), ("", unit, 1.0)]),
                ),
                ("patch/m_twice_0.ybn", What::Bound(late)),
            ],
        ),
    ];
    let (_scratch, root) = install_of(&archives);
    let game = mount(&root);
    let index = built(&game, &root);

    let named: BTreeMap<String, String> = (index.type_files().iter())
        .map(|file| (format!("{}.ytyp", file.name), file.logical.clone()))
        .collect();
    assert_eq!(named, winners(&game, ".ytyp"));
    assert_eq!(named["t_twice.ytyp"], "platform:/patch/t_twice.ytyp");
    let file = index.type_file(hash("t_twice")).expect("listed");
    assert_eq!(file.logical, "platform:/patch/t_twice.ytyp");
    assert_eq!(hash(""), 0, "an archetype with no name is named zero");
    assert_eq!(
        file.archetypes, 1,
        "the archetype with no name is not one a map can place"
    );
    let prop_x = index.archetype(hash("prop_x")).expect("declared");
    assert_eq!(prop_x.lod_dist, 20.0, "from the copy the mount loads");
    assert_eq!(
        archetypes(&index)
            .iter()
            .map(|a| a.name)
            .collect::<Vec<_>>(),
        [hash("prop_x")],
        "what only the shadowed copy declares is declared nowhere"
    );
    assert_eq!(
        index.redefined(),
        0,
        "a shadowed copy is not a second declaration"
    );

    let named: BTreeMap<String, String> = (index.bounds().iter())
        .map(|bound| (format!("{}.ybn", bound.name), bound.logical.clone()))
        .collect();
    assert_eq!(named, winners(&game, ".ybn"));
    let bound = index.bound(hash("m_twice_0")).expect("listed");
    assert_eq!(bound.logical, "platform:/patch/m_twice_0.ybn");
    assert_eq!(bound.span, Some(span(late)));
    assert_eq!(index.bounds_at(centre(early)).len(), 0);
    assert_eq!(index.bounds_at(centre(late)).len(), 1);
    assert_eq!(index.shadowed(), 2);

    let mut interiors = vec![hash("v_int_a"), hash("v_int_b")];
    interiors.sort_unstable();
    let map = index.map(hash("m_twice")).expect("listed");
    assert_eq!(map.interiors, interiors, "each once, lowest first");
}

#[test]
fn a_file_that_does_not_read_is_named_and_says_so() {
    let whole = MapSpec::around(north());
    let cut = map_bytes("m_cut", &whole);
    let cut = cut[..cut.len() / 2].to_vec();
    let unit = cube([-1.0, -1.0, -1.0], 2.0);
    let archives: Vec<(&'static str, Files)> = vec![
        (
            "common.rpf",
            vec![
                ("data/_manifest.ymf", What::Manifest),
                (
                    "data/other/_manifest.ymf",
                    What::Raw(b"no manifest".to_vec()),
                ),
            ],
        ),
        (
            "x64a.rpf",
            vec![
                ("levels/m_whole.ymap", What::Map(whole)),
                ("levels/m_cut.ymap", What::Raw(cut)),
                ("levels/m_noise.ymap", What::Raw(b"no map at all".to_vec())),
                // A type file under a map's name, and a map under a type file's.
                (
                    "levels/m_types.ymap",
                    What::Raw(types_bytes("m_types", &[("prop_a", unit, 50.0)])),
                ),
                (
                    "types/t_map.ytyp",
                    What::Raw(map_bytes("t_map", &MapSpec::around(south()))),
                ),
                ("types/t_noise.ytyp", What::Raw(vec![0xFF; 64])),
                ("levels/m_whole_0.ybn", What::Raw(b"no bound".to_vec())),
            ],
        ),
    ];
    let (scratch, root) = install_of(&archives);
    let file = kept_file(&scratch);
    let (first, ..) = read_again(&root, &file);

    let said = |index: &WorldIndex| {
        let maps: BTreeMap<String, bool> = (index.maps().iter())
            .map(|map| (map.name.clone(), map.read))
            .collect();
        let types: BTreeMap<String, bool> = (index.type_files().iter())
            .map(|file| (file.name.clone(), file.read))
            .collect();
        (
            maps,
            types,
            index.manifests(),
            index.bound(hash("m_whole_0")).cloned(),
        )
    };
    let (maps, types, manifests, bound) = said(&first);
    let expected: BTreeMap<String, bool> = [
        ("m_whole", true),
        ("m_cut", false),
        ("m_noise", false),
        ("m_types", false),
    ]
    .map(|(name, read)| (name.to_string(), read))
    .into();
    assert_eq!(maps, expected);
    let expected: BTreeMap<String, bool> = [("t_map", false), ("t_noise", false)]
        .map(|(name, read)| (name.to_string(), read))
        .into();
    assert_eq!(types, expected);
    assert_eq!(manifests, (2, 1), "two manifests, one that is none");
    let bound = bound.expect("named");
    assert_eq!(bound.logical, "platform:/levels/m_whole_0.ybn");
    assert_eq!(bound.span, None);

    for name in ["m_cut", "m_noise", "m_types"] {
        let map = first.map(hash(name)).expect(name);
        assert_eq!(
            (map.streaming, map.entities, map.parent, map.interiors.len()),
            (None, None, 0, 0),
            "{name} states nothing"
        );
    }
    assert_eq!(first.archetype_count(), 0);
    assert_eq!(
        names(first.maps_at(centre(north()))),
        ["m_whole".to_string()].into(),
        "the map cut short lies nowhere"
    );
    assert_eq!(first.maps_at(centre(south())).len(), 0);
    assert_eq!(first.bounds_at(centre(north())).len(), 0);

    // The kept file says the same of each, and nothing is opened to say it.
    let (second, kept, read) = read_again(&root, &file);
    assert!(matches!(kept, KeptState::Current), "{kept:?}");
    assert_eq!(read, 0);
    assert_eq!(said(&second), said(&first));
}

/// A mount of archives that hold no file: what a kept index is matched against, and nothing else.
struct Bare {
    identity: &'static str,
    archives: Vec<Archive>,
    listed: std::cell::RefCell<Vec<usize>>,
}

impl Bare {
    fn new(identity: &'static str, archives: &[Archive]) -> Self {
        Self {
            identity,
            archives: archives.to_vec(),
            listed: Vec::new().into(),
        }
    }

    /// The index of this mount from `kept`, the state it found `kept` in, and the archives it read.
    fn built_from(&self, kept: Option<&WorldIndex>) -> (WorldIndex, KeptState, Vec<usize>) {
        let kept = kept.map(|index| {
            let sealed = index.seal().expect("seals");
            Kept::unseal(&sealed).expect("unseals")
        });
        self.listed.borrow_mut().clear();
        let (index, report) = WorldIndex::build(self, kept);
        let listed = self.listed.borrow().clone();
        assert_eq!(report.read.len(), listed.len());
        assert_eq!(report.archives, self.archives.len());
        (index, report.kept, listed)
    }
}

impl WorldSource for Bare {
    fn identity(&self) -> String {
        self.identity.to_string()
    }

    fn archives(&self) -> Vec<Archive> {
        self.archives.clone()
    }

    fn entries(&self, archive: usize, _visit: &mut dyn FnMut(&or_title::vfs::EntryFacts)) {
        self.listed.borrow_mut().push(archive);
    }

    fn open(&self, _archive: usize, _logical: &str) -> Option<Vec<u8>> {
        None
    }

    fn open_head(&self, _archive: usize, _logical: &str, _limit: usize) -> Option<Vec<u8>> {
        None
    }
}

fn archive(device: &str, container: &str, size: u64, modified: u64) -> Archive {
    Archive {
        device: device.to_string(),
        container: container.to_string(),
        stamp: Some(Stamp { size, modified }),
    }
}

#[test]
fn the_key_tells_every_field_of_every_archive_and_an_archive_is_kept_by_all_of_them() {
    let base = [
        archive("common", "common.rpf", 100, 7),
        archive("platform", "x64a.rpf", 200, 8),
        archive("platform", "x64b.rpf", 300, 9),
    ];
    let (first, kept, read) = Bare::new("install", &base).built_from(None);
    assert!(matches!(kept, KeptState::Absent), "{kept:?}");
    assert_eq!(read, [0, 1, 2]);
    let (again, kept, read) = Bare::new("install", &base).built_from(Some(&first));
    assert!(matches!(kept, KeptState::Current), "{kept:?}");
    assert_eq!(read, Vec::<usize>::new());
    assert_eq!(again.key(), first.key());

    // One field of the second archive at a time: the key is another, and that archive alone is
    // read again.
    let other = |change: &dyn Fn(&mut Archive)| {
        let mut archives = base.to_vec();
        change(&mut archives[1]);
        archives
    };
    let changes: [(&str, Vec<Archive>); 5] = [
        ("device", other(&|a| a.device = "update".to_string())),
        (
            "container",
            other(&|a| a.container = "x64c.rpf".to_string()),
        ),
        (
            "size",
            other(&|a| {
                a.stamp = Some(Stamp {
                    size: 201,
                    modified: 8,
                })
            }),
        ),
        (
            "modified",
            other(&|a| {
                a.stamp = Some(Stamp {
                    size: 200,
                    modified: 9,
                })
            }),
        ),
        ("stamp", other(&|a| a.stamp = None)),
    ];
    let mut keys = vec![first.key()];
    for (what, archives) in &changes {
        let (index, kept, read) = Bare::new("install", archives).built_from(Some(&first));
        assert!(matches!(kept, KeptState::Changed), "{what}: {kept:?}");
        assert_eq!(read, [1], "{what}");
        keys.push(index.key());
    }

    // The same archives in another order are another mount, and none is read for it.
    let mut swapped = base.to_vec();
    swapped.swap(1, 2);
    let (index, kept, read) = Bare::new("install", &swapped).built_from(Some(&first));
    assert!(matches!(kept, KeptState::Changed), "{kept:?}");
    assert_eq!(read, Vec::<usize>::new());
    keys.push(index.key());

    // Two names that are the same letters cut at another place.
    let cut = |device: &str, container: &str| {
        let archives = [archive(device, container, 1, 1)];
        Bare::new("install", &archives).built_from(None).0.key()
    };
    keys.extend([cut("ab", "c"), cut("a", "bc")]);
    let cut = |identity: &'static str, device: &str| {
        let archives = [archive(device, "c", 1, 1)];
        Bare::new(identity, &archives).built_from(None).0.key()
    };
    keys.extend([cut("xy", "z"), cut("x", "yz")]);

    let (index, kept, read) = Bare::new("another", &base).built_from(Some(&first));
    assert!(matches!(kept, KeptState::Foreign), "{kept:?}");
    assert_eq!(read, [0, 1, 2]);
    keys.push(index.key());

    let distinct: BTreeSet<u64> = keys.iter().copied().collect();
    assert_eq!(distinct.len(), keys.len(), "{keys:x?}");
    assert_eq!(keys.len(), 12);
}

#[test]
fn an_archive_with_no_stamp_is_read_at_every_build() {
    let mut archives = [
        archive("common", "common.rpf", 100, 7),
        archive("platform", "x64a.rpf", 200, 8),
    ];
    archives[1].stamp = None;
    let bare = Bare::new("install", &archives);
    let (first, ..) = bare.built_from(None);
    for _ in 0..2 {
        let (index, kept, read) = bare.built_from(Some(&first));
        assert_eq!(read, [1], "nothing says the archive stands");
        assert!(
            matches!(kept, KeptState::Changed),
            "a kept index an archive is read beside did not stand: {kept:?}"
        );
        assert_eq!(index.key(), first.key());
    }
}

/// A content list may name one pack twice, and the mount then lists its archive twice.
#[test]
fn an_archive_the_mount_lists_twice_is_kept_twice_and_a_second_build_reads_neither() {
    let archives = [
        archive("common", "common.rpf", 100, 7),
        archive("dlc_twice", "dlcpacks/twice/dlc.rpf", 300, 9),
        archive("dlc_twice", "dlcpacks/twice/dlc.rpf", 300, 9),
        archive("platform", "x64a.rpf", 200, 8),
    ];
    let bare = Bare::new("install", &archives);
    let (first, kept, read) = bare.built_from(None);
    assert!(matches!(kept, KeptState::Absent), "{kept:?}");
    assert_eq!(read, [0, 1, 2, 3]);
    let (second, kept, read) = bare.built_from(Some(&first));
    assert_eq!(read, [0usize; 0], "an archive that stands was read again");
    assert!(matches!(kept, KeptState::Current), "{kept:?}");
    assert_eq!(second.archives().count(), archives.len());
    assert_eq!(second.key(), first.key());

    // One of the two changed: the other is still taken from the kept index.
    let mut changed = archives.clone();
    changed[2] = archive("dlc_twice", "dlcpacks/twice/dlc.rpf", 301, 9);
    let (_, kept, read) = Bare::new("install", &changed).built_from(Some(&first));
    assert_eq!(read, [2]);
    assert!(matches!(kept, KeptState::Changed), "{kept:?}");
}

#[test]
fn an_archive_of_another_size_or_one_that_left_is_not_answered_from_the_kept_file() {
    let (scratch, root) = install();
    let file = kept_file(&scratch);
    let (first, ..) = read_again(&root, &file);
    assert!(first.map(hash("m_south")).is_some());

    // `x64b.rpf` written again without `m_south`, longer, and with the time it had: only its size
    // says so. An archive is whole blocks, so it takes a file of some blocks to lengthen one.
    let x64b = root.join("x64b.rpf");
    let (stood, size) = (
        modified(&x64b),
        std::fs::metadata(&x64b).expect("there").len(),
    );
    let mut changed = archives();
    let (_, files) = changed
        .iter_mut()
        .find(|(n, _)| *n == "x64b.rpf")
        .expect("x64b");
    files.retain(|(path, _)| *path != "levels/pack.rpf/m_south.ymap");
    files.push(("levels/filler.dat", What::Raw(vec![0x5A; 1 << 14])));
    write(&root, "x64b.rpf", files);
    set_modified(&x64b, stood);
    assert_eq!(modified(&x64b), stood);
    assert_ne!(std::fs::metadata(&x64b).expect("there").len(), size);
    let (second, kept, read) = read_again(&root, &file);
    assert!(matches!(kept, KeptState::Changed), "{kept:?}");
    assert_eq!(read, 1, "the archive of another size is read again");
    assert!(second.map(hash("m_south")).is_none());

    // The archive leaves the install: nothing is read, what it held is gone, the copy it
    // shadowed is the one loaded, and the file is kept anew for the mount as it stands.
    std::fs::remove_file(&x64b).expect("removed");
    set_modified(&file, SystemTime::now() - Duration::from_secs(3600));
    let before = modified(&file);
    let (third, kept, read) = read_again(&root, &file);
    assert!(matches!(kept, KeptState::Changed), "{kept:?}");
    assert_eq!(read, 0);
    assert_eq!(third.type_files().len(), 1);
    assert!(third.archetype(hash("prop_c")).is_none());
    let north = third.map(hash("m_north")).expect("in x64a.rpf too");
    assert_eq!(north.logical, "platform:/levels/m_north.ymap");
    assert_eq!(north.content_flags, 2);
    assert_eq!(third.shadowed(), 0);
    assert_ne!(modified(&file), before, "the index is kept anew");
    let (_, kept, read) = read_again(&root, &file);
    assert!(
        matches!(kept, KeptState::Current) && read == 0,
        "not kept anew: {kept:?}"
    );
}

/// A source that reads no front of a file, as an archive answers for a payload it cannot cut.
struct Whole<'a>(&'a dyn WorldSource);

impl WorldSource for Whole<'_> {
    fn identity(&self) -> String {
        self.0.identity()
    }

    fn archives(&self) -> Vec<Archive> {
        self.0.archives()
    }

    fn entries(&self, archive: usize, visit: &mut dyn FnMut(&or_title::vfs::EntryFacts)) {
        self.0.entries(archive, visit);
    }

    fn open(&self, archive: usize, logical: &str) -> Option<Vec<u8>> {
        self.0.open(archive, logical)
    }

    fn open_head(&self, _archive: usize, _logical: &str, _limit: usize) -> Option<Vec<u8>> {
        None
    }
}

#[test]
fn a_bound_whose_front_gives_no_box_is_read_whole() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let (by_front, report) = WorldIndex::build(&source, None);
    let files: u32 = archives().iter().map(|(_, files)| files.len() as u32).sum();
    assert_eq!(report.opened, u64::from(files));

    let whole = Whole(&source);
    let counting = Counting::new(&whole);
    let (by_whole, report) = WorldIndex::build(&counting, None);
    assert_eq!(by_whole.bounds(), by_front.bounds());
    assert_eq!(by_whole.bounds().len(), 9);
    assert!(by_whole.bounds().iter().all(|bound| bound.span.is_some()));
    assert_eq!(
        report.opened,
        u64::from(files) + by_whole.bounds().len() as u64,
        "a bound read whole after its front is opened twice"
    );
    assert_eq!(
        counting.counts().1.iter().sum::<u32>() as u64,
        report.opened
    );
}

#[test]
fn a_bound_two_groups_name_says_both_lowest_name_first() {
    let groups: Vec<&'static str> = vec![
        "m_one", "m_two", "m_three", "m_four", "m_five", "m_six", "m_seven",
    ];
    let named = groups.iter().map(|group| (*group, vec!["m_shared_0"]));
    let archives: Vec<(&'static str, Files)> = vec![
        (
            "common.rpf",
            vec![("data/_manifest.ymf", What::Groups(named.collect()))],
        ),
        (
            "x64a.rpf",
            vec![("levels/m_shared_0.ybn", What::Bound(north()))],
        ),
    ];
    let (_scratch, root) = install_of(&archives);
    let game = mount(&root);
    let index = built(&game, &root);

    let found = index.bounds_at(centre(north()));
    assert_eq!(found.len(), 1);
    let carriers: Vec<u32> = found[0].carriers.iter().map(|c| c.name).collect();
    let mut expected: Vec<u32> = groups.iter().map(|group| hash(group)).collect();
    expected.sort_unstable();
    assert_eq!(carriers, expected);
    assert!(found[0]
        .carriers
        .iter()
        .all(|carrier| carrier.via == Via::ManifestMapDataGroup));
}

/// The kept file `sealed` with `change` made to what it seals, sealed again.
fn resealed(sealed: &[u8], change: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut plain = Vec::new();
    std::io::Read::read_to_end(
        &mut flate2::read::ZlibDecoder::new(&sealed[20..]),
        &mut plain,
    )
    .expect("inflates");
    change(&mut plain);
    let mut out = sealed[..12].to_vec();
    out.extend((plain.len() as u64).to_le_bytes());
    let mut stream = flate2::write::ZlibEncoder::new(&mut out, flate2::Compression::fast());
    std::io::Write::write_all(&mut stream, &plain).expect("deflates");
    stream.finish().expect("deflates");
    out
}

#[test]
fn a_kept_file_that_states_what_no_index_holds_is_refused() {
    let (scratch, root) = install();
    let file = kept_file(&scratch);
    let (first, ..) = read_again(&root, &file);
    let sealed = std::fs::read(&file).expect("kept");
    assert_eq!(
        Kept::unseal(&resealed(&sealed, |_| ())).expect("sealed again as it was"),
        Kept::unseal(&sealed).expect("whole")
    );

    let malformed = |sealed: &[u8], why: &str| {
        let refused = Kept::unseal(sealed).expect_err(why);
        assert!(
            matches!(refused, KeptError::Malformed(_)),
            "{why}: {refused}"
        );
        assert!(refused.to_string().contains(why), "{refused}");
    };
    malformed(
        &resealed(&sealed, |plain| plain.push(0)),
        "bytes after its last archive",
    );

    // Whether the first archive has a stamp is the byte after its two names, which follow the
    // key, the install's path and the number of archives.
    let identity = root.to_string_lossy().into_owned();
    let stamped = 8 + (4 + identity.len()) + 4 + (4 + "common".len()) + (4 + "common.rpf".len());
    let neither = resealed(&sealed, |plain| {
        assert_eq!(plain[stamped], 1, "the first archive has a stamp");
        assert_eq!(&plain[stamped - "common.rpf".len()..stamped], b"common.rpf");
        plain[stamped] = 2;
    });
    malformed(&neither, "a flag that is neither 0 nor 1");

    let not_text = resealed(&sealed, |plain| plain[stamped - 1] = 0xFF);
    malformed(&not_text, "a name that is not text");

    // A list said to be longer than the bytes left is cut short, whatever it says.
    let endless = resealed(&sealed, |plain| {
        let count = 8 + 4 + identity.len();
        assert_eq!(plain[count..count + 4], 3u32.to_le_bytes());
        plain[count..count + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    });
    let refused = Kept::unseal(&endless).expect_err("more archives than bytes");
    assert!(matches!(refused, KeptError::Damaged), "{refused}");

    for refused in [&neither, &endless] {
        std::fs::write(&file, refused).expect("written");
        let (index, kept, read) = read_again(&root, &file);
        assert!(matches!(kept, KeptState::Refused(_)), "{kept:?}");
        assert_eq!(read, 3);
        assert_eq!(answers(&index), answers(&first));
        assert_eq!(std::fs::read(&file).expect("kept anew"), sealed);
    }
}

/// ⚠ A mount holds each archive's table as it was when the mount was made. Stamped by the file as
/// it is when the index asks, what a mount read of the archive before would be kept as the
/// archive that replaced it, and no later start would read that one.
#[test]
fn an_archive_written_after_its_mount_was_made_is_not_kept_as_what_that_mount_read() {
    let (scratch, root) = install();
    let file = kept_file(&scratch);
    let before = mount(&root);

    // `x64b.rpf` is written again without `m_south`, ten seconds on, under the mount made before.
    let mut changed = archives();
    let (_, files) = changed
        .iter_mut()
        .find(|(n, _)| *n == "x64b.rpf")
        .expect("x64b");
    files.retain(|(path, _)| *path != "levels/pack.rpf/m_south.ymap");
    write_later(&root, "x64b.rpf", files);

    // What it reads of the archive through the table of the one before is whatever lies there.
    let (_, report) = refresh(&MountedWorld::new(&before, &root), &file);
    assert_eq!(report.read.len(), 3);
    assert!(report.not_kept.is_none(), "{:?}", report.not_kept);

    // A mount made now reads the archive as it is, whatever the mount before it kept.
    let (index, kept, read) = read_again(&root, &file);
    assert!(matches!(kept, KeptState::Changed), "{kept:?}");
    assert_eq!(read, 1, "the archive written since is read again");
    assert!(
        index.map(hash("m_south")).is_none(),
        "what the mount before read of the archive answered for it"
    );
    let (_, kept, read) = read_again(&root, &file);
    assert!(
        matches!(kept, KeptState::Current) && read == 0,
        "not kept anew: {kept:?}"
    );
}

/// ⚠ A level of DLC keeps the walk of a root out of the folders of the packs it leaves out, and the
/// file that holds the root is the same file at every level: kept by its stamp, what one level
/// walked of it would answer for a mount at another.
#[test]
fn a_root_a_level_keeps_folders_out_of_is_read_at_every_build_and_never_kept_for_another_level() {
    let (scratch, root) = install();
    let file = kept_file(&scratch);
    let (whole, ..) = read_again(&root, &file);
    assert!(whole.map(hash("m_south")).is_some());

    // The same install as a level leaves it: `levels/` of `x64b.rpf` is out of the walk.
    let mut game = mount(&root);
    let cut = (game.roots.iter_mut())
        .find(|root| root.container == "x64b.rpf")
        .expect("x64b");
    cut.left_out.push("levels/".to_string());
    for _ in 0..2 {
        let (at_level, report) = refresh(&MountedWorld::new(&game, &root), &file);
        assert_eq!(report.read, ["x64b.rpf"], "nothing says the archive stands");
        assert!(
            at_level.map(hash("m_south")).is_none(),
            "what the whole mount walked answered at the level"
        );
        assert!(
            at_level.map(hash("m_north")).is_some(),
            "a folder the level keeps was left out with the one it does not"
        );
    }

    // The whole mount again, over the file the level left.
    let (again, _, read) = read_again(&root, &file);
    assert_eq!(read, 1, "what the level walked was kept as the archive");
    assert_eq!(answers(&again), answers(&whole));
}

/// ⚠ The install suite cannot run where there is no install, so what it checks is run here, on the
/// install built by hand, where each of its checks has been seen to fail.
#[test]
fn what_the_install_suite_checks_holds_of_the_mount_built_by_hand() {
    let (scratch, root) = install();
    let held = held_to_its_mount(&|| mount(&root), &root, &kept_file(&scratch));
    let maps = fixture_winners(|what| matches!(what, What::Map(_))).len();
    let type_files = fixture_winners(|what| matches!(what, What::Types(_))).len();
    let bounds = fixture_winners(|what| matches!(what, What::Bound(_))).len();
    // Each bound a map's group or stem names that the install holds: `m_group_9` is named and is
    // no file, and the one an interior names is asked at no position.
    let asked = 3 + (GROUP_BOUNDS.len() - 1);
    let expected = Held {
        maps,
        type_files,
        bounds,
        maps_asked: maps,
        bounds_asked: asked,
    };
    assert_eq!(held, expected);
    assert_eq!((maps, type_files, bounds), (4, 2, 9));
}

/// ⚠ The measure the install suite prints, on an install built to give each of its counts.
#[test]
fn what_the_files_state_is_counted_on_an_install_built_to_state_it() {
    let far = cube([2000.0, 2000.0, 0.0], 100.0);
    let near = cube([-500.0, 300.0, 0.0], 100.0);
    let unit = cube([-1.0, -1.0, -1.0], 2.0);
    let room = cube([-10.0, -10.0, -2.0], 20.0);
    let manifest = manifest_bytes(
        &[],
        &[
            ("int_local", &["int_local_col"]),
            ("int_world", &["int_world_col"]),
            ("int_big", &["int_big_col"]),
            ("int_both", &["m_near_0"]),
        ],
    );
    let far_map = MapSpec {
        interiors: vec!["int_local"],
        ..MapSpec::around(far)
    };
    let near_map = MapSpec {
        parent: "m_far",
        interiors: vec!["int_world", "int_both"],
        ..MapSpec::around(near)
    };
    let west_map = MapSpec {
        parent: "m_far",
        interiors: vec!["int_big"],
        ..MapSpec::around(cube([-3000.0, -3000.0, -50.0], 100.0))
    };
    let renamed = MapSpec {
        parent: "m_nowhere",
        ..MapSpec::around(unit)
    };
    let archives: Vec<(&'static str, Files)> = vec![
        (
            "common.rpf",
            vec![("data/levels/gta5/_manifest.ymf", What::Raw(manifest))],
        ),
        (
            "x64a.rpf",
            vec![
                ("levels/m_far.ymap", What::Map(far_map)),
                ("levels/m_near.ymap", What::Map(near_map)),
                ("levels/m_west.ymap", What::Map(west_map)),
                // It states another name than its file's, and a parent that is no file.
                (
                    "levels/m_renamed.ymap",
                    What::Raw(map_bytes("m_other", &renamed)),
                ),
                // Each states the name of another map of the mount.
                (
                    "levels/m_grass.ymap",
                    What::Raw(map_bytes("m_far", &MapSpec::around(unit))),
                ),
                (
                    "levels/m_scrub.ymap",
                    What::Raw(map_bytes("m_near", &MapSpec::around(unit))),
                ),
                // About the interior's own origin, far from where its map puts the interior.
                (
                    "levels/int_local_col.ybn",
                    What::Bound(cube([-8.0, -8.0, -1.0], 16.0)),
                ),
                // Where the map that places its interior stands.
                (
                    "levels/int_world_col.ybn",
                    What::Bound(cube([-480.0, 320.0, 0.0], 20.0)),
                ),
                // An interior's by the manifest and its map's by its name, so it is answered where it states.
                (
                    "levels/m_near_0.ybn",
                    What::Bound(cube([-440.0, 360.0, 0.0], 20.0)),
                ),
                // About its interior's origin and off its centre.
                (
                    "levels/int_big_col.ybn",
                    What::Bound(cube([-10.0, -10.0, -2.0], 40.0)),
                ),
                (
                    "types/types_a.ytyp",
                    What::Types(vec![
                        ("int_local", room, 100.0),
                        ("int_world", room, 100.0),
                        // As the installs' type files state an interior: a box of no size.
                        ("int_big", cube([0.0, 0.0, 0.0], 0.0), 100.0),
                        ("prop_same", unit, 60.0),
                        ("prop_again", unit, 60.0),
                        ("prop_other", unit, 60.0),
                    ]),
                ),
            ],
        ),
        (
            "x64b.rpf",
            vec![(
                "types/types_b.ytyp",
                What::Types(vec![
                    ("prop_same", unit, 60.0),
                    ("prop_again", unit, 60.0),
                    ("prop_other", unit, 90.0),
                ]),
            )],
        ),
    ];
    let (_scratch, root) = install_of(&archives);
    let game = mount(&root);
    let index = built(&game, &root);
    assert_eq!(index.redefined(), 3);
    assert_eq!(
        stated_by_the_files(&game, &index),
        Stated {
            maps: 6,
            named_otherwise: 3,
            named_as_another_map: 2,
            with_parent: 3,
            parent_not_held: 1,
            redefined: 3,
            redefined_differently: 1,
            interior_bounds: 4,
            of_an_interior_with_no_size: 1,
            nearer_the_origin: 2,
            nearer_a_placement: 2,
            answered_at_a_position: 1,
        }
    );
}

/// The names of what the folder that holds `file` holds.
fn beside(file: &Path) -> Vec<String> {
    let folder = std::fs::read_dir(file.parent().expect("a folder")).expect("listed");
    let mut names: Vec<String> = folder
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn an_index_is_kept_as_one_file_and_one_that_cannot_be_kept_leaves_none() {
    let (scratch, root) = install();
    let file = kept_file(&scratch);
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let (_, report) = refresh(&source, &file);
    assert!(report.not_kept.is_none(), "{:?}", report.not_kept);
    assert_eq!(beside(&file), ["world.index"]);

    // A folder stands where the index is to be kept: the index answers, says it was not kept,
    // and leaves no half of itself beside the folder.
    std::fs::remove_file(&file).expect("removed");
    std::fs::create_dir(&file).expect("mkdir");
    std::fs::write(file.join("held"), b"so the folder is not empty").expect("written");
    let (index, report) = refresh(&source, &file);
    assert!(
        matches!(report.kept, KeptState::Refused(KeptError::Io(_))),
        "{:?}",
        report.kept
    );
    assert!(report.not_kept.is_some(), "kept over a folder");
    assert_eq!(index.maps().len(), 4);
    assert_eq!(beside(&file), ["world.index"]);
    assert!(file.is_dir());
}

/// A source whose files do not open: an archive the game holds open, or a key not loaded yet.
struct Shut<'a>(&'a dyn WorldSource);

impl WorldSource for Shut<'_> {
    fn identity(&self) -> String {
        self.0.identity()
    }
    fn archives(&self) -> Vec<Archive> {
        self.0.archives()
    }
    fn entries(&self, archive: usize, visit: &mut dyn FnMut(&or_title::vfs::EntryFacts)) {
        self.0.entries(archive, visit);
    }
    fn open(&self, _: usize, _: &str) -> Option<Vec<u8>> {
        None
    }
    fn open_head(&self, _: usize, _: &str, _: usize) -> Option<Vec<u8>> {
        None
    }
}

/// ⚠ A file that could not be opened once is not a file that does not read: an index kept from such a build is not trusted for that
/// archive, whose size and time say nothing changed.
#[test]
fn a_file_that_did_not_open_is_read_again_by_the_next_build_though_its_archive_stands() {
    let (scratch, root) = install();
    let file = kept_file(&scratch);
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);

    let (shut, _) = refresh(&Shut(&source), &file);
    let unread = |index: &WorldIndex| index.maps().iter().filter(|map| !map.read).count();
    assert!(unread(&shut) > 0 && unread(&shut) == shut.maps().len());
    assert!(shut.maps_at(centre(north())).is_empty());

    // A second start, on a mount whose files open.
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let (second, report) = refresh(&source, &file);
    let afresh = built(&game, &root);
    assert_eq!(unread(&second), unread(&afresh));
    assert_eq!(answers(&second), answers(&afresh));
    assert!(names(second.maps_at(centre(north()))).contains("m_north"));
    assert!(report.opened > 0, "the files were opened again");

    // And once every file opened, the archives are kept as before.
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let (_, report) = refresh(&source, &file);
    assert!(
        matches!(report.kept, KeptState::Current) && report.opened == 0,
        "{:?}, {} opened",
        report.kept,
        report.opened
    );
}

/// An install whose DLC list names `listed`, with each pack of `packs` on disk under its name, and a title update that patches the
/// pack `beta` from the folder `dlc_patch/beta/`, which holds `patch`.
fn patched_install(
    listed: &[&str],
    packs: &[(&str, Files)],
    patch: &Files,
) -> (tempfile::TempDir, PathBuf) {
    use or_title::{ContentXml, DlcList, ExtraTitleUpdateData, SetupData, TitleUpdateMount};

    let (scratch, root) = install_of(&[]);
    let paths = listed
        .iter()
        .map(|pack| format!("dlcpacks:/{pack}/").into());
    let list = DlcList {
        paths: paths.collect(),
    };
    let list = ("data/dlclist.xml", list.to_xml().into_bytes());
    write_archive(&root, "common.rpf", &[list]);
    for (order, (pack, files)) in packs.iter().enumerate() {
        let setup = SetupData {
            name_hash: pack.to_string(),
            device_name: format!("dlc_{pack}"),
            dat_file: "content.xml".into(),
            order: order as i32,
            ..Default::default()
        };
        let mut held = vec![
            ("setup2.xml", setup.to_xml().into_bytes()),
            ("content.xml", ContentXml::default().to_xml().into_bytes()),
        ];
        held.extend(
            files
                .iter()
                .map(|(path, what)| (*path, bytes_of(path, what))),
        );
        let folder = format!("x64/dlcpacks/{pack}");
        std::fs::create_dir_all(root.join(&folder)).expect("mkdir");
        write_archive(&root, &format!("{folder}/dlc.rpf"), &held);
    }
    // ⚠ In another case than the archive spells the folder, as a table may.
    let table = ExtraTitleUpdateData {
        mounts: vec![TitleUpdateMount {
            device_name: "dlc_beta".into(),
            path: "update:/DLC_Patch/beta/".into(),
        }],
    };
    let patched: Vec<(String, Vec<u8>)> = (patch.iter())
        .map(|(path, what)| (format!("dlc_patch/beta/{path}"), bytes_of(path, what)))
        .collect();
    let mut held = vec![(
        "common/data/ExtraTitleUpdateData.meta",
        table.to_xml().into_bytes(),
    )];
    held.extend(
        patched
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.clone())),
    );
    std::fs::create_dir_all(root.join("update")).expect("mkdir");
    write_archive(&root, "update/update.rpf", &held);
    (scratch, root)
}

/// ⚠ The title update mounts before every pack, so by the order of the archives alone the pack's own copy would be the last, and
/// the mount opens the patch's at the pack's path.
#[test]
fn a_file_the_title_update_patches_is_named_where_the_mount_opens_it() {
    let own = MapSpec {
        content_flags: 1,
        ..MapSpec::around(north())
    };
    let patched = MapSpec {
        content_flags: 2,
        ..MapSpec::around(south())
    };
    let later = MapSpec {
        content_flags: 4,
        ..MapSpec::around(group())
    };
    let unit = cube([-1.0, -1.0, -1.0], 2.0);
    let beta: Files = vec![
        ("levels/m_patched.ymap", What::Map(own.clone())),
        ("levels/m_plain.ymap", What::Map(own.clone())),
        ("levels/m_later.ymap", What::Map(own)),
        (
            "levels/m_types.ytyp",
            What::Types(vec![("prop_a", unit, 10.0)]),
        ),
        ("levels/m_patched_0.ybn", What::Bound(north())),
        (
            "levels/_manifest.ymf",
            What::Groups(vec![("m_patched", vec!["m_patched_0"])]),
        ),
    ];
    let patch: Files = vec![
        ("levels/m_patched.ymap", What::Map(patched.clone())),
        ("levels/m_later.ymap", What::Map(patched.clone())),
        (
            "levels/m_types.ytyp",
            What::Types(vec![("prop_a", unit, 20.0)]),
        ),
        ("levels/m_patched_0.ybn", What::Bound(south())),
        (
            "levels/_manifest.ymf",
            What::Groups(vec![("m_patched", vec!["m_patched_0", "hi@m_patched_0"])]),
        ),
    ];
    let gamma: Files = vec![("levels/m_later.ymap", What::Map(later))];
    let (scratch, root) = patched_install(
        &["beta", "gamma"],
        &[("beta", beta), ("gamma", gamma)],
        &patch,
    );
    let game = mount_all(&root);
    let place = |container: &str| {
        (game.roots.iter()).position(|root| root.container.replace('\\', "/") == container)
    };
    let (update, pack) = (
        place("update/update.rpf"),
        place("x64/dlcpacks/beta/dlc.rpf"),
    );
    assert!(
        update.is_some() && update < pack,
        "the update is mounted before the pack it patches: {update:?} {pack:?}"
    );
    let source = MountedWorld::new(&game, &root);
    let (index, _) = WorldIndex::build(&source, None);

    // The patched map is the patch's copy, by the path it is listed at, with what that copy states.
    let at_the_pack = "dlc_beta:/levels/m_patched.ymap";
    let map = index.map(hash("m_patched")).expect("m_patched");
    assert_eq!(map.logical, "update:/dlc_patch/beta/levels/m_patched.ymap");
    assert_eq!(Some(map.archive), update);
    assert_eq!(map.content_flags, 2);
    assert_eq!(map.streaming, Some(span(patched.streaming)));
    let opened = game.vfs.open(at_the_pack);
    assert!(opened.is_some() && opened == source.open(map.archive, &map.logical));
    assert_ne!(opened, source.open(pack.expect("the pack"), at_the_pack));

    // A file the patch does not hold is the pack's own, and a later pack's copy lies over the patch of an earlier one.
    let plain = index.map(hash("m_plain")).expect("m_plain");
    assert_eq!(plain.logical, "dlc_beta:/levels/m_plain.ymap");
    let later = index.map(hash("m_later")).expect("m_later");
    assert_eq!(later.logical, "dlc_gamma:/levels/m_later.ymap");
    assert_eq!(later.content_flags, 4);

    // A type file, a bound and a manifest follow the same rule.
    let types = index.type_file(hash("m_types")).expect("m_types");
    assert_eq!(types.logical, "update:/dlc_patch/beta/levels/m_types.ytyp");
    assert_eq!(
        index.archetype(hash("prop_a")).map(|a| a.lod_dist),
        Some(20.0)
    );
    assert_eq!(
        index.redefined(),
        0,
        "the pack's own type file was read as a second one"
    );
    let bound = index.bound(hash("m_patched_0")).expect("m_patched_0");
    assert_eq!(
        bound.logical,
        "update:/dlc_patch/beta/levels/m_patched_0.ybn"
    );
    assert_eq!(bound.span, Some(span(south())));
    assert_eq!(
        index.tables().static_bounds_of(hash("m_patched")),
        [hash("m_patched_0"), hash("hi@m_patched_0")]
    );

    // And everything the install suite checks holds of it, the second start from the kept file included.
    let held = held_to_its_mount(&|| mount_all(&root), &root, &kept_file(&scratch));
    assert_eq!((held.maps, held.type_files, held.bounds), (3, 1, 1));
}

/// ⚠ A DLC list can name a pack twice, and the mount then lists its archive twice under one device and one container.
#[test]
fn an_archive_the_mount_lists_twice_is_kept_both_times() {
    let beta: Files = vec![("levels/m_patched.ymap", What::Map(MapSpec::around(north())))];
    let (scratch, root) = patched_install(&["beta", "beta"], &[("beta", beta)], &Vec::new());
    let file = kept_file(&scratch);
    let game = mount_all(&root);
    let twice = (game.roots.iter())
        .filter(|root| root.container.replace('\\', "/") == "x64/dlcpacks/beta/dlc.rpf")
        .count();
    assert_eq!(twice, 2, "the fixture's mount lists the pack once");

    let (first, report) = refresh(&MountedWorld::new(&game, &root), &file);
    assert_eq!(report.read.len(), game.roots.len());
    for _ in 0..2 {
        let game = mount_all(&root);
        let (again, report) = refresh(&MountedWorld::new(&game, &root), &file);
        assert!(
            matches!(report.kept, KeptState::Current),
            "{:?}: read again {:?}",
            report.kept,
            report.read
        );
        assert_eq!(report.opened, 0);
        assert_eq!(answers(&again), answers(&first));
    }
}
