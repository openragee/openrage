//! `hashes census`: how many hashes of an install resolve to a name, per hash space, over the
//! mounted archive tables and the bodies that read; with `--out`, the unresolved ones, each with one
//! place it occurs, written to a file the user names.
//!
//! ⚠ What it prints is counts only. The unresolved list names archives and entries, so it is written
//! only where `--out` says, never into the repository.

use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use or_io::container::rpf::{
    read_rpf9_body, Gta6Entry, Gta6Mount, GtaViKeys, NameResolver, RpfArchive, RPF9_MAGIC,
    RPF9_SLOT,
};
use or_io::gta6::hash::name_hash32;
use or_io::gta6::names::{vocabulary_file, Vocabulary};
use or_io::gta6::script::compiled::{Compiled, NATIVE};
use or_io::gta6::script::native_addresses::NativeAddresses;
use or_io::meta::binary::hash_index::spelled_hash;
use or_io::meta::binary::pso::string_names::document_strings;
use or_io::meta::binary::root_types;
use or_io::meta::{BinaryFormat, ErrorMode, MetaInstance, Value};
use or_io::{literal_string_hash, string_hash};
use or_job::Cancel;

use super::census_cap::{Cap, CapUnit};

const USAGE: &str = "\
usage:
  openrage-cli hashes census <install> [--out <file>] [--top-only] [--every <n>] [--limit <bytes>]
                             [--max-bodies <n>|unlimited] [--names <dir>] [--keys <file>] [--exe <exe>]
                             [--listings <dir>]
  openrage-cli hashes mine <install | archive.rpf> --out <file> [--names <dir>] [--keys <file>]
                           [--exe <exe>] [--range <count>] [--max-bodies <n>|unlimited]

Counts, per hash space, the hashes an install holds, how many resolve to a name and how many do
not, and how many each source of names resolved. A GTA VI install is walked through its mounted
tables, nested archives included, every body read by its table row; a GTA V install through its
RPF7 archives, whose entry names are text, and its metadata files.
  --out <file>        write each unresolved hash, its space and one place it occurs, to <file>
  --top-only          do not descend into nested archives
  --every <n>         read one body in every <n> (default 1)
  --limit <bytes>     skip a body larger than this (default 16 MiB)
  --max-bodies <n>    stop reading bodies after <n> (default 5000), or `unlimited`
  --names <dir>       GTA VI: a recovered-names folder (else OPENRAGE_GTA6_NAMES)
  --keys <file>       GTA VI: a key snapshot; GTA V: a keys cache
  --exe <exe>         GTA VI: the executable a masked key snapshot opens with
  --listings <dir>    GTA VI: a folder of lifted script listings, whose declarations name natives
                      by address (else OPENRAGE_LIFTED_LISTINGS); the table is cached in the
                      configuration folder

`hashes mine` writes names found for the unresolved hashes, from the install's strings, the
executable's and rules over the names known, to <file>; `hashes mine --help` says more.
`hashes cross <install> --out <file>` names them from the parts of the names known, reading no
body; `hashes cross --help` says more.";

/// The environment variable naming a folder of lifted listings, as the decompiler's tests read it.
const LISTINGS_ENV: &str = "OPENRAGE_LIFTED_LISTINGS";
/// The content cipher type the mount deciphers.
const CONTENT_CIPHER: u16 = 17;
/// A cyclic mount would recurse forever; real nesting is shallow.
const MAX_DEPTH: usize = 8;
const DEFAULT_LIMIT: u64 = 16 << 20;
const DEFAULT_BODIES: u64 = 5000;
const BODIES_FLAG: &str = "--max-bodies";
/// GTA V's metadata files, read through the metadata readers.
const GTA5_METADATA: [&str; 4] = ["ymt", "ymf", "ymap", "ytyp"];
/// The sources a resolved hash is credited to, as the report names them.
const NAMES_TABLES: &str = "names-folder tables";
const NAMES_VOCABULARY: &str = "names-folder vocabulary";
const KEPT_VOCABULARY: &str = "kept vocabulary";
const CACHE_AND_WORDS: &str = "local names cache and bundled words";
const READER: &str = "metadata reader";
const ROOT_TYPES: &str = "root type table";
const DOCUMENT_STRINGS: &str = "document strings";
const LISTINGS: &str = "lifted listings";

