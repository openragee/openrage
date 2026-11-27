//! What the objects a function handles are: the receiver it runs on, what a virtual call or a
//! member is called on, what a virtual call and a cast yield, and what an array of references
//! holds.

use std::collections::{HashMap, HashSet};

use super::kinds::{self, Lat};
use super::{Around, Scan, Typing};
use crate::gta6::script::cfg;
use crate::gta6::script::ir::{Callee, Expr, FunctionKey, Kind, Scalar, Stmt, TypeKey, Var};
use crate::gta6::script::project::{signatures, Project};

/// The intrinsic that reads an element of eight bytes whole: a reference, where the array holds references.
const ELEMENT: &str = "array_get_u64";

/// The stems of the two casts to a type: one that yields null where the object is no such type, and one that must hold.
const CASTS: [&str; 2] = ["try_cast<", "cast<"];

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

/// The one of two object types a single value of both is: the one that is the other or derives from it; `None` where neither does.
fn meet(project: &Project, a: &Kind, b: &Kind) -> Option<Kind> {
    if a == b {
        return Some(a.clone());
    }
    let key = |kind: &Kind| match kind {
        Kind::Object(name) => TypeKey::of_name(name),
        _ => None,
    };
    let (x, y) = (key(a)?, key(b)?);
    if project.is_a(x, y) {
        Some(a.clone())
    } else if project.is_a(y, x) {
        Some(b.clone())
    } else {
        None
    }
}

/// How often each variable is a target of a statement.
fn assignments(cfg: &cfg::Cfg) -> HashMap<Var, u32> {
    let mut sets: HashMap<Var, u32> = HashMap::new();
    for stmt in cfg.blocks.iter().flat_map(|block| &block.stmts) {
        for target in stmt.targets() {
            let count = sets.entry(*target).or_insert(0);
            *count = count.saturating_add(1);
        }
    }
    sets
}

