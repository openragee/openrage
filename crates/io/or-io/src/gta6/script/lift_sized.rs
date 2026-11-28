//! The cells each call of a function takes and yields where its instruction states neither:
//! the walk from the entry that counts cells, what a project states of a virtual slot, and the
//! search for the one choice of counts that balances and keeps to the compiler's rules.

use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::super::decode::Insn;
use super::super::ir::{Callee, Expr, Scalar, TypeKey, Var};
use super::super::opcode::Op;
use super::super::project::Project;
use super::{shuffle, LiftError};

/// The most cells a call's result is tried at where no table states them.
const WIDEST: u32 = 40;

/// The instructions the search for the cells of such calls may walk, for each instruction of the function.
const WALKS: u64 = 1 << 15;

/// The instructions that search walks at most, whatever the function's size.
const WORK: u64 = 1 << 29;

/// The base types followed above a type before its line is given up: a ring of bases must end.
const BASES: usize = 64;

/// The cells the walk that holds a choice of element cells to the compiler's rules may keep at once; past them the rules are not asked.
const MARKS: usize = 1 << 22;

/// The times the walk of [`too_deep`] may visit each instruction of the function before it says nothing.
const VISITS: usize = 4;

/// What the walk from the entry found of each instruction.
pub(super) struct Reached {
    /// The cells on the stack before the instruction; `None` for one the entry does not reach.
    pub(super) depth: Vec<Option<u32>>,
    /// Whether a block starts at the instruction.
    pub(super) leader: Vec<bool>,
}

/// The cells a call takes and the cells it yields, by the offset of each instruction whose call states neither.
pub(super) type Sizes = BTreeMap<u32, (u32, u32)>;

/// Why a walk stopped.
enum Halt {
    Refused(LiftError),
    /// The instruction of this index holds a call whose cells are not chosen yet.
    Unsized(usize),
}

/// The cells a call of a virtual slot returns where neither the slot's type nor a base type of it states them: those the types below it state.
// A type that leaves a slot to the types derived from it holds no function and no record for
// it. Each type below fills the slot with a function of the slot's one signature, so the count
// every one of them that states a count states is the slot's.
// ⚠ Held against the counts the listings declare for the slots of script types: none differed.
// A slot of a type of the runtime is not asked.
pub(super) struct Slots<'a> {
    pub(super) project: &'a Project,
    /// Each slot asked for, by what the types below state of it.
    found: RefCell<BTreeMap<(TypeKey, u32), Said>>,
    /// The count each slot returns by the functions of the program that lifted alone, held for a slot no type below states.
    learned: RefCell<Learned>,
    /// The types each type is the base of, made at the first slot asked for where none is shared.
    children: OnceCell<Children>,
    /// The types each type is the base of, made once for the threads that lift one program.
    shared: Option<&'a Children>,
}

/// The types each type of a project is the base of.
pub(super) type Children = HashMap<TypeKey, Vec<TypeKey>>;

/// The types each type of `project` is the base of.
pub(super) fn children(project: &Project) -> Children {
    let mut children: Children = HashMap::new();
    for (key, facts) in &project.types {
        if let Some(base) = facts.base {
            children.entry(base).or_default().push(*key);
        }
    }
    children
}

/// What the types below a slot state of the cells it returns.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Said {
    Nothing,
    Count(u32),
    /// Two types below state different counts: the slot is no one signature, and nothing is taken for it.
    Two,
}

/// The cells each slot returns by the calls of it that functions lifted with: `None` where two functions took different counts.
pub(super) type Learned = BTreeMap<(TypeKey, u32), Option<u32>>;

// The two refusals a count learned from the other functions of the program may settle: each held
// count leaves fewer choices. One where no choice balances stays refused, since fewer never balance.

/// The refusal of a function that more than one choice of counts balances.
pub(super) const AMBIGUOUS: &str =
    "more than one count of cells for a call's result balances the stack";

