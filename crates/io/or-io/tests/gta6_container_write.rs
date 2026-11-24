//! The RPF9 container writer over a real GTA VI build: a copy of every archive is byte-identical
//! (each table deciphered and re-enciphered, every body copied in place), a replace of an entry by its
//! own body is byte-identical at the top level and inside a nested archive, and removing an entry
//! lays the table out anew with every other entry and column read back unchanged.
//!
//! Structure only: an edited table's signature is stale and the game refuses such an archive.
//!
//! Install-gated and game-derived, so it skips cleanly without its env vars:
//!   `OPENRAGE_GTA6_KEYS`   the `gtavi_magic.dat` key snapshot (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

#![cfg(feature = "gta6")]

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use or_io::container::rpf::{
    Gta6Archive, Gta6Class, Gta6Entry, Gta6Mount, GtaViKeys, Rpf9ArchiveWriter, Rpf9Piece,
    Rpf9Plan, GTA6_SNAPSHOT_ENV,
};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// A body larger than this is not read for the replace-by-itself check.
const CAP: u64 = 4 << 20;
/// Entries tried per archive before the replace-by-itself check passes it over.
const TRIES: usize = 20;

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

/// Every `*.rpf` directly under `root`, then every `pack/*/*.rpf`, sorted.
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

/// `plan` reproduces `file` byte for byte: what it writes equals the file where it lands, every
/// copy is of the same offset, and the length is the file's.
fn assert_identity(plan: &Rpf9Plan, file: &mut File, what: &str) {
    let mut at = 0u64;
    for piece in &plan.pieces {
        match piece {
            Rpf9Piece::Bytes(bytes) => {
                let mut held = vec![0u8; bytes.len()];
                file.seek(SeekFrom::Start(at)).unwrap();
                file.read_exact(&mut held).unwrap();
                assert!(held == *bytes, "{what}: written bytes at {at:#x} differ");
            }
            Rpf9Piece::Copy { from, .. } => assert_eq!(*from, at, "{what}: a copy moved"),
            Rpf9Piece::Zero(len) => {
                let mut held = vec![0u8; *len as usize];
                file.seek(SeekFrom::Start(at)).unwrap();
                file.read_exact(&mut held).unwrap();
                assert!(
                    held.iter().all(|&b| b == 0),
                    "{what}: zeroed bytes at {at:#x}"
                );
            }
        }
        at += piece.size();
    }
    assert_eq!(at, file.metadata().unwrap().len(), "{what}: length");
}

/// The body the writer compares a replacement with: a chunked entry inflated by its chunk table, any
/// other as its raw `[start, start + real)`.
fn body_of(mount: &Gta6Mount, file: &mut File, keys: &GtaViKeys, e: &Gta6Entry) -> Option<Vec<u8>> {
    let chunked = mount
        .archive
        .entry_extra(e.index as usize)?
        .chunk_count
        .is_some();
    if chunked && e.cipher.is_some() {
        return mount.read_entry_decrypted(&mut *file, e, keys).ok();
    }
    let mut body = vec![0u8; if chunked { e.stored? } else { e.real? } as usize];
    file.seek(SeekFrom::Start(mount.absolute_start(e)?)).ok()?;
    file.read_exact(&mut body).ok()?;
    if chunked {
        return mount.inflate_deciphered_stored(&body, e).ok();
    }
    Some(body)
}

/// The first leaf with a 24-byte row whose body reads, and that body.
fn first_leaf(mount: &Gta6Mount, file: &mut File, keys: &GtaViKeys) -> Option<(u32, Vec<u8>)> {
    mount
        .archive
        .entries
        .iter()
        .filter(|e| {
            e.kind().is_some_and(|k| k != Gta6Class::NestedArchive)
                && e.real.is_some_and(|r| r <= CAP)
        })
        .take(TRIES)
        .find_map(|e| body_of(mount, file, keys, e).map(|b| (e.index, b)))
}

