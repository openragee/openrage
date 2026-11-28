//! What a selection holds, counted for the check against what the game loads: the maps by name,
//! and the nodes of the loaded maps and those drawn by kind and level.

use std::collections::HashMap;

use or_world::streaming::{Forest, Kind, Level, Node, Selection};

use super::read::Loaded;

/// A selection's census; a kind is an entity, a prop list, a grass list or a far grass list, in that order.
#[derive(Debug, Clone, Default)]
pub struct Census {
    /// The names of the maps wanted, their parents among them.
    pub wanted: Vec<u32>,
    /// The names of the maps loaded.
    pub loaded: Vec<u32>,
    /// The nodes of the loaded maps, by kind and by level as a map numbers it.
    pub nodes: [[usize; Level::COUNT]; 4],
    /// The nodes that draw, by kind and level.
    pub drawn: [[usize; Level::COUNT]; 4],
    /// The instances the lists that draw put in the selection.
    pub list_instances: usize,
    /// Each node that draws, by its map's name and its place among the map's nodes, with how opaque.
    pub drawn_nodes: Vec<(u32, u32, f32)>,
}

/// The kind of `node`, the node at `at` of a map with `entities` entities.
// A list of far stand-ins is an `Entity` node that comes after the entities.
fn kind(node: &Node, at: usize, entities: usize) -> usize {
    match node.kind {
        Kind::Props => 1,
        Kind::Grass => 2,
        Kind::Entity if at >= entities => 3,
        Kind::Entity => 0,
    }
}

/// The census of `selection`, made of the maps `wanted` and `loaded` and the nodes `forest` holds.
// ponytail: every loaded node is looked up at each selection; count on insert if a selection shows it.
pub(super) fn count(
    wanted: &HashMap<u32, f32>,
    loaded: &HashMap<u32, Loaded>,
    forest: &Forest,
    selection: &Selection,
    list_instances: usize,
) -> Census {
    let mut census = Census {
        wanted: wanted.keys().copied().collect(),
        loaded: loaded.keys().copied().collect(),
        list_instances,
        ..Census::default()
    };
    for (name, map) in loaded {
        let key = u64::from(*name);
        for at in 0..map.drawn.len() {
            if let Some(node) = forest.node((key, at as u32)) {
                census.nodes[kind(node, at, map.entities.len())][node.level as usize] += 1;
            }
        }
    }
    let drawn = (selection.drawn.iter().map(|d| (d.node, d.fade)))
        .chain(selection.lists.iter().map(|l| (l.node, l.fade)));
    for ((key, at), fade) in drawn {
        let map = loaded.get(&(key as u32));
        if let (Some(map), Some(node)) = (map, forest.node((key, at))) {
            census.drawn[kind(node, at as usize, map.entities.len())][node.level as usize] += 1;
            census.drawn_nodes.push((key as u32, at, fade));
        }
    }
    census
}

#[cfg(test)]
#[path = "stream_census_tests.rs"]
mod tests;
