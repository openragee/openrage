//! The ground under a place of a mount built by hand: bounds files written as the game ships
//! them, found by the world index, read through the mount and asked as one.
//!
//! ⚠ Every height expected is the one the file holds for its vertices, which sixteen bits an axis
//! moved from where the fixture put them, and every file expected is named from the fixture.

mod world_fixture;

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::{Affine3A, Quat, Vec3};
use or_depends::world_collision::{collider_of, Collider, GroundAnswer, Streamed, Unread};
use or_depends::world_index::Archive;
use or_depends::{MountedWorld, WorldCollision, WorldIndex, WorldSource};
use or_io::gta5::{PhBoundAny, PhBoundGeometry};
use or_title::vfs::EntryFacts;
use or_world::collision::{BoundCollider, GroundProbe};
use world_fixture::{
    bound_bytes, cube, ground_at_the_road_nodes, level_square, manifest_bytes, map_bytes,
    mesh_bound, mesh_bound_bytes, mount, path_region_bytes, sphere_bytes, write_archive, MapSpec,
    NodesHeld, Span, NODE_SCALES,
};

const FLOOR: u64 = 0x21;
const ROOF: u64 = 0x22;
const HILL: u64 = 0x23;
const SHOP: u64 = 0x24;
const LEDGE: u64 = 0x25;
const PIER: u64 = 0x27;

/// More than any collider of the fixture comes to, so nothing is dropped.
const ROOMY: u64 = 1 << 30;

/// The town's ground: a floor a hundred metres square at 10, and a roof over part of it at 14.
fn town() -> PhBoundGeometry {
    let at = |x: f32, y: f32, z: f32| [x, y, z];
    mesh_bound(
        &[
            at(100.0, 100.0, 10.0),
            at(200.0, 100.0, 10.0),
            at(200.0, 200.0, 10.0),
            at(100.0, 200.0, 10.0),
            at(120.0, 120.0, 14.0),
            at(140.0, 120.0, 14.0),
            at(140.0, 140.0, 14.0),
            at(120.0, 140.0, 14.0),
        ],
        &[[0, 1, 2], [0, 2, 3], [4, 5, 6], [4, 6, 7]],
        &[FLOOR, ROOF],
        &[0, 0, 1, 1],
    )
}

/// A hill that stands two metres over a corner of the town's floor, in a file of its own.
fn hill() -> PhBoundGeometry {
    level_square([150.0, 150.0, 12.0], 30.0, HILL)
}

/// One triangle, which covers half of the box its file states.
fn ledge() -> PhBoundGeometry {
    mesh_bound(
        &[
            [300.0, 100.0, 5.0],
            [340.0, 100.0, 5.0],
            [300.0, 140.0, 5.0],
        ],
        &[[0, 1, 2]],
        &[LEDGE],
        &[0],
    )
}

/// A square that stands apart from everything else.
fn pier() -> PhBoundGeometry {
    level_square([400.0, 100.0, 7.0], 20.0, PIER)
}

/// A file whose header states a box high over the town, and a sphere of a radius that is no number.
fn broken() -> Vec<u8> {
    sphere_bytes(cube([180.0, 180.0, 50.0], 15.0), f32::NAN)
}

/// An interior's floor, about the interior's own origin.
fn shop() -> PhBoundGeometry {
    level_square([-10.0, -10.0, 0.0], 20.0, SHOP)
}

/// The height the file holds for vertex `vertex` of `g`.
fn stored_z(g: &PhBoundGeometry, vertex: usize) -> f32 {
    g.vertex_position(vertex).expect("a vertex")[2]
}

/// An install of two maps: a town a manifest names four bounds for and an interior's bound, and a
/// hill whose bound is named after it.
fn install() -> (tempfile::TempDir, PathBuf) {
    let scratch = tempfile::Builder::new()
        .prefix("or-depends-ground-")
        .tempdir()
        .expect("mkdir");
    let root = scratch.path().join("install");
    std::fs::create_dir(&root).expect("mkdir");
    let manifest = manifest_bytes(
        &[("m_town", &["m_town_0", "m_town_1", "m_town_2", "m_town_3"])],
        &[("v_int_shop", &["v_int_shop_0"])],
    );
    write_archive(&root, "common.rpf", &[("data/_manifest.ymf", manifest)]);
    let town_map = MapSpec {
        interiors: vec!["v_int_shop"],
        ..MapSpec::around(cube([100.0, 100.0, 0.0], 100.0))
    };
    // The hill's map places the same interior as the town's, which it stands inside.
    let hill_map = MapSpec {
        interiors: vec!["v_int_shop"],
        ..MapSpec::around(cube([150.0, 150.0, 0.0], 30.0))
    };
    write_archive(
        &root,
        "x64a.rpf",
        &[
            ("levels/m_town.ymap", map_bytes("m_town", &town_map)),
            ("levels/m_hill.ymap", map_bytes("m_hill", &hill_map)),
            ("levels/m_town_0.ybn", mesh_bound_bytes(town())),
            ("levels/m_town_1.ybn", mesh_bound_bytes(ledge())),
            ("levels/m_town_2.ybn", broken()),
            ("levels/m_town_3.ybn", mesh_bound_bytes(pier())),
            ("levels/m_hill_0.ybn", mesh_bound_bytes(hill())),
            ("levels/v_int_shop_0.ybn", mesh_bound_bytes(shop())),
            // An archetype's own collision, about its origin as the interior's is.
            (
                "props/prop_col.ybn",
                bound_bytes(cube([-1.0, -1.0, -1.0], 2.0)),
            ),
        ],
    );
    (scratch, root)
}

/// What a file is given back as, in place of what the mount holds.
#[derive(Clone)]
enum Spoiled {
    Absent,
    /// The front of the file alone, of this many bytes.
    Cut(usize),
}

