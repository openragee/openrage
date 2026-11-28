//! Values: the temporaries a lifter spreads an expression over, put back together.
//!
//! Works on the blocks, before types and structure. A rewrite is made only where what the code
//! computes and the order of its calls, stores and loads can be shown to stay; a temporary no
//! rewrite is sure of stays a variable.

#[path = "values_cells.rs"]
mod cells;
#[path = "values_effects.rs"]
pub(super) mod effects;
#[path = "values_fold.rs"]
pub(super) mod fold;
#[path = "values_joins.rs"]
mod joins;

use std::collections::BTreeSet;

use crate::gta6::script::cfg::{self, Block, BlockId, Cfg, Terminator, ENTRY};
use crate::gta6::script::ir::{
    BinOp, Expr, Head, Kind, Scalar, Stmt, StoreKind, UnOp, Var, VarInfo, Width,
};
use crate::gta6::script::names::Names;
use crate::gta6::script::{Context, Pass, Stats};

/// The rounds of rewrites a function is given: each feeds the next, and few functions gain past the third.
const ROUNDS: usize = 6;

/// How many times the rounds are given again after branches were written as choices: a choice may leave the arm of another.
const CHOICES: usize = 3;

/// Folds each single-use temporary into its use, makes one variable of the values a join carries and one of the cells of a value, and removes what is dead.
pub fn run(f: &mut cfg::Function, names: &Names, stats: &mut Stats) {
    run_in(f, &Context::baseline(names), stats);
}

/// [`run`] as the pipeline calls it: `escape`, `constants`, `coalesce`, `wide-joins`, `repeats` and `aggregates` each switch on a finer rule of this pass.
// `escape`: two accesses are told apart by the variables their addresses are in, and an allocation
// may change places with a call. `constants`: a test of constants is decided at every kind it is
// taken at, and so is a test of the variable the statement straight before it gave a constant;
// an arm that gives the tested variable the constant its test proved it holds leaves it as it is.
// `coalesce`: a local that copies a temporary keeps its name, a stack variable copied into or from
// a local is that local where their values never need to differ, and two arms that only choose a
// value for one variable are the one assignment of the choice, a stack slot's arms even where
// they call or choose; a choice between a variable and the constant its test proved it holds is
// the variable, and a stack variable that only ever carries one frame variable's address is that
// address. `aggregates`: a temporary of
// several cells that only ever holds one, its address handed out or not, is a temporary of one
// cell. `wide-joins`: temporaries of one
// cell each that only ever carry a value of several cells whole are one variable of its cells,
// the width stated by a value, a clear or a group.
// `repeats`: a test that repeats the one its block was entered by goes the way that one went.
// `Stats` counts temporaries on the trees either side of the passes, so nothing is counted here.
pub fn run_in(f: &mut cfg::Function, cx: &Context<'_>, _stats: &mut Stats) {
    if f.head.incomplete.is_some() || f.cfg.blocks.is_empty() {
        return;
    }
    let finer = cx.options.on(Pass::Escape);
    let ordered = cx.options.on(Pass::Constants);
    let keep = cx.options.on(Pass::Coalesce);
    let wide = cx.options.on(Pass::WideJoins);
    let again = cx.options.on(Pass::Repeats);
    let single = cx.options.on(Pass::Aggregates);
    named_accesses(&f.head, &mut f.cfg);
    tidy(&f.head, &mut f.cfg, ordered);
    joins::split_tuples(&mut f.cfg);
    for choices in 0..=CHOICES {
        let mut settled = false;
        for _ in 0..ROUNDS {
            // An address a round before put where a load or a store goes through it is the variable's own.
            named_accesses(&f.head, &mut f.cfg);
            let mut changed = fold::dead(f);
            // A cell zeroed, filled by one narrow store and read back is a value: as one it folds like any other.
            changed |= super::types::narrowings(f);
            changed |= fold::zeroes(f);
            changed |= fold::mirror(f);
            changed |= fold::fold(f, finer);
            changed |= fold::copies(f, keep);
            changed |= joins::roots(f, keep);
            changed |= joins::merge(f);
            changed |= keep && joins::unite(f);
            changed |= wide && joins::wholes(f);
            changed |= cells::gather(f, single);
            changed |= again && repeats(&mut f.cfg);
            changed |= ordered && proved(f);
            changed |= tidy(&f.head, &mut f.cfg, ordered);
            if !changed {
                settled = true;
                break;
            }
        }
        // ⚠ Only once nothing else moves: a join that a later round leaves as a bare test of what
        // its arms hand it is a condition, and must not have been written as a value before.
        if choices == CHOICES || !(keep && settled && joins::selects(f)) {
            break;
        }
        tidy(&f.head, &mut f.cfg, ordered);
    }
    fold::steps(f);
    declared(f);
}

