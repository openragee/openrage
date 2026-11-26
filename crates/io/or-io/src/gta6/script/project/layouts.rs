//! Field layouts: the fields of each type, from a program's member tables where it has them and
//! from every load and store through a base of known type.
//!
//! A base is of known type where it only ever holds an object the function itself makes of one
//! type, or is the receiver of a function one type's virtual slots are the first to hold. A
//! type's rows are evidence, one for each width and kind seen at an offset, until [`settle`]
//! makes one field of the rows that agree, under the base type that holds it. A field's kind is
//! the one every access of it in every program agrees on, and none where two disagree. One byte
//! whose every store is a truth and whose every load is only tested for zero is a `bool`.

use std::collections::{BTreeMap, BTreeSet};

use super::{FieldFacts, Project, TypeFacts, MAX_BASES};
use crate::gta6::script::cfg;
use crate::gta6::script::ir::{
    BinOp, Callee, Expr, FunctionKey, Head, Kind, Scalar, Stmt, StoreKind, TypeKey, UnOp, Var,
};
use crate::gta6::script::program::{Program, TypeRow};

/// The rows of each type's fields and the truths of its bytes, before they are settled.
pub type Rows = (
    BTreeMap<TypeKey, Vec<FieldFacts>>,
    BTreeMap<(TypeKey, u32), bool>,
);

/// The bytes of an object's header: its fields start after them.
const HEADER: u32 = 8;
/// The flags of a type whose values are held in place and never made as objects.
const VALUE: u32 = 0x1C;
/// The `byte_offset`s under one access that are added into its offset.
const NESTED: usize = 8;
/// The types the objects of one field are called as, past which the field holds a reference and no more is said.
const RECEIVERS: usize = 32;

/// The key the receiver's type of the function at `at` waits under, from [`of_program`] until [`of_function`] reads that function.
// ⚠ No type has such a key: a script type's is a 64-bit hash and a builtin's has bit 31 set.
// `of_function` takes the row of the function it reads; `absorb` and `settle` drop any other.
fn waiting(at: u32) -> Option<TypeKey> {
    (at >> 31 == 0).then_some(TypeKey(u64::from(at)))
}

/// Whether `key` is one [`waiting`] makes, and so no type's.
fn waits(key: TypeKey) -> bool {
    key.0 >> 31 == 0
}

/// Adds one row to a type's fields, which are kept by rising offset, unless the type has the row.
fn note(project: &mut Project, key: TypeKey, row: FieldFacts) {
    let rows = project.fields.entry(key).or_default();
    let at = rows.partition_point(|held| held.offset < row.offset);
    let mut here = rows[at..]
        .iter()
        .take_while(|held| held.offset == row.offset);
    let known = here.any(|held| *held == row);
    if !known {
        rows.insert(at, row);
    }
}

/// What a member table's field of the type `of` holds in `bytes` bytes: the value in place, or a reference to an object.
fn declared(of: &TypeRow, bytes: u32) -> Kind {
    let ty = TypeKey(of.name).to_string();
    match (of.flags & VALUE != 0, bytes) {
        (true, _) => Kind::Value {
            ty,
            cells: bytes.div_ceil(8),
        },
        (false, 8) => Kind::Object(ty),
        (false, _) => Kind::Unknown,
    }
}

/// Whether a base type of `row`, among the program's own, holds the function at `at` in a slot: the function is then the base's.
// ponytail: each base type's slots are searched in a row; index them by function if a program's tables make this slow.
fn inherited(program: &Program, row: &TypeRow, at: u32) -> bool {
    let mut base = row.base;
    for _ in 0..MAX_BASES {
        let Some(above) = base.and_then(|id| program.types.get(id as usize)) else {
            return false;
        };
        if above.slots.contains(&Some(at)) {
            return true;
        }
        base = above.base;
    }
    false
}

