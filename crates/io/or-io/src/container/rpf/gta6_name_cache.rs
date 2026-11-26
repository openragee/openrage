//! The local names a GTA VI archive resolves with: the merged `resolved-names.tsv`, and the flat
//! cache in the user's configuration that the app and the CLI both read, each name beside its
//! 64-bit and 32-bit hashes, one file per list of sources and keyed by the size and time of each,
//! so a run after the first reads one file in one pass, hashes no name and reads no source.

use std::cell::Cell;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use flate2::Crc;
use sha2::{Digest, Sha256};

use super::{parse_hash, read_vocabulary, NameResolver};
use crate::gta6::hash::{name_hash32, name_hash64};
use crate::gta6::names::{vocabulary_file, Vocabulary};
use crate::hash::literal_string_hash;

/// The cache of the configuration's own names, beside the keys cache; a list of sources with a
/// names folder keeps its own file beside it, named by the list's digest.
const FILE_NAME: &str = "gtavi_names_cache.dat";
const MAGIC: &[u8; 8] = b"ORGTA6NM";
/// Raised whenever the layout or what is gathered changes, so an older cache is rebuilt.
const VERSION: u32 = 3;
/// The bytes of a row before its name: the 64-bit hash, the 32-bit hash, the form, the length.
const ROW_HEAD: usize = 8 + 4 + 1 + 2;

thread_local! {
    /// How many times this thread read the sources of [`NameResolver::load_cached`].
    static GATHERED: Cell<usize> = const { Cell::new(0) };
    /// How many names this thread's [`NameResolver::load_cached`] hashed to build a resolver.
    static HASHED: Cell<usize> = const { Cell::new(0) };
}

fn printable(name: &str) -> bool {
    !name.is_empty() && !name.bytes().any(|b| b < b' ' || b == 127)
}

fn valid(hash: u64, name: &str) -> bool {
    printable(name) && name_hash64(name.as_bytes()) == hash
}

/// Whether `name` has the 32-bit `hash`, folded or as written, as the miner's class and member
/// rows state it.
fn valid32(hash: u32, name: &str) -> bool {
    let bytes = name.as_bytes();
    printable(name) && (name_hash32(bytes) == hash || literal_string_hash(bytes) == hash)
}

/// The rows of a `resolved-names.tsv`: each 64-bit hash's name, and each 32-bit hash's name, which
/// the miner writes in eight digits.
#[derive(Default)]
struct Table {
    names64: BTreeMap<u64, String>,
    names32: BTreeMap<u32, String>,
}

pub(super) fn read_cache(directory: &Path) -> io::Result<BTreeMap<u64, String>> {
    read_table(directory).map(|table| table.names64)
}

fn read_table(directory: &Path) -> io::Result<Table> {
    let text = match fs::read_to_string(directory.join("resolved-names.tsv")) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Table::default()),
        Err(error) => return Err(error),
    };
    let mut table = Table::default();
    let mut rejected = 0;
    for line in text.lines().filter(|line| !line.is_empty()) {
        let Some((field, name)) = line.split_once('\t') else {
            rejected += 1;
            continue;
        };
        let kept = match parse_hash(field) {
            Some(hash) if valid(hash, name) => keep(&mut table.names64, hash, name),
            Some(hash) => match u32::try_from(hash) {
                Ok(hash) if field.len() == 10 && valid32(hash, name) => {
                    keep(&mut table.names32, hash, name)
                }
                _ => false,
            },
            None => false,
        };
        rejected += usize::from(!kept);
    }
    if rejected != 0 {
        tracing::warn!(
            rejected,
            "ignored invalid rows in the local GTA VI names cache"
        );
    }
    Ok(table)
}

/// Holds `name` for `hash` where none is held; whether the row is kept, as a second spelling of
/// the held name is. The first name stays.
fn keep<K: Ord>(names: &mut BTreeMap<K, String>, hash: K, name: &str) -> bool {
    match names.entry(hash) {
        Entry::Vacant(slot) => {
            slot.insert(name.to_owned());
            true
        }
        Entry::Occupied(slot) => slot.get().eq_ignore_ascii_case(name),
    }
}

