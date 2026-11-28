//! Which functions and types of a program something outside it names: the code and the tables of
//! another program, the engine by an entry, or code no part read.
//!
//! ⚠ Only a program every way in to which was read proves anything: one read from a listing, one
//! a program nothing read imports, and every program of a set one body of which did not read as a
//! program answer that anything of them may be named.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{signatures, Project};
use crate::gta6::script::cfg;
use crate::gta6::script::decode;
use crate::gta6::script::ir::{
    Callee, Expr, FunctionKey, Kind, Stmt, TypeKey, Var, VmType, FUNCTION_OFFSET,
};
use crate::gta6::script::opcode::Op;
use crate::gta6::script::program::{FunctionRow, Program, SlotRow, TypeRow};

/// The bit that marks a builtin type's id.
const BUILTIN_ID: u32 = 0x8000_0000;
/// The ancestors followed up from a type: no line is that long, and a ring of bases must end.
const MAX_BASES: usize = 64;
/// The arrays of arrays a kind is read through.
const MAX_NESTED: u32 = 16;
/// The runtime's type every class's line runs up to, which derives from nothing and implements nothing.
const OBJECT: TypeKey = TypeKey(0x8000_0000);
/// The runtime's type every struct derives from, which derives from `OBJECT` alone and implements nothing.
const VALUE_TYPE: TypeKey = TypeKey(0x8000_0001);
/// The runtime's type every enum derives from, which derives from `VALUE_TYPE`.
const ENUM: TypeKey = TypeKey(0x8000_0029);

/// Who names a thing: a program's code or tables, or `None` for the engine and for code no part read, which count whatever they name.
type Namer = Option<u32>;

/// What something outside each program names of it, as `settle` proves it.
// One number a closed program, a reached function and a program whose cells are shared, two an
// unnamed type: at most the project's functions and types once each.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outside {
    /// The programs, rising, every way in to whose functions was read: of no other is a function proved unreached.
    #[serde(default)]
    pub closed: Vec<u32>,
    /// The offsets, rising, of the functions of each closed program that something outside it may call, refer to or reach through a slot.
    #[serde(default)]
    pub reached: BTreeMap<u32, Vec<u32>>,
    /// The types, by rising key, that nothing outside their one home names, each with that home.
    #[serde(default)]
    pub unnamed: Vec<(TypeKey, u32)>,
    /// The programs, rising, whose globals or statics code of another program reads or writes.
    #[serde(default)]
    pub shared_cells: Vec<u32>,
}

impl Outside {
    pub fn is_empty(&self) -> bool {
        self == &Outside::default()
    }

    /// Whether something outside the function's program may call it, refer to it or reach it through a slot; true where nothing proves otherwise.
    pub fn function_reached(&self, key: FunctionKey) -> bool {
        if self.closed.binary_search(&key.program()).is_err() {
            return true;
        }
        let reached = self.reached.get(&key.program());
        reached.is_some_and(|offsets| offsets.binary_search(&key.at()).is_ok())
    }

    /// Whether something outside the type's one home may name it; true where nothing proves otherwise.
    pub fn type_named(&self, ty: TypeKey) -> bool {
        self.unnamed
            .binary_search_by_key(&ty, |(key, _)| *key)
            .is_err()
    }

    /// Puts each list in the order a lookup takes, as a file read back may not hold it.
    // ⚠ A reached offset out of order would be missed, and its function read as unreached.
    pub(super) fn tidy(&mut self) {
        self.closed.sort_unstable();
        self.closed.dedup();
        for offsets in self.reached.values_mut() {
            offsets.sort_unstable();
            offsets.dedup();
        }
        self.unnamed.sort_unstable();
        // A type stated twice, of two homes, has no one home.
        let mut twice = BTreeSet::new();
        for pair in self.unnamed.windows(2) {
            if let [(a, _), (b, _)] = pair {
                if a == b {
                    twice.insert(*a);
                }
            }
        }
        self.unnamed.retain(|(key, _)| !twice.contains(key));
        self.shared_cells.sort_unstable();
        self.shared_cells.dedup();
    }

