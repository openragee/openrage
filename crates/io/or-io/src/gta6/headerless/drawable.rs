//! The drawable, its geometries over the physical section, and the root that holds a drawable
//! with a bound.

use super::bound::{bound_at, drawable_bound_at, Bound};
use super::bytes::{sections, u32_at, u64_at, Bytes};
use super::fragment::{fragment_at, Fragment};
use super::skeleton::{skeleton_at, Skeleton};
use super::streams::{self, buffer, Vertices, INDEX_BUFFER, VERTEX_BUFFER};
use super::surfaces::{surfaces_at, Surface};
use super::{HeaderlessError, Result};
use crate::container::rpf::Rpf9Role;

pub(super) const DRAWABLE_ROOT: usize = 0xE0;

#[derive(Debug)]
pub struct Drawable {
    pub name: String,
    pub box_min: [f32; 3],
    pub box_max: [f32; 3],
    pub shaders: Vec<Shader>,
    /// Four level slots, `None` where the slot is null.
    pub levels: [Option<Vec<Model>>; 4],
    /// The distance, in metres, each level slot is drawn to.
    pub lod_distances: [f32; 4],
    /// The composite bound the root points at from 0xC8, or its lone bound as a composite of one.
    pub bound: Option<Bound>,
    /// The surfaces the dictionary-shaped record at 0xD0 holds: triangles of their own in the
    /// drawable's frame, no level of detail's geometry.
    pub embedded: Option<Box<Result<Vec<Surface>>>>,
    /// The skeletons the root lists at 0xA0 besides its own: rigs of bones no other skeleton holds.
    pub extra_skeletons: Vec<Skeleton>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shader {
    pub name_hash: u32,
    /// One per texture slot: the 64-bit key a `.ttpf` directory lists, `None` for an empty slot.
    pub texture_keys: Vec<Option<u64>>,
    /// Each parameter record, the textures first.
    pub parameters: Vec<Parameter>,
}

/// One parameter record of a shader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parameter {
    /// The hash at +0x14 of the effect's record for it, which is not that record's name hash.
    pub hash: u32,
    pub value: ParameterValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParameterValue {
    /// Its slot in the texture table, whose key is the shader's `texture_keys[slot]`.
    Texture { slot: u8 },
    /// Its constant buffer, its byte offset in that buffer and its 4-byte words.
    Constant {
        buffer: u8,
        offset: u16,
        words: Vec<u32>,
    },
}

#[derive(Debug)]
pub struct Model {
    pub geometries: Vec<Result<Geometry>>,
    /// What the blend indices of the model's geometries index.
    pub palette: Result<Palette>,
    /// The bone of the drawable's own skeleton a geometry with no blend indices hangs off.
    pub matrix_index: u8,
}

/// What a model's blend indices index, by the list of the model's palette count it points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Palette {
    /// Index k is the bone whose name hash is entry k, in the wearer's skeleton; a 0 names none.
    Hashes(Vec<u32>),
    /// Index k is bone k of the drawable's own skeleton: the list is a bone mask, or there is none.
    Own,
}

#[derive(Debug)]
pub struct Geometry {
    /// An index into the drawable's shaders.
    pub shader: u16,
    pub vertex_count: u32,
    pub channels: Vec<Channel>,
    /// Triangles as indices into the vertices.
    pub triangles: Vec<[u32; 3]>,
    /// Read only where the decoded vertices reach the geometry's box.
    pub positions: Result<Vec<[f32; 3]>>,
    pub vertices: Vertices,
    /// The vertex data as the file stores it: the slot table, the quantisation block and the
    /// streams, which the game's mesh programs read as they are.
    // ponytail: a second copy of the streams beside the decoded channels; share the section's bytes when a large model's memory shows.
    pub vertex_data: Vec<u8>,
    /// The index data as the file stores it: the header, the meshlet records, the triangles and
    /// the vertex list.
    pub index_data: Vec<u8>,
}

