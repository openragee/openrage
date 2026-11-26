//! What one function's code says of the functions it calls: how it reads each result, what it
//! passes for each parameter, whose result it hands back and which it takes a reference to.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use super::{key_of, Found, Reader};
use crate::gta6::script::cfg::{self, Terminator};
use crate::gta6::script::ir::{
    BinOp, Callee, Expr, FunctionKey, Head, Kind, Scalar, Stmt, UnOp, Var, Width,
};
use crate::gta6::script::print::cells_of;
use crate::gta6::script::project::Project;

/// The readings that take a whole cell, or its 32 bits, looked through to reach what is read.
const MAX_PEEL: usize = 8;

/// How the value of a call is read where it is used.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Usage {
    /// No read says what it is.
    #[default]
    Unsaid,
    /// For whether it is zero, and nothing else.
    Truth,
    /// As 32 bits whose sign nothing reads: compared for equality, or worked on bit by bit.
    // ⚠ Such a value may count or may name, so this states no kind: it only says "no truth".
    Int,
    // The three that follow: as that type, by an operator that counts with it or orders it.
    I32,
    U32,
    F32,
    /// As two things, or as something no constant is.
    Mixed,
}

impl Usage {
    /// What two readings of one value say together.
    // A test against zero reads a number as well as a truth, so the number stands.
    pub(super) fn join(self, other: Usage) -> Usage {
        match (self, other) {
            (Usage::Unsaid, said) | (said, Usage::Unsaid) => said,
            (a, b) if a == b => a,
            (Usage::Mixed, _) | (_, Usage::Mixed) => Usage::Mixed,
            (Usage::Truth, number) | (number, Usage::Truth) => number,
            (Usage::Int, signed @ (Usage::I32 | Usage::U32))
            | (signed @ (Usage::I32 | Usage::U32), Usage::Int) => signed,
            _ => Usage::Mixed,
        }
    }

    /// The kind a result read this way is, where every constant of `found` is a value of it.
    pub(super) fn kind(self, found: &Found) -> Option<Kind> {
        let fits = |kind: Kind| super::holds(&kind, found).then_some(kind);
        match self {
            Usage::Truth => fits(Kind::Bool),
            Usage::I32 => fits(Kind::I32),
            Usage::U32 => fits(Kind::U32),
            Usage::F32 => fits(Kind::F32),
            Usage::Unsaid | Usage::Int | Usage::Mixed => None,
        }
    }
}

/// What the arguments one function passes for one parameter cell of another say, together.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Given {
    pub(super) to: FunctionKey,
    /// The cell of the callee's parameters the arguments land in.
    pub(super) cell: u32,
    /// `Unknown` where no argument states a kind.
    pub(super) kind: Kind,
    /// The arguments are of two kinds, or one of them is an integer.
    pub(super) dead: bool,
    /// The lowest and the highest integer constant among them, as signed cells.
    pub(super) lo: i64,
    pub(super) hi: i64,
    /// An integer constant among them is no float's bits.
    pub(super) odd: bool,
    /// The functions whose results are among them.
    pub(super) from: Vec<FunctionKey>,
}

impl Given {
    /// What the arguments are once every result is settled; a result of no kind says nothing.
    pub(super) fn found(&self, project: &Project) -> Found {
        let mut found = Found {
            kind: (self.kind != Kind::Unknown).then(|| self.kind.clone()),
            dead: self.dead,
            lo: self.lo,
            hi: self.hi,
            odd: self.odd,
            ..Found::default()
        };
        for function in &self.from {
            let held = project.signatures.get(function);
            let typed = held.filter(|held| held.returns == 1 && held.result != Kind::Unknown);
            if let Some(held) = typed {
                found.join(&Found::of(held.result.clone()).unnumbered());
            }
        }
        found
    }
}

/// What one function's calls say, each list in key order.
pub(super) struct Sites {
    pub(super) uses: Vec<(FunctionKey, Usage)>,
    /// How the result of each native is read, by [`native_key`], where a read says.
    pub(super) natives: Vec<(u64, Usage)>,
    /// For each parameter cell of a callee one argument of one cell lands in, whether every such argument here is an `int` widened by its sign; the cell `u32::MAX` where a call's cells are not all known.
    pub(super) narrow: Vec<(FunctionKey, u32, bool)>,
    pub(super) handed: Vec<FunctionKey>,
    pub(super) taken: Vec<FunctionKey>,
    pub(super) gives: Vec<Given>,
}

