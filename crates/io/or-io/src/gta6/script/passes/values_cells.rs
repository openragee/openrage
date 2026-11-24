//! Many-cell values: the cells of one put back together where a listing passes or keeps them in a row.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ops::Range;

use crate::gta6::script::cfg;
use crate::gta6::script::ir::{Const, Expr, Head, Kind, Scalar, Stmt, Var, VarInfo, Width};

use super::effects::{cells_of, each_top, each_top_mut, inert, one_cell, Facts};

/// How many times copies are followed from one run of locals to the next.
const HOPS: usize = 8;

/// The cells of `base` where it is a whole value whose width something states: a temporary, or the zero handle.
fn whole(facts: &Facts, base: &Expr) -> Option<u32> {
    match base {
        Expr::Var(var) if var.is_temporary() => facts.widths.get(var).copied(),
        Expr::GuidEmpty => Some(2),
        _ => None,
    }
}

/// Replaces each run `x[0], x[1], ..` that names every cell of a value `x` in order by `x`.
fn joined(facts: &Facts, items: &mut Vec<Expr>) -> bool {
    let mut changed = false;
    let mut at = 0;
    while at < items.len() {
        let base = match &items[at] {
            Expr::Cell { base, index: 0 } if matches!(**base, Expr::Var(_) | Expr::GuidEmpty) => {
                Some((**base).clone())
            }
            _ => None,
        };
        if let Some(base) = base {
            let cells = whole(facts, &base).map_or(0, |cells| cells as usize);
            let run = items.get(at..at.saturating_add(cells)).is_some_and(|run| {
                run.iter().zip(0u32..).all(|(item, i)| {
                    matches!(item, Expr::Cell { base: of, index } if *index == i && **of == base)
                })
            });
            if cells >= 2 && run {
                items.drain(at + 1..at + cells);
                items[at] = base;
                changed = true;
            }
        }
        at += 1;
    }
    changed
}

/// Passes a value whole wherever a call or a group of cells is given each of its cells in order.
fn runs(f: &mut cfg::Function) -> bool {
    let facts = Facts::of(f);
    let mut changed = false;
    each_top_mut(&mut f.cfg, &mut |top| {
        top.walk_mut(&mut |e| {
            if let Expr::Call { args: items, .. } | Expr::Cells(items) = e {
                changed |= joined(&facts, items);
            }
            // A group made of one whole value is that value.
            let lone = matches!(&*e, Expr::Cells(items) if items.len() == 1 && whole(&facts, &items[0]).is_some());
            if lone {
                if let Expr::Cells(mut items) = std::mem::replace(e, Expr::GuidEmpty) {
                    if let Some(item) = items.pop() {
                        *e = item;
                    }
                }
                changed = true;
            }
        });
    });
    changed
}

/// The cells `extract_bytes(new VmValues(items), offset, bytes)` takes, where it takes whole cells and leaves none that must be evaluated.
fn cut(head: &Head, items: &[Expr], offset: u32, bytes: u32) -> Option<Range<usize>> {
    let (first, count) = (offset as usize / 8, bytes as usize / 8);
    let whole = offset.is_multiple_of(8) && bytes.is_multiple_of(8);
    let aligned = whole && count >= 1 && first + count <= items.len();
    if !aligned || !items.iter().all(|item| one_cell(head, item)) {
        return None;
    }
    let taken = first..first + count;
    let rest_inert = items
        .iter()
        .enumerate()
        .all(|(i, item)| taken.contains(&i) || inert(item));
    rest_inert.then_some(taken)
}

/// The load that cell `index` of `load_bytes(p, offset, bytes)` is: the unsigned type of the bytes that reach the cell, and where they lie.
// The last cell of a load takes what bytes are left, with zeroes above them.
fn cell_load(offset: u32, bytes: u32, index: u32) -> Option<(Scalar, u32)> {
    let at = index.checked_mul(8)?;
    let ty = match bytes.checked_sub(at)? {
        1 => Scalar::U8,
        2 => Scalar::U16,
        4 => Scalar::U32,
        left if left >= 8 => Scalar::U64,
        _ => return None,
    };
    Some((ty, offset.checked_add(at)?))
}

