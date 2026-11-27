//! A compiled GTA VI script opened read-only: the source it decompiles to, the classes and
//! functions that source lists, and the program it was read from.
//!
//! ⚠ **Every name is the user's.** OpenRage ships none: a native, a type, a field or a method
//! reads under a name only where a list in [`names_dir`] names it.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use or_io::gta6::script::lift::Progress;
use or_io::gta6::script::program::{FunctionRow, Program};
use or_io::gta6::script::{lift, Context, Level, Options, Stats};

use super::{DocPayload, DocumentId, HexDocument};
use crate::state::AppState;

#[path = "gta6_script_build.rs"]
mod build;
#[path = "gta6_script_jobs.rs"]
mod jobs;
#[path = "gta6_script_lists.rs"]
mod lists;

pub(crate) use jobs::may_be_script;
use lists::Stamp;

/// The stack a decompile runs on: a pass recurses as deep as a function's statements nest.
const STACK: usize = 256 << 20;

/// The bytes of code past which a program opens with one function decompiled and the rest picked in the outline.
// ⚠ The load worker opens one file at a time, and the time a decompile takes grows with the
// code: past this, every other open would wait behind this one for minutes.
const WHOLE: usize = 1 << 20;

/// The statement words a line that opens a block may start with, which no head of a class or a function does.
const NOT_A_HEAD: &[&str] = &[
    "if",
    "else",
    "while",
    "for",
    "foreach",
    "do",
    "switch",
    "try",
    "catch",
    "finally",
    "using",
    "lock",
    "case",
    "default",
    "fixed",
    "checked",
    "unchecked",
    "unsafe",
    "new",
    "return",
    "await",
    "throw",
    "yield",
    "when",
    "in",
];

/// The level the next script opens at, which the inspector's choice sets: the load worker sees no panel.
static LEVEL: Mutex<Level> = Mutex::new(Level::Source);

/// The level a script opens at.
pub fn level() -> Level {
    *LEVEL.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Have scripts open at `level` from now on.
pub fn set_level(level: Level) {
    *LEVEL.lock().unwrap_or_else(PoisonError::into_inner) = level;
}

/// Whether the labels files of the names folder are read, which the inspector's choice sets: off at every start, since a label is a guess.
static LABELS: AtomicBool = AtomicBool::new(false);

/// Whether a script is decompiled with the labels files of the names folder.
pub fn labels_on() -> bool {
    LABELS.load(Ordering::Relaxed)
}

/// Have scripts decompiled with the labels files of the names folder, or without them, from now on.
pub fn set_labels(on: bool) {
    LABELS.store(on, Ordering::Relaxed);
}

/// Where natives lists, names lists and a project for GTA VI scripts are read from, in the configuration directory.
pub fn names_dir() -> Option<PathBuf> {
    crate::state::prefs::config_root().map(|root| root.join("gta6_script_names"))
}

thread_local! {
    /// The file the load on this thread reads, in whose folder a project may lie.
    static READING: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Say which file the load on this thread reads: the readers below are handed its bytes alone.
pub(crate) fn reading(path: &Path) {
    READING.with(|held| *held.borrow_mut() = Some(path.to_path_buf()));
}

/// The folder of the file the load on this thread reads.
fn beside() -> Option<PathBuf> {
    let folder = |file: &PathBuf| file.parent().map(Path::to_path_buf);
    READING.with(|held| held.borrow().as_ref().and_then(folder))
}

/// One class or function the source lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineRow {
    pub name: String,
    /// The line of its head, the first line being 0.
    pub line: u32,
    /// Its lines, from its head to its closing brace.
    pub lines: u32,
    /// How many of the rows before it hold it.
    pub depth: u8,
    pub class: bool,
    /// The offset of its code, where its name or the program states one.
    pub at: Option<u32>,
}

/// What a decompile is asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub level: Level,
    /// The offsets of the functions to write; `None` for every function.
    pub picked: Option<BTreeSet<u32>>,
}

