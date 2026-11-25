//! A server's resources folder is read as one thing: every resource under it through its categories, each file once, kept as names,
//! sizes and digests; two resources that hold one name are a finding that names both; a configuration says which starts last; and a
//! build is told only what concerns the files and the archetypes it is about to write.
//!
//! Every folder is built by hand in a scratch directory, and the type files and maps are made from nothing.

mod common;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use common::write;
use or_io::decl::File;
use or_io::meta::instance::HashedString;
use or_io::meta::typed::{CBaseArchetypeDef, CBaseArchetypeDefAny, CMapData, CMapTypes};
use or_project::content::rule::Severity;
use or_project::resource::ESCROW_MARKER;
use or_project::server::config::{Config, NotRead};
use or_project::server::{
    concerning, findings, placement, rule_of, scan, scan_with, Disk, Files, Finding, GameFiles,
    Report, Scan, ARCHETYPE_CLAIM,
};
use or_project::{
    plan_with, Claim, Container, Contribution, Module, ModuleKind, Origin, Output, OutputKind,
    Plan, PlannedFile,
};
use sha2::{Digest, Sha256};

const MANIFEST: &[u8] = b"fx_version 'cerulean'\ngame 'gta5'\nthis_is_a_map 'yes'\n";

/// What no reader of a streamed type opens.
const REFUSED: &[u8] = b"these bytes are no metadata a reader opens";

/// A `.ytyp` that declares `archetypes`.
fn types_file(name: &str, archetypes: &[&str]) -> Vec<u8> {
    let archetype = |name: &&str| {
        let mut archetype = CBaseArchetypeDef::default();
        archetype.name = HashedString::new(*name);
        archetype.asset_name = HashedString::new(*name);
        CBaseArchetypeDefAny::Base(archetype)
    };
    let mut types = CMapTypes::default();
    types.name = HashedString::new(name);
    types.archetypes = archetypes.iter().map(archetype).collect();
    (types.to_bytes()).expect("a type file built from nothing saves")
}

/// A `.ymap` that states `name` and places nothing.
fn map_file(name: &str) -> Vec<u8> {
    let mut map = CMapData::default();
    map.name = HashedString::new(name);
    map.to_bytes().expect("a map built from nothing saves")
}

/// A name as a scan shows one the file keeps only the hash of.
fn literal(name: &str) -> String {
    HashedString::new(name).to_literal()
}

/// A resource at `path` under `root` with a manifest and the files given, each inside the resource.
fn resource(root: &Path, path: &str, files: &[(&str, &[u8])]) {
    write(&root.join(path).join("fxmanifest.lua"), MANIFEST);
    for (inside, bytes) in files {
        write(&root.join(path).join(inside), bytes);
    }
}

/// The disk, counting how often each file is opened and refusing the files it is told to.
#[derive(Default)]
struct Counting {
    opened: RefCell<BTreeMap<PathBuf, usize>>,
    refused: Vec<PathBuf>,
}

impl Files for Counting {
    fn open(&self, path: &Path) -> std::io::Result<Box<dyn Read + '_>> {
        *(self.opened.borrow_mut())
            .entry(path.to_path_buf())
            .or_default() += 1;
        if self.refused.iter().any(|refused| refused == path) {
            return Err(std::io::Error::other("refused by the test"));
        }
        Disk.open(path)
    }
}

impl Counting {
    fn times(&self, path: &Path) -> usize {
        self.opened.borrow().get(path).copied().unwrap_or(0)
    }
}

fn of_code<'a>(found: &'a [Finding], code: &str) -> Vec<&'a Finding> {
    assert!(rule_of(code).is_some(), "{code} is no rule");
    found.iter().filter(|f| f.code == code).collect()
}

fn paths(scan: &Scan) -> Vec<&str> {
    scan.resources.iter().map(|r| r.path.as_str()).collect()
}

/// A server folder with a `resources` folder of two resources, `a` and `b`, that stream one map with different bytes and one dictionary with the same.
fn two_that_clash(server: &Path) -> PathBuf {
    let root = server.join("resources");
    let a_map = map_file("harbour");
    let b_map = map_file("harbour_moved");
    assert_ne!(a_map, b_map);
    resource(
        &root,
        "a",
        &[
            ("stream/harbour.ymap", &a_map),
            ("stream/shared.ytd", b"one dictionary"),
            ("stream/a_only.ydr", b"a model"),
        ],
    );
    resource(
        &root,
        "b",
        &[
            ("stream/maps/Harbour.ymap", &b_map),
            ("stream/shared.ytd", b"one dictionary"),
        ],
    );
    root
}

#[test]
fn two_resources_that_stream_one_name_are_a_warning_for_different_bytes_and_a_note_for_the_same() {
    let dir = tempfile::tempdir().unwrap();
    let root = two_that_clash(dir.path());
    let read = scan(&root);
    assert_eq!(paths(&read), ["a", "b"]);
    let found = findings(&read, None, None);

    let [different] = of_code(&found, "file-in-two-resources")[..] else {
        panic!("one finding for the map: {found:#?}");
    };
    assert_eq!(different.severity, Severity::Warning);
    assert_eq!(different.resources, ["a", "b"]);
    // A server resolves a streamed file by its name, whatever its folder and its case.
    assert_eq!(different.names, ["harbour.ymap"]);
    assert!(
        different.message.contains("`a` and `b`") && different.message.contains("different bytes"),
        "{}",
        different.message
    );

    let [same] = of_code(&found, "file-in-two-resources-same")[..] else {
        panic!("one finding for the dictionary: {found:#?}");
    };
    assert_eq!(same.severity, Severity::Info);
    assert_eq!(same.resources, ["a", "b"]);
    assert_eq!(same.names, ["shared.ytd"]);
    assert!(same.message.contains("the same bytes"), "{}", same.message);

    assert_eq!(
        found.len(),
        2,
        "a file one resource streams is no finding: {found:#?}"
    );

    // What is kept of a file is its name, its size and the digest of its bytes.
    let kept = (read.resources[0].streamed.iter())
        .find(|f| f.name == "shared.ytd")
        .expect("the dictionary");
    assert_eq!(kept.bytes, b"one dictionary".len() as u64);
    let digest = format!("{:x}", Sha256::digest(b"one dictionary"));
    assert_eq!(kept.digest.as_deref(), Some(digest.as_str()));
}

