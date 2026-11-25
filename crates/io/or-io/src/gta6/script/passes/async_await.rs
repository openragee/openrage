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
//! ⚠ Between `untangle` and `run` the blocks hold those calls and no dispatch: they run as the
//! function did only once `run` has put the pieces back.
//! ⚠ `iterators` and `wait` switch nothing: an iterator's step is not proved by one function, and
//! a script's wait is an await like any other.

use std::collections::{BTreeMap, BTreeSet};

use crate::gta6::script::cfg::{self, Block, BlockId, Cfg, Terminator, ENTRY};
use crate::gta6::script::ir::{
    BinOp, Callee, Const, Expr, Kind, Returns, Scalar, Stmt, StoreKind, UnOp, Var,
};
use crate::gta6::script::tree::{self, Label, Sugar};
use crate::gta6::script::{Context, Pass, Stats};

use super::forward::flow::{self, Set};

/// The names this file's rewrites are switched by.
pub const PASSES: &[Pass] = &[Pass::Await, Pass::Iterators, Pass::Wait];

/// The parameter a step function takes: the address of its state machine.
const MACHINE: Var = Var::Param(0);
/// Where a state machine holds its builder, the task inside the builder, and its first field.
const BUILDER: u32 = 0x8;
const TASK: u32 = 0x10;
const FIELDS: u32 = 0x18;
/// The states no await owns: the machine is running, the machine is finished.
const RUNNING: i32 = -1;
const FINISHED: i32 = -2;
/// The intrinsic whose call stands in the blocks for a piece taken out.
const PIECE: &str = "state_machine_piece";
/// The intrinsic that copies a value into a new object, which a first suspension does to its machine.
const BOX: &str = "box_at";
/// The sweeps given to finding the copies of the machine's address; a chain of copies longer than this leaves the function as it is.
const SWEEPS: usize = 8;
/// The sweeps given to finding what each block may still read of the frame, and the words its sets may take; past either the machine is left as it is.
const LIVE_SWEEPS: usize = 64;
const LIVE_WORDS: usize = 1 << 20;

/// What `untangle` took out of a step function's blocks, for `run` to put back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Machine {
    /// Each construct taken out; the blocks hold a call of the piece intrinsic with its index in its place.
    pub pieces: Vec<tree::Stmt>,
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
type Held = BTreeMap<Var, (Base, u32)>;

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

/// The end of a machine that yields no result: the block that stores the finished state, and the statement that does.
struct End {
    block: BlockId,
    from: usize,
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
}

/// Takes a proved state machine's dispatch and resume paths out of the blocks; `None`, with the blocks untouched, where none is proved.
pub fn untangle(f: &mut cfg::Function, cx: &Context<'_>, _stats: &mut Stats) -> Option<Machine> {
    if !cx.options.on(Pass::Await) {
        return None;
    }
    let read = state_read(f.cfg.blocks.first()?);
    let state = read.map(|(_, var)| var);
    if !is_step(f, state) {
        return None;
    }
    let plan = match proved(&f.cfg, state) {
        Ok(plan) => plan,
        Err(why) => return declined(f, why),
    };
    let whole = f.cfg.clone();
    let done = match take_out(&mut f.cfg.blocks, &plan, read.map(|(at, _)| at)) {
        Some(machine) => sound(&f.cfg, &machine, state).map(|()| machine),
        None => Err(Decline::Shared),
    };
    match done {
        Ok(machine) => Some(machine),
        Err(why) => {
            f.cfg = whole;
            declined(f, why)
        }
    }
}

