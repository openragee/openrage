//! Forward: a load of what the function itself just stored reads the value stored, and a load
//! nothing reads goes.
//!
//! Works on the blocks, in rounds with `values` and `coalesce`. A store is forwarded to a load
//! only where nothing between them can write the place: no call, no store through an address
//! that may be the same, no address of it handed out.

#[path = "forward_flow.rs"]
pub(super) mod flow;

use std::collections::{HashMap, HashSet};

use crate::gta6::script::cfg::{self, dominates, BlockId};
use crate::gta6::script::ir::{Const, Expr, Scalar, Stmt, Var};
use crate::gta6::script::{Context, Pass, Stats};

use self::flow::Set;
use super::values::effects::{each_top, stored, sure_loads, Effects, Facts};

/// The names this file's rewrites are switched by.
pub const PASSES: &[Pass] = &[Pass::Forward, Pass::DeadLoads];

/// The dominators the proof of a dead load looks through for an earlier access.
const ABOVE: usize = 4096;

/// The computed addresses a block remembers having gone through.
const RECENT: usize = 8;

/// Where a statement stands: its block and its place there, the terminator past the last statement.
type Site = (BlockId, usize);

/// Forwards each store to the loads that must read it and drops each load nothing reads; `true` when anything changed.
// `forward` forwards a constant only: a variable read back is as likely the source's own second
// read. With `escape` on, a store through another variable leaves the constant where one of the
// two holds an object no one else has yet, or where it is the same variable and other bytes.
pub fn run(f: &mut cfg::Function, cx: &Context<'_>, _stats: &mut Stats) -> bool {
    let options = &cx.options;
    if f.head.incomplete.is_some() || f.cfg.blocks.is_empty() {
        return false;
    }
    let mut changed = options.on(Pass::Forward) && stores(f, options.on(Pass::Escape));
    changed |= options.on(Pass::DeadLoads) && loads(f);
    if changed {
        super::values::declared(f);
    }
    changed
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

/// Removes each load nothing reads that cannot be the first to go through its address.
// A load may fault, and one that does is what the source relied on: it goes only where the same
// address was gone through before on every path, with nothing between that could change it, or
// where the address is of an object the function made.
fn loads(f: &mut cfg::Function) -> bool {
    let facts = Facts::of(f);
    let any = (f.cfg.blocks.iter().flat_map(|block| &block.stmts))
        .any(|stmt| unread_load(&facts, stmt).is_some());
    if !any {
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
    }
    if dead.is_empty() {
        return false;
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

#[cfg(test)]
#[path = "forward_tests.rs"]
mod tests;
