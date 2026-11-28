//! `openrage-cli maps` lists what a project's map sets place and declare, validates them and reads a build back, and the one `project`
//! plan and build take the maps content as they take any other.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_maps::import::{add_files, Incoming};
use or_maps::sample::{manifest_file, map_file, types_file, Archetype, Entity, Stated};

fn cli(group: &str, args: &[&str]) -> ExitCode {
    let mut all = vec![group.to_string()];
    all.extend(args.iter().map(|a| a.to_string()));
    or_cli::run(&all)
}

fn text(path: &Path) -> &str {
    path.to_str().expect("a scratch path is UTF-8")
}

/// A project with a maps content whose pack holds a map file placing two crates and a type file declaring them.
fn a_project_with_maps(dir: &Path) -> PathBuf {
    std::fs::write(
        dir.join("harbour.openrage-proj"),
        r#"{
  "documents": [],
  "game": "both",
  "modules": [{ "id": "maps", "path": "maps", "type": "maps" }],
  "open": [],
  "outputs": [{ "id": "fivem", "kind": "fivem-resource", "name": "harbour", "take": ["maps"] }],
  "schema": 4
}"#,
    )
    .unwrap();
    let map = map_file(
        "harbour",
        &[
            Entity::at("prop_crate_a", [10.0, 20.0, 0.5], 1),
            Entity::at("prop_crate_a", [14.0, 20.0, 0.5], 2),
        ],
        ([8.0, 18.0, -1.0], [16.0, 22.0, 2.0]),
    );
    let types = types_file(
        "harbour_types",
        &[Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3])],
    );
    let added = add_files(
        &dir.join("maps"),
        &[
            Incoming {
                name: "harbour.ymap",
                bytes: &map,
                from_game: false,
                virtual_path: None,
            },
            Incoming {
                name: "harbour_types.ytyp",
                bytes: &types,
                from_game: false,
                virtual_path: None,
            },
        ],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    dir.join("harbour.openrage-proj")
}

#[test]
fn a_map_pack_is_listed_and_validated_and_a_word_that_is_no_command_is_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = a_project_with_maps(dir.path());
    assert_eq!(cli("maps", &["list", text(&file)]), ExitCode::SUCCESS);
    assert_eq!(cli("maps", &["validate", text(&file)]), ExitCode::SUCCESS);
    assert_eq!(
        cli("maps", &["validate", text(&file), "--json"]),
        ExitCode::SUCCESS
    );
    assert_ne!(cli("maps", &["dance"]), ExitCode::SUCCESS);
    assert_eq!(cli("maps", &[]), ExitCode::from(2));
}

#[test]
fn a_pack_with_a_missing_source_is_refused_and_so_is_a_project_with_no_maps() {
    let dir = tempfile::tempdir().unwrap();
    let file = a_project_with_maps(dir.path());
    let folder = dir.path().join("maps");
    let mut model = or_maps::Maps::read(&folder).unwrap();
    model.maps[0].sources.push("gone.metadata.src".into());
    model.write(&folder).unwrap();
    assert_eq!(cli("maps", &["validate", text(&file)]), ExitCode::FAILURE);

    let empty = tempfile::tempdir().unwrap();
    std::fs::write(
        empty.path().join("none.openrage-proj"),
        r#"{ "documents": [], "game": "both", "modules": [], "open": [], "outputs": [], "schema": 4 }"#,
    )
    .unwrap();
    assert_eq!(
        cli(
            "maps",
            &["list", text(&empty.path().join("none.openrage-proj"))]
        ),
        ExitCode::FAILURE
    );
}

