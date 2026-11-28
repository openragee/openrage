//! `script export`: every program of a build folder, an archive or a folder decompiled with the
//! project of them all, counted a line each, and written into one folder where one is named, with
//! an index of its sources a person reads.
//!
//! ⚠ The same input, lists and options write the same bytes: no file holds a time, an absolute
//! path or a machine's name.

use std::cmp::Reverse;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use or_io::gta6::script::program::Program;
use or_io::gta6::script::project::index;
use or_io::gta6::script::{Names, Options, Project, Stats, CONSTRUCTS};
use serde::Serialize;

use super::super::lifted::{over, THREADS};
use super::source::{self, Body};
use super::{Asked, Decompile, PROJECT_FILE};

const SCRIPTS_FILE: &str = "scripts.json";

/// The file an export writes its index of sources to, for a person to read.
const INDEX_FILE: &str = "index.md";

/// The functions not lifted an index row names at most; the rest it counts.
const NAMED_UNLIFTED: usize = 16;

/// The words a class head may carry before `class`.
const CLASS_MODIFIERS: [&str; 8] = [
    "public",
    "private",
    "protected",
    "internal",
    "static",
    "sealed",
    "abstract",
    "partial",
];

/// What `scripts.json` holds: what was read, with what, and where each program is.
#[derive(Serialize)]
struct Scripts<'a> {
    format: u32,
    read: &'a str,
    level: &'a str,
    /// The passes that ran: those of the level that are on and whose names a pass reads.
    passes: Vec<&'a str>,
    /// The passes left off.
    off: Vec<&'a str>,
    project: &'a str,
    lists: &'a [String],
    programs: Vec<Row>,
}

#[derive(Serialize)]
struct Row {
    /// The source written; none for a body that failed.
    file: Option<String>,
    program: Option<String>,
    name: Option<String>,
    read: String,
    archive: Option<String>,
    entry: Option<String>,
    functions: usize,
    /// The classes its source declares, nested ones too; 0 for a body that failed.
    classes: usize,
    /// The functions its source writes with no body, by the names their comments give them.
    not_lifted: Vec<String>,
    imports: Vec<String>,
    status: String,
}

/// What one program's source holds beside what decompiling it counted.
struct Wrote {
    stats: Stats,
    classes: usize,
    unlifted: Vec<String>,
}

/// What the first pass keeps of one program.
pub(super) struct Stated {
    /// The program's tables; its code is dropped.
    program: Program,
    /// Why the program gave no part of the project, where it gave none.
    left_out: Option<String>,
}

pub(super) fn run(asked: &Asked) -> Result<(String, bool), String> {
    let [root] = asked.paths.as_slice() else {
        return Err(
            "name one build folder, one archive or one folder of deciphered programs".to_owned(),
        );
    };
    let (read, found) = source::gathered(Path::new(root), asked.keys())?;
    let unread = format!(
        "{} enciphered entr(ies) or archive(s) could not be read",
        found.unread
    );
    if found.bodies.is_empty() {
        return Err(format!("{root} holds no GTA VI program; {unread}"));
    }
    let lists = super::supplied_lists(asked)?;
    let said = lists.said();
    let asks = Asks {
        options: super::options(asked),
        threads: asked.threads.unwrap_or(THREADS),
        said: &said,
        summary: asked.summary,
    };
    let out = asked.out.as_deref();
    let (mut lines, failed) = exported(&found.bodies, read, lists.names, asks, out)?;
    if found.unread > 0 {
        lines.push(unread);
    }
    if found.passed_over > 0 {
        lines.push(format!(
            "{} file(s) under {root} do not start as a GTA VI program does and were passed over",
            found.passed_over
        ));
    }
    lines.push(String::new());
    Ok((lines.join("\n"), failed == 0))
}

/// How an export decompiles each program.
#[derive(Clone, Copy)]
pub struct Asks<'a> {
    pub options: Options,
    /// The threads worked on at most.
    pub threads: usize,
    /// More lines for the header of each source, each without its `// `.
    pub said: &'a [String],
    /// `--summary`: the programs that failed and a table of the totals, in place of a line a program.
    pub summary: bool,
}

