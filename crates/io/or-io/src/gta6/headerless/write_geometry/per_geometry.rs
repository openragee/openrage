//! A rewritten geometry's row 0 words and row 4 elements in the drawable's per-geometry table at
//! +0xB0: a word is a power of 4 times its channel's area ratio, and an element the extremes of
//! the vertices it holds on its own three axes.
//!
//! ⚠ The power of a word that stands at none, and the axes and parts of a new mesh's elements,
//! come from the tool's source mesh and are not in the file: such a word is refused, and a new
//! mesh takes one enclosing box on the section's axes, a form the build holds, though not the box
//! its tool would choose.

use std::collections::HashMap;

use super::{
    extremes, floats, of_channel, put, sections, Corners, Geometry, GeometryAt, GeometryEdit,
    GeometryWriteError, Written,
};

use GeometryWriteError::Refused;

/// A row 4 element: minimum, maximum and three axes, four floats each.
const ELEMENT: usize = 0x50;
/// How near a stored word lies to a power of 4 times its area ratio for that power to scale it.
const WITHIN: f64 = 0.05;
/// Row 0's word `c` is texcoord channel `FIRST_TEXCOORD + c`'s.
const FIRST_TEXCOORD: u8 = 28;
/// Metres a vertex may lie past a stored element and still be held by it.
const HOLD: f32 = 2e-5;
const UNSCALED: &str = "a row 0 word as read stands within 5 % of no power of 4 times its \
                        texcoord channel's area ratio, and no measured rule gives that word for \
                        new vertices";
const NO_ELEMENT: &str = "a vertex as read lies in no row 4 element of its geometry, so which \
                          element holds it once moved is not known";
const SPLIT_PART: &str = "no row 4 element of the geometry holds a welded part of its vertices \
                          whole, and which element the build gives each of them is not measured";
const EMPTY_ELEMENT: &str = "a row 4 element of the geometry holds none of its vertices as read, \
                             so what the build would store in it is not known";

/// Where the table's rows lie, each its array and count, and the geometry's place among every
/// geometry of the drawable in level order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Table {
    at: usize,
    rows: [(usize, usize); 5],
    place: usize,
}

/// The table of the drawable at `root` where its rows read by their range rule: rows of `{array,
/// u16 count, u16 capacity, u32 0}`; row 3 a `(first, end)` pair per geometry over row 4,
/// contiguous from 0; row 2 one per model; row 0 four words per geometry. `None` otherwise.
pub(super) fn table(v: &[u8], root: usize, at: GeometryAt) -> Option<Table> {
    let read = sections(v, None, |v, _| {
        let Some(table) = v.ptr("per-geometry table", root + 0xB0)? else {
            return Ok(None);
        };
        let (mut geometries, mut models, mut place) = (0, 0, None);
        for level in 0..4 {
            let Some(l) = v.ptr("level", root + 0x50 + 8 * level)? else {
                continue;
            };
            let count = usize::from(v.u16("model count", l + 8)?);
            if count == 0 {
                continue;
            }
            let list = v.to("models", l)?;
            for m in 0..count {
                let model = v.to("model", list + 8 * m)?;
                let n = usize::from(v.u16("geometry count", model + 0x10)?);
                if (level, m) == (at.level, at.model) && at.geometry < n {
                    place = Some(geometries + at.geometry);
                }
                geometries += n;
                models += 1;
            }
        }
        let mut rows = [(0usize, 0usize); 5];
        for (i, row) in rows.iter_mut().enumerate() {
            let r = table + 16 * i;
            let count = v.u16("table row's count", r + 0x08)?;
            if count != v.u16("table row's capacity", r + 0x0A)?
                || v.u32("table row's last word", r + 0x0C)? != 0
            {
                return Ok(None);
            }
            let count = usize::from(count);
            *row = (if count == 0 { 0 } else { v.to("table row", r)? }, count);
        }
        if rows[3].1 != geometries || rows[0].1 != 4 * geometries || rows[2].1 != models {
            return Ok(None);
        }
        let pairs = v.get("table row 3", rows[3].0, 4 * geometries)?;
        let mut end = 0;
        for pair in pairs.as_chunks::<4>().0 {
            let first = usize::from(u16::from_le_bytes([pair[0], pair[1]]));
            let last = usize::from(u16::from_le_bytes([pair[2], pair[3]]));
            if first != end || last < first {
                return Ok(None);
            }
            end = last;
        }
        if end != rows[4].1 {
            return Ok(None);
        }
        v.get("table row 0", rows[0].0, 16 * geometries)?;
        v.get("table row 4", rows[4].0, ELEMENT * end)?;
        Ok(place.map(|place| Table {
            at: table,
            rows,
            place,
        }))
    });
    read.ok().flatten()
}

