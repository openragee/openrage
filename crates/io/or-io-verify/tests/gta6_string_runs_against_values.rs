//! Whether a GTA VI string run the game registers no text for grows with its namespace: per
//! table-form PSO and headerless `0DRP` document of a bounded sample of a build, the distinct
//! values each namespace holds against the length of its run, printed; only the layout facts
//! measured over the whole build are asserted.
//!
//! Install-gated, game-derived and slow (it walks nested archives); it skips without:
//!   `OPENRAGE_GTA6_KEYS`   the `gtavi_magic.dat` key snapshot (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use flate2::read::ZlibDecoder;
use or_io::container::rpf::gta6_chunk_table::entry_chunks;
use or_io::container::rpf::{
    read_rpf9_body, Gta6Class, Gta6Mount, GtaViKeys, Rpf9Sections, GTA6_SNAPSHOT_ENV,
};
use or_io::meta::binary::hash_index::{default_hash_index, title_names};
use or_io::meta::binary::pso::{reader, string_blob};
use or_io::meta::binary::rbm::gta6 as headerless;
use or_io::meta::binary::rbm::schema::RbmModel;
use or_io::meta::binary::ContainerLayout;
use or_io::meta::{MetaInstance, SchemaLibrary, Value};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// A body larger than this is not sampled.
const CAP: u64 = 4 << 20;
/// Documents of each form taken from one top-level archive, and in all.
const PER_ARCHIVE: usize = 120;
const SAMPLE: usize = 1500;
const MAX_DEPTH: usize = 8;
/// The 16-byte zero slot a headerless body begins with.
const SLOT: u64 = 16;
/// The marker at +0x10 of a headerless metadata body's virtual section.
const PRD0: &[u8] = b"0DRP";
/// The one section order every measured table-form document has.
const SECTIONS: [&[u8; 4]; 7] = [
    b"PSIN", b"LSCH", b"LENM", b"LSNS", b"LGID", b"LRSC", b"CHKS",
];
const TABLE_FORM: &str = "table form";
const DRP: &str = "0DRP";
/// The resource type of a map types body.
const MAP_TYPES: u32 = 0x4A;
/// The name of a type-table row that names no member: an element or a map's key or value.
const ANONYMOUS: u32 = 0x100;
const HASH: u8 = 0x4A;
const NS_HASH: u8 = 0x4D;
const WIDE_HASH: u8 = 0x4F;
/// The key-only hash sets, whose element row is the key index of +0xC.
const SLOT_SET: u8 = 0x76;
const GROUP_SET: u8 = 0x77;

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

fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// The first inflated bytes of the body at `at`: its first chunk inflated, or as stored.
fn head_at(src: &mut File, at: u64) -> Option<Vec<u8>> {
    let mut stored = Vec::new();
    src.seek(SeekFrom::Start(at)).ok()?;
    (&mut *src).take(4096).read_to_end(&mut stored).ok()?;
    let mut head = Vec::new();
    // A chunk cut short by the read still yields its first bytes before the error.
    let _ = ZlibDecoder::new(&stored[..])
        .take(64)
        .read_to_end(&mut head);
    if head.is_empty() {
        stored.truncate(64);
        return Some(stored);
    }
    Some(head)
}

/// The sections a read past the zero slot of a headerless body at `start` returns.
fn sections_at(src: &mut File, start: u64) -> Option<Rpf9Sections> {
    let (find, bytes) = read_rpf9_body(&mut *src, start + SLOT, CAP).ok()?;
    Rpf9Sections::of(&find, &bytes)
}

/// The bodies of a table-form document's seven sections, `None` for any other document.
fn table_sections(doc: &[u8]) -> Option<Vec<&[u8]>> {
    let mut at = 0usize;
    SECTIONS
        .iter()
        .map(|tag| {
            let frame = doc.get(at..at.checked_add(8)?)?;
            let len = u32::from_be_bytes([frame[4], frame[5], frame[6], frame[7]]) as usize;
            if frame[..4] != tag[..] || len < 8 {
                return None;
            }
            let body = doc.get(at + 8..at.checked_add(len)?)?;
            at += len;
            Some(body)
        })
        .collect()
}

