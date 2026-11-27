//! The checks over a folder of lifted listings and a folder of compiled programs, on a machine
//! that has them: every input reads, every function does after the passes what it did before,
//! what is written reads back, two runs write the same, and a function lifted from its bytes
//! does what its listing does.
//!
//! Slow tier, by its module in `.config/nextest.toml`. The folders are named by variables and
//! never looked for: without its variable a test records that it skipped and asserts nothing.
//! ⚠ Both folders are game-derived. Nothing of them is written anywhere: a report is counts, and
//! the first words of what went wrong.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use super::*;
use crate::gta6::script::ir::{Const, VmType};
use crate::gta6::script::lift::LiftError;
use crate::gta6::script::opcode::Op;
use crate::gta6::script::program::{FunctionRow, Program};
use crate::gta6::script::project::{self, index};
use crate::gta6::script::reparse::{self, ReparseError};
use crate::gta6::script::{decode, decompile, function_in, lift, print, Pass};

/// A folder of `*.ysc.cs`.
const LISTINGS: &str = "OPENRAGE_LIFTED_LISTINGS";
/// Parts of file names with commas between, to narrow a run to the inputs that hold one of them.
const ONLY: &str = "OPENRAGE_LIFTED_ONLY";
/// A folder of deciphered compiled programs, flat or one folder an archive.
const PROGRAMS: &str = "OPENRAGE_GTA6_SCRIPTS";
/// The project file of every listing of the folder; without it a listing is checked with its own part alone.
const PROJECT: &str = "OPENRAGE_LIFTED_PROJECT";
/// A file of the differences a run is known to find, a line each as the report writes it up to its second `: `; the run then fails on any other and on any of these it does not find.
const EXPECTED: &str = "OPENRAGE_LIFTED_EXPECTED";

/// The project [`PROJECT`] names, where it names one.
// ⚠ A file named that does not load fails: the run would have checked with less than was asked.
pub(super) fn whole_project() -> Option<Project> {
    let path = std::env::var_os(PROJECT)?;
    let bytes = std::fs::read(&path).unwrap_or_else(|why| panic!("{PROJECT}: {why}"));
    Some(index::load(&bytes).unwrap_or_else(|why| panic!("{PROJECT}: {why}")))
}

const WORKERS: usize = 8;
/// The differences the suite that reads text back keeps the words of: the rest are counted.
const SHOWN: usize = 100;
/// The stack of a worker: the reader and the passes recurse on what a listing nests.
const STACK: usize = 256 << 20;

/// The entries of `dir` that `keep` takes.
// ⚠ A variable that names nothing readable fails: every suite would have skipped and passed.
fn entries(var: &str, dir: &Path, keep: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let read = std::fs::read_dir(dir);
    let read = read.unwrap_or_else(|why| panic!("{var} is `{}`: {why}", dir.display()));
    let paths = read.filter_map(Result::ok).map(|entry| entry.path());
    paths.filter(|path| keep(path.as_path())).collect()
}

/// `paths` as a run takes them: those [`ONLY`] narrows it to, the smallest first so that a first failure shows soon.
fn ordered(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let only = std::env::var(ONLY).ok().map(|only| only.to_lowercase());
    let named = |path: &PathBuf| match (&only, path.file_name()) {
        (Some(only), Some(name)) => {
            let name = name.to_string_lossy().to_lowercase();
            only.split(',').any(|part| name.contains(part))
        }
        (Some(_), None) => false,
        (None, _) => true,
    };
    let mut paths: Vec<PathBuf> = paths.into_iter().filter(named).collect();
    paths.sort_by_cached_key(|path| {
        (
            std::fs::metadata(path).map_or(0, |meta| meta.len()),
            path.clone(),
        )
    });
    paths
}

fn is_listing(path: &Path) -> bool {
    path.to_string_lossy().ends_with(".ysc.cs")
}

/// The listings of the folder [`LISTINGS`] names; `None`, recorded as a skip, where it names none.
fn listings(what: &str) -> Option<Vec<PathBuf>> {
    let Some(dir) = std::env::var_os(LISTINGS) else {
        let detail = format!("set {LISTINGS} to a folder of lifted listings");
        or_test_support::skip_corpus(what, detail);
        return None;
    };
    let found = entries(LISTINGS, Path::new(&dir), is_listing);
    assert!(
        !found.is_empty(),
        "{LISTINGS} names a folder with no `.ysc.cs` file"
    );
    Some(ordered(found))
}

/// Every file of the folder [`PROGRAMS`] names and of the folders in it, whatever [`ONLY`] says; `None`, recorded as a skip, where it names none.
fn programs(what: &str) -> Option<Vec<PathBuf>> {
    let Some(dir) = std::env::var_os(PROGRAMS) else {
        let detail = format!("set {PROGRAMS} to a folder of deciphered compiled scripts");
        or_test_support::skip_install(what, detail);
        return None;
    };
    let dir = Path::new(&dir);
    let mut found = entries(PROGRAMS, dir, Path::is_file);
    for archive in entries(PROGRAMS, dir, Path::is_dir) {
        found.extend(entries(PROGRAMS, &archive, Path::is_file));
    }
    assert!(!found.is_empty(), "{PROGRAMS} names a folder with no file");
    Some(found)
}

/// The program a listing states in its head, read from the head alone.
fn program_of(listing: &Path) -> Option<u32> {
    use std::io::Read;
    // The head is the imports, a line each, and a few lines more.
    let mut head = Vec::new();
    let file = std::fs::File::open(listing).ok()?;
    file.take(64 << 10).read_to_end(&mut head).ok()?;
    let head = String::from_utf8_lossy(&head);
    let (_, stated) = head.split_once("[VmProgram(0x")?;
    u32::from_str_radix(stated.get(..8)?, 16).ok()
}

