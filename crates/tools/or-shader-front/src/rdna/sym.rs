//! Where each scalar register's value came from, at every instruction: a user datum the pipeline
//! loads, or a word of a table a user pointer leads to. A program names its resources no other way.

use std::collections::{BTreeMap, BTreeSet};

use super::cfg::Cfg;
use super::decode::{Inst, Operand};

/// The scalar registers an operand field can name: `s0`..`s105`, then `vcc`, `ttmp`, `m0`, `exec`.
pub const SCALARS: usize = 128;
pub const VCC: u8 = 106;
pub const M0: u8 = 124;
pub const EXEC: u8 = 126;

/// A dword of a table: follow the pointer in user registers `root` and `root + 1`, then for each
/// offset of `path` the pointer read there, and read at `offset`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Loc {
    pub root: u8,
    pub path: Vec<u32>,
    pub offset: u32,
    /// Added to `offset` where the program indexes the table: a slot the pipeline or a constant buffer names.
    pub index: Option<Index>,
}

/// A table index: `source` shifted left by `shift`, the size of the table's entries.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Index {
    pub source: IndexSource,
    pub shift: u8,
}

/// Where a table index comes from.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IndexSource {
    /// The value the pipeline put in a user register.
    User(u8),
    /// A dword of the constant buffer whose descriptor is at `buffer`, at `offset` plus the user register `plus`.
    Data {
        buffer: Box<Loc>,
        offset: u32,
        plus: Option<u8>,
    },
    /// A dword the program computed, which only a run knows: the descriptor registers hold the
    /// entry's place in the table, and the binding is every entry of it. `site` is the word of the
    /// load, which tells one load's registers from another's.
    Runtime { site: u32 },
}

/// A buffer descriptor the code builds around an address in user registers `root` and `root + 1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Memory {
    pub root: u8,
    /// The element size an indexed read multiplies by, in bytes.
    pub stride: u32,
    /// The descriptor's fourth word: the format in bits 18:12, the component selection in bits 11:0.
    pub word3: u32,
}

impl Loc {
    pub fn plus(&self, bytes: u32) -> Self {
        Self {
            offset: self.offset.wrapping_add(bytes),
            ..self.clone()
        }
    }
}

impl std::fmt::Display for Loc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "s{}", self.root)?;
        for at in &self.path {
            write!(f, "->{at:#x}")?;
        }
        write!(f, "+{:#x}", self.offset)?;
        match &self.index {
            Some(Index {
                source: IndexSource::User(r),
                shift,
            }) => write!(f, "+(s{r}<<{shift})"),
            Some(Index {
                source: IndexSource::Data { buffer, offset, .. },
                shift,
            }) => write!(f, "+([{buffer}]+{offset:#x}<<{shift})"),
            Some(Index {
                source: IndexSource::Runtime { .. },
                ..
            }) => write!(f, "+(computed)"),
            None => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Sym {
    /// Computed by the program.
    Unknown,
    /// A table's word on one path here and something else on another: not a value to compute with.
    Mixed,
    /// The value the pipeline put in this register before the program ran.
    User(u8),
    Word(Loc),
    /// A constant the code wrote.
    Const(u32),
    /// One of a few constants, each written on another path: which, only a run knows.
    Consts(Vec<u32>),
    /// An address of the program's own words: where an instruction is, or that moved by a
    /// number, as the entry of a table the code holds is.
    Code,
    /// A user register's value with constant bits set: a descriptor word built around an address.
    UserOr(u8, u32),
    /// A dword read from a constant buffer.
    Data {
        buffer: Loc,
        offset: u32,
        plus: Option<u8>,
    },
    /// A user datum or a constant buffer's dword shifted left: a table index.
    Scaled(Index),
}

/// What every scalar register holds, and what the few vector registers a move filled carry: a
/// table's word or a constant, which `v_readfirstlane_b32` hands back to a scalar register.
// A state is copied for every block, so the vector registers are kept only where one holds something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    scalars: Vec<Sym>,
    vectors: BTreeMap<u8, Sym>,
    /// What a lane of a vector register holds where `v_writelane_b32` put a scalar register
    /// there, by the register and the lane: how a program keeps a descriptor past its registers.
    spills: BTreeMap<(u8, u8), Sym>,
}

impl State {
    /// What scalar register `register` holds, `None` past the last.
    pub fn get(&self, register: usize) -> Option<&Sym> {
        self.scalars.get(register)
    }

    fn get_mut(&mut self, register: usize) -> Option<&mut Sym> {
        self.scalars.get_mut(register)
    }

