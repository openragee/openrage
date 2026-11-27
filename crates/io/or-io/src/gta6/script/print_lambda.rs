//! Lambdas written where they are bound: which bodies a function binds in place, and the writer
//! of one function that writes them there; the closure classes written nowhere once every lambda
//! of theirs is; and the method group a delegate of no lambda reads as.

use super::*;
use crate::gta6::script::class::{Class, ClassKind, Unit};
use crate::gta6::script::ir::VarInfo;

/// The lambdas of a program whose bodies are written where they are bound, each by its offset.
pub(crate) type Lambdas<'a> = BTreeMap<u32, &'a tree::Function>;

/// The most rounds [`Hidden::of`] takes to settle which classes it hides; past them it hides none.
const ROUNDS: usize = 8;
/// The most arrays one kind is read through for the type it names.
const NESTING: usize = 64;

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

/// Writes one function as [`method_in`] does, as `hidden` writes it where it makes an object of a class written nowhere.
pub(crate) fn method_hiding(
    out: &mut String,
    method: &Method,
    class: Option<(TypeKey, &str)>,
    depth: usize,
    cx: &Context<'_>,
    lambdas: &Lambdas<'_>,
    hidden: &Hidden,
) {
    let held = match method.role {
        Role::Lambda => None,
        _ => hidden.binders.get(&method.function.head.at),
    };
    let Some((written, sited)) = held else {
        return method_in(out, method, class, depth, cx, lambdas);
    };
    // The lambdas of a hidden class read what their object held as the variables it was given.
    let bodies: Lambdas<'_> = lambdas
        .iter()
        .map(|(at, body)| (*at, hidden.bodies.get(at).unwrap_or(*body)))
        .collect();
    let mut writer = Writer::in_context(out, cx, &written.function.head);
    writer.depth = depth;
    // ⚠ What the original binds in place: the object a body was bound to is gone from `written`.
    writer.sited = sited.clone();
    writer.lambdas = Some(&bodies);
    writer.as_member(written, class);
}

/// The closure classes of a unit written nowhere, with what is written in their place.
#[derive(Debug, Default)]
pub(crate) struct Hidden {
    classes: BTreeSet<TypeKey>,
    /// Each function that makes an object of one of them, by offset, as it is written, with the lambdas the original binds in place.
    binders: BTreeMap<u32, (Method, BTreeSet<u32>)>,
    /// Each lambda of one of them, by offset, as it is written where it is bound.
    bodies: BTreeMap<u32, tree::Function>,
}

impl Hidden {
    /// Whether the class `key` is written nowhere.
    pub(crate) fn hides(&self, key: TypeKey) -> bool {
        self.classes.contains(&key)
    }

