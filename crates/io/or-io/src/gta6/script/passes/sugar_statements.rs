//! Sugar for statements: the loops a compiler expands, read back as `foreach`.
//!
//! Works on the structured form, after `async_await` and `closures`. A construct is made only
//! where every statement of the expansion is found in its place, and holds them all, so that
//! `tree::desugar` gives back the loop it replaced.
//!
//! ⚠ Nothing is made under `using`, `lock`, `try` or `for`: no input read so far holds a handler
//! or a monitor call, a `Dispose` in a straight line is what a person may have written, and the
//! structure pass already writes a counted loop as a `for`. A list's enumerator advanced in
//! place stays a loop too: its advance is statements, which a `Sugar::Foreach` has no part for.

use std::collections::BTreeMap;
use std::ops::Range;

use crate::gta6::script::ir::{self, BinOp, Callee, Expr, Head, Kind, Scalar, TypeKey, Var};
use crate::gta6::script::tree::{self, Stmt, Sugar};
use crate::gta6::script::{Context, Pass, Stats};

/// The names this file's rewrites are switched by.
pub const PASSES: &[Pass] = &[Pass::Foreach, Pass::Using, Pass::Lock, Pass::Try, Pass::For];

/// What reading one function's loops came to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Found {
    /// Counted loops over an array read as `foreach`.
    pub arrays: usize,
    /// Enumerator loops read as `foreach`, their disposal hidden.
    pub enumerators: usize,
    /// Loops with the head of one of those that a safety condition left as they were.
    pub refused: usize,
}

/// Reads each proved expansion as the statement of the source it came from.
pub fn run(f: &mut tree::Function, cx: &Context<'_>, _stats: &mut Stats) {
    rewrite(f, cx);
}

/// [`run`], telling what it read: `Stats` has no place for it.
pub fn rewrite(f: &mut tree::Function, cx: &Context<'_>) -> Found {
    let mut found = Found::default();
    if !cx.options.on(Pass::Foreach) {
        return found;
    }
    let scope = Scope::of(&f.head, &f.body, cx);
    // An explicit stack: a body nests as deep as its listing does.
    let mut work: Vec<&mut Vec<Stmt>> = vec![&mut f.body];
    while let Some(list) = work.pop() {
        let mut at = 0;
        while at < list.len() {
            let counted = matches!(&list[at], Stmt::For { cond: Some(cond), .. } if below_length(cond).is_some());
            let advanced = match &list[at] {
                Stmt::While { cond, .. } => called_on(tested(cond), Member::MoveNext, cx),
                _ => None,
            };
            if counted {
                match array_loop(&list[at]).filter(|vars| scope.proves(*vars)) {
                    Some(_) => {
                        let taken = std::mem::replace(&mut list[at], Stmt::Break);
                        list[at] = array_foreach(taken);
                        found.arrays += 1;
                    }
                    None => found.refused += 1,
                }
            } else if let Some(e) = advanced {
                match enumerator_foreach(list.as_slice(), at, e, &scope) {
                    Some((made, range)) => {
                        list.drain(range.start + 1..range.end);
                        list[range.start] = made;
                        at = range.start;
                        found.enumerators += 1;
                    }
                    None => found.refused += 1,
                }
            }
            at += 1;
        }
        // A loop just read is gone into too: the loops inside it are read in their turn.
        for stmt in list {
            work.extend(tree::nested_mut(stmt));
        }
    }
    found
}

/// How statements use one variable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Use {
    assigned: usize,
    /// Reads by name; the taking of its address is not one.
    read: usize,
    addressed: bool,
}

/// How a body, at every depth, uses each variable it names.
fn used(body: &[Stmt]) -> BTreeMap<Var, Use> {
    let mut uses: BTreeMap<Var, Use> = BTreeMap::new();
    tree::each_simple(body, &mut |s| {
        for target in s.targets() {
            uses.entry(*target).or_default().assigned += 1;
        }
    });
    tree::each_expr(body, &mut |top| {
        top.walk(&mut |e| match e {
            Expr::Var(var) => uses.entry(*var).or_default().read += 1,
            Expr::AddrOf { var, .. } => uses.entry(*var).or_default().addressed = true,
            _ => {}
        });
    });
    uses
}

