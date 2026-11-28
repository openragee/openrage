//! What the maps of a mounted install hold besides entities: the census of their parked vehicles, box
//! occluders, occlude models, time cycle modifiers and far lights, and the typed read and the save of
//! every map that holds one.
//!
//! Point `OPENRAGE_GAME_ROOT` at one or more install roots, `;`-separated. Without one this skips,
//! which is not a pass.
//!
//! The census prints, for each record, the members the files' own type tables state for it with each
//! member's type, how many files of each encoding state it so and how many records they hold, every
//! other member a map states with how many maps hold something under it, and how the far lights are
//! split across pairs of maps. The suite then holds the members `or_io::meta::typed` declares to the
//! ones the records of the install state, reads every map typed against the read views of
//! `or_world::MapData`, and saves every map that holds such a record.
//!
//! ⚠ The install is opened read only and nothing here writes into one.
//!
//! ⚠ This binary is in the slow tier, which no gate runs: what it judges of its census is held on records built by hand in
//! `the_census_of_a_maps_records_holds_on_records_built_by_hand`, over the module both share.
//!
//! ⚠ An RBM goes back under a fresh DEFLATE, so the bytes a map was read from are its RSC7 header and
//! its decompressed payload; a PSO is compared whole.

mod support;

#[path = "support/map_records.rs"]
mod map_records;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use map_records::Census;
use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
use or_io::container::RpfCryptoProvider;
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::game::Game;

fn crypto_for(root: &Path) -> Option<Arc<dyn RpfCryptoProvider>> {
    let exe = Game::detect_from_root(root)?.find_executable(root)?;
    let keys = extract_keys(&exe, &|_| {}).ok()?;
    Some(Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys))))
}

fn census_of_install(root: &Path) -> Option<Census> {
    support::schema();
    let install = or_title::GtaInstall::from_path(root)?;
    let game =
        or_title::build_mounted_game(&install, crypto_for(root)?, or_title::DlcCutoff::All).ok()?;
    let mut ymap: BTreeMap<String, String> = BTreeMap::new();
    game.for_each_file(|logical, _container, leaf| {
        if leaf.to_ascii_lowercase().ends_with(".ymap") {
            ymap.insert(logical.to_ascii_lowercase(), logical.to_string());
        }
    });
    let paths: Vec<&String> = ymap.values().collect();
    let next = AtomicUsize::new(0);
    let whole = Mutex::new(Census {
        label: root.display().to_string(),
        ..Census::default()
    });
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                let mut local = Census::default();
                while let Some(path) = paths.get(next.fetch_add(1, Ordering::Relaxed)) {
                    match game.vfs.open(path) {
                        Some(bytes) => local.fold(path, bytes),
                        None => local.problems.push(format!("{path}: does not open")),
                    }
                }
                let mut whole = whole
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                whole.merge(local);
            });
        }
    });
    let mut census = whole
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // In one order whatever thread read a map.
    census.lights.sort_by(|a, b| a.path.cmp(&b.path));
    census.problems.sort();
    census.pair_the_lights();
    Some(census)
}

#[test]
fn every_map_of_an_install_reads_its_records_typed_states_the_members_typed_and_saves_to_its_fixed_point(
) {
    const WHAT: &str =
        "every_map_of_an_install_reads_its_records_typed_states_the_members_typed_and_saves_to_its_fixed_point";
    let roots = game_roots();
    if roots.is_empty() {
        skip_install(
            WHAT,
            format_args!(
                "no install root; set {GAME_ROOT_ENV} to a ;-separated list of install directories"
            ),
        );
        return;
    }
    let installs: Vec<Census> = roots.iter().filter_map(|r| census_of_install(r)).collect();
    if installs.is_empty() {
        skip_install(WHAT, "no install root mounted");
        return;
    }
    for install in &installs {
        install.print();
        let report = install.report();
        // A census that read nothing would hold every member to nothing and pass.
        assert!(install.maps >= 10_000, "{report}");
        assert!(
            install.records.iter().all(|&(records, _)| records >= 100),
            "{report}"
        );
        assert!(install.pairs >= 100, "{report}");
        assert!(
            install.problems.is_empty(),
            "{report}\n  {}",
            install
                .problems
                .iter()
                .take(20)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n  ")
        );
        let differ = install.members_that_differ();
        assert!(differ.is_empty(), "{report}\n  {}", differ.join("\n  "));
        let differ = install.signatures_that_differ();
        assert!(differ.is_empty(), "{report}\n  {}", differ.join("\n  "));
        assert_eq!(install.saved_fixed, install.with_a_record, "{report}");
        // ⚠ Measured, and held: no map with such a record is saved by a layout other than the one it arrived with.
        assert_eq!(install.saved_as_read, install.with_a_record, "{report}");
        assert_eq!(install.pairs_saved, install.pairs, "{report}");
        assert!(install.swapped.0 > 0, "{report}");
        assert_eq!(install.swapped.1, install.swapped.0, "{report}");
        // What the refusals of a save stand on.
        assert_eq!(install.sized.1, 0, "{report}");
        assert_eq!(install.counted_runs.1, 0, "{report}");
    }
}
