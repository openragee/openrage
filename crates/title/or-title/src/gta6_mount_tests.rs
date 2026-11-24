//! The registration documents by root element, the registry they make, and the binding of the
//! level's archives to maps, over documents and maps made up here.

use super::*;

fn document(xml: &str) -> Document {
    Document::parse(xml.as_bytes()).expect("a registration document")
}

/// A setup of type `kind`, each of whose groups lists the sets beside its name.
fn setup(name: &str, kind: &str, order: i32, groups: &[(&str, &[&str])]) -> Document {
    let groups: String = (groups.iter())
        .map(|(group, sets)| {
            let sets: String = sets.iter().map(|s| format!("<Item>{s}</Item>")).collect();
            format!(
                "<Item><NameHash>{group}</NameHash><ContentChangeSets>{sets}</ContentChangeSets></Item>"
            )
        })
        .collect();
    document(&format!(
        "<SSetupData><nameHash>{name}</nameHash><type>{kind}</type><order value=\"{order}\"/>\
         <contentChangeSetGroups>{groups}</contentChangeSetGroups></SSetupData>"
    ))
}

const LEVEL: &str = "EXTRACONTENT_LEVEL_PACK";
const COMPAT: &str = "EXTRACONTENT_COMPAT_PACK";

/// The elements of a change set `id` that registers `files`.
fn set_body(id: &str, files: &[&str]) -> String {
    let files: String = (files.iter())
        .map(|f| format!("<Item><filename>{f}</filename><fileType>RPF_FILE</fileType></Item>"))
        .collect();
    format!("<id>{id}</id><flags>FIRST SECOND</flags><fileList>{files}</fileList>")
}

fn change_set(id: &str, files: &[&str]) -> Document {
    let body = set_body(id, files);
    document(&format!(
        "<CDataFileMgr__ContentChangeSetDef>{body}</CDataFileMgr__ContentChangeSetDef>"
    ))
}

/// A content definition of inline sets, of sets included by id and of ones included by load path.
fn content(inline: &[(&str, &[&str])], ids: &[&str], paths: &[&str]) -> Document {
    let inline: String = (inline.iter())
        .map(|(id, files)| format!("<Item>{}</Item>", set_body(id, files)))
        .collect();
    let ids: String = ids.iter().map(|s| format!("<Item>{s}</Item>")).collect();
    let paths: String = (paths.iter())
        .map(|p| format!("<Item><loadPath>{p}</loadPath></Item>"))
        .collect();
    document(&format!(
        "<CDataFileMgr__ContentDef><contentSets>{inline}</contentSets>\
         <includedSets>{ids}</includedSets><includedChangesets>{paths}</includedChangesets>\
         </CDataFileMgr__ContentDef>"
    ))
}

fn one_pack(documents: Vec<Document>) -> Pack {
    let mut registry = Registry::of(vec![("one.rpf".into(), documents)]);
    assert_eq!(registry.packs.len(), 1);
    registry.packs.remove(0)
}

fn ids(pack: &Pack) -> Vec<&str> {
    pack.sets.iter().map(|set| set.id.as_str()).collect()
}

#[test]
fn a_document_is_known_by_its_root_element() {
    assert!(matches!(
        setup("one", LEVEL, 0, &[]),
        Document::Setup(setup) if setup.name_hash == "one"
    ));
    assert!(matches!(
        change_set("town", &["a.rpf"]),
        Document::ChangeSet(set) if set.id == "town" && set.file_list.len() == 1
    ));
    assert!(matches!(
        content(&[], &["town"], &[]),
        Document::Content(content) if content.included_sets.len() == 1
    ));
    let prefaced = "\u{feff}<?xml version=\"1.0\"?>\n<!-- a note -->\n<SSetupData \n/>";
    assert!(matches!(document(prefaced), Document::Setup(_)));
}

#[test]
fn a_body_of_another_root_or_one_that_does_not_parse_is_no_document() {
    for body in [
        "<Unrelated><id>town</id></Unrelated>",
        "<SSetupDataMore />",
        "<SSetupData><nameHash>one</SSetupData>",
        "<!-- a note that never ends <SSetupData />",
        "a line of text",
        "",
    ] {
        assert_eq!(Document::parse(body.as_bytes()), None, "{body}");
    }
}

