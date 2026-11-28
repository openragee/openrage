//! Structure: blocks back into ifs, loops and switches.
//!
//! `graph` rewrites the blocks where a compiler spread one construct over several, `layout` puts
//! them in the order a source would state them, and `region` writes each jump as the construct
//! that explains it or as a `goto`.
//!
//! ⚠ No block is copied and none that holds a statement is dropped, so every statement comes out
//! once. `Stats::gotos_out` counts the jumps left as `goto`, on the finished tree.
//!
//! The pass `gotos` adds the rules that take out a `goto` the above leaves: a sparse switch whose
//! halves each end in a `return` of their own in `graph`, four choices of region in `region`,
//! and the rewrites of the finished statements in `exits` and `tails`. ⚠ With it on, a tail of a
//! few statements that leaves is written at each `goto` to it, so a statement may come out more
//! than once, which `Stats::statements_out` then counts. So may a `switch` body of a few
//! assignments and a statement that leaves, written again at the end of a body that runs into it.
//!
//! With the pass `for` on, a loop whose last `if` steps its counter in one arm and leaves in the
//! other puts the arm that leaves first, so that the source level reads the loop as a `for`.
//!
//! The pass `conditions` writes an `if` whose two arms only give one variable a truth as that
//! assignment (`x = a || b`, `x = c ? 1 : 0`), and an `if` that holds only an `if` as `a && b`.

#[path = "structure_cond.rs"]
mod cond;
#[path = "structure_exits.rs"]
mod exits;
#[path = "structure_graph.rs"]
mod graph;
#[path = "structure_layout.rs"]
mod layout;
#[path = "structure_region.rs"]
mod region;
#[path = "structure_tails.rs"]
mod tails;

use std::collections::{HashMap, HashSet};

use crate::gta6::script::cfg;
use crate::gta6::script::ir::{Expr, Returns};
use crate::gta6::script::names::Names;
use crate::gta6::script::options::Pass;
use crate::gta6::script::tree::{self, Label, Stmt};
use crate::gta6::script::{Context, Stats};

/// The sweeps given to dropping what the sweep before left unreachable.
const SWEEPS: usize = 4;

/// The times the rules of `exits` go over a body: one rewrite may lay bare the next.
const ROUNDS: usize = 4;

/// Rebuilds `if`, `while`, `do`, `for` and `switch` from the blocks, with a `goto` only where no construct explains a jump.
pub fn run(f: cfg::Function, names: &Names, stats: &mut Stats) -> tree::Function {
    run_in(f, &Context::baseline(names), stats)
}

/// [`run`] as the pipeline calls it: the pass `gotos` of `cx` adds the rules that take out the jumps `run` leaves.
pub fn run_in(f: cfg::Function, cx: &Context<'_>, _stats: &mut Stats) -> tree::Function {
    let gotos = cx.options.on(Pass::Gotos);
    let alone = match gotos {
        true => exits::alone(&f),
        false => HashSet::new(),
    };
    let cfg::Function {
        mut head,
        cfg: flow,
    } = f;
    let mut blocks = flow.blocks;
    if blocks.is_empty() {
        return tree::Function {
            head,
            body: Vec::new(),
        };
    }
    graph::thread_joins(&mut blocks, &mut head);
    graph::switch_trees(&mut blocks, gotos);
    graph::unbias_switches(&mut blocks);
    graph::merge_conditions(&mut blocks);
    let order = layout::order(&blocks);
    let blocks = graph::reordered(blocks, &order);
    let (mut body, mut named) = region::structured(blocks, gotos);
    tidy(&mut body, &mut named);
    // Only a function that still jumps has anything for the rules over statements.
    if gotos && named.values().any(|count| *count > 0) {
        if tails::next_gotos(&mut body, &mut named) {
            tidy(&mut body, &mut named);
        }
        for _ in 0..ROUNDS {
            let private = exits::Private::of(&head, &body, &alone);
            if !exits::rewritten(&mut body, &mut named, Some(&private)) {
                break;
            }
            tidy(&mut body, &mut named);
        }
        if tails::copied(&mut body, &mut named) {
            tidy(&mut body, &mut named);
        }
    }
    // A body that runs into the next is a `goto case` in C#, whether or not a label is named.
    if gotos {
        exits::fallen(&mut body);
    }
    if cx.options.on(Pass::Conditions) {
        cond::truths(&mut body);
    }
    // Only for the `for` it lets the source level read: the loop is no nearer the source without one.
    if cx.options.on(Pass::For) {
        exits::steps_last(&mut body);
    }
    plain_defaults(&mut body);
    // A void function returns by running off its end.
    if head.returns == Returns::Void && matches!(body.last(), Some(Stmt::Return(None))) {
        body.pop();
    }
    tree::Function { head, body }
}