#[test]
fn a_configuration_says_which_of_two_starts_last_and_without_one_the_order_is_not_known() {
    let dir = tempfile::tempdir().unwrap();
    let root = two_that_clash(dir.path());
    let read = scan(&root);
    let cfg = dir.path().join("server.cfg");
    let clash = |text: &str| {
        write(&cfg, text.as_bytes());
        let config = Config::read(&cfg, &Disk).expect("the configuration reads");
        let found = findings(&read, Some(&config), None);
        let [clash] = of_code(&found, "file-in-two-resources")[..] else {
            panic!("one finding for the map: {found:#?}");
        };
        clash.clone()
    };

    let none = findings(&read, None, None);
    let [unordered] = of_code(&none, "file-in-two-resources")[..] else {
        panic!("{none:#?}");
    };
    assert_eq!(unordered.starts_last, None);
    assert!(
        unordered
            .message
            .contains("not known: no configuration was given"),
        "{}",
        unordered.message
    );

    let b_last = clash("sv_hostname \"a server\"\nensure a\n# then the other\nensure b\n");
    assert_eq!(b_last.starts_last.as_deref(), Some("b"));
    assert!(
        b_last
            .message
            .contains("`b` starts last, on line 4 of `server.cfg`"),
        "{}",
        b_last.message
    );
    let a_last = clash("start b; ensure a\n");
    assert_eq!(a_last.starts_last.as_deref(), Some("a"));
    // By the first line that starts a resource: a second line for `b` moves nothing.
    let still_a = clash("ensure b\nensure a\nensure b\n");
    assert_eq!(still_a.starts_last.as_deref(), Some("a"));

    // An included file runs where its line stands.
    write(&dir.path().join("cfg/maps.cfg"), b"ensure a\n");
    let included = clash("ensure b\nexec cfg/maps.cfg\n");
    assert_eq!(included.starts_last.as_deref(), Some("a"));
    assert!(
        included.message.contains("line 1 of `cfg/maps.cfg`"),
        "{}",
        included.message
    );
    let before = clash("exec cfg/maps.cfg\nensure b\n");
    assert_eq!(before.starts_last.as_deref(), Some("b"));

    let only = clash("ensure a\n");
    assert_eq!(only.starts_last, None);
    assert!(only.message.contains("starts only `a`"), "{}", only.message);
    let neither = clash("ensure c\n");
    assert_eq!(neither.starts_last, None);
    assert!(
        neither.message.contains("starts none of them"),
        "{}",
        neither.message
    );
}

#[test]
fn resources_a_category_starts_together_have_no_order_between_them() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("resources");
    resource(&root, "[maps]/a", &[("stream/x.ydr", b"one")]);
    resource(&root, "[maps]/[more]/b", &[("stream/x.ydr", b"two")]);
    resource(&root, "c", &[("stream/x.ydr", b"three")]);
    let read = scan(&root);
    let cfg = dir.path().join("server.cfg");
    let clash = |text: &str| {
        write(&cfg, text.as_bytes());
        let config = Config::read(&cfg, &Disk).expect("the configuration reads");
        let found = findings(&read, Some(&config), None);
        let [clash] = of_code(&found, "file-in-two-resources")[..] else {
            panic!("one finding: {found:#?}");
        };
        clash.clone()
    };

    let together = clash("ensure c\nensure [maps]\n");
    assert_eq!(together.resources, ["[maps]/[more]/b", "[maps]/a", "c"]);
    assert_eq!(together.starts_last, None);
    assert!(
        together
            .message
            .contains("start together with the category `[maps]` on line 2"),
        "{}",
        together.message
    );
    let after = clash("ensure [maps]\nensure c\n");
    assert_eq!(after.starts_last.as_deref(), Some("c"));
    // A category inside a category is started by its own line where one names it first.
    let inner = clash("ensure c\nensure a\nensure [more]\n");
    assert_eq!(inner.starts_last.as_deref(), Some("[maps]/[more]/b"));
    // A resource in a category is started by a line that names it, and by its name alone.
    let named = clash("ensure [more]\nensure c\nensure a\n");
    assert_eq!(named.starts_last.as_deref(), Some("[maps]/a"));
    let by_path = clash("ensure [more]\nensure c\nensure [maps]/a\n");
    assert_eq!(by_path.starts_last.as_deref(), Some("c"));
    // One the configuration does not start is said, beside the one that starts last.
    let idle = clash("ensure c\nensure a\n");
    assert_eq!(idle.starts_last.as_deref(), Some("[maps]/a"));
    assert!(
        idle.message.contains(
            "`[maps]/a` starts last, on line 2 of `server.cfg`, and the configuration does not start `[maps]/[more]/b`"
        ),
        "{}",
        idle.message
    );
}

#[test]
fn a_category_in_brackets_is_walked_and_a_plain_folder_with_no_manifest_is_no_resource() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("resources");
    resource(&root, "loose", &[("stream/loose.ydr", b"1")]);
    resource(&root, "[maps]/harbour", &[("stream/harbour.ydr", b"2")]);
    resource(&root, "[maps]/[night]/dock", &[("stream/dock.ydr", b"3")]);
    // A resource under a plain folder is found by no server, and neither is a hidden one.
    resource(&root, "plain/inner", &[("stream/inner.ydr", b"4")]);
    resource(&root, ".hidden", &[("stream/hidden.ydr", b"5")]);
    write(&root.join("[maps]/readme.txt"), b"not a folder");
    // A folder in brackets is a category even with a manifest in it: its resources are found, and it is none itself.
    write(&root.join("[maps]/fxmanifest.lua"), MANIFEST);
    write(&root.join("[maps]/empty/notes.txt"), b"no manifest");
    // The older manifest name is a manifest too.
    write(
        &root.join("old/__resource.lua"),
        b"resource_manifest_version '44febabe'\n",
    );

    let read = scan(&root);
    assert_eq!(
        paths(&read),
        ["[maps]/[night]/dock", "[maps]/harbour", "loose", "old"]
    );
    assert_eq!(read.resources[0].name, "dock");
    assert_eq!(read.resources[3].manifest, "__resource.lua");
    assert!(read.resources[0].map, "its manifest declares it a map");
    assert!(!read.resources[3].map, "its manifest declares no map");
    assert_eq!(read.ignored, ["[maps]/empty", "plain"]);
    assert_eq!(read.unlisted, []);
    assert!(read.is_whole());
    assert_eq!(findings(&read, None, None), []);

    let report = Report::of(&read, None, None);
    assert!(
        (report.lines().iter()).any(|line| line.contains("2 folder(s) are neither a category")
            && line.contains("`[maps]/empty`, `plain`")),
        "{:#?}",
        report.lines()
    );
}

