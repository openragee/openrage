//! The kind of a value of several cells by the values it is copied from and into and compared with.
//!
//! Runs after `types`, which settles what the code's own forms state: a frame variable of several
//! cells it left of no named kind takes the one kind every one of those others names.

use std::collections::BTreeMap;

use super::kind_cells;
use crate::gta6::script::cfg;
use crate::gta6::script::ir::{Callee, Expr, Head, Kind, Stmt, Var, Width};
use crate::gta6::script::names::Names;
use crate::gta6::script::project::{signatures, Project};
use crate::gta6::script::{Context, Pass, Stats};

/// The runtime's own comparisons of two 16-byte values, which the project reads as two handles.
const HANDLE_TESTS: [&str; 2] = ["System.Guid.op_Equality", "System.Guid.op_Inequality"];
/// Values nested deeper than this say nothing a kind can be read off.
const DEPTH: u32 = 32;

/// What one way a value reaches a variable, or one use of it, says the variable is.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Says {
    /// Nothing: a zero, or a value of no named kind.
    Nothing,
    /// A value of this kind, which names what its cells are.
    Kind(Kind),
    /// Of the same kind as this other variable of no named kind.
    Same(Var),
    /// A way in no kind can be read off, which leaves the variable as it is.
    Refuse,
}

/// Whether a kind names what the cells of a value of it are.
fn named(kind: &Kind) -> bool {
    matches!(kind, Kind::Vector | Kind::Handle | Kind::Value { .. })
}

/// The variables of several cells `types` left of no named kind: by variable, its cells and its place.
// ⚠ Not a parameter: its kind is the signature's, which every call is held to.
type Candidates = BTreeMap<Var, (u32, usize)>;

fn candidates(head: &Head) -> Candidates {
    let mut found = Candidates::new();
    for (var, info) in &head.vars {
        let own = matches!(var, Var::Local(_) | Var::Value { .. } | Var::Stack { .. });
        if let (true, Width::Cells, Kind::Aggregate(cells)) = (own, info.width, &info.kind) {
            if *cells >= 2 {
                let at = found.len();
                found.insert(*var, (*cells, at));
            }
        }
    }
    found
}

/// What the variable `var` says a value of `cells` cells it is copied to or from is.
fn of_var(head: &Head, candidates: &Candidates, var: Var, cells: u32) -> Says {
    if let Some((held, _)) = candidates.get(&var) {
        return match *held == cells {
            true => Says::Same(var),
            false => Says::Refuse,
        };
    }
    let Some(info) = head.vars.get(&var) else {
        return Says::Refuse;
    };
    let whole = info.width == Width::Cells && kind_cells(&info.kind) == Some(cells);
    match (whole, named(&info.kind)) {
        (false, _) => Says::Refuse,
        (true, true) => Says::Kind(info.kind.clone()),
        (true, false) => Says::Nothing,
    }
}

/// What a call yielding `cells` cells says its value is.
// ⚠ A native a list states the result of, and an intrinsic, are `types`' to read: neither says
// nothing, for its kind may be one this pass does not read.
fn of_call(head: &Head, project: &Project, names: &Names, callee: &Callee, cells: u32) -> Says {
    match callee {
        Callee::Native { hash, .. } => {
            let listed = names.native(*hash).is_some_and(|n| n.returns.is_some());
            match listed {
                true => Says::Refuse,
                false => Says::Nothing,
            }
        }
        Callee::Intrinsic { .. } => Says::Refuse,
        _ => {
            let key = signatures::key_of(head.program, callee);
            let held = key.and_then(|key| project.signatures.get(&key));
            let result = held
                .filter(|own| own.returns == cells && named(&own.result))
                .map(|own| &own.result);
            match result {
                Some(kind) if kind_cells(kind) == Some(cells) => Says::Kind(kind.clone()),
                Some(_) => Says::Refuse,
                None => Says::Nothing,
            }
        }
    }
}

