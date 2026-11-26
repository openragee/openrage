//! The local names a GTA VI archive resolves with: the merged `resolved-names.tsv`, and the flat
//! cache in the user's configuration that the app and the CLI both read, one file per list of
//! sources and keyed by the size and time of each. The cache holds its names sorted by 64-bit hash
//! and is searched where it lies, so a run after the first reads one file, hashes no name, reads
//! no source and builds nothing per name.

use std::cell::Cell;
use std::cmp::Ordering;
use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use byteorder::{ByteOrder, LE};
use flate2::Crc;
use sha2::{Digest, Sha256};

use super::{parse_hash, NameResolver};
use crate::gta6::hash::{name_hash32, name_hash64};
use crate::gta6::names::{vocabulary_file, Vocabulary, MAX_NAME};
use crate::hash::literal_string_hash;

/// The cache of the configuration's own names, beside the keys cache; a list of sources with a
/// names folder keeps its own file beside it, named by the list's digest.
const FILE_NAME: &str = "gtavi_names_cache.dat";
const MAGIC: &[u8; 8] = b"ORGTA6NM";
/// Raised whenever the layout or what is gathered changes, so an older cache is rebuilt.
const VERSION: u32 = 4;
/// The bytes of a row: the 64-bit hash, the 32-bit hash, where its name ends among the names, the
/// form.
const ROW: usize = 8 + 4 + 8 + 1;
/// The first line of a vocabulary [`Vocabulary::save`] keeps.
const VOCABULARY_HEAD: &str = "openrage gta6 names 1";

