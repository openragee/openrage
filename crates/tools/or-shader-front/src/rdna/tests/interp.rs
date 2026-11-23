//! A translated module run here, a thread of this process for each thread of its group: what
//! a test compares with what the instructions' definitions give. It runs what the lowering of
//! arithmetic, registers, branches, shared memory and barriers writes, and stops on the rest.

use std::collections::HashMap;
use std::sync::{Barrier, Mutex};

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
}

/// What a thread left: the registers of its body by name, and whether it was discarded.
#[derive(Debug, Default)]
pub struct Thread {
    pub registers: HashMap<String, u32>,
    pub killed: bool,
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
        TypeInner::Struct { ref members, .. } => {
            Value::List(members.iter().map(|m| zero(module, m.ty)).collect())
        }
        ref other => panic!("no zero of {other:?}"),
    }
}

impl<'a> Machine<'a> {
    fn memory<R>(&mut self, global: Handle<GlobalVariable>, f: impl FnOnce(&mut Value) -> R) -> R {
        let variable = &self.module.global_variables[global];
        let fresh = || zero(self.module, variable.ty);
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
            Place::Local(local, path) => {
                let ty = frame.fun.local_variables[*local].ty;
                let whole = (frame.locals.entry(*local)).or_insert_with(|| zero(self.module, ty));
                whole.at(path).clone()
            }
            Place::Global(global, path) => self.memory(*global, |whole| whole.at(path).clone()),
        }
    }

    fn write(&mut self, frame: &mut Frame<'a>, place: &Place, value: Value) {
        match place {
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

    fn element(&mut self, frame: &mut Frame<'a>, base: Handle<E>, index: u32) -> Got {
        match self.got(frame, base) {
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
                    }
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
                    self.group.barrier.wait();
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
        (Mf::FirstTrailingBit, U(a), ..) => U(match a {
            0 => u32::MAX,
            _ => a.trailing_zeros(),
        }),
        (Mf::Abs, F(a), ..) => F(a.abs()),
        (Mf::Sqrt, F(a), ..) => F(a.sqrt()),
        (Mf::InverseSqrt, F(a), ..) => F(1.0 / a.sqrt()),
        (Mf::Saturate, F(a), ..) => F(a.clamp(0.0, 1.0)),
        other => panic!("{other:?} is not run"),
    }
}

/// Runs the module's entry point once for each thread of a group of `threads`, all at once;
/// `given` is an argument's value by its binding, where the stage's own numbers do not say it.
pub fn run(
    module: &naga::Module,
    threads: [u32; 3],
    given: &(dyn Fn(u32, &naga::Binding) -> Option<Value> + Sync),
) -> Vec<Thread> {
    let count = threads.iter().product::<u32>();
    let group = Group {
        memory: Mutex::new(HashMap::new()),
        barrier: Barrier::new(count as usize),
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
        };
        machine.call(&entry.function, arguments);
        machine.out
    };
    std::thread::scope(|scope| {
        let running: Vec<_> = (0..count)
            .map(|thread| scope.spawn(move || one(thread)))
            .collect();
        (running.into_iter())
            .map(|thread| thread.join().expect("a thread that stopped"))
            .collect()
    })
}
