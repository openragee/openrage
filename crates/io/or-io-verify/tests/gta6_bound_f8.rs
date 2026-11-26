//! The record a type-4 geometry child of a GTA VI bound names at +0xF8, over every fragment body of
//! a build and a sample of its bound and drawable bodies, within a deadline: how many children set the word, by the body's kind, and the
//! record's layout as the copy lays it out again. Each record is 0x50 bytes: one count four times
//! (the count and capacity of two arrays), the arrays of 6-byte elements back to back or the second
//! at a later 64 KiB multiple, no word naming a byte inside either, at +0x20 a 16-aligned record
//! outside the child, at +0x28 the child itself, then a word and a float and 24 zero bytes. The
//! count is at least the child's vertex count, and the first array begins with the child's
//! vertices. Where the holder lets a child be copied, it is, and the copy is read back; on the
//! build every such child is a fragment's, whose composite children are not copied.
//!
//! Install-gated, game-derived and slow (it walks every nested archive); it skips without:
//!   `OPENRAGE_GTA6_KEYS`   the `gtavi_magic.dat` key snapshot (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use or_io::container::rpf::gta6_chunk_table::entry_chunks;
use or_io::container::rpf::{
    read_rpf9_body, Gta6Class, Gta6Entry, Gta6Mount, Gta6NestedRow, GtaViKeys, Rpf9Sections,
    GTA6_SNAPSHOT_ENV,
};
use or_io::gta6::headerless::{reshape_bound_body, BoundHolder, BoundReshape};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// A body stored larger than this is not read.
const CAP: u64 = 64 << 20;
const MAX_DEPTH: usize = 8;
/// The 16-byte zero slot a headerless body begins with.
const SLOT: u64 = 16;
const DRAWABLE: u32 = 0x02;
const FRAGMENT: u32 = 0x03;
const BOUND: u32 = 0x06;
const COMPOSITE: u8 = 6;
const PLAIN_GEOMETRY: u8 = 4;
const PLAIN_RECORD: usize = 0x130;
const RECORD: usize = 0x50;
/// The bytes of an element of either array, as measured.
const ELEMENT: usize = 6;
/// Where the second array starts in the 3 records it does not follow the first.
const BLOCK: usize = 0x1_0000;
/// Measured on the Xbox build 1.0.2.1 by this suite over every fragment body: the bodies read, their
/// type-4 geometry children whose +0xF8 is set, and the records among them whose next named byte (or
/// the section's end) lies within the second array's padding. No drawable's or bound's sets it.
const FRAGMENTS_READ: usize = 55_488;
const MEASURED: usize = 11_003;
const ELEMENT_SHOWN: usize = 10_802;
/// Drawable and bound bodies read: every fragment is read, these kinds only for the "none sets it" check.
const SAMPLE: usize = 4_000;
/// The walk stops past this, and the exact count is then not asserted.
const DEADLINE: Duration = Duration::from_secs(600);

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

fn u32_at(v: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([v[at], v[at + 1], v[at + 2], v[at + 3]])
}

/// The virtual-section offset the word at `at` aims at; `None` for null, any other word, or a
/// word past the section.
fn ptr(v: &[u8], at: usize) -> Option<usize> {
    let word = u64::from_le_bytes(v.get(at..at.checked_add(8)?)?.try_into().ok()?);
    let to = (word & 0x0FFF_FFFF) as usize;
    (word >> 28 == 5 && to < v.len()).then_some(to)
}

/// The bound a body of `kind` holds, and the holder an edit names it by.
fn root(v: &[u8], kind: u32) -> Option<(usize, BoundHolder)> {
    match kind {
        BOUND => Some((0, BoundHolder::Bound)),
        DRAWABLE => Some((ptr(v, 0xC8)?, BoundHolder::Drawable)),
        _ => Some((ptr(v, ptr(v, 0x10)? + 0x20)?, BoundHolder::Fragment)),
    }
}

