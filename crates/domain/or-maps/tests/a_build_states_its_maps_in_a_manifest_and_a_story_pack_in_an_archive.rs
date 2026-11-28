//! What a build of maps adds for a server and for a story install: the manifest of its maps, written from a manifest of the game the
//! module keeps, the line that declares a FiveM resource a map, and, for a singleplayer DLC, the nested archives the pack registers.
//!
//! ⚠ Every file is made here from the sample module of the crate, the kept manifest included: no install is read and no game content
//! is in the repository. What the game's own manifests and packs hold is the install suites' to say; the pack built here is read
//! back through the count those suites run over an install (`common::packs`).

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::packs::{departures, measure, MAPS_ON_AT_START};
use or_io::container::rpf::crypto::NoOpCryptoProvider;
use or_io::container::rpf::{RpfArchive, RpfBuilder};
use or_io::decl::File;
use or_io::meta::instance::HashedString;
use or_io::meta::typed::{CMapData, CMapTypes};
use or_io::meta::{build_library, canonical, write_meta_binary, BinaryFormat, MetaInstance, Value};
use or_maps::import::{add_files, Incoming};
use or_maps::manifest::{self, FILE, KEPT};
use or_maps::model::Maps;
use or_maps::sample::{manifest_file, map_file, types_file, Archetype, Entity, Stated};
use or_maps::verify::verify;
use or_maps::{inspect, Depth, MapPack};
use or_project::content::Content;
use or_project::{
    build_with, cannot_ship, plan_with, Container, Contribution, Module, Origin, Output,
    OutputKind, Plan,
};
use or_source::sample::{drawable_dictionary, texture_dictionary};
use or_title::dlc::{ContentXml, DlcList};
use or_title::{build_mounted_game, DlcCutoff, GtaInstall};
use or_world_gta5::manifest_build::stated_in;

const CRATE_A: &str = "prop_crate_a";
const MANIFEST: &str = "stream/_manifest.ymf";
const ARCHIVE: &str = "x64/levels/gta5/maps.rpf";
/// The archive of a pack the type file of the sample module goes into with its model, and the name the pack asks for it by.
const PROPS: &str = "x64/levels/gta5/props/harbour_types.rpf";
const ASKED: &str = "x64/levels/gta5/props/harbour_types.ityp";

fn hash(name: &str) -> u32 {
    or_io::string_hash(name.as_bytes())
}

/// A module folder `dir/<id>` holding a map called `map` that places one crate, and the type file `types` that declares it.
fn module_named(dir: &Path, id: &str, map: &str, types: &str, from_game: bool) -> PathBuf {
    let folder = dir.join(id);
    // The crate is the content's own: an archetype's name is one the game resolves once across a project.
    let prop = format!("{CRATE_A}_{id}");
    let placed = map_file(
        map,
        &[Entity::at(&prop, [10.0, 20.0, 0.5], 1)],
        ([8.0, 18.0, -1.0], [12.0, 22.0, 2.0]),
    );
    let declared = types_file(types, &[Archetype::new(&prop, [-1.0; 3], [1.0; 3])]);
    let added = add_files(
        &folder,
        &[
            Incoming {
                name: &format!("{map}.ymap"),
                bytes: &placed,
                from_game,
                virtual_path: None,
            },
            Incoming {
                name: &format!("{types}.ytyp"),
                bytes: &declared,
                from_game: false,
                virtual_path: None,
            },
        ],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    folder
}

fn module(dir: &Path) -> PathBuf {
    module_named(dir, "maps", "harbour", "harbour_types", false)
}

/// Keep in `folder` a manifest that states `map` under `flags` with `types`.
fn keep(folder: &Path, map: &str, flags: u32, types: &[&str]) {
    let kept = manifest::keep(folder, &manifest_file(&[Stated { map, flags, types }]))
        .expect("a manifest is kept");
    assert!(kept.starts_with(folder.join(KEPT)), "{}", kept.display());
}

fn container(kind: OutputKind, modules: &[&str]) -> Container {
    let mut output = Output::fivem_resource("build", "harbour", modules);
    output.kind = kind;
    Container {
        modules: modules.iter().map(|id| Module::maps(id, id)).collect(),
        outputs: vec![output],
    }
}

/// What each maps content hands to the one build of `container`, asked as a project asks it: for that build.
fn handed(container: &Container, folder: &Path) -> Vec<Contribution> {
    (container.modules.iter())
        .map(|m| MapPack.contribution_for(folder, m, &container.outputs[0]))
        .collect()
}

fn plan(container: &Container, folder: &Path) -> Plan {
    plan_with(container, "build", folder, &handed(container, folder)).expect("the build exists")
}

fn cooker(source: &Path, scratch: &Path) -> Result<PathBuf, Vec<String>> {
    or_source::cook::cook_source(source, scratch)
}

fn document<'a>(plan: &'a Plan, path: &str) -> &'a str {
    match plan
        .files
        .iter()
        .find(|f| f.path == path)
        .map(|f| &f.origin)
    {
        Some(Origin::Document(text)) => text,
        other => panic!("`{path}` is no document of the plan: {other:?}"),
    }
}

#[test]
fn a_build_writes_a_manifest_that_reads_back_naming_the_type_file_its_map_needs() {
    let dir = tempfile::tempdir().unwrap();
    let folder = module(dir.path());
    // The game's entry says another type file than the one the map's crate is declared in, so only the derivation gives the answer.
    keep(&folder, "harbour", 4, &["t_stated_by_the_donor"]);
    let container = container(OutputKind::FivemResource, &["maps"]);
    let planned = plan(&container, dir.path());
    assert_eq!(planned.problems, Vec::<String>::new());
    assert_eq!(planned.notes, Vec::<String>::new());
    let paths: Vec<&str> = planned.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "fxmanifest.lua",
            MANIFEST,
            "stream/harbour.ymap",
            "stream/harbour_types.ytyp"
        ]
    );

    let root = dir.path().join("build");
    let built = build_with(&planned, &root, &Default::default(), Some(&cooker)).expect("it builds");
    assert_eq!(built.verification.problems, Vec::<String>::new());
    let reloaded = (built.verification.reloaded.iter())
        .find(|r| r.path == MANIFEST)
        .expect("the build reopened the manifest through a reader");
    assert!(reloaded.reader.contains("PSO"), "{}", reloaded.reader);

    let written = std::fs::read(built.folder.join(MANIFEST)).unwrap();
    assert_eq!(
        stated_in(&written).expect("the manifest reads back"),
        BTreeMap::from([(hash("harbour"), BTreeSet::from([hash("harbour_types")]))]),
        "the map the pack ships, with the type file that declares what it places"
    );

    let model = Maps::read(&folder).unwrap();
    let facts = inspect(&folder, &model, Depth::Open);
    let found = verify(&model, &facts, &built.folder);
    assert!(found.is_clean(), "{:?}", found.problems());

    // A second build of the same sources writes the same manifest.
    let again = build_with(
        &plan(&container, dir.path()),
        &root,
        &Default::default(),
        Some(&cooker),
    )
    .expect("the second build");
    assert_eq!(std::fs::read(again.folder.join(MANIFEST)).unwrap(), written);
}

