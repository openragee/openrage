//! Temporaries put back into the expressions they were taken out of, and assignments nothing reads removed.

use std::collections::{HashMap, HashSet};

use crate::gta6::script::cfg::{self, dominates, Block, BlockId, Terminator};
use crate::gta6::script::ir::{
    BinOp, Callee, Const, Expr, Head, Kind, Scalar, Stmt, StoreKind, Var, Width,
};

use super::effects::{cells_of, depth, evaluated, inert, one_cell, stored, Effects, Facts};

#[path = "values_fold_across.rs"]
mod across;
pub(in crate::gta6::script::passes) use self::across::across;
#[path = "values_fold_chain.rs"]
mod chain;
pub(in crate::gta6::script::passes) use self::chain::chained;

/// How many statements may stand between a value's assignment and the place it moves to.
// ponytail: a fixed reach keeps a block of thousands of statements linear; a running summary of
// the effects between two statements would lift it.
const REACH: usize = 256;

/// The deepest an expression is let grow, so that walking one never runs out of stack.
const DEPTH: usize = 48;

/// The most nodes of an expression of constants that is written wherever its variable was read.
const CONSTANT: usize = 8;

/// Whether `e` works one cell out of constants alone, by operators that cannot fault: it is the same wherever it is evaluated.
pub(in crate::gta6::script::passes) fn constant(e: &Expr) -> bool {
    let mut nodes = 0usize;
    let mut plain = true;
    e.walk(&mut |node| {
        nodes += 1;
        plain &= match node {
            Expr::Const(_) | Expr::Cast { .. } | Expr::Truth(_) | Expr::Unary { .. } => true,
            Expr::Binary { op, .. } => !matches!(op, BinOp::Div | BinOp::Rem),
            _ => false,
        };
    });
    plain && nodes <= CONSTANT
}

/// Whether `e` compares the bytes of variables through their own addresses, which are always there to read.
fn compares_variables(e: &Expr) -> bool {
    match e {
        Expr::Call {
            callee: Callee::Intrinsic { name, .. },
            args,
            ..
        } => {
            name.starts_with("bytes_equal_")
                && args.iter().all(|arg| matches!(arg, Expr::AddrOf { .. }))
        }
        _ => false,
    }
}

fn reads_in(e: &Expr, var: Var) -> u32 {
    let mut reads = 0;
    e.walk(&mut |node| reads += u32::from(matches!(node, Expr::Var(read) if *read == var)));
    reads
}

fn reads(stmt: &Stmt, var: Var) -> u32 {
    let mut found = 0;
    stmt.each_expr(&mut |e| found += reads_in(e, var));
    found
}

/// Writes `to` wherever `top` reads `var`.
fn renamed(top: &mut Expr, var: Var, to: Var) {
    top.walk_mut(&mut |e| {
        if matches!(e, Expr::Var(read) if *read == var) {
            *e = Expr::Var(to);
        }
    });
}

/// What is left of a value nothing reads: nothing, the call it makes, or (`Err`) all of it.
pub(in crate::gta6::script::passes) fn leftover(
    facts: &Facts,
    value: Expr,
) -> Result<Option<Expr>, Expr> {
    // A global is read by its name and through no pointer: not reading it loses nothing either.
    if Effects::of_expr(facts, &value).free() || inert(&value) {
        return Ok(None);
    }
    // Past the readings of its result, a call stands as a statement.
    let mut inner = &value;
    while let Expr::Cast { e: next, .. } | Expr::Truth(next) | Expr::Cell { base: next, .. } = inner
    {
        inner = &**next;
    }
    if compares_variables(inner) {
        return Ok(None);
    }
    let call = matches!(inner, Expr::Call { .. }).then(|| inner.clone());
    match call {
        Some(call) => Ok(Some(call)),
        None => Err(value),
    }
}

/// Removes each assignment to a variable nothing reads, keeping a call on its right side as a statement.
pub(super) fn dead(f: &mut cfg::Function) -> bool {
    let facts = Facts::of(f);
    let mut changed = false;
    for block in &mut f.cfg.blocks {
        let old = std::mem::take(&mut block.stmts);
        for stmt in old {
            match stmt {
                Stmt::Assign { target, value } if facts.unread(target) => {
                    match leftover(&facts, value) {
                        Ok(None) => changed = true,
                        Ok(Some(call)) => {
                            block.stmts.push(Stmt::Eval(call));
                            changed = true;
                        }
                        Err(value) => block.stmts.push(Stmt::Assign { target, value }),
                    }
                }
                Stmt::AssignMany { targets, values } => {
                    let pairs = targets.len();
                    let mut kept_targets = Vec::with_capacity(pairs);
                    let mut kept_values = Vec::with_capacity(pairs);
                    for (target, value) in targets.into_iter().zip(values) {
                        if facts.unread(target) && Effects::of_expr(&facts, &value).free() {
                            continue;
                        }
                        kept_targets.push(target);
                        kept_values.push(value);
                    }
                    changed |= kept_targets.len() != pairs;
                    match (kept_targets.len(), kept_targets.first().copied()) {
                        (1, Some(target)) => {
                            if let Some(value) = kept_values.pop() {
                                block.stmts.push(Stmt::Assign { target, value });
                            }
                        }
                        (0, _) => {}
                        _ => block.stmts.push(Stmt::AssignMany {
                            targets: kept_targets,
                            values: kept_values,
                        }),
                    }
                }
                other => block.stmts.push(other),
            }
        }
    }
    changed
}