/// Adds the fields a compiled program's member tables state, its global cells that hold a reference, and the receiver's type of each function one type's virtual slots are the first to hold.
// A type is keyed by its name's hash, as the types a function's code names are. A field's own
// type is kept only where it is one of the program's: what another program's type is, a value
// or an object, this program does not say.
pub fn of_program(project: &mut Project, program: &Program) {
    // The container's map has a bit a cell, the program's own cells first; a global is named by its byte offset.
    let mapped = program.global_references.len().saturating_mul(8);
    let cells = (program.globals.cells).min(u32::try_from(mapped).unwrap_or(u32::MAX));
    if let Some(info) = project.programs.get_mut(&program.program) {
        for cell in (0..cells).filter(|cell| program.global_is_reference(*cell)) {
            info.globals.insert(cell.saturating_mul(8), Kind::Pointer);
        }
    }

    // A name's hash has an upper half: a row without one is no type the registry keeps.
    let named = |row: &&TypeRow| row.name >> 32 != 0;
    for row in program.types.iter().filter(named) {
        for field in row.fields.iter().filter(|field| field.bytes > 0) {
            let of = field.ty.and_then(|id| program.types.get(id as usize));
            let of = of.filter(named);
            let facts = FieldFacts {
                offset: field.offset,
                bytes: field.bytes,
                kind: of.map_or(Kind::Unknown, |of| declared(of, field.bytes)),
            };
            note(project, TypeKey(row.name), facts);
        }
    }

    // The one type that holds each function in a slot none of its base types holds it in; `None` once two types do.
    let mut first: BTreeMap<u32, Option<usize>> = BTreeMap::new();
    for (id, row) in program.types.iter().enumerate() {
        for at in row.slots.iter().flatten() {
            if !inherited(program, row, *at) {
                let held = first.entry(*at).or_insert(Some(id));
                if *held != Some(id) {
                    *held = None;
                }
            }
        }
    }
    for (at, held) in first {
        let row = held.and_then(|id| program.types.get(id)).filter(named);
        let (Some(row), Some(key)) = (row, waiting(at)) else {
            continue;
        };
        // ⚠ A value type's method takes a pointer to the value, whose fields do not lie where an object's do.
        if row.flags & VALUE == 0 && program.function(at).is_some() {
            let receiver = FieldFacts {
                offset: 0,
                bytes: 0,
                kind: Kind::Object(TypeKey(row.name).to_string()),
            };
            project.fields.insert(key, vec![receiver]);
        }
    }
}

/// How a function sets its variables and whose address it takes.
#[derive(Default)]
struct Sets {
    /// Each variable assigned by name: the one type of the objects made into it, `None` where it is given anything else.
    made: BTreeMap<Var, Option<TypeKey>>,
    addressed: BTreeSet<Var>,
}

impl Sets {
    fn of(f: &cfg::Function) -> Sets {
        let mut sets = Sets::default();
        for block in &f.cfg.blocks {
            for stmt in &block.stmts {
                match stmt {
                    Stmt::Assign {
                        target,
                        value: Expr::NewObject(ty),
                    } => sets.give(*target, TypeKey::of(ty).filter(|key| !waits(*key))),
                    other => {
                        for target in other.targets() {
                            sets.give(*target, None);
                        }
                    }
                }
                stmt.each_expr(&mut |top| sets.addresses(top));
            }
            if let Some(e) = block.term.expr() {
                sets.addresses(e);
            }
        }
        sets
    }

    fn give(&mut self, var: Var, ty: Option<TypeKey>) {
        let held = self.made.entry(var).or_insert(ty);
        if *held != ty {
            *held = None;
        }
    }

    fn addresses(&mut self, top: &Expr) {
        top.walk(&mut |e| {
            if let Expr::AddrOf { var, .. } = e {
                self.addressed.insert(*var);
            }
        });
    }

    /// The lowest local whose address the function takes.
    fn lowest(&self) -> Option<u32> {
        let locals = self.addressed.iter().filter_map(|var| match var {
            Var::Local(n) => Some(*n),
            _ => None,
        });
        locals.min()
    }

