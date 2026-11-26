//! Signatures and the call graph: what each function takes and returns, from its own table row
//! and its own code, then carried along the calls, and who calls whom.
//!
//! A result's kind is stated only where every cell the function can hand back is widened as that
//! kind, so that reading the result as that kind says nothing. A parameter's kind is how the
//! function reads the cell, or how the functions it only passes the cell on to read it.
//!
//! Then the calls say what a function's own code does not: a result of constants alone takes the
//! kind every use of it reads, where each constant is a value of that kind; a parameter nothing
//! types takes the one kind the arguments of every call agree on, never an integer, whose sign
//! is the callee's to say; and the functions that fill one virtual slot down a line of types
//! share what they agree on. A function a slot or a delegate reaches takes nothing from the
//! calls that name it, which are not all its calls.
//!
//! ⚠ Between [`of_function`] and [`settle`] a signature may end in a parameter of no cells, which
//! carries what its kinds still wait on. Until `settle` has taken it off, `result` is the word of
//! the function's own code alone and proves nothing: `settled` tells.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::{Param, Project, Signature};
use crate::gta6::script::cfg::{self, Terminator};
use crate::gta6::script::ir::{
    Callee, Const, Expr, FunctionKey, Head, Kind, Returns, Scalar, Stmt, Var, Width,
};
use crate::gta6::script::passes::types::yielded_shape;
use crate::gta6::script::print::cells_of;
use crate::gta6::script::program::Program;
use crate::gta6::script::Names;

#[path = "signatures_shared.rs"]
mod shared;
#[path = "signatures_sites.rs"]
mod sites;

use shared::Shared;
pub(crate) use sites::native_key;
use sites::{Given, Sites, Usage};

/// The parameter cells a function may have: a table or a head that states more is not believed.
const MAX_PARAMS: u32 = 1024;
/// The rounds a fixed point is given; what has not settled by then stays unknown.
const ROUNDS: usize = 64;
/// Expressions nested deeper than this are taken to be of no known kind.
const MAX_DEPTH: u32 = 192;
/// The functions the cells of one result may come from; past that, the result is taken to be of no known kind.
const MAX_FROM: usize = 64;
/// A count of returned cells that two functions state differently.
const CLASH: u32 = u32::MAX;
/// The runtime's own comparisons of two 16-byte handles, each handle given as two cells.
const HANDLE_TESTS: [&str; 2] = ["System.Guid.op_Equality", "System.Guid.op_Inequality"];

/// Adds the cells each function of a compiled program takes and returns, as its table states them.
pub fn of_program(project: &mut Project, program: &Program) {
    let one = Param {
        kind: Kind::Unknown,
        cells: 1,
    };
    // ⚠ Every parameter cell is an entry: the cells a table may state in all are held to the bytes of its code.
    let mut left = program.code.len();
    for row in &program.functions {
        let cells = row.params as usize;
        if row.params > MAX_PARAMS || cells > left {
            continue;
        }
        left -= cells;
        let signature = Signature {
            params: vec![one.clone(); cells],
            returns: row.returns,
            result: Kind::Unknown,
        };
        let key = FunctionKey::new(program.program, row.at);
        project.signatures.insert(key, signature);
    }
}

