//! `rpf9-diff`: what one GTA VI (RPF9) archive changes against another, entry by entry.
//!
//!   `rpf9-diff <a.rpf> <b.rpf> [--names <dir>] [--keys <snapshot>] [--exe <exe>] [--json]`
//!
//! Entries are paired through the decrypted tables by their name64 hash, never by position. An entry
//! only in B is added, one only in A removed, and an unpaired pair holding the same stored bytes is a
//! rename. A pair whose stored bytes or row differ is changed: a nested archive is compared again
//! inside, a PSO or RBF body field by field through the metadata reader, and a text or XML body line
//! by line. Both archives are only read.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs::File;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::container::rpf::{
    Gta6Class, Gta6Entry, Gta6EntryExtra, Gta6Mount, GtaViKeys, NameResolver,
};
use or_io::gta6::names::{vocabulary_file, Vocabulary};
use or_io::meta::{MetaDocument, MetaInstance, SchemaLibrary, Value};
use serde::Serialize;

/// The folder of recovered-names tables used when `--names` is not given.
const NAMES_ENV: &str = "OPENRAGE_GTA6_NAMES";
/// A body past this many bytes, stored or inflated, is not read for a summary.
const SUMMARY_CAP: u64 = 16 << 20;
/// Real nesting is shallow; a table nested deeper than this is reported rather than followed.
const MAX_DEPTH: usize = 8;
/// The bytes compared per read when two stored bodies are checked for equality.
const COMPARE_STEP: u64 = 1 << 20;
/// How many fields or lines of one summary the text report shows.
const SHOWN: usize = 12;

const USAGE: &str = "\
usage:
  openrage-cli rpf9-diff <a.rpf> <b.rpf> [--names <dir>] [--keys <snapshot>] [--exe <exe>] [--json]";

/// An entry's stored and inflated sizes, as its row states them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct Size {
    pub stored: Option<u64>,
    pub real: Option<u64>,
}

/// One field whose value differs between the two bodies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct FieldChange {
    pub path: String,
    pub before: String,
    pub after: String,
}

/// What a changed entry's bodies differ in, where the tree has a reader for them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum Summary {
    /// A PSO or RBF document read on both sides: `path = value` for an added or removed field.
    Fields {
        format: &'static str,
        added: Vec<String>,
        removed: Vec<String>,
        changed: Vec<FieldChange>,
    },
    /// A text or XML body: the lines of one side the other lacks, counted as a multiset.
    Lines {
        added: Vec<String>,
        removed: Vec<String>,
    },
    /// A nested archive, whose changes follow it in the list.
    Nested { changes: usize },
    /// Only the stored form differs (compression, cipher or row): the decoded bodies are the same.
    Stored,
    /// Why the bodies were not compared further.
    Unread { why: String },
}

/// One difference between the two archives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Change {
    Added {
        path: String,
        size: Size,
    },
    Removed {
        path: String,
        size: Size,
    },
    /// The same stored bytes under another name.
    Renamed {
        from: String,
        to: String,
        size: Size,
    },
    /// Other stored bytes, or another row, under the same name.
    Changed {
        path: String,
        before: Size,
        after: Size,
        summary: Option<Summary>,
    },
}