    /// What `v_writelane_b32` left in a lane of vector register `n`.
    pub fn spilled(&self, n: u8, lane: u8) -> Option<&Sym> {
        self.spills.get(&(n, lane))
    }

    /// What a whole move left in vector register `n`.
    pub fn moved(&self, n: u8) -> Option<&Sym> {
        self.vectors.get(&n)
    }

    fn vector(&self, n: u8) -> Sym {
        self.vectors.get(&n).cloned().unwrap_or(Sym::Unknown)
    }
}

impl std::ops::Index<usize> for State {
    type Output = Sym;

    fn index(&self, register: usize) -> &Sym {
        &self.scalars[register]
    }
}

impl std::ops::IndexMut<usize> for State {
    fn index_mut(&mut self, register: usize) -> &mut Sym {
        &mut self.scalars[register]
    }
}

pub fn entry() -> State {
    State {
        scalars: (0..SCALARS as u8).map(Sym::User).collect(),
        vectors: BTreeMap::new(),
        spills: BTreeMap::new(),
    }
}

/// What a move through a vector register keeps: whatever the scalar register held, a table's
/// word with its place where a run computed it. A word only some paths gave is not followed.
fn kept(sym: Sym) -> Sym {
    match sym {
        Sym::Mixed => Sym::Unknown,
        _ => sym,
    }
}

/// What a vector instruction's source holds: a vector register's state, or a scalar source's.
fn vheld(state: &State, operand: Operand) -> Sym {
    match operand {
        Operand::Vgpr(n) => state.vector(n),
        other => held(state, other),
    }
}

/// What an instruction leaves of the vector registers a state follows: a whole move keeps what
/// it moved, and any other write forgets.
// ponytail: a move in some lanes counts as one in all, which holds for the lanes a later
// `v_readfirstlane_b32` reads; a register only some lanes rewrote would need a join here.
fn step_vectors(state: &mut State, inst: &Inst) {
    // An address of the code moved on by a number a lane computed is one still: how a program
    // reaches a table it holds at an entry each lane chooses.
    let added = match inst {
        Inst::Vop2 {
            dst, src0, src1, ..
        } => Some((*dst, *src0, Operand::Vgpr(*src1))),
        Inst::Vop3 { dst, srcs, .. } => Some((*dst, srcs[0], srcs[1])),
        _ => None,
    };
    if let Some((dst, a, b)) = added.filter(|_| matches!(inst.name(), "v_add_u32" | "v_add_co_u32"))
    {
        let (a, b) = (vheld(state, a), vheld(state, b));
        let table = |sym: &Sym| matches!(sym, Sym::Word(_) | Sym::Mixed);
        state.spills.retain(|(n, _), _| *n != dst);
        match (a == Sym::Code || b == Sym::Code) && !table(&a) && !table(&b) {
            true => state.vectors.insert(dst, Sym::Code),
            false => state.vectors.remove(&dst),
        };
        return;
    }
    let (first, count) = match inst {
        Inst::Vop1 { dst, .. } if inst.name() == "v_readfirstlane_b32" => (*dst, 0),
        // Both registers are written: neither is followed past the exchange.
        Inst::Vop1 {
            dst,
            src: Operand::Vgpr(other),
            ..
        } if inst.name() == "v_swap_b32" => {
            for written in [dst, other] {
                state.vectors.remove(written);
                state.spills.retain(|(n, _), _| n != written);
            }
            return;
        }
        // A whole move: no trailing word but a literal's own.
        Inst::Vop1 {
            dst, src, extra, ..
        } if inst.name() == "v_mov_b32"
            && (extra.is_none() || matches!(src, Operand::Literal(_))) =>
        {
            let moved = match *src {
                Operand::Vgpr(n) => state.vector(n),
                other => held(state, other),
            };
            match kept(moved) {
                Sym::Unknown => state.vectors.remove(dst),
                moved => state.vectors.insert(*dst, moved),
            };
            state.spills.retain(|(n, _), _| n != dst);
            return;
        }
        // One lane takes a scalar register's value, and the others keep theirs.
        Inst::Vop3 { dst, srcs, .. } if inst.name() == "v_writelane_b32" => {
            state.vectors.remove(dst);
            if let Operand::Int(lane) = srcs[1] {
                match kept(held(state, srcs[0])) {
                    Sym::Unknown => state.spills.remove(&(*dst, lane as u8)),
                    sym => state.spills.insert((*dst, lane as u8), sym),
                };
            }
            return;
        }
        // These write a scalar register or a mask.
        Inst::Vop3 { .. } if inst.name() == "v_readlane_b32" || inst.name().starts_with("v_cmp") => {
            return
        }
        Inst::Vop1 { dst, .. }
        | Inst::Vop2 { dst, .. }
        | Inst::Vop3 { dst, .. }
        | Inst::Vop3p { dst, .. }
        // A 64-bit result fills a register pair.
        | Inst::Vintrp { dst, .. } => (*dst, if inst.name().contains("64") { 2 } else { 1 }),
        // A memory read fills as many registers as its mnemonic or its mask says, and a
        // store none: a register beside its last is another value's, a descriptor's word too.
        Inst::Ds { dst, .. } | Inst::Flat { dst, .. } => (*dst, loaded_registers(inst.name())),
        Inst::Mubuf { data, .. } | Inst::Mtbuf { data, .. } => {
            (*data, loaded_registers(inst.name()))
        }
        Inst::Mimg { data, dmask, .. } => {
            let name = inst.name();
            let count = match name {
                "image_store" | "image_store_mip" => 0,
                _ if name.starts_with("image_gather4") => 4,
                _ if name.starts_with("image_atomic") => 2,
                _ => dmask.count_ones() as u8,
            };
            (*data, count)
        }
        _ => return,
    };
    for k in 0..count {
        let written = first.wrapping_add(k);
        state.vectors.remove(&written);
        state.spills.retain(|(n, _), _| *n != written);
    }
}