#[test]
fn a_build_of_maps_is_read_back_against_the_sets_and_a_tampered_one_fails() {
    let dir = tempfile::tempdir().unwrap();
    let file = a_project_with_maps(dir.path());
    assert_eq!(
        cli("maps", &["verify", text(&file)]),
        ExitCode::FAILURE,
        "nothing is built yet, so there is nothing to read"
    );
    assert_eq!(cli("project", &["build", text(&file)]), ExitCode::SUCCESS);
    assert_eq!(cli("maps", &["verify", text(&file)]), ExitCode::SUCCESS);

    let shipped = dir
        .path()
        .join("build")
        .join("harbour")
        .join("stream")
        .join("harbour.ymap");
    assert!(shipped.is_file(), "the map file ships in the stream folder");
    std::fs::write(&shipped, b"not a map").unwrap();
    assert_eq!(
        cli("maps", &["verify", text(&file)]),
        ExitCode::FAILURE,
        "the file the plan gave the source does not read as a map"
    );
}

/// A manifest that states `harbour`, written to `dir` as a file a person took out of a game.
fn a_manifest_of_a_game(dir: &Path) -> PathBuf {
    let path = dir.join("_manifest.ymf");
    std::fs::write(
        &path,
        manifest_file(&[Stated {
            map: "harbour",
            flags: 0,
            types: &[],
        }]),
    )
    .unwrap();
    path
}

#[test]
fn a_kept_manifest_makes_a_build_write_the_manifest_of_its_maps_and_one_removed_fails_the_check() {
    let dir = tempfile::tempdir().unwrap();
    let file = a_project_with_maps(dir.path());
    let built = dir.path().join("build").join("harbour").join("stream");

    // With none kept the build writes no manifest of the maps, and the check asks for none.
    assert_eq!(cli("project", &["build", text(&file)]), ExitCode::SUCCESS);
    assert!(!built.join("_manifest.ymf").exists());
    assert_eq!(cli("maps", &["verify", text(&file)]), ExitCode::SUCCESS);

    let donor = a_manifest_of_a_game(dir.path());
    assert_eq!(
        cli("maps", &["keep-manifest", text(&file), text(&donor)]),
        ExitCode::SUCCESS
    );
    let kept: Vec<_> = std::fs::read_dir(dir.path().join("maps").join("manifests"))
        .expect("the content keeps the manifest in a folder of its own")
        .flatten()
        .collect();
    assert_eq!(kept.len(), 1);
    assert_eq!(
        cli("maps", &["keep-manifest", text(&file), text(&donor)]),
        ExitCode::SUCCESS,
        "kept twice, it is kept once"
    );
    assert_eq!(
        std::fs::read_dir(dir.path().join("maps").join("manifests"))
            .unwrap()
            .count(),
        1
    );

    assert_eq!(cli("project", &["build", text(&file)]), ExitCode::SUCCESS);
    let written = built.join("_manifest.ymf");
    assert!(
        written.is_file(),
        "the build writes the manifest of its maps"
    );
    assert_eq!(cli("maps", &["verify", text(&file)]), ExitCode::SUCCESS);

    std::fs::remove_file(&written).unwrap();
    assert_eq!(
        cli("maps", &["verify", text(&file)]),
        ExitCode::FAILURE,
        "a build whose manifest was removed does not hold what its maps need"
    );
}

#[test]
fn a_file_that_is_no_manifest_is_not_kept_and_the_command_says_what_it_takes() {
    let dir = tempfile::tempdir().unwrap();
    let file = a_project_with_maps(dir.path());
    let other = dir.path().join("not_one.ymf");
    std::fs::write(&other, b"not a manifest").unwrap();
    assert_eq!(
        cli("maps", &["keep-manifest", text(&file), text(&other)]),
        ExitCode::FAILURE
    );
    assert!(!dir.path().join("maps").join("manifests").exists());
    assert_eq!(
        cli("maps", &["keep-manifest", text(&file)]),
        ExitCode::from(2),
        "neither a file nor an install is named"
    );
    let donor = a_manifest_of_a_game(dir.path());
    assert_eq!(
        cli(
            "maps",
            &[
                "keep-manifest",
                text(&file),
                text(&donor),
                "--game",
                text(dir.path())
            ]
        ),
        ExitCode::from(2),
        "both a file and an install are named"
    );
}