    /// What of this is of the programs `keep` takes: a function or a type of any other answers true.
    pub(super) fn cut(&self, keep: impl Fn(u32) -> bool) -> Outside {
        let reached = self.reached.iter().filter(|(program, _)| keep(**program));
        Outside {
            closed: self.closed.iter().copied().filter(|p| keep(*p)).collect(),
            reached: reached.map(|(p, offsets)| (*p, offsets.clone())).collect(),
            unnamed: (self.unnamed.iter().copied())
                .filter(|(_, home)| keep(*home))
                .collect(),
            shared_cells: self.shared_cells.clone(),
        }
    }
}

/// What the parts show of what code and tables name across programs, until every program is in; empty in a settled project.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Naming {
    /// The programs read from a compiled program, whose engine entries are among `functions`.
    stated: BTreeSet<u32>,
    /// The programs any function or type of which code no part read may name.
    blind: BTreeSet<u32>,
    /// The programs any type of which code that states no value's type, or an id no table resolves, may name.
    blind_types: BTreeSet<u32>,
    /// Whether code no part read may call through any slot of any type.
    any_slot: bool,
    /// Whether a body shaped as a program did not read as one: what it imports is nothing's to say.
    unread_file: bool,
    /// The functions something outside their program names.
    functions: BTreeSet<FunctionKey>,
    /// Each call through a slot, or delegate of one: who makes it, the type it goes through and the slot.
    slots: BTreeSet<(Namer, TypeKey, u32)>,
    /// Each script type code or a table names, by who names it.
    types: BTreeSet<(Namer, TypeKey)>,
    /// The programs whose globals or statics code of another program reads or writes.
    shared_cells: BTreeSet<u32>,
    /// One row a type of a compiled program's type table.
    rows: Vec<Row>,
}

impl Naming {
    /// Notes code of `program` no part read: it may name any function and type of it and of `imports`, and call through any slot.
    fn unread(&mut self, program: u32, imports: impl IntoIterator<Item = u32>) {
        self.blind.insert(program);
        self.blind.extend(imports);
        self.any_slot = true;
    }
}

/// One type as a compiled program's type table states it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Row {
    key: TypeKey,
    home: u32,
    base: Option<TypeKey>,
    interfaces: Vec<TypeKey>,
    /// The generic type this is an instance of.
    generic: Option<TypeKey>,
    /// Whether the base, an interface or the generic type is an id the tables do not resolve.
    unresolved: bool,
    /// The function each virtual slot holds, by the slot.
    vtable: Vec<Option<FunctionKey>>,
}

/// Notes what a compiled program's tables name: its engine entries, the functions of other programs its code names, and each row of its type table; `unread_file` is whether a body of its set did not read.
pub(super) fn of_program(
    project: &mut Project,
    program: &Program,
    tables: &Project,
    unread_file: bool,
) {
    let own = program.program;
    let naming = &mut project.naming;
    naming.stated.insert(own);
    naming.unread_file |= unread_file;
    // ⚠ The engine runs these by a name's hash or by the header's word.
    let engine = program.entries.iter().map(|entry| entry.at);
    let engine = engine.chain(program.main).chain(program.initialize);
    naming
        .functions
        .extend(engine.map(|at| FunctionKey::new(own, at)));
    // Every instruction of the code, those no function's code reaches among them.
    let foreign = program.called_elsewhere().into_iter();
    let foreign = foreign.filter(|(of, _)| *of != own);
    naming
        .functions
        .extend(foreign.map(|(of, at)| FunctionKey::new(of, at)));
    for row in &program.types {
        table_row(naming, program, tables, row);
    }
}

/// The type `id` names in `program`; `Err` with the program any type of which it may be where the tables do not resolve it.
fn type_of(program: &Program, tables: &Project, id: u32) -> Result<TypeKey, u32> {
    if id & BUILTIN_ID != 0 {
        return Ok(TypeKey(u64::from(id)));
    }
    let imported = program.imports.iter().find(|i| i.types.contains(&id));
    if let Some(import) = imported {
        return tables.type_key(program.program, id).ok_or(import.program);
    }
    // An own row is named as the code names it, by the key its name gives.
    let row = program.types.get(id as usize);
    row.map(|row| TypeKey(row.name)).ok_or(program.program)
}

/// The type `id` names in `program`, noted as named by the program; `None` where the tables do not resolve it.
fn named_id(naming: &mut Naming, program: &Program, tables: &Project, id: u32) -> Option<TypeKey> {
    match type_of(program, tables, id) {
        Ok(key) => {
            if key.builtin().is_none() {
                naming.types.insert((Some(program.program), key));
            }
            Some(key)
        }
        Err(of) => {
            naming.blind_types.insert(of);
            None
        }
    }
}