/// Takes whole cells out of a group by picking them, and makes a temporary of one cell a plain one.
fn single(f: &mut cfg::Function) -> bool {
    let mut changed = false;
    let head = &f.head;
    each_top_mut(&mut f.cfg, &mut |top| {
        top.walk_mut(&mut |e| {
            // `extract_bytes(new VmValues(..), offset, bytes)` over whole cells is those cells.
            let taken = match &*e {
                Expr::ExtractBytes {
                    value,
                    offset,
                    bytes,
                } => match &**value {
                    Expr::Cells(items) => cut(head, items, *offset, *bytes),
                    _ => None,
                },
                _ => None,
            };
            if let Some(taken) = taken {
                if let Expr::ExtractBytes { value, .. } = std::mem::replace(e, Expr::GuidEmpty) {
                    if let Expr::Cells(mut items) = *value {
                        *e = Expr::Cells(items.drain(taken).collect());
                    }
                }
                changed = true;
            }
            // `new VmValues(..)[n]` is the cell at `n`.
            let picked = match &*e {
                Expr::Cell { base, index } => match &**base {
                    Expr::Cells(items) => cut(head, items, index.saturating_mul(8), 8),
                    _ => None,
                },
                _ => None,
            };
            if let Some(picked) = picked {
                if let Expr::Cell { base, .. } = std::mem::replace(e, Expr::GuidEmpty) {
                    if let Expr::Cells(mut items) = *base {
                        *e = items.swap_remove(picked.start);
                    }
                }
                changed = true;
            }
            // `load_bytes(p, offset, bytes)[n]` is the bytes loaded into cell `n`, with nothing above them.
            let loaded = match &*e {
                Expr::Cell { base, index } => match &**base {
                    Expr::LoadBytes { offset, bytes, .. } => cell_load(*offset, *bytes, *index),
                    _ => None,
                },
                _ => None,
            };
            if let Some((ty, offset)) = loaded {
                if let Expr::Cell { base, .. } = std::mem::replace(e, Expr::GuidEmpty) {
                    if let Expr::LoadBytes { addr, .. } = *base {
                        *e = Expr::Load { ty, addr, offset };
                    }
                }
                changed = true;
            }
            // A cell of zeroes is zero.
            if matches!(&*e, Expr::Cell { base, .. } if matches!(**base, Expr::ZeroSlots(_))) {
                *e = Expr::Const(Const::i32(0));
                changed = true;
            }
            // The low bits of a group of one cell are that cell's.
            if let Expr::Cast { e: inner, .. } = e {
                let lone = matches!(&**inner, Expr::Cells(items) if items.len() == 1 && one_cell(head, &items[0]));
                if lone {
                    if let Expr::Cells(mut items) = std::mem::replace(&mut **inner, Expr::GuidEmpty)
                    {
                        if let Some(item) = items.pop() {
                            **inner = item;
                        }
                    }
                    changed = true;
                }
            }
        });
    });

    let facts = Facts::of(f);
    // How often each variable is read as its first cell.
    let mut first_cells: HashMap<Var, u32> = HashMap::new();
    each_top(&f.cfg, &mut |top| {
        top.walk(&mut |e| {
            if let Expr::Cell { base, index: 0 } = e {
                if let Expr::Var(var) = &**base {
                    *first_cells.entry(*var).or_default() += 1;
                }
            }
        });
    });
    // A many-cell temporary of one cell, read only as that cell, is the cell.
    let mut plain: HashSet<Var> = HashSet::new();
    for block in &mut f.cfg.blocks {
        for stmt in &mut block.stmts {
            let Stmt::Assign { target, value } = stmt else {
                continue;
            };
            let var = *target;
            let uses = facts.uses.get(&var).copied().unwrap_or_default();
            let many = f
                .head
                .vars
                .get(&var)
                .is_some_and(|info| info.width == Width::Cells);
            let as_cell = uses.reads == first_cells.get(&var).copied().unwrap_or(0);
            if !(var.is_temporary() && many && uses.defs == 1 && uses.addrs == 0 && as_cell) {
                continue;
            }
            let cell = match value {
                Expr::Cells(items) if items.len() == 1 && one_cell(&f.head, &items[0]) => {
                    items.pop()
                }
                Expr::LoadBytes {
                    addr,
                    offset,
                    bytes,
                } if *bytes <= 8 => cell_load(*offset, *bytes, 0).map(|(ty, offset)| Expr::Load {
                    ty,
                    addr: std::mem::replace(addr, Box::new(Expr::GuidEmpty)),
                    offset,
                }),
                _ => None,
            };
            if let Some(cell) = cell {
                *value = cell;
                plain.insert(var);
            }
        }
    }
    if plain.is_empty() {
        return changed;
    }
    each_top_mut(&mut f.cfg, &mut |top| {
        top.walk_mut(&mut |e| {
            let first = matches!(&*e, Expr::Cell { base, index: 0 } if matches!(&**base, Expr::Var(var) if plain.contains(var)));
            if first {
                if let Expr::Cell { base, .. } = std::mem::replace(e, Expr::GuidEmpty) {
                    *e = *base;
                }
            }
        });
    });
    for var in &plain {
        if let Some(info) = f.head.vars.get_mut(var) {
            info.width = Width::Cell;
        }
    }
    true
}