/// One channel of a vertex declaration: its stream's offset in the vertex data, its element size
/// and its type byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Channel {
    pub index: u8,
    pub offset: u32,
    pub size: u8,
    /// A DXGI format number: 6 is three floats, and 0x10 holds channel 0's quantised position.
    pub code: u8,
}

#[derive(Debug)]
pub struct DrawableWithBound {
    pub name: String,
    pub drawable: Drawable,
    /// `None` where the root's record at 0x10 is null, as in fragments with no physics record.
    pub bound: Option<Bound>,
    /// Its physics level: the archetype, the groups and the children placed at their bones.
    pub fragment: Box<Fragment>,
}

pub fn read_drawable(virtual_section: &[u8], physical: Option<&[u8]>) -> Result<Drawable> {
    sections(virtual_section, physical, |v, p| drawable_at(v, p, 0, true))
}

/// The word a map archetype names the drawable or fragment at the root of `virtual_section` by: the
/// 64-bit hash of the name its root states at 0xA8, less a `.#dr` or `.#ft` ending.
pub fn stated_model_word(virtual_section: &[u8]) -> Result<u64> {
    sections(virtual_section, None, |v, _| {
        let name = v.c_str("name", v.to("name", 0xA8)?)?.to_ascii_lowercase();
        let stem = (name
            .strip_suffix(".#dr")
            .or_else(|| name.strip_suffix(".#ft")))
        .unwrap_or(&name);
        Ok(crate::gta6::hash::name_hash64(stem.as_bytes()))
    })
}

/// The name the drawable or fragment at the root of `virtual_section` states at 0xA8, as written.
pub fn stated_model_name(virtual_section: &[u8]) -> Result<String> {
    sections(virtual_section, None, |v, _| {
        v.c_str("name", v.to("name", 0xA8)?)
    })
}

pub fn read_drawable_with_bound(
    virtual_section: &[u8],
    physical: Option<&[u8]>,
) -> Result<DrawableWithBound> {
    sections(virtual_section, physical, |v, p| {
        let name = v.c_str("name", v.to("name", 0xA8)?)?;
        let bound = match v.ptr("record that holds the bound", 0x10)? {
            Some(record) => Some(bound_at(v, v.to("bound", record + 0x20)?)?),
            None => None,
        };
        // Its drawable holds a word other than a pointer where a drawable's bound is.
        let at = v.to("drawable", 0x18)?;
        let drawable = drawable_at(v, p, at, false)?;
        let fragment = Box::new(fragment_at(v, at, bound.as_ref())?);
        Ok(DrawableWithBound {
            name,
            drawable,
            bound,
            fragment,
        })
    })
}

/// The name each model of a body of `role` states and its shaders, read from the virtual section
/// alone: a drawable's own, a fragment's drawable under the fragment's name, each drawable of a
/// dictionary that reads; none of a body of another role.
pub fn model_shaders(role: Rpf9Role, virtual_section: &[u8]) -> Result<Vec<(String, Vec<Shader>)>> {
    sections(virtual_section, None, |v, _| match role {
        Rpf9Role::Drawable => Ok(vec![named_shaders(v, 0, 0)?]),
        Rpf9Role::DrawableWithBound => Ok(vec![named_shaders(v, 0, v.to("drawable", 0x18)?)?]),
        Rpf9Role::Dictionary => {
            let entries = super::dictionary::drawable_entries(v)?.into_iter();
            Ok(entries
                .filter_map(|at| named_shaders(v, at, at).ok())
                .collect())
        }
        _ => Ok(Vec::new()),
    })
}

/// The name the root at `named` states at 0xA8, and the shaders of the drawable at `at`.
fn named_shaders(v: Bytes, named: usize, at: usize) -> Result<(String, Vec<Shader>)> {
    let name = v.c_str("name", v.to("name", named + 0xA8)?)?;
    let shaders = shaders_at(v, at)?.into_iter().map(|(shader, _)| shader);
    Ok((name, shaders.collect()))
}

