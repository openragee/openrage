//! Classes: which function belongs to which type and as what, and the calls, constructions and
//! accessors that read as members once that is known.
//!
//! Three parts. `of_function`, `absorb` and `settle` gather membership into the project during
//! the first pass over an install. `run` rewrites one function's body on the structured form.
//! `unit` groups a program's finished functions under their types for the printer.
//!
//! ⚠ A function is a member by one of three proofs and by nothing else: one type's virtual table
//! is the first to hold it; every mention of it in the install runs it on an object of one type
//! just made, before anything else uses the object; or every mention binds it as the delegate of
//! a closure object of one compiler-made type. An object of a type passed first proves no member:
//! a static function takes one the same way.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use crate::gta6::script::cfg;
use crate::gta6::script::class::{Class, ClassKind, Field, Method, Role, Unit};
use crate::gta6::script::ir::{
    self, Callee, Expr, ExprSugar, FunctionKey, Head, Kind, Returns, Scalar, TypeKey, Var, VmType,
};
use crate::gta6::script::listing::Import;
use crate::gta6::script::project::{FieldFacts, Member, Project, TypeFacts};
use crate::gta6::script::tree::{self, Group, Stmt, Sugar};
use crate::gta6::script::{Context, Pass, Stats};

use super::sugar_expressions::Reached;

/// The names this file's rewrites are switched by; `Pass::Classes` switches the grouping `unit` does.
pub const PASSES: &[Pass] = &[
    Pass::Receivers,
    Pass::Constructors,
    Pass::Properties,
    Pass::Operators,
];

/// The bit of a type's flags that says the compiler made the type: a lambda's captures, a state machine.
const GENERATED: u32 = 0x10_0000;

/// The bit that says no object of the type is ever made: an interface, or an abstract class.
const ABSTRACT: u32 = 0x2_0000;

/// The bit of a value type's flags that says it is a struct; a value type without it is an enum.
const STRUCT: u32 = 0x04;

/// The links a walk up a chain of bases or of enclosing classes follows before it gives up: a ring must end.
const MAX_CHAIN: usize = 64;

/// The owner of a note that names no type: what the gathering holds of a function that is no membership.
// ⚠ Notes live in `Project::members` between `of_function` and `settle` only, and `settle` leaves
// none; `member` reads past one, so a part nothing settled proves nothing.
const NO_OWNER: TypeKey = TypeKey(0);

/// What the gathering says of a function, before `settle` knows every mention of it.
fn provisional(owner: TypeKey, role: Role) -> Member {
    Member {
        owner,
        role,
        is_static: false,
        slot: None,
    }
}

/// The note that no mention of a function may make it a member: it is called like any function, two types claim it, or it writes its first parameter.
fn disputed() -> Member {
    provisional(NO_OWNER, Role::Method)
}

/// The note that a function returns a value, which a constructor never does.
fn returning() -> Member {
    provisional(NO_OWNER, Role::Getter)
}

/// What two things said of one function say together.
// A least upper bound, so the order the parts of a project arrive in changes nothing.
fn join(a: &Member, b: &Member) -> Member {
    if a == b {
        return a.clone();
    }
    // A value returned rules out a constructor and nothing else.
    let other = match (*a == returning(), *b == returning()) {
        (true, _) => b,
        (_, true) => a,
        _ => return disputed(),
    };
    match other.owner != NO_OWNER && other.role != Role::Constructor {
        true => other.clone(),
        false => disputed(),
    }
}

fn note(project: &mut Project, function: FunctionKey, said: Member) {
    let joined = match project.members.get(&function) {
        Some(held) => join(held, &said),
        None => said,
    };
    project.members.insert(function, joined);
}

/// What the project proves of a function; a note the gathering left proves nothing.
fn member(project: &Project, function: FunctionKey) -> Option<&Member> {
    let found = project.members.get(&function);
    found.filter(|member| member.owner != NO_OWNER)
}