/// What a compiled program and its listing are both named by: the hash, without what each puts round it.
fn stem(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase());
    let name = name.unwrap_or_default();
    let name = name.strip_suffix(".ysc.cs").unwrap_or(&name);
    let bare = name.strip_prefix("0x").or(name.strip_prefix("hash_"));
    bare.unwrap_or(name).to_owned()
}

/// Hands `paths` to the workers and gathers what `work` makes of each, in the order of `paths`.
// ponytail: an input is one worker's work, so the largest listing bounds the wall; hand out
// functions instead when that is what a run waits for.
fn each<T: Send>(paths: &[PathBuf], work: impl Fn(&Path) -> T + Sync) -> Vec<T> {
    let next = AtomicUsize::new(0);
    let made: Mutex<Vec<(usize, T)>> = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..WORKERS {
            let worker = std::thread::Builder::new().stack_size(STACK);
            let spawned = worker.spawn_scoped(scope, || loop {
                let n = next.fetch_add(1, Ordering::Relaxed);
                let Some(path) = paths.get(n) else {
                    break;
                };
                let one = work(path);
                made.lock().expect("no worker panicked").push((n, one));
            });
            spawned.expect("a worker starts");
        }
    });
    let mut made = made.into_inner().expect("no worker panicked");
    made.sort_by_key(|(n, _)| *n);
    made.into_iter().map(|(_, one)| one).collect()
}

/// `run`, with a panic in it as `None` and not as the end of the suite.
fn caught<T>(run: impl FnOnce() -> T) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).ok()
}

/// Takes from every call of `f` the names a listing's declarations give it, gives a native its handler's address for a hash and a type of the runtime its number for a name: a compiled program states the address and the number alone.
fn by_address(f: &mut cfg::Function) {
    // A type of the runtime has its number with the top bit set; a listing names some of them.
    let numbered = |ty: &mut VmType| {
        if ty.id & 0x8000_0000 != 0 {
            ty.name = format!("type_{:08X}", ty.id);
        }
    };
    let mut known = |e: &mut Expr| {
        // What a listing cannot say: a type's token is to it the float of the token's bits, and a
        // conversion of inferred meaning is nothing at all.
        let said = match &mut *e {
            Expr::Call {
                callee: Callee::Intrinsic { name, ty, .. },
                args,
                ..
            } => match (name.as_str(), ty, args.as_mut_slice()) {
                (decode::TYPE_TOKEN, Some(ty), []) => {
                    let bits = ty.id as i32 as i64 as u64;
                    Some(Expr::Const(Const {
                        bits,
                        kind: Kind::F32,
                    }))
                }
                ("i64_to_i32", _, [x]) => {
                    let x = std::mem::replace(x, Expr::GuidEmpty);
                    Some(Expr::cast(Scalar::I32, x))
                }
                ("u64_to_u32", _, [x]) => {
                    let x = std::mem::replace(x, Expr::GuidEmpty);
                    Some(Expr::cast(Scalar::U32, x))
                }
                _ => None,
            },
            _ => None,
        };
        if let Some(said) = said {
            *e = said;
        }
        let callee = match e {
            Expr::Call { callee, .. } => callee,
            Expr::NewObject(ty) | Expr::VirtualFunctionRef { ty, .. } => return numbered(ty),
            // One instruction pushes 32 bits widened by their sign; a listing writes them as a
            // float where it takes them for one, which leaves the upper half of the cell zero.
            Expr::Const(c) if c.kind == Kind::F32 => {
                c.bits = c.bits as u32 as i32 as i64 as u64;
                return;
            }
            Expr::NewArray { ty, wrapper, .. } => {
                wrapper.clear();
                return numbered(ty);
            }
            _ => return,
        };
        match callee {
            Callee::Native {
                namespace,
                name,
                hash,
                rva,
            } => {
                (*namespace, *hash) = (None, u64::from(*rva));
                name.clear();
            }
            Callee::Local { name, .. } => name.clear(),
            Callee::Import { module, name, .. } => {
                module.clear();
                name.clear();
            }
            Callee::Builtin { path, .. } => path.clear(),
            Callee::Virtual {
                class, method, ty, ..
            } => {
                class.clear();
                method.clear();
                // A slot of a type of the runtime is called by the type's own name in a listing and by its number in a program: neither says which the other means.
                let hashed = ty.len() == 21 && ty.starts_with("type_");
                if !hashed {
                    *ty = "a type of the runtime".to_owned();
                }
            }
            Callee::Intrinsic { wrapper, ty, name } => {
                *wrapper = None;
                // A listing names the two stores of a whole cell into an array each by the other's name: the element types of the arrays say which is which.
                if name == "array_set_ref" {
                    *name = "array_set_u64".to_owned();
                }
                // A listing prints one cast, whatever the mode of the instruction.
                let cast = format!("{}<", decode::CAST);
                if let Some(ty) = name.strip_prefix(&cast) {
                    *name = format!("try_cast<{ty}");
                }
                // The modes past the two a listing can tell apart are printed as the same cast.
                let moded = name
                    .strip_prefix("cast_mode_")
                    .and_then(|rest| rest.split_once('<'));
                let moded = moded.filter(|(mode, _)| mode.bytes().all(|b| b.is_ascii_digit()));
                if let Some(ty) = moded.map(|(_, ty)| ty.to_owned()) {
                    *name = format!("try_cast<{ty}");
                }
                if let Some(ty) = ty {
                    numbered(ty);
                }
            }
        }
    };
    for block in &mut f.cfg.blocks {
        for stmt in &mut block.stmts {
            stmt.each_expr_mut(&mut |top| top.walk_mut(&mut known));
        }
        if let Some(e) = block.term.expr_mut() {
            e.walk_mut(&mut known);
        }
    }
}

