//! How many mission state records of another shape (`Run::Other`) `Declared::elements` reads, and
//! the shapes it refuses, over every clear table-form body of a build's two script archives;
//! only the measured population of those bodies is asserted.
//!
//! Install-gated, game-derived and slow (it reads every body of two archives); it skips without:
//!   `OPENRAGE_GTA6_KEYS`   the `gtavi_magic.dat` key snapshot (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

#![cfg(all(feature = "gta6", feature = "meta"))]

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use or_io::container::rpf::{Gta6Class, Gta6Mount, GtaViKeys, GTA6_SNAPSHOT_ENV};
use or_io::gta6::hash::name_hash32;
use or_io::gta6::mission_other::{Declared, OtherShapeError};
use or_io::gta6::mission_tables::{Body, Run};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
const MAX_DEPTH: usize = 8;
/// The two script archives, by the name hash of their file name, and the table-form bodies each
/// holds clear.
const ARCHIVES: [(u32, usize); 2] = [(0x46DD_C648, 1_012), (0xA066_3C1A, 2_309)];

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

/// The archive under `root` or one of `root/pack/*` whose file name hashes to `name`.
fn archive_named(root: &Path, name: u32) -> Option<PathBuf> {
    let dirs = std::fs::read_dir(root.join("pack"))
        .into_iter()
        .flatten()
        .flatten();
    let dirs = std::iter::once(root.to_path_buf()).chain(dirs.map(|d| d.path()));
    dirs.flat_map(|dir| std::fs::read_dir(dir).into_iter().flatten().flatten())
        .map(|entry| entry.path())
        .find(|path| {
            (path.file_name()).is_some_and(|n| name_hash32(n.to_string_lossy().as_bytes()) == name)
        })
}

/// A refusal with its digits masked, so refusals that differ only by an offset tally together.
fn shape(error: &OtherShapeError) -> String {
    let text = match error {
        OtherShapeError::NotAPair { why, .. } => format!("not a pair: {why}"),
        other => other.to_string(),
    };
    (text.chars())
        .map(|c| if c.is_ascii_digit() { '#' } else { c })
        .collect()
}

#[derive(Default)]
struct Census {
    /// Table-form bodies by the name hash of their archive.
    bodies: BTreeMap<u32, usize>,
    tables_refused: usize,
    with_tables: usize,
    records: usize,
    empty: usize,
    placed: usize,
    table_runs: usize,
    other: usize,
    other_elements: usize,
    /// Other records `Declared::elements` reads, and the elements they hold.
    read: usize,
    read_elements: usize,
    refused: BTreeMap<String, usize>,
    /// Other records read and refused, by the bytes of an element.
    by_size: BTreeMap<usize, (usize, usize)>,
}

impl Census {
    fn body(&mut self, archive: u32, bytes: &[u8], body: &Body) {
        *self.bodies.entry(archive).or_default() += 1;
        let Ok(tables) = body.tables(bytes) else {
            self.tables_refused += 1;
            return;
        };
        if tables.is_empty() {
            return;
        }
        self.with_tables += 1;
        // The type table is read once per body, at its first other record.
        let mut declared: Option<Result<Declared, String>> = None;
        for record in tables.iter().flat_map(|t| &t.records) {
            self.records += 1;
            let element = match body.run(bytes, &tables, record) {
                None => {
                    self.empty += 1;
                    continue;
                }
                Some(Run::Table(_)) => {
                    self.table_runs += 1;
                    continue;
                }
                Some(Run::Placed(_)) => {
                    self.placed += 1;
                    continue;
                }
                Some(Run::Other { element }) => element,
            };
            self.other += 1;
            self.other_elements += usize::from(record.count);
            let declared =
                declared.get_or_insert_with(|| Declared::open(bytes).map_err(|e| shape(&e)));
            let outcome = match declared {
                Ok(declared) => declared.elements(body, record).map_err(|e| shape(&e)),
                Err(why) => Err(why.clone()),
            };
            let sized = self.by_size.entry(element).or_default();
            match outcome {
                Ok(elements) => {
                    self.read += 1;
                    self.read_elements += elements.len();
                    sized.0 += 1;
                }
                Err(why) => {
                    *self.refused.entry(why).or_default() += 1;
                    sized.1 += 1;
                }
            }
        }
    }
}

/// Every clear table-form body of `mount` and of every archive nested in it.
fn walk(
    file: &mut File,
    keys: &GtaViKeys,
    (mount, archive, depth): (&Gta6Mount, u32, usize),
    census: &mut Census,
) {
    for entry in &mount.archive.entries {
        if entry.kind() == Some(Gta6Class::NestedArchive) {
            if depth < MAX_DEPTH {
                if let Ok(child) = mount.open_nested(&mut *file, entry, keys) {
                    walk(file, keys, (&child, archive, depth + 1), census);
                }
            }
            continue;
        }
        // A 20-byte row states no class: those of these archives are script programs.
        if entry.cipher.is_some() || entry.class.is_none() || entry.start.is_none() {
            continue;
        }
        let Ok(bytes) = mount.read_entry(&mut *file, entry) else {
            continue;
        };
        if let Ok(body) = Body::open(&bytes) {
            census.body(archive, &bytes, &body);
        }
    }
}

#[test]
fn the_other_shape_records_read_by_their_declaration_are_counted() {
    let what = "GTA VI mission records of another shape over a build";
    let (Some(snapshot), Some(build)) = (env_path(GTA6_SNAPSHOT_ENV), env_path(BUILD_ENV)) else {
        skip_install(
            what,
            format_args!("set {GTA6_SNAPSHOT_ENV} and {BUILD_ENV}"),
        );
        return;
    };
    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
    let mut census = Census::default();
    for (archive, _) in ARCHIVES {
        let path = archive_named(&build, archive)
            .unwrap_or_else(|| panic!("the build holds no archive of name hash {archive:#010X}"));
        let mut file = File::open(&path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys).expect("the archive opens");
        walk(&mut file, &keys, (&mount, archive, 0), &mut census);
    }

    let c = &census;
    for (archive, bodies) in &c.bodies {
        println!("archive {archive:#010X}: {bodies} table-form bodies");
    }
    println!(
        "{} bodies with a state table ({} whose table does not read); {} records: {} placed, \
         {} records of another table, {} other ({} elements), {} owning nothing",
        c.with_tables,
        c.tables_refused,
        c.records,
        c.placed,
        c.table_runs,
        c.other,
        c.other_elements,
        c.empty
    );
    println!(
        "Declared::elements reads {} of {} other records ({} elements of {}) and refuses {}",
        c.read,
        c.other,
        c.read_elements,
        c.other_elements,
        c.other - c.read
    );
    for (why, n) in &c.refused {
        println!("  refused {n}: {why}");
    }
    for (size, (read, refused)) in &c.by_size {
        println!("  elements of {size:#X} bytes: {read} records read, {refused} refused");
    }

    for (archive, bodies) in ARCHIVES {
        assert_eq!(
            c.bodies.get(&archive).copied().unwrap_or(0),
            bodies,
            "table-form bodies of archive {archive:#010X}"
        );
    }
    let measured = [
        ("bodies with a state table", c.with_tables, 2_294),
        ("records", c.records, 45_397),
        ("placed runs", c.placed, 37_076),
        ("records of another table", c.table_runs, 120),
        ("other records", c.other, 8_201),
        ("elements of the other records", c.other_elements, 46_900),
    ];
    for (name, found, expected) in measured {
        assert_eq!(found, expected, "{name}");
    }
}
