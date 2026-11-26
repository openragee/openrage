//! Dead stores and dead registers. A store to a register is dropped where no path from it
//! reaches a load of that register before another store to it, and an `if` whose arms are left
//! with nothing but expressions goes with them. A store of an argument a stage takes at a
//! location stays whoever reads it. A variable of a thread's own that nothing names any more is
//! dropped from the module.
//!
//! What is live is walked backwards through the statements. A register is live where a function
//! returns if it is live after some call to that function, so a function is rewritten after
//! every function that calls it, the entry points first; what a call reads and what it lets
//! through are the callee's own walk, made before its callers'. Where an entry point returns,
//! only the registers a reader outside the module was promised are live.

use naga::{Arena, Block, Expression as E, Function, Handle, Statement as S};

use super::Registers;

/// A set of the variables a function sees, by place: the globals, then its locals.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Live(Vec<u64>);

impl Live {
    fn none(count: usize) -> Self {
        Self(vec![0; count.div_ceil(64)])
    }

    fn has(&self, var: usize) -> bool {
        self.0[var / 64] >> (var % 64) & 1 != 0
    }

    fn set(&mut self, var: usize) {
        self.0[var / 64] |= 1 << (var % 64);
    }

    fn clear(&mut self, var: usize) {
        self.0[var / 64] &= !(1 << (var % 64));
    }

    fn join(&mut self, other: &Self) {
        for (word, other) in self.0.iter_mut().zip(&other.0) {
            *word |= other;
        }
    }
}

/// What a call does to the globals live after it, a bit a global: those the function may load
/// before it stores to them, and those it does not store to on every path through it. The bits
/// past the last global are clear in the first and set in the second, so a caller's locals
/// pass a call as they are.
type Call = [Vec<u64>; 2];

struct Walk<'a> {
    registers: &'a Registers,
    function: usize,
    expressions: &'a Arena<E>,
    /// By function declared before this one.
    calls: &'a [Call],
    /// By function: the globals live after some call to it that a rewrite has met.
    after: &'a mut [Vec<u64>],
    arguments: &'a [naga::FunctionArgument],
    /// Live where the function returns.
    end: Live,
    /// Whether what is dead is dropped as it is met, or only what is live is found.
    drops: bool,
    /// Stores dropped, and `if`s.
    dropped: [usize; 2],
}

