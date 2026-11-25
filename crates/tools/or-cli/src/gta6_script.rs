//! `openrage-cli script` on a compiled GTA VI script: its source, its instructions, its natives
//! and its strings, by file or by archive entry, and every script of a build into one folder.
//!
//! ⚠ **Every name is the user's.** OpenRage ships none: a type, a method, a field or a program
//! prints under a name only where a list the user supplies names it, and a native, which the code
//! calls by the address of its handler, only where a list gives the hash of the handler there.
//!
//! ⚠ **Nothing of a program is run.** Every verb reads bytes and writes text.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Write as _};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::gta6::script::ir::{FunctionKey, TypeKey};
use or_io::gta6::script::opcode::{self, Act};
use or_io::gta6::script::program::{FunctionRow, Program};
use or_io::gta6::script::project::index;
use or_io::gta6::script::{disasm, lift, Context, Level, Names, Options, Pass, Project, Stats};
use serde_json::{json, Value};

#[path = "gta6_script_export.rs"]
mod export;
#[path = "gta6_script_source.rs"]
mod source;

use source::Head;

/// The folder of the configuration directory whose files are names lists, read when none is named.
const NAMES_FOLDER: &str = "gta6_script_names";

/// Why no list names a native of a compiled program.
const NATIVES: &str = "A compiled program calls a native by the address of its handler and \
     states no hash of it: a native is named only where a names list gives the hash of the handler \
     at that address (a table of `hash` and `rva`) and a natives list names the hash; any other \
     shows as `rva_` and the address.";

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

/// What `script <src>` prints of a program.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum What {
    #[default]
    Summary,
    Natives,
    Strings,
    Functions,
}

/// The arguments after the verb.
#[derive(Default)]
struct Asked {
    paths: Vec<String>,
    what: What,
    json: bool,
    functions: Vec<String>,
    level: Option<Level>,
    /// The passes `--no` leaves off.
    off: Vec<Pass>,
    project: Option<PathBuf>,
    out: Option<PathBuf>,
    names: Vec<PathBuf>,
    natives: Vec<PathBuf>,
    keys: Option<PathBuf>,
    entry: Option<String>,
    threads: Option<usize>,
    help: bool,
    /// The first thing wrong with the arguments, were they for a GTA VI script.
    wrong: Option<String>,
}

/// Runs `script` where its arguments name a compiled GTA VI script; `None` leaves them to the command for GTA V's.
pub fn run(args: &[String]) -> Option<ExitCode> {
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
        eprint!("{}", usage());
        return Some(ExitCode::SUCCESS);
    }
    if verb == Verb::Passes {
        for pass in Pass::ALL {
            println!("{:<16}{}", pass.name(), pass.level().name());
        }
        return Some(ExitCode::SUCCESS);
    }
    let Some(first) = asked.paths.first().map(Path::new) else {
        eprint!("{}", usage());
        return Some(ExitCode::from(2));
    };
    if !ours(verb, first) {
        return None;
    }
    if let Some(wrong) = &asked.wrong {
        eprintln!("Error: {wrong}");
        return Some(ExitCode::from(2));
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
        None => {
            print!("{text}");
            Ok(clean)
        }
    });
    Some(match put {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    })
}

