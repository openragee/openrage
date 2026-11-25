//! Install-gated: every geometry of a sample of a build's drawables states a palette the cut's
//! byte-wide ceiling holds, read as the cut reads it.
//!   `OPENRAGE_GTA6_KEYS`   the `gtavi_magic.dat` key snapshot (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

use std::fs::File;
use std::path::{Path, PathBuf};

use or_test_support::skip_install;

use super::{palette_of, walk, GeometryAt, PALETTE};
use crate::container::rpf::gta6_chunk_table::entry_chunks;
use crate::container::rpf::{
    read_rpf9_body, Gta6Class, Gta6Mount, GtaViKeys, Rpf9Sections, GTA6_SNAPSHOT_ENV,
};

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// The drawable resource type of a 20-byte row.
const DRAWABLE: u32 = 0x02;
/// The 16-byte zero slot a headerless body begins with.
const SLOT: u64 = 16;
/// A body larger than this is not sampled.
const CAP: u64 = 4 << 20;
const MAX_DEPTH: usize = 8;
/// One clear drawable in this many is read.
const STRIDE: usize = 4;
/// The sample stops once it has read this many drawables and seen this many skinned geometries,
/// or at the cap.
const DRAWABLES: usize = 1_000;
const SKINNED: usize = 100;
const DRAWABLES_CAP: usize = 20_000;
const AT: GeometryAt = GeometryAt {
    level: 0,
    model: 0,
    geometry: 0,
};

#[derive(Default)]
struct Tally {
    seen: usize,
    drawables: usize,
    geometries: usize,
    /// Geometries that state a palette.
    skinned: usize,
    /// Geometries whose four bytes at +0x2C, read as one u32, pass the ceiling.
    wide: usize,
    largest: u16,
    past: Vec<String>,
}

impl Tally {
    fn done(&self) -> bool {
        (self.drawables >= DRAWABLES && self.skinned >= SKINNED) || self.drawables >= DRAWABLES_CAP
    }
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
        if t.done() {
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
        let (None, Some(start), Some(row)) =
            (&entry.cipher, mount.absolute_start(entry), extra.nested)
        else {
            continue;
        };
        if row.resource_type != DRAWABLE
            || row.stored_size() > CAP
            || !matches!(
                entry_chunks(&mount.archive, entry.index as usize),
                Ok(Some(_))
            )
        {
            continue;
        }
        t.seen += 1;
        if !t.seen.is_multiple_of(STRIDE) {
            continue;
        }
        let sections = read_rpf9_body(&mut *file, start + SLOT, CAP)
            .ok()
            .and_then(|(find, bytes)| Rpf9Sections::of(&find, &bytes));
        let Some(sections) = sections else {
            continue;
        };
        let v = &sections.virtual_section;
        let Ok(walked) = walk(v, 0, AT) else {
            continue;
        };
        t.drawables += 1;
        for &g in walked.models.iter().flatten() {
            t.geometries += 1;
            let Ok(palette) = palette_of(v, g) else {
                t.past.push(format!(
                    "entry {}: geometry {g:#X} lies past its section",
                    entry.index
                ));
                continue;
            };
            let word = v
                .get(g + 0x2C..g + 0x30)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
            t.wide += usize::from(word.is_some_and(|w| w > u32::from(PALETTE)));
            t.skinned += usize::from(palette > 0);
            t.largest = t.largest.max(palette);
            if palette > PALETTE {
                t.past.push(format!(
                    "entry {}: geometry {g:#X} states {palette}",
                    entry.index
                ));
            }
        }
    }
}

#[test]
fn every_geometry_of_a_sample_of_the_build_states_a_palette_within_one_byte() {
    let what = "GTA VI geometry palette widths over a build";
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
    let mut t = Tally::default();
    for path in &archives {
        if t.done() {
            break;
        }
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys).expect("the archive opens");
        visit(&mut file, &keys, &mount, 0, &mut t);
    }
    println!(
        "{} drawables, {} geometries, {} skinned, largest palette {}, {} past the ceiling as a u32",
        t.drawables, t.geometries, t.skinned, t.largest, t.wide
    );
    assert!(t.skinned > 0, "the sample holds skinned geometries");
    assert!(
        t.wide > 0,
        "the sample holds a geometry whose word after its palette count is not zero"
    );
    assert!(t.past.is_empty(), "{:?}", &t.past[..t.past.len().min(20)]);
}
