//! Which `v_readfirstlane_b32` of a program read what this lane holds too, so that a lane
//! lowered alone reads the hardware's value: the source is one every lane that runs holds the
//! same value in, or the read opens a loop over the wave's distinct values, which keeps the
//! wave to the lanes that hold the value read. What the lanes of a saved mask agreed on when
//! it was saved, and nothing wrote since, they agree on when the mask is put back; and the lanes
//! a branch left waiting hold what they held, whatever the lanes that ran wrote.

use std::collections::{BTreeMap, BTreeSet};

use crate::rdna::cfg::Cfg;
use crate::rdna::decode::{Inst, Operand};
use crate::rdna::sym;

/// The vector registers every lane that runs holds one value in, a bit a register.
type Same = [u64; 4];

/// What is known of a mask of the lanes that ran, saved in a scalar register.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Kept {
    /// The vector registers its lanes agreed on when it was saved, less every one written since.
    all: Same,
    /// Where the lanes that run were narrowed from this mask once and not changed since: the
    /// registers its lanes agreed on at the narrowing, which those left waiting still hold.
    idle: Option<Same>,
    /// Whether the lanes that run are this mask's.
    fresh: bool,
}

/// What the lanes that run agree on: the vector registers they hold one value in, and the
/// scalar registers that hold a lane mask with one bit for all of them, as a condition every
/// lane shares is kept.
#[derive(Clone, PartialEq, Eq)]
struct Agreed {
    same: Same,
    level: u128,
    /// The masks of the lanes that ran saved in scalar registers, by the register. `None`
    /// before any path has said: every register, under every mask.
    saved: Option<Saved>,
}

/// The saved masks by their register, in its order: a few, copied for every block.
#[derive(Clone, PartialEq, Eq, Default)]
struct Saved(Vec<(u8, Kept)>);

impl Saved {
    fn place(&self, r: u8) -> Result<usize, usize> {
        self.0.binary_search_by_key(&r, |entry| entry.0)
    }

    fn get(&self, r: &u8) -> Option<&Kept> {
        self.place(*r).ok().map(|at| &self.0[at].1)
    }

    fn insert(&mut self, r: u8, kept: Kept) {
        match self.place(r) {
            Ok(at) => self.0[at].1 = kept,
            Err(at) => self.0.insert(at, (r, kept)),
        }
    }

    fn remove(&mut self, r: &u8) {
        if let Ok(at) = self.place(*r) {
            self.0.remove(at);
        }
    }

    fn iter(&self) -> impl Iterator<Item = (&u8, &Kept)> {
        self.0.iter().map(|(r, kept)| (r, kept))
    }

    fn iter_mut(&mut self) -> impl Iterator<Item = (&u8, &mut Kept)> {
        self.0.iter_mut().map(|(r, kept)| (&*r, kept))
    }

    fn values_mut(&mut self) -> impl Iterator<Item = &mut Kept> {
        self.0.iter_mut().map(|(_, kept)| kept)
    }
}

impl FromIterator<(u8, Kept)> for Saved {
    /// From entries in the order of their registers, as another's are.
    fn from_iter<I: IntoIterator<Item = (u8, Kept)>>(entries: I) -> Self {
        Self(entries.into_iter().collect())
    }
}

impl Agreed {
    fn nothing() -> Self {
        Self {
            same: [0; 4],
            level: 0,
            saved: Some(Saved::default()),
        }
    }

    fn all() -> Self {
        Self {
            same: [!0; 4],
            level: !0,
            saved: None,
        }
    }

    /// What the lanes of the mask saved in `r` still agree on.
    fn kept(&self, operand: &Operand) -> Same {
        let held = |saved: &Saved| {
            sym::register(*operand).and_then(|r| saved.get(&r).map(|kept| kept.all))
        };
        match &self.saved {
            Some(saved) => held(saved).unwrap_or([0; 4]),
            None => [!0; 4],
        }
    }