/// The composite at `at`'s children by index, or the lone bound at `at` as child 0.
fn children(v: &[u8], at: usize) -> Vec<(usize, usize)> {
    if at + 0xA0 > v.len() {
        return Vec::new();
    }
    if v[at + 0x10] != COMPOSITE {
        return vec![(0, at)];
    }
    let n = usize::from(u16::from_le_bytes([v[at + 0x98], v[at + 0x99]]));
    let Some(list) = ptr(v, at + 0x70) else {
        return Vec::new();
    };
    (0..n)
        .filter_map(|i| Some((i, ptr(v, list + 8 * i)?)))
        .collect()
}

/// The stored bytes of entry `e`'s chunks: deciphered where it is enciphered, past the zero slot
/// for a 20-byte row.
fn stored_of(
    mount: &Gta6Mount,
    src: &mut File,
    keys: &GtaViKeys,
    e: &Gta6Entry,
    row: Gta6NestedRow,
) -> Result<Vec<u8>, String> {
    if e.cipher.is_some() {
        return (mount.read_entry_deciphered_stored(&mut *src, e, keys)).map_err(|e| e.to_string());
    }
    let start = mount.absolute_start(e).ok_or("the entry has no start")?;
    let mut body = vec![0u8; row.stored_size().saturating_sub(SLOT) as usize];
    src.seek(SeekFrom::Start(start + SLOT))
        .map_err(|e| e.to_string())?;
    src.read_exact(&mut body).map_err(|e| e.to_string())?;
    Ok(body)
}

struct Census {
    /// When the walk stops reading bodies.
    until: Instant,
    /// Per body kind: bodies read, bodies not read.
    bodies: BTreeMap<u32, [usize; 2]>,
    /// Per body kind: type-4 children, those whose +0xF8 is set.
    plain: BTreeMap<u32, [usize; 2]>,
    /// Records whose count is the child's vertex count, and more than it.
    count_is_vertices: [usize; 2],
    /// Records whose second array ends at the next byte any word names.
    second_ends_at_next: usize,
    /// Records whose next named byte, or the section's end, lies within the second array's padding.
    element_shown: usize,
    /// Records whose second array starts the next 64 KiB block rather than follow the first.
    second_starts_a_block: usize,
    off_layout: Vec<String>,
    /// Copies made and read back, and copies the holder refused, by reason.
    copied: usize,
    refused: BTreeMap<String, usize>,
}

/// Why the record the child at `c` names at +0xF8 is off the measured layout; `None` where it holds.
fn off_layout(v: &[u8], c: usize, r: usize, targets: &[usize]) -> Option<String> {
    if !r.is_multiple_of(16) || r + RECORD > v.len() {
        return Some("the record is not 16-aligned inside the section".into());
    }
    let n = u32_at(v, r);
    if [4, 0x10, 0x14].iter().any(|o| u32_at(v, r + o) != n) {
        return Some("the four counts differ".into());
    }
    let n = n as usize;
    let (Some(first), Some(second)) = (ptr(v, r + 0x08), ptr(v, r + 0x18)) else {
        return Some("an array is null".into());
    };
    // Where it does not follow on, it starts a later 64 KiB block; others that follow on cross one.
    let follows = first + ELEMENT * n;
    if (second != follows && (second < follows || second % BLOCK != 0))
        || second + ELEMENT * n > v.len()
    {
        return Some("the second array neither follows the first's 6-byte elements nor starts a later 64 KiB block".into());
    }
    for array in [first, second] {
        let next = targets.partition_point(|t| *t <= array);
        if targets.get(next).is_some_and(|t| *t < array + ELEMENT * n) {
            return Some("a word names a byte inside an array's 6-byte elements".into());
        }
    }
    if ptr(v, r + 0x28) != Some(c) {
        return Some("+0x28 does not name the child".into());
    }
    let owner = ptr(v, r + 0x20);
    if owner.is_none_or(|o| o % 16 != 0 || (c..c + PLAIN_RECORD).contains(&o)) {
        return Some("+0x20 names no 16-aligned record outside the child".into());
    }
    if v[r + 0x38..r + RECORD].iter().any(|b| *b != 0) {
        return Some("the bytes from +0x38 are not zero".into());
    }
    let vertices = u32_at(v, c + 0xD0) as usize;
    let Some(held) = ptr(v, c + 0xB0).filter(|h| h + ELEMENT * vertices <= v.len()) else {
        return Some("the child's vertices are not in the section".into());
    };
    if n < vertices || v[first..first + ELEMENT * vertices] != v[held..held + ELEMENT * vertices] {
        return Some("the first array does not begin with the child's vertices".into());
    }
    None
}

