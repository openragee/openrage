//! The blocks before they are laid out: a join that only tests what its predecessors hand it is
//! tested by them, a tree of tests of one value is the switch it was, and tests that share a
//! target are one condition.
//!
//! ⚠ Each rewrite removes only blocks that hold no statement and that nothing else leads to, so
//! every statement stays in the block it was in and runs on the paths it ran on.

use std::collections::HashMap;

use crate::gta6::script::cfg::{Block, BlockId, Terminator, ENTRY};
use crate::gta6::script::ir::{BinOp, Expr, Head, Stmt, Var, Width};

use super::cond::{self, Test};

/// The tests one condition may gather: `&&`, `||` and `?:` nest one level for each.
const MAX_OPERANDS: u32 = 64;
/// The tests one switch may be read from, and the blocks a scan for one answer may pass.
pub(super) const SCAN: usize = 4096;
/// The cases a tree of tests must tell apart to be a switch; fewer read better as `if`.
const MIN_CASES: usize = 3;

/// The blocks that jump to each block, each once, whether or not the entry reaches them.
pub(super) fn predecessors(blocks: &[Block]) -> Vec<Vec<BlockId>> {
    let mut preds = vec![Vec::new(); blocks.len()];
    for (b, block) in blocks.iter().enumerate() {
        for s in block.term.successors() {
            preds[s].push(b);
        }
    }
    preds
}

/// The blocks `order` names, in that order, every jump following its block.
// ⚠ A block left out must be one no block kept jumps to.
pub(super) fn reordered(blocks: Vec<Block>, order: &[BlockId]) -> Vec<Block> {
    let mut renumbered = vec![usize::MAX; blocks.len()];
    for (new, old) in order.iter().enumerate() {
        renumbered[*old] = new;
    }
    let mut taken: Vec<Option<Block>> = blocks.into_iter().map(Some).collect();
    let mut out = Vec::with_capacity(order.len());
    for old in order {
        if let Some(mut block) = taken[*old].take() {
            block.term.retarget(|target| renumbered[target]);
            out.push(block);
        }
    }
    out
}

fn without(blocks: &mut Vec<Block>, dead: &[bool]) {
    if dead.iter().any(|dead| *dead) {
        let order: Vec<BlockId> = (0..blocks.len()).filter(|b| !dead[*b]).collect();
        *blocks = reordered(std::mem::take(blocks), &order);
    }
}

fn lead(preds: &mut [Vec<BlockId>], from: BlockId, to: BlockId) {
    if !preds[to].contains(&from) {
        preds[to].push(from);
    }
}

/// How often a function reads and assigns a temporary.
#[derive(Clone, Copy, Default)]
struct Uses {
    reads: u32,
    writes: u32,
}

fn count_reads(top: &Expr, uses: &mut HashMap<Var, Uses>) {
    top.walk(&mut |e| {
        if let Expr::Var(var) | Expr::AddrOf { var, .. } = e {
            if var.is_temporary() {
                uses.entry(*var).or_default().reads += 1;
            }
        }
    });
}

fn uses_of(blocks: &[Block]) -> HashMap<Var, Uses> {
    let mut uses: HashMap<Var, Uses> = HashMap::new();
    for block in blocks {
        for stmt in &block.stmts {
            // A tuple's target is no assignment of its own: counted as a read too, it is never the one read.
            let tuple = matches!(stmt, Stmt::AssignMany { .. });
            for target in stmt.targets() {
                if target.is_temporary() {
                    let entry = uses.entry(*target).or_default();
                    entry.writes += 1;
                    entry.reads += u32::from(tuple);
                }
            }
            stmt.each_expr(&mut |top| count_reads(top, &mut uses));
        }
        if let Some(top) = block.term.expr() {
            count_reads(top, &mut uses);
        }
    }
    uses
}