impl Sites {
    /// Reads the calls of `f`; `reader` holds what its variables are.
    pub(super) fn of(f: &cfg::Function, reader: &Reader<'_>) -> Sites {
        let mut scan = Scan {
            reader,
            head: &f.head,
            calls: BTreeMap::new(),
            natives: BTreeMap::new(),
            kept_natives: Vec::new(),
            stands: BTreeMap::new(),
            narrow: BTreeMap::new(),
            vars: HashMap::new(),
            kept: Vec::new(),
            handed: BTreeSet::new(),
            taken: BTreeSet::new(),
            given: BTreeMap::new(),
        };
        for block in &f.cfg.blocks {
            for stmt in &block.stmts {
                if let Stmt::Assign { target, value } = stmt {
                    scan.assigned(*target, value);
                }
                // A result no one takes is read as nothing.
                if let Stmt::Eval(Expr::Call { callee, .. }) = stmt {
                    scan.accounted(callee);
                }
                stmt.each_expr(&mut |top| scan.expr(top));
            }
            match &block.term {
                Terminator::Branch { cond, .. } => scan.tested(cond),
                Terminator::Switch { value, .. } => scan.compared(value, &Kind::Unknown),
                Terminator::Return(Some(e)) => scan.returned(e),
                _ => {}
            }
            if let Some(e) = block.term.expr() {
                scan.expr(e);
            }
        }
        scan.done()
    }
}

/// One reading of a function's code for [`Sites`].
struct Scan<'a, 'r> {
    reader: &'a Reader<'r>,
    head: &'a Head,
    /// How each function's result is read, over every call of it here.
    calls: BTreeMap<FunctionKey, Usage>,
    /// How each native's result is read, by [`native_key`].
    natives: BTreeMap<u64, Usage>,
    /// The variables assigned the result of a native, each with the native.
    kept_natives: Vec<(Var, u64)>,
    /// How often each native's result stands, and how often a read above says what it is read as.
    stands: BTreeMap<u64, (u64, u64)>,
    /// Whether every argument of one cell for each parameter cell of each callee is an `int` widened by its sign.
    narrow: BTreeMap<(FunctionKey, u32), bool>,
    /// How each variable is read.
    vars: HashMap<Var, Usage>,
    /// The variables assigned the result of a call, each with the function called.
    kept: Vec<(Var, FunctionKey)>,
    handed: BTreeSet<FunctionKey>,
    taken: BTreeSet<FunctionKey>,
    given: BTreeMap<(FunctionKey, u32), Found>,
}

impl Scan<'_, '_> {
    fn expr(&mut self, top: &Expr) {
        top.walk(&mut |e| self.node(e));
    }

    fn node(&mut self, e: &Expr) {
        match e {
            Expr::Truth(inner)
            | Expr::Unary {
                op: UnOp::Not,
                e: inner,
            } => self.tested(inner),
            Expr::Select { cond, .. } => self.tested(cond),
            Expr::Binary {
                op: BinOp::LogicalAnd | BinOp::LogicalOr,
                l,
                r,
                ..
            } => {
                self.tested(l);
                self.tested(r);
            }
            Expr::Binary {
                op: BinOp::Eq | BinOp::Ne,
                at,
                l,
                r,
            } => match (is_zero(l), is_zero(r)) {
                (_, true) => self.tested(l),
                (true, false) => self.tested(r),
                (false, false) => {
                    self.compared(l, at);
                    self.compared(r, at);
                }
            },
            Expr::Binary {
                op: BinOp::And | BinOp::Xor | BinOp::Or,
                at,
                l,
                r,
            } => {
                self.compared(l, at);
                self.compared(r, at);
            }
            Expr::Binary { at, l, r, .. } => {
                self.worked(l, at);
                self.worked(r, at);
            }
            // ⚠ A cast to 32 bits that no rule above read with its operator says the width alone:
            // an argument or a store keeps 32 bits of a truth as of a number.
            Expr::Cast { to, e: inner } => {
                let usage = match to {
                    Scalar::F32 => Usage::F32,
                    Scalar::I32 | Scalar::U32 | Scalar::I64 | Scalar::U64 => Usage::Unsaid,
                    _ => Usage::Mixed,
                };
                self.note(whole(inner), usage);
            }
            Expr::Call {
                callee,
                args,
                returns,
            } => {
                if let (Some(key), None | Some(1)) = (native_key(callee), returns) {
                    self.stands.entry(key).or_default().0 += 1;
                }
                self.call(callee, args);
            }
            Expr::FunctionRef { program, at } => {
                self.taken.insert(FunctionKey::new(*program, *at));
            }
            // What is written through an address is read by code this does not follow.
            Expr::AddrOf { var, .. } => {
                self.vars.insert(*var, Usage::Mixed);
            }
            _ => {}
        }
    }

