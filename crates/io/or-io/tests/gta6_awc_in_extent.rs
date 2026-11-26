//! A GTA VI build's audio streams replaced by themselves, an Opus packet byte flipped, through the
//! tree's writer and laid in the source's extents: each saves byte-identical, laid in extent with other
//! bytes, or is refused by name, and every body laid in extent plans as a patched copy.
//!
//! ⚠ Install-gated: `OPENRAGE_GTA6_KEYS` names the key snapshot (never committed) and
//! `OPENRAGE_GTA6_BUILD` the build root holding `*.rpf` and `pack/*/*.rpf`.

use std::collections::BTreeMap;
use std::fmt::Display;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use or_io::awc::replace::{opus_peak_windows, NotInExtent, Refused};
use or_io::awc::{AwcChunkKind, AwcFile, AwcKey, Codec};
use or_io::container::rpf::{
    plan_patch, BodyPatch, Gta6Class, Gta6Mount, GtaViKeys, GTA6_SNAPSHOT_ENV,
};
use or_test_support::skip_install;

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
const MAX_DEPTH: usize = 8;
/// A body longer than this is not sampled.
const CAP: u64 = 8 << 20;
/// Containers sampled, and the walk's bound in time.
const CONTAINERS: usize = 300;
const DEADLINE: Duration = Duration::from_secs(240);

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
    for pack in (std::fs::read_dir(root.join("pack")).into_iter().flatten()).flatten() {
        if pack.path().is_dir() {
            rpfs_in(&pack.path());
        }
    }
    out.sort();
    out
}

/// A message with its digits masked, so refusals that differ only by a count tally together.
fn masked(message: impl Display) -> String {
    (message.to_string().chars())
        .map(|c| if c.is_ascii_digit() { '#' } else { c })
        .collect()
}

/// One byte past the TOC byte of the first packet long enough flipped, so a body re-laid from the
/// packets differs from its source at its length and is planned as a patch.
fn flip_one(packets: &mut [Vec<u8>]) {
    if let Some(packet) = packets.iter_mut().find(|p| p.len() > 1) {
        packet[1] ^= 0x01;
    }
}

/// Stream `i` replaced by what it holds, one Opus byte flipped, through the writer its kind has;
/// `None` for a kind with none here.
fn by_itself(awc: &AwcFile, i: usize) -> Option<Result<AwcFile, Refused>> {
    let s = &awc.streams[i];
    if let Ok(Some(format)) = s.opus_format() {
        let mut own = awc.opus_streams(i).ok()?.into_iter().next()?;
        flip_one(&mut own.packets);
        let mut peaks = vec![format.peak];
        match s.peak_envelope() {
            Some(rest) => peaks.extend(rest),
            // With no peak chunk only window 0 is written; the rest stand in to the stated count.
            None => peaks.resize(opus_peak_windows(own.samples), format.peak),
        }
        let loudness: Vec<f32> = (s.chunk(AwcChunkKind::Loudness))
            .map(|c| {
                (c.payload.as_chunks::<8>().0.iter())
                    .map(|r| f32::from_le_bytes([r[4], r[5], r[6], r[7]]))
                    .collect()
            })
            .unwrap_or_default();
        return Some(awc.with_opus_replaced(i, &own, &peaks, &loudness));
    }
    if let Ok(Some(format)) = s.coded_format() {
        if format.codec != Codec::Pcm16 {
            return None;
        }
        let pcm = awc.stream_data(s, AwcKey::default())?;
        let rate = u32::from(format.stated_rate);
        return Some(awc.with_stream_replaced(i, &pcm, rate, 1, AwcKey::default()));
    }
    if s.stream_format().is_some() {
        let mut own = awc.opus_streams(i).ok().filter(|o| !o.is_empty())?;
        own.iter_mut()
            .for_each(|channel| flip_one(&mut channel.packets));
        return Some(awc.with_opus_channels_replaced(i, &own));
    }
    None
}

#[derive(Default)]
struct State {
    started: Option<Instant>,
    containers: usize,
    unparsed: BTreeMap<String, usize>,
    outcomes: BTreeMap<String, usize>,
    planned: usize,
    wrong: Vec<String>,
}

impl State {
    fn done(&self) -> bool {
        self.containers >= CONTAINERS || self.started.is_some_and(|t| t.elapsed() > DEADLINE)
    }
}

fn tally(map: &mut BTreeMap<String, usize>, key: String) {
    *map.entry(key).or_default() += 1;
}

