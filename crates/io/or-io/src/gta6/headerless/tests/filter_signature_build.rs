//! Install-gated: every frame filter of a build states at 0x0C the signature its elements give.
//!   `OPENRAGE_GTA6_KEYS`   the `gtavi_magic.dat` key snapshot (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

use std::fs::File;
use std::path::{Path, PathBuf};

use or_test_support::skip_install;

use crate::container::rpf::{
    read_rpf9_body, Gta6Class, Gta6Mount, GtaViKeys, Rpf9Sections, GTA6_SNAPSHOT_ENV,
};
use crate::gta6::headerless::write_filter_tracks::filter_signatures;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// The resource type of a 20-byte row the game registers frame filter dictionaries for.
const FILTER_DICTIONARY: u32 = 0x56;
/// Rows of that type the build holds, and the frame filters in them, measured by a mirror of the
/// rule over the build's extracted dictionaries.
const DICTIONARIES: usize = 11;
const FILTERS: usize = 311;
/// The 16-byte zero slot a headerless body begins with.
const SLOT: u64 = 16;
const CAP: u64 = 4 << 20;
const MAX_DEPTH: usize = 8;

#[derive(Default)]
struct Tally {
    /// Rows of the type met, read or not.
    rows: usize,
    dictionaries: usize,
    filters: usize,
    holding: usize,
    /// Rows not read, and why.
    unread: Vec<String>,
    misses: Vec<String>,
}

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

fn visit(file: &mut File, keys: &GtaViKeys, mount: &Gta6Mount, depth: usize, t: &mut Tally) {
    let extras = mount.archive.entry_extras();
    for (entry, extra) in mount.archive.entries.iter().zip(&extras) {
        if t.rows == DICTIONARIES {
            return;
        }
        if entry.kind() == Some(Gta6Class::NestedArchive) {
            if depth < MAX_DEPTH {
                if let Ok(child) = mount.open_nested(&mut *file, entry, keys) {
                    visit(file, keys, &child, depth + 1, t);
                }
            }
            continue;
        }
        let (Some(start), Some(row)) = (mount.absolute_start(entry), extra.nested) else {
            continue;
        };
        if row.resource_type != FILTER_DICTIONARY {
            continue;
        }
        t.rows += 1;
        if entry.cipher.is_some() {
            t.unread.push(format!("entry {}: enciphered", entry.index));
            continue;
        }
        let sections = read_rpf9_body(&mut *file, start + SLOT, CAP)
            .ok()
            .and_then(|(find, bytes)| Rpf9Sections::of(&find, &bytes));
        let Some(sections) = sections else {
            t.unread.push(format!("entry {}: no sections", entry.index));
            continue;
        };
        let filters = match filter_signatures(&sections.virtual_section) {
            Ok(filters) => filters,
            Err(e) => {
                t.unread.push(format!("entry {}: {e}", entry.index));
                continue;
            }
        };
        t.dictionaries += 1;
        for (code, stored, computed) in filters {
            t.filters += 1;
            if stored == computed {
                t.holding += 1;
            } else {
                t.misses.push(format!(
                    "entry {} filter {code:#010X}: states {stored:#010X}, its elements give {computed:#010X}",
                    entry.index
                ));
            }
        }
    }
}

/// Measured first by a node mirror of the rule: 311 of 311 filters of the build's 11 frame filter
/// dictionaries hold, the empty ones stating 0.
#[test]
fn every_frame_filter_of_the_build_states_the_signature_its_elements_give() {
    let what = "GTA VI frame filter signatures over a build";
    let (Some(snapshot), Some(build)) = (env_path(GTA6_SNAPSHOT_ENV), env_path(BUILD_ENV)) else {
        skip_install(
            what,
            format_args!("set {GTA6_SNAPSHOT_ENV} and {BUILD_ENV}"),
        );
        return;
    };
    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
    let mut t = Tally::default();
    for path in build_archives(&build) {
        if t.rows == DICTIONARIES {
            break;
        }
        let mut file = File::open(&path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys).expect("the archive opens");
        visit(&mut file, &keys, &mount, 0, &mut t);
    }
    println!(
        "{} rows, {} dictionaries read, {} filters, {} holding",
        t.rows, t.dictionaries, t.filters, t.holding
    );
    assert!(t.unread.is_empty(), "{:?}", t.unread);
    assert!(
        t.misses.is_empty(),
        "{:?}",
        &t.misses[..t.misses.len().min(20)]
    );
    assert_eq!(
        (t.rows, t.dictionaries, t.filters, t.holding),
        (DICTIONARIES, DICTIONARIES, FILTERS, FILTERS)
    );
}