/// A space of hashes that name one kind of thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Space {
    Name64,
    Name32,
    Column,
    Class,
    Member,
    Enum,
    Text,
    Native,
}

const SPACES: [Space; 8] = [
    Space::Name64,
    Space::Name32,
    Space::Column,
    Space::Class,
    Space::Member,
    Space::Enum,
    Space::Text,
    Space::Native,
];

impl Space {
    fn label(self) -> &'static str {
        match self {
            Self::Name64 => "entry name64",
            Self::Name32 => "entry name32",
            Self::Column => "table column",
            Self::Class => "metadata class",
            Self::Member => "metadata member",
            Self::Enum => "metadata enum",
            Self::Text => "document string",
            Self::Native => "script native",
        }
    }

    /// Whether the space's hashes are 64-bit.
    fn wide(self) -> bool {
        matches!(self, Self::Name64 | Self::Column)
    }
}

/// One space's distinct hashes: those resolved, each credited to the source that first named it,
/// and those not with the first place each was seen.
#[derive(Default)]
struct Count {
    resolved: HashSet<u64>,
    by: BTreeMap<&'static str, usize>,
    unresolved: BTreeMap<u64, String>,
}

impl Count {
    fn seen(&self) -> usize {
        self.resolved.len() + self.unresolved.len()
    }

    fn resolved(&self) -> usize {
        self.resolved.len()
    }

    fn unresolved(&self) -> &BTreeMap<u64, String> {
        &self.unresolved
    }
}

/// What one run counted.
#[derive(Default)]
struct Census {
    counts: [Count; 8],
    archives: u64,
    nested: u64,
    /// Archives and nested tables that could not be opened, or nested past [`MAX_DEPTH`].
    refused: u64,
    /// GTA V entries, named by text in their tables.
    text_names: u64,
    /// Bodies admitted to a read, and those that would not read.
    bodies: u64,
    unread: u64,
    candidates: u64,
    sampled_out: u64,
    over_limit: u64,
    /// Bodies under a cipher the key snapshot cannot open.
    no_key: u64,
    metadata: u64,
    metadata_refused: u64,
    scripts: u64,
    /// How many natives the lifted listings declare by address, `None` where none were given.
    native_table: Option<usize>,
    /// The sources of entry names the walk looked up, in the order it tried them.
    entry_sources: Vec<&'static str>,
    /// Whether a body was passed over because the ceiling was reached.
    stopped: bool,
    cancelled: bool,
}

impl Census {
    fn count(&self, space: Space) -> &Count {
        &self.counts[space as usize]
    }

    /// Record `key` in `space`, resolved `by` a source or not; `place` is built only for a hash
    /// first seen unresolved.
    fn note(
        &mut self,
        space: Space,
        key: u64,
        by: Option<&'static str>,
        place: impl FnOnce() -> String,
    ) {
        let count = &mut self.counts[space as usize];
        match by {
            Some(source) => {
                count.unresolved.remove(&key);
                if count.resolved.insert(key) {
                    *count.by.entry(source).or_default() += 1;
                }
            }
            None if !count.resolved.contains(&key) => {
                count.unresolved.entry(key).or_insert_with(place);
            }
            None => {}
        }
    }

    /// The sources a hash of `space` can be resolved by in this run.
    fn sources(&self, space: Space) -> Vec<&'static str> {
        match space {
            Space::Name64 | Space::Name32 | Space::Column => self.entry_sources.clone(),
            Space::Class => vec![READER, ROOT_TYPES],
            Space::Member | Space::Enum => vec![READER],
            Space::Text => vec![READER, DOCUMENT_STRINGS],
            Space::Native => self.native_table.map_or(Vec::new(), |_| vec![LISTINGS]),
        }
    }
}

/// How far a run reads.
struct Options {
    top_only: bool,
    every: u64,
    limit: u64,
    bodies: Cap,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            top_only: false,
            every: 1,
            limit: DEFAULT_LIMIT,
            bodies: Cap::new(DEFAULT_BODIES, CapUnit::Count("bodies")),
        }
    }
}