pub(crate) fn run(args: &[String]) -> ExitCode {
    let (mut names_dir, mut keys_path, mut json) = (None::<PathBuf>, None::<PathBuf>, false);
    let mut exe_path = None::<PathBuf>;
    let mut positionals: Vec<&str> = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--names" | "--keys" | "--exe" => {
                let Some(value) = it.next() else {
                    return usage(&format!("{arg} needs a value"));
                };
                let slot = match arg.as_str() {
                    "--names" => &mut names_dir,
                    "--keys" => &mut keys_path,
                    _ => &mut exe_path,
                };
                *slot = Some(PathBuf::from(value));
            }
            "--json" => json = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other if other.starts_with("--") => return usage(&format!("unknown option {other}")),
            other => positionals.push(other),
        }
    }
    let [a, b] = positionals.as_slice() else {
        return usage("give two archives to compare");
    };
    let keys = match crate::gta6_keys::load(keys_path.as_deref(), exe_path.as_deref()) {
        Ok(keys) => keys,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let resolver = load_resolver(names_dir.as_deref());
    if resolver.is_empty() {
        eprintln!("(no recovered names found; entries show as their hash)");
    }
    match diff_files(Path::new(a), Path::new(b), &keys, &resolver) {
        Ok(changes) if json => {
            let value = serde_json::json!({ "a": a, "b": b, "changes": changes });
            match serde_json::to_string_pretty(&value) {
                Ok(text) => println!("{text}"),
                Err(e) => {
                    eprintln!("Error: {e}");
                    return ExitCode::FAILURE;
                }
            }
            ExitCode::SUCCESS
        }
        Ok(changes) => {
            print!("{}", render(a, b, &changes));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn usage(msg: &str) -> ExitCode {
    eprintln!("Error: {msg}");
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

/// The recovered-names corpus from `dir` (or [`NAMES_ENV`]) and the config dir's vocabulary.
fn load_resolver(dir: Option<&Path>) -> NameResolver {
    let mut resolver = dir
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os(NAMES_ENV).map(PathBuf::from))
        .map(|dir| NameResolver::load(&dir))
        .unwrap_or_default();
    if let Some(config) = or_title::config::config_dir() {
        resolver.merge_vocabulary(&Vocabulary::load(&vocabulary_file(&config)));
    }
    resolver
}

fn diff_files(
    a: &Path,
    b: &Path,
    keys: &GtaViKeys,
    resolver: &NameResolver,
) -> Result<Vec<Change>, String> {
    let mut fa = File::open(a).map_err(|e| format!("{}: {e}", a.display()))?;
    let mut fb = File::open(b).map_err(|e| format!("{}: {e}", b.display()))?;
    diff_archives(&mut fa, &mut fb, keys, resolver)
}

/// Every difference between the archive `a` holds and the one `b` holds, nested archives included.
pub(crate) fn diff_archives<A: Read + Seek, B: Read + Seek>(
    a: &mut A,
    b: &mut B,
    keys: &GtaViKeys,
    resolver: &NameResolver,
) -> Result<Vec<Change>, String> {
    let mut ma = Gta6Mount::open_root(a, keys).map_err(|e| format!("the first archive: {e}"))?;
    let mut mb = Gta6Mount::open_root(b, keys).map_err(|e| format!("the second archive: {e}"))?;
    ma.archive.resolve_names(resolver);
    mb.archive.resolve_names(resolver);
    let mut ctx = Ctx {
        a,
        b,
        keys,
        resolver,
    };
    let mut out = Vec::new();
    diff_mount(&mut ctx, (&ma, &mb), "", 0, &mut out)?;
    Ok(out)
}

struct Ctx<'a, A, B> {
    a: &'a mut A,
    b: &'a mut B,
    keys: &'a GtaViKeys,
    resolver: &'a NameResolver,
}

/// Where an entry's stored bytes are, `(absolute start, length)`, when its row states both.
type Extent = Option<(u64, u64)>;

fn extent(mount: &Gta6Mount, entry: &Gta6Entry, extras: &[Gta6EntryExtra]) -> Extent {
    let start = mount.absolute_start(entry)?;
    let nested = extras.get(entry.index as usize).and_then(|x| x.nested);
    let len = entry
        .stored
        .or_else(|| nested.map(|row| row.stored_size()))?;
    Some((start, len))
}

fn size(entry: &Gta6Entry, stored: Extent) -> Size {
    Size {
        stored: stored.map(|(_, len)| len),
        real: entry.real,
    }
}

fn diff_mount<A: Read + Seek, B: Read + Seek>(
    ctx: &mut Ctx<'_, A, B>,
    (ma, mb): (&Gta6Mount, &Gta6Mount),
    prefix: &str,
    depth: usize,
    out: &mut Vec<Change>,
) -> Result<(), String> {
    let (ea, eb) = (&ma.archive.entries, &mb.archive.entries);
    let (xa, xb) = (ma.archive.entry_extras(), mb.archive.entry_extras());
    let path = |entry: &Gta6Entry| format!("{prefix}{}", entry.display_name());

    // A name64 two entries share pairs them in table order.
    let mut by_name: HashMap<u64, VecDeque<usize>> = HashMap::new();
    for (j, entry) in eb.iter().enumerate() {
        by_name.entry(entry.name64).or_default().push_back(j);
    }
    let mut paired = vec![false; eb.len()];
    let mut lone_a = Vec::new();
    for (i, a) in ea.iter().enumerate() {
        let Some(j) = by_name.get_mut(&a.name64).and_then(VecDeque::pop_front) else {
            lone_a.push(i);
            continue;
        };
        paired[j] = true;
        let b = &eb[j];
        let stored = (extent(ma, a, &xa), extent(mb, b, &xb));
        let same_row = a.real == b.real && a.class == b.class && a.cipher == b.cipher;
        if same_row && same_stored(ctx, stored)? {
            continue;
        }
        if same_body(ctx, (ma, mb), (a, b), stored) {
            out.push(Change::Changed {
                path: path(a),
                before: size(a, stored.0),
                after: size(b, stored.1),
                summary: Some(Summary::Stored),
            });
            continue;
        }
        changed(ctx, (ma, mb), (a, b), stored, path(a), depth, out)?;
    }

    let mut lone_b: Vec<usize> = (0..eb.len()).filter(|&j| !paired[j]).collect();
    for i in lone_a {
        let a = &ea[i];
        let sa = extent(ma, a, &xa);
        let mut renamed = None;
        if sa.is_some() {
            for (k, &j) in lone_b.iter().enumerate() {
                let sb = extent(mb, &eb[j], &xb);
                if same_stored(ctx, (sa, sb))? || same_body(ctx, (ma, mb), (a, &eb[j]), (sa, sb)) {
                    renamed = Some(k);
                    break;
                }
            }
        }
        out.push(match renamed {
            Some(k) => Change::Renamed {
                from: path(a),
                to: path(&eb[lone_b.remove(k)]),
                size: size(a, sa),
            },
            None => Change::Removed {
                path: path(a),
                size: size(a, sa),
            },
        });
    }
    for j in lone_b {
        out.push(Change::Added {
            path: path(&eb[j]),
            size: size(&eb[j], extent(mb, &eb[j], &xb)),
        });
    }
    Ok(())
}

/// Record a changed pair, then what its bodies differ in: a nested archive's own changes follow it.
fn changed<A: Read + Seek, B: Read + Seek>(
    ctx: &mut Ctx<'_, A, B>,
    (ma, mb): (&Gta6Mount, &Gta6Mount),
    (a, b): (&Gta6Entry, &Gta6Entry),
    (sa, sb): (Extent, Extent),
    path: String,
    depth: usize,
    out: &mut Vec<Change>,
) -> Result<(), String> {
    let mark = out.len();
    out.push(Change::Changed {
        path: path.clone(),
        before: size(a, sa),
        after: size(b, sb),
        summary: None,
    });
    let nested = ma.entry_begins_archive(&mut *ctx.a, a).unwrap_or(false)
        && mb.entry_begins_archive(&mut *ctx.b, b).unwrap_or(false);
    let summary = if !nested {
        leaf_summary(ctx, (ma, mb), (a, b), (sa, sb))
    } else if depth >= MAX_DEPTH {
        Some(Summary::Unread {
            why: format!("the archive nests deeper than {MAX_DEPTH} levels"),
        })
    } else {
        let opened = (
            ma.open_nested(&mut *ctx.a, a, ctx.keys),
            mb.open_nested(&mut *ctx.b, b, ctx.keys),
        );
        match opened {
            (Ok(mut ca), Ok(mut cb)) => {
                ca.archive.resolve_names(ctx.resolver);
                cb.archive.resolve_names(ctx.resolver);
                let before = out.len();
                diff_mount(ctx, (&ca, &cb), &format!("{path}/"), depth + 1, out)?;
                Some(Summary::Nested {
                    changes: out.len() - before,
                })
            }
            (Err(e), _) | (_, Err(e)) => Some(Summary::Unread {
                why: format!("a nested table was not read: {e}"),
            }),
        }
    };
    if let Some(Change::Changed { summary: slot, .. }) = out.get_mut(mark) {
        *slot = summary;
    }
    Ok(())
}

/// Whether two extents hold the same stored bytes; two entries with no row both hold nothing.
fn same_stored<A: Read + Seek, B: Read + Seek>(
    ctx: &mut Ctx<'_, A, B>,
    stored: (Extent, Extent),
) -> Result<bool, String> {
    let ((at_a, len), (at_b, len_b)) = match stored {
        (None, None) => return Ok(true),
        (Some(a), Some(b)) => (a, b),
        _ => return Ok(false),
    };
    if len != len_b {
        return Ok(false);
    }
    let step = len.min(COMPARE_STEP) as usize;
    let (mut x, mut y) = (vec![0u8; step], vec![0u8; step]);
    let mut done = 0u64;
    while done < len {
        let n = (len - done).min(COMPARE_STEP) as usize;
        read_exact_at(&mut *ctx.a, at_a + done, &mut x[..n])?;
        read_exact_at(&mut *ctx.b, at_b + done, &mut y[..n])?;
        if x[..n] != y[..n] {
            return Ok(false);
        }
        done += n as u64;
    }
    Ok(true)
}

/// Whether two leaves of one inflated size decode to the same body, though stored apart.
fn same_body<A: Read + Seek, B: Read + Seek>(
    ctx: &mut Ctx<'_, A, B>,
    (ma, mb): (&Gta6Mount, &Gta6Mount),
    (a, b): (&Gta6Entry, &Gta6Entry),
    (sa, sb): (Extent, Extent),
) -> bool {
    if a.real.is_none() || a.real != b.real {
        return false;
    }
    let nested = ma.entry_begins_archive(&mut *ctx.a, a).unwrap_or(true)
        || mb.entry_begins_archive(&mut *ctx.b, b).unwrap_or(true);
    !nested
        && matches!(
            (body(&mut *ctx.a, ma, a, sa, ctx.keys), body(&mut *ctx.b, mb, b, sb, ctx.keys)),
            (Ok(x), Ok(y)) if x == y
        )
}

fn read_exact_at<R: Read + Seek>(src: &mut R, at: u64, buf: &mut [u8]) -> Result<(), String> {
    src.seek(SeekFrom::Start(at))
        .and_then(|_| src.read_exact(buf))
        .map_err(|e| format!("reading {} bytes at {at:#x}: {e}", buf.len()))
}

fn leaf_summary<A: Read + Seek, B: Read + Seek>(
    ctx: &mut Ctx<'_, A, B>,
    (ma, mb): (&Gta6Mount, &Gta6Mount),
    (a, b): (&Gta6Entry, &Gta6Entry),
    (sa, sb): (Extent, Extent),
) -> Option<Summary> {
    let read = (
        body(&mut *ctx.a, ma, a, sa, ctx.keys),
        body(&mut *ctx.b, mb, b, sb, ctx.keys),
    );
    match read {
        (Ok(x), Ok(y)) => summarize(&x, &y),
        (Err(why), _) | (_, Err(why)) => Some(Summary::Unread { why }),
    }
}

/// An entry's body as a reader takes it: deciphered and inflated, or a media entry's raw bytes when
/// they open with a head [`head_kind`] knows.
fn body<R: Read + Seek>(
    src: &mut R,
    mount: &Gta6Mount,
    entry: &Gta6Entry,
    stored: Extent,
    keys: &GtaViKeys,
) -> Result<Vec<u8>, String> {
    let mut sizes = entry.real.into_iter().chain(stored.map(|(_, len)| len));
    if sizes.any(|n| n > SUMMARY_CAP) {
        return Err(format!(
            "a body is over {} MiB, so it is not read",
            SUMMARY_CAP >> 20
        ));
    }
    if entry.cipher.is_some() {
        return mount
            .read_entry_decrypted(&mut *src, entry, keys)
            .map_err(|e| e.to_string());
    }
    if let (Some(Gta6Class::Media), Some((start, len)), Some(real)) =
        (entry.kind(), stored, entry.real)
    {
        if real <= len {
            let mut raw = vec![0u8; real as usize];
            read_exact_at(src, start, &mut raw)?;
            if head_kind(&raw).is_some() {
                return Ok(raw);
            }
        }
    }
    mount
        .read_entry(&mut *src, entry)
        .map_err(|e| e.to_string())
}

/// The kinds of body a summary reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Head {
    Pso,
    Rbf,
    Text,
}

/// What a body's head says it is: `PSIN`, `RBF0`, or text (a `<` or BOM start, or printable).
pub(crate) fn head_kind(body: &[u8]) -> Option<Head> {
    if body.starts_with(b"PSIN") {
        return Some(Head::Pso);
    }
    if body.starts_with(b"RBF0") {
        return Some(Head::Rbf);
    }
    if body.first() == Some(&b'<') || body.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Some(Head::Text);
    }
    let n = body.len().min(64);
    let printable = body[..n]
        .iter()
        .all(|&c| matches!(c, b'\t' | b'\r' | b'\n') || (0x20..=0x7E).contains(&c));
    (n > 0 && printable).then_some(Head::Text)
}