fn measure(
    st: &mut State,
    file: &mut File,
    keys: &GtaViKeys,
    mount: &Gta6Mount,
    entry: u32,
    body: Vec<u8>,
) {
    let awc = match AwcFile::parse(&body, AwcKey::default()) {
        Ok(awc) => awc,
        Err(e) => return tally(&mut st.unparsed, masked(e)),
    };
    st.containers += 1;
    let what = format!("entry {entry} of the archive at {:#x}", mount.base);
    for i in 0..awc.streams.len() {
        let Some(edited) = by_itself(&awc, i) else {
            tally(
                &mut st.outcomes,
                "no writer here for this stream's kind".into(),
            );
            continue;
        };
        let laid = match edited
            .map_err(NotInExtent::from)
            .and_then(|e| awc.in_extent(&e))
        {
            Ok(laid) => laid,
            Err(NotInExtent::Refused(why)) => {
                tally(
                    &mut st.outcomes,
                    format!("replace refused: {}", masked(why)),
                );
                continue;
            }
            Err(why) => {
                tally(
                    &mut st.outcomes,
                    format!("refused in extent: {}", masked(why)),
                );
                continue;
            }
        };
        let out = match laid.to_bytes(AwcKey::default()) {
            Ok(out) => out,
            Err(e) => {
                st.wrong
                    .push(format!("{what}, stream {i}: not written: {e}"));
                continue;
            }
        };
        let same = out == body;
        let label = if same {
            "byte-identical"
        } else {
            "laid in extent, other bytes"
        };
        tally(&mut st.outcomes, label.into());
        let read = AwcFile::parse(&out, AwcKey::default());
        let offsets = |f: &AwcFile| -> Vec<u32> {
            (f.streams.iter().flat_map(|s| &s.chunks))
                .map(|c| c.offset)
                .collect()
        };
        match read {
            Ok(read) if out.len() == body.len() && offsets(&read) == offsets(&awc) => {
                if read.opus_streams(i).ok() != laid.opus_streams(i).ok() {
                    st.wrong
                        .push(format!("{what}, stream {i}: its packets do not read back"));
                }
            }
            Ok(_) => st
                .wrong
                .push(format!("{what}, stream {i}: a length or an offset moved")),
            Err(e) => st
                .wrong
                .push(format!("{what}, stream {i}: does not reparse: {e}")),
        }
        if same {
            continue;
        }
        let patch = BodyPatch {
            base: mount.base,
            entry,
            body: out,
        };
        match plan_patch(&mut *file, keys, &patch) {
            Ok(_) => st.planned += 1,
            Err(e) => st
                .wrong
                .push(format!("{what}, stream {i}: no patched copy: {e}")),
        }
    }
}

/// Sample the clear stored `ADAT` bodies of `mount` and of every archive nested in it.
fn walk(file: &mut File, keys: &GtaViKeys, mount: &Gta6Mount, depth: usize, st: &mut State) {
    let extras = mount.archive.entry_extras();
    for (entry, extra) in mount.archive.entries.iter().zip(&extras) {
        if st.done() {
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
        let (Some(start), Some(real)) = (mount.absolute_start(entry), entry.real) else {
            continue;
        };
        if entry.cipher.is_some() || extra.nested.is_some() || !(16..=CAP).contains(&real) {
            continue;
        }
        let mut head = [0u8; 4];
        let read =
            (file.seek(SeekFrom::Start(start)).ok()).and_then(|_| file.read_exact(&mut head).ok());
        if read.is_none() || &head != b"ADAT" {
            continue;
        }
        let mut body = vec![0u8; real as usize];
        if file.seek(SeekFrom::Start(start)).is_err() || file.read_exact(&mut body).is_err() {
            continue;
        }
        measure(st, file, keys, mount, entry.index, body);
    }
}

#[test]
fn the_builds_audio_streams_replaced_by_themselves_save_in_extent_or_are_refused_by_name() {
    let what = "GTA VI audio streams replaced by themselves in extent";
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
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys).expect("the archive opens");
        walk(&mut file, &keys, &mount, 0, &mut st);
    }

    println!(
        "{} container(s) in {:.0?}; {} body(ies) laid in extent with other bytes planned as a \
         patched copy",
        st.containers,
        st.started.map_or(Duration::ZERO, |t| t.elapsed()),
        st.planned
    );
    for (label, n) in &st.outcomes {
        println!("  {n} x {label}");
    }
    for (label, n) in &st.unparsed {
        println!("  not parsed: {n} x {label}");
    }
    for wrong in st.wrong.iter().take(20) {
        eprintln!("  {wrong}");
    }
    assert!(
        st.containers > 0,
        "no clear stored audio container in the build's tables"
    );
    let laid = st
        .outcomes
        .get("laid in extent, other bytes")
        .copied()
        .unwrap_or(0);
    assert!(laid > 0, "no stream was laid in extent with other bytes");
    assert!(
        st.planned > 0,
        "no body laid in extent reached the patch writer"
    );
    assert!(
        st.wrong.is_empty(),
        "{} stream(s) laid in extent went wrong",
        st.wrong.len()
    );
}
