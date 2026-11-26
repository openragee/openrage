//! Which of OpenRage's own editors can produce a body a GTA VI patched copy takes: one of the
//! row's real size, laid on the same chunks, each chunk deflating within the stored size the
//! signed table states. Over a bounded sample of a build's bodies per kind, (1) every chunk is
//! re-deflated unchanged at the tree's best level and counted against its ceiling, the slack
//! printed; (2) the body is read through its kind's reader and written back unchanged through its
//! writer, and the result's length, bytes and the chunks OpenRage's archive writer lays it on are
//! counted. Measured first on the Xbox build 1.0.2.1 with zlib at level 9: 42 of 3,249 deflated
//! chunks fitted (the game's compressor is stronger), every table walked, and every 24-byte
//! row's table was 64 KiB cuts of its body; that is what is asserted, the rest is printed.
//! (3) Beside (1), up to 40 deflated chunks per kind are re-deflated by the patch writer's optimal
//! parse, counted against the same ceilings and inflated back; that every stream inflates to its
//! chunk is asserted, the fits and the time per 64 KiB are printed, and so are the deflated chunks
//! past the patch writer's cap (`OPTIMAL_CAP` of `gta6_patch`), which it is never given there; the
//! census itself parses only chunks of at most 256 KiB, to keep its own time bounded.
//! (4) Which kinds the build holds where the census samples is asserted against a node mirror's
//! listing of every chunk table of the build (`MIRRORED`).
//!
//! Bounded: 40 bodies per kind, 8 per kind from each archive file, bodies of at most 4 MiB stored,
//! and 300 s of walking, after which it reports what it reached; the census's optimal parse takes
//! chunks of at most 256 KiB inflated, and 300 s more in all, which the walk's bound does not count: at most
//! about ten minutes in all.
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
use or_io::container::rpf::gta6_patch::{optimal_deflate, OPTIMAL_CAP as WRITER_CAP};
use or_io::container::rpf::rpf9_write::write_rpf9_resource_chunks;
use or_io::container::rpf::{
    read_rpf9_body, Gta6Class, Gta6Entry, Gta6Mount, Gta6NestedRow, GtaViKeys, Rpf9BodyKind,
    Rpf9Form, Rpf9Role, Rpf9Sections, GTA6_SNAPSHOT_ENV,
};
use or_io::gta6::audio_data::{stamped, AudioData};
use or_io::gta6::headerless::{
    cut_edges, read_bound, read_dictionary, read_dictionary_model, read_drawable,
    read_drawable_with_bound, read_navmesh, read_particle_effects, write_bound_vertices,
    write_dictionary, write_fragment_level, write_particle_effects, write_shaders, Entry,
};
use or_io::gta6::mission_tables::Body as MissionBody;
use or_io::gta6::other_bodies::{Gta6HeightGrid, Gta6HeightMap, OtherBody};
use or_io::gta6::script::compiled::Compiled;
use or_io::meta::binary::hash_index::title_names;
use or_io::meta::binary::pso::table_strings::write_adding_strings;
use or_io::meta::binary::rbm::gta6 as headerless_meta;
use or_io::meta::binary::rbm::gta6_grow_tree::write_grown;
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
/// Deflated chunks the optimal parse re-deflates per kind.
const OPTIMAL_PER_KIND: usize = 40;
/// The largest chunk, inflated, this census gives its own sampled optimal parse.
const SAMPLE_PARSE_CAP: usize = 256 << 10;
/// The optimal parse's time in all, beside the walk's.
const OPTIMAL_BUDGET: Duration = Duration::from_secs(300);

const TEXTURE_DICTIONARY: &str = "texture dictionary";
const DRAWABLE_DICTIONARY: &str = "drawable dictionary";
const DRAWABLE: &str = "drawable";
const FRAGMENT: &str = "fragment";
const CLIPS: &str = "clip family";
const TEXT_TABLE: &str = "text table";
const BOUND: &str = "bound";
const NAV_MESH: &str = "nav mesh";
const HEADERLESS_META: &str = "headerless metadata";
const MAP_DATA: &str = "map data";
const MAP_TYPES: &str = "map types";
const PARTICLES: &str = "particles";
const SCRIPT: &str = "script program";
const PSO_TABLE: &str = "PSO table form";
const PSO_P: &str = "PSO P-form";
const MISSION: &str = "mission tables";
const RBF: &str = "RBF";
const XML: &str = "XML";
const TEXT: &str = "plain text";
const AUDIO_DATA: &str = "audio game data";
const HEIGHT_MAP: &str = "height map";
const AWC: &str = "AWC";