#[test]
fn a_content_that_keeps_no_manifest_of_the_game_writes_none_and_the_plan_says_why() {
    let dir = tempfile::tempdir().unwrap();
    module(dir.path());
    let planned = plan(&container(OutputKind::FivemResource, &["maps"]), dir.path());
    assert_eq!(
        planned.problems,
        Vec::<String>::new(),
        "the build still runs"
    );
    assert!(planned.files.iter().all(|f| f.path != MANIFEST));
    let said = planned.notes.join("\n");
    assert!(
        said.contains("content `maps`") && said.contains(FILE) && said.contains(KEPT),
        "{said}"
    );
}

#[test]
fn a_kept_file_that_is_no_manifest_stops_the_manifest_and_the_plan_names_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let folder = module(dir.path());
    keep(&folder, "harbour", 5, &[]);
    // Put there by hand, past the check that keeps one: the name sorts before the kept one, which is named by its digest.
    std::fs::write(folder.join(KEPT).join("!by_hand.ymf"), b"no manifest").unwrap();
    let planned = plan(&container(OutputKind::FivemResource, &["maps"]), dir.path());
    assert_eq!(planned.problems, Vec::<String>::new());
    assert!(planned.files.iter().all(|f| f.path != MANIFEST));
    let [note] = planned.notes.as_slice() else {
        panic!("one note: {:?}", planned.notes)
    };
    assert!(
        note.contains(&format!("`{KEPT}/!by_hand.ymf`")) && note.contains("does not read"),
        "{note}"
    );
}

#[test]
fn a_map_no_kept_manifest_states_takes_an_entry_with_the_type_file_it_needs_and_the_flags_derived()
{
    let dir = tempfile::tempdir().unwrap();
    let folder = module(dir.path());
    // The kept manifest states another map, under the interior's flag and with another type file: it gives the entry its shape alone.
    keep(&folder, "another_map", 1, &["t_of_another_map"]);
    let planned = plan(&container(OutputKind::FivemResource, &["maps"]), dir.path());
    assert_eq!(planned.problems, Vec::<String>::new());
    assert_eq!(planned.notes, Vec::<String>::new());

    let model = Maps::read(&folder).unwrap();
    let stated = manifest::of(&model, &inspect(&folder, &model, Depth::Open));
    let written = stated
        .file
        .expect("a manifest is written for the map made here");
    assert_eq!(
        stated_in(&written).unwrap(),
        BTreeMap::from([(hash("harbour"), BTreeSet::from([hash("harbour_types")]))]),
        "the map of the content with the type file it places from, and nothing of the map the kept manifest states"
    );
    // The map places one crate and no interior: the flags are the ones a manifest of the game states of such a map.
    let flagged: Vec<Stated> = [0, 1]
        .into_iter()
        .map(|flags| Stated {
            map: "harbour",
            flags,
            types: &["harbour_types"],
        })
        .collect();
    let entry_of = |bytes: &[u8]| {
        let (root, _) = or_io::meta::binary::parse_meta_binary_full(
            &or_io::meta::SchemaLibrary::default(),
            bytes,
            &mut or_io::meta::Diagnostics::lenient(),
        )
        .expect("the manifest reads");
        let entries: Vec<Option<u32>> =
            or_world_gta5::field::structs_at(&root, or_world_gta5::keys::IMAP_DEPENDENCIES_2)
                .map(|entry| {
                    or_world_gta5::field::u32_at(
                        entry,
                        or_world_gta5::manifest_build::MANIFEST_FLAGS,
                    )
                })
                .collect();
        entries
    };
    assert_eq!(entry_of(&written), [Some(0)]);
    assert_eq!(
        entry_of(&written),
        entry_of(&manifest_file(&flagged[..1])),
        "as a manifest that states the map with no flag"
    );
    assert_ne!(entry_of(&written), entry_of(&manifest_file(&flagged[1..])));
}