/// Decompiles every body with the project of them all, and writes that project, a source each, what was read and an index into `out` where one is given; a line a body and a total, and how many bodies failed.
// ⚠ A body that does not read or does not decompile is a line and a row, never the end.
pub fn exported(
    bodies: &[Body],
    read: &str,
    mut names: Names,
    asks: Asks<'_>,
    out: Option<&Path>,
) -> Result<(Vec<String>, usize), String> {
    let (options, threads) = (asks.options, asks.threads);
    let write = |file: &str, bytes: &[u8]| match out {
        Some(dir) => {
            let to = dir.join(file);
            std::fs::write(&to, bytes).map_err(|e| format!("write {}: {e}", to.display()))
        }
        None => Ok(()),
    };
    if let Some(dir) = out {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }

    let (project, stated) = built(bodies, &mut names, threads, &|_| {});
    write(
        PROJECT_FILE,
        &index::save(&project).map_err(|e| e.to_string())?,
    )?;

    let mut used = BTreeSet::new();
    let files: Vec<Option<String>> = (stated.iter())
        .map(|stated| {
            let stem = stem_of(&names, stated.as_ref().ok()?.program.program);
            // A second program of one name takes `_2`, in the order the programs were found.
            let (mut file, mut n) = (format!("{stem}.cs"), 1);
            while !used.insert(file.to_ascii_lowercase()) {
                n += 1;
                file = format!("{stem}_{n}.cs");
            }
            Some(file)
        })
        .collect();

    // The second pass: each program decompiled with the project.
    let phrase = match out {
        Some(_) => format!("`{PROJECT_FILE}`"),
        None => "that of the programs read with it, kept in no file".to_owned(),
    };
    let jobs: Vec<Job<'_>> = bodies.iter().zip(&stated).zip(&files).collect();
    let outcomes = largest_first(&jobs, job_bytes, threads, |((body, stated), file)| {
        if let Err(why) = stated {
            return Err(why.clone());
        }
        guarded(|| {
            let ask = Decompile {
                names: &names,
                project: Some(&project),
                project_said: &phrase,
                options,
                only: &[],
                with_classes: false,
                said: asks.said,
            };
            let (text, stats) = super::decompiled(&body.bytes, &ask)?;
            if let Some(file) = file {
                write(file, text.as_bytes())?;
            }
            Ok(Wrote {
                stats,
                classes: classes_in(&text),
                unlifted: unlifted_in(&text),
            })
        })
    });

    let (mut lines, mut total) = (Vec::new(), Stats::default());
    let (mut failed, mut alone, mut classes) = (0, 0, 0);
    let mut rows = Vec::with_capacity(bodies.len());
    let mut indexed = Vec::with_capacity(bodies.len());
    for (((body, stated), file), outcome) in jobs.iter().zip(&outcomes) {
        let stated: Option<&Stated> = Result::as_ref(stated).ok();
        let note = match stated.and_then(|stated| stated.left_out.as_ref()) {
            Some(why) => {
                alone += 1;
                format!("; it gave the project no part: {why}")
            }
            None => String::new(),
        };
        let status = match outcome {
            Ok(wrote) => {
                total.add(&wrote.stats);
                classes += wrote.classes;
                if !asks.summary {
                    lines.push(format!("{}: {}{note}", body.label, counts(&wrote.stats)));
                }
                "ok".to_owned()
            }
            Err(why) => {
                failed += 1;
                lines.push(format!("{}: failed: {why}", body.label));
                format!("failed: {why}")
            }
        };
        let program = stated.map(|stated| &stated.program);
        let imports = |p: &Program| -> Vec<String> {
            let imports = p.imports.iter();
            imports.map(|import| super::hex(import.program)).collect()
        };
        let wrote = outcome.as_ref().ok();
        rows.push(Row {
            file: (*file).clone().filter(|_| outcome.is_ok()),
            program: program.map(|p| super::hex(p.program)),
            name: program
                .and_then(|p| names.program(u64::from(p.program)))
                .map(str::to_owned),
            read: body.label.clone(),
            archive: body.archive.clone(),
            entry: (body.entry.as_ref()).map(|entry| format!("0x{:016x}", entry.name64)),
            functions: program.map_or(0, |p| p.functions.len()),
            classes: wrote.map_or(0, |wrote| wrote.classes),
            not_lifted: wrote.map_or_else(Vec::new, |wrote| wrote.unlifted.clone()),
            imports: program.map_or_else(Vec::new, imports),
            status,
        });
        if let Some(row) = rows.last() {
            indexed.push(index_row(row, outcome));
        }
    }
    let last = format!(
        "total: {} program(s), {failed} failed, {alone} that gave the project no part: {}",
        bodies.len(),
        counts(&total)
    );
    match asks.summary {
        true => lines.extend(table(bodies.len(), failed, alone, classes, &total)),
        false => lines.push(last.clone()),
    }

    let scripts = Scripts {
        format: 1,
        read,
        level: options.level.name(),
        passes: super::ran(options),
        off: super::left_off(options),
        project: PROJECT_FILE,
        lists: &names.lists,
        programs: rows,
    };
    let mut text = serde_json::to_vec_pretty(&scripts).map_err(|e| e.to_string())?;
    text.push(b'\n');
    write(SCRIPTS_FILE, &text)?;
    let names_said = super::names_sentence(&names);
    write(
        INDEX_FILE,
        index(&scripts, &names_said, &indexed, &last).as_bytes(),
    )?;
    Ok((lines, failed))
}

