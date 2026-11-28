//! The memory a mesh group's threads share, in a program run a thread at a time: before a
//! thread's own run, the entry point runs every thread of the group up to its first barrier
//! into a copy of that memory, and the thread's own run reads the copy so filled.

use std::collections::BTreeSet;
use std::num::NonZeroU32;

use naga::{
    BinaryOperator as Bo, Block, Expression as E, Handle, Literal, MathFunction as Mf, Scalar,
    Span, Statement, TypeInner,
};

use super::{form, insert, refused, Builder, Cx, Done, Inst, ENDED, ENDED_RETURN, MESH_SHARED};
use crate::dxil::ShaderKind;
use crate::rdna::cfg::{Cfg, Terminator};

/// Why a mesh program whose threads share memory is not translated where a store comes after
/// the group's first barrier or a read before it.
pub(super) const MESH_PHASES: &str = "the memory a group's threads share, stored after the \
    group's first barrier or read before it: a thread run alone finds the group's memory by \
    first running every thread of its group to that barrier into a copy of it, which is the \
    hardware's memory only where every store comes before the barrier and every read after it";
/// Why such a program is not translated where its threads do more than store to that memory
/// before the barrier.
pub(super) const MESH_FILL: &str = "a mesh program that shares memory and, before its first \
    barrier, reads another thread's value, exports, or stores outside its group: the runs that \
    fill the group's memory for each of its threads would need those threads' values or repeat \
    those stores";
/// The forms of that memory's instructions a mesh program is lowered with.
const MESH_FORM: &str = "a part of a dword, an update or a lane exchange of the memory a mesh \
    group shares";

/// What a lowering keeps of the leftovers it translates: a mesh group's memory, and whether a
/// pixel program that waits its turn at a pixel stores where another primitive reads.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Leftovers {
    /// The copy of the group's memory and whether the run going on fills it, where the
    /// program shares memory in the form [`phases`] takes.
    memory: Option<Handle<naga::GlobalVariable>>,
    pub(super) filling: Option<Handle<naga::GlobalVariable>>,
    dwords: u32,
    /// The threads of a mesh group, as its package states them.
    threads: u32,
    /// Why the memory a mesh group shares is refused, where it is.
    why: &'static str,
    /// Whether the program stores to or updates memory outside the invocation.
    pub(super) stores: bool,
    /// Whether it is a pixel program that turns on the order pixels at one place are shaded in.
    pub(super) ordered: bool,
}

/// Whether an instruction writes hardware register 25, which turns that order on and off.
fn orders(inst: &Inst) -> bool {
    matches!(inst, Inst::Sopk { imm, .. } if *imm & 0x3F == 25)
        && matches!(inst.name(), "s_setreg_b32" | "s_setreg_imm32_b32")
}

/// Whether an instruction reads or writes the memory a group shares, as opposed to exchanging
/// a value between lanes.
fn shares(inst: &Inst) -> bool {
    matches!(inst, Inst::Ds { .. }) && inst.name() != "ds_swizzle_b32"
}

/// Whether an instruction stores to or updates memory another invocation can read.
fn stores_out(name: &str) -> bool {
    [
        "image_store",
        "image_atomic",
        "buffer_store",
        "buffer_atomic",
        "global_store",
        "global_atomic",
        "flat_store",
        "flat_atomic",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}

/// The words a walk from `starts` comes to, each start a block and the place in it to begin;
/// with `stop`, a walk ends at an `s_barrier` and goes no further.
fn reached(cfg: &Cfg, starts: Vec<(u32, usize)>, stop: bool) -> BTreeSet<u32> {
    let (mut work, mut entered, mut words) = (starts, BTreeSet::new(), BTreeSet::new());
    while let Some((start, from)) = work.pop() {
        if from == 0 && !entered.insert(start) {
            continue;
        }
        let Some(block) = cfg.blocks.get(&start) else {
            continue;
        };
        let mut ended = false;
        for (pc, inst) in block.insts.iter().skip(from) {
            words.insert(*pc);
            if stop && inst.name() == "s_barrier" {
                ended = true;
                break;
            }
        }
        if ended {
            continue;
        }
        match block.terminator {
            Terminator::Jump(to) | Terminator::Fall(to) => work.push((to, 0)),
            Terminator::Branch { taken, fallthrough } => {
                work.push((taken, 0));
                work.push((fallthrough, 0));
            }
            Terminator::End | Terminator::Indirect => {}
        }
    }
    words
}

/// Whether a mesh program's memory is filled exactly by running each thread to the group's
/// first barrier: every store to it comes before any barrier and every read after one, and
/// nothing before the barrier needs another thread or leaves the group.
pub(super) fn phases(cfg: &Cfg) -> Result<(), &'static str> {
    let entry = cfg.blocks.keys().next().copied().unwrap_or(0);
    let before = reached(cfg, vec![(entry, 0)], true);
    let barriers: Vec<(u32, usize)> = (cfg.blocks.values())
        .flat_map(|block| {
            (block.insts.iter().enumerate())
                .filter(|(_, (_, inst))| inst.name() == "s_barrier")
                .map(move |(at, _)| (block.start, at + 1))
        })
        .collect();
    let after = reached(cfg, barriers, false);
    for (pc, inst) in cfg.blocks.values().flat_map(|block| &block.insts) {
        let name = inst.name();
        let early = before.contains(pc);
        if early && (name == "ds_swizzle_b32" || name.starts_with("exp") || stores_out(name)) {
            return Err(MESH_FILL);
        }
        if !shares(inst) {
            continue;
        }
        let read = name.starts_with("ds_read");
        if (read && early) || (!read && after.contains(pc)) {
            return Err(MESH_PHASES);
        }
    }
    Ok(())
}

