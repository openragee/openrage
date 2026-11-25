//! Any resource folder is read as a server would read it: a sound one reports nothing, each rule fires on a folder built to break it and on
//! no other, a manifest this cannot read costs notes, a file an asset escrow locks is counted and not opened, the text and the JSON state
//! the same findings, and a path that leaves the resource is not followed.
//!
//! Every folder is built by hand in a scratch directory: the map and the type file from a schema written here, the dictionaries from nothing.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use common::{legacy_ytd, write};
use or_io::decl::{File, Gen8};
use or_io::gta5::{GtaDrawable, PgDictionary};
use or_io::meta::instance::HashedString;
use or_io::meta::{build_library, canonical, write_meta_binary, BinaryFormat, MetaInstance, Value};
use or_project::content::rule::Severity;
use or_project::resource::{
    check, Install, Options, Report, ESCROW_MARKER, FILE_MEMORY_LIMIT, RULES,
};

const PSC: &str = r#"<?xml version="1.0"?>
<ParserSchema>
  <structdef type="CEntityDef">
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
    <s32 name="ambientOcclusionMultiplier"/>
    <s32 name="artificialAmbientOcclusion"/>
    <u32 name="tintValue"/>
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
    <array name="entities" type="atArray"><pointer type="CEntityDef" policy="owner"/></array>
  </structdef>
  <structdef type="CBaseArchetypeDef">
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
  </structdef>
  <structdef type="CMloArchetypeDef" base="CBaseArchetypeDef">
    <u32 name="mloFlags"/>
    <array name="entities" type="atArray"><pointer type="CEntityDef" policy="owner"/></array>
  </structdef>
  <structdef type="CMapTypes">
    <string name="name" type="atHashString"/>
    <array name="archetypes" type="atArray"><pointer type="CBaseArchetypeDef" policy="owner"/></array>
    <array name="dependencies" type="atArray"><string type="atHashString"/></array>
  </structdef>
</ParserSchema>"#;

fn encode(root: &MetaInstance) -> Vec<u8> {
    let library = build_library([("inline.psc", PSC.as_bytes())]).expect("the schema parses");
    let layout =
        canonical::layout_for(&library, root, BinaryFormat::Rbm, None).expect("the file lays out");
    write_meta_binary(BinaryFormat::Rbm, Some(&layout), root).expect("the file encodes")
}

fn hash(name: &str) -> Value {
    Value::Hash(match name {
        "" => HashedString::from_hash(0),
        name => HashedString::new(name),
    })
}

/// A `.ymap` that places one entity of each archetype named.
fn map_file(name: &str, places: &[&str]) -> Vec<u8> {
    encode(&map_root(name, places))
}

/// The root of a map that places one entity of each archetype named.
fn map_root(name: &str, places: &[&str]) -> MetaInstance {
    let entity = |archetype: &&str| {
        let mut inst = MetaInstance::new("CEntityDef");
        inst.set("archetypeName", hash(archetype));
        inst.set("flags", Value::UInt(32));
        inst.set("guid", Value::UInt(1));
        inst.set("position", Value::Vec3([1.0, 2.0, 3.0]));
        inst.set("rotation", Value::Vec4([0.0, 0.0, 0.0, 1.0]));
        inst.set("scaleXY", Value::Float(1.0));
        inst.set("scaleZ", Value::Float(1.0));
        inst.set("parentIndex", Value::Int(-1));
        inst.set("lodDist", Value::Float(100.0));
        inst.set("childLodDist", Value::Float(0.0));
        inst.set("numChildren", Value::UInt(0));
        inst.set("ambientOcclusionMultiplier", Value::Int(255));
        inst.set("artificialAmbientOcclusion", Value::Int(255));
        inst.set("tintValue", Value::UInt(0));
        Value::Pointer(Some(Box::new(inst)))
    };
    let mut root = MetaInstance::new("CMapData");
    root.set("name", hash(name));
    root.set("parent", hash(""));
    root.set("flags", Value::UInt(0));
    root.set("contentFlags", Value::UInt(65));
    for extent in ["streamingExtentsMin", "entitiesExtentsMin"] {
        root.set(extent, Value::Vec3([0.0; 3]));
    }
    for extent in ["streamingExtentsMax", "entitiesExtentsMax"] {
        root.set(extent, Value::Vec3([10.0; 3]));
    }
    root.set(
        "entities",
        Value::Array(places.iter().map(entity).collect()),
    );
    root
}