#[test]
fn every_build_archive_copies_byte_identical_and_takes_structural_edits() {
    let what = "GTA VI RPF9 container writer over a build";
    let (Some(snapshot), Some(build)) = (env_path(GTA6_SNAPSHOT_ENV), env_path(BUILD_ENV)) else {
        skip_install(
            what,
            format_args!("set {GTA6_SNAPSHOT_ENV} and {BUILD_ENV}"),
        );
        return;
    };
    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
    let archives = build_archives(&build);
    assert!(!archives.is_empty(), "the build root holds no archive");
    let mut nested_checked = false;

    for path in &archives {
        let name = path.display().to_string();
        let mut file = File::open(path).expect("open the archive");

        let plan = Rpf9ArchiveWriter::new()
            .plan(&mut file, &keys)
            .expect("a copy plans");
        assert_eq!(plan.tables_changed, 0, "{name}");
        assert_eq!(
            plan.pieces.len(),
            2,
            "{name}: the table, then the rest copied"
        );
        assert_identity(&plan, &mut file, &name);

        let mount = Gta6Mount::open_root(&mut file, &keys).expect("the archive opens");
        if let Some((i, body)) = first_leaf(&mount, &mut file, &keys) {
            let plan = Rpf9ArchiveWriter::new()
                .replace(&[i], body)
                .plan(&mut file, &keys)
                .expect("a replace by itself plans");
            assert_eq!(plan.tables_changed, 0, "{name}: entry {i}");
            assert_identity(&plan, &mut file, &format!("{name}: replace {i} by itself"));
        }

        if !nested_checked {
            for entry in mount
                .archive
                .entries
                .iter()
                .filter(|e| e.kind() == Some(Gta6Class::NestedArchive))
            {
                let Ok(child) = mount.open_nested(&mut file, entry, &keys) else {
                    continue;
                };
                let Some((j, body)) = first_leaf(&child, &mut file, &keys) else {
                    continue;
                };
                let plan = Rpf9ArchiveWriter::new()
                    .replace(&[entry.index, j], body)
                    .plan(&mut file, &keys)
                    .expect("a nested replace by itself plans");
                assert_eq!(plan.tables_changed, 0, "{name}: {} / {j}", entry.index);
                assert_identity(
                    &plan,
                    &mut file,
                    &format!("{name}: nested replace by itself"),
                );
                nested_checked = true;
                break;
            }
        }

        let entries = &mount.archive.entries;
        if entries.len() < 2 {
            continue;
        }
        let last = entries.len() - 1;
        let plan = Rpf9ArchiveWriter::new()
            .remove(&[last as u32])
            .plan(&mut file, &keys)
            .expect("a remove plans");
        assert_eq!(plan.tables_changed, 1, "{name}");
        assert_eq!(
            plan.size(),
            file.metadata().unwrap().len(),
            "{name}: bodies stay in place"
        );
        let Rpf9Piece::Bytes(toc) = &plan.pieces[0] else {
            panic!("{name}: the plan starts with the table");
        };
        let after = Gta6Archive::open(toc, &keys).expect("the re-laid table deciphers and parses");
        assert_eq!(
            after.entries[..],
            entries[..last],
            "{name}: the kept entries"
        );
        assert_eq!(
            after.entry_extras()[..],
            mount.archive.entry_extras()[..last],
            "{name}: the kept entries' other columns"
        );
        let (old, new) = (&mount.archive.toc.bytes, &after.toc.bytes);
        let n = u16::from_le_bytes([old[0x1A], old[0x1B]]) as usize;
        let m = u32::from_le_bytes([old[0x1C], old[0x1D], old[0x1E], old[0x1F]]) as usize;
        let metadata_end = 0x40 + (8 * n).next_multiple_of(16) + m;
        assert_eq!(
            old[0x10..0x18],
            new[0x10..0x18],
            "{name}: the per-archive id"
        );
        assert_eq!(old[0x28..0x2C], new[0x28..0x2C], "{name}: header +0x28");
        assert_eq!(old[0x2D], new[0x2D], "{name}: header +0x2D");
        assert_eq!(
            new[0x2C],
            if last <= 0xFF { 1 } else { 2 },
            "{name}: the index width"
        );
        assert_eq!(
            old[0x40..metadata_end],
            new[0x40..metadata_end],
            "{name}: the metadata"
        );
    }
    assert!(
        nested_checked,
        "no nested archive held a readable 24-byte-row leaf"
    );
}