/// A join that only tests the one value its predecessors hand it: each predecessor tests the value itself, and the join and its variable go.
// Only where the value is all a predecessor holds, so that what is left is a test or a jump:
// `a && b` yields its value this way, and the tests then join into that condition.
// ⚠ The variable must be read by that test alone and assigned by those predecessors alone; an
// address taken of it counts as a read.
pub(super) fn thread_joins(blocks: &mut Vec<Block>, head: &mut Head) {
    let n = blocks.len();
    let mut facts: Option<HashMap<Var, Uses>> = None;
    let mut preds: Vec<Vec<BlockId>> = Vec::new();
    let mut dead = vec![false; n];
    for j in 0..n {
        if j == ENTRY || dead[j] {
            continue;
        }
        let (var, then, els) = match &blocks[j] {
            Block {
                stmts,
                term: Terminator::Branch { cond, then, els },
                ..
            } if stmts.is_empty() && then != els => match cond::tested_temporary(cond) {
                Some(var) => (var, *then, *els),
                None => continue,
            },
            _ => continue,
        };
        if head.vars.get(&var).map(|info| info.width) != Some(Width::Cell) {
            continue;
        }
        if facts.is_none() {
            facts = Some(uses_of(blocks));
            preds = predecessors(blocks);
        }
        let Some(used) = facts.as_ref().and_then(|facts| facts.get(&var)).copied() else {
            continue;
        };
        let joined = &preds[j];
        let counted = used.reads == 1 && used.writes as usize == joined.len();
        // A join that leads back to what leads to it is left alone.
        let apart = !joined.contains(&j) && !joined.contains(&then) && !joined.contains(&els);
        let handed = joined.iter().all(|p| {
            let only = matches!(
                blocks[*p].stmts.as_slice(),
                [Stmt::Assign { target, .. }] if *target == var
            );
            !dead[*p] && only && matches!(&blocks[*p].term, Terminator::Jump(to) if *to == j)
        });
        let threads = counted && apart && handed && !joined.is_empty();
        if !threads {
            continue;
        }
        let Terminator::Branch { cond, .. } =
            std::mem::replace(&mut blocks[j].term, Terminator::Unreachable)
        else {
            continue;
        };
        // What a constant makes of the test is known here; a constant it cannot be worked out for keeps the join.
        let mut decided = Vec::with_capacity(joined.len());
        let mut sure = true;
        for p in joined {
            let outcome = match blocks[*p].stmts.first() {
                Some(Stmt::Assign {
                    value: Expr::Const(c),
                    ..
                }) => {
                    let holds = cond::holds_for(&cond, c.bits);
                    sure &= holds.is_some();
                    holds
                }
                _ => None,
            };
            decided.push(outcome);
        }
        if !sure {
            blocks[j].term = Terminator::Branch { cond, then, els };
            continue;
        }
        let joined = std::mem::take(&mut preds[j]);
        for (p, outcome) in joined.into_iter().zip(decided) {
            let Some(Stmt::Assign { value, .. }) = blocks[p].stmts.pop() else {
                continue;
            };
            match outcome {
                Some(holds) => {
                    let to = if holds { then } else { els };
                    blocks[p].term = Terminator::Jump(to);
                    if p == ENTRY {
                        lead(&mut preds, p, to);
                        continue;
                    }
                    // The block holds nothing now: what led to it leads on.
                    for q in std::mem::take(&mut preds[p]) {
                        blocks[q]
                            .term
                            .retarget(|target| if target == p { to } else { target });
                        lead(&mut preds, q, to);
                    }
                    dead[p] = true;
                }
                None => {
                    let tested = cond::simplify(cond::with_value(&cond, var, value));
                    blocks[p].term = Terminator::Branch {
                        cond: tested,
                        then,
                        els,
                    };
                    lead(&mut preds, p, then);
                    lead(&mut preds, p, els);
                }
            }
        }
        for to in [then, els] {
            preds[to].retain(|p| *p != j);
        }
        dead[j] = true;
        head.vars.remove(&var);
    }
    without(blocks, &dead);
}

fn is_inner(
    blocks: &[Block],
    tests: &[Option<Test>],
    preds: &[Vec<BlockId>],
    x: BlockId,
    parent: BlockId,
) -> bool {
    let (Some(test), Some(above)) = (&tests[x], &tests[parent]) else {
        return false;
    };
    // The same bits compared: one test of a byte of the variable and one of all of it are two values.
    let same = cond::under_wide_casts(&test.tested) == cond::under_wide_casts(&above.tested);
    x != ENTRY && same && blocks[x].stmts.is_empty() && preds[x] == [parent]
}

/// Where a jump to `x` ends up once blocks that only jump on are passed.
fn through(blocks: &[Block], mut x: BlockId) -> BlockId {
    for _ in 0..64 {
        match &blocks[x] {
            Block {
                stmts,
                term: Terminator::Jump(next),
                ..
            } if x != ENTRY && stmts.is_empty() && *next != x => x = *next,
            _ => break,
        }
    }
    x
}