/// Whether a statement only leaves: nothing written after it in its list runs unless a label is there.
fn leaves(stmt: &Stmt) -> bool {
    matches!(
        stmt,
        Stmt::Return(_) | Stmt::Break | Stmt::Continue | Stmt::Goto(_) | Stmt::Unreachable
    )
}

/// The lists of statements nested in `stmt`.
fn nested(stmt: &mut Stmt) -> Vec<&mut Vec<Stmt>> {
    match stmt {
        Stmt::If { then, els, .. } => vec![then, els],
        Stmt::While { body, .. } | Stmt::DoWhile { body, .. } | Stmt::For { body, .. } => {
            vec![body]
        }
        Stmt::Switch { cases, .. } => cases.iter_mut().map(|case| &mut case.body).collect(),
        _ => Vec::new(),
    }
}

/// Drops each label no `goto` names and each jump nothing reaches, and turns an `if` with an empty arm in front of a full one.
// A jump right after a statement that only leaves is reached by nothing: a label would stand
// between them otherwise. Dropping a `goto` may leave its label unnamed, hence the sweeps.
fn tidy(body: &mut Vec<Stmt>, named: &mut HashMap<Label, u32>) {
    for _ in 0..SWEEPS {
        let mut dropped = false;
        let mut work: Vec<&mut Vec<Stmt>> = vec![&mut *body];
        while let Some(list) = work.pop() {
            let mut kept: Vec<Stmt> = Vec::with_capacity(list.len());
            for stmt in list.drain(..) {
                if let Stmt::Label(label) = &stmt {
                    if named.get(label).copied().unwrap_or(0) == 0 {
                        continue;
                    }
                }
                if leaves(&stmt) && kept.last().is_some_and(leaves) {
                    if let Stmt::Goto(label) = &stmt {
                        if let Some(count) = named.get_mut(label) {
                            *count = count.saturating_sub(1);
                        }
                        dropped = true;
                    }
                    continue;
                }
                kept.push(stmt);
            }
            *list = kept;
            for stmt in list {
                if let Stmt::If { cond, then, els } = &mut *stmt {
                    if then.is_empty() && !els.is_empty() {
                        let test = std::mem::replace(cond, Expr::GuidEmpty);
                        *cond = cond::negate(test);
                        std::mem::swap(then, els);
                    }
                }
                work.extend(nested(stmt));
            }
        }
        if !dropped {
            break;
        }
    }
}

/// Drops a `default` that only leaves the switch: a value no case names leaves it anyway.
// Only where nothing runs into that case from the one before it.
fn plain_defaults(body: &mut Vec<Stmt>) {
    let mut work: Vec<&mut Vec<Stmt>> = vec![body];
    while let Some(list) = work.pop() {
        for stmt in list {
            if let Stmt::Switch { cases, .. } = &mut *stmt {
                let plain = cases.iter().position(|case| {
                    case.labels.contains(&None) && matches!(case.body.as_slice(), [Stmt::Break])
                });
                // The case before runs into this one unless it ends by leaving.
                let entered = match plain {
                    Some(at) if at > 0 => !cases[at - 1].body.last().is_some_and(leaves),
                    _ => false,
                };
                if let (Some(at), false) = (plain, entered) {
                    cases.remove(at);
                }
            }
            work.extend(nested(stmt));
        }
    }
}

#[cfg(test)]
#[path = "structure_tests.rs"]
mod tests;
