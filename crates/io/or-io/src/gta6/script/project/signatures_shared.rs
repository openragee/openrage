//! What the functions of a project say of each other once every part is in: which fill one
//! virtual slot and so share a signature, which have callers no call names, how each result is
//! read over all its calls, and what every call passes for each parameter.

use std::collections::{BTreeMap, BTreeSet};

use super::{holds, number, param_at, settled, Found, Open, Usage, MAX_PARAMS, ROUNDS};
use crate::gta6::script::ir::{FunctionKey, Kind};
use crate::gta6::script::project::Project;

/// What is gathered over the whole project before any signature takes a kind from another.
pub(super) struct Shared {
    /// The functions that fill one virtual slot down a line of types, or one slot of an interface, where they are two or more: under the least of them.
    families: BTreeMap<FunctionKey, Vec<FunctionKey>>,
    /// Every function a virtual slot holds, with the least function of its line.
    roots: BTreeMap<FunctionKey, FunctionKey>,
    /// The functions a call may reach without naming them: through a slot, or through a reference taken to them.
    blind: BTreeSet<FunctionKey>,
    /// How the result of each function is read, over every call that names it and every function that hands it on.
    uses: BTreeMap<FunctionKey, Usage>,
}

impl Shared {
    pub(super) fn of(project: &Project, open: &BTreeMap<FunctionKey, Open>) -> Shared {
        let mut lines: BTreeMap<FunctionKey, FunctionKey> = BTreeMap::new();
        for facts in project.types.values() {
            for (slot, function) in &facts.slots {
                lines.entry(*function).or_insert(*function);
                // A table only grows down a line, so the base's function in this slot is the one overridden.
                let above = facts.base.and_then(|base| project.slot(base, *slot));
                if let Some(above) = above {
                    join(&mut lines, *function, above);
                }
            }
        }
        for slots in project.implemented.values() {
            for functions in slots.values() {
                let Some(first) = functions.first().copied() else {
                    continue;
                };
                for function in functions {
                    join(&mut lines, first, *function);
                }
            }
        }
        let members: Vec<FunctionKey> = lines.keys().copied().collect();
        let mut families: BTreeMap<FunctionKey, Vec<FunctionKey>> = BTreeMap::new();
        let mut roots = BTreeMap::new();
        for member in &members {
            let under = least(&mut lines, *member);
            families.entry(under).or_default().push(*member);
            roots.insert(*member, under);
        }
        families.retain(|_, members| members.len() >= 2);

        let mut blind: BTreeSet<FunctionKey> = members.into_iter().collect();
        for waits in open.values() {
            blind.extend(waits.taken.iter().copied());
        }
        Shared {
            families,
            roots,
            blind,
            uses: uses(open),
        }
    }

    /// The kind every call of `key` reads its result as, where each constant of `found` is a value of that kind.
    // ⚠ Nothing for a function a slot or a delegate reaches: the calls that name it are not all its calls.
    pub(super) fn read_as(&self, key: &FunctionKey, found: &Found) -> Option<Kind> {
        if self.blind.contains(key) {
            return None;
        }
        self.uses.get(key).and_then(|usage| usage.kind(found))
    }

    /// Whether every call that may reach `key` reads its result for whether it is zero alone: every call names it, and none reads it as anything else.
    pub(super) fn read_only_as_truth(&self, key: &FunctionKey) -> bool {
        let usage = self.uses.get(key).copied().unwrap_or_default();
        !self.blind.contains(key) && matches!(usage, Usage::Truth | Usage::Unsaid)
    }
}