/// One member row of a type table: its name, type code, byte 9 (a hash's namespace), and the
/// words at +0xA and +0xC that index an element row or name a class.
#[derive(Clone, Copy)]
struct Row {
    name: u32,
    code: u8,
    ns: u8,
    ah: u16,
    ch: u32,
}

/// A declared type: every row, for the indices a member states, and the named rows with the name
/// the value tree gives each one's field, in order.
struct Type {
    rows: Vec<Row>,
    named: Vec<(String, Row)>,
}

/// The types of a table-form document: `LRSC` counts them, `LSCH` holds their 32-byte headers and
/// then every type's 16-byte rows in header order.
fn lsch_types(lsch: &[u8], lrsc: &[u8]) -> Option<Vec<(u32, Vec<Row>)>> {
    let count = lrsc.get(0x38..0x3A)?;
    let classes = usize::from(u16::from_le_bytes([count[0], count[1]]));
    let headers = lsch.get(..32 * classes)?;
    let mut at = headers.len();
    let mut out = Vec::with_capacity(classes);
    for header in headers.as_chunks::<32>().0 {
        let members = usize::from(u16::from_le_bytes([header[0x1E], header[0x1F]]));
        let mut rows = Vec::with_capacity(members);
        for _ in 0..members {
            let m = lsch.get(at..at + 16)?;
            at += 16;
            rows.push(Row {
                name: le32(m, 0),
                code: m[8],
                ns: m[9],
                ah: u16::from_le_bytes([m[10], m[11]]),
                ch: le32(m, 12),
            });
        }
        out.push((le32(header, 0), rows));
    }
    Some(out)
}

/// The types of a headerless body's model.
fn model_types(model: &RbmModel) -> Vec<(u32, Vec<Row>)> {
    (model.classes.iter())
        .map(|(&hash, class)| {
            let rows = (class.entries.iter())
                .map(|e| Row {
                    name: e.name_hash,
                    code: e.data_type.to_byte(),
                    // Byte 9 whole: the array accessors give its low five bits and its top three.
                    ns: (e.array_flags() << 5) | e.array_align_power(),
                    ah: e.contents_index(),
                    ch: e.referent_hash(),
                })
                .collect();
            (hash, rows)
        })
        .collect()
}

/// The types by the name the value tree gives each, as the reader names them.
fn types_of(classes: Vec<(u32, Vec<Row>)>) -> HashMap<String, Type> {
    let index = default_hash_index();
    (classes.into_iter())
        .map(|(hash, rows)| {
            let named = (rows.iter())
                .filter(|r| r.name != ANONYMOUS)
                .map(|r| (index.name_or_literal(r.name), *r))
                .collect();
            (index.name_or_literal(hash), Type { rows, named })
        })
        .collect()
}

#[derive(Hash, PartialEq, Eq)]
enum Key {
    Narrow(u32),
    Wide(u64),
    Text(String),
}

/// The distinct values of a document by namespace: `(1, byte 9)` a 0x4D hash, `(1, 1)` a 0x4A
/// one, `(2, byte 9)` a 0x4F wide hash. Run `kind << 8 | (byte 9 + 1)` is the namespace's.
// ⚠ Kind 1 is the measured rule (0x4D byte 9 into `0x101 + byte 9`, 0x4A into 0x102); pairing
// 0x4F with the kind-2 runs is a hypothesis the printed counts test.
type Spaces = HashMap<(u8, u8), HashSet<Key>>;

/// The values below `instance`, `None` where a structure's fields are not its type's named rows.
fn spaces_of(types: &HashMap<String, Type>, instance: &MetaInstance) -> Option<Spaces> {
    let mut spaces = Spaces::new();
    walk_struct(types, instance, &mut spaces).then_some(spaces)
}

fn walk_struct(types: &HashMap<String, Type>, instance: &MetaInstance, out: &mut Spaces) -> bool {
    let Some(ty) = types.get(&instance.type_name) else {
        return false;
    };
    ty.named.len() == instance.fields.len()
        && (ty.named.iter().zip(&instance.fields)).all(|((name, row), (field, value))| {
            name == field && walk_value(types, ty, *row, value, out)
        })
}