/// The function of a script a call reaches by its own offset, `None` for any other callee.
fn callee_key(program: u32, callee: &Callee) -> Option<FunctionKey> {
    match callee {
        Callee::Local { at, .. } => Some(FunctionKey::new(program, *at)),
        Callee::Import { program, at, .. } => Some(FunctionKey::new(*program, *at)),
        _ => None,
    }
}

/// The script function `e` itself calls or takes a reference to.
fn referred(program: u32, e: &Expr) -> Option<FunctionKey> {
    match e {
        Expr::Call { callee, .. } => callee_key(program, callee),
        Expr::FunctionRef { program, at } => Some(FunctionKey::new(*program, *at)),
        _ => None,
    }
}

/// Whether `e` reads `var` or takes its address.
fn mentions(e: &Expr, var: Var) -> bool {
    let mut found = false;
    e.walk(&mut |e| {
        found |= matches!(e, Expr::Var(v) | Expr::AddrOf { var: v, .. } if *v == var);
    });
    found
}

/// `e` under the casts that keep every bit of a cell.
fn whole(e: &Expr) -> &Expr {
    let mut e = e;
    while let Expr::Cast {
        to: Scalar::U64 | Scalar::I64,
        e: inner,
    } = e
    {
        e = &**inner;
    }
    e
}

/// Marks every variable `top` takes the address of as one that may hold anything.
fn addressed(top: &Expr, assigned: &mut BTreeMap<Var, Option<&VmType>>) {
    top.walk(&mut |e| {
        if let Expr::AddrOf { var, .. } = e {
            assigned.insert(*var, None);
        }
    });
}

/// Every variable the function assigns or takes the address of, with the type of the object it holds where every assignment makes one of that type.
fn assigned(f: &cfg::Function) -> BTreeMap<Var, Option<&VmType>> {
    let mut assigned: BTreeMap<Var, Option<&VmType>> = BTreeMap::new();
    for block in &f.cfg.blocks {
        for stmt in &block.stmts {
            let fresh = match stmt {
                ir::Stmt::Assign {
                    value: Expr::NewObject(ty),
                    ..
                } => Some(ty),
                _ => None,
            };
            for target in stmt.targets() {
                let held = assigned.entry(*target).or_insert(fresh);
                if *held != fresh {
                    *held = None;
                }
            }
            stmt.each_expr(&mut |top| addressed(top, &mut assigned));
        }
        if let Some(top) = block.term.expr() {
            addressed(top, &mut assigned);
        }
    }
    assigned
}

/// Whether a statement assigns `var`, reads it or takes its address.
fn uses(stmt: &ir::Stmt, var: Var) -> bool {
    let mut found = stmt.targets().contains(&var);
    stmt.each_expr(&mut |top| found |= mentions(top, var));
    found
}

/// The function a statement proves a member, with what it proves and the expression that is the proof.
// `fresh` is the object the block last made and the type made, while nothing has used it since.
fn site<'a>(
    program: u32,
    assigned: &BTreeMap<Var, Option<&VmType>>,
    fresh: Option<(Var, &VmType)>,
    stmt: &'a ir::Stmt,
) -> Option<(FunctionKey, Member, &'a Expr)> {
    let ir::Stmt::Eval(call @ Expr::Call { callee, args, .. }) = stmt else {
        return None;
    };
    match (callee, args.as_slice()) {
        // A delegate bound to a closure object this function makes: the function bound is the closure's own.
        (
            Callee::Builtin { path, .. },
            [_, target, bound @ Expr::FunctionRef { program: home, at }],
        ) if path.ends_with(".Ctor") => {
            let Expr::Var(target) = whole(target) else {
                return None;
            };
            let made = assigned.get(target).copied().flatten();
            let closure = made.filter(|ty| ty.flags & GENERATED != 0)?;
            let lambda = provisional(TypeKey::of(closure)?, Role::Lambda);
            Some((FunctionKey::new(*home, *at), lambda, bound))
        }
        // The first thing run on an object just made, or on one made in the call itself: its constructor.
        (Callee::Local { .. } | Callee::Import { .. }, [first, passed @ ..]) => {
            let ty = match (first, fresh) {
                (Expr::NewObject(ty), _) => ty,
                (Expr::Var(object), Some((made, ty)))
                    if *object == made && !passed.iter().any(|arg| mentions(arg, made)) =>
                {
                    ty
                }
                _ => return None,
            };
            let constructor = provisional(TypeKey::of(ty)?, Role::Constructor);
            Some((callee_key(program, callee)?, constructor, call))
        }
        _ => None,
    }
}