/// Declares one cell wide each local that the form a front end made proves to be: an address handed out then reaches the variable it is the address of, and not the locals after it.
// ⚠ Once, on the form a front end made, before any pass: [`effects::single_cells`] proves
// nothing of blocks a pass has rewritten. The proof is `VarInfo::single`, which no other pass sets.
pub fn widths(f: &mut cfg::Function) {
    if f.head.incomplete.is_some() {
        return;
    }
    for cell in effects::single_cells(f) {
        let info = f.head.vars.entry(Var::Local(cell));
        let info = info.or_insert_with(|| VarInfo::new(Width::Cell));
        info.single = info.width == Width::Cell;
    }
}

/// The locals [`widths`] declares one cell wide, for a check that runs the form a front end made beside what the passes make of it.
pub fn single_locals(f: &cfg::Function) -> BTreeSet<u32> {
    effects::single_cells(f)
}

/// Leaves `head.vars` declaring the temporaries the code still mentions and no other.
pub(super) fn declared(f: &mut cfg::Function) {
    let facts = effects::Facts::of(f);
    f.head
        .vars
        .retain(|var, _| !var.is_temporary() || facts.uses.contains_key(var));
}

/// The local or the parameter `addr` is the address of.
fn frame_cell(addr: &Expr) -> Option<Var> {
    match addr {
        Expr::AddrOf {
            var: var @ (Var::Local(_) | Var::Param(_)),
            cell: 0,
        } => Some(*var),
        _ => None,
    }
}

/// Writes a load of a frame cell through its own address as the read of the variable it is, and a store of the whole cell as the assignment.
// A listing reads `local_3` as `load_i32(Vm.Address(locals, 3), 0x0)` wherever the source read a
// field of it. The low bytes of the cell are the same read either way, and with no address
// taken the variable is one only its name changes.
// ⚠ A store of fewer than eight bytes leaves the rest of the cell, so it stays a store.
// ⚠ Never of a variable of several cells: its name is all of them, and the access one cell.
fn named_accesses(head: &Head, cfg: &mut Cfg) {
    let narrow = |addr: &Expr| {
        let wide = |var: &Var| (head.vars.get(var)).is_some_and(|info| info.width == Width::Cells);
        frame_cell(addr).filter(|var| !wide(var))
    };
    for block in &mut cfg.blocks {
        for stmt in &mut block.stmts {
            let whole = match &*stmt {
                Stmt::Store {
                    kind: StoreKind::U64 | StoreKind::Ref,
                    addr,
                    offset: 0,
                    ..
                } => narrow(addr),
                _ => None,
            };
            let Some(target) = whole else {
                continue;
            };
            let store = std::mem::replace(stmt, Stmt::Eval(Expr::GuidEmpty));
            if let Stmt::Store { value, .. } = store {
                *stmt = Stmt::Assign { target, value };
            }
        }
    }
    effects::each_top_mut(cfg, &mut |top| {
        top.walk_mut(&mut |e| {
            let read = match &*e {
                Expr::Load {
                    ty,
                    addr,
                    offset: 0,
                } => narrow(addr).map(|var| (*ty, var)),
                _ => None,
            };
            if let Some((ty, var)) = read {
                *e = match ty {
                    Scalar::U64 => Expr::Var(var),
                    ty => Expr::cast(ty, Expr::Var(var)),
                };
            }
        });
    });
}

/// How two constant cells compare as operands of the kind `at`; `None` for a kind that orders nothing, `Some(None)` where one is no number.
fn compared(at: &Kind, l: u64, r: u64) -> Option<Option<std::cmp::Ordering>> {
    Some(match at {
        Kind::I8 | Kind::U8 | Kind::I16 | Kind::U16 | Kind::I32 => {
            Some((l as i32).cmp(&(r as i32)))
        }
        Kind::U32 => Some((l as u32).cmp(&(r as u32))),
        Kind::I64 => Some((l as i64).cmp(&(r as i64))),
        Kind::U64 => Some(l.cmp(&r)),
        Kind::F32 => f32::from_bits(l as u32).partial_cmp(&f32::from_bits(r as u32)),
        Kind::F64 => f64::from_bits(l).partial_cmp(&f64::from_bits(r)),
        _ => return None,
    })
}

