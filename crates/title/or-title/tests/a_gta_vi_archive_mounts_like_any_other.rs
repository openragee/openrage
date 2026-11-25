//! A GTA VI archive goes through the device every archive goes through: its root lists, a nested
//! archive lists as a folder under its own name, and a file inside it opens by its path.
//!
//! Install-gated: it skips cleanly without the keys and the build.
//!   `OPENRAGE_GTA6_KEYS`   the key snapshot, or `OPENRAGE_GTA6_EXE` for the bundled one
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf`

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use or_io::container::rpf::{GtaViKeys, RpfArchive, GTA6_EXE_ENV, GTA6_SNAPSHOT_ENV};
use or_test_support::skip_install;
use or_title::vfs::{ArchiveDevice, ArchiveEntry, ArchiveEntryKind, FileDevice};

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";

/// The path of the first entry under `dir`, folders descended, that `wanted` accepts.
fn find(
    device: &ArchiveDevice,
    dir: &str,
    depth: usize,
    wanted: &dyn Fn(&ArchiveEntry) -> bool,
) -> Option<String> {
    let entries = device.entries(dir)?;
    let join = |name: &str| match dir {
        "" => name.to_owned(),
        dir => format!("{dir}/{name}"),
    };
    if let Some(hit) = entries.iter().find(|e| wanted(e)) {
        return Some(join(&hit.name));
    }
    if depth == 0 {
        return None;
    }
    (entries.iter())
        .filter(|e| e.kind == ArchiveEntryKind::Folder)
        .find_map(|e| find(device, &join(&e.name), depth - 1, wanted))
}

#[test]
fn a_nested_archive_lists_as_a_folder_and_a_file_in_it_opens_by_its_path() {
    let what = "a GTA VI archive through the archive device";
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
    let mut archives: Vec<PathBuf> = (std::fs::read_dir(&build).into_iter().flatten().flatten())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("rpf"))
        .collect();
    archives.sort();
    assert!(!archives.is_empty(), "no .rpf under {}", build.display());

    let (mut listed, mut nested_listed, mut opened) = (0, 0, 0);
    for path in &archives {
        let archive = RpfArchive::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let device = ArchiveDevice::new(Arc::new(archive));
        let started = Instant::now();
        let root = device.entries("").expect("the root lists");
        let nested = root.iter().filter(|e| e.archive.is_some()).count();
        println!(
            "root of archive {listed}: {} entries, {nested} nested archives opened, in {:.1?}",
            root.len(),
            started.elapsed()
        );
        listed += 1;

        let is_archive = |e: &ArchiveEntry| e.archive.is_some();
        let Some(folder) = find(&device, "", 3, &is_archive) else {
            continue;
        };
        let inside = device
            .entries(&folder)
            .expect("a nested archive lists as a folder");
        nested_listed += 1;
        let small = |e: &ArchiveEntry| {
            e.kind == ArchiveEntryKind::File
                && e.archive.is_none()
                && (device.open_head(&format!("{folder}/{}", e.name), 16)).is_some()
        };
        if let Some(file) = inside.iter().find(|e| small(e)) {
            let rel = format!("{folder}/{}", file.name);
            assert!(device.exists(&rel), "a listed file exists by its path");
            opened += 1;
        }
    }
    println!("{listed} roots listed, {nested_listed} nested archives listed as folders, {opened} files opened by path");
    assert_eq!(listed, archives.len());
    assert!(nested_listed > 0, "a nested archive lists as a folder");
    assert!(
        opened > 0,
        "a file inside a nested archive opens by its path"
    );
}