impl Cx<'_> {
    /// Before a program's blocks are lowered: what its package states of a mesh group, the
    /// copy of the group's memory where its threads share some, and whether it stores
    /// anywhere another invocation reads.
    pub(super) fn plan_leftovers(&mut self, cfg: &Cfg) {
        let insts = || cfg.blocks.values().flat_map(|block| &block.insts);
        self.leftovers.stores = insts().any(|(_, inst)| stores_out(inst.name()));
        self.leftovers.ordered = self.fragment && insts().any(|(_, inst)| orders(inst));
        if self.program.kind != ShaderKind::Mesh {
            return;
        }
        let group = self.program.launch;
        self.leftovers.threads = (group.map(|g| g.threads[0]))
            .filter(|&threads| threads > 0)
            .unwrap_or(super::MESH_THREADS);
        self.leftovers.why = MESH_SHARED;
        let Some(group) = group.filter(|group| group.shared_dwords > 0) else {
            return;
        };
        if !insts().any(|(_, inst)| shares(inst)) {
            return;
        }
        if let Err(why) = phases(cfg) {
            self.leftovers.why = why;
            return;
        }
        let word = self.ty(TypeInner::Scalar(Scalar::U32));
        let size = NonZeroU32::new(group.shared_dwords).unwrap_or(NonZeroU32::MIN);
        let ty = self.ty(TypeInner::Array {
            base: word,
            size: naga::ArraySize::Constant(size),
            stride: 4,
        });
        let flag = self.ty(TypeInner::Scalar(Scalar::BOOL));
        self.leftovers.memory = Some(self.private("group_memory".to_owned(), ty));
        self.leftovers.filling = Some(self.private("filling".to_owned(), flag));
        self.leftovers.dwords = group.shared_dwords;
    }

    /// The threads of this mesh program's group.
    pub(super) fn mesh_threads(&self) -> u32 {
        match self.leftovers.threads {
            0 => super::MESH_THREADS,
            threads => threads,
        }
    }

    /// `s_barrier` in a mesh program whose group's memory is filled: a run that fills it ends here.
    pub(super) fn mesh_barrier(&mut self) -> Done {
        let Some(filling) = self.leftovers.filling else {
            return Ok(());
        };
        let pointer = self.global(filling);
        let condition = self.b.load(pointer);
        self.when(condition, |cx| {
            // A function statements were moved to says so, for the one that called it to end too.
            if cx.spread {
                let how = cx.b.u32(ENDED_RETURN);
                cx.put(ENDED, how);
            }
            cx.b.push(Statement::Return { value: None });
            Ok(())
        })
    }

    /// Before the body's statements: a run starts with no function ended, though the run
    /// before it ended at a barrier.
    pub(super) fn mesh_entry(&mut self) {
        if self.spread && self.leftovers.filling.is_some() {
            let zero = self.b.u32(0);
            self.put(ENDED, zero);
        }
    }

    /// A mesh program's instruction of the memory its group shares, on the copy: a store only
    /// in a run that fills it, a read in any run. An address past what the package states
    /// reads nought and stores nothing.
    pub(super) fn mesh_memory(&mut self, pc: u32, inst: &Inst, name: &str) -> Done {
        let Inst::Ds {
            addr,
            data0,
            data1,
            dst,
            offset0,
            offset1,
            gds,
            ..
        } = inst
        else {
            return Err(refused(pc, inst));
        };
        let (Some(memory), Some(filling)) = (self.leftovers.memory, self.leftovers.filling) else {
            return Err(form(pc, inst, self.leftovers.why));
        };
        if *gds {
            return Err(form(pc, inst, "the memory every group shares"));
        }
        let (write, unit) = match name {
            "ds_read_b32" => (false, None),
            "ds_write_b32" => (true, None),
            "ds_read2_b32" => (false, Some(4)),
            "ds_write2_b32" => (true, Some(4)),
            "ds_read2st64_b32" => (false, Some(256)),
            "ds_write2st64_b32" => (true, Some(256)),
            _ => return Err(form(pc, inst, MESH_FORM)),
        };
        let offsets = match unit {
            None => vec![u32::from(*offset0) | (u32::from(*offset1) << 8)],
            Some(unit) => vec![u32::from(*offset0) * unit, u32::from(*offset1) * unit],
        };
        let base = self.vreg(pc, *addr)?;
        let dwords = self.leftovers.dwords;
        let mut places = Vec::with_capacity(2);
        for offset in offsets {
            let (by, two) = (self.b.u32(offset), self.b.u32(2));
            let bytes = self.b.bin(Bo::Add, base, by);
            let index = self.b.bin(Bo::ShiftRight, bytes, two);
            let (count, last) = (self.b.u32(dwords), self.b.u32(dwords.saturating_sub(1)));
            let within = self.b.bin(Bo::Less, index, count);
            let held = self.b.math(Mf::Min, &[index, last]);
            let pointer = self.global(memory);
            let cell = self.b.emit(E::Access {
                base: pointer,
                index: held,
            });
            places.push((cell, within));
        }
        if write {
            let mut data = Vec::with_capacity(2);
            for register in [*data0, *data1].into_iter().take(places.len()) {
                data.push(self.vreg(pc, register)?);
            }
            let pointer = self.global(filling);
            let mut condition = self.b.load(pointer);
            if self.masked {
                let active = self.active();
                condition = self.b.bin(Bo::LogicalAnd, condition, active);
            }
            return self.when(condition, |cx| {
                for ((cell, within), value) in places.into_iter().zip(data) {
                    cx.when(within, |cx| {
                        cx.b.push(Statement::Store {
                            pointer: cell,
                            value,
                        });
                        Ok(())
                    })?;
                }
                Ok(())
            });
        }
        // Both are read before either register is written: the address may be one of them.
        let zero = self.b.u32(0);
        let mut loaded = Vec::with_capacity(2);
        for (cell, within) in places {
            let word = self.b.load(cell);
            loaded.push(self.b.select(within, word, zero));
        }
        for (k, value) in loaded.into_iter().enumerate() {
            self.vput(dst.wrapping_add(k as u8), value);
        }
        Ok(())
    }
}

