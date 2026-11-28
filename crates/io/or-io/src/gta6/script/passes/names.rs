//! Names: a role name for a variable that one fact names, and the hash constants a text may show for.
//!
//! A variable is named from the native it holds the only result of, `playerPed` from
//! `PLAYER_PED_ID`, or from the parameter of a native it is passed as on every use; a label a
//! list of guesses gives a native names no variable, and nor does a native's name a list opens
//! with `_`. Under `role-names` the type of a new object
//! or of a cast, what a getter gets and a named field name the variable
//! that holds them, a variable every assignment of which gives one role takes it, one of a single
//! value takes the parameter's name it is passed as however else it is read, one fetched once
//! by a call or a load takes the name of the script type it is proved to hold, and the index a
//! `for` counts with is `i`, `j` or `k`, which loops side by side share where each declares its
//! own; under `native-params` the pointer parameter a native fills a variable through names it.
//! Natives, types and programs take their names where the printer writes them, from [`Names`].
//!
//! ⚠ A name is a label and changes no code, but a wrong one misleads: a variable assigned twice,
//! one whose address is taken, or one two parameters name differently keeps its slot name.
//!
//! ⚠ Under `hash-text` a constant keeps the kind `Hash`, which the printer shows a text for, only
//! as the whole argument of a native in a cell a supplied table says holds hashes. Anywhere else
//! a text of the vocabulary may be another name's by accident, and the constant stays a number.

use std::collections::{BTreeMap, BTreeSet};

use crate::gta5::script::natives::{NativeName, NativeParam};
use crate::gta6::script::class::Role;
use crate::gta6::script::ir::{
    BinOp, Callee, Const, Expr, ExprSugar, FunctionKey, Head, Kind, Stmt, TypeKey, Var, VarInfo,
    Width, FUNCTION_OFFSET,
};
use crate::gta6::script::names::Names;
use crate::gta6::script::print;
use crate::gta6::script::tree;
use crate::gta6::script::{Context, Pass, Stats};

/// The base types a method's name is looked for on: no hierarchy is that deep, and a ring of bases must end.
const MAX_BASES: usize = 64;

/// The names of the index a `for` counts with, by how many counted loops it is nested in.
const COUNTERS: [&str; 3] = ["i", "j", "k"];

/// The name of the item of a `foreach` where no named field it goes through gives a singular.
const ITEM: &str = "item";

/// Names each variable of `f` that one fact names: the named native it holds the result of, or the named parameter it is always passed as.
pub fn run(f: &mut tree::Function, names: &Names, stats: &mut Stats) {
    run_in(f, &Context::baseline(names), stats);
}

/// [`run`] with the project and the options at hand: `role-names`, `native-params` and `hash-text` each add their part.
// ⚠ `Stats` holds no count of names, so what was done is traced and counted nowhere.
pub fn run_in(f: &mut tree::Function, cx: &Context<'_>, _stats: &mut Stats) {
    let did = apply(f, cx);
    tracing::trace!(function = %f.head.name, roles = did.roles, texts = did.texts, "named");
}

/// What one run did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Did {
    /// Variables given a role name.
    roles: usize,
    /// Hash constants left to show as their text.
    texts: usize,
}

fn apply(f: &mut tree::Function, cx: &Context<'_>) -> Did {
    let mut did = Did::default();
    if f.head.incomplete.is_some() {
        return did;
    }
    if cx.options.on(Pass::HashText) {
        did.texts = hash_text(f, cx.names);
    }
    let Gathered {
        facts,
        mut taken,
        loops,
        items,
    } = gather(&f.body, &f.head, cx);
    taken.insert(f.head.name.clone());
    taken.extend(f.head.vars.values().filter_map(|info| info.name.clone()));
    // A setter's one parameter is written `value`.
    let member = (cx.project.members).get(&FunctionKey::new(f.head.program, f.head.at));
    if member.is_some_and(|member| member.role == Role::Setter) {
        taken.insert("value".to_owned());
    }
    // An instance constructor's first parameter is its receiver, written `this`.
    let constructor = member.filter(|member| member.role == Role::Constructor);
    let receiver = constructor
        .filter(|member| !member.is_static)
        .map(|_| Var::Param(0));
    let exposed: Vec<u32> = facts
        .iter()
        .filter_map(|(var, facts)| match var {
            Var::Local(n) if facts.addresses > 0 => Some(*n),
            _ => None,
        })
        .collect();

    let counters = match cx.options.on(Pass::RoleNames) {
        true => counters(&facts, &loops, &exposed, &f.head),
        false => BTreeMap::new(),
    };
    // Only a letter two indices are given can be shared.
    let letters: BTreeSet<&str> = counters.values().copied().collect();
    let scoped = match letters.len() < counters.len() {
        true => scoped(&loops, &f.body, &f.head),
        false => BTreeSet::new(),
    };
    let mut shared: BTreeMap<&str, String> = BTreeMap::new();
    let more = cx.options.on(Pass::RoleNames);
    let items = match more {
        true => singulars(&items, &facts, &exposed, &f.head),
        false => BTreeMap::new(),
    };

    // In the order of the variables, so that which of two takes the bare name never changes.
    for (var, facts) in &facts {
        // ⚠ The item a `foreach` is given only by the loop, and none other: a value given
        // otherwise, or written through an address, is another variable's.
        let item = items
            .get(var)
            .filter(|_| only_items(*var, facts, &exposed, &f.head));
        let held = (f.head.vars.get(var)).and_then(|info| info.name.as_deref());
        if let Some(held) = held {
            // The item the `foreach` pass made is named only for being one: a singular says more.
            if let (true, Some(Some(singular))) = (made_item(held), item) {
                let name = unique(singular, &mut taken);
                if let Some(info) = f.head.vars.get_mut(var) {
                    info.name = Some(name);
                    did.roles += 1;
                }
            }
            continue;
        }
        let typed = more.then(|| typed_role(*var, &f.head, cx.names)).flatten();
        let made_with = constructor.is_some() && matches!(var, Var::Param(_));
        let made_with = made_with && receiver != Some(*var);
        let role = role(*var, facts, &exposed, &f.head, more, typed, made_with);
        let item_role = item.map(|one| one.clone().unwrap_or_else(|| ITEM.to_owned()));
        let role = role.or(item_role);
        let counter = counters.get(var).copied().filter(|_| role.is_none());
        let Some(role) = role.or_else(|| counter.map(str::to_owned)) else {
            continue;
        };
        let name = match counter.filter(|_| scoped.contains(var)) {
            Some(letter) => (shared.entry(letter))
                .or_insert_with(|| unique(letter, &mut taken))
                .clone(),
            None => unique(&role, &mut taken),
        };
        let info = f.head.vars.entry(*var);
        info.or_insert_with(|| VarInfo::new(Width::Cell)).name = Some(name);
        did.roles += 1;
    }
    did
}