/// The statement lists nested in `stmt`, a source construct's included.
fn nested(stmt: &Stmt) -> Vec<&[Stmt]> {
    match stmt {
        Stmt::If { then, els, .. } => vec![then.as_slice(), els.as_slice()],
        Stmt::While { body, .. } | Stmt::DoWhile { body, .. } | Stmt::For { body, .. } => {
            vec![body.as_slice()]
        }
        Stmt::Switch { cases, .. } => cases.iter().map(|case| case.body.as_slice()).collect(),
        Stmt::Sugar(sugar) => sugar.parts().bodies,
        _ => Vec::new(),
    }
}

/// Whether a body holds a label at any depth: a place something outside it may jump to.
fn labelled(body: &[Stmt]) -> bool {
    let mut work = vec![body];
    while let Some(list) = work.pop() {
        for stmt in list {
            if matches!(stmt, Stmt::Label(_)) {
                return true;
            }
            work.extend(nested(stmt));
        }
    }
    false
}

/// What the whole function says of its variables, which a loop alone cannot.
struct Scope<'a> {
    head: &'a Head,
    cx: Context<'a>,
    uses: BTreeMap<Var, Use>,
    /// The variables whose address is taken.
    addressed: Vec<Var>,
    /// How many loops [`array_loop`] takes each variable is the index of.
    indexed: BTreeMap<Var, usize>,
}

impl<'a> Scope<'a> {
    fn of(head: &'a Head, body: &[Stmt], cx: &Context<'a>) -> Scope<'a> {
        let uses = used(body);
        let addressed = (uses.iter().filter(|(_, uses)| uses.addressed))
            .map(|(var, _)| *var)
            .collect();
        let mut indexed = BTreeMap::new();
        let mut work = vec![body];
        while let Some(list) = work.pop() {
            for stmt in list {
                if let Some((index, ..)) = array_loop(stmt) {
                    *indexed.entry(index).or_insert(0) += 1;
                }
                work.extend(nested(stmt));
            }
        }
        Scope {
            head,
            cx: *cx,
            uses,
            addressed,
            indexed,
        }
    }

    /// The cells the address of `var` reaches: as many as its kind says, one where nothing says.
    fn cells(&self, var: Var) -> u32 {
        match self.head.vars.get(&var).map(|info| &info.kind) {
            Some(Kind::Vector | Kind::Handle) => 2,
            Some(Kind::Aggregate(cells) | Kind::Value { cells, .. }) => (*cells).max(1),
            _ => 1,
        }
    }

    /// Whether `var` is `held` or one of the cells after it that `held` takes.
    fn covers(&self, held: Var, var: Var) -> bool {
        match (held, var) {
            (Var::Local(m), Var::Local(n)) | (Var::Param(m), Var::Param(n)) => {
                m <= n && n - m < self.cells(held)
            }
            _ => held == var,
        }
    }

    /// Whether something other than an assignment that names `var` may change it: a global, or a cell an address taken in the function reaches.
    fn exposed(&self, var: Var) -> bool {
        let reached = self.addressed.iter().any(|held| self.covers(*held, var));
        matches!(var, Var::Global { .. }) || reached
    }

    /// Whether `e` is a constant or a variable no call changes: what a `return` hands back with the meaning it had before a disposal.
    fn settled(&self, e: &Expr) -> bool {
        match under(e, true) {
            Expr::Const(_) => true,
            Expr::Var(var) => !self.exposed(*var),
            _ => false,
        }
    }

    /// Whether the function names the index of an [`array_loop`] nowhere but in the heads of such loops, and changes none of its three variables behind their names.
    // ⚠ A slot is the index of several loops in turn, so the count is over all of them: each
    // sets the index before it reads it, and what is read after a loop is then never its index.
    fn proves(&self, (index, array, item): (Var, Var, Var)) -> bool {
        let loops = self.indexed.get(&index).copied().unwrap_or(0);
        let only = Use {
            assigned: 2 * loops,
            read: 3 * loops,
            addressed: false,
        };
        let apart = !self.covers(item, index) && !self.covers(item, array);
        let named_only = ![index, array, item]
            .into_iter()
            .any(|var| self.exposed(var));
        self.uses.get(&index) == Some(&only) && apart && named_only
    }
}

/// `e` under the source readings around it, the casts to 64 bits, and those to 32 where `narrow`.
fn under(e: &Expr, narrow: bool) -> &Expr {
    let mut e = e;
    loop {
        match e {
            Expr::Sugar { e: inner, .. } => e = &**inner,
            Expr::Cast {
                to: Scalar::I64 | Scalar::U64,
                e: inner,
            } => e = &**inner,
            Expr::Cast {
                to: Scalar::I32 | Scalar::U32,
                e: inner,
            } if narrow => e = &**inner,
            _ => return e,
        }
    }
}

