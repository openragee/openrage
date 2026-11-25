//! A ped cloth record: a shared head whose type byte picks one of five roots, each skinned guide
//! strands over a root mesh, simulation vertices and edges, shapes under part-name keys, one box
//! collider, or elements and index pairs.

use super::bytes::{f32_at, sections, u32_at, u64_at, Bytes};
use super::{HeaderlessError, Result};

/// The owner words a cloth head holds at 0x1C, whose names are not known.
pub const OWNERS: [u32; 4] = [0, 0x7D8F_3DB3, 0xE8A2_A53C, 0xCB1B_5FDE];
const SHAPE: usize = 0x90;
const GROUP: usize = 0x190;
const EMPTY: u8 = 0x80;

#[derive(Debug, Clone, PartialEq)]
pub struct Cloth {
    /// The byte at 0x18: 0, 1 or 2.
    pub flag: u8,
    /// The bytes at 0x19 and 0x1B, whose meanings are not known.
    pub bytes: [u8; 2],
    pub owner: u32,
    /// Where the record the head points at from 0x10 lies; its layout was not measured.
    pub record: Option<u64>,
    pub kind: ClothKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClothKind {
    /// Type 1: elements whose w is positive, and records of two element indices.
    Elements {
        elements: Vec<[f32; 4]>,
        records: Vec<ClothEdge>,
    },
    /// Type 2: vertices, and edges whose length is in most their vertices' distance.
    Edges {
        vertices: Vec<[f32; 4]>,
        edges: Vec<ClothEdge>,
    },
    /// Type 3: guide strands skinned to ped bones, and a triangle list over their roots.
    Mesh(Box<ClothMesh>),
    /// Type 5: shapes, and the lists of them each part-name key names.
    Shapes {
        shapes: Vec<ClothShape>,
        keys: Vec<(u32, Vec<u16>)>,
    },
    /// Type 4: one box in arrays with room for 16; the record after them is not laid out.
    Collider {
        rotation: [f32; 4],
        half_extents: [f32; 3],
        radius: f32,
    },
}

/// A 16-byte record of two indices, two floats and a word.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClothEdge {
    /// Equal only in a filler record.
    pub indices: [u16; 2],
    /// A type-2 edge's rest length; 1.0 in every type-1 record.
    pub length: f32,
    pub weight: f32,
    pub word: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClothMesh {
    pub box_min: [f32; 3],
    pub box_max: [f32; 3],
    /// The name hashes of the ped bones it is skinned to.
    pub bone_hashes: Vec<u32>,
    /// The 12-byte records at 0xB0, whose words are not known.
    pub records: usize,
    /// Its strands, whose roots the triangle list joins.
    pub vertex_count: u16,
    /// The physical buffer's offsets of its per-strand arrays, 0 for none.
    pub vertex_arrays: [u32; 11],
    /// The physical buffer's offsets of its index arrays, 0 for none; the fifth is the triangle list.
    pub index_arrays: [u32; 8],
    pub triangles: Vec<[u16; 3]>,
    /// Each strand's points from its root, strand after strand; w is not known.
    pub points: Vec<[f32; 4]>,
    pub points_per_strand: usize,
    /// Each strand's eight indices into `bone_hashes`.
    pub bones: Vec<[u8; 8]>,
    /// Each strand's eight weights out of about 255, heaviest first.
    pub weights: Vec<[u8; 8]>,
    /// The triangle each strand's root lies on.
    pub root_triangles: Vec<u32>,
    pub rest_lengths: Vec<f32>,
    /// Each strand's two float16 of per-strand array 2, read as its root's UV.
    pub root_uvs: Vec<[f32; 2]>,
    /// Each triangle's four bytes of index array 6. The first three rise and fall together from
    /// triangle to triangle; which of them counts strands no body states.
    pub triangle_bytes: Vec<[u8; 4]>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClothShape {
    pub offset: [f32; 4],
    pub rotation: [f32; 4],
    /// A ped bone's name hash, or 0.
    pub bone_hash: u32,
    pub half_extents: [f32; 3],
    /// A small integer, the shape's kind by its place.
    pub kind: u32,
    pub box_min: [f32; 3],
    pub box_max: [f32; 3],
}

fn refuse<T>(why: &'static str) -> Result<T> {
    Err(HeaderlessError::Cloth(why))
}

pub fn read_cloth(virtual_section: &[u8], physical: Option<&[u8]>) -> Result<Cloth> {
    sections(virtual_section, physical, cloth)
}

fn cloth(v: Bytes, p: Bytes) -> Result<Cloth> {
    let head = v.get("cloth head", 0, 0x20)?;
    let (flag, kind, owner) = (head[0x18], head[0x1A], u32_at(head, 0x1C));
    let root = match kind {
        1 => 0x240,
        2 => 0x7A0,
        3 => 0x140,
        5 => 0x50,
        4 => 0xC0,
        _ => return refuse("its type byte is none of the cloth types"),
    };
    let map = v.to("block map", 0x08)?;
    let blocks = v.get("block map", map, 0x10)?;
    let zero = |r: &[u8]| r.iter().all(|b| *b == 0);
    if u64_at(head, 0) != 0 || !zero(&blocks[..8]) || !zero(&blocks[10..]) || blocks[8] == 0 {
        return refuse("its first word or its block map is not the measured one");
    }
    if blocks[9] != u8::from(kind == 3) || map < root || v.len() < root + 0x10 {
        return refuse(
            "its block map counts physical blocks other than its type has, or lies in its root",
        );
    }
    if flag > 2 || !OWNERS.contains(&owner) {
        return refuse("its flag is past 2, or its owner word is none of the four measured");
    }
    let record = match kind {
        // ⚠ Type 4's word at 0x10 and its flag were not measured together.
        4 => v.ptr("record at 0x10", 0x10).ok().flatten(),
        _ => v.ptr("record at 0x10", 0x10)?,
    };
    if !matches!(kind, 3 | 4) && record.is_some() != (flag != 0) {
        return refuse(
            "its record at 0x10 is set where its flag is clear, or null where it is set",
        );
    }
    if kind != 3 && p.len() > 0 {
        return refuse("it holds a physical section, which only type 3 has");
    }
    Ok(Cloth {
        flag,
        bytes: [head[0x19], head[0x1B]],
        owner,
        record: record.map(|r| r as u64),
        kind: match kind {
            1 => elements(v)?,
            2 => edges(v)?,
            3 => ClothKind::Mesh(Box::new(mesh(v, p)?)),
            4 => collider(v)?,
            _ => shapes(v)?,
        },
    })
}

/// The array of 16-byte elements at `at`, {pointer, u16 count, u16 capacity, u32 0}, past the root.
fn array16<'a>(v: Bytes<'a>, at: usize, root: usize) -> Result<Vec<&'a [u8; 16]>> {
    let (count, capacity) = (v.u16("count", at + 8)?, v.u16("capacity", at + 10)?);
    let target = v.ptr("array", at)?;
    if count != capacity || target.is_none() != (count == 0) || v.u32("array pad", at + 12)? != 0 {
        return refuse(
            "an array's count and capacity differ, or its pointer is set for no element",
        );
    }
    let Some(target) = target else {
        return Ok(Vec::new());
    };
    if target < root {
        return refuse("an array lies in its root");
    }
    Ok(v.array("array", target, count.into(), 16)?
        .as_chunks::<16>()
        .0
        .iter()
        .collect())
}