/// Settles which constants keep the kind `Hash`, and so may show as a text; returns how many do with a text to show.
// A constant is visited before the call it is an argument of, so one that has a text loses the
// kind first and the call that vouches for it gives it back.
fn hash_text(f: &mut tree::Function, names: &Names) -> usize {
    if names.hash_slots.is_empty() && names.texts.is_empty() && names.words.is_empty() {
        return 0;
    }
    let head = &f.head;
    let mut shown = 0;
    let mut held: BTreeSet<Var> = BTreeSet::new();
    // An explicit stack: a body nests as deep as its listing does.
    let mut work: Vec<&mut Vec<tree::Stmt>> = vec![&mut f.body];
    while let Some(list) = work.pop() {
        for stmt in list {
            tree::own_exprs_mut(stmt, &mut |top| {
                top.walk_mut(&mut |e| match e {
                    Expr::Const(c)
                        if c.kind == Kind::Hash && names.text(c.bits as u32).is_some() =>
                    {
                        c.kind = Kind::U32;
                    }
                    Expr::Call {
                        callee: Callee::Native { hash, .. },
                        args,
                        ..
                    } => shown += vouched(*hash, args, head, names, &mut held),
                    _ => {}
                });
            });
            work.extend(tree::nested_mut(stmt));
        }
    }
    // A variable a cell that holds hashes is given holds hashes: a switch on it goes by hashes.
    // ⚠ Only one of a cell and of no other kind: a number's kind is what its reads proved.
    for var in held {
        let info = f.head.vars.get_mut(&var);
        let plain = |kind: &Kind| matches!(kind, Kind::Unknown | Kind::I32 | Kind::U32);
        if let Some(info) = info.filter(|info| info.width == Width::Cell && plain(&info.kind)) {
            info.kind = Kind::Hash;
        }
    }
    shown
}

/// Gives the kind `Hash` to each constant that is the whole of an argument cell of the native `hash` which holds hashes, and notes each variable that is; returns how many of the constants have a text.
fn vouched(
    hash: u64,
    args: &mut [Expr],
    head: &Head,
    names: &Names,
    held: &mut BTreeSet<Var>,
) -> usize {
    let (mut cell, mut shown) = (0u32, 0);
    for arg in args {
        // Past an argument of cells nothing counts, no later argument's cell is known.
        let Some(cells) = print::cells_of(arg, head) else {
            break;
        };
        if names.hash_slots.contains(&(hash, cell)) {
            let hashed = constant(arg).is_some_and(|c| vouch(c, names));
            shown += usize::from(hashed);
            if let Some(var) = variable(arg).filter(|_| cells == 1) {
                held.insert(var);
            }
        }
        cell = cell.saturating_add(cells);
    }
    shown
}

/// The variable `e` is, under any reading of it of 32 bits or more.
fn variable(e: &Expr) -> Option<Var> {
    match e {
        Expr::Cast { to, e } if to.bytes() >= 4 && !to.kind().is_float() => variable(e),
        Expr::Var(var) => Some(*var),
        _ => None,
    }
}

/// The constant `e` is, under any reading of it.
// No deeper than `Expr::walk_mut`, which has been down the same readings.
fn constant(e: &mut Expr) -> Option<&mut Const> {
    match e {
        Expr::Cast { e, .. } | Expr::Sugar { e, .. } => constant(e),
        Expr::Const(c) => Some(c),
        _ => None,
    }
}

/// Makes `c`, the whole of a cell that holds hashes, the hash it is; `true` where the vocabulary has its text.
// ⚠ A value below 65,536 or near -1 is an enum's or a flag passed where a hash goes. A float
// literal is its bits only where the vocabulary knows them: the cell takes a float in another
// build's signature.
fn vouch(c: &mut Const, names: &Names) -> bool {
    let (half, low) = (c.bits >> 32, c.bits as u32);
    // An `int` widened with its sign is its low half.
    let narrow = half == 0 || (half == 0xFFFF_FFFF && low >> 31 == 1);
    let text = names.text(low).is_some();
    let integer = c.kind.is_integer() || matches!(c.kind, Kind::Unknown | Kind::Hash);
    let reads = integer || (c.kind == Kind::F32 && text);
    let hash = narrow && (0x1_0000..0xFFFF_0000).contains(&low) && reads;
    if hash {
        c.kind = Kind::Hash;
    }
    hash && text
}

/// What a function's body says of one variable.
#[derive(Default)]
struct Facts {
    /// The assignments that name the variable as a target.
    assigned: u32,
    /// The role of the native an assignment holds the result of.
    result_of: Option<String>,
    /// The assignments that hold the result of a named native, and whether two give different roles.
    results: u32,
    results_apart: bool,
    /// The role of what else an assignment holds: a new object, what a getter gets, a named field.
    holds: Option<String>,
    /// The assignments that hold such a thing, and whether two give different roles.
    holding: u32,
    holds_apart: bool,
    /// Every read, the taking of its address included.
    read: u32,
    /// How often its address is taken.
    addresses: u32,
    /// The name of the pointer parameter a native is handed its address as, to fill.
    filled_as: Option<String>,
    /// The reads that pass it as the whole of a named parameter of a named native.
    passed: u32,
    /// The name of those parameters.
    parameter: Option<String>,
    /// Two of those parameters have different names.
    disagree: bool,
    /// The reads that store it whole to a field a list names, and the role that name gives.
    stored: u32,
    stored_as: Option<String>,
    /// Two of those fields have different names.
    stored_apart: bool,
    /// The assignments that are the start or the step of a `for` it is the index of.
    counted: u32,
    /// The assignments whose value is a call or a load, whatever it calls or loads.
    fetched: u32,
    /// The role the method a delegate made by an assignment is bound to gives, the assignments that make one, and whether two give different roles.
    binds: Option<String>,
    binding: u32,
    binds_apart: bool,
    /// The role of the named field an assignment reads, and the assignments that read one.
    field_read: Option<String>,
    fields_read: u32,
    /// The assignments that are the item of a `foreach`.
    items: u32,
}

/// What a `foreach` goes through, as far as names go.
enum Gone {
    /// A field a list names, by the role its name gives.
    Field(String),
    /// A variable: a named field where it is given one value and that value is a read of one.
    Held(Var),
    Other,
}

