//! The first pass over an install: each program read to a part of the project, the parts merged
//! in one order whatever order they came in, and the project kept in one file beside the output.
//!
//! A part is what one program states and what its code shows, so programs are read independently
//! of each other and a thread holds only the one it reads.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};

use super::{
    layouts, reach, signatures, types, FieldFacts, ImportInfo, ProgramInfo, Project, ProjectError,
};
use crate::gta6::script::ir::{
    Callee, Expr, FunctionKey, Kind, Scalar, Stmt, StoreKind, TypeKey, Var,
};
use crate::gta6::script::listing::Listing;
use crate::gta6::script::opcode::Op;
use crate::gta6::script::program::{FunctionRow, Program};
use crate::gta6::script::{cfg, decode, lift, par, passes, Context, Names, Options, Pass, Stats};

/// The version a project file states; a file of another version is refused.
const VERSION: u32 = 1;

/// The threads files are read on at most: each holds one file whole, and a listing reaches 200 MB.
const WORKERS: usize = 8;

/// The `byte_offset`s under one access of a global that are added into its offset.
const NESTED: usize = 8;
/// The kinds one cell is noted with at most.
// More than three are stated only of a cell accessed at two widths, which is no cell whatever its kinds.
const KINDS: usize = 8;
/// The types the objects of one cell are called as, past which the cell holds a reference and no more is said.
const RECEIVERS: usize = 32;

#[derive(Serialize)]
struct Written<'a> {
    version: u32,
    project: &'a Project,
}

#[derive(Deserialize)]
struct Read {
    version: u32,
    project: Project,
}

/// What a project file of any version states.
#[derive(Deserialize)]
struct Stated {
    version: u32,
}

/// The part of a project one lifted listing gives.
// The text is held whole, since its declarations end it, and one function of it at a time.
pub fn of_listing(text: &str) -> Result<Project, ProjectError> {
    let listing = Listing::open(text)?;
    let imports = listing.imports.iter().map(|import| ImportInfo {
        program: import.program,
        first: import.types.start,
        end: import.types.end,
    });
    let info = ProgramInfo {
        program: listing.program,
        imports: imports.collect(),
        ..ProgramInfo::default()
    };
    let mut project = Project::default();
    project.programs.insert(info.program, info);

    let names = Names::default();
    let cx = Context::baseline(&names);
    let mut stats = Stats::default();
    let imported: Vec<u32> = listing
        .imports
        .iter()
        .map(|import| import.program)
        .collect();
    for read in listing.functions() {
        let read = read?;
        // The engine runs these by the header's word, with what it will.
        if matches!(read.head.name.as_str(), "main" | "initialize") {
            let engine = BTreeSet::from([FunctionKey::new(read.head.program, read.head.at)]);
            signatures::unseen(&mut project, listing.program, &[], Some(engine));
        }
        // A function the listing holds no body for shows nothing, and an empty body would be read as one that does nothing.
        // ⚠ Its code still runs and calls what it calls: nothing says which.
        if read.head.incomplete.is_some() {
            signatures::unseen(&mut project, listing.program, &imported, None);
            continue;
        }
        let function = read.head.name.clone();
        let lowered = cfg::lower(read);
        let mut f = lowered.map_err(|source| ProjectError::Lower { function, source })?;
        accessed(&mut project, &f);
        // ⚠ What a parameter's address reaches is read off the code as it is: a pass may leave out
        // a read no later code takes, which the code still makes.
        reach::of_function(&mut project, &f);
        let first = signatures::first_read(&f);
        // The facts are read off the code as the first two passes leave it: values whole, kinds known.
        passes::values::run_in(&mut f, &cx, &mut stats);
        passes::types::run_in(&mut f, &cx, &mut stats);
        gather(&mut project, &f, first);
    }
    Ok(project)
}

/// What the compiled programs of an install state of each other before any of their code is lifted.
// A function's code names the types and calls the functions of the programs it imports: its
// instructions decode only once those programs' tables are at hand.
#[derive(Clone, Debug, Default)]
pub struct Tables {
    /// The types and the function rows of every program, settled.
    project: Project,
    /// The offsets of each program's functions that another program calls or refers to.
    called: BTreeMap<u32, BTreeSet<u32>>,
}

impl Tables {
    /// Starts a function of `program` wherever another program calls one, as nothing of the program itself may show.
    fn start(&self, program: &mut Program) {
        for at in self.called.get(&program.program).into_iter().flatten() {
            // An offset that starts no instruction is a call into another build of the program: it states nothing of this one.
            let _ = program.add_function(*at);
        }
    }
}

