//! Whether the loops of a translated stage end with the editor's noughts where the game's
//! buffers are: a device that waits on a loop that never ends is lost.

use std::collections::{BTreeMap, BTreeSet};

use naga::{Expression as E, Handle, Statement as S};

/// The loops the functions of `module` hold.
pub fn loops(module: &naga::Module) -> usize {
    fn of(block: &naga::Block) -> usize {
        (block.iter())
            .map(|statement| match statement {
                S::Block(block) => of(block),
                S::If { accept, reject, .. } => of(accept) + of(reject),
                S::Switch { cases, .. } => cases.iter().map(|case| of(&case.body)).sum(),
                S::Loop {
                    body, continuing, ..
                } => 1 + of(body) + of(continuing),
                _ => 0,
            })
            .sum()
    }
    functions(module).map(|function| of(&function.body)).sum()
}

fn functions(module: &naga::Module) -> impl Iterator<Item = &naga::Function> {
    (module.functions.iter().map(|(_, function)| function))
        .chain(module.entry_points.iter().map(|entry| &entry.function))
}

fn each_statement<'b>(block: &'b naga::Block, visit: &mut dyn FnMut(&'b S)) {
    for statement in block {
        visit(statement);
        match statement {
            S::Block(block) => each_statement(block, visit),
            S::If { accept, reject, .. } => {
                each_statement(accept, visit);
                each_statement(reject, visit);
            }
            S::Switch { cases, .. } => {
                for case in cases {
                    each_statement(&case.body, visit);
                }
            }
            S::Loop {
                body, continuing, ..
            } => {
                each_statement(body, visit);
                each_statement(continuing, visit);
            }
            _ => {}
        }
    }
}

/// A set of the buffers only the game fills, by their place in the list `counted_to` is given.
type Taint = u64;

fn bit(at: usize) -> Taint {
    1 << at.min(63)
}

/// `expression` under its casts.
fn uncast(function: &naga::Function, mut expression: Handle<E>) -> Handle<E> {
    while let E::As { expr, .. } = function.expressions[expression] {
        expression = expr;
    }
    expression
}

/// The local `expression` loads whole, under its casts.
fn loaded(function: &naga::Function, expression: Handle<E>) -> Option<Handle<naga::LocalVariable>> {
    let E::Load { pointer } = function.expressions[uncast(function, expression)] else {
        return None;
    };
    match function.expressions[pointer] {
        E::LocalVariable(local) => Some(local),
        _ => None,
    }
}

/// What a loop counts with.
#[derive(Default)]
struct Counters {
    locals: BTreeSet<Handle<naga::LocalVariable>>,
    /// What the loop stores to them, under its casts.
    // After the optimiser a count is compared as the value stored to it: no load of it is left.
    stored: BTreeSet<Handle<E>>,
}

/// The locals a loop of `body` and `continuing` counts with: one it stores as itself and a
/// stated number more or less, or as another local so moved that it then stores from this one.
fn counters(function: &naga::Function, blocks: [&naga::Block; 2]) -> Counters {
    let (mut moved, mut copied) = (Vec::new(), Vec::new());
    for block in blocks {
        each_statement(block, &mut |statement| {
            let S::Store { pointer, value } = statement else {
                return;
            };
            let E::LocalVariable(to) = function.expressions[*pointer] else {
                return;
            };
            if let Some(from) = loaded(function, *value) {
                copied.push((to, from));
            }
            let E::Binary { op, left, right } = function.expressions[uncast(function, *value)]
            else {
                return;
            };
            if !matches!(
                op,
                naga::BinaryOperator::Add | naga::BinaryOperator::Subtract
            ) {
                return;
            }
            let stated = |e| matches!(function.expressions[uncast(function, e)], E::Literal(_));
            let from = match (stated(left), stated(right)) {
                (false, true) => loaded(function, left),
                (true, false) => loaded(function, right),
                _ => None,
            };
            if let Some(from) = from {
                moved.push((to, from, uncast(function, *value)));
            }
        });
    }
    let mut counters = Counters::default();
    for (to, from, value) in moved {
        if to == from || copied.contains(&(from, to)) {
            counters.locals.extend([to, from]);
            counters.stored.insert(value);
        }
    }
    counters
}