#[test]
fn a_map_no_kept_manifest_states_that_needs_no_type_file_of_the_content_is_named_and_left_out() {
    let dir = tempfile::tempdir().unwrap();
    let folder = module(dir.path());
    let extents = ([8.0, 18.0, -1.0], [12.0, 22.0, 2.0]);
    // A second map that places a prop of the game alone, which no type file of the content declares.
    let second = map_file(
        "airfield",
        &[Entity::at("prop_of_the_game", [10.0, 20.0, 0.5], 1)],
        extents,
    );
    let added = add_files(
        &folder,
        &[Incoming {
            name: "airfield.ymap",
            bytes: &second,
            from_game: false,
            virtual_path: None,
        }],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    keep(&folder, "harbour", 4, &[]);

    let model = Maps::read(&folder).unwrap();
    let stated = manifest::of(&model, &inspect(&folder, &model, Depth::Open));
    let written = stated.file.expect("the map that is stated is written");
    assert_eq!(
        stated_in(&written).unwrap(),
        BTreeMap::from([(hash("harbour"), BTreeSet::from([hash("harbour_types")]))])
    );
    let [note] = stated.notes.as_slice() else {
        panic!("one note names the map left out: {:?}", stated.notes)
    };
    assert!(
        note.contains("`airfield`")
            && note.contains("needs no type file of this content")
            && !note.contains("harbour"),
        "{note}"
    );
}

#[test]
fn a_derived_entry_for_a_map_that_also_places_what_the_content_does_not_declare_is_said_to_be_short(
) {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("maps");
    let placed = map_file(
        "harbour",
        &[
            Entity::at(CRATE_A, [10.0, 20.0, 0.5], 1),
            Entity::at("prop_of_the_game", [11.0, 20.0, 0.5], 2),
        ],
        ([8.0, 18.0, -1.0], [12.0, 22.0, 2.0]),
    );
    let declared = types_file(
        "harbour_types",
        &[Archetype::new(CRATE_A, [-1.0; 3], [1.0; 3])],
    );
    let added = add_files(
        &folder,
        &[
            Incoming {
                name: "harbour.ymap",
                bytes: &placed,
                from_game: false,
                virtual_path: None,
            },
            Incoming {
                name: "harbour_types.ytyp",
                bytes: &declared,
                from_game: false,
                virtual_path: None,
            },
        ],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    keep(&folder, "another_map", 0, &["t_of_another_map"]);

    let model = Maps::read(&folder).unwrap();
    let stated = manifest::of(&model, &inspect(&folder, &model, Depth::Open));
    assert_eq!(
        stated_in(&stated.file.expect("a manifest is written")).unwrap(),
        BTreeMap::from([(hash("harbour"), BTreeSet::from([hash("harbour_types")]))])
    );
    let [note] = stated.notes.as_slice() else {
        panic!("one note says the entry is short: {:?}", stated.notes)
    };
    assert!(
        note.contains("`harbour`") && note.contains("type files of this content alone"),
        "{note}"
    );
}

#[test]
fn a_map_that_places_what_no_type_file_of_the_content_declares_keeps_what_the_game_states_too() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("maps");
    // One crate of the content's own and one prop of the game, which no type file of the content declares.
    let placed = map_file(
        "harbour",
        &[
            Entity::at(CRATE_A, [10.0, 20.0, 0.5], 1),
            Entity::at("prop_of_the_game", [11.0, 20.0, 0.5], 2),
        ],
        ([8.0, 18.0, -1.0], [12.0, 22.0, 2.0]),
    );
    let declared = types_file(
        "harbour_types",
        &[Archetype::new(CRATE_A, [-1.0; 3], [1.0; 3])],
    );
    let added = add_files(
        &folder,
        &[
            Incoming {
                name: "harbour.ymap",
                bytes: &placed,
                from_game: false,
                virtual_path: None,
            },
            Incoming {
                name: "harbour_types.ytyp",
                bytes: &declared,
                from_game: false,
                virtual_path: None,
            },
        ],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    keep(&folder, "harbour", 5, &["t_of_the_game"]);

    let model = Maps::read(&folder).unwrap();
    let stated = manifest::of(&model, &inspect(&folder, &model, Depth::Open));
    assert_eq!(
        stated_in(&stated.file.expect("a manifest is written")).unwrap(),
        BTreeMap::from([(
            hash("harbour"),
            BTreeSet::from([hash("t_of_the_game"), hash("harbour_types")])
        )]),
        "the type file found is added, and the one the prop of the game is declared in is not taken away"
    );
}

#[test]
fn a_manifest_of_the_game_handed_to_the_import_is_kept_and_a_file_that_is_none_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let folder = module(dir.path());
    let of_the_game = manifest_file(&[Stated {
        map: "harbour",
        flags: 1,
        types: &[],
    }]);
    let added = add_files(
        &folder,
        &[
            Incoming {
                name: "_manifest.ymf",
                bytes: &of_the_game,
                from_game: true,
                virtual_path: None,
            },
            Incoming {
                name: "other.ymf",
                bytes: b"no manifest",
                from_game: false,
                virtual_path: None,
            },
        ],
        None,
    )
    .unwrap();
    assert!(added.made.is_empty(), "a manifest is no source of a set");
    let [kept] = added.kept.as_slice() else {
        panic!("one manifest is kept: {:?}", added.kept)
    };
    assert_eq!(std::fs::read(kept).unwrap(), of_the_game);
    let [(name, why)] = added.refused.as_slice() else {
        panic!("one file is refused: {:?}", added.refused)
    };
    assert!(name == "other.ymf" && !why.is_empty(), "{name}: {why}");

    let model = Maps::read(&folder).unwrap();
    let facts = inspect(&folder, &model, Depth::Open);
    let held: Vec<&[u8]> = (facts.manifests.iter())
        .map(|kept| kept.bytes.as_deref().expect("it reads"))
        .collect();
    assert_eq!(held, [of_the_game.as_slice()]);
}

#[test]
fn a_built_manifest_that_is_gone_unreadable_or_states_other_flags_fails_the_check() {
    let dir = tempfile::tempdir().unwrap();
    let folder = module(dir.path());
    keep(&folder, "harbour", 5, &[]);
    let container = container(OutputKind::FivemResource, &["maps"]);
    let built = build_with(
        &plan(&container, dir.path()),
        &dir.path().join("build"),
        &Default::default(),
        Some(&cooker),
    )
    .expect("it builds");
    let model = Maps::read(&folder).unwrap();
    let facts = inspect(&folder, &model, Depth::Open);
    let problems = || verify(&model, &facts, &built.folder).problems().join("\n");
    assert_eq!(problems(), "");

    // The type files the build states, under flags the kept manifest does not state.
    let at = built.folder.join(MANIFEST);
    std::fs::write(
        &at,
        manifest_file(&[Stated {
            map: "harbour",
            flags: 6,
            types: &["harbour_types"],
        }]),
    )
    .unwrap();
    let said = problems();
    assert!(
        said.contains(MANIFEST) && said.contains("not the document"),
        "{said}"
    );

    std::fs::write(&at, b"no manifest").unwrap();
    let said = problems();
    assert!(
        said.contains(MANIFEST) && said.contains("does not read"),
        "{said}"
    );

    std::fs::remove_file(&at).unwrap();
    let said = problems();
    assert!(
        said.contains(MANIFEST) && said.contains("not in the resource"),
        "{said}"
    );
}

#[test]
fn a_built_resource_whose_manifest_lost_the_line_that_declares_a_map_fails_the_check() {
    let dir = tempfile::tempdir().unwrap();
    let folder = module(dir.path());
    let container = container(OutputKind::FivemResource, &["maps"]);
    let built = build_with(
        &plan(&container, dir.path()),
        &dir.path().join("build"),
        &Default::default(),
        Some(&cooker),
    )
    .expect("it builds");
    let model = Maps::read(&folder).unwrap();
    let facts = inspect(&folder, &model, Depth::Open);
    assert!(verify(&model, &facts, &built.folder).is_clean());

    let at = built.folder.join("fxmanifest.lua");
    let text = std::fs::read_to_string(&at).unwrap();
    let without: Vec<&str> = (text.lines())
        .filter(|line| !line.starts_with("this_is_a_map"))
        .collect();
    assert_eq!(without.len() + 1, text.lines().count(), "{text}");
    std::fs::write(&at, without.join("\n")).unwrap();
    let said = verify(&model, &facts, &built.folder).problems().join("\n");
    assert!(said.contains("this_is_a_map 'yes'"), "{said}");

    // ⚠ The line is read as the statement a server reads, never looked for as text: one in a comment declares nothing, and the
    // quotes Lua takes either of do not matter. `project check-resource` reads the same statement.
    let stated = |line: &str| {
        std::fs::write(&at, text.replace("this_is_a_map 'yes'", line)).unwrap();
        let declared =
            or_project::resource::manifest::Manifest::read(&std::fs::read_to_string(&at).unwrap())
                .is_map();
        (verify(&model, &facts, &built.folder).is_clean(), declared)
    };
    assert_eq!(stated("-- this_is_a_map 'yes'"), (false, false));
    assert_eq!(stated("this_is_a_map \"yes\""), (true, true));
    assert_eq!(stated("this_is_a_map 'yes'"), (true, true));
}

#[test]
fn a_map_taken_from_the_game_keeps_the_entry_the_game_states_where_the_derived_one_differs() {
    let dir = tempfile::tempdir().unwrap();
    let folder = module_named(dir.path(), "maps", "harbour", "harbour_types", true);
    keep(&folder, "harbour", 3, &["t_game_a", "t_game_b"]);
    let model = Maps::read(&folder).unwrap();
    let facts = inspect(&folder, &model, Depth::Open);
    let stated = manifest::of(&model, &facts);
    let written = stated.file.expect("a manifest is written");
    assert_eq!(
        stated_in(&written).unwrap(),
        BTreeMap::from([(
            hash("harbour"),
            BTreeSet::from([hash("t_game_a"), hash("t_game_b")])
        )]),
        "what the game states, and not the type file the derivation finds"
    );
    // The type file the map places from this content is said, since the kept entry does not name it.
    assert!(
        stated.notes.join("\n").contains("`harbour_types`"),
        "{:?}",
        stated.notes
    );
}

#[test]
fn the_line_that_declares_a_map_is_written_once_when_two_contents_ask_for_it() {
    let dir = tempfile::tempdir().unwrap();
    module_named(dir.path(), "east", "harbour", "harbour_types", false);
    module_named(dir.path(), "west", "airfield", "airfield_types", false);
    let container = container(OutputKind::FivemResource, &["east", "west"]);
    let asked = handed(&container, dir.path());
    let asks = |c: &Contribution| c.manifest.iter().any(|line| line.key == "this_is_a_map");
    assert!(
        asked.len() == 2 && asked.iter().all(asks),
        "both contents ask"
    );

    let planned = plan(&container, dir.path());
    assert_eq!(planned.problems, Vec::<String>::new());
    let lines: Vec<&str> = document(&planned, "fxmanifest.lua")
        .lines()
        .filter(|line| line.starts_with("this_is_a_map"))
        .collect();
    assert_eq!(lines, ["this_is_a_map 'yes'"]);
}

#[test]
fn a_content_that_ships_type_files_and_no_map_declares_no_map() {
    let dir = tempfile::tempdir().unwrap();
    let declared = types_file(
        "harbour_types",
        &[Archetype::new(CRATE_A, [-1.0; 3], [1.0; 3])],
    );
    let added = add_files(
        &dir.path().join("maps"),
        &[Incoming {
            name: "harbour_types.ytyp",
            bytes: &declared,
            from_game: false,
            virtual_path: None,
        }],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());

    let resource = plan(&container(OutputKind::FivemResource, &["maps"]), dir.path());
    assert_eq!(resource.problems, Vec::<String>::new());
    let text = document(&resource, "fxmanifest.lua");
    assert!(text.contains("DLC_ITYP_REQUEST"), "{text}");
    assert!(!text.contains("this_is_a_map"), "{text}");

    // Nor does a pack say that the archive of its type files holds map data.
    let pack = plan(
        &container(OutputKind::SingleplayerDlc, &["maps"]),
        dir.path(),
    );
    let stated: Vec<(&str, &str)> = (pack.registrations.iter())
        .map(|r| (r.path.as_str(), r.contents.as_str()))
        .collect();
    assert_eq!(stated, [(ASKED, "CONTENTS_PROPS"), (PROPS, "")]);
}

#[test]
fn two_map_contents_of_one_build_that_each_write_a_manifest_are_refused_by_its_path() {
    let dir = tempfile::tempdir().unwrap();
    let east = module_named(dir.path(), "east", "harbour", "harbour_types", false);
    let west = module_named(dir.path(), "west", "airfield", "airfield_types", false);
    keep(&east, "harbour", 0, &[]);
    keep(&west, "airfield", 0, &[]);
    let planned = plan(
        &container(OutputKind::FivemResource, &["east", "west"]),
        dir.path(),
    );
    let [problem] = planned.problems.as_slice() else {
        panic!("one problem: {:?}", planned.problems)
    };
    assert!(
        problem.contains(MANIFEST) && problem.contains("east") && problem.contains("west"),
        "{problem}"
    );
}

/// Add to the sample module at `folder` the model its type file names and a texture dictionary nothing names.
fn with_a_model_and_textures(folder: &Path) {
    let added = add_files(
        folder,
        &[
            Incoming {
                name: "prop_crate_a_maps.ydd",
                bytes: &drawable_dictionary("prop_crate_a_maps", 6, 0.0),
                from_game: false,
                virtual_path: None,
            },
            Incoming {
                name: "harbour_signs.ytd",
                bytes: &texture_dictionary("harbour_signs", 90),
                from_game: false,
                virtual_path: None,
            },
        ],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
}

/// The bytes of the file at `path` of `archive`.
fn bytes_of(archive: &RpfArchive, path: &str) -> Vec<u8> {
    let node = (archive.nodes.iter())
        .find(|node| {
            !node.is_dir() && node.path().replace('\\', "/").trim_start_matches('/') == path
        })
        .unwrap_or_else(|| panic!("`{path}` is not in the archive"));
    archive
        .read_entry(node.entry_index())
        .expect("the entry reads")
}

/// The names of the files `archive` holds, in order.
fn held_by(archive: &RpfArchive) -> Vec<String> {
    let mut names: Vec<String> = (archive.nodes.iter())
        .filter(|node| !node.is_dir())
        .map(|node| {
            let path = node.path().replace('\\', "/");
            path.trim_start_matches('/').to_string()
        })
        .collect();
    names.sort();
    names
}

/// The plan of the story pack of the sample module with its model and textures, and the one sentence that locks it.
fn story_pack(dir: &Path) -> (Plan, String) {
    let folder = module(dir);
    with_a_model_and_textures(&folder);
    keep(&folder, "harbour", 5, &[]);
    let container = container(OutputKind::SingleplayerDlc, &["maps"]);
    let mut planned = plan(&container, dir);
    // ⚠ The plan is refused for the change set that turns the maps on, and for nothing else: everything below is built all the same.
    let [lock] = planned.problems.as_slice() else {
        panic!("one sentence locks the pack: {:?}", planned.problems)
    };
    let lock = lock.clone();
    planned.problems.clear();
    (planned, lock)
}

#[test]
fn a_story_pack_keeps_its_maps_and_its_type_files_in_the_archives_the_packs_of_the_game_keep_theirs_in(
) {
    let dir = tempfile::tempdir().unwrap();
    let (planned, lock) = story_pack(dir.path());
    let paths: Vec<&str> = planned.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "content.xml",
            "setup2.xml",
            &format!("{ARCHIVE}/_manifest.ymf"),
            &format!("{ARCHIVE}/harbour.ymap"),
            &format!("{PROPS}/harbour_types.ytyp"),
            &format!("{PROPS}/prop_crate_a_maps.ydd"),
            "x64/levels/gta5/props/maps.rpf/harbour_signs.ytd",
        ],
        "the maps and their manifest in the archive of maps, the type file with the model it names, and what nothing names in the content's own"
    );
    assert!(
        lock.contains(ARCHIVE) && lock.contains("when the pack starts"),
        "{lock}"
    );

    let built = build_with(
        &planned,
        &dir.path().join("build"),
        &Default::default(),
        Some(&cooker),
    )
    .expect("the pack is written and read back");
    assert_eq!(built.verification.problems, Vec::<String>::new());

    let pack = RpfArchive::open(built.folder.join("dlc.rpf")).expect("the pack opens");
    let nested = |path: &str| RpfArchive::from_bytes(bytes_of(&pack, path)).expect("it opens");
    let maps = nested(ARCHIVE);
    assert_eq!(held_by(&maps), ["_manifest.ymf", "harbour.ymap"]);
    let map = CMapData::from_bytes(&bytes_of(&maps, "harbour.ymap")).expect("the map reads");
    assert_eq!(map.entities.len(), 1);
    assert_eq!(
        stated_in(&bytes_of(&maps, "_manifest.ymf")).unwrap(),
        BTreeMap::from([(hash("harbour"), BTreeSet::from([hash("harbour_types")]))])
    );
    let props = nested(PROPS);
    assert_eq!(
        held_by(&props),
        ["harbour_types.ytyp", "prop_crate_a_maps.ydd"]
    );
    let types =
        CMapTypes::from_bytes(&bytes_of(&props, "harbour_types.ytyp")).expect("the types read");
    assert_eq!(types.archetypes.len(), 1);
    assert!(
        (pack.nodes.iter()).all(|node| !node.path().ends_with(".ymap")),
        "no map file sits loose in the pack"
    );

    let content = ContentXml::parse(&bytes_of(&pack, "content.xml")).expect("content.xml parses");
    let registered: Vec<(&str, &str, &str, bool, bool)> = (content.data_files.iter())
        .map(|entry| {
            let path = entry.filename.split_once(":/").map_or("", |(_, path)| path);
            (
                path,
                entry.file_type.as_str(),
                entry.contents.as_str(),
                entry.disabled,
                entry.persistent,
            )
        })
        .collect();
    assert_eq!(
        registered,
        [
            (ARCHIVE, "RPF_FILE", "CONTENTS_DLC_MAP_DATA", true, true),
            (ASKED, "DLC_ITYP_REQUEST", "CONTENTS_PROPS", true, false),
            (PROPS, "RPF_FILE", "", true, true),
            ("x64/levels/gta5/props/maps.rpf", "RPF_FILE", "", true, true),
        ],
        "the archive of maps as map data, the type file asked for as props by the name of its folder, and its archive stating nothing"
    );
}

