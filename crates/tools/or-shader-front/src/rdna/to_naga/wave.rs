//! A compute wave kept in memory its group shares: each lane leaves its value there, the group
//! meets at a barrier, and each lane reads the others'. No subgroup of the device is asked, so
//! the wave is the program's own whatever width the device runs.
//!
//! Every thread has to reach every barrier, so every branch is voted: the lanes of the wave
//! walk the program together, as the hardware's do, each masking its own writes. The waves
//! of a group of several walk it a block of one wave at a time, the whole group in step.

use std::collections::HashMap;
use std::num::NonZeroU32;

use naga::{
    BinaryOperator as Bo, Block, Expression as E, Handle, Scalar, ScalarKind, Span, Statement,
    TypeInner,
};

use super::halves::Rounding;
use super::{fold_of, form, insert, Builder, Cx, Done, End, Inst, NotTranslated, Reg, EXEC};
use crate::dxil::ShaderKind;
use crate::rdna::cfg::{Cfg, Terminator};
use crate::rdna::decode::Operand;
use crate::rdna::sym::{self, VCC};
use crate::rdna::Program;

/// The lanes of a program's wave.
pub(super) fn width(program: &Program) -> u32 {
    if program.wave32 {
        super::WAVE / 2
    } else {
        super::WAVE
    }
}

/// The threads of each wave of a compute group and the waves it is, where shared memory can
/// stand for them: one wave, full or not, or several full ones, which take turns
/// ([`Cx::dispatch_waves`]). A group whose last wave is not full keeps the device's subgroups.
pub(super) fn waves(program: &Program) -> Option<(u32, u32)> {
    let launch = program
        .launch
        .filter(|_| program.kind == ShaderKind::Compute)?;
    let threads: u32 = launch.threads.iter().map(|&n| n.max(1)).product();
    let lanes = width(program);
    match threads <= lanes {
        true => Some((threads, 1)),
        false => threads
            .is_multiple_of(lanes)
            .then_some((lanes, threads / lanes)),
    }
}

/// `cfg` with every block cut after an `s_barrier`: a wave of several stops there until the
/// group's other waves have come as far, and a wave stops only where a block ends.
pub(super) fn split(cfg: &Cfg) -> Cfg {
    let mut blocks = std::collections::BTreeMap::new();
    for block in cfg.blocks.values() {
        let mut rest = block.clone();
        while let Some(at) = (rest.insts.iter())
            .position(|(_, inst)| inst.name() == "s_barrier")
            .filter(|at| at + 1 < rest.insts.len())
        {
            let after = rest.insts.split_off(at + 1);
            let next = after.first().map_or(rest.start, |(pc, _)| *pc);
            let first = crate::rdna::cfg::Block {
                start: rest.start,
                insts: std::mem::replace(&mut rest.insts, after),
                terminator: Terminator::Fall(next),
            };
            blocks.insert(first.start, first);
            rest.start = next;
        }
        blocks.insert(rest.start, rest);
    }
    Cfg { blocks }
}

/// Whether a wave waits for its group where `block` ends.
pub(super) fn waits(block: &crate::rdna::cfg::Block) -> bool {
    (block.insts.last()).is_some_and(|(_, inst)| inst.name() == "s_barrier")
}

/// What a wave of several leaves as the block it runs next once its program has ended, what
/// the group reads where every wave left waits at a barrier, and the bit a waiting wave sets.
const ENDED: u32 = 0x7FFF_FFFF;
const RELEASE: u32 = 0x7FFF_FFFE;
const WAITING: u32 = 0x8000_0000;

/// The shared memory a wave is exchanged through, and the functions that read it.
#[derive(Default)]
pub(super) struct Shared {
    /// The threads of the wave, where the wave is kept in shared memory at all.
    pub(super) threads: Option<u32>,
    /// The waves of the group: more than one take turns, a block at a time.
    pub(super) waves: u32,
    /// Where the waves take turns: whether this thread's wave is the one whose block runs,
    /// and whether it waits at a barrier of the program.
    turn: Option<[Handle<naga::GlobalVariable>; 2]>,
    /// Whether the statements being written are a block's own, which only the wave whose
    /// turn it is may be seen to run.
    pub(super) guarding: bool,
    /// Each lane's value and whether the lane runs, twice over: an exchange writes the half
    /// the one before it did not, which a slower lane may still be reading.
    memory: Option<[Handle<naga::GlobalVariable>; 2]>,
    /// The exchanges this thread has made: its low bit picks the half.
    round: Option<Handle<naga::GlobalVariable>>,
    functions: HashMap<String, Handle<naga::Function>>,
    /// A mesh program's reads of other threads' values: how many rounds of them each
    /// register hangs on, the instruction being lowered and the path to it; the most of any
    /// read; the patterns read by; each read's block and word; and the variables that say
    /// which pattern's thread a run is of and hold what each read found.
    rounds: HashMap<Reg, u8>,
    reading: u8,
    control: u8,
    passes: u8,
    patterns: Vec<u32>,
    /// Each read's word, the register read and the number of its pattern, in the code's order.
    planned: Vec<(u32, u8, u32)>,
    /// The scalar registers the block being lowered left a lane mask in: a comparison's
    /// result, the lanes that run, and what a move or a logic instruction makes of those.
    pub(super) masks: std::collections::BTreeSet<u8>,
    /// The block of each read that was lowered.
    sites: Vec<usize>,
    swizzle: Option<[Handle<naga::GlobalVariable>; 2]>,
}