/// Why a function was not lifted, its offsets left out, with the kinds of call in it whose cells no table states.
fn refused(program: &Program, project: &Project, row: &FunctionRow, why: &LiftError) -> String {
    let insns = decode::function_in(program, project, row).unwrap_or_default();
    let mut kinds = BTreeSet::new();
    for op in insns.iter().flat_map(|insn| &insn.ops) {
        if let Op::Unsized { callee, .. } = op {
            kinds.insert(match callee {
                Callee::Virtual { .. } => "a slot",
                Callee::Intrinsic { .. } => "an array element",
                Callee::Builtin { .. } => "a builtin",
                _ => "a function",
            });
        }
    }
    let what = match why {
        LiftError::Stack { what, .. } => what.as_str(),
        LiftError::Disasm(_) => "an instruction does not decode",
    };
    format!("not lifted: {what}; calls of unstated cells: {kinds:?}")
}

/// Where the debug forms of two things first part, as the names of the variants and fields round that place.
// ⚠ Only words a `{`, `(` or `:` follows are kept, which are the code's own: no name and no value of the corpus.
fn parting(a: &impl std::fmt::Debug, b: &impl std::fmt::Debug) -> String {
    let (a, b) = (format!("{a:?}"), format!("{b:?}"));
    let same = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
    // The debug form of a string keeps its letters, which may be of several bytes.
    let whole = |n: &usize| a.is_char_boundary(*n) && b.is_char_boundary(*n);
    let same = (0..=same).rev().find(whole).unwrap_or(0);
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    // The words before the place, each with what follows it.
    let mut context: Vec<&str> = Vec::new();
    let mut rest = &a[..same.min(a.len())];
    while let Some(start) = rest.find(word) {
        let tail = &rest[start..];
        let len = tail.find(|c| !word(c)).unwrap_or(tail.len());
        let after = tail[len..].trim_start();
        if after.starts_with(['{', '(', ':']) && !tail.starts_with(|c: char| c.is_ascii_digit()) {
            context.push(&tail[..len]);
        }
        rest = &tail[len..];
    }
    let last: Vec<&str> = context.iter().rev().take(3).rev().copied().collect();
    // The variant each side has at the place, where a variant is what differs.
    let variant = |text: &str| -> String {
        let from = text[..same.min(text.len())]
            .rfind(|c| !word(c))
            .map_or(0, |n| n + 1);
        let tail = &text[from..];
        let name = &tail[..tail.find(|c| !word(c)).unwrap_or(tail.len())];
        match name.starts_with(|c: char| c.is_ascii_uppercase()) {
            true => name.to_owned(),
            false => "a value".to_owned(),
        }
    };
    format!("{} | {} / {}", last.join(" > "), variant(&a), variant(&b))
}

/// The comparison that holds exactly where `op` does not.
fn opposite(op: BinOp) -> BinOp {
    match op {
        BinOp::Eq => BinOp::Ne,
        BinOp::Ne => BinOp::Eq,
        BinOp::Lt => BinOp::Ge,
        BinOp::Ge => BinOp::Lt,
        BinOp::Le => BinOp::Gt,
        _ => BinOp::Le,
    }
}

/// The sense of a test: the comparison it comes down to, where it is one, else whether it holds where what it tests is zero.
fn sense(cond: &Expr) -> (Option<BinOp>, bool) {
    let truth = |e: &Expr| {
        let mut e = e;
        while let Expr::Cast { e: inner, .. } = e {
            e = &**inner;
        }
        match e {
            Expr::Truth(_) | Expr::Unary { op: UnOp::Not, .. } => true,
            Expr::Binary { op, .. } => op.is_logical(),
            _ => false,
        }
    };
    let (mut e, mut not) = (cond, false);
    loop {
        match e {
            Expr::Truth(inner) | Expr::Cast { e: inner, .. } => e = &**inner,
            Expr::Unary {
                op: UnOp::Not,
                e: inner,
            } => {
                not = !not;
                e = &**inner;
            }
            // A truth held against zero is the truth, or its opposite.
            Expr::Binary {
                op: op @ (BinOp::Eq | BinOp::Ne),
                l,
                r,
                ..
            } if matches!(&**r, Expr::Const(c) if c.bits == 0) && truth(l) => {
                not ^= *op == BinOp::Eq;
                e = &**l;
            }
            Expr::Binary { op, .. } if op.is_comparison() => {
                let op = if not { opposite(*op) } else { *op };
                return (Some(op), false);
            }
            _ => return (None, not),
        }
    }
}

/// The sense of the test a block ends in, and the offsets of the two blocks it goes to.
type End = ((Option<BinOp>, bool), u32, u32);

/// The blocks, by their offsets, at whose end the two forms of one flat function test the same thing the other way round, and the branches held against each other.
// ⚠ Needs no run: a branch no seed reaches is read here too. Turned is the same two targets
// under opposite tests, or one test with its targets exchanged.
fn turned(lifted: &cfg::Function, read: &cfg::Function) -> (Vec<u32>, u64) {
    let ends = |f: &cfg::Function| {
        let offset = |block: usize| f.cfg.blocks.get(block).and_then(|b| b.at);
        let mut ends: BTreeMap<u32, End> = BTreeMap::new();
        for block in &f.cfg.blocks {
            let (Some(at), Terminator::Branch { cond, then, els }) = (block.at, &block.term) else {
                continue;
            };
            if let (Some(then), Some(els)) = (offset(*then), offset(*els)) {
                ends.insert(at, (sense(cond), then, els));
            }
        }
        ends
    };
    let (ours, theirs) = (ends(lifted), ends(read));
    let (mut found, mut held) = (Vec::new(), 0);
    for (at, (test, then, els)) in &ours {
        let Some((other, their_then, their_els)) = theirs.get(at) else {
            continue;
        };
        held += 1;
        let same_way = (then, els) == (their_then, their_els);
        let other_way = (then, els) == (their_els, their_then) && then != els;
        let against = match (test, other) {
            ((Some(a), _), (Some(b), _)) => *b == opposite(*a),
            ((None, not), (None, their_not)) => not != their_not,
            _ => false,
        };
        if (test == other && other_way) || (against && same_way) {
            found.push(*at);
        }
    }
    (found, held)
}

