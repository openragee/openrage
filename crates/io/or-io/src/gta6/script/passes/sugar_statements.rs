//! Sugar for statements: the loops and blocks a compiler expands, read back as `foreach`, `for`,
//! `using`, `lock` and `try`.
//!
//! Works on the structured form, after `async_await` and `closures`. A construct is made only
//! where every statement of the expansion is found in its place, and holds them all, so that
//! `tree::desugar` gives back the code it replaced.
//!
//! ⚠ An enumerator held in place, made by one script function and advanced by another, is a
//! `foreach` only where the project proves the advance a method of a value type that, like the
//! make, touches the enumerator alone (`enumerators`). A `try` is made of the regions a front end
//! states beside a function, by [`protected`], and never by [`run`].

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use crate::gta6::script::class::Role;
use crate::gta6::script::ir::{
    self, BinOp, Callee, Expr, FunctionKey, Head, Kind, Scalar, TypeKey, Var, VarInfo, Width,
};
use crate::gta6::script::tree::{self, Stmt, Sugar};
use crate::gta6::script::{Context, Pass, Stats};

#[path = "sugar_statements_guards.rs"]
mod guards;

#[path = "sugar_statements_enumerators.rs"]
mod enumerators;

pub use enumerators::enumerated;
pub use guards::protected;

/// The names this file's rewrites are switched by.
pub const PASSES: &[Pass] = &[Pass::Foreach, Pass::Using, Pass::Lock, Pass::Try, Pass::For];

/// The loops of one function that are each given an item no variable held.
// ⚠ An item is told from the next by seven bits of its index where a function is run to check a pass.
const ITEMS: u32 = 128;

/// The deepest that statements nest where an element read in place is looked for.
const DEPTH: u32 = 64;

/// The most cells an element read in place may yield and still be taken as an item.
const CELLS: u32 = 64;

/// The loads a path to an array is followed through before it is taken to be none.
const LOADS: usize = 16;

/// What reading one function's loops and blocks came to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Found {
    /// Counted loops over an array read as `foreach`.
    pub arrays: usize,
    /// Enumerator loops read as `foreach`, their disposal hidden.
    pub enumerators: usize,
    /// Loops over an enumerator held in place read as `foreach`.
    pub held: usize,
    /// Loops with the head of one of those that a safety condition left as they were.
    pub refused: usize,
    /// Those of `arrays` whose element was read in place, and is now an item made for it.
    pub folded: usize,
    /// `while` loops read as `for`.
    pub counted: usize,
    /// Runs of statements read as `using`.
    pub usings: usize,
    /// Runs of statements read as `lock`.
    pub locks: usize,
}

/// Reads each proved expansion as the statement of the source it came from.
pub fn run(f: &mut tree::Function, cx: &Context<'_>, _stats: &mut Stats) {
    rewrite(f, cx);
}

/// [`run`], telling what it read: `Stats` has no place for it.
pub fn rewrite(f: &mut tree::Function, cx: &Context<'_>) -> Found {
    let mut found = Found::default();
    // First, and over the whole body: a loop read as a `for` here may be a `foreach` below.
    if cx.options.on(Pass::For) {
        found.counted = guards::counted_loops(&mut f.body);
    }
    let foreach = cx.options.on(Pass::Foreach);
    let guarded = cx.options.any(&[Pass::Using, Pass::Lock]);
    if !foreach && !guarded {
        return found;
    }
    let scope = Scope::of(&f.head, &f.body, cx);
    let mut items: Vec<(Var, VarInfo)> = Vec::new();
    // An explicit stack: a body nests as deep as its listing does.
    let mut work: Vec<&mut Vec<Stmt>> = vec![&mut f.body];
    while let Some(list) = work.pop() {
        if foreach {
            loops(list, &scope, &mut items, &mut found);
        }
        // After the loops: the disposal a `foreach` hides is no `using` of its enumerator.
        if guarded {
            guards::guards(list, &scope, &mut found);
        }
        // What was just read is gone into too: the loops and blocks inside it are read in their turn.
        for stmt in list {
            work.extend(tree::nested_mut(stmt));
        }
    }
    f.head.vars.extend(items);
    found
}

