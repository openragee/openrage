//! A mesh past the 65,536 vertices a geometry holds, cut by triangle into geometries of one model:
//! the first written over the geometry it replaces, each other a copy of that geometry's objects
//! placed after it, and the model's three arrays and the drawable's per-geometry table laid out
//! anew past the virtual section's end. The arrays they replace stay as unreferenced bytes.
//!
//! ⚠ Each piece's density words are its own, derived as a replaced geometry's are: its world area
//! over its texcoord area, times the power of 4 the replaced geometry's stored word states. Its box
//! elements are one axis-aligned box per piece: it encloses the piece,
//! as every stored element encloses its parts, but it is not the box the game's tool writes.

use std::collections::HashMap;

use super::super::write_material::based;
use super::{
    extremes, floats, locate, place, put, put_box, read_box, rebox, sections, u64_at, union,
    write_geometry, Corners, GeometryAt, GeometryEdit, GeometryWriteError, GeometryWritten,
    HeaderlessError, Located, Written, MAX_VERTICES, VIRTUAL_TAG,
};
use crate::container::rpf::{grow_section, Rpf9Role, Rpf9Section, Rpf9Sections};
use crate::gta6::hash::name_hash64;

use GeometryWriteError::Refused;

/// A new geometry's objects: the geometry, its vertex buffer and declaration, its meshlet record
/// with the vertex buffer's view after it, and its index buffer with its view after it.
const OBJECTS: usize = 0x50 + 0x50 + 0x140 + 0x40 + 0x40 + 0x20;
/// A buffer view's word at +0x10; every other word of a view is zero.
const VIEW: u64 = 0xFFFF_FFFF_FFFF_0214;
/// A box element of the table's row 4: minimum, maximum and three axes, four floats each.
const ELEMENT: usize = 0x50;
/// The most palette entries a cut by vertices alone may keep: the model states its count in one
/// byte at +0x30.
const PALETTE: u16 = 0xFF;

/// What a cut write changed: the first piece's write, and how many geometries the mesh became.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GeometryCut {
    pub written: GeometryWritten,
    pub pieces: usize,
}

