//! Which of OpenRage's own editors can produce a body a GTA VI patched copy takes: one of the
//! row's real size, laid on the same chunks, each chunk deflating within the stored size the
//! signed table states. Over a bounded sample of a build's bodies per kind, (1) every chunk is
//! re-deflated unchanged at the tree's best level and counted against its ceiling, the slack
//! printed; (2) the body is read through its kind's reader and written back unchanged through its
//! writer, and the result's length, bytes and the chunks OpenRage's archive writer lays it on are
//! counted. Measured first on the Xbox build 1.0.2.1 with zlib at level 9: 42 of 3,249 deflated
//! chunks fitted (the game's compressor is stronger), every table walked, and every 24-byte
//! row's table was 64 KiB cuts of its body; that is what is asserted, the rest is printed.
//!
//! Bounded: 40 bodies per kind, 8 per kind from each archive file, bodies of at most 4 MiB stored,
//! and 300 s of walking, after which it reports what it reached.
//!
//! Install-gated, game-derived and slow (it walks nested archives); it skips without:
//!   `OPENRAGE_GTA6_KEYS`   the `gtavi_magic.dat` key snapshot (never committed)
//!   `OPENRAGE_GTA6_BUILD`  the build root holding `*.rpf` and `pack/*/*.rpf`

use std::collections::BTreeMap;
use std::fmt::Display;
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use flate2::write::{DeflateEncoder, ZlibEncoder};
use flate2::{Compression, Decompress, FlushDecompress, Status};
use or_io::container::rpf::gta6_chunk_table::{entry_chunks, Chunk};
use or_io::container::rpf::rpf9_write::write_rpf9_resource_chunks;
use or_io::container::rpf::{
    read_rpf9_body, Gta6Class, Gta6Entry, Gta6Mount, Gta6NestedRow, GtaViKeys, Rpf9BodyKind,
    Rpf9Form, Rpf9Role, Rpf9Sections, GTA6_SNAPSHOT_ENV,
};
use or_io::gta6::headerless::{
    cut_edges, read_bound, read_dictionary_model, read_drawable, read_drawable_with_bound,
    read_navmesh, write_bound_vertices, write_dictionary, write_shaders,
};
use or_io::gta6::mission_tables::Body as MissionBody;
use or_io::gta6::script::compiled::Compiled;
use or_io::meta::binary::hash_index::title_names;
use or_io::meta::binary::pso::table_strings::write_adding_strings;
use or_io::meta::binary::rbm::gta6 as headerless_meta;
use or_io::meta::binary::{parse_meta_binary_full, ContainerLayout};
use or_io::meta::{write_meta_binary, Diagnostics, SchemaLibrary};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// A body stored larger than this is not sampled.
const CAP: u64 = 4 << 20;
const MAX_DEPTH: usize = 8;
/// The 16-byte zero slot a headerless body begins with.
const SLOT: u64 = 16;
const PER_KIND: usize = 40;
const PER_ARCHIVE: usize = 8;
const DEADLINE: Duration = Duration::from_secs(300);
/// The two stream caps the archive writer cuts a block at.
const CAPS: [u64; 2] = [8 << 20, 32 << 20];
/// The piece a 24-byte row's body is cut into before each piece is deflated or stored.
const PIECE: usize = 0x1_0000;
/// The fewest bytes of slack an empty stored block pads a stream by, so the stream can end exactly
/// at its stored size as OpenRage's reader requires.
const PADDABLE: i64 = 5;

const TEXTURE_DICTIONARY: &str = "texture dictionary";
const DRAWABLE: &str = "drawable";
const CLIPS: &str = "clip family";
const TEXT_TABLE: &str = "text table";
const BOUND: &str = "bound";
const NAV_MESH: &str = "nav mesh";
const HEADERLESS_META: &str = "headerless metadata";
const SCRIPT: &str = "script program";
const PSO_TABLE: &str = "PSO table form";
const PSO_P: &str = "PSO P-form";
const MISSION: &str = "mission tables";