/// Notes for each function a virtual slot holds the kind of the one cell every function of its line hands back, where all of them prove the same.
// ⚠ A call through the slot reaches one function of the line, whichever type the object is:
// one function that proves nothing, or another kind, and the call yields no kind.
pub(super) fn slot_results(project: &mut Project, shared: &Shared) {
    let mut agreed: BTreeMap<FunctionKey, Option<Kind>> = BTreeMap::new();
    for (member, root) in &shared.roots {
        let held = project.signatures.get(member);
        let proved = held.filter(|held| settled(held) && held.returns == 1);
        let kind = proved.map(|held| held.result.clone());
        let kind = kind.filter(|kind| *kind != Kind::Unknown);
        let line = agreed.entry(*root).or_insert_with(|| kind.clone());
        if *line != kind {
            *line = None;
        }
    }
    for (member, root) in &shared.roots {
        if let Some(Some(kind)) = agreed.get(root) {
            project.slot_results.insert(*member, kind.clone());
        }
    }
}

/// Notes for each function the parameter cells every call of it passes an `int` widened by its sign: an argument so by its form, or a parameter cell of the caller so proved.
// ⚠ Only where every call names the function: one a slot or a delegate reaches has callers no
// code names. A cell no call passes anything to is proved nothing.
pub(super) fn widened(project: &mut Project, open: &BTreeMap<FunctionKey, Open>, shared: &Shared) {
    let mut cells: BTreeMap<(FunctionKey, u32), bool> = BTreeMap::new();
    let mut killed: BTreeSet<FunctionKey> = BTreeSet::new();
    for waits in open.values() {
        for (to, cell, narrow) in &waits.narrow {
            match *cell {
                u32::MAX => {
                    killed.insert(*to);
                }
                cell => *cells.entry((*to, cell)).or_insert(true) &= *narrow,
            }
        }
    }
    // A caller whose calls were not read says nothing of what it passes.
    for (caller, callees) in &project.calls {
        if !open.contains_key(caller) {
            killed.extend(callees.iter().copied());
        }
    }
    // A parameter passed on is so where the caller's cell is.
    let mut edges: Vec<((FunctionKey, u32), (FunctionKey, u32))> = Vec::new();
    for (key, waits) in open {
        for (cell, callee, at) in &waits.passes {
            cells.entry((*callee, *at)).or_insert(true);
            edges.push(((*key, *cell), (*callee, *at)));
        }
    }
    cells
        .retain(|(key, _), narrow| *narrow && !killed.contains(key) && !shared.blind.contains(key));
    loop {
        let before = cells.len();
        for (from, to) in &edges {
            if cells.contains_key(to) && !cells.contains_key(from) {
                cells.remove(to);
            }
        }
        if cells.len() == before {
            break;
        }
    }
    for (key, cell) in cells.into_keys() {
        project.widened.entry(key).or_default().insert(cell);
    }
}

/// The function `at` stands under, where that is a lesser one.
fn above(lines: &BTreeMap<FunctionKey, FunctionKey>, at: FunctionKey) -> Option<FunctionKey> {
    lines.get(&at).copied().filter(|up| *up < at)
}

/// The least function `key` is joined to, itself where it is joined to none.
// Every step goes to a lesser key, so both walks end.
fn least(lines: &mut BTreeMap<FunctionKey, FunctionKey>, key: FunctionKey) -> FunctionKey {
    let mut root = key;
    while let Some(up) = above(lines, root) {
        root = up;
    }
    let mut at = key;
    while let Some(up) = above(lines, at) {
        lines.insert(at, root);
        at = up;
    }
    root
}

/// Joins the functions `a` and `b` are joined to, under the least of them all.
fn join(lines: &mut BTreeMap<FunctionKey, FunctionKey>, a: FunctionKey, b: FunctionKey) {
    // Each is held, so that the least of a line is among the line's own functions.
    lines.entry(a).or_insert(a);
    lines.entry(b).or_insert(b);
    let (a, b) = (least(lines, a), least(lines, b));
    if a != b {
        lines.insert(a.max(b), a.min(b));
    }
}