/// `local_to = local_from;`: the two locals.
fn copy(stmt: &Stmt) -> Option<(u32, u32)> {
    match stmt {
        Stmt::Assign {
            target: Var::Local(to),
            value: Expr::Var(Var::Local(from)),
        } => Some((*to, *from)),
        _ => None,
    }
}

/// Whether `stmts` starts with `cells` copies in a row, the locals from `from` on into those from `to` on.
fn copies(stmts: &[Stmt], cells: u32, (to, from): (u32, u32)) -> bool {
    stmts.get(..cells as usize).is_some_and(|run| {
        run.iter().zip(0u32..).all(|(stmt, i)| {
            let wanted = to.checked_add(i).zip(from.checked_add(i));
            wanted.is_some() && copy(stmt) == wanted
        })
    })
}

/// The first local, the cells and the source of a spread `stmts` starts with: `local_k = x[0]; local_k+1 = x[1]; ..` over every cell of `x`.
fn spread<'a>(facts: &Facts, stmts: &'a [Stmt]) -> Option<(u32, u32, &'a Expr)> {
    let Some(Stmt::Assign {
        target: Var::Local(first),
        value: Expr::Cell { base, index: 0 },
    }) = stmts.first()
    else {
        return None;
    };
    let cells = whole(facts, base)?;
    if cells < 2 {
        return None;
    }
    let rest = stmts.get(1..cells as usize)?;
    for (stmt, i) in rest.iter().zip(1u32..) {
        let Stmt::Assign {
            target: Var::Local(local),
            value: Expr::Cell { base: of, index },
        } = stmt
        else {
            return None;
        };
        if Some(*local) != first.checked_add(i) || *index != i || of != base {
            return None;
        }
    }
    Some((*first, cells, &**base))
}

/// The group, and how many items it takes, where `items[at..]` starts with every local of one in order.
fn local_run(groups: &BTreeMap<u32, u32>, items: &[Expr], at: usize) -> Option<(u32, usize)> {
    let Expr::Var(Var::Local(first)) = items.get(at)? else {
        return None;
    };
    let cells = *groups.get(first)? as usize;
    let run = items.get(at..at.saturating_add(cells))?;
    let whole = run.iter().zip(0u32..).all(|(item, i)| {
        matches!(item, Expr::Var(Var::Local(local)) if Some(*local) == first.checked_add(i))
    });
    whole.then_some((*first, cells))
}

/// `zero_bytes(&local_k, 8n);` over two cells or more: the first local and the cells cleared.
fn cleared(stmt: &Stmt) -> Option<(u32, u32)> {
    match stmt {
        Stmt::ZeroBytes {
            addr:
                Expr::AddrOf {
                    var: Var::Local(first),
                    cell: 0,
                },
            bytes,
        } if *bytes >= 16 && bytes.is_multiple_of(8) => Some((*first, bytes / 8)),
        _ => None,
    }
}

/// A run of statements that assigns every local of a group, as the one assignment it is.
struct Whole {
    first: u32,
    /// The statements the run takes.
    len: usize,
    /// Whether the run is a clear through the group's own address, which is no assignment by name.
    clear: bool,
    /// The group copied from, where the run is a copy.
    from: Option<u32>,
    value: Expr,
}

