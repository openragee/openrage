//! `script names-search` and `script natives --unnamed`: what the names of compiled GTA VI
//! programs a word search proves, and which unnamed natives a list would name the most calls of.
//!
//! ⚠ **A search names nothing by 32 bits.** A type, a field and a program state two hashes of one
//! name, and a join of words is kept only where both are its own; the list written proves each
//! of its rows again when it is read.
//!
//! ⚠ **No native is named here.** A native's hash is no function of its name: the report says
//! where a list would pay most, and how its calls use their arguments.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};

use or_io::gta6::script::cfg::Terminator;
use or_io::gta6::script::ir::{Callee, Const, Expr, Kind};
use or_io::gta6::script::lift;
use or_io::gta6::script::names::search::{self, Budget, Search, Targets};
use or_io::gta6::script::names::words;
use or_io::gta6::script::program::Program;
use or_io::gta6::script::{Names, Project};
use serde_json::json;

/// The rounds a search runs where none is asked for: each feeds the words of what the last found to the next.
const ROUNDS: usize = 3;

/// The threads a search takes where none is asked for.
const THREADS: usize = 8;

/// The unnamed natives a report counts the calls of, as a share of all its unnamed calls.
const TOP: usize = 200;

const USAGE: &str = "usage: openrage-cli script names-search <program|folder>... [--words <file>]... \
     [--names <file>]... [--join <n>] [--candidates <n>] [--rounds <n>] [--threads <n>] [-o <file>]\n       \
     openrage-cli script natives --unnamed <program|folder>... [--names <file>]... \
     [--natives-db <file>]... [--project <file>] [--threads <n>] [--json] [-o <file>]";

/// What the arguments after the verb ask.
#[derive(Default)]
struct Asked {
    paths: Vec<PathBuf>,
    words: Vec<PathBuf>,
    names: Vec<PathBuf>,
    natives: Vec<PathBuf>,
    project: Option<PathBuf>,
    out: Option<PathBuf>,
    join: usize,
    candidates: u64,
    rounds: usize,
    threads: usize,
    json: bool,
    unnamed: bool,
}

/// Runs the verb where the arguments name one of the two; `None` leaves them to the rest of `script`.
pub fn run(args: &[String]) -> Option<ExitCode> {
    let verb = args.first().map(String::as_str)?;
    if verb != "names-search" && !(verb == "natives" && args.iter().any(|a| a == "--unnamed")) {
        return None;
    }
    let asked = match asked(&args[1..]) {
        Ok(asked) => asked,
        Err(wrong) => return Some(super::super::misused(&format!("{wrong}\n{USAGE}"))),
    };
    if asked.paths.is_empty() {
        return Some(super::super::misused(USAGE));
    }
    let done = match verb {
        "names-search" => names_search(&asked),
        _ => unnamed(&asked),
    };
    let put = done.and_then(|text| match &asked.out {
        Some(to) => std::fs::write(to, text).map_err(|e| format!("write {}: {e}", to.display())),
        None => super::super::put(&text).map_err(|e| format!("write to stdout: {e}")),
    });
    Some(match put {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => super::super::failed(&e),
    })
}

fn asked(args: &[String]) -> Result<Asked, String> {
    let mut asked = Asked {
        join: 2,
        candidates: 1 << 32,
        rounds: ROUNDS,
        threads: THREADS,
        ..Asked::default()
    };
    let mut at = args.iter();
    while let Some(arg) = at.next() {
        let mut value = |flag: &str| {
            at.next()
                .cloned()
                .ok_or_else(|| format!("{flag} takes a value"))
        };
        let number = |flag: &str, text: String| {
            text.parse::<u64>()
                .map_err(|_| format!("{flag} takes a number, not `{text}`"))
        };
        match arg.as_str() {
            "--words" => asked.words.push(PathBuf::from(value(arg)?)),
            "--names" => asked.names.push(PathBuf::from(value(arg)?)),
            "--natives-db" => asked.natives.push(PathBuf::from(value(arg)?)),
            "--project" => asked.project = Some(PathBuf::from(value(arg)?)),
            "-o" | "--out" => asked.out = Some(PathBuf::from(value(arg)?)),
            "--join" => asked.join = number(arg, value(arg)?)? as usize,
            "--candidates" => asked.candidates = number(arg, value(arg)?)?,
            "--rounds" => asked.rounds = number(arg, value(arg)?)? as usize,
            "--threads" => asked.threads = (number(arg, value(arg)?)? as usize).max(1),
            "--json" => asked.json = true,
            "--unnamed" => asked.unnamed = true,
            flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
            path => asked.paths.push(PathBuf::from(path)),
        }
    }
    Ok(asked)
}