/// What named things and what stood beside the program when its source was written.
#[derive(Clone, Debug, Default)]
pub struct Read {
    /// The natives list in use.
    pub natives: Option<String>,
    /// The natives lists the folder holds, in the order of their files' names.
    pub natives_lists: Vec<String>,
    /// The names lists read, in the order read.
    pub lists: Vec<String>,
    /// The files of the folder no reader took, each with the reason.
    pub refused: Vec<String>,
    /// Each names list in the order read, with how many hashes, cells, indices, functions or words it named that no list before it did.
    pub named: Vec<(String, usize)>,
    /// The files of the folder passed over for the guesses their names say they hold.
    pub candidates: Vec<String>,
    /// The files of native labels the folder holds, read only while the user asks.
    pub labels: Vec<String>,
    /// Each labels file read, with how many natives it labelled; empty where none was read.
    pub labelled: Vec<(String, usize)>,
    /// The file of the project the source was written with; `None` for a program decompiled alone.
    pub project: Option<PathBuf>,
    /// The project file that was found and not read, with the reason.
    pub project_refused: Option<String>,
}

/// A compiled GTA VI script as the source it decompiles to, read-only: OpenRage writes no bytecode.
#[derive(Clone)]
pub struct Gta6ScriptDocument {
    pub source: ropey::Rope,
    /// The classes and functions of the source, in the order of its lines.
    pub outline: Vec<OutlineRow>,
    pub program: Arc<Program>,
    /// The level the source was written at.
    pub level: Level,
    pub stats: Stats,
    pub read: Read,
    /// The offsets of the functions the source holds, for a program too large to decompile whole at open; `None` where it holds every function.
    pub picked: Option<BTreeSet<u32>>,
    /// The GTA VI archive the program was read from, which a project can be built from; `None` for a program read from a file of its own.
    pub archive: Option<PathBuf>,
    // ponytail: held beside the program read from them, since a project found later starts
    // functions the program read alone does not; drop them once a `Program` takes a project.
    bytes: Arc<[u8]>,
    /// The folder of the file the program was read from.
    beside: Option<PathBuf>,
    /// The project file `program` was read with.
    stamp: Option<Stamp>,
}

impl Gta6ScriptDocument {
    /// The document of `source`, the text `program` decompiled to at `level`.
    pub fn of(program: Program, source: &str, level: Level) -> Self {
        Self::over(Arc::new(program), source, level)
    }

    fn over(program: Arc<Program>, source: &str, level: Level) -> Self {
        Self {
            outline: outline_of(source, &program),
            source: ropey::Rope::from_str(source),
            program,
            level,
            stats: Stats::default(),
            read: Read::default(),
            picked: None,
            archive: None,
            bytes: Arc::from(Vec::new()),
            beside: None,
            stamp: None,
        }
    }

    /// What the source at hand was asked for.
    pub fn ask(&self) -> Ask {
        Ask {
            level: self.level,
            picked: self.picked.clone(),
        }
    }

    pub fn held_bytes(&self) -> usize {
        self.source.len_bytes()
            + self.bytes.len()
            + self.program.code.len()
            + std::mem::size_of_val(self.outline.as_slice())
    }

    /// The row `name` is defined at: the one head of that name, else the one function at the offset the name states.
    // ⚠ `None` where two rows could be meant: a jump to the wrong one of them reads as the right one.
    pub fn definition(&self, name: &str) -> Option<&OutlineRow> {
        let mut named = self.outline.iter().filter(|row| row.name == name);
        if let Some(first) = named.next() {
            return named.next().is_none().then_some(first);
        }
        let at = stated_offset(name)?;
        let mut there = (self.outline.iter()).filter(|row| !row.class && row.at == Some(at));
        let first = there.next()?;
        there.next().is_none().then_some(first)
    }

