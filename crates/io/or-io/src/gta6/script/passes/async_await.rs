//! Async: a state machine's step function back to the body of the `async` method it was compiled
//! from, each `await` one statement.
//!
//! Two steps around `structure`. `untangle` works on the blocks: where the whole machine is
//! proved it takes the dispatch, the suspend paths, the resume points and the end out, so that
//! loops structure as the source had them, and leaves a call of the piece intrinsic where each
//! stood. `run` puts every piece in the place of its call as a `Sugar::Await` or a
//! `Sugar::Hidden`, so that `tree::desugar` gives a body that runs as the step function did.
//!
//! The proof reads no name. A machine is known by its protocol: the state at offset 0 tested where
//! the function starts, state `k` stored and the awaiter kept in a field before control is handed
//! back, and a resume point that only the dispatch reaches, takes the awaiter back and goes on
//! where a ready awaiter goes.
//!
//! A kick-off that holds its machine in a local and runs the step function's code in line is
//! read the same way, through the variable it keeps the machine's address in (`kick`), and a
//! kick-off that calls its step function takes that function's body where the call stood
//! (`joined`). How a machine ends, and what that says the method hands its caller, is in `ends`;
//! an await of the task the statement before it makes, and how an await reads, in `place`.
//!
//! ⚠ Between `untangle` and `run` the blocks hold those calls and no dispatch: they run as the
//! function did only once `run` has put the pieces back.
//! ⚠ `iterators` and `wait` switch nothing: an iterator's step is not proved by one function, and
//! a script's wait is an await like any other.

#[path = "async_await_ends.rs"]
mod ends;
#[path = "async_await_frame.rs"]
mod frame;
#[path = "async_await_kick.rs"]
mod kick;
#[path = "async_await_place.rs"]
mod place;

use std::collections::{BTreeMap, BTreeSet};

use crate::gta6::script::cfg::{self, Block, BlockId, Cfg, Terminator, ENTRY};
use crate::gta6::script::ir::{
    BinOp, Callee, Const, Expr, Kind, Returns, Scalar, Stmt, StoreKind, UnOp, Var,
};
use crate::gta6::script::tree::{self, Label, Sugar};
use crate::gta6::script::{Context, Pass, Stats};

pub use self::ends::{handed_back, result, AsyncResult};
pub use self::kick::{hide_machines, joined, joined_steps, joins, Join};
pub use self::place::{reading, Reading, Taken};

/// The names this file's rewrites are switched by.
pub const PASSES: &[Pass] = &[Pass::Await, Pass::Iterators, Pass::Wait];

/// Where a state machine holds its builder, the task inside the builder, and its first field.
const BUILDER: u32 = 0x8;
const TASK: u32 = 0x10;
const FIELDS: u32 = 0x18;
/// The states no await owns: the machine is running, the machine is finished.
const RUNNING: i32 = -1;
const FINISHED: i32 = -2;
/// The intrinsic whose call stands in the blocks for a piece taken out.
pub(super) const PIECE: &str = "state_machine_piece";
/// The intrinsic that copies a value into a new object, which a first suspension does to its machine.
const BOX: &str = "box_at";
/// The sweeps given to finding the copies of the machine's address; a chain of copies longer than this leaves the function as it is.
const SWEEPS: usize = 8;

/// What `untangle` took out of a step function's blocks, for `run` to put back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Machine {
    /// Each construct taken out; the blocks hold a call of the piece intrinsic with its index in its place.
    pub pieces: Vec<tree::Stmt>,
    /// Where a kick-off holds the machine, for `run` to write the body by.
    kick: Option<kick::Settle>,
}

/// How a function reaches its state machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Reach {
    /// The variable that holds the machine's address: a step function's one parameter, or the copy a kick-off keeps of the address of its own machine.
    pointer: Var,
    /// The local a kick-off holds its machine in.
    storage: Option<Var>,
    /// The variable the state was read into.
    state: Option<Var>,
}

impl Reach {
    /// A step function's: the machine's address is its parameter, and its state is not read yet.
    const STEP: Reach = Reach {
        pointer: Var::Param(0),
        storage: None,
        state: None,
    };
}

/// Why a step function's state machine is left as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
enum Decline {
    #[error("the dispatch is no chain of tests of the state against a constant")]
    Dispatch,
    #[error("the suspend states are not 0 to n-1, each with a resume point of its own")]
    States,
    #[error("a resume point is reached from elsewhere than the dispatch, or does not jump on")]
    Resume,
    #[error("a resume point does more than take its awaiter back and set the state to running")]
    ResumeBody,
    #[error(
        "a state has not exactly one suspend path that leads on to where its resume point does"
    )]
    Suspend,
    #[error("a suspend path does more than store the state and the awaiter and hand control back")]
    SuspendBody,
    #[error("the test before a suspend path is no call of one runtime method on the awaiter")]
    Ready,
    #[error("a block is part of two pieces of the machine")]
    Shared,
    #[error("the state or the machine's address is used outside the machine's own code")]
    StateUse,
    #[error("a piece taken out is not reached once from the entry")]
    Unreached,
    #[error("the entry does more than read the machine into the frame")]
    Entry,
    #[error("a variable of the frame set before an await is read after it")]
    Frame,
}

/// What an address is taken from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Base {
    /// The state machine.
    Machine,
    /// An object made on the path that holds the address.
    Made,
    /// A variable of the frame.
    Frame,
}

/// The variables of one path that hold an address: what each is taken from and how many bytes into it.
struct Held {
    reach: Reach,
    vars: BTreeMap<Var, (Base, u32)>,
}

impl Held {
    /// No variable yet, in a function that reaches its machine as `reach` says.
    fn of(reach: Reach) -> Held {
        Held {
            reach,
            vars: BTreeMap::new(),
        }
    }
}

/// One await: the blocks it was compiled to.
struct Site {
    /// The block that tests the awaiter, and the block it goes on to where the awaiter is ready.
    test: BlockId,
    on: BlockId,
    /// Where the suspend path starts.
    suspend: BlockId,
    /// The blocks of a first suspension made in line: the one that makes the task, the one that hands control back.
    first: Option<(BlockId, BlockId)>,
    resume: BlockId,
    /// The variable the suspend path keeps in the machine as the awaiter.
    kept: Option<Var>,
    /// Whether the block the await goes on in starts by asking the awaiter for its result.
    fetch: bool,
}

impl Site {
    /// The blocks that leave the function with this await.
    fn taken(&self) -> Vec<BlockId> {
        let (make, back) = self.first.unzip();
        [Some(self.resume), Some(self.suspend), make, back]
            .into_iter()
            .flatten()
            .collect()
    }
}

/// What a suspend path stores, and the blocks of a first suspension it makes in line.
struct Path {
    state: i32,
    slot: u32,
    first: Option<(BlockId, BlockId)>,
    awaiter: Option<Var>,
}

/// What a suspend path has stored so far: the state, the field the awaiter is kept in, and the variable kept there.
#[derive(Default)]
struct Kept {
    state: Option<i32>,
    slot: Option<u32>,
    awaiter: Option<Var>,
}

/// What the tests of the state at a function's start say.
struct Dispatch {
    /// The blocks after the entry that hold a test of the state and nothing else.
    links: Vec<BlockId>,
    /// The block each state the tests name resumes in.
    resumes: BTreeMap<i32, BlockId>,
    /// Where a first call starts.
    start: BlockId,
}

