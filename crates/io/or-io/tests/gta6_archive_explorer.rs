//! The RPF9 archive explorer over a real build: every archive lists every entry through the
//! entry-major index (not the old row-order pairing), the entry rows decode to start/stored/real, the
//! content-cipher column is counted, and the one name resolver resolves each entry's name64.
//!
//! Install-gated and game-derived, so it needs the same env vars as the TOC oracle test and skips
//! cleanly without them, which is why a clean checkout (`just test-no-pack`) passes:
//!   `OPENRAGE_GTA6_KEYS`   the `gtavi_magic.dat` key snapshot (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`
//!   `OPENRAGE_GTA6_NAMES`  optional: a folder with `vocabulary.txt` and the name tables
//!
//! Walking the build's tables is slow-tier work; keep it out of the default tier.

#![cfg(feature = "gta6")]

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use or_io::container::rpf::{Gta6Archive, GtaViKeys, NameResolver, GTA6_SNAPSHOT_ENV};
use or_io::gta6::hash::name_hash32;
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
const NAMES_ENV: &str = "OPENRAGE_GTA6_NAMES";

/// The totals the decode reproduces over one build's archives, measured per build: the entry rows,
/// the content-cipher records, the 24-byte entry row's class byte split (0x00, 0x0B, 0xFF, 0x0E),
/// and what `textures.rpf` holds, which the builds lay out differently.
struct BuildShape {
    total_entries: usize,
    total_ciphers: usize,
    class_00: usize,
    class_0b: usize,
    class_ff: usize,
    class_0e: usize,
    textures_entries: usize,
    /// The name hashes `textures.rpf` must hold.
    textures_hashes: &'static [u64],
}

const XBOX_SERIES: BuildShape = BuildShape {
    total_entries: 6_534,
    total_ciphers: 455,
    class_00: 4_351,
    class_0b: 864,
    class_ff: 1_317,
    class_0e: 1,
    textures_entries: 22,
    textures_hashes: &[0x01D3_E030_56A1_A5C7],
};

const PLAYSTATION_5: BuildShape = BuildShape {
    total_entries: 7_890,
    total_ciphers: 454,
    class_00: 5_699,
    class_0b: 864,
    class_ff: 1_325,
    class_0e: 1,
    textures_entries: 2,
    textures_hashes: &[0xF21B_0A3A_5961_26D5, 0x3236_3B06_4327_5D70],
};

/// The measured shape of the build under `root`: a PlayStation 5 build is named by its `eboot.bin`.
fn shape_of(root: &Path) -> &'static BuildShape {
    if root.join("eboot.bin").is_file() {
        &PLAYSTATION_5
    } else {
        &XBOX_SERIES
    }
}

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

fn read_toc_prefix(path: &Path) -> Vec<u8> {
    let mut file = std::fs::File::open(path).expect("open the archive");
    let mut head = [0u8; 64];
    file.read_exact(&mut head).expect("read the RPF9 header");
    let toc_end = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
    let mut buf = vec![0u8; toc_end];
    file.seek(SeekFrom::Start(0)).expect("rewind");
    file.read_exact(&mut buf)
        .expect("read the table of contents");
    buf
}

/// Every `*.rpf` under `root` and every `pack/*/*.rpf`.
fn build_archives(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut rpfs_in = |dir: &Path| {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("rpf") {
                    out.push(path);
                }
            }
        }
    };
    rpfs_in(root);
    if let Ok(packs) = std::fs::read_dir(root.join("pack")) {
        for pack in packs.flatten() {
            if pack.path().is_dir() {
                rpfs_in(&pack.path());
            }
        }
    }
    out.sort();
    out
}

