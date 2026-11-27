//! `openrage-cli script` on a compiled GTA VI script: its source, its instructions, its natives
//! and its strings, by file or by archive entry, and every script of a build into one folder.
//!
//! ⚠ **Every name is the user's.** OpenRage ships none: a type, a method, a field or a program
//! prints under a name only where a list the user supplies names it, and a native, which the code
//! calls by the address of its handler, only where a list gives the hash of the handler there.
//!
//! ⚠ **Nothing of a program is run.** Every verb reads bytes and writes text.
//!
//! ⚠ **A source has one header.** The printer writes it, and this command puts its own lines
//! after the printer's last: `project`, `only`, `unproven`, `lists read`, `candidates`, `natives
//! lists`, `left out` and `classes`. At the lowest level the printer writes a listing and no header, so the first lines
//! are this command's.

use std::collections::BTreeMap;
use std::fmt::{Display, Write as _};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::gta6::script::ir::{FunctionKey, TypeKey};
use or_io::gta6::script::opcode::{self, Act};
use or_io::gta6::script::program::{FunctionRow, Program};
use or_io::gta6::script::project::index;
use or_io::gta6::script::{decode, lift, Context, Level, Names, Options, Pass, Project, Stats};
use serde_json::{json, Value};

#[path = "gta6_script_export.rs"]
mod export;
#[path = "gta6_script_usage.rs"]
mod help;
#[path = "gta6_script_lists.rs"]
mod lists;
#[path = "gta6_script_names.rs"]
mod names_verbs;
#[path = "gta6_script_source.rs"]
mod source;

pub(super) use help::usage;
use lists::{Supplied, NAMES_FOLDER};
use source::{Head, Keys};

/// The file an export writes its project to, and the one looked for beside a program.
const PROJECT_FILE: &str = "project.json";

/// Why no list names a native of a compiled program.
const NATIVES: &str = "A compiled program calls a native by the address of its handler and \
     states no hash of it: a native is named only where a names list gives the hash of the handler \
     at that address (a table of `hash` and `rva`) and a natives list names the hash; any other \
     shows as `rva_` and the address.";

/// What `--statics` is answered with for a GTA VI program.
const STATICS: &str = "--statics prints a GTA V script's initial static values; a GTA VI \
     program's container holds none, and `script <src>` says how many cells its statics take";

/// What a program decompiled with no project is, and what gives it one.
const ALONE: &str = "the program was decompiled alone, without what other programs state of the \
     types and functions it uses; `script export <folder> -o <out>` writes a project.json, and \
     --project <out>/project.json reads it";

/// The words a type opens with at the margin of a source.
const TYPE_WORDS: [&str; 4] = ["class ", "struct ", "enum ", "interface "];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Verb {
    Info,
    Disasm,
    Decompile,
    Check,
    List,
    Export,
    Passes,
}

impl Verb {
    /// The verb as a line about it names it.
    fn name(self) -> &'static str {
        match self {
            Verb::Info => "script",
            Verb::Disasm => "script disasm",
            Verb::Decompile => "script decompile",
            Verb::Check => "script check",
            Verb::List => "script list",
            Verb::Export => "script export",
            Verb::Passes => "script passes",
        }
    }
}

/// What `script <src>` prints of a program.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum What {
    #[default]
    Summary,
    Natives,
    Strings,
    Functions,
}

/// The rows of `--functions` one call prints: those after the first `offset`, `limit` of them at most.
#[derive(Clone, Copy, Default)]
struct Page {
    offset: usize,
    limit: Option<usize>,
}

/// The arguments after the verb.
#[derive(Default)]
struct Asked {
    paths: Vec<String>,
    what: What,
    json: bool,
    functions: Vec<String>,
    /// `--with-classes`: the types that hold no code of the functions asked for are written too.
    with_classes: bool,
    /// `--offset` and `--limit`.
    page: Page,
    /// Whether either of them was given.
    paged: bool,
    level: Option<Level>,
    /// The passes `--no` leaves off.
    off: Vec<Pass>,
    project: Option<PathBuf>,
    /// `--alone`: no project is looked for beside the program.
    alone: bool,
    out: Option<PathBuf>,
    names: Vec<PathBuf>,
    natives: Vec<PathBuf>,
    keys: Option<PathBuf>,
    exe: Option<PathBuf>,
    entry: Option<String>,
    threads: Option<usize>,
    help: bool,
    /// The first thing wrong with the arguments, were they for a GTA VI script.
    wrong: Option<String>,
}

impl Asked {
    /// What opens an archive, as the arguments name it.
    fn keys(&self) -> Keys<'_> {
        Keys {
            snapshot: self.keys.as_deref(),
            exe: self.exe.as_deref(),
        }
    }
}

/// What a decompile is asked beyond the bytes.
pub struct Decompile<'a> {
    pub names: &'a Names,
    /// What other programs state; without it the program is decompiled alone.
    pub project: Option<&'a Project>,
    /// What the header's `project` line says.
    pub project_said: &'a str,
    pub options: Options,
    /// The functions written; every one where none is named.
    pub only: &'a [String],
    /// Whether the types that hold no code of `only` are written beside it.
    pub with_classes: bool,
    /// More lines for the header, each without its `// `.
    pub said: &'a [String],
}

/// The project a decompile reads, what the header says of it, and what stderr is told of a file beside the program that is no project.
struct Held {
    project: Option<Project>,
    said: String,
    unread: Option<String>,
}

