//! The watchdog run by a small evaluator of the statements a walk-shaped module holds: a loop that
//! never ends is left at the cap and counted, one that ends computes what it did.

use std::collections::HashMap;

use naga::{Expression as E, Handle, Statement as S};

use super::pass::{unwatched, watchdog, watchdog_cap, WATCHDOG_MARGIN};

/// An entry whose loop holds a loop and calls a function with a loop of its own, which ends where
/// `{EXIT}` says; it leaves its sum and its outer count in `registers_left`.
const WALK_SHAPED: &str = r#"
@group(0) @binding(0) var<storage, read_write> registers_left: array<u32>;

fn spin(n: u32) -> u32 {
    var i: u32;
    i = 0u;
    loop {
        if {EXIT} { break; }
        i = i + 1u;
    }
    return i;
}

@compute @workgroup_size(1)
fn main() {
    var total: u32;
    var k: u32;
    var j: u32;
    total = 0u;
    k = 0u;
    loop {
        if k >= 3u { break; }
        j = 0u;
        loop {
            if j >= 2u { break; }
            j = j + 1u;
            total = total + 1u;
        }
        total = total + spin(5u);
        k = k + 1u;
    }
    registers_left[0] = total + 1u;
    registers_left[1] = k;
}
"#;

/// [`WALK_SHAPED`] whose called loop ends after five turns, or never.
fn walk_shaped(ends: bool) -> naga::Module {
    let exit = if ends { "i == n" } else { "n == 7u" };
    naga::front::wgsl::parse_str(&WALK_SHAPED.replace("{EXIT}", exit)).expect("the WGSL parses")
}

fn valid(module: &naga::Module) {
    use naga::valid::{Capabilities, ValidationFlags, Validator};
    Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(module)
        .expect("a module the validator takes");
}

/// What [`run`] leaves: `registers_left`, the watchdog's flag and count, and the turns of loops run.
#[derive(Debug, PartialEq)]
struct Ran {
    left: [u32; 2],
    capped: bool,
    count: u32,
    turns: u64,
}

/// A value or a pointer.
#[derive(Debug, Clone, Copy, PartialEq)]
enum V {
    U(u32),
    B(bool),
    Local(Handle<naga::LocalVariable>),
    Global(Handle<naga::GlobalVariable>, u32),
}

enum Flow {
    Next,
    Break,
    Return(Option<V>),
}

/// One invocation's memory and the turns its loops took.
struct Run<'m> {
    module: &'m naga::Module,
    globals: HashMap<(Handle<naga::GlobalVariable>, u32), V>,
    turns: u64,
}

/// One call's arguments, locals and emitted expressions.
struct Frame<'f> {
    function: &'f naga::Function,
    arguments: Vec<V>,
    locals: HashMap<Handle<naga::LocalVariable>, V>,
    values: HashMap<Handle<E>, V>,
}

fn u(v: V) -> u32 {
    match v {
        V::U(x) => x,
        other => panic!("{other:?} is no u32"),
    }
}

fn b(v: V) -> bool {
    match v {
        V::B(x) => x,
        other => panic!("{other:?} is no bool"),
    }
}

