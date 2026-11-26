//! What a build of maps adds for a server and for a story install: the manifest of its maps, written from a manifest of the game the
//! module keeps, the line that declares a FiveM resource a map, and, for a singleplayer DLC, the nested archive the pack registers.
//!
//! ⚠ Every file is made here from the sample module of the crate, the kept manifest included: no install is read and no game content
//! is in the repository. What the game's own manifests hold is the install suite's to say.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use or_io::container::rpf::RpfArchive;
use or_io::decl::File;
use or_io::meta::typed::{CMapData, CMapTypes};
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
use or_title::dlc::ContentXml;
use or_world::manifest_build::stated_in;

const CRATE_A: &str = "prop_crate_a";
const MANIFEST: &str = "stream/_manifest.ymf";
const ARCHIVE: &str = "x64/levels/gta5/maps.rpf";

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
    keep(&folder, "harbour", 5, &["t_stated_by_the_donor"]);
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
fn a_map_no_kept_manifest_states_is_named_and_left_out_of_the_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let folder = module(dir.path());
    keep(&folder, "another_map", 0, &[]);
    let planned = plan(&container(OutputKind::FivemResource, &["maps"]), dir.path());
    assert!(planned.files.iter().all(|f| f.path != MANIFEST));
    assert!(
        planned.notes.join("\n").contains("`harbour`"),
        "{:?}",
        planned.notes
    );
}

#[test]
fn a_map_no_kept_manifest_states_is_named_beside_the_maps_the_manifest_is_written_for() {
    let dir = tempfile::tempdir().unwrap();
    let folder = module(dir.path());
    let extents = ([8.0, 18.0, -1.0], [12.0, 22.0, 2.0]);
    let second = map_file(
        "airfield",
        &[Entity::at("prop_crate_a_maps", [10.0, 20.0, 0.5], 1)],
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
    keep(&folder, "harbour", 5, &[]);

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
        note.contains("`airfield`") && note.contains("states nothing") && !note.contains("harbour"),
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
    assert_eq!(
        stated,
        [("x64/levels/gta5/harbour_types.ityp", ""), (ARCHIVE, "")]
    );
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

#[test]
fn a_story_pack_holds_its_maps_in_the_nested_archive_its_content_document_registers() {
    let dir = tempfile::tempdir().unwrap();
    let folder = module(dir.path());
    keep(&folder, "harbour", 5, &[]);
    let container = container(OutputKind::SingleplayerDlc, &["maps"]);
    let mut planned = plan(&container, dir.path());
    let paths: Vec<&str> = planned.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "content.xml",
            "setup2.xml",
            &format!("{ARCHIVE}/_manifest.ymf"),
            &format!("{ARCHIVE}/harbour.ymap"),
            &format!("{ARCHIVE}/harbour_types.ytyp"),
        ],
        "every file of the content goes through the one archive"
    );

    // ⚠ The plan is refused for what nobody measured, and for nothing else: the packing below is built and the cell stays locked.
    let [lock] = planned.problems.as_slice() else {
        panic!("one sentence locks the pack: {:?}", planned.problems)
    };
    assert!(
        lock.contains(ARCHIVE) && lock.contains("never measured"),
        "{lock}"
    );
    planned.problems.clear();

    let built = build_with(
        &planned,
        &dir.path().join("build"),
        &Default::default(),
        Some(&cooker),
    )
    .expect("the pack is written and read back");
    assert_eq!(built.verification.problems, Vec::<String>::new());

    let pack = RpfArchive::open(built.folder.join("dlc.rpf")).expect("the pack opens");
    let bytes_of = |archive: &RpfArchive, path: &str| {
        let node = (archive.nodes.iter())
            .find(|node| {
                !node.is_dir() && node.path().replace('\\', "/").trim_start_matches('/') == path
            })
            .unwrap_or_else(|| panic!("`{path}` is not in the archive"));
        archive
            .read_entry(node.entry_index())
            .expect("the entry reads")
    };
    let nested =
        RpfArchive::from_bytes(bytes_of(&pack, ARCHIVE)).expect("the nested archive opens");
    let map = CMapData::from_bytes(&bytes_of(&nested, "harbour.ymap")).expect("the map reads");
    assert_eq!(map.entities.len(), 1);
    let types =
        CMapTypes::from_bytes(&bytes_of(&nested, "harbour_types.ytyp")).expect("the types read");
    assert_eq!(types.archetypes.len(), 1);
    assert_eq!(
        stated_in(&bytes_of(&nested, "_manifest.ymf")).unwrap(),
        BTreeMap::from([(hash("harbour"), BTreeSet::from([hash("harbour_types")]))])
    );
    assert!(
        pack.nodes
            .iter()
            .all(|node| !node.path().ends_with(".ymap")),
        "no map file sits loose in the pack"
    );

    let content = ContentXml::parse(&bytes_of(&pack, "content.xml")).expect("content.xml parses");
    let registered: Vec<(&str, &str, &str)> = (content.data_files.iter())
        .map(|entry| {
            let path = entry.filename.split_once(":/").map_or("", |(_, path)| path);
            (path, entry.file_type.as_str(), entry.contents.as_str())
        })
        .collect();
    assert_eq!(
        registered,
        [
            ("x64/levels/gta5/harbour_types.ityp", "DLC_ITYP_REQUEST", ""),
            (ARCHIVE, "RPF_FILE", "CONTENTS_DLC_MAP_DATA"),
        ],
        "the archive as map data, and the type file by the name the game asks it by"
    );
    let enabled: Vec<&str> = (content.content_change_sets.iter())
        .flat_map(|set| set.files_to_enable.iter())
        .map(|item| item.value.as_str())
        .collect();
    assert_eq!(enabled.len(), 2, "{enabled:?}");
}

#[test]
fn the_cell_of_a_singleplayer_dlc_stays_locked_for_maps_and_says_what_is_unmeasured() {
    let dir = tempfile::tempdir().unwrap();
    module(dir.path());
    let dlc = OutputKind::SingleplayerDlc;

    // Handed over as a resource has them, the archive is not written: the cell names the map that would sit loose.
    let loose = handed(&container(OutputKind::FivemResource, &["maps"]), dir.path());
    assert_eq!(cannot_ship(&loose[0], &OutputKind::FivemResource), None);
    let said = cannot_ship(&loose[0], &dlc).expect("a loose map locks the cell");
    assert!(
        said.contains("stream/harbour.ymap")
            && said.contains("inside an archive")
            && said.contains("never measured"),
        "{said}"
    );
    assert!(
        or_project::files_without_a_type(&loose[0]).is_empty(),
        "a map file is given no type under File types: {:?}",
        or_project::files_without_a_type(&loose[0])
    );

    // Handed over for the pack, the archive is written, and the cell is locked for what nobody measured of it.
    let packed = handed(&container(dlc.clone(), &["maps"]), dir.path());
    let said = cannot_ship(&packed[0], &dlc).expect("the unmeasured archive locks the cell");
    assert!(
        said.contains(ARCHIVE) && said.contains("never measured"),
        "{said}"
    );
}