/// One `for` that counts with a variable.
struct Counted {
    index: Var,
    /// The place in the list of the nearest such loop this one is nested in.
    outer: Option<usize>,
    /// Where the tree holds the start of the loop, which the printer may declare the index at.
    start: Option<*const Stmt>,
    /// Whether no source construct holds the loop: the form of old declares what a construct mentions at the top.
    plain: bool,
}

#[derive(Default)]
struct Gathered {
    facts: BTreeMap<Var, Facts>,
    /// The names the printer writes in front of a call's first dot, which a variable must not share.
    taken: BTreeSet<String>,
    /// Every `for` whose head counts with one variable, an outer loop before the loops in it.
    loops: Vec<Counted>,
    /// The item of each `foreach`, with what the loop goes through.
    items: Vec<(Var, Gone)>,
}

// An explicit stack: a body nests as deep as its listing does.
fn gather(body: &[tree::Stmt], head: &Head, cx: &Context<'_>) -> Gathered {
    let mut gathered = Gathered::default();
    // Each body with the nearest counted loop it is in, and whether no construct holds it.
    let mut work: Vec<(&[tree::Stmt], Option<usize>, bool)> = vec![(body, None, true)];
    while let Some((stmts, outer, plain)) = work.pop() {
        for stmt in stmts {
            match stmt {
                tree::Stmt::Simple(s) => gathered.simple(s, head, cx),
                tree::Stmt::If { cond, then, els } => {
                    gathered.expr(cond, head, cx);
                    work.push((then.as_slice(), outer, plain));
                    work.push((els.as_slice(), outer, plain));
                }
                tree::Stmt::While { cond, body } | tree::Stmt::DoWhile { body, cond } => {
                    gathered.expr(cond, head, cx);
                    work.push((body.as_slice(), outer, plain));
                }
                tree::Stmt::For {
                    init,
                    cond,
                    step,
                    body,
                } => {
                    for s in [init, step].into_iter().flatten() {
                        gathered.simple(s, head, cx);
                    }
                    if let Some(cond) = cond {
                        gathered.expr(cond, head, cx);
                    }
                    let inner = match counter(init.as_deref(), cond.as_ref(), step.as_deref()) {
                        Some(index) => {
                            gathered.facts.entry(index).or_default().counted += 2;
                            let start = init.as_deref().map(std::ptr::from_ref);
                            let counted = Counted {
                                index,
                                outer,
                                start,
                                plain,
                            };
                            gathered.loops.push(counted);
                            Some(gathered.loops.len() - 1)
                        }
                        None => outer,
                    };
                    work.push((body.as_slice(), inner, plain));
                }
                tree::Stmt::Switch { value, cases } => {
                    gathered.expr(value, head, cx);
                    let bodies = cases
                        .iter()
                        .map(|case| (case.body.as_slice(), outer, plain));
                    work.extend(bodies);
                }
                tree::Stmt::Return(Some(value)) => gathered.expr(value, head, cx),
                tree::Stmt::Sugar(sugar) => {
                    // What a handler catches is bound by the fault: an assignment no statement shows.
                    if let tree::Sugar::Try { catches, .. } = &**sugar {
                        for var in catches.iter().filter_map(|catch| catch.var) {
                            gathered.facts.entry(var).or_default().assigned += 1;
                        }
                    }
                    if let tree::Sugar::Group {
                        kind: tree::Group::Field { owner },
                        parts,
                    } = &**sugar
                    {
                        gathered.stored(*owner, parts, cx);
                    }
                    if cx.options.on(Pass::RoleNames) {
                        gathered.made(sugar, cx);
                    }
                    let parts = sugar.parts();
                    for s in parts.simples {
                        gathered.simple(s, head, cx);
                    }
                    for e in parts.exprs {
                        gathered.expr(e, head, cx);
                    }
                    work.extend(parts.bodies.into_iter().map(|body| (body, outer, false)));
                }
                tree::Stmt::Return(None)
                | tree::Stmt::Break
                | tree::Stmt::Continue
                | tree::Stmt::Goto(_)
                | tree::Stmt::Label(_)
                | tree::Stmt::Unreachable => {}
            }
        }
    }
    gathered
}

impl Gathered {
    fn simple(&mut self, s: &Stmt, head: &Head, cx: &Context<'_>) {
        s.each_expr(&mut |e| self.expr(e, head, cx));
        for target in s.targets() {
            self.facts.entry(*target).or_default().assigned += 1;
        }
        let Stmt::Assign { target, value } = s else {
            return;
        };
        if matches!(
            bare(value),
            Expr::Call { .. } | Expr::Load { .. } | Expr::LoadBytes { .. }
        ) {
            self.facts.entry(*target).or_default().fetched += 1;
        }
        let read = cx
            .options
            .on(Pass::RoleNames)
            .then(|| field_role(value, cx));
        if let Some(role) = read.flatten() {
            let facts = self.facts.entry(*target).or_default();
            facts.fields_read += 1;
            facts.field_read = Some(role);
        }
        if let Some(role) = result_role(value, cx.names) {
            let facts = self.facts.entry(*target).or_default();
            facts.results += 1;
            facts.results_apart |= (facts.result_of.as_ref()).is_some_and(|held| *held != role);
            facts.result_of = Some(role);
        } else if cx.options.on(Pass::RoleNames) {
            if let Some(role) = held_role(value, head, cx) {
                let facts = self.facts.entry(*target).or_default();
                facts.holding += 1;
                facts.holds_apart |= (facts.holds.as_ref()).is_some_and(|held| *held != role);
                facts.holds = Some(role);
            }
        }
    }

    fn expr(&mut self, top: &Expr, head: &Head, cx: &Context<'_>) {
        top.walk(&mut |e| match e {
            Expr::Var(var) => self.facts.entry(*var).or_default().read += 1,
            Expr::AddrOf { var, .. } => {
                let facts = self.facts.entry(*var).or_default();
                facts.read += 1;
                facts.addresses += 1;
            }
            Expr::Call { callee, args, .. } => {
                let printed = print::callee_name(callee, cx.names);
                let front = printed.split('.').next().unwrap_or_default();
                self.taken.insert(front.to_owned());
                // ⚠ A name a list opens with `_` is a guess, and so are the parameters it gives.
                let listed = match callee {
                    Callee::Native { hash, .. } => {
                        (cx.names.native(*hash)).filter(|native| !native.name.starts_with('_'))
                    }
                    _ => None,
                };
                if let Some(native) = listed {
                    self.passed(native, args, head, cx.options.on(Pass::NativeParams));
                }
            }
            // A function bound by its name is written by it: a variable of that name would hide it.
            Expr::FunctionRef { program, at } => {
                let key = FunctionKey::new(*program, at & FUNCTION_OFFSET);
                if let Some(name) = cx.names.function(key) {
                    self.taken.insert(name.to_owned());
                }
            }
            _ => {}
        });
    }