/// Whether a drawable root lies at `at`; a fragment's may have no level, under the empty box.
pub(super) fn is_drawable_root(v: Bytes, at: usize, fragment: bool) -> bool {
    let levels = (0..4).all(|l| matches!(v.u64("level", at + 0x50 + 8 * l), Ok(0)));
    let empty_box = (0..8).all(|k| {
        let empty = if k < 4 { 0x7F7F_FFFF } else { 0xFF7F_FFFF };
        matches!(v.u32("box", at + 0x30 + 4 * k), Ok(w) if w == empty)
    });
    matches!(v.ptr("shader group", at + 0x10), Ok(Some(_)))
        && ((matches!(v.ptr("level", at + 0x50), Ok(Some(_)))
            && v.f32("distance", at + 0x7C).is_ok_and(|d| d > 0.0))
            || (fragment && levels && empty_box))
}

/// The drawable at `at`, its word at 0xC8 read as a bound pointer where `bound_at_c8`; a
/// fragment's drawable is the one that does not.
pub(super) fn drawable_at(v: Bytes, p: Bytes, at: usize, bound_at_c8: bool) -> Result<Drawable> {
    if !is_drawable_root(v, at, !bound_at_c8) {
        return Err(HeaderlessError::NotADrawable { at: at as u64 });
    }
    v.get("drawable root", at, DRAWABLE_ROOT)?;
    let placed = shaders_at(v, at)?;
    let shader_count = placed.len() as u16;
    let shaders = placed.into_iter().map(|(s, _)| s).collect();
    let mut levels: [Option<Vec<Model>>; 4] = Default::default();
    for (l, slot) in levels.iter_mut().enumerate() {
        if let Some(level) = v.ptr("level", at + 0x50 + 8 * l)? {
            *slot = Some(level_at(v, p, level, shader_count)?);
        }
    }
    let bound = match bound_at_c8 {
        true => v.ptr("bound", at + 0xC8)?,
        false => None,
    };
    let bound = match bound {
        Some(b) => Some(drawable_bound_at(v, b)?),
        None => None,
    };
    let distance = |l: usize| v.f32("level distance", at + 0x70 + 4 * l);
    Ok(Drawable {
        name: v.c_str("name", v.to("name", at + 0xA8)?)?,
        box_min: v.vec3("box minimum", at + 0x30)?,
        box_max: v.vec3("box maximum", at + 0x40)?,
        shaders,
        levels,
        lod_distances: [distance(0)?, distance(1)?, distance(2)?, distance(3)?],
        bound,
        // A word there that is no pointer is not a record.
        embedded: (v.ptr("record at 0xD0", at + 0xD0).ok().flatten())
            .map(|record| Box::new(surfaces_at(v, record))),
        extra_skeletons: extra_skeletons(v, at)?,
    })
}

/// Bounds the count at 0x9C before it sizes the list of extra skeletons.
const MAX_EXTRA_SKELETONS: u32 = 64;

/// The skeletons the list at 0xA0 points at, its count at 0x9C; the word at 0x98 is zero.
fn extra_skeletons(v: Bytes, at: usize) -> Result<Vec<Skeleton>> {
    let word = v.u32("word before the extra skeleton count", at + 0x98)?;
    if word != 0 {
        return Err(HeaderlessError::Constant {
            what: "word before the extra skeleton count",
            at: (at + 0x98) as u64,
            found: word.into(),
        });
    }
    let Some(list) = v.ptr("extra skeletons", at + 0xA0)? else {
        return Ok(Vec::new());
    };
    let count = v.u32("extra skeleton count", at + 0x9C)?;
    if !(1..MAX_EXTRA_SKELETONS).contains(&count) {
        return Err(HeaderlessError::Constant {
            what: "extra skeleton count",
            at: (at + 0x9C) as u64,
            found: count.into(),
        });
    }
    let count = count as usize;
    v.array("extra skeletons", list, count, 8)?;
    (0..count)
        .map(|k| skeleton_at(v, v.to("extra skeleton", list + 8 * k)?))
        .collect()
}

/// Where a shader's values lie in the virtual section.
pub(super) struct Places {
    /// Each parameter's constant words; `None` for a texture.
    pub(super) constants: Vec<Option<usize>>,
    /// Each texture slot's key; `None` for an empty slot.
    pub(super) keys: Vec<Option<usize>>,
}

