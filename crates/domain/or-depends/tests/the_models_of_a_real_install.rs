//! The models and the vehicles of a real install as a browser lists them: the catalogue's totals held to the index's own
//! counts, a sample of the files it says hold a model opened from the mount, its counts of placements held to a walk of
//! every map, how long each question takes, and the vehicles of the install's data files.
//!
//! ⚠ Needs an install: without one each test records a skip and has checked nothing. The world index of the install
//! takes minutes to read, so the first test to run keeps it in a file of the machine's scratch folder and the others
//! read that file; `.config/nextest.toml` runs the tests of this binary one at a time for that.

mod world_fixture;

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use or_depends::find::{names_of, place_maps, SearchNames};
use or_depends::models::{Query, Sort};
use or_depends::vehicles::VehicleQuery;
use or_depends::world_index::{refresh, KeptState};
use or_depends::{
    Kind, ModelCatalogue, ModelContents, MountedWorld, PlacementIndex, VehicleCatalogue,
    WorldIndex, WorldSource,
};
use or_io::container::rpf::crypto::RpfCryptoProvider;
use or_title::vfs::{build_mounted_game, DlcCutoff, FileStamp};
use or_title::{GtaInstall, MountedGame};
use or_world::ArchetypeKind;
use world_fixture::Counting;

/// The first install of the machine, mounted with every pack; without an install or its keys, `what` is recorded as
/// skipped.
fn mounted(what: &str) -> Option<(PathBuf, MountedGame)> {
    let roots = or_test_support::game_roots();
    let Some(root) = roots.first() else {
        or_test_support::skip_install(
            what,
            format_args!("set {} to an install root", or_test_support::GAME_ROOT_ENV),
        );
        return None;
    };
    let exe = or_title::Game::detect_from_root(root).and_then(|game| game.find_executable(root));
    let Some(exe) = exe else {
        or_test_support::skip_install(what, "no executable, so no keys");
        return None;
    };
    let Ok(keys) = or_io::container::rpf::crypto::extract_keys(&exe, &|_| {}) else {
        or_test_support::skip_install(what, "key extraction failed");
        return None;
    };
    if let Some(path) = or_test_support::schema_snapshot() {
        if let Err(e) = or_io::meta::install_snapshot(&path) {
            panic!(
                "the staged schema snapshot {} is unusable: {e}",
                path.display()
            );
        }
    }
    let crypto: Arc<dyn RpfCryptoProvider> =
        Arc::new(or_io::container::rpf::crypto::Gta5RetailCryptoProvider::new(Arc::new(keys)));
    let install = GtaInstall { root: root.clone() };
    let game = build_mounted_game(&install, crypto, DlcCutoff::All).expect("the install mounts");
    Some((root.clone(), game))
}

/// Where the index of the install at `root` is kept between the tests of this binary: a file of the machine's scratch
/// folder named for the install and for this binary as it was built, so a binary built again reads no file an earlier
/// one kept.
fn kept_file(root: &Path) -> PathBuf {
    let folder = std::env::temp_dir().join("openrage-model-catalogue-suite");
    let built = std::env::current_exe()
        .ok()
        .and_then(|exe| FileStamp::of(&exe));
    let built = built.map_or(0, |stamp| stamp.modified ^ stamp.size);
    let install = or_io::string_hash(root.to_string_lossy().as_bytes());
    let name = format!("{install:08x}-{built:016x}.index");
    // What an earlier build of this binary kept of the same install is of no use to any later one.
    for kept in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
        let other = kept.file_name().to_string_lossy().into_owned();
        if other.starts_with(&format!("{install:08x}-")) && other != name {
            let _ = std::fs::remove_file(kept.path());
        }
    }
    folder.join(name)
}

/// One test of this process reads the install at a time, so the first keeps the index and the others read it.
static READING: Mutex<()> = Mutex::new(());