impl NameResolver {
    /// Atomically merges verified names into the local TSV without overwriting a conflicting name.
    pub fn cache_names<'a>(
        directory: &Path,
        found: impl IntoIterator<Item = (u64, &'a str)>,
    ) -> io::Result<usize> {
        let mut additions = BTreeMap::new();
        for (hash, name) in found {
            if !valid(hash, name) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "a cached name does not match its hash",
                ));
            }
            if additions
                .insert(hash, name)
                .is_some_and(|old| !old.eq_ignore_ascii_case(name))
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "conflicting cached names",
                ));
            }
        }
        if additions.is_empty() {
            return Ok(0);
        }
        fs::create_dir_all(directory)?;
        // The advisory lock serializes readers that merge then replace, across threads and processes.
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join(".resolved-names.lock"))?;
        lock.lock()?;
        let mut table = read_table(directory)?;
        let names = &mut table.names64;
        let before = names.len();
        for (hash, name) in additions {
            if let Some(old) = names.get(&hash) {
                if !old.eq_ignore_ascii_case(name) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "conflicting cached names",
                    ));
                }
            } else {
                names.insert(hash, name.to_string());
            }
        }
        let added = names.len() - before;
        if added == 0 {
            return Ok(0);
        }
        let destination = directory.join("resolved-names.tsv");
        let temporary = directory.join(".resolved-names.tsv.tmp");
        let result = (|| {
            let mut output = io::BufWriter::new(fs::File::create(&temporary)?);
            for (hash, name) in &table.names64 {
                writeln!(output, "0x{hash:016x}\t{name}")?;
            }
            for (hash, name) in &table.names32 {
                writeln!(output, "0x{hash:08x}\t{name}")?;
            }
            output.flush()?;
            output.get_ref().sync_all()?;
            drop(output);
            fs::rename(&temporary, &destination)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result?;
        Ok(added)
    }

    /// The names of `names_dir` and the harvest under the configuration folder `config`, from the cache kept there while its key holds.
    pub fn load_cached(names_dir: Option<&Path>, config: Option<&Path>) -> Self {
        let sources = sources(names_dir, config);
        let stamp = stamp(&sources);
        let cache = config.map(|config| cache_file(config, names_dir, &sources));
        if let Some(cache) = &cache {
            match read(cache, &stamp) {
                Ok(resolver) => return resolver,
                Err(why) => {
                    tracing::info!(cache = %cache.display(), why, "rebuilding the GTA VI names cache")
                }
            }
        }
        let pairs: Vec<(u64, String)> = gather(names_dir, config).into_iter().collect();
        let resolver = resolver_of(&pairs);
        if let (Some(cache), false) = (&cache, pairs.is_empty()) {
            let rows = rows_of(&pairs, resolver.vocabulary());
            if let Err(error) = write(cache, &stamp, &rows) {
                tracing::warn!(cache = %cache.display(), %error, "the GTA VI names cache was not kept");
            }
        }
        resolver
    }

    /// How many times this thread has read the sources of [`Self::load_cached`], for tests.
    #[doc(hidden)]
    pub fn sources_read_on_this_thread() -> usize {
        GATHERED.with(Cell::get)
    }

    /// How many names this thread's [`Self::load_cached`] has hashed, for tests.
    #[doc(hidden)]
    pub fn names_hashed_on_this_thread() -> usize {
        HASHED.with(Cell::get)
    }
}

fn resolver_of(pairs: &[(u64, String)]) -> NameResolver {
    HASHED.with(|n| n.set(n.get() + pairs.len()));
    let mut resolver = NameResolver::new();
    for (hash, name) in pairs {
        resolver.insert(*hash, name.as_str());
    }
    resolver
}

/// The files the names are read from, in the order a hash is first named from them.
fn sources(names_dir: Option<&Path>, config: Option<&Path>) -> Vec<PathBuf> {
    let dir = names_dir.into_iter().flat_map(|dir| {
        [
            "toc-entries-resolved.tsv",
            "resolved-names.tsv",
            "vocabulary.txt",
        ]
        .map(|f| dir.join(f))
    });
    let config = config
        .into_iter()
        .flat_map(|config| [mined(config), vocabulary_file(config)]);
    dir.chain(config).collect()
}

