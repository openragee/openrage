//! Properties: the runtime's accessors `get_X` and `set_X`, used as `receiver.X` and
//! `receiver.X = value`.
//!
//! ⚠ A name is the whole proof: a function of any other name is a method, whatever it does.

use crate::gta6::script::ir::{self, Callee, Expr};
use crate::gta6::script::tree::Stmt;

use super::whole;

/// What the accessor of a property does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accessor {
    Get,
    Set,
}

/// The accessor a function's name says it is and the name of its property: `get_X` gets `X` and `set_X` sets it; the one rule a property is declared and used by.
pub fn accessor(name: &str) -> Option<(Accessor, &str)> {
    let (accessor, property) = match (name.strip_prefix("get_"), name.strip_prefix("set_")) {
        (Some(property), _) => (Accessor::Get, property),
        (_, Some(property)) => (Accessor::Set, property),
        _ => return None,
    };
    let mut chars = property.chars();
    let first = chars.next()?;
    let word = (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    word.then_some((accessor, property))
}

/// Where an expression stands in its statement, as far as a property's use asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stands {
    /// Its value is taken.
    Inside,
    /// It is the whole of a statement that only runs it.
    Alone,
    /// The statement does not say which.
    Unsaid,
}

/// Whether a call is the use of a runtime type's property on its receiver: a getter's whose value is taken, or a setter's that is a statement of its own.
// ⚠ A runtime type's accessor takes its receiver first wherever it takes one: no property of
// the runtime is static and takes a value.
// ⚠ A script's accessor is declared and called as the method it is: a property declared under
// its name would leave every use this does not read (a statement, a loop's head, a call on no
// proved receiver, a call from another program) invoking a property.
pub(super) fn accessed(
    callee: &Callee,
    args: &[Expr],
    returns: Option<u32>,
    stands: Stands,
) -> bool {
    let Callee::Builtin { path, .. } = callee else {
        return false;
    };
    let Some((accessor, _)) = accessor(path.rsplit('.').next().unwrap_or(path)) else {
        return false;
    };
    let shaped = match accessor {
        Accessor::Get => {
            stands == Stands::Inside && args.len() == 1 && returns.is_some_and(|n| n > 0)
        }
        Accessor::Set => stands == Stands::Alone && args.len() == 2 && returns == Some(0),
    };
    shaped && !args.first().is_some_and(address)
}

/// Whether `e` is the address of a place, which is how a value type's accessor is handed its value.
// ⚠ `x.P` of a value would need the printer to write the place and not its address.
fn address(e: &Expr) -> bool {
    match whole(e) {
        Expr::AddrOf { .. } | Expr::ByteOffset { .. } => true,
        Expr::Call {
            callee: Callee::Intrinsic { name, .. },
            ..
        } => name == "array_element_address",
        _ => false,
    }
}

/// Where the expressions at the top of a statement stand: a call that is the whole statement stands alone, a test and a value stored are taken.
// ⚠ A construct's parts and a loop's head may be either, so they say neither.
pub(super) fn standing(stmt: &Stmt) -> Stands {
    match stmt {
        Stmt::Simple(ir::Stmt::Eval(_)) => Stands::Alone,
        Stmt::Simple(ir::Stmt::Assign { .. } | ir::Stmt::Store { .. })
        | Stmt::If { .. }
        | Stmt::While { .. }
        | Stmt::DoWhile { .. }
        | Stmt::Switch { .. }
        | Stmt::Return(_) => Stands::Inside,
        _ => Stands::Unsaid,
    }
}