/// A source that says which files were opened whole, and gives some back spoiled.
struct Reads<'a> {
    source: &'a dyn WorldSource,
    opened: RefCell<Vec<String>>,
    spoiled: HashMap<&'static str, Spoiled>,
}

impl<'a> Reads<'a> {
    fn new(source: &'a dyn WorldSource) -> Self {
        Self {
            source,
            opened: RefCell::default(),
            spoiled: HashMap::new(),
        }
    }

    /// How often the file whose stem is `stem` was opened whole.
    fn of(&self, stem: &str) -> usize {
        let leaf = format!("/{stem}.ybn");
        (self.opened.borrow().iter())
            .filter(|logical| logical.ends_with(&leaf))
            .count()
    }
}

impl WorldSource for Reads<'_> {
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
        let whole = self.source.open(archive, logical);
        let spoiled =
            (self.spoiled.iter()).find(|(stem, _)| logical.ends_with(&format!("/{stem}.ybn")));
        match spoiled {
            Some((_, Spoiled::Absent)) => None,
            Some((_, Spoiled::Cut(bytes))) => whole.map(|file| file[..*bytes].to_vec()),
            None => whole,
        }
    }

    fn open_head(&self, archive: usize, logical: &str, limit: usize) -> Option<Vec<u8>> {
        self.source.open_head(archive, logical, limit)
    }

    fn patch_of(&self, archive: usize) -> Option<String> {
        self.source.patch_of(archive)
    }
}

fn built(source: &dyn WorldSource) -> WorldIndex {
    WorldIndex::build(source, None).0
}

fn names(bounds: &[&or_depends::world_index::Bound]) -> Vec<String> {
    bounds.iter().map(|bound| bound.name.clone()).collect()
}

fn names_at(bounds: &[or_depends::world_index::BoundAt<'_>]) -> Vec<String> {
    bounds.iter().map(|at| at.bound.name.clone()).collect()
}

/// A probe from `rise` above a point down without end, taking a surface up to 45 degrees from level.
fn walker(rise: f32) -> GroundProbe {
    GroundProbe {
        max_rise: rise,
        max_drop: f32::INFINITY,
        max_slope: 45f32.to_radians(),
    }
}

/// The steepest surface the questions here take for ground, in radians from level.
const SLOPE: f32 = std::f32::consts::FRAC_PI_4;

/// Holds an answer to one surface: its height, its material and the file it is in.
#[track_caller]
fn assert_ground(answer: &GroundAnswer<'_>, x: f32, y: f32, z: f32, material: u64, file: &str) {
    let ground = answer.ground.as_ref().expect("ground");
    let at = ground.hit.position;
    assert!(
        (at - Vec3::new(x, y, z)).length() < 1e-3,
        "at {at} for {x} {y} {z}"
    );
    assert!((ground.hit.normal - Vec3::Z).length() < 1e-4);
    assert_eq!(ground.hit.material, Some(material));
    assert_eq!(ground.bound.map(|bound| bound.name.as_str()), Some(file));
    assert!(matches!(ground.collider, Collider::File(_)));
}

#[test]
fn a_point_over_a_floor_answers_its_height_its_material_and_its_file() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let index = built(&source);
    let mut world = WorldCollision::new(&index, &source, ROOMY);
    let (floor, roof) = (stored_z(&town(), 0), stored_z(&town(), 4));
    assert!((floor - 10.0).abs() < 1e-3 && (roof - 14.0).abs() < 1e-3);

    // Over the floor alone, from well above it.
    let open = world.ground(Vec3::new(190.0, 110.0, 40.0), &walker(0.0));
    assert_ground(&open, 190.0, 110.0, floor, FLOOR, "m_town_0");
    assert_eq!(names(&open.streamed.held), ["m_town_0"]);
    assert!(open.streamed.unread.is_empty());
    // The town's map places an interior, whose collision is named and is not looked in.
    assert_eq!(names(&open.streamed.unplaced), ["v_int_shop_0"]);
    // The file is named by the path the mount lists it under.
    let mut listed = None;
    game.for_each_file(|logical, _container, leaf| {
        if leaf == "m_town_0.ybn" {
            listed = Some(logical.to_string());
        }
    });
    let ground = open.ground.as_ref().expect("ground");
    assert!(listed.is_some());
    assert_eq!(ground.bound.map(|bound| bound.logical.clone()), listed);

    // Between the floor and the roof, the floor; from over the roof, the roof.
    let between = world.ground(Vec3::new(130.0, 130.0, 12.0), &walker(1.0));
    assert_ground(&between, 130.0, 130.0, floor, FLOOR, "m_town_0");
    let over = world.ground(Vec3::new(130.0, 130.0, 14.5), &walker(1.0));
    assert_ground(&over, 130.0, 130.0, roof, ROOF, "m_town_0");
    // A rise that reaches the roof from under it takes the roof.
    let reaching = world.ground(Vec3::new(130.0, 130.0, 12.0), &walker(3.0));
    assert_ground(&reaching, 130.0, 130.0, roof, ROOF, "m_town_0");
    // And a drop that stops short of the floor finds nothing, with the file still looked in.
    let short = GroundProbe {
        max_drop: 1.0,
        ..walker(0.0)
    };
    let hanging = world.ground(Vec3::new(190.0, 110.0, 40.0), &short);
    assert_eq!(hanging.ground, None);
    assert_eq!(names(&hanging.streamed.held), ["m_town_0"]);

    // A reach that is no length finds nothing, though the column would cut a longer one to itself.
    for no_length in [f32::NAN, -1.0] {
        for probe in [
            GroundProbe {
                max_rise: no_length,
                ..walker(0.0)
            },
            GroundProbe {
                max_drop: no_length,
                ..walker(0.0)
            },
        ] {
            let refused = world.ground(Vec3::new(190.0, 110.0, 40.0), &probe);
            assert_eq!(refused.ground, None, "{probe:?}");
            assert_eq!(names(&refused.streamed.held), ["m_town_0"]);
        }
    }

    // What is held answers a ray as it answers the ground.
    let (collider, hit) = (world.colliders())
        .ray_cast(Vec3::new(190.0, 110.0, 40.0), Vec3::NEG_Z, 100.0)
        .expect("the floor");
    assert_eq!(
        world.bound_of(*collider).map(|bound| bound.name.as_str()),
        Some("m_town_0")
    );
    assert!((hit.distance - (40.0 - floor)).abs() < 1e-3);
}