/// Replaces each local that is only ever given zero by zero, and removes what gives it.
// A frame starts as zeroes. A local that every write leaves zero and that no pointer reaches is
// zero wherever it is read: `zero_bytes(&local_0, 4); f(local_0);` is `f(0);`.
// ⚠ Every address of the local must be one of the clears counted here, and no load or store
// through another local's address may cover its cells.
pub(super) fn zeroes(f: &mut cfg::Function) -> bool {
    let facts = Facts::of(f);
    // The clears through each local's own address, and the locals something else writes.
    let mut cleared: HashMap<Var, u32> = HashMap::new();
    let mut spoiled: HashSet<Var> = HashSet::new();
    for stmt in f.cfg.blocks.iter().flat_map(|block| &block.stmts) {
        match stmt {
            Stmt::ZeroBytes {
                addr:
                    Expr::AddrOf {
                        var: var @ Var::Local(_),
                        cell: 0,
                    },
                bytes,
            } => {
                let cells = facts.widths.get(var).copied().unwrap_or(1);
                match u64::from(*bytes) <= 8 * u64::from(cells) {
                    true => *cleared.entry(*var).or_default() += 1,
                    false => {
                        spoiled.insert(*var);
                    }
                }
            }
            Stmt::Assign {
                target: target @ Var::Local(_),
                value,
            } => {
                let width = facts.widths.get(target).copied();
                let zero = match value {
                    Expr::Const(c) => c.bits == 0 && width.is_none(),
                    Expr::ZeroSlots(cells) => width == Some(*cells),
                    Expr::GuidEmpty => width == Some(2),
                    _ => false,
                };
                match zero {
                    true => {
                        cleared.entry(*target).or_default();
                    }
                    false => {
                        spoiled.insert(*target);
                    }
                }
            }
            other => spoiled.extend(other.targets().iter().copied()),
        }
    }
    let zero: HashMap<Var, Expr> = cleared
        .into_iter()
        .filter(|(var, clears)| {
            let uses = facts.uses.get(var).copied().unwrap_or_default();
            let Some(place) = facts.place(*var) else {
                return false;
            };
            let Var::Local(lo) = *var else {
                return false;
            };
            let hi = lo.saturating_add(facts.widths.get(var).copied().unwrap_or(1).max(1) - 1);
            let covering = (facts.direct_locals.iter())
                .filter(|(from, to)| *from <= hi && lo <= *to)
                .count();
            let own = uses.addrs == *clears && covering == *clears as usize;
            !spoiled.contains(var) && own && !facts.exposed(place)
        })
        .map(|(var, _)| {
            let value = match facts.widths.get(&var) {
                Some(cells) => Expr::ZeroSlots(*cells),
                None => Expr::Const(Const::i32(0)),
            };
            (var, value)
        })
        .collect();
    if zero.is_empty() {
        return false;
    }
    for block in &mut f.cfg.blocks {
        block.stmts.retain(|stmt| match stmt {
            Stmt::ZeroBytes {
                addr: Expr::AddrOf { var, .. },
                ..
            } => !zero.contains_key(var),
            Stmt::Assign { target, .. } => !zero.contains_key(target),
            _ => true,
        });
        let mut rename = |top: &mut Expr| {
            top.walk_mut(&mut |e| {
                let value = match &*e {
                    Expr::Var(read) => zero.get(read).cloned(),
                    _ => None,
                };
                if let Some(value) = value {
                    *e = value;
                }
            });
        };
        for stmt in &mut block.stmts {
            stmt.each_expr_mut(&mut rename);
        }
        if let Some(e) = block.term.expr_mut() {
            rename(e);
        }
    }
    // A local of several cells that is gone is no variable of the function any more.
    for var in zero.keys() {
        f.head.vars.remove(var);
    }
    true
}

/// Counts each load in `top` through a local's own address, and spoils the local where the load goes through the address of a later cell.
fn own_loads(top: &Expr, own: &mut HashMap<Var, u32>, spoiled: &mut HashSet<Var>) {
    top.walk(&mut |e| {
        let (Expr::Load { addr, .. } | Expr::LoadBytes { addr, .. }) = e else {
            return;
        };
        if let Expr::AddrOf {
            var: var @ Var::Local(_),
            cell,
        } = &**addr
        {
            *own.entry(*var).or_default() += 1;
            if *cell != 0 {
                spoiled.insert(*var);
            }
        }
    });
}

/// Writes each clear of the low four to seven bytes of a one-cell local as `local = 0`, where nothing ever writes a byte above the fourth but a clear.
// A frame starts as zeroes: bytes that only clears and constants of 32 bits ever write are zero all
// along, so a clear of every byte below them leaves the whole cell zero, as `local = 0` does.
// ⚠ Every address of the local must be one of its own loads and stores, no other local's address
// may reach its cell, and no pointer either: a write the function does not state may reach the
// upper bytes.
pub(in crate::gta6::script::passes) fn low_clears(f: &mut cfg::Function) -> bool {
    let facts = Facts::of(f);
    // The accesses through each local's own address, the locals a clear of the low bytes leaves
    // whole, and the locals something writes above the low four bytes.
    let mut own: HashMap<Var, u32> = HashMap::new();
    let mut clears: HashSet<Var> = HashSet::new();
    let mut spoiled: HashSet<Var> = HashSet::new();
    for block in &f.cfg.blocks {
        for stmt in &block.stmts {
            if let Some((
                Expr::AddrOf {
                    var: var @ Var::Local(_),
                    cell,
                },
                offset,
                bytes,
            )) = stored(stmt)
            {
                *own.entry(*var).or_default() += 1;
                let low = bytes.is_some_and(|bytes| offset.saturating_add(bytes) <= 4);
                match stmt {
                    // Zeroes, however far they run.
                    Stmt::ZeroBytes { bytes, .. } if *cell == 0 => {
                        if (4..8).contains(bytes) {
                            clears.insert(*var);
                        }
                    }
                    _ if *cell == 0 && low => {}
                    _ => {
                        spoiled.insert(*var);
                    }
                }
            }
            match stmt {
                Stmt::Assign {
                    value: Expr::Const(c),
                    ..
                } if c.bits <= u64::from(u32::MAX) => {}
                other => spoiled.extend(other.targets().iter().copied()),
            }
            stmt.each_expr(&mut |top| own_loads(top, &mut own, &mut spoiled));
        }
        if let Some(e) = block.term.expr() {
            own_loads(e, &mut own, &mut spoiled);
        }
    }
    let whole: HashSet<Var> = clears
        .into_iter()
        .filter(|var| {
            let Var::Local(n) = *var else {
                return false;
            };
            let uses = facts.uses.get(var).copied().unwrap_or_default();
            let accesses = own.get(var).copied().unwrap_or(0);
            let covering = (facts.direct_locals.iter())
                .filter(|(lo, hi)| *lo <= n && n <= *hi)
                .count();
            let alone = uses.addrs == uses.direct
                && uses.direct == accesses
                && covering == accesses as usize;
            let narrow = !facts.widths.contains_key(var)
                && (f.head.vars.get(var)).is_none_or(|info| info.width != Width::Cells);
            let hidden = facts.place(*var).is_some_and(|place| !facts.exposed(place));
            !spoiled.contains(var) && alone && narrow && hidden
        })
        .collect();
    if whole.is_empty() {
        return false;
    }
    for stmt in f.cfg.blocks.iter_mut().flat_map(|block| &mut block.stmts) {
        let var = match &*stmt {
            Stmt::ZeroBytes {
                addr: Expr::AddrOf { var, cell: 0 },
                bytes,
            } if (4..8).contains(bytes) && whole.contains(var) => *var,
            _ => continue,
        };
        *stmt = Stmt::Assign {
            target: var,
            value: Expr::Const(Const::i32(0)),
        };
    }
    true
}