struct Flow<'m> {
    function: &'m naga::Function,
    /// Each global of the module the game fills, by its bit.
    globals: &'m BTreeMap<Handle<naga::GlobalVariable>, Taint>,
    /// What each function reads of them anywhere.
    called: &'m BTreeMap<Handle<naga::Function>, Taint>,
    /// By expression, as last evaluated.
    values: Vec<Taint>,
    locals: BTreeMap<Handle<naga::LocalVariable>, Taint>,
    /// The counters of each loop the walk is inside of.
    inside: Vec<Counters>,
    /// The buffers a counter was compared with.
    hit: Taint,
}

impl Flow<'_> {
    fn of(&self, expression: Handle<E>) -> Taint {
        self.values[expression.index()]
    }

    /// The local a pointer is into, through the elements it picks.
    fn root(&self, mut pointer: Handle<E>) -> Option<Handle<naga::LocalVariable>> {
        loop {
            pointer = match self.function.expressions[pointer] {
                E::LocalVariable(local) => return Some(local),
                E::Access { base, .. } | E::AccessIndex { base, .. } => base,
                _ => return None,
            };
        }
    }

    /// What a counter of a loop the walk is inside of is compared with.
    fn hit_by(&self, op: naga::BinaryOperator, left: Handle<E>, right: Handle<E>) -> Taint {
        use naga::BinaryOperator as B;
        let compares = matches!(
            op,
            B::Equal | B::NotEqual | B::Less | B::LessEqual | B::Greater | B::GreaterEqual
        );
        if !compares {
            return 0;
        }
        let counts = |e| {
            let (local, value) = (loaded(self.function, e), uncast(self.function, e));
            (self.inside.iter()).any(|counters| {
                local.is_some_and(|local| counters.locals.contains(&local))
                    || counters.stored.contains(&value)
            })
        };
        let with = |counter, other| if counts(counter) { self.of(other) } else { 0 };
        with(left, right) | with(right, left)
    }

    fn evaluate(&mut self, expression: Handle<E>) {
        let of = |e: Handle<E>| self.of(e);
        let taint = match &self.function.expressions[expression] {
            E::GlobalVariable(global) => self.globals.get(global).copied().unwrap_or(0),
            E::Load { pointer } => {
                let held = self
                    .root(*pointer)
                    .and_then(|local| self.locals.get(&local));
                of(*pointer) | held.copied().unwrap_or(0)
            }
            E::Access { base, index } => of(*base) | of(*index),
            E::AccessIndex { base, .. } => of(*base),
            E::Splat { value: e, .. }
            | E::Swizzle { vector: e, .. }
            | E::Unary { expr: e, .. }
            | E::Derivative { expr: e, .. }
            | E::Relational { argument: e, .. }
            | E::As { expr: e, .. }
            | E::ArrayLength(e) => of(*e),
            E::Compose { components, .. } => components.iter().fold(0, |all, e| all | of(*e)),
            E::Binary { op, left, right } => {
                self.hit |= self.hit_by(*op, *left, *right);
                self.of(*left) | self.of(*right)
            }
            E::Select {
                condition,
                accept,
                reject,
            } => of(*condition) | of(*accept) | of(*reject),
            E::Math {
                arg,
                arg1,
                arg2,
                arg3,
                ..
            } => [arg1, arg2, arg3]
                .into_iter()
                .flatten()
                .fold(of(*arg), |all, e| all | of(*e)),
            E::ImageSample {
                image, coordinate, ..
            }
            | E::ImageLoad {
                image, coordinate, ..
            } => of(*image) | of(*coordinate),
            E::ImageQuery { image, .. } => of(*image),
            _ => return,
        };
        self.values[expression.index()] = taint;
    }

    fn walk(&mut self, block: &naga::Block) {
        for statement in block {
            match statement {
                S::Emit(range) => {
                    for expression in range.clone() {
                        self.evaluate(expression);
                    }
                }
                S::Block(block) => self.walk(block),
                S::Store { pointer, value } => {
                    let Some(local) = self.root(*pointer) else {
                        continue;
                    };
                    let whole = matches!(self.function.expressions[*pointer], E::LocalVariable(_));
                    let stored = self.of(*value) | self.of(*pointer);
                    let held = self.locals.entry(local).or_default();
                    *held = if whole { stored } else { *held | stored };
                }
                S::Call {
                    function,
                    arguments,
                    result,
                } => {
                    let read = self.called.get(function).copied().unwrap_or(0);
                    let given = arguments.iter().fold(read, |all, e| all | self.of(*e));
                    if let Some(result) = result {
                        self.values[result.index()] = given;
                    }
                }
                S::If { accept, reject, .. } => {
                    let before = self.locals.clone();
                    self.walk(accept);
                    let accepted = std::mem::replace(&mut self.locals, before);
                    self.walk(reject);
                    self.join(accepted);
                }
                S::Switch { cases, .. } => {
                    let before = self.locals.clone();
                    let mut after = before.clone();
                    for case in cases {
                        self.locals = before.clone();
                        self.walk(&case.body);
                        let ran = std::mem::replace(&mut self.locals, after);
                        self.join(ran);
                        after = self.locals.clone();
                    }
                }
                S::Loop {
                    body, continuing, ..
                } => {
                    self.inside
                        .push(counters(self.function, [body, continuing]));
                    // Until a turn adds nothing: what a turn leaves, the next one starts from.
                    loop {
                        let before = self.locals.clone();
                        self.walk(body);
                        self.walk(continuing);
                        self.join(before.clone());
                        if self.locals == before {
                            break;
                        }
                    }
                    self.inside.pop();
                }
                _ => {}
            }
        }
    }

    fn join(&mut self, other: BTreeMap<Handle<naga::LocalVariable>, Taint>) {
        for (local, taint) in other {
            *self.locals.entry(local).or_default() |= taint;
        }
    }
}

