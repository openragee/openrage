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

use super::{layouts, signatures, types, ImportInfo, ProgramInfo, Project, ProjectError};
use crate::gta6::script::ir::{FunctionKey, TypeKey};
use crate::gta6::script::listing::Listing;
use crate::gta6::script::program::Program;
use crate::gta6::script::{cfg, lift, passes, Context, Names, Stats};

/// The version a project file states; a file of another version is refused.
const VERSION: u32 = 1;

/// The threads files are read on at most: each holds one file whole, and a listing reaches 200 MB.
const WORKERS: usize = 8;

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
    for read in listing.functions() {
        let read = read?;
        // A function the listing holds no body for shows nothing, and an empty body would be read as one that does nothing.
        if read.head.incomplete.is_some() {
            continue;
        }
        let function = read.head.name.clone();
        let lowered = cfg::lower(read);
        let mut f = lowered.map_err(|source| ProjectError::Lower { function, source })?;
        // The facts are read off the code as the first two passes leave it: values whole, kinds known.
        passes::values::run_in(&mut f, &cx, &mut stats);
        passes::types::run_in(&mut f, &cx, &mut stats);
        gather(&mut project, &f);
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
    let read = |bytes: &[u8]| Program::read(bytes).ok();
    for program in bodies.clone().into_iter().filter_map(read) {
        for (to, at) in program.called_elsewhere() {
            tables.called.entry(to).or_default().insert(at);
        }
    }
    // Each program is read again, so that the code of one is held at a time.
    let mut parts = Vec::new();
    for mut program in bodies.into_iter().filter_map(read) {
        tables.start(&mut program);
        let mut part = Project::default();
        part.programs.insert(program.program, program.info());
        types::of_program(&mut part, &program);
        signatures::of_program(&mut part, &program);
        parts.push(part);
    }
    tables.project = merge(parts);
    tables
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

    let names = Names::default();
    let cx = Context::baseline(&names);
    let mut stats = Stats::default();
    for row in &program.functions {
        // A function that does not lift shows nothing: what the tables state of it stands, and the rest of the program is read.
        let Ok(lifted) = lift::function_in(&program, &tables.project, row) else {
            continue;
        };
        let mut f = lifted.function;
        // No pass understands a fault's way to its handler.
        if lifted.regions.is_empty() {
            passes::values::run_in(&mut f, &cx, &mut stats);
            passes::types::run_in(&mut f, &cx, &mut stats);
        }
        gather(&mut project, &f);
    }
    Ok(project)
}

/// Adds what one function's code shows to each table.
fn gather(project: &mut Project, f: &cfg::Function) {
    types::of_function(project, f);
    layouts::of_function(project, f);
    signatures::of_function(project, f);
    passes::classes::of_function(project, f);
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
        passes::classes::absorb(&mut project, &mut part);
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
    types::settle_implemented(&mut project);
    passes::classes::settle(&mut project);
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
        implemented: BTreeMap::new(),
    }
}

/// The entries of `map` whose key `keep` takes.
fn cut<K: Ord + Copy, V: Clone>(map: &BTreeMap<K, V>, keep: impl Fn(&K) -> bool) -> BTreeMap<K, V> {
    let kept = map.iter().filter(|(key, _)| keep(*key));
    kept.map(|(key, value)| (*key, value.clone())).collect()
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