/// How many vector registers a memory instruction fills from its first, by its mnemonic; four
/// where the mnemonic does not say.
pub(crate) fn loaded_registers(name: &str) -> u8 {
    if name.contains("_store") || name.contains("_write") {
        return 0;
    }
    let wide = if name.ends_with("_b64") { 2 } else { 1 };
    match name {
        _ if name.ends_with("_xyzw") || name.ends_with("dwordx4") => 4,
        _ if name.ends_with("_xyz") || name.ends_with("dwordx3") => 3,
        _ if name.ends_with("_xy") || name.ends_with("dwordx2") || name.ends_with("_x2") => 2,
        _ if name.starts_with("ds_read2") => 2 * wide,
        _ if name.starts_with("ds_") => wide,
        _ if name.starts_with("global_load") || name.starts_with("flat_load") => 1,
        _ if name.starts_with("buffer_load") || name.starts_with("buffer_atomic") => 1,
        _ => 4,
    }
}

/// The register an operand names, `None` for a constant or a status bit.
pub fn register(operand: Operand) -> Option<u8> {
    Some(match operand {
        Operand::Sgpr(n) => n,
        Operand::VccLo => VCC,
        Operand::VccHi => VCC + 1,
        Operand::Ttmp(n) => 108 + n,
        Operand::M0 => M0,
        Operand::ExecLo => EXEC,
        Operand::ExecHi => EXEC + 1,
        _ => return None,
    })
}

/// How many dwords a scalar or buffer load reads, from its mnemonic.
pub fn load_words(name: &str) -> Option<u8> {
    let rest = name
        .strip_prefix("s_load_dword")
        .or_else(|| name.strip_prefix("s_buffer_load_dword"))?;
    match rest {
        "" => Some(1),
        "x2" => Some(2),
        "x4" => Some(4),
        "x8" => Some(8),
        "x16" => Some(16),
        _ => None,
    }
}

/// Whether a scalar instruction's destination is a register pair.
pub fn wide(name: &str) -> bool {
    (name.ends_with("_b64") || name.ends_with("_i64") || name.ends_with("_u64"))
        && !name.contains("_i32_")
}

/// The table a pointer in registers `base` and `base + 1` leads to; its low half says which.
pub fn table(state: &State, base: u8) -> Option<Loc> {
    match state.get(usize::from(base))? {
        Sym::User(root) => Some(Loc {
            root: *root,
            path: Vec::new(),
            offset: 0,
            index: None,
        }),
        // A pointer read at an index leads to no one table.
        Sym::Word(at) if at.index.is_some() => None,
        Sym::Word(at) => {
            let mut path = at.path.clone();
            path.push(at.offset);
            Some(Loc {
                root: at.root,
                path,
                offset: 0,
                index: None,
            })
        }
        _ => None,
    }
}

/// The descriptor whose first dword register `first` holds, where the `words` registers from it
/// hold that table's consecutive dwords.
pub fn descriptor(state: &State, first: u8, words: u8) -> Option<Loc> {
    let Some(Sym::Word(at)) = state.get(usize::from(first)) else {
        return None;
    };
    (1..words)
        .all(|k| {
            state.get(usize::from(first) + usize::from(k))
                == Some(&Sym::Word(at.plus(4 * u32::from(k))))
        })
        .then(|| at.clone())
}