#[test]
fn a_point_with_no_height_answers_the_highest_ground_of_its_column() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let index = built(&source);
    let mut world = WorldCollision::new(&index, &source, ROOMY);
    let (floor, roof) = (stored_z(&town(), 0), stored_z(&town(), 4));

    let under_the_roof = world.highest_ground(130.0, 130.0, SLOPE);
    assert_ground(&under_the_roof, 130.0, 130.0, roof, ROOF, "m_town_0");
    let beside_it = world.highest_ground(145.0, 130.0, SLOPE);
    assert_ground(&beside_it, 145.0, 130.0, floor, FLOOR, "m_town_0");
    // A column that is nowhere holds no ground and looked in nothing.
    let nowhere = world.highest_ground(f32::NAN, 130.0, SLOPE);
    assert_eq!(nowhere.ground, None);
    assert_eq!(nowhere.streamed, Streamed::default());
}

#[test]
fn where_two_bounds_files_overlap_the_higher_surface_under_the_probe_answers_and_names_its_file() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let index = built(&source);
    let mut world = WorldCollision::new(&index, &source, ROOMY);
    let (floor, top) = (stored_z(&town(), 0), stored_z(&hill(), 0));
    assert!(top > floor + 1.0);

    let highest = world.highest_ground(160.0, 160.0, SLOPE);
    assert_ground(&highest, 160.0, 160.0, top, HILL, "m_hill_0");
    let both: BTreeSet<String> = names(&highest.streamed.held).into_iter().collect();
    assert_eq!(
        both,
        ["m_hill_0".to_string(), "m_town_0".to_string()].into()
    );

    // From between the two, the hill is over the probe and the town's floor is the ground.
    let between = world.ground(Vec3::new(160.0, 160.0, floor + 1.0), &walker(0.5));
    assert_ground(&between, 160.0, 160.0, floor, FLOOR, "m_town_0");
    let on_the_hill = world.ground(Vec3::new(160.0, 160.0, top + 0.5), &walker(0.5));
    assert_ground(&on_the_hill, 160.0, 160.0, top, HILL, "m_hill_0");
    // A metre past the hill's edge the town's file alone holds the column.
    let past = world.highest_ground(181.0, 160.0, SLOPE);
    assert_ground(&past, 181.0, 160.0, floor, FLOOR, "m_town_0");
    assert_eq!(names(&past.streamed.held), ["m_town_0"]);
}

#[test]
fn a_point_over_nothing_says_none_and_what_it_looked_in() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let index = built(&source);
    let mut world = WorldCollision::new(&index, &source, ROOMY);

    // In the box the ledge's file states, and past its one triangle.
    let beside = world.highest_ground(335.0, 135.0, SLOPE);
    assert_eq!(beside.ground, None);
    assert_eq!(names(&beside.streamed.held), ["m_town_1"]);
    let given_a_height = world.ground(Vec3::new(335.0, 135.0, 20.0), &walker(1.0));
    assert_eq!(given_a_height.ground, None);
    assert_eq!(names(&given_a_height.streamed.held), ["m_town_1"]);
    // The triangle itself is there to be found.
    let on_it = world.highest_ground(305.0, 105.0, SLOPE);
    assert_ground(
        &on_it,
        305.0,
        105.0,
        stored_z(&ledge(), 0),
        LEDGE,
        "m_town_1",
    );

    // Where no file's box reaches, nothing was looked in.
    let far = world.highest_ground(5000.0, 5000.0, SLOPE);
    assert_eq!(far.ground, None);
    assert_eq!(far.streamed, Streamed::default());
    // Under the floor there is no ground, and the floor's file was looked in.
    let under = world.ground(Vec3::new(190.0, 110.0, 5.0), &walker(1.0));
    assert_eq!(under.ground, None);
    assert_eq!(names(&under.streamed.held), ["m_town_0"]);
}

#[test]
fn a_bounds_file_that_does_not_read_is_named_and_the_answer_comes_from_the_others() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let index = built(&source);
    let floor = stored_z(&town(), 0);

    // A file that reads as a bound and states a sphere of no size.
    let reads = Reads::new(&source);
    let mut world = WorldCollision::new(&index, &reads, ROOMY);
    for asked in 1..=2 {
        let answer = world.highest_ground(185.0, 185.0, SLOPE);
        assert_ground(&answer, 185.0, 185.0, floor, FLOOR, "m_town_0");
        assert_eq!(names(&answer.streamed.held), ["m_town_0"]);
        let [(bound, why)] = &answer.streamed.unread[..] else {
            panic!("{:?}", answer.streamed.unread);
        };
        assert_eq!(bound.name, "m_town_2");
        assert!(
            matches!(why, Unread::NotACollider(_)),
            "question {asked}: {why}"
        );
        assert!(
            why.to_string().contains("not a finite size or place"),
            "{why}"
        );
    }
    assert_eq!(
        reads.of("m_town_2"),
        1,
        "a file that does not read is opened once"
    );
    assert_eq!(world.held_files(), 1);

    // A file the archive no longer gives, and one cut short.
    for (spoiled, expected) in [
        (Spoiled::Absent, "the archive that lists it gave no file"),
        (Spoiled::Cut(64), "it does not read as a bound"),
    ] {
        let mut reads = Reads::new(&source);
        reads.spoiled.insert("m_hill_0", spoiled);
        let mut world = WorldCollision::new(&index, &reads, ROOMY);
        let answer = world.highest_ground(160.0, 160.0, SLOPE);
        assert_ground(&answer, 160.0, 160.0, floor, FLOOR, "m_town_0");
        let [(bound, why)] = &answer.streamed.unread[..] else {
            panic!("{:?}", answer.streamed.unread);
        };
        assert_eq!(bound.name, "m_hill_0");
        assert!(why.to_string().starts_with(expected), "{why}");
    }
}