/// The tables of the compiled programs among `bodies`; a body that is no program gives none.
pub fn tables<'a>(bodies: impl IntoIterator<Item = &'a [u8]> + Clone) -> Tables {
    let mut tables = Tables::default();
    let bodies: Vec<&[u8]> = bodies.into_iter().collect();
    let read = |bytes: &[u8]| Program::read(bytes).ok();
    let called = par::map(bodies.clone(), |bytes| {
        read(bytes).map(|p| p.called_elsewhere())
    });
    for (to, at) in called.into_iter().flatten().flatten() {
        tables.called.entry(to).or_default().insert(at);
    }
    // Each program is read again, so that the code of a thread's one is held at a time.
    let parts = par::map(bodies, |bytes| {
        let mut program = read(bytes)?;
        tables.start(&mut program);
        let mut part = Project::default();
        part.programs.insert(program.program, program.info());
        types::of_program(&mut part, &program);
        signatures::of_program(&mut part, &program);
        Some(part)
    });
    tables.project = merge(parts.into_iter().flatten().collect());
    tables
}

/// What one compiled program's code shows of the fields of the types a settled `project` proves its variables and receivers of: a part for [`refine`].
// ⚠ Read with the kinds the project gives: the first part of a program was read with none.
pub fn typed_part(
    bytes: &[u8],
    tables: &Tables,
    project: &Project,
) -> Result<Project, ProjectError> {
    let mut program = Program::read(bytes)?;
    tables.start(&mut program);
    let names = Names::default();
    let options = Options::baseline().with(Pass::Kinds).with(Pass::Interproc);
    let cx = Context::new(&names, project, options);
    let mut part = Project::default();
    // Lifted as the decompiler lifts the program: what one function settles of a slot sizes the rest.
    let lifted = lift::program_in(&program, project);
    // A function that faults to a handler is left out: no pass understands its way there.
    let mut whole =
        (lifted.into_iter().filter_map(Result::ok)).filter(|one| one.regions.is_empty());
    // A run of functions at a time: the passes of each on the caller's threads, the part told of
    // each in the order of the functions.
    loop {
        let run: Vec<lift::Lifted> = whole.by_ref().take(RUN).collect();
        if run.is_empty() {
            break;
        }
        let typed = crate::gta6::script::par::map(run, |lifted| {
            let mut f = lifted.function;
            let mut stats = Stats::default();
            passes::values::run_in(&mut f, &cx, &mut stats);
            passes::interproc::run(&mut f, &cx, &mut stats);
            passes::types::run_in(&mut f, &cx, &mut stats);
            f
        });
        for f in typed {
            layouts::of_typed_function(&mut part, &f, project);
        }
    }
    Ok(part)
}

/// Settles the fields of `project` again with what the parts [`typed_part`] read show.
pub fn refine(project: &mut Project, parts: Vec<Project>) {
    layouts::refine(project, parts);
}

/// The part of a project one compiled program gives, read alone: a function that names another program's type or function gives nothing.
pub fn of_program(bytes: &[u8]) -> Result<Project, ProjectError> {
    of_program_in(bytes, &Tables::default())
}

/// The part of a project one compiled program gives, with the tables of the programs it is installed with.
pub fn of_program_in(bytes: &[u8], tables: &Tables) -> Result<Project, ProjectError> {
    let mut program = Program::read(bytes)?;
    tables.start(&mut program);
    let mut project = Project::default();
    project.programs.insert(program.program, program.info());
    types::of_program(&mut project, &program);
    layouts::of_program(&mut project, &program);
    signatures::of_program(&mut project, &program);
    passes::classes::of_program(&mut project, &program);

    let names = Names::default();
    let cx = Context::baseline(&names);
    // Lifted as the decompiler lifts the program: what one function settles of a slot sizes the rest.
    let lifted = lift::program_in(&program, &tables.project);
    let mut functions = program.functions.iter().zip(lifted);
    let work = |lifted: Result<lift::Lifted, lift::LiftError>| {
        lifted.map(|lifted| {
            let before = lifted.function.clone();
            let mut f = lifted.function;
            let first = signatures::first_read(&f);
            // No pass understands a fault's way to its handler.
            if lifted.regions.is_empty() {
                let mut stats = Stats::default();
                passes::values::run_in(&mut f, &cx, &mut stats);
                passes::types::run_in(&mut f, &cx, &mut stats);
            }
            (before, f, first)
        })
    };
    // A run of functions at a time: the passes of each on the caller's threads while the project is
    // told of the run before, each in the order of the functions, as one after another would tell it.
    let mut read: Vec<(&FunctionRow, _)> = Vec::new();
    loop {
        let (rows, run): (Vec<&FunctionRow>, Vec<_>) = functions.by_ref().take(RUN).unzip();
        let last = run.is_empty();
        let ((), next) = crate::gta6::script::par::join(
            || {
                for (row, one) in std::mem::take(&mut read) {
                    told(&mut project, &program, tables, row, one);
                }
            },
            || crate::gta6::script::par::map(run, work),
        );
        if last {
            break;
        }
        read = rows.into_iter().zip(next).collect();
    }
    Ok(project)
}

