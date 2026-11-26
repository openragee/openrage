//! A translated module run here, a thread of this process for each thread of its group: what
//! a test compares with what the instructions' definitions give. It runs what the lowering of
//! arithmetic, registers, branches, shared memory, bound buffers of dwords and barriers writes,
//! and stops on the rest.

use std::collections::HashMap;
use std::sync::{Barrier, Condvar, Mutex};

use naga::{
    BinaryOperator as Bo, Expression as E, GlobalVariable, Handle, Literal, LocalVariable,
    MathFunction as Mf, ScalarKind, Statement as S, TypeInner, UnaryOperator as Uo,
};

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    U(u32),
    I(i32),
    F(f32),
    B(bool),
    List(Vec<Value>),
}

impl Value {
    pub fn word(&self) -> u32 {
        match *self {
            Self::U(word) => word,
            Self::I(word) => word as u32,
            Self::F(float) => float.to_bits(),
            Self::B(flag) => u32::from(flag),
            Self::List(_) => panic!("a list read as a dword"),
        }
    }

    fn flag(&self) -> bool {
        match *self {
            Self::B(flag) => flag,
            _ => panic!("{self:?} read as a condition"),
        }
    }

    /// Its dwords, in order, after those `out` holds.
    pub fn words(&self, out: &mut Vec<u32>) {
        match self {
            Self::List(parts) => parts.iter().for_each(|part| part.words(out)),
            one => out.push(one.word()),
        }
    }

    /// The same shape with every dword nought.
    fn nought(&self) -> Value {
        match self {
            Self::U(_) => Self::U(0),
            Self::I(_) => Self::I(0),
            Self::F(_) => Self::F(0.0),
            Self::B(_) => Self::B(false),
            Self::List(parts) => Self::List(parts.iter().map(Value::nought).collect()),
        }
    }

    fn at(&mut self, path: &[u32]) -> &mut Value {
        match (path.split_first(), self) {
            (None, value) => value,
            (Some((&index, rest)), Value::List(parts)) => {
                let count = parts.len();
                let part = parts.get_mut(index as usize);
                part.unwrap_or_else(|| panic!("element {index} of {count}"))
                    .at(rest)
            }
            (Some(_), other) => panic!("{other:?} has no elements"),
        }
    }
}

#[derive(Debug, Clone)]
enum Place {
    Local(Handle<LocalVariable>, Vec<u32>),
    Global(Handle<GlobalVariable>, Vec<u32>),
    /// Past the end of what a whole run indexed: it reads as this, and a write to it is dropped.
    Nowhere(Value),
}

#[derive(Debug, Clone)]
enum Got {
    Value(Value),
    Place(Place),
}

enum Flow {
    Next,
    Break,
    Continue,
    Return,
}

/// What every thread of a group shares: the memory that is not a thread's own, and the barrier.
struct Group {
    memory: Mutex<HashMap<Handle<GlobalVariable>, Value>>,
    barrier: Barrier,
    /// Where the threads run one at a time, in a stated order.
    baton: Option<Baton>,
}

/// The threads of a group run one at a time: each from one barrier to its next, or to its end,
/// in an order the caller states, and round after round in that order. What a thread does
/// between two barriers is then before or after another's by that order alone, so a program
/// that lacks a barrier shows it on every run and not by chance.
struct Baton {
    /// The round, the place in the order of the thread that runs, and the threads that ended.
    held: Mutex<(u64, usize, Vec<bool>)>,
    passed: Condvar,
}

impl Baton {
    fn new(threads: usize) -> Self {
        Self {
            held: Mutex::new((0, 0, vec![false; threads])),
            passed: Condvar::new(),
        }
    }

    /// Waits for the turn of the thread at `place` in `round`.
    fn take(&self, place: usize, round: u64) {
        let held = self.held.lock().unwrap();
        let _held = (self.passed)
            .wait_while(held, |held| (held.0, held.1) != (round, place))
            .unwrap();
    }

    /// Hands on from the thread at `place`, which has come to a barrier or to its end.
    fn pass(&self, place: usize, ended: bool) {
        let mut held = self.held.lock().unwrap();
        held.2[place] |= ended;
        let (ends, from) = (held.2.clone(), place + 1);
        let next = |from: usize| (from..ends.len()).find(|at| !ends[*at]);
        match next(from) {
            Some(at) => held.1 = at,
            None => (held.0, held.1) = (held.0 + 1, next(0).unwrap_or(ends.len())),
        }
        self.passed.notify_all();
    }
}

/// What a thread left: the registers of its body by name, and whether it was discarded.
#[derive(Debug, Default)]
pub struct Thread {
    /// What the entry point returned.
    pub result: Option<Value>,
    /// Each texel stored to a texture, in order: the texture's number, the place, the texel.
    pub stores: Vec<[Vec<u32>; 3]>,
    /// How many expressions it computed, a turn of a loop counted again.
    pub computed: u64,
    /// The emitted expressions it computed at least once: the function's address, the place.
    pub reached: std::collections::HashSet<(usize, usize)>,
    /// A whole run's limit of computed expressions. With one an index past the end reads
    /// nought and writes nothing, where a test's own run stops.
    pub bounded: Option<u64>,
    pub registers: HashMap<String, u32>,
    pub killed: bool,
    /// The barriers it came to.
    pub barriers: u64,
}

impl Thread {
    /// A register the body never declared was never written: it holds nothing.
    pub fn reg(&self, name: &str) -> u32 {
        self.registers.get(name).copied().unwrap_or(0)
    }
}

struct Machine<'a> {
    module: &'a naga::Module,
    group: &'a Group,
    private: HashMap<Handle<GlobalVariable>, Value>,
    out: Thread,
    /// The thread's place in the order its group runs in, and the round it is in.
    turn: (usize, u64),
}

struct Frame<'a> {
    fun: &'a naga::Function,
    arguments: Vec<Value>,
    locals: HashMap<Handle<LocalVariable>, Value>,
    values: HashMap<Handle<E>, Got>,
}