/// Child `i`, at `c`, of the composite at `at` that `holder` names in `s` copied, and the copy's
/// record checked: its own, naming the copy, the source's unchanged.
fn copy_checked(
    st: &mut Census,
    s: &Rpf9Sections,
    (at, holder): (usize, BoundHolder),
    i: usize,
    c: usize,
) {
    let mut work = s.clone();
    let op = BoundReshape::CopyChild { child: i };
    if let Err(e) = reshape_bound_body(&mut work, holder, &op) {
        let why: String = (e.to_string().chars())
            .map(|c| if c.is_ascii_digit() { '#' } else { c })
            .collect();
        *st.refused.entry(why).or_default() += 1;
        return;
    }
    let (v, w) = (&work.virtual_section, &s.virtual_section);
    let copy = children(v, at).last().map(|(_, c)| *c).expect("a child");
    let ours = ptr(v, copy + 0xF8).expect("the copy's record");
    let theirs = ptr(w, c + 0xF8).expect("the record");
    assert_ne!(ours, theirs, "the copy's +0xF8 names a record of its own");
    assert_eq!(
        ptr(v, ours + 0x28),
        Some(copy),
        "the copy's record names the copy"
    );
    assert_eq!(v[theirs..theirs + RECORD], w[theirs..theirs + RECORD]);
    assert_eq!(
        v[ours + 0x30..ours + RECORD],
        w[theirs + 0x30..theirs + RECORD]
    );
    st.copied += 1;
}

fn measure(st: &mut Census, kind: u32, s: &Rpf9Sections) {
    let v = &s.virtual_section;
    let Some((at, holder)) = root(v, kind) else {
        return;
    };
    let mut targets: Option<Vec<usize>> = None;
    for (i, c) in children(v, at) {
        if c + PLAIN_RECORD > v.len() || v[c + 0x10] != PLAIN_GEOMETRY {
            continue;
        }
        let tally = st.plain.entry(kind).or_default();
        tally[0] += 1;
        let Some(r) = ptr(v, c + 0xF8) else {
            continue;
        };
        tally[1] += 1;
        let targets = targets.get_or_insert_with(|| {
            let mut t: Vec<usize> = (0..v.len() / 8).filter_map(|k| ptr(v, 8 * k)).collect();
            t.sort_unstable();
            t.dedup();
            t
        });
        if let Some(why) = off_layout(v, c, r, targets) {
            st.off_layout
                .push(format!("kind {kind:#X}, child at {c:#X}: {why}"));
            continue;
        }
        let n = u32_at(v, r) as usize;
        st.count_is_vertices[usize::from(n != u32_at(v, c + 0xD0) as usize)] += 1;
        let (first, second) = (ptr(v, r + 0x08), ptr(v, r + 0x18));
        st.second_starts_a_block += usize::from(second != first.map(|a| a + ELEMENT * n));
        let end = second.map_or(0, |b| b + ELEMENT * n);
        let next = targets.get(targets.partition_point(|t| *t < end));
        st.second_ends_at_next += usize::from(next == Some(&end));
        // The next named byte, or the section's end, lies within the padding past the second array,
        // so that record shows its elements are no longer than 6 bytes.
        let bound = next.copied().unwrap_or(v.len());
        st.element_shown += usize::from(bound <= end.next_multiple_of(16));
        // A fragment's composite children are also its child records, so none is copied.
        if holder != BoundHolder::Fragment {
            copy_checked(st, s, (at, holder), i, c);
        }
    }
}

