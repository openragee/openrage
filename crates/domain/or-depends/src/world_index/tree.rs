//! A tree over boxes that answers which of them hold a point, testing the boxes near the point and
//! not every one.

use glam::Vec3;

/// The most boxes a leaf holds before it is split.
const LEAF: usize = 8;

#[derive(Debug, Clone, Copy)]
struct Item {
    min: Vec3,
    max: Vec3,
    id: u32,
}

#[derive(Debug, Clone, Copy)]
struct Node {
    min: Vec3,
    max: Vec3,
    /// The items of a leaf, as a range of `items`; empty for a node with children.
    items: (u32, u32),
    /// The two children, as places in `nodes`; `None` for a leaf.
    children: Option<(u32, u32)>,
}

/// Boxes, each with the caller's number, split by where their centres lie.
#[derive(Debug, Clone, Default)]
pub(crate) struct BoxTree {
    nodes: Vec<Node>,
    items: Vec<Item>,
}

impl BoxTree {
    /// A tree over `boxes`, each `(min, max, id)`.
    ///
    /// ⚠ A box with a corner that is not a finite number, or whose minimum passes its maximum,
    /// is left out and answers no point: one infinite corner would grow every node above it.
    pub fn build(boxes: impl IntoIterator<Item = (Vec3, Vec3, u32)>) -> Self {
        let items: Vec<Item> = boxes
            .into_iter()
            .filter(|(min, max, _)| min.is_finite() && max.is_finite() && min.cmple(*max).all())
            .map(|(min, max, id)| Item { min, max, id })
            .collect();
        let mut tree = Self {
            nodes: Vec::new(),
            items,
        };
        if !tree.items.is_empty() {
            let end = tree.items.len();
            tree.split(0, end);
        }
        tree
    }

    /// Make the node over `items[start..end]` and those below it; its place in `nodes`.
    fn split(&mut self, start: usize, end: usize) -> u32 {
        let slice = &self.items[start..end];
        let (mut min, mut max) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
        let (mut low, mut high) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
        for item in slice {
            min = min.min(item.min);
            max = max.max(item.max);
            let centre = (item.min + item.max) * 0.5;
            low = low.min(centre);
            high = high.max(centre);
        }
        let at = self.nodes.len();
        self.nodes.push(Node {
            min,
            max,
            items: (start as u32, end as u32),
            children: None,
        });
        if end - start <= LEAF {
            return at as u32;
        }
        let spread = high - low;
        let axis = if spread.x >= spread.y && spread.x >= spread.z {
            0
        } else if spread.y >= spread.z {
            1
        } else {
            2
        };
        // The median by count, so boxes that share a centre still part and the depth stays bounded.
        let middle = (end - start) / 2;
        self.items[start..end].select_nth_unstable_by(middle, |a, b| {
            let (a, b) = ((a.min + a.max)[axis], (b.min + b.max)[axis]);
            a.total_cmp(&b)
        });
        let left = self.split(start, start + middle);
        let right = self.split(start + middle, end);
        let node = &mut self.nodes[at];
        node.items = (0, 0);
        node.children = Some((left, right));
        at as u32
    }

    /// The numbers of the boxes that hold `at`, lowest first.
    pub fn holding(&self, at: Vec3) -> Vec<u32> {
        self.holding_counted(at).0
    }

    /// [`Self::holding`], and how many boxes were tested to answer it.
    pub fn holding_counted(&self, at: Vec3) -> (Vec<u32>, usize) {
        let holds = |min: Vec3, max: Vec3| at.cmpge(min).all() && at.cmple(max).all();
        let mut found = Vec::new();
        let mut tested = 0;
        let mut pending: Vec<u32> = Vec::new();
        if !self.nodes.is_empty() {
            pending.push(0);
        }
        while let Some(index) = pending.pop() {
            let node = &self.nodes[index as usize];
            if !holds(node.min, node.max) {
                continue;
            }
            match node.children {
                Some((left, right)) => pending.extend([left, right]),
                None => {
                    for item in &self.items[node.items.0 as usize..node.items.1 as usize] {
                        tested += 1;
                        if holds(item.min, item.max) {
                            found.push(item.id);
                        }
                    }
                }
            }
        }
        found.sort_unstable();
        (found, tested)
    }