fn zero(module: &naga::Module, ty: Handle<naga::Type>) -> Value {
    match module.types[ty].inner {
        TypeInner::Scalar(scalar) | TypeInner::Atomic(scalar) => match scalar.kind {
            ScalarKind::Float => Value::F(0.0),
            ScalarKind::Sint => Value::I(0),
            ScalarKind::Bool => Value::B(false),
            _ => Value::U(0),
        },
        TypeInner::Vector { size, scalar } => {
            let part = match scalar.kind {
                ScalarKind::Float => Value::F(0.0),
                ScalarKind::Sint => Value::I(0),
                _ => Value::U(0),
            };
            Value::List(vec![part; size as usize])
        }
        TypeInner::Array {
            base,
            size: naga::ArraySize::Constant(count),
            ..
        } => Value::List(vec![zero(module, base); count.get() as usize]),
        // A buffer nothing was bound to holds nothing.
        TypeInner::Array {
            size: naga::ArraySize::Dynamic,
            ..
        } => Value::List(Vec::new()),
        TypeInner::Struct { ref members, .. } => {
            Value::List(members.iter().map(|m| zero(module, m.ty)).collect())
        }
        // What is bound holds nothing here; an array of them has its entries all the same.
        TypeInner::Image { .. } | TypeInner::Sampler { .. } => Value::U(0),
        TypeInner::BindingArray {
            base,
            size: naga::ArraySize::Constant(count),
        } => Value::List(vec![zero(module, base); count.get() as usize]),
        ref other => panic!("no zero of {other:?}"),
    }
}

impl<'a> Machine<'a> {
    fn memory<R>(&mut self, global: Handle<GlobalVariable>, f: impl FnOnce(&mut Value) -> R) -> R {
        let variable = &self.module.global_variables[global];
        let fresh = || match variable.init {
            Some(init) => constant(self.module, init),
            None => zero(self.module, variable.ty),
        };
        match variable.space {
            naga::AddressSpace::Private => f(self.private.entry(global).or_insert_with(fresh)),
            _ => {
                let mut shared = self.group.memory.lock().unwrap();
                f(shared.entry(global).or_insert_with(fresh))
            }
        }
    }

    fn read(&mut self, frame: &mut Frame<'a>, place: &Place) -> Value {
        match place {
            Place::Nowhere(nought) => nought.clone(),
            Place::Local(local, path) => {
                let ty = frame.fun.local_variables[*local].ty;
                let whole = (frame.locals.entry(*local)).or_insert_with(|| zero(self.module, ty));
                whole.at(path).clone()
            }
            Place::Global(global, path) => {
                let variable = &self.module.global_variables[*global];
                let bound = matches!(
                    self.module.types[variable.ty].inner,
                    TypeInner::Array {
                        size: naga::ArraySize::Dynamic,
                        ..
                    }
                );
                self.memory(*global, |whole| match (whole, path.as_slice()) {
                    // A bound buffer reads as nought past its end.
                    (Value::List(words), [index]) if bound && *index as usize >= words.len() => {
                        Value::U(0)
                    }
                    (whole, path) => whole.at(path).clone(),
                })
            }
        }
    }

    fn write(&mut self, frame: &mut Frame<'a>, place: &Place, value: Value) {
        match place {
            Place::Nowhere(_) => {}
            Place::Local(local, path) => {
                let ty = frame.fun.local_variables[*local].ty;
                let whole = (frame.locals.entry(*local)).or_insert_with(|| zero(self.module, ty));
                *whole.at(path) = value;
            }
            Place::Global(global, path) => self.memory(*global, |whole| *whole.at(path) = value),
        }
    }

    fn got(&mut self, frame: &mut Frame<'a>, at: Handle<E>) -> Got {
        if let Some(got) = frame.values.get(&at) {
            return got.clone();
        }
        // What no statement covers is in scope from the start: it is worked out where it is read.
        self.compute(frame, at)
    }

    fn value(&mut self, frame: &mut Frame<'a>, at: Handle<E>) -> Value {
        match self.got(frame, at) {
            Got::Value(value) => value,
            Got::Place(place) => panic!("{place:?} read as a value"),
        }
    }

    /// How many elements `place` holds, and one of them with every dword nought.
    fn extent(&mut self, frame: &mut Frame<'a>, place: &Place) -> (usize, Value) {
        match place {
            Place::Nowhere(nought) => span(nought),
            Place::Local(local, path) => {
                let ty = frame.fun.local_variables[*local].ty;
                let whole = (frame.locals.entry(*local)).or_insert_with(|| zero(self.module, ty));
                span(whole.at(path))
            }
            Place::Global(global, path) => self.memory(*global, |whole| span(whole.at(path))),
        }
    }

    fn element(&mut self, frame: &mut Frame<'a>, base: Handle<E>, index: u32) -> Got {
        let got = self.got(frame, base);
        if self.out.bounded.is_some() {
            let (count, nought) = match &got {
                Got::Place(place) => self.extent(frame, place),
                Got::Value(whole) => span(whole),
            };
            if index as usize >= count {
                return match got {
                    Got::Place(_) => Got::Place(Place::Nowhere(nought)),
                    Got::Value(_) => Got::Value(nought),
                };
            }
        }
        match got {
            Got::Place(Place::Nowhere(mut whole)) => {
                Got::Place(Place::Nowhere(whole.at(&[index]).clone()))
            }
            Got::Place(Place::Local(local, mut path)) => {
                path.push(index);
                Got::Place(Place::Local(local, path))
            }
            Got::Place(Place::Global(global, mut path)) => {
                path.push(index);
                Got::Place(Place::Global(global, path))
            }
            Got::Value(mut whole) => Got::Value(whole.at(&[index]).clone()),
        }
    }