/// What `index.md` holds: how the sources were written, a row a program, and the export's last line.
fn index(scripts: &Scripts<'_>, names: &str, rows: &[String], last: &str) -> String {
    let off = match scripts.off.is_empty() {
        true => "none".to_owned(),
        false => scripts.off.join(", "),
    };
    let mut out = String::new();
    let _ = writeln!(out, "# GTA VI scripts\n");
    let _ = writeln!(
        out,
        "{} program(s) read from {}, each decompiled at the {} level, passes off: {off}, with \
         `{}`, the project of them all. `{SCRIPTS_FILE}` says where each program was read from.\n",
        scripts.programs.len(),
        scripts.read,
        scripts.level,
        scripts.project
    );
    let _ = writeln!(out, "Names: {names}.\n");
    let _ = writeln!(
        out,
        "| Source | Program | Name | Functions | Classes | Not lifted |"
    );
    let _ = writeln!(out, "|---|---|---|--:|--:|---|");
    for row in rows {
        let _ = writeln!(out, "{row}");
    }
    let _ = writeln!(out, "\n{last}");
    out
}

/// One program's row of the index: its source, hash and name, its functions and classes, and what was not lifted, or why it failed.
fn index_row(row: &Row, outcome: &Result<Wrote, String>) -> String {
    let none = || "-".to_owned();
    let linked = |file: &str| format!("[{}]({file})", code(file));
    let source = row.file.as_deref().map_or_else(none, linked);
    let program = row.program.clone().unwrap_or_else(none);
    let name = row.name.as_deref().map_or_else(none, code);
    let (functions, classes, missing) = match outcome {
        Ok(wrote) => (
            wrote.stats.functions.to_string(),
            wrote.classes.to_string(),
            unlifted(wrote),
        ),
        Err(why) => (
            row.functions.to_string(),
            none(),
            format!("failed: {}", code(why)),
        ),
    };
    format!("| {source} | {program} | {name} | {functions} | {classes} | {missing} |")
}

/// What an index row says was not lifted: none, else how many and the names the source gives them.
// ⚠ The count is the decompile's, the names the source's comments: at the lowest level a
// function with no body is a head over a comment that names none.
fn unlifted(wrote: &Wrote) -> String {
    let count = wrote.stats.incomplete;
    if count == 0 {
        return "none".to_owned();
    }
    let named = wrote.unlifted.iter().take(NAMED_UNLIFTED);
    let named: Vec<String> = named.map(|name| code(name)).collect();
    let mut out = format!("{count}");
    if !named.is_empty() {
        let _ = write!(out, ": {}", named.join(", "));
    }
    let more = wrote.unlifted.len().saturating_sub(NAMED_UNLIFTED);
    if more > 0 {
        let _ = write!(out, " and {more} more");
    }
    out
}

