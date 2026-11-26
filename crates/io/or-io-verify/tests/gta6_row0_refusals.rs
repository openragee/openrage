//! Which geometries a vertex-changing write takes and which it refuses, by refusal name, over the
//! sample of a build's drawables the per_geometry build test reads: every vertex of each geometry
//! moved 1 mm along x, its triangles and count kept, and written by `write_geometry` itself.
//! Measured first by a node mirror of the writer's checks over the Xbox build 1.0.2.1, whose
//! sample reproduced that test's 2,423 and 845 row 0 words: 562 of 1,031 geometries taken, 468
//! refused for a row 0 word at no power of 4, 1 for a vertex in no row 4 element.
//!
//! Install-gated, game-derived and slow (it walks nested archives); it skips without:
//!   `OPENRAGE_GTA6_KEYS`   the `gtavi_magic.dat` key snapshot (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use or_io::container::rpf::gta6_chunk_table::entry_chunks;
use or_io::container::rpf::{
    read_rpf9_body, Gta6Class, Gta6Mount, GtaViKeys, Rpf9Role, Rpf9Sections, GTA6_SNAPSHOT_ENV,
};
use or_io::gta6::headerless::{
    read_drawable, write_geometry, GeometryAt, GeometryEdit, GeometryWriteError,
};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// The drawable resource type of a 20-byte row.
const DRAWABLE: u32 = 0x02;
/// The 16-byte zero slot a headerless body begins with.
const SLOT: u64 = 16;
/// A body larger than this is not sampled.
const CAP: u64 = 4 << 20;
const MAX_DEPTH: usize = 8;
/// One clear drawable in this many is read, until this many are: the per_geometry build test's
/// sample.
const STRIDE: usize = 4;
const DRAWABLES: usize = 300;
/// Metres every vertex moves along x.
const MOVE: f32 = 1e-3;
/// Places printed per refusal name.
const PLACES: usize = 3;

#[derive(Default)]
struct Tally {
    seen: usize,
    drawables: usize,
    geometries: usize,
    /// Geometries written, and of those the ones whose +0xB0 table took words.
    taken: usize,
    table_written: usize,
    refused: BTreeMap<String, (usize, Vec<String>)>,
    row0: usize,
    row4: usize,
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

/// A message with its digits masked, so reasons that differ only by an offset tally together.
fn masked(message: impl std::fmt::Display) -> String {
    (message.to_string().chars())
        .map(|c| if c.is_ascii_digit() { '#' } else { c })
        .collect()
}

fn tally(s: &Rpf9Sections, place: &str, t: &mut Tally) {
    let Ok(d) = read_drawable(&s.virtual_section, Some(s.physical.as_slice())) else {
        return;
    };
    t.drawables += 1;
    for (level, models) in d.levels.iter().enumerate() {
        for (model, m) in models.iter().flatten().enumerate() {
            for (geometry, g) in m.geometries.iter().enumerate() {
                let Ok(g) = g else {
                    continue;
                };
                let Ok(held) = g.positions.as_ref() else {
                    continue;
                };
                t.geometries += 1;
                let at = GeometryAt {
                    level,
                    model,
                    geometry,
                };
                let edit = GeometryEdit {
                    positions: held.iter().map(|&[x, y, z]| [x + MOVE, y, z]).collect(),
                    triangles: g.triangles.clone(),
                    ..GeometryEdit::default()
                };
                let write = write_geometry(&mut s.clone(), Rpf9Role::Drawable, None, at, &edit);
                let why = match write {
                    Ok(written) => {
                        t.taken += 1;
                        t.table_written += usize::from(written.table_rederived);
                        continue;
                    }
                    Err(e) => e,
                };
                if let GeometryWriteError::Refused(refusal) = &why {
                    t.row0 += usize::from(refusal.starts_with("a row 0 word"));
                    t.row4 += usize::from(refusal.contains("row 4 element"));
                }
                let (n, places) = t.refused.entry(masked(why)).or_default();
                *n += 1;
                if places.len() < PLACES {
                    places.push(format!(
                        "{place}, level {level} model {model} geometry {geometry}"
                    ));
                }
            }
        }
    }
}

fn visit(
    file: &mut File,
    keys: &GtaViKeys,
    (mount, label, depth): (&Gta6Mount, &str, usize),
    t: &mut Tally,
) {
    let extras = mount.archive.entry_extras();
    for (entry, extra) in mount.archive.entries.iter().zip(&extras) {
        if t.drawables >= DRAWABLES {
            return;
        }
        if entry.kind() == Some(Gta6Class::NestedArchive) {
            if depth < MAX_DEPTH {
                if let Ok(child) = mount.open_nested(&mut *file, entry, keys) {
                    visit(file, keys, (&child, label, depth + 1), t);
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
            let place = format!("{label}, entry {} at depth {depth}", entry.index);
            tally(&sections, &place, t);
        }
    }
}

#[test]
fn a_vertex_changing_write_over_the_sample_is_taken_or_refused_by_name() {
    let what = "GTA VI vertex-changing geometry writes over a build";
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
        let label = (path.file_name()).map_or_else(String::new, |n| n.to_string_lossy().into());
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys).expect("the archive opens");
        visit(&mut file, &keys, (&mount, &label, 0), &mut t);
    }
    println!(
        "{} drawables, {} geometries: {} taken ({} with table words written), {} refused",
        t.drawables,
        t.geometries,
        t.taken,
        t.table_written,
        t.geometries - t.taken
    );
    for (why, (n, places)) in &t.refused {
        println!("  refused {n} x {why}");
        for place in places {
            println!("    {place}");
        }
    }
    assert_eq!(
        t.drawables, DRAWABLES,
        "the sample reads {DRAWABLES} drawables"
    );
    let measured = (1031, 562, 468, 1);
    assert_eq!(
        (t.geometries, t.taken, t.row0, t.row4),
        measured,
        "(geometries, taken, refused for row 0, refused for row 4) against the node mirror's"
    );
}
