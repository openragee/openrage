//! R72h's reader census: GTA VI read parity between the two builds, measured family by family.
//!
//! For each build `OPENRAGE_GAME_ROOT` names (an Xbox Series folder and a PlayStation 5 folder, told
//! apart by its `eboot.bin`), every RPF9 archive and every nested archive is mounted with the key
//! snapshot, and every headerless resource is counted twice:
//!
//!  * **present** — its family, read straight from its 20-byte table row's resource type code
//!    ([`Gta6NestedRow::resource_type`]), which the archive's table states whatever the body's
//!    storage. This is the denominator, and it is the same table read on both builds.
//!  * **read** — whether the body opens through its real reader, the way the tree opens it:
//!    [`read_rpf9_body`] past the row's zero slot, its two sections handed to
//!    [`or_io::gta6::headerless::read`] (the drawable, fragment, dictionary, bound, cloth, … readers)
//!    or, for a map body that states no role, to [`read_model`].
//!
//! Each family's read share (read / present) is put beside the other build's, and a reader that reads
//! a smaller share of its PlayStation 5 bodies than its Xbox bodies is a delta, named and asserted
//! against. The readers are the shared platform-agnostic code once handed the sections, so a
//! PlayStation 5 body is expected to read exactly as its Xbox twin; the one measured platform
//! difference, the 64 KiB texel order, lives in the texture packs, which `or-io-verify`'s `ttpf`
//! suite already holds pixel-identical across the builds (R72d).
//!
//! ⚠ Install-gated and slow-tier (it walks whole installs), so it skips cleanly without:
//!   `OPENRAGE_GTA6_KEYS`   the `gtavi_magic.dat` key snapshot (never committed)
//!   `OPENRAGE_GAME_ROOT`   the build roots, `;`-separated, each holding `*.rpf` and `pack/*/*.rpf`

#![cfg(feature = "gta6")]

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use or_io::container::rpf::{
    read_rpf9_body, Gta6Mount, GtaViKeys, Rpf9BodyKind, GTA6_SNAPSHOT_ENV, RPF9_SLOT,
};
use or_io::gta6::headerless::read as read_headerless;
use or_io::gta6::map::{self, C_MAP_DATA, C_MAP_TYPES};
use or_io::meta::binary::rbm::gta6::read_model;
use or_io::Platform;
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use rayon::prelude::*;

/// The content cipher's type, whose bodies are deciphered before their head is read.
const CONTENT_CIPHER_TYPE: u16 = 17;
/// The most one resource inflates to; a body past it is counted unopened, not read short.
const LIMIT: u64 = 1 << 30;
/// Real nesting is shallow; this stops a cyclic mount recursing forever.
const MAX_DEPTH: usize = 8;
/// The machine is shared: the walk takes this many cores and no more.
const THREADS: usize = 4;

/// The family a headerless resource's 20-byte-row type code names (`gta6_form_h::VERSIONS`).
fn family_of(resource_type: u32) -> &'static str {
    match resource_type {
        0x02 => "drawable",
        0x03 => "fragment",
        0x04 => "drawable dictionary",
        0x05 => "texture dictionary",
        0x06 => "bound",
        0x0B | 0x57 | 0x6C | 0x71 | 0x73 => "metadata",
        0x0F | 0x11 => "acceleration structure",
        0x10 | 0x56 | 0x63 => "dictionary (other)",
        0x43 => "clip dictionary",
        0x46 | 0x4E | 0x66 | 0x6D => "animation",
        0x48 => "particles",
        0x49 => "map data",
        0x4A => "map types",
        0x4F => "navigation mesh",
        0x58 => "cloth",
        0x59 => "script program",
        0x5F => "keyed-blob descriptor",
        0x64 => "navigation octree",
        0x65 => "neighbour table",
        0x67 => "morph target",
        0x6E => "asset index",
        0x6F => "text table",
        0x72 => "keyed blob",
        _ => "other resource",
    }
}