    /// The variables that hold an object of one known type wherever they are read.
    // ⚠ A variable whose address is taken, and a local at or after one whose address is, may be
    // written through that address: what is assigned to it by name then proves nothing.
    fn bases(&self, receiver: Option<TypeKey>) -> BTreeMap<Var, TypeKey> {
        let lowest = self.lowest();
        let mut bases = BTreeMap::new();
        for (var, ty) in &self.made {
            let reached = match var {
                Var::Local(n) => lowest.is_some_and(|low| *n >= low),
                Var::Value { .. } | Var::Stack { .. } => self.addressed.contains(var),
                // A caller sets a parameter, and any code a global.
                Var::Param(_) | Var::Global { .. } => true,
            };
            if let (false, Some(ty)) = (reached, ty) {
                bases.insert(*var, *ty);
            }
        }
        // The receiver is its type's object only while the function leaves it as it was passed.
        let receiver = receiver.filter(|_| {
            let this = Var::Param(0);
            !self.made.contains_key(&this) && !self.addressed.contains(&this)
        });
        if let Some(owner) = receiver {
            bases.insert(Var::Param(0), owner);
        }
        bases
    }
}

/// The kind several bytes read or written whole are: a value held in place where they are more than a cell.
fn whole(bytes: u32) -> Kind {
    match bytes > 8 {
        true => Kind::Aggregate(bytes.div_ceil(8)),
        false => Kind::Unknown,
    }
}

/// Whether a call goes through its first argument as an object: a virtual call's receiver, an array, a delegate.
fn through_first(callee: &Callee) -> bool {
    match callee {
        Callee::Virtual { .. } => true,
        Callee::Intrinsic { name, .. } => {
            let stem = name.split('<').next().unwrap_or(name);
            stem.starts_with("array_") || stem == "invoke_delegate"
        }
        _ => false,
    }
}

/// Whether `e` is the constant zero.
fn is_zero(e: &Expr) -> bool {
    matches!(e, Expr::Const(c) if c.bits == 0)
}

/// One reading of a function for the fields its accesses show.
struct Seen<'a> {
    bases: &'a BTreeMap<Var, TypeKey>,
    head: &'a Head,
    rows: Vec<(TypeKey, FieldFacts)>,
    /// Each byte accessed through a base, and whether every such access was of a truth.
    truths: Vec<((TypeKey, u32), bool)>,
    /// The loads that stand where only whether they are zero is read.
    tested: BTreeSet<*const Expr>,
}

