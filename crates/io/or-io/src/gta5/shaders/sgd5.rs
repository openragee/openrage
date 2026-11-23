//! GTA VI's `SGD5` effect container: Enhanced's `SGD2` with a head of shared binding layouts and a
//! global variable table, six program banks, each program a raw DEFLATE stream of a DXIL
//! container, and variable records that state name hashes where `SGD2` states names.
//!
//! The file states no end and no table of contents, so consuming it exactly to its trailer, with
//! every count and reference in range, is the check that the read landed on the right fields. A
//! piece of the file a download has not delivered reads as zeros: the read steps over it to the
//! next program or effect and says what it could not reach. Read-only.

use std::collections::HashMap;
use std::ops::Range;

use flate2::{Decompress, FlushDecompress, Status};
use serde::{Deserialize, Serialize};

use crate::gta5::shaders::sga::{SgaVariable, SgaVariableType};
use crate::gta5::shaders::shader_defs::{
    sga_parameter, ShaderDef, ShaderDefinitions, ShaderParamDef, ShaderParamValue,
};
use crate::gta6::hash::name_hash32;

const MAGIC: &[u8; 4] = b"SGD5";
/// Program banks: vertex, pixel, geometry, two empty in every container read, and compute.
pub const BANKS: usize = 6;
/// The word that opens every program's DEFLATE frame.
const FRAME: u32 = 0x000D_DCE7;
/// Bytes after a program's stream, of which only the first 8, its key, are read.
const PROGRAM_STATE: usize = 152;
const EFFECT_STATE: usize = 80;
/// Rasterizer, depth-stencil, blend and sampler state records, then techniques.
const STATE_STRIDES: [usize; 5] = [16, 24, 68, 36, 4];
const PASS: usize = 24;
const RECORD: usize = 24;
const ANNOTATION: usize = 8;
const CONSTANT_BUFFER: u8 = 0x30;
/// The slot bit of a constant buffer whose values a drawable holds; the engine fills the others.
const DRAWABLE_BUFFER_SLOT: u16 = 0x100;
const LAYOUT_HEAD: usize = 24;
/// An effect's layout item that names a head layout rather than holding one.
const LAYOUT_REFERENCE: u32 = 0x8000_0000;
/// The page a piece not delivered is judged on, and the zeros that start a probe for one.
const PIECE_PAGE: usize = 0x1_0000;
const ZERO_PROBE: usize = 512;
const TRAILER: u8 = 0x01;
const TRAILER_PAGE: usize = 0x1000;
// Bounds a count or size read from the file holds before it sizes a loop or an allocation.
const MAX_HEAD_LAYOUTS: u32 = 4096;
const MAX_COUNT: u32 = 1_000_000;
const MAX_LIST: u32 = 65_536;
const MAX_TABLE: usize = 0x100_0000;
const MAX_BOUND_WORDS: usize = 0x40_0000;
const MAX_NAMES: usize = 0x10_0000;
const MAX_INFLATED: usize = 64 << 20;

/// Why an `SGD5` container, or a part of one, is not read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Sgd5Error {
    #[error("at +{at:#X} the container does not hold to the measured layout: {why}")]
    Invalid { at: usize, why: String },
    #[error(
        "what the read needs at +{at:#X} is in a piece of the file a download has not delivered"
    )]
    NotOnDisk { at: usize },
    #[error(
        "program {index} of bank {bank} is in a piece of the file a download has not delivered"
    )]
    ProgramNotOnDisk { bank: usize, index: u32 },
    #[error("bank {bank} states no program {index}")]
    NoSuchProgram { bank: usize, index: u32 },
    #[error("each program is a DXIL container (shader model 6), and the shader front end translates DXBC shader model 5 tokens, so none is translated")]
    NotTranslated,
    #[error("saving an SGD5 container is refused: OpenRage reads GTA VI's files and writes none, and a program carries a digest and a key that tie it to the pipeline cache")]
    ReadOnly,
}

type Read<T> = Result<T, Sgd5Error>;

/// A read container: its head, its programs as far as they are on disk, and its effects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sgd5File {
    /// The u64 after the magic, whose meaning is not known.
    pub head_word: u64,
    /// The binding layouts effects share, referenced by [`Sgd5Layout::id`].
    pub layouts: Vec<Sgd5Layout>,
    pub globals: Sgd5Table,
    /// Each bank's stated program count, `None` where the count is not on disk.
    pub bank_counts: [Option<u32>; BANKS],
    /// Every program read, in file order.
    pub programs: Vec<Sgd5Program>,
    /// The pieces not on disk the read stepped over.
    pub gaps: Vec<Sgd5Gap>,
    /// `None` where the count is not on disk; `effects` then holds those after the last gap.
    pub effect_count: Option<u32>,
    pub effects: Vec<Sgd5Effect>,
}

/// A run of the file a download has not delivered, and where the read resumed after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sgd5Gap {
    /// Where the structure the read was on starts.
    pub from: usize,
    /// The next program or effect; `None` where nothing after it was read.
    pub resume: Option<usize>,
}

/// One compiled program, located in the container rather than copied out of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sgd5Program {
    /// `None` where the bank's count is not on disk.
    pub bank: Option<usize>,
    /// The program's index in its bank; `None` between two pieces not on disk.
    pub index: Option<u32>,
    pub name: String,
    /// The byte after the name, whose meaning is not known.
    pub kind: u8,
    /// The first u64 of the state after the stream, which the pipeline cache keys a pipeline by.
    pub key: u64,
    pub at: usize,
    /// The raw DEFLATE stream, as a range of the container.
    pub stream: Range<usize>,
    pub inflated_len: usize,
    pub variables: Sgd5Table,
}