impl Walk<'_> {
    fn var(&self, pointer: Handle<E>) -> Option<usize> {
        (self.registers).var(self.function, self.expressions, pointer)
    }

    /// Whether `value` is an argument a stage takes at a location.
    fn crosses(&self, value: Handle<E>) -> bool {
        let E::FunctionArgument(index) = self.expressions[value] else {
            return false;
        };
        let binding = &self.arguments[index as usize].binding;
        matches!(binding, Some(naga::Binding::Location { .. }))
    }

    /// One turn of a loop, with `head` live where the next turn starts and `after` where the
    /// loop is left; what is live where the turn starts.
    fn turn(
        &mut self,
        [body, continuing]: [&mut Block; 2],
        after: &Live,
        head: Live,
        leaves: bool,
    ) -> Live {
        let mut end = head;
        if leaves {
            end.join(after);
        }
        let next = self.block(continuing, end, [after, after]);
        self.block(body, next.clone(), [after, &next])
    }

    /// Walks `block` from its end, where `live` is live; what is live where it starts. `around`
    /// is what is live where a `break` and where a `continue` under it lead.
    fn block(&mut self, block: &mut Block, mut live: Live, around: [&Live; 2]) -> Live {
        let mut gone = false;
        for statement in block.iter_mut().rev() {
            match statement {
                S::Emit(range) => {
                    for at in range.clone() {
                        if let E::Load { pointer } = self.expressions[at] {
                            if let Some(var) = self.var(pointer) {
                                live.set(var);
                            }
                        }
                    }
                }
                S::Store { pointer, value } => match self.var(*pointer) {
                    Some(var) if live.has(var) => live.clear(var),
                    // The join of two stages finds an input by the store of its argument,
                    // and is what drops one nothing reads.
                    Some(_) if self.crosses(*value) => {}
                    Some(_) if self.drops => {
                        *statement = S::Block(Block::new());
                        self.dropped[0] += 1;
                        gone = true;
                    }
                    _ => {}
                },
                S::Block(inner) => live = self.block(inner, live, around),
                S::If { accept, reject, .. } => {
                    let taken = self.block(accept, live.clone(), around);
                    live = self.block(reject, live, around);
                    live.join(&taken);
                    // An expression is in scope only as far as its block, and computing one
                    // changes nothing.
                    let idle = |arm: &Block| arm.iter().all(|inner| matches!(inner, S::Emit(_)));
                    if self.drops && idle(accept) && idle(reject) {
                        *statement = S::Block(Block::new());
                        self.dropped[1] += 1;
                        gone = true;
                    }
                }
                S::Switch { cases, .. } => {
                    // A `break` under a case leaves the `switch`, and a case that falls through
                    // runs the one after it. No case need be taken.
                    let after = live;
                    let (mut any, mut below) = (after.clone(), after.clone());
                    for case in cases.iter_mut().rev() {
                        let end = match case.fall_through {
                            true => below,
                            false => after.clone(),
                        };
                        below = self.block(&mut case.body, end, [&after, around[1]]);
                        any.join(&below);
                    }
                    live = any;
                }
                S::Loop {
                    body,
                    continuing,
                    break_if,
                } => {
                    let (after, leaves) = (live, break_if.is_some());
                    // A register live where a turn ends is live where it starts or is not,
                    // whatever else is: so what a turn starts with, found with nothing live
                    // where it ends, is what every turn starts with.
                    let drops = std::mem::replace(&mut self.drops, false);
                    let none = Live(vec![0; after.0.len()]);
                    let head = self.turn([&mut *body, &mut *continuing], &after, none, leaves);
                    self.drops = drops;
                    if drops {
                        let again = self.turn([body, continuing], &after, head.clone(), leaves);
                        debug_assert_eq!(again, head);
                    }
                    live = head;
                }
                S::Break => live = around[0].clone(),
                S::Continue => live = around[1].clone(),
                S::Return { .. } => live = self.end.clone(),
                S::Call { function, .. } => {
                    let called = function.index();
                    if self.drops {
                        for (after, live) in self.after[called].iter_mut().zip(&live.0) {
                            *after |= live;
                        }
                    }
                    let [reads, passes] = &self.calls[called];
                    for ((live, reads), passes) in live.0.iter_mut().zip(reads).zip(passes) {
                        *live = reads | *live & passes;
                    }
                }
                _ => {}
            }
        }
        if gone {
            let mut out = Block::with_capacity(block.len());
            for (statement, span) in std::mem::take(block).span_into_iter() {
                if !matches!(&statement, S::Block(inner) if inner.is_empty()) {
                    out.push(statement, span);
                }
            }
            *block = out;
        }
        live
    }
}

/// Drops the stores to a register of `module` that nothing reads, and the `if`s left with
/// nothing to do; how many of each. A register `keep` names is read where its function returns.
pub(super) fn stores(
    module: &mut naga::Module,
    registers: &Registers,
    keep: &[&str],
) -> [usize; 2] {
    let kept = |name: &Option<String>| keep.contains(&name.as_deref().unwrap_or(""));
    let globals = registers.globals.len();
    let words = globals.div_ceil(64);
    // The bits of the last word of globals that are globals.
    let last = match globals % 64 {
        0 => u64::MAX,
        bits => (1 << bits) - 1,
    };
    let of_globals = |live: &Live, past: u64| -> Vec<u64> {
        let mut bits = live.0[..words].to_vec();
        if let Some(word) = bits.last_mut() {
            *word = *word & last | past & !last;
        }
        bits
    };
    let kept_globals: Vec<usize> = (module.global_variables.iter())
        .filter(|(_, global)| kept(&global.name))
        .map(|(global, _)| global.index())
        .collect();
    let entries_from = module.functions.len();
    let mut functions: Vec<&mut Function> = super::functions_mut(module).collect();
    let mut after = vec![vec![0; words]; functions.len()];
    let mut calls: Vec<Call> = Vec::with_capacity(functions.len());
    for (index, function) in functions.iter_mut().enumerate() {
        let count = registers.count(index);
        let mut walk = Walk {
            registers,
            function: index,
            expressions: &function.expressions,
            calls: &calls,
            after: &mut after,
            arguments: &function.arguments,
            end: Live::none(count),
            drops: false,
            dropped: [0; 2],
        };
        let none = Live::none(count);
        let reads = walk.block(&mut function.body, none.clone(), [&none, &none]);
        let mut all = Live::none(count);
        (0..globals).for_each(|global| all.set(global));
        walk.end = all.clone();
        let passes = walk.block(&mut function.body, all, [&none, &none]);
        calls.push([of_globals(&reads, 0), of_globals(&passes, u64::MAX)]);
    }
    let mut dropped = [0; 2];
    for (index, function) in functions.iter_mut().enumerate().rev() {
        let count = registers.count(index);
        let mut end = Live::none(count);
        match index < entries_from {
            true => {
                let live = Live(after[index].clone());
                end.0[..words].copy_from_slice(&of_globals(&live, 0));
            }
            false => kept_globals.iter().for_each(|&global| end.set(global)),
        }
        for (local, variable) in function.local_variables.iter() {
            if kept(&variable.name) {
                end.set(globals + local.index());
            }
        }
        let mut walk = Walk {
            registers,
            function: index,
            expressions: &function.expressions,
            calls: &calls,
            after: &mut after,
            arguments: &function.arguments,
            end: end.clone(),
            drops: true,
            dropped: [0; 2],
        };
        let none = Live::none(count);
        walk.block(&mut function.body, end, [&none, &none]);
        dropped[0] += walk.dropped[0];
        dropped[1] += walk.dropped[1];
    }
    dropped
}