/// Measure every bound, drawable and fragment body of `mount` and of every archive nested in it.
fn walk(file: &mut File, keys: &GtaViKeys, mount: &Gta6Mount, depth: usize, st: &mut Census) {
    let extras = mount.archive.entry_extras();
    for (entry, extra) in mount.archive.entries.iter().zip(&extras) {
        if Instant::now() > st.until {
            return;
        }
        if entry.kind() == Some(Gta6Class::NestedArchive) {
            if depth < MAX_DEPTH {
                if let Ok(child) = mount.open_nested(&mut *file, entry, keys) {
                    walk(file, keys, &child, depth + 1, st);
                }
            }
            continue;
        }
        let Some(row) = extra.nested else {
            continue;
        };
        let kind = row.resource_type;
        if ![DRAWABLE, FRAGMENT, BOUND].contains(&kind) || row.stored_size() > CAP {
            continue;
        }
        let seen = st.bodies.get(&kind).map_or(0, |[r, u]| r + u);
        if kind != FRAGMENT && seen >= SAMPLE {
            continue;
        }
        let read = (entry_chunks(&mount.archive, entry.index as usize)
            .ok()
            .flatten())
        .zip(stored_of(mount, file, keys, entry, row).ok())
        .and_then(|(table, stored)| {
            let limit = table.iter().map(|c| u64::from(c.inflated)).sum();
            let (find, bytes) = read_rpf9_body(Cursor::new(stored), 0, limit).ok()?;
            Rpf9Sections::of(&find, &bytes)
        });
        let tally = st.bodies.entry(kind).or_default();
        match read {
            Some(s) => {
                tally[0] += 1;
                measure(st, kind, &s);
            }
            None => tally[1] += 1,
        }
    }
}

#[test]
fn the_0xf8_record_is_counted_and_laid_out_as_measured() {
    let what = "the +0xF8 record of a GTA VI type-4 geometry over a build";
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
    let mut st = Census {
        until: Instant::now() + DEADLINE,
        bodies: BTreeMap::new(),
        plain: BTreeMap::new(),
        count_is_vertices: [0; 2],
        second_ends_at_next: 0,
        element_shown: 0,
        second_starts_a_block: 0,
        off_layout: Vec::new(),
        copied: 0,
        refused: BTreeMap::new(),
    };
    for path in &archives {
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys).expect("the archive opens");
        walk(&mut file, &keys, &mount, 0, &mut st);
    }
    for (kind, [read, unread]) in &st.bodies {
        let [plain, set] = st.plain.get(kind).copied().unwrap_or_default();
        println!(
            "kind {kind:#04X}: {read} bodies read, {unread} not; {plain} type-4 children, \
             {set} with +0xF8 set"
        );
    }
    let timed_out = Instant::now() > st.until;
    println!(
        "walk {}",
        if timed_out {
            "stopped at its deadline"
        } else {
            "complete"
        }
    );
    let set: usize = st.plain.values().map(|[_, s]| s).sum();
    println!(
        "+0xF8 set on {set} type-4 children; element size {ELEMENT} bytes in both arrays; count \
         equal to the vertex count in {}, above it in {}; second array ending at the next named \
         byte in {}, starting the next 64 KiB block in {}",
        st.count_is_vertices[0],
        st.count_is_vertices[1],
        st.second_ends_at_next,
        st.second_starts_a_block
    );
    println!("copied and read back: {}", st.copied);
    for (why, n) in &st.refused {
        println!("  copy refused {n} x {why}");
    }
    for why in st.off_layout.iter().take(20) {
        eprintln!("  {why}");
    }
    assert!(
        st.off_layout.is_empty(),
        "{} records are off the measured layout",
        st.off_layout.len()
    );
    assert!(set > 0, "no type-4 child sets +0xF8");
    println!(
        "records whose second array's 6-byte elements end within its padding: {}",
        st.element_shown
    );
    assert!(
        st.second_ends_at_next > 0,
        "no second array ends at the next byte a word names, so its element size is not shown"
    );
    assert!(
        !st.refused.keys().any(|why| why.contains("+0xF8")),
        "a copy is refused over its +0xF8 record"
    );
    for kind in [DRAWABLE, BOUND] {
        let [_, set] = st.plain.get(&kind).copied().unwrap_or_default();
        assert_eq!(set, 0, "a kind {kind:#04X} body's type-4 child sets +0xF8");
    }
    // The count is held where this reads the fragments the measurement read.
    if st
        .bodies
        .get(&FRAGMENT)
        .is_some_and(|[read, _]| *read == FRAGMENTS_READ)
    {
        assert_eq!(set, MEASURED, "the build's type-4 children with +0xF8 set");
        assert_eq!(
            st.element_shown, ELEMENT_SHOWN,
            "the records that show their 6-byte elements end within the padding"
        );
    }
}