/// The copy a temporary's value can move into, and the local that then holds it for every later reader.
struct Mirror {
    /// The statement that assigns the local.
    at: usize,
    local: Var,
    /// Whether that statement evaluates the value again, where otherwise it copies the temporary.
    twin: bool,
}

/// Whether the temporary statement `i` assigns is mirrored by a local every later reader can read in its place.
fn mirrored(facts: &Facts, head: &Head, block: &Block, i: usize) -> Option<Mirror> {
    let Stmt::Assign { target, value } = &block.stmts[i] else {
        return None;
    };
    let var = *target;
    let many = head
        .vars
        .get(&var)
        .is_some_and(|info| info.width == Width::Cells);
    let uses = facts.uses.get(&var).copied()?;
    if !var.is_temporary() || many || uses.defs != 1 || uses.addrs != 0 {
        return None;
    }
    let moved = Effects::of_expr(facts, value);
    // `v = E; local = E;`, where evaluating `E` once gives what evaluating it twice gave.
    let twin = match block.stmts.get(i + 1) {
        Some(Stmt::Assign {
            target,
            value: again,
        }) if again == value && !moved.call && !moved.alloc => Some(*target),
        _ => None,
    };
    let (at, local) = match twin {
        Some(local) => (i + 1, local),
        None => {
            // `v = E; ..; local = v;`, the copy being the first to read the temporary.
            let end = block.stmts.len().min(i + 1 + REACH);
            let at = (i + 1..end).find(|j| reads(&block.stmts[*j], var) > 0)?;
            let Stmt::Assign {
                target,
                value: Expr::Var(copied),
            } = &block.stmts[at]
            else {
                return None;
            };
            if *copied != var || uses.reads < 2 {
                return None;
            }
            (at, *target)
        }
    };
    let place = facts.place(local)?;
    let plain = matches!(local, Var::Local(_) | Var::Param(_))
        && !facts.exposed(place)
        && !facts.widths.contains_key(&local);
    if !plain {
        return None;
    }
    for crossed in &block.stmts[i + 1..at] {
        if !moved.may_cross(&Effects::of_stmt(facts, crossed)) {
            return None;
        }
    }

    // Every other read comes later in this block, and the local still holds the value at each.
    let mut left = uses.reads - u32::from(twin.is_none());
    let mut last = at;
    let end = block.stmts.len().min(at + 1 + REACH);
    for (j, stmt) in block.stmts.iter().enumerate().take(end).skip(at + 1) {
        if left == 0 {
            break;
        }
        let found = reads(stmt, var);
        if found > 0 {
            left = left.checked_sub(found)?;
            last = j;
        }
    }
    if left > 0 && end == block.stmts.len() {
        let found = block.term.expr().map_or(0, |e| reads_in(e, var));
        left = left.checked_sub(found)?;
        last = block.stmts.len();
    }
    if left > 0 {
        return None;
    }
    // The statement that reads last may assign the local itself: it reads before it writes.
    let between = at + 1..last.clamp(at + 1, block.stmts.len());
    let rewritten = block.stmts[between]
        .iter()
        .any(|stmt| Effects::of_stmt(facts, stmt).writes(place));
    (!rewritten).then_some(Mirror {
        at,
        local,
        twin: twin.is_some(),
    })
}

/// The copy `local = var;` that can stand where statement `i` assigns `var`, so that the local holds the value from the start.
// `v = E; v->f_8 = a; local = v;` is `local = E; local->f_8 = a;`: `E` stays where it was
// evaluated, and only the local's assignment moves, over statements that neither read the
// local's old value nor write it. `var` is a temporary or a local kept like one, and every read
// of it is in this block.
fn hoisted(facts: &Facts, head: &Head, block: &Block, i: usize) -> Option<(usize, Var)> {
    let Stmt::Assign { target, .. } = &block.stmts[i] else {
        return None;
    };
    let var = *target;
    let many = (head.vars.get(&var)).is_some_and(|info| info.width == Width::Cells)
        || facts.widths.contains_key(&var);
    let uses = facts.uses.get(&var).copied()?;
    let kept = matches!(var, Var::Local(_)) && facts.named_only(var);
    let lone = uses.defs == 1 && uses.addrs == 0 && uses.reads >= 2;
    if !(var.is_temporary() || kept) || many || !lone {
        return None;
    }
    let end = block.stmts.len().min(i + 1 + REACH);
    let (at, local) = (i + 1..end).find_map(|j| match &block.stmts[j] {
        Stmt::Assign {
            target,
            value: Expr::Var(copied),
        } if *copied == var && *target != var => Some((j, *target)),
        _ => None,
    })?;
    let place = facts.place(local)?;
    let plain = matches!(local, Var::Local(_) | Var::Param(_))
        && !facts.exposed(place)
        && !facts.widths.contains_key(&local);
    let untouched = || {
        (block.stmts[i + 1..at].iter()).all(|stmt| !Effects::of_stmt(facts, stmt).touches(place))
    };
    if !plain || !untouched() {
        return None;
    }

    // Every read is in this block: up to the copy, and after it while the local still holds the value.
    let mut left = uses.reads;
    for stmt in &block.stmts[i + 1..=at] {
        left = left.checked_sub(reads(stmt, var))?;
    }
    let mut last = at;
    let end = block.stmts.len().min(at + 1 + REACH);
    for (j, stmt) in block.stmts.iter().enumerate().take(end).skip(at + 1) {
        if left == 0 {
            break;
        }
        let found = reads(stmt, var);
        if found > 0 {
            left = left.checked_sub(found)?;
            last = j;
        }
    }
    if left > 0 && end == block.stmts.len() {
        let found = block.term.expr().map_or(0, |e| reads_in(e, var));
        left = left.checked_sub(found)?;
        last = block.stmts.len();
    }
    if left > 0 {
        return None;
    }
    let between = at + 1..last.clamp(at + 1, block.stmts.len());
    let rewritten = block.stmts[between]
        .iter()
        .any(|stmt| Effects::of_stmt(facts, stmt).writes(place));
    (!rewritten).then_some((at, local))
}

