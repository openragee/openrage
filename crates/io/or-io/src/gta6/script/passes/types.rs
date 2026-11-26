//! Types: what each variable, constant and memory access is, from how the code uses it.
//!
//! Evidence is joined per variable to a fixed point; then each cast the kinds make redundant
//! goes, a constant takes the kind of what reads it and an access through an address is typed.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};

use crate::gta6::script::cfg::{self, Terminator};
use crate::gta6::script::ir::{
    BinOp, Callee, Const, Expr, FunctionKey, Head, Kind, Returns, Scalar, Stmt, StoreKind, TypeKey,
    UnOp, Var, VarInfo, Width,
};
use crate::gta6::script::names::Names;
use crate::gta6::script::project::{signatures, Signature};
use crate::gta6::script::{Context, Pass, Project, Stats};

#[path = "types_kinds.rs"]
mod kinds;

use kinds::Lat;

/// Expressions nested deeper than this leave their function as the pass found it.
const MAX_DEPTH: u32 = 192;
/// The rounds a fixed point may take before the function is left untyped.
const MAX_ROUNDS: usize = 24;
/// The rounds the layouts' evidence is given to agree with the kinds it leads to.
const FIELD_ROUNDS: usize = 4;
/// The loads an address may be under before the type of its object is given up on.
const MAX_OWNERS: u32 = 16;
/// The rounds a kind lost may cost the kinds of what is copied from it, before every unproved kind goes.
const LOOSE_ROUNDS: usize = 8;
/// The flags of a type whose values are held in place.
const VALUE: u32 = 0x1C;

/// Infers the kind of every variable and constant, then drops each cast the kinds make redundant and types each access.
pub fn run(f: &mut cfg::Function, names: &Names, stats: &mut Stats) {
    run_in(f, &Context::baseline(names), stats);
}

/// [`run`] with the project as evidence where `kinds` is on, and each conflict of signs settled where `signs` is.
// `Stats` has no count of kinds or casts, so nothing is counted here.
pub fn run_in(f: &mut cfg::Function, cx: &Context<'_>, _stats: &mut Stats) {
    let around = Around::of(&f.head, cx);
    let Some(mut facts) = Facts::of(f, &around) else {
        return;
    };
    if narrowings(f) {
        match Facts::of(f, &around) {
            Some(again) => facts = again,
            None => return,
        }
    }
    let mut typing = Typing {
        around: &around,
        wide: wide_of(f, around.kinds),
        facts,
        known: HashMap::new(),
        normal: HashSet::new(),
        past: RefCell::new(HashSet::new()),
        loose: HashSet::new(),
    };
    typing.infer(&f.cfg);
    typing.settle(&f.cfg);
    typing.loosen(&f.cfg);
    for block in &mut f.cfg.blocks {
        for stmt in &mut block.stmts {
            typing.stmt(stmt);
        }
        typing.term(&mut block.term);
    }
    typing.record(f);
}

/// What stands around a function: the user's names, the project's facts, and which of this pass's later rules are on.
struct Around<'a> {
    names: &'a Names,
    project: &'a Project,
    /// The hash of the function's program, which a call of one of its own functions is looked up by.
    program: u32,
    /// `kinds` is on: signatures, layouts and the kinds another pass wrote are evidence.
    kinds: bool,
    /// `signs` is on: a 32-bit integer read under both signs takes one of them.
    signs: bool,
    /// `interproc` is on: a call's cell is widened as the integer its callee's signature names.
    interproc: bool,
    /// What the function's own signature says the one cell it returns is.
    returned: Lat,
    /// The parameter cells every call of the function passes an `int` widened by its sign, where `interproc` is on.
    widened: Option<&'a BTreeSet<u32>>,
}

impl<'a> Around<'a> {
    fn of(head: &Head, cx: &Context<'a>) -> Around<'a> {
        let mut around = Around {
            names: cx.names,
            project: cx.project,
            program: head.program,
            kinds: cx.options.on(Pass::Kinds),
            signs: cx.options.on(Pass::Signs),
            interproc: cx.options.on(Pass::Interproc),
            returned: Lat::None,
            widened: None,
        };
        if around.interproc {
            let own = FunctionKey::new(head.program, head.at);
            around.widened = cx.project.widened.get(&own);
        }
        let own = around.held(FunctionKey::new(head.program, head.at));
        if let Some(own) = own.filter(|own| own.returns == 1 && head.returns == Returns::Cell) {
            around.returned = kinds::stated(&own.result);
        }
        around
    }

    /// The project's signature of a function, where `kinds` is on and the signature waits on nothing more.
    // ⚠ One that still waits holds what its function's own code says so far, which proves nothing.
    fn held(&self, key: FunctionKey) -> Option<&'a Signature> {
        let held = self.project.signatures.get(&key)?;
        (self.kinds && signatures::settled(held)).then_some(held)
    }

    /// The project's signature of the function a call names outright: one of this program, or of one it imports.
    // ⚠ Not a virtual call's: the slot's function is one of those the call may reach.
    fn signature(&self, callee: &Callee) -> Option<&'a Signature> {
        self.held(signatures::key_of(self.program, callee)?)
    }

    /// What a call yields in one cell: what it is, and the kind its cell is widened as.
    // ⚠ A signature's word on how the cell is widened is what `interproc` rests on, so it is
    // taken only where that name is on, and only for a truth or an integer: with it off the
    // result only types what it is assigned to and tested as.
    fn yields(&self, callee: &Callee, returns: Option<u32>) -> (Lat, Option<Kind>) {
        let one = |own: &&Signature| own.returns == 1 && returns.is_none_or(|cells| cells == 1);
        match self.signature(callee).filter(one) {
            Some(own) => {
                let widened =
                    self.interproc && kinds::narrow(&own.result) && !own.result.is_float();
                let shape = widened.then(|| own.result.clone());
                (kinds::stated(&own.result), shape)
            }
            None => yields(self.names, callee, returns),
        }
    }

    /// How a call takes each of its arguments.
    fn arg_reads(&self, callee: &Callee, args: &[Expr], wide: &HashSet<Var>) -> Vec<Read> {
        let Some(own) = self.signature(callee) else {
            return arg_reads(self.names, callee, args.len());
        };
        // ⚠ A parameter of one cell must face an argument of one, and a wider one a wider
        // argument, or a kind would land on the cell beside its own.
        let faces = |cells: u32, arg: &Expr| (cells == 1) == one_cell(arg, wide);
        let fits = own.params.len() == args.len()
            && own
                .params
                .iter()
                .zip(args)
                .all(|(param, arg)| faces(param.cells, arg));
        if !fits {
            return vec![Read::Cell; args.len()];
        }
        let read = own
            .params
            .iter()
            .map(|param| match kinds::stated(&param.kind) {
                lat if param.cells == 1 && lat != Lat::None => Read::Hint(lat),
                _ => Read::Cell,
            });
        read.collect()
    }

    /// What the project says the global cell at byte `offset` of a program holds.
    fn global(&self, program: u32, offset: u32, statics: bool) -> Lat {
        let cells = self
            .project
            .programs
            .get(&program)
            .map(|info| match statics {
                true => &info.statics,
                false => &info.globals,
            });
        match cells.and_then(|cells| cells.get(&offset)) {
            Some(kind) if self.kinds => kinds::stated(kind),
            _ => Lat::None,
        }
    }

    /// What two pieces of evidence say together.
    // Where `kinds` is on, two named references are the one the other derives from, else some
    // reference for good: `Lat::join` alone would let a third type take the place of two.
    fn join(&self, a: &Lat, b: &Lat) -> Lat {
        match (a, b) {
            (Lat::Is(x), Lat::Is(y))
                if self.kinds && x != y && kinds::named(x) && kinds::named(y) =>
            {
                let key = |kind: &Kind| match kind {
                    Kind::Object(name) => TypeKey::of_name(name),
                    _ => None,
                };
                match (key(x), key(y)) {
                    (Some(from), Some(to)) if self.project.is_a(from, to) => Lat::Is(y.clone()),
                    (Some(to), Some(from)) if self.project.is_a(from, to) => Lat::Is(x.clone()),
                    _ => Lat::Refs,
                }
            }
            _ => a.join(b),
        }
    }
}

/// How the expression around one takes its value.
#[derive(Clone, Debug)]
enum Read {
    /// The whole cell.
    Cell,
    /// The whole cell, for whether it is zero and nothing else.
    Truth,
    /// The whole cell, by something a signature or a declared kind says is of this kind.
    Hint(Lat),
    /// As the address a load or a store goes through.
    Base,
    /// As an operand of an operator that works at `at`; `weak` when it reads either sign alike.
    Operand { weak: bool, at: Kind },
    /// The low bits alone, read as this type; `weak` as for an operand.
    As(Scalar, bool),
    /// The low bytes alone, of no stated type.
    Low(u32),
    /// By an operator that takes its type from the operand's own cast, which must stay.
    Keep,
}

/// What a `byte_offset` is taken from, where that can be told apart cheaply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Base {
    Var(Var),
    Addr(Var, u32),
}

/// How a function uses its variables, read before any kind is known.
#[derive(Default)]
struct Facts {
    /// What the reads of each variable say it is, joined.
    uses: HashMap<Var, Lat>,
    /// The most bytes any read of a variable takes.
    read: HashMap<Var, u32>,
    /// Variables whose address the function takes.
    addressed: HashSet<Var>,
    /// The lowest local whose address is taken: it and every local after it may be written through it.
    lowest_local: Option<u32>,
    /// The lowest local, and the lowest parameter, whose address is used other than to read or write that one cell.
    // ⚠ Such an address reaches its own cell and every cell after it, never one before.
    locals_escape: Option<u32>,
    params_escape: Option<u32>,
    /// Each `byte_offset(base, n)` taken as a value: an embedded struct, whose offset stays apart.
    apart: HashSet<(Base, u64)>,
    /// How often something that reads one sign alone reads each variable as an `int`, and as a `uint`.
    signs: HashMap<Var, (u32, u32)>,
    /// How often each variable is read by name, and how many of those reads take only whether it is zero.
    // ⚠ A read through the variable's own address is in neither count.
    tested: HashMap<Var, (u32, u32)>,
    /// The one named reference each variable is handed on as, a conflict where two calls take two.
    // ⚠ Kept from the joins: what is passed may derive from what is taken, so an assignment says more.
    wanted: HashMap<Var, Lat>,
    /// An expression was nested too deep to read.
    deep: bool,
}

impl Facts {
    /// Reads a function; `None` where an expression is nested too deep to follow.
    fn of(f: &cfg::Function, around: &Around<'_>) -> Option<Facts> {
        let wide = wide_of(f, around.kinds);
        let mut facts = Facts::default();
        let mut scan = Scan {
            around,
            head: &f.head,
            wide: &wide,
            facts: &mut facts,
        };
        if around.kinds {
            scan.stated();
        }
        scan.function(&f.cfg);
        (!facts.deep).then_some(facts)
    }
}

/// The variables no one-cell kind is inferred for: those declared as several cells and, where `by_kind`, those another pass gave a kind this lattice does not join.
fn wide_of(f: &cfg::Function, by_kind: bool) -> HashSet<Var> {
    let opaque = |kind: &Kind| *kind != Kind::Unknown && kinds::stated(kind) == Lat::None;
    let vars = f.head.vars.iter();
    let wide =
        vars.filter(|(_, info)| info.width == Width::Cells || (by_kind && opaque(&info.kind)));
    wide.map(|(var, _)| *var).collect()
}

