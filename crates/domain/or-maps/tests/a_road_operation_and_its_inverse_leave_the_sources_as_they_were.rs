//! Every operation of the vocabulary of road edits, followed by the inverse `apply` gives for it, leaves every source of the set byte
//! for byte as it was, and each region an operation leaves cooks, reads back through the owned type and saves to its fixed point. A
//! node another region names is not taken out, one none names is taken out with every link of the set renumbered, and a request the
//! file cannot state is refused by name with nothing changed.
//!
//! ⚠ The regions are built by hand, so the clean checkout runs this.

#[path = "common/roads.rs"]
mod roads;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use or_io::gta5::CPathRegion;
use or_maps::roads::operation::{
    apply, apply_all, Document, Error, JunctionRecord, LinkRecord, NamedBy, NodeRecord, Operation,
    Operations, PairRecord, FORMAT,
};
use or_maps::roads::{Address, Word};
use roads::{cooked, coors, extract, files, links_of, node, region, Junction};

const SEVEN: &str = "nodes7.paths.src";
const EIGHT: &str = "nodes8.paths.src";

fn at(region: u16, index: u16) -> Address {
    Address::new(region, index)
}

/// Region 7: three vehicle nodes and a pedestrian one, two of them with a junction, and two links into region 8.
fn seven() -> CPathRegion {
    region(
        7,
        3,
        &[
            node([0, 0, 10], &[(7, 1, 0x11), (7, 3, 0x12)]),
            node([100, 0, 10], &[(7, 0, 0x21), (8, 1, 0x22)]),
            node([200, 0, 10], &[(7, 3, 0x31)]),
            node([300, 50, 12], &[(7, 0, 0x41), (7, 2, 0x42), (8, 2, 0x43)]),
        ],
        &[
            Junction {
                node: 0,
                min_x: 12,
                patch: (3, 1, vec![1, 2, 3]),
            },
            Junction {
                node: 2,
                min_x: -8,
                patch: (2, 2, vec![40, 50, 60, 70]),
            },
        ],
    )
}

/// Region 8: two vehicle nodes and a pedestrian one, with the links back into region 7.
fn eight() -> CPathRegion {
    region(
        8,
        2,
        &[
            node([1000, 0, 5], &[(8, 1, 0x51)]),
            node([1100, 0, 5], &[(8, 0, 0x61), (7, 1, 0x62)]),
            node([1200, 0, 5], &[(7, 3, 0x71)]),
        ],
        &[],
    )
}

/// A folder holding regions 7 and 8 as sources.
fn set(dir: &Path) -> PathBuf {
    let folder = dir.join("roads");
    extract(&folder, "nodes7", &seven());
    extract(&folder, "nodes8", &eight());
    folder
}

/// A folder holding region 7 alone.
fn seven_alone(dir: &Path) -> PathBuf {
    let folder = dir.join("roads");
    extract(&folder, "nodes7", &seven());
    folder
}

/// Every region of the set reads back through the owned type and saves to its fixed point.
fn every_region_reads_back(folder: &Path) -> Result<(), String> {
    for (name, _) in or_maps::roads::sources(folder).map_err(|e| e.to_string())? {
        cooked(folder, &name)?;
    }
    Ok(())
}

fn refusal(folder: &Path, operation: &Operation) -> String {
    let before = files(folder);
    let refused = apply(folder, operation).expect_err("the operation is refused");
    assert!(files(folder) == before, "a refusal changes no source");
    assert!(
        matches!(&refused, Error::Refused { name, .. } if *name == operation.name()),
        "{refused:?}"
    );
    refused.to_string()
}

/// One case: an operation to try on a fresh set.
type Case = fn(&Path) -> Operation;

