//! Lambdas written where they are bound: which bodies a function binds in place, and the writer
//! of one function that writes them there.

use super::*;

/// The lambdas of a program whose bodies are written where they are bound, each by its offset.
pub(crate) type Lambdas<'a> = BTreeMap<u32, &'a tree::Function>;

/// Writes one function as [`method`] does, the body of each lambda of `lambdas` it binds written where it binds it.
pub(crate) fn method_in(
    out: &mut String,
    method: &Method,
    class: Option<(TypeKey, &str)>,
    depth: usize,
    cx: &Context<'_>,
    lambdas: &Lambdas<'_>,
) {
    let mut writer = Writer::in_context(out, cx, &method.function.head);
    writer.depth = depth;
    // ⚠ A lambda's own body binds none in place: written in place itself, it is written by a
    // writer that holds no lambda, and what it binds must be written in one place either way.
    if method.role != Role::Lambda {
        writer.sited = bound_in_place(&method.function, lambdas, cx);
        writer.lambdas = Some(lambdas);
    }
    writer.as_member(method, class);
}

/// `e` under its source readings and its casts to a whole cell, which change no bit of what it holds.
pub(super) fn underneath(e: &Expr) -> &Expr {
    let mut e = e;
    while let Expr::Sugar { e: inner, .. }
    | Expr::Cast {
        to: Scalar::I64 | Scalar::U64,
        e: inner,
    } = e
    {
        e = &**inner;
    }
    e
}

/// What a delegate is bound to and the offset of the function bound, where `parts` are a delegate made and bound to a function of `program` that is read as a lambda's body.
pub(super) fn bound_lambda(parts: &[Stmt], program: u32) -> Option<(&Expr, u32)> {
    let [Stmt::Assign { target, value }, Stmt::Eval(call)] = parts else {
        return None;
    };
    if !matches!(underneath(value), Expr::NewObject(_)) {
        return None;
    }
    let Expr::Call { args, .. } = underneath(call) else {
        return None;
    };
    let [object, held, Expr::Sugar {
        kind: ExprSugar::Lambda,
        e: function,
    }] = args.as_slice()
    else {
        return None;
    };
    let Expr::FunctionRef { program: of, at } = &**function else {
        return None;
    };
    let made = *underneath(object) == Expr::Var(*target);
    (made && *of == program).then_some((held, *at))
}

/// The name a variable of a function is written under where no class gives it a word of its own.
pub(super) fn label(head: &Head, var: Var) -> String {
    match head.vars.get(&var).and_then(|info| info.name.as_deref()) {
        Some(name) => name.to_owned(),
        None => var.to_string(),
    }
}

/// Whether `body`, written as a lambda in `binder` where it is bound to `held`, reads the object it was bound to and no other.
// ⚠ In place the body names the binder's variable where it named its own first parameter. That
// variable must hold one object for as long as the function runs, and nothing of the body may
// go by its name.
pub(super) fn reads_as_bound(binder: &tree::Function, held: &Expr, body: &tree::Function) -> bool {
    let mut mentioned: BTreeSet<Var> = (1..body.head.params).map(Var::Param).collect();
    tree::each_simple(&body.body, &mut |s| {
        mentioned.extend(s.targets().iter().copied())
    });
    tree::each_expr(&body.body, &mut |top| {
        top.walk(&mut |e| {
            if let Expr::Var(var) | Expr::AddrOf { var, .. } = e {
                mentioned.insert(*var);
            }
        });
    });
    let reads = mentioned.remove(&Var::Param(0));
    let object = match underneath(held) {
        Expr::Var(object @ (Var::Local(_) | Var::Value { .. } | Var::Stack { .. })) => Some(object),
        _ => None,
    };
    // A body that never reads its object says nothing of what it was bound to: only its own
    // variables must not go by the name of the variable it is given to.
    if !reads {
        let text = object.map(|object| label(&binder.head, *object));
        return mentioned
            .iter()
            .all(|var| Some(label(&body.head, *var)) != text);
    }
    let Some(object) = object else {
        return false;
    };
    let (mut assigned, mut addressed) = (0usize, false);
    tree::each_simple(&binder.body, &mut |s| {
        assigned += s
            .targets()
            .iter()
            .filter(|target| *target == object)
            .count();
    });
    tree::each_expr(&binder.body, &mut |top| {
        top.walk(&mut |e| {
            addressed |= matches!(e, Expr::AddrOf { var, .. } if var == object);
        });
    });
    let text = label(&binder.head, *object);
    let own = |var: &Var| label(&body.head, *var) != text;
    assigned == 1 && !addressed && mentioned.iter().all(own)
}

/// The lambdas of `bodies` that `binder` binds and whose bodies are written where it binds them, by offset.
// ⚠ Only a statement every setting of the passes writes is read: a construct may leave some of
// what it holds unwritten, and a body counted here is written nowhere else.
pub(crate) fn bound_in_place(
    binder: &tree::Function,
    bodies: &Lambdas<'_>,
    cx: &Context<'_>,
) -> BTreeSet<u32> {
    let mut found = BTreeSet::new();
    let on =
        true_csharp(&cx.options) && cx.options.on(Pass::Delegates) && cx.options.on(Pass::Lambdas);
    if !on || bodies.is_empty() || binder.head.incomplete.is_some() {
        return found;
    }
    let steady = closures::inlinable(binder);
    if steady.is_empty() {
        return found;
    }
    // An explicit stack: a body nests as deep as its listing does.
    let mut work: Vec<&[tree::Stmt]> = vec![binder.body.as_slice()];
    while let Some(list) = work.pop() {
        for stmt in list {
            match stmt {
                tree::Stmt::If { then, els, .. } => {
                    work.extend([then.as_slice(), els.as_slice()]);
                }
                tree::Stmt::While { body, .. }
                | tree::Stmt::DoWhile { body, .. }
                | tree::Stmt::For { body, .. } => work.push(body.as_slice()),
                tree::Stmt::Switch { cases, .. } => {
                    work.extend(cases.iter().map(|case| case.body.as_slice()));
                }
                tree::Stmt::Sugar(sugar) => {
                    let tree::Sugar::Group {
                        kind: tree::Group::Delegate,
                        parts,
                    } = &**sugar
                    else {
                        continue;
                    };
                    let Some((held, at)) = bound_lambda(parts, binder.head.program) else {
                        continue;
                    };
                    let Some(body) = bodies.get(&at).copied() else {
                        continue;
                    };
                    if steady.contains(&at) && reads_as_bound(binder, held, body) {
                        found.insert(at);
                    }
                }
                _ => {}
            }
        }
    }
    found
}