/// Runs `script` where its arguments name a compiled GTA VI script; `None` leaves them to the command for GTA V's.
pub fn run(args: &[String]) -> Option<ExitCode> {
    if let Some(done) = names_verbs::run(args) {
        return Some(done);
    }
    let (verb, rest) = match args.first().map(String::as_str) {
        Some("disasm") => (Verb::Disasm, &args[1..]),
        Some("decompile") => (Verb::Decompile, &args[1..]),
        Some("check") => (Verb::Check, &args[1..]),
        Some("list") => (Verb::List, &args[1..]),
        Some("export") => (Verb::Export, &args[1..]),
        Some("passes") => (Verb::Passes, &args[1..]),
        _ => (Verb::Info, args),
    };
    let asked = asked(rest);
    if asked.help {
        return Some(super::printed(&usage(), ExitCode::SUCCESS));
    }
    if verb == Verb::Passes {
        return Some(super::printed(&passes(), ExitCode::SUCCESS));
    }
    if asked.paths.is_empty() {
        // With no file to tell the game by, what is wrong is said of the arguments as they stand.
        return Some(match &asked.wrong {
            Some(wrong) => super::misused(&format!("{wrong}; `script --help` prints the usage")),
            None => {
                super::note(usage().trim_end());
                ExitCode::from(2)
            }
        });
    }
    if !claimed(verb, &asked.paths) {
        return None;
    }
    if let Some(wrong) = &asked.wrong {
        return Some(super::misused(wrong));
    }
    for flag in unused(verb, &asked) {
        let verb = verb.name();
        super::note(&format!(
            "note: {flag} says nothing to `{verb}` and was not used"
        ));
    }
    let done = match verb {
        Verb::Check => check(&asked.paths),
        Verb::Export => export::run(&asked),
        Verb::List => list(&asked).map(|text| (text, true)),
        _ => one(verb, &asked).map(|text| (text, true)),
    };
    // An export's `-o` is the folder its files go to; its lines are printed.
    let to = asked.out.as_deref().filter(|_| verb != Verb::Export);
    let put = done.and_then(|(text, clean)| match to {
        Some(to) => std::fs::write(to, text)
            .map(|()| clean)
            .map_err(|e| format!("write {}: {e}", to.display())),
        None => super::put(&text)
            .map(|()| clean)
            .map_err(|e| format!("write to stdout: {e}")),
    });
    Some(match put {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => super::failed(&e),
    })
}

/// Whether the paths named make the arguments this command's.
// `check` reads each file by its own first bytes, so one GTA VI file among them makes the run
// this command's; any other verb is told by the first path alone.
fn claimed(verb: Verb, paths: &[String]) -> bool {
    match verb {
        Verb::Check => paths.iter().any(|path| ours(verb, Path::new(path))),
        _ => paths
            .first()
            .is_some_and(|path| ours(verb, Path::new(path))),
    }
}

/// Whether one path named makes the arguments this command's: by the file's first bytes, never by its name.
// ⚠ GTA V's command keeps every file this one does not know for GTA VI's, with its refusal of it.
fn ours(verb: Verb, first: &Path) -> bool {
    match verb {
        Verb::List | Verb::Export | Verb::Passes => true,
        Verb::Check if first.is_dir() => true,
        _ => {
            let head = File::open(first).and_then(source::head_of);
            matches!(head, Ok(Head::Program | Head::Archive | Head::Text))
        }
    }
}

/// Reads the arguments after the verb; what is wrong with them is kept, not reported, since they may be GTA V's.
fn asked(args: &[String]) -> Asked {
    let mut asked = Asked::default();
    let mut wrong: Option<String> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" | "help" => asked.help = true,
            "--natives" => asked.what = What::Natives,
            "--strings" => asked.what = What::Strings,
            "--functions" => asked.what = What::Functions,
            "--json" => asked.json = true,
            "--with-classes" => asked.with_classes = true,
            "--alone" => asked.alone = true,
            // GTA V's listing has a column of bytes to leave out, and this one has none.
            "--no-bytes" => {}
            // GTA V's script holds the first values of its statics, and this container holds none.
            "--statics" => {
                wrong.get_or_insert(STATICS.to_owned());
            }
            "--natives-db" | "--names" | "--keys" | "--exe" | "--entry" | "--function"
            | "--level" | "--no" | "--project" | "--threads" | "--offset" | "--limit" | "-o" => {
                let Some(value) = iter.next() else {
                    wrong.get_or_insert(format!("{arg} needs a value"));
                    continue;
                };
                let unknown = |kind: &str, all: &[&str]| {
                    format!("unknown {kind} `{value}`; it is one of: {}", all.join(", "))
                };
                // A count of `least` or more, or what the flag is refused with.
                let count = |least: usize| {
                    let read = value.parse::<usize>().ok().filter(|n| *n >= least);
                    read.ok_or_else(|| match least {
                        0 => format!("{arg} takes a number, not `{value}`"),
                        _ => format!("{arg} takes a number above 0, not `{value}`"),
                    })
                };
                let mut refused = None;
                match arg.as_str() {
                    "--natives-db" => asked.natives.push(PathBuf::from(value)),
                    "--names" => asked.names.push(PathBuf::from(value)),
                    "--keys" => asked.keys = Some(PathBuf::from(value)),
                    "--exe" => asked.exe = Some(PathBuf::from(value)),
                    "--entry" => asked.entry = Some(value.clone()),
                    "--function" => asked.functions.push(value.clone()),
                    "--project" => asked.project = Some(PathBuf::from(value)),
                    "-o" => asked.out = Some(PathBuf::from(value)),
                    "--level" => match Level::from_name(value) {
                        Some(level) => asked.level = Some(level),
                        None => refused = Some(unknown("level", &Level::ALL.map(Level::name))),
                    },
                    "--no" => match Pass::from_name(value) {
                        Some(pass) => asked.off.push(pass),
                        None => refused = Some(unknown("pass", &Pass::ALL.map(Pass::name))),
                    },
                    "--offset" => match count(0) {
                        Ok(offset) => (asked.page.offset, asked.paged) = (offset, true),
                        Err(why) => refused = Some(why),
                    },
                    "--limit" => match count(1) {
                        Ok(limit) => (asked.page.limit, asked.paged) = (Some(limit), true),
                        Err(why) => refused = Some(why),
                    },
                    _ => match count(1) {
                        Ok(threads) => asked.threads = Some(threads),
                        Err(why) => refused = Some(why),
                    },
                }
                if let Some(refused) = refused {
                    wrong.get_or_insert(refused);
                }
            }
            other if other.starts_with('-') => {
                wrong.get_or_insert(format!("unknown option: {other}"));
            }
            other => asked.paths.push(other.to_owned()),
        }
    }
    asked.wrong = wrong;
    asked
}