/// The world index of `game`, read from its kept file by a source that is asked nothing: the file is built first where no
/// test kept it yet.
fn indexed(game: &MountedGame, root: &Path) -> WorldIndex {
    let _one = READING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let file = kept_file(root);
    let source = MountedWorld::new(game, root);
    let began = Instant::now();
    let (_, report) = refresh(&source, &file);
    assert!(report.not_kept.is_none(), "{:?}", report.not_kept);
    eprintln!(
        "\n=== the index: kept {:?}, {} of {} archive(s) read, {} file(s) opened, in {:.1} s ===",
        report.kept,
        report.read.len(),
        report.archives,
        report.opened,
        began.elapsed().as_secs_f64()
    );
    // The index every check asks is the kept one, read by a source that counts what it is asked.
    let counting = Counting::new(&source);
    let began = Instant::now();
    let (index, report) = refresh(&counting, &file);
    let read = began.elapsed();
    assert!(
        matches!(report.kept, KeptState::Current),
        "{:?}",
        report.kept
    );
    let (listed, opened) = counting.counts();
    assert!(
        listed.iter().chain(&opened).all(|count| *count == 0) && report.opened == 0,
        "the kept index listed {listed:?} and opened {opened:?}"
    );
    let size = std::fs::metadata(&file).map_or(0, |meta| meta.len());
    eprintln!(
        "=== the kept index: {size} byte(s), read in {:.3} s with no archive listed and no file opened ===",
        read.as_secs_f64()
    );
    index
}

fn kind_word(kind: ArchetypeKind) -> &'static str {
    match kind {
        ArchetypeKind::Base => "base",
        ArchetypeKind::Time => "time",
        ArchetypeKind::Mlo => "interior",
    }
}

const TOTALS: &str =
    "the_catalogue_of_a_real_install_agrees_with_its_index_kind_by_kind_and_pack_by_pack";