/// The game an install's archives are of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Game {
    Gta6,
    Gta5,
}

pub(crate) fn run(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("census") => match census_command(&args[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("Error: {e}");
                ExitCode::FAILURE
            }
        },
        Some("-h" | "--help" | "help") => {
            eprintln!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn census_command(args: &[String]) -> Result<(), String> {
    let (every, args) = super::rpf::split_offset_flag(args, "--every")?;
    let (limit, args) = super::rpf::split_offset_flag(&args, "--limit")?;
    let mut options = Options {
        every: every.unwrap_or(1).max(1),
        limit: limit.unwrap_or(DEFAULT_LIMIT),
        ..Options::default()
    };
    let (mut root, mut out, mut names, mut keys, mut exe) = (None, None, None, None, None);
    let mut listings = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if let Some(v) = arg.strip_prefix("--max-bodies=") {
            options.bodies = options.bodies.parse(BODIES_FLAG, v)?;
            continue;
        }
        let mut value = |flag: &str| {
            it.next()
                .cloned()
                .ok_or(format!("{flag} needs a value\n{USAGE}"))
        };
        match arg.as_str() {
            "--top-only" => options.top_only = true,
            "--out" => out = Some(PathBuf::from(value("--out")?)),
            "--names" => names = Some(PathBuf::from(value("--names")?)),
            "--keys" => keys = Some(PathBuf::from(value("--keys")?)),
            "--exe" => exe = Some(PathBuf::from(value("--exe")?)),
            "--listings" => listings = Some(PathBuf::from(value("--listings")?)),
            BODIES_FLAG => {
                options.bodies = options.bodies.parse(BODIES_FLAG, &value(BODIES_FLAG)?)?
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown option {other}\n{USAGE}"))
            }
            other if root.is_none() => root = Some(PathBuf::from(other)),
            _ => return Err(format!("give one install\n{USAGE}")),
        }
    }
    let root = root.ok_or(format!("give an install to census\n{USAGE}"))?;
    let archives = archives_under(&root);
    let game = game_of(&archives).ok_or(format!("no RPF archive under {}", root.display()))?;
    let cancel = super::jobs::on_interrupt();
    let census = match game {
        Game::Gta6 => {
            let keys = super::gta6_keys::load(keys.as_deref(), exe.as_deref())?;
            let names = load_names(names.as_deref());
            let natives = load_natives(listings.as_deref())?;
            let gta6 = Gta6 {
                keys: &keys,
                names: &names,
                natives: natives.as_ref(),
            };
            census_gta6(&root, &archives, &options, &cancel, &gta6)
        }
        Game::Gta5 => {
            let crypto = super::keys::resolve_crypto(&root, keys.as_deref());
            for note in &crypto.notes {
                eprintln!("note: {note}");
            }
            let mut walk = Walk::new(&options, &cancel);
            walk.gta5(&root, &archives, &crypto.provider);
            walk.census
        }
    };
    print!("{}", report(&census, game, &options));
    if let Some(out) = out {
        let written = write_unresolved(&census, &out)?;
        println!("  wrote {written} unresolved hashes to {}", out.display());
    }
    match census.cancelled {
        true => Err("cancelled".to_string()),
        false => Ok(()),
    }
}

/// The census of a GTA VI install's `archives`, opened and named with what `gta6` holds.
fn census_gta6(
    root: &Path,
    archives: &[PathBuf],
    options: &Options,
    cancel: &Cancel,
    gta6: &Gta6,
) -> Census {
    let mut walk = Walk::new(options, cancel);
    walk.census.native_table = gta6.natives.map(NativeAddresses::len);
    walk.census.entry_sources = gta6.names.iter().map(|s| s.label).collect();
    let keys = gta6.keys;
    for (i, path) in archives.iter().enumerate() {
        if cancel.is_set() {
            walk.census.cancelled = true;
            break;
        }
        let label = relative_label(root, path);
        eprintln!("  [{}/{}] {label}", i + 1, archives.len());
        let mounted = File::open(path)
            .map_err(|e| e.to_string())
            .and_then(|mut file| {
                let mount = Gta6Mount::open_root(&mut file, keys).map_err(|e| e.to_string())?;
                Ok((file, mount))
            });
        match mounted {
            Ok((mut file, mount)) => {
                walk.census.archives += 1;
                walk.gta6_mount(gta6, &mut file, &mount, &label, 0);
            }
            Err(e) => {
                eprintln!("  {label}: {e}");
                walk.census.refused += 1;
            }
        }
    }
    walk.census
}

/// What a GTA VI walk opens and names with.
struct Gta6<'a> {
    keys: &'a GtaViKeys,
    /// The sources of entry names, tried in order.
    names: &'a [Source],
    natives: Option<&'a NativeAddresses>,
}