    /// The pattern that finds every place spelling what `word` names; `after_dot` is the name before its dot, empty where the dot follows no name.
    // ⚠ A name that stands alone is searched with the character before it, which is never a dot:
    // `other.func_000010` is another program's function, not this one's.
    pub fn references(&self, word: &str, after_dot: Option<&str>) -> String {
        if let Some(before) = after_dot {
            let boundary = if before.is_empty() { "" } else { r"\b" };
            return format!(
                r"{boundary}{}\.{}\b",
                regex::escape(before),
                regex::escape(word)
            );
        }
        let mut spelled = vec![word.to_owned()];
        // A function is called by its slot name even where its head has another.
        if let Some(row) = self.definition(word).filter(|row| !row.class) {
            let slot = row.at.map(slot_name);
            for other in [Some(row.name.clone()), slot].into_iter().flatten() {
                if !spelled.contains(&other) {
                    spelled.push(other);
                }
            }
        }
        let spelled: Vec<String> = spelled.iter().map(|name| regex::escape(name)).collect();
        format!(r"[^.\w](?:{})\b", spelled.join("|"))
    }

    /// The decompile of the same bytes as `ask` asks, on the calling thread: a worker's work.
    pub fn again(&self, ask: &Ask, progress: &Progress) -> Result<Gta6ScriptDocument, String> {
        let held = (Arc::clone(&self.program), self.stamp.clone());
        written(
            held,
            &self.bytes,
            ask,
            self.beside.as_deref(),
            Some(progress),
        )
    }
}

/// The name a function at `at` has where nothing names it.
pub fn slot_name(at: u32) -> String {
    format!("func_{at:06X}")
}

/// The offset a slot name states.
fn stated_offset(name: &str) -> Option<u32> {
    let digits = name.strip_prefix("func_")?;
    let hex = !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_hexdigit());
    hex.then(|| u32::from_str_radix(digits, 16).ok()).flatten()
}