/// A tree of tests of one variable against constants, as a compiler lays a sparse `switch` out, becomes that switch.
// The tree's inner tests hold no statement and are reached from the tree alone, and the variable
// is read and nothing else, so reading it once is reading it at every test.
// ⚠ Exact only because every way out of the tree other than by an equality goes to one block:
// a value no case names then reaches that block, and what each constant reaches is worked out by
// running the tests on it.
pub(super) fn switch_trees(blocks: &mut Vec<Block>) {
    let n = blocks.len();
    let mut tests: Vec<Option<Test>> = blocks
        .iter()
        .map(|block| match &block.term {
            Terminator::Branch { cond, then, els } if then != els => Test::of(cond),
            _ => None,
        })
        .collect();
    if tests.iter().all(Option::is_none) {
        return;
    }
    let mut preds = predecessors(blocks);
    let mut dead = vec![false; n];
    // The tree each block was last gathered into.
    let mut tree = vec![usize::MAX; n];
    let mut roots: Vec<BlockId> = (0..n)
        .rev()
        .filter(|r| {
            let inner = preds[*r].len() == 1 && is_inner(blocks, &tests, &preds, *r, preds[*r][0]);
            tests[*r].is_some() && !inner
        })
        .collect();
    let mut id = 0usize;
    while let Some(r) = roots.pop() {
        if dead[r] || tests[r].is_none() {
            continue;
        }
        id += 1;
        let mut nodes = vec![r];
        tree[r] = id;
        let mut next = 0;
        while next < nodes.len() && nodes.len() < SCAN {
            let x = nodes[next];
            next += 1;
            for y in blocks[x].term.successors() {
                if tree[y] != id && is_inner(blocks, &tests, &preds, y, x) {
                    tree[y] = id;
                    nodes.push(y);
                }
            }
        }

        // Every way out that is not an equality must be to one block, the default.
        let mut default: Option<BlockId> = None;
        let mut fits = true;
        let mut constants: Vec<u32> = Vec::new();
        let mut arms: Vec<BlockId> = Vec::new();
        for x in &nodes {
            let (Some(test), Terminator::Branch { then, els, .. }) = (&tests[*x], &blocks[*x].term)
            else {
                fits = false;
                break;
            };
            if test.is_equality() {
                constants.push(test.constant);
            }
            let equal = match test.op {
                BinOp::Eq => Some(*then),
                BinOp::Ne => Some(*els),
                _ => None,
            };
            for y in [*then, *els] {
                if tree[y] == id {
                    continue;
                }
                arms.push(y);
                if Some(y) == equal {
                    continue;
                }
                let leaf = through(blocks, y);
                match default {
                    None => default = Some(leaf),
                    Some(d) => fits &= d == leaf,
                }
            }
        }
        constants.sort_unstable();
        constants.dedup();
        let mut cases: Vec<(i64, BlockId)> = Vec::new();
        if let (true, Some(d)) = (fits, default) {
            for k in &constants {
                let mut x = r;
                for _ in 0..=nodes.len() {
                    let (Some(test), Terminator::Branch { then, els, .. }) =
                        (&tests[x], &blocks[x].term)
                    else {
                        break;
                    };
                    if tree[x] != id {
                        break;
                    }
                    x = if test.holds(*k) { *then } else { *els };
                }
                if tree[x] == id {
                    fits = false;
                    break;
                }
                let leaf = through(blocks, x);
                if leaf != d {
                    cases.push((i64::from(*k as i32), leaf));
                }
            }
        }
        let mut targets: Vec<BlockId> = cases.iter().map(|case| case.1).collect();
        targets.sort_unstable();
        targets.dedup();
        let (true, Some(default), Some(test)) = (
            fits && cases.len() >= MIN_CASES && targets.len() >= 2,
            default,
            tests[r].clone(),
        ) else {
            // The tests under this one may still be a switch of their own.
            for y in blocks[r].term.successors() {
                if is_inner(blocks, &tests, &preds, y, r) {
                    roots.push(y);
                }
            }
            continue;
        };

        cases.sort_unstable();
        for x in &nodes {
            for s in blocks[*x].term.successors() {
                preds[s].retain(|p| p != x);
            }
            tests[*x] = None;
        }
        for x in nodes.iter().skip(1) {
            dead[*x] = true;
            blocks[*x].term = Terminator::Unreachable;
        }
        blocks[r].term = Terminator::Switch {
            value: test.tested,
            cases,
            default,
        };
        for s in blocks[r].term.successors() {
            lead(&mut preds, r, s);
        }
        // A block that only jumped on, and that the tree alone led to, is led to by nothing now.
        for mut x in arms {
            while x != ENTRY && !dead[x] && preds[x].is_empty() && blocks[x].stmts.is_empty() {
                let on = match &blocks[x].term {
                    Terminator::Jump(on) => *on,
                    _ => break,
                };
                dead[x] = true;
                preds[on].retain(|p| *p != x);
                x = on;
            }
        }
    }
    without(blocks, &dead);
}

/// A switch on `x - k` reads as a switch on `x`.
pub(super) fn unbias_switches(blocks: &mut [Block]) {
    for block in blocks {
        if let Terminator::Switch { value, cases, .. } = &mut block.term {
            let labels: Vec<i64> = cases.iter().map(|case| case.0).collect();
            if let Some((plain, moved)) = cond::unbiased(value, &labels) {
                *value = plain;
                for (case, label) in cases.iter_mut().zip(moved) {
                    case.0 = label;
                }
            }
        }
    }
}