/// `text` as a code span of a table cell: on one line, each `|` escaped, between fences longer than any run of backticks in it.
fn code(text: &str) -> String {
    let text = text.replace(['\n', '\r'], " ").replace('|', "\\|");
    let (mut longest, mut run) = (0, 0);
    for c in text.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat(longest + 1);
    // A span that starts or ends on a backtick needs a space between it and the fence.
    let pad = match text.starts_with('`') || text.ends_with('`') {
        true => " ",
        false => "",
    };
    format!("{fence}{pad}{text}{pad}{fence}")
}

/// How many classes `text` declares: each line that is a class head as the printer writes one, with a `{` alone under it at its indent.
// A head too long for its line goes on over lines indented under it, each but the last ending on
// a comma.
fn classes_in(text: &str) -> usize {
    let mut lines = text.lines().peekable();
    let mut classes = 0;
    while let Some(line) = lines.next() {
        let head = line.trim_start();
        let indent = &line[..line.len() - head.len()];
        if !class_head(head) {
            continue;
        }
        let deeper = |next: &str| {
            let rest = next.strip_prefix(indent);
            rest.is_some_and(|rest| rest.starts_with("    "))
        };
        if head.trim_end().ends_with(',') {
            while lines.peek().is_some_and(|next| deeper(next)) {
                let last = lines
                    .next()
                    .is_some_and(|next| !next.trim_end().ends_with(','));
                if last {
                    break;
                }
            }
        }
        let opens = (lines.peek()).is_some_and(|next| next.strip_prefix(indent) == Some("{"));
        classes += usize::from(opens);
    }
    classes
}

/// Whether `head` is a class head: modifiers, `class`, a name, and what it derives from; no statement.
fn class_head(head: &str) -> bool {
    // A listing's own syntax ends a head with its offset as a comment.
    let head = head.split(" //").next().unwrap_or(head);
    if head.contains(['(', '=', ';']) {
        return false;
    }
    let mut words = head.split_whitespace();
    let class = words.find(|word| !CLASS_MODIFIERS.contains(word));
    let name = words.next().map(|name| name.trim_end_matches(':'));
    class == Some("class") && name.is_some_and(is_name)
}

fn is_name(word: &str) -> bool {
    !word.is_empty() && word.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// The functions `text` writes with no body, by the name each comment that stands for one gives it, in the order written.
// The two shapes the printer writes: `// func_000040: no body (why)` and `// name at 0x000040: the
// listing holds no body (why)`.
fn unlifted_in(text: &str) -> Vec<String> {
    let named = |line: &str| -> Option<String> {
        let comment = line.trim_start().strip_prefix("// ")?;
        let (name, rest) = comment.split_once(": ")?;
        let name = match name.split_once(" at 0x") {
            Some((name, at)) => {
                let hex = !at.is_empty() && at.bytes().all(|b| b.is_ascii_hexdigit());
                (hex && rest.starts_with("the listing holds no body (")).then_some(name)?
            }
            None => rest.starts_with("no body (").then_some(name)?,
        };
        is_name(name).then(|| name.to_owned())
    };
    text.lines().filter_map(named).collect()
}

/// The counts of the export's last line, a row each, for `--summary`.
fn table(programs: usize, failed: usize, alone: usize, classes: usize, s: &Stats) -> Vec<String> {
    let mut rows: Vec<(String, usize)> = [
        ("programs", programs),
        ("failed", failed),
        ("gave the project no part", alone),
        ("functions", s.functions),
        ("classes", classes),
        ("not lifted", s.incomplete),
        ("statements in", s.statements_in),
        ("statements out", s.statements_out),
        ("gotos left", s.gotos_out),
        ("temporaries in", s.temporaries_in),
        ("temporaries out", s.temporaries_out),
        ("native calls", s.native_calls),
        ("native calls named", s.natives_named),
    ]
    .into_iter()
    .map(|(what, count)| (what.to_owned(), count))
    .collect();
    let read = CONSTRUCTS.iter().zip(s.constructs);
    rows.extend(
        read.filter(|(_, count)| *count > 0)
            .map(|(kind, count)| (format!("constructs read: {kind}"), count)),
    );
    let wide = rows.iter().map(|(what, _)| what.len()).max().unwrap_or(0);
    let digits = rows
        .iter()
        .map(|(_, n)| n.to_string().len())
        .max()
        .unwrap_or(1);
    let rows = rows.iter();
    rows.map(|(what, n)| format!("{what:<wide$}  {n:>digits$}"))
        .collect()
}

/// Reads one program, and its part of the project where it gives one.
// A program whose part is refused is still decompiled, with what the other programs state.
fn first(bytes: &[u8], tables: &index::Tables) -> Result<(Stated, Option<Project>), String> {
    let mut program = Program::read(bytes).map_err(|e| e.to_string())?;
    // Only its tables are kept: the code of every program at once is the size of the build's scripts.
    program.code = Vec::new();
    let (part, left_out) = match index::of_program_in(bytes, tables) {
        Ok(part) => (Some(part), None),
        Err(why) => (None, Some(why.to_string())),
    };
    Ok((Stated { program, left_out }, part))
}

/// One program of the second pass: its body, what the first pass kept of it, and the file its source goes to.
type Job<'a> = ((&'a Body, &'a Result<Stated, String>), &'a Option<String>);