/// The headerless kinds and their type codes, as the measured table of rows lists them.
const HEADERLESS: [(&str, &[u32]); 8] = [
    (TEXTURE_DICTIONARY, &[0x04, 0x05]),
    (DRAWABLE, &[0x02]),
    (CLIPS, &[0x43, 0x46, 0x4E, 0x66, 0x6D]),
    (TEXT_TABLE, &[0x6F]),
    (BOUND, &[0x06]),
    (NAV_MESH, &[0x4F]),
    (HEADERLESS_META, &[0x0B, 0x57, 0x6C, 0x71, 0x73]),
    (SCRIPT, &[0x59]),
];
const PSO: [&str; 3] = [PSO_TABLE, PSO_P, MISSION];

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
fn masked(message: impl Display) -> String {
    (message.to_string().chars())
        .map(|c| if c.is_ascii_digit() { '#' } else { c })
        .collect()
}

/// `data` inflated as one stream (zlib, or raw DEFLATE) that ends exactly at its end and gives
/// exactly `want` bytes; `None` otherwise.
fn inflate_exact(data: &[u8], zlib: bool, want: usize) -> Option<Vec<u8>> {
    let mut d = Decompress::new(zlib);
    let mut out = Vec::with_capacity(want + 1);
    match d.decompress_vec(data, &mut out, FlushDecompress::Finish) {
        Ok(Status::StreamEnd) if d.total_in() as usize == data.len() && out.len() == want => {
            Some(out)
        }
        _ => None,
    }
}

/// The length of `data` deflated at the best level, raw or under a zlib header.
fn deflated_len(data: &[u8], raw: bool) -> usize {
    let out = if raw {
        let mut z = DeflateEncoder::new(Vec::new(), Compression::best());
        z.write_all(data).expect("deflated");
        z.finish().expect("finished")
    } else {
        let mut z = ZlibEncoder::new(Vec::new(), Compression::best());
        z.write_all(data).expect("deflated");
        z.finish().expect("finished")
    };
    out.len()
}

/// One chunk inflated, and whether it was raw DEFLATE.
struct Piece {
    data: Vec<u8>,
    raw: bool,
}

/// The deciphered stored body `stored` walked by its table, as the reader walks it.
fn walk_stored(stored: &[u8], table: &[Chunk]) -> Result<Vec<Piece>, String> {
    let mut pos = 0usize;
    let mut out = Vec::with_capacity(table.len());
    for (i, c) in table.iter().enumerate() {
        let bytes = (stored.get(pos..pos + c.stored as usize))
            .ok_or_else(|| format!("chunk {i} runs past the stored body"))?;
        let want = c.inflated as usize;
        out.push(match c.zlib {
            false => Piece {
                data: bytes.to_vec(),
                raw: false,
            },
            true => match inflate_exact(bytes, true, want) {
                Some(data) => Piece { data, raw: false },
                None => Piece {
                    data: inflate_exact(bytes, false, want).ok_or_else(|| {
                        format!(
                            "chunk {i} does not inflate from {} bytes to {want}",
                            c.stored
                        )
                    })?,
                    raw: true,
                },
            },
        });
        pos = (pos + c.stored as usize).next_multiple_of(16);
    }
    Ok(out)
}

/// The stored bytes of entry `e`'s chunks: deciphered where it is enciphered, past the zero slot
/// for a 20-byte row.
fn stored_of(
    mount: &Gta6Mount,
    src: &mut File,
    keys: &GtaViKeys,
    e: &Gta6Entry,
    row: Option<Gta6NestedRow>,
) -> Result<Vec<u8>, String> {
    if e.cipher.is_some() {
        return (mount.read_entry_deciphered_stored(&mut *src, e, keys)).map_err(|e| e.to_string());
    }
    let start = mount.absolute_start(e).ok_or("the entry has no start")?;
    let (at, len) = match row {
        Some(row) => (start + SLOT, row.stored_size().saturating_sub(SLOT)),
        None => (start, e.stored.ok_or("the entry has no stored size")?),
    };
    let mut body = vec![0u8; len as usize];
    src.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
    src.read_exact(&mut body).map_err(|e| e.to_string())?;
    Ok(body)
}

/// What a kind's writer makes of a body read and written back unchanged.
enum Rewrite {
    /// The sections (virtual, physical), or a 24-byte row's body and nothing.
    Wrote(Vec<u8>, Vec<u8>),
    NoWriter,
    Unread(String),
    Refused(String),
}

/// `s` with `write` made in a copy of its virtual section.
fn in_place<E: Display>(
    s: &Rpf9Sections,
    write: impl FnOnce(&mut [u8]) -> Result<(), E>,
) -> Rewrite {
    let mut v = s.virtual_section.clone();
    match write(&mut v) {
        Ok(()) => Rewrite::Wrote(v, s.physical.clone()),
        Err(e) => Rewrite::Refused(masked(e)),
    }
}