/// The four constant words of a descriptor the code wrote itself, such as a sampler's.
pub fn literal(state: &State, first: u8) -> Option<[u32; 4]> {
    let mut words = [0; 4];
    for (k, word) in words.iter_mut().enumerate() {
        let Some(Sym::Const(value)) = state.get(usize::from(first) + k) else {
            return None;
        };
        *word = *value;
    }
    Some(words)
}

/// The most descriptors a program is taken to choose among at one place.
const MAX_CHOICES: usize = 4;

/// The constant descriptors the four registers from `first` may hold, where each word is a
/// constant or one of a few: every combination of them, and one alone where [`literal`] finds one.
pub fn literals(state: &State, first: u8) -> Option<Vec<[u32; 4]>> {
    let mut found = vec![[0u32; 4]];
    for k in 0..4 {
        let words = match state.get(usize::from(first) + k)? {
            Sym::Const(word) => std::slice::from_ref(word),
            Sym::Consts(words) => words.as_slice(),
            _ => return None,
        };
        found = (found.iter())
            .flat_map(|so_far| {
                words.iter().map(move |&word| {
                    let mut next = *so_far;
                    next[k] = word;
                    next
                })
            })
            .collect();
        if found.len() > MAX_CHOICES {
            return None;
        }
    }
    Some(found)
}

/// The constants two paths leave a register, where each left it one or a few.
fn joined_constants(a: &Sym, b: &Sym) -> Option<Sym> {
    let of = |sym: &Sym| match sym {
        Sym::Const(word) => Some(vec![*word]),
        Sym::Consts(words) => Some(words.clone()),
        _ => None,
    };
    let mut words = of(a)?;
    words.extend(of(b)?);
    words.sort_unstable();
    words.dedup();
    (words.len() <= MAX_CHOICES).then_some(Sym::Consts(words))
}

/// The buffer descriptor in the four registers from `first` where the code built it around a user address.
pub fn memory(state: &State, first: u8) -> Option<Memory> {
    let at = usize::from(first);
    let Some(Sym::User(root)) = state.get(at) else {
        return None;
    };
    // The address's high half shares its dword with the stride, in bits 29:16.
    let high = match state.get(at + 1)? {
        Sym::User(r) if *r == root + 1 => 0,
        Sym::UserOr(r, bits) if *r == root + 1 => *bits,
        _ => return None,
    };
    let (Some(Sym::Const(_)), Some(Sym::Const(word3))) = (state.get(at + 2), state.get(at + 3))
    else {
        return None;
    };
    Some(Memory {
        root: *root,
        stride: (high >> 16) & 0x3FFF,
        word3: *word3,
    })
}

/// What a scalar source holds: a register's state, or the constant an operand is.
fn held(state: &State, operand: Operand) -> Sym {
    match (register(operand), operand) {
        (Some(r), _) => state.get(usize::from(r)).cloned().unwrap_or(Sym::Unknown),
        (None, Operand::Int(n)) => Sym::Const(n as u32),
        (None, Operand::Literal(word) | Operand::Float(word)) => Sym::Const(word),
        (None, Operand::Null) => Sym::Const(0),
        _ => Sym::Unknown,
    }
}

/// The table word a scalar load reads first: a table at a constant offset, with the index an offset register holds.
pub fn loaded(state: &State, base: u8, offset: u32, soffset: Operand) -> Option<Loc> {
    // A scalar load's immediate is 21 bits, signed, and truncated to a dword.
    if offset >> 20 != 0 {
        return None;
    }
    let index = match soffset {
        Operand::Null => None,
        _ => match held(state, soffset) {
            Sym::Scaled(index) => Some(index),
            Sym::User(r) => Some(Index {
                source: IndexSource::User(r),
                shift: 0,
            }),
            _ => return None,
        },
    };
    let at = table(state, base)?;
    Some(Loc {
        offset: offset & !3,
        index,
        ..at
    })
}

/// [`loaded`], or the table's entry at whatever dword the offset register holds when the load at
/// word `site` runs.
pub fn loaded_at(state: &State, base: u8, offset: u32, soffset: Operand, site: u32) -> Option<Loc> {
    if let Some(at) = loaded(state, base, offset, soffset) {
        return Some(at);
    }
    if offset >> 20 != 0 || soffset == Operand::Null {
        return None;
    }
    // A table's word is no offset: it is a place, not a number.
    if matches!(held(state, soffset), Sym::Word(_) | Sym::Mixed) {
        return None;
    }
    let at = table(state, base)?;
    Some(Loc {
        offset: offset & !3,
        index: Some(Index {
            source: IndexSource::Runtime { site },
            shift: 0,
        }),
        ..at
    })
}