/// The summary of two changed bodies of one kind; `None` when either is of no kind a reader takes.
pub(crate) fn summarize(a: &[u8], b: &[u8]) -> Option<Summary> {
    let format = match (head_kind(a)?, head_kind(b)?) {
        (Head::Text, Head::Text) => return Some(line_diff(a, b)),
        (Head::Pso, Head::Pso) => "PSO",
        (Head::Rbf, Head::Rbf) => "RBF",
        _ => return None,
    };
    let library = SchemaLibrary::default();
    let read = |bytes: &[u8]| MetaDocument::parse(&library, bytes.to_vec());
    match (read(a), read(b)) {
        (Ok(x), Ok(y)) => Some(field_diff(format, &x.instance, &y.instance)),
        (Err(e), _) | (_, Err(e)) => Some(Summary::Unread {
            why: format!("the {format} document was not read: {e}"),
        }),
    }
}

/// The fields of `b` that `a` lacks, those it alone has, and those whose values differ.
pub(crate) fn field_diff(format: &'static str, a: &MetaInstance, b: &MetaInstance) -> Summary {
    let (mut fa, mut fb) = (BTreeMap::new(), BTreeMap::new());
    flatten_instance(&a.type_name, a, &mut fa);
    flatten_instance(&b.type_name, b, &mut fb);
    let (mut removed, mut changed) = (Vec::new(), Vec::new());
    for (path, before) in &fa {
        match fb.get(path) {
            None => removed.push(format!("{path} = {before}")),
            Some(after) if after != before => changed.push(FieldChange {
                path: path.clone(),
                before: before.clone(),
                after: after.clone(),
            }),
            Some(_) => {}
        }
    }
    let added = fb
        .iter()
        .filter(|(path, _)| !fa.contains_key(*path))
        .map(|(path, after)| format!("{path} = {after}"))
        .collect();
    Summary::Fields {
        format,
        added,
        removed,
        changed,
    }
}