/// The refusal of a function whose choices of counts are more than the search walks.
pub(super) const TOO_MANY: &str = "too many ways to size its calls";

impl<'a> Slots<'a> {
    pub(super) fn new(project: &'a Project) -> Self {
        Slots {
            project,
            found: RefCell::new(BTreeMap::new()),
            learned: RefCell::new(Learned::new()),
            children: OnceCell::new(),
            shared: None,
        }
    }

    /// The types each type of the project is the base of: those shared, else made now.
    pub(super) fn children_made(&self) -> &Children {
        match self.shared {
            Some(children) => children,
            None => self.children.get_or_init(|| children(self.project)),
        }
    }

    /// As [`Slots::new`], with the types each type is the base of made already.
    pub(super) fn sharing(project: &'a Project, children: &'a Children) -> Self {
        Slots {
            shared: Some(children),
            ..Slots::new(project)
        }
    }

    /// Holds the counts `learned` states from now on; whether any is new.
    pub(super) fn install(&self, learned: &Learned) -> bool {
        let mut held = self.learned.borrow_mut();
        let changed = *held != *learned;
        if changed {
            *held = learned.clone();
        }
        changed
    }

    /// The counts learned from other functions that size calls of `insns` no type below states, a slot once and in order.
    pub(super) fn bearing(&self, insns: &[Insn]) -> Vec<((TypeKey, u32), u32)> {
        let slots = insns
            .iter()
            .flat_map(|insn| insn.ops.iter().filter_map(slot_of));
        let wanted: BTreeSet<(TypeKey, u32)> = slots.map(|(slot, _)| slot).collect();
        self.ask(&wanted);
        let found = self.found.borrow();
        let learned = self.learned.borrow();
        let open = |slot: &&(TypeKey, u32)| found.get(*slot) == Some(&Said::Nothing);
        let known = |slot: &(TypeKey, u32)| Some((*slot, learned.get(slot).copied().flatten()?));
        wanted.iter().filter(open).filter_map(known).collect()
    }

    /// Finds every slot of `wanted` not asked for before, by the types below each slot's type.
    fn ask(&self, wanted: &BTreeSet<(TypeKey, u32)>) {
        let mut found = self.found.borrow_mut();
        let new = wanted.iter().filter(|key| !found.contains_key(*key));
        let new: BTreeSet<(TypeKey, u32)> = new.copied().collect();
        if new.is_empty() {
            return;
        }
        let types = &self.project.types;
        let children = self.children_made();
        // `None` once two types below have stated different counts.
        let mut said: BTreeMap<(TypeKey, u32), Option<u32>> = BTreeMap::new();
        let bases: BTreeSet<TypeKey> = new.iter().map(|(base, _)| *base).collect();
        for base in bases {
            // The types `base` stands at most BASES steps above, level by level, as the walk up from each of them meets it.
            let mut level: Vec<TypeKey> = children.get(&base).cloned().unwrap_or_default();
            for _ in 0..BASES {
                if level.is_empty() {
                    break;
                }
                for facts in level.iter().filter_map(|below| types.get(below)) {
                    for key in new.range((base, 0)..=(base, u32::MAX)) {
                        let slot = key.1;
                        let filled = facts.slots.get(&slot);
                        let function = filled.and_then(|at| self.project.signatures.get(at));
                        let record = facts.returns.get(&slot).copied();
                        let Some(cells) = record.or(function.map(|held| held.returns)) else {
                            continue;
                        };
                        let held = said.entry(*key).or_insert(Some(cells));
                        if *held != Some(cells) {
                            *held = None;
                        }
                    }
                }
                let next = level.iter().filter_map(|below| children.get(below));
                level = next.flatten().copied().collect();
            }
        }
        for key in new {
            let cells = match said.get(&key) {
                None => Said::Nothing,
                Some(Some(cells)) => Said::Count(*cells),
                Some(None) => Said::Two,
            };
            found.insert(key, cells);
        }
    }