/// The install mounted with every pack its content list names, none when it cannot be.
fn mounted(root: &Path) -> Option<or_title::MountedGame> {
    use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
    use std::sync::Arc;
    let exe = or_title::game::Game::detect_from_root(root)?.find_executable(root)?;
    let keys = extract_keys(&exe, &|_| {}).ok()?;
    let crypto: Arc<dyn or_io::container::RpfCryptoProvider> =
        Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys)));
    or_title::build_mounted_game(
        &or_title::GtaInstall::from_path(root)?,
        crypto,
        or_title::DlcCutoff::All,
    )
    .ok()
}

/// Point `OPENRAGE_GAME_ROOT` at an install root. Without one this skips, which is not a pass.
///
/// ⚠ The install is opened read-only: the map and the manifests are copied into a scratch project.
#[test]
fn a_map_taken_from_an_install_is_stated_as_the_install_states_it_from_the_manifests_kept_of_it() {
    use std::collections::BTreeSet;

    const WHAT: &str = "a_map_taken_from_an_install_is_stated_as_the_install_states_it";
    let Some(root) = or_test_support::game_roots().into_iter().next() else {
        or_test_support::skip_install(
            WHAT,
            format!("{} names no install", or_test_support::GAME_ROOT_ENV),
        );
        return;
    };
    let game = mounted(&root).expect("the install mounts");
    let (mut manifests, mut maps) = (Vec::new(), Vec::new());
    game.for_each_file(|logical, _, leaf| {
        let leaf = leaf.to_ascii_lowercase();
        if leaf.ends_with(".ymf") {
            manifests.push(logical.to_string());
        } else if let Some(stem) = leaf.strip_suffix(".ymap") {
            maps.push((or_io::string_hash(stem.as_bytes()), logical.to_string()));
        }
    });
    manifests.sort();
    maps.sort();

    // What every manifest of the install states of each map, joined, as the game joins them.
    let mut stated: std::collections::BTreeMap<u32, BTreeSet<u32>> = Default::default();
    for path in &manifests {
        let Some(bytes) = game.vfs.open(path) else {
            continue;
        };
        let Ok(said) = or_world_gta5::manifest_build::stated_in(&bytes) else {
            continue;
        };
        for (map, types) in said {
            stated.entry(map).or_default().extend(types);
        }
    }
    assert!(!stated.is_empty(), "no manifest of the install reads");

    // The first few stated maps whose file becomes a source: a file whose XML does not reach its fixed point is refused and passed by.
    let mut checked = 0;
    for (name, logical) in maps.iter().filter(|(name, _)| stated.contains_key(name)) {
        if checked == 5 {
            break;
        }
        let Some(bytes) = game.vfs.open(logical) else {
            continue;
        };
        let leaf = logical.rsplit('/').next().unwrap_or(logical);
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("taken.openrage-proj");
        std::fs::write(
            &file,
            r#"{ "documents": [], "game": "both", "modules": [{ "id": "maps", "path": "maps", "type": "maps" }], "open": [],
  "outputs": [{ "id": "fivem", "kind": "fivem-resource", "name": "taken", "take": ["maps"] }], "schema": 4 }"#,
        )
        .unwrap();
        let folder = dir.path().join("maps");
        let added = add_files(
            &folder,
            &[Incoming {
                name: leaf,
                bytes: &bytes,
                from_game: true,
                virtual_path: Some(logical),
            }],
            None,
        )
        .unwrap();
        if !added.refused.is_empty() {
            continue;
        }
        assert_eq!(
            cli(
                "maps",
                &["keep-manifest", text(&file), "--game", text(&root)]
            ),
            ExitCode::SUCCESS,
            "{logical}"
        );
        let model = or_maps::Maps::read(&folder).unwrap();
        let facts = or_maps::inspect(&folder, &model, or_maps::Depth::Open);
        let written = or_maps::manifest::of(&model, &facts);
        let bytes = written
            .file
            .unwrap_or_else(|| panic!("{logical}: no manifest is written: {:?}", written.notes));
        let read =
            or_world_gta5::manifest_build::stated_in(&bytes).expect("the manifest reads back");
        assert_eq!(
            read.get(name),
            stated.get(name),
            "{logical}: the entry kept is what the install states"
        );
        checked += 1;
    }
    assert!(checked > 0, "no stated map of the install became a source");
    println!(
        "{checked} map(s) of {} stated as the install states them",
        root.display()
    );
}

