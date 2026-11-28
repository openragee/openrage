//! A field of a closure object reached through a variable that holds its address: where the
//! variable is read only for that field's cell, it is read away with the object.

use super::*;

/// The most variables that hold a field's address [`pointers`] reads of one object; past them the rest stay, and so does the object.
const POINTERS: usize = 64;

/// How often each variable of a body is assigned, read or has its address taken.
pub(super) fn mentions(body: &[tree::Stmt]) -> BTreeMap<Var, usize> {
    let mut found: BTreeMap<Var, usize> = BTreeMap::new();
    tree::each_simple(body, &mut |s| {
        for target in s.targets() {
            *found.entry(*target).or_default() += 1;
        }
    });
    tree::each_expr(body, &mut |top| {
        top.walk(&mut |e| {
            if let Expr::Var(var) | Expr::AddrOf { var, .. } = e {
                *found.entry(*var).or_default() += 1;
            }
        });
    });
    found
}

/// The variables of `binder` that hold the address of a field of one cell of `object`, each with the field's offset: assigned it once, from `list[from..]` on, by a statement before which nothing names the variable and from which on no label stands in its list, and reached through no address.
// ⚠ Such a variable holds the field's address wherever it is read: the statement that gives it
// runs before every other mention, which all follow it in its list, as nothing jumps past it
// into them, and nothing else writes the variable. `now` counts the body as it is now written and
// `binder` as it was read: a mention another object's reading took away sets them apart.
pub(super) fn pointers(
    list: &[tree::Stmt],
    from: usize,
    object: Var,
    plan: &Plan,
    binder: &Binder<'_>,
    now: &BTreeMap<Var, usize>,
) -> BTreeMap<Var, u32> {
    let mut found = BTreeMap::new();
    let mut tried = 0usize;
    // An explicit stack: a body nests as deep as its listing does.
    let mut work: Vec<(&[tree::Stmt], usize)> = vec![(list, from)];
    while let Some((stmts, start)) = work.pop() {
        for (k, stmt) in stmts.iter().enumerate().skip(start) {
            work.extend(lists(stmt).into_iter().map(|inner| (inner, 0)));
            let tree::Stmt::Simple(Stmt::Assign { target, value }) = stmt else {
                continue;
            };
            let Some(offset) = field_address(value, object) else {
                continue;
            };
            tried += 1;
            if tried > POINTERS {
                return found;
            }
            let whole = binder.facts.mentions.get(target).copied().unwrap_or(0);
            let rest = stmts.get(k.saturating_add(1)..).unwrap_or_default();
            let after = mentions(rest).get(target).copied().unwrap_or(0);
            let alone = plan.cells.contains_key(&offset)
                && binder.private(*target)
                && now.get(target) == Some(&whole)
                && after.checked_add(1) == Some(whole)
                && tree::counts(stmts.get(k..).unwrap_or_default()).labels == 0;
            if alone {
                found.insert(*target, offset);
            }
        }
    }
    found
}

/// The statement lists nested in `stmt`, a source construct's included.
fn lists(stmt: &tree::Stmt) -> Vec<&[tree::Stmt]> {
    match stmt {
        tree::Stmt::If { then, els, .. } => vec![then.as_slice(), els.as_slice()],
        tree::Stmt::While { body, .. }
        | tree::Stmt::DoWhile { body, .. }
        | tree::Stmt::For { body, .. } => vec![body.as_slice()],
        tree::Stmt::Switch { cases, .. } => cases.iter().map(|case| case.body.as_slice()).collect(),
        tree::Stmt::Sugar(sugar) => sugar.parts().bodies,
        _ => Vec::new(),
    }
}

/// The offset of the field of `object` whose address `value` is.
fn field_address(value: &Expr, object: Var) -> Option<u32> {
    let Expr::ByteOffset { addr, offset } = underneath(value) else {
        return None;
    };
    let Expr::Const(c) = &**offset else {
        return None;
    };
    if *underneath(addr) != Expr::Var(object) {
        return None;
    }
    u32::try_from(c.bits).ok()
}

