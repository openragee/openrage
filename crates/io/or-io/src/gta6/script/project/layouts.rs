//! Field layouts: the fields of each type, from a program's member tables where it has them and
//! from every load and store through a base of known type.
//!
//! A base is of known type where it only ever holds an object the function itself makes of one
//! type, or is the receiver of a function one type's virtual slots are the first to hold; once
//! every program is in, also a variable the kinds pass types and a member's receiver, which for a
//! value type points at the value's first byte. A
//! type's rows are evidence, one for each width and kind seen at an offset, until [`settle`]
//! makes one field of the rows that agree, under the base type that holds it. A field's kind is
//! the one every access of it in every program agrees on, and none where two disagree. One byte
//! whose every store is a truth and whose every load is only tested for zero or kept in a `bool`
//! is a `bool`. A reference field is of the most derived type every call on it is made as, where
//! every object a store seen puts in it derives from that type: a store through a base of no
//! known type is never seen, so a store only rules a type out. Read again with a settled project,
//! an access through a field the project types as an object is that type's, for as long as the
//! field, settled again with it, still refers to that type.

use std::collections::{BTreeMap, BTreeSet};

use super::{reach, signatures, FieldFacts, Project, TypeFacts, MAX_BASES};
use crate::gta6::script::cfg;
use crate::gta6::script::ir::{
    BinOp, Callee, Expr, FunctionKey, Head, Kind, Scalar, Stmt, StoreKind, TypeKey, UnOp, Var,
};
use crate::gta6::script::print::cells_of;
use crate::gta6::script::program::{Program, TypeRow};

/// The rows of each type's fields, the truths of its bytes and the objects stored in its references, before they are settled.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rows {
    pub fields: BTreeMap<TypeKey, Vec<FieldFacts>>,
    pub truths: BTreeMap<(TypeKey, u32), bool>,
    /// The script types of the objects stored at each offset, by the type the store went through.
    // ⚠ Kept apart from the rows: a type a call is made as lies above the field's type, and the
    // type of an object stored lies below it.
    pub made: BTreeMap<(TypeKey, u32), BTreeSet<TypeKey>>,
    /// The member-table fields of a type another program defines, until every program is in.
    pub stated: Vec<Stated>,
    /// What the accesses read through a field the settled project types showed, kept apart from the rows.
    // ⚠ Each holds only while every field it went through refers to the type it did: [`refine`]
    // settles them in and checks that again.
    pub chained: Vec<Chained>,
}

/// A member table's field whose type is another program's, by the id the stating program gives that type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stated {
    pub owner: TypeKey,
    pub offset: u32,
    pub bytes: u32,
    pub program: u32,
    pub id: u32,
}

/// One field a chain of loads went through: the type it is a field of, its offset, and the type the settled project gave what it refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Link {
    pub ty: TypeKey,
    pub offset: u32,
    pub refers: TypeKey,
}

/// What one access shows of a field: a row, whether a byte is a truth, the type of an object stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fact {
    Row(TypeKey, FieldFacts),
    Truth((TypeKey, u32), bool),
    Made((TypeKey, u32), TypeKey),
}

/// What an access through fields the settled project types showed, with the fields it went through from the base out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chained {
    pub links: Vec<Link>,
    pub fact: Fact,
}

/// The bytes of an object's header: its fields start after them.
const HEADER: u32 = 8;
/// The bit a builtin type's id carries: no program states such a type.
const BUILTIN: u32 = 0x8000_0000;
/// The fields an address may be loaded through, one after another, below a base of known type.
const CHAIN: usize = 8;
/// The times [`refine`] settles what was read through fields before it settles without it.
const ROUNDS: usize = 4;
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
// type is kept where it is one of the program's: what another program's type is, a value or an
// object, this program does not say, and [`settle`] looks it up once every program is in.
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
            if let (Some(id), None) = (field.ty, of) {
                if id & BUILTIN == 0 {
                    project.rows.stated.push(Stated {
                        owner: TypeKey(row.name),
                        offset: field.offset,
                        bytes: field.bytes,
                        program: program.program,
                        id,
                    });
                }
            }
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