#[test]
fn a_set_is_included_by_load_path_and_by_set_id() {
    let by_id = "common:/data/levels/harbour/Area_One.meta";
    let pack = one_pack(vec![
        setup("one", LEVEL, 0, &[]),
        change_set("Town_Core", &["a.rpf"]),
        change_set(by_id, &["b.rpf"]),
        content(
            &[],
            &["COMMON:/data/levels/harbour/areaone.meta"],
            &["$(root)/changesets/sub/CCS_towncore.meta"],
        ),
    ]);
    assert_eq!(ids(&pack), ["Town_Core", by_id]);
    // A set of the level is one a definition includes as a set, not one it includes by path.
    let level: Vec<bool> = pack.sets.iter().map(|set| set.level).collect();
    assert_eq!(level, [false, true]);
    assert_eq!(pack.sets[0].flags, ["FIRST", "SECOND"]);
    assert_eq!(pack.sets[1].files[0].filename, "b.rpf");
    assert_eq!(pack.unread, Vec::<String>::new());
}

#[test]
fn an_inline_content_set_is_a_level_set_with_its_own_files() {
    let pack = one_pack(vec![
        setup("one", LEVEL, 0, &[]),
        change_set("HARBOUR_INLINE", &["from_the_body.rpf"]),
        content(&[("harbour_inline", &["from_the_entry.rpf"])], &[], &[]),
    ]);
    assert_eq!(ids(&pack), ["harbour_inline"]);
    assert!(pack.sets[0].level);
    assert_eq!(pack.sets[0].files[0].filename, "from_the_entry.rpf");
}

#[test]
fn a_set_carries_the_groups_that_name_it() {
    let pack = one_pack(vec![
        setup(
            "one",
            LEVEL,
            0,
            &[
                ("GROUP_ONE", &["town_core", "HARBOUR"]),
                ("GROUP_TWO", &["TownCore"]),
            ],
        ),
        change_set("Town_Core", &[]),
        change_set("harbour", &[]),
        change_set("loose", &[]),
    ]);
    let groups: Vec<&[String]> = pack.sets.iter().map(|set| set.groups.as_slice()).collect();
    assert_eq!(groups[0], ["GROUP_ONE", "GROUP_TWO"]);
    assert_eq!(groups[1], ["GROUP_ONE"]);
    assert!(groups[2].is_empty());
    assert!(pack.sets.iter().all(|set| !set.level));
}

#[test]
fn an_id_named_with_no_body_is_unread_once() {
    let pack = one_pack(vec![
        setup("one", LEVEL, 0, &[("GROUP_ONE", &["Ghost_Set", "town"])]),
        change_set("town", &[]),
        content(
            &[],
            &["ghost_set", "missing:/area.meta"],
            &[
                "$(root)/changesets/ccs_Other_One.meta",
                "$(root)/changesets/ccs_town.meta",
            ],
        ),
    ]);
    assert_eq!(ids(&pack), ["town"]);
    assert_eq!(pack.unread, ["ghostset", "missing:/area.meta", "otherone"]);
}

#[test]
fn an_archive_with_no_setup_is_no_pack() {
    let registry = Registry::of(vec![
        (
            "sets_only.rpf".into(),
            vec![change_set("stray", &[]), content(&[], &["stray"], &[])],
        ),
        (
            "pack.rpf".into(),
            vec![change_set("town", &[]), setup("one", COMPAT, 0, &[])],
        ),
    ]);
    assert_eq!(registry.packs.len(), 1);
    assert_eq!(registry.packs[0].archive, Path::new("pack.rpf"));
    assert_eq!(ids(&registry.packs[0]), ["town"]);
}

#[test]
fn packs_mount_in_the_order_of_their_setups() {
    let registry = Registry::of(vec![
        ("c.rpf".into(), vec![setup("late", COMPAT, 7, &[])]),
        ("b.rpf".into(), vec![setup("early", COMPAT, 2, &[])]),
        ("a.rpf".into(), vec![setup("level", LEVEL, 9, &[])]),
        (
            "u.rpf".into(),
            vec![setup("update", "EXTRACONTENT_TU_PACK", 5, &[])],
        ),
    ]);
    let names: Vec<&str> = (registry.packs.iter())
        .map(|pack| pack.setup.name_hash.as_str())
        .collect();
    assert_eq!(names, ["update", "level", "early", "late"]);
    let mut by_setup = registry.packs.clone();
    by_setup.sort_by(|a, b| a.setup.mount_cmp(&b.setup));
    assert_eq!(by_setup, registry.packs);
}

