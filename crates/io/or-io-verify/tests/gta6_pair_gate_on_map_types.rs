//! Whether the pair gate takes a pair added to or removed from a GTA VI map types body: over every
//! clear, chunked map types row of a build, the first map its tree holds gains a pair and loses
//! one, each saved through `write_grown` and read back, and each outcome is counted, a refusal by
//! the reason the save gives. Measured on the Xbox build 1.0.2.1: of its 6,308 clear map types rows
//! of 4 MiB or less, none holds a map with a pair, so the gate is never met there; that is what is
//! asserted, and a build that holds one prints the gate's counts and fails, to be read.
//!
//! Install-gated, game-derived and slow (it walks nested archives); it skips without:
//!   `OPENRAGE_GTA6_KEYS`   the `gtavi_magic.dat` key snapshot (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use or_io::container::rpf::gta6_chunk_table::entry_chunks;
use or_io::container::rpf::{
    read_rpf9_body, Gta6Class, Gta6Mount, GtaViKeys, Rpf9Sections, GTA6_SNAPSHOT_ENV,
};
use or_io::meta::binary::hash_index::title_names;
use or_io::meta::binary::rbm::gta6 as headerless;
use or_io::meta::binary::rbm::gta6_grow_tree::write_grown;
use or_io::meta::binary::ContainerLayout;
use or_io::meta::instance::HashedString;
use or_io::meta::{MetaInstance, Value};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// A body larger than this is not read.
const CAP: u64 = 4 << 20;
const MAX_DEPTH: usize = 8;
/// The 16-byte zero slot a headerless body begins with.
const SLOT: u64 = 16;
/// The resource type of a map types body.
const MAP_TYPES: u32 = 0x4A;
/// What the gate's own refusal says.
const GATE: &str = "data blocks do not lay out again as it stores them";

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

/// The pairs of the first map below `value` that holds one, depth first in field order.
fn first_map(value: &mut Value) -> Option<&mut Vec<(Value, Value)>> {
    match value {
        Value::Map(pairs) if !pairs.is_empty() => Some(pairs),
        Value::Struct(inner) | Value::Pointer(Some(inner)) => first_map_in(inner),
        Value::Array(items) => items.iter_mut().find_map(first_map),
        _ => None,
    }
}

fn first_map_in(tree: &mut MetaInstance) -> Option<&mut Vec<(Value, Value)>> {
    tree.fields.iter_mut().find_map(|(_, v)| first_map(v))
}

/// How many pairs the maps below `value` hold, nested ones included.
fn pair_count(value: &Value) -> usize {
    match value {
        Value::Map(pairs) => (pairs.iter())
            .map(|(k, v)| 1 + pair_count(k) + pair_count(v))
            .sum(),
        Value::Struct(inner) | Value::Pointer(Some(inner)) => pairs_in(inner),
        Value::Array(items) => items.iter().map(pair_count).sum(),
        _ => 0,
    }
}

fn pairs_in(tree: &MetaInstance) -> usize {
    tree.fields.iter().map(|(_, v)| pair_count(v)).sum()
}

/// A key of `pairs`' kind that none of them holds, next after `key`.
fn fresh_key(key: &Value, pairs: &[(Value, Value)]) -> Option<Value> {
    let candidate = |i: u32| match key {
        Value::Hash(h) => Some(Value::Hash(HashedString::from_hash(h.hash.wrapping_add(i)))),
        Value::UInt(u) => Some(Value::UInt(u.wrapping_add(u64::from(i)))),
        Value::Int(n) => Some(Value::Int(n.wrapping_add(i64::from(i)))),
        _ => None,
    };
    let held = |k: &Value| {
        (pairs.iter()).any(|(p, _)| match (p, k) {
            (Value::Hash(a), Value::Hash(b)) => a.hash == b.hash,
            _ => p == k,
        })
    };
    (1..=pairs.len() as u32 + 1)
        .map_while(candidate)
        .find(|k| !held(k))
}

#[derive(Default)]
struct Census {
    /// Rows met and not measured, by reason.
    skipped: BTreeMap<String, usize>,
    measured: usize,
    /// Per edit, saves taken and read back, and refusals by reason.
    outcomes: BTreeMap<(&'static str, String), usize>,
}

impl Census {
    fn skip(&mut self, why: impl Into<String>) {
        *self.skipped.entry(why.into()).or_default() += 1;
    }

    fn count(&mut self, edit: &'static str, outcome: String) {
        *self.outcomes.entry((edit, outcome)).or_default() += 1;
    }

