//! Two measures of a mounted install: the doors its maps place, found by the door extension and by the name patterns, and
//! every `.ybn` and `.ynd` it lists made a source that cooks to the canonical save of its file.
//!
//! Point `OPENRAGE_GAME_ROOT` at an install root. Without one each measure records its skip, which is not a pass. What the
//! measures count is held on files built by hand in `the_measures_of_the_install_suite_hold_on_files_built_by_hand.rs`.
//!
//! ⚠ The install is opened read-only: every source is written under a scratch directory.

mod common;
#[path = "common/install_measures.rs"]
mod install_measures;

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use install_measures::{cooked_back, doors_of, doors_stated_in, winners_of, Cooked, DoorCount};
use or_io::meta::binary::parse_meta_binary_full;
use or_io::meta::{Diagnostics, ErrorMode, MetaInstance};
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::MountedGame;

/// The threads a walk of an install reads its files on.
const WORKERS: usize = 12;

/// The maps one search of doors is handed: their trees are held together only that long.
const MAPS_A_SEARCH: usize = 64;

/// A metadata file of the mount as its value tree.
fn tree(game: &MountedGame, logical: &str) -> Option<MetaInstance> {
    let bytes = game.vfs.open(logical)?;
    let mut diags = Diagnostics::new(ErrorMode::Lenient);
    parse_meta_binary_full(or_io::meta::default_library(), &bytes, &mut diags)
        .ok()
        .map(|(root, _)| root)
}

/// Run `work` over `items` on [`WORKERS`] threads, each item once, and give the answers in the order of the items.
fn each<T: Sync, R: Send>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let done = Mutex::new(Vec::with_capacity(items.len()));
    std::thread::scope(|scope| {
        for _ in 0..WORKERS {
            scope.spawn(|| loop {
                let at = next.fetch_add(1, Ordering::Relaxed);
                let Some(item) = items.get(at) else { break };
                let answer = work(item);
                done.lock().unwrap().push((at, answer));
            });
        }
    });
    let mut done = done.into_inner().unwrap();
    done.sort_by_key(|(at, _)| *at);
    done.into_iter().map(|(_, answer)| answer).collect()
}

/// Every file of the mount as `(logical, leaf)`, in the mount's order.
fn listed(game: &MountedGame) -> Vec<(String, String)> {
    let mut listed = Vec::new();
    game.for_each_file(|logical, _, leaf| listed.push((logical.to_string(), leaf.to_string())));
    listed
}

/// The first install root and its mount, or the skip of `what` recorded.
fn install(what: &str) -> Option<(std::path::PathBuf, MountedGame)> {
    let Some(root) = game_roots().into_iter().next() else {
        skip_install(what, format!("{GAME_ROOT_ENV} names no install"));
        return None;
    };
    if let Some(path) = or_test_support::schema_snapshot() {
        if let Err(why) = or_io::meta::install_snapshot(&path) {
            panic!(
                "the staged schema snapshot {} is unusable: {why}",
                path.display()
            );
        }
    }
    let game = common::mount(&root).expect("the install mounts");
    Some((root, game))
}