    /// The cells a slot asked for returns: what the types below state, else what the program's other functions were found to take.
    fn returns(&self, slot: (TypeKey, u32)) -> Option<u32> {
        match self.found.borrow().get(&slot) {
            Some(Said::Count(cells)) => Some(*cells),
            Some(Said::Nothing) => self.learned.borrow().get(&slot).copied().flatten(),
            Some(Said::Two) | None => None,
        }
    }
}

/// The cells each call of a virtual slot among `insns` was sized with, by `sizes`, as the slot's count.
pub(super) fn counts(insns: &[Insn], sizes: &Sizes) -> Vec<((TypeKey, u32), u32)> {
    let mut counts = Vec::new();
    for (at, (_, returns)) in sizes {
        let found = insns.binary_search_by_key(at, |insn| insn.at);
        let Some(insn) = found.ok().and_then(|i| insns.get(i)) else {
            continue;
        };
        if let Some((slot, _)) = insn.ops.iter().find_map(slot_of) {
            counts.push((slot, *returns));
        }
    }
    counts
}

/// Adds what one function's calls took to what the program's functions agree on.
pub(super) fn learn(learned: &mut Learned, counts: &[((TypeKey, u32), u32)]) {
    for (slot, cells) in counts {
        let held = learned.entry(*slot).or_insert(Some(*cells));
        if *held != Some(*cells) {
            *held = None;
        }
    }
}

/// The slot a step calls and the cells the call takes, for a call of a script type's virtual slot that states no result.
fn slot_of(op: &Op) -> Option<((TypeKey, u32), u32)> {
    let Op::Unsized {
        callee: Callee::Virtual { ty, slot, .. },
        args: Some(args),
        returns: None,
        ..
    } = op
    else {
        return None;
    };
    let ty = TypeKey::of_name(ty).filter(|ty| ty.builtin().is_none())?;
    Some(((ty, *slot), *args))
}

/// The calls of virtual slots among `insns` that state no result and whose slot `slots` finds one for.
fn below(insns: &[Insn], slots: &Slots<'_>) -> Sizes {
    let open = |insn: &Insn| {
        let (slot, args) = insn.ops.iter().find_map(slot_of)?;
        Some((insn.at, slot, args))
    };
    let asked: Vec<(u32, (TypeKey, u32), u32)> = insns.iter().filter_map(open).collect();
    if asked.is_empty() {
        return Sizes::new();
    }
    let wanted: BTreeSet<(TypeKey, u32)> = asked.iter().map(|(_, slot, _)| *slot).collect();
    slots.ask(&wanted);
    let known = |(at, slot, args): (u32, (TypeKey, u32), u32)| {
        let returns = slots.returns(slot)?;
        Some((at, (args, returns)))
    };
    asked.into_iter().filter_map(known).collect()
}

/// One choice of the search of [`sized`]: the calls that take it, each by its instruction's offset with the cells it takes, and the count tried now.
struct Choice {
    sites: Vec<(u32, u32)>,
    cells: u32,
}

/// The choice the call of instruction `i` that states no result opens, at its least count; `None` where the call states no argument cells either.
// Every call of one virtual slot returns as many cells, so those not sized yet are sized with it.
fn open_at(insns: &[Insn], i: usize, sizes: &Sizes) -> Option<Choice> {
    let insn = insns.get(i)?;
    let (callee, args) = insn.ops.iter().find_map(|op| match op {
        Op::Unsized {
            callee,
            args: Some(args),
            returns: None,
            ..
        } => Some((callee, *args)),
        _ => None,
    })?;
    let cells = least(callee);
    let mut sites = vec![(insn.at, args)];
    if let Some((slot, _)) = insn.ops.iter().find_map(slot_of) {
        for other in insns {
            if other.at == insn.at || sizes.contains_key(&other.at) {
                continue;
            }
            let mut same = other.ops.iter().filter_map(slot_of);
            if let Some((_, args)) = same.find(|(of, _)| *of == slot) {
                sites.push((other.at, args));
            }
        }
    }
    Some(Choice { sites, cells })
}

