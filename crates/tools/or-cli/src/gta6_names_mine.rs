//! `hashes mine`: names for the hashes of a GTA VI install that no table names. Candidates are the
//! strings the install's bodies, the user's executable and the user's lifted script listings
//! state, and what rules make of the names already resolved; a candidate is kept only where its
//! hash equals an unresolved one, and the matches go to the user's file as `0xHASH<TAB>name`, the
//! rows `NameResolver` and `ParserNames` read.

use std::cell::Cell;
use std::collections::hash_map::RandomState;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::File;
use std::hash::BuildHasher;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::container::rpf::{Gta6Entry, Gta6Mount, GtaViKeys, NameResolver, GTA6_EXE_ENV};
use or_job::Cancel;

use super::census_cap::{Cap, CapUnit};
use super::gta6_hashes::{read_row, row_size};
use or_io::gta6::hash::{name_hash32, name_hash64, NameHasher};
use or_io::gta6::names::{vocabulary_file, Vocabulary};
use or_io::gta6::script::native_addresses::LISTING_SUFFIX;
use or_io::meta::binary::layout::{ContainerLayout, MetaLayout};
use or_io::meta::binary::pso::schema::{PsoSchemaItem, ARRAY_INFO_NAME_HASH};
use or_io::meta::binary::pso::string_names::document_strings;
use or_io::meta::binary::ragepar_names::{self, ParserNames};
use or_io::meta::binary::{detect_binary_format, parse_meta_binary_full, BinaryFormat};
use or_io::meta::diagnostics::Diagnostics;
use or_io::meta::schema::SchemaLibrary;
use or_io::meta::Value;
use or_io::{literal_string_hash, string_hash};

#[path = "gta6_names_rules.rs"]
mod rules;
use rules::{NESTED_CAP, SIBLING_CAP};
#[path = "gta6_names_cross.rs"]
mod cross;

const USAGE: &str = "\
usage: openrage-cli hashes mine <install-folder | archive.rpf> --out <file> [--names <dir>] [--keys <snapshot>] [--exe <exe>] [--listings <dir>] [--range <count>] [--max-bodies <n>|unlimited] [--max-total <n>|unlimited] [--archive <n>]...
  Writes the names it finds to <file> as 0xHASH<TAB>name rows, after each archive as it goes;
  rows already in <file> are kept. At the end it also merges them into the names table of the
  configuration folder, which every verb, the explorer and the metadata reader read with no
  setting. No archive is written.
  --listings <dir>   also try the string literals of the lifted script listings in <dir>
                     (else OPENRAGE_LIFTED_LISTINGS)
  --max-bodies <n>   read at most <n> bodies of each archive in each walk (default 50000)
  --max-total <n>    read at most <n> bodies of the whole install in each walk (default unlimited)
  --archive <n>      read the bodies of the n-th archive only, as the report numbers them; give it
                     again for more. Every archive's table is still read for its unnamed rows
  -h, --help         print this text";

const CROSS_USAGE: &str = "usage: openrage-cli hashes cross <install-folder | archive.rpf> --out <file> [--names <dir>] [--keys <snapshot>] [--exe <exe>]
  Names the entries no name resolves from the parts of the names known: every stem of a known
  name with every frequent ending, number and word, hashed on every thread. Only the tables are
  read, no body. Writes the names it finds to <file> as 0xHASH<TAB>name rows after the rows it
  holds, and merges them into the names table of the configuration folder, which every verb and
  the explorer read with no setting. No archive is written.
  -h, --help         print this text";

const MIN_CANDIDATE: usize = 3;
const MAX_CANDIDATE: usize = 160;
/// The longest structure or member name tried.
const MAX_IDENT: usize = 96;
/// A body stated larger than this is not read: it is a movie or a stream, which states no name.
const MAX_BODY: u64 = 64 << 20;
/// The bytes of a body whose strings are taken.
const BODY_PREFIX: usize = 16 << 20;
const MAX_DEPTH: usize = 8;
/// The directory-and-stem joins tried in one table at most; a join hashes only its last component.
const JOIN_CAP: usize = 1 << 31;
/// The last components one table's bodies give that are held for its joins at most.
// ponytail: a table past this joins only the ones met first; the report counts the rest dropped.
const BASENAME_CAP: usize = 1 << 21;
const EXE_WINDOW: u64 = 16 << 20;
/// The bytes one executable window carries into the next, so a string across the edge is whole once.
const EXE_OVERLAP: usize = 256;
const DEFAULT_RANGE: u32 = 100;
/// A body larger than this is not read for its schema in the first walk.
const SCHEMA_BODY: u64 = 4 << 20;
/// The candidate hashes held to skip a repeated try; cleared past this, as a repeat is cheap.
const SEEN_CAP: usize = 1 << 22;
const BODIES_FLAG: &str = "--max-bodies";
/// The bodies each walk reads of one archive by default, so that every archive is sampled.
const DEFAULT_BODIES: u64 = 50_000;
const TOTAL_FLAG: &str = "--max-total";
const LISTINGS_ENV: &str = "OPENRAGE_LIFTED_LISTINGS";
const ARCHIVE_FLAG: &str = "--archive";

/// A table the first walk met: the table above it, the 64-bit name of the row it was opened from
/// there, and its rows' 64-bit names.
struct Table {
    parent: Option<usize>,
    from: Option<u64>,
    rows: Vec<u64>,
}

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
    Listings,
    Rules,
}

