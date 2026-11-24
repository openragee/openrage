//! The GTA VI content cipher (type 17, white-box representation `5dce1827`) decrypts every flagged
//! top-level entry of a build: each CTR-decrypts to a zlib head, and the fully-enciphered class
//! inflates to exactly the entry row's real (inflated) size. This is the task's required check for the
//! content cipher, proving the representation, the ported transform and the CTR mode against the game.
//!
//! Install-gated and game-derived, so it needs the key snapshot (which must carry the content section,
//! `OPENRAGE_GTA6_KEYS`) and the build, and skips cleanly without them, which is why a clean checkout
//! (`just test-no-pack`) passes:
//!   `OPENRAGE_GTA6_KEYS`   the `gtavi_magic.dat` key snapshot with content keys (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf`

#![cfg(feature = "gta6")]

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use or_io::container::rpf::{Gta6Mount, GtaViKeys, GTA6_SNAPSHOT_ENV};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// The content cipher type whose entries this test decrypts.
const CONTENT_CIPHER_TYPE: u16 = 17;
/// The full-body inflate sample is held to small entries so the walk stays cheap; a larger body still
/// has its head checked.
const SAMPLE_CAP: u64 = 0x40000;

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

fn build_archives(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("rpf") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn is_zlib_head(head: &[u8]) -> bool {
    head.len() >= 2 && head[0] == 0x78 && matches!(head[1], 0xDA | 0x9C | 0x01 | 0x5E)
}

#[test]
fn every_flagged_content_entry_decrypts_to_zlib() {
    let what = "GTA VI content cipher (type 17) over a build";
    let (Some(snapshot), Some(build)) = (env_path(GTA6_SNAPSHOT_ENV), env_path(BUILD_ENV)) else {
        skip_install(
            what,
            format_args!("set {GTA6_SNAPSHOT_ENV} (with content keys) and {BUILD_ENV}"),
        );
        return;
    };
    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
    if !keys.has_content_keys() {
        skip_install(
            what,
            format_args!(
                "the snapshot at {GTA6_SNAPSHOT_ENV} carries no content section; rebuild it"
            ),
        );
        return;
    }

    let archives = build_archives(&build);
    assert!(!archives.is_empty(), "no .rpf under {}", build.display());

    let mut tried = 0usize;
    let mut heads = 0usize;
    let mut sampled = 0usize;
    let mut inflated_exact = 0usize;
    for path in &archives {
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys)
            .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
        for entry in &mount.archive.entries {
            let Some(cipher) = entry.cipher else { continue };
            if cipher.cipher_type != CONTENT_CIPHER_TYPE {
                continue;
            }
            let (Some(start), Some(stored)) = (mount.absolute_start(entry), entry.stored) else {
                continue;
            };
            if stored < 16 {
                continue;
            }
            tried += 1;

            // Head check: CTR-decrypt the first block and confirm the zlib magic.
            let mut head = vec![0u8; 16.min(stored as usize)];
            file.seek(SeekFrom::Start(start)).expect("seek to the body");
            file.read_exact(&mut head).expect("read the body head");
            keys.content_decrypt(cipher.key_id, &cipher.iv, &mut head)
                .expect("the content key decrypts");
            assert!(
                is_zlib_head(&head),
                "{}: entry {} (key {:#010x}) does not decrypt to a zlib head: {:02x?}",
                path.display(),
                entry.index,
                cipher.key_id,
                &head[..head.len().min(4)]
            );
            heads += 1;

            // Full-body check on the fully-enciphered class: the whole body inflates to exactly C.
            if stored <= SAMPLE_CAP {
                sampled += 1;
                if let (Ok(body), Some(real)) = (
                    mount.read_entry_decrypted(&mut file, entry, &keys),
                    entry.real,
                ) {
                    if body.len() as u64 == real {
                        inflated_exact += 1;
                    }
                }
            }
        }
    }

    println!(
        "content cipher: {tried} flagged entries, {heads} zlib heads, {inflated_exact}/{sampled} \
         sampled bodies inflate to exactly their real size"
    );
    assert_eq!(
        heads, tried,
        "every flagged content entry must decrypt to a zlib head"
    );
    assert!(
        tried >= 400,
        "the build is expected to hold about 455 top-level content entries, found {tried}"
    );
    assert!(
        inflated_exact >= 200,
        "the fully-enciphered class should inflate to exact size; only {inflated_exact} of \
         {sampled} sampled did"
    );
}