/// The fewest cells a call whose result no table states is tried at.
// A value read out of an array is a cell at the least; a function may yield none.
fn least(callee: &Callee) -> u32 {
    u32::from(matches!(callee, Callee::Intrinsic { .. }))
}

/// Whether the counts of `sizes`, with every call they leave open at its least, already leave cells at a return on some way to it: then so does every count above them.
// A call whose count is not stated only adds cells, and every other step takes and leaves as many
// whatever is chosen, so the cells along one way only grow with each count. Where two ways meet,
// a choice that balances brings as many by both: the most either brings is no more than it.
// `false` says nothing: the walk was cut, or met a jump or a call it cannot count.
pub(super) fn too_deep(insns: &[Insn], sizes: &Sizes, spent: &mut u64) -> bool {
    let index = |to: u32| insns.binary_search_by_key(&to, |insn| insn.at).ok();
    // The fewest cells each instruction can find, for the instructions the walk reached.
    let mut fewest: Vec<Option<u64>> = vec![None; insns.len()];
    let Some(entry) = fewest.first_mut() else {
        return false;
    };
    *entry = Some(0);
    let mut work = vec![0usize];
    let mut visits = insns.len().saturating_mul(VISITS);
    while let Some(i) = work.pop() {
        let (Some(insn), Some(Some(found))) = (insns.get(i), fewest.get(i).copied()) else {
            continue;
        };
        visits = match visits.checked_sub(1) {
            Some(left) => left,
            None => return false,
        };
        *spent = spent.saturating_add(1);
        let mut cells = found;
        let mut next: Vec<usize> = Vec::new();
        let mut falls = true;
        for op in &insn.ops {
            let (pops, pushes) = match op {
                Op::Unsized {
                    callee,
                    args,
                    returns,
                } => match (args.zip(*returns), sizes.get(&insn.at), args) {
                    (Some(stated), _, _) => stated,
                    (None, Some(chosen), _) => *chosen,
                    (None, None, Some(args)) => (*args, least(callee)),
                    (None, None, None) => return false,
                },
                _ => op.stack(),
            };
            // A way that balances takes no cell it does not have: it holds at least as many as this takes.
            cells = cells
                .saturating_sub(u64::from(pops))
                .saturating_add(u64::from(pushes));
            match op {
                Op::Return { .. } if cells > 0 => return true,
                Op::Jump { to } | Op::Branch { to } => match index(*to) {
                    Some(to) => next.push(to),
                    None => return false,
                },
                Op::Switch { cases, default } => {
                    for to in cases.iter().map(|(_, to)| to).chain([default]) {
                        match index(*to) {
                            Some(to) => next.push(to),
                            None => return false,
                        }
                    }
                }
                _ => {}
            }
            if op.leaves() {
                falls = false;
                break;
            }
        }
        if falls && i + 1 < insns.len() {
            next.push(i + 1);
        }
        for to in next {
            if let Some(held) = fewest.get_mut(to) {
                if held.is_none_or(|had| cells > had) {
                    *held = Some(cells);
                    work.push(to);
                }
            }
        }
    }
    false
}

