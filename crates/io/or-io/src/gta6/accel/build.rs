//! The arena and its builder: bottom levels from triangles and top levels from instances, in one
//! buffer of dwords.
//!
//! The tree is OpenRage's own, so the nodes a ray visits are not the console's. What holds of
//! every structure: it starts on a multiple of 256 bytes, never at address nought; every node
//! starts on a multiple of 64 bytes inside it; a box's bounds hold its children's to the bit; the
//! parent table is the inverse of the child pointers and the root's parent is no node; a pair
//! bit is only on a node whose pointer is of type 0. A top level ends in two boxes of the tree's
//! root and the second of them, under bounds no ray meets: the boxes a device's query is walked
//! through, which no walk reaches unless the header is made to state the first as the root. A
//! top level's boxes state the instances under each child, in bytes no walk reads, so that the
//! query's candidate leads to its instance with no table.

use std::collections::HashMap;

use super::layout::*;

/// Why a structure was not built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AccelError {
    #[error("a structure of nothing has no root")]
    Empty,
    #[error("triangle {0} has a corner that is not a finite number")]
    NotFinite(usize),
    #[error("triangle {0} names a geometry past 24 bits")]
    Geometry(usize),
    #[error("instance {0} has a transform with no inverse")]
    Singular(usize),
    #[error("the arena would pass the 4 GiB an address reaches")]
    TooLarge,
    #[error("a top level of {0} instances, where a node keeps 16 bits of an instance's number")]
    Instances(usize),
    #[error("the structure holds triangle {1} of geometry {0}, which the pose does not give")]
    Unposed(u32, u32),
    #[error("the two triangles of node {0:#x} no longer share an edge")]
    Torn(u32),
    #[error("the structure holds instance {0}, which the list does not")]
    Unplaced(usize),
}

/// One triangle of a bottom level, its corners in the model's winding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Triangle {
    pub corners: [[f32; 3]; 3],
    /// The geometry's number in its bottom level, below 2^24.
    pub geometry: u32,
    pub primitive: u32,
    pub opaque: bool,
}

/// A built structure: where it is, its root and what it bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Structure {
    /// Its byte address in the arena, a multiple of 256.
    pub base: u32,
    /// Its root's node pointer.
    pub root: u32,
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// One placement of a bottom level in a top level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Instance {
    pub bottom: Structure,
    /// Three rows of four floats.
    pub object_to_world: [[f32; 4]; 3],
    /// The low 24 bits are kept.
    pub id: u32,
    pub mask: u8,
    /// The `FLAG_*` bits.
    pub flags: u8,
    /// The low 24 bits are kept.
    pub contribution: u32,
}

/// The share of an instance's greatest coordinate, in either space, its world box is widened by
/// on every side: one unit in the last place of that coordinate.
///
/// The box is tested with the world ray and the triangles with the ray turned into the instance's
/// space; the box test's own widening by the origin's coordinates covers the rest.
const INSTANCE_PAD: f32 = 1.0 / 8_388_608.0;

/// How the builder parts what a box of a bottom level is to hold in two, which it does twice a
/// box. A top level is always parted about the median.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Split {
    /// About the median of the centres along the axis they spread most along.
    Median,
    /// Where the two parts' surfaces, each times what it holds, add up to the least, among the
    /// planes between sixteen bins of the centres along each axis. A ray walks fewer nodes of
    /// such a tree, which is why it is the default; it takes longer to build and holds more
    /// boxes.
    #[default]
    SurfaceArea,
}

/// The split of an arena that is given none.
fn split_of_a_new_arena() -> Split {
    // This crate's own tests run under either split by this variable; nothing else reads it.
    #[cfg(test)]
    match std::env::var("OPENRAGE_ACCEL_SPLIT").as_deref() {
        Ok("median") => return Split::Median,
        Ok("area") => return Split::SurfaceArea,
        _ => {}
    }
    Split::default()
}