/// Notes one row of a program's type table: the types it names and what each of its slots holds.
fn table_row(naming: &mut Naming, program: &Program, tables: &Project, row: &TypeRow) {
    let vtable = row.vtable.iter().map(|slot| match slot {
        SlotRow::Function { import, at, .. } => program
            .imported(*import)
            .map(|of| FunctionKey::new(of, *at)),
        SlotRow::Empty | SlotRow::Builtin { .. } => None,
    });
    let vtable: Vec<Option<FunctionKey>> = vtable.collect();
    let mut unresolved = false;
    let mut base = None;
    if let Some(id) = row.base {
        base = named_id(naming, program, tables, id);
        unresolved |= base.is_none();
    }
    let mut interfaces = Vec::with_capacity(row.interfaces.len());
    for id in &row.interfaces {
        match named_id(naming, program, tables, *id) {
            Some(key) => interfaces.push(key),
            None => unresolved = true,
        }
    }
    let mut generic = None;
    if let Some((of, arguments)) = &row.generic {
        generic = named_id(naming, program, tables, *of);
        unresolved |= generic.is_none();
        for id in arguments {
            named_id(naming, program, tables, *id);
        }
    }
    for id in row.fields.iter().filter_map(|field| field.ty) {
        named_id(naming, program, tables, id);
    }
    let own = row.methods.start as usize..row.methods.end as usize;
    let methods = program.methods.get(own).unwrap_or_default();
    for id in methods.iter().filter_map(|method| method.result) {
        named_id(naming, program, tables, id);
    }
    naming.rows.push(Row {
        key: TypeKey(row.name),
        home: program.program,
        base,
        interfaces,
        generic,
        unresolved,
        vtable,
    });
}

/// Notes what one function's instructions name; `read` is whether its code lifted, as code no part read counts whatever it names.
pub(super) fn of_function(
    project: &mut Project,
    program: &Program,
    tables: &Project,
    row: &FunctionRow,
    read: bool,
) {
    let own = program.program;
    let naming = &mut project.naming;
    let Ok(insns) = decode::function_in(program, tables, row) else {
        naming.unread(own, program.imports.iter().map(|import| import.program));
        return;
    };
    let namer = read.then_some(own);
    let mut seen = Seen { naming, own, namer };
    for op in insns.iter().flat_map(|insn| &insn.ops) {
        seen.op(op);
    }
    // ⚠ A step that stores a value, or takes it out of its box, keeps only the bytes of its type.
    for key in decode::value_types(program, tables, &insns).into_values() {
        seen.ty(key);
    }
}

/// Notes what one function of a lifted listing names, as first read.
// ⚠ A listing states no value's type: `listed` has taken any type of what it imports to be named.
pub(super) fn of_listed(project: &mut Project, f: &cfg::Function) {
    let own = f.head.program;
    let mut seen = Seen {
        naming: &mut project.naming,
        own,
        namer: Some(own),
    };
    for block in &f.cfg.blocks {
        for stmt in &block.stmts {
            match stmt {
                Stmt::Assign { target, .. } => seen.var(*target),
                Stmt::AssignMany { targets, .. } => {
                    for target in targets {
                        seen.var(*target);
                    }
                }
                _ => {}
            }
            stmt.each_expr(&mut |top| top.walk(&mut |e| seen.node(e)));
        }
        if let Some(e) = block.term.expr() {
            e.walk(&mut |e| seen.node(e));
        }
    }
}

/// Notes a program read from a lifted listing, which imports `imports`: no type of theirs is proved unnamed.
pub(super) fn listed(project: &mut Project, imports: &[u32]) {
    let naming = &mut project.naming;
    naming.blind_types.extend(imports.iter().copied());
}

/// Notes code of `program` no part read, which imports `imports`.
pub(super) fn unread(project: &mut Project, program: u32, imports: &[u32]) {
    project.naming.unread(program, imports.iter().copied());
}

/// One reading of what a function's code names.
// ⚠ A reference is counted where it is taken, whatever it is then handed to: a native, a builtin,
// a delegate. One a program takes of its own function and hands on is named by its own code.
struct Seen<'a> {
    naming: &'a mut Naming,
    own: u32,
    namer: Namer,
}

