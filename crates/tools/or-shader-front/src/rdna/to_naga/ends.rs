//! Whether each loop of a program has, in the blocks it was lowered to, an exit the lane lowered
//! can change: a device waits for ever on a loop that has none, and takes its desktop with it.

use std::collections::{BTreeSet, HashMap};

use naga::{BinaryOperator as Bo, Expression as E, Handle, Literal, Statement as S};

/// What both counts of a loop not shown to end start with.
pub const LOOP_NOT_SHOWN: &str = "a loop the lowering cannot show to end";
/// Every exit of the loop is a vote over a mask that may hold lanes the lowering does not run,
/// which nothing it runs ever takes out.
pub const LOOP_OTHER_LANES: &str =
    "a loop the lowering cannot show to end: it repeats while lanes it does not run are left";
/// The loop has no exit, or none that hangs on what a turn leaves the next.
pub const LOOP_UNCHANGING: &str =
    "a loop the lowering cannot show to end: no turn changes what an exit hangs on";

/// Whether a thread's wave is the one whose block runs, as the lowering names it where the
/// waves of a group take turns.
const TURN: &str = "wave_turn";
/// The masks of lanes, as the lowering names their registers.
const MASKS: [&str; 4] = ["exec_lo", "exec_hi", "vcc_lo", "vcc_hi"];

/// In a set of what a value hangs on: memory a turn or another thread may have changed.
const VOLATILE: u32 = u32::MAX;
/// For what a variable hangs on: itself, as the block found it.
const ITSELF: u32 = u32::MAX;
const NOTHING: u32 = 0;

/// What is known of a value wherever the statements may have come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Known {
    /// The bits that are nought.
    zero: u32,
    /// Made of a mask of lanes by logic alone.
    mask: bool,
    /// Whether it says that a mask which may hold a lane other than this one holds any.
    foreign: bool,
    /// The set of variables, as the block found them, that it hangs on.
    deps: u32,
}

/// Sets of variables, each kept once.
#[derive(Default)]
struct Sets {
    list: Vec<Box<[u32]>>,
    ids: HashMap<Box<[u32]>, u32>,
    unions: HashMap<(u32, u32), u32>,
}

impl Sets {
    fn id(&mut self, set: Vec<u32>) -> u32 {
        let set = set.into_boxed_slice();
        if let Some(&id) = self.ids.get(&set) {
            return id;
        }
        let id = self.list.len() as u32;
        self.list.push(set.clone());
        self.ids.insert(set, id);
        id
    }

    fn one(&mut self, var: u32) -> u32 {
        self.id(vec![var])
    }

    fn union(&mut self, a: u32, b: u32) -> u32 {
        if a == b || b == NOTHING {
            return a;
        }
        if a == NOTHING {
            return b;
        }
        let key = (a.min(b), a.max(b));
        if let Some(&id) = self.unions.get(&key) {
            return id;
        }
        let both = self.list[a as usize].iter().chain(&*self.list[b as usize]);
        let mut merged: Vec<u32> = both.copied().collect();
        merged.sort_unstable();
        merged.dedup();
        let id = self.id(merged);
        self.unions.insert(key, id);
        id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Var {
    Local(usize, Handle<naga::LocalVariable>),
    Global(Handle<naga::GlobalVariable>),
}

/// What a variable of a thread's own is to the walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Unseen,
    /// A register, by its number among those seen.
    Held(u32),
    /// What a block's branch is decided by: stored by its block and read by no other.
    Decides,
}

/// Where a pointer leads.
enum Root {
    /// A variable of the thread's own, and whether the pointer is to the whole of it.
    Var(u32, bool),
    Decides,
    /// Memory nothing the program runs writes.
    Fixed,
    Volatile,
}

/// What is known of each variable seen, by its number; nothing of one a path says nothing of.
type State = Vec<Option<Known>>;

struct Walk<'a> {
    module: &'a naga::Module,
    functions: &'a [&'a naga::Function],
    /// The function the block being walked was lowered in.
    home: usize,
    /// Each function's locals, and the module's globals, by their place.
    locals: Vec<Vec<Slot>>,
    globals: Vec<Slot>,
    /// By variable: whether it is a mask of lanes, and the set that holds it alone.
    masks: Vec<bool>,
    alone: Vec<u32>,
    sets: Sets,
    /// By function and expression.
    values: Vec<Vec<Known>>,
    state: State,
    /// What the block being walked stored its branch's condition as.
    decided: Option<Known>,
    /// What each write since the block began replaced.
    log: Vec<(u32, Option<Known>)>,
    shares: HashMap<Handle<naga::Function>, bool>,
}