/// [`write_geometry`], with a mesh past 65,536 vertices cut into geometries placed from `at` on.
// Each piece after the first takes `at`'s shader and its words at +0x28..+0x2F; a refusal leaves
// the sections as they were.
pub fn write_geometry_cut(
    sections: &mut Rpf9Sections,
    role: Rpf9Role,
    code: Option<u32>,
    at: GeometryAt,
    edit: &GeometryEdit,
) -> Written<GeometryCut> {
    let n = edit.positions.len();
    if n <= MAX_VERTICES {
        let written = write_geometry(sections, role, code, at, edit)?;
        return Ok(GeometryCut { written, pieces: 1 });
    }
    if edit.triangles.iter().flatten().any(|i| *i as usize >= n) {
        return Err(Refused("a triangle names a vertex past the vertex count"));
    }
    if lengths(edit).any(|len| len != 0 && len != n) {
        return Err(Refused(
            "the edit gives a channel another number of values than of positions",
        ));
    }
    let mut pieces = cut(edit);
    if pieces.len() < 2 {
        // The triangles use no more vertices than one geometry holds.
        let one = pieces
            .pop()
            .ok_or(Refused("a geometry holds at least one triangle"))?;
        let written = write_geometry(sections, role, code, at, &one)?;
        return Ok(GeometryCut { written, pieces: 1 });
    }
    let k = pieces.len() - 1;
    let mut out = sections.clone();
    let (_, walked, table) = held(&out, role, code, at)?;
    let geometry = walked.geometry(at)?;
    if palette_of(&out.virtual_section, geometry)? > PALETTE {
        return Err(Refused(
            "a geometry whose palette passes 255 entries is cut by its bones as well, and no such \
             cut was measured: the model states its palette count in one byte at +0x30, and the \
             largest palette the build holds has 209 entries",
        ));
    }
    let q = walked.first(at);
    let total: usize = walked.models.iter().map(Vec::len).sum();
    let elements: usize = table.elements.iter().map(|e| e.len() / ELEMENT).sum();
    let replaced = table.elements.get(q).map_or(0, |e| e.len() / ELEMENT);
    let (n_new, g_new) = (walked.count + k, total + k);
    let e_new = elements - replaced + pieces.len();
    if [n_new, 4 * g_new, e_new]
        .iter()
        .any(|&c| c > usize::from(u16::MAX))
    {
        return Err(Refused(
            "the pieces would pass the 16-bit counts of the model or of the drawable's \
             per-geometry table",
        ));
    }

    // Each further piece written alone over the geometry, for the bytes and density words it then
    // holds.
    let (mut alone, mut own_densities) = (Vec::with_capacity(k), Vec::with_capacity(k));
    for piece in &pieces[1..] {
        let mut scratch = out.clone();
        write_geometry(&mut scratch, role, code, at, piece)?;
        alone.push(written_alone(&scratch, role, code, at)?);
        let (_, _, own) = held(&scratch, role, code, at)?;
        let density = (own.densities.get(q).cloned()).ok_or(Refused(
            "the per-geometry table names no row 0 words of the geometry",
        ))?;
        own_densities.push(density);
    }
    let mut written = write_geometry(&mut out, role, code, at, &pieces[0])?;

    let arrays = [
        (8 * n_new).next_multiple_of(16),
        32 * (n_new + 1),
        (2 * n_new).next_multiple_of(16),
        16 * g_new,
        (4 * g_new).next_multiple_of(16),
        ELEMENT * e_new,
    ];
    let len = OBJECTS * k + arrays.iter().sum::<usize>();
    let end = out.virtual_section.len().next_multiple_of(16);
    out.virtual_section.resize(end, 0);
    let (pointers, by) = (based(&out), len as isize);
    let base = grow_section(&mut out, &pointers, Rpf9Section::Virtual, end, by)?;
    // The block map's slots may have grown before the new bytes, so everything is found again.
    let (found, walked, table) = held(&out, role, code, at)?;
    let geometry = walked.geometry(at)?;
    let v = &out.virtual_section;
    let geometry_bytes = bytes_at(v, geometry, 0x50)?.to_vec();
    let vertex_buffer = bytes_at(v, found.vertex_buffer, 0x50)?.to_vec();
    let index_buffer = bytes_at(v, found.index_buffer, 0x40)?.to_vec();
    let mut digests = (walked.models.iter().flatten())
        .map(|&g| bytes_at(v, g + 0x30, 8).map(|b| u64_at(b, 0)))
        .collect::<Written<Vec<u64>>>()?;

    let mut added = Vec::with_capacity(k);
    for (i, piece) in alone.iter().enumerate() {
        let g = base + OBJECTS * i;
        let (vb, declaration) = (g + 0x50, g + 0xA0);
        let record = declaration + 0x140;
        let ib = record + 0x40;
        // The digest at +0x30..+0x37 is fresh and distinct from every other geometry's.
        let mut digest = name_hash64(&[&piece.vertices[..], &piece.index[..]].concat());
        while digests.contains(&digest) {
            digest = digest.wrapping_add(1);
        }
        digests.push(digest);
        let v = &mut out.virtual_section;
        put(v, g, &geometry_bytes)?;
        put(v, g + 0x08, &pointer(vb))?;
        put(v, g + 0x30, &digest.to_le_bytes())?;
        put(v, g + 0x40, &pointer(record))?;
        put(v, vb, &vertex_buffer)?;
        put(v, vb + 0x08, &words(&piece.vertices))?;
        put(v, vb + 0x18, &[0; 8])?;
        put(v, vb + 0x28, &pointer(record + 0x20))?;
        put(v, vb + 0x40, &pointer(declaration))?;
        put(v, declaration, &piece.declaration)?;
        put(v, record, &pointer(ib))?;
        put(v, record + 0x08, &piece.counts)?;
        put(v, record + 0x30, &VIEW.to_le_bytes())?;
        put(v, ib, &index_buffer)?;
        put(v, ib + 0x08, &words(&piece.index))?;
        put(v, ib + 0x18, &[0; 8])?;
        put(v, ib + 0x28, &pointer(ib + 0x40))?;
        put(v, ib + 0x50, &VIEW.to_le_bytes())?;
        let end = out.physical.len();
        place(&mut out, vb, (end, 0), &piece.vertices)?;
        let end = out.physical.len();
        place(&mut out, ib, (end, 0), &piece.index)?;
        added.push(g);
    }

    // The model's arrays: the pieces at `at` and after it, the union box first.
    let pointers_at = base + OBJECTS * k;
    let boxes_at = pointers_at + arrays[0];
    let shaders_at = boxes_at + arrays[1];
    let row0 = shaders_at + arrays[2];
    let row3 = row0 + arrays[3];
    let row4 = row3 + arrays[4];
    let first = usize::from(walked.count > 1);
    let (mut targets, mut boxes, mut shaders) = (Vec::new(), Vec::new(), Vec::new());
    let held_here = &walked.models[walked.index];
    for (i, &target) in held_here.iter().enumerate() {
        let shader = bytes_at(&out.virtual_section, walked.shaders + 2 * i, 2)?;
        let shader = u16::from_le_bytes([shader[0], shader[1]]);
        if i != at.geometry {
            targets.push(target);
            boxes.push(read_box(
                &out.virtual_section,
                walked.boxes + 32 * (i + first),
            )?);
            shaders.push(shader);
            continue;
        }
        for (piece, target) in pieces
            .iter()
            .zip(std::iter::once(target).chain(added.clone()))
        {
            targets.push(target);
            boxes.push(extremes(&piece.positions));
            shaders.push(shader);
        }
    }
    let v = &mut out.virtual_section;
    for (i, target) in targets.iter().enumerate() {
        put(v, pointers_at + 8 * i, &pointer(*target))?;
    }
    let x = |c: [f32; 3]| c[0];
    put_box(v, boxes_at, union(&boxes), x)?;
    for (i, corners) in boxes.iter().enumerate() {
        put_box(v, boxes_at + 32 * (i + 1), *corners, x)?;
    }
    for (i, shader) in shaders.iter().enumerate() {
        put(v, shaders_at + 2 * i, &shader.to_le_bytes())?;
    }
    for (o, array) in [(0x08, pointers_at), (0x18, boxes_at), (0x20, shaders_at)] {
        put(v, walked.model + o, &pointer(array))?;
    }
    for o in [0x10, 0x12, 0x36] {
        put(v, walked.model + o, &(n_new as u16).to_le_bytes())?;
    }

    // The table: row 0 and row 4 per geometry in model order, row 3 ranges over row 4, and row 2's
    // ranges over row 3 rewritten in place.
    let (mut densities, mut elements) = (Vec::new(), Vec::new());
    for (i, (density, run)) in table.densities.iter().zip(&table.elements).enumerate() {
        match i == q {
            true => {
                let own = std::iter::once(density).chain(&own_densities);
                for (piece, density) in pieces.iter().zip(own) {
                    densities.extend_from_slice(density);
                    elements.push(element(extremes(&piece.positions)));
                }
            }
            false => {
                densities.extend_from_slice(density);
                elements.push(run.clone());
            }
        }
    }
    put(v, row0, &densities)?;
    put(v, row3, &ranges(elements.iter().map(|e| e.len() / ELEMENT)))?;
    put(v, row4, &elements.concat())?;
    let per_model = (walked.models.iter().enumerate())
        .map(|(m, g)| g.len() + if m == walked.index { k } else { 0 });
    put(v, table.models_at, &ranges(per_model))?;
    for (row, array, count) in [(0, row0, 4 * g_new), (3, row3, g_new), (4, row4, e_new)] {
        let r = table.at + 16 * row;
        put(v, r, &pointer(array))?;
        put(v, r + 0x08, &(count as u16).to_le_bytes())?;
        put(v, r + 0x0A, &(count as u16).to_le_bytes())?;
    }

    let found = locate(&out, role, code, at)?;
    written.root_reboxed = rebox(&mut out.virtual_section, &found, at, &pieces[0].positions)?;
    for (i, piece) in pieces.iter().enumerate().skip(1) {
        let placed = GeometryAt {
            geometry: at.geometry + i,
            ..at
        };
        let count = locate(&out, role, code, placed)?.geometry.vertex_count;
        if count as usize != piece.positions.len() {
            return Err(Refused("a piece does not read back with its vertex count"));
        }
    }
    written.physical_grown = out.physical.len().saturating_sub(sections.physical.len());
    *sections = out;
    Ok(GeometryCut {
        written,
        pieces: pieces.len(),
    })
}

