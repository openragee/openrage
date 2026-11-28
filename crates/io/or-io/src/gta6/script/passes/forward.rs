//! Forward: a load of what the function itself just stored reads the value stored, and a load
//! nothing reads goes.
//!
//! Works on the blocks, in rounds with `values` and `coalesce`. A store is forwarded to a load
//! only where nothing between them can write the place: no call, no store through an address
//! that may be the same, no address of it handed out.

#[path = "forward_flow.rs"]
pub(super) mod flow;

use std::collections::{HashMap, HashSet};

use crate::gta6::script::cfg::{self, dominates, BlockId, Terminator};
use crate::gta6::script::ir::{BinOp, Const, Expr, Scalar, Stmt, StoreKind, Var, Width};
use crate::gta6::script::{Context, Pass, Stats};

use self::flow::Set;
use super::values::effects::{each_top, evaluated, stored, sure_loads, Effects, Facts, Place};
use super::values::fold::{across, constant, low_clears, made};

/// The names this file's rewrites are switched by.
pub const PASSES: &[Pass] = &[Pass::Forward, Pass::DeadLoads];

/// The dominators the proof of a dead load looks through for an earlier access.
const ABOVE: usize = 4096;

/// The computed addresses a block remembers having gone through.
const RECENT: usize = 8;

/// The statements after the one that zeroes a temporary that are looked through for the stores that fill it.
const NEAR: usize = 64;

/// The blocks that only jump on that are followed to where a branch's arm comes.
const STEPS: usize = 64;

/// The most nodes a statement may evaluate for a read in it to be weighed against the read before it.
const NODES: usize = 4096;

/// The most clears of a block that are kept in mind while the statements after them are weighed.
const CLEARS: usize = 16;

/// Where a statement stands: its block and its place there, the terminator past the last statement.
type Site = (BlockId, usize);

/// Forwards each store to the loads that must read it and drops each load nothing reads; `true` when anything changed.
// `forward` forwards a constant only: a variable read back is as likely the source's own second
// read. With `escape` on, a store through another variable leaves the constant where one of the
// two holds an object no one else has yet, or where it is the same variable and other bytes.
// A temporary that is zeroed and filled through its own address is no memory at all: `cells`.
// `forward` also moves a temporary's value to its one read in a later block (`across`), and
// writes a clear of the low bytes of a local whose upper bytes stay zero as `local = 0`, and reads
// a temporary given zero cells whose address only clears it as those zeroes (`zeroed`), and
// removes a clear of bytes an earlier clear left zero (`cleared_again`). With `escape`, it moves a
// value in its block past the binding of a delegate just made (`delegates`), and past the setting
// of an element of an array just made (`initialisers`): `made`.
// `dead-loads` also drops a branch whose arms meet where its test cannot fault, and a read kept
// for nothing that the next statement makes first.
pub fn run(f: &mut cfg::Function, cx: &Context<'_>, _stats: &mut Stats) -> bool {
    let options = &cx.options;
    if f.head.incomplete.is_some() || f.cfg.blocks.is_empty() {
        return false;
    }
    let mut changed = false;
    if options.on(Pass::Forward) {
        changed |= cleared_again(f);
        changed |= cells(f);
        changed |= zeroed(f);
        changed |= stores(f, options.on(Pass::Escape));
        changed |= low_clears(f);
        let escape = options.on(Pass::Escape);
        let delegates = escape && options.on(Pass::Delegates);
        changed |= made(f, delegates, escape && options.on(Pass::Initialisers));
        changed |= across(f, options.on(Pass::Escape));
    }
    changed |= options.on(Pass::DeadLoads) && loads(f);
    changed |= options.on(Pass::DeadLoads) && repeated(f);
    if changed {
        super::values::declared(f);
    }
    changed
}

/// Whether a statement names `var` at all: assigns it, reads it or takes its address.
fn mentions(stmt: &Stmt, var: Var) -> bool {
    let mut found = stmt.targets().contains(&var);
    stmt.each_expr(&mut |top| found |= names(top, var));
    found
}

/// Whether `top` reads `var` or takes its address.
fn names(top: &Expr, var: Var) -> bool {
    let mut found = false;
    top.walk(&mut |e| {
        found |= matches!(e, Expr::Var(named) | Expr::AddrOf { var: named, .. } if *named == var);
    });
    found
}

/// The kind and the value of a store that fills the cell of `var` from its first byte, through the address of `var` itself, with a value that names `var` nowhere.
fn fill(stmt: &Stmt, var: Var) -> Option<(StoreKind, &Expr)> {
    let Stmt::Store {
        kind,
        addr: Expr::AddrOf { var: of, cell: 0 },
        offset: 0,
        value,
    } = stmt
    else {
        return None;
    };
    (*of == var && !names(value, var)).then_some((*kind, value))
}