/// The walk of a function under the one choice of result cells, for the calls that state none, that lets the cells balance.
// Measured on the functions a listing sizes: where one choice alone balances, it is the listing's.
// A count is chosen for one call at a time, in the order [`elements`] meets them, and dropped
// as soon as what was walked up to the next such call does not balance or breaks a rule: a
// function of many array reads is then sized read by read, and not by every product of counts.
pub(super) fn sized(
    function: u32,
    insns: &[Insn],
    slots: &Slots<'_>,
) -> Result<(Reached, Sizes), LiftError> {
    let refuse = |at: u32, what: &str| LiftError::Stack {
        function,
        at,
        what: what.to_owned(),
    };
    // Most functions state every count: one walk is all there is to do.
    match reach(function, insns, &Sizes::new()) {
        Ok(reached) => return Ok((reached, Sizes::new())),
        Err(Halt::Refused(why)) => return Err(why),
        Err(Halt::Unsized(_)) => {}
    }
    // What the project states is no choice: it is held whatever else is tried.
    let mut sizes = below(insns, slots);
    let budget = (insns.len() as u64).saturating_mul(WALKS).min(WORK);
    let mut spent = 0u64;
    let mut found: Option<(Reached, Sizes)> = None;
    let mut choices: Vec<Choice> = Vec::new();
    loop {
        // Each walk lays out its tables for the whole function, however soon it stops.
        spent = spent.saturating_add(insns.len() as u64);
        if spent > budget {
            return Err(refuse(function, TOO_MANY));
        }
        let mut open = None;
        match elements(insns, &sizes, true, &mut spent) {
            Elements::Open(i) => open = Some(i),
            Elements::Broken => {}
            Elements::Short if !choices.is_empty() => {}
            Elements::Short | Elements::Hold => {
                spent = spent.saturating_add(2 * insns.len() as u64);
                // ⚠ Only ever turns a choice down: one the compiler's rules for an array's elements rule out.
                let ruled_out = |reached: &Reached| {
                    !choices.is_empty() && !elements_hold(insns, &sizes, reached)
                };
                match reach(function, insns, &sizes) {
                    Ok(reached) if ruled_out(&reached) => {}
                    Ok(_) if found.is_some() => return Err(refuse(function, AMBIGUOUS)),
                    Ok(reached) => found = Some((reached, sizes.clone())),
                    Err(Halt::Unsized(i)) => open = Some(i),
                    // With nothing chosen the refusal is the function's own.
                    Err(Halt::Refused(why)) if choices.is_empty() => return Err(why),
                    Err(Halt::Refused(_)) => {}
                }
            }
        }
        if let Some(i) = open {
            let Some(choice) = open_at(insns, i, &sizes) else {
                let at = insns.get(i).map_or(function, |insn| insn.at);
                return Err(refuse(at, "a call whose argument cells no table states"));
            };
            spent = spent.saturating_add(insns.len() as u64);
            for (at, args) in &choice.sites {
                sizes.insert(*at, (*args, choice.cells));
            }
            choices.push(choice);
            continue;
        }
        // The next count of the last choice that has one left; a choice with none left is undone.
        loop {
            let Some(last) = choices.last_mut() else {
                let none = "no count of cells for a call's result balances the stack";
                return found.ok_or_else(|| refuse(function, none));
            };
            if last.cells < WIDEST {
                last.cells += 1;
                for (at, args) in &last.sites {
                    sizes.insert(*at, (*args, last.cells));
                }
                // ⚠ Only ever turns counts down: this one, and every one above it, which adds more cells still.
                if !too_deep(insns, &sizes, &mut spent) {
                    break;
                }
            }
            for (at, _) in &last.sites {
                sizes.remove(at);
            }
            choices.pop();
        }
    }
}