/// The headerless body of `kind`, its sections `s`, read and written back by its kind's editor.
fn rewrite_headerless(kind: &str, s: &Rpf9Sections, role: Option<Rpf9Role>) -> Rewrite {
    let (v, p) = (&s.virtual_section, &s.physical);
    match kind {
        TEXTURE_DICTIONARY => match read_dictionary_model(v, p) {
            Err(e) => Rewrite::Unread(masked(e)),
            Ok(model) => match write_dictionary(&model) {
                Ok(out) => Rewrite::Wrote(out.virtual_section, out.physical),
                Err(e) => Rewrite::Refused(masked(e)),
            },
        },
        DRAWABLE => {
            let shaders = match role {
                Some(Rpf9Role::Drawable) => read_drawable(v, Some(p.as_slice())).map(|d| d.shaders),
                Some(Rpf9Role::DrawableWithBound) => {
                    read_drawable_with_bound(v, Some(p.as_slice())).map(|d| d.drawable.shaders)
                }
                _ => {
                    let role = role.map_or("none", Rpf9Role::label);
                    return Rewrite::Unread(format!("its role is {role}"));
                }
            };
            let role = role.expect("matched above");
            match shaders {
                Err(e) => Rewrite::Unread(masked(e)),
                Ok(shaders) => in_place(s, |v| write_shaders(role, v, None, &shaders)),
            }
        }
        CLIPS => {
            let stored = matches!(s.form, Rpf9Form::StoredRsc8 { .. });
            let read = match stored {
                true => or_io::gta6::anim::read_stored(v),
                false => or_io::gta6::anim::read(v),
            };
            if let Err(e) = read {
                return Rewrite::Unread(masked(e));
            }
            match or_io::gta6::anim::reencode(v, stored) {
                Ok(out) => Rewrite::Wrote(out, p.clone()),
                Err(e) => Rewrite::Refused(masked(e)),
            }
        }
        TEXT_TABLE => match or_io::gta6::text::read(v) {
            Err(e) => Rewrite::Unread(masked(e)),
            Ok((table, _)) => match table.to_section() {
                Ok(out) => Rewrite::Wrote(out, p.clone()),
                Err(e) => Rewrite::Refused(masked(e)),
            },
        },
        BOUND => match read_bound(v) {
            Err(e) => Rewrite::Unread(masked(e)),
            Ok(bound) => in_place(s, |v| write_bound_vertices(v, &bound)),
        },
        NAV_MESH => match read_navmesh(v) {
            Err(e) => Rewrite::Unread(masked(e)),
            Ok(_) => in_place(s, |v| cut_edges(v, &[])),
        },
        HEADERLESS_META => match headerless_meta::open(v, p) {
            Err(e) => Rewrite::Unread(masked(e)),
            Ok((instance, layout)) => match layout.container {
                ContainerLayout::Rbm(rbm) => match headerless_meta::write_sections(&rbm, &instance)
                {
                    Ok((v, p)) => Rewrite::Wrote(v, p),
                    Err(e) => Rewrite::Refused(masked(e)),
                },
                _ => Rewrite::Unread("it opens as no RBM layout".into()),
            },
        },
        _ => Rewrite::NoWriter,
    }
}

/// A PSO body read and written back as the app saves an edited metadata document.
fn rewrite_pso(library: &SchemaLibrary, body: &[u8]) -> Rewrite {
    let mut diags = Diagnostics::lenient();
    let (instance, layout) = match parse_meta_binary_full(library, body, &mut diags) {
        Ok(read) => read,
        Err(e) => return Rewrite::Unread(masked(e)),
    };
    let table = matches!(&layout.container, ContainerLayout::Pso(p) if p.is_table_form());
    let out = match table {
        true => write_adding_strings(&layout, &instance, None),
        false => write_meta_binary(layout.format, Some(&layout), &instance),
    };
    match out {
        Ok(out) => Rewrite::Wrote(out, Vec::new()),
        Err(e) => Rewrite::Refused(masked(e)),
    }
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
    bodies: usize,
    ciphered: usize,
    chunks: usize,
    stored_chunks: usize,
    deflated: usize,
    fit: usize,
    /// Stored size less the size re-deflated, per deflated chunk.
    slack: Vec<i64>,
    /// Bodies every deflated chunk of which fits.
    all_fit: usize,
    /// 24-byte rows whose table is OpenRage's 64 KiB cut of the body.
    cut_ours: usize,
    writes: BTreeMap<String, usize>,
    archive: BTreeMap<String, usize>,
}

