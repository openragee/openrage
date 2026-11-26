//! What a frame holds across an await: the check that the one body the awaits are read as reads
//! no variable a resumed call would find empty.

use std::collections::BTreeMap;

use crate::gta6::script::cfg::{Block, Cfg, ENTRY};
use crate::gta6::script::ir::{Expr, Stmt, Var};
use crate::gta6::script::passes::forward::flow::{self, Set};

use super::{constant, framed, Reach, Site};

/// The sweeps given to finding what each block may still read of the frame, and the words its sets may take; past either the machine is left as it is.
const LIVE_SWEEPS: usize = 64;
const LIVE_WORDS: usize = 1 << 20;

/// The variables of the frame a block mentions, each with a number; a kick-off's machine and the variable that holds its address are none.
fn frame_of(blocks: &[Block], reach: Reach) -> BTreeMap<Var, usize> {
    let mut frame = BTreeMap::new();
    let mut note = |var: Var| {
        if framed(var) && var != reach.pointer && Some(var) != reach.storage {
            let next = frame.len();
            frame.entry(var).or_insert(next);
        }
    };
    for block in blocks {
        for stmt in &block.stmts {
            stmt.targets().iter().for_each(|var| note(*var));
            stmt.each_expr(&mut |top| {
                top.walk(&mut |e| {
                    if let Expr::Var(var) | Expr::AddrOf { var, .. } = e {
                        note(*var);
                    }
                });
            });
        }
        if let Some(test) = block.term.expr() {
            test.walk(&mut |e| {
                if let Expr::Var(var) | Expr::AddrOf { var, .. } = e {
                    note(*var);
                }
            });
        }
    }
    frame
}

/// The variable of the frame a statement writes through that variable's own address, and the expression that is the address.
fn own_store(stmt: &Stmt) -> Option<(Var, &Expr)> {
    let (Stmt::Store { addr, .. }
    | Stmt::StoreBytes { addr, .. }
    | Stmt::StoreF32Slots { addr, .. }) = stmt
    else {
        return None;
    };
    let mut root = addr;
    while let Expr::ByteOffset {
        addr: under,
        offset,
    } = root
    {
        constant(offset)?;
        root = &**under;
    }
    match root {
        Expr::AddrOf { var, .. } if framed(*var) => Some((*var, root)),
        _ => None,
    }
}