    /// How many boxes the tree holds.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A square of `side * side` boxes ten metres wide on a twenty-metre pitch, numbered by row.
    fn tiles(side: u32) -> Vec<(Vec3, Vec3, u32)> {
        (0..side * side)
            .map(|id| {
                let corner = Vec3::new((id % side) as f32, (id / side) as f32, 0.0) * 20.0;
                (corner, corner + Vec3::splat(10.0), id)
            })
            .collect()
    }

    fn by_scanning(boxes: &[(Vec3, Vec3, u32)], at: Vec3) -> Vec<u32> {
        let mut found: Vec<u32> = boxes
            .iter()
            .filter(|(min, max, _)| at.cmpge(*min).all() && at.cmple(*max).all())
            .map(|(_, _, id)| *id)
            .collect();
        found.sort_unstable();
        found
    }

    #[test]
    fn a_point_is_answered_from_the_boxes_near_it_and_not_from_all() {
        let side = 32;
        let boxes = tiles(side);
        let tree = BoxTree::build(boxes.clone());
        assert_eq!(tree.len(), boxes.len());

        let inside = Vec3::new(20.0 * 17.0 + 5.0, 20.0 * 9.0 + 5.0, 5.0);
        let (found, tested) = tree.holding_counted(inside);
        assert_eq!(found, vec![9 * side + 17]);
        assert!(
            tested <= 4 * LEAF,
            "{tested} of {} boxes were tested for one point",
            boxes.len()
        );

        let between = inside + Vec3::new(10.0, 0.0, 0.0);
        assert_eq!(tree.holding(between), Vec::<u32>::new());
    }

    #[test]
    fn the_tree_answers_what_a_scan_of_every_box_answers() {
        // Boxes of every size from one tile to the whole square, so a wide one overlaps many.
        let mut boxes = tiles(16);
        for (n, wide) in [40.0f32, 90.0, 170.0, 330.0].into_iter().enumerate() {
            for corner in [0.0f32, 55.0, 110.0] {
                let min = Vec3::new(corner, corner * 0.5, -5.0);
                boxes.push((
                    min,
                    min + Vec3::splat(wide),
                    1000 + (boxes.len() + n) as u32,
                ));
            }
        }
        let tree = BoxTree::build(boxes.clone());
        let mut asked = 0;
        for x in 0..40 {
            for y in 0..40 {
                let at = Vec3::new(x as f32 * 8.5 - 3.0, y as f32 * 8.5 - 3.0, 2.0);
                assert_eq!(tree.holding(at), by_scanning(&boxes, at), "{at}");
                asked += 1;
            }
        }
        assert_eq!(asked, 1600);
    }

    #[test]
    fn boxes_that_share_a_centre_still_part() {
        let boxes: Vec<_> = (0..100u32)
            .map(|id| (Vec3::splat(-1.0), Vec3::splat(1.0), id))
            .collect();
        let tree = BoxTree::build(boxes);
        assert_eq!(tree.holding(Vec3::ZERO).len(), 100);
        assert_eq!(tree.holding(Vec3::splat(2.0)).len(), 0);
    }

    #[test]
    fn a_box_that_is_no_box_answers_no_point_and_widens_nothing() {
        let boxes = vec![
            (Vec3::ZERO, Vec3::splat(10.0), 1),
            (
                Vec3::splat(f32::NEG_INFINITY),
                Vec3::splat(f32::INFINITY),
                2,
            ),
            (Vec3::splat(f32::NAN), Vec3::splat(10.0), 3),
            (Vec3::splat(10.0), Vec3::ZERO, 4),
        ];
        let tree = BoxTree::build(boxes);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree.holding(Vec3::splat(5.0)), vec![1]);
        assert_eq!(tree.holding(Vec3::splat(50.0)), Vec::<u32>::new());
        assert!(BoxTree::default().holding(Vec3::ZERO).is_empty());
    }
}
