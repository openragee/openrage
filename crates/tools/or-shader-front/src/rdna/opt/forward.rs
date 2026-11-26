//! Forwarding and common subexpressions. A load of a register reads what the last store left
//! there, and an expression equal to one already in scope reads that one: the same operator of
//! the same operands in the same order, so a float is the value it was and nothing is
//! re-associated.
//!
//! A register is a variable read and written only whole, so a store to anything else cannot
//! change one. A load of memory a statement may write (a buffer bound for storing, the group's
//! memory, a table of the thread's own) is never taken for an earlier load, nor is a sample, a
//! texel load or a derivative. What a loop, a `switch` or a call stores to is forgotten at it,
//! and what an arm of an `if` stores differently from the other is forgotten after it.

use std::collections::HashMap;

use naga::{Arena, Block, Expression as E, Function, Handle, Literal, Statement as S};

use super::Registers;

/// In scope from a function's start: what no statement emits.
const START: u32 = 0;
/// In scope nowhere: what the walk has not reached.
const NOWHERE: u32 = 1;

/// What an expression is made of, with each operand as the expression that stands for it.
type Key = (u8, u32, [u32; 4]);

/// What each register holds, by its place among the function's variables: an expression, good
/// for as long as the block that emitted it is open.
type State = Vec<Option<Handle<E>>>;

/// What a walk of one function found.
pub(super) struct Walked {
    /// By expression: the equal expression its readers read in its place, or itself.
    stands: Vec<Handle<E>>,
    forwarded: usize,
    /// Stores of a `select` whose other arm is what the register held.
    pub(super) guarded: usize,
}

struct Walk<'a> {
    registers: &'a Registers,
    function: usize,
    expressions: &'a Arena<E>,
    stands: Vec<Handle<E>>,
    /// By expression: the block that emitted it, as a place in `open`.
    scope: Vec<u32>,
    open: Vec<bool>,
    current: u32,
    seen: HashMap<Key, Handle<E>>,
    forwarded: usize,
    guarded: usize,
}

pub(super) fn walk(function: &Function, index: usize, registers: &Registers) -> Walked {
    let expressions = &function.expressions;
    let mut walk = Walk {
        registers,
        function: index,
        expressions,
        stands: expressions.iter().map(|(at, _)| at).collect(),
        scope: vec![NOWHERE; expressions.len()],
        open: vec![true, false],
        current: START,
        seen: HashMap::new(),
        forwarded: 0,
        guarded: 0,
    };
    for (at, expression) in expressions.iter() {
        if expression.needs_pre_emit() {
            walk.scope[at.index()] = START;
            walk.number(at);
        }
    }
    walk.block(&function.body, vec![None; registers.count(index)]);
    Walked {
        stands: walk.stands,
        forwarded: walk.forwarded,
        guarded: walk.guarded,
    }
}