impl Seen<'_> {
    fn function(&mut self, key: FunctionKey) {
        if self.namer.is_none() || key.program() != self.own {
            self.naming.functions.insert(key);
        }
    }

    fn ty(&mut self, key: TypeKey) {
        if key.builtin().is_none() {
            self.naming.types.insert((self.namer, key));
        }
    }

    fn vm_type(&mut self, ty: &VmType) {
        if let Some(key) = TypeKey::of(ty) {
            self.ty(key);
        }
    }

    /// A call through slot `slot` of the type `ty` names, or a delegate of it; a type no key names may be any.
    fn slot(&mut self, ty: Option<TypeKey>, slot: u32) {
        match ty {
            Some(key) => {
                self.ty(key);
                self.naming.slots.insert((self.namer, key, slot));
            }
            None => self.naming.any_slot = true,
        }
    }

    fn var(&mut self, var: Var) {
        if let Var::Global { program, .. } = var {
            if program != self.own {
                self.naming.shared_cells.insert(program);
            }
        }
    }

    fn callee(&mut self, callee: &Callee) {
        if let Some(key) = signatures::key_of(self.own, callee) {
            self.function(key);
        }
        match callee {
            Callee::Virtual { ty, slot, .. } => self.slot(TypeKey::of_name(ty), *slot),
            Callee::Intrinsic { ty: Some(ty), .. } => self.vm_type(ty),
            _ => {}
        }
    }

    /// One expression, without those inside it.
    fn node(&mut self, e: &Expr) {
        match e {
            Expr::FunctionRef { program, at } => {
                self.function(FunctionKey::new(*program, *at & FUNCTION_OFFSET));
            }
            Expr::VirtualFunctionRef { ty, slot, .. } => self.slot(TypeKey::of(ty), *slot),
            Expr::NewObject(ty) | Expr::NewArray { ty, .. } => self.vm_type(ty),
            Expr::Call { callee, .. } => self.callee(callee),
            Expr::Var(var) | Expr::AddrOf { var, .. } => self.var(*var),
            _ => {}
        }
    }

    fn op(&mut self, op: &Op) {
        match op {
            Op::Push { e, .. } => e.walk(&mut |e| self.node(e)),
            Op::Set(var) => self.var(*var),
            Op::NewArray { ty, .. } => self.vm_type(ty),
            Op::VirtualFunctionRef { ty, slot } => self.slot(TypeKey::of(ty), *slot),
            Op::Call { callee, .. } | Op::Unsized { callee, .. } => self.callee(callee),
            _ => {}
        }
    }
}

/// Moves what `from` shows of what is named across programs into `into`.
pub(super) fn absorb(into: &mut Project, from: &mut Project) {
    let from = std::mem::take(&mut from.naming);
    let into = &mut into.naming;
    into.stated.extend(from.stated);
    into.blind.extend(from.blind);
    into.blind_types.extend(from.blind_types);
    into.any_slot |= from.any_slot;
    into.unread_file |= from.unread_file;
    into.functions.extend(from.functions);
    into.slots.extend(from.slots);
    into.types.extend(from.types);
    into.shared_cells.extend(from.shared_cells);
    into.rows.extend(from.rows);
}

/// The type rows of every program, with what each lies under.
struct Graph<'a> {
    rows: BTreeMap<TypeKey, Vec<&'a Row>>,
    /// The types each type is the base, an interface or the generic type of.
    children: BTreeMap<TypeKey, Vec<TypeKey>>,
    /// The types whose line runs up through a type no row states: any type may be above them.
    rootless: Vec<TypeKey>,
    /// Every type some row lists as an interface.
    interfaces: BTreeSet<TypeKey>,
}

