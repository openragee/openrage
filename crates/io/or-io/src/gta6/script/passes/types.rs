//! Types: what each variable, constant and memory access is, from how the code uses it.
//!
//! Evidence is joined per variable to a fixed point; then each cast the kinds make redundant
//! goes, a constant takes the kind of what reads it and an access through an address is typed.

use std::collections::{HashMap, HashSet};

use crate::gta6::script::cfg::{self, Terminator};
use crate::gta6::script::ir::{
    BinOp, Callee, Const, Expr, Kind, Scalar, Stmt, StoreKind, UnOp, Var, VarInfo, Width,
};
use crate::gta6::script::names::Names;
use crate::gta6::script::Stats;

#[path = "types_kinds.rs"]
mod kinds;

use kinds::Lat;

/// Expressions nested deeper than this leave their function as the pass found it.
const MAX_DEPTH: u32 = 192;
/// The rounds a fixed point may take before the function is left untyped.
const MAX_ROUNDS: usize = 24;

/// Infers the kind of every variable and constant, then drops each cast the kinds make redundant and types each access.
pub fn run(f: &mut cfg::Function, names: &Names, _stats: &mut Stats) {
    let Some(mut facts) = Facts::of(f, names) else {
        return;
    };
    if narrowings(f) {
        match Facts::of(f, names) {
            Some(again) => facts = again,
            None => return,
        }
    }
    let mut typing = Typing {
        names,
        wide: wide_of(f),
        facts,
        known: HashMap::new(),
        normal: HashSet::new(),
    };
    typing.infer(&f.cfg);
    typing.settle(&f.cfg);
    for block in &mut f.cfg.blocks {
        for stmt in &mut block.stmts {
            typing.stmt(stmt);
        }
        typing.term(&mut block.term);
    }
    typing.record(f);
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
    /// An expression was nested too deep to read.
    deep: bool,
}

impl Facts {
    /// Reads a function; `None` where an expression is nested too deep to follow.
    fn of(f: &cfg::Function, names: &Names) -> Option<Facts> {
        let wide = wide_of(f);
        let mut facts = Facts::default();
        let mut scan = Scan {
            names,
            wide: &wide,
            facts: &mut facts,
        };
        scan.function(&f.cfg);
        (!facts.deep).then_some(facts)
    }
}

