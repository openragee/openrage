//! What `RpfArchive::rpf9_deflated` says of a GTA VI row is what its bytes are: a row it calls
//! stored holds its body as it reads, padded to 16, and a row it calls deflated does not.
//!
//! Install-gated: it skips cleanly without the keys and the build.
//!   `OPENRAGE_GTA6_KEYS`   the key snapshot, or `OPENRAGE_GTA6_EXE` for the bundled one
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

#![cfg(feature = "gta6")]

use std::path::{Path, PathBuf};

use or_io::container::rpf::{BodyForm, GtaViKeys, RpfArchive, GTA6_EXE_ENV, GTA6_SNAPSHOT_ENV};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// A body larger than this is not read: the check is the label, not the volume.
const BODY_CAP: u64 = 1 << 20;
/// How many rows of one archive are read.
const ROWS: usize = 40;

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

#[derive(Default)]
struct Tally {
    stored: usize,
    /// Stored rows whose table states a size past the real one: the padding to 16.
    padded: usize,
    deflated: usize,
    unread: usize,
    failures: Vec<String>,
}

fn check(archive: &RpfArchive, at: &str, t: &mut Tally) {
    let mut read = 0;
    for node in archive.nodes.iter().filter(|n| !n.is_dir()) {
        let index = node.entry_index();
        let Some(row) = archive.rpf9_row(index) else {
            continue;
        };
        // A clear 24-byte row's body starts at its row; a nested archive is not a body.
        if row.ciphered || row.row_bytes != Some(24) || row.real > BODY_CAP {
            continue;
        }
        if node.name().ends_with(".rpf") || read == ROWS {
            continue;
        }
        read += 1;
        let Some(deflated) = archive.rpf9_deflated(index) else {
            t.failures.push(format!("{at}: entry {index} has no word"));
            continue;
        };
        let (Ok(held), Ok(body)) = (
            archive.read_form(index, BodyForm::AsStored),
            archive.read_entry(index),
        ) else {
            t.unread += 1;
            continue;
        };
        let head = held.get(..body.len());
        match deflated {
            false if head == Some(&body[..]) => {
                t.stored += 1;
                t.padded += usize::from(row.stored != row.real);
            }
            true if head != Some(&body[..]) => t.deflated += 1,
            _ => t.failures.push(format!(
                "{at}: entry {index}, stored {} real {}, is called {} against its bytes",
                row.stored,
                row.real,
                if deflated { "deflated" } else { "stored" }
            )),
        }
    }
}

#[test]
fn a_row_called_stored_holds_its_body_and_a_row_called_deflated_does_not() {
    let what = "the stored or deflated word of a GTA VI build's rows";
    let build = std::env::var_os(BUILD_ENV)
        .map(PathBuf::from)
        .filter(|p| p.is_dir());
    let (Some(build), Ok(Some(_))) = (build, GtaViKeys::for_install(None)) else {
        skip_install(
            what,
            format_args!("set {BUILD_ENV} and {GTA6_SNAPSHOT_ENV} (or {GTA6_EXE_ENV})"),
        );
        return;
    };
    let archives = build_archives(&build);
    assert!(!archives.is_empty(), "no .rpf under {}", build.display());

    let mut t = Tally::default();
    for path in &archives {
        match RpfArchive::open(path) {
            Ok(archive) => check(&archive, &path.display().to_string(), &mut t),
            Err(e) => t.failures.push(format!("{}: {e}", path.display())),
        }
    }
    println!(
        "{} stored ({} padded to 16), {} deflated, {} unread, {} failures",
        t.stored,
        t.padded,
        t.deflated,
        t.unread,
        t.failures.len()
    );
    for failure in t.failures.iter().take(20) {
        eprintln!("  {failure}");
    }
    assert!(t.failures.is_empty(), "every row's word matches its bytes");
    assert!(t.padded > 0, "a stored row padded to 16 was read");
    assert!(t.deflated > 0, "a deflated row was read");
}