#[test]
fn a_resource_whose_manifest_does_not_read_is_kept_with_its_note_and_the_scan_goes_on() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("resources");
    resource(&root, "broken", &[("stream/x.ydr", b"one")]);
    resource(&root, "odd", &[("stream/y.ydr", b"only here")]);
    write(
        &root.join("odd/fxmanifest.lua"),
        b"fx_version 'cerulean'\nlocal maps = { 'a' }\nthis_is_a_map 'yes'\n",
    );
    resource(&root, "sound", &[("stream/x.ydr", b"two")]);
    let disk = Counting {
        refused: vec![root.join("broken/fxmanifest.lua")],
        ..Counting::default()
    };

    let read = scan_with(&root, &disk);
    assert_eq!(paths(&read), ["broken", "odd", "sound"]);
    let broken = &read.resources[0];
    assert_eq!(broken.notes.len(), 1, "{:#?}", broken.notes);
    assert!(
        broken.notes[0].contains("`fxmanifest.lua` cannot be read")
            && broken.notes[0].contains("refused by the test"),
        "{}",
        broken.notes[0]
    );
    assert!(
        !broken.map,
        "nothing a manifest that did not read states is known"
    );
    assert_eq!(broken.streamed.len(), 1, "its files are still read");
    let odd = &read.resources[1];
    assert!(odd.map);
    assert!(
        odd.notes.len() == 1 && odd.notes[0].contains("1 run(s) of lines"),
        "{:#?}",
        odd.notes
    );
    assert_eq!(read.resources[2].notes, Vec::<String>::new());

    let found = findings(&read, None, None);
    let unread = of_code(&found, "resource-unread");
    assert_eq!(unread.len(), 2, "{found:#?}");
    assert!(unread.iter().all(|f| f.severity == Severity::Info));
    assert_eq!(unread[0].resources, ["broken"]);
    assert_eq!(unread[1].resources, ["odd"]);
    // The scan went on: what the resource streams is still held against the others.
    let [clash] = of_code(&found, "file-in-two-resources")[..] else {
        panic!("{found:#?}");
    };
    assert_eq!(clash.resources, ["broken", "sound"]);
}

#[test]
fn a_locked_file_is_counted_digested_and_handed_to_no_reader() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("resources");
    let mut locked = ESCROW_MARKER.to_vec();
    locked.extend_from_slice(REFUSED);
    resource(
        &root,
        "escrowed",
        &[
            ("stream/props.ytyp", &locked),
            ("stream/yard.ymap", &locked),
            ("stream/crate.ydr", &locked),
            ("stream/open.ydr", b"not locked"),
        ],
    );
    // The same bytes with no marker are what a reader refuses, which is what the marker spares the locked ones.
    resource(&root, "plain", &[("stream/broken.ytyp", REFUSED)]);

    let read = scan(&root);
    let escrowed = &read.resources[0];
    assert_eq!(escrowed.locked(), 3);
    assert_eq!(escrowed.streamed.len(), 4);
    assert!(escrowed.names_incomplete);
    assert_eq!(escrowed.archetypes, []);
    assert_eq!(
        escrowed.notes,
        ["2 type file(s) or map(s) are locked by an asset escrow and not opened, so the names they state are not known"]
    );
    let kept = (escrowed.streamed.iter())
        .find(|f| f.name == "props.ytyp")
        .expect("the locked type file");
    assert!(kept.locked);
    assert_eq!(kept.bytes, locked.len() as u64);
    let digest = format!("{:x}", Sha256::digest(&locked));
    assert_eq!(kept.digest.as_deref(), Some(digest.as_str()));

    let plain = &read.resources[1];
    assert_eq!(plain.locked(), 0);
    assert!(plain.names_incomplete);
    assert_eq!(
        plain.notes,
        ["`stream/broken.ytyp` is not metadata in a binary form, so the names it states are not known"]
    );
    assert!(!read.is_whole());

    // However many files a reader refuses, a resource says so once.
    resource(
        &root,
        "plain",
        &[
            ("stream/also.ymap", REFUSED),
            ("stream/third.ymap", REFUSED),
        ],
    );
    let again = scan(&root);
    assert_eq!(
        again.resources[1].notes,
        ["3 type files or maps do not open, so the names they state are not known; the first, `stream/also.ymap`, is not metadata in a binary form"]
    );

    let report = Report::of(&read, None, None);
    assert_eq!(report.resources[0].locked, 3);
    assert!(
        report.lines()[1].contains("3 locked by an asset escrow and not opened"),
        "{}",
        report.lines()[1]
    );
    assert!(
        !report.fails(),
        "a locked file is a note: {:#?}",
        report.findings
    );
}

#[test]
fn two_resources_that_declare_one_archetype_are_named() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("resources");
    let yard = types_file("yard", &["yard_crate", "yard_barrel"]);
    resource(
        &root,
        "a",
        &[
            ("stream/yard.ytyp", &yard),
            ("stream/a.ymap", &map_file("a_map")),
        ],
    );
    resource(
        &root,
        "b",
        &[(
            "stream/dock.ytyp",
            &types_file("dock", &["yard_crate", "dock_lamp"]),
        )],
    );
    // The same type file under the same name is one copy of a file, said once and not once for each archetype.
    resource(&root, "c", &[("stream/yard.ytyp", &yard)]);
    // What one resource holds twice is the check of one resource's to say, and no finding between two.
    resource(
        &root,
        "d",
        &[
            ("stream/d.ytyp", &types_file("d", &["d_only"])),
            (
                "stream/more/d_more.ytyp",
                &types_file("d_more", &["d_only"]),
            ),
            ("stream/one/d_twice.ydr", b"one"),
            ("stream/two/d_twice.ydr", b"two"),
        ],
    );

    let read = scan(&root);
    assert!(read.is_whole(), "{:#?}", read.resources);
    assert_eq!(read.resources[3].archetypes.len(), 2);
    let declared: Vec<&str> = (read.resources[0].archetypes.iter())
        .map(|a| a.name.as_str())
        .collect();
    assert_eq!(declared, [literal("yard_crate"), literal("yard_barrel")]);
    assert_eq!(read.resources[0].archetypes[0].file, "stream/yard.ytyp");
    let maps: Vec<&str> = read.resources[0]
        .maps
        .iter()
        .map(|m| m.name.as_str())
        .collect();
    assert_eq!(maps, [literal("a_map")]);

    let found = findings(&read, None, None);
    let [twice] = of_code(&found, "archetype-in-two-resources")[..] else {
        panic!("one finding for the crate: {found:#?}");
    };
    assert_eq!(twice.severity, Severity::Warning);
    assert_eq!(twice.resources, ["a", "b", "c"]);
    assert_eq!(twice.names, [literal("yard_crate")]);
    assert!(
        twice
            .message
            .contains("`a`, `b` and `c` declare the archetype"),
        "{}",
        twice.message
    );
    let [same] = of_code(&found, "file-in-two-resources-same")[..] else {
        panic!("one finding for the type file: {found:#?}");
    };
    assert_eq!(
        (same.resources.as_slice(), same.names.as_slice()),
        (
            &["a".to_string(), "c".to_string()][..],
            &["yard.ytyp".to_string()][..]
        )
    );
    assert_eq!(found.len(), 2, "{found:#?}");
}

/// An install that holds the files named.
struct Holds(&'static [&'static str]);

