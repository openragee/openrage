//! The two scripts of the World screen's reference, held to each other: the one that draws its
//! shots and dumps, and the one that holds a run's shots and dumps to a reference's.

use std::path::PathBuf;

use super::script::{parse_with, Step};
use super::world::WorldStep;

const DRAWS: &str = include_str!("../../../scripts/shots/gta6_world_reference.shots");
const HOLDS: &str = include_str!("../../../scripts/shots/gta6_world_reference_compare.shots");

/// The steps of `script`, each variable standing for its own name.
fn steps(script: &str) -> Vec<Step> {
    let steps = parse_with(script, &|name| Some(format!("<{name}>"))).expect("the script reads");
    steps.into_iter().map(|(_, _, step)| step).collect()
}

#[test]
fn every_shot_of_the_reference_is_settled_then_converged_then_dumped() {
    let drawn = steps(DRAWS);
    assert!(
        drawn.contains(&Step::Game(
            crate::state::prefs::Game::GtaVI,
            Some(PathBuf::from("<OPENRAGE_GTA6_BUILD>"))
        )),
        "the build's folder is the environment's, never a folder written in the tree"
    );
    let (mut settled, mut converged, mut shots) = (false, false, 0);
    for (at, step) in drawn.iter().enumerate() {
        match step {
            // What moves the camera or changes what streams or how it is lit undoes the wait before it.
            Step::World(
                WorldStep::Camera(_)
                | WorldStep::Hour(_)
                | WorldStep::Modes(_)
                | WorldStep::Weather(_)
                | WorldStep::Group(..),
            )
            | Step::Mode(_)
            | Step::Size(..) => (settled, converged) = (false, false),
            Step::World(WorldStep::Settle(_)) => (settled, converged) = (true, false),
            Step::World(WorldStep::Converge(_)) => converged = settled,
            Step::Shot(name) => {
                assert!(
                    settled && converged,
                    "`shot {name}` comes after a settle and then a converge of what was set before it"
                );
                assert_eq!(
                    drawn.get(at + 1),
                    Some(&Step::World(WorldStep::Dump(format!("{name}.json")))),
                    "`shot {name}` is followed by the dump of the frame it took"
                );
                shots += 1;
            }
            Step::Compare(_) | Step::CompareDump(_) => {
                panic!("the script that draws judges nothing: a reference is a run of it")
            }
            _ => {}
        }
    }
    assert!(shots > 0, "the script takes no shot");
}

#[test]
fn the_script_that_holds_names_every_shot_and_dump_of_the_script_that_draws_and_draws_nothing() {
    let shots: Vec<String> = (steps(DRAWS).into_iter())
        .filter_map(|step| match step {
            Step::Shot(name) => Some(name),
            _ => None,
        })
        .collect();
    let (mut compared, mut dumps) = (Vec::new(), Vec::new());
    for step in steps(HOLDS) {
        match step {
            Step::Compare(compare) => {
                let reference = format!("<OPENRAGE_WORLD_REFERENCE>/{}.png", compare.shot);
                assert_eq!(compare.reference, PathBuf::from(reference));
                // ⚠ The widest bounds pass any frame: a shot held by one of them is not held.
                assert!(
                    compare.tolerance < 255.0 && compare.share < 100.0,
                    "`{}` is held by a bound that passes anything",
                    compare.shot
                );
                compared.push(compare.shot);
            }
            Step::CompareDump(compare) => {
                let reference = format!("<OPENRAGE_WORLD_REFERENCE>/{}", compare.dump);
                assert_eq!(compare.reference, PathBuf::from(reference));
                dumps.push(compare.dump);
            }
            // ⚠ It runs over the folder a run of the first script wrote: a step that drew would write over what it judges.
            other => panic!("the script that holds only compares: {other:?}"),
        }
    }
    assert_eq!(compared, shots, "each shot is compared once, in its order");
    let named: Vec<String> = shots.iter().map(|name| format!("{name}.json")).collect();
    assert_eq!(dumps, named, "each dump is compared once, in its order");
}