/// Where a temporary and a local are given one value, lets the local hold it for both.
pub(super) fn mirror(f: &mut cfg::Function) -> bool {
    let facts = Facts::of(f);
    let head = &f.head;
    let mut changed = false;
    // ⚠ The facts were counted before any rewrite here. A rewrite gives its local the reads of
    // the variable it stood in for, so that local's count is stale: nothing more is proved of
    // it by counting its reads until the facts are counted anew.
    let mut stale: HashSet<Var> = HashSet::new();
    for block in &mut f.cfg.blocks {
        let mut i = 0;
        while i < block.stmts.len() {
            let Some(found) = mirrored(&facts, head, block, i) else {
                let counted = !matches!(&block.stmts[i], Stmt::Assign { target, .. } if stale.contains(target));
                let hoist = match counted {
                    true => hoisted(&facts, head, block, i),
                    false => None,
                };
                if let Some((at, local)) = hoist {
                    stale.insert(local);
                    block.stmts.remove(at);
                    let var = match &mut block.stmts[i] {
                        Stmt::Assign { target, .. } => std::mem::replace(target, local),
                        _ => local,
                    };
                    for stmt in &mut block.stmts[i + 1..] {
                        stmt.each_expr_mut(&mut |top| renamed(top, var, local));
                    }
                    if let Some(top) = block.term.expr_mut() {
                        renamed(top, var, local);
                    }
                    changed = true;
                }
                i += 1;
                continue;
            };
            let Stmt::Assign { target: var, value } = block.stmts.remove(i) else {
                i += 1;
                continue;
            };
            stale.insert(found.local);
            // With statement `i` gone, the local's assignment is one place nearer.
            let at = found.at - 1;
            if !found.twin {
                block.stmts[at] = Stmt::Assign {
                    target: found.local,
                    value,
                };
            }
            for stmt in &mut block.stmts[at + 1..] {
                stmt.each_expr_mut(&mut |top| renamed(top, var, found.local));
            }
            if let Some(top) = block.term.expr_mut() {
                renamed(top, var, found.local);
            }
            changed = true;
        }
    }
    changed
}

/// Calls `f` on the expressions site `k` of a block evaluates: statement `k`, or past the last the terminator.
fn site<'a>(
    stmts: &'a [Option<Stmt>],
    term: &'a Terminator,
    k: usize,
    f: &mut impl FnMut(&'a Expr),
) {
    match stmts.get(k) {
        Some(Some(stmt)) => stmt.each_expr(f),
        Some(None) => {}
        None => {
            if let Some(e) = term.expr() {
                f(e);
            }
        }
    }
}

fn site_mut(
    stmts: &mut [Option<Stmt>],
    term: &mut Terminator,
    k: usize,
    f: &mut impl FnMut(&mut Expr),
) {
    match stmts.get_mut(k) {
        Some(Some(stmt)) => stmt.each_expr_mut(f),
        Some(None) => {}
        None => {
            if let Some(e) = term.expr_mut() {
                f(e);
            }
        }
    }
}

/// Whether `top` reads `var` in an arm it may not evaluate.
fn guarded(top: &Expr, var: Var) -> bool {
    let mut guarded = false;
    top.walk(&mut |e| {
        let arms: [Option<&Expr>; 2] = match e {
            Expr::Select { then, els, .. } => [Some(&**then), Some(&**els)],
            Expr::Binary {
                op: BinOp::LogicalAnd | BinOp::LogicalOr,
                r,
                ..
            } => [Some(&**r), None],
            _ => [None, None],
        };
        for arm in arms.into_iter().flatten() {
            guarded |= reads_in(arm, var) > 0;
        }
    });
    guarded
}

/// Where a temporary's address first goes to anyone but a load or a store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hand {
    /// Only loads and stores go through it, there or moved by a constant.
    Never,
    /// From this statement of a block no way comes back to, and in no other block.
    From(BlockId, usize),
    /// In more than one block, in a block on a way that comes back to it, or where nothing tells.
    Anywhere,
}

/// The temporaries whose address no one holds before some statement, for a value to be moved past what writes them there.
// A temporary's address handed to a call, kept in a variable or worked on is in other hands from
// there on; a load or a store through it, or through it moved by a constant, hands it to no one.
// Before the first statement that hands it on, in a block that runs at most once, nothing but the
// statements that name the temporary reads or writes it: a call or a load moved past a write of
// it cannot tell the write was there. With `escape` off, no temporary is taken so.
struct Unheld {
    on: bool,
    hands: HashMap<Var, Hand>,
}

/// The temporary `addr` is the address of, there or moved by a constant.
fn own_temporary(addr: &Expr) -> Option<Var> {
    let inner = match addr {
        Expr::ByteOffset { addr, offset } if matches!(**offset, Expr::Const(_)) => &**addr,
        other => other,
    };
    match inner {
        Expr::AddrOf { var, .. } if var.is_temporary() => Some(*var),
        _ => None,
    }
}

/// Whether `top` reads `var` or takes its address.
fn mentioned(top: &Expr, var: Var) -> bool {
    let mut found = false;
    top.walk(&mut |e| {
        found |= matches!(e, Expr::Var(named) | Expr::AddrOf { var: named, .. } if *named == var);
    });
    found
}

/// Which blocks lie on a way that comes back to them; a block the entry does not reach is taken to.
// Taken in reverse post-order, each block not yet gathered gathers the blocks that lead to it and
// that no earlier one gathered: those are the blocks it shares a cycle with.
fn circling(cfg: &cfg::Cfg) -> Vec<bool> {
    let n = cfg.blocks.len();
    let preds = cfg.predecessors();
    let mut part = vec![usize::MAX; n];
    let mut sizes: Vec<usize> = Vec::new();
    for root in cfg.reverse_post_order() {
        match part.get_mut(root) {
            Some(slot) if *slot == usize::MAX => *slot = sizes.len(),
            _ => continue,
        }
        let mut size = 0usize;
        let mut stack = vec![root];
        while let Some(x) = stack.pop() {
            size += 1;
            for p in preds.get(x).into_iter().flatten() {
                if let Some(slot) = part.get_mut(*p) {
                    if *slot == usize::MAX {
                        *slot = sizes.len();
                        stack.push(*p);
                    }
                }
            }
        }
        sizes.push(size);
    }
    (0..n)
        .map(|b| {
            let own = (cfg.blocks.get(b)).is_some_and(|block| block.term.successors().contains(&b));
            match part.get(b).and_then(|id| sizes.get(*id)) {
                Some(size) => *size > 1 || own,
                None => true,
            }
        })
        .collect()
}

