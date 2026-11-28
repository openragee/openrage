//! Fields: a load or a store through a reference of known type is an access of that type's field.
//!
//! Works on the structured form. A load becomes `ExprSugar::Field` and a store `Group::Field`,
//! each naming the type that declares the field: where the project's layout of that type has a
//! field at the offset as wide as the access, or has none that the access overlaps, inside the
//! bytes the type holds. A reference is of known type where it is a base of one, a variable the
//! kinds pass proves holds one, a parameter the project's signature types and the function
//! leaves as it was passed, or a field the layouts give a type, loaded through such a reference.
//! A member's receiver is one too, and of a value type it points at the value, whose first field
//! lies at its first byte. So does the address of an element of an array of values of a script
//! value type, in a variable the kinds pass types so: an access of a part of the element, and
//! not all of it, is a field of that type.

use std::collections::BTreeMap;

use crate::gta6::script::ir::{
    self, Expr, ExprSugar, FunctionKey, Kind, Scalar, StoreKind, TypeKey, Var,
};
use crate::gta6::script::project::{layouts, FieldFacts, Project};
use crate::gta6::script::tree::{self, Group, Sugar};
use crate::gta6::script::{Context, Pass, Stats};

/// The names this file's rewrites are switched by.
pub const PASSES: &[Pass] = &[Pass::Fields];

/// The base types a field is looked for on before the access is left as it is.
const BASES: usize = 64;
/// The fields an address may be loaded through, one after another, below a base of known type.
const CHAIN: usize = 8;
/// The bytes of an object's header: its fields start after them.
const HEADER: u32 = 8;

/// Calls `f` on each simple statement `stmt` holds itself: its own, a loop head's, a source construct's.
fn own_simples<'a>(stmt: &'a tree::Stmt, f: &mut impl FnMut(&'a ir::Stmt)) {
    match stmt {
        tree::Stmt::Simple(s) => f(s),
        tree::Stmt::For { init, step, .. } => {
            for s in init.iter().chain(step) {
                f(s);
            }
        }
        tree::Stmt::Sugar(sugar) => {
            for s in sugar.parts().simples {
                f(s);
            }
        }
        _ => {}
    }
}

/// The variables an access through which may be read as a field.
struct Bases {
    /// Each variable that holds an object of one known type wherever the function reads it.
    objects: BTreeMap<Var, TypeKey>,
    /// Each variable that holds an array of values of one script value type wherever the function reads it.
    elements: BTreeMap<Var, TypeKey>,
}

