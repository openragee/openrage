//! Modules written by hand for what no program assembled here reaches in a pass: a call that
//! stores a register, a `switch` whose case falls through, a loop that narrows a mask, a buffer
//! read back after a store to it. Each is one thread that reads `inp` and writes `out`, run as
//! written and after the optimiser on the same seeds.

use naga::{
    AddressSpace, BinaryOperator as Bo, Block, Expression as E, Function, GlobalVariable, Handle,
    Literal, LocalVariable, Span, Statement as S, StorageAccess, SwitchCase, SwitchValue, Type,
    TypeInner,
};

use super::equal::{compare, SEEDS};
use super::{check, interp};
use crate::rdna::opt;

/// A module being written: `inp` of eight dwords to read, `out` of four to write, `shared` of
/// four that are the group's memory, and the registers of the thread's own that every function
/// sees.
struct Hand {
    module: naga::Module,
    uint: Handle<Type>,
    inp: Handle<GlobalVariable>,
    out: Handle<GlobalVariable>,
    shared: Handle<GlobalVariable>,
    private: Vec<Handle<GlobalVariable>>,
}

/// A function being written. An expression is emitted where it is made.
struct Body {
    function: Function,
    /// The blocks being written, the innermost last.
    open: Vec<Block>,
    uint: Handle<Type>,
    inp: Handle<GlobalVariable>,
    out: Handle<GlobalVariable>,
    shared: Handle<GlobalVariable>,
    private: Vec<Handle<GlobalVariable>>,
}

impl Hand {
    fn new(private: &[&str]) -> Self {
        let mut module = naga::Module::default();
        let mut ty = |inner| {
            module
                .types
                .insert(Type { name: None, inner }, Span::UNDEFINED)
        };
        let uint = ty(TypeInner::Scalar(naga::Scalar::U32));
        let mut dwords = |count| {
            ty(TypeInner::Array {
                base: uint,
                size: naga::ArraySize::Constant(std::num::NonZeroU32::new(count).unwrap()),
                stride: 4,
            })
        };
        let (eight, four) = (dwords(8), dwords(4));
        let mut global = |name: &str, space, ty, bound: bool| {
            let binding = module.global_variables.len() as u32;
            let variable = GlobalVariable {
                name: Some(name.to_owned()),
                space,
                binding: bound.then_some(naga::ResourceBinding { group: 0, binding }),
                ty,
                init: None,
                memory_decorations: Default::default(),
            };
            module.global_variables.append(variable, Span::UNDEFINED)
        };
        let buffer = |access| AddressSpace::Storage { access };
        let inp = global("inp", buffer(StorageAccess::LOAD), eight, true);
        let both = StorageAccess::LOAD | StorageAccess::STORE;
        let out = global("out", buffer(both), four, true);
        let shared = global("shared", AddressSpace::WorkGroup, four, false);
        let private = (private.iter())
            .map(|name| global(name, AddressSpace::Private, uint, false))
            .collect();
        Self {
            module,
            uint,
            inp,
            out,
            shared,
            private,
        }
    }

    fn body(&self, write: impl FnOnce(&mut Body)) -> Function {
        let mut body = Body {
            function: Function::default(),
            open: vec![Block::new()],
            uint: self.uint,
            inp: self.inp,
            out: self.out,
            shared: self.shared,
            private: self.private.clone(),
        };
        write(&mut body);
        body.function.body = body.open.pop().unwrap();
        body.function
    }

    /// A function of no arguments that returns nothing.
    fn function(&mut self, write: impl FnOnce(&mut Body)) -> Handle<Function> {
        let function = self.body(write);
        self.module.functions.append(function, Span::UNDEFINED)
    }

    /// The module with its entry point, which the validator takes.
    fn entry(mut self, write: impl FnOnce(&mut Body)) -> naga::Module {
        let function = self.body(write);
        self.module.entry_points.push(naga::EntryPoint {
            name: "main".to_owned(),
            stage: naga::ShaderStage::Compute,
            early_depth_test: None,
            workgroup_size: [1; 3],
            workgroup_size_overrides: None,
            function,
            mesh_info: None,
            task_payload: None,
            incoming_ray_payload: None,
        });
        check(&self.module).unwrap();
        self.module
    }
}

impl Body {
    fn push(&mut self, statement: S) {
        let block = self.open.last_mut().unwrap();
        block.push(statement, Span::UNDEFINED);
    }

    fn e(&mut self, expression: E) -> Handle<E> {
        let emits = !expression.needs_pre_emit();
        let before = self.function.expressions.len();
        let at = (self.function.expressions).append(expression, Span::UNDEFINED);
        if emits {
            self.push(S::Emit(self.function.expressions.range_from(before)));
        }
        at
    }

    fn word(&mut self, word: u32) -> Handle<E> {
        self.e(E::Literal(Literal::U32(word)))
    }