fn put(out: &mut Spaces, space: (u8, u8), key: Key) -> bool {
    out.entry(space).or_default().insert(key);
    true
}

/// The values of `value`, which `row` of `ty` declares.
fn walk_value(
    types: &HashMap<String, Type>,
    ty: &Type,
    row: Row,
    value: &Value,
    out: &mut Spaces,
) -> bool {
    match value {
        Value::Hash(h) if row.code == NS_HASH => put(out, (1, row.ns), Key::Narrow(h.hash)),
        Value::Hash(h) if row.code == HASH => put(out, (1, 1), Key::Narrow(h.hash)),
        Value::UInt(u) if row.code == WIDE_HASH => put(out, (2, row.ns), Key::Wide(*u)),
        Value::String(s) if row.code == WIDE_HASH => put(out, (2, row.ns), Key::Text(s.clone())),
        Value::Struct(inner) | Value::Pointer(Some(inner)) => walk_struct(types, inner, out),
        Value::Array(items) => {
            let index = match row.code {
                SLOT_SET | GROUP_SET => row.ch & 0xFFFF,
                _ => u32::from(row.ah),
            };
            let Some(&element) = ty.rows.get(index as usize) else {
                return false;
            };
            (items.iter()).all(|item| walk_value(types, ty, element, item, out))
        }
        Value::Map(pairs) => {
            let rows = (
                ty.rows.get((row.ch & 0xFFFF) as usize),
                ty.rows.get((row.ch >> 16) as usize),
            );
            let (Some(&key), Some(&val)) = rows else {
                return false;
            };
            (pairs.iter()).all(|(k, v)| {
                walk_value(types, ty, key, k, out) && walk_value(types, ty, val, v, out)
            })
        }
        _ => true,
    }
}

/// A run as its document states it: its id, its length and, in a headerless table, the u16
/// between the kind byte and the length.
struct RunRow {
    id: u32,
    len: u32,
    word: Option<u16>,
}

/// The runs a headerless body's string table states, from the pointer at +0x40 of its virtual
/// section: `{u32 count; count x {u8 namespace, u8 kind, u16, u32 len}; blobs}`. `None` where
/// the table is not inside the section.
fn headerless_runs(v: &[u8]) -> Option<Vec<RunRow>> {
    let ptr = u64::from_le_bytes(v.get(0x40..0x48)?.try_into().ok()?);
    if ptr == 0 {
        return Some(Vec::new());
    }
    if ptr >> 28 != 5 {
        return None;
    }
    let at = (ptr & 0x0FFF_FFFF) as usize;
    let count = le32(v.get(at..at + 4)?, 0) as usize;
    let entries = v.get(at + 4..(at + 4).checked_add(count.checked_mul(8)?)?)?;
    let rows = (entries.as_chunks::<8>().0.iter()).map(|e| RunRow {
        id: (u32::from(e[1]) << 8) | u32::from(e[0]),
        len: le32(e, 4),
        word: Some(u16::from_le_bytes([e[2], e[3]])),
    });
    Some(rows.collect())
}

/// A message with its digits masked, so reasons that differ only by an offset tally together.
fn masked(message: impl std::fmt::Display) -> String {
    (message.to_string().chars())
        .map(|c| if c.is_ascii_digit() { '#' } else { c })
        .collect()
}

