//! The type registry: every type of an install under one key, with its base, its interfaces, the
//! generic type it is an instance of and the functions it is the first of its line to hold.
//!
//! A program numbers its types itself, so a type is first kept under its home, the program that
//! defines it and its id there, which an importer's id reaches through an import range.
//! [`settle`] then keys every type by its name, as code names it.
//!
//! ⚠ From [`of_program`] and [`of_function`] to [`settle`], the keys of `Project::types` and the
//! keys its facts name are homes and not names.
//! ⚠ Two programs may define one name. Its homes must state it alike or the name gets no facts,
//! and it gets no slot either way, since each home has functions of its own.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use super::{ImportInfo, ProgramInfo, Project, TypeFacts};
use crate::gta6::script::cfg;
use crate::gta6::script::ir::{Callee, Expr, FunctionKey, TypeKey, VmType};
use crate::gta6::script::names::Names;
use crate::gta6::script::program::{Program, SlotRow, TypeRow};

/// The base of a type only code has named: nothing states whether it has one.
const UNSTATED: TypeKey = TypeKey(u64::MAX);
/// The flag word of a home two statements disagree on; `settle` gives the name of such a home no facts.
const REFUSED: u32 = u32::MAX;
/// The flag of a type the runtime defines.
const BUILTIN: u32 = 0x2000;
/// The flag of a type no object is made of: an interface or an abstract class.
const NOT_ALLOCATED: u32 = 0x2_0000;
/// The flag bits a meaning is known for; two homes of one name must state these alike.
const READ_FLAGS: u32 = 0x1F | 0x40 | BUILTIN | NOT_ALLOCATED | 0x10_0000 | 0x40_0000;
/// The ancestors a type's slots are held against: no line is that long, and a ring of bases must end.
const MAX_BASES: usize = 64;
/// The argument types one printed name spells at most, theirs counted: a ring of arguments must end.
const MAX_SPELLED: u32 = 64;

/// Adds the rows of a compiled program's type table: each type's facts, and the program's own ids.
pub fn of_program(project: &mut Project, program: &Program) {
    let own = program.program;
    let info = project
        .programs
        .entry(own)
        .or_insert_with(|| program.info());
    let imports = info.imports.clone();
    let at = |id: u32| refer(own, &imports, id);
    let keys = |ids: &[u32]| -> Vec<TypeKey> { ids.iter().map(|id| at(*id)).collect() };
    for (id, row) in (0u32..).zip(&program.types) {
        // A key with no upper half is a builtin's index, never the hash of a name.
        if row.name >> 32 == 0 {
            continue;
        }
        info.types.insert(id, TypeKey(row.name));
        let generic = row.generic.as_ref();
        let said = TypeFacts {
            byte_size: row.byte_size,
            flags: row.flags,
            home: own,
            base: row.base.map(at),
            interfaces: keys(&row.interfaces),
            slots: first_held(program, row),
            generic: generic.map(|(of, args)| (at(*of), keys(args))),
            // As the table keeps them: widened with zeros from the enum's own width, whatever its sign.
            constants: row.constants.iter().map(|value| *value as i64).collect(),
            returns: stated_returns(program, row),
        };
        state(&mut project.types, home(own, id), said);
        // The slots of an interface are empty in its own table: a type that implements it holds
        // its methods from the slot the type's base list states for it.
        for (interface, first) in row.interfaces.iter().zip(&row.first_slots) {
            let next = row.first_slots.iter().filter(|other| *other > first).min();
            let end = next.map_or(row.vtable.len(), |next| *next as usize);
            let filled = project.implemented.entry(at(*interface)).or_default();
            let held = row
                .vtable
                .iter()
                .enumerate()
                .take(end)
                .skip(*first as usize);
            for (slot, holds) in held {
                let SlotRow::Function {
                    import, at: code, ..
                } = holds
                else {
                    continue;
                };
                if let Some(of) = program.imported(*import) {
                    let slot = slot as u32 - first;
                    (filled.entry(slot).or_default()).insert(FunctionKey::new(of, *code));
                }
            }
        }
    }
}