/// The functions whose passes run at once before the project is told of them: each is held twice meanwhile.
const RUN: usize = 1024;

/// Tells `project` what one function of `program` shows: the code as lifted and after the passes, and what was first read of it; or, where it did not lift, what the tables state of it.
fn told(
    project: &mut Project,
    program: &Program,
    tables: &Tables,
    row: &FunctionRow,
    read: Result<(cfg::Function, cfg::Function, signatures::FirstRead), lift::LiftError>,
) {
    // A function that does not lift shows nothing: what the tables state of it stands, and the rest of the program is read.
    let Ok((before, f, first)) = read else {
        let named = named_by(program, &tables.project, row);
        let imported: Vec<u32> = program
            .imports
            .iter()
            .map(|import| import.program)
            .collect();
        let (functions, calls) = match named {
            Some(named) => (Some(named.functions), Some((named.slots, named.natives))),
            None => (None, None),
        };
        signatures::unseen(project, program.program, &imported, functions);
        signatures::unseen_calls(project, calls);
        return;
    };
    accessed(project, &before);
    // ⚠ What a parameter's address reaches is read off the code as it is: a pass may leave out
    // a read no later code takes, which the code still makes.
    reach::of_function(project, &before);
    gather(project, &f, first);
}

/// The part of a program whose own part is refused or faults: its code may call any function of it and of the programs it imports, through any slot, and any native; `None` where its bytes are no program.
pub fn unread_part(bytes: &[u8]) -> Option<Project> {
    let program = Program::read(bytes).ok()?;
    let mut project = Project::default();
    project.programs.insert(program.program, program.info());
    let imported: Vec<u32> = (program.imports.iter())
        .map(|import| import.program)
        .collect();
    signatures::unseen(&mut project, program.program, &imported, None);
    Some(project)
}

/// What the instructions of one function name: functions called or referred to, slots called through or taken, and natives called.
#[derive(Default)]
struct Named {
    functions: BTreeSet<FunctionKey>,
    slots: BTreeSet<u32>,
    natives: BTreeSet<u64>,
}

/// What the instructions of one function name; `None` where they do not decode.
fn named_by(program: &Program, tables: &Project, row: &FunctionRow) -> Option<Named> {
    let insns = decode::function_in(program, tables, row).ok()?;
    let mut named = Named::default();
    let callee_of = |named: &mut Named, callee: &Callee| {
        named
            .functions
            .extend(signatures::key_of(program.program, callee));
        if let Callee::Virtual { slot, .. } = callee {
            named.slots.insert(*slot);
        }
        named.natives.extend(signatures::native_key(callee));
    };
    for op in insns.iter().flat_map(|insn| &insn.ops) {
        match op {
            Op::Call { callee, .. } | Op::Unsized { callee, .. } => callee_of(&mut named, callee),
            Op::VirtualFunctionRef { slot, .. } => {
                named.slots.insert(*slot);
            }
            Op::Push { e, .. } => e.walk(&mut |e| match e {
                Expr::FunctionRef { program, at } => {
                    named.functions.insert(FunctionKey::new(*program, *at));
                }
                Expr::VirtualFunctionRef { slot, .. } => {
                    named.slots.insert(*slot);
                }
                _ => {}
            }),
            _ => {}
        }
    }
    Some(named)
}

/// Adds what one function's code shows to each table; `first` is what [`signatures::first_read`] read of it before any pass.
fn gather(project: &mut Project, f: &cfg::Function, first: signatures::FirstRead) {
    types::of_function(project, f);
    layouts::of_function(project, f);
    signatures::of_function_read(project, f, first);
    passes::classes::of_function(project, f);
    called(project, f);
}