fn job_bytes(job: &Job<'_>) -> usize {
    job.0 .0.bytes.len()
}

/// The project of `bodies` as an export builds it, and what the first pass kept of each body or why it read as no program; every program's records are named in `names` first, in the order of the bodies.
// `said` is told a line as each pass starts and as about each tenth of its programs is done.
pub(super) fn built(
    bodies: &[Body],
    names: &mut Names,
    threads: usize,
    said: &(dyn Fn(String) + Sync),
) -> (Project, Vec<Result<Stated, String>>) {
    let size = |body: &Body| body.bytes.len();
    // The first pass: what the programs state of each other, then what the code of each shows.
    let tables = or_io::gta6::script::par::pooled(threads, || {
        index::tables(bodies.iter().map(|body| body.bytes.as_slice()))
    });
    let counted = Counted::new("first reading", bodies.len(), said);
    let firsts = pooled_largest_first(bodies, size, threads, |body| {
        let one = guarded(|| first(&body.bytes, &tables));
        counted.one();
        one
    });
    let mut parts = Vec::new();
    let mut stated = Vec::with_capacity(bodies.len());
    for (one, body) in firsts.into_iter().zip(bodies.iter()) {
        // ⚠ A program that gives no part still runs its code, which may call into any other.
        if !matches!(&one, Ok((_, Some(_)))) {
            let unread = guarded(|| Ok(index::unread_part(&body.bytes)));
            parts.extend(unread.ok().flatten());
        }
        stated.push(one.map(|(kept, part)| {
            // A type's members are named in every program that uses the type, so all are bound before any is written.
            super::bind(names, &kept.program);
            // A word with no hash names a program only where the entry's 64-bit hash is of the same name.
            if let Some(entry) = &body.entry {
                names.bind_program(kept.program.program, entry.name64);
            }
            parts.extend(part);
            kept
        }));
    }
    let mut project = index::merge(parts);
    // What the code shows through the variables and receivers the whole project types.
    let counted = Counted::new("second reading with the project", bodies.len(), said);
    let typed = pooled_largest_first(bodies, size, threads, |body| {
        let one = guarded(|| {
            index::typed_part(&body.bytes, &tables, &project).map_err(|e| e.to_string())
        });
        counted.one();
        one
    });
    index::refine(
        &mut project,
        typed.into_iter().filter_map(Result::ok).collect(),
    );
    (project, stated)
}

/// The project of `bodies` built as [`built`] builds it, kept in memory, with how many of the bodies gave it no part.
pub(super) fn in_memory(
    bodies: &[Body],
    names: &mut Names,
    threads: usize,
    said: &(dyn Fn(String) + Sync),
) -> (Project, usize) {
    let (project, stated) = built(bodies, names, threads, said);
    let gave = |one: &Result<Stated, String>| one.as_ref().is_ok_and(|s| s.left_out.is_none());
    let none = stated.iter().filter(|one| !gave(one)).count();
    (project, none)
}