    /// Open the map types body `sections`, then save it with a pair added and with one removed.
    fn body(&mut self, sections: &Rpf9Sections) {
        let opened = headerless::open(&sections.virtual_section, &sections.physical);
        let (tree, layout) = match opened {
            Ok((tree, layout)) => match layout.container {
                ContainerLayout::Rbm(layout) => (tree, layout),
                _ => return self.skip("it opens as no headerless model"),
            },
            Err(e) => return self.skip(format!("it does not open: {}", masked(e))),
        };
        let mut probe = tree.clone();
        let Some(pairs) = first_map_in(&mut probe) else {
            return self.skip("its tree holds no map with a pair");
        };
        self.measured += 1;
        let last = pairs[pairs.len() - 1].clone();
        let added = fresh_key(&last.0, pairs).map(|key| (key, last.1.clone()));
        let total = pairs_in(&tree);
        let removed = total - 1 - pair_count(&last.0) - pair_count(&last.1);
        let mut edits = vec![("removed", removed, None)];
        match added {
            Some(pair) => edits.push(("added", total + 1 + pair_count(&last.1), Some(pair))),
            None => self.count(
                "added",
                "not tried: a key of a kind with no next value".into(),
            ),
        }
        for (edit, want, pair) in edits {
            let mut edited = tree.clone();
            let Some(pairs) = first_map_in(&mut edited) else {
                continue;
            };
            match pair {
                Some(pair) => pairs.push(pair),
                None => {
                    pairs.pop();
                }
            }
            let outcome = match write_grown(&layout, &edited) {
                Ok((v, p)) => {
                    let back = headerless::open(&v, &p).map(|(back, _)| pairs_in(&back));
                    match back.ok() == Some(want) {
                        true => "taken, and read back".to_string(),
                        false => "taken, and read back wrong".to_string(),
                    }
                }
                Err(e) if e.to_string().contains(GATE) => {
                    format!("refused by the gate: {}", masked(e))
                }
                Err(e) => format!("refused: {}", masked(e)),
            };
            self.count(edit, outcome);
        }
    }

    fn report(&self) {
        println!("map types bodies measured: {}", self.measured);
        for (why, n) in &self.skipped {
            println!("not measured, {n}: {why}");
        }
        for ((edit, outcome), n) in &self.outcomes {
            println!("pair {edit}, {n}: {outcome}");
        }
    }
}

/// Measure every map types body of `mount` and of every archive nested in it.
fn walk(file: &mut File, keys: &GtaViKeys, mount: &Gta6Mount, depth: usize, census: &mut Census) {
    let extras = mount.archive.entry_extras();
    for (entry, extra) in mount.archive.entries.iter().zip(&extras) {
        if entry.kind() == Some(Gta6Class::NestedArchive) {
            if depth < MAX_DEPTH {
                if let Ok(child) = mount.open_nested(&mut *file, entry, keys) {
                    walk(file, keys, &child, depth + 1, census);
                }
            }
            continue;
        }
        let Some(row) = extra.nested.filter(|row| row.resource_type == MAP_TYPES) else {
            continue;
        };
        if entry.cipher.is_some() {
            census.skip("its row is ciphered");
            continue;
        }
        if row.stored_size() > CAP {
            census.skip(format!("it is stored larger than {CAP} bytes"));
            continue;
        }
        if !matches!(
            entry_chunks(&mount.archive, entry.index as usize),
            Ok(Some(_))
        ) {
            census.skip("its row has no chunk table");
            continue;
        }
        let sections = (mount.absolute_start(entry))
            .and_then(|start| read_rpf9_body(&mut *file, start + SLOT, CAP).ok())
            .and_then(|(find, bytes)| Rpf9Sections::of(&find, &bytes));
        match sections {
            Some(sections) => census.body(&sections),
            None => census.skip("its body reads as no resource"),
        }
    }
}

#[test]
fn the_pair_gate_is_counted_over_every_map_types_body_of_a_build() {
    let what = "GTA VI pair gate over the map types bodies of a build";
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
    // Types are named as the reader names them, which is under the second title's names.
    let _names = title_names();
    let mut census = Census::default();
    for path in &archives {
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys).expect("the archive opens");
        walk(&mut file, &keys, &mount, 0, &mut census);
    }

    census.report();
    let met: usize = census.skipped.values().sum::<usize>() + census.measured;
    assert!(met > 0, "the build holds no clear map types body");
    assert_eq!(
        census.measured, 0,
        "a map types body holds a map with a pair: the gate's counts above are new"
    );
}