    /// The register of the thread's own that [`Hand::new`] named at `place`.
    fn private(&mut self, place: usize) -> Handle<E> {
        self.e(E::GlobalVariable(self.private[place]))
    }

    /// A register of this function's own, which starts as `starts` or as nought.
    fn local(&mut self, name: &str, starts: Option<u32>) -> Handle<E> {
        let init = starts.map(|word| self.word(word));
        let variable = LocalVariable {
            name: Some(name.to_owned()),
            ty: self.uint,
            init,
        };
        let local = (self.function.local_variables).append(variable, Span::UNDEFINED);
        self.e(E::LocalVariable(local))
    }

    fn load(&mut self, pointer: Handle<E>) -> Handle<E> {
        self.e(E::Load { pointer })
    }

    fn store(&mut self, pointer: Handle<E>, value: Handle<E>) {
        self.push(S::Store { pointer, value });
    }

    fn op(&mut self, op: Bo, left: Handle<E>, right: Handle<E>) -> Handle<E> {
        self.e(E::Binary { op, left, right })
    }

    /// Dword `index` of the buffer read.
    fn inp(&mut self, index: u32) -> Handle<E> {
        let base = self.e(E::GlobalVariable(self.inp));
        let pointer = self.e(E::AccessIndex { base, index });
        self.load(pointer)
    }

    /// Where dword `index` of the buffer written is.
    fn out(&mut self, index: u32) -> Handle<E> {
        let base = self.e(E::GlobalVariable(self.out));
        self.e(E::AccessIndex { base, index })
    }

    /// Where dword `index` of the group's memory is.
    fn shared(&mut self, index: u32) -> Handle<E> {
        let base = self.e(E::GlobalVariable(self.shared));
        self.e(E::AccessIndex { base, index })
    }

    /// `out[index] = word`.
    fn says(&mut self, index: u32, word: u32) {
        let (pointer, value) = (self.out(index), self.word(word));
        self.store(pointer, value);
    }

    /// Whether what `pointer` holds is not nought.
    fn set(&mut self, pointer: Handle<E>) -> Handle<E> {
        let (held, nought) = (self.load(pointer), self.word(0));
        self.op(Bo::NotEqual, held, nought)
    }

    fn block(&mut self, write: impl FnOnce(&mut Self)) -> Block {
        self.open.push(Block::new());
        write(self);
        self.open.pop().unwrap()
    }

    fn iff(
        &mut self,
        condition: Handle<E>,
        accept: impl FnOnce(&mut Self),
        reject: impl FnOnce(&mut Self),
    ) {
        let (accept, reject) = (self.block(accept), self.block(reject));
        self.push(S::If {
            condition,
            accept,
            reject,
        });
    }

    fn call(&mut self, function: Handle<Function>) {
        self.push(S::Call {
            function,
            arguments: Vec::new(),
            result: None,
        });
    }
}

/// `out` as `module` leaves it on each seed.
fn outs(module: &naga::Module) -> Vec<Vec<u32>> {
    SEEDS
        .map(|seed| interp::run_whole(module, seed, 1 << 16).memory.remove(0).1)
        .collect()
}

/// `module` after the optimiser, which the validator takes, a second run leaves as it is, and
/// every seed finds equal to `module`; and what the optimiser changed.
fn passed(module: &naga::Module) -> (naga::Module, opt::Changed) {
    let mut passed = module.clone();
    let changed = opt::optimise(&mut passed, &[]);
    check(&passed).unwrap();
    let compared = compare(module, &passed, SEEDS).unwrap();
    assert_eq!(u64::from(compared.equal), SEEDS.end);
    let again = passed.clone();
    assert_eq!(opt::optimise(&mut passed, &[]), opt::Changed::default());
    assert_eq!(format!("{passed:?}"), format!("{again:?}"));
    (passed, changed)
}

/// The loads the entry point of `module` computes.
fn loads(module: &naga::Module) -> usize {
    (module.entry_points[0].function.expressions.iter())
        .filter(|(_, expression)| matches!(expression, E::Load { .. }))
        .count()
}