    /// The closure classes of `unit` every lambda of which `lambdas` writes in place and that nothing written otherwise names, with their binders and bodies as they are then written.
    // ⚠ A closure captures a variable and not its value: the object's field is what both sides
    // read. A field reads as the variable it was given only where neither is written after the
    // object is given it, and the object reaches nothing but the lambdas bound to it. Each part
    // is proved on its own copy: the tree keeps the object, and `lambdas` off writes it.
    pub(crate) fn of(unit: &Unit, lambdas: &Lambdas<'_>, cx: &Context<'_>) -> Hidden {
        let on = cx.options.level == Level::Source
            && true_csharp(&cx.options)
            && cx.options.on(Pass::Delegates)
            && cx.options.on(Pass::Lambdas);
        if !on || lambdas.is_empty() || !unimported(unit.program, cx.project) {
            return Hidden::default();
        }
        // Every method of the unit, and whether it is written as a member of a class.
        let mut methods: Vec<(&Method, bool)> = unit.functions.iter().map(|m| (m, false)).collect();
        let mut classes: Vec<&Class> = Vec::new();
        let mut open: Vec<&Class> = unit.classes.iter().collect();
        while let Some(class) = open.pop() {
            classes.push(class);
            methods.extend(class.methods.iter().map(|m| (m, true)));
            open.extend(&class.nested);
        }
        let mut keep: BTreeSet<TypeKey> = (classes.iter())
            .filter(|class| candidate(class, lambdas, cx.project))
            .map(|class| class.key)
            .collect();
        if keep.is_empty() {
            return Hidden::default();
        }
        let owned: BTreeMap<TypeKey, BTreeSet<u32>> = (classes.iter())
            .filter(|class| keep.contains(&class.key))
            .map(|class| {
                let ats = class.methods.iter().map(|m| m.function.head.at);
                (class.key, ats.collect())
            })
            .collect();
        let binders: Vec<Binder<'_>> = (methods.iter())
            .filter_map(|(method, member)| Binder::of(method, *member, &keep, lambdas, cx))
            .collect();
        let found = Found {
            methods: &methods,
            classes: &classes,
            binders: &binders,
            owned: &owned,
            program: unit.program,
        };
        // Each round leaves out the classes some part of whose writing failed.
        for _ in 0..ROUNDS {
            if keep.is_empty() {
                break;
            }
            let (hidden, failed) = found.settle(&keep, cx);
            if failed.is_empty() {
                return hidden;
            }
            keep.retain(|key| !failed.contains(key));
        }
        Hidden::default()
    }
}

/// Whether the project holds `program` and no program of it imports `program`: what another program names of it, it names through an import.
pub(crate) fn unimported(program: u32, project: &Project) -> bool {
    let mut others = project.programs.iter().filter(|(of, _)| **of != program);
    let named = others.any(|(_, info)| info.imports.iter().any(|i| i.program == program));
    project.programs.contains_key(&program) && !named
}

/// Whether `class` is a closure class whose every member is a lambda `lambdas` writes in place.
fn candidate(class: &Class, lambdas: &Lambdas<'_>, project: &Project) -> bool {
    let lambda = |m: &Method| {
        m.role == Role::Lambda && !m.is_async && lambdas.contains_key(&m.function.head.at)
    };
    let slots = (project.types.get(&class.key)).is_none_or(|facts| facts.slots.is_empty());
    class.kind == ClassKind::Closure
        && class.nested.is_empty()
        && !class.methods.is_empty()
        && class.methods.iter().all(lambda)
        && class.fields.iter().all(|field| !field.is_static)
        && slots
}

/// What a lambda reads of its object, field by field: the offset, the name of the binder's variable the field was given, and what the binder knows of it.
#[derive(Clone)]
struct Given {
    offset: u32,
    name: String,
    info: VarInfo,
}

/// How a function's code uses its variables.
#[derive(Default)]
struct Facts {
    assigned: BTreeMap<Var, u32>,
    addressed: BTreeSet<Var>,
}

impl Facts {
    fn of(body: &[tree::Stmt]) -> Facts {
        let mut facts = Facts::default();
        tree::each_simple(body, &mut |s| {
            for target in s.targets() {
                *facts.assigned.entry(*target).or_default() += 1;
            }
        });
        tree::each_expr(body, &mut |top| {
            top.walk(&mut |e| {
                if let Expr::AddrOf { var, .. } = e {
                    facts.addressed.insert(*var);
                }
            });
        });
        facts
    }
}

/// A function that makes objects of classes that may be hidden.
struct Binder<'u> {
    method: &'u Method,
    /// Whether it is written as a member of a class, its receiver as `this`.
    member: bool,
    /// Each variable that holds an object of such a class, and the class.
    objects: BTreeMap<Var, TypeKey>,
    /// Every variable that holds a closure object, of any class.
    closures: BTreeSet<Var>,
    /// The lambdas the function binds in place.
    sited: BTreeSet<u32>,
    facts: Facts,
}