/// ⚠ Needs an install. A model's name is the stem of a file the mount lists, and no other: a model that is an entry of a
/// drawable dictionary has none here.
#[test]
fn the_doors_of_the_install_are_counted_by_their_extension_and_by_their_name() {
    const WHAT: &str = "the_doors_of_the_install_are_counted_by_their_extension_and_by_their_name";
    let Some((root, game)) = install(WHAT) else {
        return;
    };
    let listed = listed(&game);
    let mut names: HashMap<u32, String> = HashMap::new();
    for (_, leaf) in &listed {
        let leaf = leaf.to_ascii_lowercase();
        let stem = leaf
            .rsplit_once('.')
            .map_or(leaf.as_str(), |(stem, _)| stem);
        names.insert(or_io::string_hash(stem.as_bytes()), stem.to_string());
    }
    let patterns = or_maps::doors::built_in_patterns().expect("the patterns of the tree");

    let type_files = winners_of(&listed, ".ytyp");
    let types: Vec<MetaInstance> = each(&type_files, |logical| tree(&game, logical))
        .into_iter()
        .flatten()
        .collect();
    let maps = winners_of(&listed, ".ymap");
    let searches: Vec<&[String]> = maps.chunks(MAPS_A_SEARCH).collect();
    let counted = each(&searches, |logicals| {
        let trees: Vec<MetaInstance> = (logicals.iter())
            .filter_map(|logical| tree(&game, logical))
            .collect();
        (trees.len(), doors_of(&trees, &types, &names, &patterns))
    });
    let (mut read, mut count) = (0, DoorCount::default());
    for (maps, doors) in counted {
        read += maps;
        count.merge(doors);
    }
    let by_name: usize = count.by_name.values().sum();
    let (archetypes_stating, inside_stating) = doors_stated_in(&types);
    eprintln!(
        "\n=== {}: {read} of {} map(s) and {} of {} type file(s) read; {} entit(ies) looked at; {} door(s) by the door \
         extension, {by_name} by a name {:?}; {} entit(ies) of {} model(s) with no name, of {} model(s) placed; the door \
         extension is stated by {archetypes_stating} archetype(s), by {} entit(ies) of the maps and by {inside_stating} of \
         the interiors ===",
        root.display(),
        maps.len(),
        types.len(),
        type_files.len(),
        count.entities,
        count.by_extension,
        count.by_name,
        count.unnamed_entities,
        count.unnamed_models.len(),
        count.models.len(),
        count.stated_by_the_entity,
    );
    assert!(read > 0 && !types.is_empty(), "no map or no type file read");
    assert!(count.entities > 0, "no entity was looked at");
    assert!(count.by_extension + by_name > 0, "an install with no door");
    assert!(
        count.by_extension >= count.stated_by_the_entity,
        "an entity that states the door extension is no door by it"
    );
}

/// ⚠ Needs an install. Every copy the mount lists is walked, a copy another archive shadows among them.
#[test]
fn every_bound_and_path_region_of_the_install_becomes_a_source_that_cooks_to_its_canonical_save() {
    const WHAT: &str =
        "every_bound_and_path_region_of_the_install_becomes_a_source_that_cooks_to_its_canonical_save";
    let Some((root, game)) = install(WHAT) else {
        return;
    };
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let mut failed = Vec::new();
    for extension in ["ynd", "ybn"] {
        let mut files = Vec::new();
        game.for_each_file(|logical, _, leaf| {
            let leaf = leaf.to_ascii_lowercase();
            if leaf.rsplit_once('.').is_some_and(|(_, e)| e == extension) {
                files.push((files.len(), logical.to_string()));
            }
        });
        let outcomes = each(&files, |(at, logical)| {
            let Some(bytes) = game.vfs.open(logical) else {
                return Cooked::Unopened("the mount lists it and does not open it".to_string());
            };
            let work = scratch.path().join(format!("{extension}{at}"));
            let outcome = cooked_back(extension, &bytes, &work);
            // Thousands of sources are not left on the disk until the walk ends.
            let _ = std::fs::remove_dir_all(&work);
            outcome
        });
        let same = outcomes.iter().filter(|o| **o == Cooked::Same).count();
        let mut others: BTreeMap<String, Vec<&str>> = BTreeMap::new();
        for (outcome, (_, logical)) in outcomes.iter().zip(&files) {
            let why = match outcome {
                Cooked::Same => continue,
                Cooked::Unopened(why) => format!("unopened: {why}"),
                Cooked::Refused(why) => format!("refused: {why}"),
                Cooked::Differs => "cooks to other bytes".to_string(),
            };
            others.entry(why).or_default().push(logical);
        }
        eprintln!(
            "\n=== {}: {same} of {} .{extension} became a source that cooks to its canonical save ===",
            root.display(),
            files.len()
        );
        for (why, logicals) in &others {
            eprintln!(
                "  {} file(s), the first {}: {why}",
                logicals.len(),
                logicals[0]
            );
        }
        assert!(!files.is_empty(), "the mount lists no .{extension}");
        if same != files.len() {
            failed.push(format!(
                "{} of {} .{extension}",
                files.len() - same,
                files.len()
            ));
        }
    }
    assert!(
        failed.is_empty(),
        "no source that cooks to the canonical save: {failed:?}"
    );
}
