//! `hashes mine`: names for the hashes of a GTA VI install that no table names. Candidates are the
//! strings the install's bodies and the user's executable state, and what rules make of the names
//! already resolved; a candidate is kept only where its hash equals an unresolved one, and the
//! matches go to the user's file as `0xHASH<TAB>name`, the rows `NameResolver` and `ParserNames` read.

use std::collections::hash_map::RandomState;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::File;
use std::hash::BuildHasher;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::container::rpf::{Gta6Entry, Gta6Mount, GtaViKeys, NameResolver, GTA6_EXE_ENV};
use or_job::Cancel;

use super::census_cap::{Cap, CapUnit};
use or_io::gta6::hash::{name_hash32, name_hash64};
use or_io::gta6::names::{vocabulary_file, Vocabulary};
use or_io::meta::binary::layout::ContainerLayout;
use or_io::meta::binary::pso::schema::{PsoSchemaItem, ARRAY_INFO_NAME_HASH};
use or_io::meta::binary::pso::string_names::document_strings;
use or_io::meta::binary::ragepar_names::{self, ParserNames};
use or_io::meta::binary::{detect_binary_format, parse_meta_binary_full, BinaryFormat};
use or_io::meta::diagnostics::Diagnostics;
use or_io::meta::schema::SchemaLibrary;
use or_io::{literal_string_hash, string_hash};

const USAGE: &str = "\
usage: openrage-cli hashes mine <install-folder | archive.rpf> --out <file> [--names <dir>] [--keys <snapshot>] [--exe <exe>] [--range <count>] [--max-bodies <n>|unlimited]
  Writes the names it finds to <file> as 0xHASH<TAB>name rows; rows already in <file> are kept.
  Name the file resolved-names.tsv in the folder --names or OPENRAGE_GTA6_NAMES reads, and the
  explorer and the metadata reader take its names. No archive is written.";