/// One project of the parts, the same whatever order they are given in.
pub fn merge(parts: Vec<Project>) -> Project {
    let mut parts = parts;
    // A part may state facts of the programs it imports, so every program it holds is its key,
    // and two parts of the same programs are told apart by their bytes: where two parts state
    // one fact differently, the first of this order is kept, never the first given.
    parts.sort_by(|a, b| {
        let by_programs = a.programs.keys().cmp(b.programs.keys());
        by_programs.then_with(|| save(a).ok().cmp(&save(b).ok()))
    });
    let mut project = Project::default();
    for mut part in parts {
        // A program's row is there before a topic moves its own tables into it.
        for (program, info) in &mut part.programs {
            let row = project.programs.entry(*program).or_default();
            row.program = *program;
            if row.imports.is_empty() {
                row.imports = std::mem::take(&mut info.imports);
            }
            keep(&mut row.strings, std::mem::take(&mut info.strings));
        }
        types::absorb(&mut project, &mut part);
        layouts::absorb(&mut project, &mut part);
        signatures::absorb(&mut project, &mut part);
        reach::absorb(&mut project, &mut part);
        passes::classes::absorb(&mut project, &mut part);
        absorb_cells(&mut project, &mut part);
        // What a topic left in the part's rows is kept where the project's rows say nothing yet.
        for (program, info) in part.programs {
            let row = project.programs.entry(program).or_default();
            row.program = program;
            keep(&mut row.types, info.types);
            keep(&mut row.globals, info.globals);
            keep(&mut row.statics, info.statics);
        }
    }
    // Each table settles on those before it: types, then layouts, signatures and members.
    types::settle(&mut project);
    layouts::settle(&mut project);
    signatures::settle(&mut project);
    reach::settle(&mut project);
    types::settle_implemented(&mut project);
    passes::classes::settle(&mut project);
    settle_cells(&mut project);
    project
}

/// Adds the entries of `from` whose key `into` lacks.
fn keep<K: Ord, V>(into: &mut BTreeMap<K, V>, from: BTreeMap<K, V>) {
    for (key, value) in from {
        into.entry(key).or_insert(value);
    }
}

/// The part of each file named, in the order of `paths`, read on several threads.
pub fn parts(paths: &[PathBuf]) -> Vec<Result<Project, ProjectError>> {
    let workers = std::thread::available_parallelism().map_or(1, usize::from);
    read_on(workers.min(WORKERS), paths)
}

/// The bytes of a file that opens as a compiled program does, with a zero byte.
fn compiled(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut first = [0u8; 1];
    let mut file = std::fs::File::open(path).ok()?;
    file.read_exact(&mut first).ok()?;
    match first {
        [0] => std::fs::read(path).ok(),
        _ => None,
    }
}

/// [`parts`] on `workers` threads.
// The largest files are read first, so that none is left to run alone once the others are done.
fn read_on(workers: usize, paths: &[PathBuf]) -> Vec<Result<Project, ProjectError>> {
    // The compiled programs' tables first: every program's code is read against them.
    // ponytail: every compiled file is held at once meanwhile, 163 MB for one build's scripts; read
    // them one at a time, twice, when an install's scripts are too many to hold.
    let programs: Vec<Vec<u8>> = paths.iter().filter_map(|path| compiled(path)).collect();
    let tables = tables(programs.iter().map(Vec::as_slice));
    drop(programs);
    let tables = &tables;
    let mut order: Vec<(usize, &PathBuf)> = paths.iter().enumerate().collect();
    order.sort_by_cached_key(|(_, path)| {
        let size = std::fs::metadata(path).map_or(0, |file| file.len());
        Reverse(size)
    });
    let next = AtomicUsize::new(0);
    let work = || {
        let mut done = Vec::new();
        while let Some((n, path)) = order.get(next.fetch_add(1, Ordering::Relaxed)) {
            done.push((*n, of_file(path, tables)));
        }
        done
    };
    let mut done = match workers.min(paths.len()) {
        // No thread is started for one file, as where none can be.
        0 | 1 => work(),
        workers => std::thread::scope(|scope| {
            let started: Vec<_> = (0..workers).map(|_| scope.spawn(work)).collect();
            let joined = started.into_iter().flat_map(|worker| match worker.join() {
                Ok(done) => done,
                // A fault in a reader is the caller's to catch, as it is where no thread is started.
                Err(fault) => std::panic::resume_unwind(fault),
            });
            joined.collect()
        }),
    };
    done.sort_by_key(|(n, _)| *n);
    done.into_iter().map(|(_, part)| part).collect()
}

/// The part of one file: text is read as a lifted listing, any other file as a compiled program.
fn of_file(path: &Path, tables: &Tables) -> Result<Project, ProjectError> {
    let bytes = std::fs::read(path).map_err(|source| ProjectError::Io {
        path: path.display().to_string(),
        source,
    })?;
    match std::str::from_utf8(&bytes) {
        // A compiled program opens with zero bytes, which no listing does, and may be text by chance.
        Ok(text) if !text.starts_with('\0') => of_listing(text),
        _ => of_program_in(&bytes, tables),
    }
}