/// Row 0 and row 4 of the geometry `t` places, `g` as read and `edit` as written over it with
/// `positions` its decoded vertices; whether either was written. A word whose channel and
/// vertices are unchanged keeps what it holds, and so does a channel the geometry does not
/// declare (the build holds 0 there); row 4 is written only where the vertices moved, each
/// element over its own vertices where only they moved, else one box.
pub(super) fn rederive(
    v: &mut [u8],
    t: &Table,
    g: &Geometry,
    edit: &GeometryEdit,
    positions: &[[f32; 3]],
) -> Written<bool> {
    let held = g.positions.as_deref().unwrap_or_default();
    let positions_same = edit.positions == held;
    let triangles_same = edit.triangles == g.triangles;
    let n = positions.len();
    if n != edit.positions.len() {
        return Ok(false);
    }
    let row0 = t.rows[0].0 + 16 * t.place;
    let mut written = false;
    for c in 0..4u8 {
        let channel = FIRST_TEXCOORD + c;
        if !g.channels.iter().any(|k| k.index == channel) {
            continue;
        }
        let was = of_channel(&g.vertices.uvs, channel);
        let uvs = match of_channel(&edit.uvs, channel) {
            given if given.len() == n => given,
            _ if was.len() == n => was,
            _ => continue,
        };
        if positions_same && triangles_same && uvs == was {
            continue;
        }
        let at = row0 + 4 * usize::from(c);
        let stored = f32::from_le_bytes(word(v, at)?);
        let r = ratio(positions, &edit.triangles, uvs).unwrap_or(0.0);
        // A ratio of 0 stores 0.
        let density = match r == 0.0 {
            true => 0.0,
            false => {
                let old = ratio(held, &g.triangles, was);
                r * old
                    .and_then(|old| power(stored, old))
                    .ok_or(Refused(UNSCALED))?
            }
        };
        put(v, at, &(density as f32).to_le_bytes())?;
        written = true;
    }
    if !positions_same {
        let moved_only = triangles_same && n == held.len();
        let boxes = match moved_only {
            true => re_extend(v, t, held, &g.triangles, positions)?,
            false => one_element(v, t, extremes(positions))?,
        };
        written |= boxes;
    }
    Ok(written)
}

/// A stored row 4 element: where it lies, its extents and its three axes.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Element {
    at: usize,
    min: [f32; 3],
    max: [f32; 3],
    axes: [[f32; 3]; 3],
}

impl Element {
    fn on(&self, p: [f32; 3], a: usize) -> f32 {
        (0..3).map(|k| p[k] * self.axes[a][k]).sum()
    }

    fn holds(&self, p: [f32; 3]) -> bool {
        (0..3).all(|a| (self.min[a] - HOLD..=self.max[a] + HOLD).contains(&self.on(p, a)))
    }

    fn volume(&self) -> f32 {
        (0..3).map(|a| self.max[a] - self.min[a]).product()
    }

    /// The extremes of `points` on the element's axes.
    fn extents(&self, points: impl Iterator<Item = [f32; 3]>) -> Corners {
        points.fold(([f32::MAX; 3], [f32::MIN; 3]), |(min, max), p| {
            let d: [f32; 3] = std::array::from_fn(|a| self.on(p, a));
            (
                std::array::from_fn(|a| min[a].min(d[a])),
                std::array::from_fn(|a| max[a].max(d[a])),
            )
        })
    }
}

