//! The local names a GTA VI archive resolves with: the merged `resolved-names.tsv`, and the flat
//! cache in the user's configuration that the app and the CLI both read, each name beside its
//! 64-bit hash, keyed by the size and time of every file they were read from, so a run after the
//! first reads one file in one pass and reads no source.

use std::cell::Cell;
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use sha2::{Digest, Sha256};

use super::{parse_hash, read_vocabulary, NameResolver};
use crate::gta6::hash::name_hash64;
use crate::gta6::names::{vocabulary_file, Vocabulary};

/// The flat cache's file in the configuration folder, beside the keys cache.
const FILE_NAME: &str = "gtavi_names_cache.dat";
const MAGIC: &[u8; 8] = b"ORGTA6NM";
/// Raised whenever the layout or what is gathered changes, so an older cache is rebuilt.
const VERSION: u32 = 2;

thread_local! {
    /// How many times this thread read the sources of [`NameResolver::load_cached`].
    static GATHERED: Cell<usize> = const { Cell::new(0) };
}

fn valid(hash: u64, name: &str) -> bool {
    !name.is_empty()
        && !name.bytes().any(|b| b < b' ' || b == 127)
        && name_hash64(name.as_bytes()) == hash
}

pub(super) fn read_cache(directory: &Path) -> io::Result<BTreeMap<u64, String>> {
    let text = match fs::read_to_string(directory.join("resolved-names.tsv")) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(error),
    };
    let mut names = BTreeMap::new();
    let mut rejected = 0;
    for line in text.lines().filter(|line| !line.is_empty()) {
        let Some((hash, name)) = line.split_once('\t') else {
            rejected += 1;
            continue;
        };
        match parse_hash(hash).filter(|&hash| valid(hash, name)) {
            Some(hash) => match names.entry(hash) {
                std::collections::btree_map::Entry::Vacant(slot) => {
                    slot.insert(name.to_string());
                }
                // The first name stays; a second name for the same hash is one more rejected row.
                std::collections::btree_map::Entry::Occupied(slot) => {
                    if !slot.get().eq_ignore_ascii_case(name) {
                        rejected += 1;
                    }
                }
            },
            None => rejected += 1,
        }
    }
    if rejected != 0 {
        tracing::warn!(
            rejected,
            "ignored invalid rows in the local GTA VI names cache"
        );
    }
    Ok(names)
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
        let mut names = read_cache(directory)?;
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
            for (hash, name) in names {
                writeln!(output, "0x{hash:016x}\t{name}")?;
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
        let stamp = stamp(&sources(names_dir, config));
        let cache = config.map(|config| config.join(FILE_NAME));
        if let Some(cache) = &cache {
            match read(cache, &stamp) {
                Ok(pairs) => return resolver_of(pairs),
                Err(why) => {
                    tracing::info!(cache = %cache.display(), why, "rebuilding the GTA VI names cache")
                }
            }
        }
        let pairs: Vec<(u64, String)> = gather(names_dir, config).into_iter().collect();
        if let (Some(cache), false) = (&cache, pairs.is_empty()) {
            if let Err(error) = write(cache, &stamp, &pairs) {
                tracing::warn!(cache = %cache.display(), %error, "the GTA VI names cache was not kept");
            }
        }
        resolver_of(pairs)
    }

    /// How many times this thread has read the sources of [`Self::load_cached`], for tests.
    #[doc(hidden)]
    pub fn sources_read_on_this_thread() -> usize {
        GATHERED.with(Cell::get)
    }
}

fn resolver_of(pairs: Vec<(u64, String)>) -> NameResolver {
    let mut resolver = NameResolver::new();
    for (hash, name) in pairs {
        resolver.insert(hash, name);
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

/// The cache's bytes: the magic, `version`, the stamp's length and bytes, the count of pairs, the
/// SHA-256 of the pairs, then each pair as its hash, its name's length in a `u16` and the name, all
/// little-endian.
fn encode(version: u32, stamp: &[u8], pairs: &[(u64, String)]) -> Vec<u8> {
    let held = pairs
        .iter()
        .filter(|(_, n)| n.len() <= usize::from(u16::MAX));
    let names: usize = held.clone().map(|(_, n)| 10 + n.len()).sum();
    let mut out = Vec::with_capacity(24 + stamp.len() + names);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&(stamp.len() as u32).to_le_bytes());
    out.extend_from_slice(stamp);
    out.extend_from_slice(&(held.clone().count() as u64).to_le_bytes());
    let mut section = Vec::with_capacity(names);
    for (hash, name) in held {
        section.extend_from_slice(&hash.to_le_bytes());
        section.extend_from_slice(&(name.len() as u16).to_le_bytes());
        section.extend_from_slice(name.as_bytes());
    }
    out.extend_from_slice(&Sha256::digest(&section));
    out.extend_from_slice(&section);
    out
}

/// Keeps the cache at `file`, written whole beside it and then moved into place.
fn write(file: &Path, stamp: &[u8], pairs: &[(u64, String)]) -> io::Result<()> {
    if let Some(folder) = file.parent() {
        fs::create_dir_all(folder)?;
    }
    // The app and the CLI may keep it at once: each writes its own file beside it, and a reader
    // refuses one cut short.
    let beside = file.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&beside, encode(VERSION, stamp, pairs))?;
    fs::rename(&beside, file).inspect_err(|_| {
        let _ = fs::remove_file(&beside);
    })
}

/// The pairs the cache at `file` holds, where it is this version's, kept under `stamp` and whole;
/// else why it is not taken.
fn read(file: &Path, stamp: &[u8]) -> Result<Vec<(u64, String)>, &'static str> {
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
    // Each pair takes ten bytes at least, so a count past what the file holds is refused before it
    // sizes anything.
    let count = usize::try_from(u64::from_le_bytes(take_n(&bytes, at)?));
    let count = count.map_err(|_| "the cache's count is too large")?;
    if count > bytes.len() / 10 {
        return Err("the cache states more pairs than it holds");
    }
    let digest = take(&bytes, at, 32)?;
    if Sha256::digest(&bytes[*at..])[..] != *digest {
        return Err("the cache's pairs are not the ones it was kept with");
    }
    let mut pairs = Vec::with_capacity(count);
    for _ in 0..count {
        let hash = u64::from_le_bytes(take_n(&bytes, at)?);
        let len = usize::from(u16::from_le_bytes(take_n(&bytes, at)?));
        let name = std::str::from_utf8(take(&bytes, at, len)?).map_err(|_| "a name is not text")?;
        pairs.push((hash, name.to_owned()));
    }
    if *at != bytes.len() {
        return Err("the cache runs past its last pair");
    }
    Ok(pairs)
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
