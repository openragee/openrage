//! Deleting an entity of each of a sample of the maps a mounted install ships, with its chain of far models, through the typed roots:
//! every LOD link that stays joins the two entities it joined, the files hold to the checks they held to, the plan lists the files
//! written, and the inverses put every file back.
//!
//! Point `OPENRAGE_GAME_ROOT` at an install root. Without one this skips, which is not a pass. `OPENRAGE_DELETE_STRIDE=N` takes every
//! Nth map that holds entities; the default takes one in twenty-five.
//!
//! Each sample is a pack of its own in a scratch folder: the map, every map above it, and every map under the topmost of those, so
//! that no link into a file that loses an entity is outside the pack. It is made, planned, applied, read back, undone and dropped
//! before the next. The body is the `chain` module of this crate's shared test code, which
//! `the_checks_of_the_install_suite_hold_on_a_tree_of_maps_built_by_hand` runs where no install is.
//!
//! ⚠ The install is opened read-only and nothing here writes into one.

mod common;

use std::collections::BTreeMap;

use common::chain::{delete_one, survey, Report};
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};

const WHAT: &str = "deleting_an_entity_of_the_install_keeps_every_link";

#[test]
fn deleting_an_entity_of_the_install_keeps_every_link() {
    let Some(root) = game_roots().into_iter().next() else {
        skip_install(WHAT, format!("{GAME_ROOT_ENV} names no install"));
        return;
    };
    let stride: usize = std::env::var("OPENRAGE_DELETE_STRIDE")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(25);
    let game = common::mount(&root).expect("the install mounts");

    // The file each name of a map file resolves to, and what each map name holds.
    let mut winners: BTreeMap<String, String> = BTreeMap::new();
    game.for_each_file(|logical, _, leaf| {
        let leaf = leaf.to_ascii_lowercase();
        if leaf.ends_with(".ymap") {
            winners.insert(leaf, logical.to_string());
        }
    });
    let open = |path: &str| game.vfs.open(path);
    let paths: Vec<String> = winners.into_values().collect();
    let (facts, children) = survey(&paths, &open);

    let with_entities: Vec<u32> = (facts.iter())
        .filter(|(_, held)| held.entities > 0)
        .map(|(name, _)| *name)
        .collect();
    let mut report = Report {
        maps: with_entities.len(),
        ..Report::default()
    };
    for name in with_entities.iter().step_by(stride) {
        report.sampled += 1;
        delete_one(&open, *name, &facts, &children, &mut report);
    }
    let text = report.text(stride);
    println!("{text}");
    assert!(report.failures.is_empty(), "{text}");
    assert!(report.deleted > 0, "no deletion was carried out: {text}");
    assert!(
        report.renumbered + report.renumbered_into_parent > 0,
        "no deletion moved an index, so nothing was tested: {text}"
    );
    assert!(
        report.across_files > 0,
        "no deletion reached a second file: {text}"
    );
}