impl GameFiles for Holds {
    fn holds_file(&self, name: &str) -> bool {
        self.0.contains(&name)
    }
}

#[test]
fn a_file_of_the_name_of_a_game_file_is_an_override_and_no_fault() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("resources");
    resource(
        &root,
        "a",
        &[
            ("stream/Hei_Dock.ymap", &map_file("hei_dock")),
            ("stream/new.ydr", b"1"),
        ],
    );
    resource(&root, "b", &[("stream/other.ydr", b"2")]);
    let read = scan(&root);

    assert_eq!(
        findings(&read, None, None),
        [],
        "with no install nothing is asked"
    );
    let game = Holds(&["hei_dock.ymap", "unrelated.ydr"]);
    let report = Report::of(&read, None, Some(&game));
    let [over] = &report.findings[..] else {
        panic!("{:#?}", report.findings);
    };
    assert_eq!(over.code, "game-file-overridden");
    assert_eq!(over.severity, Severity::Info);
    assert_eq!(over.resources, ["a"]);
    assert_eq!(over.names, ["hei_dock.ymap"]);
    assert!(over.message.contains("overrides it"), "{}", over.message);
    assert!(!report.fails());

    let game = Holds(&["hei_dock.ymap", "new.ydr", "other.ydr"]);
    let found = findings(&read, None, Some(&game));
    let [several, one] = &found[..] else {
        panic!("one finding for each resource: {found:#?}");
    };
    assert_eq!(several.names, ["hei_dock.ymap", "new.ydr"]);
    assert!(
        several.message.contains(
            "`a` streams 2 files of the names of files of the game, and overrides them: `hei_dock.ymap`, `new.ydr`"
        ),
        "{}",
        several.message
    );
    assert_eq!(
        (one.resources.as_slice(), one.names.as_slice()),
        (&["b".to_string()][..], &["other.ydr".to_string()][..])
    );
}

/// A build of a resource called `resource` that writes `files` and whose one content makes `claims`.
fn plan_of(kind: OutputKind, resource: &str, files: &[&str], claims: &[(&str, &str)]) -> Plan {
    let container = Container {
        modules: vec![Module {
            id: "maps".to_string(),
            kind: ModuleKind::Unknown("widgets".into()),
            extra: Default::default(),
        }],
        outputs: vec![Output::of(kind, "build", resource, &["maps"])],
    };
    let mut contribution = Contribution::of("maps");
    contribution.files = (files.iter())
        .map(|path| PlannedFile {
            path: (*path).to_string(),
            origin: Origin::Generated(b"made by the build".to_vec()),
        })
        .collect();
    contribution.claims = (claims.iter())
        .map(|(kind, name)| Claim {
            kind: (*kind).to_string(),
            name: (*name).to_string(),
        })
        .collect();
    plan_with(
        &container,
        "build",
        Path::new("/projects/mine"),
        &[contribution],
    )
    .expect("the build is declared")
}

#[test]
fn a_build_is_told_only_what_concerns_the_files_and_the_archetypes_it_writes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("resources");
    resource(
        &root,
        "[maps]/theirs",
        &[
            ("stream/Harbour.ymap", &map_file("harbour")),
            (
                "stream/theirs.ytyp",
                &types_file("theirs", &["yard_crate", "their_lamp"]),
            ),
            ("stream/both.ytd", b"theirs"),
            ("stream/lamp_post.ydr", b"theirs"),
        ],
    );
    // Two other resources clash between themselves, which is no concern of this build.
    resource(
        &root,
        "other",
        &[("stream/both.ytd", b"other"), ("stream/notes.ydr", b"x")],
    );
    // The build's own earlier copy is the resource it replaces.
    resource(
        &root,
        "mine",
        &[
            ("stream/harbour.ymap", &map_file("harbour")),
            ("stream/mine.ytyp", &types_file("mine", &["yard_crate"])),
        ],
    );
    let read = scan(&root);
    assert!(
        !of_code(&findings(&read, None, None), "file-in-two-resources").is_empty(),
        "the folder holds clashes the build is not told of"
    );

    let files = [
        "stream/harbour.ymap",
        "stream/mine.ytyp",
        "stream/mine_only.ydr",
        // A server resolves a streamed file by its name whatever its case, in the build as in the other resource.
        "stream/props/Lamp_Post.YDR",
        // Not under the stream folder, so no server resolves it by its name.
        "data/both.ytd",
    ];
    let claims = [
        (ARCHETYPE_CLAIM, "yard_crate"),
        (ARCHETYPE_CLAIM, "mine_only"),
        // A name of another kind that another resource's archetype has.
        ("map file", "their_lamp"),
    ];
    let plan = plan_of(OutputKind::FivemResource, "mine", &files, &claims);
    assert!(plan.is_buildable(), "{:#?}", plan.problems);
    assert_eq!(
        plan.claims.len(),
        3,
        "a plan carries the claims of what it takes"
    );

    let found = concerning(&plan, &read);
    let [file, archetype] = &found[..] else {
        panic!("one finding for the map and one for the archetype: {found:#?}");
    };
    assert_eq!(file.code, "build-file-streamed-elsewhere");
    assert_eq!(file.severity, Severity::Warning);
    assert_eq!(file.resources, ["[maps]/theirs"]);
    assert_eq!(file.names, ["harbour.ymap", "lamp_post.ydr"]);
    assert_eq!(archetype.code, "build-archetype-declared-elsewhere");
    assert_eq!(archetype.resources, ["[maps]/theirs"]);
    assert_eq!(archetype.names, ["yard_crate"]);

    // Under another name the earlier copy is another resource.
    let renamed = plan_of(OutputKind::FivemResource, "mine_v2", &files, &claims);
    let renamed = concerning(&renamed, &read);
    let stated: Vec<(&str, &str)> = (renamed.iter())
        .map(|f| (f.code, f.resources[0].as_str()))
        .collect();
    assert_eq!(
        stated,
        [
            ("build-file-streamed-elsewhere", "[maps]/theirs"),
            ("build-file-streamed-elsewhere", "mine"),
            ("build-archetype-declared-elsewhere", "[maps]/theirs"),
            ("build-archetype-declared-elsewhere", "mine"),
        ],
        "in the order of the rules, and of the scan under each"
    );

    // A build that writes none of it is told nothing, and so is a pack, which no server finds by a name.
    let apart = plan_of(
        OutputKind::FivemResource,
        "apart",
        &["stream/apart.ydr"],
        &[],
    );
    assert_eq!(concerning(&apart, &read), []);
    let pack = plan_of(
        OutputKind::SingleplayerDlc,
        "pack",
        &["stream/harbour.ymap"],
        &[],
    );
    assert_eq!(concerning(&pack, &read), []);
}

/// Every file under `dir`, links left out.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let kind = entry.file_type().unwrap();
        if kind.is_dir() {
            found.extend(files_under(&entry.path()));
        } else if kind.is_file() {
            found.push(entry.path());
        }
    }
    found
}