    /// What the lanes of the mask saved in `r` that a branch left waiting agree on.
    fn waiting(&self, operand: &Operand) -> Same {
        let held = |saved: &Saved| {
            sym::register(*operand).and_then(|r| saved.get(&r).and_then(|kept| kept.idle))
        };
        match &self.saved {
            Some(saved) => held(saved).unwrap_or([0; 4]),
            None => [!0; 4],
        }
    }

    /// What both of two paths leave.
    fn met(&self, other: &Self) -> Self {
        let saved = match (&self.saved, &other.saved) {
            (None, saved) | (saved, None) => saved.clone(),
            (Some(a), Some(b)) => Some(
                (a.iter())
                    .filter_map(|(r, kept)| {
                        let also = b.get(r)?;
                        let both = |x: Same, y: Same| [0, 1, 2, 3].map(|k| x[k] & y[k]);
                        let met = Kept {
                            all: both(kept.all, also.all),
                            idle: kept.idle.zip(also.idle).map(|(x, y)| both(x, y)),
                            fresh: kept.fresh && also.fresh,
                        };
                        Some((*r, met))
                    })
                    .collect(),
            ),
        };
        Self {
            same: [0, 1, 2, 3].map(|k| self.same[k] & other.same[k]),
            level: self.level & other.level,
            saved,
        }
    }

    /// Whether a scalar source is a mask with one bit for every lane that runs: the lanes
    /// that run themselves, none or all, or a register that holds such a mask.
    fn level(&self, operand: &Operand) -> bool {
        match operand {
            Operand::ExecLo | Operand::Int(0 | -1) => true,
            _ => sym::register(*operand).is_some_and(|r| r < 128 && self.level >> r & 1 != 0),
        }
    }

    fn set_level(&mut self, operand: &Operand, on: bool) {
        if let Some(r) = sym::register(*operand) {
            self.wrote(r, on);
        }
    }

    /// A scalar instruction wrote `dst`: a pair where its mnemonic says 64 bits or the result
    /// is a mask, and one register otherwise.
    fn put(&mut self, dst: &Operand, level: bool, name: &str) {
        match sym::register(*dst) {
            Some(r) if !level && !name.contains("64") => self.loaded(r),
            _ => self.set_level(dst, level),
        }
    }

    /// What the lanes of a mask agree on: of those that run, or of one saved.
    fn within(&self, operand: &Operand) -> Same {
        match operand {
            Operand::ExecLo => self.same,
            _ => self.kept(operand),
        }
    }

    /// Scalar register `r` was written, with such a mask or not. A mask of 64 lanes is a pair
    /// kept under its first register, so the pairs that begin beside `r` hold one no longer.
    fn wrote(&mut self, r: u8, on: bool) {
        self.loaded(r.saturating_add(1));
        self.loaded(r);
        if on && r < 128 {
            self.level |= 1 << r;
        }
    }

    /// Scalar register `r` alone was written: the pair it begins and the pair it ends hold
    /// a mask no longer.
    fn loaded(&mut self, r: u8) {
        for beside in [r.wrapping_sub(1), r] {
            if beside < 128 {
                self.level &= !(1 << beside);
            }
            if let Some(saved) = &mut self.saved {
                saved.remove(&beside);
            }
        }
    }
}

fn has(same: &Same, n: u8) -> bool {
    same[usize::from(n >> 6)] >> (n & 63) & 1 != 0
}

fn set(same: &mut Same, n: u8, on: bool) {
    let (word, bit) = (&mut same[usize::from(n >> 6)], 1u64 << (n & 63));
    match on {
        true => *word |= bit,
        false => *word &= !bit,
    }
}