impl<'a> Gta6<'a> {
    /// The path whose name64 is `hash`, and the first source that names it.
    fn name64(&self, hash: u64) -> Option<(&'a str, &'static str)> {
        let names = self.names;
        names
            .iter()
            .find_map(|s| s.resolver.lookup(hash).map(|name| (name, s.label)))
    }

    /// The first source whose vocabulary names the name32 `hash`.
    fn name32(&self, hash: u32) -> Option<&'static str> {
        let names = self.names;
        let source = names
            .iter()
            .find(|s| s.resolver.lookup_name32(hash).is_some());
        source.map(|s| s.label)
    }
}

/// A source of entry names, under the label the report credits it with.
struct Source {
    label: &'static str,
    resolver: NameResolver,
}

/// `entry`'s size as its row states it: a 24-byte row's inflated size, else a 20-byte row's
/// stored extent.
pub(super) fn row_size(mount: &Gta6Mount, entry: &Gta6Entry) -> Option<u64> {
    entry.real.or_else(|| slot_extent(mount, entry))
}

/// The stored extent of a 20-byte row, the row of a headerless body that follows a 16-byte slot;
/// `None` for any other row.
fn slot_extent(mount: &Gta6Mount, entry: &Gta6Entry) -> Option<u64> {
    if entry.stored.is_some() {
        return None;
    }
    let extra = mount.archive.entry_extra(entry.index as usize)?;
    extra.nested.map(|row| row.stored_size())
}

/// `entry`'s inflated body, read by its table row: deciphered and walked by its chunk table, or
/// for a clear 20-byte row, read past the slot its headerless body follows, inflating at most
/// `limit` bytes.
pub(super) fn read_row(
    file: &mut File,
    mount: &Gta6Mount,
    entry: &Gta6Entry,
    keys: &GtaViKeys,
    limit: u64,
) -> Result<Vec<u8>, String> {
    if entry.cipher.is_none() && slot_extent(mount, entry).is_some() {
        let start = mount.absolute_start(entry).ok_or("no entry row")?;
        let read = read_rpf9_body(&mut *file, start + RPF9_SLOT, limit);
        return read.map(|(_, bytes)| bytes).map_err(|e| e.to_string());
    }
    (mount.read_entry_decrypted(&mut *file, entry, keys)).map_err(|e| e.to_string())
}

struct Walk<'a> {
    options: &'a Options,
    cancel: &'a Cancel,
    census: Census,
}

impl<'a> Walk<'a> {
    fn new(options: &'a Options, cancel: &'a Cancel) -> Self {
        Self {
            options,
            cancel,
            census: Census::default(),
        }
    }

    /// Whether the next body is read: under the ceiling and picked by the sampling.
    fn admit(&mut self) -> bool {
        if self.options.bodies.reached(self.census.bodies) {
            self.census.stopped = true;
            return false;
        }
        self.census.candidates += 1;
        if !(self.census.candidates - 1).is_multiple_of(self.options.every) {
            self.census.sampled_out += 1;
            return false;
        }
        self.census.bodies += 1;
        true
    }