fn var_of(e: &Expr, narrow: bool) -> Option<Var> {
    match under(e, narrow) {
        Expr::Var(var) => Some(*var),
        _ => None,
    }
}

fn is_const(e: &Expr, bits: u64) -> bool {
    matches!(under(e, true), Expr::Const(c) if c.bits == bits)
}

/// The arguments of `e` where it is a call of an intrinsic whose name `named` takes.
fn intrinsic(e: &Expr, named: impl Fn(&str) -> bool) -> Option<&[Expr]> {
    match under(e, false) {
        Expr::Call {
            callee: Callee::Intrinsic { name, .. },
            args,
            ..
        } if named(name.as_str()) => Some(args.as_slice()),
        _ => None,
    }
}

/// The index and the array of `cond` where it is `i < array_length(a)`.
fn below_length(cond: &Expr) -> Option<(Var, Var)> {
    let Expr::Binary {
        op: BinOp::Lt,
        l,
        r,
        ..
    } = under(cond, false)
    else {
        return None;
    };
    let [array] = intrinsic(under(r, true), |name| name == "array_length")? else {
        return None;
    };
    Some((var_of(l, true)?, var_of(array, false)?))
}

/// The variable `stmt` steps up by one, as a listing's `i++` reads.
fn stepped_up(stmt: &ir::Stmt) -> Option<Var> {
    let ir::Stmt::Assign { target, value } = stmt else {
        return None;
    };
    let Expr::Binary {
        op: BinOp::Add,
        l,
        r,
        ..
    } = under(value, false)
    else {
        return None;
    };
    (var_of(l, true) == Some(*target) && is_const(r, 1)).then_some(*target)
}

/// The index, the array and the item of `stmt` where it is `for (i = 0; i < array_length(a); i++) { x = a[i]; .. }` and the rest of its body names `i` nowhere and assigns neither `a` nor `x`.
// ⚠ The element's address taken in place of its value is no item: a write through it changes
// the array, which a `foreach` over copies would not.
fn array_loop(stmt: &Stmt) -> Option<(Var, Var, Var)> {
    let Stmt::For {
        init: Some(init),
        cond: Some(cond),
        step: Some(step),
        body,
    } = stmt
    else {
        return None;
    };
    let (i, a) = below_length(cond)?;
    let [Stmt::Simple(ir::Stmt::Assign { target: x, value }), rest @ ..] = body.as_slice() else {
        return None;
    };
    let [array, at] = intrinsic(value, |name| name.starts_with("array_get_"))? else {
        return None;
    };
    let starts =
        matches!(&**init, ir::Stmt::Assign { target, value } if *target == i && is_const(value, 0));
    let reads = var_of(array, false) == Some(a) && var_of(at, true) == Some(i);
    let apart = *x != i && *x != a && i != a;
    if !starts || !reads || !apart || stepped_up(step) != Some(i) || labelled(rest) {
        return None;
    }
    let rest = used(rest);
    let of = |var: Var| rest.get(&var).copied().unwrap_or_default();
    let quiet = of(i) == Use::default() && of(a).assigned == 0 && of(*x).assigned == 0;
    quiet.then_some((i, a, *x))
}

/// The `foreach` that a loop [`array_loop`] takes is; any other statement as it is.
fn array_foreach(stmt: Stmt) -> Stmt {
    match stmt {
        Stmt::For {
            init: Some(init),
            cond: Some(cond),
            step,
            mut body,
        } if !body.is_empty() => match body.remove(0) {
            Stmt::Simple(item) => Stmt::Sugar(Box::new(Sugar::Foreach {
                setup: vec![*init],
                cond,
                item: Box::new(item),
                body,
                step,
                after: Vec::new(),
            })),
            first => {
                body.insert(0, first);
                Stmt::For {
                    init: Some(init),
                    cond: Some(cond),
                    step,
                    body,
                }
            }
        },
        other => other,
    }
}

/// What a virtual slot is to a `foreach`, under the name C# itself knows the member by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Member {
    GetEnumerator,
    MoveNext,
    Current,
    Dispose,
}