const MIN_CANDIDATE: usize = 3;
const MAX_CANDIDATE: usize = 160;
/// The longest structure or member name tried.
const MAX_IDENT: usize = 96;
/// A body stated larger than this is not read: it is a movie or a stream, which states no name.
const MAX_BODY: u64 = 64 << 20;
/// The bytes of a body whose strings are taken.
const BODY_PREFIX: usize = 16 << 20;
const MAX_DEPTH: usize = 8;
/// The directory-and-stem joins tried in one table at most.
const JOIN_CAP: usize = 2_000_000;
const EXE_WINDOW: u64 = 16 << 20;
/// The bytes one executable window carries into the next, so a string across the edge is whole once.
const EXE_OVERLAP: usize = 256;
/// The rounds of rules over the names each round found.
const ROUNDS: usize = 3;
/// The extensions and suffixes the rules try, the commonest first.
const RULE_TOP: usize = 64;
const DEFAULT_RANGE: u32 = 100;
/// A body larger than this is not read for its schema in the first walk.
const SCHEMA_BODY: u64 = 4 << 20;
/// The candidate hashes held to skip a repeated try; cleared past this, as a repeat is cheap.
const SEEN_CAP: usize = 1 << 22;
const BODIES_FLAG: &str = "--max-bodies";
const DEFAULT_BODIES: u64 = 200_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Space {
    Entry,
    Class,
    Member,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
enum Source {
    #[default]
    Install,
    Executable,
    Rules,
}

/// The unresolved hashes of an install, the candidates tried against them and what matched.
#[derive(Default)]
struct Miner {
    /// Unresolved entry names, each with the 32-bit name its entry states beside it.
    entry: HashMap<u64, Option<u32>>,
    /// Unresolved structure and enum names.
    class: HashSet<u32>,
    /// Unresolved member and enum-value names.
    member: HashSet<u32>,
    /// The 64-bit and 32-bit hashes the kept rows or a name table already name.
    known64: HashSet<u64>,
    known32: HashSet<u32>,
    /// The names the tables' entries already have, the rules' seeds.
    resolved: Vec<String>,
    held: HashSet<String>,
    found64: BTreeMap<u64, String>,
    found_class: BTreeMap<u32, String>,
    found_member: BTreeMap<u32, String>,
    /// Every name found, in the order found.
    order: Vec<String>,
    by: BTreeMap<(Space, Source), usize>,
    source: Source,
    tried32: u64,
    seen: HashSet<u64>,
    state: RandomState,
    tables: usize,
    bodies: usize,
    skipped: usize,
    joins_skipped: usize,
    schemas_unread: usize,
}

impl Miner {
    fn new() -> Self {
        Self::default()
    }

    /// Takes the hashes the rows of an existing output file name as named already.
    fn keep_rows(&mut self, text: &str) {
        for line in text.lines() {
            let Some(digits) = (line.split('\t').next()).and_then(|h| h.strip_prefix("0x")) else {
                continue;
            };
            match digits.len() {
                16 => self.known64.extend(u64::from_str_radix(digits, 16).ok()),
                8 => self.known32.extend(u32::from_str_radix(digits, 16).ok()),
                _ => {}
            }
        }
    }

    /// Takes the unnamed entries of one table as targets; the names `resolver` gives the others.
    fn add_table(&mut self, entries: &[Gta6Entry], resolver: &NameResolver) -> Vec<String> {
        let mut named = Vec::new();
        for entry in entries.iter().filter(|e| e.name64 != 0) {
            match resolver.lookup(entry.name64) {
                Some(name) => {
                    if self.held.insert(name.to_string()) {
                        self.resolved.push(name.to_string());
                    }
                    named.push(name.to_string());
                }
                None if !self.known64.contains(&entry.name64) => {
                    self.entry.entry(entry.name64).or_insert(entry.name32);
                }
                None => {}
            }
        }
        named
    }

    /// Mines the bodies of one table, then joins the directories its names hold with the last
    /// component of each string they state.
    fn mine_table(
        &mut self,
        named: &[String],
        bodies: impl IntoIterator<Item = Vec<u8>>,
        keys: Option<&GtaViKeys>,
    ) {
        self.tables += 1;
        let mark = self.order.len();
        let mut basenames = BTreeSet::new();
        for body in bodies {
            self.bodies += 1;
            self.body(&body, keys, &mut basenames);
        }
        let found = self.order[mark..].to_vec();
        let dirs: BTreeSet<String> = (named.iter().chain(&found))
            .filter_map(|name| name.rfind('/').map(|at| name[..=at].to_ascii_lowercase()))
            .collect();
        let was = std::mem::replace(&mut self.source, Source::Rules);
        let mut budget = JOIN_CAP;
        for dir in &dirs {
            for base in &basenames {
                if budget == 0 {
                    self.joins_skipped += 1;
                    continue;
                }
                budget -= 1;
                self.try_name(&format!("{dir}{base}"));
            }
        }
        self.source = was;
    }

    fn body(&mut self, bytes: &[u8], keys: Option<&GtaViKeys>, basenames: &mut BTreeSet<String>) {
        if detect_binary_format(bytes) == Some(BinaryFormat::Pso) {
            // The first walk took the schema of every body up to this size.
            if bytes.len() as u64 > SCHEMA_BODY {
                self.schema_targets(bytes);
            }
            for text in document_strings(bytes, keys).unwrap_or_default() {
                self.consider(text.as_bytes(), Some(basenames));
            }
        }
        self.consider(&bytes[..bytes.len().min(BODY_PREFIX)], Some(basenames));
    }

    /// Takes the structure, enum, member and value hashes of a PSO body's schema no table names.
    fn schema_targets(&mut self, bytes: &[u8]) {
        let library = SchemaLibrary::default();
        let Ok((_, layout)) = parse_meta_binary_full(&library, bytes, &mut Diagnostics::lenient())
        else {
            self.schemas_unread += 1;
            return;
        };
        let ContainerLayout::Pso(pso) = layout.container else {
            return;
        };
        let unnamed = |hash: &u32| *hash != 0 && !self.known32.contains(hash);
        let (mut classes, mut members) = (Vec::new(), Vec::new());
        for item in &pso.schema.items {
            match item {
                PsoSchemaItem::Struct(s) => {
                    classes.push(s.name_hash);
                    let real = s
                        .entries
                        .iter()
                        .filter(|e| e.name_hash != ARRAY_INFO_NAME_HASH);
                    members.extend(real.map(|e| e.name_hash));
                }
                PsoSchemaItem::Enum(e) => {
                    classes.push(e.name_hash);
                    members.extend(e.entries.iter().map(|v| v.name_hash));
                }
            }
        }
        classes.retain(unnamed);
        members.retain(unnamed);
        self.class.extend(classes);
        self.member.extend(members);
    }

    /// Tries each string `bytes` states; the last component of each goes into `basenames`.
    fn consider(&mut self, bytes: &[u8], mut basenames: Option<&mut BTreeSet<String>>) {
        candidates(bytes, &mut |piece, last| {
            self.try_name(piece);
            if let (true, Some(held)) = (
                last && piece.len() >= MIN_CANDIDATE,
                basenames.as_deref_mut(),
            ) {
                held.insert(piece.to_ascii_lowercase());
            }
        });
    }

    /// The names of the kept vocabulary, which a harvest of the install's bodies gathered.
    fn mine_vocabulary(&mut self, vocabulary: &Vocabulary) {
        let mut text = Vec::new();
        if vocabulary.write(&mut text).is_err() {
            return;
        }
        // Past the head line, each line is a two-character kind, a space and the name.
        for line in text.split(|b| *b == b'\n').skip(1) {
            if let Some(name) = line.get(3..) {
                self.consider(name, None);
            }
        }
    }

    /// The strings of the executable at `path`, read a window at a time.
    fn mine_exe(&mut self, path: &Path) -> std::io::Result<()> {
        self.source = Source::Executable;
        let mut file = File::open(path)?;
        let mut carry = Vec::new();
        loop {
            let mut window = std::mem::take(&mut carry);
            let had = window.len();
            (&mut file).take(EXE_WINDOW).read_to_end(&mut window)?;
            if window.len() == had {
                break;
            }
            self.consider(&window, None);
            carry = window[window.len().saturating_sub(EXE_OVERLAP)..].to_vec();
        }
        self.source = Source::Install;
        Ok(())
    }

    fn try_name(&mut self, name: &str) {
        if self.seen.len() >= SEEN_CAP {
            self.seen.clear();
        }
        let fresh = self.seen.insert(self.state.hash_one(name));
        if !fresh || !(MIN_CANDIDATE..=MAX_CANDIDATE).contains(&name.len()) {
            return;
        }
        // A plain file is named with its extension, a resource and a nested archive without.
        let lower = name.to_ascii_lowercase();
        self.try_entry(&lower);
        let stem = stem_of(&lower);
        if stem.len() < lower.len() {
            self.try_entry(stem);
        }
        if is_ident(name) {
            self.try_ident(name, self.source);
        }
    }

    fn try_entry(&mut self, form: &str) {
        let word = name_hash64(form.as_bytes());
        let Some(&said) = self.entry.get(&word) else {
            return;
        };
        // The 32-bit name the entry states beside the word has to agree.
        if said.is_some_and(|hash| hash != name_hash32(form.as_bytes()))
            || self.found64.contains_key(&word)
        {
            return;
        }
        self.found64.insert(word, form.to_string());
        self.record(Space::Entry, self.source, form);
    }

    /// A structure name is hashed as written; a member name as written or case-folded, both occur.
    fn try_ident(&mut self, name: &str, source: Source) {
        self.tried32 += 1;
        let literal = literal_string_hash(name.as_bytes());
        if self.class.contains(&literal) && !self.found_class.contains_key(&literal) {
            self.found_class.insert(literal, name.to_string());
            self.record(Space::Class, source, name);
        }
        for hash in [literal, string_hash(name.as_bytes())] {
            if self.member.contains(&hash) && !self.found_member.contains_key(&hash) {
                self.found_member.insert(hash, name.to_string());
                self.record(Space::Member, source, name);
            }
        }
    }

    fn record(&mut self, space: Space, source: Source, name: &str) {
        *self.by.entry((space, source)).or_default() += 1;
        self.order.push(name.to_string());
    }

    /// The rules over the names resolved, `extra` and those found, then over what each round found.
    fn rules(&mut self, extra: impl IntoIterator<Item = String>, range: u32) {
        self.source = Source::Rules;
        let mut work: Vec<String> = self.resolved.clone();
        work.extend(extra);
        work.extend(self.order.iter().cloned());
        let exts = work.iter().filter_map(|name| {
            let ext = &name[stem_of(name).len()..];
            (2..=8)
                .contains(&ext.len())
                .then(|| ext.to_ascii_lowercase())
        });
        let exts = ranked(counted(exts), 1);
        let suffixes = known_suffixes(&work);
        let mut expanded = HashSet::new();
        for _ in 0..ROUNDS {
            let mark = self.order.len();
            for name in &work {
                if expanded.insert(name.clone()) {
                    rule_candidates(name, &exts, &suffixes, range, &mut |c| self.try_name(c));
                }
            }
            work = self.order[mark..].to_vec();
            if work.is_empty() {
                break;
            }
        }
        self.source = Source::Install;
    }

    /// The rows found, entry names first, as `NameResolver::add_hash_name_tsv` reads them.
    fn rows(&self) -> String {
        let mut text = String::new();
        for (hash, name) in &self.found64 {
            text.push_str(&format!("0x{hash:016X}\t{name}\n"));
        }
        let members =
            (self.found_member.iter()).filter(|(h, _)| !self.found_class.contains_key(*h));
        for (hash, name) in self.found_class.iter().chain(members) {
            text.push_str(&format!("0x{hash:08X}\t{name}\n"));
        }
        text
    }

    /// The report: what was read, and per space how many unresolved hashes were named and how.
    fn lines(&self, refused: usize) -> Vec<String> {
        let by = |space, source| self.by.get(&(space, source)).copied().unwrap_or(0);
        let sources = |space| {
            format!(
                "{} from the install's strings, {} from the executable, {} by the rules",
                by(space, Source::Install),
                by(space, Source::Executable),
                by(space, Source::Rules)
            )
        };
        // The 32-bit spaces carry a chance rate: each identifier tried names a hash by chance once in 2^32.
        let chance = |flavours: f64, unresolved: usize| {
            flavours * self.tried32 as f64 * unresolved as f64 / 4_294_967_296.0
        };
        let mut lines = vec![
            format!(
                "  tables        {} read, {refused} refused; {} bodies read, {} not read",
                self.tables, self.bodies, self.skipped
            ),
            format!(
                "  entry names   {} of {} unresolved 64-bit names: {}",
                self.found64.len(),
                self.entry.len(),
                sources(Space::Entry)
            ),
            format!(
                "  class names   {} of {} unresolved structure and enum hashes: {}; about {:.1} by chance over {} identifiers",
                self.found_class.len(),
                self.class.len(),
                sources(Space::Class),
                chance(1.0, self.class.len()),
                self.tried32
            ),
            format!(
                "  member names  {} of {} unresolved member and value hashes: {}; about {:.1} by chance",
                self.found_member.len(),
                self.member.len(),
                sources(Space::Member),
                chance(2.0, self.member.len())
            ),
        ];
        if self.joins_skipped > 0 {
            lines.push(format!(
                "  joins         {} not tried past {JOIN_CAP} in a table",
                self.joins_skipped
            ));
        }
        if self.schemas_unread > 0 {
            lines.push(format!(
                "  schemas       {} PSO bodies whose schema did not read",
                self.schemas_unread
            ));
        }
        lines
    }
}

/// A byte a path or a name is spelled with.
fn path_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"_-+@./\\:$#~".contains(&b)
}