/// The end of a machine: the block that stores the finished state, the statement that does, and how the task is completed after it.
struct End {
    block: BlockId,
    from: usize,
    finish: Finish,
}

/// How a machine completes its task once it has stored the finished state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Finish {
    /// In line: the test of the builder's task ends the block, and its two arms leave.
    InLine,
    /// Through a function given the builder's address: the statements before `upto` are the compiler's alone.
    // A call that is given a result too stays in sight, and `upto` stops before it.
    Through { upto: usize },
}

/// A proved machine: what `take_out` takes, by block.
struct Plan {
    links: Vec<BlockId>,
    start: BlockId,
    /// The awaits, by state.
    sites: Vec<Site>,
    ends: Vec<End>,
    /// The blocks nothing is left of.
    gone: Vec<bool>,
    /// One flag for each statement a kick-off's entry starts with: whether it goes out of sight.
    hide: Vec<bool>,
    /// What every way out of the function returns.
    leave: Option<Expr>,
}

/// Takes a proved state machine's dispatch and resume paths out of the blocks; `None`, with the blocks untouched, where none is proved.
pub fn untangle(f: &mut cfg::Function, cx: &Context<'_>, _stats: &mut Stats) -> Option<Machine> {
    if !cx.options.on(Pass::Await) {
        return None;
    }
    let entry = f.cfg.blocks.first()?;
    let step = Reach {
        state: state_read(&entry.stmts, Reach::STEP).map(|(_, var)| var),
        ..Reach::STEP
    };
    // A step function, else a kick-off that runs its step function's code on a machine of its own.
    let kick = match is_step(f, step) {
        true => None,
        false => Some(kick::found(f)?),
    };
    let reach = kick.as_ref().map_or(step, |kick| kick.reach);
    let leave = Terminator::Return(kick.as_ref().and_then(|kick| kick.leave.clone()));
    // A kick-off's ways out all go to one `return`: each is made to return where it stands first.
    let mut whole = None;
    if kick.is_some() {
        whole = Some(f.cfg.clone());
        kick::returned(&mut f.cfg.blocks, &leave);
    }
    let plan = match proved(&f.cfg, reach, kick.as_ref(), &leave) {
        Ok(plan) => plan,
        Err(why) => {
            if let Some(whole) = whole {
                f.cfg = whole;
            }
            return declined(f, why);
        }
    };
    let whole = whole.unwrap_or_else(|| f.cfg.clone());
    let unread = Reach {
        state: None,
        ..reach
    };
    let read = state_read(&f.cfg.blocks[ENTRY].stmts, unread).map(|(at, _)| at);
    let done = match take_out(&mut f.cfg.blocks, &plan, read) {
        Some(machine) => sound(&f.cfg, &machine, reach).map(|()| machine),
        None => Err(Decline::Shared),
    };
    match done {
        Ok(mut machine) => {
            machine.kick = kick.map(kick::Kick::settle);
            Some(machine)
        }
        Err(why) => {
            f.cfg = whole;
            declined(f, why)
        }
    }
}

/// Puts back what `untangle` took out, each await as the source construct it is.
// ⚠ `structure` may write a short tail that leaves at each jump to it, and with it the call that
// stands for a piece: every such call takes the piece, and only the first its resume point.
pub fn run(f: &mut tree::Function, machine: Machine, cx: &Context<'_>, _stats: &mut Stats) {
    let Machine { pieces, kick } = machine;
    let mut placed = vec![false; pieces.len()];
    // The labels of copies count down from under those `take_out` gives resume points with no offset.
    let mut spare = u32::MAX.wrapping_sub(pieces.len() as u32);
    // An explicit stack: a body nests as deep as its listing does.
    let mut work: Vec<&mut Vec<tree::Stmt>> = vec![&mut f.body];
    while let Some(list) = work.pop() {
        for stmt in list {
            let index = match &*stmt {
                tree::Stmt::Simple(simple) => piece_of(simple),
                _ => None,
            };
            let Some(index) = index.filter(|index| *index < pieces.len()) else {
                work.extend(tree::nested_mut(stmt));
                continue;
            };
            // A piece holds no call that stands for another, so it is not gone into.
            *stmt = match std::mem::replace(&mut placed[index], true) {
                false => pieces[index].clone(),
                true => {
                    spare = spare.wrapping_sub(1);
                    again(&pieces[index], Label::Block(spare))
                }
            };
        }
    }
    if placed.contains(&false) {
        tracing::warn!(function = %f.head.name, "a piece of a state machine found no place");
    }
    if let Some(kick) = &kick {
        kick::settle(&mut f.body, kick);
    }
    // Each moves a statement into an await that is already there: `await` switches both off.
    if cx.options.on(Pass::Await) {
        place::tasks(&mut f.body);
        in_place(&mut f.body);
    }
}

/// How often each variable of a body is assigned and read, and which have their address taken.
#[derive(Default)]
struct Uses {
    writes: BTreeMap<Var, u32>,
    reads: BTreeMap<Var, u32>,
    addressed: BTreeSet<Var>,
}

impl Uses {
    fn of(body: &[tree::Stmt]) -> Uses {
        let mut uses = Uses::default();
        tree::each_simple(body, &mut |stmt| {
            for target in stmt.targets() {
                *uses.writes.entry(*target).or_default() += 1;
            }
        });
        tree::each_expr(body, &mut |top| {
            top.walk(&mut |e| match e {
                Expr::Var(var) => *uses.reads.entry(*var).or_default() += 1,
                Expr::AddrOf { var, .. } => {
                    uses.addressed.insert(*var);
                }
                _ => {}
            });
        });
        uses
    }

    /// Whether `var` is assigned once, read once and its address never taken.
    fn once(&self, var: Var) -> bool {
        let count = |held: &BTreeMap<Var, u32>| held.get(&var).copied().unwrap_or_default();
        let framed = matches!(var, Var::Local(_) | Var::Value { .. } | Var::Stack { .. });
        framed
            && count(&self.writes) == 1
            && count(&self.reads) == 1
            && !self.addressed.contains(&var)
    }
}

/// The variable an await takes its awaiter from, where the one statement before the await takes it as the first thing the call it makes evaluates.
fn taken_from(before: &[Stmt]) -> Option<Var> {
    let [Stmt::Assign { value, .. }] = before else {
        return None;
    };
    let Expr::Call { args, .. } = unwide(value) else {
        return None;
    };
    match unwide(args.first()?) {
        Expr::Var(var) => Some(*var),
        _ => None,
    }
}

