//! What GTA V's world states that no rule of the kit asks and no comparison with the forest
//! sees: a map's gate by the weather, a drawable as the model its file states, the pieces of a
//! bound a box holds, a placement row, the link of an interior's entity, an instance's sphere,
//! and a pack the title update patches.

mod compare;
mod forest_fixture;
mod hand_built;
mod world_fixture;

use std::path::Path;
use std::sync::Arc;

use glam::{Quat, Vec3};
use or_job::{Cancel, Progress};
use or_world::caps::Capability;
use or_world::collision::OrientedBox;
use or_world::contract::{Keys, Opening, Worker, WorldInstall, WorldSpec, WorldTitle};
use or_world::driver::Driver;
use or_world::kept;
use or_world::model::{
    interior_key, AssetKey, Element, ElementFormat, Feed, FeedName, Gate, Index, Mesh, Model,
    Moment, PlacementRow, Semantic, SwitchId, Switched, EVERY_HOUR,
};
use or_world::streaming::{NodeState, ParentRef, View};
use or_world_gta5::state::{flags, NOT_BY_DEFAULT};
use or_world_gta5::WorldState;
use or_world_gta5::{Gta5Title, Gta5World, Linked, BOX, LEGACY, TITLE, WORDS};
use or_world_gta5::{MountedWorld, WorldIndex};

use forest_fixture::{
    arch, ent, install, manifest_bytes, map_around, map_file, types_file, Arch, Ent, Files, MapOf,
};
use world_fixture::{cube, dlc_list_bytes, hash, mount, mount_all, write_archive, write_pack};

/// The world of the install at `root` with no DLC, its index through the kept file under `scratch`, and the index `or_depends` builds of the same mount by itself.
fn opened(root: &Path, scratch: &Path) -> (Arc<Gta5World>, Arc<Index>, WorldIndex) {
    let world = Arc::new(Gta5World::over(mount(root), root.to_path_buf()));
    let spec = WorldSpec::of(LEGACY, root);
    let file = kept::file_for(scratch, TITLE, &spec);
    let install: &dyn WorldInstall = &*world;
    let (index, _) = kept::refresh(TITLE, &spec, install, &file).expect("the index builds");
    let (reference, _) = WorldIndex::build(&MountedWorld::new(world.game(), root), None);
    (world, Arc::new(index), reference)
}

/// The kit's scene, opened.
fn scene() -> (
    tempfile::TempDir,
    Arc<Gta5World>,
    Arc<Index>,
    tempfile::TempDir,
) {
    let dir = install(&[("x64a.rpf", hand_built::files())]);
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let (world, index, _) = opened(dir.path(), scratch.path());
    (dir, world, index, scratch)
}

fn key(name: &str) -> AssetKey {
    AssetKey(u64::from(hash(name)))
}

/// What the world's model reader gives for `asks`.
fn models(world: &Gta5World, index: &Index, asks: &[AssetKey]) -> Vec<(AssetKey, Model)> {
    let (mut out, mut worker) = (Vec::new(), Worker::new());
    WorldInstall::models(world).models(index, asks, &mut worker, &mut out);
    out
}

/// A map of one bench at `x` that only a request or its group lets in.
fn held_back(x: f32) -> MapOf {
    MapOf {
        flags: NOT_BY_DEFAULT,
        ..map_around(
            cube([x - 10.0, -10.0, -10.0], 20.0),
            vec![ent("bench", [x, 0.0, 0.0])],
        )
    }
}

