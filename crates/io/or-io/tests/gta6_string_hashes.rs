//! The strings a build's PSO documents register hash, by their category's hasher, to values the same
//! documents carry: each registered string is hashed by `category_hash`, by the crate's other 32-bit
//! hasher and by the 64-bit name hash, and counted where the hash is a word outside the string
//! sections. The walk reads only the compressed resources of the archives that hold registered runs,
//! and the counts are the build's.
//!
//! Install-gated and game-derived; it prints counts only, and skips without:
//!   `OPENRAGE_GTA6_KEYS`   the key snapshot with the string section (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root

#![cfg(all(feature = "gta6", feature = "meta"))]

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};

use or_io::container::rpf::{Gta6Mount, GtaViKeys, GTA6_SNAPSHOT_ENV};
use or_io::gta6::hash::{name_hash32, name_hash64};
use or_io::meta::binary::pso::string_names::{category_hash, registered_strings};
use or_io::{literal_string_hash, string_hash};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
const CONTENT_CIPHER_TYPE: u16 = 17;
/// The archives that hold every registered run outside the two largest nested trees, by the name hash
/// of their file name, and whether their nested archives are walked too: a sorted whole-build walk
/// reads the largest archives first and never gets here.
const ARCHIVES: [(u32, bool); 2] = [(0xBC35_2010, true), (0x61CA_8E15, false)];
/// The class byte of a nested archive's row; no archive sits inside a nested one.
const NESTED_CLASS: u8 = 0x00;
/// The class byte of a compressed resource's row, which every document holding a registered run has.
// ⚠ Reading every row of the nested archives, and listing the words of every PSO document whether
// it registers a string or not, held this test at its 900 s cap.
const RESOURCE_CLASS: u8 = 0x0B;
const MAX_DEPTH: usize = 8;
const MAX_ENTRY: u64 = 4 << 20;
const STRING_SECTIONS: [&[u8; 4]; 4] = [b"LSNS", b"STNS", b"STDN", b"STFN"];

/// What the build's registered strings give, per category, over [`ARCHIVES`].
const MEASURED: [(u32, Fit); 2] = [
    (
        0x01,
        Fit {
            documents: 17,
            strings: 33_072,
            own: 33_072,
            folded: 33_072,
            literal: 438,
            wide: 60,
        },
    ),
    (
        0x1E,
        Fit {
            documents: 4,
            strings: 729,
            own: 729,
            folded: 654,
            literal: 729,
            wide: 0,
        },
    ),
];

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

/// The archive under `root` or one of `root/pack/*` whose file name hashes to `name`.
fn archive_named(root: &Path, name: u32) -> Option<PathBuf> {
    let dirs = std::fs::read_dir(root.join("pack"))
        .into_iter()
        .flatten()
        .flatten();
    let dirs = std::iter::once(root.to_path_buf()).chain(dirs.map(|d| d.path()));
    dirs.flat_map(|dir| std::fs::read_dir(dir).into_iter().flatten().flatten())
        .map(|entry| entry.path())
        .find(|path| {
            (path.file_name()).is_some_and(|n| name_hash32(n.to_string_lossy().as_bytes()) == name)
        })
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

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
struct Fit {
    documents: usize,
    strings: usize,
    own: usize,
    folded: usize,
    literal: usize,
    wide: usize,
}

#[derive(Default)]
struct Tally {
    unread: usize,
    by_category: BTreeMap<u32, Fit>,
}

fn check_doc(doc: &[u8], keys: &GtaViKeys, t: &mut Tally) {
    if !doc.starts_with(b"PSIN") {
        return;
    }
    let Ok(strings) = registered_strings(doc, Some(keys)) else {
        t.unread += 1;
        return;
    };
    if strings.is_empty() {
        return;
    }
    let (narrow, wide) = carried_words(doc);
    let mut categories = BTreeSet::new();
    for s in &strings {
        let text = s.text.as_bytes();
        categories.insert(s.category);
        let fit = t.by_category.entry(s.category).or_default();
        fit.strings += 1;
        let carried = |hash: Option<u32>| usize::from(hash.is_some_and(|h| narrow.contains(&h)));
        fit.own += carried(category_hash(s.category, text));
        fit.folded += carried(Some(string_hash(text)));
        fit.literal += carried(Some(literal_string_hash(text)));
        fit.wide += usize::from(wide.contains(&name_hash64(text)));
    }
    for category in categories {
        t.by_category.entry(category).or_default().documents += 1;
    }
}

fn walk(
    file: &mut File,
    mount: &Gta6Mount,
    keys: &GtaViKeys,
    depth: usize,
    nested: bool,
    t: &mut Tally,
) {
    if depth > MAX_DEPTH {
        return;
    }
    for entry in &mount.archive.entries {
        let clear = entry.cipher.is_none();
        if clear
            && entry.class == Some(NESTED_CLASS)
            && entry.start.is_some()
            && mount
                .entry_begins_archive(&mut *file, entry)
                .unwrap_or(false)
        {
            if nested {
                if let Ok(child) = mount.open_nested(&mut *file, entry, keys) {
                    walk(file, &child, keys, depth + 1, nested, t);
                }
            }
            continue;
        }
        if entry.class != Some(RESOURCE_CLASS) {
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
    let mut t = Tally::default();
    for (archive, nested) in ARCHIVES {
        let path = archive_named(&build, archive)
            .unwrap_or_else(|| panic!("the build holds no archive of name hash {archive:#010X}"));
        let mut file = File::open(&path).unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
        let mount = Gta6Mount::open_root(&mut file, &keys)
            .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
        walk(&mut file, &mount, &keys, 0, nested, &mut t);
    }

    for (category, f) in &t.by_category {
        println!(
            "category {category:#x}: {} strings in {} documents; own hasher {:.4}, atStringHash {:.4}, \
             literal {:.4}, 64-bit {:.4}",
            f.strings,
            f.documents,
            share(f.own, f.strings),
            share(f.folded, f.strings),
            share(f.literal, f.strings),
            share(f.wide, f.strings),
        );
    }
    assert_eq!(t.unread, 0, "a PSO document's string sections did not read");
    for (category, f) in &t.by_category {
        let other = match category {
            0x01 => f.literal,
            _ => f.folded,
        };
        assert!(
            f.own == f.strings && other < f.strings,
            "category {category:#x}: its own hasher must name every string and the other fewer: {f:?}"
        );
    }
    assert_eq!(t.by_category, BTreeMap::from(MEASURED));
}
