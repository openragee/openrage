//! A mapping that comes in as a zip is reported as the folder of the same files is, and adopted to the same project byte for byte;
//! the report says of each file what it becomes before anything is written, of each line of the manifest whether a build carries it,
//! and what the map rules say of the set; a locked file is kept whole and no reader is handed it; a place with no resource and no
//! stream folder adopts nothing; and an adoption that cannot finish leaves nothing.
//!
//! Every file is the crate's sample or bytes written here, and every archive is written here with the crate that reads it.

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use or_maps::mapping::{adopt, analyse, check, Adopted, Becomes, Error, Report, Request, RootKind};
use or_maps::model::Maps;
use or_maps::sample::{map_file, types_file, Archetype, Entity};
use or_project::place::{Limits, Place, Refusal};
use or_project::resource::ESCROW_MARKER;
use or_project::{ModuleKind, Project};
use or_source::sample::{drawable_dictionary, texture_dictionary};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const CRATE: &str = "prop_crate_a";

type Files = Vec<(String, Vec<u8>)>;

fn zip_of(files: &Files) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, bytes) in files {
        zip.start_file(name.as_str(), options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// `files` as a folder called `mapping` and as `mapping.zip`, both in `dir`.
fn both(dir: &Path, files: &Files) -> (PathBuf, PathBuf) {
    let folder = dir.join("mapping");
    for (name, bytes) in files {
        let path = folder.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let archive = dir.join("mapping.zip");
    std::fs::write(&archive, zip_of(files)).unwrap();
    (folder, archive)
}

fn open(at: &Path) -> Place {
    Place::open(at, &Limits::default()).unwrap()
}

/// Every file under `base`, by its path with `/`, with its bytes.
fn tree(base: &Path) -> Files {
    let mut found = Vec::new();
    let mut stack = vec![base.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path.strip_prefix(base).unwrap().to_string_lossy();
                found.push((rel.replace('\\', "/"), std::fs::read(&path).unwrap()));
            }
        }
    }
    found.sort();
    found
}

/// What follows the marker of a locked file here: bytes no reader of a map file takes.
const SEALED: &[u8] = b"what the escrow wrote, which is no metadata document";

fn locked() -> Vec<u8> {
    [ESCROW_MARKER.as_slice(), SEALED].concat()
}

const MANIFEST: &str = "fx_version 'cerulean'\n\
game 'gta5'\n\
this_is_a_map 'yes'\n\
author 'Ann'\n\
data_file 'DLC_ITYP_REQUEST' 'stream/harbour_types.ytyp'\n\
data_file 'HANDLING_FILE' 'data/handling.meta'\n\
client_script 'client.lua'\n";

fn the_map() -> Vec<u8> {
    map_file(
        "harbour",
        &[
            Entity::at(CRATE, [1.0, 1.0, 0.0], 1),
            Entity::at(CRATE, [5.0, 1.0, 0.0], 2),
        ],
        ([-1.0, -1.0, -1.0], [7.0, 3.0, 1.0]),
    )
}

fn the_types() -> Vec<u8> {
    types_file(
        "harbour_types",
        &[Archetype::new(CRATE, [-1.0; 3], [1.0; 3])],
    )
}

/// A resource called `harbour` in a folder of its own, with a file beside it.
fn a_sound_mapping() -> Files {
    [
        ("harbour/.git/config", b"[core]".to_vec()),
        ("harbour/.git/objects/1a/2b3c", b"an object".to_vec()),
        ("harbour/fxmanifest.lua", MANIFEST.as_bytes().to_vec()),
        ("harbour/client.lua", b"print('harbour')".to_vec()),
        (
            "harbour/data/handling.meta",
            b"<CHandlingDataMgr/>".to_vec(),
        ),
        ("harbour/more.zip", b"PK, as far as a name says".to_vec()),
        ("harbour/stream/harbour.ymap", the_map()),
        ("harbour/stream/harbour_types.ytyp", the_types()),
        (
            "harbour/stream/hull.ybn",
            b"a bound no maps content holds".to_vec(),
        ),
        (
            "harbour/stream/prop_crate_a.ydd",
            drawable_dictionary(CRATE, 6, 0.0),
        ),
        (
            "harbour/stream/prop_crate_a.ytd",
            texture_dictionary(CRATE, 90),
        ),
        ("harbour/stream/sealed.ydr", locked()),
        ("readme.txt", b"thanks for downloading".to_vec()),
    ]
    .into_iter()
    .map(|(name, bytes)| (name.to_string(), bytes))
    .collect()
}

/// What each file of `report`'s only root becomes, as `path: word`.
fn becomes(report: &Report) -> Vec<String> {
    (report.roots[0].files.iter())
        .map(|file| match &file.becomes {
            Becomes::Source { kind } => format!("{}: source {kind}", file.path),
            other => format!("{}: {}", file.path, other.word()),
        })
        .collect()
}

fn request<'a>(into: &'a Path) -> Request<'a> {
    Request {
        into,
        name: "harbour",
        resource: "harbour",
        root: None,
    }
}