fn edge(r: &[u8; 16]) -> ClothEdge {
    ClothEdge {
        indices: [
            u16::from_le_bytes([r[0], r[1]]),
            u16::from_le_bytes([r[2], r[3]]),
        ],
        length: f32_at(r, 4),
        weight: f32_at(r, 8),
        word: u32_at(r, 12),
    }
}

fn vec4(r: &[u8]) -> [f32; 4] {
    std::array::from_fn(|k| f32_at(r, 4 * k))
}

fn elements(v: Bytes) -> Result<ClothKind> {
    let elements: Vec<[f32; 4]> = array16(v, 0x28, 0x240)?
        .into_iter()
        .map(|r| vec4(r))
        .collect();
    let records: Vec<ClothEdge> = array16(v, 0x38, 0x240)?.into_iter().map(edge).collect();
    if elements.iter().any(|e| !e[3].is_finite() || e[3] <= 0.0) {
        return refuse("a type-1 element's w is not a positive finite float");
    }
    let n = elements.len();
    let bad = |r: &ClothEdge| {
        let [i, j] = r.indices.map(usize::from);
        r.length != 1.0 || (i != j && (i >= n || j >= n))
    };
    if records.iter().any(bad) {
        return refuse(
            "a type-1 record's float at +4 is not 1.0, or it names an element past the count",
        );
    }
    Ok(ClothKind::Elements { elements, records })
}