/// Puts back what `untangle` took out, each await as the source construct it is.
// ⚠ `structure` may write a short tail that leaves at each jump to it, and with it the call that
// stands for a piece: every such call takes the piece, and only the first its resume point.
pub fn run(f: &mut tree::Function, machine: Machine, _cx: &Context<'_>, _stats: &mut Stats) {
    let pieces = machine.pieces;
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

/// `e` without the 64-bit readings around it, which change no bit of a cell.
fn unwide(e: &Expr) -> &Expr {
    let mut e = e;
    while let Expr::Cast {
        to: Scalar::I64 | Scalar::U64,
        e: inner,
    } = e
    {
        e = &**inner;
    }
    e
}

/// What `e` is an address of, and how many bytes into it.
fn rooted(e: &Expr, held: &Held) -> Option<(Base, u32)> {
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
            Expr::Var(MACHINE) => return Some((Base::Machine, past)),
            Expr::Var(var) => {
                let (base, at) = *held.get(var)?;
                return Some((base, at.checked_add(past)?));
            }
            Expr::AddrOf { var, .. } if *var != MACHINE => return Some((Base::Frame, past)),
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
fn reads_state(e: &Expr, state: Option<Var>) -> bool {
    let mut e = e;
    while let Expr::Cast {
        to: Scalar::I32 | Scalar::U32 | Scalar::I64 | Scalar::U64,
        e: inner,
    } = e
    {
        e = &**inner;
    }
    match e {
        Expr::Var(var) => Some(*var) == state,
        Expr::Load {
            ty: Scalar::I32 | Scalar::U32,
            addr,
            offset: 0,
        } => matches!(&**addr, Expr::Var(MACHINE)),
        _ => false,
    }
}

/// The state a test names, and whether the test holds where the state is that one.
fn state_test(cond: &Expr, state: Option<Var>) -> Option<(i32, bool)> {
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
                return reads_state(l, state).then_some((named.bits as i32, is));
            }
            // The state read as a truth value is the test of it against zero.
            other => return reads_state(other, state).then_some((0, !holds)),
        }
    }
}

/// Whether the test a block ends in reads the state.
fn tests_state(term: &Terminator, state: Option<Var>) -> bool {
    let (Terminator::Branch { cond: e, .. } | Terminator::Switch { value: e, .. }) = term else {
        return false;
    };
    let mut reads = false;
    e.walk(&mut |node| reads |= reads_state(node, state));
    reads
}

/// The entry's statement that reads the state into a variable, and the variable.
fn state_read(entry: &Block) -> Option<(usize, Var)> {
    entry
        .stmts
        .iter()
        .enumerate()
        .find_map(|(at, stmt)| match stmt {
            Stmt::Assign { target, value } if framed(*target) && reads_state(value, None) => {
                Some((at, *target))
            }
            _ => None,
        })
}

/// Whether a statement stores the finished state.
fn finishes(stmt: &Stmt) -> bool {
    matches!(
        stmt,
        Stmt::Store {
            kind: StoreKind::I32,
            addr: Expr::Var(MACHINE),
            offset: 0,
            value: Expr::Const(c),
        } if c.bits as i32 == FINISHED
    )
}