/// How a function sets its variables and what the addresses it takes may reach.
#[derive(Default)]
struct Sets {
    /// Each variable assigned by name: the one type of the objects made into it, `None` where it is given anything else.
    made: BTreeMap<Var, Option<TypeKey>>,
    frame: Frame,
}

impl Sets {
    // ⚠ An address the function only loads or stores its own cell through reaches no other
    // variable: such a store is an assignment of the variable, and no address taken.
    fn of(f: &cfg::Function, project: Option<&Project>) -> Sets {
        let mut sets = Sets::default();
        let mut reached = Reached::default();
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
                if let Some(var) = own_cell_store(stmt) {
                    sets.give(var, None);
                }
                reached.stmt(stmt, &f.head, project);
            }
            if let Some(e) = block.term.expr() {
                reached.expr(e, &f.head, project);
            }
        }
        sets.frame = reached.finish();
        sets
    }

    fn give(&mut self, var: Var, ty: Option<TypeKey>) {
        let held = self.made.entry(var).or_insert(ty);
        if *held != ty {
            *held = None;
        }
    }

    /// The variables that hold an object of one known type wherever they are read.
    // ⚠ A variable an address the function takes may reach may be written through that address:
    // what is assigned to it by name then proves nothing.
    fn bases(&self, receiver: Option<TypeKey>) -> BTreeMap<Var, TypeKey> {
        let mut bases = BTreeMap::new();
        for (var, ty) in &self.made {
            let reached = match var {
                Var::Local(_) | Var::Value { .. } | Var::Stack { .. } => self.frame.reaches(*var),
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
            !self.made.contains_key(&this) && !self.frame.reaches(this)
        });
        if let Some(owner) = receiver {
            bases.insert(Var::Param(0), owner);
        }
        bases
    }
}

/// The addresses one function takes, as they are read: each variable's, and how far each may reach.
#[derive(Default)]
pub(crate) struct Reached {
    /// Each variable whose address is taken: how often, and how often only to reach its own cell or to be handed to a function whose reach the project bounds.
    taken: BTreeMap<Var, (u32, u32)>,
    /// Each address handed to such a function: the variable, the cell of it the address is of, and the cells from there the function may touch.
    spans: Vec<(Var, u32, u32)>,
}

impl Reached {
    /// Notes the addresses a statement takes, and a store through a variable's own address.
    pub(crate) fn stmt(&mut self, s: &Stmt, head: &Head, project: Option<&Project>) {
        self.own_store(s);
        s.each_expr(&mut |top| self.expr(top, head, project));
    }

    /// Notes a store, a clear or a step that reaches a variable's own cell through its address, and no other.
    pub(crate) fn own_store(&mut self, s: &Stmt) {
        if let Some(var) = own_cell_store(s) {
            self.taken.entry(var).or_default().1 += 1;
        }
    }

    /// Notes the addresses `top` takes; `project` is a settled project, whose reach table bounds an address handed whole to a function, or `None`, where any address handed on may reach anything.
    pub(crate) fn expr(&mut self, top: &Expr, head: &Head, project: Option<&Project>) {
        let Reached { taken, spans } = self;
        let mut handed: BTreeMap<*const Expr, u32> = BTreeMap::new();
        top.walk(&mut |e| match e {
            Expr::Call { callee, args, .. } => {
                if let Some(project) = project {
                    handed.extend(handed_on(project, head, callee, args));
                }
            }
            Expr::AddrOf { var, cell } => {
                let held = taken.entry(*var).or_default();
                held.0 += 1;
                if let Some(bytes) = handed.get(&std::ptr::from_ref(e)) {
                    held.1 += 1;
                    spans.push((*var, *cell, bytes.div_ceil(8)));
                }
            }
            Expr::Load { ty, addr, offset } => {
                if let Some(var) = own_cell(addr, *offset, ty.bytes()) {
                    taken.entry(var).or_default().1 += 1;
                }
            }
            _ => {}
        });
    }