impl Run<'_> {
    /// An expression's value: emitted, or one that needs no emitting.
    fn value(&self, frame: &Frame, e: Handle<E>) -> V {
        use naga::Literal as L;
        match frame.function.expressions[e] {
            E::Literal(L::U32(x)) => V::U(x),
            E::Literal(L::Bool(x)) => V::B(x),
            E::LocalVariable(local) => V::Local(local),
            E::GlobalVariable(global) => V::Global(global, 0),
            E::FunctionArgument(k) => frame.arguments[k as usize],
            _ => *(frame.values.get(&e)).unwrap_or_else(|| panic!("{e:?} is not emitted")),
        }
    }

    fn evaluate(&self, frame: &Frame, e: Handle<E>) -> V {
        use naga::BinaryOperator as B;
        let of = |e| self.value(frame, e);
        match frame.function.expressions[e] {
            E::Load { pointer } => self.load(frame, of(pointer)),
            E::Access { base, index } => match of(base) {
                V::Global(global, _) => V::Global(global, u(of(index))),
                other => panic!("{other:?} is indexed"),
            },
            E::AccessIndex { base, index } => match of(base) {
                V::Global(global, _) => V::Global(global, index),
                other => panic!("{other:?} is indexed"),
            },
            E::Binary { op, left, right } => match op {
                B::Add => V::U(u(of(left)).wrapping_add(u(of(right)))),
                B::Equal => V::B(of(left) == of(right)),
                B::GreaterEqual => V::B(u(of(left)) >= u(of(right))),
                B::LogicalOr => V::B(b(of(left)) || b(of(right))),
                other => panic!("the evaluator computes no {other:?}"),
            },
            E::Unary {
                op: naga::UnaryOperator::LogicalNot,
                expr,
            } => V::B(!b(of(expr))),
            E::Select {
                condition,
                accept,
                reject,
            } => match b(of(condition)) {
                true => of(accept),
                false => of(reject),
            },
            ref other => panic!("the evaluator computes no {other:?}"),
        }
    }

    fn load(&self, frame: &Frame, pointer: V) -> V {
        match pointer {
            V::Local(local) => frame.locals[&local],
            V::Global(global, at) => match self.globals.get(&(global, at)) {
                Some(v) => *v,
                None => match self.module.global_variables[global].init {
                    Some(init) => match self.module.global_expressions[init] {
                        E::Literal(naga::Literal::Bool(x)) => V::B(x),
                        ref other => panic!("{other:?}"),
                    },
                    None => V::U(0),
                },
            },
            other => panic!("{other:?} is no pointer"),
        }
    }

    fn store(&mut self, frame: &mut Frame, pointer: V, value: V) {
        match pointer {
            V::Local(local) => {
                frame.locals.insert(local, value);
            }
            V::Global(global, at) => {
                self.globals.insert((global, at), value);
            }
            other => panic!("{other:?} is no pointer"),
        }
    }

    fn call(&mut self, function: &naga::Function, arguments: Vec<V>) -> Option<V> {
        let mut frame = Frame {
            function,
            arguments,
            locals: HashMap::new(),
            values: HashMap::new(),
        };
        match self.block(&mut frame, &function.body) {
            Flow::Return(value) => value,
            _ => None,
        }
    }

    fn block(&mut self, frame: &mut Frame, block: &naga::Block) -> Flow {
        for statement in block {
            let flow = match statement {
                S::Emit(range) => {
                    for e in range.clone() {
                        let v = self.evaluate(frame, e);
                        frame.values.insert(e, v);
                    }
                    Flow::Next
                }
                S::Block(inner) => self.block(frame, inner),
                S::Store { pointer, value } => {
                    let (pointer, value) = (self.value(frame, *pointer), self.value(frame, *value));
                    self.store(frame, pointer, value);
                    Flow::Next
                }
                S::Atomic {
                    pointer,
                    fun: naga::AtomicFunction::Add,
                    value,
                    ..
                } => {
                    let pointer = self.value(frame, *pointer);
                    let sum = u(self.load(frame, pointer)) + u(self.value(frame, *value));
                    self.store(frame, pointer, V::U(sum));
                    Flow::Next
                }
                S::If {
                    condition,
                    accept,
                    reject,
                } => match b(self.value(frame, *condition)) {
                    true => self.block(frame, accept),
                    false => self.block(frame, reject),
                },
                S::Loop {
                    body,
                    continuing,
                    break_if,
                } => loop {
                    self.turns += 1;
                    assert!(self.turns < 1 << 20, "past the evaluator's own bound");
                    match self.block(frame, body) {
                        Flow::Break => break Flow::Next,
                        Flow::Return(v) => break Flow::Return(v),
                        Flow::Next => {}
                    }
                    self.block(frame, continuing);
                    if break_if.is_some_and(|e| b(self.value(frame, e))) {
                        break Flow::Next;
                    }
                },
                S::Break => Flow::Break,
                S::Return { value } => Flow::Return(value.map(|e| self.value(frame, e))),
                S::Call {
                    function,
                    arguments,
                    result,
                } => {
                    let given = arguments.iter().map(|e| self.value(frame, *e)).collect();
                    let returned = self.call(&self.module.functions[*function], given);
                    if let (Some(result), Some(v)) = (result, returned) {
                        frame.values.insert(*result, v);
                    }
                    Flow::Next
                }
                other => panic!("the evaluator runs no {other:?}"),
            };
            if !matches!(flow, Flow::Next) {
                return flow;
            }
        }
        Flow::Next
    }
}

