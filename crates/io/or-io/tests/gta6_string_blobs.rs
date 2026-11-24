//! The GTA VI PSO string cipher (type `0x13`, white-box `504f865e`) deciphers the enciphered string
//! runs of a build's `LSNS`/`STNS` sections: each enciphered run (category 1 or 0x1e) decrypts to a
//! sequence of NUL-terminated printable strings that fill its length to the zero pad. This proves the
//! representation, the ported block transform and the CBC mode against the game. The raw (still
//! enciphered) runs are high-entropy, and deciphering a run twice scrambles it, so the check cannot
//! pass on noise.
//!
//! Install-gated and game-derived: it needs the key snapshot carrying the string section
//! (`OPENRAGE_GTA6_KEYS`) and the build, and skips cleanly without them, which is why a clean checkout
//! (`just test-no-pack`) passes. It walks nested archives, so it belongs in the slow tier.
//!   `OPENRAGE_GTA6_KEYS`   the `gtavi_magic.dat` snapshot with the string section (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

#![cfg(all(feature = "gta6", feature = "meta"))]

use std::fs::File;
use std::path::{Path, PathBuf};

use or_io::container::rpf::{Gta6Mount, GtaViKeys, GTA6_SNAPSHOT_ENV};
use or_io::meta::binary::pso::string_blob;
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
const CONTENT_CIPHER_TYPE: u16 = 17;
/// Stop once this many enciphered runs are decoded; enough to prove the cipher without walking the
/// whole multi-gigabyte build.
const RUN_TARGET: usize = 300;
const MAX_DEPTH: usize = 8;
/// PSO string tables are small; a content entry whose inflated size is larger is skipped, so the
/// sweep does not decrypt and inflate gigabytes of textures and models before finding its runs.
const MAX_ENTRY: u64 = 4 << 20;

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

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

/// Big-endian PSO section frames: `[u32 ident][i32 len incl 8][body…]`. Returns `(4cc, body)` pairs.
fn sections(doc: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut p = 0usize;
    while p + 8 <= doc.len() {
        let ident = [doc[p], doc[p + 1], doc[p + 2], doc[p + 3]];
        let len = i32::from_be_bytes([doc[p + 4], doc[p + 5], doc[p + 6], doc[p + 7]]);
        if len < 8 || p + len as usize > doc.len() {
            break;
        }
        out.push((ident, &doc[p + 8..p + len as usize]));
        p += len as usize;
    }
    out
}

/// Whether a byte is printable ASCII or ordinary whitespace (not NUL).
fn printable(b: u8) -> bool {
    b == b'\t' || b == b'\n' || b == b'\r' || (0x20u8..0x7f).contains(&b)
}

/// A deciphered run matches the grammar `(printable+ NUL)+ NUL*`: one or more non-empty
/// NUL-terminated printable strings, then a zero pad. No printable byte may follow the first double
/// NUL and the pad is exactly zero, so an IV near-miss that turned a pad NUL into a space is caught.
fn is_well_formed_run(data: &[u8]) -> bool {
    if data.is_empty() || *data.last().unwrap() != 0 {
        return false;
    }
    let mut pad = false; // reached the trailing zero pad (seen a double NUL)
    let mut prev_nul = true; // a leading NUL would be an empty first string
    for &b in data {
        if b == 0 {
            if prev_nul {
                pad = true;
            }
            prev_nul = true;
        } else {
            if pad || !printable(b) {
                return false;
            }
            prev_nul = false;
        }
    }
    true
}

#[derive(Default)]
struct Tally {
    runs: usize,
    enciphered_raw_was_noise: usize,
    double_decipher_broke: bool,
}