#[test]
fn a_map_its_group_gates_by_the_weather_comes_in_under_the_weathers_the_group_names() {
    let both = flags::TIMED | flags::WEATHER;
    let files: Files = vec![
        map_file("m_rain", &held_back(0.0)),
        map_file("m_storm", &held_back(1000.0)),
        map_file("m_never", &held_back(2000.0)),
        types_file("props", &[arch("bench", 80.0)]),
        (
            "levels/_manifest.ymf".to_owned(),
            manifest_bytes(&[
                ("m_rain", flags::WEATHER, 0, &["RAIN"]),
                (
                    "m_storm",
                    both,
                    hand_built::NIGHT_HOURS,
                    &["RAIN", "THUNDER"],
                ),
                ("m_never", flags::WEATHER, 0, &[]),
            ]),
        ),
    ];
    let dir = install(&[("x64a.rpf", files)]);
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let (world, index, reference) = opened(dir.path(), scratch.path());
    let weather = |name: &str| u64::from(WorldState::at(12, name).weather);
    let gate = |map: &str| {
        let id = index.named(u64::from(hash(map))).expect("a row");
        index.map(id).expect("a row").gate.clone()
    };
    assert_eq!(
        gate("m_rain"),
        Gate::When {
            hours: EVERY_HOUR,
            weathers: vec![weather("RAIN")]
        }
    );
    assert_eq!(
        gate("m_storm"),
        Gate::When {
            hours: hand_built::NIGHT_HOURS,
            weathers: vec![weather("RAIN"), weather("THUNDER")]
        }
    );
    // A group by the weather that names none lets its map in under no weather.
    assert_eq!(
        gate("m_never"),
        Gate::Held {
            switch: Some(SwitchId(hash("m_never")))
        }
    );
    let asked = [
        (0.0, 12, "RAIN", 1),
        (0.0, 12, "CLEAR", 0),
        (1000.0, 22, "RAIN", 1),
        (1000.0, 22, "THUNDER", 1),
        (1000.0, 12, "RAIN", 0),
        (1000.0, 22, "CLEAR", 0),
        (2000.0, 12, "RAIN", 0),
        (2000.0, 22, "CLEAR", 0),
    ];
    for (x, hour, under, draws) in asked {
        let eye = Vec3::new(x, 1.0, 1.0);
        let drawn = compare::compared(&world, &index, &reference, eye, (hour, under));
        assert_eq!(drawn, Ok(draws), "at {x}, {hour} h, {under}");
    }
}

#[test]
fn a_drawable_of_its_own_file_reads_as_the_levels_the_vertices_and_the_triangles_it_states() {
    let (_dir, world, index, _scratch) = scene();
    let read = models(&world, &index, &[key("hd_a"), key("lod_p")]);
    assert_eq!(
        read.iter().map(|(ask, _)| *ask).collect::<Vec<_>>(),
        [key("hd_a"), key("lod_p")]
    );
    let levels: Vec<usize> = read.iter().map(|(_, model)| model.lods.len()).collect();
    assert_eq!(levels, [2, 1]);
    // The file's own bytes, read back as the declared drawable, say what the model must.
    let bytes = hand_built::drawable_bytes("hd_a", 2);
    let stated =
        <or_io::gta5::GtaDrawable<or_io::decl::Gen8> as or_io::decl::File>::from_bytes(&bytes)
            .expect("the drawable reads");
    let geometry = &stated.base.lod_group.lods[0].models[0].geometries[0];
    let element = |semantic, format, offset| Element {
        semantic,
        format,
        offset,
    };
    for (level, lod) in read[0].1.lods.iter().enumerate() {
        assert_eq!(lod.distance, hand_built::LOD_DISTANCES[level]);
        let [mesh] = &lod.meshes[..] else {
            panic!("{} meshes at level {level}", lod.meshes.len());
        };
        assert_eq!(mesh.layout.stride, 32);
        assert_eq!(
            mesh.layout.elements,
            [
                element(Semantic::Position, ElementFormat::F32x3, 0),
                element(Semantic::Normal, ElementFormat::F32x3, 12),
                element(Semantic::TexCoord(0), ElementFormat::F32x2, 24),
            ]
        );
        assert_eq!(mesh.vertices, geometry.vertex_buffer.data);
        let stated: Vec<u32> = (geometry.index_buffer.indices.iter())
            .map(|index| u32::from(*index))
            .collect();
        assert_eq!(mesh.indices, stated);
        assert_eq!(mesh.indices, [0, 1, 2]);
    }
}

/// The corners of `mesh` as places, where its position is three floats at the front of a vertex.
fn corners(mesh: &Mesh) -> Vec<Vec3> {
    let stride = mesh.layout.stride as usize;
    let number = |bytes: &[u8]| f32::from_le_bytes(bytes.try_into().expect("four bytes"));
    (mesh.vertices.chunks_exact(stride))
        .map(|v| Vec3::new(number(&v[0..4]), number(&v[4..8]), number(&v[8..12])))
        .collect()
}