/// The project of the files named, each read as [`parts`] reads it; the first file refused ends it, named.
pub fn build(paths: &[PathBuf]) -> Result<Project, ProjectError> {
    let mut read = Vec::with_capacity(paths.len());
    for (path, part) in paths.iter().zip(parts(paths)) {
        read.push(part.map_err(|refused| match refused {
            ProjectError::Io { .. } => refused,
            // No other refusal says which file it is of.
            other => ProjectError::File {
                path: path.display().to_string(),
                source: Box::new(other),
            },
        })?);
    }
    Ok(merge(read))
}

/// What of `project` one program's decompilation reads: the programs it imports, however far, with their types and functions.
// A type's facts lead on to the program they were read from and to those that fill its slots.
// ⚠ What a program left out says of one kept is gone with it, a caller or a derived type among
// them: only what is asked starting from the program's own ids, functions and types is answered
// as the whole project answers it.
pub fn view(project: &Project, program: u32) -> Project {
    let mut programs = BTreeSet::from([program]);
    let mut types = BTreeSet::new();
    let mut pending = vec![program];
    // A program is pending once and a type followed once, so this ends.
    while let Some(next) = pending.pop() {
        let Some(info) = project.programs.get(&next) else {
            continue;
        };
        let mut reached: Vec<u32> = info.imports.iter().map(|import| import.program).collect();
        let mut named: Vec<TypeKey> = info.types.values().copied().collect();
        while let Some(key) = named.pop() {
            // A builtin type is the runtime's: the program its facts were read from is not reached through it.
            if key.builtin().is_some() || !types.insert(key) {
                continue;
            }
            let Some(facts) = project.types.get(&key) else {
                continue;
            };
            reached.push(facts.home);
            reached.extend(facts.slots.values().map(|function| function.program()));
            named.extend(facts.base);
            named.extend(&facts.interfaces);
            if let Some((of, arguments)) = &facts.generic {
                named.push(*of);
                named.extend(arguments);
            }
        }
        pending.extend(reached.into_iter().filter(|to| programs.insert(*to)));
    }
    let reads = |function: &FunctionKey| programs.contains(&function.program());
    let holds = |key: &TypeKey| types.contains(key) || key.builtin().is_some();
    Project {
        programs: cut(&project.programs, |of| programs.contains(of)),
        types: cut(&project.types, holds),
        fields: cut(&project.fields, holds),
        signatures: cut(&project.signatures, reads),
        calls: cut(&project.calls, reads),
        members: cut(&project.members, reads),
        slot_results: cut(&project.slot_results, reads),
        slot_params: cut(&project.slot_params, reads),
        reach: cut(&project.reach, reads),
        widened: cut(&project.widened, reads),
        native_truths: project.native_truths.clone(),
        native_uses: BTreeMap::new(),
        implemented: BTreeMap::new(),
        cells: Cells::default(),
        truths: BTreeMap::new(),
        rows: Default::default(),
        unseen: BTreeSet::new(),
        unseen_slots: BTreeSet::new(),
        blind_natives: false,
    }
}

/// The entries of `map` whose key `keep` takes.
fn cut<K: Ord + Copy, V: Clone>(map: &BTreeMap<K, V>, keep: impl Fn(&K) -> bool) -> BTreeMap<K, V> {
    let kept = map.iter().filter(|(key, _)| keep(*key));
    kept.map(|(key, value)| (*key, value.clone())).collect()
}

/// A global or a static: its program, whether it is a static, and its byte offset.
type Place = (u32, bool, u32);

/// What the accesses of one global or static show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Shown {
    /// Each width in bytes an access took.
    widths: BTreeSet<u32>,
    /// Each kind an access stated, once; none is `Unknown`, and none an object of a script type.
    kinds: Vec<Kind>,
    /// The script types the cell's object was called as.
    called: BTreeSet<TypeKey>,
}

impl Shown {
    fn note(&mut self, bytes: u32, kind: Kind) {
        self.widths.insert(bytes);
        let script = match &kind {
            Kind::Object(name) => TypeKey::of_name(name).filter(|key| key.builtin().is_none()),
            _ => None,
        };
        if let Some(key) = script {
            self.called.insert(key);
            return;
        }
        let new = kind != Kind::Unknown && !self.kinds.contains(&kind);
        if new && self.kinds.len() < KINDS {
            self.kinds.push(kind);
        }
    }
}

/// What the accesses of globals and statics show, until [`merge`] makes one cell an offset of them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cells {
    shown: BTreeMap<Place, Shown>,
}