/// Whether an expression is one cell wide by its form.
fn one_cell(e: &Expr, wide: &HashSet<Var>) -> bool {
    match e {
        Expr::Var(var) => !wide.contains(var),
        Expr::Call { returns, .. } => *returns == Some(1),
        Expr::Cells(_)
        | Expr::GuidEmpty
        | Expr::LoadBytes { .. }
        | Expr::ExtractBytes { .. }
        | Expr::ZeroSlots(_)
        | Expr::Vector3(_)
        | Expr::BuiltinGlobal(_) => false,
        _ => true,
    }
}

/// Variables joined into sets, each set under one of its members.
#[derive(Default)]
struct Groups {
    /// The member each variable stands under; one that is not here stands for its set.
    above: HashMap<Var, Var>,
}

impl Groups {
    /// The member the set of `var` goes by.
    fn find(&mut self, var: Var) -> Var {
        let mut top = var;
        while let Some(up) = self.above.get(&top) {
            top = *up;
        }
        let mut at = var;
        while at != top {
            at = self.above.insert(at, top).unwrap_or(top);
        }
        top
    }

    fn join(&mut self, a: Var, b: Var) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.above.insert(a, b);
        }
    }
}

/// What the variables of one set say of their sign together.
#[derive(Default)]
struct Signs {
    /// Reads of any of them as an `int`, and as a `uint`, by something that reads one sign alone.
    signed: u32,
    unsigned: u32,
    /// An assignment makes one of them an `int`, or a `uint`.
    int: bool,
    uint: bool,
    /// One of them is neither: the set has no one sign to take.
    other: bool,
}

/// How often a function mentions each variable, for [`narrowings`].
#[derive(Default)]
struct Census {
    /// Mentions of the variable as a value.
    reads: HashMap<Var, u32>,
    addresses: HashMap<Var, u32>,
    sets: HashMap<Var, u32>,
    /// The lowest local, and the lowest parameter, whose address is taken.
    lowest_local: Option<u32>,
    lowest_param: Option<u32>,
}

impl Census {
    fn of(cfg: &cfg::Cfg) -> Census {
        let mut census = Census::default();
        for block in &cfg.blocks {
            for stmt in &block.stmts {
                for var in stmt.targets() {
                    *census.sets.entry(*var).or_insert(0) += 1;
                }
                stmt.each_expr(&mut |top| census.expr(top));
            }
            if let Some(e) = block.term.expr() {
                census.expr(e);
            }
        }
        census
    }

    fn expr(&mut self, e: &Expr) {
        e.walk(&mut |node| match node {
            Expr::Var(var) => *self.reads.entry(*var).or_insert(0) += 1,
            Expr::AddrOf { var, .. } => {
                *self.addresses.entry(*var).or_insert(0) += 1;
                match var {
                    Var::Local(n) => {
                        self.lowest_local = Some(self.lowest_local.map_or(*n, |low| low.min(*n)));
                    }
                    Var::Param(n) => {
                        self.lowest_param = Some(self.lowest_param.map_or(*n, |low| low.min(*n)));
                    }
                    Var::Value { .. } | Var::Stack { .. } | Var::Global { .. } => {}
                }
            }
            _ => {}
        });
    }

    /// Whether only an assignment that names the variable can change it.
    fn fixed(&self, var: Var) -> bool {
        match var {
            Var::Local(n) => self.lowest_local.is_none_or(|lowest| n < lowest),
            Var::Param(n) => self.lowest_param.is_none_or(|lowest| n < lowest),
            Var::Value { .. } | Var::Stack { .. } => !self.addresses.contains_key(&var),
            Var::Global { .. } => false,
        }
    }
}

/// Whether `e` is `var[0]`.
fn is_cell(e: &Expr, var: Var) -> bool {
    matches!(e, Expr::Cell { base, index: 0 } if **base == Expr::Var(var))
}

fn cell_reads(e: &Expr, var: Var) -> u32 {
    let mut reads = 0;
    e.walk(&mut |node| reads += u32::from(is_cell(node, var)));
    reads
}

fn stmt_cell_reads(stmt: &Stmt, var: Var) -> u32 {
    let mut reads = 0;
    stmt.each_expr(&mut |top| reads += cell_reads(top, var));
    reads
}

fn replace_cells(e: &mut Expr, var: Var, with: &Expr) {
    e.walk_mut(&mut |node| {
        if is_cell(node, var) {
            *node = with.clone();
        }
    });
}

/// The temporary that `made` zeroes and `stored` fills, what its cell then holds, and whether that may be read later in its place.
fn narrowed(census: &Census, made: &Stmt, stored: &Stmt) -> Option<(Var, Expr, bool)> {
    let Stmt::Assign {
        target,
        value: Expr::ZeroSlots(1),
    } = made
    else {
        return None;
    };
    let Stmt::Store {
        kind,
        addr: Expr::AddrOf { var, cell: 0 },
        offset: 0,
        value,
    } = stored
    else {
        return None;
    };
    let once = |counts: &HashMap<Var, u32>| counts.get(target) == Some(&1);
    let temporary = matches!(target, Var::Value { .. });
    if var != target || !temporary || !once(&census.sets) || !once(&census.addresses) {
        return None;
    }
    // The store fills the low bytes of a cell of zeroes: the cell is the value with nothing above them.
    let cell = match kind {
        StoreKind::U8 => Expr::cast(Scalar::U8, value.clone()),
        StoreKind::U16 => Expr::cast(Scalar::U16, value.clone()),
        StoreKind::I32 => Expr::cast(Scalar::U32, value.clone()),
        StoreKind::U64 | StoreKind::Ref => value.clone(),
    };
    let stable = match value {
        Expr::Const(_) => true,
        Expr::Var(read) => census.fixed(*read),
        _ => false,
    };
    Some((*target, cell, stable))
}

/// Whether a statement names `var` at all: assigns it, reads it or takes its address.
fn mentions(stmt: &Stmt, var: Var) -> bool {
    let mut found = stmt.targets().contains(&var);
    stmt.each_expr(&mut |top| {
        top.walk(&mut |e| {
            found |=
                matches!(e, Expr::Var(named) | Expr::AddrOf { var: named, .. } if *named == var);
        });
    });
    found
}

/// The first statement after `made` that names the temporary `made` zeroes, where `made` makes one cell of zeroes.
fn filled(stmts: &[Stmt], made: usize) -> Option<usize> {
    let Stmt::Assign {
        target,
        value: Expr::ZeroSlots(1),
    } = stmts.get(made)?
    else {
        return None;
    };
    (made + 1..stmts.len()).find(|at| mentions(&stmts[*at], *target))
}

/// Rewrites `v = zero_slots(1); ..; store_T(&v, 0, x);` and the `v[0]` that follow: a value cut to its low bytes, not memory.
// The reads must follow the store in its own block and nothing else may mention `v`. A constant
// goes to every read, a variable that cannot change to its one read in the statement after the
// store; any other value stays in `v`, now one cell.
pub(super) fn narrowings(f: &mut cfg::Function) -> bool {
    let census = Census::of(&f.cfg);
    let mut changed = false;
    for block in &mut f.cfg.blocks {
        let mut i = 0;
        while i + 1 < block.stmts.len() {
            let found = filled(&block.stmts, i).and_then(|at| {
                narrowed(&census, &block.stmts[i], &block.stmts[at]).map(|found| (at, found))
            });
            let Some((at, (var, cell, stable))) = found else {
                i += 1;
                continue;
            };
            let tail = &block.stmts[at + 1..];
            let later: u32 = tail.iter().map(|stmt| stmt_cell_reads(stmt, var)).sum();
            let last = block.term.expr().map_or(0, |e| cell_reads(e, var));
            let next = tail.first().map_or(last, |stmt| stmt_cell_reads(stmt, var));
            let reads = later + last;
            if reads == 0 || census.reads.get(&var) != Some(&reads) {
                i += 1;
                continue;
            }
            let constant = match &cell {
                Expr::Cast { e, .. } => matches!(**e, Expr::Const(_)),
                other => matches!(other, Expr::Const(_)),
            };
            // With the two statements gone, what followed the store stands at `at - 1`.
            if constant {
                block.stmts.remove(at);
                block.stmts.remove(i);
                for stmt in &mut block.stmts[at - 1..] {
                    stmt.each_expr_mut(&mut |top| replace_cells(top, var, &cell));
                }
                if let Some(e) = block.term.expr_mut() {
                    replace_cells(e, var, &cell);
                }
                f.head.vars.remove(&var);
            } else if stable && reads == 1 && next == 1 {
                block.stmts.remove(at);
                block.stmts.remove(i);
                match block.stmts.get_mut(at - 1) {
                    Some(stmt) => stmt.each_expr_mut(&mut |top| replace_cells(top, var, &cell)),
                    None => {
                        if let Some(e) = block.term.expr_mut() {
                            replace_cells(e, var, &cell);
                        }
                    }
                }
                f.head.vars.remove(&var);
            } else {
                block.stmts[at] = Stmt::Assign {
                    target: var,
                    value: cell,
                };
                block.stmts.remove(i);
                let named = Expr::Var(var);
                for stmt in &mut block.stmts[at..] {
                    stmt.each_expr_mut(&mut |top| replace_cells(top, var, &named));
                }
                if let Some(e) = block.term.expr_mut() {
                    replace_cells(e, var, &named);
                }
                if let Some(info) = f.head.vars.get_mut(&var) {
                    info.width = Width::Cell;
                }
            }
            changed = true;
        }
    }
    changed
}

/// Whether an operator reads a signed and an unsigned operand alike.
fn either_sign(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::Eq | BinOp::Ne | BinOp::And | BinOp::Or | BinOp::Xor
    )
}

/// How a binary operator that works at `at` takes its two operands.
fn operand_reads(op: BinOp, at: &Kind) -> (Read, Read) {
    if !(at.is_integer() || at.is_float()) {
        return (Read::Cell, Read::Cell);
    }
    let left = Read::Operand {
        weak: either_sign(op),
        at: at.clone(),
    };
    let right = match op {
        // A shift counts by an `int` whatever it shifts.
        BinOp::Shl | BinOp::Shr => Read::Operand {
            weak: false,
            at: Kind::I32,
        },
        _ => left.clone(),
    };
    (left, right)
}

/// How a cast to `to`, itself read as `read`, takes what it casts.
fn under_cast(to: Scalar, read: &Read) -> Read {
    match read {
        // A cast that a read no wider than it looks through says nothing: that read decides.
        Read::As(outer, weak) if outer.bytes() <= to.bytes() => Read::As(*outer, *weak),
        Read::Low(bytes) if *bytes <= to.bytes() => Read::Low(*bytes),
        Read::Operand { weak, at } if *at == to.kind() => Read::As(to, *weak),
        // 32 bits passed on as a cell with nothing above them: how a listing hands any 32-bit
        // value over, and no word on its sign.
        Read::Cell | Read::Hint(_) if to == Scalar::U32 => Read::As(to, true),
        _ => Read::As(to, false),
    }
}

/// How a store of this kind takes its value.
fn store_read(kind: StoreKind) -> Read {
    match kind {
        StoreKind::U8 | StoreKind::U16 | StoreKind::I32 => Read::Low(kind.bytes()),
        StoreKind::U64 => Read::Cell,
        StoreKind::Ref => Read::Hint(Lat::Is(Kind::Pointer)),
    }
}