#[test]
fn a_scan_opens_each_file_once_and_a_second_scan_gives_the_same_findings_in_the_same_order() {
    let dir = tempfile::tempdir().unwrap();
    let root = two_that_clash(dir.path());
    let yard = types_file("yard", &["yard_crate"]);
    resource(
        &root,
        "[maps]/c",
        &[
            ("stream/harbour.ymap", &map_file("harbour")),
            ("stream/yard.ytyp", &yard),
            // Read by no scan: a resource's own scripts and data are not streamed.
            ("client.lua", b"print('x')"),
            ("data/notes.meta", b"<x/>"),
        ],
    );
    resource(
        &root,
        "d",
        &[("stream/dock.ytyp", &types_file("dock", &["yard_crate"]))],
    );
    write(
        &dir.path().join("server.cfg"),
        b"ensure a\nexec more.cfg\nexec more.cfg\nensure d\n",
    );
    write(
        &dir.path().join("more.cfg"),
        b"ensure b\nexec server.cfg\nensure [maps]\n",
    );

    let disk = Counting::default();
    let first = scan_with(&root, &disk);
    let config = Config::read(&dir.path().join("server.cfg"), &disk).expect("reads");
    assert_eq!(config.read, ["server.cfg", "more.cfg"]);
    let resources: Vec<&str> = config.starts.iter().map(|s| s.resource.as_str()).collect();
    assert_eq!(resources, ["a", "b", "[maps]", "d"]);

    let mut read = 0;
    for file in files_under(dir.path()) {
        let inside = file
            .strip_prefix(dir.path())
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let streamed = inside.contains("/stream/");
        let wanted = usize::from(
            streamed
                || inside.ends_with(".lua") && !inside.ends_with("client.lua")
                || inside.ends_with(".cfg"),
        );
        assert_eq!(disk.times(&file), wanted, "{inside}");
        read += wanted;
    }
    let opened: usize = disk.opened.borrow().values().sum();
    assert_eq!(
        opened,
        read,
        "nothing else was opened: {:#?}",
        disk.opened.borrow()
    );
    assert!(read >= 12, "the folder built here holds files to read");

    let second = scan(&root);
    assert_eq!(first, second);
    let found = findings(&first, Some(&config), None);
    assert_eq!(found, findings(&second, Some(&config), None));
    let codes: Vec<&str> = found.iter().map(|f| f.code).collect();
    assert_eq!(
        codes,
        [
            "file-in-two-resources",
            "file-in-two-resources-same",
            "archetype-in-two-resources",
        ],
        "{found:#?}"
    );
    assert_eq!(found[0].resources, ["[maps]/c", "a", "b"]);
    assert_eq!(found[0].starts_last.as_deref(), Some("[maps]/c"));
    assert_eq!(found[2].resources, ["[maps]/c", "d"]);
    assert_eq!(found[2].starts_last.as_deref(), Some("d"));
}

#[test]
fn many_hundreds_of_resources_are_read_in_one_pass_and_named_in_a_sentence_of_bounded_length() {
    const RESOURCES: usize = 400;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("resources");
    for n in 0..RESOURCES {
        let path = format!("[pack{}]/[maps]/r{n:03}", n % 4);
        let own = format!("stream/r{n:03}.ydr");
        resource(
            &root,
            &path,
            &[
                ("stream/common.ytd", b"in every one"),
                (&own, path.as_bytes()),
            ],
        );
    }
    let disk = Counting::default();
    let read = scan_with(&root, &disk);
    assert_eq!(read.resources.len(), RESOURCES);
    assert_eq!(disk.opened.borrow().len(), RESOURCES * 3);
    assert!(disk.opened.borrow().values().all(|times| *times == 1));

    let found = findings(&read, None, None);
    let [common] = &found[..] else {
        panic!(
            "one finding for the one name every resource streams: {}",
            found.len()
        );
    };
    assert_eq!(common.code, "file-in-two-resources-same");
    assert_eq!(
        common.resources.len(),
        RESOURCES,
        "the data names every resource"
    );
    assert!(
        common
            .message
            .contains(&format!("and {} more", RESOURCES - 8))
            && common.message.len() < 400,
        "the sentence names the first of them and counts the rest: {}",
        common.message
    );
}

#[test]
fn a_path_of_a_configuration_that_leaves_the_server_folder_is_reported_and_not_followed() {
    let dir = tempfile::tempdir().unwrap();
    let server = dir.path().join("server");
    let root = two_that_clash(&server);
    let outside = dir.path().join("outside.cfg");
    write(&outside, b"ensure b\n");
    let absolute = outside.to_string_lossy().replace('\\', "/");
    let cfg = server.join("server.cfg");
    write(
        &cfg,
        format!("ensure a\nexec ../outside.cfg\nexec \"{absolute}\"\nexec cfg/../../outside.cfg\nexec gone.cfg\nexec @a/x.cfg\n").as_bytes(),
    );

    let disk = Counting::default();
    let config = Config::read(&cfg, &disk).expect("the configuration reads");
    assert_eq!(disk.times(&outside), 0, "the file outside was never opened");
    assert_eq!(config.read, ["server.cfg"]);
    let started: Vec<&str> = config.starts.iter().map(|s| s.resource.as_str()).collect();
    assert_eq!(started, ["a"], "nothing the file outside starts is taken");
    assert!(!config.is_whole());
    let why: Vec<(usize, bool)> = (config.includes_unread.iter())
        .map(|include| (include.line, include.why == NotRead::Escapes))
        .collect();
    assert_eq!(
        why,
        [(2, true), (3, true), (4, true), (5, false), (6, false)]
    );

    let read = scan(&root);
    let report = Report::of(&read, Some(&config), None);
    let escapes = of_code(&report.findings, "configuration-path-escapes");
    assert_eq!(escapes.len(), 3, "{:#?}", report.findings);
    assert!(escapes.iter().all(|f| f.severity == Severity::Warning));
    assert_eq!(escapes[0].names, ["../outside.cfg"]);
    assert!(
        escapes[0].message.contains("line 2 of `server.cfg`")
            && escapes[0]
                .message
                .contains("leaves the server's folder and is not followed"),
        "{}",
        escapes[0].message
    );
    assert_eq!(
        of_code(&report.findings, "configuration-file-unread").len(),
        2
    );
    assert!(report.fails());
    // With a file of the configuration unread, nothing is said of the order.
    let [clash] = of_code(&report.findings, "file-in-two-resources")[..] else {
        panic!("{:#?}", report.findings);
    };
    assert_eq!(clash.starts_last, None);
    assert!(clash
        .message
        .contains("a file the configuration includes was not read"));

    assert!(Config::read(&server.join("absent.cfg"), &Disk).is_err());
}