/// Whether a descriptor's place is one only a run knows.
pub fn computed(at: &Loc) -> bool {
    matches!(
        at.index,
        Some(Index {
            source: IndexSource::Runtime { .. },
            ..
        })
    )
}

/// The constant a scalar operation over constants gives, for the ones a descriptor is built with.
fn fold(name: &str, a: u32, b: u32) -> Option<u32> {
    Some(match name {
        "s_and_b32" => a & b,
        "s_or_b32" => a | b,
        "s_xor_b32" => a ^ b,
        "s_andn2_b32" => a & !b,
        "s_orn2_b32" => a | !b,
        "s_lshl_b32" => a << (b & 31),
        "s_lshr_b32" => a >> (b & 31),
        "s_add_u32" | "s_add_i32" => a.wrapping_add(b),
        "s_sub_u32" | "s_sub_i32" => a.wrapping_sub(b),
        "s_mul_i32" => a.wrapping_mul(b),
        _ => return None,
    })
}

/// The dword a constant buffer read loads first, where its descriptor and its offset are known.
fn data(state: &State, base: u8, offset: u32, soffset: Operand) -> Option<Sym> {
    let buffer = descriptor(state, base, 4)?;
    let plus = match soffset {
        Operand::Null => None,
        _ => match held(state, soffset) {
            Sym::User(r) => Some(r),
            _ => return None,
        },
    };
    Some(Sym::Data {
        buffer,
        offset: offset & !3,
        plus,
    })
}

fn set(state: &mut State, register: u8, count: u8, sym: impl Fn(u8) -> Sym) {
    for k in 0..count {
        if let Some(slot) = state.get_mut(usize::from(register) + usize::from(k)) {
            *slot = sym(k);
        }
    }
}

/// `state` after `inst`, for a program 64 lanes wide whose loads are not told apart.
pub fn step(state: &mut State, inst: &Inst) {
    step_at(state, 0, inst, false);
}

/// `state` after the instruction at word `pc`; a lane mask is one register where `wave32`, else a pair.
pub fn step_at(state: &mut State, pc: u32, inst: &Inst, wave32: bool) {
    // The vector registers first, from the scalar ones as the instruction found them: a sum
    // may leave its carry in the scalar register it added.
    step_vectors(state, inst);
    step_scalars(state, pc, inst, wave32);
}

