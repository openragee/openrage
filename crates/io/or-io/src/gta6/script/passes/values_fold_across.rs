//! A temporary's value moved to its one read in a later block, where every way on from the
//! assignment comes there once and nothing on the way forbids.

use std::collections::HashMap;

use crate::gta6::script::cfg::{self, dominates, Block, BlockId};
use crate::gta6::script::ir::{Expr, Stmt, Var};

use super::super::effects::{depth, evaluated, Effects, Facts};
use super::{guarded, reads_in, site, site_mut, Site, DEPTH, REACH};

/// The most blocks that may stand between a value's assignment and the block that reads it.
const BETWEEN: usize = 64;

/// The blocks every way on from the end of block `b` goes through before it comes to block `c`; none where a way may leave the function, come back to `b`, go round a loop or come to `c` otherwise.
// `looped` holds each block an edge enters from a block no earlier in reverse post-order. A way
// from `c` back to `c` or to a block between that does not pass `b` would have its earliest block
// so entered: every block it holds past `c` is one `c` dominates, which comes after `c` in that
// order.
fn between(
    idom: &[Option<BlockId>],
    succs: &[Vec<BlockId>],
    looped: &[bool],
    b: BlockId,
    c: BlockId,
) -> Option<Vec<BlockId>> {
    let circled = |x: BlockId| looped.get(x).copied().unwrap_or(true);
    if b == c || circled(c) || !dominates(idom, b, c) {
        return None;
    }
    let mut stack: Vec<BlockId> = succs.get(b)?.clone();
    if stack.is_empty() {
        return None;
    }
    let mut region: Vec<BlockId> = Vec::new();
    while let Some(x) = stack.pop() {
        if x == c || region.contains(&x) {
            continue;
        }
        let next = succs.get(x)?;
        if x == b || next.is_empty() || circled(x) || region.len() >= BETWEEN {
            return None;
        }
        region.push(x);
        stack.extend(next.iter().copied());
    }
    Some(region)
}