impl Shared {
    pub(super) fn of(shape: Option<(u32, u32)>) -> Self {
        Self {
            threads: shape.map(|(threads, _)| threads),
            waves: shape.map_or(1, |(_, waves)| waves),
            ..Self::default()
        }
    }
}

/// A function's local of one dword, as the pointer to it.
fn local(f: &mut Builder, module: &mut naga::Module, name: &str) -> Handle<E> {
    let ty = insert(module, TypeInner::Scalar(Scalar::U32));
    let variable = naga::LocalVariable {
        name: Some(name.to_owned()),
        ty,
        init: None,
    };
    let variable = f.fun.local_variables.append(variable, Span::UNDEFINED);
    f.emit(E::LocalVariable(variable))
}

/// `inside` for each value of the counter from where it stands, by `step`, until `ended` says so.
fn repeat(
    f: &mut Builder,
    counter: Handle<E>,
    ended: impl FnOnce(&mut Builder, Handle<E>) -> Handle<E>,
    inside: impl FnOnce(&mut Builder, Handle<E>),
    step: impl FnOnce(&mut Builder, Handle<E>) -> Handle<E>,
) {
    f.open();
    let at = f.load(counter);
    let condition = ended(f, at);
    let mut accept = Block::new();
    accept.push(Statement::Break, Span::UNDEFINED);
    f.push(Statement::If {
        condition,
        accept,
        reject: Block::new(),
    });
    inside(f, at);
    let value = step(f, at);
    f.push(Statement::Store {
        pointer: counter,
        value,
    });
    let body = f.close();
    f.push(Statement::Loop {
        body,
        continuing: Block::new(),
        break_if: None,
    });
}

