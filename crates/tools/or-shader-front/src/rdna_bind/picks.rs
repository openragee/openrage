//! Which textures of its material a GTA VI program picks from the heap by number, and which
//! targets its pixel stage writes: both read from the program's own code.

use std::collections::{BTreeMap, BTreeSet};

use or_io::gta5::shaders::sgd5::{Sgd5Resource, Sgd5Table};

use super::{numbered, Class, Place, Record};
use crate::dxil::ShaderKind;
use crate::rdna::{decode_one, Inst, Operand};

/// The dwords of a heap entry's place: a number is shifted by this to reach its entry.
const ENTRY_SHIFT: u32 = 5;
/// The effect container's resource codes of a material's texture and of its texture array.
const TEXTURE: u8 = 0x02;
pub(super) const TEXTURE_ARRAY: u8 = 0x42;
/// The first register an effect gives a material's textures; a number is the register's low byte.
const MATERIAL_REGISTERS: u16 = 256;
/// The dimension field of an image instruction that reads a two-dimensional array.
const DIM_2D_ARRAY: u8 = 5;
/// The element size a binding record states for a buffer of bytes.
const RAW: u32 = 1;
/// The dwords of a buffer's descriptor.
const BUFFER_DWORDS: u32 = 4;

/// Where a program's numbers start in their buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Base {
    /// At the byte a user register holds.
    User(u8),
    /// At the byte a dword of another buffer holds: that buffer, as [`Pick::buffer`] names
    /// one, and the dword's byte offset there. The numbers are then the head of a record of the
    /// material that the game keeps among others in one buffer.
    Dword((u8, u32), u32),
}

/// One heap entry a program loads by a number it reads from a buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pick {
    /// The buffer of numbers: the register that points at its table and its descriptor's byte
    /// offset there, as the code numbers its registers.
    pub buffer: (u8, u32),
    /// What is added to the number's place.
    pub base: Base,
    /// The number's place in bytes, past `base`.
    pub at: u32,
    /// The user register that points at the heap.
    pub heap: u8,
}

/// What a program's code picks and exports.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Picks {
    /// Each entry picked by a number, with the dimension field of every image instruction that reads it.
    pub by_number: BTreeMap<Pick, BTreeSet<u8>>,
    /// The loads of a table entry at a place computed any other way.
    pub other: usize,
    /// The target and the component mask of each export, in the code's order.
    pub exports: Vec<(u8, u8)>,
}

/// What a scalar register holds, as far as a pick needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Held {
    Unknown,
    Const(u32),
    /// A user register's value plus a constant.
    User(u8, u32),
    /// The first word of the descriptor at a byte offset of the table a user register points at.
    Descriptor(u8, u32),
    /// A dword of a buffer at a stated byte offset, plus a constant.
    Dword((u8, u32), u32, u32),
    /// A dword of a buffer, read at its base plus a constant.
    Number((u8, u32), Base, u32),
    /// A number times the size of a heap entry.
    Entry((u8, u32), Base, u32),
    /// The first word of the entry a number names.
    Picked(Pick),
}

fn register(operand: Operand) -> Option<usize> {
    match operand {
        Operand::Sgpr(n) => Some(usize::from(n)),
        _ => None,
    }
}

fn held(state: &[Held], operand: Operand) -> Held {
    match operand {
        Operand::Sgpr(n) => state.get(usize::from(n)).copied().unwrap_or(Held::Unknown),
        Operand::Int(value) => Held::Const(value as u32),
        Operand::Literal(word) => Held::Const(word),
        _ => Held::Unknown,
    }
}

fn set(state: &mut [Held], at: usize, value: Held) {
    if let Some(slot) = state.get_mut(at) {
        *slot = value;
    }
}

/// The dwords a scalar load reads: `s_load_dwordx8` reads eight.
fn load_words(name: &str) -> Option<usize> {
    let rest = name
        .strip_prefix("s_buffer_load_dword")
        .or_else(|| name.strip_prefix("s_load_dword"))?;
    match rest.strip_prefix('x') {
        Some(count) => count.parse().ok(),
        None => rest.is_empty().then_some(1),
    }
}

