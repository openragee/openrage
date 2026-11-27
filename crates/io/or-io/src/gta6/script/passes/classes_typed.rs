//! The fifth proof of a member, by the objects every call hands a function first, and what a
//! function's own code says of the classes its receiver is made from.

use std::collections::{BTreeMap, BTreeSet};

use crate::gta6::script::class::{Carried, Role};
use crate::gta6::script::ir::{Callee, Expr, FunctionKey, Head, Kind, TypeKey, Var, Width};
use crate::gta6::script::project::signatures::settled;
use crate::gta6::script::project::{Member, Project};

use super::{hands, whole};

/// Whether a call's first argument is a value of one cell whose kind the caller's own reading states: a variable of its frame, the result of one call, an object made.
// ⚠ Only of these does the signatures' join over the calls say what each one hands: a parameter
// passed on is what that function's callers hand, which the join takes only where its kind is
// known; a global any code may write; a choice is joined only on those of its ways whose kind is
// known; a value of several cells lands in more than the first parameter.
pub(super) fn handed_value(args: &[Expr], head: &Head) -> bool {
    let Some(first) = args.first() else {
        return false;
    };
    match whole(first) {
        Expr::Var(var @ (Var::Local(_) | Var::Value { .. } | Var::Stack { .. })) => head
            .vars
            .get(var)
            .is_none_or(|info| info.width == Width::Cell),
        Expr::Call { returns, .. } => *returns == Some(1),
        Expr::NewObject(_) => true,
        _ => false,
    }
}

/// Notes the type of each slot `top` calls on `receiver` itself.
pub(super) fn needed(top: &Expr, receiver: Var, needs: &mut BTreeSet<TypeKey>) {
    top.walk(&mut |e| {
        if let Expr::Call {
            callee: Callee::Virtual { ty, .. },
            args,
            ..
        } = e
        {
            if hands(args, Some(receiver)) {
                needs.extend(TypeKey::of_name(ty));
            }
        }
    });
}

/// Whether the function's own code types its first parameter as an object or an array: what its signature then states of that parameter is that code's, not its callers'.
pub(super) fn self_typed(head: &Head) -> bool {
    let first = head.vars.get(&Var::Param(0));
    first.is_some_and(|info| matches!(info.kind, Kind::Object(_) | Kind::Array(_)))
}

/// The script type of the objects the function's settled signature says every call hands it first, where those calls alone can have said it.
// ⚠ A signature states the kind its function's own code reads a parameter as, and what a
// parameter the function only passes on is handed where it is passed: neither is what callers
// hand. A function that reads or writes through its receiver does more than pass it on.
pub(super) fn handed_object(
    project: &Project,
    carried: &Carried,
    function: FunctionKey,
) -> Option<TypeKey> {
    if carried.self_typed.contains(&function) || !carried.reach.contains_key(&function) {
        return None;
    }
    let signature = project
        .signatures
        .get(&function)
        .filter(|held| settled(held))?;
    let first = signature.params.first().filter(|param| param.cells == 1)?;
    let Kind::Object(name) = &first.kind else {
        return None;
    };
    TypeKey::of_name(name).filter(|key| key.builtin().is_none())
}

/// The classes the code of `function` proves its receiver made from: the class of each slot it calls on it, and the type of each function of a table it hands it to.
// ⚠ An interface's slot says no class: any type may implement it.
pub(super) fn needs(
    project: &Project,
    carried: &Carried,
    members: &BTreeMap<FunctionKey, Member>,
    hands: Option<&[FunctionKey]>,
    function: FunctionKey,
) -> Vec<TypeKey> {
    let mut found: BTreeSet<TypeKey> = BTreeSet::new();
    found.extend(carried.needs.get(&function).into_iter().flatten());
    for to in hands.into_iter().flatten() {
        let member = members.get(to);
        let tabled = member.filter(|m| m.slot.is_some() && !m.is_static && m.role == Role::Method);
        found.extend(tabled.map(|m| m.owner));
    }
    let class = |ty: &TypeKey| {
        let facts = project.types.get(ty);
        facts.is_some_and(|facts| facts.base.is_some() && !facts.is_value())
    };
    found.into_iter().filter(class).collect()
}

#[cfg(test)]
#[path = "classes_typed_tests.rs"]
mod tests;