/// ⚠ A variant with no case here, that is not one of [`ALWAYS_REFUSED`], fails `every_road_operation_followed_by_its_inverse_leaves_every_source_byte_for_byte_as_it_was`.
const CASES: &[Case] = &[
    // A node with no junction, moved and renamed.
    |_| Operation::SetNode {
        node: at(7, 1),
        street_name_hash: Some(Word(0xDEAD_BEEF)),
        distance_to_target: Some(44),
        coors_x: Some(-120),
        coors_y: Some(7),
        coors_z: Some(-3),
        as_integer1: None,
        as_integer2: Some(Word(0x0000_0007)),
    },
    // The first word whole, which carries the height.
    |_| Operation::SetNode {
        node: at(8, 2),
        street_name_hash: None,
        distance_to_target: None,
        coors_x: None,
        coors_y: None,
        coors_z: None,
        as_integer1: Some(Word(0x0009_BEEF)),
        as_integer2: None,
    },
    |_| Operation::SetLink {
        node: at(7, 3),
        link: 2,
        as_integer1: Word(0xABCD_0123),
    },
    // Across two regions, in the middle of the links of each node.
    |_| Operation::AddLink {
        node: at(7, 0),
        other_node: at(8, 1),
        as_integer1: Word(0x0000_0A0A),
        link: Some(1),
        back_as_integer1: Some(Word(0x0000_0B0B)),
        back_link: Some(0),
    },
    // One way, last among the links of its node.
    |_| Operation::AddLink {
        node: at(8, 0),
        other_node: at(8, 2),
        as_integer1: Word(0x0000_0C0C),
        link: None,
        back_as_integer1: None,
        back_link: None,
    },
    // With the link back, which the other region states.
    |_| Operation::RemoveLink {
        node: at(7, 1),
        link: 1,
        back_link: None,
        one_way: false,
    },
    // The link alone, though its far node links back.
    |_| Operation::RemoveLink {
        node: at(7, 0),
        link: 0,
        back_link: None,
        one_way: true,
    },
    // A vehicle node, which stands before the pedestrian node and renumbers it in both regions.
    |_| Operation::AddNode {
        region: 7,
        ped: false,
        coors_x: 250,
        coors_y: -5,
        coors_z: 11,
        linked_to: at(7, 2),
        as_integer1: Word(0x0000_0D0D),
        back_as_integer1: Word(0x0000_0E0E),
    },
    // A pedestrian node, linked to the pedestrian node before it.
    |_| Operation::AddNode {
        region: 7,
        ped: true,
        coors_x: 350,
        coors_y: 60,
        coors_z: 12,
        linked_to: at(7, 3),
        as_integer1: Word(0x0000_0F0F),
        back_as_integer1: Word(0x0000_1010),
    },
    // The first node, with its junction, the links that name it and every later node renumbered.
    |_| Operation::RemoveNode { node: at(7, 0) },
    // A node whose junction is the last.
    |_| Operation::RemoveNode { node: at(7, 2) },
    // A node put in with a link, a link that names it and a junction of its own.
    |_| Operation::InsertNode {
        node: at(7, 1),
        ped: false,
        record: NodeRecord {
            street_name_hash: Word(0x7777_0001),
            distance_to_target: 9,
            coors_x: 50,
            coors_y: 5,
            as_integer1: Word(0x000A_0303),
            as_integer2: Word(0x0000_0001),
        },
        links: vec![LinkRecord {
            other_node: at(7, 0),
            as_integer1: Word(0x0000_1111),
        }],
        named_by: vec![NamedBy {
            node: at(7, 0),
            link: 2,
            as_integer1: Word(0x0000_1212),
        }],
        virtual_junctions: vec![JunctionRecord {
            index: 1,
            max_z: 4,
            min_x: 3,
            min_y: 2,
            height_base_world: 1,
            x_samples: 1,
            y_samples: 2,
            height_samples: vec![9, 8],
        }],
        junction_map: vec![PairRecord { place: 1, data: 1 }],
    },
    // Both documents rewritten, one with a node moved.
    |folder| {
        let text = |name: &str| std::fs::read_to_string(folder.join(name).join("paths.json"));
        let moved = text(SEVEN)
            .unwrap()
            .replacen("\"coors_x\": 100", "\"coors_x\": 150", 1);
        assert_ne!(moved, text(SEVEN).unwrap(), "the document states the node");
        Operation::RewriteRegions {
            documents: vec![
                Document {
                    source: SEVEN.into(),
                    document: moved,
                },
                Document {
                    source: EIGHT.into(),
                    document: text(EIGHT).unwrap(),
                },
            ],
        }
    },
];

/// The operations that are refused whatever they name, which no inverse takes back: each has its own test.
const ALWAYS_REFUSED: &[&str] = &["disable_node"];

/// The names of the operations, as the type itself lists them when it refuses a name.
fn variants() -> BTreeSet<String> {
    let refused = serde_json::from_str::<Operation>(r#"{ "op": "no such operation" }"#)
        .unwrap_err()
        .to_string();
    let (_, listed) = refused
        .split_once("expected one of")
        .unwrap_or_else(|| panic!("the refusal lists the operations: {refused}"));
    listed
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

#[test]
fn every_road_operation_followed_by_its_inverse_leaves_every_source_byte_for_byte_as_it_was() {
    let mut covered = BTreeSet::new();
    for (number, case) in CASES.iter().enumerate() {
        let scratch = tempfile::tempdir().unwrap();
        let folder = set(scratch.path());
        let operation = case(&folder);
        let name = operation.name();
        covered.insert(name.to_string());

        let before = files(&folder);
        let applied =
            apply(&folder, &operation).unwrap_or_else(|e| panic!("case {number} ({name}): {e}"));
        let after = files(&folder);
        assert_ne!(after, before, "case {number} ({name}) changes a source");
        assert!(!applied.edited.is_empty(), "case {number} ({name})");
        for edited in &applied.edited {
            assert_eq!(
                after.get(&edited.file),
                Some(&edited.after),
                "case {number} ({name}) says what it wrote"
            );
            assert_eq!(before.get(&edited.file), Some(&edited.before));
        }
        every_region_reads_back(&folder)
            .unwrap_or_else(|why| panic!("case {number} ({name}): {why}"));
        // The inverse says what to put back, and is the whole documents only for an operation that states documents.
        assert_eq!(
            applied.inverse.name() == "rewrite_regions",
            name == "rewrite_regions",
            "case {number} ({name}) is taken back by {}",
            applied.inverse.name()
        );

        let undone = apply(&folder, &applied.inverse)
            .unwrap_or_else(|e| panic!("the inverse of case {number} ({name}): {e}"));
        let restored = files(&folder);
        for (path, bytes) in &before {
            assert!(
                restored.get(path) == Some(bytes),
                "case {number} ({name}): {} is not as it was after its inverse",
                path.display()
            );
        }
        assert_eq!(
            restored.len(),
            before.len(),
            "case {number} ({name}) leaves no file behind"
        );

        // The inverse of the inverse does the operation again.
        apply(&folder, &undone.inverse)
            .unwrap_or_else(|e| panic!("case {number} ({name}) done again: {e}"));
        assert!(
            files(&folder) == after,
            "case {number} ({name}) done again writes what it wrote"
        );
    }
    let variants = variants();
    assert!(
        variants.contains("remove_node") && variants.len() > 1,
        "{variants:?}"
    );
    let refused: BTreeSet<String> = ALWAYS_REFUSED.iter().map(|n| n.to_string()).collect();
    assert!(covered.is_disjoint(&refused), "{covered:?}");
    assert_eq!(
        covered.union(&refused).cloned().collect::<BTreeSet<_>>(),
        variants,
        "every operation has a case that its inverse takes back, or is refused whatever it names"
    );
}

#[test]
fn a_region_that_does_not_cook_is_not_said_to_read_back() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = set(scratch.path());
    every_region_reads_back(&folder).unwrap();
    let document = folder.join(SEVEN).join("paths.json");
    let broken = std::fs::read_to_string(&document)
        .unwrap()
        .replacen("\"7:3\"", "\"7:9\"", 1);
    std::fs::write(&document, broken).unwrap();
    let why = every_region_reads_back(&folder).expect_err("a link to no node cooks");
    assert!(why.contains("7:9"), "{why}");
}

#[test]
fn the_json_of_every_road_operation_reads_back_as_itself_and_another_schema_is_refused() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = set(scratch.path());
    let mut operations: Vec<Operation> = CASES.iter().map(|case| case(&folder)).collect();
    operations.push(Operation::DisableNode { node: at(7, 1) });
    // The inverses carry records, and are operations a list may hold.
    for case in CASES {
        let scratch = tempfile::tempdir().unwrap();
        let folder = set(scratch.path());
        operations.push(apply(&folder, &case(&folder)).unwrap().inverse);
    }
    let list = Operations::new(operations);
    let text = list.to_json().unwrap();
    assert!(text.ends_with("}\n"));
    assert_eq!(Operations::from_json(text.as_bytes(), "x").unwrap(), list);
    // A node is named as a document names it, and a word as a document writes it.
    assert!(text.contains("\"node\": \"7:1\""), "{text}");
    assert!(
        text.contains("\"street_name_hash\": \"0xdeadbeef\""),
        "{text}"
    );

    let newer = text.replacen("\"schema\": 1", "\"schema\": 2", 1);
    assert!(matches!(
        Operations::from_json(newer.as_bytes(), "x"),
        Err(Error::Schema(_, 2))
    ));
    let foreign = text.replacen(FORMAT, "openrage-map-operations", 1);
    assert!(matches!(
        Operations::from_json(foreign.as_bytes(), "x"),
        Err(Error::Format(..))
    ));
    let unknown = text.replacen("\"node\": \"7:1\"", "\"node\": \"7:1\", \"lanes\": 2", 1);
    assert!(matches!(
        Operations::from_json(unknown.as_bytes(), "x"),
        Err(Error::Json(..))
    ));
}

