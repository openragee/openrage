//! The statements that only fill an object the function has just made, which no one else holds yet: a value may move past them.

use std::collections::HashMap;

use crate::gta6::script::cfg::{self, BlockId};
use crate::gta6::script::ir::{Callee, Expr, Stmt, Var, Width};

use super::super::effects::{stored, Effects, Facts};
use super::{mentioned, REACH};

/// The bit a runtime type's number carries where the runtime, not a program, declares the type.
const BUILTIN: u32 = 0x8000_0000;

/// How a statement fills an object no one else holds yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fill {
    /// The delegate the statement before made is bound to its target and function.
    Bound,
    /// An element of an array made of a stated length is set at a stated index inside it.
    Set,
}

/// Where a block fills an object it has just made.
// An object the function made is in no other hands until its variable goes anywhere but into
// the address of a load or a store, or into the object argument of what fills it: a call or a
// load evaluated before such a statement or after it reads and does the same.
pub(super) struct Made {
    fills: HashMap<(BlockId, usize), (Var, Fill)>,
}

/// Whether `stmt` names `var` anywhere but as the address a load or a store goes through.
fn loose(stmt: &Stmt, var: Var) -> bool {
    let mut count = 0i64;
    stmt.each_expr(&mut |top| {
        top.walk(&mut |e| match e {
            Expr::Var(read) | Expr::AddrOf { var: read, .. } if *read == var => count += 1,
            Expr::Load { addr, .. } | Expr::LoadBytes { addr, .. } if **addr == Expr::Var(var) => {
                count -= 1;
            }
            _ => {}
        });
    });
    let through = matches!(stored(stmt), Some((Expr::Var(base), ..)) if *base == var);
    stmt.targets().contains(&var) || count - i64::from(through) > 0
}

/// Whether `stmt` binds the delegate `var` holds, made as an object of the runtime's type `id`: `Ctor(var, target, function)`.
fn bound(stmt: &Stmt, var: Var, id: u32) -> bool {
    let Stmt::Eval(Expr::Call {
        callee: Callee::Builtin { ty, .. },
        args,
        ..
    }) = stmt
    else {
        return false;
    };
    let [object, target, function] = args.as_slice() else {
        return false;
    };
    let own = id & BUILTIN != 0 && id & !BUILTIN == *ty;
    let reference = matches!(
        function,
        Expr::FunctionRef { .. }
            | Expr::BuiltinFunctionRef { .. }
            | Expr::VirtualFunctionRef { .. }
    );
    let apart = !mentioned(target, var) && !mentioned(function, var);
    own && reference && apart && *object == Expr::Var(var)
}

/// Whether `stmt` sets an element of the array `var` holds, of `len` elements, at a constant index below `len`, with values that name `var` nowhere.
// The runtime reads the index and the length as `int`s: neither can fault at such an index.
fn set(stmt: &Stmt, var: Var, len: i32) -> bool {
    let Stmt::Eval(Expr::Call {
        callee: Callee::Intrinsic { name, .. },
        args,
        ..
    }) = stmt
    else {
        return false;
    };
    let [array, Expr::Const(index), values @ ..] = args.as_slice() else {
        return false;
    };
    let index = index.bits as u32 as i32;
    let inside = 0 <= index && index < len;
    let apart = !values.is_empty() && !values.iter().any(|value| mentioned(value, var));
    name.starts_with("array_set_") && *array == Expr::Var(var) && inside && apart
}

/// Whether a block of `f` holds an object made and filled as [`Made::of`] looks for, by the form alone.
pub(super) fn any(f: &cfg::Function, (delegates, arrays): (bool, bool)) -> bool {
    f.cfg.blocks.iter().any(|block| {
        block.stmts.windows(2).any(|pair| match pair {
            [Stmt::Assign {
                target,
                value: Expr::NewObject(ty),
            }, next] => delegates && bound(next, *target, ty.id),
            [Stmt::Assign {
                target,
                value: Expr::NewArray { len, .. },
            }, next] => match &**len {
                Expr::Const(len) => arrays && set(next, *target, len.bits as u32 as i32),
                _ => false,
            },
            _ => false,
        })
    })
}

impl Made {
    /// The fills of `f`'s blocks: the binding of a delegate with `delegates`, the setting of an array's elements with `arrays`.
    pub(super) fn of(f: &cfg::Function, facts: &Facts, (delegates, arrays): (bool, bool)) -> Made {
        let mut fills: HashMap<(BlockId, usize), (Var, Fill)> = HashMap::new();
        if !(delegates || arrays) {
            return Made { fills };
        }
        for (b, block) in f.cfg.blocks.iter().enumerate() {
            for (a, stmt) in block.stmts.iter().enumerate() {
                let Stmt::Assign { target, value } = stmt else {
                    continue;
                };
                let var = *target;
                let once =
                    (facts.uses.get(&var)).is_some_and(|uses| uses.defs == 1 && uses.addrs == 0);
                let narrow = (f.head.vars.get(&var)).is_none_or(|info| info.width != Width::Cells);
                if !(once && narrow && facts.named_only(var)) {
                    continue;
                }
                match value {
                    Expr::NewObject(ty) if delegates => {
                        let next = block.stmts.get(a + 1);
                        if next.is_some_and(|next| bound(next, var, ty.id)) {
                            fills.insert((b, a + 1), (var, Fill::Bound));
                        }
                    }
                    Expr::NewArray { len, .. } if arrays => {
                        let Expr::Const(len) = &**len else {
                            continue;
                        };
                        let len = len.bits as u32 as i32;
                        let end = block.stmts.len().min(a + 1 + REACH);
                        for (j, later) in block.stmts.iter().enumerate().take(end).skip(a + 1) {
                            if set(later, var, len) {
                                fills.insert((b, j), (var, Fill::Set));
                            } else if loose(later, var) {
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Made { fills }
    }

    /// Whether no statement was found to fill an object.
    pub(super) fn fills_nothing(&self) -> bool {
        self.fills.is_empty()
    }

    /// Whether `stmt`, at `site`, does nothing but fill an object no one else holds yet and what `value`, which does `moving`, may pass.
    // ⚠ `value` must name the object nowhere: it would read what the statement fills. A delegate's
    // constructor may refuse its target, so only a value that reads and does nothing else passes
    // it: such a value faults where it stands or not at all, and changes nothing either way.
    pub(super) fn lets_pass(
        &self,
        facts: &Facts,
        stmt: &Stmt,
        (value, moving): (&Expr, &Effects),
        site: (BlockId, usize),
    ) -> bool {
        let (var, fill, rest): (Var, Fill, Vec<&Expr>) = match self.fills.get(&site) {
            Some((var, fill)) => match stmt {
                Stmt::Eval(Expr::Call { args, .. }) if args.first() == Some(&Expr::Var(*var)) => {
                    (*var, *fill, args.iter().skip(1).collect())
                }
                _ => return false,
            },
            None => return false,
        };
        if mentioned(value, var) {
            return false;
        }
        if fill == Fill::Bound && (moving.call || moving.alloc || moving.store) {
            return false;
        }
        let mut parts = rest.into_iter();
        let Some(first) = parts.next() else {
            return false;
        };
        let mut effects = Effects::of_expr(facts, first);
        for part in parts {
            part.walk(&mut |e| effects.node(facts, e));
        }
        moving.may_cross(&effects.placed(site))
    }
}

#[cfg(test)]
#[path = "values_fold_made_tests.rs"]
mod tests;
