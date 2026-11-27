//! A row of temporaries, each read once by the statement straight after the row, put back into
//! that statement where it reads them in the order they were assigned.

use crate::gta6::script::cfg::{self, Block};
use crate::gta6::script::ir::{BinOp, Expr, Head, Stmt, Var, Width};

use super::super::effects::{depth, evaluated, Facts};
use super::{guarded, reads_in, DEPTH};

/// The most assignments of one row that are put back into the statement after it.
const ROW: usize = 16;

/// The most nodes a statement may evaluate for the values before it to be weighed against them.
const NODES: usize = 4096;

/// Whether evaluating `e` itself, apart from its operands, does nothing and reads nothing a value evaluated before it may change.
// A constant, the address of a variable, an operator that cannot fault, or a read of a variable
// that only its own assignments change and that no address reaches.
// ⚠ A frame variable of several cells is not taken at its word: a pass that made it may have left
// its width unstated, and then what reaches its later cells is not seen.
fn still(facts: &Facts, head: &Head, e: &Expr) -> bool {
    match e {
        Expr::Var(var) => {
            let wide = (head.vars.get(var)).is_some_and(|info| info.width == Width::Cells)
                || facts.widths.contains_key(var);
            let held = var.is_temporary() || !wide;
            !matches!(var, Var::Global { .. }) && held && facts.named_only(*var)
        }
        Expr::Binary { op, .. } => !matches!(op, BinOp::Div | BinOp::Rem),
        Expr::Const(_)
        | Expr::GuidEmpty
        | Expr::ZeroSlots(_)
        | Expr::AddrOf { .. }
        | Expr::FunctionRef { .. }
        | Expr::BuiltinFunctionRef { .. }
        | Expr::Cell { .. }
        | Expr::Cells(_)
        | Expr::Cast { .. }
        | Expr::Truth(_)
        | Expr::Unary { .. }
        | Expr::ExtractBytes { .. }
        | Expr::ByteOffset { .. }
        | Expr::Vector3(_)
        | Expr::Select { .. } => true,
        _ => false,
    }
}

/// Whether `order`, the nodes a statement evaluates in turn, reads each of `row` in the order of `row`, and every other node before the last of those reads is [`still`].
fn fits(facts: &Facts, head: &Head, order: &[&Expr], row: &[Var]) -> bool {
    let mut next = 0;
    for e in order {
        let Some(wanted) = row.get(next) else {
            break;
        };
        match e {
            Expr::Var(var) if var == wanted => next += 1,
            Expr::Var(var) if row.contains(var) => return false,
            other => {
                if !still(facts, head, other) {
                    return false;
                }
            }
        }
    }
    next == row.len()
}

/// How many times site `k` of `block` reads `var`: statement `k`, or past the last the terminator.
fn site_reads(block: &Block, k: usize, var: Var) -> u32 {
    let mut found = 0;
    match block.stmts.get(k) {
        Some(stmt) => stmt.each_expr(&mut |top| found += reads_in(top, var)),
        None => {
            if let Some(e) = block.term.expr() {
                found += reads_in(e, var);
            }
        }
    }
    found
}

/// Writes in `top` the value of each of `values` where `top` reads its variable, taking it out of `values`.
fn put(top: &mut Expr, values: &mut [(Var, Option<Expr>)]) {
    top.walk_mut(&mut |e| {
        let value = match &*e {
            Expr::Var(read) => (values.iter_mut())
                .find(|(var, _)| var == read)
                .and_then(|(_, value)| value.take()),
            _ => None,
        };
        if let Some(value) = value {
            *e = value;
        }
    });
}