/// Moves the value statement `i` of block `b` assigns to `var` into site `k` of block `c`, its one read, where every statement and test between lets it pass.
fn moved_across(
    facts: &Facts,
    stmts: &mut [Vec<Option<Stmt>>],
    blocks: &mut [Block],
    effects: &mut [Vec<Option<Effects>>],
    region: &[BlockId],
    ((b, i), var, (c, k)): (Site, Var, Site),
) -> bool {
    let Some(Some(Stmt::Assign { value, .. })) = stmts.get(b).and_then(|kept| kept.get(i)) else {
        return false;
    };
    let moving = Effects::of_expr(facts, value);
    let deep = depth(value);

    // The rest of `b` and the test it ends on, each block between whole, and `c` up to the read.
    let length = |x: BlockId| stmts.get(x).map_or(0, Vec::len);
    let mut spans: Vec<(BlockId, usize, usize, bool)> = vec![(b, i + 1, length(b), true)];
    spans.extend(region.iter().map(|x| (*x, 0, length(*x), true)));
    spans.push((c, 0, k.min(length(c)), false));
    let mut passed = 0usize;
    for (x, lo, hi, test) in spans {
        let (Some(kept), Some(known)) = (stmts.get(x), effects.get_mut(x)) else {
            return false;
        };
        for j in lo..hi {
            let Some(Some(stmt)) = kept.get(j) else {
                continue;
            };
            passed += 1;
            let Some(slot) = known.get_mut(j) else {
                return false;
            };
            if passed > REACH {
                return false;
            }
            let crossed = slot.get_or_insert_with(|| Effects::of_stmt(facts, stmt).placed((x, j)));
            if !moving.may_cross(crossed) {
                return false;
            }
        }
        let tested = blocks.get(x).and_then(|block| block.term.expr());
        if let (true, Some(e)) = (test, tested) {
            if !moving.may_cross(&Effects::of_expr(facts, e).placed((x, hi))) {
                return false;
            }
        }
    }

    // What the reading site evaluates before it comes to the temporary is passed over too.
    let (Some(kept), Some(block)) = (stmts.get(c), blocks.get(c)) else {
        return false;
    };
    let mut order = Vec::new();
    let mut deepest = 0;
    let mut skipped = false;
    site(kept, &block.term, k, &mut |top| {
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
    // In an arm that may not run, only what could as well not be evaluated may sit.
    let placed = !skipped || moving.free();
    if !found || deepest + deep > DEPTH || !placed || !moving.may_cross(&before) {
        return false;
    }

    // The reading site is there to take the value before the value leaves its statement.
    if stmts.get(c).is_none() || blocks.get(c).is_none() {
        return false;
    }
    let Some(slot) = stmts.get_mut(b).and_then(|kept| kept.get_mut(i)) else {
        return false;
    };
    let value = match slot.take() {
        Some(Stmt::Assign { value, .. }) => value,
        other => {
            *slot = other;
            return false;
        }
    };
    let (Some(kept), Some(block)) = (stmts.get_mut(c), blocks.get_mut(c)) else {
        return false;
    };
    let mut value = Some(value);
    site_mut(kept, &mut block.term, k, &mut |top| {
        top.walk_mut(&mut |e| {
            if matches!(e, Expr::Var(read) if *read == var) {
                if let Some(value) = value.take() {
                    *e = value;
                }
            }
        });
    });
    // Not taken, the value goes back to its statement.
    if let Some(value) = value.take() {
        if let Some(slot) = stmts.get_mut(b).and_then(|kept| kept.get_mut(i)) {
            *slot = Some(Stmt::Assign { target: var, value });
        }
        return false;
    }
    if let Some(stale) = effects.get_mut(c).and_then(|known| known.get_mut(k)) {
        *stale = None;
    }
    true
}

/// Moves the value of each temporary assigned once and read once, in a later block, into the expression that reads it; `finer` tells two accesses apart as [`Facts::finer`] does.
// `v = E;` in block `b` and its read in block `c` are one evaluation where `b` dominates `c`,
// every way on from `b` comes to `c` with no way out, no block on those ways is come to again
// before `b` is, and `E` may pass every statement and test on them. Then `c` runs once after
// each run of `b`, as `E` did, and on no other way.
pub(in crate::gta6::script::passes) fn across(f: &mut cfg::Function, finer: bool) -> bool {
    let facts = match finer {
        true => Facts::finer(f),
        false => Facts::of(f),
    };
    let mut assigned: HashMap<Var, Site> = HashMap::new();
    for (b, block) in f.cfg.blocks.iter().enumerate() {
        for (i, stmt) in block.stmts.iter().enumerate() {
            let Stmt::Assign { target, value } = stmt else {
                continue;
            };
            let uses = facts.uses.get(target).copied().unwrap_or_default();
            let lone = uses.defs == 1 && uses.reads == 1 && uses.addrs == 0;
            if target.is_temporary() && lone && reads_in(value, *target) == 0 {
                assigned.insert(*target, (b, i));
            }
        }
    }
    if assigned.is_empty() {
        return false;
    }
    // Each of them whose one read is in another block, with where it is read.
    let mut wanted: Vec<(Site, Var, Site)> = Vec::new();
    for (c, block) in f.cfg.blocks.iter().enumerate() {
        for k in 0..=block.stmts.len() {
            let mut see = |top: &Expr| {
                top.walk(&mut |e| {
                    let Expr::Var(var) = e else {
                        return;
                    };
                    if let Some(at) = assigned.get(var).filter(|at| at.0 != c) {
                        wanted.push((*at, *var, (c, k)));
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
    if wanted.is_empty() {
        return false;
    }

    let n = f.cfg.blocks.len();
    let idom = f.cfg.dominators();
    let mut rank = vec![usize::MAX; n];
    for (at, b) in f.cfg.reverse_post_order().into_iter().enumerate() {
        if let Some(slot) = rank.get_mut(b) {
            *slot = at;
        }
    }
    let preds = f.cfg.predecessors();
    let looped: Vec<bool> = (0..n)
        .map(|w| (preds[w].iter()).any(|p| rank[*p] >= rank[w]))
        .collect();
    let succs: Vec<Vec<BlockId>> = (0..n).map(|b| f.cfg.successors(b)).collect();
    // The latest assignment first: once moved, it no longer stands between an earlier one and its
    // reader.
    wanted.sort_unstable_by_key(|((b, i), _, _)| std::cmp::Reverse((rank[*b], *i)));

    let mut stmts: Vec<Vec<Option<Stmt>>> = (f.cfg.blocks.iter_mut())
        .map(|block| {
            std::mem::take(&mut block.stmts)
                .into_iter()
                .map(Some)
                .collect()
        })
        .collect();
    let mut effects: Vec<Vec<Option<Effects>>> = (stmts.iter())
        .map(|kept| kept.iter().map(|_| None).collect())
        .collect();
    let mut changed = false;
    for (from, var, to) in wanted {
        let Some(region) = between(&idom, &succs, &looped, from.0, to.0) else {
            continue;
        };
        let blocks = &mut f.cfg.blocks;
        let site = (from, var, to);
        changed |= moved_across(&facts, &mut stmts, blocks, &mut effects, &region, site);
    }
    for (block, kept) in f.cfg.blocks.iter_mut().zip(stmts) {
        block.stmts = kept.into_iter().flatten().collect();
    }
    changed
}

#[cfg(test)]
#[path = "values_fold_across_tests.rs"]
mod tests;