/// Adds the calls one function makes, and what its code shows of its own parameters and result.
pub fn of_function(project: &mut Project, f: &cfg::Function) {
    let head = &f.head;
    let key = FunctionKey::new(head.program, head.at);
    let body = Body::of(f);
    if !body.calls.is_empty() {
        project
            .calls
            .insert(key, body.calls.iter().copied().collect());
    }
    // A compiled program's table has said the cells already; a listing says them here.
    let stated = project.signatures.remove(&key).map(|held| held.returns);
    if head.params > MAX_PARAMS {
        return;
    }
    let mut open = Open::default();
    let returns = match head.returns {
        Returns::Void => Some(0),
        Returns::Cell => Some(1),
        Returns::Cells => stated.or_else(|| several(f, &mut open)),
    };
    let Some(returns) = returns else {
        return;
    };
    let names = Names::default();
    let reader = Reader::of(f, &names, &body);
    let mut result = Kind::Unknown;
    if head.returns == Returns::Cell {
        let found = returned(f, &reader);
        // Constants alone are of no kind, but a function that hands them on must know of them.
        let waits = !found.from.is_empty() || found.kind.is_none();
        match (found.dead, waits) {
            (true, _) => {}
            (false, true) => {
                open.live = true;
                (open.lo, open.hi, open.odd) = (found.lo, found.hi, found.odd);
                open.returned = found.from.iter().copied().collect();
                result = found.kind.clone().unwrap_or(Kind::Unknown);
            }
            (false, false) => {
                let kind = found.kind.clone().filter(|kind| holds(kind, &found));
                result = kind.unwrap_or(Kind::Unknown);
            }
        }
    }

    let groups = body.groups();
    let mut params = Vec::new();
    let mut waiting = BTreeSet::new();
    let mut cell = 0;
    while cell < head.params {
        if let Some((cells, handle)) = groups.get(&cell).copied() {
            let kind = match handle {
                true => Kind::Handle,
                false => Kind::Aggregate(cells),
            };
            params.push(Param { kind, cells });
            cell += cells;
            continue;
        }
        let kind = own_kind(head, cell);
        let passed = body.cells.get(cell as usize).is_some_and(Cell::only_passed);
        if kind == Kind::Unknown && passed {
            waiting.insert(cell);
        }
        params.push(Param { kind, cells: 1 });
        cell += 1;
    }
    let forwards = body.forwards.iter();
    open.forwards = forwards
        .filter(|forward| waiting.contains(&forward.0))
        .copied()
        .collect();
    open.forwards.sort_unstable();
    open.forwards.dedup();
    open.passes.clone_from(&body.forwards);
    open.passes.sort_unstable();
    open.passes.dedup();
    let sites = Sites::of(f, &reader);
    for (native, usage) in &sites.natives {
        *project.native_uses.entry(*native).or_insert(true) &= *usage == Usage::Truth;
    }
    open.uses = sites.uses;
    open.handed = sites.handed;
    open.taken = sites.taken;
    open.gives = sites.gives;

    let mut signature = Signature {
        params,
        returns,
        result,
    };
    if open != Open::default() && !park(&mut signature, &open) {
        if open.wide {
            return;
        }
        signature.result = Kind::Unknown;
    }
    project.signatures.insert(key, signature);
}

/// Moves the signatures and the calls of `from` into `into`; a function both hold keeps what `into` has.
pub fn absorb(into: &mut Project, from: &mut Project) {
    for (key, signature) in std::mem::take(&mut from.signatures) {
        into.signatures.entry(key).or_insert(signature);
    }
    for (key, callees) in std::mem::take(&mut from.calls) {
        into.calls.entry(key).or_insert(callees);
    }
    for (native, truth) in std::mem::take(&mut from.native_uses) {
        *into.native_uses.entry(native).or_insert(true) &= truth;
    }
}

/// Once every part is in: carries kinds along the calls until nothing changes, a bounded number of rounds.
pub fn settle(project: &mut Project) {
    let mut open = BTreeMap::new();
    for (key, signature) in &mut project.signatures {
        if let Some(waits) = unpark(signature) {
            open.insert(*key, waits);
        }
    }
    let shared = Shared::of(project, &open);
    results(project, &open, &shared);
    parameters(project, &open);
    shared::passed(project, &open, &shared);
    shared::overridden(project, &shared);
    shared::slot_results(project, &shared);
    let uses = std::mem::take(&mut project.native_uses);
    project.native_truths = uses
        .into_iter()
        .filter(|(_, truth)| *truth)
        .map(|(native, _)| native)
        .collect();
}

/// The function a call reaches, where it is a function of a program: this one's, or another's by its hash and offset.
pub(crate) fn key_of(program: u32, callee: &Callee) -> Option<FunctionKey> {
    match callee {
        Callee::Local { at, .. } => Some(FunctionKey::new(program, *at)),
        Callee::Import { program, at, .. } => Some(FunctionKey::new(*program, *at)),
        _ => None,
    }
}

/// Whether a signature waits on nothing: [`settle`] has run over it, or it never waited.
pub(crate) fn settled(signature: &Signature) -> bool {
    signature.params.last().is_none_or(|last| last.cells != 0)
}

/// The place in `params` of the parameter that starts at cell `cell` of a function's parameters.
pub(crate) fn param_at(signature: &Signature, cell: u32) -> Option<usize> {
    let mut at = 0u32;
    for (n, param) in signature.params.iter().enumerate() {
        if at == cell && param.cells > 0 {
            return Some(n);
        }
        at = at.checked_add(param.cells)?;
        if at > cell {
            return None;
        }
    }
    None
}

/// The parameter cells a function only ever passes whole to functions of a program, so that no reading of its own types them.
pub(crate) fn passed_on(f: &cfg::Function) -> Vec<u32> {
    let body = Body::of(f);
    let cells = (0u32..).zip(&body.cells);
    cells
        .filter(|(_, cell)| cell.only_passed())
        .map(|(n, _)| n)
        .collect()
}