/// A scratch install whose one pack is the `dlc.rpf` at `pack`, called `name`.
fn install_of(pack: &Path, name: &str) -> (tempfile::TempDir, GtaInstall) {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().to_path_buf();
    let list = DlcList {
        paths: vec![format!("dlcpacks:/{name}/").into()],
    };
    let mut common = RpfBuilder::default();
    common
        .insert_with("data/dlclist.xml", list.to_xml().into_bytes(), false)
        .unwrap();
    std::fs::write(root.join("common.rpf"), common.to_bytes().unwrap()).unwrap();
    let folder = root.join("x64/dlcpacks").join(name);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::copy(pack, folder.join("dlc.rpf")).unwrap();
    (scratch, GtaInstall { root })
}

/// ⚠ The count the install suite runs over the packs of the game (`how_the_packs_of_an_install_carry_their_maps`), run over the
/// pack a build writes: it keeps the pattern they keep in everything but the change set that turns its maps on, which is why the
/// cell stays locked.
#[test]
fn a_story_pack_reads_back_as_the_packs_of_an_install_read_but_for_what_turns_its_maps_on() {
    let dir = tempfile::tempdir().unwrap();
    let (planned, _) = story_pack(dir.path());
    let built = build_with(
        &planned,
        &dir.path().join("build"),
        &Default::default(),
        Some(&cooker),
    )
    .expect("the pack is written");
    let (_scratch, install) = install_of(&built.folder.join("dlc.rpf"), "harbour");
    let game = build_mounted_game(
        &install,
        Arc::new(NoOpCryptoProvider::new()),
        DlcCutoff::All,
    )
    .expect("the scratch install mounts");
    let f = measure(&game);

    assert_eq!((f.packs, f.packs_with_maps), (1, 1));
    // The archive of maps: in the folder of the level, registered as the packs of the game register theirs, with its manifest and nothing else.
    assert_eq!((f.map_archives, f.registered, f.under_the_level), (1, 1, 1));
    assert_eq!(f.depth, BTreeMap::from([(0, 1)]));
    assert_eq!(
        (f.as_archive, f.as_map_data, f.disabled, f.persistent),
        (1, 1, 1, 1)
    );
    assert_eq!(
        (f.with_manifest, f.with_types, f.with_models, f.with_bounds),
        (1, 0, 0, 0)
    );
    assert_eq!((f.manifests, f.manifests_atop_an_archive), (1, 1));
    // The type file: asked for as props from an archive stated to hold nothing, alone in it with its model, under the props folder.
    assert_eq!((f.type_files, f.type_files_with_maps, f.asked), (1, 0, 1));
    assert_eq!(
        (
            f.asked_as_props,
            f.asked_archive_plain,
            f.asked_archive_with_models,
            f.asked_alone,
            f.asked_under_props
        ),
        (1, 1, 1, 1, 1)
    );
    assert_eq!((f.asked_interiors, f.asked_unread), (0, 0));
    assert_eq!(
        f.asked_flags,
        BTreeMap::from([(
            "disabled true, persistent false, overlay false".to_string(),
            1
        )]),
        "as most requests of the game's packs are"
    );
    assert_eq!(
        f.asked_archive_flags,
        BTreeMap::from([(
            "disabled true, persistent true, overlay false".to_string(),
            1
        )]),
        "as the archive of a request is registered in the game's packs, all but a few of which are no overlay"
    );
    let at_start = BTreeMap::from([("GROUP_STARTUP, in the change set's own list".to_string(), 1)]);
    assert_eq!(
        f.asked_turned_on, at_start,
        "as the game's packs turn a request on"
    );
    assert_eq!(f.asked_archive_turned_on, at_start);
    // ⚠ The one way it leaves the pattern, and what the cell is locked for.
    assert_eq!(f.turned_on, at_start);
    assert_eq!(departures(&f), [MAPS_ON_AT_START]);
}