fn step_scalars(state: &mut State, pc: u32, inst: &Inst, wave32: bool) {
    let name = inst.name();
    let mask = if wave32 { 1 } else { 2 };
    let unknown = |_| Sym::Unknown;
    match inst {
        Inst::Sop1 { dst, src, .. } => {
            let Some(to) = register(*dst) else { return };
            let from = register(*src);
            match (name, from) {
                ("s_mov_b32", Some(from)) => {
                    state[usize::from(to)] = state[usize::from(from)].clone()
                }
                ("s_mov_b32", None) => state[usize::from(to)] = held(state, *src),
                ("s_mov_b64", None) => {
                    let low = held(state, *src);
                    // An inline integer is sign-extended; a literal word is not.
                    let high = match *src {
                        Operand::Int(n) if n < 0 => u32::MAX,
                        _ => 0,
                    };
                    set(state, to, 2, |k| match (&low, k) {
                        (Sym::Const(_), 0) => low.clone(),
                        (Sym::Const(_), _) => Sym::Const(high),
                        _ => Sym::Unknown,
                    });
                }
                ("s_mov_b64", Some(from)) => {
                    for k in 0..2 {
                        let value = state.get(usize::from(from) + k).cloned();
                        if let Some(slot) = state.get_mut(usize::from(to) + k) {
                            *slot = value.unwrap_or(Sym::Unknown);
                        }
                    }
                }
                // The address of the next instruction, counted from the code's first word.
                ("s_getpc_b64", _) => set(state, to, 2, |k| match k {
                    0 => Sym::Code,
                    _ => Sym::Const(0),
                }),
                ("s_brev_b32" | "s_not_b32", _) => {
                    state[usize::from(to)] = match (name, held(state, *src)) {
                        ("s_brev_b32", Sym::Const(word)) => Sym::Const(word.reverse_bits()),
                        ("s_not_b32", Sym::Const(word)) => Sym::Const(!word),
                        _ => Sym::Unknown,
                    }
                }
                // One bit of the destination set or cleared: how a descriptor word takes its stride.
                ("s_bitset0_b32" | "s_bitset1_b32", _) => {
                    let bit = match held(state, *src) {
                        Sym::Const(n) => Some(1u32 << (n & 31)),
                        _ => None,
                    };
                    let set = name == "s_bitset1_b32";
                    let old = state[usize::from(to)].clone();
                    state[usize::from(to)] = match (old, bit, set) {
                        (Sym::Const(word), Some(bit), true) => Sym::Const(word | bit),
                        (Sym::Const(word), Some(bit), false) => Sym::Const(word & !bit),
                        (Sym::User(r), Some(bit), true) => Sym::UserOr(r, bit),
                        (Sym::UserOr(r, bits), Some(bit), true) => Sym::UserOr(r, bits | bit),
                        _ => Sym::Unknown,
                    };
                }
                _ => set(state, to, if wide(name) { 2 } else { 1 }, unknown),
            }
            if name.contains("exec") {
                set(state, EXEC, 2, unknown);
            }
        }
        Inst::Sop2 {
            dst, src0, src1, ..
        } => {
            let Some(to) = register(*dst) else { return };
            let (a, b) = (held(state, *src0), held(state, *src1));
            let value = match (name, a, b) {
                ("s_lshl_b32", Sym::User(r), Sym::Const(by)) if by < 32 => Sym::Scaled(Index {
                    source: IndexSource::User(r),
                    shift: by as u8,
                }),
                (
                    "s_lshl_b32",
                    Sym::Data {
                        buffer,
                        offset,
                        plus,
                    },
                    Sym::Const(by),
                ) if by < 32 => Sym::Scaled(Index {
                    source: IndexSource::Data {
                        buffer: Box::new(buffer),
                        offset,
                        plus,
                    },
                    shift: by as u8,
                }),
                ("s_or_b32", Sym::User(r), Sym::Const(bits))
                | ("s_or_b32", Sym::Const(bits), Sym::User(r)) => Sym::UserOr(r, bits),
                ("s_or_b32", Sym::UserOr(r, had), Sym::Const(bits))
                | ("s_or_b32", Sym::Const(bits), Sym::UserOr(r, had)) => Sym::UserOr(r, had | bits),
                ("s_add_u32" | "s_add_i32" | "s_sub_u32" | "s_sub_i32", a, b)
                    if (a == Sym::Code || b == Sym::Code)
                        && !matches!(a, Sym::Word(_) | Sym::Mixed)
                        && !matches!(b, Sym::Word(_) | Sym::Mixed) =>
                {
                    Sym::Code
                }
                (_, Sym::Const(a), Sym::Const(b)) => {
                    fold(name, a, b).map_or(Sym::Unknown, Sym::Const)
                }
                _ => Sym::Unknown,
            };
            set(state, to, if wide(name) { 2 } else { 1 }, |k| match k {
                0 => value.clone(),
                _ => Sym::Unknown,
            });
        }
        // A compare with a constant names the register it reads where the others name the one they write.
        // So does a mode write, which writes no scalar register at all, and a wait, which reads none.
        Inst::Sopk { .. }
            if name.starts_with("s_cmpk_")
                || name.starts_with("s_setreg")
                || name.starts_with("s_waitcnt_") => {}
        Inst::Sopk { dst, imm, .. } => {
            if let Some(to) = register(*dst) {
                let value = match name {
                    "s_movk_i32" => Sym::Const(i32::from(*imm) as u32),
                    _ => Sym::Unknown,
                };
                set(state, to, if wide(name) { 2 } else { 1 }, |_| value.clone());
            }
        }
        Inst::Smem {
            base,
            data: into,
            offset,
            soffset,
            ..
        } => {
            let (Some(to), Some(words)) = (register(*into), load_words(name)) else {
                return;
            };
            if name.starts_with("s_load_dword") {
                match loaded_at(state, *base, *offset, *soffset, pc) {
                    Some(at) => set(state, to, words, |k| Sym::Word(at.plus(4 * u32::from(k)))),
                    None => set(state, to, words, unknown),
                }
                return;
            }
            match data(state, *base, *offset, *soffset) {
                Some(Sym::Data {
                    buffer,
                    offset,
                    plus,
                }) => set(state, to, words, |k| Sym::Data {
                    buffer: buffer.clone(),
                    offset: offset + 4 * u32::from(k),
                    plus,
                }),
                _ => set(state, to, words, unknown),
            }
        }
        // `v_readfirstlane_b32` writes a scalar register, with what a move left in the vector one.
        Inst::Vop1 { dst, src, .. } if name == "v_readfirstlane_b32" => {
            let moved = match *src {
                Operand::Vgpr(n) => state.vector(n),
                _ => Sym::Unknown,
            };
            set(state, *dst, 1, |_| moved.clone());
        }
        Inst::Vopc { src0, extra, .. } => {
            // A comparison of register parts may name its own mask register: bit 15 of its word, then bits 14:8.
            let named = match (src0, extra) {
                (Operand::Sdwa, Some(word)) if (word >> 15) & 1 != 0 => {
                    Some(((word >> 8) & 0x7F) as u8)
                }
                _ => None,
            };
            let base = match (name.starts_with("v_cmpx"), named) {
                (true, _) => EXEC,
                (false, Some(own)) => own,
                (false, None) => VCC,
            };
            set(state, base, mask, unknown);
        }
        Inst::Vop2 { .. } if name.contains("_co_") => set(state, VCC, mask, unknown),
        Inst::Vop3 {
            dst,
            srcs,
            opsel,
            abs,
            ..
        } => {
            if name.starts_with("v_cmpx") {
                set(state, EXEC, mask, unknown);
            } else if name == "v_readlane_b32" {
                // A lane a scalar register was kept in gives back what the register held.
                let back = match (srcs[0], srcs[1]) {
                    (Operand::Vgpr(from), Operand::Int(lane)) => {
                        state.spilled(from, lane as u8).cloned()
                    }
                    _ => None,
                };
                set(state, *dst, 1, |_| back.clone().unwrap_or(Sym::Unknown));
            } else if name.starts_with("v_cmp") {
                set(state, *dst, mask, unknown);
            } else if name.contains("_co_")
                || name.starts_with("v_mad_u64")
                || name.starts_with("v_mad_i64")
            {
                // The carry destination sits in the modifier bits.
                set(state, (opsel << 3 | abs) & 0x7F, mask, unknown);
            }
        }
        _ => {}
    }
}