    fn gta6_mount(
        &mut self,
        gta6: &Gta6,
        file: &mut File,
        mount: &Gta6Mount,
        label: &str,
        depth: usize,
    ) {
        for group in mount.archive.groups() {
            let by = gta6.name64(group.name_hash).map(|(_, source)| source);
            self.census
                .note(Space::Column, group.name_hash, by, || label.to_string());
        }
        for entry in &mount.archive.entries {
            if self.cancel.is_set() {
                self.census.cancelled = true;
                return;
            }
            let place = || format!("{label}#{}", entry.index);
            let named = gta6.name64(entry.name64);
            let name = named.map(|(name, _)| name);
            if entry.name64 != 0 {
                let by = named.map(|(_, source)| source);
                self.census.note(Space::Name64, entry.name64, by, place);
            }
            if let Some(hash) = entry.name32 {
                // A name32 is also named by the path its name64 resolves to.
                let by = gta6.name32(hash).or_else(|| {
                    let named = named.filter(|(n, _)| name_hash32(n.as_bytes()) == hash);
                    named.map(|(_, source)| source)
                });
                self.census.note(Space::Name32, u64::from(hash), by, place);
            }
            match mount.entry_begins_archive(&mut *file, entry) {
                Ok(true) if self.options.top_only => self.census.nested += 1,
                Ok(true) if depth >= MAX_DEPTH => self.census.refused += 1,
                Ok(true) => match mount.open_nested(&mut *file, entry, gta6.keys) {
                    Ok(child) => {
                        self.census.nested += 1;
                        let child_label = match name {
                            Some(name) => format!("{label}/{name}"),
                            None => format!("{label}/hash_{:016x}", entry.name64),
                        };
                        self.gta6_mount(gta6, file, &child, &child_label, depth + 1);
                    }
                    Err(_) => self.census.refused += 1,
                },
                Ok(false) => self.gta6_body(gta6, file, mount, entry, &place()),
                Err(_) => self.census.refused += 1,
            }
        }
    }

    fn gta6_body(
        &mut self,
        gta6: &Gta6,
        file: &mut File,
        mount: &Gta6Mount,
        entry: &Gta6Entry,
        place: &str,
    ) {
        let limit = self.options.limit;
        let keys = gta6.keys;
        let unopened = |c: &or_io::container::rpf::Gta6Cipher| {
            c.cipher_type != CONTENT_CIPHER || !keys.has_content_keys()
        };
        if entry.cipher.as_ref().is_some_and(unopened) {
            self.census.no_key += 1;
            return;
        }
        if row_size(mount, entry).is_some_and(|s| s > limit) {
            self.census.over_limit += 1;
            return;
        }
        if !self.admit() {
            return;
        }
        match read_row(file, mount, entry, keys, limit) {
            Ok(bytes) => {
                let census = &mut self.census;
                tally_body(&bytes, place, Some(gta6.keys), gta6.natives, census)
            }
            Err(_) => self.census.unread += 1,
        }
    }

    /// Walk GTA V's `archives`, whose entry names are text, reading their metadata files.
    fn gta5(
        &mut self,
        root: &Path,
        archives: &[PathBuf],
        crypto: &Arc<dyn or_io::container::rpf::RpfCryptoProvider>,
    ) {
        for (i, path) in archives.iter().enumerate() {
            if self.cancel.is_set() {
                self.census.cancelled = true;
                return;
            }
            let label = relative_label(root, path);
            eprintln!("  [{}/{}] {label}", i + 1, archives.len());
            match RpfArchive::open_with(path, Arc::clone(crypto)) {
                Ok(archive) => {
                    self.census.archives += 1;
                    self.gta5_archive(&Arc::new(archive), &label, 0);
                }
                Err(e) => {
                    eprintln!("  {label}: {e}");
                    self.census.refused += 1;
                }
            }
        }
    }

    fn gta5_archive(&mut self, archive: &Arc<RpfArchive>, label: &str, depth: usize) {
        for node in archive.nodes.iter().filter(|n| !n.is_dir()) {
            if self.cancel.is_set() {
                self.census.cancelled = true;
                return;
            }
            self.census.text_names += 1;
            let path = node.path().replace('\\', "/");
            let extension = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
            if extension == "rpf" {
                if self.options.top_only {
                    self.census.nested += 1;
                } else if depth >= MAX_DEPTH {
                    self.census.refused += 1;
                } else {
                    match archive.open_nested(node.entry_index()) {
                        Ok(nested) => {
                            self.census.nested += 1;
                            let nested = Arc::new(nested);
                            self.gta5_archive(&nested, &format!("{label}/{path}"), depth + 1);
                        }
                        Err(_) => self.census.refused += 1,
                    }
                }
            } else if GTA5_METADATA.contains(&extension.as_str()) && self.admit() {
                match archive.read_entry(node.entry_index()) {
                    Ok(bytes) if bytes.len() as u64 > self.options.limit => {
                        self.census.over_limit += 1
                    }
                    Ok(bytes) => {
                        let place = format!("{label}/{path}");
                        tally_body(&bytes, &place, None, None, &mut self.census)
                    }
                    Err(_) => self.census.unread += 1,
                }
            }
        }
    }
}