/// What a value assigned whole to a variable of `cells` cells says the variable is.
fn arrival(
    head: &Head,
    project: &Project,
    names: &Names,
    candidates: &Candidates,
    e: &Expr,
    cells: u32,
    depth: u32,
) -> Says {
    let Some(deeper) = depth.checked_sub(1) else {
        return Says::Refuse;
    };
    match e {
        Expr::Sugar { e: inner, .. } => {
            arrival(head, project, names, candidates, inner, cells, deeper)
        }
        Expr::ZeroSlots(n) if *n == cells => Says::Nothing,
        Expr::GuidEmpty if cells == 2 => Says::Kind(Kind::Handle),
        Expr::Vector3(_) if cells == 2 => Says::Kind(Kind::Vector),
        Expr::Var(var) => of_var(head, candidates, *var, cells),
        Expr::LoadBytes { bytes, .. } if u64::from(*bytes) == 8 * u64::from(cells) => Says::Nothing,
        Expr::Call {
            callee,
            returns: Some(n),
            ..
        } if *n == cells => of_call(head, project, names, callee, cells),
        _ => Says::Refuse,
    }
}

/// What the ways in and the uses of each candidate say, by its place.
struct Heard {
    refused: Vec<bool>,
    kinds: Vec<Vec<Kind>>,
    same: Vec<(usize, usize)>,
}

impl Heard {
    fn new(len: usize) -> Heard {
        Heard {
            refused: vec![false; len],
            kinds: vec![Vec::new(); len],
            same: Vec::new(),
        }
    }

    fn hear(&mut self, candidates: &Candidates, var: Var, says: Says) {
        let Some((_, at)) = candidates.get(&var).copied() else {
            return;
        };
        match says {
            Says::Nothing => {}
            Says::Kind(kind) => self.kinds[at].push(kind),
            Says::Same(other) => {
                if let Some((_, to)) = candidates.get(&other).copied() {
                    self.same.push((at, to));
                }
            }
            Says::Refuse => self.refused[at] = true,
        }
    }
}

/// What a use of a candidate says, which no way in it is: a variable of no named kind says nothing.
fn read(says: Says) -> Says {
    match says {
        Says::Refuse => Says::Nothing,
        says => says,
    }
}

/// The candidate a comparison of `bytes` bytes through two own addresses compares whole, with what the other side says.
fn compared(
    head: &Head,
    candidates: &Candidates,
    bytes: u32,
    var: Var,
    other: Var,
) -> Option<(Var, Says)> {
    let (cells, _) = candidates.get(&var).copied()?;
    let whole = u64::from(bytes) == 8 * u64::from(cells);
    whole.then(|| (var, read(of_var(head, candidates, other, cells))))
}

/// The bytes one of the runtime's comparisons of two values compares, by its name.
fn compared_bytes(name: &str) -> Option<u32> {
    name.strip_prefix("bytes_equal_")?.parse().ok()
}

/// The two variables a call is handed the own addresses of, where it is handed those alone.
fn own_pair(args: &[Expr]) -> Option<(Var, Var)> {
    match args {
        [Expr::AddrOf { var: a, cell: 0 }, Expr::AddrOf { var: b, cell: 0 }] => Some((*a, *b)),
        _ => None,
    }
}