/// How a terminator takes its expression; `returned` is what the function's own signature says it returns.
fn term_read(term: &Terminator, returned: &Lat) -> Read {
    match term {
        // A switch compares as an `int`, and equality reads either sign alike.
        Terminator::Switch { .. } => Read::As(Scalar::I32, true),
        Terminator::Branch { .. } => Read::Truth,
        Terminator::Return(_) if *returned != Lat::None => Read::Hint(returned.clone()),
        _ => Read::Cell,
    }
}

/// The address under the constant `byte_offset`s of `addr`, and `offset` counted from it.
fn based(mut addr: &Expr, mut offset: u32) -> Option<(&Expr, u32)> {
    for _ in 0..8 {
        let Expr::ByteOffset {
            addr: under,
            offset: by,
        } = addr
        else {
            return Some((addr, offset));
        };
        let Expr::Const(c) = &**by else {
            return None;
        };
        let add = u32::try_from(c.bits).ok()?;
        offset = offset.checked_add(add)?;
        addr = &**under;
    }
    None
}

/// How a call takes each of its arguments.
fn arg_reads(names: &Names, callee: &Callee, passed: usize) -> Vec<Read> {
    match callee {
        Callee::Intrinsic { name, .. } => (0..passed)
            .map(|index| match kinds::intrinsic_reads(name, index) {
                Some(ty) => Read::As(ty, false),
                None => match kinds::intrinsic_takes(name, index) {
                    Lat::None => Read::Cell,
                    lat => Read::Hint(lat),
                },
            })
            .collect(),
        Callee::Native { hash, .. } => {
            let listed = names
                .native(*hash)
                .and_then(|native| kinds::native_args(native, passed));
            match listed {
                Some(cells) => cells
                    .into_iter()
                    .map(|lat| match lat {
                        Lat::None => Read::Cell,
                        lat => Read::Hint(lat),
                    })
                    .collect(),
                None => vec![Read::Cell; passed],
            }
        }
        _ => vec![Read::Cell; passed],
    }
}

/// What a call yields in one cell: what it is, and the kind its cell is widened as.
fn yields(names: &Names, callee: &Callee, returns: Option<u32>) -> (Lat, Option<Kind>) {
    if returns != Some(1) {
        return (Lat::None, None);
    }
    match callee {
        Callee::Intrinsic { name, .. } => kinds::intrinsic_yields(name),
        Callee::Native { hash, .. } => {
            let listed = names
                .native(*hash)
                .and_then(|native| native.returns.as_deref())
                .filter(|ty| kinds::native_cells(ty) == 1);
            (listed.map_or(Lat::None, kinds::native_lat), None)
        }
        Callee::Builtin { path, .. }
            if path.contains(".op_Equality") || path.contains(".op_Inequality") =>
        {
            (Lat::Is(Kind::Bool), None)
        }
        _ => (Lat::None, None),
    }
}

/// The kind the one cell a call yields is widened as, where the runtime itself says: an intrinsic's, never a native's.
pub(crate) fn yielded_shape(names: &Names, callee: &Callee, returns: Option<u32>) -> Kind {
    yields(names, callee, returns).1.unwrap_or(Kind::Unknown)
}

/// Whether the statement is `x++` or `x--` as a listing's reader writes it, which this pass leaves in that shape.
fn steps(target: Var, value: &Expr) -> bool {
    let Expr::Binary {
        op: BinOp::Add | BinOp::Sub,
        at: Kind::I32,
        l,
        r,
    } = value
    else {
        return false;
    };
    let Expr::Cast {
        to: Scalar::I32,
        e: stepped,
    } = &**l
    else {
        return false;
    };
    **stepped == Expr::Var(target) && matches!(&**r, Expr::Const(c) if c.bits == 1)
}

/// `e` without the casts a read of its low `bytes` bytes looks through.
fn peeled(mut e: &Expr, bytes: u32) -> &Expr {
    while let Expr::Cast { to, e: inner } = e {
        if to.bytes() < bytes {
            break;
        }
        e = &**inner;
    }
    e
}

/// Calls `f` on each variable a statement sets and the value it takes; a store through a variable's own address sets it.
fn each_def<'a>(cfg: &'a cfg::Cfg, f: &mut impl FnMut(Var, &'a Expr)) {
    for block in &cfg.blocks {
        for stmt in &block.stmts {
            match stmt {
                Stmt::Assign { target, value } => f(*target, value),
                Stmt::AssignMany { targets, values } => {
                    for (target, value) in targets.iter().zip(values) {
                        f(*target, value);
                    }
                }
                Stmt::Store {
                    kind,
                    addr: Expr::AddrOf { var, cell: 0 },
                    offset: 0,
                    value,
                } => f(*var, peeled(value, kind.bytes())),
                _ => {}
            }
        }
    }
}

fn base_of(e: &Expr) -> Option<Base> {
    match e {
        Expr::Var(var) => Some(Base::Var(*var)),
        Expr::AddrOf { var, cell } => Some(Base::Addr(*var, *cell)),
        _ => None,
    }
}

/// Takes an expression out of its place, leaving a placeholder the caller overwrites or drops.
fn take(e: &mut Expr) -> Expr {
    std::mem::replace(e, Expr::GuidEmpty)
}

fn is_zero(e: &Expr) -> bool {
    matches!(e, Expr::Const(c) if c.bits == 0 && !c.kind.is_float())
}

/// The cell of a constant, or of a cast of one.
fn constant_cell(e: &Expr) -> Option<u64> {
    match e {
        Expr::Const(c) => Some(c.bits),
        Expr::Cast { to, e: inner } => match &**inner {
            Expr::Const(c) => Some(kinds::cast_bits(*to, c.bits)),
            _ => None,
        },
        _ => None,
    }
}

/// The cell a shift of two constants makes, and the type the shift is written at.
// How a compiler writes a constant of one bit: at 64 bits, whatever then reads it.
fn literal_shift(e: &Expr) -> Option<(u64, &Kind)> {
    let Expr::Binary {
        op: BinOp::Shl,
        at,
        l,
        r,
    } = e
    else {
        return None;
    };
    let bits = kinds::shifted(at, constant_cell(l)?, constant_cell(r)?)?;
    Some((bits, at))
}

/// Calls `f` with each shift of two constants a value may be: the value itself, or an arm of a choice.
fn each_literal<'a>(e: &'a Expr, depth: u32, f: &mut impl FnMut(u64, &'a Kind)) {
    if depth > MAX_DEPTH {
        return;
    }
    match e {
        Expr::Select { then, els, .. } => {
            each_literal(then, depth + 1, f);
            each_literal(els, depth + 1, f);
        }
        other => {
            if let Some((bits, at)) = literal_shift(other) {
                f(bits, at);
            }
        }
    }
}

/// The variable an expression is, bare or under one cast to an integer: what a test of the expression against zero reads.
fn tested_var(e: &Expr) -> Option<Var> {
    match e {
        Expr::Var(var) => Some(*var),
        Expr::Cast { to, e: inner } if !to.kind().is_float() => match &**inner {
            Expr::Var(var) => Some(*var),
            _ => None,
        },
        _ => None,
    }
}

/// Whether a value is the constant 1, where it is the integer constant 0 or 1 or a truth written out.
fn zero_or_one(e: &Expr) -> Option<bool> {
    match e {
        Expr::Const(c) if c.bits <= 1 && (c.kind.is_integer() || c.kind == Kind::Bool) => {
            Some(c.bits == 1)
        }
        _ => None,
    }
}

/// `!e`, written as the opposite comparison where that is the same test.
fn negated(e: Expr) -> Expr {
    match e {
        Expr::Binary { op, at, l, r } if flips(op, &at) => Expr::Binary {
            op: flipped(op),
            at,
            l,
            r,
        },
        Expr::Unary {
            op: UnOp::Not,
            e: inner,
        } => *inner,
        Expr::Const(c) if c.kind == Kind::Bool => Expr::Const(Const::bool(c.bits == 0)),
        other => Expr::Unary {
            op: UnOp::Not,
            e: Box::new(other),
        },
    }
}

/// Whether `!(a op b)` is `a` under the opposite operator `b`.
// ⚠ Not for an ordering of floats: `!(a < b)` holds for a NaN and `a >= b` does not.
fn flips(op: BinOp, at: &Kind) -> bool {
    match op {
        BinOp::Eq | BinOp::Ne => true,
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => at.is_integer(),
        _ => false,
    }
}

fn flipped(op: BinOp) -> BinOp {
    match op {
        BinOp::Eq => BinOp::Ne,
        BinOp::Ne => BinOp::Eq,
        BinOp::Lt => BinOp::Ge,
        BinOp::Ge => BinOp::Lt,
        BinOp::Le => BinOp::Gt,
        BinOp::Gt => BinOp::Le,
        other => other,
    }
}

/// One reading of a function for [`Facts`].
struct Scan<'a> {
    around: &'a Around<'a>,
    head: &'a Head,
    wide: &'a HashSet<Var>,
    facts: &'a mut Facts,
}