/// What a signature still waits on once the function's own code is read; kept at its end until [`settle`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Open {
    /// Whether the result's kind is still to be found: `result` is then what the function's own code says so far.
    live: bool,
    /// The lowest and the highest constant the function returns, as signed cells.
    lo: i64,
    hi: i64,
    /// Whether the function returns several cells and its own code does not say how many.
    wide: bool,
    /// The functions whose result the function hands back as its own.
    returned: Vec<FunctionKey>,
    /// Each parameter cell the function only passes on: the cell, the function given it, and the cell of that function's parameters it lands in.
    forwards: Vec<(u32, FunctionKey, u32)>,
    /// Whether an integer the function returns is no float's bits.
    #[serde(default)]
    odd: bool,
    /// Each parameter cell the function passes whole, whatever else it does with it: the cell, the function given it, and the cell it lands in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    passes: Vec<(u32, FunctionKey, u32)>,
    /// How the function reads the result of each function it calls, where a read says anything.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    uses: Vec<(FunctionKey, Usage)>,
    /// The functions a call of which the function returns, whatever else it returns.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    handed: Vec<FunctionKey>,
    /// The functions the function takes a reference to, as a delegate is made of one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    taken: Vec<FunctionKey>,
    /// What the function passes for each parameter cell of the functions it calls, where an argument says anything.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    gives: Vec<Given>,
}

/// Puts what a signature waits on at its end, as a parameter of no cells; `false` where it cannot be written.
fn park(signature: &mut Signature, open: &Open) -> bool {
    let Ok(text) = serde_json::to_string(open) else {
        return false;
    };
    let kind = Kind::Value { ty: text, cells: 0 };
    signature.params.push(Param { kind, cells: 0 });
    true
}

/// Takes what [`park`] put off a signature; `None` for one that waits on nothing.
fn unpark(signature: &mut Signature) -> Option<Open> {
    if settled(signature) {
        return None;
    }
    let open: Option<Open> = match signature.params.pop()?.kind {
        Kind::Value { ty, .. } => serde_json::from_str(&ty).ok(),
        _ => None,
    };
    if open.is_none() {
        // What was waited on is lost, so what the function's own code said proves nothing.
        signature.result = Kind::Unknown;
    }
    open
}

/// What a run of cells is, as far as has been found.
#[derive(Clone, Debug, Default, PartialEq)]
struct Found {
    /// The kind every cell so far is widened as, where they have one.
    kind: Option<Kind>,
    /// A cell of no known kind is among them, or cells of two kinds are.
    dead: bool,
    /// The lowest and the highest integer constant among them, as signed cells; zero is a value of every kind.
    lo: i64,
    hi: i64,
    /// An integer constant among them is no float's bits.
    odd: bool,
    /// The functions whose results are among them.
    from: BTreeSet<FunctionKey>,
}

impl Found {
    fn dead() -> Found {
        Found {
            dead: true,
            ..Found::default()
        }
    }

    fn of(kind: Kind) -> Found {
        Found {
            kind: Some(kind),
            ..Found::default()
        }
    }

    /// The same, as what a caller may say of a parameter: an integer is of no kind a caller states.
    // ⚠ An integer's sign, and whether it counts or names, is for the callee's own reads to say:
    // a kind stated on a caller's word would be held against them, and both would be lost.
    fn unnumbered(self) -> Found {
        match self.kind.as_ref().is_some_and(number) {
            true => Found::dead(),
            false => self,
        }
    }

    /// Adds what `other` says; returns whether that changed anything.
    fn join(&mut self, other: &Found) -> bool {
        if self.dead {
            return false;
        }
        let clash =
            matches!((&self.kind, &other.kind), (Some(mine), Some(theirs)) if mine != theirs);
        let held = self.from.len();
        self.from.extend(other.from.iter().copied());
        if other.dead || clash || self.from.len() > MAX_FROM {
            *self = Found::dead();
            return true;
        }
        let mut changed = self.from.len() != held;
        if self.kind.is_none() && other.kind.is_some() {
            self.kind.clone_from(&other.kind);
            changed = true;
        }
        let (lo, hi) = (self.lo.min(other.lo), self.hi.max(other.hi));
        changed |= (lo, hi) != (self.lo, self.hi);
        (self.lo, self.hi) = (lo, hi);
        changed |= other.odd && !self.odd;
        self.odd |= other.odd;
        changed
    }
}

/// Whether every integer constant among the cells of `found` is, as a cell, what a value of `kind` widens to.
fn holds(kind: &Kind, found: &Found) -> bool {
    let (lo, hi) = (found.lo, found.hi);
    let within = |least: i64, most: i64| least <= lo && hi <= most;
    match kind {
        Kind::Bool => within(0, 1),
        Kind::I8 => within(i8::MIN.into(), i8::MAX.into()),
        Kind::U8 => within(0, u8::MAX.into()),
        Kind::I16 => within(i16::MIN.into(), i16::MAX.into()),
        Kind::U16 => within(0, u16::MAX.into()),
        Kind::I32 => within(i32::MIN.into(), i32::MAX.into()),
        Kind::U32 | Kind::Hash => within(0, u32::MAX.into()),
        Kind::I64 | Kind::U64 => true,
        // An integer among floats is one only as the bits of a float.
        Kind::F32 => !found.odd,
        // An integer among references is one only as zero.
        _ => within(0, 0),
    }
}

