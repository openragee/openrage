//! The order the blocks are structured in: the order a structured source would state them.
//!
//! Every block stands after the block that dominates it, with all it dominates before anything
//! else; a loop's blocks stand together, before what the loop leaves to; and among blocks free to
//! stand either way the earlier by the listing's own offsets stands first. A listing already in
//! that order keeps it, and one whose blocks were hoisted out of place gets them back.
//!
//! ⚠ The order changes how the result reads and never what it does: every jump is still stated,
//! and a wrong guess here costs a `goto`.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::gta6::script::cfg::{Block, BlockId, ENTRY};

use super::graph::predecessors;

/// No block: an unreached block's rank and dominator, a block's loop when it is in none.
const NONE: usize = usize::MAX;

/// The nearest block that dominates both `a` and `b`, by the dominators found so far.
fn meet(idom: &[usize], rank: &[usize], mut a: BlockId, mut b: BlockId) -> BlockId {
    while a != b {
        while rank[a] > rank[b] {
            a = idom[a];
        }
        while rank[b] > rank[a] {
            b = idom[b];
        }
    }
    a
}

/// Whether `a` dominates `b`; a dominator never ranks after what it dominates.
fn dominates(idom: &[usize], rank: &[usize], a: BlockId, mut b: BlockId) -> bool {
    while a != b {
        if b == ENTRY || rank[b] < rank[a] {
            return false;
        }
        b = idom[b];
    }
    true
}

/// Whether the loop headed by `header` holds `b`.
fn holds(inner: &[usize], outer: &[usize], header: BlockId, b: BlockId) -> bool {
    let mut l = inner[b];
    while l != NONE {
        if l == header {
            return true;
        }
        l = outer[l];
    }
    false
}

/// Every block once, in the order to structure them in; the entry first.
pub(super) fn order(blocks: &[Block]) -> Vec<BlockId> {
    let n = blocks.len();
    if n == 0 {
        return Vec::new();
    }
    let preds = predecessors(blocks);

    // Where each block stands by the offsets the labels state: a block with none stands with
    // the label before it. A flat listing is in this order already.
    let mut key = vec![-1i64; n];
    let mut at = -1i64;
    for (b, block) in blocks.iter().enumerate().skip(1) {
        if let Some(offset) = block.at {
            at = i64::from(offset);
        }
        key[b] = at;
    }
    let mut by: Vec<BlockId> = (0..n).collect();
    by.sort_by_key(|b| (key[*b], *b));
    let mut stand = vec![0usize; n];
    for (i, b) in by.iter().enumerate() {
        stand[*b] = i;
    }

    // The edges that go forward: all but those to a block still being walked.
    let mut state = vec![0u8; n];
    let mut waits = vec![0u32; n];
    let mut forward: Vec<Vec<BlockId>> = vec![Vec::new(); n];
    let mut walk = vec![(ENTRY, blocks[ENTRY].term.successors(), 0usize)];
    state[ENTRY] = 1;
    while let Some(top) = walk.last_mut() {
        let from = top.0;
        match top.1.get(top.2).copied() {
            Some(s) => {
                top.2 += 1;
                if state[s] == 1 {
                    continue;
                }
                forward[from].push(s);
                waits[s] += 1;
                if state[s] == 0 {
                    state[s] = 1;
                    walk.push((s, blocks[s].term.successors(), 0));
                }
            }
            None => {
                state[from] = 2;
                walk.pop();
            }
        }
    }

    // The reached blocks, each after every block that jumps forward to it, and of those free to
    // come next the one that stands first.
    let mut rank = vec![NONE; n];
    let mut reached: Vec<BlockId> = Vec::with_capacity(n);
    let mut ready = BinaryHeap::new();
    ready.push(Reverse((stand[ENTRY], ENTRY)));
    while let Some(Reverse((_, b))) = ready.pop() {
        rank[b] = reached.len();
        reached.push(b);
        for s in forward[b].iter().copied() {
            waits[s] -= 1;
            if waits[s] == 0 {
                ready.push(Reverse((stand[s], s)));
            }
        }
    }

    let mut idom = vec![NONE; n];
    idom[ENTRY] = ENTRY;
    let mut changed = true;
    while changed {
        changed = false;
        for b in reached.iter().copied().skip(1) {
            let mut new = NONE;
            for p in preds[b].iter().copied() {
                if rank[p] != NONE && idom[p] != NONE {
                    new = match new {
                        NONE => p,
                        other => meet(&idom, &rank, p, other),
                    };
                }
            }
            if new != NONE && idom[b] != new {
                idom[b] = new;
                changed = true;
            }
        }
    }

    // The loops, inner before outer: each block's innermost, each loop's enclosing one.
    let mut inner = vec![NONE; n];
    let mut outer = vec![NONE; n];
    for h in reached.iter().rev().copied() {
        let mut work: Vec<BlockId> = preds[h]
            .iter()
            .copied()
            .filter(|p| rank[*p] != NONE && rank[*p] >= rank[h] && dominates(&idom, &rank, h, *p))
            .collect();
        if work.is_empty() {
            continue;
        }
        inner[h] = h;
        while let Some(x) = work.pop() {
            if x == h {
                continue;
            }
            let top = match inner[x] {
                NONE => {
                    inner[x] = h;
                    x
                }
                found => {
                    // Already in a loop found earlier: that loop, at its outermost, is inside this one.
                    let mut top = found;
                    while outer[top] != NONE {
                        top = outer[top];
                    }
                    if top == h {
                        continue;
                    }
                    outer[top] = h;
                    top
                }
            };
            work.extend(preds[top].iter().copied().filter(|p| rank[*p] != NONE));
        }
    }
    // How far each loop reaches in the listing's own order.
    let mut extent = vec![0usize; n];
    for b in reached.iter().copied() {
        let mut l = inner[b];
        while l != NONE {
            extent[l] = extent[l].max(stand[b]);
            l = outer[l];
        }
    }

    // Each block under the block that dominates it. A block a loop leaves to, which the listing
    // has past the loop's last block, goes under the loop's header to follow the whole loop;
    // one the listing has inside the loop's extent stays where it is left from.
    let mut inside: Vec<Vec<BlockId>> = vec![Vec::new(); n];
    let mut after: Vec<Vec<BlockId>> = vec![Vec::new(); n];
    for c in reached.iter().copied().skip(1) {
        let mut parent = match idom[c] {
            NONE => ENTRY,
            found => found,
        };
        let mut left = false;
        let mut l = inner[parent];
        while l != NONE && !holds(&inner, &outer, l, c) && stand[c] > extent[l] {
            parent = l;
            left = true;
            l = outer[l];
        }
        match left {
            true => after[parent].push(c),
            false => inside[parent].push(c),
        }
    }

    let mut order = Vec::with_capacity(n);
    let mut next = vec![ENTRY];
    while let Some(x) = next.pop() {
        order.push(x);
        // Code nothing reaches stays behind the block it followed.
        let mut dead = x + 1;
        while dead < n && rank[dead] == NONE {
            order.push(dead);
            dead += 1;
        }
        next.extend(after[x].iter().rev().copied());
        next.extend(inside[x].iter().rev().copied());
    }
    order
}