/// ⚠ Needs an install. The index's own counts are taken from its archetypes one by one, and the catalogue's from its
/// rows and from the total each filter answers with.
#[test]
fn the_catalogue_of_a_real_install_agrees_with_its_index_kind_by_kind_and_pack_by_pack() {
    let Some((root, game)) = mounted(TOTALS) else {
        return;
    };
    let index = indexed(&game, &root);
    let began = Instant::now();
    let catalogue = ModelCatalogue::new(&index);
    let built = began.elapsed();
    assert!(index.archetype_count() > 1000, "an install with no models");
    assert_eq!(catalogue.len(), index.archetype_count());

    // ⚠ A pack by another road than the catalogue's: the device a type file's own path begins with, and for a file of a
    // folder that patches a pack, the device of the last archive the mount says such a folder patches.
    let source = MountedWorld::new(&game, &root);
    let archives = source.archives();
    let folders: Vec<(String, &str)> = (0..archives.len())
        .filter_map(|at| {
            let folder = source.patch_of(at)?.to_ascii_lowercase();
            Some((folder, archives[at].device.as_str()))
        })
        .collect();
    let pack_by_path = |logical: &str| -> (String, bool) {
        let lower = logical.to_ascii_lowercase();
        let mut patched = folders.iter().rev();
        match patched.find(|(folder, _)| lower.starts_with(folder.as_str())) {
            Some((_, device)) => (device.to_string(), true),
            None => (logical.split(":/").next().unwrap_or("").to_string(), false),
        }
    };
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    let mut packs: BTreeMap<String, usize> = BTreeMap::new();
    let mut of_a_patch = 0usize;
    for archetype in index.archetypes() {
        *kinds.entry(kind_word(archetype.kind)).or_default() += 1;
        let file = index.type_file(archetype.type_file);
        let file = file.expect("the type file that declares an archetype");
        let (pack, patched) = pack_by_path(&file.logical);
        assert_eq!(
            index.pack_of(file.archive, &file.logical),
            pack,
            "{}",
            file.logical
        );
        of_a_patch += usize::from(patched);
        *packs.entry(pack).or_default() += 1;
    }
    let total = |query: Query<'_>| {
        let page = catalogue.page(&Query { count: 0, ..query });
        page.expect("a question").total
    };
    assert_eq!(total(Query::default()), index.archetype_count());
    let mut by_kind = 0;
    for (kind, count) in catalogue.kinds() {
        let own = kinds.get(kind_word(kind)).copied().unwrap_or(0);
        assert_eq!(count, own, "{kind:?}");
        let asked = total(Query {
            kind: Some(kind),
            ..Query::default()
        });
        assert_eq!(asked, own, "{kind:?} by its filter");
        by_kind += asked;
    }
    assert_eq!(by_kind, catalogue.len(), "each model is of one kind");
    let (mut by_pack, mut with_models) = (0, 0);
    for (pack, count) in catalogue.packs() {
        let own = packs.get(pack).copied().unwrap_or(0);
        assert_eq!(count, own, "{pack}");
        let asked = total(Query {
            pack: Some(pack),
            ..Query::default()
        });
        assert_eq!(asked, own, "{pack} by its filter");
        by_pack += asked;
        with_models += usize::from(own > 0);
    }
    assert_eq!(by_pack, catalogue.len(), "each model is of one pack");
    assert_eq!(
        with_models,
        packs.len(),
        "a pack the catalogue does not list"
    );
    assert!(with_models > 1, "{packs:?}");

    let replaced: usize = (catalogue.models().iter())
        .map(|model| catalogue.replaced(model).len())
        .sum();
    assert_eq!(replaced, index.redefined());
    let placed = total(Query {
        placed: Some(true),
        ..Query::default()
    });
    let unplaced = total(Query {
        placed: Some(false),
        ..Query::default()
    });
    assert_eq!(placed + unplaced, catalogue.len());
    assert!(
        placed > 0 && unplaced > 0,
        "{placed} placed, {unplaced} never"
    );

    // A model is named by a file the index keeps; the search of the install names it by any file of the mount.
    let (_, names) = names_of(&source);
    let by_any_file = SearchNames::new(&index, &names).unnamed();
    assert!(catalogue.unnamed() >= by_any_file);
    assert!(catalogue.unnamed() < catalogue.len(), "no model has a name");
    let mut held: BTreeMap<&str, usize> = BTreeMap::new();
    for model in catalogue.models() {
        let by = catalogue
            .held(model)
            .map_or("no file", |file| match file.kind {
                Kind::DrawableDict => "a drawable dictionary",
                Kind::Fragment => "a fragment",
                _ => "a drawable",
            });
        *held.entry(by).or_default() += 1;
    }
    let inside = (catalogue.models().iter())
        .filter(|m| m.placed == 0 && m.inside > 0)
        .count();
    let sized = catalogue.models().iter().filter(|m| m.size.is_some());
    // An interior a later type file declares again: what the earlier file's interior holds is counted with the later's.
    let interiors_again = (catalogue.models().iter())
        .filter(|m| m.kind == ArchetypeKind::Mlo && !catalogue.replaced(m).is_empty())
        .count();
    // What the index keeps for the list beside what it kept before: the paths of the model and texture files, and a
    // count for each archetype a map places or an interior holds.
    let files = index.files();
    let paths: usize = files.iter().map(|(_, logical)| logical.len()).sum();
    let placing: usize = index.maps().iter().map(|map| map.placed.len()).sum();
    let holding: usize = (index.type_files().iter())
        .map(|file| file.inside.len())
        .sum();
    eprintln!(
        "\n=== kept for the list: {} model and texture file(s) in {paths} byte(s) of paths, {placing} count(s) of an \
         archetype a map places and {holding} of one an interior holds ===",
        files.len()
    );
    eprintln!(
        "\n=== {} model(s), listed in {:.3} s: {kinds:?}; {} pack(s) with models of {}; {} named by no file the index \
         keeps, {by_any_file} by no file of the mount; held by {held:?}; {placed} placed, of which {inside} inside an \
         interior alone, {unplaced} never; {} with a size; {replaced} declaration(s) replaced, {interiors_again} \
         interior(s) among the models declared again; {of_a_patch} model(s) declared in a folder that patches a pack, of {} such folder(s) ===",
        catalogue.len(),
        built.as_secs_f64(),
        with_models,
        catalogue.packs().len(),
        catalogue.unnamed(),
        sized.count(),
        folders.len(),
    );
}

const OPENS: &str =
    "every_model_the_catalogue_of_a_real_install_says_a_file_holds_opens_from_the_mount";

/// One model in this many of those the catalogue says a file holds is opened.
const ONE_IN: usize = 500;