/// Each path-shaped run of `bytes`, then each tail of it after a `/`; `each` is told whether the
/// piece is the run's last component.
fn candidates(bytes: &[u8], each: &mut impl FnMut(&str, bool)) {
    for run in bytes.split(|b| !path_byte(*b)) {
        if !(MIN_CANDIDATE..=MAX_CANDIDATE).contains(&run.len())
            || !run.iter().any(u8::is_ascii_alphabetic)
        {
            continue;
        }
        let text: String = (run.iter())
            .map(|&b| if b == b'\\' { '/' } else { char::from(b) })
            .collect();
        let text = text.trim_end_matches('.');
        let mut at = 0;
        loop {
            let piece = &text[at..];
            let next = piece.find('/');
            if !piece.is_empty() {
                each(piece, next.is_none());
            }
            match next {
                Some(n) => at += n + 1,
                None => break,
            }
        }
    }
}

/// `name` without the extension of its last component.
fn stem_of(name: &str) -> &str {
    let from = name.rfind('/').map_or(0, |at| at + 1);
    match name[from..].rfind('.') {
        Some(dot) if dot > 0 => &name[..from + dot],
        _ => name,
    }
}

/// Whether `name` is shaped as a C++ structure or member name.
fn is_ident(name: &str) -> bool {
    let bytes = name.as_bytes();
    name.len() <= MAX_IDENT
        && (bytes[0].is_ascii_alphabetic() || bytes[0] == b'_')
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b':'))
}

