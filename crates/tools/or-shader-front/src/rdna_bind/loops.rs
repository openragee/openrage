//! Whether each loop of a translated stage ends with what the editor binds: a device that waits
//! on a loop that never ends is lost, and takes the desktop's display with it.
//!
//! A loop is shown to end by an exit tested every turn whose outcome, turn by turn, is computed
//! from stated numbers and the words of bindings nothing can write, every counter it tests moved
//! by a stated step each turn; or by lying in a branch the device never takes.

mod fold;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use naga::valid::{Capabilities, FunctionInfo, ModuleInfo, ValidationFlags, Validator};
use naga::{
    ArraySize, BinaryOperator as B, Expression as E, Handle, ScalarKind, Statement as S,
    TypeInner as T,
};

use fold::Word;

/// The most turns a loop is shown to end within.
// One whose every exit is known and holds it past this many is `Ends::Runaway`.
pub const MOST_TURNS: u32 = 4096;

/// The loops the functions of `module` hold.
pub fn loops(module: &naga::Module) -> usize {
    bodies(module).len()
}

/// Each loop's body, in the order [`judge`] reports them: the functions, then the entry points,
/// each loop before the loops it holds.
fn bodies(module: &naga::Module) -> Vec<*const naga::Block> {
    fn of(block: &naga::Block, out: &mut Vec<*const naga::Block>) {
        for statement in block {
            match statement {
                S::Block(block) => of(block, out),
                S::If { accept, reject, .. } => {
                    of(accept, out);
                    of(reject, out);
                }
                S::Switch { cases, .. } => cases.iter().for_each(|case| of(&case.body, out)),
                S::Loop {
                    body, continuing, ..
                } => {
                    out.push(body as *const _);
                    of(body, out);
                    of(continuing, out);
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    functions(module).for_each(|(_, function)| of(&function.body, &mut out));
    out
}

/// The functions of `module`, then its entry points, each with its handle where it has one.
fn functions(
    module: &naga::Module,
) -> impl Iterator<Item = (Option<Handle<naga::Function>>, &naga::Function)> {
    (module.functions.iter().map(|(h, f)| (Some(h), f))).chain(
        module
            .entry_points
            .iter()
            .map(|entry| (None, &entry.function)),
    )
}

/// What the editor binds at a binding, as the device reads its words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Words {
    /// Nothing known of them: the renderer fills them from what it draws.
    Unknown,
    Zero,
    /// These bytes from the binding's start; past them nothing is known.
    Bytes(Vec<u8>),
    /// The word at each of these bytes; elsewhere nothing is known.
    Places(Vec<(u32, u32)>),
}

/// One binding of a stage as [`judge`] takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub at: (u32, u32),
    pub words: Words,
    /// The hash of the game's buffer whose numbers no exit of a loop may hang on.
    pub game: Option<u32>,
}

/// How a loop ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ends {
    /// It lies in a branch the device never takes.
    Dead,
    /// Within this many turns, by numbers the program states.
    Literal(u32),
    /// Within this many turns, by words of the bindings.
    Known(u32),
    /// Not shown to end: an exit hangs on what is not known when it is bound.
    Unshown,
    /// An exit of it hangs on a number of the game's buffer of this hash.
    Game(u32),
    /// Every exit is known, and none leaves within [`MOST_TURNS`] turns.
    Runaway,
}

impl Ends {
    fn rank(self) -> u8 {
        match self {
            Self::Dead => 0,
            Self::Literal(_) => 1,
            Self::Known(_) => 2,
            Self::Unshown => 3,
            Self::Game(_) => 4,
            Self::Runaway => 5,
        }
    }

    /// The worse of two ways a loop was seen to end, as two visits of it are joined.
    fn join(self, other: Self) -> Self {
        match (self, other) {
            (Self::Known(a) | Self::Literal(a), Self::Known(b))
            | (Self::Known(a), Self::Literal(b)) => Self::Known(a.max(b)),
            (Self::Literal(a), Self::Literal(b)) => Self::Literal(a.max(b)),
            _ if other.rank() > self.rank() => other,
            _ => self,
        }
    }
}

/// How each loop of a module ends, in the order of [`loops`], and the game's buffers its exits hang on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judged {
    pub ends: Vec<Ends>,
    pub hits: BTreeSet<u32>,
}

/// A set of the game's buffers, by their place in a chunk of at most 64 of them.
type Taint = u64;

/// How each loop of `module` ends with `bindings` bound; a binding not listed reads as unknown.
pub fn judge(module: &naga::Module, bindings: &[Binding]) -> Judged {
    let count = loops(module);
    let hits = BTreeSet::new();
    if count == 0 {
        return Judged {
            ends: Vec::new(),
            hits,
        };
    }
    let unshown = Judged {
        ends: vec![Ends::Unshown; count],
        hits: hits.clone(),
    };
    // Only to know each value's kind; whether the module is valid is the validator's to say.
    let Ok(info) = Validator::new(ValidationFlags::empty(), Capabilities::all()).validate(module)
    else {
        return unshown;
    };
    let game: Vec<(usize, u32)> = (bindings.iter().enumerate())
        .filter_map(|(at, binding)| binding.game.map(|hash| (at, hash)))
        .collect();
    let mut judged: Option<Judged> = None;
    // Every chunk walks alike but for which buffers it follows: a loop is the game's where any
    // chunk says so, named after the first.
    let chunks = game.chunks(64).map(<[_]>::to_vec);
    for chunk in chunks.chain(game.is_empty().then(Vec::new)) {
        let walked = walked(module, &info, bindings, &chunk);
        judged = Some(match judged {
            None => walked,
            Some(mut was) => {
                for (end, now) in was.ends.iter_mut().zip(walked.ends) {
                    if now.rank() > end.rank() {
                        *end = now;
                    }
                }
                was.hits.extend(walked.hits);
                was
            }
        });
    }
    judged.unwrap_or(unshown)
}