/// The shaders of the drawable whose root is at `at`, each with where its values lie.
pub(super) fn shaders_at(v: Bytes, at: usize) -> Result<Vec<(Shader, Places)>> {
    let group = v.to("shader group", at + 0x10)?;
    let shader_count = v.u16("shader count", group + 0x18)?;
    let shaders_at = match shader_count {
        0 => 0,
        _ => v.to("shaders", group + 0x10)?,
    };
    v.array("shaders", shaders_at, shader_count.into(), 8)?;
    (0..usize::from(shader_count))
        .map(|i| shader(v, v.to("shader", shaders_at + 8 * i)?))
        .collect()
}

/// A shader: its parameter infos at +0x20, a header then one hash and one packed word per record,
/// and its parameter block at +0x08, each constant buffer's pointer, the buffers 16-aligned and in
/// order, then the texture table +0x10 points at.
fn shader(v: Bytes, at: usize) -> Result<(Shader, Places)> {
    v.get("shader", at, 0x40)?;
    let infos = v.to("parameter infos", at + 0x20)?;
    let buffers = usize::from(v.u8("constant buffer count", infos)?);
    let slots = usize::from(v.u8("texture count", infos + 1)?);
    let count = usize::from(v.u8("parameter count", infos + 4)?);
    let records = v.get("parameter records", infos + 8, 8 * count)?;
    // A buffer ends where its last constant does, rounded up to 16 bytes.
    let mut ends = vec![0usize; buffers];
    let mut fields = Vec::with_capacity(count);
    for (k, r) in records.as_chunks::<8>().0.iter().enumerate() {
        let (hash, word) = (u32_at(r, 0), u32_at(r, 4));
        let word_at = (infos + 12 + 8 * k) as u64;
        let (buffer, offset, size) = (
            ((word >> 2) & 0x3F) as usize,
            ((word >> 8) & 0x3FFF) as usize,
            (word >> 22) as usize,
        );
        // Texture k's word is its kind 0 and its ordinal, and every texture comes first.
        let expected = match k < slots {
            true => 4 * k as u32,
            false => (word & !3) | 3,
        };
        if word != expected || (k >= slots && size == 0) {
            return Err(HeaderlessError::Constant {
                what: "parameter record's packed word",
                at: word_at,
                found: word.into(),
            });
        }
        if k >= slots {
            let end = ends
                .get_mut(buffer)
                .ok_or(HeaderlessError::IndexPastCount {
                    what: "constant's buffer",
                    index: buffer as u64,
                    count: buffers as u64,
                })?;
            *end = (*end).max(offset + 4 * size);
        }
        fields.push((hash, buffer, offset, size));
    }
    let mut starts = Vec::with_capacity(buffers);
    let mut texture_table = None;
    if buffers + slots > 0 {
        let block = v.to("parameter block", at + 0x08)?;
        let mut next = block + 8 * buffers;
        for (b, end) in ends.iter().enumerate() {
            let stated = v.to("constant buffer", block + 8 * b)?;
            if stated != next {
                return Err(HeaderlessError::Disagree {
                    what: "constant buffer's address and the end of the block before it",
                    first: stated as u64,
                    second: next as u64,
                });
            }
            starts.push(next);
            next += end.next_multiple_of(16);
        }
        texture_table = Some(next);
    }
    let mut constants = Vec::with_capacity(count);
    let parameters = (fields.into_iter().enumerate())
        .map(|(k, (hash, buffer, offset, size))| {
            let value = match k < slots {
                true => {
                    constants.push(None);
                    ParameterValue::Texture { slot: k as u8 }
                }
                false => {
                    let bytes = v.get("constant value", starts[buffer] + offset, 4 * size)?;
                    constants.push(Some(starts[buffer] + offset));
                    ParameterValue::Constant {
                        buffer: buffer as u8,
                        offset: offset as u16,
                        words: (bytes.as_chunks::<4>().0.iter())
                            .map(|w| u32::from_le_bytes(*w))
                            .collect(),
                    }
                }
            };
            Ok(Parameter { hash, value })
        })
        .collect::<Result<_>>()?;
    let mut texture_keys = Vec::with_capacity(slots);
    let mut keys = Vec::with_capacity(slots);
    if slots > 0 {
        let bindings = v.to("texture bindings", at + 0x10)?;
        let expected = texture_table.unwrap_or_default();
        if bindings != expected {
            return Err(HeaderlessError::Disagree {
                what: "texture table's address and the end of the constant buffers",
                first: bindings as u64,
                second: expected as u64,
            });
        }
        for k in 0..slots {
            let key = match v.ptr("texture binding", bindings + 8 * k)? {
                Some(binding) => {
                    let entry = v.to("texture entry", binding + 0x30)?;
                    Some((v.u64("texture key", entry + 8)?, entry + 8))
                }
                None => None,
            };
            texture_keys.push(key.map(|(key, _)| key));
            keys.push(key.map(|(_, at)| at));
        }
    }
    let shader = Shader {
        name_hash: v.u32("shader name hash", at)?,
        texture_keys,
        parameters,
    };
    Ok((shader, Places { constants, keys }))
}

