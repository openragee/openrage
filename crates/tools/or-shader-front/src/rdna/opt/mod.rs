//! Passes over a lowered module that leave every value it computes as it is, bit for bit: a bare
//! block joined to its parent, a constant followed through the registers and the branches on the
//! lane mask, a load read from what was stored, an expression computed once, a bit cast of a
//! bit cast read from what was first cast, and a store nothing reads dropped with the register
//! nothing names any more. No float operation is added, reordered or fused, and none whose
//! result is read is removed.

mod casts;
mod constants;
mod dead;
mod forward;
mod splice;

use naga::{Arena, Block, Expression as E, Function, Handle, Statement as S};

/// What [`optimise`] changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Changed {
    /// Bare blocks joined to their parents.
    pub blocks: usize,
    /// Expressions that come to one dword or one condition, now literals.
    pub literals: usize,
    /// `select`s on a known condition, read as the arm it takes.
    pub selects: usize,
    /// Operators that leave an operand as it was, read as that operand.
    pub idle: usize,
    /// `if`s on a known condition, now the arm it takes.
    pub branches: usize,
    /// Writes that kept what a register held unless a condition held, whose condition is known.
    pub guards: usize,
    /// Loads and other expressions read from an equal value already in scope.
    pub forwarded: usize,
    /// Bit casts replaced by what an earlier bit cast read, or that now read it themselves.
    pub casts: usize,
    /// Stores to a register that nothing reads before the next store to it, dropped.
    pub stores: usize,
    /// `if`s left with nothing to do in either arm, dropped.
    pub empty: usize,
    /// Variables of a thread's own that nothing names any more, dropped.
    pub registers: usize,
    /// The rounds of the passes that changed something.
    pub rounds: usize,
}

impl Changed {
    fn add(&mut self, other: Self) {
        self.blocks += other.blocks;
        self.literals += other.literals;
        self.selects += other.selects;
        self.idle += other.idle;
        self.branches += other.branches;
        self.guards += other.guards;
        self.forwarded += other.forwarded;
        self.casts += other.casts;
        self.stores += other.stores;
        self.empty += other.empty;
        self.registers += other.registers;
        self.rounds += other.rounds;
    }
}

fn functions(module: &naga::Module) -> impl Iterator<Item = &Function> {
    (module.functions.iter().map(|(_, function)| function))
        .chain(module.entry_points.iter().map(|entry| &entry.function))
}

fn functions_mut(module: &mut naga::Module) -> impl Iterator<Item = &mut Function> {
    (module.functions.iter_mut().map(|(_, function)| function)).chain(
        module
            .entry_points
            .iter_mut()
            .map(|entry| &mut entry.function),
    )
}

/// Calls `f` on each statement under `block`, those of the blocks inside it included.
fn statements(block: &Block, f: &mut impl FnMut(&S)) {
    for statement in block.iter() {
        f(statement);
        match statement {
            S::Block(inner) => statements(inner, f),
            S::If { accept, reject, .. } => {
                statements(accept, f);
                statements(reject, f);
            }
            S::Switch { cases, .. } => cases.iter().for_each(|case| statements(&case.body, f)),
            S::Loop {
                body, continuing, ..
            } => {
                statements(body, f);
                statements(continuing, f);
            }
            _ => {}
        }
    }
}

/// The expressions `module` computes: those its `Emit` statements cover.
pub fn emitted(module: &naga::Module) -> usize {
    let mut emitted = 0;
    for function in functions(module) {
        statements(&function.body, &mut |statement| {
            if let S::Emit(range) = statement {
                emitted += range.clone().count();
            }
        });
    }
    emitted
}

/// The variables of a module that are read and written only whole: its registers. No pointer
/// reaches one but a load's and a store's own, so nothing else reads or writes it.
struct Registers {
    /// By global: a private variable used only whole.
    globals: Vec<bool>,
    /// By function, in the order of [`functions`], then by local.
    locals: Vec<Vec<bool>>,
    /// By function, then by global: the registers it stores to, a function it calls included.
    stores: Vec<Vec<bool>>,
    /// By global: memory no statement of a module writes.
    fixed: Vec<bool>,
}