/// Puts in the place of the variable an await takes its awaiter from the value assigned to it in the statement just before, where nothing else reads or assigns the variable or takes its address: `await f(a)`.
// ⚠ Nothing is worked out between the assignment and the taking, which evaluates the variable
// first: the value is worked out where it was.
fn in_place(body: &mut Vec<tree::Stmt>) {
    let uses = Uses::of(body);
    // An explicit stack: a body nests as deep as its listing does.
    let mut work: Vec<&mut Vec<tree::Stmt>> = vec![body];
    while let Some(list) = work.pop() {
        let mut at = 1;
        while at < list.len() {
            let folds = match (&list[at - 1], &list[at]) {
                (tree::Stmt::Simple(Stmt::Assign { target, .. }), tree::Stmt::Sugar(sugar)) => {
                    match &**sugar {
                        Sugar::Await { before, .. } => {
                            taken_from(before).is_some_and(|var| var == *target && uses.once(var))
                        }
                        _ => false,
                    }
                }
                _ => false,
            };
            if folds {
                let tree::Stmt::Simple(Stmt::Assign { value, .. }) = list.remove(at - 1) else {
                    continue;
                };
                if let tree::Stmt::Sugar(sugar) = &mut list[at - 1] {
                    if let Sugar::Await { before, .. } = &mut **sugar {
                        put_first(&mut before[0], value);
                    }
                }
                continue;
            }
            at += 1;
        }
        for stmt in list.iter_mut() {
            work.extend(tree::nested_mut(stmt));
        }
    }
}

/// Puts `value` in the place of the first argument of the call `taking` assigns, under the readings around it.
fn put_first(taking: &mut Stmt, value: Expr) {
    let Stmt::Assign { value: call, .. } = taking else {
        return;
    };
    let mut call = call;
    loop {
        match call {
            Expr::Sugar { e, .. }
            | Expr::Cast {
                to: Scalar::I64 | Scalar::U64,
                e,
            } => call = &mut **e,
            Expr::Call { args, .. } => {
                if let Some(first) = args.first_mut() {
                    let mut place = first;
                    while let Expr::Sugar { e, .. }
                    | Expr::Cast {
                        to: Scalar::I64 | Scalar::U64,
                        e,
                    } = place
                    {
                        place = &mut **e;
                    }
                    *place = value;
                }
                return;
            }
            _ => return,
        }
    }
}

/// A piece as it stands a second time: an await's resume point stays with the first, from which control runs on as it does from this one.
fn again(piece: &tree::Stmt, spare: Label) -> tree::Stmt {
    let mut copy = piece.clone();
    if let tree::Stmt::Sugar(sugar) = &mut copy {
        if let Sugar::Await {
            resumed, resume, ..
        } = &mut **sugar
        {
            *resumed = spare;
            resume.clear();
        }
    }
    copy
}

fn declined(f: &cfg::Function, why: Decline) -> Option<Machine> {
    tracing::debug!(
        function = %f.head.name,
        program = f.head.program,
        reason = %why,
        "a state machine is left as it is"
    );
    None
}

/// The statement that stands for piece `index`.
fn stand_in(index: usize) -> Stmt {
    Stmt::Eval(Expr::Call {
        callee: Callee::Intrinsic {
            name: PIECE.to_owned(),
            ty: None,
            wrapper: None,
        },
        args: vec![Expr::Const(Const {
            bits: index as u64,
            kind: Kind::Unknown,
        })],
        returns: Some(0),
    })
}

/// The piece a statement stands for.
fn piece_of(stmt: &Stmt) -> Option<usize> {
    match stmt {
        Stmt::Eval(Expr::Call {
            callee: Callee::Intrinsic { name, .. },
            args,
            ..
        }) if name == PIECE => match args.as_slice() {
            [Expr::Const(index)] => usize::try_from(index.bits).ok(),
            _ => None,
        },
        _ => None,
    }
}

/// Whether a variable is the frame's own: a local or a temporary.
fn framed(var: Var) -> bool {
    matches!(var, Var::Local(_) | Var::Value { .. } | Var::Stack { .. })
}

/// The constant `e` is, under any reading of it.
fn constant(e: &Expr) -> Option<&Const> {
    let mut e = e;
    while let Expr::Cast { e: inner, .. } = e {
        e = &**inner;
    }
    match e {
        Expr::Const(c) => Some(c),
        _ => None,
    }
}

/// `e` without the 64-bit readings and the source readings around it, which change no bit of a cell.
fn unwide(e: &Expr) -> &Expr {
    let mut e = e;
    while let Expr::Cast {
        to: Scalar::I64 | Scalar::U64,
        e: inner,
    }
    | Expr::Sugar { e: inner, .. } = e
    {
        e = &**inner;
    }
    e
}

/// What `e` is an address of, and how many bytes into it.
fn rooted(e: &Expr, held: &Held) -> Option<(Base, u32)> {
    let reach = held.reach;
    let (mut e, mut past) = (e, 0u32);
    loop {
        match unwide(e) {
            Expr::ByteOffset { addr, offset } => {
                let Expr::Const(by) = &**offset else {
                    return None;
                };
                past = past.checked_add(u32::try_from(by.bits).ok()?)?;
                e = &**addr;
            }
            Expr::Var(var) if *var == reach.pointer => return Some((Base::Machine, past)),
            Expr::Var(var) => {
                let (base, at) = *held.vars.get(var)?;
                return Some((base, at.checked_add(past)?));
            }
            // A kick-off's machine is a local: cell `n` of it is eight times `n` bytes in.
            Expr::AddrOf { var, cell } if Some(*var) == reach.storage => {
                let at = cell.checked_mul(8)?.checked_add(past)?;
                return Some((Base::Machine, at));
            }
            Expr::AddrOf { var, .. } if *var != reach.pointer => {
                return Some((Base::Frame, past));
            }
            _ => return None,
        }
    }
}

/// Where an access at `offset` through `addr` lands.
fn place(addr: &Expr, offset: u32, held: &Held) -> Option<(Base, u32)> {
    let (base, past) = rooted(addr, held)?;
    Some((base, past.checked_add(offset)?))
}

/// The field of the machine that `e` loads eight bytes of.
fn field_read(e: &Expr, held: &Held) -> Option<u32> {
    match unwide(e) {
        Expr::Load { ty, addr, offset } if ty.bytes() == 8 => match place(addr, *offset, held)? {
            (Base::Machine, at) => Some(at),
            _ => None,
        },
        _ => None,
    }
}

/// Whether evaluating `e` calls nothing and makes nothing.
fn quiet(e: &Expr) -> bool {
    let mut calm = true;
    e.walk(&mut |node| {
        calm &= !matches!(
            node,
            Expr::Call { .. } | Expr::NewObject(_) | Expr::NewArray { .. }
        );
    });
    calm
}

/// What a test is of, and whether the test holds where that is not zero.
fn tested(cond: &Expr) -> (&Expr, bool) {
    let (mut e, mut holds) = (cond, true);
    loop {
        match e {
            Expr::Truth(inner) | Expr::Cast { e: inner, .. } => e = &**inner,
            Expr::Unary {
                op: UnOp::Not,
                e: inner,
            } => {
                holds = !holds;
                e = &**inner;
            }
            Expr::Binary {
                op: op @ (BinOp::Eq | BinOp::Ne),
                l,
                r,
                ..
            } if constant(r).is_some_and(|c| c.bits == 0) => {
                holds ^= *op == BinOp::Eq;
                e = &**l;
            }
            _ => return (e, holds),
        }
    }
}

/// Whether `e` reads the state: from the machine, or from the variable it was read into.
fn reads_state(e: &Expr, reach: Reach) -> bool {
    let mut e = e;
    while let Expr::Cast {
        to: Scalar::I32 | Scalar::U32 | Scalar::I64 | Scalar::U64,
        e: inner,
    } = e
    {
        e = &**inner;
    }
    match e {
        Expr::Var(var) => Some(*var) == reach.state,
        Expr::Load {
            ty: Scalar::I32 | Scalar::U32,
            addr,
            offset: 0,
        } => matches!(&**addr, Expr::Var(var) if *var == reach.pointer),
        _ => false,
    }
}