/// The headerless kinds and their type codes, as the measured table of rows lists them.
const HEADERLESS: [(&str, &[u32]); 13] = [
    (TEXTURE_DICTIONARY, &[0x05]),
    (DRAWABLE_DICTIONARY, &[0x04]),
    (DRAWABLE, &[0x02]),
    (FRAGMENT, &[0x03]),
    (CLIPS, &[0x43, 0x46, 0x4E, 0x66, 0x6D]),
    (TEXT_TABLE, &[0x6F]),
    (BOUND, &[0x06]),
    (NAV_MESH, &[0x4F]),
    (HEADERLESS_META, &[0x0B, 0x57, 0x6C, 0x71, 0x73]),
    (MAP_DATA, &[0x49]),
    (MAP_TYPES, &[0x4A]),
    (PARTICLES, &[0x48]),
    (SCRIPT, &[0x59]),
];
const PSO: [&str; 3] = [PSO_TABLE, PSO_P, MISSION];
/// The 24-byte-row kinds, named by the head of a body's first chunk.
const ROW24: [&str; 9] = [
    PSO_TABLE, PSO_P, MISSION, RBF, XML, TEXT, AUDIO_DATA, HEIGHT_MAP, AWC,
];
/// Inflated bytes of a 24-byte row's first chunk its kind is named by.
const HEAD: usize = 512;
/// Per kind this census added in round 36, the bodies a node mirror of its sampling (a chunk table,
/// at most 4 MiB stored, deciphered, walked by its table, 40 per kind and 8 per archive file) takes
/// over every table of the Xbox build 1.0.2.1. The mirror finds no chunk-tabled `ADAT` body.
const MIRRORED: [(&str, usize); 12] = [
    (TEXTURE_DICTIONARY, 40),
    (DRAWABLE_DICTIONARY, 40),
    (FRAGMENT, 40),
    (MAP_DATA, 24),
    (MAP_TYPES, 27),
    (PARTICLES, 18),
    (RBF, 35),
    (XML, 40),
    (TEXT, 35),
    (AUDIO_DATA, 27),
    (HEIGHT_MAP, 11),
    (AWC, 0),
];