/// Notes every script function `top` calls or refers to as one mentioned like any function, but for the mention at `proof`.
fn others(top: &Expr, program: u32, proof: Option<&Expr>, found: &mut Vec<(FunctionKey, Member)>) {
    top.walk(&mut |e| {
        if proof.is_some_and(|proof| std::ptr::eq(proof, e)) {
            return;
        }
        if let Some(function) = referred(program, e) {
            found.push((function, disputed()));
        }
    });
}

/// Adds what one function's code proves of membership: a constructor run on an object just made, a lambda bound to its closure object.
pub fn of_function(project: &mut Project, f: &cfg::Function) {
    let program = f.head.program;
    let assigned = assigned(f);
    let mut found: Vec<(FunctionKey, Member)> = Vec::new();
    for block in &f.cfg.blocks {
        let mut fresh: Option<(Var, &VmType)> = None;
        for stmt in &block.stmts {
            let proved = site(program, &assigned, fresh, stmt);
            let proof = proved.as_ref().map(|(_, _, proof)| *proof);
            found.extend(proved.map(|(function, said, _)| (function, said)));
            stmt.each_expr(&mut |top| others(top, program, proof, &mut found));
            fresh = match stmt {
                ir::Stmt::Assign {
                    target,
                    value: Expr::NewObject(ty),
                } => Some((*target, ty)),
                _ => fresh.filter(|(object, _)| !uses(stmt, *object)),
            };
        }
        if let Some(top) = block.term.expr() {
            others(top, program, None, &mut found);
        }
    }
    // What the function's own code says of it: `this` is never written, and a constructor returns nothing.
    let own = FunctionKey::new(program, f.head.at);
    if assigned.contains_key(&Var::Param(0)) {
        found.push((own, disputed()));
    } else if f.head.returns != Returns::Void {
        found.push((own, returning()));
    }
    for (function, said) in found {
        note(project, function, said);
    }
}

/// Moves the members of `from` into `into`; a function both hold keeps what the two say together.
pub fn absorb(into: &mut Project, from: &mut Project) {
    for (function, said) in std::mem::take(&mut from.members) {
        note(into, function, said);
    }
}

/// Whether a type takes a function of its table from an ancestor that holds it too.
fn inherits(project: &Project, ty: TypeKey, holders: &BTreeMap<TypeKey, u32>) -> bool {
    let mut ty = ty;
    for _ in 0..MAX_CHAIN {
        match project.types.get(&ty).and_then(|facts| facts.base) {
            Some(base) if holders.contains_key(&base) => return true,
            Some(base) => ty = base,
            None => return false,
        }
    }
    false
}

/// Whether every ancestor of a type is known, so that none can hold a function unseen.
fn rooted(project: &Project, ty: TypeKey) -> bool {
    let mut ty = ty;
    for _ in 0..MAX_CHAIN {
        match project.types.get(&ty) {
            Some(facts) => match facts.base {
                Some(base) => ty = base,
                None => return true,
            },
            // A builtin type holds no function of a script.
            None => return ty.builtin().is_some(),
        }
    }
    false
}