/// What `words` picks from a heap by number and exports.
// ponytail: one pass over the code in its order, no joins at branches, as the census this encodes
// was measured; a number whose place is computed under a branch is counted under `other`.
pub fn picks(words: &[u32]) -> Picks {
    let mut out = Picks::default();
    let mut state: Vec<Held> = (0..128u8).map(|r| Held::User(r, 0)).collect();
    let mut pc = 0;
    // Words the code reads as data may follow its last instruction: the pass ends where one does not decode.
    while let Ok((inst, len)) = decode_one(words, pc) {
        pc += len.max(1);
        let inst = &inst;
        let name = inst.name();
        let wide = name.ends_with("_b64");
        match inst {
            Inst::Sop1 { dst, src, .. } => {
                let Some(to) = register(*dst) else { continue };
                let moved = name.starts_with("s_mov_b");
                let value = if moved {
                    held(&state, *src)
                } else {
                    Held::Unknown
                };
                set(&mut state, to, value);
                if wide {
                    let high = match (moved, register(*src)) {
                        (true, Some(from)) => state.get(from + 1).copied(),
                        _ => None,
                    };
                    set(&mut state, to + 1, high.unwrap_or(Held::Unknown));
                }
            }
            Inst::Sop2 {
                dst, src0, src1, ..
            } => {
                let Some(to) = register(*dst) else { continue };
                let (a, b) = (held(&state, *src0), held(&state, *src1));
                let value = match (name, a, b) {
                    ("s_add_i32" | "s_add_u32", Held::User(r, had), Held::Const(by))
                    | ("s_add_i32" | "s_add_u32", Held::Const(by), Held::User(r, had)) => {
                        Held::User(r, had.wrapping_add(by))
                    }
                    ("s_add_i32" | "s_add_u32", Held::Dword(of, at, had), Held::Const(by))
                    | ("s_add_i32" | "s_add_u32", Held::Const(by), Held::Dword(of, at, had)) => {
                        Held::Dword(of, at, had.wrapping_add(by))
                    }
                    ("s_add_i32" | "s_add_u32", Held::Const(a), Held::Const(b)) => {
                        Held::Const(a.wrapping_add(b))
                    }
                    ("s_lshl_b32", Held::Number(buffer, base, at), Held::Const(ENTRY_SHIFT)) => {
                        Held::Entry(buffer, base, at)
                    }
                    _ => Held::Unknown,
                };
                set(&mut state, to, value);
                if wide {
                    set(&mut state, to + 1, Held::Unknown);
                }
            }
            Inst::Sopk { dst, imm, .. } => {
                if name.starts_with("s_cmpk_") || name.starts_with("s_setreg") {
                    continue;
                }
                let Some(to) = register(*dst) else { continue };
                let value = match name {
                    "s_movk_i32" => Held::Const(i32::from(*imm) as u32),
                    _ => Held::Unknown,
                };
                set(&mut state, to, value);
            }
            Inst::Smem {
                base,
                data,
                offset,
                soffset,
                ..
            } => {
                let (Some(to), Some(count)) = (register(*data), load_words(name)) else {
                    continue;
                };
                let from = held(&state, Operand::Sgpr(*base));
                let index = match soffset {
                    Operand::Null => None,
                    other => Some(held(&state, *other)),
                };
                let mut first = Held::Unknown;
                if name.starts_with("s_buffer_load") {
                    first = match (from, index) {
                        (Held::Descriptor(root, at), Some(Held::User(by, plus))) => {
                            Held::Number((root, at), Base::User(by), plus.wrapping_add(*offset))
                        }
                        (Held::Descriptor(root, at), Some(Held::Dword(of, start, plus))) => {
                            let at_byte = plus.wrapping_add(*offset);
                            Held::Number((root, at), Base::Dword(of, start), at_byte)
                        }
                        (Held::Descriptor(root, at), None) => Held::Dword((root, at), *offset, 0),
                        _ => Held::Unknown,
                    };
                } else {
                    match (from, index) {
                        (Held::User(root, 0), None) => first = Held::Descriptor(root, *offset),
                        (Held::User(heap, 0), Some(Held::Entry(buffer, base, at)))
                            if *offset == 0 =>
                        {
                            let pick = Pick {
                                buffer,
                                base,
                                at,
                                heap,
                            };
                            out.by_number.entry(pick).or_default();
                            first = Held::Picked(pick);
                        }
                        (_, Some(_)) => out.other += 1,
                        _ => {}
                    }
                }
                for word in 0..count {
                    let value = match (first, word) {
                        (_, 0) => first,
                        // Two buffers' descriptors loaded as one: the second starts four dwords on.
                        (Held::Descriptor(root, at), 4) => {
                            Held::Descriptor(root, at + 4 * BUFFER_DWORDS)
                        }
                        (Held::Number(buffer, by, at), word) => {
                            Held::Number(buffer, by, at.wrapping_add(4 * (word as u32)))
                        }
                        (Held::Dword(buffer, at, 0), word) => {
                            Held::Dword(buffer, at.wrapping_add(4 * (word as u32)), 0)
                        }
                        _ => Held::Unknown,
                    };
                    set(&mut state, to + word, value);
                }
            }
            Inst::Vop1 { dst, .. } if name == "v_readfirstlane_b32" => {
                set(&mut state, usize::from(*dst), Held::Unknown);
            }
            Inst::Mimg { rsrc, dim, .. } => {
                if let Some(Held::Picked(pick)) = state.get(usize::from(*rsrc)) {
                    out.by_number.entry(*pick).or_default().insert(*dim);
                }
            }
            Inst::Exp { target, en, .. } => out.exports.push((*target, *en)),
            _ => {}
        }
    }
    out
}