/// Where the cache of `sources` is kept under `config`: the configuration's own names alone in
/// [`FILE_NAME`], and a list with a names folder in a file named by the list's digest, so a run
/// with another folder keeps its own cache and replaces no other.
// ponytail: a folder used once leaves its cache behind; prune by age if the folder fills.
fn cache_file(config: &Path, names_dir: Option<&Path>, sources: &[PathBuf]) -> PathBuf {
    if names_dir.is_none() {
        return config.join(FILE_NAME);
    }
    let mut list = Sha256::new();
    for source in sources {
        list.update(source.as_os_str().as_encoded_bytes());
        list.update([0]);
    }
    let digest: String = list.finalize()[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    config.join(format!("gtavi_names_cache.{digest}.dat"))
}

/// The table `hashes mine` writes under the configuration folder, beside the harvest.
fn mined(config: &Path) -> PathBuf {
    vocabulary_file(config).with_file_name("resolved-names.tsv")
}

/// The `hash<TAB>name` rows of a `resolved-names.tsv` table; a row that is not one is skipped.
fn table_rows(text: &str) -> impl Iterator<Item = (u64, &str)> {
    text.lines().filter_map(|line| {
        let mut fields = line.split('\t');
        Some((fields.next().and_then(parse_hash)?, fields.next()?))
    })
}

/// The key a cache is kept under: each source's path, then its size and modification time in
/// nanoseconds, or that it is absent.
fn stamp(sources: &[PathBuf]) -> Vec<u8> {
    let mut stamp = String::new();
    for source in sources {
        let meta = fs::metadata(source).ok();
        let time = (meta.as_ref())
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok());
        stamp += &match (meta, time) {
            (Some(meta), Some(time)) => format!(
                "{}\t{}\t{}\n",
                source.display(),
                meta.len(),
                time.as_nanos()
            ),
            _ => format!("{}\tabsent\n", source.display()),
        };
    }
    stamp.into_bytes()
}

/// Every hash the sources name, with the first name a source gives it: the folder's two name
/// tables, the configuration folder's mined table, the folder's vocabulary, then the harvested one.
fn gather(names_dir: Option<&Path>, config: Option<&Path>) -> HashMap<u64, String> {
    GATHERED.with(|n| n.set(n.get() + 1));
    let mut pairs = HashMap::new();
    let mut add = |hash: u64, name: &str| {
        if !name.is_empty() {
            pairs.entry(hash).or_insert_with(|| name.to_owned());
        }
    };
    if let Some(dir) = names_dir {
        let table = |file: &str| fs::read_to_string(dir.join(file)).unwrap_or_default();
        // archive, entry, name64, name32, name, source; a header row parses as no hash.
        for line in table("toc-entries-resolved.tsv").lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            if let [_, _, hash, _, path, ..] = fields.as_slice() {
                if let Some(hash) = parse_hash(hash) {
                    add(hash, path);
                }
            }
        }
        for (hash, path) in table_rows(&table("resolved-names.tsv")) {
            add(hash, path);
        }
    }
    if let Some(config) = config {
        for (hash, path) in table_rows(&fs::read_to_string(mined(config)).unwrap_or_default()) {
            add(hash, path);
        }
    }
    let folder = names_dir.map(|dir| read_vocabulary(&dir.join("vocabulary.txt")));
    let harvested = config.map(|config| Vocabulary::load(&vocabulary_file(config)));
    for vocabulary in folder.iter().chain(harvested.iter()) {
        for text in vocabulary.names() {
            add(name_hash64(text.as_bytes()), text);
        }
    }
    pairs
}

/// A name as the cache keeps it.
struct Row<'a> {
    hash64: u64,
    /// The 32-bit hash of the name lower-cased, where `held`; else zero.
    hash32: u32,
    /// Whether the vocabulary holds the name lower-cased; else only the shortcut table holds it.
    held: bool,
    /// As it was named.
    name: &'a str,
}

/// What the cache keeps of each pair the resolver over `vocabulary` was built from.
fn rows_of<'a>(pairs: &'a [(u64, String)], vocabulary: &Vocabulary) -> Vec<Row<'a>> {
    let row = |(hash64, name): &'a (u64, String)| {
        let held = vocabulary.name64(*hash64);
        Row {
            hash64: *hash64,
            hash32: held.map_or(0, |text| name_hash32(text.as_bytes())),
            held: held.is_some(),
            name: name.as_str(),
        }
    };
    pairs.iter().map(row).collect()
}

fn checksum(bytes: &[u8]) -> u32 {
    let mut crc = Crc::new();
    crc.update(bytes);
    crc.sum()
}

/// The cache's bytes: the magic, `version`, the stamp's length and bytes, the count of rows, the
/// CRC-32 of the rows, then each row as its 64-bit hash, its 32-bit hash, whether it is held, its
/// name's length in a `u16` and the name, all little-endian.
fn encode(version: u32, stamp: &[u8], rows: &[Row]) -> Vec<u8> {
    let kept = || {
        rows.iter()
            .filter(|row| row.name.len() <= usize::from(u16::MAX))
    };
    let mut section = Vec::with_capacity(kept().map(|row| ROW_HEAD + row.name.len()).sum());
    for row in kept() {
        section.extend_from_slice(&row.hash64.to_le_bytes());
        section.extend_from_slice(&row.hash32.to_le_bytes());
        section.push(u8::from(row.held));
        section.extend_from_slice(&(row.name.len() as u16).to_le_bytes());
        section.extend_from_slice(row.name.as_bytes());
    }
    let mut out = Vec::with_capacity(28 + stamp.len() + section.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&(stamp.len() as u32).to_le_bytes());
    out.extend_from_slice(stamp);
    out.extend_from_slice(&(kept().count() as u64).to_le_bytes());
    out.extend_from_slice(&checksum(&section).to_le_bytes());
    out.extend_from_slice(&section);
    out
}