/// [`judge`] following the game's buffers of `chunk`, each a binding's place and its hash.
fn walked(
    module: &naga::Module,
    info: &ModuleInfo,
    bindings: &[Binding],
    chunk: &[(usize, u32)],
) -> Judged {
    let written = written(module);
    let mut words: BTreeMap<Handle<naga::GlobalVariable>, &Words> = BTreeMap::new();
    let mut taint: BTreeMap<Handle<naga::GlobalVariable>, Taint> = BTreeMap::new();
    for (handle, global) in module.global_variables.iter() {
        let at = (global.binding.as_ref()).map(|binding| (binding.group, binding.binding));
        let Some(place) = bindings.iter().position(|bound| Some(bound.at) == at) else {
            continue;
        };
        // ⚠ What a stage may store to has no words: another invocation, this one, or the frame
        // before may have written it.
        let writable = matches!(global.space, naga::AddressSpace::Storage { access }
            if access.contains(naga::StorageAccess::STORE));
        if !writable && !written.contains(&handle) {
            words.insert(handle, &bindings[place].words);
        }
        if let Some(bit) = chunk.iter().position(|(at, _)| *at == place) {
            taint.insert(handle, 1 << bit);
        }
    }
    let order: HashMap<*const naga::Block, usize> = (bodies(module).into_iter().enumerate())
        .map(|(at, body)| (body, at))
        .collect();
    let mut args: BTreeMap<Handle<naga::Function>, Vec<Taint>> = BTreeMap::new();
    let mut returns: BTreeMap<Handle<naga::Function>, Taint> = BTreeMap::new();
    let mut globals = taint;
    let mut ends = vec![Ends::Dead; order.len()];
    let mut exits: Taint = 0;
    // Until a walk of every function adds nothing: what a call or a global carries is joined
    // from every place that hands it on.
    for _ in 0..32 {
        let before = (args.clone(), returns.clone(), globals.clone());
        ends.fill(Ends::Dead);
        exits = 0;
        for (at, (handle, function)) in functions(module).enumerate() {
            let fi = match handle {
                Some(handle) => &info[handle],
                None => info.get_entry_point(at - module.functions.len()),
            };
            let mut flow = Flow {
                module,
                function,
                fi,
                bound: &words,
                globals: &globals,
                args: handle
                    .and_then(|h| args.get(&h))
                    .cloned()
                    .unwrap_or_default(),
                returns: &returns,
                order: &order,
                chunk,
                words: vec![None; function.expressions.len()],
                values: vec![0; function.expressions.len()],
                known: Known(vec![None; function.local_variables.len()]),
                locals: vec![0; function.local_variables.len()],
                stored: BTreeMap::new(),
                handed: BTreeMap::new(),
                returned: 0,
                ends: &mut ends,
                exits: 0,
            };
            flow.run();
            exits |= flow.exits;
            let (stored, handed, returned) = (flow.stored, flow.handed, flow.returned);
            for (global, t) in stored {
                *globals.entry(global).or_default() |= t;
            }
            for (callee, given) in handed {
                let into = args.entry(callee).or_default();
                into.resize(into.len().max(given.len()), 0);
                for (at, t) in given.into_iter().enumerate() {
                    into[at] |= t;
                }
            }
            if let Some(handle) = handle {
                *returns.entry(handle).or_default() |= returned;
            }
        }
        if (args.clone(), returns.clone(), globals.clone()) == before {
            break;
        }
    }
    let hits = (chunk.iter().enumerate())
        .filter(|(bit, _)| exits & (1 << bit) != 0)
        .map(|(_, (_, hash))| *hash)
        .collect();
    Judged { ends, hits }
}

/// The globals a statement of `module` stores to, whole or in part, or changes atomically.
fn written(module: &naga::Module) -> BTreeSet<Handle<naga::GlobalVariable>> {
    let mut out = BTreeSet::new();
    for (_, function) in functions(module) {
        let root = |pointer| global_root(function, pointer);
        each_statement(&function.body, &mut |statement| match statement {
            S::Store { pointer, .. } | S::Atomic { pointer, .. } => out.extend(root(*pointer)),
            S::ImageStore { image, .. } | S::ImageAtomic { image, .. } => out.extend(root(*image)),
            S::Call { arguments, .. } => out.extend(arguments.iter().filter_map(|a| root(*a))),
            _ => {}
        });
    }
    out
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

/// Whether `statement` holds a `continue` of the loop it lies in: a loop inside it holds its own.
fn continues(statement: &S) -> bool {
    match statement {
        S::Continue => true,
        S::Block(inner) => inner.iter().any(continues),
        S::If { accept, reject, .. } => accept.iter().chain(reject).any(continues),
        S::Switch { cases, .. } => cases.iter().any(|case| case.body.iter().any(continues)),
        _ => false,
    }
}

/// Whether `statement` holds a `break` of the loop it lies in: one in a `switch` leaves the switch.
fn breaks(statement: &S) -> bool {
    match statement {
        S::Break => true,
        S::Block(inner) => inner.iter().any(breaks),
        S::If { accept, reject, .. } => accept.iter().chain(reject).any(breaks),
        _ => false,
    }
}

/// Whether `statement` holds a `return`, in a loop inside it or not.
fn returns(statement: &S) -> bool {
    let any = |block: &naga::Block| block.iter().any(returns);
    match statement {
        S::Return { .. } => true,
        S::Block(inner) => any(inner),
        S::If { accept, reject, .. } => any(accept) || any(reject),
        S::Switch { cases, .. } => cases.iter().any(|case| any(&case.body)),
        S::Loop {
            body, continuing, ..
        } => any(body) || any(continuing),
        _ => false,
    }
}

/// Whether taking `block` always leaves the loop it lies in.
fn leaves(block: &naga::Block) -> bool {
    !block.iter().any(continues) && matches!(block.last(), Some(S::Break | S::Return { .. }))
}

/// The locals the statements of `blocks` store to, whole or in part, or hand to a call.
fn stored_in<'b>(
    function: &naga::Function,
    blocks: impl Iterator<Item = &'b naga::Block>,
) -> BTreeMap<Handle<naga::LocalVariable>, Vec<&'b S>> {
    let mut stored: BTreeMap<_, Vec<&S>> = BTreeMap::new();
    for block in blocks {
        each_statement(block, &mut |statement| match statement {
            S::Store { pointer, .. } | S::Atomic { pointer, .. } => {
                if let Some(local) = local_root(function, *pointer) {
                    stored.entry(local).or_default().push(statement);
                }
            }
            S::Call { arguments, .. } => {
                for local in arguments.iter().filter_map(|a| local_root(function, *a)) {
                    stored.entry(local).or_default().push(statement);
                }
            }
            _ => {}
        });
    }
    stored
}