#[test]
fn the_box_is_a_closed_cube_of_side_one() {
    let files: Files = vec![types_file("props", &[arch("no_file", 80.0)])];
    let dir = install(&[("x64a.rpf", files)]);
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let (world, index, _) = opened(dir.path(), scratch.path());
    let read = models(&world, &index, &[key("no_file"), BOX]);
    let [(BOX, cube)] = &read[..] else {
        panic!("{:?}", read.iter().map(|(ask, _)| *ask).collect::<Vec<_>>());
    };
    let [lod] = &cube.lods[..] else {
        panic!("{} levels", cube.lods.len());
    };
    let [mesh] = &lod.meshes[..] else {
        panic!("{} meshes", lod.meshes.len());
    };
    let corners = corners(mesh);
    assert_eq!(corners.len(), 8);
    assert!(corners.iter().all(|c| c.abs() == Vec3::splat(0.5)));
    assert_eq!(mesh.indices.len(), 36);
    // Every triangle faces out of the cube, and together they cover its six faces of area 1.
    let mut area = 0.0;
    for triangle in mesh.indices.as_chunks::<3>().0 {
        let [a, b, c] = triangle.map(|corner| corners[corner as usize]);
        let normal = (b - a).cross(c - a);
        assert!(normal.dot(a + b + c) > 0.0, "{triangle:?} faces inwards");
        area += normal.length() / 2.0;
    }
    assert_eq!(area, 6.0);
}

#[test]
fn the_last_copy_of_a_drawable_in_the_mounts_order_is_the_one_read() {
    let first: Files = vec![
        types_file("props", &[arch("bench", 80.0)]),
        (
            "models/bench.ydr".to_owned(),
            hand_built::drawable_bytes("bench", 2),
        ),
    ];
    let second: Files = vec![(
        "models/bench.ydr".to_owned(),
        hand_built::drawable_bytes("bench", 1),
    )];
    let dir = install(&[("x64a.rpf", first), ("x64b.rpf", second)]);
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let (world, index, reference) = opened(dir.path(), scratch.path());
    let files = reference.files();
    let last = (files.iter())
        .rfind(|(_, logical)| logical.ends_with("bench.ydr"))
        .expect("a copy");
    let levels = |bytes: &[u8]| {
        let read =
            <or_io::gta5::GtaDrawable<or_io::decl::Gen8> as or_io::decl::File>::from_bytes(bytes);
        let lods = read.expect("the drawable reads").base.lod_group.lods;
        lods.iter().filter(|lod| !lod.models.is_empty()).count()
    };
    let stated = levels(&world.source().open(last.0, last.1).expect("its bytes"));
    let read = models(&world, &index, &[key("bench")]);
    assert_eq!(read[0].1.lods.len(), stated);
    assert_eq!(stated, 1);
}

#[test]
fn a_box_about_the_floor_holds_pieces_of_its_bound_by_name_and_a_box_elsewhere_none() {
    let (_dir, world, index, _scratch) = scene();
    let install: &dyn WorldInstall = &*world;
    let mut worker = Worker::new();
    let about = |centre: Vec3| OrientedBox {
        centre,
        rotation: Quat::IDENTITY,
        half_extents: Vec3::splat(2.0),
    };
    let floor = Vec3::new(-5000.0, 0.0, hand_built::FLOOR);
    let held = install
        .overlap(&index, &about(floor), &mut worker)
        .expect("a world that states a collision query");
    let [(name, pieces)] = &held[..] else {
        panic!("{} bounds", held.len());
    };
    assert_eq!(*name, u64::from(hash("m_yard_0")));
    assert!(pieces.inside.len() + pieces.crossing.len() > 0);
    let above = install.overlap(&index, &about(floor + Vec3::Z * 30.0), &mut worker);
    assert_eq!(above.map(|held| held.len()).ok(), Some(0));
    let away = install.overlap(&index, &about(hand_built::NOWHERE), &mut worker);
    assert_eq!(away.map(|held| held.len()).ok(), Some(0));
    // A ray that stops short of the floor meets nothing, and one that reaches it meets it there.
    let from = floor + Vec3::Z * 5.0;
    let short = install.ray(&index, from, Vec3::NEG_Z, 4.0, &mut worker);
    assert_eq!(short.map(|hit| hit.is_none()).ok(), Some(true));
    let long = install.ray(&index, from, Vec3::NEG_Z, 6.0, &mut worker);
    let hit = long.ok().flatten().expect("the floor");
    assert!((hit.position - floor).length() < 1e-3, "{:?}", hit.position);
    assert!((hit.distance - 5.0).abs() < 1e-3 && hit.normal.z > 0.99);
}