#[test]
fn removing_a_node_a_link_of_another_region_names_is_refused_and_names_the_link() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = set(scratch.path());
    let said = refusal(&folder, &Operation::RemoveNode { node: at(7, 1) });
    assert!(
        said.contains("link 1 of node 8:1 (`nodes8.paths.src`)"),
        "{said}"
    );
    let said = refusal(&folder, &Operation::RemoveNode { node: at(7, 3) });
    assert!(
        said.contains("link 0 of node 8:2 (`nodes8.paths.src`)"),
        "{said}"
    );
    // The link taken out, the node is no longer named and goes.
    apply(
        &folder,
        &Operation::RemoveLink {
            node: at(8, 2),
            link: 0,
            back_link: None,
            one_way: false,
        },
    )
    .unwrap();
    apply(&folder, &Operation::RemoveNode { node: at(7, 3) }).unwrap();
    every_region_reads_back(&folder).unwrap();
}

/// Where the far node of each link into region 7 stands, for the links of the nodes of the source `name` from `first` on.
fn reached_in_seven(folder: &Path, name: &str, first: usize) -> Vec<[i16; 3]> {
    let seven = cooked(folder, SEVEN).unwrap();
    let region = cooked(folder, name).unwrap();
    (first..region.nodes.len())
        .flat_map(|index| links_of(&region, index))
        .filter(|((to, _), _)| *to == 7)
        .map(|((_, index), _)| coors(&seven.nodes[usize::from(index)]))
        .collect()
}

#[test]
fn removing_a_node_none_names_renumbers_the_links_of_the_neighbour_so_each_reaches_the_node_it_reached(
) {
    let scratch = tempfile::tempdir().unwrap();
    let folder = set(scratch.path());
    let gone = coors(&seven().nodes[0]);
    let from_eight = reached_in_seven(&folder, EIGHT, 0);
    // The links of the nodes that stay, without those to the node that goes.
    let from_seven: Vec<[i16; 3]> = reached_in_seven(&folder, SEVEN, 1)
        .into_iter()
        .filter(|stand| *stand != gone)
        .collect();
    assert_eq!(from_seven.len(), 2, "two links of region 7 stay");
    assert_eq!(from_eight.len(), 2, "region 8 links into region 7 twice");

    let applied = apply(&folder, &Operation::RemoveNode { node: at(7, 0) }).unwrap();
    let written: Vec<&Path> = applied.edited.iter().map(|e| e.file.as_path()).collect();
    assert_eq!(
        written,
        [
            folder.join(SEVEN).join("paths.json"),
            folder.join(EIGHT).join("paths.json")
        ],
        "the neighbour is written in the same step"
    );
    // By where the far node stands, and not by its index, which is what changed.
    assert_eq!(reached_in_seven(&folder, EIGHT, 0), from_eight);
    assert_eq!(reached_in_seven(&folder, SEVEN, 0), from_seven);
    let seven_now = cooked(&folder, SEVEN).unwrap();
    let eight_now = cooked(&folder, EIGHT).unwrap();
    assert_eq!(
        links_of(&eight_now, 1),
        [((8, 0), 0x61), ((7, 0), 0x62)],
        "the link that named 7:1 names 7:0, with its word"
    );
    assert_eq!(links_of(&eight_now, 2), [((7, 2), 0x71)]);
    assert_eq!(seven_now.nodes.len(), 3);
    assert_eq!(
        (seven_now.num_nodes_car_nodes, seven_now.num_nodes_ped_nodes),
        (2, 1)
    );
    for (place, node) in seven_now.nodes.iter().enumerate() {
        assert_eq!(
            (node.address.region, usize::from(node.address.index)),
            (7, place),
            "a node's address is its place"
        );
    }
    // No link of the set names the node that is gone, or a node past the last.
    for region in [&seven_now, &eight_now] {
        for index in 0..region.nodes.len() {
            for ((to, far), _) in links_of(region, index) {
                let held = if to == 7 { &seven_now } else { &eight_now }.nodes.len();
                assert!(usize::from(far) < held, "{to}:{far}");
            }
        }
    }
}