#[cfg(unix)]
#[test]
fn a_link_is_followed_by_neither_the_scan_nor_the_configuration() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let server = dir.path().join("server");
    let root = server.join("resources");
    resource(&root, "real", &[("stream/x.ydr", b"one")]);
    // A resource and a category outside the server's folder, reached only through links.
    let outside = dir.path().join("outside");
    resource(&outside, "elsewhere", &[("stream/x.ydr", b"two")]);
    resource(&outside, "[more]/inner", &[("stream/x.ydr", b"three")]);
    symlink(outside.join("elsewhere"), root.join("linked")).unwrap();
    // A folder whose manifest is a link is no resource, and a stream folder that is a link streams nothing.
    std::fs::create_dir_all(root.join("manifest_linked")).unwrap();
    symlink(
        outside.join("elsewhere/fxmanifest.lua"),
        root.join("manifest_linked/fxmanifest.lua"),
    )
    .unwrap();
    resource(&root, "stream_linked", &[]);
    symlink(
        outside.join("elsewhere/stream"),
        root.join("stream_linked/stream"),
    )
    .unwrap();
    symlink(outside.join("[more]"), root.join("[more]")).unwrap();
    write(&outside.join("more.cfg"), b"ensure real\n");
    symlink(outside.join("more.cfg"), server.join("more.cfg")).unwrap();
    symlink(&outside, server.join("cfg")).unwrap();
    write(
        &server.join("server.cfg"),
        b"exec more.cfg\nexec cfg/more.cfg\n",
    );

    let disk = Counting::default();
    let read = scan_with(&root, &disk);
    assert_eq!(paths(&read), ["real", "stream_linked"]);
    assert_eq!(read.ignored, ["manifest_linked"]);
    assert_eq!(read.resources[1].streamed, []);
    assert_eq!(findings(&read, None, None), []);
    let config = Config::read(&server.join("server.cfg"), &disk).expect("reads");
    assert_eq!(config.starts, []);
    assert_eq!(config.includes_unread.len(), 2);
    assert!(
        (config.includes_unread.iter())
            .all(|i| matches!(&i.why, NotRead::Unreadable(why) if why.contains("a link"))),
        "{:#?}",
        config.includes_unread
    );
    let opened_outside = (disk.opened.borrow().keys())
        .filter(|path| path.to_string_lossy().contains("more.cfg") || path.starts_with(&outside))
        .count();
    assert_eq!(opened_outside, 0, "{:#?}", disk.opened.borrow());
}

#[test]
fn a_line_a_configuration_reader_does_not_know_is_a_note_and_never_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let root = two_that_clash(dir.path());
    let cfg = dir.path().join("server.cfg");
    write(
        &cfg,
        b"\xef\xbb\xbfendpoint_add_tcp \"0.0.0.0:30120\"\nsv_maxclients 48\n\n# resources\nENSURE a\nensure\nstart a b\nset mysql \"x; y\"\nensure b\n\"unclosed\n",
    );
    let config = Config::read(&cfg, &Disk).expect("nothing in a configuration is refused");
    let started: Vec<(&str, usize)> = (config.starts.iter())
        .map(|s| (s.resource.as_str(), s.line))
        .collect();
    assert_eq!(started, [("a", 5), ("b", 9)]);
    let runs: Vec<(usize, usize)> = config
        .unread
        .iter()
        .map(|run| (run.first, run.last))
        .collect();
    assert_eq!(runs, [(1, 2), (6, 8), (10, 10)]);
    assert!(config.is_whole());

    let report = Report::of(&scan(&root), Some(&config), None);
    let [note] = of_code(&report.findings, "configuration-line-unread")[..] else {
        panic!("{:#?}", report.findings);
    };
    assert_eq!(note.severity, Severity::Info);
    assert!(
        note.message
            .contains("`server.cfg`: line(s) 1 to 2, 6 to 8, 10 neither start"),
        "{}",
        note.message
    );
    assert!(report
        .lines()
        .iter()
        .any(|line| line.contains("2 line(s) start a resource or a category")));

    // The mark some editors put before the first byte hides no first line, and an unread line is said of the file it is in.
    write(
        &cfg,
        b"\xef\xbb\xbfensure a\nexec more.cfg\nsv_maxclients 48\n",
    );
    write(
        &dir.path().join("more.cfg"),
        b"sv_scriptHookAllowed 0\nensure b\n",
    );
    let config = Config::read(&cfg, &Disk).expect("reads");
    let started: Vec<(&str, &str, usize)> = (config.starts.iter())
        .map(|s| (s.resource.as_str(), s.file.as_str(), s.line))
        .collect();
    assert_eq!(started, [("a", "server.cfg", 1), ("b", "more.cfg", 2)]);
    let found = findings(&scan(&root), Some(&config), None);
    let notes: Vec<&str> = (of_code(&found, "configuration-line-unread").iter())
        .map(|f| f.message.split(" neither").next().unwrap_or_default())
        .collect();
    assert_eq!(notes, ["`server.cfg`: line(s) 3", "`more.cfg`: line(s) 1"]);
}