/// Structures in one buffer of dwords, each read by its byte address.
#[derive(Debug, Clone)]
pub struct Arena {
    pub(super) words: Vec<u32>,
    split: Split,
}

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}

struct Item {
    min: [f32; 3],
    max: [f32; 3],
    /// Which leaf of the list being placed.
    leaf: u32,
}

/// One structure while it is laid out: its dwords from its own base.
struct Tree<'a> {
    words: Vec<u32>,
    /// A dword a unit after the header.
    parents: Vec<u32>,
    leaves: &'a [u32],
    leaf_type: u32,
    /// The instance nodes placed so far.
    instances: u32,
    split: Split,
}

fn union(items: &[Item]) -> ([f32; 3], [f32; 3]) {
    let (mut min, mut max) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for item in items {
        for k in 0..3 {
            min[k] = min[k].min(item.min[k]);
            max[k] = max[k].max(item.max[k]);
        }
    }
    (min, max)
}

/// Half the surface of the box from `min` to `max`; nought for a box of nothing.
fn half_area((min, max): ([f32; 3], [f32; 3])) -> f32 {
    let d: [f32; 3] = std::array::from_fn(|k| (max[k] - min[k]).max(0.0));
    d[0] * d[1] + d[1] * d[2] + d[2] * d[0]
}

/// The bins a plane is put between, along each axis.
const BINS: usize = 16;

/// `items` moved so that the first of them, as many as are counted, are the part a plane cuts off
/// at the least cost to walk: each part's surface times what it holds, added. `None` where no
/// plane between the bins of the centres parts them along any axis.
fn cheapest(items: &mut [Item]) -> Option<usize> {
    // Twice a centre, which orders as the centre does; and its bin along `axis`, where the
    // centres are not all one.
    let centre = |item: &Item, axis: usize| item.min[axis] + item.max[axis];
    let binned = |items: &[Item], axis: usize| {
        let (mut low, mut high) = (f32::INFINITY, f32::NEG_INFINITY);
        for item in items {
            low = low.min(centre(item, axis));
            high = high.max(centre(item, axis));
        }
        let scale = BINS as f32 / (high - low);
        (scale.is_finite() && scale > 0.0).then_some(move |item: &Item| {
            (((centre(item, axis) - low) * scale) as usize).min(BINS - 1)
        })
    };
    let empty = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    let grow = |bounds: &mut ([f32; 3], [f32; 3]), (min, max): ([f32; 3], [f32; 3])| {
        for k in 0..3 {
            bounds.0[k] = bounds.0[k].min(min[k]);
            bounds.1[k] = bounds.1[k].max(max[k]);
        }
    };
    // The least cost, with its axis and the first bin of its second part.
    let mut best: Option<(f32, usize, usize)> = None;
    for axis in 0..3 {
        let Some(bin) = binned(items, axis) else {
            continue;
        };
        let (mut held, mut bounds) = ([0usize; BINS], [empty; BINS]);
        for item in items.iter() {
            held[bin(item)] += 1;
            grow(&mut bounds[bin(item)], (item.min, item.max));
        }
        // What the bins from each plane on cost, then with what the bins before it cost.
        let mut after = [0.0f32; BINS];
        let (mut count, mut around) = (0, empty);
        for plane in (1..BINS).rev() {
            count += held[plane];
            grow(&mut around, bounds[plane]);
            after[plane] = half_area(around) * count as f32;
        }
        let (mut count, mut around) = (0, empty);
        for plane in 1..BINS {
            count += held[plane - 1];
            grow(&mut around, bounds[plane - 1]);
            let cost = half_area(around) * count as f32 + after[plane];
            // The first bin holds the least centre and the last the greatest, so every plane has
            // something on either side of it.
            if best.is_none_or(|(least, ..)| cost < least) {
                best = Some((cost, axis, plane));
            }
        }
    }
    let (_, axis, plane) = best?;
    let bin = binned(items, axis)?;
    let mut count = 0;
    for at in 0..items.len() {
        if bin(&items[at]) < plane {
            items.swap(count, at);
            count += 1;
        }
    }
    Some(count)
}