#[derive(Default)]
struct Census {
    library: SchemaLibrary,
    docs: BTreeMap<&'static str, usize>,
    read: BTreeMap<&'static str, usize>,
    /// Documents whose value tree does not read, by form and reason.
    unread: BTreeMap<(&'static str, String), usize>,
    /// Documents whose value tree is not the shape its type table declares.
    misread: BTreeMap<&'static str, usize>,
    /// `(distinct values, run length)` per run, by form and run id.
    pairs: BTreeMap<(&'static str, u32), Vec<(usize, u32)>>,
    /// Documents holding values of a namespace with its run, and without it.
    held: BTreeMap<(&'static str, u8, u8), (usize, usize)>,
    /// Headerless runs whose u16 equals their namespace's distinct values, of all.
    words: (usize, usize),
    failures: Vec<String>,
}

impl Census {
    fn fail(&mut self, what: &str, why: String) {
        self.failures.push(format!("{what}: {why}"));
    }

    fn add(&mut self, form: &'static str, runs: &[RunRow], spaces: &Spaces) {
        *self.read.entry(form).or_default() += 1;
        for run in runs {
            let kind = (run.id >> 8) as u8;
            let values = ((run.id & 0xFF) as u8).checked_sub(1);
            let values = values.and_then(|b| spaces.get(&(kind, b)));
            let distinct = values.map_or(0, HashSet::len);
            let pairs = self.pairs.entry((form, run.id)).or_default();
            pairs.push((distinct, run.len));
            if let Some(word) = run.word {
                self.words.0 += usize::from(usize::from(word) == distinct);
                self.words.1 += 1;
            }
        }
        for (&(kind, b), values) in spaces {
            if values.is_empty() {
                continue;
            }
            let id = (u32::from(kind) << 8) + u32::from(b) + 1;
            let tally = self.held.entry((form, kind, b)).or_default();
            match runs.iter().any(|r| r.id == id) {
                true => tally.0 += 1,
                false => tally.1 += 1,
            }
        }
    }

    fn table_form(&mut self, what: &str, doc: &[u8], sections: &[&[u8]]) {
        *self.docs.entry(TABLE_FORM).or_default() += 1;
        let lsns = sections[3];
        let runs = match string_blob::parse(lsns) {
            Ok(runs) => runs,
            Err(e) => return self.fail(what, format!("its LSNS blob does not parse: {e}")),
        };
        let end = 4 + 8 * runs.len() + runs.iter().map(|r| r.len as usize).sum::<usize>();
        if end != lsns.len() {
            self.fail(
                what,
                format!(
                    "its LSNS blob of {} bytes ends its runs at {end}",
                    lsns.len()
                ),
            );
        }
        if runs.windows(2).any(|w| w[0].id >= w[1].id) {
            self.fail(what, "its LSNS ids do not ascend".into());
        }
        if let Some(run) = runs.iter().find(|r| r.len % 16 != 0) {
            let why = format!("run {:#X} is {} bytes, not whole blocks", run.id, run.len);
            self.fail(what, why);
        }
        let rows: Vec<RunRow> = (runs.iter())
            .map(|r| RunRow {
                id: r.id,
                len: r.len,
                word: None,
            })
            .collect();
        let Some(classes) = lsch_types(sections[1], sections[5]) else {
            let why = "its type table does not read".to_string();
            *self.unread.entry((TABLE_FORM, why)).or_default() += 1;
            return;
        };
        let instance = match reader::parse(&self.library, doc) {
            Ok(instance) => instance,
            Err(e) => {
                *self.unread.entry((TABLE_FORM, masked(e))).or_default() += 1;
                return;
            }
        };
        let Some(spaces) = spaces_of(&types_of(classes), &instance) else {
            *self.misread.entry(TABLE_FORM).or_default() += 1;
            return;
        };
        let has = |id: u32| rows.iter().any(|r| r.id == id);
        for (&(kind, b), values) in &spaces {
            let missing = match (kind, b) {
                _ if values.is_empty() => None,
                (1, 27) if !has(0x11C) => Some("byte 9 = 27 values and no run 0x11C".to_string()),
                (1, 30) if !has(0x11F) => Some("byte 9 = 30 values and no run 0x11F".to_string()),
                (2, b) if has(0x101 + u32::from(b)) => Some(format!(
                    "wide values of byte 9 = {b} and run {:#X}",
                    0x101 + u32::from(b)
                )),
                _ => None,
            };
            if let Some(why) = missing {
                self.fail(what, format!("it holds {why}"));
            }
        }
        self.add(TABLE_FORM, &rows, &spaces);
    }

    fn headerless(&mut self, what: &str, sections: &Rpf9Sections, code: u32) {
        *self.docs.entry(DRP).or_default() += 1;
        let virtual_section = &sections.virtual_section;
        let Some(runs) = headerless_runs(virtual_section) else {
            let why = "its string table is not inside the virtual section".to_string();
            *self.unread.entry((DRP, why)).or_default() += 1;
            return;
        };
        if let Some(run) = (runs.iter()).find(|r| r.id < 0x100 || r.id == 0x102 || r.id == 0x11F) {
            self.fail(what, format!("its string table states run {:#X}", run.id));
        }
        if code == MAP_TYPES && runs.is_empty() {
            self.fail(what, "a map types body has no table with runs".into());
        }
        let model = match headerless::open(virtual_section, &sections.physical) {
            Ok((instance, layout)) => match layout.container {
                ContainerLayout::Rbm(rbm) => Some((instance, rbm.model)),
                _ => None,
            },
            Err(e) => {
                *self.unread.entry((DRP, masked(e))).or_default() += 1;
                return;
            }
        };
        let Some((instance, model)) = model else {
            let why = "it opens as no headerless model".to_string();
            *self.unread.entry((DRP, why)).or_default() += 1;
            return;
        };
        let Some(spaces) = spaces_of(&types_of(model_types(&model)), &instance) else {
            *self.misread.entry(DRP).or_default() += 1;
            return;
        };
        self.add(DRP, &runs, &spaces);
    }

    fn report(&self) {
        for (form, docs) in &self.docs {
            println!(
                "{form}: {docs} documents sampled, {} value trees read, {} not the shape their type table declares",
                self.read.get(form).copied().unwrap_or(0),
                self.misread.get(form).copied().unwrap_or(0),
            );
        }
        for ((form, why), n) in &self.unread {
            println!("{form}: {n} value trees do not read: {why}");
        }
        for ((form, id), pairs) in &self.pairs {
            let mut per_value: Vec<f64> = (pairs.iter())
                .filter(|p| p.0 > 0)
                .map(|&(d, len)| f64::from(len) / d as f64)
                .collect();
            per_value.sort_by(f64::total_cmp);
            let spread = match per_value.as_slice() {
                [] => "no run with values".to_string(),
                p => format!(
                    "bytes per value {:.1} / {:.1} / {:.1} (least, median, most)",
                    p[0],
                    p[p.len() / 2],
                    p[p.len() - 1]
                ),
            };
            let mut lengths: HashMap<usize, (usize, HashSet<u32>)> = HashMap::new();
            for &(d, len) in pairs {
                let seen = lengths.entry(d).or_default();
                seen.0 += 1;
                seen.1.insert(len);
            }
            let repeated: Vec<_> = lengths.values().filter(|(n, _)| *n >= 2).collect();
            let fixed = repeated.iter().filter(|(_, lens)| lens.len() == 1).count();
            println!(
                "{form} run {id:#06X}: {} runs, {} with no value in their namespace; {spread}; \
                 r(values, length) {:.3}; one length per value count in {fixed} of {} counts met twice or more",
                pairs.len(),
                pairs.len() - per_value.len(),
                pearson(pairs),
                repeated.len(),
            );
        }
        for ((form, kind, b), (present, absent)) in &self.held {
            let code = match *kind {
                1 if *b == 1 => "0x4A or 0x4D",
                1 => "0x4D",
                _ => "0x4F",
            };
            let id = (u32::from(*kind) << 8) + u32::from(*b) + 1;
            println!(
                "{form}: {} documents hold {code} values of namespace {b}; run {id:#06X} is in {present}, not in {absent}",
                present + absent
            );
        }
        println!(
            "{DRP}: the u16 of a string table entry equals its namespace's distinct values in {} of {} entries",
            self.words.0, self.words.1
        );
    }
}

fn pearson(pairs: &[(usize, u32)]) -> f64 {
    let n = pairs.len() as f64;
    let (mx, my) = (pairs.iter()).fold((0.0, 0.0), |(x, y), &(d, len)| {
        (x + d as f64 / n, y + f64::from(len) / n)
    });
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for &(d, len) in pairs {
        let (dx, dy) = (d as f64 - mx, f64::from(len) - my);
        sxy += dx * dy;
        sxx += dx * dx;
        syy += dy * dy;
    }
    sxy / (sxx * syy).sqrt()
}

/// How many documents of each form are taken, from the current top-level archive and in all.
#[derive(Default)]
struct Quota {
    archive: [usize; 2],
    total: [usize; 2],
}

impl Quota {
    fn open(&self, form: usize) -> bool {
        self.archive[form] < PER_ARCHIVE && self.total[form] < SAMPLE
    }

    fn take(&mut self, form: usize) {
        self.archive[form] += 1;
        self.total[form] += 1;
    }
}

const TABLE: usize = 0;
const HEADERLESS: usize = 1;

/// Sample the documents of `mount`, an archive of the top-level archive `label`, and of every
/// archive nested in it.
fn walk(
    file: &mut File,
    keys: &GtaViKeys,
    (mount, label, depth): (&Gta6Mount, &str, usize),
    quota: &mut Quota,
    census: &mut Census,
) {
    let extras = mount.archive.entry_extras();
    for (entry, extra) in mount.archive.entries.iter().zip(&extras) {
        if !quota.open(TABLE) && !quota.open(HEADERLESS) {
            return;
        }
        if entry.kind() == Some(Gta6Class::NestedArchive) {
            if depth < MAX_DEPTH {
                if let Ok(child) = mount.open_nested(&mut *file, entry, keys) {
                    walk(file, keys, (&child, label, depth + 1), quota, census);
                }
            }
            continue;
        }
        let Some(start) = mount.absolute_start(entry) else {
            continue;
        };
        let what = format!("{label}, entry {} at depth {depth}", entry.index);
        let marked = |bytes: &[u8]| bytes.get(0x10..0x14) == Some(PRD0);
        match extra.nested {
            Some(row) => {
                let chunked = matches!(
                    entry_chunks(&mount.archive, entry.index as usize),
                    Ok(Some(_))
                );
                if !quota.open(HEADERLESS)
                    || entry.cipher.is_some()
                    || row.stored_size() > CAP
                    || !chunked
                    || !head_at(file, start + SLOT).is_some_and(|h| marked(&h))
                {
                    continue;
                }
                let Some(sections) = sections_at(file, start) else {
                    continue;
                };
                if marked(&sections.virtual_section) {
                    quota.take(HEADERLESS);
                    census.headerless(&what, &sections, row.resource_type);
                }
            }
            None => {
                if !quota.open(TABLE) || entry.real.is_none_or(|r| r > CAP) {
                    continue;
                }
                if entry.cipher.is_none()
                    && !head_at(file, start).is_some_and(|h| h.starts_with(b"PSIN"))
                {
                    continue;
                }
                let Ok(doc) = mount.read_entry_decrypted(&mut *file, entry, keys) else {
                    continue;
                };
                if let Some(sections) = table_sections(&doc) {
                    quota.take(TABLE);
                    census.table_form(&what, &doc, &sections);
                }
            }
        }
    }
}

#[test]
fn an_unregistered_string_run_is_measured_against_the_values_of_its_namespace() {
    let what = "GTA VI string runs against the values of their namespaces";
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
    let (mut quota, mut census) = (Quota::default(), Census::default());
    for path in &archives {
        if quota.total[TABLE] >= SAMPLE && quota.total[HEADERLESS] >= SAMPLE {
            break;
        }
        quota.archive = [0; 2];
        let label = (path.file_name()).map_or_else(String::new, |n| n.to_string_lossy().into());
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys).expect("the archive opens");
        walk(
            &mut file,
            &keys,
            (&mount, &label, 0),
            &mut quota,
            &mut census,
        );
    }

    census.report();
    for failure in census.failures.iter().take(20) {
        eprintln!("  {failure}");
    }
    for form in [TABLE_FORM, DRP] {
        assert!(
            census.read.get(form).is_some_and(|&n| n > 0),
            "the sample holds no {form} document whose values read"
        );
    }
    assert!(
        census.failures.is_empty(),
        "{} documents break a measured layout fact",
        census.failures.len()
    );
}