fn flatten_instance(prefix: &str, inst: &MetaInstance, out: &mut BTreeMap<String, String>) {
    if inst.fields.is_empty() && inst.extras.is_empty() {
        out.insert(prefix.to_owned(), format!("{} {{}}", inst.type_name));
    }
    for (name, value) in &inst.fields {
        flatten(&format!("{prefix}.{name}"), value, out);
    }
    for (i, node) in inst.extras.iter().enumerate() {
        out.insert(format!("{prefix}.<{}>[{i}]", node.tag), format!("{node:?}"));
    }
}

fn flatten(path: &str, value: &Value, out: &mut BTreeMap<String, String>) {
    match value {
        Value::Struct(inst) | Value::Pointer(Some(inst)) => flatten_instance(path, inst, out),
        Value::Array(items) if !items.is_empty() => {
            for (i, item) in items.iter().enumerate() {
                flatten(&format!("{path}[{i}]"), item, out);
            }
        }
        Value::Map(pairs) if !pairs.is_empty() => {
            for (key, item) in pairs {
                flatten(&format!("{path}[{}]", leaf(key)), item, out);
            }
        }
        Value::Array(_) | Value::Map(_) => {
            out.insert(path.to_owned(), "[]".to_owned());
        }
        other => {
            out.insert(path.to_owned(), leaf(other));
        }
    }
}