    /// Notes each variable a store of a field of `owner` stores whole, with the role the field's name gives, where a list names it.
    fn stored(&mut self, owner: TypeKey, parts: &[Stmt], cx: &Context<'_>) {
        if !cx.options.on(Pass::RoleNames) {
            return;
        }
        for part in parts {
            let Stmt::Store { offset, value, .. } = part else {
                continue;
            };
            let Expr::Var(var) = bare(value) else {
                continue;
            };
            let Some(role) = field_name(owner, *offset, cx).and_then(camel) else {
                continue;
            };
            let facts = self.facts.entry(*var).or_default();
            facts.stored += 1;
            facts.stored_apart |= (facts.stored_as.as_ref()).is_some_and(|held| *held != role);
            facts.stored_as = Some(role);
        }
    }

    /// Notes the role the method a delegate made is bound to gives the variable it is made in, and the item of a `foreach` with what the loop goes through.
    fn made(&mut self, sugar: &tree::Sugar, cx: &Context<'_>) {
        match sugar {
            tree::Sugar::Group {
                kind: tree::Group::Delegate,
                parts,
            } => {
                let Some((target, role)) = bound(parts, cx) else {
                    return;
                };
                let facts = self.facts.entry(target).or_default();
                facts.binding += 1;
                facts.binds_apart |= (facts.binds.as_ref()).is_some_and(|held| *held != role);
                facts.binds = Some(role);
            }
            tree::Sugar::Foreach {
                setup,
                cond,
                item,
                step,
                ..
            } => {
                let Stmt::Assign { target, .. } = &**item else {
                    return;
                };
                let source = collection(setup, cond, item, step.as_deref());
                let gone = match source.map(|source| (field_role(source, cx), bare(source))) {
                    Some((Some(role), _)) => Gone::Field(role),
                    Some((None, Expr::Var(var))) => Gone::Held(*var),
                    _ => Gone::Other,
                };
                self.facts.entry(*target).or_default().items += 1;
                self.items.push((*target, gone));
            }
            _ => {}
        }
    }

    /// Counts each argument that is a variable passed as the whole of a named parameter of `native`, and where `fills`, notes each whose address is passed for the native to fill.
    fn passed(&mut self, native: &NativeName, args: &[Expr], head: &Head, fills: bool) {
        for (arg, param) in args.iter().zip(whole_params(native, args, head)) {
            let Some(param) = param else {
                continue;
            };
            let Some(role) = parameter_role(&param.name) else {
                continue;
            };
            match bare(arg) {
                Expr::Var(var) => {
                    let facts = self.facts.entry(*var).or_default();
                    facts.passed += 1;
                    facts.disagree |= (facts.parameter.as_ref()).is_some_and(|held| *held != role);
                    facts.parameter = Some(role);
                }
                Expr::AddrOf { var, cell: 0 } if fills && scalar_pointer(&param.ty) => {
                    self.facts.entry(*var).or_default().filled_as = Some(role);
                }
                _ => {}
            }
        }
    }
}

/// `e` without the casts and the source readings around it: a reading of a value's low bits is still that value's role.
fn bare(e: &Expr) -> &Expr {
    let mut e = e;
    while let Expr::Cast { e: inner, .. } | Expr::Sugar { e: inner, .. } = e {
        e = &**inner;
    }
    e
}

/// The role a call of a named native gives the variable that holds its result.
fn result_role(value: &Expr, names: &Names) -> Option<String> {
    let Expr::Call {
        callee: Callee::Native { name, hash, .. },
        ..
    } = bare(value)
    else {
        return None;
    };
    let name = names.native(*hash).map_or(name, |listed| &listed.name);
    role_of_native(name)
}

/// The start, the test and the step of a `for`, where they are those of a loop that counts with one variable: the variable.
// ⚠ The whole head: the variable is set, then tested, then moved by a constant. A head with a
// part missing, or one that steps to what a call or a load says, walks something other than a count.
fn counter(init: Option<&Stmt>, cond: Option<&Expr>, step: Option<&Stmt>) -> Option<Var> {
    let (Some(Stmt::Assign { target, .. }), Some(cond), Some(step)) = (init, cond, step) else {
        return None;
    };
    let Stmt::Assign {
        target: stepped,
        value,
    } = step
    else {
        return None;
    };
    let Expr::Binary {
        op: BinOp::Add | BinOp::Sub,
        l,
        r,
        ..
    } = bare(value)
    else {
        return None;
    };
    let by_constant = *bare(l) == Expr::Var(*target) && matches!(bare(r), Expr::Const(_));
    let mut tested = false;
    cond.walk(&mut |e| tested |= *e == Expr::Var(*target));
    (stepped == target && by_constant && tested).then_some(*target)
}

/// The letter each variable takes that is nothing but the index of counted loops: `i`, then `j` and `k` for a loop inside one counted with a letter.
// ⚠ A letter says "this counts a loop", so every part must hold: a local proved an integer,
// written by the starts and steps of `for` heads alone, and its address never taken and under no
// earlier local's. How the count is read is not asked. A variable that counts at two depths, or
// deeper than the letters go, keeps its slot name.
fn counters(
    facts: &BTreeMap<Var, Facts>,
    loops: &[Counted],
    exposed: &[u32],
    head: &Head,
) -> BTreeMap<Var, &'static str> {
    let counts = |var: Var| {
        let integer = (head.vars.get(&var))
            .is_some_and(|info| info.width == Width::Cell && info.kind.is_integer());
        let alone = (facts.get(&var))
            .is_some_and(|facts| facts.assigned == facts.counted && facts.addresses == 0);
        matches!(var, Var::Local(_)) && integer && alone && !covered(var, exposed, head)
    };
    // An outer loop is listed before the loops in it, so its depth is known when theirs is asked.
    let mut depths: Vec<usize> = Vec::with_capacity(loops.len());
    let mut depth_of: BTreeMap<Var, Option<usize>> = BTreeMap::new();
    for counted in loops {
        let outer = counted
            .outer
            .and_then(|n| Some((loops.get(n)?, *depths.get(n)?)));
        let depth = match outer {
            Some((outer, depth)) => depth.saturating_add(usize::from(counts(outer.index))),
            None => 0,
        };
        depths.push(depth);
        if counts(counted.index) {
            let held = depth_of.entry(counted.index).or_insert(Some(depth));
            if *held != Some(depth) {
                *held = None;
            }
        }
    }
    let lettered = depth_of.into_iter().filter_map(|(var, depth)| {
        let letter = *COUNTERS.get(depth?)?;
        Some((var, letter))
    });
    lettered.collect()
}