/// Once every part is in: adds the functions the types' virtual slots name, and drops a function two types claim.
pub fn settle(project: &mut Project) {
    let gathered = std::mem::take(&mut project.members);
    // The types whose table holds each function, each with the lowest slot it holds it in.
    let mut holders: BTreeMap<FunctionKey, BTreeMap<TypeKey, u32>> = BTreeMap::new();
    for (ty, facts) in &project.types {
        for (slot, function) in &facts.slots {
            let types = holders.entry(*function).or_default();
            types.entry(*ty).or_insert(*slot);
        }
    }
    let mut members = BTreeMap::new();
    for (function, types) in &holders {
        // The one type that holds the function and inherits it from no ancestor declares it.
        let mut first = types
            .iter()
            .filter(|(ty, _)| !inherits(project, **ty, types));
        if let (Some((owner, slot)), None) = (first.next(), first.next()) {
            if rooted(project, *owner) {
                let method = Member {
                    slot: Some(*slot),
                    ..provisional(*owner, Role::Method)
                };
                members.insert(*function, method);
            }
        }
    }
    for (function, said) in gathered {
        // A function of a virtual table is no constructor and no lambda, whichever type holds it.
        if said.owner != NO_OWNER && !holders.contains_key(&function) {
            members.insert(function, said);
        }
    }
    project.members = members;
}

/// Whether `call` runs a constructor of the type `made` makes an object of, on that object and with nothing else of it.
// ⚠ `x = new T(a)` assigns after the constructor has run, and the two statements before: the
// constructor must have no way to `x` but the object it is handed, so `x` is no global and no
// variable a pointer reaches.
fn constructs(
    made: &ir::Stmt,
    call: &ir::Stmt,
    program: u32,
    project: &Project,
    reached: &Reached,
) -> bool {
    let (
        ir::Stmt::Assign {
            target,
            value: Expr::NewObject(ty),
        },
        ir::Stmt::Eval(Expr::Call { callee, args, .. }),
    ) = (made, call)
    else {
        return false;
    };
    let Some((Expr::Var(receiver), passed)) = args.split_first() else {
        return false;
    };
    let found = callee_key(program, callee).and_then(|key| member(project, key));
    let owned =
        found.is_some_and(|m| m.role == Role::Constructor && Some(m.owner) == TypeKey::of(ty));
    let unseen = !reached.has(*target);
    owned && unseen && receiver == target && !passed.iter().any(|arg| mentions(arg, *target))
}

/// Groups each object made and the constructor the next statement runs on it into the one `new` they were.
fn construct(list: &mut Vec<Stmt>, program: u32, project: &Project, reached: &Reached) {
    let mut out: Vec<Stmt> = Vec::with_capacity(list.len());
    for stmt in list.drain(..) {
        match (out.pop(), stmt) {
            (Some(Stmt::Simple(made)), Stmt::Simple(call))
                if constructs(&made, &call, program, project, reached) =>
            {
                let parts = vec![made, call];
                let kind = Group::Construct;
                out.push(Stmt::Sugar(Box::new(Sugar::Group { kind, parts })));
            }
            (before, stmt) => {
                out.extend(before);
                out.push(stmt);
            }
        }
    }
    *list = out;
}

/// Whether a call's first argument is the object its callee is a method of, and the callee's own name says which method.
fn on_receiver(callee: &Callee, args: &[Expr], head: &Head, project: &Project) -> bool {
    let Some(receiver) = args.first() else {
        return false;
    };
    match callee {
        // ⚠ A class's slot is one method down its whole chain, so its number names it on any
        // receiver. An interface numbers its own slots from 0: without the interface the number
        // names nothing, and such a call stays as it is unless the receiver's own type is the one named.
        Callee::Virtual { class, ty, .. } => {
            let declared = TypeKey::of_name(ty).and_then(|key| project.types.get(&key));
            let held = match whole(receiver) {
                Expr::Var(var) => head.vars.get(var).map(|info| &info.kind),
                _ => None,
            };
            declared.is_some_and(|facts| facts.base.is_some())
                || matches!(held, Some(Kind::Object(name)) if name == ty || name == class)
        }
        // ⚠ A struct's method takes the address of its value, which reads as no receiver.
        Callee::Local { .. } | Callee::Import { .. } => {
            let found = callee_key(head.program, callee).and_then(|key| member(project, key));
            let owner = found.and_then(|m| project.types.get(&m.owner));
            found.is_some_and(|m| m.role == Role::Method && !m.is_static)
                && owner.is_some_and(|facts| !facts.is_value())
        }
        _ => false,
    }
}