/// Adds the types one function's code names, with the id each has in the function's program.
pub fn of_function(project: &mut Project, f: &cfg::Function) {
    let program = f.head.program;
    let mut visit = |e: &Expr| {
        let ty = match e {
            Expr::NewObject(ty)
            | Expr::NewArray { ty, .. }
            | Expr::VirtualFunctionRef { ty, .. } => ty,
            Expr::Call {
                callee: Callee::Intrinsic { ty: Some(ty), .. },
                ..
            } => ty,
            _ => return,
        };
        listed(project, program, ty);
    };
    for block in &f.cfg.blocks {
        for stmt in &block.stmts {
            stmt.each_expr(&mut |e| e.walk(&mut visit));
        }
        if let Some(e) = block.term.expr() {
            e.walk(&mut visit);
        }
    }
}

/// Moves the types of `from` into `into`, each statement of a home held against what `into` says of it.
pub fn absorb(into: &mut Project, from: &mut Project) {
    for (key, said) in std::mem::take(&mut from.types) {
        state(&mut into.types, key, said);
    }
    for (interface, slots) in std::mem::take(&mut from.implemented) {
        let held = into.implemented.entry(interface).or_default();
        for (slot, functions) in slots {
            held.entry(slot).or_default().extend(functions);
        }
    }
}

/// Once every part is in: keys every type by its name, resolves what one program states by another's ids, and leaves out what its homes disagree on.
// ⚠ A type of a program the project does not hold has no name: a base, an interface or an
// argument that is one stays the home it was stated as, so a class never reads as having no base.
pub fn settle(project: &mut Project) {
    name_imports(project);
    let homes = std::mem::take(&mut project.types);
    let programs = &project.programs;
    let resolved = |key: TypeKey| named(programs, key).unwrap_or(key);
    // `None` once the homes of a name disagree.
    let mut settled: BTreeMap<TypeKey, Option<TypeFacts>> = BTreeMap::new();
    for (key, mut said) in homes {
        let name = match key.builtin() {
            Some(_) => Some(key),
            None => named(programs, key),
        };
        let Some(name) = name else {
            continue;
        };
        if said.flags == REFUSED {
            settled.insert(name, None);
            continue;
        }
        said.base = said.base.map(resolved);
        for key in &mut said.interfaces {
            *key = resolved(*key);
        }
        if let Some((of, args)) = &mut said.generic {
            *of = resolved(*of);
            for key in args {
                *key = resolved(*key);
            }
        }
        match settled.entry(name) {
            Entry::Vacant(free) => {
                free.insert(Some(said));
            }
            Entry::Occupied(mut held) => {
                let first = held.get_mut().take();
                *held.get_mut() = first.and_then(|first| one_type(first, said));
            }
        }
    }
    let stated = settled.into_iter().filter_map(|(name, facts)| {
        let mut facts = facts?;
        if facts.base == Some(UNSTATED) {
            // A type no object is made of reads as an interface once it has no base, and nothing says this one has none.
            if facts.flags & NOT_ALLOCATED != 0 {
                return None;
            }
            facts.base = None;
        }
        Some((name, facts))
    });
    project.types = stated.collect();
    // What implements an interface was stated of its home, as its types were.
    for (key, slots) in std::mem::take(&mut project.implemented) {
        let name = match key.builtin() {
            Some(_) => Some(key),
            None => named(&project.programs, key),
        };
        let Some(name) = name else {
            continue;
        };
        let held = project.implemented.entry(name).or_default();
        for (slot, functions) in slots {
            held.entry(slot).or_default().extend(functions);
        }
    }
}

/// Once the signatures are settled: gives each slot of an interface the cells a call of it returns, where every function a type fills the slot with returns the same, and no method record of the interface states them.
// ⚠ One implementer that returns otherwise leaves the slot unstated: a call of it is then sized by nothing, never by the most.
pub fn settle_implemented(project: &mut Project) {
    for (interface, slots) in std::mem::take(&mut project.implemented) {
        let Some(facts) = project.types.get_mut(&interface) else {
            continue;
        };
        for (slot, functions) in slots {
            let stated = functions.iter().filter_map(|f| project.signatures.get(f));
            let cells: BTreeSet<u32> = stated.map(|held| held.returns).collect();
            if let (Some(one), 1) = (cells.first(), cells.len()) {
                facts.returns.entry(slot).or_insert(*one);
            }
        }
    }
}

/// The name a type prints under: the user's, else its generic type and arguments as C# writes them, else `type_` and its hash.
pub fn display(project: &Project, names: &Names, key: TypeKey) -> String {
    let (mut out, mut left) = (String::new(), MAX_SPELLED);
    spell(project, names, key, &mut left, &mut out);
    out
}