fn leaf(value: &Value) -> String {
    match value {
        Value::Null | Value::Pointer(None) => "null".to_owned(),
        Value::Bool(v) => v.to_string(),
        Value::Int(v) => v.to_string(),
        Value::UInt(v) => v.to_string(),
        Value::Float(v) => v.to_string(),
        Value::String(s) => format!("{s:?}"),
        Value::Hash(h) => format!("hash_{:08X}", h.hash),
        Value::Enum { variant, .. } => variant.clone(),
        Value::Bitset { bits, .. } => format!("{bits:#x}"),
        Value::Color(c) => format!("{c:#010x}"),
        Value::Raw(s) => s.clone(),
        Value::ByteArray(bytes) => {
            let mut hasher = DefaultHasher::new();
            bytes.hash(&mut hasher);
            format!("{} bytes #{:016x}", bytes.len(), hasher.finish())
        }
        other => format!("{other:?}"),
    }
}

/// The lines of `b` past those `a` holds, and the lines of `a` past those `b` holds, in order.
pub(crate) fn line_diff(a: &[u8], b: &[u8]) -> Summary {
    let (ta, tb) = (String::from_utf8_lossy(a), String::from_utf8_lossy(b));
    let past = |from: &str, against: &str| {
        let mut held: HashMap<&str, usize> = HashMap::new();
        for line in against.lines() {
            *held.entry(line).or_default() += 1;
        }
        let mut extra = Vec::new();
        for line in from.lines() {
            match held.get_mut(line) {
                Some(n) if *n > 0 => *n -= 1,
                _ => extra.push(line.to_owned()),
            }
        }
        extra
    };
    Summary::Lines {
        added: past(&tb, &ta),
        removed: past(&ta, &tb),
    }
}