#[derive(Default)]
struct State {
    kinds: BTreeMap<&'static str, Tally>,
    /// Bodies taken per kind from the current archive file.
    here: BTreeMap<&'static str, usize>,
    walk_failed: Vec<String>,
    started: Option<Instant>,
    timed_out: bool,
}

impl State {
    fn wants(&self, kind: &str) -> bool {
        self.kinds.get(kind).map_or(0, |t| t.bodies) < PER_KIND
            && self.here.get(kind).copied().unwrap_or(0) < PER_ARCHIVE
    }

    fn done(&mut self) -> bool {
        if self.started.is_some_and(|t| t.elapsed() > DEADLINE) {
            self.timed_out = true;
            return true;
        }
        HEADERLESS
            .iter()
            .map(|(k, _)| *k)
            .chain(PSO)
            .all(|k| self.kinds.get(k).map_or(0, |t| t.bodies) >= PER_KIND)
    }
}

/// The headroom of one body's chunks, as part (1) counts it, into `t`.
fn headroom(t: &mut Tally, table: &[Chunk], pieces: &[Piece]) {
    let mut all = true;
    for (c, piece) in table.iter().zip(pieces) {
        t.chunks += 1;
        if !c.zlib {
            t.stored_chunks += 1;
            continue;
        }
        t.deflated += 1;
        let slack = i64::from(c.stored) - deflated_len(&piece.data, piece.raw) as i64;
        t.slack.push(slack);
        if slack >= 0 {
            t.fit += 1;
        } else {
            all = false;
        }
    }
    t.all_fit += usize::from(all);
}

/// Where the sections `(v, p)` a headerless save writes land on the archive writer's chunks,
/// against the table's: one label.
fn headerless_archive_leg(v: &[u8], p: &[u8], table: &[Chunk]) -> String {
    let inflated = |cs: &[Chunk]| cs.iter().map(|c| c.inflated).collect::<Vec<u32>>();
    let mut written = Vec::new();
    let mut refused = String::new();
    for cap in CAPS {
        match write_rpf9_resource_chunks(v, p, cap) {
            Ok(w) => written.push(w),
            Err(e) => refused = masked(e),
        }
    }
    if written.is_empty() {
        return format!("the archive writer refuses it: {refused}");
    }
    let Some(w) = written
        .iter()
        .find(|w| inflated(&w.chunks) == inflated(table))
    else {
        return "laid on other chunks".into();
    };
    if w.chunks.iter().zip(table).any(|(n, o)| n.zlib != o.zlib) {
        return "same chunks, a stored chunk deflated".into();
    }
    if w.chunks.iter().zip(table).any(|(n, o)| n.stored > o.stored) {
        return "same chunks, a chunk past its ceiling".into();
    }
    "same chunks, every one within its ceiling".into()
}

/// Where a 24-byte row's new body lands on the archive writer's 64 KiB pieces, against the table.
fn row24_archive_leg(body: &[u8], table: &[Chunk]) -> String {
    let cut: Vec<usize> = body.chunks(PIECE).map(<[u8]>::len).collect();
    let stated: Vec<usize> = table.iter().map(|c| c.inflated as usize).collect();
    if cut != stated {
        return "laid on other chunks".into();
    }
    let mut over = false;
    for (piece, c) in body.chunks(PIECE).zip(table) {
        let z = deflated_len(piece, false);
        if (z < piece.len()) != c.zlib {
            return "same chunks, another stored or deflated choice".into();
        }
        over |= c.zlib && z > c.stored as usize;
    }
    match over {
        true => "same chunks, a chunk past its ceiling".into(),
        false => "same chunks, every one within its ceiling".into(),
    }
}