/// The key a type is kept under until `settle`: the program that defines it and its id there.
fn home(program: u32, id: u32) -> TypeKey {
    TypeKey(u64::from(program) << 32 | u64::from(id))
}

/// What one of a program's type ids names: a builtin, or the home of an own or an imported type.
fn refer(program: u32, imports: &[ImportInfo], id: u32) -> TypeKey {
    if id & 0x8000_0000 != 0 {
        return TypeKey(u64::from(id));
    }
    let imported = imports.iter().find(|i| (i.first..i.end).contains(&id));
    match imported {
        Some(import) => home(import.program, id - import.first),
        None => home(program, id),
    }
}

/// The name of the type a home is, where the program that defines it is held and names that id.
fn named(programs: &BTreeMap<u32, ProgramInfo>, home: TypeKey) -> Option<TypeKey> {
    let info = programs.get(&((home.0 >> 32) as u32))?;
    info.types.get(&(home.0 as u32)).copied()
}

/// Whether a statement is a type's whole record, and not the size and flags code names it with.
fn whole(said: &TypeFacts) -> bool {
    said.base != Some(UNSTATED)
}

/// Holds one more statement of a home, or of a builtin, against what is already said of it.
// ⚠ A program's record stands whatever code says of its type, and replaces what only code said:
// code is read by a front end, and a front end that states a size wrongly must not unseat a record.
fn state(types: &mut BTreeMap<TypeKey, TypeFacts>, key: TypeKey, said: TypeFacts) {
    let held = match types.entry(key) {
        Entry::Vacant(free) => {
            free.insert(said);
            return;
        }
        Entry::Occupied(held) => held.into_mut(),
    };
    match (whole(held), whole(&said)) {
        (true, false) => {}
        (false, true) => *held = said,
        _ => {
            if !agree(held, said) {
                held.flags = REFUSED;
            }
        }
    }
}

/// Whether two statements hold together: one size and one flag word, and one record where both are records; `held` is left with the fuller.
fn agree(held: &mut TypeFacts, said: TypeFacts) -> bool {
    if (held.byte_size, held.flags) != (said.byte_size, said.flags) {
        return false;
    }
    match (whole(held), whole(&said)) {
        // Slots are left out: two readings of one home state the same, and two homes never do.
        (true, true) => {
            held.base == said.base
                && held.interfaces == said.interfaces
                && held.generic == said.generic
                && held.constants == said.constants
        }
        (false, true) => {
            *held = said;
            true
        }
        _ => true,
    }
}

/// What two homes of one name are together: one type of the first home where they state it alike, with no slot.
fn one_type(mut first: TypeFacts, mut said: TypeFacts) -> Option<TypeFacts> {
    if (first.flags ^ said.flags) & READ_FLAGS == 0 {
        // Of the bits no meaning is known for, the name is given those every home sets.
        let both = first.flags & said.flags;
        (first.flags, said.flags) = (both, both);
    }
    let home = first.home;
    // A slot's cells stand only where every home states the same of it.
    let theirs = std::mem::take(&mut said.returns);
    first
        .returns
        .retain(|slot, cells| theirs.get(slot) == Some(cells));
    if !agree(&mut first, said) {
        return None;
    }
    first.home = home;
    first.slots.clear();
    Some(first)
}

/// The functions a type is the first of its line to hold, by the slot it holds each in.
// ⚠ A function an ancestor's table holds in any slot is the ancestor's: a type that maps a new
// interface onto an inherited method holds that method in a slot the ancestor lacks.
// A table only grows down a line, so a slot keeps its number and `Project::slot` may ask a base.
fn first_held(program: &Program, row: &TypeRow) -> BTreeMap<u32, FunctionKey> {
    let mut inherited = BTreeSet::new();
    let mut base = row.base;
    for _ in 0..MAX_BASES {
        // An ancestor of another program holds none of this program's functions.
        let Some(above) = base.and_then(|id| program.types.get(id as usize)) else {
            break;
        };
        inherited.extend(above.slots.iter().flatten().copied());
        base = above.base;
    }
    let mut held = BTreeMap::new();
    for (slot, at) in (0u32..).zip(&row.slots) {
        if let Some(at) = at.filter(|at| !inherited.contains(at)) {
            held.insert(slot, FunctionKey::new(program.program, at));
        }
    }
    held
}