/// ⚠ Needs an install. Each file is opened from the archive the catalogue names, and held to what the mount opens at the
/// same path. How many are a resource and how many dictionaries hold their entry is counted and printed, not asked.
#[test]
fn every_model_the_catalogue_of_a_real_install_says_a_file_holds_opens_from_the_mount() {
    let Some((root, game)) = mounted(OPENS) else {
        return;
    };
    let index = indexed(&game, &root);
    let catalogue = ModelCatalogue::new(&index);
    let source = MountedWorld::new(&game, &root);
    let held = (catalogue.models().iter()).filter(|model| catalogue.held(model).is_some());
    let held: Vec<_> = held.collect();
    assert!(
        held.len() > ONE_IN,
        "{} model(s) held by a file",
        held.len()
    );

    let (mut opened, mut resources) = (0usize, 0usize);
    let (mut dictionaries, mut read, mut holding) = (0usize, 0usize, 0usize);
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    for model in held.iter().step_by(ONE_IN) {
        let file = catalogue.held(model).expect("held");
        let bytes = source.open(file.archive, file.logical);
        let bytes = bytes.unwrap_or_else(|| panic!("{} does not open", file.logical));
        assert!(!bytes.is_empty(), "{} is empty", file.logical);
        assert!(
            game.vfs.open(file.logical).as_ref() == Some(&bytes),
            "{} is one file in archive {} and another through the mount",
            file.logical,
            file.archive
        );
        opened += 1;
        resources += usize::from(bytes.starts_with(b"RSC7"));
        *kinds.entry(file.kind.extension()).or_default() += 1;
        if file.kind == Kind::DrawableDict {
            dictionaries += 1;
            let mut contents = ModelContents::new();
            if contents.add_dictionary(model.drawable_dictionary, &bytes) {
                read += 1;
                let holds = contents.dictionary_holds(model.drawable_dictionary, model.asset_name);
                holding += usize::from(holds == Some(true));
            }
        }
    }
    assert_eq!(opened, held.len().div_ceil(ONE_IN));
    eprintln!(
        "\n=== {opened} of {} model(s) held by a file opened, one in {ONE_IN}: {kinds:?}; {resources} a resource; \
         of {dictionaries} drawable dictionar(ies), {read} read and {holding} hold the model's entry ===",
        held.len()
    );
}

const PLACED: &str = "the_placement_counts_of_a_real_install_are_those_of_a_walk_of_every_map";

/// ⚠ Needs an install. The walk reads every map of the mount again and keeps every placement, which is what the index
/// counted once and kept.
#[test]
fn the_placement_counts_of_a_real_install_are_those_of_a_walk_of_every_map() {
    let Some((root, game)) = mounted(PLACED) else {
        return;
    };
    let index = indexed(&game, &root);
    let catalogue = ModelCatalogue::new(&index);
    let source = MountedWorld::new(&game, &root);
    let mut walked = PlacementIndex::new();
    let unread = place_maps(&index, &source, &mut walked);
    assert_eq!(unread, 0, "map(s) the index read gave no map now");

    let mut differing = Vec::new();
    let mut of_models = 0u64;
    for model in catalogue.models() {
        let walk = walked.of(model.hash).len();
        if model.placed as usize != walk && differing.len() < 12 {
            differing.push(format!(
                "{:#010x}: {} kept, {walk} walked",
                model.hash, model.placed
            ));
        }
        of_models += u64::from(model.placed);
    }
    assert!(
        differing.is_empty(),
        "the kept count is not the walk's, the first:\n{}",
        differing.join("\n")
    );
    // What the walk saw and no model accounts for: a placement that names nothing, or an archetype no type file declares.
    let declared: HashSet<u32> = catalogue.models().iter().map(|model| model.hash).collect();
    let undeclared = walked.archetypes().filter(|name| !declared.contains(name));
    let (mut strangers, mut of_strangers) = (0usize, 0u64);
    for name in undeclared {
        strangers += 1;
        of_strangers += walked.of(name).len() as u64;
    }
    assert_eq!(of_models + of_strangers + walked.nameless(), walked.seen());
    assert!(of_models > 1000, "{of_models} placement(s)");
    let inside: u64 = (catalogue.models().iter())
        .map(|model| u64::from(model.inside))
        .sum();
    eprintln!(
        "\n=== {} map(s) walked: {} placement(s), {of_models} of the {} model(s), {of_strangers} of {strangers} \
         archetype(s) no type file declares, {} that name nothing; {inside} more inside the interiors of the type \
         files ===",
        walked.maps(),
        walked.seen(),
        catalogue.len(),
        walked.nameless(),
    );
}

const TIMES: &str = "the_questions_a_browser_asks_of_a_real_install_are_answered_in_milliseconds";

