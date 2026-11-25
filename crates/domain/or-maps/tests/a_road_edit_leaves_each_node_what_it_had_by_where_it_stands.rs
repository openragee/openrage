//! A road edit judged by a second reading of the set, which knows no index: each node is told by where it stands, a link by where its
//! far node stands and a junction by its patch. Every node, every link and every place a node can be added is tried on one set, so an
//! edit is held to what it should leave and not only to its own inverse, which a writer and a reader wrong the same way would pass.
//!
//! ⚠ The regions are built by hand, so the clean checkout runs this.

#[path = "common/roads.rs"]
mod roads;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use or_io::gta5::{AtBinaryMapDataPair, CPathRegion};
use or_maps::roads::operation::{apply, Error, Operation};
use or_maps::roads::{Address, Word};
use roads::{cooked, extract, files, node, region, Junction};

const FIVE: &str = "nodes5.paths.src";
const SIX: &str = "nodes6.paths.src";

/// Where a node stands, which names it here: no two nodes of the set stand at one place.
type Stand = [i16; 3];

/// Region 5: three vehicle nodes and two pedestrian ones. Its first node links to itself and twice to one node, its junctions are not
/// in the order of the nodes that name them, and two nodes name one junction.
fn five() -> CPathRegion {
    let mut built = region(
        5,
        3,
        &[
            node([0, 0, 1], &[(5, 1, 0x01), (5, 1, 0x02), (5, 0, 0x03)]),
            node([10, 0, 1], &[(5, 0, 0x04), (5, 2, 0x05), (6, 0, 0x06)]),
            node([20, 0, 1], &[(5, 1, 0x07), (5, 3, 0x08), (5, 4, 0x09)]),
            node([30, 0, 2], &[(5, 2, 0x0a), (5, 4, 0x0b)]),
            node([40, 0, 2], &[(5, 3, 0x0c), (5, 2, 0x0d), (6, 1, 0x0e)]),
        ],
        &[
            Junction {
                node: 4,
                min_x: 1,
                patch: (1, 1, vec![11]),
            },
            Junction {
                node: 0,
                min_x: 2,
                patch: (2, 1, vec![21, 22]),
            },
            Junction {
                node: 2,
                min_x: 3,
                patch: (1, 3, vec![31, 32, 33]),
            },
        ],
    );
    // The first pedestrian node names the junction the last vehicle node names.
    let table = &mut built.junction_map.junction_map.data;
    table.push(AtBinaryMapDataPair {
        key: 5 | 3 << 16,
        data: 2,
        ..Default::default()
    });
    table.sort_by_key(AtBinaryMapDataPair::address);
    built
}

/// Region 6: two vehicle nodes, each linked both ways with a node of region 5.
fn six() -> CPathRegion {
    region(
        6,
        2,
        &[
            node([100, 0, 1], &[(6, 1, 0x61), (5, 1, 0x62)]),
            node([110, 0, 1], &[(6, 0, 0x63), (5, 4, 0x64)]),
        ],
        &[],
    )
}

fn set(dir: &Path) -> PathBuf {
    let folder = dir.join("roads");
    extract(&folder, "nodes5", &five());
    extract(&folder, "nodes6", &six());
    folder
}

/// Where the far node of a link stands, or the address the link names when no node of the set is there.
type Far = Result<Stand, (u16, u16)>;

/// What one node is, with no index in it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    pedestrian: bool,
    street_name_hash: u32,
    distance_to_target: i16,
    as_integer1: u32,
    as_integer2: u32,
    /// Each link in the order the node holds them, with its word.
    links: Vec<(Far, u32)>,
    /// The junction each reference of the set names for the node: the region of the table, the corner of the patch and its samples.
    junctions: Vec<(u16, i16, Vec<u8>)>,
}

/// The set as a second reading sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Picture {
    nodes: BTreeMap<Stand, Seen>,
    /// The nodes of each region in the order the file holds them.
    order: BTreeMap<u16, Vec<Stand>>,
    /// The junctions of each region in the order the file holds them, each as the corner of its patch and its samples.
    junctions: BTreeMap<u16, Vec<(i16, Vec<u8>)>>,
}

fn stand(node: &or_io::gta5::CPathNode) -> Stand {
    [node.coors_x, node.coors_y, node.coors_z()]
}