/// The cells each virtual slot returns, for the slots one method record of the type alone states.
fn stated_returns(program: &Program, row: &TypeRow) -> BTreeMap<u32, u32> {
    let own = row.methods.start as usize..row.methods.end as usize;
    let records = program.methods.get(own).unwrap_or(&[]);
    // `None` once two records state one slot.
    let mut stated: BTreeMap<u32, Option<u32>> = BTreeMap::new();
    for method in records {
        if let Some(slot) = method.slot {
            let first = stated.insert(slot, Some(method.returns)).is_none();
            if !first {
                stated.insert(slot, None);
            }
        }
    }
    let one = stated.into_iter();
    one.filter_map(|(slot, cells)| Some((slot, cells?)))
        .collect()
}

/// Holds a type a function's code names against its home: its size and flags, under the id the program gives it.
fn listed(project: &mut Project, program: u32, ty: &VmType) {
    let Some(name) = TypeKey::of(ty) else {
        return;
    };
    let mut said = TypeFacts {
        byte_size: ty.byte_size,
        flags: ty.flags,
        base: Some(UNSTATED),
        ..TypeFacts::default()
    };
    if name.builtin().is_some() {
        // A front end with no table of the runtime's types states one with no flags: that says nothing.
        if ty.flags & BUILTIN != 0 {
            state(&mut project.types, name, said);
        }
        return;
    }
    // Without the program's imports nothing says whose type the id is.
    let Some(info) = project.programs.get_mut(&program) else {
        return;
    };
    // One id of a program names one type, and a name has an upper half.
    if name.0 >> 32 == 0 || *info.types.entry(ty.id).or_insert(name) != name {
        return;
    }
    let key = refer(program, &info.imports, ty.id);
    said.home = (key.0 >> 32) as u32;
    state(&mut project.types, key, said);
}

/// Moves the name code gave an imported type to the program that defines it, under the id it has there.
// ⚠ A program names its own types whatever an importer's code says, and an id it does not name
// gets a name only where every importer gives it the same.
fn name_imports(project: &mut Project) {
    let mut said = Vec::new();
    for info in project.programs.values_mut() {
        let imports = &info.imports;
        info.types.retain(|id, name| {
            let import = imports.iter().find(|i| (i.first..i.end).contains(id));
            if let Some(import) = import {
                said.push((import.program, *id - import.first, *name));
            }
            import.is_none()
        });
    }
    // `None` once two importers name an id differently.
    let mut given: BTreeMap<(u32, u32), Option<TypeKey>> = BTreeMap::new();
    for (program, id, name) in said {
        let own = project.programs.get(&program);
        if own.is_some_and(|info| !info.types.contains_key(&id)) {
            let held = given.entry((program, id)).or_insert(Some(name));
            *held = held.filter(|held| *held == name);
        }
    }
    for ((program, id), name) in given {
        if let (Some(info), Some(name)) = (project.programs.get_mut(&program), name) {
            info.types.insert(id, name);
        }
    }
}

/// Writes a type's name, spending `left` on the arguments it spells.
fn spell(project: &Project, names: &Names, key: TypeKey, left: &mut u32, out: &mut String) {
    if let Some(name) = names.type_name(key.0) {
        out.push_str(name);
        return;
    }
    let facts = project.types.get(&key);
    let generic = facts.and_then(|facts| facts.generic.as_ref());
    // A type with more arguments than are left to spell keeps its own hash.
    let fits = generic.filter(|(_, args)| !args.is_empty() && args.len() <= *left as usize);
    let Some((of, args)) = fits else {
        let _ = write!(out, "{key}");
        return;
    };
    *left -= args.len() as u32;
    // The generic type prints under its own name: its record holds arguments too, those it was first made with.
    match names.type_name(of.0) {
        Some(name) => out.push_str(unmarked(name, args.len())),
        None => {
            let _ = write!(out, "{of}");
        }
    }
    for (n, arg) in args.iter().enumerate() {
        out.push_str(if n == 0 { "<" } else { ", " });
        spell(project, names, *arg, left, out);
    }
    out.push('>');
}

/// A generic type's name without the count of its arguments, where its end states that very count.
fn unmarked(name: &str, args: usize) -> &str {
    let bare = name.strip_suffix(&format!("_{args}"));
    bare.filter(|bare| !bare.is_empty()).unwrap_or(name)
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