thread_local! {
    /// How many times this thread read the sources of [`NameResolver::load_cached`].
    static GATHERED: Cell<usize> = const { Cell::new(0) };
    /// How many names this thread's [`NameResolver::load_cached`] hashed to build a resolver.
    static HASHED: Cell<usize> = const { Cell::new(0) };
    /// How many names this thread copied out of a kept cache into owned strings.
    static OWNED: Cell<usize> = const { Cell::new(0) };
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

/// Whether a vocabulary holds `name`, as [`Vocabulary::add`] judges it: 1 to [`MAX_NAME`]
/// printable bytes with no space.
fn holdable(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_NAME && name.bytes().all(|b| b.is_ascii_graphic())
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
        let (bytes, count) = gather(names_dir, config, &stamp);
        if let (Some(cache), true) = (&cache, count > 0) {
            if let Err(error) = write(cache, &bytes) {
                tracing::warn!(cache = %cache.display(), %error, "the GTA VI names cache was not kept");
            }
        }
        // The names gathered are read as a kept cache is, so a cold load and a warm one agree.
        match Kept::parse(bytes, &stamp) {
            Ok(kept) => Self::from_kept(kept),
            Err(why) => {
                tracing::warn!(why, "the GTA VI names gathered did not read back");
                Self::new()
            }
        }
    }

    fn from_kept(kept: Kept) -> Self {
        Self {
            kept: Arc::new(kept),
            ..Self::default()
        }
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

    /// How many names this thread has copied out of a kept cache into owned strings, for tests.
    #[doc(hidden)]
    pub fn names_owned_on_this_thread() -> usize {
        OWNED.with(Cell::get)
    }
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

/// The `hash<TAB>name` rows of a `resolved-names.tsv` table naming entries; a row that is not one
/// is skipped, as is a class or member name's eight-digit row, which names no entry.
fn table_rows(text: &str) -> impl Iterator<Item = (u64, &str)> {
    text.lines().filter_map(|line| {
        let mut fields = line.split('\t');
        let field = fields.next()?;
        let (hash, name) = (parse_hash(field)?, fields.next()?);
        let row32 = field.len() == 10
            && !valid(hash, name)
            && u32::try_from(hash).is_ok_and(|hash| valid32(hash, name));
        (!row32).then_some((hash, name))
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

/// The text of a vocabulary file, a byte that is not text read as U+FFFD, which names nothing; an
/// empty text where it is absent.
fn vocabulary_text(file: Option<PathBuf>) -> String {
    match file.map(fs::read) {
        Some(Ok(bytes)) => String::from_utf8(bytes)
            .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned()),
        _ => String::new(),
    }
}

/// The names of a vocabulary's `text`, as [`Vocabulary::load`] reads the kept format and, where
/// `plain`, as a list of one name a line with `//` comments.
fn vocabulary_rows<'a>(text: &'a str, plain: bool, rows: &mut Vec<Row<'a>>) {
    if text.lines().next() == Some(VOCABULARY_HEAD) {
        for line in text.lines().skip(1) {
            let Some((kind, name)) = line.split_once(' ') else {
                continue;
            };
            let kind_read = kind == "--"
                || (kind.len() == 2 && kind.bytes().all(|b| b.is_ascii_alphanumeric()));
            if kind_read && holdable(name) {
                rows.push(Row::spelled(name));
            }
        }
    } else if plain {
        let names = text.lines().map(str::trim);
        let names = names.filter(|line| !line.starts_with("//") && holdable(line));
        rows.extend(names.map(Row::spelled));
    }
}

/// The cache's bytes under `stamp` for every hash the sources name, with the first name a source
/// gives it: the folder's two name tables, the configuration folder's mined table, the folder's
/// vocabulary, then the harvested one; and how many names that is.
fn gather(names_dir: Option<&Path>, config: Option<&Path>, stamp: &[u8]) -> (Vec<u8>, usize) {
    GATHERED.with(|n| n.set(n.get() + 1));
    let table = |file: Option<PathBuf>| {
        file.and_then(|file| fs::read_to_string(file).ok())
            .unwrap_or_default()
    };
    let toc = table(names_dir.map(|dir| dir.join("toc-entries-resolved.tsv")));
    let listed = table(names_dir.map(|dir| dir.join("resolved-names.tsv")));
    let mined_rows = table(config.map(mined));
    let folder = vocabulary_text(names_dir.map(|dir| dir.join("vocabulary.txt")));
    let harvested = vocabulary_text(config.map(vocabulary_file));

    let mut rows = Vec::new();
    // archive, entry, name64, name32, name, source; a header row parses as no hash.
    for line in toc.lines() {
        let mut fields = line.split('\t').skip(2);
        if let (Some(hash), Some(name)) = (fields.next().and_then(parse_hash), fields.nth(1)) {
            rows.push(Row::stated(hash, name));
        }
    }
    let listed = table_rows(&listed).chain(table_rows(&mined_rows));
    rows.extend(listed.map(|(hash, name)| Row::stated(hash, name)));
    let spelled = rows.len();
    vocabulary_rows(&folder, true, &mut rows);
    if rows.len() == spelled && !folder.is_empty() {
        tracing::warn!("the names folder's vocabulary held no usable names");
    }
    vocabulary_rows(&harvested, false, &mut rows);
    rows.retain(|row| !row.name.is_empty());

    hash_rows(&mut rows);
    // A stable sort keeps the rows of one hash in the order of their sources, the first of which stays.
    rows.sort_by_key(|row| row.hash64);
    rows.dedup_by_key(|row| row.hash64);
    (encode(VERSION, stamp, &rows), rows.len())
}

/// Hashes every row, the rows split evenly over as many threads as the machine runs at once.
fn hash_rows(rows: &mut [Row]) {
    HASHED.with(|n| n.set(n.get() + rows.len()));
    let threads = std::thread::available_parallelism().map_or(1, usize::from);
    let chunk = rows.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        for part in rows.chunks_mut(chunk) {
            scope.spawn(move || part.iter_mut().for_each(Row::hash));
        }
    });
}

/// A name as the cache keeps it.
struct Row<'a> {
    hash64: u64,
    /// The 32-bit hash of the name, where `held`; else zero.
    hash32: u32,
    /// Whether the vocabulary holds the name: one it takes, whose 64-bit hash is `hash64`.
    held: bool,
    /// Whether the name was read from a vocabulary, which keeps it lower-cased and hashes it.
    spelled: bool,
    name: &'a str,
}

impl<'a> Row<'a> {
    /// A name a table states under `hash64`, as it was written.
    fn stated(hash64: u64, name: &'a str) -> Self {
        Self {
            hash64,
            hash32: 0,
            held: false,
            spelled: false,
            name,
        }
    }

    fn spelled(name: &'a str) -> Self {
        Self {
            spelled: true,
            ..Self::stated(0, name)
        }
    }