/// The options given that say nothing to `verb`, each as a line names it.
fn unused(verb: Verb, asked: &Asked) -> Vec<&'static str> {
    let reads_code = matches!(verb, Verb::Disasm | Verb::Decompile);
    let levelled = matches!(verb, Verb::Decompile | Verb::Export);
    let lists_functions = verb == Verb::Info && asked.what == What::Functions;
    let given = [
        (
            asked.json && !matches!(verb, Verb::Info | Verb::List),
            "--json",
        ),
        (
            asked.what != What::Summary && verb != Verb::Info,
            "--natives, --strings or --functions",
        ),
        (!asked.functions.is_empty() && !reads_code, "--function"),
        (
            asked.with_classes && verb != Verb::Decompile,
            "--with-classes",
        ),
        (asked.paged && !lists_functions, "--offset or --limit"),
        (
            (asked.level.is_some() || !asked.off.is_empty()) && !levelled,
            "--level or --no",
        ),
        (asked.project.is_some() && !reads_code, "--project"),
        (asked.alone && verb != Verb::Decompile, "--alone"),
        (asked.threads.is_some() && verb != Verb::Export, "--threads"),
    ];
    let said = given.into_iter().filter(|(given, _)| *given);
    said.map(|(_, flag)| flag).collect()
}

/// The level and the passes asked for: every pass of the level but those `--no` names.
fn options(asked: &Asked) -> Options {
    let at = Options::at(asked.level.unwrap_or(Level::Source));
    asked
        .off
        .iter()
        .fold(at, |options, pass| options.without(*pass))
}

/// The passes of the level of `options` that are off, by name.
fn left_off(options: Options) -> Vec<&'static str> {
    let all = Options::at(options.level).passes.iter();
    all.filter(|pass| !options.on(*pass))
        .map(Pass::name)
        .collect()
}

/// The passes that ran for `options`, by name: those on whose names a pass reads.
fn ran(options: Options) -> Vec<&'static str> {
    let on = options.passes.iter().filter(|pass| pass.rebuilds());
    on.map(Pass::name).collect()
}

/// What `passes` prints: each pass and its level, and of a name no pass reads yet that it rebuilds nothing.
fn passes() -> String {
    let mut out = String::new();
    for pass in Pass::ALL {
        let (name, level) = (pass.name(), pass.level().name());
        let _ = match pass.rebuilds() {
            true => writeln!(out, "{name:<16}{level}"),
            false => writeln!(out, "{name:<16}{level:<12}rebuilds nothing yet"),
        };
    }
    out
}

/// The lists the arguments name, else those of the folder of the configuration directory.
fn supplied_lists(asked: &Asked) -> Result<Supplied, String> {
    let config = or_title::config::config_dir();
    let supplied = lists::supplied(&asked.names, &asked.natives, config.as_deref())?;
    for refused in &supplied.refused {
        super::note(&format!("note: {refused}"));
    }
    for label in &supplied.candidates {
        super::note(&format!(
            "note: `{label}` of the names folder was passed over: a file whose name holds `{}` \
             holds guesses, and --names reads it",
            lists::CANDIDATES
        ));
    }
    Ok(supplied)
}

/// Names each field and method record of `program` that a supplied list names by the hash the record holds.
pub fn bind(names: &mut Names, program: &Program) {
    for ty in &program.types {
        for field in &ty.fields {
            names.bind_field(field.name, TypeKey(ty.name), field.offset);
        }
    }
    for method in &program.methods {
        let owner = program.types.get(method.owner as usize);
        if let (Some(owner), Some(slot)) = (owner, method.slot) {
            names.bind_method(method.name, TypeKey(owner.name), slot);
        }
    }
    names.bind_entries(program.program, &program.entries);
}

/// One verb on one program.
fn one(verb: Verb, asked: &Asked) -> Result<String, String> {
    let src = source::open(&asked.paths, asked.entry.as_deref(), asked.keys())?;
    let program = Program::read(&src.bytes)
        .map_err(|e| format!("{} is no GTA VI program: {e}", src.read_from))?;
    let mut lists = supplied_lists(asked)?;
    bind(&mut lists.names, &program);
    let names = &lists.names;
    match verb {
        // With a project, a function only another program calls starts where the project holds it.
        Verb::Disasm => {
            let project = asked.project.as_deref().map(project_at).transpose()?;
            let program = match &project {
                Some((_, project)) => lift::read(&src.bytes, project).map_err(|e| e.to_string())?,
                None => program,
            };
            listing(&program, names, &asked.functions)
        }
        Verb::Decompile => {
            let held = held(asked)?;
            if let Some(unread) = &held.unread {
                super::note(&format!("note: {unread}"));
            }
            let said = lists.said();
            let ask = Decompile {
                names,
                project: held.project.as_ref(),
                project_said: &held.said,
                options: options(asked),
                only: &asked.functions,
                with_classes: asked.with_classes,
                said: &said,
            };
            let (text, stats) = decompiled(&src.bytes, &ask)?;
            if let Some(bodies) = bodies_note(&stats, held.project.is_some()) {
                super::note(&bodies);
            }
            Ok(text)
        }
        _ => match asked.what {
            What::Summary => Ok(info(&src, &program, names, asked.json)),
            What::Natives => natives(&program, names, asked.json),
            What::Strings => Ok(strings(&program, asked.json)),
            What::Functions => {
                let (text, page) = functions(&program, names, asked.json, asked.page);
                if let Some(page) = page {
                    super::note(&page);
                }
                Ok(text)
            }
        },
    }
}

