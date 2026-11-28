//! What the functions of a project say of each other once every part is in: which fill one
//! virtual slot and so share a signature, which have callers no call names, how each result is
//! read over all its calls, and what every call passes for each parameter.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    holds, number, param_at, resolved, settled, Back, Found, Open, Slot, Usage, ANY, MAX_PARAMS,
    ROUNDS,
};
use crate::gta6::script::ir::{FunctionKey, Kind};
use crate::gta6::script::project::{Param, Project, Signature};

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
    /// How the result of each line of a slot is read, by the least function of the line: over every call that names a function of it, and every call through the slot.
    line_uses: BTreeMap<FunctionKey, Usage>,
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

        // The functions a call no code names may reach, besides through a slot.
        let mut opaque: BTreeSet<FunctionKey> = BTreeSet::new();
        for waits in open.values() {
            opaque.extend(waits.taken.iter().copied());
        }
        // ⚠ Code no part read calls what it calls with what it holds: what its instructions name,
        // or, where they were not read either, any function of its program or of one it imports.
        let anywhere: BTreeSet<u32> = (project.unseen.iter())
            .filter(|key| key.at() == ANY)
            .map(|key| key.program())
            .collect();
        opaque.extend(project.unseen.iter().filter(|key| key.at() != ANY));
        let reached = project.signatures.keys().chain(project.calls.keys());
        opaque.extend(reached.filter(|key| anywhere.contains(&key.program())));
        let mut blind: BTreeSet<FunctionKey> = members.into_iter().collect();
        blind.extend(opaque.iter().copied());
        let reads = slot_reads(project, open, &roots, &opaque);
        let uses = uses(open, &opaque, &roots, &reads);
        // A function of a line may be called by name or through the slot.
        let mut line_uses = reads;
        for (member, root) in &roots {
            let usage = uses.get(member).copied().unwrap_or_default();
            let held = line_uses.entry(*root).or_default();
            *held = held.join(usage);
        }
        Shared {
            families,
            roots,
            blind,
            uses,
            line_uses,
        }
    }

    /// The kind every call that may reach `key`, a function a slot holds, reads its result as, where each constant of `found` is a value of that kind: the calls that name a function of its line and the calls through the slot.
    // ⚠ Nothing where a delegate or code no part read may reach a function of the line.
    pub(super) fn line_read_as(&self, key: &FunctionKey, found: &Found) -> Option<Kind> {
        let root = self.roots.get(key)?;
        self.line_uses.get(root).and_then(|usage| usage.kind(found))
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
    // A function a slot holds is read as its whole line is, and only a test of it says so.
    pub(super) fn read_only_as_truth(&self, key: &FunctionKey) -> bool {
        if let Some(root) = self.roots.get(key) {
            return self.line_uses.get(root) == Some(&Usage::Truth);
        }
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

/// The functions a virtual slot holds, by line: under the least function of each.
fn lines(shared: &Shared) -> BTreeMap<FunctionKey, Vec<FunctionKey>> {
    let mut lines: BTreeMap<FunctionKey, Vec<FunctionKey>> = BTreeMap::new();
    for (member, root) in &shared.roots {
        lines.entry(*root).or_default().push(*member);
    }
    lines
}

/// Gives the result `bool` to each function nothing else has typed whose every `return` is zero, one, a truth, or the result of a function or a slot so proved.
// The largest set that holds, found by taking out what fails until nothing does: every value a
// function so kept returns comes, down a finite chain of calls, from a truth, a zero or a one. A
// slot's result is so where every function of its line is, whichever the object's type.
pub(super) fn truths(project: &mut Project, open: &BTreeMap<FunctionKey, Open>, shared: &Shared) {
    let facts: &Project = project;
    let one = |key: &FunctionKey| {
        let held = facts.signatures.get(key);
        held.filter(|held| settled(held) && held.returns == 1)
    };
    let proved = |key: &FunctionKey| one(key).is_some_and(|held| held.result == Kind::Bool);
    let lines = lines(shared);
    // Each function that may be so, with the functions and the lines, by their least, it rests on.
    let mut rests: BTreeMap<FunctionKey, (Vec<FunctionKey>, Vec<FunctionKey>)> = BTreeMap::new();
    // The functions with a truth, a zero or a one of their own among what they return.
    let mut grounds: BTreeSet<FunctionKey> = BTreeSet::new();
    for (key, waits) in open {
        let Some(backs) = &waits.backs else {
            continue;
        };
        let open_result = |held: &Signature| matches!(held.result, Kind::Unknown | Kind::Bool);
        let untyped = one(key).is_some_and(open_result);
        let (mut calls, mut slots, mut whole) = (Vec::new(), Vec::new(), untyped);
        if backs.contains(&Back::Ground) {
            grounds.insert(*key);
        }
        for back in backs {
            match back {
                Back::Ground => {}
                Back::Call(callee) => calls.push(*callee),
                Back::Slot(ty, slot) => {
                    let held = facts.slot(*ty, *slot);
                    match held.and_then(|held| shared.roots.get(&held)) {
                        Some(root) => slots.push(*root),
                        None => whole = false,
                    }
                }
            }
        }
        if whole {
            rests.insert(*key, (calls, slots));
        }
    }
    let mut dead_lines: BTreeSet<FunctionKey> = BTreeSet::new();
    for (root, members) in &lines {
        let lost = |member: &FunctionKey| !rests.contains_key(member) && !proved(member);
        if members.iter().any(lost) {
            dead_lines.insert(*root);
        }
    }
    let mut callers: BTreeMap<FunctionKey, Vec<FunctionKey>> = BTreeMap::new();
    let mut users: BTreeMap<FunctionKey, Vec<FunctionKey>> = BTreeMap::new();
    for (key, (calls, slots)) in &rests {
        for callee in calls {
            callers.entry(*callee).or_default().push(*key);
        }
        for root in slots {
            users.entry(*root).or_default().push(*key);
        }
    }
    // Each function is taken out once, and each line lost once: every push follows one of those.
    let mut out: BTreeSet<FunctionKey> = BTreeSet::new();
    let mut queue: Vec<FunctionKey> = rests.keys().copied().collect();
    while let Some(key) = queue.pop() {
        let Some((calls, slots)) = rests.get(&key).filter(|_| !out.contains(&key)) else {
            continue;
        };
        let lost = |callee: &FunctionKey| match rests.contains_key(callee) {
            true => out.contains(callee),
            false => !proved(callee),
        };
        let fails = calls.iter().any(lost) || slots.iter().any(|root| dead_lines.contains(root));
        if !fails {
            continue;
        }
        out.insert(key);
        queue.extend(callers.get(&key).into_iter().flatten().copied());
        if let Some(root) = shared.roots.get(&key) {
            if dead_lines.insert(*root) {
                queue.extend(users.get(root).into_iter().flatten().copied());
            }
        }
    }
    // ⚠ The largest set also holds functions that only hand each other's results on, which
    // return nothing: only those a value of their own or a proved function reaches down their
    // calls and lines are so, the least set that holds.
    let mut reached: BTreeSet<FunctionKey> = BTreeSet::new();
    let mut lines_reached: BTreeSet<FunctionKey> = BTreeSet::new();
    for (root, members) in &lines {
        if members
            .iter()
            .any(|member| !rests.contains_key(member) && proved(member))
        {
            lines_reached.insert(*root);
        }
    }
    let seeded = |key: &FunctionKey, calls: &[FunctionKey], slots: &[FunctionKey]| {
        let outside = |callee: &FunctionKey| !rests.contains_key(callee) && proved(callee);
        grounds.contains(key)
            || calls.iter().any(outside)
            || slots.iter().any(|root| lines_reached.contains(root))
    };
    let mut queue: Vec<FunctionKey> = (rests.iter())
        .filter(|(key, (calls, slots))| !out.contains(key) && seeded(key, calls, slots))
        .map(|(key, _)| *key)
        .collect();
    // Each function is reached once, and each line once: every push follows one of those.
    while let Some(key) = queue.pop() {
        if out.contains(&key) || !reached.insert(key) {
            continue;
        }
        queue.extend(callers.get(&key).into_iter().flatten().copied());
        if let Some(root) = shared.roots.get(&key) {
            if lines_reached.insert(*root) {
                queue.extend(users.get(root).into_iter().flatten().copied());
            }
        }
    }
    let kept: Vec<FunctionKey> = rests
        .into_keys()
        .filter(|key| reached.contains(key))
        .collect();
    for key in kept {
        let held = project.signatures.get_mut(&key);
        if let Some(held) = held.filter(|held| held.result == Kind::Unknown) {
            held.result = Kind::Bool;
        }
    }
}

/// Notes for each function a virtual slot holds the parameters a call through the slot passes: each of the kind every function of its line that mentions it states, where all state one.
// ⚠ A function that mentions a cell and types it nothing reads all of it: the line then states
// no kind there. One that never mentions the cell reads none of it, whatever is passed.
pub(super) fn slot_params(
    project: &mut Project,
    open: &BTreeMap<FunctionKey, Open>,
    shared: &Shared,
) {
    for members in lines(shared).values() {
        let Some(params) = line_params(project, open, members) else {
            continue;
        };
        if params.iter().all(|param| param.kind == Kind::Unknown) {
            continue;
        }
        for member in members {
            project.slot_params.insert(*member, params.clone());
        }
    }
}

/// The parameters every function of one line takes alike, each of the kind all that mention it state; `None` where they take them otherwise.
fn line_params(
    project: &Project,
    open: &BTreeMap<FunctionKey, Open>,
    members: &[FunctionKey],
) -> Option<Vec<Param>> {
    let mut held = Vec::with_capacity(members.len());
    for member in members {
        let signature = project
            .signatures
            .get(member)
            .filter(|held| settled(held))?;
        let unread = open.get(member).map(|waits| waits.unread.as_slice());
        held.push((signature, unread.unwrap_or_default()));
    }
    let (first, _) = held.first()?;
    let shape: Vec<u32> = first.params.iter().map(|param| param.cells).collect();
    let alike = |signature: &&Signature| {
        let cells = signature.params.iter().map(|param| param.cells);
        cells.eq(shape.iter().copied())
    };
    if !held.iter().all(|(signature, _)| alike(signature)) {
        return None;
    }
    let mut params = Vec::with_capacity(shape.len());
    let mut cell = 0u32;
    for (n, cells) in shape.iter().copied().enumerate() {
        let mut kind: Option<&Kind> = None;
        let mut agree = true;
        for (signature, unread) in &held {
            if cells == 1 && unread.binary_search(&cell).is_ok() {
                continue;
            }
            let theirs = signature.params.get(n).map(|param| &param.kind);
            let differs = kind.is_some_and(|mine| Some(mine) != theirs);
            if theirs.is_none_or(|theirs| *theirs == Kind::Unknown) || differs {
                agree = false;
                break;
            }
            kind = theirs;
        }
        let kind = kind.filter(|_| agree).cloned().unwrap_or(Kind::Unknown);
        params.push(Param { kind, cells });
        cell = cell.checked_add(cells)?;
    }
    Some(params)
}

/// Notes for each function the parameter cells every call of it passes an `int` widened by its sign: an argument so by its form as first read, a parameter cell of the caller so proved, or the result of a function whose signature says it is so.
// ⚠ Only where every call names the function: one a slot or a delegate reaches has callers no
// code names. A cell no call passes anything to is proved nothing.
pub(super) fn widened(project: &mut Project, open: &BTreeMap<FunctionKey, Open>, shared: &Shared) {
    let facts: &Project = project;
    // A settled result of these kinds is a cell each value of it widens to as an `int` of it does.
    let int = |key: &FunctionKey| {
        let held = facts.signatures.get(key);
        held.is_some_and(|held| {
            let narrow = matches!(
                held.result,
                Kind::Bool | Kind::I8 | Kind::U8 | Kind::I16 | Kind::U16 | Kind::I32
            );
            settled(held) && held.returns == 1 && narrow
        })
    };
    let mut cells: BTreeMap<(FunctionKey, u32), bool> = BTreeMap::new();
    let mut killed: BTreeSet<FunctionKey> = BTreeSet::new();
    // A parameter passed on is so where the caller's cell is.
    let mut edges: Vec<((FunctionKey, u32), (FunctionKey, u32))> = Vec::new();
    for (key, waits) in open {
        for given in &waits.widening {
            if given.cell == u32::MAX {
                killed.insert(given.to);
                continue;
            }
            let proved = given.proved && given.results.iter().all(int);
            *cells.entry((given.to, given.cell)).or_insert(true) &= proved;
            for cell in &given.params {
                edges.push(((*key, *cell), (given.to, given.cell)));
            }
        }
    }
    // A caller whose calls were not read says nothing of what it passes.
    for (caller, callees) in &facts.calls {
        if !open.contains_key(caller) {
            killed.extend(callees.iter().copied());
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
// ⚠ What a function hands on is read wherever the function is: through its slot, by `reads`, and
// as anything where a delegate or code no part read may reach it.
fn uses(
    open: &BTreeMap<FunctionKey, Open>,
    opaque: &BTreeSet<FunctionKey>,
    roots: &BTreeMap<FunctionKey, FunctionKey>,
    reads: &BTreeMap<FunctionKey, Usage>,
) -> BTreeMap<FunctionKey, Usage> {
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
            let own = uses.get(key).copied().unwrap_or_default();
            let theirs = match (opaque.contains(key), roots.get(key)) {
                (true, _) => Usage::Mixed,
                (false, Some(root)) => own.join(reads.get(root).copied().unwrap_or_default()),
                (false, None) => own,
            };
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
        // A reading still on its way down a chain of calls has not met every read: each function
        // is read as anything, since a test alone, or no read, would prove a truth.
        let all = open.iter().flat_map(|(key, waits)| {
            let called = waits.uses.iter().map(|(callee, _)| callee);
            std::iter::once(key).chain(&waits.handed).chain(called)
        });
        uses = all.map(|key| (*key, Usage::Mixed)).collect();
    }
    uses
}

/// How the calls through each line's slot read its result, by the least function of the line; `Mixed` where a delegate or code no part read may reach a function of it.
// A call through a type reaches the function the type's slot holds or one that overrides it, all
// of one line, or one of the functions that implement the slot of an interface. ⚠ A call through
// a type no table resolves may reach any line of its slot, and a result handed on through a slot
// is read where the function that hands it is: as anything here.
fn slot_reads(
    project: &Project,
    open: &BTreeMap<FunctionKey, Open>,
    roots: &BTreeMap<FunctionKey, FunctionKey>,
    opaque: &BTreeSet<FunctionKey>,
) -> BTreeMap<FunctionKey, Usage> {
    let mut lines: BTreeMap<FunctionKey, Usage> = BTreeMap::new();
    // Code no part read may call through these slots, `ANY` through every one.
    let mut unresolved: BTreeSet<u32> = project.unseen_slots.clone();
    let read = |lines: &mut BTreeMap<FunctionKey, Usage>, root: FunctionKey, usage: Usage| {
        let held = lines.entry(root).or_default();
        *held = held.join(usage);
    };
    for (member, root) in roots {
        if opaque.contains(member) {
            read(&mut lines, *root, Usage::Mixed);
        }
    }
    let reached = |(ty, slot): &Slot| -> Vec<FunctionKey> {
        let Some(ty) = ty else {
            return Vec::new();
        };
        let held = project.slot(*ty, *slot);
        let implemented = project
            .implemented
            .get(ty)
            .and_then(|slots| slots.get(slot));
        held.into_iter()
            .chain(implemented.into_iter().flatten().copied())
            .collect()
    };
    // Each slot's lines and whether any function it reaches is of no line, worked out once: the
    // same slots are called by many functions.
    let mut known: BTreeMap<Slot, (Vec<FunctionKey>, bool)> = BTreeMap::new();
    for waits in open.values() {
        let called = waits.slot_uses.iter().map(|(slot, usage)| (slot, *usage));
        let handed_on = waits.handed_slots.iter().map(|slot| (slot, Usage::Mixed));
        for (slot, usage) in called.chain(handed_on) {
            let (rooted, open_ended) = known.entry(*slot).or_insert_with(|| {
                let targets = reached(slot);
                let rooted: Vec<FunctionKey> = targets
                    .iter()
                    .filter_map(|target| roots.get(target).copied())
                    .collect();
                let open_ended = rooted.is_empty() || rooted.len() != targets.len();
                (rooted, open_ended)
            });
            if *open_ended {
                unresolved.insert(slot.1);
            }
            for root in rooted.iter() {
                read(&mut lines, *root, usage);
            }
        }
    }
    let every = unresolved.contains(&ANY);
    for facts in project.types.values() {
        for (slot, function) in &facts.slots {
            let root = (roots.get(function)).filter(|_| every || unresolved.contains(slot));
            if let Some(root) = root {
                read(&mut lines, *root, Usage::Mixed);
            }
        }
    }
    for slots in project.implemented.values() {
        for (slot, functions) in slots {
            if !(every || unresolved.contains(slot)) {
                continue;
            }
            for root in functions.iter().filter_map(|function| roots.get(function)) {
                read(&mut lines, *root, Usage::Mixed);
            }
        }
    }
    lines
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
// ⚠ Every way a value arrives counts: a call whose arguments state no cells, or a caller whose
// calls were not read, says nothing of its callee, and an argument of no kind may be anything.
pub(super) fn passed(project: &mut Project, open: &BTreeMap<FunctionKey, Open>, shared: &Shared) {
    let facts: &Project = project;
    let mut killed: BTreeSet<FunctionKey> = BTreeSet::new();
    for waits in open.values() {
        let unsaid = waits.narrow.iter().filter(|(_, cell, _)| *cell == u32::MAX);
        killed.extend(unsaid.map(|(to, _, _)| *to));
    }
    for (caller, callees) in &facts.calls {
        if !open.contains_key(caller) {
            killed.extend(callees.iter().copied());
        }
    }
    let takes = |key: &FunctionKey, cell: u32| {
        !shared.blind.contains(key) && !killed.contains(key) && untyped(facts, key, cell)
    };
    let mut kinds: BTreeMap<(FunctionKey, u32), Found> = BTreeMap::new();
    let mut vague: BTreeSet<(FunctionKey, u32)> = BTreeSet::new();
    for waits in open.values() {
        for given in &waits.gives {
            if takes(&given.to, given.cell) {
                let found = given.found(facts);
                kinds
                    .entry((given.to, given.cell))
                    .or_default()
                    .join(&found);
                if given.vague(facts) {
                    vague.insert((given.to, given.cell));
                }
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
                if vague.contains(&(*key, *cell)) && stated(facts, key, *cell).is_none() {
                    moving |= vague.insert((*callee, *at));
                }
            }
            for (cell, callee, at) in &waits.forwards {
                if !untyped(facts, key, *cell) {
                    continue;
                }
                if let Some(theirs) = is(&kinds, callee, *at) {
                    moving |= kinds.entry((*key, *cell)).or_default().join_exact(&theirs);
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
        let kind = resolved(project, &found).filter(|kind| !found.dead && holds(kind, &found));
        // What may be anything beside objects of a type is a reference of no type.
        let named = matches!(kind, Some(Kind::Object(_) | Kind::Array(_)));
        let kind = match named && vague.contains(&(key, cell)) {
            true => Some(Kind::Pointer),
            false => kind,
        };
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