/// How many bytes the collider of the file whose stem is `stem` comes to.
fn bytes_of(index: &WorldIndex, source: &dyn WorldSource, stem: &str) -> u64 {
    let bound = index.bound(world_fixture::hash(stem)).expect(stem);
    collider_of(source, bound).expect("reads").size_in_bytes() as u64
}

#[test]
fn the_budget_drops_the_file_asked_for_longest_ago_and_a_later_question_reads_it_again() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let index = built(&source);
    // Three files that each hold a column of their own: the town, the ledge and the pier.
    const STEMS: [&str; 3] = ["m_town_0", "m_town_1", "m_town_3"];
    let sizes = STEMS.map(|stem| bytes_of(&index, &source, stem));
    let [town_bytes, ledge_bytes, pier_bytes] = sizes;
    assert!(
        town_bytes > pier_bytes && pier_bytes > ledge_bytes && ledge_bytes > 0,
        "{sizes:?}"
    );

    // Room for the town and the pier, so for any two of the three and not for all of them.
    let budget = town_bytes + pier_bytes;
    let reads = Reads::new(&source);
    let mut world = WorldCollision::new(&index, &reads, budget);
    let counts = |reads: &Reads| STEMS.map(|stem| reads.of(stem));
    let ask = |world: &mut WorldCollision, (x, y): (f32, f32), material: u64| {
        let answer = world.highest_ground(x, y, SLOPE);
        assert_eq!(answer.streamed.held.len(), 1, "one file a column");
        assert_eq!(answer.ground.and_then(|g| g.hit.material), Some(material));
        assert!(world.held_bytes() <= budget);
        // What the budget dropped is out of the set, not only out of the count.
        assert_eq!(world.colliders().len(), world.held_files());
    };
    let (town, ledge, pier) = ((190.0, 110.0), (305.0, 105.0), (410.0, 110.0));

    ask(&mut world, town, FLOOR);
    assert_eq!(counts(&reads), [1, 0, 0]);
    ask(&mut world, ledge, LEDGE);
    assert_eq!(counts(&reads), [1, 1, 0]);
    assert_eq!(world.held_bytes(), town_bytes + ledge_bytes);
    // Asked again, the town is answered from what is held, and is now the later of the two.
    ask(&mut world, town, FLOOR);
    assert_eq!(counts(&reads), [1, 1, 0]);

    // The pier does not fit beside both: the ledge, asked for longest ago, goes.
    ask(&mut world, pier, PIER);
    assert_eq!(counts(&reads), [1, 1, 1]);
    assert_eq!(world.held_files(), 2);
    assert_eq!(world.held_bytes(), town_bytes + pier_bytes);
    ask(&mut world, town, FLOOR);
    assert_eq!(counts(&reads), [1, 1, 1], "the town was never dropped");

    // The ledge is read again, and the pier, now the one asked for longest ago, goes for it.
    ask(&mut world, ledge, LEDGE);
    assert_eq!(counts(&reads), [1, 2, 1]);
    assert_eq!(world.held_bytes(), town_bytes + ledge_bytes);
    // Then the pier again for the town, and the town again for the ledge.
    ask(&mut world, pier, PIER);
    assert_eq!(counts(&reads), [1, 2, 2]);
    assert_eq!(world.held_bytes(), ledge_bytes + pier_bytes);
    ask(&mut world, town, FLOOR);
    assert_eq!(counts(&reads), [2, 2, 2]);
    assert_eq!(world.held_bytes(), town_bytes + pier_bytes);

    // With no room at all a question still holds what it asks for, until the next.
    let reads = Reads::new(&source);
    let mut world = WorldCollision::new(&index, &reads, 0);
    let hill = world.highest_ground(160.0, 160.0, SLOPE);
    assert_eq!(hill.ground.and_then(|g| g.hit.material), Some(HILL));
    assert_eq!(world.held_files(), 2, "the hill and the town under it");
    let ledge = world.highest_ground(ledge.0, ledge.1, SLOPE);
    assert_eq!(ledge.ground.and_then(|g| g.hit.material), Some(LEDGE));
    assert_eq!(world.held_files(), 1);
    assert_eq!(
        reads.of("m_hill_0") + reads.of("m_town_0") + reads.of("m_town_1"),
        3
    );
}

