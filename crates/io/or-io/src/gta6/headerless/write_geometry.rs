//! A geometry's vertices and triangles written over what its reader read: its streams,
//! declaration, slot table and quantisation block, its meshlet list, and the boxes they derive. A
//! buffer that outgrows its bytes moves to the physical section's end, which grows through the
//! section-growth function.
//!
//! ⚠ The geometry's word at +0x30 is a digest of its content that nothing can derive; it is left
//! as read.

use super::bytes::{f32_at, sections, u32_at, u64_at};
use super::dictionary::entry_of;
use super::drawable::{drawable_at, Channel, Geometry};
use super::geometry_encode::{
    frame_word, index_words, meshlet_bounds, meshlets, pack_lanes, s16_pair, unorm10_word,
    unorm_bytes, Quantisation,
};
use super::streams::{lanes, SLOT_CHANNELS};
use super::write_physics::put;
use super::HeaderlessError;
use crate::container::rpf::{grow_section, Rpf9GrowError, Rpf9Role, Rpf9Section, Rpf9Sections};

mod cut;
mod posed;
mod reference;
mod reference_cells;
mod skin;

pub use cut::{write_geometry_cut, GeometryCut};
pub use posed::{bind_positions, rebox_bind_pose};

/// A vertex list's u16 entries name at most this many vertices.
const MAX_VERTICES: usize = 0x1_0000;
/// No physical object crosses one of these.
const PHYSICAL_BLOCK: usize = 0x1_0000;
const PHYSICAL_TAG: u64 = 0x6000_0000;
const VIRTUAL_TAG: u64 = 0x5000_0000;
/// The slot table's bytes, which every stream follows.
const SLOT_TABLE: usize = 0x60;