impl Cx<'_> {
    /// The wave's memory: the lanes' values, and whether each runs.
    fn wave_memory(&mut self) -> [Handle<naga::GlobalVariable>; 2] {
        if let Some(memory) = self.shared_wave.memory {
            return memory;
        }
        let base = self.ty(TypeInner::Scalar(Scalar::U32));
        let all = 2 * width(self.program) * self.shared_wave.waves;
        let size = NonZeroU32::new(all).unwrap_or(NonZeroU32::MIN);
        let ty = self.ty(TypeInner::Array {
            base,
            size: naga::ArraySize::Constant(size),
            stride: 4,
        });
        let memory = ["wave_values", "wave_active"].map(|name| {
            let variable = naga::GlobalVariable {
                name: Some(name.to_owned()),
                space: naga::AddressSpace::WorkGroup,
                binding: None,
                ty,
                init: None,
                memory_decorations: naga::MemoryDecorations::empty(),
            };
            (self.module.global_variables).append(variable, Span::UNDEFINED)
        });
        self.shared_wave.memory = Some(memory);
        memory
    }

    /// This thread's number in its group: the waves fill in that order.
    fn wave_thread(&mut self) -> Handle<E> {
        let threads = self.launch.map_or([1; 3], |launch| launch.threads);
        let [x, y, z] = [0, 1, 2].map(|axis| self.launch_number(0, axis));
        let (across, down) = (self.b.u32(threads[0].max(1)), self.b.u32(threads[1].max(1)));
        let rows = self.b.bin(Bo::Multiply, z, down);
        let rows = self.b.bin(Bo::Add, rows, y);
        let before = self.b.bin(Bo::Multiply, rows, across);
        self.b.bin(Bo::Add, before, x)
    }

    /// One axis of the number of its wave's first thread: what the first lane holds of a
    /// compute thread's number while every lane runs.
    pub(super) fn wave_first_thread(&mut self, axis: u32) -> Handle<E> {
        let threads = self
            .launch
            .map_or([1; 3], |launch| launch.threads.map(|n| n.max(1)));
        let thread = self.wave_thread();
        let rest = self.b.u32(!(width(self.program) - 1));
        let first = self.b.bin(Bo::And, thread, rest);
        let (across, down) = (self.b.u32(threads[0]), self.b.u32(threads[1]));
        match axis {
            0 => self.b.bin(Bo::Modulo, first, across),
            1 => {
                let rows = self.b.bin(Bo::Divide, first, across);
                self.b.bin(Bo::Modulo, rows, down)
            }
            _ => {
                let plane = self.b.u32(threads[0] * threads[1]);
                self.b.bin(Bo::Divide, first, plane)
            }
        }
    }

    /// This thread's lane of its wave: its number in its group, past the waves before its own.
    pub(super) fn wave_lane(&mut self) -> Handle<E> {
        let thread = self.wave_thread();
        match self.shared_wave.waves {
            1 => thread,
            _ => {
                let last = self.b.u32(width(self.program) - 1);
                self.b.bin(Bo::And, thread, last)
            }
        }
    }

    /// Whether this thread's wave is the one whose block runs, where the statements being
    /// written are a block's own and the group's waves take turns.
    pub(super) fn turn(&mut self) -> Option<Handle<E>> {
        let [here, _] = (self.shared_wave.turn).filter(|_| self.shared_wave.guarding)?;
        let pointer = self.global(here);
        Some(self.b.load(pointer))
    }

    /// Leaves this lane's `value` and whether it runs where the wave's other lanes read them,
    /// and waits for theirs: the first dword of its wave's in the half of the memory they are
    /// all in. One function of the module, since a program makes thousands of exchanges.
    fn exchange(&mut self, value: Handle<E>) -> Handle<E> {
        let runs = self.active();
        let runs = self.b.flag(runs);
        let call = self.wave_call(
            "wave_exchange".to_owned(),
            vec![value, runs],
            |cx, f, taken| {
                std::mem::swap(&mut cx.b, f);
                let base = cx.exchanged(taken[0], taken[1]);
                std::mem::swap(&mut cx.b, f);
                Some(base)
            },
        );
        match call {
            Some(base) => base,
            None => self.exchanged(value, runs),
        }
    }

    /// [`Cx::exchange`], written where the builder stands.
    fn exchanged(&mut self, value: Handle<E>, runs: Handle<E>) -> Handle<E> {
        let [values, active] = self.wave_memory();
        let round = match self.shared_wave.round {
            Some(round) => round,
            None => {
                let ty = self.ty(TypeInner::Scalar(Scalar::U32));
                let round = self.private("wave_round".to_owned(), ty);
                self.shared_wave.round = Some(round);
                round
            }
        };
        let counter = self.global(round);
        let made = self.b.load(counter);
        let all = width(self.program) * self.shared_wave.waves;
        let (one, all) = (self.b.u32(1), self.b.u32(all));
        let half = self.b.bin(Bo::And, made, one);
        let half = self.b.bin(Bo::Multiply, half, all);
        let (thread, lane) = (self.wave_thread(), self.wave_lane());
        let place = self.b.bin(Bo::Add, half, thread);
        let base = self.b.bin(Bo::Subtract, place, lane);
        for (global, value) in [(values, value), (active, runs)] {
            let memory = self.global(global);
            let pointer = self.b.emit(E::Access {
                base: memory,
                index: place,
            });
            self.b.push(Statement::Store { pointer, value });
        }
        (self.b).push(Statement::ControlBarrier(naga::Barrier::WORK_GROUP));
        let next = self.b.bin(Bo::Add, made, one);
        self.b.push(Statement::Store {
            pointer: counter,
            value: next,
        });
        base
    }

    /// Calls a function of the module's own, written on its first use: one of the wave's
    /// memory, or of dwords alone. `None` where `write` has no way to write it.
    pub(super) fn wave_call(
        &mut self,
        name: String,
        arguments: Vec<Handle<E>>,
        write: impl FnOnce(&mut Self, &mut Builder, &[Handle<E>]) -> Option<Handle<E>>,
    ) -> Option<Handle<E>> {
        let function = match self.shared_wave.functions.get(&name) {
            Some(&function) => function,
            None => {
                let word = self.ty(TypeInner::Scalar(Scalar::U32));
                let mut f = Builder::new();
                let taken: Vec<Handle<E>> = (0..arguments.len() as u32)
                    .map(|index| f.emit(E::FunctionArgument(index)))
                    .collect();
                let value = write(self, &mut f, &taken)?;
                f.push(Statement::Return { value: Some(value) });
                let mut function = f.finish();
                function.name = Some(name.clone());
                function.arguments = (0..arguments.len())
                    .map(|_| naga::FunctionArgument {
                        name: None,
                        ty: word,
                        binding: None,
                    })
                    .collect();
                function.result = Some(naga::FunctionResult {
                    ty: word,
                    binding: None,
                });
                let function = self.module.functions.append(function, Span::UNDEFINED);
                self.shared_wave.functions.insert(name, function);
                function
            }
        };
        let result = self.b.result(E::CallResult(function));
        self.b.push(Statement::Call {
            function,
            arguments,
            result: Some(result),
        });
        Some(result)
    }

    /// The pointer to the dword of a wave's memory at `place`, in a function of it.
    fn wave_word(&mut self, f: &mut Builder, which: usize, place: Handle<E>) -> Handle<E> {
        let memory = self.wave_memory()[which];
        let memory = f.emit(E::GlobalVariable(memory));
        f.emit(E::Access {
            base: memory,
            index: place,
        })
    }

    /// The fold `name`'s steps make of the lanes' values, joined as the hardware's steps join
    /// them: neighbours, then pairs of neighbours, and so on up, which is the order a sum of
    /// floats is rounded in. A lane the group has no thread for joins nothing.
    fn wave_fold(&mut self, name: &str, base: Handle<E>) -> Option<Handle<E>> {
        let (_, _, half) = fold_of(name)?;
        let (threads, lanes) = (self.shared_wave.threads?, width(self.program));
        let wide = match half {
            true => name.replace("_f16", "_f32"),
            false => name.to_owned(),
        };
        self.wave_call(format!("wave_fold_{name}"), vec![base], |cx, f, taken| {
            let word = cx.ty(TypeInner::Scalar(Scalar::U32));
            let size = NonZeroU32::new(lanes).unwrap_or(NonZeroU32::MIN);
            let ty = cx.ty(TypeInner::Array {
                base: word,
                size: naga::ArraySize::Constant(size),
                stride: 4,
            });
            let held = naga::LocalVariable {
                name: Some("held".to_owned()),
                ty,
                init: None,
            };
            let held = f.fun.local_variables.append(held, Span::UNDEFINED);
            let held = f.emit(E::LocalVariable(held));
            let (lane, apart) = (
                local(f, &mut cx.module, "lane"),
                local(f, &mut cx.module, "apart"),
            );
            let (zero, one, count, all) = (f.u32(0), f.u32(1), f.u32(threads), f.u32(lanes));
            f.push(Statement::Store {
                pointer: lane,
                value: zero,
            });
            let ended = |f: &mut Builder, at| f.bin(Bo::GreaterEqual, at, count);
            let next = |f: &mut Builder, at| f.bin(Bo::Add, at, one);
            let mut copy = |f: &mut Builder, at| {
                let place = f.bin(Bo::Add, taken[0], at);
                let from = cx.wave_word(f, 0, place);
                let value = f.load(from);
                let pointer = f.emit(E::Access {
                    base: held,
                    index: at,
                });
                f.push(Statement::Store { pointer, value });
            };
            repeat(f, lane, ended, &mut copy, next);
            f.push(Statement::Store {
                pointer: apart,
                value: one,
            });
            // Each round joins a run of lanes with the run after it; a run with no thread ends the round.
            let mut joined = true;
            let rounds = |f: &mut Builder, apart_now: Handle<E>| {
                f.push(Statement::Store {
                    pointer: lane,
                    value: zero,
                });
                let ended = |f: &mut Builder, at| {
                    let other = f.bin(Bo::Add, at, apart_now);
                    f.bin(Bo::GreaterEqual, other, count)
                };
                let join = |f: &mut Builder, at| {
                    let other = f.bin(Bo::Add, at, apart_now);
                    let (a, b) = (
                        f.emit(E::Access {
                            base: held,
                            index: at,
                        }),
                        f.emit(E::Access {
                            base: held,
                            index: other,
                        }),
                    );
                    let (mut x, mut y) = (f.load(a), f.load(b));
                    if half {
                        (x, y) = (f.float_of_half(x), f.float_of_half(y));
                    }
                    let value = match f.alu(&wide, &[x, y]) {
                        Some(value) if half => f.half_of(value, Rounding::Nearest),
                        Some(value) => value,
                        None => {
                            joined = false;
                            x
                        }
                    };
                    f.push(Statement::Store { pointer: a, value });
                };
                let step = |f: &mut Builder, at| {
                    let twice = f.bin(Bo::Add, apart_now, apart_now);
                    f.bin(Bo::Add, at, twice)
                };
                repeat(f, lane, ended, join, step);
            };
            let ended = |f: &mut Builder, at| f.bin(Bo::GreaterEqual, at, all);
            let twice = |f: &mut Builder, at| f.bin(Bo::Add, at, at);
            repeat(f, apart, ended, rounds, twice);
            let first = f.emit(E::AccessIndex {
                base: held,
                index: 0,
            });
            let value = f.load(first);
            joined.then_some(value)
        })
    }

    /// The sum of the values of the `count` lanes from the first.
    fn wave_sum(&mut self, base: Handle<E>, count: Handle<E>) -> Option<Handle<E>> {
        self.wave_call("wave_sum".to_owned(), vec![base, count], |cx, f, taken| {
            let (lane, sum) = (
                local(f, &mut cx.module, "lane"),
                local(f, &mut cx.module, "sum"),
            );
            let (zero, one) = (f.u32(0), f.u32(1));
            for pointer in [lane, sum] {
                f.push(Statement::Store {
                    pointer,
                    value: zero,
                });
            }
            let ended = |f: &mut Builder, at| f.bin(Bo::GreaterEqual, at, taken[1]);
            let add = |f: &mut Builder, at| {
                let place = f.bin(Bo::Add, taken[0], at);
                let from = cx.wave_word(f, 0, place);
                let (value, so_far) = (f.load(from), f.load(sum));
                let value = f.bin(Bo::Add, so_far, value);
                f.push(Statement::Store {
                    pointer: sum,
                    value,
                });
            };
            repeat(f, lane, ended, add, |f, at| f.bin(Bo::Add, at, one));
            Some(f.load(sum))
        })
    }

    /// The low bits of the values of the 32 lanes from `first`, a bit a lane.
    fn wave_mask(&mut self, base: Handle<E>, first: u32) -> Option<Handle<E>> {
        let threads = self.shared_wave.threads?;
        let first = self.b.u32(first);
        self.wave_call("wave_mask".to_owned(), vec![base, first], |cx, f, taken| {
            let (lane, mask) = (
                local(f, &mut cx.module, "lane"),
                local(f, &mut cx.module, "mask"),
            );
            let (zero, one, word, count) = (f.u32(0), f.u32(1), f.u32(32), f.u32(threads));
            for pointer in [lane, mask] {
                f.push(Statement::Store {
                    pointer,
                    value: zero,
                });
            }
            let ended = |f: &mut Builder, at| {
                let whole = f.bin(Bo::GreaterEqual, at, word);
                let lane = f.bin(Bo::Add, taken[1], at);
                let none = f.bin(Bo::GreaterEqual, lane, count);
                f.bin(Bo::LogicalOr, whole, none)
            };
            let set = |f: &mut Builder, at| {
                let place = f.bin(Bo::Add, taken[0], taken[1]);
                let place = f.bin(Bo::Add, place, at);
                let from = cx.wave_word(f, 0, place);
                let (value, so_far) = (f.load(from), f.load(mask));
                let bit = f.bin(Bo::And, value, one);
                let bit = f.bin(Bo::ShiftLeft, bit, at);
                let value = f.bin(Bo::InclusiveOr, so_far, bit);
                f.push(Statement::Store {
                    pointer: mask,
                    value,
                });
            };
            repeat(f, lane, ended, set, |f, at| f.bin(Bo::Add, at, one));
            Some(f.load(mask))
        })
    }

    /// The value of the first lane that runs, or of the first lane where none does.
    fn wave_first(&mut self, base: Handle<E>) -> Option<Handle<E>> {
        let threads = self.shared_wave.threads?;
        self.wave_call("wave_first".to_owned(), vec![base], |cx, f, taken| {
            let lane = local(f, &mut cx.module, "lane");
            let (zero, one, count) = (f.u32(0), f.u32(1), f.u32(threads));
            f.push(Statement::Store {
                pointer: lane,
                value: zero,
            });
            // The loop ends at the first lane that runs, or past the last: the first then.
            let ended = |f: &mut Builder, at| {
                let none = f.bin(Bo::GreaterEqual, at, count);
                let last = f.bin(Bo::Subtract, count, one);
                let within = f.math(naga::MathFunction::Min, &[at, last]);
                let place = f.bin(Bo::Add, taken[0], within);
                let from = cx.wave_word(f, 1, place);
                let runs = f.load(from);
                let runs = f.ne0(runs);
                f.bin(Bo::LogicalOr, none, runs)
            };
            repeat(f, lane, ended, |_, _| {}, |f, at| f.bin(Bo::Add, at, one));
            let found = f.load(lane);
            let none = f.bin(Bo::GreaterEqual, found, count);
            let found = f.select(none, zero, found);
            let place = f.bin(Bo::Add, taken[0], found);
            let from = cx.wave_word(f, 0, place);
            Some(f.load(from))
        })
    }

    /// [`Cx::collective`] over the wave in shared memory.
    pub(super) fn shared_fold(
        &mut self,
        name: &str,
        how: naga::CollectiveOperation,
        bits: Handle<E>,
    ) -> Option<Handle<E>> {
        use naga::CollectiveOperation as How;
        let (_, kind, _) = fold_of(name)?;
        if how != How::Reduce && kind != ScalarKind::Uint {
            return None;
        }
        let base = self.exchange(bits);
        match how {
            How::Reduce => self.wave_fold(name, base),
            How::InclusiveScan | How::ExclusiveScan => {
                let mut count = self.wave_lane();
                if how == How::InclusiveScan {
                    let one = self.b.u32(1);
                    count = self.b.bin(Bo::Add, count, one);
                }
                self.wave_sum(base, count)
            }
        }
    }

    /// [`Cx::ballot`] over the wave in shared memory: the two dwords of the lanes' own bits.
    pub(super) fn shared_ballot(&mut self, bit: Handle<E>) -> Option<[Handle<E>; 2]> {
        let base = self.exchange(bit);
        Some([self.wave_mask(base, 0)?, self.wave_mask(base, 32)?])
    }

    /// [`Cx::gather`] over the wave in shared memory: `bits` as the lane numbered `index` holds it.
    pub(super) fn shared_gather(&mut self, bits: Handle<E>, index: Handle<E>) -> Handle<E> {
        let base = self.exchange(bits);
        let last = self.b.u32(width(self.program) - 1);
        let lane = self.b.bin(Bo::And, index, last);
        let place = self.b.bin(Bo::Add, base, lane);
        let memory = self.wave_memory()[0];
        let memory = self.global(memory);
        let pointer = self.b.emit(E::Access {
            base: memory,
            index: place,
        });
        self.b.load(pointer)
    }

    /// `bits` as the first lane that runs holds it: what `v_readfirstlane_b32` reads.
    pub(super) fn shared_first(&mut self, bits: Handle<E>) -> Option<Handle<E>> {
        let base = self.exchange(bits);
        self.wave_first(base)
    }

    /// Whether `condition` holds in any lane of the wave: what a branch of the wave is taken
    /// on. Outside a wave kept in shared memory it is this lane's own.
    pub(super) fn voted(&mut self, condition: Handle<E>) -> Handle<E> {
        if self.shared_wave.threads.is_none() {
            return condition;
        }
        let bit = self.b.flag(condition);
        let base = self.exchange(bit);
        match self.wave_fold("v_or_b32", base) {
            Some(any) => self.b.ne0(any),
            None => condition,
        }
    }
}

