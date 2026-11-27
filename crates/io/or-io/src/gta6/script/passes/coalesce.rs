//! Coalesce: a variable that only ever holds one constant reads as the constant, and two
//! variables that are copies of each other and never live with different values are one.
//!
//! Works on the blocks, in rounds with `values` and `forward`. A frame cell the compiler used
//! for several variables is taken apart by which assignments each read can see, and each of those
//! variables is judged alone: it is rewritten whole, with its assignments gone, or not at all.

use std::collections::{HashMap, HashSet};

use crate::gta6::script::cfg::{self, BlockId};
use crate::gta6::script::ir::{Expr, Stmt, Var, Width};
use crate::gta6::script::{Context, Pass, Stats};

use super::forward::flow::{self, Set};
use super::values::effects::Facts;
use super::values::fold::{adjacent, chained, constant, leftover};

/// The names this file's rewrites are switched by.
pub const PASSES: &[Pass] = &[Pass::Constants, Pass::Coalesce];

/// The times a function's assignments are followed anew: a rewrite may leave the next one to find.
const SWEEPS: usize = 4;

/// The assignments one variable may have and still be followed.
const DEFS: u32 = 512;

/// Where a statement stands: its block and its place there, the terminator past the last statement.
type Site = (BlockId, usize);

/// Carries constants to their uses and makes one variable of copies that never differ; `true` when anything changed.
// `constants` replaces a variable every assignment of which gives it one constant. `coalesce`
// replaces a variable that only copies another by that other, and removes an assignment no read
// can see.
pub fn run(f: &mut cfg::Function, cx: &Context<'_>, _stats: &mut Stats) -> bool {
    let (constants, copies) = (
        cx.options.on(Pass::Constants),
        cx.options.on(Pass::Coalesce),
    );
    if f.head.incomplete.is_some() || f.cfg.blocks.is_empty() {
        return false;
    }
    let mut changed = false;
    for _ in 0..SWEEPS {
        if !sweep(f, constants, copies) {
            break;
        }
        changed = true;
    }
    if changed {
        super::values::declared(f);
    }
    changed
}

/// Lets a statement evaluate the value the statement before it made for it alone, after a pass that made one variable of several cells; `true` when anything changed.
// `aggregates` runs once the last fold is over and leaves `v = f(); local = v;` where it made
// `local` of two locals. Switched by `values`, whose fold this is. With `coalesce` on too, a row
// of such values goes into the statement after it that reads them in their order (`chained`).
pub fn settle(f: &mut cfg::Function, cx: &Context<'_>, _stats: &mut Stats) -> bool {
    if !cx.options.on(Pass::Values) || f.head.incomplete.is_some() || f.cfg.blocks.is_empty() {
        return false;
    }
    let mut changed = adjacent(f);
    changed |= cx.options.on(Pass::Coalesce) && chained(f);
    if changed {
        super::values::declared(f);
    }
    changed
}

/// The locals whose cells another local the function names also covers: no assignment of one says all that is written to it.
fn shared(facts: &Facts) -> HashSet<u32> {
    let mut spans: Vec<(u32, u32)> = (facts.uses.keys())
        .filter_map(|var| match var {
            Var::Local(n) => {
                let more = facts.widths.get(var).copied().unwrap_or(1).max(1) - 1;
                Some((*n, n.saturating_add(more)))
            }
            _ => None,
        })
        .collect();
    spans.sort_unstable();
    let mut shared = HashSet::new();
    let mut reach: Option<u32> = None;
    for (i, (lo, hi)) in spans.iter().enumerate() {
        let earlier = reach.is_some_and(|reach| *lo <= reach);
        let later = spans.get(i + 1).is_some_and(|(next, _)| *next <= *hi);
        if earlier || later {
            shared.insert(*lo);
        }
        reach = Some(reach.map_or(*hi, |reach| reach.max(*hi)));
    }
    shared
}

/// The web a definition belongs to: the lowest definition some read sees together with it.
fn root(web: &mut [usize], mut id: usize) -> usize {
    while web[id] != id {
        web[id] = web[web[id]];
        id = web[id];
    }
    id
}

fn join(web: &mut [usize], a: usize, b: usize) {
    let (a, b) = (root(web, a), root(web, b));
    if a != b {
        web[a.max(b)] = a.min(b);
    }
}