/// The project a file holds, and the phrase a header names it by.
fn project_at(path: &Path) -> Result<(String, Project), String> {
    let refused = |why: String| format!("{}: {why}", path.display());
    let bytes = std::fs::read(path).map_err(|e| refused(format!("read: {e}")))?;
    let project = index::load(&bytes).map_err(|e| refused(e.to_string()))?;
    let name = path.file_name().map(|name| name.to_string_lossy());
    Ok((format!("`{}`", name.unwrap_or_default()), project))
}

/// The project a decompile reads: the one named, else the `project.json` beside a program that is a file, else none.
// ⚠ A file nobody named is no reason to fail: one that is no project leaves the program
// decompiled alone, and the header and stderr both say so.
fn held(asked: &Asked) -> Result<Held, String> {
    if let Some(path) = &asked.project {
        let (said, project) = project_at(path)?;
        return Ok(Held {
            project: Some(project),
            said,
            unread: None,
        });
    }
    let alone = |said: String, unread: Option<String>| Held {
        project: None,
        said,
        unread,
    };
    if asked.alone {
        return Ok(alone(format!("none, as --alone asks: {ALONE}"), None));
    }
    // An entry of an archive lies beside no export.
    let beside = match (asked.paths.as_slice(), &asked.entry) {
        ([path], None) => Path::new(path).parent().map(|at| at.join(PROJECT_FILE)),
        _ => None,
    };
    Ok(match beside.filter(|file| file.is_file()) {
        None => alone(format!("none: {ALONE}"), None),
        Some(file) => match project_at(&file) {
            Ok((said, project)) => Held {
                project: Some(project),
                said: format!("{said}, found beside the program; --alone decompiles without it"),
                unread: None,
            },
            Err(why) => {
                let unread = format!(
                    "`{PROJECT_FILE}` beside the program is no project and was not read ({why})"
                );
                alone(format!("none: {unread}; {ALONE}"), Some(unread))
            }
        },
    })
}

/// What stderr is told of the functions written with no body; nothing where each has one.
fn bodies_note(stats: &Stats, with_project: bool) -> Option<String> {
    let remedy = match with_project {
        true => "each says why where it stands",
        false => {
            "each says why where it stands, and most need what another program states: `script \
             export <folder> -o <out>` writes a project.json, and --project <out>/project.json \
             reads it"
        }
    };
    (stats.incomplete > 0).then(|| {
        format!(
            "note: {} of {} function(s) have no body; {remedy}",
            stats.incomplete, stats.functions
        )
    })
}

fn hex(hash: u32) -> String {
    format!("0x{hash:08X}")
}

/// A program's hash, with its name where a list names it.
fn program_name(names: &Names, hash: u32) -> String {
    match names.program(u64::from(hash)) {
        Some(name) => format!("0x{hash:08X} ({name})"),
        None => hex(hash),
    }
}

/// The name a function prints under: the user's, else the two the header names, else its offset.
pub fn function_name(program: &Program, names: &Names, row: &FunctionRow) -> String {
    if let Some(name) = names.function(FunctionKey::new(program.program, row.at)) {
        return name.to_owned();
    }
    match Some(row.at) {
        at if at == program.main => "main".to_owned(),
        at if at == program.initialize => "initialize".to_owned(),
        _ => format!("func_{:06X}", row.at),
    }
}

/// The functions `asked` names, by rising offset: `0x` or `func_` and an offset, a name as `--functions` prints it, or an offset as a listing prints it.
fn selected(
    program: &Program,
    names: &Names,
    asked: &[String],
) -> Result<Vec<FunctionRow>, String> {
    let mut rows = Vec::new();
    for name in asked {
        let digits = name
            .strip_prefix("0x")
            .or_else(|| name.strip_prefix("func_"));
        let at = digits.and_then(|digits| u32::from_str_radix(digits, 16).ok());
        let found: Vec<&FunctionRow> = match at {
            Some(at) => program.function(at).into_iter().collect(),
            None => {
                let named = |row: &&FunctionRow| function_name(program, names, row) == *name;
                let by_name: Vec<&FunctionRow> = program.functions.iter().filter(named).collect();
                // A name comes first: hex digits alone are an offset only where no function is called so.
                let digits = name.bytes().all(|b| b.is_ascii_hexdigit());
                let at = u32::from_str_radix(name, 16).ok().filter(|_| digits);
                match (by_name.is_empty(), at) {
                    (true, Some(at)) => program.function(at).into_iter().collect(),
                    _ => by_name,
                }
            }
        };
        match found.as_slice() {
            [row] => rows.push(**row),
            [] => {
                return Err(format!(
                    "no function `{name}` in program {}; `--functions` lists them",
                    hex(program.program)
                ))
            }
            // A name two functions carry chooses neither.
            several => {
                let at: Vec<String> = several
                    .iter()
                    .map(|row| format!("0x{:06X}", row.at))
                    .collect();
                return Err(format!(
                    "`{name}` names {} functions, at {}: name one by its offset",
                    several.len(),
                    at.join(", ")
                ));
            }
        }
    }
    rows.sort_by_key(|row| row.at);
    rows.dedup();
    Ok(rows)
}