    fn compute(&mut self, frame: &mut Frame<'a>, at: Handle<E>) -> Got {
        use Value::{B, F, I, U};
        let value = match frame.fun.expressions[at] {
            E::Literal(Literal::U32(word)) => U(word),
            E::Literal(Literal::I32(word)) => I(word),
            E::Literal(Literal::F32(float)) => F(float),
            E::Literal(Literal::Bool(flag)) => B(flag),
            E::FunctionArgument(index) => frame.arguments[index as usize].clone(),
            E::GlobalVariable(global) => return Got::Place(Place::Global(global, Vec::new())),
            E::LocalVariable(local) => return Got::Place(Place::Local(local, Vec::new())),
            E::AccessIndex { base, index } => return self.element(frame, base, index),
            E::Access { base, index } => {
                let index = self.value(frame, index).word();
                return self.element(frame, base, index);
            }
            E::Load { pointer } => match self.got(frame, pointer) {
                Got::Place(place) => self.read(frame, &place),
                Got::Value(value) => panic!("a load of {value:?}"),
            },
            E::Compose { ref components, .. } => {
                Value::List(components.iter().map(|&c| self.value(frame, c)).collect())
            }
            E::Select {
                condition,
                accept,
                reject,
            } => match self.value(frame, condition).flag() {
                true => self.value(frame, accept),
                false => self.value(frame, reject),
            },
            E::Unary { op, expr } => match (op, self.value(frame, expr)) {
                (Uo::BitwiseNot, U(a)) => U(!a),
                (Uo::LogicalNot, B(a)) => B(!a),
                (Uo::Negate, F(a)) => F(-a),
                (Uo::Negate, I(a)) => I(a.wrapping_neg()),
                other => panic!("{other:?} is not run"),
            },
            E::Binary { op, left, right } => {
                let (l, r) = (self.value(frame, left), self.value(frame, right));
                binary(op, l, r)
            }
            E::As {
                expr,
                kind,
                convert,
            } => match (self.value(frame, expr), kind, convert) {
                (value, ScalarKind::Uint, None) => U(value.word()),
                (value, ScalarKind::Sint, None) => I(value.word() as i32),
                (value, ScalarKind::Float, None) => F(f32::from_bits(value.word())),
                (U(a), ScalarKind::Float, Some(_)) => F(a as f32),
                (I(a), ScalarKind::Float, Some(_)) => F(a as f32),
                (F(a), ScalarKind::Uint, Some(_)) => U(a as u32),
                (F(a), ScalarKind::Sint, Some(_)) => I(a as i32),
                (U(a), ScalarKind::Sint, Some(_)) => I(a as i32),
                (I(a), ScalarKind::Uint, Some(_)) => U(a as u32),
                other => panic!("the cast {other:?} is not run"),
            },
            E::Math {
                fun,
                arg,
                arg1,
                arg2,
                ..
            } => {
                let a = self.value(frame, arg);
                let b = arg1.map(|b| self.value(frame, b));
                let c = arg2.map(|c| self.value(frame, c));
                math(fun, a, b, c)
            }
            // The texture and the sampler are looked up, which stops at an entry an array of
            // them does not hold, and the texel is noughts. A whole run numbers each of them:
            // its texel is then what the two and the place sampled decide.
            E::ImageSample {
                image,
                sampler,
                gather,
                coordinate,
                array_index,
                offset,
                level,
                depth_ref,
                clamp_to_edge: _,
            } => {
                let mut seen = Vec::new();
                for bound in [image, sampler] {
                    if let Got::Place(place) = self.got(frame, bound) {
                        seen.push(self.read(frame, &place).word());
                    }
                }
                if seen.first().is_none_or(|&texture| texture == 0) {
                    return Got::Value(Value::List(vec![F(0.0); 4]));
                }
                let level = match level {
                    naga::SampleLevel::Auto | naga::SampleLevel::Zero => [None, None],
                    naga::SampleLevel::Exact(at) | naga::SampleLevel::Bias(at) => [Some(at), None],
                    naga::SampleLevel::Gradient { x, y } => [Some(x), Some(y)],
                };
                let placed = [Some(coordinate), array_index, offset, depth_ref];
                for at in placed.into_iter().chain(level).flatten() {
                    self.value(frame, at).words(&mut seen);
                }
                seen.push(gather.map_or(4, |component| component as u32));
                let texel = texel(&seen);
                match (depth_ref, gather) {
                    (Some(_), None) => F(texel[0]),
                    _ => Value::List(texel.map(F).to_vec()),
                }
            }
            E::ImageLoad {
                image,
                coordinate,
                array_index,
                sample,
                level,
            } => {
                let Got::Place(place) = self.got(frame, image) else {
                    panic!("a texel loaded of no texture");
                };
                let mut seen = vec![self.read(frame, &place).word()];
                for at in [Some(coordinate), array_index, sample, level]
                    .into_iter()
                    .flatten()
                {
                    self.value(frame, at).words(&mut seen);
                }
                let texel = texel(&seen);
                let whole = |float: f32| (float * 65536.0) as u32;
                match texel_kind(self.module, &place) {
                    None => F(texel[0]),
                    Some(ScalarKind::Float) => Value::List(texel.map(F).to_vec()),
                    Some(ScalarKind::Sint) => {
                        Value::List(texel.map(|float| I(whole(float) as i32 - 32768)).to_vec())
                    }
                    Some(_) => Value::List(texel.map(|float| U(whole(float))).to_vec()),
                }
            }
            // One invocation has no neighbour: a slope is a fixed share of the value, so that
            // what reads a slope still depends on what it is the slope of.
            E::Derivative { axis, ctrl, expr } => match self.value(frame, expr) {
                F(a) => {
                    let share = match axis {
                        naga::DerivativeAxis::X => 0.25,
                        naga::DerivativeAxis::Y => -0.125,
                        naga::DerivativeAxis::Width => 0.375,
                    };
                    F(match ctrl {
                        naga::DerivativeControl::Coarse => a * share * 0.5,
                        _ => a * share,
                    })
                }
                other => panic!("the slope of {other:?} is not run"),
            },
            E::ZeroValue(ty) => zero(self.module, ty),
            E::Relational { fun, argument } => match (fun, self.value(frame, argument)) {
                (naga::RelationalFunction::Any, Value::List(parts)) => {
                    B(parts.iter().any(Value::flag))
                }
                (naga::RelationalFunction::All, Value::List(parts)) => {
                    B(parts.iter().all(Value::flag))
                }
                other => panic!("{other:?} is not run"),
            },
            ref other => panic!("{other:?} is not run"),
        };
        Got::Value(value)
    }