#[test]
fn the_level_pack_is_the_first_level_setup_whose_definitions_include_sets() {
    let by_path = content(&[], &[], &["$(root)/changesets/ccs_town.meta"]);
    let registry = Registry::of(vec![
        (
            "compat.rpf".into(),
            vec![
                setup("compat", COMPAT, 0, &[]),
                change_set("town", &[]),
                content(&[], &["town"], &[]),
            ],
        ),
        (
            "second.rpf".into(),
            vec![
                setup("second", LEVEL, 2, &[]),
                change_set("town", &[]),
                content(&[], &["town"], &[]),
            ],
        ),
        (
            "first.rpf".into(),
            vec![
                setup("first", LEVEL, 1, &[]),
                change_set("town", &[]),
                content(&[], &["town"], &[]),
            ],
        ),
        (
            "paths.rpf".into(),
            vec![
                setup("paths", LEVEL, 0, &[]),
                change_set("town", &[]),
                by_path,
            ],
        ),
    ]);
    let level = registry.level_pack().expect("a level pack");
    assert_eq!(level.setup.name_hash, "first");
    assert_eq!(Registry::default().level_pack(), None);
}

fn rpf(filename: &str) -> RegisteredFile {
    RegisteredFile {
        filename: filename.to_string(),
        ..RegisteredFile::default()
    }
}

/// A map that holds something, named `name`, whose block map is `block`, or none where it is empty.
fn map(archive: u32, container: Option<u64>, name: &str, block: &str) -> MapFact {
    MapFact {
        archive,
        container,
        name: name_hash32(name.as_bytes()),
        block: match block {
            "" => 0,
            block => name_hash32(block.as_bytes()),
        },
        flags: 0,
        holds: true,
    }
}

#[test]
fn a_container_that_holds_a_named_map_is_bound() {
    let files = [
        rpf("platform:/levels/harbour/map/DOCKS_Metadata.RPF"),
        rpf("platform:/levels/harbour/map/docks_metadata.rpf"),
        rpf("platform:/levels/harbour/yard.meta"),
    ];
    let maps = [
        map(0, Some(0x1000), "docks_metadata", ""),
        map(0, Some(0x1000), "pier", "elsewhere"),
        map(0, Some(0x9000), "yard", ""),
        map(1, None, "docks_metadata_more", ""),
    ];
    let binding = bind(&files.iter().collect::<Vec<_>>(), &maps);
    assert_eq!(
        binding.held,
        [None, None, None, Some(HeldBack::NotRegistered)]
    );
    assert_eq!(
        binding.report,
        Report {
            named_archives: 1,
            bound_archives: 1,
            maps: 4,
            registered: 3,
            at_rest: 3,
            script_managed: 0,
            empty: 0,
            level_archive: Some(0),
        }
    );
}

#[test]
fn a_map_whose_block_lies_in_a_bound_container_is_tied() {
    let files = [rpf("docks_metadata.rpf")];
    let maps = [
        map(0, Some(0x10), "docks_metadata", ""),
        map(1, None, "pier_a", "docks_metadata"),
        map(1, None, "pier_b", "docks_metadata"),
        map(1, None, "yard", "another_block"),
    ];
    let binding = bind(&files.iter().collect::<Vec<_>>(), &maps);
    // One map in the bound container, two tied to it by their block in archive 1.
    assert_eq!(binding.report.level_archive, Some(1));
    assert_eq!(
        binding.held,
        [Some(HeldBack::NotRegistered), None, None, None]
    );
    assert_eq!(binding.report.registered, 3);
    assert_eq!(binding.report.bound_archives, 1);
}