impl Scan<'_> {
    /// Notes what stands before the code is read: the kinds another pass wrote, and those the function's own signature gives its parameters.
    fn stated(&mut self) {
        let (head, around) = (self.head, self.around);
        for (var, info) in &head.vars {
            self.note(*var, &kinds::stated(&info.kind));
        }
        let Some(own) = around.held(FunctionKey::new(head.program, head.at)) else {
            return;
        };
        // ⚠ A signature of other cells than the function takes is another function's.
        let cells = own
            .params
            .iter()
            .try_fold(0u32, |sum, param| sum.checked_add(param.cells));
        if cells != Some(head.params) {
            return;
        }
        let mut cell = 0;
        for param in &own.params {
            if param.cells == 1 {
                self.note(Var::Param(cell), &kinds::stated(&param.kind));
            }
            cell += param.cells;
        }
    }

    fn function(&mut self, cfg: &cfg::Cfg) {
        let around = self.around;
        for block in &cfg.blocks {
            for stmt in &block.stmts {
                self.stmt(stmt);
            }
            if let Some(e) = block.term.expr() {
                self.expr(e, &term_read(&block.term, &around.returned), 0);
            }
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match stmt {
            // An increment is an `int`'s whatever the variable is, so it says only "32 bits".
            Stmt::Assign { target, value } if steps(*target, value) => {
                self.seen(*target, 4);
                self.note(*target, &Lat::Int32);
            }
            Stmt::Assign { value, .. } => self.expr(value, &Read::Cell, 0),
            Stmt::AssignMany { values, .. } => {
                for value in values {
                    self.expr(value, &Read::Cell, 0);
                }
            }
            Stmt::Store {
                kind,
                addr,
                offset,
                value,
            } => {
                self.base(addr, *offset, kind.bytes(), false, 0);
                self.expr(value, &store_read(*kind), 0);
            }
            Stmt::StoreBytes {
                addr,
                offset,
                bytes,
                value,
            } => {
                self.base(addr, *offset, *bytes, false, 0);
                self.expr(value, &Read::Cell, 0);
            }
            Stmt::StoreF32Slots { addr, value, .. } => {
                self.base(addr, 0, u32::MAX, false, 0);
                self.expr(value, &Read::Cell, 0);
            }
            Stmt::ZeroBytes { addr, bytes } => self.base(addr, 0, *bytes, false, 0),
            Stmt::StepI32 { addr, offset, .. } => self.base(addr, *offset, 4, true, 0),
            Stmt::Eval(e) => self.expr(e, &Read::Cell, 0),
        }
    }

    /// The address an access of `bytes` bytes at `offset` goes through; `reads` when the access reads.
    fn base(&mut self, addr: &Expr, offset: u32, bytes: u32, reads: bool, depth: u32) {
        match addr {
            Expr::AddrOf { var, cell } => {
                let within = *cell == 0 && offset.checked_add(bytes).is_some_and(|end| end <= 8);
                self.address(*var, !within);
                if reads {
                    // A read from the cell's first byte takes its low bytes; any other is taken to see it all.
                    let taken = match within && offset == 0 {
                        true => bytes,
                        false => 8,
                    };
                    self.seen(*var, taken);
                }
            }
            // The base of an access and not a value: its offset may join the access's own.
            Expr::ByteOffset {
                addr: inner,
                offset: by,
            } => {
                self.expr(inner, &Read::Base, depth + 1);
                self.expr(by, &Read::Cell, depth + 1);
            }
            other => self.expr(other, &Read::Base, depth + 1),
        }
    }

    fn expr(&mut self, e: &Expr, read: &Read, depth: u32) {
        if depth > MAX_DEPTH {
            self.facts.deep = true;
            return;
        }
        let d = depth + 1;
        // ⚠ Counted here and once: the variable itself is then read as an operand or under a
        // cast, never as a truth again.
        match (e, read) {
            (
                Expr::Binary {
                    op: BinOp::Eq | BinOp::Ne,
                    at,
                    l,
                    r,
                },
                _,
            ) if !at.is_float() => match (is_zero(l), is_zero(r)) {
                (false, true) => self.test(l),
                (true, false) => self.test(r),
                _ => {}
            },
            (
                Expr::Binary {
                    op: BinOp::LogicalAnd | BinOp::LogicalOr,
                    l,
                    r,
                    ..
                },
                _,
            ) => {
                self.test(l);
                self.test(r);
            }
            (_, Read::Truth) => self.test(e),
            _ => {}
        }
        match e {
            Expr::Var(var) => self.var(*var, read),
            Expr::Sugar { e: inner, .. } => self.expr(inner, read, d),
            Expr::Cast { to, e: inner } => {
                let under = match (read, to) {
                    // Whether 32 bits are zero is the same under either sign.
                    (Read::Truth, Scalar::I32 | Scalar::U32) if self.around.signs => {
                        Read::As(*to, true)
                    }
                    _ => under_cast(*to, read),
                };
                self.expr(inner, &under, d);
            }
            Expr::Binary { op, at, l, r } => {
                let (left, right) = operand_reads(*op, at);
                self.expr(l, &left, d);
                self.expr(r, &right, d);
            }
            Expr::Unary {
                op: UnOp::Not,
                e: inner,
            }
            | Expr::Truth(inner) => self.expr(inner, &Read::Truth, d),
            Expr::Unary {
                op: UnOp::BitNot,
                e: inner,
            } if self.around.signs => match &**inner {
                // `~` flips each bit whatever the sign: the cast under it says 32 bits and no more.
                Expr::Cast {
                    to: to @ (Scalar::I32 | Scalar::U32),
                    e: under,
                } => self.expr(under, &Read::As(*to, true), d),
                other => self.expr(other, &Read::Keep, d),
            },
            Expr::Unary { e: inner, .. } => self.expr(inner, &Read::Keep, d),
            Expr::Select { cond, then, els } => {
                self.expr(cond, &Read::Truth, d);
                self.expr(then, read, d);
                self.expr(els, read, d);
            }
            Expr::Call { callee, args, .. } => {
                let reads = self.around.arg_reads(callee, args, self.wide);
                for (arg, read) in args.iter().zip(&reads) {
                    self.expr(arg, read, d);
                }
            }
            Expr::Load { ty, addr, offset } => {
                self.base(addr, *offset, ty.bytes(), true, d);
                // A cast over a load of a variable's own cell reads the variable as the cast says.
                if let (Expr::AddrOf { var, cell: 0 }, 0, Read::As(outer, weak)) =
                    (&**addr, *offset, read)
                {
                    if outer.bytes() <= ty.bytes() {
                        self.reading(*var, *outer, *weak);
                    }
                }
            }
            Expr::LoadBytes {
                addr,
                offset,
                bytes,
            } => self.base(addr, *offset, *bytes, true, d),
            Expr::ByteOffset { addr, offset } => {
                if let (Some(base), Expr::Const(c)) = (base_of(addr), &**offset) {
                    self.facts.apart.insert((base, c.bits));
                }
                self.expr(addr, &Read::Base, d);
                self.expr(offset, &Read::Cell, d);
            }
            Expr::AddrOf { var, .. } => self.address(*var, true),
            Expr::ExtractBytes { value: inner, .. }
            | Expr::Cell { base: inner, .. }
            | Expr::VirtualFunctionRef {
                receiver: inner, ..
            } => self.expr(inner, &Read::Cell, d),
            Expr::NewArray { len, .. } => self.expr(len, &Read::Hint(Lat::Is(Kind::I32)), d),
            Expr::Cells(items) => {
                for item in items {
                    self.expr(item, &Read::Cell, d);
                }
            }
            Expr::Vector3(parts) => {
                for part in parts.iter() {
                    self.expr(part, &Read::Hint(Lat::Is(Kind::F32)), d);
                }
            }
            Expr::Const(_)
            | Expr::GuidEmpty
            | Expr::ZeroSlots(_)
            | Expr::NewObject(_)
            | Expr::FunctionRef { .. }
            | Expr::BuiltinFunctionRef { .. }
            | Expr::StringConstant { .. }
            | Expr::BuiltinGlobal(_) => {}
        }
    }

    /// Notes that `e` is read only for whether it is zero, where it is a variable bare or under a cast to an integer.
    fn test(&mut self, e: &Expr) {
        if let Some(var) = tested_var(e) {
            let count = self.facts.tested.entry(var).or_default();
            count.1 = count.1.saturating_add(1);
        }
    }

    fn var(&mut self, var: Var, read: &Read) {
        let count = self.facts.tested.entry(var).or_default();
        count.0 = count.0.saturating_add(1);
        let (lat, bytes) = match read {
            Read::Cell | Read::Truth | Read::Keep | Read::Operand { .. } => (Lat::None, 8),
            Read::Hint(lat) => (self.hinted(var, lat), 8),
            Read::Base => (Lat::Is(Kind::Pointer), 8),
            Read::As(to, weak) => {
                self.seen(var, to.bytes());
                return self.reading(var, *to, *weak);
            }
            Read::Low(bytes) => (Lat::None, *bytes),
        };
        self.seen(var, bytes);
        self.note(var, &lat);
    }

    /// What a hint says of `var` at once; a named reference is kept for the end, and says only "a reference" till then.
    fn hinted(&mut self, var: Var, lat: &Lat) -> Lat {
        match lat {
            Lat::Is(Kind::Object(_) | Kind::Array(_)) if !self.untyped(var) => {
                let wanted = self.facts.wanted.entry(var).or_default();
                *wanted = match &*wanted {
                    Lat::None => lat.clone(),
                    held if held == lat => lat.clone(),
                    _ => Lat::Conflict,
                };
                Lat::Is(Kind::Pointer)
            }
            other => other.clone(),
        }
    }

    /// Records a read of `var` as `to`; `weak` where what reads it takes either sign alike.
    // Where `signs` is on, a read under one sign is counted and says "32 bits" alone, so that
    // reads under both are settled together and not a conflict.
    fn reading(&mut self, var: Var, to: Scalar, weak: bool) {
        let counted = !weak && self.around.signs && matches!(to, Scalar::I32 | Scalar::U32);
        if counted && !self.untyped(var) {
            let count = self.facts.signs.entry(var).or_default();
            match to {
                Scalar::I32 => count.0 = count.0.saturating_add(1),
                _ => count.1 = count.1.saturating_add(1),
            }
        }
        self.note(var, &kinds::cast_evidence(to, weak || counted));
    }

    /// Whether no one-cell kind is inferred for a variable.
    fn untyped(&self, var: Var) -> bool {
        matches!(var, Var::Global { .. }) || self.wide.contains(&var)
    }

    fn note(&mut self, var: Var, lat: &Lat) {
        if self.untyped(var) || *lat == Lat::None {
            return;
        }
        let around = self.around;
        let known = self.facts.uses.entry(var).or_default();
        *known = around.join(known, lat);
    }

    fn seen(&mut self, var: Var, bytes: u32) {
        let most = self.facts.read.entry(var).or_insert(0);
        *most = (*most).max(bytes);
    }

    /// Records that the address of `var` is taken; `escapes` unless only to read or write that one cell.
    fn address(&mut self, var: Var, escapes: bool) {
        self.facts.addressed.insert(var);
        match var {
            Var::Local(n) => {
                let lowest = self.facts.lowest_local.map_or(n, |lowest| lowest.min(n));
                self.facts.lowest_local = Some(lowest);
                if escapes {
                    let low = self.facts.locals_escape.map_or(n, |low| low.min(n));
                    self.facts.locals_escape = Some(low);
                }
            }
            Var::Param(n) => {
                if escapes {
                    let low = self.facts.params_escape.map_or(n, |low| low.min(n));
                    self.facts.params_escape = Some(low);
                }
            }
            Var::Value { .. } | Var::Stack { .. } | Var::Global { .. } => {}
        }
    }
}

/// The kinds of a function's variables, and the rewriting they allow.
struct Typing<'a> {
    around: &'a Around<'a>,
    /// Variables of several cells, which no one-cell kind is given.
    wide: HashSet<Var>,
    facts: Facts,
    /// What each one-cell variable is, as far as the evidence goes.
    known: HashMap<Var, Lat>,
    /// Variables whose cell always holds a value widened as their kind widens it.
    normal: HashSet<Var>,
    /// Variables some read takes more bytes of than their kind fills.
    past: RefCell<HashSet<Var>>,
    /// Variables [`Typing::loosen`] took the kind from.
    loose: HashSet<Var>,
}