/// The name the item made for a loop is written under: `item`, or `item_2` and on while a variable of the function has it.
// ⚠ No instruction made this variable: under the slot name of a value it would read as one of the lifter's temporaries.
fn item_name(head: &Head, items: &[(Var, VarInfo)]) -> String {
    let held = |name: &str| {
        let has = |info: &VarInfo| info.name.as_deref() == Some(name);
        head.vars.values().any(has) || items.iter().any(|(_, info)| has(info))
    };
    let mut name = "item".to_owned();
    let mut n = 1;
    while held(&name) {
        n += 1;
        name = format!("item_{n}");
    }
    name
}

/// Reads the loops that stand in `list` itself.
fn loops(
    list: &mut Vec<Stmt>,
    scope: &Scope<'_>,
    items: &mut Vec<(Var, VarInfo)>,
    found: &mut Found,
) {
    let cx = &scope.cx;
    let mut at = 0;
    while at < list.len() {
        let counted =
            matches!(&list[at], Stmt::For { cond: Some(cond), .. } if below_length(cond).is_some());
        let advanced = match &list[at] {
            Stmt::While { cond, .. } => called_on(tested(cond), Member::MoveNext, cx),
            _ => None,
        };
        if counted {
            match array_loop(&list[at]).filter(|read| scope.proves(read)) {
                Some(read) if read.item.is_some() => {
                    let taken = std::mem::replace(&mut list[at], Stmt::Break);
                    list[at] = array_foreach(taken);
                    found.arrays += 1;
                }
                Some(read) => {
                    let made = scope.item_var(items).and_then(|item| {
                        let info = folded_foreach(&mut list[at], &read, item, scope.head)?;
                        Some((item, info))
                    });
                    match made {
                        Some((item, mut info)) => {
                            info.name = Some(item_name(scope.head, items));
                            items.push((item, info));
                            found.arrays += 1;
                            found.folded += 1;
                        }
                        None => found.refused += 1,
                    }
                }
                None => found.refused += 1,
            }
        } else if let Some(e) = advanced {
            match enumerator_foreach(list.as_slice(), at, e, scope) {
                Some((made, range)) => {
                    list.drain(range.start + 1..range.end);
                    list[range.start] = made;
                    at = range.start;
                    found.enumerators += 1;
                }
                None => found.refused += 1,
            }
        } else if let Some(shape) = enumerators::shape(list.as_slice(), at, scope.head.program) {
            let made = match enumerators::proved(list.as_slice(), at, &shape, scope) {
                true => enumerators::fold(list, at),
                false => None,
            };
            match made {
                Some(first) => {
                    at = first;
                    found.held += 1;
                }
                None => found.refused += 1,
            }
        }
        at += 1;
    }
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
    /// The variables an address of which is taken and not only read through at once.
    addressed: Vec<Var>,
    /// Each taking of an address, by the variable taken.
    addresses: BTreeMap<Var, Vec<(u32, enumerators::Taken)>>,
    /// The uses of each variable that the loops [`array_loop`] takes it is the index of make, all together.
    indexed: BTreeMap<Var, Use>,
    /// The loops over an enumerator held in place that each variable is the enumerator of.
    enumerated: BTreeMap<Var, usize>,
    /// The make, the advance and the read of the item that those loops take, by the value type the advance is a method of.
    pairs: BTreeMap<TypeKey, BTreeSet<(FunctionKey, FunctionKey, enumerators::Read)>>,
}