/// `m` = 1; of `inp[0]`'s low two bits, nought stores nought to `m` and falls through to one,
/// which says in `out[0]` whether `m` is set: 7, or 9. `out[1]` is the two bits, `out[2]` what
/// a local that starts as 5 holds, and `out[0]` is 5 where neither case ran.
fn falls_through() -> naga::Module {
    Hand::new(&[]).entry(|f| {
        f.says(0, 5);
        let (m, five) = (f.local("m", None), f.local("five", Some(5)));
        let one = f.word(1);
        f.store(m, one);
        let (bits, three) = (f.inp(0), f.word(3));
        let selector = f.op(Bo::And, bits, three);
        let place = f.out(1);
        f.store(place, selector);
        let clears = f.block(|f| {
            let nought = f.word(0);
            f.store(m, nought);
        });
        let tests = f.block(|f| {
            let set = f.set(m);
            f.iff(set, |f| f.says(0, 7), |f| f.says(0, 9));
        });
        let case = |value, body, fall_through| SwitchCase {
            value,
            body,
            fall_through,
        };
        f.push(S::Switch {
            selector,
            cases: vec![
                // The default first: a case is found by its value before it.
                case(SwitchValue::Default, Block::new(), false),
                case(SwitchValue::U32(0), clears, true),
                case(SwitchValue::U32(1), tests, false),
            ],
        });
        let (place, held) = (f.out(2), f.load(five));
        f.store(place, held);
    })
}

#[test]
fn a_case_that_falls_through_runs_the_next_and_a_local_starts_as_its_initialiser() {
    let module = falls_through();
    let left = outs(&module);
    for out in &left {
        let said = match out[1] {
            0 => 9,
            1 => 7,
            _ => 5,
        };
        assert_eq!(out[..3], [said, out[1], 5]);
    }
    // Each way through the `switch` was taken on some seed.
    for bits in 0..3 {
        assert!(left.iter().any(|out| out[1] == bits), "{bits}: {left:?}");
    }
    // The constants forget `m` where a case may be entered from the one before it, the
    // forwarding reads no load of it from before the `switch`, and the store that falls
    // through is read by the next case.
    passed(&module);
}

#[test]
fn a_load_after_a_call_or_a_switch_reads_what_was_stored_there() {
    // `m` = `inp[0]`, the function called stores `inp[1]` to it, and both are written out.
    let mut hand = Hand::new(&["m"]);
    let stores = hand.function(|f| {
        let (m, other) = (f.private(0), f.inp(1));
        f.store(m, other);
    });
    let called = hand.entry(|f| {
        let (m, first) = (f.private(0), f.inp(0));
        f.store(m, first);
        let before = f.load(m);
        f.call(stores);
        let after = f.load(m);
        for (index, value) in [(0, before), (1, after)] {
            let place = f.out(index);
            f.store(place, value);
        }
    });
    let left = outs(&called);
    assert!(left.iter().any(|out| out[0] != out[1]), "{left:?}");
    let (passed_by, changed) = passed(&called);
    // The load before the call reads what was stored; the one after it is still a load.
    assert!(changed.forwarded > 0, "{changed:?}");
    assert_eq!(loads(&passed_by), 2);

    // The same with a case of a `switch` on `inp[1]`'s low bit in the call's place.
    let switched = Hand::new(&[]).entry(|f| {
        let (r, first) = (f.local("r", None), f.inp(0));
        f.store(r, first);
        let before = f.load(r);
        let (bits, one) = (f.inp(1), f.word(1));
        let selector = f.op(Bo::And, bits, one);
        let stores = f.block(|f| {
            let other = f.inp(2);
            f.store(r, other);
        });
        let case = |value, body| SwitchCase {
            value,
            body,
            fall_through: false,
        };
        f.push(S::Switch {
            selector,
            cases: vec![
                case(SwitchValue::U32(0), stores),
                case(SwitchValue::Default, Block::new()),
            ],
        });
        let after = f.load(r);
        for (index, value) in [(0, before), (1, after)] {
            let place = f.out(index);
            f.store(place, value);
        }
    });
    let left = outs(&switched);
    // The case stored on some seed and was passed by on another.
    assert!(left.iter().any(|out| out[0] != out[1]), "{left:?}");
    assert!(left.iter().any(|out| out[0] == out[1]), "{left:?}");
    let (passed_by, changed) = passed(&switched);
    // Of `inp` three times, and of `r` after the `switch`.
    assert!(changed.forwarded > 0, "{changed:?}");
    assert_eq!(loads(&passed_by), 4);
}