/// One family's present and read counts on one build.
type Counts = (usize, usize);

/// What a build's walk found.
#[derive(Default)]
struct Census {
    archives: usize,
    entries: usize,
    /// Per family (by type code), (present, read): present counts the table row, read the bodies
    /// that opened through their reader.
    families: BTreeMap<&'static str, Counts>,
    /// The heads of the bodies no headerless reader claims: content-ciphered bodies and clear slot
    /// bodies that are not resources. Coverage of the metadata, PSO and script families.
    heads: BTreeMap<&'static str, usize>,
    /// Slot resources [`read_rpf9_body`] could not open at all.
    unopened: usize,
}

impl Census {
    fn present(&mut self, family: &'static str) {
        self.families.entry(family).or_default().0 += 1;
    }

    fn read(&mut self, family: &'static str) {
        self.families.entry(family).or_default().1 += 1;
    }

    fn merge(&mut self, other: Census) {
        self.archives += other.archives;
        self.entries += other.entries;
        self.unopened += other.unopened;
        for (family, (present, read)) in other.families {
            let into = self.families.entry(family).or_default();
            into.0 += present;
            into.1 += read;
        }
        for (head, n) in other.heads {
            *self.heads.entry(head).or_default() += n;
        }
    }
}

/// The family a body's first bytes place it in, for the heads no headerless reader claims.
fn classify_head(body: &[u8]) -> &'static str {
    if body.starts_with(b"PSIN") {
        "PSO (PSIN)"
    } else if body.starts_with(b"RBF0") {
        "metadata (RBF0)"
    } else if body.len() >= 8 && body[..8].iter().all(|&b| b == 0) {
        "script (headerless)"
    } else if is_text_head(body) {
        "text or XML"
    } else {
        "other"
    }
}

fn is_text_head(body: &[u8]) -> bool {
    if body.first() == Some(&b'<') || body.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return true;
    }
    let n = body.len().min(32);
    n > 0
        && body[..n]
            .iter()
            .all(|&c| matches!(c, b'\t' | b'\r' | b'\n') || (0x20..=0x7E).contains(&c))
}

/// Whether the folder is a PlayStation 5 build, by the `eboot.bin` beside its archives.
fn platform_of(root: &Path) -> Platform {
    if root.join("eboot.bin").is_file() {
        Platform::Ps5
    } else {
        Platform::XboxSeries
    }
}

/// Every `*.rpf` under `root` and every `pack/*/*.rpf`.
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

/// Walk one mounted archive and every archive nested in it into `census`.
fn walk(file: &mut File, mount: &Gta6Mount, keys: &GtaViKeys, depth: usize, census: &mut Census) {
    // Cloned so the recursion can reborrow the file while this level iterates.
    let entries = mount.archive.entries.clone();
    let extras = mount.archive.entry_extras();
    for entry in &entries {
        census.entries += 1;

        if depth < MAX_DEPTH
            && mount
                .entry_begins_archive(&mut *file, entry)
                .unwrap_or(false)
        {
            if let Ok(child) = mount.open_nested(&mut *file, entry, keys) {
                census.archives += 1;
                walk(file, &child, keys, depth + 1, census);
            }
            continue;
        }

        if let Some(cipher) = entry.cipher {
            if cipher.cipher_type == CONTENT_CIPHER_TYPE {
                match mount.read_entry_decrypted(&mut *file, entry, keys) {
                    Ok(body) => *census.heads.entry(classify_head(&body)).or_default() += 1,
                    Err(_) => census.unopened += 1,
                }
            }
            continue;
        }

        // A headerless resource is a clear 20-byte-row entry (a nested row, no stored size), read
        // past its zero slot. A sized clear body is a metadata, text or archive entry, not a reader.
        let extra = extras.get(entry.index as usize);
        let (Some(nested), Some(start)) = (
            extra
                .and_then(|x| x.nested)
                .filter(|_| entry.stored.is_none()),
            mount.absolute_start(entry),
        ) else {
            continue;
        };
        let family = family_of(nested.resource_type);
        census.present(family);
        census_read(file, start + RPF9_SLOT, family, census);
    }
}