fn fixed(space: naga::AddressSpace) -> bool {
    match space {
        naga::AddressSpace::Uniform | naga::AddressSpace::Handle => true,
        naga::AddressSpace::Storage { access } => !access.contains(naga::StorageAccess::STORE),
        _ => false,
    }
}

impl<'a> Walk<'a> {
    fn var(&mut self, var: Var) -> Slot {
        let (slot, name) = match var {
            Var::Local(home, local) => (
                &mut self.locals[home][local.index()],
                &self.functions[home].local_variables[local].name,
            ),
            Var::Global(global) => (
                &mut self.globals[global.index()],
                &self.module.global_variables[global].name,
            ),
        };
        if *slot == Slot::Unseen {
            let name = name.as_deref().unwrap_or_default();
            let decides = (name.strip_prefix("cond")).is_some_and(|n| n.parse::<u32>().is_ok());
            *slot = match decides {
                true => Slot::Decides,
                false => {
                    let id = self.masks.len() as u32;
                    self.masks.push(MASKS.contains(&name));
                    self.alone.push(self.sets.one(id));
                    self.state.push(None);
                    Slot::Held(id)
                }
            };
        }
        *slot
    }

    fn rooted(&mut self, var: Var, whole: bool) -> Root {
        match self.var(var) {
            Slot::Held(id) => Root::Var(id, whole),
            _ => Root::Decides,
        }
    }

    fn of(&self, expression: Handle<E>) -> Known {
        self.values[self.home][expression.index()]
    }

    /// The variable `pointer` is into, and what the places it picks hang on.
    fn root(&mut self, mut pointer: Handle<E>) -> (Root, u32) {
        let (mut whole, mut picked) = (true, NOTHING);
        loop {
            match self.functions[self.home].expressions[pointer] {
                E::Access { base, index } => {
                    picked = self.sets.union(picked, self.of(index).deps);
                    whole = false;
                    pointer = base;
                }
                E::AccessIndex { base, .. } => {
                    whole = false;
                    pointer = base;
                }
                E::LocalVariable(local) => {
                    return (self.rooted(Var::Local(self.home, local), whole), picked);
                }
                E::GlobalVariable(global) => {
                    let root = match self.module.global_variables[global].space {
                        naga::AddressSpace::Private => self.rooted(Var::Global(global), whole),
                        space if fixed(space) => Root::Fixed,
                        _ => Root::Volatile,
                    };
                    return (root, picked);
                }
                _ => return (Root::Volatile, picked),
            }
        }
    }

    /// What a variable hangs on: what its last store in the block did, else itself.
    fn deps_of(&self, var: u32) -> u32 {
        match self.state[var as usize] {
            Some(held) if held.deps != ITSELF => held.deps,
            _ => self.alone[var as usize],
        }
    }

    fn load(&mut self, pointer: Handle<E>) -> Known {
        let (root, picked) = self.root(pointer);
        match root {
            Root::Var(var, whole) => {
                let deps = self.deps_of(var);
                let deps = self.sets.union(deps, picked);
                let held = self.state[var as usize].unwrap_or_default();
                let mask = held.mask || self.masks[var as usize];
                match whole {
                    true => Known { mask, deps, ..held },
                    false => Known {
                        zero: 0,
                        mask,
                        foreign: held.foreign,
                        deps,
                    },
                }
            }
            Root::Fixed | Root::Decides => Known {
                deps: picked,
                ..Known::default()
            },
            Root::Volatile => Known {
                deps: self.volatile(picked),
                ..Known::default()
            },
        }
    }

