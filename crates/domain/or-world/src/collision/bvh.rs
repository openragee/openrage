//! A tree over boxes, split at the median of the longest axis, built once and walked by a ray or a box.

use glam::Vec3;

use crate::extents::Aabb;

/// The most items a leaf holds.
const LEAF_ITEMS: usize = 4;

#[derive(Debug, Clone, Copy)]
struct Node {
    aabb: Aabb,
    /// A leaf's first slot of `order`.
    first: u32,
    /// A leaf's items; 0 for a node with two children, the first of which is the next node.
    count: u32,
    right: u32,
}

/// A tree over the boxes of a list of items.
#[derive(Debug, Clone, Default)]
pub(crate) struct Bvh {
    nodes: Vec<Node>,
    /// The items in leaf order.
    order: Vec<u32>,
}

impl Bvh {
    /// A tree over `boxes`, which a caller keeps to `u32::MAX` items.
    pub fn build(boxes: &[Aabb]) -> Self {
        let mut order: Vec<u32> = (0..boxes.len() as u32).collect();
        let mut nodes = Vec::new();
        if !boxes.is_empty() {
            split(boxes, &mut order, 0, boxes.len(), &mut nodes);
        }
        Self { nodes, order }
    }

    /// The box of every item; empty for a tree over none.
    pub fn bounds(&self) -> Aabb {
        self.nodes.first().map_or_else(Aabb::default, |n| n.aabb)
    }

    /// Visits the items whose box, grown by `pad`, a ray reaches; `visit` returns how far along it is still of interest.
    pub fn ray(
        &self,
        origin: Vec3,
        direction: Vec3,
        max: f32,
        pad: f32,
        mut visit: impl FnMut(u32) -> f32,
    ) {
        if self.nodes.is_empty() {
            return;
        }
        let mut limit = max;
        let mut stack = vec![0u32];
        while let Some(index) = stack.pop() {
            let node = &self.nodes[index as usize];
            if !reaches(&node.aabb, origin, direction, limit, pad) {
                continue;
            }
            if node.count == 0 {
                stack.push(node.right);
                stack.push(index + 1);
                continue;
            }
            for &item in &self.order[node.first as usize..][..node.count as usize] {
                limit = visit(item);
            }
        }
    }

    /// Visits the items whose box touches `region`.
    pub fn overlap(&self, region: &Aabb, mut visit: impl FnMut(u32)) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = vec![0u32];
        while let Some(index) = stack.pop() {
            let node = &self.nodes[index as usize];
            if !boxes_touch(&node.aabb, region) {
                continue;
            }
            if node.count == 0 {
                stack.push(node.right);
                stack.push(index + 1);
                continue;
            }
            for &item in &self.order[node.first as usize..][..node.count as usize] {
                visit(item);
            }
        }
    }
}

fn split(
    boxes: &[Aabb],
    order: &mut [u32],
    first: usize,
    count: usize,
    nodes: &mut Vec<Node>,
) -> u32 {
    let mut aabb = Aabb::default();
    for &item in &order[first..first + count] {
        aabb.union(&boxes[item as usize]);
    }
    let index = nodes.len();
    nodes.push(Node {
        aabb,
        first: first as u32,
        count: count as u32,
        right: 0,
    });
    if count <= LEAF_ITEMS {
        return index as u32;
    }
    let extent = aabb.max - aabb.min;
    let axis = if extent.x >= extent.y && extent.x >= extent.z {
        0
    } else if extent.y >= extent.z {
        1
    } else {
        2
    };
    let middle = |item: u32| {
        let b = &boxes[item as usize];
        b.min[axis] + b.max[axis]
    };
    let half = count / 2;
    order[first..first + count]
        .select_nth_unstable_by(half, |a, b| middle(*a).total_cmp(&middle(*b)));
    nodes[index].count = 0;
    split(boxes, order, first, half, nodes);
    let right = split(boxes, order, first + half, count - half, nodes);
    nodes[index].right = right;
    index as u32
}

/// Whether two boxes share a point, their faces included.
pub(crate) fn boxes_touch(a: &Aabb, b: &Aabb) -> bool {
    a.min.cmple(b.max).all() && b.min.cmple(a.max).all()
}