#[test]
fn a_sound_mapping_in_a_zip_is_reported_and_adopted_as_the_folder_of_the_same_files_is() {
    let dir = tempfile::tempdir().unwrap();
    let files = a_sound_mapping();
    let (folder, archive) = both(dir.path(), &files);
    let before = tree(dir.path());

    let (mut from_folder, mut from_zip) = (open(&folder), open(&archive));
    let (of_folder, of_zip) = (analyse(&mut from_folder), analyse(&mut from_zip));
    let inside = adopt(
        &mut from_folder,
        &of_folder,
        &request(&folder.join("project")),
    );
    assert!(matches!(inside, Err(Error::Inside(_))), "{inside:?}");
    assert_eq!(tree(dir.path()), before, "a report writes nothing");

    assert!(of_zip.archive && !of_folder.archive);
    assert_eq!(
        (of_zip.from.as_str(), of_folder.from.as_str()),
        ("mapping.zip", "mapping")
    );
    assert_eq!(of_zip.roots, of_folder.roots);
    assert_eq!(of_zip.outside, of_folder.outside);
    assert_eq!(of_zip.notes, of_folder.notes);

    let root = &of_zip.roots[0];
    assert_eq!(of_zip.roots.len(), 1);
    assert_eq!(
        (root.path.as_str(), &root.kind),
        ("harbour", &RootKind::Resource)
    );
    assert_eq!(root.manifest.as_deref(), Some("fxmanifest.lua"));
    assert_eq!(
        becomes(&of_zip),
        [
            "client.lua: kept whole",
            "data/handling.meta: kept whole",
            "fxmanifest.lua: manifest",
            "more.zip: left out",
            "stream/harbour.ymap: source metadata",
            "stream/harbour_types.ytyp: source metadata",
            "stream/hull.ybn: kept whole",
            "stream/prop_crate_a.ydd: source drawables",
            "stream/prop_crate_a.ytd: source textures",
            "stream/sealed.ydr: locked",
        ]
    );
    assert_eq!(root.locked, 1);
    let carried: Vec<(&str, bool)> = root
        .lines
        .iter()
        .map(|l| (l.key.as_str(), l.carried))
        .collect();
    assert_eq!(
        carried,
        [
            ("fx_version", true),
            ("game", true),
            ("this_is_a_map", true),
            ("author", true),
            ("data_file", true),
            ("data_file", true),
            ("client_script", false),
        ]
    );
    let outside: Vec<&str> = of_zip.outside.iter().map(|o| o.path.as_str()).collect();
    assert_eq!(
        outside,
        ["harbour/.git", "readme.txt"],
        "a hidden folder is said once, whatever it holds"
    );
    let codes: Vec<&str> = root.findings.iter().map(|f| f.code).collect();
    assert_eq!(
        codes,
        Vec::<&str>::new(),
        "a sound set breaks no rule: {:#?}",
        root.findings
    );

    let (a, b) = (dir.path().join("from_zip"), dir.path().join("from_folder"));
    let adopted_zip = adopt(&mut from_zip, &of_zip, &request(&a)).unwrap();
    let adopted_folder = adopt(&mut from_folder, &of_folder, &request(&b)).unwrap();
    assert_eq!(adopted_zip.root, adopted_folder.root);
    assert_eq!(
        adopted_zip.root, of_zip.roots[0],
        "what the report said is what was made"
    );

    // What two adoptions do not share are the ids OpenRage gives: one to the map set and one to each source, random and stable
    // from then on. Each is read from the file that states it and taken out of that file, and every other byte has to be equal.
    let id_of = |project: &Path| {
        let model = Maps::read(&project.join("maps")).unwrap();
        assert_eq!(model.maps.len(), 1);
        assert_eq!(model.maps[0].name, "harbour");
        model.maps[0].id.clone()
    };
    let (id_a, id_b) = (id_of(&a), id_of(&b));
    assert_ne!(id_a, id_b);
    let without_ids = |project: &Path, set: &str| -> Files {
        tree(project)
            .into_iter()
            .map(|(name, bytes)| {
                let id = match name.as_str() {
                    "maps/maps.json" => set.to_string(),
                    name if name.ends_with(".src/asset.json") => {
                        let stated: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                        stated["id"].as_str().unwrap().to_string()
                    }
                    _ => return (name, bytes),
                };
                let text = String::from_utf8(bytes).unwrap();
                assert_eq!(text.matches(&id).count(), 1, "{name}: {text}");
                (name, text.replace(&id, "<id>").into_bytes())
            })
            .collect()
    };
    let (written_a, written_b) = (without_ids(&a, &id_a), without_ids(&b, &id_b));
    let names = |written: &Files| {
        written
            .iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&written_a), names(&written_b));
    for ((name, from_zip), (_, from_folder)) in written_a.iter().zip(&written_b) {
        assert!(
            from_zip == from_folder,
            "{name} differs between the two adoptions"
        );
    }

    what_was_made(&a, &adopted_zip, &files);
    let beside: Files = tree(dir.path())
        .into_iter()
        .filter(|(name, _)| !name.starts_with("from_zip/") && !name.starts_with("from_folder/"))
        .collect();
    assert_eq!(
        beside, before,
        "the folder and the archive are as they were"
    );
}