/// The variables a function declares as several cells.
fn wide_of(f: &cfg::Function) -> HashSet<Var> {
    let wide = f
        .head
        .vars
        .iter()
        .filter(|(_, info)| info.width == Width::Cells);
    wide.map(|(var, _)| *var).collect()
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

/// How a terminator takes its expression.
fn term_read(term: &Terminator) -> Read {
    match term {
        // A switch compares as an `int`, and equality reads either sign alike.
        Terminator::Switch { .. } => Read::As(Scalar::I32, true),
        Terminator::Branch { .. } => Read::Truth,
        _ => Read::Cell,
    }
}

/// How a call takes each of its arguments.
fn arg_reads(names: &Names, callee: &Callee, passed: usize) -> Vec<Read> {
    match callee {
        Callee::Intrinsic { name, .. } => (0..passed)
            .map(|index| match kinds::intrinsic_reads(name, index) {
                Some(ty) => Read::As(ty, false),
                None => Read::Cell,
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
    names: &'a Names,
    wide: &'a HashSet<Var>,
    facts: &'a mut Facts,
}

impl Scan<'_> {
    fn function(&mut self, cfg: &cfg::Cfg) {
        for block in &cfg.blocks {
            for stmt in &block.stmts {
                self.stmt(stmt);
            }
            if let Some(e) = block.term.expr() {
                self.expr(e, &term_read(&block.term), 0);
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
        match e {
            Expr::Var(var) => self.var(*var, read),
            Expr::Cast { to, e: inner } => self.expr(inner, &under_cast(*to, read), d),
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
            Expr::Unary { e: inner, .. } => self.expr(inner, &Read::Keep, d),
            Expr::Select { cond, then, els } => {
                self.expr(cond, &Read::Truth, d);
                self.expr(then, read, d);
                self.expr(els, read, d);
            }
            Expr::Call { callee, args, .. } => {
                let reads = arg_reads(self.names, callee, args.len());
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
                        self.note(*var, &kinds::cast_evidence(*outer, *weak));
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

    fn var(&mut self, var: Var, read: &Read) {
        let (lat, bytes) = match read {
            Read::Cell | Read::Truth | Read::Keep | Read::Operand { .. } => (Lat::None, 8),
            Read::Hint(lat) => (lat.clone(), 8),
            Read::Base => (Lat::Is(Kind::Pointer), 8),
            Read::As(to, weak) => (kinds::cast_evidence(*to, *weak), to.bytes()),
            Read::Low(bytes) => (Lat::None, *bytes),
        };
        self.seen(var, bytes);
        self.note(var, &lat);
    }

    fn note(&mut self, var: Var, lat: &Lat) {
        if matches!(var, Var::Global { .. }) || self.wide.contains(&var) || *lat == Lat::None {
            return;
        }
        let known = self.facts.uses.entry(var).or_default();
        *known = known.join(lat);
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
    names: &'a Names,
    /// Variables of several cells, which no one-cell kind is given.
    wide: HashSet<Var>,
    facts: Facts,
    /// What each one-cell variable is, as far as the evidence goes.
    known: HashMap<Var, Lat>,
    /// Variables whose cell always holds a value widened as their kind widens it.
    normal: HashSet<Var>,
}

impl Typing<'_> {
    fn typed(&self, var: Var) -> bool {
        !matches!(var, Var::Global { .. }) && !self.wide.contains(&var)
    }

    fn lat(&self, var: Var) -> Lat {
        match self.typed(var) {
            true => self.known.get(&var).cloned().unwrap_or_default(),
            false => Lat::None,
        }
    }

    fn infer(&mut self, cfg: &cfg::Cfg) {
        self.known = std::mem::take(&mut self.facts.uses);
        match self.flow(cfg) {
            true => self.fill(cfg),
            false => self.known.clear(),
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
                let joined = known.join(&given);
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

    /// Gives a variable nothing types the kind of what it is copied into, where that is one kind.
    fn fill(&mut self, cfg: &cfg::Cfg) {
        for _ in 0..3 {
            let mut wanted: HashMap<Var, Lat> = HashMap::new();
            each_def(cfg, &mut |target, value| {
                let Expr::Var(from) = value else {
                    return;
                };
                if self.typed(*from) && self.lat(*from) == Lat::None {
                    let slot = wanted.entry(*from).or_default();
                    *slot = slot.join(&self.lat(target));
                }
            });
            wanted.retain(|_, lat| matches!(lat, Lat::Int32 | Lat::Is(_)));
            if wanted.is_empty() {
                return;
            }
            self.known.extend(wanted);
            if !self.flow(cfg) {
                self.known.clear();
                return;
            }
        }
    }

    /// Finds the variables every assignment leaves widened as their kind widens.
    fn settle(&mut self, cfg: &cfg::Cfg) {
        let normal: HashSet<Var> = self
            .known
            .iter()
            .filter(|(var, lat)| self.sealed(**var) && kinds::narrow(&lat.kind()))
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

    /// Whether only an assignment that names the variable writes it.
    fn sealed(&self, var: Var) -> bool {
        match var {
            Var::Local(n) => self.facts.lowest_local.is_none_or(|lowest| n < lowest),
            Var::Value { .. } | Var::Stack { .. } => !self.facts.addressed.contains(&var),
            // A caller sets a parameter, and nothing here says how it widened the cell.
            Var::Param(_) | Var::Global { .. } => false,
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
            Expr::Unary { op, e: inner } => match (op, self.evidence(inner, depth + 1)) {
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
            },
            Expr::Binary { op, .. } if op.is_logical() => Lat::Is(Kind::Bool),
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
            Expr::Select { then, els, .. } => self
                .evidence(then, depth + 1)
                .join(&self.evidence(els, depth + 1)),
            // Four bytes and eight are loaded as they lie; a narrower load names its type.
            Expr::Load { ty, .. } => match ty {
                Scalar::I32 | Scalar::U64 => Lat::None,
                ty => Lat::Is(ty.kind()),
            },
            Expr::Call {
                callee, returns, ..
            } => yields(self.names, callee, *returns).0,
            Expr::NewObject(ty) => Lat::Is(Kind::Object(ty.name.clone())),
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
            Expr::Binary { at, .. } => fits(at),
            Expr::Select { then, els, .. } => {
                self.norm(then, k, depth + 1) && self.norm(els, k, depth + 1)
            }
            Expr::Load { ty, .. } => fits(&ty.kind()),
            Expr::Call {
                callee, returns, ..
            } => yields(self.names, callee, *returns)
                .1
                .is_some_and(|shape| fits(&shape)),
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
        let read = term_read(term);
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
                let reads = arg_reads(self.names, callee, args.len());
                for (arg, read) in args.iter_mut().zip(&reads) {
                    self.expr(arg, read, d);
                }
                None
            }
            Expr::Load { ty, addr, offset } => {
                self.merge(addr, offset);
                match self.own_read(addr, *offset) {
                    Some(var) => Some(self.read_of(*ty, var, read)),
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
            Expr::Var(_)
            | Expr::GuidEmpty
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
                if given == Lat::Is(k) {
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
                    return Some(Expr::Const(Const {
                        bits,
                        kind: at.clone(),
                    }));
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
        for (var, lat) in &self.known {
            found.insert(*var, lat.kind());
        }
        for (var, kind) in found {
            if kind != Kind::Unknown {
                let info = f.head.vars.entry(var);
                info.or_insert_with(|| VarInfo::new(Width::Cell)).kind = kind;
            }
        }
    }

    /// Whether an expression is one cell wide by its form.
    fn one_cell(&self, e: &Expr) -> bool {
        match e {
            Expr::Var(var) => !self.wide.contains(var),
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

    /// What a value of several cells is, by the expression that makes it.
    fn wide_kind(&self, e: &Expr) -> Kind {
        match e {
            Expr::Vector3(_) => Kind::Vector,
            Expr::GuidEmpty => Kind::Handle,
            Expr::ZeroSlots(cells) => Kind::Aggregate(*cells),
            // ⚠ An item may itself be a value of several cells, and then the count of items is not the count of cells.
            Expr::Cells(items) if items.iter().all(|item| self.one_cell(item)) => {
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

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
