//! Temporaries put back into the expressions they were taken out of, and assignments nothing reads removed.

use std::collections::{HashMap, HashSet};

use crate::gta6::script::cfg::{self, dominates, Block, BlockId, Terminator};
use crate::gta6::script::ir::{
    BinOp, Const, Expr, Head, Kind, Scalar, Stmt, StoreKind, Var, Width,
};

use super::effects::{depth, evaluated, inert, Effects, Facts};

/// How many statements may stand between a value's assignment and the place it moves to.
// ponytail: a fixed reach keeps a block of thousands of statements linear; a running summary of
// the effects between two statements would lift it.
const REACH: usize = 256;

/// The deepest an expression is let grow, so that walking one never runs out of stack.
const DEPTH: usize = 48;

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
fn leftover(facts: &Facts, value: Expr) -> Result<Option<Expr>, Expr> {
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

/// Moves the value statement `i` assigns to `var` into site `k`, the one place that reads it, where nothing between forbids.
fn moved(
    facts: &Facts,
    stmts: &mut [Option<Stmt>],
    term: &mut Terminator,
    effects: &mut [Option<Effects>],
    (i, var, k): (usize, Var, usize),
) -> bool {
    let Some(Some(Stmt::Assign { value, .. })) = stmts.get(i) else {
        return false;
    };
    let moving = Effects::of_expr(facts, value);
    let mut passed = 0;
    for j in i + 1..k {
        let Some(stmt) = &stmts[j] else {
            continue;
        };
        passed += 1;
        if passed > REACH {
            return false;
        }
        let crossed = effects[j].get_or_insert_with(|| Effects::of_stmt(facts, stmt));
        if !moving.may_cross(crossed) {
            return false;
        }
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

fn fold_block(facts: &Facts, block: &mut Block) -> bool {
    let mut stmts: Vec<Option<Stmt>> = std::mem::take(&mut block.stmts)
        .into_iter()
        .map(Some)
        .collect();
    let mut effects: Vec<Option<Effects>> = stmts.iter().map(|_| None).collect();
    // Where this block assigns each temporary, and each local kept like one, that is assigned once and read once.
    let mut assigned: HashMap<Var, usize> = HashMap::new();
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
                facts,
                &mut stmts,
                &mut block.term,
                &mut effects,
                (i, var, k),
            ) {
                assigned.remove(&var);
                changed = true;
            }
        }
        if let Some(Some(Stmt::Assign { target, .. })) = stmts.get(k) {
            let once = facts
                .uses
                .get(target)
                .is_some_and(|uses| uses.defs == 1 && uses.reads == 1 && uses.addrs == 0);
            // A local of one cell that no address reaches is a temporary the compiler kept in the
            // frame: its one read, found later in this block, takes its value the same way.
            let kept = matches!(target, Var::Local(_))
                && facts.named_only(*target)
                && !facts.widths.contains_key(target);
            if (target.is_temporary() || kept) && once {
                assigned.insert(*target, k);
            }
        }
    }
    block.stmts = stmts.into_iter().flatten().collect();
    changed
}

/// Moves the value of each temporary that is assigned once and read once into the expression that reads it.
pub(super) fn fold(f: &mut cfg::Function) -> bool {
    let facts = Facts::of(f);
    let mut changed = false;
    for block in &mut f.cfg.blocks {
        changed |= fold_block(&facts, block);
    }
    changed
}

/// Where a statement stands: its block and its place there, the terminator past the last statement.
type Site = (BlockId, usize);

/// Lets each temporary, and each local kept like one, that only holds a constant or a copy of a variable be that, where the variable cannot change between the copy and any read of the copy.
// `t = s;` with `s` assigned at most once and reached by no address. Where that one assignment
// comes before the copy on every path, and the copy before every read of `t`, no path leads from
// the assignment to a read without passing the copy again: `t` and `s` are equal at every read.
pub(super) fn copies(f: &mut cfg::Function) -> bool {
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
                // A constant is the same wherever it is read, and so is the address of a variable.
                Expr::Const(_) | Expr::AddrOf { .. } => one_cell(*target),
                Expr::Var(source) => {
                    let named = !matches!(source, Var::Global { .. }) && facts.named_only(*source);
                    named && uses(*source).defs <= 1 && alike(*target, *source)
                }
                _ => false,
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
    if held.is_empty() {
        return false;
    }

    for block in &mut f.cfg.blocks {
        block.stmts.retain(
            |stmt| !matches!(stmt, Stmt::Assign { target, .. } if held.contains_key(target)),
        );
        let mut rename = |top: &mut Expr| {
            top.walk_mut(&mut |e| {
                let source = match &*e {
                    Expr::Var(read) => held.get(read).cloned(),
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