/// Every value count the edit gives besides its positions'.
fn lengths(e: &GeometryEdit) -> impl Iterator<Item = usize> + '_ {
    [e.normals.len(), e.tangents.len()]
        .into_iter()
        .chain(e.uvs.iter().map(|(_, values)| values.len()))
        .chain(e.colours.iter().map(|(_, values)| values.len()))
        .chain(e.weights.iter().map(|(_, values)| values.len()))
        .chain(e.joints.iter().map(|(_, values)| values.len()))
        .chain(e.uv_tangents.iter().map(|(_, values)| values.len()))
}

/// `edit` cut by triangle in its order: a piece takes the next triangle while it would then hold
/// at most 65,536 vertices, else the triangle starts the next piece. A piece numbers its vertices
/// in the order its triangles first use them, so a vertex two pieces use is in each.
// A piece is bounded by its vertex count and by its meshlets' vertex-list total: the index data's
// per-meshlet vertex offset is 16-bit, so that total cannot pass 0xFFFF. A ribbon, whose meshlets
// each carry two seam vertices again, hits the list bound well before the vertex bound.
fn cut(edit: &GeometryEdit) -> Vec<GeometryEdit> {
    let mut pieces = Vec::new();
    let (mut order, mut triangles): (Vec<u32>, Vec<[u32; 3]>) = (Vec::new(), Vec::new());
    let mut remap: HashMap<u32, u32> = HashMap::new();
    let mut fit = MeshletFit::default();
    for t in &edit.triangles {
        let mut fresh: Vec<u32> = (t.iter().copied())
            .filter(|i| !remap.contains_key(i))
            .collect();
        fresh.sort_unstable();
        fresh.dedup();
        let over_vertices = order.len() + fresh.len() > MAX_VERTICES;
        let over_meshlets = fit.list_with(t) > LIST_LIMIT || triangles.len() >= LIST_LIMIT;
        if (over_vertices || over_meshlets) && !order.is_empty() {
            pieces.push(piece(edit, &order, std::mem::take(&mut triangles)));
            order.clear();
            remap.clear();
            fit = MeshletFit::default();
        }
        let corners = (*t).map(|i| {
            let next = order.len() as u32;
            let at = *remap.entry(i).or_insert(next);
            if at == next {
                order.push(i);
            }
            at
        });
        triangles.push(corners);
        fit.add(t);
    }
    if !order.is_empty() {
        pieces.push(piece(edit, &order, triangles));
    }
    pieces
}