#[test]
fn removing_a_node_takes_its_junction_and_leaves_each_other_reference_its_own() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = set(scratch.path());
    apply(&folder, &Operation::RemoveNode { node: at(7, 0) }).unwrap();
    let region = cooked(&folder, SEVEN).unwrap();
    // The junction of the node that was 7:2 is the one left, named by the node's address now.
    assert_eq!(region.virtual_junctions.len(), 1);
    assert_eq!(region.virtual_junctions[0].min_x, -8);
    assert_eq!(region.height_samples, [40, 50, 60, 70]);
    let table = &region.junction_map.junction_map.data;
    assert_eq!(table.len(), 1);
    assert_eq!((table[0].address(), table[0].data), ((7, 1), 0));
    assert_eq!(coors(&region.nodes[1]), [200, 0, 10]);
}

#[test]
fn a_node_the_junction_table_names_is_not_moved_and_takes_any_other_value() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = set(scratch.path());
    let moved = |node, coors_x, coors_z| Operation::SetNode {
        node,
        street_name_hash: None,
        distance_to_target: None,
        coors_x,
        coors_y: None,
        coors_z,
        as_integer1: None,
        as_integer2: None,
    };
    for operation in [
        moved(at(7, 0), Some(5), None),
        moved(at(7, 2), None, Some(99)),
    ] {
        let said = refusal(&folder, &operation);
        assert!(
            said.contains("the junction table of region 7 names the node"),
            "{said}"
        );
    }
    // Where it stands stated again is no move.
    apply(&folder, &moved(at(7, 0), Some(0), Some(10))).unwrap();
    let named = Operation::SetNode {
        node: at(7, 0),
        street_name_hash: Some(Word(0x0BAD_F00D)),
        distance_to_target: None,
        coors_x: None,
        coors_y: None,
        coors_z: None,
        as_integer1: None,
        as_integer2: None,
    };
    apply(&folder, &named).unwrap();
    assert_eq!(
        cooked(&folder, SEVEN).unwrap().nodes[0].street_name_hash,
        0x0BAD_F00D
    );
}

#[test]
fn adding_a_node_keeps_the_vehicle_nodes_before_the_pedestrian_ones_and_the_counts_the_file_states()
{
    let scratch = tempfile::tempdir().unwrap();
    let folder = set(scratch.path());
    let pedestrian = coors(&seven().nodes[3]);
    let from_eight = reached_in_seven(&folder, EIGHT, 0);
    let added = |ped, linked_to| Operation::AddNode {
        region: 7,
        ped,
        coors_x: 250,
        coors_y: -5,
        coors_z: if ped { 21 } else { 11 },
        linked_to,
        as_integer1: Word(0x0000_0D0D),
        back_as_integer1: Word(0x0000_0E0E),
    };

    let applied = apply(&folder, &added(false, at(7, 2))).unwrap();
    assert_eq!(applied.added, Some(at(7, 3)), "after the last vehicle node");
    let region = cooked(&folder, SEVEN).unwrap();
    assert_eq!(region.nodes.len(), 5);
    assert_eq!(
        (region.num_nodes_car_nodes, region.num_nodes_ped_nodes),
        (4, 1)
    );
    assert_eq!(coors(&region.nodes[3]), [250, -5, 11]);
    assert_eq!(
        coors(&region.nodes[4]),
        pedestrian,
        "the pedestrian node is last"
    );
    assert_eq!(links_of(&region, 3), [((7, 2), 0x0D0D)]);
    assert_eq!(links_of(&region, 2), [((7, 4), 0x31), ((7, 3), 0x0E0E)]);
    // It states what the node it is linked to states, at its own place.
    assert_eq!(
        region.nodes[3].street_name_hash,
        region.nodes[2].street_name_hash
    );
    assert_eq!(region.nodes[3].as_integer2(), region.nodes[2].as_integer2());
    assert_eq!(
        region.nodes[3].as_integer1 & 0xFFFF,
        region.nodes[2].as_integer1 & 0xFFFF
    );
    assert_eq!(
        reached_in_seven(&folder, EIGHT, 0),
        from_eight,
        "the neighbour's link follows the pedestrian node"
    );
    assert_eq!(
        links_of(&cooked(&folder, EIGHT).unwrap(), 2),
        [((7, 4), 0x71)]
    );

    let applied = apply(&folder, &added(true, at(7, 4))).unwrap();
    assert_eq!(
        applied.added,
        Some(at(7, 5)),
        "after the last pedestrian node"
    );
    let region = cooked(&folder, SEVEN).unwrap();
    assert_eq!(
        (region.num_nodes_car_nodes, region.num_nodes_ped_nodes),
        (4, 2)
    );
    assert_eq!(region.nodes.len(), 6);
    assert_eq!(coors(&region.nodes[5]), [250, -5, 21]);

    let said = refusal(&folder, &added(true, at(8, 0)));
    assert!(said.contains("a node of its own region"), "{said}");
}