/// Every program the paths name, a folder's at any depth, with the file it was read from; a file that is no program is counted and passed over.
fn programs(paths: &[PathBuf]) -> (Vec<(PathBuf, Program)>, usize) {
    let mut files = Vec::new();
    for path in paths {
        collect(path, &mut files);
    }
    files.sort();
    let (mut read, mut refused) = (Vec::new(), 0);
    for file in files {
        match std::fs::read(&file).map(|bytes| Program::read(&bytes)) {
            Ok(Ok(program)) => read.push((file, program)),
            _ => refused += 1,
        }
    }
    (read, refused)
}

fn collect(path: &Path, files: &mut Vec<PathBuf>) {
    if path.is_dir() {
        for entry in std::fs::read_dir(path).into_iter().flatten().flatten() {
            collect(&entry.path(), files);
        }
    } else if path.is_file() {
        files.push(path.to_path_buf());
    }
}

/// The 64-bit hash of a program's file where its name states it, `hash_` and sixteen hex digits.
fn file_hash(file: &Path) -> Option<u64> {
    let stem = file.file_name()?.to_str()?;
    let hex = stem.strip_prefix("hash_")?;
    (hex.len() == 16)
        .then(|| u64::from_str_radix(hex, 16).ok())
        .flatten()
}

/// The words of the word files: each line that holds one word.
fn word_files(paths: &[PathBuf]) -> Result<Vec<String>, String> {
    let mut held = Vec::new();
    for path in paths {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        for line in text.lines().map(str::trim) {
            if !line.is_empty() && !line.starts_with('#') && !line.contains(char::is_whitespace) {
                held.push(line.to_owned());
            }
        }
    }
    Ok(held)
}

fn names_search(asked: &Asked) -> Result<String, String> {
    let supplied = super::lists::supplied(&asked.names, &[], None)?;
    let names = supplied.names;
    let (read, refused) = programs(&asked.paths);
    let mut targets = Targets::default();
    for (file, program) in &read {
        targets.of_program(program, &names);
        if let Some(file) = file_hash(file) {
            let hash = u64::from(program.program);
            if names.program(hash).is_none() {
                targets.program(program.program, file);
            }
        }
    }
    let mut lexicon: Vec<String> = word_files(&asked.words)?;
    lexicon.extend(words::bundled().map(str::to_owned));
    let mut held: BTreeSet<String> = lexicon.iter().map(|w| w.to_ascii_lowercase()).collect();
    let budget = Budget {
        join: asked.join,
        candidates: asked.candidates,
    };
    let mut all = Search {
        complete: true,
        ..Search::default()
    };
    let mut notes = Vec::new();
    for round in 1..=asked.rounds.max(1) {
        let found = search::search_threads(
            lexicon.iter().map(String::as_str),
            &targets,
            budget,
            asked.threads,
        );
        let before = all.found.len();
        all.tried += found.tried;
        all.complete &= found.complete;
        let learned: Vec<String> = (found.found.iter())
            .flat_map(|f| search::words_of(&f.name))
            .filter(|w| held.insert(w.to_ascii_lowercase()))
            .collect();
        all.found.extend(found.found);
        all.found.sort();
        all.found.dedup();
        notes.push(format!(
            "round {round}: {} word(s), {} join(s) tried, {} name(s) new, {} word(s) learned",
            lexicon.len(),
            found.tried,
            all.found.len() - before,
            learned.len()
        ));
        if learned.is_empty() {
            break;
        }
        lexicon.extend(learned);
    }
    // The generic types of the standard library, filled with every type's name a list or the
    // rounds gave.
    let mut typed: BTreeSet<String> = (all.found.iter())
        .filter(|f| f.kind == search::Target::Member)
        .map(|f| f.name.clone())
        .collect();
    for (_, program) in &read {
        for ty in &program.types {
            typed.extend(names.type_name(ty.name).map(str::to_owned));
        }
    }
    let generics = search::composed(typed.iter().map(String::as_str), &targets);
    notes.push(format!(
        "generic types: {} name(s) tried, {} proved",
        generics.tried,
        generics.found.len()
    ));
    all.tried += generics.tried;
    all.found.extend(generics.found);
    all.found.sort();
    all.found.dedup();
    for note in &notes {
        super::super::note(note);
    }
    super::super::note(&format!(
        "{} program(s) read, {refused} file(s) passed over; {} pair(s) of hashes sought; {} name(s) proved by both hashes{}",
        read.len(),
        targets.len(),
        all.found.len(),
        if all.complete { "" } else { "; the budget ended a round before its last length" }
    ));
    Ok(all.list())
}