/// Which assignments of each followed variable every read of it can see.
// A variable is followed where nothing but an assignment that names it changes it. Its first
// definition, numbered as the variable is, stands for the value it has where the function
// starts; a web is the assignments that some chain of reads sees together, which is one variable
// of the source however many share the frame cell.
struct Reach {
    vars: Vec<Var>,
    /// The variable each definition is of, and the statement that makes it; none for the value at the start.
    defs: Vec<(usize, Option<Site>)>,
    /// Each read, in the order of the blocks: where, of which variable, and a definition it sees.
    reads: Vec<(Site, usize, usize)>,
    /// The web of each definition, by [`root`].
    web: Vec<usize>,
    /// The definitions some read sees.
    seen: Vec<bool>,
    /// The variables read where no definition reaches: code nothing reaches, which is left alone.
    spoiled: Vec<bool>,
}

impl Reach {
    fn of(f: &cfg::Function, facts: &Facts) -> Option<Reach> {
        let shared = shared(facts);
        let mut vars: Vec<Var> = Vec::new();
        let mut index: HashMap<Var, usize> = HashMap::new();
        let mut refused: HashSet<Var> = HashSet::new();
        for stmt in f.cfg.blocks.iter().flat_map(|block| &block.stmts) {
            for target in stmt.targets() {
                if index.contains_key(target) || refused.contains(target) {
                    continue;
                }
                let own = match target {
                    Var::Local(n) => !shared.contains(n),
                    Var::Value { .. } | Var::Stack { .. } => true,
                    Var::Param(_) | Var::Global { .. } => false,
                };
                let few = facts.uses.get(target).is_some_and(|uses| uses.defs <= DEFS);
                if own && few && facts.named_only(*target) {
                    index.insert(*target, vars.len());
                    vars.push(*target);
                } else {
                    refused.insert(*target);
                }
            }
        }
        if vars.is_empty() {
            return None;
        }

        let mut defs: Vec<(usize, Option<Site>)> = (0..vars.len()).map(|var| (var, None)).collect();
        let mut of_var: Vec<Vec<usize>> = (0..vars.len()).map(|var| vec![var]).collect();
        // The definitions each block makes, with the statement that makes each.
        let mut made_in: Vec<Vec<(usize, usize)>> = Vec::with_capacity(f.cfg.blocks.len());
        for (b, block) in f.cfg.blocks.iter().enumerate() {
            let mut here = Vec::new();
            for (k, stmt) in block.stmts.iter().enumerate() {
                for target in stmt.targets() {
                    if let Some(var) = index.get(target).copied() {
                        here.push((k, defs.len()));
                        of_var[var].push(defs.len());
                        defs.push((var, Some((b, k))));
                    }
                }
            }
            made_in.push(here);
        }
        let n = defs.len();
        if !flow::fits(&f.cfg, n) {
            return None;
        }

        let (mut made, mut ended) = (Vec::new(), Vec::new());
        for here in &made_in {
            let (mut live, mut gone) = (Set::new(n), Set::new(n));
            for (_, id) in here {
                for other in &of_var[defs[*id].0] {
                    live.remove(*other);
                    gone.insert(*other);
                }
                live.insert(*id);
            }
            made.push(live);
            ended.push(gone);
        }
        let mut start = Set::new(n);
        for var in 0..vars.len() {
            start.insert(var);
        }
        let ins = flow::entering(&f.cfg, n, (&made[..], &ended[..]), &start, false)?;

        let mut reach = Reach {
            spoiled: vec![false; vars.len()],
            vars,
            defs,
            reads: Vec::new(),
            web: (0..n).collect(),
            seen: vec![false; n],
        };
        for (b, block) in f.cfg.blocks.iter().enumerate() {
            let mut live = ins.get(b)?.clone();
            let mut next = made_in.get(b)?.iter().peekable();
            for k in 0..=block.stmts.len() {
                let mut read: Vec<usize> = Vec::new();
                let mut see = |top: &Expr| {
                    top.walk(&mut |e| {
                        let var = match e {
                            Expr::Var(var) => index.get(var).copied(),
                            _ => None,
                        };
                        if let Some(var) = var.filter(|var| !read.contains(var)) {
                            read.push(var);
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
                    let mut first: Option<usize> = None;
                    for id in &of_var[var] {
                        if !live.has(*id) {
                            continue;
                        }
                        reach.seen[*id] = true;
                        match first {
                            Some(first) => join(&mut reach.web, first, *id),
                            None => first = Some(*id),
                        }
                    }
                    match first {
                        Some(id) => reach.reads.push(((b, k), var, id)),
                        None => reach.spoiled[var] = true,
                    }
                }
                while let Some((_, id)) = next.next_if(|(at, _)| *at == k) {
                    for other in &of_var[reach.defs[*id].0] {
                        live.remove(*other);
                    }
                    live.insert(*id);
                }
            }
        }
        reach.web = (0..n).map(|id| root(&mut reach.web, id)).collect();
        Some(reach)
    }
}

/// The value the plain assignment at `site` gives `var`; none where the statement is no such assignment.
fn given(f: &cfg::Function, site: Site, var: Var) -> Option<&Expr> {
    match f.cfg.blocks.get(site.0)?.stmts.get(site.1)? {
        Stmt::Assign { target, value } if *target == var => Some(value),
        _ => None,
    }
}

/// The copies `l = s;` that may stand in for a variable, each pair known by its number.
#[derive(Default)]
struct Pairs {
    numbered: HashMap<(Var, Var), usize>,
    /// The pairs an assignment of each variable ends.
    touching: HashMap<Var, Vec<usize>>,
}

impl Pairs {
    fn number(&mut self, l: Var, s: Var) -> usize {
        let next = self.numbered.len();
        let pair = *self.numbered.entry((l, s)).or_insert(next);
        if pair == next {
            self.touching.entry(l).or_default().push(pair);
            self.touching.entry(s).or_default().push(pair);
        }
        pair
    }

    /// Lets `standing` be the copies that stand after `stmt`, and adds to `ended` those it ends.
    fn pass(&self, stmt: &Stmt, standing: &mut Set, ended: &mut Set) {
        for target in stmt.targets() {
            for pair in self.touching.get(target).into_iter().flatten() {
                standing.remove(*pair);
                ended.insert(*pair);
            }
        }
        if let Stmt::Assign {
            target,
            value: Expr::Var(source),
        } = stmt
        {
            if let Some(pair) = self.numbered.get(&(*target, *source)) {
                standing.insert(*pair);
            }
        }
    }
}

/// The webs, of those `wanted` gives a pair, whose copy does not stand at one of their reads.
// A copy stands from `l = s;` to the next assignment of either, and at a join only where it
// stands on every path in: equal assignments of `s` reaching the copy and the read prove
// nothing, since a loop may run `s` on while `l` lags a round.
fn fallen(
    f: &cfg::Function,
    reach: &Reach,
    pairs: &Pairs,
    wanted: &HashMap<usize, usize>,
) -> Option<HashSet<usize>> {
    let n = pairs.numbered.len();
    if !flow::fits(&f.cfg, n) {
        return None;
    }
    let (mut made, mut ended) = (Vec::new(), Vec::new());
    for block in &f.cfg.blocks {
        let (mut standing, mut gone) = (Set::new(n), Set::new(n));
        for stmt in &block.stmts {
            pairs.pass(stmt, &mut standing, &mut gone);
        }
        made.push(standing);
        ended.push(gone);
    }
    let ins = flow::entering(&f.cfg, n, (&made[..], &ended[..]), &Set::new(n), true)?;
    let mut fallen = HashSet::new();
    let mut scratch = Set::new(n);
    let mut reads = reach.reads.iter().peekable();
    for (b, block) in f.cfg.blocks.iter().enumerate() {
        let mut standing = ins.get(b)?.clone();
        for k in 0..=block.stmts.len() {
            while let Some((_, _, id)) = reads.next_if(|(at, ..)| *at == (b, k)) {
                let web = reach.web[*id];
                if wanted.get(&web).is_some_and(|pair| !standing.has(*pair)) {
                    fallen.insert(web);
                }
            }
            if let Some(stmt) = block.stmts.get(k) {
                pairs.pass(stmt, &mut standing, &mut scratch);
            }
        }
    }
    // A read the walk did not come to proves nothing of its web.
    fallen.extend(reads.map(|(_, _, id)| reach.web[*id]));
    Some(fallen)
}

/// One round of the rewrites over what the assignments reach now.
fn sweep(f: &mut cfg::Function, constants: bool, copies: bool) -> bool {
    let facts = Facts::of(f);
    let Some(reach) = Reach::of(f, &facts) else {
        return false;
    };
    let first = reach.vars.len();
    let one_cell = |var: Var| {
        let many = (f.head.vars.get(&var)).is_some_and(|info| info.width == Width::Cells);
        !many && !facts.widths.contains_key(&var)
    };
    let alike = |a: Var, b: Var| {
        let wide = facts.widths.contains_key(&a) && facts.widths.get(&a) == facts.widths.get(&b);
        (one_cell(a) && one_cell(b)) || wide
    };

    // What every assignment of each web gives, where each gives the same constant or the same variable.
    // A web that holds the value at the start is its own root and is never looked at, for the roots are the lowest.
    let mut read = vec![false; reach.defs.len()];
    for (_, _, id) in &reach.reads {
        read[reach.web[*id]] = true;
    }
    let mut gives: Vec<Option<Option<Expr>>> = vec![None; reach.defs.len()];
    for (id, (var, at)) in reach.defs.iter().copied().enumerate().skip(first) {
        let web = reach.web[id];
        // What operators make of constants is a constant too: the same wherever it is evaluated.
        let value = (at.and_then(|site| given(f, site, reach.vars[var])))
            .filter(|value| matches!(value, Expr::Var(_)) || constant(value));
        gives[web] = match (&gives[web], value) {
            (None, Some(value)) => Some(Some(value.clone())),
            (Some(Some(known)), Some(value)) if known == value => continue,
            _ => Some(None),
        };
    }

    // The webs replaced, the variables that lose a web and those that take a web's reads.
    let mut replaced: HashMap<usize, Expr> = HashMap::new();
    let mut losing: HashSet<Var> = HashSet::new();
    let mut taking: HashSet<Var> = HashSet::new();
    let mut pairs = Pairs::default();
    let mut wanted: HashMap<usize, usize> = HashMap::new();
    let mut copied: Vec<(usize, Var, Var)> = Vec::new();
    for web in first..reach.defs.len() {
        let var = reach.vars[reach.defs[web].0];
        let whole = reach.web[web] == web && read[web] && !reach.spoiled[reach.defs[web].0];
        match (whole, &gives[web]) {
            (true, Some(Some(value))) if constants && one_cell(var) && constant(value) => {
                replaced.insert(web, value.clone());
                losing.insert(var);
            }
            (true, Some(Some(Expr::Var(source)))) if copies => {
                let source = *source;
                let named = !matches!(source, Var::Global { .. }) && facts.named_only(source);
                // A local keeps its name: it does not become a temporary that holds the same.
                let kept = var.is_temporary() || !source.is_temporary();
                if source != var && named && kept && alike(var, source) {
                    wanted.insert(web, pairs.number(var, source));
                    copied.push((web, var, source));
                }
            }
            _ => {}
        }
    }
    if !copied.is_empty() {
        let fallen = fallen(f, &reach, &pairs, &wanted);
        for (web, var, source) in copied {
            // ⚠ A variable both read in place of one web and losing another would be read
            // where its assignment is gone: such a pair waits for the next round.
            let stands = fallen.as_ref().is_some_and(|fallen| !fallen.contains(&web));
            if stands && !losing.contains(&source) && !taking.contains(&var) {
                replaced.insert(web, Expr::Var(source));
                losing.insert(var);
                taking.insert(source);
            }
        }
    }

    let mut swap: HashMap<(Site, Var), Expr> = HashMap::new();
    for (site, var, id) in &reach.reads {
        if let Some(to) = replaced.get(&reach.web[*id]) {
            swap.insert((*site, reach.vars[*var]), to.clone());
        }
    }
    let mut gone: HashSet<Site> = HashSet::new();
    let mut dead: HashSet<Site> = HashSet::new();
    for (id, (var, at)) in reach.defs.iter().copied().enumerate().skip(first) {
        let Some(site) = at else {
            continue;
        };
        if replaced.contains_key(&reach.web[id]) {
            gone.insert(site);
        } else if copies && !reach.seen[id] && !reach.spoiled[var] {
            dead.insert(site);
        }
    }
    if gone.is_empty() && dead.is_empty() {
        return false;
    }

    let mut changed = !gone.is_empty();
    let rewrite = |top: &mut Expr, site: Site| {
        top.walk_mut(&mut |e| {
            let to = match &*e {
                Expr::Var(var) => swap.get(&(site, *var)).cloned(),
                _ => None,
            };
            if let Some(to) = to {
                *e = to;
            }
        });
    };
    for (b, block) in f.cfg.blocks.iter_mut().enumerate() {
        let old = std::mem::take(&mut block.stmts);
        let last = old.len();
        for (k, mut stmt) in old.into_iter().enumerate() {
            if gone.contains(&(b, k)) {
                continue;
            }
            stmt.each_expr_mut(&mut |top| rewrite(top, (b, k)));
            match stmt {
                // An assignment no read can see leaves what its value does and nothing else.
                Stmt::Assign { target, value } if dead.contains(&(b, k)) => {
                    match leftover(&facts, value) {
                        Ok(None) => changed = true,
                        Ok(Some(call)) => {
                            block.stmts.push(Stmt::Eval(call));
                            changed = true;
                        }
                        Err(value) => block.stmts.push(Stmt::Assign { target, value }),
                    }
                }
                other => block.stmts.push(other),
            }
        }
        if let Some(e) = block.term.expr_mut() {
            rewrite(e, (b, last));
        }
    }
    changed
}

#[cfg(test)]
#[path = "coalesce_tests.rs"]
mod tests;
