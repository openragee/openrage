//! Buffer reads by address: raw dwords and typed elements, of a buffer a table names or one the
//! code describes itself around an address in its user registers.

use naga::{
    BinaryOperator as Bo, Block, Expression as E, Handle, MathFunction as Mf, Scalar, Span,
    Statement, TypeInner,
};

use super::{form, refused, whole, Cx, Done, NotTranslated, Reg, ResourceKind, Value, SLOT_DWORDS};
use crate::rdna::decode::{Inst, Operand};
use crate::rdna::sym::{self, Loc};

/// The dwords before the first element of a buffer that states its own stride.
const HEADER_WORDS: u32 = 4;
/// The header dword that states the element format of a buffer a table names: the bits of a
/// component in its low byte, and above it how they are read, as the number of a [`Channel`].
const HEADER_FORMAT: u32 = 1;
/// A descriptor's component selection that leaves the four components where they are.
const IDENTITY_SELECTION: u32 = 0xFAC;

/// How the bits of an element's component are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Channel {
    Unorm = 0,
    Snorm = 1,
    Uscaled = 2,
    Sscaled = 3,
    Uint = 4,
    Sint = 5,
    Float = 6,
}

/// The six integer readings a format family comes in, in the order of their codes.
const FAMILY: [Channel; 6] = [
    Channel::Unorm,
    Channel::Snorm,
    Channel::Uscaled,
    Channel::Sscaled,
    Channel::Uint,
    Channel::Sint,
];

/// An element format by its gfx10 code: each component's first bit and width, and how its bits are read.
/// The packed 10- and 11-bit float formats are not read.
fn element_format(code: u8) -> Option<(&'static [(u32, u32)], Channel)> {
    const B8: &[(u32, u32)] = &[(0, 8), (8, 8), (16, 8), (24, 8)];
    const B16: &[(u32, u32)] = &[(0, 16), (16, 16), (32, 16), (48, 16)];
    const B32: &[(u32, u32)] = &[(0, 32), (32, 32), (64, 32), (96, 32)];
    // A packed format's name lists its widths from the top bit down.
    const B10_2: &[(u32, u32)] = &[(0, 2), (2, 10), (12, 10), (22, 10)];
    const B2_10: &[(u32, u32)] = &[(0, 10), (10, 10), (20, 10), (30, 2)];
    let family = |first: u8| FAMILY[usize::from(code - first)];
    Some(match code {
        1..=6 => (&B8[..1], family(1)),
        7..=12 => (&B16[..1], family(7)),
        13 => (&B16[..1], Channel::Float),
        14..=19 => (&B8[..2], family(14)),
        20 => (&B32[..1], Channel::Uint),
        21 => (&B32[..1], Channel::Sint),
        22 => (&B32[..1], Channel::Float),
        23..=28 => (&B16[..2], family(23)),
        29 => (&B16[..2], Channel::Float),
        44..=49 => (B10_2, family(44)),
        50..=55 => (B2_10, family(50)),
        56..=61 => (B8, family(56)),
        62 => (&B32[..2], Channel::Uint),
        63 => (&B32[..2], Channel::Sint),
        64 => (&B32[..2], Channel::Float),
        65..=70 => (B16, family(65)),
        71 => (B16, Channel::Float),
        72 => (&B32[..3], Channel::Uint),
        73 => (&B32[..3], Channel::Sint),
        74 => (&B32[..3], Channel::Float),
        75 => (B32, Channel::Uint),
        76 => (B32, Channel::Sint),
        77 => (B32, Channel::Float),
        _ => return None,
    })
}

/// The atomic update a mnemonic's last word names, over unsigned dwords; `compare` is what a
/// compare-and-swap swaps on.
pub(super) fn atomic_function(
    name: &str,
    compare: Option<Handle<E>>,
) -> Option<naga::AtomicFunction> {
    use naga::AtomicFunction as F;
    Some(match name {
        "add" => F::Add,
        "sub" => F::Subtract,
        "and" => F::And,
        "or" => F::InclusiveOr,
        "xor" => F::ExclusiveOr,
        "umin" => F::Min,
        "umax" => F::Max,
        "swap" => F::Exchange { compare: None },
        "cmpswap" => F::Exchange { compare },
        _ => return None,
    })
}