/// The cell an expression of constants, integer readings and tests of equality comes to; `None` for any other.
// With `ordered`, the readings as floats and the tests of order too, each at the kind its operator
// works at. ⚠ A test of a float that is no number is false whichever way it is asked, but for `!=`.
// `known` is a variable and the cell it holds where the expression is evaluated: a read of it is that cell.
fn constant(e: &Expr, depth: u32, ordered: bool, known: Option<(Var, u64)>) -> Option<u64> {
    let d = depth.checked_sub(1)?;
    Some(match e {
        Expr::Const(c) => c.bits,
        Expr::Var(var) => known.filter(|(held, _)| held == var)?.1,
        Expr::Truth(e) => u64::from(constant(e, d, ordered, known)? != 0),
        Expr::Unary { op: UnOp::Not, e } => u64::from(constant(e, d, ordered, known)? == 0),
        Expr::Cast { to, e } => {
            let bits = constant(e, d, ordered, known)?;
            match to {
                Scalar::I8 => bits as i8 as i64 as u64,
                Scalar::U8 => u64::from(bits as u8),
                Scalar::I16 => bits as i16 as i64 as u64,
                Scalar::U16 => u64::from(bits as u16),
                Scalar::I32 => bits as i32 as i64 as u64,
                Scalar::U32 => u64::from(bits as u32),
                Scalar::I64 | Scalar::U64 => bits,
                Scalar::F32 if ordered => u64::from(bits as u32),
                Scalar::F64 if ordered => bits,
                Scalar::F32 | Scalar::F64 => return None,
            }
        }
        Expr::Binary {
            op: op @ (BinOp::Eq | BinOp::Ne),
            at,
            l,
            r,
        } => {
            let (l, r) = (
                constant(l, d, ordered, known)?,
                constant(r, d, ordered, known)?,
            );
            let equal = match at {
                Kind::Bool => (l != 0) == (r != 0),
                Kind::I32 | Kind::U32 => l as u32 == r as u32,
                Kind::I64 | Kind::U64 | Kind::Unknown => l == r,
                Kind::F32 | Kind::F64 if ordered => {
                    compared(at, l, r)?.is_some_and(std::cmp::Ordering::is_eq)
                }
                _ => return None,
            };
            u64::from(equal == (*op == BinOp::Eq))
        }
        Expr::Binary {
            op: op @ (BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge),
            at,
            l,
            r,
        } if ordered => {
            let (l, r) = (
                constant(l, d, ordered, known)?,
                constant(r, d, ordered, known)?,
            );
            let order = compared(at, l, r)?;
            u64::from(order.is_some_and(|order| match op {
                BinOp::Lt => order.is_lt(),
                BinOp::Le => order.is_le(),
                BinOp::Gt => order.is_gt(),
                _ => order.is_ge(),
            }))
        }
        _ => return None,
    })
}