    fn run(&mut self, frame: &mut Frame<'a>, block: &'a naga::Block) -> Flow {
        for statement in block {
            match *statement {
                S::Emit(ref range) => {
                    for at in range.clone() {
                        let got = self.compute(frame, at);
                        frame.values.insert(at, got);
                        self.out.computed += 1;
                    }
                    let computed = self.out.computed;
                    assert!(
                        self.out.bounded.is_none_or(|most| computed <= most),
                        "a run past its limit of expressions"
                    );
                }
                S::Block(ref inner) => match self.run(frame, inner) {
                    Flow::Next => {}
                    flow => return flow,
                },
                S::If {
                    condition,
                    ref accept,
                    ref reject,
                } => {
                    let taken = match self.value(frame, condition).flag() {
                        true => accept,
                        false => reject,
                    };
                    match self.run(frame, taken) {
                        Flow::Next => {}
                        flow => return flow,
                    }
                }
                S::Switch {
                    selector,
                    ref cases,
                } => {
                    let chosen = self.value(frame, selector).word();
                    let case = (cases.iter())
                        .find(|case| match case.value {
                            naga::SwitchValue::U32(value) => value == chosen,
                            naga::SwitchValue::I32(value) => value as u32 == chosen,
                            naga::SwitchValue::Default => true,
                        })
                        .expect("a switch with no default");
                    match self.run(frame, &case.body) {
                        // A break inside a case leaves the switch, not the loop around it.
                        Flow::Next | Flow::Break => {}
                        flow => return flow,
                    }
                }
                S::Loop {
                    ref body,
                    ref continuing,
                    break_if,
                } => {
                    for turn in 0.. {
                        assert!(turn < 100_000, "a loop that does not end");
                        match self.run(frame, body) {
                            Flow::Next | Flow::Continue => {}
                            Flow::Break => break,
                            Flow::Return => return Flow::Return,
                        }
                        self.run(frame, continuing);
                        if break_if.is_some_and(|at| self.value(frame, at).flag()) {
                            break;
                        }
                    }
                }
                S::Break => return Flow::Break,
                S::Continue => return Flow::Continue,
                S::Return { value } => {
                    let value = value.map(|at| self.value(frame, at));
                    frame.arguments = value.into_iter().collect();
                    return Flow::Return;
                }
                S::Kill => {
                    self.out.killed = true;
                    return Flow::Return;
                }
                S::Store { pointer, value } => {
                    let value = self.value(frame, value);
                    match self.got(frame, pointer) {
                        Got::Place(place) => self.write(frame, &place, value),
                        Got::Value(value) => panic!("a store to {value:?}"),
                    }
                }
                S::ControlBarrier(_) => {
                    self.out.barriers += 1;
                    match &self.group.baton {
                        Some(baton) => {
                            baton.pass(self.turn.0, false);
                            self.turn.1 += 1;
                            baton.take(self.turn.0, self.turn.1);
                        }
                        None => {
                            self.group.barrier.wait();
                        }
                    }
                }
                S::Atomic {
                    pointer,
                    fun,
                    value,
                    result,
                } => {
                    use naga::AtomicFunction as Af;
                    let value = self.value(frame, value).word();
                    let Got::Place(Place::Global(global, path)) = self.got(frame, pointer) else {
                        panic!("an update in one step of what is no memory of the group");
                    };
                    let old = self.memory(global, |whole| {
                        let cell = whole.at(&path);
                        let old = cell.word();
                        *cell = Value::U(match fun {
                            Af::Add => old.wrapping_add(value),
                            Af::Subtract => old.wrapping_sub(value),
                            Af::And => old & value,
                            Af::InclusiveOr => old | value,
                            Af::ExclusiveOr => old ^ value,
                            Af::Min => old.min(value),
                            Af::Max => old.max(value),
                            Af::Exchange { compare: None } => value,
                            other => panic!("{other:?} is not run"),
                        });
                        old
                    });
                    if let Some(at) = result {
                        frame.values.insert(at, Got::Value(Value::U(old)));
                    }
                }
                S::Call {
                    function,
                    ref arguments,
                    result,
                } => {
                    let arguments = arguments.iter().map(|&a| self.value(frame, a)).collect();
                    let returned = self.call(&self.module.functions[function], arguments);
                    if let (Some(at), Some(value)) = (result, returned) {
                        frame.values.insert(at, Got::Value(value));
                    }
                }
                S::ImageStore {
                    image,
                    coordinate,
                    array_index,
                    value,
                } => {
                    let Got::Place(place) = self.got(frame, image) else {
                        panic!("a texel stored to no texture");
                    };
                    let mut stored = [vec![self.read(frame, &place).word()], vec![], vec![]];
                    for at in [Some(coordinate), array_index].into_iter().flatten() {
                        self.value(frame, at).words(&mut stored[1]);
                    }
                    self.value(frame, value).words(&mut stored[2]);
                    self.out.stores.push(stored);
                }
                S::Atomic {
                    pointer,
                    ref fun,
                    value,
                    result,
                } => {
                    use naga::AtomicFunction as Af;
                    let value = self.value(frame, value).word();
                    let Got::Place(place) = self.got(frame, pointer) else {
                        panic!("an update in one step of what is no memory");
                    };
                    let old = self.read(frame, &place).word();
                    let new = match fun {
                        Af::Add => old.wrapping_add(value),
                        Af::Subtract => old.wrapping_sub(value),
                        Af::And => old & value,
                        Af::InclusiveOr => old | value,
                        Af::ExclusiveOr => old ^ value,
                        Af::Min => old.min(value),
                        Af::Max => old.max(value),
                        Af::Exchange { compare: None } => value,
                        other => panic!("{other:?} is not run"),
                    };
                    self.write(frame, &place, Value::U(new));
                    if let Some(at) = result {
                        frame.values.insert(at, Got::Value(Value::U(old)));
                    }
                }
                ref other => panic!("{other:?} is not run"),
            }
        }
        Flow::Next
    }

