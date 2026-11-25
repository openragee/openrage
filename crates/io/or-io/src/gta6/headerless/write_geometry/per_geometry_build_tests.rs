//! Install-gated: rows 0 and 4 of a sample of a build's drawables derived again from each
//! geometry as read, beside what the table stores; the agreement is printed, not asserted.
//!   `OPENRAGE_GTA6_KEYS`   the `gtavi_magic.dat` key snapshot (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

use std::fs::File;
use std::path::{Path, PathBuf};

use or_test_support::skip_install;

use super::*;
use crate::container::rpf::gta6_chunk_table::entry_chunks;
use crate::container::rpf::{
    read_rpf9_body, Gta6Class, Gta6Mount, GtaViKeys, Rpf9Sections, GTA6_SNAPSHOT_ENV,
};
use crate::gta6::headerless::read_drawable;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// The drawable resource type of a 20-byte row.
const DRAWABLE: u32 = 0x02;
/// The 16-byte zero slot a headerless body begins with.
const SLOT: u64 = 16;
/// A body larger than this is not sampled.
const CAP: u64 = 4 << 20;
const MAX_DEPTH: usize = 8;
/// One clear drawable in this many is read, until this many are.
const STRIDE: usize = 4;
const DRAWABLES: usize = 300;
/// Metres a stored element may miss a vertex by and still hold it.
const HOLD: f32 = 2e-5;

#[derive(Default, Debug)]
struct Tally {
    seen: usize,
    drawables: usize,
    tables: usize,
    geometries: usize,
    /// Row 0 words of a channel the geometry does not declare, and of those the ones stored 0.
    absent: usize,
    absent_zero: usize,
    /// Row 0 words derived within 5 % of the stored word, or not.
    agree: usize,
    differ: usize,
    /// Row 4 runs of one element equal to the derived box, of one element otherwise, and longer.
    box_equal: usize,
    one_element: usize,
    longer: usize,
    /// Vertices some stored element of their run holds, and vertices none does.
    held: usize,
    missed: usize,
    /// Vertices outside their derived box.
    outside: usize,
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

fn f32_in(v: &[u8], at: usize) -> f32 {
    f32::from_le_bytes([v[at], v[at + 1], v[at + 2], v[at + 3]])
}

/// Whether the stored element at `e` holds `p` on its own axes.
fn holds(v: &[u8], e: usize, p: [f32; 3]) -> bool {
    (0..3).all(|a| {
        let axis = e + 0x20 + 0x10 * a;
        let along: f32 = (0..3).map(|k| p[k] * f32_in(v, axis + 4 * k)).sum();
        (f32_in(v, e + 4 * a) - HOLD..=f32_in(v, e + 0x10 + 4 * a) + HOLD).contains(&along)
    })
}

fn tally(s: &Rpf9Sections, t: &mut Tally) {
    let v = &s.virtual_section;
    let Ok(d) = read_drawable(v, Some(&s.physical)) else {
        return;
    };
    t.drawables += 1;
    let mut counted = false;
    for (level, models) in d.levels.iter().enumerate() {
        for (model, m) in models.iter().flatten().enumerate() {
            for (geometry, g) in m.geometries.iter().enumerate() {
                let at = GeometryAt {
                    level,
                    model,
                    geometry,
                };
                let (Ok(g), Some(found)) = (g, table(v, 0, at)) else {
                    continue;
                };
                let Ok(positions) = g.positions.as_ref() else {
                    continue;
                };
                if !counted {
                    t.tables += 1;
                    counted = true;
                }
                t.geometries += 1;
                for c in 0..4u8 {
                    let stored = f32_in(v, found.rows[0].0 + 16 * found.place + 4 * usize::from(c));
                    let channel = FIRST_TEXCOORD + c;
                    if !g.channels.iter().any(|k| k.index == channel) {
                        t.absent += 1;
                        t.absent_zero += usize::from(stored == 0.0);
                        continue;
                    }
                    let uvs = of_channel(&g.vertices.uvs, channel);
                    let Some(r) = ratio(positions, &g.triangles, uvs) else {
                        continue;
                    };
                    let derived = r * scale(stored, r);
                    let near = (derived - f64::from(stored)).abs() <= WITHIN * f64::from(stored);
                    match near || (derived == 0.0 && stored == 0.0) {
                        true => t.agree += 1,
                        false => t.differ += 1,
                    }
                }
                let (min, max) = extremes(positions);
                t.outside += (positions.iter())
                    .filter(|p| (0..3).any(|a| !(min[a]..=max[a]).contains(&p[a])))
                    .count();
                let (row3, row4) = (found.rows[3].0, found.rows[4].0);
                let pair = |o: usize| usize::from(u16::from_le_bytes([v[o], v[o + 1]]));
                let at3 = row3 + 4 * found.place;
                let (first, end) = (pair(at3), pair(at3 + 2));
                match end - first {
                    1 if v[row4 + ELEMENT * first..row4 + ELEMENT * end]
                        == element((min, max))[..] =>
                    {
                        t.box_equal += 1
                    }
                    1 => t.one_element += 1,
                    _ => t.longer += 1,
                }
                for p in positions {
                    match (first..end).any(|e| holds(v, row4 + ELEMENT * e, *p)) {
                        true => t.held += 1,
                        false => t.missed += 1,
                    }
                }
            }
        }
    }
}

fn visit(file: &mut File, keys: &GtaViKeys, mount: &Gta6Mount, depth: usize, t: &mut Tally) {
    let extras = mount.archive.entry_extras();
    for (entry, extra) in mount.archive.entries.iter().zip(&extras) {
        if t.drawables >= DRAWABLES {
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
        if let Some(sections) = sections {
            tally(&sections, t);
        }
    }
}

#[test]
fn rows_0_and_4_derived_again_from_a_sample_of_the_build_beside_what_they_store() {
    let what = "GTA VI per-geometry table rows 0 and 4 over a build";
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
        if t.drawables >= DRAWABLES {
            break;
        }
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys).expect("the archive opens");
        visit(&mut file, &keys, &mount, 0, &mut t);
    }
    println!("{t:#?}");
    println!(
        "row 0: {} agree, {} differ; row 4: {} equal the derived box, {} differ",
        t.agree,
        t.differ,
        t.box_equal,
        t.one_element + t.longer
    );
    assert!(t.tables > 0, "the sample holds drawables whose table reads");
    assert_eq!(t.outside, 0, "a derived box holds each of its vertices");
}