fn level_at(v: Bytes, p: Bytes, at: usize, shaders: u16) -> Result<Vec<Model>> {
    let count = usize::from(v.u16("model count", at + 8)?);
    let models = match count {
        0 => 0,
        _ => v.to("models", at)?,
    };
    v.array("models", models, count, 8)?;
    (0..count)
        .map(|i| model(v, p, v.to("model", models + 8 * i)?, shaders))
        .collect()
}

fn model(v: Bytes, p: Bytes, at: usize, shaders: u16) -> Result<Model> {
    v.get("model", at, 0x38)?;
    let count = v.u16("geometry count", at + 0x10)?;
    let again = v.u16("geometry count", at + 0x36)?;
    if count != again {
        return Err(HeaderlessError::Disagree {
            what: "model's two geometry counts",
            first: count.into(),
            second: again.into(),
        });
    }
    let count = usize::from(count);
    let geometries = v.to("geometries", at + 0x08)?;
    let boxes = v.to("geometry boxes", at + 0x18)?;
    let shader_indices = v.to("shader indices", at + 0x20)?;
    v.array("geometries", geometries, count, 8)?;
    // With two or more geometries the union of their boxes comes first.
    let first_box = usize::from(count > 1);
    v.array("geometry boxes", boxes, count + first_box, 32)?;
    v.array("shader indices", shader_indices, count, 2)?;
    let mut read = Vec::with_capacity(count);
    for j in 0..count {
        let shader = v.u16("shader index", shader_indices + 2 * j)?;
        if shader >= shaders {
            return Err(HeaderlessError::IndexPastCount {
                what: "geometry's shader index",
                index: shader.into(),
                count: shaders.into(),
            });
        }
        let b = boxes + 32 * (j + first_box);
        let bounds = (v.vec3("box minimum", b)?, v.vec3("box maximum", b + 0x10)?);
        read.push(geometry(
            v,
            p,
            v.to("geometry", geometries + 8 * j)?,
            shader,
            bounds,
        ));
    }
    // The model's byte keeps a palette count's low 8 bits; a geometry's +0x2C states it whole.
    let byte = v.u8("palette count", at + 0x30)?;
    let whole = match count {
        0 => None,
        _ => v
            .u32(
                "geometry's palette count",
                v.to("geometry", geometries)? + 0x2C,
            )
            .ok(),
    };
    let palette_count = whole
        .filter(|&n| n > 0xFF && n & 0xFF == u32::from(byte))
        .map_or(usize::from(byte), |n| n as usize);
    Ok(Model {
        geometries: read,
        palette: palette(v, at, palette_count),
        matrix_index: v.u8("matrix index", at + 0x33)?,
    })
}