/// The two blocks a block that holds only a test leads to, where `b` alone leads to that block.
fn only_test(
    blocks: &[Block],
    preds: &[Vec<BlockId>],
    dead: &[bool],
    b: BlockId,
    x: BlockId,
) -> Option<(BlockId, BlockId)> {
    if x == b || x == ENTRY || dead[x] || preds[x] != [b] || !blocks[x].stmts.is_empty() {
        return None;
    }
    match &blocks[x].term {
        Terminator::Branch { then, els, .. } if then != els => Some((*then, *els)),
        _ => None,
    }
}

/// The condition of a block that ends in a branch, taken out of it.
fn take_test(block: &mut Block) -> Option<Expr> {
    match std::mem::replace(&mut block.term, Terminator::Unreachable) {
        Terminator::Branch { cond, .. } => Some(cond),
        other => {
            block.term = other;
            None
        }
    }
}

/// Joins the test of `b` with a test one of its arms leads to; `false` when no arm is one.
fn merge_at(
    blocks: &mut [Block],
    preds: &mut [Vec<BlockId>],
    dead: &mut [bool],
    weight: &mut [u32],
    b: BlockId,
) -> bool {
    let (a, z) = match &blocks[b].term {
        Terminator::Branch { then, els, .. } if then != els => (*then, *els),
        _ => return false,
    };
    let under_a = only_test(blocks, preds, dead, b, a);
    let under_z = only_test(blocks, preds, dead, b, z);

    // Both arms are tests that leave to the same two blocks: `c ? a : z`.
    if let (Some((a1, a2)), Some((z1, z2))) = (under_a, under_z) {
        let straight = a1 == z1 && a2 == z2;
        let crossed = a1 == z2 && a2 == z1;
        let apart = ![a, z].contains(&a1) && ![a, z].contains(&a2);
        let light = weight[b] + weight[a] + weight[z] <= MAX_OPERANDS;
        if (straight || crossed) && apart && light {
            let tests = (
                take_test(&mut blocks[b]),
                take_test(&mut blocks[a]),
                take_test(&mut blocks[z]),
            );
            let (Some(c), Some(ca), Some(cz)) = tests else {
                return false;
            };
            let cz = if straight { cz } else { cond::negate(cz) };
            blocks[b].term = Terminator::Branch {
                cond: cond::select(c, ca, cz),
                then: a1,
                els: a2,
            };
            for s in [a1, a2] {
                preds[s].retain(|p| *p != a && *p != z);
                lead(preds, b, s);
            }
            dead[a] = true;
            dead[z] = true;
            weight[b] += weight[a] + weight[z];
            return true;
        }
    }

    for (x, under, other, on_then) in [(a, under_a, z, true), (z, under_z, a, false)] {
        let Some((x1, x2)) = under else {
            continue;
        };
        if (x1 != other && x2 != other) || weight[b] + weight[x] > MAX_OPERANDS {
            continue;
        }
        let (Some(c), Some(cx)) = (take_test(&mut blocks[b]), take_test(&mut blocks[x])) else {
            return false;
        };
        // `x` is reached exactly when `c` sends control to it, so its test runs when it ran before.
        let (cond, then, els) = match (on_then, x1 == other, x2 == other) {
            // b: c ? x : other, x: cx ? x1 : other
            (true, _, true) => (cond::and(c, cx), x1, other),
            // b: c ? x : other, x: cx ? other : x2
            (true, _, false) => (cond::and(c, cond::negate(cx)), x2, other),
            // b: c ? other : x, x: cx ? other : x2
            (false, true, _) => (cond::or(c, cx), other, x2),
            // b: c ? other : x, x: cx ? x1 : other
            (false, false, _) => (cond::or(c, cond::negate(cx)), other, x1),
        };
        blocks[b].term = Terminator::Branch { cond, then, els };
        for s in [x1, x2] {
            preds[s].retain(|p| *p != x);
            lead(preds, b, s);
        }
        dead[x] = true;
        weight[b] += weight[x];
        return true;
    }
    false
}

/// Two tests that share a target become one condition: `a && b`, `a || b`, `c ? a : b`.
// Safe because the second block holds nothing but its test and only the first leads to it.
pub(super) fn merge_conditions(blocks: &mut Vec<Block>) {
    let n = blocks.len();
    let mut preds = predecessors(blocks);
    let mut dead = vec![false; n];
    let mut weight = vec![1u32; n];
    // Later blocks first: a test joined below is then one test for the block above it.
    for b in (0..n).rev() {
        while !dead[b] && merge_at(blocks, &mut preds, &mut dead, &mut weight, b) {}
    }
    without(blocks, &dead);
}
