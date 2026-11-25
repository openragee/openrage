//! The strings a build's PSO documents register hash, by their category's hasher, to values the same
//! documents carry: each registered string is hashed by `category_hash` and by the crate's other
//! hashers, and the share of strings whose hash is a 32-bit or 64-bit word outside the string sections
//! is printed per category. The category's own hasher must fit at least as well as any other and name
//! nearly every string, and some strings must name a 64-bit word.
//!
//! Install-gated and game-derived; it prints counts only, and skips without:
//!   `OPENRAGE_GTA6_KEYS`   the key snapshot with the string section (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

#![cfg(all(feature = "gta6", feature = "meta"))]

use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};

use or_io::container::rpf::{Gta6Mount, GtaViKeys, GTA6_SNAPSHOT_ENV};
use or_io::gta6::hash::name_hash64;
use or_io::meta::binary::pso::string_names::{category_hash, registered_strings};
use or_io::{literal_string_hash, string_hash};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
const CONTENT_CIPHER_TYPE: u16 = 17;
/// The categories the game registers, each walked for until it has [`MEASURED`] strings.
const REGISTERED: [u32; 2] = [0x01, 0x1E];
/// Strings enough to tell a category's hashers apart.
const MEASURED: usize = 20;
/// The strings after which the walk stops, whether or not every category was met: walking the
/// whole build for a category it may not hold runs past the slow tier's limit.
const STRING_LIMIT: usize = 600;
/// How long the walk reads before it stops and judges what it met, inside the slow tier's limit.
const WALK_TIME: std::time::Duration = std::time::Duration::from_secs(600);
const MAX_DEPTH: usize = 8;
const MAX_ENTRY: u64 = 4 << 20;
const STRING_SECTIONS: [&[u8; 4]; 4] = [b"LSNS", b"STNS", b"STDN", b"STFN"];
/// A category's own hasher names at least this share of its strings.
const FIT: f64 = 0.9;

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

fn build_archives(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut rpfs_in = |dir: &Path| {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("rpf") {
                out.push(path);
            }
        }
    };
    rpfs_in(root);
    for pack in std::fs::read_dir(root.join("pack"))
        .into_iter()
        .flatten()
        .flatten()
    {
        if pack.path().is_dir() {
            rpfs_in(&pack.path());
        }
    }
    out.sort();
    out
}

/// The words of a document's sections other than its string sections: every 4-aligned u32, read
/// both ways round, and every 4-aligned little-endian u64.
fn carried_words(doc: &[u8]) -> (HashSet<u32>, HashSet<u64>) {
    let (mut narrow, mut wide) = (HashSet::new(), HashSet::new());
    let mut p = 0usize;
    while p + 8 <= doc.len() {
        let len = i32::from_be_bytes([doc[p + 4], doc[p + 5], doc[p + 6], doc[p + 7]]);
        let Some(end) = usize::try_from(len).ok().filter(|&l| l >= 8).map(|l| p + l) else {
            break;
        };
        if end > doc.len() {
            break;
        }
        if !STRING_SECTIONS.iter().any(|tag| doc[p..p + 4] == tag[..]) {
            let body = &doc[p + 8..end];
            for &w in body.as_chunks::<4>().0 {
                narrow.insert(u32::from_le_bytes(w));
                narrow.insert(u32::from_be_bytes(w));
            }
            for at in (0..body.len().saturating_sub(7)).step_by(4) {
                let mut w = [0u8; 8];
                w.copy_from_slice(&body[at..at + 8]);
                wide.insert(u64::from_le_bytes(w));
            }
        }
        p = end;
    }
    (narrow, wide)
}

#[derive(Default, Debug)]
struct Fit {
    strings: usize,
    own: usize,
    folded: usize,
    literal: usize,
    wide: usize,
}

#[derive(Default)]
struct Tally {
    documents: usize,
    strings: usize,
    by_category: BTreeMap<u32, Fit>,
    until: Option<std::time::Instant>,
}

fn check_doc(doc: &[u8], keys: &GtaViKeys, t: &mut Tally) {
    if !doc.starts_with(b"PSIN") {
        return;
    }
    let Ok(strings) = registered_strings(doc, Some(keys)) else {
        return;
    };
    if strings.is_empty() {
        return;
    }
    let (narrow, wide) = carried_words(doc);
    t.documents += 1;
    for s in &strings {
        let text = s.text.as_bytes();
        let fit = t.by_category.entry(s.category).or_default();
        fit.strings += 1;
        let carried = |hash: Option<u32>| usize::from(hash.is_some_and(|h| narrow.contains(&h)));
        fit.own += carried(category_hash(s.category, text));
        fit.folded += carried(Some(string_hash(text)));
        fit.literal += carried(Some(literal_string_hash(text)));
        fit.wide += usize::from(wide.contains(&name_hash64(text)));
    }
    t.strings += strings.len();
}