/// One archetype of a type file made here.
#[derive(Clone, Copy)]
struct Archetype {
    name: &'static str,
    /// The asset it names; empty for an archetype that names none, whose asset is its own name.
    asset: &'static str,
    drawable_dictionary: &'static str,
    texture_dictionary: &'static str,
    interior: bool,
}

/// A `.ytyp` that declares `archetypes`.
fn types_file(name: &str, archetypes: &[Archetype]) -> Vec<u8> {
    let archetype = |a: &Archetype| {
        let class = if a.interior {
            "CMloArchetypeDef"
        } else {
            "CBaseArchetypeDef"
        };
        let mut inst = MetaInstance::new(class);
        inst.set("lodDist", Value::Float(100.0));
        inst.set("flags", Value::UInt(0));
        inst.set("specialAttribute", Value::UInt(0));
        inst.set("bbMin", Value::Vec3([-1.0; 3]));
        inst.set("bbMax", Value::Vec3([1.0; 3]));
        inst.set("bsCentre", Value::Vec3([0.0; 3]));
        inst.set("bsRadius", Value::Float(2.0));
        inst.set("hdTextureDist", Value::Float(5.0));
        inst.set("name", hash(a.name));
        inst.set("textureDictionary", hash(a.texture_dictionary));
        inst.set("clipDictionary", hash(""));
        inst.set("drawableDictionary", hash(a.drawable_dictionary));
        inst.set("physicsDictionary", hash(""));
        inst.set("assetName", hash(a.asset));
        if a.interior {
            inst.set("mloFlags", Value::UInt(0));
            inst.set("entities", Value::Array(Vec::new()));
        }
        Value::Pointer(Some(Box::new(inst)))
    };
    let mut root = MetaInstance::new("CMapTypes");
    root.set("name", hash(name));
    root.set(
        "archetypes",
        Value::Array(archetypes.iter().map(archetype).collect()),
    );
    root.set("dependencies", Value::Array(Vec::new()));
    encode(&root)
}

/// A drawable dictionary that holds no drawable, which is a model file by its name.
fn empty_ydd() -> Vec<u8> {
    PgDictionary::<GtaDrawable<Gen8>>::default()
        .to_bytes()
        .expect("an empty dictionary saves")
}

const MANIFEST: &str = "-- the yard\n\
fx_version 'cerulean'\n\
game 'gta5'\n\
this_is_a_map 'yes'\n\
\n\
files {\n\
\x20 'data/props.meta',\n\
\x20 \"data/**/*.xml\",\n\
}\n\
\n\
data_file 'DLC_ITYP_REQUEST' 'stream/props.ytyp'\n\
data_file 'AUDIO_GAMEDATA' 'audio/yard_game.dat'\n\
data_file 'AUDIO_WAVEPACK' 'audio/sfx'\n";

/// The archetypes of the sound resource: a prop in a drawable dictionary, a prop that is its own model, and an interior.
const DECLARED: [Archetype; 3] = [
    Archetype {
        name: "yard_crate",
        asset: "yard_crate",
        drawable_dictionary: "yard_props",
        texture_dictionary: "yard_txd",
        interior: false,
    },
    Archetype {
        name: "yard_gate",
        asset: "",
        drawable_dictionary: "",
        texture_dictionary: "yard_txd",
        interior: false,
    },
    Archetype {
        name: "yard_office",
        asset: "yard_office",
        drawable_dictionary: "",
        texture_dictionary: "",
        interior: true,
    },
];

/// A resource nothing is wrong with: a manifest, the files it names, and a map, a type file, two models and a texture dictionary under `stream/`.
fn sound(folder: &Path) {
    write(&folder.join("fxmanifest.lua"), MANIFEST.as_bytes());
    write(&folder.join("data/props.meta"), b"<CExtraData/>\n");
    write(&folder.join("audio/yard_game.dat151.rel"), b"\0\0\0\0");
    write(&folder.join("audio/sfx/yard.awc"), b"\0\0\0\0");
    write(
        &folder.join("stream/props.ytyp"),
        &types_file("props", &DECLARED),
    );
    write(&folder.join("stream/models/yard_props.ydd"), &empty_ydd());
    write(&folder.join("stream/models/yard_gate.ydd"), &empty_ydd());
    write(
        &folder.join("stream/models/yard_txd.ytd"),
        &legacy_ytd("yard_wood"),
    );
    write(
        &folder.join("stream/maps/yard.ymap"),
        &map_file("yard", &["yard_crate", "yard_gate", "yard_office"]),
    );
}

