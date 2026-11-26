//! A module of map sources goes through the one plan and the one build: a map file imports to a metadata source and builds back to a
//! file that reads equal, each source cooks to `stream/<name>.<extension>`, a type file is registered by the line that tells the game to
//! read it, and the verifier reads the built resource back; a pack that breaks a rule is refused before anything is written, and the cell
//! of a singleplayer DLC is derived from what the content hands over.
//!
//! ⚠ The files are made here, from the sample module of the crate: the clean checkout runs this, and the install-gated suites hold the
//! source form and the rules to the files of a mounted game.

use std::path::{Path, PathBuf};

use or_io::decl::File;
use or_io::meta::typed::CMapData;
use or_maps::import::{add_files, Incoming};
use or_maps::model::Maps;
use or_maps::sample::{map_file, types_file, Archetype, Entity};
use or_maps::verify::verify;
use or_maps::{contributions, inspect, Depth};
use or_project::{build_with, Container, Module, Origin, Output, OutputKind, Plan};

const CRATE_A: &str = "prop_crate_a";
const CRATE_B: &str = "prop_crate_b";

/// A map file placing two crates, whose extents hold them.
pub fn harbour_map() -> Vec<u8> {
    map_file(
        "harbour",
        &[
            Entity::at(CRATE_A, [10.0, 20.0, 0.5], 1),
            Entity {
                yaw: 0.6,
                ..Entity::at(CRATE_B, [14.0, 20.0, 0.5], 2)
            },
        ],
        ([8.0, 18.0, -1.0], [16.0, 22.0, 2.0]),
    )
}

pub fn harbour_types() -> Vec<u8> {
    types_file(
        "harbour_types",
        &[
            Archetype::new(CRATE_A, [-1.0; 3], [1.0; 3]),
            Archetype::new(CRATE_B, [-1.0; 3], [1.0; 3]),
        ],
    )
}

/// The module folder `dir/maps` holding the map file and the type file as sources of one set.
fn module(dir: &Path) -> PathBuf {
    let folder = dir.join("maps");
    let (map, types) = (harbour_map(), harbour_types());
    let added = add_files(
        &folder,
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
    assert_eq!(added.set_made.as_deref(), Some("harbour"));
    folder
}

fn container(kind: OutputKind) -> Container {
    let mut output = Output::fivem_resource("fivem", "harbour", &["maps"]);
    output.kind = kind;
    Container {
        modules: vec![Module::maps("maps", "maps")],
        outputs: vec![output],
    }
}

fn plan(container: &Container, folder: &Path) -> Plan {
    let contributions = or_maps::contributions(container, folder);
    or_project::plan_with(container, "fivem", folder, &contributions).expect("the output exists")
}

fn cooker(source: &Path, scratch: &Path) -> Result<PathBuf, Vec<String>> {
    or_source::cook::cook_source(source, scratch)
}

#[test]
fn a_ymap_imports_to_a_source_and_builds_back_to_a_file_that_reads_equal() {
    let dir = tempfile::tempdir().unwrap();
    let folder = module(dir.path());
    let container = container(OutputKind::FivemResource);
    let planned = plan(&container, dir.path());
    assert_eq!(planned.problems, Vec::<String>::new());

    let paths: Vec<&str> = planned.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "fxmanifest.lua",
            "stream/harbour.ymap",
            "stream/harbour_types.ytyp"
        ],
        "each cooked file at the folder FiveM streams from, and the manifest, by path"
    );
    assert!(planned
        .files
        .iter()
        .filter(|f| f.path.starts_with("stream/"))
        .all(|f| matches!(f.origin, Origin::Cook { .. })));
    assert_eq!(planned.items.len(), 1, "one item for the one set");

    let root = dir.path().join("build");
    let built = build_with(&planned, &root, &Default::default(), Some(&cooker))
        .expect("the build cooks, writes and reads back");
    assert_eq!(built.verification.problems, Vec::<String>::new());
    let reloaded = built
        .verification
        .reloaded
        .iter()
        .find(|r| r.path == "stream/harbour.ymap")
        .expect("the map file was reopened through its reader");
    assert!(
        reloaded.reader.starts_with("CMapData"),
        "{}",
        reloaded.reader
    );

    // The file the build wrote reads as the file the source was made from.
    let original = CMapData::from_bytes(&harbour_map()).unwrap();
    let written =
        CMapData::from_bytes(&std::fs::read(built.folder.join("stream/harbour.ymap")).unwrap())
            .unwrap();
    assert_eq!(written.entities.len(), original.entities.len());
    for (a, b) in written.entities.iter().zip(&original.entities) {
        assert_eq!(
            (a.guid, a.position, a.rotation),
            (b.guid, b.position, b.rotation)
        );
        assert_eq!(a.archetype_name.hash, b.archetype_name.hash);
    }

    let manifest = std::fs::read_to_string(built.folder.join("fxmanifest.lua")).unwrap();
    assert!(
        manifest.contains("data_file 'DLC_ITYP_REQUEST' 'stream/harbour_types.ytyp'"),
        "the type file is registered: {manifest}"
    );

    let model = Maps::read(&folder).unwrap();
    let facts = inspect(&folder, &model, Depth::Open);
    let found = verify(&model, &facts, &built.folder);
    assert!(found.is_clean(), "{:?}", found.problems());
    let checked: Vec<&str> = found.checks.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(checked, ["map harbour", "map resource"]);

    // A second build of the same sources is the same build.
    let again = build_with(
        &plan(&container, dir.path()),
        &root,
        &Default::default(),
        Some(&cooker),
    )
    .expect("the second build");
    for file in ["stream/harbour.ymap", "stream/harbour_types.ytyp"] {
        assert_eq!(
            std::fs::read(built.folder.join(file)).unwrap(),
            std::fs::read(again.folder.join(file)).unwrap(),
            "{file}"
        );
    }
}