/// The longest the middle run of a question may take.
///
/// ⚠ A ceiling set before any install was asked, not a measurement: far over the milliseconds a question takes, because
/// other lanes build on every core of the machine beside the suite.
const CEILING: Duration = Duration::from_millis(250);

/// How often each question is asked after its first time.
const RUNS: usize = 15;

/// ⚠ Needs an install. Each question is asked once, which sorts the order it lists in where none asked for it before,
/// and then [`RUNS`] times more, of which the middle time is the one held to [`CEILING`].
#[test]
fn the_questions_a_browser_asks_of_a_real_install_are_answered_in_milliseconds() {
    let Some((root, game)) = mounted(TIMES) else {
        return;
    };
    let index = indexed(&game, &root);
    let began = Instant::now();
    let catalogue = ModelCatalogue::new(&index);
    let built = began.elapsed();
    let (packs, models) = (catalogue.packs(), catalogue.models());
    // The install's own words: its pack with the most models, and the name of the model in the middle of the list.
    let (largest, _) = packs
        .iter()
        .max_by_key(|(_, count)| *count)
        .expect("a pack");
    let named = models.iter().filter_map(|model| model.name.as_deref());
    let named: Vec<&str> = named.collect();
    let middle = named[named.len() / 2];
    let part = middle.split('_').next_back().expect("a part");
    let letters = &middle[..3.min(middle.len())];
    let page = Query {
        count: 50,
        ..Query::default()
    };
    let questions: Vec<(&str, Query<'_>)> = vec![
        ("the first page by name", page),
        (
            "the last page by name",
            Query {
                offset: models.len().saturating_sub(50),
                ..page
            },
        ),
        (
            "largest first",
            Query {
                sort: Some(Sort::Size),
                descending: true,
                ..page
            },
        ),
        (
            "most placed first",
            Query {
                sort: Some(Sort::Placed),
                descending: true,
                ..page
            },
        ),
        (
            "a name in full",
            Query {
                text: Some(middle),
                ..page
            },
        ),
        (
            "a part of a name",
            Query {
                text: Some(part),
                ..page
            },
        ),
        (
            "three letters",
            Query {
                text: Some(letters),
                ..page
            },
        ),
        (
            "the pack with the most models",
            Query {
                pack: Some(largest),
                ..page
            },
        ),
        (
            "interiors",
            Query {
                kind: Some(ArchetypeKind::Mlo),
                ..page
            },
        ),
        (
            "time archetypes",
            Query {
                kind: Some(ArchetypeKind::Time),
                ..page
            },
        ),
        (
            "one to two metres",
            Query {
                size: (Some(1.0), Some(2.0)),
                ..page
            },
        ),
        (
            "never placed",
            Query {
                placed: Some(false),
                ..page
            },
        ),
        (
            "three letters, placed, under ten metres, largest first",
            Query {
                text: Some(letters),
                placed: Some(true),
                size: (None, Some(10.0)),
                sort: Some(Sort::Size),
                descending: true,
                ..page
            },
        ),
    ];
    eprintln!(
        "\n=== {} model(s), listed from the index in {:.3} s; each question once and then {RUNS} times ===",
        models.len(),
        built.as_secs_f64()
    );
    let mut slowest = Duration::ZERO;
    let mut answering = 0;
    for (what, question) in &questions {
        let ask = || {
            let began = Instant::now();
            let page = catalogue.page(question).expect("a question");
            (began.elapsed(), page.total, page.rows.len())
        };
        let (first, total, rows) = ask();
        let mut times: Vec<Duration> = (0..RUNS).map(|_| ask().0).collect();
        times.sort();
        let middle = times[RUNS / 2];
        eprintln!(
            "=== {what}: {total} answer, {rows} listed; first {:.3} ms, middle {:.3} ms, longest {:.3} ms ===",
            first.as_secs_f64() * 1e3,
            middle.as_secs_f64() * 1e3,
            times[RUNS - 1].as_secs_f64() * 1e3,
        );
        assert!(middle <= CEILING, "{what} took {middle:?}");
        slowest = slowest.max(middle);
        answering += usize::from(total > 0);
    }
    // A question that answers nothing says little of what one costs: the name in full and the pages answer by what they are.
    assert!(
        answering + 2 >= questions.len(),
        "{answering} question(s) answer"
    );
    eprintln!(
        "=== the slowest question's middle run: {:.3} ms ===",
        slowest.as_secs_f64() * 1e3
    );
}