fn counted(items: impl Iterator<Item = String>) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for item in items {
        *counts.entry(item).or_default() += 1;
    }
    counts
}

/// The [`RULE_TOP`] commonest items of `counts` seen at least `least` times.
fn ranked(counts: HashMap<String, usize>, least: usize) -> Vec<String> {
    let mut ranked: Vec<(String, usize)> =
        counts.into_iter().filter(|(_, n)| *n >= least).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked
        .into_iter()
        .take(RULE_TOP)
        .map(|(item, _)| item)
        .collect()
}

/// The `_xxx` endings that turn one stem of `names` into another, seen twice or more.
fn known_suffixes(names: &[String]) -> Vec<String> {
    let mut stems: Vec<&str> = names.iter().map(|n| stem_of(n)).collect();
    stems.sort_unstable();
    stems.dedup();
    let mut endings = Vec::new();
    for (i, a) in stems.iter().enumerate() {
        for b in stems[i + 1..]
            .iter()
            .take(32)
            .take_while(|b| b.starts_with(a))
        {
            let rest = &b[a.len()..];
            if (2..=12).contains(&rest.len()) && rest.starts_with('_') && !rest.contains('/') {
                endings.push(rest.to_string());
            }
        }
    }
    ranked(counted(endings.into_iter()), 2)
}

/// What the rules make of `name`: its stem with each extension of `exts`, its stem without a suffix
/// of `suffixes` and with each, and where the stem ends in digits, each number below `range` that
/// many digits wide, with the name's own extension and without.
fn rule_candidates(
    name: &str,
    exts: &[String],
    suffixes: &[String],
    range: u32,
    each: &mut impl FnMut(&str),
) {
    let stem = stem_of(name);
    let ext = &name[stem.len()..];
    for other in exts {
        each(&format!("{stem}{other}"));
    }
    let base = (suffixes.iter().find(|s| stem.ends_with(s.as_str())))
        .map_or(stem, |s| &stem[..stem.len() - s.len()]);
    each(base);
    for suffix in suffixes {
        each(&format!("{base}{suffix}"));
        if !ext.is_empty() {
            each(&format!("{base}{suffix}{ext}"));
        }
    }
    let digits = stem.bytes().rev().take_while(u8::is_ascii_digit).count();
    if !(1..=4).contains(&digits) {
        return;
    }
    let head = &stem[..stem.len() - digits];
    for n in 0..range.min(10u32.pow(digits as u32)) {
        let numbered = format!("{head}{n:0digits$}");
        each(&numbered);
        if !ext.is_empty() {
            each(&format!("{numbered}{ext}"));
        }
    }
}

type Visit<'a> = dyn FnMut(&mut File, &Gta6Mount, &[usize]) + 'a;

/// Visits the table of `mount` with the entries that are not archives, then each nested table.
fn walk(
    file: &mut File,
    mount: &Gta6Mount,
    keys: &GtaViKeys,
    depth: usize,
    refused: &mut usize,
    visit: &mut Visit<'_>,
) {
    let (mut leaves, mut nested) = (Vec::new(), Vec::new());
    for (at, entry) in mount.archive.entries.iter().enumerate() {
        match mount.entry_begins_archive(&mut *file, entry) {
            Ok(false) => leaves.push(at),
            Ok(true) if depth < MAX_DEPTH => nested.push(at),
            _ => *refused += 1,
        }
    }
    visit(file, mount, &leaves);
    for at in nested {
        match mount.open_nested(&mut *file, &mount.archive.entries[at], keys) {
            Ok(child) => walk(file, &child, keys, depth + 1, refused, visit),
            Err(_) => *refused += 1,
        }
    }
}

/// Visits every table of the archive at `path`; an archive whose table does not open is refused.
fn walk_archive(path: &Path, keys: &GtaViKeys, refused: &mut usize, visit: &mut Visit<'_>) {
    let opened = File::open(path)
        .map_err(|e| e.to_string())
        .and_then(|mut file| {
            let root = Gta6Mount::open_root(&mut file, keys).map_err(|e| e.to_string())?;
            Ok((file, root))
        });
    match opened {
        Ok((mut file, root)) => walk(&mut file, &root, keys, 0, refused, visit),
        Err(e) => {
            eprintln!("  {}: {e}", path.display());
            *refused += 1;
        }
    }
}

/// The inflated body of `entry`, deciphered where it is enciphered; `None` where it cannot be read.
fn read_body(
    file: &mut File,
    mount: &Gta6Mount,
    entry: &Gta6Entry,
    keys: &GtaViKeys,
) -> Option<Vec<u8>> {
    if entry
        .real
        .max(entry.stored)
        .is_some_and(|len| len > MAX_BODY)
    {
        return None;
    }
    match entry.cipher {
        Some(_) if !keys.has_content_keys() => None,
        Some(_) => mount.read_entry_decrypted(&mut *file, entry, keys).ok(),
        None => mount.read_entry(&mut *file, entry).ok(),
    }
}