/// How each function's result is read: by the calls that name it, and as the result of each function that hands it on is.
fn uses(open: &BTreeMap<FunctionKey, Open>) -> BTreeMap<FunctionKey, Usage> {
    let mut uses: BTreeMap<FunctionKey, Usage> = BTreeMap::new();
    for waits in open.values() {
        for (callee, usage) in &waits.uses {
            let held = uses.entry(*callee).or_default();
            *held = held.join(*usage);
        }
    }
    let mut moving = true;
    for _ in 0..ROUNDS {
        moving = false;
        for (key, waits) in open {
            let theirs = uses.get(key).copied().unwrap_or_default();
            if theirs == Usage::Unsaid {
                continue;
            }
            for callee in &waits.handed {
                let held = uses.entry(*callee).or_default();
                let joined = held.join(theirs);
                moving |= joined != *held;
                *held = joined;
            }
        }
        if !moving {
            break;
        }
    }
    if moving {
        // A reading still on its way down a chain of calls has not met every read.
        uses.clear();
    }
    uses
}

/// Gives each function of a line whose result is constants alone the kind the rest of the line states, where the whole line agrees.
// ⚠ Additive: a function whose own code proves its result keeps it whatever its line says.
pub(super) fn agreed(
    project: &mut Project,
    open: &BTreeMap<FunctionKey, Open>,
    kinds: &BTreeMap<FunctionKey, Found>,
    shared: &Shared,
) {
    for members in shared.families.values() {
        let mut kind: Option<Kind> = None;
        let mut constant: Vec<(FunctionKey, &Found)> = Vec::new();
        let mut whole = true;
        for member in members {
            let held = project.signatures.get(member);
            let Some(held) = held.filter(|held| settled(held) && held.returns == 1) else {
                whole = false;
                break;
            };
            if held.result != Kind::Unknown {
                if kind.as_ref().is_some_and(|kind| *kind != held.result) {
                    whole = false;
                    break;
                }
                kind = Some(held.result.clone());
                continue;
            }
            // Constants of its own alone: one that hands a call on was settled with that call.
            let alone = open
                .get(member)
                .is_some_and(|waits| waits.live && waits.returned.is_empty());
            let found = kinds.get(member).filter(|found| !found.dead && alone);
            let Some(found) = found.filter(|found| found.kind.is_none()) else {
                whole = false;
                break;
            };
            constant.push((*member, found));
        }
        let Some(kind) = kind.filter(|_| whole) else {
            continue;
        };
        if !constant.iter().all(|(_, found)| holds(&kind, found)) {
            continue;
        }
        for (member, _) in constant {
            if let Some(held) = project.signatures.get_mut(&member) {
                held.result = kind.clone();
            }
        }
    }
}

/// The kind a signature states for the parameter of one cell that starts at `cell`.
fn stated(project: &Project, key: &FunctionKey, cell: u32) -> Option<Kind> {
    let held = project.signatures.get(key)?;
    let param = held.params.get(param_at(held, cell)?)?;
    (param.cells == 1 && param.kind != Kind::Unknown).then(|| param.kind.clone())
}

/// Whether a signature holds a parameter of one cell at `cell` that nothing has typed.
fn untyped(project: &Project, key: &FunctionKey, cell: u32) -> bool {
    let param = project
        .signatures
        .get(key)
        .and_then(|held| held.params.get(param_at(held, cell)?));
    param.is_some_and(|param| param.cells == 1 && param.kind == Kind::Unknown)
}