/// In a mesh program's entry point, before the runs of its own thread: the body run for every
/// thread of the group, each to the group's first barrier, storing into the copy of its memory.
pub(super) fn fill(
    w: &mut Builder,
    module: &mut naga::Module,
    leftovers: &Leftovers,
    thread: Handle<naga::GlobalVariable>,
    call: Statement,
) {
    let Some(filling) = leftovers.filling else {
        return;
    };
    let word = insert(module, TypeInner::Scalar(Scalar::U32));
    let variable = naga::LocalVariable {
        name: Some("filled".to_owned()),
        ty: word,
        init: None,
    };
    let variable = w.fun.local_variables.append(variable, Span::UNDEFINED);
    let counter = w.emit(E::LocalVariable(variable));
    let zero = w.u32(0);
    let (yes, no) = (
        w.emit(E::Literal(Literal::Bool(true))),
        w.emit(E::Literal(Literal::Bool(false))),
    );
    let flag = w.emit(E::GlobalVariable(filling));
    let pointer = w.emit(E::GlobalVariable(thread));
    w.push(Statement::Store {
        pointer: counter,
        value: zero,
    });
    w.push(Statement::Store {
        pointer: flag,
        value: yes,
    });
    w.open();
    let at = w.load(counter);
    let count = w.u32(leftovers.threads.max(1));
    let done = w.bin(Bo::GreaterEqual, at, count);
    let mut leave = Block::new();
    leave.push(Statement::Break, Span::UNDEFINED);
    w.push(Statement::If {
        condition: done,
        accept: leave,
        reject: Block::new(),
    });
    w.push(Statement::Store { pointer, value: at });
    w.push(call);
    let one = w.u32(1);
    let next = w.bin(Bo::Add, at, one);
    w.push(Statement::Store {
        pointer: counter,
        value: next,
    });
    let body = w.close();
    w.push(Statement::Loop {
        body,
        continuing: Block::new(),
        break_if: None,
    });
    w.push(Statement::Store {
        pointer: flag,
        value: no,
    });
}