/// `module`'s compute entry run once.
fn run(module: &naga::Module) -> Ran {
    let mut run = Run {
        module,
        globals: HashMap::new(),
        turns: 0,
    };
    run.call(&module.entry_points[0].function, Vec::new());
    let named = |name: &str| {
        (module.global_variables.iter())
            .find(|(_, g)| g.name.as_deref() == Some(name))
            .map(|(h, _)| h)
    };
    let word = |global, at| match run.globals.get(&(global, at)) {
        Some(V::U(x)) => *x,
        _ => 0,
    };
    let left = named("registers_left").expect("the module's registers_left");
    Ran {
        left: [word(left, 0), word(left, 1)],
        capped: named("watchdog_capped")
            .is_some_and(|g| run.globals.get(&(g, 0)) == Some(&V::B(true))),
        count: named("watchdog_count").map_or(0, |g| word(g, 0)),
        turns: run.turns,
    }
}

#[test]
fn every_loop_of_a_walk_shaped_module_is_watched_nested_and_called_alike() {
    let module = walk_shaped(true);
    assert_eq!(
        unwatched(&module),
        3,
        "the outer, the nested and the called loop"
    );
    let watched = watchdog(&module, 100).unwrap();
    valid(&watched.module);
    assert_eq!(
        (watched.loops, watched.unwatched, watched.stores),
        (3, 0, 2)
    );
    // The count is bound after the module's own binding.
    assert_eq!(watched.binding, 1);
    // A module with no registers_left has nowhere to leave a miss.
    let bare =
        naga::front::wgsl::parse_str("@compute @workgroup_size(1) fn main() { loop {} }").unwrap();
    assert!(watchdog(&bare, 100).is_err());
}

#[test]
fn a_walk_that_ends_before_the_cap_computes_what_it_did() {
    let module = walk_shaped(true);
    let before = run(&module);
    // Three outer turns, each with two nested and five called ones, the sum one more.
    assert_eq!(before.left, [3 * (2 + 5) + 1, 3]);
    let after = run(&watchdog(&module, 100).unwrap().module);
    assert_eq!(after.left, before.left);
    assert!(!after.capped);
    assert_eq!(after.count, 0);
    assert_eq!(after.turns, before.turns);
    // The cap is per entry into a loop: six turns of the called loop each time stay under seven.
    assert!(!run(&watchdog(&module, 7).unwrap().module).capped);
}

#[test]
fn a_loop_that_never_ends_is_left_at_the_cap_counted_once_and_leaves_a_miss() {
    let cap = 100;
    let ran = run(&watchdog(&walk_shaped(false), cap).unwrap().module);
    assert!(ran.capped);
    assert_eq!(ran.count, 1, "the invocation is counted once");
    assert_eq!(ran.left, [0, 0], "every register left nought: a miss");
    // The outer turn, the nested loop's three, the called loop's `cap` and the turn that leaves
    // it, and the outer turn that leaves on the flag.
    assert_eq!(ran.turns, 1 + 3 + u64::from(cap) + 1 + 1);
    // A loop that ends, cut by a cap below its turns, is capped likewise.
    let cut = run(&watchdog(&walk_shaped(true), 3).unwrap().module);
    assert!(cut.capped && cut.left == [0, 0] && cut.count == 1);
}

#[test]
fn the_cap_is_a_margin_over_the_walk_s_bound_or_the_arena_a_query_walks() {
    assert_eq!(
        u64::from(watchdog_cap(0xFFFF, 0)),
        0x10000 * WATCHDOG_MARGIN
    );
    assert_eq!(
        u64::from(watchdog_cap(0xFFFF, 1 << 20)),
        (1 << 20) * WATCHDOG_MARGIN
    );
    assert_eq!(watchdog_cap(u32::MAX, usize::MAX), u32::MAX - 1);
}