/// Gives each parameter cell nothing has typed the one kind the arguments of every call of its function agree on, where that is no integer.
// A parameter passed on carries what it is to the parameter it lands in, and a parameter only
// passed on takes what that one is, so the kinds run both ways along a chain of calls until
// nothing changes. Two kinds anywhere along it are no kind.
// ⚠ A function a slot or a delegate reaches takes nothing from the calls that name it.
pub(super) fn passed(project: &mut Project, open: &BTreeMap<FunctionKey, Open>, shared: &Shared) {
    let facts: &Project = project;
    let takes =
        |key: &FunctionKey, cell: u32| !shared.blind.contains(key) && untyped(facts, key, cell);
    let mut kinds: BTreeMap<(FunctionKey, u32), Found> = BTreeMap::new();
    for waits in open.values() {
        for given in &waits.gives {
            if takes(&given.to, given.cell) {
                let found = given.found(facts);
                kinds
                    .entry((given.to, given.cell))
                    .or_default()
                    .join(&found);
            }
        }
    }
    // What a parameter cell is so far: what its signature states, else what was gathered for it.
    let is = |kinds: &BTreeMap<(FunctionKey, u32), Found>, key: &FunctionKey, cell: u32| {
        let stated = stated(facts, key, cell).map(Found::of);
        let held = stated.or_else(|| kinds.get(&(*key, cell)).cloned());
        held.map(Found::unnumbered)
    };
    let mut moving = true;
    for _ in 0..ROUNDS {
        moving = false;
        for (key, waits) in open {
            for (cell, callee, at) in &waits.passes {
                if !takes(callee, *at) {
                    continue;
                }
                if let Some(theirs) = is(&kinds, key, *cell) {
                    moving |= kinds.entry((*callee, *at)).or_default().join(&theirs);
                }
            }
            for (cell, callee, at) in &waits.forwards {
                if !untyped(facts, key, *cell) {
                    continue;
                }
                if let Some(theirs) = is(&kinds, callee, *at) {
                    moving |= kinds.entry((*key, *cell)).or_default().join(&theirs);
                }
            }
        }
        if !moving {
            break;
        }
    }
    if moving {
        return;
    }
    for ((key, cell), found) in kinds {
        let kind = found
            .kind
            .clone()
            .filter(|kind| !found.dead && holds(kind, &found));
        let (Some(kind), Some(held)) = (kind, project.signatures.get_mut(&key)) else {
            continue;
        };
        let param = param_at(held, cell).and_then(|n| held.params.get_mut(n));
        if let Some(param) = param.filter(|param| param.cells == 1 && param.kind == Kind::Unknown) {
            param.kind = kind;
        }
    }
}

/// Gives each parameter cell that some functions of a line type alike, and the rest leave untyped, that kind in all of them, where it is no integer.
// An override declares what the method it overrides declares; two kinds for one cell stay as each function has them.
pub(super) fn overridden(project: &mut Project, shared: &Shared) {
    for members in shared.families.values() {
        let Some(cells) = same_cells(project, members) else {
            continue;
        };
        for cell in 0..cells.min(MAX_PARAMS) {
            let mut kind: Option<Kind> = None;
            let mut wanted = false;
            let mut agree = true;
            for member in members {
                if untyped(project, member, cell) {
                    wanted = true;
                    continue;
                }
                // Another kind, cells that are no parameter of one cell there, or an integer,
                // whose sign each function's own reads say.
                let theirs = stated(project, member, cell).filter(|kind| !number(kind));
                let differs = kind
                    .as_ref()
                    .is_some_and(|mine| theirs.as_ref() != Some(mine));
                if theirs.is_none() || differs {
                    agree = false;
                    break;
                }
                kind = theirs;
            }
            let (true, true, Some(kind)) = (agree, wanted, kind) else {
                continue;
            };
            for member in members {
                let Some(held) = project.signatures.get_mut(member) else {
                    continue;
                };
                let param = param_at(held, cell).and_then(|n| held.params.get_mut(n));
                let open = param.filter(|param| param.cells == 1 && param.kind == Kind::Unknown);
                if let Some(param) = open {
                    param.kind = kind.clone();
                }
            }
        }
    }
}

/// The parameter cells every function of a line takes, where each has a settled signature and all take as many.
fn same_cells(project: &Project, members: &[FunctionKey]) -> Option<u32> {
    let mut cells: Option<u32> = None;
    for member in members {
        let held = project
            .signatures
            .get(member)
            .filter(|held| settled(held))?;
        let own = held
            .params
            .iter()
            .try_fold(0u32, |sum, param| sum.checked_add(param.cells))?;
        if cells.is_some_and(|cells| cells != own) {
            return None;
        }
        cells = Some(own);
    }
    cells
}
