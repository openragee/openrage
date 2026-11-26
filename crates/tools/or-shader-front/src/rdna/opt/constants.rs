//! Constants. A dword and a condition are followed bit by bit through the registers: what comes
//! to one value becomes a literal, a `select` on a known condition its arm, an operator that
//! leaves an operand as it was that operand, and an `if` on a known condition the arm it takes.
//! In the arms of an `if` the condition says what it was computed from, so a test of the lane
//! mask leaves the mask's bit set in one arm and clear in the other, for as long as the mask
//! holds the value that was tested.
//!
//! Only dwords and conditions are folded, never a float. A shift by 32 or more and a division
//! by nought stay as they are: a device decides those. A turn of a loop starts with what the
//! loop was entered with or what the turn before left. A function called from one place is
//! walked there, as part of its caller; what any other call stores to is forgotten at it. A
//! mask another lane's vote or a wave's turn decides stays unknown.

use std::collections::HashMap;

use naga::front::Typifier;
use naga::proc::ResolveContext;
use naga::{
    Arena, BinaryOperator as Bo, Block, Expression as E, Handle, Literal, Range, Scalar,
    ScalarKind, Statement as S, TypeInner, UnaryOperator as Uo,
};

use super::Registers;

/// What an expression or a variable is to this pass.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Other,
    /// A `u32`.
    Word,
    /// A `bool`: bit 0 of a dword whose other bits are clear.
    Flag,
}

fn kind_of(inner: &TypeInner) -> Kind {
    match *inner {
        TypeInner::Scalar(Scalar::U32) => Kind::Word,
        TypeInner::Scalar(Scalar::BOOL) => Kind::Flag,
        _ => Kind::Other,
    }
}

/// The bits of a dword known clear and known set.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Known {
    zero: u32,
    one: u32,
}

impl Known {
    const NONE: Self = Self { zero: 0, one: 0 };

    const fn word(word: u32) -> Self {
        Self {
            zero: !word,
            one: word,
        }
    }

    /// The dword, where every bit of it is known.
    fn whole(self) -> Option<u32> {
        (self.zero | self.one == u32::MAX).then_some(self.one)
    }

    fn either(self, other: Self) -> Self {
        Self {
            zero: self.zero & other.zero,
            one: self.one & other.one,
        }
    }
}

/// An expression of a function, the function by its place among the module's.
type Root = (u32, Handle<E>);

/// Bit 0 of a value is bit 0 of what expression `root` came to, or its reverse.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Truth {
    root: Root,
    same: bool,
}

fn reverse(truth: Option<Truth>) -> Option<Truth> {
    truth.map(|truth| Truth {
        same: !truth.same,
        ..truth
    })
}

/// What a register holds at a point of a function.
#[derive(Clone, Copy)]
struct Held {
    known: Known,
    /// What the value it holds was computed from, where a load or a store has said.
    truth: Option<Truth>,
    /// The expression of the function being walked whose value it holds, where one is known.
    holds: Option<Handle<E>>,
}

// ponytail: a copy of every variable for each `if`; an undo log if a module of very many
// private registers makes this the slow pass.
type State = Vec<Held>;

/// What the pass needs of one function that only the whole module says.
pub(super) struct Plan {
    /// By expression.
    kinds: Vec<Kind>,
    /// By variable, the globals then the locals: what every store of the module leaves known
    /// of a register of dwords or conditions, and `None` of anything else.
    floors: Vec<Option<Known>>,
}

/// The bits `at` may hold set, by how it is made of numbers, registers and logic over those.
fn may(expressions: &Arena<E>, at: Handle<E>, depth: u8, load: &impl Fn(Handle<E>) -> u32) -> u32 {
    let Some(depth) = depth.checked_sub(1) else {
        return u32::MAX;
    };
    let of = |at| may(expressions, at, depth, load);
    match expressions[at] {
        E::Literal(Literal::U32(word)) => word,
        E::Load { pointer } => load(pointer),
        E::Binary {
            op: Bo::And,
            left,
            right,
        } => of(left) & of(right),
        E::Binary {
            op: Bo::InclusiveOr | Bo::ExclusiveOr,
            left,
            right,
        } => of(left) | of(right),
        E::Select { accept, reject, .. } => of(accept) | of(reject),
        _ => u32::MAX,
    }
}