fn edges(v: Bytes) -> Result<ClothKind> {
    let vertices: Vec<[f32; 4]> = array16(v, 0x28, 0x7A0)?
        .into_iter()
        .map(|r| vec4(r))
        .collect();
    let edges: Vec<ClothEdge> = array16(v, 0x38, 0x7A0)?.into_iter().map(edge).collect();
    let n = vertices.len();
    let holds = |e: &ClothEdge| {
        let [i, j] = e.indices.map(usize::from);
        let filler = (i, j, e.length, e.weight, e.word) == (0, 0, 1.0, 0.0, 0);
        filler || (i != j && i < n && j < n && e.length.is_finite() && e.length >= 0.0)
    };
    if !edges.iter().all(holds) {
        return refuse("a type-2 edge names a vertex past the count, or is neither an edge nor the filler record");
    }
    Ok(ClothKind::Edges { vertices, edges })
}

fn mesh(v: Bytes, p: Bytes) -> Result<ClothMesh> {
    let (box_min, box_max) = (v.vec3("box minimum", 0x30)?, v.vec3("box maximum", 0x40)?);
    let finite = box_min.iter().chain(&box_max).all(|x| x.is_finite());
    if !finite || (0..3).any(|a| box_min[a] > box_max[a]) || v.u64("word at 0x90", 0x90)? != 0 {
        return refuse(
            "its box is not finite with its minimum below its maximum, or its word at 0x90 is set",
        );
    }
    let buffer = v.to("raw buffer", 0x70)?;
    let o = v.get("raw buffer", buffer, 0x50)?;
    let stated = (
        u16::from_le_bytes([o[0xC], o[0xD]]),
        u16::from_le_bytes([o[0xE], o[0xF]]),
    );
    if stated != (4, 8) || u32_at(o, 0x10) != 0x1380 || u64_at(o, 0x18) != 0x6000_0000 {
        return refuse("the record at 0x70 is not the raw buffer at the physical section's start");
    }
    let bytes = 4 * u32_at(o, 8) as usize;
    let whole = p.get("raw buffer", 0, p.len())?;
    let fills = bytes <= whole.len()
        && (whole.len() - bytes < 16 || whole.len() % 0x1_0000 == 0)
        && whole[bytes..].iter().all(|b| *b == 0);
    if !fills {
        return refuse("its raw buffer does not fill the physical section to its pad");
    }
    let (count, capacity) = (
        v.u32("bone hash count", 0x98)?,
        v.u32("bone hash capacity", 0x9C)?,
    );
    if count != capacity || count == 0 {
        return refuse("its bone hashes' count and capacity differ, or it has none");
    }
    let hashes = v.array("bone hashes", v.to("bone hashes", 0xA0)?, count as usize, 4)?;
    let bone_hashes: Vec<u32> = (0..count as usize).map(|k| u32_at(hashes, 4 * k)).collect();
    let mut sorted = bone_hashes.clone();
    sorted.sort_unstable();
    sorted.dedup();
    let (records, again) = (
        v.u32("record count", 0xA8)?,
        v.u32("record capacity", 0xAC)?,
    );
    if sorted.len() != bone_hashes.len() || records != again {
        return refuse("a bone hash repeats, or its records' count and capacity differ");
    }
    v.array(
        "12-byte records",
        v.to("12-byte records", 0xB0)?,
        records as usize,
        12,
    )?;
    let head = p.get("physical tables", 0, 0x60)?;
    let vertex_arrays: [u32; 11] = std::array::from_fn(|k| u32_at(head, 4 * k));
    let index_arrays: [u32; 8] = std::array::from_fn(|k| u32_at(head, 0x30 + 4 * k));
    let n = u16::from_le_bytes([head[0x58], head[0x59]]);
    let stated_triangles = u16::from_le_bytes([head[0x5A], head[0x5B]]);
    let mut offsets: Vec<usize> = (vertex_arrays.iter().chain(&index_arrays))
        .filter(|o| **o != 0)
        .map(|o| *o as usize)
        .chain([bytes])
        .collect();
    offsets.sort_unstable();
    offsets.dedup();
    if n == 0 || offsets.iter().any(|o| o % 4 != 0 || *o > bytes) {
        return refuse("it has no vertex, or an array's offset is not 4-aligned inside its buffer");
    }
    // An array spans to the next array's start, or to the buffer's end.
    let span = |at: usize| offsets.iter().find(|o| **o > at).map_or(0, |end| end - at);
    for (slot, size) in [(1, 4), (2, 4), (5, 8)] {
        let at = vertex_arrays[slot] as usize;
        if at == 0 || span(at) < size * usize::from(n) {
            return refuse(
                "its per-vertex arrays 1, 2 and 5 do not hold 4, 4 and 8 bytes a vertex",
            );
        }
        let words = p.get("per-vertex array", at, size * usize::from(n))?;
        if words
            .as_chunks::<4>()
            .0
            .iter()
            .any(|w| !f32::from_le_bytes(*w).is_finite())
        {
            return refuse("a per-vertex array 1, 2 or 5 holds a word that is no finite float");
        }
    }
    let list = index_arrays[4] as usize;
    if list == 0 {
        return refuse("it has no triangle list");
    }
    let raw = p.get("triangle list", list, span(list) & !1)?;
    let mut indices: Vec<u16> = raw
        .as_chunks::<2>()
        .0
        .iter()
        .map(|w| u16::from_le_bytes(*w))
        .collect();
    while !indices.len().is_multiple_of(3) && indices.last() == Some(&0) {
        indices.pop();
    }
    let triangles: Vec<[u16; 3]> = indices.as_chunks::<3>().0.to_vec();
    let below =
        |t: &[u16; 3]| t.iter().all(|i| *i < n) && t[0] != t[1] && t[1] != t[2] && t[0] != t[2];
    if !indices.len().is_multiple_of(3) || triangles.is_empty() || !triangles.iter().all(below) {
        return refuse(
            "its triangle list is not whole triangles of three distinct vertices below its count",
        );
    }
    if usize::from(stated_triangles) != triangles.len() {
        return refuse("the triangle count it states is not its list's");
    }
    let (strands, m) = (usize::from(n), triangles.len());
    let spans = |at: u32| match at {
        0 => 0,
        at => span(at as usize),
    };
    let half = |h: &[u8; 2]| crate::half::half_to_f32(u16::from_le_bytes(*h));
    let whole_points = spans(index_arrays[5]);
    let per = whole_points / (8 * strands);
    if !whole_points.is_multiple_of(8 * strands) || per < 2 {
        return refuse("its index array 5 is not K points of 8 bytes a strand, K past 1");
    }
    let raw = p.get("strand points", index_arrays[5] as usize, whole_points)?;
    let points: Vec<[f32; 4]> = (raw.as_chunks::<8>().0.iter())
        .map(|r| std::array::from_fn(|k| half(&[r[2 * k], r[2 * k + 1]])))
        .collect();
    // A root may lie up to 2 mm outside the box.
    let inside =
        |q: &[f32; 4]| (0..3).all(|a| (box_min[a] - 2e-3..=box_max[a] + 2e-3).contains(&q[a]));
    let finite = points.iter().flatten().all(|x| x.is_finite());
    if !finite || !points.iter().step_by(per).all(inside) {
        return refuse("a strand point is not finite, or a strand's root is outside its box");
    }
    if spans(vertex_arrays[4]) < 8 * strands {
        return refuse("its per-strand array 4 does not hold 8 bone indices a strand");
    }
    let eights = |what: &'static str, at: u32| -> Result<Vec<[u8; 8]>> {
        let eight = p.get(what, at as usize, 8 * strands)?;
        Ok(eight.as_chunks::<8>().0.to_vec())
    };
    let bones = eights("bone indices", vertex_arrays[4])?;
    let weights = eights("skin weights", vertex_arrays[5])?;
    let skinned = |(b, w): (&[u8; 8], &[u8; 8])| {
        let sum: u32 = w.iter().map(|x| u32::from(*x)).sum();
        w.windows(2).all(|pair| pair[0] >= pair[1])
            && (252..=258).contains(&sum)
            && (0..8).all(|k| w[k] == 0 || usize::from(b[k]) < bone_hashes.len())
    };
    if !bones.iter().zip(&weights).all(skinned) {
        return refuse("a strand's weights do not descend to a sum of 252 to 258 over its bones");
    }
    let words = p.get("root triangles", vertex_arrays[1] as usize, 4 * strands)?;
    let root_triangles: Vec<u32> = (words.as_chunks::<4>().0.iter())
        .map(|w| u32::from_le_bytes(*w))
        .collect();
    if root_triangles.iter().any(|t| *t as usize >= m) {
        return refuse("a strand's root triangle, per-strand array 1, is past the count");
    }
    if spans(index_arrays[0]) < 2 * strands {
        return refuse("its index array 0 does not hold a rest length a strand");
    }
    let lengths = p.get("rest lengths", index_arrays[0] as usize, 2 * strands)?;
    let rest_lengths: Vec<f32> = lengths.as_chunks::<2>().0.iter().map(half).collect();
    let (six, seven) = (spans(index_arrays[6]), spans(index_arrays[7]));
    if six != 4 * m || (seven != m && seven != m.next_multiple_of(4)) {
        return refuse("its index arrays 6 and 7 do not hold 4 bytes and 1 byte a triangle");
    }
    let uvs = p.get("root UVs", vertex_arrays[2] as usize, 4 * strands)?;
    let root_uvs: Vec<[f32; 2]> = (uvs.as_chunks::<4>().0.iter())
        .map(|r| [half(&[r[0], r[1]]), half(&[r[2], r[3]])])
        .collect();
    let bytes = p.get("triangle bytes", index_arrays[6] as usize, six)?;
    let triangle_bytes = bytes.as_chunks::<4>().0.to_vec();
    Ok(ClothMesh {
        box_min,
        box_max,
        bone_hashes,
        records: records as usize,
        vertex_count: n,
        vertex_arrays,
        index_arrays,
        triangles,
        points,
        points_per_strand: per,
        bones,
        weights,
        root_triangles,
        rest_lengths,
        root_uvs,
        triangle_bytes,
    })
}

