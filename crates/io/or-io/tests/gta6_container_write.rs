//! The RPF9 container writer over a real GTA VI build: a copy of every archive is byte-identical
//! (each table deciphered and re-enciphered, every body copied in place), a replace of an entry by its
//! own body is byte-identical at the top level and inside a nested archive, and removing an entry
//! lays the table out anew with every other entry and column read back unchanged. Every table of
//! the build, nested ones at every depth, lays out anew byte for byte with its heaps, and a
//! headerless resource replaced by its own loose form reads back as the same resource.
//!
//! Structure only: an edited table's signature is stale and the game refuses such an archive.
//!
//! Install-gated and game-derived, so it skips cleanly without its env vars:
//!   `OPENRAGE_GTA6_KEYS`   the `gtavi_magic.dat` key snapshot (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

#![cfg(feature = "gta6")]

use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use or_io::container::rpf::gta6_chunk_table::entry_chunks;
use or_io::container::rpf::gta6_container_write::relays_identically;
use or_io::container::rpf::rpf9_scan::{extract_rpf9_body, read_rpf9_body};
use or_io::container::rpf::rpf9_write::Rpf9Sections;
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
/// Nesting deeper than this is not walked.
const MAX_DEPTH: usize = 8;
/// A nested archive larger than this is not read whole for the headerless replace.
const NESTED_CAP: u64 = 64 << 20;
/// Nested archives read per top-level archive, and headerless entries replaced per nested archive.
const NESTED_READS: usize = 8;
const RESOURCE_TRIES: usize = 4;
/// The 16-byte zero slot a headerless body begins with.
const SLOT: u64 = 16;

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

/// Count every table under `mount` (itself included) that lays out anew byte for byte, and note
/// each that does not by its archive number and nesting depth.
fn relay(
    file: &mut File,
    mount: &Gta6Mount,
    keys: &GtaViKeys,
    (archive, depth): (usize, usize),
    (tables, failures): (&mut usize, &mut Vec<String>),
) {
    *tables += 1;
    match relays_identically(&mount.archive.toc) {
        Ok(true) => {}
        Ok(false) => failures.push(format!(
            "archive {archive}, depth {depth}: lays out anew differently"
        )),
        Err(e) => failures.push(format!("archive {archive}, depth {depth}: {e}")),
    }
    if depth >= MAX_DEPTH {
        return;
    }
    for entry in &mount.archive.entries {
        if !mount
            .entry_begins_archive(&mut *file, entry)
            .unwrap_or(false)
        {
            continue;
        }
        if let Ok(child) = mount.open_nested(&mut *file, entry, keys) {
            relay(
                file,
                &child,
                keys,
                (archive, depth + 1),
                (&mut *tables, &mut *failures),
            );
        }
    }
}

#[test]
fn every_table_of_the_build_lays_out_anew_byte_identical() {
    let what = "GTA VI RPF9 table layout with heaps over a build";
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
    let (mut tables, mut failures) = (0usize, Vec::new());
    for (i, path) in archives.iter().enumerate() {
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys).expect("the archive opens");
        relay(
            &mut file,
            &mount,
            &keys,
            (i, 0),
            (&mut tables, &mut failures),
        );
    }
    println!(
        "tables {tables}, laid out anew differently or refused {}",
        failures.len()
    );
    for failure in failures.iter().take(20) {
        eprintln!("  {failure}");
    }
    assert!(tables > archives.len(), "the build holds nested tables");
    assert!(
        failures.is_empty(),
        "every table must lay out anew byte for byte"
    );
}

/// The sections a read at `at` of `bytes` returns, where it is a resource.
fn sections_at(bytes: &[u8], at: u64) -> Option<Rpf9Sections> {
    let (find, read) = read_rpf9_body(Cursor::new(bytes), at, u64::MAX).ok()?;
    Rpf9Sections::of(&find, &read)
}