impl Registers {
    fn of(module: &mut naga::Module) -> Self {
        use naga::{AddressSpace as Space, StorageAccess};
        let spaces = || {
            module
                .global_variables
                .iter()
                .map(|(_, global)| global.space)
        };
        let mut globals: Vec<bool> = spaces().map(|space| space == Space::Private).collect();
        let fixed = (spaces())
            .map(|space| match space {
                Space::Uniform => true,
                Space::Storage { access } => !access.contains(StorageAccess::STORE),
                _ => false,
            })
            .collect();
        let mut locals = Vec::new();
        for function in functions_mut(module) {
            // How often each expression is read, and how often as the pointer of a load or of
            // a store: a variable read any other way is no register.
            let count = function.expressions.len();
            let (mut read, mut whole) = (vec![0u32; count], vec![0u32; count]);
            for (_, expression) in function.expressions.iter_mut() {
                if let E::Load { pointer } = *expression {
                    whole[pointer.index()] += 1;
                }
                operands(expression, &mut |at| read[at.index()] += 1);
            }
            reads(&mut function.body, &mut |at| read[at.index()] += 1);
            statements(&function.body, &mut |statement| {
                if let S::Store { pointer, .. } = *statement {
                    whole[pointer.index()] += 1;
                }
            });
            let mut own = vec![true; function.local_variables.len()];
            for (at, expression) in function.expressions.iter() {
                if read[at.index()] == whole[at.index()] {
                    continue;
                }
                match *expression {
                    E::LocalVariable(local) => own[local.index()] = false,
                    E::GlobalVariable(global) => globals[global.index()] = false,
                    _ => {}
                }
            }
            locals.push(own);
        }
        // A function is declared before the functions that call it, and the entry points last.
        let mut stores: Vec<Vec<bool>> = Vec::new();
        for function in functions(module) {
            let mut own = vec![false; globals.len()];
            statements(&function.body, &mut |statement| match *statement {
                S::Store { pointer, .. } => {
                    if let E::GlobalVariable(global) = function.expressions[pointer] {
                        own[global.index()] |= globals[global.index()];
                    }
                }
                S::Call {
                    function: called, ..
                } => {
                    for (own, &called) in own.iter_mut().zip(&stores[called.index()]) {
                        *own |= called;
                    }
                }
                _ => {}
            });
            stores.push(own);
        }
        Self {
            globals,
            locals,
            stores,
            fixed,
        }
    }

    /// How many variables function `function` sees: the globals, then its locals.
    fn count(&self, function: usize) -> usize {
        self.globals.len() + self.locals[function].len()
    }

    /// The register `pointer` is, as its place among the variables of function `function`.
    fn var(&self, function: usize, expressions: &Arena<E>, pointer: Handle<E>) -> Option<usize> {
        match expressions[pointer] {
            E::GlobalVariable(global) if self.globals[global.index()] => Some(global.index()),
            E::LocalVariable(local) if self.locals[function][local.index()] => {
                Some(self.globals.len() + local.index())
            }
            _ => None,
        }
    }

    /// Adds to `out` the registers the statements under `block` may store to.
    fn stored(&self, function: usize, expressions: &Arena<E>, block: &Block, out: &mut Vec<usize>) {
        statements(block, &mut |statement| match *statement {
            S::Store { pointer, .. } => out.extend(self.var(function, expressions, pointer)),
            S::Call {
                function: called, ..
            } => {
                let stores = self.stores[called.index()].iter().enumerate();
                out.extend(
                    stores
                        .filter(|&(_, &stored)| stored)
                        .map(|(global, _)| global),
                );
            }
            _ => {}
        });
    }
}

/// The writes of `module` that keep what a variable held unless a condition holds: `v = select(c, new, v)`.
pub fn guarded(module: &naga::Module) -> usize {
    let mut module = module.clone();
    let registers = Registers::of(&mut module);
    (functions(&module).enumerate())
        .map(|(index, function)| forward::walk(function, index, &registers).guarded)
        .sum()
}

/// The rounds of the passes run at most: a round reads what the one before it left.
const ROUNDS: usize = 32;

/// Runs the passes over `module`, which the validator must have taken, until a round of them changes nothing. `keep` names the registers read from outside the module once it has run: the last store to each stays.
pub fn optimise(module: &mut naga::Module, keep: &[&str]) -> Changed {
    // A round that finds nothing must leave the module as it was, so what nothing reads goes
    // before the first as it does after each.
    unread(module);
    let mut all = Changed::default();
    for _ in 0..ROUNDS {
        let changed = round(module, keep);
        if changed == Changed::default() {
            break;
        }
        all.add(changed);
        all.rounds += 1;
    }
    all
}