/// The variables that hold an object of one known type, or an array of values of one, wherever the function reads them, with that type.
// A variable only ever assigned objects the function makes of one type, the receiver of a
// function the project proves a member of a type, and a variable the kinds pass types.
// ⚠ A variable an address the function takes may reach may be written through that address: an
// address the function only loads or stores its own cell through reaches no other, and one
// handed whole to a function reaches what the project's reach table says that function touches.
fn objects(f: &mut tree::Function, project: &Project) -> Bases {
    let tree::Function { head, body } = f;
    let head = &*head;
    let mut made: BTreeMap<Var, Option<TypeKey>> = BTreeMap::new();
    let mut reached = layouts::Reached::default();
    // An explicit stack: a body nests as deep as its listing does.
    let mut work: Vec<&mut Vec<tree::Stmt>> = vec![body];
    while let Some(list) = work.pop() {
        for stmt in list {
            own_simples(stmt, &mut |s| {
                let ty = match s {
                    ir::Stmt::Assign {
                        value: Expr::NewObject(ty),
                        ..
                    } => TypeKey::of(ty),
                    _ => None,
                };
                for target in s.targets() {
                    let held = made.entry(*target).or_insert(ty);
                    if *held != ty {
                        *held = None;
                    }
                }
                // A store through the variable's own address writes it as an assignment does.
                if let Some(var) = layouts::own_cell_store(s) {
                    made.insert(var, None);
                }
                reached.own_store(s);
            });
            tree::own_exprs_mut(stmt, &mut |top| reached.expr(top, head, Some(project)));
            work.extend(tree::nested_mut(stmt));
        }
    }
    let frame = reached.finish();

    let mut bases = BTreeMap::new();
    for (var, ty) in &made {
        let reached = match var {
            Var::Local(_) | Var::Value { .. } | Var::Stack { .. } => frame.reaches(*var),
            // A caller sets a parameter, and any code a global.
            Var::Param(_) | Var::Global { .. } => true,
        };
        if let (false, Some(ty)) = (reached, ty) {
            bases.insert(*var, *ty);
        }
    }

    // The receiver is its type's object only while the function leaves it as it was passed. A
    // value type's method takes a pointer to the value, whose fields lie from its first byte.
    let this = Var::Param(0);
    let member = project
        .members
        .get(&FunctionKey::new(head.program, head.at));
    let receiver = member
        .filter(|member| !member.is_static && head.params > 0)
        .map(|member| member.owner)
        .filter(|owner| project.types.contains_key(owner))
        .filter(|_| !made.contains_key(&this) && !frame.reaches(this));
    if let Some(owner) = receiver {
        bases.insert(this, owner);
    }

    // A variable the kinds pass declares a reference to an object of a script type is one, as
    // far as no address of the frame reaches it.
    for (var, info) in &head.vars {
        let Some(ty) = layouts::object_of(project, &info.kind) else {
            continue;
        };
        if !frame.reaches(*var) {
            bases.entry(*var).or_insert(ty);
        }
    }

    // ⚠ Only the kinds pass says what an array holds: no assignment the function makes does.
    let mut elements = BTreeMap::new();
    for (var, info) in &head.vars {
        let Some(ty) = layouts::element_of(project, &info.kind) else {
            continue;
        };
        if !frame.reaches(*var) {
            elements.insert(*var, ty);
        }
    }
    Bases {
        objects: bases,
        elements,
    }
}

/// The type that declares the field of `bytes` bytes at `offset` of an object of type `ty`, and the field.
// ⚠ `None` where the nearest type that has rows at the offset has one of another width: the
// evidence disagrees, and the access keeps the form it has.
fn field_of(
    project: &Project,
    ty: TypeKey,
    offset: u32,
    bytes: u32,
) -> Option<(TypeKey, &FieldFacts)> {
    let mut ty = ty;
    for _ in 0..BASES {
        let rows = project.fields.get(&ty).into_iter().flatten();
        let mut here = rows.filter(|field| field.offset == offset);
        if let Some(first) = here.next() {
            let agree = first.bytes == bytes && here.all(|field| field.bytes == bytes);
            return agree.then_some((ty, first));
        }
        // ⚠ The builtin a value type derives from holds none of its bytes.
        let facts = project.types.get(&ty)?;
        if facts.is_value() {
            return None;
        }
        ty = facts.base?;
    }
    None
}

/// The script type the reference at `offset` of an object of type `ty` refers to, where the layouts give that field one.
// ⚠ The type is the one the source gives the field or a type above it. The object may be of a
// type below it, whose fields past the type's own are not looked for.
fn referred(project: &Project, ty: TypeKey, offset: u32) -> Option<TypeKey> {
    let (_, field) = field_of(project, ty, offset, 8)?;
    match &field.kind {
        Kind::Object(name) => TypeKey::of_name(name).filter(|key| key.builtin().is_none()),
        _ => None,
    }
}

