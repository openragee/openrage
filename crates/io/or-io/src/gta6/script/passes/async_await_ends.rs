//! Where a state machine is done: the end of the method, which completes the builder's task, and
//! the end of an await, which asks the awaiter for its result.
//!
//! Both are known by where they stand. After the finished state is stored the machine does one
//! thing more, complete its task; after an await only the compiler's own code can name the
//! awaiter, since every variable of the source that lives across an await is a field of the
//! machine.

use crate::gta6::script::cfg::{Block, BlockId, Terminator};
use crate::gta6::script::ir::{Callee, Expr, Scalar, Stmt, StoreKind, Var};
use crate::gta6::script::tree::{self, Sugar};

use super::{clears, finishes, rooted, tested, unwide, Base, Held, Reach, Site, BUILDER, FINISHED};

/// What an `async` method hands its caller, as the end of its state machine shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsyncResult<'a> {
    /// No end of the machine is proved: nothing is known of what the method hands back.
    Unknown,
    /// A task with no result: the machine completes the builder's task and gives it nothing.
    Task,
    /// A task with a result: the cells the machine gives the builder with its task.
    TaskOf(&'a [Expr]),
}

/// The end of a machine that completes its task through a function, in `block`: where the store of the finished state stands, and where what the compiler alone wrote stops.
// The function is given the builder's address first. One given nothing more is the compiler's
// too; one given a result is the method's `return` of that result, and stays in sight.
pub(super) fn through(block: &Block, reach: Reach, leave: &Terminator) -> Option<(usize, usize)> {
    if block.term != *leave {
        return None;
    }
    let none = Held::of(reach);
    let from = block.stmts.iter().rposition(|stmt| finishes(stmt, reach))?;
    let (call, between) = block.stmts.get(from + 1..)?.split_last()?;
    let Stmt::Eval(Expr::Call {
        callee: Callee::Local { .. } | Callee::Import { .. },
        args,
        ..
    }) = call
    else {
        return None;
    };
    let (builder, given) = args.split_first()?;
    let fits = rooted(builder, &none) == Some((Base::Machine, BUILDER))
        && between.iter().all(|stmt| clears(stmt, &none));
    let last = block.stmts.len() - 1;
    fits.then_some((from, last + usize::from(given.is_empty())))
}

/// Whether a statement asks `awaiter` for its result: a call given the awaiter alone, whose value is the statement's whole value.
fn asks(stmt: &Stmt, awaiter: Var) -> bool {
    let value = match stmt {
        Stmt::Assign { value, .. }
        | Stmt::Eval(value)
        | Stmt::Store { value, .. }
        | Stmt::StoreBytes { value, .. } => value,
        _ => return false,
    };
    let given = match unwide(value) {
        Expr::Call {
            callee: Callee::Intrinsic { .. },
            ..
        } => return false,
        Expr::Call { args, .. } => args.as_slice(),
        _ => return false,
    };
    matches!(given, [asked] if *asked == Expr::Var(awaiter))
}

/// Whether the block an await goes on in starts by asking the awaiter for its result.
// ⚠ The statement moves to the await itself, so every way into the block must be the await's:
// the test of a ready awaiter and the resume point, and no other.
pub(super) fn fetched(blocks: &[Block], preds: &[Vec<BlockId>], site: &Site) -> bool {
    let Some(awaiter) = site.kept else {
        return false;
    };
    let Terminator::Branch { cond, .. } = &blocks[site.test].term else {
        return false;
    };
    let (Expr::Call { args, .. }, _) = tested(cond) else {
        return false;
    };
    let tests_it = matches!(args.as_slice(), [asked] if *unwide(asked) == Expr::Var(awaiter));
    let ways = &preds[site.on];
    let own = site.on != site.test
        && ways.len() == 2
        && ways.contains(&site.test)
        && ways.contains(&site.resume);
    let first = blocks[site.on].stmts.first();
    tests_it && own && first.is_some_and(|stmt| asks(stmt, awaiter))
}

