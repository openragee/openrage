//! One check per kind a save into a GTA VI archive can emit, over a sample of a real build's
//! entries. A headerless resource (a 20-byte row) is read as its sections, written as a loose file
//! and put back with `replace_resource`; a 24-byte-row body is put back with `replace`, unchanged
//! and with one byte edited. The written archive is reopened and must read back the same resource
//! bytes, and the same row wherever the save changed no table. A kind the writer refuses is
//! reported by the writer's reason; the test fails on a wrong result, and on a refusal the writer
//! no longer makes.
//!
//! The written copy is read through its plan, the bytes `Rpf9Plan::write_to` writes, since a
//! top-level archive is too large to hold in memory. Structure only: an archive whose table a save
//! changes has a stale signature, and the game refuses it.
//!
//! Install-gated and game-derived, so it skips cleanly without its env vars:
//!   `OPENRAGE_GTA6_KEYS`   the `gtavi_magic.dat` key snapshot (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

#![cfg(feature = "gta6")]

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use flate2::read::ZlibDecoder;
use or_io::container::rpf::gta6_chunk_table::entry_chunks;
use or_io::container::rpf::{
    read_rpf9_body, write_rpf9_resource, Gta6Class, Gta6Entry, Gta6Mount, Gta6NestedRow, GtaViKeys,
    Rpf9ArchiveWriter, Rpf9Piece, Rpf9Plan, Rpf9Sections, GTA6_SNAPSHOT_ENV,
};
use or_io::gta6::audio_data::stamped;
use or_io::gta6::mission_tables::Body as MissionBody;
use or_io::gta6::other_bodies::OtherBody;
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// A body larger than this is not sampled.
const CAP: u64 = 4 << 20;
const MAX_DEPTH: usize = 8;
/// The 16-byte zero slot a headerless body begins with.
const SLOT: u64 = 16;
/// Headerless entries sampled per type code, and 24-byte-row entries per kind.
const PER_CODE: usize = 2;
const PER_KIND: usize = 3;
/// Stored bytes read, and inflated bytes kept, to name a 24-byte-row body by its head.
const HEAD_STORED: usize = 4096;
const HEAD: u64 = 512;
/// What the writer no longer refuses: a mode-4 chunk table, a dependency key the table lacks.
const CLOSED: [&str; 2] = ["unknown mode", "not in the archive's dependency table"];

/// The headerless kinds and their type codes, as the measured table of rows lists them.
const HEADERLESS: [(&str, &[u32]); 14] = [
    ("drawable", &[0x02]),
    ("fragment", &[0x03]),
    ("drawable dictionary", &[0x04]),
    ("texture dictionary", &[0x05]),
    ("bound", &[0x06]),
    ("nav mesh", &[0x4F]),
    ("nav octree", &[0x64]),
    ("cloth", &[0x58]),
    ("map data", &[0x49]),
    ("map types", &[0x4A]),
    ("particles", &[0x48]),
    ("clip family", &[0x43, 0x46, 0x4E, 0x66, 0x6D]),
    ("headerless metadata", &[0x0B, 0x57, 0x6C, 0x71, 0x73]),
    ("text table", &[0x6F]),
];

const PSO_TABLE: &str = "PSO table form";
const PSO_P: &str = "PSO P-form";
const MISSION: &str = "mission tables";
/// The 24-byte-row kinds, each saved with `replace`.
const ROW24: [&str; 8] = [
    PSO_TABLE,
    PSO_P,
    "RBF",
    "XML",
    "text",
    MISSION,
    "audio game data",
    "height map",
];

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

/// The archive a plan writes, read without writing it: each read is served from the plan's pieces.
struct Written<'a, R> {
    plan: &'a Rpf9Plan,
    src: &'a mut R,
    pos: u64,
}

impl<R: Read + Seek> Read for Written<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut at = 0u64;
        for piece in &self.plan.pieces {
            let size = piece.size();
            if self.pos < at + size {
                let off = self.pos - at;
                let n = (size - off).min(buf.len() as u64) as usize;
                match piece {
                    Rpf9Piece::Bytes(bytes) => {
                        buf[..n].copy_from_slice(&bytes[off as usize..off as usize + n]);
                    }
                    Rpf9Piece::Copy { from, .. } => {
                        self.src.seek(SeekFrom::Start(from + off))?;
                        self.src.read_exact(&mut buf[..n])?;
                    }
                    Rpf9Piece::Zero(_) => buf[..n].fill(0),
                }
                self.pos += n as u64;
                return Ok(n);
            }
            at += size;
        }
        Ok(0)
    }
}

impl<R> Seek for Written<'_, R> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let pos = match to {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.plan.size().checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        self.pos = pos.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "before 0"))?;
        Ok(self.pos)
    }
}

