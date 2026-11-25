//! Deleting an entity of each of a sample of the maps a mounted install ships, with its chain of far models, through the typed roots:
//! every LOD link that stays joins the two entities it joined, the files hold to the checks they held to, the plan lists the files
//! written, and the inverses put every file back.
//!
//! Point `OPENRAGE_GAME_ROOT` at an install root. Without one this skips, which is not a pass. `OPENRAGE_DELETE_STRIDE=N` takes every
//! Nth map that holds entities; the default takes one in two hundred.
//!
//! ⚠ A sample costs what its tree holds, since the import, the plan, the deletion and its inverses each read every source of the
//! pack, and a map in a large tree is the likelier to be sampled. On the Legacy install the largest tree sampled at one in
//! twenty-five holds 233 map files and 75,237 entities, and a run at that stride, 370 samples in 122 trees, had not finished after
//! 130 minutes of processor time on twelve threads.
//!
//! Each sample is deleted in a pack in a scratch folder: the map, every map above it, and every map under the topmost of those, so
//! that no link into a file that loses an entity is outside the pack. The samples of one tree share its pack, imported once: each
//! is planned, applied, read back and undone, and the pack held to the bytes it was made with, before the next. The body is the
//! `chain` module of this crate's shared test code, which
//! `the_checks_of_the_install_suite_hold_on_a_tree_of_maps_built_by_hand` runs where no install is.
//!
//! The second test counts the map files of the install whose first edit is refused, each with its sentence
//! (`common::census`): `OPENRAGE_FIRST_EDIT_STRIDE=N` asks every Nth map file, and the default asks every one. Its body is held on
//! an install built by hand by `the_census_of_an_install_counts_the_map_files_a_first_edit_refuses_each_with_its_sentence`.
//!
//! ⚠ The install is opened read-only and nothing here writes into one.

mod common;

use std::collections::BTreeMap;

use common::chain::{delete_sampled, survey};
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
        .unwrap_or(200);
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
    let sampled: Vec<u32> = with_entities.iter().step_by(stride).copied().collect();
    let mut report = delete_sampled(&open, &sampled, &facts, &children);
    report.maps = with_entities.len();
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

#[test]
fn the_map_files_of_the_install_a_first_edit_refuses_are_counted() {
    const WHAT: &str = "the_map_files_of_the_install_a_first_edit_refuses_are_counted";
    let Some(root) = game_roots().into_iter().next() else {
        skip_install(WHAT, format!("{GAME_ROOT_ENV} names no install"));
        return;
    };
    let stride: usize = std::env::var("OPENRAGE_FIRST_EDIT_STRIDE")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(1);
    let game = common::mount(&root).expect("the install mounts");
    let census = common::census::census(&game, stride);
    let text = census.text();
    println!("{text}");
    assert!(census.asked > 0, "no map file was asked: {text}");
    assert!(
        census.refused.len() + census.empty < census.asked,
        "no map file of the install takes a first edit, so the census measured the harness: {text}"
    );
    for (file, why) in &census.refused {
        assert!(
            why.contains(file.as_str()),
            "a refusal names its file: {why}"
        );
    }
}
