//! Every `EDUP` body in the tables of a real build's top-level archives reads its head, and the
//! count of them and their counts are the ones measured. The measurement found no such body in a
//! nested archive and none with a cipher record, so neither is walked here.
//!
//! Install-gated and game-derived, so it skips cleanly without its env vars:
//!   `OPENRAGE_GTA6_KEYS`   the `gtavi_magic.dat` key snapshot (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

#![cfg(feature = "gta6")]

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use flate2::read::ZlibDecoder;
use or_io::container::rpf::{Gta6Class, Gta6Mount, GtaViKeys, GTA6_SNAPSHOT_ENV};
use or_io::gta6::other_bodies::{EdupHead, OtherBody};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// Stored bytes read to inflate a body's first bytes.
const HEAD_STORED: u64 = 1024;
const HEAD: u64 = 8;
/// The `EDUP` bodies of the build's top-level tables, and their count words in ascending order.
const BODIES: usize = 5;
const COUNTS: [u32; BODIES] = [0, 0, 4, 33, 17_431];

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

/// Every `*.rpf` directly under `root`, then every `pack/*/*.rpf`, sorted.
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

/// The first bytes of the body at `at`: its first chunk inflated, or as stored.
fn head_at(src: &mut File, at: u64) -> Option<Vec<u8>> {
    let mut stored = Vec::new();
    src.seek(SeekFrom::Start(at)).ok()?;
    (&mut *src)
        .take(HEAD_STORED)
        .read_to_end(&mut stored)
        .ok()?;
    let mut head = Vec::new();
    // A chunk cut short by the read still yields its first bytes before the error.
    let _ = ZlibDecoder::new(&stored[..])
        .take(HEAD)
        .read_to_end(&mut head);
    if head.is_empty() {
        stored.truncate(HEAD as usize);
        return Some(stored);
    }
    Some(head)
}

#[test]
fn every_top_level_edup_body_of_the_build_reads_its_head() {
    let what = "GTA VI EDUP heads over a build";
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
    let mut heads = Vec::new();
    let mut failures = Vec::new();
    for path in &archives {
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys).expect("the archive opens");
        for entry in &mount.archive.entries {
            if entry.kind() == Some(Gta6Class::NestedArchive) || entry.cipher.is_some() {
                continue;
            }
            let Some(start) = mount.absolute_start(entry) else {
                continue;
            };
            let named = head_at(&mut file, start)
                .is_some_and(|h| h.len() == HEAD as usize && h.starts_with(b"EDUP"));
            if !named {
                continue;
            }
            let where_ = format!("entry {} of {}", entry.index, path.display());
            let body = match mount.read_entry(&mut file, entry) {
                Ok(body) => body,
                Err(e) => {
                    failures.push(format!("{where_}: the body does not read: {e}"));
                    continue;
                }
            };
            if OtherBody::of(&body, body.len() as u64) != Some(OtherBody::Edup) {
                failures.push(format!("{where_}: the scan does not name it EDUP"));
            }
            match EdupHead::open(&body) {
                Ok(head) => heads.push(head),
                Err(e) => failures.push(format!("{where_}: {e}")),
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    let mut counts: Vec<u32> = heads.iter().map(|h| h.count).collect();
    counts.sort_unstable();
    println!("{} EDUP bodies, counts {counts:?}", heads.len());
    assert_eq!(heads.len(), BODIES, "the EDUP bodies of the build");
    assert_eq!(counts, COUNTS, "their count words");
    for head in &heads {
        assert_eq!(head.stated_len, (head.count == 0).then_some(16), "{head:?}");
    }
}