/// The model's palette: `count` entries at the pointer at 0x28.
///
/// ⚠ A mask states only its first ceil(count / 32) words; the words after it are another record's.
fn palette(v: Bytes, model: usize, count: usize) -> Result<Palette> {
    let list = match v.ptr("palette", model + 0x28)? {
        Some(list) if count > 0 => list,
        _ => return Ok(Palette::Own),
    };
    let mut hashes = v.get("palette", list, 4 * count).ok().map(|words| {
        (0..count)
            .map(|k| u32_at(words, 4 * k))
            .collect::<Vec<u32>>()
    });
    // One mask word under a count of 1 holds at most bit 0. A first entry of 0 before ascending
    // hashes names no bone; up to 32 entries such a list also passes as a mask, so it goes first.
    if let Some(hashes) =
        hashes.take_if(|h| h.windows(2).all(|w| w[0] < w[1]) && (count > 1 || h[0] > 0xFFFF))
    {
        return Ok(Palette::Hashes(hashes));
    }
    let words = v.get("palette mask", list, 4 * count.div_ceil(32))?;
    let last = u32_at(words, words.len() - 4);
    if count.is_multiple_of(32) || last >> (count % 32) == 0 {
        return Ok(Palette::Own);
    }
    // Neither form: distinct words, none 0 after the first, are bone hashes out of order.
    let unordered = |h: &mut Vec<u32>| {
        let mut sorted = h.clone();
        sorted.sort_unstable();
        count > 1 && !h[1..].contains(&0) && sorted.windows(2).all(|w| w[0] != w[1])
    };
    match hashes.take_if(unordered) {
        Some(hashes) => Ok(Palette::Hashes(hashes)),
        None => Err(HeaderlessError::Palette {
            at: list as u64,
            count: count as u32,
        }),
    }
}

fn geometry(
    v: Bytes,
    p: Bytes,
    at: usize,
    shader: u16,
    bounds: ([f32; 3], [f32; 3]),
) -> Result<Geometry> {
    v.get("geometry", at, 0x50)?;
    let vb = v.to("vertex buffer", at + 0x08)?;
    let data = buffer(v, p, "vertex data", vb, VERTEX_BUFFER)?;
    let (vertex_count, channels) =
        declaration(v, v.to("vertex declaration", vb + 0x40)?, data.len())?;
    streams::slot_table(data, &channels)?;
    let record = v.to("meshlet record", at + 0x40)?;
    let index = buffer(
        v,
        p,
        "index data",
        v.to("index buffer", record)?,
        INDEX_BUFFER,
    )?;
    let triangles = meshlets(v, index, record, vertex_count)?;
    let positions = streams::positions(data, &channels, vertex_count, &triangles, bounds);
    Ok(Geometry {
        shader,
        vertex_count,
        triangles,
        positions,
        vertices: streams::vertices(data, &channels, vertex_count),
        channels,
        vertex_data: data.to_vec(),
        index_data: index.to_vec(),
    })
}

/// The vertex count and channels of a structure-of-arrays declaration over `data_len` bytes.
pub(super) fn declaration(v: Bytes, at: usize, data_len: usize) -> Result<(u32, Vec<Channel>)> {
    let d = v.get("vertex declaration", at, 0x140)?;
    let refuse = |why| HeaderlessError::Declaration { at: at as u64, why };
    let packed = u64_at(d, 0x138);
    if packed & 1 == 0 {
        return Err(refuse("its structure-of-arrays bit is clear"));
    }
    let count = packed >> 10;
    let mut channels: Vec<Channel> = (0..52)
        .filter(|&k| d[0x104 + k] != 0)
        .map(|k| Channel {
            index: k as u8,
            offset: u32_at(d, 4 * k),
            size: d[0xD0 + k],
            code: d[0x104 + k],
        })
        .collect();
    let sizes: u64 = channels.iter().map(|c| u64::from(c.size)).sum();
    if sizes != (packed >> 2) & 0xFF {
        return Err(refuse("its channel sizes do not sum to its vertex size"));
    }
    channels.sort_by_key(|c| c.offset);
    let end = |c: &Channel| u64::from(c.offset) + u64::from(c.size) * count;
    // Each stream starts where the one before ends, rounded up to 4 bytes.
    if channels
        .windows(2)
        .any(|w| u64::from(w[1].offset) != end(&w[0]).next_multiple_of(4))
    {
        return Err(refuse("its channels do not follow one another"));
    }
    let last = channels
        .last()
        .map(end)
        .ok_or_else(|| refuse("it has no channel"))?;
    if last > data_len as u64 || data_len as u64 - last >= 16 {
        return Err(refuse(
            "its last channel does not end within 16 bytes of the buffer's end",
        ));
    }
    let count = u32::try_from(count).map_err(|_| refuse("its vertex count passes 32 bits"))?;
    channels.sort_by_key(|c| c.index);
    Ok((count, channels))
}