impl Unheld {
    fn of(cfg: &cfg::Cfg, on: bool) -> Unheld {
        let mut hands: HashMap<Var, Hand> = HashMap::new();
        if !on {
            return Unheld { on, hands };
        }
        for (b, block) in cfg.blocks.iter().enumerate() {
            for k in 0..=block.stmts.len() {
                // Each address of a temporary the site names, and each one a load or a store uses.
                let stmt = block.stmts.get(k);
                let mut through: Vec<Var> = (stmt.and_then(stored))
                    .and_then(|(addr, ..)| own_temporary(addr))
                    .into_iter()
                    .collect();
                let mut named: Vec<Var> = Vec::new();
                let mut see = |top: &Expr| {
                    top.walk(&mut |e| match e {
                        Expr::AddrOf { var, .. } if var.is_temporary() => named.push(*var),
                        Expr::Load { addr, .. } | Expr::LoadBytes { addr, .. } => {
                            through.extend(own_temporary(addr));
                        }
                        _ => {}
                    });
                };
                match stmt {
                    Some(stmt) => stmt.each_expr(&mut see),
                    None => {
                        if let Some(e) = block.term.expr() {
                            see(e);
                        }
                    }
                }
                for var in &named {
                    let count = |vars: &[Var]| vars.iter().filter(|seen| *seen == var).count();
                    if count(&named) <= count(&through) {
                        continue;
                    }
                    let hand = hands.entry(*var).or_insert(Hand::Never);
                    *hand = match *hand {
                        Hand::Never => Hand::From(b, k),
                        Hand::From(at, first) if at == b => Hand::From(at, first),
                        _ => Hand::Anywhere,
                    };
                }
            }
        }
        let circles = circling(cfg);
        for hand in hands.values_mut() {
            if let Hand::From(b, _) = *hand {
                if circles.get(b).copied().unwrap_or(true) {
                    *hand = Hand::Anywhere;
                }
            }
        }
        Unheld { on, hands }
    }

    /// Whether `stmt`, at `site`, writes nothing but a temporary no one holds the address of before `until`, a later site of its block, and `value`, which does `moving`, may pass the rest of what it does.
    // ⚠ `value` must name the temporary nowhere: it would read what the statement writes.
    fn lets_pass(
        &self,
        facts: &Facts,
        stmt: &Stmt,
        (value, moving): (&Expr, &Effects),
        site: (BlockId, usize),
        until: usize,
    ) -> bool {
        if !self.on {
            return false;
        }
        let (var, rest): (Option<Var>, [Option<&Expr>; 2]) = match stmt {
            Stmt::Assign {
                target,
                value: given,
            } => (
                Some(*target).filter(|var| var.is_temporary()),
                [Some(given), None],
            ),
            Stmt::Store {
                addr, value: given, ..
            }
            | Stmt::StoreBytes {
                addr, value: given, ..
            } => (own_temporary(addr), [Some(addr), Some(given)]),
            Stmt::ZeroBytes { addr, .. } => (own_temporary(addr), [Some(addr), None]),
            _ => (None, [None, None]),
        };
        let Some(var) = var else {
            return false;
        };
        let unheld = match self.hands.get(&var).copied().unwrap_or(Hand::Never) {
            Hand::Never => true,
            Hand::From(b, from) => b == site.0 && from >= until,
            Hand::Anywhere => false,
        };
        if !unheld || mentioned(value, var) {
            return false;
        }
        let mut parts = rest.into_iter().flatten();
        let Some(first) = parts.next() else {
            return false;
        };
        let mut effects = Effects::of_expr(facts, first);
        for part in parts {
            part.walk(&mut |e| effects.node(facts, e));
        }
        moving.may_cross(&effects.placed(site))
    }
}

/// Whether a value that does `moving`, assigned by statement `i` of block `b`, may be evaluated at site `k` instead: past every statement between the two.
fn crosses(
    facts: &Facts,
    unheld: &Unheld,
    stmts: &[Option<Stmt>],
    effects: &mut [Option<Effects>],
    moving: &Effects,
    (b, i, k): (BlockId, usize, usize),
) -> bool {
    let value = match stmts.get(i) {
        Some(Some(Stmt::Assign { value, .. })) => Some(value),
        _ => None,
    };
    let mut passed = 0;
    for j in i + 1..k {
        let Some(Some(stmt)) = stmts.get(j) else {
            continue;
        };
        passed += 1;
        if passed > REACH {
            return false;
        }
        let Some(slot) = effects.get_mut(j) else {
            return false;
        };
        let crossed = slot.get_or_insert_with(|| Effects::of_stmt(facts, stmt).placed((b, j)));
        if moving.may_cross(crossed) {
            continue;
        }
        let apart =
            value.is_some_and(|value| unheld.lets_pass(facts, stmt, (value, moving), (b, j), k));
        if !apart {
            return false;
        }
    }
    true
}

/// Whether evaluating `e` calls or allocates, and whether it reads memory.
fn weight(e: &Expr) -> (bool, bool) {
    let (mut calls, mut loads) = (false, false);
    e.walk(&mut |node| match node {
        Expr::Call { .. } | Expr::NewObject(_) | Expr::NewArray { .. } => calls = true,
        Expr::Load { .. }
        | Expr::LoadBytes { .. }
        | Expr::VirtualFunctionRef { .. }
        | Expr::BuiltinGlobal(_)
        | Expr::Var(Var::Global { .. }) => loads = true,
        _ => {}
    });
    (calls, loads)
}

/// The variable a store goes through and its value first loads through, at one offset: `store(l, o, load(l, o) op r)` with no other read of `l`.
fn worked(stmt: &Stmt) -> Option<Var> {
    let Stmt::Store {
        addr: Expr::Var(var),
        offset,
        value: Expr::Binary { l, r, .. },
        ..
    } = stmt
    else {
        return None;
    };
    let mut read = &**l;
    while let Expr::Cast { e, .. } = read {
        read = &**e;
    }
    let Expr::Load {
        addr, offset: at, ..
    } = read
    else {
        return None;
    };
    let back = **addr == Expr::Var(*var) && at == offset;
    (back && reads_in(r, *var) == 0).then_some(*var)
}