/// An install that knows the names it is given, each as an archetype, a model and a texture dictionary.
#[derive(Default)]
struct Knows(Vec<&'static str>);

impl Knows {
    fn has(&self, name: u32) -> bool {
        (self.0.iter()).any(|known| or_io::string_hash(known.as_bytes()) == name)
    }
}

impl Install for Knows {
    fn declares_archetype(&self, name: u32) -> bool {
        self.has(name)
    }
    fn holds_model(&self, name: u32) -> bool {
        self.has(name)
    }
    fn holds_texture_dictionary(&self, name: u32) -> bool {
        self.has(name)
    }
}

/// A name as a finding states one the file keeps only the hash of.
fn literal(name: &str) -> String {
    format!("`{}`", HashedString::new(name).to_literal())
}

fn read(folder: &Path) -> Report {
    check(folder, &Options::default())
}

fn codes(report: &Report) -> BTreeSet<&'static str> {
    report.findings.iter().map(|f| f.code).collect()
}

fn replace_in_manifest(folder: &Path, from: &str, to: &str) {
    let at = folder.join("fxmanifest.lua");
    let text = std::fs::read_to_string(&at).unwrap();
    assert!(text.contains(from), "the manifest has no {from:?}");
    std::fs::write(at, text.replace(from, to)).unwrap();
}

/// What is not a texture dictionary, a model or a map: the bytes a reader of any streamed type refuses.
const REFUSED: &[u8] = b"\0\0\0\0these bytes are no resource a reader opens";

#[test]
fn a_sound_resource_reports_nothing() {
    let dir = tempfile::tempdir().unwrap();
    sound(dir.path());
    let report = read(dir.path());

    assert_eq!(report.findings, [], "{:#?}", report.findings);
    assert!(!report.has_error());
    assert_eq!(report.manifest.as_deref(), Some("fxmanifest.lua"));
    assert_eq!(report.version.as_deref(), Some("cerulean"));
    assert_eq!(report.games, ["gta5"]);
    assert!(report.map);
    assert_eq!(
        (report.streamed, report.opened, report.locked),
        (5, 5, 0),
        "every streamed file opens through its reader"
    );

    // With an install that knows nothing, the resource still holds every name it states.
    let nothing = Knows::default();
    let asked = check(
        dir.path(),
        &Options {
            install: Some(&nothing),
            ..Options::default()
        },
    );
    assert_eq!(asked.findings, [], "{:#?}", asked.findings);
}

#[test]
fn a_resource_that_streams_nothing_is_sound() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = b"fx_version 'cerulean'\ngame 'gta5'\nclient_script 'client.lua'\n";
    write(&dir.path().join("fxmanifest.lua"), manifest);
    let report = read(dir.path());
    assert_eq!(report.findings, [], "{:#?}", report.findings);
    assert_eq!(report.streamed, 0);

    // A file of that name is no folder of streamed files.
    write(&dir.path().join("stream"), REFUSED);
    let report = read(dir.path());
    assert_eq!(report.findings, [], "{:#?}", report.findings);
}

#[test]
fn the_older_manifest_with_its_own_version_line_is_a_manifest() {
    let dir = tempfile::tempdir().unwrap();
    sound(dir.path());
    std::fs::rename(
        dir.path().join("fxmanifest.lua"),
        dir.path().join("__resource.lua"),
    )
    .unwrap();
    let at = dir.path().join("__resource.lua");
    let text = std::fs::read_to_string(&at).unwrap();
    std::fs::write(
        &at,
        text.replace("fx_version 'cerulean'", "resource_manifest_version 'v1'"),
    )
    .unwrap();

    let report = read(dir.path());
    assert_eq!(report.findings, [], "{:#?}", report.findings);
    assert_eq!(report.manifest.as_deref(), Some("__resource.lua"));
    assert_eq!(report.version.as_deref(), Some("v1"));
}

/// A folder built to break one rule.
struct Case {
    code: &'static str,
    /// Whether an install is asked, one that knows nothing.
    asked: bool,
    /// The ceiling stated for the memory of one file.
    file_memory: Option<u64>,
    breaks: fn(&Path),
}

const fn case(code: &'static str, breaks: fn(&Path)) -> Case {
    Case {
        code,
        asked: false,
        file_memory: None,
        breaks,
    }
}

fn no_model(folder: &Path) {
    std::fs::remove_file(folder.join("stream/models/yard_props.ydd")).unwrap();
}

fn stray_entity(folder: &Path) {
    write(
        &folder.join("stream/maps/yard.ymap"),
        &map_file("yard", &["yard_crate", "nobody_declares_this"]),
    );
}