impl Seen<'_> {
    /// The type and the offset in it that an access at `offset` from `addr` reaches, where `addr` is a base of known type moved by constants.
    fn place(&self, addr: &Expr, offset: u32) -> Option<(TypeKey, u32)> {
        let (mut addr, mut offset) = (addr, offset);
        for _ in 0..NESTED {
            let Expr::ByteOffset {
                addr: inner,
                offset: by,
            } = addr
            else {
                break;
            };
            let Expr::Const(c) = &**by else {
                return None;
            };
            let by = u32::try_from(c.bits).ok()?;
            offset = offset.checked_add(by)?;
            addr = &**inner;
        }
        match addr {
            Expr::Var(var) => self.bases.get(var).map(|key| (*key, offset)),
            _ => None,
        }
    }

    /// Notes whether an access of the one byte at `offset` from `addr` is of a truth, where that is a field of a base of known type.
    fn truth(&mut self, addr: &Expr, offset: u32, truth: bool) {
        if let Some((key, offset)) = self.place(addr, offset) {
            if offset >= HEADER {
                self.truths.push(((key, offset), truth));
            }
        }
    }

    /// Whether `value`, stored in one byte, is a truth by its own form: zero or one, a comparison, a test, a truth held in a variable.
    fn is_truth(&self, value: &Expr) -> bool {
        let mut e = value;
        while let Expr::Cast { e: inner, .. } = e {
            e = &**inner;
        }
        match e {
            Expr::Const(c) => c.bits <= 1,
            Expr::Truth(_) | Expr::Unary { op: UnOp::Not, .. } => true,
            Expr::Binary { op, .. } => op.is_comparison() || op.is_logical(),
            Expr::Var(var) => (self.head.vars.get(var)).is_some_and(|info| info.kind == Kind::Bool),
            _ => false,
        }
    }

    /// Notes each load under `top` that stands where only whether it is zero is read: tested, negated, compared with zero, joined by `&&` or `||`.
    fn tests(&mut self, top: &Expr) {
        let mut tested = |e: &Expr| {
            let mut e = e;
            while let Expr::Cast { e: inner, .. } = e {
                e = &**inner;
            }
            if let Expr::Load { .. } = e {
                self.tested.insert(std::ptr::from_ref(e));
            }
        };
        top.walk(&mut |e| match e {
            Expr::Truth(inner)
            | Expr::Unary {
                op: UnOp::Not,
                e: inner,
            } => tested(inner),
            Expr::Binary {
                op: BinOp::Eq | BinOp::Ne,
                l,
                r,
                ..
            } => {
                if is_zero(r) {
                    tested(l);
                }
                if is_zero(l) {
                    tested(r);
                }
            }
            Expr::Binary { op, l, r, .. } if op.is_logical() => {
                tested(l);
                tested(r);
            }
            _ => {}
        });
    }

    /// Notes `bytes` bytes of kind `kind` at `offset` from `addr`, where that is a field of a base of known type.
    fn row(&mut self, addr: &Expr, offset: u32, bytes: u32, kind: Kind) {
        if let Some((key, offset)) = self.place(addr, offset) {
            if offset >= HEADER && bytes > 0 {
                let facts = FieldFacts {
                    offset,
                    bytes,
                    kind,
                };
                self.rows.push((key, facts));
            }
        }
    }

    /// Notes an access of `bytes` bytes at `offset` from `addr`, of what `kind` says the bytes are.
    fn field(&mut self, addr: &Expr, offset: u32, bytes: u32, kind: Kind) {
        // What is loaded from a field and gone through at an object's own offsets is a reference.
        if offset >= HEADER {
            self.reference(addr);
        }
        self.row(addr, offset, bytes, kind);
    }

    /// Notes that the field `e` loads whole, where it loads one, holds a reference.
    fn reference(&mut self, e: &Expr) {
        if let Expr::Load {
            ty: Scalar::U64,
            addr,
            offset,
        } = e
        {
            self.row(addr, *offset, 8, Kind::Pointer);
        }
    }

    /// Notes that the field `e` loads whole, where it loads one, holds an object of the type named `ty`, whose slot a call takes on it.
    // ⚠ The type a call goes through is the one the source gives its receiver or a type above
    // that one, never the type of the object itself: [`settle`] keeps the most derived of those
    // seen. A builtin's slot says nothing a layout can use.
    fn receiver(&mut self, e: &Expr, ty: &str) {
        let script = TypeKey::of_name(ty).filter(|key| key.builtin().is_none() && !waits(*key));
        if let (
            Some(key),
            Expr::Load {
                ty: Scalar::U64,
                addr,
                offset,
            },
        ) = (script, e)
        {
            self.row(addr, *offset, 8, Kind::Object(key.to_string()));
        }
    }

    /// Notes the sign an operator that works at `at` reads a field's four bytes under, where `read` loads them under that sign; `test` is a comparison with zero for being equal.
    // ⚠ Only a division, a remainder, a right shift and an ordering have an instruction for each
    // sign. Any other operator is listed at one sign whatever its operands hold: arithmetic and
    // equality signed, a bit operator unsigned. So a field's sign is the one every operator on it
    // is listed at, and a field worked on at both has none. A test of 32 bits against zero is
    // listed unsigned whatever they are, and says nothing.
    fn operand(&mut self, at: &Kind, read: &Expr, test: bool) {
        let Expr::Load { ty, addr, offset } = read else {
            return;
        };
        let sign = match (ty, at) {
            (Scalar::I32, Kind::I32) => Kind::I32,
            (Scalar::U32, Kind::U32) if !test => Kind::U32,
            _ => return,
        };
        self.row(addr, *offset, 4, sign);
    }

    /// Whether a value stored in four bytes is a float by its own form.
    fn is_float(&self, e: &Expr) -> bool {
        match e {
            Expr::Const(c) => c.kind == Kind::F32,
            Expr::Cast { to, .. } | Expr::Load { ty: to, .. } => *to == Scalar::F32,
            Expr::Binary { at, .. } => *at == Kind::F32,
            Expr::Var(var) => (self.head.vars.get(var)).is_some_and(|info| info.kind == Kind::F32),
            _ => false,
        }
    }

    fn stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Store {
                kind,
                addr,
                offset,
                value,
            } => {
                let zero = matches!(value, Expr::Const(c) if c.bits == 0);
                if *kind == StoreKind::U8 {
                    self.truth(addr, *offset, self.is_truth(value));
                }
                let held = match kind {
                    StoreKind::Ref => Kind::Pointer,
                    // Eight zero bytes clear a reference as well as a number.
                    StoreKind::U64 if !zero => Kind::U64,
                    StoreKind::I32 if self.is_float(value) => Kind::F32,
                    _ => Kind::Unknown,
                };
                self.field(addr, *offset, kind.bytes(), held);
            }
            Stmt::StoreBytes {
                addr,
                offset,
                bytes,
                ..
            } => {
                if *bytes == 1 {
                    self.truth(addr, *offset, false);
                }
                self.field(addr, *offset, *bytes, whole(*bytes));
            }
            Stmt::ZeroBytes { addr, bytes } => self.field(addr, 0, *bytes, whole(*bytes)),
            // One up or down is arithmetic, which is listed signed.
            Stmt::StepI32 { addr, offset, .. } => self.field(addr, *offset, 4, Kind::I32),
            // Floats one after another, as many as the value has cells: no width is stated.
            Stmt::StoreF32Slots { .. }
            | Stmt::Assign { .. }
            | Stmt::AssignMany { .. }
            | Stmt::Eval(_) => {}
        }
        stmt.each_expr(&mut |top| self.expr(top));
    }

    fn expr(&mut self, top: &Expr) {
        self.tests(top);
        top.walk(&mut |e| match e {
            Expr::Load { ty, addr, offset } => {
                if ty.bytes() == 1 {
                    let tested = self.tested.contains(&std::ptr::from_ref(e));
                    self.truth(addr, *offset, tested);
                }
                let held = match ty {
                    // ⚠ Four bytes and eight are loaded as they lie, and a listing hands any
                    // 32 bits on as `as_u32`: neither load says a sign.
                    Scalar::I32 | Scalar::U32 | Scalar::I64 | Scalar::U64 => Kind::Unknown,
                    stated => stated.kind(),
                };
                self.field(addr, *offset, ty.bytes(), held);
            }
            Expr::LoadBytes {
                addr,
                offset,
                bytes,
            } => self.field(addr, *offset, *bytes, whole(*bytes)),
            Expr::Binary { op, at, l, r } => {
                let equality = matches!(op, BinOp::Eq | BinOp::Ne);
                self.operand(at, l, equality && is_zero(r));
                // What a shift moves by is a count, read as its own instruction reads it.
                if !matches!(op, BinOp::Shl | BinOp::Shr) {
                    self.operand(at, r, equality && is_zero(l));
                }
            }
            Expr::Call { callee, args, .. } if through_first(callee) => {
                if let Some(first) = args.first() {
                    self.reference(first);
                    if let Callee::Virtual { ty, .. } = callee {
                        self.receiver(first, ty);
                    }
                }
            }
            _ => {}
        });
    }
}