#[test]
fn a_placement_row_states_the_entity_by_its_place_its_archetype_and_where_it_stands() {
    let (_dir, world, index, _scratch) = scene();
    let install: &dyn WorldInstall = &*world;
    let rows_of = |map: &str| {
        let id = index.named(u64::from(hash(map))).expect("a row");
        let mut rows = Vec::new();
        let read = install.indexer().placements(&index, id, &mut rows);
        read.map(|()| rows).map_err(|e| e.to_string())
    };
    let row = |node, archetype: &str, position| PlacementRow {
        node,
        archetype: u64::from(hash(archetype)),
        position,
    };
    assert_eq!(
        rows_of("m_hd"),
        Ok(vec![
            row(0, "hd_a", Vec3::new(150.0, 0.0, 0.0)),
            row(1, "hd_b", Vec3::new(-150.0, 0.0, 0.0)),
        ])
    );
    assert_eq!(
        rows_of("m_yard"),
        Ok(vec![row(0, "shop", Vec3::new(-5000.0, 0.0, 5.0))])
    );
}

#[test]
fn an_entity_of_an_interior_stands_under_the_instance_or_under_its_own_parent_there() {
    let inside = vec![
        Ent {
            lod_dist: 70.0,
            num_children: 1,
            ..ent("lamp", [1.0, 0.0, 0.0])
        },
        Ent {
            parent: Some((0, false)),
            lod_dist: 30.0,
            ..ent("bulb", [1.0, 0.0, 1.0])
        },
        ent("lamp", [3.0, 0.0, 0.0]),
    ];
    let types = [
        arch("lamp", 120.0),
        arch("bulb", 120.0),
        Arch {
            interior: Some(inside),
            ..arch("flat", 80.0)
        },
    ];
    let placing = map_around(
        cube([-50.0; 3], 100.0),
        vec![
            ent("lamp", [0.0, 0.0, 0.0]),
            Ent {
                interior: true,
                lod_dist: 80.0,
                ..ent("flat", [10.0, 10.0, 0.0])
            },
        ],
    );
    let files: Files = vec![map_file("m_flat", &placing), types_file("flat", &types)];
    let dir = install(&[("x64a.rpf", files)]);
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let (world, index, _) = opened(dir.path(), scratch.path());
    let install: &dyn WorldInstall = &*world;
    let map = index.named(u64::from(hash("m_flat"))).expect("a row");
    let mut worker = Worker::new();
    let body = install.maps().read(&index, map, &mut worker);
    let body = body.unwrap_or_else(|e| panic!("{e}"));
    let [placed] = body.interiors() else {
        panic!("{} interiors", body.interiors().len());
    };
    assert_eq!((placed.node, placed.interior), (1, u64::from(hash("flat"))));
    let within = (install.maps()).interior(&index, map, &*body, placed, &mut worker);
    let within = within.unwrap_or_else(|e| panic!("{e}"));
    let links: Vec<_> = (within.nodes().iter())
        .map(|node| (node.parent, node.instance))
        .collect();
    let under = ParentRef {
        map: map.key(),
        node: 1,
    };
    let own = ParentRef {
        map: interior_key(map, 1),
        node: 0,
    };
    assert_eq!(
        links,
        [
            (Some(under), Some(0)),
            (Some(own), None),
            (Some(under), Some(2))
        ]
    );
}