/// The most rounds of other threads' values a mesh thread's value may hang on.
const SWIZZLE_ROUNDS: u8 = 3;

/// What stops a mesh thread reading another's value: the place is in a loop, so a run of the
/// other thread comes to it more than once and does not say what it held there each time.
const SWIZZLE_PLACE: &str = "another thread's value at a place the program comes to more than once";

impl Shared {
    /// The reads of other threads' values a mesh program makes, found before any is lowered:
    /// a block that jumps past one has to leave what its thread holds there.
    pub(super) fn plan(&mut self, cfg: &Cfg) {
        for (pc, inst) in cfg.blocks.values().flat_map(|block| &block.insts) {
            let Inst::Ds {
                addr,
                offset0,
                offset1,
                ..
            } = inst
            else {
                continue;
            };
            if inst.name() != "ds_swizzle_b32" || offset1 & 0x80 != 0 {
                continue;
            }
            let pattern = u32::from(*offset0) | (u32::from(*offset1) << 8);
            let which = match self.patterns.iter().position(|&known| known == pattern) {
                Some(which) => which,
                None => {
                    self.patterns.push(pattern);
                    self.patterns.len() - 1
                }
            };
            self.planned.push((*pc, *addr, which as u32));
        }
        self.planned.sort_unstable();
    }

