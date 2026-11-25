//! Install-gated: rows 0 and 4 of a sample of a build's drawables derived again by the writer's
//! rules from each geometry as read, beside what the table stores; agreement printed and held to
//! the share measured.
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
/// Metres an element re-extended over the vertices it holds may differ from what it stores by.
const NEAR: f32 = 1e-4;

#[derive(Default, Debug)]
struct Tally {
    seen: usize,
    drawables: usize,
    tables: usize,
    geometries: usize,
    /// Row 0 words of a channel the geometry does not declare, and of those the ones stored 0.
    absent: usize,
    absent_zero: usize,
    /// Row 0 words a power of 4 times their area ratio within 5 % (or both 0), and the others,
    /// which a write refuses.
    agree: usize,
    differ: usize,
    /// Row 4 elements of one-element and longer runs whose extremes over the vertices they hold
    /// lie within 1e-4 m of what they store, and the others.
    one_agree: usize,
    one_differ: usize,
    more_agree: usize,
    more_differ: usize,
    /// Geometries whose elements a moved-vertex write refuses, and why.
    no_element: usize,
    split_part: usize,
    empty_element: usize,
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
                    match power(stored, r).is_some() || (r == 0.0 && stored == 0.0) {
                        true => t.agree += 1,
                        false => t.differ += 1,
                    }
                }
                let Ok(elements) = run(v, &found) else {
                    continue;
                };
                let held_by = match members(&elements, positions, &g.triangles) {
                    Ok(held_by) => held_by,
                    Err(why) => {
                        t.no_element += usize::from(why == NO_ELEMENT);
                        t.split_part += usize::from(why == SPLIT_PART);
                        t.empty_element += usize::from(why == EMPTY_ELEMENT);
                        continue;
                    }
                };
                for (e, m) in elements.iter().zip(&held_by) {
                    let (min, max) = e.extents(m.iter().map(|&i| positions[i]));
                    let near = (0..3).all(|a| {
                        (min[a] - e.min[a]).abs() <= NEAR && (max[a] - e.max[a]).abs() <= NEAR
                    });
                    match (elements.len() == 1, near) {
                        (true, true) => t.one_agree += 1,
                        (true, false) => t.one_differ += 1,
                        (false, true) => t.more_agree += 1,
                        (false, false) => t.more_differ += 1,
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
    let (agree4, differ4) = (t.one_agree + t.more_agree, t.one_differ + t.more_differ);
    println!(
        "row 0: {} agree, {} differ; row 4: {agree4} agree, {differ4} differ, {} geometries \
         refused",
        t.agree,
        t.differ,
        t.no_element + t.split_part + t.empty_element
    );
    assert!(t.tables > 0, "the sample holds drawables whose table reads");
    // Round 33 measured 2,423 agree and 845 differ over this sample, 74 %.
    assert!(
        10 * t.agree >= 7 * (t.agree + t.differ),
        "row 0 words at a power of 4 times their area ratio: {} of {}",
        t.agree,
        t.agree + t.differ
    );
    // A node mirror over 1,500 drawables found 19,392 of 19,421 lone elements and 46,106 of
    // 46,461 elements of longer runs within 1e-4 m.
    assert!(
        100 * agree4 >= 97 * (agree4 + differ4),
        "row 4 elements re-extended within 1e-4 m: {agree4} of {}",
        agree4 + differ4
    );
    assert!(
        50 * (t.no_element + t.split_part + t.empty_element) <= t.geometries,
        "geometries a moved-vertex write refuses: {} of {}",
        t.no_element + t.split_part + t.empty_element,
        t.geometries
    );
}
