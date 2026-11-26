//! The far stand-ins of the maps streamed in, as the card pass takes them: each list's box, scales
//! and distance with its instances as the body stores them, and the species each id names.

use std::f32::consts::SQRT_2;
use std::sync::Arc;

use glam::Vec3;
use or_io::gta6::map::stream::StreamGrassList;

use super::super::index::{ArchetypeRow, ModelRef, WorldIndex};

/// A list of far stand-ins that draws, as the card pass takes it.
#[derive(Debug, Clone, PartialEq)]
pub struct CardList {
    /// The sphere that holds every card of the list, corners included.
    pub centre: [f32; 3],
    pub radius: f32,
    /// The lowest corner of the box each instance's place is a fraction of.
    pub min: [f32; 3],
    /// The size of that box.
    pub size: [f32; 3],
    /// The least and the most scale of an instance.
    pub scale_range: [f32; 2],
    /// How far from the camera an instance draws to, in metres.
    pub distance: f32,
    /// How opaque the list draws, above 0 and to 1.
    pub fade: f32,
    /// Each instance as the body stores it: the u16 at +0x0E is the id of its species.
    pub records: Arc<[[u8; 16]]>,
    /// The ids of the species its instances take, each once.
    pub ids: Arc<[u16]>,
}

/// What a far stand-in id stands in for: an archetype's name and its model's sphere; a name of 0 for an id no list settles.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Species {
    pub archetype: u32,
    /// The model's sphere in its own space; a radius of 0 draws nothing.
    pub centre: [f32; 3],
    pub radius: f32,
}

impl CardList {
    /// The cards of the far stand-in list `list`, whole and at its own distance, by the species of `index`.
    pub fn of(list: &StreamGrassList, index: &WorldIndex) -> Self {
        let (min, max) = (
            Vec3::from(list.batch_aabb.min),
            Vec3::from(list.batch_aabb.max),
        );
        // A card stands off its place by its species' centre and its corner reaches past it by
        // the radius times √2, both at the instance's scale.
        let mut reach = 0.0f32;
        let mut ids = std::collections::BTreeSet::new();
        for instance in list.instances.iter().filter(|i| ids.insert(i.guid)) {
            if let Some(a) = archetype_of(index, usize::from(instance.guid)) {
                reach = reach.max(Vec3::from(a.centre).length() + a.radius * SQRT_2);
            }
        }
        let most = list.scale_range[1];
        Self {
            centre: ((min + max) / 2.0).to_array(),
            radius: (max - min).length() / 2.0 + reach * most,
            min: min.to_array(),
            size: (max - min).to_array(),
            scale_range: list.scale_range,
            distance: list.instance_lod_dist,
            fade: 1.0,
            records: list.instances.iter().map(|i| i.record()).collect(),
            ids: ids.into_iter().collect(),
        }
    }

    /// The same seen with distances scaled by `scale`, `fade` opaque, less the instances `hidden`
    /// names: those whose tree is drawn as a model.
    pub fn seen(&self, scale: f32, fade: f32, hidden: &[u32]) -> Self {
        let mut records = Arc::clone(&self.records);
        if !hidden.is_empty() {
            let mut less = records.to_vec();
            for at in hidden {
                if let Some(record) = less.get_mut(*at as usize) {
                    record[14..16].copy_from_slice(&HIDDEN.to_le_bytes());
                }
            }
            records = less.into();
        }
        Self {
            distance: self.distance * scale,
            fade,
            records,
            ..self.clone()
        }
    }

    /// The places of its instances that a model drawn stands at, by `standing`: a tree drawn as a
    /// model yields its card, and one not drawn keeps it.
    pub fn handed_off(&self, standing: &Standing) -> Vec<u32> {
        if standing.cells.is_empty() || !standing.reaches(self) {
            return Vec::new();
        }
        let (min, size) = (Vec3::from(self.min), Vec3::from(self.size));
        let place = |r: &[u8; 16]| {
            let q = |at: usize| f32::from(u16::from_le_bytes([r[at], r[at + 1]])) / 65535.0;
            min + Vec3::new(q(0), q(2), q(4)) * size
        };
        // ⚠ A plain loop, the bits of the at most four cells a card's room reaches tested first: a
        // look-up a card was most of a selection's time at a city camera.
        let room = Vec3::new(SAME_TREE, SAME_TREE, 0.0);
        let mut out = Vec::new();
        for (at, record) in self.records.iter().enumerate() {
            let p = place(record);
            let ((x0, y0), (x1, y1)) = (cell_of(p - room), cell_of(p + room));
            let near = standing.may_hold((x0, y0))
                || standing.may_hold((x1, y0))
                || standing.may_hold((x0, y1))
                || standing.may_hold((x1, y1));
            if near && standing.stands_at(p) {
                out.push(at as u32);
            }
        }
        out
    }
}

/// Where every instance a selection draws stands, by the cell of the ground it stands in: what
/// a far stand-in yields to.
#[derive(Default)]
pub struct Standing {
    cells: std::collections::HashMap<(i32, i32), Vec<Vec3>>,
    /// The box that holds every place.
    low: Vec3,
    high: Vec3,
    /// A bit a cell of the box's ground, set where a place stands: most cards are let go by it without a look-up.
    taken: Vec<u64>,
    /// The cell the bits start at, and how many cells a row.
    first: (i32, i32),
    columns: i32,
}