/// Where two forms of one function first part.
fn cause(lifted: &cfg::Function, read: &cfg::Function) -> String {
    // The types the instructions name are what no listing states.
    let mut head = lifted.head.clone();
    head.value_types.clear();
    if head != read.head {
        return format!("head: {}", parting(&head, &read.head));
    }
    if lifted.cfg.blocks.len() != read.cfg.blocks.len() {
        return "the count of blocks".to_owned();
    }
    for (a, b) in lifted.cfg.blocks.iter().zip(&read.cfg.blocks) {
        if let Some((s, t)) = a.stmts.iter().zip(&b.stmts).find(|(s, t)| s != t) {
            return format!("statement: {}", parting(s, t));
        }
        if a.stmts.len() != b.stmts.len() {
            return "the count of statements of a block".to_owned();
        }
        if a.term != b.term {
            return format!("end of a block: {}", parting(&a.term, &b.term));
        }
        if a.at != b.at {
            return "the offset of a block".to_owned();
        }
    }
    "nothing".to_owned()
}

/// Counts by what they count, and the first words on each thing that went wrong.
#[derive(Default)]
struct Found {
    counts: BTreeMap<&'static str, u64>,
    /// Where a lifted function and its listing's first part, by [`cause`].
    causes: BTreeMap<String, u64>,
    wrong: Vec<String>,
}

impl Found {
    fn count(&mut self, what: &'static str, n: u64) {
        *self.counts.entry(what).or_default() += n;
    }

    /// Notes what went wrong at `at`, cut short: the rest of a failure is cells of the corpus.
    fn wrong(&mut self, what: &'static str, at: &str, why: &str) {
        self.count(what, 1);
        let why: String = why.chars().take(160).collect();
        self.wrong.push(format!("{what}: {at}: {why}"));
    }

    /// Takes what the before/after check of the function at `at` came to; a difference is counted under the pass it names.
    fn checked(&mut self, at: &str, checked: Option<Result<Checked, String>>) {
        match checked {
            Some(Ok(done)) if done.skipped => self.count("functions not run", 1),
            Some(Ok(done)) => {
                self.count("functions compared", 1);
                self.count("seeds run", done.seeds.into());
                self.count("blocks entered", done.reached.into());
                self.count("blocks", done.blocks.into());
                // What entered them, and what stands before those none did.
                for (by, entered) in Turned::ALL.into_iter().zip(done.entered) {
                    self.count(by.label(), entered.into());
                }
                for (before, left) in inputs::LEFT.into_iter().zip(done.left) {
                    self.count(before, left.into());
                }
            }
            Some(Err(why)) => {
                let first = why.split(',').next().unwrap_or("");
                // A proof of the project the function's own code denies, by its kind.
                let proof = PROOFS.into_iter().find(|kind| *kind == first);
                let what = match (Pass::from_name(first), proof) {
                    (Some(pass), _) => pass.name(),
                    (None, Some(kind)) => kind,
                    (None, None) => "differs",
                };
                self.wrong(what, at, &why);
            }
            None => self.wrong("panics", at, "a pass or the oracle panicked"),
        }
    }

    /// Takes what reading the text of the function at `at` back came to: `None` for a panic, `Some(None)` for a function that does not lower.
    fn read_back(&mut self, at: &str, read: Option<Option<Result<(), ReparseError>>>) {
        match read {
            Some(Some(Ok(()))) => self.count("functions that read back", 1),
            Some(Some(Err(ReparseError::Differs { text, held, .. }))) => {
                let why = format!("the text says `{text}`, the function holds `{held}`");
                self.wrong("functions that read back as another", at, &why);
            }
            // ⚠ Counted and compared by nothing: the bound is what one function may take of memory.
            Some(Some(Err(ReparseError::TooLarge { .. }))) => {
                self.count("functions too large to read back", 1);
            }
            // ⚠ Counted by the form, and compared no further than it.
            Some(Some(Err(ReparseError::Unread { form, .. }))) => {
                self.count("functions with a form not read back yet", 1);
                *self
                    .causes
                    .entry(format!("a form not read back: {form}"))
                    .or_default() += 1;
            }
            Some(Some(Err(why))) => self.wrong("functions not read", at, &why.to_string()),
            Some(None) => self.wrong("functions not decompiled", at, "does not lower"),
            None => self.wrong("panics", at, "a pass, the printer or the reader panicked"),
        }
    }

    /// Keeps the words of the first `most` things that went wrong and counts the rest, which stay counted by what they are.
    fn keep(&mut self, most: usize) {
        let more = self.wrong.len().saturating_sub(most);
        if more > 0 {
            self.wrong.truncate(most);
            self.count("differences counted and not shown", more as u64);
        }
    }

    fn all(parts: impl IntoIterator<Item = Found>) -> Found {
        let mut all = Found::default();
        for part in parts {
            for (what, n) in part.counts {
                all.count(what, n);
            }
            for (cause, n) in part.causes {
                *all.causes.entry(cause).or_default() += n;
            }
            all.wrong.extend(part.wrong);
        }
        all
    }

    fn report(&self, label: &str) {
        eprintln!("{label}: {:?}", self.counts);
        let mut causes: Vec<(&String, &u64)> = self.causes.iter().collect();
        causes.sort_by_key(|(cause, n)| (std::cmp::Reverse(**n), *cause));
        for (cause, n) in causes.into_iter().take(60) {
            eprintln!("{label}: parts at {cause}: {n}");
        }
        for wrong in self.wrong.iter().take(400) {
            eprintln!("{label}: {wrong}");
        }
    }