/// The triangles of a meshlet list: a header, meshlet records, triangle words and a vertex list.
fn meshlets(v: Bytes, data: &[u8], record: usize, vertex_count: u32) -> Result<Vec<[u32; 3]>> {
    let refuse = |why| HeaderlessError::Meshlets { why };
    let words = data.len() / 4;
    let word = |k: usize| {
        data.get(4 * k..4 * k + 4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .ok_or(refuse("it is shorter than its header states"))
    };
    let head = word(0)?;
    if head >> 31 == 0 {
        return Err(refuse("bit 31 of its first word is clear"));
    }
    let (m, flag) = ((head & 0xFFFF) as usize, (head >> 30) & 1);
    let (t, list) = (word(1)? as usize, word(2)? as usize);
    let stated = v.u32("meshlet record's counts", record + 0x0C)?;
    if usize::from(v.u16("meshlet record's count", record + 0x08)?) != m
        || usize::from(v.u16("meshlet record's count", record + 0x0A)?) != t & 0xFFFF
        || ((stated >> 8) & 0x7F_FFFF) as usize != list
        || stated >> 31 != flag
    {
        return Err(refuse("its counts differ from its meshlet record's"));
    }
    // Bit 30 adds ceil(m/2) words, placed after the vertex list.
    let extra = if flag == 1 { m.div_ceil(2) } else { 0 };
    if (3 + 4 * m + t + list.div_ceil(2) + extra).next_multiple_of(4) != words {
        return Err(refuse("its length is not the sum of its parts"));
    }
    let mut starts = Vec::with_capacity(m);
    for j in 0..m {
        let w = word(3 + 4 * j + 1)?;
        starts.push(((w & 0xFFFF) as usize, (w >> 16) as usize));
    }
    let ordered = starts.first().is_none_or(|s| *s == (0, 0))
        && starts
            .windows(2)
            .all(|w| w[1].0 > w[0].0 && w[1].1 > w[0].1)
        && starts.last().is_none_or(|s| s.0 <= list);
    if !ordered || (m == 0 && t > 0) {
        return Err(refuse(
            "its meshlet offsets do not start at zero and increase",
        ));
    }
    let list_at = 4 * (3 + 4 * m + t);
    let entry = |i: usize| u16::from_le_bytes([data[list_at + 2 * i], data[list_at + 2 * i + 1]]);
    let mut triangles = Vec::with_capacity(t);
    let mut k = 0;
    for i in 0..t {
        while k + 1 < m && i >= starts[k + 1].1 {
            k += 1;
        }
        let x = word(3 + 4 * m + i)?;
        if (x >> 21) as usize != k & 0x7FF {
            return Err(refuse("a triangle's meshlet bits do not name its meshlet"));
        }
        let first = starts[k].0;
        let held = starts.get(k + 1).map_or(list, |s| s.0) - first;
        let mut triangle = [0u32; 3];
        for (c, corner) in triangle.iter_mut().enumerate() {
            let local = ((x >> (7 * c)) & 0x7F) as usize;
            if local >= held {
                return Err(HeaderlessError::IndexPastCount {
                    what: "triangle's index into its meshlet",
                    index: local as u64,
                    count: held as u64,
                });
            }
            *corner = u32::from(entry(first + local));
            if *corner >= vertex_count {
                return Err(HeaderlessError::IndexPastCount {
                    what: "vertex list entry",
                    index: (*corner).into(),
                    count: vertex_count.into(),
                });
            }
        }
        triangles.push(triangle);
    }
    Ok(triangles)
}