/// Count the hashes a body holds: a metadata document's names and values, or a compiled script
/// program's native calls, each resolved where `natives` names its address; any other body holds
/// none this counts.
fn tally_body(
    bytes: &[u8],
    place: &str,
    keys: Option<&GtaViKeys>,
    natives: Option<&NativeAddresses>,
    census: &mut Census,
) {
    if let Some(format) = or_io::meta::detect_binary_format(bytes) {
        match super::meta::read_instance(bytes, ErrorMode::Lenient) {
            Ok((instance, _, _)) => {
                census.metadata += 1;
                let strings = match format {
                    BinaryFormat::Pso => string_hashes(bytes, keys),
                    _ => HashSet::new(),
                };
                tally_instance(&instance, place, &strings, census);
            }
            Err(_) => census.metadata_refused += 1,
        }
        return;
    }
    let Ok(program) = Compiled::open(bytes) else {
        return;
    };
    census.scripts += 1;
    // A decode that stops partway still counts the calls before it.
    for ins in program.instructions().map_while(Result::ok) {
        if ins.opcode == NATIVE {
            let o = ins.operands;
            let rva = u32::from_le_bytes([o[0], o[1], o[2], o[3]]);
            let named = natives.is_some_and(|table| table.name_of(rva).is_some());
            let by = named.then_some(LISTINGS);
            census.note(Space::Native, u64::from(rva), by, || {
                format!("{place}+{:#x}", ins.offset)
            });
        }
    }
}

/// The hashes of the strings a PSO document's string sections hold, by both 32-bit hash flavours;
/// none where the sections do not read or decipher.
fn string_hashes(bytes: &[u8], keys: Option<&GtaViKeys>) -> HashSet<u32> {
    let strings = document_strings(bytes, keys).unwrap_or_default();
    (strings.iter())
        .flat_map(|s| [string_hash(s.as_bytes()), literal_string_hash(s.as_bytes())])
        .collect()
}

/// Count a document's class, member and enum names and its hash values; a hash value the reader
/// left unnamed is resolved when one of the document's own `strings` hashes to it.
fn tally_instance(
    instance: &MetaInstance,
    place: &str,
    strings: &HashSet<u32>,
    census: &mut Census,
) {
    let mut tree = Tree {
        place,
        strings,
        census,
    };
    tree.instance(instance);
}

struct Tree<'a> {
    place: &'a str,
    strings: &'a HashSet<u32>,
    census: &'a mut Census,
}

impl Tree<'_> {
    fn instance(&mut self, instance: &MetaInstance) {
        self.name(Space::Class, &instance.type_name);
        for (field, value) in &instance.fields {
            self.name(Space::Member, field);
            self.value(value);
        }
    }

    fn value(&mut self, value: &Value) {
        let place = self.place;
        match value {
            Value::Hash(h) if !h.raw.is_empty() => {
                self.census
                    .note(Space::Text, u64::from(h.hash), Some(READER), String::new)
            }
            Value::Hash(h) if h.hash != 0 => {
                let by = self.strings.contains(&h.hash).then_some(DOCUMENT_STRINGS);
                self.census
                    .note(Space::Text, u64::from(h.hash), by, || place.to_string())
            }
            Value::Enum { type_name, variant } => {
                self.name(Space::Enum, type_name);
                self.name(Space::Enum, variant);
            }
            Value::Bitset { type_name, .. } => self.name(Space::Enum, type_name),
            Value::Struct(inner) | Value::Pointer(Some(inner)) => self.instance(inner),
            Value::Array(items) => items.iter().for_each(|v| self.value(v)),
            Value::Map(pairs) => {
                for (key, v) in pairs {
                    self.value(key);
                    self.value(v);
                }
            }
            _ => {}
        }
    }

    /// A name the reader gave: resolved unless it is spelled as a hash; a class spelled so is still
    /// resolved where the root type table names it.
    fn name(&mut self, space: Space, text: &str) {
        let place = self.place;
        match spelled_hash(text) {
            Some(hash) => {
                let named = space == Space::Class && root_types::name_for(hash).is_some();
                let by = named.then_some(ROOT_TYPES);
                self.census
                    .note(space, u64::from(hash), by, || place.to_string());
            }
            None if text.is_empty() => {}
            None => {
                let key = u64::from(literal_string_hash(text.as_bytes()));
                self.census.note(space, key, Some(READER), String::new);
            }
        }
    }
}

