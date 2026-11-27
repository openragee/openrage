//! `glob("**/*.ydr")` reaches into nested archives, from the library.
//!
//! The fixture is **built here** — a real RPF holding a real RPF holding a
//! real RPF — so the test needs no corpus and runs on a clean checkout.

use std::sync::Arc;

use or_io::container::rpf::{RpfArchive, RpfBuilder};
use or_title::vfs::{ArchiveDevice, FileDevice, Vfs, VfsPath};

/// An archive holding the given `(path, bytes)` pairs.
fn archive_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = RpfBuilder::default();
    for (path, bytes) in entries {
        builder
            .insert_with(path, *bytes, false)
            .unwrap_or_else(|e| panic!("add {path}: {e}"));
    }
    builder.to_bytes().expect("save")
}

/// `outer.rpf` → `mid.rpf` → `inner.rpf`, with a `.ydr` at every level.
fn four_levels() -> Vec<u8> {
    let inner = archive_of(&[
        ("deep.ydr", b"level-three"),
        ("deep.ytd", b"not-a-drawable"),
    ]);
    let mid = archive_of(&[("mid.ydr", b"level-two"), ("inner.rpf", &inner)]);
    archive_of(&[
        ("top.ydr", b"level-one"),
        ("data/nested/also.ydr", b"level-one-deeper"),
        ("mid.rpf", &mid),
    ])
}

fn mounted() -> Vfs {
    let archive = RpfArchive::from_bytes(four_levels()).expect("the built archive opens");
    let device = ArchiveDevice::new(Arc::new(archive));
    let mut vfs = Vfs::new();
    vfs.mount("game:", Arc::new(device));
    vfs
}

#[test]
fn glob_reaches_a_drawable_at_every_depth() {
    let vfs = mounted();
    let found = vfs.glob("**/*.ydr");

    let mut paths = found.matches.clone();
    paths.sort();
    eprintln!("\n=== glob(\"**/*.ydr\") across three archive boundaries ===");
    for path in &paths {
        eprintln!("  {path}");
    }

    assert!(
        found.is_complete(),
        "a mount could not be walked, so this list is not exhaustive: {:?}",
        found.unwalkable
    );

    for wanted in [
        "game:/top.ydr",
        "game:/data/nested/also.ydr",
        "game:/mid.rpf/mid.ydr",
        "game:/mid.rpf/inner.rpf/deep.ydr",
    ] {
        assert!(
            paths.iter().any(|p| p == wanted),
            "{wanted} is not in {paths:?}"
        );
    }
    assert!(
        !paths.iter().any(|p| p.ends_with(".ytd")),
        "the pattern matched something that is not a drawable: {paths:?}"
    );
}

/// ⚠ Without this, a `glob` that returned every path it walked would pass the
/// test above — the wanted four are a subset of everything.
#[test]
fn a_pattern_that_matches_nothing_returns_nothing() {
    let vfs = mounted();
    assert!(
        vfs.glob("**/*.ybn").matches.is_empty(),
        "the fixture holds no .ybn"
    );
    assert!(
        vfs.glob("nowhere/**/*.ydr").matches.is_empty(),
        "no path in the fixture begins with `nowhere/`"
    );
}

/// A single `*` stops at a separator, so depth is something a caller chooses.
///
/// `*.ydr` here is a path, *directly under the mount root*; to
/// `openrage-cli search` it is a leaf, matching at any depth.
#[test]
fn one_star_stays_in_its_directory_and_two_do_not() {
    let vfs = mounted();

    let shallow = vfs.glob("*.ydr").matches;
    assert_eq!(
        shallow,
        vec!["game:/top.ydr"],
        "`*.ydr` must not cross a separator, nor an archive boundary"
    );

    let subdir = vfs.glob("data/*/*.ydr").matches;
    assert_eq!(
        subdir,
        vec!["game:/data/nested/also.ydr"],
        "a fixed segment then one wildcard segment"
    );

    let inside = vfs.glob("mid.rpf/**/*.ydr").matches;
    assert_eq!(
        inside.len(),
        2,
        "everything under one named archive, at any depth: {inside:?}"
    );
}

/// The path type answers *where* a match is without anybody parsing a string.
#[test]
fn every_match_says_whether_it_is_inside_an_archive() {
    let vfs = mounted();
    let mut inside = 0usize;
    let mut loose = 0usize;
    let mut deepest = 0usize;

    for hit in vfs.glob("**/*.ydr").matches {
        let path = VfsPath::parse(&hit).expect("a glob result parses as a path");
        deepest = deepest.max(path.archive_depth());
        if path.is_inside_archive() {
            inside += 1;
        } else {
            loose += 1;
        }
    }

    assert_eq!(loose, 2, "two drawables sit outside any nested archive");
    assert_eq!(inside, 2, "two sit inside one");
    assert_eq!(deepest, 2, "the deepest crossed two boundaries");
}

/// The `.rpf` entries are listed as well as descended into.
///
/// ⚠ A walk that only descended would lose the archive from the listing,
/// which is the one entry an explorer most needs to show.
#[test]
fn an_archive_is_both_an_entry_and_a_boundary() {
    let vfs = mounted();
    let archives = vfs.glob("**/*.rpf").matches;
    assert_eq!(
        archives,
        vec!["game:/mid.rpf", "game:/mid.rpf/inner.rpf"],
        "both nested archives are entries in their own right"
    );

    for hit in &archives {
        let path = VfsPath::parse(hit).expect("parses");
        assert!(path.is_archive(), "{hit} names an archive");
    }
    let inner = VfsPath::parse(&archives[1]).expect("parses");
    assert!(
        inner.is_archive() && inner.is_inside_archive(),
        "a nested archive is both, and a caller treating them as exclusive \
         gets exactly this case wrong"
    );
}