/// Whether a builtin's name says it is the `==` or the `!=` its type declares.
fn is_operator(path: &str) -> bool {
    matches!(
        path.rsplit('.').next(),
        Some("op_Equality" | "op_Inequality")
    )
}

/// Reads one call as the method call or the operator it is proved to be.
fn read(e: &mut Expr, head: &Head, cx: &Context<'_>) {
    let Expr::Call { callee, args, .. } = &*e else {
        return;
    };
    let options = &cx.options;
    let kind = match callee {
        Callee::Builtin { path, .. } => {
            let operator = options.on(Pass::Operators) && args.len() == 2 && is_operator(path);
            operator.then_some(ExprSugar::Operator)
        }
        _ => {
            let method = options.on(Pass::Receivers) && on_receiver(callee, args, head, cx.project);
            method.then_some(ExprSugar::Method)
        }
    };
    if let Some(kind) = kind {
        *e = Expr::sugar(kind, std::mem::replace(e, Expr::GuidEmpty));
    }
}

/// Reads a call on a receiver as `receiver.Method(..)`, an object made and constructed as `new T(..)` and a type's own `==` as the operator.
pub fn run(f: &mut tree::Function, cx: &Context<'_>, _stats: &mut Stats) {
    let options = &cx.options;
    let calls = options.on(Pass::Receivers) || options.on(Pass::Operators);
    let constructors = options.on(Pass::Constructors) && !cx.project.members.is_empty();
    if !calls && !constructors {
        return;
    }
    let tree::Function { head, body } = f;
    let head: &Head = head;
    let reached = match constructors {
        true => Reached::of(body),
        false => Reached::default(),
    };
    // An explicit stack: a body nests as deep as its listing does.
    let mut work: Vec<&mut Vec<Stmt>> = vec![body];
    while let Some(list) = work.pop() {
        if constructors {
            construct(list, head.program, cx.project, &reached);
        }
        for stmt in list {
            if calls {
                tree::own_exprs_mut(stmt, &mut |top| top.walk_mut(&mut |e| read(e, head, cx)));
            }
            work.extend(tree::nested_mut(stmt));
        }
    }
}

/// What a type is in the source, by the flags its program states.
fn kind(facts: Option<&TypeFacts>) -> ClassKind {
    let Some(facts) = facts else {
        return ClassKind::Class;
    };
    let has = |bit: u32| facts.flags & bit != 0;
    match facts.is_value() {
        true if has(GENERATED) => ClassKind::StateMachine,
        true if has(STRUCT) => ClassKind::Struct,
        true => ClassKind::Enum,
        false if facts.base.is_none() && has(ABSTRACT) => ClassKind::Interface,
        false => ClassKind::Class,
    }
}

/// A type as the project knows it, with no method yet.
fn class_of(project: &Project, key: TypeKey) -> Class {
    let facts = project.types.get(&key);
    let fields = project.fields.get(&key).map(Vec::as_slice);
    let field = |held: &FieldFacts| Field {
        offset: held.offset,
        bytes: held.bytes,
        kind: held.kind.clone(),
        is_static: false,
    };
    Class {
        key,
        kind: kind(facts),
        base: facts.and_then(|facts| facts.base),
        interfaces: facts
            .map(|facts| facts.interfaces.clone())
            .unwrap_or_default(),
        fields: fields.unwrap_or_default().iter().map(field).collect(),
        methods: Vec::new(),
        nested: Vec::new(),
    }
}