/// Adds the fields one function's loads and stores show, each under the type of the base it goes through.
pub fn of_function(project: &mut Project, f: &cfg::Function) {
    let waited = waiting(f.head.at).and_then(|key| project.fields.remove(&key));
    let receiver = match waited.as_deref() {
        Some(
            [FieldFacts {
                kind: Kind::Object(name),
                ..
            }],
        ) if f.head.params > 0 => TypeKey::of_name(name),
        _ => None,
    };
    let bases = Sets::of(f).bases(receiver);
    seen_through(project, f, &bases);
}

/// Adds the fields one function's loads and stores show through what a settled project proves of it: the variables the kinds pass types as an object of a script type, and the receiver of a member.
// ⚠ Read after every program is in, with the kinds the project gives: a part for [`refine`].
pub fn of_typed_function(part: &mut Project, f: &cfg::Function, project: &Project) {
    let own = FunctionKey::new(f.head.program, f.head.at);
    let object = |key: TypeKey| project.types.get(&key).is_some_and(|ty| !ty.is_value());
    let receiver = (project.members.get(&own))
        .filter(|member| !member.is_static && f.head.params > 0)
        .map(|member| member.owner)
        .filter(|owner| object(*owner));
    let sets = Sets::of(f);
    let mut bases = sets.bases(receiver);
    let lowest = sets.lowest();
    for (var, info) in &f.head.vars {
        let Kind::Object(name) = &info.kind else {
            continue;
        };
        let typed = TypeKey::of_name(name).filter(|key| key.builtin().is_none() && object(*key));
        let reached = match var {
            Var::Local(n) => lowest.is_some_and(|low| *n >= low),
            Var::Value { .. } | Var::Stack { .. } | Var::Param(_) => sets.addressed.contains(var),
            Var::Global { .. } => true,
        };
        if let (Some(key), false) = (typed, reached) {
            bases.entry(*var).or_insert(key);
        }
    }
    seen_through(part, f, &bases);
}