/// The most entries a piece's meshlet vertex lists and its triangles may hold: the index data
/// addresses each with a 16-bit offset.
const LIST_LIMIT: usize = 0xFFFF;
/// A meshlet holds no more than this many vertices or triangles, as [`meshlets`] packs them.
const MESHLET_LIMIT: usize = 128;

/// A running image of how [`super::super::geometry_encode::meshlets`] would pack a piece's
/// triangles, kept to bound the piece by the vertex-list total those meshlets reach.
#[derive(Default)]
struct MeshletFit {
    /// The open meshlet's vertices.
    current: Vec<u32>,
    triangles: usize,
    /// The vertex counts of the meshlets already closed in this piece.
    closed: usize,
}

impl MeshletFit {
    /// How many of `t`'s vertices are not already in the open meshlet.
    fn new_vertices(&self, t: &[u32; 3]) -> usize {
        (0..3)
            .filter(|&c| !self.current.contains(&t[c]) && !t[..c].contains(&t[c]))
            .count()
    }

    fn closes(&self, new: usize) -> bool {
        self.triangles == MESHLET_LIMIT || self.current.len() + new > MESHLET_LIMIT
    }

    /// The piece's meshlet vertex-list total were `t` added next.
    fn list_with(&self, t: &[u32; 3]) -> usize {
        let new = self.new_vertices(t);
        match self.closes(new) {
            true => {
                self.closed
                    + self.current.len()
                    + (0..3).filter(|&c| !t[..c].contains(&t[c])).count()
            }
            false => self.closed + self.current.len() + new,
        }
    }

    fn add(&mut self, t: &[u32; 3]) {
        let new = self.new_vertices(t);
        if self.closes(new) {
            self.closed += self.current.len();
            self.current.clear();
            self.triangles = 0;
        }
        for &v in t {
            if !self.current.contains(&v) {
                self.current.push(v);
            }
        }
        self.triangles += 1;
    }
}

/// The vertices of `edit` that `order` names, in that order, with `triangles` over them.
fn piece(edit: &GeometryEdit, order: &[u32], triangles: Vec<[u32; 3]>) -> GeometryEdit {
    GeometryEdit {
        positions: picked(&edit.positions, order),
        normals: picked(&edit.normals, order),
        tangents: picked(&edit.tangents, order),
        uvs: by_channel(&edit.uvs, order),
        colours: by_channel(&edit.colours, order),
        weights: by_channel(&edit.weights, order),
        joints: by_channel(&edit.joints, order),
        uv_tangents: by_channel(&edit.uv_tangents, order),
        triangles,
    }
}