fn no_textures(folder: &Path) {
    std::fs::remove_file(folder.join("stream/models/yard_txd.ytd")).unwrap();
}

const CASES: &[Case] = &[
    case("manifest-missing", |folder| {
        std::fs::remove_file(folder.join("fxmanifest.lua")).unwrap();
    }),
    case("version-missing", |folder| {
        replace_in_manifest(folder, "fx_version 'cerulean'\n", "");
    }),
    case("manifest-line-unread", |folder| {
        replace_in_manifest(
            folder,
            "game 'gta5'\n",
            "game 'gta5'\nlocal built = os.time()\n",
        );
    }),
    case("path-escapes", |folder| {
        replace_in_manifest(folder, "'data/props.meta'", "'../elsewhere/props.meta'");
    }),
    case("file-missing", |folder| {
        std::fs::remove_file(folder.join("data/props.meta")).unwrap();
    }),
    case("file-twice", |folder| {
        std::fs::copy(
            folder.join("stream/models/yard_txd.ytd"),
            folder.join("stream/maps/YARD_TXD.ytd"),
        )
        .unwrap();
    }),
    case("file-not-streamed", |folder| {
        write(
            &folder.join("stream/models/readme.txt"),
            b"how to install\n",
        );
        // A type the tree has a reader for and a server does not stream is not opened either.
        write(&folder.join("stream/models/loose.dds"), REFUSED);
    }),
    case("file-locked", |folder| {
        let mut locked = ESCROW_MARKER.to_vec();
        locked.extend_from_slice(REFUSED);
        write(&folder.join("stream/models/yard_sealed.ytd"), &locked);
    }),
    case("file-unreadable", |folder| {
        write(&folder.join("stream/models/yard_broken.ytd"), REFUSED);
    }),
    Case {
        file_memory: Some(1),
        ..case("file-oversized", |_| {})
    },
    case("map-not-declared", |folder| {
        replace_in_manifest(folder, "this_is_a_map 'yes'\n", "");
    }),
    Case {
        asked: true,
        ..case("model-missing", no_model)
    },
    case("archetype-without-model", no_model),
    Case {
        asked: true,
        ..case("archetype-missing", stray_entity)
    },
    case("archetype-undeclared", stray_entity),
    Case {
        asked: true,
        ..case("texture-dictionary-missing", no_textures)
    },
    case("texture-dictionary-elsewhere", no_textures),
];

/// The rules that fail a check and the ones that are notes, said here and not read from the table: a row that changes its severity is seen.
const ERRORS: [&str; 4] = [
    "manifest-missing",
    "version-missing",
    "file-missing",
    "file-unreadable",
];
const NOTES: [&str; 5] = [
    "manifest-line-unread",
    "file-locked",
    "archetype-without-model",
    "archetype-undeclared",
    "texture-dictionary-elsewhere",
];

#[test]
fn each_rule_fires_on_the_folder_built_to_break_it_and_on_no_other() {
    for rule in RULES {
        let stated = match rule.code {
            code if ERRORS.contains(&code) => Severity::Error,
            code if NOTES.contains(&code) => Severity::Info,
            _ => Severity::Warning,
        };
        assert_eq!(rule.severity, stated, "`{}`", rule.code);
        let cases: Vec<&Case> = CASES.iter().filter(|c| c.code == rule.code).collect();
        let [case] = cases.as_slice() else {
            panic!(
                "the rule `{}` has {} folders built to break it, and needs one",
                rule.code,
                cases.len()
            );
        };
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("yard");
        sound(&folder);
        (case.breaks)(&folder);
        let nothing = Knows::default();
        let limits: BTreeMap<String, u64> = (case.file_memory.iter())
            .map(|max| (FILE_MEMORY_LIMIT.to_string(), *max))
            .collect();
        let report = check(
            &folder,
            &Options {
                limits,
                install: case.asked.then_some(&nothing as &dyn Install),
            },
        );

        assert_eq!(
            codes(&report),
            BTreeSet::from([rule.code]),
            "the folder built to break `{}`: {:#?}",
            rule.code,
            report.findings
        );
        for finding in &report.findings {
            assert_eq!(finding.severity, rule.severity, "{finding:?}");
            assert!(!finding.message.is_empty(), "{finding:?}");
        }
        assert_eq!(
            report.has_error(),
            rule.severity == Severity::Error,
            "`{}` fails a check exactly when it is an error",
            rule.code
        );
    }
    for case in CASES {
        assert!(
            RULES.iter().any(|r| r.code == case.code),
            "a folder is built to break `{}`, which is no rule",
            case.code
        );
    }
}