    /// Whether what went wrong is what the file [`EXPECTED`] names says a run finds: nothing, where no file is named.
    // ⚠ A difference is known by what it is and where: `what: file: function`, never by the cells of the corpus that follow.
    fn as_expected(&self, label: &str) -> bool {
        let place = |line: &str| -> String {
            let mut parts = line.splitn(4, ": ");
            let known: Vec<&str> = parts.by_ref().take(3).collect();
            known.join(": ")
        };
        let found: BTreeSet<String> = self.wrong.iter().map(|line| place(line)).collect();
        let expected: BTreeSet<String> = match std::env::var_os(EXPECTED) {
            Some(path) => {
                let text = std::fs::read_to_string(&path);
                let text = text.unwrap_or_else(|why| panic!("{EXPECTED}: {why}"));
                text.lines().map(place).filter(|l| !l.is_empty()).collect()
            }
            None => BTreeSet::new(),
        };
        for new in found.difference(&expected) {
            eprintln!("{label}: not expected: {new}");
        }
        for gone in expected.difference(&found) {
            eprintln!("{label}: expected and not found: {gone}");
        }
        eprintln!(
            "{label}: {} difference(s) found, {} expected",
            found.len(),
            expected.len()
        );
        found == expected
    }
}

/// Holds the two functions an `async` method gives back to the kick-off and the step function it was made of, each as the passes left it.
fn given_back(
    join: &passes::async_await::Join,
    finished: &[tree::Function],
    project: &Project,
) -> Result<(), String> {
    let (kick, step) = join
        .parts()
        .ok_or("the method does not go back into two functions")?;
    for (part, was) in [(kick, &finished[join.kick]), (step, &finished[join.step])] {
        let name = part.head.name.clone();
        let part =
            cfg::lower(part).map_err(|e| format!("{name} given back does not lower: {e}"))?;
        let was = cfg::lower(was.clone()).map_err(|e| format!("{name} does not lower: {e}"))?;
        agree(&part, &was, Limits::CORPUS, project).map_err(|why| format!("{name}: {why}"))?;
    }
    Ok(())
}

#[test]
fn every_function_of_every_listing_does_what_it_did() {
    let what = "the before/after check over a folder of lifted listings";
    let Some(paths) = listings(what) else {
        return;
    };
    let names = Names::default();
    let whole = whole_project();
    let found = each(&paths, |path| {
        let (mut first, mut every) = (Found::default(), Found::default());
        let text = std::fs::read_to_string(path).expect("the listing is text");
        let stem = stem(path);
        let listing = match Listing::open(&text) {
            Ok(listing) => listing,
            Err(why) => {
                first.wrong("listings refused", &stem, &format!("line {}", why.line()));
                return (first, every);
            }
        };
        // What the passes of the source level know of the program: the project named, else its own part of one.
        let own = match &whole {
            Some(_) => None,
            None => {
                let part = index::of_listing(&text).map(|part| index::merge(vec![part]));
                if part.is_err() {
                    every.count("listings with no project", 1);
                }
                Some(part.unwrap_or_default())
            }
        };
        let project = whole.as_ref().or(own.as_ref()).unwrap_or(Project::empty());
        let all = Context::new(&names, project, Options::default());
        let mut finished = Vec::new();
        for f in listing.functions() {
            let f = match f {
                Ok(f) => f,
                Err(why) => {
                    first.wrong("listings refused", &stem, &format!("line {}", why.line()));
                    break;
                }
            };
            let at = format!("{stem}: {}", f.head.name);
            let again = f.clone();
            if let Some(Ok(done)) = caught(|| function_in(f.clone(), &all, &mut Stats::default())) {
                finished.push(done);
            }
            let four = Context::baseline(&names);
            first.checked(&at, caught(|| check_in(f, &four, Limits::CORPUS)));
            every.checked(&at, caught(|| check_in(again, &all, Limits::CORPUS)));
        }
        for join in passes::async_await::joins(&finished, &all) {
            let at = format!("{stem}: {}", join.whole.head.name);
            every.count("async methods joined", 1);
            match caught(|| given_back(&join, &finished, project)) {
                Some(Ok(())) => every.count("async methods given back as their two functions", 1),
                Some(Err(why)) => every.wrong("join", &at, &why),
                None => every.wrong("panics", &at, "the join's way back or the oracle panicked"),
            }
        }
        (first, every)
    });
    let (first, every): (Vec<Found>, Vec<Found>) = found.into_iter().unzip();
    let (first, every) = (Found::all(first), Found::all(every));
    eprintln!("ORACLE: {} listing(s)", paths.len());
    first.report("ORACLE, the four first passes");
    every.report("ORACLE, every pass");
    assert!(first.wrong.is_empty() && every.wrong.is_empty());
}

/// The tallies of the addresses of locals handed to a function, by what the callee reaches against what the passes take it to reach.
const REACHED: [&str; 4] = [
    "addresses whose callee reaches no further than the passes take it to",
    "addresses whose callee reaches further than the passes take it to",
    "addresses whose callee reaches without bound",
    "addresses whose callee the project holds no reach of",
];