impl Typing<'_> {
    fn typed(&self, var: Var) -> bool {
        !matches!(var, Var::Global { .. }) && !self.wide.contains(&var)
    }

    /// What a variable is: by the evidence for one this pass types, by the project for a global.
    fn lat(&self, var: Var) -> Lat {
        match var {
            var if self.wide.contains(&var) => Lat::None,
            Var::Global {
                program,
                offset,
                statics,
            } => self.around.global(program, offset, statics),
            var => self.known.get(&var).cloned().unwrap_or_default(),
        }
    }

    fn infer(&mut self, cfg: &cfg::Cfg) {
        self.known = std::mem::take(&mut self.facts.uses);
        if !(self.flow(cfg) && self.fill(cfg) && self.refine(cfg) && self.literals(cfg)) {
            self.known.clear();
        }
    }

    /// Gives each variable assigned a shift of two constants the type the shift is written at, unless the variable is already a number that constant is a value of; `false` when the joins that follow run out of rounds.
    // ⚠ Last, and never among the joins: the type a compiler writes a one-bit constant at says
    // nothing of what reads the constant, so it stands only where nothing else does.
    fn literals(&mut self, cfg: &cfg::Cfg) -> bool {
        let mut given: Vec<(Var, Kind)> = Vec::new();
        each_def(cfg, &mut |target, value| {
            each_literal(value, 0, &mut |bits, at| {
                if self.typed(target) && !kinds::holds(&self.lat(target), bits) {
                    given.push((target, at.clone()));
                }
            });
        });
        if given.is_empty() {
            return true;
        }
        for (var, at) in given {
            let joined = self.around.join(&self.lat(var), &Lat::Is(at));
            self.known.insert(var, joined);
        }
        self.flow(cfg) && self.fill(cfg)
    }

    /// Makes a truth of each variable given both 0 and 1 and nothing else, where every read of it takes only whether it is zero.
    // ⚠ Only one no write through an address can reach: its cell then holds 0 or 1, and each test
    // of it against zero is the variable itself. Given 0 alone it may be a reference never set.
    fn flags(&mut self, cfg: &cfg::Cfg) {
        // Whether each variable is given only such constants, and whether a 0 and a 1 are among them.
        let mut given: HashMap<Var, (bool, bool, bool)> = HashMap::new();
        for stmt in cfg.blocks.iter().flat_map(|block| &block.stmts) {
            let mut note = |target: Var, value: Option<&Expr>| {
                let held = given.entry(target).or_insert((true, false, false));
                match value.and_then(zero_or_one) {
                    Some(true) => held.2 = true,
                    Some(false) => held.1 = true,
                    None => held.0 = false,
                }
            };
            match stmt {
                Stmt::Assign { target, value } => note(*target, Some(value)),
                Stmt::AssignMany { targets, values } if targets.len() == values.len() => {
                    for (target, value) in targets.iter().zip(values) {
                        note(*target, Some(value));
                    }
                }
                // A value of several cells fills several targets: none of them is given a constant.
                Stmt::AssignMany { targets, .. } => {
                    for target in targets {
                        note(*target, None);
                    }
                }
                _ => {}
            }
        }
        for (var, (only, zero, one)) in given {
            let (reads, tests) = self.facts.tested.get(&var).copied().unwrap_or((0, 0));
            let tested = tests > 0 && tests == reads;
            // ⚠ A parameter holds what its caller passed before any assignment here.
            let own = !matches!(var, Var::Param(_));
            let free =
                own && self.typed(var) && self.sealed(var) && kinds::takes_truth(&self.lat(var));
            if only && zero && one && tested && free {
                self.known.insert(var, Lat::Is(Kind::Bool));
            }
        }
    }

    /// Joins what each assignment gives its target until nothing changes; `false` when the rounds run out.
    // ⚠ A join stopped early has not yet met its conflicts, so its kinds must not be used.
    fn flow(&mut self, cfg: &cfg::Cfg) -> bool {
        for _ in 0..MAX_ROUNDS {
            let mut changed = false;
            each_def(cfg, &mut |target, value| {
                if !self.typed(target) {
                    return;
                }
                let given = match steps(target, value) {
                    true => Lat::Int32,
                    false => self.evidence(value, 0),
                };
                let known = self.lat(target);
                let joined = self.around.join(&known, &given);
                if joined != known {
                    self.known.insert(target, joined);
                    changed = true;
                }
            });
            if !changed {
                return true;
            }
        }
        false
    }

    /// Gives a variable nothing types the kind of what it is copied into, where that is one kind; `false` when the joins that follow run out of rounds.
    fn fill(&mut self, cfg: &cfg::Cfg) -> bool {
        for _ in 0..3 {
            let mut wanted: HashMap<Var, Lat> = HashMap::new();
            each_def(cfg, &mut |target, value| {
                let Expr::Var(from) = value else {
                    return;
                };
                if self.typed(*from) && self.lat(*from) == Lat::None {
                    let slot = wanted.entry(*from).or_default();
                    *slot = self.around.join(slot, &self.lat(target));
                }
            });
            wanted.retain(|_, lat| matches!(lat, Lat::Int32 | Lat::Is(_) | Lat::Refs));
            if wanted.is_empty() {
                return true;
            }
            self.known.extend(wanted);
            if !self.flow(cfg) {
                return false;
            }
        }
        true
    }

    /// The rules of `kinds` and of `signs`, each with the joins it feeds; `false` when a join runs out of rounds.
    fn refine(&mut self, cfg: &cfg::Cfg) -> bool {
        let around = self.around;
        if around.kinds {
            if self.handed_on() && !self.flow(cfg) {
                return false;
            }
            if !around.project.fields.is_empty() && !self.fields(cfg) {
                return false;
            }
            self.defaults(cfg);
            self.flags(cfg);
        }
        if around.signs && self.signs(cfg) {
            return self.flow(cfg);
        }
        true
    }

    /// Gives a variable that is some reference and no more the one type every signature takes it as; `true` when any took one.
    fn handed_on(&mut self) -> bool {
        let mut changed = false;
        for (var, lat) in std::mem::take(&mut self.facts.wanted) {
            let open = matches!(self.lat(var), Lat::None | Lat::Is(Kind::Pointer));
            if open && matches!(lat, Lat::Is(_)) {
                self.known.insert(var, lat);
                changed = true;
            }
        }
        changed
    }

    /// Joins in what the project's layouts say of each value loaded from, or stored to, a field of an object of known type.
    // ⚠ What a layout says rests on the kind of the base, which the joins may then change: the
    // kinds stand only when the evidence read off them is the evidence they were found with.
    fn fields(&mut self, cfg: &cfg::Cfg) -> bool {
        let before = self.known.clone();
        let mut held: Vec<(Var, Lat)> = Vec::new();
        for _ in 0..FIELD_ROUNDS {
            let found = self.field_evidence(cfg);
            if found == held {
                return true;
            }
            self.known = before.clone();
            for (var, lat) in &found {
                let joined = self.around.join(&self.lat(*var), lat);
                self.known.insert(*var, joined);
            }
            held = found;
            if !(self.flow(cfg) && self.fill(cfg)) {
                return false;
            }
        }
        self.known = before;
        true
    }

    /// What the layouts say of each variable given a field's bytes or stored into a field, by the kinds now held, in the order of the code.
    fn field_evidence(&self, cfg: &cfg::Cfg) -> Vec<(Var, Lat)> {
        let mut found = Vec::new();
        each_def(cfg, &mut |target, value| {
            if let Expr::Load { ty, addr, offset } = peeled(value, 8) {
                let lat = self.field(addr, *offset, ty.bytes(), 0);
                // ⚠ Four bytes loaded as an `int` are not a float until a read says so.
                let says = !matches!(lat, Lat::None | Lat::Is(Kind::F32));
                if says && self.typed(target) {
                    found.push((target, lat));
                }
            }
        });
        for stmt in cfg.blocks.iter().flat_map(|block| &block.stmts) {
            let Stmt::Store {
                kind,
                addr,
                offset,
                value,
            } = stmt
            else {
                continue;
            };
            let Expr::Var(var) = peeled(value, kind.bytes()) else {
                continue;
            };
            // A reference stored to a field may derive from what the field takes: only a number says what the variable is.
            let lat = match self.field(addr, *offset, kind.bytes(), 0) {
                Lat::Is(of) if kinds::reference(&of) => Lat::None,
                lat => lat,
            };
            if lat != Lat::None && self.typed(*var) {
                found.push((*var, lat));
            }
        }
        found
    }

    /// What the project's layout says the `bytes` bytes at `offset` from `addr` are; nothing where the type `addr` points at, or its field, is not known.
    fn field(&self, addr: &Expr, offset: u32, bytes: u32, depth: u32) -> Lat {
        let found = based(addr, offset).and_then(|(base, offset)| {
            let owner = self.owner(base, depth)?;
            self.around.project.field(owner, offset)
        });
        match found {
            Some(field) if field.bytes == bytes => kinds::field(&field.kind, bytes),
            _ => Lat::None,
        }
    }

    /// The type of the object `addr` points at, where a variable's kind or a field's says.
    fn owner(&self, addr: &Expr, depth: u32) -> Option<TypeKey> {
        if depth > MAX_OWNERS {
            return None;
        }
        let kind = match addr {
            Expr::Var(var) => self.lat(*var).kind(),
            Expr::Cast {
                to: Scalar::I64 | Scalar::U64,
                e,
            } => return self.owner(e, depth + 1),
            Expr::Load {
                ty: Scalar::U64,
                addr: under,
                offset,
            } => self.field(under, *offset, 8, depth + 1).kind(),
            _ => return None,
        };
        match kind {
            Kind::Object(name) => TypeKey::of_name(&name),
            _ => None,
        }
    }

    /// Gives a variable nothing types the width of its widest read, where each value it is given is an integer constant that fits: a `byte`, a `ushort`, or 32 bits of either sign.
    // ⚠ Never one whose cell is read whole, which says more than its low bytes do, nor one whose
    // address is taken, which something else may then fill.
    fn defaults(&mut self, cfg: &cfg::Cfg) {
        // The bytes the constants each variable is given fill; 0 once it is given anything else.
        let mut given: HashMap<Var, u32> = HashMap::new();
        each_def(cfg, &mut |target, value| {
            let bytes = match value {
                Expr::Const(c) => kinds::integer_bytes(c),
                // A shift of two constants is the constant it makes.
                other => literal_shift(other).map_or(0, |(bits, at)| {
                    let kind = at.clone();
                    kinds::integer_bytes(&Const { bits, kind })
                }),
            };
            let held = given.entry(target).or_insert(bytes);
            *held = match bytes.min(*held) {
                0 => 0,
                _ => bytes.max(*held),
            };
        });
        for (var, bytes) in given {
            let read = self.facts.read.get(&var).copied().unwrap_or(0);
            let open = self.typed(var)
                && self.lat(var) == Lat::None
                && !self.facts.addressed.contains(&var);
            if bytes == 0 || !(1..=4).contains(&read) || !open {
                continue;
            }
            let lat = match bytes.max(read) {
                1 => Lat::Is(Kind::U8),
                2 => Lat::Is(Kind::U16),
                _ => Lat::Int32,
            };
            self.known.insert(var, lat);
        }
    }

    /// Settles the sign of each 32-bit integer read under one: the sign an assignment gives its set, else the one more reads take; `true` when any took one.
    // A variable goes with every variable it is copied to or from. Where a set cannot agree, one
    // whose own reads all take one sign keeps that sign, as it does with `signs` off.
    fn signs(&mut self, cfg: &cfg::Cfg) -> bool {
        if self.facts.signs.is_empty() {
            return false;
        }
        let mut groups = Groups::default();
        each_def(cfg, &mut |target, value| {
            if let Expr::Var(from) = value {
                if self.typed(target) && self.typed(*from) {
                    groups.join(target, *from);
                }
            }
        });
        let own = |var: &Var| self.facts.signs.get(var).copied().unwrap_or((0, 0));
        let mut sets: HashMap<Var, Signs> = HashMap::new();
        for (var, lat) in &self.known {
            let set = sets.entry(groups.find(*var)).or_default();
            match lat {
                Lat::None | Lat::Int32 => {}
                Lat::Is(Kind::I32) => set.int = true,
                Lat::Is(Kind::U32) => set.uint = true,
                _ => set.other = true,
            }
            let (signed, unsigned) = own(var);
            set.signed = set.signed.saturating_add(signed);
            set.unsigned = set.unsigned.saturating_add(unsigned);
        }
        let mut settled: Vec<(Var, Kind)> = Vec::new();
        for (var, lat) in &self.known {
            let Some(set) = sets.get(&groups.find(*var)) else {
                continue;
            };
            if *lat != Lat::Int32 {
                continue;
            }
            let (signed, unsigned) = own(var);
            let agreed = !set.other && !(set.int && set.uint);
            let kind = match agreed {
                true if set.int => Kind::I32,
                true if set.uint => Kind::U32,
                true if set.unsigned > set.signed => Kind::U32,
                true if set.signed > 0 => Kind::I32,
                false if unsigned > 0 && signed == 0 => Kind::U32,
                false if signed > 0 && unsigned == 0 => Kind::I32,
                _ => continue,
            };
            settled.push((*var, kind));
        }
        let changed = !settled.is_empty();
        for (var, kind) in settled {
            self.known.insert(var, Lat::Is(kind));
        }
        changed
    }

    /// Finds the variables every assignment leaves widened as their kind widens.
    fn settle(&mut self, cfg: &cfg::Cfg) {
        let normal: HashSet<Var> = self
            .known
            .iter()
            // ⚠ What the project proves of a parameter is an `int` widened by its sign, and no other kind.
            .filter(|(var, lat)| {
                let int = !matches!(var, Var::Param(_)) || lat.kind() == Kind::I32;
                self.sealed(**var) && kinds::narrow(&lat.kind()) && int
            })
            .map(|(var, _)| *var)
            .collect();
        self.normal = normal;
        for _ in 0..MAX_ROUNDS {
            let mut failed = Vec::new();
            each_def(cfg, &mut |target, value| {
                if self.normal.contains(&target) && !self.norm(value, &self.lat(target).kind(), 0) {
                    failed.push(target);
                }
            });
            if failed.is_empty() {
                return;
            }
            for var in failed {
                self.normal.remove(&var);
            }
        }
        self.normal.clear();
    }

    /// Takes the kind from each number narrower than its cell that is read past its kind and not proved widened as it.
    // ⚠ Declared `int`, such a variable reads as 32 bits where the code takes the 64 its cell
    // holds. One that loses its kind is a whole cell to what it is copied into, hence the rounds.
    fn loosen(&mut self, cfg: &cfg::Cfg) {
        for round in 0..=LOOSE_ROUNDS {
            let unproved = |(var, lat): (&Var, &Lat)| {
                (kinds::number_bytes(&lat.kind()).is_some() && !self.normal.contains(var))
                    .then_some(*var)
            };
            let mut loose: Vec<Var> = self.known.iter().filter_map(unproved).collect();
            if loose.is_empty() {
                return;
            }
            if round < LOOSE_ROUNDS {
                self.past.borrow_mut().clear();
                // The reads are those the rewrite itself meets: it runs on a copy, for them alone.
                for block in &cfg.blocks {
                    for stmt in &block.stmts {
                        self.stmt(&mut stmt.clone());
                    }
                    self.term(&mut block.term.clone());
                }
                let past = self.past.borrow();
                loose.retain(|var| past.contains(var));
            }
            if loose.is_empty() {
                return;
            }
            for var in loose {
                self.known.insert(var, Lat::Conflict);
                self.loose.insert(var);
            }
        }
    }

    /// Notes a read of `var` that takes more of its cell than its kind fills.
    fn read(&self, var: Var, read: &Read) {
        let Some(own) = kinds::number_bytes(&self.lat(var).kind()) else {
            return;
        };
        let taken = match read {
            Read::As(to, _) => to.bytes(),
            Read::Low(bytes) => *bytes,
            // What takes the cell as a kind reads as much of it as that kind fills.
            Read::Hint(lat) => kinds::number_bytes(&lat.kind()).unwrap_or(8),
            Read::Operand { at, .. } => kinds::number_bytes(at).unwrap_or(8),
            Read::Cell | Read::Truth | Read::Base | Read::Keep => 8,
        };
        if taken > own {
            self.past.borrow_mut().insert(var);
        }
    }

    /// Whether only an assignment that names the variable writes it.
    fn sealed(&self, var: Var) -> bool {
        match var {
            Var::Local(n) => self.facts.lowest_local.is_none_or(|lowest| n < lowest),
            Var::Value { .. } | Var::Stack { .. } => !self.facts.addressed.contains(&var),
            // A caller sets a parameter: only where the project proves every caller widens it.
            Var::Param(n) => {
                let widened = self.around.widened.is_some_and(|cells| cells.contains(&n));
                widened && !self.facts.addressed.contains(&var)
            }
            Var::Global { .. } => false,
        }
    }

    /// What an expression is, by its own form.
    fn evidence(&self, e: &Expr, depth: u32) -> Lat {
        if depth > MAX_DEPTH {
            return Lat::Conflict;
        }
        match e {
            // An integer literal is whatever reads it; a float literal a source could hold is a float.
            Expr::Const(c) => match c.kind {
                Kind::Bool => Lat::Is(Kind::Bool),
                Kind::F64 => Lat::Is(Kind::F64),
                Kind::F32 if kinds::plausible_float(c.bits as u32) => Lat::Is(Kind::F32),
                _ => Lat::None,
            },
            Expr::Var(var) => self.lat(*var),
            Expr::Cast { to, .. } => Lat::Is(to.kind()),
            Expr::Truth(_) | Expr::Unary { op: UnOp::Not, .. } => Lat::Is(Kind::Bool),
            Expr::Unary { op, e: inner } => {
                // Under an operator a shift of two constants is of the type it is written at: the operator works at it.
                let under = match literal_shift(inner) {
                    Some((_, at)) => Lat::Is(at.clone()),
                    None => self.evidence(inner, depth + 1),
                };
                match (op, under) {
                    (UnOp::Neg, Lat::Is(kind))
                        if matches!(kind, Kind::I32 | Kind::I64 | Kind::F32 | Kind::F64) =>
                    {
                        Lat::Is(kind)
                    }
                    (UnOp::BitNot, Lat::Is(kind))
                        if matches!(kind, Kind::I32 | Kind::U32 | Kind::I64 | Kind::U64) =>
                    {
                        Lat::Is(kind)
                    }
                    _ => Lat::None,
                }
            }
            Expr::Binary { op, .. } if op.is_logical() => Lat::Is(Kind::Bool),
            // A shift of two constants is the constant it makes: an integer literal, as above.
            Expr::Binary { .. } if literal_shift(e).is_some() => Lat::None,
            // A bit operation yields a 32-bit integer of either sign, whichever its casts name.
            Expr::Binary { op, at, .. }
                if either_sign(*op) && matches!(at, Kind::I32 | Kind::U32) =>
            {
                Lat::Int32
            }
            Expr::Binary { at, .. } => match at {
                Kind::Unknown => Lat::None,
                at => Lat::Is(at.clone()),
            },
            Expr::Select { then, els, .. } => {
                let (then, els) = (
                    self.evidence(then, depth + 1),
                    self.evidence(els, depth + 1),
                );
                self.around.join(&then, &els)
            }
            // Four bytes and eight are loaded as they lie; a narrower load names its type.
            Expr::Load { ty, .. } => match ty {
                Scalar::I32 | Scalar::U64 => Lat::None,
                ty => Lat::Is(ty.kind()),
            },
            Expr::Call {
                callee, returns, ..
            } => self.around.yields(callee, *returns).0,
            Expr::NewObject(ty) => Lat::Is(Kind::Object(ty.name.clone())),
            // An array is a reference to its elements: objects, or values held in place.
            Expr::NewArray { ty, .. } if self.around.kinds => {
                let element = match ty.flags & VALUE != 0 {
                    true => Kind::Value {
                        ty: ty.name.clone(),
                        cells: ty.byte_size.div_ceil(8).max(1),
                    },
                    false => Kind::Object(ty.name.clone()),
                };
                Lat::Is(Kind::Array(Box::new(element)))
            }
            Expr::NewArray { .. } | Expr::AddrOf { .. } | Expr::ByteOffset { .. } => {
                Lat::Is(Kind::Pointer)
            }
            Expr::StringConstant { .. } => Lat::Is(Kind::String),
            _ => Lat::None,
        }
    }

    /// Whether the cell of `e` is what a value of kind `k` widens to, so that reading it as `k` changes nothing.
    fn norm(&self, e: &Expr, k: &Kind, depth: u32) -> bool {
        if matches!(k, Kind::I64 | Kind::U64 | Kind::F64) {
            return true;
        }
        if depth > MAX_DEPTH {
            return false;
        }
        let fits = |shape: &Kind| shape == k || kinds::widens(shape, k);
        match e {
            Expr::Const(c) => kinds::normal(k, c.bits),
            Expr::Var(var) => self.normal.contains(var) && fits(&self.lat(*var).kind()),
            Expr::Cast { to, .. } => fits(&to.kind()),
            Expr::Truth(_) | Expr::Unary { op: UnOp::Not, .. } => fits(&Kind::Bool),
            Expr::Binary { op, .. } if op.is_logical() => fits(&Kind::Bool),
            Expr::Binary { at, .. } => match literal_shift(e) {
                Some((bits, _)) => kinds::normal(k, bits),
                None => fits(at),
            },
            Expr::Select { then, els, .. } => {
                self.norm(then, k, depth + 1) && self.norm(els, k, depth + 1)
            }
            Expr::Load { ty, .. } => fits(&ty.kind()),
            Expr::Call {
                callee, returns, ..
            } => {
                let shape = self.around.yields(callee, *returns).1;
                shape.is_some_and(|shape| fits(&shape))
            }
            _ => false,
        }
    }

    /// Whether an expression is a truth value and nothing more.
    fn truth(&self, e: &Expr) -> bool {
        match e {
            Expr::Truth(_) | Expr::Unary { op: UnOp::Not, .. } => true,
            Expr::Binary { op, .. } if op.is_logical() => true,
            other => self.evidence(other, 0) == Lat::Is(Kind::Bool),
        }
    }

    /// Whether only accesses of its one cell go through an address that reaches the variable.
    fn plain(&self, var: Var) -> bool {
        let reached = match var {
            Var::Local(n) => self.facts.locals_escape.is_some_and(|low| n >= low),
            Var::Param(n) => self.facts.params_escape.is_some_and(|low| n >= low),
            Var::Value { .. } | Var::Stack { .. } | Var::Global { .. } => true,
        };
        !reached && !self.wide.contains(&var)
    }

    /// The local or parameter whose own cell a load at `offset` through `addr` reads.
    fn own_read(&self, addr: &Expr, offset: u32) -> Option<Var> {
        match addr {
            Expr::AddrOf { var, cell: 0 } if offset == 0 && self.plain(*var) => Some(*var),
            _ => None,
        }
    }

    /// The local or parameter a write of `bytes` bytes through `addr` sets, where no read of it takes more.
    // ⚠ A narrower write leaves the rest of the cell as it was, which an assignment would not.
    fn own_cell(&self, addr: &Expr, offset: u32, bytes: u32) -> Option<Var> {
        let var = self.own_read(addr, offset)?;
        let read = self.facts.read.get(&var).copied().unwrap_or(0);
        (bytes <= 8 && read <= bytes).then_some(var)
    }

    fn stmt(&self, stmt: &mut Stmt) {
        let replaced = match stmt {
            Stmt::Assign { target, value } => {
                if !steps(*target, value) {
                    self.expr(value, &Read::Hint(self.lat(*target)), 0);
                }
                None
            }
            Stmt::AssignMany { targets, values } => {
                for (target, value) in targets.iter().zip(values.iter_mut()) {
                    self.expr(value, &Read::Hint(self.lat(*target)), 0);
                }
                None
            }
            Stmt::Store {
                kind,
                addr,
                offset,
                value,
            } => {
                self.merge(addr, offset);
                self.expr(value, &store_read(*kind), 0);
                // A constant stored to a field is of the kind the project's layout gives the field.
                if self.around.kinds {
                    if let Expr::Const(c) = &mut *value {
                        kinds::retype(c, &self.field(addr, *offset, kind.bytes(), 0).kind());
                    }
                }
                match self.own_cell(addr, *offset, kind.bytes()) {
                    Some(var) => {
                        let mut value = take(value);
                        if let Expr::Const(c) = &mut value {
                            self.constant(c, &Read::Hint(self.lat(var)));
                        }
                        Some(Stmt::Assign { target: var, value })
                    }
                    None => {
                        self.expr(addr, &Read::Base, 0);
                        None
                    }
                }
            }
            Stmt::StoreBytes {
                addr,
                offset,
                value,
                ..
            }
            | Stmt::StoreF32Slots {
                addr,
                offset,
                value,
            } => {
                self.merge(addr, offset);
                self.expr(addr, &Read::Base, 0);
                self.expr(value, &Read::Cell, 0);
                None
            }
            Stmt::ZeroBytes { addr, bytes } => match self.own_cell(addr, 0, *bytes) {
                Some(var) => {
                    let mut zero = Const::i32(0);
                    self.constant(&mut zero, &Read::Hint(self.lat(var)));
                    Some(Stmt::Assign {
                        target: var,
                        value: Expr::Const(zero),
                    })
                }
                None => {
                    let kind = match (&*addr, *bytes) {
                        // Through a variable's own address a clear stays one: an assignment would
                        // say the whole cell.
                        (Expr::AddrOf { .. }, _) => None,
                        (_, 1) => Some(StoreKind::U8),
                        (_, 2) => Some(StoreKind::U16),
                        (_, 4) => Some(StoreKind::I32),
                        (_, 8) => Some(StoreKind::U64),
                        _ => None,
                    };
                    match kind {
                        Some(kind) => {
                            let (mut addr, mut offset) = (take(addr), 0);
                            self.merge(&mut addr, &mut offset);
                            self.expr(&mut addr, &Read::Base, 0);
                            Some(Stmt::Store {
                                kind,
                                addr,
                                offset,
                                value: Expr::Const(Const::i32(0)),
                            })
                        }
                        None => {
                            self.expr(addr, &Read::Base, 0);
                            None
                        }
                    }
                }
            },
            Stmt::StepI32 { addr, offset, .. } => {
                self.merge(addr, offset);
                self.expr(addr, &Read::Base, 0);
                None
            }
            Stmt::Eval(e) => {
                self.expr(e, &Read::Cell, 0);
                None
            }
        };
        if let Some(new) = replaced {
            *stmt = new;
        }
    }

    fn term(&self, term: &mut Terminator) {
        let read = term_read(term, &self.around.returned);
        if let Some(e) = term.expr_mut() {
            self.expr(e, &read, 0);
        }
    }

    fn expr(&self, e: &mut Expr, read: &Read, depth: u32) {
        if depth > MAX_DEPTH {
            return;
        }
        let d = depth + 1;
        let replaced = match e {
            Expr::Const(c) => {
                self.constant(c, read);
                None
            }
            Expr::Sugar { e: inner, .. } => {
                self.expr(inner, read, d);
                None
            }
            Expr::Cast { to, e: inner } => {
                self.expr(inner, &under_cast(*to, read), d);
                self.cast(*to, inner, read)
            }
            Expr::Binary { op, at, l, r } => {
                let (left, right) = operand_reads(*op, at);
                self.expr(l, &left, d);
                self.expr(r, &right, d);
                self.binary(*op, at, l, r, read)
            }
            Expr::Unary {
                op: UnOp::Not,
                e: inner,
            } => {
                self.expr(inner, &Read::Truth, d);
                Some(negated(take(inner)))
            }
            Expr::Unary { e: inner, .. } => {
                self.expr(inner, &Read::Keep, d);
                None
            }
            Expr::Truth(inner) => {
                self.expr(inner, &Read::Truth, d);
                let folded = match &**inner {
                    Expr::Const(c) => Some(Expr::Const(Const::bool(c.bits != 0))),
                    _ => None,
                };
                match folded {
                    None if self.stands_for_truth(inner, read) => Some(take(inner)),
                    folded => folded,
                }
            }
            Expr::Select { cond, then, els } => {
                self.expr(cond, &Read::Truth, d);
                self.expr(then, read, d);
                self.expr(els, read, d);
                None
            }
            Expr::Call { callee, args, .. } => {
                let reads = self.around.arg_reads(callee, args, &self.wide);
                for (arg, read) in args.iter_mut().zip(&reads) {
                    self.expr(arg, read, d);
                }
                None
            }
            Expr::Load { ty, addr, offset } => {
                self.merge(addr, offset);
                match self.own_read(addr, *offset) {
                    Some(var) => {
                        match *ty {
                            Scalar::U64 => self.read(var, read),
                            ty => self.read(var, &Read::As(ty, false)),
                        }
                        Some(self.read_of(*ty, var, read))
                    }
                    None => {
                        self.expr(addr, &Read::Base, d);
                        None
                    }
                }
            }
            Expr::LoadBytes { addr, offset, .. } => {
                self.merge(addr, offset);
                self.expr(addr, &Read::Base, d);
                None
            }
            Expr::ByteOffset { addr, offset } => {
                self.expr(addr, &Read::Base, d);
                self.expr(offset, &Read::Cell, d);
                None
            }
            Expr::ExtractBytes { value: inner, .. }
            | Expr::Cell { base: inner, .. }
            | Expr::VirtualFunctionRef {
                receiver: inner, ..
            } => {
                self.expr(inner, &Read::Cell, d);
                None
            }
            Expr::NewArray { len, .. } => {
                self.expr(len, &Read::Hint(Lat::Is(Kind::I32)), d);
                None
            }
            Expr::Cells(items) => {
                for item in items {
                    self.expr(item, &Read::Cell, d);
                }
                None
            }
            Expr::Vector3(parts) => {
                for part in parts.iter_mut() {
                    self.expr(part, &Read::Hint(Lat::Is(Kind::F32)), d);
                }
                None
            }
            Expr::Var(var) => {
                self.read(*var, read);
                None
            }
            Expr::GuidEmpty
            | Expr::AddrOf { .. }
            | Expr::ZeroSlots(_)
            | Expr::NewObject(_)
            | Expr::FunctionRef { .. }
            | Expr::BuiltinFunctionRef { .. }
            | Expr::StringConstant { .. }
            | Expr::BuiltinGlobal(_) => None,
        };
        if let Some(new) = replaced {
            *e = new;
        }
    }

    /// Gives a constant the kind of what reads it; its bits stay.
    fn constant(&self, c: &mut Const, read: &Read) {
        let want = match read {
            Read::Hint(lat) => lat.kind(),
            Read::As(to, _) => to.kind(),
            Read::Cell | Read::Truth | Read::Low(_) | Read::Base => Kind::Unknown,
            // An operator converts a literal by the literal's own type, which must stay.
            Read::Operand { .. } | Read::Keep => return,
        };
        kinds::retype(c, &want);
    }

    /// What `as_to(inner)` becomes, `inner` already rewritten, where the cast goes.
    fn cast(&self, to: Scalar, inner: &mut Expr, read: &Read) -> Option<Expr> {
        if let Expr::Const(c) = inner {
            return Some(Expr::Const(Const {
                bits: kinds::cast_bits(to, c.bits),
                kind: to.kind(),
            }));
        }
        if matches!(read, Read::Keep) {
            return None;
        }
        // A load read at once as another type of its width is a load of that type.
        let retyped = match inner {
            Expr::Load { ty, .. } if *ty != to && ty.bytes() == to.bytes() && to != Scalar::I64 => {
                *ty = to;
                true
            }
            _ => false,
        };
        (retyped || self.drops(to, inner, read)).then(|| take(inner))
    }

    /// Whether `as_to(inner)`, read as `read`, says nothing that `inner` does not.
    fn drops(&self, to: Scalar, inner: &Expr, read: &Read) -> bool {
        let k = to.kind();
        let wide = matches!(to, Scalar::I64 | Scalar::U64 | Scalar::F64);
        // A cell already widened as the cast widens it: the cast is the identity, wherever it stands.
        let same = !wide && self.norm(inner, &k, 0);
        match read {
            Read::Keep => false,
            // A read of the low bytes looks through a cast that keeps as many.
            Read::Low(bytes) => to.bytes() >= *bytes || same,
            Read::As(outer, _) => outer.bytes() <= to.bytes() || same,
            Read::Operand { weak, at } => {
                if same {
                    return true;
                }
                // The operator's own type must say what the cast said, or the operand must.
                if k != *at {
                    return false;
                }
                let given = self.evidence(inner, 0);
                // With `signs` on a read under a sign no longer widens a narrower variable's
                // kind, and C# widens such an operand itself; never a `bool`, which it does not.
                let narrower = |from: &Kind| *from != Kind::Bool && kinds::widens(from, &k);
                let promoted =
                    self.around.signs && matches!(&given, Lat::Is(from) if narrower(from));
                if promoted || given == Lat::Is(k) {
                    return true;
                }
                *weak
                    && match to {
                        Scalar::I32 | Scalar::U32 => matches!(
                            given,
                            Lat::Int32 | Lat::Is(Kind::I32 | Kind::U32 | Kind::Bool | Kind::Hash)
                        ),
                        Scalar::I64 | Scalar::U64 => !kinds::thin(&given),
                        _ => false,
                    }
            }
            Read::Cell | Read::Truth | Read::Hint(_) | Read::Base => match to {
                Scalar::I64 | Scalar::U64 => true,
                Scalar::F64 => self.evidence(inner, 0) == Lat::Is(Kind::F64),
                _ => same,
            },
        }
    }

    /// A load of a variable's own cell through its address, as a read of the variable.
    fn read_of(&self, ty: Scalar, var: Var, read: &Read) -> Expr {
        let cell = Expr::Var(var);
        match ty == Scalar::U64 || self.drops(ty, &cell, read) {
            true => cell,
            false => Expr::cast(ty, cell),
        }
    }

    /// What a binary operator over operands already rewritten becomes, where it becomes another expression.
    fn binary(
        &self,
        op: BinOp,
        at: &Kind,
        l: &mut Expr,
        r: &mut Expr,
        read: &Read,
    ) -> Option<Expr> {
        if op == BinOp::Shl {
            if let (Expr::Const(value), Expr::Const(count)) = (&*l, &*r) {
                if let Some(bits) = kinds::shifted(at, value.bits, count.bits) {
                    // A constant like any written whole: it takes the kind of what reads it.
                    let mut folded = Const {
                        bits,
                        kind: at.clone(),
                    };
                    self.constant(&mut folded, read);
                    return Some(Expr::Const(folded));
                }
            }
        }
        if op == BinOp::And {
            // A mask that clears a bit or two reads as the bits it clears.
            for side in [&mut *l, &mut *r] {
                let mask = match side {
                    Expr::Const(c) => kinds::cleared(at, c.bits),
                    _ => None,
                };
                if let Some(bits) = mask {
                    *side = Expr::Unary {
                        op: UnOp::BitNot,
                        e: Box::new(Expr::Const(Const {
                            bits,
                            kind: at.clone(),
                        })),
                    };
                    return None;
                }
            }
        }
        if matches!(op, BinOp::Eq | BinOp::Ne) && !at.is_float() {
            let tested = if is_zero(r) {
                Some(l)
            } else if is_zero(l) {
                Some(r)
            } else {
                None
            };
            // `!x` is a truth value whatever `x` holds; `x` in place of `x != 0` must be one itself.
            let stands = |tested: &Expr| match op {
                BinOp::Eq => self.truth(tested),
                _ => self.stands_for_truth(tested, read),
            };
            if let Some(tested) = tested.filter(|tested| stands(tested)) {
                let tested = take(tested);
                return Some(match op {
                    BinOp::Eq => negated(tested),
                    _ => tested,
                });
            }
        }
        None
    }

    /// Whether `e` may stand where "`e` is not zero" stood, read as `read`.
    // ⚠ A variable or a call said to be a truth value may still hold more than 0 or 1: it stands
    // for its own test only where nothing but the test is taken.
    fn stands_for_truth(&self, e: &Expr, read: &Read) -> bool {
        match read {
            Read::Truth => self.truth(e),
            _ => self.truth(e) && self.norm(e, &Kind::Bool, 0),
        }
    }

    /// Adds a constant `byte_offset` under an access into the access's own offset, unless that address is also a value.
    fn merge(&self, addr: &mut Expr, offset: &mut u32) {
        for _ in 0..8 {
            let merged = match addr {
                Expr::ByteOffset {
                    addr: base,
                    offset: by,
                } => match (base_of(base), &**by) {
                    (Some(key), Expr::Const(c)) if !self.facts.apart.contains(&(key, c.bits)) => {
                        u32::try_from(c.bits)
                            .ok()
                            .and_then(|add| offset.checked_add(add))
                            .map(|sum| (sum, take(base)))
                    }
                    _ => None,
                },
                _ => None,
            };
            let Some((sum, base)) = merged else {
                return;
            };
            *offset = sum;
            *addr = base;
        }
    }

    /// Writes the kinds found into the head, where the printer and the names pass read them.
    fn record(&self, f: &mut cfg::Function) {
        // Only an assignment makes a whole value of several cells; a store fills a part of one.
        let mut wide: Vec<(Var, Kind)> = Vec::new();
        for stmt in f.cfg.blocks.iter().flat_map(|block| &block.stmts) {
            match stmt {
                Stmt::Assign { target, value } if self.wide.contains(target) => {
                    wide.push((*target, self.wide_kind(value)));
                }
                _ => {}
            }
        }
        let mut found: HashMap<Var, Kind> = HashMap::new();
        for (var, kind) in wide {
            let known = found.entry(var).or_insert_with(|| kind.clone());
            if *known != kind {
                *known = Kind::Unknown;
            }
        }
        // A value of several cells an instruction names the type of is a value of that type.
        if self.around.kinds {
            for (var, ty) in values(&f.cfg, f.head.program, &self.wide, self.around.project) {
                let held = found
                    .get(&var)
                    .or_else(|| f.head.vars.get(&var).map(|info| &info.kind));
                if let Some(Kind::Aggregate(cells)) = held.cloned() {
                    found.insert(var, Kind::Value { ty, cells });
                }
            }
        }
        // ⚠ Not over a kind a list gives the value: a vector natives make and take is a vector.
        if self.around.kinds {
            for var in handles(&f.cfg, &self.wide) {
                let unsaid = matches!(found.get(&var), Some(Kind::Aggregate(2) | Kind::Unknown));
                if unsaid {
                    found.insert(var, Kind::Handle);
                }
            }
        }
        for (var, lat) in &self.known {
            found.insert(*var, lat.kind());
        }
        for (var, kind) in found {
            if kind != Kind::Unknown {
                let info = f.head.vars.entry(var);
                let info = info.or_insert_with(|| VarInfo::new(Width::Cell));
                // ⚠ A type another pass proved for a value of several cells says more than the
                // expression that fills it; the cells the values pass counted say less.
                let proved = !matches!(info.kind, Kind::Unknown | Kind::Aggregate(_));
                if !(proved && self.wide.contains(&var)) {
                    info.kind = kind;
                }
            } else if self.loose.contains(&var)
                || (self.around.kinds && self.lat(var) == Lat::Conflict)
            {
                // Evidence that disagrees with a kind another pass wrote leaves the variable none.
                if let Some(info) = f.head.vars.get_mut(&var) {
                    info.kind = Kind::Unknown;
                }
            }
        }
    }

    /// What a value of several cells is, by the expression that makes it.
    fn wide_kind(&self, e: &Expr) -> Kind {
        match e {
            Expr::Vector3(_) => Kind::Vector,
            Expr::GuidEmpty => Kind::Handle,
            Expr::ZeroSlots(cells) => Kind::Aggregate(*cells),
            // ⚠ An item may itself be a value of several cells, and then the count of items is not the count of cells.
            Expr::Cells(items) if items.iter().all(|item| one_cell(item, &self.wide)) => {
                Kind::Aggregate(items.len() as u32)
            }
            Expr::LoadBytes { bytes, .. } | Expr::ExtractBytes { bytes, .. } => {
                Kind::Aggregate(bytes.div_ceil(8))
            }
            Expr::Call {
                callee,
                returns: Some(cells),
                ..
            } if *cells > 1 => {
                let named = match callee {
                    Callee::Native { hash, .. } if *cells == 2 => self
                        .around
                        .names
                        .native(*hash)
                        .and_then(|native| native.returns.as_deref())
                        .map_or(Kind::Unknown, kinds::native_wide),
                    Callee::Intrinsic { name, .. }
                        if *cells == 2 && name.starts_with("packed_vector_") =>
                    {
                        Kind::Vector
                    }
                    _ => Kind::Unknown,
                };
                match named {
                    Kind::Unknown => Kind::Aggregate(*cells),
                    named => named,
                }
            }
            _ => Kind::Unknown,
        }
    }
}