/// Why a program's picks are not tied to its material's textures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Unsettled {
    #[error("its package lists no one record of the dwords passed with a draw")]
    NoDrawWords,
    #[error("it picks an entry by a number at +{at:#X} of a buffer that is not a buffer of bytes placed by the second dword of a draw")]
    Place { at: u32 },
    #[error("its numbers come from two buffers")]
    TwoBuffers,
    #[error("the number at +{at:#X} is at no dword")]
    Unaligned { at: u32 },
    #[error("number {number} is the register of {found} textures of the effect")]
    Texture { number: u32, found: usize },
    #[error("number {number} is read with dimension {dim}, which its texture's kind is not")]
    Kind { number: u32, dim: u8 },
}

/// The textures of a material a program picks by number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterialTextures<'t> {
    /// The buffer of numbers, as a place the binding records number.
    pub buffer: Option<Place>,
    /// Each number picked, with the effect's texture it names.
    pub textures: Vec<(u32, &'t Sgd5Resource)>,
    /// The buffer that holds the material's record, where the program reads numbers at the
    /// head of one, and the number each dword of the head holds.
    pub record: Option<Place>,
    pub listed: Vec<(u32, u32)>,
}

/// Whether `resource` is a texture a material sets, and whether it is an array.
fn material_texture(resource: &Sgd5Resource) -> Option<bool> {
    let array = match resource.code {
        TEXTURE => false,
        TEXTURE_ARRAY => true,
        _ => return None,
    };
    (resource.slot >= MATERIAL_REGISTERS).then_some(array)
}