/// The archive at `path` (entry indices, top level first) of `src`.
fn mount_at<R: Read + Seek>(src: &mut R, keys: &GtaViKeys, path: &[u32]) -> Option<Gta6Mount> {
    let mut mount = Gta6Mount::open_root(src, keys).ok()?;
    for &i in path {
        let entry = mount.archive.entries.get(i as usize)?.clone();
        mount = mount.open_nested(src, &entry, keys).ok()?;
    }
    Some(mount)
}

/// A chunked entry as the reader inflates it, any other as its raw `[start, start + real)`.
fn body_of<R: Read + Seek>(
    mount: &Gta6Mount,
    src: &mut R,
    keys: &GtaViKeys,
    e: &Gta6Entry,
) -> Option<Vec<u8>> {
    let chunked = mount
        .archive
        .entry_extra(e.index as usize)?
        .chunk_count
        .is_some();
    if chunked {
        return mount.read_entry_decrypted(&mut *src, e, keys).ok();
    }
    let mut body = vec![0u8; e.real? as usize];
    src.seek(SeekFrom::Start(mount.absolute_start(e)?)).ok()?;
    src.read_exact(&mut body).ok()?;
    Some(body)
}

/// The sections a read past the zero slot of a headerless body at `start` returns.
fn sections_at<R: Read + Seek>(src: &mut R, start: u64) -> Option<Rpf9Sections> {
    let (find, bytes) = read_rpf9_body(src, start + SLOT, CAP).ok()?;
    Rpf9Sections::of(&find, &bytes)
}

/// The first inflated bytes of the body at `at`: its first chunk inflated, or as stored.
fn head_at<R: Read + Seek>(src: &mut R, at: u64) -> Option<Vec<u8>> {
    let mut stored = Vec::new();
    src.seek(SeekFrom::Start(at)).ok()?;
    (&mut *src)
        .take(HEAD_STORED as u64)
        .read_to_end(&mut stored)
        .ok()?;
    let mut head = Vec::new();
    // A chunk cut short by the read still yields its first bytes before the error.
    let _ = ZlibDecoder::new(&stored[..])
        .take(HEAD)
        .read_to_end(&mut head);
    if head.is_empty() {
        stored.truncate(HEAD as usize);
        return Some(stored);
    }
    Some(head)
}

/// The 24-byte-row kind a body's head names; a PSO is named later, from its whole body.
fn kind_of_head(head: &[u8], real: u64) -> Option<&'static str> {
    if head.starts_with(b"PSIN") {
        return Some(PSO_TABLE);
    }
    if head.starts_with(b"RBF0") {
        return Some("RBF");
    }
    if stamped(head) {
        return Some("audio game data");
    }
    if matches!(
        OtherBody::of(head, real),
        Some(OtherBody::HeightMap | OtherBody::HeightGrid)
    ) {
        return Some("height map");
    }
    let text = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    let utf8 = match std::str::from_utf8(text) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none(),
    };
    let plain = text
        .iter()
        .all(|&c| c >= 0x20 || matches!(c, b'\t' | b'\n' | b'\r'));
    if text.is_empty() || !utf8 || !plain {
        return None;
    }
    Some(if text.trim_ascii_start().starts_with(b"<") {
        "XML"
    } else {
        "text"
    })
}

/// A PSO's form: `PMAP` after `PSIN` is the P-form; a table form whose blocks hold a state table
/// is the mission tables.
fn pso_kind(body: &[u8]) -> &'static str {
    let psin = body
        .get(4..8)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
    if psin.and_then(|at| body.get(at..at + 4)) == Some(b"PMAP") {
        return PSO_P;
    }
    let states = MissionBody::open(body)
        .and_then(|b| b.tables(body))
        .is_ok_and(|t| !t.is_empty());
    if states {
        MISSION
    } else {
        PSO_TABLE
    }
}

#[derive(Default)]
struct Tally {
    sampled: usize,
    passed: usize,
    /// Passed with no table changed: the archive is the source, row and all.
    unchanged: usize,
    refused: BTreeMap<String, usize>,
}

#[derive(Default)]
struct State {
    per_code: BTreeMap<u32, usize>,
    kinds: BTreeMap<&'static str, Tally>,
    failures: Vec<String>,
}

enum Outcome {
    Passed { unchanged: bool },
    Refused(String),
    Failed(String),
}

impl State {
    fn done(&self) -> bool {
        HEADERLESS
            .iter()
            .flat_map(|(_, codes)| codes.iter())
            .all(|c| self.per_code.get(c).copied().unwrap_or(0) >= PER_CODE)
            && ROW24.iter().all(|k| !self.wants(k))
    }