/// What a walk of a program's code counts.
#[derive(Default)]
struct Walk {
    instructions: usize,
    /// Each native handler called, by its address: the cells in, the cells out, the call sites.
    natives: BTreeMap<u32, (u64, u64, usize)>,
}

/// Walks every instruction of every function against no table: only bytes that are no instruction stop it.
// A type or a function another program defines is not asked for, so a program is walked alone.
fn walked(program: &Program) -> Result<Walk, decode::DisasmError> {
    let mut walk = Walk::default();
    for row in &program.functions {
        let mut at = row.at;
        while at < row.end {
            let raw = decode::raw(&program.code, at)?;
            walk.instructions += 1;
            if opcode::spec(raw.opcode).is_some_and(|spec| spec.act == Act::CallNative) {
                let [rva, args, returns, _] = raw.fields;
                let native = walk.natives.entry(rva as u32).or_insert((args, returns, 0));
                native.2 += 1;
            }
            // An instruction is two bytes at least, so the walk ends; one past its function is refused.
            at = match at
                .checked_add(raw.len.max(1))
                .filter(|next| *next <= row.end)
            {
                Some(next) => next,
                None => {
                    let what = "it does not end inside its function".to_owned();
                    return Err(decode::DisasmError::Malformed { at, what });
                }
            };
        }
    }
    Ok(walk)
}

fn line(out: &mut String, key: &str, value: impl Display) {
    let _ = writeln!(out, "{key:<18}{value}");
}

/// What lists named things, as the last lines of `script <src>` and the first of a listing say it.
fn names_sentence(names: &Names) -> String {
    let lists = names.sources();
    match lists.is_empty() {
        true => "no names list supplied: a type, a method, a field or a program is shown by its \
                 hash, its slot or its offset"
            .to_owned(),
        false => format!("from {}", lists.join(", ")),
    }
}

/// What `script <src>` prints: one line per fact the reader reads, and none for what it does not.
fn info(src: &source::Source, program: &Program, names: &Names, json: bool) -> String {
    let walk = walked(program);
    let imports = program.imports.iter();
    let imports: Vec<String> = imports
        .map(|import| program_name(names, import.program))
        .collect();
    let strings = program.info().strings.len();
    if json {
        let counted = walk.as_ref().ok();
        let value = json!({
            "program": hex(program.program),
            "name": names.program(u64::from(program.program)),
            "readFrom": src.read_from,
            "size": src.bytes.len(),
            "code": program.code.len(),
            "functions": program.functions.len(),
            "types": program.types.len(),
            "methods": program.methods.len(),
            "imports": program.imports.iter().map(|i| hex(i.program)).collect::<Vec<_>>(),
            "entries": program.entries.len(),
            "globals": program.globals.cells,
            "statics": program.statics.cells,
            "strings": strings,
            "pooled": program.strings.len(),
            "natives": counted.map(|walk| walk.natives.len()),
            "instructions": counted.map(|walk| walk.instructions),
            "decodes": walk.as_ref().err().map(ToString::to_string),
            "names": names.lists,
        });
        return format!("{value:#}\n");
    }
    let mut out = String::new();
    line(&mut out, "program", program_name(names, program.program));
    line(&mut out, "read from", &src.read_from);
    line(&mut out, "size", format!("{} byte(s)", src.bytes.len()));
    let functions = program.functions.len();
    let code = format!("{} byte(s) in {functions} function(s)", program.code.len());
    line(&mut out, "code", code);
    let (types, methods) = (program.types.len(), program.methods.len());
    line(
        &mut out,
        "types",
        format!("{types} own, {methods} method record(s)"),
    );
    let imported = match imports.is_empty() {
        true => "none".to_owned(),
        false => format!("{}: {}", imports.len(), imports.join(", ")),
    };
    line(&mut out, "imports", imported);
    let entries = program.entries.len();
    line(
        &mut out,
        "entries",
        format!("{entries} function(s) the engine calls by name"),
    );
    let (globals, statics) = (program.globals.cells, program.statics.cells);
    line(
        &mut out,
        "globals",
        format!("{globals} cell(s), statics {statics}"),
    );
    let pooled = program.strings.len();
    line(
        &mut out,
        "strings",
        format!("{strings} read from {pooled} byte(s) of pooled data"),
    );
    match &walk {
        Ok(walk) => {
            let sites: usize = walk.natives.values().map(|native| native.2).sum();
            let natives = format!(
                "{} handler(s) called at {sites} site(s)",
                walk.natives.len()
            );
            line(&mut out, "natives", natives);
            line(&mut out, "instructions", walk.instructions);
        }
        Err(refused) => line(
            &mut out,
            "instructions",
            format!("does not decode: {refused}"),
        ),
    }
    let _ = writeln!(out, "\n{NATIVES}");
    let _ = writeln!(out, "Names: {}.", names_sentence(names));
    if names.lists.is_empty() {
        let _ = writeln!(
            out,
            "Supply lists with --names, or put them in the `{NAMES_FOLDER}` folder of the \
             configuration directory."
        );
    }
    out
}