    /// Runs a function; what it returned.
    fn call(&mut self, fun: &'a naga::Function, arguments: Vec<Value>) -> Option<Value> {
        let mut frame = Frame {
            fun,
            arguments,
            locals: HashMap::new(),
            values: HashMap::new(),
        };
        let returned = match self.run(&mut frame, &fun.body) {
            // A return leaves its value where the arguments were.
            Flow::Return if !self.out.killed => frame.arguments.pop(),
            _ => None,
        };
        // The entry point is the last to return.
        self.out.result.clone_from(&returned);
        let function = std::ptr::from_ref(fun) as usize;
        let emitted = (frame.values.keys()).filter(|&&at| {
            !matches!(
                fun.expressions[at],
                E::CallResult(_) | E::AtomicResult { .. }
            )
        });
        (self.out.reached).extend(emitted.map(|at| (function, at.index())));
        if fun.name.as_deref() == Some("main_body") {
            // The registers of the body's last run: an earlier run's are another thread's.
            self.out.registers.clear();
            for (local, variable) in fun.local_variables.iter() {
                let (Some(name), Some(value)) = (&variable.name, frame.locals.get(&local)) else {
                    continue;
                };
                if !matches!(value, Value::List(_)) {
                    self.out.registers.insert(name.clone(), value.word());
                }
            }
        }
        returned
    }
}

fn binary(op: Bo, l: Value, r: Value) -> Value {
    use Value::{B, F, I, U};
    match (op, l, r) {
        (Bo::Add, U(a), U(b)) => U(a.wrapping_add(b)),
        (Bo::Subtract, U(a), U(b)) => U(a.wrapping_sub(b)),
        (Bo::Multiply, U(a), U(b)) => U(a.wrapping_mul(b)),
        (Bo::Divide, U(a), U(b)) => U(a.checked_div(b).unwrap_or(a)),
        (Bo::Modulo, U(a), U(b)) => U(a.checked_rem(b).unwrap_or(0)),
        (Bo::And, U(a), U(b)) => U(a & b),
        (Bo::InclusiveOr, U(a), U(b)) => U(a | b),
        (Bo::ExclusiveOr, U(a), U(b)) => U(a ^ b),
        (Bo::ShiftLeft, U(a), U(b)) => U(a.checked_shl(b).expect("a shift past a dword")),
        (Bo::ShiftRight, U(a), U(b)) => U(a.checked_shr(b).expect("a shift past a dword")),
        (Bo::ShiftRight, I(a), U(b)) => I(a.checked_shr(b).expect("a shift past a dword")),
        (Bo::Add, I(a), I(b)) => I(a.wrapping_add(b)),
        (Bo::Subtract, I(a), I(b)) => I(a.wrapping_sub(b)),
        (Bo::Add, F(a), F(b)) => F(a + b),
        (Bo::Subtract, F(a), F(b)) => F(a - b),
        (Bo::Multiply, F(a), F(b)) => F(a * b),
        (Bo::Divide, F(a), F(b)) => F(a / b),
        (Bo::Equal, a, b) => B(a == b),
        (Bo::NotEqual, a, b) => B(a != b),
        (Bo::Less, U(a), U(b)) => B(a < b),
        (Bo::LessEqual, U(a), U(b)) => B(a <= b),
        (Bo::Greater, U(a), U(b)) => B(a > b),
        (Bo::GreaterEqual, U(a), U(b)) => B(a >= b),
        (Bo::Less, I(a), I(b)) => B(a < b),
        (Bo::LessEqual, I(a), I(b)) => B(a <= b),
        (Bo::Greater, I(a), I(b)) => B(a > b),
        (Bo::GreaterEqual, I(a), I(b)) => B(a >= b),
        (Bo::Less, F(a), F(b)) => B(a < b),
        (Bo::LessEqual, F(a), F(b)) => B(a <= b),
        (Bo::Greater, F(a), F(b)) => B(a > b),
        (Bo::GreaterEqual, F(a), F(b)) => B(a >= b),
        (Bo::LogicalAnd, B(a), B(b)) => B(a && b),
        (Bo::LogicalOr, B(a), B(b)) => B(a || b),
        (Bo::And, B(a), B(b)) => B(a & b),
        (Bo::InclusiveOr, B(a), B(b)) => B(a | b),
        (Bo::ExclusiveOr, B(a), B(b)) => B(a ^ b),
        (Bo::Multiply, I(a), I(b)) => I(a.wrapping_mul(b)),
        (Bo::Divide, I(a), I(b)) => I(a.checked_div(b).unwrap_or(a)),
        (Bo::Modulo, I(a), I(b)) => I(a.checked_rem(b).unwrap_or(0)),
        (Bo::And, I(a), I(b)) => I(a & b),
        (Bo::InclusiveOr, I(a), I(b)) => I(a | b),
        (Bo::ExclusiveOr, I(a), I(b)) => I(a ^ b),
        (Bo::ShiftLeft, I(a), U(b)) => I(a.checked_shl(b).expect("a shift past a dword")),
        (Bo::Modulo, F(a), F(b)) => F(a % b),
        // A vector's components, each with the other's.
        (op, Value::List(a), Value::List(b)) => Value::List(
            (a.into_iter().zip(b))
                .map(|(a, b)| binary(op, a, b))
                .collect(),
        ),
        other => panic!("{other:?} is not run"),
    }
}