    fn wants(&self, kind: &str) -> bool {
        self.kinds.get(kind).map_or(0, |t| t.sampled) < PER_KIND
    }

    fn record(&mut self, kind: &'static str, what: String, outcome: Outcome) {
        let tally = self.kinds.entry(kind).or_default();
        tally.sampled += 1;
        match outcome {
            Outcome::Passed { unchanged } => {
                tally.passed += 1;
                tally.unchanged += usize::from(unchanged);
            }
            Outcome::Refused(why) => *tally.refused.entry(why).or_default() += 1,
            Outcome::Failed(why) => self.failures.push(format!("{kind}, {what}: {why}")),
        }
    }
}

/// Save headerless entry `i` of the archive at `path` through the loose writer and
/// `replace_resource`, then reopen the written copy.
fn save_headerless(
    file: &mut File,
    keys: &GtaViKeys,
    (path, i): (&[u32], u32),
    row: Gta6NestedRow,
    sections: Rpf9Sections,
) -> Outcome {
    let loose = match write_rpf9_resource(&sections) {
        Ok(loose) => loose,
        Err(e) => return Outcome::Refused(format!("the loose writer: {e}")),
    };
    // The writer stores the canonical form, which the loose file read at its start states.
    let canonical = read_rpf9_body(&mut std::io::Cursor::new(&loose), 0, CAP)
        .ok()
        .and_then(|(find, bytes)| Rpf9Sections::of(&find, &bytes));
    let Some(canonical) = canonical else {
        return Outcome::Failed("the loose file does not read back".into());
    };
    let full: Vec<u32> = path.iter().copied().chain([i]).collect();
    let plan = match Rpf9ArchiveWriter::new()
        .replace_resource(&full, loose, None)
        .plan(file, keys)
    {
        Ok(plan) => plan,
        Err(e) => return Outcome::Refused(e.to_string()),
    };
    let mut view = Written {
        plan: &plan,
        src: file,
        pos: 0,
    };
    let Some(after) = mount_at(&mut view, keys, path) else {
        return Outcome::Failed("the written archive does not reopen".into());
    };
    let (Some(entry), Some(new_row)) = (
        after.archive.entries.get(i as usize).cloned(),
        after.archive.entry_extra(i as usize).and_then(|x| x.nested),
    ) else {
        return Outcome::Failed("the entry has no 20-byte row after the save".into());
    };
    let back = after
        .absolute_start(&entry)
        .and_then(|start| sections_at(&mut view, start));
    if back.as_ref() != Some(&canonical) {
        return Outcome::Failed("the resource does not read back".into());
    }
    if (new_row.virtual_size, new_row.flags, new_row.resource_type)
        != (row.virtual_size, row.flags, row.resource_type)
    {
        return Outcome::Failed("the row states other sections or another type".into());
    }
    let unchanged = plan.tables_changed == 0;
    if unchanged && new_row != row {
        return Outcome::Failed("an unchanged table states another row".into());
    }
    Outcome::Passed { unchanged }
}

/// Save 24-byte-row entry `i` of the archive at `path` with `replace`, unchanged and then with its
/// last byte edited, reopening the written copy each time.
fn save_row24(
    file: &mut File,
    keys: &GtaViKeys,
    (path, i): (&[u32], u32),
    before: &Gta6Entry,
    body: &[u8],
) -> Outcome {
    let full: Vec<u32> = path.iter().copied().chain([i]).collect();
    let mut edited = body.to_vec();
    if let Some(last) = edited.last_mut() {
        *last ^= 0x20;
    }
    for (label, new) in [("unchanged", body), ("edited", &edited[..])] {
        let plan = match Rpf9ArchiveWriter::new()
            .replace(&full, new.to_vec())
            .plan(file, keys)
        {
            Ok(plan) => plan,
            Err(e) => return Outcome::Refused(format!("{label}: {e}")),
        };
        let unchanged = label == "unchanged";
        if unchanged && plan.tables_changed != 0 {
            return Outcome::Failed("an unchanged body changed a table".into());
        }
        let mut view = Written {
            plan: &plan,
            src: &mut *file,
            pos: 0,
        };
        let Some(after) = mount_at(&mut view, keys, path) else {
            return Outcome::Failed(format!("{label}: the written archive does not reopen"));
        };
        let Some(entry) = after.archive.entries.get(i as usize).cloned() else {
            return Outcome::Failed(format!("{label}: the entry is gone"));
        };
        let same_row = if unchanged {
            entry == *before
        } else {
            (entry.name64, entry.class, entry.real)
                == (before.name64, before.class, Some(new.len() as u64))
        };
        if !same_row {
            return Outcome::Failed(format!("{label}: the row states another entry"));
        }
        if body_of(&after, &mut view, keys, &entry).as_deref() != Some(new) {
            return Outcome::Failed(format!("{label}: the body does not read back"));
        }
        if body.is_empty() {
            break;
        }
    }
    Outcome::Passed { unchanged: true }
}