#[test]
fn a_link_added_across_two_regions_is_stated_in_both() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = set(scratch.path());
    let linked = Operation::AddLink {
        node: at(7, 2),
        other_node: at(8, 0),
        as_integer1: Word(0x0000_0A0A),
        link: None,
        back_as_integer1: Some(Word(0x0000_0B0B)),
        back_link: None,
    };
    let applied = apply(&folder, &linked).unwrap();
    let written: Vec<&Path> = applied.edited.iter().map(|e| e.file.as_path()).collect();
    assert_eq!(
        written,
        [
            folder.join(SEVEN).join("paths.json"),
            folder.join(EIGHT).join("paths.json")
        ]
    );
    assert!(applied.edited.iter().all(|e| e.changed()));
    assert_eq!(
        links_of(&cooked(&folder, SEVEN).unwrap(), 2),
        [((7, 3), 0x31), ((8, 0), 0x0A0A)]
    );
    assert_eq!(
        links_of(&cooked(&folder, EIGHT).unwrap(), 0),
        [((8, 1), 0x51), ((7, 2), 0x0B0B)]
    );
    assert_eq!(applied.notes, Vec::<String>::new());

    // A link to a node its region does not hold, and a link from a node to itself, are refused.
    let to = |other_node| Operation::AddLink {
        node: at(7, 2),
        other_node,
        as_integer1: Word(1),
        link: None,
        back_as_integer1: None,
        back_link: None,
    };
    let said = refusal(&folder, &to(at(8, 9)));
    assert!(said.contains("node 8:9 is past the 3 node(s)"), "{said}");
    let said = refusal(&folder, &to(at(7, 2)));
    assert!(said.contains("not linked to itself"), "{said}");
}

#[test]
fn a_node_holds_no_more_links_than_its_word_counts() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = set(scratch.path());
    let most = or_maps::roads::rules::most_links();
    let held = seven().nodes[2].num_links();
    let one_more = Operation::AddLink {
        node: at(7, 2),
        other_node: at(7, 1),
        as_integer1: Word(1),
        link: None,
        back_as_integer1: None,
        back_link: None,
    };
    let list = vec![one_more.clone(); most - held];
    apply_all(&folder, &list, false).unwrap();
    assert_eq!(cooked(&folder, SEVEN).unwrap().nodes[2].num_links(), most);
    let said = refusal(&folder, &one_more);
    assert!(said.contains("its word counts no more"), "{said}");
}

#[test]
fn a_request_to_disable_a_node_is_refused_because_the_file_states_no_such_thing() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = set(scratch.path());
    let said = refusal(&folder, &Operation::DisableNode { node: at(7, 1) });
    assert!(said.contains("the file states no such thing"), "{said}");
    assert!(said.starts_with("disable_node of node 7:1"), "{said}");
    // A node that is not there is refused for that.
    let said = refusal(&folder, &Operation::DisableNode { node: at(7, 40) });
    assert!(said.contains("node 7:40 is past the 4 node(s)"), "{said}");
}

#[test]
fn renumbering_is_refused_while_a_region_that_may_name_a_renumbered_node_is_not_in_the_set() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = seven_alone(scratch.path());
    // Node 7:1 links into region 8, whose link back could not be rewritten.
    let said = refusal(&folder, &Operation::RemoveNode { node: at(7, 0) });
    assert!(said.contains("node 7:1 links to 8:1"), "{said}");
    assert!(said.contains("add region(s) 8 to the set"), "{said}");
    let said = refusal(
        &folder,
        &Operation::AddNode {
            region: 7,
            ped: false,
            coors_x: 1,
            coors_y: 2,
            coors_z: 3,
            linked_to: at(7, 0),
            as_integer1: Word(1),
            back_as_integer1: Word(2),
        },
    );
    assert!(said.contains("node 7:3 links to 8:2"), "{said}");

    // A pedestrian node goes last and renumbers nothing, so it is added.
    let added = Operation::AddNode {
        region: 7,
        ped: true,
        coors_x: 1,
        coors_y: 2,
        coors_z: 3,
        linked_to: at(7, 0),
        as_integer1: Word(1),
        back_as_integer1: Word(2),
    };
    assert_eq!(apply(&folder, &added).unwrap().added, Some(at(7, 4)));

    // A link into the region that is not there is stated one way, and said; its link back cannot be stated.
    let linked = |back_as_integer1| Operation::AddLink {
        node: at(7, 0),
        other_node: at(8, 0),
        as_integer1: Word(3),
        link: None,
        back_as_integer1,
        back_link: None,
    };
    let said = refusal(&folder, &linked(Some(Word(4))));
    assert!(said.contains("region 8 is not in the set"), "{said}");
    let applied = apply(&folder, &linked(None)).unwrap();
    assert_eq!(applied.notes.len(), 1);
    assert!(
        applied.notes[0].contains("region 8 is not in the set"),
        "{:?}",
        applied.notes
    );
    every_region_reads_back(&folder).unwrap();
}

#[test]
fn a_document_typed_by_hand_is_put_back_as_it_was_typed() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = set(scratch.path());
    let document = folder.join(SEVEN).join("paths.json");
    let exported = std::fs::read_to_string(&document).unwrap();
    // The same document, on one line: what a person's editor or script may write.
    let typed =
        serde_json::to_string(&serde_json::from_str::<serde_json::Value>(&exported).unwrap())
            .unwrap();
    assert_ne!(typed, exported);
    std::fs::write(&document, &typed).unwrap();
    let before = files(&folder);

    let applied = apply(&folder, &Operation::RemoveNode { node: at(7, 0) }).unwrap();
    let Operation::RewriteRegions { documents } = &applied.inverse else {
        panic!("{:?}", applied.inverse);
    };
    assert_eq!(documents.len(), 2, "one for each source the removal wrote");
    assert_eq!(documents[0].document, typed);
    apply(&folder, &applied.inverse).unwrap();
    assert!(files(&folder) == before);

    // An operation that changes nothing leaves the document as it was typed.
    let same = Operation::SetLink {
        node: at(7, 0),
        link: 0,
        as_integer1: Word(0x11),
    };
    let applied = apply(&folder, &same).unwrap();
    assert_eq!(applied.edited, []);
    assert!(files(&folder) == before);
}