/// Whether a cell is the bits of a float a source would hold: nothing above them, or, unless `exact`, the sign an `int` of those bits widens with.
pub(crate) fn float_bits(cell: u64, exact: bool) -> bool {
    let (low, high) = (cell & 0xFFFF_FFFF, cell >> 32);
    let widened = !exact && high == 0xFFFF_FFFF && low >> 31 == 1;
    plausible(low) && (high == 0 || widened)
}

/// Whether 32 bits are a float a source would hold: not a denormal, which is a small integer, nor a NaN.
fn plausible(bits: u64) -> bool {
    let exponent = (bits >> 23) & 0xFF;
    let no_number = exponent == 0xFF && bits & 0x007F_FFFF != 0;
    bits <= u64::from(u32::MAX) && exponent != 0 && !no_number
}

/// What a constant says of the cells it is among: a kind for a truth or a float, a bound for an integer, nothing for zero.
// `exact` asks of an integer that is a float's bits that its cell be the float's cell too.
fn constant(c: &Const, exact: bool) -> Found {
    match &c.kind {
        _ if c.bits == 0 => Found::default(),
        Kind::Bool if c.bits == 1 => Found::of(Kind::Bool),
        Kind::F32 if plausible(c.bits) => Found::of(Kind::F32),
        Kind::F64 => Found::of(Kind::F64),
        kind if kind.is_integer() || matches!(kind, Kind::Hash | Kind::F32) => {
            let cell = c.bits as i64;
            Found {
                lo: cell.min(0),
                hi: cell.max(0),
                odd: !float_bits(c.bits, exact),
                ..Found::default()
            }
        }
        _ => Found::dead(),
    }
}

/// What an operator that works at `kind` yields: a cell widened as that kind, for a kind operators work at.
fn computed(kind: Kind) -> Found {
    match kind {
        Kind::Bool | Kind::I32 | Kind::U32 | Kind::I64 | Kind::U64 | Kind::F32 | Kind::F64 => {
            Found::of(kind)
        }
        _ => Found::dead(),
    }
}

/// Reads what the expressions of one function are, by their form and by what its variables hold.
struct Reader<'a> {
    head: &'a Head,
    names: &'a Names,
    /// What each variable that only assignments write holds, joined over those assignments.
    vars: HashMap<Var, Found>,
}

impl<'a> Reader<'a> {
    /// Reads what each variable of `f` that only assignments write holds.
    fn of(f: &'a cfg::Function, names: &'a Names, body: &Body) -> Reader<'a> {
        let mut reader = Reader {
            head: &f.head,
            names,
            vars: HashMap::new(),
        };
        // ⚠ Only a variable nothing but an assignment writes: a parameter is the caller's, and a
        // local at or above one whose address is taken may be written through that address.
        let sealed = |var: Var| match var {
            Var::Local(n) => body.lowest_local.is_none_or(|low| n < low),
            Var::Value { .. } | Var::Stack { .. } => !body.addressed.contains(&var),
            Var::Param(_) | Var::Global { .. } => false,
        };
        let mut sets: Vec<(Var, &Expr)> = Vec::new();
        for stmt in f.cfg.blocks.iter().flat_map(|block| &block.stmts) {
            match stmt {
                Stmt::Assign { target, value } => sets.push((*target, value)),
                Stmt::AssignMany { targets, values } => {
                    sets.extend(targets.iter().copied().zip(values));
                }
                _ => {}
            }
        }
        sets.retain(|(target, _)| sealed(*target));
        for (target, _) in &sets {
            reader.vars.entry(*target).or_default();
        }
        let mut moving = true;
        for _ in 0..ROUNDS {
            moving = false;
            for (target, value) in &sets {
                let given = reader.expr(value, 0);
                if let Some(held) = reader.vars.get_mut(target) {
                    moving |= held.join(&given);
                }
            }
            if !moving {
                break;
            }
        }
        if moving {
            // A join stopped early has not met all it holds.
            reader.vars.clear();
        }
        reader
    }
}