/// The bodies of `kind` that fill its sample: [`PER_KIND`], or fewer where the mirror found fewer.
fn full_at(kind: &str) -> usize {
    let mirrored = MIRRORED.iter().find(|(m, _)| *m == kind);
    mirrored.map_or(PER_KIND, |&(_, n)| n.min(PER_KIND))
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
fn masked(message: impl Display) -> String {
    (message.to_string().chars())
        .map(|c| if c.is_ascii_digit() { '#' } else { c })
        .collect()
}

/// `data` inflated as one stream (zlib, or raw DEFLATE) that ends exactly at its end and gives
/// exactly `want` bytes; `None` otherwise.
fn inflate_exact(data: &[u8], zlib: bool, want: usize) -> Option<Vec<u8>> {
    inflate_within(data, zlib, want)
        .filter(|(used, _)| *used == data.len())
        .map(|(_, out)| out)
}

/// `data` inflated as one stream that ends within it and gives exactly `want` bytes, with the bytes it
/// used: the reader ignores what follows a chunk's stream.
fn inflate_within(data: &[u8], zlib: bool, want: usize) -> Option<(usize, Vec<u8>)> {
    let mut d = Decompress::new(zlib);
    let mut out = Vec::with_capacity(want + 1);
    match d.decompress_vec(data, &mut out, FlushDecompress::Finish) {
        Ok(Status::StreamEnd) if out.len() == want => Some((d.total_in() as usize, out)),
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
            true => match inflate_within(bytes, true, want) {
                Some((_, data)) => Piece { data, raw: false },
                None => Piece {
                    data: inflate_within(bytes, false, want)
                        .map(|(_, d)| d)
                        .ok_or_else(|| {
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
        DRAWABLE_DICTIONARY => match read_dictionary(v, Some(p.as_slice())) {
            Err(e) => Rewrite::Unread(masked(e)),
            Ok(dictionary) => {
                let mut out = v.clone();
                for (code, entry) in &dictionary.entries {
                    let shaders = match entry {
                        Err(e) => return Rewrite::Unread(masked(e)),
                        Ok(Entry::Drawable(d)) => &d.shaders,
                        Ok(_) => continue,
                    };
                    if let Err(e) =
                        write_shaders(Rpf9Role::Dictionary, &mut out, Some(*code), shaders)
                    {
                        return Rewrite::Refused(masked(e));
                    }
                }
                Rewrite::Wrote(out, p.clone())
            }
        },
        FRAGMENT => match read_drawable_with_bound(v, Some(p.as_slice())) {
            Err(e) => Rewrite::Unread(masked(e)),
            Ok(read) => in_place(s, |v| write_fragment_level(v, p, &read)),
        },
        PARTICLES => match read_particle_effects(v, None) {
            Err(e) => Rewrite::Unread(masked(e)),
            Ok(list) => in_place(s, |v| write_particle_effects(v, &list)),
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
            Ok((table, _)) => match table.to_section_over(v) {
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
        // The app saves a headerless metadata document, a map's among them, through `write_grown`.
        HEADERLESS_META | MAP_DATA | MAP_TYPES => match headerless_meta::open(v, p) {
            Err(e) => Rewrite::Unread(masked(e)),
            Ok((instance, layout)) => match layout.container {
                ContainerLayout::Rbm(rbm) => match write_grown(&rbm, &instance) {
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

/// A 24-byte row's body of `kind` read and written back as the app saves it.
fn rewrite_row24(library: &SchemaLibrary, kind: &str, body: &[u8]) -> Rewrite {
    fn saved<E: Display>(out: Result<Vec<u8>, E>) -> Rewrite {
        match out {
            Ok(out) => Rewrite::Wrote(out, Vec::new()),
            Err(e) => Rewrite::Refused(masked(e)),
        }
    }
    match kind {
        HEIGHT_MAP => match OtherBody::of(body, body.len() as u64) {
            Some(OtherBody::HeightGrid) => match Gta6HeightGrid::open(body) {
                Ok(grid) => saved(grid.save()),
                Err(e) => Rewrite::Unread(masked(e)),
            },
            _ => match Gta6HeightMap::open(body) {
                Ok(map) => saved(map.save()),
                Err(e) => Rewrite::Unread(masked(e)),
            },
        },
        // A text body is saved as the text it holds.
        XML | TEXT => match String::from_utf8(body.to_vec()) {
            Ok(text) => Rewrite::Wrote(text.into_bytes(), Vec::new()),
            Err(e) => Rewrite::Unread(masked(e)),
        },
        AUDIO_DATA => match AudioData::read(body) {
            Ok(_) => Rewrite::NoWriter,
            Err(e) => Rewrite::Unread(masked(e)),
        },
        AWC => Rewrite::NoWriter,
        _ => rewrite_pso(library, body),
    }
}

/// The 24-byte-row kind the head of a body's first chunk names, a PSO's form named later from the
/// whole body; `None` where the head opens none of them.
fn kind_of_head(head: &[u8], real: u64) -> Option<&'static str> {
    let head = &head[..head.len().min(HEAD)];
    if head.starts_with(b"PSIN") {
        return Some(PSO_TABLE);
    }
    if head.starts_with(b"RBF0") {
        return Some(RBF);
    }
    if head.starts_with(b"ADAT") {
        return Some(AWC);
    }
    if stamped(head) {
        return Some(AUDIO_DATA);
    }
    if matches!(
        OtherBody::of(head, real),
        Some(OtherBody::HeightMap | OtherBody::HeightGrid)
    ) {
        return Some(HEIGHT_MAP);
    }
    let text = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    let utf8 = match std::str::from_utf8(text) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none(),
    };
    let plain = (text.iter()).all(|&c| c >= 0x20 || matches!(c, b'\t' | b'\n' | b'\r'));
    if text.is_empty() || !utf8 || !plain {
        return None;
    }
    Some(match text.trim_ascii_start().starts_with(b"<") {
        true => XML,
        false => TEXT,
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
    bodies: usize,
    ciphered: usize,
    chunks: usize,
    stored_chunks: usize,
    deflated: usize,
    fit: usize,
    /// Deflated chunks past the patch writer's cap inflated, which its optimal parse is never given.
    past_cap: usize,
    /// Stored size less the size re-deflated, per deflated chunk.
    slack: Vec<i64>,
    /// Bodies every deflated chunk of which fits.
    all_fit: usize,
    /// 24-byte rows whose table is OpenRage's 64 KiB cut of the body.
    cut_ours: usize,
    writes: BTreeMap<String, usize>,
    archive: BTreeMap<String, usize>,
    /// Per chunk the optimal parse re-deflated: its ceiling, the tree's best, the optimal parse's.
    optimal: Vec<[i64; 3]>,
    optimal_inflated: u64,
    optimal_time: Duration,
    /// Optimal streams that do not inflate back to their chunk.
    optimal_wrong: usize,
}

#[derive(Default)]
struct State {
    kinds: BTreeMap<&'static str, Tally>,
    /// Bodies taken per kind from the current archive file.
    here: BTreeMap<&'static str, usize>,
    walk_failed: Vec<String>,
    started: Option<Instant>,
    timed_out: bool,
    /// The optimal parse's time so far, which the walk's bound leaves out.
    optimal_spent: Duration,
}

impl State {
    fn wants(&self, kind: &str) -> bool {
        self.kinds.get(kind).map_or(0, |t| t.bodies) < PER_KIND
            && self.here.get(kind).copied().unwrap_or(0) < PER_ARCHIVE
    }

    fn done(&mut self) -> bool {
        if (self.started).is_some_and(|t| t.elapsed().saturating_sub(self.optimal_spent) > DEADLINE)
        {
            self.timed_out = true;
            return true;
        }
        HEADERLESS
            .iter()
            .map(|(k, _)| *k)
            .chain(ROW24)
            .all(|k| self.kinds.get(k).map_or(0, |t| t.bodies) >= full_at(k))
    }
}

/// The headroom of one body's chunks, as parts (1) and (3) count it, into `t`; the optimal parse's
/// time is added to `spent`.
fn headroom(t: &mut Tally, spent: &mut Duration, table: &[Chunk], pieces: &[Piece]) {
    let mut all = true;
    for (c, piece) in table.iter().zip(pieces) {
        t.chunks += 1;
        if !c.zlib {
            t.stored_chunks += 1;
            continue;
        }
        t.deflated += 1;
        t.past_cap += usize::from(piece.data.len() > WRITER_CAP);
        let tree = deflated_len(&piece.data, piece.raw) as i64;
        let slack = i64::from(c.stored) - tree;
        if t.optimal.len() < OPTIMAL_PER_KIND
            && piece.data.len() <= SAMPLE_PARSE_CAP
            && *spent < OPTIMAL_BUDGET
        {
            let began = Instant::now();
            let stream =
                optimal_deflate(&piece.data, !piece.raw).expect("the optimal parse deflates");
            let took = began.elapsed();
            *spent += took;
            t.optimal_time += took;
            t.optimal_inflated += piece.data.len() as u64;
            let back = inflate_exact(&stream, !piece.raw, piece.data.len());
            t.optimal_wrong += usize::from(back.as_deref() != Some(&piece.data[..]));
            t.optimal
                .push([i64::from(c.stored), tree, stream.len() as i64]);
        }
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
    headroom(t, &mut st.optimal_spent, table, &pieces);

    let (write, before) = match row {
        None => {
            let cut: Vec<usize> = body.chunks(PIECE).map(<[u8]>::len).collect();
            let stated: Vec<usize> = table.iter().map(|c| c.inflated as usize).collect();
            t.cut_ours += usize::from(cut == stated);
            (
                rewrite_row24(library, kind, &body),
                (body.clone(), Vec::new()),
            )
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
        // A 24-byte row's kind is named from its body's head below.
        let row24 = extra.nested.is_none();
        let kind = match extra.nested {
            Some(row) => HEADERLESS
                .iter()
                .find(|(_, codes)| codes.contains(&row.resource_type))
                .map(|(kind, _)| *kind),
            None if ROW24.iter().any(|k| st.wants(k)) => Some(PSO_TABLE),
            None => None,
        };
        let Some(mut kind) = kind.filter(|k| row24 || st.wants(k)) else {
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
        if row24 {
            let real =
                (entry.real).unwrap_or_else(|| table.iter().map(|c| u64::from(c.inflated)).sum());
            let head = walk_stored(&stored, &table[..1.min(table.len())]);
            let named =
                (head.ok()).and_then(|p| p.first().and_then(|p| kind_of_head(&p.data, real)));
            let Some(named) = named else {
                continue;
            };
            kind = named;
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

/// Milliseconds per 64 KiB inflated.
fn per_64k(time: Duration, inflated: u64) -> f64 {
    time.as_secs_f64() * 1000.0 * PIECE as f64 / inflated.max(1) as f64
}

fn print(kind: &str, t: &Tally) {
    let mut s = t.slack.clone();
    s.sort_unstable();
    let at = |f: f64| s[((s.len() as f64 * f) as usize).min(s.len().saturating_sub(1))];
    let band = |lo: i64, hi: i64| s.iter().filter(|&&v| (lo..=hi).contains(&v)).count();
    println!(
        "{kind}: {} bodies ({} enciphered), {} chunks ({} stored, {} deflated); {} deflated chunks fit, \
         {} bodies fit whole; {} deflated chunks past the patch writer's {} MiB cap; 24-byte \
         tables cut as OpenRage cuts: {}",
        t.bodies,
        t.ciphered,
        t.chunks,
        t.stored_chunks,
        t.deflated,
        t.fit,
        t.all_fit,
        t.past_cap,
        WRITER_CAP >> 20,
        t.cut_ours
    );
    if !s.is_empty() {
        println!(
            "  slack least {}, tenth {}, median {}, ninetieth {}, most {}; below 0: {}, 0 or more: {}",
            at(0.0),
            at(0.1),
            at(0.5),
            at(0.9),
            at(1.0),
            band(i64::MIN, -1),
            band(0, i64::MAX),
        );
    }
    if !t.optimal.is_empty() {
        let n = t.optimal.len();
        // Slack against the ceiling of the tree's best (1) or the optimal parse's (2) stream.
        let of = |i: usize| t.optimal.iter().map(move |c| c[0] - c[i]);
        let total = |i: usize| t.optimal.iter().map(|c| c[i]).sum::<i64>();
        let over = |i: usize| 100.0 * (total(i) - total(0)) as f64 / total(0).max(1) as f64;
        let mut slack: Vec<i64> = of(2).collect();
        slack.sort_unstable();
        println!(
            "  optimal parse over {n} of those chunks: {} fit, against {} at the tree's best \
             level; {:+.2} % over the stored bytes, against {:+.2} %; slack median {}, most {}; \
             {:.0} ms per 64 KiB; {} streams do not inflate back",
            of(2).filter(|&s| s >= 0).count(),
            of(1).filter(|&s| s >= 0).count(),
            over(2),
            over(1),
            slack[n / 2],
            slack[n - 1],
            per_64k(t.optimal_time, t.optimal_inflated),
            t.optimal_wrong,
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
    for kind in HEADERLESS.iter().map(|(k, _)| *k).chain(ROW24) {
        print(kind, st.kinds.get(kind).unwrap_or(&Tally::default()));
    }
    let kinds = || st.kinds.values();
    let measured: usize = kinds().map(|t| t.optimal.len()).sum();
    let fit_optimal = (kinds().flat_map(|t| &t.optimal))
        .filter(|c| c[0] >= c[2])
        .count();
    let fit_tree = (kinds().flat_map(|t| &t.optimal))
        .filter(|c| c[0] >= c[1])
        .count();
    let inflated = kinds().map(|t| t.optimal_inflated).sum();
    println!(
        "optimal parse in all: {fit_optimal} of {measured} chunks fit, against {fit_tree} at the \
         tree's best level; {:.1} s spent of the {} s budget, {:.0} ms per 64 KiB",
        st.optimal_spent.as_secs_f64(),
        OPTIMAL_BUDGET.as_secs(),
        per_64k(st.optimal_spent, inflated),
    );
    for failure in st.walk_failed.iter().take(20) {
        eprintln!("  {failure}");
    }
    let wrong: usize = kinds().map(|t| t.optimal_wrong).sum();
    assert_eq!(
        wrong, 0,
        "{wrong} optimal-parse streams do not inflate back to their chunk"
    );
    assert!(measured > 0, "the optimal parse measured no chunk");
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
    let texts = st.kinds.get(TEXT_TABLE).map(|t| &t.writes);
    assert!(
        texts.is_none_or(|w| w.keys().all(|label| label == "identical")),
        "a text table written over its own section is not byte-identical: {texts:?}"
    );
    let bodies = |kind: &str| st.kinds.get(kind).map_or(0, |t| t.bodies);
    for (kind, mirrored) in MIRRORED {
        if mirrored == 0 {
            assert_eq!(
                bodies(kind),
                0,
                "a {kind} body the node mirror does not list"
            );
        } else if !st.timed_out {
            assert!(
                bodies(kind) >= mirrored,
                "{} {kind} bodies sampled, where the node mirror samples {mirrored}",
                bodies(kind)
            );
        }
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
