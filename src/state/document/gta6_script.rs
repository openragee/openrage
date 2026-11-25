//! A compiled GTA VI script opened read-only: the source it decompiles to, the classes and
//! functions that source lists, and the program it was read from.
//!
//! ⚠ **Every name is the user's.** OpenRage ships none: a native, a type, a field or a method
//! reads under a name only where a list in [`names_dir`] names it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use or_io::gta5::script::natives::NativeDb;
use or_io::gta6::script::program::Program;
use or_io::gta6::script::{decompile_program, Context, Level, Names, Options, Project, Stats};

use super::{DocPayload, DocumentId, HexDocument};
use crate::state::AppState;

/// The stack a decompile runs on: a pass recurses as deep as a function's statements nest.
const STACK: usize = 256 << 20;

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

/// Where natives lists and names lists for GTA VI scripts are read from, in the configuration directory.
pub fn names_dir() -> Option<PathBuf> {
    crate::state::prefs::config_root().map(|root| root.join("gta6_script_names"))
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

/// A compiled GTA VI script as the source it decompiles to.
///
/// ⚠ Read-only: OpenRage writes no bytecode.
#[derive(Clone)]
pub struct Gta6ScriptDocument {
    pub source: ropey::Rope,
    /// The classes and functions of the source, in the order of its lines.
    pub outline: Vec<OutlineRow>,
    pub program: Arc<Program>,
    /// The level the source was written at.
    pub level: Level,
    pub stats: Stats,
    /// The natives list read, where the folder holds one.
    pub natives: Option<String>,
    /// The names lists read, in the order read.
    pub lists: Vec<String>,
    // ponytail: held beside the program read from them, so that another level decompiles them
    // again; drop them once a decompile takes a `Program`.
    bytes: Arc<[u8]>,
}

impl Gta6ScriptDocument {
    /// The document of `source`, the text `program` decompiled to at `level`.
    pub fn of(program: Program, source: &str, level: Level) -> Self {
        Self {
            outline: outline_of(source, &program),
            source: ropey::Rope::from_str(source),
            program: Arc::new(program),
            level,
            stats: Stats::default(),
            natives: None,
            lists: Vec::new(),
            bytes: Arc::from(Vec::new()),
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

    /// The decompile of the same bytes at `level`, on the calling thread: a worker's work.
    pub fn at_level(&self, level: Level) -> DocPayload {
        opened((*self.program).clone(), &self.bytes, level)
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

/// The lists of the folder `dir`, in the order of their names.
// ponytail: read again for every script opened; cache them if opening scripts in bulk shows the cost.
fn supplied(dir: Option<PathBuf>) -> Names {
    let mut names = Names::default();
    let entries = dir.and_then(|dir| std::fs::read_dir(dir).ok());
    let mut files: Vec<PathBuf> = (entries.into_iter().flatten().flatten())
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    for path in files {
        let label =
            (path.file_name()).map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::warn!(list = %label, "{e}");
                continue;
            }
        };
        // ponytail: the first natives list that reads names the natives; take the list that names most once a program states the natives it calls.
        if label.to_ascii_lowercase().starts_with("natives") {
            match NativeDb::parse(label.as_str(), &bytes) {
                Ok(db) if names.natives.is_none() => names.natives = Some(db),
                Ok(_) => {}
                Err(e) => tracing::warn!(list = %label, "{e}"),
            }
            continue;
        }
        // ⚠ A list refused part-way has named things already, so it is read into a copy that is kept only whole.
        let mut tried = names.clone();
        let read = String::from_utf8(bytes)
            .map_err(|e| e.to_string())
            .and_then(|text| (tried.read_list(&label, &text)).map_err(|e| e.to_string()));
        match read {
            Ok(_) => names = tried,
            Err(e) => tracing::warn!(list = %label, "{e}"),
        }
    }
    names
}

/// The source `bytes` decompile to at `level`, and what decompiling counted.
fn decompiled(bytes: &[u8], level: Level, names: &Names) -> Result<(String, Stats), String> {
    let cx = Context::new(names, Project::empty(), Options::at(level));
    let output = decompile_program(bytes, &cx).map_err(|e| e.to_string())?;
    let text = output.files.into_iter().map(|file| file.text).collect();
    Ok((text, output.stats))
}

/// What `work` comes to on a thread of its own with a deep stack, a fault in it being the error.
// ⚠ A fault on the load worker itself would leave the app waiting for a file that never arrives.
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
    let names = supplied(names_dir());
    let done = guarded(|| decompiled(bytes, level, &names)).and_then(|done| done);
    match done {
        Ok((source, stats)) => DocPayload::Gta6Script(Arc::new(Gta6ScriptDocument {
            stats,
            natives: names.natives.as_ref().map(|db| db.label().to_owned()),
            lists: names.lists.clone(),
            bytes: Arc::from(bytes),
            ..Gta6ScriptDocument::of(program, &source, level)
        })),
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