/// The project in `into` is what the report said: sources in a maps content, the rest kept whole, the lines carried where they go.
fn what_was_made(into: &Path, adopted: &Adopted, files: &Files) {
    let written: Vec<String> = tree(into).into_iter().map(|(name, _)| name).collect();
    assert!(
        !written.iter().any(|name| name.starts_with(".adopting")),
        "the staging folder is gone: {written:#?}"
    );
    let original = |name: &str| {
        let (_, bytes) = files
            .iter()
            .find(|(held, _)| held == &format!("harbour/{name}"))
            .unwrap();
        bytes.clone()
    };
    for kept in [
        "client.lua",
        "data/handling.meta",
        "stream/hull.ybn",
        "stream/sealed.ydr",
        "fxmanifest.lua",
    ] {
        assert_eq!(
            std::fs::read(into.join(kept)).unwrap(),
            original(kept),
            "{kept} is kept whole"
        );
    }
    for gone in [
        ".git",
        "more.zip",
        "readme.txt",
        "stream/harbour.ymap",
        "stream/prop_crate_a.ydd",
    ] {
        assert!(
            !into.join(gone).exists(),
            "{gone} is no loose file of the project"
        );
    }
    let sources: Vec<String> = (adopted.sources.iter())
        .map(|dir| {
            dir.strip_prefix(into)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    assert_eq!(
        sources,
        [
            "maps/harbour.metadata.src",
            "maps/harbour_types.metadata.src",
            "maps/prop_crate_a.drawables.src",
            "maps/prop_crate_a.textures.src"
        ]
    );
    assert!(adopted
        .sources
        .iter()
        .all(|dir| dir.join("asset.json").is_file()));

    let project = Project::load(&adopted.project).unwrap();
    let kinds: Vec<&ModuleKind> = project.container.modules.iter().map(|m| &m.kind).collect();
    assert_eq!(kinds, [&ModuleKind::Maps, &ModuleKind::Files]);
    let output = &project.container.outputs[0];
    assert_eq!(
        (output.name.as_str(), output.take.as_slice()),
        (
            "harbour",
            ["maps".to_string(), "files".to_string()].as_slice()
        )
    );
    assert_eq!(output.extra.get("author"), Some(&serde_json::json!("Ann")));
    let files_content = &project.container.modules[1];
    assert_eq!(
        files_content.file_types().into_iter().collect::<Vec<_>>(),
        [(
            "data/handling.meta".to_string(),
            "HANDLING_FILE".to_string()
        )]
    );
    let mut documents: Vec<String> = (project.members.iter())
        .map(|member| {
            member
                .path
                .strip_prefix(into)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    documents.sort();
    assert_eq!(
        documents,
        [
            "client.lua",
            "data/handling.meta",
            "stream/hull.ybn",
            "stream/sealed.ydr"
        ],
        "the manifest is no document: a build writes its own"
    );

    let text = std::fs::read_to_string(&adopted.project).unwrap();
    let file: serde_json::Value = serde_json::from_str(&text).unwrap();
    let record = &file[or_project::adopt::ADOPTED_KEY];
    let unplaced: Vec<&str> = (record["unplaced"].as_array().unwrap().iter())
        .map(|u| u["path"].as_str().unwrap())
        .collect();
    assert_eq!(unplaced, ["more.zip", "fxmanifest.lua"]);
    let notes = record["notes"].as_array().unwrap();
    assert!(
        notes
            .iter()
            .any(|n| n.as_str().unwrap().contains("client_script")),
        "{notes:?}"
    );
    assert!(
        notes
            .iter()
            .any(|n| n.as_str().unwrap().contains("1 file(s) are locked")),
        "{notes:?}"
    );
}

#[test]
fn a_locked_file_is_kept_whole_and_counted_and_no_reader_is_handed_it() {
    let dir = tempfile::tempdir().unwrap();
    // The same bytes under two names: with the marker in front they are locked, and without it a reader is asked and refuses them.
    let files: Files = vec![
        ("stream/harbour.ymap".into(), the_map()),
        ("stream/open.ymap".into(), SEALED.to_vec()),
        ("stream/sealed.ymap".into(), locked()),
        ("stream/sealed_types.ytyp".into(), locked()),
    ];
    let (_folder, archive) = both(dir.path(), &files);
    let mut place = open(&archive);
    let report = analyse(&mut place);
    let root = &report.roots[0];
    assert_eq!(root.kind, RootKind::StreamFolder);
    assert_eq!(
        becomes(&report),
        [
            "stream/harbour.ymap: source metadata",
            "stream/open.ymap: refused",
            "stream/sealed.ymap: locked",
            "stream/sealed_types.ytyp: locked",
        ]
    );
    assert_eq!(root.locked, 2);
    let refused = &root.files[1];
    assert!(
        refused.why.contains("does not read as a metadata document"),
        "{}",
        refused.why
    );
    let sealed = &root.files[2];
    assert!(
        sealed.why.contains("asset escrow") && !sealed.why.contains("metadata"),
        "{}",
        sealed.why
    );
    assert!(
        root.findings.iter().all(|f| !f.message.contains("sealed")),
        "no rule read a locked file: {:#?}",
        root.findings
    );
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("type file") && note.contains("locked")),
        "{:?}",
        report.notes
    );

    let into = dir.path().join("project");
    let adopted = adopt(&mut place, &report, &request(&into)).unwrap();
    assert_eq!(
        std::fs::read(into.join("stream/sealed.ymap")).unwrap(),
        locked()
    );
    assert_eq!(
        std::fs::read(into.join("stream/sealed_types.ytyp")).unwrap(),
        locked()
    );
    assert!(
        !into.join("stream/open.ymap").exists(),
        "a refused file is not adopted"
    );
    let made: Vec<String> = (adopted.sources.iter())
        .map(|dir| dir.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        made,
        ["harbour.metadata.src"],
        "no source is made of a locked file"
    );
    let text = std::fs::read_to_string(&adopted.project).unwrap();
    let file: serde_json::Value = serde_json::from_str(&text).unwrap();
    let unplaced = &file[or_project::adopt::ADOPTED_KEY]["unplaced"];
    assert_eq!(unplaced.as_array().map(Vec::len), Some(1), "{unplaced}");
    assert_eq!(unplaced[0]["path"], "stream/open.ymap");
    assert_eq!(
        unplaced[0]["why"],
        refused.why.as_str(),
        "the project records why it was refused"
    );
    assert_eq!(adopted.root, report.roots[0]);
}

#[test]
fn the_map_rules_say_what_a_set_built_to_break_them_lacks_before_it_is_adopted() {
    let dir = tempfile::tempdir().unwrap();
    let files: Files = vec![
        (
            "pack/__resource.lua".into(),
            b"resource_manifest_version '44febabe'\n".to_vec(),
        ),
        (
            "pack/stream/harbour.ymap".into(),
            map_file(
                "harbour",
                &[
                    Entity::at(CRATE, [1.0, 1.0, 0.0], 1),
                    Entity::at("prop_ghost", [5.0, 1.0, 0.0], 2),
                ],
                ([-1.0, -1.0, -1.0], [7.0, 3.0, 1.0]),
            ),
        ),
        ("pack/stream/harbour_types.ytyp".into(), the_types()),
    ];
    let (folder, archive) = both(dir.path(), &files);
    for at in [&folder, &archive] {
        let report = analyse(&mut open(at));
        let root = &report.roots[0];
        assert_eq!(root.manifest.as_deref(), Some("__resource.lua"));
        let found: Vec<(&str, Option<&str>)> = (root.findings.iter())
            .map(|f| (f.code, f.place.source.as_deref()))
            .collect();
        assert_eq!(
            found,
            [
                ("archetype-undeclared", Some("stream/harbour.ymap")),
                ("archetype-without-model", Some("stream/harbour_types.ytyp")),
            ],
            "{:#?}",
            root.findings
        );
        let undeclared = &root.findings[0];
        assert_eq!(undeclared.place.entity, Some(1));
        assert!(
            undeclared.message.contains("no type file"),
            "{}",
            undeclared.message
        );
        let lines = report.lines();
        for finding in &root.findings {
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains(finding.code) && line.contains(&finding.message)),
                "{} is in the text",
                finding.code
            );
        }
    }
    assert!(!dir.path().join("project").exists());
}