/// The geometry's run of row 4 elements as stored.
fn run(v: &[u8], t: &Table) -> Written<Vec<Element>> {
    let b = word(v, t.rows[3].0 + 4 * t.place)?;
    let [first, end] = [0, 2].map(|o| usize::from(u16::from_le_bytes([b[o], b[o + 1]])));
    let float = |at: usize| word(v, at).map(f32::from_le_bytes);
    let lanes =
        |at: usize| -> Written<[f32; 3]> { Ok([float(at)?, float(at + 4)?, float(at + 8)?]) };
    (first..end)
        .map(|e| {
            let at = t.rows[4].0 + ELEMENT * e;
            Ok(Element {
                at,
                min: lanes(at)?,
                max: lanes(at + 0x10)?,
                axes: [lanes(at + 0x20)?, lanes(at + 0x30)?, lanes(at + 0x40)?],
            })
        })
        .collect()
}

/// The vertices each element holds: a welded part (its vertices joined by position and by
/// triangle) goes to the smallest element that holds all of it; one no element holds whole is refused.
fn members(
    elements: &[Element],
    held: &[[f32; 3]],
    triangles: &[[u32; 3]],
) -> Result<Vec<Vec<usize>>, &'static str> {
    fn root(up: &mut [usize], mut i: usize) -> usize {
        while up[i] != i {
            up[i] = up[up[i]];
            i = up[i];
        }
        i
    }
    fn join(up: &mut [usize], a: usize, b: usize) {
        let (a, b) = (root(up, a), root(up, b));
        up[a] = b;
    }
    let mut up: Vec<usize> = (0..held.len()).collect();
    let mut at: HashMap<[u32; 3], usize> = HashMap::new();
    for (i, p) in held.iter().enumerate() {
        let j = *at.entry(p.map(f32::to_bits)).or_insert(i);
        join(&mut up, i, j);
    }
    for t in triangles {
        if t.iter().any(|&i| i as usize >= held.len()) {
            return Err("a triangle names a vertex past the vertex count");
        }
        join(&mut up, t[0] as usize, t[1] as usize);
        join(&mut up, t[1] as usize, t[2] as usize);
    }
    let (mut parts, mut part_of) = (Vec::<Vec<usize>>::new(), HashMap::new());
    for i in 0..held.len() {
        let r = root(&mut up, i);
        let k = *part_of.entry(r).or_insert_with(|| {
            parts.push(Vec::new());
            parts.len() - 1
        });
        parts[k].push(i);
    }
    let smallest = |holds: &dyn Fn(&Element) -> bool| {
        (elements.iter().enumerate())
            .filter(|(_, e)| holds(e))
            .min_by(|(_, a), (_, b)| a.volume().total_cmp(&b.volume()))
            .map(|(k, _)| k)
    };
    let mut members = vec![Vec::new(); elements.len()];
    for part in parts {
        let whole = smallest(&|e| part.iter().all(|&i| e.holds(held[i])));
        match whole {
            Some(k) => members[k].extend(part),
            None => {
                let lone = part
                    .iter()
                    .any(|&i| !elements.iter().any(|e| e.holds(held[i])));
                return Err(if lone { NO_ELEMENT } else { SPLIT_PART });
            }
        }
    }
    match members.iter().any(Vec::is_empty) {
        true => Err(EMPTY_ELEMENT),
        false => Ok(members),
    }
}

/// Each element of the geometry's run made the extremes of its vertices, `held` moved to
/// `positions`, on its own axes, which it keeps with every `w`; whether one was written.
fn re_extend(
    v: &mut [u8],
    t: &Table,
    held: &[[f32; 3]],
    triangles: &[[u32; 3]],
    positions: &[[f32; 3]],
) -> Written<bool> {
    let elements = run(v, t)?;
    if elements.is_empty() {
        return Ok(false);
    }
    let members = members(&elements, held, triangles).map_err(Refused)?;
    for (e, m) in elements.iter().zip(&members) {
        let (min, max) = e.extents(m.iter().map(|&i| positions[i]));
        put(v, e.at, &floats(&min))?;
        put(v, e.at + 0x10, &floats(&max))?;
    }
    Ok(true)
}