fn math(fun: Mf, a: Value, b: Option<Value>, c: Option<Value>) -> Value {
    use Value::{F, I, U};
    match (fun, a, b, c) {
        (Mf::Min, U(a), Some(U(b)), _) => U(a.min(b)),
        (Mf::Max, U(a), Some(U(b)), _) => U(a.max(b)),
        (Mf::Min, I(a), Some(I(b)), _) => I(a.min(b)),
        (Mf::Max, I(a), Some(I(b)), _) => I(a.max(b)),
        (Mf::Min, F(a), Some(F(b)), _) => F(a.min(b)),
        (Mf::Max, F(a), Some(F(b)), _) => F(a.max(b)),
        (Mf::Clamp, U(a), Some(U(low)), Some(U(high))) => U(a.clamp(low, high)),
        (Mf::CountLeadingZeros, U(a), ..) => U(a.leading_zeros()),
        (Mf::CountOneBits, U(a), ..) => U(a.count_ones()),
        (Mf::ReverseBits, U(a), ..) => U(a.reverse_bits()),
        (Mf::FirstTrailingBit, U(a), ..) => U(match a {
            0 => u32::MAX,
            _ => a.trailing_zeros(),
        }),
        (Mf::Abs, F(a), ..) => F(a.abs()),
        (Mf::Sqrt, F(a), ..) => F(a.sqrt()),
        (Mf::InverseSqrt, F(a), ..) => F(1.0 / a.sqrt()),
        (Mf::Saturate, F(a), ..) => F(a.clamp(0.0, 1.0)),
        (Mf::Fma, F(a), Some(F(b)), Some(F(c))) => F(a.mul_add(b, c)),
        (Mf::Clamp, F(a), Some(F(low)), Some(F(high))) => F(a.max(low).min(high)),
        (Mf::Clamp, I(a), Some(I(low)), Some(I(high))) => I(a.max(low).min(high)),
        (Mf::Abs, I(a), ..) => I(a.wrapping_abs()),
        (Mf::Floor, F(a), ..) => F(a.floor()),
        (Mf::Ceil, F(a), ..) => F(a.ceil()),
        (Mf::Trunc, F(a), ..) => F(a.trunc()),
        (Mf::Fract, F(a), ..) => F(a - a.floor()),
        (Mf::Round, F(a), ..) => F(a.round_ties_even()),
        (Mf::Sin, F(a), ..) => F(a.sin()),
        (Mf::Cos, F(a), ..) => F(a.cos()),
        (Mf::Exp2, F(a), ..) => F(a.exp2()),
        (Mf::Log2, F(a), ..) => F(a.log2()),
        other => panic!("{other:?} is not run"),
    }
}

/// A buffer of dwords bound to a module, by its variable's name.
pub type Bound<'a> = (&'a str, &'a [u32]);

/// The memory a group starts with: each buffer bound, as its variable's value.
fn bound(module: &naga::Module, buffers: &[Bound]) -> HashMap<Handle<GlobalVariable>, Value> {
    (buffers.iter())
        .map(|(name, words)| {
            let (global, _) = (module.global_variables.iter())
                .find(|(_, variable)| variable.name.as_deref() == Some(name))
                .unwrap_or_else(|| panic!("the module binds no {name}"));
            (
                global,
                Value::List(words.iter().map(|&word| Value::U(word)).collect()),
            )
        })
        .collect()
}

/// A module with its buffers bound, for calls of one of its functions on this thread.
pub struct Calls<'a> {
    module: &'a naga::Module,
    group: Group,
}

impl<'a> Calls<'a> {
    pub fn new(module: &'a naga::Module, buffers: &[Bound]) -> Self {
        let group = Group {
            memory: Mutex::new(bound(module, buffers)),
            barrier: Barrier::new(1),
            baton: None,
        };
        Self { module, group }
    }

    /// What the function named `name` returns for `arguments`.
    pub fn call(&self, name: &str, arguments: Vec<Value>) -> Value {
        let (_, function) = (self.module.functions.iter())
            .find(|(_, function)| function.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("no function {name}"));
        let mut machine = Machine {
            module: self.module,
            group: &self.group,
            private: HashMap::new(),
            out: Thread::default(),
            turn: (0, 0),
        };
        let returned = machine.call(function, arguments);
        returned.unwrap_or_else(|| panic!("{name} returned nothing"))
    }
}

/// Runs the module's entry point once for each thread of a group of `threads`, all at once;
/// `given` is an argument's value by its binding, where the stage's own numbers do not say it.
pub fn run(
    module: &naga::Module,
    threads: [u32; 3],
    given: &(dyn Fn(u32, &naga::Binding) -> Option<Value> + Sync),
) -> Vec<Thread> {
    run_bound(module, threads, given, &[]).0
}

/// [`run`] with `buffers` bound; what each of them holds after it, in their order.
pub fn run_bound(
    module: &naga::Module,
    threads: [u32; 3],
    given: &(dyn Fn(u32, &naga::Binding) -> Option<Value> + Sync),
    buffers: &[Bound],
) -> (Vec<Thread>, Vec<Vec<u32>>) {
    run_in_order(module, threads, given, buffers, None)
}