/// A value of an entry as the reader prints it, with a set of flags printed as its bits alone.
///
/// ⚠ The type table of a shipped manifest names an enum for the flags of an entry, and the one the tree's writer lays out names
/// none, as for every PSO document it writes: the bits are the entry's statement, and the name is the table's.
fn printed(value: &or_io::meta::Value) -> String {
    match value {
        or_io::meta::Value::Bitset { bits, .. } => format!("bits {bits:?}"),
        other => format!("{other:?}"),
    }
}

/// The entries of both pairings of the manifest in `bytes`, each as the reader prints it, in one order whatever order the file has them in.
fn entries_of(bytes: &[u8]) -> Vec<String> {
    use or_io::meta::binary::hash_index::with_public_names_only;
    use or_io::meta::binary::parse_meta_binary_full;
    use or_world_gta5::{field, keys};
    with_public_names_only(|| {
        let (root, _) = parse_meta_binary_full(
            &or_io::meta::SchemaLibrary::default(),
            bytes,
            &mut or_io::meta::Diagnostics::lenient(),
        )
        .expect("the manifest reads");
        let mut entries: Vec<String> = [keys::IMAP_DEPENDENCIES_2, keys::IMAP_DEPENDENCIES]
            .into_iter()
            .flat_map(|key| field::structs_at(&root, key))
            .map(|entry| {
                let members: Vec<String> = (entry.fields.iter())
                    .map(|(name, value)| format!("{name}: {}", printed(value)))
                    .collect();
                format!("{} {{ {} }}", entry.type_name, members.join(", "))
            })
            .collect();
        entries.sort();
        entries
    })
}

/// The flags of each entry the manifest in `bytes` states for the map `name`.
fn flags_stated_for(bytes: &[u8], name: u32) -> Vec<u32> {
    use or_io::meta::binary::hash_index::with_public_names_only;
    use or_io::meta::binary::parse_meta_binary_full;
    use or_world_gta5::manifest_build::MANIFEST_FLAGS;
    use or_world_gta5::{field, keys};
    with_public_names_only(|| {
        let (root, _) = parse_meta_binary_full(
            &or_io::meta::SchemaLibrary::default(),
            bytes,
            &mut or_io::meta::Diagnostics::lenient(),
        )
        .expect("the manifest reads");
        field::structs_at(&root, keys::IMAP_DEPENDENCIES_2)
            .filter(|entry| field::hash_at(entry, keys::IMAP_NAME) == Some(name))
            .filter_map(|entry| field::u32_at(entry, MANIFEST_FLAGS))
            .collect()
    })
}