/// `items` in two parts: by the plane `split` asks for, and about the median of their centres on
/// the axis they spread most along where it asks for that or no plane parts them.
fn halve(items: &mut [Item], split: Split) -> (&mut [Item], &mut [Item]) {
    if split == Split::SurfaceArea {
        if let Some(count) = cheapest(items) {
            return items.split_at_mut(count);
        }
    }
    let (min, max) = union(items);
    let spread = |k: usize| max[k] - min[k];
    let axis = (1..3).fold(0, |best, k| if spread(k) > spread(best) { k } else { best });
    let middle = items.len() / 2;
    items.select_nth_unstable_by(middle, |p, q| {
        (p.min[axis] + p.max[axis]).total_cmp(&(q.min[axis] + q.max[axis]))
    });
    items.split_at_mut(middle)
}

impl Tree<'_> {
    fn units(&mut self, count: u32, parent: u32) -> u32 {
        let unit = (self.words.len() / 16) as u32;
        self.words.resize(self.words.len() + 16 * count as usize, 0);
        self.parents.push(parent);
        self.parents
            .resize(self.parents.len() + count as usize - 1, INVALID);
        unit
    }

    /// The two boxes a device's query is walked through, after the last node of the tree under
    /// `root`.
    fn query_boxes(&mut self, root: u32) {
        let first = self.units(BOX_BYTES / UNIT, INVALID);
        let again = self.units(BOX_BYTES / UNIT, INVALID);
        for unit in [first, again] {
            let at = unit as usize * 16;
            let children = [root, pointer(again, TYPE_BOX32), INVALID, INVALID];
            self.words[at..at + 4].copy_from_slice(&children);
            let bounds = at + (BOX_BOUNDS / 4) as usize;
            self.words[bounds..bounds + 24].fill(f32::NAN.to_bits());
        }
    }

    /// The node over `items`, placed with its subtree: its pointer and bounds.
    fn place(&mut self, items: &mut [Item], parent: u32) -> (u32, [f32; 3], [f32; 3]) {
        if let [item] = items {
            let stride = if self.leaf_type == TYPE_INSTANCE {
                32
            } else {
                16
            };
            let unit = self.units(stride as u32 / 16, parent);
            let from = item.leaf as usize * stride;
            let to = unit as usize * 16;
            self.words[to..to + stride].copy_from_slice(&self.leaves[from..from + stride]);
            if self.leaf_type == TYPE_INSTANCE {
                self.words[to + (INSTANCE_PARENT / 4) as usize] = parent;
                self.instances += 1;
            }
            return (pointer(unit, self.leaf_type), item.min, item.max);
        }
        let unit = self.units(BOX_BYTES / UNIT, parent);
        let me = pointer(unit, TYPE_BOX32);
        let (min, max) = union(items);
        let (low, high) = halve(items, self.split);
        let mut children = 0;
        for half in [low, high] {
            let quarters = if half.len() > 1 {
                let (a, b) = halve(half, self.split);
                [Some(a), Some(b)]
            } else {
                [Some(half), None]
            };
            for quarter in quarters.into_iter().flatten() {
                let first = self.instances;
                let (child, min, max) = self.place(quarter, me);
                let at = unit as usize * 16;
                self.words[at + children] = child;
                if self.leaf_type == TYPE_INSTANCE {
                    let range = first | (self.instances - 1) << 16;
                    self.words[at + (BOX_INSTANCES / 4) as usize + children] = range;
                }
                let bounds = at + ((BOX_BOUNDS + BOX_BOUNDS_STRIDE * children as u32) / 4) as usize;
                for k in 0..3 {
                    self.words[bounds + k] = min[k].to_bits();
                    self.words[bounds + 3 + k] = max[k].to_bits();
                }
                children += 1;
            }
        }
        for absent in children..4 {
            let at = unit as usize * 16;
            self.words[at + absent] = INVALID;
            if self.leaf_type == TYPE_INSTANCE {
                self.words[at + (BOX_INSTANCES / 4) as usize + absent] = INVALID;
            }
            let bounds = at + ((BOX_BOUNDS + BOX_BOUNDS_STRIDE * absent as u32) / 4) as usize;
            self.words[bounds..bounds + 6].fill(f32::NAN.to_bits());
        }
        (me, min, max)
    }
}