fn picture(folder: &Path) -> Picture {
    let regions: BTreeMap<u16, CPathRegion> = [FIVE, SIX]
        .iter()
        .map(|name| cooked(folder, name).unwrap_or_else(|why| panic!("{why}")))
        .map(|held| (held.nodes[0].address.region, held))
        .collect();
    let at = |(region, index): (u16, u16)| -> Far {
        (regions.get(&region))
            .and_then(|held| held.nodes.get(usize::from(index)))
            .map(stand)
            .ok_or((region, index))
    };
    let mut seen = Picture {
        nodes: BTreeMap::new(),
        order: BTreeMap::new(),
        junctions: BTreeMap::new(),
    };
    for (id, held) in &regions {
        assert_eq!(
            held.num_nodes_car_nodes + held.num_nodes_ped_nodes,
            held.nodes.len() as i32
        );
        let mut links = held.links.iter();
        for (place, node) in held.nodes.iter().enumerate() {
            assert_eq!(
                (node.address.region, usize::from(node.address.index)),
                (*id, place),
                "a node's address is its place"
            );
            let own = (links.by_ref().take(node.num_links()))
                .map(|l| (at((l.other_node.region, l.other_node.index)), l.as_integer1))
                .collect();
            let was = seen.nodes.insert(
                stand(node),
                Seen {
                    pedestrian: place >= held.num_nodes_car_nodes as usize,
                    street_name_hash: node.street_name_hash,
                    distance_to_target: node.distance_to_target,
                    as_integer1: node.as_integer1,
                    as_integer2: node.as_integer2(),
                    links: own,
                    junctions: Vec::new(),
                },
            );
            assert!(was.is_none(), "no two nodes of the set stand at one place");
            seen.order.entry(*id).or_default().push(stand(node));
        }
        assert_eq!(links.count(), 0, "every link is a node's");
        let mut samples = held.height_samples.iter().copied();
        let patches: Vec<(i16, Vec<u8>)> = (held.virtual_junctions.iter())
            .map(|j| {
                let own: Vec<u8> = samples.by_ref().take(j.num_height_samples()).collect();
                assert_eq!(own.len(), j.num_height_samples());
                (j.min_x, own)
            })
            .collect();
        assert_eq!(samples.count(), 0, "every height sample is a junction's");
        seen.junctions.insert(*id, patches);
    }
    for (id, held) in &regions {
        let table = &held.junction_map.junction_map.data;
        assert!(
            table
                .windows(2)
                .all(|two| two[0].address() <= two[1].address()),
            "the table is in the order a lookup searches it"
        );
        for pair in table {
            let named = at(pair.address()).expect("a reference names a node that is there");
            let (min_x, samples) = seen.junctions[id][pair.data as usize].clone();
            let node = seen.nodes.get_mut(&named).unwrap();
            node.junctions.push((*id, min_x, samples));
        }
    }
    seen
}

/// Whether a link or a junction reference of another region names the node that stands at `gone`.
fn named_by_another_region(was: &Picture, region: u16, gone: Stand) -> bool {
    was.order.iter().any(|(id, nodes)| {
        *id != region
            && nodes
                .iter()
                .any(|holder| (was.nodes[holder].links.iter()).any(|(to, _)| *to == Ok(gone)))
    }) || was.nodes[&gone]
        .junctions
        .iter()
        .any(|(table, ..)| *table != region)
}

/// What the set is once the node that stands at `gone` is taken out of `region`.
fn without(was: &Picture, region: u16, gone: Stand) -> Picture {
    let mut expected = was.clone();
    let taken = expected.nodes.remove(&gone).unwrap();
    expected
        .order
        .get_mut(&region)
        .unwrap()
        .retain(|s| *s != gone);
    for node in expected.nodes.values_mut() {
        node.links.retain(|(to, _)| *to != Ok(gone));
    }
    // A junction goes when no node that stays names it.
    for (table, min_x, samples) in taken.junctions {
        let still_named = (expected.nodes.values())
            .any(|node| node.junctions.contains(&(table, min_x, samples.clone())));
        if !still_named {
            (expected.junctions.get_mut(&table).unwrap())
                .retain(|j| *j != (min_x, samples.clone()));
        }
    }
    expected
}

#[test]
fn each_node_taken_out_leaves_every_other_node_its_links_and_its_junction_by_where_they_stand() {
    let (mut removed, mut refused) = (0, 0);
    for (region, source) in [(5u16, FIVE), (6, SIX)] {
        let count = cooked(set(tempfile::tempdir().unwrap().path()).as_path(), source)
            .unwrap()
            .nodes
            .len();
        for index in 0..count as u16 {
            let scratch = tempfile::tempdir().unwrap();
            let folder = set(scratch.path());
            let before = files(&folder);
            let was = picture(&folder);
            let gone = was.order[&region][usize::from(index)];
            let take = Operation::RemoveNode {
                node: Address::new(region, index),
            };
            if named_by_another_region(&was, region, gone) {
                let why = apply(&folder, &take).expect_err("a node another region names stays");
                assert!(matches!(why, Error::Refused { .. }), "{why:?}");
                assert!(files(&folder) == before);
                refused += 1;
                continue;
            }
            let applied = apply(&folder, &take).unwrap_or_else(|e| panic!("{region}:{index}: {e}"));
            assert_eq!(
                picture(&folder),
                without(&was, region, gone),
                "{region}:{index} taken out"
            );
            assert_ne!(
                applied.inverse.name(),
                "rewrite_regions",
                "{region}:{index} is put back by what it took"
            );
            apply(&folder, &applied.inverse).unwrap_or_else(|e| panic!("{region}:{index}: {e}"));
            assert!(files(&folder) == before, "{region}:{index} put back");
            removed += 1;
        }
    }
    // The set holds both: a sweep that only refused, or never did, would show nothing.
    assert_eq!((removed, refused), (3, 4));
}