/// The table a run prints: what was walked and read, then a row per hash space.
fn report(census: &Census, game: Game, options: &Options) -> String {
    let game_label = match game {
        Game::Gta6 => "GTA VI",
        Game::Gta5 => "GTA V",
    };
    let mut out = format!(
        "hashes census of a {game_label} install: {} archives, {} nested, {} refused\n",
        census.archives, census.nested, census.refused
    );
    out.push_str(&format!(
        "  bodies         {} read (one in {}, up to {} bytes each), {} unread; {} metadata, {} \
         refused by a metadata reader, {} script programs; {} over the limit, {} sampled out, {} \
         under a cipher the keys do not open\n",
        census.bodies,
        options.every,
        options.limit,
        census.unread,
        census.metadata,
        census.metadata_refused,
        census.scripts,
        census.over_limit,
        census.sampled_out,
        census.no_key,
    ));
    if game == Game::Gta5 {
        out.push_str(&format!(
            "  entry names    {} entries, each named by text in its table\n",
            census.text_names
        ));
    }
    out.push_str(&format!(
        "  {:<18} {:>10} {:>10} {:>10}\n",
        "space", "seen", "resolved", "unresolved"
    ));
    for space in SPACES {
        if game == Game::Gta5 && matches!(space, Space::Name64 | Space::Name32 | Space::Column) {
            continue;
        }
        let count = census.count(space);
        let note = match (space, census.native_table) {
            (Space::Native, Some(declared)) => format!(
                "  (named by address through the {declared} natives the lifted listings declare)"
            ),
            (Space::Native, None) => {
                format!("  (no lifted listings: give --listings or {LISTINGS_ENV})")
            }
            _ => String::new(),
        };
        out.push_str(&format!(
            "  {:<18} {:>10} {:>10} {:>10}{note}\n",
            space.label(),
            count.seen(),
            count.resolved(),
            count.unresolved().len(),
        ));
    }
    out.push_str(&shares(census, game));
    if census.stopped {
        if let Some(note) = options.bodies.note(census.bodies, BODIES_FLAG) {
            out.push_str(&format!("  bodies: {note}\n"));
        }
    }
    if census.cancelled {
        out.push_str("  CANCELLED: these counts cover only what was walked before the stop.\n");
    }
    out
}

/// Per space seen, how many of its hashes each source it could be named by resolved, and their
/// share of the hashes seen, so a later run shows what a new source adds.
fn shares(census: &Census, game: Game) -> String {
    let mut out = String::from("  resolved by source, of the hashes each space saw\n");
    for space in SPACES {
        let count = census.count(space);
        let sources = census.sources(space);
        let entry_space = matches!(space, Space::Name64 | Space::Name32 | Space::Column);
        if count.seen() == 0 || sources.is_empty() || (game == Game::Gta5 && entry_space) {
            continue;
        }
        let share = |source: &&str| {
            let n = count.by.get(source).copied().unwrap_or(0);
            let percent = 100.0 * n as f64 / count.seen() as f64;
            format!("{source} {n} ({percent:.1}%)")
        };
        let row: Vec<String> = sources.iter().map(share).collect();
        out.push_str(&format!("  {:<18} {}\n", space.label(), row.join(", ")));
    }
    out
}