#[test]
fn an_interiors_collision_is_named_and_not_loaded_until_a_caller_places_it() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let index = built(&source);
    let reads = Reads::new(&source);
    let mut world = WorldCollision::new(&index, &reads, ROOMY);

    // The interior's file states its box about the interior's origin. That is no place of the
    // world: the index answers nothing there, and nothing of the interior is said of that column.
    let file = index.bound(world_fixture::hash("v_int_shop_0"));
    let span = file.and_then(|bound| bound.span);
    let (low, high) = span.expect("its file states a box");
    assert!(low.cmple(Vec3::ZERO).all() && high.cmpge(Vec3::ZERO).all());
    assert!(index.bounds_in_column(0.0, 0.0).is_empty());
    let origin = world.highest_ground(0.0, 0.0, SLOPE);
    assert_eq!(origin.ground, None);
    assert_eq!(origin.streamed, Streamed::default());
    // It is said where the map that places the interior streams, and past that map it is not.
    let in_town = world.stream(95.0, 95.0, 0.0);
    assert_eq!(names(&in_town.unplaced), ["v_int_shop_0"]);
    assert!(in_town.held.is_empty() && in_town.unread.is_empty());
    assert!(world.stream(335.0, 135.0, 0.0).unplaced.is_empty());
    // Two maps that stream over one column both place it, and its file is named once.
    assert_eq!(index.maps_in_column(160.0, 160.0).len(), 2);
    assert_eq!(
        names(&world.stream(160.0, 160.0, 0.0).unplaced),
        ["v_int_shop_0"]
    );
    assert_eq!(
        names(&world.stream(335.0, 135.0, 130.0).unplaced),
        ["v_int_shop_0"]
    );
    assert_eq!(
        reads.of("v_int_shop_0"),
        0,
        "what is not placed is not read"
    );
    assert_eq!(
        reads.of("prop_col"),
        0,
        "an archetype's own collision is not asked"
    );

    // A caller that knows where the interior stands reads its collider and places it.
    let shop_file = index
        .bound(world_fixture::hash("v_int_shop_0"))
        .expect("named");
    let collider = Arc::new(collider_of(&source, shop_file).expect("reads"));
    let stands = Vec3::new(110.0, 110.0, 30.0);
    world
        .place(7, collider, Affine3A::from_translation(stands))
        .expect("a translation places");
    let floor = stored_z(&town(), 0);
    let shop_floor = stands.z + stored_z(&shop(), 0);

    let placed = world.highest_ground(112.0, 112.0, SLOPE);
    let ground = placed.ground.as_ref().expect("the shop's floor");
    assert!((ground.hit.position - Vec3::new(112.0, 112.0, shop_floor)).length() < 1e-3);
    assert_eq!(ground.hit.material, Some(SHOP));
    assert_eq!(ground.collider, Collider::Placed(7));
    assert_eq!(ground.bound, None);
    // The files of the column are the town's alone: what was placed is no file of it.
    assert_eq!(names(&placed.streamed.held), ["m_town_0"]);
    // Under the shop, the town's floor.
    let under = world.ground(Vec3::new(112.0, 112.0, floor + 2.0), &walker(1.0));
    assert_ground(&under, 112.0, 112.0, floor, FLOOR, "m_town_0");

    assert!(world.displace(7));
    assert!(!world.displace(7));
    let gone = world.highest_ground(112.0, 112.0, SLOPE);
    assert_ground(&gone, 112.0, 112.0, floor, FLOOR, "m_town_0");
}

#[test]
fn a_floor_placed_at_any_scale_and_turn_is_the_highest_ground_of_its_column() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let index = built(&source);
    let mut world = WorldCollision::new(&index, &source, ROOMY);
    // A floor off its own origin in every axis, so no placement leaves its height a round number.
    let floor = level_square([-9.3, -8.1, 3.7], 20.0, SHOP);
    let local = stored_z(&floor, 0);
    let collider = Arc::new(
        BoundCollider::new(&PhBoundAny::PhBoundGeometry(floor)).expect("a floor is a collider"),
    );
    let mut dice = Dice(0x9_0f);
    for n in 0..400 {
        let scale = dice.between(0.3, 4.0);
        let stands = Vec3::new(
            dice.between(2000.0, 3000.0),
            dice.between(2000.0, 3000.0),
            dice.between(-50.0, 900.0),
        );
        let to_world = Affine3A::from_scale_rotation_translation(
            Vec3::splat(scale),
            Quat::from_rotation_z(dice.between(0.0, std::f32::consts::TAU)),
            stands,
        );
        world
            .place(n, Arc::clone(&collider), to_world)
            .expect("placed");
        // Over the middle of the floor, which a turn about the upright leaves over it. The probe
        // starts from the top of the collider's box and ends at its bottom, which are the floor.
        let over = to_world.transform_point3(Vec3::new(0.7, 1.9, local));
        let answer = world.highest_ground(over.x, over.y, SLOPE);
        let ground = answer.ground.as_ref().unwrap_or_else(|| {
            panic!("placement {n}: no ground at scale {scale} standing at {stands}")
        });
        assert_eq!(ground.collider, Collider::Placed(n));
        assert!((ground.hit.position.z - over.z).abs() < 2e-3, "{n}");
        let from_above = world.ground(Vec3::new(over.x, over.y, over.z + 5.0), &walker(0.0));
        assert_eq!(
            from_above.ground.map(|g| g.collider),
            Some(Collider::Placed(n))
        );
        assert!(world.displace(n));
    }
}

#[test]
fn a_radius_reads_the_files_whose_box_comes_within_it() {
    let (_scratch, root) = install();
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let index = built(&source);
    let mut world = WorldCollision::new(&index, &source, ROOMY);
    let held = |streamed: Streamed<'_>| -> BTreeSet<String> {
        names(&streamed.held).into_iter().collect()
    };
    // Between the town, which ends at x = 200, and the ledge, which starts at x = 300.
    assert_eq!(held(world.stream(250.0, 120.0, 10.0)), BTreeSet::new());
    assert_eq!(
        held(world.stream(250.0, 120.0, 50.0)),
        ["m_town_0".to_string(), "m_town_1".to_string()].into()
    );
    assert_eq!(
        held(world.stream(240.0, 120.0, 45.0)),
        ["m_town_0".to_string()].into()
    );
    assert_eq!(world.held_files(), 2);
}

const YARD: u64 = 0x31;
const PORCH: u64 = 0x32;
const LOT: u64 = 0x33;