    /// Fills the hashes; `name_hash32` folds case, so a spelled name is hashed as written.
    fn hash(&mut self) {
        let hash = name_hash64(self.name.as_bytes());
        if self.spelled {
            self.hash64 = hash;
        }
        self.held = hash == self.hash64 && holdable(self.name);
        self.hash32 = if self.held {
            name_hash32(self.name.as_bytes())
        } else {
            0
        };
    }
}

fn checksum(bytes: &[u8]) -> u32 {
    let mut crc = Crc::new();
    crc.update(bytes);
    crc.sum()
}

/// The cache's bytes: the magic, `version`, the stamp's length and bytes, the count of rows, the
/// CRC-32 of what follows it, each row as its 64-bit hash, its 32-bit hash, where its name ends
/// among the names in a `u64` and whether it is held, then the names end to end, all
/// little-endian. `rows` rise by 64-bit hash, or the cache does not read back.
fn encode(version: u32, stamp: &[u8], rows: &[Row]) -> Vec<u8> {
    let names: usize = rows.iter().map(|row| row.name.len()).sum();
    let mut out = Vec::with_capacity(28 + stamp.len() + rows.len() * ROW + names);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&(stamp.len() as u32).to_le_bytes());
    out.extend_from_slice(stamp);
    out.extend_from_slice(&(rows.len() as u64).to_le_bytes());
    let crc_at = out.len();
    out.extend_from_slice(&[0; 4]);
    let mut end = 0u64;
    for row in rows {
        end += row.name.len() as u64;
        out.extend_from_slice(&row.hash64.to_le_bytes());
        out.extend_from_slice(&row.hash32.to_le_bytes());
        out.extend_from_slice(&end.to_le_bytes());
        out.push(u8::from(row.held));
    }
    for row in rows {
        let at = out.len();
        out.extend_from_slice(row.name.as_bytes());
        if row.spelled {
            out[at..].make_ascii_lowercase();
        }
    }
    let crc = checksum(&out[crc_at + 4..]);
    out[crc_at..crc_at + 4].copy_from_slice(&crc.to_le_bytes());
    out
}

/// Keeps `bytes` as the cache at `file`, written whole beside it and then moved into place.
fn write(file: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(folder) = file.parent() {
        fs::create_dir_all(folder)?;
    }
    // The app and the CLI may keep it at once: each writes its own file beside it, and a reader
    // refuses one cut short.
    let beside = file.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&beside, bytes)?;
    fs::rename(&beside, file).inspect_err(|_| {
        let _ = fs::remove_file(&beside);
    })
}

/// The resolver over the cache at `file`, where it is this version's, kept under `stamp` and
/// whole; else why it is not taken.
fn read(file: &Path, stamp: &[u8]) -> Result<NameResolver, &'static str> {
    let bytes = fs::read(file).map_err(|_| "no cache is kept")?;
    Kept::parse(bytes, stamp).map(NameResolver::from_kept)
}

/// A kept cache, searched where it lies: its rows sorted by 64-bit hash, then its names end to end.
#[derive(Default)]
pub(super) struct Kept {
    bytes: Vec<u8>,
    /// Where the rows start in `bytes`.
    rows: usize,
    count: usize,
    /// Where the names start in `bytes`.
    names: usize,
}

impl fmt::Debug for Kept {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Kept").field("count", &self.count).finish()
    }
}