/// The state a test names, and whether the test holds where the state is that one.
fn state_test(cond: &Expr, reach: Reach) -> Option<(i32, bool)> {
    let (mut e, mut holds) = (cond, true);
    loop {
        match e {
            Expr::Truth(inner) => e = &**inner,
            Expr::Unary {
                op: UnOp::Not,
                e: inner,
            } => {
                holds = !holds;
                e = &**inner;
            }
            Expr::Binary {
                op: op @ (BinOp::Eq | BinOp::Ne),
                l,
                r,
                ..
            } => {
                let named = constant(r)?;
                let is = holds == (*op == BinOp::Eq);
                return reads_state(l, reach).then_some((named.bits as i32, is));
            }
            // The state read as a truth value is the test of it against zero.
            other => return reads_state(other, reach).then_some((0, !holds)),
        }
    }
}

/// Whether the test a block ends in reads the state.
fn tests_state(term: &Terminator, reach: Reach) -> bool {
    let (Terminator::Branch { cond: e, .. } | Terminator::Switch { value: e, .. }) = term else {
        return false;
    };
    let mut reads = false;
    e.walk(&mut |node| reads |= reads_state(node, reach));
    reads
}

/// The statement of an entry that reads the state into a variable, and the variable.
// `reach` names no state yet: the read is the one that goes through the machine's address.
fn state_read(entry: &[Stmt], reach: Reach) -> Option<(usize, Var)> {
    entry.iter().enumerate().find_map(|(at, stmt)| match stmt {
        Stmt::Assign { target, value } if framed(*target) && reads_state(value, reach) => {
            Some((at, *target))
        }
        _ => None,
    })
}

/// Whether a statement stores the finished state.
fn finishes(stmt: &Stmt, reach: Reach) -> bool {
    matches!(
        stmt,
        Stmt::Store {
            kind: StoreKind::I32,
            addr: Expr::Var(var),
            offset: 0,
            value: Expr::Const(c),
        } if *var == reach.pointer && c.bits as i32 == FINISHED
    )
}

/// Whether a function has the form of a step: the state tested where the entry ends and the finished state stored somewhere; with the machine behind its parameter, that parameter alone and no result.
fn is_step(f: &cfg::Function, reach: Reach) -> bool {
    let head = &f.head;
    let blocks = &f.cfg.blocks;
    let shaped = reach.storage.is_some() || (head.params == 1 && head.returns == Returns::Void);
    shaped
        && head.incomplete.is_none()
        && blocks
            .first()
            .is_some_and(|entry| tests_state(&entry.term, reach))
        && blocks
            .iter()
            .flat_map(|block| &block.stmts)
            .any(|stmt| finishes(stmt, reach))
}

/// The blocks that jump to each block, each once, whether or not the entry reaches them.
fn predecessors(blocks: &[Block]) -> Vec<Vec<BlockId>> {
    let mut preds = vec![Vec::new(); blocks.len()];
    for (b, block) in blocks.iter().enumerate() {
        for to in block.term.successors() {
            preds[to].push(b);
        }
    }
    preds
}

/// Reads the tests of the state from the entry on: a chain of tests against one constant each, or a switch.
fn dispatch(blocks: &[Block], preds: &[Vec<BlockId>], reach: Reach) -> Result<Dispatch, Decline> {
    let mut found = Dispatch {
        links: Vec::new(),
        resumes: BTreeMap::new(),
        start: ENTRY,
    };
    let mut at = ENTRY;
    // A link is a block only the test before it leads to, so no round takes a block twice.
    for _ in 0..blocks.len() {
        let miss = match &blocks[at].term {
            Terminator::Branch { cond, then, els } => {
                let (named, holds) = state_test(cond, reach).ok_or(Decline::Dispatch)?;
                let (hit, miss) = match holds {
                    true => (*then, *els),
                    false => (*els, *then),
                };
                if found.resumes.insert(named, hit).is_some() {
                    return Err(Decline::States);
                }
                miss
            }
            Terminator::Switch {
                value,
                cases,
                default,
            } if reads_state(value, reach) => {
                for (named, to) in cases {
                    let named = i32::try_from(*named).map_err(|_| Decline::States)?;
                    if to != default && found.resumes.insert(named, *to).is_some() {
                        return Err(Decline::States);
                    }
                }
                *default
            }
            _ => return Err(Decline::Dispatch),
        };
        let link = miss != ENTRY
            && blocks[miss].stmts.is_empty()
            && preds[miss] == [at]
            && tests_state(&blocks[miss].term, reach);
        if !link {
            found.start = miss;
            return Ok(found);
        }
        found.links.push(miss);
        at = miss;
    }
    Err(Decline::Dispatch)
}

/// Whether `at` is a field and the one `slot` names, which it names where `slot` names none yet.
fn same_slot(slot: &mut Option<u32>, at: u32) -> bool {
    at >= FIELDS && *slot.get_or_insert(at) == at
}

/// The field a resume point takes its awaiter back from, where its block does that, sets the state to running and nothing more.
fn resumed(block: &Block, reach: Reach) -> Option<u32> {
    let mut held = Held::of(reach);
    let (mut slot, mut running) = (None, 0u32);
    for stmt in &block.stmts {
        let fits = match stmt {
            Stmt::Assign { target, value } if framed(*target) => {
                match (rooted(value, &held), value) {
                    (Some(root), _) => {
                        held.vars.insert(*target, root);
                        true
                    }
                    (None, Expr::Const(_)) => true,
                    (None, value) => {
                        field_read(value, &held).is_some_and(|at| same_slot(&mut slot, at))
                    }
                }
            }
            Stmt::Store {
                kind,
                addr,
                offset,
                value,
            } => match (place(addr, *offset, &held), constant(value)) {
                (Some((Base::Machine, 0)), Some(c))
                    if *kind == StoreKind::I32 && c.bits as i32 == RUNNING =>
                {
                    running += 1;
                    true
                }
                (Some((Base::Machine, at)), Some(c)) => {
                    kind.bytes() == 8 && c.bits == 0 && same_slot(&mut slot, at)
                }
                _ => false,
            },
            Stmt::ZeroBytes { addr, bytes: 8 } => place(addr, 0, &held)
                .is_some_and(|(base, at)| base == Base::Machine && same_slot(&mut slot, at)),
            _ => false,
        };
        if !fits {
            return None;
        }
    }
    slot.filter(|_| running == 1)
}