#[test]
fn a_mask_narrowed_in_a_loop_is_tested_again_in_each_turn() {
    // `m` = `inp[0]`'s low bit. Where it is set, three turns: each counts itself in `out[1]`
    // where `m` is still set, then `m` &= `inp[1 + turn]`. `out[0]` is `m` at the end.
    let module = Hand::new(&[]).entry(|f| {
        f.says(1, 0);
        let (m, turn) = (f.local("m", None), f.local("turn", None));
        let (bits, one) = (f.inp(0), f.word(1));
        let low = f.op(Bo::And, bits, one);
        f.store(m, low);
        let entered = f.set(m);
        let body = f.block(|f| {
            let still = f.set(m);
            f.iff(
                still,
                |f| {
                    let place = f.out(1);
                    let (count, one) = (f.load(place), f.word(1));
                    let more = f.op(Bo::Add, count, one);
                    f.store(place, more);
                },
                |_| {},
            );
            let (at, one) = (f.load(turn), f.word(1));
            let next = f.op(Bo::Add, at, one);
            let base = f.e(E::GlobalVariable(f.inp));
            let pointer = f.e(E::Access { base, index: next });
            let (held, narrows) = (f.load(m), f.load(pointer));
            let narrowed = f.op(Bo::And, held, narrows);
            f.store(m, narrowed);
            f.store(turn, next);
            let three = f.word(3);
            let done = f.op(Bo::GreaterEqual, next, three);
            f.iff(done, |f| f.push(S::Break), |_| {});
        });
        f.iff(
            entered,
            |f| {
                f.push(S::Loop {
                    body,
                    continuing: Block::new(),
                    break_if: None,
                });
            },
            |_| {},
        );
        let (place, held) = (f.out(0), f.load(m));
        f.store(place, held);
    });
    let left = outs(&module);
    // Some seed's mask was narrowed to nought before the last turn, and some seed's never.
    for turns in [1, 2, 3] {
        assert!(left.iter().any(|out| out[1] == turns), "{turns}: {left:?}");
    }
    passed(&module);
}

/// `m` = 1, then a call of a function that leaves `m` &= `inp[1]`, and whether `m` is still
/// set said in `out`: 7, or 9. `calls` times over, the second in `out[1]`.
fn narrowed_by_a_call(calls: u32) -> naga::Module {
    let mut hand = Hand::new(&["m"]);
    let narrows = hand.function(|f| {
        let m = f.private(0);
        let (held, bits) = (f.load(m), f.inp(1));
        let narrowed = f.op(Bo::And, held, bits);
        f.store(m, narrowed);
    });
    hand.entry(|f| {
        let m = f.private(0);
        for call in 0..calls {
            let one = f.word(1);
            f.store(m, one);
            f.call(narrows);
            let set = f.set(m);
            f.iff(set, |f| f.says(call, 7), |f| f.says(call, 9));
        }
    })
}

#[test]
fn a_mask_a_called_function_stores_is_tested_after_the_call() {
    // Called from one place the function is walked there, and what it left is what the caller
    // goes on with; called from two it is walked alone, and what it stores is forgotten.
    for calls in [1, 2] {
        let module = narrowed_by_a_call(calls);
        let left = outs(&module);
        for said in [7, 9] {
            assert!(left.iter().any(|out| out[0] == said), "{said}: {left:?}");
        }
        let (_, changed) = passed(&module);
        assert_eq!(changed.branches, 0, "{calls}: {changed:?}");
    }
}

#[test]
fn a_buffer_stored_to_and_the_group_s_memory_are_loaded_again_after_a_store() {
    // `out[0]` += 1, and `out[1]` = `out[0]` as it then is; the same of the group's memory,
    // which starts as nought, into `out[2]`.
    let module = Hand::new(&[]).entry(|f| {
        for (place, again, copy) in [
            (f.out(0), f.out(0), f.out(1)),
            (f.shared(0), f.shared(0), f.out(2)),
        ] {
            let (held, one) = (f.load(place), f.word(1));
            let more = f.op(Bo::Add, held, one);
            f.store(place, more);
            let held = f.load(again);
            f.store(copy, held);
        }
    });
    let left = outs(&module);
    assert!(
        left.iter().all(|out| out[0] == out[1] && out[2] == 1),
        "{left:?}"
    );
    let (passed, changed) = passed(&module);
    // Each place is computed once, and both loads of it stay.
    assert!(changed.forwarded > 0, "{changed:?}");
    assert_eq!(loads(&passed), 4);
}

#[test]
fn what_nothing_reads_goes_before_the_first_round() {
    // `r` is stored where `inp[0]` is odd and loaded into a name nothing reads: with the load
    // gone the store is dead, and the `if` around it has nothing left to do.
    let module = Hand::new(&[]).entry(|f| {
        let r = f.local("r", None);
        let (bits, one) = (f.inp(0), f.word(1));
        let low = f.op(Bo::And, bits, one);
        let nought = f.word(0);
        let odd = f.op(Bo::NotEqual, low, nought);
        f.iff(
            odd,
            |f| {
                let other = f.inp(1);
                f.store(r, other);
            },
            |_| {},
        );
        let unread = f.load(r);
        (f.function.named_expressions).insert(unread, "unread".to_owned());
        let (place, last) = (f.out(0), f.inp(2));
        f.store(place, last);
    });
    // One round finds it all, which `passed` holds a second run to.
    let (passed, changed) = passed(&module);
    let found = (changed.stores, changed.empty, changed.registers);
    assert_eq!((found, changed.rounds), ((1, 1, 1), 1), "{changed:?}");
    let function = &passed.entry_points[0].function;
    assert!(function.named_expressions.is_empty());
    assert!(function.local_variables.is_empty());
}