/// A copy of `arena` without what `dead` says of, and where each of the rest went.
fn without<T>(
    arena: &mut Arena<T>,
    dead: impl Fn(Handle<T>, &T) -> bool,
) -> Vec<Option<Handle<T>>> {
    let mut left = Arena::new();
    let moved = (std::mem::take(arena).drain())
        .map(|(handle, value, span)| (!dead(handle, &value)).then(|| left.append(value, span)))
        .collect();
    *arena = left;
    moved
}

/// Drops the variables of a thread's own that no expression of `module` names; how many. A
/// register kept by name is named by the last store to it.
pub(super) fn registers(module: &mut naga::Module) -> usize {
    let mut named = vec![false; module.global_variables.len()];
    for entry in &module.entry_points {
        let held = [entry.task_payload, entry.incoming_ray_payload];
        let output = entry.mesh_info.as_ref().map(|info| info.output_variable);
        for global in held.into_iter().flatten().chain(output) {
            named[global.index()] = true;
        }
    }
    let mut dropped = 0;
    for function in super::functions_mut(module) {
        let mut local = vec![false; function.local_variables.len()];
        for (_, expression) in function.expressions.iter() {
            match *expression {
                E::GlobalVariable(global) => named[global.index()] = true,
                E::LocalVariable(variable) => local[variable.index()] = true,
                _ => {}
            }
        }
        if local.iter().all(|&named| named) {
            continue;
        }
        let moved = without(&mut function.local_variables, |at, _| !local[at.index()]);
        dropped += moved.iter().filter(|moved| moved.is_none()).count();
        for (_, expression) in function.expressions.iter_mut() {
            if let E::LocalVariable(variable) = expression {
                *variable = moved[variable.index()].expect("a local an expression names");
            }
        }
    }
    let dead = |at: Handle<naga::GlobalVariable>, global: &naga::GlobalVariable| {
        global.space == naga::AddressSpace::Private && !named[at.index()]
    };
    // A module with comments on its globals names them there as well: it is left as it is.
    let none = module.doc_comments.is_some()
        || !(module.global_variables.iter()).any(|(at, global)| dead(at, global));
    if none {
        return dropped;
    }
    let moved = without(&mut module.global_variables, dead);
    dropped += moved.iter().filter(|moved| moved.is_none()).count();
    let at = |global: &mut Handle<naga::GlobalVariable>| {
        *global = moved[global.index()].expect("a global the module names");
    };
    for entry in &mut module.entry_points {
        entry.task_payload.iter_mut().for_each(at);
        entry.incoming_ray_payload.iter_mut().for_each(at);
        if let Some(info) = &mut entry.mesh_info {
            at(&mut info.output_variable);
        }
    }
    for function in super::functions_mut(module) {
        for (_, expression) in function.expressions.iter_mut() {
            if let E::GlobalVariable(global) = expression {
                at(global);
            }
        }
    }
    dropped
}