/// [`run_bound`] with the threads run one at a time where `order` names them all: each from
/// one barrier to its next in that order, round after round.
pub fn run_in_order(
    module: &naga::Module,
    threads: [u32; 3],
    given: &(dyn Fn(u32, &naga::Binding) -> Option<Value> + Sync),
    buffers: &[Bound],
    order: Option<&[u32]>,
) -> (Vec<Thread>, Vec<Vec<u32>>) {
    let count = threads.iter().product::<u32>();
    let memory = bound(module, buffers);
    let handles: Vec<Handle<GlobalVariable>> = (buffers.iter())
        .map(|(name, _)| {
            let found = (module.global_variables.iter())
                .find(|(_, variable)| variable.name.as_deref() == Some(name));
            found.expect("a buffer that was bound").0
        })
        .collect();
    let group = Group {
        memory: Mutex::new(memory),
        barrier: Barrier::new(count as usize),
        baton: order.map(|order| Baton::new(order.len())),
    };
    let place = |thread: u32| {
        let found = order.and_then(|order| order.iter().position(|named| *named == thread));
        found.unwrap_or(thread as usize)
    };
    let entry = &module.entry_points[0];
    let one = |thread: u32| {
        let id = [
            thread % threads[0],
            thread / threads[0] % threads[1],
            thread / (threads[0] * threads[1]),
        ];
        let arguments = (entry.function.arguments.iter())
            .map(|argument| {
                let binding = argument
                    .binding
                    .as_ref()
                    .expect("an argument with no binding");
                given(thread, binding).unwrap_or_else(|| match binding {
                    naga::Binding::BuiltIn(naga::BuiltIn::LocalInvocationId) => {
                        Value::List(id.map(Value::U).to_vec())
                    }
                    _ => zero(module, argument.ty),
                })
            })
            .collect();
        let mut machine = Machine {
            module,
            group: &group,
            private: HashMap::new(),
            out: Thread::default(),
            turn: (place(thread), 0),
        };
        if let Some(baton) = &group.baton {
            baton.take(machine.turn.0, 0);
        }
        machine.call(&entry.function, arguments);
        if let Some(baton) = &group.baton {
            baton.pass(machine.turn.0, true);
        }
        machine.out
    };
    let ran = std::thread::scope(|scope| {
        let running: Vec<_> = (0..count)
            .map(|thread| scope.spawn(move || one(thread)))
            .collect();
        (running.into_iter())
            .map(|thread| thread.join().expect("a thread that stopped"))
            .collect()
    });
    let memory = group.memory.lock().unwrap();
    let left = (handles.iter())
        .map(|global| match memory.get(global) {
            Some(Value::List(words)) => words.iter().map(Value::word).collect(),
            _ => Vec::new(),
        })
        .collect();
    (ran, left)
}

/// What a constant expression of the module comes to.
fn constant(module: &naga::Module, at: Handle<E>) -> Value {
    match module.global_expressions[at] {
        E::Literal(Literal::U32(word)) => Value::U(word),
        E::Literal(Literal::I32(word)) => Value::I(word),
        E::Literal(Literal::F32(float)) => Value::F(float),
        E::Literal(Literal::Bool(flag)) => Value::B(flag),
        E::Constant(named) => constant(module, module.constants[named].init),
        E::ZeroValue(ty) => zero(module, ty),
        E::Compose { ref components, .. } => {
            Value::List(components.iter().map(|&c| constant(module, c)).collect())
        }
        ref other => panic!("the constant {other:?} is not worked out"),
    }
}

/// How many elements `whole` holds, and one of them with every dword nought.
fn span(whole: &Value) -> (usize, Value) {
    match whole {
        Value::List(parts) => (
            parts.len(),
            parts.first().map_or(Value::U(0), Value::nought),
        ),
        other => panic!("{other:?} has no elements"),
    }
}