/// `install` where it is an archive, else every `*.rpf` in it and in each folder of its `pack`.
fn archives(install: &Path) -> Vec<PathBuf> {
    if install.is_file() {
        return vec![install.to_path_buf()];
    }
    let rpfs_in = |dir: &Path| -> Vec<PathBuf> {
        let listed = std::fs::read_dir(dir).into_iter().flatten().flatten();
        let paths = listed.map(|e| e.path());
        paths
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("rpf")))
            .collect()
    };
    let mut out = rpfs_in(install);
    let packs = std::fs::read_dir(install.join("pack"))
        .into_iter()
        .flatten()
        .flatten();
    for pack in packs.map(|e| e.path()).filter(|p| p.is_dir()) {
        out.extend(rpfs_in(&pack));
    }
    out.sort();
    out
}

/// The first `*.exe` directly in `install`.
fn first_exe(install: &Path) -> Option<PathBuf> {
    let listed = std::fs::read_dir(install).ok()?.flatten().map(|e| e.path());
    let mut exes: Vec<PathBuf> = listed
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe")))
        .collect();
    exes.sort();
    exes.into_iter().next()
}

/// The names folder `dir` or `OPENRAGE_GTA6_NAMES` gives, with the vocabulary kept in the configuration folder.
fn load_resolver(dir: Option<&Path>) -> NameResolver {
    crate::gta6_name_sources::load(dir)
}

/// Writes the rows `kept` held before and then the rows found to `out`; how many were found.
fn write_rows(out: &Path, kept: &str, miner: &Miner) -> std::io::Result<usize> {
    let directory = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(directory)?;
    let name = out.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the names output needs a file name",
        )
    })?;
    let beside = |suffix: &str| {
        let mut adjacent = std::ffi::OsString::from(".");
        adjacent.push(name);
        adjacent.push(suffix);
        directory.join(adjacent)
    };
    // A mining snapshot can be stale; cache output must share the cache writer's lock.
    let lock_path = if name == std::ffi::OsStr::new("resolved-names.tsv") {
        directory.join(".resolved-names.lock")
    } else {
        beside(".lock")
    };
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.lock()?;
    let mut text = match std::fs::read_to_string(out) {
        Ok(latest) => latest,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => kept.to_string(),
        Err(error) => return Err(error),
    };
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    let mut present: HashSet<String> = text.lines().map(str::to_owned).collect();
    let rows = miner.rows();
    for row in rows.lines() {
        if present.insert(row.to_string()) {
            text.push_str(row);
            text.push('\n');
        }
    }
    let temporary = beside(".tmp");
    let result = (|| {
        let mut file = File::create(&temporary)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, out)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result?;
    Ok(rows.lines().count())
}

struct Options {
    install: PathBuf,
    out: PathBuf,
    names: Option<PathBuf>,
    keys: Option<PathBuf>,
    exe: Option<PathBuf>,
    range: u32,
    bodies: Cap,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let (mut install, mut out, mut names, mut keys, mut exe) = (None, None, None, None, None);
    let mut range = DEFAULT_RANGE;
    let mut bodies = Cap::new(DEFAULT_BODIES, CapUnit::Count("bodies"));
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if let Some(v) = arg.strip_prefix("--max-bodies=") {
            bodies = bodies.parse(BODIES_FLAG, v)?;
            continue;
        }
        let mut value = |what: &str| {
            (it.next().map(PathBuf::from)).ok_or_else(|| format!("{arg} needs {what}"))
        };
        match arg.as_str() {
            "--out" => out = Some(value("a file")?),
            "--names" => names = Some(value("a folder")?),
            "--keys" => keys = Some(value("a snapshot file")?),
            "--exe" => exe = Some(value("the executable")?),
            BODIES_FLAG => {
                let v = value("a count")?;
                bodies = bodies.parse(BODIES_FLAG, &v.to_string_lossy())?;
            }
            "--range" => {
                let count = value("a count")?;
                range = (count
                    .to_str()
                    .and_then(|c| c.parse().ok())
                    .filter(|n| *n > 0))
                .ok_or("--range needs a count")?;
            }
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            other if install.is_none() => install = Some(PathBuf::from(other)),
            _ => return Err("give one install".to_string()),
        }
    }
    Ok(Options {
        install: install.ok_or("give an install folder or an archive")?,
        out: out.ok_or("give --out, the file the names are written to")?,
        names,
        keys,
        exe,
        range,
        bodies,
    })
}

/// How far the walk reads: the bodies it may mine, and the flag Ctrl-C sets.
struct Bounds<'a> {
    bodies: Cap,
    cancel: &'a Cancel,
}