/// Keeps the cache at `file`, written whole beside it and then moved into place.
fn write(file: &Path, stamp: &[u8], rows: &[Row]) -> io::Result<()> {
    if let Some(folder) = file.parent() {
        fs::create_dir_all(folder)?;
    }
    // The app and the CLI may keep it at once: each writes its own file beside it, and a reader
    // refuses one cut short.
    let beside = file.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&beside, encode(VERSION, stamp, rows))?;
    fs::rename(&beside, file).inspect_err(|_| {
        let _ = fs::remove_file(&beside);
    })
}

/// The resolver the cache at `file` holds, built with the hashes it states, where it is this
/// version's, kept under `stamp` and whole; else why it is not taken.
// The CRC-32 over every row is the integrity check: a file cut short fails the count or the end,
// and CRC-32 catches every flipped bit and every damaged run of up to 32 bits, and other damage
// but for one chance in 2^32. It guards against accident only: whoever can edit the cache can
// edit the sources it was built from, which sit beside it.
fn read(file: &Path, stamp: &[u8]) -> Result<NameResolver, &'static str> {
    let bytes = fs::read(file).map_err(|_| "no cache is kept")?;
    let at = &mut 0usize;
    if take(&bytes, at, 8)? != MAGIC {
        return Err("the file is not a names cache");
    }
    if u32::from_le_bytes(take_n(&bytes, at)?) != VERSION {
        return Err("the cache is of another version");
    }
    let kept = u32::from_le_bytes(take_n(&bytes, at)?) as usize;
    if take(&bytes, at, kept)? != stamp {
        return Err("a source changed since the cache was kept");
    }
    // A count past what the file can hold is refused before it sizes anything.
    let count = usize::try_from(u64::from_le_bytes(take_n(&bytes, at)?));
    let count = count.map_err(|_| "the cache's count is too large")?;
    if count > bytes.len() / ROW_HEAD {
        return Err("the cache states more rows than it holds");
    }
    let crc = u32::from_le_bytes(take_n(&bytes, at)?);
    if checksum(&bytes[*at..]) != crc {
        return Err("the cache's rows are not the ones it was kept with");
    }
    let mut held = Vec::with_capacity(count);
    let mut shortcut = HashMap::new();
    for _ in 0..count {
        let hash64 = u64::from_le_bytes(take_n(&bytes, at)?);
        let hash32 = u32::from_le_bytes(take_n(&bytes, at)?);
        let [form] = take_n::<1>(&bytes, at)?;
        let len = usize::from(u16::from_le_bytes(take_n(&bytes, at)?));
        let name = std::str::from_utf8(take(&bytes, at, len)?).map_err(|_| "a name is not text")?;
        match form {
            0 => {
                shortcut.insert(hash64, name.to_owned());
            }
            // A name the vocabulary holds as written is not repeated in the shortcut table.
            1 => {
                if name.bytes().any(|b| b.is_ascii_uppercase()) {
                    shortcut.insert(hash64, name.to_owned());
                }
                held.push((hash64, hash32, name.to_ascii_lowercase().into_boxed_str()));
            }
            _ => return Err("a row is of no known form"),
        }
    }
    if *at != bytes.len() {
        return Err("the cache runs past its last row");
    }
    let mut vocabulary = Vocabulary::default();
    vocabulary.extend_hashed(held);
    Ok(NameResolver {
        vocabulary,
        shortcut,
    })
}

/// The `n` bytes of `bytes` at `at`, which moves past them.
fn take<'a>(bytes: &'a [u8], at: &mut usize, n: usize) -> Result<&'a [u8], &'static str> {
    let end = (at.checked_add(n)).filter(|end| *end <= bytes.len());
    let taken = &bytes[*at..end.ok_or("the cache is cut short")?];
    *at += n;
    Ok(taken)
}

/// [`take`] of `N` bytes, as an array.
fn take_n<const N: usize>(bytes: &[u8], at: &mut usize) -> Result<[u8; N], &'static str> {
    let taken = take(bytes, at, N)?;
    taken.try_into().map_err(|_| "the cache is cut short")
}

#[cfg(test)]
#[path = "gta6_name_cache_tests.rs"]
mod tests;