impl Kept {
    /// The cache in `bytes`, where it is this version's, kept under `stamp` and whole; else why it
    /// is not taken.
    // The CRC-32 over the rows and names is the integrity check: a file cut short fails the count
    // or the end, and CRC-32 catches every flipped bit and every damaged run of up to 32 bits, and
    // other damage but for one chance in 2^32. It guards against accident only: whoever can edit
    // the cache can edit the sources it was built from, which sit beside it.
    fn parse(bytes: Vec<u8>, stamp: &[u8]) -> Result<Self, &'static str> {
        let at = &mut 0usize;
        if take(&bytes, at, 8)? != MAGIC {
            return Err("the file is not a names cache");
        }
        if LE::read_u32(take(&bytes, at, 4)?) != VERSION {
            return Err("the cache is of another version");
        }
        let kept = LE::read_u32(take(&bytes, at, 4)?) as usize;
        if take(&bytes, at, kept)? != stamp {
            return Err("a source changed since the cache was kept");
        }
        // A count past what the file can hold is refused before it sizes anything.
        let count = usize::try_from(LE::read_u64(take(&bytes, at, 8)?));
        let count = count.map_err(|_| "the cache's count is too large")?;
        if count > bytes.len() / ROW {
            return Err("the cache states more rows than it holds");
        }
        let crc = LE::read_u32(take(&bytes, at, 4)?);
        if checksum(&bytes[*at..]) != crc {
            return Err("the cache's rows are not the ones it was kept with");
        }
        let rows = *at;
        take(&bytes, at, count * ROW)?;
        let kept = Self {
            rows,
            count,
            names: *at,
            bytes,
        };
        kept.check()?;
        Ok(kept)
    }

    /// Whether the rows rise by 64-bit hash, are each of a known form, and each name is text that
    /// ends at or past the one before, the last where the file does.
    fn check(&self) -> Result<(), &'static str> {
        let names = &self.bytes[self.names..];
        let mut start = 0;
        for i in 0..self.count {
            if i > 0 && self.hash64(i - 1) >= self.hash64(i) {
                return Err("the cache's rows are not in order");
            }
            if self.row(i)[20] > 1 {
                return Err("a row is of no known form");
            }
            let end = self.end(i);
            // A range that ends before it starts gets nothing, as one past the names does.
            let name = names.get(start..end).ok_or("a name runs past the names")?;
            std::str::from_utf8(name).map_err(|_| "a name is not text")?;
            start = end;
        }
        if start != names.len() {
            return Err("the cache runs past its last name");
        }
        Ok(())
    }

    fn row(&self, i: usize) -> &[u8] {
        &self.bytes[self.rows + i * ROW..][..ROW]
    }

    fn hash64(&self, i: usize) -> u64 {
        LE::read_u64(self.row(i))
    }

    fn hash32(&self, i: usize) -> u32 {
        LE::read_u32(&self.row(i)[8..])
    }

    fn end(&self, i: usize) -> usize {
        usize::try_from(LE::read_u64(&self.row(i)[12..])).unwrap_or(usize::MAX)
    }

    fn held(&self, i: usize) -> bool {
        self.row(i)[20] == 1
    }

    /// The name of row `i`, as [`Self::check`] found it.
    fn name(&self, i: usize) -> &str {
        let start = if i == 0 { 0 } else { self.end(i - 1) };
        let name = &self.bytes[self.names + start..self.names + self.end(i)];
        std::str::from_utf8(name).unwrap_or_default()
    }

    /// The row of `hash`, by a binary search of the rows.
    fn find(&self, hash: u64) -> Option<usize> {
        let (mut low, mut high) = (0, self.count);
        while low < high {
            let mid = low + (high - low) / 2;
            match self.hash64(mid).cmp(&hash) {
                Ordering::Less => low = mid + 1,
                Ordering::Greater => high = mid,
                Ordering::Equal => return Some(mid),
            }
        }
        None
    }

    pub(super) fn len(&self) -> usize {
        self.count
    }

    pub(super) fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The name of `hash` as it was named, and whether the vocabulary holds it.
    pub(super) fn get(&self, hash: u64) -> Option<(&str, bool)> {
        self.find(hash).map(|i| (self.name(i), self.held(i)))
    }

    /// The held names, lower-cased, under the hashes they were kept with; it copies every name.
    pub(super) fn vocabulary(&self) -> Vocabulary {
        let held: Vec<_> = (0..self.count)
            .filter(|&i| self.held(i))
            .map(|i| {
                let text = self.name(i).to_ascii_lowercase().into_boxed_str();
                (self.hash64(i), self.hash32(i), text)
            })
            .collect();
        OWNED.with(|n| n.set(n.get() + held.len()));
        let mut vocabulary = Vocabulary::default();
        vocabulary.extend_hashed(held);
        vocabulary
    }
}

/// The `n` bytes of `bytes` at `at`, which moves past them.
fn take<'a>(bytes: &'a [u8], at: &mut usize, n: usize) -> Result<&'a [u8], &'static str> {
    let end = (at.checked_add(n)).filter(|end| *end <= bytes.len());
    let taken = &bytes[*at..end.ok_or("the cache is cut short")?];
    *at += n;
    Ok(taken)
}

#[cfg(test)]
#[path = "gta6_name_cache_tests.rs"]
mod tests;
