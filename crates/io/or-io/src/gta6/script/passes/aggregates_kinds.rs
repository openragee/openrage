//! The kind of a value of several cells by the values it is copied from and into and compared with.
//!
//! Runs after `types`, which settles what the code's own forms state: a frame variable of several
//! cells it left of no named kind takes the one kind every one of those others names, and every
//! parameter it is handed to whole whose signature, or natives list, names one.

use std::collections::BTreeMap;

use super::kind_cells;
use crate::gta6::script::cfg;
use crate::gta6::script::ir::{Callee, Expr, Head, Kind, Stmt, Var, Width};
use crate::gta6::script::names::Names;
use crate::gta6::script::passes::types::listed_wide;
use crate::gta6::script::project::{signatures, Param, Project};
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
// ⚠ A native a list states another result of, and an intrinsic, are `types`' to read: neither
// says nothing, for its kind may be one this pass does not read.
fn of_call(head: &Head, project: &Project, names: &Names, callee: &Callee, cells: u32) -> Says {
    match callee {
        Callee::Native { hash, .. } => {
            let listed = names.native(*hash).and_then(|n| n.returns.as_deref());
            match listed.map(listed_wide) {
                None => Says::Nothing,
                Some(Some(kind)) if kind_cells(&kind) == Some(cells) => Says::Kind(kind),
                Some(_) => Says::Refuse,
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

/// The cells an argument fills, where its form says; `None` where nothing states them, or it fills none.
fn arg_cells(head: &Head, e: &Expr, depth: u32) -> Option<u32> {
    let deeper = depth.checked_sub(1)?;
    let cells = match e {
        Expr::Sugar { e: inner, .. } => return arg_cells(head, inner, deeper),
        Expr::Var(var) => match head.vars.get(var) {
            Some(info) if info.width == Width::Cells => kind_cells(&info.kind),
            _ => Some(1),
        },
        Expr::Cells(items) => items.iter().try_fold(0u32, |sum, item| {
            sum.checked_add(arg_cells(head, item, deeper)?)
        }),
        Expr::Select { then, els, .. } => {
            let cells = arg_cells(head, then, deeper)?;
            (arg_cells(head, els, deeper) == Some(cells)).then_some(cells)
        }
        Expr::BuiltinGlobal(_) => None,
        Expr::GuidEmpty | Expr::Vector3(_) => Some(2),
        Expr::ZeroSlots(cells) => Some(*cells),
        Expr::LoadBytes { bytes, .. } | Expr::ExtractBytes { bytes, .. } => Some(bytes.div_ceil(8)),
        Expr::Call { returns, .. } => *returns,
        _ => Some(1),
    };
    cells.filter(|cells| *cells > 0)
}

/// Each parameter's cells and the kind it is of, where that is a named kind of as many cells.
fn stated(params: &[Param]) -> Vec<(u32, Option<Kind>)> {
    let kind = |param: &Param| {
        let fits = named(&param.kind) && kind_cells(&param.kind) == Some(param.cells);
        fits.then(|| param.kind.clone())
    };
    params
        .iter()
        .map(|param| (param.cells, kind(param)))
        .collect()
}

/// The kind of the parameter each argument fills, by argument: one that starts at the argument's first cell and takes as many cells.
// ⚠ Only where the arguments fill every cell the parameters take, and no more: a call that fits
// otherwise says nothing of any argument.
fn answered(filled: &[u32], params: &[(u32, Option<Kind>)]) -> Option<Vec<Option<Kind>>> {
    let mut starts: BTreeMap<u32, (u32, Option<&Kind>)> = BTreeMap::new();
    let mut taken = 0u32;
    for (cells, kind) in params {
        starts.entry(taken).or_insert((*cells, kind.as_ref()));
        taken = taken.checked_add(*cells)?;
    }
    let mut cell = 0u32;
    let mut found = Vec::with_capacity(filled.len());
    for cells in filled {
        let kind = match starts.get(&cell) {
            Some((held, kind)) if held == cells => kind.cloned(),
            _ => None,
        };
        found.push(kind);
        cell = cell.checked_add(*cells)?;
    }
    (cell == taken).then_some(found)
}

/// What a call hands each argument to, by argument: the named kind of several cells its callee's signature, or the natives list, states for the parameter the argument fills whole.
fn handed(
    head: &Head,
    project: &Project,
    names: &Names,
    callee: &Callee,
    args: &[Expr],
) -> Vec<Option<Kind>> {
    let none = || vec![None; args.len()];
    let filled: Option<Vec<u32>> = args.iter().map(|arg| arg_cells(head, arg, DEPTH)).collect();
    let Some(filled) = filled else {
        return none();
    };
    let params: Vec<(u32, Option<Kind>)> = match callee {
        Callee::Native { hash, .. } => match names.native(*hash) {
            Some(native) => (native.params.iter())
                .map(|param| match listed_wide(&param.ty) {
                    Some(kind) => (2, Some(kind)),
                    None => (1, None),
                })
                .collect(),
            None => return none(),
        },
        _ => {
            let key = signatures::key_of(head.program, callee);
            match key.and_then(|key| project.signatures.get(&key)) {
                Some(own) => stated(&own.params),
                None => return none(),
            }
        }
    };
    answered(&filled, &params).unwrap_or_else(none)
}

/// The candidate an access of `bytes` bytes at `offset` through `addr` lies within, where `addr` is the address of one of its cells.
fn within(candidates: &Candidates, addr: &Expr, offset: u32, bytes: u32) -> Option<Var> {
    let Expr::AddrOf { var, cell } = addr else {
        return None;
    };
    let (cells, _) = candidates.get(var)?;
    let start = 8 * u64::from(*cell) + u64::from(offset);
    let inside = bytes > 0 && start + u64::from(bytes) <= 8 * u64::from(*cells);
    inside.then_some(*var)
}

/// The candidate a statement writes a field of through the address of one of its cells: a scalar, at most one cell's bytes, or zeroes, all within it.
// ⚠ Not `store_f32_slots`, which states no count of bytes, and no store of more than a cell of a
// value, which may bring in a part of a value of another kind; zero cells bring in none.
fn stored_within(candidates: &Candidates, stmt: &Stmt) -> Option<Var> {
    match stmt {
        Stmt::Store {
            kind, addr, offset, ..
        } => within(candidates, addr, *offset, kind.bytes()),
        Stmt::StoreBytes {
            addr,
            offset,
            bytes,
            value: Expr::ZeroSlots(cells),
        } if u64::from(*bytes) <= 8 * u64::from(*cells) => {
            within(candidates, addr, *offset, *bytes)
        }
        Stmt::StoreBytes {
            addr,
            offset,
            bytes,
            ..
        } if *bytes <= 8 => within(candidates, addr, *offset, *bytes),
        Stmt::ZeroBytes { addr, bytes } => within(candidates, addr, 0, *bytes),
        Stmt::StepI32 { addr, offset, .. } => within(candidates, addr, *offset, 4),
        _ => None,
    }
}

/// What the ways in and the uses of each candidate say, by its place.
struct Heard {
    refused: Vec<bool>,
    kinds: Vec<Vec<Kind>>,
    same: Vec<(usize, usize)>,
    /// Whether a field of it is read or written through its address.
    fielded: Vec<bool>,
}

impl Heard {
    fn new(len: usize) -> Heard {
        Heard {
            refused: vec![false; len],
            kinds: vec![Vec::new(); len],
            same: Vec::new(),
            fielded: vec![false; len],
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

/// How the addresses of one candidate are taken.
#[derive(Clone, Copy, Debug, Default)]
struct Addresses {
    all: u64,
    /// Those a comparison of its bytes reads.
    compared: u64,
    /// Those a field of it is read or written through.
    fields: u64,
}

/// What one function is read with.
struct Reader<'a> {
    head: &'a Head,
    project: &'a Project,
    names: &'a Names,
    candidates: &'a Candidates,
}

impl Reader<'_> {
    /// Hears what one expression says, those inside it aside.
    fn note(&self, e: &Expr, heard: &mut Heard, addressed: &mut BTreeMap<Var, Addresses>) {
        let candidates = self.candidates;
        // A candidate handed whole to a parameter of a named kind is of that kind.
        if let Expr::Call { callee, args, .. } = e {
            let any = |arg: &Expr| matches!(arg, Expr::Var(var) if candidates.contains_key(var));
            if args.iter().any(any) {
                let kinds = handed(self.head, self.project, self.names, callee, args);
                for (arg, kind) in args.iter().zip(kinds) {
                    if let (Expr::Var(var), Some(kind)) = (arg, kind) {
                        heard.hear(candidates, *var, Says::Kind(kind));
                    }
                }
            }
        }
        match e {
            Expr::AddrOf { var, .. } => {
                if candidates.contains_key(var) {
                    addressed.entry(*var).or_default().all += 1;
                }
            }
            Expr::Load { ty, addr, offset } => {
                if let Some(var) = within(candidates, addr, *offset, ty.bytes()) {
                    addressed.entry(var).or_default().fields += 1;
                }
            }
            Expr::LoadBytes {
                addr,
                offset,
                bytes,
            } => {
                if let Some(var) = within(candidates, addr, *offset, *bytes) {
                    addressed.entry(var).or_default().fields += 1;
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
                        addressed.entry(var).or_default().compared += 1;
                    }
                }
                for (var, other) in [(a, b), (b, a)] {
                    if let Some((var, says)) = compared(self.head, candidates, bytes, var, other) {
                        heard.hear(candidates, var, says);
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
                        heard.hear(candidates, var, Says::Kind(Kind::Handle));
                    }
                }
            }
            _ => {}
        }
    }
}

/// The kinds the candidates of a function are found of.
fn found(f: &cfg::Function, project: &Project, names: &Names) -> Vec<(Var, Kind)> {
    let head = &f.head;
    let candidates = candidates(head);
    if candidates.is_empty() {
        return Vec::new();
    }
    let reader = Reader {
        head,
        project,
        names,
        candidates: &candidates,
    };
    let mut heard = Heard::new(candidates.len());
    let mut addressed: BTreeMap<Var, Addresses> = BTreeMap::new();
    let uses = |top: &Expr, heard: &mut Heard, addressed: &mut BTreeMap<Var, Addresses>| {
        top.walk(&mut |e| reader.note(e, heard, addressed));
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
            if let Some(var) = stored_within(&candidates, stmt) {
                addressed.entry(var).or_default().fields += 1;
            }
            stmt.each_expr(&mut |top| uses(top, &mut heard, &mut addressed));
        }
        if let Some(e) = block.term.expr() {
            uses(e, &mut heard, &mut addressed);
        }
    }
    // ⚠ An address handed anywhere but to a comparison, or to an access of a field within the
    // value, may write a value no kind is read off.
    for (var, held) in addressed {
        if held.all > held.compared.saturating_add(held.fields) {
            heard.hear(&candidates, var, Says::Refuse);
        }
        let at = candidates.get(&var).map(|(_, at)| *at);
        let fielded = at.and_then(|at| heard.fielded.get_mut(at));
        if let (true, Some(fielded)) = (held.fields > 0, fielded) {
            *fielded = true;
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

/// The kind each candidate is of: the one named kind its group is said to be, where no member is refused and no two kinds are said, and no handle a field of which is used.
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
    let mut fielded = vec![false; len];
    let mut kind: Vec<Option<Kind>> = vec![None; len];
    let each = heard.refused.iter().zip(&heard.kinds).enumerate();
    for (at, (no, kinds)) in each {
        let top = root(&mut up, at);
        refused[top] |= *no;
        fielded[top] |= heard.fielded.get(at).copied().unwrap_or(false);
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
        // ⚠ A handle is whole wherever it is used: one a field of which is read or written is
        // no handle.
        let opaque = |held: &Kind| !(fielded[top] && *held == Kind::Handle);
        match &kind[top] {
            Some(held) if !refused[top] && opaque(held) && kind_cells(held) == Some(*cells) => {
                found.push((*var, held.clone()));
            }
            _ => {}
        }
    }
    found
}

/// Gives each variable of several cells `types` left of no named kind the one kind every value copied into or out of it, every value it is compared with whole and every parameter it is handed to whole names.
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