/// Whether a statement stores the finished state, through whatever address.
fn finished(stmt: &Stmt) -> bool {
    matches!(
        stmt,
        Stmt::Store {
            kind: StoreKind::I32,
            offset: 0,
            value: Expr::Const(c),
            ..
        } if c.bits as i32 == FINISHED
    )
}

/// Whether a hidden run is the end of a machine, and whether it completes the task itself.
fn end_of(hidden: &[tree::Stmt]) -> Option<bool> {
    let mut ends = false;
    tree::each_simple(hidden, &mut |stmt| ends |= finished(stmt));
    if !ends {
        return None;
    }
    let mut completes = false;
    tree::each_expr(hidden, &mut |top| {
        top.walk(&mut |e| completes |= matches!(e, Expr::Call { .. }));
    });
    Some(completes)
}

/// The cells a statement hands a function after the first thing it gives it.
fn handed(stmt: Option<&tree::Stmt>) -> Option<&[Expr]> {
    let tree::Stmt::Simple(Stmt::Eval(called)) = stmt? else {
        return None;
    };
    let mut called = called;
    while let Expr::Sugar { e, .. }
    | Expr::Cast {
        to: Scalar::I64 | Scalar::U64,
        e,
    } = called
    {
        called = &**e;
    }
    match called {
        Expr::Call { args, .. } if args.len() > 1 => args.get(1..),
        _ => None,
    }
}

/// What the `async` method with this body hands its caller.
// Read from the pieces `untangle` took out: a hidden end that completes the task itself is a
// task with no result, and one that stops before the call that completes it left that call in
// sight because it carries the result. A body that holds no await was never proved a machine.
pub fn result(body: &[tree::Stmt]) -> AsyncResult<'_> {
    let mut waits = false;
    tree::each_sugar(body, &mut |sugar| {
        waits |= matches!(sugar, Sugar::Await { .. })
    });
    if !waits {
        return AsyncResult::Unknown;
    }
    let mut found = AsyncResult::Unknown;
    // An explicit stack: a body nests as deep as its listing does.
    let mut open: Vec<&[tree::Stmt]> = vec![body];
    while let Some(list) = open.pop() {
        for (at, stmt) in list.iter().enumerate() {
            let hidden = match stmt {
                tree::Stmt::Sugar(sugar) => match &**sugar {
                    Sugar::Hidden(hidden) => end_of(hidden),
                    _ => None,
                },
                _ => None,
            };
            match (hidden, handed(list.get(at + 1))) {
                (Some(false), Some(given)) => return AsyncResult::TaskOf(given),
                (Some(true), _) => found = AsyncResult::Task,
                _ => {}
            }
            open.extend(nested(stmt));
        }
    }
    found
}

/// The statement lists nested in `stmt`, a source construct's included.
fn nested(stmt: &tree::Stmt) -> Vec<&[tree::Stmt]> {
    match stmt {
        tree::Stmt::If { then, els, .. } => vec![then.as_slice(), els.as_slice()],
        tree::Stmt::While { body, .. }
        | tree::Stmt::DoWhile { body, .. }
        | tree::Stmt::For { body, .. } => vec![body.as_slice()],
        tree::Stmt::Switch { cases, .. } => cases.iter().map(|case| case.body.as_slice()).collect(),
        tree::Stmt::Sugar(sugar) => sugar.parts().bodies,
        _ => Vec::new(),
    }
}

/// The task a kick-off read as an `async` method hands back where its body ends: what its other `return` statements return too.
pub fn handed_back(body: &[tree::Stmt]) -> Option<&Expr> {
    let tree::Stmt::Sugar(sugar) = body.last()? else {
        return None;
    };
    match &**sugar {
        Sugar::Hidden(hidden) => match hidden.as_slice() {
            [tree::Stmt::Return(Some(task))] => Some(task),
            _ => None,
        },
        _ => None,
    }
}