/// Adds the rows and the truths the accesses of `f` through `bases` show.
fn seen_through(project: &mut Project, f: &cfg::Function, bases: &BTreeMap<Var, TypeKey>) {
    if bases.is_empty() {
        return;
    }
    let mut seen = Seen {
        bases,
        head: &f.head,
        rows: Vec::new(),
        truths: Vec::new(),
        tested: BTreeSet::new(),
    };
    for block in &f.cfg.blocks {
        for stmt in &block.stmts {
            seen.stmt(stmt);
        }
        if let Some(e) = block.term.expr() {
            seen.expr(e);
        }
        // A switch reads what it is given as an `int`.
        if let cfg::Terminator::Switch { value, .. } = &block.term {
            seen.operand(&Kind::I32, value, false);
        }
    }
    for (key, row) in seen.rows {
        note(project, key, row);
    }
    for (place, truth) in seen.truths {
        *project.truths.entry(place).or_insert(true) &= truth;
    }
}

/// Moves the fields of `from` into `into`: a type both hold has the rows of both, each once.
pub fn absorb(into: &mut Project, from: &mut Project) {
    for (key, rows) in std::mem::take(&mut from.fields) {
        if waits(key) {
            continue;
        }
        for row in rows {
            note(into, key, row);
        }
    }
    for (place, truth) in std::mem::take(&mut from.truths) {
        *into.truths.entry(place).or_insert(true) &= truth;
    }
}

/// The type that declares the field a row was seen as on `key`: the furthest base type the field still lies in; `None` for a row `key` itself does not hold.
// A type's fields lie after its base type's, so what ends within the base is the base's.
pub(crate) fn declarer(
    types: &BTreeMap<TypeKey, TypeFacts>,
    key: TypeKey,
    row: &FieldFacts,
) -> Option<TypeKey> {
    let end = u64::from(row.offset) + u64::from(row.bytes);
    let holds = |ty: &TypeFacts| end <= u64::from(HEADER) + u64::from(ty.byte_size);
    let mut owner = key;
    let mut facts = types.get(&key);
    if facts.is_some_and(|ty| !holds(ty)) {
        return None;
    }
    for _ in 0..MAX_BASES {
        let Some(base) = facts.and_then(|ty| ty.base) else {
            break;
        };
        let above = types.get(&base);
        if !above.is_some_and(holds) {
            break;
        }
        (owner, facts) = (base, above);
    }
    Some(owner)
}

/// What two kinds seen of one field say together; `None` where they disagree.
fn both(a: Kind, b: &Kind) -> Option<Kind> {
    let named = |kind: &Kind| matches!(kind, Kind::Object(_) | Kind::String | Kind::Array(_));
    match (a, b) {
        (a, b) if a == *b => Some(a),
        (Kind::Unknown, b) => Some(b.clone()),
        (a, Kind::Unknown) => Some(a),
        // A reference, and what one store says it refers to.
        (Kind::Pointer, b) if named(b) => Some(b.clone()),
        (a, Kind::Pointer) if named(&a) => Some(a),
        // Cells held in place, and the type a member table gives them.
        (Kind::Aggregate(n), Kind::Value { cells, .. }) if n == *cells => Some(b.clone()),
        (Kind::Value { ty, cells }, Kind::Aggregate(n)) if cells == *n => {
            Some(Kind::Value { ty, cells })
        }
        _ => None,
    }
}