#[test]
fn a_place_with_no_resource_and_no_stream_folder_says_what_it_found_and_adopts_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let files: Files = vec![
        ("harbour.ymap".into(), the_map()),
        (
            "props/prop_crate_a.ydd".into(),
            drawable_dictionary(CRATE, 6, 0.0),
        ),
        ("readme.txt".into(), b"drop these in a resource".to_vec()),
    ];
    let (folder, archive) = both(dir.path(), &files);
    for at in [&folder, &archive] {
        let mut place = open(at);
        let report = analyse(&mut place);
        assert!(!report.adoptable());
        assert_eq!(report.roots, []);
        let outside: Vec<(&str, &str)> = (report.outside.iter())
            .map(|o| (o.path.as_str(), o.why.as_str()))
            .collect();
        assert_eq!(outside.len(), 3);
        assert_eq!(outside[0].0, "harbour.ymap");
        assert!(outside[0].1.starts_with("a map file"), "{}", outside[0].1);
        assert!(
            outside[1].1.contains("becomes a drawables source"),
            "{}",
            outside[1].1
        );
        assert_eq!(
            report.notes,
            ["no resource and no stream folder, so there is nothing to adopt: it holds 1 .txt, 1 .ydd, 1 .ymap"]
        );

        let into = dir.path().join("project");
        let refused = adopt(&mut place, &report, &request(&into)).unwrap_err();
        assert!(matches!(refused, Error::Nothing), "{refused}");
        assert!(!into.exists(), "nothing is made");
    }
}