/// Whether the first path named makes the arguments this command's: by the file's first bytes, never by its name.
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
            // GTA V's listing has a column of bytes to leave out, and this one has none.
            "--no-bytes" => {}
            "--natives-db" | "--names" | "--keys" | "--entry" | "--function" | "--level"
            | "--no" | "--project" | "--threads" | "-o" => {
                let Some(value) = iter.next() else {
                    wrong.get_or_insert(format!("{arg} needs a value"));
                    continue;
                };
                let unknown = |kind: &str, all: &[&str]| {
                    format!("unknown {kind} `{value}`; it is one of: {}", all.join(", "))
                };
                let mut refused = None;
                match arg.as_str() {
                    "--natives-db" => asked.natives.push(PathBuf::from(value)),
                    "--names" => asked.names.push(PathBuf::from(value)),
                    "--keys" => asked.keys = Some(PathBuf::from(value)),
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
                    _ => match value.parse::<usize>().ok().filter(|threads| *threads > 0) {
                        Some(threads) => asked.threads = Some(threads),
                        None => {
                            let what = format!("--threads takes a number above 0, not `{value}`");
                            refused = Some(what);
                        }
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

/// Where names lists are read from when none is named, under the configuration directory the app also uses.
pub fn names_dir(config_root: &Path) -> PathBuf {
    config_root.join(NAMES_FOLDER)
}

/// The names lists named, each of which must read; else every file of [`names_dir`] that reads as one, in the order of their names. With them, the natives list naming most of the hashes they give handlers' addresses.
fn supplied_names(asked: &Asked) -> Result<Names, String> {
    let mut names = match asked.names.is_empty() {
        false => super::lifted::names_from(&asked.names)?,
        true => {
            let folder = or_title::config::config_dir().map(|root| names_dir(&root));
            folder.map_or_else(Names::default, |folder| names_in(&folder))
        }
    };
    // A natives list names by hash: with no hash of any handler it would name nothing.
    if !names.native_hashes.is_empty() {
        let lists = super::load_lists(asked.natives.clone())?;
        let hashes: BTreeSet<u64> = names.native_hashes.values().copied().collect();
        let best = Names::best_natives(lists.dbs(), &hashes);
        names.natives = best.map(|(list, _)| list.clone());
    }
    Ok(names)
}

/// The names lists of a folder; a file of it that is no names list is passed over, since the app keeps natives lists there too.
fn names_in(folder: &Path) -> Names {
    let entries = std::fs::read_dir(folder).into_iter().flatten().flatten();
    let mut files: Vec<PathBuf> = entries.map(|entry| entry.path()).collect();
    files.sort();
    let mut names = Names::default();
    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let label = path.file_name().map(|name| name.to_string_lossy());
        // ⚠ A list that is refused leaves the lines before its bad one read, so each is tried on a copy.
        let mut tried = names.clone();
        if tried.read_list(&label.unwrap_or_default(), &text).is_ok() {
            names = tried;
        }
    }
    names
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
}

/// One verb on one program.
fn one(verb: Verb, asked: &Asked) -> Result<String, String> {
    let src = source::open(&asked.paths, asked.entry.as_deref(), asked.keys.as_deref())?;
    let program = Program::read(&src.bytes).map_err(|e| format!("{}: {e}", src.read_from))?;
    let mut names = supplied_names(asked)?;
    bind(&mut names, &program);
    match verb {
        // With a project, a function only another program calls starts where the project holds it.
        Verb::Disasm => {
            let project = asked.project.as_deref().map(project_at).transpose()?;
            let program = match &project {
                Some((_, project)) => lift::read(&src.bytes, project).map_err(|e| e.to_string())?,
                None => program,
            };
            listing(&program, &names, &asked.functions)
        }
        Verb::Decompile => {
            let project = asked.project.as_deref().map(project_at).transpose()?;
            let project = project
                .as_ref()
                .map(|(file, project)| (file.as_str(), project));
            let options = options(asked);
            let done = decompiled(&src.bytes, &names, project, options, &asked.functions);
            done.map(|(text, _)| text)
        }
        _ => match asked.what {
            What::Summary => Ok(info(&src, &program, &names, asked.json)),
            What::Natives => natives(&program, asked.json),
            What::Strings => Ok(strings(&program, asked.json)),
            What::Functions => Ok(functions(&program, &names, asked.json)),
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

/// The functions `asked` names, by rising offset: `0x` or `func_` and an offset, or a name as `--functions` prints it.
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
                program.functions.iter().filter(named).collect()
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
fn walked(program: &Program) -> Result<Walk, disasm::DisasmError> {
    let mut walk = Walk::default();
    for row in &program.functions {
        let mut at = row.at;
        while at < row.end {
            let raw = disasm::raw(&program.code, at)?;
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
                    return Err(disasm::DisasmError::Malformed { at, what });
                }
            };
        }
    }
    Ok(walk)
}

fn line(out: &mut String, key: &str, value: impl Display) {
    let _ = writeln!(out, "{key:<18}{value}");
}

/// What lists named things, as the first lines of every output say it.
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

/// What `--natives` prints: each handler the code calls, by its address.
fn natives(program: &Program, json: bool) -> Result<String, String> {
    let walk = walked(program);
    let walk =
        walk.map_err(|e| format!("program {} does not decode: {e}", hex(program.program)))?;
    if json {
        let rows = walk.natives.iter().map(|(rva, (args, returns, sites))| {
            json!({ "rva": hex(*rva), "args": args, "returns": returns, "sites": sites })
        });
        return Ok(format!("{:#}\n", Value::Array(rows.collect())));
    }
    let mut out = String::new();
    for (rva, (args, returns, sites)) in &walk.natives {
        let _ = writeln!(out, "rva_{rva:08X}  {args} -> {returns}  {sites} site(s)");
    }
    let sites: usize = walk.natives.values().map(|native| native.2).sum();
    let _ = writeln!(
        out,
        "\n{} distinct native(s) across {sites} call site(s), with the cells each takes and \
         returns; the native table holds {} word(s), none of them a native's hash.",
        walk.natives.len(),
        program.natives.len()
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
        out.push_str(" The reader states no string of them.");
    }
    out.push('\n');
    out
}

/// What `--functions` prints: each function by offset, its bytes, its cells in and out, its name.
fn functions(program: &Program, names: &Names, json: bool) -> String {
    let rows = program.functions.iter();
    if json {
        let rows = rows.map(|row| {
            json!({
                "at": format!("0x{:06X}", row.at),
                "bytes": row.end.saturating_sub(row.at),
                "params": row.params,
                "locals": row.locals,
                "returns": row.returns,
                "name": function_name(program, names, row),
            })
        });
        return format!("{:#}\n", Value::Array(rows.collect()));
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
    out
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
        disasm::write_function(&mut body, program, row, names)
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

/// The first lines of a source: what was read, what named things, with what project, at what level.
// ⚠ No line of it may start `// import `: a listing's reader takes that for an import, and the
// output of the lowest level is read back as a listing.
fn header(
    program: &Program,
    names: &Names,
    project: Option<&str>,
    options: Options,
    only: &[FunctionRow],
) -> String {
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
    let _ = match project {
        Some(project) => writeln!(out, "// project: {project}"),
        None => writeln!(
            out,
            "// project: none: the program was decompiled alone, without what other programs \
             state of the types and functions it uses"
        ),
    };
    let off = left_off(options);
    let off = match off.is_empty() {
        true => "none".to_owned(),
        false => off.join(", "),
    };
    let _ = writeln!(
        out,
        "// level: {}; passes left off: {off}",
        options.level.name()
    );
    if !only.is_empty() {
        let only: Vec<String> = only
            .iter()
            .map(|row| function_name(program, names, row))
            .collect();
        let _ = writeln!(out, "// only: {}", only.join(", "));
    }
    if let Some((at, name)) = disasm::first_unseen(&program.code) {
        let _ = writeln!(
            out,
            "// unproven: instruction {at:06X} is `{name}`, an opcode no program was seen to \
             hold: its length is its family's, and where each function after it starts rests \
             on that"
        );
    }
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

/// The source of the program `bytes` hold under its header, and what decompiling it counted; only the functions `only` names where it names any.
// `project` is the project and the phrase the header names it by; without one the program is decompiled alone.
pub fn decompiled(
    bytes: &[u8],
    names: &Names,
    project: Option<(&str, &Project)>,
    options: Options,
    only: &[String],
) -> Result<(String, Stats), String> {
    let held = project.map_or(Project::empty(), |(_, project)| project);
    let program = lift::read(bytes, held).map_err(|e| e.to_string())?;
    let rows = selected(&program, names, only)?;
    let cx = Context::new(names, held, options);
    let asked = match rows.is_empty() {
        true => program.functions.as_slice(),
        false => rows.as_slice(),
    };
    let output = lift::functions(&program, asked, &cx);
    let phrase = project.map(|(phrase, _)| phrase);
    let mut text = header(&program, names, phrase, options, &rows);
    if project.is_none() {
        text.push_str(&left_out(&program, names, asked));
    }
    text.extend(output.files.iter().map(|file| file.text.as_str()));
    Ok((text, output.stats))
}

/// What `check` prints, and whether every script decodes; a folder stands for every file under it that starts as a program does.
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
    let (mut out, mut clean) = (String::new(), 0);
    for path in &files {
        // ⚠ Only a file that starts as a program does is read: an archive is tens of gigabytes.
        let outcome = match File::open(path).and_then(source::head_of) {
            Ok(Head::Program) => std::fs::read(path)
                .map_err(|e| format!("read: {e}"))
                .and_then(|bytes| checked(&bytes)),
            Ok(Head::Archive) => Err(
                "a GTA VI archive: `script list` names its programs, and --entry reads one"
                    .to_owned(),
            ),
            Ok(Head::Text) => Err(source::TEXT.to_owned()),
            Ok(Head::Other) => Err(
                "no deciphered GTA VI program: it does not start with eight zero bytes".to_owned(),
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
    let unseen = disasm::first_unseen(&program.code).map(|(at, name)| {
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
    let names = supplied_names(asked)?;
    let reader = source::Reader::new(asked.keys.as_deref())?;
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

fn usage() -> String {
    let levels = Level::ALL.map(Level::name).join(", ");
    format!(
        "\
Inspect, disassemble or decompile a compiled script: a GTA V .ysc, or a GTA VI program.

Usage: openrage-cli script <src> [--natives|--strings|--statics|--functions] [--json]
       openrage-cli script disasm <src> [--function <f>]... [--no-bytes] [--project <file>]
                                        [-o <file>]
       openrage-cli script decompile <src> [--function <f>]... [--level <name>] [--no <pass>]...
                                           [--project <file>] [-o <file>]
       openrage-cli script check <file-or-folder>...
       openrage-cli script list <archive.rpf> [inner/path] [--json]
       openrage-cli script export <build-folder|archive.rpf|folder> [-o <folder>]
                                  [--level <name>] [--no <pass>]... [--threads <n>]
       openrage-cli script passes
       openrage-cli script download-natives
       openrage-cli script lifted <listing.ysc.cs> [--level <name>] [--no <pass>]... [-o <file>]
       openrage-cli script lifted --census <folder> [-o <folder>]

  <src> is a file, or a GTA VI archive and one entry of it:
          <file.ysc>                               a GTA V script
          <file>                                   a deciphered GTA VI program, told by its
                                                   first bytes: it has no extension
          <archive.rpf> [inner/path] --entry <e>   a GTA VI program in an archive (RPF9)
        <e> is the entry's name, its 0x... or hash_... hash, or its index, as `script list`
        prints them; a name the table does not hold is hashed and matched.

  What is read of a GTA VI program: its container (functions, types with their fields and
  method records, imports, the functions the engine calls by name) and its bytecode. What is
  not: the hash or the name of a native, since the code calls a native by the address of its
  handler and only supplied lists say what is there; initial values of globals and statics,
  since the container holds none. Nothing of a program is ever run.

  disasm      An instruction listing: jump and switch targets resolved, calls named by
              function.
  decompile   Source: functions, expressions, if/else, loops and switches, with a labelled
              goto only where none fits; for a GTA VI program also the classes, awaits,
              lambdas and loops its compiler's expansions prove. A GTA VI program
              decompiled without --project is decompiled alone: a function that needs what
              another program states is written as a comment saying so.
  check       Decode each script and check it against its own file. A folder stands for
              every file under it that starts as a GTA VI program does. Exits non-zero if
              any script does not decode.
  list        The GTA VI programs an archive holds: index, entry hash, size, program hash,
              entry name. Every enciphered entry is deciphered to tell, so a large archive
              takes a while.
  export      Decompile every GTA VI program of a build folder, of an archive or of a folder
              of deciphered programs, and print one line of counts per program and a total.
              With -o <folder> it writes there project.json (what the programs state of
              each other: types, fields, signatures, calls), one .cs per program written
              with it, and scripts.json (what was read, where each program is, with which
              lists, level and passes). A program that fails is reported and the export
              goes on; it exits non-zero if any did.
  passes      The passes and the level each belongs to, one per line, as --no and --level
              take them.
  lifted      Readable source from a lifted listing of a GTA VI script, the text another
              decompiler writes from its bytecode; `script lifted --help` says what it takes.
  --natives   The natives the script calls: by hash for a GTA V script, named where a list
              does; by handler address for a GTA VI program, with the cells each takes and
              returns.
  --strings   The script's strings, one per line; for a GTA VI program each under its offset.
  --statics   A GTA V script's initial static values; the leading ones are arguments.
  --functions A GTA VI program's functions, one per line: offset, bytes, cells in and out,
              name.
  --function <f>   Only this function: 0x<offset>, func_<offset> or its name as --functions
                   prints it. Repeat for several. (GTA VI)
  --level <name>   How far the source is taken from the bytecode: {levels}.
                   Default: source. (GTA VI)
  --no <pass>      Leave one pass off, by the name `script passes` gives it; the code stays
                   in the lower form. Repeat for several. (GTA VI)
  --project <file> The project.json an export wrote: what the other programs state of the
                   types, fields and functions this one uses. (GTA VI)
  --threads <n>    The threads an export works on at most. Default: 8.
  --json      The same facts as JSON, for a program that reads them. (GTA VI)
  -o <path>   Write to a file (a folder for export) instead of printing. (GTA VI)

Names:
  --natives-db <path>   A natives list, JSON or `0xHASH NAME` text. Repeat for one list per
                        build; each script takes the list naming most of its natives.
                        Without the flag, OPENRAGE_NATIVES is read as ;-separated paths,
                        and without either, what download-natives fetched. A natives list
                        names by hash: it names a native of a GTA VI program only where a
                        names list gives the hash of the handler at the address called.
  --names <path>        Names of a GTA VI program's types, programs, methods and fields: one
                        `0xHASH name` per line, or a tab-separated table. A table whose
                        first columns are `hash` and `rva` gives each native's hash by
                        the address of its handler. Repeat for several. Without the flag,
                        every file of the `{NAMES_FOLDER}` folder of the configuration
                        directory that reads as one is read.
  download-natives      Fetch alloc8or's natives lists and CSY0N's crossmap from GitHub into
                        the configuration directory, recording each file's commit. Runs only
                        when you ask. They name GTA V's natives and none of GTA VI's.
  ⚠ OpenRage ships no natives list and no names. A hash no list names stays a hash, and the
    first lines of every output say which lists named things.

GTA VI archives:
  --keys <snapshot>     The key snapshot, as `rpf9` takes it; without the flag,
                        OPENRAGE_GTA6_KEYS or the snapshot beside the keys cache.
  Entry names come from OPENRAGE_GTA6_NAMES and the names the app harvested; without them
  an entry is named by its hash.
"
    )
}

#[cfg(test)]
#[path = "gta6_script_tests.rs"]
mod tests;