/// The value type each variable of several cells is an instance of, by what names its type: a `box<T>` of the whole variable, or a call of a method of a value type `T` handed the variable's address as its receiver; none where two types are named.
fn values(
    cfg: &cfg::Cfg,
    program: u32,
    wide: &HashSet<Var>,
    project: &Project,
) -> Vec<(Var, String)> {
    let mut named: HashMap<Var, Option<String>> = HashMap::new();
    let mut name = |var: Var, ty: String| {
        let held = named.entry(var).or_insert_with(|| Some(ty.clone()));
        if held.as_deref() != Some(ty.as_str()) {
            *held = None;
        }
    };
    let member_of = |callee: &Callee| {
        let key = signatures::key_of(program, callee)?;
        let member = project
            .members
            .get(&key)
            .filter(|member| !member.is_static)?;
        let facts = project.types.get(&member.owner)?;
        facts.is_value().then(|| member.owner.to_string())
    };
    for block in &cfg.blocks {
        let mut note = |top: &Expr| {
            top.walk(&mut |e| {
                let Expr::Call { callee, args, .. } = e else {
                    return;
                };
                match (callee, args.as_slice()) {
                    (Callee::Intrinsic { name: boxed, .. }, [Expr::Var(var)])
                        if wide.contains(var) =>
                    {
                        let ty = boxed
                            .strip_prefix("box<")
                            .and_then(|ty| ty.strip_suffix('>'));
                        if let Some(ty) = ty {
                            name(*var, ty.to_owned());
                        }
                    }
                    (_, [Expr::AddrOf { var, cell: 0 }, ..]) if wide.contains(var) => {
                        if let Some(ty) = member_of(callee) {
                            name(*var, ty);
                        }
                    }
                    _ => {}
                }
            });
        };
        for stmt in &block.stmts {
            stmt.each_expr(&mut note);
        }
        if let Some(e) = block.term.expr() {
            note(e);
        }
    }
    let mut values: Vec<(Var, String)> = (named.into_iter())
        .filter_map(|(var, ty)| Some((var, ty?)))
        .collect();
    values.sort();
    values
}