impl Cells {
    /// Notes an access of `bytes` bytes at `place` that states `kind` of them.
    fn note(&mut self, place: Place, bytes: u32, kind: Kind) {
        if bytes > 0 {
            self.shown.entry(place).or_default().note(bytes, kind);
        }
    }
}

/// The global or static a variable is.
fn global(var: Var) -> Option<Place> {
    match var {
        Var::Global {
            program,
            offset,
            statics,
        } => Some((program, statics, offset)),
        _ => None,
    }
}

/// Where an access at `offset` from `addr` starts, where `addr` is the address of a global or a static moved by constants.
fn place(addr: &Expr, offset: u32) -> Option<Place> {
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
    let Expr::AddrOf { var, cell } = addr else {
        return None;
    };
    let (program, statics, base) = global(*var)?;
    let within = cell.checked_mul(8)?.checked_add(offset)?;
    Some((program, statics, base.checked_add(within)?))
}

/// The kind a read under `as_T` or `load_T` states of what it reads.
// ⚠ Four bytes and eight are read as they lie: neither read says a sign.
fn read_as(ty: Scalar) -> Kind {
    match ty {
        Scalar::I32 | Scalar::U32 | Scalar::I64 | Scalar::U64 => Kind::Unknown,
        stated => stated.kind(),
    }
}

/// The kind several bytes read or written whole are: a value held in place where they are more than a cell.
fn whole(bytes: u32) -> Kind {
    match bytes > 8 {
        true => Kind::Aggregate(bytes.div_ceil(8)),
        false => Kind::Unknown,
    }
}

/// One reading of a function for the widths its accesses of globals and statics take.
#[derive(Default)]
struct Accesses<'a> {
    rows: Vec<(Place, u32, Kind)>,
    /// The read of a cell that the cast around it has stated the width of.
    cast: Option<&'a Expr>,
}

impl<'a> Accesses<'a> {
    /// Notes an access of `bytes` bytes at `offset` from `addr`, where that is a global's address or a static's.
    fn at(&mut self, addr: &Expr, offset: u32, bytes: u32, kind: Kind) {
        if let Some(place) = place(addr, offset) {
            self.rows.push((place, bytes, kind));
        }
    }

    /// Notes a store to `target` by name: of the bytes its value is cast to, and of a whole cell where it is cast to none.
    fn set(&mut self, target: Var, value: Option<&Expr>) {
        let Some(place) = global(target) else {
            return;
        };
        let bytes = match value {
            Some(Expr::Cast { to, .. }) => to.bytes(),
            _ => 8,
        };
        self.rows.push((place, bytes, Kind::Unknown));
    }

    fn stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::Assign { target, value } => self.set(*target, Some(value)),
            Stmt::AssignMany { targets, values } => {
                // A value of several cells goes to its targets a whole cell each.
                let paired = targets.len() == values.len();
                for (n, target) in targets.iter().enumerate() {
                    self.set(*target, values.get(n).filter(|_| paired));
                }
            }
            Stmt::Store {
                kind, addr, offset, ..
            } => {
                let held = match kind {
                    StoreKind::Ref => Kind::Pointer,
                    _ => Kind::Unknown,
                };
                self.at(addr, *offset, kind.bytes(), held);
            }
            Stmt::StoreBytes {
                addr,
                offset,
                bytes,
                ..
            } => self.at(addr, *offset, *bytes, whole(*bytes)),
            Stmt::ZeroBytes { addr, bytes } => self.at(addr, 0, *bytes, whole(*bytes)),
            Stmt::StepI32 { addr, offset, .. } => self.at(addr, *offset, 4, Kind::Unknown),
            // Floats one after another, as many as the value has cells: no width is stated.
            Stmt::StoreF32Slots { .. } | Stmt::Eval(_) => {}
        }
        stmt.each_expr(&mut |top| self.expr(top));
    }

    fn expr(&mut self, top: &'a Expr) {
        top.walk(&mut |e| match e {
            Expr::Cast { to, e: inner } => {
                if let Expr::Var(var) = &**inner {
                    if let Some(place) = global(*var) {
                        self.rows.push((place, to.bytes(), read_as(*to)));
                        self.cast = Some(&**inner);
                    }
                }
            }
            // A cell read under no cast is read whole.
            Expr::Var(var) => {
                let stated = self.cast.take().is_some_and(|read| std::ptr::eq(read, e));
                if let (false, Some(place)) = (stated, global(*var)) {
                    self.rows.push((place, 8, Kind::Unknown));
                }
            }
            Expr::Load { ty, addr, offset } => self.at(addr, *offset, ty.bytes(), read_as(*ty)),
            Expr::LoadBytes {
                addr,
                offset,
                bytes,
            } => self.at(addr, *offset, *bytes, whole(*bytes)),
            _ => {}
        });
    }
}