// --- the text report ----------------------------------------------------------

/// The plain-text report of `changes` between the archives labelled `a` and `b`.
pub(crate) fn render(a: &str, b: &str, changes: &[Change]) -> String {
    if changes.is_empty() {
        return format!("{a} and {b}: no entry differs\n");
    }
    let count = |want: fn(&Change) -> bool| changes.iter().filter(|&c| want(c)).count();
    let mut out = format!(
        "{a} -> {b}: {} added, {} removed, {} renamed, {} changed\n",
        count(|c: &Change| matches!(c, Change::Added { .. })),
        count(|c: &Change| matches!(c, Change::Removed { .. })),
        count(|c: &Change| matches!(c, Change::Renamed { .. })),
        count(|c: &Change| matches!(c, Change::Changed { .. })),
    );
    for change in changes {
        match change {
            Change::Added { path, size } => out += &format!("  + {path}  {}\n", sizes(size)),
            Change::Removed { path, size } => out += &format!("  - {path}  {}\n", sizes(size)),
            Change::Renamed { from, to, size } => {
                out += &format!("  > {from} -> {to}  {}\n", sizes(size));
            }
            Change::Changed {
                path,
                before,
                after,
                summary,
            } => {
                out += &format!("  ~ {path}  {} -> {}\n", sizes(before), sizes(after));
                if let Some(summary) = summary {
                    render_summary(summary, &mut out);
                }
            }
        }
    }
    out
}

fn sizes(size: &Size) -> String {
    let n = |v: Option<u64>| v.map_or_else(|| "-".to_owned(), |v| v.to_string());
    format!("stored {}, real {}", n(size.stored), n(size.real))
}

fn render_summary(summary: &Summary, out: &mut String) {
    match summary {
        Summary::Fields {
            format,
            added,
            removed,
            changed,
        } => {
            *out += &format!(
                "      {format}: {} field(s) changed, {} added, {} removed\n",
                changed.len(),
                added.len(),
                removed.len()
            );
            let lines = changed
                .iter()
                .map(|c| format!("{}: {} -> {}", c.path, c.before, c.after));
            shown(out, "", lines);
            shown(out, "+ ", added.iter().cloned());
            shown(out, "- ", removed.iter().cloned());
        }
        Summary::Lines { added, removed } => {
            *out += &format!(
                "      text: {} line(s) added, {} removed\n",
                added.len(),
                removed.len()
            );
            shown(out, "+ ", added.iter().cloned());
            shown(out, "- ", removed.iter().cloned());
        }
        Summary::Nested { changes } => {
            *out += &format!("      nested archive: {changes} change(s) inside\n");
        }
        Summary::Stored => *out += "      stored apart only: the decoded bodies are the same\n",
        Summary::Unread { why } => *out += &format!("      not compared: {why}\n"),
    }
}

/// The first [`SHOWN`] of `items`, each cut to 160 characters, and how many more there are.
fn shown(out: &mut String, mark: &str, items: impl Iterator<Item = String>) {
    let mut more = 0usize;
    for (i, item) in items.enumerate() {
        if i >= SHOWN {
            more += 1;
            continue;
        }
        let cut: String = item.chars().take(160).collect();
        *out += &format!("        {mark}{cut}\n");
    }
    if more > 0 {
        *out += &format!("        ... and {more} more\n");
    }
}

#[cfg(test)]
#[path = "rpf9_diff_tests.rs"]
mod tests;