/// Open the clear slot body at `offset` through its reader, and record it read where it opens.
fn census_read(file: &mut File, offset: u64, family: &'static str, census: &mut Census) {
    let Ok((find, bytes)) = read_rpf9_body(&mut *file, offset, LIMIT) else {
        census.unopened += 1;
        return;
    };
    let Rpf9BodyKind::Zlib(run) = find.kind else {
        *census.heads.entry(classify_head(&bytes)).or_default() += 1;
        return;
    };
    let (Some(resource), Some(signature)) = (run.resource, run.signature) else {
        *census.heads.entry(classify_head(&bytes)).or_default() += 1;
        return;
    };
    let (virtual_section, physical) = resource.sections(&bytes);
    let opened = if signature.role().is_some() {
        // A role the headerless readers dispatch on: the drawable, dictionary, bound, cloth, … path.
        read_headerless(signature, virtual_section, Some(physical)).is_ok()
    } else {
        // A map body states no role: its class table stands where a signature would, and it reads
        // through the RBM model reader, as the world index reads it.
        matches!(
            map::root_type(virtual_section),
            Some(C_MAP_DATA | C_MAP_TYPES)
        ) && read_model(virtual_section, physical).is_ok()
    };
    if opened {
        census.read(family);
    }
}

/// The census of one build: its archives walked in parallel, then merged.
fn census_of(archives: &[PathBuf], keys: &GtaViKeys) -> Census {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(THREADS)
        .build()
        .expect("a thread pool");
    let parts: Vec<Census> = pool.install(|| {
        archives
            .par_iter()
            .map(|path| {
                let mut census = Census::default();
                let Ok(mut file) = File::open(path) else {
                    return census;
                };
                match Gta6Mount::open_root(&mut file, keys) {
                    Ok(mount) => {
                        census.archives += 1;
                        walk(&mut file, &mount, keys, 0, &mut census);
                    }
                    Err(e) => eprintln!("open {}: {e}", path.display()),
                }
                census
            })
            .collect()
    });
    let mut all = Census::default();
    for part in parts {
        all.merge(part);
    }
    all
}

/// One build's table printed, and its families returned as family → (present, read).
fn report(platform: Platform, census: &Census) -> BTreeMap<&'static str, Counts> {
    println!(
        "\n{} — {} archive(s), {} entries; {} slot resource(s) unopened",
        platform.label(),
        census.archives,
        census.entries,
        census.unopened,
    );
    println!("  families (read / present = share):");
    for (family, &(present, read)) in &census.families {
        println!(
            "    {family:>22}: {read:>6} / {present:<6} = {:>6.2}%",
            share((present, read)).unwrap_or(0.0) * 100.0,
        );
    }
    println!("  heads no headerless reader claims:");
    for (head, n) in &census.heads {
        println!("    {head:>22}: {n}");
    }
    census.families.clone()
}

/// The share of its present bodies a family read, or `None` where none was present.
fn share((present, read): Counts) -> Option<f64> {
    (present > 0).then(|| read as f64 / present as f64)
}

/// A family whose PlayStation 5 read share falls below its Xbox share.
struct Delta {
    family: &'static str,
    xbox: f64,
    ps5: f64,
}

/// Every family both builds hold whose PlayStation 5 read share is below its Xbox share by more than
/// a rounding margin. A family Xbox itself never read (share 0) sets no bar: no reader is claimed.
fn deltas(
    xbox: &BTreeMap<&'static str, Counts>,
    ps5: &BTreeMap<&'static str, Counts>,
) -> Vec<Delta> {
    let mut out = Vec::new();
    for (&family, &x) in xbox {
        let (Some(xs), Some(ps)) = (share(x), ps5.get(family).copied().and_then(share)) else {
            continue;
        };
        if xs > 0.0 && ps + 1e-9 < xs {
            out.push(Delta {
                family,
                xbox: xs,
                ps5: ps,
            });
        }
    }
    out
}