/// Sample and save the entries of `mount`, the archive at `path`, and of every archive nested in it.
fn walk(file: &mut File, keys: &GtaViKeys, mount: &Gta6Mount, path: &mut Vec<u32>, st: &mut State) {
    let extras = mount.archive.entry_extras();
    for (entry, extra) in mount.archive.entries.iter().zip(&extras) {
        if st.done() {
            return;
        }
        let i = entry.index;
        let what = |path: &[u32]| format!("entry {i} of the archive at {path:?}");
        if entry.kind() == Some(Gta6Class::NestedArchive) {
            if path.len() < MAX_DEPTH {
                if let Ok(child) = mount.open_nested(&mut *file, entry, keys) {
                    path.push(i);
                    walk(file, keys, &child, path, st);
                    path.pop();
                }
            }
            continue;
        }
        let (None, Some(start)) = (&entry.cipher, mount.absolute_start(entry)) else {
            continue;
        };
        if let Some(row) = extra.nested {
            let code = row.resource_type;
            let Some(&(kind, _)) = HEADERLESS.iter().find(|(_, c)| c.contains(&code)) else {
                continue;
            };
            if st.per_code.get(&code).copied().unwrap_or(0) >= PER_CODE
                || row.stored_size() > CAP
                || !matches!(entry_chunks(&mount.archive, i as usize), Ok(Some(_)))
            {
                continue;
            }
            let Some(sections) = sections_at(&mut *file, start) else {
                continue;
            };
            *st.per_code.entry(code).or_default() += 1;
            let outcome = save_headerless(file, keys, (&path[..], i), row, sections);
            st.record(
                kind,
                format!("{}, type {code:#04x}", what(path.as_slice())),
                outcome,
            );
            continue;
        }
        let Some(real) = entry.real.filter(|&r| r <= CAP) else {
            continue;
        };
        if extra.chunk_count.is_none() || !ROW24.iter().any(|k| st.wants(k)) {
            continue;
        }
        let Some(mut kind) = head_at(&mut *file, start).and_then(|h| kind_of_head(&h, real)) else {
            continue;
        };
        let pso = kind == PSO_TABLE;
        let wanted = if pso {
            [PSO_TABLE, PSO_P, MISSION].iter().any(|k| st.wants(k))
        } else {
            st.wants(kind)
        };
        if !wanted {
            continue;
        }
        let Some(body) = body_of(mount, &mut *file, keys, entry) else {
            continue;
        };
        if pso {
            kind = pso_kind(&body);
            if !st.wants(kind) {
                continue;
            }
        }
        let outcome = save_row24(file, keys, (&path[..], i), entry, &body);
        st.record(kind, what(path.as_slice()), outcome);
    }
}

#[test]
fn every_kind_a_save_emits_reads_back_from_the_written_archive() {
    let what = "GTA VI saves per kind over a build";
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
    let mut st = State::default();
    for path in &archives {
        if st.done() {
            break;
        }
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys).expect("the archive opens");
        walk(&mut file, &keys, &mount, &mut Vec::new(), &mut st);
    }

    let names = HEADERLESS.iter().map(|(k, _)| *k).chain(ROW24);
    let mut closed = Vec::new();
    for kind in names {
        let t = st.kinds.remove(kind).unwrap_or_default();
        closed.extend(
            (t.refused.keys())
                .filter(|why| CLOSED.iter().any(|c| why.contains(c)))
                .map(|why| format!("{kind}: {why}")),
        );
        let refused: Vec<String> = t
            .refused
            .iter()
            .map(|(why, n)| format!("{n} x {why}"))
            .collect();
        println!(
            "{kind}: sampled {}, passed {} ({} with no table changed), refused {}{}{}",
            t.sampled,
            t.passed,
            t.unchanged,
            t.refused.values().sum::<usize>(),
            if refused.is_empty() { "" } else { ": " },
            refused.join("; "),
        );
    }
    for failure in st.failures.iter().take(20) {
        eprintln!("  {failure}");
    }
    assert!(
        !st.per_code.is_empty(),
        "the build holds clear headerless entries a save can emit"
    );
    assert!(
        st.failures.is_empty(),
        "{} saves read back wrong",
        st.failures.len()
    );
    assert!(
        closed.is_empty(),
        "kinds refused for what the writer now writes: {closed:?}"
    );
}