const VEHICLES: &str =
    "the_vehicles_of_a_real_install_are_each_listed_once_with_a_class_a_type_and_a_pack";

/// ⚠ Needs an install. Every vehicle data file the mount would load must read, and every vehicle must state a class and
/// a type; how many have a fragment of their name among the files the index keeps is counted and printed, not asked.
#[test]
fn the_vehicles_of_a_real_install_are_each_listed_once_with_a_class_a_type_and_a_pack() {
    let Some((root, game)) = mounted(VEHICLES) else {
        return;
    };
    let began = Instant::now();
    let catalogue = VehicleCatalogue::new(or_title::install::vehicles::vehicles(&game));
    let read = began.elapsed();
    let list = catalogue.list();
    for file in &list.files {
        assert!(
            file.read.is_ok(),
            "{} of {}: {:?}",
            file.path,
            file.pack,
            file.read
        );
    }
    assert!(
        list.files.len() > 1 && list.vehicles.len() > 100,
        "{} vehicle(s)",
        list.vehicles.len()
    );
    let models: HashSet<String> = (list.vehicles.iter())
        .map(|vehicle| vehicle.vehicle.model_name.to_ascii_lowercase())
        .collect();
    assert_eq!(models.len(), list.vehicles.len(), "a model is listed twice");
    let stated: usize = (list.files.iter())
        .map(|file| *file.read.as_ref().unwrap_or(&0))
        .sum();
    let replaced: usize = list.vehicles.iter().map(|v| v.replaced.len()).sum();
    assert_eq!(stated, list.vehicles.len() + replaced);
    for vehicle in &list.vehicles {
        let said = &vehicle.vehicle;
        assert!(
            !said.class.is_empty() && !said.vehicle_type.is_empty(),
            "{} states no class or no type",
            said.model_name
        );
    }

    // Each class, type and pack lists its own vehicles, and each vehicle is in one of each.
    let total = |query: VehicleQuery<'_>| catalogue.page(&query).total;
    assert_eq!(total(VehicleQuery::default()), list.vehicles.len());
    let (classes, types, packs) = (catalogue.classes(), catalogue.types(), catalogue.packs());
    let mut seen = [0usize; 3];
    for (class, count) in &classes {
        let page = catalogue.page(&VehicleQuery {
            class: Some(class),
            ..VehicleQuery::default()
        });
        assert_eq!(page.total, *count, "{class}");
        assert!(page
            .rows
            .iter()
            .all(|row| row.vehicle.vehicle.class.eq_ignore_ascii_case(class)));
        seen[0] += count;
    }
    for (kind, count) in &types {
        let asked = total(VehicleQuery {
            vehicle_type: Some(kind),
            ..VehicleQuery::default()
        });
        assert_eq!(asked, *count, "{kind}");
        seen[1] += count;
    }
    for (pack, count) in &packs {
        let asked = total(VehicleQuery {
            pack: Some(pack),
            ..VehicleQuery::default()
        });
        assert_eq!(asked, *count, "{pack}");
        seen[2] += count;
    }
    assert_eq!(seen, [list.vehicles.len(); 3]);
    assert!(classes.len() > 5 && types.len() > 3 && packs.len() > 1);

    // A vehicle's model is a fragment of its name, where the mount holds one.
    let index = indexed(&game, &root);
    let fragments: HashSet<String> = (index.files().iter())
        .filter_map(|(_, logical)| {
            let leaf = logical.rsplit('/').next()?.to_ascii_lowercase();
            leaf.strip_suffix(".yft").map(str::to_string)
        })
        .collect();
    let held = models.iter().filter(|model| fragments.contains(*model));
    let began = Instant::now();
    let asked = total(VehicleQuery {
        text: Some("a"),
        ..VehicleQuery::default()
    });
    let answered = began.elapsed();
    eprintln!(
        "\n=== {} vehicle(s) from {} data file(s) read in {:.3} s, {replaced} statement(s) replaced by a later \
         file's; {} class(es), {} type(s), {} pack(s); {} have a fragment of their name; a text of one letter lists \
         {asked} in {:.3} ms ===",
        list.vehicles.len(),
        list.files.len(),
        read.as_secs_f64(),
        classes.len(),
        types.len(),
        packs.len(),
        held.count(),
        answered.as_secs_f64() * 1e3,
    );
}
