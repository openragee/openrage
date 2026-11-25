//! The source of a `.ynd`: a path region built by hand goes to a folder and compiles back to its canonical
//! save with its links and junctions, an edit of the document reaches the region with its chains closed,
//! a source taken from the game is the game's file to a build until it is edited, and a document that
//! names a node the region does not hold is refused with the name. Needs no game file.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use or_io::decl::File;
use or_io::gta5::*;
use or_source::asset::{sha256_hex, Source};
use or_source::cook::{cook_source, describe, Cooker, Ignored, Recook};
use or_source::paths::{self, BASE, KIND, RULES};
use or_source::textures::Extraction;
use serde_json::{json, Value};

const NAME: &str = "nodes7";
const REGION: u16 = 7;

fn address(region: u16, index: u16) -> CNodeAddress {
    CNodeAddress {
        region,
        index,
        ..Default::default()
    }
}

fn node(index: u16, num_links: usize) -> CPathNode {
    let mut node = CPathNode {
        address: address(REGION, index),
        street_name_hash: 0x1000_0000 + u32::from(index),
        distance_to_target: -3 - index as i16,
        coors_x: 100 * index as i16 - 50,
        coors_y: -20 * index as i16,
        as_integer1: 0x0040_0000 | (0x0101 << index),
        ..Default::default()
    };
    node.set_as_integer2(0x8000_0401 + u32::from(index));
    node.set_num_links(num_links).unwrap();
    node
}

fn link(region: u16, index: u16, word: u32) -> CPathNodeLink {
    CPathNodeLink {
        other_node: address(region, index),
        as_integer1: word,
        ..Default::default()
    }
}

fn junction(min_x: i16, x_samples: u8, y_samples: u8) -> CPathVirtualJunction {
    CPathVirtualJunction {
        max_z: 10,
        min_x,
        min_y: -min_x,
        height_base_world: 0xFFFE,
        x_samples,
        y_samples,
        ..Default::default()
    }
}