/// The most derived of the types `called`: the one that each of the others is a base type or an interface of; `None` where no one of them is all of them.
fn narrowest(project: &Project, called: &BTreeSet<TypeKey>) -> Option<TypeKey> {
    if called.len() > RECEIVERS {
        return None;
    }
    let all = |ty: &TypeKey| called.iter().all(|other| project.is_a(*ty, *other));
    called.iter().copied().find(all)
}

/// The one field the rows seen at an offset make: none where two widths were seen, and of no kind where two kinds were.
// A reference whose objects are of script types is of the most derived of them, and where they
// are of no one line it is a reference and no more.
fn agreed(project: &Project, rows: &[FieldFacts]) -> Option<FieldFacts> {
    let first = rows.first()?;
    if rows.iter().any(|row| row.bytes != first.bytes) {
        return None;
    }
    let mut called: BTreeSet<TypeKey> = BTreeSet::new();
    let mut kind = Some(Kind::Unknown);
    for row in rows {
        let script = match &row.kind {
            Kind::Object(name) => TypeKey::of_name(name),
            _ => None,
        };
        if let Some(key) = script {
            called.insert(key);
        }
        kind = kind.and_then(|known| match script {
            Some(_) => both(known, &Kind::Pointer),
            None => both(known, &row.kind),
        });
    }
    let kind = match (kind, called.is_empty()) {
        (Some(kind), true) => kind,
        (Some(Kind::Pointer), false) => match narrowest(project, &called) {
            Some(key) => Kind::Object(key.to_string()),
            None => Kind::Pointer,
        },
        // A string or an array by one row and an object of a script type by another.
        (Some(_), false) => Kind::Pointer,
        (None, _) => Kind::Unknown,
    };
    Some(FieldFacts {
        offset: first.offset,
        bytes: first.bytes,
        kind,
    })
}

/// Once every part is in: one field an offset, by rising offset, with the kind every access agrees on, under the type that declares it.
// ⚠ An offset seen at two widths has no field: each access of it keeps the form it has.
pub fn settle(project: &mut Project) {
    // Kept, so that [`refine`] settles them again with more.
    project.rows = (project.fields.clone(), project.truths.clone());
    let mut seen: BTreeMap<TypeKey, BTreeMap<u32, Vec<FieldFacts>>> = BTreeMap::new();
    for (key, rows) in std::mem::take(&mut project.fields) {
        if waits(key) {
            continue;
        }
        for row in rows {
            if let Some(owner) = declarer(&project.types, key, &row) {
                let at = seen.entry(owner).or_default().entry(row.offset);
                let held = at.or_default();
                if !held.contains(&row) {
                    held.push(row);
                }
            }
        }
    }
    // A byte every access of which, in every program, is of a truth.
    let mut truths: BTreeMap<(TypeKey, u32), bool> = BTreeMap::new();
    for ((key, offset), truth) in std::mem::take(&mut project.truths) {
        let row = FieldFacts {
            offset,
            bytes: 1,
            kind: Kind::Unknown,
        };
        if let Some(owner) = declarer(&project.types, key, &row) {
            *truths.entry((owner, offset)).or_insert(true) &= truth;
        }
    }
    let known: &Project = project;
    let mut settled = BTreeMap::new();
    for (owner, offsets) in seen {
        let mut fields: Vec<FieldFacts> = offsets
            .into_values()
            .filter_map(|rows| agreed(known, &rows))
            .collect();
        // ⚠ Not a signed byte: its loads widen it by its sign, which a truth has no use for.
        for field in &mut fields {
            let unsigned = matches!(field.kind, Kind::U8 | Kind::Unknown);
            let truth = truths.get(&(owner, field.offset)) == Some(&true);
            if field.bytes == 1 && unsigned && truth {
                field.kind = Kind::Bool;
            }
        }
        if !fields.is_empty() {
            settled.insert(owner, fields);
        }
    }
    project.fields = settled;
}

/// Adds the rows and the truths of `parts`, each read by [`of_typed_function`], to those the fields of `project` were settled from, and settles them again.
pub fn refine(project: &mut Project, parts: Vec<Project>) {
    let (rows, truths) = std::mem::take(&mut project.rows);
    project.fields = rows;
    project.truths = truths;
    for mut part in parts {
        absorb(project, &mut part);
    }
    settle(project);
}

#[cfg(test)]
#[path = "layouts_tests.rs"]
mod tests;