/// Decipher the `LSNS`/`STNS` sections of a PSO document and check every enciphered run.
fn check_doc(doc: &[u8], keys: &GtaViKeys, t: &mut Tally) {
    if doc.len() < 4 || &doc[0..4] != b"PSIN" {
        return;
    }
    for (ident, body) in sections(doc) {
        if &ident != b"LSNS" && &ident != b"STNS" {
            continue;
        }
        let Ok(runs) = string_blob::parse(body) else {
            continue;
        };
        if !runs.iter().any(|r| r.enciphered) {
            continue;
        }
        let mut deciphered = body.to_vec();
        if string_blob::decipher_blob(&mut deciphered, keys).is_err() {
            continue;
        }
        for run in &runs {
            if !run.enciphered {
                continue;
            }
            let whole = (run.len as usize) & !0xF;
            if whole == 0 {
                continue;
            }
            let plain = &deciphered[run.off..run.off + run.len as usize];
            assert!(
                is_well_formed_run(plain),
                "an enciphered run (id {:#x}) did not decipher to NUL-terminated strings: {:02x?}",
                run.id,
                &plain[..plain.len().min(32)]
            );
            // The raw run was high-entropy (the cipher did real work), and deciphering the deciphered
            // bytes again scrambles them (CBC decrypt is not an involution) — so the check is not
            // passing on already-clear or trivially-transformed data.
            let raw = &body[run.off..run.off + run.len as usize];
            if !is_well_formed_run(raw) {
                t.enciphered_raw_was_noise += 1;
            }
            if !t.double_decipher_broke {
                let mut again = deciphered[run.off..run.off + whole].to_vec();
                keys.string_decrypt(&mut again)
                    .expect("string key decrypts");
                t.double_decipher_broke = !again.iter().all(|&b| b == 0 || printable(b));
            }
            t.runs += 1;
        }
        if t.runs >= RUN_TARGET {
            return;
        }
    }
}

fn walk(file: &mut File, mount: &Gta6Mount, keys: &GtaViKeys, depth: usize, t: &mut Tally) {
    if depth > MAX_DEPTH || t.runs >= RUN_TARGET {
        return;
    }
    for entry in &mount.archive.entries {
        if t.runs >= RUN_TARGET {
            return;
        }
        match entry.cipher {
            Some(cipher) if cipher.cipher_type == CONTENT_CIPHER_TYPE => {
                if entry.real.is_some_and(|r| r > MAX_ENTRY) {
                    continue;
                }
                if let Ok(doc) = mount.read_entry_decrypted(&mut *file, entry, keys) {
                    check_doc(&doc, keys, t);
                }
            }
            Some(_) => {}
            None => {
                // A clear entry may be a nested archive; recurse when its body begins one.
                if entry.start.is_some()
                    && mount
                        .entry_begins_archive(&mut *file, entry)
                        .unwrap_or(false)
                {
                    if let Ok(child) = mount.open_nested(&mut *file, entry, keys) {
                        walk(file, &child, keys, depth + 1, t);
                    }
                }
            }
        }
    }
}

#[test]
fn enciphered_string_runs_decipher_to_strings() {
    let what = "GTA VI PSO string cipher (type 0x13) over a build";
    let (Some(snapshot), Some(build)) = (env_path(GTA6_SNAPSHOT_ENV), env_path(BUILD_ENV)) else {
        skip_install(
            what,
            format_args!("set {GTA6_SNAPSHOT_ENV} (with the string section) and {BUILD_ENV}"),
        );
        return;
    };
    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
    if !keys.has_string_keys() {
        skip_install(
            what,
            format_args!(
                "the snapshot at {GTA6_SNAPSHOT_ENV} carries no string section; rebuild it"
            ),
        );
        return;
    }

    let archives = build_archives(&build);
    assert!(!archives.is_empty(), "no .rpf under {}", build.display());

    let mut t = Tally::default();
    for path in &archives {
        if t.runs >= RUN_TARGET {
            break;
        }
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys)
            .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
        walk(&mut file, &mount, &keys, 0, &mut t);
    }

    println!(
        "string cipher: {} enciphered runs deciphered to strings ({} raw runs were noise)",
        t.runs, t.enciphered_raw_was_noise
    );
    // Every run met was asserted well formed in `check_doc`. The content-ciphered PSO documents of the
    // build hold few enciphered runs: the walk meets 11.
    assert!(
        t.runs >= 11,
        "the walk met only {} enciphered string runs; the build holds 11",
        t.runs
    );
    assert_eq!(
        t.enciphered_raw_was_noise, t.runs,
        "every enciphered run was noise before deciphering; the cipher did real work"
    );
    assert!(
        t.double_decipher_broke,
        "deciphering a deciphered run again should scramble it, proving the key matters"
    );
}