/// The singular each `foreach` item takes from the named field its loops go through; `None` where one of them goes through no such field, gives no singular, or gives another.
fn singulars(
    items: &[(Var, Gone)],
    facts: &BTreeMap<Var, Facts>,
    exposed: &[u32],
    head: &Head,
) -> BTreeMap<Var, Option<String>> {
    let mut found: BTreeMap<Var, Option<String>> = BTreeMap::new();
    for (item, gone) in items {
        let field = match gone {
            Gone::Field(role) => Some(role.as_str()),
            Gone::Held(var) => held_field(*var, facts, exposed, head),
            Gone::Other => None,
        };
        let one = field.and_then(singular);
        let held = found.entry(*item).or_insert_with(|| one.clone());
        if *held != one {
            *held = None;
        }
    }
    found
}

/// The role of the named field `var` holds, where reading it is the one value `var` is given and no address reaches it.
fn held_field<'f>(
    var: Var,
    facts: &'f BTreeMap<Var, Facts>,
    exposed: &[u32],
    head: &Head,
) -> Option<&'f str> {
    let facts = facts.get(&var)?;
    let one = facts.assigned == 1 && facts.fields_read == 1 && facts.addresses == 0;
    let local = !matches!(var, Var::Param(_) | Var::Global { .. }) && !covered(var, exposed, head);
    facts.field_read.as_deref().filter(|_| one && local)
}

/// Whether `var` is given values only as the item of `foreach` loops, and none through an address.
fn only_items(var: Var, facts: &Facts, exposed: &[u32], head: &Head) -> bool {
    let local = !matches!(var, Var::Param(_) | Var::Global { .. }) && !covered(var, exposed, head);
    facts.items > 0 && facts.items == facts.assigned && facts.addresses == 0 && local
}

/// The indices the printer declares in the head of every loop they count, where no construct holds the loop: two of them of one letter may share its name.
// ⚠ C# allows one name to two variables only where neither's scope holds the other's. Such an
// index is mentioned in its loops alone, and two indices of one letter never count loops nested
// one in the other: the loop inside a lettered one takes the next letter. The form of old
// declares what a construct mentions at the top of the function, where two would meet.
fn scoped(loops: &[Counted], body: &[tree::Stmt], head: &Head) -> BTreeSet<Var> {
    let sites = print::declared_at(body, head);
    let mut own: BTreeMap<Var, bool> = BTreeMap::new();
    for counted in loops {
        let declared = counted.start.is_some_and(|start| sites.contains(&start));
        *own.entry(counted.index).or_insert(true) &= counted.plain && declared;
    }
    let own = own.into_iter().filter_map(|(var, own)| own.then_some(var));
    own.collect()
}

/// The role the one value a variable holds gives it: the type of a new object or of a cast, what a getter gets, or the named field it is read from.
fn held_role(value: &Expr, head: &Head, cx: &Context<'_>) -> Option<String> {
    let mut value = value;
    loop {
        match value {
            Expr::Cast { e, .. } => value = &**e,
            Expr::Sugar {
                kind: ExprSugar::Field { .. },
                ..
            } => return field_role(value, cx),
            Expr::Sugar { e, .. } => value = &**e,
            Expr::NewObject(ty) => {
                // A type nothing names is no role, whatever its hash spells.
                let name = match TypeKey::of_name(&ty.name) {
                    Some(key) => cx.names.type_name(key.0)?,
                    None => ty.name.as_str(),
                };
                return type_role(name);
            }
            // C# reads an array's length as `a.Length`.
            Expr::Call {
                callee: Callee::Intrinsic { name, .. },
                args,
                ..
            } if name == "array_length" && args.len() == 1 => return Some("length".to_owned()),
            Expr::Call {
                callee: Callee::Intrinsic { name, .. },
                ..
            } => return cast_role(name, cx.names),
            Expr::Call { callee, .. } => return method_role(method_name(callee, head, cx)?),
            _ => return None,
        }
    }
}

/// The name of the method a call reaches, where the program or a list names it.
// ⚠ A slot's name is the declaring type's: it names the slot of a type only where that type's own
// base chain reaches the declaring one, never the same slot of an unrelated type.
fn method_name<'a>(callee: &'a Callee, head: &Head, cx: &Context<'a>) -> Option<&'a str> {
    match callee {
        // A function is a method here only under a name a list gave it.
        Callee::Local { at, .. } => cx.names.function(FunctionKey::new(head.program, *at)),
        Callee::Import { program, at, .. } => cx.names.function(FunctionKey::new(*program, *at)),
        Callee::Builtin { path, .. } => path.rsplit('.').next(),
        Callee::Virtual { ty, slot, .. } => slot_name(TypeKey::of_name(ty)?, *slot, cx),
        _ => None,
    }
}

/// The name a list gives virtual slot `slot` of `ty`, looked for on the type and then on its bases.
fn slot_name<'a>(ty: TypeKey, slot: u32, cx: &Context<'a>) -> Option<&'a str> {
    let mut key = ty;
    for _ in 0..MAX_BASES {
        if let Some(name) = cx.names.method(key, slot) {
            return Some(name);
        }
        key = cx.project.types.get(&key)?.base?;
    }
    None
}

/// The name a list gives the field at byte `offset` of `owner` or of a base of it, as the printer writes the field.
fn field_name<'a>(owner: TypeKey, offset: u32, cx: &Context<'a>) -> Option<&'a str> {
    let mut key = owner;
    for _ in 0..MAX_BASES {
        if let Some(name) = cx.names.field(key, offset) {
            return Some(name);
        }
        key = cx.project.types.get(&key)?.base?;
    }
    None
}

/// The role of the named field `value` reads, under its casts and source readings.
fn field_role(value: &Expr, cx: &Context<'_>) -> Option<String> {
    let mut value = value;
    loop {
        match value {
            Expr::Sugar {
                kind: ExprSugar::Field { owner },
                e,
            } => {
                let (Expr::Load { offset, .. } | Expr::LoadBytes { offset, .. }) = &**e else {
                    return None;
                };
                return camel(field_name(*owner, *offset, cx)?);
            }
            Expr::Cast { e, .. } | Expr::Sugar { e, .. } => value = &**e,
            _ => return None,
        }
    }
}