/// Nested archives are opened in memory, never extracted to a temp file.
#[test]
fn a_nested_archive_costs_no_temp_file() {
    // ⚠ The shared temp directory is written by every test process nextest runs
    // beside this one, so counting it fails at random; count a private one.
    let private = tempfile::tempdir().expect("a private temp directory");
    for var in ["TMP", "TEMP", "TMPDIR"] {
        std::env::set_var(var, private.path());
    }
    assert_eq!(std::env::temp_dir(), private.path());

    let before = temp_files();
    let vfs = mounted();
    let bytes = vfs
        .open("game:/mid.rpf/inner.rpf/deep.ydr")
        .expect("three boundaries deep");
    assert_eq!(bytes, b"level-three");
    let after = temp_files();

    assert_eq!(
        after, before,
        "opening through three nested archives changed the temp directory's \
         file count, so something extracted to disk on the way"
    );
}

fn temp_files() -> usize {
    std::fs::read_dir(std::env::temp_dir())
        .map(|d| d.flatten().count())
        .unwrap_or(0)
}

/// Which app files reach this VFS, and which still walk archives around it.
///
/// The RPF reader is shared; what is duplicated is the traversal.
///
/// `state/clip_regions.rs` calls `build_vfs_for_install_with` to fetch a
/// single table. `state/name_index.rs` mounts the game and walks every entry
/// for its names, and `state/reference_names.rs` mounts it to read every
/// `.ytyp` for its interiors. `state/dependencies.rs` mounts it to read what
/// a row's Uses and Placed by answer from, as `openrage-cli depends` does, and
/// asks the mount where each file it names sits in the install's tree.
/// `state/vfs_session.rs` mounts the *open document* over the
/// `Arc<RpfArchive>` the explorer already holds, so no second open
/// happens, and resolves the hierarchy's glob into nested archives the tree
/// has never expanded. `state/expander.rs` lists an archive through
/// [`ArchiveDevice::entries`] off the UI thread, and `state/rpf_session.rs`
/// fills the explorer's tree from that listing: each node takes its entry
/// index and, for a nested archive, the opened handle. The session reads an
/// entry's bytes and opens a nested archive the listing could not through
/// [`ArchiveDevice::read_entry`] and [`ArchiveDevice::open_nested`], and
/// `state/extract_job.rs` extracts through the same device; an override
/// slot is the session's own. `state/compare/mod.rs` reads the game's own copy
/// of a file in the mods overlay through an `ArchiveDevice` over the
/// install's archive, and `state/compare/archive.rs` lists the entries of two
/// archives, a nested one among them, and reads each both hold through the
/// same device. `photo.rs` matches names in an install's archives for the garments it draws, `state/workbench/dressed.rs` and `state/workbench/dressed/read.rs` mount the game to dress a ped, `state/workbench/game_collections.rs` mounts it with every DLC to list the collections a pack may replace, and `state/workbench/imports.rs` tells an archive segment of a path from a folder.
#[test]
fn the_app_still_walks_archives_without_this_vfs() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("the workspace root is three above or-title")
        .join("src");

    let mut naming = Vec::new();
    let mut files = Vec::new();
    collect_rs(&root, &mut files);
    assert!(
        files.len() > 50,
        "the walk found {} file(s) under src/, which is not the app",
        files.len()
    );

    for file in &files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        if text.contains("or_title::vfs")
            || text.contains("Vfs::")
            || text.contains("build_vfs_for_install")
            || text.contains("MountedGame")
        {
            naming.push(
                file.strip_prefix(&root)
                    .unwrap_or(file)
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    naming.sort();

    eprintln!("\n=== app files that reach the VFS: {naming:?} ===");
    assert_eq!(
        naming,
        vec![
            "photo.rs",
            "state/clip_regions.rs",
            "state/compare/archive.rs",
            "state/compare/mod.rs",
            "state/dependencies.rs",
            "state/expander.rs",
            "state/extract_job.rs",
            "state/name_index.rs",
            "state/reference_names.rs",
            "state/retail.rs",
            "state/rpf_session.rs",
            "state/the_panel_counts_the_apps_cpu_state.rs",
            "state/vfs_session.rs",
            "state/workbench/dressed.rs",
            "state/workbench/dressed/read.rs",
            "state/workbench/game_collections.rs",
            "state/workbench/imports.rs",
        ],
        "the set of app files using the VFS has changed.\n\n\
         If it **grew**, R19's residue is shrinking and this row should record \
         the new set. If it **shrank to nothing**, the app has stopped using \
         the VFS entirely and the risk R19 names has got worse rather than \
         better."
    );
}

fn collect_rs(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Dropping the cached handles is something a caller can ask for.
///
/// ⚠ An archive handle held past an edit serves the bytes from before the
/// edit.
#[test]
fn a_cached_archive_can_be_dropped_on_request() {
    let archive = RpfArchive::from_bytes(four_levels()).expect("opens");
    let device = ArchiveDevice::new(Arc::new(archive));

    assert_eq!(device.nested_cache_len(), 0, "nothing open yet");
    device
        .open("mid.rpf/inner.rpf/deep.ydr")
        .expect("reaches the deepest file");
    assert!(
        device.nested_cache_len() > 0,
        "the descent opened nested archives and kept none of them"
    );

    device.invalidate();
    assert_eq!(device.nested_cache_len(), 0, "invalidate drops them");

    assert_eq!(
        device.open("mid.rpf/inner.rpf/deep.ydr").as_deref(),
        Some(&b"level-three"[..]),
        "and the next read re-opens rather than failing"
    );
}
