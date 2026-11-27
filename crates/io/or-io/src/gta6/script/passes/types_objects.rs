//! What the objects a function handles are: the receiver it runs on, what a virtual call is made
//! on and yields, and what an array of references holds.

use std::collections::{HashMap, HashSet};

use super::kinds::{self, Lat};
use super::{Around, Scan, Typing};
use crate::gta6::script::cfg;
use crate::gta6::script::ir::{Callee, Expr, FunctionKey, Kind, Scalar, TypeKey, Var};
use crate::gta6::script::project::Project;

/// The intrinsic that reads an element of eight bytes whole: a reference, where the array holds references.
const ELEMENT: &str = "array_get_u64";

/// A reference to an object of the script type `key`, where the project holds the type and it is no value type.
// ⚠ A builtin type's line is not the project's to state, and a value type's method is handed a
// pointer to the value, never an object.
fn object_of(project: &Project, key: TypeKey) -> Option<Kind> {
    let facts = project
        .types
        .get(&key)
        .filter(|_| key.builtin().is_none())?;
    (!facts.is_value()).then(|| Kind::Object(key.to_string()))
}

/// A reference to an object of the script type named `name`, as [`object_of`] states it.
fn object_named(project: &Project, name: &str) -> Option<Kind> {
    object_of(project, TypeKey::of_name(name)?)
}

/// Whether `e` takes the address of `var` anywhere in it.
fn addressed(e: &Expr, var: Var) -> bool {
    let mut found = false;
    e.walk(&mut |node| {
        found |= matches!(node, Expr::AddrOf { var: of, .. } if *of == var);
    });
    found
}

/// What the receiver of a function the project proves an instance member of an object type is.
// ⚠ Only while the function leaves its receiver as it was passed: one it assigns, or whose
// address it takes, may hold something else by the time it is read.
pub(super) fn receiver(f: &cfg::Function, project: &Project) -> Option<Kind> {
    let this = Var::Param(0);
    let own = FunctionKey::new(f.head.program, f.head.at);
    let member = project.members.get(&own)?;
    if member.is_static || f.head.params == 0 {
        return None;
    }
    let kind = object_of(project, member.owner)?;
    let mut touched = false;
    for block in &f.cfg.blocks {
        for stmt in &block.stmts {
            touched |= stmt.targets().contains(&this);
            stmt.each_expr(&mut |top| touched |= addressed(top, this));
        }
        if let Some(e) = block.term.expr() {
            touched |= addressed(e, this);
        }
    }
    (!touched).then_some(kind)
}

/// Whether a call reads one element of an array whole.
fn reads_element(callee: &Callee) -> bool {
    matches!(callee, Callee::Intrinsic { name, .. } if name == ELEMENT)
}

/// What an element of an array of this kind is, read whole.
// ⚠ An array of no one kind, once it was one, leaves its elements of none: they are not what an
// earlier round took the array to hold.
fn held(array: &Lat) -> Lat {
    match array {
        Lat::Is(Kind::Array(of)) if kinds::reference(of) => Lat::Is((**of).clone()),
        Lat::Refs | Lat::Conflict => Lat::Conflict,
        _ => Lat::None,
    }
}