/// What a scalar instruction leaves of the masks the lanes agree on: a choice or a logic
/// instruction of two such masks is one, and any other result is not.
fn scalar(agreed: &mut Agreed, inst: &Inst, wave32: bool) {
    let name = inst.name();
    // A mask is as wide as the wave: a narrower instruction computes with a number.
    let suffix = if wave32 { "_b32" } else { "_b64" };
    let width = |kind: &str| name.strip_prefix(kind) == Some(suffix);
    match inst {
        Inst::Sop2 {
            dst, src0, src1, ..
        } => {
            let logic = ["s_cselect", "s_and", "s_or", "s_xor", "s_andn2", "s_orn2"];
            let logic = logic.iter().any(|kind| width(kind))
                || ["s_nand", "s_nor", "s_xnor"].iter().any(|kind| width(kind));
            let level = logic && agreed.level(src0) && agreed.level(src1);
            // Lanes of a saved mask, or of those that run, agree on what all of them did.
            let within = match () {
                _ if width("s_and") => {
                    let (a, b) = (agreed.within(src0), agreed.within(src1));
                    [0, 1, 2, 3].map(|k| a[k] | b[k])
                }
                _ if width("s_andn2") => agreed.within(src0),
                _ => [0; 4],
            };
            agreed.put(dst, level, name);
            if let (Some(r), Some(saved)) = (sym::register(*dst), &mut agreed.saved) {
                if within != [0; 4] {
                    let kept = Kept {
                        all: within,
                        idle: None,
                        fresh: false,
                    };
                    saved.insert(r, kept);
                }
            }
        }
        Inst::Sop1 { dst, src, .. } => {
            // The lanes that ran, saved as the mask is changed, are a mask of them all.
            let level = (width("s_mov") || width("s_not")) && agreed.level(src);
            agreed.put(dst, level, name);
        }
        Inst::Sopk { dst, .. } => agreed.put(dst, false, name),
        // A load fills as many registers from its first as its mnemonic says, sixteen at most.
        Inst::Smem { data, .. } => {
            if let Some(first) = sym::register(*data) {
                let words = sym::load_words(name).unwrap_or(16);
                for r in first..first.saturating_add(words) {
                    agreed.loaded(r);
                }
            }
        }
        _ => {}
    }
}

/// Whether an instruction changes the lanes that run, and if so whether it only takes lanes
/// away: the lanes left still agree on what they agreed on, where lanes brought back hold
/// whatever they held.
fn masks(inst: &Inst) -> Option<bool> {
    let name = inst.name();
    let exec = |operand: &Operand| matches!(operand, Operand::ExecLo | Operand::ExecHi);
    if name.starts_with("v_cmpx") {
        return Some(true);
    }
    match inst {
        Inst::Sop1 { .. } if name.contains("saveexec") || name.contains("wrexec") => {
            Some(name.starts_with("s_and_saveexec") || name.starts_with("s_andn1_saveexec"))
        }
        Inst::Sop1 { dst, .. } | Inst::Sopk { dst, .. } if exec(dst) => Some(false),
        Inst::Sop2 {
            dst, src0, src1, ..
        } if exec(dst) => Some(match name.split('_').nth(1) {
            Some("and") => exec(src0) || exec(src1),
            Some("andn2") => exec(src0),
            _ => false,
        }),
        _ => None,
    }
}

