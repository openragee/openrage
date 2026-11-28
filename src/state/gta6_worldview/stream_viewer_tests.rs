//! The world's thread as the viewer keeps it: started again when it dies, and given up at last.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use glam::Vec3;

use super::index::tests_support::index_of;
use super::index::WorldIndex;
use super::stream::World;

const TREE: u64 = 100;
const LAMP: u64 = 200;
const ROOM: u64 = 300;
const FERN: u64 = 400;
const STAND_IN: u64 = 500;

fn made_up_index() -> WorldIndex {
    index_of(
        vec![PathBuf::from("made-up.rpf")],
        &[
            (TREE, 120.0, Some(0x1000)),
            (LAMP, 300.0, Some(0x2000)),
            (ROOM, 80.0, None),
            (FERN, 150.0, Some(0x3000)),
            (STAND_IN, 900.0, Some(0x4000)),
        ],
    )
}

#[test]
fn a_world_thread_that_dies_is_started_again_until_it_has_died_too_often_and_the_panel_is_told() {
    use std::time::Duration;
    let index = Arc::new(made_up_index());
    let mut viewer = super::Viewer {
        world: Some(World::new(
            Arc::clone(&index),
            Vec3::ZERO,
            1.0,
            HashSet::new(),
        )),
        index: Some(index),
        ..Default::default()
    };
    for death in 1..=super::MOST_RESTARTS + 1 {
        let world = viewer.world.as_ref().expect("a world");
        world.end();
        let until = Instant::now() + Duration::from_secs(20);
        while !world.died() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(1));
        }
        viewer.selected(&mut Vec::new());
        let restarts = death.min(super::MOST_RESTARTS);
        assert_eq!(viewer.world_restarts, restarts, "after death {death}");
        let given_up = death > super::MOST_RESTARTS;
        assert_eq!(viewer.world_given_up(), given_up, "after death {death}");
        assert_eq!(viewer.world.as_ref().is_some_and(World::died), given_up);
    }
}

#[test]
fn a_world_thread_that_dies_on_a_map_it_just_took_in_is_started_again_without_it_and_never_given_up(
) {
    use std::time::Duration;
    let index = Arc::new(made_up_index());
    let mut viewer = super::Viewer {
        world: Some(World::new(
            Arc::clone(&index),
            Vec3::ZERO,
            1.0,
            HashSet::new(),
        )),
        index: Some(index),
        ..Default::default()
    };
    // More deaths than give the thread up, each on a map of its own.
    let deaths = super::MOST_RESTARTS + 2;
    for death in 1..=deaths {
        let world = viewer.world.as_ref().expect("a world");
        world.took(0x100 + death);
        world.end();
        let until = Instant::now() + Duration::from_secs(20);
        while !world.died() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(1));
        }
        viewer.selected(&mut Vec::new());
        assert_eq!(viewer.world_restarts, death, "after death {death}");
        assert!(
            viewer.skipped.contains(&(0x100 + death)),
            "after death {death}"
        );
        assert!(!viewer.world_given_up(), "after death {death}");
        assert!(!viewer.world.as_ref().is_some_and(World::died));
    }
    assert_eq!(viewer.skipped.len() as u32, deaths);
}
