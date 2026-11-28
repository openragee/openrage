//! What one function's code says of the functions it calls: how it reads each result, what it
//! passes for each parameter, whose result it hands back and which it takes a reference to.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use super::{key_of, Found, Reader, MAX_DEPTH, ROUNDS};
use crate::gta6::script::cfg::{self, Terminator};
use crate::gta6::script::ir::{
    BinOp, Callee, Expr, FunctionKey, Head, Kind, Scalar, Stmt, StoreKind, TypeKey, UnOp, Var,
    Width,
};
use crate::gta6::script::print::cells_of;
use crate::gta6::script::project::Project;

/// The readings that take a whole cell, or its 32 bits, looked through to reach what is read.
const MAX_PEEL: usize = 8;

/// The cells of one argument past which a call is taken to say nothing: more than any function's parameters.
const SEVERAL: u32 = 1024;

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
    /// Where `kind` is a pointer only because the arguments are objects of two or more types: those types.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) objects: Vec<String>,
    /// An argument among them is of no kind its reading says: what it holds is not known.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) vague: bool,
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
            objects: self.objects.iter().cloned().collect(),
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

    /// Whether an argument among them may hold anything: one of no kind, or the result of a function whose result is of none.
    pub(super) fn vague(&self, project: &Project) -> bool {
        let typed = |function: &FunctionKey| {
            let held = project.signatures.get(function);
            held.is_some_and(|held| held.returns == 1 && held.result != Kind::Unknown)
        };
        self.vague || !self.from.iter().all(typed)
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
    /// The natives, by [`native_key`], whose result the function hands back as its own.
    pub(super) returned_natives: Vec<u64>,
    pub(super) taken: Vec<FunctionKey>,
    pub(super) gives: Vec<Given>,
    /// How the result of each virtual slot called through is read, by [`Slot`], where a read says.
    pub(super) slot_uses: Vec<(Slot, Usage)>,
    /// The virtual slots a call through which the function hands back as its own result.
    pub(super) handed_slots: Vec<Slot>,
}

/// A virtual slot as a call names it: the type called through, `None` where no key names it, and the slot.
pub(super) type Slot = (Option<TypeKey>, u32);