    fn volatile(&mut self, with: u32) -> u32 {
        let volatile = self.sets.one(VOLATILE);
        self.sets.union(with, volatile)
    }

    /// Whether an image is one the program stores to.
    fn stored_image(&self, mut image: Handle<E>) -> bool {
        let function = self.functions[self.home];
        while let E::Access { base, .. } | E::AccessIndex { base, .. } = function.expressions[image]
        {
            image = base;
        }
        let E::GlobalVariable(global) = function.expressions[image] else {
            return true;
        };
        let mut ty = &self.module.types[self.module.global_variables[global].ty].inner;
        while let naga::TypeInner::BindingArray { base, .. } = *ty {
            ty = &self.module.types[base].inner;
        }
        !matches!(
            ty,
            naga::TypeInner::Image {
                class: naga::ImageClass::Sampled { .. } | naga::ImageClass::Depth { .. },
                ..
            }
        )
    }

    fn evaluate(&mut self, expression: Handle<E>) -> Known {
        let function = self.functions[self.home];
        let mut operands = [None; 4];
        let mut volatile = false;
        match function.expressions[expression] {
            E::Load { pointer } => return self.load(pointer),
            E::Binary { op, left, right } => {
                let (a, b) = (self.of(left), self.of(right));
                let deps = self.sets.union(a.deps, b.deps);
                // Logic and arithmetic make a number of what a vote said, which says it no more:
                // a lane that runs because of a vote is still asked for its own bit.
                let logic = Known {
                    zero: 0,
                    mask: a.mask || b.mask,
                    foreign: false,
                    deps,
                };
                let said = a.foreign || b.foreign;
                let by = match function.expressions[right] {
                    E::Literal(Literal::U32(by)) if by < 32 => Some(by),
                    _ => None,
                };
                // Whether `x` is a mask that may hold a lane other than this one, asked if empty.
                let vote = |x: Known, nought: Known| {
                    x.mask && nought.zero == u32::MAX && x.zero | 1 != u32::MAX
                };
                return match op {
                    Bo::And => Known {
                        zero: a.zero | b.zero,
                        ..logic
                    },
                    Bo::InclusiveOr | Bo::ExclusiveOr => Known {
                        zero: a.zero & b.zero,
                        ..logic
                    },
                    Bo::ShiftLeft => Known {
                        zero: by.map_or(0, |by| a.zero << by | !(u32::MAX << by)),
                        ..logic
                    },
                    Bo::ShiftRight => Known {
                        zero: by.map_or(0, |by| a.zero >> by | !(u32::MAX >> by)),
                        ..logic
                    },
                    Bo::Equal | Bo::NotEqual => Known {
                        mask: false,
                        foreign: said || vote(a, b) || vote(b, a),
                        ..logic
                    },
                    Bo::LogicalAnd | Bo::LogicalOr => Known {
                        mask: false,
                        foreign: said,
                        ..logic
                    },
                    _ => Known {
                        mask: false,
                        ..logic
                    },
                };
            }
            E::Unary { op, expr } => {
                let of = self.of(expr);
                return Known {
                    zero: 0,
                    foreign: of.foreign && op == naga::UnaryOperator::LogicalNot,
                    ..of
                };
            }
            E::Select {
                condition,
                accept,
                reject,
            } => {
                let (c, a, b) = (self.of(condition), self.of(accept), self.of(reject));
                let deps = self.sets.union(a.deps, b.deps);
                // A condition as a number says what the condition said; a mask a condition
                // chose is a mask, and a vote over it is over the lanes it holds.
                let stated = |e| matches!(function.expressions[e], E::Literal(_));
                let says = c.foreign && stated(accept) && stated(reject);
                return Known {
                    zero: a.zero & b.zero,
                    mask: a.mask || b.mask,
                    foreign: says || a.foreign || b.foreign,
                    deps: self.sets.union(deps, c.deps),
                };
            }
            E::As { expr, .. }
            | E::Splat { value: expr, .. }
            | E::Swizzle { vector: expr, .. }
            | E::Derivative { expr, .. }
            | E::Relational { argument: expr, .. }
            | E::AccessIndex { base: expr, .. } => operands[0] = Some(expr),
            E::Access { base, index } => operands = [Some(base), Some(index), None, None],
            E::Compose { ref components, .. } => {
                let mut known = Known::default();
                for &component in components {
                    let of = self.of(component);
                    known.foreign |= of.foreign;
                    known.deps = self.sets.union(known.deps, of.deps);
                }
                return known;
            }
            E::Math {
                arg,
                arg1,
                arg2,
                arg3,
                ..
            } => operands = [Some(arg), arg1, arg2, arg3],
            E::ImageSample {
                coordinate,
                array_index,
                depth_ref,
                ..
            } => operands = [Some(coordinate), array_index, depth_ref, None],
            E::ImageLoad {
                image,
                coordinate,
                array_index,
                sample,
                level,
            } => {
                volatile = self.stored_image(image);
                operands = [Some(coordinate), array_index, sample, level];
            }
            E::ImageQuery { .. } | E::ArrayLength(_) => {}
            _ => volatile = true,
        }
        let mut known = Known::default();
        for operand in operands.into_iter().flatten() {
            let of = self.of(operand);
            known.foreign |= of.foreign;
            known.deps = self.sets.union(known.deps, of.deps);
        }
        if volatile {
            known.deps = self.volatile(known.deps);
        }
        known
    }