/// One effect: its programs per bank, techniques, passes, parameters and binding layouts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sgd5Effect {
    pub name: String,
    pub at: usize,
    /// The u32 after the name, whose meaning is not known.
    pub word: u32,
    /// Indices into each bank's programs.
    pub programs: [Vec<u32>; BANKS],
    /// Rasterizer, depth-stencil, blend and sampler state records, whose bytes are not decoded.
    pub state_counts: [usize; 4],
    pub techniques: Vec<Sgd5Technique>,
    pub passes: Vec<Sgd5Pass>,
    /// Each pass's bound-name lists, in pass order.
    pub bound: Vec<Sgd5BoundPass>,
    pub parameters: Sgd5Table,
    /// The second variable table, which holds a constant buffer and annotations.
    pub second: Sgd5Table,
    /// The top byte of the layout count, a flag whose meaning is not known.
    pub layout_flag: u8,
    /// The ids of the head layouts the effect references.
    pub layout_refs: Vec<u32>,
    /// The layouts the effect holds itself.
    pub layouts: Vec<Sgd5Layout>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sgd5Technique {
    pub name: String,
    /// Bit 15 of the name offset, whose meaning is not known.
    pub flag: bool,
    pub index: u8,
    pub passes: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sgd5Pass {
    pub name: String,
    /// Per bank, 1 + an index into the effect's program list for that bank; 0 for none.
    pub programs: [u8; BANKS],
}

/// What a pass binds, by the +0x10 hash of the effect's resource records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sgd5BoundPass {
    /// Shader resources, unordered-access resources, a list always empty, and constant buffers.
    pub lists: [Vec<u32>; 4],
    /// The word whose bits 8 and up are 1 that ends a pass in some effects; its low byte is not known.
    pub trailing: Option<u32>,
}

/// A table of binding slots: a name hash, a kind and a slot per record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sgd5Layout {
    pub hash: u64,
    /// Records per class, in class order, which sum to the record count; see [`class_of`].
    pub classes: [u8; 6],
    /// The id an effect references a head layout by.
    pub id: u32,
    pub bindings: Vec<Sgd5Binding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sgd5Binding {
    pub hash: u32,
    pub kind: u8,
    pub flags: u8,
    pub index: u16,
}

/// A variable table: resources, constants, annotations and a pool of names.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sgd5Table {
    pub resources: Vec<Sgd5Resource>,
    pub constants: Vec<Sgd5Constant>,
    /// Name hash and value.
    pub annotations: Vec<(u32, u32)>,
    /// The pool's strings: samplers' names, and values of annotations.
    pub names: Vec<String>,
    pub pool_len: usize,
}

/// A resource binding, or a constant buffer whose members are constants of the same table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sgd5Resource {
    /// `SGD2`'s resource type code.
    pub code: u8,
    pub flags: u8,
    /// The register.
    pub slot: u16,
    /// A constant buffer's member count.
    pub members: u8,
    /// A constant buffer's size in bytes: the low half of +0x0C, whose high half is not known.
    pub buffer_size: u32,
    pub name_hash: u32,
    /// The hash at +0x14 a drawable's parameter record states, which can differ from `name_hash`.
    pub semantic_hash: u32,
}

/// A member of a constant buffer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sgd5Constant {
    /// `SGD2`'s constant type code.
    pub code: u8,
    /// Array length in elements; 0 for a scalar.
    pub count: u16,
    /// Byte offset into the buffer.
    pub offset: u16,
    /// Where its default lies, from the start of its record; 0 for none.
    pub default_offset: u16,
    /// The default's words, `4 × units × max(count, 1)` bytes; `None` where none is set, or where
    /// its type's size is not fixed.
    pub default: Option<Vec<u32>>,
    pub name_hash: u32,
    pub semantic_hash: u32,
    /// The name hash of the constant buffer it is a member of.
    pub cbuffer: Option<u32>,
}

impl Sgd5Table {
    /// The pool string that hashes to `hash`: only samplers have one.
    pub fn name_of(&self, hash: u32) -> Option<&str> {
        (self.names.iter())
            .find(|n| name_hash32(n.as_bytes()) == hash)
            .map(String::as_str)
    }
}

impl Sgd5Program {
    /// The DXIL container the stream inflates to, from `container`, the bytes it was read from.
    pub fn dxil(&self, container: &[u8]) -> Result<Vec<u8>, Sgd5Error> {
        let stream = container
            .get(self.stream.clone())
            .ok_or_else(|| invalid(self.at, "the program's stream is past this container's end"))?;
        inflate(stream, self.inflated_len).ok_or_else(|| {
            invalid(
                self.stream.start,
                format!("program `{}` does not inflate to its container", self.name),
            )
        })
    }

    /// The DXIL container `stream` inflates to, where `stream` is this program's stream read alone.
    pub fn inflate(&self, stream: &[u8]) -> Result<Vec<u8>, Sgd5Error> {
        if stream.len() != self.stream.len() {
            return Err(invalid(
                self.stream.start,
                format!(
                    "program `{}` states a {}-byte stream and {} bytes were read",
                    self.name,
                    self.stream.len(),
                    stream.len()
                ),
            ));
        }
        inflate(stream, self.inflated_len).ok_or_else(|| {
            invalid(
                self.stream.start,
                format!("program `{}` does not inflate to its container", self.name),
            )
        })
    }
}