impl<'a> Graph<'a> {
    fn of(rows: &'a [Row]) -> Graph<'a> {
        let mut by_key: BTreeMap<TypeKey, Vec<&Row>> = BTreeMap::new();
        for row in rows {
            by_key.entry(row.key).or_default().push(row);
        }
        let mut children: BTreeMap<TypeKey, Vec<TypeKey>> = BTreeMap::new();
        let mut rootless = BTreeSet::new();
        let mut interfaces = BTreeSet::new();
        // A builtin is the runtime's, which no row states.
        let known = |key: &TypeKey| key.builtin().is_some() || by_key.contains_key(key);
        for row in rows {
            let above = row.base.iter().chain(&row.interfaces).chain(&row.generic);
            for key in above {
                children.entry(*key).or_default().push(row.key);
                if !known(key) {
                    rootless.insert(row.key);
                }
            }
            if row.unresolved {
                rootless.insert(row.key);
            }
            interfaces.extend(row.interfaces.iter().copied());
        }
        for below in children.values_mut() {
            below.sort_unstable();
            below.dedup();
        }
        Graph {
            rows: by_key,
            children,
            rootless: rootless.into_iter().collect(),
            interfaces,
        }
    }

    /// `ty`, every type below it, and every type whose line no row states whole.
    // Each type is taken once, so this ends.
    fn below(&self, ty: TypeKey) -> BTreeSet<TypeKey> {
        let mut seen = BTreeSet::from([ty]);
        let mut next = vec![ty];
        while let Some(at) = next.pop() {
            for child in self.children.get(&at).into_iter().flatten() {
                if seen.insert(*child) {
                    next.push(*child);
                }
            }
        }
        seen.extend(self.rootless.iter().copied());
        seen
    }

    fn all_rows(&self) -> impl Iterator<Item = &'a Row> + '_ {
        self.rows.values().flatten().copied()
    }

    /// The rows of the types `below` gives.
    fn rows_below(&self, ty: TypeKey) -> Vec<&'a Row> {
        let below = self.below(ty);
        let rows = below.iter().filter_map(|key| self.rows.get(key));
        rows.flatten().copied().collect()
    }

    /// Adds the functions a type's table names in another program: those a slot holds, and every slot of a line another program's type derives from.
    fn held(&self, out: &mut BTreeSet<FunctionKey>) {
        for row in self.all_rows() {
            let foreign = |f: &&FunctionKey| f.program() != row.home;
            out.extend(row.vtable.iter().flatten().filter(foreign).copied());
            let mut above = row.base;
            let mut seen = BTreeSet::new();
            for _ in 0..MAX_BASES {
                let Some(key) = above.filter(|key| seen.insert(*key)) else {
                    break;
                };
                let Some(rows) = self.rows.get(&key) else {
                    break;
                };
                let mut next = None;
                for base in rows {
                    if base.home != row.home {
                        out.extend(base.vtable.iter().flatten().filter(foreign).copied());
                    }
                    next = next.or(base.base);
                }
                above = next;
            }
        }
    }

    /// The line `ty` runs up through rows to the first builtin, that builtin included; `None` where no row, or no one row, states some type of it.
    fn line(&self, ty: TypeKey) -> Option<Vec<TypeKey>> {
        let mut line = Vec::new();
        let mut at = ty;
        for _ in 0..MAX_BASES {
            if line.contains(&at) {
                return None;
            }
            line.push(at);
            if at.builtin().is_some() {
                return Some(line);
            }
            let rows = self.rows.get(&at)?;
            let first = rows.first()?;
            let alike = rows.iter().all(|row| row.base == first.base);
            if !alike || rows.iter().any(|row| row.unresolved) {
                return None;
            }
            // A type with no base is a line's root: an interface's, or one the runtime holds no more of.
            match first.base {
                Some(base) => at = base,
                None => return Some(line),
            }
        }
        None
    }

    /// Whether an object of `ty` may be of the builtin class `class`.
    // ⚠ Only the runtime knows what its own types derive from, but for its roots: a line that
    // runs up to one of those is of the builtins above it and no other.
    fn may_be(&self, ty: TypeKey, class: TypeKey) -> bool {
        let Some(line) = self.line(ty) else {
            return true;
        };
        if line.contains(&class) {
            return true;
        }
        match line.last() {
            Some(&OBJECT) => false,
            Some(&VALUE_TYPE) => class == OBJECT,
            Some(&ENUM) => class == VALUE_TYPE || class == OBJECT,
            // A root of no base is below the runtime's object alone.
            Some(top) if top.builtin().is_none() => class == OBJECT,
            _ => true,
        }
    }

    /// Whether an object of `ty` may implement an interface its rows do not list: through a builtin above it that is not one of the runtime's roots, or a line no row states whole.
    fn may_implement(&self, ty: TypeKey) -> bool {
        match self.line(ty) {
            Some(line) => match line.last() {
                Some(&OBJECT) | Some(&VALUE_TYPE) => false,
                Some(top) => top.builtin().is_some(),
                None => false,
            },
            None => true,
        }
    }