#[test]
fn a_place_of_several_resources_is_adopted_one_at_a_time_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let files: Files = vec![
        (
            "[maps]/east/fxmanifest.lua".into(),
            b"fx_version 'cerulean'\nclient_script 'c.lua'\n".to_vec(),
        ),
        ("[maps]/east/c.lua".into(), b"print('east')".to_vec()),
        ("[maps]/west/stream/harbour.ymap".into(), the_map()),
    ];
    let (_folder, archive) = both(dir.path(), &files);
    let mut place = open(&archive);
    let report = analyse(&mut place);
    let roots: Vec<(&str, &RootKind)> = report
        .roots
        .iter()
        .map(|r| (r.path.as_str(), &r.kind))
        .collect();
    assert_eq!(
        roots,
        [
            ("[maps]/east", &RootKind::Resource),
            ("[maps]/west", &RootKind::StreamFolder)
        ]
    );

    let into = dir.path().join("project");
    let refused = adopt(&mut place, &report, &request(&into)).unwrap_err();
    assert!(
        matches!(&refused, Error::Several(named) if named == &["[maps]/east", "[maps]/west"]),
        "{refused}"
    );
    assert!(!into.exists());
    let missing = Request {
        root: Some("[maps]/north"),
        ..request(&into)
    };
    assert!(matches!(
        adopt(&mut place, &report, &missing),
        Err(Error::NoSuchRoot(_))
    ));

    // A resource that makes no maps content keeps its own manifest, with every line.
    let east = Request {
        root: Some("[maps]/east"),
        ..request(&into)
    };
    assert!(
        report.roots[0].lines.iter().all(|line| line.carried),
        "{:#?}",
        report.roots[0].lines
    );
    let adopted = adopt(&mut place, &report, &east).unwrap();
    let project = Project::load(&adopted.project).unwrap();
    let kinds: Vec<&ModuleKind> = project.container.modules.iter().map(|m| &m.kind).collect();
    assert_eq!(kinds, [&ModuleKind::Files]);
    let names: Vec<String> = tree(&into).into_iter().map(|(name, _)| name).collect();
    assert_eq!(names, ["c.lua", "fxmanifest.lua", "harbour.openrage-proj"]);
    assert_eq!(
        project.members.len(),
        2,
        "the manifest is a document of the project"
    );
}