/// Adds the width each access of a global or a static in one function takes, as the front end wrote the access.
// ⚠ Before any pass: an access is as wide as the cast it stands under, and a pass takes a cast
// off wherever what is around it says as much.
fn accessed(project: &mut Project, f: &cfg::Function) {
    let mut seen = Accesses::default();
    for block in &f.cfg.blocks {
        for stmt in &block.stmts {
            seen.stmt(stmt);
        }
        if let Some(e) = block.term.expr() {
            seen.expr(e);
        }
    }
    for (place, bytes, kind) in seen.rows {
        project.cells.note(place, bytes, kind);
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

/// The global or static `e` reads a whole cell of.
fn read_whole(e: &Expr) -> Option<Place> {
    match e {
        Expr::Var(var) => global(*var),
        Expr::Cast {
            to: Scalar::I64 | Scalar::U64,
            e: inner,
        } => match &**inner {
            Expr::Var(var) => global(*var),
            _ => None,
        },
        _ => None,
    }
}

/// Adds the globals and statics one function goes through as objects, once its values are whole: those it calls through, and those it stores a new object to.
// ⚠ The type a call goes through is the one the source gives the cell or a type above that one,
// never the type of the object itself: `settle_cells` keeps the most derived of those seen. What
// is made into a cell says only that it holds a reference, since its type may be a base's.
fn called(project: &mut Project, f: &cfg::Function) {
    let mut rows: Vec<(Place, Kind)> = Vec::new();
    for stmt in f.cfg.blocks.iter().flat_map(|block| &block.stmts) {
        if let Stmt::Assign {
            target,
            value: Expr::NewObject(_) | Expr::NewArray { .. },
        } = stmt
        {
            rows.extend(global(*target).map(|place| (place, Kind::Pointer)));
        }
    }
    let mut visit = |e: &Expr| {
        let Expr::Call { callee, args, .. } = e else {
            return;
        };
        let read = args.first().and_then(read_whole);
        let Some(place) = read.filter(|_| through_first(callee)) else {
            return;
        };
        rows.push((place, Kind::Pointer));
        if let Callee::Virtual { ty, .. } = callee {
            // A builtin's slot says nothing of a script type.
            let script = TypeKey::of_name(ty).filter(|key| key.builtin().is_none());
            rows.extend(script.map(|key| (place, Kind::Object(key.to_string()))));
        }
    };
    for block in &f.cfg.blocks {
        for stmt in &block.stmts {
            stmt.each_expr(&mut |top| top.walk(&mut visit));
        }
        if let Some(e) = block.term.expr() {
            e.walk(&mut visit);
        }
    }
    for (place, kind) in rows {
        project.cells.note(place, 8, kind);
    }
}

/// Moves what the accesses of globals and statics show from `from` into `into`: a cell both hold has what both show, each once.
fn absorb_cells(into: &mut Project, from: &mut Project) {
    for (place, shown) in std::mem::take(&mut from.cells.shown) {
        let held = into.cells.shown.entry(place).or_default();
        held.widths.extend(shown.widths);
        held.called.extend(shown.called);
        for kind in shown.kinds {
            if !held.kinds.contains(&kind) && held.kinds.len() < KINDS {
                held.kinds.push(kind);
            }
        }
    }
}

/// The bytes a cell of this kind takes, for the kinds that say.
fn width(kind: &Kind) -> Option<u32> {
    match kind {
        Kind::I8 | Kind::U8 => Some(1),
        Kind::I16 | Kind::U16 => Some(2),
        Kind::I32 | Kind::U32 | Kind::F32 | Kind::Hash => Some(4),
        Kind::I64 | Kind::U64 | Kind::F64 => Some(8),
        Kind::Pointer | Kind::Object(_) | Kind::String | Kind::Array(_) => Some(8),
        _ => None,
    }
}

/// What two kinds stated of one cell say together; `None` where they disagree.
fn both(a: Kind, b: &Kind) -> Option<Kind> {
    let named = |kind: &Kind| matches!(kind, Kind::Object(_) | Kind::String | Kind::Array(_));
    match (a, b) {
        (a, b) if a == *b => Some(a),
        (Kind::Unknown, b) => Some(b.clone()),
        (a, Kind::Unknown) => Some(a),
        // A reference, and what one statement says it refers to.
        (Kind::Pointer, b) if named(b) => Some(b.clone()),
        (a, Kind::Pointer) if named(&a) => Some(a),
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

/// The one cell the accesses of an offset make: none where they took two widths, and of no kind where two kinds were stated.
// A reference whose objects are called as script types is of the most derived of them, and
// where they are of no one line it is a reference and no more.
fn agreed(project: &Project, offset: u32, shown: &Shown) -> Option<FieldFacts> {
    let mut widths = shown.widths.iter();
    let (Some(bytes), None) = (widths.next(), widths.next()) else {
        return None;
    };
    let mut kind = Some(Kind::Unknown);
    for said in &shown.kinds {
        kind = kind.and_then(|known| both(known, said));
    }
    if !shown.called.is_empty() {
        kind = kind.and_then(|known| both(known, &Kind::Pointer));
    }
    let kind = match (kind, shown.called.is_empty()) {
        (Some(kind), true) => kind,
        (Some(Kind::Pointer), false) => match narrowest(project, &shown.called) {
            Some(key) => Kind::Object(key.to_string()),
            None => Kind::Pointer,
        },
        // A string or an array by one statement and an object of a script type by another.
        (Some(_), false) => Kind::Pointer,
        (None, _) => Kind::Unknown,
    };
    Some(FieldFacts {
        offset,
        bytes: *bytes,
        kind,
    })
}

/// Once every part is in and the types are settled: one cell an offset of each program's globals and of its statics, by rising offset.
// ⚠ An offset accessed at two widths has no cell: each access of it keeps the form it has.
// ⚠ `ProgramInfo::globals` and `statics` are what a pass reads as proved of every value a cell
// holds, so of a cell only that it holds a reference, or an object of the type its objects are
// called as, is stated there, and never over what a part stated of it otherwise.
fn settle_cells(project: &mut Project) {
    let mut areas: BTreeMap<(u32, bool), BTreeMap<u32, Shown>> = BTreeMap::new();
    for ((program, statics, offset), shown) in std::mem::take(&mut project.cells.shown) {
        let area = areas.entry((program, statics)).or_default();
        area.insert(offset, shown);
    }
    // What a part states of a cell outright, a container's map of its references among it, is one more access.
    for (program, info) in &project.programs {
        for (statics, stated) in [(false, &info.globals), (true, &info.statics)] {
            for (offset, kind) in stated {
                if let Some(bytes) = width(kind) {
                    let area = areas.entry((*program, statics)).or_default();
                    area.entry(*offset).or_default().note(bytes, kind.clone());
                }
            }
        }
    }
    let known: &Project = project;
    let mut settled = Vec::new();
    for ((program, statics), offsets) in &areas {
        // A program the project does not hold has no row to state its cells in.
        if !known.programs.contains_key(program) {
            continue;
        }
        let cells = offsets.iter();
        let cells: Vec<FieldFacts> = cells
            .filter_map(|(offset, shown)| agreed(known, *offset, shown))
            .collect();
        settled.push((*program, *statics, cells));
    }
    for (program, statics, cells) in settled {
        let Some(info) = project.programs.get_mut(&program) else {
            continue;
        };
        let (stated, held) = match statics {
            true => (&mut info.statics, &mut info.static_cells),
            false => (&mut info.globals, &mut info.global_cells),
        };
        for cell in &cells {
            let refers = matches!(cell.kind, Kind::Pointer | Kind::Object(_));
            let open = matches!(stated.get(&cell.offset), None | Some(Kind::Pointer));
            if refers && open {
                stated.insert(cell.offset, cell.kind.clone());
            }
        }
        *held = cells;
    }
}

/// The bytes of the file a project is kept in.
// ponytail: plain JSON, the whole project in memory twice while it is written; deflate it (`flate2`
// is here) or write table by table when an install's file is too large to keep or to load.
pub fn save(project: &Project) -> Result<Vec<u8>, ProjectError> {
    let written = Written {
        version: VERSION,
        project,
    };
    serde_json::to_vec(&written).map_err(|e| ProjectError::Sidecar(e.to_string()))
}

/// Reads a project back from the bytes [`save`] wrote.
pub fn load(bytes: &[u8]) -> Result<Project, ProjectError> {
    let other = |version: u32| {
        let why = format!("it is of version {version} and version {VERSION} is read");
        ProjectError::Sidecar(why)
    };
    match serde_json::from_slice::<Read>(bytes) {
        Ok(read) if read.version == VERSION => Ok(read.project),
        Ok(read) => Err(other(read.version)),
        // A file of another version is refused for its version, not for the shape of its tables.
        Err(unread) => Err(match serde_json::from_slice::<Stated>(bytes) {
            Ok(stated) if stated.version != VERSION => other(stated.version),
            _ => ProjectError::Sidecar(unread.to_string()),
        }),
    }
}

#[cfg(test)]
#[path = "index_tests.rs"]
mod tests;