#[test]
fn the_cell_of_a_singleplayer_dlc_says_yes_to_type_files_and_models_and_no_to_a_map_for_what_turns_it_on(
) {
    let dir = tempfile::tempdir().unwrap();
    let folder = module(dir.path());
    with_a_model_and_textures(&folder);
    let dlc = OutputKind::SingleplayerDlc;

    // Handed over as a resource has them, no archive is written: the cell names the map that would sit loose.
    let loose = handed(&container(OutputKind::FivemResource, &["maps"]), dir.path());
    assert_eq!(cannot_ship(&loose[0], &OutputKind::FivemResource), None);
    let said = cannot_ship(&loose[0], &dlc).expect("a loose map locks the cell");
    assert!(
        said.contains("stream/harbour.ymap") && said.contains("inside an archive"),
        "{said}"
    );
    assert!(
        (or_project::files_without_a_type(&loose[0]).iter()).all(|path| !path.ends_with(".ymap")),
        "a map file is given no type under File types: {:?}",
        or_project::files_without_a_type(&loose[0])
    );

    // Handed over for the pack, everything is laid out and registered as measured, and one thing is withheld: the change set.
    let mut packed = handed(&container(dlc.clone(), &["maps"]), dir.path()).remove(0);
    let said = cannot_ship(&packed, &dlc).expect("the map locks the cell");
    assert!(
        said.contains(ARCHIVE)
            && said.contains("when the pack starts")
            && said.contains("at start or for the story")
            && said.contains("seen to load in the game"),
        "{said}"
    );
    assert!(
        !said.contains("never measured"),
        "the folder and the type files are measured and built: {said}"
    );
    // A package installs the same pack, so its cell says the same of the same content.
    let oiv = OutputKind::OivPackage;
    assert_eq!(cannot_ship(&packed, &oiv).as_ref(), Some(&said));

    // Where the archive is, and what the pack states of it, are parts of that yes: each withheld is named in its place.
    let mut unstated = packed.clone();
    unstated.archives.retain(|said| said.path != ARCHIVE);
    let said = cannot_ship(&unstated, &dlc).expect("an archive of maps stated as nothing");
    assert!(said.contains("does not state to hold map data"), "{said}");
    let mut elsewhere = packed.clone();
    for file in &mut elsewhere.files {
        file.path = file
            .path
            .replace("x64/levels/gta5/maps.rpf", "x64/maps.rpf");
    }
    let said = cannot_ship(&elsewhere, &dlc).expect("an archive of maps outside the level");
    assert!(
        said.contains("x64/maps.rpf") && said.contains("under x64/levels/gta5"),
        "{said}"
    );

    // ⚠ The same content without its map: everything it hands over is measured and built, and the cell says yes.
    packed.files.retain(|file| !file.path.starts_with(ARCHIVE));
    packed.archives.retain(|said| said.path != ARCHIVE);
    assert!(
        (packed.files.iter()).any(|file| file.path == format!("{PROPS}/harbour_types.ytyp")),
        "the type file is still handed over"
    );
    assert_eq!(cannot_ship(&packed, &dlc), None);
    assert_eq!(cannot_ship(&packed, &oiv), None);
}