/// The values of two cells that only natives make, compare and take: each is assigned only what a native yields in two cells, no value or another such variable, and is read only whole, as an argument or as one side of a comparison of its sixteen bytes.
// ⚠ A value read a cell at a time is two things, or a struct whose fields are read: no handle.
fn handles(cfg: &cfg::Cfg, wide: &HashSet<Var>) -> HashSet<Var> {
    let mut made: HashMap<Var, bool> = HashMap::new();
    let mut copies: Vec<(Var, Var)> = Vec::new();
    let mut whole: HashMap<Var, (u32, u32)> = HashMap::new();
    let note = |top: &Expr, whole: &mut HashMap<Var, (u32, u32)>| {
        top.walk(&mut |e| match e {
            Expr::Var(var) | Expr::AddrOf { var, .. } => whole.entry(*var).or_default().0 += 1,
            Expr::Call { callee, args, .. } => {
                let compared =
                    matches!(callee, Callee::Intrinsic { name, .. } if name == "bytes_equal_16");
                for arg in args {
                    let taken = match (arg, callee, compared) {
                        (Expr::Var(var), Callee::Native { .. }, _) => Some(var),
                        (Expr::AddrOf { var, cell: 0 }, _, true) => Some(var),
                        _ => None,
                    };
                    if let Some(var) = taken {
                        whole.entry(*var).or_default().1 += 1;
                    }
                }
            }
            _ => {}
        });
    };
    for block in &cfg.blocks {
        for stmt in &block.stmts {
            if let Stmt::Assign { target, value } = stmt {
                if wide.contains(target) {
                    let native = matches!(
                        value,
                        Expr::Call {
                            callee: Callee::Native { .. },
                            returns: Some(2),
                            ..
                        }
                    );
                    let held = match value {
                        Expr::Var(from) if wide.contains(from) => {
                            copies.push((*target, *from));
                            whole.entry(*from).or_default().1 += 1;
                            true
                        }
                        _ => native || matches!(value, Expr::GuidEmpty),
                    };
                    *made.entry(*target).or_insert(true) &= held;
                }
            }
            stmt.each_expr(&mut |top| note(top, &mut whole));
            for target in stmt.targets() {
                made.entry(*target).or_insert(true);
            }
            if !matches!(stmt, Stmt::Assign { target, .. } if wide.contains(target)) {
                for target in stmt.targets() {
                    made.insert(*target, false);
                }
            }
        }
        if let Some(e) = block.term.expr() {
            note(e, &mut whole);
        }
    }
    let mut handles: HashSet<Var> = (made.into_iter())
        .filter(|(var, made)| {
            let (stands, taken) = whole.get(var).copied().unwrap_or_default();
            let own = matches!(var, Var::Local(_) | Var::Value { .. } | Var::Stack { .. });
            *made && own && stands == taken
        })
        .map(|(var, _)| var)
        .collect();
    // A copy holds a handle only where what it copies does.
    loop {
        let before = handles.len();
        for (to, from) in &copies {
            if !(handles.contains(to) && handles.contains(from)) {
                handles.remove(to);
                handles.remove(from);
            }
        }
        if handles.len() == before {
            return handles;
        }
    }
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
