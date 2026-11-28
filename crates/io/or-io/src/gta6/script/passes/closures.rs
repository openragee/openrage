//! Closures: a delegate made and bound read as one statement, the function it binds read as a
//! lambda's body where what it is bound to is an object of a class the compiler made, a delegate
//! made on first use and kept in a global read as the one value it is, and the loop an event's
//! `+=` or `-=` is compiled to read as that statement.
//!
//! Works on the structured form and moves no statement: a construct holds the statements it was
//! read from. [`sole`], [`inline`] and [`inlinable`] say which bodies may be written where they
//! are bound, and [`kept_delegate`] and [`subscription`] what a construct says in the source.
//!
//! ⚠ `Pass::LocalFunctions` switches nothing. A local function that captures takes its variables
//! through an address, as any `ref` argument does, so none is proved.

use std::collections::{BTreeMap, BTreeSet};

use crate::gta6::script::class::{Class, ClassKind, Method, Role, Unit};
use crate::gta6::script::ir::{self, BinOp, Callee, Expr, ExprSugar, Returns, Scalar, UnOp, Var};
use crate::gta6::script::print;
use crate::gta6::script::tree::{self, Group, Stmt, Sugar};
use crate::gta6::script::{Context, Pass, Stats};

/// The names this file's rewrites are switched by.
pub const PASSES: &[Pass] = &[
    Pass::Lambdas,
    Pass::LocalFunctions,
    Pass::Delegates,
    Pass::Events,
];

/// The bit of a type's flag word that a type the compiler made carries: a closure class, a state machine.
const GENERATED: u32 = 0x0010_0000;
/// The bits of a type's flag word of which a value type carries one or more.
const VALUE: u32 = 0x1C;
/// The bit a builtin type's id carries above its index in the runtime's table.
const BUILTIN: u32 = 0x8000_0000;
/// The most statements one round of an event's loop holds: the copy kept, the join, the exchange, the copy back, and as many copies again.
const ROUND: usize = 8;
/// The statements before an event's loop that are read for the copy of the object it works on.
const COPIES: usize = 4;
/// The most simple statements a body holds to be written as a block where it is bound.
// The source wrote every lambda where it is bound, whatever its length: the bound only keeps a
// body of thousands of statements its own method.
const BLOCK: usize = 2048;

/// What the rewrites made of one function.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Found {
    /// Delegates made and bound, each now one group.
    pub delegates: usize,
    /// Functions bound to a closure object the same function makes.
    pub lambdas: usize,
    /// Functions bound to a holder in a global and kept in another: lambdas that capture nothing.
    pub cached: usize,
    /// Delegates made on first use and kept in a global, each now one construct.
    pub kept: usize,
    /// Loops that add a handler to an event or take one away, each now one construct.
    pub events: usize,
}

/// A delegate made and bound: the variable it is held in, the object bound and the function bound.
struct Made<'a> {
    delegate: Var,
    target: &'a Expr,
    function: &'a Expr,
}

/// Reads each delegate made and bound as one statement, the function bound to a closure as a lambda's body, a delegate kept in a global as one value and an event's loop as `+=` or `-=`.
pub fn run(f: &mut tree::Function, cx: &Context<'_>, _stats: &mut Stats) {
    rewrite(f, cx);
}

/// [`run`], with the count of what it made.
pub fn rewrite(f: &mut tree::Function, cx: &Context<'_>) -> Found {
    let group = cx.options.on(Pass::Delegates);
    let lambdas = cx.options.on(Pass::Lambdas);
    let events = cx.options.on(Pass::Events);
    let mut found = Found::default();
    if !group && !lambdas && !events {
        return found;
    }
    let program = f.head.program;
    let objects = match lambdas {
        true => closures(&f.body),
        false => BTreeSet::new(),
    };
    let first = survey(&f.body, events, lambdas);
    // Most functions hold neither a loop nor a test that asks how their variables are used.
    let uses = match first.tests || !first.looped.is_empty() {
        true => Uses::of(&f.body),
        false => Uses::default(),
    };
    // An explicit stack: a body nests as deep as its listing does.
    let mut work: Vec<&mut Vec<Stmt>> = vec![&mut f.body];
    while let Some(list) = work.pop() {
        if group {
            found.delegates += grouped(list);
        }
        if events && !first.looped.is_empty() {
            found.events += events_in(list, &uses, &first.looped);
        }
        if lambdas {
            for i in 0..list.len() {
                let closure = made(list, i).is_some_and(|(bound, _)| {
                    matches!(bare(bound.target), Expr::Var(held) if objects.contains(held))
                });
                if closure && read_as_lambda(list, i) {
                    found.lambdas += 1;
                }
            }
            for stmt in list.iter_mut() {
                let Stmt::If { cond, then, els } = stmt else {
                    continue;
                };
                let Some((cache, null)) = null_test(cond) else {
                    continue;
                };
                let (made_in, kept_in) = match null {
                    true => (then, els),
                    false => (els, then),
                };
                let Some(at) = cached(cache, made_in, kept_in, program) else {
                    continue;
                };
                found.cached += usize::from(read_as_lambda(made_in, at));
                if !kept_whole(cache, made_in, kept_in, at, &uses) {
                    continue;
                }
                if let Stmt::If { cond, then, els } = std::mem::replace(stmt, Stmt::Break) {
                    let (made, kept) = match null {
                        true => (then, els),
                        false => (els, then),
                    };
                    *stmt = Stmt::Sugar(Box::new(Sugar::Cached {
                        test: cond,
                        null,
                        made,
                        kept,
                    }));
                    found.kept += 1;
                }
            }
        }
        for stmt in list {
            work.extend(tree::nested_mut(stmt));
        }
    }
    found
}

