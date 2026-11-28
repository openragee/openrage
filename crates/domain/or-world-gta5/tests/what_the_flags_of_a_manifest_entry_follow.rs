//! What the flags of an entry of a map manifest follow from, over every `_manifest.ymf` of a mounted install: each candidate rule
//! with the entries it agrees with and the entries it does not, for the entries that state a map and for the ones that state a type
//! file. It is the measurement an entry written for a map no manifest of the game states is derived from
//! (`or_world_gta5::manifest_build`).
//!
//! Point `OPENRAGE_GAME_ROOT` at an install root. Without one this skips, which is not a pass.
//!
//! ⚠ The install is opened read-only and nothing here writes into one. It prints counts, and never a name of the game.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
use or_io::container::RpfCryptoProvider;
use or_io::meta::binary::parse_meta_binary_full;
use or_io::meta::{Diagnostics, MetaInstance, Value};
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::game::Game;
use or_title::{build_mounted_game, DlcCutoff, GtaInstall, MountedGame};
use or_world_gta5::content_flags::content;
use or_world_gta5::manifest_build::{interior_flags, MANIFEST_FLAGS};
use or_world_gta5::map_data::MapData;
use or_world_gta5::map_types::{ArchetypeKind, MapTypes};
use or_world_gta5::{field, keys, Manifest};

const WHAT: &str = "what_the_flags_of_a_manifest_entry_follow";

/// The threads the map files and type files are read on.
const READERS: usize = 12;

fn mount(root: &Path) -> Option<MountedGame> {
    let exe = Game::detect_from_root(root)?.find_executable(root)?;
    let keys = extract_keys(&exe, &|_| {}).ok()?;
    let crypto: Arc<dyn RpfCryptoProvider> =
        Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys)));
    build_mounted_game(&GtaInstall::from_path(root)?, crypto, DlcCutoff::All).ok()
}

/// What one map file places, as far as a candidate asks.
#[derive(Clone, Copy)]
struct Placed {
    entities: usize,
    interiors: usize,
    content_flags: u32,
}

/// The name a file is streamed by and the archive it is directly inside.
fn name_and_archive(logical: &str) -> (u32, &str) {
    let (archive, leaf) = logical.rsplit_once('/').unwrap_or(("", logical));
    let stem = leaf.rsplit_once('.').map_or(leaf, |(stem, _)| stem);
    (
        or_io::string_hash(stem.to_ascii_lowercase().as_bytes()),
        archive,
    )
}

/// The names an entry lists under `key`.
fn listed(entry: &MetaInstance, key: u32) -> BTreeSet<u32> {
    (field::array_at(entry, key).iter())
        .filter_map(|value| match value {
            Value::Hash(name) => Some(name.hash),
            Value::UInt(name) => Some(*name as u32),
            _ => None,
        })
        .collect()
}

/// A candidate rule: the entries whose flags it gives, and the entries it gives other flags for.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
struct Tally {
    agrees: usize,
    disagrees: usize,
}

impl Tally {
    fn count(&mut self, agrees: bool) {
        match agrees {
            true => self.agrees += 1,
            false => self.disagrees += 1,
        }
    }
}

#[derive(Default)]
struct Figures {
    manifests: usize,
    manifests_read: usize,
    map_files: usize,
    type_files: usize,
    unread: usize,
    /// The entries that state a map with a list of type files, and each value their flags take.
    map_entries: usize,
    map_flags: BTreeMap<u32, usize>,
    /// Where the map an entry states is: in the archive of its manifest, in another archive, in no file of the install.
    beside: usize,
    elsewhere: usize,
    nowhere: usize,
    without_a_type_file: usize,
    candidates: BTreeMap<&'static str, Tally>,
    /// The rule the writer takes, [`interior_flags`], against the flags each entry states.
    written: Tally,
    /// The pairs of a map and a type file, which state no flags.
    pairs: usize,
    map_names: usize,
    map_names_unstated: usize,
    map_names_unstated_empty: usize,
    /// The entries that state a type file, and each value their flags take.
    type_entries: usize,
    type_flags: BTreeMap<u32, usize>,
    type_nowhere: usize,
    type_candidates: BTreeMap<&'static str, Tally>,
    interior_type_files: usize,
    interior_type_files_stated: usize,
}