/// The assignment of a whole group that `stmts` starts with: a spread of a value over it, or a copy of another group.
fn whole_write(facts: &Facts, groups: &BTreeMap<u32, u32>, stmts: &[Stmt]) -> Option<Whole> {
    if let Some((first, cells)) = stmts.first().and_then(cleared) {
        if groups.get(&first) == Some(&cells) {
            return Some(Whole {
                first,
                len: 1,
                clear: true,
                from: None,
                value: Expr::ZeroSlots(cells),
            });
        }
    }
    if let Some((first, cells, source)) = spread(facts, stmts) {
        if groups.get(&first) == Some(&cells) {
            return Some(Whole {
                first,
                len: cells as usize,
                clear: false,
                from: None,
                value: source.clone(),
            });
        }
    }
    let (to, from) = copy(stmts.first()?)?;
    let cells = *groups.get(&to)?;
    let copied = groups.get(&from) == Some(&cells) && copies(stmts, cells, (to, from));
    copied.then_some(Whole {
        first: to,
        len: cells as usize,
        clear: false,
        from: Some(from),
        value: Expr::Var(Var::Local(from)),
    })
}

/// The runs of locals proposed as one variable each, by first local, and those two proposals disagree on.
#[derive(Default)]
struct Proposed {
    cells: BTreeMap<u32, u32>,
    refused: BTreeSet<u32>,
}

impl Proposed {
    /// Notes that the locals from `first` on hold a value of `cells` cells; says whether that is news.
    fn propose(&mut self, first: u32, cells: u32) -> bool {
        match self.cells.get(&first).copied() {
            Some(known) if known == cells => false,
            Some(_) => {
                self.refused.insert(first);
                false
            }
            None => {
                self.cells.insert(first, cells);
                true
            }
        }
    }
}

