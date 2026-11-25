//! The places the World panel jumps to, found in the index at run time: the densest cells of the
//! maps that stream at rest, one for each head of their maps' names.

use std::collections::HashMap;

use glam::Vec3;

use crate::state::gta6_world::worldview::index::{Enabled, WorldIndex};

/// The side of a cell the maps are counted in, in metres.
const CELL: f32 = 512.0;
/// How many places the panel offers.
pub const MOST: usize = 8;
/// A box wider than this, a far level of detail's, says nothing of how tall the buildings of a cell are.
const WIDEST_BOX: f32 = 400.0;
/// How far south of a cell's middle the camera stands, looking north, in metres.
const BACK: f32 = 250.0;
/// How high over the tallest box of the cell the camera stands, in metres.
const OVER: f32 = 80.0;
/// How far the camera looks down, in degrees.
const PITCH: f32 = -25.0;

/// A place to jump to.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    /// The place the world-state data names at its middle, else the letters the names of most of its maps start with; `None` where the vocabulary names neither.
    pub head: Option<String>,
    /// Whether the head is the name of a place the world-state data outlines.
    pub from_place: bool,
    /// How many maps at rest lie in its cell.
    pub maps: usize,
    /// The middle of its maps, on the ground of the tallest box.
    pub middle: Vec3,
}

impl Place {
    /// Where the camera stands to see the place, looking north over it.
    pub fn pose(&self) -> super::world_viewer::Pose {
        super::world_viewer::Pose {
            position: self.middle + Vec3::new(0.0, -BACK, OVER),
            yaw: 0.0,
            pitch: PITCH,
            fov: None,
        }
    }
}

/// A map as the places count it: the head of its name, the middle of its entities' box, its top and its widest side.
pub struct Counted {
    pub head: Option<String>,
    pub middle: Vec3,
    pub top: f32,
    pub side: f32,
}

/// The letters a name starts with: `tw4_26_x` is `tw`.
pub fn head_of(name: &str) -> Option<String> {
    let head: String = (name.chars())
        .take_while(char::is_ascii_alphabetic)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    (!head.is_empty()).then_some(head)
}

#[derive(Default)]
struct Cell {
    maps: usize,
    sum: glam::DVec2,
    top: Option<f32>,
    heads: HashMap<String, usize>,
}

/// The densest cells, densest first, `most` at most, a head's densest alone: a cell is headed by the place `name_at` names at its middle, else by its maps' names.
pub fn places(
    maps: impl IntoIterator<Item = Counted>,
    most: usize,
    name_at: &dyn Fn(glam::Vec2) -> Option<String>,
) -> Vec<Place> {
    let mut cells: HashMap<(i32, i32), Cell> = HashMap::new();
    for map in maps {
        if !map.middle.is_finite() {
            continue;
        }
        let key = (
            (map.middle.x / CELL).floor() as i32,
            (map.middle.y / CELL).floor() as i32,
        );
        let cell = cells.entry(key).or_default();
        cell.maps += 1;
        cell.sum += map.middle.truncate().as_dvec2();
        if map.side <= WIDEST_BOX && map.top.is_finite() {
            cell.top = Some(cell.top.map_or(map.top, |top| top.max(map.top)));
        }
        if let Some(head) = map.head {
            *cell.heads.entry(head).or_default() += 1;
        }
    }
    let mut found: Vec<Place> = (cells.into_values())
        .map(|cell| {
            let middle = (cell.sum / cell.maps as f64).as_vec2();
            let named = name_at(middle);
            let from_place = named.is_some();
            let head = named.or_else(|| {
                (cell.heads.into_iter())
                    .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
                    .map(|(head, _)| head)
            });
            Place {
                head,
                from_place,
                maps: cell.maps,
                middle: middle.extend(cell.top.unwrap_or(0.0)),
            }
        })
        .collect();
    // Densest first, and a tie by place, so that the list is the same every run.
    found.sort_by(|a, b| {
        (b.maps.cmp(&a.maps))
            .then_with(|| a.middle.x.total_cmp(&b.middle.x))
            .then_with(|| a.middle.y.total_cmp(&b.middle.y))
    });
    // ⚠ A harvest names a few maps only: a cell none of whose maps is named still counts, by its place.
    let mut seen = std::collections::HashSet::new();
    found.retain(|p| p.head.as_ref().is_none_or(|head| seen.insert(head.clone())));
    found.truncate(most);
    found
}