/// The member a callee is, by the name the user's list gives its slot on the type, or on the generic type the type is made from.
// ⚠ OpenRage ships no names: with no list nothing is a member, and no enumerator loop is read
// as a `foreach`.
fn member_of(callee: &Callee, cx: &Context<'_>) -> Option<Member> {
    let Callee::Virtual { ty, slot, .. } = callee else {
        return None;
    };
    let key = TypeKey::of_name(ty)?;
    let made_from = || Some(cx.project.types.get(&key)?.generic.as_ref()?.0);
    let listed = cx.names.method(key, *slot);
    match listed.or_else(|| cx.names.method(made_from()?, *slot))? {
        "GetEnumerator" => Some(Member::GetEnumerator),
        "MoveNext" => Some(Member::MoveNext),
        "Current" | "get_Current" => Some(Member::Current),
        "Dispose" => Some(Member::Dispose),
        _ => None,
    }
}

/// The variable a call of `member` is made on, with no other argument.
fn called_on(e: &Expr, member: Member, cx: &Context<'_>) -> Option<Var> {
    let Expr::Call { callee, args, .. } = under(e, false) else {
        return None;
    };
    let [receiver] = args.as_slice() else {
        return None;
    };
    (member_of(callee, cx) == Some(member))
        .then(|| var_of(receiver, false))
        .flatten()
}

/// What `cond` asks to be other than zero.
fn tested(cond: &Expr) -> &Expr {
    let mut e = cond;
    loop {
        match e {
            Expr::Sugar { e: inner, .. } | Expr::Cast { e: inner, .. } | Expr::Truth(inner) => {
                e = &**inner;
            }
            Expr::Binary {
                op: BinOp::Ne,
                l,
                r,
                ..
            } if is_const(r, 0) => e = &**l,
            _ => return e,
        }
    }
}

/// The statements `stmt` runs where `var` is not null, where it is that test and nothing else.
fn unless_null(stmt: &Stmt, var: Var) -> Option<&[Stmt]> {
    let Stmt::If { cond, then, els } = stmt else {
        return None;
    };
    let Expr::Binary { op, l, r, .. } = under(cond, false) else {
        return None;
    };
    if var_of(l, false) != Some(var) || !is_const(r, 0) {
        return None;
    }
    match (op, then.is_empty(), els.is_empty()) {
        (BinOp::Ne, false, true) => Some(then.as_slice()),
        (BinOp::Eq, true, false) => Some(els.as_slice()),
        _ => None,
    }
}

/// The shape of a disposal: the call alone, the call where the enumerator is not null, or the call on a cast of it that is not null.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Form {
    Direct,
    Guarded,
    Cast,
}

/// The shape of `seq` where it is the whole of a disposal of `e`, and the variable the cast of `e` is held in for it.
fn disposal(seq: &[Stmt], e: Var, cx: &Context<'_>) -> Option<(Form, Option<Var>)> {
    let disposes = |stmt: &Stmt, on: Var| matches!(stmt, Stmt::Simple(ir::Stmt::Eval(call)) if called_on(call, Member::Dispose, cx) == Some(on));
    let guarded =
        |stmt: &Stmt, on: Var| matches!(unless_null(stmt, on), Some([call]) if disposes(call, on));
    match seq {
        [only] if disposes(only, e) => Some((Form::Direct, None)),
        [only] if guarded(only, e) => Some((Form::Guarded, None)),
        [Stmt::Simple(ir::Stmt::Assign { target, value }), test] => {
            let [cast] = intrinsic(value, |name| name.starts_with("try_cast<"))? else {
                return None;
            };
            let held = var_of(cast, false) == Some(e) && *target != e && guarded(test, *target);
            held.then_some((Form::Cast, Some(*target)))
        }
        _ => None,
    }
}

/// Adds to `expected` the uses one disposal of `e` makes of `e` and of the variable it casts to.
fn expect(expected: &mut BTreeMap<Var, Use>, e: Var, (form, cast): (Form, Option<Var>)) {
    expected.entry(e).or_default().read += match form {
        Form::Guarded => 2,
        Form::Direct | Form::Cast => 1,
    };
    if let Some(cast) = cast {
        let held = expected.entry(cast).or_default();
        held.assigned += 1;
        held.read += 2;
    }
}