/// Makes one variable of each run of locals that only ever takes, holds and gives a many-cell value whole.
// The run is a notation for the same frame cells: it is made only where every read and every
// write of each of its locals is of the whole run, so no reader of one cell is left behind.
fn locals(f: &mut cfg::Function) -> bool {
    let facts = Facts::of(f);
    let mut proposed = Proposed::default();
    for block in &f.cfg.blocks {
        for at in 0..block.stmts.len() {
            if let Some((first, cells, _)) = spread(&facts, &block.stmts[at..]) {
                proposed.propose(first, cells);
            }
            // A clear of whole cells says how far a value runs, as a spread does.
            if let Some((first, cells)) = cleared(&block.stmts[at]) {
                proposed.propose(first, cells);
            }
        }
    }
    if proposed.cells.is_empty() {
        return false;
    }
    // A copy from one run of locals to another carries the shape across.
    for _ in 0..HOPS {
        let mut grew = false;
        for block in &f.cfg.blocks {
            for at in 0..block.stmts.len() {
                let Some((to, from)) = copy(&block.stmts[at]) else {
                    continue;
                };
                let known = proposed
                    .cells
                    .get(&to)
                    .or_else(|| proposed.cells.get(&from));
                let Some(cells) = known.copied() else {
                    continue;
                };
                if copies(&block.stmts[at..], cells, (to, from)) {
                    grew |= proposed.propose(to, cells);
                    grew |= proposed.propose(from, cells);
                }
            }
        }
        if !grew {
            break;
        }
    }

    // Runs that overlap one another, or locals already made one variable, are left alone.
    let Proposed { cells, mut refused } = proposed;
    let end = |first: u32, cells: u32| u64::from(first) + u64::from(cells);
    let made: Vec<(u32, u32)> = f
        .head
        .vars
        .iter()
        .filter_map(|(var, info)| match (var, &info.kind) {
            (Var::Local(first), Kind::Aggregate(cells)) => Some((*first, *cells)),
            _ => None,
        })
        .collect();
    // The furthest end of the runs before this one, and whose it is.
    let mut reach: Option<(u64, u32)> = None;
    for (first, width) in &cells {
        let start = u64::from(*first);
        if let Some((_, whose)) = reach.filter(|(far, _)| start < *far) {
            refused.insert(whose);
            refused.insert(*first);
        }
        let taken = made.iter().any(|(other, cells)| {
            start < end(*other, *cells) && u64::from(*other) < end(*first, *width)
        });
        if taken {
            refused.insert(*first);
        }
        let mine = end(*first, *width);
        if reach.is_none_or(|(far, _)| mine > far) {
            reach = Some((mine, *first));
        }
    }
    let live: BTreeMap<u32, u32> = cells
        .into_iter()
        .filter(|(first, _)| !refused.contains(first))
        .collect();
    if live.is_empty() {
        return false;
    }

    // How often each run is written whole and read whole, and the copies that tie two runs together.
    let mut whole_writes: HashMap<u32, u32> = HashMap::new();
    let mut clears: HashMap<u32, u32> = HashMap::new();
    let mut whole_reads: HashMap<u32, u32> = HashMap::new();
    let mut ties: Vec<(u32, u32)> = Vec::new();
    for block in &f.cfg.blocks {
        let mut at = 0;
        while at < block.stmts.len() {
            match whole_write(&facts, &live, &block.stmts[at..]) {
                Some(write) => {
                    match write.clear {
                        true => *clears.entry(write.first).or_default() += 1,
                        false => *whole_writes.entry(write.first).or_default() += 1,
                    }
                    if let Some(from) = write.from {
                        *whole_reads.entry(from).or_default() += 1;
                        ties.push((write.first, from));
                    }
                    at += write.len;
                }
                None => at += 1,
            }
        }
    }
    each_top(&f.cfg, &mut |top| {
        top.walk(&mut |e| {
            if let Expr::Call { args: items, .. } | Expr::Cells(items) = e {
                let mut at = 0;
                while at < items.len() {
                    match local_run(&live, items, at) {
                        Some((first, len)) => {
                            *whole_reads.entry(first).or_default() += 1;
                            at += len;
                        }
                        None => at += 1,
                    }
                }
            }
        });
    });

    let used = |local: u32| {
        let uses = facts.uses.get(&Var::Local(local));
        uses.copied().unwrap_or_default()
    };
    let mut valid: BTreeMap<u32, u32> = BTreeMap::new();
    for (first, width) in &live {
        let reads = whole_reads.get(first).copied().unwrap_or(0);
        let writes = whole_writes.get(first).copied().unwrap_or(0);
        let top = first.saturating_add(*width - 1);
        // Each local is read and written only with the others, and no address points into the run.
        let together = (0..*width).all(|i| {
            let uses = used(first.saturating_add(i));
            uses.reads == reads && uses.defs == writes && (i == 0 || uses.addrs == 0)
        });
        // A load or a store through an address reaches all inside the run or all outside it.
        let unsplit = facts
            .direct_locals
            .iter()
            .all(|(lo, hi)| *hi < *first || *lo > top || (*lo >= *first && *hi <= top));
        if together && unsplit {
            valid.insert(*first, *width);
        }
    }
    // A copy counted as whole on both sides holds only while both runs do.
    loop {
        let before = valid.len();
        for (to, from) in &ties {
            if !(valid.contains_key(to) && valid.contains_key(from)) {
                valid.remove(to);
                valid.remove(from);
            }
        }
        if valid.len() == before {
            break;
        }
    }
    if valid.is_empty() {
        return false;
    }

    for block in &mut f.cfg.blocks {
        let mut merged: Vec<(usize, Whole)> = Vec::new();
        let mut at = 0;
        while at < block.stmts.len() {
            match whole_write(&facts, &valid, &block.stmts[at..]) {
                Some(write) => {
                    let len = write.len;
                    merged.push((at, write));
                    at += len;
                }
                None => at += 1,
            }
        }
        // From the last back, so that the places before stay where they were found.
        for (at, write) in merged.into_iter().rev() {
            block.stmts.drain(at + 1..at + write.len);
            block.stmts[at] = Stmt::Assign {
                target: Var::Local(write.first),
                value: write.value,
            };
        }
    }
    each_top_mut(&mut f.cfg, &mut |top| {
        top.walk_mut(&mut |e| {
            if let Expr::Call { args: items, .. } | Expr::Cells(items) = e {
                let mut at = 0;
                while at < items.len() {
                    if let Some((first, len)) = local_run(&valid, &items[..], at) {
                        items.drain(at + 1..at + len);
                        items[at] = Expr::Var(Var::Local(first));
                    }
                    at += 1;
                }
            }
            let group = match &*e {
                // Every byte of the run, loaded through its own address, is the run.
                Expr::LoadBytes {
                    addr,
                    offset: 0,
                    bytes,
                } => match &**addr {
                    Expr::AddrOf {
                        var: Var::Local(first),
                        cell: 0,
                    } => valid
                        .get(first)
                        .filter(|cells| u64::from(*bytes) == 8 * u64::from(**cells))
                        .map(|_| *first),
                    _ => None,
                },
                Expr::Cells(items) => match &items[..] {
                    [Expr::Var(Var::Local(first))] if valid.contains_key(first) => Some(*first),
                    _ => None,
                },
                _ => None,
            };
            if let Some(first) = group {
                *e = Expr::Var(Var::Local(first));
            }
        });
    });
    for (first, width) in &valid {
        let info = VarInfo {
            width: Width::Cells,
            kind: Kind::Aggregate(*width),
            name: None,
        };
        f.head.vars.insert(Var::Local(*first), info);
    }
    true
}