    /// Notes that `e` is read as `usage`, where `e` is a variable or a call of a function of a program that yields one cell.
    fn note(&mut self, e: &Expr, usage: Usage) {
        match e {
            Expr::Var(var) => {
                let held = self.vars.entry(*var).or_default();
                *held = held.join(usage);
            }
            Expr::Call {
                callee,
                returns: None | Some(1),
                ..
            } => {
                if let Some(key) = key_of(self.head.program, callee) {
                    let held = self.calls.entry(key).or_default();
                    *held = held.join(usage);
                }
                if let Some(key) = native_key(callee).filter(|_| usage != Usage::Unsaid) {
                    let held = self.natives.entry(key).or_default();
                    *held = held.join(usage);
                    self.accounted(callee);
                }
            }
            _ => {}
        }
    }

    /// Notes that one place a native's result stands at is read as something said.
    fn accounted(&mut self, callee: &Callee) {
        if let Some(key) = native_key(callee) {
            self.stands.entry(key).or_default().1 += 1;
        }
    }

    /// Notes a value read only for whether it is zero; 32 bits of a cell that is zero or one are the cell.
    fn tested(&mut self, e: &Expr) {
        self.note(low(e), Usage::Truth);
    }

    /// Notes a value read by something that takes either sign alike, working at `at`.
    fn compared(&mut self, e: &Expr, at: &Kind) {
        let read = low(e);
        // A cast to 32 bits was looked through, or the operator itself works at 32.
        let int = !std::ptr::eq(read, whole(e)) || matches!(at, Kind::I32 | Kind::U32);
        let usage = match at {
            _ if int => Usage::Int,
            Kind::F32 => Usage::F32,
            Kind::F64 => Usage::Mixed,
            _ => Usage::Unsaid,
        };
        self.note(read, usage);
    }

    /// Notes a value an operator that counts or orders works on at `at`, under the cast to 32 bits that hands it to the operator.
    fn worked(&mut self, e: &Expr, at: &Kind) {
        let usage = match at {
            Kind::I32 => Usage::I32,
            Kind::U32 => Usage::U32,
            Kind::F32 => Usage::F32,
            Kind::F64 => Usage::Mixed,
            _ => Usage::Unsaid,
        };
        let read = match at {
            Kind::I32 | Kind::U32 => low(e),
            _ => whole(e),
        };
        self.note(read, usage);
    }

    /// Notes a variable assigned the result of a call: how the variable is read is how the result is.
    fn assigned(&mut self, target: Var, value: &Expr) {
        if let Expr::Call {
            callee,
            returns: None | Some(1),
            ..
        } = low(value)
        {
            if let Some(key) = key_of(self.head.program, callee) {
                self.kept.push((target, key));
            }
            if let Some(key) = native_key(callee) {
                self.kept_natives.push((target, key));
                self.accounted(callee);
            }
        }
    }

    /// Notes a call whose result the function hands back as its own.
    fn returned(&mut self, e: &Expr) {
        if let Expr::Call {
            callee,
            returns: None | Some(1),
            ..
        } = whole(e)
        {
            if let Some(key) = key_of(self.head.program, callee) {
                self.handed.insert(key);
            }
        }
    }

    /// Notes what each argument of one cell says of the parameter cell it lands in.
    fn call(&mut self, callee: &Callee, args: &[Expr]) {
        let Some(key) = key_of(self.head.program, callee) else {
            return;
        };
        // The cell of the callee's parameters each argument starts at, while the arguments before it say their cells.
        let mut at = Some(0u32);
        for arg in args {
            let Some(cell) = at else {
                // Where the cells do not say, no argument after says which parameter it is.
                self.narrow.insert((key, u32::MAX), false);
                break;
            };
            let cells = cells_of(arg, self.head);
            at = cells.and_then(|cells| cell.checked_add(cells));
            // A parameter passed whole is carried apart, with the function's own kind for it.
            if cells != Some(1) || matches!(arg, Expr::Var(Var::Param(_))) {
                continue;
            }
            *self.narrow.entry((key, cell)).or_insert(true) &= widened_i32(arg);
            let found = self.reader.given(arg, 0);
            if found != Found::default() {
                self.given.entry((key, cell)).or_default().join(&found);
            }
        }
    }

    /// How a variable that holds a result is read: by its declared kind where that is no integer, and by its reads.
    fn kept_as(&self, var: Var) -> Usage {
        if matches!(var, Var::Global { .. }) {
            return Usage::Unsaid;
        }
        let info = self.head.vars.get(&var);
        if info.is_some_and(|info| info.width == Width::Cells) {
            return Usage::Mixed;
        }
        // An integer's reads say which integer, so its declared kind says nothing here.
        let number = |kind: &Kind| matches!(kind, Kind::Unknown | Kind::Hash) || kind.is_integer();
        let declared = match info.map(|info| &info.kind) {
            Some(Kind::Bool) => Usage::Truth,
            Some(Kind::F32) => Usage::F32,
            Some(kind) if !number(kind) => Usage::Mixed,
            _ => Usage::Unsaid,
        };
        declared.join(self.vars.get(&var).copied().unwrap_or_default())
    }