#[test]
fn the_json_states_the_findings_the_text_states_in_the_same_order() {
    let dir = tempfile::tempdir().unwrap();
    let server = dir.path().join("server");
    let root = two_that_clash(&server);
    resource(
        &root,
        "c",
        &[
            ("stream/c.ytyp", &types_file("c", &["yard_crate"])),
            ("stream/broken.ymap", REFUSED),
        ],
    );
    resource(
        &root,
        "d",
        &[("stream/d.ytyp", &types_file("d", &["yard_crate"]))],
    );
    write(
        &server.join("server.cfg"),
        b"sv_maxclients 48\nensure a\nensure b\nexec ../x.cfg\n",
    );
    let config = Config::read(&server.join("server.cfg"), &Disk).expect("reads");
    let game = Holds(&["shared.ytd"]);
    let report = Report::of(&scan(&root), Some(&config), Some(&game));

    let codes: Vec<&str> = report.findings.iter().map(|f| f.code).collect();
    assert_eq!(
        codes,
        [
            "resource-unread",
            "file-in-two-resources",
            "file-in-two-resources-same",
            "archetype-in-two-resources",
            "game-file-overridden",
            "game-file-overridden",
            "configuration-path-escapes",
            "configuration-line-unread",
        ],
        "in the order of the rules: {:#?}",
        report.findings
    );

    let lines = report.lines();
    let document = serde_json::to_value(&report).expect("the report serialises");
    let stated = document["findings"].as_array().expect("the findings");
    assert_eq!(stated.len(), report.findings.len());
    // The lines between what was read and the counts are the findings, one each.
    let body = &lines[3..lines.len() - 2];
    assert_eq!(body.len(), stated.len(), "{lines:#?}");
    for (line, finding) in body.iter().zip(stated) {
        let code = finding["code"].as_str().expect("a code");
        let message = finding["message"].as_str().expect("a sentence");
        let severity = match finding["severity"].as_str().expect("a severity") {
            "info" => "note",
            other => other,
        };
        assert!(
            line.contains(code)
                && line.ends_with(message)
                && line.trim_start().starts_with(severity),
            "{line}\n{finding:#}"
        );
        assert!(finding["resources"].is_array() && finding["names"].is_array());
    }
    assert_eq!(
        lines[lines.len() - 2],
        format!(
            "{} error(s), {} warning(s), {} note(s)",
            report.count(Severity::Error),
            report.count(Severity::Warning),
            report.count(Severity::Info)
        )
    );
    assert_eq!(document["resources"].as_array().map(Vec::len), Some(4));
    assert_eq!(
        document["resources"][2]["notes"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(document["starts"], 2);
}

#[test]
fn a_streamed_file_that_cannot_be_read_is_noted_and_is_never_said_to_be_the_same_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("resources");
    resource(
        &root,
        "a",
        &[
            ("stream/a.ytyp", &types_file("a", &["a_crate"])),
            ("stream/x.ydr", b"one"),
        ],
    );
    resource(&root, "b", &[("stream/x.ydr", b"one")]);
    let disk = Counting {
        refused: vec![
            root.join("a/stream/a.ytyp"),
            root.join("a/stream/x.ydr"),
            root.join("b/stream/x.ydr"),
        ],
        ..Counting::default()
    };

    let read = scan_with(&root, &disk);
    let a = &read.resources[0];
    assert_eq!(
        a.streamed.len(),
        2,
        "a file that cannot be read is still one it streams"
    );
    assert!(a
        .streamed
        .iter()
        .all(|f| f.digest.is_none() && f.bytes == 0));
    assert_eq!(
        a.notes,
        [
            "`stream/a.ytyp` cannot be read, so its bytes are not compared: refused by the test",
            "`stream/x.ydr` cannot be read, so its bytes are not compared: refused by the test",
        ]
    );
    assert!(
        a.names_incomplete,
        "a type file that was not read may declare anything"
    );
    assert!(!read.resources[1].names_incomplete);
    assert!(!read.is_whole());

    // Neither copy has a digest, and two that were not read are not said to be the same bytes.
    let found = findings(&read, None, None);
    let [clash] = of_code(&found, "file-in-two-resources")[..] else {
        panic!("{found:#?}");
    };
    assert_eq!(clash.names, ["x.ydr"]);
    assert!(of_code(&found, "file-in-two-resources-same").is_empty());
    assert!(
        clash.message.contains("of which a copy was not read")
            && !clash.message.contains("different bytes"),
        "what was not read is not said to differ: {}",
        clash.message
    );
    assert_eq!(of_code(&found, "resource-unread").len(), 3, "{found:#?}");
}

#[cfg(unix)]
#[test]
fn a_folder_that_cannot_be_listed_is_a_warning_and_the_scan_is_not_whole() {
    use std::fs::{set_permissions, Permissions};
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("resources");
    resource(&root, "[shut]/inside", &[("stream/x.ydr", b"1")]);
    resource(
        &root,
        "open",
        &[
            ("stream/x.ydr", b"2"),
            ("stream/shut/y.ytyp", &types_file("y", &["y_crate"])),
        ],
    );
    let shut = [root.join("[shut]"), root.join("open/stream/shut")];
    for folder in &shut {
        set_permissions(folder, Permissions::from_mode(0o000)).unwrap();
    }
    // An administrator lists any folder, and then nothing here is shut.
    let listed_anyway = std::fs::read_dir(&shut[0]).is_ok();
    let read = scan(&root);
    for folder in &shut {
        set_permissions(folder, Permissions::from_mode(0o755)).unwrap();
    }
    if listed_anyway {
        return;
    }

    assert_eq!(paths(&read), ["open"]);
    let unlisted: Vec<&str> = read.unlisted.iter().map(|u| u.path.as_str()).collect();
    assert_eq!(unlisted, ["[shut]"]);
    assert!(!read.is_whole());
    let open = &read.resources[0];
    assert!(
        open.names_incomplete,
        "a folder of streamed files that was not listed may hold a type file"
    );
    assert!(
        matches!(&open.notes[..], [note] if note.starts_with("`stream/shut` cannot be listed, so the files in it are not known")),
        "{:#?}",
        open.notes
    );

    let report = Report::of(&read, None, None);
    let [folder] = of_code(&report.findings, "folder-unread")[..] else {
        panic!("{:#?}", report.findings);
    };
    assert_eq!(folder.severity, Severity::Warning);
    assert_eq!(folder.names, ["[shut]"]);
    assert!(
        folder
            .message
            .starts_with("`[shut]` cannot be listed, so the resources in it are not known"),
        "{}",
        folder.message
    );
    assert_eq!(of_code(&report.findings, "resource-unread").len(), 1);
    assert!(report.fails(), "what was not read is not a pass");
}

#[test]
fn two_folders_of_one_resource_name_are_named_and_a_line_that_starts_the_name_orders_neither() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("resources");
    resource(&root, "[a]/harbour", &[("stream/x.ydr", b"one")]);
    resource(&root, "[b]/harbour", &[("stream/x.ydr", b"two")]);
    resource(&root, "yard", &[("stream/y.ydr", b"three")]);
    let read = scan(&root);
    let cfg = dir.path().join("server.cfg");
    write(&cfg, b"ensure yard\nensure harbour\n");
    let config = Config::read(&cfg, &Disk).expect("reads");

    let found = findings(&read, Some(&config), None);
    let [twice] = of_code(&found, "resource-name-twice")[..] else {
        panic!("one finding for the name: {found:#?}");
    };
    assert_eq!(twice.severity, Severity::Warning);
    assert_eq!(twice.resources, ["[a]/harbour", "[b]/harbour"]);
    assert_eq!(twice.names, ["harbour"]);
    assert!(
        twice.message.contains(
            "`[a]/harbour` and `[b]/harbour` are 2 folders of one resource name, `harbour`"
        ),
        "{}",
        twice.message
    );

    // One line starts the name, so it says nothing of which folder starts last, and they are no category.
    let [clash] = of_code(&found, "file-in-two-resources")[..] else {
        panic!("{found:#?}");
    };
    assert_eq!(clash.starts_last, None);
    assert!(
        clash.message.contains(
            "`[a]/harbour` and `[b]/harbour` have one name, which line 2 of `server.cfg` starts"
        ) && !clash.message.contains("category"),
        "{}",
        clash.message
    );
    assert_eq!(found.len(), 2, "{found:#?}");

    // A file of a resource is read in neither folder while two have the name.
    write(&root.join("[a]/harbour/more.cfg"), b"ensure yard\n");
    write(&cfg, b"exec @harbour/more.cfg\n");
    let config = Config::read_beside(&cfg, &Disk, &read).expect("reads");
    assert_eq!(config.starts, []);
    assert!(
        matches!(&config.includes_unread[..], [one] if matches!(&one.why, NotRead::Unreadable(why) if why.contains("2 folders have the resource name `harbour`"))),
        "{:#?}",
        config.includes_unread
    );
    write(&cfg, b"ensure yard\nensure harbour\n");
    let config = Config::read(&cfg, &Disk).expect("reads");

    // A name one folder has is no finding.
    std::fs::remove_dir_all(root.join("[b]")).unwrap();
    assert_eq!(findings(&scan(&root), Some(&config), None), []);
}