/// Measure one body of `kind` (a 24-byte row's PSO kind is named here, from its body).
#[allow(clippy::too_many_arguments)]
fn measure(
    st: &mut State,
    library: &SchemaLibrary,
    kind: &'static str,
    what: String,
    ciphered: bool,
    stored: &[u8],
    table: &[Chunk],
    row: Option<Gta6NestedRow>,
) {
    let pieces = match walk_stored(stored, table) {
        Ok(pieces) => pieces,
        Err(why) => {
            st.walk_failed.push(format!("{kind}, {what}: {why}"));
            return;
        }
    };
    let body: Vec<u8> = pieces.iter().flat_map(|p| p.data.iter().copied()).collect();
    let kind = if kind == PSO_TABLE {
        pso_kind(&body)
    } else {
        kind
    };
    if !st.wants(kind) {
        return;
    }
    *st.here.entry(kind).or_default() += 1;
    let t = st.kinds.entry(kind).or_default();
    t.bodies += 1;
    t.ciphered += usize::from(ciphered);
    headroom(t, table, &pieces);

    let (write, before) = match row {
        None => {
            let cut: Vec<usize> = body.chunks(PIECE).map(<[u8]>::len).collect();
            let stated: Vec<usize> = table.iter().map(|c| c.inflated as usize).collect();
            t.cut_ours += usize::from(cut == stated);
            (rewrite_pso(library, &body), (body.clone(), Vec::new()))
        }
        Some(_) if kind == SCRIPT => match Compiled::open(&body) {
            Ok(_) => (Rewrite::NoWriter, (body.clone(), Vec::new())),
            Err(e) => (Rewrite::Unread(masked(e)), (body.clone(), Vec::new())),
        },
        Some(_) => {
            let limit = table.iter().map(|c| u64::from(c.inflated)).sum();
            let read =
                read_rpf9_body(Cursor::new(stored), 0, limit)
                    .ok()
                    .and_then(|(find, bytes)| {
                        let sections = Rpf9Sections::of(&find, &bytes)?;
                        let role = match find.kind {
                            Rpf9BodyKind::Zlib(run) => run.signature.and_then(|s| s.role()),
                            _ => None,
                        };
                        Some((sections, role))
                    });
            match read {
                None => (
                    Rewrite::Unread("the resource reader finds no sections".into()),
                    (Vec::new(), Vec::new()),
                ),
                Some((s, role)) => {
                    let before = (s.virtual_section.clone(), s.physical.clone());
                    (rewrite_headerless(kind, &s, role), before)
                }
            }
        }
    };
    let (label, archive) = match write {
        Rewrite::NoWriter => ("no writer".to_string(), None),
        Rewrite::Unread(why) => (format!("unread: {why}"), None),
        Rewrite::Refused(why) => (format!("refused: {why}"), None),
        Rewrite::Wrote(v, p) => {
            let label = if (&v, &p) == (&before.0, &before.1) {
                "identical"
            } else if (v.len(), p.len()) == (before.0.len(), before.1.len()) {
                "same length, other bytes"
            } else {
                "another length"
            };
            let leg = match row {
                None => row24_archive_leg(&v, table),
                Some(_) => headerless_archive_leg(&v, &p, table),
            };
            (label.to_string(), Some(leg))
        }
    };
    *t.writes.entry(label).or_default() += 1;
    if let Some(leg) = archive {
        *t.archive.entry(leg).or_default() += 1;
    }
}

/// Sample the bodies of `mount`, the archive at `depth`, and of every archive nested in it.
fn walk(
    file: &mut File,
    keys: &GtaViKeys,
    library: &SchemaLibrary,
    (mount, label, depth): (&Gta6Mount, &str, usize),
    st: &mut State,
) {
    let extras = mount.archive.entry_extras();
    for (entry, extra) in mount.archive.entries.iter().zip(&extras) {
        if st.done() {
            return;
        }
        if entry.kind() == Some(Gta6Class::NestedArchive) {
            if depth < MAX_DEPTH {
                if let Ok(child) = mount.open_nested(&mut *file, entry, keys) {
                    walk(file, keys, library, (&child, label, depth + 1), st);
                }
            }
            continue;
        }
        let kind = match extra.nested {
            Some(row) => HEADERLESS
                .iter()
                .find(|(_, codes)| codes.contains(&row.resource_type))
                .map(|(kind, _)| *kind),
            None if PSO.iter().any(|k| st.wants(k)) => Some(PSO_TABLE),
            None => None,
        };
        let Some(kind) = kind.filter(|k| *k == PSO_TABLE || st.wants(k)) else {
            continue;
        };
        let stored_len = extra.nested.map_or(entry.stored, |r| Some(r.stored_size()));
        if stored_len.is_none_or(|n| n > CAP) {
            continue;
        }
        let Ok(Some(table)) = entry_chunks(&mount.archive, entry.index as usize) else {
            continue;
        };
        let what = format!("{label}, entry {} at depth {depth}", entry.index);
        // A body whose content key the snapshot lacks is not sampled.
        let Ok(stored) = stored_of(mount, file, keys, entry, extra.nested) else {
            continue;
        };
        if kind == PSO_TABLE {
            let head = walk_stored(&stored, &table[..1.min(table.len())]);
            if !head.is_ok_and(|p| p.first().is_some_and(|p| p.data.starts_with(b"PSIN"))) {
                continue;
            }
        }
        let ciphered = entry.cipher.is_some();
        measure(
            st,
            library,
            kind,
            what,
            ciphered,
            &stored,
            &table,
            extra.nested,
        );
    }
}