/// The variable a delegate `parts` make is held in, with the role the method it is bound to gives, where a list names that method.
// ⚠ Only the form the printer writes as `d = x.Method`: the object made, then constructed on
// itself. A lambda is written in place and has no name, whatever its function is called.
fn bound(parts: &[Stmt], cx: &Context<'_>) -> Option<(Var, String)> {
    let [Stmt::Assign { target, value }, Stmt::Eval(call)] = parts else {
        return None;
    };
    if !matches!(bare(value), Expr::NewObject(_)) {
        return None;
    }
    let Expr::Call { args, .. } = bare(call) else {
        return None;
    };
    let [object, _, function] = args.as_slice() else {
        return None;
    };
    if *bare(object) != Expr::Var(*target) {
        return None;
    }
    let mut function = function;
    loop {
        match function {
            Expr::Sugar {
                kind: ExprSugar::Lambda,
                ..
            } => return None,
            Expr::Cast { e, .. } | Expr::Sugar { e, .. } => function = &**e,
            _ => break,
        }
    }
    let name = match function {
        // The top byte of a reference's offset holds its flags.
        Expr::FunctionRef { program, at } => {
            (cx.names).function(FunctionKey::new(*program, at & FUNCTION_OFFSET))
        }
        Expr::VirtualFunctionRef { ty, slot, .. } => slot_name(TypeKey::of(ty)?, *slot, cx),
        // The lifter tells two overloads of one builtin apart after `__`.
        Expr::BuiltinFunctionRef { ty, method } => (cx.names.builtin(*ty, *method)?)
            .rsplit('.')
            .next()
            .and_then(|name| name.split("__").next()),
        _ => None,
    }?;
    Some((*target, camel(name)?))
}

/// What a `foreach` goes through, as the printer reads its parts: the array an index steps over, the collection a struct enumerator is made from, or what the enumerator each round asks is taken from.
fn collection<'e>(
    setup: &'e [Stmt],
    cond: &'e Expr,
    item: &'e Stmt,
    step: Option<&'e Stmt>,
) -> Option<&'e Expr> {
    if let Some(step) = step {
        return indexed(setup, item, step);
    }
    if let Some((_, source)) = super::sugar_statements::enumerated(setup, cond) {
        return Some(source);
    }
    let Some(Stmt::Assign { target, value }) = setup.last() else {
        return None;
    };
    let asked = matches!(receiver(cond), Some(Expr::Var(var)) if var == target);
    asked.then(|| receiver(value)).flatten()
}

/// The array the item of a `foreach` is read from at an index that its setup starts at zero and its step moves.
fn indexed<'e>(setup: &[Stmt], item: &'e Stmt, step: &Stmt) -> Option<&'e Expr> {
    let [Stmt::Assign {
        target: index,
        value: start,
    }] = setup
    else {
        return None;
    };
    let (
        Stmt::Assign { value, .. },
        Stmt::Assign {
            target: stepped, ..
        },
    ) = (item, step)
    else {
        return None;
    };
    let Expr::Call {
        callee: Callee::Intrinsic { name, .. },
        args,
        ..
    } = bare(value)
    else {
        return None;
    };
    let [array, at] = args.as_slice() else {
        return None;
    };
    let read = name.starts_with("array_get_") && *integral(at) == Expr::Var(*index);
    (read && index == stepped && is_zero(start)).then_some(array)
}

/// What a test asks of the call it makes, under its readings and a comparison with zero: the call's first argument.
fn receiver(e: &Expr) -> Option<&Expr> {
    let mut e = e;
    loop {
        match e {
            Expr::Cast { e: inner, .. }
            | Expr::Sugar { e: inner, .. }
            | Expr::Truth(inner)
            | Expr::Unary { e: inner, .. } => e = &**inner,
            Expr::Binary {
                op: BinOp::Eq | BinOp::Ne,
                l,
                r,
                ..
            } if is_zero(r) => e = &**l,
            Expr::Call { args, .. } => return args.first(),
            _ => return None,
        }
    }
}

/// `e` under the readings of it as an integer, which leave its bits.
fn integral(e: &Expr) -> &Expr {
    let mut e = e;
    while let Expr::Cast { to, e: inner } = e {
        if to.kind().is_float() {
            break;
        }
        e = &**inner;
    }
    e
}

/// Whether `e` is a constant with no bit set, under any reading of it.
fn is_zero(e: &Expr) -> bool {
    let mut e = e;
    while let Expr::Cast { e: inner, .. } | Expr::Truth(inner) = e {
        e = &**inner;
    }
    matches!(e, Expr::Const(c) if c.bits == 0)
}

/// `snorkel` for `snorkels`: the singular of a plural that ends in a plain `s`; `None` for any other word, `boxes`, `entries`, `status` and `ids` among them.
// ⚠ Only where dropping the `s` leaves the word: after a hiss or a vowel an `s` may be part of
// it or of an `-es`, and a stem of two letters is more often a short word than a plural.
fn singular(plural: &str) -> Option<String> {
    let stem = plural.strip_suffix('s')?;
    let last = stem.chars().last()?;
    let plain = last.is_ascii_lowercase() && !matches!(last, 's' | 'u' | 'i');
    let before = stem.strip_suffix('e').and_then(|rest| rest.chars().last());
    let es = before.is_some_and(|c| matches!(c, 's' | 'x' | 'z' | 'h' | 'i' | 'o'));
    if !plain || es || stem.len() < 3 {
        return None;
    }
    spelled(stem).filter(|stem| !stem.starts_with('@'))
}

/// Whether `name` is one the `foreach` pass makes for an item no variable of the listing holds: `item`, or `item_` and a number.
fn made_item(name: &str) -> bool {
    let number = |n: &str| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit());
    name == "item" || name.strip_prefix("item_").is_some_and(number)
}