/// The yard's ground, twenty metres square at 5.
fn yard() -> PhBoundGeometry {
    level_square([0.0, 0.0, 5.0], 20.0, YARD)
}

/// A porch that stands a metre over half of the yard, and past it.
fn porch() -> PhBoundGeometry {
    level_square([10.0, 10.0, 6.0], 20.0, PORCH)
}

/// A floor whose header states a box no place is in: its least corner is past its greatest.
fn lot() -> PhBoundGeometry {
    let mut floor = level_square([520.0, 520.0, 3.0], 20.0, LOT);
    floor.base.set_bounding_box_min([1.0e6; 3]);
    floor
}

/// An install of maps a manifest names bounds for: a yard, whose porch an interior is named for
/// as well, and far from it a lot, whose files state no box: one is no bound, and the other a
/// floor under a box that is none, which a plot inside the lot is named for too. An interior
/// that shares the lot's name is named for a third, which is no bound either.
fn yard_install() -> (tempfile::TempDir, PathBuf) {
    let scratch = tempfile::Builder::new()
        .prefix("or-depends-yard-")
        .tempdir()
        .expect("mkdir");
    let root = scratch.path().join("install");
    std::fs::create_dir(&root).expect("mkdir");
    let manifest = manifest_bytes(
        &[
            ("m_yard", &["m_yard_0", "m_yard_1"]),
            // The last is a name the mount holds no file for.
            ("m_lot", &["m_lot_0", "m_lot_1", "m_lot_2"]),
            ("m_plot", &["m_lot_1"]),
        ],
        &[("v_int_porch", &["m_yard_1"]), ("m_lot", &["m_lot_3"])],
    );
    write_archive(&root, "common.rpf", &[("data/_manifest.ymf", manifest)]);
    let yard_map = MapSpec {
        interiors: vec!["v_int_porch"],
        ..MapSpec::around(cube([0.0, 0.0, 0.0], 30.0))
    };
    let lot_map = MapSpec::around(cube([500.0, 500.0, 0.0], 60.0));
    let plot_map = MapSpec::around(cube([510.0, 510.0, 0.0], 40.0));
    write_archive(
        &root,
        "x64a.rpf",
        &[
            ("levels/m_yard.ymap", map_bytes("m_yard", &yard_map)),
            ("levels/m_lot.ymap", map_bytes("m_lot", &lot_map)),
            ("levels/m_plot.ymap", map_bytes("m_plot", &plot_map)),
            ("levels/m_yard_0.ybn", mesh_bound_bytes(yard())),
            ("levels/m_yard_1.ybn", mesh_bound_bytes(porch())),
            ("levels/m_lot_0.ybn", b"no bound at all".to_vec()),
            ("levels/m_lot_1.ybn", mesh_bound_bytes(lot())),
            (
                "levels/m_lot_3.ybn",
                b"an interior's, and no bound".to_vec(),
            ),
        ],
    );
    (scratch, root)
}

#[test]
fn a_bound_a_map_and_an_interior_both_carry_is_read_as_the_maps() {
    let (_scratch, root) = yard_install();
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let index = built(&source);
    let carried = index.bounds_in_column(25.0, 25.0);
    let [porch_at] = &carried[..] else {
        panic!("{carried:?}");
    };
    assert_eq!(porch_at.carriers.len(), 2, "a map group and an interior");

    let mut world = WorldCollision::new(&index, &source, ROOMY);
    let answer = world.highest_ground(25.0, 25.0, SLOPE);
    assert_ground(
        &answer,
        25.0,
        25.0,
        stored_z(&porch(), 0),
        PORCH,
        "m_yard_1",
    );
    assert_eq!(names(&answer.streamed.held), ["m_yard_1"]);
    // The yard's map places that interior, and the file is still the map's: read, and not left
    // for whoever places the interior.
    assert_eq!(
        names_at(&index.interior_bounds_near_column(25.0, 25.0, 0.0)),
        ["m_yard_1"]
    );
    assert!(answer.streamed.unplaced.is_empty() && answer.streamed.unread.is_empty());
}

#[test]
fn a_file_of_the_column_that_is_held_is_not_dropped_for_the_file_read_beside_it() {
    let (_scratch, root) = yard_install();
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let index = built(&source);
    let [yard_bytes, porch_bytes] = ["m_yard_0", "m_yard_1"].map(|s| bytes_of(&index, &source, s));
    assert_eq!(yard_bytes, porch_bytes);

    // Room for one file. Whichever of the two is held first, the column they share asks for
    // both, and the one held is of that question: it is kept, and neither is read twice.
    for alone in [(5.0, 5.0), (25.0, 25.0)] {
        let reads = Reads::new(&source);
        let mut world = WorldCollision::new(&index, &reads, yard_bytes);
        let first = world.highest_ground(alone.0, alone.1, SLOPE);
        assert_eq!(first.streamed.held.len(), 1);
        let both = world.highest_ground(15.0, 15.0, SLOPE);
        assert_ground(&both, 15.0, 15.0, stored_z(&porch(), 0), PORCH, "m_yard_1");
        assert_eq!(names(&both.streamed.held), ["m_yard_0", "m_yard_1"]);
        assert_eq!(
            (reads.of("m_yard_0"), reads.of("m_yard_1")),
            (1, 1),
            "held first at {alone:?}"
        );
        assert_eq!(world.held_files(), 2);
    }
}

