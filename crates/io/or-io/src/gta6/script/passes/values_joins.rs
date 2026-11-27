//! The values a lifter carries across a join in `stack_` variables: tuples taken apart, each
//! variable that only renames another value replaced by that value, a join that is only
//! copied into the next one merged with it or with a local, two arms that only choose a value
//! written as the choice, and a value of several cells carried in a row of temporaries made one variable.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::gta6::script::cfg::{self, Block, BlockId, Cfg, Terminator, ENTRY};
use crate::gta6::script::ir::{Expr, Head, Kind, Stmt, UnOp, Var, VarInfo, Width};

use super::effects::{depth, each_top_mut, one_cell, Effects, Facts};

/// The sweeps given to settling what each stack variable carries; a chain of renames longer than this that runs against the order of the blocks is left alone.
const SWEEPS: usize = 32;

/// The sweeps given to merging joins: each merges one more link of a chain of them.
const MERGES: usize = 16;

/// The deepest a test or a value may nest and still be written into a choice between two values.
const CHOICE_DEPTH: usize = 24;

/// The statements of a join that are looked at for copies of the value two arms hand it.
const JOIN_COPIES: usize = 64;

/// The most cells a value carried in temporaries of one cell each is made one variable of.
const WHOLE: u32 = 256;

/// The statements a function's walks along its paths may look at before the rest is left as it is.
fn allowance(cfg: &Cfg) -> usize {
    64 * (cfg.statements() + cfg.blocks.len()) + 4096
}

fn reads_in(e: &Expr, var: Var) -> bool {
    let mut read = false;
    e.walk(&mut |node| read |= matches!(node, Expr::Var(named) if *named == var));
    read
}

fn reads(stmt: &Stmt, var: Var) -> bool {
    let mut read = false;
    stmt.each_expr(&mut |e| read |= reads_in(e, var));
    read
}

/// Whether a tuple's targets are stack variables, each named once, that none of its values reads.
// A stack variable has no address, so only a value that names one can see it change.
fn apart(targets: &[Var], values: &[Expr]) -> bool {
    let stack = targets.iter().all(|t| matches!(t, Var::Stack { .. }));
    let once = targets
        .iter()
        .enumerate()
        .all(|(i, target)| !targets[..i].contains(target));
    let mut read = false;
    for value in values {
        value.walk(&mut |e| {
            if let Expr::Var(var) | Expr::AddrOf { var, .. } = e {
                read |= targets.contains(var);
            }
        });
    }
    stack && once && !read
}

/// Writes a tuple assignment as one assignment after another where that gives every target the same value.
pub(super) fn split_tuples(cfg: &mut Cfg) {
    for block in &mut cfg.blocks {
        if !block
            .stmts
            .iter()
            .any(|stmt| matches!(stmt, Stmt::AssignMany { .. }))
        {
            continue;
        }
        let old = std::mem::take(&mut block.stmts);
        for stmt in old {
            match stmt {
                Stmt::AssignMany { targets, values } if apart(&targets, &values) => {
                    let pairs = targets.into_iter().zip(values);
                    block
                        .stmts
                        .extend(pairs.map(|(target, value)| Stmt::Assign { target, value }));
                }
                other => block.stmts.push(other),
            }
        }
    }
}

/// What the assignments to a stack variable give it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Root {
    /// Nothing says yet: every assignment looked at copies a variable that is itself not settled.
    Unknown,
    /// This constant or variable, whichever assignment ran.
    Is(Expr),
    /// Values that differ: the variable is the one the join needs.
    Own,
}

/// Whether `value` is a constant, or one cell of a variable that only an assignment naming it can change.
fn atom(facts: &Facts, head: &Head, value: &Expr) -> bool {
    match value {
        Expr::Const(_) => true,
        Expr::Var(var) => {
            let many = facts.widths.contains_key(var)
                || head
                    .vars
                    .get(var)
                    .is_some_and(|info| info.width == Width::Cells);
            !many && facts.named_only(*var)
        }
        _ => false,
    }
}

/// What a walk back from a read needs: the blocks, what leads into each, and what each stack variable carries.
struct Paths<'a> {
    cfg: &'a Cfg,
    preds: &'a [Vec<BlockId>],
    found: &'a HashMap<Var, Root>,
}

impl Paths<'_> {
    /// Whether `var`, read at site `k` of block `b`, holds what `of` holds there.
    // Back along every path from the read, the nearest assignments must lead through renames to
    // one of `of` itself, with nothing assigned to `of` on the way and the function's start never
    // reached. `budget` bounds the statements looked at; running out answers no.
    fn holds(&self, var: Var, of: &Expr, (b, k): (BlockId, usize), budget: &mut usize) -> bool {
        let guard = match of {
            Expr::Var(guard) => Some(*guard),
            _ => None,
        };
        let mut seen: HashSet<(BlockId, Var)> = HashSet::new();
        let mut work = vec![(b, k, var)];
        while let Some((b, upto, mut wanted)) = work.pop() {
            let stmts = &self.cfg.blocks[b].stmts;
            let mut met = false;
            for stmt in stmts[..upto.min(stmts.len())].iter().rev() {
                if *budget == 0 {
                    return false;
                }
                *budget -= 1;
                if let Stmt::Assign { target, value } = stmt {
                    if *target == wanted {
                        if value == of {
                            met = true;
                            break;
                        }
                        match value {
                            Expr::Var(from) if self.carries(*from, of) => {
                                wanted = *from;
                                continue;
                            }
                            _ => return false,
                        }
                    }
                }
                if guard.is_some_and(|guard| stmt.targets().contains(&guard)) {
                    return false;
                }
            }
            if met {
                continue;
            }
            if b == ENTRY {
                return false;
            }
            for p in &self.preds[b] {
                if seen.insert((*p, wanted)) {
                    work.push((*p, usize::MAX, wanted));
                }
            }
        }
        true
    }

    fn carries(&self, var: Var, of: &Expr) -> bool {
        matches!(self.found.get(&var), Some(Root::Is(carried)) if carried == of)
    }
}

/// The one thing each stack variable carries, for those every assignment gives a constant or a plain variable.
fn settle(sources: &BTreeMap<Var, Vec<Expr>>) -> Option<HashMap<Var, Root>> {
    let mut found: HashMap<Var, Root> = HashMap::new();
    for _ in 0..SWEEPS {
        let mut moved = false;
        for (var, values) in sources {
            let mut root = Root::Unknown;
            for value in values {
                let carried = match value {
                    // A variable copied from itself, at once or around a loop, says nothing new.
                    Expr::Var(from) if from == var => continue,
                    Expr::Var(from) if sources.contains_key(from) => match found.get(from) {
                        Some(Root::Is(of)) if *of == Expr::Var(*var) => continue,
                        Some(Root::Is(of)) => of.clone(),
                        Some(Root::Own) => value.clone(),
                        Some(Root::Unknown) | None => continue,
                    },
                    other => other.clone(),
                };
                root = match root {
                    Root::Unknown => Root::Is(carried),
                    Root::Is(of) if of == carried => Root::Is(of),
                    _ => Root::Own,
                };
            }
            if found.get(var) != Some(&root) {
                found.insert(*var, root);
                moved = true;
            }
        }
        if !moved {
            return Some(found);
        }
    }
    None
}