impl Around<'_> {
    /// The variable a virtual call on an object of a script type is made on, and a reference to that type, where `kinds` is on.
    // ⚠ The slot is called through the receiver's own object, so what the variable holds there is
    // an object of the type or of one derived from it.
    pub(super) fn receiver_of(
        &self,
        callee: &Callee,
        args: &[Expr],
        wide: &HashSet<Var>,
    ) -> Option<(Var, Kind)> {
        let Callee::Virtual { ty, .. } = callee else {
            return None;
        };
        let Some(Expr::Var(var)) = args.first() else {
            return None;
        };
        if !self.kinds || wide.contains(var) || matches!(var, Var::Global { .. }) {
            return None;
        }
        Some((*var, object_named(self.project, ty)?))
    }

    /// What a virtual call yields in one cell, where `kinds` is on and every function the slot's line may reach proves one kind: what it is, and the kind its cell is widened as.
    // ⚠ The widening is taken only where `interproc` is on, as a signature's is.
    pub(super) fn slot_yields(
        &self,
        callee: &Callee,
        returns: Option<u32>,
    ) -> Option<(Lat, Option<Kind>)> {
        let Callee::Virtual { ty, slot, .. } = callee else {
            return None;
        };
        if !self.kinds || returns != Some(1) {
            return None;
        }
        let kind = self.project.slot_result(TypeKey::of_name(ty)?, *slot)?;
        let lat = kinds::stated(kind);
        if lat == Lat::None {
            return None;
        }
        let widened = self.interproc && kinds::narrow(kind) && !kind.is_float();
        Some((lat, widened.then(|| kind.clone())))
    }
}

impl Scan<'_> {
    /// Notes that a virtual call of a slot of the type `kind` refers to is made on `var`.
    pub(super) fn receive(&mut self, var: Var, kind: Kind) {
        let held = self.facts.received.entry(var).or_default();
        *held = match &*held {
            Lat::None => Lat::Is(kind),
            Lat::Is(was) if *was == kind => Lat::Is(kind),
            _ => Lat::Conflict,
        };
    }
}

impl Typing<'_> {
    /// Gives each variable virtual calls are made on, where nothing else says more of it than "a reference", the type the calls are made on; `true` when any took one.
    // ⚠ Only a variable that holds one value all through: a parameter the function never assigns,
    // or a local assigned once that no address reaches. A slot reused for another value would
    // carry the type to code where it holds something else.
    pub(super) fn receivers(&mut self, cfg: &cfg::Cfg) -> bool {
        if self.facts.received.is_empty() {
            return false;
        }
        let mut sets: HashMap<Var, u32> = HashMap::new();
        for stmt in cfg.blocks.iter().flat_map(|block| &block.stmts) {
            for target in stmt.targets() {
                let count = sets.entry(*target).or_insert(0);
                *count = count.saturating_add(1);
            }
        }
        let mut changed = false;
        for (var, lat) in std::mem::take(&mut self.facts.received) {
            if !matches!(lat, Lat::Is(_)) {
                continue;
            }
            let once = match var {
                Var::Param(_) => !sets.contains_key(&var) && !self.facts.addressed.contains(&var),
                Var::Local(_) | Var::Value { .. } | Var::Stack { .. } => {
                    sets.get(&var) == Some(&1) && self.sealed(var)
                }
                Var::Global { .. } => false,
            };
            let open = matches!(self.lat(var), Lat::None | Lat::Is(Kind::Pointer));
            if once && open && self.typed(var) {
                self.known.insert(var, lat);
                changed = true;
            }
        }
        changed
    }

    /// What an element read whole from an array a variable holds is, where `kinds` is on; `None` where the call reads no such element.
    pub(super) fn element(&self, callee: &Callee, args: &[Expr]) -> Option<Lat> {
        let [Expr::Var(array), _] = args else {
            return None;
        };
        if !self.around.kinds || !reads_element(callee) {
            return None;
        }
        Some(held(&self.lat(*array)))
    }

    /// What an element read whole from an array a field holds is, where the project's layout gives the field an array of references.
    pub(super) fn field_element(&self, value: &Expr) -> Option<Lat> {
        let Expr::Call { callee, args, .. } = value else {
            return None;
        };
        let [array, _] = args.as_slice() else {
            return None;
        };
        let Expr::Load {
            ty: Scalar::U64,
            addr,
            offset,
        } = array
        else {
            return None;
        };
        if !self.around.kinds || !reads_element(callee) {
            return None;
        }
        match self.field(addr, *offset, 8, 0) {
            Lat::Is(Kind::Array(of)) if kinds::reference(&of) => Some(Lat::Is(*of)),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "types_objects_tests.rs"]
mod tests;