pub(super) fn plan(module: &naga::Module, registers: &Registers) -> Vec<Plan> {
    let kind = |ty| kind_of(&module.types[ty].inner);
    let globals = module.global_variables.len();
    // By function, then by variable: its kind where it is a register, and the bits a register
    // of dwords may ever hold set. A variable starts as nought where it has no initialiser.
    let (mut kinds, mut bits) = (Vec::new(), Vec::new());
    for (index, function) in super::functions(module).enumerate() {
        let variables = (module.global_variables.iter())
            .map(|(_, global)| (global.ty, global.init.is_some()))
            .chain(
                (function.local_variables.iter())
                    .map(|(_, local)| (local.ty, local.init.is_some())),
            );
        let is_register = |var: usize| match var.checked_sub(globals) {
            None => registers.globals[var],
            Some(local) => registers.locals[index][local],
        };
        let (of_kinds, of_bits): (Vec<Kind>, Vec<u32>) = (variables.enumerate())
            .map(|(var, (ty, starts))| match (is_register(var), kind(ty)) {
                (true, Kind::Word) if !starts => (Kind::Word, 0),
                (true, kind) => (kind, u32::MAX),
                (false, _) => (Kind::Other, u32::MAX),
            })
            .unzip();
        kinds.push(of_kinds);
        bits.push(of_bits);
    }
    // A global's bits are every function's: they are kept in the first function's row.
    let place = |index: usize, var: usize| match var < globals {
        true => (0, var),
        false => (index, var),
    };
    let mut stores: Vec<Vec<(usize, Handle<E>)>> = Vec::new();
    for (index, function) in super::functions(module).enumerate() {
        let mut own = Vec::new();
        super::statements(&function.body, &mut |statement| {
            if let S::Store { pointer, value } = *statement {
                let var = registers.var(index, &function.expressions, pointer);
                own.extend(var.map(|var| (var, value)));
            }
        });
        stores.push(own);
    }
    let mut settled = false;
    for _ in 0..64 {
        let mut grew = false;
        for (index, function) in super::functions(module).enumerate() {
            for &(var, value) in &stores[index] {
                let (row, at) = place(index, var);
                let held = bits[row][at];
                if held == u32::MAX {
                    continue;
                }
                let load = |pointer| match registers.var(index, &function.expressions, pointer) {
                    Some(var) => {
                        let (row, at) = place(index, var);
                        bits[row][at]
                    }
                    None => u32::MAX,
                };
                let stored = may(&function.expressions, value, 6, &load);
                if stored & !held != 0 {
                    bits[row][at] = held | stored;
                    grew = true;
                }
            }
        }
        if !grew {
            settled = true;
            break;
        }
    }
    (super::functions(module).enumerate())
        .map(|(index, function)| {
            let floors = (kinds[index].iter().enumerate())
                .map(|(var, kind)| {
                    let (row, at) = place(index, var);
                    match kind {
                        Kind::Other => None,
                        Kind::Flag => Some(Known { zero: !1, one: 0 }),
                        // A count of rounds that ran out says nothing of any register.
                        Kind::Word if !settled => Some(Known::NONE),
                        Kind::Word => Some(Known {
                            zero: !bits[row][at],
                            one: 0,
                        }),
                    }
                })
                .collect();
            let expressions = &function.expressions;
            let context =
                ResolveContext::with_locals(module, &function.local_variables, &function.arguments);
            let mut types = Typifier::new();
            // A module the validator took resolves; one that does not is left as it is.
            let resolved = (expressions.iter().next_back())
                .is_some_and(|(last, _)| types.grow(last, expressions, &context).is_ok());
            let kinds = (expressions.iter())
                .map(|(at, _)| match resolved {
                    true => kind_of(types.get(at, &module.types)),
                    false => Kind::Other,
                })
                .collect();
            Plan { kinds, floors }
        })
        .collect()
}

/// The most global variables a module may have for a function called from one place to be
/// walked there, as part of its caller.
// ponytail: each `if` copies what every register holds, so a module of very many private
// registers walked as one function takes minutes. A state that copies only what an arm
// changes lifts this; until then such a module's functions are each walked alone.
const WALKED_IN_PLACE: usize = 2048;

/// The most expressions a module may compute for a function called under a loop to be walked
/// where it is called.
// ponytail: the start of a turn is found by walking copies of the loop, and a function under
// it is copied and walked with each: minutes in a module of some hundred thousand
// expressions. What a function does to the registers, kept for each start, lifts this.
const WALKED_UNDER_A_LOOP: usize = 1 << 16;

/// A loop or a `switch` being walked, and what holds where a statement under it leaves it.
struct Around {
    turns: bool,
    /// Where a `break` leaves it.
    left: Option<State>,
    /// Where a `continue` ends a turn of it.
    next: Option<State>,
}

/// The function being walked.
struct Of<'a> {
    function: usize,
    plan: &'a Plan,
    expressions: Arena<E>,
    /// By expression, as each is emitted.
    known: Vec<Known>,
    truths: Vec<Truth>,
    /// By expression: the expression its readers read in its place.
    same: Vec<Option<Handle<E>>>,
}

impl<'a> Of<'a> {
    fn new(function: usize, plan: &'a Plan, expressions: Arena<E>) -> Self {
        let count = expressions.len();
        let mut of = Self {
            function,
            plan,
            expressions,
            known: vec![Known::NONE; count],
            truths: Vec::with_capacity(count),
            same: vec![None; count],
        };
        for (at, expression) in of.expressions.iter() {
            of.truths.push(Truth {
                root: (function as u32, at),
                same: true,
            });
            match *expression {
                E::Literal(Literal::U32(word)) => of.known[at.index()] = Known::word(word),
                E::Literal(Literal::Bool(flag)) => {
                    of.known[at.index()] = Known::word(u32::from(flag));
                }
                _ => {}
            }
        }
        of
    }