#[test]
fn a_finding_names_the_file_and_says_what_is_wrong_with_it() {
    let said = |breaks: fn(&Path), asked: bool| {
        let dir = tempfile::tempdir().unwrap();
        sound(dir.path());
        breaks(dir.path());
        let nothing = Knows::default();
        let report = check(
            dir.path(),
            &Options {
                install: asked.then_some(&nothing as &dyn Install),
                ..Options::default()
            },
        );
        let [finding] = report.findings.as_slice() else {
            panic!("one finding was expected: {:#?}", report.findings);
        };
        (
            finding.file.clone().unwrap_or_default(),
            finding.message.clone(),
        )
    };

    let (file, message) = said(no_model, false);
    assert_eq!(file, "stream/props.ytyp");
    assert!(
        message.starts_with("1 archetype(s)") && message.contains(&literal("yard_crate")),
        "only the archetype whose dictionary went is named, not the interior and not the gate: {message}"
    );

    let (file, message) = said(stray_entity, true);
    assert_eq!(file, "stream/maps/yard.ymap");
    assert!(
        message.starts_with("1 archetype(s)") && message.contains(&literal("nobody_declares_this")),
        "{message}"
    );

    let (file, message) = said(no_textures, false);
    assert_eq!(file, "stream/props.ytyp");
    assert!(
        message.starts_with("1 texture dictionary(ies)") && message.contains(&literal("yard_txd")),
        "two archetypes name one dictionary, which is named once: {message}"
    );

    let (file, message) = said(
        |folder| write(&folder.join("stream/models/yard_broken.ytd"), REFUSED),
        false,
    );
    assert_eq!(file, "stream/models/yard_broken.ytd");
    let refusal = or_io::gta5::formats::format("ytd")
        .expect("a declared format")
        .open_declared(REFUSED)
        .expect_err("the reader refuses these bytes")
        .to_string();
    assert!(
        message.contains(&refusal),
        "the reader's own error is named: {message}"
    );

    let (file, message) = said(
        |folder| std::fs::remove_file(folder.join("data/props.meta")).unwrap(),
        false,
    );
    assert_eq!(file, "data/props.meta");
    assert!(message.contains("line 6"), "{message}");
}

#[test]
fn an_install_answers_for_a_name_the_resource_does_not_hold() {
    let dir = tempfile::tempdir().unwrap();
    sound(dir.path());
    no_model(dir.path());
    no_textures(dir.path());
    stray_entity(dir.path());

    let game = Knows(vec!["yard_props", "yard_txd", "nobody_declares_this"]);
    let report = check(
        dir.path(),
        &Options {
            install: Some(&game),
            ..Options::default()
        },
    );
    assert_eq!(
        report.findings,
        [],
        "the game holds what the resource lacks"
    );

    let other = Knows(vec!["yard_crate"]);
    let report = check(
        dir.path(),
        &Options {
            install: Some(&other),
            ..Options::default()
        },
    );
    assert_eq!(
        codes(&report),
        BTreeSet::from(["archetype-missing", "texture-dictionary-missing"]),
        "the asset of the archetype is a model of the game, and nothing else is: {:#?}",
        report.findings
    );
}

#[test]
fn a_type_file_that_does_not_open_leaves_the_maps_placements_unjudged() {
    let dir = tempfile::tempdir().unwrap();
    sound(dir.path());
    write(&dir.path().join("stream/props.ytyp"), REFUSED);
    assert_eq!(
        codes(&read(dir.path())),
        BTreeSet::from(["file-unreadable"]),
        "what the resource declares is not known, so no placement is called undeclared"
    );

    // A resource of another type under the name of a type file is refused by the reader of type files.
    write(
        &dir.path().join("stream/props.ytyp"),
        &legacy_ytd("yard_wood"),
    );
    let report = read(dir.path());
    assert_eq!(codes(&report), BTreeSet::from(["file-unreadable"]));
    assert!(
        report.findings[0].message.starts_with("does not open: "),
        "{}",
        report.findings[0].message
    );

    let mut locked = ESCROW_MARKER.to_vec();
    locked.extend_from_slice(REFUSED);
    write(&dir.path().join("stream/props.ytyp"), &locked);
    assert_eq!(codes(&read(dir.path())), BTreeSet::from(["file-locked"]));
}