/// What `--natives` prints: each handler the code calls, by its address, with the hash a names list gives the handler and the name a natives list gives the hash.
fn natives(program: &Program, names: &Names, json: bool) -> Result<String, String> {
    let walk = walked(program);
    let walk =
        walk.map_err(|e| format!("program {} does not decode: {e}", hex(program.program)))?;
    let listed = |rva: u32| {
        let hash = names.native_at(rva)?;
        Some((hash, names.native(hash).map(ToString::to_string)))
    };
    if json {
        let rows = walk.natives.iter().map(|(rva, (args, returns, sites))| {
            let (hash, name) = listed(*rva).unzip();
            json!({
                "rva": hex(*rva),
                "args": args,
                "returns": returns,
                "sites": sites,
                "hash": hash.map(|hash| format!("0x{hash:016X}")),
                "name": name.flatten(),
            })
        });
        return Ok(format!("{:#}\n", Value::Array(rows.collect())));
    }
    let mut out = String::new();
    let (mut hashed, mut named) = (0usize, 0usize);
    for (rva, (args, returns, sites)) in &walk.natives {
        let _ = write!(out, "rva_{rva:08X}  {args} -> {returns}  {sites} site(s)");
        if let Some((hash, name)) = listed(*rva) {
            hashed += 1;
            let _ = write!(out, "  0x{hash:016X}");
            if let Some(name) = name {
                named += 1;
                let _ = write!(out, "  {name}");
            }
        }
        out.push('\n');
    }
    let sites: usize = walk.natives.values().map(|native| native.2).sum();
    let _ = writeln!(
        out,
        "\n{} distinct native(s) across {sites} call site(s), with the cells each takes and \
         returns; the native table holds {} word(s), none of them a native's hash.",
        walk.natives.len(),
        program.natives.len()
    );
    let _ = writeln!(
        out,
        "{hashed} of them have the hash a names list gives their handler, and {named} of those \
         the name a natives list gives that hash."
    );
    let _ = writeln!(out, "{NATIVES}");
    Ok(out)
}

/// What `--strings` prints: each string the reader states under its offset, one line each whatever it holds.
fn strings(program: &Program, json: bool) -> String {
    let strings = program.info().strings;
    if json {
        let rows = strings
            .iter()
            .map(|(offset, text)| json!({ "offset": format!("0x{offset:06X}"), "text": text }));
        return format!("{:#}\n", Value::Array(rows.collect()));
    }
    let mut out = String::new();
    for (offset, text) in &strings {
        let _ = writeln!(out, "0x{offset:06X}\t{text:?}");
    }
    let pooled = program.strings.len();
    let _ = write!(
        out,
        "\n{} string(s) read from {pooled} byte(s) of pooled data.",
        strings.len()
    );
    if strings.is_empty() && pooled > 0 {
        out.push_str(" The program's code pushes no string.");
    }
    out.push('\n');
    out
}

/// What `--functions` prints of one page: each function by offset, its bytes, its cells in and out, its name; and what stderr is told where the page is not the whole list.
// As JSON a function is one line, so that a list cut short loses whole functions and is seen to.
fn functions(program: &Program, names: &Names, json: bool, page: Page) -> (String, Option<String>) {
    let total = program.functions.len();
    let from = page.offset.min(total);
    let to = page
        .limit
        .map_or(total, |limit| from.saturating_add(limit).min(total));
    let rows = program.functions.get(from..to).unwrap_or_default();
    let said = match (rows.is_empty(), to < total) {
        _ if rows.len() == total => None,
        (true, _) => Some(format!(
            "note: no function printed: the program has {total}, numbered from 0, and --offset \
             {} is past the last",
            page.offset
        )),
        (false, true) => Some(format!(
            "note: functions {from} to {} of {total}, numbered from 0; --offset {to} prints \
             the next",
            to - 1
        )),
        (false, false) => Some(format!(
            "note: functions {from} to {} of {total}, numbered from 0: the last of them",
            to - 1
        )),
    };
    if json {
        let rows: Vec<String> = rows
            .iter()
            .map(|row| {
                let value = json!({
                    "at": format!("0x{:06X}", row.at),
                    "bytes": row.end.saturating_sub(row.at),
                    "params": row.params,
                    "locals": row.locals,
                    "returns": row.returns,
                    "name": function_name(program, names, row),
                });
                value.to_string()
            })
            .collect();
        let text = match rows.is_empty() {
            true => "[]\n".to_owned(),
            false => format!("[\n{}\n]\n", rows.join(",\n")),
        };
        return (text, said);
    }
    let mut out = String::new();
    for row in rows {
        let _ = writeln!(
            out,
            "0x{:06X}  {} byte(s)  {} -> {}  {}",
            row.at,
            row.end.saturating_sub(row.at),
            row.params,
            row.returns,
            function_name(program, names, row)
        );
    }
    (out, said)
}

/// The instruction listing of `program` under its two first lines; only the functions `only` names where it names any.
pub fn listing(program: &Program, names: &Names, only: &[String]) -> Result<String, String> {
    let rows = selected(program, names, only)?;
    let rows = match only.is_empty() {
        true => program.functions.as_slice(),
        false => rows.as_slice(),
    };
    let mut body = String::new();
    for row in rows {
        decode::write_function(&mut body, program, row, names)
            .map_err(|e| format!("program {} does not decode: {e}", hex(program.program)))?;
    }
    // A function's line starts at the margin; an instruction's is indented.
    let instructions = body.lines().filter(|line| line.starts_with(' ')).count();
    let kept = rows.len();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "; program {}: {} byte(s) of code; {instructions} instruction(s) in {kept} function(s) \
         listed",
        hex(program.program),
        program.code.len()
    );
    let _ = writeln!(out, "; {NATIVES}");
    out.push_str(&body);
    Ok(out)
}