/// Follows one statement of a suspend path; `false` where it does anything but store into the machine or into an object made on the path, or compute without a call.
// `kept` is given on the path's first block alone: the state and the awaiter are stored there.
fn plumb(stmt: &Stmt, held: &mut Held, kept: Option<&mut Kept>) -> bool {
    match stmt {
        Stmt::Assign { target, value } if framed(*target) => {
            let made = match value {
                Expr::NewObject(_) => true,
                Expr::Call {
                    callee: Callee::Intrinsic { name, .. },
                    args,
                    ..
                } if name == BOX => {
                    matches!(args.as_slice(), [boxed] if rooted(boxed, held) == Some((Base::Machine, 0)))
                }
                _ => false,
            };
            let root = match made {
                true => Some((Base::Made, 0)),
                false => rooted(value, held),
            };
            match root {
                Some(root) => {
                    held.vars.insert(*target, root);
                    true
                }
                None => {
                    held.vars.remove(target);
                    quiet(value)
                }
            }
        }
        Stmt::Store {
            kind,
            addr,
            offset,
            value,
        } => {
            let carried = quiet(value) || matches!(value, Expr::NewObject(_));
            match (place(addr, *offset, held), kept) {
                (Some((Base::Made, _)), _) => carried,
                (Some((Base::Machine, 0)), Some(kept)) => {
                    let state = constant(value).map(|c| c.bits as i32);
                    let named = state.filter(|state| *kind == StoreKind::I32 && *state >= 0);
                    named.is_some_and(|state| kept.state.replace(state).is_none())
                }
                (Some((Base::Machine, at)), Some(kept)) if at >= FIELDS => {
                    kept.awaiter = match unwide(value) {
                        Expr::Var(var) if framed(*var) => Some(*var),
                        _ => None,
                    };
                    kind.bytes() == 8 && quiet(value) && kept.slot.replace(at).is_none()
                }
                (Some((Base::Machine, at)), _) => (BUILDER..FIELDS).contains(&at) && carried,
                _ => false,
            }
        }
        _ => false,
    }
}

/// Whether a statement hands control back in line: the runtime called with the builder's address, the awaiter and three flags.
fn hands_back(stmt: &Stmt, held: &Held) -> bool {
    match stmt {
        Stmt::Eval(Expr::Call {
            callee: Callee::Builtin { .. },
            args,
            ..
        }) => {
            args.len() == 5
                && rooted(&args[0], held) == Some((Base::Machine, BUILDER))
                && args.iter().all(quiet)
        }
        _ => false,
    }
}

/// Whether a statement hands control back through a function of the scripts, given the builder's address, the awaiter's and the machine's.
fn hands_back_through(stmt: &Stmt, held: &Held) -> bool {
    let Stmt::Eval(Expr::Call {
        callee: Callee::Local { .. } | Callee::Import { .. },
        args,
        ..
    }) = stmt
    else {
        return false;
    };
    let [builder, awaiter, machine] = args.as_slice() else {
        return false;
    };
    rooted(builder, held) == Some((Base::Machine, BUILDER))
        && matches!(rooted(awaiter, held), Some((Base::Frame, _)))
        && rooted(machine, held) == Some((Base::Machine, 0))
}

/// Reads the suspend path that starts in block `start`: one block that ends in a call of a script function, or the three of a first suspension made in line; either leaves as `leave` says.
fn suspended(
    blocks: &[Block],
    preds: &[Vec<BlockId>],
    start: BlockId,
    reach: Reach,
    leave: &Terminator,
) -> Result<Path, Decline> {
    let (mut held, mut kept) = (Held::of(reach), Kept::default());
    let block = &blocks[start];
    let fits = |holds: bool| holds.then_some(()).ok_or(Decline::SuspendBody);
    let first = match &block.term {
        term if term == leave => {
            let (tail, body) = block.stmts.split_last().ok_or(Decline::SuspendBody)?;
            fits(
                body.iter()
                    .all(|stmt| plumb(stmt, &mut held, Some(&mut kept))),
            )?;
            fits(hands_back_through(tail, &held))?;
            None
        }
        Terminator::Branch { cond, then, els } => {
            let body = &block.stmts;
            fits(
                body.iter()
                    .all(|stmt| plumb(stmt, &mut held, Some(&mut kept))),
            )?;
            // The arm that makes the task runs into the other, which hands control back.
            let (make, back) = match (&blocks[*then].term, &blocks[*els].term) {
                (Terminator::Jump(to), _) if to == els => (*then, *els),
                (_, Terminator::Jump(to)) if to == then => (*els, *then),
                _ => return Err(Decline::SuspendBody),
            };
            let (task, holds) = tested(cond);
            let once =
                quiet(cond) && field_read(task, &held) == Some(TASK) && holds == (*then == back);
            let own = make != back
                && make != start
                && back != start
                && preds[make] == [start]
                && preds[back].len() == 2;
            fits(once && own)?;
            let made = &blocks[make].stmts;
            fits(made.iter().all(|stmt| plumb(stmt, &mut held, None)))?;
            let tail = &blocks[back];
            fits(tail.term == *leave)?;
            fits(matches!(tail.stmts.as_slice(), [call] if hands_back(call, &held)))?;
            Some((make, back))
        }
        _ => return Err(Decline::SuspendBody),
    };
    match (kept.state, kept.slot) {
        (Some(state), Some(slot)) => Ok(Path {
            state,
            slot,
            first,
            awaiter: kept.awaiter,
        }),
        _ => Err(Decline::SuspendBody),
    }
}

/// The await of state `state`: its resume point, and the one test that leads to a suspend path of that state on one arm and on to where the resume point goes on the other.
fn await_of(
    blocks: &[Block],
    preds: &[Vec<BlockId>],
    state: i32,
    resume: BlockId,
    reach: Reach,
    leave: &Terminator,
) -> Result<Site, Decline> {
    let Terminator::Jump(on) = &blocks[resume].term else {
        return Err(Decline::Resume);
    };
    let on = *on;
    if preds[resume].len() != 1 || on == resume {
        return Err(Decline::Resume);
    }
    let slot = resumed(&blocks[resume], reach).ok_or(Decline::ResumeBody)?;
    let mut found = None;
    let mut why = Decline::Suspend;
    let tests = preds[on].iter().copied();
    for test in tests.filter(|test| *test != resume && *test != on) {
        let Terminator::Branch { then, els, .. } = &blocks[test].term else {
            continue;
        };
        let suspend = match (*then == on, *els == on) {
            (true, false) => *els,
            (false, true) => *then,
            _ => continue,
        };
        if preds[suspend] != [test] {
            continue;
        }
        match suspended(blocks, preds, suspend, reach, leave) {
            Ok(path) if path.state == state && path.slot == slot => {
                let site = Site {
                    test,
                    on,
                    suspend,
                    first: path.first,
                    resume,
                    kept: path.awaiter,
                    fetch: false,
                };
                if found.replace(site).is_some() {
                    return Err(Decline::Suspend);
                }
            }
            Ok(_) => {}
            Err(other) => why = other,
        }
    }
    found.ok_or(why)
}

/// The runtime method an await's test calls on its awaiter, by type and method, where the test holds exactly when the awaiter is ready.
fn ready_call(blocks: &[Block], site: &Site) -> Result<(u32, u32), Decline> {
    let Terminator::Branch { cond, then, .. } = &blocks[site.test].term else {
        return Err(Decline::Ready);
    };
    match tested(cond) {
        (
            Expr::Call {
                callee: Callee::Builtin { ty, method, .. },
                args,
                ..
            },
            holds,
        ) if args.len() == 1 && holds == (*then == site.on) => Ok((*ty, *method)),
        _ => Err(Decline::Ready),
    }
}