fn local_root(
    function: &naga::Function,
    mut pointer: Handle<E>,
) -> Option<Handle<naga::LocalVariable>> {
    loop {
        pointer = match function.expressions[pointer] {
            E::LocalVariable(local) => return Some(local),
            E::Access { base, .. } | E::AccessIndex { base, .. } => base,
            _ => return None,
        };
    }
}

fn global_root(
    function: &naga::Function,
    mut pointer: Handle<E>,
) -> Option<Handle<naga::GlobalVariable>> {
    loop {
        pointer = match function.expressions[pointer] {
            E::GlobalVariable(global) => return Some(global),
            E::Access { base, .. } | E::AccessIndex { base, .. } => base,
            _ => return None,
        };
    }
}

/// `expression` under its conversions.
fn uncast(function: &naga::Function, mut expression: Handle<E>) -> Handle<E> {
    while let E::As { expr, .. } = function.expressions[expression] {
        expression = expr;
    }
    expression
}

/// The name the lowering gives the local that holds the low half of the lane mask.
const LANE_MASK: &str = "exec_lo";

/// The bits of each local of a function where they are known, by its index.
#[derive(Clone, PartialEq)]
struct Known(Vec<Option<Word>>);

impl Known {
    fn get(&self, local: &Handle<naga::LocalVariable>) -> Option<&Word> {
        self.0[local.index()].as_ref()
    }

    fn insert(&mut self, local: Handle<naga::LocalVariable>, word: Word) {
        self.0[local.index()] = Some(word);
    }

    fn remove(&mut self, local: &Handle<naga::LocalVariable>) {
        self.0[local.index()] = None;
    }

    /// Forgets the locals of `stored`.
    fn forget<T>(&mut self, stored: &BTreeMap<Handle<naga::LocalVariable>, T>) {
        for local in stored.keys() {
            self.remove(local);
        }
    }

    /// Keeps what `other` knows alike.
    fn meet(&mut self, other: &Self) {
        for (word, theirs) in self.0.iter_mut().zip(&other.0) {
            if word != theirs {
                *word = None;
            }
        }
    }
}

/// A local a loop moves by a stated step once a turn, at one place of the turn every turn
/// reaches, and stores nowhere else.
struct Counter {
    local: Handle<naga::LocalVariable>,
    /// Where in the turn it is stored.
    at: usize,
    /// The value stored, under its casts.
    moved: Handle<E>,
    start: Word,
    step: Word,
}

impl Counter {
    /// What it holds after `steps` steps.
    fn after(&self, steps: u32) -> Word {
        let moved = self.step.lanes[0].wrapping_mul(steps);
        Word::scalar(
            self.start.lanes[0].wrapping_add(moved),
            self.start.bound || self.step.bound,
        )
    }
}

/// What [`Flow::eval`] has computed: of what no counter moves, once; of the rest, this turn.
#[derive(Default)]
struct Memo {
    fixed: HashMap<Handle<E>, Option<Word>>,
    turn: HashMap<Handle<E>, Option<Word>>,
}

/// The operands of an expression [`Flow::compute`] folds.
fn operands(expression: &E) -> Vec<Handle<E>> {
    match *expression {
        E::Load { pointer: e }
        | E::AccessIndex { base: e, .. }
        | E::Swizzle { vector: e, .. }
        | E::Splat { value: e, .. }
        | E::As { expr: e, .. }
        | E::Unary { expr: e, .. }
        | E::Relational { argument: e, .. } => vec![e],
        E::Access { base, index } => vec![base, index],
        E::Binary { left, right, .. } => vec![left, right],
        E::Select {
            condition,
            accept,
            reject,
        } => vec![condition, accept, reject],
        E::Compose { ref components, .. } => components.clone(),
        E::Math {
            arg,
            arg1,
            arg2,
            arg3,
            ..
        } => [Some(arg), arg1, arg2, arg3]
            .into_iter()
            .flatten()
            .collect(),
        _ => Vec::new(),
    }
}

/// A turn of a loop as [`Flow::eval`] computes it.
struct Sim<'a> {
    counters: &'a [Counter],
    /// Where in the turn each expression a statement of it emits is emitted.
    emitted: &'a HashMap<Handle<E>, usize>,
    /// Every expression the loop emits, in a branch or not.
    inside: &'a BTreeSet<Handle<E>>,
    /// Those whose bits a counter's moves change.
    moving: &'a BTreeSet<Handle<E>>,
    turn: u32,
}

/// A step of a pointer from a global's start.
enum Step {
    Stated(u32),
    Computed(Handle<E>),
}

struct Flow<'m> {
    module: &'m naga::Module,
    function: &'m naga::Function,
    fi: &'m FunctionInfo,
    /// The words of each binding nothing can write.
    bound: &'m BTreeMap<Handle<naga::GlobalVariable>, &'m Words>,
    /// What each global carries of the game's buffers.
    globals: &'m BTreeMap<Handle<naga::GlobalVariable>, Taint>,
    /// What each argument of this function carries, from every call of it.
    args: Vec<Taint>,
    returns: &'m BTreeMap<Handle<naga::Function>, Taint>,
    order: &'m HashMap<*const naga::Block, usize>,
    chunk: &'m [(usize, u32)],
    /// By expression, as last evaluated, where its bits are known.
    words: Vec<Option<Word>>,
    /// By expression, as last evaluated.
    values: Vec<Taint>,
    /// The locals whose bits are known here.
    known: Known,
    /// What each local carries, by its index.
    locals: Vec<Taint>,
    stored: BTreeMap<Handle<naga::GlobalVariable>, Taint>,
    handed: BTreeMap<Handle<naga::Function>, Vec<Taint>>,
    returned: Taint,
    ends: &'m mut [Ends],
    /// What the exits of the loops walked hang on.
    exits: Taint,
}