/// The word two paths agree a register holds where each left it a word of one table: the same
/// word of an entry, of whichever entry that path loaded. A load leaves the word's place beside
/// its register, so the place is one a run computed. An entry is 32 bytes, or 16 where the two
/// words are an odd number of 16 bytes apart, as two samplers are.
fn joined_place(a: &Sym, b: &Sym) -> Option<Sym> {
    let (Sym::Word(a), Sym::Word(b)) = (a, b) else {
        return None;
    };
    if (a.root, &a.path) != (b.root, &b.path) {
        return None;
    }
    let apart = a.offset.abs_diff(b.offset);
    let entry = match (apart % 32, apart % 16) {
        (0, _) => 32,
        (_, 0) => 16,
        _ => return None,
    };
    Some(Sym::Word(Loc {
        root: a.root,
        path: a.path.clone(),
        offset: a.offset % entry,
        index: Some(Index {
            source: IndexSource::Runtime { site: JOINED },
            shift: 0,
        }),
    }))
}

/// The load a joined place names: none of the program's.
const JOINED: u32 = u32::MAX;

/// The state on entry to each block of a program 64 lanes wide.
pub fn solve(cfg: &Cfg) -> BTreeMap<u32, State> {
    solve_for(cfg, false)
}

/// What the vector registers carry along the edges into a block: where every edge that lanes
/// run along agrees, and where every edge taken only once no lane runs agrees.
type Carried = [Option<BTreeMap<u8, Sym>>; 2];

/// What a block's vector registers carry on entry. An edge no lane runs along writes nothing,
/// so it takes nothing away from what the lanes that do arrive carry, unless it carries
/// something else in the same register: then another set of lanes holds another value there.
// ponytail: lanes parked while the others ran may read a register the others moved a
// descriptor into, and find what the register held before; no program of the build reads a
// descriptor so.
fn carried(edges: &Carried) -> BTreeMap<u8, Sym> {
    match edges {
        [Some(live), Some(idle)] => (live.iter())
            .filter(|(n, sym)| idle.get(*n).is_none_or(|other| other == *sym))
            .map(|(n, sym)| (*n, sym.clone()))
            .collect(),
        [Some(only), None] | [None, Some(only)] => only.clone(),
        [None, None] => BTreeMap::new(),
    }
}

