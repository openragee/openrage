//! Which archives the index is built from and which packs register each map, over documents
//! and tables made up here.

use or_io::gta6::hash::{name_hash32, name_hash64};
use or_title::gta6_mount::table::Entry;
use or_title::gta6_mount::Document;

use super::*;

fn document(xml: &str) -> Document {
    Document::parse(xml.as_bytes()).expect("a registration document")
}

/// A level pack of the device `device` whose one set registers `files` as archives, and is a set
/// of the level where `level` says so.
pub(in crate::index) fn pack(device: &str, files: &[&str], level: bool) -> Vec<Document> {
    let files: String = (files.iter())
        .map(|f| format!("<Item><filename>{f}</filename><fileType>RPF_FILE</fileType></Item>"))
        .collect();
    let included = if level { "<Item>town</Item>" } else { "" };
    vec![
        document(&format!(
            "<SSetupData><deviceName>{device}</deviceName><nameHash>{device}</nameHash>\
             <type>EXTRACONTENT_LEVEL_PACK</type></SSetupData>"
        )),
        document(&format!(
            "<CDataFileMgr__ContentChangeSetDef><id>town</id><fileList>{files}</fileList>\
             </CDataFileMgr__ContentChangeSetDef>"
        )),
        document(&format!(
            "<CDataFileMgr__ContentDef><includedSets>{included}</includedSets>\
             </CDataFileMgr__ContentDef>"
        )),
    ]
}

fn archive(path: &str, offset: u64) -> Entry {
    Entry {
        word: name_hash64(path.as_bytes()),
        name: None,
        offset,
        stored: 16,
        stated: 16,
        kind: 0,
        ciphered: false,
        chunks: 0,
    }
}

fn every() -> Vec<PathBuf> {
    [
        "common.rpf",
        "levels.rpf",
        "other.rpf",
        "pack/tale/tale.rpf",
    ]
    .map(|name| Path::new("root").join(name))
    .to_vec()
}

/// A base pack in the first archive and a story pack in the last, with the tables `tables` gives.
fn mount(tables: bool) -> Mount {
    let every = every();
    let registry = Registry::of(vec![
        (
            every[0].clone(),
            pack(
                "core",
                &[
                    "platform:/levels/town/docks_metadata.rpf",
                    "platform:/levels/town/docks.rpf",
                ],
                true,
            ),
        ),
        (
            every[3].clone(),
            pack(
                "story_tale",
                &["$(target)/levels/town/pier_metadata.rpf"],
                false,
            ),
        ),
    ]);
    let tables = match tables {
        false => Vec::new(),
        true => vec![
            (
                1,
                Table::of_entries(vec![
                    archive("levels/town/docks_metadata", 0x1000),
                    archive("levels/town/docks", 0x2000),
                ]),
            ),
            (2, Table::of_entries(vec![archive("far/away", 0x1000)])),
            (
                3,
                Table::of_entries(vec![archive("levels/town/pier_metadata", 0x1000)]),
            ),
        ],
    };
    Mount { registry, tables }
}

fn fact(archive: u32, container: Option<u64>, name: &str) -> MapFact {
    MapFact {
        archive,
        container,
        name: name_hash32(name.as_bytes()),
        block: 0,
        flags: 0,
        holds: true,
    }
}

#[test]
fn the_index_is_built_from_the_archives_that_list_a_registered_metadata_archive() {
    let every = every();
    let root = Path::new("root");
    assert_eq!(
        mount(true).archives(root, &every),
        [every[1].clone(), every[3].clone()]
    );
    // With no table, and no folder the level names many archives under, every archive is read.
    assert_eq!(mount(false).archives(root, &every), every);
}

#[test]
fn with_no_table_the_archive_of_a_folder_the_level_names_many_archives_under_is_read() {
    let every = every();
    let names: Vec<String> = (0..NAMED)
        .map(|k| format!("platform:/LEVELS/town/t{k}.rpf"))
        .collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let registry = Registry::of(vec![(every[0].clone(), pack("core", &names, true))]);
    let mount = Mount {
        registry,
        tables: Vec::new(),
    };
    assert_eq!(
        mount.archives(Path::new("root"), &every),
        [every[1].clone()]
    );
    // An archive of that name in another folder is not the level's.
    assert_eq!(mount.archives(Path::new("elsewhere"), &every), every);
}

#[test]
fn the_tables_say_which_packs_register_a_map_and_the_block_maps_are_checked_against_them() {
    let every = every();
    let archives = [every[1].clone(), every[3].clone()];
    let maps = [
        fact(0, Some(0x1000), "docks_metadata"),
        fact(0, Some(0x2000), "a_map"),
        fact(0, Some(0x9000), "a_map_no_set_registers"),
        fact(1, Some(0x1000), "pier"),
        fact(0, None, "loose"),
    ];
    let bound = mount(true).bind(&every, &archives, &maps);
    assert_eq!(bound.modes, [1, 1, 0, 2, 0]);
    assert!(bound.by_table);
    assert_eq!((bound.named_archives, bound.bound_archives), (3, 3));
    // The block maps take the level's archive whole: its two maps no table binds are the difference.
    assert_eq!(bound.disagree, 2);

    // With no table the block maps bind: the level's archive whole, and no pack's map.
    let scanned = mount(false).bind(&every, &archives, &maps);
    assert_eq!(scanned.modes, [1, 1, 1, 0, 1]);
    assert!(!scanned.by_table);
    assert_eq!((scanned.named_archives, scanned.bound_archives), (2, 1));
    assert_eq!(scanned.disagree, 0);
}

#[test]
fn the_level_s_folders_are_counted_from_the_archives_named_on_the_platform_device() {
    let file = |name: &str| RegisteredFile {
        filename: name.to_string(),
        ..Default::default()
    };
    let files = [
        file("platform:/levels/made/up/a_metadata.rpf"),
        file("platform:/LEVELS/made/up/b.rpf"),
        file("platform:/levels/made/up/c.ityp"),
        file("platform:/textures/d.rpf"),
        file("common:/data/levels/e.rpf"),
        file("platform:/loose.rpf"),
    ];
    let refs: Vec<&RegisteredFile> = files.iter().collect();
    let folders = level_folders(&refs);
    assert_eq!(folders.get("levels"), Some(&2));
    assert_eq!(folders.get("textures"), Some(&1));
    assert_eq!(folders.len(), 2);
}