#[test]
fn every_build_archive_lists_every_entry_with_decoded_rows() {
    let what = "GTA VI RPF9 archive explorer over a build";
    let (Some(snapshot), Some(build)) = (env_path(GTA6_SNAPSHOT_ENV), env_path(BUILD_ENV)) else {
        skip_install(
            what,
            format_args!(
                "set {GTA6_SNAPSHOT_ENV} (the gtavi_magic.dat) and {BUILD_ENV} (the build root)"
            ),
        );
        return;
    };

    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
    let shape = shape_of(&build);
    let archives = build_archives(&build);
    assert!(!archives.is_empty(), "no .rpf under {}", build.display());

    let mut total_entries = 0usize;
    let mut total_ciphers = 0usize;
    let mut class = [0usize; 256];
    for path in &archives {
        let prefix = read_toc_prefix(path);
        let toc_end = prefix.len() as u64;
        let file_len = std::fs::metadata(path).expect("archive length").len();
        let archive = Gta6Archive::open(&prefix, &keys)
            .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap();

        // Every entry is listed, in the archive's own order — the fix for the four archives (levels,
        // packs, navmeshes, core_restofworld) the row-order pairing listed nothing for.
        assert_eq!(
            archive.entries.len(),
            archive.toc.entry_count as usize,
            "{stem}: listed {} rows, not the {} entries the header states",
            archive.entries.len(),
            archive.toc.entry_count
        );
        for (i, entry) in archive.entries.iter().enumerate() {
            assert_eq!(entry.index as usize, i, "{stem}: entries out of order");
            // Every top-level entry carries a name64 and an entry row through the index, so none is
            // listed blind: a decode that lost a column, or indexed by the wrong slot, drops these.
            assert_ne!(entry.name64, 0, "{stem}: entry {i} has no name64");
            let start = entry
                .start
                .unwrap_or_else(|| panic!("{stem}: entry {i} has no entry row"));
            // The content begins after the table of contents, and a 24-byte row's stored body lies
            // within the file — a wrong start or stored size breaks one of these.
            assert!(
                start >= toc_end,
                "{stem}: entry {i} starts inside the table of contents"
            );
            if let Some(stored) = entry.stored {
                assert_eq!(stored % 16, 0, "{stem}: stored size not 16-byte aligned");
                assert!(
                    entry.real.is_some(),
                    "{stem}: a 24-byte row without a real size"
                );
                assert!(
                    start + stored <= file_len,
                    "{stem}: entry {i} runs past the end of the file"
                );
            }
            if let Some(c) = entry.class {
                class[c as usize] += 1;
            }
        }
        total_entries += archive.entries.len();
        total_ciphers += archive
            .entries
            .iter()
            .filter(|e| e.cipher.is_some())
            .count();
    }

    assert_eq!(
        total_entries,
        shape.total_entries,
        "entries across {} archives",
        archives.len()
    );
    assert_eq!(total_ciphers, shape.total_ciphers, "content-cipher records");
    // The 24-byte class byte split; a mislabelled row shape, a dropped archive or a miscounted column
    // slot would move these.
    assert_eq!(class[0x00], shape.class_00, "class 0x00 rows");
    assert_eq!(class[0x0B], shape.class_0b, "class 0x0B rows");
    assert_eq!(class[0xFF], shape.class_ff, "class 0xFF rows");
    assert_eq!(class[0x0E], shape.class_0e, "class 0x0E rows");
    println!(
        "{} RPF9 archives: {total_entries} entries, {total_ciphers} content-cipher records",
        archives.len()
    );
}

