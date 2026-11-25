//! Every expression set of a real build that writes facial controls (group D), emptied of them
//! and given back its first: a set that wrote one is its stored bytes again, and every other
//! writes that one alone. The sets are found by walking the build's archives, root first, and
//! stop at the first archive that holds any (every measured set lies in one archive).
//!
//! Install-gated and game-derived, so it skips cleanly without its env vars:
//!   `OPENRAGE_GTA6_KEYS`   the `gtavi_magic.dat` key snapshot (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

#![cfg(feature = "gta6")]

use std::fs::File;
use std::path::{Path, PathBuf};

use or_io::container::rpf::{
    read_rpf9_body, Gta6Class, Gta6Mount, GtaViKeys, Rpf9BodyKind, Rpf9Sections, Rpf9ZlibRun,
    GTA6_SNAPSHOT_ENV,
};
use or_io::gta6::anim::{
    add_expression_track, read_expressions, remove_expression_track, write_edited_expressions,
    ExpressionGroup, Expressions, EXPRESSIONS_SIGNATURE,
};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// The type code an expression set's 20-byte row states, which other dictionaries state too.
const DICTIONARY: u32 = 0x63;
/// A row storing more than this is not read: the largest set measured stores under 3 KiB.
const CAP: u64 = 64 << 10;
const MAX_DEPTH: usize = 8;
/// The 16-byte zero slot a headerless body begins with.
const SLOT: u64 = 16;
/// Measured on the build: the sets, those that write group D, and those of one D track.
const SETS: usize = 98;
const WRITING_D: usize = 7;
const ONE_D_TRACK: usize = 2;
const D: ExpressionGroup = ExpressionGroup::FacialOutputs;

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

/// Every `*.rpf` directly under `root`, sorted, then every `pack/*/*.rpf`, sorted.
fn build_archives(root: &Path) -> Vec<PathBuf> {
    let rpfs_in = |dir: &Path| {
        let mut out: Vec<PathBuf> = (std::fs::read_dir(dir).into_iter().flatten().flatten())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("rpf"))
            .collect();
        out.sort();
        out
    };
    let mut out = rpfs_in(root);
    let mut packs: Vec<PathBuf> = (std::fs::read_dir(root.join("pack"))
        .into_iter()
        .flatten()
        .flatten())
    .map(|e| e.path())
    .filter(|p| p.is_dir())
    .collect();
    packs.sort();
    for pack in packs {
        out.extend(rpfs_in(&pack));
    }
    out
}

/// The sections of every expression set in `mount` and the archives nested in it.
fn walk(
    file: &mut File,
    keys: &GtaViKeys,
    mount: &Gta6Mount,
    depth: usize,
    found: &mut Vec<Rpf9Sections>,
) {
    let extras = mount.archive.entry_extras();
    for (entry, extra) in mount.archive.entries.iter().zip(&extras) {
        if entry.kind() == Some(Gta6Class::NestedArchive) {
            if depth < MAX_DEPTH {
                if let Ok(child) = mount.open_nested(&mut *file, entry, keys) {
                    walk(file, keys, &child, depth + 1, found);
                }
            }
            continue;
        }
        let (None, Some(row), Some(start)) =
            (&entry.cipher, extra.nested, mount.absolute_start(entry))
        else {
            continue;
        };
        if row.resource_type != DICTIONARY || row.stored_size() > CAP {
            continue;
        }
        let Ok((find, bytes)) = read_rpf9_body(&mut *file, start + SLOT, CAP) else {
            continue;
        };
        let is_set = matches!(
            find.kind,
            Rpf9BodyKind::Zlib(Rpf9ZlibRun { signature: Some(s), .. })
                if s.to_string() == EXPRESSIONS_SIGNATURE
        );
        if let Some(sections) = Rpf9Sections::of(&find, &bytes).filter(|_| is_set) {
            found.push(sections);
        }
    }
}

/// `set` without every track of group D, last first.
fn emptied(set: &Expressions) -> Expressions {
    let mut out = set.clone();
    while let Some(last) = out.facial_outputs.tracks.len().checked_sub(1) {
        out = remove_expression_track(&out, D, last).expect("a stored D track is removed");
    }
    out
}

#[test]
fn a_set_emptied_of_its_d_tracks_regains_its_first_d_track_as_stored() {
    let what = "GTA VI expression sets regain their first D track";
    let (Some(snapshot), Some(build)) = (env_path(GTA6_SNAPSHOT_ENV), env_path(BUILD_ENV)) else {
        skip_install(
            what,
            format_args!("set {GTA6_SNAPSHOT_ENV} and {BUILD_ENV}"),
        );
        return;
    };
    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
    let mut sets = Vec::new();
    for path in build_archives(&build) {
        let mut file = File::open(&path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys).expect("the archive opens");
        walk(&mut file, &keys, &mount, 0, &mut sets);
        if !sets.is_empty() {
            break;
        }
    }
    assert_eq!(sets.len(), SETS, "the expression sets of the build");

    let (mut writing, mut regained, mut failures) = (0, 0, Vec::new());
    for (k, sections) in sets.iter().enumerate() {
        let stored = &sections.virtual_section;
        let physical = sections.physical.len();
        let set = read_expressions(stored).expect("a stored set reads");
        let Some(&(first, _)) = set.facial_outputs.tracks.first() else {
            continue;
        };
        writing += 1;
        let mut work = stored.clone();
        let none = emptied(&set);
        let again = write_edited_expressions(&mut work, physical, &none)
            .map_err(|e| format!("emptied: {e}"))
            .and_then(|()| add_expression_track(&none, D, 0, first).map_err(|e| e.to_string()))
            .and_then(|again| {
                write_edited_expressions(&mut work, physical, &again)
                    .map(|()| again)
                    .map_err(|e| format!("regained: {e}"))
            });
        let again = match again {
            Ok(again) => again,
            Err(e) => {
                failures.push(format!("set {k}: {e}"));
                continue;
            }
        };
        if read_expressions(&work).ok().as_ref() != Some(&again)
            || again.facial_outputs.tracks != set.facial_outputs.tracks[..1]
        {
            failures.push(format!(
                "set {k}: the first D track does not read back alone"
            ));
            continue;
        }
        if set.facial_outputs.tracks.len() == 1 {
            if again != set || work != *stored {
                failures.push(format!(
                    "set {k}: one D track regained is not the stored set"
                ));
                continue;
            }
            regained += 1;
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    assert_eq!(writing, WRITING_D, "the sets that write group D");
    assert_eq!(
        regained, ONE_D_TRACK,
        "the sets of one D track, regained as stored"
    );
}