/// Whether a statement only makes a field of the machine zero.
fn clears(stmt: &Stmt, held: &Held) -> bool {
    let at = match stmt {
        Stmt::Store {
            addr,
            offset,
            value,
            ..
        } if constant(value).is_some_and(|c| c.bits == 0) => place(addr, *offset, held),
        Stmt::ZeroBytes { addr, .. } => place(addr, 0, held),
        _ => None,
    };
    matches!(at, Some((Base::Machine, at)) if at >= FIELDS)
}

/// The end of a machine with no result in block `x`: where the store of the finished state stands, and the blocks that complete the task that is there and store the runtime's finished one where none is.
// After the finished state only fields are cleared; the test is of the builder's task, which no
// code of the source can name.
fn ended(
    blocks: &[Block],
    preds: &[Vec<BlockId>],
    gone: &[bool],
    x: BlockId,
    reach: Reach,
    leave: &Terminator,
) -> Option<(usize, BlockId, BlockId)> {
    let none = Held::of(reach);
    let block = &blocks[x];
    let Terminator::Branch { cond, then, els } = &block.term else {
        return None;
    };
    let (task, holds) = tested(cond);
    let (set, make) = match holds {
        true => (*then, *els),
        false => (*els, *then),
    };
    let leaf = |b: BlockId| b != x && !gone[b] && preds[b] == [x] && blocks[b].term == *leave;
    let sets = matches!(
        blocks[set].stmts.as_slice(),
        [Stmt::Eval(Expr::Call { callee: Callee::Builtin { .. }, args, .. })]
            if matches!(args.as_slice(), [given] if field_read(given, &none) == Some(TASK))
    );
    let makes = matches!(
        blocks[make].stmts.as_slice(),
        [Stmt::Store { kind, addr, offset, value: Expr::Call { callee: Callee::Builtin { .. }, args, .. } }]
            if kind.bytes() == 8 && args.is_empty() && place(addr, *offset, &none) == Some((Base::Machine, TASK))
    );
    let from = block.stmts.iter().rposition(|stmt| finishes(stmt, reach))?;
    let cleared = block.stmts[from + 1..]
        .iter()
        .all(|stmt| clears(stmt, &none));
    let fits = field_read(task, &none) == Some(TASK)
        && set != make
        && leaf(set)
        && leaf(make)
        && sets
        && makes
        && cleared;
    fits.then_some((from, set, make))
}

/// Whether a statement only reads the machine into a variable of the frame.
fn entry_read(stmt: &Stmt, none: &Held) -> bool {
    let Stmt::Assign { target, value } = stmt else {
        return false;
    };
    let mut read = value;
    while let Expr::Cast { e, .. } = read {
        read = &**e;
    }
    let at = match read {
        Expr::Load { addr, offset, .. } | Expr::LoadBytes { addr, offset, .. } => {
            place(addr, *offset, none)
        }
        _ => None,
    };
    framed(*target) && matches!(at, Some((Base::Machine, _)))
}

/// Whether the step's own entry only reads the machine into variables of the frame.
// ⚠ The entry runs again at every resumed call, and once in the one body the awaits are read as.
fn entry_reads(entry: &[Stmt], reach: Reach) -> bool {
    let none = Held::of(reach);
    entry.iter().all(|stmt| entry_read(stmt, &none))
}

// Proves the whole machine of a step function, or of a kick-off that runs one in line, reading the blocks and changing none.
fn proved(
    cfg: &Cfg,
    reach: Reach,
    kick: Option<&kick::Kick>,
    leave: &Terminator,
) -> Result<Plan, Decline> {
    let blocks = cfg.blocks.as_slice();
    let preds = predecessors(blocks);
    // A function that already calls the piece intrinsic would have its own calls taken for pieces.
    let marked = blocks
        .iter()
        .flat_map(|block| &block.stmts)
        .any(|stmt| piece_of(stmt).is_some());
    if marked || !preds[ENTRY].is_empty() {
        return Err(Decline::Dispatch);
    }
    // A kick-off's entry starts with what makes the machine: the step's own entry comes after it.
    let hide = kick.map(|kick| kick.hide.clone()).unwrap_or_default();
    let cut = hide.len();
    let own = blocks[ENTRY].stmts.get(cut..).ok_or(Decline::Entry)?;
    if !entry_reads(own, reach) {
        return Err(Decline::Entry);
    }
    let Dispatch {
        links,
        resumes,
        start,
    } = dispatch(blocks, &preds, reach)?;
    let counted = !resumes.is_empty() && resumes.keys().zip(0i32..).all(|(named, k)| *named == k);
    let mut apart = BTreeSet::new();
    let own = resumes
        .values()
        .all(|resume| *resume != start && *resume != ENTRY && apart.insert(*resume));
    if !counted || !own {
        return Err(Decline::States);
    }

    let mut sites = Vec::with_capacity(resumes.len());
    let mut ready: Option<(u32, u32)> = None;
    for (named, resume) in &resumes {
        let mut site = await_of(blocks, &preds, *named, *resume, reach, leave)?;
        let callee = ready_call(blocks, &site)?;
        if ready.replace(callee).is_some_and(|other| other != callee) {
            return Err(Decline::Ready);
        }
        site.fetch = ends::fetched(blocks, &preds, &site);
        sites.push(site);
    }
    if !frame::holds(cfg, &sites, reach, cut) {
        return Err(Decline::Frame);
    }

    let mut gone = vec![false; blocks.len()];
    let taken = sites.iter().flat_map(Site::taken);
    for b in links.iter().copied().chain(taken) {
        if std::mem::replace(&mut gone[b], true) {
            return Err(Decline::Shared);
        }
    }
    // What stays must be none of what goes.
    if gone[start] || sites.iter().any(|site| gone[site.test] || gone[site.on]) {
        return Err(Decline::Shared);
    }
    let reached = cfg.reachable();
    let mut ends = Vec::new();
    for x in 1..blocks.len() {
        if gone[x] {
            continue;
        }
        if let Some((from, set, make)) = ended(blocks, &preds, &gone, x, reach, leave) {
            gone[set] = true;
            gone[make] = true;
            let finish = Finish::InLine;
            ends.push(End {
                block: x,
                from,
                finish,
            });
        } else if let Some((from, upto)) = ends::through(&blocks[x], reach, leave) {
            // ⚠ A piece must stand where the entry reaches it: an end no way leads to stays.
            if reached[x] {
                let finish = Finish::Through { upto };
                ends.push(End {
                    block: x,
                    from,
                    finish,
                });
            }
        }
    }
    // The one `return` a kick-off's ways out went to, where each now returns for itself.
    if kick.is_some() {
        for x in 1..blocks.len() {
            let block = &blocks[x];
            let spent = preds[x].is_empty() && block.stmts.is_empty() && block.term == *leave;
            if spent && !gone[x] {
                gone[x] = true;
            }
        }
    }
    let leave = match leave {
        Terminator::Return(value) => value.clone(),
        _ => None,
    };
    Ok(Plan {
        links,
        start,
        sites,
        ends,
        gone,
        hide,
        leave,
    })
}

/// The statements of a block, taken out of it.
fn simple(block: &mut Block) -> Vec<tree::Stmt> {
    std::mem::take(&mut block.stmts)
        .into_iter()
        .map(tree::Stmt::Simple)
        .collect()
}