/// The functions of one program that are a lambda's body and that the program names once, by offset.
// ⚠ A function another program binds or calls is named there, and no table of the project
// says so: whoever writes a body where it is bound must know the program is read alone.
pub fn sole(functions: &[tree::Function]) -> BTreeSet<u32> {
    let mut named: BTreeMap<u32, u32> = BTreeMap::new();
    let mut bodies = BTreeSet::new();
    for f in functions {
        let program = f.head.program;
        tree::each_expr(&f.body, &mut |top| {
            top.walk(&mut |e| match e {
                Expr::FunctionRef { program: of, at } if *of == program => {
                    *named.entry(*at & ir::FUNCTION_OFFSET).or_default() += 1;
                }
                Expr::Call {
                    callee: Callee::Local { at, .. },
                    ..
                } => *named.entry(*at).or_default() += 1,
                Expr::Sugar {
                    kind: ExprSugar::Lambda,
                    e: inner,
                } => {
                    if let Expr::FunctionRef { program: of, at } = &**inner {
                        if *of == program {
                            bodies.insert(*at);
                        }
                    }
                }
                _ => {}
            });
        });
    }
    bodies.retain(|at| named.get(at) == Some(&1));
    bodies
}

/// How a lambda's body is written where the lambda is bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inline<'a> {
    /// `(args) => value`: the body returns one value, or is one call where it returns nothing.
    Value(&'a Expr),
    /// `(args) => { .. }`: a few statements, without the `return;` a body ends on.
    Block(&'a [Stmt]),
}

/// The form `body` takes written as a lambda where it is bound; `None` for a body that is long, that jumps, that waits, or that writes the parameter its closure object comes in.
// The closure object is parameter 0: a body that never assigns it and never takes its address
// reads one object throughout, which the place that binds it holds in a variable of its own.
pub fn inline(body: &tree::Function) -> Option<Inline<'_>> {
    const RECEIVER: Var = Var::Param(0);
    if body.head.incomplete.is_some() || body.head.params == 0 {
        return None;
    }
    let uses = Uses::of(&body.body);
    let written = uses.assigned.contains_key(&RECEIVER) || uses.addressed.contains(&RECEIVER);
    let mut waits = false;
    tree::each_sugar(&body.body, &mut |sugar| {
        waits |= matches!(sugar, Sugar::Await { .. } | Sugar::Yield { .. });
    });
    let counts = tree::counts(&body.body);
    if written || waits || counts.gotos != 0 || counts.labels != 0 {
        return None;
    }
    let stmts = match body.body.split_last() {
        Some((Stmt::Return(None), rest)) => rest,
        _ => body.body.as_slice(),
    };
    // ⚠ A value written alone declares nothing: one that names a variable of the body's own
    // frame would read as the binder's variable of that name, so such a body keeps its block.
    let own = |value: &Expr| {
        let mut own = false;
        value.walk(&mut |e| {
            if let Expr::Var(var) | Expr::AddrOf { var, .. } = e {
                own |= matches!(var, Var::Local(_) | Var::Value { .. } | Var::Stack { .. });
            }
        });
        own
    };
    match stmts {
        [Stmt::Return(Some(value))] if !own(value) => Some(Inline::Value(value)),
        [Stmt::Simple(ir::Stmt::Eval(call))]
            if body.head.returns == Returns::Void && !own(call) =>
        {
            Some(Inline::Value(call))
        }
        _ if counts.statements <= BLOCK => Some(Inline::Block(stmts)),
        _ => None,
    }
}

/// The functions of its own program that `binder` binds as a lambda where the body may be written in place: at one place, to a holder in a global or to a closure object made outside every loop.
// ⚠ A closure object made in a loop is another object each round: a body written in place
// would read the one variable that holds the last of them. A jump back may close a loop no
// statement shows, so a function that holds one gives none.
pub fn inlinable(binder: &tree::Function) -> BTreeSet<u32> {
    let program = binder.head.program;
    if !forward_only(&binder.body) {
        return BTreeSet::new();
    }
    let mut looped: BTreeSet<Var> = BTreeSet::new();
    let mut work: Vec<(&[Stmt], bool)> = vec![(binder.body.as_slice(), false)];
    while let Some((list, within)) = work.pop() {
        for stmt in list {
            let (own, rounds): (Vec<&ir::Stmt>, bool) = match stmt {
                Stmt::Simple(s) => (vec![s], false),
                Stmt::For { init, step, .. } => {
                    (init.iter().chain(step).map(|s| &**s).collect(), true)
                }
                Stmt::While { .. } | Stmt::DoWhile { .. } => (Vec::new(), true),
                Stmt::Sugar(sugar) => {
                    let rounds = matches!(**sugar, Sugar::Foreach { .. } | Sugar::Event { .. });
                    (sugar.parts().simples, rounds)
                }
                _ => (Vec::new(), false),
            };
            if within || rounds {
                looped.extend(own.iter().flat_map(|s| s.targets()).copied());
            }
            work.extend(
                nested(stmt)
                    .into_iter()
                    .map(|body| (body, within || rounds)),
            );
        }
    }
    // Each body with the times it is bound and what it is bound to the last time.
    let mut bound: BTreeMap<u32, (u32, Option<Var>)> = BTreeMap::new();
    tree::each_expr(&binder.body, &mut |top| {
        top.walk(&mut |e| {
            let Expr::Call { args, .. } = e else {
                return;
            };
            let [_, target, Expr::Sugar {
                kind: ExprSugar::Lambda,
                e: inner,
            }] = args.as_slice()
            else {
                return;
            };
            let Expr::FunctionRef { program: of, at } = &**inner else {
                return;
            };
            if *of != program {
                return;
            }
            let held = match bare(target) {
                Expr::Var(var) => Some(*var),
                _ => None,
            };
            let entry = bound.entry(*at).or_insert((0, held));
            *entry = (entry.0 + 1, held);
        });
    });
    bound
        .into_iter()
        .filter(|(_, (times, held))| {
            let steady =
                held.is_some_and(|var| matches!(var, Var::Global { .. }) || !looped.contains(&var));
            *times == 1 && steady
        })
        .map(|(at, _)| at)
        .collect()
}