impl Reader<'_> {
    fn expr(&self, e: &Expr, depth: u32) -> Found {
        if depth > MAX_DEPTH {
            return Found::dead();
        }
        let d = depth + 1;
        match e {
            Expr::Const(c) => constant(c, true),
            Expr::Var(var) => self.var(*var),
            Expr::Sugar { e: inner, .. } => self.expr(inner, d),
            // All 64 bits are the cell itself.
            Expr::Cast {
                to: Scalar::I64 | Scalar::U64,
                e: inner,
            } => self.expr(inner, d),
            Expr::Cast { to, .. } => Found::of(to.kind()),
            Expr::Truth(_) => Found::of(Kind::Bool),
            Expr::Binary { op, .. } if op.is_logical() => Found::of(Kind::Bool),
            Expr::Binary { at, .. } => computed(at.clone()),
            Expr::Unary { .. } => computed(e.listed_kind()),
            Expr::Select { then, els, .. } => {
                let mut found = self.expr(then, d);
                found.join(&self.expr(els, d));
                found
            }
            Expr::Call {
                callee, returns, ..
            } => match key_of(self.head.program, callee) {
                Some(key) if matches!(returns, None | Some(1)) => Found {
                    from: BTreeSet::from([key]),
                    ..Found::default()
                },
                Some(_) => Found::dead(),
                // The runtime's own operations say how they widen what they yield.
                None => match yielded_shape(self.names, callee, *returns) {
                    Kind::Unknown => Found::dead(),
                    shape => Found::of(shape),
                },
            },
            // Four bytes and eight are loaded as they lie, whatever they are.
            Expr::Load {
                ty: Scalar::I32 | Scalar::U64,
                ..
            } => Found::dead(),
            Expr::Load { ty, .. } => Found::of(ty.kind()),
            Expr::NewObject(ty) => Found::of(Kind::Object(ty.name.clone())),
            Expr::NewArray { .. } | Expr::AddrOf { .. } | Expr::ByteOffset { .. } => {
                Found::of(Kind::Pointer)
            }
            Expr::StringConstant { .. } => Found::of(Kind::String),
            _ => Found::dead(),
        }
    }

    fn var(&self, var: Var) -> Found {
        let info = self.head.vars.get(&var);
        if info.is_some_and(|info| info.width == Width::Cells) {
            return Found::dead();
        }
        // A kind that fills the cell and that the uses of the variable prove is the cell, however it came there.
        let filling = info.map(|info| &info.kind).filter(|kind| {
            matches!(
                kind,
                Kind::I64 | Kind::U64 | Kind::F64 | Kind::Pointer | Kind::String
            )
        });
        match (filling, self.vars.get(&var)) {
            (Some(kind), _) => Found::of(kind.clone()),
            (None, Some(found)) => found.clone(),
            (None, None) => Found::dead(),
        }
    }

    /// What an argument says of the parameter it is passed as: a kind where the code states one, nothing where it states none.
    // ⚠ Not what `expr` proves: a variable's declared kind is how its function reads it, never
    // how its cell is widened, so this types a parameter and may take no cast off anything.
    fn given(&self, e: &Expr, depth: u32) -> Found {
        if depth > MAX_DEPTH {
            return Found::default();
        }
        let d = depth + 1;
        match e {
            Expr::Var(var) => self.held(*var),
            Expr::Sugar { e: inner, .. }
            | Expr::Cast {
                to: Scalar::I64 | Scalar::U64,
                e: inner,
            } => self.given(inner, d),
            // Two kinds on the two ways of one argument are a parameter of neither.
            Expr::Select { then, els, .. } => {
                let mut found = self.given(then, d);
                found.join(&self.given(els, d));
                found
            }
            Expr::Const(c) => said(constant(c, false)),
            other => said(self.expr(other, d)),
        }
    }

    /// What a variable passed as an argument is: the kind its function's reads gave it, else what its assignments make it.
    fn held(&self, var: Var) -> Found {
        let info = self.head.vars.get(&var);
        if info.is_some_and(|info| info.width == Width::Cells) {
            return Found::default();
        }
        match info.map(|info| own_kind_of(&info.kind)) {
            Some(Kind::Unknown) | None => said(self.vars.get(&var).cloned().unwrap_or_default()),
            Some(kind) => said(Found::of(kind)),
        }
    }
}

/// What an argument's reading says of a parameter: nothing where nothing is known, and no kind this may state where it is an integer.
fn said(found: Found) -> Found {
    match found.dead {
        true => Found::default(),
        false => found.unnumbered(),
    }
}

/// Whether a kind is an integer of some width, or a hash.
fn number(kind: &Kind) -> bool {
    kind.is_integer() || *kind == Kind::Hash
}

/// A kind as a kind of one cell: one that takes several cells is unknown.
fn own_kind_of(kind: &Kind) -> Kind {
    match kind {
        Kind::Aggregate(_) | Kind::Vector | Kind::Handle | Kind::Value { .. } => Kind::Unknown,
        kind => kind.clone(),
    }
}

/// What the cells a function of one returned cell hands back are, by its own code.
fn returned(f: &cfg::Function, reader: &Reader<'_>) -> Found {
    let mut found = Found::default();
    // Every block, reached or not: a handler's `return` is reached by no jump.
    for block in &f.cfg.blocks {
        if let Terminator::Return(Some(e)) = &block.term {
            found.join(&reader.expr(e, 0));
        }
    }
    found
}