fn word(v: &[u8], at: usize) -> Written<[u8; 4]> {
    (v.get(at..at + 4))
        .and_then(|b| b.try_into().ok())
        .ok_or(Refused("a table row lies past the virtual section"))
}

/// Σ triangle world area over Σ texcoord area, 0 where either is 0; `None` where a triangle
/// names a vertex past the values.
fn ratio(positions: &[[f32; 3]], triangles: &[[u32; 3]], uvs: &[[f32; 2]]) -> Option<f64> {
    let (mut world, mut texture) = (0.0f64, 0.0f64);
    for t in triangles {
        let p = |i: usize| {
            positions
                .get(t[i] as usize)
                .copied()
                .map(|p| p.map(f64::from))
        };
        let u = |i: usize| uvs.get(t[i] as usize).copied().map(|u| u.map(f64::from));
        let (a, b, c) = (p(0)?, p(1)?, p(2)?);
        let (e, f) = (sub(b, a), sub(c, a));
        let cross = [
            e[1] * f[2] - e[2] * f[1],
            e[2] * f[0] - e[0] * f[2],
            e[0] * f[1] - e[1] * f[0],
        ];
        world += cross.iter().map(|x| x * x).sum::<f64>().sqrt();
        let (a, b, c) = (u(0)?, u(1)?, u(2)?);
        texture += ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs();
    }
    Some(match world == 0.0 || texture == 0.0 {
        true => 0.0,
        false => world / texture,
    })
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|k| a[k] - b[k])
}

/// The power of 4 that brings `stored` within 5 % of `r` times it, if one does.
fn power(stored: f32, r: f64) -> Option<f64> {
    let stored = f64::from(stored);
    if !(stored > 0.0 && r > 0.0) {
        return None;
    }
    let s = 4f64.powi((stored / r).log(4.0).round() as i32);
    ((stored / (r * s) - 1.0).abs() <= WITHIN).then_some(s)
}

/// A row 4 element of `corners` on the section's own axes, every `w` 0: a form the build holds,
/// not the box its tool would choose for a new mesh.
fn element((min, max): Corners) -> Vec<u8> {
    floats(&[
        min[0], min[1], min[2], 0.0, max[0], max[1], max[2], 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
    ])
}

/// The geometry's row 4 run made the one element of `corners`: the elements after it move down
/// over the ones it drops, row 3's later pairs follow them, and row 4's count and capacity
/// shrink; whether it was written. A run of no element is left, since one more needs new room.
fn one_element(v: &mut [u8], t: &Table, corners: Corners) -> Written<bool> {
    let ((row3, geometries), (row4, count)) = (t.rows[3], t.rows[4]);
    let pair = |v: &[u8], k: usize| -> Written<[usize; 2]> {
        let b = word(v, row3 + 4 * k)?;
        Ok([0, 2].map(|o| usize::from(u16::from_le_bytes([b[o], b[o + 1]]))))
    };
    let [first, end] = pair(v, t.place)?;
    if end == first {
        return Ok(false);
    }
    let dropped = end - first - 1;
    let elements = (v.get_mut(row4..row4 + ELEMENT * count))
        .ok_or(Refused("a table row lies past the virtual section"))?;
    elements[ELEMENT * first..ELEMENT * (first + 1)].copy_from_slice(&element(corners));
    if dropped == 0 {
        return Ok(true);
    }
    elements.copy_within(ELEMENT * end.., ELEMENT * (first + 1));
    elements[ELEMENT * (count - dropped)..].fill(0);
    for k in t.place..geometries {
        let [a, b] = pair(v, k)?;
        let a = if k == t.place { a } else { a - dropped };
        let pair = [a as u16, (b - dropped) as u16];
        put(v, row3 + 4 * k, &pair.map(u16::to_le_bytes).concat())?;
    }
    let left = ((count - dropped) as u16).to_le_bytes();
    let r = t.at + 16 * 4;
    put(v, r + 0x08, &left)?;
    put(v, r + 0x0A, &left)?;
    Ok(true)
}