/// Write each unresolved hash as `space<TAB>hash<TAB>place` to `path`; how many were written.
fn write_unresolved(census: &Census, path: &Path) -> Result<usize, String> {
    let fail = |e: std::io::Error| format!("{}: {e}", path.display());
    let mut out = BufWriter::new(File::create(path).map_err(fail)?);
    let mut written = 0;
    for space in SPACES {
        for (hash, place) in census.count(space).unresolved() {
            match space.wide() {
                true => writeln!(out, "{}\t{hash:016x}\t{place}", space.label()),
                false => writeln!(out, "{}\t{hash:08x}\t{place}", space.label()),
            }
            .map_err(fail)?;
            written += 1;
        }
    }
    out.flush().map_err(fail)?;
    Ok(written)
}

/// The game of an install's archives: GTA VI where any opens with the RPF9 magic.
fn game_of(archives: &[PathBuf]) -> Option<Game> {
    if archives.is_empty() {
        return None;
    }
    let rpf9 = archives.iter().any(|path| {
        let mut head = [0u8; 4];
        File::open(path)
            .and_then(|mut f| f.read_exact(&mut head))
            .is_ok_and(|()| u32::from_le_bytes(head) == RPF9_MAGIC)
    });
    Some(if rpf9 { Game::Gta6 } else { Game::Gta5 })
}

/// Every `.rpf` under `root`, at any depth, sorted.
fn archives_under(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => walk(&path, out),
                Ok(kind)
                    if kind.is_file()
                        && path
                            .extension()
                            .is_some_and(|e| e.eq_ignore_ascii_case("rpf")) =>
                {
                    out.push(path)
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort();
    out
}

fn relative_label(root: &Path, path: &Path) -> String {
    (path.strip_prefix(root).unwrap_or(path))
        .to_string_lossy()
        .replace('\\', "/")
}

/// The natives the listings `--listings` (or [`LISTINGS_ENV`]) points at declare, cached in the configuration folder; `None` where neither names a folder.
fn load_natives(dir: Option<&Path>) -> Result<Option<NativeAddresses>, String> {
    let Some(folder) = dir
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os(LISTINGS_ENV).map(PathBuf::from))
    else {
        return Ok(None);
    };
    let config = or_title::config::config_dir();
    let table = NativeAddresses::load(&folder, config.as_deref()).map_err(|e| e.to_string())?;
    Ok(Some(table))
}

/// The sources of entry names, in the order a lookup tries them: the name tables and the
/// vocabulary of the recovered-names folder `--names` (or [`or_io::container::rpf::GTA6_NAMES_ENV`]) points at, then the
/// vocabulary the app keeps, then its cache of resolved names and the bundled words.
fn load_names(dir: Option<&Path>) -> Vec<Source> {
    let folder = dir
        .map(Path::to_path_buf)
        .or_else(or_io::container::rpf::default_names_dir);
    let mut names = Vec::new();
    if let Some(folder) = folder {
        // The files `NameResolver::load` reads as its shortcut, kept apart to be credited apart.
        let mut tables = NameResolver::new();
        if let Ok(text) = std::fs::read_to_string(folder.join("toc-entries-resolved.tsv")) {
            tables.add_toc_entries_tsv(&text);
        }
        if let Ok(text) = std::fs::read_to_string(folder.join("resolved-names.tsv")) {
            tables.add_hash_name_tsv(&text);
        }
        names.push(Source {
            label: NAMES_TABLES,
            resolver: tables,
        });
        names.push(Source {
            label: NAMES_VOCABULARY,
            resolver: NameResolver::load_vocabulary(&folder),
        });
    }
    if let Some(config) = or_title::config::config_dir() {
        let kept = Vocabulary::load(&vocabulary_file(&config));
        names.push(Source {
            label: KEPT_VOCABULARY,
            resolver: NameResolver::from_vocabulary(kept),
        });
    }
    // Every source at once, last: what it alone names is the local cache and the bundled words.
    names.push(Source {
        label: CACHE_AND_WORDS,
        resolver: crate::gta6_name_sources::load(dir),
    });
    names
}

#[cfg(test)]
#[path = "gta6_hashes_tests.rs"]
mod tests;
