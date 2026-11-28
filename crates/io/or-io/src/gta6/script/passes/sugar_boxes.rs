//! A value boxed where the source converts it to a reference without a word: `box<T>(x)` handed
//! on whole, to a call, a `return` or a store, is `(T)x` there, the value C# boxes as its type.
//!
//! ⚠ A box is a reference, so a place that takes one whole is declared a reference in the source
//! and the box is the conversion to it. A box compared, taken apart or followed to its value is
//! none of that and stays as it is, and so does one whose value `(T)` would convert otherwise
//! than the box copies it.

use crate::gta6::script::ir::{
    self, BinOp, Callee, Expr, ExprSugar, FunctionKey, Head, Kind, Returns, Scalar, TypeKey, UnOp,
    Var,
};
use crate::gta6::script::print;
use crate::gta6::script::project::{signatures, Param, TypeFacts};
use crate::gta6::script::tree::{self, Stmt};
use crate::gta6::script::{Names, Project};

/// The method whose two boxes the printer reads as `a == b`.
const OBJECT_EQUALS: &str = "System.Object.Equals";

/// The method whose one box the printer reads as `(int)x` where it can.
const CONVERT_TO_INT32: &str = "System.Convert.ToInt32";

/// The runtime's integers, by the names the programs give them: `(T)x` keeps the bytes of `x` a box of `T` copies.
const INTEGERS: [&str; 9] = [
    "System.SByte",
    "System.Byte",
    "System.Int16",
    "System.UInt16",
    "System.Char",
    "System.Int32",
    "System.UInt32",
    "System.Int64",
    "System.UInt64",
];

/// The runtime's truth, by the name the programs give it.
const BOOLEAN: &str = "System.Boolean";

/// Reads each box handed on whole as the conversion C# writes in its place.
pub(super) fn run(body: &mut [Stmt], head: &Head, project: &Project, names: &Names) {
    let returned = returns_reference(head, project);
    // An explicit stack: a body nests as deep as its listing does.
    let mut work: Vec<&mut [Stmt]> = vec![body];
    while let Some(list) = work.pop() {
        for stmt in list {
            if let (Stmt::Return(Some(value)), true) = (&mut *stmt, returned) {
                convert(value, head, project, names);
            }
            each_simple_mut(stmt, &mut |s| match s {
                ir::Stmt::Store { kind, value, .. } if kind.bytes() == 8 => {
                    convert(value, head, project, names);
                }
                _ => {}
            });
            tree::own_exprs_mut(stmt, &mut |top| {
                top.walk_mut(&mut |e| handed(e, head, project, names));
            });
            work.extend(tree::nested_mut(stmt).into_iter().map(Vec::as_mut_slice));
        }
    }
}

/// Calls `f` on each simple statement `stmt` holds itself, a construct's own parts included and nested lists not.
fn each_simple_mut(stmt: &mut Stmt, f: &mut impl FnMut(&mut ir::Stmt)) {
    match stmt {
        Stmt::Simple(s) => f(s),
        Stmt::For { init, step, .. } => {
            for s in [init, step].into_iter().flatten() {
                f(s);
            }
        }
        Stmt::Sugar(sugar) => {
            for s in sugar.parts_mut().simples {
                f(s);
            }
        }
        _ => {}
    }
}

/// The type `e` boxes its one value as, and that value, where `e` is a `box<T>` of one value.
fn boxed(e: &Expr) -> Option<(&str, &Expr)> {
    let Expr::Call {
        callee: Callee::Intrinsic { name, ty: None, .. },
        args,
        ..
    } = e
    else {
        return None;
    };
    let ty = name.strip_prefix("box<")?.strip_suffix('>')?;
    match args.as_slice() {
        [value] if !ty.is_empty() => Some((ty, value)),
        _ => None,
    }
}

/// Reads `e` as `(T)x` where it is a box of one value that `(T)` converts as the box copies it.
fn convert(e: &mut Expr, head: &Head, project: &Project, names: &Names) {
    let reads = boxed(e).is_some_and(|(ty, value)| copies(ty, value, head, project, names));
    if reads {
        let held = std::mem::replace(e, Expr::GuidEmpty);
        *e = Expr::sugar(ExprSugar::Convert, held);
    }
}

/// The facts of the value type of a script the type named `ty` is: an enum or a struct, which no runtime method names.
fn script_value<'a>(ty: &str, project: &'a Project) -> Option<&'a TypeFacts> {
    let key = TypeKey::of_name(ty).filter(|key| key.builtin().is_none())?;
    project.types.get(&key).filter(|facts| facts.is_value())
}

/// The name of the runtime's type a box names: a builtin by the name the user's list gives its index, any other as the listing spells it.
// ⚠ A program names a builtin by its index alone, `type_` and eight digits: only a list says
// which of the runtime's types that is, and where none does the box stays.
fn runtime_name<'a>(ty: &'a str, names: &'a Names) -> Option<&'a str> {
    match TypeKey::of_name(ty) {
        Some(key) if key.builtin().is_some() => names.types.get(&key.0).map(String::as_str),
        Some(_) => None,
        None => Some(ty),
    }
}