#[test]
fn a_content_of_type_files_and_models_plans_and_builds_as_a_singleplayer_dlc() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("maps");
    let declared = types_file(
        "harbour_types",
        &[Archetype::new("prop_crate_a_maps", [-1.0; 3], [1.0; 3])],
    );
    let added = add_files(
        &folder,
        &[Incoming {
            name: "harbour_types.ytyp",
            bytes: &declared,
            from_game: false,
            virtual_path: None,
        }],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    with_a_model_and_textures(&folder);

    let container = container(OutputKind::SingleplayerDlc, &["maps"]);
    let handed = handed(&container, dir.path());
    assert_eq!(cannot_ship(&handed[0], &OutputKind::SingleplayerDlc), None);
    let planned = plan(&container, dir.path());
    assert_eq!(
        planned.problems,
        Vec::<String>::new(),
        "nothing locks a pack of type files and models"
    );
    let built = build_with(
        &planned,
        &dir.path().join("build"),
        &Default::default(),
        Some(&cooker),
    )
    .expect("the pack is written and read back");
    assert_eq!(built.verification.problems, Vec::<String>::new());

    let (_scratch, install) = install_of(&built.folder.join("dlc.rpf"), "harbour");
    let game = build_mounted_game(
        &install,
        Arc::new(NoOpCryptoProvider::new()),
        DlcCutoff::All,
    )
    .expect("the scratch install mounts");
    let f = measure(&game);
    assert_eq!((f.map_archives, f.type_files, f.asked), (0, 1, 1));
    assert_eq!(
        departures(&f),
        Vec::<&str>::new(),
        "it keeps the pattern of the game's packs whole"
    );
}

/// The classes of a type file whose one archetype is an interior, with the members a plain archetype states and no room.
const INTERIOR_PSC: &str = r#"<?xml version="1.0"?>
<ParserSchema>
  <structdef type="CMloArchetypeDef">
    <float name="lodDist"/>
    <u32 name="flags"/>
    <u32 name="specialAttribute"/>
    <Vector3 name="bbMin"/>
    <Vector3 name="bbMax"/>
    <Vector3 name="bsCentre"/>
    <float name="bsRadius"/>
    <float name="hdTextureDist"/>
    <string name="name" type="atHashString"/>
    <string name="textureDictionary" type="atHashString"/>
    <string name="clipDictionary" type="atHashString"/>
    <string name="drawableDictionary" type="atHashString"/>
    <string name="physicsDictionary" type="atHashString"/>
    <string name="assetName" type="atHashString"/>
    <u32 name="mloFlags"/>
  </structdef>
  <structdef type="CMapTypes">
    <string name="name" type="atHashString"/>
    <array name="archetypes" type="atArray"><pointer type="CMloArchetypeDef" policy="owner"/></array>
    <array name="dependencies" type="atArray"><string type="atHashString"/></array>
  </structdef>
</ParserSchema>"#;

/// A `.ytyp` called `name` that declares the one interior `interior`, made as the crate's sample files are.
fn interior_types_file(name: &str, interior: &str) -> Vec<u8> {
    types_file_of("CMloArchetypeDef", name, interior, "", "")
}

/// A `.ytyp` called `name` whose one archetype, of the class `class`, is `archetype` and names the texture dictionary `textures`
/// and the dictionary of drawables `drawables`, each none when empty.
fn types_file_of(
    class: &str,
    name: &str,
    archetype: &str,
    textures: &str,
    drawables: &str,
) -> Vec<u8> {
    let interior = class == "CMloArchetypeDef";
    let named = |text: &str| match text {
        "" => Value::Hash(HashedString::from_hash(0)),
        text => Value::Hash(HashedString::new(text)),
    };
    let mut declared = MetaInstance::new(class);
    for member in ["lodDist", "bsRadius", "hdTextureDist"] {
        declared.set(member, Value::Float(50.0));
    }
    for member in ["flags", "specialAttribute"] {
        declared.set(member, Value::UInt(0));
    }
    declared.set("bbMin", Value::Vec3([-4.0; 3]));
    declared.set("bbMax", Value::Vec3([4.0; 3]));
    declared.set("bsCentre", Value::Vec3([0.0; 3]));
    declared.set("name", Value::Hash(HashedString::new(archetype)));
    declared.set("textureDictionary", named(textures));
    declared.set("clipDictionary", named(""));
    declared.set("drawableDictionary", named(drawables));
    declared.set("physicsDictionary", named(""));
    // An interior is drawn by what it holds and names no model of its own.
    declared.set("assetName", named(if interior { "" } else { archetype }));
    let mut schema = INTERIOR_PSC.replace("CMloArchetypeDef", class);
    if interior {
        declared.set("mloFlags", Value::UInt(0));
    } else {
        schema = schema.replace("    <u32 name=\"mloFlags\"/>\n", "");
    }
    let mut root = MetaInstance::new("CMapTypes");
    root.set("name", Value::Hash(HashedString::new(name)));
    root.set(
        "archetypes",
        Value::Array(vec![Value::Pointer(Some(Box::new(declared)))]),
    );
    root.set("dependencies", Value::Array(Vec::new()));
    let library = build_library([("inline.psc", schema.as_bytes())]).expect("the schema");
    let layout = canonical::layout_for(&library, &root, BinaryFormat::Rbm, None).expect("a layout");
    write_meta_binary(BinaryFormat::Rbm, Some(&layout), &root).expect("the file encodes")
}

/// ⚠ An archetype names its model three ways and its textures one: by its asset, by a dictionary of drawables, and by a texture
/// dictionary. Each goes into the archive of the type file that names it, whichever type file comes first, and what none names
/// into the content's own.
#[test]
fn a_model_and_a_texture_dictionary_go_into_the_archive_of_the_type_file_whose_archetype_names_them(
) {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("maps");
    let first = types_file(
        "first_types",
        &[Archetype::new("prop_first", [-1.0; 3], [1.0; 3])],
    );
    let second = types_file_of(
        "CBaseArchetypeDef",
        "second_types",
        "prop_second",
        "second_textures",
        "second_models",
    );
    let incoming = |name: &'static str, bytes: Vec<u8>| (name, bytes);
    let files = [
        incoming("first_types.ytyp", first),
        incoming("second_types.ytyp", second),
        incoming("prop_first.ydd", drawable_dictionary("prop_first", 6, 0.0)),
        incoming(
            "second_models.ydd",
            drawable_dictionary("second_models", 6, 1.0),
        ),
        incoming(
            "second_textures.ytd",
            texture_dictionary("second_textures", 90),
        ),
        incoming(
            "nobodys_textures.ytd",
            texture_dictionary("nobodys_textures", 30),
        ),
    ];
    let incoming: Vec<Incoming> = (files.iter())
        .map(|(name, bytes)| Incoming {
            name,
            bytes,
            from_game: false,
            virtual_path: None,
        })
        .collect();
    let added = add_files(&folder, &incoming, None).unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());

    let dlc = OutputKind::SingleplayerDlc;
    let handed = handed(&container(dlc.clone(), &["maps"]), dir.path()).remove(0);
    assert_eq!(cannot_ship(&handed, &dlc), None);
    let mut paths: Vec<&str> = handed.files.iter().map(|f| f.path.as_str()).collect();
    paths.sort();
    let props = or_maps::layout::props_folder();
    let expected = [
        "first_types.rpf/first_types.ytyp",
        "first_types.rpf/prop_first.ydd",
        "maps.rpf/nobodys_textures.ytd",
        "second_types.rpf/second_models.ydd",
        "second_types.rpf/second_textures.ytd",
        "second_types.rpf/second_types.ytyp",
    ]
    .map(|path| format!("{props}/{path}"));
    assert_eq!(paths, expected);
}