impl Walk<'_> {
    /// Whether `earlier` can stand for `at`: its block is open, and it comes before `at` in the
    /// function's expressions, as everything an expression reads must.
    // ⚠ A statement placed first may emit an expression made last, so "in scope" is not "before".
    fn stands_for(&self, earlier: Handle<E>, at: Handle<E>) -> bool {
        self.open[self.scope[earlier.index()] as usize] && earlier.index() < at.index()
    }

    /// `None` for what is computed anew each time it is emitted, or is not told apart here.
    fn key(&self, expression: &E) -> Option<Key> {
        // One more than its place, so that nought is no operand.
        let of = |at: Handle<E>| self.stands[at.index()].index() as u32 + 1;
        let or_none = |at: Option<Handle<E>>| at.map_or(0, of);
        Some(match *expression {
            E::Literal(Literal::U32(word)) => (0, word, [0; 4]),
            E::Literal(Literal::I32(word)) => (1, word as u32, [0; 4]),
            // By its bits: nought and minus nought are two values, and so are two NaNs.
            E::Literal(Literal::F32(float)) => (2, float.to_bits(), [0; 4]),
            E::Literal(Literal::Bool(flag)) => (3, u32::from(flag), [0; 4]),
            E::GlobalVariable(global) => (4, global.index() as u32, [0; 4]),
            E::LocalVariable(local) => (5, local.index() as u32, [0; 4]),
            E::FunctionArgument(index) => (6, index, [0; 4]),
            E::Access { base, index } => (7, 0, [of(base), of(index), 0, 0]),
            E::AccessIndex { base, index } => (8, index, [of(base), 0, 0, 0]),
            E::Load { pointer } => (9, 0, [of(pointer), 0, 0, 0]),
            E::Unary { op, expr } => (10, op as u32, [of(expr), 0, 0, 0]),
            E::Binary { op, left, right } => (11, op as u32, [of(left), of(right), 0, 0]),
            E::Select {
                condition,
                accept,
                reject,
            } => (12, 0, [of(condition), of(accept), of(reject), 0]),
            E::As {
                expr,
                kind,
                convert,
            } => {
                let how = kind as u32 | u32::from(convert.unwrap_or(u8::MAX)) << 8;
                (13, how, [of(expr), 0, 0, 0])
            }
            E::Math {
                fun,
                arg,
                arg1,
                arg2,
                arg3,
            } => {
                let rest = [or_none(arg1), or_none(arg2), or_none(arg3)];
                (14, fun as u32, [of(arg), rest[0], rest[1], rest[2]])
            }
            E::Splat { size, value } => (15, size as u32, [of(value), 0, 0, 0]),
            E::Swizzle {
                size,
                vector,
                pattern,
            } => {
                let pattern = pattern.iter().fold(0, |all, &one| all << 2 | one as u32);
                (16, size as u32 | pattern << 8, [of(vector), 0, 0, 0])
            }
            E::Compose { ty, ref components } if components.len() <= 4 => {
                let mut parts = [0; 4];
                for (part, &component) in parts.iter_mut().zip(components) {
                    *part = of(component);
                }
                (17, ty.index() as u32, parts)
            }
            E::Relational { fun, argument } => (18, fun as u32, [of(argument), 0, 0, 0]),
            _ => return None,
        })
    }

    /// Has `at` stand as the equal expression in scope, if there is one; whether there was.
    fn number(&mut self, at: Handle<E>) -> bool {
        let Some(key) = self.key(&self.expressions[at]) else {
            return false;
        };
        match self.seen.get(&key) {
            Some(&first) if self.stands_for(first, at) => {
                self.stands[at.index()] = first;
                true
            }
            _ => {
                self.seen.insert(key, at);
                false
            }
        }
    }

    /// Whether a load through `pointer` reads memory no statement of the module writes.
    fn fixed(&self, mut pointer: Handle<E>) -> bool {
        loop {
            match self.expressions[pointer] {
                E::Access { base, .. } | E::AccessIndex { base, .. } => pointer = base,
                E::GlobalVariable(global) => return self.registers.fixed[global.index()],
                _ => return false,
            }
        }
    }

    fn eval(&mut self, at: Handle<E>, state: &mut State) {
        self.scope[at.index()] = self.current;
        if let E::Load { pointer } = self.expressions[at] {
            let pointer = self.stands[pointer.index()];
            if let Some(var) = (self.registers).var(self.function, self.expressions, pointer) {
                match state[var] {
                    Some(held) if self.stands_for(held, at) => {
                        self.stands[at.index()] = held;
                        self.forwarded += 1;
                    }
                    // What it held is not known here: from now on it is known as this load.
                    _ => state[var] = Some(at),
                }
                return;
            }
            if !self.fixed(pointer) {
                return;
            }
        }
        if self.number(at) {
            self.forwarded += 1;
        }
    }

    fn forget(&self, block: &Block, state: &mut State) {
        let mut stored = Vec::new();
        (self.registers).stored(self.function, self.expressions, block, &mut stored);
        stored.iter().for_each(|&var| state[var] = None);
    }

    /// Walks `block` as a scope of its own; `None` where no path leaves its end.
    fn block(&mut self, block: &Block, state: State) -> Option<State> {
        let outer = self.current;
        self.current = self.open.len() as u32;
        self.open.push(true);
        let state = self.run(block, state);
        self.open[self.current as usize] = false;
        self.current = outer;
        state
    }

    fn run(&mut self, block: &Block, mut state: State) -> Option<State> {
        for statement in block.iter() {
            match *statement {
                S::Emit(ref range) => {
                    for at in range.clone() {
                        self.eval(at, &mut state);
                    }
                }
                S::Block(ref inner) => state = self.block(inner, state)?,
                S::If {
                    ref accept,
                    ref reject,
                    ..
                } => {
                    let taken = self.block(accept, state.clone());
                    state = match (taken, self.block(reject, state)) {
                        (Some(mut taken), Some(left)) => {
                            for (held, other) in taken.iter_mut().zip(left) {
                                if *held != other {
                                    *held = None;
                                }
                            }
                            taken
                        }
                        (taken, left) => taken.or(left)?,
                    };
                }
                S::Switch { ref cases, .. } => {
                    // A case is entered from the `switch` or from the case before it.
                    for case in cases {
                        self.forget(&case.body, &mut state);
                    }
                    for case in cases {
                        self.block(&case.body, state.clone());
                    }
                }
                S::Loop {
                    ref body,
                    ref continuing,
                    ..
                } => {
                    // A turn is entered from the loop's start or from the turn before it.
                    self.forget(body, &mut state);
                    self.forget(continuing, &mut state);
                    self.block(body, state.clone());
                    self.block(continuing, state.clone());
                }
                S::Store { pointer, value } => {
                    let pointer = self.stands[pointer.index()];
                    let var = (self.registers).var(self.function, self.expressions, pointer);
                    if let Some(var) = var {
                        if let E::Select { reject, .. } = self.expressions[value] {
                            let kept = self.stands[reject.index()];
                            self.guarded += usize::from(state[var] == Some(kept));
                        }
                        state[var] = Some(self.stands[value.index()]);
                    }
                }
                S::Call {
                    function, result, ..
                } => {
                    let stores = &self.registers.stores[function.index()];
                    for (held, &stored) in state.iter_mut().zip(stores) {
                        if stored {
                            *held = None;
                        }
                    }
                    if let Some(result) = result {
                        self.scope[result.index()] = self.current;
                    }
                }
                S::Atomic {
                    result: Some(result),
                    ..
                } => self.scope[result.index()] = self.current,
                S::Break | S::Continue | S::Return { .. } => return None,
                _ => {}
            }
        }
        Some(state)
    }
}

impl Walked {
    /// Has every reader in `function` read what stands for what it read; how many emitted
    /// expressions another now stands for.
    pub(super) fn apply(self, function: &mut Function) -> usize {
        let mut turn = |read: &mut Handle<E>| *read = self.stands[read.index()];
        for (_, expression) in function.expressions.iter_mut() {
            super::operands(expression, &mut turn);
        }
        super::reads(&mut function.body, &mut turn);
        self.forwarded
    }
}