#[test]
fn an_instance_is_laid_with_the_sphere_of_its_node_and_a_box_with_the_sphere_about_its_cube() {
    let scaled = Ent {
        scale: 4.0,
        ..ent("nowhere_declared", [10.0, 0.0, 0.0])
    };
    let map = map_around(
        cube([-50.0; 3], 100.0),
        vec![ent("bench", [0.0, 0.0, 0.0]), scaled],
    );
    let files: Files = vec![
        map_file("m_two", &map),
        types_file("props", &[arch("bench", 80.0)]),
    ];
    let dir = install(&[("x64a.rpf", files)]);
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let (world, index, _) = opened(dir.path(), scratch.path());
    let install: Arc<dyn WorldInstall> = world.clone();
    let mut driver = Driver::new(install, Arc::clone(&index));
    let (view, noon) = (View::at(Vec3::new(5.0, 0.0, 1.0), 1.0), Moment::noon(0));
    driver.stream(&view, &noon, &Switched::default(), None, usize::MAX);
    let selected = driver.select(&view, &noon, &|_| NodeState::Ready);
    let of = &selected.instances;
    assert_eq!((of.len(), of.width()), (2, WORDS));
    let mut boxes = 0;
    for at in 0..of.len() {
        let node = driver.node(of.node[at]).expect("its node");
        let [x, y, z, radius] = of.sphere[at];
        assert_eq!(Vec3::new(x, y, z), node.position);
        match of.model_of(at) == Some(BOX) {
            // The cube of side 1 under the entity's scale of 4: half its diagonal.
            true => {
                boxes += 1;
                assert!((radius - 4.0 * 0.75f32.sqrt()).abs() < 1e-4, "{radius}");
            }
            // Half the diagonal of the box its archetype states, which the fixture makes of side 2.
            false => {
                assert_eq!(radius, node.radius);
                assert!((radius - 3.0f32.sqrt()).abs() < 1e-4, "{radius}");
            }
        }
    }
    assert_eq!(boxes, 1);
}

#[test]
fn the_world_states_its_capabilities_and_a_map_that_names_itself_has_no_parent() {
    let own = MapOf {
        parent: "m_own",
        ..map_around(cube([-50.0; 3], 100.0), vec![ent("bench", [0.0; 3])])
    };
    let files: Files = vec![
        map_file("m_own", &own),
        types_file("props", &[arch("bench", 80.0)]),
    ];
    let dir = install(&[("x64a.rpf", files)]);
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let (world, index, _) = opened(dir.path(), scratch.path());
    let caps = WorldInstall::caps(&*world);
    let stated: Vec<Capability> = (Capability::ALL.into_iter())
        .filter(|capability| caps.has(*capability))
        .collect();
    assert_eq!(
        stated,
        [
            Capability::Ground,
            Capability::Collision,
            Capability::Placements,
            Capability::WeatherGate
        ]
    );
    let [row] = index.maps() else {
        panic!("{} maps", index.maps().len());
    };
    assert_eq!(row.parents, []);
}

