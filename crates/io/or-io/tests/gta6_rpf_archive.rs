//! A GTA VI archive opens through `RpfArchive`, as every other generation does: every top-level
//! archive of a build lists as a tree holding each entry of its table once, a nested archive opens
//! as an archive of its own through `open_nested`, and a body read through `RpfArchive` is the body
//! the GTA VI reader gives for the same row.
//!
//! Install-gated: it skips cleanly without the keys and the build.
//!   `OPENRAGE_GTA6_KEYS`   the key snapshot, or `OPENRAGE_GTA6_EXE` for the bundled one
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

#![cfg(feature = "gta6")]

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use or_io::container::rpf::{
    read_rpf9_body, Gta6Context, Gta6Entry, Gta6Mount, GtaViKeys, NameResolver, RpfArchive,
    RpfVersion, GTA6_EXE_ENV, GTA6_SNAPSHOT_ENV, RPF9_SLOT,
};
use or_io::container::RpfCryptoProvider;
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// A body stored larger than this is not read back: the check is the read, not the volume.
const BODY_CAP: u64 = 1 << 20;
/// How many bodies of one archive, and how many of its nested archives, are compared.
const BODIES: usize = 24;
const NESTED: usize = 4;

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
    archives: usize,
    entries: usize,
    nested_listed: usize,
    nested_opened: usize,
    bodies: usize,
    /// Clear bodies the content reader opened at their place, of those tried: by row shape.
    headerless: (usize, usize),
    sized: (usize, usize),
    /// Files listed by the kind their row names, and those whose row no table names.
    kinds: BTreeMap<&'static str, usize>,
    kindless: usize,
    failures: Vec<String>,
}

/// Rows of the whole build whose class byte no table names: two of `0x0E` and one of `0x60`,
/// counted over every top-level and nested table (round 33).
const KINDLESS_ROWS: usize = 3;

/// Compare `archive` with the same table mounted by the GTA VI reader: one file per entry, the
/// nested archives named as archives, and a sample of bodies equal.
fn compare(
    file: &mut File,
    archive: &Arc<RpfArchive>,
    mount: &Gta6Mount,
    keys: &GtaViKeys,
    t: &mut Tally,
) -> Vec<u32> {
    let what = format!("the archive at {:#X}", mount.base);
    if archive.version != RpfVersion::Rpf9 {
        t.failures
            .push(format!("{what} opened as {}", archive.version));
    }
    if archive.file_count() != mount.archive.entries.len() {
        t.failures.push(format!(
            "{what} lists {} files for {} entries",
            archive.file_count(),
            mount.archive.entries.len()
        ));
    }
    t.entries += mount.archive.entries.len();

    let rows: HashMap<u64, &Gta6Entry> = (mount.archive.entries.iter())
        .filter_map(|e| e.start.map(|start| (start, e)))
        .collect();
    let mut nested = Vec::new();
    let mut compared = 0;
    for node in archive.nodes.iter().filter(|n| !n.is_dir()) {
        let index = node.entry_index();
        let Some(binary) = archive.binary_entry(index) else {
            t.failures
                .push(format!("{what}: entry {index} is not a file"));
            continue;
        };
        let Some(row) = rows.get(&binary.file_offset) else {
            continue;
        };
        match archive.rpf9_kind(index) {
            Some(kind) => *t.kinds.entry(kind.label).or_default() += 1,
            None => t.kindless += 1,
        }
        let begins = mount.entry_begins_archive(file, row).unwrap_or(false);
        if begins != node.name().ends_with(".rpf") {
            t.failures
                .push(format!("{what}: entry {index} is named against its body"));
        }
        if begins {
            nested.push(index);
            continue;
        }
        // The place a caller opens the body at is the row's own entry, and its body's start: the
        // row's start, past the 16-byte zero slot where the row is a headerless resource's.
        let headerless = row.stored.is_none();
        let slot = if headerless { RPF9_SLOT } else { 0 };
        let placed = (archive.rpf9_place(index)).map(|p| (p.base, p.offset, p.entry));
        let stated = (mount.absolute_start(row)).map(|at| (mount.base, at + slot, row.index));
        if placed != stated {
            t.failures
                .push(format!("{what}: entry {index} is placed off its row"));
        }
        if compared == BODIES || u64::from(binary.read_size()) > BODY_CAP {
            continue;
        }
        compared += 1;
        // A clear body is opened by the reader that tells a body's kind from its bytes at its place.
        if let (Some((_, at, _)), None) = (placed, row.cipher) {
            let found = read_rpf9_body(&mut *file, at, u64::MAX).is_ok();
            let count = match headerless {
                true => &mut t.headerless,
                false => &mut t.sized,
            };
            count.0 += usize::from(found);
            count.1 += 1;
        }
        let through_archive = archive.read_entry(index).map_err(|e| e.to_string());
        let through_reader =
            (mount.read_entry_decrypted(&mut *file, row, keys)).map_err(|e| e.to_string());
        // A body both refuse is refused alike; what is compared is that neither reads another body.
        if through_archive.as_ref().ok() != through_reader.as_ref().ok() {
            t.failures
                .push(format!("{what}: entry {index} reads differently"));
        }
        t.bodies += usize::from(through_archive.is_ok());
    }
    t.nested_listed += nested.len();
    nested
}

