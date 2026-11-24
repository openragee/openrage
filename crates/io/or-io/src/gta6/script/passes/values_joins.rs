//! The values a lifter carries across a join in `stack_` variables: tuples taken apart, each
//! variable that only renames another value replaced by that value, and a join that is only
//! copied into the next one merged with it.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::gta6::script::cfg::{self, BlockId, Cfg, ENTRY};
use crate::gta6::script::ir::{Expr, Head, Stmt, Var, Width};

use super::effects::{each_top_mut, Facts};

/// The sweeps given to settling what each stack variable carries; a chain of renames longer than this that runs against the order of the blocks is left alone.
const SWEEPS: usize = 32;

/// The sweeps given to merging joins: each merges one more link of a chain of them.
const MERGES: usize = 16;

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
                    Expr::Var(from) if matches!(from, Var::Stack { .. }) && from != target => {
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