    fn set(&mut self, var: u32, known: Known) {
        let before = self.state[var as usize].replace(known);
        self.log.push((var, before));
    }

    /// The value a store keeps for the wave whose turn it is, where the store is one that a
    /// thread of another wave only walks, keeping what it held: for the thread a block is
    /// lowered for, the store is of that value.
    fn in_turn(&self, pointer: Handle<E>, value: Handle<E>) -> Handle<E> {
        let function = self.functions[self.home];
        let E::Select {
            condition,
            accept,
            reject,
        } = function.expressions[value]
        else {
            return value;
        };
        let E::Load { pointer: flag } = function.expressions[condition] else {
            return value;
        };
        let turn = matches!(function.expressions[flag], E::GlobalVariable(global)
            if self.module.global_variables[global].name.as_deref() == Some(TURN));
        let kept = matches!(function.expressions[reject], E::Load { pointer: p } if p == pointer);
        match turn && kept {
            true => accept,
            false => value,
        }
    }

    fn store(&mut self, pointer: Handle<E>, value: Handle<E>) {
        let value = self.in_turn(pointer, value);
        let (var, whole) = match self.root(pointer).0 {
            Root::Var(var, whole) => (var, whole),
            Root::Decides => {
                self.decided = Some(self.of(value));
                return;
            }
            _ => return,
        };
        let function = self.functions[self.home];
        let mut new = self.of(value);
        if !whole {
            let before = self.deps_of(var);
            let held = self.state[var as usize].unwrap_or_default();
            new = Known {
                zero: 0,
                mask: new.mask || held.mask,
                foreign: new.foreign || held.foreign,
                deps: self.sets.union(new.deps, before),
            };
        } else if let E::Select {
            condition,
            accept,
            reject,
        } = function.expressions[value]
        {
            // A variable written in some lanes and kept as it was in the others: holding what
            // it held is no change a turn makes.
            let keeps =
                |e| matches!(function.expressions[e], E::Load { pointer: p } if p == pointer);
            let fresh = match (keeps(accept), keeps(reject)) {
                (false, true) => Some(accept),
                (true, false) => Some(reject),
                _ => None,
            };
            if let Some(fresh) = fresh {
                new.deps = self
                    .sets
                    .union(self.of(condition).deps, self.of(fresh).deps);
                if let Some(held) = self.state[var as usize].filter(|held| held.deps != ITSELF) {
                    new.deps = self.sets.union(new.deps, held.deps);
                }
            }
        }
        self.set(var, new);
    }