/// The inverse of the affine transform `m`, in single precision, or `None` where it has none.
fn inverse(m: &[[f32; 4]; 3]) -> Option<([[f32; 3]; 3], [f32; 3])> {
    let c = |i: usize, j: usize| {
        let (a, b) = ((i + 1) % 3, (i + 2) % 3);
        let (p, q) = ((j + 1) % 3, (j + 2) % 3);
        m[a][p] * m[b][q] - m[a][q] * m[b][p]
    };
    let determinant = m[0][0] * c(0, 0) + m[0][1] * c(0, 1) + m[0][2] * c(0, 2);
    if !(determinant.abs() > 0.0 && determinant.abs() < f32::INFINITY) {
        return None;
    }
    let rows: [[f32; 3]; 3] =
        std::array::from_fn(|i| std::array::from_fn(|j| c(j, i) / determinant));
    let translation = std::array::from_fn(|i| {
        -(rows[i][0] * m[0][3] + rows[i][1] * m[1][3] + rows[i][2] * m[2][3])
    });
    Some((rows, translation))
}

impl Arena {
    /// An arena of no structure, built by the default split: its first 256 bytes are kept, so
    /// no structure is at nought.
    pub fn new() -> Self {
        Self::with_split(split_of_a_new_arena())
    }

    /// [`Arena::new`] whose structures are built by `split`.
    pub fn with_split(split: Split) -> Self {
        Self {
            words: vec![0; (STRUCTURE_ALIGN / 4) as usize],
            split,
        }
    }

    /// The split the arena's structures are built by.
    pub fn split(&self) -> Split {
        self.split
    }

    /// The arena's dwords, which a storage buffer holds as they are.
    pub fn words(&self) -> &[u32] {
        &self.words
    }

    fn structure(
        &mut self,
        items: &mut [Item],
        leaves: &[u32],
        leaf_type: u32,
    ) -> Result<Structure, AccelError> {
        if items.is_empty() {
            return Err(AccelError::Empty);
        }
        let mut tree = Tree {
            words: vec![0; (HEADER_BYTES / 4) as usize],
            parents: Vec::new(),
            leaves,
            leaf_type,
            instances: 0,
            // A wave's stack holds three pairs a level of a top level, and a push past its
            // bound is lost: a balanced tree keeps a top level of any size far under it.
            split: match leaf_type {
                TYPE_INSTANCE => Split::Median,
                _ => self.split,
            },
        };
        let (root, min, max) = tree.place(items, INVALID);
        if leaf_type == TYPE_INSTANCE {
            tree.query_boxes(root);
        }
        let table = tree.words.len() * 4;
        tree.words[(HEADER_ROOT / 4) as usize] = root;
        tree.words[(HEADER_FIRST_NODE / 4) as usize] = HEADER_BYTES;
        tree.words[(HEADER_PARENTS / 4) as usize] = table as u32;
        tree.words.extend_from_slice(&tree.parents);
        let aligned = tree
            .words
            .len()
            .next_multiple_of((STRUCTURE_ALIGN / 4) as usize);
        tree.words.resize(aligned, 0);
        // A node pointer keeps 29 bits, so a structure's nodes end below 4 GiB with the arena.
        let end = (self.words.len() + tree.words.len()) as u64 * 4;
        if end > u64::from(u32::MAX) {
            return Err(AccelError::TooLarge);
        }
        let base = self.words.len() as u32 * 4;
        self.words.append(&mut tree.words);
        Ok(Structure {
            base,
            root,
            min,
            max,
        })
    }