    /// What the addresses noted may reach.
    pub(crate) fn finish(self) -> Frame {
        let mut frame = Frame::default();
        let low = |held: Option<u32>, n: u32| Some(held.map_or(n, |low| low.min(n)));
        for (var, (all, accounted)) in &self.taken {
            if all <= accounted {
                continue;
            }
            match *var {
                Var::Local(n) => frame.locals = low(frame.locals, n),
                Var::Param(n) => frame.params = low(frame.params, n),
                Var::Value { .. } | Var::Stack { .. } => {
                    frame.others.insert(*var);
                }
                Var::Global { .. } => {}
            }
        }
        let (mut locals, mut params) = (Vec::new(), Vec::new());
        for (var, cell, cells) in self.spans {
            let span = |n: u32| {
                let first = u64::from(n) + u64::from(cell);
                (first, first + u64::from(cells))
            };
            match var {
                _ if cells == 0 => {}
                Var::Local(n) => locals.push(span(n)),
                Var::Param(n) => params.push(span(n)),
                Var::Value { .. } | Var::Stack { .. } => {
                    frame.others.insert(var);
                }
                Var::Global { .. } => {}
            }
        }
        frame.local_spans = merged(locals);
        frame.param_spans = merged(params);
        frame
    }
}

/// The cells of a function's frame its addresses may reach.
#[derive(Default)]
pub(crate) struct Frame {
    /// The lowest local, and the lowest parameter, an address that may reach anything from it is taken of.
    locals: Option<u32>,
    params: Option<u32>,
    /// The temporaries an address of which is taken other than to reach their own cell.
    others: BTreeSet<Var>,
    /// The cells, first and past the last, a bounded address may touch, apart and by rising cell.
    local_spans: Vec<(u64, u64)>,
    param_spans: Vec<(u64, u64)>,
}

impl Frame {
    /// Whether an address the function takes may reach `var`, so that it may change otherwise than by its name.
    pub(crate) fn reaches(&self, var: Var) -> bool {
        match var {
            Var::Local(n) => {
                self.locals.is_some_and(|low| n >= low) || within(&self.local_spans, n)
            }
            Var::Param(n) => {
                self.params.is_some_and(|low| n >= low) || within(&self.param_spans, n)
            }
            Var::Value { .. } | Var::Stack { .. } => self.others.contains(&var),
            Var::Global { .. } => true,
        }
    }
}

/// `spans` by rising first cell, each overlapping or touching run made one.
fn merged(mut spans: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    spans.sort_unstable();
    let mut out: Vec<(u64, u64)> = Vec::with_capacity(spans.len());
    for (first, end) in spans {
        match out.last_mut() {
            Some(last) if first <= last.1 => last.1 = last.1.max(end),
            _ => out.push((first, end)),
        }
    }
    out
}

/// Whether cell `n` lies in one of `spans`, which are apart and by rising first cell.
fn within(spans: &[(u64, u64)], n: u32) -> bool {
    let n = u64::from(n);
    let at = spans.partition_point(|(first, _)| *first <= n);
    (at.checked_sub(1).and_then(|at| spans.get(at))).is_some_and(|(_, end)| *end > n)
}

/// Each argument of a call that is an address handed whole to a function of a program, with the bytes from it the function may touch, where the project bounds them.
// ⚠ An argument is placed at its cell of the callee's parameters only while every argument before
// it states its cells; past one that does not, no address is bounded.
fn handed_on(
    project: &Project,
    head: &Head,
    callee: &Callee,
    args: &[Expr],
) -> Vec<(*const Expr, u32)> {
    let Some(to) = signatures::key_of(head.program, callee) else {
        return Vec::new();
    };
    let mut handed = Vec::new();
    let mut at = Some(0u32);
    for arg in args {
        let Some(cell) = at else {
            break;
        };
        // ⚠ Only an address nothing keeps past the call: one kept may be written through later.
        if let Expr::AddrOf { .. } = arg {
            let bytes = project
                .reach(to, cell)
                .filter(|bytes| *bytes != reach::UNBOUNDED);
            let dies = project.address_dies(to, cell) == Some(true);
            if let (Some(bytes), true) = (bytes, dies) {
                handed.push((std::ptr::from_ref(arg), bytes));
            }
        }
        at = cells_of(arg, head).and_then(|cells| cell.checked_add(cells));
    }
    handed
}