#[test]
fn a_map_kept_as_text_is_not_a_file_the_game_streams() {
    let dir = tempfile::tempdir().unwrap();
    sound(dir.path());
    let text = or_io::meta::write_meta(&map_root("yard", &["yard_crate"]))
        .expect("a map writes as text")
        .into_bytes();
    // The tree's reader takes the text, so only the check stands between it and a server.
    let read = or_io::meta::typed::CMapData::from_bytes(&text).expect("the reader takes the text");
    assert_eq!(read.entities.len(), 1);
    write(&dir.path().join("stream/maps/yard.ymap"), &text);
    write(&dir.path().join("stream/maps/yard_points.ymt"), &text);
    let report = self::read(dir.path());

    let said: Vec<(&str, Option<&str>)> = (report.findings.iter())
        .map(|f| (f.code, f.file.as_deref()))
        .collect();
    assert_eq!(
        said,
        [
            ("file-unreadable", Some("stream/maps/yard.ymap")),
            ("file-unreadable", Some("stream/maps/yard_points.ymt")),
        ]
    );
    assert_eq!((report.streamed, report.opened), (6, 4));
}

#[test]
fn a_manifest_this_does_not_understand_costs_notes_and_no_error() {
    let dir = tempfile::tempdir().unwrap();
    sound(dir.path());
    std::fs::write(
        dir.path().join("fxmanifest.lua"),
        "local version = 'cerulean'\n\
         fx_version(version)\n\
         game(GetConvar('game', 'gta5'))\n\
         for _, name in ipairs({ 'data/props.meta' }) do files { name } end\n",
    )
    .unwrap();
    let report = read(dir.path());

    assert!(!report.has_error(), "{:#?}", report.findings);
    assert_eq!(codes(&report), BTreeSet::from(["manifest-line-unread"]));
    assert!(report.findings.iter().all(|f| f.severity == Severity::Info));
    assert_eq!(
        report.version, None,
        "what a manifest computes is not known"
    );
    assert!(
        report.findings[0].message.contains("lines 1 to 4"),
        "{}",
        report.findings[0].message
    );

    // A manifest that is another kind of file altogether is read the same way.
    std::fs::write(
        dir.path().join("fxmanifest.lua"),
        "{ \"fx_version\": \"cerulean\" }\n",
    )
    .unwrap();
    let report = read(dir.path());
    assert!(!report.has_error(), "{:#?}", report.findings);
    assert_eq!(codes(&report), BTreeSet::from(["manifest-line-unread"]));

    // A data-file line that states a type and no path names no file, and is said and not dropped.
    let line = MANIFEST.lines().count() + 1;
    std::fs::write(
        dir.path().join("fxmanifest.lua"),
        format!("{MANIFEST}data_file 'AUDIO_SOUNDDATA'\n"),
    )
    .unwrap();
    let report = read(dir.path());
    let [note] = report.findings.as_slice() else {
        panic!("one note was expected: {:#?}", report.findings);
    };
    assert_eq!(
        (note.code, note.severity),
        ("manifest-line-unread", Severity::Info)
    );
    assert!(
        note.message.contains(&format!("line {line} ")) && note.message.contains("1 text(s)"),
        "{}",
        note.message
    );
}

#[test]
fn a_locked_file_is_counted_named_and_not_opened() {
    let dir = tempfile::tempdir().unwrap();
    sound(dir.path());
    let mut locked = ESCROW_MARKER.to_vec();
    locked.extend_from_slice(REFUSED);
    write(&dir.path().join("stream/models/yard_sealed.ytd"), &locked);
    write(&dir.path().join("stream/models/yard_sealed.ydd"), &locked);
    let report = read(dir.path());

    assert_eq!(report.locked, 2);
    assert_eq!((report.streamed, report.opened), (7, 5));
    assert!(!report.has_error());
    let named: Vec<(&str, Option<&str>, Severity)> = (report.findings.iter())
        .map(|f| (f.code, f.file.as_deref(), f.severity))
        .collect();
    assert_eq!(
        named,
        [
            (
                "file-locked",
                Some("stream/models/yard_sealed.ydd"),
                Severity::Info
            ),
            (
                "file-locked",
                Some("stream/models/yard_sealed.ytd"),
                Severity::Info
            ),
        ]
    );

    // The bytes after the marker are ones the reader refuses: had the file been opened, this is what it would have said.
    write(&dir.path().join("stream/models/yard_sealed.ytd"), REFUSED);
    let report = read(dir.path());
    assert_eq!(report.locked, 1);
    assert!(report.has_error());
    assert!(
        (report.findings.iter()).any(|f| f.code == "file-unreadable"
            && f.file.as_deref() == Some("stream/models/yard_sealed.ytd")),
        "{:#?}",
        report.findings
    );
}