/// The classes of a map whose entities are interiors, with the members a plain entity states.
const INTERIOR_MAP_PSC: &str = r#"<?xml version="1.0"?>
<ParserSchema>
  <structdef type="CMloInstanceDef">
    <string name="archetypeName" type="atHashString"/>
    <u32 name="flags"/>
    <u32 name="guid"/>
    <Vector3 name="position"/>
    <Vector4 name="rotation"/>
    <float name="scaleXY"/>
    <float name="scaleZ"/>
    <s32 name="parentIndex"/>
    <float name="lodDist"/>
    <float name="childLodDist"/>
    <u32 name="numChildren"/>
  </structdef>
  <structdef type="CMapData">
    <string name="name" type="atHashString"/>
    <string name="parent" type="atHashString"/>
    <u32 name="flags"/>
    <u32 name="contentFlags"/>
    <Vector3 name="streamingExtentsMin"/>
    <Vector3 name="streamingExtentsMax"/>
    <Vector3 name="entitiesExtentsMin"/>
    <Vector3 name="entitiesExtentsMax"/>
    <array name="entities" type="atArray"><pointer type="CMloInstanceDef" policy="owner"/></array>
  </structdef>
</ParserSchema>"#;

/// A `.ymap` called `name` that places each of `interiors` as an interior and nothing else, made as the crate's sample files are.
fn interior_map_file(name: &str, interiors: &[&str]) -> Vec<u8> {
    let placed = (interiors.iter().zip(1u64..))
        .map(|(interior, guid)| {
            let mut inst = MetaInstance::new("CMloInstanceDef");
            inst.set("archetypeName", Value::Hash(HashedString::new(*interior)));
            inst.set("flags", Value::UInt(0));
            inst.set("guid", Value::UInt(guid));
            inst.set("position", Value::Vec3([10.0, 20.0, 0.5]));
            inst.set("rotation", Value::Vec4([0.0, 0.0, 0.0, 1.0]));
            for member in ["scaleXY", "scaleZ"] {
                inst.set(member, Value::Float(1.0));
            }
            inst.set("parentIndex", Value::Int(-1));
            inst.set("lodDist", Value::Float(100.0));
            inst.set("childLodDist", Value::Float(0.0));
            inst.set("numChildren", Value::UInt(0));
            Value::Pointer(Some(Box::new(inst)))
        })
        .collect();
    let mut root = MetaInstance::new("CMapData");
    root.set("name", Value::Hash(HashedString::new(name)));
    root.set("parent", Value::Hash(HashedString::from_hash(0)));
    for member in ["flags", "contentFlags"] {
        root.set(member, Value::UInt(0));
    }
    for (member, corner) in [
        ("streamingExtentsMin", [8.0, 18.0, -1.0]),
        ("streamingExtentsMax", [12.0, 22.0, 2.0]),
        ("entitiesExtentsMin", [8.0, 18.0, -1.0]),
        ("entitiesExtentsMax", [12.0, 22.0, 2.0]),
    ] {
        root.set(member, Value::Vec3(corner));
    }
    root.set("entities", Value::Array(placed));
    let library = build_library([("inline.psc", INTERIOR_MAP_PSC.as_bytes())]).expect("the schema");
    let layout = canonical::layout_for(&library, &root, BinaryFormat::Rbm, None).expect("a layout");
    write_meta_binary(BinaryFormat::Rbm, Some(&layout), &root).expect("the file encodes")
}

/// The flags of each entry for a map the manifest in `bytes` holds.
fn flags_of_the_entries(bytes: &[u8]) -> Vec<Option<u32>> {
    let (root, _) = or_io::meta::binary::parse_meta_binary_full(
        &or_io::meta::SchemaLibrary::default(),
        bytes,
        &mut or_io::meta::Diagnostics::lenient(),
    )
    .expect("the manifest reads");
    or_world_gta5::field::structs_at(&root, or_world_gta5::keys::IMAP_DEPENDENCIES_2)
        .map(|entry| {
            or_world_gta5::field::u32_at(entry, or_world_gta5::manifest_build::MANIFEST_FLAGS)
        })
        .collect()
}

/// ⚠ The count of interiors a map places reaches the writer from the plan: without it every derived entry would state no flag,
/// which is what a manifest of the game states of a map of props and never of an interior's.
#[test]
fn a_map_no_kept_manifest_states_that_places_interiors_alone_takes_the_flag_of_an_interiors_map() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("maps");
    let added = add_files(
        &folder,
        &[
            Incoming {
                name: "shop_types.ytyp",
                bytes: &interior_types_file("shop_types", "v_shop"),
                from_game: false,
                virtual_path: None,
            },
            Incoming {
                name: "shop_placed.ymap",
                bytes: &interior_map_file("shop_placed", &["v_shop"]),
                from_game: false,
                virtual_path: None,
            },
        ],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    // The entry that gives the shape states no flag, so the flag written is derived and not copied.
    keep(&folder, "another_map", 0, &["t_of_another_map"]);

    let model = Maps::read(&folder).unwrap();
    let facts = inspect(&folder, &model, Depth::Open);
    let placed = (model.maps.iter())
        .flat_map(|set| set.sources.iter())
        .find_map(|path| facts.source(path)?.map())
        .expect("the map is read");
    assert_eq!(
        placed
            .entities
            .iter()
            .map(|e| e.interior)
            .collect::<Vec<_>>(),
        [true],
        "the one entity is read as an interior"
    );
    let stated = manifest::of(&model, &facts);
    let written = stated
        .file
        .expect("a manifest is written for the map made here");
    assert_eq!(
        stated_in(&written).unwrap(),
        BTreeMap::from([(hash("shop_placed"), BTreeSet::from([hash("shop_types")]))])
    );
    assert_eq!(
        flags_of_the_entries(&written),
        [Some(or_world_gta5::manifest_build::INTERIOR_DATA)]
    );
}