/// Whether every jump of `body` goes to a label that stands after it, and no source construct holds a jump or a label.
// Without a jump back only a loop runs a statement twice in one call: control otherwise goes on
// through the statements in the order they stand, skipping some at most. A jump to a label no
// statement shows, such as an `await`'s, is no jump this can place.
fn forward_only(body: &[Stmt]) -> bool {
    let mut passed: BTreeSet<tree::Label> = BTreeSet::new();
    let mut ahead: BTreeSet<tree::Label> = BTreeSet::new();
    // An explicit stack of the lists still being read, in the order they stand.
    let mut work: Vec<std::slice::Iter<'_, Stmt>> = vec![body.iter()];
    while let Some(list) = work.last_mut() {
        let Some(stmt) = list.next() else {
            work.pop();
            continue;
        };
        match stmt {
            Stmt::Label(label) => {
                if !passed.insert(*label) {
                    return false;
                }
                ahead.remove(label);
            }
            Stmt::Goto(label) => {
                if passed.contains(label) {
                    return false;
                }
                ahead.insert(*label);
            }
            Stmt::If { then, els, .. } => {
                work.push(els.iter());
                work.push(then.iter());
            }
            Stmt::While { body, .. } | Stmt::DoWhile { body, .. } | Stmt::For { body, .. } => {
                work.push(body.iter());
            }
            Stmt::Switch { cases, .. } => {
                work.extend(cases.iter().rev().map(|case| case.body.iter()));
            }
            Stmt::Sugar(_) => {
                let held = tree::counts(std::slice::from_ref(stmt));
                if held.gotos != 0 || held.labels != 0 {
                    return false;
                }
            }
            Stmt::Simple(_)
            | Stmt::Break
            | Stmt::Continue
            | Stmt::Return(_)
            | Stmt::Unreachable => {}
        }
    }
    ahead.is_empty()
}

/// What the construct of a delegate kept in a global says in the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kept<'a> {
    /// The global the delegate is kept in.
    pub cache: Var,
    /// The object the delegate is bound to.
    pub target: &'a Expr,
    /// The function bound, under its reading as a lambda's body.
    pub function: &'a Expr,
    /// The statement of the arm that runs where the delegate is kept that takes it, where one does.
    pub take: Option<usize>,
}

/// Reads the parts of a `Sugar::Cached`: `cache ??= target.function`, taken by one statement of `kept` or by none.
pub fn kept_delegate<'a>(test: &Expr, making: &'a [Stmt], kept: &'a [Stmt]) -> Option<Kept<'a>> {
    let (cache, _) = null_test(test)?;
    let at = (0..making.len()).find(|i| made(making, *i).is_some())?;
    let (bound, _) = made(making, at)?;
    let take = take_of(kept, cache)?;
    Some(Kept {
        cache,
        target: bound.target,
        function: bound.function,
        take,
    })
}

/// What the construct of an event's loop says in the source: `place += handler`, or `-=`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Subscription<'a> {
    /// The delegate's place, as the statement before the loop reads it.
    pub place: &'a Expr,
    pub handler: &'a Expr,
    /// Whether the handler is taken away.
    pub remove: bool,
}

/// Reads the parts of a `Sugar::Event`; `None` where they are no longer the loop of an event.
pub fn subscription<'a>(
    seed: &'a ir::Stmt,
    round: &'a [ir::Stmt],
    again: &Expr,
) -> Option<Subscription<'a>> {
    let ir::Stmt::Assign { target, value } = seed else {
        return None;
    };
    let round: Vec<&ir::Stmt> = round.iter().collect();
    let swap = exchanged(*target, &round, again)?;
    Some(Subscription {
        place: value,
        handler: swap.handler,
        remove: swap.remove,
    })
}

/// `e` under its source readings and its casts to a whole cell, which change no bit of a reference.
fn bare(e: &Expr) -> &Expr {
    let mut e = e;
    while let Expr::Sugar { e: inner, .. }
    | Expr::Cast {
        to: Scalar::U64 | Scalar::I64,
        e: inner,
    } = e
    {
        e = &**inner;
    }
    e
}

fn is_global(var: Var) -> bool {
    matches!(var, Var::Global { .. })
}

fn mentions(e: &Expr, var: Var) -> bool {
    let mut found = false;
    e.walk(&mut |e| {
        found |= matches!(e, Expr::Var(v) | Expr::AddrOf { var: v, .. } if *v == var);
    });
    found
}

/// The variable and the value of a statement that assigns one variable.
fn assignment(stmt: &Stmt) -> Option<(Var, &Expr)> {
    match stmt {
        Stmt::Simple(ir::Stmt::Assign { target, value }) => Some((*target, value)),
        _ => None,
    }
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

/// How a body uses its variables.
#[derive(Default)]
struct Uses {
    /// How often each variable is read, assigned or has its address taken.
    mentions: BTreeMap<Var, u32>,
    /// How often each variable is assigned.
    assigned: BTreeMap<Var, u32>,
    /// The variables whose address is taken.
    addressed: BTreeSet<Var>,
    /// The variable each variable that is assigned once, by a copy, is a copy of.
    copies: BTreeMap<Var, Var>,
}

impl Uses {
    fn of(body: &[Stmt]) -> Uses {
        let mut uses = Uses::default();
        tree::each_simple(body, &mut |s| uses.assigns(s));
        tree::each_expr(body, &mut |top| uses.read(top));
        uses
    }

    /// Counts the variables one statement assigns.
    fn assigns(&mut self, s: &ir::Stmt) {
        let copied = match s {
            ir::Stmt::Assign { value, .. } => match bare(value) {
                Expr::Var(from) => Some(*from),
                _ => None,
            },
            _ => None,
        };
        for target in s.targets() {
            *self.mentions.entry(*target).or_default() += 1;
            let times = self.assigned.entry(*target).or_default();
            *times += 1;
            match (*times, copied) {
                (1, Some(from)) => {
                    self.copies.insert(*target, from);
                }
                _ => {
                    self.copies.remove(target);
                }
            }
        }
    }

    /// Counts the variables one expression reads or takes the address of.
    fn read(&mut self, top: &Expr) {
        top.walk(&mut |e| match e {
            Expr::Var(var) => *self.mentions.entry(*var).or_default() += 1,
            Expr::AddrOf { var, .. } => {
                *self.mentions.entry(*var).or_default() += 1;
                self.addressed.insert(*var);
            }
            _ => {}
        });
    }

    /// Counts every mention of one statement.
    fn statement(&mut self, s: &ir::Stmt) {
        self.assigns(s);
        s.each_expr(&mut |top| self.read(top));
    }

    /// The variable `var` is assigned once as a copy of, where that one is no global and is itself assigned once at most.
    fn copy_of(&self, var: Var) -> Option<Var> {
        let from = *self.copies.get(&var)?;
        let steady = self.assigned.get(&from).copied().unwrap_or(0) <= 1;
        (steady && !matches!(from, Var::Global { .. })).then_some(from)
    }
}

/// What a first reading of a body finds for the rewrites to ask about.
#[derive(Default)]
struct Survey {
    /// For each variable the loop of an event assigns, how often such loops mention it.
    looped: BTreeMap<Var, u32>,
    /// Whether an `if` tests a global against null.
    tests: bool,
}

fn survey(body: &[Stmt], events: bool, lambdas: bool) -> Survey {
    let mut read = Survey::default();
    // An explicit stack: a body nests as deep as its listing does.
    let mut work = vec![body];
    while let Some(list) = work.pop() {
        for (i, stmt) in list.iter().enumerate() {
            if lambdas {
                read.tests |= matches!(stmt, Stmt::If { cond, .. } if null_test(cond).is_some());
            }
            let found = match events {
                true => loop_at(list, i),
                false => None,
            };
            if let Some(found) = found {
                for var in &found.targets {
                    let times = found.inner.mentions.get(var).copied().unwrap_or(0);
                    *read.looped.entry(*var).or_default() += times;
                }
            }
            work.extend(nested(stmt));
        }
    }
    read
}

/// The arguments of a call of the runtime's method `method`, of its type `ty` where one is asked.
// ⚠ By the name the call is written under, which a listing states and a compiled program has
// only where the user's list of the runtime's methods gives it: a method nothing names is read
// as nothing, whatever its shape.
fn runtime<'a>(e: &'a Expr, ty: Option<&str>, method: &str) -> Option<&'a [Expr]> {
    let Expr::Call {
        callee: Callee::Builtin { path, .. },
        args,
        ..
    } = bare(e)
    else {
        return None;
    };
    let mut names = path.rsplit('.');
    let named = names.next() == Some(method) && ty.is_none_or(|ty| names.next() == Some(ty));
    named.then_some(args.as_slice())
}