/// The type of the object `addr` points at: a base of known type, or what a field loaded through such a reference refers to.
fn pointee(project: &Project, bases: &BTreeMap<Var, TypeKey>, addr: &Expr) -> Option<TypeKey> {
    // The offsets `addr` is loaded through, the last load first.
    let mut through: Vec<u32> = Vec::new();
    let mut at = addr;
    let var = loop {
        match at {
            Expr::Var(var) => break var,
            Expr::Sugar {
                kind: ExprSugar::Field { .. },
                e,
            } => at = &**e,
            Expr::Load {
                ty: Scalar::U64,
                addr: under,
                offset,
            } if through.len() < CHAIN => {
                through.push(*offset);
                at = &**under;
            }
            _ => return None,
        }
    };
    let mut ty = *bases.get(var)?;
    for offset in through.iter().rev() {
        ty = referred(project, ty, *offset)?;
    }
    Some(ty)
}

/// The type an access of `bytes` bytes at `offset` of an object of type `ty` lies in, where no row of `ty` or of its bases says anything at those bytes.
// ⚠ Past the bytes `ty` holds the object may be of a type below it, whose field no row of `ty`
// states: the access keeps the form it has, as it does at the object's header. A value held in
// place has no header.
fn unrowed(project: &Project, ty: TypeKey, offset: u32, bytes: u32) -> Option<TypeKey> {
    let end = u64::from(offset) + u64::from(bytes);
    let value = (project.types.get(&ty)).is_some_and(|facts| facts.is_value());
    let header = if value { 0 } else { HEADER };
    if offset < header || bytes == 0 {
        return None;
    }
    let mut at = ty;
    for _ in 0..BASES {
        let rows = project.fields.get(&at).into_iter().flatten();
        let overlaps = |row: &FieldFacts| {
            u64::from(row.offset) < end
                && u64::from(offset) < u64::from(row.offset) + u64::from(row.bytes)
        };
        if rows.into_iter().any(overlaps) {
            return None;
        }
        let facts = project.types.get(&at)?;
        match facts.base.filter(|_| !facts.is_value()) {
            Some(base) => at = base,
            None => break,
        }
    }
    let row = FieldFacts {
        offset,
        bytes,
        kind: Kind::Unknown,
    };
    layouts::declarer(&project.types, ty, &row)
}

/// The type that declares the field an access of `bytes` bytes at `offset` through `addr` is of, where `addr` points at an object of known type or at an element of an array of values of one.
fn declared(
    project: &Project,
    bases: &Bases,
    addr: &Expr,
    offset: u32,
    bytes: u32,
) -> Option<TypeKey> {
    let element = layouts::element_array(addr).and_then(|array| bases.elements.get(&array));
    let ty = match element {
        // ⚠ All of an element is the element, which is no field of it: an enum's value is one.
        Some(ty) if layouts::within_element(project, *ty, offset, bytes) => *ty,
        Some(_) => return None,
        None => pointee(project, &bases.objects, addr)?,
    };
    match field_of(project, ty, offset, bytes) {
        Some((owner, _)) => Some(owner),
        None => unrowed(project, ty, offset, bytes),
    }
}

/// The type that declares the field a store writes, where it writes one whole.
fn stored(project: &Project, bases: &Bases, s: &ir::Stmt) -> Option<TypeKey> {
    match s {
        ir::Stmt::Store {
            kind, addr, offset, ..
        } => declared(project, bases, addr, *offset, kind.bytes()),
        ir::Stmt::StoreBytes {
            addr,
            offset,
            bytes,
            ..
        } => declared(project, bases, addr, *offset, *bytes),
        _ => None,
    }
}

/// Gives an integer constant that a store writes to a field the layouts prove a float's the kind of a float; its bits stay.
// ⚠ By the field's kind alone, which every access the layouts saw of it agrees on, never by the
// look of the bits.
fn float_constant(project: &Project, owner: TypeKey, s: &mut ir::Stmt) {
    let ir::Stmt::Store {
        kind: StoreKind::I32,
        offset,
        value: Expr::Const(c),
        ..
    } = s
    else {
        return;
    };
    let field = project.field(owner, *offset);
    let float = field.is_some_and(|field| field.bytes == 4 && field.kind == Kind::F32);
    // A 32-bit value, widened with its sign or without.
    let half = c.bits >> 32;
    if float && c.kind.is_integer() && (half == 0 || half == 0xFFFF_FFFF) {
        c.kind = Kind::F32;
    }
}