#[test]
fn a_list_is_applied_whole_or_not_at_all_and_a_dry_run_writes_nothing() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = set(scratch.path());
    let before = files(&folder);
    let good = Operation::SetLink {
        node: at(7, 0),
        link: 0,
        as_integer1: Word(0x99),
    };
    let bad = Operation::SetLink {
        node: at(7, 0),
        link: 9,
        as_integer1: Word(0x99),
    };
    let refused = apply_all(&folder, &[good.clone(), bad], false).unwrap_err();
    assert!(
        matches!(
            &refused,
            Error::InList {
                number: 2,
                of: 2,
                ..
            }
        ),
        "{refused:?}"
    );
    assert!(
        refused
            .to_string()
            .contains("operation 2 of 2, set_link of node 7:0: link 9 is past the 2 link(s)"),
        "{refused}"
    );
    assert!(files(&folder) == before, "a refused list changes no source");

    // A later operation of a list sees what an earlier one did.
    let list = [
        Operation::RemoveNode { node: at(7, 0) },
        Operation::SetNode {
            node: at(7, 2),
            street_name_hash: None,
            distance_to_target: Some(77),
            coors_x: None,
            coors_y: None,
            coors_z: None,
            as_integer1: None,
            as_integer2: None,
        },
    ];
    let dry = apply_all(&folder, &list, true).unwrap();
    assert!(files(&folder) == before, "a dry run writes nothing");
    let wet = apply_all(&folder, &list, false).unwrap();
    assert_eq!(dry, wet, "a dry run says what the run does");
    let region = cooked(&folder, SEVEN).unwrap();
    assert_eq!(region.nodes[2].distance_to_target, 77);
    assert_eq!(coors(&region.nodes[2]), [300, 50, 12]);

    // The inverses, newest first, put the set back.
    let back: Vec<Operation> = wet.iter().rev().map(|done| done.inverse.clone()).collect();
    apply_all(&folder, &back, false).unwrap();
    assert!(files(&folder) == before);
}

#[test]
fn an_operation_is_refused_while_a_source_of_the_set_does_not_cook_and_a_rewrite_mends_it() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = set(scratch.path());
    let document = folder.join(EIGHT).join("paths.json");
    let good = std::fs::read_to_string(&document).unwrap();
    std::fs::write(&document, good.replacen("\"8:1\"", "\"8:9\"", 1)).unwrap();
    let said = refusal(&folder, &Operation::RemoveNode { node: at(7, 2) });
    assert!(
        said.contains("`nodes8.paths.src` does not cook, so the regions of the set cannot be told"),
        "{said}"
    );
    // A document that does not compile is not held either.
    let rewrite = |document: &str| Operation::RewriteRegions {
        documents: vec![Document {
            source: EIGHT.into(),
            document: document.to_string(),
        }],
    };
    let said = refusal(&folder, &rewrite("{}"));
    assert!(said.contains("nodes8.paths.src"), "{said}");
    apply(&folder, &rewrite(&good)).unwrap();
    apply(&folder, &Operation::RemoveNode { node: at(7, 2) }).unwrap();
    every_region_reads_back(&folder).unwrap();
}

#[test]
fn a_junction_reference_of_another_region_holds_a_node_and_follows_a_renumbered_one() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = set(scratch.path());
    // Region 8 given a junction whose reference names the pedestrian node of region 7, in a document typed for it.
    let document = folder.join(EIGHT).join("paths.json");
    let mut stated: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&document).unwrap()).unwrap();
    stated["virtual_junctions"] = serde_json::json!([{
        "max_z": 1, "min_x": 2, "min_y": 3, "height_base_world": 4,
        "x_samples": 1, "y_samples": 1, "height_samples": [9],
    }]);
    stated["junction_map"]["data"] = serde_json::json!([{ "key": "7:3", "data": 0 }]);
    apply(
        &folder,
        &Operation::RewriteRegions {
            documents: vec![Document {
                source: EIGHT.into(),
                document: serde_json::to_string_pretty(&stated).unwrap(),
            }],
        },
    )
    .unwrap();
    // The link of region 8 that names 7:3 taken out, the reference alone still holds the node.
    let unlinked = Operation::RemoveLink {
        node: at(8, 2),
        link: 0,
        back_link: None,
        one_way: false,
    };
    apply(&folder, &unlinked).unwrap();
    let said = refusal(&folder, &Operation::RemoveNode { node: at(7, 3) });
    assert!(
        said.contains("by the junction table of region 8 (`nodes8.paths.src`)"),
        "{said}"
    );
    let said = refusal(
        &folder,
        &Operation::SetNode {
            node: at(7, 3),
            street_name_hash: None,
            distance_to_target: None,
            coors_x: Some(1),
            coors_y: None,
            coors_z: None,
            as_integer1: None,
            as_integer2: None,
        },
    );
    assert!(
        said.contains("the junction table of region 8 names the node"),
        "{said}"
    );

    // A node before it taken out, the reference names it where it is now.
    let before = files(&folder);
    let applied = apply(&folder, &Operation::RemoveNode { node: at(7, 0) }).unwrap();
    let table = cooked(&folder, EIGHT)
        .unwrap()
        .junction_map
        .junction_map
        .data;
    assert_eq!((table[0].address(), table[0].data), ((7, 2), 0));
    assert_eq!(
        coors(&cooked(&folder, SEVEN).unwrap().nodes[2]),
        [300, 50, 12]
    );
    apply(&folder, &applied.inverse).unwrap();
    assert!(
        files(&folder) == before,
        "the reference is put back with the node"
    );
}

#[test]
fn the_last_node_of_a_region_is_taken_out_and_put_back() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = scratch.path().join("roads");
    extract(
        &folder,
        "nodes9",
        &region(9, 1, &[node([5, 6, 7], &[])], &[]),
    );
    let before = files(&folder);
    let applied = apply(&folder, &Operation::RemoveNode { node: at(9, 0) }).unwrap();
    let emptied = cooked(&folder, "nodes9.paths.src").unwrap();
    assert_eq!(
        (emptied.nodes.len(), emptied.num_nodes_car_nodes),
        (0, 0),
        "a region with no node cooks"
    );
    // A region with no node states no address, so nothing can name a node of it until the documents are put back.
    let said = refusal(
        &folder,
        &Operation::AddNode {
            region: 9,
            ped: false,
            coors_x: 0,
            coors_y: 0,
            coors_z: 0,
            linked_to: at(9, 0),
            as_integer1: Word(1),
            back_as_integer1: Word(2),
        },
    );
    assert!(said.contains("region 9 is not in the set"), "{said}");
    assert_eq!(applied.inverse.name(), "rewrite_regions");
    apply(&folder, &applied.inverse).unwrap();
    assert!(files(&folder) == before);
}