impl Flow<'_> {
    fn run(&mut self) {
        let function = self.function;
        // What no statement emits stands from the start.
        for (expression, e) in function.expressions.iter() {
            let standing = matches!(
                e,
                E::GlobalVariable(_)
                    | E::Literal(_)
                    | E::Constant(_)
                    | E::ZeroValue(_)
                    | E::FunctionArgument(_)
                    | E::LocalVariable(_)
            );
            if standing {
                self.evaluate(expression);
            }
        }
        // A local with no initialiser is not taken for nought: that is a backend's to give.
        for (local, variable) in function.local_variables.iter() {
            if let Some(init) = variable.init {
                self.locals[local.index()] = self.of(init);
                if let Some(word) = self.words[init.index()] {
                    self.known.insert(local, word);
                }
            }
        }
        self.walk(&function.body);
    }

    fn of(&self, expression: Handle<E>) -> Taint {
        self.values[expression.index()]
    }

    /// The scalar kind of `expression`'s value, where it is a 32-bit scalar or vector or a bool.
    fn kind(&self, expression: Handle<E>) -> Option<ScalarKind> {
        match *self.fi[expression].ty.inner_with(&self.module.types) {
            T::Scalar(s) | T::Vector { scalar: s, .. }
                if s.width == 4 || s.kind == ScalarKind::Bool =>
            {
                Some(s.kind)
            }
            _ => None,
        }
    }

    fn lanes_of(&self, ty: Handle<naga::Type>) -> Option<u8> {
        match self.module.types[ty].inner {
            T::Scalar(s) if s.width == 4 || s.kind == ScalarKind::Bool => Some(1),
            T::Vector { size, scalar } if scalar.width == 4 || scalar.kind == ScalarKind::Bool => {
                Some(size as u8)
            }
            _ => None,
        }
    }

    /// What a global expression states: a literal or noughts.
    fn stated(&self, expression: Handle<E>) -> Option<Word> {
        match self.module.global_expressions[expression] {
            E::Literal(literal) => literal_word(literal),
            E::ZeroValue(ty) => self.lanes_of(ty).map(|n| Word {
                n,
                ..Word::scalar(0, false)
            }),
            _ => None,
        }
    }

    /// The bits of `expression` every device computes with what is bound, each operand's as
    /// `get` gives it.
    fn compute(
        &self,
        expression: Handle<E>,
        get: &mut dyn FnMut(Handle<E>) -> Option<Word>,
    ) -> Option<Word> {
        let vector = |e: Handle<E>| {
            matches!(
                *self.fi[e].ty.inner_with(&self.module.types),
                T::Vector { .. }
            )
        };
        match self.function.expressions[expression] {
            E::Literal(literal) => literal_word(literal),
            E::ZeroValue(ty) => self.lanes_of(ty).map(|n| Word {
                n,
                ..Word::scalar(0, false)
            }),
            E::Constant(constant) => self.stated(self.module.constants[constant].init),
            E::Load { pointer } => match self.function.expressions[pointer] {
                E::LocalVariable(local) => self.known.get(&local).copied(),
                _ => self.read(pointer, get),
            },
            E::AccessIndex { base, index } if vector(base) => {
                let word = get(base)?;
                (index < u32::from(word.n)).then(|| Word {
                    n: 1,
                    lanes: [word.lanes[index as usize]; 4],
                    ..word
                })
            }
            E::Access { base, index } if vector(base) => {
                let (word, at) = (get(base)?, get(index)?);
                let k = at.one()?;
                (k < u32::from(word.n)).then(|| Word {
                    n: 1,
                    lanes: [word.lanes[k as usize]; 4],
                    bound: word.bound || at.bound,
                })
            }
            E::Swizzle {
                size,
                vector,
                pattern,
            } => {
                let word = get(vector)?;
                let mut lanes = [0; 4];
                for (lane, component) in lanes.iter_mut().zip(&pattern[..size as usize]) {
                    *lane = *word.lanes().get(*component as usize)?;
                }
                Some(Word {
                    lanes,
                    n: size as u8,
                    ..word
                })
            }
            E::Splat { size, value } => {
                let word = get(value)?;
                (word.n == 1).then_some(Word {
                    n: size as u8,
                    ..word
                })
            }
            E::Compose { ty, ref components } => {
                self.lanes_of(ty)?;
                let mut out = Word {
                    n: 0,
                    ..Word::scalar(0, false)
                };
                for component in components {
                    let word = get(*component)?;
                    for lane in word.lanes() {
                        *out.lanes.get_mut(usize::from(out.n))? = *lane;
                        out.n += 1;
                    }
                    out.bound |= word.bound;
                }
                (out.n > 0).then_some(out)
            }
            E::As {
                expr,
                kind,
                convert,
            } => {
                let from = self.kind(expr)?;
                if convert.is_some_and(|width| width != 4 && kind != ScalarKind::Bool) {
                    return None;
                }
                get(expr)?.map(|v| fold::converted(from, kind, convert.is_none(), v))
            }
            E::Unary { op, expr } => {
                let kind = self.kind(expr)?;
                get(expr)?.map(|v| fold::unary(op, kind, v))
            }
            E::Binary { op, left, right } => {
                let kind = self.kind(left)?;
                let (l, r) = (get(left)?, get(right)?);
                l.zip(r, |a, b| fold::binary(op, kind, a, b))
            }
            E::Select {
                condition,
                accept,
                reject,
            } => match get(condition) {
                Some(c) if c.n == 1 => {
                    let picked = get(if c.lanes[0] != 0 { accept } else { reject })?;
                    Some(Word {
                        bound: picked.bound || c.bound,
                        ..picked
                    })
                }
                Some(c) => {
                    let (a, r) = (get(accept)?, get(reject)?);
                    let mut lanes = a.lanes;
                    for (k, lane) in lanes.iter_mut().enumerate().take(usize::from(a.n)) {
                        if c.lanes[k] == 0 {
                            *lane = r.lanes[k];
                        }
                    }
                    Some(Word {
                        lanes,
                        bound: a.bound || r.bound || c.bound,
                        ..a
                    })
                }
                None => {
                    let (a, r) = (get(accept)?, get(reject)?);
                    (a.lanes() == r.lanes()).then_some(Word { bound: true, ..a })
                }
            },
            E::Relational { fun, argument } => {
                let word = get(argument)?;
                let truth = match fun {
                    naga::RelationalFunction::All => word.lanes().iter().all(|v| *v != 0),
                    naga::RelationalFunction::Any => word.lanes().iter().any(|v| *v != 0),
                    _ => return None,
                };
                Some(Word::scalar(u32::from(truth), word.bound))
            }
            E::Math {
                fun: fun @ (naga::MathFunction::Min | naga::MathFunction::Max),
                arg,
                arg1: Some(arg1),
                ..
            } => {
                let kind = self.kind(arg)?;
                if !matches!(kind, ScalarKind::Uint | ScalarKind::Sint) {
                    return None;
                }
                let (a, b) = (get(arg)?, get(arg1)?);
                a.zip(b, |x, y| {
                    let less = match kind {
                        ScalarKind::Sint => (x as i32) < (y as i32),
                        _ => x < y,
                    };
                    Some(match (fun, less) {
                        (naga::MathFunction::Min, true) | (naga::MathFunction::Max, false) => x,
                        _ => y,
                    })
                })
            }
            _ => None,
        }
    }

    /// The words a load through `pointer` reads of a binding nothing can write, where its place
    /// in the binding is known.
    fn read(
        &self,
        mut pointer: Handle<E>,
        get: &mut dyn FnMut(Handle<E>) -> Option<Word>,
    ) -> Option<Word> {
        let mut steps = Vec::new();
        let global = loop {
            pointer = match self.function.expressions[pointer] {
                E::GlobalVariable(global) => break global,
                E::AccessIndex { base, index } => {
                    steps.push(Step::Stated(index));
                    base
                }
                E::Access { base, index } => {
                    steps.push(Step::Computed(index));
                    base
                }
                _ => return None,
            };
        };
        let words = self.bound.get(&global)?;
        let types = &self.module.types;
        let (mut ty, mut byte) = (self.module.global_variables[global].ty, 0u64);
        let mut lane = false;
        for step in steps.into_iter().rev() {
            let k = u64::from(match step {
                Step::Stated(k) => k,
                Step::Computed(e) => get(e)?.one()?,
            });
            if lane {
                return None;
            }
            match types[ty].inner {
                T::Struct { ref members, .. } => {
                    let member = members.get(usize::try_from(k).ok()?)?;
                    byte += u64::from(member.offset);
                    ty = member.ty;
                }
                // ⚠ Past a stated length a backend clamps the place or reads nought: neither is known.
                T::Array {
                    base, size, stride, ..
                } => {
                    if let ArraySize::Constant(n) = size {
                        if k >= u64::from(n.get()) {
                            return None;
                        }
                    }
                    byte += k.checked_mul(u64::from(stride))?;
                    ty = base;
                }
                T::Vector { size, scalar } if scalar.width == 4 && k < size as u64 => {
                    byte += 4 * k;
                    lane = true;
                }
                _ => return None,
            }
        }
        let n = match (lane, &types[ty].inner) {
            (true, _) => 1,
            (false, T::Scalar(s)) if s.width == 4 => 1,
            (false, T::Vector { size, scalar }) if scalar.width == 4 => *size as u8,
            _ => return None,
        };
        let mut out = Word {
            n,
            ..Word::scalar(0, true)
        };
        for (k, lane) in out.lanes.iter_mut().enumerate().take(usize::from(n)) {
            *lane = word_at(words, byte + 4 * k as u64)?;
        }
        Some(out)
    }

    /// Whether `condition` is the lowering's test of the lane's bit of the lane mask:
    /// `(exec_lo & 1u) != 0u`.
    fn lane_masked(&self, condition: Handle<E>) -> bool {
        let expressions = &self.function.expressions;
        let literal = |e: Handle<E>, n: u32| matches!(expressions[e], E::Literal(naga::Literal::U32(v)) if v == n);
        let E::Binary {
            op: B::NotEqual,
            left,
            right,
        } = expressions[condition]
        else {
            return false;
        };
        let E::Binary {
            op: B::And,
            left: mask,
            right: one,
        } = expressions[left]
        else {
            return false;
        };
        let mask = uncast(self.function, mask);
        let lane_mask = matches!(expressions[mask], E::Load { pointer }
            if matches!(expressions[pointer], E::LocalVariable(local)
                if self.function.local_variables[local].name.as_deref() == Some(LANE_MASK)));
        literal(right, 0) && literal(one, 1) && lane_mask
    }

    fn evaluate(&mut self, expression: Handle<E>) {
        self.words[expression.index()] = self.compute(expression, &mut |e| self.words[e.index()]);
        let of = |e: Handle<E>| self.of(e);
        let taint = match self.function.expressions[expression] {
            E::GlobalVariable(global) => self.globals.get(&global).copied().unwrap_or(0),
            E::FunctionArgument(at) => self.args.get(at as usize).copied().unwrap_or(0),
            E::Load { pointer } => {
                let held = local_root(self.function, pointer).map(|l| self.locals[l.index()]);
                of(pointer) | held.unwrap_or(0)
            }
            E::Access { base, index } => of(base) | of(index),
            E::AccessIndex { base, .. }
            | E::Splat { value: base, .. }
            | E::Swizzle { vector: base, .. }
            | E::Unary { expr: base, .. }
            | E::Derivative { expr: base, .. }
            | E::Relational { argument: base, .. }
            | E::As { expr: base, .. }
            | E::ArrayLength(base) => of(base),
            E::Compose { ref components, .. } => components.iter().fold(0, |all, e| all | of(*e)),
            E::Binary { left, right, .. } => of(left) | of(right),
            // What it picks between, as a branch is: the condition decides which, and a count
            // that is one of two numbers the game does not fill is not the game's.
            // A lane's write under the lane mask keeps the old value only for a lane the mask
            // leaves off, whose every result the mask leaves off too.
            E::Select {
                condition,
                accept,
                reject,
            } => match self.lane_masked(condition) {
                true => of(accept),
                false => of(accept) | of(reject),
            },
            E::Math {
                arg,
                arg1,
                arg2,
                arg3,
                ..
            } => [arg1, arg2, arg3]
                .into_iter()
                .flatten()
                .fold(of(arg), |all, e| all | of(e)),
            E::ImageSample {
                image,
                coordinate,
                array_index,
                ..
            }
            | E::ImageLoad {
                image,
                coordinate,
                array_index,
                ..
            } => of(image) | of(coordinate) | array_index.map_or(0, of),
            E::ImageQuery { image, .. } => of(image),
            _ => return,
        };
        self.values[expression.index()] = taint;
    }

    /// Gives a statement's result what it carries.
    fn result(&mut self, result: Option<Handle<E>>, taint: Taint) {
        if let Some(result) = result {
            self.values[result.index()] = taint;
            self.words[result.index()] = None;
        }
    }

    fn walk(&mut self, block: &naga::Block) {
        for statement in block {
            match *statement {
                S::Emit(ref range) => {
                    for expression in range.clone() {
                        self.evaluate(expression);
                    }
                }
                S::Block(ref block) => self.walk(block),
                S::Store { pointer, value } => {
                    let stored = self.of(value) | self.of(pointer);
                    if let Some(global) = global_root(self.function, pointer) {
                        *self.stored.entry(global).or_default() |= stored;
                        continue;
                    }
                    let Some(local) = local_root(self.function, pointer) else {
                        continue;
                    };
                    let whole = matches!(self.function.expressions[pointer], E::LocalVariable(_));
                    let held = &mut self.locals[local.index()];
                    *held = if whole { stored } else { *held | stored };
                    match self.words[value.index()] {
                        Some(word) if whole => self.known.insert(local, word),
                        _ => self.known.remove(&local),
                    };
                }
                S::Atomic {
                    pointer,
                    value,
                    result,
                    ref fun,
                } => {
                    let mut taint = self.of(pointer) | self.of(value);
                    if let naga::AtomicFunction::Exchange {
                        compare: Some(compare),
                    } = *fun
                    {
                        taint |= self.of(compare);
                    }
                    if let Some(global) = global_root(self.function, pointer) {
                        *self.stored.entry(global).or_default() |= taint;
                    }
                    if let Some(local) = local_root(self.function, pointer) {
                        self.locals[local.index()] |= taint;
                        self.known.remove(&local);
                    }
                    self.result(result, taint);
                }
                S::WorkGroupUniformLoad { pointer, result } => {
                    let taint = self.of(pointer);
                    self.result(Some(result), taint);
                }
                S::SubgroupBallot { result, predicate } => {
                    let taint = predicate.map_or(0, |p| self.of(p));
                    self.result(Some(result), taint);
                }
                S::SubgroupGather {
                    mode,
                    argument,
                    result,
                } => {
                    let index = match mode {
                        naga::GatherMode::Broadcast(i)
                        | naga::GatherMode::Shuffle(i)
                        | naga::GatherMode::ShuffleDown(i)
                        | naga::GatherMode::ShuffleUp(i)
                        | naga::GatherMode::ShuffleXor(i)
                        | naga::GatherMode::QuadBroadcast(i) => self.of(i),
                        _ => 0,
                    };
                    let taint = self.of(argument) | index;
                    self.result(Some(result), taint);
                }
                S::SubgroupCollectiveOperation {
                    argument, result, ..
                } => {
                    let taint = self.of(argument);
                    self.result(Some(result), taint);
                }
                S::Call {
                    function,
                    ref arguments,
                    result,
                } => {
                    let given: Vec<Taint> = arguments.iter().map(|e| self.of(*e)).collect();
                    let all = given.iter().fold(0, |all, t| all | t);
                    let into = self.handed.entry(function).or_default();
                    into.resize(into.len().max(given.len()), 0);
                    for (at, t) in given.iter().enumerate() {
                        into[at] |= t;
                    }
                    let read = self.returns.get(&function).copied().unwrap_or(0);
                    // A callee may store to a pointer it is handed whatever it reads.
                    let carried = read | all | self.globals.values().fold(0, |a, t| a | t);
                    self.result(result, read | all);
                    for argument in arguments {
                        if let Some(local) = local_root(self.function, *argument) {
                            self.known.remove(&local);
                            self.locals[local.index()] |= carried;
                        }
                    }
                }
                S::Return { value } => {
                    self.returned |= value.map_or(0, |v| self.of(v));
                }
                // A branch the device never takes is not followed.
                S::If {
                    condition,
                    ref accept,
                    ref reject,
                } => match self.words[condition.index()].and_then(|w| w.one()) {
                    Some(0) => self.walk(reject),
                    Some(_) => self.walk(accept),
                    None => {
                        let (before, known) = (self.locals.clone(), self.known.clone());
                        self.walk(accept);
                        let accepted = std::mem::replace(&mut self.locals, before);
                        let known_accepted = std::mem::replace(&mut self.known, known);
                        self.walk(reject);
                        self.join(accepted);
                        self.known.meet(&known_accepted);
                    }
                },
                // Every case, each from what the switch found: a case falls through from another.
                S::Switch { ref cases, .. } => {
                    let stored = stored_in(self.function, cases.iter().map(|case| &case.body));
                    self.known.forget(&stored);
                    let (before, known) = (self.locals.clone(), self.known.clone());
                    let mut after = before.clone();
                    for case in cases {
                        self.locals = before.clone();
                        self.known = known.clone();
                        self.walk(&case.body);
                        let ran = std::mem::replace(&mut self.locals, after);
                        self.join(ran);
                        after = self.locals.clone();
                    }
                    self.known = known;
                }
                S::Loop {
                    ref body,
                    ref continuing,
                    break_if,
                } => self.turns(body, continuing, break_if),
                _ => {}
            }
        }
    }

    fn turns(&mut self, body: &naga::Block, continuing: &naga::Block, break_if: Option<Handle<E>>) {
        let entry = self.known.clone();
        let stored = stored_in(self.function, [body, continuing].into_iter());
        self.known.forget(&stored);
        let start = self.known.clone();
        // Until a turn adds nothing: what a turn leaves, the next one starts from.
        loop {
            let before = self.locals.clone();
            self.known = start.clone();
            self.walk(body);
            self.walk(continuing);
            self.join(before.clone());
            if self.locals == before {
                break;
            }
        }
        let exits = self.counted_to(body, continuing, &stored);
        // Shown to end, a loop's other exits only leave sooner; not shown, it counts to the game's
        // number where a count of it is compared with one.
        let ends = match self.ends_by(body, continuing, break_if, &entry, &stored) {
            Ends::Unshown if exits != 0 => {
                self.exits |= exits;
                let bit = exits.trailing_zeros() as usize;
                Ends::Game(self.chunk.get(bit).map_or(0, |(_, hash)| *hash))
            }
            ends => ends,
        };
        if let Some(at) = self.order.get(&(body as *const _)) {
            self.ends[*at] = self.ends[*at].join(ends);
        }
        self.known = start;
    }

    /// What a count of the loop is compared with: a local it stores as itself and a stated number
    /// more or less, or that value, on one side of a comparison the loop makes.
    fn counted_to(
        &self,
        body: &naga::Block,
        continuing: &naga::Block,
        stored: &BTreeMap<Handle<naga::LocalVariable>, Vec<&S>>,
    ) -> Taint {
        let expressions = &self.function.expressions;
        let own = |e: Handle<E>, local| {
            matches!(expressions[uncast(self.function, e)], E::Load { pointer }
                if expressions[pointer] == E::LocalVariable(local))
        };
        let stated = |e: Handle<E>| matches!(expressions[uncast(self.function, e)], E::Literal(_));
        let (mut counts, mut moved) = (BTreeSet::new(), BTreeSet::new());
        for (local, stores) in stored {
            for statement in stores {
                let S::Store { value, .. } = statement else {
                    continue;
                };
                let value = uncast(self.function, *value);
                let E::Binary {
                    op: B::Add | B::Subtract,
                    left,
                    right,
                } = expressions[value]
                else {
                    continue;
                };
                let steps =
                    (own(left, *local) && stated(right)) || (stated(left) && own(right, *local));
                if steps {
                    counts.insert(*local);
                    moved.insert(value);
                }
            }
        }
        let counted = |e: Handle<E>| {
            let e = uncast(self.function, e);
            moved.contains(&e) || counts.iter().any(|local| own(e, *local))
        };
        let mut taint = 0;
        for block in [body, continuing] {
            each_statement(block, &mut |statement| {
                let S::Emit(range) = statement else {
                    return;
                };
                for e in range.clone() {
                    let E::Binary { op, left, right } = expressions[e] else {
                        continue;
                    };
                    let compares = matches!(
                        op,
                        B::Equal
                            | B::NotEqual
                            | B::Less
                            | B::LessEqual
                            | B::Greater
                            | B::GreaterEqual
                    );
                    if compares && counted(left) {
                        taint |= self.of(right);
                    }
                    if compares && counted(right) {
                        taint |= self.of(left);
                    }
                }
            });
        }
        taint
    }

    /// How the loop ends by its exits tested at places every turn reaches, turn by turn.
    fn ends_by(
        &self,
        body: &naga::Block,
        continuing: &naga::Block,
        break_if: Option<Handle<E>>,
        entry: &Known,
        stored: &BTreeMap<Handle<naga::LocalVariable>, Vec<&S>>,
    ) -> Ends {
        let turn: Vec<&S> = body.iter().chain(continuing).collect();
        // Past a statement that may `continue`, the rest of the body is not reached every turn;
        // the continuing block is.
        let reached = body
            .iter()
            .position(continues)
            .map_or(body.len(), |at| at + 1);
        let every = |at: usize| at < reached || at >= body.len();
        let mut emitted: HashMap<Handle<E>, usize> = HashMap::new();
        for (at, statement) in turn.iter().enumerate() {
            if let S::Emit(range) = statement {
                emitted.extend(range.clone().map(|e| (e, at)));
            }
        }
        let mut inside: BTreeSet<Handle<E>> = BTreeSet::new();
        for block in [body, continuing] {
            each_statement(block, &mut |statement| {
                if let S::Emit(range) = statement {
                    inside.extend(range.clone());
                }
            });
        }
        let counters = self.counters(&turn, &emitted, entry, stored, &every);
        // Each exit tested where every turn reaches it: what it tests, and on which outcome it
        // leaves; none tests nothing.
        let mut exits: Vec<(Option<Handle<E>>, bool)> = Vec::new();
        // Whether the loop may also leave where no exit tested here says.
        let mut elsewhere = false;
        for (at, statement) in turn.iter().enumerate() {
            match statement {
                S::Break | S::Return { .. } if every(at) => exits.push((None, true)),
                S::If {
                    condition,
                    accept,
                    reject,
                } if every(at) => {
                    for (branch, leaving) in [(accept, true), (reject, false)] {
                        if leaves(branch) {
                            exits.push((Some(*condition), leaving));
                        } else {
                            elsewhere |= branch.iter().any(|s| breaks(s) || returns(s));
                        }
                    }
                }
                _ => elsewhere |= breaks(statement) || returns(statement),
            }
        }
        exits.extend(break_if.map(|e| (Some(e), true)));
        // Without a counter every turn tests alike.
        let moving = self.moving(&counters, &inside);
        let mut memo = Memo::default();
        let mut live = vec![true; exits.len()];
        let mut known = true;
        for turn in 1..=MOST_TURNS {
            let sim = Sim {
                counters: &counters,
                emitted: &emitted,
                inside: &inside,
                moving: &moving,
                turn,
            };
            memo.turn.clear();
            // An exit no counter moves tests alike every turn: once is enough. A turn none of
            // whose exits is known ends the walk: the loop is not shown to end.
            let (mut moved, mut told) = (false, false);
            for ((condition, leaving), live) in exits.iter().zip(live.iter_mut()) {
                if !*live {
                    continue;
                }
                let Some(condition) = condition else {
                    return Ends::Literal(turn);
                };
                let leaves = self.eval(*condition, &sim, &mut memo).and_then(|word| {
                    let v = word.one()?;
                    Some(((v != 0) == *leaving, word.bound))
                });
                match leaves {
                    Some((true, false)) => return Ends::Literal(turn),
                    Some((true, true)) => return Ends::Known(turn),
                    Some((false, _)) => told = true,
                    None => known = false,
                }
                *live = moving.contains(condition);
                moved |= *live;
            }
            if !moved || !told {
                break;
            }
        }
        match known && !elsewhere {
            true => Ends::Runaway,
            false => Ends::Unshown,
        }
    }

    /// The counters of a loop whose turn is `turn`: each local the loop stores once, whole, at a
    /// place every turn reaches, as what it held that turn and a stated step.
    fn counters(
        &self,
        turn: &[&S],
        emitted: &HashMap<Handle<E>, usize>,
        entry: &Known,
        stored: &BTreeMap<Handle<naga::LocalVariable>, Vec<&S>>,
        every: &dyn Fn(usize) -> bool,
    ) -> Vec<Counter> {
        let mut counters = Vec::new();
        for (local, stores) in stored {
            let [statement @ S::Store { pointer, value }] = stores.as_slice() else {
                continue;
            };
            let Some(at) = turn.iter().position(|s| std::ptr::eq(*s, *statement)) else {
                continue;
            };
            let whole = matches!(self.function.expressions[*pointer], E::LocalVariable(_));
            let moved = uncast(self.function, *value);
            let E::Binary { op, left, right } = self.function.expressions[moved] else {
                continue;
            };
            // What it held this turn: loaded in this turn, before it is stored.
            let own = |e: Handle<E>| {
                let e = uncast(self.function, e);
                let loaded = matches!(self.function.expressions[e], E::Load { pointer }
                    if self.function.expressions[pointer] == E::LocalVariable(*local));
                loaded && emitted.get(&e).is_some_and(|loaded| *loaded < at)
            };
            let step = match (op, own(left), own(right)) {
                (B::Add, true, false) => self.words[right.index()],
                (B::Add, false, true) => self.words[left.index()],
                (B::Subtract, true, false) => self.words[right.index()]
                    .map(|w| Word::scalar(w.lanes[0].wrapping_neg(), w.bound)),
                _ => None,
            };
            let integer = matches!(self.kind(moved), Some(ScalarKind::Uint | ScalarKind::Sint))
                && matches!(self.kind(*value), Some(ScalarKind::Uint | ScalarKind::Sint));
            let (Some(step), Some(start)) = (step.filter(|w| w.n == 1), entry.get(local)) else {
                continue;
            };
            if whole && integer && every(at) && start.n == 1 && step.lanes[0] != 0 {
                counters.push(Counter {
                    local: *local,
                    at,
                    moved,
                    start: *start,
                    step,
                });
            }
        }
        counters
    }

    /// The expressions of a loop whose bits a counter's moves change.
    fn moving(&self, counters: &[Counter], inside: &BTreeSet<Handle<E>>) -> BTreeSet<Handle<E>> {
        let mut moving = BTreeSet::new();
        // An operand stands before what takes it.
        for expression in inside {
            let e = &self.function.expressions[*expression];
            let counted = counters.iter().any(|c| {
                c.moved == *expression
                    || matches!(*e, E::Load { pointer }
                        if self.function.expressions[pointer] == E::LocalVariable(c.local))
            });
            if counted || operands(e).iter().any(|o| moving.contains(o)) {
                moving.insert(*expression);
            }
        }
        moving
    }

    /// The bits of `expression` in turn `sim.turn` of a loop, its counters moved so far.
    fn eval(&self, expression: Handle<E>, sim: &Sim, memo: &mut Memo) -> Option<Word> {
        let fixed = !sim.moving.contains(&expression);
        let held = match fixed {
            true => memo.fixed.get(&expression),
            false => memo.turn.get(&expression),
        };
        if let Some(word) = held {
            return *word;
        }
        let inside = sim.inside.contains(&expression);
        let counter = |local| sim.counters.iter().find(|c| c.local == local);
        let word = match self.function.expressions[expression] {
            _ if sim.counters.iter().any(|c| c.moved == expression) => {
                (sim.counters.iter().find(|c| c.moved == expression)).map(|c| c.after(sim.turn))
            }
            E::Load { pointer } if inside => match self.function.expressions[pointer] {
                E::LocalVariable(local) => match (counter(local), sim.emitted.get(&expression)) {
                    (Some(c), Some(at)) => Some(c.after(sim.turn - u32::from(*at < c.at))),
                    // A counter loaded where not every turn loads it, or another local the
                    // loop stores, is not known past the walk's own words.
                    (Some(_), None) => None,
                    (None, _) => self.words[expression.index()],
                },
                _ => match self.words[expression.index()] {
                    Some(word) => Some(word),
                    None => self.read(pointer, &mut |e| self.eval(e, sim, memo)),
                },
            },
            _ if !inside => self.words[expression.index()],
            _ => match self.words[expression.index()] {
                Some(word) => Some(word),
                None => self.compute(expression, &mut |e| self.eval(e, sim, memo)),
            },
        };
        match fixed {
            true => memo.fixed.insert(expression, word),
            false => memo.turn.insert(expression, word),
        };
        word
    }

    fn join(&mut self, other: Vec<Taint>) {
        for (held, taint) in self.locals.iter_mut().zip(other) {
            *held |= taint;
        }
    }
}