#[test]
fn a_file_of_a_resource_that_a_configuration_includes_is_read_inside_that_resource() {
    let dir = tempfile::tempdir().unwrap();
    let server = dir.path().join("server");
    let root = two_that_clash(&server);
    resource(&root, "[tools]/menu", &[]);
    write(&root.join("[tools]/menu/config/starts.cfg"), b"ensure a\n");
    write(&server.join("outside.cfg"), b"ensure a\n");
    let cfg = server.join("server.cfg");
    let read = scan(&root);
    let disk = Counting::default();
    let config_of = |text: &str| {
        write(&cfg, text.as_bytes());
        Config::read_beside(&cfg, &disk, &read).expect("the configuration reads")
    };

    let config =
        config_of("ensure b\nexec @menu/config/starts.cfg\nexec @menu/config/starts.cfg\n");
    assert!(config.is_whole(), "{:#?}", config.includes_unread);
    assert_eq!(config.read, ["server.cfg", "@menu/config/starts.cfg"]);
    assert_eq!(
        disk.times(&root.join("[tools]/menu/config/starts.cfg")),
        1,
        "included twice and read once"
    );
    let started: Vec<(&str, &str, usize)> = (config.starts.iter())
        .map(|s| (s.resource.as_str(), s.file.as_str(), s.line))
        .collect();
    assert_eq!(
        started,
        [("b", "server.cfg", 1), ("a", "@menu/config/starts.cfg", 1)]
    );
    let found = findings(&read, Some(&config), None);
    let [clash] = of_code(&found, "file-in-two-resources")[..] else {
        panic!("{found:#?}");
    };
    assert_eq!(clash.starts_last.as_deref(), Some("a"));
    assert!(
        clash
            .message
            .contains("`a` starts last, on line 1 of `@menu/config/starts.cfg`"),
        "{}",
        clash.message
    );

    // A path that climbs out of the resource is not followed, and a resource that is not there has no file to read.
    let config = config_of(
        "exec @menu/../../../outside.cfg\nexec @absent/x.cfg\nexec @menu/config/gone.cfg\nexec @menu\n",
    );
    assert_eq!(disk.times(&server.join("outside.cfg")), 0);
    assert_eq!(config.starts, []);
    let why: Vec<(usize, bool)> = (config.includes_unread.iter())
        .map(|include| (include.line, include.why == NotRead::Escapes))
        .collect();
    assert_eq!(why, [(1, true), (2, false), (3, false), (4, false)]);
    assert!(
        matches!(&config.includes_unread[1].why, NotRead::Unreadable(why) if why.contains("no resource of the name `absent`")),
        "{:#?}",
        config.includes_unread[1]
    );

    // With no resources read beside it, a configuration says so of such a line and follows nothing.
    let alone = Config::read(&cfg, &disk).expect("reads");
    assert!(
        (alone.includes_unread.iter()).all(
            |i| matches!(&i.why, NotRead::Unreadable(why) if why.contains("no resources were read"))
        ) && alone.includes_unread.len() == 4,
        "{:#?}",
        alone.includes_unread
    );
}

#[test]
fn a_build_written_where_a_server_does_not_look_is_warned_of_and_another_folder_of_its_name_is_named(
) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("server/resources");
    resource(&root, "[maps]/harbour", &[("stream/x.ydr", b"earlier")]);
    resource(&root, "yard", &[("stream/y.ydr", b"other")]);
    let read = scan(&root);
    let plan = plan_of(OutputKind::FivemResource, "harbour", &["stream/x.ydr"], &[]);
    let stated = |at: &str| -> Vec<(&'static str, Vec<String>, Vec<String>)> {
        (placement(&plan, &root.join(at), &read).into_iter())
            .map(|f| (f.code, f.resources, f.names))
            .collect()
    };
    let none = Vec::<String>::new;
    let named = |name: &str| vec![name.to_string()];

    // Written over its earlier copy, the build replaces it, and a category at any depth is where a server looks.
    assert_eq!(stated("[maps]/harbour"), []);
    // A plain folder is one a server does not walk, and the earlier copy stays beside the new one.
    let plain = placement(&plan, &root.join("plain/harbour"), &read);
    assert_eq!(
        stated("plain/harbour"),
        [
            ("build-folder-outside-category", none(), named("plain")),
            (
                "build-resource-elsewhere",
                named("[maps]/harbour"),
                named("harbour")
            ),
        ]
    );
    assert!(plain.iter().all(|f| f.severity == Severity::Warning));
    assert!(
        plain[0].message.contains(
            "the build writes `plain/harbour`, and `plain` is no category in brackets, so a server does not look in it"
        ),
        "{}",
        plain[0].message
    );
    assert!(
        plain[1].message.contains(
            "`[maps]/harbour` is another folder of the resource `harbour`, which this build writes at `plain/harbour`"
        ),
        "{}",
        plain[1].message
    );
    // The first folder above it that is no category is the one named.
    assert_eq!(
        stated("[new]/sub/[deep]/harbour")[0],
        ("build-folder-outside-category", none(), named("[new]/sub"))
    );
    assert_eq!(
        stated("[new]/harbour"),
        [(
            "build-resource-elsewhere",
            named("[maps]/harbour"),
            named("harbour")
        )]
    );
    assert_eq!(
        stated("harbour"),
        [(
            "build-resource-elsewhere",
            named("[maps]/harbour"),
            named("harbour")
        )]
    );
    assert_eq!(
        stated("plain/[x]/deep/harbour")[0],
        ("build-folder-outside-category", none(), named("plain"))
    );
    // A path spelled another way is the same place.
    assert_eq!(stated("[maps]/../[maps]/./harbour"), []);
    assert_eq!(stated("[maps]/../plain/./harbour"), stated("plain/harbour"));

    // A build written outside the server's folder is placed nowhere in it, and a pack is no resource.
    assert_eq!(
        placement(&plan, &dir.path().join("project/build/harbour"), &read),
        []
    );
    let pack = plan_of(
        OutputKind::SingleplayerDlc,
        "harbour",
        &["stream/x.ydr"],
        &[],
    );
    assert_eq!(placement(&pack, &root.join("plain/harbour"), &read), []);
}