/// Replace up to [`RESOURCE_TRIES`] clear headerless entries of the nested archive `bytes` by their
/// own loose form, each read back; the entries replaced, and each failure.
fn replace_headerless(
    bytes: &[u8],
    keys: &GtaViKeys,
    what: &str,
    failures: &mut Vec<String>,
) -> usize {
    let Ok(mount) = Gta6Mount::open_root(&mut Cursor::new(bytes), keys) else {
        return 0;
    };
    let extras = mount.archive.entry_extras();
    let mut replaced = 0;
    for (entry, extra) in mount.archive.entries.iter().zip(&extras) {
        if replaced >= RESOURCE_TRIES {
            break;
        }
        let (Some(row), None, Some(start)) = (extra.nested, entry.cipher, entry.start) else {
            continue;
        };
        if row.stored_size() > CAP
            || !matches!(
                entry_chunks(&mount.archive, entry.index as usize),
                Ok(Some(_))
            )
        {
            continue;
        }
        let Some(before) = sections_at(bytes, start + SLOT) else {
            continue;
        };
        let Ok((_, loose)) = extract_rpf9_body(Cursor::new(bytes), start + SLOT, CAP) else {
            continue;
        };
        let e = entry.index;
        let fail = |why: String| format!("{what}, entry {e}: {why}");
        let mut src = Cursor::new(bytes);
        let plan = match Rpf9ArchiveWriter::new()
            .replace_resource(&[e], loose, None)
            .plan(&mut src, keys)
        {
            Ok(plan) => plan,
            Err(err) => {
                failures.push(fail(format!("refused: {err}")));
                continue;
            }
        };
        let mut out = Vec::new();
        if let Err(err) = plan.write_to(&mut src, &mut out) {
            failures.push(fail(format!("write: {err}")));
            continue;
        }
        let Ok(after) = Gta6Mount::open_root(&mut Cursor::new(&out[..]), keys) else {
            failures.push(fail("the written archive does not reopen".into()));
            continue;
        };
        let new_row = after.archive.entry_extras()[e as usize].nested;
        let same_shape = new_row.is_some_and(|n| {
            (n.virtual_size, n.flags, n.resource_type)
                == (row.virtual_size, row.flags, row.resource_type)
        });
        let read_back = after.archive.entries[e as usize]
            .start
            .and_then(|s| sections_at(&out, s + SLOT));
        if !same_shape {
            failures.push(fail("the row states other sections or another type".into()));
        } else if read_back.as_ref() != Some(&before) {
            failures.push(fail("the resource does not read back".into()));
        }
        replaced += 1;
    }
    replaced
}

#[test]
fn a_headerless_entry_replaced_by_its_own_loose_form_reads_back() {
    let what = "GTA VI headerless replace over a build";
    let (Some(snapshot), Some(build)) = (env_path(GTA6_SNAPSHOT_ENV), env_path(BUILD_ENV)) else {
        skip_install(
            what,
            format_args!("set {GTA6_SNAPSHOT_ENV} and {BUILD_ENV}"),
        );
        return;
    };
    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
    let (mut replaced, mut failures) = (0usize, Vec::new());
    for (i, path) in build_archives(&build).iter().enumerate() {
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys).expect("the archive opens");
        let mut reads = 0;
        for entry in &mount.archive.entries {
            if reads >= NESTED_READS {
                break;
            }
            let (Some(Gta6Class::NestedArchive), Some(start), Some(stored)) =
                (entry.kind(), mount.absolute_start(entry), entry.stored)
            else {
                continue;
            };
            if stored > NESTED_CAP {
                continue;
            }
            let mut bytes = vec![0u8; stored as usize];
            if file.seek(SeekFrom::Start(start)).is_err() || file.read_exact(&mut bytes).is_err() {
                continue;
            }
            reads += 1;
            let what = format!("archive {i}, nested entry {}", entry.index);
            replaced += replace_headerless(&bytes, &keys, &what, &mut failures);
        }
    }
    println!(
        "headerless entries replaced {replaced}, failures {}",
        failures.len()
    );
    for failure in failures.iter().take(20) {
        eprintln!("  {failure}");
    }
    assert!(
        replaced > 0,
        "the build's nested tables hold clear headerless entries"
    );
    assert!(
        failures.is_empty(),
        "every replaced resource must read back"
    );
}