/// The places of `index`, by the maps that stream at rest and the names the vocabulary gives them.
pub fn of_index(index: &WorldIndex) -> Vec<Place> {
    let at_rest = Enabled::default();
    let maps = (index.maps.iter()).filter(|m| at_rest.allows(m)).map(|m| {
        let (low, high) = m.boxes().entities;
        Counted {
            head: crate::state::gta6_names::name32(m.name).and_then(|name| head_of(&name)),
            middle: (low + high) / 2.0,
            top: high.z,
            side: (high.x - low.x).max(high.y - low.y),
        }
    });
    let vocabulary = crate::state::gta6_names::vocabulary();
    places(maps, MOST, &|at| {
        super::world_viewer_map::place_at(index, &vocabulary, at)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(head: Option<&str>, x: f32, y: f32, top: f32) -> Counted {
        Counted {
            head: head.map(str::to_string),
            middle: Vec3::new(x, y, top / 2.0),
            top,
            side: 50.0,
        }
    }

    #[test]
    fn a_head_is_the_letters_a_name_starts_with() {
        assert_eq!(head_of("tw4_26_strip").as_deref(), Some("tw"));
        assert_eq!(head_of("DN_tower").as_deref(), Some("dn"));
        assert_eq!(head_of("4_lone"), None);
        assert_eq!(head_of(""), None);
    }

    #[test]
    fn each_head_offers_its_densest_cell_densest_first() {
        let mut maps = Vec::new();
        // Four maps of `dn` in one cell, one of `dn` in another, two of `po` and one unnamed in a third.
        for (x, top) in [(10.0, 120.0), (20.0, 300.0), (30.0, 80.0), (20.0, 60.0)] {
            maps.push(map(Some("dn"), x, 10.0, top));
        }
        maps.push(map(Some("dn"), 2000.0, 10.0, 40.0));
        maps.push(map(Some("po"), -900.0, 10.0, 20.0));
        maps.push(map(Some("po"), -910.0, 10.0, 25.0));
        maps.push(map(None, -920.0, 10.0, 25.0));
        // Two unnamed maps in a cell of their own, which a harvest that named few maps leaves unnamed.
        maps.push(map(None, 9000.0, 10.0, 25.0));
        maps.push(map(None, 9010.0, 10.0, 25.0));
        let found = places(maps, MOST, &|_| None);
        let heads: Vec<_> = found.iter().map(|p| p.head.as_deref()).collect();
        assert_eq!(
            heads,
            [Some("dn"), Some("po"), None],
            "one place a head, and the unnamed by place: {found:?}"
        );
        assert_eq!(found[0].maps, 4);
        assert_eq!(
            found[0].middle,
            Vec3::new(20.0, 10.0, 300.0),
            "the middle of its maps, on its tallest box"
        );
        assert_eq!(
            found[1].maps, 3,
            "a map with no name still counts in its cell"
        );
        let pose = found[0].pose();
        assert!(
            pose.position.y < found[0].middle.y && pose.position.z > 300.0,
            "{pose:?}"
        );
        assert_eq!(places(Vec::new(), MOST, &|_| None), Vec::new());
    }

    #[test]
    fn a_cell_a_named_place_holds_is_headed_by_that_place() {
        let maps = vec![
            map(Some("tw"), 10.0, 10.0, 30.0),
            map(Some("tw"), 20.0, 10.0, 30.0),
            map(Some("tw"), 5000.0, 10.0, 30.0),
        ];
        let name_at = |at: glam::Vec2| (at.x < 1000.0).then(|| "Vice Beach".to_string());
        let found = places(maps, MOST, &name_at);
        let heads: Vec<_> = found
            .iter()
            .map(|p| (p.head.as_deref(), p.from_place))
            .collect();
        assert_eq!(heads, [(Some("Vice Beach"), true), (Some("tw"), false)]);
    }

    #[test]
    fn with_no_name_the_densest_cells_are_offered_and_a_far_box_does_not_raise_the_camera() {
        let mut maps = vec![map(None, 10.0, 10.0, 30.0), map(None, 20.0, 10.0, 30.0)];
        maps.push(Counted {
            side: 4000.0,
            ..map(None, 15.0, 10.0, 900.0)
        });
        maps.push(map(None, 5000.0, 10.0, 30.0));
        let found = places(maps, 1, &|_| None);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].head.clone(), found[0].maps), (None, 3));
        assert_eq!(
            found[0].middle.z, 30.0,
            "a box wider than a building is left out of the top"
        );
    }
}
