//! The names the GTA VI archive tree resolves hashes with, kept flat in the user's configuration:
//! each name beside its 64-bit hash, keyed by the size and time of every file they were read from,
//! so a run after the first reads one file in one pass and hashes nothing.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use or_io::container::rpf::NameResolver;
use or_io::gta6::hash::name_hash64;
use or_io::gta6::names::{vocabulary_file, Vocabulary};

/// The cache's file in the configuration folder, beside the keys cache.
pub(crate) const FILE_NAME: &str = "gtavi_names_cache.dat";
const MAGIC: &[u8; 8] = b"ORGTA6NM";
/// Raised whenever the layout or what is gathered changes, so an older cache is rebuilt.
const VERSION: u32 = 1;

/// The names of `names_dir` and `config`'s harvest, from the cache in `config` while its key holds.
pub(crate) fn load(names_dir: Option<&Path>, config: Option<&Path>) -> NameResolver {
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
    dir.chain(config.map(vocabulary_file)).collect()
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
/// tables, its vocabulary, then the harvested one, as the resolver looked them up before.
fn gather(names_dir: Option<&Path>, config: Option<&Path>) -> HashMap<u64, String> {
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
                if let Some(hash) = hash_of(hash) {
                    add(hash, path);
                }
            }
        }
        for line in table("resolved-names.tsv").lines() {
            let mut fields = line.split('\t');
            if let (Some(hash), Some(path)) = (fields.next().and_then(hash_of), fields.next()) {
                add(hash, path);
            }
        }
    }
    let folder = names_dir.map(|dir| {
        let file = dir.join("vocabulary.txt");
        let kept = Vocabulary::load(&file);
        if !kept.is_empty() {
            return kept;
        }
        // The plain list of one name a line a recovered-names folder holds.
        let mut plain = Vocabulary::default();
        plain.add_lines(&fs::read_to_string(&file).unwrap_or_default());
        plain
    });
    let harvested = config.map(|config| Vocabulary::load(&vocabulary_file(config)));
    for vocabulary in folder.iter().chain(harvested.iter()) {
        for text in vocabulary.names() {
            add(name_hash64(text.as_bytes()), text);
        }
    }
    pairs
}

/// A `0x…` hash of 1 to 16 hex digits.
fn hash_of(field: &str) -> Option<u64> {
    let digits = (field.strip_prefix("0x")).or_else(|| field.strip_prefix("0X"))?;
    let hex = matches!(digits.len(), 1..=16) && digits.bytes().all(|b| b.is_ascii_hexdigit());
    hex.then(|| u64::from_str_radix(digits, 16).ok()).flatten()
}

/// The cache's bytes: the magic, `version`, the stamp's length and bytes, the count of pairs, then
/// each pair as its hash, its name's length in a `u16` and the name, all little-endian.
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
    for (hash, name) in held {
        out.extend_from_slice(&hash.to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
    }
    out
}

