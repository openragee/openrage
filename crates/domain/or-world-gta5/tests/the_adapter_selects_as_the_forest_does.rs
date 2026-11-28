//! The adapter's selection through the core's driver is the selection `WorldForest`'s own loop
//! makes, over an install built by hand: the same maps wanted, the same instances with the same
//! models, places and fades, and the same models waited for.

mod compare;
mod forest_fixture;
mod hand_built;
mod world_fixture;

use std::sync::Arc;

use glam::Vec3;
use or_world::contract::WorldInstall;
use or_world::kept;
use or_world::model::Index;
use or_world::streaming::FADE;
use or_world_gta5::{Gta5World, TITLE};
use or_world_gta5::{MountedWorld, WorldIndex};

use forest_fixture::{arch, ent, install, map_around, map_file, types_file, Ent, Files, ORPHAN};
use world_fixture::{cube, mount};

/// The world of the install at `root`, its index through the kept file under `scratch`, and the index `world_index` builds of the same mount by itself.
fn opened(
    root: &std::path::Path,
    scratch: &std::path::Path,
) -> (Arc<Gta5World>, Arc<Index>, WorldIndex) {
    let world = Arc::new(Gta5World::over(mount(root), root.to_path_buf()));
    let spec = or_world::contract::WorldSpec::of(or_world_gta5::LEGACY, root);
    let file = kept::file_for(scratch, TITLE, &spec);
    let install: &dyn WorldInstall = &*world;
    let (index, _) = kept::refresh(TITLE, &spec, install, &file).expect("the index builds");
    let (reference, _) = WorldIndex::build(&MountedWorld::new(world.game(), root), None);
    (world, Arc::new(index), reference)
}

#[test]
fn the_scene_of_the_kit_is_selected_as_the_forest_selects_it() {
    let dir = install(&[("x64a.rpf", hand_built::files())]);
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let (world, index, reference) = opened(dir.path(), scratch.path());
    let band = Vec3::new(100.0 + FADE / 2.0, 0.0, 0.0);
    let eyes = [
        (hand_built::FAR_EYE, 12, 1),
        (hand_built::NEAR_EYE, 12, 2),
        // Half into the band past the children's distance, the parent and both children draw.
        (band, 12, 3),
        (hand_built::NIGHT_EYE, 22, 1),
        (hand_built::NIGHT_EYE, 12, 0),
        (hand_built::EVENT_EYE, 12, 0),
        (hand_built::YARD_EYE, 12, 2),
        (hand_built::NOWHERE, 12, 0),
        // Within the 50 m a GTA V map is asked for ahead of its box, at a corner no sphere about the box's centre reaches.
        (Vec3::new(740.0, 740.0, 0.0), 12, 0),
    ];
    for (eye, hour, draws) in eyes {
        let drawn = compare::compared(&world, &index, &reference, eye, (hour, "CLEAR"));
        assert_eq!(drawn, Ok(draws), "at {eye}, {hour} h");
    }
}

/// Maps that state what the kit's scene does not: an entity that names no archetype, one whose archetype nothing declares, an orphan, an interior whose entity names a parent of its own, and a child whose parent map the mount lacks.
fn odd_files() -> Files {
    let lone = Ent {
        level: ORPHAN,
        lod_dist: 60.0,
        ..ent("lamp", [20.0, 0.0, 0.0])
    };
    let odd = map_around(
        cube([-50.0; 3], 100.0),
        vec![
            ent("", [0.0, 0.0, 0.0]),
            ent("nowhere_declared", [5.0, 0.0, 0.0]),
            lone,
            Ent {
                interior: true,
                lod_dist: 80.0,
                rotation: [0.0, 0.0, 0.5f32.sqrt(), 0.5f32.sqrt()],
                ..ent("flat", [10.0, 10.0, 0.0])
            },
        ],
    );
    let lost = forest_fixture::MapOf {
        parent: "m_never_written",
        ..map_around(
            cube([950.0, -50.0, -50.0], 100.0),
            vec![Ent {
                parent: Some((0, true)),
                lod_dist: 90.0,
                ..ent("lamp", [1000.0, 0.0, 0.0])
            }],
        )
    };
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
        ent("undeclared_inside", [3.0, 0.0, 0.0]),
    ];
    let types = [
        arch("lamp", 120.0),
        arch("bulb", 120.0),
        forest_fixture::Arch {
            interior: Some(inside),
            ..arch("flat", 80.0)
        },
    ];
    // Two parents in a map nothing asks for and that streams nowhere near: one states a child
    // more than it has, the other a child fewer.
    let stating = |num_children, y| Ent {
        lod_dist: 1000.0,
        child_lod_dist: 100.0,
        level: forest_fixture::LOD,
        num_children,
        ..ent("lamp", [3000.0, y, 0.0])
    };
    let far_lod = forest_fixture::MapOf {
        flags: or_world_gta5::state::NOT_BY_DEFAULT,
        streaming: cube([9000.0, 9000.0, 0.0], 10.0),
        extents: cube([9001.0, 9001.0, 1.0], 1.0),
        ..map_around(
            cube([0.0; 3], 1.0),
            vec![stating(3, 0.0), stating(1, 400.0)],
        )
    };
    let child = |parent, x, y| Ent {
        lod_dist: 40.0,
        parent: Some((parent, true)),
        ..ent("bulb", [x, y, 0.0])
    };
    let far_hd = forest_fixture::MapOf {
        parent: "m_far_lod",
        ..map_around(
            cube([2800.0, -200.0, -200.0], 800.0),
            vec![
                child(0, 2980.0, 0.0),
                child(0, 3020.0, 0.0),
                child(1, 2980.0, 400.0),
                child(1, 3020.0, 400.0),
            ],
        )
    };
    let second = forest_fixture::MapOf {
        parent: "m_far_lod",
        ..map_around(
            cube([2800.0, -200.0, -200.0], 800.0),
            vec![ent("lamp", [3000.0, 200.0, 0.0])],
        )
    };
    vec![
        map_file("m_odd", &odd),
        map_file("m_lost", &lost),
        map_file("m_far_lod", &far_lod),
        map_file("m_far_hd", &far_hd),
        // Two more maps under the same parent map, wanted with the first from the same places.
        map_file("m_far_hd_b", &second),
        map_file("m_far_hd_c", &second),
        types_file("odd", &types),
    ]
}

#[test]
fn boxes_orphans_an_interiors_own_ladder_and_a_lost_parent_are_selected_as_the_forest_selects_them()
{
    let dir = install(&[("x64a.rpf", odd_files())]);
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let (world, index, reference) = opened(dir.path(), scratch.path());
    let mut drawn = 0;
    for x in [0.0, 12.0, 25.0, 45.0, 70.0, 85.0, 110.0, 990.0] {
        for hour in [0, 12] {
            let eye = Vec3::new(x, 4.0, 1.0);
            let seen = compare::compared(&world, &index, &reference, eye, (hour, "RAIN"));
            drawn += seen.unwrap_or_else(|differs| panic!("{differs}"));
        }
    }
    for (x, y) in [
        (3050.0, 0.0),
        (3050.0, 400.0),
        (3000.0, 200.0),
        (3110.0, 0.0),
        (3300.0, 400.0),
    ] {
        let eye = Vec3::new(x, y, 1.0);
        let seen = compare::compared(&world, &index, &reference, eye, (12, "RAIN"));
        drawn += seen.unwrap_or_else(|differs| panic!("{differs}"));
    }
    assert!(
        drawn > 24,
        "the places asked about draw {drawn} instances in all"
    );
}