    /// Has every reader under `body` read what stands for what it read; the expressions.
    fn finish(self, body: &mut Block) -> Arena<E> {
        let Self {
            mut expressions,
            same,
            ..
        } = self;
        let mut turn = |read: &mut Handle<E>| {
            if let Some(same) = same[read.index()] {
                *read = same;
            }
        };
        for (_, expression) in expressions.iter_mut() {
            super::operands(expression, &mut turn);
        }
        super::reads(body, &mut turn);
        expressions
    }
}

struct Walk<'a> {
    registers: &'a Registers,
    plans: &'a [Plan],
    /// By function: its expressions and its body, but while it is being walked.
    parts: Vec<Option<(Arena<E>, Block)>>,
    /// By function: whether it is walked where the one statement that calls it stands.
    once: Vec<bool>,
    of: Of<'a>,
    /// For each loop whose start is being found, the starts found of the loops under it, in
    /// the order the walk meets them, each with how many registers its loop stores to.
    finding: Vec<Vec<(usize, State)>>,
    /// The starts found of the loops under the loop being rewritten, the next one first.
    found: std::collections::VecDeque<(usize, State)>,
    /// For each function being walked as part of its caller, what holds where it returns.
    returns: Vec<Option<State>>,
    /// Bit 0 of what an expression came to, as the branches around the walk say it.
    learned: HashMap<Root, bool>,
    /// What each thing learned replaced, in order: a block forgets what it learned.
    log: Vec<(Root, Option<bool>)>,
    /// The loops and `switch`es the walk is inside, the innermost last.
    around: Vec<Around>,
    /// Whether the walk only looks: it is of a copy of a loop, to find what a turn starts with.
    dry: bool,
    /// Literals made, selects and idle operators read past, branches taken, and guarded
    /// writes whose condition is known.
    changed: [usize; 5],
}