#[test]
fn every_archive_of_the_build_opens_as_a_tree_and_reads_as_the_gta_vi_reader_does() {
    let what = "a GTA VI build through RpfArchive";
    let build = std::env::var_os(BUILD_ENV)
        .map(PathBuf::from)
        .filter(|p| p.is_dir());
    let (Some(build), Ok(Some(keys))) = (build, GtaViKeys::for_install(None)) else {
        skip_install(
            what,
            format_args!("set {BUILD_ENV} and {GTA6_SNAPSHOT_ENV} (or {GTA6_EXE_ENV})"),
        );
        return;
    };
    let archives = build_archives(&build);
    assert!(!archives.is_empty(), "no .rpf under {}", build.display());

    let keys = Arc::new(keys);
    // The keys travel into every open through the provider; names are not needed to compare bodies.
    let crypto: Arc<dyn RpfCryptoProvider> = Arc::new(Gta6Context {
        keys: Arc::clone(&keys),
        names: Arc::new(NameResolver::new()),
    });
    let mut t = Tally::default();
    for path in &archives {
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys).expect("the GTA VI reader mounts it");
        let archive = match RpfArchive::open_with(path, Arc::clone(&crypto)) {
            Ok(archive) => Arc::new(archive),
            Err(e) => {
                t.failures.push(format!("{}: {e}", path.display()));
                continue;
            }
        };
        t.archives += 1;
        let nested = compare(&mut file, &archive, &mount, &keys, &mut t);

        let rows: HashMap<u64, &Gta6Entry> = (mount.archive.entries.iter())
            .filter_map(|e| e.start.map(|start| (start, e)))
            .collect();
        for index in nested.into_iter().take(NESTED) {
            let offset = archive.binary_entry(index).map(|b| b.file_offset);
            let Some(row) = offset.and_then(|at| rows.get(&at)) else {
                continue;
            };
            let child_mount = mount
                .open_nested(&mut file, row, &keys)
                .expect("the reader opens it");
            match archive.open_nested(index) {
                Ok(child) => {
                    t.nested_opened += 1;
                    compare(&mut file, &Arc::new(child), &child_mount, &keys, &mut t);
                }
                Err(e) => t
                    .failures
                    .push(format!("{}: nested {index}: {e}", path.display())),
            }
        }
    }

    println!(
        "RpfArchive over a GTA VI build: {} archives, {} entries listed, {} nested archives named, \
         {} opened, {} bodies equal, {} failures",
        t.archives,
        t.entries,
        t.nested_listed,
        t.nested_opened,
        t.bodies,
        t.failures.len()
    );
    println!(
        "opened by the content reader at their place: {} of {} headerless, {} of {} sized",
        t.headerless.0, t.headerless.1, t.sized.0, t.sized.1
    );
    println!("files by kind ({} with none):", t.kindless);
    for (kind, count) in &t.kinds {
        println!("  {kind}: {count}");
    }
    for failure in t.failures.iter().take(20) {
        eprintln!("  {failure}");
    }
    assert!(
        t.failures.is_empty(),
        "RpfArchive and the GTA VI reader must agree"
    );
    assert_eq!(
        t.archives,
        archives.len(),
        "every archive of the build opens"
    );
    assert!(
        t.nested_opened > 0,
        "nested archives open through open_nested"
    );
    assert!(t.bodies > 0, "bodies read through RpfArchive");
    assert!(
        t.kindless <= KINDLESS_ROWS,
        "every sampled row has a kind but the build's {KINDLESS_ROWS} of a class no table names"
    );
    assert!(
        t.kinds.get("archive").copied().unwrap_or(0) == t.nested_listed,
        "every nested archive is of the archive kind"
    );
}

/// Needs no key: a header is in the clear. `OPENRAGE_GAME_ROOT` names the folders, one a build.
#[test]
fn every_archive_header_of_a_build_parses_and_states_its_build_s_one_platform() {
    use or_io::container::rpf::{Rpf9Header, RPF9_HEADER_SIZE, RPF9_MAGIC};
    use std::io::Read;

    let what = "gta6 archive headers by platform";
    let mut builds = 0;
    for root in or_test_support::game_roots() {
        let mut platforms = BTreeMap::new();
        for path in build_archives(&root) {
            let mut head = [0u8; RPF9_HEADER_SIZE];
            let read = File::open(&path).and_then(|mut file| file.read_exact(&mut head));
            let magic = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
            if read.is_err() || magic != RPF9_MAGIC {
                continue;
            }
            let header =
                Rpf9Header::parse(&head).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let facts = header
                .facts()
                .expect("a parsed header states a measured platform");
            *platforms.entry(facts.platform.label()).or_insert(0usize) += 1;
        }
        if platforms.is_empty() {
            continue;
        }
        builds += 1;
        eprintln!("{}: {platforms:?}", root.display());
        assert_eq!(
            platforms.len(),
            1,
            "{} holds archives of more than one platform: {platforms:?}",
            root.display()
        );
    }
    if builds == 0 {
        skip_install(what, "OPENRAGE_GAME_ROOT names no folder of RPF9 archives");
    }
}