#[cfg(test)]
#[path = "per_geometry_build_tests.rs"]
mod build_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_takes_the_power_of_4_that_brings_it_within_5_percent_else_none() {
        assert_eq!(power(16.0 * 1.04, 1.0), Some(16.0));
        assert_eq!(power(0.125 * 0.97, 2.0), Some(0.0625));
        assert_eq!(power(16.0 * 1.06, 1.0), None);
        assert_eq!(power(2.0, 1.0), None, "2 is no power of 4 within 5 %");
        assert_eq!(power(0.0, 1.0), None);
        assert_eq!(power(5.0, 0.0), None);
        assert_eq!(power(f32::NAN, 1.0), None);
    }

    /// Axes turned 45 degrees about z.
    const TURNED: [[f32; 3]; 3] = [
        [
            std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
            0.0,
        ],
        [
            -std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
            0.0,
        ],
        [0.0, 0.0, 1.0],
    ];

    fn element_of(at: usize, axes: [[f32; 3]; 3], points: &[[f32; 3]]) -> Element {
        let e = Element {
            at,
            min: [0.0; 3],
            max: [0.0; 3],
            axes,
        };
        let (min, max) = e.extents(points.iter().copied());
        Element { min, max, ..e }
    }

    /// Two welded parts, a triangle at the origin and one 10 m along x, and a seventh vertex in no
    /// triangle at the second's position, which only the weld joins to the first part.
    fn two_parts() -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
        let held = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 1.0],
            [10.0, 0.0, 0.0],
            [11.0, 0.0, 0.0],
            [10.0, 1.0, 1.0],
            [1.0, 0.0, 0.0],
        ];
        (held, vec![[0, 1, 2], [3, 4, 5]])
    }

    const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    #[test]
    fn a_welded_part_goes_to_the_smallest_element_that_holds_it_whole() {
        let (held, triangles) = two_parts();
        let near = element_of(0, TURNED, &held[..3]);
        let far = element_of(0x50, IDENTITY, &held[3..6]);
        let held_by = members(&[near, far], &held, &triangles);
        assert_eq!(held_by, Ok(vec![vec![0, 1, 2, 6], vec![3, 4, 5]]));
        // An element round both parts is larger than either, so it is left with none.
        let all = element_of(0xA0, TURNED, &held);
        assert_eq!(
            members(&[all, near, far], &held, &triangles),
            Err(EMPTY_ELEMENT)
        );
        let mut past = held.clone();
        past.push([50.0, 0.0, 0.0]);
        assert_eq!(members(&[near, far], &past, &triangles), Err(NO_ELEMENT));
        // A triangle joining the two parts leaves the joined part in no one element whole.
        let joined = [triangles.clone(), vec![[0, 3, 4]]].concat();
        assert_eq!(members(&[near, far], &held, &joined), Err(SPLIT_PART));
    }

    #[test]
    fn moved_vertices_re_extend_each_element_on_its_own_axes_and_keep_its_axes_and_w() {
        let (held, triangles) = two_parts();
        let (row3, row4) = (0x100, 0x200);
        let mut v = vec![0u8; row4 + 2 * ELEMENT];
        v[row3 + 2..row3 + 4].copy_from_slice(&2u16.to_le_bytes());
        let stored = [
            element_of(row4, TURNED, &held[..3]),
            element_of(row4 + ELEMENT, IDENTITY, &held[3..6]),
        ];
        for e in &stored {
            // Axis 1's `w` 0.5, as the build holds a value there whose meaning is not known.
            let [x, y, z] = e.axes;
            let lanes: Vec<f32> = ([e.min, e.max, x, y, z].iter())
                .zip([0.0, 0.0, 0.0, 0.5, 0.0])
                .flat_map(|(l, w)| [l[0], l[1], l[2], w])
                .collect();
            v[e.at..e.at + ELEMENT].copy_from_slice(&floats(&lanes));
        }
        let before = v.clone();
        let t = Table {
            at: 0,
            rows: [(0, 4), (0, 1), (0, 1), (row3, 1), (row4, 2)],
            place: 0,
        };
        let mut moved = held.clone();
        (moved[1], moved[6]) = ([2.0, 0.0, 0.0], [2.0, 0.0, 0.0]);
        moved[4][1] = 3.0;
        assert!(re_extend(&mut v, &t, &held, &triangles, &moved).expect("written"));
        for (e, points) in stored.iter().zip([&moved[..3], &moved[3..6]]) {
            let (min, max) = e.extents(points.iter().copied());
            let read =
                |o: usize| f32::from_le_bytes(v[e.at + o..e.at + o + 4].try_into().expect("4"));
            assert_eq!([read(0), read(4), read(8)], min);
            assert_eq!([read(0x10), read(0x14), read(0x18)], max);
            assert_eq!(
                v[e.at + 0x0C..e.at + 0x10],
                before[e.at + 0x0C..e.at + 0x10]
            );
            assert_eq!(
                v[e.at + 0x1C..e.at + ELEMENT],
                before[e.at + 0x1C..e.at + ELEMENT]
            );
        }
        assert_ne!(v, before);
    }

    #[test]
    fn the_ratio_is_world_area_over_texcoord_area_and_0_where_either_is_0() {
        let square = [
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, 2.0, 0.0],
            [0.0, 2.0, 0.0],
        ];
        let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let triangles = [[0, 1, 2], [0, 2, 3]];
        assert_eq!(ratio(&square, &triangles, &uvs), Some(4.0));
        assert_eq!(ratio(&square, &triangles, &[[0.5, 0.5]; 4]), Some(0.0));
        assert_eq!(ratio(&square, &[[0, 1, 4]], &uvs), None);
    }

    /// Three geometries' runs of 1, 3 and 2 elements, each element's first float its index.
    fn three_runs() -> (Vec<u8>, Table) {
        let (row3, row4, at) = (0x100, 0x200, 0x0);
        let mut v = vec![0u8; row4 + ELEMENT * 6];
        for (k, pair) in [[0u16, 1], [1, 4], [4, 6]].into_iter().enumerate() {
            v[row3 + 4 * k..row3 + 4 * k + 4].copy_from_slice(&pair.map(u16::to_le_bytes).concat());
        }
        for e in 0..6 {
            let x = (e as f32).to_le_bytes();
            v[row4 + ELEMENT * e..row4 + ELEMENT * e + 4].copy_from_slice(&x);
        }
        for o in [0x08, 0x0A] {
            v[at + 16 * 4 + o..at + 16 * 4 + o + 2].copy_from_slice(&6u16.to_le_bytes());
        }
        let rows = [(0, 12), (0, 1), (0, 1), (row3, 3), (row4, 6)];
        (v, Table { at, rows, place: 1 })
    }

    #[test]
    fn a_run_of_three_becomes_one_box_and_the_later_run_and_its_range_move_down() {
        let (mut v, t) = three_runs();
        let corners = ([-1.0, -2.0, -3.0], [1.0, 2.0, 3.0]);
        assert!(one_element(&mut v, &t, corners).expect("written"));
        let u16_at = |at: usize| u16::from_le_bytes([v[at], v[at + 1]]);
        let pairs: Vec<u16> = (0..6).map(|k| u16_at(0x100 + 2 * k)).collect();
        assert_eq!(pairs, [0, 1, 1, 2, 2, 4]);
        assert_eq!((u16_at(16 * 4 + 8), u16_at(16 * 4 + 0x0A)), (4, 4));
        let element_at = |e: usize| &v[0x200 + ELEMENT * e..0x200 + ELEMENT * (e + 1)];
        assert_eq!(element_at(1), element(corners).as_slice());
        let first = |e: usize| f32::from_le_bytes(element_at(e)[..4].try_into().expect("4"));
        assert_eq!([first(0), first(2), first(3)], [0.0, 4.0, 5.0]);
        assert!(element_at(4).iter().chain(element_at(5)).all(|b| *b == 0));
    }
}