/// The first of `game`, the bindings only the game fills, each with the hash of its name, that
/// a loop of `module` compares its count with: the number such a loop runs to is the game's,
/// and with noughts there a count that leaves on an exact hit never hits it.
// ponytail: a count against a number alone, followed along what the program stores and no
// branch it takes; a loop that walks a list or the bits of a mask the game fills is not seen.
// Follow those when an effect that has one is wanted.
pub fn counted_to(module: &naga::Module, game: &[((u32, u32), u32)]) -> Option<u32> {
    let mut globals = BTreeMap::new();
    for (handle, global) in module.global_variables.iter() {
        let at = (global.binding.as_ref()).map(|binding| (binding.group, binding.binding));
        if let Some(place) = game.iter().position(|(bound, _)| Some(*bound) == at) {
            globals.insert(handle, bit(place));
        }
    }
    let reads = |function: &naga::Function| {
        (function.expressions.iter()).fold(0, |all, (_, expression)| match expression {
            E::GlobalVariable(global) => all | globals.get(global).copied().unwrap_or(0),
            _ => all,
        })
    };
    let called: BTreeMap<_, _> = (module.functions.iter())
        .map(|(handle, function)| (handle, reads(function)))
        .collect();
    let mut hit = 0;
    for function in functions(module) {
        let mut flow = Flow {
            function,
            globals: &globals,
            called: &called,
            values: vec![0; function.expressions.len()],
            locals: BTreeMap::new(),
            inside: Vec::new(),
            hit: 0,
        };
        // A global is no statement's to emit: its bit stands from the start.
        for (expression, _) in function.expressions.iter() {
            if matches!(function.expressions[expression], E::GlobalVariable(_)) {
                flow.evaluate(expression);
            }
        }
        flow.walk(&function.body);
        hit |= flow.hit;
    }
    (game.iter().enumerate())
        .find(|(place, _)| hit & bit(*place) != 0)
        .map(|(_, (_, hash))| *hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use naga::{BinaryOperator as B, Literal, Span};

    /// A function that reads a number of the buffer at binding 3, then loops: `i = i + 1`, and
    /// leaves when `i` is `exit` the number or, with none, when it is 4; the mask is stored
    /// `mask` first.
    fn counting(exit: Option<B>, mask: u32) -> naga::Module {
        let mut module = naga::Module::default();
        let u32_ty = module.types.insert(
            naga::Type {
                name: None,
                inner: naga::TypeInner::Scalar(naga::Scalar::U32),
            },
            Span::UNDEFINED,
        );
        let buffer = module.global_variables.append(
            naga::GlobalVariable {
                name: None,
                space: naga::AddressSpace::Uniform,
                binding: Some(naga::ResourceBinding {
                    group: 0,
                    binding: 3,
                }),
                ty: u32_ty,
                init: None,
                memory_decorations: Default::default(),
            },
            Span::UNDEFINED,
        );
        let mut function = naga::Function::default();
        let mut local = |name: &str| {
            function.local_variables.append(
                naga::LocalVariable {
                    name: Some(name.to_owned()),
                    ty: u32_ty,
                    init: None,
                },
                Span::UNDEFINED,
            )
        };
        let (i, bound, exec) = (local("s1_"), local("s2_"), local("exec_lo"));
        let x = &mut function.expressions;
        let mut add = |e| x.append(e, Span::UNDEFINED);
        let (i_at, bound_at, exec_at) = (
            add(E::LocalVariable(i)),
            add(E::LocalVariable(bound)),
            add(E::LocalVariable(exec)),
        );
        let global = add(E::GlobalVariable(buffer));
        let (one, lanes, four) = (
            add(E::Literal(Literal::U32(1))),
            add(E::Literal(Literal::U32(mask))),
            add(E::Literal(Literal::U32(4))),
        );
        let read = add(E::Load { pointer: global });
        let was = add(E::Load { pointer: i_at });
        let next = add(E::Binary {
            op: B::Add,
            left: was,
            right: one,
        });
        let now = add(E::Load { pointer: i_at });
        let to = add(E::Load { pointer: bound_at });
        let leave = add(E::Binary {
            op: exit.unwrap_or(B::Equal),
            left: now,
            right: if exit.is_some() { to } else { four },
        });
        let emit = |from: Handle<E>, to: Handle<E>| S::Emit(naga::Range::new_from_bounds(from, to));
        let store = |pointer, value| S::Store { pointer, value };
        let body = vec![
            emit(was, next),
            store(i_at, next),
            emit(now, leave),
            S::If {
                condition: leave,
                accept: naga::Block::from_vec(vec![S::Break]),
                reject: naga::Block::new(),
            },
        ];
        function.body = naga::Block::from_vec(vec![
            store(exec_at, lanes),
            emit(read, read),
            store(bound_at, read),
            S::Loop {
                body: naga::Block::from_vec(body),
                continuing: naga::Block::new(),
                break_if: None,
            },
        ]);
        module.functions.append(function, Span::UNDEFINED);
        module
    }

    #[test]
    fn a_loop_that_counts_to_a_number_only_the_game_fills_is_told() {
        let game = [((0, 7), 0xAAAA_0001), ((0, 3), 0x1948_E9AA)];
        let hit = counting(Some(B::Equal), 1);
        assert_eq!(loops(&hit), 1);
        assert_eq!(counted_to(&hit, &game), Some(0x1948_E9AA));
        let past = counting(Some(B::GreaterEqual), 1);
        assert_eq!(counted_to(&past, &game), Some(0x1948_E9AA));
        // A count to a number the program states, beside a number of the game's it only reads.
        assert_eq!(counted_to(&counting(None, 1), &game), None);
        // The number is the model's own.
        assert_eq!(counted_to(&hit, &game[..1]), None);
    }

    #[test]
    fn a_count_is_told_of_a_module_the_optimiser_has_run_over() {
        // The count is then compared as the value stored to it, and the number as the load
        // of the buffer itself: no load of either register is left.
        let game = [((0, 7), 0xAAAA_0001), ((0, 3), 0x1948_E9AA)];
        for exit in [B::Equal, B::GreaterEqual] {
            let mut module = counting(Some(exit), 1);
            let changed = crate::rdna::opt::optimise(&mut module, &[]);
            assert!(
                changed.forwarded > 0 && changed.registers > 0,
                "{changed:?}"
            );
            assert_eq!(loops(&module), 1);
            assert_eq!(counted_to(&module, &game), Some(0x1948_E9AA));
        }
        let mut stated = counting(None, 1);
        crate::rdna::opt::optimise(&mut stated, &[]);
        assert_eq!(counted_to(&stated, &game), None);
    }
}