/// The cells a function of several returns, by its `return`s; where only the functions it hands the result of can say, `open` waits on them.
fn several(f: &cfg::Function, open: &mut Open) -> Option<u32> {
    let mut stated: Option<u32> = None;
    let mut from = BTreeSet::new();
    for block in &f.cfg.blocks {
        let Terminator::Return(Some(e)) = &block.term else {
            continue;
        };
        match (cells_of(e, &f.head), e) {
            (Some(cells), _) => {
                if stated.is_some_and(|held| held != cells) {
                    return None;
                }
                stated = Some(cells);
            }
            (None, Expr::Call { callee, .. }) => from.extend(key_of(f.head.program, callee)),
            (None, _) => {}
        }
    }
    if stated.is_none() && !from.is_empty() {
        open.wide = true;
        open.returned = from.into_iter().collect();
        return Some(0);
    }
    stated
}

/// The kind a function's own code gives one of its parameter cells, where it is a kind of one cell.
fn own_kind(head: &Head, cell: u32) -> Kind {
    match head.vars.get(&Var::Param(cell)) {
        Some(info) if info.width == Width::Cell => match &info.kind {
            Kind::Aggregate(_) | Kind::Vector | Kind::Handle | Kind::Value { .. } => Kind::Unknown,
            kind => kind.clone(),
        },
        _ => Kind::Unknown,
    }
}

/// How a function's code mentions one of its parameter cells.
#[derive(Clone, Copy, Debug, Default)]
struct Cell {
    /// Every mention: a read, an assignment, its address taken.
    mentions: u32,
    /// Mentions as one item among a call's arguments or a group's cells, or as the first of cells read at once.
    listed: u32,
    /// Mentions as a whole argument of a function of a program, at a cell of its parameters that is known.
    passed: u32,
    /// Some such item does not follow the cell before this one.
    split: bool,
    /// Some such item is not followed by the cell after this one.
    cut: bool,
}

impl Cell {
    fn only_passed(&self) -> bool {
        self.passed > 0 && self.passed == self.mentions
    }
}

/// What one reading of a function's code finds of its calls and of how it uses its parameters.
#[derive(Default)]
struct Body {
    /// The functions of a program the function calls.
    calls: BTreeSet<FunctionKey>,
    /// One entry a parameter cell.
    cells: Vec<Cell>,
    /// Runs of parameter cells used as one value: the first cell and the cells, and whether as a handle.
    runs: BTreeMap<(u32, u32), bool>,
    /// Each parameter cell passed whole to a function of a program: the cell, the function, and the cell of its parameters.
    forwards: Vec<(u32, FunctionKey, u32)>,
    /// The lowest local whose address is taken.
    lowest_local: Option<u32>,
    /// The temporaries whose address is taken.
    addressed: HashSet<Var>,
}

impl Body {
    fn of(f: &cfg::Function) -> Body {
        let mut body = Body {
            cells: vec![Cell::default(); f.head.params.min(MAX_PARAMS) as usize],
            ..Body::default()
        };
        for block in &f.cfg.blocks {
            for stmt in &block.stmts {
                for target in stmt.targets() {
                    body.mention(*target);
                }
                stmt.each_expr(&mut |top| body.expr(&f.head, top));
            }
            if let Some(e) = block.term.expr() {
                body.expr(&f.head, e);
            }
        }
        body
    }

    fn mention(&mut self, var: Var) {
        if let Var::Param(n) = var {
            if let Some(cell) = self.cells.get_mut(n as usize) {
                cell.mentions += 1;
            }
        }
    }

    fn expr(&mut self, head: &Head, top: &Expr) {
        top.walk(&mut |e| match e {
            Expr::Var(var) => self.mention(*var),
            Expr::AddrOf { var, .. } => {
                self.mention(*var);
                match *var {
                    Var::Local(n) => {
                        self.lowest_local = Some(self.lowest_local.map_or(n, |low| low.min(n)));
                    }
                    Var::Value { .. } | Var::Stack { .. } => {
                        self.addressed.insert(*var);
                    }
                    Var::Param(_) | Var::Global { .. } => {}
                }
            }
            Expr::LoadBytes {
                addr,
                offset: 0,
                bytes,
            } => {
                if let Expr::AddrOf {
                    var: Var::Param(first),
                    cell: 0,
                } = &**addr
                {
                    self.whole(*first, bytes.div_ceil(8));
                }
            }
            Expr::Cells(items) => {
                self.list(items);
                if let Some(whole) = run(items) {
                    self.runs.entry(whole).or_insert(false);
                }
            }
            Expr::Call { callee, args, .. } => self.call(head, callee, args),
            _ => {}
        });
    }