/// The script type `kind` refers to an object of, where the project holds that type and it is no value type.
pub(crate) fn object_of(project: &Project, kind: &Kind) -> Option<TypeKey> {
    let Kind::Object(name) = kind else {
        return None;
    };
    let key = TypeKey::of_name(name).filter(|key| key.builtin().is_none() && !waits(*key))?;
    let facts = project.types.get(&key)?;
    (!facts.is_value()).then_some(key)
}

/// The variable whose own cell an access of `bytes` bytes at `offset` through `addr` reaches, where `addr` is its address and the access stays in that cell.
pub(crate) fn own_cell(addr: &Expr, offset: u32, bytes: u32) -> Option<Var> {
    let Expr::AddrOf { var, cell: 0 } = addr else {
        return None;
    };
    (u64::from(offset) + u64::from(bytes) <= 8).then_some(*var)
}

/// The variable whose own cell a store, a clear or a step reaches through its address, and no other.
pub(crate) fn own_cell_store(s: &Stmt) -> Option<Var> {
    match s {
        Stmt::Store {
            kind, addr, offset, ..
        } => own_cell(addr, *offset, kind.bytes()),
        Stmt::ZeroBytes { addr, bytes } => own_cell(addr, 0, *bytes),
        Stmt::StepI32 { addr, offset, .. } => own_cell(addr, *offset, 4),
        _ => None,
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

/// The sign a conversion of 32 bits to a wider integer or to a float reads them under: each sign has an instruction of its own.
fn converted(name: &str) -> Option<Kind> {
    match name {
        "i32_to_i64" | "i32_to_f32" | "i32_to_f64" => Some(Kind::I32),
        "u32_to_u64" | "u32_to_f32" | "u32_to_f64" => Some(Kind::U32),
        _ => None,
    }
}

/// The script type of object a settled `project` gives the reference at `offset` of a `ty`, its own field or a base type's.
fn refers(project: &Project, ty: TypeKey, offset: u32) -> Option<TypeKey> {
    let field = project.field(ty, offset).filter(|field| field.bytes == 8)?;
    object_of(project, &field.kind)
}

/// Whether the field a chain went through refers, in a settled `project`, to the type it did when the chain was read, or to one derived from it.
fn holds(project: &Project, link: &Link) -> bool {
    refers(project, link.ty, link.offset).is_some_and(|now| project.is_a(now, link.refers))
}

/// Which of `chained` every field it went through still holds for in a settled `project`.
fn holding(project: &Project, chained: &[Chained]) -> Vec<bool> {
    // Many accesses go through one field: each is looked up once.
    let mut known: BTreeMap<Link, bool> = BTreeMap::new();
    let mut held = Vec::with_capacity(chained.len());
    for seen in chained {
        let mut all = true;
        for link in &seen.links {
            let ok = *known.entry(*link).or_insert_with(|| holds(project, link));
            if !ok {
                all = false;
                break;
            }
        }
        held.push(all);
    }
    held
}

/// `e` under the casts around it.
fn uncast(e: &Expr) -> &Expr {
    let mut e = e;
    while let Expr::Cast { e: inner, .. } = e {
        e = &**inner;
    }
    e
}

/// One reading of a function for the fields its accesses show.
struct Seen<'a> {
    bases: &'a BTreeMap<Var, TypeKey>,
    /// The settled project of a second reading, whose fields of object types a base's loads go through; `None` in the first.
    project: Option<&'a Project>,
    /// The types among the bases' that are held in place: a base of one points at the value's first byte, where its first field lies.
    values: &'a BTreeSet<TypeKey>,
    head: &'a Head,
    rows: Vec<(TypeKey, FieldFacts)>,
    /// Each byte accessed through a base, and whether every such access was of a truth.
    truths: Vec<((TypeKey, u32), bool)>,
    /// The loads that stand where only whether they are zero is read, or that a `bool` keeps.
    tested: BTreeSet<*const Expr>,
    /// Each reference field stored an object of a script type through a base: the base's type, the offset, the object's type.
    made: Vec<(TypeKey, u32, TypeKey)>,
    /// What the accesses through fields the settled project types showed.
    chained: Vec<Chained>,
}

impl Seen<'_> {
    /// The type and the offset in it that an access at `offset` from `addr` reaches, and the fields loaded on the way, where `addr` is a base of known type moved by constants, or a reference loaded through fields the settled project types.
    fn place(&self, addr: &Expr, offset: u32) -> Option<(TypeKey, u32, Vec<Link>)> {
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
        // The offsets `addr` is loaded through, the last load first.
        let mut through: Vec<u32> = Vec::new();
        let var = loop {
            match addr {
                Expr::Var(var) => break var,
                Expr::Load {
                    ty: Scalar::U64,
                    addr: under,
                    offset: at,
                } if self.project.is_some() && through.len() < CHAIN => {
                    through.push(*at);
                    addr = &**under;
                }
                _ => return None,
            }
        };
        let mut key = *self.bases.get(var)?;
        let mut links = Vec::with_capacity(through.len());
        for at in through.iter().rev() {
            let to = refers(self.project?, key, *at)?;
            links.push(Link {
                ty: key,
                offset: *at,
                refers: to,
            });
            key = to;
        }
        Some((key, offset, links))
    }

    /// Keeps what one access showed: with the rows where it went through no field, apart where it did.
    fn keep(&mut self, links: Vec<Link>, fact: Fact) {
        if !links.is_empty() {
            self.chained.push(Chained { links, fact });
            return;
        }
        match fact {
            Fact::Row(key, row) => self.rows.push((key, row)),
            Fact::Truth(place, truth) => self.truths.push((place, truth)),
            Fact::Made((key, offset), ty) => self.made.push((key, offset, ty)),
        }
    }

    /// The offset the fields of a base of type `key` start at: an object's after its header, a value's at its first byte.
    fn start(&self, key: TypeKey) -> u32 {
        match self.values.contains(&key) {
            true => 0,
            false => HEADER,
        }
    }

    /// Notes whether an access of the one byte at `offset` from `addr` is of a truth, where that is a field of a base of known type.
    fn truth(&mut self, addr: &Expr, offset: u32, truth: bool) {
        if let Some((key, offset, links)) = self.place(addr, offset) {
            if offset >= self.start(key) {
                self.keep(links, Fact::Truth((key, offset), truth));
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
        if let Some((key, offset, links)) = self.place(addr, offset) {
            if offset >= self.start(key) && bytes > 0 {
                let facts = FieldFacts {
                    offset,
                    bytes,
                    kind,
                };
                self.keep(links, Fact::Row(key, facts));
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
    // ⚠ Only a division, a remainder, a right shift, an ordering and a conversion to a wider
    // number have an instruction for each sign. Any other operator is listed at one sign whatever
    // its operands hold: arithmetic and equality signed, a bit operator unsigned. So a field's
    // sign is the one every operator on it is listed at, and a field worked on at both has none.
    // A test of 32 bits against zero is listed unsigned whatever they are, and says nothing.
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

    /// The script type of the object a reference stored is made as, or that the variable it comes from holds.
    // ⚠ Evidence against a type only: a store through a base of no proved type is never seen, so
    // what the stores seen hold names no type the field is, nor an array or a string it holds.
    fn stored(&self, value: &Expr) -> Option<TypeKey> {
        let script = match value {
            Expr::NewObject(ty) => TypeKey::of(ty),
            Expr::Var(var) => self.bases.get(var).copied(),
            _ => None,
        };
        script.filter(|key| key.builtin().is_none() && !waits(*key))
    }

    /// The sign a value stored in four bytes says the field's are: that of an integer variable the kinds pass gives one.
    // ⚠ Never that of an expression: a cast to the other sign stores the same bits, and the stored
    // value's outer cast may be gone. A variable's own kind is what it is read as everywhere.
    fn sign(&self, value: &Expr) -> Kind {
        let Expr::Var(var) = value else {
            return Kind::Unknown;
        };
        match self.head.vars.get(var).map(|info| &info.kind) {
            Some(Kind::I32) => Kind::I32,
            Some(Kind::U32) => Kind::U32,
            _ => Kind::Unknown,
        }
    }

    /// Notes that a reference field at `offset` from `addr` is stored an object of the script type `ty`, where that is a field of a base of known type.
    fn made_as(&mut self, addr: &Expr, offset: u32, ty: TypeKey) {
        if let Some((key, offset, links)) = self.place(addr, offset) {
            if offset >= self.start(key) {
                self.keep(links, Fact::Made((key, offset), ty));
            }
        }
    }

    /// Notes a load a variable the kinds pass gives a `bool` keeps whole as one read only for whether it is zero.
    fn kept(&mut self, stmt: &Stmt) {
        let Stmt::Assign { target, value } = stmt else {
            return;
        };
        let truth = (self.head.vars.get(target)).is_some_and(|info| info.kind == Kind::Bool);
        let value = uncast(value);
        if let (true, Expr::Load { .. }) = (truth, value) {
            self.tested.insert(std::ptr::from_ref(value));
        }
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
                    StoreKind::Ref => {
                        if let Some(ty) = self.stored(value) {
                            self.made_as(addr, *offset, ty);
                        }
                        Kind::Pointer
                    }
                    // Eight zero bytes clear a reference as well as a number.
                    StoreKind::U64 if !zero => Kind::U64,
                    StoreKind::I32 if self.is_float(value) => Kind::F32,
                    StoreKind::I32 => self.sign(value),
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
            Stmt::Assign { .. } => self.kept(stmt),
            // Floats one after another, as many as the value has cells: no width is stated.
            Stmt::StoreF32Slots { .. } | Stmt::AssignMany { .. } | Stmt::Eval(_) => {}
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
            Expr::Call {
                callee: Callee::Intrinsic { name, .. },
                args,
                ..
            } => {
                if let (Some(sign), [read]) = (converted(name), args.as_slice()) {
                    self.operand(&sign, read, false);
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
    let sets = Sets::of(f, None);
    let bases = sets.bases(receiver);
    seen_through(project, f, &bases, &BTreeSet::new(), None);
}

/// Adds the fields one function's loads and stores show through what a settled project proves of it: the variables the kinds pass types as an object of a script type, and the receiver of a member, an object or a value.
// ⚠ Read after every program is in, with the kinds the project gives: a part for [`refine`].
pub fn of_typed_function(part: &mut Project, f: &cfg::Function, project: &Project) {
    let own = FunctionKey::new(f.head.program, f.head.at);
    let object = |key: TypeKey| project.types.get(&key).is_some_and(|ty| !ty.is_value());
    let receiver = (project.members.get(&own))
        .filter(|member| !member.is_static && f.head.params > 0)
        .map(|member| member.owner)
        .filter(|owner| project.types.contains_key(owner));
    // ⚠ A value type's method is handed a pointer to the value: its fields lie from the pointer's
    // first byte, at the offsets the value type's member table states them at.
    let values: BTreeSet<TypeKey> = receiver
        .filter(|owner| !object(*owner))
        .into_iter()
        .collect();
    let sets = Sets::of(f, Some(project));
    let mut bases = sets.bases(receiver);
    for (var, info) in &f.head.vars {
        let Kind::Object(name) = &info.kind else {
            continue;
        };
        let typed = TypeKey::of_name(name).filter(|key| key.builtin().is_none() && object(*key));
        if let (Some(key), false) = (typed, sets.frame.reaches(*var)) {
            bases.entry(*var).or_insert(key);
        }
    }
    seen_through(part, f, &bases, &values, Some(project));
}

/// Adds the rows, the truths and the objects stored the accesses of `f` through `bases` show; a base of a type in `values` points at a value held in place, and `settled`, where given, types the fields loaded below a base.
fn seen_through(
    project: &mut Project,
    f: &cfg::Function,
    bases: &BTreeMap<Var, TypeKey>,
    values: &BTreeSet<TypeKey>,
    settled: Option<&Project>,
) {
    if bases.is_empty() {
        return;
    }
    let mut seen = Seen {
        bases,
        project: settled,
        values,
        head: &f.head,
        rows: Vec::new(),
        truths: Vec::new(),
        tested: BTreeSet::new(),
        made: Vec::new(),
        chained: Vec::new(),
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
    for (key, offset, ty) in seen.made {
        project
            .rows
            .made
            .entry((key, offset))
            .or_default()
            .insert(ty);
    }
    project.rows.chained.extend(seen.chained);
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
    for (place, types) in std::mem::take(&mut from.rows.made) {
        into.rows.made.entry(place).or_default().extend(types);
    }
    into.rows.stated.append(&mut from.rows.stated);
    into.rows.chained.append(&mut from.rows.chained);
}

/// What a member table's field of another program's type holds, where a settled project knows that type: the value in place, or a reference to an object.
fn stated_kind(project: &Project, stated: &Stated) -> Option<Kind> {
    let key = project.type_key(stated.program, stated.id)?;
    if key.builtin().is_some() || waits(key) {
        return None;
    }
    let facts = project.types.get(&key)?;
    match (facts.is_value(), stated.bytes) {
        (true, bytes) => Some(Kind::Value {
            ty: key.to_string(),
            cells: bytes.div_ceil(8),
        }),
        (false, 8) => Some(Kind::Object(key.to_string())),
        (false, _) => None,
    }
}

/// The type that declares the field a row was seen as on `key`: the furthest base type the field still lies in; `None` for a row `key` itself does not hold.
// A type's fields lie after its base type's, so what ends within the base is the base's. A value
// held in place has no header: its bytes are its fields'.
pub(crate) fn declarer(
    types: &BTreeMap<TypeKey, TypeFacts>,
    key: TypeKey,
    row: &FieldFacts,
) -> Option<TypeKey> {
    let end = u64::from(row.offset) + u64::from(row.bytes);
    let holds = |ty: &TypeFacts| {
        let header = if ty.is_value() { 0 } else { HEADER };
        end <= u64::from(header) + u64::from(ty.byte_size)
    };
    let mut owner = key;
    let mut facts = types.get(&key);
    if facts.is_some_and(|ty| !holds(ty)) {
        return None;
    }
    // ⚠ The builtin a value type derives from holds none of its bytes.
    if facts.is_some_and(TypeFacts::is_value) {
        return Some(owner);
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
        // Two references that say other things of what they refer to are a reference and no more.
        (a, b) if named(&a) && named(b) => Some(Kind::Pointer),
        // Cells held in place, and the type a member table gives them.
        (Kind::Aggregate(n), Kind::Value { cells, .. }) if n == *cells => Some(b.clone()),
        (Kind::Value { ty, cells }, Kind::Aggregate(n)) if cells == *n => {
            Some(Kind::Value { ty, cells })
        }
        _ => None,
    }
}

/// The most derived of the types `called` that is each of them and that each of the types `made` is; `None` where none is.
// A call is made as the field's type or one above it, and an object stored is of the field's type
// or one below it. ⚠ Only a call names the type: a store through a base of no proved type is never
// seen, so the objects stored that are seen can only rule a type out.
fn narrowest(
    project: &Project,
    called: &BTreeSet<TypeKey>,
    made: &BTreeSet<TypeKey>,
) -> Option<TypeKey> {
    if called.len().saturating_add(made.len()) > RECEIVERS {
        return None;
    }
    let fits = |ty: TypeKey| {
        called.iter().all(|other| project.is_a(ty, *other))
            && made.iter().all(|object| project.is_a(*object, ty))
    };
    called.iter().copied().find(|ty| fits(*ty))
}

/// The one field the rows seen at an offset make: none where two widths were seen, and of no kind where two kinds were.
// A reference is of the most derived type the calls on it are made as, where every object stored
// in it is one; where there is none it is a reference and no more.
fn agreed(project: &Project, rows: &[FieldFacts], made: &BTreeSet<TypeKey>) -> Option<FieldFacts> {
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
    // ⚠ Only a reference of eight bytes is stored an object.
    let empty = BTreeSet::new();
    let made = if first.bytes == 8 { made } else { &empty };
    let kind = match (kind, called.is_empty() && made.is_empty()) {
        (Some(kind), true) => kind,
        (Some(Kind::Pointer), false) => match narrowest(project, &called, made) {
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
    // A member table's field of another program's type, now that the type is known.
    for stated in std::mem::take(&mut project.rows.stated) {
        if let Some(kind) = stated_kind(project, &stated) {
            let row = FieldFacts {
                offset: stated.offset,
                bytes: stated.bytes,
                kind,
            };
            note(project, stated.owner, row);
        }
    }
    // Kept, so that [`refine`] settles them again with more.
    let stored = std::mem::take(&mut project.rows.made);
    let chained = std::mem::take(&mut project.rows.chained);
    project.rows = Rows {
        fields: project.fields.clone(),
        truths: project.truths.clone(),
        made: stored.clone(),
        stated: Vec::new(),
        chained,
    };
    // The objects stored in each field, under the type that declares it.
    let mut made: BTreeMap<(TypeKey, u32), BTreeSet<TypeKey>> = BTreeMap::new();
    for ((key, offset), types) in stored {
        let row = FieldFacts {
            offset,
            bytes: 8,
            kind: Kind::Pointer,
        };
        if let Some(owner) = declarer(&project.types, key, &row) {
            made.entry((owner, offset)).or_default().extend(types);
        }
    }
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
    let none = BTreeSet::new();
    let mut settled = BTreeMap::new();
    for (owner, offsets) in seen {
        let mut fields: Vec<FieldFacts> = offsets
            .into_iter()
            .filter_map(|(offset, rows)| {
                let stored = made.get(&(owner, offset)).unwrap_or(&none);
                agreed(known, &rows, stored)
            })
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

/// Adds the rows, the truths and the objects stored of `parts`, each read by [`of_typed_function`], to those the fields of `project` were settled from, and settles them again.
// ⚠ What was read through a field holds only while that field refers to the type it did then,
// and settling it in may change the field: it is settled in until what holds is what was
// settled, and left out where that takes more than a few rounds.
pub fn refine(project: &mut Project, parts: Vec<Project>) {
    let Rows {
        fields,
        truths,
        made,
        stated,
        chained,
    } = std::mem::take(&mut project.rows);
    project.fields = fields;
    project.truths = truths;
    project.rows.made = made;
    project.rows.stated = stated;
    project.rows.chained = chained;
    for mut part in parts {
        absorb(project, &mut part);
    }
    let chained = std::mem::take(&mut project.rows.chained);
    settle(project);
    let plain = std::mem::take(&mut project.rows);
    let mut applied = vec![false; chained.len()];
    let mut agrees = false;
    for _ in 0..ROUNDS {
        let held = holding(project, &chained);
        if held == applied {
            agrees = true;
            break;
        }
        applied = held;
        let facts = chained
            .iter()
            .zip(&applied)
            .filter_map(|(seen, on)| on.then_some(&seen.fact));
        settle_with(project, &plain, facts);
    }
    if !agrees {
        settle_with(project, &plain, std::iter::empty::<&Fact>());
    }
    project.rows = Rows { chained, ..plain };
}

/// Settles the fields of `project` from `plain`, the rows they were settled from, and `facts` besides.
fn settle_with<'a>(project: &mut Project, plain: &Rows, facts: impl Iterator<Item = &'a Fact>) {
    project.fields = plain.fields.clone();
    project.truths = plain.truths.clone();
    project.rows.made = plain.made.clone();
    for fact in facts {
        match fact {
            Fact::Row(key, row) => note(project, *key, row.clone()),
            Fact::Truth(place, truth) => {
                *project.truths.entry(*place).or_insert(true) &= *truth;
            }
            Fact::Made(place, ty) => {
                project.rows.made.entry(*place).or_default().insert(*ty);
            }
        }
    }
    settle(project);
}

#[cfg(test)]
#[path = "layouts_tests.rs"]
mod tests;