/// Keeps the cache at `file`, written whole beside it and then moved into place.
fn write(file: &Path, stamp: &[u8], pairs: &[(u64, String)]) -> std::io::Result<()> {
    if let Some(folder) = file.parent() {
        fs::create_dir_all(folder)?;
    }
    let beside = file.with_extension("tmp");
    fs::write(&beside, encode(VERSION, stamp, pairs))?;
    fs::rename(beside, file)
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
mod tests {
    use std::collections::BTreeMap;
    use std::time::Instant;

    use super::*;

    const NAMES: [&str; 3] = ["x/one.ydr", "x/two.ytd", "y/three.ybn"];

    /// A names folder of synthetic paths, one a line, and an empty configuration folder.
    fn folders() -> (tempfile::TempDir, tempfile::TempDir) {
        let names = tempfile::tempdir().expect("a names folder");
        fs::write(names.path().join("vocabulary.txt"), NAMES.join("\n")).expect("write names");
        (names, tempfile::tempdir().expect("a config folder"))
    }

    fn names_in(resolver: &NameResolver, names: &[&str]) -> Vec<bool> {
        let hash = |name: &str| name_hash64(name.as_bytes());
        names
            .iter()
            .map(|n| resolver.lookup(hash(n)).is_some())
            .collect()
    }

    fn sorted(pairs: impl IntoIterator<Item = (u64, String)>) -> BTreeMap<u64, String> {
        pairs.into_iter().collect()
    }

    #[test]
    fn a_kept_cache_reads_back_what_was_gathered() {
        let (names, config) = folders();
        let resolver = load(Some(names.path()), Some(config.path()));
        assert_eq!(names_in(&resolver, &NAMES), [true; 3]);

        let stamp = stamp(&sources(Some(names.path()), Some(config.path())));
        let kept = read(&config.path().join(FILE_NAME), &stamp).expect("the cache reads back");
        let gathered = gather(Some(names.path()), Some(config.path()));
        assert_eq!(gathered.len(), NAMES.len());
        assert_eq!(sorted(kept), sorted(gathered));
    }

    #[test]
    fn a_cache_whose_key_holds_is_read_and_the_sources_are_not() {
        let (names, config) = folders();
        let stamp = stamp(&sources(Some(names.path()), Some(config.path())));
        let only = vec![(name_hash64(b"z/cached"), "z/cached".to_owned())];
        write(&config.path().join(FILE_NAME), &stamp, &only).expect("keep a cache");

        let resolver = load(Some(names.path()), Some(config.path()));
        assert_eq!(names_in(&resolver, &["z/cached", NAMES[0]]), [true, false]);
    }

    #[test]
    fn a_source_whose_size_changes_rebuilds_the_cache() {
        let (names, config) = folders();
        load(Some(names.path()), Some(config.path()));
        let mut more = NAMES.join("\n");
        more.push_str("\nz/four.ydd");
        fs::write(names.path().join("vocabulary.txt"), more).expect("grow the names");

        let resolver = load(Some(names.path()), Some(config.path()));
        assert_eq!(names_in(&resolver, &["z/four.ydd"]), [true]);
        let stamp = stamp(&sources(Some(names.path()), Some(config.path())));
        let kept = read(&config.path().join(FILE_NAME), &stamp).expect("kept again");
        assert_eq!(kept.len(), NAMES.len() + 1);
    }

    #[test]
    fn a_harvested_vocabulary_is_named_beside_the_folder() {
        let (names, config) = folders();
        let mut harvested = Vocabulary::default();
        harvested.add("h/harvested.ydr", None);
        harvested
            .save(&vocabulary_file(config.path()))
            .expect("keep a vocabulary");
        let resolver = load(Some(names.path()), Some(config.path()));
        assert_eq!(
            names_in(&resolver, &["h/harvested.ydr", NAMES[1]]),
            [true; 2]
        );
    }

    #[test]
    fn a_corrupt_or_older_cache_is_rebuilt_and_not_trusted() {
        let (names, config) = folders();
        let file = config.path().join(FILE_NAME);
        let stamp = stamp(&sources(Some(names.path()), Some(config.path())));
        let fake = vec![(name_hash64(b"z/fake"), "z/fake".to_owned())];
        let mut cut = encode(VERSION, &stamp, &fake);
        cut.pop();
        let mut past = encode(VERSION, &stamp, &fake);
        past.push(0);
        let mut huge = encode(VERSION, &stamp, &[]);
        let at = huge.len() - 8;
        huge[at..].copy_from_slice(&u64::MAX.to_le_bytes());
        let cases = [
            ("older", encode(VERSION - 1, &stamp, &fake)),
            ("cut short", cut),
            ("past its end", past),
            ("a count past its bytes", huge),
            ("not a cache", b"garbage".to_vec()),
        ];
        for (what, bytes) in cases {
            fs::write(&file, bytes).expect("write the cache");
            assert!(read(&file, &stamp).is_err(), "{what}: taken");
            let resolver = load(Some(names.path()), Some(config.path()));
            assert_eq!(
                names_in(&resolver, &["z/fake", NAMES[2]]),
                [false, true],
                "{what}"
            );
            assert_eq!(
                read(&file, &stamp).map(|p| p.len()),
                Ok(NAMES.len()),
                "{what}"
            );
        }
    }

    #[test]
    fn the_folders_name_tables_name_a_hash_first() {
        let (names, config) = folders();
        let hash = name_hash64(NAMES[0].as_bytes());
        let toc = format!("archive\tentry\tname64\tname32\tname\tsource\na\t0\t{hash:#018x}\t0x0\tfrom/table\tt\n");
        fs::write(names.path().join("toc-entries-resolved.tsv"), toc).expect("write a table");
        fs::write(
            names.path().join("resolved-names.tsv"),
            "0x2a\tfrom/list\nnot a row\n",
        )
        .expect("write a list");
        let resolver = load(Some(names.path()), Some(config.path()));
        assert_eq!(resolver.lookup(hash), Some("from/table"));
        assert_eq!(resolver.lookup(0x2a), Some("from/list"));
    }

    #[test]
    fn the_resolver_is_built_once_a_run() {
        use super::super::{resolver, warm_names};
        warm_names();
        let calls: Vec<_> = (0..4).map(|_| std::thread::spawn(resolver)).collect();
        let first = resolver();
        for call in calls {
            let other = call.join().expect("a call returns");
            assert!(std::sync::Arc::ptr_eq(&first, &other), "built again");
        }
        assert!(std::sync::Arc::ptr_eq(&first, &resolver()), "built again");
    }

    #[test]
    fn no_source_keeps_no_cache() {
        let config = tempfile::tempdir().expect("a config folder");
        assert!(load(None, Some(config.path())).is_empty());
        assert!(!config.path().join(FILE_NAME).exists());
    }

    /// Env-gated: the names folder `OPENRAGE_GTA6_NAMES` names, loaded cold into an empty
    /// configuration and then warm from the cache that kept; prints both times.
    #[test]
    fn the_names_load_warm_from_the_cache_faster_than_cold() {
        let Some(dir) = std::env::var_os(or_io::container::rpf::GTA6_NAMES_ENV) else {
            or_test_support::skip_install(
                "the_names_load_warm_from_the_cache_faster_than_cold",
                "set OPENRAGE_GTA6_NAMES",
            );
            return;
        };
        let (dir, config) = (PathBuf::from(dir), tempfile::tempdir().expect("config"));
        let started = Instant::now();
        let cold = load(Some(&dir), Some(config.path()));
        let cold_took = started.elapsed();
        let started = Instant::now();
        let warm = load(Some(&dir), Some(config.path()));
        let warm_took = started.elapsed();
        println!(
            "names load: {} names, cold {cold_took:?}, warm {warm_took:?}",
            cold.len()
        );
        assert_eq!(warm.len(), cold.len());
        assert!(
            warm_took < cold_took,
            "warm {warm_took:?}, cold {cold_took:?}"
        );
    }
}