/// Replaces each stack variable that only ever carries one constant or one variable by that, and removes its assignments.
pub(super) fn roots(f: &mut cfg::Function) -> bool {
    let facts = Facts::of(f);
    let mut sources: BTreeMap<Var, Vec<Expr>> = BTreeMap::new();
    let mut varied: HashSet<Var> = HashSet::new();
    for block in &f.cfg.blocks {
        for stmt in &block.stmts {
            match stmt {
                Stmt::Assign { target, value }
                    if matches!(target, Var::Stack { .. }) && atom(&facts, &f.head, value) =>
                {
                    sources.entry(*target).or_default().push(value.clone());
                }
                other => varied.extend(other.targets().iter().copied()),
            }
        }
    }
    sources.retain(|var, _| !varied.contains(var));
    let Some(found) = settle(&sources) else {
        return false;
    };
    if !found.values().any(|root| matches!(root, Root::Is(_))) {
        return false;
    }

    let preds = f.cfg.predecessors();
    let paths = Paths {
        cfg: &f.cfg,
        preds: &preds,
        found: &found,
    };
    let mut budget = allowance(&f.cfg);
    let mut failed: HashSet<Var> = HashSet::new();
    for (b, block) in f.cfg.blocks.iter().enumerate() {
        for k in 0..=block.stmts.len() {
            let mut read: Vec<Var> = Vec::new();
            let mut see = |top: &Expr| {
                top.walk(&mut |e| {
                    if let Expr::Var(var) = e {
                        if matches!(found.get(var), Some(Root::Is(_))) {
                            read.push(*var);
                        }
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
            for var in read {
                let Some(Root::Is(of)) = found.get(&var) else {
                    continue;
                };
                if !failed.contains(&var) && !paths.holds(var, of, (b, k), &mut budget) {
                    failed.insert(var);
                }
            }
        }
    }

    let replaced: HashMap<Var, Expr> = found
        .into_iter()
        .filter_map(|(var, root)| match root {
            Root::Is(of) if !failed.contains(&var) => Some((var, of)),
            _ => None,
        })
        .collect();
    if replaced.is_empty() {
        return false;
    }
    for block in &mut f.cfg.blocks {
        block.stmts.retain(
            |stmt| !matches!(stmt, Stmt::Assign { target, .. } if replaced.contains_key(target)),
        );
    }
    each_top_mut(&mut f.cfg, &mut |top| {
        top.walk_mut(&mut |e| {
            let of = match &*e {
                Expr::Var(var) => replaced.get(var).cloned(),
                _ => None,
            };
            if let Some(of) = of {
                *e = of;
            }
        });
    });
    true
}

/// A copy `t = s;` that is the one reader of the stack variable `s`.
struct Link {
    s: Var,
    t: Var,
    /// The block of the copy and its place there.
    at: (BlockId, usize),
}

/// Whether assigning `link.t` at `def`, where `link.s` is assigned, changes nothing a later statement sees.
// From the assignment on, along every path: nothing reads `t` while it still holds what the
// assignment gave it, and the copy is not reached after `t` was assigned anew. The copy, the next
// assignment to `s` and the function's end close a path.
fn carried(cfg: &Cfg, link: &Link, def: (BlockId, usize), budget: &mut usize) -> bool {
    // Each block as reached with `t` untouched, and as reached with `t` assigned anew.
    let mut seen = vec![[false; 2]; cfg.blocks.len()];
    let mut work = vec![(def.0, def.1 + 1, false)];
    while let Some((b, from, mut anew)) = work.pop() {
        let block = &cfg.blocks[b];
        let mut closed = false;
        for (i, stmt) in block.stmts.iter().enumerate().skip(from) {
            if *budget == 0 {
                return false;
            }
            *budget -= 1;
            if (b, i) == link.at {
                if anew {
                    return false;
                }
                closed = true;
                break;
            }
            if !anew && reads(stmt, link.t) {
                return false;
            }
            if stmt.targets().contains(&link.s) {
                closed = true;
                break;
            }
            anew |= stmt.targets().contains(&link.t);
        }
        if closed {
            continue;
        }
        if !anew && block.term.expr().is_some_and(|e| reads_in(e, link.t)) {
            return false;
        }
        for next in block.term.successors() {
            if !std::mem::replace(&mut seen[next][usize::from(anew)], true) {
                work.push((next, 0, anew));
            }
        }
    }
    true
}

/// Whether every path to the copy passes an assignment to `link.s`.
fn reached(cfg: &Cfg, preds: &[Vec<BlockId>], link: &Link, budget: &mut usize) -> bool {
    let mut seen = vec![false; cfg.blocks.len()];
    let mut work = vec![link.at];
    while let Some((b, upto)) = work.pop() {
        let stmts = &cfg.blocks[b].stmts;
        let mut met = false;
        for stmt in stmts[..upto.min(stmts.len())].iter().rev() {
            if *budget == 0 {
                return false;
            }
            *budget -= 1;
            if stmt.targets().contains(&link.s) {
                met = true;
                break;
            }
        }
        if met {
            continue;
        }
        if b == ENTRY {
            return false;
        }
        for p in &preds[b] {
            if !std::mem::replace(&mut seen[*p], true) {
                work.push((*p, usize::MAX));
            }
        }
    }
    true
}

/// Lets every assignment to a stack variable that is only ever copied into another, or into a local, assign that one, and removes the copy.
pub(super) fn merge(f: &mut cfg::Function) -> bool {
    let mut changed = false;
    for _ in 0..MERGES {
        let facts = Facts::of(f);
        // Where each stack variable is assigned by a statement of its own.
        let mut assigned: HashMap<Var, Vec<(BlockId, usize)>> = HashMap::new();
        let mut links: Vec<Link> = Vec::new();
        // ⚠ A copy of a value of several cells renames no assignment: the variable it would
        // rename to is declared a cell wide.
        let wide = |var: &Var| {
            facts.widths.contains_key(var)
                || (f.head.vars.get(var)).is_some_and(|info| info.width == Width::Cells)
        };
        for (b, block) in f.cfg.blocks.iter().enumerate() {
            for (i, stmt) in block.stmts.iter().enumerate() {
                let Stmt::Assign { target, value } = stmt else {
                    continue;
                };
                // A local that only its own name reads and writes takes a join's value the same way.
                let kept = matches!(target, Var::Local(_))
                    && facts.named_only(*target)
                    && !facts.widths.contains_key(target);
                if matches!(target, Var::Stack { .. }) {
                    assigned.entry(*target).or_default().push((b, i));
                } else if !kept {
                    continue;
                }
                match value {
                    Expr::Var(from)
                        if matches!(from, Var::Stack { .. })
                            && from != target
                            && !wide(from)
                            && !wide(target) =>
                    {
                        links.push(Link {
                            s: *from,
                            t: *target,
                            at: (b, i),
                        });
                    }
                    _ => {}
                }
            }
        }

        let preds = f.cfg.predecessors();
        let mut budget = allowance(&f.cfg);
        // Merges that share no variable hold together, each as it was found on the code unchanged.
        let mut taken: HashSet<Var> = HashSet::new();
        let mut merged: Vec<Link> = Vec::new();
        for link in links {
            let Some(defs) = assigned.get(&link.s) else {
                continue;
            };
            let uses = facts.uses.get(&link.s).copied().unwrap_or_default();
            let alone = uses.reads == 1 && uses.addrs == 0 && uses.defs as usize == defs.len();
            if !alone || taken.contains(&link.s) || taken.contains(&link.t) {
                continue;
            }
            let clear = defs
                .iter()
                .all(|def| carried(&f.cfg, &link, *def, &mut budget))
                && reached(&f.cfg, &preds, &link, &mut budget);
            if clear {
                taken.insert(link.s);
                taken.insert(link.t);
                merged.push(link);
            }
        }
        if merged.is_empty() {
            break;
        }

        // The copies go from the last back, so that the places before stay where they were found.
        let mut copies: Vec<(BlockId, usize)> = merged.iter().map(|link| link.at).collect();
        copies.sort_unstable_by_key(|at| std::cmp::Reverse(*at));
        for (b, i) in copies {
            f.cfg.blocks[b].stmts.remove(i);
        }
        let renamed: HashMap<Var, Var> = merged.iter().map(|link| (link.s, link.t)).collect();
        for block in &mut f.cfg.blocks {
            for stmt in &mut block.stmts {
                let Stmt::Assign { target, .. } = stmt else {
                    continue;
                };
                let old = *target;
                if let Some(to) = renamed.get(&old) {
                    *target = *to;
                }
            }
        }
        changed = true;
    }
    changed
}

/// Whether `stmt` copies one of `a` and `b` into the other.
fn copy_between(stmt: &Stmt, a: Var, b: Var) -> bool {
    match stmt {
        Stmt::Assign {
            target,
            value: Expr::Var(from),
        } => (*target, *from) == (a, b) || (*target, *from) == (b, a),
        _ => false,
    }
}

/// What the walks of [`united`] read of a function: the blocks that lead into each block, and the blocks that name each variable looked at.
struct Graph {
    preds: Vec<Vec<BlockId>>,
    named: HashMap<Var, Vec<BlockId>>,
}

impl Graph {
    /// The graph of `cfg`, with the blocks that read or assign each of `vars` by name.
    fn of(cfg: &Cfg, vars: &HashSet<Var>) -> Graph {
        let mut preds: Vec<Vec<BlockId>> = vec![Vec::new(); cfg.blocks.len()];
        let mut named: HashMap<Var, Vec<BlockId>> = HashMap::new();
        for (b, block) in cfg.blocks.iter().enumerate() {
            for next in block.term.successors() {
                if let Some(into) = preds.get_mut(next) {
                    into.push(b);
                }
            }
            let mut note = |var: Var| {
                if vars.contains(&var) {
                    let blocks = named.entry(var).or_default();
                    if blocks.last() != Some(&b) {
                        blocks.push(b);
                    }
                }
            };
            for stmt in &block.stmts {
                for target in stmt.targets() {
                    note(*target);
                }
                stmt.each_expr(&mut |top| {
                    top.walk(&mut |e| {
                        if let Expr::Var(var) = e {
                            note(*var);
                        }
                    });
                });
            }
            if let Some(top) = block.term.expr() {
                top.walk(&mut |e| {
                    if let Expr::Var(var) = e {
                        note(*var);
                    }
                });
            }
        }
        Graph { preds, named }
    }

    fn named(&self, var: Var) -> &[BlockId] {
        match self.named.get(&var) {
            Some(blocks) => blocks,
            None => &[],
        }
    }
}

/// Where `var` is live as each block starts and as it ends: some path from there reads it before it is assigned; `None` past `budget`.
fn liveness(
    cfg: &Cfg,
    graph: &Graph,
    var: Var,
    budget: &mut usize,
) -> Option<(Vec<bool>, Vec<bool>)> {
    let n = cfg.blocks.len();
    *budget = budget.checked_sub(n)?;
    let mut read = vec![false; n];
    let mut assigned = vec![false; n];
    // Only a block that names the variable reads or assigns it.
    for b in graph.named(var).iter().copied() {
        let Some(block) = cfg.blocks.get(b) else {
            continue;
        };
        *budget = budget.checked_sub(block.stmts.len() + 1)?;
        // A statement reads what it reads before it assigns its targets.
        for stmt in &block.stmts {
            read[b] |= !assigned[b] && reads(stmt, var);
            assigned[b] |= stmt.targets().contains(&var);
        }
        read[b] |= !assigned[b] && block.term.expr().is_some_and(|e| reads_in(e, var));
    }
    // Each block turns live once at most, so the work is bounded by the edges.
    let mut live_in = read;
    let mut work: Vec<BlockId> = (0..n).filter(|b| live_in[*b]).collect();
    while let Some(b) = work.pop() {
        for p in graph.preds.get(b).into_iter().flatten() {
            if !assigned[*p] && !live_in[*p] {
                live_in[*p] = true;
                work.push(*p);
            }
        }
    }
    let live_out = (cfg.blocks.iter())
        .map(|block| {
            let next = block.term.successors();
            next.into_iter()
                .any(|b| live_in.get(b).copied().unwrap_or(false))
        })
        .collect();
    Some((live_in, live_out))
}

/// Whether the stack variable `s` and the local `t` may be one variable: wherever either is assigned but by a copy of the other, the other is not read again before it is assigned anew, and `s` is never read before it is assigned.
// With `exposed`, a pointer may reach `t`: then nothing that runs while `s` holds a value still
// to be read may call, load or store through a pointer, or name `t`, for `t` would hold that
// value there too. The statement that assigns `s` runs what it runs before the assignment.
fn united(
    cfg: &Cfg,
    graph: &Graph,
    facts: &Facts,
    (s, t): (Var, Var),
    exposed: bool,
    budget: &mut usize,
) -> bool {
    let Some((into_s, out_s)) = liveness(cfg, graph, s, budget) else {
        return false;
    };
    let Some((_, out_t)) = liveness(cfg, graph, t, budget) else {
        return false;
    };
    if into_s.get(ENTRY).copied().unwrap_or(true) {
        return false;
    }
    let held = facts.place(t);
    let quiet = |effects: Effects| {
        let named = held.is_some_and(|place| effects.touches(place));
        !(effects.call || effects.store || effects.load || named)
    };
    // A block that names neither, and that `s` is not live out of, assigns neither while the other is still to be read and runs nothing while `s` is.
    let mut looked = vec![false; cfg.blocks.len()];
    for b in graph.named(s).iter().chain(graph.named(t)).copied() {
        if let Some(seen) = looked.get_mut(b) {
            *seen = true;
        }
    }
    for (b, block) in cfg.blocks.iter().enumerate() {
        if !(looked[b] || out_s[b]) {
            continue;
        }
        let Some(left) = budget.checked_sub(block.stmts.len() + 1) else {
            return false;
        };
        *budget = left;
        let (mut live_s, mut live_t) = (out_s[b], out_t[b]);
        if let Some(e) = block.term.expr() {
            live_s |= reads_in(e, s);
            live_t |= reads_in(e, t);
            if exposed && live_s && !quiet(Effects::of_expr(facts, e)) {
                return false;
            }
        }
        // Back from the block's end: `live_s` and `live_t` say what is still to be read after the statement.
        for stmt in block.stmts.iter().rev() {
            let copy = copy_between(stmt, s, t);
            let targets = stmt.targets();
            let (to_s, to_t) = (targets.contains(&s), targets.contains(&t));
            if !copy && ((to_s && (live_t || to_t)) || (to_t && live_s)) {
                return false;
            }
            live_s = (live_s && !to_s) || reads(stmt, s);
            live_t = (live_t && !to_t) || reads(stmt, t);
            if exposed && live_s && !copy && !quiet(Effects::of_stmt(facts, stmt)) {
                return false;
            }
        }
    }
    !exposed || unobserved(cfg, graph, facts, (s, t), &quiet, budget)
}

/// Whether nothing that may reach `t` through a pointer runs on any path from an assignment of `s` that is no copy of `t` to the next assignment of `t`.
// Once the two are one, that assignment writes `t`'s cell, which until then held another value.
fn unobserved(
    cfg: &Cfg,
    graph: &Graph,
    facts: &Facts,
    (s, t): (Var, Var),
    quiet: &impl Fn(Effects) -> bool,
    budget: &mut usize,
) -> bool {
    let n = cfg.blocks.len();
    let Some(left) = budget.checked_sub(n) else {
        return false;
    };
    *budget = left;
    // A block that names neither keeps the state it is given; each block is entered pending once at most.
    let walk = |b: BlockId, pending: bool, budget: &mut usize| -> Option<bool> {
        let block = cfg.blocks.get(b)?;
        *budget = budget.checked_sub(block.stmts.len() + 1)?;
        let mut pending = pending;
        for stmt in &block.stmts {
            let copy = copy_between(stmt, s, t);
            if pending && !copy && !quiet(Effects::of_stmt(facts, stmt)) {
                return None;
            }
            let targets = stmt.targets();
            if targets.contains(&t) {
                pending = false;
            }
            if targets.contains(&s) && !copy {
                pending = true;
            }
        }
        if pending && (block.term.expr()).is_some_and(|e| !quiet(Effects::of_expr(facts, e))) {
            return None;
        }
        Some(pending)
    };
    let mut seeded: Vec<BlockId> = (graph.named(s).iter().chain(graph.named(t)))
        .copied()
        .collect();
    seeded.sort_unstable();
    seeded.dedup();
    let mut pending_in = vec![false; n];
    let mut work: Vec<BlockId> = Vec::new();
    for b in seeded {
        match walk(b, false, budget) {
            None => return false,
            Some(true) => work.push(b),
            Some(false) => {}
        }
    }
    while let Some(b) = work.pop() {
        let Some(block) = cfg.blocks.get(b) else {
            continue;
        };
        for next in block.term.successors() {
            if next >= n || pending_in[next] {
                continue;
            }
            pending_in[next] = true;
            match walk(next, true, budget) {
                None => return false,
                Some(true) => work.push(next),
                Some(false) => {}
            }
        }
    }
    true
}

/// Whether `s` takes one round's value of `t` out of a loop that makes `t` anew each round, while another assignment gives `s` something else: `found = item; break;`.
// ⚠ One name for the two would mean the same, and leave the place after the loop reachable only
// by a jump: the copy is what lets the loop end in a `break`. So the two stay apart. Past
// `budget` the answer is yes, which only ever keeps a temporary.
fn found_in_a_loop(cfg: &Cfg, graph: &Graph, (s, t): (Var, Var), budget: &mut usize) -> bool {
    let (mut copies, mut made, mut joined) = (Vec::new(), Vec::new(), false);
    let mut seen: HashSet<BlockId> = HashSet::new();
    for b in graph.named(s).iter().chain(graph.named(t)).copied() {
        let Some(block) = cfg.blocks.get(b).filter(|_| seen.insert(b)) else {
            continue;
        };
        for stmt in &block.stmts {
            let (targets, copy) = (stmt.targets(), copy_between(stmt, s, t));
            match (targets.contains(&s), targets.contains(&t), copy) {
                (true, _, true) => copies.push(b),
                (true, _, false) => joined = true,
                (false, true, false) => made.push(b),
                _ => {}
            }
        }
    }
    if !joined || copies.is_empty() || made.is_empty() {
        return false;
    }
    // Whether some path of one edge or more leads from `from` to `to`.
    let mut leads = |from: BlockId, to: BlockId| -> bool {
        let mut reached = vec![false; cfg.blocks.len()];
        let mut work: Vec<BlockId> = cfg
            .blocks
            .get(from)
            .map_or(Vec::new(), |b| b.term.successors());
        while let Some(b) = work.pop() {
            if b == to {
                return true;
            }
            let Some(mark) = reached.get_mut(b).filter(|mark| !**mark) else {
                continue;
            };
            *mark = true;
            match budget.checked_sub(1) {
                Some(left) => *budget = left,
                None => return true,
            }
            work.extend(cfg.blocks[b].term.successors());
        }
        false
    };
    // The round's variable is made in a loop, and the copy is on a way out of it.
    for anew in &made {
        if !leads(*anew, *anew) {
            continue;
        }
        for copy in &copies {
            if leads(*anew, *copy) && !leads(*copy, *anew) {
                return true;
            }
        }
    }
    false
}

/// Makes one variable of each stack variable and a local it is copied into or from, where no place needs the two to hold different values, and removes the copies.
// `if (c) { s = a; } else { s = b; } t = s; if (s == 0) ..` is `if (c) { t = a; } else { t = b; }
// if (t == 0) ..`, and `if (c) { t = a; .. s = t; } else { s = b; } f(s);` is `if (c) { t = a; .. }
// else { t = b; } f(t);`: the value a join carries is the local's from where it is made.
// ⚠ Neither may have its address taken or hold several cells, and each pair found on the code
// unchanged shares no variable with another.
pub(super) fn unite(f: &mut cfg::Function) -> bool {
    let mut changed = false;
    for _ in 0..MERGES {
        let facts = Facts::of(f);
        let head = &f.head;
        let one = |var: &Var| {
            let wide = (head.vars.get(var)).is_some_and(|info| info.width == Width::Cells);
            let uses = facts.uses.get(var).copied().unwrap_or_default();
            !wide && !facts.widths.contains_key(var) && uses.addrs == 0
        };
        let mut seen: HashSet<(Var, Var)> = HashSet::new();
        // ⚠ A local the join is copied into comes before a local copied into the join: the first
        // is the join under its own name, the second one arm's value, which keeps its variable.
        let mut pairs: Vec<(Var, Var)> = Vec::new();
        let mut arms: Vec<(Var, Var)> = Vec::new();
        for block in &f.cfg.blocks {
            for stmt in &block.stmts {
                let Stmt::Assign {
                    target,
                    value: Expr::Var(from),
                } = stmt
                else {
                    continue;
                };
                let (pair, into) = match (target, from) {
                    (Var::Local(_), Var::Stack { .. }) => ((*from, *target), true),
                    (Var::Stack { .. }, Var::Local(_)) => ((*target, *from), false),
                    _ => continue,
                };
                if one(&pair.0) && one(&pair.1) && seen.insert(pair) {
                    match into {
                        true => pairs.push(pair),
                        false => arms.push(pair),
                    }
                }
            }
        }
        pairs.append(&mut arms);
        if pairs.is_empty() {
            break;
        }
        let mut budget = allowance(&f.cfg);
        let vars: HashSet<Var> = pairs.iter().flat_map(|(s, t)| [*s, *t]).collect();
        let graph = Graph::of(&f.cfg, &vars);
        let mut taken: HashSet<Var> = HashSet::new();
        let mut renamed: HashMap<Var, Var> = HashMap::new();
        for (s, t) in pairs {
            if budget == 0 {
                break;
            }
            if taken.contains(&s) || taken.contains(&t) {
                continue;
            }
            if found_in_a_loop(&f.cfg, &graph, (s, t), &mut budget) {
                continue;
            }
            let exposed = !facts.named_only(t);
            if united(&f.cfg, &graph, &facts, (s, t), exposed, &mut budget) {
                taken.insert(s);
                taken.insert(t);
                renamed.insert(s, t);
            }
        }
        if renamed.is_empty() {
            break;
        }
        for block in &mut f.cfg.blocks {
            for stmt in &mut block.stmts {
                match stmt {
                    Stmt::Assign { target, .. } => {
                        if let Some(to) = renamed.get(&*target) {
                            *target = *to;
                        }
                    }
                    Stmt::AssignMany { targets, .. } => {
                        for target in targets.iter_mut() {
                            if let Some(to) = renamed.get(&*target) {
                                *target = *to;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        each_top_mut(&mut f.cfg, &mut |top| {
            top.walk_mut(&mut |e| {
                if let Expr::Var(var) = e {
                    if let Some(to) = renamed.get(&*var) {
                        *var = *to;
                    }
                }
            });
        });
        // The copies between the two are now of the local into itself.
        let kept: HashSet<Var> = renamed.values().copied().collect();
        let itself = |stmt: &Stmt| match stmt {
            Stmt::Assign {
                target,
                value: Expr::Var(from),
            } => target == from && kept.contains(target),
            _ => false,
        };
        for block in &mut f.cfg.blocks {
            block.stmts.retain(|stmt| !itself(stmt));
        }
        for s in renamed.keys() {
            f.head.vars.remove(s);
        }
        changed = true;
    }
    changed
}

/// A row of temporaries a lifter numbers one after another, `first` and the `cells - 1` after it, that a value of several cells may be carried in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Slots {
    first: Var,
    cells: u32,
}

impl Slots {
    /// Where `var` stands in the row.
    fn place(self, var: Var) -> Option<u32> {
        let i = match (self.first, var) {
            (Var::Stack { at, index }, Var::Stack { at: of, index: n })
            | (Var::Value { at, index }, Var::Value { at: of, index: n })
                if at == of =>
            {
                n.checked_sub(index)?
            }
            _ => return None,
        };
        (i < self.cells).then_some(i)
    }

    /// The temporary at place `i` of the row.
    fn member(self, i: u32) -> Option<Var> {
        match self.first {
            Var::Stack { at, index } => Some(Var::Stack {
                at,
                index: index.checked_add(i)?,
            }),
            Var::Value { at, index } => Some(Var::Value {
                at,
                index: index.checked_add(i)?,
            }),
            _ => None,
        }
    }
}

/// Whether `var` is a temporary of one cell, which may stand in a row.
fn lone_cell(facts: &Facts, head: &Head, var: Var) -> bool {
    let many = (head.vars.get(&var)).is_some_and(|info| info.width == Width::Cells);
    var.is_temporary() && !many && !facts.widths.contains_key(&var)
}

/// The cells of `base` where it is a whole value of several cells whose width something states.
// ⚠ A global's name reads one cell of memory, whatever kind it is given.
fn width_of(facts: &Facts, head: &Head, base: &Expr) -> Option<u32> {
    match base {
        Expr::GuidEmpty => Some(2),
        Expr::Var(var) if !matches!(var, Var::Global { .. }) => {
            let many = (head.vars.get(var)).is_some_and(|info| info.width == Width::Cells);
            facts.widths.get(var).copied().filter(|_| many)
        }
        _ => None,
    }
}

/// The temporary a lifter numbers just before `var`.
fn before(var: Var) -> Option<Var> {
    match var {
        Var::Stack { at, index } => Some(Var::Stack {
            at,
            index: index.checked_sub(1)?,
        }),
        Var::Value { at, index } => Some(Var::Value {
            at,
            index: index.checked_sub(1)?,
        }),
        _ => None,
    }
}

/// How many statements `stmts` starts with that give the temporaries from `first` on the cells of `base` from `low` on, one each in turn, within its `width`.
fn part_of(stmts: &[Stmt], first: Var, base: &Expr, low: u32, width: u32) -> u32 {
    let slots = Slots {
        first,
        cells: width,
    };
    let mut n = 0u32;
    for stmt in stmts.iter().take(WHOLE as usize) {
        let Stmt::Assign {
            target,
            value: Expr::Cell { base: of, index },
        } = stmt
        else {
            break;
        };
        let wanted = low.checked_add(n).filter(|cell| *cell < width);
        if slots.member(n) != Some(*target) || wanted != Some(*index) || **of != *base {
            break;
        }
        n += 1;
    }
    n
}

/// The one value the assignments `stmts` starts with give the temporaries of `slots`, each in turn: every cell of one whole value, zeroes, or locals in a row; with `part`, also cells in a row inside one value.
fn row(facts: &Facts, head: &Head, stmts: &[Stmt], slots: Slots, part: bool) -> Option<Expr> {
    let run = stmts.get(..usize::try_from(slots.cells).ok()?)?;
    let mut values: Vec<&Expr> = Vec::with_capacity(run.len());
    for (stmt, i) in run.iter().zip(0u32..) {
        let Stmt::Assign { target, value } = stmt else {
            return None;
        };
        if slots.place(*target) != Some(i) {
            return None;
        }
        values.push(value);
    }
    let first = *values.first()?;
    // `t = x[0]; t+1 = x[1]; ..` over every cell of an `x` that is none of the row, or with
    // `part`, `t = x[k]; t+1 = x[k+1]; ..` over cells of `x` in a row, as the bytes they are.
    if let Expr::Cell { base, index: low } = first {
        let apart = !matches!(&**base, Expr::Var(var) if slots.place(*var).is_some());
        let width = width_of(facts, head, base);
        let cells = (values.iter().copied().zip(0u32..)).all(|(value, i)| {
            matches!(value, Expr::Cell { base: of, index } if Some(*index) == low.checked_add(i) && of == base)
        });
        if !(apart && cells) {
            return None;
        }
        if *low == 0 && width == Some(slots.cells) {
            return Some((**base).clone());
        }
        let end = low.checked_add(slots.cells);
        let inside = width.zip(end).is_some_and(|(width, end)| end <= width);
        let (offset, bytes) = (low.checked_mul(8)?, slots.cells.checked_mul(8)?);
        return (part && inside).then(|| Expr::ExtractBytes {
            value: base.clone(),
            offset,
            bytes,
        });
    }
    let zeroes =
        (values.iter().copied()).all(|value| matches!(value, Expr::Const(c) if c.bits == 0));
    if zeroes {
        return Some(Expr::ZeroSlots(slots.cells));
    }
    // Locals of one cell each, one after another: the group of them is what the row holds.
    let Expr::Var(Var::Local(low)) = first else {
        return None;
    };
    let locals = (values.iter().copied().zip(0u32..)).all(|(value, i)| match value {
        Expr::Var(var @ Var::Local(n)) => {
            let one = !facts.widths.contains_key(var) && one_cell(head, value);
            Some(*n) == low.checked_add(i) && one
        }
        _ => false,
    });
    locals.then(|| Expr::Cells(values.into_iter().cloned().collect()))
}

/// The row whose every temporary `items[at..]` names in order, from its first.
fn run_of(members: &HashMap<Var, (Slots, u32)>, items: &[Expr], at: usize) -> Option<Slots> {
    let Some(Expr::Var(var)) = items.get(at) else {
        return None;
    };
    let (slots, 0) = *members.get(var)? else {
        return None;
    };
    let end = at.checked_add(usize::try_from(slots.cells).ok()?)?;
    let run = items.get(at..end)?;
    let named = (run.iter().zip(0u32..))
        .all(|(item, i)| matches!(item, Expr::Var(var) if slots.place(*var) == Some(i)));
    named.then_some(slots)
}

/// Notes as taken apart each row a temporary of which stands in `inner`, where `inner` is a group that what holds it reads only a part of.
fn picked(members: &HashMap<Var, (Slots, u32)>, inner: &Expr, refused: &mut HashSet<Var>) {
    let Expr::Cells(items) = inner else {
        return;
    };
    for item in items {
        let Expr::Var(var) = item else {
            continue;
        };
        if let Some((slots, _)) = members.get(var) {
            refused.insert(slots.first);
        }
    }
}

/// Makes one variable of each row of temporaries that only ever takes a value of several cells whole and hands it on whole.
// `t0 = x[0]; t1 = x[1]; .. f(t0, t1);` is `t0 = x; .. f(t0);`, however many rows assign the
// temporaries on the way to a join: each row is the value one way in gives. A row of every cell
// of one whole value says the temporaries carry a value; another row may give zeroes, or locals
// in a row. So does a row of cells in a row inside one value, `t0 = x[2]; t1 = x[3];`, where
// every read of the temporaries is a group of exactly them, `f((t0, t1))`: the value is those bytes.
// ⚠ Every assignment of each temporary must be in such a row, and every read in a run of all of
// them in order that a call or a group is given whole: no cell is taken alone, so the variable is
// only another name for the same cells.
pub(super) fn wholes(f: &mut cfg::Function) -> bool {
    let facts = Facts::of(f);
    let head = &f.head;
    // The rows each row of every cell of one whole value proposes, by their first temporary, and
    // those a row of cells inside a value proposes: a group of exactly them must read each of these.
    let mut proposed: BTreeMap<Var, u32> = BTreeMap::new();
    let mut parted: HashSet<Var> = HashSet::new();
    let mut refused: HashSet<Var> = HashSet::new();
    for block in &f.cfg.blocks {
        for (i, stmt) in block.stmts.iter().enumerate() {
            let Stmt::Assign {
                target,
                value: Expr::Cell { base, index: low },
            } = stmt
            else {
                continue;
            };
            let width = width_of(&facts, head, base).filter(|cells| (2..=WHOLE).contains(cells));
            let Some(width) = width else {
                continue;
            };
            if !lone_cell(&facts, head, *target) {
                continue;
            }
            let whole = Slots {
                first: *target,
                cells: width,
            };
            let every = *low == 0 && row(&facts, head, &block.stmts[i..], whole, false).is_some();
            // ⚠ A row inside a value only from its first statement: a later one starts no row of its own.
            let earlier = before(*target).zip(low.checked_sub(1));
            let continues = match (i.checked_sub(1).and_then(|j| block.stmts.get(j)), earlier) {
                (
                    Some(Stmt::Assign {
                        target: prior,
                        value: Expr::Cell { base: of, index },
                    }),
                    Some((slot, cell)),
                ) => *prior == slot && *index == cell && of == base,
                _ => false,
            };
            let (cells, part) = match (every, continues) {
                (true, _) => (width, false),
                (false, true) => continue,
                (false, false) => (part_of(&block.stmts[i..], *target, base, *low, width), true),
            };
            if cells < 2 {
                continue;
            }
            if part {
                parted.insert(*target);
            }
            if proposed
                .insert(*target, cells)
                .is_some_and(|known| known != cells)
            {
                refused.insert(*target);
            }
        }
    }
    if proposed.is_empty() {
        return false;
    }
    // A temporary stands in one row at most: rows that share one are left alone.
    let mut members: HashMap<Var, (Slots, u32)> = HashMap::new();
    let mut rows: Vec<Slots> = Vec::new();
    for (first, cells) in &proposed {
        let slots = Slots {
            first: *first,
            cells: *cells,
        };
        rows.push(slots);
        for i in 0..*cells {
            let Some(var) = slots.member(i) else {
                refused.insert(*first);
                break;
            };
            if let Some((other, _)) = members.insert(var, (slots, i)) {
                refused.insert(other.first);
                refused.insert(*first);
            }
        }
    }
    members.retain(|_, (slots, _)| !refused.contains(&slots.first));

    // How many rows assign each row's temporaries, and how many runs read them.
    let wide = |var: &Var| (head.vars.get(var)).is_some_and(|info| info.width == Width::Cells);
    let mut assigned: HashMap<Var, u32> = HashMap::new();
    let mut handed: HashMap<Var, u32> = HashMap::new();
    // The rows a run of whose temporaries something reads other than as a group of exactly them.
    let mut loose: HashSet<Var> = HashSet::new();
    for block in &f.cfg.blocks {
        for stmt in &block.stmts {
            // What a statement reads of a group only in part: an address, one cell stored, a cell assigned.
            match stmt {
                Stmt::Assign { target, value } if !wide(target) => {
                    picked(&members, value, &mut refused);
                }
                Stmt::AssignMany { values, .. } => {
                    for value in values {
                        picked(&members, value, &mut refused);
                    }
                }
                Stmt::Store { addr, value, .. } => {
                    picked(&members, addr, &mut refused);
                    picked(&members, value, &mut refused);
                }
                Stmt::StoreBytes { addr, .. }
                | Stmt::StoreF32Slots { addr, .. }
                | Stmt::ZeroBytes { addr, .. }
                | Stmt::StepI32 { addr, .. } => picked(&members, addr, &mut refused),
                Stmt::Assign { .. } | Stmt::Eval(_) => {}
            }
            stmt.each_expr(&mut |top| {
                each_run(&members, top, &mut handed, &mut refused, &mut loose);
            });
        }
        // Each assignment of a temporary of a row is a statement of a whole row that starts at its first.
        let mut i = 0;
        while i < block.stmts.len() {
            let hit = (block.stmts[i].targets().iter())
                .find_map(|target| members.get(target))
                .copied();
            match hit {
                Some((slots, 0))
                    if row(
                        &facts,
                        head,
                        &block.stmts[i..],
                        slots,
                        parted.contains(&slots.first),
                    )
                    .is_some() =>
                {
                    *assigned.entry(slots.first).or_default() += 1;
                    i += slots.cells as usize;
                }
                Some((slots, _)) => {
                    refused.insert(slots.first);
                    i += 1;
                }
                None => i += 1,
            }
        }
        match &block.term {
            Terminator::Branch { cond: e, .. } | Terminator::Switch { value: e, .. } => {
                picked(&members, e, &mut refused);
            }
            Terminator::Jump(_) | Terminator::Return(_) | Terminator::Unreachable => {}
        }
        if let Some(e) = block.term.expr() {
            each_run(&members, e, &mut handed, &mut refused, &mut loose);
        }
    }

    let valid: HashMap<Var, Slots> = rows
        .into_iter()
        .filter(|slots| {
            let made = assigned.get(&slots.first).copied().unwrap_or(0);
            let runs = handed.get(&slots.first).copied().unwrap_or(0);
            let each = (0..slots.cells).all(|i| {
                slots.member(i).is_some_and(|var| {
                    let uses = facts.uses.get(&var).copied().unwrap_or_default();
                    let counted = uses.defs == made && uses.reads == runs && uses.addrs == 0;
                    lone_cell(&facts, head, var) && counted
                })
            });
            let grouped = !(parted.contains(&slots.first) && loose.contains(&slots.first));
            !refused.contains(&slots.first) && made > 0 && each && grouped
        })
        .map(|slots| (slots.first, slots))
        .collect();
    if valid.is_empty() {
        return false;
    }
    members.retain(|_, (slots, _)| valid.contains_key(&slots.first));

    for block in &mut f.cfg.blocks {
        let mut found: Vec<(usize, Slots, Expr)> = Vec::new();
        let mut i = 0;
        while i < block.stmts.len() {
            let first = match &block.stmts[i] {
                Stmt::Assign { target, .. } => members.get(target).copied(),
                _ => None,
            };
            let made = first
                .filter(|(_, place)| *place == 0)
                .and_then(|(slots, _)| {
                    let part = parted.contains(&slots.first);
                    Some((slots, row(&facts, head, &block.stmts[i..], slots, part)?))
                });
            match made {
                Some((slots, value)) => {
                    found.push((i, slots, value));
                    i += slots.cells as usize;
                }
                None => i += 1,
            }
        }
        // From the last back, so that the places before stay where they were found.
        for (i, slots, value) in found.into_iter().rev() {
            block.stmts.drain(i + 1..i + slots.cells as usize);
            block.stmts[i] = Stmt::Assign {
                target: slots.first,
                value,
            };
        }
    }
    each_top_mut(&mut f.cfg, &mut |top| {
        top.walk_mut(&mut |e| {
            if let Expr::Call { args: items, .. } | Expr::Cells(items) = e {
                let mut at = 0;
                while at < items.len() {
                    if let Some(slots) = run_of(&members, &items[..], at) {
                        items.drain(at + 1..at + slots.cells as usize);
                        items[at] = Expr::Var(slots.first);
                    }
                    at += 1;
                }
            }
            // A group of the one variable alone is the variable.
            let lone = matches!(&*e, Expr::Cells(items) if matches!(&items[..], [Expr::Var(var)] if valid.contains_key(var)));
            if lone {
                if let Expr::Cells(mut items) = std::mem::replace(e, Expr::GuidEmpty) {
                    if let Some(item) = items.pop() {
                        *e = item;
                    }
                }
            }
        });
    });
    for slots in valid.values() {
        for i in 1..slots.cells {
            if let Some(var) = slots.member(i) {
                f.head.vars.remove(&var);
            }
        }
        let info = (f.head.vars.entry(slots.first)).or_insert_with(|| VarInfo::new(Width::Cells));
        info.width = Width::Cells;
        info.kind = Kind::Aggregate(slots.cells);
        info.single = false;
    }
    true
}

/// Counts in `top` each run of a row's temporaries a call or a group is given, notes as taken apart each row a group of whose temporaries something reads only in part, and as `loose` each row a run of which is no group of exactly its temporaries.
fn each_run(
    members: &HashMap<Var, (Slots, u32)>,
    top: &Expr,
    handed: &mut HashMap<Var, u32>,
    refused: &mut HashSet<Var>,
    loose: &mut HashSet<Var>,
) {
    top.walk(&mut |e| match e {
        Expr::Call { args: items, .. } | Expr::Cells(items) => {
            let grouped = matches!(e, Expr::Cells(_));
            let mut at = 0;
            while at < items.len() {
                match run_of(members, items, at) {
                    Some(slots) => {
                        *handed.entry(slots.first).or_default() += 1;
                        if !(grouped && items.len() == slots.cells as usize) {
                            loose.insert(slots.first);
                        }
                        at += slots.cells as usize;
                    }
                    None => at += 1,
                }
            }
        }
        // A cell picked out of a group, its low bits, a test of it or an address made of it.
        Expr::Cell { base: inner, .. }
        | Expr::ExtractBytes { value: inner, .. }
        | Expr::Cast { e: inner, .. }
        | Expr::Truth(inner)
        | Expr::Unary { e: inner, .. }
        | Expr::Load { addr: inner, .. }
        | Expr::LoadBytes { addr: inner, .. }
        | Expr::NewArray { len: inner, .. }
        | Expr::VirtualFunctionRef {
            receiver: inner, ..
        }
        | Expr::Select { cond: inner, .. } => picked(members, inner, refused),
        Expr::Binary { l, r, .. } | Expr::ByteOffset { addr: l, offset: r } => {
            picked(members, l, refused);
            picked(members, r, refused);
        }
        Expr::Vector3(items) => {
            for item in items.iter() {
                picked(members, item, refused);
            }
        }
        _ => {}
    });
}

/// The one assignment of a block that assigns one variable and jumps on: the variable, the value and where the block goes.
fn lone_assign(block: &Block) -> Option<(Var, &Expr, BlockId)> {
    match (block.stmts.as_slice(), &block.term) {
        ([Stmt::Assign { target, value }], Terminator::Jump(to)) => Some((*target, value, *to)),
        _ => None,
    }
}

/// Whether `e` yields 0 or 1 by its form: a test, or one of those two constants.
fn truth(e: &Expr) -> bool {
    match e {
        Expr::Truth(_) | Expr::Unary { op: UnOp::Not, .. } => true,
        Expr::Binary { op, .. } => op.is_logical(),
        Expr::Const(c) => c.bits <= 1 && !c.kind.is_float(),
        _ => false,
    }
}

/// Whether `value` may stand as an arm of a choice: one cell that no call, store or allocation makes, and no choice itself.
fn chosen(facts: &Facts, head: &Head, value: &Expr) -> bool {
    let effects = Effects::of_expr(facts, value);
    let quiet = !(effects.call || effects.store || effects.alloc);
    quiet && one_cell(head, value) && !matches!(value, Expr::Select { .. })
}

/// Whether the block two arms meet in only tests what they hand it in `var`, at once or through copies of it.
// The test may still compare the value with constants and join such comparisons: that is the
// test of a condition whose parts the arms hold. One that reads anything else uses a value.
fn only_tested(join: &Block, var: Var) -> bool {
    let Terminator::Branch { cond, .. } = &join.term else {
        return false;
    };
    if join.stmts.len() > JOIN_COPIES {
        return false;
    }
    let mut carried = vec![var];
    for stmt in &join.stmts {
        if let Stmt::Assign {
            target,
            value: Expr::Var(from),
        } = stmt
        {
            if carried.contains(from) {
                carried.push(*target);
            }
        }
    }
    let (mut read, mut plain) = (false, true);
    cond.walk(&mut |e| match e {
        Expr::Var(named) => {
            let ours = carried.contains(named);
            read |= ours;
            plain &= ours;
        }
        Expr::Const(_) | Expr::Truth(_) | Expr::Cast { .. } | Expr::Unary { op: UnOp::Not, .. } => {
        }
        Expr::Binary { op, .. } => plain &= op.is_logical(),
        _ => plain = false,
    });
    read && plain
}

/// The variable block `b` chooses a value for, the choice, and the block its two arms meet in, where `b` branches to two arms that each only assign that variable.
fn choice(
    facts: &Facts,
    head: &Head,
    cfg: &Cfg,
    preds: &[Vec<BlockId>],
    b: BlockId,
) -> Option<(Var, Expr, BlockId)> {
    let Terminator::Branch { cond, then, els } = &cfg.blocks.get(b)?.term else {
        return None;
    };
    let (then, els) = (*then, *els);
    // An arm is entered by this branch alone, so that nothing else needs what it assigns there.
    let entered = |arm: BlockId| {
        let only = preds.get(arm).is_some_and(|from| from.as_slice() == [b]);
        arm != b && arm != ENTRY && only
    };
    if then == els || !entered(then) || !entered(els) {
        return None;
    }
    let (target, yes, join) = lone_assign(cfg.blocks.get(then)?)?;
    let (other, no, meet) = lone_assign(cfg.blocks.get(els)?)?;
    let one = !matches!(target, Var::Global { .. })
        && (head.vars.get(&target)).is_none_or(|info| info.width != Width::Cells)
        && !facts.widths.contains_key(&target);
    let apart = join != then && join != els;
    if target != other || join != meet || !one || !apart {
        return None;
    }
    let values = chosen(facts, head, yes) && chosen(facts, head, no);
    if !values || (truth(yes) && truth(no)) || only_tested(cfg.blocks.get(join)?, target) {
        return None;
    }
    let shallow = [cond, yes, no].into_iter().all(|e| depth(e) < CHOICE_DEPTH);
    shallow.then(|| {
        let value = Expr::Select {
            cond: Box::new(cond.clone()),
            then: Box::new(yes.clone()),
            els: Box::new(no.clone()),
        };
        (target, value, join)
    })
}

/// Writes a branch whose two arms each give one variable one value, and do nothing else, as the one assignment of a choice between the two values.
// `if (c) { s = a; } else { s = b; }` is `s = c ? a : b;`: the test is evaluated where it was,
// and of the two values only the one whose arm ran. The arms are left with nothing that leads
// to them, for `tidy` to remove.
// ⚠ Not where the two values are truths, nor where the block the arms meet in only tests the
// variable: those are the value and the test of an `&&` or an `||`, which the structure pass
// reads from the arms as they stand.
pub(super) fn selects(f: &mut cfg::Function) -> bool {
    let facts = Facts::of(f);
    let preds = f.cfg.predecessors();
    // Every choice is found on the blocks as they stand: a branch written as a choice no longer
    // shows whether it tested what other arms hand it. No two share a block that changes.
    let found: Vec<(BlockId, (Var, Expr, BlockId))> = (0..f.cfg.blocks.len())
        .filter_map(|b| Some((b, choice(&facts, &f.head, &f.cfg, &preds, b)?)))
        .collect();
    let changed = !found.is_empty();
    for (b, (target, value, join)) in found {
        let block = &mut f.cfg.blocks[b];
        block.stmts.push(Stmt::Assign { target, value });
        block.term = Terminator::Jump(join);
    }
    changed
}