#[test]
fn each_link_taken_out_leaves_every_other_link_and_goes_with_the_link_back_or_alone() {
    let mut taken = 0;
    for (region, source) in [(5u16, FIVE), (6, SIX)] {
        let probe = tempfile::tempdir().unwrap();
        let held = cooked(&set(probe.path()), source).unwrap();
        for (index, node) in held.nodes.iter().enumerate() {
            for link in 0..node.num_links() {
                for one_way in [false, true] {
                    let scratch = tempfile::tempdir().unwrap();
                    let folder = set(scratch.path());
                    let before = files(&folder);
                    let was = picture(&folder);
                    let here = was.order[&region][index];
                    let (to, _) = was.nodes[&here].links[link];
                    let said = format!("link {link} of {region}:{index}, one way: {one_way}");
                    let applied = apply(
                        &folder,
                        &Operation::RemoveLink {
                            node: Address::new(region, index as u16),
                            link,
                            back_link: None,
                            one_way,
                        },
                    )
                    .unwrap_or_else(|e| panic!("{said}: {e}"));

                    let mut expected = was.clone();
                    let _ = expected.nodes.get_mut(&here).unwrap().links.remove(link);
                    // The link back is the first the far node states, and a node's link to itself has none.
                    if let (false, Ok(far)) = (one_way || to == Ok(here), to) {
                        let links = &mut expected.nodes.get_mut(&far).unwrap().links;
                        if let Some(back) = links.iter().position(|(to, _)| *to == Ok(here)) {
                            let _ = links.remove(back);
                        }
                    }
                    assert_eq!(picture(&folder), expected, "{said}");
                    apply(&folder, &applied.inverse).unwrap_or_else(|e| panic!("{said}: {e}"));
                    assert!(files(&folder) == before, "{said} put back");
                    taken += 1;
                }
            }
        }
    }
    let links = five().links.len() + six().links.len();
    assert_eq!(taken, 2 * links);
}

#[test]
fn a_node_added_beside_each_node_stands_after_its_kind_and_moves_no_link_and_no_junction() {
    let mut added = 0;
    for (region, source) in [(5u16, FIVE), (6, SIX)] {
        let probe = tempfile::tempdir().unwrap();
        let count = cooked(&set(probe.path()), source).unwrap().nodes.len();
        for index in 0..count as u16 {
            for ped in [false, true] {
                let scratch = tempfile::tempdir().unwrap();
                let folder = set(scratch.path());
                let before = files(&folder);
                let was = picture(&folder);
                let beside = was.order[&region][usize::from(index)];
                let new: Stand = [beside[0] + 1, -7, 9];
                let said = format!("a node beside {region}:{index}, pedestrian: {ped}");
                let applied = apply(
                    &folder,
                    &Operation::AddNode {
                        region,
                        ped,
                        coors_x: new[0],
                        coors_y: new[1],
                        coors_z: new[2],
                        linked_to: Address::new(region, index),
                        as_integer1: Word(0xA000 + u32::from(index)),
                        back_as_integer1: Word(0xB000 + u32::from(index)),
                    },
                )
                .unwrap_or_else(|e| panic!("{said}: {e}"));

                let mut expected = was.clone();
                let like = was.nodes[&beside].clone();
                (expected.nodes.get_mut(&beside).unwrap().links)
                    .push((Ok(new), 0xB000 + u32::from(index)));
                expected.nodes.insert(
                    new,
                    Seen {
                        pedestrian: ped,
                        as_integer1: (like.as_integer1 & 0xFFFF) | u32::from(new[2] as u16) << 16,
                        links: vec![(Ok(beside), 0xA000 + u32::from(index))],
                        junctions: Vec::new(),
                        ..like
                    },
                );
                let order = expected.order.get_mut(&region).unwrap();
                let vehicles = (order.iter()).filter(|s| !was.nodes[*s].pedestrian).count();
                order.insert(if ped { order.len() } else { vehicles }, new);
                let now = picture(&folder);
                assert_eq!(now, expected, "{said}");
                let place = now.order[&region].iter().position(|s| *s == new).unwrap();
                assert_eq!(
                    applied.added,
                    Some(Address::new(region, place as u16)),
                    "{said}"
                );
                apply(&folder, &applied.inverse).unwrap_or_else(|e| panic!("{said}: {e}"));
                assert!(files(&folder) == before, "{said} taken back");
                added += 1;
            }
        }
    }
    assert_eq!(added, 2 * (five().nodes.len() + six().nodes.len()));
}