/// Whether the edge from `block` to `next` is taken only when no lane runs: the branch of
/// `s_cbranch_execz`, or what follows `s_cbranch_execnz`.
fn idle_edge(block: &super::cfg::Block, next: u32) -> bool {
    let super::cfg::Terminator::Branch { taken, fallthrough } = block.terminator else {
        return false;
    };
    match block.insts.last().map(|(_, inst)| inst.name()) {
        _ if taken == fallthrough => false,
        Some("s_cbranch_execz") => next == taken,
        Some("s_cbranch_execnz") => next == fallthrough,
        _ => false,
    }
}

/// What two paths leave a scalar register that each left something else.
fn joined(slot: &Sym, new: &Sym) -> Sym {
    let table = |sym: &Sym| matches!(sym, Sym::Word(_) | Sym::Mixed);
    match joined_place(slot, new).or_else(|| joined_constants(slot, new)) {
        Some(joined) => joined,
        None if table(slot) || table(new) => Sym::Mixed,
        None => Sym::Unknown,
    }
}

/// What the scalar registers hold along the edges into a block, by the two kinds of edge.
type Held = [Option<Vec<Sym>>; 2];

/// What a block's scalar registers hold on entry. A register that holds a table's word or a
/// constant where lanes arrive holds it for every lane that reads it, whatever the edge taken
/// when none runs left there: the code makes a descriptor beside the first lanes that need it.
fn held_on_entry(edges: &Held) -> Vec<Sym> {
    match edges {
        [Some(live), Some(idle)] => (live.iter().zip(idle))
            .map(|(live, idle)| match (live, idle) {
                _ if live == idle => live.clone(),
                (Sym::Word(_) | Sym::Const(_), Sym::Unknown | Sym::User(_)) => live.clone(),
                _ => joined(live, idle),
            })
            .collect(),
        [Some(only), None] | [None, Some(only)] => only.clone(),
        [None, None] => entry().scalars,
    }
}

/// The state on entry to each block: what every path to it agrees on.
pub fn solve_for(cfg: &Cfg, wave32: bool) -> BTreeMap<u32, State> {
    let mut at: BTreeMap<u32, State> = BTreeMap::from([(0, entry())]);
    // The entry is reached with nothing carried, whatever branches back to it.
    let mut edges: BTreeMap<u32, Carried> = BTreeMap::from([(0, [Some(BTreeMap::new()), None])]);
    let mut scalars: BTreeMap<u32, Held> = BTreeMap::from([(0, [Some(entry().scalars), None])]);
    // Lowest word first and each block once however many paths changed it: code runs forward,
    // so a block is mostly solved after the blocks that lead to it.
    let mut work = BTreeSet::from([0u32]);
    while let Some(start) = work.pop_first() {
        let (Some(block), Some(mut state)) = (cfg.blocks.get(&start), at.get(&start).cloned())
        else {
            continue;
        };
        for (pc, inst) in &block.insts {
            step_at(&mut state, *pc, inst, wave32);
        }
        for next in cfg.successors(start) {
            let idle = usize::from(idle_edge(block, next));
            let slot = &mut edges.entry(next).or_default()[idle];
            match slot {
                Some(agreed) => agreed.retain(|n, sym| state.vectors.get(n) == Some(sym)),
                None => *slot = Some(state.vectors.clone()),
            }
            let vectors = carried(&edges[&next]);
            let slot = &mut scalars.entry(next).or_default()[idle];
            // Whether this edge brought the block's scalar registers anything new.
            let mut moved = false;
            match slot {
                Some(agreed) => {
                    for (agreed, new) in agreed.iter_mut().zip(&state.scalars) {
                        if agreed != new {
                            let both = joined(agreed, new);
                            moved |= *agreed != both;
                            *agreed = both;
                        }
                    }
                }
                None => {
                    *slot = Some(state.scalars.clone());
                    moved = true;
                }
            }
            let entering = moved.then(|| held_on_entry(&scalars[&next]));
            let changed = match at.get_mut(&next) {
                None => {
                    let spills = state.spills.clone();
                    at.insert(
                        next,
                        State {
                            scalars: entering.unwrap_or_else(|| entry().scalars),
                            vectors,
                            spills,
                        },
                    );
                    true
                }
                Some(held) => {
                    let mut changed = held.vectors != vectors;
                    held.vectors = vectors;
                    let kept = held.spills.len();
                    (held.spills).retain(|at, sym| state.spills.get(at) == Some(sym));
                    changed |= held.spills.len() != kept;
                    if let Some(entering) = entering.filter(|new| held.scalars != *new) {
                        held.scalars = entering;
                        changed = true;
                    }
                    changed
                }
            };
            if changed {
                work.insert(next);
            }
        }
    }
    at
}