/// The role one fact gives `var`: the result of a native, then the parameter of one, then what else it holds, or the parameter it is filled through; under `more` also a role every value it is given has, the role of its type `typed` where a call or a load gives its one value, and a parameter's name its one value is passed as; `made_with` where `var` is a parameter of a constructor other than its receiver.
// ⚠ One slot may be several variables whose lives do not meet. Under `more` a slot assigned
// several times takes a role only where every assignment gives that one role, and a parameter's
// name names only a slot given one value, which is then what the parameter is passed.
fn role(
    var: Var,
    facts: &Facts,
    exposed: &[u32],
    head: &Head,
    more: bool,
    typed: Option<String>,
    made_with: bool,
) -> Option<String> {
    if matches!(var, Var::Global { .. }) {
        return None;
    }
    let apart = !covered(var, exposed, head);
    if facts.addresses > 0 {
        // ⚠ One address, handed to the native that fills the one cell: any other write, and any
        // other use of the address, is another value.
        let one_cell = (head.vars.get(&var)).is_none_or(|info| info.width == Width::Cell);
        let local = matches!(var, Var::Local(_)) && one_cell && apart;
        let filled = facts.addresses == 1 && facts.assigned == 0 && local;
        return facts.filled_as.clone().filter(|_| filled);
    }
    // ⚠ A parameter also holds what its caller passed: no assignment is its only value.
    let local = !matches!(var, Var::Param(_)) && apart;
    let holds_one = facts.assigned == 1 && local;
    let every = |given: u32, differ: bool| more && local && given == facts.assigned && !differ;
    let results = every(facts.results, facts.results_apart);
    if let Some(role) = facts.result_of.as_ref().filter(|_| holds_one || results) {
        return Some(role.clone());
    }
    let always = facts.read > 0 && facts.read == facts.passed && !facts.disagree;
    if let Some(role) = facts.parameter.as_ref().filter(|_| always) {
        return Some(role.clone());
    }
    // A value only ever stored whole to one named field is what that field holds: `this.target = target`.
    let kept = facts.read > 0 && facts.read == facts.stored && !facts.stored_apart;
    let given = holds_one || (matches!(var, Var::Param(_)) && facts.assigned == 0);
    if let Some(role) = facts.stored_as.as_ref().filter(|_| kept && given) {
        return Some(role.clone());
    }
    // What a constructor is handed and keeps in one named field is that field, however else it is read.
    let made_from = more && made_with && facts.assigned == 0 && facts.stored > 0;
    if let Some(role) = (facts.stored_as.as_ref()).filter(|_| made_from && !facts.stored_apart) {
        return Some(role.clone());
    }
    let binding = every(facts.binding, facts.binds_apart);
    if let Some(role) = facts.binds.as_ref().filter(|_| holds_one || binding) {
        return Some(role.clone());
    }
    let holding = every(facts.holding, facts.holds_apart);
    if let Some(role) = facts.holds.as_ref().filter(|_| holds_one || holding) {
        return Some(role.clone());
    }
    // A constant or a copy says nothing of where the object is from: only a call or a load.
    let fetched = more && holds_one && facts.fetched == 1;
    if let Some(role) = typed.filter(|_| fetched) {
        return Some(role);
    }
    let passed = more && given && facts.passed > 0 && !facts.disagree;
    facts.parameter.clone().filter(|_| passed)
}

/// Whether a write through the address of an earlier local reaches `var`.
// ⚠ The address of `local_n` is also that of the locals after it; how many, only the kind of
// `local_n` says. A local of no known kind is taken to be the one cell the listing shows.
fn covered(var: Var, exposed: &[u32], head: &Head) -> bool {
    let Var::Local(n) = var else {
        return false;
    };
    exposed.iter().any(|m| {
        let cells = match head.vars.get(&Var::Local(*m)).map(|info| &info.kind) {
            Some(Kind::Vector | Kind::Handle) => 2,
            Some(Kind::Aggregate(cells)) => *cells,
            _ => 1,
        };
        *m < n && n - *m < cells
    })
}

/// `role`, or `role_2` and on while a name of the function has it; the name is taken from then on.
fn unique(role: &str, taken: &mut BTreeSet<String>) -> String {
    let mut name = role.to_owned();
    let mut n = 1;
    while taken.contains(&name) {
        n += 1;
        name = format!("{role}_{n}");
    }
    taken.insert(name.clone());
    name
}

/// For each argument, the parameter of `native` it is the whole of.
// ⚠ A list's parameters are another game's or a guess. They name an argument only where their
// cells, counted by `param_cells`, are exactly the call's: a parameter then lines up with one
// argument of its width, or with a run of one-cell arguments, none of which is the whole of it.
fn whole_params<'n>(
    native: &'n NativeName,
    args: &[Expr],
    head: &Head,
) -> Vec<Option<&'n NativeParam>> {
    let nothing = vec![None; args.len()];
    let mut whole = nothing.clone();
    let mut next = 0;
    for param in &native.params {
        let cells = param_cells(&param.ty);
        let width = args.get(next).and_then(|arg| print::cells_of(arg, head));
        let spread = |run: &[Expr]| run.iter().all(|arg| print::cells_of(arg, head) == Some(1));
        if width == Some(cells) {
            whole[next] = Some(param);
            next += 1;
        } else if args.get(next..next + cells as usize).is_some_and(spread) {
            next += cells as usize;
        } else {
            return nothing;
        }
    }
    match next == args.len() {
        true => whole,
        false => nothing,
    }
}

/// The cells a parameter of the type a list names takes in a GTA VI script: two for a world handle and for a vector.
fn param_cells(ty: &str) -> u32 {
    match ty {
        "Entity" | "Ped" | "Vehicle" | "Object" | "Pickup" | "Vector3" => 2,
        _ => 1,
    }
}

/// Whether a list's parameter type is a pointer to one scalar cell, which a native fills.
// A pointer to text, to a handle or a vector, or to a value of no stated size fills what no list sizes.
fn scalar_pointer(ty: &str) -> bool {
    matches!(ty, "int*" | "float*" | "BOOL*" | "Hash*")
}

/// `playerPed` for `PLAYER_PED_ID`: a native's name in camel case, without a leading `GET_` or `CREATE_` and a trailing `_ID`; `None` for one opening with `_`.
// ⚠ A list writes a name it guessed, or a hash, after a `_`: neither says what the result is.
fn role_of_native(name: &str) -> Option<String> {
    if name.starts_with('_') {
        return None;
    }
    let name = ["GET_", "CREATE_"]
        .iter()
        .find_map(|verb| name.strip_prefix(*verb))
        .unwrap_or(name);
    let name = name.strip_suffix("_ID").unwrap_or(name);
    spelled(&words(name)?)
}

/// The words `name` joins with `_`, as one word in camel case; `None` where one is no letters and digits.
fn words(name: &str) -> Option<String> {
    let mut role = String::new();
    for word in name.split('_').filter(|word| !word.is_empty()) {
        if !word.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return None;
        }
        let (first, rest) = word.split_at(1);
        match role.is_empty() {
            true => role.push_str(&first.to_ascii_lowercase()),
            false => role.push_str(&first.to_ascii_uppercase()),
        }
        role.push_str(&rest.to_ascii_lowercase());
    }
    Some(role)
}