/// Every bit of `x` in every bit of the result.
fn mix(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Four floats in [0, 1) that the dwords of `seen`, in their order, decide.
fn texel(seen: &[u32]) -> [f32; 4] {
    let all = (seen.iter()).fold(0, |all, &word| mix(all ^ u64::from(word)).wrapping_add(1));
    [1, 2, 3, 4].map(|part| (mix(all ^ part << 56) >> 40) as f32 / 16_777_216.0)
}

/// The kind of a texel of the texture at `place`; `None` for a depth, which is one float.
fn texel_kind(module: &naga::Module, place: &Place) -> Option<ScalarKind> {
    use naga::ImageClass as Class;
    let Place::Global(global, _) = place else {
        return Some(ScalarKind::Float);
    };
    let mut inner = &module.types[module.global_variables[*global].ty].inner;
    if let TypeInner::BindingArray { base, .. } = *inner {
        inner = &module.types[base].inner;
    }
    match *inner {
        TypeInner::Image { class, .. } => match class {
            Class::Depth { .. } => None,
            Class::Sampled { kind, .. } => Some(kind),
            Class::Storage { format, .. } => Some(naga::Scalar::from(format).kind),
            Class::External => Some(ScalarKind::Float),
        },
        _ => Some(ScalarKind::Float),
    }
}

/// A dword of a bound input. A loop counted to a dword ends soon where the dword is a small
/// count, whether it is read as one or as a float, and may not where it is a float read as a
/// count: so a seed is one of three kinds. The first holds small counts alone, the second a
/// float in one dword of four, and the third floats a shader meets, odd ones and any pattern.
fn dword(hash: u64, seed: u64) -> u32 {
    let any = (hash >> 32) as u32;
    let unit = (any >> 8) as f32 / 16_777_216.0;
    // Nought, one, 1.0, the sign alone, every bit, a quiet NaN, infinity, a NaN with a payload.
    const ODD: [u32; 8] = [
        0,
        1,
        0x3F80_0000,
        0x8000_0000,
        u32::MAX,
        0x7FC0_0000,
        0x7F80_0000,
        0x7FA0_0001,
    ];
    match (seed % 3, hash % 16) {
        (0, _) | (1, 0..=11) | (_, 0..=5) => any % 9,
        (1, _) => (unit * 8.0 - 4.0).to_bits(),
        (_, 6..=9) => (unit * 8.0 - 4.0).to_bits(),
        (_, 10..=12) => unit.to_bits(),
        (_, 13) => ODD[(any % 8) as usize],
        _ => any,
    }
}

/// An array bound without a length holds this many elements.
const UNSIZED: usize = 512;

/// A value of type `ty` with each dword decided by `seed` and the next of `hashes`.
fn filled(
    module: &naga::Module,
    ty: Handle<naga::Type>,
    seed: u64,
    hashes: &mut dyn FnMut() -> u64,
) -> Value {
    let scalar = |kind: ScalarKind, word: u32| match kind {
        ScalarKind::Float => Value::F(f32::from_bits(word)),
        ScalarKind::Sint => Value::I(word as i32),
        ScalarKind::Bool => Value::B(word & 1 != 0),
        _ => Value::U(word),
    };
    let mut several = |count: usize, base: Handle<naga::Type>| {
        Value::List(
            (0..count)
                .map(|_| filled(module, base, seed, hashes))
                .collect(),
        )
    };
    match module.types[ty].inner {
        TypeInner::Scalar(of) | TypeInner::Atomic(of) => scalar(of.kind, dword(hashes(), seed)),
        TypeInner::Vector { size, scalar: of } => Value::List(
            (0..size as usize)
                .map(|_| scalar(of.kind, dword(hashes(), seed)))
                .collect(),
        ),
        TypeInner::Array {
            base,
            size: naga::ArraySize::Constant(count),
            ..
        }
        | TypeInner::BindingArray {
            base,
            size: naga::ArraySize::Constant(count),
        } => several(count.get() as usize, base),
        TypeInner::Array { base, .. } => several(UNSIZED, base),
        TypeInner::Struct { ref members, .. } => Value::List(
            members
                .iter()
                .map(|m| filled(module, m.ty, seed, hashes))
                .collect(),
        ),
        // A texture or a sampler is a number of its own, and never the nought of one not bound.
        TypeInner::Image { .. } | TypeInner::Sampler { .. } => Value::U(hashes() as u32 | 1),
        ref other => panic!("no value of {other:?}"),
    }
}

/// Whether a statement under `block` is a barrier the threads of a group meet at.
fn meets(block: &naga::Block) -> bool {
    block.iter().any(|statement| match statement {
        S::ControlBarrier(_) => true,
        S::Block(inner) => meets(inner),
        S::If { accept, reject, .. } => meets(accept) || meets(reject),
        S::Switch { cases, .. } => cases.iter().any(|case| meets(&case.body)),
        S::Loop {
            body, continuing, ..
        } => meets(body) || meets(continuing),
        _ => false,
    })
}

/// What a whole run left.
pub struct Whole {
    pub threads: Vec<Thread>,
    /// At the end, the memory the group shares and each bound buffer the module may store
    /// to, in the module's order: the variable and its dwords.
    pub memory: Vec<(String, Vec<u32>)>,
}

/// Runs the module's entry point for each thread of its group, one thread after another, with
/// every bound buffer, texture, sampler and argument holding what `seed` gives it, and at most
/// `most` expressions a thread. It stops, as a test's own run does, on what it does not run,
/// and on a group whose threads meet at a barrier, which one after another they cannot.
pub fn run_whole(module: &naga::Module, seed: u64, most: u64) -> Whole {
    use naga::AddressSpace as Space;
    use naga::BuiltIn as Given;
    let entry = &module.entry_points[0];
    let mut functions =
        (module.functions.iter().map(|(_, function)| function)).chain([&entry.function]);
    assert!(
        !functions.any(|function| meets(&function.body)),
        "the threads of its group meet at a barrier"
    );
    let threads = match entry.stage {
        naga::ShaderStage::Compute => entry.workgroup_size,
        _ => [1; 3],
    };
    let filled_by = |stream: u64, ty| {
        let mut at = 0u64;
        filled(module, ty, seed, &mut || {
            at += 1;
            mix(stream ^ at)
        })
    };
    let memory = (module.global_variables.iter())
        .filter(|(_, variable)| {
            matches!(
                variable.space,
                Space::Uniform | Space::Storage { .. } | Space::Handle
            )
        })
        .map(|(global, variable)| {
            let stream = mix(seed ^ mix(global.index() as u64));
            (global, filled_by(stream, variable.ty))
        })
        .collect();
    let group = Group {
        memory: Mutex::new(memory),
        barrier: Barrier::new(1),
    };
    let ran = (0..threads.iter().product::<u32>())
        .map(|thread| {
            let id = [
                thread % threads[0],
                thread / threads[0] % threads[1],
                thread / (threads[0] * threads[1]),
            ];
            let arguments = (entry.function.arguments.iter().enumerate())
                .map(|(index, argument)| match &argument.binding {
                    Some(naga::Binding::BuiltIn(
                        Given::LocalInvocationId | Given::GlobalInvocationId,
                    )) => Value::List(id.map(Value::U).to_vec()),
                    Some(naga::Binding::BuiltIn(Given::LocalInvocationIndex)) => Value::U(thread),
                    // The first group: a thread's place in the whole launch is then its own.
                    Some(naga::Binding::BuiltIn(Given::WorkGroupId)) => {
                        Value::List(vec![Value::U(0); 3])
                    }
                    _ => {
                        let stream = mix(!seed ^ mix(u64::from(thread) << 32 | index as u64));
                        filled_by(stream, argument.ty)
                    }
                })
                .collect();
            let mut machine = Machine {
                module,
                group: &group,
                private: HashMap::new(),
                out: Thread {
                    bounded: Some(most),
                    ..Thread::default()
                },
            };
            machine.call(&entry.function, arguments);
            machine.out
        })
        .collect();
    let left = group.memory.into_inner().unwrap();
    let memory = (module.global_variables.iter())
        .filter(|(_, variable)| match variable.space {
            Space::WorkGroup => true,
            Space::Storage { access } => access.contains(naga::StorageAccess::STORE),
            _ => false,
        })
        .filter_map(|(global, variable)| {
            let mut words = Vec::new();
            left.get(&global)?.words(&mut words);
            let name = variable.name.clone().unwrap_or_default();
            Some((format!("{} {name}", global.index()), words))
        })
        .collect();
    Whole {
        threads: ran,
        memory,
    }
}
