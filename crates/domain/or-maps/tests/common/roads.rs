//! What the road suites share: path regions built by hand, each made a source in a folder. They hold no game content.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use or_io::decl::File;
use or_io::gta5::*;
use or_source::asset::Source;
use or_source::paths;
use or_source::textures::Extraction;

/// One node to build: where it stands, and the (region, index, word) of each link out of it.
#[derive(Clone)]
pub struct Node {
    pub coors: [i16; 3],
    pub links: Vec<(u16, u16, u32)>,
}

pub fn node(coors: [i16; 3], links: &[(u16, u16, u32)]) -> Node {
    Node {
        coors,
        links: links.to_vec(),
    }
}

/// One junction to build: the index of the node the table names for it, the corner of its patch, and its patch as (x, y, samples).
#[derive(Clone)]
pub struct Junction {
    pub node: u16,
    pub min_x: i16,
    pub patch: (u8, u8, Vec<u8>),
}

fn address(region: u16, index: u16) -> CNodeAddress {
    CNodeAddress {
        region,
        index,
        ..Default::default()
    }
}

/// Region `id` holding `nodes`, the first `car` of them vehicle nodes, with a junction for each of `junctions`.
pub fn region(id: u16, car: usize, nodes: &[Node], junctions: &[Junction]) -> CPathRegion {
    let mut built = CPathRegion {
        num_nodes_car_nodes: car as i32,
        num_nodes_ped_nodes: (nodes.len() - car) as i32,
        ..Default::default()
    };
    for (index, stated) in nodes.iter().enumerate() {
        let index = index as u16;
        let [x, y, z] = stated.coors;
        let mut node = CPathNode {
            address: address(id, index),
            street_name_hash: 0x1000_0000 + (u32::from(id) << 8) + u32::from(index),
            distance_to_target: -3 - index as i16,
            coors_x: x,
            coors_y: y,
            as_integer1: (0x0101 + u32::from(index)) | u32::from(z as u16) << 16,
            ..Default::default()
        };
        assert_eq!(
            node.coors_z(),
            z,
            "the height is where the declaration reads it"
        );
        node.set_as_integer2(0x8000_0401 + u32::from(index));
        node.set_num_links(stated.links.len()).unwrap();
        built.nodes.push(node);
        for (region, index, word) in &stated.links {
            built.links.push(CPathNodeLink {
                other_node: address(*region, *index),
                as_integer1: *word,
                ..Default::default()
            });
        }
    }
    // The table is searched by address, so it is built in that order.
    let mut by_node: Vec<(usize, &Junction)> = junctions.iter().enumerate().collect();
    by_node.sort_by_key(|(_, junction)| junction.node);
    for junction in junctions {
        let (x_samples, y_samples, samples) = &junction.patch;
        built.virtual_junctions.push(CPathVirtualJunction {
            max_z: 10,
            min_x: junction.min_x,
            min_y: -junction.min_x,
            height_base_world: 0xFFFE,
            x_samples: *x_samples,
            y_samples: *y_samples,
            ..Default::default()
        });
        built.height_samples.extend(samples);
    }
    built.junction_map.junction_map.sorted = 1;
    for (data, junction) in by_node {
        built
            .junction_map
            .junction_map
            .data
            .push(AtBinaryMapDataPair {
                key: u32::from(id) | u32::from(junction.node) << 16,
                data: data as i32,
                ..Default::default()
            });
    }
    built
}

/// Make `region` the source `<name>.paths.src` of `folder`.
pub fn extract(folder: &Path, name: &str, region: &CPathRegion) -> PathBuf {
    std::fs::create_dir_all(folder).unwrap();
    let file = region.to_bytes().expect("the region saves");
    let how = Extraction {
        from_game: false,
        virtual_path: None,
    };
    paths::extract(&file, name, folder, &how).unwrap_or_else(|why| panic!("{name}: {why:?}"))
}

/// Every file under `folder`, by its path.
pub fn files(folder: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(dir: &Path, into: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, into);
            } else {
                into.insert(path.clone(), std::fs::read(&path).unwrap());
            }
        }
    }
    let mut found = BTreeMap::new();
    walk(folder, &mut found);
    found
}

/// The region the source `name` of `folder` cooks to, read back through the owned type, once saving it again gives the file it was read from.
pub fn cooked(folder: &Path, name: &str) -> Result<CPathRegion, String> {
    let source = Source::open(&folder.join(name)).map_err(|p| p.to_string())?;
    let file = paths::compile(&source)
        .map_err(|found| format!("{name} does not cook: {found:?}"))?
        .bytes;
    let region = CPathRegion::from_bytes(&file).map_err(|e| format!("{name}: {e}"))?;
    let saved = region.to_bytes().map_err(|e| format!("{name}: {e}"))?;
    if saved != file {
        return Err(format!(
            "{name} cooks to a file its own save does not write again"
        ));
    }
    let again = CPathRegion::from_bytes(&saved)
        .and_then(|read| read.to_bytes())
        .map_err(|e| format!("{name}: {e}"))?;
    if again != saved {
        return Err(format!("{name} does not save to a fixed point"));
    }
    Ok(region)
}

/// The links out of the node at `index`, each as the address it names and its word.
pub fn links_of(region: &CPathRegion, index: usize) -> Vec<((u16, u16), u32)> {
    let start: usize = region.nodes[..index].iter().map(|n| n.num_links()).sum();
    region.links[start..start + region.nodes[index].num_links()]
        .iter()
        .map(|l| ((l.other_node.region, l.other_node.index), l.as_integer1))
        .collect()
}

/// Where a node stands.
pub fn coors(node: &CPathNode) -> [i16; 3] {
    [node.coors_x, node.coors_y, node.coors_z()]
}