/// Whether a value type is an enum, as the printer decides it.
fn enumeration(facts: &TypeFacts) -> bool {
    print::enum_flags(facts)
}

/// Whether `(T)value`, in C#, keeps what a box of `T` copies of `value`: an integer or an enum of the bits of an integer, a truth of a truth, a struct of a variable of that struct.
// ⚠ C# converts a number by its value: `(float)` of an integer, or `(E)` of a float, is another
// value than its bits. And a value written with a leading sign after `(E)` reads as `E - 1`.
fn copies(ty: &str, value: &Expr, head: &Head, project: &Project, names: &Names) -> bool {
    if signed_text(value) {
        return false;
    }
    let runtime = runtime_name(ty, names);
    if runtime.is_some_and(|name| INTEGERS.contains(&name)) {
        return integral(value, head, project);
    }
    if runtime == Some(BOOLEAN) {
        return truth(value, head, project);
    }
    match script_value(ty, project) {
        Some(facts) if enumeration(facts) => integral(value, head, project),
        Some(_) => of_type(ty, value, head),
        None => false,
    }
}

/// Whether `value` may be written with a sign before it.
fn signed_text(value: &Expr) -> bool {
    match value {
        Expr::Unary { op: UnOp::Neg, .. } => true,
        Expr::Const(c) => match c.kind {
            Kind::U8 | Kind::U16 | Kind::U32 | Kind::U64 | Kind::Hash | Kind::Bool => false,
            _ => c.bits >> 63 != 0 || (c.bits as u32) >> 31 != 0,
        },
        _ => false,
    }
}

/// `e` under the readings of a call as a method or a property, which change neither the call nor what it yields.
fn under_call(e: &Expr) -> &Expr {
    let mut e = e;
    while let Expr::Sugar {
        kind: ExprSugar::Method | ExprSugar::Property,
        e: inner,
    } = e
    {
        e = &**inner;
    }
    e
}

/// The kind of the one cell a call yields, where the project proves it: by the settled signature of the script function called, or of every function a slot's call may reach.
// ⚠ A native's result is never read here: the project knows only how its result is read.
fn result_kind<'p>(e: &Expr, head: &Head, project: &'p Project) -> Option<&'p Kind> {
    let Expr::Call {
        callee, returns, ..
    } = under_call(e)
    else {
        return None;
    };
    if let Callee::Virtual { ty, slot, .. } = callee {
        let one = returns.is_none_or(|cells| cells == 1);
        let ty = TypeKey::of_name(ty).filter(|_| one)?;
        return project.slot_result(ty, *slot);
    }
    let held = project
        .signatures
        .get(&signatures::key_of(head.program, callee)?)?;
    let proved = signatures::settled(held) && held.returns == 1 && held.result != Kind::Unknown;
    proved.then_some(&held.result)
}

/// Whether `value` is an integer by what it is written as: no float, no truth, no value of several cells.
// A global's kind is the project's and not the function's: one is no integer by what is written.
// A call is one where the project proves it yields an integer.
fn integral(value: &Expr, head: &Head, project: &Project) -> bool {
    let kind_integral =
        |kind: &Kind| kind.is_integer() || matches!(kind, Kind::Unknown | Kind::Hash);
    let mut value = value;
    while let Expr::Sugar {
        kind: ExprSugar::Field { .. },
        e,
    } = value
    {
        value = &**e;
    }
    match value {
        Expr::Const(c) => kind_integral(&c.kind),
        Expr::Var(Var::Global { .. }) => false,
        Expr::Var(var) => head
            .vars
            .get(var)
            .is_none_or(|info| info.width == ir::Width::Cell && kind_integral(&info.kind)),
        Expr::Cast { to, .. } => !to.kind().is_float(),
        Expr::Load { ty, .. } => !ty.kind().is_float(),
        Expr::Binary { op, at, .. } => !comparison(*op) && kind_integral(at),
        Expr::Call { .. }
        | Expr::Sugar {
            kind: ExprSugar::Method | ExprSugar::Property,
            ..
        } => result_kind(value, head, project)
            .is_some_and(|kind| kind.is_integer() || *kind == Kind::Hash),
        _ => false,
    }
}

/// Whether `op` yields a truth.
fn comparison(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::Eq
            | BinOp::Ne
            | BinOp::Lt
            | BinOp::Le
            | BinOp::Gt
            | BinOp::Ge
            | BinOp::LogicalAnd
            | BinOp::LogicalOr
    )
}