#[test]
fn the_level_archive_holds_the_most_tied_maps() {
    let files = [rpf("docks_metadata.rpf"), rpf("yard_metadata.rpf")];
    let files: Vec<&RegisteredFile> = files.iter().collect();
    let mut maps = vec![
        map(3, Some(0x10), "docks_metadata", ""),
        map(1, Some(0x20), "yard_metadata", ""),
    ];
    // Of two archives with one tied map each, the lower index.
    let report = bind(&files, &maps).report;
    assert_eq!(report.level_archive, Some(1));
    assert_eq!((report.named_archives, report.bound_archives), (2, 2));
    maps.push(map(3, Some(0x10), "pier", ""));
    assert_eq!(bind(&files, &maps).report.level_archive, Some(3));
}

#[test]
fn a_registered_map_is_held_back_by_its_flag_then_by_holding_nothing() {
    let files = [rpf("docks_metadata.rpf")];
    let block = map(0, None, "docks_metadata", "");
    let with = |flags, holds| MapFact {
        flags,
        holds,
        ..map(0, None, "pier", "docks_metadata")
    };
    let maps = [
        block,
        with(1, true),
        with(3, false),
        with(2, false),
        with(2, true),
    ];
    let binding = bind(&files.iter().collect::<Vec<_>>(), &maps);
    assert_eq!(
        binding.held,
        [
            None,
            Some(HeldBack::ScriptManaged),
            Some(HeldBack::ScriptManaged),
            Some(HeldBack::Empty),
            None,
        ]
    );
    let report = binding.report;
    assert_eq!(
        (
            report.registered,
            report.at_rest,
            report.script_managed,
            report.empty
        ),
        (5, 2, 2, 1)
    );
}

#[test]
fn with_no_named_archive_no_map_is_registered() {
    let maps = [map(0, None, "docks_metadata", ""), map(0, None, "pier", "")];
    for files in [vec![], vec![rpf("docks_metadata.meta")]] {
        let binding = bind(&files.iter().collect::<Vec<_>>(), &maps);
        assert_eq!(binding.held, [Some(HeldBack::NotRegistered); 2]);
        assert_eq!(
            binding.report,
            Report {
                maps: 2,
                ..Report::default()
            }
        );
    }
}

/// `body` as a zlib stream of one stored block, padded to the slot a scan steps by.
fn stored(body: &[u8]) -> Vec<u8> {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in body {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    let len = body.len() as u16;
    let mut out = vec![0x78, 0xDA, 1];
    out.extend(len.to_le_bytes());
    out.extend((!len).to_le_bytes());
    out.extend(body);
    out.extend(((b << 16) | a).to_be_bytes());
    out.resize(out.len().next_multiple_of(16), 0);
    out
}

/// A file of four bodies: a setup, a document of another root, a change set and plain text.
fn scanned_file(dir: &Path) -> (PathBuf, u64) {
    let set = format!(
        "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <CDataFileMgr__ContentChangeSetDef>{}</CDataFileMgr__ContentChangeSetDef>",
        set_body("town", &["a.rpf"])
    );
    let mut bytes = stored(b"<SSetupData><nameHash>one</nameHash></SSetupData>");
    bytes.extend(stored(b"<Unrelated><id>town</id></Unrelated>"));
    let set_at = bytes.len() as u64;
    bytes.extend(stored(set.as_bytes()));
    bytes.extend(stored(b"a line of text\r\nand one more line of it\r\n"));
    let path = dir.join("made_up.rpf");
    std::fs::write(&path, bytes).expect("write");
    (path, set_at)
}

#[test]
fn a_scan_reads_the_registration_documents_of_a_file() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (path, set_at) = scanned_file(dir.path());
    let found = read_documents(&path, &|| false).expect("read");
    let offsets: Vec<u64> = found.iter().map(|f| f.offset).collect();
    assert_eq!(offsets, [0, set_at]);
    assert!(matches!(&found[0].document, Document::Setup(setup) if setup.name_hash == "one"));
    assert!(matches!(
        &found[1].document,
        Document::ChangeSet(set) if set.id == "town" && set.file_list[0].filename == "a.rpf"
    ));
}

#[test]
fn a_stopped_scan_is_interrupted() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (path, _) = scanned_file(dir.path());
    let stopped = read_documents(&path, &|| true).expect_err("stopped");
    assert_eq!(stopped.kind(), io::ErrorKind::Interrupted);
}