/// Type 4: one shape and its extents, each in an array with room for 16, and a record after them.
fn collider(v: Bytes) -> Result<ClothKind> {
    let stated = (
        v.u16("shape count", 0x30)?,
        v.u16("shape capacity", 0x32)?,
        v.u16("extent count", 0x40)?,
        v.u16("extent capacity", 0x42)?,
    );
    if v.u32("word at 0x20", 0x20)? != u32::MAX || stated != (1, 16, 1, 16) {
        return refuse("its word at 0x20 is not all ones, or an array not 1 element in 16");
    }
    let (shape_at, extents_at) = (v.to("shape", 0x28)?, v.to("extents", 0x38)?);
    let map = v.to("block map", 0x08)?;
    if shape_at < 0xC0 || extents_at < shape_at + 16 * 64 {
        return refuse("its shapes lie in its root, or its extents in the shapes' room");
    }
    let shape = v.array("shapes", shape_at, 16, 64)?;
    let extents = v.array("extents", extents_at, 16, 16)?;
    let rotation = vec4(&shape[0x10..]);
    let length = rotation.iter().map(|x| x * x).sum::<f32>().sqrt();
    if !length.is_finite() || (length - 1.0).abs() >= 1e-3 {
        return refuse("its shape's rotation is not a unit quaternion");
    }
    let half_extents: [f32; 3] = std::array::from_fn(|k| f32_at(extents, 4 * k));
    let (lo, hi) = (v.vec3("box minimum", 0x90)?, v.vec3("box maximum", 0xA0)?);
    let agree = |k: usize| (half_extents[k] - hi[k]).abs() < 1e-6 && (lo[k] + hi[k]).abs() < 1e-6;
    if !(0..3).all(agree) || u32_at(extents, 12) != 6 {
        return refuse("its half extents are not its box's, or their kind is not 6");
    }
    let holds = |at: usize| -> Option<usize> {
        let ones = (
            v.u32("record item", at).ok()?,
            v.u32("record item", at + 4).ok()?,
        ) == (1, 1);
        ones.then(|| v.ptr("record", at + 8).ok().flatten())?
    };
    let found: Vec<usize> = [0x70, 0x80].into_iter().filter_map(holds).collect();
    let [record] = found[..] else {
        return refuse("not exactly one of its slots at 0x70 and 0x80 points at its record");
    };
    if record < extents_at + 16 * 16 || record >= map {
        return refuse("its record does not lie between its extents' room and its block map");
    }
    v.get("record", record, 0x70)?;
    Ok(ClothKind::Collider {
        rotation,
        half_extents,
        radius: f32_at(shape, 0x0C),
    })
}