    /// Adds the functions a call through a slot that `slots` names may land on, where the caller is not the function's program; with `any`, every function a slot holds.
    // ⚠ A call through a class lands on what the slot of that index holds in the object's type,
    // which lies below the class. An interface's slots lie where each type's base list puts them,
    // so every slot of every type that may implement one is taken. A type lists each interface
    // it implements that no builtin above it does, as the runtime finds the interface's slots by
    // that list.
    fn dispatched(
        &self,
        slots: &BTreeSet<(Namer, TypeKey, u32)>,
        any: bool,
        out: &mut BTreeSet<FunctionKey>,
    ) {
        if any {
            for row in self.all_rows() {
                out.extend(row.vtable.iter().flatten().copied());
            }
            return;
        }
        let mut calls: BTreeMap<TypeKey, BTreeMap<u32, BTreeSet<Namer>>> = BTreeMap::new();
        for (namer, ty, slot) in slots {
            let by_slot = calls.entry(*ty).or_default();
            by_slot.entry(*slot).or_default().insert(*namer);
        }
        // The types that may implement an interface they do not list, found once.
        let mut open: Option<Vec<&Row>> = None;
        for (ty, by_slot) in &calls {
            let interface = self.interfaces.contains(ty);
            let stated = self.rows.contains_key(ty);
            // A type no row states is the runtime's, and may be an interface no row lists.
            if interface || !stated {
                let open = open.get_or_insert_with(|| {
                    let rows = self.all_rows();
                    rows.filter(|row| self.may_implement(row.key)).collect()
                });
                let listed: Vec<&Row> = match interface {
                    true => self.rows_below(*ty),
                    false => Vec::new(),
                };
                let namers: BTreeSet<Namer> = by_slot.values().flatten().copied().collect();
                for row in listed.into_iter().chain(open.iter().copied()) {
                    for f in row.vtable.iter().flatten() {
                        land(&namers, *f, out);
                    }
                }
                if interface {
                    continue;
                }
            }
            // What may be of a class the runtime holds is read off each line.
            let rows: Vec<&Row> = match stated {
                true => self.rows_below(*ty),
                false => (self.all_rows())
                    .filter(|row| self.may_be(row.key, *ty))
                    .collect(),
            };
            for row in rows {
                for (slot, namers) in by_slot {
                    if let Some(Some(f)) = row.vtable.get(*slot as usize) {
                        land(namers, *f, out);
                    }
                }
            }
        }
    }
}

/// Adds `f` where one of `namers` is outside its program.
fn land(namers: &BTreeSet<Namer>, f: FunctionKey, out: &mut BTreeSet<FunctionKey>) {
    if namers
        .iter()
        .any(|namer| namer.is_none_or(|n| n != f.program()))
    {
        out.insert(f);
    }
}

/// Once every part is in and every table is settled: which functions of each closed program are reached from outside, and which of its types nothing outside names.
pub(super) fn settle(project: &mut Project) {
    let mut naming = std::mem::take(&mut project.naming);
    naming.rows.sort_unstable();
    naming.rows.dedup();
    let graph = Graph::of(&naming.rows);
    let closed: BTreeSet<u32> = match naming.unread_file {
        true => BTreeSet::new(),
        false => naming.stated.difference(&naming.blind).copied().collect(),
    };
    let mut reached = std::mem::take(&mut naming.functions);
    graph.held(&mut reached);
    graph.dispatched(&naming.slots, naming.any_slot, &mut reached);
    let mut by_program: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    // In key order: each program's offsets rise.
    for key in reached
        .into_iter()
        .filter(|key| closed.contains(&key.program()))
    {
        by_program.entry(key.program()).or_default().push(key.at());
    }

    let typed: BTreeSet<u32> = closed.difference(&naming.blind_types).copied().collect();
    let mut unnamed: BTreeMap<TypeKey, u32> = BTreeMap::new();
    for (key, rows) in &graph.rows {
        // ⚠ Two programs may define one name: neither is then its one home.
        let Some(home) = rows.first().map(|row| row.home) else {
            continue;
        };
        let alone = rows.iter().all(|row| row.home == home);
        if alone && typed.contains(&home) && key.builtin().is_none() {
            unnamed.insert(*key, home);
        }
    }
    for (namer, key) in &naming.types {
        if unnamed.get(key).is_some_and(|home| *namer != Some(*home)) {
            unnamed.remove(key);
        }
    }
    project.outside = Outside {
        closed: closed.into_iter().collect(),
        reached: by_program,
        unnamed: unnamed.into_iter().collect(),
        shared_cells: naming.shared_cells.into_iter().collect(),
    };
    close(project);
}