#[test]
fn a_file_is_weighed_only_against_a_limit_that_is_stated() {
    let dir = tempfile::tempdir().unwrap();
    sound(dir.path());
    let ytd = std::fs::read(dir.path().join("stream/models/yard_txd.ytd")).unwrap();
    let memory = {
        use or_io::container::resource::{ResourceEnvelope, Rsc7};
        u64::from(Rsc7::peek(&ytd).expect("an RSC7 file").total_size())
    };
    let with = |max: u64| {
        let limits = BTreeMap::from([(FILE_MEMORY_LIMIT.to_string(), max)]);
        check(
            dir.path(),
            &Options {
                limits,
                ..Options::default()
            },
        )
    };
    let over = |report: &Report| {
        (report.findings.iter())
            .filter(|f| f.code == "file-oversized")
            .filter_map(|f| f.file.clone())
            .collect::<Vec<_>>()
    };

    assert!(
        !over(&with(memory)).contains(&"stream/models/yard_txd.ytd".to_string()),
        "a file at the limit is within it"
    );
    let report = with(memory - 1);
    assert!(over(&report).contains(&"stream/models/yard_txd.ytd".to_string()));
    let said = (report.findings.iter())
        .find(|f| f.file.as_deref() == Some("stream/models/yard_txd.ytd"))
        .unwrap();
    assert!(
        said.message.contains(&memory.to_string())
            && said.message.contains(&(memory - 1).to_string()),
        "{}",
        said.message
    );
    assert_eq!(
        over(&read(dir.path())),
        Vec::<String>::new(),
        "no limit, no weighing"
    );
}

#[test]
fn a_path_that_climbs_out_of_the_resource_is_reported_and_not_followed() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("yard");
    sound(&folder);
    // The file is there, outside: a check that followed the path would find it and say nothing.
    write(&dir.path().join("elsewhere/props.meta"), b"<CExtraData/>\n");
    write(&dir.path().join("elsewhere/secret.ytyp"), REFUSED);
    replace_in_manifest(&folder, "'data/props.meta'", "'../elsewhere/props.meta'");
    replace_in_manifest(
        &folder,
        "'stream/props.ytyp'",
        "'stream/../../elsewhere/secret.ytyp'",
    );
    let outside = dir.path().join("elsewhere/absolute.meta");
    write(&outside, b"<CExtraData/>\n");
    replace_in_manifest(
        &folder,
        "\"data/**/*.xml\"",
        &format!("'{}'", outside.display().to_string().replace('\\', "/")),
    );
    // A pattern that climbs out is a path that climbs out.
    replace_in_manifest(&folder, "files {\n", "files {\n  '../elsewhere/*.meta',\n");
    let report = read(&folder);

    assert_eq!(codes(&report), BTreeSet::from(["path-escapes"]));
    assert_eq!(report.findings.len(), 4, "{:#?}", report.findings);
    assert!(!report.has_error());
    for stated in [
        "../elsewhere/props.meta",
        "stream/../../elsewhere/secret.ytyp",
        "../elsewhere/*.meta",
    ] {
        assert!(
            (report.findings.iter()).any(|f| f.message.contains(stated)),
            "{stated} is not reported: {:#?}",
            report.findings
        );
    }

    // A path that goes up and stays inside is a path of the resource.
    replace_in_manifest(
        &folder,
        "'../elsewhere/props.meta'",
        "'stream/../data/props.meta'",
    );
    assert_eq!(read(&folder).findings.len(), 3);
}

#[test]
fn a_hidden_entry_and_a_link_under_the_stream_folder_are_not_walked() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("yard");
    sound(&folder);
    write(&folder.join("stream/.DS_Store"), REFUSED);
    write(&folder.join("stream/.cache/yard_old.ytd"), REFUSED);
    // What a link names is outside the resource: a walk that followed it would open these and refuse them.
    #[cfg(unix)]
    {
        write(&dir.path().join("elsewhere/yard_far.ytd"), REFUSED);
        std::os::unix::fs::symlink(dir.path().join("elsewhere"), folder.join("stream/far"))
            .unwrap();
        std::os::unix::fs::symlink(
            dir.path().join("elsewhere/yard_far.ytd"),
            folder.join("stream/yard_far.ytd"),
        )
        .unwrap();
    }
    let report = read(&folder);

    assert_eq!(report.findings, [], "{:#?}", report.findings);
    assert_eq!((report.streamed, report.opened), (5, 5));
}