/// Walks every table of `archives` for its unnamed entries and its schemas' hashes, then again for
/// its bodies' strings, so each string is tried against every unresolved hash as it is read; how
/// many tables were refused.
fn mine_archives(
    miner: &mut Miner,
    archives: &[PathBuf],
    keys: &GtaViKeys,
    resolver: &NameResolver,
    bounds: &Bounds<'_>,
) -> usize {
    let mut refused = 0;
    for path in archives {
        walk_archive(path, keys, &mut refused, &mut |file, mount, leaves| {
            let entries = &mount.archive.entries;
            miner.add_table(entries, resolver);
            for &at in leaves {
                let entry = &entries[at];
                if bounds.cancel.is_set()
                    || entry
                        .real
                        .max(entry.stored)
                        .is_some_and(|n| n > SCHEMA_BODY)
                {
                    continue;
                }
                let body = read_body(&mut *file, mount, entry, keys);
                if let Some(body) =
                    body.filter(|b| detect_binary_format(b) == Some(BinaryFormat::Pso))
                {
                    miner.schema_targets(&body);
                }
            }
        });
    }
    for path in archives {
        walk_archive(path, keys, &mut 0, &mut |file, mount, leaves| {
            let entries = &mount.archive.entries;
            let named: Vec<String> = (entries.iter())
                .filter_map(|e| resolver.lookup(e.name64).map(str::to_owned))
                .collect();
            let (mut skipped, mut taken) = (0, miner.bodies as u64);
            let bodies = leaves.iter().filter_map(|&at| {
                if bounds.cancel.is_set() || bounds.bodies.reached(taken) {
                    return None;
                }
                let body = read_body(&mut *file, mount, &entries[at], keys);
                skipped += usize::from(body.is_none());
                taken += u64::from(body.is_some());
                body
            });
            miner.mine_table(&named, bodies, Some(keys));
            miner.skipped += skipped;
        });
        eprintln!("  {}: {} bodies read so far", path.display(), miner.bodies);
    }
    refused
}

fn mine(options: &Options) -> Result<Vec<String>, String> {
    let keys = crate::gta6_keys::load(options.keys.as_deref(), options.exe.as_deref())?;
    let resolver = load_resolver(options.names.as_deref());
    let local = match &options.names {
        Some(dir) => Some(ParserNames::load_dir(dir).map_err(|e| e.to_string())?),
        None => None,
    };
    let kept = match std::fs::read_to_string(&options.out) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", options.out.display())),
    };
    let archives = archives(&options.install);
    if archives.is_empty() {
        return Err(format!("no .rpf in {}", options.install.display()));
    }
    let mut miner = Miner::new();
    miner.keep_rows(&kept);
    let mut seeds32 = Vec::new();
    for names in [local.as_ref(), ragepar_names::installed()]
        .into_iter()
        .flatten()
    {
        for (hash, name) in names.pairs() {
            miner.known32.insert(hash);
            seeds32.push(name.to_string());
        }
    }
    let cancel = super::jobs::on_interrupt();
    let bounds = Bounds {
        bodies: options.bodies,
        cancel: &cancel,
    };
    let refused = mine_archives(&mut miner, &archives, &keys, &resolver, &bounds);
    if let Some(config) = or_title::config::config_dir() {
        miner.mine_vocabulary(&Vocabulary::load(&vocabulary_file(&config)));
    }
    let exe = (options.exe.clone())
        .or_else(|| std::env::var_os(GTA6_EXE_ENV).map(PathBuf::from))
        .or_else(|| first_exe(&options.install));
    if let Some(exe) = exe.as_ref().filter(|_| !cancel.is_set()) {
        miner
            .mine_exe(exe)
            .map_err(|e| format!("{}: {e}", exe.display()))?;
    }
    miner.rules(seeds32, options.range);
    let found = write_rows(&options.out, &kept, &miner)
        .map_err(|e| format!("{}: {e}", options.out.display()))?;
    if let Some(config) = or_title::config::config_dir() {
        if let Err(error) = NameResolver::cache_names(
            &config.join("gta6_names"),
            miner
                .found64
                .iter()
                .map(|(&hash, name)| (hash, name.as_str())),
        ) {
            tracing::warn!(%error, "could not update the local GTA VI names cache");
        }
    }
    let mut lines = miner.lines(refused);
    if exe.is_none() {
        lines.push(format!(
            "  executable    none read: pass --exe or set {GTA6_EXE_ENV}"
        ));
    }
    lines.extend(options.bodies.note(miner.bodies as u64, BODIES_FLAG));
    lines.push(format!(
        "  written       {}: {found} rows found, after the rows it held; no archive is written",
        options.out.display()
    ));
    if cancel.is_set() {
        lines.push(
            "  CANCELLED     the walk stopped short; the rows found before it are written"
                .to_string(),
        );
    }
    Ok(lines)
}