    /// The statements of which one of `bodies` runs, or none where `skips`, as `chooser` says.
    fn arms(&mut self, chooser: Known, bodies: &[&naga::Block], skips: bool) {
        let mut wrote: Vec<Vec<(u32, Known)>> = Vec::with_capacity(bodies.len());
        let mut vars: Vec<u32> = Vec::new();
        for body in bodies {
            let mark = self.log.len();
            self.block(body);
            let mut arm: Vec<(u32, Known)> = Vec::new();
            while self.log.len() > mark {
                let Some((var, before)) = self.log.pop() else {
                    break;
                };
                // The first write undone is the last made: what the arm leaves.
                if !arm.iter().any(|(wrote, _)| *wrote == var) {
                    arm.push((var, self.state[var as usize].unwrap_or_default()));
                }
                self.state[var as usize] = before;
                if !vars.contains(&var) {
                    vars.push(var);
                }
            }
            wrote.push(arm);
        }
        for var in vars {
            let before = self.state[var as usize];
            let kept = Known {
                zero: 0,
                deps: ITSELF,
                ..Known::default()
            };
            let mut joined = Known {
                zero: u32::MAX,
                deps: chooser.deps,
                ..Known::default()
            };
            let left = |arm: &Vec<(u32, Known)>| {
                let wrote = arm.iter().find(|(wrote, _)| *wrote == var);
                wrote.map(|(_, known)| *known).or(before)
            };
            let ways = wrote.iter().map(left).chain(skips.then_some(before));
            for way in ways {
                let way = way.unwrap_or(kept);
                joined.zero &= way.zero;
                joined.mask |= way.mask;
                joined.foreign |= way.foreign;
                if way.deps != ITSELF {
                    joined.deps = self.sets.union(joined.deps, way.deps);
                }
            }
            self.set(var, joined);
        }
    }

    /// Whether a function reads or writes memory the thread shares.
    fn shares(&mut self, function: Handle<naga::Function>) -> bool {
        if let Some(&shares) = self.shares.get(&function) {
            return shares;
        }
        let globals = &self.module.global_variables;
        let private = |space| space == naga::AddressSpace::Private || fixed(space);
        let shares = (self.module.functions[function].expressions.iter()).any(|(_, e)| match e {
            E::GlobalVariable(global) => !private(globals[*global].space),
            E::CallResult(_) => true,
            _ => false,
        });
        self.shares.insert(function, shares);
        shares
    }

    fn block(&mut self, block: &naga::Block) {
        for statement in block {
            match *statement {
                S::Emit(ref range) => {
                    for expression in range.clone() {
                        let known = self.evaluate(expression);
                        self.values[self.home][expression.index()] = known;
                    }
                }
                S::Block(ref inner) => self.block(inner),
                S::Store { pointer, value } => self.store(pointer, value),
                S::If {
                    condition,
                    ref accept,
                    ref reject,
                } => self.arms(self.of(condition), &[accept, reject], false),
                S::Switch {
                    selector,
                    ref cases,
                } => {
                    let bodies: Vec<&naga::Block> = cases.iter().map(|case| &case.body).collect();
                    self.arms(self.of(selector), &bodies, true);
                }
                // A loop the lowering wrote within a block: what it writes is not followed.
                S::Loop {
                    ref body,
                    ref continuing,
                    ..
                } => {
                    let mark = self.log.len();
                    self.block(body);
                    self.block(continuing);
                    let wrote: BTreeSet<u32> = self.log[mark..].iter().map(|(v, _)| *v).collect();
                    for var in wrote {
                        let left = self.state[var as usize].unwrap_or_default();
                        let known = Known {
                            zero: 0,
                            mask: left.mask || self.masks[var as usize],
                            foreign: left.foreign,
                            deps: self.volatile(NOTHING),
                        };
                        self.set(var, known);
                    }
                }
                S::Call {
                    function,
                    ref arguments,
                    result: Some(result),
                } => {
                    let mut known = Known::default();
                    if self.shares(function) {
                        known.deps = self.volatile(NOTHING);
                    }
                    for &argument in arguments {
                        let of = self.of(argument);
                        known.foreign |= of.foreign;
                        known.deps = self.sets.union(known.deps, of.deps);
                    }
                    self.values[self.home][result.index()] = known;
                }
                S::Atomic {
                    result: Some(result),
                    ..
                }
                | S::WorkGroupUniformLoad { result, .. }
                | S::SubgroupBallot { result, .. }
                | S::SubgroupCollectiveOperation { result, .. }
                | S::SubgroupGather { result, .. } => {
                    let known = Known {
                        deps: self.volatile(NOTHING),
                        ..Known::default()
                    };
                    self.values[self.home][result.index()] = known;
                }
                _ => {}
            }
        }
    }
}