/// Walks the instructions from the first, counting cells: every way into an instruction must find the same number.
fn reach(function: u32, insns: &[Insn], sizes: &Sizes) -> Result<Reached, Halt> {
    let refuse = |at: u32, what: String| Halt::Refused(LiftError::Stack { function, at, what });
    let Some(entry) = insns.first() else {
        return Err(refuse(
            function,
            "the function holds no instruction".to_owned(),
        ));
    };
    if entry.at != function {
        let what = "the first instruction is not at the function's offset".to_owned();
        return Err(refuse(entry.at, what));
    }
    let index = |to: u32, from: u32| {
        let found = insns.binary_search_by_key(&to, |insn| insn.at);
        found.map_err(|_| {
            refuse(
                from,
                format!("a jump to {to:06X}, where no instruction starts"),
            )
        })
    };
    let mut depth = vec![None; insns.len()];
    let mut leader = vec![false; insns.len()];
    depth[0] = Some(0);
    leader[0] = true;
    // An instruction enters the work list once, when its depth is first known.
    let mut work = vec![0usize];
    while let Some(i) = work.pop() {
        let insn = &insns[i];
        let mut cells = depth[i].unwrap_or(0);
        // Where control goes on: the instruction, the cells it finds, whether a jump names it.
        let mut next: Vec<(usize, u32, bool)> = Vec::new();
        let mut falls = true;
        let mut splits = false;
        for op in &insn.ops {
            let (pops, pushes) = match op {
                Op::Unsized { args, returns, .. } => {
                    match (args.zip(*returns), sizes.get(&insn.at)) {
                        (Some(stated), _) => stated,
                        (None, Some(chosen)) => *chosen,
                        (None, None) => return Err(Halt::Unsized(i)),
                    }
                }
                _ => op.stack(),
            };
            let needs = match op {
                Op::Copy {
                    depth: under,
                    cells: count,
                    ..
                } => under.saturating_add(*count),
                Op::AddrOfTop { cells: count, .. } => *count,
                _ => pops,
            };
            if cells < needs {
                let what = format!("a step takes {needs} cell(s) of a stack of {cells}");
                return Err(refuse(insn.at, what));
            }
            let Some(after) = (cells - pops).checked_add(pushes) else {
                return Err(refuse(insn.at, "more cells than a stack holds".to_owned()));
            };
            cells = after;
            match op {
                Op::Return { .. } if cells != 0 => {
                    let what = format!("a return leaves {cells} cell(s) on the stack");
                    return Err(refuse(insn.at, what));
                }
                Op::Jump { to, .. } | Op::Branch { to, .. } => {
                    next.push((index(*to, insn.at)?, cells, true));
                }
                Op::Switch { cases, default, .. } => {
                    for (_, to) in cases {
                        next.push((index(*to, insn.at)?, cells, true));
                    }
                    next.push((index(*default, insn.at)?, cells, true));
                }
                _ => {}
            }
            splits |= matches!(op, Op::Branch { .. });
            if op.leaves() {
                falls = false;
                break;
            }
        }
        if falls {
            if i + 1 >= insns.len() {
                let what = "control runs off the end of the function".to_owned();
                return Err(refuse(insn.at, what));
            }
            next.push((i + 1, cells, splits));
        }
        for (to, cells, named) in next {
            leader[to] |= named;
            match depth[to] {
                None => {
                    depth[to] = Some(cells);
                    work.push(to);
                }
                Some(had) if had != cells => {
                    let what = format!("reached with {had} cell(s) on the stack and with {cells}");
                    return Err(refuse(insns[to].at, what));
                }
                Some(_) => {}
            }
        }
    }
    Ok(Reached { depth, leader })
}

/// What the walk of [`elements`] keeps of one cell of the stack.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Mark {
    /// Which cell of an array element's value this is: 0 for its first, and for a cell of nothing such.
    part: u32,
    /// The array the cell holds, where it is one the walk follows.
    leaf: Option<u32>,
}

/// What a cell followed as an array is read from: a variable, or eight bytes at an offset through another such cell.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Leaf {
    Var(Var),
    Field(u32, u32),
}

/// Whether a choice of cells for the array elements a function reads keeps to the compiler's two rules.
// ⚠ Held against the functions a listing sizes, the rules turned down none of the listing's
// choices. Where the walk would keep more than `MARKS` cells the rules are not asked.
fn elements_hold(insns: &[Insn], sizes: &Sizes, reached: &Reached) -> bool {
    let kept = |sum: usize, depth: &Option<u32>| sum.saturating_add(depth.unwrap_or(0) as usize);
    if reached.depth.iter().fold(0usize, kept) > MARKS {
        return true;
    }
    elements(insns, sizes, false, &mut 0) != Elements::Broken
}