/// Takes the test a block ends in, with its two arms, and leaves the block to end in `leave`.
fn test_out(block: &mut Block, leave: Terminator) -> Option<(Expr, BlockId, BlockId)> {
    match std::mem::replace(&mut block.term, leave) {
        Terminator::Branch { cond, then, els } => Some((cond, then, els)),
        _ => None,
    }
}

/// Leaves one call in `into` for the statements gathered in `run`, which go out of sight together.
fn hide_run(into: &mut Vec<Stmt>, run: &mut Vec<tree::Stmt>, pieces: &mut Vec<tree::Stmt>) {
    if run.is_empty() {
        return;
    }
    into.push(stand_in(pieces.len()));
    let hidden = Sugar::Hidden(std::mem::take(run));
    pieces.push(tree::Stmt::Sugar(Box::new(hidden)));
}

/// Leaves a call that stands for `piece` at the end of block `at`.
fn put(blocks: &mut [Block], at: BlockId, pieces: &mut Vec<tree::Stmt>, piece: Sugar) {
    blocks[at].stmts.push(stand_in(pieces.len()));
    pieces.push(tree::Stmt::Sugar(Box::new(piece)));
}

/// Takes the pieces of a proved machine out of the blocks, a call standing where each stood; `None` where a block is not what the proof took it for.
// ⚠ On `None` the blocks are half taken apart: the caller puts back the copy it kept.
fn take_out(blocks: &mut Vec<Block>, plan: &Plan, read: Option<usize>) -> Option<Machine> {
    let mut pieces: Vec<tree::Stmt> = Vec::new();
    let leave = || tree::Stmt::Return(plan.leave.clone());
    // A resume point is named by its own offset, else by a number no block is numbered by.
    let labels: BTreeMap<BlockId, Label> = plan
        .sites
        .iter()
        .zip(0u32..)
        .map(|(site, k)| {
            let at = blocks[site.resume].at;
            (
                site.resume,
                at.map_or(Label::Block(u32::MAX - k), Label::At),
            )
        })
        .collect();

    // What makes a kick-off's machine goes out of sight run by run, between the statements that
    // copy a parameter into it, which stay.
    let mut read = read;
    let cut = plan.hide.len();
    if cut > 0 {
        let entry = &mut blocks[ENTRY].stmts;
        if cut > entry.len() {
            return None;
        }
        let rest = entry.split_off(cut);
        let mut run: Vec<tree::Stmt> = Vec::new();
        for (stmt, hide) in std::mem::take(entry).into_iter().zip(&plan.hide) {
            if *hide {
                run.push(tree::Stmt::Simple(stmt));
                continue;
            }
            hide_run(entry, &mut run, &mut pieces);
            entry.push(stmt);
        }
        hide_run(entry, &mut run, &mut pieces);
        read = match read {
            Some(at) => Some(at.checked_sub(cut)?.checked_add(entry.len())?),
            None => None,
        };
        entry.extend(rest);
    }

    // The entry: the read of the state where it stood, and the tests of it where the entry ends.
    if let Some(read) = read {
        let stand = stand_in(pieces.len());
        let stmt = std::mem::replace(blocks[ENTRY].stmts.get_mut(read)?, stand);
        let hidden = Sugar::Hidden(vec![tree::Stmt::Simple(stmt)]);
        pieces.push(tree::Stmt::Sugar(Box::new(hidden)));
    }
    let mut tests = Vec::new();
    let mut at = ENTRY;
    for next in plan.links.iter().copied().chain([plan.start]) {
        // A test falls out to the next one, and the last to where a first call starts.
        let go = |to: BlockId| match to == next {
            true => Some(Vec::new()),
            false => labels.get(&to).map(|label| vec![tree::Stmt::Goto(*label)]),
        };
        let leave = Terminator::Jump(plan.start);
        tests.push(match std::mem::replace(&mut blocks[at].term, leave) {
            Terminator::Branch { cond, then, els } => tree::Stmt::If {
                cond,
                then: go(then)?,
                els: go(els)?,
            },
            Terminator::Switch {
                value,
                cases,
                default,
            } => {
                let mut arms = Vec::with_capacity(cases.len());
                for (named, to) in cases {
                    // A case with no statement would run into the next one.
                    let body = match to == default {
                        true => vec![tree::Stmt::Break],
                        false => go(to)?,
                    };
                    arms.push(tree::Case {
                        labels: vec![Some(named)],
                        body,
                    });
                }
                tree::Stmt::Switch { value, cases: arms }
            }
            _ => return None,
        });
        at = next;
    }
    put(blocks, ENTRY, &mut pieces, Sugar::Hidden(tests));

    // ⚠ The ends go first: each was found by where its statements stand in a block, and an await
    // that takes its result takes the first statement of the block it goes on in.
    for end in &plan.ends {
        match end.finish {
            Finish::InLine => {
                let back = Terminator::Return(plan.leave.clone());
                let (cond, then, els) = test_out(&mut blocks[end.block], back)?;
                if end.from > blocks[end.block].stmts.len() {
                    return None;
                }
                let tail = blocks[end.block].stmts.split_off(end.from);
                let mut hidden: Vec<tree::Stmt> =
                    tail.into_iter().map(tree::Stmt::Simple).collect();
                let mut arm = |b: BlockId| {
                    let mut body = simple(&mut blocks[b]);
                    body.push(leave());
                    body
                };
                let (then, els) = (arm(then), arm(els));
                hidden.push(tree::Stmt::If { cond, then, els });
                put(blocks, end.block, &mut pieces, Sugar::Hidden(hidden));
            }
            Finish::Through { upto } => {
                let stmts = &mut blocks[end.block].stmts;
                if end.from > upto || upto > stmts.len() {
                    return None;
                }
                let kept = stmts.split_off(upto);
                let hidden: Vec<tree::Stmt> = stmts
                    .split_off(end.from)
                    .into_iter()
                    .map(tree::Stmt::Simple)
                    .collect();
                stmts.push(stand_in(pieces.len()));
                pieces.push(tree::Stmt::Sugar(Box::new(Sugar::Hidden(hidden))));
                stmts.extend(kept);
            }
        }
    }

    for site in &plan.sites {
        let resumed = *labels.get(&site.resume)?;
        let (cond, then, _) = test_out(&mut blocks[site.test], Terminator::Jump(site.on))?;
        let ready = match then == site.on {
            true => cond,
            false => Expr::Unary {
                op: UnOp::Not,
                e: Box::new(cond),
            },
        };
        let mut suspend = simple(&mut blocks[site.suspend]);
        if let Some((make, back)) = site.first {
            let (task, then, _) = test_out(&mut blocks[site.suspend], Terminator::Unreachable)?;
            let made = simple(&mut blocks[make]);
            suspend.push(match then == make {
                true => tree::Stmt::If {
                    cond: task,
                    then: made,
                    els: Vec::new(),
                },
                false => tree::Stmt::If {
                    cond: task,
                    then: Vec::new(),
                    els: made,
                },
            });
            suspend.extend(simple(&mut blocks[back]));
        }
        suspend.push(leave());
        let resume = simple(&mut blocks[site.resume]);
        // The awaiter is the task itself, taken by a statement like any other: `before` holds
        // nothing. `after` holds the one statement that asks the awaiter for its result.
        let mut after = Vec::new();
        if site.fetch {
            let on = &mut blocks[site.on].stmts;
            if on.is_empty() {
                return None;
            }
            after.push(on.remove(0));
        }
        let piece = Sugar::Await {
            before: Vec::new(),
            ready,
            suspend,
            resumed,
            resume,
            after,
        };
        put(blocks, site.test, &mut pieces, piece);
    }

    // What is left, renumbered: nothing left jumps to a block that goes.
    let mut renumbered = vec![usize::MAX; blocks.len()];
    let mut kept = 0;
    for (b, gone) in plan.gone.iter().enumerate() {
        if !gone {
            renumbered[b] = kept;
            kept += 1;
        }
    }
    let all = std::mem::take(blocks);
    for (mut block, gone) in all.into_iter().zip(&plan.gone) {
        if !gone {
            block.term.retarget(|to| renumbered[to]);
            blocks.push(block);
        }
    }
    Some(Machine { pieces, kick: None })
}