/// What a cell of zeroes holds once a store of `kind` has filled its low bytes with `value`.
fn cut(kind: StoreKind, value: Expr) -> Expr {
    match kind {
        StoreKind::U8 => Expr::cast(Scalar::U8, value),
        StoreKind::U16 => Expr::cast(Scalar::U16, value),
        StoreKind::I32 => Expr::cast(Scalar::U32, value),
        StoreKind::U64 | StoreKind::Ref => value,
    }
}

/// What a clear through a variable's own address covers: the variable, its bytes from the variable's first as a range, and the place they are in.
fn clear_of(stmt: &Stmt) -> Option<(Var, (u64, u64), Place)> {
    let Stmt::ZeroBytes {
        addr: Expr::AddrOf { var, cell },
        bytes,
    } = stmt
    else {
        return None;
    };
    let from = 8 * u64::from(*cell);
    let lo = |n: u32| n.saturating_add(*cell);
    let hi = |n: u32| lo(n).saturating_add(((*bytes).max(1) - 1) / 8);
    let place = match *var {
        Var::Local(n) => Place::Frame {
            params: false,
            lo: lo(n),
            hi: hi(n),
        },
        Var::Param(n) => Place::Frame {
            params: true,
            lo: lo(n),
            hi: hi(n),
        },
        Var::Global { .. } => return None,
        temporary => Place::Temp(temporary),
    };
    Some((*var, (from, from + u64::from(*bytes)), place))
}

/// Removes each clear through a variable's own address of bytes that a clear before it in the block left zero, where nothing between may have written them.
// `zero_bytes(&local_6, 4); v = local_6; zero_bytes(&local_6, 4);`: the second clear writes the
// zeroes the bytes already hold, and through the variable's own address it cannot fault.
// ⚠ A variable a call or a store through a pointer may reach is written by any of them.
fn cleared_again(f: &mut cfg::Function) -> bool {
    let facts = Facts::of(f);
    let mut changed = false;
    for block in &mut f.cfg.blocks {
        // The clears whose bytes are still zero.
        let mut standing: Vec<(Var, (u64, u64), Place)> = Vec::new();
        let old = std::mem::take(&mut block.stmts);
        for stmt in old {
            let clear = clear_of(&stmt);
            let again = clear.is_some_and(|(var, (from, to), _)| {
                (standing.iter())
                    .any(|(held, (lo, hi), _)| *held == var && *lo <= from && to <= *hi)
            });
            if again {
                changed = true;
                continue;
            }
            let effects = Effects::of_stmt(&facts, &stmt);
            standing.retain(|(.., place)| {
                let reached = facts.exposed(*place) && effects.writes_memory();
                !effects.writes(*place) && !reached
            });
            if let Some(clear) = clear {
                if standing.len() >= CLEARS {
                    standing.remove(0);
                }
                standing.push(clear);
            }
            block.stmts.push(stmt);
        }
    }
    changed
}

/// A temporary of one cell that a block zeroes and then fills through the temporary's own address.
struct Filled {
    /// The statement that zeroes it.
    made: Site,
    /// The stores that fill it, by their places in that block, in the order they stand.
    stores: Vec<usize>,
}