/// Moves the address statement `i` of block `b` assigns to `var` into both places statement `k` goes through it, where evaluating it twice there gives what evaluating it once gave.
// `l = &x->f; store(l, 0, load(l, 0) | m);` is `store(&x->f, 0, load(&x->f, 0) | m);`: the
// address is evaluated first, and again at once for the load, with nothing between the two.
// ⚠ Only what `x->f |= m` evaluates once in C# is written twice: no call and no allocation in
// the address, and none in `m` where the address reads memory.
fn moved_twice(
    (facts, unheld): (&Facts, &Unheld),
    stmts: &mut [Option<Stmt>],
    effects: &mut [Option<Effects>],
    (b, i, var, k): (BlockId, usize, Var, usize),
) -> bool {
    let Some(Some(Stmt::Assign { value, .. })) = stmts.get(i) else {
        return false;
    };
    let Some(Some(Stmt::Store {
        value: stored @ Expr::Binary { r, .. },
        ..
    })) = stmts.get(k)
    else {
        return false;
    };
    let (calls, loads) = weight(value);
    let steady = !calls && !(loads && weight(r).0);
    let fits = depth(stored) + depth(value) <= DEPTH;
    let moving = Effects::of_expr(facts, value);
    let pure = !moving.call && !moving.alloc && !moving.store;
    if !steady || !fits || !pure || !crosses(facts, unheld, stmts, effects, &moving, (b, i, k)) {
        return false;
    }

    let Some(Stmt::Assign { value, .. }) = stmts[i].take() else {
        return false;
    };
    if let Some(Some(stmt)) = stmts.get_mut(k) {
        stmt.each_expr_mut(&mut |top| {
            top.walk_mut(&mut |e| {
                if matches!(e, Expr::Var(read) if *read == var) {
                    *e = value.clone();
                }
            });
        });
    }
    if let Some(stale) = effects.get_mut(k) {
        *stale = None;
    }
    true
}

/// The cells a value yields, where its own form states them.
fn stated(facts: &Facts, head: &Head, value: &Expr) -> Option<u32> {
    match value {
        Expr::Cells(items) if items.iter().all(|item| one_cell(head, item)) => {
            u32::try_from(items.len()).ok()
        }
        other => cells_of(other, &facts.widths),
    }
}

/// Whether the local `target` is one cell, or as many cells as `value` states it yields: read in its place, the value is what the local held.
fn fills(facts: &Facts, head: &Head, target: Var, value: &Expr) -> bool {
    match facts.widths.get(&target) {
        Some(cells) => stated(facts, head, value) == Some(*cells),
        None => (head.vars.get(&target)).is_none_or(|info| info.width != Width::Cells),
    }
}

/// Moves the value statement `i` of block `b` assigns to `var` into site `k`, the one place that reads it, where nothing between forbids.
fn moved(
    (facts, unheld): (&Facts, &Unheld),
    stmts: &mut [Option<Stmt>],
    term: &mut Terminator,
    effects: &mut [Option<Effects>],
    (b, i, var, k): (BlockId, usize, Var, usize),
) -> bool {
    let Some(Some(Stmt::Assign { value, .. })) = stmts.get(i) else {
        return false;
    };
    let moving = Effects::of_expr(facts, value);
    if !crosses(facts, unheld, stmts, effects, &moving, (b, i, k)) {
        return false;
    }

    // What the reading statement evaluates before it comes to the temporary is passed over too.
    let mut order = Vec::new();
    let mut deepest = 0;
    let mut skipped = false;
    site(stmts, term, k, &mut |top| {
        evaluated(top, &mut order);
        deepest = deepest.max(depth(top));
        skipped |= guarded(top, var);
    });
    let mut before = Effects::default();
    let mut found = false;
    for e in order {
        if matches!(e, Expr::Var(read) if *read == var) {
            found = true;
            break;
        }
        before.node(facts, e);
    }
    let fits = deepest + depth(value) <= DEPTH;
    // In an arm that may not run, only what could as well not be evaluated may sit.
    let placed = !skipped || moving.free();
    if !found || !fits || !placed || !moving.may_cross(&before) {
        return false;
    }

    let Some(Stmt::Assign { value, .. }) = stmts[i].take() else {
        return false;
    };
    let mut value = Some(value);
    site_mut(stmts, term, k, &mut |top| {
        top.walk_mut(&mut |e| {
            if matches!(e, Expr::Var(read) if *read == var) {
                if let Some(value) = value.take() {
                    *e = value;
                }
            }
        });
    });
    if let Some(stale) = effects.get_mut(k) {
        *stale = None;
    }
    true
}

/// Folds one block; `emptied` takes each local of several cells whose one assignment and one read are gone.
fn fold_block(
    (facts, unheld): (&Facts, &Unheld),
    head: &Head,
    b: BlockId,
    block: &mut Block,
    emptied: &mut Vec<Var>,
) -> bool {
    let mut stmts: Vec<Option<Stmt>> = std::mem::take(&mut block.stmts)
        .into_iter()
        .map(Some)
        .collect();
    let mut effects: Vec<Option<Effects>> = stmts.iter().map(|_| None).collect();
    // Where this block assigns each temporary, and each local kept like one, that is assigned once and read once.
    let mut assigned: HashMap<Var, usize> = HashMap::new();
    // Where it assigns each such variable of one cell that is read twice: by one statement, where [`worked`] finds both reads.
    let mut pointers: HashMap<Var, usize> = HashMap::new();
    let mut changed = false;
    for k in 0..=stmts.len() {
        let mut wanted: Vec<(usize, Var)> = Vec::new();
        site(&stmts, &block.term, k, &mut |top| {
            top.walk(&mut |e| {
                if let Expr::Var(var) = e {
                    if let Some(i) = assigned.get(var) {
                        wanted.push((*i, *var));
                    }
                }
            });
        });
        // The latest assignment first: once moved, it no longer stands between an earlier one and the reader.
        wanted.sort_unstable_by_key(|(i, _)| std::cmp::Reverse(*i));
        for (i, var) in wanted {
            if moved(
                (facts, unheld),
                &mut stmts,
                &mut block.term,
                &mut effects,
                (b, i, var, k),
            ) {
                assigned.remove(&var);
                if matches!(var, Var::Local(_)) && facts.widths.contains_key(&var) {
                    emptied.push(var);
                }
                changed = true;
            }
        }
        let through = match stmts.get(k) {
            Some(Some(stmt)) => worked(stmt),
            _ => None,
        };
        let held = through.and_then(|var| pointers.get(&var).map(|i| (*i, var)));
        if let Some((i, var)) = held {
            if moved_twice((facts, unheld), &mut stmts, &mut effects, (b, i, var, k)) {
                pointers.remove(&var);
                changed = true;
            }
        }
        if let Some(Some(Stmt::Assign { target, value })) = stmts.get(k) {
            let uses = facts.uses.get(target).copied().unwrap_or_default();
            let lone = uses.defs == 1 && uses.addrs == 0;
            // A local that no address reaches is a temporary the compiler kept in the frame: its
            // one read, found later in this block, takes its value the same way. One of several
            // cells is such a local where its value is those cells and no fewer.
            let kept = matches!(target, Var::Local(_))
                && facts.named_only(*target)
                && fills(facts, head, *target, value);
            let narrow = !facts.widths.contains_key(target)
                && (head.vars.get(target)).is_none_or(|info| info.width != Width::Cells);
            if (target.is_temporary() || kept) && lone {
                match uses.reads {
                    1 => {
                        assigned.insert(*target, k);
                    }
                    2 if narrow => {
                        pointers.insert(*target, k);
                    }
                    _ => {}
                }
            }
        }
    }
    block.stmts = stmts.into_iter().flatten().collect();
    changed
}