/// The delegate a call joins a handler to or takes one away from, the handler, and whether it is taken away.
fn joined(e: &Expr) -> Option<(&Expr, &Expr, bool)> {
    let (args, remove) = match runtime(e, Some("Delegate"), "Combine") {
        Some(args) => (args, false),
        None => (runtime(e, Some("Delegate"), "Remove")?, true),
    };
    match args {
        [held, handler] => Some((held, handler, remove)),
        _ => None,
    }
}

/// What one round of an event's loop does.
struct Swap<'a> {
    /// The address the exchange writes through.
    place: &'a Expr,
    handler: &'a Expr,
    remove: bool,
}

/// Reads statements as one round of an event's loop over the delegate `cur` holds as the round begins, and `again` as "another writer came between".
// `entry` are the variables that hold what `cur` held as the round began and `swapped` those
// that hold what the exchange returned. A statement is a copy among them, the join of the
// handler to what was read, or the exchange of the join for what was read; the round must leave
// in `cur` what the exchange returned, and go again while that is not what was read.
fn exchanged<'a>(cur: Var, round: &[&'a ir::Stmt], again: &Expr) -> Option<Swap<'a>> {
    if round.len() > ROUND {
        return None;
    }
    let mut entry = vec![cur];
    let mut swapped: Vec<Var> = Vec::new();
    let mut join: Option<(Var, &'a Expr, bool)> = None;
    let mut swap: Option<Swap<'a>> = None;
    for stmt in round.iter().copied() {
        let ir::Stmt::Assign { target, value } = stmt else {
            return None;
        };
        let target = *target;
        if matches!(target, Var::Global { .. }) || join.is_some_and(|(held, ..)| held == target) {
            return None;
        }
        if let Expr::Var(from) = bare(value) {
            let read = entry.contains(from);
            if !read && !swapped.contains(from) {
                return None;
            }
            entry.retain(|var| *var != target);
            swapped.retain(|var| *var != target);
            match read {
                true => entry.push(target),
                false => swapped.push(target),
            }
            continue;
        }
        if let Some((held, handler, remove)) = joined(value) {
            let read = matches!(bare(held), Expr::Var(var) if entry.contains(var));
            let fresh = !entry.contains(&target) && !swapped.contains(&target);
            if swap.is_some() || join.is_some() || !read || !fresh {
                return None;
            }
            join = Some((target, handler, remove));
            continue;
        }
        let [place, new, comparand] = runtime(value, None, "CompareExchange")? else {
            return None;
        };
        let read = matches!(bare(comparand), Expr::Var(var) if entry.contains(var));
        if swap.is_some() || !read {
            return None;
        }
        let (handler, remove) = match (bare(new), join) {
            (Expr::Var(var), Some((held, handler, remove))) if *var == held => (handler, remove),
            (_, None) => {
                let (held, handler, remove) = joined(new)?;
                if !matches!(bare(held), Expr::Var(var) if entry.contains(var)) {
                    return None;
                }
                (handler, remove)
            }
            _ => return None,
        };
        entry.retain(|var| *var != target);
        swapped.push(target);
        swap = Some(Swap {
            place,
            handler,
            remove,
        });
    }
    let swap = swap?;
    let Expr::Binary {
        op: BinOp::Ne,
        l,
        r,
        ..
    } = bare(again)
    else {
        return None;
    };
    let (Expr::Var(l), Expr::Var(r)) = (bare(l), bare(r)) else {
        return None;
    };
    let differs =
        (swapped.contains(l) && entry.contains(r)) || (swapped.contains(r) && entry.contains(l));
    (differs && swapped.contains(&cur)).then_some(swap)
}

/// A delegate read and the loop after it that exchanges it, as an event's `+=` or `-=` is compiled.
struct Loop<'a> {
    /// The address and the offset of the 64-bit load the delegate is first read by.
    read: (&'a Expr, u32),
    swap: Swap<'a>,
    /// Every variable the statements assign, each once here; each is assigned before it is read.
    targets: Vec<Var>,
    /// The mentions of variables in the statements and the test.
    inner: Uses,
}