impl<'a> Scope<'a> {
    fn of(head: &'a Head, body: &[Stmt], cx: &Context<'a>) -> Scope<'a> {
        let uses = used(body);
        let addresses = enumerators::addresses(body, head);
        // ⚠ An address only ever loaded through changes nothing it points at.
        let addressed = (addresses.iter())
            .filter(|(_, takings)| {
                (takings.iter()).any(|(_, taken)| *taken != enumerators::Taken::Loaded)
            })
            .map(|(var, _)| *var)
            .collect();
        let mut indexed: BTreeMap<Var, Use> = BTreeMap::new();
        let mut enumerated: BTreeMap<Var, usize> = BTreeMap::new();
        let mut pairs: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
        let mut work = vec![body];
        while let Some(list) = work.pop() {
            for (at, stmt) in list.iter().enumerate() {
                if let Some(read) = array_loop(stmt) {
                    // Set by the head and stepped; read by the test, the step and the body's reads.
                    let only = indexed.entry(read.index).or_default();
                    only.assigned += 2;
                    only.read += 2 + read.index_reads;
                }
                if let Some(shape) = enumerators::shape(list, at, head.program) {
                    *enumerated.entry(shape.held()).or_default() += 1;
                    let ty = enumerators::owner(cx.project, shape.advance(), &[Role::Method]);
                    if let Some(ty) = ty {
                        pairs.entry(ty).or_default().insert(shape.pair());
                    }
                }
                work.extend(nested(stmt));
            }
        }
        Scope {
            head,
            cx: *cx,
            uses,
            addressed,
            addresses,
            indexed,
            enumerated,
            pairs,
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

    /// Whether the function names the index of an [`array_loop`] nowhere but in the heads of such loops and the reads of their elements, and changes none of its three variables behind their names.
    // ⚠ A slot is the index of several loops in turn, so the count is over all of them: each
    // sets the index before it reads it, and what is read after a loop is then never its index.
    fn proves(&self, read: &Counted) -> bool {
        let (index, array) = (read.index, read.array.base);
        let only = self.indexed.get(&index).copied().unwrap_or_default();
        let apart = read
            .item
            .is_none_or(|item| !self.covers(item, index) && !self.covers(item, array));
        let named_only = ![Some(index), Some(array), read.item]
            .into_iter()
            .flatten()
            .any(|var| self.exposed(var));
        self.uses.get(&index) == Some(&only) && apart && named_only
    }

    /// A variable for the item of a loop that kept none: a value at the function's own offset that nothing names, `taken` included.
    // ⚠ No slot of the frame: a callee given the address of a local reaches the locals after it,
    // and would reach an item held in one.
    fn item_var(&self, taken: &[(Var, VarInfo)]) -> Option<Var> {
        let at = self.head.at;
        (0..ITEMS)
            .map(|index| Var::Value { at, index })
            .find(|var| {
                let named = self.head.vars.contains_key(var) || self.uses.contains_key(var);
                !named && !taken.iter().any(|(held, _)| held == var)
            })
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

/// What an array is reached by: a variable, or the fields read one from another starting at one.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Path {
    base: Var,
    /// Each load from what the one before it gave, the first from `base`: what it reads and at which byte.
    loads: Vec<(Scalar, u32)>,
}

impl Path {
    /// The variable that is the array itself, where no field lies between.
    fn var(&self) -> Option<Var> {
        self.loads.is_empty().then_some(self.base)
    }
}

/// The path `e` is, under the source readings and the casts to 64 bits at each step of it.
fn path_of(e: &Expr) -> Option<Path> {
    let mut loads = Vec::new();
    let mut e = under(e, false);
    loop {
        match e {
            Expr::Var(base) => {
                loads.reverse();
                return Some(Path { base: *base, loads });
            }
            Expr::Load { ty, addr, offset } if loads.len() < LOADS => {
                loads.push((*ty, *offset));
                e = under(addr, false);
            }
            _ => return None,
        }
    }
}

/// The index and the array of `cond` where it is `i < array_length(a)`.
fn below_length(cond: &Expr) -> Option<(Var, Path)> {
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
    Some((var_of(l, true)?, path_of(array)?))
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

/// The variables of a counted loop over an array.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Counted {
    index: Var,
    array: Path,
    /// The variable the body opens by taking the element in; `None` where it reads the element in place.
    item: Option<Var>,
    /// How often the body reads the index: once in each read of the element, and nowhere else.
    index_reads: usize,
}

impl Counted {
    /// Whether `e` is the read of the element at the index, where the cells it yields are stated.
    // ⚠ A listing states no cells of `array_get_value`: an item of a width guessed would be no item.
    fn reads(&self, e: &Expr) -> bool {
        let Expr::Call {
            callee: Callee::Intrinsic { name, .. },
            args,
            returns,
        } = e
        else {
            return false;
        };
        let [array, at] = args.as_slice() else {
            return false;
        };
        let stated = match name.strip_prefix("array_get_") {
            Some("value") => returns.is_some_and(|cells| (1..=CELLS).contains(&cells)),
            Some(ty) => Scalar::from_name(ty).is_some(),
            None => false,
        };
        stated
            && var_of(at, true) == Some(self.index)
            && path_of(array).as_ref() == Some(&self.array)
    }
}

/// The variables of `stmt` where it is `for (i = 0; i < array_length(a); i++) { .. }` and its body either opens with `x = a[i];` and then names `i` nowhere and assigns neither `a` nor `x`, or names `i` only in reads of `a[i]`, all alike, that nothing able to change the element comes before.
// ⚠ The element's address taken in place of its value is no item: a write through it changes
// the array, which a `foreach` over copies would not. An array reached through fields is read
// again by each round's test: it is the one array gone through only where nothing in the body
// may store or call.
fn array_loop(stmt: &Stmt) -> Option<Counted> {
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
    let starts =
        matches!(&**init, ir::Stmt::Assign { target, value } if *target == i && is_const(value, 0));
    if !starts || i == a.base || stepped_up(step) != Some(i) {
        return None;
    }
    let held = a.loads.is_empty();
    let base = a.base;
    let read = Counted {
        index: i,
        array: a,
        item: None,
        index_reads: 1,
    };
    // ⚠ A body that opens by taking an element is judged as that alone: where the item it takes
    // is not a `foreach`'s, the loop stays as it is and no other item is made for it.
    if let [Stmt::Simple(ir::Stmt::Assign { target: x, value }), rest @ ..] = body.as_slice() {
        if let Some([array, at]) = intrinsic(value, |name| name.starts_with("array_get_")) {
            let reads = path_of(array).as_ref() == Some(&read.array) && var_of(at, true) == Some(i);
            let kept = held || still(rest);
            if !reads || *x == i || *x == base || labelled(rest) || !kept {
                return None;
            }
            let rest = used(rest);
            let of = |var: Var| rest.get(&var).copied().unwrap_or_default();
            let quiet = of(i) == Use::default() && of(base).assigned == 0 && of(*x).assigned == 0;
            return quiet.then_some(Counted {
                item: Some(*x),
                ..read
            });
        }
    }
    if labelled(body) || suspends(body) || !(held || still(body)) {
        return None;
    }
    let uses = used(body);
    let of = |var: Var| uses.get(&var).copied().unwrap_or_default();
    let index = of(i);
    if index.assigned != 0 || index.addressed || of(base).assigned != 0 {
        return None;
    }
    // Every read of the index is one of the element: the walk meets as many reads as there are.
    let reads = index.read;
    let led = reads == 1 && lead(body, &read, 0) == Lead::Read;
    let mut seen = Ahead::default();
    let walked = reads > 0 && ahead(body, &read, 0, &mut seen);
    let first = led || (walked && seen.alike && seen.reads == reads);
    first.then_some(Counted {
        index_reads: reads,
        ..read
    })
}

/// Whether nothing in `body`, at any depth, may write to memory: no store, and no call but of a read or a conversion.
fn still(body: &[Stmt]) -> bool {
    let mut quiet = true;
    tree::each_simple(body, &mut |s| quiet &= !stores(s));
    tree::each_expr(body, &mut |top| top.walk(&mut |e| quiet &= !loud(e)));
    quiet
}

/// What a walk through a body in the order it is written has met.
#[derive(Debug)]
struct Ahead {
    /// The reads of the element met.
    reads: usize,
    /// The first of them.
    first: Option<Expr>,
    /// Whether every read met is the first, at the same type.
    alike: bool,
    /// Whether something that may change the element has been met.
    loud: bool,
}

impl Default for Ahead {
    fn default() -> Ahead {
        Ahead {
            reads: 0,
            first: None,
            alike: true,
            loud: false,
        }
    }
}

impl Ahead {
    /// Counts `e` as a read of the element; `false` where something that may change it came before.
    fn read(&mut self, e: &Expr) -> bool {
        self.reads += 1;
        let alike = self.first.as_ref().is_none_or(|first| first == e);
        self.alike &= alike;
        if self.first.is_none() {
            self.first = Some(e.clone());
        }
        !self.loud
    }
}

/// [`ahead`] over one expression, its parts in the order they are evaluated.
fn ahead_expr(top: &Expr, read: &Counted, seen: &mut Ahead) -> bool {
    let mut calls = false;
    top.walk(&mut |e| calls |= matches!(e, Expr::Call { .. }));
    if !calls {
        return true;
    }
    // ⚠ `walk_mut` goes through operands before the call they are given to, as `lead_expr` reads.
    let mut copy = top.clone();
    let mut fine = true;
    copy.walk_mut(&mut |e| {
        if read.reads(e) {
            fine &= seen.read(e);
        } else if loud(e) {
            seen.loud = true;
        }
    });
    fine
}

/// [`ahead`] over one simple statement: its expressions in their order, then its own write.
fn ahead_simple(s: &ir::Stmt, read: &Counted, seen: &mut Ahead) -> bool {
    let mut fine = true;
    s.each_expr(&mut |top| fine &= ahead_expr(top, read, seen));
    seen.loud |= stores(s);
    fine
}

/// Walks `list` in the order it is written, an `if`'s two arms and a switch's cases one after another; `false` where a read of the element follows something that may change the element, or stands in a loop or a construct that may change it.
// A way through runs some of what is written, in the order written: a read before every change
// on the walk is one before every change on each way. A loop runs its body again, so a read in
// one is taken only where nothing in that loop may change the element.
fn ahead(list: &[Stmt], read: &Counted, depth: u32, seen: &mut Ahead) -> bool {
    if depth > DEPTH {
        return false;
    }
    for stmt in list {
        let fine = match stmt {
            Stmt::Simple(s) => ahead_simple(s, read, seen),
            Stmt::If { cond, then, els } => {
                ahead_expr(cond, read, seen)
                    && ahead(then, read, depth + 1, seen)
                    && ahead(els, read, depth + 1, seen)
            }
            Stmt::Switch { value, cases } => {
                ahead_expr(value, read, seen)
                    && (cases.iter()).all(|case| ahead(&case.body, read, depth + 1, seen))
            }
            Stmt::Return(Some(value)) => ahead_expr(value, read, seen),
            Stmt::Return(None)
            | Stmt::Break
            | Stmt::Continue
            | Stmt::Goto(_)
            | Stmt::Label(_)
            | Stmt::Unreachable => true,
            Stmt::Sugar(sugar) if matches!(**sugar, Sugar::Group { .. }) => {
                let parts = sugar.parts().simples;
                parts.into_iter().all(|part| ahead_simple(part, read, seen))
            }
            _ => {
                let within = std::slice::from_ref(stmt);
                let mut fine = true;
                let mut met = false;
                tree::each_expr(within, &mut |top| {
                    top.walk(&mut |e| {
                        if read.reads(e) {
                            met = true;
                            fine &= seen.read(e);
                        }
                    });
                });
                let calm = still(within);
                seen.loud |= !calm;
                fine && (calm || !met)
            }
        };
        if !fine {
            return false;
        }
    }
    true
}

/// Whether a body holds an `await` or a `yield` at any depth: a place the function leaves and comes back to.
fn suspends(body: &[Stmt]) -> bool {
    let mut found = false;
    tree::each_sugar(body, &mut |sugar| {
        found |= matches!(sugar, Sugar::Await { .. } | Sugar::Yield { .. });
    });
    found
}

/// What a run of code does first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lead {
    /// Reads the element, before anything that may change it.
    Read,
    /// Something that may change the element, or a read of it that is not one a round.
    Loud,
    /// Neither.
    Quiet,
}

/// Whether evaluating `e` itself may change an element of an array: a call of anything but a read or a conversion.
fn loud(e: &Expr) -> bool {
    fn reads_only(name: &str) -> bool {
        let converts = name.split_once("_to_").is_some_and(|(from, to)| {
            Scalar::from_name(from).is_some() && Scalar::from_name(to).is_some()
        });
        converts || name == "array_length" || name.starts_with("array_get_")
    }
    match e {
        Expr::Call {
            callee: Callee::Intrinsic { name, .. },
            ..
        } => !reads_only(name),
        Expr::Call { .. } => true,
        _ => false,
    }
}

/// Whether a simple statement writes through an address.
fn stores(s: &ir::Stmt) -> bool {
    !matches!(
        s,
        ir::Stmt::Assign { .. } | ir::Stmt::AssignMany { .. } | ir::Stmt::Eval(_)
    )
}

/// What evaluating `top` does first.
fn lead_expr(top: &Expr, read: &Counted) -> Lead {
    let mut calls = false;
    top.walk(&mut |e| calls |= matches!(e, Expr::Call { .. }));
    if !calls {
        return Lead::Quiet;
    }
    // ⚠ `walk_mut` goes through an expression in the order its parts are evaluated, operands
    // before the call they are given to, which `walk` does not: hence the copy. An operand a
    // test may skip counts as evaluated.
    let mut copy = top.clone();
    let mut got = Lead::Quiet;
    copy.walk_mut(&mut |e| {
        if got == Lead::Quiet {
            got = match (read.reads(e), loud(e)) {
                (true, _) => Lead::Read,
                (false, true) => Lead::Loud,
                (false, false) => Lead::Quiet,
            };
        }
    });
    got
}

/// What a simple statement does first: its expressions in their order, then its own write.
fn lead_simple(s: &ir::Stmt, read: &Counted) -> Lead {
    let mut got = Lead::Quiet;
    s.each_expr(&mut |top| {
        if got == Lead::Quiet {
            got = lead_expr(top, read);
        }
    });
    match got {
        Lead::Quiet if stores(s) => Lead::Loud,
        got => got,
    }
}

/// Whether `stmt` names the index nowhere and holds nothing that may change an element, at any depth.
fn calm(stmt: &Stmt, read: &Counted) -> bool {
    let within = std::slice::from_ref(stmt);
    let mut calm = true;
    tree::each_simple(within, &mut |s| calm &= !stores(s));
    tree::each_expr(within, &mut |top| {
        top.walk(&mut |e| {
            calm &= !loud(e) && !matches!(e, Expr::Var(var) if *var == read.index);
        });
    });
    calm
}

/// What the statements of `list` do first, on any way through them that reads the element.
// A way that never reads the element may do anything before it leaves: the item taken at the
// top of the round is then read by nothing. ⚠ A loop and a source construct run what they hold
// in an order of their own, so the read is looked for in neither.
fn lead(list: &[Stmt], read: &Counted, depth: u32) -> Lead {
    if depth > DEPTH {
        return Lead::Loud;
    }
    for stmt in list {
        let got = match stmt {
            Stmt::Simple(s) => lead_simple(s, read),
            Stmt::If { cond, then, els } => match lead_expr(cond, read) {
                Lead::Quiet => match (lead(then, read, depth + 1), lead(els, read, depth + 1)) {
                    (Lead::Read, _) | (_, Lead::Read) => Lead::Read,
                    (Lead::Quiet, Lead::Quiet) => Lead::Quiet,
                    _ => Lead::Loud,
                },
                first => first,
            },
            // A case runs into the next: the read in one must find every case before it quiet.
            Stmt::Switch { value, cases } => match lead_expr(value, read) {
                Lead::Quiet => {
                    let mut all = Lead::Quiet;
                    for case in cases {
                        if all == Lead::Quiet {
                            all = lead(&case.body, read, depth + 1);
                        }
                    }
                    all
                }
                first => first,
            },
            Stmt::Return(Some(value)) => lead_expr(value, read),
            Stmt::Return(None)
            | Stmt::Break
            | Stmt::Continue
            | Stmt::Goto(_)
            | Stmt::Unreachable => Lead::Quiet,
            Stmt::Sugar(sugar) => match &**sugar {
                // Simple statements side by side, run in their order.
                Sugar::Group { parts, .. } => {
                    let mut all = Lead::Quiet;
                    for part in parts {
                        if all == Lead::Quiet {
                            all = lead_simple(part, read);
                        }
                    }
                    all
                }
                _ if calm(stmt, read) => Lead::Quiet,
                _ => Lead::Loud,
            },
            _ if calm(stmt, read) => Lead::Quiet,
            _ => Lead::Loud,
        };
        if got != Lead::Quiet {
            return got;
        }
    }
    Lead::Quiet
}

/// Puts `item` wherever `body` reads the element and returns the read; `None`, with nothing changed, where it reads none.
// [`array_loop`] has seen every read alike: each is the one read taken first.
fn take_element(body: &mut Vec<Stmt>, read: &Counted, item: Var) -> Option<Expr> {
    let mut taken: Option<Expr> = None;
    let mut work: Vec<&mut Vec<Stmt>> = vec![body];
    while let Some(list) = work.pop() {
        for stmt in list {
            tree::own_exprs_mut(stmt, &mut |top| {
                top.walk_mut(&mut |e| {
                    let again = taken.as_ref().is_some_and(|held| *held == *e);
                    if again || (taken.is_none() && read.reads(e)) {
                        let held = std::mem::replace(e, Expr::Var(item));
                        if taken.is_none() {
                            taken = Some(held);
                        }
                    } else if matches!(e, Expr::Sugar { e: held, .. } if **held == Expr::Var(item))
                    {
                        // What the read was read as says nothing of a variable.
                        *e = Expr::Var(item);
                    }
                });
            });
            work.extend(tree::nested_mut(stmt));
        }
    }
    taken
}

/// What is known of an item made for `element`: as many cells as the read yields, of the kind the array's elements are where it states those cells; else, for one cell, of the type the read is made at where that is narrower than a cell, and for several, a value of no stated layout.
fn item_info(element: &Expr, array: &Path, head: &Head) -> VarInfo {
    let cells = match element {
        Expr::Call {
            callee: Callee::Intrinsic { name, .. },
            returns: Some(cells),
            ..
        } if name == "array_get_value" => (*cells).max(1),
        _ => 1,
    };
    fn states(kind: &Kind, cells: u32) -> bool {
        match kind {
            Kind::Unknown | Kind::Aggregate(_) => false,
            Kind::Vector | Kind::Handle => cells == 2,
            Kind::Value { cells: of, .. } => *of == cells,
            _ => cells == 1,
        }
    }
    let kind = array.var().and_then(|var| head.vars.get(&var));
    let of = match kind.map(|info| &info.kind) {
        Some(Kind::Array(of)) if states(of, cells) => Some((**of).clone()),
        _ => None,
    };
    if cells > 1 {
        return VarInfo {
            width: Width::Cells,
            kind: of.unwrap_or(Kind::Aggregate(cells)),
            name: None,
            single: false,
        };
    }
    let read_at = match element {
        Expr::Call {
            callee: Callee::Intrinsic { name, .. },
            ..
        } => name.strip_prefix("array_get_").and_then(Scalar::from_name),
        _ => None,
    };
    let narrow = read_at.filter(|ty| ty.bytes() < 8).map(Scalar::kind);
    VarInfo {
        width: Width::Cell,
        kind: of.or(narrow).unwrap_or_default(),
        name: None,
        single: false,
    }
}

/// Turns a loop [`array_loop`] finds to read its element in place into the `foreach` over `item`, the read made the taking of it, and returns what is known of `item`; `None`, with nothing changed, for any other statement.
// The item is taken at the top of each round, where the index is below the length and nothing
// has run since the test: the cell read is the one the body read.
fn folded_foreach(stmt: &mut Stmt, read: &Counted, item: Var, head: &Head) -> Option<VarInfo> {
    match std::mem::replace(stmt, Stmt::Break) {
        Stmt::For {
            init: Some(init),
            cond: Some(cond),
            step,
            mut body,
        } => match take_element(&mut body, read, item) {
            Some(element) => {
                let info = item_info(&element, &read.array, head);
                let taken = ir::Stmt::Assign {
                    target: item,
                    value: element,
                };
                *stmt = Stmt::Sugar(Box::new(Sugar::Foreach {
                    setup: vec![*init],
                    cond,
                    item: Box::new(taken),
                    body,
                    step,
                    after: Vec::new(),
                }));
                Some(info)
            }
            None => {
                *stmt = Stmt::For {
                    init: Some(init),
                    cond: Some(cond),
                    step,
                    body,
                };
                None
            }
        },
        other => {
            *stmt = other;
            None
        }
    }
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

/// The member a callee is, by the name the user's list gives its slot on the type, or on the generic type the type is made from; a method of the runtime is `Dispose` by the last name of its path.
// ⚠ OpenRage ships no names: with no list nothing is a member, and no enumerator loop is read
// as a `foreach`. The path of a method of the runtime is a list's or a listing's own; one that
// neither names is two numbers, which end in no member's name.
fn member_of(callee: &Callee, cx: &Context<'_>) -> Option<Member> {
    let (ty, slot) = match callee {
        Callee::Virtual { ty, slot, .. } => (ty, *slot),
        Callee::Builtin { path, .. } => {
            let disposes = path.rsplit('.').next() == Some("Dispose");
            return disposes.then_some(Member::Dispose);
        }
        _ => return None,
    };
    let key = TypeKey::of_name(ty)?;
    let made_from = || Some(cx.project.types.get(&key)?.generic.as_ref()?.0);
    let listed = cx.names.method(key, slot);
    match listed.or_else(|| cx.names.method(made_from()?, slot))? {
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

/// The variable `stmt` disposes, alone or where it is not null.
fn disposed(stmt: &Stmt, cx: &Context<'_>) -> Option<Var> {
    let call = match stmt {
        Stmt::Simple(ir::Stmt::Eval(call)) => call,
        Stmt::If { then, els, .. } => match (then.as_slice(), els.as_slice()) {
            ([Stmt::Simple(ir::Stmt::Eval(call))], []) => call,
            ([], [Stmt::Simple(ir::Stmt::Eval(call))]) => call,
            _ => return None,
        },
        _ => return None,
    };
    called_on(call, Member::Dispose, cx)
}

/// The variable the statements `list` ends in dispose, how many they are, and the shape they take.
fn closing(list: &[Stmt], cx: &Context<'_>) -> Option<(Var, usize, (Form, Option<Var>))> {
    let (last, before) = list.split_last()?;
    let held = disposed(last, cx)?;
    // The disposal may be of a cast of the variable, held in one of its own.
    let cast_of = match before.last() {
        Some(Stmt::Simple(ir::Stmt::Assign { target, value })) if *target == held => {
            match intrinsic(value, |name| name.starts_with("try_cast<")) {
                Some([cast]) => var_of(cast, false),
                _ => None,
            }
        }
        _ => None,
    };
    let (var, len) = match cast_of {
        Some(var) => (var, 2),
        None => (held, 1),
    };
    let seq = list.get(list.len().checked_sub(len)?..)?;
    Some((var, len, disposal(seq, var, cx)?))
}

/// The statements a list of `before` ends in that are the disposal of `e` in the shape `form`, counted, their uses added to `expected`.
fn disposal_before(
    before: &[Stmt],
    e: Var,
    form: Form,
    cx: &Context<'_>,
    expected: &mut BTreeMap<Var, Use>,
) -> Option<usize> {
    let (found, len) = [2usize, 1].into_iter().find_map(|len| {
        let seq = before.get(before.len().checked_sub(len)?..)?;
        Some((disposal(seq, e, cx)?, len))
    })?;
    if found.0 != form {
        return None;
    }
    expect(expected, e, found);
    Some(len)
}

/// Wraps as hidden the statements `leaving` counts at the end of those before each `return` of `body`, where `exits`; `None` where it counts none before one, where a `return` hands back what they may change, or where the body holds a jump.
// ⚠ A `return` runs its value before what a source construct runs on the way out, and here
// after it: only a constant or a variable no call changes reads the same both ways.
fn hide(
    body: &mut Vec<Stmt>,
    scope: &Scope<'_>,
    exits: bool,
    leaving: &mut dyn FnMut(&[Stmt]) -> Option<usize>,
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
            if let (Some(settled), true) = (settled, exits) {
                // What a construct around this one runs on the way out is hidden already, and stands between.
                let mut end = at;
                while end > 0
                    && matches!(&list[end - 1], Stmt::Sugar(s) if matches!(**s, Sugar::Hidden(_)))
                {
                    end -= 1;
                }
                let len = leaving(&list[..end])?;
                if !settled || len == 0 || len > end {
                    return None;
                }
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
    let form = ending.map(|(found, _)| found.0);
    let mut leaving = |before: &[Stmt]| disposal_before(before, e, form?, cx, &mut expected);
    hide(&mut body, scope, form.is_some(), &mut leaving)?;

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