#[test]
fn an_adoption_that_cannot_finish_leaves_the_folder_as_it_found_it() {
    let dir = tempfile::tempdir().unwrap();
    const TRUE_SIZE: u32 = 70_001;
    const STATED: u32 = 12;
    let files: Files = vec![
        ("stream/a.ybn".into(), b"kept whole".to_vec()),
        ("stream/harbour.ymap".into(), the_map()),
        ("stream/z.ybn".into(), vec![b'z'; TRUE_SIZE as usize]),
    ];
    let mut bytes = zip_of(&files);
    // The last entry states a size it is larger than, in its local header and in the central one.
    let (from, to) = (TRUE_SIZE.to_le_bytes(), STATED.to_le_bytes());
    let mut replaced = 0;
    for at in 0..bytes.len() - 4 {
        if bytes[at..at + 4] == from {
            bytes[at..at + 4].copy_from_slice(&to);
            replaced += 1;
        }
    }
    assert_eq!(replaced, 2);
    let archive = dir.path().join("mapping.zip");
    std::fs::write(&archive, &bytes).unwrap();

    let mut place = open(&archive);
    let report = analyse(&mut place);
    assert_eq!(
        report.roots[0].files[2].bytes,
        u64::from(STATED),
        "the report states what the archive states"
    );

    let lies = |refused: Error| match refused {
        Error::Refused(Refusal::SizeLies { entry, stated }) => (entry, stated),
        other => panic!("{other}"),
    };
    let new = dir.path().join("not/there/yet/new");
    let refused = adopt(&mut place, &report, &request(&new)).unwrap_err();
    assert_eq!(
        lies(refused),
        ("stream/z.ybn".to_string(), u64::from(STATED))
    );
    assert!(
        !dir.path().join("not").exists(),
        "every folder the adoption made is removed"
    );

    let empty = dir.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let refused = adopt(&mut place, &report, &request(&empty)).unwrap_err();
    assert_eq!(lies(refused).0, "stream/z.ybn");
    assert_eq!(
        std::fs::read_dir(&empty).unwrap().count(),
        0,
        "a folder that was empty is empty again"
    );

    let held = dir.path().join("held");
    std::fs::create_dir_all(&held).unwrap();
    std::fs::write(held.join("mine.txt"), b"mine").unwrap();
    let refused = adopt(&mut place, &report, &request(&held)).unwrap_err();
    assert!(matches!(refused, Error::Held(_)), "{refused}");
    assert_eq!(tree(&held), [("mine.txt".to_string(), b"mine".to_vec())]);
}

#[test]
fn a_file_no_source_is_made_of_at_the_adoption_is_kept_whole_with_the_reason() {
    let dir = tempfile::tempdir().unwrap();
    let files: Files = vec![
        ("stream/harbour.ymap".into(), the_map()),
        (
            "stream/prop_crate_a.ydd".into(),
            b"no drawable dictionary a reader opens".to_vec(),
        ),
    ];
    let (_folder, archive) = both(dir.path(), &files);
    let mut place = open(&archive);
    let report = analyse(&mut place);
    assert_eq!(
        becomes(&report),
        [
            "stream/harbour.ymap: source metadata",
            "stream/prop_crate_a.ydd: source drawables"
        ],
        "a model is judged by its name until it is adopted"
    );
    let into = dir.path().join("project");
    let adopted = adopt(&mut place, &report, &request(&into)).unwrap();
    let model = &adopted.root.files[1];
    assert_eq!(model.becomes, Becomes::Kept);
    assert!(
        model.why.starts_with("no source is made of it ("),
        "{}",
        model.why
    );
    assert_eq!(
        std::fs::read(into.join("stream/prop_crate_a.ydd")).unwrap(),
        files[1].1
    );
    assert_eq!(adopted.sources.len(), 1);
    assert_eq!(adopted.kept, [into.join("stream").join("prop_crate_a.ydd")]);
}

/// A mapping whose report has a line of every kind: files, the lines of a manifest and a run of it that is no statement, a
/// finding of the map rules, a file outside every root and a note.
fn a_mapping_with_something_of_everything() -> Files {
    let manifest = "fx_version 'cerulean'\n\
                    local built = os.date()\n\
                    if built then print(built) end\n\
                    this_is_a_map 'yes'\n\
                    client_script 'client.lua'\n";
    let map = map_file(
        "harbour",
        &[Entity::at("prop_ghost", [5.0, 1.0, 0.0], 2)],
        ([-1.0, -1.0, -1.0], [7.0, 3.0, 1.0]),
    );
    [
        ("harbour/fxmanifest.lua", manifest.as_bytes().to_vec()),
        ("harbour/client.lua", b"print('harbour')".to_vec()),
        ("harbour/more.zip", b"PK".to_vec()),
        ("harbour/.git/config", b"[core]".to_vec()),
        ("harbour/stream/harbour.ymap", map),
        ("harbour/stream/sealed_types.ytyp", locked()),
        ("readme.txt", b"thanks".to_vec()),
    ]
    .into_iter()
    .map(|(name, bytes)| (name.to_string(), bytes))
    .collect()
}