/// The event's loop that `list[i]` and the statement after it are, by their shape alone.
fn loop_at(list: &[Stmt], i: usize) -> Option<Loop<'_>> {
    let (Stmt::Simple(seed), Stmt::DoWhile { body, cond }) = (list.get(i)?, list.get(i + 1)?)
    else {
        return None;
    };
    let ir::Stmt::Assign { target: cur, value } = seed else {
        return None;
    };
    let Expr::Load {
        ty: Scalar::U64,
        addr,
        offset,
    } = bare(value)
    else {
        return None;
    };
    if body.len() > ROUND {
        return None;
    }
    let round: Vec<&ir::Stmt> = body
        .iter()
        .map(|stmt| match stmt {
            Stmt::Simple(s) => Some(s),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let swap = exchanged(*cur, &round, cond)?;
    let addr: &Expr = addr;
    let mut inner = Uses::default();
    inner.statement(seed);
    let mut targets = vec![*cur];
    for stmt in round.iter().copied() {
        inner.statement(stmt);
        targets.extend(stmt.targets());
    }
    inner.read(cond);
    targets.sort();
    targets.dedup();
    Some(Loop {
        read: (addr, *offset),
        swap,
        targets,
        inner,
    })
}

/// Whether, in the few statements before `list[i]`, one of two variables was made a copy of the other and neither was assigned since.
fn copied_before(list: &[Stmt], i: usize, a: Var, b: Var) -> bool {
    for stmt in list.iter().take(i).rev().take(COPIES) {
        let Some((target, value)) = assignment(stmt) else {
            return false;
        };
        if target == a || target == b {
            let other = match target == a {
                true => b,
                false => a,
            };
            return *bare(value) == Expr::Var(other);
        }
    }
    false
}

/// Whether `list[i]` and the loop after it are an event's `+=` or `-=` that may be read as one statement.
// The loop ends with the handler joined to whatever the place then held, whatever was read
// first. What must hold besides the shape: the read and the exchange name one field of one
// object; the object and the handler are variables nothing in the loop can change, since the
// statement reads each once and the loop each round; and every variable the loop assigns is
// mentioned only in loops of this shape, each of which assigns it before reading it.
fn event_at(list: &[Stmt], i: usize, uses: &Uses, looped: &BTreeMap<Var, u32>) -> bool {
    let Some(found) = loop_at(list, i) else {
        return false;
    };
    let Expr::ByteOffset {
        addr: written,
        offset: by,
    } = bare(found.swap.place)
    else {
        return false;
    };
    let (Expr::Var(read), Expr::Var(base), Expr::Var(handler)) =
        (bare(found.read.0), bare(written), bare(found.swap.handler))
    else {
        return false;
    };
    let field = matches!(bare(by), Expr::Const(c) if c.bits == u64::from(found.read.1));
    let object = read == base
        || uses.copy_of(*base) == Some(*read)
        || uses.copy_of(*read) == Some(*base)
        || copied_before(list, i, *base, *read);
    let private = found
        .targets
        .iter()
        .all(|var| !matches!(var, Var::Global { .. }) && uses.mentions.get(var) == looped.get(var));
    let steady = [base, handler].into_iter().all(|var| {
        !matches!(var, Var::Global { .. })
            && !found.targets.contains(var)
            && !uses.addressed.contains(var)
    });
    field && object && private && steady
}

/// Puts each event's loop of a list and the statement that reads the delegate before it into one construct, and counts them.
// One pass that rebuilds the list: a function subscribes to hundreds of events.
fn events_in(list: &mut Vec<Stmt>, uses: &Uses, looped: &BTreeMap<Var, u32>) -> usize {
    let at: Vec<usize> = (0..list.len())
        .filter(|i| event_at(list, *i, uses, looped))
        .collect();
    if at.is_empty() {
        return 0;
    }
    let mut count = 0;
    let old = std::mem::take(list);
    list.reserve(old.len());
    let mut starts = at.into_iter().peekable();
    let mut rest = old.into_iter().enumerate();
    while let Some((i, stmt)) = rest.next() {
        if starts.peek() != Some(&i) {
            list.push(stmt);
            continue;
        }
        starts.next();
        let after = rest.next().map(|(_, stmt)| stmt);
        match (stmt, after) {
            (Stmt::Simple(seed), Some(Stmt::DoWhile { body, cond }))
                if body.iter().all(|stmt| matches!(stmt, Stmt::Simple(_))) =>
            {
                let round = body
                    .into_iter()
                    .filter_map(|stmt| match stmt {
                        Stmt::Simple(s) => Some(s),
                        _ => None,
                    })
                    .collect();
                list.push(Stmt::Sugar(Box::new(Sugar::Event {
                    seed: Box::new(seed),
                    round,
                    again: cond,
                })));
                count += 1;
            }
            (stmt, after) => {
                list.push(stmt);
                list.extend(after);
            }
        }
    }
    count
}

/// What `alloc` and `ctor` make where they are a delegate made and bound.
// No name is asked: an object of a builtin type, handed with a target and a function reference
// to a builtin of that same type, is what only the runtime's delegate is made by.
fn delegate<'a>(alloc: &ir::Stmt, ctor: &'a ir::Stmt) -> Option<Made<'a>> {
    let ir::Stmt::Assign {
        target: held,
        value,
    } = alloc
    else {
        return None;
    };
    let Expr::NewObject(ty) = bare(value) else {
        return None;
    };
    let ir::Stmt::Eval(call) = ctor else {
        return None;
    };
    let Expr::Call {
        callee: Callee::Builtin { ty: builtin, .. },
        args,
        ..
    } = bare(call)
    else {
        return None;
    };
    let [object, target, function] = args.as_slice() else {
        return None;
    };
    let own = ty.id & BUILTIN != 0 && ty.id & !BUILTIN == *builtin;
    let reference = matches!(
        bare(function),
        Expr::FunctionRef { .. }
            | Expr::BuiltinFunctionRef { .. }
            | Expr::VirtualFunctionRef { .. }
    );
    // A target that reads the delegate would read it half made in the one-statement form.
    let again = mentions(target, *held) || mentions(function, *held);
    let whole = own && reference && !again && *bare(object) == Expr::Var(*held);
    whole.then_some(Made {
        delegate: *held,
        target,
        function,
    })
}