    /// Before an instruction: nothing it reads hangs on another thread yet.impl Shared {
    /// Before an instruction: nothing it reads hangs on another thread yet.
    pub(super) fn begin(&mut self) {
        self.reading = 0;
    }

    pub(super) fn read(&mut self, reg: Reg) {
        if let Some(&rounds) = self.rounds.get(&reg) {
            self.reading = self.reading.max(rounds);
        }
    }

    /// A register written from what the instruction read, on the path the program took.
    pub(super) fn wrote(&mut self, reg: Reg) {
        if let Reg::S(r) = reg {
            self.masks.remove(&r);
        }
        let rounds = self.reading.max(self.control);
        if rounds == 0 {
            return;
        }
        let held = self.rounds.entry(reg).or_default();
        *held = (*held).max(rounds);
        if matches!(reg, Reg::Cond(_)) || reg == Reg::S(EXEC) || reg == Reg::S(EXEC + 1) {
            self.control = self.control.max(rounds);
        }
    }

    /// The runs of the body that find what the other threads hold, before a run of `thread`:
    /// each pattern's thread in turn, as many times over as a value hangs on rounds of others.
    pub(super) fn swizzle_runs(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        let each =
            (self.patterns.iter().enumerate()).map(|(which, &pattern)| (which as u32, pattern));
        (0..self.passes).flat_map(move |_| each.clone())
    }