/// Whether a function has the form of a step: one parameter, no result, the state tested where the entry ends and the finished state stored somewhere.
fn is_step(f: &cfg::Function, state: Option<Var>) -> bool {
    let head = &f.head;
    let blocks = &f.cfg.blocks;
    head.params == 1
        && head.returns == Returns::Void
        && head.incomplete.is_none()
        && blocks
            .first()
            .is_some_and(|entry| tests_state(&entry.term, state))
        && blocks.iter().flat_map(|block| &block.stmts).any(finishes)
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
fn dispatch(
    blocks: &[Block],
    preds: &[Vec<BlockId>],
    state: Option<Var>,
) -> Result<Dispatch, Decline> {
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
                let (named, holds) = state_test(cond, state).ok_or(Decline::Dispatch)?;
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
            } if reads_state(value, state) => {
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
            && tests_state(&blocks[miss].term, state);
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
fn resumed(block: &Block) -> Option<u32> {
    let mut held = Held::new();
    let (mut slot, mut running) = (None, 0u32);
    for stmt in &block.stmts {
        let fits = match stmt {
            Stmt::Assign { target, value } if framed(*target) => {
                match (rooted(value, &held), value) {
                    (Some(root), _) => {
                        held.insert(*target, root);
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
                    held.insert(*target, root);
                    true
                }
                None => {
                    held.remove(target);
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

/// Reads the suspend path that starts in block `start`: one block that ends in a call of a script function, or the three of a first suspension made in line.
fn suspended(blocks: &[Block], preds: &[Vec<BlockId>], start: BlockId) -> Result<Path, Decline> {
    let (mut held, mut kept) = (Held::new(), Kept::default());
    let block = &blocks[start];
    let fits = |holds: bool| holds.then_some(()).ok_or(Decline::SuspendBody);
    let first = match &block.term {
        Terminator::Return(None) => {
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
            fits(matches!(tail.term, Terminator::Return(None)))?;
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
) -> Result<Site, Decline> {
    let Terminator::Jump(on) = &blocks[resume].term else {
        return Err(Decline::Resume);
    };
    let on = *on;
    if preds[resume].len() != 1 || on == resume {
        return Err(Decline::Resume);
    }
    let slot = resumed(&blocks[resume]).ok_or(Decline::ResumeBody)?;
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
        match suspended(blocks, preds, suspend) {
            Ok(path) if path.state == state && path.slot == slot => {
                let site = Site {
                    test,
                    on,
                    suspend,
                    first: path.first,
                    resume,
                    kept: path.awaiter,
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
) -> Option<(usize, BlockId, BlockId)> {
    let none = Held::new();
    let block = &blocks[x];
    let Terminator::Branch { cond, then, els } = &block.term else {
        return None;
    };
    let (task, holds) = tested(cond);
    let (set, make) = match holds {
        true => (*then, *els),
        false => (*els, *then),
    };
    let leaf = |b: BlockId| {
        b != x && !gone[b] && preds[b] == [x] && matches!(blocks[b].term, Terminator::Return(None))
    };
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
    let from = block.stmts.iter().rposition(finishes)?;
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

/// Whether the entry only reads the machine into variables of the frame.
// ⚠ The entry runs again at every resumed call, and once in the one body the awaits are read as.
fn entry_reads(entry: &Block) -> bool {
    let none = Held::new();
    entry.stmts.iter().all(|stmt| {
        let Stmt::Assign { target, value } = stmt else {
            return false;
        };
        let mut read = value;
        while let Expr::Cast { e, .. } = read {
            read = &**e;
        }
        let at = match read {
            Expr::Load { addr, offset, .. } | Expr::LoadBytes { addr, offset, .. } => {
                place(addr, *offset, &none)
            }
            _ => None,
        };
        framed(*target) && matches!(at, Some((Base::Machine, _)))
    })
}

/// The variables of the frame a block mentions, each with a number.
fn frame_of(blocks: &[Block]) -> BTreeMap<Var, usize> {
    let mut frame = BTreeMap::new();
    let mut note = |var: Var| {
        if framed(var) {
            let next = frame.len();
            frame.entry(var).or_insert(next);
        }
    };
    for block in blocks {
        for stmt in &block.stmts {
            stmt.targets().iter().for_each(|var| note(*var));
            stmt.each_expr(&mut |top| {
                top.walk(&mut |e| {
                    if let Expr::Var(var) | Expr::AddrOf { var, .. } = e {
                        note(*var);
                    }
                });
            });
        }
        if let Some(test) = block.term.expr() {
            test.walk(&mut |e| {
                if let Expr::Var(var) | Expr::AddrOf { var, .. } = e {
                    note(*var);
                }
            });
        }
    }
    frame
}

/// The variable of the frame a statement writes through that variable's own address, and the expression that is the address.
fn own_store(stmt: &Stmt) -> Option<(Var, &Expr)> {
    let (Stmt::Store { addr, .. }
    | Stmt::StoreBytes { addr, .. }
    | Stmt::StoreF32Slots { addr, .. }) = stmt
    else {
        return None;
    };
    let mut root = addr;
    while let Expr::ByteOffset {
        addr: under,
        offset,
    } = root
    {
        constant(offset)?;
        root = &**under;
    }
    match root {
        Expr::AddrOf { var, .. } if framed(*var) => Some((*var, root)),
        _ => None,
    }
}

/// Whether the one body the awaits are read as holds, after each await, what a resumed call holds there.
// ⚠ A resumed call starts on a new frame: the entry and the resume point run and every other
// variable is zero, where the one body still holds what was written before the await. So no
// variable written on a way to an await may be read after it, but the awaiter the suspend path
// kept and one the entry alone writes. A write is an assignment, a store through the
// variable's own address, or that address handed on, which whatever takes it may write
// through; a clear through the address sets the variable back to what a new frame holds.
fn frame_holds(cfg: &Cfg, sites: &[Site]) -> bool {
    let blocks = &cfg.blocks;
    let frame = frame_of(blocks);
    let n = frame.len();
    if n == 0 {
        return true;
    }
    if blocks.len().saturating_mul(n.div_ceil(64)) > LIVE_WORDS {
        return false;
    }
    let mut written = vec![0u32; n];
    let none = || vec![Set::new(n); blocks.len()];
    // What each block reads of what it is entered with and what it sets whole; what it leaves written and what it leaves cleared.
    let (mut reads, mut sets, mut made, mut ended) = (none(), none(), none(), none());
    for (b, block) in blocks.iter().enumerate() {
        let (read, set) = (&mut reads[b], &mut sets[b]);
        let (made, ended) = (&mut made[b], &mut ended[b]);
        let mut write = |at: usize, made: &mut Set, ended: &mut Set| {
            written[at] = written[at].saturating_add(1);
            made.insert(at);
            ended.remove(at);
        };
        for stmt in &block.stmts {
            // A clear of a local runs on into the locals after it.
            if let Stmt::ZeroBytes {
                addr: Expr::AddrOf { var, cell: 0 },
                bytes,
            } = stmt
            {
                let last = match var {
                    Var::Local(first) => {
                        Var::Local(first.saturating_add(bytes.div_ceil(8).saturating_sub(1)))
                    }
                    other => *other,
                };
                for (_, at) in frame.range(*var..=last) {
                    set.insert(*at);
                    made.remove(*at);
                    ended.insert(*at);
                }
                continue;
            }
            let stored = own_store(stmt);
            let mut named: Vec<(Var, bool)> = Vec::new();
            stmt.each_expr(&mut |top| {
                top.walk(&mut |e| match e {
                    Expr::Var(var) => named.push((*var, false)),
                    // The address a store goes through reads nothing of what it stores into.
                    Expr::AddrOf { .. }
                        if stored.is_some_and(|(_, root)| std::ptr::eq(root, e)) => {}
                    Expr::AddrOf { var, .. } => named.push((*var, true)),
                    _ => {}
                });
            });
            for (var, by_address) in named {
                if let Some(at) = frame.get(&var) {
                    if !set.has(*at) {
                        read.insert(*at);
                    }
                    if by_address {
                        write(*at, made, ended);
                    }
                }
            }
            if let Some(at) = stored.and_then(|(var, _)| frame.get(&var)) {
                write(*at, made, ended);
            }
            for var in stmt.targets() {
                if let Some(at) = frame.get(var) {
                    set.insert(*at);
                    write(*at, made, ended);
                }
            }
        }
        if let Some(test) = block.term.expr() {
            test.walk(&mut |e| {
                let (var, by_address) = match e {
                    Expr::Var(var) => (var, false),
                    Expr::AddrOf { var, .. } => (var, true),
                    _ => return,
                };
                if let Some(at) = frame.get(var) {
                    if !set.has(*at) {
                        read.insert(*at);
                    }
                    if by_address {
                        write(*at, made, ended);
                    }
                }
            });
        }
    }
    // What a block may read of what it is entered with: its own reads, and those of what follows that it does not set first.
    let mut live = vec![Set::new(n); blocks.len()];
    let mut settled = false;
    for _ in 0..LIVE_SWEEPS {
        let mut changed = false;
        for b in (0..blocks.len()).rev() {
            let mut entered = Set::new(n);
            for to in blocks[b].term.successors() {
                if let Some(next) = live.get(to) {
                    entered.add(next);
                }
            }
            entered.without(&sets[b]);
            entered.add(&reads[b]);
            if entered != live[b] {
                live[b] = entered;
                changed = true;
            }
        }
        if !changed {
            settled = true;
            break;
        }
    }
    // What may have been written on some way to each block, and not cleared since.
    let Some(before) = flow::entering(cfg, n, (&made[..], &ended[..]), &Set::new(n), false) else {
        return false;
    };
    settled
        && sites.iter().all(|site| {
            let mut held = before[site.test].clone();
            held.without(&ended[site.test]);
            held.add(&made[site.test]);
            frame.iter().all(|(var, at)| {
                let entered = sets[ENTRY].has(*at) && written[*at] == 1;
                !live[site.on].has(*at)
                    || match sets[site.resume].has(*at) {
                        // ⚠ What the resume point sets is hidden in the one body: only the awaiter is the same there.
                        true => site.kept == Some(*var),
                        false => entered || !held.has(*at),
                    }
            })
        })
}

// Proves the whole machine of a step function, reading the blocks and changing none.
fn proved(cfg: &Cfg, state: Option<Var>) -> Result<Plan, Decline> {
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
    if !entry_reads(&blocks[ENTRY]) {
        return Err(Decline::Entry);
    }
    let Dispatch {
        links,
        resumes,
        start,
    } = dispatch(blocks, &preds, state)?;
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
        let site = await_of(blocks, &preds, *named, *resume)?;
        let callee = ready_call(blocks, &site)?;
        if ready.replace(callee).is_some_and(|other| other != callee) {
            return Err(Decline::Ready);
        }
        sites.push(site);
    }
    if !frame_holds(cfg, &sites) {
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
    let mut ends = Vec::new();
    for x in 1..blocks.len() {
        if gone[x] {
            continue;
        }
        if let Some((from, set, make)) = ended(blocks, &preds, &gone, x) {
            gone[set] = true;
            gone[make] = true;
            ends.push(End { block: x, from });
        }
    }
    Ok(Plan {
        links,
        start,
        sites,
        ends,
        gone,
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

/// Leaves a call that stands for `piece` at the end of block `at`.
fn put(blocks: &mut [Block], at: BlockId, pieces: &mut Vec<tree::Stmt>, piece: Sugar) {
    blocks[at].stmts.push(stand_in(pieces.len()));
    pieces.push(tree::Stmt::Sugar(Box::new(piece)));
}

/// Takes the pieces of a proved machine out of the blocks, a call standing where each stood; `None` where a block is not what the proof took it for.
// ⚠ On `None` the blocks are half taken apart: the caller puts back the copy it kept.
fn take_out(blocks: &mut Vec<Block>, plan: &Plan, read: Option<usize>) -> Option<Machine> {
    let mut pieces: Vec<tree::Stmt> = Vec::new();
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
        suspend.push(tree::Stmt::Return(None));
        let resume = simple(&mut blocks[site.resume]);
        // The awaiter is the task itself, taken by a statement like any other, and no call is
        // proved to be the one that asks it for its result: `before` and `after` hold nothing.
        let piece = Sugar::Await {
            before: Vec::new(),
            ready,
            suspend,
            resumed,
            resume,
            after: Vec::new(),
        };
        put(blocks, site.test, &mut pieces, piece);
    }

    for end in &plan.ends {
        let (cond, then, els) = test_out(&mut blocks[end.block], Terminator::Return(None))?;
        if end.from > blocks[end.block].stmts.len() {
            return None;
        }
        let tail = blocks[end.block].stmts.split_off(end.from);
        let mut hidden: Vec<tree::Stmt> = tail.into_iter().map(tree::Stmt::Simple).collect();
        let mut arm = |b: BlockId| {
            let mut body = simple(&mut blocks[b]);
            body.push(tree::Stmt::Return(None));
            body
        };
        let (then, els) = (arm(then), arm(els));
        hidden.push(tree::Stmt::If { cond, then, els });
        put(blocks, end.block, &mut pieces, Sugar::Hidden(hidden));
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
    Some(Machine { pieces })
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
    fn of(&mut self, top: &Expr, copies: &BTreeSet<Var>, state: Option<Var>) {
        top.walk(&mut |e| match e {
            Expr::Var(var) | Expr::AddrOf { var, .. } => {
                self.named += usize::from(copies.contains(var));
                self.state |= Some(*var) == state;
            }
            Expr::Load { addr, offset, .. } | Expr::LoadBytes { addr, offset, .. } => {
                self.fair += usize::from(*offset >= BUILDER && copy(addr, copies));
            }
            Expr::ByteOffset { addr, offset } => {
                let past = constant(offset).is_some_and(|by| {
                    (u64::from(BUILDER)..=u64::from(u32::MAX)).contains(&by.bits)
                });
                self.fair += usize::from(past && copy(addr, copies));
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
fn copy(e: &Expr, copies: &BTreeSet<Var>) -> bool {
    matches!(e, Expr::Var(var) if copies.contains(var))
}

/// The variables of the frame a statement gives a copy of the machine's address.
fn copied(stmt: &Stmt, copies: &BTreeSet<Var>) -> Vec<Var> {
    let pairs: Vec<(&Var, &Expr)> = match stmt {
        Stmt::Assign { target, value } => vec![(target, value)],
        Stmt::AssignMany { targets, values } => targets.iter().zip(values).collect(),
        _ => Vec::new(),
    };
    pairs
        .into_iter()
        .filter(|(target, value)| framed(**target) && copy(unwide(value), copies))
        .map(|(target, _)| *target)
        .collect()
}

/// Whether a statement left in a step function leaves the state alone.
fn left_alone(stmt: &Stmt, copies: &BTreeSet<Var>, state: Option<Var>) -> bool {
    let mut seen = Mentions::default();
    stmt.each_expr(&mut |top| seen.of(top, copies, state));
    // The address a store goes through and a copy made of the address are fair mentions of their own.
    seen.fair += match stmt {
        Stmt::Store {
            kind,
            addr,
            offset,
            value,
        } if copy(addr, copies) => {
            let finished = *offset == 0
                && *kind == StoreKind::I32
                && constant(value).is_some_and(|c| c.bits as i32 == FINISHED);
            usize::from(*offset >= BUILDER || finished)
        }
        Stmt::StoreBytes { addr, offset, .. }
        | Stmt::StoreF32Slots { addr, offset, .. }
        | Stmt::StepI32 { addr, offset, .. }
            if copy(addr, copies) =>
        {
            usize::from(*offset >= BUILDER)
        }
        Stmt::Assign { .. } | Stmt::AssignMany { .. } => copied(stmt, copies).len(),
        _ => 0,
    };
    seen.clean() && !stmt.targets().contains(&MACHINE)
}

/// The test of the awaiter, where a piece is an await: the one part of a piece that stays in sight.
fn awaited(piece: &tree::Stmt) -> Option<&Expr> {
    match piece {
        tree::Stmt::Sugar(sugar) => match &**sugar {
            Sugar::Await { ready, .. } => Some(ready),
            _ => None,
        },
        _ => None,
    }
}

/// Checks what `take_out` left: every piece stands once where the entry reaches it, and nothing left reads the state or uses the machine's address but to reach what lies past the state.
fn sound(cfg: &Cfg, machine: &Machine, state: Option<Var>) -> Result<(), Decline> {
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

    let stmts: Vec<&Stmt> = blocks.iter().flat_map(|block| &block.stmts).collect();
    let mut tests: Vec<&Expr> = blocks
        .iter()
        .filter_map(|block| block.term.expr())
        .collect();
    tests.extend(machine.pieces.iter().filter_map(awaited));
    // A copy a resume point makes is still held where control goes on, so the pieces are read too.
    let mut hidden: Vec<&Stmt> = Vec::new();
    tree::each_simple(&machine.pieces, &mut |stmt| hidden.push(stmt));
    let mut copies = BTreeSet::from([MACHINE]);
    let mut settled = false;
    for _ in 0..SWEEPS {
        let known = copies.len();
        for stmt in stmts.iter().chain(&hidden) {
            let more = copied(stmt, &copies);
            copies.extend(more);
        }
        settled = copies.len() == known;
        if settled {
            break;
        }
    }
    let apart = settled
        && stmts.iter().all(|stmt| left_alone(stmt, &copies, state))
        && tests.iter().all(|test| {
            let mut seen = Mentions::default();
            seen.of(test, &copies, state);
            seen.clean()
        });
    apart.then_some(()).ok_or(Decline::StateUse)
}

#[cfg(test)]
#[path = "async_await_tests.rs"]
mod tests;