/// Every pass once, then what nothing reads dropped.
fn round(module: &mut naga::Module, keep: &[&str]) -> Changed {
    let mut changed = Changed::default();
    for function in functions_mut(module) {
        changed.blocks += splice::splice(&mut function.body);
    }
    let registers = Registers::of(module);
    // What a pass takes out only narrows what reads a register: the same registers stand.
    // Forwarding first has a value compared with itself read as one expression, which the
    // constants then decide.
    for (index, function) in functions_mut(module).enumerate() {
        changed.forwarded += forward::walk(function, index, &registers).apply(function);
    }
    let [literals, selects, idle, branches, guards] = constants::fold(module, &registers);
    (changed.literals, changed.selects, changed.idle) = (literals, selects, idle);
    (changed.branches, changed.guards) = (branches, guards);
    // And again: an arm read in its `select`'s place may be an expression already computed.
    for (index, function) in functions_mut(module).enumerate() {
        changed.forwarded += forward::walk(function, index, &registers).apply(function);
    }
    let plans: Vec<casts::Plan> = (functions(module))
        .map(|function| casts::plan(module, function))
        .collect();
    for (function, plan) in functions_mut(module).zip(plans) {
        changed.casts += plan.apply(function);
    }
    [changed.stores, changed.empty] = dead::stores(module, &registers, keep);
    unread(module);
    changed.registers = dead::registers(module);
    changed
}

/// Drops the expressions of `module` that no statement reads, itself or through another. Every
/// function, bound global and named type stays.
fn unread(module: &mut naga::Module) {
    // The compaction keeps an expression for its name alone, and a pass would meet it again
    // in every round: a name goes with the last reader.
    for function in functions_mut(module) {
        if function.named_expressions.is_empty() {
            continue;
        }
        let mut read = vec![false; function.expressions.len()];
        reads(&mut function.body, &mut |at| read[at.index()] = true);
        // An expression comes after everything it reads.
        for (at, expression) in function.expressions.iter_mut().rev() {
            if read[at.index()] {
                operands(expression, &mut |at| read[at.index()] = true);
            }
        }
        (function.named_expressions).retain(|at, _| read[at.index()]);
    }
    naga::compact::compact(module, naga::compact::KeepUnused::Yes);
}

/// Calls `f` on each expression `expression` reads.
fn operands(expression: &mut E, f: &mut impl FnMut(&mut Handle<E>)) {
    match expression {
        E::Literal(_)
        | E::Constant(_)
        | E::Override(_)
        | E::ZeroValue(_)
        | E::FunctionArgument(_)
        | E::GlobalVariable(_)
        | E::LocalVariable(_)
        | E::CallResult(_)
        | E::AtomicResult { .. }
        | E::WorkGroupUniformLoadResult { .. }
        | E::RayQueryProceedResult
        | E::SubgroupBallotResult
        | E::SubgroupOperationResult { .. } => {}
        E::Compose { components, .. } => components.iter_mut().for_each(f),
        E::Access { base, index } => [base, index].into_iter().for_each(f),
        E::AccessIndex { base: one, .. }
        | E::Splat { value: one, .. }
        | E::Swizzle { vector: one, .. }
        | E::Load { pointer: one }
        | E::Unary { expr: one, .. }
        | E::Derivative { expr: one, .. }
        | E::Relational { argument: one, .. }
        | E::As { expr: one, .. }
        | E::ArrayLength(one)
        | E::RayQueryVertexPositions { query: one, .. }
        | E::RayQueryGetIntersection { query: one, .. } => f(one),
        E::ImageSample {
            image,
            sampler,
            coordinate,
            array_index,
            offset,
            level,
            depth_ref,
            gather: _,
            clamp_to_edge: _,
        } => {
            [image, sampler, coordinate].into_iter().for_each(&mut *f);
            match level {
                naga::SampleLevel::Auto | naga::SampleLevel::Zero => {}
                naga::SampleLevel::Exact(one) | naga::SampleLevel::Bias(one) => f(one),
                naga::SampleLevel::Gradient { x, y } => [x, y].into_iter().for_each(&mut *f),
            }
            [array_index, offset, depth_ref]
                .into_iter()
                .flatten()
                .for_each(f);
        }
        E::ImageLoad {
            image,
            coordinate,
            array_index,
            sample,
            level,
        } => {
            [image, coordinate].into_iter().for_each(&mut *f);
            [array_index, sample, level]
                .into_iter()
                .flatten()
                .for_each(f);
        }
        E::ImageQuery { image, query } => {
            f(image);
            if let naga::ImageQuery::Size { level: Some(level) } = query {
                f(level);
            }
        }
        E::Binary { left, right, .. } => [left, right].into_iter().for_each(f),
        E::Select {
            condition,
            accept,
            reject,
        } => [condition, accept, reject].into_iter().for_each(f),
        E::Math {
            arg,
            arg1,
            arg2,
            arg3,
            fun: _,
        } => {
            f(arg);
            [arg1, arg2, arg3].into_iter().flatten().for_each(f);
        }
        E::CooperativeLoad { data, .. } => [&mut data.pointer, &mut data.stride]
            .into_iter()
            .for_each(f),
        E::CooperativeMultiplyAdd { a, b, c } => [a, b, c].into_iter().for_each(f),
    }
}