#[test]
fn a_type_file_that_declares_an_interior_is_asked_for_by_nothing_and_refused_for_a_pack_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("maps");
    let added = add_files(
        &folder,
        &[Incoming {
            name: "shop_types.ytyp",
            bytes: &interior_types_file("shop_types", "v_shop"),
            from_game: false,
            virtual_path: None,
        }],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());

    // A FiveM resource registers it as it registers every type file.
    let resource = plan(&container(OutputKind::FivemResource, &["maps"]), dir.path());
    assert_eq!(resource.problems, Vec::<String>::new());
    assert!(document(&resource, "fxmanifest.lua").contains("shop_types.ytyp"));

    let pack = plan(
        &container(OutputKind::SingleplayerDlc, &["maps"]),
        dir.path(),
    );
    let [problem] = pack.problems.as_slice() else {
        panic!("one sentence refuses it: {:?}", pack.problems)
    };
    assert!(
        problem.contains("shop_types.ytyp")
            && problem.contains("declares an interior")
            && problem.contains("a build does not write yet"),
        "{problem}"
    );
    assert!(
        pack.registrations.iter().all(|r| !r.asked),
        "no pack of the game asks for such a type file: {:?}",
        pack.registrations
    );

    // ⚠ The cell says what the plan says: a content of that type file alone is locked for a story pack, by the same sentence.
    let dlc = OutputKind::SingleplayerDlc;
    let handed = handed(&container(dlc.clone(), &["maps"]), dir.path());
    assert_eq!(
        cannot_ship(&handed[0], &dlc).as_deref(),
        Some(problem.as_str())
    );
    assert_eq!(cannot_ship(&handed[0], &OutputKind::FivemResource), None);
}

#[test]
fn an_oiv_of_a_maps_content_installs_the_pack_a_singleplayer_dlc_is_and_is_locked_as_one() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("maps");
    let declared = types_file(
        "harbour_types",
        &[Archetype::new("prop_crate_a_maps", [-1.0; 3], [1.0; 3])],
    );
    let added = add_files(
        &folder,
        &[Incoming {
            name: "harbour_types.ytyp",
            bytes: &declared,
            from_game: false,
            virtual_path: None,
        }],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());

    // A content of a type file alone: the package installs it inside the pack it adds, registered as the pack registers it.
    let oiv = OutputKind::OivPackage;
    let package = container(oiv.clone(), &["maps"]);
    let handed_over = handed(&package, dir.path());
    assert_eq!(cannot_ship(&handed_over[0], &oiv), None);
    let planned = plan(&package, dir.path());
    assert_eq!(planned.problems, Vec::<String>::new());
    let pack = "update/x64/dlcpacks/harbour/dlc.rpf";
    let paths: Vec<&str> = planned.files.iter().map(|f| f.path.as_str()).collect();
    assert!(
        paths.contains(&format!("{pack}/{PROPS}/harbour_types.ytyp").as_str())
            && paths.contains(&format!("{pack}/content.xml").as_str()),
        "nothing is installed at the stream folder of a resource, which the game does not read: {paths:?}"
    );
    let story = plan(
        &container(OutputKind::SingleplayerDlc, &["maps"]),
        dir.path(),
    );
    assert_eq!(
        planned.registrations, story.registrations,
        "the pack a package installs registers what the pack built alone registers"
    );
    assert!(
        (planned.registrations.iter()).any(|r| r.path == ASKED && r.contents == "CONTENTS_PROPS"),
        "{:?}",
        planned.registrations
    );

    // ⚠ With a map the package is locked by the sentence that locks the pack: nothing is written where the game reads nothing.
    let placed = map_file(
        "harbour",
        &[Entity::at("prop_crate_a_maps", [10.0, 20.0, 0.5], 1)],
        ([8.0, 18.0, -1.0], [12.0, 22.0, 2.0]),
    );
    let added = add_files(
        &folder,
        &[Incoming {
            name: "harbour.ymap",
            bytes: &placed,
            from_game: false,
            virtual_path: None,
        }],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    let handed_over = handed(&package, dir.path());
    let dlc = OutputKind::SingleplayerDlc;
    let said = cannot_ship(&handed_over[0], &oiv).expect("the map locks the cell of a package");
    assert_eq!(
        Some(&said),
        cannot_ship(
            &handed(&container(dlc.clone(), &["maps"]), dir.path())[0],
            &dlc
        )
        .as_ref()
    );
    assert!(
        said.contains(ARCHIVE) && said.contains("when the pack starts"),
        "{said}"
    );
    let planned = plan(&package, dir.path());
    assert!(planned.problems.contains(&said), "{:?}", planned.problems);
    assert!(
        planned.files.iter().all(|f| !f.path.starts_with("stream/")),
        "{:?}",
        planned.files.iter().map(|f| &f.path).collect::<Vec<_>>()
    );
    assert!(
        (planned.registrations.iter())
            .any(|r| r.path == ARCHIVE && r.contents == "CONTENTS_DLC_MAP_DATA"),
        "{:?}",
        planned.registrations
    );
}

#[test]
fn a_map_is_stated_in_the_manifest_by_the_name_of_its_file_whatever_name_it_states() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("maps");
    // `yard.ymap` states the name of a map of the game, which a kept manifest states with the type file the game gives it.
    let placed = map_file(
        "harbour",
        &[Entity::at(CRATE_A, [10.0, 20.0, 0.5], 1)],
        ([8.0, 18.0, -1.0], [12.0, 22.0, 2.0]),
    );
    let declared = types_file(
        "yard_types",
        &[Archetype::new(CRATE_A, [-1.0; 3], [1.0; 3])],
    );
    let files = [("yard.ymap", &placed), ("yard_types.ytyp", &declared)];
    let incoming: Vec<Incoming> = (files.iter())
        .map(|(name, bytes)| Incoming {
            name,
            bytes,
            from_game: false,
            virtual_path: None,
        })
        .collect();
    let added = add_files(&folder, &incoming, None).unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    keep(&folder, "harbour", 0, &["v_harbour_game"]);

    let planned = plan(&container(OutputKind::FivemResource, &["maps"]), dir.path());
    assert!(
        planned.files.iter().any(|f| f.path == "stream/yard.ymap"),
        "the build ships the file by its own name"
    );
    let model = Maps::read(&folder).unwrap();
    let stated = manifest::of(&model, &inspect(&folder, &model, Depth::Open));
    let written = stated.file.expect("a manifest is written");
    assert_eq!(
        stated_in(&written).expect("the manifest reads back"),
        BTreeMap::from([(hash("yard"), BTreeSet::from([hash("yard_types")]))]),
        "the map the build ships, and nothing for the map of the game whose name it states"
    );
}

#[test]
fn a_map_a_kept_manifest_states_as_an_interiors_map_is_named_where_it_places_what_is_no_interior() {
    let dir = tempfile::tempdir().unwrap();
    let folder = module(dir.path());
    // The game states `harbour` with the bit of an interior's map, and the map of the content places one crate.
    keep(
        &folder,
        "harbour",
        4 | or_world_gta5::manifest_build::INTERIOR_DATA,
        &[],
    );
    let model = Maps::read(&folder).unwrap();
    let stated = manifest::of(&model, &inspect(&folder, &model, Depth::Open));
    let written = stated.file.expect("a manifest is written");
    assert_eq!(
        flags_of_the_entries(&written),
        [Some(4 | or_world_gta5::manifest_build::INTERIOR_DATA)],
        "the flags the game states are kept"
    );
    let [note] = stated.notes.as_slice() else {
        panic!("one note names the map: {:?}", stated.notes)
    };
    assert!(
        note.contains("`harbour`") && note.contains("keeps the flags the game states"),
        "{note}"
    );
}