#[test]
fn a_tampered_resource_fails_the_verifier_by_the_file_it_names() {
    let dir = tempfile::tempdir().unwrap();
    let folder = module(dir.path());
    let planned = plan(&container(OutputKind::FivemResource), dir.path());
    let built = build_with(
        &planned,
        &dir.path().join("build"),
        &Default::default(),
        Some(&cooker),
    )
    .unwrap();
    let model = Maps::read(&folder).unwrap();
    let facts = inspect(&folder, &model, Depth::Open);

    std::fs::write(built.folder.join("stream/harbour.ymap"), b"not a map").unwrap();
    let found = verify(&model, &facts, &built.folder);
    assert!(!found.is_clean());
    assert!(
        found.problems().join("\n").contains("stream/harbour.ymap"),
        "{:?}",
        found.problems()
    );

    std::fs::write(
        built.folder.join("fxmanifest.lua"),
        "fx_version 'cerulean'\n",
    )
    .unwrap();
    let found = verify(&model, &facts, &built.folder);
    assert!(
        found
            .problems()
            .join("\n")
            .contains("data_file 'DLC_ITYP_REQUEST'"),
        "{:?}",
        found.problems()
    );
}

#[test]
fn a_pack_that_breaks_a_rule_is_refused_by_the_plan_and_names_the_source() {
    let dir = tempfile::tempdir().unwrap();
    let folder = module(dir.path());
    let mut model = Maps::read(&folder).unwrap();
    model.maps[0].sources.push("gone.metadata.src".into());
    model.write(&folder).unwrap();

    let planned = plan(&container(OutputKind::FivemResource), dir.path());
    let said = planned.problems.join("\n");
    assert!(
        said.contains("source-missing") && said.contains("gone.metadata.src"),
        "{said}"
    );
    assert!(!or_maps::rule_of("source-missing").unwrap().waivable);
}

#[test]
fn a_singleplayer_dlc_cell_is_derived_from_what_the_content_hands_over() {
    let dir = tempfile::tempdir().unwrap();
    module(dir.path());
    let container = container(OutputKind::FivemResource);
    let handed = contributions(&container, dir.path());
    assert_eq!(handed.len(), 1);
    assert_eq!(
        or_project::cannot_ship(&handed[0], &OutputKind::FivemResource),
        None
    );
    // A `.ymap` streams from `stream/`, and no data file of the retail packs registers one: the install keeps its map data inside
    // nested archives, which `content.xml` registers, so a map that would sit loose locks the cell.
    let said = or_project::cannot_ship(&handed[0], &OutputKind::SingleplayerDlc)
        .expect("a map file outside every archive locks the cell");
    assert!(said.contains("stream/harbour.ymap"), "{said}");
}

/// A module holding only the type file, as one set.
fn types_only(dir: &Path) -> PathBuf {
    let folder = dir.join("maps");
    let types = harbour_types();
    let added = add_files(
        &folder,
        &[Incoming {
            name: "harbour_types.ytyp",
            bytes: &types,
            from_game: false,
            virtual_path: None,
        }],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    folder
}

/// The install registers a type file by an `.ityp` name from a nested archive (684 of 685, `where_the_install_keeps_its_map_files`), so a
/// type file that would sit loose in the pack is a placement nobody measured, with a map file or without one.
#[test]
fn a_pack_of_type_files_only_is_locked_for_a_singleplayer_dlc_too() {
    let dir = tempfile::tempdir().unwrap();
    types_only(dir.path());
    let handed = contributions(&container(OutputKind::FivemResource), dir.path());
    assert_eq!(
        or_project::cannot_ship(&handed[0], &OutputKind::FivemResource),
        None,
        "a FiveM resource streams a type file from `stream/`"
    );
    let said = or_project::cannot_ship(&handed[0], &OutputKind::SingleplayerDlc)
        .expect("a type file that would sit loose locks the cell");
    assert!(said.contains("stream/harbour_types.ytyp"), "{said}");

    let planned = plan(&container(OutputKind::SingleplayerDlc), dir.path());
    assert!(
        planned
            .problems
            .iter()
            .any(|p| p.contains("stream/harbour_types.ytyp")),
        "the plan of a singleplayer DLC says it too: {:?}",
        planned.problems
    );
}