/// The delegate made at `list[i]` and the statements of the list it takes: two side by side, or one group that holds both.
fn made(list: &[Stmt], i: usize) -> Option<(Made<'_>, usize)> {
    match (list.get(i)?, list.get(i + 1)) {
        (Stmt::Simple(alloc), Some(Stmt::Simple(ctor))) => delegate(alloc, ctor).map(|m| (m, 2)),
        (Stmt::Sugar(sugar), _) => match &**sugar {
            Sugar::Group {
                kind: Group::Construct | Group::Delegate,
                parts,
            } => match parts.as_slice() {
                [alloc, ctor] => delegate(alloc, ctor).map(|m| (m, 1)),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// Puts each delegate made by one statement and bound by the next into one group, and counts them.
// One pass that rebuilds the list: a function holds thousands of them.
fn grouped(list: &mut Vec<Stmt>) -> usize {
    let mut count = 0;
    let old = std::mem::take(list);
    list.reserve(old.len());
    let mut rest = old.into_iter().peekable();
    while let Some(stmt) = rest.next() {
        let pair = match (&stmt, rest.peek()) {
            (Stmt::Simple(alloc), Some(Stmt::Simple(ctor))) => delegate(alloc, ctor).is_some(),
            _ => false,
        };
        let bound = match pair {
            true => rest.next(),
            false => None,
        };
        match (stmt, bound) {
            (Stmt::Simple(alloc), Some(Stmt::Simple(ctor))) => {
                count += 1;
                list.push(Stmt::Sugar(Box::new(Sugar::Group {
                    kind: Group::Delegate,
                    parts: vec![alloc, ctor],
                })));
            }
            (mut stmt, bound) => {
                count += usize::from(regrouped(&mut stmt));
                list.push(stmt);
                list.extend(bound);
            }
        }
    }
    count
}

/// Reads a pair the constructors pass gathered as the delegate it is; `false` where `stmt` is no such pair.
fn regrouped(stmt: &mut Stmt) -> bool {
    let Stmt::Sugar(sugar) = stmt else {
        return false;
    };
    let Sugar::Group { kind, parts } = &mut **sugar else {
        return false;
    };
    let pair = match parts.as_slice() {
        [alloc, ctor] => *kind == Group::Construct && delegate(alloc, ctor).is_some(),
        _ => false,
    };
    if pair {
        *kind = Group::Delegate;
    }
    pair
}

/// The variables that hold a closure object: assigned once in the body, an object of a class the compiler made, the address never taken.
pub(crate) fn closures(body: &[Stmt]) -> BTreeSet<Var> {
    let mut assigned: BTreeMap<Var, u32> = BTreeMap::new();
    tree::each_simple(body, &mut |s| {
        let ir::Stmt::Assign { target, value } = s else {
            return;
        };
        let class = matches!(
            bare(value),
            Expr::NewObject(ty) if ty.flags & GENERATED != 0 && ty.flags & VALUE == 0
        );
        // A parameter and a global hold a value before any statement runs.
        if class && !matches!(target, Var::Param(_) | Var::Global { .. }) {
            assigned.insert(*target, 0);
        }
    });
    if assigned.is_empty() {
        return BTreeSet::new();
    }
    tree::each_simple(body, &mut |s| {
        for target in s.targets() {
            if let Some(times) = assigned.get_mut(target) {
                *times += 1;
            }
        }
    });
    tree::each_expr(body, &mut |top| {
        top.walk(&mut |e| {
            if let Expr::AddrOf { var, .. } = e {
                assigned.remove(var);
            }
        });
    });
    assigned
        .into_iter()
        .filter(|(_, times)| *times == 1)
        .map(|(var, _)| var)
        .collect()
}

/// Reads the function bound by the delegate made at `list[i]` as a lambda's body; `false` where it is read so already or is no function of a program.
fn read_as_lambda(list: &mut [Stmt], i: usize) -> bool {
    // The constructor call is the next statement, or the second part of the group at `i`.
    let at = match list.get(i) {
        Some(Stmt::Sugar(_)) => i,
        _ => i + 1,
    };
    let ctor = match list.get_mut(at) {
        Some(Stmt::Simple(ctor)) => ctor,
        Some(Stmt::Sugar(sugar)) => match &mut **sugar {
            Sugar::Group { parts, .. } if parts.len() == 2 => &mut parts[1],
            _ => return false,
        },
        _ => return false,
    };
    let ir::Stmt::Eval(call) = ctor else {
        return false;
    };
    let mut call = call;
    while let Expr::Sugar { e: inner, .. } = call {
        call = &mut **inner;
    }
    let Expr::Call { args, .. } = call else {
        return false;
    };
    let Some(function) = args.get_mut(2) else {
        return false;
    };
    if !matches!(function, Expr::FunctionRef { .. }) {
        return false;
    }
    let bound = std::mem::replace(function, Expr::GuidEmpty);
    *function = Expr::sugar(ExprSugar::Lambda, bound);
    true
}

/// The global a condition tests against null, and whether the condition holds where the global is null.
fn null_test(cond: &Expr) -> Option<(Var, bool)> {
    let (mut e, mut negated) = (bare(cond), false);
    while let Expr::Unary {
        op: UnOp::Not,
        e: inner,
    } = e
    {
        e = bare(inner);
        negated = !negated;
    }
    let Expr::Binary { op, l, r, .. } = e else {
        return None;
    };
    let null = match op {
        BinOp::Eq => true,
        BinOp::Ne => false,
        _ => return None,
    };
    match (bare(l), bare(r)) {
        (Expr::Var(global @ Var::Global { .. }), Expr::Const(zero)) if zero.bits == 0 => {
            Some((*global, null != negated))
        }
        _ => None,
    }
}

/// Whether a statement only carries a value to where the arms of an `if` meet: it assigns a variable that is no global a value that calls and makes nothing.
fn carried(stmt: &Stmt) -> bool {
    let Stmt::Simple(ir::Stmt::Assign { target, value }) = stmt else {
        return false;
    };
    let mut quiet = !matches!(target, Var::Global { .. });
    value.walk(&mut |e| {
        quiet &= !matches!(
            e,
            Expr::Call { .. } | Expr::NewObject(_) | Expr::NewArray { .. }
        );
    });
    quiet
}

/// Where the arm that runs while the global `cache` is null makes the delegate it keeps there, when that is all the two arms do.
// The compiler's own form for a lambda that captures nothing: bound to a holder kept in a
// global, made once and kept in `cache`. The arms may carry values to where they meet, as the
// stack does across a branch; anything else leaves the function a method someone wrote a
// cache for.
fn cached(cache: Var, made_in: &[Stmt], kept_in: &[Stmt], program: u32) -> Option<usize> {
    let at = (0..made_in.len()).find(|i| made(made_in, *i).is_some())?;
    let (bound, taken) = made(made_in, at)?;
    let holder = matches!(
        bare(bound.target),
        Expr::Var(held @ Var::Global { .. }) if *held != cache
    );
    let own = matches!(
        bare(bound.function),
        Expr::FunctionRef { program: of, .. } if *of == program
    );
    let (kept, after) = made_in.get(at + taken..)?.split_first()?;
    let keeps = matches!(
        kept,
        Stmt::Simple(ir::Stmt::Assign { target, value })
            if *target == cache && *bare(value) == Expr::Var(bound.delegate)
    );
    let only = made_in[..at]
        .iter()
        .chain(after)
        .chain(kept_in)
        .all(carried);
    (holder && own && keeps && only).then_some(at)
}

/// Where the one statement of `kept` that takes what the global `cache` holds stands, where one does; `None` where two do.
fn take_of(kept: &[Stmt], cache: Var) -> Option<Option<usize>> {
    let takes =
        |stmt: &Stmt| assignment(stmt).is_some_and(|(_, value)| *bare(value) == Expr::Var(cache));
    let mut taking = kept.iter().enumerate().filter(|(_, stmt)| takes(stmt));
    let take = taking.next().map(|(i, _)| i);
    taking.next().is_none().then_some(take)
}

/// Whether a value reads the frame alone: no memory behind an address, no global, nothing the runtime owns, and no call.
fn frame_only(e: &Expr) -> bool {
    let mut only = true;
    e.walk(&mut |e| {
        only &= !matches!(
            e,
            Expr::Var(Var::Global { .. })
                | Expr::AddrOf {
                    var: Var::Global { .. },
                    ..
                }
                | Expr::Call { .. }
                | Expr::Load { .. }
                | Expr::LoadBytes { .. }
                | Expr::NewObject(_)
                | Expr::NewArray { .. }
                | Expr::BuiltinGlobal(_)
                | Expr::VirtualFunctionRef { .. }
        );
    });
    only
}

/// Whether the two arms [`cached`] accepted differ by the delegate alone, so that they read as `cache ??= delegate`, taken by one statement or by none.
// Read so, what the arms carry is written once, in the order of the arm that takes the kept
// delegate, with the making where the take stands. Both arms must then carry the same
// statements in the same order, none of which names the cache, the delegate or the variable
// that takes it; and a statement that the arm which makes the delegate ran on the other side of
// the making may read the frame alone, which the making does not write. The delegate's own
// variable is not written at all, so nothing else may mention it.
fn kept_whole(cache: Var, made_in: &[Stmt], kept_in: &[Stmt], at: usize, uses: &Uses) -> bool {
    let Some((bound, taken)) = made(made_in, at) else {
        return false;
    };
    let delegate = bound.delegate;
    let Some(take) = take_of(kept_in, cache) else {
        return false;
    };
    if is_global(delegate) {
        return false;
    }
    let join = take
        .and_then(|i| kept_in.get(i))
        .and_then(assignment)
        .map(|(target, _)| target);
    let copies = |stmt: &Stmt| {
        assignment(stmt).is_some_and(|(target, value)| {
            Some(target) == join && *bare(value) == Expr::Var(delegate)
        })
    };
    let before = made_in.get(..at).unwrap_or_default();
    let after = made_in.get(at + taken + 1..).unwrap_or_default();
    // The copy to where the arms meet stands after the delegate is kept, once, unless the
    // delegate's own variable is what takes it.
    let copied = usize::from(join.is_some_and(|join| join != delegate));
    if after.iter().filter(|stmt| copies(stmt)).count() != copied {
        return false;
    }
    let in_made: Vec<&Stmt> = before
        .iter()
        .chain(after.iter().filter(|stmt| !copies(stmt)))
        .collect();
    let in_kept: Vec<&Stmt> = kept_in
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != take)
        .map(|(_, stmt)| stmt)
        .collect();
    let named = [Some(cache), Some(delegate), join];
    let apart = in_kept.iter().all(|stmt| {
        assignment(stmt).is_some_and(|(target, value)| {
            !named.contains(&Some(target))
                && named.iter().flatten().all(|var| !mentions(value, *var))
        })
    });
    if in_made != in_kept || !apart {
        return false;
    }
    let (written, ran) = (take.unwrap_or(0), before.len());
    let moved = in_kept
        .get(written.min(ran)..written.max(ran))
        .unwrap_or_default();
    let frame = moved
        .iter()
        .all(|stmt| assignment(stmt).is_some_and(|(_, value)| frame_only(value)));
    let private = join == Some(delegate)
        || uses.mentions.get(&delegate) == Uses::of(made_in).mentions.get(&delegate);
    frame && private
}

/// Gives the variable of a lambda's own that goes by the name of the object its binder binds it to the name followed by the lambda's offset, where that name alone keeps the body from being written where it is bound.
// ⚠ Written in place a body reads its object by the binder's variable, and one of its own of that
// name would hide it. Only a variable nothing named is renamed, and only where the body is then
// written in place: every writer of the unit, the read-back's among them, reads it renamed.
pub(crate) fn renamed_in_place(unit: &mut Unit, cx: &Context<'_>) {
    if !(cx.options.on(Pass::Lambdas) && cx.options.on(Pass::Delegates)) {
        return;
    }
    let renames = renames(unit, cx);
    if renames.is_empty() {
        return;
    }
    // An explicit stack: classes nest as deep as the closures they were made for.
    let mut open: Vec<&mut Class> = unit.classes.iter_mut().collect();
    while let Some(class) = open.pop() {
        if class.kind == ClassKind::Closure {
            for method in class.methods.iter_mut() {
                let head = &mut method.function.head;
                let Some((var, name)) = renames.get(&head.at) else {
                    continue;
                };
                if let Some(info) = head.vars.get_mut(var) {
                    info.name = Some(name.clone());
                }
            }
        }
        open.extend(class.nested.iter_mut());
    }
}

/// The lambdas of `unit` [`renamed_in_place`] renames a variable of, by offset, with the variable and its name.
fn renames(unit: &Unit, cx: &Context<'_>) -> BTreeMap<u32, (Var, String)> {
    let program = unit.program;
    let mut all: Vec<&Method> = unit.functions.iter().collect();
    let mut made: Vec<&Method> = Vec::new();
    let mut open: Vec<&Class> = unit.classes.iter().collect();
    while let Some(class) = open.pop() {
        all.extend(&class.methods);
        if class.kind == ClassKind::Closure {
            let lambdas = class.methods.iter();
            made.extend(lambdas.filter(|m| m.role == Role::Lambda && !m.is_async));
        }
        open.extend(&class.nested);
    }
    let mut found = BTreeMap::new();
    if made.is_empty() {
        return found;
    }
    // How often the unit names each of its functions, and where a function that is no lambda
    // binds one to an object.
    let mut named: BTreeMap<u32, u32> = BTreeMap::new();
    let mut sites: BTreeMap<u32, Vec<(&tree::Function, Var)>> = BTreeMap::new();
    for method in all.iter().copied() {
        let f = &method.function;
        tree::each_expr(&f.body, &mut |top| {
            top.walk(&mut |e| match e {
                Expr::FunctionRef { program: of, at } if *of == program => {
                    *named.entry(*at & ir::FUNCTION_OFFSET).or_default() += 1;
                }
                Expr::Call {
                    callee: Callee::Local { at, .. },
                    ..
                } => *named.entry(*at).or_default() += 1,
                _ => {}
            });
        });
        if method.role == Role::Lambda {
            continue;
        }
        tree::each_sugar(&f.body, &mut |sugar| {
            if let Sugar::Group {
                kind: Group::Delegate,
                parts,
            } = sugar
            {
                if let Some((object, at)) = bound_to_object(parts, program) {
                    sites.entry(at).or_default().push((f, object));
                }
            }
        });
    }
    for method in made {
        let body = &method.function;
        let at = body.head.at;
        let site = match sites.get(&at).map(Vec::as_slice) {
            Some([site]) => *site,
            _ => continue,
        };
        if named.get(&at) != Some(&1) || inline(body).is_none() {
            continue;
        }
        if let Some(renamed) = rename(site, body, cx) {
            found.insert(at, renamed);
        }
    }
    found
}

/// The variable a delegate `parts` make is bound to, where it is one of the frame, and the offset of the function of `program` bound as a lambda's body.
fn bound_to_object(parts: &[ir::Stmt], program: u32) -> Option<(Var, u32)> {
    let [ir::Stmt::Assign { target, value }, ir::Stmt::Eval(call)] = parts else {
        return None;
    };
    if !matches!(bare(value), Expr::NewObject(_)) {
        return None;
    }
    let Expr::Call { args, .. } = bare(call) else {
        return None;
    };
    let [made, held, Expr::Sugar {
        kind: ExprSugar::Lambda,
        e: function,
    }] = args.as_slice()
    else {
        return None;
    };
    let Expr::FunctionRef { program: of, at } = &**function else {
        return None;
    };
    let (Expr::Var(made), Expr::Var(object)) = (bare(made), bare(held)) else {
        return None;
    };
    let frame = matches!(
        object,
        Var::Local(_) | Var::Value { .. } | Var::Stack { .. }
    );
    (made == target && *of == program && frame).then_some((*object, *at))
}

/// The variable of `body`'s own that goes by the name of the object `site` binds it to, with the name it is given, where the body is then written in place there.
fn rename(
    site: (&tree::Function, Var),
    body: &tree::Function,
    cx: &Context<'_>,
) -> Option<(Var, String)> {
    let (binder, object) = site;
    let text = spoken(&binder.head, object);
    let own = own_vars(body);
    let mut clashing = own.iter().filter(|var| spoken(&body.head, **var) == text);
    let var = *clashing.next()?;
    if clashing.next().is_some() {
        return None;
    }
    // ⚠ A name a list or a pass gave is the variable's: such a body stays where it is.
    let info = body.head.vars.get(&var)?;
    if info.name.is_some() {
        return None;
    }
    let name = format!("{text}_{:06X}", body.head.at);
    let mut taken: BTreeSet<String> = own.iter().map(|v| spoken(&body.head, *v)).collect();
    let mut theirs: BTreeSet<Var> = (0..binder.head.params).map(Var::Param).collect();
    theirs.extend(binder.head.vars.keys().copied());
    theirs.extend(own_vars(binder));
    taken.extend(theirs.iter().map(|v| spoken(&binder.head, *v)));
    taken.insert("this".to_owned());
    if taken.contains(&name) {
        return None;
    }
    let mut renamed = body.clone();
    if let Some(info) = renamed.head.vars.get_mut(&var) {
        info.name = Some(name.clone());
    }
    let mut one = print::Lambdas::new();
    one.insert(body.head.at, &renamed);
    let sited = print::bound_in_place(binder, &one, cx);
    sited.contains(&body.head.at).then_some((var, name))
}

/// The name a variable of a function is written under: the one its head gives it, else its slot's.
fn spoken(head: &ir::Head, var: Var) -> String {
    match head.vars.get(&var).and_then(|info| info.name.as_deref()) {
        Some(name) => name.to_owned(),
        None => var.to_string(),
    }
}

/// The variables a body names of its own: its parameters but the receiver, and every other one it mentions but a global.
fn own_vars(f: &tree::Function) -> BTreeSet<Var> {
    let mut own: BTreeSet<Var> = (1..f.head.params).map(Var::Param).collect();
    tree::each_simple(&f.body, &mut |s| own.extend(s.targets().iter().copied()));
    tree::each_expr(&f.body, &mut |top| {
        top.walk(&mut |e| {
            if let Expr::Var(var) | Expr::AddrOf { var, .. } = e {
                own.insert(*var);
            }
        });
    });
    own.retain(|var| *var != Var::Param(0) && !is_global(*var));
    own
}

#[cfg(test)]
#[path = "closures_tests.rs"]
mod tests;
