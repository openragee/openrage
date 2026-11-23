//! Which `v_readfirstlane_b32` of a program read what this lane holds too, so that a lane
//! lowered alone reads the hardware's value: the source is one every lane that runs holds the
//! same value in, or the read opens a loop over the wave's distinct values, which keeps the
//! wave to the lanes that hold the value read.

use std::collections::{BTreeMap, BTreeSet};

use crate::rdna::cfg::Cfg;
use crate::rdna::decode::{Inst, Operand};
use crate::rdna::sym;

/// The vector registers every lane that runs holds one value in, a bit a register.
type Same = [u64; 4];

/// What the lanes that run agree on: the vector registers they hold one value in, and the
/// scalar registers that hold a lane mask with one bit for all of them, as a condition every
/// lane shares is kept.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Agreed {
    same: Same,
    level: u128,
}

impl Agreed {
    const NOTHING: Self = Self {
        same: [0; 4],
        level: 0,
    };
    const ALL: Self = Self {
        same: [!0; 4],
        level: !0,
    };

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

    /// Scalar register `r` was written, with such a mask or not. A mask of 64 lanes is a pair
    /// kept under its first register, so the pairs that begin beside `r` hold one no longer.
    fn wrote(&mut self, r: u8, on: bool) {
        for beside in [r.wrapping_sub(1), r, r.saturating_add(1)] {
            if beside < 128 {
                self.level &= !(1 << beside);
            }
        }
        if on && r < 128 {
            self.level |= 1 << r;
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
            agreed.set_level(dst, level);
        }
        Inst::Sop1 { dst, src, .. } => {
            // The lanes that ran, saved as the mask is changed, are a mask of them all.
            let level = (width("s_mov") || width("s_not")) && agreed.level(src);
            agreed.set_level(dst, level);
        }
        Inst::Sopk { dst, .. } => agreed.set_level(dst, false),
        // A load fills up to sixteen registers from its first.
        Inst::Smem { data, .. } => {
            if let Some(first) = sym::register(*data) {
                for r in first..first.saturating_add(16) {
                    agreed.wrote(r, false);
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
    // A result that fills part of its register, or adds to it, keeps what the rest held.
    let keeps = sources.contains(&Operand::Sdwa) || name.contains("16") || name.contains("mac");
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
        if !narrows {
            *agreed = Agreed::NOTHING;
        }
        if let Some(dst) = saved {
            agreed.set_level(&dst, narrows);
        }
        return;
    }
    scalar(agreed, inst, wave32);
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
        Inst::Vop1 { dst, src, .. } => vop(same, name, *dst, &[*src], false),
        Inst::Vop2 {
            dst, src0, src1, ..
        } => vop(same, name, *dst, &[*src0, Operand::Vgpr(*src1)], chosen),
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
        Inst::Unknown { .. } => *agreed = Agreed::NOTHING,
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
fn waterfall(rest: &[(u32, Inst)], scalar: u8, vector: u8) -> bool {
    let read = |a: &Operand, b: &Operand| {
        let pair = (Operand::Sgpr(scalar), Operand::Vgpr(vector));
        (*a, *b) == pair || (*b, *a) == pair
    };
    let equal = |name: &str| matches!(name, "v_cmp_eq_u32" | "v_cmp_eq_i32");
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
                match equal(name) && read(src0, &Operand::Vgpr(*src1)) {
                    true => alike.insert(sym::VCC),
                    false => alike.remove(&sym::VCC),
                };
            }
            Inst::Vop3 { dst, srcs, .. } if name.starts_with("v_cmp_") => {
                match equal(name) && read(&srcs[0], &srcs[1]) {
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
        (cfg.blocks.keys().map(|&start| (start, Agreed::ALL))).collect();
    entry.insert(0, Agreed::NOTHING);
    let mut work: BTreeSet<u32> = cfg.blocks.keys().copied().collect();
    while let Some(start) = work.pop_first() {
        let (Some(block), Some(mut agreed)) = (cfg.blocks.get(&start), entry.get(&start).copied())
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
            let met = Agreed {
                same: [0, 1, 2, 3].map(|k| held.same[k] & agreed.same[k]),
                level: held.level & agreed.level,
            };
            if met != *held {
                *held = met;
                work.insert(next);
            }
        }
    }
    let mut exact = BTreeSet::new();
    for (start, block) in &cfg.blocks {
        let mut agreed = entry.get(start).copied().unwrap_or(Agreed::NOTHING);
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