/// The slot a virtual call goes through.
fn slot_of(callee: &Callee) -> Option<Slot> {
    match callee {
        Callee::Virtual { ty, slot, .. } => Some((TypeKey::of_name(ty), *slot)),
        _ => None,
    }
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
            call_stands: BTreeMap::new(),
            slot_stands: BTreeMap::new(),
            narrow: BTreeMap::new(),
            changed: changed(f),
            vars: HashMap::new(),
            kept: Vec::new(),
            handed: BTreeSet::new(),
            returned_natives: BTreeSet::new(),
            returned_vars: BTreeSet::new(),
            taken: BTreeSet::new(),
            given: BTreeMap::new(),
            vague: BTreeSet::new(),
            var_stands: HashMap::new(),
            slot_calls: BTreeMap::new(),
            kept_slots: Vec::new(),
            handed_slots: BTreeSet::new(),
        };
        for block in &f.cfg.blocks {
            for stmt in &block.stmts {
                if let Stmt::Assign { target, value } = stmt {
                    scan.assigned(*target, value);
                }
                // A result no one takes is read as nothing.
                if let Stmt::Eval(Expr::Call { callee, .. }) = stmt {
                    scan.accounted(callee);
                    scan.accounted_call(callee);
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
    /// The same for each function's result of one cell, and for each slot's.
    call_stands: BTreeMap<FunctionKey, (u64, u64)>,
    slot_stands: BTreeMap<Slot, (u64, u64)>,
    /// Whether every argument of one cell for each parameter cell of each callee is an `int` widened by its sign.
    narrow: BTreeMap<(FunctionKey, u32), bool>,
    /// The function's own parameters that it assigns or takes the address of: one passed on is no longer what a caller handed.
    changed: BTreeSet<u32>,
    /// How each variable is read.
    vars: HashMap<Var, Usage>,
    /// The variables assigned the result of a call, each with the function called.
    kept: Vec<(Var, FunctionKey)>,
    handed: BTreeSet<FunctionKey>,
    taken: BTreeSet<FunctionKey>,
    given: BTreeMap<(FunctionKey, u32), Found>,
    /// The parameter cells an argument of no kind is passed for.
    vague: BTreeSet<(FunctionKey, u32)>,
    /// The natives whose result the function hands back, and the variables it returns.
    returned_natives: BTreeSet<u64>,
    returned_vars: BTreeSet<Var>,
    /// How often each variable stands, and how often a read above says what it is read as.
    var_stands: HashMap<Var, (u64, u64)>,
    /// How the result of each virtual slot is read, over every call through it here.
    slot_calls: BTreeMap<Slot, Usage>,
    /// The variables assigned the result of a call through a slot, each with the slot.
    kept_slots: Vec<(Var, Slot)>,
    handed_slots: BTreeSet<Slot>,
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
                if matches!(returns, None | Some(1)) {
                    if let Some(key) = key_of(self.head.program, callee) {
                        self.call_stands.entry(key).or_default().0 += 1;
                    }
                    if let Some(slot) = slot_of(callee) {
                        self.slot_stands.entry(slot).or_default().0 += 1;
                    }
                }
                self.call(callee, args);
            }
            Expr::FunctionRef { program, at } => {
                self.taken.insert(FunctionKey::new(*program, *at));
            }
            // A delegate of a slot is called where no code says.
            Expr::VirtualFunctionRef { ty, slot, .. } => {
                let held = self.slot_calls.entry((TypeKey::of(ty), *slot)).or_default();
                *held = held.join(Usage::Mixed);
            }
            // What is written through an address is read by code this does not follow.
            Expr::AddrOf { var, .. } => {
                self.vars.insert(*var, Usage::Mixed);
            }
            Expr::Var(var) => self.var_stands.entry(*var).or_default().0 += 1,
            _ => {}
        }
    }

    /// Notes that `e` is read as `usage`, where `e` is a variable or a call of a function of a program that yields one cell.
    fn note(&mut self, e: &Expr, usage: Usage) {
        match e {
            Expr::Var(var) => {
                let held = self.vars.entry(*var).or_default();
                *held = held.join(usage);
                if usage != Usage::Unsaid {
                    self.var_stands.entry(*var).or_default().1 += 1;
                }
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
                if let Some(slot) = slot_of(callee) {
                    let held = self.slot_calls.entry(slot).or_default();
                    *held = held.join(usage);
                }
                if usage != Usage::Unsaid {
                    self.accounted_call(callee);
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

    /// Notes that one place a function's or a slot's result stands at is read as something said, or kept where its reads are followed.
    fn accounted_call(&mut self, callee: &Callee) {
        if let Some(key) = key_of(self.head.program, callee) {
            self.call_stands.entry(key).or_default().1 += 1;
        }
        if let Some(slot) = slot_of(callee) {
            self.slot_stands.entry(slot).or_default().1 += 1;
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
            if let Some(slot) = slot_of(callee) {
                self.kept_slots.push((target, slot));
            }
            self.accounted_call(callee);
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
            if let Some(slot) = slot_of(callee) {
                self.handed_slots.insert(slot);
            }
            self.accounted_call(callee);
            if let Some(key) = native_key(callee) {
                self.returned_natives.insert(key);
                self.accounted(callee);
            }
        }
        if let Expr::Var(var) = whole(e) {
            self.returned_vars.insert(*var);
            // Handed back, it is read as the function's result is: the project's to say.
            self.var_stands.entry(*var).or_default().1 += 1;
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
            match (cells, arg) {
                // A parameter passed whole is carried apart, with the function's own kind for it.
                (Some(1), Expr::Var(Var::Param(n))) if !self.changed.contains(n) => continue,
                (Some(1), _) => {}
                // A value of several cells lands in each of them, and none is an `int` by that.
                (Some(cells), _) if cells <= SEVERAL => {
                    for taken in cell..cell.saturating_add(cells) {
                        self.narrow.insert((key, taken), false);
                    }
                    continue;
                }
                // Where the cells do not say, or say more than a function takes, this argument proves nothing of where it lands.
                _ => {
                    self.narrow.insert((key, u32::MAX), false);
                    break;
                }
            }
            *self.narrow.entry((key, cell)).or_insert(true) &= widened_i32(arg);
            let found = self.reader.given(arg, 0);
            let entry = self.given.entry((key, cell)).or_default();
            // ⚠ No kind is not nothing: zero is a value of every kind, anything else may hold
            // what no other argument does.
            match found != Found::default() {
                true => {
                    entry.join(&found);
                }
                false if !matches!(arg, Expr::Const(c) if c.bits == 0) => {
                    self.vague.insert((key, cell));
                }
                false => {}
            }
        }
    }

    /// How a variable that holds a result is read: by its declared kind where that is no integer, and by its reads.
    // ⚠ What any code reads a global as is not said here, nor what a variable that stands where
    // nothing above reads it (an argument, a value stored, a copy) is read as.
    fn kept_as(&self, var: Var) -> Usage {
        let (stands, said) = self.var_stands.get(&var).copied().unwrap_or_default();
        if matches!(var, Var::Global { .. }) || stands > said {
            return Usage::Mixed;
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

    /// How a variable that holds a native's result is read: by its reads alone, as wide as it is.
    fn held_as(&self, var: Var) -> Usage {
        let info = self.head.vars.get(&var);
        if info.is_some_and(|info| info.width == Width::Cells) {
            return Usage::Mixed;
        }
        self.vars.get(&var).copied().unwrap_or_default()
    }

    fn done(mut self) -> Sites {
        let kept = std::mem::take(&mut self.kept);
        // A result kept in a variable the function hands back is read as the function's result is.
        for (var, key) in kept {
            if self.returned_vars.contains(&var) {
                self.handed.insert(key);
            }
            let usage = self.kept_as(var);
            let held = self.calls.entry(key).or_default();
            *held = held.join(usage);
        }
        for (var, slot) in std::mem::take(&mut self.kept_slots) {
            if self.returned_vars.contains(&var) {
                self.handed_slots.insert(slot);
            }
            let usage = self.kept_as(var);
            let held = self.slot_calls.entry(slot).or_default();
            *held = held.join(usage);
        }
        // A result that stands where nothing above reads it (an argument, a stored value) is read as anything.
        for (key, (stands, said)) in std::mem::take(&mut self.call_stands) {
            if stands > said {
                self.calls.insert(key, Usage::Mixed);
            }
        }
        for (slot, (stands, said)) in std::mem::take(&mut self.slot_stands) {
            if stands > said {
                self.slot_calls.insert(slot, Usage::Mixed);
            }
        }
        // ⚠ A native's result kept in a variable is read as the variable is, and a variable
        // assigned anything else as well is read as that too: it is no truth by its reads alone.
        let kept = std::mem::take(&mut self.kept_natives);
        for (var, key) in kept {
            if self.returned_vars.contains(&var) {
                self.returned_natives.insert(key);
            }
            // What any code reads a global as is not said here.
            // ⚠ Nor is what a variable that stands where nothing above reads it (an argument, a
            // value stored, a copy) is read as, whatever kind it is declared: a `bool` passed on
            // or stored passes on what the native gave.
            let (stands, said) = self.var_stands.get(&var).copied().unwrap_or_default();
            let usage = match var {
                Var::Global { .. } => Usage::Mixed,
                _ if stands > said => Usage::Mixed,
                var => self.held_as(var),
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
        let vague = self.vague;
        let gives = self.given.into_iter().map(|((to, cell), found)| Given {
            vague: vague.contains(&(to, cell)),
            to,
            cell,
            kind: found.kind.unwrap_or(Kind::Unknown),
            dead: found.dead,
            lo: found.lo,
            hi: found.hi,
            odd: found.odd,
            from: found.from.into_iter().collect(),
            objects: found.objects.into_iter().collect(),
        });
        let natives = self.natives.into_iter();
        let slot_uses = self.slot_calls.into_iter();
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
            returned_natives: self.returned_natives.into_iter().collect(),
            taken: self.taken.into_iter().collect(),
            gives: gives.collect(),
            slot_uses: slot_uses
                .filter(|(_, usage)| *usage != Usage::Unsaid)
                .collect(),
            handed_slots: self.handed_slots.into_iter().collect(),
        }
    }
}

/// The parameters `f` assigns, or takes the address of, anywhere in its code.
pub(super) fn changed(f: &cfg::Function) -> BTreeSet<u32> {
    let mut changed = BTreeSet::new();
    let mut note = |var: &Var| {
        if let Var::Param(n) = var {
            changed.insert(*n);
        }
    };
    for block in &f.cfg.blocks {
        for stmt in &block.stmts {
            stmt.targets().iter().for_each(&mut note);
            stmt.each_expr(&mut |top| {
                top.walk(&mut |e| {
                    if let Expr::AddrOf { var, .. } = e {
                        note(var);
                    }
                })
            });
        }
        if let Some(e) = block.term.expr() {
            e.walk(&mut |e| {
                if let Expr::AddrOf { var, .. } = e {
                    note(var);
                }
            });
        }
    }
    changed
}

/// The caller's parameter cells and the callees' results past which an argument is taken to say nothing.
const MAX_RESTS: usize = 64;

/// What every argument one function's calls, as first read, hand one parameter cell of another is: an `int` widened by its sign, or not known to be.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Widening {
    pub(super) to: FunctionKey,
    /// The cell of the callee's parameters the arguments land in; `u32::MAX` where a call's cells are not all known.
    pub(super) cell: u32,
    /// Every argument is so by its form, or where each of `params` and `results` is.
    pub(super) proved: bool,
    /// The caller's own parameter cells handed on there as they came.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) params: Vec<u32>,
    /// The functions whose result of one cell is handed on there.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) results: Vec<FunctionKey>,
}

/// What one function's code as first read, before any pass, says of the cells its calls hand on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Firsts {
    /// By callee and cell, in key order.
    pub(super) widening: Vec<Widening>,
    /// The functions the code takes a reference to, in key order.
    pub(super) taken: Vec<FunctionKey>,
}

impl Firsts {
    /// Reads `f` as it is: each argument is what the code pushes, and nothing a pass reads into it.
    // ⚠ After `values` and `types` a cell reads as every use of it agrees, which is not how it was
    // widened: a load, a cast or an operator is known for what it makes only as first read.
    pub(super) fn of(f: &cfg::Function) -> Firsts {
        let first = First::of(f);
        let mut cells: BTreeMap<(FunctionKey, u32), Option<Rests>> = BTreeMap::new();
        let mut taken: BTreeSet<FunctionKey> = BTreeSet::new();
        let mut visit = |e: &Expr| match e {
            Expr::Call { callee, args, .. } => first.call(callee, args, &mut cells),
            Expr::FunctionRef { program, at } => {
                taken.insert(FunctionKey::new(*program, *at));
            }
            _ => {}
        };
        for block in &f.cfg.blocks {
            for stmt in &block.stmts {
                stmt.each_expr(&mut |top| top.walk(&mut visit));
            }
            if let Some(e) = block.term.expr() {
                e.walk(&mut visit);
            }
        }
        let widening = cells.into_iter().map(|((to, cell), rests)| {
            let proved = rests.is_some();
            let rests = rests.unwrap_or_default();
            Widening {
                to,
                cell,
                proved,
                params: rests.params.into_iter().collect(),
                results: rests.results.into_iter().collect(),
            }
        });
        Firsts {
            widening: widening.collect(),
            taken: taken.into_iter().collect(),
        }
    }
}

/// What a value is an `int` widened by its sign on, where it is one: the caller's parameter cells and the callees' results among it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Rests {
    params: BTreeSet<u32>,
    results: BTreeSet<FunctionKey>,
}

/// What both `a` and `b` rest on; `None` where either is no such value, or they rest on too much.
fn both(a: Option<Rests>, b: Option<Rests>) -> Option<Rests> {
    let (mut a, b) = (a?, b?);
    a.params.extend(b.params);
    a.results.extend(b.results);
    (a.params.len() + a.results.len() <= MAX_RESTS).then_some(a)
}

/// A function's code as it was first read, for whether each value is an `int` widened by its sign.
struct First<'f> {
    head: &'f Head,
    /// The parameter cells that may no longer hold what the caller handed: assigned, or at or past one whose address is taken.
    changed: BTreeSet<u32>,
    /// Each variable only its assignments write, with what all of them make it.
    vars: HashMap<Var, Option<Rests>>,
}

impl<'f> First<'f> {
    fn of(f: &'f cfg::Function) -> First<'f> {
        let mut lowest_local: Option<u32> = None;
        let mut lowest_param: Option<u32> = None;
        let mut addressed: BTreeSet<Var> = BTreeSet::new();
        let mut note = |e: &Expr| {
            if let Expr::AddrOf { var, .. } = e {
                match *var {
                    Var::Local(n) => lowest_local = Some(lowest_local.map_or(n, |low| low.min(n))),
                    Var::Param(n) => lowest_param = Some(lowest_param.map_or(n, |low| low.min(n))),
                    _ => {}
                }
                addressed.insert(*var);
            }
        };
        for block in &f.cfg.blocks {
            for stmt in &block.stmts {
                stmt.each_expr(&mut |top| top.walk(&mut note));
            }
            if let Some(e) = block.term.expr() {
                e.walk(&mut note);
            }
        }
        let mut changed = self::changed(f);
        if let Some(low) = lowest_param {
            changed.extend(low..f.head.params.max(low));
        }
        let one = |var: &Var| {
            let info = f.head.vars.get(var);
            info.is_none_or(|info| info.width == Width::Cell)
        };
        // ⚠ A frame cell an address reaches is written by whatever has the address: a local at or
        // past the lowest one whose address is taken, and every local once a parameter's is, as
        // nothing bounds what that address reaches. A frame starts as zeroes, an `int` so widened.
        let sealed = |var: &Var| match var {
            Var::Local(n) => {
                lowest_param.is_none() && lowest_local.is_none_or(|low| *n < low) && one(var)
            }
            Var::Value { .. } | Var::Stack { .. } => !addressed.contains(var) && one(var),
            Var::Param(_) | Var::Global { .. } => false,
        };
        let mut sets: Vec<(Var, Option<&Expr>)> = Vec::new();
        for stmt in f.cfg.blocks.iter().flat_map(|block| &block.stmts) {
            match stmt {
                Stmt::Assign { target, value } => sets.push((*target, Some(value))),
                // A value of several cells spread over the targets is no one value of each.
                Stmt::AssignMany { targets, values } => {
                    let paired = targets.len() == values.len();
                    for (n, target) in targets.iter().enumerate() {
                        sets.push((*target, values.get(n).filter(|_| paired)));
                    }
                }
                _ => {}
            }
        }
        sets.retain(|(target, _)| sealed(target));
        let mut first = First {
            head: &f.head,
            changed,
            vars: HashMap::new(),
        };
        for (target, _) in &sets {
            first.vars.insert(*target, Some(Rests::default()));
        }
        // Each round takes a variable out or adds to what it rests on, never the other way: the
        // largest assignment of what every variable is that each assignment keeps.
        let mut moving = true;
        for _ in 0..ROUNDS {
            moving = false;
            for (target, value) in &sets {
                let given = value.and_then(|value| first.eval(value, 0));
                let held = first.vars.get(target).cloned().flatten();
                let joined = both(held.clone(), given);
                if joined != held {
                    moving = true;
                    first.vars.insert(*target, joined);
                }
            }
            if !moving {
                break;
            }
        }
        if moving {
            // A join stopped early has not met every assignment.
            first.vars.values_mut().for_each(|held| *held = None);
        }
        first
    }

    /// What `e` is an `int` widened by its sign on, where it is one by its form.
    // Each form as the code computes it: a constant of an `int`'s range is its own cell; a load or
    // a reading of eight, sixteen or thirty-two bits widens them by their sign, or by zeroes where
    // they are fewer than thirty-two; an operator at such a width yields its 32 bits widened by
    // their sign; a comparison, a truth and a negation yield zero or one.
    fn eval(&self, e: &Expr, depth: u32) -> Option<Rests> {
        if depth > MAX_DEPTH {
            return None;
        }
        let d = depth + 1;
        let int = || Some(Rests::default());
        match e {
            Expr::Const(c) => (c.bits as i64 == i64::from(c.bits as i32)).then(Rests::default),
            Expr::Sugar { e: inner, .. }
            | Expr::Cast {
                to: Scalar::I64 | Scalar::U64,
                e: inner,
            } => self.eval(inner, d),
            Expr::Cast { to, .. } => narrow(&to.kind()).then(Rests::default),
            Expr::Load { ty, .. } => narrow(&ty.kind()).then(Rests::default),
            Expr::Truth(_) | Expr::Unary { op: UnOp::Not, .. } => int(),
            Expr::Unary { e: inner, .. } => narrow(&inner.listed_kind()).then(Rests::default),
            Expr::Binary { op, .. } if op.is_logical() => int(),
            Expr::Binary {
                op: BinOp::And | BinOp::Or | BinOp::Xor,
                at: Kind::Bool,
                ..
            } => int(),
            Expr::Binary { at, .. } => narrow(at).then(Rests::default),
            Expr::Select { then, els, .. } => both(self.eval(then, d), self.eval(els, d)),
            Expr::Var(var @ Var::Param(n)) => {
                let info = self.head.vars.get(var);
                let one = info.is_none_or(|info| info.width == Width::Cell);
                let kept = one && !self.changed.contains(n);
                kept.then(|| Rests {
                    params: BTreeSet::from([*n]),
                    ..Rests::default()
                })
            }
            Expr::Var(var) => self.vars.get(var).cloned().flatten(),
            Expr::Call {
                callee,
                returns: None | Some(1),
                ..
            } => key_of(self.head.program, callee).map(|key| Rests {
                results: BTreeSet::from([key]),
                ..Rests::default()
            }),
            _ => None,
        }
    }

    /// Notes what each argument of a call of a function of a program is, at the parameter cell it lands in.
    // ⚠ The same accounting as [`Scan::call`]: a value of several cells lands in each of them as
    // no `int`, and where an argument's cells are not known no argument says where it lands.
    fn call(
        &self,
        callee: &Callee,
        args: &[Expr],
        cells: &mut BTreeMap<(FunctionKey, u32), Option<Rests>>,
    ) {
        let Some(key) = key_of(self.head.program, callee) else {
            return;
        };
        let mut at = Some(0u32);
        for arg in args {
            let Some(cell) = at else {
                cells.insert((key, u32::MAX), None);
                break;
            };
            let width = cells_of(arg, self.head);
            at = width.and_then(|width| cell.checked_add(width));
            match width {
                Some(1) => {
                    let held = cells
                        .entry((key, cell))
                        .or_insert_with(|| Some(Rests::default()));
                    *held = both(held.take(), self.eval(arg, 0));
                }
                Some(width) if width <= SEVERAL => {
                    for taken in cell..cell.saturating_add(width) {
                        cells.insert((key, taken), None);
                    }
                }
                _ => {
                    cells.insert((key, u32::MAX), None);
                    break;
                }
            }
        }
    }
}

/// Whether a value of `kind` is an integer of 32 bits or fewer that the VM widens to a cell as an `int` of it is.
fn narrow(kind: &Kind) -> bool {
    matches!(
        kind,
        Kind::I8 | Kind::U8 | Kind::I16 | Kind::U16 | Kind::I32
    )
}

/// Whether `e`, by its own form, is an `int` widened by its sign to its cell: a constant of that range, a truth, a comparison.
// ⚠ Not a variable, nor a result. Nor a load, a reading or an operator: the facts are read off
// the code as the kinds pass leaves it, whose readings and widths are those every read of the
// value agrees on, and a call hands on the whole cell, whose upper half they do not state.
fn widened_i32(e: &Expr) -> bool {
    match e {
        Expr::Const(c) => c.bits as i64 == i64::from(c.bits as i32),
        Expr::Sugar { e, .. } => widened_i32(e),
        Expr::Binary { op, .. } => op.is_logical() || op.is_comparison(),
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

/// How one reading takes the low 32 bits of a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Low {
    I32,
    U32,
    F32,
    /// As four bytes stored, which says no kind.
    Bits,
}

impl Low {
    /// The reading of each operand of an operator that works at `at`.
    fn at(at: &Kind) -> Option<Low> {
        match at {
            Kind::I32 => Some(Low::I32),
            Kind::U32 => Some(Low::U32),
            Kind::F32 => Some(Low::F32),
            _ => None,
        }
    }

    /// The reading a cast to `to` makes of what it casts.
    fn cast(to: Scalar) -> Option<Low> {
        match to {
            Scalar::I32 => Some(Low::I32),
            Scalar::U32 => Some(Low::U32),
            Scalar::F32 => Some(Low::F32),
            _ => None,
        }
    }
}

/// The parameter cells of `open` that `f` reads only as the low 32 bits of one kind, besides passing them whole to functions of a program, each with that kind; `counts` holds by cell every mention and the mentions as such an argument.
// ⚠ Every mention counts: one that is neither such a reading nor such an argument (a value
// stored, returned or tested whole, a copy, an item of a group, a native's argument) leaves the
// cell out. Four bytes stored say no kind, so they stand only beside a reading that does.
pub(super) fn read_alone(
    f: &cfg::Function,
    open: &BTreeSet<u32>,
    counts: &[(u32, u32)],
) -> Vec<(u32, Kind)> {
    let mut found: Vec<(u32, Low)> = Vec::new();
    for block in &f.cfg.blocks {
        for stmt in &block.stmts {
            if let Stmt::Store {
                kind: StoreKind::I32,
                value: Expr::Var(Var::Param(n)),
                ..
            } = stmt
            {
                found.push((*n, Low::Bits));
            }
            stmt.each_expr(&mut |top| lows(top, &mut found));
        }
        if let Some(e) = block.term.expr() {
            lows(e, &mut found);
        }
    }
    let mut reads: BTreeMap<u32, (u32, BTreeSet<Low>)> = BTreeMap::new();
    for (n, low) in found {
        if open.contains(&n) {
            let held = reads.entry(n).or_default();
            held.0 = held.0.saturating_add(1);
            held.1.insert(low);
        }
    }
    let alone = reads.into_iter().filter_map(|(n, (read, lows))| {
        let (mentions, passed) = counts.get(n as usize).copied()?;
        if read.checked_add(passed) != Some(mentions) {
            return None;
        }
        let said: Vec<Low> = lows.into_iter().filter(|low| *low != Low::Bits).collect();
        let kind = match said.as_slice() {
            [Low::I32] => Kind::I32,
            [Low::U32] => Kind::U32,
            [Low::F32] => Kind::F32,
            _ => return None,
        };
        Some((n, kind))
    });
    alone.collect()
}

/// Notes each parameter cell `top` reads, at any depth, as its low 32 bits: an operand of an operator that works at 32 bits, or what a cast to 32 bits casts.
fn lows(top: &Expr, found: &mut Vec<(u32, Low)>) {
    let mut note = |e: &Expr, low: Low| {
        if let Expr::Var(Var::Param(n)) = e {
            found.push((*n, low));
        }
    };
    top.walk(&mut |e| match e {
        Expr::Binary { op, at, l, r } if !op.is_logical() => {
            if let Some(low) = Low::at(at) {
                note(l, low);
                note(r, low);
            }
        }
        Expr::Cast { to, e: inner } => {
            if let Some(low) = Low::cast(*to) {
                note(inner, low);
            }
        }
        _ => {}
    });
}