/// Every `_manifest.ymf` staged in the metadata corpus is one a manifest can be written from, and what is written from it states what
/// it states: each entry kept whole as for a map taken from the game, and each entry rewritten with the type files it states already.
///
/// ⚠ The staged files stay where they are staged: none is copied into the tree, and a build keeps one only in a person's project.
#[test]
fn every_staged_manifest_of_the_game_is_written_back_stating_what_it_states() {
    use or_world_gta5::manifest_build::{build, interior_flags, stated_in, BuildError, Wanted};

    const WHAT: &str = "every_staged_manifest_of_the_game_is_written_back_stating_what_it_states";
    let origin = or_test_support::owned_corpus_origin(
        "or-meta",
        env!("CARGO_MANIFEST_DIR"),
        "OPENRAGE_META_CORPUS",
        "binary",
    );
    if matches!(origin, or_test_support::CorpusOrigin::Absent(_)) {
        or_test_support::skip_corpus(WHAT, "no metadata corpus is staged");
        return;
    }
    let Some(files): Option<Vec<PathBuf>> =
        or_test_support::Corpus::with_root(origin.path(), "ymf")
            .all_targets()
            .find_or_skip(WHAT)
    else {
        return;
    };

    // A manifest that states no map: its type table leaves out the class of an entry, which a shipped table declares only where a list holds one.
    let bare = files.iter().find_map(|path| {
        let bytes = std::fs::read(path).ok()?;
        stated_in(&bytes).ok()?.is_empty().then_some(bytes)
    });
    let mut behind_a_bare_one = 0usize;
    let (mut read, mut other_container, mut stating, mut entries) =
        (0usize, 0usize, 0usize, 0usize);
    // The entries that carry flags, and those of them that set no bit: what a derivation of the flags would have to tell apart.
    let (mut flagged, mut flagless) = (0usize, 0usize);
    let mut failures = Vec::new();
    // The entries derived for a map no manifest states, and the manifests that hold no entry to shape one by.
    let (mut derived, mut shapeless) = (0usize, 0usize);
    // Every manifest that states a map, to be kept at once as the manifests of one install are.
    let mut every: Vec<Vec<u8>> = Vec::new();
    for path in &files {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let bytes = std::fs::read(path).expect("a staged file reads");
        let stated = match stated_in(&bytes) {
            Ok(stated) => stated,
            // The game ships a few manifests in another container, which a build does not write from.
            Err(BuildError::NotPso(..)) => {
                other_container += 1;
                continue;
            }
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        read += 1;
        if stated.is_empty() {
            continue;
        }
        stating += 1;
        entries += stated.len();
        let held = entries_of(&bytes);
        flagged += held.iter().filter(|e| e.contains("bits ")).count();
        flagless += held.iter().filter(|e| e.contains("bits 0,")).count();
        // Kept behind a manifest that states nothing, the entries are typed by the table of the manifest they came from.
        if let Some(bare) = &bare {
            let wanted: Vec<Wanted> = (stated.iter())
                .map(|(map, types)| Wanted {
                    name: *map,
                    keep_stated: true,
                    needs: types.clone(),
                    incomplete: false,
                    ..Wanted::default()
                })
                .collect();
            match build(&[bare, &bytes], &wanted) {
                Ok(built) if built.stated == stated => behind_a_bare_one += 1,
                Ok(_) => failures.push(format!(
                    "{name}: behind a manifest that states nothing it is written back stating something else"
                )),
                Err(e) => failures.push(format!(
                    "{name}: behind a manifest that states nothing: {e}"
                )),
            }
        }
        // A derivation that resolved nothing takes nothing away from what the manifest states, in either pairing.
        let short: Vec<Wanted> = (stated.keys())
            .map(|map| Wanted {
                name: *map,
                keep_stated: false,
                needs: Default::default(),
                incomplete: true,
                ..Wanted::default()
            })
            .collect();
        match build(&[&bytes], &short) {
            Ok(built) if built.stated == stated => {}
            Ok(_) => failures.push(format!(
                "{name}: rewritten from a derivation that resolved nothing it states something else"
            )),
            Err(e) => failures.push(format!(
                "{name}: rewritten from a derivation that resolved nothing: {e}"
            )),
        }
        // A map no manifest states takes an entry shaped by one of this manifest's: its type files and its derived flags read back,
        // typed by the table the game wrote, whichever way that table spells the flags.
        let made = or_io::string_hash(b"openrage_a_map_no_manifest_states");
        let needs = std::collections::BTreeSet::from([or_io::string_hash(
            b"openrage_a_type_file_no_manifest_states",
        )]);
        for (entities, interiors) in [(3, 0), (2, 2)] {
            let wanted = [Wanted {
                name: made,
                needs: needs.clone(),
                entities,
                interiors,
                ..Wanted::default()
            }];
            match build(&[&bytes], &wanted) {
                Ok(built)
                    if built.derived.contains(&made)
                        && built.stated.get(&made) == Some(&needs)
                        && flags_stated_for(&built.bytes, made)
                            == [interior_flags(entities, interiors)] =>
                {
                    derived += 1
                }
                Ok(built) => failures.push(format!(
                    "{name}: an entry derived for a map of {entities} entities, {interiors} of them interiors, reads back under the flags {:?}",
                    flags_stated_for(&built.bytes, made)
                )),
                // A manifest that states its maps by pairs alone holds no entry to shape one by.
                Err(BuildError::NoEntry(_)) => shapeless += 1,
                Err(e) => failures.push(format!("{name}: an entry derived: {e}")),
            }
        }
        for keep_stated in [true, false] {
            let wanted: Vec<Wanted> = (stated.iter())
                .map(|(map, types)| Wanted {
                    name: *map,
                    keep_stated,
                    needs: types.clone(),
                    incomplete: false,
                    ..Wanted::default()
                })
                .collect();
            match build(&[&bytes], &wanted) {
                // ⚠ Kept whole, an entry is every member the game states of it, its flags among them, and not only its type files.
                Ok(built) if keep_stated && entries_of(&built.bytes) != entries_of(&bytes) => {
                    failures.push(format!("{name}: an entry kept whole reads back changed"))
                }
                Ok(built) if built.stated == stated && built.unstated.is_empty() => {}
                Ok(built) => failures.push(format!(
                    "{name} (kept whole: {keep_stated}): written back it states {} map(s) and the file states {}",
                    built.stated.len(),
                    stated.len()
                )),
                Err(e) => failures.push(format!("{name} (kept whole: {keep_stated}): {e}")),
            }
        }
        every.push(bytes);
    }

    // ⚠ An update of the game restates entries of the pack it patches, so the manifests of one install state many maps more than
    // once: kept at once, an entry is written as the first manifest that states it states it, and none is written that they do not state.
    let (mut once, mut seen, mut stated_in_all) =
        (Vec::new(), std::collections::BTreeSet::new(), 0);
    for bytes in &every {
        let held = entries_of(bytes);
        stated_in_all += held.len();
        once.extend(held.iter().filter(|entry| !seen.contains(*entry)).cloned());
        seen.extend(held);
    }
    once.sort();
    let names: std::collections::BTreeSet<u32> = (every.iter())
        .filter_map(|bytes| stated_in(bytes).ok())
        .flat_map(|stated| stated.into_keys())
        .collect();
    let wanted: Vec<Wanted> = (names.iter())
        .map(|map| Wanted {
            name: *map,
            keep_stated: true,
            needs: Default::default(),
            incomplete: false,
            ..Wanted::default()
        })
        .collect();
    let kept: Vec<&[u8]> = every.iter().map(Vec::as_slice).collect();
    match build(&kept, &wanted) {
        Ok(built) if entries_of(&built.bytes) == once => {}
        Ok(built) => failures.push(format!(
            "kept at once, the manifests state {} entries no earlier one states and {} are written",
            once.len(),
            entries_of(&built.bytes).len()
        )),
        Err(e) => failures.push(format!("kept at once: {e}")),
    }
    println!(
        "{} staged .ymf: {read} read as a manifest a build writes from, {other_container} in another container, {stating} state {entries} map(s) in {flagged} entries with flags, {flagless} of which set none; {behind_a_bare_one} written back behind a manifest that states nothing; {derived} entries derived for a map none states read back with their flags, and {shapeless} were not derived for want of an entry to shape one by; kept at once they state {} map name(s) in {stated_in_all} entries, {} of them stated by no earlier manifest",
        files.len(),
        names.len(),
        once.len()
    );
    assert!(
        failures.is_empty(),
        "{} of {} staged manifests: {:#?}",
        failures.len(),
        files.len(),
        &failures[..failures.len().min(10)]
    );
    assert!(
        stating > 0,
        "no staged manifest states a map, so nothing was written back"
    );
    assert_eq!(
        behind_a_bare_one, stating,
        "no staged manifest states nothing, so the join of two type tables went unexercised"
    );
    assert!(
        derived > 0,
        "no staged manifest holds an entry to shape one by, so no derived entry was read back"
    );
}

/// ⚠ Needs an install (`OPENRAGE_GAME_ROOT`): the one place the check of a resource folder asks a mounted game about a name.
#[test]
fn a_resource_checked_with_an_install_asks_the_game_about_what_it_places_and_does_not_declare() {
    const WHAT: &str = "project check-resource --game over an install";
    let Some(root) = or_test_support::game_roots().into_iter().next() else {
        or_test_support::skip_install(WHAT, "set OPENRAGE_GAME_ROOT to a GTA V install");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("bench");
    std::fs::create_dir_all(folder.join("stream")).unwrap();
    std::fs::write(
        folder.join("fxmanifest.lua"),
        "fx_version 'cerulean'\ngame 'gta5'\nthis_is_a_map 'yes'\n",
    )
    .unwrap();
    // A bench the game declares, and a name nothing declares.
    let map = map_file(
        "bench",
        &[
            Entity::at("prop_bench_01a", [10.0, 20.0, 0.5], 1),
            Entity::at("openrage_nobody_declares_this", [14.0, 20.0, 0.5], 2),
        ],
        ([8.0, 18.0, -1.0], [16.0, 22.0, 2.0]),
    );
    std::fs::write(folder.join("stream/bench.ymap"), map).unwrap();

    let alone = or_cli::check_resource_folder(&folder, None, None, Default::default())
        .expect("a folder is read");
    let [note] = alone.findings.as_slice() else {
        panic!("one note was expected: {:#?}", alone.findings);
    };
    assert_eq!(note.code, "archetype-undeclared");
    assert!(
        note.message.starts_with("2 archetype(s)"),
        "{}",
        note.message
    );

    let asked = match or_cli::check_resource_folder(&folder, Some(&root), None, Default::default())
    {
        Ok(report) => report,
        Err(why) => {
            or_test_support::skip_install(
                WHAT,
                format_args!("{} does not mount: {why}", root.display()),
            );
            return;
        }
    };
    let [warning] = asked.findings.as_slice() else {
        panic!("one warning was expected: {:#?}", asked.findings);
    };
    assert_eq!(warning.code, "archetype-missing");
    assert!(
        warning.message.starts_with("1 archetype(s)"),
        "the game declares the bench and not the other: {}",
        warning.message
    );
}

const DOORS_CHILD_ENV: &str = "OPENRAGE_MAPS_DOORS_OF_AN_INSTALL_ARGS";
const DOORS_CHILD_TEST: &str = "run_maps_doors_of_an_install_for_a_parent";
const DOORS_SEPARATOR: char = '\u{1f}';

/// Not a test of its own: the body a parent's child process runs, since the command prints with `println!`.
#[test]
fn run_maps_doors_of_an_install_for_a_parent() {
    let Some(args) = std::env::var_os(DOORS_CHILD_ENV) else {
        return;
    };
    let args: Vec<String> = (args.to_string_lossy().split(DOORS_SEPARATOR))
        .map(str::to_string)
        .collect();
    if or_cli::run(&args) != ExitCode::SUCCESS {
        std::process::exit(1);
    }
}

/// How many maps of an install the suite of `maps doors --game` lists the doors of, spread over the maps by name.
const MAPS_LISTED: usize = 12;

/// ⚠ Needs an install (`OPENRAGE_GAME_ROOT`). It is opened read-only: the index and the keys the command keeps go to a scratch folder.
#[test]
fn the_doors_of_maps_of_an_install_are_listed_from_the_command_line() {
    use std::collections::BTreeMap;
    const WHAT: &str = "the_doors_of_maps_of_an_install_are_listed_from_the_command_line";
    let Some(root) = or_test_support::game_roots().into_iter().next() else {
        or_test_support::skip_install(
            WHAT,
            format!("{} names no install", or_test_support::GAME_ROOT_ENV),
        );
        return;
    };
    let Some(game) = mounted(&root) else {
        or_test_support::skip_install(WHAT, format!("{} does not mount", root.display()));
        return;
    };
    // The last path the mount lists of each map file. ⚠ Where a title update patches a pack's file, that is the pack's own path,
    // which answers the patched copy, and the world index lists that copy at its own path.
    let mut winners: BTreeMap<String, String> = BTreeMap::new();
    game.for_each_file(|logical, _, leaf| {
        let leaf = leaf.to_ascii_lowercase();
        if leaf.ends_with(".ymap") {
            winners.insert(leaf, logical.to_string());
        }
    });
    assert!(!winners.is_empty(), "the mount lists no map file");
    let every = (winners.len() / MAPS_LISTED).max(1);
    let sampled: Vec<&String> = (winners.values().step_by(every))
        .take(MAPS_LISTED)
        .collect();

    let scratch = tempfile::tempdir().unwrap();
    let config = scratch.path().join("config");
    std::fs::create_dir(&config).unwrap();
    let kept = scratch.path().join("world.index");
    let (mut entities, mut doors, mut read, mut unnamed) = (0, 0, 0, 0);
    let (mut by_extension, mut types_unread, mut at_a_patch) = (0, 0, 0);
    for logical in &sampled {
        let args = [
            "maps",
            "doors",
            "--game",
            text(&root),
            "--map",
            logical.as_str(),
            "--kept",
            text(&kept),
            "--json",
        ];
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                DOORS_CHILD_TEST,
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(DOORS_CHILD_ENV, args.join(&DOORS_SEPARATOR.to_string()))
            .env("OPENRAGE_CONFIG_DIR", &config)
            .output()
            .expect("this test binary runs as a child");
        let printed = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "a map file the mount lists is refused: {printed}{}",
            String::from_utf8_lossy(&output.stderr)
        );
        // The harness prints lines of its own around the array.
        let from = printed.find("[\n").expect("an array begins a line");
        let to = printed.rfind("\n]").expect("and ends one") + 2;
        let listed: serde_json::Value = serde_json::from_str(&printed[from..to]).unwrap();
        // The file is named as the index spells it: the path asked, or the path of the copy that lies where the path asked does.
        let named = listed[0]["map"].as_str().expect("the map is named");
        if !named.eq_ignore_ascii_case(logical) {
            let lies = |path: &str| game.vfs.locate(path);
            assert!(lies(named).is_some(), "{named} lies nowhere");
            assert_eq!(lies(named), lies(logical), "{logical} is listed as {named}");
            at_a_patch += 1;
        }
        let set = &listed[0]["sets"][0];
        entities += set["entities"].as_u64().expect("entities");
        unnamed += set["unnamed"].as_u64().expect("unnamed");
        types_unread += set["unread"].as_array().map_or(0, Vec::len);
        for door in set["doors"].as_array().expect("doors") {
            doors += 1;
            read += u64::from(!door["leaf"].is_null());
            by_extension += u64::from(door["found_by"] == "extension");
        }
    }
    eprintln!(
        "\n=== {}: maps doors --game listed {} of {} map file(s) (one in {every}): {entities} entit(ies), {unnamed} with no \
         name and no door extension; {doors} door(s), {by_extension} by the door extension, {read} with a leaf read from \
         its box; {types_unread} type file(s) not read; {at_a_patch} map file(s) listed at the path of the copy that patches the \
         one asked ===",
        root.display(),
        sampled.len(),
        winners.len(),
    );
    assert!(
        entities > 0,
        "no entity was looked at in {} maps",
        sampled.len()
    );
    assert_eq!(types_unread, 0, "a type file the index names did not read");
}