/// The types still unnamed, and those taken off them and not yet followed.
struct Closing {
    unnamed: BTreeMap<TypeKey, u32>,
    named: Vec<TypeKey>,
}

impl Closing {
    /// Notes `key` as named by `by`: taken off where that is not its home.
    fn name(&mut self, key: TypeKey, by: Namer) {
        if self.unnamed.get(&key).is_some_and(|home| by != Some(*home)) {
            self.unnamed.remove(&key);
            self.named.push(key);
        }
    }

    /// Notes each type `kind` names as named by `by`.
    fn kind(&mut self, kind: &Kind, by: Namer) {
        let mut kind = kind;
        for _ in 0..MAX_NESTED {
            match kind {
                Kind::Object(name) | Kind::Value { ty: name, .. } => {
                    if let Some(key) = TypeKey::of_name(name) {
                        self.name(key, by);
                    }
                    return;
                }
                Kind::Array(inner) => kind = &**inner,
                _ => return,
            }
        }
    }
}

/// Notes the types what is stated of `key` names, as named by `by`: its base, interfaces and generic type and arguments, and its fields' kinds.
fn holds(project: &Project, closing: &mut Closing, key: TypeKey, by: Namer) {
    if let Some(facts) = project.types.get(&key) {
        let generic =
            (facts.generic.iter()).flat_map(|(of, arguments)| std::iter::once(of).chain(arguments));
        let above = facts.base.iter().chain(&facts.interfaces).chain(generic);
        for named in above.copied().collect::<Vec<TypeKey>>() {
            closing.name(named, by);
        }
    }
    for field in project.fields.get(&key).into_iter().flatten() {
        closing.kind(&field.kind, by);
    }
}

/// Takes off the unnamed types each one what the project states may show outside its home: the kinds of what is reached from outside, of what another program states, and of all a type named outside holds.
// ⚠ What a later pass types a value as comes of these tables, so a type the code outside its home
// never names may still be written there: as a parameter of a function called from outside, a
// field of a type named there, a global another program reads.
pub(super) fn close(project: &mut Project) {
    if project.outside.unnamed.is_empty() {
        return;
    }
    let unnamed = std::mem::take(&mut project.outside.unnamed);
    let mut closing = Closing {
        unnamed: unnamed.into_iter().collect(),
        named: Vec::new(),
    };
    let seen: &Project = project;
    let by = |key: &FunctionKey| (!seen.outside.function_reached(*key)).then_some(key.program());
    for (key, signature) in &seen.signatures {
        let by = by(key);
        for param in &signature.params {
            closing.kind(&param.kind, by);
        }
        closing.kind(&signature.result, by);
    }
    for (key, member) in &seen.members {
        closing.name(member.owner, by(key));
    }
    for (key, kind) in &seen.slot_results {
        closing.kind(kind, by(key));
    }
    for (key, params) in &seen.slot_params {
        let by = by(key);
        for param in params {
            closing.kind(&param.kind, by);
        }
    }
    for (program, info) in &seen.programs {
        let shared = seen.outside.shared_cells.binary_search(program).is_ok();
        let by = (!shared).then_some(*program);
        let stated = info.globals.values().chain(info.statics.values());
        let cells = (info.global_cells.iter().chain(&info.static_cells)).map(|cell| &cell.kind);
        for kind in stated.chain(cells) {
            closing.kind(kind, by);
        }
    }
    // A type still unnamed is stated in its home; one taken off is followed again below.
    let keys: BTreeSet<TypeKey> = seen
        .types
        .keys()
        .chain(seen.fields.keys())
        .copied()
        .collect();
    for key in keys {
        let by = closing.unnamed.get(&key).copied();
        holds(seen, &mut closing, key, by);
    }
    // A type is taken off once, so this ends.
    while let Some(key) = closing.named.pop() {
        holds(seen, &mut closing, key, None);
    }
    project.outside.unnamed = closing.unnamed.into_iter().collect();
}

#[cfg(test)]
#[path = "outside_tests.rs"]
mod tests;
