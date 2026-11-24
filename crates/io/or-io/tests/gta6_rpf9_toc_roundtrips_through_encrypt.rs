//! The RPF9 TOC encrypt (`Rpf9Builder`) is the exact inverse of the decrypt: for every archive of a
//! GTA VI build, decrypting the on-disk table of contents and rebuilding it reproduces the original
//! ciphertext byte for byte. An edited cell survives a rebuild and a re-decrypt.
//!
//! This is the honest write-side check: a *byte-preserving* round trip, and that a raw cell edit
//! encrypts and decrypts back intact — a property of the cipher, not a claim the game accepts an
//! edited archive. The running game refuses one: the type-6 descriptor is an RSA-shaped signature
//! over the plaintext TOC, verified at mount, and the `0xb6f5` content hash is unidentified. Only the
//! unedited, byte-identical rebuild is a supported claim.
//!
//! Install-gated and game-derived, so it skips cleanly without its env vars, which is why a clean
//! checkout (`just test-no-pack`) passes:
//!   `OPENRAGE_GTA6_KEYS`   the `gtavi_magic.dat` key snapshot (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

#![cfg(feature = "gta6")]

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use or_io::container::rpf::{
    GtaViKeys, Rpf9Builder, Rpf9Toc, GROUP_ENTRY_ROW, GROUP_ENTRY_ROW_20, GTA6_SNAPSHOT_ENV,
};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";

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

/// Only `[0, toc_end)`, which is all the cipher reads and all the builder writes.
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

#[test]
fn every_build_archive_toc_rebuilds_byte_identical_and_survives_a_cell_edit() {
    let what = "GTA VI RPF9 TOC encrypt round trip";
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
    let archives = build_archives(&build);
    assert!(!archives.is_empty(), "no .rpf under {}", build.display());

    let mut rebuilt = 0usize;
    let mut edit_checked = false;
    for path in &archives {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
        let prefix = read_toc_prefix(path);
        let toc = Rpf9Toc::decrypt(&prefix, &keys)
            .unwrap_or_else(|e| panic!("decrypt {}: {e}", path.display()));

        // The unedited TOC re-encrypts to the exact on-disk bytes.
        let out = Rpf9Builder::serialize(&toc, &keys)
            .unwrap_or_else(|e| panic!("serialize {}: {e}", path.display()));
        assert_eq!(out.len(), prefix.len(), "{stem}: rebuilt length differs");
        if out != prefix {
            let at = out
                .iter()
                .zip(&prefix)
                .position(|(a, b)| a != b)
                .unwrap_or(prefix.len());
            panic!(
                "{stem}: rebuilt TOC first differs at {at:#X}: ours {:#04X}, on-disk {:#04X}",
                out[at], prefix[at]
            );
        }
        rebuilt += 1;

        // On one archive, a real cell edit must encrypt and decrypt back intact.
        if !edit_checked {
            let row = toc
                .groups
                .iter()
                .find(|g| (g.name_hash, g.width) == (GROUP_ENTRY_ROW, 24))
                .or_else(|| {
                    toc.groups
                        .iter()
                        .find(|g| (g.name_hash, g.width) == (GROUP_ENTRY_ROW_20, 20))
                });
            if let Some(group) = row.filter(|g| g.count > 0) {
                let (hash, width) = (group.name_hash, usize::from(group.width));
                let start = group.data.start;
                let mut edited = toc.clone();
                let mut cell = toc.bytes[start..start + width].to_vec();
                cell[0] ^= 0xFF;
                edited
                    .set_cell(hash, 0, &cell)
                    .expect("set the edited cell");

                let out = Rpf9Builder::serialize(&edited, &keys).expect("serialize the edit");
                assert_ne!(out, prefix, "{stem}: an edited TOC should differ on disk");
                let back = Rpf9Toc::decrypt(&out, &keys).expect("decrypt the edited TOC");
                assert_eq!(
                    back.bytes, edited.bytes,
                    "{stem}: decrypt(serialize(edit)) must equal the edited plaintext"
                );
                assert_eq!(
                    &back.bytes[start..start + width],
                    &cell[..],
                    "{stem}: the edited cell did not survive"
                );
                edit_checked = true;
            }
        }
    }

    assert_eq!(rebuilt, archives.len(), "not every TOC rebuilt");
    assert!(edit_checked, "no archive had an entry-row column to edit");
    println!(
        "{rebuilt}/{} RPF9 TOCs rebuilt byte for byte",
        archives.len()
    );
}