    /// The variable that says which pattern's thread a run is of, and the number that says none.
    pub(super) fn swizzle_run(&self) -> Option<(Handle<naga::GlobalVariable>, u32)> {
        Some((self.swizzle?[0], self.patterns.len() as u32))
    }
}

/// The thread whose value `ds_swizzle_b32` by bits gives `thread`: its lane of its 32 masked,
/// then set, then flipped by the pattern's three five-bit fields.
pub(super) fn swizzled(f: &mut Builder, thread: Handle<E>, pattern: u32) -> Handle<E> {
    let (and, or, xor) = (
        f.u32(pattern & 0x1F),
        f.u32((pattern >> 5) & 0x1F),
        f.u32((pattern >> 10) & 0x1F),
    );
    let (low, rest) = (f.u32(31), f.u32(!31));
    let within = f.bin(Bo::And, thread, low);
    let kept = f.bin(Bo::And, within, and);
    let set = f.bin(Bo::InclusiveOr, kept, or);
    let flipped = f.bin(Bo::ExclusiveOr, set, xor);
    let base = f.bin(Bo::And, thread, rest);
    f.bin(Bo::InclusiveOr, base, flipped)
}

/// Whether a walk of the graph from `from` comes to a block `found` accepts.
fn comes_to(successors: &[Vec<usize>], from: &[usize], found: impl Fn(usize) -> bool) -> bool {
    let (mut work, mut seen) = (from.to_vec(), std::collections::BTreeSet::new());
    while let Some(at) = work.pop() {
        if !seen.insert(at) {
            continue;
        }
        if found(at) {
            return true;
        }
        work.extend(successors.get(at).into_iter().flatten());
    }
    false
}

impl Cx<'_> {
    /// The variables of a mesh program's reads of other threads: which pattern's thread a run
    /// is of, and what each read found.
    fn swizzle_globals(&mut self) -> [Handle<naga::GlobalVariable>; 2] {
        if let Some(globals) = self.shared_wave.swizzle {
            return globals;
        }
        let ty = self.ty(TypeInner::Scalar(Scalar::U32));
        let globals = [
            self.private("swizzle_run".to_owned(), ty),
            self.private("swizzle_held".to_owned(), ty),
        ];
        self.shared_wave.swizzle = Some(globals);
        globals
    }

    /// In the run for the thread a read's pattern names, leaves `own` where that read finds
    /// it: whether this is that run, and the pointer to the place.
    fn swizzle_leave(&mut self, site: usize, own: Handle<E>) -> (Handle<E>, Handle<E>) {
        let which = self.shared_wave.planned.get(site).map_or(0, |read| read.2);
        let [run, held] = self.swizzle_globals();
        let pointer = self.global(run);
        let (run, which) = (self.b.load(pointer), self.b.u32(which));
        let mine = self.b.bin(Bo::Equal, run, which);
        let memory = self.global(held);
        let slot = self.b.emit(E::AccessIndex {
            base: memory,
            index: site as u32,
        });
        let mut accept = Block::new();
        accept.push(
            Statement::Store {
                pointer: slot,
                value: own,
            },
            Span::UNDEFINED,
        );
        self.b.push(Statement::If {
            condition: mine,
            accept,
            reject: Block::new(),
        });
        (mine, slot)
    }

    /// Where a block ends on a jump forward from `last` to `target`: a thread that jumps past
    /// a read holds, where the wave comes to that read, what it holds now.
    pub(super) fn swizzles_passed(&mut self, last: u32, target: u32) {
        let passed: Vec<(usize, u8)> = (self.shared_wave.planned.iter().enumerate())
            .filter(|(_, read)| last < read.0 && read.0 < target)
            .map(|(site, read)| (site, read.1))
            .collect();
        for (site, source) in passed {
            let own = self.raw(Reg::V(source));
            self.swizzle_leave(site, own);
        }
    }

    /// `ds_swizzle_b32` by bits in a mesh program, which runs here a thread at a time: the
    /// other thread's value is what a run of the body for that thread leaves at this place.
    /// The entry point makes those runs first; each leaves its own value here, and reads the
    /// values the runs before it left.
    pub(super) fn mesh_swizzle(&mut self, pc: u32, inst: &Inst, dst: u8, source: u8) -> Done {
        let own = self.vreg(pc, source)?;
        let rounds = 1 + self.shared_wave.reading.max(self.shared_wave.control);
        if rounds > SWIZZLE_ROUNDS {
            let what =
                "another thread's value that hangs on more rounds of other threads' than are run";
            return Err(form(pc, inst, what));
        }
        let planned = &self.shared_wave.planned;
        let Ok(site) = planned.binary_search_by_key(&pc, |read| read.0) else {
            return Err(form(
                pc,
                inst,
                "another thread's value by a pattern no plan holds",
            ));
        };
        let wave = &mut self.shared_wave;
        wave.passes = wave.passes.max(rounds);
        wave.sites.push(self.index as usize);
        let (mine, slot) = self.swizzle_leave(site, own);
        let left = self.b.load(slot);
        let value = self.b.select(mine, own, left);
        self.shared_wave.reading = rounds;
        self.vput(dst, value);
        Ok(())
    }

    /// Once every block is lowered: the places a mesh thread reads another's value have to be
    /// ones every thread's run comes to, once, and the values kept there get their count.
    pub(super) fn settle_swizzles(&mut self, successors: &[Vec<usize>]) -> Done {
        let Some([_, held]) = self.shared_wave.swizzle else {
            return Ok(());
        };
        for &block in &self.shared_wave.sites {
            let after = successors.get(block).map_or(&[][..], Vec::as_slice);
            if comes_to(successors, after, |at| at == block) {
                return Err(NotTranslated::Pipeline(SWIZZLE_PLACE));
            }
        }
        let word = self.ty(TypeInner::Scalar(Scalar::U32));
        let count = self.shared_wave.planned.len() as u32;
        let count = NonZeroU32::new(count).unwrap_or(NonZeroU32::MIN);
        let ty = self.ty(TypeInner::Array {
            base: word,
            size: naga::ArraySize::Constant(count),
            stride: 4,
        });
        self.module.global_variables[held].ty = ty;
        Ok(())
    }
}

