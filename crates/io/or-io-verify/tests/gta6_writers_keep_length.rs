//! Whether OpenRage's writers give a GTA VI body back at its own length, which a patched copy needs:
//! over a bounded sample of a build's text tables, texture dictionaries (type 0x05) and drawable
//! dictionaries (type 0x04), each body is read and written back unchanged through the writer that
//! keeps its layout, and counted as identical, of the same length with other bytes, of another
//! length, or unread. Measured first by a node mirror over the Xbox build 1.0.2.1: 1,118 of 1,118
//! text tables rebuilt over their own sections came back byte-identical, every entry of 297 type
//! 0x05 bodies is a texture record, and none of 15,967 entries of 3,000 type 0x04 bodies is.
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
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use or_io::container::rpf::gta6_chunk_table::entry_chunks;
use or_io::container::rpf::{
    read_rpf9_body, Gta6Class, Gta6Entry, Gta6Mount, Gta6NestedRow, GtaViKeys, Rpf9Role,
    Rpf9Sections, GTA6_SNAPSHOT_ENV,
};
use or_io::gta6::headerless::{
    read_dictionary, read_dictionary_model, write_dictionary, write_shaders, Entry,
};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
const CAP: u64 = 4 << 20;
const MAX_DEPTH: usize = 8;
/// The 16-byte zero slot a headerless body begins with.
const SLOT: u64 = 16;
const PER_KIND: usize = 40;
const PER_ARCHIVE: usize = 8;
const DEADLINE: Duration = Duration::from_secs(300);

const TEXT_TABLE: &str = "text table";
const TEXTURE_DICTIONARY: &str = "texture dictionary";
const DRAWABLE_DICTIONARY: &str = "drawable dictionary";
const KINDS: [(&str, u32); 3] = [
    (TEXT_TABLE, 0x6F),
    (TEXTURE_DICTIONARY, 0x05),
    (DRAWABLE_DICTIONARY, 0x04),
];
/// How the texture dictionary reader refuses a dictionary one of whose entries is no texture.
const NO_TEXTURE: &str = "an entry is no texture record";

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

/// The stored bytes of entry `e`: deciphered where it is enciphered, past the zero slot.
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
    let len = row.stored_size().saturating_sub(SLOT);
    let mut body = vec![0u8; len as usize];
    src.seek(SeekFrom::Start(start + SLOT))
        .map_err(|e| e.to_string())?;
    src.read_exact(&mut body).map_err(|e| e.to_string())?;
    Ok(body)
}

/// What a kind's writer makes of a body read and written back unchanged.
enum Rewrite {
    Wrote(Vec<u8>, Vec<u8>),
    Unread(String),
    Refused(String),
}

/// The body of `kind`, its sections `s`, read and written back by the writer that keeps its layout.
fn rewrite(kind: &str, s: &Rpf9Sections) -> Rewrite {
    let (v, p) = (&s.virtual_section, &s.physical);
    match kind {
        TEXT_TABLE => match or_io::gta6::text::read(v) {
            Err(e) => Rewrite::Unread(masked(e)),
            Ok((table, _)) => match table.to_section_over(v) {
                Ok(out) => Rewrite::Wrote(out, p.clone()),
                Err(e) => Rewrite::Refused(masked(e)),
            },
        },
        TEXTURE_DICTIONARY => match read_dictionary_model(v, p) {
            Err(e) => Rewrite::Unread(masked(e)),
            Ok(model) => match write_dictionary(&model) {
                Ok(out) => Rewrite::Wrote(out.virtual_section, out.physical),
                Err(e) => Rewrite::Refused(masked(e)),
            },
        },
        _ => match read_dictionary(v, Some(p.as_slice())) {
            Err(e) => Rewrite::Unread(masked(e)),
            Ok(dictionary) => {
                let mut out = v.clone();
                for (code, entry) in &dictionary.entries {
                    match entry {
                        Err(e) => return Rewrite::Unread(masked(e)),
                        Ok(Entry::Drawable(d)) => {
                            let written = write_shaders(
                                Rpf9Role::Dictionary,
                                &mut out,
                                Some(*code),
                                &d.shaders,
                            );
                            if let Err(e) = written {
                                return Rewrite::Refused(masked(e));
                            }
                        }
                        Ok(_) => {}
                    }
                }
                Rewrite::Wrote(out, p.clone())
            }
        },
    }
}

#[derive(Default)]
struct State {
    /// Per kind, each outcome's count.
    kinds: BTreeMap<&'static str, BTreeMap<String, usize>>,
    /// Drawable dictionaries the texture dictionary reader refuses, and why.
    as_textures: BTreeMap<String, usize>,
    here: BTreeMap<&'static str, usize>,
    started: Option<Instant>,
    timed_out: bool,
}

impl State {
    fn bodies(&self, kind: &str) -> usize {
        self.kinds.get(kind).map_or(0, |t| t.values().sum())
    }

    fn wants(&self, kind: &str) -> bool {
        self.bodies(kind) < PER_KIND && self.here.get(kind).copied().unwrap_or(0) < PER_ARCHIVE
    }