    /// Notes `cells` parameter cells from `first` on read at once through the address of the first.
    // The address is the one mention, and stands for the cells as a group of them would.
    fn whole(&mut self, first: u32, cells: u32) {
        let last = match cells {
            0 | 1 => None,
            _ => first.checked_add(cells - 1),
        };
        let Some(last) = last else {
            return;
        };
        if let Some(cell) = self.cells.get_mut(first as usize) {
            cell.listed += 1;
            cell.split = true;
        }
        if let Some(cell) = self.cells.get_mut(last as usize) {
            cell.cut = true;
        }
        self.runs.entry((first, cells)).or_insert(false);
    }

    /// Notes each parameter cell that is an item of `items`, and whether its neighbours there are the cells beside it.
    fn list(&mut self, items: &[Expr]) {
        let param = |at: usize| match items.get(at) {
            Some(Expr::Var(Var::Param(n))) => Some(*n),
            _ => None,
        };
        for at in 0..items.len() {
            let Some(n) = param(at) else {
                continue;
            };
            let follows = at > 0 && n > 0 && param(at - 1) == Some(n - 1);
            let followed = n
                .checked_add(1)
                .is_some_and(|next| param(at + 1) == Some(next));
            if let Some(cell) = self.cells.get_mut(n as usize) {
                cell.listed += 1;
                cell.split |= !follows;
                cell.cut |= !followed;
            }
        }
    }

    fn call(&mut self, head: &Head, callee: &Callee, args: &[Expr]) {
        self.list(args);
        let handles =
            matches!(callee, Callee::Builtin { path, .. } if HANDLE_TESTS.contains(&path.as_str()));
        if handles && args.len() == 4 {
            for pair in args.chunks(2) {
                if let Some(handle) = run(pair) {
                    self.runs.insert(handle, true);
                }
            }
        }
        let Some(key) = key_of(head.program, callee) else {
            return;
        };
        self.calls.insert(key);
        // The cell of the callee's parameters each argument starts at, while the arguments before it say their cells.
        let mut at = Some(0u32);
        for arg in args {
            let Some(cell) = at else {
                break;
            };
            if let Expr::Var(Var::Param(n)) = arg {
                if let Some(own) = self.cells.get_mut(*n as usize) {
                    own.passed += 1;
                    self.forwards.push((*n, key, cell));
                }
            }
            at = cells_of(arg, head).and_then(|cells| cell.checked_add(cells));
        }
    }

    /// The runs of parameter cells that are one value wherever the function mentions them: by first cell, the cells and whether a handle.
    // A run stands only where no other run takes some of its cells and not all.
    fn groups(&self) -> BTreeMap<u32, (u32, bool)> {
        let mut groups = BTreeMap::new();
        // One past the last cell of the runs before this one.
        let mut reach = 0u64;
        let mut runs = self.runs.iter().peekable();
        while let Some((&(first, cells), &handle)) = runs.next() {
            let end = u64::from(first) + u64::from(cells);
            let next_starts = runs.peek().map(|next| u64::from(next.0 .0));
            let apart = reach <= u64::from(first) && next_starts.is_none_or(|next| next >= end);
            reach = reach.max(end);
            if apart && self.whole_run(first, cells) {
                groups.insert(first, (cells, handle));
            }
        }
        groups
    }

    /// Whether every mention of each cell of a run is of the run whole and in order.
    fn whole_run(&self, first: u32, cells: u32) -> bool {
        let range = first
            .checked_add(cells)
            .map(|end| first as usize..end as usize);
        let Some(run) = range.and_then(|range| self.cells.get(range)) else {
            return false;
        };
        run.iter().enumerate().all(|(at, cell)| {
            let inside = (at == 0 || !cell.split) && (at + 1 == run.len() || !cell.cut);
            cell.mentions == cell.listed && inside
        })
    }
}

/// The parameter cells a list of expressions is made of, where it is two or more of them in a row and nothing else: the first and how many.
fn run(items: &[Expr]) -> Option<(u32, u32)> {
    let Some(Expr::Var(Var::Param(first))) = items.first() else {
        return None;
    };
    let cells = u32::try_from(items.len())
        .ok()
        .filter(|cells| *cells >= 2)?;
    let in_order = (0..cells).zip(items).all(
        |(at, item)| matches!(item, Expr::Var(Var::Param(n)) if first.checked_add(at) == Some(*n)),
    );
    in_order.then_some((*first, cells))
}

/// What a function that waits on nothing returns, by its signature; one the project does not hold returns what nothing knows.
fn stated(project: &Project, function: &FunctionKey) -> Found {
    match project.signatures.get(function) {
        Some(held) if held.returns == 1 && held.result != Kind::Unknown => {
            Found::of(held.result.clone())
        }
        _ => Found::dead(),
    }
}