/// Whether a ray comes within `limit` of a box grown by `pad`.
pub(crate) fn reaches(aabb: &Aabb, origin: Vec3, direction: Vec3, limit: f32, pad: f32) -> bool {
    let (mut enter, mut exit) = (0.0f32, limit);
    for axis in 0..3 {
        let (min, max, from) = (aabb.min[axis], aabb.max[axis], origin[axis]);
        // A few units in the last place outward: the shape under a node is tested with other
        // arithmetic than its box.
        let grown = pad + 4.0 * f32::EPSILON * min.abs().max(max.abs()).max(from.abs());
        let (low, high) = (min - grown - from, max + grown - from);
        let rate = direction[axis];
        // ⚠ An axis the ray does not move along is a range test, never a division: an origin on
        // the box's face would make `0 / 0` of it, and the node with all under it would be missed.
        if rate == 0.0 {
            if low > 0.0 || high < 0.0 {
                return false;
            }
            continue;
        }
        let (near, far) = if rate > 0.0 {
            (low / rate, high / rate)
        } else {
            (high / rate, low / rate)
        };
        enter = enter.max(near);
        exit = exit.min(far);
        if enter > exit {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_box_at(x: f32, y: f32) -> Aabb {
        Aabb {
            min: Vec3::new(x, y, 0.0),
            max: Vec3::new(x + 1.0, y + 1.0, 1.0),
        }
    }

    /// A grid of unit boxes two apart, enough to make a tree several levels deep.
    fn grid() -> Vec<Aabb> {
        (0..12)
            .flat_map(|i| (0..12).map(move |j| unit_box_at(i as f32 * 2.0, j as f32 * 2.0)))
            .collect()
    }

    fn brute_ray(boxes: &[Aabb], origin: Vec3, direction: Vec3, max: f32) -> Vec<u32> {
        (0..boxes.len() as u32)
            .filter(|i| reaches(&boxes[*i as usize], origin, direction, max, 0.0))
            .collect()
    }

    #[test]
    fn a_ray_visits_every_item_whose_box_it_reaches() {
        let boxes = grid();
        let tree = Bvh::build(&boxes);
        let rays = [
            (Vec3::new(-1.0, 0.5, 0.5), Vec3::X, 100.0),
            (Vec3::new(4.5, 4.5, 9.0), Vec3::NEG_Z, 100.0),
            (
                Vec3::new(-3.0, -3.0, 0.5),
                Vec3::new(1.0, 1.0, 0.0).normalize(),
                100.0,
            ),
            (Vec3::new(-1.0, 0.5, 0.5), Vec3::X, 4.5),
            (Vec3::new(30.0, 30.0, 30.0), Vec3::Z, 10.0),
        ];
        for (origin, direction, max) in rays {
            let mut seen = Vec::new();
            tree.ray(origin, direction, max, 0.0, |item| {
                seen.push(item);
                max
            });
            let want = brute_ray(&boxes, origin, direction, max);
            for item in &want {
                assert!(
                    seen.contains(item),
                    "{origin} {direction}: item {item} was not visited"
                );
            }
            assert!(seen.len() < boxes.len() || want.len() == boxes.len());
        }
    }

    /// ⚠ The origin sits on the faces of boxes along an axis the ray does not move on.
    #[test]
    fn a_ray_along_a_face_still_reaches_the_boxes_it_borders() {
        let boxes = grid();
        let tree = Bvh::build(&boxes);
        let (origin, max) = (Vec3::new(4.0, 6.0, 5.0), 10.0);
        let mut seen = Vec::new();
        tree.ray(origin, Vec3::NEG_Z, max, 0.0, |item| {
            seen.push(item);
            max
        });
        let touched: Vec<u32> = (0..boxes.len() as u32)
            .filter(|i| {
                let b = &boxes[*i as usize];
                (b.min.x..=b.max.x).contains(&origin.x) && (b.min.y..=b.max.y).contains(&origin.y)
            })
            .collect();
        assert!(!touched.is_empty(), "the ray was meant to border a box");
        for item in touched {
            assert!(
                seen.contains(&item),
                "item {item} borders the ray and was not visited"
            );
        }
    }

    #[test]
    fn a_ray_stops_visiting_past_what_the_visitor_still_wants() {
        let boxes = grid();
        let tree = Bvh::build(&boxes);
        let mut far = Vec::new();
        tree.ray(Vec3::new(-1.0, 0.5, 0.5), Vec3::X, 100.0, 0.0, |item| {
            far.push(boxes[item as usize].min.x);
            2.5
        });
        assert!(!far.is_empty());
        assert!(
            far.iter().filter(|x| **x > 2.5 + 1.0).count() <= LEAF_ITEMS,
            "only the first leaf may hold items past the limit: {far:?}"
        );
    }

    #[test]
    fn a_pad_reaches_the_boxes_a_bare_ray_passes() {
        let boxes = grid();
        let tree = Bvh::build(&boxes);
        let (origin, direction) = (Vec3::new(-1.0, 1.5, 0.5), Vec3::X);
        let visited = |pad: f32| {
            let mut seen = Vec::new();
            tree.ray(origin, direction, 100.0, pad, |item| {
                seen.push(item);
                100.0
            });
            seen.sort_unstable();
            seen
        };
        let bare: Vec<u32> = visited(0.0)
            .into_iter()
            .filter(|i| reaches(&boxes[*i as usize], origin, direction, 100.0, 0.0))
            .collect();
        assert!(bare.is_empty(), "the ray runs between two rows of boxes");
        let padded: Vec<u32> = visited(0.75)
            .into_iter()
            .filter(|i| reaches(&boxes[*i as usize], origin, direction, 100.0, 0.75))
            .collect();
        assert_eq!(padded.len(), 2 * 12, "the rows on both sides");
    }

    #[test]
    fn a_box_visits_every_item_it_touches_and_no_tree_is_no_visit() {
        let boxes = grid();
        let tree = Bvh::build(&boxes);
        let region = Aabb {
            min: Vec3::new(3.0, 3.0, -1.0),
            max: Vec3::new(8.0, 6.0, 0.0),
        };
        let mut seen = Vec::new();
        tree.overlap(&region, |item| seen.push(item));
        // Box `i * 12 + j` spans 2i to 2i + 1 along X and 2j to 2j + 1 along Y, and a shared face touches.
        let want: Vec<u32> = (0..12u32)
            .flat_map(|i| (0..12u32).map(move |j| (i, j)))
            .filter(|(i, j)| 2 * i + 1 >= 3 && 2 * i <= 8 && 2 * j + 1 >= 3 && 2 * j <= 6)
            .map(|(i, j)| i * 12 + j)
            .collect();
        assert!(want.len() > 8, "the region was meant to touch some boxes");
        for item in &want {
            assert!(seen.contains(item), "box {item} touches the region");
            assert!(boxes_touch(&boxes[*item as usize], &region));
        }
        assert!(seen.len() < boxes.len(), "the far boxes are never visited");

        let empty = Bvh::build(&[]);
        assert!(empty.bounds().is_empty());
        empty.overlap(&region, |_| panic!("an empty tree holds nothing"));
        empty.ray(Vec3::ZERO, Vec3::X, 1.0, 0.0, |_| {
            panic!("an empty tree holds nothing")
        });
    }

    #[test]
    fn the_tree_spans_its_items_and_every_item_is_in_one_leaf() {
        let boxes = grid();
        let tree = Bvh::build(&boxes);
        let all = tree.bounds();
        assert_eq!(all.min, Vec3::ZERO);
        assert_eq!(all.max, Vec3::new(23.0, 23.0, 1.0));
        let mut order = tree.order.clone();
        order.sort_unstable();
        assert_eq!(order, (0..boxes.len() as u32).collect::<Vec<_>>());
        let leaves: usize = tree.nodes.iter().map(|n| n.count as usize).sum();
        assert_eq!(leaves, boxes.len());
        assert!(tree.nodes.iter().all(|n| n.count as usize <= LEAF_ITEMS));
    }

    /// ⚠ The build and both walks go as deep as the tree does, so a tree that
    /// peels one item off at a time would be as deep as a bound has polygons.
    #[test]
    fn the_tree_is_no_deeper_than_halving_its_items_makes_it() {
        fn depth(tree: &Bvh, index: usize) -> usize {
            let node = &tree.nodes[index];
            if node.count > 0 {
                return 1;
            }
            1 + depth(tree, index + 1).max(depth(tree, node.right as usize))
        }
        let boxes = grid();
        let tree = Bvh::build(&boxes);
        let halvings = boxes.len().ilog2() as usize + 1;
        assert!(
            depth(&tree, 0) <= halvings,
            "{} levels over {} items",
            depth(&tree, 0),
            boxes.len()
        );
    }
}