    fn done(&mut self) -> bool {
        if self.started.is_some_and(|t| t.elapsed() > DEADLINE) {
            self.timed_out = true;
            return true;
        }
        KINDS.iter().all(|(k, _)| self.bodies(k) >= PER_KIND)
    }

    fn count(&mut self, kind: &'static str, label: String) {
        *self.here.entry(kind).or_default() += 1;
        *self
            .kinds
            .entry(kind)
            .or_default()
            .entry(label)
            .or_default() += 1;
    }
}

/// Measure one stored body of `kind`.
fn measure(st: &mut State, kind: &'static str, stored: &[u8], limit: u64) {
    let read = read_rpf9_body(Cursor::new(stored), 0, limit)
        .ok()
        .and_then(|(find, bytes)| Rpf9Sections::of(&find, &bytes));
    let Some(s) = read else {
        st.count(kind, "unread: the resource reader finds no sections".into());
        return;
    };
    if kind == DRAWABLE_DICTIONARY {
        let why = match read_dictionary_model(&s.virtual_section, &s.physical) {
            Ok(_) => "read".to_string(),
            Err(e) => masked(e),
        };
        *st.as_textures.entry(why).or_default() += 1;
    }
    let label = match rewrite(kind, &s) {
        Rewrite::Unread(why) => format!("unread: {why}"),
        Rewrite::Refused(why) => format!("refused: {why}"),
        Rewrite::Wrote(v, p) if (&v, &p) == (&s.virtual_section, &s.physical) => "identical".into(),
        Rewrite::Wrote(v, p)
            if (v.len(), p.len()) == (s.virtual_section.len(), s.physical.len()) =>
        {
            "same length, other bytes".into()
        }
        Rewrite::Wrote(..) => "another length".into(),
    };
    st.count(kind, label);
}

/// Sample the bodies of `mount`, the archive at `depth`, and of every archive nested in it.
fn walk(file: &mut File, keys: &GtaViKeys, (mount, depth): (&Gta6Mount, usize), st: &mut State) {
    let extras = mount.archive.entry_extras();
    for (entry, extra) in mount.archive.entries.iter().zip(&extras) {
        if st.done() {
            return;
        }
        if entry.kind() == Some(Gta6Class::NestedArchive) {
            if depth < MAX_DEPTH {
                if let Ok(child) = mount.open_nested(&mut *file, entry, keys) {
                    walk(file, keys, (&child, depth + 1), st);
                }
            }
            continue;
        }
        let Some(row) = extra.nested else {
            continue;
        };
        let Some(kind) = (KINDS.iter())
            .find(|(k, code)| *code == row.resource_type && st.wants(k))
            .map(|(k, _)| *k)
        else {
            continue;
        };
        if row.stored_size() > CAP {
            continue;
        }
        let Ok(Some(table)) = entry_chunks(&mount.archive, entry.index as usize) else {
            continue;
        };
        // A body whose content key the snapshot lacks is not sampled.
        let Ok(stored) = stored_of(mount, file, keys, entry, row) else {
            continue;
        };
        let limit = table.iter().map(|c| u64::from(c.inflated)).sum();
        measure(st, kind, &stored, limit);
    }
}

#[test]
fn each_writer_gives_a_body_back_at_its_length_or_says_why_not() {
    let what = "GTA VI writers that keep a body's length over a build";
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
    let mut st = State {
        started: Some(Instant::now()),
        ..State::default()
    };
    for path in &archives {
        if st.done() {
            break;
        }
        st.here.clear();
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys).expect("the archive opens");
        walk(&mut file, &keys, (&mount, 0), &mut st);
    }

    if st.timed_out {
        println!("the walk stopped at its {} s bound", DEADLINE.as_secs());
    }
    let empty = BTreeMap::new();
    for (kind, _) in KINDS {
        let t = st.kinds.get(kind).unwrap_or(&empty);
        println!("{kind}: {} bodies", st.bodies(kind));
        for (label, n) in t {
            println!("  written back unchanged: {n} x {label}");
        }
    }
    for (why, n) in &st.as_textures {
        println!("  drawable dictionaries the texture dictionary reader takes: {n} x {why}");
    }

    let of = |kind: &str| st.kinds.get(kind).unwrap_or(&empty);
    let texts = of(TEXT_TABLE);
    assert!(
        texts.keys().all(|label| label == "identical"),
        "a text table written over its own section is not byte-identical: {texts:?}"
    );
    let textures = of(TEXTURE_DICTIONARY);
    assert!(
        textures.keys().all(|label| !label.contains(NO_TEXTURE)),
        "a type 0x05 entry is no texture record: {textures:?}"
    );
    assert!(
        st.as_textures.keys().all(|why| why.contains(NO_TEXTURE)),
        "a type 0x04 body reads as textures: {:?}",
        st.as_textures
    );
    if !st.timed_out {
        for (kind, _) in KINDS {
            assert!(
                st.bodies(kind) > 0,
                "the build holds no sampled {kind} body"
            );
        }
    }
}