/// Each number `picks` holds joined to the texture of `effect` it names: the number at dword `i`
/// of the buffer of bytes the second dword of a draw places is the texture whose register's low
/// byte is `i`, and an array is read as an array. `records` are the program's binding records.
pub fn material_textures<'t>(
    picks: &Picks,
    records: &[Record],
    kind: ShaderKind,
    effect: &'t Sgd5Table,
) -> Result<MaterialTextures<'t>, Unsettled> {
    let mut out = MaterialTextures {
        buffer: None,
        textures: Vec::new(),
        record: None,
        listed: Vec::new(),
    };
    // The textures a record lists: those of the effect that are no array, in their registers' order.
    let mut flat: Vec<&Sgd5Resource> = (effect.resources.iter())
        .filter(|r| material_texture(r) == Some(false))
        .collect();
    flat.sort_by_key(|r| r.slot);
    if picks.by_number.is_empty() {
        return Ok(out);
    }
    let mut words = records.iter().filter(|r| r.class == Class::RootConstants);
    let (Some(draw), None) = (words.next(), words.next()) else {
        return Err(Unsettled::NoDrawWords);
    };
    for (pick, dims) in &picks.by_number {
        let refused = Unsettled::Place { at: pick.at };
        let table = Place::Table {
            root: pick.buffer.0,
            offset: pick.buffer.1,
        };
        let buffer = numbered(table, kind);
        let Place::Table { root, offset } = buffer else {
            return Err(refused);
        };
        // The dwords of a draw are numbered as the tables are: the same shift takes both.
        let shifted = |register: u8| match numbered(
            Place::Table {
                root: register,
                offset: 0,
            },
            kind,
        ) {
            Place::Table { root, .. } => root,
            _ => register,
        };
        let bytes = (records.iter()).any(|r| {
            r.class == Class::Srv && r.stride == RAW && (r.root, r.offset) == (root, offset)
        });
        if !bytes || shifted(pick.heap) != draw.root.wrapping_add(2) {
            return Err(refused);
        }
        if pick.at % 4 != 0 {
            return Err(Unsettled::Unaligned { at: pick.at });
        }
        let number = pick.at / 4;
        let by = match pick.base {
            Base::User(by) => by,
            // The head of the material's record: dword `k` is the number of the effect's
            // `k`-th texture that is no array. A dword past those, or read as an array, is one
            // the rule does not cover: it stays at nought, which picks the blank entry.
            Base::Dword(..) => {
                if out.record.is_some_and(|had| had != buffer) {
                    return Err(Unsettled::TwoBuffers);
                }
                out.record = Some(buffer);
                let covered = (flat.get(number as usize)).filter(|_| !dims.contains(&DIM_2D_ARRAY));
                let Some(texture) = covered else { continue };
                let named = u32::from(texture.slot & 0xFF);
                out.listed.push((number, named));
                if out.textures.iter().all(|(had, _)| *had != named) {
                    out.textures.push((named, texture));
                }
                continue;
            }
        };
        if shifted(by) != draw.root.wrapping_add(1) {
            return Err(refused);
        }
        if out.buffer.is_some_and(|had| had != buffer) {
            return Err(Unsettled::TwoBuffers);
        }
        out.buffer = Some(buffer);
        let mut named = (effect.resources.iter())
            .filter(|r| material_texture(r).is_some() && u32::from(r.slot & 0xFF) == number);
        let (Some(texture), None) = (named.next(), named.next()) else {
            let found = (effect.resources.iter())
                .filter(|r| material_texture(r).is_some() && u32::from(r.slot & 0xFF) == number)
                .count();
            return Err(Unsettled::Texture { number, found });
        };
        let array = material_texture(texture) == Some(true);
        if let Some(&dim) = dims.iter().find(|d| (**d == DIM_2D_ARRAY) != array) {
            return Err(Unsettled::Kind { number, dim });
        }
        if out.textures.iter().all(|(had, _)| *had != number) {
            out.textures.push((number, texture));
        }
    }
    Ok(out)
}

/// The colour targets a deferred pixel program of a model writes whole, and the one it writes two
/// components of.
const WHOLE_TARGETS: [u8; 4] = [0, 1, 2, 3];
const MOTION_TARGET: u8 = 4;
const EXTRA_TARGET: u8 = 5;
/// The first export target that is no colour target.
const COLOUR_TARGETS: u8 = 8;
const WHOLE: u8 = 0xF;
const TWO: u8 = 0x3;