/// What one archive gave: the bodies each walk read of it and the rows its string walk made.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ArchiveRead {
    schema_bodies: u64,
    string_bodies: u64,
    rows: usize,
    /// Whether the per-archive ceiling stopped each walk with a body of it still unread.
    schema_short: bool,
    string_short: bool,
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
    /// The rows found and not yet written out, as `NameResolver::add_hash_name_tsv` reads them.
    pending: String,
    /// The 32-bit hashes a row was made for, so a hash both a class and a member is written once.
    written32: HashSet<u32>,
    /// The rows made in all.
    rows: usize,
    by: BTreeMap<(Space, Source), usize>,
    source: Source,
    tried32: u64,
    seen: HashSet<u64>,
    state: RandomState,
    tables: usize,
    /// The bodies the first walk read for their schemas, and those the second read for strings.
    schema_bodies: u64,
    bodies: usize,
    skipped: usize,
    joins_skipped: usize,
    basenames_dropped: usize,
    schemas_unread: usize,
    /// Every table the first walk met, in walk order, and the ones open on its path from the root.
    tree: Vec<Table>,
    path: Vec<usize>,
    /// The names the resolver and the kept rows give the tables' rows, by 64-bit hash.
    named64: HashMap<u64, String>,
    /// The unnamed nested archives whose candidates ran past [`NESTED_CAP`].
    nested_capped: HashSet<u64>,
    /// The tables whose sibling candidates ran past [`SIBLING_CAP`], by their place in `tree`.
    siblings_capped: HashSet<usize>,
    /// Each archive walked, in walk order.
    archives: Vec<ArchiveRead>,
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
                16 => {
                    let Ok(hash) = u64::from_str_radix(digits, 16) else {
                        continue;
                    };
                    self.known64.insert(hash);
                    if let Some(name) = line.split('\t').nth(1).map(str::trim_end) {
                        self.named64.insert(hash, name.to_string());
                    }
                }
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
                    self.named64.insert(entry.name64, name.to_string());
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

    /// Records the table met at `place` of the walk.
    fn place(&mut self, (depth, from): Place, entries: &[Gta6Entry]) {
        self.tables += 1;
        self.path.truncate(depth);
        let parent = self.path.last().copied();
        self.path.push(self.tree.len());
        let rows = entries.iter().map(|e| e.name64).collect();
        self.tree.push(Table { parent, from, rows });
    }

    /// The name a row has, from the resolver, the kept rows or this run.
    fn name64(&self, hash: u64) -> Option<&str> {
        (self.found64.get(&hash).or_else(|| self.named64.get(&hash))).map(String::as_str)
    }

    /// Whether `hash` is an unresolved entry's that this run has not named.
    fn unresolved(&self, hash: u64) -> bool {
        self.entry.contains_key(&hash) && !self.found64.contains_key(&hash)
    }

    /// Joins each directory of `dirs` with each of `basenames`, whole, without its extension and
    /// with each of `exts` in place of it, through one hash state per directory; a join is spelled
    /// out only where its hash is an unresolved entry's.
    fn join(&mut self, dirs: &BTreeSet<String>, basenames: &BTreeSet<String>, exts: &[String]) {
        let was = std::mem::replace(&mut self.source, Source::Rules);
        let mut budget = JOIN_CAP;
        for dir in dirs {
            let state = NameHasher::default().with(dir.as_bytes());
            for base in basenames {
                if budget == 0 {
                    self.joins_skipped += 1;
                    continue;
                }
                budget -= 1;
                let stem = stem_of(base);
                let at_stem = state.with(stem.as_bytes());
                self.try_joined(dir, stem, at_stem.finish());
                if stem.len() < base.len() {
                    let whole = at_stem.with(&base.as_bytes()[stem.len()..]);
                    self.try_joined(dir, base, whole.finish());
                }
                for ext in exts {
                    if self.unresolved(at_stem.with(ext.as_bytes()).finish()) {
                        self.try_entry(&format!("{dir}{stem}{ext}"));
                    }
                }
            }
        }
        self.source = was;
    }

    fn try_joined(&mut self, dir: &str, base: &str, hash: u64) {
        if self.unresolved(hash) {
            self.try_entry(&format!("{dir}{base}"));
        }
    }

    /// Mines the bodies of one table, then joins the directories its names hold with the last
    /// component of each string they state, and of each string member their documents read to
    /// with the extensions its named rows carry too.
    fn mine_table<B: AsRef<[u8]>>(
        &mut self,
        named: &[String],
        bodies: impl IntoIterator<Item = B>,
        keys: Option<&GtaViKeys>,
    ) {
        let mark = self.order.len();
        let (mut basenames, mut values) = (BTreeSet::new(), BTreeSet::new());
        // Each body is dropped before the next is read.
        for body in bodies {
            self.bodies += 1;
            self.body(body.as_ref(), keys, &mut basenames, &mut values);
        }
        let found = self.order[mark..].to_vec();
        let names: Vec<String> = (named.iter().chain(&found))
            .map(|name| name.to_ascii_lowercase())
            .collect();
        let dirs: BTreeSet<String> = (names.iter())
            .filter_map(|name| name.rfind('/').map(|at| name[..=at].to_string()))
            .collect();
        let exts: BTreeSet<&str> = (names.iter())
            .map(|name| &name[stem_of(name).len()..])
            .filter(|ext| !ext.is_empty())
            .collect();
        let exts: Vec<String> = exts.into_iter().map(str::to_owned).collect();
        self.join(&dirs, &basenames, &[]);
        self.join(&dirs, &values, &exts);
    }

    /// Tries the strings of one body; the last component of each goes into `basenames`, and of
    /// each string member the metadata reader reads of it into `values` as well.
    fn body(
        &mut self,
        bytes: &[u8],
        keys: Option<&GtaViKeys>,
        basenames: &mut BTreeSet<String>,
        values: &mut BTreeSet<String>,
    ) {
        let format = detect_binary_format(bytes);
        if let Some(format) = format {
            // The first walk took the schema of every body up to this size.
            let schema = format == BinaryFormat::Pso && bytes.len() as u64 > SCHEMA_BODY;
            let library = SchemaLibrary::default();
            match parse_meta_binary_full(&library, bytes, &mut Diagnostics::lenient()) {
                Ok((instance, layout)) => {
                    if schema {
                        self.schema_of(&layout);
                    }
                    let mut texts = Vec::new();
                    for (_, value) in &instance.fields {
                        value_strings(value, &mut |text| texts.push(text.to_string()));
                    }
                    for text in texts {
                        candidates(text.as_bytes(), &mut |piece, last| {
                            if last && values.len() < BASENAME_CAP {
                                values.insert(piece.to_ascii_lowercase());
                            }
                        });
                        self.consider(text.as_bytes(), Some(basenames));
                    }
                }
                Err(_) => self.schemas_unread += usize::from(schema),
            }
        }
        if format == Some(BinaryFormat::Pso) {
            for text in document_strings(bytes, keys).unwrap_or_default() {
                self.consider(text.as_bytes(), Some(basenames));
            }
        }
        self.consider(&bytes[..bytes.len().min(BODY_PREFIX)], Some(basenames));
    }

    /// Takes the structure, enum, member and value hashes of a PSO body's schema no table names.
    fn schema_targets(&mut self, bytes: &[u8]) {
        let library = SchemaLibrary::default();
        match parse_meta_binary_full(&library, bytes, &mut Diagnostics::lenient()) {
            Ok((_, layout)) => self.schema_of(&layout),
            Err(_) => self.schemas_unread += 1,
        }
    }

    /// Takes the hashes of a PSO `layout`'s schema no table names.
    fn schema_of(&mut self, layout: &MetaLayout) {
        let ContainerLayout::Pso(pso) = &layout.container else {
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
                let piece = piece.to_ascii_lowercase();
                if held.len() < BASENAME_CAP {
                    held.insert(piece);
                } else if !held.contains(&piece) {
                    self.basenames_dropped += 1;
                }
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

    /// The string literals of the lifted script listings in `folder`, which name the streams,
    /// movies and documents the scripts load; how many listings were read.
    fn mine_listings(&mut self, folder: &Path) -> std::io::Result<usize> {
        let is_listing = |path: &Path| {
            let name = path.file_name().and_then(|n| n.to_str());
            path.is_file() && name.is_some_and(|n| n.ends_with(LISTING_SUFFIX))
        };
        let listed = std::fs::read_dir(folder)?.flatten().map(|e| e.path());
        let mut paths: Vec<PathBuf> = listed.filter(|p| is_listing(p)).collect();
        paths.sort();
        self.source = Source::Listings;
        for path in &paths {
            let text = std::fs::read(path)?;
            for literal in string_literals(&text) {
                self.consider(literal, None);
            }
        }
        self.source = Source::Install;
        Ok(paths.len())
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
        self.record(Space::Entry, self.source, word, form);
    }

    /// A structure name is hashed as written; a member name as written or case-folded, both occur.
    fn try_ident(&mut self, name: &str, source: Source) {
        self.tried32 += 1;
        let literal = literal_string_hash(name.as_bytes());
        if self.class.contains(&literal) && !self.found_class.contains_key(&literal) {
            self.found_class.insert(literal, name.to_string());
            self.record(Space::Class, source, u64::from(literal), name);
        }
        for hash in [literal, string_hash(name.as_bytes())] {
            if self.member.contains(&hash) && !self.found_member.contains_key(&hash) {
                self.found_member.insert(hash, name.to_string());
                self.record(Space::Member, source, u64::from(hash), name);
            }
        }
    }

    /// Counts `name`, found for `hash` in `space`, and makes its row.
    fn record(&mut self, space: Space, source: Source, hash: u64, name: &str) {
        *self.by.entry((space, source)).or_default() += 1;
        self.order.push(name.to_string());
        let row = match space {
            Space::Entry => format!("0x{hash:016X}\t{name}\n"),
            _ if !self.written32.insert(hash as u32) => return,
            _ => format!("0x{hash:08X}\t{name}\n"),
        };
        self.pending.push_str(&row);
        self.rows += 1;
    }

    /// The rows made since the last call.
    fn take_rows(&mut self) -> String {
        std::mem::take(&mut self.pending)
    }

    /// The report: what was read, and per space how many unresolved hashes were named and how.
    fn lines(&self, refused: usize) -> Vec<String> {
        let by = |space, source| self.by.get(&(space, source)).copied().unwrap_or(0);
        let sources = |space| {
            format!(
                "{} from the install's strings, {} from the executable, {} from the script \
                 listings, {} by the rules",
                by(space, Source::Install),
                by(space, Source::Executable),
                by(space, Source::Listings),
                by(space, Source::Rules)
            )
        };
        // The 32-bit spaces carry a chance rate: each identifier tried names a hash by chance once in 2^32.
        let chance = |flavours: f64, unresolved: usize| {
            flavours * self.tried32 as f64 * unresolved as f64 / 4_294_967_296.0
        };
        let mut lines = vec![
            format!(
                "  tables        {} read, {refused} refused; {} bodies read for their schemas, {} \
                 for their strings, {} not read",
                self.tables, self.schema_bodies, self.bodies, self.skipped
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
        if self.basenames_dropped > 0 {
            lines.push(format!(
                "  basenames     {} not held for joins past {BASENAME_CAP} in a table",
                self.basenames_dropped
            ));
        }
        if !self.siblings_capped.is_empty() {
            lines.push(format!(
                "  siblings      {} tables not tried past {SIBLING_CAP} candidates",
                self.siblings_capped.len()
            ));
        }
        if !self.nested_capped.is_empty() {
            lines.push(format!(
                "  nested        {} unnamed archives not tried past {NESTED_CAP} candidates",
                self.nested_capped.len()
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

    /// A line per archive of `paths`, numbered in walk order and named from `install`, with what
    /// each walk read of it and the names found per 1000 bodies, then a line for each walk that
    /// one of `bounds`' ceilings stopped short.
    fn walk_lines(&self, paths: &[PathBuf], install: &Path, bounds: &Bounds<'_>) -> Vec<String> {
        let mut lines = Vec::new();
        for (at, (path, read)) in paths.iter().zip(&self.archives).enumerate() {
            let name = path.strip_prefix(install).unwrap_or(path);
            let head = format!("  archive {:<6}{}", at + 1, name.display());
            if !bounds.reads(at) {
                lines.push(format!(
                    "{head}: table read, no body (not given to {ARCHIVE_FLAG})"
                ));
                continue;
            }
            let rate = match read.string_bodies {
                0 => String::new(),
                n => format!(
                    ", {:.1} per 1000 bodies",
                    read.rows as f64 * 1000.0 / n as f64
                ),
            };
            let short = read.schema_short || read.string_short;
            lines.push(format!(
                "{head}: {} bodies read for their schemas, {} for their strings, {} names \
                 found{rate}{}",
                read.schema_bodies,
                read.string_bodies,
                read.rows,
                if short {
                    "; stopped at the ceiling"
                } else {
                    ""
                }
            ));
        }
        let schema: Vec<(u64, bool)> = (self.archives.iter())
            .map(|a| (a.schema_bodies, a.schema_short))
            .collect();
        let string: Vec<(u64, bool)> = (self.archives.iter())
            .map(|a| (a.string_bodies, a.string_short))
            .collect();
        let walks = [
            ("schema walk", schema, self.schema_bodies),
            ("string walk", string, self.bodies as u64),
        ];
        for (walk, each, all) in walks {
            let stopped = each.iter().filter(|(_, short)| *short).count();
            let most = (each.iter().filter(|(_, short)| *short))
                .map(|(n, _)| *n)
                .max();
            let note = most.and_then(|n| bounds.bodies.note(n, BODIES_FLAG));
            if let Some(note) = note {
                let of = each.len();
                lines.push(format!("  {walk}   {stopped} of {of} archives: {note}"));
            }
            if let Some(note) = bounds.total.note(all, TOTAL_FLAG) {
                lines.push(format!("  {walk}   {note}"));
            }
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

/// Each text `value` holds: its strings, the names its hashes were read by, and those of every
/// value inside it.
fn value_strings(value: &Value, each: &mut impl FnMut(&str)) {
    match value {
        Value::String(text) => each(text),
        Value::Hash(hashed) if !hashed.raw.is_empty() => each(&hashed.raw),
        Value::Struct(inner) | Value::Pointer(Some(inner)) => {
            (inner.fields.iter()).for_each(|(_, value)| value_strings(value, each))
        }
        Value::Array(items) => items.iter().for_each(|value| value_strings(value, each)),
        Value::Map(pairs) => pairs.iter().for_each(|(key, value)| {
            value_strings(key, each);
            value_strings(value, each);
        }),
        _ => {}
    }
}

/// What each `"…"` of `text` holds; a `\` keeps the byte after it inside, and a literal ends at
/// the end of its line, so one quote out of place spoils one line.
fn string_literals(text: &[u8]) -> Vec<&[u8]> {
    let mut literals = Vec::new();
    let mut at = 0;
    while let Some(open) = text[at..].iter().position(|&b| b == b'"') {
        let start = at + open + 1;
        let mut end = start;
        while end < text.len() && !matches!(text[end], b'"' | b'\n') {
            // ⚠ A `\` at a line's end keeps nothing: the line end still closes the literal.
            let escapes = text[end] == b'\\' && text.get(end + 1) != Some(&b'\n');
            end += if escapes { 2 } else { 1 };
        }
        let end = end.min(text.len());
        literals.push(&text[start..end]);
        at = (end + 1).min(text.len());
    }
    literals
}

/// Where a table sits in the walk: its depth, and the 64-bit name of the row it was opened from.
type Place = (usize, Option<u64>);

type Visit<'a> = dyn FnMut(&mut File, &Gta6Mount, &[usize], Place) + 'a;

/// Visits the table of `mount` with the entries that are not archives, then each nested table.
fn walk(
    file: &mut File,
    mount: &Gta6Mount,
    keys: &GtaViKeys,
    (depth, from): Place,
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
    visit(file, mount, &leaves, (depth, from));
    for at in nested {
        let entry = &mount.archive.entries[at];
        match mount.open_nested(&mut *file, entry, keys) {
            Ok(child) => {
                let place = (depth + 1, Some(entry.name64));
                walk(file, &child, keys, place, refused, visit);
            }
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
        Ok((mut file, root)) => walk(&mut file, &root, keys, (0, None), refused, visit),
        Err(e) => {
            eprintln!("  {}: {e}", path.display());
            *refused += 1;
        }
    }
}

/// How many bodies read are held at once, and the most that ever were.
#[derive(Default)]
struct Holding {
    now: Cell<usize>,
    peak: Cell<usize>,
}

/// A body read, counted by its [`Holding`] until it is dropped.
struct Held<'a> {
    bytes: Vec<u8>,
    holding: &'a Holding,
}

impl<'a> Held<'a> {
    fn new(bytes: Vec<u8>, holding: &'a Holding) -> Self {
        let now = holding.now.get() + 1;
        holding.now.set(now);
        holding.peak.set(holding.peak.get().max(now));
        Self { bytes, holding }
    }
}

impl AsRef<[u8]> for Held<'_> {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        self.holding.now.set(self.holding.now.get() - 1);
    }
}

/// The size `entry`'s row states, the larger of its stored and inflated sizes where it states both.
fn stated_size(mount: &Gta6Mount, entry: &Gta6Entry) -> Option<u64> {
    row_size(mount, entry).max(entry.stored)
}

/// The inflated body of `entry`, read by its table row and deciphered where it is enciphered;
/// `None` where it cannot be read.
fn read_body<'a>(
    file: &mut File,
    mount: &Gta6Mount,
    entry: &Gta6Entry,
    keys: &GtaViKeys,
    holding: &'a Holding,
) -> Option<Held<'a>> {
    if stated_size(mount, entry).is_some_and(|len| len > MAX_BODY)
        || (entry.cipher.is_some() && !keys.has_content_keys())
    {
        return None;
    }
    let bytes = read_row(file, mount, entry, keys, MAX_BODY).ok()?;
    Some(Held::new(bytes, holding))
}

/// `install` where it is an archive, else every `*.rpf` in it and in each folder of its `pack`.
fn archives(install: &Path) -> Vec<PathBuf> {
    match install.is_file() {
        true => vec![install.to_path_buf()],
        false => or_title::install::gta6_files::mounted(install),
    }
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

/// Joins `rows` to what `out` holds now, or to `kept` where it is gone, under the lock its other writers take.
fn write_rows(out: &Path, kept: &str, rows: &str) -> std::io::Result<()> {
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
    // A hash already named keeps its name, in whichever case its digits are written.
    let hash_of = |row: &str| row.split('\t').next().unwrap_or(row).to_ascii_lowercase();
    let mut present: HashSet<String> = text.lines().map(hash_of).collect();
    for row in rows.lines() {
        if present.insert(hash_of(row)) {
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
    result
}

/// The class and member rows `miner` found, in eight lower-case digits as the names cache writes them.
fn rows32(miner: &Miner) -> String {
    let mut rows = BTreeMap::new();
    for (&hash, name) in miner.found_class.iter().chain(&miner.found_member) {
        rows.entry(hash).or_insert(name.as_str());
    }
    (rows.iter())
        .map(|(hash, name)| format!("0x{hash:08x}\t{name}\n"))
        .collect()
}

/// Merges the entry names `miner` found into the table under `config` that every verb's names
/// cache reads, then its class and member names, which the metadata reader reads from there.
fn merge_into_config(miner: &Miner, config: &Path) {
    let table = ragepar_names::mined_table(config);
    let folder = table.parent().unwrap_or(config);
    let found = (miner.found64.iter()).map(|(&hash, name)| (hash, name.as_str()));
    // The names cache rewrites the table with the 32-bit rows it holds, so they go in after it.
    if let Err(error) = NameResolver::cache_names(folder, found) {
        tracing::warn!(%error, "could not update the local GTA VI names cache");
    }
    let rows = rows32(miner);
    if rows.is_empty() {
        return;
    }
    if let Err(error) = write_rows(&table, "", &rows) {
        tracing::warn!(%error, "could not add the class and member names to the local GTA VI names");
    }
}

/// Has the metadata reader read the class and member names `hashes mine` merged into the configuration folder.
pub fn read_mined_parser_names() {
    if let Some(config) = or_title::config::config_dir() {
        ragepar_names::use_config_dir(&config);
    }
}

/// The file `out`, written anew with the rows `kept` held before, which the rows found follow.
fn open_rows(out: &Path, kept: &str) -> std::io::Result<BufWriter<File>> {
    if let Some(folder) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(folder)?;
    }
    let mut file = BufWriter::new(File::create(out)?);
    file.write_all(kept.as_bytes())?;
    if !kept.is_empty() && !kept.ends_with('\n') {
        file.write_all(b"\n")?;
    }
    file.flush()?;
    Ok(file)
}

/// Writes the rows `miner` made since the last write to `out`, and flushes it.
fn flush_rows(miner: &mut Miner, out: &mut dyn Write) -> std::io::Result<()> {
    out.write_all(miner.take_rows().as_bytes())?;
    out.flush()
}

struct Options {
    install: PathBuf,
    out: PathBuf,
    names: Option<PathBuf>,
    keys: Option<PathBuf>,
    exe: Option<PathBuf>,
    listings: Option<PathBuf>,
    range: u32,
    bodies: Cap,
    total: Cap,
    /// The archives whose bodies are read, by their place in walk order from 1; every one where empty.
    chosen: BTreeSet<usize>,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let (mut install, mut out, mut names, mut keys, mut exe) = (None, None, None, None, None);
    let mut listings = None;
    let mut range = DEFAULT_RANGE;
    let mut bodies = Cap::new(DEFAULT_BODIES, CapUnit::Count("bodies"));
    let mut total = Cap::new(0, CapUnit::Count("bodies")).lifted();
    let mut chosen = BTreeSet::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if let Some(v) = arg.strip_prefix("--max-bodies=") {
            bodies = bodies.parse(BODIES_FLAG, v)?;
            continue;
        }
        if let Some(v) = arg.strip_prefix("--max-total=") {
            total = total.parse(TOTAL_FLAG, v)?;
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
            "--listings" => listings = Some(value("a folder")?),
            BODIES_FLAG => {
                let v = value("a count")?;
                bodies = bodies.parse(BODIES_FLAG, &v.to_string_lossy())?;
            }
            ARCHIVE_FLAG => {
                let place = value("a place in walk order")?;
                let place = (place.to_str().and_then(|p| p.parse().ok()))
                    .filter(|n: &usize| *n > 0)
                    .ok_or("--archive needs a place in walk order, from 1")?;
                chosen.insert(place);
            }
            TOTAL_FLAG => {
                let v = value("a count")?;
                total = total.parse(TOTAL_FLAG, &v.to_string_lossy())?;
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
        listings,
        range,
        bodies,
        total,
        chosen,
    })
}

/// How far the walk reads: the bodies each walk may read of one archive and of the install, the
/// flag Ctrl-C sets, and the count of bodies held at once.
struct Bounds<'a> {
    bodies: Cap,
    total: Cap,
    cancel: &'a Cancel,
    holding: Holding,
    /// The archives whose bodies are read, by their place from 1; every one where empty.
    chosen: BTreeSet<usize>,
}

impl<'a> Bounds<'a> {
    fn new(bodies: Cap, cancel: &'a Cancel) -> Self {
        Self {
            bodies,
            total: Cap::new(0, CapUnit::Count("bodies")).lifted(),
            cancel,
            holding: Holding::default(),
            chosen: BTreeSet::new(),
        }
    }

    /// Whether the bodies of the archive at `at` of the walk, from 0, are read.
    fn reads(&self, at: usize) -> bool {
        self.chosen.is_empty() || self.chosen.contains(&(at + 1))
    }

    /// Whether a walk that has read `read` bodies, `since` of them before this archive, stops.
    fn stops(&self, read: u64, since: u64) -> bool {
        self.cancel.is_set() || self.bodies.reached(read - since) || self.total.reached(read)
    }
}

/// Walks every table of `archives` for its unnamed entries and its schemas' hashes, then again for
/// its bodies' strings, so each string is tried against every unresolved hash as it is read; each
/// walk reads one body at a time and at most as many as `bounds` allows, and the rows found are
/// written to `out` after each archive. How many tables were refused.
fn mine_archives(
    miner: &mut Miner,
    archives: &[PathBuf],
    keys: &GtaViKeys,
    resolver: &NameResolver,
    bounds: &Bounds<'_>,
    out: &mut dyn Write,
) -> std::io::Result<usize> {
    let (mut refused, total) = (0, archives.len());
    let first = miner.archives.len();
    for (i, path) in archives.iter().enumerate() {
        eprintln!("  schemas [{}/{total}] {}", i + 1, path.display());
        let since = miner.schema_bodies;
        let mut short = false;
        // An archive not chosen gives its tables' unnamed rows as targets, and no body.
        let reads = bounds.reads(i);
        walk_archive(path, keys, &mut refused, &mut |file, mount, leaves, at| {
            let entries = &mount.archive.entries;
            miner.place(at, entries);
            miner.add_table(entries, resolver);
            for &at in leaves.iter().filter(|_| reads) {
                let entry = &entries[at];
                if bounds.stops(miner.schema_bodies, since) {
                    short |= bounds.bodies.reached(miner.schema_bodies - since);
                    break;
                }
                if stated_size(mount, entry).is_some_and(|n| n > SCHEMA_BODY) {
                    continue;
                }
                let Some(body) = read_body(&mut *file, mount, entry, keys, &bounds.holding) else {
                    continue;
                };
                miner.schema_bodies += 1;
                if detect_binary_format(&body.bytes) == Some(BinaryFormat::Pso) {
                    miner.schema_targets(&body.bytes);
                }
            }
        });
        let read = miner.schema_bodies - since;
        miner.archives.push(ArchiveRead {
            schema_bodies: read,
            schema_short: short,
            ..ArchiveRead::default()
        });
        eprintln!("  schemas [{}/{total}] {read} bodies read", i + 1);
    }
    for (i, path) in archives.iter().enumerate() {
        if !bounds.reads(i) {
            continue;
        }
        eprintln!("  strings [{}/{total}] {}", i + 1, path.display());
        let (since, rows) = (miner.bodies as u64, miner.rows);
        let mut short = false;
        walk_archive(path, keys, &mut 0, &mut |file, mount, leaves, _| {
            let entries = &mount.archive.entries;
            let named: Vec<String> = (entries.iter())
                .filter_map(|e| resolver.lookup(e.name64).map(str::to_owned))
                .collect();
            let (mut skipped, mut taken) = (0, miner.bodies as u64);
            let holding = &bounds.holding;
            let bodies = leaves.iter().filter_map(|&at| {
                if bounds.stops(taken, since) {
                    short |= bounds.bodies.reached(taken - since);
                    return None;
                }
                let body = read_body(&mut *file, mount, &entries[at], keys, holding);
                skipped += usize::from(body.is_none());
                taken += u64::from(body.is_some());
                body
            });
            miner.mine_table(&named, bodies, Some(keys));
            miner.skipped += skipped;
        });
        flush_rows(miner, out)?;
        let read = &mut miner.archives[first + i];
        read.string_bodies = miner.bodies as u64 - since;
        read.rows = miner.rows - rows;
        read.string_short = short;
        eprintln!(
            "  strings [{}/{total}] {} bodies read, {} names found",
            i + 1,
            read.string_bodies,
            read.rows
        );
    }
    Ok(refused)
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
    if let Some(past) = options.chosen.iter().find(|&&n| n > archives.len()) {
        return Err(format!(
            "{ARCHIVE_FLAG} {past}: {} holds {} archives",
            options.install.display(),
            archives.len()
        ));
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
    let mut bounds = Bounds::new(options.bodies, &cancel);
    bounds.total = options.total;
    bounds.chosen = options.chosen.clone();
    let unwritten = |e: std::io::Error| format!("{}: {e}", options.out.display());
    // The rows are written beside the output as they are found, so a run that stops keeps them
    // there and leaves the output whole; they are joined to it under its lock at the end.
    let mining = options.out.with_extension("mining");
    let mut out = open_rows(&mining, "").map_err(unwritten)?;
    let refused = mine_archives(&mut miner, &archives, &keys, &resolver, &bounds, &mut out)
        .map_err(unwritten)?;
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
    let listings =
        (options.listings.clone()).or_else(|| std::env::var_os(LISTINGS_ENV).map(PathBuf::from));
    let mut listings_read = None;
    if let Some(folder) = listings.as_ref().filter(|_| !cancel.is_set()) {
        let read =
            (miner.mine_listings(folder)).map_err(|e| format!("{}: {e}", folder.display()))?;
        listings_read = Some(read);
    }
    flush_rows(&mut miner, &mut out).map_err(unwritten)?;
    miner.rules(seeds32, options.range);
    flush_rows(&mut miner, &mut out).map_err(unwritten)?;
    drop(out);
    let found = miner.rows;
    let rows = std::fs::read_to_string(&mining).map_err(unwritten)?;
    write_rows(&options.out, &kept, &rows).map_err(unwritten)?;
    let _ = std::fs::remove_file(&mining);
    let config = or_title::config::config_dir();
    if let Some(config) = &config {
        merge_into_config(&miner, config);
    }
    let mut lines = miner.lines(refused);
    if exe.is_none() {
        lines.push(format!(
            "  executable    none read: pass --exe or set {GTA6_EXE_ENV}"
        ));
    }
    lines.push(match (&listings, listings_read) {
        (Some(folder), Some(read)) => {
            format!("  listings      {read} read from {}", folder.display())
        }
        _ => format!("  listings      none read: pass --listings or set {LISTINGS_ENV}"),
    });
    lines.extend(miner.walk_lines(&archives, &options.install, &bounds));
    lines.push(format!(
        "  written       {}: {found} rows found, after the rows it held; no archive is written",
        options.out.display()
    ));
    if let Some(config) = &config {
        lines.push(format!(
            "  merged        {}: what every verb and the metadata reader read with no setting",
            ragepar_names::mined_table(config).display()
        ));
    }
    if cancel.is_set() {
        lines.push(
            "  CANCELLED     the walk stopped short; the rows found before it are written"
                .to_string(),
        );
    }
    Ok(lines)
}

/// `hashes cross`: reads every table of the install for its unnamed entries and names them by the cross products.
fn cross_names(options: &Options) -> Result<Vec<String>, String> {
    let keys = crate::gta6_keys::load(options.keys.as_deref(), options.exe.as_deref())?;
    let resolver = load_resolver(options.names.as_deref());
    let unwritten = |e: std::io::Error| format!("{}: {e}", options.out.display());
    let kept = match std::fs::read_to_string(&options.out) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(unwritten(e)),
    };
    let archives = archives(&options.install);
    if archives.is_empty() {
        return Err(format!("no .rpf in {}", options.install.display()));
    }
    let mut miner = Miner::new();
    miner.keep_rows(&kept);
    let mut refused = 0;
    for path in &archives {
        walk_archive(path, &keys, &mut refused, &mut |_, mount, _, at| {
            let entries = &mount.archive.entries;
            miner.place(at, entries);
            miner.add_table(entries, &resolver);
        });
    }
    let config = or_title::config::config_dir();
    let vocabulary = config
        .as_ref()
        .map(|c| Vocabulary::load(&vocabulary_file(c)));
    let extra: Vec<String> = (vocabulary.iter())
        .flat_map(|v| v.names().map(str::to_string))
        .collect();
    let mut lines = vec![format!(
        "hashes cross: {} tables read, {refused} refused; {} entries unresolved",
        miner.tables,
        miner.entry.len()
    )];
    lines.extend(miner.cross(extra));
    let rows = miner.take_rows();
    write_rows(&options.out, &kept, &rows).map_err(unwritten)?;
    lines.push(format!(
        "  written       {}: {} of {} unresolved names found, after the rows it held; no archive is written",
        options.out.display(),
        miner.found64.len(),
        miner.entry.len()
    ));
    if let Some(config) = &config {
        merge_into_config(&miner, config);
        lines.push(format!(
            "  merged        {}: what every verb and the metadata reader read with no setting",
            ragepar_names::mined_table(config).display()
        ));
    }
    Ok(lines)
}

pub(crate) fn run(args: &[String]) -> ExitCode {
    let split = args.split_first().map(|(verb, rest)| (verb.as_str(), rest));
    let Some((verb @ ("mine" | "cross"), rest)) = split else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let usage = if verb == "cross" { CROSS_USAGE } else { USAGE };
    if rest.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{usage}");
        return ExitCode::SUCCESS;
    }
    let options = match parse(rest) {
        Ok(options) => options,
        Err(e) => {
            eprintln!("Error: {e}");
            eprintln!("{usage}");
            return ExitCode::from(2);
        }
    };
    let ran = if verb == "cross" {
        cross_names(&options)
    } else {
        mine(&options)
    };
    match ran {
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
    use super::rules::{
        dir_stem, known_suffixes, rule_candidates, shared_stems, short_endings, short_heads,
        sibling_parts,
    };
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
        assert!(lines[1].contains("3 of 5 unresolved 64-bit names: 1 from the install's strings, 0 from the executable, 0 from the script listings, 2 by the rules"), "{lines:?}");
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
        let cancel = Cancel::new();
        let bounds = Bounds::new(Cap::new(DEFAULT_BODIES, CapUnit::Count("bodies")), &cancel);
        let mut written = Vec::new();
        let refused = mine_archives(
            &mut miner,
            &archives(dir.path()),
            &keys,
            &resolver,
            &bounds,
            &mut written,
        )
        .expect("the rows are written");
        // A name the walk found is written before the rules run.
        let row = format!(
            "0x{:016X}\tzq/data/levels.xml\n",
            name_hash64(b"zq/data/levels.xml")
        );
        let written = String::from_utf8(written).expect("the rows are text");
        assert!(written.contains(&row), "{written}");
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

    /// Over a table of many bodies, each walk holds one body at a time, reads every body when
    /// unbounded, stops at the ceiling, and reads none once cancelled.
    #[test]
    fn each_walk_holds_one_body_at_a_time_and_stops_at_the_ceiling() {
        const BODIES: usize = 12;
        let keys = fixture::keys();
        let paths: Vec<String> = (0..BODIES)
            .map(|i| format!("zq/many/body_{i:02}"))
            .collect();
        let entries: Vec<(&str, Body)> = (paths.iter())
            .map(|path| (path.as_str(), Body::Zlib(fixture::pso(path))))
            .collect();
        let dir = tempfile::tempdir().expect("a scratch folder");
        let parent = fixture::archive(&entries);
        std::fs::write(dir.path().join("parent.rpf"), &parent).expect("the archive is written");
        let read = |bodies: Cap, cancel: &Cancel| {
            let mut miner = Miner::new();
            let bounds = Bounds::new(bodies, cancel);
            mine_archives(
                &mut miner,
                &archives(dir.path()),
                &keys,
                &NameResolver::new(),
                &bounds,
                &mut std::io::sink(),
            )
            .expect("the rows are written");
            (miner.schema_bodies, miner.bodies, bounds.holding.peak.get())
        };
        let unlimited = Cap::new(DEFAULT_BODIES, CapUnit::Count("bodies")).lifted();
        assert_eq!(read(unlimited, &Cancel::new()), (BODIES as u64, BODIES, 1));
        let five = Cap::at(5, CapUnit::Count("bodies"));
        assert_eq!(
            read(five, &Cancel::new()),
            (5, 5, 1),
            "the ceiling bounds both walks"
        );
        let cancelled = Cancel::new();
        cancelled.set();
        assert_eq!(read(unlimited, &cancelled), (0, 0, 0));
    }

    /// The smallest archive of the build mined under a ceiling of four bodies: each walk reads at
    /// most four, one at a time. Install-gated.
    #[test]
    fn the_miner_runs_on_the_build_within_its_ceiling() {
        const CEILING: u64 = 4;
        let present = |var: &str| {
            std::env::var_os(var)
                .map(PathBuf::from)
                .filter(|p| p.exists())
        };
        let snapshot = present(or_io::container::rpf::GTA6_SNAPSHOT_ENV);
        let (Some(snapshot), Some(build)) = (snapshot, present("OPENRAGE_GTA6_BUILD")) else {
            or_test_support::skip_install(
                "hashes mine over a GTA VI build",
                format_args!(
                    "set {} and OPENRAGE_GTA6_BUILD",
                    or_io::container::rpf::GTA6_SNAPSHOT_ENV
                ),
            );
            return;
        };
        let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
        let size = |p: &PathBuf| std::fs::metadata(p).map_or(u64::MAX, |m| m.len());
        let mut smallest = archives(&build);
        smallest.sort_by_key(size);
        smallest.truncate(3);
        assert_eq!(smallest.len(), 3, "the build holds three archives");

        let mut miner = Miner::new();
        let cancel = Cancel::new();
        let bounds = Bounds::new(Cap::at(CEILING, CapUnit::Count("bodies")), &cancel);
        let refused = mine_archives(
            &mut miner,
            &smallest,
            &keys,
            &NameResolver::new(),
            &bounds,
            &mut std::io::sink(),
        )
        .expect("the rows are written");
        assert!(miner.tables > 0, "{refused} tables refused");
        assert_eq!(miner.archives.len(), 3, "a record per archive");
        for read in &miner.archives {
            assert!(read.schema_bodies <= CEILING && read.string_bodies <= CEILING);
        }
        let each: u64 = miner.archives.iter().map(|a| a.string_bodies).sum();
        assert_eq!(
            each, miner.bodies as u64,
            "the archives' bodies add up to the walk's"
        );
        assert!(bounds.holding.peak.get() <= 1, "one body held at a time");
    }

    #[test]
    fn a_table_holds_no_more_basenames_than_its_cap_and_counts_those_dropped() {
        let mut held: BTreeSet<String> = (0..BASENAME_CAP).map(|i| format!("zq{i}")).collect();
        let mut miner = Miner::new();
        miner.consider(b"\0zq/a/fresh_one\0zq/b/fresh_two\0zq7\0", Some(&mut held));
        assert_eq!(held.len(), BASENAME_CAP);
        assert!(!held.contains("fresh_one"));
        assert_eq!(
            miner.basenames_dropped, 2,
            "one already held is not dropped"
        );
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
        assert_eq!(
            miner.name64(entries[1].name64),
            Some("zq/props/crate_07"),
            "a kept row's name seeds the nested rule"
        );
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
        miner.member.insert(member);
        miner.consider(b"\0zqIntensity\0", None);
        let dir = tempfile::tempdir().expect("a scratch folder");
        let out = dir.path().join("resolved-names.tsv");
        let kept = "0x0000000000000001\tzq/kept/before";
        let mut file = open_rows(&out, kept).expect("the file opens");
        flush_rows(&mut miner, &mut file).expect("written");
        drop(file);
        assert_eq!(miner.rows, 4, "three entry names and a member");
        let text = std::fs::read_to_string(&out).expect("read back");
        assert_eq!(text.lines().count(), 5, "{text}");
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
    fn the_rules_number_the_last_run_of_digits_and_keep_what_follows_it() {
        let mut got = BTreeSet::new();
        rule_candidates("zq7/a/zq_w10_036_hd", &[], &[], 100, &mut |c| {
            got.insert(c.to_string());
        });
        assert!(got.contains("zq7/a/zq_w10_041_hd"));
        assert!(!got.contains("zq7/a/zq_w11_036_hd"), "only the last run");
        assert!(
            !got.contains("zq7/a/zq_w10_100_hd"),
            "the range ends below 100"
        );
        let mut wide = 0;
        rule_candidates("zq/a/zq_12345_hd", &[], &[], 100, &mut |c| {
            wide += usize::from(c != "zq/a/zq_12345_hd");
        });
        assert_eq!(wide, 0, "a run wider than four digits is not numbered");
    }

    #[test]
    fn the_stems_are_the_prefixes_two_rows_share_or_every_prefix_of_one_row() {
        let rows = ["Zq_Alpha_01", "zq/b/zq_alpha_02.ydr", "zq_beta"];
        assert_eq!(shared_stems(rows.into_iter()), ["zq", "zq_alpha"]);
        assert_eq!(shared_stems(["zq/a/zq_x"].into_iter()), ["zq", "zq_x"]);
        assert_eq!(dir_stem("zq/maps/"), Some("maps"));
        assert_eq!(dir_stem(""), None);
    }

    /// A table of four nested archives: two named, whose rows share their stems; one unnamed whose
    /// rows are named and share a stem; one unnamed whose rows are unnamed too.
    #[test]
    fn an_unnamed_nested_archive_is_named_by_its_siblings_directory_and_its_rows_stem() {
        let keys = fixture::keys();
        let nested = |rows: &[&str]| {
            let entries: Vec<(&str, Body)> = (rows.iter())
                .map(|row| (*row, Body::Zlib(fixture::pso("zq"))))
                .collect();
            Body::Nested(fixture::archive(&entries))
        };
        let parent = fixture::archive(&[
            (
                "zq/maps/zq_alpha_metadata",
                nested(&["zq_alpha_01", "zq_alpha_02"]),
            ),
            (
                "zq/maps/zq_gamma_metadata",
                nested(&["zq_gamma_01", "zq_gamma_02"]),
            ),
            (
                "zq/maps/zq_beta_metadata",
                nested(&["zq_beta_01", "zq_beta_02"]),
            ),
            (
                "zq/maps/zq_delta_metadata",
                nested(&["zq_delta_01", "zq_delta_02"]),
            ),
        ]);
        let dir = tempfile::tempdir().expect("a scratch folder");
        std::fs::write(dir.path().join("parent.rpf"), &parent).expect("the archive is written");
        let mut resolver = NameResolver::new();
        for name in [
            "zq/maps/zq_alpha_metadata",
            "zq/maps/zq_gamma_metadata",
            "zq_alpha_01",
            "zq_alpha_02",
            "zq_gamma_01",
            "zq_gamma_02",
            "zq_beta_01",
            "zq_beta_02",
        ] {
            resolver.insert(name_hash64(name.as_bytes()), name);
        }

        let mut miner = Miner::new();
        let cancel = Cancel::new();
        let bounds = Bounds::new(Cap::new(DEFAULT_BODIES, CapUnit::Count("bodies")), &cancel);
        let mut written = Vec::new();
        let refused = mine_archives(
            &mut miner,
            &archives(dir.path()),
            &keys,
            &resolver,
            &bounds,
            &mut written,
        )
        .expect("the rows are written");
        assert_eq!((refused, miner.tree.len()), (0, 5));
        assert!(miner.tree[1..].iter().all(|t| t.parent == Some(0)));
        assert_eq!(
            found(&miner, "zq/maps/zq_beta_metadata"),
            None,
            "no string names it"
        );
        miner.rules(Vec::new(), DEFAULT_RANGE);

        assert_eq!(
            found(&miner, "zq/maps/zq_beta_metadata"),
            Some("zq/maps/zq_beta_metadata")
        );
        assert_eq!(found(&miner, "zq/maps/zq_delta_metadata"), None);
        let mut rows = String::from_utf8(written).expect("the rows are text");
        rows.push_str(&miner.take_rows());
        assert_eq!(rows.lines().count(), miner.rows);
        for line in rows.lines() {
            let hash = line
                .get(2..18)
                .and_then(|h| u64::from_str_radix(h, 16).ok());
            let hash = hash.expect("a 64-bit row");
            assert!(
                miner.entry.contains_key(&hash),
                "{line} names no unresolved hash"
            );
        }
        assert!(
            !rows.contains("zq/maps/maps"),
            "a candidate that matches nothing"
        );
    }

    /// The smallest archives of the build walked for their tables alone, until one holds a nested
    /// table: every nested table sits under an earlier one and was opened from one of its rows. Install-gated.
    #[test]
    fn the_walk_places_every_nested_table_of_the_build_under_the_row_it_was_opened_from() {
        let present = |var: &str| {
            std::env::var_os(var)
                .map(PathBuf::from)
                .filter(|p| p.exists())
        };
        let snapshot = present(or_io::container::rpf::GTA6_SNAPSHOT_ENV);
        let (Some(snapshot), Some(build)) = (snapshot, present("OPENRAGE_GTA6_BUILD")) else {
            or_test_support::skip_install(
                "the nested tables of a GTA VI build",
                format_args!(
                    "set {} and OPENRAGE_GTA6_BUILD",
                    or_io::container::rpf::GTA6_SNAPSHOT_ENV
                ),
            );
            return;
        };
        let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
        let size = |p: &PathBuf| std::fs::metadata(p).map_or(u64::MAX, |m| m.len());
        let mut smallest = archives(&build);
        smallest.sort_by_key(size);
        let cancel = Cancel::new();
        let walked = smallest.iter().take(6).find_map(|path| {
            let mut miner = Miner::new();
            let bounds = Bounds::new(Cap::at(0, CapUnit::Count("bodies")), &cancel);
            let refused = mine_archives(
                &mut miner,
                std::slice::from_ref(path),
                &keys,
                &NameResolver::new(),
                &bounds,
                &mut std::io::sink(),
            )
            .expect("the rows are written");
            (miner.tree.len() > 1).then_some((miner, refused))
        });
        let (miner, refused) = walked.expect("one of the six smallest archives nests one");
        assert_eq!(miner.tree.len(), miner.tables, "{refused} tables refused");
        for (at, table) in miner.tree.iter().enumerate() {
            match (table.parent, table.from) {
                (None, None) => {}
                (Some(parent), Some(from)) => {
                    assert!(parent < at);
                    assert!(miner.tree[parent].rows.contains(&from));
                }
                _ => panic!("table {at} has a parent without a row, or a row without one"),
            }
        }
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
        write_rows(&out, &snapshot, "").expect("merge mining output");
        assert_eq!(std::fs::read_to_string(out).expect("read"), latest);
    }

    /// Three archives of four bodies each, each body stating its own path: the ceiling is per
    /// archive, so every archive is read up to it, and the overall ceiling stops the walk early.
    #[test]
    fn the_per_archive_ceiling_reads_from_every_archive_and_the_total_stops_the_walk() {
        let keys = fixture::keys();
        let dir = tempfile::tempdir().expect("a scratch folder");
        for archive in ["a", "b", "c"] {
            let paths: Vec<String> = (0..4).map(|i| format!("zq/{archive}/body_{i}")).collect();
            let entries: Vec<(&str, Body)> = (paths.iter())
                .map(|path| (path.as_str(), Body::Zlib(fixture::pso(path))))
                .collect();
            let bytes = fixture::archive(&entries);
            std::fs::write(dir.path().join(format!("{archive}.rpf")), bytes).expect("written");
        }
        let paths = archives(dir.path());
        let resolver = NameResolver::new();
        let read = |bodies: Cap, total: Cap| {
            let mut miner = Miner::new();
            let cancel = Cancel::new();
            let mut bounds = Bounds::new(bodies, &cancel);
            bounds.total = total;
            mine_archives(
                &mut miner,
                &paths,
                &keys,
                &resolver,
                &bounds,
                &mut std::io::sink(),
            )
            .expect("the rows are written");
            let lines = miner.walk_lines(&paths, dir.path(), &bounds);
            (miner.archives, lines)
        };
        let unlimited = Cap::at(1, CapUnit::Count("bodies")).lifted();
        let two = Cap::at(2, CapUnit::Count("bodies"));
        let (each, lines) = read(two, unlimited);
        let reads: Vec<(u64, u64)> = each
            .iter()
            .map(|a| (a.schema_bodies, a.string_bodies))
            .collect();
        assert_eq!(
            reads,
            [(2, 2); 3],
            "every archive is read up to its ceiling"
        );
        assert!(
            each.iter().all(|a| a.rows == 2),
            "each body read names itself"
        );
        assert_eq!(lines.len(), 5, "{lines:?}");
        assert_eq!(lines[0], "  archive 1     a.rpf: 2 bodies read for their schemas, 2 for their strings, 2 names found, 1000.0 per 1000 bodies; stopped at the ceiling");
        assert!(
            lines[3].starts_with("  schema walk   3 of 3 archives: STOPPED SHORT at 2 bodies"),
            "{lines:?}"
        );
        assert!(
            lines[4].starts_with("  string walk   3 of 3 archives: STOPPED SHORT"),
            "{lines:?}"
        );

        let (each, lines) = read(unlimited, unlimited);
        assert!(each
            .iter()
            .all(|a| (a.schema_bodies, a.string_bodies, a.rows) == (4, 4, 4)));
        assert_eq!(lines.len(), 3, "nothing stopped short: {lines:?}");

        // A ceiling of exactly an archive's bodies reads them all, so nothing is said to stop.
        let (each, lines) = read(Cap::at(4, CapUnit::Count("bodies")), unlimited);
        assert!(each
            .iter()
            .all(|a| (a.schema_bodies, a.string_bodies) == (4, 4)));
        assert_eq!(
            lines.len(),
            3,
            "a ceiling every body fits stops nothing: {lines:?}"
        );

        let (each, lines) = read(unlimited, Cap::at(6, CapUnit::Count("bodies")));
        let strings: Vec<u64> = each.iter().map(|a| a.string_bodies).collect();
        assert_eq!(strings, [4, 2, 0], "the overall ceiling stops the walk");
        assert_eq!(lines.len(), 5, "{lines:?}");
        assert!(lines[4].contains("STOPPED SHORT at 6 bodies") && lines[4].contains(TOTAL_FLAG));
    }

    #[test]
    fn both_ceilings_parse_and_the_per_archive_one_is_the_default() {
        let args = |rest: &[&str]| {
            let mut all = vec![
                "install".to_string(),
                "--out".to_string(),
                "o.tsv".to_string(),
            ];
            all.extend(rest.iter().map(|a| a.to_string()));
            parse(&all)
        };
        let options = args(&[]).expect("the defaults parse");
        assert_eq!(options.bodies.limit(), Some(DEFAULT_BODIES));
        assert!(options.total.is_unlimited());
        let options = args(&[
            "--max-total=9",
            "--max-bodies",
            "unlimited",
            "--listings",
            "l",
        ])
        .expect("both ceilings parse");
        assert_eq!(options.total.limit(), Some(9));
        assert!(options.bodies.is_unlimited());
        assert_eq!(options.listings, Some(PathBuf::from("l")));
        assert!(args(&["--max-total", "0"]).is_err(), "zero is no ceiling");
        assert!(
            options.chosen.is_empty(),
            "every archive is read by default"
        );

        let options = args(&["--archive", "3", "--archive", "1"]).expect("two archives parse");
        assert_eq!(options.chosen, BTreeSet::from([1, 3]));
        assert_eq!(
            options.bodies.limit(),
            Some(DEFAULT_BODIES),
            "chosen archives keep the ceiling"
        );
        let options = args(&["--archive", "3", "--max-bodies=7"]).expect("a ceiling given");
        assert_eq!(options.bodies.limit(), Some(7), "unless one is asked for");
        assert!(args(&["--archive", "0"]).is_err(), "places start at 1");
        assert!(args(&["--archive", "two"]).is_err());
        assert!(args(&["--archive"]).is_err());
    }

    #[test]
    fn help_is_asked_for_with_either_flag_and_is_no_error() {
        let code = |flag: &str| format!("{:?}", run(&["mine".to_string(), flag.to_string()]));
        let success = format!("{:?}", ExitCode::SUCCESS);
        assert_eq!(code("--help"), success);
        assert_eq!(code("-h"), success);
        assert_eq!(code("--nonsense"), format!("{:?}", ExitCode::from(2)));
    }

    /// Three archives of four bodies each, each body stating its own path, and the second given to
    /// `--archive`: it is read whole, the others give their tables' rows as targets and no body,
    /// and the report says so of each; with a ceiling asked for, the ceiling holds.
    #[test]
    fn an_archive_given_to_archive_is_read_past_the_ceiling_and_the_others_give_their_tables_only()
    {
        let keys = fixture::keys();
        let dir = tempfile::tempdir().expect("a scratch folder");
        for archive in ["a", "b", "c"] {
            let paths: Vec<String> = (0..4).map(|i| format!("zq/{archive}/body_{i}")).collect();
            let entries: Vec<(&str, Body)> = (paths.iter())
                .map(|path| (path.as_str(), Body::Zlib(fixture::pso(path))))
                .collect();
            let bytes = fixture::archive(&entries);
            std::fs::write(dir.path().join(format!("{archive}.rpf")), bytes).expect("written");
        }
        let paths = archives(dir.path());
        let read = |ceiling: &[&str]| {
            let args: Vec<String> = (["install", "--out", "o.tsv", "--archive", "2"].iter())
                .chain(ceiling)
                .map(|a| a.to_string())
                .collect();
            let options = parse(&args).expect("the options parse");
            let (mut miner, cancel) = (Miner::new(), Cancel::new());
            let mut bounds = Bounds::new(options.bodies, &cancel);
            bounds.chosen = options.chosen;
            let resolver = NameResolver::new();
            mine_archives(
                &mut miner,
                &paths,
                &keys,
                &resolver,
                &bounds,
                &mut std::io::sink(),
            )
            .expect("the rows are written");
            let lines = miner.walk_lines(&paths, dir.path(), &bounds);
            (miner, lines)
        };
        let (miner, lines) = read(&[]);
        let each: Vec<(u64, u64, usize)> = (miner.archives.iter())
            .map(|a| (a.schema_bodies, a.string_bodies, a.rows))
            .collect();
        assert_eq!(each, [(0, 0, 0), (4, 4, 4), (0, 0, 0)]);
        assert_eq!(miner.entry.len(), 12, "every table's rows are targets");
        assert_eq!(
            lines,
            [
                "  archive 1     a.rpf: table read, no body (not given to --archive)",
                "  archive 2     b.rpf: 4 bodies read for their schemas, 4 for their strings, 4 names found, 1000.0 per 1000 bodies",
                "  archive 3     c.rpf: table read, no body (not given to --archive)",
            ]
        );

        let (miner, lines) = read(&["--max-bodies", "2"]);
        assert_eq!(
            (miner.archives[1].string_bodies, miner.bodies),
            (2, 2),
            "a ceiling asked for holds"
        );
        assert!(lines[1].ends_with("; stopped at the ceiling"), "{lines:?}");
    }

    /// One directory of named rows that share two stems, one row with a `_lod` ending: the unnamed
    /// row that is the other stem with that ending renumbered is named by the sibling rule, and a
    /// row no stem and ending make is not.
    #[test]
    fn an_unnamed_row_takes_a_stem_its_named_siblings_share_with_an_ending_seen_beside_it() {
        let keys = fixture::keys();
        let named = [
            "zq/props/zq_lamp_01",
            "zq/props/zq_lamp_01_lod",
            "zq/props/zq_bench_02",
            "zq/props/zq_bench_05",
        ];
        let unnamed = ["zq/props/zq_bench_03_lod", "zq/props/zq_never_9"];
        let rows: Vec<(&str, Body)> = (named.iter().chain(&unnamed))
            .map(|path| (*path, Body::Zlib(fixture::pso("zq"))))
            .collect();
        let dir = tempfile::tempdir().expect("a scratch folder");
        std::fs::write(dir.path().join("parent.rpf"), fixture::archive(&rows)).expect("written");
        let mut resolver = NameResolver::new();
        for name in named {
            resolver.insert(name_hash64(name.as_bytes()), name);
        }
        let (mut miner, cancel) = (Miner::new(), Cancel::new());
        let bounds = Bounds::new(Cap::new(DEFAULT_BODIES, CapUnit::Count("bodies")), &cancel);
        let sink = &mut std::io::sink();
        mine_archives(
            &mut miner,
            &archives(dir.path()),
            &keys,
            &resolver,
            &bounds,
            sink,
        )
        .expect("the rows are written");
        assert_eq!(miner.entry.len(), 2);
        assert_eq!(found(&miner, unnamed[0]), None, "no string names it");

        miner.sibling_rules(DEFAULT_RANGE);
        assert_eq!(found(&miner, unnamed[0]), Some(unnamed[0]));
        assert_eq!(found(&miner, unnamed[1]), None);
        assert_eq!(miner.by.get(&(Space::Entry, Source::Rules)), Some(&1));
        assert_eq!(miner.source, Source::Install, "the source is put back");
        let (stems, endings) = sibling_parts(&["zq_a_01".into(), "zq_a_02_lod".into()], 3);
        assert_eq!(stems, ["zq", "zq_a"]);
        assert_eq!(endings[0], "", "the bare stem first");
        assert!(
            endings.contains(&"_02_lod".to_string()) && endings.contains(&"_00_lod".to_string())
        );
        assert!(
            !endings.contains(&"_03_lod".to_string()),
            "the range ends below 3"
        );
        // An ending keeps its extension, and one numbered past the range or past four digits is
        // still tried as it was seen.
        let lasts = [
            "zq_lamp.ydr",
            "zq_lamp_lod.ydr",
            "zq_lamp_0150.ydr",
            "zq_lamp_123456.ydr",
        ];
        let lasts: Vec<String> = lasts.map(String::from).into();
        let (_, endings) = sibling_parts(&lasts, 100);
        for seen in ["_lod.ydr", "_0150.ydr", "_123456.ydr", "_0099.ydr"] {
            assert!(endings.contains(&seen.to_string()), "{seen}: {endings:?}");
        }
    }

    /// The miner after both walks of a fixture archive of one table holding `rows`, each body
    /// stating nothing a row is named by, with the resolver naming `named`.
    fn walked(rows: &[&str], named: &[&str]) -> Miner {
        let keys = fixture::keys();
        let entries: Vec<(&str, Body)> = (rows.iter())
            .map(|path| (*path, Body::Zlib(fixture::pso("zq"))))
            .collect();
        let dir = tempfile::tempdir().expect("a scratch folder");
        std::fs::write(dir.path().join("parent.rpf"), fixture::archive(&entries)).expect("written");
        let mut resolver = NameResolver::new();
        for name in named.iter().copied() {
            resolver.insert(name_hash64(name.as_bytes()), name);
        }
        let (mut miner, cancel) = (Miner::new(), Cancel::new());
        let bounds = Bounds::new(Cap::new(DEFAULT_BODIES, CapUnit::Count("bodies")), &cancel);
        let sink = &mut std::io::sink();
        let paths = archives(dir.path());
        mine_archives(&mut miner, &paths, &keys, &resolver, &bounds, sink)
            .expect("the rows are written");
        miner
    }

    /// Seven named rows and four unnamed ones, each a short head and an `@` or a `_` before a named
    /// row's last component: a head a row the rule samples takes is learnt and names every row it
    /// begins, and a head only an unsampled row takes is not tried.
    #[test]
    fn a_head_a_few_named_rows_take_is_tried_before_every_named_row_of_the_table() {
        let named: Vec<String> = (1..=7)
            .map(|i| format!("zq/props/zq_crate_{i:02}"))
            .collect();
        let named: Vec<&str> = named.iter().map(String::as_str).collect();
        let unnamed = [
            "zq/props/qx@zq_crate_03",
            "zq/props/qx@zq_crate_04",
            "zq/props/zw@zq_crate_06",
            "zq/props/q7_zq_crate_05",
        ];
        let rows: Vec<&str> = named.iter().chain(&unnamed).copied().collect();
        let mut miner = walked(&rows, &named);
        assert_eq!(miner.entry.len(), 4);
        miner.head_rules();
        assert_eq!(
            found(&miner, unnamed[0]),
            Some(unnamed[0]),
            "a sampled row's head"
        );
        assert_eq!(
            found(&miner, unnamed[1]),
            Some(unnamed[1]),
            "the learnt head"
        );
        assert_eq!(
            found(&miner, unnamed[2]),
            None,
            "no sampled row takes its head"
        );
        assert_eq!(
            found(&miner, unnamed[3]),
            Some(unnamed[3]),
            "a head before a `_`"
        );
        assert_eq!(miner.by.get(&(Space::Entry, Source::Rules)), Some(&3));
        assert_eq!(miner.source, Source::Install, "the source is put back");
        let (heads, each) = (short_heads(), 36 + 36 * 36 + 36 * 36 * 36);
        assert_eq!(heads.len(), 2 * each);
        assert_eq!((heads[0].as_str(), heads[36].as_str()), ("a@", "aa@"));
        assert_eq!(
            (heads[each].as_str(), heads.last().map(String::as_str)),
            ("a_", Some("999_"))
        );
    }

    /// Named rows numbered in two digits: an unnamed row their stem numbers in three is named by
    /// the sibling rule, and one in four is not.
    #[test]
    fn a_stem_named_rows_share_is_numbered_past_the_two_digits_they_show() {
        let named = ["zq/w/zq_bay_07", "zq/w/zq_bay_12"];
        let unnamed = ["zq/w/zq_bay_457", "zq/w/zq_bay_1457"];
        let rows: Vec<&str> = named.iter().chain(&unnamed).copied().collect();
        let mut miner = walked(&rows, &named);
        miner.sibling_rules(DEFAULT_RANGE);
        assert_eq!(found(&miner, unnamed[0]), Some(unnamed[0]));
        assert_eq!(found(&miner, unnamed[1]), None, "past the three digits");
        let (_, endings) = sibling_parts(&["zq_bay_07".into(), "zq_bay_12".into()], DEFAULT_RANGE);
        assert!(endings.contains(&"_100".to_string()) && endings.contains(&"_999".to_string()));
        assert!(!endings.contains(&"_1000".to_string()));
    }

    /// Two rows the head rule names give their directory a stem and an ending no row had: the
    /// sibling rule names the third row only when it runs again after the head rule, as the rules'
    /// rounds run it.
    #[test]
    fn the_sibling_rule_runs_again_over_the_names_the_other_rules_found() {
        let named = ["zq/c/zq_lamp_01", "zq/c/zq_lamp_02", "zq/c/zq_lamp_02_lod"];
        let unnamed = [
            "zq/c/hi@zq_lamp_01",
            "zq/c/hi@zq_lamp_02",
            "zq/c/hi@zq_lamp_01_lod",
        ];
        let rows: Vec<&str> = named.iter().chain(&unnamed).copied().collect();
        let mut once = walked(&rows, &named);
        once.sibling_rules(DEFAULT_RANGE);
        once.head_rules();
        assert!(found(&once, unnamed[0]).is_some() && found(&once, unnamed[1]).is_some());
        assert_eq!(
            found(&once, unnamed[2]),
            None,
            "one sibling pass before the heads"
        );
        let mut miner = walked(&rows, &named);
        miner.rules(Vec::new(), DEFAULT_RANGE);
        for path in unnamed {
            assert_eq!(found(&miner, path), Some(path));
        }
        assert_eq!(miner.rows, 3, "one row per name");
    }

    /// A document whose one string member is a sound's name, beside a named file of the same
    /// directory: the name joined with that file's extension names the unnamed file; the same
    /// string outside a document is joined without it, and names nothing.
    #[test]
    fn a_string_member_a_document_reads_to_is_joined_with_the_extensions_its_named_siblings_carry()
    {
        let paths = ["zq/sfx/zq_horn.awc", "zq/sfx/zq_siren.awc"];
        let entries: Vec<Gta6Entry> = (paths.iter().zip(0..))
            .map(|(path, at)| entry(at, path, Some(name_hash32(path.as_bytes()))))
            .collect();
        let mut resolver = NameResolver::new();
        resolver.insert(entries[0].name64, paths[0]);
        let mut document = or_io::meta::MetaInstance::new("CZqSounds");
        document.set("Sound", Value::String("zq_siren".into()));
        let rbf = or_io::meta::write_meta_binary(BinaryFormat::Rbf, None, &document)
            .expect("the document encodes");
        let mine = |body: Vec<u8>| {
            let mut miner = Miner::new();
            let named = miner.add_table(&entries, &resolver);
            miner.mine_table(&named, vec![body], None);
            miner
        };
        let miner = mine(rbf);
        assert_eq!(found(&miner, paths[1]), Some(paths[1]));
        assert_eq!(miner.by.get(&(Space::Entry, Source::Rules)), Some(&1));
        assert_eq!(found(&mine(b"\0zq_siren\0".to_vec()), paths[1]), None);
    }

    #[test]
    fn the_strings_of_a_value_are_its_own_its_hashes_names_and_those_inside_it() {
        use or_io::meta::instance::HashedString;
        let mut inner = or_io::meta::MetaInstance::new("CZqInner");
        inner.set("Name", Value::String("zq_inner".into()));
        let value = Value::Array(vec![
            Value::Struct(Box::new(inner)),
            Value::Hash(HashedString::new("zq_hashed")),
            Value::Hash(HashedString::from_hash(7)),
            Value::Map(vec![(Value::String("zq_key".into()), Value::UInt(1))]),
            Value::Pointer(None),
        ]);
        let mut got = Vec::new();
        value_strings(&value, &mut |text| got.push(text.to_string()));
        assert_eq!(got, ["zq_inner", "zq_hashed", "zq_key"]);
    }

    /// The class and member names a run found go to the configuration folder's table, where the
    /// metadata reader reads them with no setting, and a later merge of entry names keeps them.
    #[test]
    fn the_class_and_member_names_reach_the_metadata_reader_from_the_configuration_folder() {
        let config = tempfile::tempdir().expect("a configuration folder");
        let mut miner = Miner::new();
        let class = literal_string_hash(b"ZqWidgetMeta");
        let member = string_hash(b"zqIntensity");
        miner.class.insert(class);
        miner.member.insert(member);
        miner.consider(b"\0ZqWidgetMeta\0zqIntensity\0", None);
        miner
            .found64
            .insert(name_hash64(b"zq/a/one"), "zq/a/one".to_string());
        merge_into_config(&miner, config.path());

        let table = ragepar_names::mined_table(config.path());
        let cached = vocabulary_file(config.path()).with_file_name("resolved-names.tsv");
        assert_eq!(table, cached, "the names cache reads the same table");
        let two = [(name_hash64(b"zq/b/two"), "zq/b/two")];
        NameResolver::cache_names(table.parent().expect("a folder"), two).expect("merged");
        let names = ragepar_names::load_from(None, Some(config.path())).expect("the table reads");
        assert_eq!(names.name(class), Some("ZqWidgetMeta"));
        assert_eq!(names.name(member), Some("zqIntensity"));
        let resolver = NameResolver::load_cached(None, Some(config.path()));
        assert_eq!(resolver.lookup(name_hash64(b"zq/a/one")), Some("zq/a/one"));
        assert_eq!(resolver.lookup(name_hash64(b"zq/b/two")), Some("zq/b/two"));
        // A class or member row names no entry.
        assert_eq!(resolver.lookup(u64::from(class)), None);
        assert_eq!(resolver.lookup(u64::from(member)), None);

        let before = std::fs::read_to_string(&table).expect("read");
        merge_into_config(&miner, config.path());
        let after = std::fs::read_to_string(&table).expect("read");
        assert_eq!(after, before, "a second run of the same names adds no row");
    }

    /// Two directories joined with three basenames: each join whose hash is an unresolved entry's
    /// is found, whole or without its extension, and no other join makes a row.
    #[test]
    fn a_join_is_tried_whole_and_without_its_extension_against_the_unresolved_entries_only() {
        let targets = ["zq/a/lamp.xml", "zq/b/lamp", "zq/a/crate"];
        let entries: Vec<Gta6Entry> = (targets.iter().zip(0..))
            .map(|(path, at)| entry(at, path, Some(name_hash32(path.as_bytes()))))
            .collect();
        let mut miner = Miner::new();
        miner.add_table(&entries, &NameResolver::new());
        let dirs: BTreeSet<String> = ["zq/a/", "zq/b/"].map(String::from).into();
        let basenames: BTreeSet<String> = ["lamp.xml", "crate", "never"].map(String::from).into();
        miner.join(&dirs, &basenames, &[]);
        for path in targets {
            assert_eq!(found(&miner, path), Some(path));
        }
        assert_eq!(miner.rows, 3, "one row per unresolved entry joined");
        assert_eq!(miner.by.get(&(Space::Entry, Source::Rules)), Some(&3));
        assert_eq!(miner.joins_skipped, 0);
        assert_eq!(miner.source, Source::Install, "the source is put back");
    }

    #[test]
    fn a_literal_ends_at_its_quote_or_its_line_and_a_backslash_keeps_a_quote_inside() {
        let text =
            b"Load(\"zq/a/one\"); x = \"two \\\"q\\\" three\"; y = \"open\nz = \"zq/b/four\"";
        let got: Vec<&[u8]> = string_literals(text);
        let want: [&[u8]; 4] = [b"zq/a/one", b"two \\\"q\\\" three", b"open", b"zq/b/four"];
        assert_eq!(got, want);
        assert!(string_literals(b"no quote at all").is_empty());
        // A backslash at a line's end does not carry the literal onto the next line.
        let text = b"x = \"a\\\nLoad(\"zq/b/c\")";
        let want: [&[u8]; 2] = [b"a\\", b"zq/b/c"];
        assert_eq!(string_literals(text), want);
    }

    /// A folder of one listing and one other file: a literal of the listing names an entry and a
    /// structure, and the other file is not read.
    #[test]
    fn the_string_literals_of_the_lifted_listings_name_their_entries_and_structures() {
        let dir = tempfile::tempdir().expect("a scratch folder");
        let listing = "void Main() { Play(\"zq/streams/zq_intro_01\"); Make(\"ZqWidgetMeta\"); }";
        std::fs::write(dir.path().join(format!("zq{LISTING_SUFFIX}")), listing).expect("written");
        std::fs::write(dir.path().join("zq.txt"), "\"zq/streams/zq_other\"").expect("written");
        let targets = ["zq/streams/zq_intro_01", "zq/streams/zq_other"];
        let entries: Vec<Gta6Entry> = (targets.iter().zip(0..))
            .map(|(path, at)| entry(at, path, Some(name_hash32(path.as_bytes()))))
            .collect();
        let mut miner = Miner::new();
        miner.add_table(&entries, &NameResolver::new());
        let class = literal_string_hash(b"ZqWidgetMeta");
        miner.class.insert(class);
        assert_eq!(
            miner.mine_listings(dir.path()).expect("the folder reads"),
            1
        );
        assert_eq!(found(&miner, targets[0]), Some(targets[0]));
        assert_eq!(found(&miner, targets[1]), None, "not a listing");
        assert_eq!(
            miner.found_class.get(&class).map(String::as_str),
            Some("ZqWidgetMeta")
        );
        assert_eq!(miner.by.get(&(Space::Entry, Source::Listings)), Some(&1));
        assert!(miner.lines(0)[1].contains("1 from the script listings"));
        assert!(miner.mine_listings(&dir.path().join("absent")).is_err());
    }

    #[test]
    fn the_short_endings_are_every_underscore_and_one_to_three_letters_or_digits() {
        let endings = short_endings();
        assert_eq!(endings.len(), 36 + 36 * 36 + 36 * 36 * 36);
        assert_eq!((endings[0].as_str(), endings[36].as_str()), ("_a", "_aa"));
        assert!(endings.iter().any(|e| e == "_z9q"));
        assert!(endings
            .iter()
            .all(|e| e.starts_with('_') && (2..=4).contains(&e.len())));
    }

    /// Two archives: one holds a named nested archive, the other an unnamed one built of the same
    /// rows and named the first one's name and a short ending, and an unnamed one whose rows no
    /// named archive holds. Only the twin is named, and by the twin rule alone.
    #[test]
    fn an_unnamed_nested_archive_is_named_after_the_named_archive_whose_rows_it_holds() {
        let keys = fixture::keys();
        let nested = |rows: &[&str]| {
            let entries: Vec<(&str, Body)> = (rows.iter())
                .map(|row| (*row, Body::Zlib(fixture::pso("zq"))))
                .collect();
            Body::Nested(fixture::archive(&entries))
        };
        let rows = ["zq_r1", "zq_r2", "zq_r3"];
        let first = fixture::archive(&[("zq/maps/zq_alpha", nested(&rows))]);
        let second = fixture::archive(&[
            ("zq/maps/zq_alpha_q7", nested(&rows[..2])),
            ("zq/maps/zq_omega_q7", nested(&["zq_r9"])),
        ]);
        let dir = tempfile::tempdir().expect("a scratch folder");
        std::fs::write(dir.path().join("a.rpf"), first).expect("written");
        std::fs::write(dir.path().join("b.rpf"), second).expect("written");
        let mut resolver = NameResolver::new();
        resolver.insert(name_hash64(b"zq/maps/zq_alpha"), "zq/maps/zq_alpha");

        let mut miner = Miner::new();
        let cancel = Cancel::new();
        let bounds = Bounds::new(Cap::at(1, CapUnit::Count("bodies")), &cancel);
        let paths = archives(dir.path());
        mine_archives(
            &mut miner,
            &paths,
            &keys,
            &resolver,
            &bounds,
            &mut std::io::sink(),
        )
        .expect("the rows are written");
        miner.nested_rules();
        assert_eq!(
            found(&miner, "zq/maps/zq_alpha_q7"),
            None,
            "no other rule names it"
        );
        miner.twin_rules();
        assert_eq!(
            found(&miner, "zq/maps/zq_alpha_q7"),
            Some("zq/maps/zq_alpha_q7")
        );
        assert_eq!(found(&miner, "zq/maps/zq_omega_q7"), None, "it has no twin");
        assert_eq!(miner.by.get(&(Space::Entry, Source::Rules)), Some(&1));
        assert_eq!(miner.source, Source::Install, "the source is put back");
    }

    /// Every table of the build placed with the user's names, then the twin rule: the nested
    /// archives left unnamed after it are at most `UNNAMED_SHARE` of them (110 of 4351 with round
    /// 37's table, where the rule names none and 107 have no named twin; round 35's left about 100
    /// after the rule named 1195), and each one it names was unresolved before. Install-gated, and
    /// needs the user's names folder: without it the build's nested archives are nearly all
    /// unnamed, so none has a named twin.
    #[test]
    fn the_miner_runs_on_the_build_within_its_ceiling_and_names_the_twin_archives() {
        /// The share of the nested archives the user's names and the twin rule may leave unnamed.
        const UNNAMED_SHARE: f64 = 0.05;
        let present = |var: &str| {
            std::env::var_os(var)
                .map(PathBuf::from)
                .filter(|p| p.exists())
        };
        let snapshot = present(or_io::container::rpf::GTA6_SNAPSHOT_ENV);
        let names = present(or_io::container::rpf::GTA6_NAMES_ENV);
        let (Some(snapshot), Some(build), Some(_)) =
            (snapshot, present("OPENRAGE_GTA6_BUILD"), names)
        else {
            or_test_support::skip_install(
                "the twin archives of a GTA VI build",
                format_args!(
                    "set {}, OPENRAGE_GTA6_BUILD and {}, the user's names folder",
                    or_io::container::rpf::GTA6_SNAPSHOT_ENV,
                    or_io::container::rpf::GTA6_NAMES_ENV
                ),
            );
            return;
        };
        let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
        let resolver = load_resolver(None);
        let mut miner = Miner::new();
        let mut refused = 0;
        for path in archives(&build) {
            walk_archive(&path, &keys, &mut refused, &mut |_, mount, _, at| {
                miner.place(at, &mount.archive.entries);
                miner.add_table(&mount.archive.entries, &resolver);
            });
        }
        let unresolved: HashSet<u64> = miner.entry.keys().copied().collect();
        let unnamed = |miner: &Miner| {
            (miner.tree.iter())
                .filter_map(|t| t.from)
                .filter(|&from| miner.unresolved(from))
                .count()
        };
        let nested = miner.tree.iter().filter(|t| t.from.is_some()).count();
        let before = unnamed(&miner);
        miner.twin_rules();
        let (named, after) = (miner.found64.len(), unnamed(&miner));
        println!(
            "{named} of {before} unnamed nested archives named after their twins, {after} of {nested} left"
        );
        assert!(nested > 0 && named <= before, "{refused} tables refused");
        assert!(
            (after as f64) <= UNNAMED_SHARE * nested as f64,
            "{after} of {nested} nested archives left unnamed, {refused} tables refused"
        );
        assert!(miner.found64.keys().all(|hash| unresolved.contains(hash)));
    }

    /// The build's key snapshot and folder, or `None` after the skip is said.
    fn the_build(what: &str) -> Option<(GtaViKeys, PathBuf)> {
        let present = |var: &str| {
            std::env::var_os(var)
                .map(PathBuf::from)
                .filter(|p| p.exists())
        };
        let snapshot = present(or_io::container::rpf::GTA6_SNAPSHOT_ENV);
        let (Some(snapshot), Some(build)) = (snapshot, present("OPENRAGE_GTA6_BUILD")) else {
            or_test_support::skip_install(
                what,
                format_args!(
                    "set {} and OPENRAGE_GTA6_BUILD",
                    or_io::container::rpf::GTA6_SNAPSHOT_ENV
                ),
            );
            return None;
        };
        let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
        Some((keys, build))
    }

    /// The three smallest archives of the build with the second alone given to `--archive`: the
    /// others give their tables and no body, and it is read whole. Install-gated.
    #[test]
    fn the_miner_runs_on_the_build_within_its_ceiling_on_the_chosen_archive_alone() {
        let Some((keys, build)) = the_build("hashes mine --archive over a GTA VI build") else {
            return;
        };
        let size = |p: &PathBuf| std::fs::metadata(p).map_or(u64::MAX, |m| m.len());
        let mut smallest = archives(&build);
        smallest.sort_by_key(size);
        smallest.truncate(3);
        let cancel = Cancel::new();
        let mut bounds = Bounds::new(Cap::at(1, CapUnit::Count("bodies")).lifted(), &cancel);
        bounds.chosen = BTreeSet::from([2]);
        let mut miner = Miner::new();
        let sink = &mut std::io::sink();
        let resolver = NameResolver::new();
        let refused = mine_archives(&mut miner, &smallest, &keys, &resolver, &bounds, sink)
            .expect("the rows are written");
        let read: Vec<(u64, u64)> = (miner.archives.iter())
            .map(|a| (a.schema_bodies, a.string_bodies))
            .collect();
        assert_eq!(
            (read[0], read[2]),
            ((0, 0), (0, 0)),
            "{refused} tables refused"
        );
        assert!(read[1].1 > 0, "the chosen archive's bodies are read");
        assert!(!miner.archives[1].string_short, "and read whole");
        assert!(miner.tables >= 3, "every archive's table is read");
    }

    /// Every table of the build placed with the user's names, then the sibling rule: whatever it
    /// names was unresolved, one row each. Install-gated, and needs the user's names folder,
    /// without which no row has a named sibling.
    #[test]
    fn the_miner_runs_on_the_build_within_its_ceiling_and_names_siblings_that_were_unresolved() {
        let names = std::env::var_os(or_io::container::rpf::GTA6_NAMES_ENV)
            .filter(|dir| Path::new(dir).exists());
        if names.is_none() {
            or_test_support::skip_install(
                "the sibling rule over a GTA VI build",
                format_args!("set {}", or_io::container::rpf::GTA6_NAMES_ENV),
            );
            return;
        }
        let Some((keys, build)) = the_build("the sibling rule over a GTA VI build") else {
            return;
        };
        let resolver = load_resolver(None);
        let mut miner = Miner::new();
        let mut refused = 0;
        for path in archives(&build) {
            walk_archive(&path, &keys, &mut refused, &mut |_, mount, _, at| {
                miner.place(at, &mount.archive.entries);
                miner.add_table(&mount.archive.entries, &resolver);
            });
        }
        let unresolved: HashSet<u64> = miner.entry.keys().copied().collect();
        miner.sibling_rules(DEFAULT_RANGE);
        let named = miner.found64.len();
        println!("{named} rows named by their siblings, {refused} tables refused");
        assert!(miner.found64.keys().all(|hash| unresolved.contains(hash)));
        assert_eq!(miner.rows, named, "one row per name");
    }

    /// Every table of the build placed with the user's names, then the head rule: whatever it
    /// names was unresolved, one row each. Install-gated, and needs the user's names folder,
    /// without which a table has no named row to learn a head from.
    #[test]
    fn the_miner_runs_on_the_build_within_its_ceiling_and_names_rows_by_their_tables_heads() {
        let names = std::env::var_os(or_io::container::rpf::GTA6_NAMES_ENV)
            .filter(|dir| Path::new(dir).exists());
        if names.is_none() {
            or_test_support::skip_install(
                "the head rule over a GTA VI build",
                format_args!("set {}", or_io::container::rpf::GTA6_NAMES_ENV),
            );
            return;
        }
        let Some((keys, build)) = the_build("the head rule over a GTA VI build") else {
            return;
        };
        let resolver = load_resolver(None);
        let mut miner = Miner::new();
        let mut refused = 0;
        for path in archives(&build) {
            walk_archive(&path, &keys, &mut refused, &mut |_, mount, _, at| {
                miner.place(at, &mount.archive.entries);
                miner.add_table(&mount.archive.entries, &resolver);
            });
        }
        let unresolved: HashSet<u64> = miner.entry.keys().copied().collect();
        miner.head_rules();
        let named = miner.found64.len();
        println!("{named} rows named by a head their tables take, {refused} tables refused");
        assert!(miner.found64.keys().all(|hash| unresolved.contains(hash)));
        assert_eq!(miner.rows, named, "one row per name");
    }
}