/// Calls `f` on each expression the statements of `block` read, those of the blocks inside it
/// included. What a statement leaves its result in is not read, and is not passed.
fn reads(block: &mut Block, f: &mut impl FnMut(&mut Handle<E>)) {
    use naga::{GatherMode, RayPipelineFunction, RayQueryFunction};
    for statement in block.iter_mut() {
        match statement {
            S::Emit(_)
            | S::Break
            | S::Continue
            | S::Kill
            | S::ControlBarrier(_)
            | S::MemoryBarrier(_) => {}
            S::Block(inner) => reads(inner, f),
            S::If {
                condition,
                accept,
                reject,
            } => {
                f(condition);
                reads(accept, f);
                reads(reject, f);
            }
            S::Switch { selector, cases } => {
                f(selector);
                cases.iter_mut().for_each(|case| reads(&mut case.body, f));
            }
            S::Loop {
                body,
                continuing,
                break_if,
            } => {
                reads(body, f);
                reads(continuing, f);
                break_if.iter_mut().for_each(&mut *f);
            }
            S::Return { value } => value.iter_mut().for_each(&mut *f),
            S::Store { pointer, value } => [pointer, value].into_iter().for_each(&mut *f),
            S::ImageStore {
                image,
                coordinate,
                array_index,
                value,
            } => {
                [image, coordinate, value].into_iter().for_each(&mut *f);
                array_index.iter_mut().for_each(&mut *f);
            }
            S::Atomic {
                pointer,
                fun,
                value,
                result: _,
            } => {
                [pointer, value].into_iter().for_each(&mut *f);
                if let naga::AtomicFunction::Exchange {
                    compare: Some(compare),
                } = fun
                {
                    f(compare);
                }
            }
            S::ImageAtomic {
                image,
                coordinate,
                array_index,
                fun,
                value,
            } => {
                [image, coordinate, value].into_iter().for_each(&mut *f);
                array_index.iter_mut().for_each(&mut *f);
                if let naga::AtomicFunction::Exchange {
                    compare: Some(compare),
                } = fun
                {
                    f(compare);
                }
            }
            S::WorkGroupUniformLoad { pointer, result: _ } => f(pointer),
            S::Call {
                arguments,
                function: _,
                result: _,
            } => arguments.iter_mut().for_each(&mut *f),
            S::RayQuery { query, fun } => {
                f(query);
                match fun {
                    RayQueryFunction::Initialize {
                        acceleration_structure,
                        descriptor,
                    } => [acceleration_structure, descriptor]
                        .into_iter()
                        .for_each(&mut *f),
                    RayQueryFunction::GenerateIntersection { hit_t } => f(hit_t),
                    RayQueryFunction::Proceed { result: _ }
                    | RayQueryFunction::ConfirmIntersection
                    | RayQueryFunction::Terminate => {}
                }
            }
            S::RayPipelineFunction(RayPipelineFunction::TraceRay {
                acceleration_structure,
                descriptor,
                payload,
            }) => [acceleration_structure, descriptor, payload]
                .into_iter()
                .for_each(&mut *f),
            S::SubgroupBallot {
                predicate,
                result: _,
            } => predicate.iter_mut().for_each(&mut *f),
            S::SubgroupGather {
                mode,
                argument,
                result: _,
            } => {
                f(argument);
                match mode {
                    GatherMode::BroadcastFirst | GatherMode::QuadSwap(_) => {}
                    GatherMode::Broadcast(one)
                    | GatherMode::Shuffle(one)
                    | GatherMode::ShuffleDown(one)
                    | GatherMode::ShuffleUp(one)
                    | GatherMode::ShuffleXor(one)
                    | GatherMode::QuadBroadcast(one) => f(one),
                }
            }
            S::SubgroupCollectiveOperation {
                argument,
                op: _,
                collective_op: _,
                result: _,
            } => f(argument),
            S::CooperativeStore { target, data } => [target, &mut data.pointer, &mut data.stride]
                .into_iter()
                .for_each(&mut *f),
        }
    }
}