/// Removes the blocks nothing reaches and those that only jump on, and joins a block to the one block that leads to it; `ordered` is [`constant`]'s.
fn tidy(head: &Head, cfg: &mut Cfg, ordered: bool) -> bool {
    let n = cfg.blocks.len();
    let mut changed = false;

    // A block with no statement that jumps on is passed through. A ring of them is a loop that
    // does nothing, and stays.
    // ponytail: each block walks its own chain; a chain of thousands of empty blocks would want the ends remembered.
    let forward: Vec<BlockId> = (0..n)
        .map(|b| {
            let mut end = b;
            for _ in 0..n {
                match &cfg.blocks[end] {
                    Block {
                        stmts,
                        term: Terminator::Jump(next),
                        ..
                    } if end != ENTRY && stmts.is_empty() && *next != end => end = *next,
                    _ => return end,
                }
            }
            b
        })
        .collect();
    for block in &mut cfg.blocks {
        block.term.retarget(|target| forward[target]);
        // The variable the statement straight before the terminator gave a constant holds it
        // where the terminator reads it: nothing stands between the two, whatever else reaches
        // the variable. ⚠ One cell, read by its name: a global is read from memory.
        let known = match block.stmts.last() {
            Some(Stmt::Assign { target, value }) if ordered => {
                let many = (head.vars.get(target)).is_some_and(|info| info.width == Width::Cells);
                let cell = constant(value, 16, ordered, None);
                let named = !matches!(target, Var::Global { .. }) && !many;
                cell.filter(|_| named).map(|cell| (*target, cell))
            }
            _ => None,
        };
        // A branch whose arms meet decides nothing, and goes where evaluating its condition does nothing either.
        let met = match &block.term {
            Terminator::Branch { cond, then, els } if then == els && effects::inert(cond) => {
                Some(*then)
            }
            // A test of constants goes one way.
            Terminator::Branch { cond, then, els } => {
                constant(cond, 16, ordered, known).map(|cell| if cell != 0 { *then } else { *els })
            }
            // A switch on a constant goes to the case that is the constant as an `int`.
            Terminator::Switch {
                value,
                cases,
                default,
            } if ordered => constant(value, 16, ordered, known).map(|cell| {
                let wanted = i64::from(cell as i32);
                let case = cases.iter().find(|case| case.0 == wanted);
                case.map_or(*default, |case| case.1)
            }),
            _ => None,
        };
        if let Some(target) = met {
            block.term = Terminator::Jump(target);
            changed = true;
        }
    }

    let preds = cfg.predecessors();
    let mut gone: Vec<bool> = cfg.reachable().iter().map(|reached| !reached).collect();
    for b in 0..n {
        if gone[b] {
            continue;
        }
        // A block only `b` leads to, by a plain jump, is the rest of `b`.
        while let Terminator::Jump(next) = &cfg.blocks[b].term {
            let next = *next;
            if next == b || next == ENTRY || gone[next] || preds[next].len() != 1 {
                break;
            }
            let emptied = Block {
                at: None,
                stmts: Vec::new(),
                term: Terminator::Unreachable,
            };
            let rest = std::mem::replace(&mut cfg.blocks[next], emptied);
            cfg.blocks[b].stmts.extend(rest.stmts);
            cfg.blocks[b].term = rest.term;
            gone[next] = true;
        }
    }

    if gone.iter().any(|gone| *gone) {
        let mut renumbered = vec![usize::MAX; n];
        let mut kept = 0;
        for b in 0..n {
            if !gone[b] {
                renumbered[b] = kept;
                kept += 1;
            }
        }
        let old = std::mem::take(&mut cfg.blocks);
        for (b, mut block) in old.into_iter().enumerate() {
            if !gone[b] {
                block.term.retarget(|target| renumbered[target]);
                cfg.blocks.push(block);
            }
        }
        changed = true;
    }
    changed
}

/// How deep a test is looked into for the readings of truth around it.
const TRUTHS: usize = 16;

/// `e` with the readings of truth around it taken off: a branch on either goes the same way.
fn tested(e: &Expr) -> &Expr {
    let mut e = e;
    for _ in 0..TRUTHS {
        match e {
            Expr::Truth(inner) => e = &**inner,
            _ => break,
        }
    }
    e
}

/// Whether a branch on `b` goes the way a branch on `a` goes (`Some(false)`) or the other way (`Some(true)`), by their forms alone.
// ⚠ `l == r` and `l != r` at one kind are each other's opposite, a float that is no number too.
fn opposite(a: &Expr, b: &Expr) -> Option<bool> {
    let (a, b) = (tested(a), tested(b));
    if a == b {
        return Some(false);
    }
    let flipped = |op: BinOp| match op {
        BinOp::Eq => Some(BinOp::Ne),
        BinOp::Ne => Some(BinOp::Eq),
        _ => None,
    };
    match (a, b) {
        (Expr::Unary { op: UnOp::Not, e }, other) | (other, Expr::Unary { op: UnOp::Not, e })
            if tested(e) == other =>
        {
            Some(true)
        }
        (
            Expr::Binary { op, at, l, r },
            Expr::Binary {
                op: other,
                at: at_b,
                l: l_b,
                r: r_b,
            },
        ) if flipped(*op) == Some(*other) && at == at_b && l == l_b && r == r_b => Some(true),
        _ => None,
    }
}

/// Sends each test that repeats the test its block was entered by, with nothing run between, the way that test went.
// `if (c) { if (c) { A } else { B } }` is `if (c) { A }`: the block of the second test holds no
// statement and is entered from the first alone, by the arm `c` chose. The arm left behind has
// nothing that leads to it, for `tidy` to remove.
// ⚠ Only a test that gives the same again and does nothing when evaluated again: no call and no new object.
fn repeats(cfg: &mut Cfg) -> bool {
    let preds = cfg.predecessors();
    let mut decided: Vec<(BlockId, BlockId)> = Vec::new();
    for (b, block) in cfg.blocks.iter().enumerate() {
        let Terminator::Branch { cond, then, els } = &block.term else {
            continue;
        };
        let [p] = preds[b].as_slice() else {
            continue;
        };
        if b == ENTRY || *p == b || !block.stmts.is_empty() {
            continue;
        }
        let Terminator::Branch {
            cond: first,
            then: yes,
            els: no,
        } = &cfg.blocks[*p].term
        else {
            continue;
        };
        let held = match (*yes == b, *no == b) {
            (true, false) => true,
            (false, true) => false,
            _ => continue,
        };
        let Some(flipped) = opposite(first, cond) else {
            continue;
        };
        if effects::repeatable(cond) {
            decided.push((b, if held != flipped { *then } else { *els }));
        }
    }
    let changed = !decided.is_empty();
    for (b, to) in decided {
        cfg.blocks[b].term = Terminator::Jump(to);
    }
    changed
}