/// What the calls of one unnamed native are.
#[derive(Default)]
struct Calls {
    rva: u32,
    args: u64,
    returns: u64,
    sites: usize,
    programs: BTreeSet<u32>,
    /// By argument place: the count of each kind of argument there.
    kinds: BTreeMap<usize, BTreeMap<&'static str, usize>>,
}

/// The kind of an argument as a report counts it.
fn kind_of(e: &Expr) -> &'static str {
    match e {
        Expr::Const(c) => const_kind(c),
        Expr::Cast { e, .. } | Expr::Sugar { e, .. } => kind_of(e),
        Expr::Call {
            callee: Callee::Native { .. },
            ..
        } => "native result",
        Expr::Call { .. } => "call result",
        Expr::Var(_) => "variable",
        _ => "expression",
    }
}

fn const_kind(c: &Const) -> &'static str {
    if c.kind == Kind::Hash {
        return "hash";
    }
    if c.kind.is_float() {
        return "float";
    }
    let low = c.bits as u32;
    // A small value, or one widened from a small negative.
    if !(0x1_0000..0xFFFF_0000).contains(&low) {
        "small int"
    } else {
        "large int"
    }
}

/// What one program's calls of unnamed natives are, with its count of named calls, of unnamed ones and of functions that did not lift.
fn tally(program: &Program, project: &Project, names: &Names) -> Tally {
    let mut t = Tally::default();
    for row in &program.functions {
        let Ok(lifted) = lift::function_in(program, project, row) else {
            t.failed += 1;
            continue;
        };
        let mut seen = |e: &Expr| {
            let Expr::Call {
                callee: Callee::Native { rva, .. },
                args,
                returns,
            } = e
            else {
                return;
            };
            let hash = names.native_at(*rva);
            if hash.is_some_and(|h| names.native(h).is_some()) {
                t.named += 1;
                return;
            }
            t.unnamed += 1;
            // A handler no list gives a hash of is kept by its address, under the top bit.
            let key = hash.unwrap_or(u64::from(*rva) | 1 << 63);
            let calls = t.by.entry(key).or_default();
            calls.rva = *rva;
            calls.args = args.len() as u64;
            calls.returns = u64::from(returns.unwrap_or(0));
            calls.sites += 1;
            calls.programs.insert(program.program);
            for (n, arg) in args.iter().enumerate() {
                let kinds = calls.kinds.entry(n).or_default();
                *kinds.entry(kind_of(arg)).or_default() += 1;
            }
        };
        for block in &lifted.function.cfg.blocks {
            for stmt in &block.stmts {
                stmt.each_expr(&mut |e: &Expr| e.walk(&mut seen));
            }
            match &block.term {
                Terminator::Branch { cond: e, .. }
                | Terminator::Switch { value: e, .. }
                | Terminator::Return(Some(e)) => e.walk(&mut seen),
                _ => {}
            }
        }
    }
    t
}

#[derive(Default)]
struct Tally {
    by: BTreeMap<u64, Calls>,
    named: usize,
    unnamed: usize,
    failed: usize,
}