const EVERY: &str = "every entity the map places is an interior, and it places one";
const ANY: &str = "the map places an interior";
const CONTENT: &str = "the content flags of the map say it holds an interior";
const NAMED: &str = "a type file the entry names declares an interior";
const NO_OTHER: &str = "the map places nothing that is not an interior";
const DECLARES: &str = "the type file declares an interior";
const ONLY: &str = "the type file declares nothing but interiors";

fn measure(game: &MountedGame) -> Figures {
    let library = support::schema();
    let (mut manifests, mut maps, mut types) = (Vec::new(), Vec::new(), Vec::new());
    game.for_each_file(|logical, _, leaf| {
        let lower = leaf.to_ascii_lowercase();
        if lower.ends_with(".ymf") {
            manifests.push(logical.to_string());
        } else if lower.ends_with(".ymap") {
            maps.push(logical.to_string());
        } else if lower.ends_with(".ytyp") {
            types.push(logical.to_string());
        }
    });

    // Each copy of a map by its name, with the archive it is in, and each type file by its name: how many archetypes, how many interiors.
    let placed: Mutex<BTreeMap<u32, Vec<(String, Placed)>>> = Mutex::new(BTreeMap::new());
    let declared: Mutex<BTreeMap<u32, Vec<(usize, usize)>>> = Mutex::new(BTreeMap::new());
    let unread = AtomicUsize::new(0);
    let next = AtomicUsize::new(0);
    let files: Vec<&String> = maps.iter().chain(&types).collect();
    std::thread::scope(|scope| {
        for _ in 0..READERS {
            scope.spawn(|| {
                while let Some(logical) = files.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let read = game.vfs.open(logical).and_then(|bytes| {
                        parse_meta_binary_full(library, &bytes, &mut Diagnostics::lenient()).ok()
                    });
                    let Some((root, _)) = read else {
                        unread.fetch_add(1, Ordering::Relaxed);
                        continue;
                    };
                    let (name, archive) = name_and_archive(logical);
                    if let Some(map) = MapData::new(&root) {
                        let facts = Placed {
                            entities: map.entity_count(),
                            interiors: map.entities().filter(|e| e.is_interior_instance()).count(),
                            content_flags: map.content_flags(),
                        };
                        let mut placed = placed.lock().expect("no reader panicked");
                        (placed.entry(name).or_default()).push((archive.to_string(), facts));
                    } else if let Some(file) = MapTypes::new(&root) {
                        let interiors = (file.archetypes())
                            .filter(|a| a.kind() == ArchetypeKind::Mlo)
                            .count();
                        let mut declared = declared.lock().expect("no reader panicked");
                        (declared.entry(name).or_default())
                            .push((file.archetype_count(), interiors));
                    } else {
                        unread.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
        }
    });
    let placed = placed.into_inner().expect("no reader panicked");
    let declared = declared.into_inner().expect("no reader panicked");

    let mut figures = Figures {
        manifests: manifests.len(),
        map_files: maps.len(),
        type_files: types.len(),
        unread: unread.into_inner(),
        map_names: placed.len(),
        ..Figures::default()
    };
    let type_entries_key = field::name_hash("itypDependencies_2");
    let declares_an_interior = |name: &u32| {
        (declared.get(name)).is_some_and(|files| files.iter().any(|(_, mlo)| *mlo > 0))
    };
    let mut stated_maps: BTreeSet<u32> = BTreeSet::new();
    let mut stated_types: BTreeSet<u32> = BTreeSet::new();
    for path in &manifests {
        let read = game.vfs.open(path).and_then(|bytes| {
            parse_meta_binary_full(library, &bytes, &mut Diagnostics::lenient()).ok()
        });
        let Some((root, _)) = read.filter(|(root, _)| Manifest::new(root).is_some()) else {
            continue;
        };
        figures.manifests_read += 1;
        let archive = path.rsplit_once('/').map_or("", |(archive, _)| archive);
        let pairs = Manifest::new(&root)
            .into_iter()
            .flat_map(|manifest| manifest.flat_imap_dependencies());
        for (map, _) in pairs {
            figures.pairs += 1;
            stated_maps.insert(map);
        }
        for entry in field::structs_at(&root, keys::IMAP_DEPENDENCIES_2) {
            let Some(name) = field::hash_at(entry, keys::IMAP_NAME) else {
                continue;
            };
            let flags = field::u32_at(entry, MANIFEST_FLAGS).unwrap_or(0);
            let names = listed(entry, keys::ITYP_DEP_ARRAY);
            figures.map_entries += 1;
            *figures.map_flags.entry(flags).or_default() += 1;
            figures.without_a_type_file += usize::from(names.is_empty());
            stated_maps.insert(name);
            // ⚠ The copy an entry is about is the one in its manifest's own archive; with none there, every copy of the name has to agree.
            let copies = placed.get(&name).map_or(&[][..], Vec::as_slice);
            let own: Vec<Placed> = (copies.iter())
                .filter(|(held, _)| held == archive)
                .map(|(_, facts)| *facts)
                .collect();
            let about: Vec<Placed> = match (own.is_empty(), copies.is_empty()) {
                (false, _) => {
                    figures.beside += 1;
                    own
                }
                (true, false) => {
                    figures.elsewhere += 1;
                    copies.iter().map(|(_, facts)| *facts).collect()
                }
                (true, true) => {
                    figures.nowhere += 1;
                    continue;
                }
            };
            let set = flags != 0;
            let every = |says: &dyn Fn(&Placed) -> bool| about.iter().all(|map| says(map) == set);
            for (candidate, agrees) in [
                (
                    EVERY,
                    every(&|m| m.interiors > 0 && m.interiors == m.entities),
                ),
                (ANY, every(&|m| m.interiors > 0)),
                (
                    CONTENT,
                    every(&|m| m.content_flags & content::INTERIOR != 0),
                ),
                (NAMED, names.iter().any(declares_an_interior) == set),
                (NO_OTHER, every(&|m| m.interiors == m.entities)),
            ] {
                (figures.candidates.entry(candidate).or_default()).count(agrees);
            }
            let written =
                (about.iter()).all(|map| interior_flags(map.entities, map.interiors) == flags);
            figures.written.count(written);
        }
        for entry in field::structs_at(&root, type_entries_key) {
            let Some(name) = field::hash_at(entry, keys::ITYP_NAME) else {
                continue;
            };
            let flags = field::u32_at(entry, MANIFEST_FLAGS).unwrap_or(0);
            figures.type_entries += 1;
            *figures.type_flags.entry(flags).or_default() += 1;
            stated_types.insert(name);
            let Some(files) = declared.get(&name) else {
                figures.type_nowhere += 1;
                continue;
            };
            let set = flags != 0;
            for (candidate, agrees) in [
                (DECLARES, files.iter().all(|(_, mlo)| (*mlo > 0) == set)),
                (
                    ONLY,
                    (files.iter()).all(|(all, mlo)| (*mlo > 0 && mlo == all) == set),
                ),
            ] {
                (figures.type_candidates.entry(candidate).or_default()).count(agrees);
            }
        }
    }
    for (name, copies) in &placed {
        if !stated_maps.contains(name) {
            figures.map_names_unstated += 1;
            figures.map_names_unstated_empty +=
                usize::from(copies.iter().all(|(_, map)| map.entities == 0));
        }
    }
    for name in declared.keys().filter(|name| declares_an_interior(name)) {
        figures.interior_type_files += 1;
        figures.interior_type_files_stated += usize::from(stated_types.contains(name));
    }
    figures
}

fn report(f: &Figures) {
    println!(
        "{} of {} manifests read; {} map files under {} names and {} type files, {} of them unread",
        f.manifests_read, f.manifests, f.map_files, f.map_names, f.type_files, f.unread
    );
    println!(
        "{} entries state a map with its type files, by the value of their flags {:?}; {} of them name no type file",
        f.map_entries, f.map_flags, f.without_a_type_file
    );
    println!(
        "  the map of {} is in the archive of its manifest, of {} in another archive, of {} in no file of the install",
        f.beside, f.elsewhere, f.nowhere
    );
    println!(
        "  the flags are set exactly where, of the {} whose map is held:",
        f.beside + f.elsewhere
    );
    for (candidate, tally) in &f.candidates {
        println!(
            "    {:6} agree, {:6} do not: {candidate}",
            tally.agrees, tally.disagrees
        );
    }
    println!(
        "  the flags the writer derives are the stated ones on {} and not on {}",
        f.written.agrees, f.written.disagrees
    );
    println!(
        "{} pairs of a map and a type file, which state no flags",
        f.pairs
    );
    println!(
        "{} of the {} map names are stated by no manifest; {} of those place no entity",
        f.map_names_unstated, f.map_names, f.map_names_unstated_empty
    );
    println!(
        "{} entries state a type file, by the value of their flags {:?}; the type file of {} is in no file of the install",
        f.type_entries, f.type_flags, f.type_nowhere
    );
    for (candidate, tally) in &f.type_candidates {
        println!(
            "    {:6} agree, {:6} do not: {candidate}",
            tally.agrees, tally.disagrees
        );
    }
    println!(
        "{} type file names declare an interior, and an entry states {} of them",
        f.interior_type_files, f.interior_type_files_stated
    );
}

#[test]
fn the_flags_of_an_entry_for_a_map_say_that_the_map_places_interiors_and_nothing_else() {
    let Some(root) = game_roots().into_iter().next() else {
        skip_install(WHAT, format!("{GAME_ROOT_ENV} names no install"));
        return;
    };
    let game = mount(&root).expect("the install mounts");
    let figures = measure(&game);
    report(&figures);
    let f = &figures;

    // Enough was read for the figures to mean something.
    assert!(f.manifests_read == f.manifests && f.unread == 0);
    assert!(
        f.map_entries >= 10_000 && f.type_entries >= 100,
        "too little was read"
    );
    assert_eq!(
        f.nowhere, 0,
        "an entry states a map no file of the install is"
    );
    // ⚠ The rule the writer takes holds on every entry, or an entry it writes for a map no manifest states is a guess.
    let held = f.beside + f.elsewhere;
    let all = Tally {
        agrees: held,
        disagrees: 0,
    };
    assert_eq!(f.candidates[EVERY], all);
    assert_eq!(f.written, all);
    assert!(
        f.map_flags.keys().all(|flags| *flags <= 1),
        "an entry states a flag the rule does not know: {:?}",
        f.map_flags
    );
    // An entry is written only for a map that needs a type file: the install states none without.
    assert_eq!(f.without_a_type_file, 0);
    // The candidates that are near and not the rule stay refuted, so nobody takes one for it.
    for near in [ANY, CONTENT, NAMED, NO_OTHER] {
        assert!(
            f.candidates[near].disagrees > 0,
            "{near} holds on every entry too"
        );
    }
    // An entry for a type file is stated of an interior's and of no other.
    assert_eq!(
        f.type_candidates[DECLARES],
        Tally {
            agrees: f.type_entries - f.type_nowhere,
            disagrees: 0
        }
    );
}