/// What a vector instruction of the one-, two- and three-source encodings leaves of `dst`;
/// `chosen` is whether the mask a choice between two sources reads is one the lanes agree on.
fn vop(same: &mut Same, name: &str, dst: u8, sources: &[Operand], chosen: bool) {
    // These write a scalar register or a mask.
    if matches!(name, "v_readfirstlane_b32" | "v_readlane_b32" | "v_nop")
        || name.starts_with("v_cmp")
    {
        return;
    }
    if name == "v_movreld_b32" {
        for k in 0..super::REGISTER_ARRAY {
            set(same, dst.wrapping_add(k), false);
        }
        return;
    }
    if let ("v_swap_b32", Some(&Operand::Vgpr(other))) = (name, sources.first()) {
        let (a, b) = (has(same, dst), has(same, other));
        set(same, dst, b);
        set(same, other, a);
        return;
    }
    let wide = name.contains("64");
    let permuted =
        (sources.iter()).any(|s| matches!(s, Operand::Dpp | Operand::Dpp8 | Operand::Dpp8Fi));
    // A value chosen by a mask's bit of each lane, or by the lane's own number.
    let by_lane = (name.starts_with("v_cndmask") && !chosen)
        || ["v_writelane", "v_movrels", "v_permlane", "v_mbcnt"]
            .iter()
            .any(|kind| name.starts_with(kind))
        || ["_ci_", "addc", "subb"]
            .iter()
            .any(|kind| name.contains(kind));
    // A result that fills part of its register, or adds to it, keeps what the rest held. A
    // 16-bit float widened, and two results packed as halves, fill the register.
    let whole = name == "v_cvt_f32_f16" || name.starts_with("v_cvt_pk");
    let keeps =
        sources.contains(&Operand::Sdwa) || (name.contains("16") && !whole) || name.contains("mac");
    let agreed = !wide
        && !permuted
        && !by_lane
        && (!keeps || has(same, dst))
        && (sources.iter()).all(|source| match source {
            Operand::Vgpr(n) => has(same, *n),
            _ => true,
        });
    set(same, dst, agreed);
    if wide {
        set(same, dst.wrapping_add(1), false);
    }
}

/// The register a selection of parts reads where its result fills the whole destination: a
/// scalar register's part is one value, and a vector register's is what the register is.
fn selected(src: Operand, extra: Option<u32>) -> Operand {
    match (src, extra) {
        (Operand::Sdwa, Some(word)) if (word >> 8) & 7 == 6 => match (word >> 23) & 1 {
            0 => Operand::Vgpr(word as u8),
            _ => Operand::Int(0),
        },
        _ => src,
    }
}

/// Whether every source of a comparison is one value in every lane that runs: its mask
/// then has one bit for all of them.
fn compared(same: &Same, sources: &[Operand]) -> bool {
    (sources.iter()).all(|source| match source {
        Operand::Vgpr(n) => has(same, *n),
        Operand::Sdwa | Operand::Dpp | Operand::Dpp8 | Operand::Dpp8Fi => false,
        _ => true,
    })
}