fn shapes(v: Bytes) -> Result<ClothKind> {
    let (count, capacity) = (v.u32("shape count", 0x28)?, v.u32("shape capacity", 0x2C)?);
    let at = v.ptr("shapes", 0x30)?;
    if count != capacity || at.is_none() != (count == 0) {
        return refuse("its shapes' count and capacity differ, or its pointer is set for none");
    }
    let records = match at {
        Some(at) => v.array("shapes", at, count as usize, SHAPE)?,
        None => &[],
    };
    let shapes: Vec<ClothShape> = (records.as_chunks::<SHAPE>().0.iter())
        .map(|r| ClothShape {
            offset: vec4(r),
            rotation: vec4(&r[0x10..]),
            bone_hash: u32_at(r, 0x28),
            half_extents: std::array::from_fn(|k| f32_at(r, 0x50 + 4 * k)),
            kind: u32_at(r, 0x5C),
            box_min: std::array::from_fn(|k| f32_at(r, 0x60 + 4 * k)),
            box_max: std::array::from_fn(|k| f32_at(r, 0x70 + 4 * k)),
        })
        .collect();
    if shapes
        .iter()
        .any(|s| !s.rotation.iter().map(|x| x * x).sum::<f32>().is_finite())
    {
        return refuse("a shape's rotation is not finite");
    }
    let (groups, stated) = (
        v.u32("key groups", 0x38)? as usize,
        v.u32("key count", 0x3C)? as usize,
    );
    let table_at = v.ptr("key table", 0x40)?;
    if table_at.is_none() != (groups == 0) || (groups == 0) != (stated == 0) {
        return refuse("its key table, group count and key count are not all zero or all set");
    }
    if groups & groups.wrapping_sub(1) != 0 || stated > 16 * groups {
        return refuse("its key groups are not a power of two that holds its keys");
    }
    let table = match table_at {
        Some(t) => v.array("key table", t, groups, GROUP)?,
        None => &[],
    };
    let control = |g: usize| &table[GROUP * g..GROUP * g + 16];
    let full: Vec<bool> = (0..groups).map(|g| !control(g).contains(&EMPTY)).collect();
    let full_before = crate::gta6::hash::full_before(&full);
    let mut keys = Vec::with_capacity(stated);
    for (g, &crossed) in full_before.iter().enumerate() {
        for s in 0..16 {
            let slot_at = GROUP * g + 0x10 + 0x18 * s;
            let slot = &table[slot_at..slot_at + 0x18];
            if control(g)[s] == EMPTY {
                if slot.iter().any(|b| *b != 0) {
                    return refuse("an empty slot of its key table is not zero");
                }
                continue;
            }
            let (key, count) = (u32_at(slot, 0), u32_at(slot, 8));
            let home = (key as usize >> 7) & (groups - 1);
            let probed = crossed >= (g + groups - home) % groups;
            if control(g)[s] != (key & 0x7F) as u8 || !probed {
                return refuse("a key's control byte is not its low seven bits, or it sits past a group with room");
            }
            if u32_at(slot, 4) != 0 || count != u32_at(slot, 12) {
                return refuse(
                    "a key's slot has a word set after its key, or two counts that differ",
                );
            }
            let list = v.ptr(
                "key's shapes",
                table_at.unwrap_or_default() + slot_at + 0x10,
            )?;
            if list.is_none() != (count == 0) {
                return refuse("a key's shape list is set for no shape, or null for some");
            }
            let indices: Vec<u16> = match list {
                Some(at) => (v
                    .array("key's shapes", at, count as usize, 2)?
                    .as_chunks::<2>()
                    .0
                    .iter())
                .map(|w| u16::from_le_bytes(*w))
                .collect(),
                None => Vec::new(),
            };
            let ascends = indices.windows(2).all(|w| w[0] <= w[1]);
            if !ascends || indices.iter().any(|i| usize::from(*i) >= shapes.len()) {
                return refuse("a key's shape list does not ascend below the shape count");
            }
            if shapes.iter().any(|s| s.bone_hash == key) {
                return refuse("a key is a shape's bone hash");
            }
            keys.push((key, indices));
        }
    }
    if keys.len() != stated {
        return refuse("its key table holds another number of keys than it states");
    }
    Ok(ClothKind::Shapes { shapes, keys })
}