/// `animScene` for `AnimScene`, `uiState` for `UIState`, `tint` for `_tint`, `eventDone` for `EVENT_DONE`: a type's, a member's or a field's name as a variable's.
// A name that is a word of C# names nothing here: `@delegate` on every delegate says less than `local_3`.
fn camel(name: &str) -> Option<String> {
    let name = name.strip_prefix('_').unwrap_or(name);
    let role = match name.bytes().any(|b| b.is_ascii_lowercase()) {
        false => words(name)?,
        true => {
            // The capitals a name opens with are one word, less the last where a word follows it.
            let opening = match name.bytes().take_while(u8::is_ascii_uppercase).count() {
                capitals @ (0 | 1) => capitals,
                capitals => capitals - 1,
            };
            let (opening, rest) = name.split_at(opening);
            format!("{}{rest}", opening.to_ascii_lowercase())
        }
    };
    spelled(&role).filter(|role| !role.starts_with('@'))
}

/// `lantern` for `Lantern`, `dictionary` for `System.Collections.Generic.Dictionary_2` and `list` for `List<Lantern>`: a type's short name as a variable's.
fn type_role(name: &str) -> Option<String> {
    let name = without_arguments(name);
    let short = name.rsplit('.').next().unwrap_or(name);
    // A generic type's name ends in the count of its parameters.
    let (stem, count) = short.rsplit_once('_').unwrap_or((short, ""));
    let generic = !count.is_empty() && count.bytes().all(|b| b.is_ascii_digit());
    camel(if generic { stem } else { short })
}

/// `name` without the arguments of a generic type it ends in: `A.List` for `A.List<B<C>>`; `name` where it ends in no `>` or its `<` and `>` do not pair.
// A nested type of a generic one, `Map<A, B>.Keys`, ends in its own name and keeps it.
fn without_arguments(name: &str) -> &str {
    if !name.ends_with('>') {
        return name;
    }
    let mut open = 0u32;
    for (at, c) in name.char_indices().rev() {
        match c {
            '>' => open = open.saturating_add(1),
            '<' => {
                open = open.saturating_sub(1);
                if open == 0 {
                    return &name[..at];
                }
            }
            _ => {}
        }
    }
    name
}

/// The role the script type a variable is proved to hold gives it, where a list names that type.
// ⚠ A script's type only, as for a cast: a runtime type's name is as often a number's or a box's.
fn typed_role(var: Var, head: &Head, names: &Names) -> Option<String> {
    let ty = match &head.vars.get(&var)?.kind {
        Kind::Object(ty) | Kind::Value { ty, .. } => ty,
        _ => return None,
    };
    let key = TypeKey::of_name(ty).filter(|key| key.builtin().is_none())?;
    type_role(names.type_name(key.0)?)
}

/// The role `x as T` or `(T)x` gives the variable that holds it, where `T` is a script's type a list names.
// ⚠ A script's type only: a cast to one of the runtime's is as often an unboxing, `(int)o`,
// and the runtime's name of a number says nothing of what the number is.
fn cast_role(intrinsic: &str, names: &Names) -> Option<String> {
    let ty = ["try_cast<", "cast<"]
        .iter()
        .find_map(|cast| intrinsic.strip_prefix(*cast))?;
    let key = TypeKey::of_name(ty.strip_suffix('>')?).filter(|key| key.builtin().is_none())?;
    type_role(names.type_name(key.0)?)
}

/// `kite` for `GetKite`, `count` for `get_Count` and `isMoored` for `IsMoored`: what a getter, a maker or a test hands back; `None` for any other method.
fn method_role(name: &str) -> Option<String> {
    // The lifter tells two overloads of one builtin apart after `__`.
    let name = name.split("__").next().unwrap_or(name);
    let word = |rest: &&str| rest.starts_with(|c: char| c.is_ascii_uppercase());
    let after = |verbs: &[&str]| verbs.iter().find_map(|verb| name.strip_prefix(*verb));
    let got =
        (name.strip_prefix("get_")).or_else(|| after(&["Get", "Create", "Find"]).filter(word));
    let tested = after(&["Is", "Has"]).filter(word).map(|_| name);
    camel(got.or(tested)?)
}

/// A list's name for a parameter as a variable's, where it is a name and no placeholder.
fn parameter_role(name: &str) -> Option<String> {
    let stem = name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '_');
    let placeholder = ["", "p", "unk", "unknown", "param", "arg"]
        .iter()
        .any(|held| stem.eq_ignore_ascii_case(held));
    spelled(name).filter(|_| !placeholder)
}

/// `name` as the printer may write it for a variable: `None` for what is no identifier or reads as a name the printer makes.
fn spelled(name: &str) -> Option<String> {
    const MADE: [&str; 9] = [
        "Global_", "Static_", "String_", "func_", "label_", "block_", "type_", "module_",
        "program_",
    ];
    let starts = name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_');
    let identifier = starts && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    let made = Var::from_name(name).is_some() || MADE.iter().any(|made| name.starts_with(*made));
    if !identifier || made {
        return None;
    }
    // C# spells a keyword used as a name with `@` in front.
    Some(match KEYWORDS.contains(&name) {
        true => format!("@{name}"),
        false => name.to_owned(),
    })
}

/// The words C# reserves, and `var`.
const KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "base",
    "bool",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "checked",
    "class",
    "const",
    "continue",
    "decimal",
    "default",
    "delegate",
    "do",
    "double",
    "else",
    "enum",
    "event",
    "explicit",
    "extern",
    "false",
    "finally",
    "fixed",
    "float",
    "for",
    "foreach",
    "goto",
    "if",
    "implicit",
    "in",
    "int",
    "interface",
    "internal",
    "is",
    "lock",
    "long",
    "namespace",
    "new",
    "null",
    "object",
    "operator",
    "out",
    "override",
    "params",
    "private",
    "protected",
    "public",
    "readonly",
    "ref",
    "return",
    "sbyte",
    "sealed",
    "short",
    "sizeof",
    "stackalloc",
    "static",
    "string",
    "struct",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "uint",
    "ulong",
    "unchecked",
    "unsafe",
    "ushort",
    "using",
    "var",
    "virtual",
    "void",
    "volatile",
    "while",
];

#[cfg(test)]
#[path = "names_tests.rs"]
mod tests;