fn unnamed(asked: &Asked) -> Result<String, String> {
    let supplied = super::lists::supplied(&asked.names, &asked.natives, None)?;
    let names = supplied.names;
    let loaded = match &asked.project {
        Some(path) => Some(super::project_at(path)?.1),
        None => None,
    };
    let project = loaded.as_ref().unwrap_or(Project::empty());
    let (read, refused) = programs(&asked.paths);
    let next = AtomicUsize::new(0);
    let tallies: Vec<Tally> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..asked.threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let n = next.fetch_add(1, Ordering::Relaxed);
                        let Some((_, program)) = read.get(n) else {
                            break;
                        };
                        mine.push(tally(program, project, &names));
                    }
                    mine
                })
            })
            .collect();
        // A thread that panicked carries its panic on.
        (handles.into_iter())
            .flat_map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
            .collect()
    });
    let mut by: BTreeMap<u64, Calls> = BTreeMap::new();
    let (mut named_calls, mut unnamed_calls, mut failed) = (0usize, 0usize, 0usize);
    for t in tallies {
        named_calls += t.named;
        unnamed_calls += t.unnamed;
        failed += t.failed;
        for (key, c) in t.by {
            let held = by.entry(key).or_default();
            held.rva = c.rva;
            held.args = c.args;
            held.returns = c.returns;
            held.sites += c.sites;
            held.programs.extend(c.programs);
            for (n, kinds) in c.kinds {
                let place = held.kinds.entry(n).or_default();
                for (kind, count) in kinds {
                    *place.entry(kind).or_default() += count;
                }
            }
        }
    }
    let mut rows: Vec<(&u64, &Calls)> = by.iter().collect();
    rows.sort_by(|a, b| b.1.sites.cmp(&a.1.sites).then(a.0.cmp(b.0)));
    let top: usize = rows.iter().take(TOP).map(|(_, c)| c.sites).sum();
    let share = |n: usize| {
        if unnamed_calls == 0 {
            0.0
        } else {
            100.0 * n as f64 / unnamed_calls as f64
        }
    };
    let hash_text = |key: u64| match key >> 63 {
        1 => String::new(),
        _ => format!("0x{key:016X}"),
    };
    if asked.json {
        let list: Vec<_> = rows
            .iter()
            .map(|(key, c)| {
                json!({
                    "hash": hash_text(**key),
                    "rva": format!("0x{:08X}", c.rva),
                    "args": c.args,
                    "returns": c.returns,
                    "calls": c.sites,
                    "programs": c.programs.len(),
                    "kinds": c.kinds.iter().map(|(n, k)| (n.to_string(), json!(k))).collect::<serde_json::Map<_, _>>(),
                })
            })
            .collect();
        let report = json!({
            "programs": read.len(),
            "calls_named": named_calls,
            "calls_unnamed": unnamed_calls,
            "top": TOP,
            "top_share_percent": share(top),
            "natives": list,
        });
        return Ok(format!("{report:#}\n"));
    }
    let mut out = String::new();
    for (key, c) in &rows {
        let kinds: Vec<String> = (c.kinds.iter())
            .map(|(n, k)| {
                let k: Vec<String> = k
                    .iter()
                    .map(|(kind, count)| format!("{kind} {count}"))
                    .collect();
                format!("{n}: {}", k.join(", "))
            })
            .collect();
        let _ = writeln!(
            out,
            "{:<18}  rva_{:08X}  {} -> {}  {} call(s) in {} program(s)  {}",
            hash_text(**key),
            c.rva,
            c.args,
            c.returns,
            c.sites,
            c.programs.len(),
            kinds.join(" | ")
        );
    }
    let _ = writeln!(
        out,
        "\n{} unnamed native(s), {unnamed_calls} call(s) of them and {named_calls} of named ones, in {} program(s) \
         ({refused} file(s) passed over, {failed} function(s) not lifted); the {} called most take {top} of \
         the unnamed calls ({:.1}%).",
        rows.len(),
        read.len(),
        TOP.min(rows.len()),
        share(top)
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn only_the_two_verbs_are_taken() {
        assert!(run(&args("decompile prog")).is_none());
        assert!(
            run(&args("natives prog")).is_none(),
            "natives without --unnamed is a path"
        );
        let line = "a b --words w --join 3 --threads 0 --rounds 2 -o out --unnamed";
        let read = asked(&args(line)).expect("the arguments read");
        assert_eq!(read.paths, [PathBuf::from("a"), PathBuf::from("b")]);
        assert_eq!((read.join, read.threads, read.rounds), (3, 1, 2));
        assert!(read.unnamed && read.out.is_some());
        assert!(asked(&args("a --join")).is_err());
        assert!(asked(&args("a --join two")).is_err());
        assert!(asked(&args("a --bogus")).is_err());
    }

    #[test]
    fn a_file_named_by_its_hash_states_it() {
        let file = Path::new("dir/hash_0123456789abcdef");
        assert_eq!(file_hash(file), Some(0x0123_4567_89AB_CDEF));
        assert_eq!(file_hash(Path::new("hash_0123")), None);
        assert_eq!(file_hash(Path::new("kestrel")), None);
    }

    #[test]
    fn an_argument_is_counted_by_its_kind() {
        let c = |bits: u64, kind: Kind| Expr::Const(Const { bits, kind });
        assert_eq!(kind_of(&c(7, Kind::I32)), "small int");
        assert_eq!(kind_of(&c(0xFFFF_FFFF, Kind::I32)), "small int");
        assert_eq!(kind_of(&c(0x1A2B_3C4D, Kind::I32)), "large int");
        assert_eq!(kind_of(&c(0x1A2B_3C4D, Kind::Hash)), "hash");
        assert_eq!(kind_of(&c(0x3F80_0000, Kind::F32)), "float");
    }
}