impl<'u> Binder<'u> {
    fn of(
        method: &'u Method,
        member: bool,
        keep: &BTreeSet<TypeKey>,
        lambdas: &Lambdas<'_>,
        cx: &Context<'_>,
    ) -> Option<Binder<'u>> {
        let f = &method.function;
        let accessor = matches!(
            method.role,
            Role::Lambda | Role::Getter | Role::Setter | Role::Step
        );
        if accessor || method.is_async || f.head.incomplete.is_some() {
            return None;
        }
        let closures = closures::closures(&f.body);
        if closures.is_empty() {
            return None;
        }
        let mut objects = BTreeMap::new();
        tree::each_simple(&f.body, &mut |s| {
            let Stmt::Assign { target, value } = s else {
                return;
            };
            let Expr::NewObject(ty) = underneath(value) else {
                return;
            };
            let key = TypeKey::of(ty).filter(|key| keep.contains(key));
            if let (true, Some(key)) = (closures.contains(target), key) {
                objects.insert(*target, key);
            }
        });
        if objects.is_empty() {
            return None;
        }
        Some(Binder {
            method,
            member,
            objects,
            closures,
            sited: bound_in_place(f, lambdas, cx),
            facts: Facts::of(&f.body),
        })
    }

    /// The name the binder is written with for `var`.
    fn spoken(&self, var: Var) -> String {
        let head = &self.method.function.head;
        let this = self.member && !self.method.is_static && head.params > 0;
        match this && var == Var::Param(0) {
            true => "this".to_owned(),
            false => label(head, var),
        }
    }

    fn info(&self, var: Var) -> VarInfo {
        let held = self.method.function.head.vars.get(&var).cloned();
        held.unwrap_or_else(|| VarInfo::new(Width::Cell))
    }

    /// Whether `var`, given to an object by a statement that `before` stands before, holds that value for as long as the function runs: one cell, assigned in `before` or never, and reached through no address.
    // ⚠ An address of a parameter reaches every cell from it on, the locals included; one of a
    // local, every local from it on.
    fn steady(&self, var: Var, before: &[tree::Stmt]) -> bool {
        let head = &self.method.function.head;
        let several = |kind: &Kind| {
            matches!(
                kind,
                Kind::Aggregate(_) | Kind::Value { .. } | Kind::Vector | Kind::Handle
            )
        };
        let whole = (head.vars.get(&var))
            .is_none_or(|info| info.width == Width::Cell && !several(&info.kind));
        let times = self.facts.assigned.get(&var).copied().unwrap_or(0);
        let addressed = &self.facts.addressed;
        let set_before = || {
            let mut set = false;
            tree::each_simple(before, &mut |s| set |= s.targets().contains(&var));
            set
        };
        let unchanged = match var {
            Var::Global { .. } => false,
            Var::Param(n) => {
                times == 0
                    && !addressed
                        .iter()
                        .any(|v| matches!(v, Var::Param(k) if *k <= n))
            }
            Var::Local(n) => {
                let reached = addressed
                    .iter()
                    .any(|v| matches!(v, Var::Param(_)) || matches!(v, Var::Local(k) if *k <= n));
                times == 1 && !reached && set_before()
            }
            Var::Value { .. } | Var::Stack { .. } => {
                times == 1 && !addressed.contains(&var) && set_before()
            }
        };
        whole && unchanged && !self.closures.contains(&var)
    }
}

/// What one allocation of a closure object came to, read away.
struct Captured {
    /// The binder's variable each field was given, by offset.
    held: BTreeMap<u32, Var>,
    /// The lambdas bound to the object.
    bound: BTreeSet<u32>,
}

/// What [`Hidden::of`] reads of a unit once.
struct Found<'f, 'u> {
    methods: &'f [(&'u Method, bool)],
    classes: &'f [&'u Class],
    binders: &'f [Binder<'u>],
    /// The lambdas of each class that may be hidden.
    owned: &'f BTreeMap<TypeKey, BTreeSet<u32>>,
    program: u32,
}