/// The array an assignment of `var` reads one element of whole, where that is all the statement is.
fn element_of(stmt: &Stmt, var: Var) -> Option<Var> {
    let Stmt::Assign {
        target,
        value: Expr::Call { callee, args, .. },
    } = stmt
    else {
        return None;
    };
    match args.as_slice() {
        [Expr::Var(array), _] if *target == var && reads_element(callee) => Some(*array),
        _ => None,
    }
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
    /// The variable a call is made on as an object of a script type, and a reference to that type, where `kinds` is on: a virtual call's receiver, or the receiver a call of an instance member of the type is handed first.
    // ⚠ The slot is called through the receiver's own object, so what the variable holds there is
    // an object of the type or of one derived from it. A member is called on its owner's objects
    // alone: what the project proves a member is handed `this`, never a value its own code shows.
    pub(super) fn receiver_of(
        &self,
        callee: &Callee,
        args: &[Expr],
        wide: &HashSet<Var>,
    ) -> Option<(Var, Kind)> {
        let Some(Expr::Var(var)) = args.first() else {
            return None;
        };
        if !self.kinds || wide.contains(var) || matches!(var, Var::Global { .. }) {
            return None;
        }
        let kind = match callee {
            Callee::Virtual { ty, .. } => object_named(self.project, ty)?,
            other => {
                let key = signatures::key_of(self.program, other)?;
                let member = self.project.members.get(&key)?;
                if member.is_static {
                    return None;
                }
                object_of(self.project, member.owner)?
            }
        };
        Some((*var, kind))
    }

    /// What a cast to a script object type yields, where `kinds` is on: a reference to that type.
    // ⚠ Either cast yields the object it is given or null, so an object of the type or of one
    // derived from it; a cast of any other mode keeps its mode in its name and is not read.
    pub(super) fn cast_yields(&self, callee: &Callee) -> Option<Lat> {
        let Callee::Intrinsic { name, .. } = callee else {
            return None;
        };
        if !self.kinds {
            return None;
        }
        let ty = CASTS.iter().find_map(|stem| name.strip_prefix(stem))?;
        let ty = ty.strip_suffix('>')?;
        object_named(self.project, ty).map(Lat::Is)
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
    /// Notes that a call is made on `var` as an object of the type `kind` refers to.
    // Calls made on it as two types, one of which derives from the other, are made on an object of
    // the derived one: the variable holds one value, which is of both.
    pub(super) fn receive(&mut self, var: Var, kind: Kind) {
        let project = self.around.project;
        let held = self.facts.received.entry(var).or_default();
        *held = match &*held {
            Lat::None => Lat::Is(kind),
            Lat::Is(was) => meet(project, was, &kind).map_or(Lat::Conflict, Lat::Is),
            _ => Lat::Conflict,
        };
    }
}

impl Typing<'_> {
    /// Gives each variable calls are made on as an object type, where nothing else says more of it than "a reference", the type the calls are made on, and [`Typing::arrays`] the arrays such variables are read from; `true` when any took one.
    // ⚠ Only a variable that holds one value all through: a parameter the function never assigns,
    // or a local assigned once that no address reaches. A slot reused for another value would
    // carry the type to code where it holds something else.
    pub(super) fn receivers(&mut self, cfg: &cfg::Cfg) -> bool {
        if self.facts.received.is_empty() {
            return false;
        }
        let sets = assignments(cfg);
        let mut given: Vec<(Var, Kind)> = Vec::new();
        for (var, lat) in std::mem::take(&mut self.facts.received) {
            let Lat::Is(kind) = lat else {
                continue;
            };
            if self.held_once(var, &sets) && self.open(var) && self.typed(var) {
                given.push((var, kind));
            }
        }
        let arrays = self.arrays(cfg, &sets, &given);
        let changed = !given.is_empty() || !arrays.is_empty();
        for (var, kind) in given.into_iter().chain(arrays) {
            self.known.insert(var, Lat::Is(kind));
        }
        changed
    }

    /// Whether a variable holds one value all through: a parameter the function never assigns, or a local assigned once that no address reaches.
    fn held_once(&self, var: Var, sets: &HashMap<Var, u32>) -> bool {
        match var {
            Var::Param(_) => !sets.contains_key(&var) && !self.facts.addressed.contains(&var),
            Var::Local(_) | Var::Value { .. } | Var::Stack { .. } => {
                sets.get(&var) == Some(&1) && self.sealed(var)
            }
            Var::Global { .. } => false,
        }
    }

    /// Whether nothing says more of a variable than that it is a reference.
    fn open(&self, var: Var) -> bool {
        matches!(self.lat(var), Lat::None | Lat::Is(Kind::Pointer))
    }

    /// The arrays an element of which is read whole into a variable calls are made on as an object type: each an array of that type.
    // ⚠ The element is read with no cast and then called on as the type, so the array's own
    // element type is that type or one derived from it: `T[]` is what the array is, or holds it,
    // as C# reads an array of a derived type. Only an array that holds one value all through, and
    // an element assigned once and read straight from it; where its elements are called on as two
    // types, the derived one, and none where neither derives from the other.
    fn arrays(
        &self,
        cfg: &cfg::Cfg,
        sets: &HashMap<Var, u32>,
        given: &[(Var, Kind)],
    ) -> Vec<(Var, Kind)> {
        let mut found: HashMap<Var, Option<Kind>> = HashMap::new();
        for (item, kind) in given {
            if !matches!(item, Var::Local(_) | Var::Value { .. } | Var::Stack { .. }) {
                continue;
            }
            let mut read = cfg.blocks.iter().flat_map(|block| &block.stmts);
            let Some(array) = read.find_map(|stmt| element_of(stmt, *item)) else {
                continue;
            };
            let slot = found.entry(array).or_insert_with(|| Some(kind.clone()));
            *slot = slot
                .take()
                .and_then(|was| meet(self.around.project, &was, kind));
        }
        let mut arrays: Vec<(Var, Kind)> = Vec::new();
        for (array, kind) in found {
            let Some(kind) = kind else {
                continue;
            };
            // A variable calls are made on is no array.
            if given.iter().any(|(var, _)| *var == array) {
                continue;
            }
            if self.held_once(array, sets) && self.open(array) && self.typed(array) {
                arrays.push((array, Kind::Array(Box::new(kind))));
            }
        }
        arrays
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