/// What two paths into a block both leave, into `into`; whether that changed it.
fn join(into: &mut Option<State>, out: &State) -> bool {
    let Some(held) = into else {
        *into = Some(out.clone());
        return true;
    };
    let mut changed = false;
    for (var, slot) in held.iter_mut().enumerate() {
        let Some(known) = *slot else {
            continue;
        };
        let joined = out.get(var).copied().flatten().map(|other| Known {
            zero: known.zero & other.zero,
            mask: known.mask || other.mask,
            foreign: known.foreign || other.foreign,
            deps: ITSELF,
        });
        changed |= joined != *slot;
        *slot = joined;
    }
    changed
}

/// The loops of a program not shown to end, as [`LOOP_OTHER_LANES`] and [`LOOP_UNCHANGING`]
/// count them: `blocks` are each block's statements and the function of `functions` they were
/// lowered in, `successors` the blocks each hands control to.
// ponytail: a loop is one header and the blocks that reach its back edges; what a variable
// hangs on is kept a block at a time and joined over the loop's blocks whatever their order.
// A loop whose exit is a count against a number a buffer holds is shown to change, not to end.
pub(super) fn not_shown_to_end(
    module: &naga::Module,
    functions: &[&naga::Function],
    blocks: &[(usize, &naga::Block)],
    successors: &[Vec<usize>],
) -> [u32; 2] {
    let count = blocks.len();
    // Reverse post-order of the blocks control reaches.
    let mut post = Vec::with_capacity(count);
    let mut seen = vec![false; count];
    let mut stack = vec![(0usize, 0usize)];
    seen[0] = true;
    while let Some(top) = stack.last_mut() {
        let (block, nth) = *top;
        match successors[block].get(nth) {
            Some(&to) => {
                top.1 += 1;
                if !seen[to] {
                    seen[to] = true;
                    stack.push((to, 0));
                }
            }
            None => {
                post.push(block);
                stack.pop();
            }
        }
    }
    let order: Vec<usize> = post.into_iter().rev().collect();
    let mut place = vec![usize::MAX; count];
    for (at, &block) in order.iter().enumerate() {
        place[block] = at;
    }
    let mut latches: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); count];
    for &block in &order {
        for &to in &successors[block] {
            preds[to].push(block);
            if place[to] <= place[block] {
                latches[to].push(block);
            }
        }
    }
    if latches.iter().all(Vec::is_empty) {
        return [0, 0];
    }

    let mut walk = Walk {
        module,
        functions,
        home: 0,
        locals: (functions.iter())
            .map(|function| vec![Slot::Unseen; function.local_variables.len()])
            .collect(),
        globals: vec![Slot::Unseen; module.global_variables.len()],
        masks: Vec::new(),
        alone: Vec::new(),
        sets: Sets::default(),
        values: Vec::with_capacity(functions.len()),
        state: State::new(),
        decided: None,
        log: Vec::new(),
        shares: HashMap::new(),
    };
    walk.sets.id(Vec::new());
    for function in functions {
        let literal = |(_, e): (Handle<E>, &E)| Known {
            zero: match *e {
                E::Literal(Literal::U32(word)) => !word,
                _ => 0,
            },
            ..Known::default()
        };
        (walk.values).push(function.expressions.iter().map(literal).collect());
    }
    // A lane starts as the one lane of its wave that runs.
    let named = |name: &str| {
        let local = (functions[0].local_variables.iter())
            .find(|(_, local)| local.name.as_deref() == Some(name))
            .map(|(local, _)| Var::Local(0, local));
        local.or_else(|| {
            (module.global_variables.iter())
                .find(|(_, global)| global.name.as_deref() == Some(name))
                .map(|(global, _)| Var::Global(global))
        })
    };
    for (name, zero) in [(MASKS[0], !1), (MASKS[1], u32::MAX)] {
        if let Some(Slot::Held(var)) = named(name).map(|var| walk.var(var)) {
            walk.state[var as usize] = Some(Known {
                zero,
                mask: true,
                foreign: false,
                deps: ITSELF,
            });
        }
    }

    let mut ins: Vec<Option<State>> = vec![None; count];
    ins[0] = Some(std::mem::take(&mut walk.state));
    // By block: what each variable it stores hangs on, and what its branch is decided by.
    let mut stores: Vec<Vec<(u32, u32)>> = vec![Vec::new(); count];
    let mut decided: Vec<Option<Known>> = vec![None; count];
    let mut dirty = vec![false; count];
    dirty[0] = true;
    loop {
        let mut any = false;
        for &block in &order {
            if !std::mem::take(&mut dirty[block]) {
                continue;
            }
            any = true;
            walk.home = blocks[block].0;
            walk.state
                .clone_from(ins[block].as_ref().unwrap_or(&State::new()));
            // A variable seen since the block was last reached is one its paths say nothing of.
            walk.state.resize(walk.masks.len(), None);
            walk.log.clear();
            walk.decided = None;
            walk.block(blocks[block].1);
            let mut wrote: Vec<u32> = walk.log.iter().map(|(var, _)| *var).collect();
            wrote.sort_unstable();
            wrote.dedup();
            stores[block].clear();
            for var in wrote {
                // The blocks after this one hang on a variable as they find it.
                if let Some(known) = &mut walk.state[var as usize] {
                    stores[block].push((var, known.deps));
                    known.deps = ITSELF;
                }
            }
            decided[block] = walk.decided;
            for &to in &successors[block] {
                dirty[to] |= join(&mut ins[to], &walk.state);
            }
        }
        if !any {
            break;
        }
    }

    let mut counts = [0, 0];
    for header in 0..count {
        if latches[header].is_empty() {
            continue;
        }
        let mut inside = vec![false; count];
        inside[header] = true;
        let mut stack = latches[header].clone();
        while let Some(block) = stack.pop() {
            if !std::mem::replace(&mut inside[block], true) {
                stack.extend(&preds[block]);
            }
        }
        // What a turn of the loop makes each variable it stores hang on: each set its blocks
        // store it as, once, then what those hold, once.
        let vars = walk.masks.len();
        let mut hangs: Vec<Vec<u32>> = vec![Vec::new(); vars];
        for block in (0..count).filter(|&block| inside[block]) {
            for &(var, deps) in &stores[block] {
                hangs[var as usize].push(deps);
            }
        }
        for on in &mut hangs {
            on.sort_unstable();
            on.dedup();
            let sets = std::mem::take(on);
            on.extend(sets.iter().flat_map(|&set| &*walk.sets.list[set as usize]));
            on.sort_unstable();
            on.dedup();
        }
        // What may differ from one turn to the next: what hangs, through what the loop stores,
        // on memory or on a variable made of itself, as a count is or a mask lanes are taken
        // from. Everything else a turn finds as the last did.
        let mut differs: Vec<bool> = hangs.iter().map(|on| !on.is_empty()).collect();
        loop {
            let mut settled = false;
            for var in 0..vars {
                let hung = |on: &u32| *on == VOLATILE || differs[*on as usize];
                if differs[var] && !hangs[var].iter().any(hung) {
                    differs[var] = false;
                    settled = true;
                }
            }
            if !settled {
                break;
            }
        }
        let differs = |on: &u32| *on == VOLATILE || differs[*on as usize];
        let exits = (0..count)
            .filter(|&block| inside[block] && successors[block].iter().any(|&to| !inside[to]));
        let (mut shown, mut foreign) = (false, false);
        for block in exits {
            let Some(known) = decided[block] else {
                continue;
            };
            foreign |= known.foreign;
            let on: &[u32] = match known.deps {
                ITSELF => &[],
                deps => &walk.sets.list[deps as usize],
            };
            shown |= !known.foreign && on.iter().any(differs);
        }
        if !shown {
            counts[usize::from(!foreign)] += 1;
        }
    }
    counts
}