#[test]
fn a_pack_the_title_update_patches_links_from_its_sections_as_or_depends_indexes_it() {
    use or_title::{ExtraTitleUpdateData, TitleUpdateMount};

    let dir = tempfile::tempdir().expect("a scratch folder");
    let root = dir.path();
    let list = dlc_list_bytes(&["beta"]);
    write_archive(root, "common.rpf", &[("data/dlclist.xml", list)]);
    let town = |x: f32| map_around(cube([x, 0.0, 0.0], 50.0), vec![ent("bench", [x; 3])]);
    let (types_path, types) = types_file("props", &[arch("bench", 80.0)]);
    let (map_path, base) = map_file("m_patched", &town(0.0));
    write_pack(
        root,
        "beta",
        10,
        Vec::new(),
        vec![(map_path.as_str(), base), (types_path.as_str(), types)],
    );
    let table = ExtraTitleUpdateData {
        mounts: vec![TitleUpdateMount {
            device_name: "dlc_beta".into(),
            path: "update:/dlc_patch/beta/".into(),
        }],
    };
    let (_, patch) = map_file("m_patched", &town(3000.0));
    std::fs::create_dir_all(root.join("update")).expect("mkdir");
    write_archive(
        root,
        "update/update.rpf",
        &[
            (
                "common/data/ExtraTitleUpdateData.meta",
                table.to_xml().into_bytes(),
            ),
            (&format!("dlc_patch/beta/{map_path}"), patch),
        ],
    );
    let config = tempfile::tempdir().expect("a folder");
    let (cancel, progress) = (Cancel::new(), Progress::new());
    let opening = Opening {
        config: config.path(),
        keys: Keys::None,
        cancel: &cancel,
        progress: &progress,
    };
    let world = WorldSpec::of(LEGACY, root);
    let file = kept::file_for(config.path(), TITLE, &world);
    let game = mount_all(root);
    let (reference, _) = WorldIndex::build(&MountedWorld::new(&game, root), None);
    let stated = reference.map(hash("m_patched")).expect("the map");
    assert!(
        stated.logical.starts_with("update:/dlc_patch/beta/"),
        "{}",
        stated.logical
    );
    // Built, and then linked again from the kept sections alone.
    for _ in 0..2 {
        let install = Gta5Title.open(&world, &opening).expect("the world opens");
        let (index, _) = kept::refresh(TITLE, &world, &*install, &file).expect("the index builds");
        let linked = index.title::<Linked>().expect("a GTA V link");
        assert_eq!(linked.world.seal().ok(), reference.seal().ok());
        let row = index.named(u64::from(hash("m_patched"))).expect("a row");
        // The box the patch's copy states, which stands 3,000 m from the pack's own.
        let boxes = index.map(row).and_then(|row| row.boxes);
        assert_eq!(boxes, or_world_gta5::world_forest::map_box(stated));
        assert!(boxes.is_some_and(|boxes| boxes.entities.0.x == 3000.0));
    }
}

/// A feed that keeps nothing.
struct Unfed;

impl Feed for Unfed {
    fn set(&mut self, _: FeedName<'_>, _: &[f32]) {}
}

#[test]
fn the_sun_rises_in_the_east_stands_overhead_at_noon_and_sets_in_the_west() {
    let (_dir, world, _index, _scratch) = scene();
    let install: &dyn WorldInstall = &*world;
    let sun = |hour: f32| {
        let at = Moment { hour, weather: 0 };
        install.look().frame(&at, &mut Unfed).sun
    };
    // East is +X and up is +Z, as the game's maps are laid.
    assert!((sun(6.0) - Vec3::X).length() < 1e-5, "{}", sun(6.0));
    assert!((sun(12.0) - Vec3::Z).length() < 1e-5, "{}", sun(12.0));
    assert!((sun(18.0) - Vec3::NEG_X).length() < 1e-5, "{}", sun(18.0));
    assert!(sun(9.0).x > 0.0 && sun(9.0).z > 0.0 && sun(0.0).z < -0.99);
}

#[test]
fn a_kept_key_file_that_is_absent_and_an_executable_that_holds_no_key_are_refused() {
    let dir = install(&[("x64a.rpf", hand_built::files())]);
    let config = tempfile::tempdir().expect("a folder");
    let (cancel, progress) = (Cancel::new(), Progress::new());
    let absent = config.path().join("no_such_keys.dat");
    let empty = config.path().join("empty.exe");
    std::fs::write(&empty, [0u8; 4096]).expect("a file");
    let world = WorldSpec::of(LEGACY, dir.path());
    let refused = |keys| {
        let opening = Opening {
            config: config.path(),
            keys,
            cancel: &cancel,
            progress: &progress,
        };
        match Gta5Title.open(&world, &opening) {
            Ok(_) => "it opened".to_string(),
            Err(e) => e.to_string(),
        }
    };
    let said = refused(Keys::Snapshot(&absent));
    assert!(
        said.contains("no_such_keys.dat") && said.contains("there is no such file"),
        "{said}"
    );
    let said = refused(Keys::Executable(&empty));
    assert!(said.contains("empty.exe") && said != "it opened", "{said}");
    assert_eq!(refused(Keys::None), "it opened");
}