#[test]
fn a_bound_whose_file_states_no_box_is_asked_wherever_its_map_stands() {
    let (_scratch, root) = yard_install();
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let index = built(&source);
    let stated = |stem: &str| index.bound(world_fixture::hash(stem)).expect(stem).span;
    assert_eq!(stated("m_lot_0"), None);
    let (least, greatest) = stated("m_lot_1").expect("a box, though none a place is in");
    assert!(least.cmpgt(greatest).all());
    // No box holds a place of the lot, so the index answers no bound by its box there.
    assert!(index.bounds_in_column(530.0, 530.0).is_empty());

    let reads = Reads::new(&source);
    let mut world = WorldCollision::new(&index, &reads, ROOMY);
    for _ in 0..2 {
        // The floor is read and answers, and the file that is no bound is named. The lot and
        // the plot both stream here and both carry the floor, which is one file all the same.
        assert_eq!(index.maps_in_column(530.0, 530.0).len(), 2);
        let over = world.highest_ground(530.0, 530.0, SLOPE);
        assert_ground(&over, 530.0, 530.0, stored_z(&lot(), 0), LOT, "m_lot_1");
        assert_eq!(names(&over.streamed.held), ["m_lot_1"]);
        // What an interior of the lot's name carries is no file of the map of that name.
        assert_eq!(
            names_at(&index.boxless_bounds_near_column(530.0, 530.0, 0.0)),
            ["m_lot_0", "m_lot_1"]
        );
        assert!(over.streamed.unplaced.is_empty());
        let [(bound, why)] = &over.streamed.unread[..] else {
            panic!("{:?}", over.streamed.unread);
        };
        assert_eq!(bound.name, "m_lot_0");
        assert!(matches!(why, Unread::NotABound(_)), "{why}");
    }
    assert_eq!((reads.of("m_lot_0"), reads.of("m_lot_1")), (1, 1));
    // Anywhere the lot's map streams they are asked, where the floor is and where it is not.
    let beside = world.ground(Vec3::new(500.0, 500.0, 50.0), &walker(0.0));
    assert_eq!(beside.ground, None);
    assert_eq!(names(&beside.streamed.held), ["m_lot_1"]);
    assert_eq!(beside.streamed.unread.len(), 1);
    // Past where it streams they are not, short of a radius that reaches it.
    let yard_alone = world.highest_ground(5.0, 5.0, SLOPE);
    assert_eq!(names(&yard_alone.streamed.held), ["m_yard_0"]);
    assert!(yard_alone.streamed.unread.is_empty());
    let far = world.stream(5.0, 5.0, 100.0);
    assert!(far.unread.is_empty(), "{:?}", far.unread);
    let reaching = world.stream(5.0, 5.0, 800.0);
    // Each list is in the mount's order, whichever way a file was found.
    assert_eq!(names(&reaching.held), ["m_lot_1", "m_yard_0", "m_yard_1"]);
    assert_eq!(reaching.unread.len(), 1);
    assert_eq!((reads.of("m_lot_0"), reads.of("m_lot_1")), (1, 1));
}

/// A repeatable stream of numbers in `0..1`.
struct Dice(u64);

impl Dice {
    fn between(&mut self, low: f32, high: f32) -> f32 {
        self.0 = (self.0)
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        low + (high - low) * ((self.0 >> 40) as f32 / (1u64 << 24) as f32)
    }

    fn span(&mut self, widest: f32) -> Span {
        let corner = [
            self.between(-600.0, 600.0),
            self.between(-600.0, 600.0),
            self.between(-100.0, 600.0),
        ];
        let size = [
            self.between(2.0, widest),
            self.between(2.0, widest),
            self.between(2.0, 80.0),
        ];
        (corner, [0, 1, 2].map(|axis| corner[axis] + size[axis]))
    }
}

/// How far the rectangle a box stands on is from a point of the plane.
fn rectangle_distance((min, max): Span, x: f32, y: f32) -> f32 {
    (x.clamp(min[0], max[0]) - x).hypot(y.clamp(min[1], max[1]) - y)
}

/// Files of one kind, each by its name and the box it states.
type Named = Vec<(String, Span)>;

/// An install of maps and bounds scattered at random, and each one's name and box; a bound is
/// carried, by a map's name, where its own name says so.
fn scattered(root: &Path) -> (Named, Named) {
    let mut dice = Dice(0x67_70);
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let (mut maps, mut bounds) = (Vec::new(), Vec::new());
    for n in 0..40 {
        let name = format!("m_scatter{n}");
        let entities = dice.span(500.0);
        let spec = MapSpec::around(entities);
        files.push((format!("levels/{name}.ymap"), map_bytes(&name, &spec)));
        maps.push((name, spec.streaming));
    }
    for n in 0..300 {
        // Every fifth is named after no map, so nothing carries it.
        let name = match n % 5 {
            4 => format!("prop_scatter{n}x"),
            _ => format!("m_scatter{}_{n}", n % 40),
        };
        let span = dice.span(if n % 9 == 0 { 700.0 } else { 90.0 });
        files.push((format!("levels/{name}.ybn"), bound_bytes(span)));
        bounds.push((name, span));
    }
    let files: Vec<(&str, Vec<u8>)> = (files.iter())
        .map(|(path, bytes)| (path.as_str(), bytes.clone()))
        .collect();
    write_archive(root, "x64a.rpf", &files);
    (maps, bounds)
}