fn literal_word(literal: naga::Literal) -> Option<Word> {
    let bits = match literal {
        naga::Literal::U32(v) => v,
        naga::Literal::I32(v) => v as u32,
        naga::Literal::F32(v) => v.to_bits(),
        naga::Literal::Bool(v) => u32::from(v),
        _ => return None,
    };
    Some(Word::scalar(bits, false))
}

/// The word at `byte` of a binding, where it is known.
fn word_at(words: &Words, byte: u64) -> Option<u32> {
    match words {
        Words::Unknown => None,
        Words::Zero => Some(0),
        Words::Bytes(bytes) => {
            let at = usize::try_from(byte).ok()?;
            let four = bytes.get(at..at.checked_add(4)?)?;
            Some(u32::from_le_bytes(four.try_into().ok()?))
        }
        Words::Places(places) => (places.iter())
            .find(|(at, _)| u64::from(*at) == byte)
            .map(|(_, word)| *word),
    }
}

/// The bindings of `places` an exit of a loop of `module` hangs on, no binding's words known.
// The cache's guard asks it of a module before and after the optimiser: the binder's question
// with no feed known, a proxy of the binder's verdict, which takes the feeds.
pub fn counted_to_each(module: &naga::Module, places: &[(u32, u32)]) -> BTreeSet<(u32, u32)> {
    if loops(module) == 0 {
        return BTreeSet::new();
    }
    let bindings: Vec<Binding> = (places.iter().zip(0..))
        .map(|(at, hash)| Binding {
            at: *at,
            words: Words::Unknown,
            game: Some(hash),
        })
        .collect();
    (judge(module, &bindings).hits.into_iter())
        .filter_map(|hash| places.get(hash as usize).copied())
        .collect()
}