/// Reads a temporary of one cell that is zeroed, filled through its own address and read only after as the value that filled it.
// `v = zero_slots(1); store_i32(&v, 0, x);` and then `v[0]` wherever the block that filled it
// leads: `v` is `x` cut to the bytes stored, with nothing above them. Every read stands after the
// last store in that block, or in a block that one dominates, so it sees that store whichever
// way it was reached; a store of a constant before the last is written over before any read.
// A constant goes to every read. Any other value stays in `v`, now one cell assigned once.
// ⚠ Every address of `v` must be one of those stores': an address held or handed on is memory.
fn cells(f: &mut cfg::Function) -> bool {
    let facts = Facts::of(f);
    let mut found: HashMap<Var, Filled> = HashMap::new();
    for (b, block) in f.cfg.blocks.iter().enumerate() {
        for (i, stmt) in block.stmts.iter().enumerate() {
            let Stmt::Assign {
                target: var @ Var::Value { .. },
                value: Expr::ZeroSlots(1),
            } = stmt
            else {
                continue;
            };
            let uses = facts.uses.get(var).copied().unwrap_or_default();
            let own =
                uses.defs == 1 && uses.reads > 0 && uses.addrs > 0 && uses.addrs == uses.direct;
            if !own {
                continue;
            }
            let mut stores = Vec::new();
            for (k, later) in block.stmts.iter().enumerate().skip(i + 1).take(NEAR) {
                if !mentions(later, *var) {
                    continue;
                }
                if fill(later, *var).is_none() {
                    break;
                }
                stores.push(k);
            }
            let Some((last, earlier)) = stores.split_last() else {
                continue;
            };
            let Some((kind, _)) = block.stmts.get(*last).and_then(|stmt| fill(stmt, *var)) else {
                continue;
            };
            // A store before the last leaves nothing: a constant, in bytes the last one fills.
            let covered = earlier.iter().all(|k| {
                let before = block.stmts.get(*k).and_then(|stmt| fill(stmt, *var));
                matches!(before, Some((first, Expr::Const(_))) if first.bytes() <= kind.bytes())
            });
            if stores.len() == uses.addrs as usize && covered {
                found.insert(
                    *var,
                    Filled {
                        made: (b, i),
                        stores,
                    },
                );
            }
        }
    }
    if found.is_empty() {
        return false;
    }

    // Where each of them is read as its one cell.
    let mut reads: HashMap<Var, Vec<Site>> = found.keys().map(|var| (*var, Vec::new())).collect();
    for (b, block) in f.cfg.blocks.iter().enumerate() {
        for k in 0..=block.stmts.len() {
            let mut see = |top: &Expr| {
                top.walk(&mut |e| {
                    let Expr::Cell { base, index: 0 } = e else {
                        return;
                    };
                    if let Expr::Var(var) = &**base {
                        if let Some(sites) = reads.get_mut(var) {
                            sites.push((b, k));
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
        }
    }
    let idom = f.cfg.dominators();
    found.retain(|var, filled| {
        let Some(last) = filled.stores.last().copied() else {
            return false;
        };
        let Some(sites) = reads.get(var) else {
            return false;
        };
        // Every read of the temporary is one of those, and none can run before the last store.
        let counted = (facts.uses.get(var)).is_some_and(|uses| uses.reads as usize == sites.len());
        let block = filled.made.0;
        counted
            && sites.iter().all(|(at, k)| match *at == block {
                true => *k > last,
                false => dominates(&idom, block, *at),
            })
    });
    if found.is_empty() {
        return false;
    }

    // What each read becomes, the statements that go, and the stores that become assignments.
    let mut read_as: HashMap<Var, Expr> = HashMap::new();
    let mut gone: HashSet<Site> = HashSet::new();
    let mut assigns: HashMap<Site, Var> = HashMap::new();
    for (var, filled) in &found {
        let (b, made) = filled.made;
        let Some((last, earlier)) = filled.stores.split_last() else {
            continue;
        };
        let stored = (f.cfg.blocks.get(b))
            .and_then(|block| block.stmts.get(*last))
            .and_then(|stmt| fill(stmt, *var));
        let Some((kind, value)) = stored else {
            continue;
        };
        gone.insert((b, made));
        gone.extend(earlier.iter().map(|k| (b, *k)));
        match constant(value) {
            true => {
                gone.insert((b, *last));
                read_as.insert(*var, cut(kind, value.clone()));
            }
            false => {
                assigns.insert((b, *last), *var);
                read_as.insert(*var, Expr::Var(*var));
            }
        }
    }
    if read_as.is_empty() {
        return false;
    }

    let reread = |top: &mut Expr| {
        top.walk_mut(&mut |e| {
            let to = match &*e {
                Expr::Cell { base, index: 0 } => match &**base {
                    Expr::Var(var) => read_as.get(var).cloned(),
                    _ => None,
                },
                _ => None,
            };
            if let Some(to) = to {
                *e = to;
            }
        });
    };
    for (b, block) in f.cfg.blocks.iter_mut().enumerate() {
        let old = std::mem::take(&mut block.stmts);
        for (k, stmt) in old.into_iter().enumerate() {
            if gone.contains(&(b, k)) {
                continue;
            }
            let mut stmt = match (assigns.get(&(b, k)), stmt) {
                (Some(var), Stmt::Store { kind, value, .. }) => Stmt::Assign {
                    target: *var,
                    value: cut(kind, value),
                },
                (_, other) => other,
            };
            stmt.each_expr_mut(&mut |top| reread(top));
            block.stmts.push(stmt);
        }
        if let Some(e) = block.term.expr_mut() {
            reread(e);
        }
    }
    for (var, to) in &read_as {
        match to {
            // Kept, it is a temporary of one cell.
            Expr::Var(_) => {
                if let Some(info) = f.head.vars.get_mut(var) {
                    info.width = Width::Cell;
                }
            }
            _ => {
                f.head.vars.remove(var);
            }
        }
    }
    true
}

/// Reads each temporary given zero cells by its one assignment, whose address only ever clears bytes of those cells, as the zeroes it holds.
// `v = zero_slots(2); zero_bytes(&v, 16); store_bytes(p, 0x10, 16, v);` is
// `store_bytes(p, 0x10, 16, zero_slots(2));`: every write of `v` leaves its cells zero, so a read
// its assignment comes before on every path reads zeroes, and a cell of it read alone is zero.
// ⚠ Every address of `v` must be one of those clears: an address handed on, or a load or a store
// gone through it, is memory something else may write.
fn zeroed(f: &mut cfg::Function) -> bool {
    let facts = Facts::of(f);
    // Each such temporary: its cells, and where it is assigned.
    let mut found: HashMap<Var, (u32, Site)> = HashMap::new();
    for (b, block) in f.cfg.blocks.iter().enumerate() {
        for (i, stmt) in block.stmts.iter().enumerate() {
            let Stmt::Assign {
                target,
                value: Expr::ZeroSlots(cells),
            } = stmt
            else {
                continue;
            };
            let once = (facts.uses.get(target)).is_some_and(|uses| uses.defs == 1);
            if target.is_temporary() && once && *cells > 0 {
                found.insert(*target, (*cells, (b, i)));
            }
        }
    }
    if found.is_empty() {
        return false;
    }
    // The clears through each one's own address that stay within its cells, and those that do not.
    let mut clears: HashMap<Var, u32> = HashMap::new();
    let mut spoiled: HashSet<Var> = HashSet::new();
    for stmt in f.cfg.blocks.iter().flat_map(|block| &block.stmts) {
        let Stmt::ZeroBytes {
            addr: Expr::AddrOf { var, cell },
            bytes,
        } = stmt
        else {
            continue;
        };
        let Some((cells, _)) = found.get(var) else {
            continue;
        };
        let end = 8 * u64::from(*cell) + u64::from(*bytes);
        match end <= 8 * u64::from(*cells) {
            true => *clears.entry(*var).or_default() += 1,
            false => {
                spoiled.insert(*var);
            }
        }
    }
    found.retain(|var, _| {
        let addrs = facts.uses.get(var).map_or(0, |uses| uses.addrs);
        !spoiled.contains(var) && addrs == clears.get(var).copied().unwrap_or(0)
    });
    if found.is_empty() {
        return false;
    }

    // Every read comes after the assignment on every path, and a cell read alone is one of its cells.
    let idom = f.cfg.dominators();
    let before = |a: Site, b: Site| match a.0 == b.0 {
        true => a.1 < b.1,
        false => dominates(&idom, a.0, b.0),
    };
    let mut late: HashSet<Var> = HashSet::new();
    for (b, block) in f.cfg.blocks.iter().enumerate() {
        for k in 0..=block.stmts.len() {
            let mut see = |top: &Expr| {
                top.walk(&mut |e| {
                    let (var, index) = match e {
                        Expr::Var(var) => (var, None),
                        Expr::Cell { base, index } => match &**base {
                            Expr::Var(var) => (var, Some(*index)),
                            _ => return,
                        },
                        _ => return,
                    };
                    let Some((cells, set)) = found.get(var) else {
                        return;
                    };
                    if !before(*set, (b, k)) || index.is_some_and(|index| index >= *cells) {
                        late.insert(*var);
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
    for var in late {
        found.remove(&var);
    }
    if found.is_empty() {
        return false;
    }

    // A cell read alone is a zero cell; read whole, it is the zero cells it was given.
    let reread = |top: &mut Expr| {
        top.walk_mut(&mut |e| {
            let cell = match &*e {
                Expr::Cell { base, .. } => {
                    matches!(&**base, Expr::Var(var) if found.contains_key(var))
                }
                _ => false,
            };
            if cell {
                *e = Expr::Const(Const::i32(0));
            }
        });
        top.walk_mut(&mut |e| {
            let whole = match &*e {
                Expr::Var(var) => found.get(var).map(|(cells, _)| *cells),
                _ => None,
            };
            if let Some(cells) = whole {
                *e = Expr::ZeroSlots(cells);
            }
        });
    };
    for block in &mut f.cfg.blocks {
        block.stmts.retain(|stmt| match stmt {
            Stmt::Assign {
                target,
                value: Expr::ZeroSlots(_),
            } => !found.contains_key(target),
            Stmt::ZeroBytes {
                addr: Expr::AddrOf { var, .. },
                ..
            } => !found.contains_key(var),
            _ => true,
        });
        for stmt in &mut block.stmts {
            stmt.each_expr_mut(&mut |top| reread(top));
        }
        if let Some(e) = block.term.expr_mut() {
            reread(e);
        }
    }
    // A temporary that is gone is no variable of the function any more.
    for var in found.keys() {
        f.head.vars.remove(var);
    }
    true
}

/// A constant a store leaves in memory: `bytes` bytes at `offset` from the address `base` holds.
struct Field {
    base: Var,
    offset: u32,
    bytes: u32,
    value: Const,
}

/// The field a statement fills: a store of a constant through a variable that holds a pointer and that only an assignment changes.
// ⚠ A variable that holds the address of a local is no such pointer: the local is also written
// by its name and through its own address, which no store through a pointer shows.
fn filled(facts: &Facts, stmt: &Stmt) -> Option<Field> {
    let Stmt::Store {
        kind,
        addr: Expr::Var(base),
        offset,
        value: Expr::Const(value),
    } = stmt
    else {
        return None;
    };
    (facts.named_only(*base) && !facts.holds_address(*base)).then(|| Field {
        base: *base,
        offset: *offset,
        bytes: kind.bytes(),
        value: value.clone(),
    })
}

/// The constants that stores leave in fields some load reads back, each known by its number.
struct Fields<'a> {
    facts: &'a Facts,
    /// Whether two stores are told apart by the variables their addresses are in (`escape`).
    finer: bool,
    all: Vec<Field>,
    /// The fields at each place: a variable, an offset and a count of bytes.
    at: HashMap<(Var, u32, u32), Vec<usize>>,
    of_base: HashMap<Var, Vec<usize>>,
}

/// What a statement does to the fields.
struct Passed {
    /// The fields it may write, or whose address it changes.
    ended: Set,
    /// The field it fills.
    made: Option<usize>,
    /// Whether it calls: then a load of it may come after the call.
    calls: bool,
}

impl Fields<'_> {
    /// The field at `place` that `held` says still holds its constant.
    fn held(&self, held: &Set, place: (Var, u32, u32)) -> Option<Const> {
        let ids = self.at.get(&place)?;
        let id = ids.iter().copied().find(|id| held.has(*id))?;
        Some(self.all.get(id)?.value.clone())
    }

    fn passed(&self, stmt: &Stmt, site: Site) -> Passed {
        let n = self.all.len();
        let effects = Effects::of_stmt(self.facts, stmt);
        if effects.call {
            return Passed {
                ended: Set::full(n),
                made: None,
                calls: true,
            };
        }
        let mut ended = Set::new(n);
        for target in stmt.targets() {
            for id in self.of_base.get(target).into_iter().flatten() {
                ended.insert(*id);
            }
        }
        if effects.writes_memory() {
            let through = match stored(stmt) {
                Some((Expr::Var(base), offset, bytes)) => Some((*base, offset, bytes)),
                _ => None,
            };
            let own = through.is_some_and(|(base, ..)| self.facts.fresh_at(base, site));
            for (id, field) in self.all.iter().enumerate() {
                let kept = match through {
                    Some((base, offset, bytes)) if base == field.base => {
                        let last = bytes.map(|bytes| offset.saturating_add(bytes.max(1) - 1));
                        let field_last = field.offset.saturating_add(field.bytes.max(1) - 1);
                        let apart =
                            last.is_some_and(|last| last < field.offset) || field_last < offset;
                        self.finer && apart
                    }
                    _ => own || self.facts.fresh_at(field.base, site),
                };
                if !kept {
                    ended.insert(id);
                }
            }
        }
        let made = filled(self.facts, stmt).and_then(|field| {
            let ids = self.at.get(&(field.base, field.offset, field.bytes))?;
            (ids.iter().copied()).find(|id| {
                self.all
                    .get(*id)
                    .is_some_and(|known| known.value == field.value)
            })
        });
        Passed {
            ended,
            made,
            calls: false,
        }
    }

    /// Writes each load in `top` of a field `held` says holds a constant as that constant, read as the load reads it.
    fn read_back(&self, top: &mut Expr, held: &Set) -> bool {
        let mut changed = false;
        top.walk_mut(&mut |e| {
            let found = match &*e {
                Expr::Load { ty, addr, offset } => match &**addr {
                    Expr::Var(base) => (self.held(held, (*base, *offset, ty.bytes())))
                        .map(|value| (*ty, Expr::Const(value))),
                    _ => None,
                },
                _ => None,
            };
            if let Some((ty, value)) = found {
                *e = match ty {
                    Scalar::U64 => value,
                    ty => Expr::cast(ty, value),
                };
                changed = true;
            }
        });
        changed
    }
}

/// Writes a load of a field that every path left holding one constant as that constant.
// A field is `bytes` bytes at an offset from a variable's value. It holds a constant from the
// store of it to the first statement that calls, assigns the variable or may write those bytes,
// and at a join only where every path in holds the same.
fn stores(f: &mut cfg::Function, finer: bool) -> bool {
    let facts = match finer {
        true => Facts::finer(f),
        false => Facts::of(f),
    };
    let mut wanted: HashSet<(Var, u32, u32)> = HashSet::new();
    each_top(&f.cfg, &mut |top| {
        top.walk(&mut |e| {
            if let Expr::Load { ty, addr, offset } = e {
                if let Expr::Var(base) = &**addr {
                    wanted.insert((*base, *offset, ty.bytes()));
                }
            }
        });
    });
    let mut fields = Fields {
        facts: &facts,
        finer,
        all: Vec::new(),
        at: HashMap::new(),
        of_base: HashMap::new(),
    };
    for stmt in f.cfg.blocks.iter().flat_map(|block| &block.stmts) {
        let Some(field) = filled(&facts, stmt) else {
            continue;
        };
        let place = (field.base, field.offset, field.bytes);
        let known = fields.at.entry(place).or_default();
        let listed =
            (known.iter()).any(|id| fields.all.get(*id).is_some_and(|k| k.value == field.value));
        if !wanted.contains(&place) || listed {
            continue;
        }
        known.push(fields.all.len());
        (fields.of_base.entry(field.base).or_default()).push(fields.all.len());
        fields.all.push(field);
    }
    let n = fields.all.len();
    if n == 0 || !flow::fits(&f.cfg, n) {
        return false;
    }

    let (mut made, mut ended) = (Vec::new(), Vec::new());
    for (b, block) in f.cfg.blocks.iter().enumerate() {
        let (mut held, mut gone) = (Set::new(n), Set::new(n));
        for (k, stmt) in block.stmts.iter().enumerate() {
            let passed = fields.passed(stmt, (b, k));
            held.without(&passed.ended);
            gone.add(&passed.ended);
            if let Some(id) = passed.made {
                held.insert(id);
            }
        }
        // ⚠ A call in the test a block ends on runs before every block it leads to: no field outlasts it.
        if (block.term.expr()).is_some_and(|test| Effects::of_expr(&facts, test).call) {
            (held, gone) = (Set::new(n), Set::full(n));
        }
        made.push(held);
        ended.push(gone);
    }
    let Some(ins) = flow::entering(&f.cfg, n, (&made[..], &ended[..]), &Set::new(n), true) else {
        return false;
    };

    let mut changed = false;
    for (b, block) in f.cfg.blocks.iter_mut().enumerate() {
        let Some(mut held) = ins.get(b).cloned() else {
            continue;
        };
        for (k, stmt) in block.stmts.iter_mut().enumerate() {
            let passed = fields.passed(stmt, (b, k));
            if !passed.calls {
                stmt.each_expr_mut(&mut |top| changed |= fields.read_back(top, &held));
            }
            held.without(&passed.ended);
            if let Some(id) = passed.made {
                held.insert(id);
            }
        }
        if let Some(e) = block.term.expr_mut() {
            if !Effects::of_expr(&facts, e).call {
                changed |= fields.read_back(e, &held);
            }
        }
    }
    changed
}

/// The address an assignment nothing reads loads through, where the load is all its value does.
fn unread_load<'a>(facts: &Facts, stmt: &'a Stmt) -> Option<&'a Expr> {
    let Stmt::Assign { target, value } = stmt else {
        return None;
    };
    if !facts.unread(*target) {
        return None;
    }
    let mut inner = value;
    while let Expr::Cast { e, .. } = inner {
        inner = &**e;
    }
    match inner {
        Expr::Load { addr, .. } | Expr::LoadBytes { addr, .. } => Some(&**addr),
        _ => None,
    }
}

/// Calls `f` on each address a statement is sure to load or store through.
fn gone_through<'a>(stmt: &'a Stmt, f: &mut impl FnMut(&'a Expr)) {
    stmt.each_expr(&mut |top| sure_loads(top, &mut *f));
    if let Some((addr, ..)) = stored(stmt) {
        f(addr);
    }
}

fn reads(e: &Expr, var: Var) -> bool {
    let mut read = false;
    e.walk(&mut |node| read |= matches!(node, Expr::Var(named) if *named == var));
    read
}

/// The block control comes to from `b` through blocks that only jump on.
fn landing(cfg: &cfg::Cfg, mut b: BlockId) -> BlockId {
    for _ in 0..STEPS {
        match cfg.blocks.get(b) {
            Some(block) if block.stmts.is_empty() => match block.term {
                Terminator::Jump(next) if next != b => b = next,
                _ => break,
            },
            _ => break,
        }
    }
    b
}

/// Where a block's branch goes whichever way it is decided, where both its arms come to one block.
fn met(cfg: &cfg::Cfg, b: BlockId) -> Option<BlockId> {
    match &cfg.blocks.get(b)?.term {
        Terminator::Branch { then, els, .. } => {
            (landing(cfg, *then) == landing(cfg, *els)).then_some(*then)
        }
        _ => None,
    }
}

/// Whether evaluating `e` shows nothing: it calls, stores, allocates and divides nothing, and every load it makes goes through an address `safe` says cannot be the first to fault.
fn idle(facts: &Facts, e: &Expr, safe: impl Fn(&Expr) -> bool) -> bool {
    let mut idle = true;
    e.walk(&mut |node| {
        idle &= match node {
            Expr::Load { addr, .. } | Expr::LoadBytes { addr, .. } => safe(addr),
            // A variable is always there to read.
            Expr::Var(_) => true,
            Expr::Binary {
                op: BinOp::Div | BinOp::Rem,
                ..
            } => false,
            other => quiet(facts, other),
        };
    });
    idle
}

/// Whether evaluating `e` itself, apart from its operands, changes nothing and reads no memory.
fn quiet(facts: &Facts, e: &Expr) -> bool {
    let mut alone = Effects::default();
    alone.node(facts, e);
    alone.free()
}

/// Removes each load nothing reads that cannot be the first to go through its address.
// A load may fault, and one that does is what the source relied on: it goes only where the same
// address was gone through before on every path, with nothing between that could change it, or
// where the address is of an object the function made. By the same proof a branch whose arms
// meet goes straight on where its test does nothing else.
fn loads(f: &mut cfg::Function) -> bool {
    let facts = Facts::of(f);
    let any = (f.cfg.blocks.iter().flat_map(|block| &block.stmts))
        .any(|stmt| unread_load(&facts, stmt).is_some());
    let meets = (0..f.cfg.blocks.len()).any(|b| met(&f.cfg, b).is_some());
    if !any && !meets {
        return false;
    }
    let defs = |var: Var| facts.uses.get(&var).map_or(0, |uses| uses.defs);
    // Where each variable assigned once is assigned, which of them are given a new object, and
    // the last statement of each block that is sure to go through each variable.
    let mut assigned: HashMap<Var, Site> = HashMap::new();
    let mut made: HashSet<Var> = HashSet::new();
    let mut through: HashMap<(Var, BlockId), usize> = HashMap::new();
    for (b, block) in f.cfg.blocks.iter().enumerate() {
        for (k, stmt) in block.stmts.iter().enumerate() {
            for target in stmt.targets() {
                if defs(*target) == 1 {
                    assigned.insert(*target, (b, k));
                }
            }
            if let Stmt::Assign {
                target,
                value: Expr::NewObject(_) | Expr::NewArray { .. },
            } = stmt
            {
                if defs(*target) == 1 {
                    made.insert(*target);
                }
            }
            gone_through(stmt, &mut |addr| {
                if let Expr::Var(base) = addr {
                    through.insert((*base, b), k);
                }
            });
        }
        if let Some(e) = block.term.expr() {
            sure_loads(e, &mut |addr| {
                if let Expr::Var(base) = addr {
                    through.insert((*base, b), block.stmts.len());
                }
            });
        }
    }
    let idom = f.cfg.dominators();
    let before = |a: Site, b: Site| match a.0 == b.0 {
        true => a.1 < b.1,
        false => dominates(&idom, a.0, b.0),
    };
    // Whether a load through `base` at `site` goes into an object the function made, or where a
    // statement of a block above went, the variable holding one value all along.
    let above = |base: Var, site: Site| {
        let set = assigned.get(&base).copied();
        let steady = facts.named_only(base) && (defs(base) == 0 || set.is_some());
        if !steady {
            return false;
        }
        if made.contains(&base) && set.is_some_and(|set| before(set, site)) {
            return true;
        }
        let mut block = site.0;
        for _ in 0..ABOVE {
            match idom.get(block).copied().flatten() {
                Some(up) if up != block => block = up,
                _ => return false,
            }
            let gone = through.get(&(base, block)).copied();
            if gone.is_some_and(|k| set.is_none_or(|set| before(set, (block, k)))) {
                return true;
            }
        }
        false
    };

    let mut dead: HashSet<Site> = HashSet::new();
    let mut decided: Vec<(BlockId, BlockId)> = Vec::new();
    for (b, block) in f.cfg.blocks.iter().enumerate() {
        // The variables the block has gone through since it last assigned them, and the computed
        // addresses it has gone through since memory was last written.
        let mut since: HashSet<Var> = HashSet::new();
        let mut recent: Vec<&Expr> = Vec::new();
        for (k, stmt) in block.stmts.iter().enumerate() {
            let safe = unread_load(&facts, stmt).is_some_and(|addr| match addr {
                Expr::Var(base) => since.contains(base) || above(*base, (b, k)),
                computed => recent.contains(&computed),
            });
            if safe {
                dead.insert((b, k));
                continue;
            }
            let steady = !Effects::of_stmt(&facts, stmt).writes_memory();
            if !steady {
                recent.clear();
            }
            gone_through(stmt, &mut |addr| match addr {
                Expr::Var(base) => {
                    if facts.named_only(*base) {
                        since.insert(*base);
                    }
                }
                Expr::AddrOf { .. } => {}
                computed => {
                    let does = Effects::of_expr(&facts, computed);
                    if steady && !does.call && !does.alloc {
                        if recent.len() >= RECENT {
                            recent.remove(0);
                        }
                        recent.push(computed);
                    }
                }
            });
            for target in stmt.targets() {
                since.remove(target);
                recent.retain(|seen| !reads(seen, *target));
            }
        }
        let (Some(to), Some(test)) = (met(&f.cfg, b), block.term.expr()) else {
            continue;
        };
        let at = (b, block.stmts.len());
        let safe = |addr: &Expr| match addr {
            Expr::Var(base) => since.contains(base) || above(*base, at),
            Expr::AddrOf { .. } => true,
            computed => recent.contains(&computed),
        };
        if idle(&facts, test, safe) {
            decided.push((b, to));
        }
    }
    if dead.is_empty() && decided.is_empty() {
        return false;
    }
    for (b, to) in decided {
        if let Some(block) = f.cfg.blocks.get_mut(b) {
            block.term = Terminator::Jump(to);
        }
    }
    for (b, block) in f.cfg.blocks.iter_mut().enumerate() {
        let mut k = 0;
        block.stmts.retain(|_| {
            k += 1;
            !dead.contains(&(b, k - 1))
        });
    }
    true
}

/// What a statement that keeps no value evaluates, under the readings of its result: `_ = (int)p->f[i];` is `p->f[i]`.
fn discarded(stmt: &Stmt) -> Option<&Expr> {
    let Stmt::Eval(e) = stmt else {
        return None;
    };
    let mut inner = e;
    while let Expr::Cast { e: next, .. } | Expr::Truth(next) = inner {
        inner = &**next;
    }
    Some(inner)
}

/// Whether `top` evaluates something equal to `read` in an arm that may not run.
fn in_arm(top: &Expr, read: &Expr) -> bool {
    let mut armed = false;
    top.walk(&mut |e| {
        let arms: [Option<&Expr>; 2] = match e {
            Expr::Select { then, els, .. } => [Some(&**then), Some(&**els)],
            Expr::Binary {
                op: BinOp::LogicalAnd | BinOp::LogicalOr,
                r,
                ..
            } => [Some(&**r), None],
            _ => [None, None],
        };
        for arm in arms.into_iter().flatten() {
            arm.walk(&mut |node| armed |= node == read);
        }
    });
    armed
}

/// Whether the expressions `tops` of one statement, in the order it evaluates them, come to `read` before they do anything that shows, and not in an arm that may not run.
fn first_of(facts: &Facts, tops: &[&Expr], read: &Expr) -> bool {
    let mut order = Vec::new();
    for top in tops {
        evaluated(top, &mut order);
    }
    let mut own = Vec::new();
    evaluated(read, &mut own);
    if order.len() > NODES {
        return false;
    }
    // A node comes after every node it works on: the read's own nodes stand just before it.
    let Some(at) = order.iter().position(|e| *e == read) else {
        return false;
    };
    let Some(start) = (at + 1).checked_sub(own.len()) else {
        return false;
    };
    let still = (order[..start].iter()).all(|e| match e {
        Expr::Var(_) => true,
        Expr::Binary {
            op: BinOp::Div | BinOp::Rem,
            ..
        } => false,
        other => quiet(facts, other),
    });
    still && !tops.iter().any(|top| in_arm(top, read))
}

/// Removes each statement that only reads what the statement after it reads first: `_ = p->f[i]; g(p->f[i]);` is `g(p->f[i]);`.
// The two reads are made in one state: what the second statement evaluates before it changes
// nothing. So the read left faults where the one removed would have, and reads what it read.
fn repeated(f: &mut cfg::Function) -> bool {
    let facts = Facts::of(f);
    let mut changed = false;
    for block in &mut f.cfg.blocks {
        let old = std::mem::take(&mut block.stmts);
        let mut kept: Vec<Stmt> = Vec::with_capacity(old.len());
        // From the last: each statement is weighed against the one kept after it.
        for stmt in old.into_iter().rev() {
            let again = discarded(&stmt).is_some_and(|read| {
                let does = Effects::of_expr(&facts, read);
                let mut tops: Vec<&Expr> = Vec::new();
                match kept.last() {
                    Some(next) => next.each_expr(&mut |top| tops.push(top)),
                    None => tops.extend(block.term.expr()),
                }
                !(does.call || does.store || does.alloc) && first_of(&facts, &tops, read)
            });
            if again {
                changed = true;
                continue;
            }
            kept.push(stmt);
        }
        kept.reverse();
        block.stmts = kept;
    }
    changed
}

#[cfg(test)]
#[path = "forward_tests.rs"]
mod tests;