pub(crate) fn run(args: &[String]) -> ExitCode {
    let Some(("mine", rest)) = args.split_first().map(|(verb, rest)| (verb.as_str(), rest)) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let options = match parse(rest) {
        Ok(options) => options,
        Err(e) => {
            eprintln!("Error: {e}");
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match mine(&options) {
        Ok(lines) => {
            lines.iter().for_each(|line| println!("{line}"));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("Error: {e}");
            ExitCode::FAILURE
        }
    }
}

// Shared with the `rpf9` tests, which use parts of it this file does not.
#[cfg(test)]
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "rpf9_fixture.rs"]
mod fixture;

#[cfg(test)]
mod tests {
    use super::fixture::{self, Body};
    use super::*;

    fn entry(index: u32, path: &str, name32: Option<u32>) -> Gta6Entry {
        Gta6Entry {
            index,
            name64: name_hash64(path.as_bytes()),
            name32,
            name: None,
            start: None,
            stored: None,
            real: None,
            class: None,
            cipher: None,
        }
    }

    /// A synthetic table: one entry the resolver names, the unnamed ones the rules and a body name, a
    /// decoy no string states, and an entry whose stated 32-bit name is not its path's.
    fn table() -> (Vec<Gta6Entry>, NameResolver) {
        let paths = [
            "zq/props/crate_01",
            "zq/props/crate_07",
            "zq/props/lamp_post",
            "zq/data/levels.xml",
            "zq/props/never_said",
        ];
        let mut entries: Vec<Gta6Entry> = (paths.iter().zip(0..))
            .map(|(path, at)| entry(at, path, Some(name_hash32(path.as_bytes()))))
            .collect();
        entries.push(entry(
            5,
            "zq/props/crate_08",
            Some(name_hash32(b"zq/props/other")),
        ));
        let mut resolver = NameResolver::new();
        resolver.insert(entries[0].name64, paths[0]);
        (entries, resolver)
    }

    const BODY: &[u8] =
        b"<Items><Item>lamp_post</Item><Path>platform:/zq/data/levels.xml</Path></Items>";

    fn mined(bodies: Vec<Vec<u8>>) -> Miner {
        let (entries, resolver) = table();
        let mut miner = Miner::new();
        let named = miner.add_table(&entries, &resolver);
        miner.mine_table(&named, bodies, None);
        miner.rules(Vec::new(), DEFAULT_RANGE);
        miner
    }

    fn found<'a>(miner: &'a Miner, path: &str) -> Option<&'a str> {
        miner
            .found64
            .get(&name_hash64(path.as_bytes()))
            .map(String::as_str)
    }

    #[test]
    fn an_unnamed_entry_is_found_by_its_numbered_sibling_and_by_a_string_in_a_body() {
        let miner = mined(vec![BODY.to_vec()]);
        assert_eq!(miner.entry.len(), 5, "the named entry is no target");
        assert_eq!(
            found(&miner, "zq/props/crate_07"),
            Some("zq/props/crate_07")
        );
        assert_eq!(
            found(&miner, "zq/data/levels.xml"),
            Some("zq/data/levels.xml")
        );
        // The directory of the named sibling joined with a name the body states.
        assert_eq!(
            found(&miner, "zq/props/lamp_post"),
            Some("zq/props/lamp_post")
        );
        assert_eq!(found(&miner, "zq/props/never_said"), None);
        assert_eq!(
            found(&miner, "zq/props/crate_08"),
            None,
            "its 32-bit name refuses the sibling"
        );
        assert_eq!(miner.by.get(&(Space::Entry, Source::Install)), Some(&1));
        assert_eq!(miner.by.get(&(Space::Entry, Source::Rules)), Some(&2));
        let lines = miner.lines(0);
        assert!(lines[1].contains("3 of 5 unresolved 64-bit names: 1 from the install's strings, 0 from the executable, 2 by the rules"), "{lines:?}");
    }

    /// The walk over an archive enciphered under a synthetic key: a parent table whose one named
    /// entry has a numbered sibling and whose document states a name and a path, and a nested table
    /// holding the entry the path names.
    #[test]
    fn a_synthetic_archive_s_unnamed_entries_are_found_by_a_sibling_and_a_stated_string() {
        let keys = fixture::keys();
        let child = fixture::archive(&[("zq/data/levels.xml", Body::Zlib(fixture::pso("zq")))]);
        let parent = fixture::archive(&[
            ("zq/props/crate_01", Body::Zlib(fixture::pso("crate"))),
            ("zq/props/crate_07", Body::Zlib(fixture::pso("crate"))),
            ("zq/props/lamp_post", Body::Zlib(fixture::pso("lamp"))),
            (
                "zq/docs/list",
                Body::Zlib(fixture::pso("lamp_post platform:/zq/data/levels.xml")),
            ),
            ("zq/nested", Body::Nested(child)),
        ]);
        let dir = tempfile::tempdir().expect("a scratch folder");
        std::fs::write(dir.path().join("parent.rpf"), &parent).expect("the archive is written");
        let mut resolver = NameResolver::new();
        resolver.insert(name_hash64(b"zq/props/crate_01"), "zq/props/crate_01");

        let mut miner = Miner::new();
        let bounds = Bounds {
            bodies: Cap::new(DEFAULT_BODIES, CapUnit::Count("bodies")),
            cancel: &Cancel::new(),
        };
        let refused = mine_archives(&mut miner, &archives(dir.path()), &keys, &resolver, &bounds);
        miner.rules(Vec::new(), DEFAULT_RANGE);

        assert_eq!(
            (refused, miner.tables),
            (0, 2),
            "the nested table is walked"
        );
        assert_eq!(miner.entry.len(), 5, "every entry but the named one");
        for path in [
            "zq/props/crate_07",
            "zq/props/lamp_post",
            "zq/data/levels.xml",
        ] {
            assert_eq!(found(&miner, path), Some(path));
        }
        assert_eq!(found(&miner, "zq/docs/list"), None);
        assert_eq!(miner.by.get(&(Space::Entry, Source::Install)), Some(&1));
        assert_eq!(miner.by.get(&(Space::Entry, Source::Rules)), Some(&2));
    }

    #[test]
    fn without_the_body_only_the_numbered_sibling_is_found() {
        let miner = mined(Vec::new());
        assert!(found(&miner, "zq/props/crate_07").is_some());
        assert_eq!(found(&miner, "zq/data/levels.xml"), None);
        assert_eq!(found(&miner, "zq/props/lamp_post"), None);
        assert_eq!(miner.found64.len(), 1);
    }

    #[test]
    fn the_body_ceiling_and_a_cancel_stop_the_reads() {
        let keys = fixture::keys();
        let parent = fixture::archive(&[
            ("zq/a", Body::Zlib(fixture::pso("a"))),
            ("zq/b", Body::Zlib(fixture::pso("b"))),
        ]);
        let dir = tempfile::tempdir().expect("a scratch folder");
        std::fs::write(dir.path().join("parent.rpf"), &parent).expect("the archive is written");
        let read = |bodies: Cap, cancel: &Cancel| {
            let mut miner = Miner::new();
            let bounds = Bounds { bodies, cancel };
            mine_archives(
                &mut miner,
                &archives(dir.path()),
                &keys,
                &NameResolver::new(),
                &bounds,
            );
            miner.bodies
        };
        let unlimited = Cap::new(DEFAULT_BODIES, CapUnit::Count("bodies")).lifted();
        assert_eq!(read(unlimited, &Cancel::new()), 2);
        assert_eq!(
            read(Cap::at(1, CapUnit::Count("bodies")), &Cancel::new()),
            1
        );
        let cancelled = Cancel::new();
        cancelled.set();
        assert_eq!(read(unlimited, &cancelled), 0);
    }

    #[test]
    fn a_hash_the_kept_rows_name_is_no_target() {
        let (entries, resolver) = table();
        let mut miner = Miner::new();
        miner.keep_rows(&format!(
            "0x{:016X}\tzq/props/crate_07\n",
            entries[1].name64
        ));
        miner.add_table(&entries, &resolver);
        assert!(!miner.entry.contains_key(&entries[1].name64));
        assert_eq!(miner.entry.len(), 4);
    }

    #[test]
    fn a_structure_and_a_member_are_named_by_the_hash_their_spelling_takes() {
        let mut miner = Miner::new();
        let class = literal_string_hash(b"ZqWidgetMeta");
        let member = string_hash(b"zqIntensity");
        assert_ne!(member, literal_string_hash(b"zqIntensity"));
        miner.class.insert(class);
        miner.member.insert(member);
        miner.consider(b"\0zqwidgetmeta\0ZqWidgetMeta\0zqIntensity\0", None);
        // A structure name is hashed as written, so only the stated spelling names it.
        assert_eq!(
            miner.found_class.get(&class).map(String::as_str),
            Some("ZqWidgetMeta")
        );
        assert_eq!(
            miner.found_member.get(&member).map(String::as_str),
            Some("zqIntensity")
        );
        assert_eq!(miner.tried32, 3);
    }

    #[test]
    fn the_rows_read_back_through_the_resolver_and_the_parser_names() {
        let mut miner = mined(vec![BODY.to_vec()]);
        let member = string_hash(b"zqIntensity");
        miner.found_member.insert(member, "zqIntensity".to_string());
        let dir = tempfile::tempdir().expect("a scratch folder");
        let out = dir.path().join("resolved-names.tsv");
        let kept = "0x0000000000000001\tzq/kept/before";
        assert_eq!(write_rows(&out, kept, &miner).expect("written"), 4);
        let text = std::fs::read_to_string(&out).expect("read back");
        assert!(
            text.starts_with("0x0000000000000001\tzq/kept/before\n"),
            "{text}"
        );
        let resolver = NameResolver::load(dir.path());
        assert_eq!(resolver.lookup(1), Some("zq/kept/before"));
        for path in [
            "zq/props/crate_07",
            "zq/props/lamp_post",
            "zq/data/levels.xml",
        ] {
            assert_eq!(resolver.lookup(name_hash64(path.as_bytes())), Some(path));
        }
        let parser = ParserNames::load_dir(dir.path()).expect("the rows parse");
        assert_eq!(parser.name(member), Some("zqIntensity"));
    }

    #[test]
    fn a_run_gives_itself_and_each_tail_and_says_which_is_last() {
        let mut got = Vec::new();
        candidates(
            b"\x01platform:\\zq\\data\\levels.xml.\0ab\0",
            &mut |piece, last| got.push((piece.to_string(), last)),
        );
        let want = [
            ("platform:/zq/data/levels.xml", false),
            ("zq/data/levels.xml", false),
            ("data/levels.xml", false),
            ("levels.xml", true),
        ];
        assert_eq!(got, want.map(|(p, l)| (p.to_string(), l)));
        assert_eq!(stem_of("zq/data/levels.xml"), "zq/data/levels");
        assert_eq!(stem_of("zq/props.d/crate"), "zq/props.d/crate");
        assert_eq!(stem_of("crate.#dr"), "crate");
    }

    #[test]
    fn the_rules_number_a_sibling_and_carry_its_extension() {
        let mut got = BTreeSet::new();
        let (exts, suffixes) = ([".meta".to_string()], ["_lod".to_string()]);
        rule_candidates("zq/a/crate_01.xml", &exts, &suffixes, 12, &mut |c| {
            got.insert(c.to_string());
        });
        assert!(got.contains("zq/a/crate_11.xml") && got.contains("zq/a/crate_11"));
        assert!(
            !got.contains("zq/a/crate_12.xml"),
            "the range ends below 12"
        );
        assert!(got.contains("zq/a/crate_01.meta"));
        assert!(got.contains("zq/a/crate_01_lod.xml"));
        let names = ["a/b_lod", "a/b", "a/c_lod", "a/c", "a/d_hi", "a/d"].map(String::from);
        assert_eq!(
            known_suffixes(&names),
            ["_lod"],
            "an ending seen once is no suffix"
        );
    }
    #[test]
    fn writing_mined_names_preserves_matches_added_after_the_snapshot() {
        let dir = tempfile::tempdir().expect("scratch");
        let out = dir.path().join("resolved-names.tsv");
        let first = "initial/path";
        let later = "concurrent/path";
        let snapshot = format!("0x{:016x}\t{first}\n", name_hash64(first.as_bytes()));
        let latest = format!(
            "{snapshot}0x{:016x}\t{later}\n",
            name_hash64(later.as_bytes())
        );
        std::fs::write(&out, &latest).expect("concurrent cache update");
        let miner = Miner::new();
        write_rows(&out, &snapshot, &miner).expect("merge mining output");
        assert_eq!(std::fs::read_to_string(out).expect("read"), latest);
    }
}