/// Settles the result of each function that waited: its kind where all it hands on agree, its cells where they say.
// A result that is constants alone, its own and those of the functions it hands on, is of no
// kind by that: it takes the kind its uses read, then the kind its line of a slot agrees on.
fn results(project: &mut Project, open: &BTreeMap<FunctionKey, Open>, shared: &Shared) {
    let mut kinds: BTreeMap<FunctionKey, Found> = BTreeMap::new();
    let mut cells: BTreeMap<FunctionKey, u32> = BTreeMap::new();
    for (key, waits) in open {
        let Some(held) = project.signatures.get(key).filter(|_| waits.live) else {
            continue;
        };
        let own = Found {
            kind: (held.result != Kind::Unknown).then(|| held.result.clone()),
            lo: waits.lo,
            hi: waits.hi,
            odd: waits.odd,
            ..Found::default()
        };
        kinds.insert(*key, own);
    }
    let mut moving = true;
    for _ in 0..ROUNDS {
        moving = false;
        for (key, waits) in open {
            for callee in &waits.returned {
                if waits.live {
                    let theirs = match kinds.get(callee) {
                        Some(found) => found.clone(),
                        None => stated(project, callee),
                    };
                    if let Some(mine) = kinds.get_mut(key) {
                        moving |= mine.join(&theirs);
                    }
                }
                if waits.wide {
                    let theirs = match open.get(callee) {
                        Some(other) if other.wide => cells.get(callee).copied(),
                        _ => project.signatures.get(callee).map(|held| held.returns),
                    };
                    let mine = cells.get(key).copied();
                    let joined = match (mine, theirs) {
                        (_, None) => mine,
                        (None, _) => theirs,
                        (Some(a), Some(b)) if a == b => mine,
                        _ => Some(CLASH),
                    };
                    if let (true, Some(count)) = (joined != mine, joined) {
                        cells.insert(*key, count);
                        moving = true;
                    }
                }
            }
        }
        if !moving {
            break;
        }
    }
    for (key, waits) in open {
        // A function still changing when the rounds ran out has not met all it waits on.
        let unsettled = moving && !waits.returned.is_empty();
        if waits.wide {
            let count = cells.get(key).copied();
            let count = count.filter(|count| *count != CLASH && *count > 0 && !unsettled);
            match count {
                Some(count) => {
                    if let Some(held) = project.signatures.get_mut(key) {
                        held.returns = count;
                    }
                }
                // A count nothing states is no signature: a call of the function keeps saying none.
                None => {
                    project.signatures.remove(key);
                }
            }
        }
        if waits.live {
            let found = kinds.get(key).filter(|found| !found.dead && !unsettled);
            let kind = found.and_then(|found| {
                let kind = found.kind.clone().or_else(|| shared.read_as(key, found));
                kind.filter(|kind| holds(kind, found))
            });
            if let Some(held) = project.signatures.get_mut(key) {
                held.result = kind.unwrap_or(Kind::Unknown);
            }
        }
    }
    shared::agreed(project, open, &kinds, shared);
}

/// Gives each parameter cell a function only passes on the one kind the functions given it read it as.
fn parameters(project: &mut Project, open: &BTreeMap<FunctionKey, Open>) {
    let mut kinds: BTreeMap<(FunctionKey, u32), Found> = BTreeMap::new();
    let mut moving = true;
    for _ in 0..ROUNDS {
        moving = false;
        for (key, waits) in open {
            for (cell, callee, at) in &waits.forwards {
                let theirs = match kinds.get(&(*callee, *at)) {
                    Some(found) => found.clone(),
                    None => {
                        let held = project.signatures.get(callee);
                        let param = held.and_then(|held| held.params.get(param_at(held, *at)?));
                        match param {
                            Some(param) if param.cells == 1 && param.kind != Kind::Unknown => {
                                Found::of(param.kind.clone())
                            }
                            // A cell nothing types says nothing.
                            _ => continue,
                        }
                    }
                };
                moving |= kinds.entry((*key, *cell)).or_default().join(&theirs);
            }
        }
        if !moving {
            break;
        }
    }
    if moving {
        return;
    }
    for ((key, cell), found) in kinds {
        let (false, Some(kind)) = (found.dead, found.kind) else {
            continue;
        };
        let Some(held) = project.signatures.get_mut(&key) else {
            continue;
        };
        let param = param_at(held, cell).and_then(|n| held.params.get_mut(n));
        if let Some(param) = param.filter(|param| param.cells == 1 && param.kind == Kind::Unknown) {
            param.kind = kind;
        }
    }
}

#[cfg(test)]
#[path = "signatures_tests.rs"]
pub(crate) mod tests;