/// Wraps the disposal before each `return` of `body` as hidden statements; `None` where a way out skips it, takes another shape, hands back what it may change, or is a jump.
// ⚠ A `return` runs its value before the disposal a `foreach` hides, and here after it: only a
// constant or a variable no call changes reads the same both ways.
fn hide(
    body: &mut Vec<Stmt>,
    e: Var,
    form: Option<Form>,
    scope: &Scope<'_>,
    expected: &mut BTreeMap<Var, Use>,
) -> Option<()> {
    let mut work: Vec<&mut Vec<Stmt>> = vec![body];
    while let Some(list) = work.pop() {
        let mut at = 0;
        while at < list.len() {
            let settled = match &list[at] {
                Stmt::Goto(_) | Stmt::Label(_) => return None,
                Stmt::Return(value) => Some(value.as_ref().is_none_or(|v| scope.settled(v))),
                _ => None,
            };
            if let (Some(settled), Some(form)) = (settled, form) {
                // The disposal of a loop around this one is hidden already, and stands between.
                let mut end = at;
                while end > 0
                    && matches!(&list[end - 1], Stmt::Sugar(s) if matches!(**s, Sugar::Hidden(_)))
                {
                    end -= 1;
                }
                let (found, len) = [2usize, 1].into_iter().find_map(|len| {
                    let seq = list.get(end.checked_sub(len)?..end)?;
                    Some((disposal(seq, e, &scope.cx)?, len))
                })?;
                if !settled || found.0 != form {
                    return None;
                }
                expect(expected, e, found);
                let seq: Vec<Stmt> = list.drain(end - len..end).collect();
                list.insert(end - len, Stmt::Sugar(Box::new(Sugar::Hidden(seq))));
                at -= len - 1;
            }
            at += 1;
        }
        for stmt in list {
            work.extend(tree::nested_mut(stmt));
        }
    }
    Some(())
}

/// The `foreach` that the enumerator `e` taken at `list[at - 1]`, the loop at `list[at]` and the disposal after it are, and the statements it stands in place of.
// Every way out disposes in one shape or none does, and the function names `e` nowhere else:
// the count of its uses is what proves that no disposal and no other use was missed.
fn enumerator_foreach(
    list: &[Stmt],
    at: usize,
    e: Var,
    scope: &Scope<'_>,
) -> Option<(Stmt, Range<usize>)> {
    let cx = &scope.cx;
    let first = at.checked_sub(1)?;
    let Stmt::Simple(setup @ ir::Stmt::Assign { target, value }) = list.get(first)? else {
        return None;
    };
    let Expr::Call { callee, args, .. } = under(value, false) else {
        return None;
    };
    let taken = *target == e && args.len() == 1;
    if !taken || member_of(callee, cx) != Some(Member::GetEnumerator) {
        return None;
    }
    let Stmt::While { cond, body } = list.get(at)? else {
        return None;
    };
    let [Stmt::Simple(item @ ir::Stmt::Assign { target: x, value }), rest @ ..] = body.as_slice()
    else {
        return None;
    };
    // ⚠ The item is what the member the list names `Current` yields on the enumerator alone:
    // any other call there is a statement of the body, which a `foreach` would hide.
    let Expr::Call { callee, args, .. } = under(value, false) else {
        return None;
    };
    let [receiver] = args.as_slice() else {
        return None;
    };
    let current = member_of(callee, cx) == Some(Member::Current);
    if !current || var_of(receiver, false) != Some(e) || scope.covers(*x, e) {
        return None;
    }

    let after = list.get(at + 1..)?;
    let ending = [2usize, 1]
        .into_iter()
        .find_map(|len| Some((disposal(after.get(..len)?, e, cx)?, len)));
    let len = ending.map_or(0, |(_, len)| len);
    let only = Use {
        assigned: 1,
        read: 2,
        addressed: false,
    };
    let mut expected = BTreeMap::from([(e, only)]);
    if let Some((found, _)) = ending {
        expect(&mut expected, e, found);
    }
    let mut body = rest.to_vec();
    hide(
        &mut body,
        e,
        ending.map(|(found, _)| found.0),
        scope,
        &mut expected,
    )?;

    let assigned_again = used(&body).get(x).is_some_and(|uses| uses.assigned > 0);
    let counted = (expected.iter())
        .all(|(var, uses)| scope.uses.get(var) == Some(uses) && !scope.exposed(*var));
    if assigned_again || scope.exposed(*x) || !counted {
        return None;
    }
    let made = Sugar::Foreach {
        setup: vec![setup.clone()],
        cond: cond.clone(),
        item: Box::new(item.clone()),
        body,
        step: None,
        after: match len {
            0 => Vec::new(),
            _ => vec![Stmt::Sugar(Box::new(Sugar::Hidden(after[..len].to_vec())))],
        },
    };
    Some((Stmt::Sugar(Box::new(made)), first..at + 1 + len))
}

#[cfg(test)]
#[path = "sugar_statements_tests.rs"]
mod tests;