impl Cx<'_> {
    /// What a block that ends on a jump forward leaves for the reads it jumps past.
    pub(super) fn swizzles_past(&mut self, block: &crate::rdna::cfg::Block) {
        let Some(&(last, _)) = block.insts.last() else {
            return;
        };
        let (Terminator::Jump(target) | Terminator::Branch { taken: target, .. }) =
            block.terminator
        else {
            return;
        };
        if !self.shared_wave.planned.is_empty() {
            self.swizzles_passed(last, target);
        }
    }
}

impl Cx<'_> {
    /// Whether a scalar operand holds a lane mask, so far as this block says: the lanes that
    /// run, a comparison's result, or what the block made of those.
    pub(super) fn holds_mask(&self, operand: Operand) -> bool {
        sym::register(operand)
            .is_some_and(|r| r == EXEC || r == VCC || self.shared_wave.masks.contains(&r))
    }

    /// Whether a scalar instruction of the wave's own width leaves a lane mask: a move, a
    /// complement or a logic instruction of one, or the lanes that ran before it changed them.
    pub(super) fn makes_mask(&self, name: &str, sources: &[Operand]) -> bool {
        let width = if self.program.wave32 { "_b32" } else { "_b64" };
        let Some(kind) = (name.strip_prefix("s_")).and_then(|rest| rest.strip_suffix(width)) else {
            return false;
        };
        let known = matches!(
            kind,
            "mov"
                | "not"
                | "wqm"
                | "and"
                | "or"
                | "xor"
                | "andn2"
                | "orn2"
                | "nand"
                | "nor"
                | "xnor"
        );
        kind.ends_with("_saveexec") || (known && sources.iter().any(|&s| self.holds_mask(s)))
    }

    /// Marks what a scalar instruction wrote as a lane mask, where it made one.
    pub(super) fn keep_mask(&mut self, dst: Operand, made: bool, done: Done) -> Done {
        if let (true, Ok(()), Some(r)) = (made, &done, sym::register(dst)) {
            self.shared_wave.masks.insert(r);
        }
        done
    }
}