/// What one instruction leaves of what the lanes agree on.
fn step(agreed: &mut Agreed, inst: &Inst, wave32: bool) {
    let name = inst.name();
    if let Some(narrows) = masks(inst) {
        // The mask saved beside the change is the lanes that ran.
        let saved = match inst {
            Inst::Sop1 { dst, .. } if name.contains("saveexec") => Some(*dst),
            _ => None,
        };
        let before = agreed.same;
        // A mask is as wide as the wave: a narrower instruction moves half of one.
        let whole = name.ends_with(if wave32 { "_b32" } else { "_b64" });
        if !narrows {
            // A saved mask put back, or lanes of one: its lanes agree on what they did.
            let back = match inst {
                Inst::Sop1 { src, .. } if whole && name.starts_with("s_mov_b") => agreed.kept(src),
                Inst::Sop2 { src0, src1, .. } if whole && name.starts_with("s_and_b") => {
                    let (a, b) = (agreed.kept(src0), agreed.kept(src1));
                    [0, 1, 2, 3].map(|k| a[k] | b[k])
                }
                // The lanes of the mask that the branch left waiting take their turn.
                Inst::Sop2 {
                    src0,
                    src1: Operand::ExecLo,
                    ..
                } if whole && name.starts_with("s_andn2_b") => {
                    let (a, b) = (agreed.kept(src0), agreed.waiting(src0));
                    [0, 1, 2, 3].map(|k| a[k] | b[k])
                }
                Inst::Sop2 { src0, .. } if whole && name.starts_with("s_andn2_b") => {
                    agreed.kept(src0)
                }
                _ => [0; 4],
            };
            agreed.same = back;
            agreed.level = 0;
        }
        // The lanes that run are no saved mask's any more, but the one put back whole; the
        // lanes a narrowing leaves waiting are those of the masks they were.
        let back = match inst {
            Inst::Sop1 { src, .. } if whole && name.starts_with("s_mov_b") => sym::register(*src),
            _ => None,
        };
        if let Some(kept) = &mut agreed.saved {
            for (r, kept) in kept.iter_mut() {
                kept.idle = (narrows && kept.fresh).then_some(kept.all);
                kept.fresh = Some(*r) == back;
            }
        }
        if let Some(dst) = saved {
            agreed.set_level(&dst, narrows);
            if let (true, Some(r), Some(kept)) = (whole, sym::register(dst), &mut agreed.saved) {
                let idle = narrows.then_some(before);
                kept.insert(
                    r,
                    Kept {
                        all: before,
                        idle,
                        fresh: false,
                    },
                );
            }
        }
        return;
    }
    // The lanes that run, kept in a scalar register as they stand.
    let kept = match inst {
        Inst::Sop1 {
            dst,
            src: Operand::ExecLo,
            ..
        } if name == if wave32 { "s_mov_b32" } else { "s_mov_b64" } => sym::register(*dst),
        _ => None,
    };
    scalar(agreed, inst, wave32);
    if let (Some(r), Some(saved)) = (kept, &mut agreed.saved) {
        let kept = Kept {
            all: agreed.same,
            idle: None,
            fresh: true,
        };
        saved.insert(r, kept);
    }
    written(agreed, inst);
    // A register written is one the lanes of a saved mask no longer all hold their value in.
    let wrote = wrote(inst);
    if let Some(saved) = &mut agreed.saved {
        for kept in saved.values_mut() {
            kept.all = [0, 1, 2, 3].map(|k| kept.all[k] & !wrote[k]);
        }
    }
}

/// The vector registers an instruction writes, a bit each; every one for an instruction
/// this front end does not know.
fn wrote(inst: &Inst) -> Same {
    let name = inst.name();
    let mut wrote = [0; 4];
    let (first, count) = match inst {
        _ if matches!(name, "v_readfirstlane_b32" | "v_readlane_b32" | "v_nop")
            || name.starts_with("v_cmp") =>
        {
            return wrote
        }
        Inst::Unknown { .. } => return [!0; 4],
        Inst::Vop1 { dst, src, .. } if name == "v_swap_b32" => {
            if let Operand::Vgpr(other) = src {
                set(&mut wrote, *other, true);
            }
            (*dst, 1)
        }
        Inst::Vop1 { dst, .. } if name == "v_movreld_b32" => (*dst, super::REGISTER_ARRAY),
        Inst::Vop1 { dst, .. }
        | Inst::Vop2 { dst, .. }
        | Inst::Vop3 { dst, .. }
        | Inst::Vop3p { dst, .. }
        | Inst::Vintrp { dst, .. } => (*dst, if name.contains("64") { 2 } else { 1 }),
        Inst::Mubuf { data, tfe, .. } | Inst::Mtbuf { data, tfe, .. } => {
            (*data, sym::loaded_registers(name) + u8::from(*tfe))
        }
        Inst::Flat { dst, .. } | Inst::Ds { dst, .. } => (*dst, sym::loaded_registers(name)),
        Inst::Mimg {
            data, dmask, tfe, ..
        } => {
            let count = match name {
                "image_store" | "image_store_mip" => 0,
                _ if name.starts_with("image_gather4") => 4,
                _ if name.starts_with("image_atomic") => 2,
                _ => dmask.count_ones() as u8,
            };
            (*data, count + u8::from(*tfe))
        }
        _ => return wrote,
    };
    for k in 0..count {
        set(&mut wrote, first.wrapping_add(k), true);
    }
    wrote
}