#[test]
fn the_document_of_a_report_states_what_its_lines_state() {
    let dir = tempfile::tempdir().unwrap();
    let (_folder, archive) = both(dir.path(), &a_mapping_with_something_of_everything());
    let report = analyse(&mut open(&archive));
    let document = report.document();
    assert_eq!(document["format"], "openrage-mapping-report");
    let lines = report.lines();

    // Each entry of the document is one line of the text that no other entry is, and no line is left over but the two that count.
    let mut matched = vec![false; lines.len()];
    (matched[0], matched[1]) = (true, true);
    let mut claim = |is: &dyn Fn(&str) -> bool, what: String| {
        let at: Vec<usize> = (0..lines.len())
            .filter(|at| !matched[*at] && is(lines[*at].trim_start()))
            .collect();
        assert_eq!(at.len(), 1, "{what} is one line of:\n{lines:#?}");
        matched[at[0]] = true;
    };
    let text = |value: &serde_json::Value| value.as_str().unwrap().to_string();
    let root = &document["roots"][0];
    let mut kinds = Vec::new();
    for file in root["files"].as_array().unwrap() {
        let word = match text(&file["becomes"]["as"]).as_str() {
            "source" => format!("source ({})", text(&file["becomes"]["kind"])),
            "kept" => "kept whole".to_string(),
            other => other.replace('-', " "),
        };
        let begins = format!("{word} {}", text(&file["path"]));
        let why = text(&file["why"]);
        claim(
            &|line| line.starts_with(&begins) && line.ends_with(&why),
            begins.clone(),
        );
        kinds.push("file");
    }
    for line in root["lines"].as_array().unwrap() {
        let carried = if line["carried"].as_bool().unwrap() {
            "carried"
        } else {
            "not carried"
        };
        let begins = format!("manifest line {} {} ", line["line"], text(&line["key"]));
        let ends = format!(": {carried}: {}", text(&line["how"]));
        claim(
            &|l| l.starts_with(&begins) && l.ends_with(&ends),
            begins.clone(),
        );
        kinds.push("line");
    }
    for run in root["unread"].as_array().unwrap() {
        let begins = format!("manifest lines {} to {}: ", run[0], run[1]);
        claim(&|l| l.starts_with(&begins), begins.clone());
        kinds.push("unread");
    }
    for finding in root["findings"].as_array().unwrap() {
        let ends = format!("[{}] {}", text(&finding["code"]), text(&finding["message"]));
        claim(&|l| l.ends_with(&ends), ends.clone());
        kinds.push("finding");
    }
    for outside in document["outside"].as_array().unwrap() {
        let whole = format!(
            "outside {}: {}",
            text(&outside["path"]),
            text(&outside["why"])
        );
        claim(&|l| l == whole, whole.clone());
        kinds.push("outside");
    }
    for note in document["notes"].as_array().unwrap() {
        let whole = format!("note: {}", text(note));
        claim(&|l| l == whole, whole.clone());
        kinds.push("note");
    }
    assert!(
        matched.iter().all(|m| *m),
        "a line of the text is no entry of the document: {lines:#?}"
    );
    // The mapping is built to have one of each, so no kind of line goes unchecked because the report held none.
    for kind in ["file", "line", "unread", "finding", "outside", "note"] {
        assert!(
            kinds.contains(&kind),
            "the report has no {kind}: {lines:#?}"
        );
    }
}