/// Three nodes holding five links, one of them into another region, and two junctions over seven height samples.
fn region() -> CPathRegion {
    CPathRegion {
        first_node: Some(Box::new(PgBasePageMap {
            subtype: 2,
            ..Default::default()
        })),
        nodes: vec![node(0, 2), node(1, 0), node(2, 3)],
        num_nodes_car_nodes: 2,
        num_nodes_ped_nodes: 1,
        links: vec![
            link(REGION, 1, 0x11),
            link(REGION, 2, 0x22),
            link(REGION, 0, 0x33),
            link(REGION + 1, 40, 0x44),
            link(REGION, 1, 0x55),
        ],
        virtual_junctions: vec![junction(12, 3, 1), junction(-8, 2, 2)],
        height_samples: vec![1, 2, 3, 40, 50, 60, 70],
        junction_map: JunctionMapContainer {
            junction_map: AtBinaryMap {
                sorted: 1,
                data: vec![
                    AtBinaryMapDataPair {
                        key: u32::from(REGION),
                        data: 1,
                        ..Default::default()
                    },
                    AtBinaryMapDataPair {
                        key: u32::from(REGION) | (2 << 16),
                        data: 0,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    }
}

/// The file a region is, and what opening and saving that file gives: the canonical save a source owes.
fn file_and_canonical(region: &CPathRegion) -> (Vec<u8>, Vec<u8>) {
    let file = region.to_bytes().expect("the region saves");
    let canonical = CPathRegion::from_bytes(&file).unwrap().to_bytes().unwrap();
    (file, canonical)
}

fn extract(parent: &Path, file: &[u8]) -> PathBuf {
    let how = Extraction {
        from_game: false,
        virtual_path: None,
    };
    paths::extract(file, NAME, parent, &how).unwrap_or_else(|why| panic!("{why:?}"))
}

fn compiled(dir: &Path) -> Vec<u8> {
    paths::compile(&Source::open(dir).unwrap())
        .unwrap_or_else(|why| panic!("{why:?}"))
        .bytes
}

fn refusal(dir: &Path) -> String {
    let problems = paths::compile(&Source::open(dir).unwrap()).expect_err("the source compiled");
    problems
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

fn document(dir: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(dir.join(RULES)).unwrap()).unwrap()
}

fn edit_document(dir: &Path, edit: impl FnOnce(&mut Value)) {
    let mut doc = document(dir);
    edit(&mut doc);
    fs::write(dir.join(RULES), serde_json::to_string_pretty(&doc).unwrap()).unwrap();
}

/// What a region holds that a document states, without what the declaration keeps to itself.
fn stated(region: &CPathRegion) -> Value {
    json!({
        "nodes": region.nodes.iter().map(|n| json!([
            n.address.region, n.address.index, n.street_name_hash, n.distance_to_target,
            n.coors_x, n.coors_y, n.as_integer1, n.as_integer2(), n.num_links(),
        ])).collect::<Vec<_>>(),
        "split": [region.num_nodes_car_nodes, region.num_nodes_ped_nodes],
        "links": region.links.iter().map(|l| json!([
            l.other_node.region, l.other_node.index, l.as_integer1,
        ])).collect::<Vec<_>>(),
        "junctions": region.virtual_junctions.iter().map(|j| json!([
            j.max_z, j.min_x, j.min_y, j.height_base_world, j.x_samples, j.y_samples,
        ])).collect::<Vec<_>>(),
        "height_samples": region.height_samples,
        "map": region.junction_map.junction_map.data.iter().map(|p| json!([p.key, p.data])).collect::<Vec<_>>(),
        "sorted": region.junction_map.junction_map.sorted,
        "page_map": region.first_node.as_deref().map(|p| json!([p.root_virtual_chunk, p.subtype])),
    })
}

#[test]
fn a_path_region_goes_to_its_source_and_cooks_back_to_its_canonical_save_links_and_junctions_included(
) {
    let work = tempfile::tempdir().unwrap();
    let built = region();
    let (file, canonical) = file_and_canonical(&built);
    let dir = extract(work.path(), &file);
    assert_eq!(
        dir.file_name().unwrap().to_string_lossy(),
        format!("{NAME}.{KIND}.src")
    );
    let mut names: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["asset.json", "base.ynd", "paths.json"]);
    assert_eq!(fs::read(dir.join(BASE)).unwrap(), file);
    let manifest = Source::open(&dir).unwrap().manifest;
    assert_eq!(manifest.kind, "paths");
    assert_eq!(manifest.extra["compile"]["base"], sha256_hex(&file));

    let said = describe(&dir).unwrap();
    assert_eq!(
        (said.kind.as_str(), said.extension.as_str()),
        ("paths", "ynd")
    );
    let output = cook_source(&dir, &work.path().join("scratch")).unwrap();
    assert_eq!(output.file_name().unwrap(), "nodes7.ynd");
    let cooked = fs::read(&output).unwrap();
    assert!(
        cooked == canonical,
        "the cook of an untouched source is not the canonical save of its file"
    );
    let back = CPathRegion::from_bytes(&cooked).unwrap();
    assert_eq!(stated(&back), stated(&built));
    assert_eq!((back.links.len(), back.virtual_junctions.len()), (5, 2));
}

#[test]
fn the_document_states_every_node_with_its_links_and_every_junction_with_its_samples() {
    let work = tempfile::tempdir().unwrap();
    let built = region();
    let dir = extract(work.path(), &file_and_canonical(&built).0);
    let doc = document(&dir);
    let hex = |word: u32| format!("{word:#010x}");
    assert_eq!(
        doc["first_node"],
        json!({ "root_virtual_chunk": 0, "subtype": 2 })
    );
    assert_eq!(doc["num_nodes_car_nodes"], 2);
    assert_eq!(doc["num_nodes_ped_nodes"], 1);
    let nodes = doc["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 3);
    assert_eq!(
        nodes[2],
        json!({
            "address": "7:2",
            "street_name_hash": hex(built.nodes[2].street_name_hash),
            "distance_to_target": built.nodes[2].distance_to_target,
            "coors_x": built.nodes[2].coors_x,
            "coors_y": built.nodes[2].coors_y,
            "as_integer1": hex(built.nodes[2].as_integer1),
            "as_integer2": hex(built.nodes[2].as_integer2()),
            "links": [
                { "other_node": "7:0", "as_integer1": hex(0x33) },
                { "other_node": "8:40", "as_integer1": hex(0x44) },
                { "other_node": "7:1", "as_integer1": hex(0x55) },
            ],
        })
    );
    assert_eq!(nodes[0]["links"].as_array().unwrap().len(), 2);
    assert_eq!(nodes[1]["links"], json!([]));
    assert_eq!(
        doc["virtual_junctions"],
        json!([
            { "max_z": 10, "min_x": 12, "min_y": -12, "height_base_world": 0xFFFE, "x_samples": 3, "y_samples": 1, "height_samples": [1, 2, 3] },
            { "max_z": 10, "min_x": -8, "min_y": 8, "height_base_world": 0xFFFE, "x_samples": 2, "y_samples": 2, "height_samples": [40, 50, 60, 70] },
        ])
    );
    assert_eq!(
        doc["junction_map"],
        json!({ "sorted": 1, "data": [{ "key": "7:0", "data": 1 }, { "key": "7:2", "data": 0 }] })
    );
}

#[test]
fn an_edit_of_the_document_reaches_the_region_with_its_chains_closed() {
    let work = tempfile::tempdir().unwrap();
    let (file, canonical) = file_and_canonical(&region());
    let dir = extract(work.path(), &file);
    edit_document(&dir, |doc| {
        // A link out of the node that had none, which moves the first link of every node after it.
        doc["nodes"][1]["links"] = json!([{ "other_node": "7:3", "as_integer1": "0x00000066" }]);
        doc["nodes"][0]["coors_x"] = 1234.into();
        doc["nodes"][2]["as_integer2"] = "0x00000007".into();
        // A fourth node, a pedestrian one.
        let mut added = doc["nodes"][1].clone();
        added["address"] = "7:3".into();
        added["links"] = json!([]);
        doc["nodes"].as_array_mut().unwrap().push(added);
        doc["num_nodes_ped_nodes"] = 2.into();
        // A wider patch under the first junction, which moves the first sample of the second.
        doc["virtual_junctions"][0]["y_samples"] = 2.into();
        doc["virtual_junctions"][0]["height_samples"] = json!([1, 2, 3, 4, 5, 6]);
        doc["junction_map"]["data"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "key": "7:3", "data": 1 }));
    });
    let cooked = compiled(&dir);
    assert_ne!(cooked, canonical);
    let back = CPathRegion::from_bytes(&cooked).unwrap();
    let counts: Vec<usize> = back.nodes.iter().map(CPathNode::num_links).collect();
    assert_eq!(counts, [2, 1, 3, 0]);
    assert_eq!(back.links.len(), 6);
    assert_eq!(
        (back.links[2].other_node.index, back.links[2].as_integer1),
        (3, 0x66),
        "the new link sits after the first node's and before the third's"
    );
    assert_eq!(back.links[3].as_integer1, 0x33);
    assert_eq!(back.nodes[0].coors_x, 1234);
    assert_eq!(back.nodes[2].as_integer2(), 7);
    assert_eq!(back.nodes[3].address.index, 3);
    assert_eq!((back.num_nodes_car_nodes, back.num_nodes_ped_nodes), (2, 2));
    assert_eq!(back.height_samples, [1, 2, 3, 4, 5, 6, 40, 50, 60, 70]);
    assert_eq!(back.virtual_junctions[0].num_height_samples(), 6);
    assert_eq!(back.junction_map.junction_map.data.len(), 3);
    assert_eq!(back.junction_map.junction_map.data[2].address(), (7, 3));
    // What the document did not touch is as the region had it.
    assert_eq!(
        stated(&back)["nodes"][1][2],
        stated(&region())["nodes"][1][2]
    );
}

#[test]
fn a_document_that_names_a_node_the_region_does_not_hold_is_refused_with_the_name() {
    let work = tempfile::tempdir().unwrap();
    let dir = extract(work.path(), &file_and_canonical(&region()).0);
    let pristine = fs::read_to_string(dir.join(RULES)).unwrap();

    edit_document(&dir, |doc| {
        doc["nodes"][0]["links"][1]["other_node"] = "7:9".into()
    });
    let why = refusal(&dir);
    assert_eq!(
        why,
        "paths.json: nodes[0].links[1].other_node names node 7:9, which is not in this region"
    );

    fs::write(dir.join(RULES), &pristine).unwrap();
    edit_document(&dir, |doc| {
        doc["junction_map"]["data"][1]["key"] = "7:12".into()
    });
    let why = refusal(&dir);
    assert_eq!(
        why,
        "paths.json: junction_map.data[1].key names node 7:12, which is not in this region"
    );

    // A node of another region is in another file, so a link to one is taken as it is stated.
    fs::write(dir.join(RULES), &pristine).unwrap();
    edit_document(&dir, |doc| {
        doc["nodes"][0]["links"][1]["other_node"] = "9:900".into()
    });
    let back = CPathRegion::from_bytes(&compiled(&dir)).unwrap();
    assert_eq!(
        (
            back.links[1].other_node.region,
            back.links[1].other_node.index
        ),
        (9, 900)
    );
}

#[test]
fn a_document_the_region_cannot_hold_is_refused_where_it_says_so() {
    let work = tempfile::tempdir().unwrap();
    let dir = extract(work.path(), &file_and_canonical(&region()).0);
    let pristine = fs::read_to_string(dir.join(RULES)).unwrap();
    let refused = |edit: &dyn Fn(&mut Value), place: &str, word: &str| {
        fs::write(dir.join(RULES), &pristine).unwrap();
        edit_document(&dir, edit);
        let why = refusal(&dir);
        assert!(
            why.starts_with(&format!("paths.json: {place}")),
            "{place}: {why}"
        );
        assert!(why.contains(word), "{word}: {why}");
    };
    refused(
        &|doc| {
            let link = doc["nodes"][0]["links"][0].clone();
            doc["nodes"][0]["links"] = Value::Array(vec![link; 32]);
        },
        "nodes[0] ",
        "holds 32 links",
    );
    refused(
        &|doc| doc["virtual_junctions"][1]["height_samples"] = json!([1, 2, 3]),
        "virtual_junctions[1] ",
        "holds 3 height samples",
    );
    refused(
        &|doc| doc["nodes"][1]["speed"] = 3.into(),
        "nodes[1] ",
        "has no member `speed`",
    );
    refused(
        &|doc| doc["nodes"][1]["address"] = "seven".into(),
        "nodes[1].address ",
        "region:index",
    );
    refused(
        &|doc| doc["nodes"][1]["coors_x"] = 70000.into(),
        "nodes[1].coors_x ",
        "70000",
    );
    refused(
        &|doc| {
            doc.as_object_mut().unwrap().remove("virtual_junctions");
        },
        "states no `virtual_junctions`",
        "",
    );
    // What the game would misread is refused by the save, in the document's name.
    refused(&|doc| doc["num_nodes_car_nodes"] = 5.into(), "", "vehicle");
    refused(
        &|doc| doc["junction_map"]["data"][0]["data"] = 9.into(),
        "",
        "junction 9 of 2",
    );
}

#[test]
fn every_value_the_document_states_reaches_the_region_or_is_refused() {
    let work = tempfile::tempdir().unwrap();
    let (file, canonical) = file_and_canonical(&region());
    let dir = extract(work.path(), &file);
    let pristine = document(&dir);
    let places = common::stated_values(&pristine);
    let mut tried = 0usize;
    for place in &places {
        for other in common::other_values(pristine.pointer(place).unwrap()) {
            let mut doc = pristine.clone();
            *doc.pointer_mut(place).unwrap() = other.clone();
            fs::write(dir.join(RULES), serde_json::to_string(&doc).unwrap()).unwrap();
            if let Ok(made) = paths::compile(&Source::open(&dir).unwrap()) {
                assert!(
                    made.bytes != canonical,
                    "{place} states {other} and the compile read nothing of it"
                );
            }
            tried += 1;
        }
    }
    // Eight values of each of three nodes, two of each of five links, six of each of two junctions and its samples.
    assert!(tried >= 3 * 8 + 5 * 2 + 2 * 6 + 7, "{tried} values tried");
    assert_eq!(tried, places.len(), "a value of a region is one number");
}

#[test]
fn a_node_whose_address_is_not_its_place_is_refused_with_both() {
    let work = tempfile::tempdir().unwrap();
    let dir = extract(work.path(), &file_and_canonical(&region()).0);
    let pristine = fs::read_to_string(dir.join(RULES)).unwrap();

    // The middle node goes and nothing is renumbered: `7:2` is still stated, and is now the second node.
    edit_document(&dir, |doc| {
        doc["nodes"].as_array_mut().unwrap().remove(1);
        doc["nodes"][0]["links"][0]["other_node"] = "7:2".into();
        doc["nodes"][1]["links"][2]["other_node"] = "7:0".into();
        doc["num_nodes_car_nodes"] = 1.into();
    });
    let why = refusal(&dir);
    assert_eq!(
        why,
        "paths.json: nodes[1].address is 7:2 and the node is at place 1 of `nodes`: the game finds a node by its place in its region, so the index of an address is that place"
    );

    // Renumbered, the same region compiles, and the link that named the last node names it at its new place.
    edit_document(&dir, |doc| {
        doc["nodes"][1]["address"] = "7:1".into();
        doc["nodes"][0]["links"][0]["other_node"] = "7:1".into();
        doc["nodes"][0]["links"][1]["other_node"] = "7:1".into();
        doc["junction_map"]["data"][1]["key"] = "7:1".into();
    });
    let back = CPathRegion::from_bytes(&compiled(&dir)).unwrap();
    assert_eq!(back.nodes.len(), 2);
    assert_eq!(back.nodes[1].address.index, 1);
    assert_eq!(
        (
            back.links[0].other_node.index,
            back.links[1].other_node.index
        ),
        (1, 1)
    );

    // Every node given another region, and no page map: the file is that region's.
    let pristine_text = pristine.replace("\"7:", "\"9:");
    assert_ne!(pristine_text, pristine);
    fs::write(dir.join(RULES), &pristine_text).unwrap();
    edit_document(&dir, |doc| doc["first_node"] = Value::Null);
    let back = CPathRegion::from_bytes(&compiled(&dir)).unwrap();
    assert!(back.nodes.iter().all(|node| node.address.region == 9));
    assert_eq!(back.links[0].other_node.region, 9);
    assert_eq!(back.junction_map.junction_map.data[1].address(), (9, 2));
    assert!(back.first_node.is_none());

    fs::write(dir.join(RULES), &pristine).unwrap();
    edit_document(&dir, |doc| doc["nodes"][2]["address"] = "8:2".into());
    let why = refusal(&dir);
    assert_eq!(
        why,
        "paths.json: nodes[2].address is 8:2 and nodes[0] is of region 7: a file holds the nodes of one region"
    );
}

#[test]
fn a_word_that_states_the_count_of_a_nodes_links_is_refused_and_never_dropped() {
    let work = tempfile::tempdir().unwrap();
    let dir = extract(work.path(), &file_and_canonical(&region()).0);
    // Bits 11 to 15 of the word count the links, which `links` states.
    let count_of_one = {
        let mut whole = CPathNode::default();
        whole.set_as_integer2(u32::MAX);
        let count = !whole.as_integer2();
        count & count.wrapping_neg()
    };
    edit_document(&dir, |doc| {
        doc["nodes"][1]["as_integer2"] = format!("{:#010x}", 0x0000_0007 | count_of_one).into()
    });
    let why = refusal(&dir);
    assert!(
        why.starts_with("paths.json: nodes[1].as_integer2 ") && why.contains("`links`"),
        "{why}"
    );
}

#[test]
fn an_untouched_source_taken_from_the_game_is_the_games_file_to_a_build_until_it_is_edited() {
    use or_source::game_import::{import, Facts, Input, Options, Verdict};
    let project = tempfile::tempdir().unwrap();
    let (file, _) = file_and_canonical(&region());
    let facts = Facts {
        from_game: true,
        virtual_path: Some("common.rpf/data/levels/gta5/nodes7.ynd".to_string()),
        ..Facts::default()
    };
    let report = import(
        project.path(),
        &[Input::Bytes {
            name: "nodes7.ynd",
            bytes: &file,
            facts,
        }],
        &Options::default(),
    );
    let Verdict::Made { folder } = &report.files[0].verdict else {
        panic!("{:?}: {}", report.files[0].verdict, report.files[0].reason);
    };
    assert_eq!(
        folder,
        &project.path().join("sources").join("nodes7.paths.src")
    );
    let cook = |scratch: &str| {
        let output = cook_source(folder, &project.path().join(scratch)).unwrap();
        sha256_hex(&fs::read(output).unwrap())
    };
    let untouched = cook("first");
    assert!(describe(folder).unwrap().game.contains(&untouched));
    edit_document(folder, |doc| {
        doc["nodes"][0]["as_integer1"] = "0x00400202".into()
    });
    let edited = cook("second");
    assert_ne!(untouched, edited);
    assert!(!describe(folder).unwrap().game.contains(&edited));
}

#[test]
fn a_paths_source_is_recognised_by_its_document_and_recooked_when_one_of_its_files_changes() {
    let work = tempfile::tempdir().unwrap();
    let (file, _) = file_and_canonical(&region());
    let dir = extract(work.path(), &file);
    let mut cooker = Cooker::new(work.path().join("vfs"));
    assert!(matches!(
        cooker.recook(&dir.join(RULES)),
        Recook::Cooked { .. }
    ));
    assert!(matches!(
        cooker.recook(&dir.join(BASE)),
        Recook::Unchanged { .. }
    ));
    assert_eq!(
        cooker.recook(&dir.join("notes.txt")),
        Recook::Ignored(Ignored::NotAnInput)
    );
    // A save of a source nobody edited changes no byte of it.
    let before = fs::read(dir.join(RULES)).unwrap();
    Source::open(&dir).unwrap().save().unwrap();
    assert!(fs::read(dir.join(RULES)).unwrap() == before);
    // An edit of the document is seen by the cooker that cooked it before.
    edit_document(&dir, |doc| doc["nodes"][0]["coors_y"] = 77.into());
    match cooker.recook(&dir.join(RULES)) {
        Recook::Cooked { output, .. } => {
            let back = CPathRegion::from_bytes(&fs::read(output).unwrap()).unwrap();
            assert_eq!(back.nodes[0].coors_y, 77);
        }
        other => panic!("the edit was not cooked: {other:?}"),
    }
    fs::write(dir.join(BASE), &file[..file.len() - 1]).unwrap();
    let why = refusal(&dir);
    assert!(
        why.starts_with("base.ynd: ") && why.contains("compile.base"),
        "{why}"
    );
    fs::remove_file(dir.join(RULES)).unwrap();
    assert_eq!(
        Source::open(&dir).unwrap_err().file.as_deref(),
        Some("paths.json")
    );
}

#[test]
fn a_file_that_is_no_region_is_refused_and_nothing_is_left_behind() {
    let work = tempfile::tempdir().unwrap();
    let how = Extraction {
        from_game: false,
        virtual_path: None,
    };
    assert!(paths::extract(b"nodes", NAME, work.path(), &how).is_err());
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 0);
}