/// The lines this command adds to a source's header: the project, the functions asked for, an opcode nothing proves, then what its caller has to say.
// ⚠ No line of it may start `// import `: a listing's reader takes that for an import, and the
// output of the lowest level is read back as a listing.
fn header(program: &Program, ask: &Decompile<'_>, only: &[FunctionRow]) -> String {
    let mut lines = vec![format!("project: {}", ask.project_said)];
    if !only.is_empty() {
        let only: Vec<String> = only
            .iter()
            .map(|row| function_name(program, ask.names, row))
            .collect();
        lines.push(format!("only: {}", only.join(", ")));
    }
    if let Some((at, name)) = decode::first_unseen(&program.code) {
        lines.push(format!(
            "unproven: instruction {at:06X} is `{name}`, an opcode no program was seen to hold: \
             its length is its family's, and where each function after it starts rests on that"
        ));
    }
    lines.extend(ask.said.iter().cloned());
    let mut out = String::new();
    for line in lines {
        // What a user's file or a reader's refusal says is kept to the one line of its comment.
        let _ = writeln!(out, "// {}", line.replace(['\n', '\r'], " "));
    }
    out
}

/// The first lines of a source at the lowest level, where the printer writes a listing and no header.
fn intro(program: &Program, names: &Names) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "// Readable source of a compiled GTA VI script, program {}: decompiled from the \
         program's own bytes, of which nothing was run.",
        program_name(names, program.program)
    );
    if !program.imports.is_empty() {
        let imports = program.imports.iter();
        let imports: Vec<String> = imports.map(|i| program_name(names, i.program)).collect();
        let _ = writeln!(out, "// imports: {}", imports.join(", "));
    }
    let _ = writeln!(out, "// natives: {NATIVES}");
    let _ = writeln!(out, "// names: {}", names_sentence(names));
    let _ = writeln!(
        out,
        "// level: {}, at which no pass runs",
        Level::Lifted.name()
    );
    out
}

/// A header line for each function of `rows` that holds code its entry does not reach.
// ⚠ Read alone, a program shows no function that only another program calls: it stands in that code.
fn left_out(program: &Program, names: &Names, rows: &[FunctionRow]) -> String {
    let mut out = String::new();
    for row in rows {
        if let Some((at, bytes)) = lift::unreached(program, Project::empty(), row) {
            let _ = writeln!(
                out,
                "// left out: {bytes} byte(s) from {at:06X}, which the entry of {} does not \
                 reach: a function only another program calls may stand there, and --project \
                 finds it",
                function_name(program, names, row)
            );
        }
    }
    out
}

/// `line` without the line break it ends on.
fn bare(line: &str) -> &str {
    line.trim_end_matches(['\n', '\r'])
}

/// Where the type that opens at line `at` of `lines` ends, one past its closing brace; `None` where no type opens there as the printer writes one.
// ⚠ Strict on purpose: a head at the margin, then indented lines of it, a `{` alone, lines that are
// blank or indented, and a `}` alone. Anything else is no type here and is written as it stands.
fn type_end(lines: &[&str], at: usize) -> Option<usize> {
    let head = bare(lines.get(at)?);
    if !TYPE_WORDS.iter().any(|word| head.starts_with(word)) {
        return None;
    }
    let mut n = at + 1;
    while lines.get(n).is_some_and(|line| line.starts_with(' ')) {
        n += 1;
    }
    if bare(lines.get(n)?) != "{" {
        return None;
    }
    loop {
        n += 1;
        let line = bare(lines.get(n)?);
        if line == "}" {
            return Some(n + 1);
        }
        if !(line.is_empty() || line.starts_with(' ')) {
            return None;
        }
    }
}

/// `text` without each type at its margin that holds no code and none of `marks`, and how many went.
// ⚠ Code is a line of the type that ends on a `{` of its own: a method's body, a type nested in
// it. A type that holds one stays whole, and so does one that only names a function asked for,
// so no line of code is ever dropped.
fn without_data_types(text: &str, marks: &[String]) -> (String, usize) {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut out = String::with_capacity(text.len());
    let (mut at, mut dropped) = (0, 0);
    while let Some(line) = lines.get(at) {
        let Some(end) = type_end(&lines, at) else {
            out.push_str(line);
            at += 1;
            continue;
        };
        let inside = lines.get(at + 1..end).unwrap_or_default();
        let code = |line: &&str| line.starts_with(' ') && bare(line).ends_with('{');
        let marked = |line: &&str| marks.iter().any(|mark| line.contains(mark.as_str()));
        if inside.iter().any(|line| code(line) || marked(line)) {
            lines
                .get(at..end)
                .unwrap_or_default()
                .iter()
                .for_each(|line| out.push_str(line));
        } else {
            dropped += 1;
            // The blank line before the type goes with it.
            if out.ends_with("\n\n") {
                out.pop();
            }
        }
        at = end;
    }
    (out, dropped)
}

/// The source of the program `bytes` hold under its header, and what decompiling it counted.
pub fn decompiled(bytes: &[u8], ask: &Decompile<'_>) -> Result<(String, Stats), String> {
    let held = ask.project.unwrap_or(Project::empty());
    let program = lift::read(bytes, held).map_err(|e| e.to_string())?;
    written(&program, ask)
}