impl Walk<'_> {
    /// Takes bit 0 of what `root` came to as `bit`, until the block being walked ends.
    fn learn(&mut self, root: Root, bit: bool) {
        let before = self.learned.insert(root, bit);
        self.log.push((root, before));
    }

    /// What is known of what `at` came to, with what the branches around this point say of
    /// what it was computed from: an expression emitted before a branch is read inside it.
    fn known_at(&self, at: Handle<E>) -> Known {
        let mut known = self.of.known[at.index()];
        let of = self.of.truths[at.index()];
        if let Some(&bit) = self.learned.get(&of.root) {
            if self.of.plan.kinds[at.index()] != Kind::Other {
                let bit = u32::from(bit == of.same);
                known.one |= bit;
                known.zero |= bit ^ 1;
            }
        }
        known
    }

    /// Writes what is learned into what each register holds, for where it is no longer learned.
    fn settle(&self, state: &mut State) {
        if self.learned.is_empty() {
            return;
        }
        for held in state.iter_mut() {
            let learned = held
                .truth
                .and_then(|of| Some((of, *self.learned.get(&of.root)?)));
            if let Some((of, bit)) = learned {
                let bit = u32::from(bit == of.same);
                held.known.one |= bit;
                held.known.zero |= bit ^ 1;
            }
        }
    }

    /// The register `pointer` is, where this pass follows it.
    fn register(&self, pointer: Handle<E>) -> Option<usize> {
        (self
            .registers
            .var(self.of.function, &self.of.expressions, pointer))
        .filter(|&var| self.of.plan.floors[var].is_some())
    }

    /// Leaves `var` holding what every store of the module leaves known of it, and no more.
    fn forget(&mut self, state: &mut State, var: usize) {
        if let Some(known) = self.of.plan.floors[var] {
            state[var] = Held {
                known,
                truth: None,
                holds: None,
            };
        }
    }

    /// The registers this pass follows that the statements under `blocks` may store to.
    fn stored(&self, blocks: &[&Block]) -> Vec<usize> {
        let mut stored = Vec::new();
        for block in blocks {
            (self.registers).stored(self.of.function, &self.of.expressions, block, &mut stored);
        }
        stored.sort_unstable();
        stored.dedup();
        stored.retain(|&var| self.of.plan.floors[var].is_some());
        stored
    }

    /// One turn of a loop that starts with `head`: what holds where it ends, to start the
    /// next, and what holds where a `break` leaves the loop.
    fn turn(
        &mut self,
        body: &mut Block,
        continuing: &mut Block,
        head: State,
    ) -> [Option<State>; 2] {
        self.around.push(Around {
            turns: true,
            left: None,
            next: None,
        });
        let fell = self.block(body, head, None);
        let Some(around) = self.around.pop() else {
            return [None, None];
        };
        let ended = match Self::either(fell, around.next) {
            Some(state) => self.block(continuing, state, None),
            None => None,
        };
        [ended, around.left]
    }

    /// Walks a loop entered with `head`; what holds after it, and `None` where nothing leaves
    /// it. A turn starts with what the loop was entered with or what the turn before left:
    /// copies of the loop are walked until a turn adds nothing to that, and where eight walks
    /// do not get there, with what every store of the module leaves its registers.
    // ⚠ A loop under a loop is not walked again for each walk of the outer one's last copy: the
    // starts found there are kept in order and read back when the outer loop is rewritten.
    // Walked again, a loop under `n` others is walked two to the `n` times at the least.
    fn turns(
        &mut self,
        body: &mut Block,
        continuing: &mut Block,
        leaves_at_end: bool,
        mut head: State,
    ) -> Option<State> {
        let stored = self.stored(&[&*body, &*continuing]);
        // What reaches the start by two ways is a value nothing was computed from: what the
        // branches say of the value it holds now is written into it first.
        self.settle(&mut head);
        for &var in &stored {
            (head[var].truth, head[var].holds) = (None, None);
        }
        let after = |[ended, left]: [Option<State>; 2]| match leaves_at_end {
            true => Self::either(ended, left),
            false => left,
        };
        let floors = |head: &mut State, plan: &Plan| {
            for &var in &stored {
                head[var].known = plan.floors[var].unwrap_or(Known::NONE);
            }
        };
        if !self.dry {
            if let Some((count, found)) = self.found.pop_front() {
                // The walk that found this loop's start is being repeated, to rewrite.
                match count == stored.len() && found.len() == head.len() {
                    true => stored
                        .iter()
                        .for_each(|&var| head[var].known = found[var].known),
                    false => floors(&mut head, self.of.plan),
                }
                return after(self.turn(body, continuing, head));
            }
        }
        let dry = std::mem::replace(&mut self.dry, true);
        let mut settled = None;
        for round in 0..9 {
            if round == 8 {
                floors(&mut head, self.of.plan);
            }
            self.finding.push(Vec::new());
            let [ended, left] = self.turn(&mut body.clone(), &mut continuing.clone(), head.clone());
            let under = self.finding.pop().unwrap_or_default();
            // What every store leaves is what a turn starts with, whatever a walk finds.
            let mut grew = false;
            for &var in ended.iter().filter(|_| round < 8).flat_map(|_| &stored) {
                let ended = ended
                    .as_ref()
                    .map_or(head[var].known, |ended| ended[var].known);
                let either = head[var].known.either(ended);
                grew |= either != head[var].known;
                head[var].known = either;
            }
            if !grew {
                settled = Some(([ended, left], under));
                break;
            }
        }
        self.dry = dry;
        let (ends, under) = settled?;
        if let Some(outer) = self.finding.last_mut().filter(|_| dry) {
            // Under a loop whose own start is being found: that walk is this loop's last.
            outer.push((stored.len(), head));
            outer.extend(under);
            return after(ends);
        }
        self.found = under.into();
        let ends = self.turn(body, continuing, head);
        self.found.clear();
        after(ends)
    }

    fn eval(&mut self, at: Handle<E>, state: &mut State) {
        let kind = self.of.plan.kinds[at.index()];
        let known_of = |at: Handle<E>| self.known_at(at);
        let truth_of = |at: Handle<E>| Some(self.of.truths[at.index()]);
        let (mut known, mut truth) = (Known::NONE, None);
        // The operand an operator leaves as it was, and the arm a `select` takes.
        let (mut idle, mut arm) = (None, None);
        match self.of.expressions[at] {
            E::Load { pointer } => {
                if let Some(var) = self.register(pointer) {
                    known = state[var].known;
                    // A register that nothing is known to stand for is, from here on, known
                    // as this load of it.
                    let as_loaded = Truth {
                        root: (self.of.function as u32, at),
                        same: true,
                    };
                    truth = Some(*state[var].truth.get_or_insert(as_loaded));
                    // As the forwarding has it: an expression made before this load still
                    // stands for what the register holds, and one made after it cannot.
                    if state[var]
                        .holds
                        .is_none_or(|held| held.index() > at.index())
                    {
                        state[var].holds = Some(at);
                    }
                }
            }
            E::Unary {
                op: Uo::BitwiseNot | Uo::LogicalNot,
                expr,
            } => {
                let of = known_of(expr);
                known = Known {
                    zero: of.one,
                    one: of.zero,
                };
                if kind == Kind::Flag {
                    known = Known {
                        zero: known.zero | !1,
                        one: known.one & 1,
                    };
                }
                truth = reverse(truth_of(expr));
            }
            E::As {
                expr,
                kind: ScalarKind::Uint,
                convert: None,
            } if self.of.plan.kinds[expr.index()] == Kind::Word => {
                (known, truth, idle) = (known_of(expr), truth_of(expr), Some(expr));
            }
            E::Select {
                condition,
                accept,
                reject,
            } => match known_of(condition).whole() {
                Some(flag) => {
                    let taken = if flag & 1 == 1 { accept } else { reject };
                    (known, truth, arm) = (known_of(taken), truth_of(taken), Some(taken));
                }
                None => {
                    let (taken, left) = (known_of(accept), known_of(reject));
                    known = taken.either(left);
                    // One and nought, or nought and one, are the condition and its reverse.
                    truth = match taken.whole().zip(left.whole()) {
                        Some((taken, left)) if taken & 1 == 1 && left & 1 == 0 => {
                            truth_of(condition)
                        }
                        Some((taken, left)) if taken & 1 == 0 && left & 1 == 1 => {
                            reverse(truth_of(condition))
                        }
                        _ => None,
                    };
                }
            },
            E::Binary { op, left, right } => {
                let (l, r) = (known_of(left), known_of(right));
                let (of_left, of_right) = (truth_of(left), truth_of(right));
                let both = l.whole().zip(r.whole());
                match op {
                    Bo::And | Bo::LogicalAnd => {
                        known = Known {
                            zero: l.zero | r.zero,
                            one: l.one & r.one,
                        };
                        // A number that keeps every bit the other side may hold set.
                        if r.whole().is_some() && (r.one | l.zero) == u32::MAX {
                            idle = Some(left);
                        } else if l.whole().is_some() && (l.one | r.zero) == u32::MAX {
                            idle = Some(right);
                        }
                        truth = match () {
                            () if r.one & 1 == 1 => of_left,
                            () if l.one & 1 == 1 => of_right,
                            () => None,
                        };
                    }
                    Bo::InclusiveOr | Bo::LogicalOr => {
                        known = Known {
                            zero: l.zero & r.zero,
                            one: l.one | r.one,
                        };
                        if r.whole().is_some() && (r.zero | l.one) == u32::MAX {
                            idle = Some(left);
                        } else if l.whole().is_some() && (l.zero | r.one) == u32::MAX {
                            idle = Some(right);
                        }
                        truth = match () {
                            () if r.zero & 1 == 1 => of_left,
                            () if l.zero & 1 == 1 => of_right,
                            () => None,
                        };
                    }
                    Bo::ExclusiveOr => {
                        let seen = (l.zero | l.one) & (r.zero | r.one);
                        let bits = l.one ^ r.one;
                        known = Known {
                            zero: !bits & seen,
                            one: bits & seen,
                        };
                        if r.whole() == Some(0) {
                            idle = Some(left);
                        } else if l.whole() == Some(0) {
                            idle = Some(right);
                        }
                        truth = match () {
                            () if r.zero & 1 == 1 => of_left,
                            () if r.one & 1 == 1 => reverse(of_left),
                            () if l.zero & 1 == 1 => of_right,
                            () if l.one & 1 == 1 => reverse(of_right),
                            () => None,
                        };
                    }
                    Bo::ShiftLeft | Bo::ShiftRight => match r.whole() {
                        // A shift by 32 or more is a device's to decide.
                        Some(by) if by < 32 => {
                            known = match op {
                                Bo::ShiftLeft => Known {
                                    zero: l.zero << by | !(u32::MAX << by),
                                    one: l.one << by,
                                },
                                _ => Known {
                                    zero: l.zero >> by | !(u32::MAX >> by),
                                    one: l.one >> by,
                                },
                            };
                            if by == 0 {
                                idle = Some(left);
                            }
                        }
                        _ => {}
                    },
                    Bo::Add | Bo::Subtract | Bo::Multiply | Bo::Divide | Bo::Modulo => {
                        let folded = both.and_then(|(a, b)| match op {
                            Bo::Add => Some(a.wrapping_add(b)),
                            Bo::Subtract => Some(a.wrapping_sub(b)),
                            Bo::Multiply => Some(a.wrapping_mul(b)),
                            // A division by nought is a device's to decide.
                            Bo::Divide => a.checked_div(b),
                            _ => a.checked_rem(b),
                        });
                        match folded {
                            Some(word) => known = Known::word(word),
                            // Nought added or taken away.
                            None if matches!(op, Bo::Add | Bo::Subtract)
                                && r.whole() == Some(0) =>
                            {
                                idle = Some(left);
                            }
                            None if op == Bo::Add && l.whole() == Some(0) => idle = Some(right),
                            None => {}
                        }
                    }
                    Bo::Equal | Bo::NotEqual => {
                        // A bit known set on one side and clear on the other.
                        let differ = l.one & r.zero != 0 || l.zero & r.one != 0;
                        // A dword or a condition is itself; a float that is no number is not.
                        let itself =
                            left == right && self.of.plan.kinds[left.index()] != Kind::Other;
                        let equal = match both {
                            Some((a, b)) => Some(a == b),
                            None if differ => Some(false),
                            None if itself => Some(true),
                            None => None,
                        };
                        if let Some(equal) = equal {
                            known = Known::word(u32::from(equal == (op == Bo::Equal)));
                        }
                        // Against nought or one, a value of one bit is the answer or its reverse.
                        let (other, number, of) = match (l.whole(), r.whole()) {
                            (_, Some(number)) => (l, number, of_left),
                            (Some(number), _) => (r, number, of_right),
                            _ => (Known::NONE, 2, None),
                        };
                        if number < 2 && (other.zero | 1) == u32::MAX {
                            truth = match (number == 1) == (op == Bo::Equal) {
                                true => of,
                                false => reverse(of),
                            };
                        }
                    }
                    Bo::Less | Bo::LessEqual | Bo::Greater | Bo::GreaterEqual => {
                        if let Some((a, b)) = both {
                            known = Known::word(u32::from(match op {
                                Bo::Less => a < b,
                                Bo::LessEqual => a <= b,
                                Bo::Greater => a > b,
                                _ => a >= b,
                            }));
                        }
                    }
                }
            }
            _ => {}
        }
        // Bits are followed through dwords and conditions only: a signed number shifts and
        // compares by other rules, and a float is never folded.
        match kind {
            Kind::Other => (known, truth, idle) = (Known::NONE, None, None),
            // A condition is its bit 0, whatever else is known of it.
            Kind::Flag => known.zero |= !1,
            Kind::Word => {}
        }
        self.of.known[at.index()] = known;
        self.of.truths[at.index()] = truth.unwrap_or(Truth {
            root: (self.of.function as u32, at),
            same: true,
        });
        let known = self.known_at(at);
        let literal = known.whole().filter(|_| arm.is_none());
        if self.dry {
        } else if let Some(word) = literal {
            if !matches!(self.of.expressions[at], E::Literal(_)) {
                self.of.expressions[at] = E::Literal(match kind {
                    Kind::Flag => Literal::Bool(word & 1 == 1),
                    _ => Literal::U32(word),
                });
                self.changed[0] += 1;
            }
        } else if let Some(read) = arm.or(idle) {
            // What stands for an earlier expression stands for this one.
            self.of.same[at.index()] = Some(self.of.same[read.index()].unwrap_or(read));
            self.changed[if arm.is_some() { 1 } else { 2 }] += 1;
        }
    }

    fn either(taken: Option<State>, left: Option<State>) -> Option<State> {
        let (mut taken, left) = match (taken, left) {
            (Some(taken), Some(left)) => (taken, left),
            (taken, left) => return taken.or(left),
        };
        for (held, other) in taken.iter_mut().zip(left) {
            held.known = held.known.either(other.known);
            if held.truth != other.truth {
                held.truth = None;
            }
            if held.holds != other.holds {
                held.holds = None;
            }
        }
        Some(taken)
    }

    /// Walks `block` and leaves what it becomes in its place, with bit 0 of what an
    /// expression came to as `says` gives it. `None` where no path leaves its end: what stood
    /// after that point is gone from it.
    fn block(
        &mut self,
        block: &mut Block,
        state: State,
        says: Option<(Root, bool)>,
    ) -> Option<State> {
        let mark = self.log.len();
        if let Some((root, bit)) = says {
            self.learn(root, bit);
        }
        let mut out = Block::with_capacity(block.len());
        let mut state = self.run(std::mem::take(block), &mut out, state);
        *block = out;
        if self.log.len() > mark {
            if let Some(state) = &mut state {
                self.settle(state);
            }
            for (root, before) in self.log.split_off(mark).into_iter().rev() {
                match before {
                    Some(bit) => self.learned.insert(root, bit),
                    None => self.learned.remove(&root),
                };
            }
        }
        state
    }

    /// Walks the statements of `block` into `out`.
    fn run(&mut self, block: Block, out: &mut Block, mut state: State) -> Option<State> {
        for (mut statement, span) in block.span_into_iter() {
            match &mut statement {
                S::Emit(range) => {
                    // A literal is in scope from the start: no statement emits one, so a
                    // range that holds one now is cut into the runs between.
                    let (mut runs, mut run) = (Vec::new(), None::<(Handle<E>, Handle<E>)>);
                    let mut cut = false;
                    for at in range.clone() {
                        self.eval(at, &mut state);
                        if !matches!(self.of.expressions[at], E::Literal(_)) {
                            run = Some((run.map_or(at, |(first, _)| first), at));
                        } else {
                            cut = true;
                            runs.extend(run.take());
                        }
                    }
                    if cut {
                        for (first, last) in runs.into_iter().chain(run) {
                            out.push(S::Emit(Range::new_from_bounds(first, last)), span);
                        }
                        continue;
                    }
                }
                S::Block(inner) => {
                    let after = self.block(inner, state, None);
                    out.push(statement, span);
                    state = after?;
                    continue;
                }
                S::If {
                    condition,
                    accept,
                    reject,
                } => {
                    if let Some(flag) = self.known_at(*condition).whole() {
                        let arm = if flag & 1 == 1 { accept } else { reject };
                        self.changed[3] += usize::from(!self.dry);
                        state = self.run(std::mem::take(arm), out, state)?;
                        continue;
                    }
                    let Truth { root, same } = self.of.truths[condition.index()];
                    let taken = self.block(accept, state.clone(), Some((root, same)));
                    let left = self.block(reject, state, Some((root, !same)));
                    out.push(statement, span);
                    // Past an arm whose end nothing leaves, the other arm's side still holds.
                    match (&taken, &left) {
                        (None, Some(_)) => self.learn(root, !same),
                        (Some(_), None) => self.learn(root, same),
                        _ => {}
                    }
                    state = Self::either(taken, left)?;
                    continue;
                }
                S::Switch { cases, .. } => {
                    // A case entered from the case before it starts with what that one left:
                    // where one is, every case starts knowing nothing of what a case stores.
                    let stored = match cases.iter().any(|case| case.fall_through) {
                        true => {
                            let bodies: Vec<&Block> = cases.iter().map(|case| &case.body).collect();
                            self.stored(&bodies)
                        }
                        false => Vec::new(),
                    };
                    stored.iter().for_each(|&var| self.forget(&mut state, var));
                    self.around.push(Around {
                        turns: false,
                        left: None,
                        next: None,
                    });
                    // It is left with no case run where none is the default.
                    let default =
                        |case: &naga::SwitchCase| matches!(case.value, naga::SwitchValue::Default);
                    let mut after = match cases.iter().any(default) {
                        true => None,
                        false => Some(state.clone()),
                    };
                    for case in cases.iter_mut() {
                        let ended = self.block(&mut case.body, state.clone(), None);
                        after = Self::either(after, ended);
                    }
                    let left = self.around.pop().and_then(|around| around.left);
                    after = Self::either(after, left);
                    out.push(statement, span);
                    state = after?;
                    // A case entered from the one before may have left more than was seen.
                    stored.iter().for_each(|&var| self.forget(&mut state, var));
                    continue;
                }
                S::Loop {
                    body,
                    continuing,
                    break_if,
                } => {
                    let after = self.turns(body, continuing, break_if.is_some(), state);
                    out.push(statement, span);
                    state = after?;
                    continue;
                }
                S::Store { pointer, value } => {
                    if let Some(var) = self.register(*pointer) {
                        let stands = |at: Handle<E>| self.of.same[at.index()].unwrap_or(at);
                        // A write that keeps what the register held unless a condition
                        // holds, as a lane that does not run keeps its register.
                        if let E::Select {
                            condition, reject, ..
                        } = self.of.expressions[*value]
                        {
                            let guards = state[var].holds == Some(stands(reject));
                            let decided = self.known_at(condition).whole().is_some();
                            self.changed[4] += usize::from(guards && decided && !self.dry);
                        }
                        state[var] = Held {
                            known: self.known_at(*value),
                            truth: Some(self.of.truths[value.index()]),
                            holds: Some(stands(*value)),
                        };
                    }
                }
                S::Call { function, .. }
                    if self.once[function.index()] && self.parts[function.index()].is_some() =>
                {
                    let returns = self.inside(function.index(), &mut state);
                    out.push(statement, span);
                    if !returns {
                        return None;
                    }
                    continue;
                }
                S::Call { function, .. } => {
                    let stores = &self.registers.stores[function.index()];
                    let stored: Vec<usize> = (0..stores.len()).filter(|&var| stores[var]).collect();
                    stored.iter().for_each(|&var| self.forget(&mut state, var));
                }
                S::Break | S::Continue | S::Return { .. } => {
                    // A `break` leaves the innermost loop or `switch`, a `continue` ends a
                    // turn of the innermost loop.
                    let around = match statement {
                        S::Break => self.around.len().checked_sub(1),
                        S::Continue => self.around.iter().rposition(|around| around.turns),
                        _ => None,
                    };
                    // It is read where what the branches here say is no longer learned.
                    self.settle(&mut state);
                    if let (S::Return { .. }, Some(returned)) =
                        (&statement, self.returns.last_mut())
                    {
                        *returned = Self::either(returned.take(), Some(state));
                    } else if let Some(around) = around {
                        let to = match statement {
                            S::Break => self.around[around].left.take(),
                            _ => self.around[around].next.take(),
                        };
                        let joined = Self::either(to, Some(state));
                        match statement {
                            S::Break => self.around[around].left = joined,
                            _ => self.around[around].next = joined,
                        }
                    }
                    out.push(statement, span);
                    return None;
                }
                _ => {}
            }
            out.push(statement, span);
        }
        Some(state)
    }
}

