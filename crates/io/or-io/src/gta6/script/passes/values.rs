//! Values: the temporaries a lifter spreads an expression over, put back together.
//!
//! Works on the blocks, before types and structure. A rewrite is made only where what the code
//! computes and the order of its calls, stores and loads can be shown to stay; a temporary no
//! rewrite is sure of stays a variable.

#[path = "values_cells.rs"]
mod cells;
#[path = "values_effects.rs"]
mod effects;
#[path = "values_fold.rs"]
mod fold;
#[path = "values_joins.rs"]
mod joins;

use crate::gta6::script::cfg::{self, Block, BlockId, Cfg, Terminator, ENTRY};
use crate::gta6::script::ir::{BinOp, Expr, Kind, Scalar, Stmt, StoreKind, UnOp, Var};
use crate::gta6::script::names::Names;
use crate::gta6::script::Stats;

/// The rounds of rewrites a function is given: each feeds the next, and few functions gain past the third.
const ROUNDS: usize = 6;

/// Folds each single-use temporary into its use, makes one variable of the values a join carries and one of the cells of a value, and removes what is dead.
// `Stats` counts temporaries on the trees either side of the passes, so nothing is counted here.
pub fn run(f: &mut cfg::Function, _names: &Names, _stats: &mut Stats) {
    if f.head.incomplete.is_some() || f.cfg.blocks.is_empty() {
        return;
    }
    named_accesses(&mut f.cfg);
    tidy(&mut f.cfg);
    joins::split_tuples(&mut f.cfg);
    for _ in 0..ROUNDS {
        let mut changed = fold::dead(f);
        // A cell zeroed, filled by one narrow store and read back is a value: as one it folds like any other.
        changed |= super::types::narrowings(f);
        changed |= fold::zeroes(f);
        changed |= fold::mirror(f);
        changed |= fold::fold(f);
        changed |= fold::copies(f);
        changed |= joins::roots(f);
        changed |= joins::merge(f);
        changed |= cells::gather(f);
        changed |= tidy(&mut f.cfg);
        if !changed {
            break;
        }
    }
    fold::steps(f);
    // `head.vars` declares the temporaries the code still mentions and no other.
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
fn named_accesses(cfg: &mut Cfg) {
    for block in &mut cfg.blocks {
        for stmt in &mut block.stmts {
            let whole = match &*stmt {
                Stmt::Store {
                    kind: StoreKind::U64 | StoreKind::Ref,
                    addr,
                    offset: 0,
                    ..
                } => frame_cell(addr),
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
                } => frame_cell(addr).map(|var| (*ty, var)),
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

/// The cell an expression of constants, integer readings and tests of equality comes to; `None` for any other.
fn constant(e: &Expr, depth: u32) -> Option<u64> {
    let d = depth.checked_sub(1)?;
    Some(match e {
        Expr::Const(c) => c.bits,
        Expr::Truth(e) => u64::from(constant(e, d)? != 0),
        Expr::Unary { op: UnOp::Not, e } => u64::from(constant(e, d)? == 0),
        Expr::Cast { to, e } => {
            let bits = constant(e, d)?;
            match to {
                Scalar::I8 => bits as i8 as i64 as u64,
                Scalar::U8 => u64::from(bits as u8),
                Scalar::I16 => bits as i16 as i64 as u64,
                Scalar::U16 => u64::from(bits as u16),
                Scalar::I32 => bits as i32 as i64 as u64,
                Scalar::U32 => u64::from(bits as u32),
                Scalar::I64 | Scalar::U64 => bits,
                Scalar::F32 | Scalar::F64 => return None,
            }
        }
        Expr::Binary {
            op: op @ (BinOp::Eq | BinOp::Ne),
            at,
            l,
            r,
        } => {
            let (l, r) = (constant(l, d)?, constant(r, d)?);
            let equal = match at {
                Kind::Bool => (l != 0) == (r != 0),
                Kind::I32 | Kind::U32 => l as u32 == r as u32,
                Kind::I64 | Kind::U64 | Kind::Unknown => l == r,
                _ => return None,
            };
            u64::from(equal == (*op == BinOp::Eq))
        }
        _ => return None,
    })
}

/// Removes the blocks nothing reaches and those that only jump on, and joins a block to the one block that leads to it.
fn tidy(cfg: &mut Cfg) -> bool {
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
        // A branch whose arms meet decides nothing, and goes where evaluating its condition does nothing either.
        let met = match &block.term {
            Terminator::Branch { cond, then, els } if then == els && effects::inert(cond) => {
                Some(*then)
            }
            // A test of constants goes one way.
            Terminator::Branch { cond, then, els } => {
                constant(cond, 16).map(|cell| if cell != 0 { *then } else { *els })
            }
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

#[cfg(test)]
#[path = "values_tests.rs"]
mod tests;