/// Moves the value of each temporary that is assigned once and read once into the expression that reads it; `finer` tells two accesses apart as [`Facts::finer`] does.
// With `finer`, a value also moves past what writes a temporary no one holds the address of yet
// (`Unheld`): `v = f(); t = default; store_i32(&t, 0, 5); store_bytes(&t, 4, 16, v);` is
// `t = default; store_i32(&t, 0, 5); store_bytes(&t, 4, 16, f());`.
pub(super) fn fold(f: &mut cfg::Function, finer: bool) -> bool {
    let facts = match finer {
        true => Facts::finer(f),
        false => Facts::of(f),
    };
    let unheld = Unheld::of(&f.cfg, finer);
    let head = &f.head;
    let mut emptied = Vec::new();
    let mut changed = false;
    for (b, block) in f.cfg.blocks.iter_mut().enumerate() {
        changed |= fold_block((&facts, &unheld), head, b, block, &mut emptied);
    }
    // A local of several cells that nothing names any more is no variable of the function.
    for var in emptied {
        f.head.vars.remove(&var);
    }
    changed
}

/// How deep the expressions of site `k` of a block nest, where the first thing the site evaluates is a read of `var`.
fn led_by(block: &Block, k: usize, var: Var) -> Option<usize> {
    let mut tops: Vec<&Expr> = Vec::new();
    match block.stmts.get(k) {
        Some(stmt) => stmt.each_expr(&mut |top| tops.push(top)),
        None => {
            if let Some(e) = block.term.expr() {
                tops.push(e);
            }
        }
    }
    let mut order = Vec::new();
    evaluated(tops.first().copied()?, &mut order);
    let first = order.first().copied();
    let leads = matches!(first, Some(Expr::Var(read)) if *read == var);
    leads.then(|| tops.iter().copied().map(depth).max().unwrap_or(0))
}

/// Lets the site after `v = E;` evaluate `E` itself, where reading the temporary `v` is the first thing it does and nothing else reads `v`.
// Nothing is weighed: `E` is evaluated after all that ran before it and before all that runs
// after it, as it was. For the pairs left by a pass that makes one variable of several cells
// once the last fold is over, when no effect is known of a variable such a pass made.
pub(in crate::gta6::script::passes) fn adjacent(f: &mut cfg::Function) -> bool {
    let facts = Facts::of(f);
    let mut changed = false;
    for block in &mut f.cfg.blocks {
        let mut i = 0;
        while i < block.stmts.len() {
            let lone = match &block.stmts[i] {
                Stmt::Assign { target, value } if target.is_temporary() => {
                    let once = (facts.uses.get(target))
                        .is_some_and(|uses| uses.defs == 1 && uses.reads == 1 && uses.addrs == 0);
                    (once && reads_in(value, *target) == 0).then(|| (*target, depth(value)))
                }
                _ => None,
            };
            let fits = match lone {
                Some((var, deep)) => {
                    led_by(block, i + 1, var).is_some_and(|reader| reader + deep <= DEPTH)
                }
                None => false,
            };
            if !fits {
                i += 1;
                continue;
            }
            let Stmt::Assign { target: var, value } = block.stmts.remove(i) else {
                i += 1;
                continue;
            };
            // With statement `i` gone, its reader stands at `i`, or is the terminator.
            let mut value = Some(value);
            let mut put = |top: &mut Expr| {
                top.walk_mut(&mut |e| {
                    if matches!(e, Expr::Var(read) if *read == var) {
                        if let Some(value) = value.take() {
                            *e = value;
                        }
                    }
                });
            };
            match block.stmts.get_mut(i) {
                Some(stmt) => stmt.each_expr_mut(&mut put),
                None => {
                    if let Some(e) = block.term.expr_mut() {
                        put(e);
                    }
                }
            }
            changed = true;
        }
    }
    changed
}

/// Where a statement stands: its block and its place there, the terminator past the last statement.
type Site = (BlockId, usize);