#[test]
fn two_files_that_would_be_one_source_are_told_apart_before_the_adoption() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = "fx_version 'cerulean'\n\
                    data_file 'DLC_ITYP_REQUEST' 'stream/harbour.ytyp'\n";
    // A map file and a type file of one name are one metadata source by name, and so are two dictionaries of one name in two
    // folders, in whichever case each is spelled.
    let files: Files = vec![
        ("fxmanifest.lua".into(), manifest.as_bytes().to_vec()),
        (
            "stream/a/prop_crate_a.ydd".into(),
            drawable_dictionary(CRATE, 6, 0.0),
        ),
        (
            "stream/b/Prop_Crate_A.ydd".into(),
            drawable_dictionary(CRATE, 6, 0.0),
        ),
        ("stream/harbour.ymap".into(), the_map()),
        (
            "stream/harbour.ytyp".into(),
            types_file("harbour", &[Archetype::new(CRATE, [-1.0; 3], [1.0; 3])]),
        ),
    ];
    let (folder, archive) = both(dir.path(), &files);
    for (at, into) in [(&folder, "from_folder"), (&archive, "from_zip")] {
        let mut place = open(at);
        let report = analyse(&mut place);
        assert_eq!(
            becomes(&report),
            [
                "fxmanifest.lua: manifest",
                "stream/a/prop_crate_a.ydd: source drawables",
                "stream/b/Prop_Crate_A.ydd: kept whole",
                "stream/harbour.ymap: source metadata",
                "stream/harbour.ytyp: kept whole",
            ]
        );
        let root = &report.roots[0];
        let (twin, types) = (&root.files[2], &root.files[4]);
        assert!(
            twin.why.contains("`stream/a/prop_crate_a.ydd`")
                && twin.why.contains("Prop_Crate_A.drawables.src"),
            "{}",
            twin.why
        );
        assert!(
            types.why.contains("`stream/harbour.ymap`")
                && types.why.contains("harbour.metadata.src"),
            "{}",
            types.why
        );
        let line = &root.lines[1];
        assert!(
            line.carried && line.how.contains("the Files content"),
            "a type file that is kept whole keeps its line: {line:?}"
        );
        assert!(
            report
                .notes
                .iter()
                .any(|note| note.contains("type file") && note.contains("kept whole")),
            "{:?}",
            report.notes
        );

        let into = dir.path().join(into);
        let adopted = adopt(&mut place, &report, &request(&into)).unwrap();
        assert_eq!(
            adopted.root, report.roots[0],
            "what the report said is what was made"
        );
        assert_eq!(adopted.sources.len(), 2);
        for kept in [&files[2], &files[4]] {
            assert_eq!(
                std::fs::read(into.join(&kept.0)).unwrap(),
                kept.1,
                "{}",
                kept.0
            );
        }
        let project = Project::load(&adopted.project).unwrap();
        let stated: Vec<(String, String)> = (project.container.modules.iter())
            .find(|module| module.kind == ModuleKind::Files)
            .map(|module| module.file_types().into_iter().collect())
            .unwrap();
        assert_eq!(
            stated,
            [(
                "stream/harbour.ytyp".to_string(),
                "DLC_ITYP_REQUEST".to_string()
            )]
        );
    }
}

#[test]
fn what_stops_an_adoption_is_said_before_anything_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let files: Files = vec![
        ("east/stream/harbour.ymap".into(), the_map()),
        ("west/stream/harbour.ymap".into(), the_map()),
    ];
    let (folder, archive) = both(dir.path(), &files);
    let before = tree(dir.path());
    let held = dir.path().join("held");
    std::fs::create_dir_all(&held).unwrap();
    std::fs::write(held.join("mine.txt"), b"mine").unwrap();
    let new = dir.path().join("new");

    let place = open(&archive);
    let mut from_folder = open(&folder);
    let (report, of_folder) = (analyse(&mut open(&archive)), analyse(&mut from_folder));
    fn east(into: &Path) -> Request<'_> {
        Request {
            root: Some("east"),
            ..request(into)
        }
    }
    let inside = folder.join("project");
    assert!(matches!(check(&place, &report, &east(&new)), Ok(())));
    assert!(matches!(
        check(&place, &report, &request(&new)),
        Err(Error::Several(_))
    ));
    let north = Request {
        root: Some("north"),
        ..request(&new)
    };
    assert!(matches!(
        check(&place, &report, &north),
        Err(Error::NoSuchRoot(_))
    ));
    assert!(matches!(
        check(&place, &report, &east(&held)),
        Err(Error::Held(_))
    ));
    assert!(matches!(
        check(&from_folder, &of_folder, &east(&inside)),
        Err(Error::Inside(_))
    ));
    // A path that fits the listing and not the folder it would be unpacked into.
    let mut limits = Limits::default();
    limits
        .set("path", "east/stream/harbour.ymap".len() as u64)
        .unwrap();
    let tight = Place::open(&archive, &limits).unwrap();
    let refused = check(&tight, &report, &east(&new)).unwrap_err();
    assert!(
        matches!(&refused, Error::Refused(Refusal::PathTooLong { entry, .. }) if entry == "east/stream/harbour.ymap"),
        "{refused}"
    );

    // What `check` says is what `adopt` then does, and neither wrote anything to say it.
    let mut tight = tight;
    assert!(matches!(
        adopt(&mut tight, &report, &east(&new)),
        Err(Error::Refused(Refusal::PathTooLong { .. }))
    ));
    let mut after = tree(dir.path());
    after.retain(|(name, _)| name != "held/mine.txt");
    assert_eq!(after, before);
    assert!(!new.exists());
}