/// A `set_node` of `node` that states its first word, its height and its second word, each when given.
fn stated(
    node: Address,
    as_integer1: Option<u32>,
    coors_z: Option<i16>,
    as_integer2: Option<u32>,
) -> Operation {
    Operation::SetNode {
        node,
        street_name_hash: None,
        distance_to_target: None,
        coors_x: None,
        coors_y: None,
        coors_z,
        as_integer1: as_integer1.map(Word),
        as_integer2: as_integer2.map(Word),
    }
}

#[test]
fn a_request_that_states_what_cannot_be_is_refused_for_what_it_states() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = set(scratch.path());

    let said = refusal(&folder, &stated(at(7, 1), None, None, None));
    assert!(said.contains("it states no value to set"), "{said}");
    // The first word states one height and `coors_z` another.
    let said = refusal(&folder, &stated(at(7, 1), Some(0x0009_0101), Some(8), None));
    assert!(
        said.contains(
            "`as_integer1` holds the height 9 in its high sixteen bits and `coors_z` states 8"
        ),
        "{said}"
    );
    apply(&folder, &stated(at(7, 1), Some(0x0009_0101), Some(9), None)).unwrap();
    // A bit of the second word that counts the links, asked of the declaration.
    let counting = (0..32)
        .map(|bit| 1u32 << bit)
        .find(|bit| {
            let mut probe = or_io::gta5::CPathNode::default();
            probe.set_as_integer2(*bit);
            probe.as_integer2() != *bit
        })
        .expect("the second word counts the links");
    let said = refusal(&folder, &stated(at(7, 1), None, None, Some(counting)));
    assert!(
        said.contains("states the bits that count the links of the node"),
        "{said}"
    );

    let said = refusal(
        &folder,
        &Operation::AddLink {
            node: at(7, 2),
            other_node: at(8, 0),
            as_integer1: Word(1),
            link: None,
            back_as_integer1: None,
            back_link: Some(0),
        },
    );
    assert!(said.contains("no `back_as_integer1` states one"), "{said}");
    let unlinked = |back_link, one_way| Operation::RemoveLink {
        node: at(7, 1),
        link: 1,
        back_link,
        one_way,
    };
    let said = refusal(&folder, &unlinked(Some(1), true));
    assert!(said.contains("`one_way` leaves the link back"), "{said}");
    // Link 0 of the far node goes elsewhere: it is not taken out as the link back.
    let said = refusal(&folder, &unlinked(Some(0), false));
    assert!(
        said.contains("link 0 of node 8:1 is not a link back to 7:1"),
        "{said}"
    );
    apply(&folder, &unlinked(Some(1), false)).unwrap();
    assert_eq!(
        links_of(&cooked(&folder, EIGHT).unwrap(), 1),
        [((8, 0), 0x61)]
    );

    let text = std::fs::read_to_string(folder.join(SEVEN).join("paths.json")).unwrap();
    let document = |source: &str| Document {
        source: source.into(),
        document: text.clone(),
    };
    let rewrite = |documents| Operation::RewriteRegions { documents };
    let said = refusal(&folder, &rewrite(vec![]));
    assert!(said.contains("it names no source to rewrite"), "{said}");
    let said = refusal(&folder, &rewrite(vec![document(SEVEN), document(SEVEN)]));
    assert!(said.contains("`nodes7.paths.src` is named twice"), "{said}");
    let said = refusal(&folder, &rewrite(vec![document("nodes9.paths.src")]));
    assert!(
        said.contains("no paths source of the folder is called `nodes9.paths.src`"),
        "{said}"
    );
}