/// What one instruction that leaves the lanes that run as they are leaves of what they agree on.
fn written(agreed: &mut Agreed, inst: &Inst) {
    let name = inst.name();
    let chosen = match inst {
        Inst::Vop2 { .. } => agreed.level(&Operand::VccLo),
        Inst::Vop3 { srcs, .. } => agreed.level(&srcs[2]),
        _ => false,
    };
    // A comparison's mask, in the register its encoding names.
    if name.starts_with("v_cmp_") {
        match inst {
            // A comparison of parts names its mask register in its trailing word.
            Inst::Vopc {
                src0: Operand::Sdwa,
                extra: Some(word),
                ..
            } => {
                let named = (word >> 15) & 1 != 0;
                let dst = if named {
                    ((word >> 8) & 0x7F) as u8
                } else {
                    sym::VCC
                };
                agreed.wrote(dst, false);
            }
            Inst::Vopc { src0, src1, .. } => {
                let level = compared(&agreed.same, &[*src0, Operand::Vgpr(*src1)]);
                agreed.set_level(&Operand::VccLo, level);
            }
            Inst::Vop3 { dst, srcs, .. } => {
                let level = compared(&agreed.same, &srcs[..2]);
                agreed.wrote(*dst, level);
            }
            _ => {}
        }
    }
    // A scalar register a vector instruction writes holds no mask.
    if let Inst::Vop1 { dst, .. } | Inst::Vop3 { dst, .. } = inst {
        if matches!(name, "v_readfirstlane_b32" | "v_readlane_b32") {
            agreed.wrote(*dst, false);
        }
    }
    // A sum's carry is each lane's own.
    if let Inst::Vop3 { abs, opsel, .. } = inst {
        if name.contains("_co_") {
            agreed.wrote(((opsel << 3) | abs) & 0x7F, false);
        }
    }
    if let Inst::Vop2 { .. } = inst {
        if name.contains("_co_") || name.contains("addc") || name.contains("subb") {
            agreed.set_level(&Operand::VccLo, false);
        }
    }
    let same = &mut agreed.same;
    match inst {
        Inst::Vop1 {
            dst, src, extra, ..
        } => vop(same, name, *dst, &[selected(*src, *extra)], false),
        Inst::Vop2 {
            dst,
            src0,
            src1,
            extra,
            ..
        } => {
            let sources = [selected(*src0, *extra), Operand::Vgpr(*src1)];
            vop(same, name, *dst, &sources, chosen)
        }
        Inst::Vop3 { dst, srcs, .. } => {
            // The mask a choice reads is no value of its own.
            let read = if name.starts_with("v_cndmask") { 2 } else { 3 };
            vop(same, name, *dst, &srcs[..read], chosen);
        }
        Inst::Vop3p { dst, srcs, .. } => vop(same, name, *dst, srcs, false),
        Inst::Vintrp { dst, .. } => set(same, *dst, false),
        // One address read by every lane is one value, a store fills nothing.
        Inst::Mubuf {
            addr,
            data,
            offen,
            idxen,
            tfe,
            ..
        }
        | Inst::Mtbuf {
            addr,
            data,
            offen,
            idxen,
            tfe,
            ..
        } => {
            let used = u8::from(*offen) + u8::from(*idxen);
            let known = name.starts_with("buffer_load")
                && (0..used).all(|k| has(same, addr.wrapping_add(k)));
            for k in 0..sym::loaded_registers(name) + u8::from(*tfe) {
                set(same, data.wrapping_add(k), known);
            }
        }
        // Scratch memory is each lane's own, whatever the address.
        Inst::Flat { addr, dst, .. } => {
            let shared = name.starts_with("global_load_") || name.starts_with("flat_load_");
            let known = shared && has(same, *addr) && has(same, addr.wrapping_add(1));
            for k in 0..sym::loaded_registers(name) {
                set(same, dst.wrapping_add(k), known);
            }
        }
        Inst::Ds { dst, .. } => {
            for k in 0..sym::loaded_registers(name) {
                set(same, dst.wrapping_add(k), false);
            }
        }
        Inst::Mimg {
            data, dmask, tfe, ..
        } => {
            let count = match name {
                "image_store" | "image_store_mip" => 0,
                _ if name.starts_with("image_gather4") => 4,
                _ if name.starts_with("image_atomic") => 2,
                _ => dmask.count_ones() as u8,
            };
            for k in 0..count + u8::from(*tfe) {
                set(same, data.wrapping_add(k), false);
            }
        }
        Inst::Unknown { .. } => *agreed = Agreed::nothing(),
        // The scalar encodings, a comparison into a mask and an export write no vector register.
        Inst::Sop2 { .. }
        | Inst::Sopk { .. }
        | Inst::Sop1 { .. }
        | Inst::Sopc { .. }
        | Inst::Sopp { .. }
        | Inst::Smem { .. }
        | Inst::Vopc { .. }
        | Inst::Exp { .. } => {}
    }
}