/// How an expression left in a step function mentions the machine's address and the state.
#[derive(Default)]
struct Mentions {
    /// Mentions of the machine's address or of a copy of it.
    named: usize,
    /// Those that only reach what lies past the state.
    fair: usize,
    /// The variable the state was read into is mentioned.
    state: bool,
}

impl Mentions {
    fn of(&mut self, top: &Expr, copies: &BTreeSet<Var>, reach: Reach) {
        top.walk(&mut |e| match e {
            // The local a kick-off holds its machine in is named by its address alone.
            Expr::Var(var) | Expr::AddrOf { var, .. } => {
                self.named += usize::from(copies.contains(var) || Some(*var) == reach.storage);
                self.state |= Some(*var) == reach.state;
            }
            Expr::Load { addr, offset, .. } | Expr::LoadBytes { addr, offset, .. } => {
                self.fair += usize::from(*offset >= BUILDER && copy(addr, copies, reach));
            }
            Expr::ByteOffset { addr, offset } => {
                let past = constant(offset).is_some_and(|by| {
                    (u64::from(BUILDER)..=u64::from(u32::MAX)).contains(&by.bits)
                });
                self.fair += usize::from(past && copy(addr, copies, reach));
            }
            _ => {}
        });
    }

    /// Whether every mention is a fair one and none is of the state.
    fn clean(&self) -> bool {
        !self.state && self.named == self.fair
    }
}

/// Whether `e` is the machine's address or a copy of it.
fn copy(e: &Expr, copies: &BTreeSet<Var>, reach: Reach) -> bool {
    match e {
        Expr::Var(var) => copies.contains(var),
        Expr::AddrOf { var, cell: 0 } => Some(*var) == reach.storage,
        _ => false,
    }
}

/// The variables of the frame a statement gives a copy of the machine's address.
fn copied(stmt: &Stmt, copies: &BTreeSet<Var>, reach: Reach) -> Vec<Var> {
    let pairs: Vec<(&Var, &Expr)> = match stmt {
        Stmt::Assign { target, value } => vec![(target, value)],
        Stmt::AssignMany { targets, values } => targets.iter().zip(values).collect(),
        _ => Vec::new(),
    };
    pairs
        .into_iter()
        .filter(|(target, value)| framed(**target) && copy(unwide(value), copies, reach))
        .map(|(target, _)| *target)
        .collect()
}

/// Whether a statement left in a step function leaves the state alone.
fn left_alone(stmt: &Stmt, copies: &BTreeSet<Var>, reach: Reach) -> bool {
    let mut seen = Mentions::default();
    stmt.each_expr(&mut |top| seen.of(top, copies, reach));
    // The address a store goes through and a copy made of the address are fair mentions of their own.
    seen.fair += match stmt {
        Stmt::Store {
            kind,
            addr,
            offset,
            value,
        } if copy(addr, copies, reach) => {
            let finished = *offset == 0
                && *kind == StoreKind::I32
                && constant(value).is_some_and(|c| c.bits as i32 == FINISHED);
            usize::from(*offset >= BUILDER || finished)
        }
        Stmt::StoreBytes { addr, offset, .. }
        | Stmt::StoreF32Slots { addr, offset, .. }
        | Stmt::StepI32 { addr, offset, .. }
            if copy(addr, copies, reach) =>
        {
            usize::from(*offset >= BUILDER)
        }
        Stmt::Assign { .. } | Stmt::AssignMany { .. } => copied(stmt, copies, reach).len(),
        _ => 0,
    };
    let own = |target: &Var| *target == reach.pointer || Some(*target) == reach.storage;
    seen.clean() && !stmt.targets().iter().any(own)
}

/// The parts of a piece that stay in sight, where it is an await: the test of the awaiter, and the statement that takes its result.
fn awaited(piece: &tree::Stmt) -> Option<(&Expr, &[Stmt])> {
    match piece {
        tree::Stmt::Sugar(sugar) => match &**sugar {
            Sugar::Await { ready, after, .. } => Some((ready, after.as_slice())),
            _ => None,
        },
        _ => None,
    }
}

/// Checks what `take_out` left: every piece stands once where the entry reaches it, and nothing left reads the state or uses the machine's address but to reach what lies past the state.
fn sound(cfg: &Cfg, machine: &Machine, reach: Reach) -> Result<(), Decline> {
    let blocks = &cfg.blocks;
    let lost = |block: &Block| block.term.successors().iter().any(|to| *to >= blocks.len());
    if blocks.iter().any(lost) {
        return Err(Decline::Shared);
    }
    let reached = cfg.reachable();
    let mut stood = vec![0u32; machine.pieces.len()];
    for (block, _) in blocks.iter().zip(&reached).filter(|(_, reached)| **reached) {
        for index in block.stmts.iter().filter_map(piece_of) {
            *stood.get_mut(index).ok_or(Decline::Unreached)? += 1;
        }
    }
    if stood.iter().any(|times| *times != 1) {
        return Err(Decline::Unreached);
    }

    let awaits = || machine.pieces.iter().filter_map(awaited);
    let mut stmts: Vec<&Stmt> = blocks.iter().flat_map(|block| &block.stmts).collect();
    stmts.extend(awaits().flat_map(|(_, after)| after));
    let mut tests: Vec<&Expr> = blocks
        .iter()
        .filter_map(|block| block.term.expr())
        .collect();
    tests.extend(awaits().map(|(ready, _)| ready));
    // A copy a resume point makes is still held where control goes on, so the pieces are read too.
    let mut hidden: Vec<&Stmt> = Vec::new();
    tree::each_simple(&machine.pieces, &mut |stmt| hidden.push(stmt));
    let mut copies = BTreeSet::from([reach.pointer]);
    let mut settled = false;
    for _ in 0..SWEEPS {
        let known = copies.len();
        for stmt in stmts.iter().chain(&hidden) {
            let more = copied(stmt, &copies, reach);
            copies.extend(more);
        }
        settled = copies.len() == known;
        if settled {
            break;
        }
    }
    let apart = settled
        && stmts.iter().all(|stmt| left_alone(stmt, &copies, reach))
        && tests.iter().all(|test| {
            let mut seen = Mentions::default();
            seen.of(test, &copies, reach);
            seen.clean()
        });
    apart.then_some(()).ok_or(Decline::StateUse)
}

#[cfg(test)]
#[path = "async_await_tests.rs"]
mod tests;