fn print(kind: &str, t: &Tally) {
    let mut s = t.slack.clone();
    s.sort_unstable();
    let at = |f: f64| s[((s.len() as f64 * f) as usize).min(s.len().saturating_sub(1))];
    let band = |lo: i64, hi: i64| s.iter().filter(|&&v| (lo..=hi).contains(&v)).count();
    println!(
        "{kind}: {} bodies ({} enciphered), {} chunks ({} stored, {} deflated); {} deflated chunks fit, \
         {} bodies fit whole; 24-byte tables cut as OpenRage cuts: {}",
        t.bodies, t.ciphered, t.chunks, t.stored_chunks, t.deflated, t.fit, t.all_fit, t.cut_ours
    );
    if !s.is_empty() {
        println!(
            "  slack least {}, tenth {}, median {}, ninetieth {}, most {}; below 0: {}, \
             0 to {}: {}, {PADDABLE} or more: {}",
            at(0.0),
            at(0.1),
            at(0.5),
            at(0.9),
            at(1.0),
            band(i64::MIN, -1),
            PADDABLE - 1,
            band(0, PADDABLE - 1),
            band(PADDABLE, i64::MAX),
        );
    }
    for (label, n) in &t.writes {
        println!("  written back unchanged: {n} x {label}");
    }
    for (label, n) in &t.archive {
        println!("  on the archive writer's chunks: {n} x {label}");
    }
}

#[test]
fn which_edits_fit_in_place_is_measured_per_kind() {
    let what = "GTA VI edits that fit in place over a build";
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
    let library = SchemaLibrary::default();
    let mut st = State {
        started: Some(Instant::now()),
        ..State::default()
    };
    for path in &archives {
        if st.done() {
            break;
        }
        st.here.clear();
        let label = (path.file_name()).map_or_else(String::new, |n| n.to_string_lossy().into());
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys).expect("the archive opens");
        walk(&mut file, &keys, &library, (&mount, &label, 0), &mut st);
    }

    if st.timed_out {
        println!("the walk stopped at its {} s bound", DEADLINE.as_secs());
    }
    for kind in HEADERLESS.iter().map(|(k, _)| *k).chain(PSO) {
        print(kind, st.kinds.get(kind).unwrap_or(&Tally::default()));
    }
    for failure in st.walk_failed.iter().take(20) {
        eprintln!("  {failure}");
    }
    assert!(
        st.walk_failed.is_empty(),
        "{} bodies do not walk by their chunk tables",
        st.walk_failed.len()
    );
    let (fit, deflated) = (st.kinds.values()).fold((0, 0), |(f, d), t| (f + t.fit, d + t.deflated));
    assert!(deflated > 0, "the sample holds no deflated chunk");
    assert!(
        fit * 20 < deflated,
        "{fit} of {deflated} deflated chunks fit their ceiling re-deflated: the tree's deflater \
         now has headroom the measurement did not find, so read the figures again"
    );
    for kind in PSO {
        let t = st.kinds.get(kind);
        assert!(
            t.is_none_or(|t| t.cut_ours == t.bodies),
            "a {kind} table is not 64 KiB cuts of its body"
        );
    }
    if !st.timed_out {
        for (kind, _) in HEADERLESS {
            assert!(
                st.kinds.get(kind).is_some_and(|t| t.bodies > 0),
                "the build holds no sampled {kind} body"
            );
        }
        assert!(
            st.kinds.get(PSO_TABLE).is_some_and(|t| t.bodies > 0),
            "the build holds no sampled table-form PSO"
        );
    }
}