/// Whether the instructions after `v_readfirstlane_b32 scalar, vector` keep the wave to the
/// lanes whose `vector` is the value read, before anything else: the loop a program handles
/// each distinct value of a wave in. A lane's turn comes with its own value in `scalar`.
/// A value of 64 bits is read a half at a time and compared whole: the lanes kept hold both.
fn waterfall(rest: &[(u32, Inst)], scalar: u8, vector: u8) -> bool {
    let pair = |a: &Operand, b: &Operand, down: u8| {
        let pair = (Operand::Sgpr(scalar - down), Operand::Vgpr(vector - down));
        (*a, *b) == pair || (*b, *a) == pair
    };
    let read = |a: &Operand, b: &Operand| pair(a, b, 0);
    let high = scalar > 0 && vector > 0;
    let read_wide = |a: &Operand, b: &Operand| pair(a, b, 0) || (high && pair(a, b, 1));
    let equal = |name: &str| matches!(name, "v_cmp_eq_u32" | "v_cmp_eq_i32");
    let equal_wide = |name: &str| matches!(name, "v_cmp_eq_u64" | "v_cmp_eq_i64");
    // The registers that hold a mask of lanes within those that hold the value read.
    let mut alike = BTreeSet::new();
    let held = |alike: &BTreeSet<u8>, operand: &Operand| {
        sym::register(*operand).is_some_and(|r| alike.contains(&r))
    };
    for (_, inst) in rest {
        let name = inst.name();
        match inst {
            // The comparison that keeps the lanes itself: of this value, or of another
            // the same loop read before it.
            Inst::Vopc { src0, src1, .. } if name.starts_with("v_cmpx_eq_") => {
                if matches!(name, "v_cmpx_eq_u32" | "v_cmpx_eq_i32")
                    && read(src0, &Operand::Vgpr(*src1))
                {
                    return true;
                }
            }
            Inst::Vopc { src0, src1, .. } if name.starts_with("v_cmp_") => {
                let own = Operand::Vgpr(*src1);
                let kept = (equal(name) && read(src0, &own))
                    || (equal_wide(name) && read_wide(src0, &own));
                match kept {
                    true => alike.insert(sym::VCC),
                    false => alike.remove(&sym::VCC),
                };
            }
            Inst::Vop3 { dst, srcs, .. } if name.starts_with("v_cmp_") => {
                let kept = (equal(name) && read(&srcs[0], &srcs[1]))
                    || (equal_wide(name) && read_wide(&srcs[0], &srcs[1]));
                match kept {
                    true => alike.insert(*dst),
                    false => alike.remove(dst),
                };
            }
            // Another value of the same loop read, into another register.
            Inst::Vop1 { dst, .. } if name == "v_readfirstlane_b32" && *dst != scalar => {
                alike.remove(dst);
            }
            Inst::Sop1 { src, .. } if name.starts_with("s_and_saveexec") => {
                return held(&alike, src);
            }
            Inst::Sop2 {
                dst, src0, src1, ..
            } if name.starts_with("s_and_b") => {
                let exec = |operand: &Operand| matches!(operand, Operand::ExecLo);
                let within = held(&alike, src0) || held(&alike, src1);
                if exec(dst) {
                    return within && (exec(src0) || exec(src1));
                }
                let Some(dst) = sym::register(*dst).filter(|r| *r != scalar) else {
                    return false;
                };
                match within {
                    true => alike.insert(dst),
                    false => alike.remove(&dst),
                };
            }
            _ => return false,
        }
    }
    false
}