impl Cx<'_> {
    /// Before a program's blocks are lowered: where its group's waves take turns, the
    /// variables that say whose turn it is, and that the statements written from here are
    /// the blocks' own.
    pub(super) fn take_turns(&mut self, turns: bool) {
        if !turns {
            return;
        }
        let flag = self.ty(TypeInner::Scalar(Scalar::BOOL));
        let word = self.ty(TypeInner::Scalar(Scalar::U32));
        self.shared_wave.turn = Some([
            self.private("wave_turn".to_owned(), flag),
            self.private("wave_waits".to_owned(), word),
        ]);
        self.shared_wave.guarding = true;
    }

    /// What the group does next, read by every thread alike where each wave left the block
    /// it runs next: the least block a wave that does not wait is at, or that every wave has
    /// ended, or that those left all wait for the group and go on together.
    fn wave_next(&mut self, group: Handle<E>) -> Option<Handle<E>> {
        let (lanes, waves) = (width(self.program), self.shared_wave.waves);
        self.wave_call("wave_next".to_owned(), vec![group], |cx, f, taken| {
            let (wave, least, waiting) = (
                local(f, &mut cx.module, "wave"),
                local(f, &mut cx.module, "least"),
                local(f, &mut cx.module, "waiting"),
            );
            let (zero, one, count) = (f.u32(0), f.u32(1), f.u32(waves));
            let (ended, bit, lanes) = (f.u32(ENDED), f.u32(WAITING), f.u32(lanes));
            for (pointer, value) in [(wave, zero), (least, ended), (waiting, zero)] {
                f.push(Statement::Store { pointer, value });
            }
            let last = |f: &mut Builder, at| f.bin(Bo::GreaterEqual, at, count);
            let look = |f: &mut Builder, at| {
                let first = f.bin(Bo::Multiply, at, lanes);
                let place = f.bin(Bo::Add, taken[0], first);
                let from = cx.wave_word(f, 0, place);
                let (stands, so_far, held) = (f.load(from), f.load(least), f.load(waiting));
                let waits = f.bin(Bo::And, stands, bit);
                let waits = f.ne0(waits);
                let less = f.math(naga::MathFunction::Min, &[so_far, stands]);
                let value = f.select(waits, so_far, less);
                f.push(Statement::Store {
                    pointer: least,
                    value,
                });
                let flag = f.flag(waits);
                let value = f.bin(Bo::InclusiveOr, held, flag);
                f.push(Statement::Store {
                    pointer: waiting,
                    value,
                });
            };
            repeat(f, wave, last, look, |f, at| f.bin(Bo::Add, at, one));
            let (least, waiting) = (f.load(least), f.load(waiting));
            let none = f.bin(Bo::Equal, least, ended);
            let any = f.ne0(waiting);
            let release = f.u32(RELEASE);
            let left = f.select(any, release, ended);
            Some(f.select(none, left, least))
        })
    }

    /// The blocks of a group of several waves as one loop the whole group walks in step, a
    /// block of one wave a round: every thread tells the group the block its wave runs next,
    /// all take the least, and the threads of the waves that are elsewhere walk it for its
    /// barriers alone, their lanes off and their registers kept. A wave at `s_barrier` is
    /// passed over until every other wave waits or has ended. So a wave is the program's own
    /// on a device of any width, and two waves never run a block at once: the order of the
    /// waves between two barriers of the program is this loop's, where the hardware has none.
    pub(super) fn dispatch_waves(
        &mut self,
        ends: &[End],
        waits: &[bool],
        blocks: Vec<Block>,
    ) -> Done {
        let Some([turn, waiting]) = self.shared_wave.turn else {
            return Ok(());
        };
        let next = self.local(Reg::Next);
        let entry = self.b.u32(0);
        self.b.push(Statement::Store {
            pointer: next,
            value: entry,
        });
        self.b.open();
        let stands = self.b.load(next);
        let pointer = self.global(waiting);
        let held = self.b.load(pointer);
        let waits_now = self.b.ne0(held);
        let bit = self.b.u32(WAITING);
        let flagged = self.b.bin(Bo::InclusiveOr, stands, bit);
        let word = self.b.select(waits_now, flagged, stands);
        let base = self.exchange(word);
        let (thread, lane) = (self.wave_thread(), self.wave_lane());
        let before = self.b.bin(Bo::Subtract, thread, lane);
        let group = self.b.bin(Bo::Subtract, base, before);
        let chosen = (self.wave_next(group)).ok_or_else(|| {
            NotTranslated::Structure("the waves of a group have no turn to take".to_owned())
        })?;
        let ended = self.b.u32(ENDED);
        let done = self.b.bin(Bo::Equal, chosen, ended);
        self.when(done, |cx| {
            cx.b.push(Statement::Return { value: None });
            Ok(())
        })?;
        let release = self.b.u32(RELEASE);
        let released = self.b.bin(Bo::Equal, chosen, release);
        self.when(released, |cx| {
            let (pointer, zero) = (cx.global(waiting), cx.b.u32(0));
            cx.b.push(Statement::Store {
                pointer,
                value: zero,
            });
            cx.b.push(Statement::Continue);
            Ok(())
        })?;
        let mine = self.b.bin(Bo::Equal, stands, chosen);
        let idle = self.b.lnot(waits_now);
        let here = self.b.bin(Bo::LogicalAnd, mine, idle);
        let pointer = self.global(turn);
        self.b.push(Statement::Store {
            pointer,
            value: here,
        });
        self.shared_wave.guarding = true;
        let mut cases = Vec::with_capacity(blocks.len() + 1);
        for (index, statements) in blocks.into_iter().enumerate() {
            self.b.open();
            self.b.push(Statement::Block(statements));
            match ends.get(index) {
                Some(&End::Jump(to)) => {
                    let to = self.b.u32(to);
                    self.put(Reg::Next, to);
                }
                Some(&End::Branch { then, otherwise }) => {
                    let condition = self.raw(Reg::Cond(index as u32));
                    let (then, otherwise) = (self.b.u32(then), self.b.u32(otherwise));
                    self.put(Reg::Next, otherwise);
                    self.when(condition, |cx| {
                        cx.put(Reg::Next, then);
                        Ok(())
                    })?;
                }
                Some(End::Return) | None => {
                    let ended = self.b.u32(ENDED);
                    self.put(Reg::Next, ended);
                }
            }
            if waits.get(index) == Some(&true) {
                let (pointer, one) = (self.global(waiting), self.b.u32(1));
                let old = self.b.load(pointer);
                let value = self.b.select(here, one, old);
                self.b.push(Statement::Store { pointer, value });
            }
            cases.push(naga::SwitchCase {
                value: naga::SwitchValue::U32(index as u32),
                body: self.b.close(),
                fall_through: false,
            });
        }
        self.shared_wave.guarding = false;
        cases.push(naga::SwitchCase {
            value: naga::SwitchValue::Default,
            body: Block::new(),
            fall_through: false,
        });
        self.b.push(Statement::Switch {
            selector: chosen,
            cases,
        });
        let body = self.b.close();
        self.b.push(Statement::Loop {
            body,
            continuing: Block::new(),
            break_if: None,
        });
        Ok(())
    }
}