/// Holds each address of a local that `f` hands to a function against what the callee reaches: what the passes take it to reach is the local alone where it is proved one cell wide, else every local up to the next one so proved.
fn reached_sites(f: &mut cfg::Function, project: &Project, found: &mut Found, at: &str) {
    passes::values::widths(f);
    let singles: BTreeSet<u32> = (f.head.vars.iter())
        .filter_map(|(var, info)| match var {
            Var::Local(n) if info.single => Some(*n),
            _ => None,
        })
        .collect();
    let frame = f.head.locals;
    let taken = |start: u32| -> u64 {
        let end = match singles.contains(&start) {
            true => start.saturating_add(1),
            false => singles
                .range(start.saturating_add(1)..)
                .next()
                .copied()
                .unwrap_or(frame),
        };
        u64::from(end.clamp(start, frame.max(start)) - start) * 8
    };
    let head = f.head.clone();
    let mut sites: Vec<(u64, Option<u32>)> = Vec::new();
    let mut note = |top: &Expr| {
        top.walk(&mut |e| {
            let Expr::Call { callee, args, .. } = e else {
                return;
            };
            let Some(to) = project::signatures::key_of(head.program, callee) else {
                return;
            };
            let mut cell = Some(0u32);
            for arg in args {
                if let (
                    Some(n),
                    Expr::AddrOf {
                        var: Var::Local(k),
                        cell: c,
                    },
                ) = (cell, arg)
                {
                    let start = k.saturating_add(*c);
                    sites.push((taken(start), project.reach(to, n)));
                }
                cell = cell
                    .zip(print::cells_of(arg, &head))
                    .and_then(|(a, b)| a.checked_add(b));
            }
        });
    };
    for block in &f.cfg.blocks {
        for stmt in &block.stmts {
            stmt.each_expr(&mut note);
        }
        if let Some(e) = block.term.expr() {
            note(e);
        }
    }
    for (assumed, reach) in sites {
        let tally = match reach {
            None => 3,
            Some(project::reach::UNBOUNDED) => 2,
            Some(bytes) if u64::from(bytes) <= assumed => 0,
            Some(bytes) => {
                let why = format!("reaches {bytes} bytes where the passes take {assumed}");
                found.wrong("addresses past what the passes take", at, &why);
                1
            }
        };
        found.count(REACHED[tally], 1);
    }
}

#[test]
fn every_address_of_a_local_handed_to_a_function_reaches_no_further_than_the_passes_take_it() {
    let what =
        "the reach of every address of a local a listing hands to a function, against the project";
    let Some(paths) = listings(what) else {
        return;
    };
    let Some(project) = whole_project() else {
        or_test_support::skip_corpus(what, format!("set {PROJECT} to the project of the folder"));
        return;
    };
    let found = each(&paths, |path| {
        let mut found = Found::default();
        let text = std::fs::read_to_string(path).expect("the listing is text");
        let stem = stem(path);
        let Ok(listing) = Listing::open(&text) else {
            found.count("listings refused", 1);
            return found;
        };
        for f in listing.functions() {
            let Ok(f) = f else {
                found.count("listings refused", 1);
                break;
            };
            let at = format!("{stem}: {}", f.head.name);
            if let Ok(mut lowered) = cfg::lower(f) {
                reached_sites(&mut lowered, &project, &mut found, &at);
            }
        }
        found
    });
    let found = Found::all(found);
    eprintln!("REACH: {} listing(s)", paths.len());
    found.report("REACH");
}

/// Takes `f` through the passes `cx` asks, writes it in the readable form and reads that back; `None` where it does not lower.
// ⚠ The text read back is the readable form of no names list.
fn reread(f: tree::Function, cx: &Context<'_>) -> Option<Result<(), ReparseError>> {
    let done = function_in(f, cx, &mut Stats::default()).ok()?;
    let mut source = String::new();
    print::function(&mut source, &done, cx.names);
    Some(reparse::agrees(&done, &source))
}

/// Takes `f` through the passes `cx` asks once, and reads back both what the readable form writes of it with each construct as what it holds, and what the source level writes of it as its own text; `None` where it does not lower.
// ⚠ Both texts are of no names list. The first sees of a construct the statements it holds, as
// the before/after check does; the second how the source level spells the construct.
fn reread_both(
    f: tree::Function,
    cx: &Context<'_>,
) -> Option<(Result<(), ReparseError>, Result<(), ReparseError>)> {
    let done = function_in(f, cx, &mut Stats::default()).ok()?;
    let mut own = String::new();
    print::function_in(&mut own, &done, cx);
    let sharp = reparse::agrees_source(&done, &own, cx.options, false);
    let tree::Function { head, body } = done;
    let lowered = tree::Function {
        head,
        body: tree::desugar(body),
    };
    let mut source = String::new();
    print::function(&mut source, &lowered, cx.names);
    Some((reparse::agrees(&lowered, &source), sharp))
}

#[test]
fn every_function_written_reads_back_as_the_function() {
    let what = "the readable source of a folder of lifted listings, read back";
    let Some(paths) = listings(what) else {
        return;
    };
    let names = Names::default();
    let whole = whole_project();
    let found = each(&paths, |path| {
        let (mut first, mut every) = (Found::default(), Found::default());
        let mut third = Found::default();
        let text = std::fs::read_to_string(path).expect("the listing is text");
        let stem = stem(path);
        let Ok(listing) = Listing::open(&text) else {
            first.wrong("listings refused", &stem, "does not open");
            return (first, every, third);
        };
        // What the passes of the source level know of the program: the project named, else its own part of one.
        let own = match &whole {
            Some(_) => None,
            None => {
                let part = index::of_listing(&text).map(|part| index::merge(vec![part]));
                if part.is_err() {
                    every.count("listings with no project", 1);
                }
                Some(part.unwrap_or_default())
            }
        };
        let project = whole.as_ref().or(own.as_ref()).unwrap_or(Project::empty());
        let four = Context::baseline(&names);
        let all = Context::new(&names, project, Options::default());
        // A function at a time: what one came to is counts, and the words of the first differences.
        for f in listing.functions() {
            let Ok(f) = f else {
                first.wrong("listings refused", &stem, "a function does not read");
                break;
            };
            let at = format!("{stem}: {}", f.head.name);
            let again = f.clone();
            first.read_back(&at, caught(|| reread(f, &four)));
            let (lowered, sharp) = match caught(|| reread_both(again, &all)) {
                Some(Some((lowered, sharp))) => (Some(Some(lowered)), Some(Some(sharp))),
                Some(None) => (Some(None), Some(None)),
                None => (None, None),
            };
            every.read_back(&at, lowered);
            third.read_back(&at, sharp);
            first.keep(SHOWN);
            every.keep(SHOWN);
            third.keep(SHOWN);
        }
        (first, every, third)
    });
    let (mut firsts, mut everys, mut owns) = (Vec::new(), Vec::new(), Vec::new());
    for (first, every, own) in found {
        firsts.push(first);
        everys.push(every);
        owns.push(own);
    }
    let (mut first, mut every) = (Found::all(firsts), Found::all(everys));
    let mut own = Found::all(owns);
    first.keep(SHOWN);
    every.keep(SHOWN);
    own.keep(SHOWN);
    first.report("REREAD, the four first passes");
    every.report("REREAD, every pass, each construct as what it holds");
    // ⚠ Reported and not held: the reader does not yet hold every form the source level writes,
    // and a form it does not read is counted by name, not compared.
    own.report("REREAD, every pass, the source level's own text");
    assert!(first.wrong.is_empty() && every.wrong.is_empty());
}