    /// States in the header of the top level `top` the first of its two query boxes as the root,
    /// or the tree's own root again; false, with nothing written, where `top` is no top level.
    pub fn state_query_root(&mut self, top: &Structure, query: bool) -> bool {
        let header = (top.base / 4) as usize;
        let table = self.words[header + (HEADER_PARENTS / 4) as usize];
        // The two boxes are the last two nodes before the parent table.
        let first = (table / UNIT).wrapping_sub(2 * (BOX_BYTES / UNIT));
        let named = (self.words).get(header + first as usize * 16);
        if named != Some(&top.root) || first < HEADER_BYTES / UNIT {
            return false;
        }
        let root = if query {
            pointer(first, TYPE_BOX32)
        } else {
            top.root
        };
        self.words[header + (HEADER_ROOT / 4) as usize] = root;
        true
    }

    /// A bottom level of `triangles`: one node a triangle, or one for two that share an edge
    /// in opposite directions, one geometry and one opacity.
    pub fn bottom_level(&mut self, triangles: &[Triangle]) -> Result<Structure, AccelError> {
        for (index, triangle) in triangles.iter().enumerate() {
            if !triangle.corners.iter().flatten().all(|x| x.is_finite()) {
                return Err(AccelError::NotFinite(index));
            }
            if triangle.geometry >= 1 << 24 {
                return Err(AccelError::Geometry(index));
            }
        }
        type Edge = ([u32; 3], [u32; 3], u32, bool);
        let bits = |corner: [f32; 3]| corner.map(f32::to_bits);
        let edge = |t: &Triangle, e: usize, reversed: bool| -> Edge {
            let (from, to) = (bits(t.corners[e]), bits(t.corners[(e + 1) % 3]));
            match reversed {
                false => (from, to, t.geometry, t.opaque),
                true => (to, from, t.geometry, t.opaque),
            }
        };
        let mut owners: HashMap<Edge, (u32, u8)> = HashMap::new();
        for (index, triangle) in triangles.iter().enumerate() {
            for e in 0..3 {
                (owners.entry(edge(triangle, e, false))).or_insert((index as u32, e as u8));
            }
        }
        let mut placed = vec![false; triangles.len()];
        let (mut leaves, mut items) = (Vec::new(), Vec::new());
        for (index, first) in triangles.iter().enumerate() {
            if placed[index] {
                continue;
            }
            placed[index] = true;
            let partner = (0..3).find_map(|e| {
                let &(other, f) = owners.get(&edge(first, e, true))?;
                (!placed[other as usize]).then_some((e, other as usize, f as usize))
            });
            // The shared edge runs from vertex 1 to vertex 2, so the first triangle is turned to
            // end on it and the second starts at its far end.
            let (turn, second) = match partner {
                Some((e, other, f)) => {
                    placed[other] = true;
                    (e, Some((&triangles[other], f)))
                }
                None => (1, None),
            };
            let mut vertices = [[0.0f32; 3]; 4];
            for (place, corner) in [turn + 2, turn, turn + 1].into_iter().enumerate() {
                vertices[place] = first.corners[corner % 3];
            }
            // The place in its node's triangle of the corner the model numbers `k`.
            let id = |places: [usize; 3]| (places[1] | (places[2] << 2)) as u32;
            let mut ids = id(std::array::from_fn(|k| (k + 4 - turn) % 3));
            let mut word = (first.geometry << 8) | (u32::from(first.opaque) * TRIANGLE_OPAQUE);
            let mut primitives = [first.primitive, 0];
            if let Some((other, f)) = second {
                vertices[3] = other.corners[(f + 2) % 3];
                ids |= id(std::array::from_fn(|k| (k + 5 - f) % 3)) << 8;
                word |= TRIANGLE_PAIR;
                primitives[1] = other.primitive;
            }
            let used = &vertices[..if second.is_some() { 4 } else { 3 }];
            let mut item = Item {
                min: [f32::INFINITY; 3],
                max: [f32::NEG_INFINITY; 3],
                leaf: items.len() as u32,
            };
            for vertex in used {
                for k in 0..3 {
                    item.min[k] = item.min[k].min(vertex[k]);
                    item.max[k] = item.max[k].max(vertex[k]);
                }
            }
            items.push(item);
            leaves.extend(vertices.iter().flatten().map(|x| x.to_bits()));
            leaves.extend([word, primitives[0], primitives[1], ids]);
        }
        self.structure(&mut items, &leaves, 0)
    }