/// Moves each compiler-made class into the class whose methods alone make its objects.
fn nest(
    classes: &mut BTreeMap<TypeKey, Class>,
    makers: &BTreeMap<TypeKey, BTreeSet<Option<TypeKey>>>,
) {
    let mut around: BTreeMap<TypeKey, TypeKey> = BTreeMap::new();
    for (made, by) in makers {
        let mut who = by.iter();
        if let (Some(Some(by)), None) = (who.next(), who.next()) {
            if by != made && classes.contains_key(made) && classes.contains_key(by) {
                around.insert(*made, *by);
            }
        }
    }
    // The classes around a class; `None` where they close a ring, which nests nowhere.
    let depth = |key: TypeKey| {
        let mut key = key;
        for steps in 0..MAX_CHAIN {
            match around.get(&key) {
                Some(outer) => key = *outer,
                None => return Some(steps),
            }
        }
        None
    };
    let mut inner: Vec<(usize, TypeKey, TypeKey)> = around
        .iter()
        .filter_map(|(made, by)| Some((depth(*made)?, *made, *by)))
        .collect();
    // Deepest first, so that a class is whole when it goes into the one around it.
    inner.sort_by_key(|&(depth, made, _)| (Reverse(depth), made));
    for (_, made, by) in inner {
        let Some(class) = classes.remove(&made) else {
            continue;
        };
        match classes.get_mut(&by) {
            Some(outer) => outer.nested.push(class),
            None => {
                classes.insert(made, class);
            }
        }
    }
}

/// Whether a body holds a wait: only the body of an `async` method does, since a wait is made of a proved state machine alone.
fn waits(body: &[Stmt]) -> bool {
    let mut found = false;
    tree::each_sugar(body, &mut |sugar| {
        found |= matches!(sugar, Sugar::Await { .. })
    });
    found
}

/// A program's finished functions under the types the project proves them members of; every other function stays free.
pub fn unit(
    program: u32,
    imports: Vec<Import>,
    functions: Vec<tree::Function>,
    cx: &Context<'_>,
) -> Unit {
    if !cx.options.on(Pass::Classes) {
        let mut flat = Unit::flat(program, imports, functions);
        for method in &mut flat.functions {
            method.is_async = waits(&method.function.body);
        }
        return flat;
    }
    let project = cx.project;
    let mut classes: BTreeMap<TypeKey, Class> = BTreeMap::new();
    // ponytail: every type of the install is looked at for each program; index the types by home if units take long.
    for (key, facts) in &project.types {
        let fields = project.fields.get(key).is_some_and(|held| !held.is_empty());
        if facts.home == program && (fields || !facts.constants.is_empty()) {
            classes.insert(*key, class_of(project, *key));
        }
    }

    // Which classes' methods make the objects of each compiler-made type; `None` is a free function.
    let mut makers: BTreeMap<TypeKey, BTreeSet<Option<TypeKey>>> = BTreeMap::new();
    let mut free = Vec::new();
    for f in functions {
        let found = member(project, FunctionKey::new(f.head.program, f.head.at));
        let owner = found.map(|m| m.owner);
        tree::each_expr(&f.body, &mut |top| {
            top.walk(&mut |e| {
                let made = match e {
                    Expr::NewObject(ty) if ty.flags & GENERATED != 0 => TypeKey::of(ty),
                    _ => None,
                };
                if let Some(made) = made {
                    makers.entry(made).or_default().insert(owner);
                }
            });
        });
        let is_async = waits(&f.body);
        let Some(found) = found else {
            let mut method = Method::free(f);
            method.is_async = is_async;
            // The one function a program names as what readies its statics.
            if method.function.head.name == "initialize" {
                method.role = Role::StaticConstructor;
            }
            free.push(method);
            continue;
        };
        let base = project.types.get(&found.owner).and_then(|t| t.base);
        let filled = |(slot, base): (u32, TypeKey)| project.slot(base, slot).is_some();
        let method = Method {
            function: f,
            role: found.role,
            is_static: found.is_static,
            is_async,
            slot: found.slot,
            overrides: found.slot.zip(base).is_some_and(filled),
        };
        let class = classes
            .entry(found.owner)
            .or_insert_with(|| class_of(project, found.owner));
        class.methods.push(method);
    }

    for class in classes.values_mut() {
        class.methods.sort_by_key(|method| method.function.head.at);
        // ⚠ No method is made the accessor of a property: every call of it still names it as a
        // method, and a property nothing reads as one would be declared under a name no call uses.
        if class.methods.iter().any(|m| m.role == Role::Lambda) {
            class.kind = ClassKind::Closure;
        }
    }
    nest(&mut classes, &makers);
    Unit {
        program,
        imports,
        classes: classes.into_values().collect(),
        functions: free,
        front: None,
    }
}