#[test]
fn a_family_that_reads_less_on_ps5_is_named() {
    // Xbox reads every bound; PlayStation 5 reads four of five: a delta, named.
    let xbox = BTreeMap::from([("bound", (10, 10)), ("drawable", (8, 8))]);
    let ps5 = BTreeMap::from([("bound", (5, 4)), ("drawable", (8, 8))]);
    let found = deltas(&xbox, &ps5);
    assert_eq!(found.len(), 1, "one family fell behind");
    assert_eq!(found[0].family, "bound");
    // Parity (equal shares), and a family neither build reads, set no delta.
    let even = BTreeMap::from([("bound", (3, 3)), ("clip dictionary", (2, 0))]);
    let more = BTreeMap::from([("bound", (7, 7)), ("clip dictionary", (9, 0))]);
    assert!(
        deltas(&even, &more).is_empty(),
        "equal or unread-by-both is no delta"
    );
}

#[test]
fn every_typed_reader_reads_the_same_share_on_ps5_as_on_xbox() {
    let what = "GTA VI reader census over the builds";
    let Some(snapshot) = env_path(GTA6_SNAPSHOT_ENV) else {
        skip_install(
            what,
            format_args!("set {GTA6_SNAPSHOT_ENV} and {GAME_ROOT_ENV}"),
        );
        return;
    };
    let roots = game_roots();
    if roots.is_empty() {
        skip_install(what, format_args!("{GAME_ROOT_ENV} names no GTA VI folder"));
        return;
    }
    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");

    let mut by_platform: BTreeMap<&str, BTreeMap<&'static str, Counts>> = BTreeMap::new();
    let mut any_present = false;
    for root in &roots {
        let archives = build_archives(root);
        if archives.is_empty() {
            continue;
        }
        let platform = platform_of(root);
        let census = census_of(&archives, &keys);
        any_present |= !census.families.is_empty();
        let families = report(platform, &census);
        // One build per platform; a second folder of the same platform is folded in.
        let slot = by_platform.entry(platform.label()).or_default();
        for (family, (present, read)) in families {
            let e = slot.entry(family).or_default();
            e.0 += present;
            e.1 += read;
        }
    }

    if !any_present {
        skip_install(
            what,
            format_args!("no headerless RPF9 resource under the folders {GAME_ROOT_ENV} names"),
        );
        return;
    }

    let xbox = by_platform.get(Platform::XboxSeries.label());
    let ps5 = by_platform.get(Platform::Ps5.label());
    let (Some(xbox), Some(ps5)) = (xbox, ps5) else {
        eprintln!(
            "reader census: only {} measured, so no cross-build parity; name both builds in {} to \
             compare",
            by_platform
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(" and "),
            GAME_ROOT_ENV,
        );
        return;
    };

    // Families one build holds and the other does not, reported for judgement, not asserted.
    for (label, a, b) in [("Xbox only", xbox, ps5), ("PlayStation 5 only", ps5, xbox)] {
        let only: Vec<&str> = a
            .keys()
            .filter(|family| !b.contains_key(**family))
            .copied()
            .collect();
        if !only.is_empty() {
            eprintln!("families {label}: {}", only.join(", "));
        }
    }

    let found = deltas(xbox, ps5);
    for d in &found {
        eprintln!(
            "delta: the {} reader reads {:.2}% of its PlayStation 5 bodies and {:.2}% of its Xbox \
             bodies",
            d.family,
            d.ps5 * 100.0,
            d.xbox * 100.0,
        );
    }
    assert!(
        found.is_empty(),
        "{} reader(s) read a smaller share on PlayStation 5 than on Xbox; the readers are shared \
         code, so a PlayStation 5 body reads as an Xbox one once its sections are in hand",
        found.len(),
    );
}

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.exists())
}