    /// A top level of `instances`, each an instance node whose number is its place in the list.
    pub fn top_level(&mut self, instances: &[Instance]) -> Result<Structure, AccelError> {
        if instances.len() > 1 << 16 {
            return Err(AccelError::Instances(instances.len()));
        }
        let (mut leaves, mut items) = (Vec::new(), Vec::new());
        for (index, instance) in instances.iter().enumerate() {
            let (node, min, max) = instance_node(instance, index)?;
            items.push(Item {
                min,
                max,
                leaf: index as u32,
            });
            leaves.extend(node);
        }
        self.structure(&mut items, &leaves, TYPE_INSTANCE)
    }
}

/// An instance node's dwords, and the least and the greatest corner of its box in the world.
pub(super) type InstanceNode = ([u32; (INSTANCE_BYTES / 4) as usize], [f32; 3], [f32; 3]);

/// The node of `instance`, the `index`th of its list, under no parent yet, with its box in the
/// world.
pub(super) fn instance_node(instance: &Instance, index: usize) -> Result<InstanceNode, AccelError> {
    let m = &instance.object_to_world;
    let (rows, translation) = inverse(m).ok_or(AccelError::Singular(index))?;
    let identity = rows == [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let (mut min, mut max) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for corner in 0..8 {
        let p: [f32; 3] = std::array::from_fn(|k| match (corner >> k) & 1 {
            0 => instance.bottom.min[k],
            _ => instance.bottom.max[k],
        });
        for k in 0..3 {
            let x = m[k][0] * p[0] + m[k][1] * p[1] + m[k][2] * p[2] + m[k][3];
            min[k] = min[k].min(x);
            max[k] = max[k].max(x);
        }
    }
    let bottom = (instance.bottom.min.into_iter()).chain(instance.bottom.max);
    let reach = (bottom.chain(min).chain(max).chain(translation))
        .fold(0.0f32, |reach, x| reach.max(x.abs()));
    for k in 0..3 {
        min[k] -= reach * INSTANCE_PAD;
        max[k] += reach * INSTANCE_PAD;
    }
    let mut node = [0u32; (INSTANCE_BYTES / 4) as usize];
    let at = |field: u32| (field / 4) as usize;
    for (k, x) in m.as_flattened().iter().enumerate() {
        node[at(INSTANCE_OBJECT_TO_WORLD) + k] = x.to_bits();
    }
    node[at(INSTANCE_ID_MASK)] = (u32::from(instance.mask) << 24) | (instance.id & 0xFF_FFFF);
    node[at(INSTANCE_CONTRIBUTION_FLAGS)] =
        (u32::from(instance.flags) << 24) | (instance.contribution & 0xFF_FFFF);
    let root = instance.bottom.root & INSTANCE_NO_ROOT;
    node[at(INSTANCE_ROOT)] = root | if identity { INSTANCE_IDENTITY } else { 0 };
    node[at(INSTANCE_BOTTOM)] = instance.bottom.base >> 8;
    // The translation's three floats, then the rotation's nine.
    for (k, x) in translation.iter().chain(rows.as_flattened()).enumerate() {
        node[at(INSTANCE_TRANSLATION) + k] = x.to_bits();
    }
    // The parent is written when the node is placed.
    node[at(INSTANCE_INDEX)] = index as u32;
    node[at(INSTANCE_PARENT)] = INVALID;
    Ok((node, min, max))
}

#[cfg(test)]
#[path = "build_tests.rs"]
pub(super) mod tests;