impl Binder<'_> {
    /// Whether `var` is a variable of the frame that holds no closure object, assigned once and reached through no address.
    // ⚠ An address of a parameter reaches every cell from it on, the locals included; one of a
    // local, every local from it on.
    fn private(&self, var: Var) -> bool {
        let addressed = &self.facts.addressed;
        let reached = match var {
            Var::Local(n) => addressed
                .iter()
                .any(|v| matches!(v, Var::Param(_)) || matches!(v, Var::Local(k) if *k <= n)),
            Var::Value { .. } | Var::Stack { .. } => addressed.contains(&var),
            Var::Param(_) | Var::Global { .. } => true,
        };
        let once = self.facts.assigned.get(&var) == Some(&1);
        once && !reached && !self.closures.contains(&var)
    }
}

/// Reads each store of a whole cell through a variable of `deref`, among what `stmt` holds itself, as the assignment of the variable whose address it holds.
pub(super) fn assign_through(stmt: &mut tree::Stmt, deref: &BTreeMap<Var, Var>) {
    if deref.is_empty() {
        return;
    }
    let target_of = |s: &Stmt| {
        let Stmt::Store {
            kind: StoreKind::U64 | StoreKind::Ref,
            addr,
            offset: 0,
            ..
        } = s
        else {
            return None;
        };
        match underneath(addr) {
            Expr::Var(pointer) => deref.get(pointer).copied(),
            _ => None,
        }
    };
    assign_where(stmt, &target_of);
}

/// Reads each store among what `stmt` holds itself for which `target_of` names a variable as the assignment of that variable.
pub(super) fn assign_where(stmt: &mut tree::Stmt, target_of: &dyn Fn(&Stmt) -> Option<Var>) {
    let assign = |s: &mut Stmt| {
        let Some(target) = target_of(s) else {
            return;
        };
        let old = std::mem::replace(s, Stmt::Eval(Expr::GuidEmpty));
        *s = match old {
            Stmt::Store { value, .. } => Stmt::Assign { target, value },
            other => other,
        };
    };
    match stmt {
        tree::Stmt::Simple(s) => assign(s),
        tree::Stmt::For { init, step, .. } => {
            for s in [init, step].into_iter().flatten() {
                assign(s);
            }
        }
        tree::Stmt::Sugar(sugar) => {
            for s in sugar.parts_mut().simples {
                assign(s);
            }
        }
        _ => {}
    }
    let lone = match stmt {
        tree::Stmt::Sugar(sugar) => match &mut **sugar {
            tree::Sugar::Group {
                kind: tree::Group::Field { .. },
                parts,
            } if matches!(parts.as_slice(), [Stmt::Assign { .. }]) => parts.pop(),
            _ => None,
        },
        _ => None,
    };
    if let Some(s) = lone {
        *stmt = tree::Stmt::Simple(s);
    }
}

/// Reads each load of a whole cell through a variable of `deref` as the variable whose address it holds.
// The cell it reads is the one the variable stands for, so the two hold the same bits.
pub(super) fn read_through(top: &mut Expr, deref: &BTreeMap<Var, Var>) {
    if deref.is_empty() {
        return;
    }
    top.walk_mut(&mut |e| {
        let read = match &*e {
            Expr::Load {
                ty: Scalar::U64 | Scalar::I64,
                addr,
                offset: 0,
            } => match underneath(addr) {
                Expr::Var(pointer) => deref.get(pointer).copied(),
                _ => None,
            },
            _ => None,
        };
        if let Some(var) = read {
            *e = Expr::Var(var);
        }
    });
}

/// Takes out of `body` each statement that gives a variable of `through` the address of its field of `object`: nothing reads it once its loads and stores are read away.
pub(super) fn unpointed(body: &mut Vec<tree::Stmt>, object: Var, through: &BTreeMap<Var, u32>) {
    if through.is_empty() {
        return;
    }
    let pointing = |stmt: &tree::Stmt| match stmt {
        tree::Stmt::Simple(Stmt::Assign { target, value }) => {
            (through.get(target)).is_some_and(|field| field_address(value, object) == Some(*field))
        }
        _ => false,
    };
    // An explicit stack: a body nests as deep as its listing does.
    let mut work: Vec<&mut Vec<tree::Stmt>> = vec![body];
    while let Some(list) = work.pop() {
        list.retain(|stmt| !pointing(stmt));
        for stmt in list {
            work.extend(tree::nested_mut(stmt));
        }
    }
}