/// Moves into site `k` of `block` the longest row of assignments straight before it that the site can take; where the site then stands.
fn moved_row(facts: &Facts, head: &Head, block: &mut Block, k: usize) -> Option<usize> {
    // The temporaries assigned straight before the site, each once, read once and there.
    let mut row: Vec<Var> = Vec::new();
    let mut first = k;
    while first > 0 && row.len() < ROW {
        let Some(Stmt::Assign { target, value }) = block.stmts.get(first - 1) else {
            break;
        };
        let uses = facts.uses.get(target).copied().unwrap_or_default();
        let lone = uses.defs == 1 && uses.reads == 1 && uses.addrs == 0;
        let fresh = reads_in(value, *target) == 0;
        if !target.is_temporary() || !lone || !fresh || site_reads(block, k, *target) != 1 {
            break;
        }
        row.push(*target);
        first -= 1;
    }
    if row.is_empty() {
        return None;
    }
    row.reverse();

    let mut tops: Vec<&Expr> = Vec::new();
    match block.stmts.get(k) {
        Some(stmt) => stmt.each_expr(&mut |top| tops.push(top)),
        None => tops.extend(block.term.expr()),
    }
    let mut order = Vec::new();
    for top in &tops {
        evaluated(top, &mut order);
    }
    if order.len() > NODES {
        return None;
    }
    let deepest = tops.iter().copied().map(depth).max().unwrap_or(0);
    let deep = |from: usize| {
        let values = block.stmts.get(from..k).unwrap_or_default().iter();
        let each = values.map(|stmt| match stmt {
            Stmt::Assign { value, .. } => depth(value),
            _ => 0,
        });
        each.max().unwrap_or(0)
    };
    // The longest tail of the row the site can take: an earlier value may have to stay where a
    // later one is read first.
    let skip = (0..row.len()).find(|skip| {
        let part = &row[*skip..];
        let armed = part
            .iter()
            .any(|var| tops.iter().any(|top| guarded(top, *var)));
        let fits_depth = deepest + deep(first + skip) <= DEPTH;
        !armed && fits_depth && fits(facts, head, &order, part)
    })?;
    let from = first + skip;

    // Into a copy of the site first, so that the row goes only where the site takes all of it.
    let mut values: Vec<(Var, Option<Expr>)> = (block.stmts.get(from..k))
        .unwrap_or_default()
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::Assign { target, value } => Some((*target, Some(value.clone()))),
            _ => None,
        })
        .collect();
    if values.len() != k - from {
        return None;
    }
    match block.stmts.get(k).cloned() {
        Some(mut stmt) => {
            stmt.each_expr_mut(&mut |top| put(top, &mut values));
            if values.iter().any(|(_, value)| value.is_some()) {
                return None;
            }
            let slot = block.stmts.get_mut(k)?;
            *slot = stmt;
        }
        None => {
            let mut e = block.term.expr()?.clone();
            put(&mut e, &mut values);
            if values.iter().any(|(_, value)| value.is_some()) {
                return None;
            }
            let slot = block.term.expr_mut()?;
            *slot = e;
        }
    }
    block.stmts.drain(from..k);
    Some(from)
}

/// Lets the statement after a row of temporaries, each assigned once and read once there, evaluate their values itself, where it reads them in the order they were assigned and does nothing that shows before the last.
// `v = load(p, 8); w = f(); store_bytes(v, 0x278, 48, w);` is `store_bytes(load(p, 8), 0x278,
// 48, f());`: the values are evaluated in the order they were, before what the statement does,
// and what it evaluates between them neither does anything nor reads what they may change.
// Nothing between two values is weighed but that: for the rows a pass that makes one variable of
// several cells leaves once the last fold is over.
pub(in crate::gta6::script::passes) fn chained(f: &mut cfg::Function) -> bool {
    let facts = Facts::of(f);
    let head = &f.head;
    let mut changed = false;
    for block in &mut f.cfg.blocks {
        // A move takes a statement away and looks at the site again; a site that takes nothing is passed.
        let mut k = 0;
        while k <= block.stmts.len() {
            match moved_row(&facts, head, block, k) {
                Some(at) => {
                    k = at;
                    changed = true;
                }
                None => k += 1,
            }
        }
    }
    changed
}

#[cfg(test)]
#[path = "values_fold_chain_tests.rs"]
mod tests;