#[test]
fn appdata0_and_textures_match_the_known_entry_names() {
    let what = "GTA VI RPF9 explorer against known entry names";
    let (Some(snapshot), Some(build)) = (env_path(GTA6_SNAPSHOT_ENV), env_path(BUILD_ENV)) else {
        skip_install(
            what,
            format_args!("set {GTA6_SNAPSHOT_ENV} and {BUILD_ENV}"),
        );
        return;
    };
    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");

    let has = |a: &Gta6Archive, hash: u64| a.entries.iter().any(|e| e.name64 == hash);
    let shape = shape_of(&build);
    let appdata0 = build.join("appdata0.rpf");
    if appdata0.exists() {
        let a = Gta6Archive::open(&read_toc_prefix(&appdata0), &keys).expect("open appdata0");
        assert_eq!(a.toc.entry_count, 2);
        assert_eq!(a.entries.len(), 2);
        assert!(has(&a, 0x092D_F786_29F7_833A) && has(&a, 0x3363_73B5_5666_93F1));
    }
    let textures = build.join("textures.rpf");
    if textures.exists() {
        let t = Gta6Archive::open(&read_toc_prefix(&textures), &keys).expect("open textures");
        assert_eq!(t.toc.entry_count as usize, shape.textures_entries);
        assert_eq!(t.entries.len(), shape.textures_entries);
        for hash in shape.textures_hashes {
            assert!(has(&t, *hash), "textures lacks the {hash:#018X} entry");
        }
    }

    if let Some(dir) = env_path(NAMES_ENV) {
        let resolver = NameResolver::load(&dir);
        if !resolver.is_empty() && appdata0.exists() {
            let mut a =
                Gta6Archive::open(&read_toc_prefix(&appdata0), &keys).expect("open appdata0");
            a.resolve_names(&resolver);
            let name_of = |hash: u64| {
                a.entries
                    .iter()
                    .find(|e| e.name64 == hash)
                    .and_then(|e| e.name.clone())
            };
            assert_eq!(
                name_of(0x092D_F786_29F7_833A).as_deref(),
                Some("config.dat")
            );
            assert_eq!(
                name_of(0x3363_73B5_5666_93F1).as_deref(),
                Some("install.dat")
            );
        }
    }
}

#[test]
fn the_name_resolver_resolves_the_corpus_and_cross_checks_name32() {
    let what = "GTA VI RPF9 one name resolver over a build";
    let (Some(snapshot), Some(build), Some(names)) = (
        env_path(GTA6_SNAPSHOT_ENV),
        env_path(BUILD_ENV),
        env_path(NAMES_ENV),
    ) else {
        skip_install(
            what,
            format_args!("set {GTA6_SNAPSHOT_ENV}, {BUILD_ENV} and {NAMES_ENV}"),
        );
        return;
    };
    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
    // The corpus vocabulary alone, with no per-build shortcut tables: those are derived from one
    // build's own tables and resolve by construction, so they would mask a broken name64 decode.
    let resolver = NameResolver::load_vocabulary(&names);
    if resolver.is_empty() {
        skip_install(
            what,
            format_args!("no names vocabulary under {}", names.display()),
        );
        return;
    }

    let mut resolved = 0usize;
    let mut name32_checked = 0usize;
    for path in build_archives(&build) {
        let mut archive =
            Gta6Archive::open(&read_toc_prefix(&path), &keys).expect("open the archive");
        archive.resolve_names(&resolver);
        for entry in &archive.entries {
            let Some(name) = entry.name.as_deref() else {
                continue;
            };
            resolved += 1;
            // Where the archive also carries a name32, the resolved path must hash to it: this is the
            // cross-check that name64 resolution found the right name, not a collision.
            if let Some(name32) = entry.name32 {
                assert_eq!(
                    name_hash32(name.as_bytes()),
                    name32,
                    "{}: {name}'s joaat disagrees with its name32 column",
                    path.display()
                );
                name32_checked += 1;
            }
        }
    }

    println!("names: {resolved} resolved, {name32_checked} cross-checked against name32");
    // About 5,977 of the Xbox build's 6,534 top-level entries resolve through the corpus; the bar
    // allows a tenth of slack for a slightly different corpus, since a decode that listed nothing for
    // the four broken archives or resolved by the wrong hash falls far below. The hard correctness
    // check is the name32 cross-check above, which fails on any wrong name64 resolution.
    const TOTAL_NAMES_RESOLVED: usize = 5_977;
    assert!(
        resolved >= TOTAL_NAMES_RESOLVED - TOTAL_NAMES_RESOLVED / 10,
        "only {resolved} names resolved; about {TOTAL_NAMES_RESOLVED} are expected"
    );
}