/// Lets each temporary, and each local kept like one, that only holds a constant or a copy of a variable be that, where the variable cannot change between the copy and any read of the copy.
// `t = s;` with `s` assigned at most once and reached by no address. Where that one assignment
// comes before the copy on every path, and the copy before every read of `t`, no path leads from
// the assignment to a read without passing the copy again: `t` and `s` are equal at every read.
// With `keep`, a local that copies a temporary keeps its name: by the same proof the
// temporary's one assignment can assign the local, which nothing reads before the copy.
pub(super) fn copies(f: &mut cfg::Function, keep: bool) -> bool {
    let facts = Facts::of(f);
    let uses = |var: Var| facts.uses.get(&var).copied().unwrap_or_default();
    let one_cell = |var: Var| {
        let many = (f.head.vars.get(&var)).is_some_and(|info| info.width == Width::Cells);
        !many && !facts.widths.contains_key(&var)
    };
    // Two variables one value fits: one cell each, or as many cells each as something states.
    let alike = |a: Var, b: Var| {
        let wide = facts.widths.contains_key(&a) && facts.widths.get(&a) == facts.widths.get(&b);
        (one_cell(a) && one_cell(b)) || wide
    };
    // The one assignment of each variable assigned once.
    let mut assigned: HashMap<Var, Site> = HashMap::new();
    let mut held: HashMap<Var, Expr> = HashMap::new();
    for (b, block) in f.cfg.blocks.iter().enumerate() {
        for (i, stmt) in block.stmts.iter().enumerate() {
            for target in stmt.targets() {
                if uses(*target).defs == 1 {
                    assigned.insert(*target, (b, i));
                }
            }
            let Stmt::Assign { target, value } = stmt else {
                continue;
            };
            let copy = uses(*target);
            // A local no address reaches is held to the same proof: assigned once, before every read.
            let kept = matches!(target, Var::Local(_)) && facts.named_only(*target);
            let lone = (target.is_temporary() || kept) && copy.defs == 1 && copy.addrs == 0;
            let fixed = match value {
                // The address of a variable is the same wherever it is read.
                Expr::AddrOf { .. } => one_cell(*target),
                Expr::Var(source) => {
                    let named = !matches!(source, Var::Global { .. }) && facts.named_only(*source);
                    named && uses(*source).defs <= 1 && alike(*target, *source)
                }
                // So is a constant, and what operators make of constants.
                other => constant(other) && one_cell(*target),
            };
            if lone && fixed {
                held.insert(*target, value.clone());
            }
        }
    }
    // A copy of a copy waits for the round after this one.
    let chained: Vec<Var> = (held.iter())
        .filter(|(_, source)| matches!(source, Expr::Var(source) if held.contains_key(source)))
        .map(|(target, _)| *target)
        .collect();
    for target in chained {
        held.remove(&target);
    }
    if held.is_empty() {
        return false;
    }

    let idom = f.cfg.dominators();
    let before = |a: Site, b: Site| match a.0 == b.0 {
        true => a.1 < b.1,
        false => dominates(&idom, a.0, b.0),
    };
    held.retain(|target, source| {
        let Some(copy) = assigned.get(target).copied() else {
            return false;
        };
        let Expr::Var(source) = source else {
            return true;
        };
        match assigned.get(source).copied() {
            Some(set) => before(set, copy),
            None => uses(*source).defs == 0,
        }
    });
    let mut late: Vec<Var> = Vec::new();
    for (b, block) in f.cfg.blocks.iter().enumerate() {
        for k in 0..=block.stmts.len() {
            let mut see = |top: &Expr| {
                top.walk(&mut |e| {
                    let Expr::Var(read) = e else {
                        return;
                    };
                    let copy = (held.get(read)).and_then(|_| assigned.get(read).copied());
                    if copy.is_some_and(|copy| !before(copy, (b, k))) {
                        late.push(*read);
                    }
                });
            };
            match block.stmts.get(k) {
                Some(stmt) => stmt.each_expr(&mut see),
                None => {
                    if let Some(e) = block.term.expr() {
                        see(e);
                    }
                }
            }
        }
    }
    for target in late {
        held.remove(&target);
    }
    // The temporaries that give their one assignment to the local they are copied into.
    // In the order of the blocks, so that of two locals copied from one temporary the same one is chosen every time.
    let mut renamed: HashMap<Var, Var> = HashMap::new();
    if keep {
        for stmt in f.cfg.blocks.iter().flat_map(|block| &block.stmts) {
            let Stmt::Assign {
                target: local @ Var::Local(_),
                value: Expr::Var(source),
            } = stmt
            else {
                continue;
            };
            let lone = source.is_temporary() && uses(*source).defs == 1;
            let copied = held.get(local) == Some(&Expr::Var(*source));
            if lone && copied && !renamed.contains_key(source) {
                renamed.insert(*source, *local);
            }
        }
        for local in renamed.values() {
            held.remove(local);
        }
        // Every other copy of such a temporary is a copy of the local it became.
        for source in held.values_mut() {
            let local = match &*source {
                Expr::Var(var) => renamed.get(var).copied(),
                _ => None,
            };
            if let Some(local) = local {
                *source = Expr::Var(local);
            }
        }
    }
    if held.is_empty() && renamed.is_empty() {
        return false;
    }

    for block in &mut f.cfg.blocks {
        block.stmts.retain(|stmt| match stmt {
            Stmt::Assign {
                target,
                value: Expr::Var(source),
            } if renamed.get(source) == Some(target) => false,
            Stmt::Assign { target, .. } => !held.contains_key(target),
            _ => true,
        });
        for stmt in &mut block.stmts {
            let targets: &mut [Var] = match stmt {
                Stmt::Assign { target, .. } => std::slice::from_mut(target),
                Stmt::AssignMany { targets, .. } => &mut targets[..],
                _ => &mut [],
            };
            for target in targets {
                if let Some(local) = renamed.get(target) {
                    *target = *local;
                }
            }
        }
        let mut rename = |top: &mut Expr| {
            top.walk_mut(&mut |e| {
                let source = match &*e {
                    Expr::Var(read) => (held.get(read).cloned())
                        .or_else(|| renamed.get(read).map(|local| Expr::Var(*local))),
                    _ => None,
                };
                if let Some(source) = source {
                    *e = source;
                }
            });
        };
        for stmt in &mut block.stmts {
            stmt.each_expr_mut(&mut rename);
        }
        if let Some(e) = block.term.expr_mut() {
            rename(e);
        }
    }
    true
}

/// Whether `e` is the integer one as a 32-bit operand.
fn one(e: &Expr) -> bool {
    match e {
        Expr::Const(c) => c.kind == Kind::I32 && c.bits == 1,
        Expr::Cast { to: Scalar::I32, e } => matches!(&**e, Expr::Const(c) if c.bits as u32 == 1),
        _ => false,
    }
}

/// Whether a store writes back the 32 bits it loads from the same place, one more or one less: `Some(true)` for less.
fn stepped(stmt: &Stmt) -> Option<bool> {
    let Stmt::Store {
        kind: StoreKind::I32,
        addr,
        offset,
        value:
            Expr::Binary {
                op,
                at: Kind::I32,
                l,
                r,
            },
    } = stmt
    else {
        return None;
    };
    let down = match op {
        BinOp::Add => false,
        BinOp::Sub => true,
        _ => return None,
    };
    let Expr::Cast {
        to: Scalar::I32,
        e: loaded,
    } = &**l
    else {
        return None;
    };
    let Expr::Load {
        ty: Scalar::I32,
        addr: from,
        offset: at,
    } = &**loaded
    else {
        return None;
    };
    // The address is evaluated once where it was evaluated twice.
    let same = **from == *addr && at == offset && inert(addr);
    (same && one(r)).then_some(down)
}

/// Writes a step by one the way the reader writes `x++` and `increment_i32`, for the printer to know it by.
pub(super) fn steps(f: &mut cfg::Function) {
    for block in &mut f.cfg.blocks {
        for stmt in &mut block.stmts {
            if let Some(down) = stepped(stmt) {
                let store = std::mem::replace(stmt, Stmt::Eval(Expr::GuidEmpty));
                if let Stmt::Store { addr, offset, .. } = store {
                    *stmt = Stmt::StepI32 { addr, offset, down };
                }
                continue;
            }
            let Stmt::Assign {
                target,
                value:
                    Expr::Binary {
                        op: BinOp::Add | BinOp::Sub,
                        at: Kind::I32,
                        l,
                        r,
                    },
            } = stmt
            else {
                continue;
            };
            let own =
                matches!(&**l, Expr::Cast { to: Scalar::I32, e } if **e == Expr::Var(*target));
            if own && one(r) {
                **r = Expr::Const(Const::i32(1));
            }
        }
    }
}

#[cfg(test)]
#[path = "values_fold_tests.rs"]
mod tests;