impl Found<'_, '_> {
    /// How the unit is written with the classes of `keep` hidden, and those of them some part of whose writing fails.
    fn settle(&self, keep: &BTreeSet<TypeKey>, cx: &Context<'_>) -> (Hidden, BTreeSet<TypeKey>) {
        let mut hidden = Hidden::default();
        let mut failed: BTreeSet<TypeKey> = BTreeSet::new();
        let mut given: BTreeMap<u32, Vec<Given>> = BTreeMap::new();
        for binder in self.binders {
            let objects: Vec<(Var, TypeKey)> = (binder.objects.iter())
                .filter(|(_, key)| keep.contains(*key))
                .map(|(object, key)| (*object, *key))
                .collect();
            if objects.is_empty() {
                continue;
            }
            // ⚠ A round in which one fails is thrown away whole: what it changed of the copy
            // is never written.
            let mut written = binder.method.clone();
            for (object, key) in objects {
                let own = self.owned.get(&key);
                let allowed: BTreeSet<u32> = (binder.sited.iter().copied())
                    .filter(|at| own.is_some_and(|all| all.contains(at)))
                    .collect();
                let Some(captured) = dissolve(&mut written.function, object, key, &allowed, binder)
                else {
                    failed.insert(key);
                    continue;
                };
                let read: Vec<Given> = (captured.held.iter())
                    .map(|(offset, var)| Given {
                        offset: *offset,
                        name: binder.spoken(*var),
                        info: binder.info(*var),
                    })
                    .collect();
                for at in captured.bound {
                    if given.insert(at, read.clone()).is_some() {
                        failed.insert(key);
                    }
                }
            }
            let at = binder.method.function.head.at;
            hidden.binders.insert(at, (written, binder.sited.clone()));
        }
        for class in self.classes {
            if !keep.contains(&class.key) || failed.contains(&class.key) {
                continue;
            }
            for method in &class.methods {
                let at = method.function.head.at;
                let read = given.get(&at);
                match read.and_then(|read| read_in_place(&method.function, read)) {
                    Some(body) => {
                        hidden.bodies.insert(at, body);
                    }
                    None => {
                        failed.insert(class.key);
                    }
                }
            }
        }
        let named = self.named(keep, &hidden, cx);
        failed.extend(keep.intersection(&named).copied());
        hidden.classes = keep.difference(&failed).copied().collect();
        (hidden, failed)
    }

    /// Every type what is written of the unit names, with the classes of `keep` and their objects read away as `hidden` writes them.
    fn named(
        &self,
        keep: &BTreeSet<TypeKey>,
        hidden: &Hidden,
        cx: &Context<'_>,
    ) -> BTreeSet<TypeKey> {
        let mut found = BTreeSet::new();
        for (method, member) in self.methods {
            let at = method.function.head.at;
            let lambda = method.role == Role::Lambda;
            let f = match (lambda, hidden.bodies.get(&at), hidden.binders.get(&at)) {
                (true, Some(body), _) => body,
                (false, _, Some((written, _))) => &written.function,
                _ => &method.function,
            };
            // The receiver is declared nowhere: a member's is `this`, a lambda's the object.
            let receiver = lambda || (*member && !method.is_static);
            types_of(f, receiver, &mut found);
        }
        for class in self
            .classes
            .iter()
            .filter(|class| !keep.contains(&class.key))
        {
            found.extend(class.base);
            found.extend(class.interfaces.iter().copied());
            for field in &class.fields {
                kind_types(&field.kind, &mut found);
            }
        }
        if let Some(info) = cx.project.programs.get(&self.program) {
            for ty in info.types.values().filter(|ty| !keep.contains(*ty)) {
                for field in cx.project.fields.get(ty).into_iter().flatten() {
                    kind_types(&field.kind, &mut found);
                }
            }
        }
        found
    }
}