impl Walk<'_> {
    /// What nothing is known of but what every store of the module leaves.
    fn unknown(floor: &Option<Known>) -> Held {
        Held {
            known: floor.unwrap_or(Known::NONE),
            truth: None,
            holds: None,
        }
    }

    /// Walks function `index` as nothing but itself: its registers may hold anything at its start.
    fn alone(&mut self, index: usize) {
        let Some((expressions, mut body)) = self.parts[index].take() else {
            return;
        };
        let plans = self.plans;
        let plan = &plans[index];
        let outer = std::mem::replace(&mut self.of, Of::new(index, plan, expressions));
        self.block(
            &mut body,
            plan.floors.iter().map(Self::unknown).collect(),
            None,
        );
        let of = std::mem::replace(&mut self.of, outer);
        self.parts[index] = Some((of.finish(&mut body), body));
    }

    /// Walks function `called` where its one call stands, with what the registers of the
    /// module hold in `state` there, and leaves in `state` what they hold where it returns;
    /// whether it returns.
    fn inside(&mut self, called: usize, state: &mut State) -> bool {
        let Some((expressions, mut body)) = self.parts[called].take() else {
            return true;
        };
        let globals = self.registers.globals.len();
        let plans = self.plans;
        let plan = &plans[called];
        // An expression of the caller's is none of this function's.
        let mut inner = state[..globals].to_vec();
        inner.iter_mut().for_each(|held| held.holds = None);
        inner.extend(plan.floors[globals..].iter().map(Self::unknown));
        let outer = std::mem::replace(&mut self.of, Of::new(called, plan, expressions));
        // A `break` of the function leaves nothing of its caller's.
        let around = std::mem::take(&mut self.around);
        self.returns.push(None);
        // A walk that only looks walks a copy, as it does of a loop.
        let mut copy = self.dry.then(|| body.clone());
        let fell = self.block(copy.as_mut().unwrap_or(&mut body), inner, None);
        let returned = self.returns.pop().flatten();
        self.around = around;
        let of = std::mem::replace(&mut self.of, outer);
        self.parts[called] = Some(match self.dry {
            true => (of.expressions, body),
            false => (of.finish(&mut body), body),
        });
        match Self::either(fell, returned) {
            Some(after) => {
                state[..globals].copy_from_slice(&after[..globals]);
                state[..globals]
                    .iter_mut()
                    .for_each(|held| held.holds = None);
                true
            }
            None => false,
        }
    }
}