#[test]
fn the_json_form_lists_the_findings_the_text_lists() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("yard");
    sound(&folder);
    no_model(&folder);
    stray_entity(&folder);
    write(&folder.join("stream/models/yard_broken.ytd"), REFUSED);
    // Read before the file of a type no server streams, and listed after it.
    write(&folder.join("stream/maps/yard_broken.ydd"), REFUSED);
    write(
        &folder.join("stream/models/readme.txt"),
        b"how to install\n",
    );
    replace_in_manifest(&folder, "'data/props.meta'", "'data/gone.meta'");
    replace_in_manifest(&folder, "this_is_a_map 'yes'\n", "local x = 1\n");
    replace_in_manifest(
        &folder,
        "game 'gta5'\n",
        "game 'gta5'\ngames { 'gta5', 'rdr3' }\n",
    );
    let report = read(&folder);
    assert!(report.findings.len() >= 6, "{:#?}", report.findings);
    let rows: Vec<usize> = (report.findings.iter())
        .filter_map(|f| RULES.iter().position(|r| r.code == f.code))
        .collect();
    assert_eq!(rows.len(), report.findings.len());
    assert!(
        rows.is_sorted(),
        "the findings are listed in the order of the rules, whatever order the files were read in: {:#?}",
        report.findings
    );
    assert_eq!(report.games, ["gta5", "rdr3"], "a game named twice is one");
    let severities: BTreeSet<Severity> = report.findings.iter().map(|f| f.severity).collect();
    assert_eq!(
        severities.len(),
        3,
        "an error, a warning and a note are all stated"
    );

    let json = serde_json::to_value(&report).expect("a report is data");
    let listed = json["findings"]
        .as_array()
        .expect("the findings are a list");
    let lines = report.lines();
    assert!(lines[1].contains("game gta5 and rdr3"), "{}", lines[1]);
    let stated: Vec<&String> = lines
        .iter()
        .filter(|line| {
            ["  error ", "  warning ", "  note "]
                .iter()
                .any(|word| line.starts_with(word))
        })
        .collect();
    assert_eq!(listed.len(), report.findings.len());
    assert_eq!(stated.len(), listed.len(), "{lines:#?}");
    for (entry, line) in listed.iter().zip(stated) {
        let word = match entry["severity"].as_str().expect("a severity") {
            "error" => "error",
            "warning" => "warning",
            "info" => "note",
            other => panic!("`{other}` is no severity"),
        };
        assert!(line.trim_start().starts_with(word), "{line}");
        assert!(
            line.contains(entry["code"].as_str().expect("a code")),
            "{line}"
        );
        assert!(
            line.contains(entry["message"].as_str().expect("a sentence")),
            "{line}"
        );
        match entry["file"].as_str() {
            Some(file) => assert!(line.contains(file), "{line}"),
            None => assert!(entry["file"].is_null()),
        }
    }
    assert_eq!(json["manifest"], "fxmanifest.lua");
    assert_eq!(json["streamed"], report.streamed);
    let counts = format!(
        "{} error(s), {} warning(s), {} note(s)",
        report.count(Severity::Error),
        report.count(Severity::Warning),
        report.count(Severity::Info)
    );
    assert!(lines.contains(&counts), "{lines:#?}");
    assert!(
        lines
            .last()
            .is_some_and(|last| last.contains("cannot see whether the game loads a file")),
        "{lines:#?}"
    );
}

#[cfg(unix)]
#[test]
fn a_folder_under_the_stream_folder_that_cannot_be_read_is_said() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("yard");
    sound(&folder);
    let shut = folder.join("stream/shut");
    write(&shut.join("yard_hidden.ytd"), &legacy_ytd("yard_wood"));
    let mode = |bits| std::fs::set_permissions(&shut, std::fs::Permissions::from_mode(bits));
    mode(0o000).unwrap();
    if std::fs::read_dir(&shut).is_ok() {
        // Whoever may read every folder, as root may, has none to be shut out of.
        mode(0o755).unwrap();
        return;
    }
    let report = self::read(&folder);
    mode(0o755).unwrap();

    let [finding] = report.findings.as_slice() else {
        panic!("one finding was expected: {:#?}", report.findings);
    };
    assert_eq!(
        (finding.code, finding.file.as_deref()),
        ("file-unreadable", Some("stream/shut"))
    );
    assert!(
        report.has_error(),
        "files a check could not see are not passed"
    );
}