/// The kinds the candidates of a function are found of.
fn found(f: &cfg::Function, project: &Project, names: &Names) -> Vec<(Var, Kind)> {
    let head = &f.head;
    let candidates = candidates(head);
    if candidates.is_empty() {
        return Vec::new();
    }
    let mut heard = Heard::new(candidates.len());
    // Every address of a candidate, and those of them a comparison of its bytes reads.
    let mut addressed: BTreeMap<Var, (u64, u64)> = BTreeMap::new();
    let mut uses = |top: &Expr, heard: &mut Heard| {
        top.walk(&mut |e| match e {
            Expr::AddrOf { var, .. } => {
                if candidates.contains_key(var) {
                    addressed.entry(*var).or_default().0 += 1;
                }
            }
            Expr::Call {
                callee: Callee::Intrinsic { name, .. },
                args,
                ..
            } => {
                let (Some(bytes), Some((a, b))) = (compared_bytes(name), own_pair(args)) else {
                    return;
                };
                for var in [a, b] {
                    if candidates.contains_key(&var) {
                        addressed.entry(var).or_default().1 += 1;
                    }
                }
                for (var, other) in [(a, b), (b, a)] {
                    if let Some((var, says)) = compared(head, &candidates, bytes, var, other) {
                        heard.hear(&candidates, var, says);
                    }
                }
            }
            Expr::Call {
                callee: Callee::Builtin { path, .. },
                args,
                ..
            } if HANDLE_TESTS.contains(&path.as_str()) && args.len() == 2 => {
                for arg in args {
                    let held = match arg {
                        Expr::Var(var) => candidates.get(var).map(|(cells, _)| (*var, *cells)),
                        _ => None,
                    };
                    if let Some((var, 2)) = held {
                        heard.hear(&candidates, var, Says::Kind(Kind::Handle));
                    }
                }
            }
            _ => {}
        });
    };
    for block in &f.cfg.blocks {
        for stmt in &block.stmts {
            for target in stmt.targets() {
                let Some((cells, _)) = candidates.get(target).copied() else {
                    continue;
                };
                let says = match stmt {
                    Stmt::Assign { value, .. } => {
                        arrival(head, project, names, &candidates, value, cells, DEPTH)
                    }
                    _ => Says::Refuse,
                };
                heard.hear(&candidates, *target, says);
            }
            // A candidate copied whole into a variable of a named kind is of that kind.
            if let Stmt::Assign {
                target,
                value: Expr::Var(var),
            } = stmt
            {
                let into = (!candidates.contains_key(target)).then_some(*target);
                if let (Some(into), Some((cells, _))) = (into, candidates.get(var).copied()) {
                    let says = read(of_var(head, &candidates, into, cells));
                    heard.hear(&candidates, *var, says);
                }
            }
            stmt.each_expr(&mut |top| uses(top, &mut heard));
        }
        if let Some(e) = block.term.expr() {
            uses(e, &mut heard);
        }
    }
    // ⚠ An address handed anywhere but to a comparison may write a value no kind is read off.
    for (var, (all, compares)) in addressed {
        if all > compares {
            heard.hear(&candidates, var, Says::Refuse);
        }
    }
    settled(&candidates, heard)
}

/// The root of `at` among the groups `up` joins, halving the way there.
// Each step is one link up a tree of at most `up.len()` members, so no more steps are needed.
fn root(up: &mut [usize], mut at: usize) -> usize {
    for _ in 0..up.len() {
        let next = up[at];
        if next == at {
            break;
        }
        up[at] = up[next];
        at = next;
    }
    at
}

/// The kind each candidate is of: the one named kind its group is said to be, where no member is refused and no two kinds are said.
fn settled(candidates: &Candidates, heard: Heard) -> Vec<(Var, Kind)> {
    let len = candidates.len();
    let mut up: Vec<usize> = (0..len).collect();
    for (a, b) in heard.same {
        let (a, b) = (root(&mut up, a), root(&mut up, b));
        if a != b {
            up[a.max(b)] = a.min(b);
        }
    }
    let mut refused = vec![false; len];
    let mut kind: Vec<Option<Kind>> = vec![None; len];
    let each = heard.refused.iter().zip(&heard.kinds).enumerate();
    for (at, (no, kinds)) in each {
        let top = root(&mut up, at);
        refused[top] |= *no;
        for said in kinds {
            // Two kinds said of one group: neither.
            let clash = kind[top].as_ref().is_some_and(|held| held != said);
            refused[top] |= clash;
            if kind[top].is_none() {
                kind[top] = Some(said.clone());
            }
        }
    }
    let mut found = Vec::new();
    for (var, (cells, at)) in candidates {
        let top = root(&mut up, *at);
        match &kind[top] {
            Some(held) if !refused[top] && kind_cells(held) == Some(*cells) => {
                found.push((*var, held.clone()));
            }
            _ => {}
        }
    }
    found
}

/// Gives each variable of several cells `types` left of no named kind the one kind every value copied into or out of it, and every value it is compared with whole, names.
// A kind is the variable's declaration alone: its cells, and so what the code does, stay as they were.
pub fn copied_kinds(f: &mut cfg::Function, cx: &Context<'_>, _stats: &mut Stats) {
    let on = cx.options.on(Pass::Aggregates) && cx.options.on(Pass::Kinds);
    if !on || f.head.incomplete.is_some() {
        return;
    }
    for (var, kind) in found(f, cx.project, cx.names) {
        if let Some(info) = f.head.vars.get_mut(&var) {
            info.kind = kind;
        }
    }
}

#[cfg(test)]
#[path = "aggregates_kinds_tests.rs"]
mod tests;