fn is_name(word: &str) -> bool {
    !word.is_empty() && word.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// The name of the function a line is the head of: words, a name, then its parameters between brackets to the line's end.
// ⚠ Every bracket closed: `Add(new Item(1)` over an initialiser's brace is a statement cut in two.
fn function_head(head: &str) -> Option<&str> {
    let (before, after) = head.split_once('(')?;
    if !after.ends_with(')') || head.matches('(').count() != head.matches(')').count() {
        return None;
    }
    let mut words = before.split_whitespace();
    let name = words.next_back()?;
    let typed = |word: &str| {
        let plain = |c: char| c.is_alphanumeric() || "_<>[],.?".contains(c);
        word.chars().all(plain) && !NOT_A_HEAD.contains(&word)
    };
    (is_name(name) && !NOT_A_HEAD.contains(&name) && words.all(typed)).then_some(name)
}

/// The name of the type a line is the head of.
fn class_head(head: &str) -> Option<&str> {
    if head.contains(['(', '=', ';']) {
        return None;
    }
    let mut words = head.split_whitespace();
    words.find(|word| matches!(*word, "class" | "struct" | "interface" | "enum"))?;
    let name = words.next()?.trim_end_matches(':');
    is_name(name).then_some(name)
}

/// The function a comment line stands for where the source gives it no body: its name and its offset.
fn bodiless(line: &str) -> Option<(&str, u32)> {
    let (name, rest) = line.strip_prefix("// ")?.split_once(" at 0x")?;
    let (at, _) = rest.split_once(": ")?;
    let at = u32::from_str_radix(at, 16).ok()?;
    is_name(name).then_some((name, at))
}

/// The classes and functions `text` lists: each line that is a head with an opening brace alone on the line under it, at the head's own indent.
// ⚠ Read from the text, since the decompiler states no place of what it wrote: a head the
// rules below do not prove is left out, never guessed.
fn outline_of(text: &str, program: &Program) -> Vec<OutlineRow> {
    let offset = |name: &str| match name {
        "main" => program.main,
        "initialize" => program.initialize,
        _ => stated_offset(name),
    };
    let mut rows: Vec<OutlineRow> = Vec::new();
    // The rows whose closing brace is still to come, each with the indent of its head.
    let mut open: Vec<(usize, usize)> = Vec::new();
    let mut lines = text.lines().enumerate().peekable();
    while let Some((n, line)) = lines.next() {
        let n = u32::try_from(n).unwrap_or(u32::MAX);
        let body = line.trim_start();
        let indent = line.len() - body.len();
        let depth = u8::try_from(open.len()).unwrap_or(u8::MAX);
        if body == "}" {
            if open.last().is_some_and(|(_, at)| *at == indent) {
                let closed = open.pop().and_then(|(row, _)| rows.get_mut(row));
                if let Some(row) = closed {
                    row.lines = n.saturating_sub(row.line) + 1;
                }
            }
            continue;
        }
        if let Some((name, at)) = bodiless(body) {
            rows.push(OutlineRow {
                name: name.to_owned(),
                line: n,
                lines: 1,
                depth,
                class: false,
                at: Some(at),
            });
            continue;
        }
        let opens =
            (lines.peek()).is_some_and(|(_, next)| next.strip_prefix(&line[..indent]) == Some("{"));
        if !opens {
            continue;
        }
        // A listing's own syntax ends a head with its offset as a comment.
        let head = body.split(" //").next().unwrap_or(body).trim_end();
        let found = match class_head(head) {
            Some(name) => Some((name, true)),
            None => function_head(head).map(|name| (name, false)),
        };
        if let Some((name, class)) = found {
            open.push((rows.len(), indent));
            rows.push(OutlineRow {
                name: name.to_owned(),
                line: n,
                lines: 1,
                depth,
                class,
                at: offset(name).filter(|_| !class),
            });
        }
    }
    rows
}

/// What stands over the lines of the functions a large program's source leaves out.
const STUBS_HEAD: &str =
    "\n// Not decompiled: this program is large, so its source holds only the \
    functions picked in the outline. Each line below is a function still to pick.\n";

/// A line for each function of `program` that `picked` leaves out, in the shape of a function written with no body.
// ⚠ English like the rest of the source, and never translated: the outline reads each line back.
fn stubs(program: &Program, picked: &BTreeSet<u32>) -> String {
    let mut out = String::new();
    for row in (program.functions.iter()).filter(|row| !picked.contains(&row.at)) {
        if out.is_empty() {
            out.push_str(STUBS_HEAD);
        }
        let name = match Some(row.at) {
            at if at == program.main => "main".to_owned(),
            at if at == program.initialize => "initialize".to_owned(),
            _ => slot_name(row.at),
        };
        let _ = writeln!(out, "// {name} at 0x{:06X}: not decompiled yet", row.at);
    }
    out
}

/// What a script opens as at `level`: every function, or for a program of more code than [`WHOLE`] the one its header names as `main`.
fn first_ask(program: &Program, level: Level) -> Ask {
    let large = program.code.len() > WHOLE;
    Ask {
        level,
        picked: large.then(|| program.main.into_iter().collect()),
    }
}

/// The source `ask` asks of the program `bytes` hold, with what the names folder and a project give; `held` is the program as read, and the project file it was read with.
// ⚠ A project starts functions the program read alone does not show, so the program is read
// again whenever the project found is another than the one it was read with.
fn written(
    held: (Arc<Program>, Option<Stamp>),
    bytes: &Arc<[u8]>,
    ask: &Ask,
    beside: Option<&Path>,
    progress: Option<&Progress>,
) -> Result<Gta6ScriptDocument, String> {
    let folder = names_dir();
    let mut supplied = lists::supplied(folder.as_deref());
    if labels_on() {
        lists::labelled(&mut supplied, folder.as_deref());
    }
    let found = lists::project(&[beside, folder.as_deref()]);
    let project = found.project();
    let stamp = found.stamp().cloned();
    let program = match held {
        (program, with) if with == stamp => program,
        _ => Arc::new(guarded(|| {
            lift::read(bytes, project).map_err(|e| e.to_string())
        })??),
    };
    lists::bind(&mut supplied.names, &program);
    let cx = Context::new(&supplied.names, project, Options::at(ask.level));
    let rows: Vec<FunctionRow> = match &ask.picked {
        None => program.functions.clone(),
        Some(picked) => (program.functions.iter())
            .filter(|row| picked.contains(&row.at))
            .copied()
            .collect(),
    };
    let output = guarded(|| lift::functions_in(&program, &rows, &cx, progress))?;
    if progress.is_some_and(Progress::stopped) {
        return Err(tr!("the decompile was stopped").to_string());
    }
    let mut source: String = output.files.into_iter().map(|file| file.text).collect();
    // The offsets asked that start a function: what the source holds, and what a later ask adds to.
    let picked: Option<BTreeSet<u32>> =
        (ask.picked.as_ref()).map(|_| rows.iter().map(|row| row.at).collect());
    if let Some(picked) = &picked {
        source.push_str(&stubs(&program, picked));
    }
    let read = Read {
        natives: (supplied.names.natives.as_ref()).map(|list| list.label().to_owned()),
        natives_lists: supplied.natives,
        lists: supplied.names.lists.clone(),
        refused: supplied.refused,
        named: supplied.named,
        candidates: supplied.candidates,
        labels: supplied.labels,
        labelled: supplied.labelled,
        project: stamp.as_ref().map(|stamp| stamp.file.clone()),
        project_refused: found.refused.clone(),
    };
    Ok(Gta6ScriptDocument {
        stats: output.stats,
        read,
        picked,
        // A program of an archive is read under a path of the archive's own, whose folder is the archive.
        archive: beside
            .filter(|at| build::is_archive(at))
            .map(Path::to_path_buf),
        bytes: Arc::clone(bytes),
        beside: beside.map(Path::to_path_buf),
        stamp,
        ..Gta6ScriptDocument::over(program, &source, ask.level)
    })
}

/// What `work` comes to on a thread of its own with a deep stack, a panic in it being the error.
// ⚠ A panic on the load worker itself would leave the app waiting for a file that never arrives.
// An overflow of this stack is no panic: no join catches it, and it ends the process.
fn guarded<T: Send>(work: impl FnOnce() -> T + Send) -> Result<T, String> {
    std::thread::scope(|scope| {
        let run = std::thread::Builder::new()
            .stack_size(STACK)
            .spawn_scoped(scope, work);
        match run {
            Ok(thread) => thread
                .join()
                .map_err(|_| tr!("the decompiler stopped on a fault in this program").to_string()),
            Err(e) => Err(e.to_string()),
        }
    })
}

/// The program `bytes` are, refused in the reader's own words.
fn read(bytes: &[u8]) -> Result<Program, String> {
    guarded(|| Program::read(bytes).map_err(|e| e.to_string()))?
}

/// The document of a program the reader took: its source at `level`, or its bytes under the reason it does not decompile.
fn opened(program: Program, bytes: &[u8], level: Level) -> DocPayload {
    let ask = first_ask(&program, level);
    let held: Arc<[u8]> = Arc::from(bytes);
    let folder = beside();
    match written((Arc::new(program), None), &held, &ask, folder.as_deref(), None) {
        Ok(script) => DocPayload::Gta6Script(Arc::new(script)),
        Err(e) => DocPayload::Hex(HexDocument {
            bytes: bytes.to_vec(),
            banner: tr!(
                "This is a compiled GTA VI script that does not decompile, so it is shown as its bytes: {e}",
                e = e
            ),
        }),
    }
}

/// The script `bytes` are, where they start as a deciphered program does and its reader takes them.
// ⚠ Eight zero bytes only say the reader is worth asking: a body that starts so and is no
// program opens as it did before there was a reader.
pub(crate) fn sniffed(bytes: &[u8]) -> Option<DocPayload> {
    if !bytes.starts_with(&[0u8; 8]) {
        return None;
    }
    let program = read(bytes).ok()?;
    Some(opened(program, bytes, level()))
}

/// The script `bytes` are, refused in the reader's own words.
pub(crate) fn open(bytes: &[u8]) -> Result<DocPayload, String> {
    Ok(opened(read(bytes)?, bytes, level()))
}

impl AppState {
    /// Put what a script decompiled to at another level in place of the script `doc` holds.
    pub(crate) fn replace_gta6_script(&mut self, doc: DocumentId, payload: DocPayload) {
        let Some(document) = self.document_mut(doc) else {
            return;
        };
        if matches!(document.payload, DocPayload::Gta6Script(_)) {
            document.reloaded(payload, None);
            document.kind = document.payload.kind();
        }
    }
}

#[cfg(test)]
#[path = "gta6_script_tests.rs"]
mod tests;