/// Notes the types a function names: what it makes, the fields it reads and stores, the slots it calls, the types of the variables it mentions and of its parameters but a receiver.
pub(crate) fn types_of(f: &tree::Function, receiver: bool, found: &mut BTreeSet<TypeKey>) {
    let mut vars: BTreeSet<Var> = (u32::from(receiver)..f.head.params)
        .map(Var::Param)
        .collect();
    tree::each_simple(&f.body, &mut |s| vars.extend(s.targets().iter().copied()));
    tree::each_expr(&f.body, &mut |top| {
        top.walk(&mut |e| match e {
            Expr::Var(var) | Expr::AddrOf { var, .. } => {
                vars.insert(*var);
            }
            Expr::NewObject(ty)
            | Expr::NewArray { ty, .. }
            | Expr::VirtualFunctionRef { ty, .. } => found.extend(TypeKey::of(ty)),
            Expr::Sugar {
                kind: ExprSugar::Field { owner },
                ..
            } => {
                found.insert(*owner);
            }
            Expr::Call {
                callee: Callee::Virtual { ty, .. },
                ..
            } => found.extend(TypeKey::of_name(ty)),
            Expr::Call {
                callee: Callee::Intrinsic { ty: Some(ty), .. },
                ..
            } => found.extend(TypeKey::of(ty)),
            Expr::Const(c) => kind_types(&c.kind, found),
            Expr::Binary { at, .. } => kind_types(at, found),
            _ => {}
        });
    });
    tree::each_sugar(&f.body, &mut |sugar| match sugar {
        tree::Sugar::Group {
            kind: tree::Group::Field { owner },
            ..
        } => {
            found.insert(*owner);
        }
        tree::Sugar::Try { catches, .. } => found.extend(catches.iter().filter_map(|c| c.ty)),
        _ => {}
    });
    for var in &vars {
        if let Some(info) = f.head.vars.get(var) {
            kind_types(&info.kind, found);
        }
    }
}

/// Notes the type a kind names, through its arrays.
pub(crate) fn kind_types(kind: &Kind, found: &mut BTreeSet<TypeKey>) {
    let mut kind = kind;
    for _ in 0..NESTING {
        match kind {
            Kind::Object(name) | Kind::Value { ty: name, .. } => {
                found.extend(TypeKey::of_name(name));
                return;
            }
            Kind::Array(of) => kind = &**of,
            _ => return,
        }
    }
}

/// Whether a body reads, assigns or takes the address of `var`.
fn mentioned(body: &[tree::Stmt], var: Var) -> bool {
    let mut found = false;
    tree::each_simple(body, &mut |s| found |= s.targets().contains(&var));
    tree::each_expr(body, &mut |top| {
        top.walk(&mut |e| {
            found |= matches!(e, Expr::Var(v) | Expr::AddrOf { var: v, .. } if *v == var);
        });
    });
    found
}