#[test]
fn a_node_put_in_by_hand_is_refused_where_it_would_misstate_its_region() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = set(scratch.path());
    /// What an `insert_node` brings with its node.
    #[derive(Default)]
    struct With {
        named_by: Vec<NamedBy>,
        virtual_junctions: Vec<JunctionRecord>,
        junction_map: Vec<PairRecord>,
    }
    let put_in = |node, ped, with: With| Operation::InsertNode {
        node,
        ped,
        record: NodeRecord {
            street_name_hash: Word(1),
            distance_to_target: 0,
            coors_x: 900,
            coors_y: 900,
            as_integer1: Word(0),
            as_integer2: Word(0),
        },
        links: vec![],
        named_by: with.named_by,
        virtual_junctions: with.virtual_junctions,
        junction_map: with.junction_map,
    };
    let last = at(7, 4);

    // Region 7 holds three vehicle nodes and then a pedestrian one, and nothing in a node says which it is.
    let said = refusal(&folder, &put_in(last, false, With::default()));
    assert!(
        said.contains("node 7:4 is not a place for a vehicle node"),
        "{said}"
    );
    let said = refusal(&folder, &put_in(at(7, 2), true, With::default()));
    assert!(
        said.contains("node 7:2 is not a place for a pedestrian node"),
        "{said}"
    );

    let named_by = |node| With {
        named_by: vec![NamedBy {
            node,
            link: 0,
            as_integer1: Word(5),
        }],
        ..Default::default()
    };
    for (node, shown) in [(at(8, 0), "node 8:0"), (at(7, 9), "node 7:9")] {
        let said = refusal(&folder, &put_in(last, true, named_by(node)));
        assert!(
            said.contains(&format!("{shown} is not a node of region 7 as it stands")),
            "{said}"
        );
    }

    let junction = |index, height_samples| JunctionRecord {
        index,
        max_z: 1,
        min_x: 2,
        min_y: 3,
        height_base_world: 4,
        x_samples: 2,
        y_samples: 2,
        height_samples,
    };
    let with_junction = |record, pair| With {
        virtual_junctions: vec![record],
        junction_map: vec![pair],
        ..Default::default()
    };
    let named = PairRecord { place: 2, data: 2 };
    let said = refusal(
        &folder,
        &put_in(
            last,
            true,
            with_junction(junction(2, vec![1]), named.clone()),
        ),
    );
    assert!(
        said.contains("junction 2 holds 1 height samples, and its patch is x_samples by y_samples, which is 4"),
        "{said}"
    );
    let said = refusal(
        &folder,
        &put_in(
            last,
            true,
            with_junction(junction(5, vec![1, 2, 3, 4]), named.clone()),
        ),
    );
    assert!(
        said.contains("junction 5 is past the end of the 2 the region holds"),
        "{said}"
    );

    let referenced = |place, data| With {
        junction_map: vec![PairRecord { place, data }],
        ..Default::default()
    };
    let said = refusal(&folder, &put_in(last, true, referenced(9, 0)));
    assert!(
        said.contains("reference 9 is past the end of the 2 the junction table holds"),
        "{said}"
    );
    // The last node of the region named first in the table, which a lookup searches by address.
    let said = refusal(&folder, &put_in(last, true, referenced(0, 0)));
    assert!(
        said.contains("the junction table would not be in the order of its addresses"),
        "{said}"
    );
    let said = refusal(&folder, &put_in(last, true, referenced(2, 7)));
    assert!(
        said.contains("a reference names junction 7 of the 2 the region holds"),
        "{said}"
    );

    // The same node with a junction of its own, stated rightly, is put in.
    let applied = apply(
        &folder,
        &put_in(
            last,
            true,
            with_junction(junction(2, vec![1, 2, 3, 4]), named),
        ),
    )
    .unwrap();
    assert_eq!(applied.added, Some(last));
    let region = cooked(&folder, SEVEN).unwrap();
    assert_eq!(region.height_samples, [1, 2, 3, 40, 50, 60, 70, 1, 2, 3, 4]);
    let table = &region.junction_map.junction_map.data;
    assert_eq!((table[2].address(), table[2].data), ((7, 4), 2));
}

#[test]
fn an_operation_that_changes_nothing_is_taken_back_by_one_that_changes_nothing() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = set(scratch.path());
    // On a document typed by hand, which no inverse that edits a region puts back.
    let document = folder.join(SEVEN).join("paths.json");
    let exported = std::fs::read_to_string(&document).unwrap();
    let typed =
        serde_json::to_string(&serde_json::from_str::<serde_json::Value>(&exported).unwrap())
            .unwrap();
    std::fs::write(&document, typed).unwrap();
    let before = files(&folder);
    let same = Operation::SetLink {
        node: at(7, 0),
        link: 0,
        as_integer1: Word(0x11),
    };
    let applied = apply(&folder, &same).unwrap();
    assert_eq!(applied.edited, []);
    let back = apply(&folder, &applied.inverse).unwrap();
    assert_eq!(back.edited, []);
    assert!(files(&folder) == before);
}

#[test]
fn a_document_that_cannot_be_written_leaves_the_one_written_before_it_as_it_was() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = set(scratch.path());
    let before = files(&folder);
    // The part file of the second source is a folder, so its document cannot be written.
    std::fs::create_dir(folder.join(EIGHT).join("paths.json.part")).unwrap();
    let linked = Operation::AddLink {
        node: at(7, 2),
        other_node: at(8, 0),
        as_integer1: Word(0x0A0A),
        link: None,
        back_as_integer1: Some(Word(0x0B0B)),
        back_link: None,
    };
    let refused = apply(&folder, &linked).expect_err("a document cannot be written");
    assert!(
        matches!(&refused, Error::Write(why) if why.contains(EIGHT) && !why.contains("could not be put back")),
        "{refused:?}"
    );
    assert!(
        files(&folder) == before,
        "the document of the first source is put back"
    );
}

#[test]
fn a_link_left_one_way_is_said_and_a_link_that_is_returned_is_not() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = set(scratch.path());
    let one_way = Operation::AddLink {
        node: at(8, 0),
        other_node: at(8, 2),
        as_integer1: Word(1),
        link: None,
        back_as_integer1: None,
        back_link: None,
    };
    assert_eq!(
        apply(&folder, &one_way).unwrap().notes,
        ["the link is stated one way: node 8:2 states no link back to 8:0"]
    );
    // A link taken out alone and added again alone is returned by the link the far node kept.
    let alone = Operation::RemoveLink {
        node: at(7, 3),
        link: 2,
        back_link: None,
        one_way: true,
    };
    let applied = apply(&folder, &alone).unwrap();
    assert_eq!(applied.notes, Vec::<String>::new());
    assert_eq!(
        apply(&folder, &applied.inverse).unwrap().notes,
        Vec::<String>::new()
    );

    // The far region not in the set, its link back is left, and said.
    let scratch = tempfile::tempdir().unwrap();
    let folder = seven_alone(scratch.path());
    let unlinked = Operation::RemoveLink {
        node: at(7, 1),
        link: 1,
        back_link: None,
        one_way: false,
    };
    assert_eq!(
        apply(&folder, &unlinked).unwrap().notes,
        ["region 8 is not in the set, so a link back from 8:1 is left if it states one"]
    );
}

#[test]
fn an_operation_is_refused_while_two_sources_state_the_nodes_of_one_region() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = set(scratch.path());
    extract(&folder, "nodes7_again", &seven());
    let said = refusal(
        &folder,
        &Operation::SetLink {
            node: at(7, 0),
            link: 0,
            as_integer1: Word(0x99),
        },
    );
    assert!(
        said.contains(
            "`nodes7.paths.src` and `nodes7_again.paths.src` both state the nodes of region 7"
        ),
        "{said}"
    );
}