#[test]
fn two_runs_write_the_same_bytes() {
    let what = "a folder of lifted listings decompiled twice";
    let Some(paths) = listings(what) else {
        return;
    };
    let names = Names::default();
    let found = each(&paths, |path| {
        let mut found = Found::default();
        let text = std::fs::read_to_string(path).expect("the listing is text");
        let written = || decompile(&text, &names).map(|done| done.text).ok();
        match (written(), written()) {
            (Some(one), Some(other)) if one == other => found.count("listings written the same", 1),
            (Some(_), Some(_)) => found.wrong("listings written otherwise", &stem(path), ""),
            _ => found.wrong("listings refused", &stem(path), "does not decompile"),
        }
        found
    });
    let found = Found::all(found);
    found.report("SAME BYTES");
    assert!(found.wrong.is_empty());
}

#[test]
#[ignore = "prints one function as lifted and as its listing reads, named by OPENRAGE_LIFTED_SHOW as `program file|listing path|function`; run by name"]
fn shows_one_function_lifted_beside_its_listing() {
    let named = std::env::var("OPENRAGE_LIFTED_SHOW").expect("the function is named");
    let parts: Vec<&str> = named.split('|').collect();
    let [compiled, listing, wanted] = parts.as_slice() else {
        panic!("`program file|listing path|function`");
    };
    let project = whole_project().unwrap_or_default();
    let bytes = std::fs::read(compiled).expect("the program reads");
    let program = lift::read(&bytes, &project).expect("the program is one");
    let text = std::fs::read_to_string(listing).expect("the listing is text");
    let listing = Listing::open(&text).expect("the listing opens");
    let shown = |f: &cfg::Function| {
        let mut writer = print::ListingWriter::new(f.head.program, &[]);
        writer.function(&super::flat(f));
        let text = writer.finish();
        let kept = |line: &&str| !line.contains("ref VmValue ") && !line.contains(" = default;");
        let body: Vec<&str> = text.lines().filter(kept).collect();
        body.join("\n")
    };
    for f in listing.functions().flatten() {
        if f.head.name != *wanted {
            continue;
        }
        let row = program.function(f.head.at).expect("the program has it");
        let read = cfg::lower(f).expect("the function lowers");
        eprintln!("======== listing ========\n{}", shown(&read));
        match lift::function_in(&program, &project, row) {
            Ok(lifted) => {
                eprintln!("======== lifted ========\n{}", shown(&lifted.function));
                let (mut a, mut b) = (lifted.function, read);
                by_address(&mut a);
                by_address(&mut b);
                eprintln!("======== parts at {} ========", cause(&a, &b));
                eprintln!("branches turned: {:X?}", turned(&a, &b).0);
                eprintln!("{:?}", agree(&a, &b, Limits::CORPUS, &project));
            }
            Err(why) => eprintln!("======== not lifted: {why} ========"),
        }
    }
}

/// The opcodes that no test written by hand holds to a meaning: this suite is their check.
const CHECKED: [u16; 3] = [168, 349, 352];

/// The tallies of the sites of each of [`CHECKED`], by what holding its function against its listing came to.
const SITES: [[&str; 4]; 3] = [
    [
        "sites of opcode 168 where every block of both forms ran the same",
        "sites of opcode 168 in functions that run the same where a seed went",
        "sites of opcode 168 in functions that run otherwise",
        "sites of opcode 168 in functions not run",
    ],
    [
        "sites of opcode 349 where every block of both forms ran the same",
        "sites of opcode 349 in functions that run the same where a seed went",
        "sites of opcode 349 in functions that run otherwise",
        "sites of opcode 349 in functions not run",
    ],
    [
        "sites of opcode 352 where every block of both forms ran the same",
        "sites of opcode 352 in functions that run the same where a seed went",
        "sites of opcode 352 in functions that run otherwise",
        "sites of opcode 352 in functions not run",
    ],
];

/// How many instructions of each of [`CHECKED`] the function at `at` holds.
fn sites(program: &Program, project: &Project, at: u32) -> [u64; 3] {
    let mut sites = [0; 3];
    let insns = program
        .function(at)
        .and_then(|row| decode::function_in(program, project, row).ok());
    for insn in insns.into_iter().flatten() {
        if let Some(n) = CHECKED.iter().position(|opcode| *opcode == insn.opcode) {
            sites[n] += 1;
        }
    }
    sites
}