/// What the class model of one program holds and what `run` read in its functions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Classes, those nested in another included.
    pub classes: usize,
    /// Functions under a class.
    pub methods: usize,
    /// Functions no type claims.
    pub free: usize,
    pub constructors: usize,
    pub lambdas: usize,
    /// Getters and setters.
    pub accessors: usize,
    pub overrides: usize,
    /// Calls read as `receiver.Method(..)`.
    pub method_calls: usize,
    /// Objects made and constructed that read as one `new`.
    pub constructions: usize,
    /// Calls read as an operator.
    pub operators: usize,
}

/// Counts a unit's classes and members, and the readings `run` left in its functions.
pub fn counts(unit: &Unit) -> Counts {
    let mut counts = Counts {
        free: unit.functions.len(),
        ..Counts::default()
    };
    let mut methods: Vec<&Method> = unit.functions.iter().collect();
    let mut open: Vec<&Class> = unit.classes.iter().collect();
    while let Some(class) = open.pop() {
        counts.classes += 1;
        counts.methods += class.methods.len();
        methods.extend(&class.methods);
        open.extend(&class.nested);
    }
    for method in methods {
        match method.role {
            Role::Constructor => counts.constructors += 1,
            Role::Lambda => counts.lambdas += 1,
            Role::Getter | Role::Setter => counts.accessors += 1,
            _ => {}
        }
        counts.overrides += usize::from(method.overrides);
        tree::each_expr(&method.function.body, &mut |top| {
            top.walk(&mut |e| match e {
                Expr::Sugar {
                    kind: ExprSugar::Method,
                    ..
                } => counts.method_calls += 1,
                Expr::Sugar {
                    kind: ExprSugar::Operator,
                    ..
                } => counts.operators += 1,
                _ => {}
            });
        });
        counts.constructions += constructions(&method.function.body);
    }
    counts
}

/// The groups of a body that hold a call of a script's own function: the constructions `run` grouped.
fn constructions(body: &[Stmt]) -> usize {
    let mut found = 0;
    let mut work = vec![body];
    while let Some(list) = work.pop() {
        for stmt in list {
            match stmt {
                Stmt::If { then, els, .. } => work.extend([then.as_slice(), els.as_slice()]),
                Stmt::While { body, .. } | Stmt::DoWhile { body, .. } | Stmt::For { body, .. } => {
                    work.push(body);
                }
                Stmt::Switch { cases, .. } => work.extend(cases.iter().map(|c| c.body.as_slice())),
                Stmt::Sugar(sugar) => {
                    let scripted = |part: &ir::Stmt| {
                        let ir::Stmt::Eval(Expr::Call { callee, .. }) = part else {
                            return false;
                        };
                        matches!(callee, Callee::Local { .. } | Callee::Import { .. })
                    };
                    found += usize::from(matches!(
                        &**sugar,
                        Sugar::Group { kind: Group::Construct, parts } if parts.iter().any(scripted)
                    ));
                    work.extend(sugar.parts().bodies);
                }
                _ => {}
            }
        }
    }
    found
}

#[cfg(test)]
#[path = "classes_tests.rs"]
mod tests;