/// What the walk of [`elements`] found of a choice of cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Elements {
    /// Nothing the walk met turns the choice down.
    Hold,
    /// A rule of the compiler's turns it down.
    Broken,
    /// The cells do not balance on a way the walk took.
    Short,
    /// The walk stopped at the instruction of this index, which holds a call whose cells are not chosen.
    Open(usize),
}

/// Walks the instructions from the first with a mark for each cell, holding a choice of cells to the compiler's two rules for an array's elements.
// No instruction takes, copies or moves a part of one element's value, and every element read
// from or stored to one array is as many cells. An array is told by what it is read from: a
// variable, or a field loaded through one.
// `strict` is the walk of a choice not made whole yet: it stops at the first call with no count
// (`Open`) and says where the cells cannot balance (`Short`); without it both are `Hold`.
// ⚠ The walk takes the instructions in one order whatever is chosen, so all it found before an
// `Open` holds of every count chosen for that call and for those after it: a choice it turns
// down there is turned down whole.
pub(super) fn elements(insns: &[Insn], sizes: &Sizes, strict: bool, spent: &mut u64) -> Elements {
    let short = match strict {
        true => Elements::Short,
        false => Elements::Hold,
    };
    let index = |to: u32| insns.binary_search_by_key(&to, |insn| insn.at).ok();
    let mut leaves: BTreeMap<Leaf, u32> = BTreeMap::new();
    let mut leaf = |of: Leaf| {
        let next = leaves.len() as u32;
        *leaves.entry(of).or_insert(next)
    };
    // The cells of the elements of each array followed.
    let mut widths: BTreeMap<u32, u32> = BTreeMap::new();
    let mut agrees = |array: Option<u32>, cells: u32| match array {
        Some(array) => *widths.entry(array).or_insert(cells) == cells,
        None => true,
    };
    // An instruction is queued once, with the cells it was first reached with.
    let mut seen = vec![false; insns.len()];
    let mut arrived = vec![0usize; insns.len()];
    if let Some(first) = seen.first_mut() {
        *first = true;
    }
    // The cells of the stacks queued so far, held to `MARKS`.
    let mut kept = 0usize;
    let mut work: Vec<(usize, Vec<Mark>)> = vec![(0, Vec::new())];
    while let Some((i, mut stack)) = work.pop() {
        let Some(insn) = insns.get(i) else {
            continue;
        };
        *spent = spent.saturating_add(1);
        // The deepest cell the instruction takes: the first of a value, or it takes a part of one.
        let mut lowest = (stack.len(), Mark::default());
        let mut next: Vec<usize> = Vec::new();
        let mut falls = true;
        // The raises left of a run whose ends were held already.
        let mut raised = 0usize;
        for (k, op) in insn.ops.iter().enumerate() {
            let len = stack.len();
            let whole =
                |at: Option<usize>| at.and_then(|at| stack.get(at)).is_none_or(|m| m.part == 0);
            let under = |cells: u32| len.checked_sub(cells as usize);
            match op {
                Op::Copy { depth, cells } => {
                    let end = under(*depth);
                    let start = end.and_then(|end| end.checked_sub(*cells as usize));
                    if !whole(start) || (*depth > 0 && !whole(end)) {
                        return Elements::Broken;
                    }
                }
                Op::Swap if !(whole(under(1)) && whole(under(2))) => return Elements::Broken,
                Op::Sink { depth } if !(whole(under(1)) && whole(under(*depth))) => {
                    return Elements::Broken;
                }
                // A run of raises from one depth lifts the cells it passes over as one block, the
                // deepest first: what it moves is whole where the block starts and ends between two values.
                Op::Raise { .. } if raised > 0 => raised -= 1,
                Op::Raise { depth } => {
                    let run = insn.ops.iter().skip(k).take_while(|same| *same == op);
                    let run = run.count();
                    // A run as long as its depth turns the cells round and is no block.
                    let span = match run < *depth as usize {
                        true => run,
                        false => 1,
                    };
                    let from = under(*depth);
                    if !whole(from) || !whole(from.map(|from| from + span)) {
                        return Elements::Broken;
                    }
                    raised = span - 1;
                }
                Op::AddrOfTop { cells } => match under(*cells) {
                    None if strict => return Elements::Short,
                    at if !whole(at) => return Elements::Broken,
                    _ => {}
                },
                _ => {}
            }
            if matches!(
                op,
                Op::Copy { .. } | Op::Swap | Op::Sink { .. } | Op::Raise { .. }
            ) {
                if shuffle(&mut stack, op).is_err() {
                    return short;
                }
                continue;
            }
            let (pops, pushes) = match op {
                Op::Unsized { args, returns, .. } => {
                    match (args.zip(*returns), sizes.get(&insn.at)) {
                        (Some(stated), _) => stated,
                        (None, Some(chosen)) => *chosen,
                        (None, None) if strict => return Elements::Open(i),
                        (None, None) => return Elements::Hold,
                    }
                }
                _ => op.stack(),
            };
            let Some(floor) = under(pops) else {
                return short;
            };
            if let (true, Some(mark)) = (floor < lowest.0, stack.get(floor)) {
                lowest = (floor, *mark);
            }
            let mut made = vec![Mark::default(); pushes as usize];
            match op {
                Op::Unsized {
                    callee: Callee::Intrinsic { name, .. },
                    ..
                } if name == "array_get_value" => {
                    if !agrees(stack.get(floor).and_then(|array| array.leaf), pushes) {
                        return Elements::Broken;
                    }
                    for (part, mark) in made.iter_mut().enumerate() {
                        mark.part = part as u32;
                    }
                }
                Op::Call {
                    callee: Callee::Intrinsic { name, .. },
                    args,
                    ..
                } if name == "array_set_slots" && *args >= 2 => {
                    if !agrees(stack.get(floor).and_then(|array| array.leaf), args - 2) {
                        return Elements::Broken;
                    }
                }
                Op::Push {
                    e: Expr::Var(var),
                    cells: 1,
                } => made[0].leaf = Some(leaf(Leaf::Var(*var))),
                Op::Load { ty, offset } if ty.bytes() == 8 => {
                    let through = stack.last().and_then(|address| address.leaf);
                    made[0].leaf = through.map(|through| leaf(Leaf::Field(through, *offset)));
                }
                Op::Cast(Scalar::I64 | Scalar::U64) => {
                    made[0].leaf = stack.last().and_then(|m| m.leaf)
                }
                _ => {}
            }
            stack.truncate(floor);
            stack.append(&mut made);
            match op {
                Op::Return { .. } if strict && !stack.is_empty() => return Elements::Short,
                Op::Jump { to } | Op::Branch { to } => next.extend(index(*to)),
                Op::Switch { cases, default } => {
                    next.extend(cases.iter().filter_map(|(_, to)| index(*to)));
                    next.extend(index(*default));
                }
                _ => {}
            }
            if op.leaves() {
                falls = false;
                break;
            }
        }
        if lowest.1.part != 0 {
            return Elements::Broken;
        }
        if falls && i + 1 < insns.len() {
            next.push(i + 1);
        }
        // Two ways into one instruction that bring different counts balance under no choice.
        let cells = stack.len();
        if strict && next.iter().any(|to| seen[*to] && arrived[*to] != cells) {
            return Elements::Short;
        }
        next.retain(|to| !std::mem::replace(&mut seen[*to], true));
        for to in &next {
            arrived[*to] = cells;
        }
        kept = kept.saturating_add(cells.saturating_mul(next.len()));
        if kept > MARKS {
            return Elements::Hold;
        }
        while let Some(to) = next.pop() {
            match next.is_empty() {
                true => work.push((to, std::mem::take(&mut stack))),
                false => work.push((to, stack.clone())),
            }
        }
    }
    Elements::Hold
}