#[test]
fn every_compiled_program_reads_and_lifts_to_what_its_listing_does() {
    let what = "a folder of compiled programs read, lifted and held against their listings";
    let Some(all) = programs(what) else {
        return;
    };
    // A function that names another program's type or function lifts only with what that program
    // states: the project named, else that of every program of the folder, a run narrowed or not.
    let project = whole_project().unwrap_or_else(|| {
        let parts = index::parts(&all).into_iter().filter_map(Result::ok);
        index::merge(parts.collect())
    });
    let project = &project;
    let paths = ordered(all);
    // The listing of each program, where a folder of listings is named too.
    // ⚠ By the program each states, never by a file's name: a listing may go by its program's name and the compiled file by a hash.
    let listed: BTreeMap<u32, PathBuf> = match std::env::var_os(LISTINGS) {
        Some(dir) => {
            let found = entries(LISTINGS, Path::new(&dir), is_listing);
            let stated = |path: PathBuf| Some((program_of(&path)?, path));
            found.into_iter().filter_map(stated).collect()
        }
        None => BTreeMap::new(),
    };
    let names = Names::default();
    let cx = Context::new(&names, project, Options::default());
    let found = each(&paths, |path| {
        let mut found = Found::default();
        let bytes = std::fs::read(path).expect("the file reads");
        // The folder holds other things too: a compiled program starts with four zero bytes.
        if !bytes.starts_with(&[0; 4]) {
            found.count("files that are no program", 1);
            return found;
        }
        let stem = stem(path);
        let program = match caught(|| lift::read(&bytes, project)) {
            Some(Ok(program)) => program,
            Some(Err(why)) => {
                found.wrong("programs refused", &stem, &why.to_string());
                return found;
            }
            None => {
                found.wrong("panics", &stem, "the reader panicked");
                return found;
            }
        };
        found.count("programs read", 1);
        // Lifted as the decompiler lifts a program, slot counts one function learns given to the rest;
        // a function at a time where that panics.
        let whole = caught(|| lift::program_in(&program, project));
        let lifted = |at: u32| {
            let row = program.function(at)?;
            match &whole {
                Some(all) => {
                    let place = program.functions.iter().position(|of| of.at == row.at)?;
                    all.get(place).cloned()
                }
                None => caught(|| lift::function_in(&program, project, row)),
            }
        };
        for row in &program.functions {
            let at = format!("{stem}: func_{:06X}", row.at);
            match lifted(row.at) {
                Some(Ok(lifted)) if lifted.regions.is_empty() => {
                    found.count("functions lifted", 1);
                    let run = || check_blocks(&lifted.function, &cx, Limits::CORPUS);
                    found.checked(&at, caught(run));
                }
                Some(Ok(_)) => found.count("functions with regions, not run", 1),
                Some(Err(why)) => {
                    found.count("functions not lifted", 1);
                    let why = refused(&program, project, row, &why);
                    *found.causes.entry(why).or_default() += 1;
                }
                None => found.wrong("panics", &at, "the lifter panicked"),
            }
        }

        let text = listed
            .get(&program.program)
            .and_then(|path| std::fs::read_to_string(path).ok());
        let Some(listing) = text.as_deref().and_then(|text| Listing::open(text).ok()) else {
            found.count("programs with no listing", 1);
            return found;
        };
        // A function at a time: a listing is not held as trees.
        for f in listing.functions() {
            let Ok(f) = f else {
                found.wrong("listed functions that do not read", &stem, "");
                continue;
            };
            if f.head.incomplete.is_some() {
                continue;
            }
            let at = format!("{stem}: {}", f.head.name);
            let (Some(Ok(lifted)), Ok(mut read)) = (lifted(f.head.at), cfg::lower(f)) else {
                found.count("listed functions with no lifted form", 1);
                continue;
            };
            let mut lifted = lifted.function;
            // The types a lifted program's instructions name are what no listing states.
            lifted.head.value_types.clear();
            by_address(&mut lifted);
            by_address(&mut read);
            found.count("functions in both", 1);
            found.count("heads equal", u64::from(lifted.head == read.head));
            found.count("blocks equal", u64::from(lifted.cfg == read.cfg));
            if lifted == read {
                continue;
            }
            // A structured listing lowers to other blocks: only a flat one's are the lifter's to equal.
            if read.cfg.blocks.iter().any(|block| block.at.is_some()) {
                found.count("flat functions that differ", 1);
                *found.causes.entry(cause(&lifted, &read)).or_default() += 1;
                let (turned, held) = turned(&lifted, &read);
                found.count("branches held against the listing's", held);
                for block in turned {
                    let why = format!("the block at {block:06X} ends in the opposite test");
                    found.wrong("branches turned", &at, &why);
                }
            }
            let agreed = caught(|| agree(&lifted, &read, Limits::CORPUS, project));
            let outcome = match &agreed {
                Some(Ok(done)) if done.skipped => 3,
                Some(Ok(done)) if done.whole => 0,
                Some(Ok(_)) => 1,
                _ => 2,
            };
            for (n, counted) in sites(&program, project, read.head.at)
                .into_iter()
                .enumerate()
            {
                found.count(SITES[n][outcome], counted);
            }
            match agreed {
                Some(Ok(done)) if done.skipped => found.count("functions not run", 1),
                Some(Ok(done)) if done.whole => {
                    found.count("functions that run the same, every block entered", 1);
                }
                Some(Ok(_)) => found.count("functions that run the same where a seed went", 1),
                Some(Err(why)) => {
                    // How the events part is what `same` writes between the event's number and its cells.
                    let how = why
                        .split_once(", ")
                        .and_then(|(_, rest)| rest.split_once(": "));
                    let how = how.map_or("more or fewer events", |(how, _)| how);
                    *found
                        .causes
                        .entry(format!("runs otherwise: {how}"))
                        .or_default() += 1;
                    found.wrong("functions that run otherwise", &at, &why);
                }
                None => found.wrong("panics", &at, "the oracle panicked"),
            }
        }
        found
    });
    let found = Found::all(found);
    eprintln!(
        "COMPILED: {} file(s), {} listing(s) to pair",
        paths.len(),
        listed.len()
    );
    found.report("COMPILED");
    assert!(found.as_expected("COMPILED"));
}