/// Whether a statement names `var` at all: assigns it, reads it or takes its address.
fn mentions(stmt: &Stmt, var: Var) -> bool {
    let mut found = stmt.targets().contains(&var);
    stmt.each_expr(&mut |top| {
        top.walk(&mut |e| {
            found |=
                matches!(e, Expr::Var(named) | Expr::AddrOf { var: named, .. } if *named == var);
        });
    });
    found
}

/// The value that fills every cell of the temporary statement `made` zeroes, and the store that writes it.
// `v = zero_slots(2); ..; store_bytes(&v, 0, 16, x);` with nothing between that names `v`, an
// address that goes nowhere else and an `x` of exactly those cells: `v` never holds anything
// but `x`.
fn filled(facts: &Facts, head: &Head, stmts: &[Stmt], made: usize) -> Option<(usize, Expr)> {
    let Stmt::Assign {
        target,
        value: Expr::ZeroSlots(cells),
    } = stmts.get(made)?
    else {
        return None;
    };
    let uses = facts.uses.get(target).copied()?;
    let lone = matches!(target, Var::Value { .. }) && uses.defs == 1 && uses.addrs == 1;
    let at = (made + 1..stmts.len()).find(|at| mentions(&stmts[*at], *target))?;
    let Stmt::StoreBytes {
        addr: Expr::AddrOf { var, cell: 0 },
        offset: 0,
        bytes,
        value,
    } = &stmts[at]
    else {
        return None;
    };
    let width = match value {
        Expr::Cells(items) if items.iter().all(|item| one_cell(head, item)) => {
            u32::try_from(items.len()).ok()
        }
        other => cells_of(other, &facts.widths),
    };
    let whole = u64::from(*bytes) == 8 * u64::from(*cells) && width == Some(*cells);
    let mut reads_itself = false;
    value.walk(&mut |e| reads_itself |= matches!(e, Expr::Var(named) if named == target));
    (lone && var == target && whole && !reads_itself).then(|| (at, value.clone()))
}

/// Assigns a temporary the value that a store fills every zeroed cell of it with.
fn fills(f: &mut cfg::Function) -> bool {
    let facts = Facts::of(f);
    let head = &f.head;
    let mut changed = false;
    for block in &mut f.cfg.blocks {
        let mut i = 0;
        while i < block.stmts.len() {
            let Some((at, value)) = filled(&facts, head, &block.stmts, i) else {
                i += 1;
                continue;
            };
            // With statement `i` gone, the store stands one place nearer.
            if let Stmt::Assign { target, .. } = block.stmts.remove(i) {
                block.stmts[at - 1] = Stmt::Assign { target, value };
            }
            changed = true;
        }
    }
    changed
}

/// Puts the cells of many-cell values back together: in what is passed, in temporaries of one cell, and in locals.
pub(super) fn gather(f: &mut cfg::Function) -> bool {
    let mut changed = runs(f);
    changed |= fills(f);
    changed |= single(f);
    changed |= locals(f);
    changed
}