/// Whether an operator of equality at `at` compares every bit of its two cells.
fn whole_compare(at: &Kind) -> bool {
    matches!(
        at,
        Kind::I64
            | Kind::U64
            | Kind::Unknown
            | Kind::Pointer
            | Kind::Object(_)
            | Kind::String
            | Kind::Array(_)
    )
}

/// `e` under the readings that keep every bit of its cell.
fn whole_cell(e: &Expr) -> &Expr {
    let mut e = e;
    for _ in 0..TRUTHS {
        match e {
            Expr::Cast {
                to: Scalar::I64 | Scalar::U64,
                e: inner,
            } => e = &**inner,
            _ => break,
        }
    }
    e
}

/// The variable a branch on `cond` tests whole against a constant, that constant's cell, and whether the variable holds it where the branch goes to `then` (else where it goes to `els`).
// `x != 0`, `x == c`, `!x` and a branch on `x` itself each say what all of `x` holds on one way
// out. ⚠ Only a test of every bit: `(int)x != 0` leaves the upper half unknown. The caller
// checks that the variable is one cell wide.
fn pinned(cond: &Expr) -> Option<(Var, u64, bool)> {
    let mut e = cond;
    let mut negated = false;
    for _ in 0..TRUTHS {
        match e {
            Expr::Truth(inner) => e = &**inner,
            Expr::Unary {
                op: UnOp::Not,
                e: inner,
            } => {
                e = &**inner;
                negated = !negated;
            }
            Expr::Binary {
                op: op @ (BinOp::Eq | BinOp::Ne),
                at,
                l,
                r,
            } => {
                if !whole_compare(at) {
                    return None;
                }
                let (var, cell) = match (whole_cell(l), whole_cell(r)) {
                    (Expr::Var(var), Expr::Const(c)) | (Expr::Const(c), Expr::Var(var)) => {
                        (*var, c.bits)
                    }
                    _ => return None,
                };
                let named = !matches!(var, Var::Global { .. });
                return named.then_some((var, cell, (*op == BinOp::Eq) != negated));
            }
            // A branch on a value goes to `then` where any bit of it is set.
            other => {
                let Expr::Var(var) = whole_cell(other) else {
                    return None;
                };
                let named = !matches!(var, Var::Global { .. });
                return named.then_some((*var, 0, negated));
            }
        }
    }
    None
}

/// Removes the assignment an arm starts with where the test that alone leads into it proved the variable already holds that constant: `if (x != 0) { .. } else { x = 0; .. }`.
// Nothing runs between the test and the arm's first statement, so the cell is the one the test
// read. ⚠ One cell, read by its name: a variable of several cells is more than the test read.
fn proved(f: &mut cfg::Function) -> bool {
    let facts = effects::Facts::of(f);
    let preds = f.cfg.predecessors();
    let mut gone: Vec<BlockId> = Vec::new();
    for (b, block) in f.cfg.blocks.iter().enumerate() {
        let Terminator::Branch { cond, then, els } = &block.term else {
            continue;
        };
        if then == els {
            continue;
        }
        let Some((var, cell, on_then)) = pinned(cond) else {
            continue;
        };
        let arm = if on_then { *then } else { *els };
        let only = preds.get(arm).is_some_and(|from| from.as_slice() == [b]);
        if arm == b || arm == ENTRY || !only {
            continue;
        }
        let narrow =
            effects::one_cell(&f.head, &Expr::Var(var)) && !facts.widths.contains_key(&var);
        let first = f.cfg.blocks.get(arm).and_then(|arm| arm.stmts.first());
        let again = matches!(
            first,
            Some(Stmt::Assign {
                target,
                value: Expr::Const(c),
            }) if *target == var && c.bits == cell
        );
        if narrow && again {
            gone.push(arm);
        }
    }
    let changed = !gone.is_empty();
    for arm in gone {
        if let Some(block) = f.cfg.blocks.get_mut(arm) {
            if !block.stmts.is_empty() {
                block.stmts.remove(0);
            }
        }
    }
    changed
}

#[cfg(test)]
#[path = "values_tests.rs"]
mod tests;