/// The values `order` names; none where `values` gives none.
fn picked<T: Copy>(values: &[T], order: &[u32]) -> Vec<T> {
    (order.iter())
        .filter_map(|&i| values.get(i as usize).copied())
        .collect()
}

fn by_channel<T: Copy>(list: &[(u8, Vec<T>)], order: &[u32]) -> Vec<(u8, Vec<T>)> {
    (list.iter())
        .map(|(c, values)| (*c, picked(values, order)))
        .collect()
}

fn pointer(target: usize) -> [u8; 8] {
    (VIRTUAL_TAG + target as u64).to_le_bytes()
}

/// A buffer's count of 4-byte words.
fn words(data: &[u8]) -> [u8; 4] {
    ((data.len() / 4) as u32).to_le_bytes()
}

/// The palette count a geometry states: the u16 at +0x2C, whose next u16 is a separate word.
fn palette_of(v: &[u8], geometry: usize) -> Written<u16> {
    let b = bytes_at(v, geometry + 0x2C, 2)?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

fn bytes_at(section: &[u8], at: usize, len: usize) -> Written<&[u8]> {
    (at.checked_add(len))
        .and_then(|end| section.get(at..end))
        .ok_or(Refused("an object of the geometry lies past its section"))
}

/// `(first, end)` u16 pairs over consecutive runs of `sizes`, from 0.
fn ranges(sizes: impl IntoIterator<Item = usize>) -> Vec<u8> {
    let mut at = 0;
    (sizes.into_iter())
        .flat_map(|size| {
            let pair = [at as u16, (at + size) as u16];
            at += size;
            pair.into_iter().flat_map(u16::to_le_bytes)
        })
        .collect()
}

/// A row 4 element of `corners` on the section's own axes, every `w` 0.
fn element((min, max): Corners) -> Vec<u8> {
    floats(&[
        min[0], min[1], min[2], 0.0, max[0], max[1], max[2], 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
    ])
}

/// What a piece written alone over the geometry left: its vertex and index data, its declaration
/// and its meshlet record's counts.
struct Alone {
    vertices: Vec<u8>,
    index: Vec<u8>,
    declaration: Vec<u8>,
    counts: Vec<u8>,
}

fn written_alone(
    s: &Rpf9Sections,
    role: Rpf9Role,
    code: Option<u32>,
    at: GeometryAt,
) -> Written<Alone> {
    let found = locate(s, role, code, at)?;
    let physical = |(start, len): (usize, usize)| bytes_at(&s.physical, start, len);
    Ok(Alone {
        vertices: physical(found.vertex_data)?.to_vec(),
        index: physical(found.index_data)?.to_vec(),
        declaration: bytes_at(&s.virtual_section, found.declaration, 0x140)?.to_vec(),
        counts: bytes_at(&s.virtual_section, found.record + 0x08, 8)?.to_vec(),
    })
}

/// The geometry `at` names, where its model and the drawable's per-geometry table lie.
fn held(
    s: &Rpf9Sections,
    role: Rpf9Role,
    code: Option<u32>,
    at: GeometryAt,
) -> Written<(Located, Walked, Table)> {
    let found = locate(s, role, code, at)?;
    let walked = walk(&s.virtual_section, found.root, at)?;
    let table = table(&s.virtual_section, found.root, &walked.models)?;
    Ok((found, walked, table))
}

/// The model that holds a geometry, its arrays, and every model's geometries in level order.
struct Walked {
    model: usize,
    boxes: usize,
    shaders: usize,
    count: usize,
    /// Each model's geometries, the levels in order and only levels with a model.
    models: Vec<Vec<usize>>,
    /// Where in `models` the model lies.
    index: usize,
}

impl Walked {
    fn geometry(&self, at: GeometryAt) -> Written<usize> {
        (self.models.get(self.index))
            .and_then(|g| g.get(at.geometry))
            .copied()
            .ok_or(Refused("the model holds no geometry there"))
    }

    /// The geometry's place among every geometry of the drawable.
    fn first(&self, at: GeometryAt) -> usize {
        (self.models.iter().take(self.index))
            .map(Vec::len)
            .sum::<usize>()
            + at.geometry
    }
}

fn walk(v: &[u8], root: usize, at: GeometryAt) -> Written<Walked> {
    let (models, here) = sections(v, None, |v, _| {
        let (mut models, mut here) = (Vec::new(), None);
        for level in 0..4 {
            let Some(l) = v.ptr("level", root + 0x50 + 8 * level)? else {
                continue;
            };
            let count = usize::from(v.u16("model count", l + 8)?);
            if count == 0 {
                continue;
            }
            let list = v.to("models", l)?;
            v.array("models", list, count, 8)?;
            for m in 0..count {
                let model = v.to("model", list + 8 * m)?;
                let n = usize::from(v.u16("geometry count", model + 0x10)?);
                let array = v.to("geometries", model + 0x08)?;
                v.array("geometries", array, n, 8)?;
                let geometries = (0..n)
                    .map(|j| v.to("geometry", array + 8 * j))
                    .collect::<std::result::Result<Vec<usize>, HeaderlessError>>()?;
                if (level, m) == (at.level, at.model) {
                    let boxes = v.to("geometry boxes", model + 0x18)?;
                    let shaders = v.to("shader indices", model + 0x20)?;
                    here = Some((model, boxes, shaders, n, models.len()));
                }
                models.push(geometries);
            }
        }
        Ok((models, here))
    })?;
    let (model, boxes, shaders, count, index) =
        here.ok_or(Refused("the drawable holds no model there"))?;
    Ok(Walked {
        model,
        boxes,
        shaders,
        count,
        models,
        index,
    })
}

/// The drawable's per-geometry table: where it lies, row 2's array, and each geometry's row 0
/// words and row 4 elements.
struct Table {
    at: usize,
    models_at: usize,
    densities: Vec<Vec<u8>>,
    elements: Vec<Vec<u8>>,
}

/// The table at +0xB0 where its rows read by the range rule: rows of `{array, u16 count, u16
/// capacity, u32 0}`; row 3 a `(first, end)` pair per geometry over row 4, contiguous from 0;
/// row 2 one per model; row 0 four words per geometry.
fn table(v: &[u8], root: usize, models: &[Vec<usize>]) -> Written<Table> {
    let geometries: usize = models.iter().map(Vec::len).sum();
    let read = sections(v, None, |v, _| {
        let Some(at) = v.ptr("per-geometry table", root + 0xB0)? else {
            return Ok(None);
        };
        let mut rows = [(0usize, 0usize); 5];
        for (i, row) in rows.iter_mut().enumerate() {
            let r = at + 16 * i;
            let count = v.u16("table row's count", r + 0x08)?;
            let capacity = v.u16("table row's capacity", r + 0x0A)?;
            if capacity != count || v.u32("table row's last word", r + 0x0C)? != 0 {
                return Ok(None);
            }
            let count = usize::from(count);
            let array = match count {
                0 => 0,
                _ => v.to("table row", r)?,
            };
            *row = (array, count);
        }
        if rows[3].1 != geometries || rows[0].1 != 4 * geometries || rows[2].1 != models.len() {
            return Ok(None);
        }
        let pairs = v.get("table row 3", rows[3].0, 4 * geometries)?;
        let (mut counts, mut end) = (Vec::with_capacity(geometries), 0);
        for pair in pairs.as_chunks::<4>().0 {
            let first = usize::from(u16::from_le_bytes([pair[0], pair[1]]));
            let last = usize::from(u16::from_le_bytes([pair[2], pair[3]]));
            if first != end || last < first {
                return Ok(None);
            }
            counts.push(last - first);
            end = last;
        }
        if end != rows[4].1 {
            return Ok(None);
        }
        let densities = v.get("table row 0", rows[0].0, 16 * geometries)?;
        let boxes = v.get("table row 4", rows[4].0, ELEMENT * end)?;
        let mut from = 0;
        let elements = (counts.iter())
            .map(|c| {
                let run = boxes[ELEMENT * from..ELEMENT * (from + c)].to_vec();
                from += c;
                run
            })
            .collect();
        Ok(Some(Table {
            at,
            models_at: rows[2].0,
            densities: (densities.as_chunks::<16>().0.iter())
                .map(|d| d.to_vec())
                .collect(),
            elements,
        }))
    })?;
    read.ok_or(Refused(
        "the drawable's per-geometry table at +0xB0 is missing or does not read by its range \
         rule, so no geometry is added to it",
    ))
}

#[cfg(test)]
#[path = "cut_palette_tests.rs"]
mod palette_tests;
