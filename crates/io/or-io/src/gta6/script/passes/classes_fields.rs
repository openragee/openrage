//! The proof of a member by the fields every call hands a function first: a reference field of
//! the receiver a member was given, which the project types as holding objects of one class.

use std::collections::{BTreeMap, BTreeSet};

use crate::gta6::script::class::{Carried, Role};
use crate::gta6::script::ir::{Expr, FunctionKey, Kind, TypeKey, Var};
use crate::gta6::script::project::{Member, Project, TypeFacts};

use super::{declarer, provisional, through, typed, whole, Arrives, MAX_CHAIN, REFERENCE};

/// The byte offset of the field of the receiver itself whose whole eight bytes a call's first argument loads.
pub(super) fn handed(args: &[Expr], receiver: Option<Var>) -> Option<u32> {
    let receiver = receiver?;
    match whole(args.first()?) {
        Expr::Load { ty, addr, offset } if ty.bytes() == REFERENCE && through(addr, receiver) => {
            Some(*offset)
        }
        _ => None,
    }
}

/// What the fields a function is handed say of the objects it is handed, so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fielded {
    /// A caller is itself only handed fields, and not settled yet.
    Waits,
    /// The nearest type every field handed is declared to hold.
    Holds(TypeKey),
    /// A caller is no member, or a field handed is of no one script class the project states.
    Free,
}

/// What the fields `function` is handed by each of its callers say of what it is handed, where `waiting` are the functions not settled yet.
// ⚠ A field's type is the most derived type the calls made on it are made as, so every object it
// holds is one of that type or of one made from it. The caller must be a member that takes a
// receiver, whose `this` is an object of its type or of one made from it: the field is that type's.
fn fields_of(
    project: &Project,
    carried: &Carried,
    members: &BTreeMap<FunctionKey, Member>,
    waiting: &BTreeSet<FunctionKey>,
    function: FunctionKey,
) -> Fielded {
    let mut held = Arrives::Nothing;
    for (caller, offset) in carried.fields.get(&function).into_iter().flatten() {
        if waiting.contains(caller) {
            return Fielded::Waits;
        }
        let Some(member) = members.get(caller).filter(|member| !member.is_static) else {
            return Fielded::Free;
        };
        let class = project.types.get(&member.owner);
        if class.is_none_or(TypeFacts::is_value) {
            return Fielded::Free;
        }
        let field = project.field(member.owner, *offset);
        let field = field.filter(|field| field.bytes == REFERENCE);
        let Some(Kind::Object(name)) = field.map(|field| &field.kind) else {
            return Fielded::Free;
        };
        let Some(ty) = TypeKey::of_name(name).filter(|ty| ty.builtin().is_none()) else {
            return Fielded::Free;
        };
        held = held.with(Arrives::One(ty), project);
    }
    match held {
        Arrives::One(ty) => Fielded::Holds(ty),
        Arrives::Nothing | Arrives::Unrelated => Fielded::Free,
    }
}

/// Settles the functions of `waiting`, every mention of which hands them, first, a reference field of a member's receiver: each is a method of the one type `declarer` names for the nearest type all those fields are declared to hold.
// A round settles the functions whose callers are all settled; one a ring of such functions
// calls, or one still waiting once the rounds run out, stays free.
pub(super) fn settle(
    project: &Project,
    carried: &Carried,
    members: &mut BTreeMap<FunctionKey, Member>,
    mut waiting: BTreeSet<FunctionKey>,
) {
    // The functions each function hands its receiver to.
    let mut hands: BTreeMap<FunctionKey, Vec<FunctionKey>> = BTreeMap::new();
    for (function, by) in &carried.handed {
        for by in by {
            hands.entry(*by).or_default().push(*function);
        }
    }
    for _ in 0..MAX_CHAIN {
        let mut decided: Vec<(FunctionKey, Option<TypeKey>)> = Vec::new();
        for function in &waiting {
            match fields_of(project, carried, members, &waiting, *function) {
                Fielded::Waits => {}
                Fielded::Holds(ty) => decided.push((*function, Some(ty))),
                Fielded::Free => decided.push((*function, None)),
            }
        }
        if decided.is_empty() {
            return;
        }
        for (function, arrived) in decided {
            waiting.remove(&function);
            let Some(arrived) = arrived else {
                continue;
            };
            let reach = carried.reach.get(&function).copied();
            let handed = hands.get(&function).map(Vec::as_slice);
            let needs = typed::needs(project, carried, members, handed, function);
            // ⚠ As in `received`: one that hands its receiver on may hand it to a method of a
            // type under the highest that may declare it.
            let several = handed.is_none_or(<[FunctionKey]>::is_empty);
            if let Some(owner) = declarer(project, arrived, function, reach, &needs, several) {
                members.insert(function, provisional(owner, Role::Method));
            }
        }
    }
}

#[cfg(test)]
#[path = "classes_fields_tests.rs"]
mod tests;