    fn done(mut self) -> Sites {
        let kept = std::mem::take(&mut self.kept);
        for (var, key) in kept {
            let usage = self.kept_as(var);
            let held = self.calls.entry(key).or_default();
            *held = held.join(usage);
        }
        // ⚠ A native's result kept in a variable is read as the variable is, and a variable
        // assigned anything else as well is read as that too: it is no truth by its reads alone.
        let kept = std::mem::take(&mut self.kept_natives);
        for (var, key) in kept {
            // What any code reads a global as is not said here.
            let usage = match var {
                Var::Global { .. } => Usage::Mixed,
                var => self.kept_as(var),
            };
            let held = self.natives.entry(key).or_default();
            *held = held.join(usage);
        }
        // A result that stands where nothing above reads it (an argument, a stored value) is read as anything.
        for (key, (stands, said)) in std::mem::take(&mut self.stands) {
            if stands > said {
                self.natives.insert(key, Usage::Mixed);
            }
        }
        let said = self.calls.into_iter();
        let gives = self.given.into_iter().map(|((to, cell), found)| Given {
            to,
            cell,
            kind: found.kind.unwrap_or(Kind::Unknown),
            dead: found.dead,
            lo: found.lo,
            hi: found.hi,
            odd: found.odd,
            from: found.from.into_iter().collect(),
        });
        let natives = self.natives.into_iter();
        let narrow = self
            .narrow
            .into_iter()
            .map(|((to, cell), narrow)| (to, cell, narrow));
        Sites {
            narrow: narrow.collect(),
            uses: said.filter(|(_, usage)| *usage != Usage::Unsaid).collect(),
            natives: natives
                .filter(|(_, usage)| *usage != Usage::Unsaid)
                .collect(),
            handed: self.handed.into_iter().collect(),
            taken: self.taken.into_iter().collect(),
            gives: gives.collect(),
        }
    }
}

/// Whether `e`, by its own form, is an `int` widened by its sign to its cell: a constant of that range, a reading or a load of 32 bits or fewer that widens so, an operator that works at such a width, a truth.
// ⚠ Not a variable, nor a result: what a variable holds is not said here.
fn widened_i32(e: &Expr) -> bool {
    let small = |ty: &Scalar| {
        matches!(
            ty,
            Scalar::I8 | Scalar::U8 | Scalar::I16 | Scalar::U16 | Scalar::I32
        )
    };
    match e {
        Expr::Const(c) => c.bits as i64 == i64::from(c.bits as i32),
        Expr::Sugar { e, .. } => widened_i32(e),
        Expr::Cast { to, .. } | Expr::Load { ty: to, .. } => small(to),
        Expr::Binary { op, at, .. } => {
            op.is_logical()
                || matches!(
                    at,
                    Kind::Bool | Kind::I8 | Kind::U8 | Kind::I16 | Kind::U16 | Kind::I32
                )
        }
        Expr::Truth(_) | Expr::Unary { op: UnOp::Not, .. } => true,
        _ => false,
    }
}

/// The key a native is known by over a project: its handler's address where the program states one, else its hash.
// ⚠ A compiled program states the address alone, a listing the hash: the same native read from
// both is two keys, which only ever leaves a native of no proved use.
pub(crate) fn native_key(callee: &Callee) -> Option<u64> {
    match callee {
        Callee::Native { rva, .. } if *rva != 0 => Some(u64::from(*rva)),
        Callee::Native { hash, .. } if *hash != 0 => Some(*hash),
        _ => None,
    }
}

/// `e` under the readings that take its whole cell or its 32 bits.
fn low(e: &Expr) -> &Expr {
    let mut at = e;
    for _ in 0..MAX_PEEL {
        match at {
            Expr::Sugar { e: inner, .. }
            | Expr::Cast {
                to: Scalar::I32 | Scalar::U32 | Scalar::I64 | Scalar::U64,
                e: inner,
            } => at = &**inner,
            _ => break,
        }
    }
    at
}

/// `e` under the readings that take its whole cell.
fn whole(e: &Expr) -> &Expr {
    let mut at = e;
    for _ in 0..MAX_PEEL {
        match at {
            Expr::Sugar { e: inner, .. }
            | Expr::Cast {
                to: Scalar::I64 | Scalar::U64,
                e: inner,
            } => at = &**inner,
            _ => break,
        }
    }
    at
}

fn is_zero(e: &Expr) -> bool {
    matches!(e, Expr::Const(c) if c.bits == 0 && !c.kind.is_float())
}