impl Sgd5File {
    /// The container in `bytes`, which may run past its trailer into zero pad.
    pub fn open(bytes: &[u8]) -> Result<Self, Sgd5Error> {
        let b = bytes;
        if take(b, 0, 4)? != MAGIC {
            return Err(invalid(0, "expected `SGD5`"));
        }
        let head_word = u64_at(b, 4)?;
        let count = u32_at(b, 0xC)?;
        if count > MAX_HEAD_LAYOUTS {
            return Err(invalid(0xC, format!("{count} head layouts")));
        }
        let mut p = 0x10;
        let mut layouts = Vec::new();
        for _ in 0..count {
            let size = u32_at(b, p)? as usize;
            let layout = layout(take(b, p + 4, size)?, p)?;
            if !(layout.bindings.iter().map(|r| usize::from(r.index))).eq(0..layout.bindings.len())
            {
                return Err(invalid(
                    p,
                    "a head layout's record indices do not run 0..n-1",
                ));
            }
            layouts.push(layout);
            p += 4 + size;
        }
        let (p, globals) = table(b, p)?;
        let (items, mut gaps, effects_at) = walk(b, p)?;
        let (bank_counts, programs) = banks(items, &effects_at)?;

        let heads: Vec<u32> = layouts.iter().map(|l| l.id).collect();
        let mut effects = Vec::new();
        let mut read = |at: usize| -> Read<usize> {
            let (next, e) = effect(b, at)?;
            check_effect(&e, &bank_counts, &heads)?;
            effects.push(e);
            Ok(next)
        };
        // Where the trailer is, unless a piece not delivered cuts the effects short.
        let (effect_count, trailer) = match effects_at {
            EffectsAt::Count(at) => {
                let count = u32_at(b, at)?;
                let mut q = Some(at + 4);
                for _ in 0..count {
                    let Some(from) = q else { break };
                    match read(from) {
                        Ok(next) => q = Some(next),
                        Err(Sgd5Error::NotOnDisk { .. }) => {
                            gaps.push(Sgd5Gap { from, resume: None });
                            q = None;
                        }
                        Err(e) => return Err(e),
                    }
                }
                (Some(count), q)
            }
            EffectsAt::After(mut q) => {
                while b.get(q) != Some(&TRAILER) {
                    q = read(q)?;
                }
                (None, Some(q))
            }
        };
        if let Some(at) = trailer {
            if b.get(at) != Some(&TRAILER) {
                return Err(invalid(at, "no trailer byte after the last effect"));
            }
            let page_end = (at + 1).next_multiple_of(TRAILER_PAGE).min(b.len());
            if b[at + 1..page_end].iter().any(|x| *x != 0) {
                return Err(invalid(
                    at + 1,
                    "bytes after the trailer before the page ends",
                ));
            }
        }
        Ok(Self {
            head_word,
            layouts,
            globals,
            bank_counts,
            programs,
            gaps,
            effect_count,
            effects,
        })
    }

    /// Program `index` of bank `bank`, or why it was not read.
    pub fn program(&self, bank: usize, index: u32) -> Result<&Sgd5Program, Sgd5Error> {
        if let Some(p) =
            (self.programs.iter()).find(|p| p.bank == Some(bank) && p.index == Some(index))
        {
            return Ok(p);
        }
        match self.bank_counts.get(bank) {
            Some(Some(count)) if index < *count => Err(Sgd5Error::ProgramNotOnDisk { bank, index }),
            Some(None) => Err(Sgd5Error::ProgramNotOnDisk { bank, index }),
            _ => Err(Sgd5Error::NoSuchProgram { bank, index }),
        }
    }

    /// Always refused: see [`Sgd5Error::ReadOnly`].
    pub fn save(&self) -> Result<Vec<u8>, Sgd5Error> {
        Err(Sgd5Error::ReadOnly)
    }
}