/// A count of the programs a pass has done, told to `said` at about each tenth of them.
struct Counted<'a> {
    pass: &'static str,
    all: usize,
    step: usize,
    done: AtomicUsize,
    said: &'a (dyn Fn(String) + Sync),
}

impl<'a> Counted<'a> {
    fn new(pass: &'static str, all: usize, said: &'a (dyn Fn(String) + Sync)) -> Self {
        said(format!("{pass}: {all} program(s) to read"));
        Counted {
            pass,
            all,
            step: (all / 10).max(1),
            done: AtomicUsize::new(0),
            said,
        }
    }

    fn one(&self) {
        let done = self.done.fetch_add(1, Ordering::Relaxed) + 1;
        if done.is_multiple_of(self.step) || done == self.all {
            (self.said)(format!(
                "{}: {done} of {} program(s) done",
                self.pass, self.all
            ));
        }
    }
}

/// What `each` gives for every item, in the items' order, the items of most `size` started first.
// ⚠ An item is one thread's from start to end: the largest started last would run alone after the rest.
fn largest_first<I: Sync, T: Send>(
    items: &[I],
    size: impl Fn(&I) -> usize,
    threads: usize,
    each: impl Fn(&I) -> T + Sync,
) -> Vec<T> {
    let mut order: Vec<usize> = (0..items.len()).collect();
    // Stable, so that items of one size keep their own order.
    order.sort_by_key(|n| Reverse(items.get(*n).map_or(0, &size)));
    let done = over(&order, threads, |n| items.get(*n).map(&each));
    let mut placed: Vec<Option<T>> = (0..items.len()).map(|_| None).collect();
    for (n, result) in order.iter().zip(done) {
        if let Some(place) = placed.get_mut(*n) {
            *place = result;
        }
    }
    // Every place was given its item's result once, so none is left out.
    placed.into_iter().flatten().collect()
}

/// What `each` gives for every item, in the items' order, the items of most `size` started first on a pool of `threads` threads, which the functions of one item spread over too.
fn pooled_largest_first<I: Sync, T: Send>(
    items: &[I],
    size: impl Fn(&I) -> usize + Sync + Send,
    threads: usize,
    each: impl Fn(&I) -> T + Sync + Send,
) -> Vec<T> {
    or_io::gta6::script::par::pooled(threads, || {
        let mut order: Vec<usize> = (0..items.len()).collect();
        // Stable, so that items of one size keep their own order.
        order.sort_by_key(|n| Reverse(items.get(*n).map_or(0, &size)));
        let done = or_io::gta6::script::par::map(order.clone(), |n| items.get(n).map(&each));
        let mut placed: Vec<Option<T>> = (0..items.len()).map(|_| None).collect();
        for (n, result) in order.iter().zip(done) {
            if let Some(place) = placed.get_mut(*n) {
                *place = result;
            }
        }
        // Every place was given its item's result once, so none is left out.
        placed.into_iter().flatten().collect()
    })
}

/// What `work` gives, a fault in it turned into its body's failure.
fn guarded<T>(work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    match catch_unwind(AssertUnwindSafe(work)) {
        Ok(done) => done,
        Err(_) => Err("the decompiler panicked; its message is on stderr".to_owned()),
    }
}

/// The name a program's source is written under: its name where a list names it, else its hash.
fn stem_of(names: &Names, program: u32) -> String {
    // ⚠ A name is the user's text: nothing of it may leave the folder or open a stream of a file.
    let kept = |c: char| match c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.') {
        true => c,
        false => '_',
    };
    match names.program(u64::from(program)) {
        Some(name) => name.chars().map(kept).collect(),
        None => super::hex(program),
    }
}

fn counts(s: &Stats) -> String {
    format!(
        "{} function(s), {} not lifted; statements {} -> {}; {} goto(s) left; temporaries {} -> \
         {}; {} of {} native call(s) named{}",
        s.functions,
        s.incomplete,
        s.statements_in,
        s.statements_out,
        s.gotos_out,
        s.temporaries_in,
        s.temporaries_out,
        s.natives_named,
        s.native_calls,
        super::super::lifted::constructs(s)
    )
}

#[cfg(test)]
#[path = "gta6_script_export_tests.rs"]
mod tests;