/// Gives the zero that a store writes whole to a field the layouts prove a reference's the kind of a reference, which is `null`; its bits stay.
// ⚠ By the field's kind alone: every access the layouts saw of it holds a reference, and eight
// zero bytes are a number only to a field that ever holds one.
fn null_constant(project: &Project, owner: TypeKey, s: &mut ir::Stmt) {
    let ir::Stmt::Store {
        kind: StoreKind::U64 | StoreKind::Ref,
        offset,
        value: Expr::Const(c),
        ..
    } = s
    else {
        return;
    };
    let field = project.field(owner, *offset);
    let reference = field.is_some_and(|field| {
        let held = matches!(
            field.kind,
            Kind::Object(_) | Kind::String | Kind::Array(_) | Kind::Pointer
        );
        field.bytes == 8 && held
    });
    if reference && c.bits == 0 {
        c.kind = Kind::Pointer;
    }
}

/// Reads each load in `top` of a field the layouts have as the field; returns how many.
fn loads(top: &mut Expr, project: &Project, bases: &Bases) -> usize {
    let mut read = 0;
    top.walk_mut(&mut |e| {
        let owner = match &*e {
            Expr::Load { ty, addr, offset } => declared(project, bases, addr, *offset, ty.bytes()),
            Expr::LoadBytes {
                addr,
                offset,
                bytes,
            } => declared(project, bases, addr, *offset, *bytes),
            _ => None,
        };
        if let Some(owner) = owner {
            let load = std::mem::replace(e, Expr::GuidEmpty);
            *e = Expr::sugar(ExprSugar::Field { owner }, load);
            read += 1;
        }
    });
    read
}

/// What one run did, each a count of accesses read as a field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Found {
    pub loads: usize,
    pub stores: usize,
}

/// Reads each access of a field the project's layouts prove as that field.
// `Stats` has no count of readings: what was read is traced.
pub fn run(f: &mut tree::Function, cx: &Context<'_>, _stats: &mut Stats) {
    if !cx.options.on(Pass::Fields) || f.head.incomplete.is_some() {
        return;
    }
    let done = apply(f, cx.project);
    tracing::trace!(
        function = %f.head.name,
        loads = done.loads,
        stores = done.stores,
        "fields"
    );
}

/// [`run`] whatever the options say, with the count of what it did.
pub(crate) fn apply(f: &mut tree::Function, project: &Project) -> Found {
    let mut done = Found::default();
    if project.fields.is_empty() {
        return done;
    }
    let bases = objects(f, project);
    if bases.objects.is_empty() && bases.elements.is_empty() {
        return done;
    }
    let mut work: Vec<&mut Vec<tree::Stmt>> = vec![&mut f.body];
    while let Some(list) = work.pop() {
        for stmt in list {
            tree::own_exprs_mut(stmt, &mut |top| done.loads += loads(top, project, &bases));
            let owner = match &*stmt {
                tree::Stmt::Simple(s) => stored(project, &bases, s),
                _ => None,
            };
            if let Some(owner) = owner {
                let kind = Group::Field { owner };
                let taken = std::mem::replace(stmt, tree::Stmt::Break);
                if let tree::Stmt::Simple(mut store) = taken {
                    float_constant(project, owner, &mut store);
                    null_constant(project, owner, &mut store);
                    let parts = vec![store];
                    *stmt = tree::Stmt::Sugar(Box::new(Sugar::Group { kind, parts }));
                    done.stores += 1;
                }
            }
            work.extend(tree::nested_mut(stmt));
        }
    }
    done
}

#[cfg(test)]
#[path = "fields_tests.rs"]
mod tests;
