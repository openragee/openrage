//! A rewritten geometry's row 0 words and row 4 elements in the drawable's per-geometry table at
//! +0xB0: the four words become the texel densities a measured rule gives, and the elements one box
//! that encloses the vertices.
//!
//! ⚠ Both are approximations. The tool's row 4 elements are oriented boxes over welded parts, and
//! its row 0 words come from the source mesh before quantisation, a texture-sized power of 4 off
//! the area ratio; neither is byte-derivable from the decoded vertices.

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
/// declare (the build holds 0 there); row 4 is written only where the vertices moved.
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
        let power = ratio(held, &g.triangles, was).map_or(1.0, |r| scale(stored, r));
        let r = ratio(positions, &edit.triangles, uvs).unwrap_or(0.0);
        put(v, at, &((r * power) as f32).to_le_bytes())?;
        written = true;
    }
    if !positions_same && one_element(v, t, extremes(positions))? {
        written = true;
    }
    Ok(written)
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

/// The power of 4 that brings `stored` within 5 % of `r` times it; 1 where none does.
fn scale(stored: f32, r: f64) -> f64 {
    let stored = f64::from(stored);
    let positive = stored > 0.0 && r > 0.0;
    if !positive {
        return 1.0;
    }
    let s = 4f64.powi((stored / r).log(4.0).round() as i32);
    match (stored / (r * s) - 1.0).abs() <= WITHIN {
        true => s,
        false => 1.0,
    }
}

/// A row 4 element of `corners` on the section's own axes, every `w` 0.
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
        assert_eq!(scale(16.0 * 1.04, 1.0), 16.0);
        assert_eq!(scale(0.125 * 0.97, 2.0), 0.0625);
        assert_eq!(scale(2.0, 1.0), 1.0, "2 is no power of 4 within 5 %");
        assert_eq!(scale(0.0, 1.0), 1.0);
        assert_eq!(scale(5.0, 0.0), 1.0);
        assert_eq!(scale(f32::NAN, 1.0), 1.0);
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