/// Calls `f` with each function a statement under `block` calls, and whether the call stands
/// under a loop, `under` for the block itself.
fn called(block: &Block, under: bool, f: &mut impl FnMut(usize, bool)) {
    for statement in block.iter() {
        match statement {
            S::Call { function, .. } => f(function.index(), under),
            S::Block(inner) => called(inner, under, f),
            S::If { accept, reject, .. } => {
                called(accept, under, f);
                called(reject, under, f);
            }
            S::Switch { cases, .. } => cases.iter().for_each(|case| called(&case.body, under, f)),
            S::Loop {
                body, continuing, ..
            } => {
                called(body, true, f);
                called(continuing, true, f);
            }
            _ => {}
        }
    }
}

/// Runs the pass over every function of `module`: how many literals it made, selects and idle
/// operators it read past, branches it took, and guarded writes it met with a known condition.
pub(super) fn fold(module: &mut naga::Module, registers: &Registers) -> [usize; 5] {
    let plans = plan(module, registers);
    let Some(first) = plans.first() else {
        return [0; 5];
    };
    let mut calls = vec![0u32; plans.len()];
    // By function: whether a call to it, or to a function that calls it, stands under a loop.
    let mut looped = vec![false; plans.len()];
    let all: Vec<_> = super::functions(module).collect();
    // A function is declared before the functions that call it.
    for (index, function) in all.iter().enumerate().rev() {
        let under = looped[index];
        called(&function.body, under, &mut |function, under| {
            calls[function] += 1;
            looped[function] |= under;
        });
    }
    let small = super::emitted(module) <= WALKED_UNDER_A_LOOP;
    let parts = super::functions_mut(module)
        .map(|function| {
            let expressions = std::mem::take(&mut function.expressions);
            Some((expressions, std::mem::take(&mut function.body)))
        })
        .collect();
    let mut walk = Walk {
        registers,
        plans: &plans,
        parts,
        once: (calls.iter().zip(&looped))
            .map(|(&calls, &looped)| {
                calls == 1 && (!looped || small) && registers.globals.len() <= WALKED_IN_PLACE
            })
            .collect(),
        of: Of::new(0, first, Arena::new()),
        finding: Vec::new(),
        found: std::collections::VecDeque::new(),
        returns: Vec::new(),
        learned: HashMap::new(),
        log: Vec::new(),
        around: Vec::new(),
        dry: false,
        changed: [0; 5],
    };
    for index in 0..plans.len() {
        if !walk.once[index] {
            walk.alone(index);
        }
    }
    // A function whose one call is in what a branch never reaches was not walked: it is as it was.
    let Walk { parts, changed, .. } = walk;
    for (function, part) in super::functions_mut(module).zip(parts) {
        if let Some((expressions, body)) = part {
            function.expressions = expressions;
            function.body = body;
        }
    }
    changed
}