/// The field stores a statement that makes the object `object` of the class `key` holds besides the making: none for the assignment alone.
fn allocation(stmt: &tree::Stmt, object: Var, key: TypeKey) -> Option<&[Stmt]> {
    let made = |s: &Stmt| match s {
        Stmt::Assign { target, value } if *target == object => {
            matches!(underneath(value), Expr::NewObject(ty) if TypeKey::of(ty) == Some(key))
        }
        _ => false,
    };
    match stmt {
        tree::Stmt::Simple(s) if made(s) => Some(Default::default()),
        tree::Stmt::Sugar(sugar) => match &**sugar {
            tree::Sugar::Group {
                kind: tree::Group::Construct,
                parts,
            } => match parts.split_first() {
                Some((first, rest)) if made(first) => Some(rest),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// The offset and the variable of a store of a variable's whole cell to a field of `object`.
fn capture(s: &Stmt, object: Var) -> Option<(u32, Var)> {
    let Stmt::Store {
        kind: StoreKind::U64 | StoreKind::Ref,
        addr,
        offset,
        value,
    } = s
    else {
        return None;
    };
    let (Expr::Var(base), Expr::Var(held)) = (underneath(addr), underneath(value)) else {
        return None;
    };
    (*base == object && *held != object).then_some((*offset, *held))
}

/// [`capture`] of a statement of a list: the store alone, or the group a field's store is read as.
fn captured_at(stmt: &tree::Stmt, object: Var) -> Option<(u32, Var)> {
    match stmt {
        tree::Stmt::Simple(s) => capture(s, object),
        tree::Stmt::Sugar(sugar) => match &**sugar {
            tree::Sugar::Group {
                kind: tree::Group::Field { .. },
                parts,
            } => match parts.as_slice() {
                [s] => capture(s, object),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// Reads the closure object `object` of the class `key` away from `f`: its making and the stores of its fields go, each field read as the variable it was given, each delegate of a lambda of `allowed` bound to it bound to nothing; `None` where any of it does not hold, with `f` left part changed.
fn dissolve(
    f: &mut tree::Function,
    object: Var,
    key: TypeKey,
    allowed: &BTreeSet<u32>,
    binder: &Binder<'_>,
) -> Option<Captured> {
    let found = {
        let mut found = None;
        // An explicit stack: a body nests as deep as its listing does.
        let mut work: Vec<&mut Vec<tree::Stmt>> = vec![&mut f.body];
        while let Some(list) = work.pop() {
            let at = list
                .iter()
                .position(|stmt| allocation(stmt, object, key).is_some());
            match at {
                Some(i) => {
                    found = dissolve_in(list, i, object, key, allowed, binder);
                    break;
                }
                None => {
                    for stmt in list {
                        work.extend(tree::nested_mut(stmt));
                    }
                }
            }
        }
        found
    };
    let captured = found?;
    (!mentioned(&f.body, object)).then_some(captured)
}

/// [`dissolve`] where `list[i]` makes the object.
// Every statement after the last store of a field is reached only through the making and the
// stores, as the list holds no label from the making on; the object is made outside every loop,
// as a lambda is written in place only where it is. Between the making and the last store
// nothing else mentions the object, so no field is read before it is given its variable.
fn dissolve_in(
    list: &mut Vec<tree::Stmt>,
    i: usize,
    object: Var,
    key: TypeKey,
    allowed: &BTreeSet<u32>,
    binder: &Binder<'_>,
) -> Option<Captured> {
    let mut held: BTreeMap<u32, Var> = BTreeMap::new();
    // Each variable given and where the statement that gives it stands.
    let mut given: Vec<(Var, usize)> = Vec::new();
    for s in allocation(list.get(i)?, object, key)? {
        let (offset, var) = capture(s, object)?;
        if held.insert(offset, var).is_some() {
            return None;
        }
        given.push((var, i));
    }
    let after = i.checked_add(1)?;
    let last = (after..list.len())
        .rev()
        .find(|j| captured_at(&list[*j], object).is_some())
        .unwrap_or(i);
    let mut stores = Vec::new();
    for j in after..=last {
        match captured_at(&list[j], object) {
            Some((offset, var)) => {
                if held.insert(offset, var).is_some() {
                    return None;
                }
                given.push((var, j));
                stores.push(j);
            }
            None if mentioned(&list[j..=j], object) => return None,
            None => {}
        }
    }
    if tree::counts(&list[i..]).labels != 0 {
        return None;
    }
    if !given
        .iter()
        .all(|(var, at)| binder.steady(*var, &list[..*at]))
    {
        return None;
    }
    let program = binder.method.function.head.program;
    let values: BTreeSet<Var> = held.values().copied().collect();
    let mut bound = BTreeSet::new();
    {
        // An explicit stack: a body nests as deep as its listing does.
        let mut work: Vec<&mut tree::Stmt> = list[last + 1..].iter_mut().collect();
        while let Some(stmt) = work.pop() {
            if let Some(at) = unbound(stmt, object, allowed, program) {
                bound.insert(at);
            }
            tree::own_exprs_mut(stmt, &mut |top| read_fields(top, object, &held, &values));
            for nested in tree::nested_mut(stmt) {
                work.extend(nested);
            }
        }
    }
    for j in stores.iter().rev() {
        list.remove(*j);
    }
    list.remove(i);
    Some(Captured { held, bound })
}

/// Binds the delegate `stmt` makes to nothing where it is bound to `object` and to a lambda of `allowed`, and says which lambda.
// The lambda is written in place, so what it is bound to is written nowhere.
fn unbound(
    stmt: &mut tree::Stmt,
    object: Var,
    allowed: &BTreeSet<u32>,
    program: u32,
) -> Option<u32> {
    let tree::Stmt::Sugar(sugar) = stmt else {
        return None;
    };
    let tree::Sugar::Group {
        kind: tree::Group::Delegate,
        parts,
    } = &mut **sugar
    else {
        return None;
    };
    let [Stmt::Assign { target, value }, Stmt::Eval(call)] = parts.as_mut_slice() else {
        return None;
    };
    if !matches!(underneath(value), Expr::NewObject(_)) {
        return None;
    }
    let delegate = *target;
    let mut call = call;
    while let Expr::Sugar { e: inner, .. } = call {
        call = &mut **inner;
    }
    let Expr::Call { args, .. } = call else {
        return None;
    };
    let [made, held, Expr::Sugar {
        kind: ExprSugar::Lambda,
        e: function,
    }] = args.as_mut_slice()
    else {
        return None;
    };
    let Expr::FunctionRef { program: of, at } = &**function else {
        return None;
    };
    let at = *at;
    let ours = *of == program
        && allowed.contains(&at)
        && *underneath(made) == Expr::Var(delegate)
        && *underneath(held) == Expr::Var(object);
    if !ours {
        return None;
    }
    *held = Expr::Const(Const::i32(0));
    Some(at)
}

/// Reads each whole cell of a field of `object` that `held` gives a variable as that variable.
// A whole cell stored and a whole cell read are the same bits; any other width is left, and so
// is the object it reads.
fn read_fields(top: &mut Expr, object: Var, held: &BTreeMap<u32, Var>, values: &BTreeSet<Var>) {
    top.walk_mut(&mut |e| {
        let read = match &*e {
            Expr::Load {
                ty: Scalar::U64 | Scalar::I64,
                addr,
                offset,
            } if *underneath(addr) == Expr::Var(object) => held.get(offset).copied(),
            _ => None,
        };
        if let Some(var) = read {
            *e = Expr::Var(var);
        }
    });
    // A field's reading around what is now a variable.
    top.walk_mut(&mut |e| {
        let alone = match &*e {
            Expr::Sugar {
                kind: ExprSugar::Field { .. },
                e: inner,
            } => match &**inner {
                Expr::Var(var) if values.contains(var) => Some(*var),
                _ => None,
            },
            _ => None,
        };
        if let Some(var) = alone {
            *e = Expr::Var(var);
        }
    });
}

/// `body` as it is written where it is bound once its object is read away: each field `given` names read as a variable of that name, a parameter past the body's own; `None` where the body reads its object otherwise, or one of its own variables goes by such a name.
fn read_in_place(body: &tree::Function, given: &[Given]) -> Option<tree::Function> {
    const RECEIVER: Var = Var::Param(0);
    let mut f = body.clone();
    let mut held: BTreeMap<u32, Var> = BTreeMap::new();
    let mut names: BTreeSet<&str> = BTreeSet::new();
    for (n, one) in given.iter().enumerate() {
        let var = Var::Param(f.head.params.checked_add(u32::try_from(n).ok()?)?);
        let taken = f.head.vars.contains_key(&var) || mentioned(&f.body, var);
        if taken || !names.insert(one.name.as_str()) {
            return None;
        }
        held.insert(one.offset, var);
        let info = VarInfo {
            name: Some(one.name.clone()),
            ..one.info.clone()
        };
        f.head.vars.insert(var, info);
    }
    let values: BTreeSet<Var> = held.values().copied().collect();
    {
        // An explicit stack: a body nests as deep as its listing does.
        let mut work: Vec<&mut Vec<tree::Stmt>> = vec![&mut f.body];
        while let Some(list) = work.pop() {
            for stmt in list {
                tree::own_exprs_mut(stmt, &mut |top| read_fields(top, RECEIVER, &held, &values));
                work.extend(tree::nested_mut(stmt));
            }
        }
    }
    if mentioned(&f.body, RECEIVER) {
        return None;
    }
    // ⚠ A variable of the body's own that goes by the name of one it now reads would hide it.
    let mut own: BTreeSet<Var> = (1..f.head.params).map(Var::Param).collect();
    tree::each_simple(&f.body, &mut |s| own.extend(s.targets().iter().copied()));
    tree::each_expr(&f.body, &mut |top| {
        top.walk(&mut |e| {
            if let Expr::Var(var) | Expr::AddrOf { var, .. } = e {
                own.insert(*var);
            }
        });
    });
    let clash = (own.iter())
        .filter(|var| !values.contains(*var) && !matches!(**var, Var::Global { .. }))
        .any(|var| names.contains(label(&f.head, *var).as_str()));
    if clash || closures::inline(&f).is_none() {
        return None;
    }
    Some(f)
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
                        work.extend(written_bodies(sugar));
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

/// The bodies of a construct that every form of it writes as statements: written as itself where its pass is on, as the statements it stands for where it is not.
// ⚠ None of what is left out where a construct is written as itself: an `await`'s suspension,
// the arms of a kept delegate, the exit of a `using`, and what is hidden; nor the handlers of a
// `try`, which the statements it stands for drop.
fn written_bodies(sugar: &tree::Sugar) -> Vec<&[tree::Stmt]> {
    match sugar {
        tree::Sugar::Foreach { body, after, .. } => vec![body.as_slice(), after.as_slice()],
        tree::Sugar::Guarded { body, .. } => vec![body.as_slice()],
        tree::Sugar::Try { body, finally, .. } => vec![body.as_slice(), finally.as_slice()],
        tree::Sugar::SwitchOn { arms, default } => {
            let mut all: Vec<&[tree::Stmt]> = arms.iter().map(|arm| arm.body.as_slice()).collect();
            all.push(default.as_slice());
            all
        }
        _ => Vec::new(),
    }
}

/// Whether evaluating `e` calls or makes anything.
fn effects(e: &Expr) -> bool {
    let mut found = false;
    e.walk(&mut |node| {
        found |= matches!(
            node,
            Expr::Call { .. } | Expr::NewObject(_) | Expr::NewArray { .. }
        );
    });
    found
}

impl Writer<'_> {
    /// The method group a delegate made of `function` and bound to `bound` reads as: `x.Name` for a slot of what it is bound to; a function of another program alone where it is bound to nothing.
    // `None` for a function of this program, which the delegate's own form writes, and for
    // anything else. ⚠ The code reads a slot's receiver once for the target and once for the
    // slot, and the group once: the two must be one value, read with nothing called.
    pub(super) fn method_group(&mut self, bound: &Expr, function: &Expr) -> Option<String> {
        if !(self.sharp && self.options.on(Pass::Delegates)) {
            return None;
        }
        match underneath(function) {
            Expr::VirtualFunctionRef { receiver, ty, slot } => {
                let held = underneath(bound);
                if held != underneath(receiver) || is_zero(held) || effects(held) {
                    return None;
                }
                let named = TypeKey::of(ty).and_then(|key| self.method_name(key, *slot));
                let name = named.map_or_else(|| format!("Method_{slot}"), str::to_owned);
                Some(format!("{}.{name}", self.expr(bits(bound), POSTFIX)))
            }
            // ⚠ Another program's function bound to an object is no member of it: `x.func_X`
            // would read as this program's function of that offset, so the delegate stays a `new`.
            Expr::FunctionRef { program, at } if *program != self.head.program => {
                is_zero(bound).then(|| self.function_ref(*program, *at))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "print_lambda_tests.rs"]
mod tests;
