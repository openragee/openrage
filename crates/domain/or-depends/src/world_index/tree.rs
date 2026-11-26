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

/// Whether a tree takes the box: every corner a finite number, and no minimum past its maximum.
pub(crate) fn is_a_box(min: Vec3, max: Vec3) -> bool {
    min.is_finite() && max.is_finite() && min.cmple(max).all()
}

impl BoxTree {
    /// A tree over `boxes`, each `(min, max, id)`.
    ///
    /// ⚠ A box that [`is_a_box`] refuses is left out and answers no point: one infinite corner
    /// would grow every node above it.
    pub fn build(boxes: impl IntoIterator<Item = (Vec3, Vec3, u32)>) -> Self {
        let items: Vec<Item> = boxes
            .into_iter()
            .filter(|(min, max, _)| is_a_box(*min, *max))
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
        self.walk(|min, max| at.cmpge(min).all() && at.cmple(max).all())
    }

    /// The numbers of the boxes that come within `radius` metres of the upright line through `x` and `y`, at any height, lowest first.
    pub fn in_column(&self, x: f32, y: f32, radius: f32) -> Vec<u32> {
        self.in_column_counted(x, y, radius).0
    }

    /// [`Self::in_column`], and how many boxes were tested to answer it.
    pub fn in_column_counted(&self, x: f32, y: f32, radius: f32) -> (Vec<u32>, usize) {
        // ⚠ `max` passes over a NaN, which would put a line that is nowhere inside every box.
        if x.is_nan() || y.is_nan() || radius.is_nan() || radius < 0.0 {
            return (Vec::new(), 0);
        }
        self.walk(|min, max| {
            let out_x = (min.x - x).max(x - max.x).max(0.0);
            let out_y = (min.y - y).max(y - max.y).max(0.0);
            out_x * out_x + out_y * out_y <= radius * radius
        })
    }

    /// The numbers of the boxes `holds` takes, lowest first, and how many boxes were tested.
    ///
    /// ⚠ `holds` must take a node's box wherever it takes a box inside it.
    fn walk(&self, holds: impl Fn(Vec3, Vec3) -> bool) -> (Vec<u32>, usize) {
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

    /// A repeatable stream of numbers in `0..1`.
    struct Dice(u64);

    impl Dice {
        fn between(&mut self, low: f32, high: f32) -> f32 {
            self.0 = (self.0)
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            low + (high - low) * ((self.0 >> 40) as f32 / (1u64 << 24) as f32)
        }
    }

    /// How far the rectangle a box stands on is from a point of the plane, by its nearest point.
    fn rectangle_distance(min: Vec3, max: Vec3, x: f32, y: f32) -> f32 {
        let nearest = (x.clamp(min.x, max.x), y.clamp(min.y, max.y));
        (nearest.0 - x).hypot(nearest.1 - y)
    }

    fn column_by_scanning(boxes: &[(Vec3, Vec3, u32)], x: f32, y: f32, radius: f32) -> Vec<u32> {
        let mut found: Vec<u32> = boxes
            .iter()
            .filter(|(min, max, _)| rectangle_distance(*min, *max, x, y) <= radius)
            .map(|(_, _, id)| *id)
            .collect();
        found.sort_unstable();
        found
    }

    #[test]
    fn a_column_answers_what_a_scan_of_every_box_answers() {
        let mut dice = Dice(0x70_67);
        // Boxes from a metre to four hundred across, at every height, so many stand over others.
        let boxes: Vec<(Vec3, Vec3, u32)> = (0..400u32)
            .map(|id| {
                let corner = Vec3::new(
                    dice.between(-2000.0, 2000.0),
                    dice.between(-2000.0, 2000.0),
                    dice.between(-200.0, 800.0),
                );
                let wide = if id % 7 == 0 { 400.0 } else { 60.0 };
                let size = Vec3::new(
                    dice.between(1.0, wide),
                    dice.between(1.0, wide),
                    dice.between(1.0, 90.0),
                );
                (corner, corner + size, id)
            })
            .collect();
        let tree = BoxTree::build(boxes.clone());
        let (mut asked, mut answered, mut stacked) = (0, 0, 0);
        for _ in 0..600 {
            let (x, y) = (dice.between(-2100.0, 2500.0), dice.between(-2100.0, 2500.0));
            for radius in [0.0f32, 35.0, 500.0] {
                let found = tree.in_column(x, y, radius);
                assert_eq!(
                    found,
                    column_by_scanning(&boxes, x, y, radius),
                    "{x} {y} within {radius}"
                );
                asked += 1;
                answered += usize::from(!found.is_empty());
            }
            // A column holds every box a point of it does, and those above and below as well.
            let in_column = tree.in_column(x, y, 0.0);
            let at_a_height = tree.holding(Vec3::new(x, y, 300.0));
            assert!(at_a_height.iter().all(|id| in_column.contains(id)));
            stacked += usize::from(in_column.len() > at_a_height.len());
        }
        assert_eq!(asked, 1800);
        assert!(
            answered > 600 && stacked > 50,
            "{answered} answers held a box, {stacked} columns more than one of their points"
        );
        // A corner of a box is in the column that runs through it, and a metre off is not.
        let (min, max, id) = boxes[0];
        assert!(tree.in_column(max.x, min.y, 0.0).contains(&id));
        assert!(!tree.in_column(max.x + 1.0, min.y, 0.0).contains(&id));
        assert!(tree.in_column(max.x + 1.0, min.y, 1.0).contains(&id));
    }

    #[test]
    fn a_column_is_answered_from_the_boxes_near_it_and_not_from_all() {
        let side = 32;
        let boxes = tiles(side);
        let tree = BoxTree::build(boxes.clone());
        let (x, y) = (20.0 * 17.0 + 5.0, 20.0 * 9.0 + 5.0);
        // Far above every box: no point holds it, and its column does.
        let (found, tested) = tree.in_column_counted(x, y, 0.0);
        assert_eq!(found, vec![9 * side + 17]);
        assert!(tree.holding(Vec3::new(x, y, 900.0)).is_empty());
        assert!(
            tested <= 4 * LEAF,
            "{tested} of {} boxes were tested for one column",
            boxes.len()
        );
    }

    #[test]
    fn a_column_that_is_nowhere_holds_nothing() {
        let tree = BoxTree::build(tiles(4));
        assert_eq!(tree.in_column(5.0, 5.0, 0.0), vec![0]);
        for (x, y, radius) in [
            (f32::NAN, 5.0, 0.0),
            (5.0, f32::NAN, 0.0),
            (5.0, 5.0, f32::NAN),
            (5.0, 5.0, -1.0),
            (f32::INFINITY, 5.0, 10.0),
        ] {
            assert_eq!(
                tree.in_column(x, y, radius),
                Vec::<u32>::new(),
                "{x} {y} {radius}"
            );
        }
        assert_eq!(tree.in_column(5.0, 5.0, f32::INFINITY).len(), 16);
        assert!(BoxTree::default().in_column(0.0, 0.0, 10.0).is_empty());
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