/// How many components a typed buffer instruction names, and whether its registers hold them
/// as 16-bit values, from what follows `format_` in its mnemonic.
fn components(rest: &str) -> Option<(bool, usize)> {
    let (d16, parts) = match rest.strip_prefix("d16_") {
        Some(parts) => (true, parts),
        None => (false, rest),
    };
    Some((
        d16,
        match parts {
            "x" => 1,
            "xy" => 2,
            "xyz" => 3,
            "xyzw" => 4,
            _ => return None,
        },
    ))
}

/// A buffer an instruction reads: its global, its element stride, the dwords before its first
/// element, and the fourth descriptor word where the code wrote it.
struct Bound {
    global: Handle<naga::GlobalVariable>,
    /// The entry of the table's binding array, where a run computed the descriptor's place.
    slot: Option<Handle<E>>,
    stride: Handle<E>,
    first: u32,
    word3: Option<u32>,
}

impl Cx<'_> {
    /// One dword of a buffer, or of one buffer of a table bound whole.
    pub(super) fn buffer_word(
        &mut self,
        global: Handle<naga::GlobalVariable>,
        slot: Option<Handle<E>>,
        index: Handle<E>,
    ) -> Handle<E> {
        let cell = self.buffer_cell(global, slot, index);
        self.b.load(cell)
    }

    /// The place of one dword of a buffer, or of one buffer of a table bound whole.
    fn buffer_cell(
        &mut self,
        global: Handle<naga::GlobalVariable>,
        slot: Option<Handle<E>>,
        index: Handle<E>,
    ) -> Handle<E> {
        let pointer = self.global(global);
        // A table's buffers lie one after another in one buffer, whose dword `slot` is the dword
        // that entry's buffer starts at: an array of storage buffers is nothing Direct3D binds.
        let index = match slot {
            Some(slot) => {
                let start = self.b.emit(E::Access {
                    base: pointer,
                    index: slot,
                });
                let start = self.b.load(start);
                self.b.bin(Bo::Add, start, index)
            }
            None => index,
        };
        self.b.emit(E::Access {
            base: pointer,
            index,
        })
    }

    /// A scalar read of memory the code describes itself: dwords of the buffer at a byte offset.
    pub(super) fn memory_words(
        &mut self,
        pc: u32,
        rsrc: u8,
        offset: u32,
        dynamic: Option<Handle<E>>,
        (words, to): (u8, u8),
    ) -> Done {
        if sym::memory(&self.state, rsrc).is_none() {
            return Err(NotTranslated::Descriptor {
                pc,
                what: "a buffer whose descriptor it did not load from a table",
            });
        }
        let bound = self.buffer(pc, rsrc)?;
        let mut first = self.b.u32(offset / 4 + bound.first);
        if let Some(by) = dynamic {
            let two = self.b.u32(2);
            let moved = self.b.bin(Bo::ShiftRight, by, two);
            first = self.b.bin(Bo::Add, first, moved);
        }
        for k in 0..words {
            let by = self.b.u32(u32::from(k));
            let index = self.b.bin(Bo::Add, first, by);
            let value = self.buffer_word(bound.global, bound.slot, index);
            self.put(Reg::S(to + k), value);
        }
        Ok(())
    }

    /// The buffer the four descriptor registers from `rsrc` name.
    fn buffer(&mut self, pc: u32, rsrc: u8) -> Result<Bound, NotTranslated> {
        let memory = sym::memory(&self.state, rsrc);
        let key = match (sym::descriptor(&self.state, rsrc, 4), memory) {
            (Some(at), _) => (at, None),
            (None, Some(memory)) => (
                Loc {
                    root: memory.root,
                    path: Vec::new(),
                    offset: 0,
                    index: None,
                },
                Some(memory.stride),
            ),
            (None, None) => {
                return Err(NotTranslated::Descriptor {
                    pc,
                    what: "a buffer whose descriptor it did not load from a table",
                })
            }
        };
        // A place a run computed picks one buffer of the whole table.
        let slot = self.entry(pc, rsrc, &key.0, SLOT_DWORDS);
        let key = match slot {
            Some(_) => (whole(&key.0), key.1),
            None => key,
        };
        let global = match self.storage.get(&key) {
            Some(&global) => global,
            None => {
                let base = self.ty(TypeInner::Scalar(Scalar::U32));
                let inner = TypeInner::Array {
                    base,
                    size: naga::ArraySize::Dynamic,
                    stride: 4,
                };
                let space = naga::AddressSpace::Storage {
                    access: naga::StorageAccess::LOAD,
                };
                let kind = ResourceKind::Buffer { stride: key.1 };
                let name = format!("buf{}", self.storage.len());
                let global = self.bound(name, inner, space, kind, Some(key.0.clone()));
                if slot.is_some() {
                    if let Some(resource) = self.resources.last_mut() {
                        resource.count = Some(super::TABLE_SLOTS);
                    }
                }
                self.storage.insert(key.clone(), global);
                global
            }
        };
        let (stride, first) = match key.1 {
            Some(stride) => (self.b.u32(stride), 0),
            None => {
                let zero = self.b.u32(0);
                (self.buffer_word(global, slot, zero), HEADER_WORDS)
            }
        };
        Ok(Bound {
            global,
            slot,
            stride,
            first,
            // A table's descriptor is no memory, whatever its registers look like.
            word3: match key.1 {
                Some(_) => memory.map(|memory| memory.word3),
                None => None,
            },
        })
    }

    /// The number of the dword a buffer instruction reads first: the index times the stride, the
    /// offset register, the scalar offset and the immediate, in bytes, then as a dword past the header.
    // ponytail: an address that is no multiple of four reads the dword it falls in; no program was seen to pass one.
    fn buffer_address(
        &mut self,
        pc: u32,
        bound: &Bound,
        (addr, idxen, offen): (u8, bool, bool),
        soffset: Operand,
        offset: u16,
    ) -> Value {
        let mut bytes = self.b.u32(u32::from(offset));
        let mut register = addr;
        if idxen {
            let index = self.vreg(pc, register)?;
            let scaled = self.b.bin(Bo::Multiply, index, bound.stride);
            bytes = self.b.bin(Bo::Add, bytes, scaled);
            register = register.wrapping_add(1);
        }
        if offen {
            let by = self.vreg(pc, register)?;
            bytes = self.b.bin(Bo::Add, bytes, by);
        }
        if !matches!(soffset, Operand::Null | Operand::Int(0)) {
            let by = self.sv(pc, soffset)?;
            bytes = self.b.bin(Bo::Add, bytes, by);
        }
        let (two, first) = (self.b.u32(2), self.b.u32(bound.first));
        let word = self.b.bin(Bo::ShiftRight, bytes, two);
        Ok(self.b.bin(Bo::Add, word, first))
    }

    /// `width` bits from bit `first` of `word`, sign-extended where `signed`.
    pub(super) fn field(
        &mut self,
        word: Handle<E>,
        first: u32,
        width: u32,
        signed: bool,
    ) -> Handle<E> {
        if width == 32 {
            return word;
        }
        let up = self.b.u32(32 - first - width);
        let raised = self.b.bin(Bo::ShiftLeft, word, up);
        let down = self.b.u32(32 - width);
        self.b.shift(Bo::ShiftRight, signed, raised, down)
    }

    /// Reads `count` components of the element at dword `at` in the format `code` into the registers from `data`.
    fn typed(
        &mut self,
        pc: u32,
        inst: &Inst,
        bound: &Bound,
        at: Handle<E>,
        code: u8,
        (count, data): (usize, u8),
    ) -> Done {
        let Some((fields, channel)) = element_format(code) else {
            return Err(form(
                pc,
                inst,
                "an element format this front end does not read",
            ));
        };
        let Some(fields) = fields.get(..count) else {
            return Err(form(pc, inst, "more components than its format holds"));
        };
        let words = fields.iter().map(|(first, _)| first / 32 + 1).max();
        let mut loaded = Vec::with_capacity(4);
        for k in 0..words.unwrap_or(0) {
            let by = self.b.u32(k);
            let index = self.b.bin(Bo::Add, at, by);
            loaded.push(self.buffer_word(bound.global, bound.slot, index));
        }
        for (i, &(first, width)) in fields.iter().enumerate() {
            let word = loaded[(first / 32) as usize];
            let signed = matches!(channel, Channel::Snorm | Channel::Sscaled | Channel::Sint);
            let raw = self.field(word, first % 32, width, signed);
            let value = match channel {
                Channel::Uint | Channel::Sint => raw,
                Channel::Float if width == 32 => raw,
                Channel::Float => {
                    let half = self.half(raw, false);
                    self.b.bits(half)
                }
                Channel::Uscaled | Channel::Unorm => {
                    let mut x = self.b.convert(raw, naga::ScalarKind::Float);
                    if channel == Channel::Unorm {
                        let most = self.b.f32(((1u64 << width) - 1) as f32);
                        x = self.b.bin(Bo::Divide, x, most);
                    }
                    self.b.bits(x)
                }
                Channel::Sscaled | Channel::Snorm => {
                    let integer = self.b.sint(raw);
                    let mut x = self.b.convert(integer, naga::ScalarKind::Float);
                    if channel == Channel::Snorm {
                        let (most, least) = (
                            self.b.f32(((1u64 << (width - 1)) - 1) as f32),
                            self.b.f32(-1.0),
                        );
                        x = self.b.bin(Bo::Divide, x, most);
                        x = self.b.math(Mf::Max, &[x, least]);
                    }
                    self.b.bits(x)
                }
            };
            self.vput(data.wrapping_add(i as u8), value);
        }
        Ok(())
    }

    /// Reads `count` components of the element at dword `at` in the format the buffer's header
    /// states: components of one width, 8, 16 or 32 bits, read one way.
    // ponytail: the packed 10-10-10-2 formats have no width; a buffer of one needs its own header code.
    fn typed_by_header(&mut self, bound: &Bound, at: Handle<E>, (count, data): (usize, u8)) {
        let which = self.b.u32(HEADER_FORMAT);
        let header = self.buffer_word(bound.global, bound.slot, which);
        let (byte, eight, seven) = (self.b.u32(0xFF), self.b.u32(8), self.b.u32(7));
        let width = self.b.bin(Bo::And, header, byte);
        let reading = self.b.bin(Bo::ShiftRight, header, eight);
        let reading = self.b.bin(Bo::And, reading, seven);
        let mask = self.b.low_bits(width);
        let (word, five, low5) = (self.b.u32(32), self.b.u32(5), self.b.u32(31));
        let spare = self.b.bin(Bo::Subtract, word, width);
        for i in 0..count {
            let nth = self.b.u32(i as u32);
            let bit = self.b.bin(Bo::Multiply, nth, width);
            let (over, within) = (
                self.b.bin(Bo::ShiftRight, bit, five),
                self.b.bin(Bo::And, bit, low5),
            );
            let index = self.b.bin(Bo::Add, at, over);
            let loaded = self.buffer_word(bound.global, bound.slot, index);
            let shifted = self.b.bin(Bo::ShiftRight, loaded, within);
            let raw = self.b.bin(Bo::And, shifted, mask);
            let up = self.b.shift(Bo::ShiftLeft, false, raw, spare);
            let extended = self.b.shift(Bo::ShiftRight, true, up, spare);
            let unsigned = self.b.convert(raw, naga::ScalarKind::Float);
            let signed = {
                let integer = self.b.sint(extended);
                self.b.convert(integer, naga::ScalarKind::Float)
            };
            let most = self.b.convert(mask, naga::ScalarKind::Float);
            let unorm = self.b.bin(Bo::Divide, unsigned, most);
            let snorm = {
                let one = self.b.u32(1);
                let half = self.b.bin(Bo::ShiftRight, mask, one);
                let most = self.b.convert(half, naga::ScalarKind::Float);
                let scaled = self.b.bin(Bo::Divide, signed, most);
                let least = self.b.f32(-1.0);
                self.b.math(Mf::Max, &[scaled, least])
            };
            let float = {
                let sixteen = self.b.u32(16);
                let narrow = self.b.bin(Bo::Equal, width, sixteen);
                let half = self.half(raw, false);
                let half = self.b.bits(half);
                self.b.select(narrow, half, raw)
            };
            let readings = [
                (Channel::Unorm, self.b.bits(unorm)),
                (Channel::Snorm, self.b.bits(snorm)),
                (Channel::Uscaled, self.b.bits(unsigned)),
                (Channel::Sscaled, self.b.bits(signed)),
                (Channel::Sint, extended),
                (Channel::Float, float),
            ];
            let mut value = raw;
            for (channel, read) in readings {
                let code = self.b.u32(channel as u32);
                let chosen = self.b.bin(Bo::Equal, reading, code);
                value = self.b.select(chosen, read, value);
            }
            self.vput(data.wrapping_add(i as u8), value);
        }
    }

    /// An atomic update of the dword at `pointer`, and what it held before.
    pub(super) fn atomic(
        &mut self,
        pointer: Handle<E>,
        fun: naga::AtomicFunction,
        value: Handle<E>,
    ) -> Handle<E> {
        let compared = matches!(fun, naga::AtomicFunction::Exchange { compare: Some(_) });
        let ty = match compared {
            true => self.module.generate_predeclared_type(
                naga::PredeclaredType::AtomicCompareExchangeWeakResult(Scalar::U32),
            ),
            false => self.ty(TypeInner::Scalar(Scalar::U32)),
        };
        let result = self.b.result(E::AtomicResult {
            ty,
            comparison: compared,
        });
        self.b.push(Statement::Atomic {
            pointer,
            fun,
            value,
            result: Some(result),
        });
        match compared {
            true => self.b.index(result, 0),
            false => result,
        }
    }

    /// The dword at `pointer` made the lesser or the greater of itself and `value`, both read
    /// signed, and what it held before. No update of unsigned dwords does that in one step: it
    /// is tried until no other thread wrote between the read and the store.
    pub(super) fn atomic_signed(
        &mut self,
        pointer: Handle<E>,
        greatest: bool,
        value: Handle<E>,
    ) -> Handle<E> {
        let ty = self.ty(TypeInner::Scalar(Scalar::U32));
        let variable = naga::LocalVariable {
            name: None,
            ty,
            init: None,
        };
        let variable = (self.b.fun.local_variables).append(variable, naga::Span::UNDEFINED);
        let before = self.b.emit(E::LocalVariable(variable));
        self.b.open();
        let old = self.b.load(pointer);
        let (a, b) = (self.b.sint(old), self.b.sint(value));
        let chosen = self
            .b
            .math(if greatest { Mf::Max } else { Mf::Min }, &[a, b]);
        let new = self.b.bits(chosen);
        let ty = self.module.generate_predeclared_type(
            naga::PredeclaredType::AtomicCompareExchangeWeakResult(Scalar::U32),
        );
        let result = self.b.result(E::AtomicResult {
            ty,
            comparison: true,
        });
        self.b.push(Statement::Atomic {
            pointer,
            fun: naga::AtomicFunction::Exchange { compare: Some(old) },
            value: new,
            result: Some(result),
        });
        let (seen, stored) = (self.b.index(result, 0), self.b.index(result, 1));
        self.b.push(Statement::Store {
            pointer: before,
            value: seen,
        });
        let mut accept = naga::Block::new();
        accept.push(Statement::Break, naga::Span::UNDEFINED);
        self.b.push(Statement::If {
            condition: stored,
            accept,
            reject: naga::Block::new(),
        });
        let body = self.b.close();
        self.b.push(Statement::Loop {
            body,
            continuing: naga::Block::new(),
            break_if: None,
        });
        self.b.load(before)
    }

    /// `buffer_atomic_*`: the dword updated in one step, its old value left in the data register.
    fn buffer_atomic(&mut self, pc: u32, inst: &Inst, fun: &str) -> Done {
        let Inst::Mubuf {
            addr,
            data,
            rsrc,
            soffset,
            offset,
            offen,
            idxen,
            ..
        } = inst
        else {
            return Err(refused(pc, inst));
        };
        let bound = self.buffer(pc, *rsrc)?;
        let at = self.buffer_address(pc, &bound, (*addr, *idxen, *offen), *soffset, *offset)?;
        let value = self.vreg(pc, *data)?;
        // A compare-and-swap holds the new value, then the one to find.
        let compare = match fun {
            "cmpswap" => Some(self.vreg(pc, data.wrapping_add(1))?),
            _ => None,
        };
        let signed = match fun {
            "smin" => Some(false),
            "smax" => Some(true),
            _ => None,
        };
        let fun = atomic_function(fun, compare);
        if fun.is_none() && signed.is_none() {
            return Err(form(
                pc,
                inst,
                "a 64-bit update of a buffer of unsigned dwords",
            ));
        }
        self.stored.insert(bound.global);
        self.atomics.insert(bound.global);
        let (global, slot, data) = (bound.global, bound.slot, *data);
        self.when_active(|cx| {
            let pointer = cx.buffer_cell(global, slot, at);
            let old = match (fun, signed) {
                (Some(fun), _) => cx.atomic(pointer, fun, value),
                (None, greatest) => cx.atomic_signed(pointer, greatest == Some(true), value),
            };
            cx.vput(data, old);
            Ok(())
        })
    }

    /// What a component's register holds as the `width` bits a buffer keeps of it, read the way
    /// `reading` numbers (a [`Channel`]): a float scaled and rounded, an integer cut, a float
    /// as it is or as a half.
    // ponytail: a signed scaled value past its field wraps; the hardware clamps it.
    fn encode(
        &mut self,
        bits: Handle<E>,
        width: Handle<E>,
        reading: Handle<E>,
        d16: bool,
    ) -> Handle<E> {
        let mask = self.b.low_bits(width);
        let x = match d16 {
            true => self.half(bits, false),
            false => self.b.float(bits),
        };
        let (zero, one, minus) = (self.b.f32(0.0), self.b.f32(1.0), self.b.f32(-1.0));
        let most = self.b.convert(mask, naga::ScalarKind::Float);
        let unorm = {
            let unit = self.b.math(Mf::Clamp, &[x, zero, one]);
            let scaled = self.b.bin(Bo::Multiply, unit, most);
            let nearest = self.b.math(Mf::Round, &[scaled]);
            self.b.convert(nearest, naga::ScalarKind::Uint)
        };
        let snorm = {
            let by = self.b.u32(1);
            let half = self.b.bin(Bo::ShiftRight, mask, by);
            let most = self.b.convert(half, naga::ScalarKind::Float);
            let unit = self.b.math(Mf::Clamp, &[x, minus, one]);
            let scaled = self.b.bin(Bo::Multiply, unit, most);
            let nearest = self.b.math(Mf::Round, &[scaled]);
            let whole = self.b.convert(nearest, naga::ScalarKind::Sint);
            let pattern = self.b.bits(whole);
            self.b.bin(Bo::And, pattern, mask)
        };
        let uscaled = {
            let whole = self.b.convert(x, naga::ScalarKind::Uint);
            self.b.math(Mf::Min, &[whole, mask])
        };
        let sscaled = {
            let whole = self.b.convert(x, naga::ScalarKind::Sint);
            let pattern = self.b.bits(whole);
            self.b.bin(Bo::And, pattern, mask)
        };
        let raw = self.b.bin(Bo::And, bits, mask);
        let float = {
            let sixteen = self.b.u32(16);
            let narrow = self.b.bin(Bo::Equal, width, sixteen);
            let (wide, half) = match d16 {
                true => (self.widen(bits), raw),
                false => (bits, self.narrow(bits)),
            };
            self.b.select(narrow, half, wide)
        };
        let mut value = raw;
        for (channel, encoded) in [
            (Channel::Unorm, unorm),
            (Channel::Snorm, snorm),
            (Channel::Uscaled, uscaled),
            (Channel::Sscaled, sscaled),
            (Channel::Float, float),
        ] {
            let code = self.b.u32(channel as u32);
            let chosen = self.b.bin(Bo::Equal, reading, code);
            value = self.b.select(chosen, encoded, value);
        }
        value
    }

    /// Stores the components an element of the format `code` holds, of those in `values`.
    fn store_fields(
        &mut self,
        bound: &Bound,
        at: Handle<E>,
        (fields, channel): (&[(u32, u32)], Channel),
        values: &[Handle<E>],
        d16: bool,
    ) {
        for (&(first, width), &bits) in fields.iter().zip(values) {
            let (bits_wide, reading) = (self.b.u32(width), self.b.u32(channel as u32));
            let encoded = self.encode(bits, bits_wide, reading, d16);
            let by = self.b.u32(first / 32);
            let index = self.b.bin(Bo::Add, at, by);
            let pointer = self.buffer_cell(bound.global, bound.slot, index);
            let value = match width {
                32 => encoded,
                _ => {
                    let (shift, mask) = (first % 32, ((1u32 << width) - 1) << (first % 32));
                    let (old, hole, by) =
                        (self.b.load(pointer), self.b.u32(!mask), self.b.u32(shift));
                    let kept = self.b.bin(Bo::And, old, hole);
                    let placed = self.b.bin(Bo::ShiftLeft, encoded, by);
                    self.b.bin(Bo::InclusiveOr, kept, placed)
                }
            };
            self.b.push(Statement::Store { pointer, value });
        }
    }

    /// Stores components in the format the buffer's header states, as far as an element of
    /// its stride holds them: a store of four to a buffer of one component writes one.
    fn store_by_header(&mut self, bound: &Bound, at: Handle<E>, values: &[Handle<E>], d16: bool) {
        let which = self.b.u32(HEADER_FORMAT);
        let header = self.buffer_word(bound.global, bound.slot, which);
        let (byte, eight, seven) = (self.b.u32(0xFF), self.b.u32(8), self.b.u32(7));
        let width = self.b.bin(Bo::And, header, byte);
        let reading = self.b.bin(Bo::ShiftRight, header, eight);
        let reading = self.b.bin(Bo::And, reading, seven);
        let mask = self.b.low_bits(width);
        let room = self.b.bin(Bo::Multiply, bound.stride, eight);
        let (five, low5) = (self.b.u32(5), self.b.u32(31));
        for (i, &bits) in values.iter().enumerate() {
            let encoded = self.encode(bits, width, reading, d16);
            let nth = self.b.u32(i as u32);
            let bit = self.b.bin(Bo::Multiply, nth, width);
            let end = self.b.bin(Bo::Add, bit, width);
            let fits = self.b.bin(Bo::LessEqual, end, room);
            let (over, within) = (
                self.b.bin(Bo::ShiftRight, bit, five),
                self.b.bin(Bo::And, bit, low5),
            );
            let index = self.b.bin(Bo::Add, at, over);
            let pointer = self.buffer_cell(bound.global, bound.slot, index);
            let old = self.b.load(pointer);
            let field = self.b.shift(Bo::ShiftLeft, false, mask, within);
            let hole = self.b.not(field);
            let kept = self.b.bin(Bo::And, old, hole);
            let placed = self.b.shift(Bo::ShiftLeft, false, encoded, within);
            let value = self.b.bin(Bo::InclusiveOr, kept, placed);
            let mut accept = Block::new();
            accept.push(Statement::Store { pointer, value }, Span::UNDEFINED);
            self.b.push(Statement::If {
                condition: fits,
                accept,
                reject: Block::new(),
            });
        }
    }

    /// `buffer_store_format_*`: components written in the format the buffer's descriptor states.
    fn typed_store(&mut self, pc: u32, inst: &Inst, rest: &str) -> Done {
        let Inst::Mubuf {
            addr,
            data,
            rsrc,
            soffset,
            offset,
            offen,
            idxen,
            ..
        } = inst
        else {
            return Err(refused(pc, inst));
        };
        let Some((d16, count)) = components(rest) else {
            return Err(refused(pc, inst));
        };
        let bound = self.buffer(pc, *rsrc)?;
        let at = self.buffer_address(pc, &bound, (*addr, *idxen, *offen), *soffset, *offset)?;
        let mut values = Vec::with_capacity(count);
        for k in 0..count {
            values.push(self.vreg(pc, data.wrapping_add(k as u8))?);
        }
        self.stored.insert(bound.global);
        let Some(word3) = bound.word3 else {
            return self.when_active(|cx| {
                cx.store_by_header(&bound, at, &values, d16);
                Ok(())
            });
        };
        if word3 & 0xFFF != IDENTITY_SELECTION {
            return Err(form(pc, inst, "a descriptor that reorders the components"));
        }
        let Some(format) = element_format(((word3 >> 12) & 0x7F) as u8) else {
            return Err(form(
                pc,
                inst,
                "an element format this front end does not read",
            ));
        };
        self.when_active(|cx| {
            cx.store_fields(&bound, at, format, &values, d16);
            Ok(())
        })
    }

    /// Leaves what a typed read loaded as 16-bit values: a float as a half, an integer cut.
    fn halve_loaded(&mut self, pc: u32, bound: &Bound, (count, data): (usize, u8)) -> Done {
        // The reading is the code's own where it wrote the descriptor, the header's otherwise.
        let whole = match bound
            .word3
            .map(|word3| element_format(((word3 >> 12) & 0x7F) as u8))
        {
            Some(Some((_, channel))) => {
                let whole = matches!(channel, Channel::Uint | Channel::Sint);
                self.b.emit(E::Literal(naga::Literal::Bool(whole)))
            }
            _ => {
                let which = self.b.u32(HEADER_FORMAT);
                let header = self.buffer_word(bound.global, bound.slot, which);
                let (eight, six, four) = (self.b.u32(8), self.b.u32(6), self.b.u32(4));
                let reading = self.b.bin(Bo::ShiftRight, header, eight);
                let reading = self.b.bin(Bo::And, reading, six);
                self.b.bin(Bo::Equal, reading, four)
            }
        };
        for k in 0..count {
            let register = data.wrapping_add(k as u8);
            let loaded = self.vreg(pc, register)?;
            let (half, low) = (self.narrow(loaded), self.b.u32(0xFFFF));
            let cut = self.b.bin(Bo::And, loaded, low);
            let value = self.b.select(whole, cut, half);
            self.vput(register, value);
        }
        Ok(())
    }

    /// `buffer_load_dword*` and `buffer_load_format_*`; a store from a stage that cannot make one is dropped.
    pub(super) fn mubuf(&mut self, pc: u32, inst: &Inst, name: &str) -> Done {
        let Inst::Mubuf {
            addr,
            data,
            rsrc,
            soffset,
            offset,
            offen,
            idxen,
            ..
        } = inst
        else {
            return Err(refused(pc, inst));
        };
        if name.starts_with("buffer_store_") && !self.fragment && self.launch.is_none() {
            // ponytail: a vertex stage has no writable buffer on every backend, and what it stores
            // is read by another pass, never by this draw.
            self.note("a buffer store before the pixel stage, dropped");
            return Ok(());
        }
        if let Some(fun) = name.strip_prefix("buffer_atomic_") {
            return self.buffer_atomic(pc, inst, fun);
        }
        if let Some(rest) = name.strip_prefix("buffer_store_format_") {
            return self.typed_store(pc, inst, rest);
        }
        if let Some(rest) = name.strip_prefix("buffer_store_dword") {
            let count = match rest {
                "" => 1u8,
                "x2" => 2,
                "x3" => 3,
                "x4" => 4,
                _ => return Err(refused(pc, inst)),
            };
            let bound = self.buffer(pc, *rsrc)?;
            let at = self.buffer_address(pc, &bound, (*addr, *idxen, *offen), *soffset, *offset)?;
            let mut stores = Vec::with_capacity(4);
            for k in 0..count {
                let by = self.b.u32(u32::from(k));
                let index = self.b.bin(Bo::Add, at, by);
                let cell = self.buffer_cell(bound.global, bound.slot, index);
                stores.push((cell, self.vreg(pc, data.wrapping_add(k))?));
            }
            self.stored.insert(bound.global);
            self.guarded(stores);
            return Ok(());
        }
        let typed = name
            .strip_prefix("buffer_load_format_")
            .and_then(components);
        let (raw, d16, count) = match (name, typed) {
            ("buffer_load_dword", _) => (true, false, 1),
            ("buffer_load_dwordx2", _) => (true, false, 2),
            ("buffer_load_dwordx3", _) => (true, false, 3),
            ("buffer_load_dwordx4", _) => (true, false, 4),
            (_, Some((d16, count))) => (false, d16, count),
            _ => return Err(refused(pc, inst)),
        };
        let bound = self.buffer(pc, *rsrc)?;
        let at = self.buffer_address(pc, &bound, (*addr, *idxen, *offen), *soffset, *offset)?;
        if !raw {
            // The element's format is the descriptor's: a descriptor the code wrote states it here,
            // and a table's buffer states it in its header, where the binder writes it from the resource.
            match bound.word3 {
                None => self.typed_by_header(&bound, at, (count, *data)),
                Some(word3) => {
                    if word3 & 0xFFF != IDENTITY_SELECTION {
                        return Err(form(pc, inst, "a descriptor that reorders the components"));
                    }
                    let code = ((word3 >> 12) & 0x7F) as u8;
                    self.typed(pc, inst, &bound, at, code, (count, *data))?;
                }
            }
            if d16 {
                self.halve_loaded(pc, &bound, (count, *data))?;
            }
            return Ok(());
        }
        for k in 0..count {
            let by = self.b.u32(k as u32);
            let index = self.b.bin(Bo::Add, at, by);
            let value = self.buffer_word(bound.global, bound.slot, index);
            self.vput(data.wrapping_add(k as u8), value);
        }
        Ok(())
    }

    /// `tbuffer_load_format_*`: an element in the format the instruction states.
    pub(super) fn mtbuf(&mut self, pc: u32, inst: &Inst, name: &str) -> Done {
        let Inst::Mtbuf {
            format,
            addr,
            data,
            rsrc,
            soffset,
            offset,
            offen,
            idxen,
            ..
        } = inst
        else {
            return Err(refused(pc, inst));
        };
        let count = match name {
            "tbuffer_load_format_x" => 1,
            "tbuffer_load_format_xy" => 2,
            "tbuffer_load_format_xyz" => 3,
            "tbuffer_load_format_xyzw" => 4,
            _ => return Err(refused(pc, inst)),
        };
        let bound = self.buffer(pc, *rsrc)?;
        let at = self.buffer_address(pc, &bound, (*addr, *idxen, *offen), *soffset, *offset)?;
        self.typed(pc, inst, &bound, at, *format, (count, *data))
    }
}