fn walk(file: &mut File, mount: &Gta6Mount, keys: &GtaViKeys, depth: usize, t: &mut Tally) {
    if depth > MAX_DEPTH {
        return;
    }
    for entry in &mount.archive.entries {
        if t.done() {
            return;
        }
        let clear = entry.cipher.is_none();
        if clear
            && entry.start.is_some()
            && mount
                .entry_begins_archive(&mut *file, entry)
                .unwrap_or(false)
        {
            if let Ok(child) = mount.open_nested(&mut *file, entry, keys) {
                walk(file, &child, keys, depth + 1, t);
            }
            continue;
        }
        let cipher = entry.cipher.as_ref();
        let content = cipher.is_some_and(|c| c.cipher_type == CONTENT_CIPHER_TYPE);
        if (clear || content) && !entry.real.is_some_and(|r| r > MAX_ENTRY) {
            if let Ok(doc) = mount.read_entry_decrypted(&mut *file, entry, keys) {
                check_doc(&doc, keys, t);
            }
        }
    }
}

impl Tally {
    fn done(&self) -> bool {
        self.strings >= STRING_LIMIT
            || self
                .until
                .is_some_and(|until| std::time::Instant::now() >= until)
            || REGISTERED.iter().all(|c| {
                self.by_category
                    .get(c)
                    .is_some_and(|f| f.strings >= MEASURED)
            })
    }
}

fn share(n: usize, of: usize) -> f64 {
    n as f64 / of.max(1) as f64
}

#[test]
fn registered_strings_hash_by_their_category_to_values_their_documents_carry() {
    let what = "GTA VI PSO string hashes per category over a build";
    let (Some(snapshot), Some(build)) = (env_path(GTA6_SNAPSHOT_ENV), env_path(BUILD_ENV)) else {
        skip_install(
            what,
            format_args!("set {GTA6_SNAPSHOT_ENV} and {BUILD_ENV}"),
        );
        return;
    };
    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
    if !keys.has_string_keys() {
        skip_install(
            what,
            format_args!("the snapshot carries no string section; rebuild it"),
        );
        return;
    }
    let mut t = Tally {
        until: Some(std::time::Instant::now() + WALK_TIME),
        ..Tally::default()
    };
    for path in build_archives(&build) {
        if t.done() {
            break;
        }
        let mut file = File::open(&path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys)
            .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
        walk(&mut file, &mount, &keys, 0, &mut t);
    }

    println!(
        "{} registered strings in {} documents",
        t.strings, t.documents
    );
    for (category, f) in &t.by_category {
        println!(
            "category {category:#x}: {} strings; own hasher {:.3}, atStringHash {:.3}, literal {:.3}, \
             64-bit {:.3}",
            f.strings,
            share(f.own, f.strings),
            share(f.folded, f.strings),
            share(f.literal, f.strings),
            share(f.wide, f.strings),
        );
    }
    if t.strings < 100 {
        skip_install(
            what,
            format_args!(
                "the walk met only {} registered strings in {} documents before it stopped",
                t.strings, t.documents
            ),
        );
        return;
    }
    for (category, f) in t.by_category.iter().filter(|(_, f)| f.strings >= MEASURED) {
        // Category 1's own hasher is atStringHash, so only the literal hash is another.
        let others_fit = if *category == 0x01 {
            f.own >= f.literal
        } else {
            f.own >= f.folded && f.own >= f.literal
        };
        assert!(
            others_fit,
            "category {category:#x}: another hasher fits better than its own: {f:?}"
        );
        assert!(
            share(f.own, f.strings) >= FIT,
            "category {category:#x}: its hasher names too few strings: {f:?}"
        );
    }
    let wide: usize = t.by_category.values().map(|f| f.wide).sum();
    assert!(
        wide > 0,
        "no registered string named a 64-bit word its document carries"
    );
    for category in REGISTERED {
        let met = t.by_category.get(&category).map_or(0, |f| f.strings);
        if met < MEASURED {
            skip_install(
                what,
                format_args!(
                    "category {category:#x} was met {met} times in {} strings, so its hasher is unmeasured",
                    t.strings
                ),
            );
        }
    }
}