/// Whether `exports` are the targets whose contents are established: 0 the diffuse colour, 1 the
/// gloss and the packed normal, 2 the specular colour, 3 the ambient terms and the draw's bytes,
/// each written whole, and 4 the motion in two components or a colour written whole, which the
/// resolve does not read; a sixth target may follow.
pub fn deferred_targets(exports: &[(u8, u8)]) -> bool {
    let colour = || {
        exports
            .iter()
            .filter(|(target, _)| *target < COLOUR_TARGETS)
    };
    let mask = |target: u8| {
        let mut masks = colour().filter(|(t, _)| *t == target).map(|(_, en)| *en);
        let first = masks.next()?;
        masks.all(|en| en == first).then_some(first)
    };
    WHOLE_TARGETS.iter().all(|t| mask(*t) == Some(WHOLE))
        && matches!(mask(MOTION_TARGET), Some(TWO | WHOLE))
        && colour().all(|(t, _)| *t <= EXTRA_TARGET)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first word of an instruction named `name`, found by trying each number of its field.
    fn numbered_word(name: &str, make: impl Fn(u32) -> Vec<u32>) -> Vec<u32> {
        (0..256)
            .map(make)
            .find(|words| decode_one(words, 0).is_ok_and(|(inst, _)| inst.name() == name))
            .unwrap_or_else(|| panic!("no instruction is named {name}"))
    }

    fn sop2(name: &str, dst: u32, src0: u32, src1: u32) -> Vec<u32> {
        numbered_word(name, |op| {
            vec![0x8000_0000 | (op << 23) | (dst << 16) | (src1 << 8) | src0]
        })
    }

    /// A scalar load of `data` from the buffer or table at `base`, at `offset` plus register `index`.
    fn smem(name: &str, data: u32, base: u32, offset: u32, index: Option<u32>) -> Vec<u32> {
        numbered_word(name, |op| {
            vec![
                (0x3D << 26) | (op << 18) | (data << 6) | (base / 2),
                offset | (index.unwrap_or(125) << 25),
            ]
        })
    }

    fn sample(rsrc: u32, dim: u32) -> Vec<u32> {
        numbered_word("image_sample", |op| {
            vec![
                (0x3C << 26) | ((op & 0x7F) << 18) | (0xF << 8) | (dim << 3) | (op >> 7),
                (rsrc / 4) << 16,
            ]
        })
    }

    fn export(target: u32, en: u32) -> Vec<u32> {
        vec![(0x3E << 26) | (target << 4) | en, 0]
    }

    /// An integer constant operand: 128 is nought.
    const fn int(value: u32) -> u32 {
        128 + value
    }

    /// A pixel program that reads the numbers at dwords 0 and 3 of the buffer at +0x40 of the
    /// table in register 8, placed by register 13, and samples the entries of the heap in 14.
    fn program() -> Vec<u32> {
        [
            smem("s_load_dwordx4", 72, 8, 0x40, None),
            sop2("s_add_i32", 20, 13, int(12)),
            smem("s_buffer_load_dword", 21, 72, 0, Some(13)),
            smem("s_buffer_load_dword", 22, 72, 0, Some(20)),
            sop2("s_lshl_b32", 21, 21, int(5)),
            sop2("s_lshl_b32", 22, 22, int(5)),
            smem("s_load_dwordx8", 32, 14, 0, Some(21)),
            smem("s_load_dwordx8", 40, 14, 0, Some(22)),
            sample(32, 1),
            sample(40, 5),
            export(0, 15),
            export(1, 15),
            export(2, 15),
            export(3, 15),
            export(4, 3),
        ]
        .concat()
    }

    fn texture(code: u8, slot: u16, hash: u32) -> Sgd5Resource {
        Sgd5Resource {
            code,
            flags: 0,
            slot,
            members: 0,
            buffer_size: 0,
            name_hash: hash,
            semantic_hash: hash,
        }
    }

    fn records() -> Vec<Record> {
        let record = |class, root, offset, stride| Record {
            class,
            register: 7,
            root,
            offset,
            dwords: 4,
            stride,
        };
        vec![
            record(Class::Srv, 8, 0x40, RAW),
            record(Class::RootConstants, 12, 0, 0),
        ]
    }

    #[test]
    fn a_number_names_the_texture_whose_register_holds_it_and_an_array_reads_as_one() {
        let picks = picks(&program());
        let pick = |at| Pick {
            buffer: (8, 0x40),
            base: Base::User(13),
            at,
            heap: 14,
        };
        assert_eq!(
            picks.by_number,
            BTreeMap::from([
                (pick(0), BTreeSet::from([1])),
                (pick(12), BTreeSet::from([5])),
            ])
        );
        assert_eq!(picks.other, 0);
        assert!(deferred_targets(&picks.exports));

        let effect = Sgd5Table {
            resources: vec![
                texture(TEXTURE, 256, 0xA000_0001),
                texture(TEXTURE, 257, 0xA000_0002),
                texture(TEXTURE_ARRAY, 515, 0xA000_0003),
                // A texture below the material's registers is none of its own.
                texture(TEXTURE, 3, 0xA000_0004),
            ],
            ..Sgd5Table::default()
        };
        let joined = material_textures(&picks, &records(), ShaderKind::Pixel, &effect).unwrap();
        assert_eq!(
            joined.buffer,
            Some(Place::Table {
                root: 8,
                offset: 0x40
            })
        );
        let names: Vec<(u32, u32)> = (joined.textures.iter())
            .map(|(number, texture)| (*number, texture.name_hash))
            .collect();
        assert_eq!(names, [(0, 0xA000_0001), (3, 0xA000_0003)]);

        // The same numbers against an effect whose third register is no array, then one with none there.
        let flat = Sgd5Table {
            resources: vec![
                texture(TEXTURE, 256, 0xA000_0001),
                texture(TEXTURE, 259, 0xA000_0003),
            ],
            ..Sgd5Table::default()
        };
        assert_eq!(
            material_textures(&picks, &records(), ShaderKind::Pixel, &flat),
            Err(Unsettled::Kind { number: 3, dim: 5 })
        );
        let short = Sgd5Table {
            resources: vec![texture(TEXTURE, 256, 0xA000_0001)],
            ..Sgd5Table::default()
        };
        assert_eq!(
            material_textures(&picks, &records(), ShaderKind::Pixel, &short),
            Err(Unsettled::Texture {
                number: 3,
                found: 0
            })
        );
    }

    #[test]
    fn a_record_s_head_lists_the_textures_that_are_no_array_in_their_registers_order() {
        // The record starts at the byte the first dword of the buffer at +0x60 holds; its
        // dwords 0 and 2 are numbers, read from the buffer at +0x40.
        let code = [
            smem("s_load_dwordx4", 72, 8, 0x40, None),
            smem("s_load_dwordx4", 76, 8, 0x60, None),
            smem("s_buffer_load_dword", 20, 76, 0, None),
            smem("s_buffer_load_dword", 21, 72, 0, Some(20)),
            smem("s_buffer_load_dword", 22, 72, 8, Some(20)),
            sop2("s_lshl_b32", 21, 21, int(5)),
            sop2("s_lshl_b32", 22, 22, int(5)),
            smem("s_load_dwordx8", 32, 14, 0, Some(21)),
            smem("s_load_dwordx8", 40, 14, 0, Some(22)),
            sample(32, 1),
            sample(40, 1),
        ]
        .concat();
        let picks = picks(&code);
        let pick = |at| Pick {
            buffer: (8, 0x40),
            base: Base::Dword((8, 0x60), 0),
            at,
            heap: 14,
        };
        assert_eq!(
            picks.by_number.keys().copied().collect::<Vec<_>>(),
            [pick(0), pick(8)]
        );
        let effect = Sgd5Table {
            resources: vec![
                texture(TEXTURE, 256, 0xA000_0001),
                texture(TEXTURE_ARRAY, 513, 0xA000_0002),
                texture(TEXTURE, 258, 0xA000_0003),
                texture(TEXTURE, 259, 0xA000_0004),
            ],
            ..Sgd5Table::default()
        };
        let joined = material_textures(&picks, &records(), ShaderKind::Pixel, &effect).unwrap();
        let table = Place::Table {
            root: 8,
            offset: 0x40,
        };
        assert_eq!((joined.buffer, joined.record), (None, Some(table)));
        // The array is passed over: the record's third dword is the fourth register's texture.
        assert_eq!(joined.listed, [(0, 0), (2, 3)]);
        let names: Vec<(u32, u32)> = (joined.textures.iter())
            .map(|(number, texture)| (*number, texture.name_hash))
            .collect();
        assert_eq!(names, [(0, 0xA000_0001), (3, 0xA000_0004)]);
        // An effect with fewer such textures than the record lists.
        let short = Sgd5Table {
            resources: effect.resources[..3].to_vec(),
            ..Sgd5Table::default()
        };
        let fewer = material_textures(&picks, &records(), ShaderKind::Pixel, &short).unwrap();
        assert_eq!(
            fewer.listed,
            [(0, 0)],
            "the dword past them stays at nought"
        );
    }

    #[test]
    fn a_number_placed_by_another_register_or_read_from_another_buffer_is_not_settled() {
        let picks = picks(&program());
        let effect = Sgd5Table {
            resources: vec![
                texture(TEXTURE, 256, 0xA000_0001),
                texture(TEXTURE_ARRAY, 515, 0xA000_0003),
            ],
            ..Sgd5Table::default()
        };
        // The draw's dwords start a register later: the numbers are then placed by its first.
        let mut moved = records();
        moved[1].root = 14;
        assert_eq!(
            material_textures(&picks, &moved, ShaderKind::Pixel, &effect),
            Err(Unsettled::Place { at: 0 })
        );
        // The buffer is one of elements, not of bytes.
        let mut structured = records();
        structured[0].stride = 48;
        assert_eq!(
            material_textures(&picks, &structured, ShaderKind::Pixel, &effect),
            Err(Unsettled::Place { at: 0 })
        );
        assert_eq!(
            material_textures(&picks, &records()[..1], ShaderKind::Pixel, &effect),
            Err(Unsettled::NoDrawWords)
        );
        // A program that picks nothing is settled with nothing to fill.
        let none = material_textures(&Picks::default(), &[], ShaderKind::Pixel, &effect).unwrap();
        assert!(none.buffer.is_none() && none.textures.is_empty());
    }

    #[test]
    fn an_entry_loaded_at_a_place_no_number_gives_is_counted_apart() {
        let code = [
            // The place is a register the program computed.
            sop2("s_lshl_b32", 21, 30, int(5)),
            smem("s_load_dwordx8", 32, 14, 0, Some(21)),
            export(0, 15),
        ]
        .concat();
        let picks = picks(&code);
        assert!(picks.by_number.is_empty());
        assert_eq!(picks.other, 1);
        assert_eq!(picks.exports, [(0, 15)]);
    }

    #[test]
    fn the_deferred_targets_are_four_whole_and_a_fifth() {
        let five = [(0, 15), (1, 15), (2, 15), (3, 15), (4, 3)];
        assert!(deferred_targets(&five));
        // A sixth target and a depth export change nothing.
        let mut six = five.to_vec();
        six.extend([(5, 15), (8, 1)]);
        assert!(deferred_targets(&six));
        // A fifth target written whole is the same four before it; a missing one is another layout.
        let blended = [(0, 15), (1, 15), (2, 15), (3, 15), (4, 15), (5, 15)];
        assert!(deferred_targets(&blended));
        assert!(!deferred_targets(&five[..4]));
        assert!(!deferred_targets(&[
            (0, 15),
            (1, 7),
            (2, 15),
            (3, 15),
            (4, 3)
        ]));
        let mut seventh = five.to_vec();
        seventh.push((6, 15));
        assert!(!deferred_targets(&seventh));
    }
}