/// The source of `program` as `ask` asks it, under one header: the printer's, then this command's lines.
fn written(program: &Program, ask: &Decompile<'_>) -> Result<(String, Stats), String> {
    let held = ask.project.unwrap_or(Project::empty());
    let rows = selected(program, ask.names, ask.only)?;
    let cx = Context::new(ask.names, held, ask.options);
    let asked = match rows.is_empty() {
        true => program.functions.as_slice(),
        false => rows.as_slice(),
    };
    let output = lift::functions(program, asked, &cx);
    let mut body: String = output.files.iter().map(|file| file.text.as_str()).collect();
    let mut own = header(program, ask, &rows);
    if ask.project.is_none() {
        own.push_str(&left_out(program, ask.names, asked));
    }
    let lifted = ask.options.level == Level::Lifted;
    if !(rows.is_empty() || ask.with_classes || lifted) {
        // A function with no body is a comment that names it, by its offset in either form.
        let marks: Vec<String> = rows
            .iter()
            .flat_map(|row| {
                [
                    format!("func_{:06X}", row.at),
                    format!(" at 0x{:06X}", row.at),
                ]
            })
            .collect();
        let (kept, dropped) = without_data_types(&body, &marks);
        if dropped > 0 {
            body = kept;
            let _ = writeln!(
                own,
                "// classes: {dropped} type(s) that hold no code of the function(s) asked for \
                 are left out; --with-classes writes them"
            );
        }
    }
    let text = match lifted {
        true => format!("{}{own}{body}", intro(program, ask.names)),
        false => super::after_header(&body, &own),
    };
    Ok((text, output.stats))
}

/// What `check` prints, and whether every script decodes; a folder stands for every file under it that starts as a program does.
// Each file goes to the reader its own first bytes ask for, so the order of the files changes nothing.
fn check(paths: &[String]) -> Result<(String, bool), String> {
    let mut files = Vec::new();
    for path in paths.iter().map(Path::new) {
        match path.is_dir() {
            true => files.extend(source::program_files(path)?),
            false => files.push(path.to_path_buf()),
        }
    }
    if files.is_empty() {
        return Err(
            "no file there starts with eight zero bytes, as a deciphered GTA VI program does"
                .to_owned(),
        );
    }
    let (mut out, mut clean) = (String::new(), 0usize);
    for path in &files {
        let head = File::open(path).and_then(source::head_of);
        // A GTA V script among them is GTA V's reader's, and its line is the one that reader prints.
        if matches!(head, Ok(Head::Resource)) {
            let _ = match super::checked(path) {
                Ok((line, agrees)) => {
                    clean += usize::from(agrees);
                    writeln!(out, "{line}")
                }
                Err(why) => writeln!(out, "{why}"),
            };
            continue;
        }
        // ⚠ Only a file that starts as a program does is read: an archive is tens of gigabytes.
        let outcome = match head {
            Ok(Head::Program) => std::fs::read(path)
                .map_err(|e| format!("read: {e}"))
                .and_then(|bytes| checked(&bytes)),
            Ok(Head::Archive) => Err(
                "a GTA VI archive: `script list` names its programs, and --entry reads one"
                    .to_owned(),
            ),
            Ok(Head::Text) => Err(source::TEXT.to_owned()),
            Ok(Head::Resource | Head::Other) => Err(
                "neither a GTA V script, which starts `RSC7`, nor a deciphered GTA VI program, \
                 which starts with eight zero bytes"
                    .to_owned(),
            ),
            Err(e) => Err(format!("read: {e}")),
        };
        let _ = match outcome {
            Ok((counts, None)) => {
                clean += 1;
                writeln!(out, "{}: clean — {counts}", path.display())
            }
            // ⚠ Never "clean": the walk stepped over it by a length nothing establishes.
            Ok((counts, Some(unseen))) => {
                writeln!(out, "{}: unproven — {counts}; {unseen}", path.display())
            }
            Err(why) => writeln!(out, "{}: {why}", path.display()),
        };
    }
    let _ = writeln!(out, "{clean} of {} script(s) decode.", files.len());
    Ok((out, clean == files.len()))
}

/// What a program that decodes is reported as, with what is said of its first instruction of an opcode no program was seen to hold; or the first refusal of its container or of an instruction.
fn checked(bytes: &[u8]) -> Result<(String, Option<String>), String> {
    let program = Program::read(bytes).map_err(|e| e.to_string())?;
    let walk = walked(&program).map_err(|e| e.to_string())?;
    let counts = format!(
        "{} byte(s) of code, {} instruction(s) in {} function(s)",
        program.code.len(),
        walk.instructions,
        program.functions.len()
    );
    let unseen = decode::first_unseen(&program.code).map(|(at, name)| {
        format!(
            "instruction {at:06X} is `{name}`, an opcode no program was seen to hold, and where \
             each function after it starts rests on its family's length"
        )
    });
    Ok((counts, unseen))
}

/// What `list` prints: the entries of an archive whose deciphered bodies read as programs.
fn list(asked: &Asked) -> Result<String, String> {
    let Some((archive, inner)) = asked.paths.split_first() else {
        return Err("no archive was named".to_owned());
    };
    let names = supplied_lists(asked)?.names;
    let reader = source::Reader::new(asked.keys())?;
    let found = reader.programs(Path::new(archive), inner, 0, None)?;
    let mut rows = Vec::new();
    for body in &found.bodies {
        if let (Some(entry), Ok(program)) = (&body.entry, Program::read(&body.bytes)) {
            rows.push((entry, body.bytes.len(), program.program));
        }
    }
    if asked.json {
        let rows = rows.iter().map(|(entry, size, program)| {
            json!({
                "index": entry.index,
                "hash": format!("0x{:016x}", entry.name64),
                "entry": entry.name,
                "size": size,
                "program": hex(*program),
                "name": names.program(u64::from(*program)),
            })
        });
        return Ok(format!("{:#}\n", Value::Array(rows.collect())));
    }
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{} program(s) among {} enciphered entr(ies), {} of which did not decipher",
        rows.len(),
        found.enciphered,
        found.unread
    );
    for (entry, size, program) in rows {
        let _ = writeln!(
            out,
            "  {:>6}  0x{:016X}  {size:>10}  {}  {}",
            entry.index,
            entry.name64,
            program_name(&names, program),
            entry.display_name()
        );
    }
    Ok(out)
}

#[cfg(test)]
#[path = "gta6_script_tests.rs"]
mod tests;