/// Whether `value` is a truth by what it is written as: a call where the project proves it yields one, and a field of one byte the project types a truth, read as its one byte.
// The field's reading is the one the printer writes bare, so `(bool)` stands before the field itself.
fn truth(value: &Expr, head: &Head, project: &Project) -> bool {
    match value {
        Expr::Const(c) => c.kind == Kind::Bool,
        Expr::Var(var) => head
            .vars
            .get(var)
            .is_some_and(|info| info.kind == Kind::Bool),
        Expr::Truth(_) | Expr::Unary { op: UnOp::Not, .. } => true,
        Expr::Binary { op, .. } => comparison(*op),
        Expr::Sugar {
            kind: ExprSugar::Field { owner },
            e,
        } => match &**e {
            Expr::Load {
                ty: Scalar::U8,
                offset,
                ..
            } => project
                .field(*owner, *offset)
                .is_some_and(|field| field.kind == Kind::Bool && field.bytes == 1),
            _ => false,
        },
        Expr::Call { .. }
        | Expr::Sugar {
            kind: ExprSugar::Method | ExprSugar::Property,
            ..
        } => result_kind(value, head, project) == Some(&Kind::Bool),
        _ => false,
    }
}

/// Whether `value` is a variable declared as the value type named `ty`.
fn of_type(ty: &str, value: &Expr, head: &Head) -> bool {
    let Expr::Var(var) = value else {
        return false;
    };
    let kind = head.vars.get(var).map(|info| &info.kind);
    matches!(kind, Some(Kind::Value { ty: held, .. }) if held == ty)
}

/// Whether the function hands back one cell that nothing proves of another kind than a reference: the type its source declares, which a box returned converts to.
fn returns_reference(head: &Head, project: &Project) -> bool {
    if head.returns != Returns::Cell {
        return false;
    }
    let key = FunctionKey::new(head.program, head.at);
    let result = project
        .signatures
        .get(&key)
        .map(|signature| &signature.result);
    matches!(
        result,
        None | Some(Kind::Unknown | Kind::Pointer | Kind::Object(_))
    )
}

/// The first argument of a call to the script function `key` that is no receiver: a method's first is, unless it is static.
fn first(project: &Project, key: FunctionKey) -> usize {
    let member = project.members.get(&key);
    usize::from(member.is_some_and(|member| !member.is_static))
}

/// Whether the argument at `index` of a call is handed to a parameter the source declares a reference, where a box of `ty` is C#'s own conversion and `(T)x` calls the same method.
// ⚠ A method of the runtime may take the value itself by another overload, which `(T)x` would
// call: there only a script's own type, which no runtime method names, is read, and never at
// the printer's own readings, an operator, or a constructor. A receiver is written before the
// dot, a native's parameters are of no stated type, and of the runtime's own operations only an
// array's store of a reference takes one whole.
fn takes_reference(
    callee: &Callee,
    args: &[Expr],
    index: usize,
    ty: &str,
    head: &Head,
    project: &Project,
) -> bool {
    // ⚠ Of a script function, only where the head the callee is written with declares that
    // parameter an object: a parameter of no such kind is written a number or a pointer, and
    // `(T)x` handed to one reads as the value itself.
    let declared = |params: Option<&[Param]>| {
        let cell = (args.get(..index).into_iter().flatten()).try_fold(0u32, |cell, arg| {
            cell.checked_add(print::cells_of(arg, head)?)
        });
        let params = params.unwrap_or_default();
        let mut at = 0u32;
        let param = params.iter().find(|param| {
            let here = Some(at) == cell && param.cells == 1;
            at = at.saturating_add(param.cells);
            here
        });
        param.is_some_and(|param| matches!(param.kind, Kind::Object(_)))
    };
    let signed = |key: FunctionKey| {
        let params = project.signatures.get(&key).map(|s| s.params.as_slice());
        index >= first(project, key) && declared(params)
    };
    match callee {
        Callee::Local { at, .. } => signed(FunctionKey::new(head.program, *at)),
        Callee::Import { program, at, .. } => signed(FunctionKey::new(*program, *at)),
        Callee::Virtual {
            ty: class, slot, ..
        } => {
            let params = TypeKey::of_name(class).and_then(|ty| project.slot_parameters(ty, *slot));
            index >= 1 && declared(params)
        }
        Callee::Builtin { path, .. } => {
            let Some(facts) = script_value(ty, project) else {
                return false;
            };
            let member = path.rsplit('.').next().unwrap_or_default();
            if member.starts_with("op_") || member == "Ctor" || path == OBJECT_EQUALS {
                return false;
            }
            // ⚠ Where the printer reads the call as `(int)x` the box stays for it to read.
            let printer_casts = print::enum_fits_int(facts);
            path != CONVERT_TO_INT32 || !(enumeration(facts) && printer_casts)
        }
        Callee::Intrinsic { name, .. } => name == "array_set_ref" && index == 2,
        Callee::Native { .. } => false,
    }
}

/// Reads each box an argument of the call `e` hands on whole as its conversion.
fn handed(e: &mut Expr, head: &Head, project: &Project, names: &Names) {
    let Expr::Call { callee, args, .. } = e else {
        return;
    };
    let callee: &Callee = callee;
    for index in 0..args.len() {
        let taken = boxed(&args[index])
            .is_some_and(|(ty, _)| takes_reference(callee, args, index, ty, head, project));
        if taken {
            convert(&mut args[index], head, project, names);
        }
    }
}

#[cfg(test)]
#[path = "sugar_boxes_tests.rs"]
mod tests;