/// The side of a cell of `Standing`, in metres.
const CELL: f32 = 8.0;
/// How far off its far stand-in's place a model drawn may stand and still be the same tree: the
/// most a list's box quantises a place by, with room.
const SAME_TREE: f32 = 0.15;
/// How far above or below it, the height quantised by a box much taller than it is wide.
const SAME_TREE_HEIGHT: f32 = 2.0;

fn cell_of(at: Vec3) -> (i32, i32) {
    ((at.x / CELL).floor() as i32, (at.y / CELL).floor() as i32)
}

impl Standing {
    /// The places `places` held for looking up.
    pub fn of(places: impl IntoIterator<Item = Vec3>) -> Self {
        let mut standing = Self {
            low: Vec3::splat(f32::MAX),
            high: Vec3::splat(f32::MIN),
            ..Self::default()
        };
        for at in places {
            standing.cells.entry(cell_of(at)).or_default().push(at);
            standing.low = standing.low.min(at);
            standing.high = standing.high.max(at);
        }
        if standing.cells.is_empty() {
            return standing;
        }
        let (first, last) = (cell_of(standing.low), cell_of(standing.high));
        let span = |a: i32, b: i32| i64::from(b) - i64::from(a) + 1;
        let (columns, rows) = (span(first.0, last.0), span(first.1, last.1));
        // ponytail: past a grid of 2^26 cells (65 km on a side) every card is looked up.
        if columns * rows > 1 << 26 {
            return standing;
        }
        let columns = columns as i32;
        standing.taken = vec![0; (columns as usize * rows as usize).div_ceil(64)];
        (standing.first, standing.columns) = (first, columns);
        for cell in standing.cells.keys() {
            let bit = ((cell.1 - first.1) * columns + cell.0 - first.0) as usize;
            standing.taken[bit / 64] |= 1 << (bit % 64);
        }
        standing
    }

    /// Whether a place may stand in `cell`: false only where the bits say none does.
    fn may_hold(&self, cell: (i32, i32)) -> bool {
        if self.taken.is_empty() {
            return true;
        }
        let (x, y) = (
            cell.0.saturating_sub(self.first.0),
            cell.1.saturating_sub(self.first.1),
        );
        if x < 0 || y < 0 || x >= self.columns {
            return false;
        }
        let bit = (i64::from(y) * i64::from(self.columns) + i64::from(x)) as usize;
        (self.taken.get(bit / 64)).is_some_and(|word| word >> (bit % 64) & 1 == 1)
    }

    /// Whether some place lies inside `list`'s box, grown by the room a place is allowed.
    fn reaches(&self, list: &CardList) -> bool {
        let room = Vec3::new(SAME_TREE, SAME_TREE, SAME_TREE_HEIGHT);
        let (min, max) = (
            Vec3::from(list.min),
            Vec3::from(list.min) + Vec3::from(list.size),
        );
        (self.low - room).cmple(max).all() && (self.high + room).cmpge(min).all()
    }

    /// Whether a place lies at `at`, within the room a far stand-in's place is allowed.
    // ⚠ A selection asks this of every card of every list, over a million at a city camera: the
    // nine cells around each took seconds of the selection, where the cells its room reaches are
    // nearly always its own alone.
    fn stands_at(&self, at: Vec3) -> bool {
        let room = Vec3::new(SAME_TREE, SAME_TREE, 0.0);
        let ((x0, y0), (x1, y1)) = (cell_of(at - room), cell_of(at + room));
        (x0..=x1)
            .flat_map(|cx| (y0..=y1).map(move |cy| (cx, cy)))
            .filter(|c| self.may_hold(*c))
            .filter_map(|c| self.cells.get(&c))
            .flatten()
            .any(|p| {
                (p.x - at.x).hypot(p.y - at.y) <= SAME_TREE
                    && (p.z - at.z).abs() <= SAME_TREE_HEIGHT
            })
    }
}

/// The species id a hidden instance is given: no species table reaches it, so its card draws nothing.
pub const HIDDEN: u16 = u16::MAX;

/// The archetype the far stand-in id `id` names.
fn archetype_of(index: &WorldIndex, id: usize) -> Option<&ArchetypeRow> {
    let word = index.slod.get(id).copied().filter(|w| *w != 0)?;
    index.archetype(word)
}

/// Each far stand-in id's species, indexed by the id.
pub fn species(index: &WorldIndex) -> Arc<[Species]> {
    (0..index.slod.len())
        .map(|id| {
            archetype_of(index, id).map_or_else(Species::default, |a| Species {
                archetype: a.name,
                centre: a.centre,
                radius: a.radius,
            })
        })
        .collect()
}

/// The model that draws the species of id `id`, for baking its atlas; `None` where unsolved or modelless.
pub fn model_of(index: &WorldIndex, id: usize) -> Option<ModelRef> {
    index.model(archetype_of(index, id)?)
}

#[cfg(test)]
#[path = "stream_slod_tests.rs"]
mod tests;