/// The shader definitions the material code reads, one per effect of `files`, keyed by name hash.
// Each parameter of an effect's first table keeps the name hash its record states at +0x10 and,
// as its semantic, the hash at +0x14 a drawable states; it is named where the pool names it and
// `hash_XXXXXXXX` elsewhere, and keeps the default its table sets.
pub fn shader_definitions(
    title: impl Into<String>,
    target: impl Into<String>,
    files: &[Sgd5File],
) -> ShaderDefinitions {
    let mut out = ShaderDefinitions::new(title, target);
    for effect in files.iter().flat_map(|f| &f.effects) {
        let table = &effect.parameters;
        let name = |hash: u32| {
            (table.name_of(hash).map(str::to_string)).unwrap_or_else(|| format!("hash_{hash:08X}"))
        };
        let def = |var_type, count, slot, cbuffer, name_hash: u32, semantic_hash: u32| {
            let variable = SgaVariable {
                name: name(name_hash),
                semantic: None,
                var_type,
                count,
                slot,
                cbuffer,
                default: Vec::new(),
            };
            let mut param = sga_parameter(&variable);
            param.name_hash = name_hash;
            param.semantic_hash = (semantic_hash != name_hash).then_some(semantic_hash);
            param
        };
        let resources = (table.resources.iter())
            .filter(|r| r.code != CONSTANT_BUFFER)
            .map(|r| {
                let var_type = SgaVariableType::resource(r.code);
                def(var_type, 0, r.slot, None, r.name_hash, r.semantic_hash)
            });
        // A drawable holds only the buffers whose slot has bit 8 set, each named by its rank among them.
        let held: Vec<u32> = (table.resources.iter())
            .filter(|r| r.code == CONSTANT_BUFFER && r.slot & DRAWABLE_BUFFER_SLOT != 0)
            .map(|r| r.name_hash)
            .collect();
        let constants = table.constants.iter().map(|c| {
            let var_type = SgaVariableType::constant(c.code);
            let cbuffer = c.cbuffer.map(name);
            let mut param = def(
                var_type,
                c.count,
                c.offset,
                cbuffer,
                c.name_hash,
                c.semantic_hash,
            );
            param.drawable_place = (c.cbuffer)
                .and_then(|b| held.iter().position(|h| *h == b))
                .and_then(|rank| u8::try_from(rank).ok())
                .map(|rank| (rank, c.offset));
            param.default = c.default.clone().map(|words| match c.code {
                0x09..=0x0E => {
                    ShaderParamValue::Floats(words.into_iter().map(f32::from_bits).collect())
                }
                _ => ShaderParamValue::Uints(words),
            });
            param
        });
        let parameters = resources.chain(constants).collect();
        out.shaders.push(ShaderDef {
            name: effect.name.clone(),
            name_hash: name_hash32(effect.name.as_bytes()),
            effect: effect.name.clone(),
            draw_bucket: None,
            vertex_type: 0,
            traits: Vec::new(),
            parameters,
        });
    }
    out.shaders.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The parameter of `effect` a drawable's record of `hash` names: the one stating it at +0x14.
// Whatever the record's name hash at +0x10 is: a drawable never states that one.
pub fn drawable_parameter(effect: &ShaderDef, hash: u32) -> Option<&ShaderParamDef> {
    (effect.parameters.iter()).find(|p| p.semantic_hash.unwrap_or(p.name_hash) == hash)
}

/// Whether a constant `def` declares holds floats, whose words then read as `f32`.
pub fn holds_floats(def: &ShaderParamDef) -> bool {
    use crate::gta5::shaders::shader_defs::ShaderParamKind as K;
    matches!(
        def.kind,
        K::Float | K::Float2 | K::Float3 | K::Float4 | K::Float3x4 | K::Float4x4
    )
}

/// Whether a constant `def` declares holds 32-bit integers: GTA VI's two series of one to four, types 0x01 to 0x08.
pub fn holds_integers(def: &ShaderParamDef) -> bool {
    use crate::gta5::shaders::shader_defs::ShaderParamKind as K;
    matches!(def.kind, K::Int | K::Unknown(0x02..=0x08))
}

/// The 32-bit words one element of a constant of type `code` holds; `None` for a structure, 0x0F.
// 1 to 4 in each integer series, 0x01 to 0x04 and 0x05 to 0x08, and in the floats, 0x09 to 0x0C,
// 16 in 0x0E's 4×4 and 1 in 0x00's boolean; a structure's is the size of its room.
pub fn constant_units(code: u8) -> Option<usize> {
    match code {
        0x00 => Some(1),
        0x01..=0x08 => Some(usize::from(code - 1) % 4 + 1),
        0x09..=0x0C => Some(usize::from(code) - 8),
        0x0E => Some(16),
        _ => None,
    }
}

/// A type-0x03 constant's three words as 16 entries, each a 4-bit value and a 2-bit flag.
pub fn remap_table([w0, w1, w2]: [u32; 3]) -> [(u8, u8); 16] {
    std::array::from_fn(|i| {
        let lane = |w: u32| ((w >> (2 * i)) & 3) as u8;
        ((lane(w1) << 2) | lane(w0), lane(w2))
    })
}

/// Whether `words` are the remap table that maps every entry to itself with no flag set.
pub fn is_identity_remap(words: &[u32]) -> bool {
    let table = |w: &[u32]| remap_table([w[0], w[1], w[2]]);
    words.len() == 3 && (table(words).iter().enumerate()).all(|(i, e)| *e == (i as u8, 0))
}

/// Refuses a member of a constant buffer of `size` bytes whose room, to the next member or to the
/// buffer's end, does not fit its type: a 0x00 is one aligned word with no default, a 0x0F has no
/// default and an array of it divides its room, and a 0x03 is three words and no array.
fn fits_rooms(members: &[Sgd5Constant], size: u32, at: usize) -> Read<()> {
    let mut by_offset: Vec<&Sgd5Constant> = members.iter().collect();
    by_offset.sort_by_key(|c| c.offset);
    for (k, c) in by_offset.iter().enumerate() {
        let end = by_offset
            .get(k + 1)
            .map_or(size, |next| u32::from(next.offset));
        let room = end.saturating_sub(u32::from(c.offset));
        let (count, default) = (u32::from(c.count), c.default_offset != 0);
        let fits = match c.code {
            0x00 => room >= 4 && c.offset % 4 == 0 && count == 0 && !default,
            0x0F => !default && room >= 4 && (count == 0 || room % count == 0),
            0x03 => room >= 12 && count == 0,
            _ => true,
        };
        if !fits {
            return Err(invalid(
                at,
                format!(
                    "constant {:08X} of type {:#04X} does not fit its {room} bytes of room",
                    c.name_hash, c.code
                ),
            ));
        }
    }
    Ok(())
}

/// Each name and semantic `defs` declares, by its hash, to name a GTA VI parameter that hashes the same.
pub fn parameter_names(defs: &ShaderDefinitions) -> HashMap<u32, String> {
    let mut names = HashMap::new();
    for p in defs.shaders.iter().flat_map(|s| &s.parameters) {
        names.entry(p.name_hash).or_insert_with(|| p.name.clone());
        if let (Some(hash), Some(semantic)) = (p.semantic_hash, &p.semantic) {
            names.entry(hash).or_insert_with(|| semantic.clone());
        }
    }
    names
}

fn invalid(at: usize, why: impl Into<String>) -> Sgd5Error {
    Sgd5Error::Invalid {
        at,
        why: why.into(),
    }
}

/// Whether 64 KiB of zeros start at `at`: a piece a download has not delivered.
fn zero_piece(b: &[u8], at: usize) -> bool {
    (at.checked_add(PIECE_PAGE).and_then(|end| b.get(at..end)))
        .is_some_and(|p| p.iter().all(|x| *x == 0))
}

/// Whether a piece not delivered starts at `at`. A piece is a whole number of 64 KiB, so the zero
/// run that holds it is measured from its end: zero bytes before a piece are data.
fn piece_starts(b: &[u8], at: usize) -> bool {
    zero_piece(b, at) && b[at..].iter().take_while(|x| **x == 0).count() % PIECE_PAGE == 0
}

/// The first place in `from..to` where `run` zero bytes start a piece not delivered.
fn piece_in(b: &[u8], from: usize, to: usize, run: usize) -> Option<usize> {
    let mut zeros = 0;
    // A zero run that does not start a piece cannot have one start inside it.
    for (k, x) in b.get(from..to.min(b.len()))?.iter().enumerate() {
        zeros = if *x == 0 { zeros + 1 } else { 0 };
        let start = from + k + 1 - zeros;
        if zeros == run && zero_piece(b, start) {
            return Some(start);
        }
    }
    None
}

/// `n` bytes at `at`; refused where they run past the end or into a piece not delivered.
fn take(b: &[u8], at: usize, n: usize) -> Read<&[u8]> {
    let bytes = (at.checked_add(n).and_then(|end| b.get(at..end))).ok_or_else(|| {
        invalid(
            at,
            format!("{n} bytes run past the container's end at +{:#X}", b.len()),
        )
    })?;
    if n >= ZERO_PROBE {
        if let Some(gap) = piece_in(b, at, at + n, ZERO_PROBE) {
            return Err(Sgd5Error::NotOnDisk { at: gap });
        }
    }
    Ok(bytes)
}

fn u8_at(b: &[u8], at: usize) -> Read<u8> {
    Ok(take(b, at, 1)?[0])
}

fn u32_at(b: &[u8], at: usize) -> Read<u32> {
    let w = take(b, at, 4)?;
    Ok(u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
}

fn u64_at(b: &[u8], at: usize) -> Read<u64> {
    Ok(u64::from(u32_at(b, at)?) | u64::from(u32_at(b, at + 4)?) << 32)
}

/// A length byte that counts the NUL, then a name of printable ASCII without spaces; `None`
/// where the bytes at `at` are not one.
fn name_at(b: &[u8], at: usize) -> Read<Option<(String, usize)>> {
    let len = usize::from(u8_at(b, at)?);
    let raw = take(b, at + 1, len)?;
    Ok(match raw.split_last() {
        Some((0, name)) if !name.is_empty() && name.iter().all(|c| (0x21..=0x7E).contains(c)) => {
            Some((String::from_utf8_lossy(name).into_owned(), at + 1 + len))
        }
        _ => None,
    })
}

fn is_program(b: &[u8], at: usize) -> bool {
    let frame = |q: usize| -> Read<bool> {
        let size = u32_at(b, q + 1)?;
        Ok(u32_at(b, q + 5)? == FRAME && u32_at(b, q + 9)? == size)
    };
    matches!(name_at(b, at), Ok(Some((_, q))) if frame(q).unwrap_or(false))
}

/// `stream` inflated as raw DEFLATE, where it ends exactly and gives `len` bytes.
fn inflate(stream: &[u8], len: usize) -> Option<Vec<u8>> {
    let mut z = Decompress::new(false);
    let mut out = Vec::with_capacity(len + 1);
    let status = z
        .decompress_vec(stream, &mut out, FlushDecompress::Finish)
        .ok()?;
    (status == Status::StreamEnd && z.total_in() == stream.len() as u64 && out.len() == len)
        .then_some(out)
}

/// The program at `at`, its stream checked to inflate to a DXBC container of its stated size.
fn program(b: &[u8], at: usize) -> Read<(usize, Sgd5Program)> {
    let (name, q) = name_at(b, at)?.ok_or_else(|| invalid(at, "no program name"))?;
    let kind = u8_at(b, q)?;
    // The size counts the bytes from the frame word to the stream's end.
    let frame_at = q + 5;
    let (size, frame) = (u32_at(b, q + 1)? as usize, u32_at(b, frame_at)?);
    let (size2, raw) = (
        u32_at(b, frame_at + 4)? as usize,
        u32_at(b, frame_at + 8)? as usize,
    );
    if frame != FRAME || size2 != size || size < 12 || raw > MAX_INFLATED {
        return Err(invalid(
            at,
            format!("program `{name}` states frame {frame:#010X}, sizes {size} and {size2}, {raw} inflated"),
        ));
    }
    let stream = frame_at + 12..frame_at + size;
    let body = take(b, stream.start, size - 12)?;
    let total = (raw as u32).to_le_bytes();
    let dxbc = inflate(body, raw)
        .filter(|out| out.starts_with(b"DXBC") && out.get(24..28) == Some(&total[..]));
    if dxbc.is_none() {
        if let Some(gap) = piece_in(b, frame_at, frame_at + size + 64, 64) {
            return Err(Sgd5Error::NotOnDisk { at: gap });
        }
        return Err(invalid(
            at,
            format!("program `{name}` is not a raw DEFLATE stream of a {raw}-byte DXBC container"),
        ));
    }
    let key = u64_at(b, stream.end)?;
    let (next, variables) = table(b, stream.end + PROGRAM_STATE)?;
    let program = Sgd5Program {
        bank: None,
        index: None,
        name,
        kind,
        key,
        at,
        stream,
        inflated_len: raw,
        variables,
    };
    Ok((next, program))
}

/// A variable table at `at`: u32 size, then {u16 resources, u16 constants, u16 annotations, u16
/// size}, 24-byte resource and constant records, 8-byte annotations and a name pool.
fn table(b: &[u8], at: usize) -> Read<(usize, Sgd5Table)> {
    let size = u32_at(b, at)? as usize;
    if !(8..=MAX_TABLE).contains(&size) {
        return Err(invalid(at, format!("a variable table of {size} bytes")));
    }
    let t = take(b, at + 4, size)?;
    let word = |o: usize| u16::from_le_bytes([t[o], t[o + 1]]);
    let dword = |o: usize| u32::from_le_bytes([t[o], t[o + 1], t[o + 2], t[o + 3]]);
    let (resources, constants) = (usize::from(word(0)), usize::from(word(2)));
    let (annotations, stated) = (usize::from(word(4)), usize::from(word(6)));
    let first_constant = 8 + RECORD * resources;
    let pool_at = first_constant + RECORD * constants + ANNOTATION * annotations;
    if stated != size & 0xFFFF || pool_at > size {
        return Err(invalid(
            at,
            format!("a {size}-byte table states {resources} resources, {constants} constants, {annotations} annotations and {stated} bytes"),
        ));
    }
    let mut out = Sgd5Table {
        constants: (0..constants)
            .map(|k| {
                let r = first_constant + RECORD * k;
                let (code, count, default_offset) = (t[r], word(r + 2), word(r + 10));
                let start = r + usize::from(default_offset);
                let len = constant_units(code).map(|units| 4 * units * usize::from(count.max(1)));
                let end = start + len.unwrap_or(1);
                if default_offset != 0 && (start < pool_at || end > size) {
                    return Err(invalid(
                        at + 4 + r,
                        format!("a constant's default at +{default_offset} from its record lies outside its table's pool"),
                    ));
                }
                let default = len.filter(|_| default_offset != 0).map(|len| {
                    (t[start..start + len].as_chunks::<4>().0.iter())
                        .map(|w| u32::from_le_bytes(*w))
                        .collect()
                });
                Ok(Sgd5Constant {
                    code,
                    count,
                    offset: word(r + 4),
                    default_offset,
                    default,
                    name_hash: dword(r + 16),
                    semantic_hash: dword(r + 20),
                    cbuffer: None,
                })
            })
            .collect::<Read<_>>()?,
        annotations: (0..annotations)
            .map(|k| {
                let r = first_constant + RECORD * constants + ANNOTATION * k;
                (dword(r), dword(r + 4))
            })
            .collect(),
        names: pool_names(&t[pool_at..]),
        pool_len: size - pool_at,
        ..Sgd5Table::default()
    };
    for i in 0..resources {
        let r = 8 + RECORD * i;
        let resource = Sgd5Resource {
            code: t[r],
            flags: t[r + 1],
            slot: word(r + 2),
            members: t[r + 4],
            buffer_size: dword(r + 12) & 0xFFFF,
            name_hash: dword(r + 16),
            semantic_hash: dword(r + 20),
        };
        if resource.code == CONSTANT_BUFFER && resource.members > 0 {
            // The first member is stated relative to this record.
            let members = usize::from(resource.members);
            let first = (r + usize::from(word(r + 6)))
                .checked_sub(first_constant)
                .filter(|o| o % RECORD == 0)
                .map(|o| o / RECORD)
                .filter(|k| k + members <= constants)
                .ok_or_else(|| {
                    invalid(
                        at + 4 + r,
                        format!(
                            "constant buffer {:08X} places its members outside its table",
                            resource.name_hash
                        ),
                    )
                })?;
            for c in &mut out.constants[first..first + members] {
                c.cbuffer = Some(resource.name_hash);
            }
            let members = &out.constants[first..first + members];
            fits_rooms(members, resource.buffer_size, at + 4 + r)?;
        }
        out.resources.push(resource);
    }
    Ok((at + 4 + size, out))
}

/// Each NUL-terminated run of printable ASCII in `pool`, at most its last 64 characters.
fn pool_names(pool: &[u8]) -> Vec<String> {
    let mut pieces: Vec<&[u8]> = pool.split(|x| *x == 0).collect();
    // The bytes after the last NUL end no name.
    pieces.pop();
    (pieces.into_iter())
        .filter_map(|piece| {
            let start =
                (piece.iter().rposition(|c| !(0x20..=0x7E).contains(c))).map_or(0, |i| i + 1);
            let name = &piece[start.max(piece.len().saturating_sub(64))..];
            (!name.is_empty()).then(|| String::from_utf8_lossy(name).into_owned())
        })
        .collect()
}

/// A binding layout of `bytes`, which lay at `at`.
fn layout(bytes: &[u8], at: usize) -> Read<Sgd5Layout> {
    let size = bytes.len();
    if size < LAYOUT_HEAD || !(size - LAYOUT_HEAD).is_multiple_of(8) {
        return Err(invalid(at, format!("a binding layout of {size} bytes")));
    }
    let records = (size - LAYOUT_HEAD) / 8;
    let word = |o: usize| u16::from_le_bytes([bytes[o], bytes[o + 1]]);
    let dword = |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    let mut classes = [0u8; 6];
    classes.copy_from_slice(&bytes[10..16]);
    let class_sum: usize = classes.iter().map(|c| usize::from(*c)).sum();
    if usize::from(word(8)) != records || class_sum != records || dword(20) != 0 {
        return Err(invalid(
            at,
            format!(
                "a layout of {records} records states {} and classes {classes:?}",
                word(8)
            ),
        ));
    }
    let bindings: Vec<Sgd5Binding> = (0..records)
        .map(|k| {
            let r = LAYOUT_HEAD + 8 * k;
            Sgd5Binding {
                hash: dword(r),
                kind: bytes[r + 4],
                flags: bytes[r + 5],
                index: word(r + 6),
            }
        })
        .collect();
    let mut stated = (classes.iter().enumerate())
        .flat_map(|(class, n)| std::iter::repeat_n(class, usize::from(*n)));
    if let Some(r) = bindings.iter().find(|r| class_of(r.kind) != stated.next()) {
        return Err(invalid(
            at,
            format!(
                "a layout's record of kind {} sits in another class than its kind's",
                r.kind
            ),
        ));
    }
    Ok(Sgd5Layout {
        hash: u64::from(dword(0)) | u64::from(dword(4)) << 32,
        classes,
        id: dword(16),
        bindings,
    })
}

/// The class a binding layout's record of `kind` falls in; classes 0 and 2 hold no record.
pub fn class_of(kind: u8) -> Option<usize> {
    match kind {
        1 | 2 | 5 | 7 | 14 => Some(1),
        4 | 6 | 8 | 12 => Some(3),
        3 => Some(4),
        0 => Some(5),
        _ => None,
    }
}

enum Item {
    Count(u32),
    Program(Sgd5Program),
    Gap,
}

/// Where the effects start: after the effect count at this offset, or at this offset after a
/// piece not delivered that held the count.
enum EffectsAt {
    Count(usize),
    After(usize),
}

/// The bank counts and programs from `at` to the effect count, stepping past pieces not
/// delivered; the gaps are `Item::Gap`, in order with `gaps`.
fn walk(b: &[u8], mut at: usize) -> Read<(Vec<Item>, Vec<Sgd5Gap>, EffectsAt)> {
    let (mut items, mut gaps) = (Vec::new(), Vec::new());
    loop {
        let step = if piece_starts(b, at) {
            Err(Sgd5Error::NotOnDisk { at })
        } else if is_program(b, at) {
            program(b, at).map(|(next, p)| (next, Item::Program(p)))
        } else {
            u32_at(b, at).and_then(|c| match c {
                c if c > MAX_COUNT => Err(invalid(
                    at,
                    format!("a count of {c} is neither a bank's nor the effects'"),
                )),
                c => Ok((at + 4, Item::Count(c))),
            })
        };
        match step {
            Ok((next, item)) => {
                let ends = matches!(item, Item::Count(c) if c != 0 && !is_program(b, next));
                items.push(item);
                if ends {
                    return Ok((items, gaps, EffectsAt::Count(at)));
                }
                at = next;
            }
            Err(Sgd5Error::NotOnDisk { at: gap }) => {
                let mut past = gap;
                while zero_piece(b, past) {
                    past += PIECE_PAGE;
                }
                items.push(Item::Gap);
                if let Some(next) = next_program(b, past) {
                    gaps.push(Sgd5Gap {
                        from: at,
                        resume: Some(next),
                    });
                    at = next;
                    continue;
                }
                let next = next_effect(b, past).ok_or_else(|| {
                    invalid(past, "nothing readable after the piece not delivered")
                })?;
                gaps.push(Sgd5Gap {
                    from: at,
                    resume: Some(next),
                });
                return Ok((items, gaps, EffectsAt::After(next)));
            }
            Err(e) => return Err(e),
        }
    }
}

/// The first program at or after `from`: its frame word, then back over its sizes, kind and name.
fn next_program(b: &[u8], from: usize) -> Option<usize> {
    let pattern = FRAME.to_le_bytes();
    let mut search = from.checked_add(8)?;
    while let Some(k) = b.get(search..)?.windows(4).position(|w| w == pattern) {
        let frame = search + k;
        let start = (2..256).find_map(|len| {
            let start = frame.checked_sub(6 + len).filter(|s| *s >= from)?;
            (usize::from(b[start]) == len && is_program(b, start)).then_some(start)
        });
        if start.is_some() {
            return start;
        }
        search = frame + 1;
    }
    None
}

/// The first effect at or after `from` that reads, and after which the trailer or another effect
/// reads.
fn next_effect(b: &[u8], from: usize) -> Option<usize> {
    let name_char = |c: &u8| c.is_ascii_alphanumeric() || *c == b'_';
    (from..b.len()).find(|&at| {
        let len = usize::from(b[at]);
        let candidate = (3..=0x41).contains(&len)
            && b.get(at + 1..at + len)
                .is_some_and(|n| n.iter().all(name_char))
            && b.get(at + len) == Some(&0);
        candidate
            && effect(b, at)
                .is_ok_and(|(next, _)| b.get(next) == Some(&TRAILER) || effect(b, next).is_ok())
    })
}

type Banked = ([Option<u32>; BANKS], Vec<Sgd5Program>);

/// Each program's bank and index. With no piece missing, or with all six bank counts found, the
/// counts are the banks in order; else the counts before the first gap count up from bank 0 and
/// those after the last back from bank 5. A program after a gap counts back from its bank's count.
fn banks(items: Vec<Item>, effects_at: &EffectsAt) -> Read<Banked> {
    let mut openers: Vec<usize> = (items.iter().enumerate())
        .filter(|(_, it)| matches!(it, Item::Count(_)))
        .map(|(k, _)| k)
        .collect();
    let end = match effects_at {
        EffectsAt::Count(_) => openers.pop().unwrap_or(items.len()),
        EffectsAt::After(_) => items.len(),
    };
    let gap_at: Vec<usize> = (items.iter().enumerate())
        .filter(|(_, it)| matches!(it, Item::Gap))
        .map(|(k, _)| k)
        .collect();
    if openers.len() > BANKS || (gap_at.is_empty() && openers.len() != BANKS) {
        return Err(invalid(
            0,
            format!(
                "{} bank counts, where a container has {BANKS}",
                openers.len()
            ),
        ));
    }
    let mut bank_of = vec![None; items.len()];
    match (gap_at.first(), gap_at.last()) {
        (Some(first), Some(last)) if openers.len() < BANKS => {
            for (bank, k) in openers.iter().filter(|k| *k < first).enumerate() {
                bank_of[*k] = Some(bank);
            }
            for (bank, k) in (0..BANKS)
                .rev()
                .zip(openers.iter().filter(|k| *k > last).rev())
            {
                bank_of[*k] = Some(bank);
            }
        }
        _ => {
            for (bank, k) in openers.iter().enumerate() {
                bank_of[*k] = Some(bank);
            }
        }
    }
    let mut counts = [None; BANKS];
    let mut assigned: Vec<(Option<usize>, Option<u32>)> = vec![(None, None); items.len()];
    let bounds: Vec<usize> = openers.iter().copied().chain([end]).collect();
    for pair in bounds.windows(2) {
        let (opener, next) = (pair[0], pair[1]);
        let (Some(bank), Item::Count(count)) = (bank_of[opener], &items[opener]) else {
            continue;
        };
        counts[bank] = Some(*count);
        let segment = opener + 1..next;
        let first_gap = segment.clone().find(|k| matches!(items[*k], Item::Gap));
        let last_gap = segment
            .clone()
            .rev()
            .find(|k| matches!(items[*k], Item::Gap));
        let before: Vec<usize> = (segment.start..first_gap.unwrap_or(next)).collect();
        let after: Vec<usize> = last_gap.map_or(Vec::new(), |g| (g + 1..next).collect());
        if before.len() + after.len() > *count as usize
            || (gap_at.is_empty() && before.len() != *count as usize)
        {
            return Err(invalid(
                0,
                format!(
                    "bank {bank} states {count} programs and holds {}",
                    before.len() + after.len()
                ),
            ));
        }
        for k in segment {
            assigned[k].0 = Some(bank);
        }
        for (i, k) in before.iter().enumerate() {
            assigned[*k].1 = Some(i as u32);
        }
        let from = *count as usize - after.len();
        for (i, k) in after.iter().enumerate() {
            assigned[*k].1 = Some((from + i) as u32);
        }
    }
    let programs = (items.into_iter().zip(assigned))
        .filter_map(|(item, (bank, index))| match item {
            Item::Program(p) => Some(Sgd5Program { bank, index, ..p }),
            _ => None,
        })
        .collect();
    Ok((counts, programs))
}

/// The effect at `at` and where the next one starts.
fn effect(b: &[u8], at: usize) -> Read<(usize, Sgd5Effect)> {
    if zero_piece(b, at) {
        return Err(Sgd5Error::NotOnDisk { at });
    }
    let (name, mut q) = name_at(b, at)?.ok_or_else(|| invalid(at, "no effect name"))?;
    let word = u32_at(b, q)?;
    q += 4;
    let mut programs: [Vec<u32>; BANKS] = Default::default();
    for list in &mut programs {
        let n = counted(b, q, MAX_LIST)?;
        let bytes = take(b, q + 4, 4 * n)?;
        *list = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|w| u32::from_le_bytes(*w))
            .collect();
        q += 4 + 4 * n;
    }
    q += EFFECT_STATE;
    let mut state_counts = [0; 4];
    let mut technique_records: &[u8] = &[];
    for (k, stride) in STATE_STRIDES.into_iter().enumerate() {
        let n = counted(b, q, MAX_LIST)?;
        let bytes = take(b, q + 4, n * stride)?;
        match state_counts.get_mut(k) {
            Some(count) => *count = n,
            None => technique_records = bytes,
        }
        q += 4 + n * stride;
    }
    let pass_count = counted(b, q, MAX_LIST)?;
    let pass_records = take(b, q + 4, PASS * pass_count)?;
    q += 4 + PASS * pass_count;
    // The bound-name section: its size in words, then the words.
    let words = counted(b, q, MAX_BOUND_WORDS as u32)?;
    let section: Vec<u32> = (take(b, q + 4, 4 * words)?.as_chunks::<4>().0.iter())
        .map(|w| u32::from_le_bytes(*w))
        .collect();
    let bound = bound_lists(&section, pass_count, false)
        .or_else(|| bound_lists(&section, pass_count, true))
        .ok_or_else(|| {
            invalid(
                q,
                format!(
                    "effect `{name}`: its {words}-word bound-name section closes as neither form"
                ),
            )
        })?;
    q += 4 + 4 * words;
    let flag = u32_at(b, q)?;
    if flag & !LAYOUT_REFERENCE != 0 {
        return Err(invalid(
            q,
            format!("effect `{name}` states {flag:#010X} before its tables"),
        ));
    }
    let (next, parameters) = table(b, q + 4)?;
    // A default of type 0x03 is the identity remap; one of only zeros is read as none set.
    let off_identity = |w: &[u32]| !is_identity_remap(w) && w.iter().any(|x| *x != 0);
    let stray = (parameters.constants.iter())
        .find(|c| c.code == 0x03 && c.default.as_deref().is_some_and(off_identity));
    if let Some(c) = stray {
        return Err(invalid(
            q + 4,
            format!(
                "effect `{name}`: constant {:08X} of type 0x03 defaults to no identity remap",
                c.name_hash
            ),
        ));
    }
    let (next, second) = table(b, next)?;
    q = next;
    let stated = u32_at(b, q)?;
    let layout_flag = (stated >> 24) as u8;
    let layout_count = stated & 0x00FF_FFFF;
    if layout_count > MAX_LIST {
        return Err(invalid(
            q,
            format!("effect `{name}` states {layout_count} layouts"),
        ));
    }
    q += 4;
    let (mut layout_refs, mut layouts) = (Vec::new(), Vec::new());
    for _ in 0..layout_count {
        let head = u32_at(b, q)?;
        if head & LAYOUT_REFERENCE != 0 {
            layout_refs.push(u32_at(b, q + 4)?);
            q += 8;
        } else {
            layouts.push(layout(take(b, q + 4, head as usize)?, q)?);
            q += 4 + head as usize;
        }
    }
    let pool_len = counted(b, q, MAX_NAMES as u32)?;
    let pool = take(b, q + 4, pool_len)?;
    q += 4 + pool_len;
    let named = |offset: u16, what: &str| {
        pool_name(pool, offset).ok_or_else(|| {
            invalid(
                at,
                format!("effect `{name}`: a {what} name is outside its pool"),
            )
        })
    };
    let techniques = (technique_records.as_chunks::<4>().0.iter())
        .map(|r| {
            let offset = u16::from_le_bytes([r[0], r[1]]);
            Ok(Sgd5Technique {
                name: named(offset, "technique")?,
                flag: offset & 0x8000 != 0,
                index: r[2],
                passes: r[3],
            })
        })
        .collect::<Read<_>>()?;
    let passes = (pass_records.as_chunks::<PASS>().0.iter())
        .map(|r| {
            let mut programs = [0; BANKS];
            programs.copy_from_slice(&r[..BANKS]);
            Ok(Sgd5Pass {
                name: named(u16::from_le_bytes([r[14], r[15]]), "pass")?,
                programs,
            })
        })
        .collect::<Read<_>>()?;
    let effect = Sgd5Effect {
        name,
        at,
        word,
        programs,
        state_counts,
        techniques,
        passes,
        bound,
        parameters,
        second,
        layout_flag,
        layout_refs,
        layouts,
    };
    Ok((q, effect))
}

/// `words` as `passes` passes of four counted lists, each pass then closed by a word whose bits 8
/// and up are 1 where `trailing`; `None` where they do not end exactly at the last word.
fn bound_lists(words: &[u32], passes: usize, trailing: bool) -> Option<Vec<Sgd5BoundPass>> {
    let mut q = 0usize;
    let mut out = Vec::with_capacity(passes.min(words.len()));
    for _ in 0..passes {
        let mut lists: [Vec<u32>; 4] = Default::default();
        for list in &mut lists {
            let end = (q + 1).checked_add(*words.get(q)? as usize)?;
            *list = words.get(q + 1..end)?.to_vec();
            q = end;
        }
        let trailing = match trailing {
            true => {
                let word = *words.get(q).filter(|w| **w >> 8 == 1)?;
                q += 1;
                Some(word)
            }
            false => None,
        };
        out.push(Sgd5BoundPass { lists, trailing });
    }
    (q == words.len()).then_some(out)
}

/// The u32 count at `at`, refused past `max`.
fn counted(b: &[u8], at: usize, max: u32) -> Read<usize> {
    match u32_at(b, at)? {
        n if n > max => Err(invalid(
            at,
            format!("a count of {n}, past the {max} a container holds"),
        )),
        n => Ok(n as usize),
    }
}

/// The name at `offset` in an effect's name pool; bit 15 of the offset is a flag.
fn pool_name(pool: &[u8], offset: u16) -> Option<String> {
    let tail = pool.get(usize::from(offset & 0x7FFF)..)?;
    let end = tail.iter().position(|c| *c == 0)?;
    (end > 0).then(|| String::from_utf8_lossy(&tail[..end]).into_owned())
}

/// Every program index in range of its bank's count, and every layout reference to a head layout.
fn check_effect(e: &Sgd5Effect, counts: &[Option<u32>; BANKS], heads: &[u32]) -> Read<()> {
    for (bank, (list, count)) in e.programs.iter().zip(counts).enumerate() {
        let Some(count) = count else { continue };
        if let Some(index) = list.iter().find(|i| *i >= count) {
            return Err(invalid(
                e.at,
                format!(
                    "effect `{}` names program {index} of bank {bank}, which states {count}",
                    e.name
                ),
            ));
        }
    }
    match e.layout_refs.iter().find(|id| !heads.contains(id)) {
        Some(id) => Err(invalid(
            e.at,
            format!(
                "effect `{}` references {id:08X}, which no head layout is",
                e.name
            ),
        )),
        None => Ok(()),
    }
}

#[cfg(test)]
#[path = "sgd5_tests.rs"]
mod tests;