/// Why a geometry is not written.
#[derive(Debug, thiserror::Error)]
pub enum GeometryWriteError {
    #[error(transparent)]
    Read(#[from] HeaderlessError),
    #[error(transparent)]
    Grow(#[from] Rpf9GrowError),
    #[error("the geometry is not written: {0}")]
    Refused(&'static str),
    #[error("the geometry's channel {channel} is not written: {why}")]
    Channel { channel: u8, why: &'static str },
}

use GeometryWriteError::Refused;

type Written<T> = std::result::Result<T, GeometryWriteError>;

/// Where a geometry lies in its drawable: its level of detail, its model and its place there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeometryAt {
    pub level: usize,
    pub model: usize,
    pub geometry: usize,
}

/// What a geometry's vertices and triangles become; an empty channel keeps what the geometry holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GeometryEdit {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// Texcoord 0's tangent, `w` 1 where the cross product of normal and tangent runs along v.
    pub tangents: Vec<[f32; 4]>,
    /// Texcoords by declaration channel, 28 to 33.
    pub uvs: Vec<(u8, Vec<[f32; 2]>)>,
    /// Colours by declaration channel, 24 to 26.
    pub colours: Vec<(u8, Vec<[f32; 4]>)>,
    /// Weights by declaration channel, 16 and 17; a vertex's lanes over both sum to one.
    pub weights: Vec<(u8, Vec<[f32; 4]>)>,
    /// Blend indices by declaration channel, 20 and 21: each lane's index into the model's palette.
    pub joints: Vec<(u8, Vec<[u16; 4]>)>,
    /// Tangents of texcoords 1 to 3 by declaration channel, 9 to 11, `w` as in `tangents`.
    pub uv_tangents: Vec<(u8, Vec<[f32; 4]>)>,
    pub triangles: Vec<[u32; 3]>,
}

/// What a geometry's write changed besides its streams.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GeometryWritten {
    /// The channels written as the geometry held them, which the edit gave no values for.
    pub carried: Vec<u8>,
    /// Whether the meshlet list was built again from the triangles.
    pub triangles_rebuilt: bool,
    /// Whether only the meshlets' bounds were written again, around moved vertices.
    pub bounds_rewritten: bool,
    /// Whether the drawable's box and sphere were derived again from its level-0 boxes.
    pub root_reboxed: bool,
    /// Bytes the physical section grew by.
    pub physical_grown: usize,
}

/// Where a geometry and what derives from it lie in the virtual section, and its read.
struct Located {
    geometry: Geometry,
    root: usize,
    /// The model's box array and its geometry count.
    boxes: usize,
    count: usize,
    vertex_buffer: usize,
    declaration: usize,
    packed: u64,
    record: usize,
    index_buffer: usize,
    /// Each buffer's bytes in the physical section: where they start, and how many.
    vertex_data: (usize, usize),
    index_data: (usize, usize),
    /// Each level-0 geometry's box, and whether no level-0 model has a matrix count.
    level0: Vec<usize>,
    rigid: bool,
}

fn locate(
    s: &Rpf9Sections,
    role: Rpf9Role,
    code: Option<u32>,
    at: GeometryAt,
) -> Result<Located, HeaderlessError> {
    sections(&s.virtual_section, Some(&s.physical), |v, p| {
        let (root, bound_at_c8) = match (role, code) {
            (Rpf9Role::Drawable, None) => (0, true),
            (Rpf9Role::DrawableWithBound, None) => (v.to("drawable", 0x18)?, false),
            (Rpf9Role::Dictionary, Some(code)) => {
                let entry = entry_of(v, code)?.ok_or(HeaderlessError::NotInPlace(
                    "no entry of the dictionary has that code",
                ))?;
                (entry, true)
            }
            _ => {
                return Err(HeaderlessError::NotInPlace(
                    "the resource holds no drawable there",
                ))
            }
        };
        let mut drawable = drawable_at(v, p, root, bound_at_c8)?;
        let past = |what, index: usize, count: usize| HeaderlessError::IndexPastCount {
            what,
            index: index as u64,
            count: count as u64,
        };
        let mut models = (drawable.levels.get_mut(at.level))
            .and_then(Option::take)
            .ok_or(past("level of detail", at.level, 4))?;
        let model_count = models.len();
        if at.model >= model_count {
            return Err(past("model", at.model, model_count));
        }
        let geometries = models.swap_remove(at.model).geometries;
        let count = geometries.len();
        let geometry = (geometries.into_iter().nth(at.geometry)).ok_or(past(
            "geometry",
            at.geometry,
            count,
        ))??;
        let level = v.to("level", root + 0x50 + 8 * at.level)?;
        let model = v.to("model", v.to("models", level)? + 8 * at.model)?;
        let geometry_at = v.to(
            "geometry",
            v.to("geometries", model + 0x08)? + 8 * at.geometry,
        )?;
        let vertex_buffer = v.to("vertex buffer", geometry_at + 0x08)?;
        let declaration = v.to("vertex declaration", vertex_buffer + 0x40)?;
        let record = v.to("meshlet record", geometry_at + 0x40)?;
        let index_buffer = v.to("index buffer", record)?;
        let data = |object: usize| -> Result<(usize, usize), HeaderlessError> {
            let words = v.u32("buffer's count", object + 0x08)? as usize;
            Ok((v.phys("buffer's data", object + 0x18)?, 4 * words))
        };
        let (mut level0, mut rigid) = (Vec::new(), true);
        if let Some(level) = v.ptr("level", root + 0x50)? {
            for i in 0..usize::from(v.u16("model count", level + 8)?) {
                let model = v.to("model", v.to("models", level)? + 8 * i)?;
                let n = usize::from(v.u16("geometry count", model + 0x10)?);
                let boxes = v.to("geometry boxes", model + 0x18)?;
                let first = usize::from(n > 1);
                level0.extend((0..n).map(|j| boxes + 32 * (j + first)));
                rigid &= v.u8("matrix count", model + 0x30)? == 0;
            }
        }
        Ok(Located {
            geometry,
            root,
            boxes: v.to("geometry boxes", model + 0x18)?,
            count,
            vertex_buffer,
            declaration,
            packed: v.u64("declaration's counts", declaration + 0x138)?,
            record,
            index_buffer,
            vertex_data: data(vertex_buffer)?,
            index_data: data(index_buffer)?,
            level0,
            rigid,
        })
    })
}

/// Geometry `at` of the drawable of `role` (or of dictionary entry `code`) written from `edit`.
// Every derived word is written with it; a refusal leaves the sections as they were.
pub fn write_geometry(
    sections: &mut Rpf9Sections,
    role: Rpf9Role,
    code: Option<u32>,
    at: GeometryAt,
    edit: &GeometryEdit,
) -> Written<GeometryWritten> {
    let found = locate(sections, role, code, at)?;
    let g = &found.geometry;
    let (n, n_old) = (edit.positions.len(), g.vertex_count as usize);
    if n == 0 || n > MAX_VERTICES {
        return Err(Refused("a geometry holds 1 to 65,536 vertices"));
    }
    if edit.positions.iter().flatten().any(|x| !x.is_finite()) {
        return Err(Refused("a position is not a finite number"));
    }
    if edit.triangles.is_empty() {
        return Err(Refused("a geometry holds at least one triangle"));
    }
    if edit.triangles.iter().flatten().any(|i| *i as usize >= n) {
        return Err(Refused("a triangle names a vertex past the vertex count"));
    }
    let held = (g.positions.as_ref())
        .map_err(|_| Refused("its positions do not read, so none of its vertices can be kept"))?;
    let declared = |index: u8| g.channels.iter().any(|c| c.index == index);
    let mut named = (edit.uvs.iter().map(|(c, _)| *c))
        .chain(edit.colours.iter().map(|(c, _)| *c))
        .chain(edit.weights.iter().map(|(c, _)| *c))
        .chain(edit.joints.iter().map(|(c, _)| *c))
        .chain(edit.uv_tangents.iter().map(|(c, _)| *c));
    if let Some(channel) = named.find(|c| !declared(*c)) {
        return Err(GeometryWriteError::Channel {
            channel,
            why: "the geometry declares no such channel",
        });
    }
    if !edit.normals.is_empty() && !declared(4) {
        return Err(Refused(
            "the edit gives normals and the geometry declares none",
        ));
    }
    if !edit.tangents.is_empty() && g.vertices.tangents.is_empty() {
        return Err(Refused(
            "the edit gives tangents and no channel of the geometry holds them",
        ));
    }

    let (start, len) = found.vertex_data;
    let old = (sections.physical.get(start..start + len))
        .ok_or(Refused("its vertex data lies past the physical section"))?;
    let positions_same = edit.positions == *held;
    let (built, decoded, carried) = vertex_data(g, edit, old, positions_same)?;

    let (start, len) = found.index_data;
    let old_index = (sections.physical.get(start..start + len))
        .ok_or(Refused("its index data lies past the physical section"))?;
    let triangles_same = edit.triangles == g.triangles;
    let cut = (!triangles_same).then(|| meshlets(&edit.triangles));
    let index = match (&cut, positions_same) {
        (Some(cut), _) => {
            // Bit 30 of the first word adds per-meshlet words after the vertex list.
            if old_index.get(3).is_some_and(|b| b & 0x40 != 0) {
                return Err(Refused(
                    "its index data holds per-meshlet words whose meaning is not known, so its \
                     triangles are kept",
                ));
            }
            let words = index_words(cut, &decoded).map_err(Refused)?;
            Some(
                words
                    .iter()
                    .flat_map(|w| w.to_le_bytes())
                    .collect::<Vec<u8>>(),
            )
        }
        (None, false) => Some(rebounded(old_index, &decoded)?),
        (None, true) => None,
    };

    let mut out = sections.clone();
    let mut written = GeometryWritten {
        carried,
        triangles_rebuilt: !triangles_same,
        bounds_rewritten: triangles_same && !positions_same,
        ..GeometryWritten::default()
    };
    let before = out.physical.len();
    if built != old {
        let why = "another geometry shares its vertex buffer";
        refuse_shared(&out.virtual_section, found.vertex_buffer, why)?;
        place(&mut out, found.vertex_buffer, found.vertex_data, &built)?;
        let words = (built.len() / 4) as u32;
        put(
            &mut out.virtual_section,
            found.vertex_buffer + 0x08,
            &words.to_le_bytes(),
        )?;
    }
    if n != n_old {
        let why = "another vertex buffer shares its declaration";
        refuse_shared(&out.virtual_section, found.declaration, why)?;
        declare(&mut out.virtual_section, &found, &built_offsets(g, n), n)?;
    }
    if let Some(index) = &index {
        let why = "another meshlet record shares its index buffer";
        refuse_shared(&out.virtual_section, found.index_buffer, why)?;
        place(&mut out, found.index_buffer, found.index_data, index)?;
        let words = (index.len() / 4) as u32;
        put(
            &mut out.virtual_section,
            found.index_buffer + 0x08,
            &words.to_le_bytes(),
        )?;
    }
    if let Some(cut) = &cut {
        let why = "another geometry shares its meshlet record";
        refuse_shared(&out.virtual_section, found.record, why)?;
        let t: usize = cut.iter().map(|k| k.triangles.len()).sum();
        let list: usize = cut.iter().map(|k| k.vertices.len()).sum();
        let v = &mut out.virtual_section;
        put(v, found.record + 0x08, &(cut.len() as u16).to_le_bytes())?;
        // The record keeps the low 16 bits of the triangle count.
        put(v, found.record + 0x0A, &(t as u16).to_le_bytes())?;
        put(v, found.record + 0x0C, &((list as u32) << 8).to_le_bytes())?;
    }
    if !positions_same {
        written.root_reboxed = rebox(&mut out.virtual_section, &found, at, &edit.positions)?;
    }
    written.physical_grown = out.physical.len().saturating_sub(before);
    *sections = out;
    Ok(written)
}

/// The values a channel is written from: the edit's, else the ones the geometry holds, which fit
/// only while the vertex count is unchanged.
fn chosen<'a, T>(given: &'a [T], held: &'a [T], n: usize, channel: u8) -> Written<&'a [T]> {
    match given.len() {
        len if len == n => Ok(given),
        0 if held.len() == n => Ok(held),
        0 => Err(GeometryWriteError::Channel {
            channel,
            why: "the edit gives none of its values and the vertex count changed",
        }),
        _ => Err(GeometryWriteError::Channel {
            channel,
            why: "the edit gives another number of its values than of positions",
        }),
    }
}

/// The values a list of channels holds for `channel`; empty where it holds none.
fn of_channel<T>(list: &[(u8, Vec<T>)], channel: u8) -> &[T] {
    (list.iter().find(|(c, _)| *c == channel))
        .map(|(_, values)| values.as_slice())
        .unwrap_or_default()
}

/// `n` elements of `size` bytes: element `i` as `old` holds it where `same(i)`, else as `encode`
/// writes it.
fn elements(
    n: usize,
    size: usize,
    old: &[u8],
    same: impl Fn(usize) -> bool,
    mut encode: impl FnMut(usize, &mut [u8]) -> Written<()>,
) -> Written<Vec<u8>> {
    let mut out = vec![0u8; n * size];
    for (i, e) in out.chunks_exact_mut(size).enumerate() {
        match old.get(i * size..(i + 1) * size) {
            Some(was) if same(i) => e.copy_from_slice(was),
            _ => encode(i, e)?,
        }
    }
    Ok(out)
}

fn floats<'a>(values: impl IntoIterator<Item = &'a f32>) -> Vec<u8> {
    values.into_iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// Each channel's stream offset for `n` vertices: as declared while the count is unchanged, else
/// in ascending channel order from the first stream, each 4-aligned after the one before.
fn built_offsets(g: &Geometry, n: usize) -> Vec<usize> {
    let channels = &g.channels;
    if n == g.vertex_count as usize {
        return channels.iter().map(|c| c.offset as usize).collect();
    }
    let mut next = channels
        .iter()
        .map(|c| c.offset as usize)
        .min()
        .unwrap_or(0);
    (channels.iter())
        .map(|c| {
            let at = next.next_multiple_of(4);
            next = at + usize::from(c.size) * n;
            at
        })
        .collect()
}

/// A geometry's vertex data, the decoded position of each vertex, and the channels carried as read.
type VertexData = (Vec<u8>, Vec<[f64; 3]>, Vec<u8>);

/// The vertex data `edit` gives the geometry, the decoded position of each vertex, and the
/// channels carried as read.
fn vertex_data(
    g: &Geometry,
    edit: &GeometryEdit,
    old: &[u8],
    positions_same: bool,
) -> Written<VertexData> {
    let (n, n_old) = (edit.positions.len(), g.vertex_count as usize);
    let channels: &[Channel] = &g.channels;
    let header = channels
        .iter()
        .map(|c| c.offset as usize)
        .min()
        .unwrap_or(0);
    if header < SLOT_TABLE {
        return Err(Refused("its streams start inside its slot table"));
    }
    let offsets = built_offsets(g, n);
    let end = (channels.iter().zip(&offsets))
        .map(|(c, at)| at + usize::from(c.size) * n)
        .max()
        .unwrap_or(header);
    let mut data = match n == n_old {
        true => vec![0u8; old.len()],
        false => vec![0u8; end.div_ceil(4) * 4],
    };
    let head = old
        .get(..header)
        .ok_or(Refused("its vertex data is shorter than its header"))?;
    data[..header].copy_from_slice(head);
    if n != n_old {
        for (slot, channel) in SLOT_CHANNELS.iter().enumerate() {
            let word = u32_at(&data, 4 * slot);
            let k = channel.and_then(|index| channels.iter().position(|c| c.index == index));
            if let (true, Some(k)) = (word != 0, k) {
                let word = ((offsets[k] / 2) as u32) << 8 | (word & 0xFF);
                data[4 * slot..4 * slot + 4].copy_from_slice(&word.to_le_bytes());
            }
        }
    }
    let v = &g.vertices;
    let held = g.positions.as_ref().map(Vec::as_slice).unwrap_or_default();
    let tangents_in_8 = channels.iter().all(|c| c.index != 4 || c.code != 0x29)
        && !v.skipped.iter().any(|(c, _)| *c == 8);
    let (mut decoded, mut carried) = (Vec::with_capacity(n), Vec::new());
    for (c, at) in channels.iter().zip(&offsets) {
        let size = usize::from(c.size);
        let from = c.offset as usize;
        let was = (old.get(from..from + size * n_old))
            .ok_or(Refused("a stream lies past its vertex data"))?;
        let same_position = |i: usize| held.get(i) == Some(&edit.positions[i]);
        let bytes = match (c.index, c.code, c.size) {
            (0, 0x06, 12) => {
                decoded = (edit.positions.iter()).map(|p| p.map(f64::from)).collect();
                floats(edit.positions.iter().flatten())
            }
            (0, 0x10, 8) => {
                if header < 0x80 {
                    return Err(Refused("its streams start inside its quantisation block"));
                }
                let read = (old.get(..0x80).and_then(Quantisation::read)).ok_or(Refused(
                    "its quantisation block's axis word is not 0, 1 or 2",
                ))?;
                let kept = positions_same || edit.positions.iter().all(|p| read.quanta(*p).1);
                let block = match kept {
                    true => read,
                    false => {
                        let (min, max) = extremes(&edit.positions);
                        let block = Quantisation::of_box(min, max);
                        block.write(&mut data);
                        block
                    }
                };
                let bytes = elements(
                    n,
                    8,
                    was,
                    |i| kept && same_position(i),
                    |i, e| {
                        e.copy_from_slice(&pack_lanes(block.quanta(edit.positions[i]).0));
                        Ok(())
                    },
                )?;
                decoded = (bytes.as_chunks::<8>().0.iter())
                    .map(|e| block.decode(lanes(e)))
                    .collect();
                bytes
            }
            (0, ..) => return Err(Refused("its positions are in a form no writer encodes")),
            (4, 0x29, 4) => {
                let normals = chosen(&edit.normals, &v.normals, n, 4)?;
                let tangents = chosen(&edit.tangents, &v.tangents, n, 4)?;
                let same = |i: usize| {
                    v.normals.get(i) == Some(&normals[i]) && v.tangents.get(i) == Some(&tangents[i])
                };
                elements(n, 4, was, same, |i, e| {
                    e.copy_from_slice(&frame_word(normals[i], tangents[i]).to_le_bytes());
                    Ok(())
                })?
            }
            (4, 0x18, 4) => {
                let normals = chosen(&edit.normals, &v.normals, n, 4)?;
                elements(
                    n,
                    4,
                    was,
                    |i| v.normals.get(i) == Some(&normals[i]),
                    |i, e| {
                        // Bits 30 and 31 are kept as read; no reader gives them a meaning.
                        let high = was.get(4 * i..4 * i + 4).map_or(0, |w| u32_at(w, 0) >> 30);
                        e.copy_from_slice(&unorm10_word(normals[i], high).to_le_bytes());
                        Ok(())
                    },
                )?
            }
            (4, 0x06, 12) => floats(chosen(&edit.normals, &v.normals, n, 4)?.iter().flatten()),
            (8, 0x18, 4) if tangents_in_8 => {
                let tangents = chosen(&edit.tangents, &v.tangents, n, 8)?;
                elements(
                    n,
                    4,
                    was,
                    |i| v.tangents.get(i) == Some(&tangents[i]),
                    |i, e| {
                        let [x, y, z, w] = tangents[i];
                        let sign = if w > 0.0 { 0 } else { 3 };
                        e.copy_from_slice(&unorm10_word([x, y, z], sign).to_le_bytes());
                        Ok(())
                    },
                )?
            }
            (24..=26, 0x1C, 4) => {
                let read = of_channel(&v.colours, c.index);
                let colours = chosen(of_channel(&edit.colours, c.index), read, n, c.index)?;
                elements(
                    n,
                    4,
                    was,
                    |i| read.get(i) == Some(&colours[i]),
                    |i, e| {
                        e.copy_from_slice(&unorm_bytes(colours[i]));
                        Ok(())
                    },
                )?
            }
            (28..=32, 0x10, 8) => {
                let read = of_channel(&v.uvs, c.index);
                let uvs = chosen(of_channel(&edit.uvs, c.index), read, n, c.index)?;
                floats(uvs.iter().flatten())
            }
            (28..=33, 0x26, 4) => {
                let read = of_channel(&v.uvs, c.index);
                let uvs = chosen(of_channel(&edit.uvs, c.index), read, n, c.index)?;
                elements(
                    n,
                    4,
                    was,
                    |i| read.get(i) == Some(&uvs[i]),
                    |i, e| {
                        let pair = s16_pair(uvs[i]).ok_or(GeometryWriteError::Channel {
                            channel: c.index,
                            why: "a texcoord passes the -4 to 4 its 16-bit form holds",
                        })?;
                        e.copy_from_slice(&pair);
                        Ok(())
                    },
                )?
            }
            (16 | 17, 0x1C, 4) if !edit.weights.is_empty() => {
                skin::weights(c.index, edit, v, n, was)?
            }
            (20 | 21, 0x03, 16) if n != n_old => return Err(skin::no_fill(c.index)),
            (20 | 21, ..) if !edit.joints.is_empty() => skin::indices(c, edit, v, n, was)?,
            (9..=11, 0x18, 4) if !edit.uv_tangents.is_empty() => {
                skin::uv_tangents(c.index, edit, v, n, was)?
            }
            (33, 0x10, 8) if n != n_old => {
                reference_cells::channel33(held, &edit.positions, &edit.triangles, was)?
            }
            (33, ..) if n != n_old => return Err(reference_cells::unknown_form()),
            _ if n == n_old => {
                carried.push(c.index);
                was.to_vec()
            }
            _ => {
                return Err(GeometryWriteError::Channel {
                    channel: c.index,
                    why: "the edit gives no values for it, and it is carried only while the \
                          vertex count is unchanged",
                })
            }
        };
        let to = (data.get_mut(*at..*at + bytes.len()))
            .ok_or(Refused("a stream lies past its vertex data"))?;
        to.copy_from_slice(&bytes);
    }
    Ok((data, decoded, carried))
}

/// The box of `positions`, each axis its least and greatest value.
fn extremes(positions: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
    for p in positions {
        for a in 0..3 {
            min[a] = min[a].min(p[a]);
            max[a] = max[a].max(p[a]);
        }
    }
    (min, max)
}

/// The index data `old` with each meshlet's bounds written again about `positions`, every other
/// word as read; its cone becomes 0, which culls from no direction.
fn rebounded(old: &[u8], positions: &[[f64; 3]]) -> Written<Vec<u8>> {
    let bad = || Refused("its index data does not hold the meshlets its reader read");
    let word = |k: usize| {
        old.get(4 * k..4 * k + 4)
            .map(|w| u32_at(w, 0))
            .ok_or_else(bad)
    };
    let (m, t, list) = (
        (word(0)? & 0xFFFF) as usize,
        word(1)? as usize,
        word(2)? as usize,
    );
    let list_at = 4 * (3 + 4 * m + t);
    let mut out = old.to_vec();
    for k in 0..m {
        let first = (word(4 + 4 * k)? & 0xFFFF) as usize;
        let last = match k + 1 < m {
            true => (word(8 + 4 * k)? & 0xFFFF) as usize,
            false => list,
        };
        let points = (first..last)
            .map(|j| {
                let entry = old.get(list_at + 2 * j..list_at + 2 * j + 2)?;
                positions
                    .get(usize::from(u16::from_le_bytes([entry[0], entry[1]])))
                    .copied()
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(bad)?;
        let (radius, centre, z) = meshlet_bounds(&points).map_err(Refused)?;
        let counts = word(3 + 4 * k)? & 0xFFFF;
        for (w, value) in [
            (3 + 4 * k, counts | radius),
            (5 + 4 * k, centre),
            (6 + 4 * k, z),
        ] {
            let to = out.get_mut(4 * w..4 * w + 4).ok_or_else(bad)?;
            to.copy_from_slice(&value.to_le_bytes());
        }
    }
    Ok(out)
}

/// Refuses, saying `why`, an edit of the object at `target` that another word of the virtual
/// section also points at, since the write would change that one too.
fn refuse_shared(v: &[u8], target: usize, why: &'static str) -> Written<()> {
    let word = VIRTUAL_TAG + target as u64;
    let named = (0..v.len().saturating_sub(7))
        .step_by(8)
        .filter(|&o| u64_at(v, o) == word)
        .count();
    match named > 1 {
        true => Err(Refused(why)),
        false => Ok(()),
    }
}

/// The declaration of `n` vertices at `offsets`: each unused channel at channel 0's offset.
fn declare(v: &mut [u8], found: &Located, offsets: &[usize], n: usize) -> Written<()> {
    let channels = &found.geometry.channels;
    let first = (channels.iter().position(|c| c.index == 0)).map_or(0, |k| offsets[k]);
    for k in 0..52u8 {
        let offset = (channels.iter().position(|c| c.index == k)).map_or(first, |i| offsets[i]);
        put(
            v,
            found.declaration + 4 * usize::from(k),
            &(offset as u32).to_le_bytes(),
        )?;
    }
    let packed = (found.packed & 0x3FF) | (n as u64) << 10;
    put(v, found.declaration + 0x138, &packed.to_le_bytes())?;
    Ok(())
}

/// A box, its minimum then its maximum.
type Corners = ([f32; 3], [f32; 3]);

/// The box at `at`: its minimum and maximum, each with `w` after it.
fn put_box(
    v: &mut [u8],
    at: usize,
    (min, max): Corners,
    w: impl Fn([f32; 3]) -> f32,
) -> Written<()> {
    for (o, corner) in [(0, min), (0x10, max)] {
        let words = [corner[0], corner[1], corner[2], w(corner)];
        put(v, at + o, &floats(&words))?;
    }
    Ok(())
}

fn read_box(v: &[u8], at: usize) -> Written<Corners> {
    let corner = |o: usize| -> Option<[f32; 3]> {
        let b = v.get(at + o..at + o + 12)?;
        Some(std::array::from_fn(|a| f32_at(b, 4 * a)))
    };
    corner(0)
        .zip(corner(0x10))
        .ok_or(Refused("a box lies past the virtual section"))
}

fn union(boxes: &[Corners]) -> Corners {
    let start = ([f32::MAX; 3], [f32::MIN; 3]);
    boxes.iter().fold(start, |(lo, hi), (min, max)| {
        (
            std::array::from_fn(|a| lo[a].min(min[a])),
            std::array::from_fn(|a| hi[a].max(max[a])),
        )
    })
}

/// The geometry's box from `positions`, its model's union box where it has more than one
/// geometry, and the drawable's box and sphere from its level-0 boxes where every level-0 model is
/// rigid; whether the drawable's were written.
fn rebox(v: &mut [u8], found: &Located, at: GeometryAt, positions: &[[f32; 3]]) -> Written<bool> {
    let x = |c: [f32; 3]| c[0];
    let first = usize::from(found.count > 1);
    put_box(
        v,
        found.boxes + 32 * (at.geometry + first),
        extremes(positions),
        x,
    )?;
    if found.count > 1 {
        let mut boxes = Vec::with_capacity(found.count);
        for j in 1..=found.count {
            boxes.push(read_box(v, found.boxes + 32 * j)?);
        }
        put_box(v, found.boxes, union(&boxes), x)?;
    }
    // A bone-attached model's vertices lie in its bone's space, the drawable's box in its own.
    if at.level != 0 || !found.rigid || found.level0.is_empty() {
        return Ok(false);
    }
    let mut boxes = Vec::with_capacity(found.level0.len());
    for b in &found.level0 {
        boxes.push(read_box(v, *b)?);
    }
    let (min, max) = union(&boxes);
    put_box(v, found.root + 0x30, (min, max), |_| 0.0)?;
    let centre: [f32; 3] =
        std::array::from_fn(|a| ((f64::from(min[a]) + f64::from(max[a])) / 2.0) as f32);
    let half: f64 = (0..3)
        .map(|a| (f64::from(max[a]) - f64::from(min[a])).powi(2))
        .sum::<f64>()
        .sqrt()
        / 2.0;
    let sphere = [centre[0], centre[1], centre[2], half as f32];
    put(v, found.root + 0x20, &floats(&sphere))?;
    Ok(true)
}

/// Blocks a physical section of `len` bytes is cut into: whole 64 KiB pieces, then the rest.
fn blocks(len: usize) -> usize {
    usize::from(len >= PHYSICAL_BLOCK) + usize::from(!len.is_multiple_of(PHYSICAL_BLOCK))
}

/// `data` written as the bytes of the buffer object at `object`, which held `old` bytes at
/// `start`: over them where they fit, else from the end of the physical section, or from `start`
/// where they ended it. The section grows through the section-growth function, to a whole number
/// of blocks where the bytes would cross a 64 KiB boundary or the count of blocks would rise.
fn place(
    s: &mut Rpf9Sections,
    object: usize,
    (start, old): (usize, usize),
    data: &[u8],
) -> Written<()> {
    let len = s.physical.len();
    let aligned = start.is_multiple_of(16);
    let room = if aligned {
        old.next_multiple_of(16)
    } else {
        old
    };
    let room = room.min(len.saturating_sub(start));
    let span = start..start + room;
    let v = &s.virtual_section;
    let shared = (0..v.len().saturating_sub(7)).step_by(8).any(|o| {
        let word = u64_at(v, o);
        let target = (word & 0x0FFF_FFFF) as usize;
        o != object + 0x18 && word >> 28 == 6 && span.contains(&target)
    });
    if shared {
        return Err(Refused(
            "another object's pointer aims into the buffer's bytes",
        ));
    }
    if data.len() <= room {
        let bytes = &mut s.physical[start..start + room];
        bytes[..data.len()].copy_from_slice(data);
        bytes[data.len()..].fill(0);
        return Ok(());
    }
    // The loose-resource writer pads the section with zeros to 16 bytes the same way.
    s.physical.resize(len.next_multiple_of(16), 0);
    let p0 = s.physical.len();
    let last = aligned && start + old.next_multiple_of(16) >= p0;
    let at = if last { start } else { p0 };
    let end = at + data.len().next_multiple_of(16);
    let crosses = at / PHYSICAL_BLOCK != (end - 1) / PHYSICAL_BLOCK;
    let end = match crosses || blocks(end) > blocks(p0) {
        true => end.next_multiple_of(PHYSICAL_BLOCK),
        false => end,
    };
    let virtual_len = s.virtual_section.len();
    let mut grown_to = p0;
    // Each step stays inside one 64 KiB piece, which the growth function asks of an append.
    while grown_to < end {
        let next = end.min((grown_to / PHYSICAL_BLOCK + 1) * PHYSICAL_BLOCK);
        grow_section(
            s,
            &[],
            Rpf9Section::Physical,
            grown_to,
            (next - grown_to) as isize,
        )?;
        grown_to = next;
    }
    if s.virtual_section.len() != virtual_len {
        return Err(Refused(
            "the block map took another slot, which moved the virtual section",
        ));
    }
    if !last {
        s.physical[start..start + old].fill(0);
    }
    let bytes = &mut s.physical[at..end];
    bytes[..data.len()].copy_from_slice(data);
    bytes[data.len()..].fill(0);
    put(
        &mut s.virtual_section,
        object + 0x18,
        &(PHYSICAL_TAG + at as u64).to_le_bytes(),
    )?;
    Ok(())
}