/// The words of the `v_readfirstlane_b32` that read what this lane holds too.
pub(super) fn first_lanes(cfg: &Cfg, wave32: bool) -> BTreeSet<u32> {
    // What every path to a block agrees on: nothing at the entry, whose registers are each
    // lane's own, and everything elsewhere until a path says less.
    let mut entry: BTreeMap<u32, Agreed> =
        (cfg.blocks.keys().map(|&start| (start, Agreed::all()))).collect();
    entry.insert(0, Agreed::nothing());
    let mut work: BTreeSet<u32> = cfg.blocks.keys().copied().collect();
    while let Some(start) = work.pop_first() {
        let (Some(block), Some(mut agreed)) = (cfg.blocks.get(&start), entry.get(&start).cloned())
        else {
            continue;
        };
        for (_, inst) in &block.insts {
            step(&mut agreed, inst, wave32);
        }
        for next in cfg.successors(start) {
            let Some(held) = entry.get_mut(&next).filter(|_| next != 0) else {
                continue;
            };
            let met = held.met(&agreed);
            if met != *held {
                *held = met;
                work.insert(next);
            }
        }
    }
    let mut exact = BTreeSet::new();
    for (start, block) in &cfg.blocks {
        let mut agreed = entry.get(start).cloned().unwrap_or_else(Agreed::nothing);
        for (at, (pc, inst)) in block.insts.iter().enumerate() {
            if let Inst::Vop1 {
                dst,
                src: Operand::Vgpr(n),
                ..
            } = inst
            {
                let rest = block.insts.get(at + 1..).unwrap_or_default();
                if inst.name() == "v_readfirstlane_b32"
                    && (has(&agreed.same, *n) || waterfall(rest, *dst, *n))
                {
                    exact.insert(*pc);
                }
            }
            step(&mut agreed, inst, wave32);
        }
    }
    exact
}

/// The `v_readfirstlane_b32` at a program's entry that read a register the launch gave it,
/// by their words, each with the register: nothing has written the register and every lane
/// still runs, so the first lane is the first thread of the wave.
pub(super) fn entry_lanes(cfg: &Cfg) -> BTreeMap<u32, u8> {
    let mut found = BTreeMap::new();
    let mut written = [0u64; 4];
    for (pc, inst) in cfg.blocks.get(&0).map_or(&[][..], |block| &block.insts) {
        let name = inst.name();
        match inst {
            Inst::Vop1 {
                src: Operand::Vgpr(n),
                ..
            } if name == "v_readfirstlane_b32" => {
                if !has(&written, *n) {
                    found.insert(*pc, *n);
                }
            }
            // Every lane is set to run, as it does at the entry.
            Inst::Sop1 {
                dst: Operand::ExecLo,
                src: Operand::Int(-1),
                ..
            } if name.starts_with("s_mov_b") => {}
            _ if masks(inst).is_some() => break,
            Inst::Vop1 { dst, .. } | Inst::Vop2 { dst, .. } | Inst::Vop3 { dst, .. } => {
                if matches!(name, "v_movreld_b32" | "v_swap_b32") {
                    break;
                }
                set(&mut written, *dst, true);
                set(&mut written, dst.wrapping_add(1), true);
            }
            Inst::Sop2 { .. }
            | Inst::Sopk { .. }
            | Inst::Sop1 { .. }
            | Inst::Sopc { .. }
            | Inst::Sopp { .. }
            | Inst::Smem { .. } => {}
            _ => break,
        }
    }
    found
}