#[test]
fn a_column_answers_the_maps_and_the_bounds_a_scan_of_every_entry_answers() {
    let scratch = tempfile::Builder::new()
        .prefix("or-depends-column-")
        .tempdir()
        .expect("mkdir");
    let root = scratch.path().join("install");
    std::fs::create_dir(&root).expect("mkdir");
    let (maps, bounds) = scattered(&root);
    let no_manifest = manifest_bytes(&[], &[]);
    write_archive(&root, "common.rpf", &[("data/_manifest.ymf", no_manifest)]);
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let index = built(&source);
    assert_eq!(index.maps().len(), maps.len());
    assert_eq!(index.bounds().len(), bounds.len());
    let carried = |name: &str| name.starts_with("m_scatter");
    assert_eq!(
        index.carried_bounds(),
        bounds.iter().filter(|(name, _)| carried(name)).count()
    );

    let scan = |all: &[(String, Span)], x: f32, y: f32, radius: f32| -> BTreeSet<String> {
        (all.iter())
            .filter(|(_, span)| rectangle_distance(*span, x, y) <= radius)
            .map(|(name, _)| name.clone())
            .collect()
    };
    let carried_bounds: Vec<(String, Span)> = (bounds.iter())
        .filter(|(name, _)| carried(name))
        .cloned()
        .collect();
    let mut dice = Dice(0x0c01);
    let (mut held_maps, mut held_bounds, mut stacked) = (0usize, 0usize, 0usize);
    for _ in 0..400 {
        let (x, y) = (dice.between(-700.0, 1000.0), dice.between(-700.0, 1000.0));
        let found: BTreeSet<String> = (index.maps_in_column(x, y).iter())
            .map(|map| map.name.clone())
            .collect();
        assert_eq!(found, scan(&maps, x, y, 0.0), "maps of {x} {y}");
        held_maps += found.len();

        let found: BTreeSet<String> = (index.bounds_in_column(x, y).iter())
            .map(|at| at.bound.name.clone())
            .collect();
        assert_eq!(found, scan(&carried_bounds, x, y, 0.0), "bounds of {x} {y}");
        held_bounds += found.len();
        // A column holds what any of its points does.
        let at_a_height = index.bounds_at(Vec3::new(x, y, 250.0));
        assert!(at_a_height.iter().all(|at| found.contains(&at.bound.name)));
        stacked += usize::from(found.len() > at_a_height.len());

        let near: BTreeSet<String> = (index.bounds_near_column(x, y, 120.0).iter())
            .map(|at| at.bound.name.clone())
            .collect();
        assert_eq!(
            near,
            scan(&carried_bounds, x, y, 120.0),
            "bounds near {x} {y}"
        );
    }
    assert!(
        held_maps > 200 && held_bounds > 200 && stacked > 100,
        "{held_maps} maps and {held_bounds} bounds were answered, {stacked} columns held more \
         than one of their points"
    );
}

/// Two roads a hundred metres wide at 10 that cross: one along X and one along Y, so a place
/// moved along either axis alone stays on a road.
fn crossroads() -> PhBoundGeometry {
    mesh_bound(
        &[
            [0.0, 100.0, 10.0],
            [1000.0, 100.0, 10.0],
            [1000.0, 200.0, 10.0],
            [0.0, 200.0, 10.0],
            [100.0, 0.0, 10.0],
            [200.0, 0.0, 10.0],
            [200.0, 1000.0, 10.0],
            [100.0, 1000.0, 10.0],
        ],
        &[[0, 1, 2], [0, 2, 3], [4, 5, 6], [4, 6, 7]],
        &[FLOOR],
        &[0, 0, 0, 0],
    )
}

#[test]
fn what_the_install_suite_checks_of_road_nodes_holds_of_a_mount_built_by_hand() {
    let scratch = tempfile::Builder::new()
        .prefix("or-depends-nodes-")
        .tempdir()
        .expect("mkdir");
    let root = scratch.path().join("install");
    std::fs::create_dir(&root).expect("mkdir");
    let manifest = manifest_bytes(&[("m_town", &["m_town_0"])], &[]);
    write_archive(&root, "common.rpf", &[("data/_manifest.ymf", manifest)]);
    // What a node stores for a place, at the first of the scales: four units a metre across
    // the ground and thirty-two in height.
    let (across, up) = NODE_SCALES[0];
    let stored = |x: f32, y: f32, z: f32| [x * across, y * across, z * up].map(|v| v as i16);
    let on_the_road = [
        stored(150.0, 150.0, 10.0),
        // Half a metre over the road, and half a metre under it.
        stored(120.0, 180.0, 10.5),
        stored(180.0, 120.0, 9.5),
    ];
    let off_it = [
        // Twenty metres over the road, and where no bounds file reaches.
        stored(150.0, 150.0, 30.0),
        stored(2000.0, 2000.0, 10.0),
    ];
    let car: Vec<[i16; 3]> = on_the_road.iter().chain(&off_it).copied().collect();
    // A pedestrian node on the road, which is not asked.
    let ped = [stored(160.0, 160.0, 10.0)];
    let town_map = MapSpec::around(cube([0.0, 0.0, 0.0], 1000.0));
    write_archive(
        &root,
        "x64a.rpf",
        &[
            ("levels/m_town.ymap", map_bytes("m_town", &town_map)),
            ("levels/m_town_0.ybn", mesh_bound_bytes(crossroads())),
            ("paths/nodes7.ynd", path_region_bytes(&car, &ped)),
            ("paths/nodes8.ynd", b"no path region".to_vec()),
        ],
    );
    let game = mount(&root);
    let source = MountedWorld::new(&game, &root);
    let index = built(&source);

    let held = ground_at_the_road_nodes(&game, &index, &source, NODE_SCALES[0], 1);
    assert_eq!(
        held,
        NodesHeld {
            regions: 2,
            unread: 1,
            asked: car.len(),
            no_file: 1,
            within_a_step: on_the_road.len(),
            unread_bounds: 0,
        }
    );
    // Every other node: the first, the third and the fifth.
    let sampled = ground_at_the_road_nodes(&game, &index, &source, NODE_SCALES[0], 2);
    assert_eq!((sampled.asked, sampled.within_a_step), (3, 2));
    // Read at any other scale, the same nodes stand off both roads or far from their height.
    for rival in &NODE_SCALES[1..] {
        let held = ground_at_the_road_nodes(&game, &index, &source, *rival, 1);
        assert_eq!(
            (held.asked, held.within_a_step),
            (car.len(), 0),
            "{rival:?}"
        );
    }
}