/// Whether the one body the awaits are read as holds, after each await, what a resumed call holds there.
// ⚠ A resumed call starts on a new frame: the entry and the resume point run and every other
// variable is zero, where the one body still holds what was written before the await. So no
// variable written on a way to an await may be read after it, but the awaiter the suspend path
// kept and one the entry alone writes. A write is an assignment, a store through the
// variable's own address, or that address handed on, which whatever takes it may write
// through; a clear through the address sets the variable back to what a new frame holds.
// ⚠ What a kick-off's entry does before the step's own reads does not run at a resumed call:
// only a variable those reads set, from statement `cut` of the entry on, is set again there.
pub(super) fn holds(cfg: &Cfg, sites: &[Site], reach: Reach, cut: usize) -> bool {
    let blocks = &cfg.blocks;
    let frame = frame_of(blocks, reach);
    let n = frame.len();
    if n == 0 {
        return true;
    }
    if blocks.len().saturating_mul(n.div_ceil(64)) > LIVE_WORDS {
        return false;
    }
    let mut written = vec![0u32; n];
    let none = || vec![Set::new(n); blocks.len()];
    // What each block reads of what it is entered with and what it sets whole; what it leaves written and what it leaves cleared.
    let (mut reads, mut sets, mut made, mut ended) = (none(), none(), none(), none());
    for (b, block) in blocks.iter().enumerate() {
        let (read, set) = (&mut reads[b], &mut sets[b]);
        let (made, ended) = (&mut made[b], &mut ended[b]);
        let mut write = |at: usize, made: &mut Set, ended: &mut Set| {
            written[at] = written[at].saturating_add(1);
            made.insert(at);
            ended.remove(at);
        };
        for stmt in &block.stmts {
            // A clear of a local runs on into the locals after it.
            if let Stmt::ZeroBytes {
                addr: Expr::AddrOf { var, cell: 0 },
                bytes,
            } = stmt
            {
                let last = match var {
                    Var::Local(first) => {
                        Var::Local(first.saturating_add(bytes.div_ceil(8).saturating_sub(1)))
                    }
                    other => *other,
                };
                for (_, at) in frame.range(*var..=last) {
                    set.insert(*at);
                    made.remove(*at);
                    ended.insert(*at);
                }
                continue;
            }
            let stored = own_store(stmt);
            let mut named: Vec<(Var, bool)> = Vec::new();
            stmt.each_expr(&mut |top| {
                top.walk(&mut |e| match e {
                    Expr::Var(var) => named.push((*var, false)),
                    // The address a store goes through reads nothing of what it stores into.
                    Expr::AddrOf { .. }
                        if stored.is_some_and(|(_, root)| std::ptr::eq(root, e)) => {}
                    Expr::AddrOf { var, .. } => named.push((*var, true)),
                    _ => {}
                });
            });
            for (var, by_address) in named {
                if let Some(at) = frame.get(&var) {
                    if !set.has(*at) {
                        read.insert(*at);
                    }
                    if by_address {
                        write(*at, made, ended);
                    }
                }
            }
            if let Some(at) = stored.and_then(|(var, _)| frame.get(&var)) {
                write(*at, made, ended);
            }
            for var in stmt.targets() {
                if let Some(at) = frame.get(var) {
                    set.insert(*at);
                    write(*at, made, ended);
                }
            }
        }
        if let Some(test) = block.term.expr() {
            test.walk(&mut |e| {
                let (var, by_address) = match e {
                    Expr::Var(var) => (var, false),
                    Expr::AddrOf { var, .. } => (var, true),
                    _ => return,
                };
                if let Some(at) = frame.get(var) {
                    if !set.has(*at) {
                        read.insert(*at);
                    }
                    if by_address {
                        write(*at, made, ended);
                    }
                }
            });
        }
    }
    let mut reread = Set::new(n);
    for stmt in blocks[ENTRY].stmts.iter().skip(cut) {
        for at in stmt.targets().iter().filter_map(|var| frame.get(var)) {
            reread.insert(*at);
        }
    }
    // What a block may read of what it is entered with: its own reads, and those of what follows that it does not set first.
    let mut live = vec![Set::new(n); blocks.len()];
    let mut settled = false;
    for _ in 0..LIVE_SWEEPS {
        let mut changed = false;
        for b in (0..blocks.len()).rev() {
            let mut entered = Set::new(n);
            for to in blocks[b].term.successors() {
                if let Some(next) = live.get(to) {
                    entered.add(next);
                }
            }
            entered.without(&sets[b]);
            entered.add(&reads[b]);
            if entered != live[b] {
                live[b] = entered;
                changed = true;
            }
        }
        if !changed {
            settled = true;
            break;
        }
    }
    // What may have been written on some way to each block, and not cleared since.
    let Some(before) = flow::entering(cfg, n, (&made[..], &ended[..]), &Set::new(n), false) else {
        return false;
    };
    settled
        && sites.iter().all(|site| {
            let mut held = before[site.test].clone();
            held.without(&ended[site.test]);
            held.add(&made[site.test]);
            frame.iter().all(|(var, at)| {
                let entered = reread.has(*at) && written[*at] == 1;
                !live[site.on].has(*at)
                    || match sets[site.resume].has(*at) {
                        // ⚠ What the resume point sets is hidden in the one body: only the awaiter is the same there.
                        true => site.kept == Some(*var),
                        false => entered || !held.has(*at),
                    }
            })
        })
}
