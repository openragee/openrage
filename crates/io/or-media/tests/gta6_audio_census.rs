//! R72i's audio census: the codecs GTA VI's `.awc` containers state, the PlayStation 5 build beside
//! the Xbox Series build, and the invariant that each is decoded or refused by name.
//!
//! For each build `OPENRAGE_GAME_ROOT` names (told apart by its `eboot.bin`), every RPF9 archive and
//! every nested archive is mounted with the key snapshot, and every clear stored `ADAT` body is
//! parsed as an `.awc` container. A GTA VI audio body is stored in the clear on both builds, so the
//! walk is the same whatever the platform's body storage — it is the codec a stream's format record
//! states that is expected to differ. Each stream's stated codec byte is read straight from its
//! format record (the authoritative byte, never read as the nearest named one) and tallied per build.
//!
//! Then the safety invariant every build holds: a codec OpenRage decodes is decoded (the first stream
//! of each codec opens through the media table and reads to samples), and a codec it does not is
//! refused by name — the refusal names the codec, and no codec without a decoder row ever yields
//! samples. PlayStation platforms commonly carry ATRAC9 where Xbox carries an XMA- or ADPCM-family
//! codec, so the PlayStation 5 tag is expected to differ; whatever it is, it is named and either
//! decoded or refused, never silently wrong.
//!
//! ⚠ Install-gated and slow-tier (it walks whole installs), so it skips cleanly without:
//!   OPENRAGE_GTA6_KEYS   the `gtavi_magic.dat` key snapshot (never committed)
//!   OPENRAGE_GAME_ROOT   the build roots, `;`-separated, each holding `*.rpf` and `pack/*/*.rpf`

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use or_io::awc::{AwcChunkKind, AwcFile, AwcKey, AwcStream};
use or_io::container::rpf::{Gta6Class, Gta6Mount, GtaViKeys, GTA6_SNAPSHOT_ENV};
use or_io::Platform;
use or_media::{Codec, Decoders};
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};

const MAX_DEPTH: usize = 8;
/// An `.awc` body longer than this is not sampled; the largest ship well under it.
const CAP: u64 = 64 << 20;
/// Containers sampled per build, and the walk's bound in time, so the census stays within the tier.
const CONTAINERS: usize = 600;
const DEADLINE: Duration = Duration::from_secs(240);

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

/// Whether the folder is a PlayStation 5 build, by the `eboot.bin` beside its archives.
fn platform_of(root: &Path) -> Platform {
    if root.join("eboot.bin").is_file() {
        Platform::Ps5
    } else {
        Platform::XboxSeries
    }
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

/// The codec byte(s) a stream's format record states, read straight from the record: a GTA VI 32-byte
/// record's byte 0x1C, a GTA V single-stream `format`, or each channel of a `streamformat`. Empty for
/// a stream that carries no format record of its own (a multi-channel container's child streams).
fn stated_codecs(s: &AwcStream) -> Vec<u8> {
    if let Some(record) = s.chunk(AwcChunkKind::OpusFormat) {
        return record
            .payload
            .get(0x1C)
            .map(|b| vec![*b])
            .unwrap_or_default();
    }
    if let Some(format) = s.format() {
        return vec![format.codec.as_byte()];
    }
    if let Some(format) = s.stream_format() {
        return format.channels.iter().map(|c| c.codec.as_byte()).collect();
    }
    Vec::new()
}

/// A readable name for a codec byte, the media table's label where it has one.
fn label(byte: u8) -> String {
    // Codec 2 is GTA VI's Spectra, which the media layer names so once read; the container states the
    // bare byte.
    if byte == 2 {
        return "codec 2 (Spectra)".to_string();
    }
    Codec::from(or_io::awc::Codec::from_byte(byte)).label()
}

/// What opening the first stream of a codec through the media table did.
enum Outcome {
    /// The media table decoded it to this many samples.
    Decoded(usize),
    /// It was refused; the message, and whether the table even has a row for the codec.
    Refused { message: String, has_row: bool },
}

/// One build's census.
#[derive(Default)]
struct Build {
    containers: usize,
    unparsed: usize,
    streams: usize,
    /// Streams with no readable format record of their own (a container's interleaved children).
    headless: usize,
    /// Per stated codec byte: how many streams, and the outcome of spot-opening the first of them.
    codecs: BTreeMap<u8, (usize, Option<Outcome>)>,
}

impl Build {
    /// Spot-open stream `i` of `file`, whose stated codec is `byte`, the first time the build meets it.
    fn spot(&mut self, file: &AwcFile, i: usize, byte: u8) {
        let entry = self.codecs.entry(byte).or_insert((0, None));
        entry.0 += 1;
        if entry.1.is_some() {
            return;
        }
        let codec = Codec::from(or_io::awc::Codec::from_byte(byte));
        let has_row = Decoders::new().audio_decoder(codec).is_ok();
        let outcome = match or_media::awc::open(file, AwcKey::default(), i)
            .and_then(|src| src.audio(&Decoders::new()))
            .and_then(|mut audio| audio.read_all_i16())
        {
            Ok(pcm) => Outcome::Decoded(pcm.len()),
            Err(e) => Outcome::Refused {
                message: e.to_string(),
                has_row,
            },
        };
        entry.1 = Some(outcome);
    }
}

/// Parse one `ADAT` body and fold its streams' codecs into `build`.
fn measure(build: &mut Build, body: &[u8]) {
    let file = match AwcFile::parse(body, AwcKey::default()) {
        Ok(file) => file,
        Err(_) => {
            build.unparsed += 1;
            return;
        }
    };
    build.containers += 1;
    for i in 0..file.streams.len() {
        build.streams += 1;
        let bytes = stated_codecs(&file.streams[i]);
        if bytes.is_empty() {
            build.headless += 1;
            continue;
        }
        for byte in bytes {
            build.spot(&file, i, byte);
        }
    }
}

/// Sample the clear stored `ADAT` bodies of `mount` and of every archive nested in it.
fn walk(
    file: &mut File,
    keys: &GtaViKeys,
    mount: &Gta6Mount,
    depth: usize,
    build: &mut Build,
    started: Instant,
) {
    let extras = mount.archive.entry_extras();
    for (entry, extra) in mount.archive.entries.iter().zip(&extras) {
        if build.containers >= CONTAINERS || started.elapsed() > DEADLINE {
            return;
        }
        if entry.kind() == Some(Gta6Class::NestedArchive) {
            if depth < MAX_DEPTH {
                if let Ok(child) = mount.open_nested(&mut *file, entry, keys) {
                    walk(file, keys, &child, depth + 1, build, started);
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
        measure(build, &body);
    }
}

fn census_of(archives: &[PathBuf], keys: &GtaViKeys) -> Build {
    let mut build = Build::default();
    let started = Instant::now();
    for path in archives {
        if build.containers >= CONTAINERS || started.elapsed() > DEADLINE {
            break;
        }
        let Ok(mut file) = File::open(path) else {
            continue;
        };
        match Gta6Mount::open_root(&mut file, keys) {
            Ok(mount) => walk(&mut file, keys, &mount, 0, &mut build, started),
            Err(e) => eprintln!("open {}: {e}", path.display()),
        }
    }
    build
}

/// Print one build's table, and return the hard failures: a codec that produced samples though the
/// table has no decoder row for it — a silent wrong output, the one thing a census must never see.
/// A codec refused without its name in the message is reported but not failed (an edge like a stream
/// with no data chunk refuses before the decoder names the codec).
fn report(platform: Platform, build: &Build) -> Vec<String> {
    println!(
        "\n{} — {} container(s), {} stream(s); {} body(ies) not an .awc, {} stream(s) with no record",
        platform.label(),
        build.containers,
        build.streams,
        build.unparsed,
        build.headless,
    );
    let mut wrong = Vec::new();
    for (&byte, (count, outcome)) in &build.codecs {
        let has_row = Decoders::new()
            .audio_decoder(Codec::from(or_io::awc::Codec::from_byte(byte)))
            .is_ok();
        let note = match outcome {
            Some(Outcome::Decoded(samples)) => {
                if !has_row {
                    wrong.push(format!(
                        "{}: decoded to {samples} samples with no decoder row",
                        label(byte)
                    ));
                }
                format!("decoded, {samples} samples")
            }
            Some(Outcome::Refused { message, has_row }) => {
                if !has_row && !message.contains(&label(byte)) {
                    eprintln!(
                        "  note: {} refused without its name in the message: {message}",
                        label(byte)
                    );
                }
                format!("refused: {message}")
            }
            None => "not spot-checked".to_string(),
        };
        let kind = if has_row { "decodes" } else { "no row" };
        println!(
            "    codec {byte:>3} {:<20} x{count:<6} [{kind}] {note}",
            label(byte)
        );
    }
    wrong
}

#[test]
fn codecs_only_one_build_states_are_named_and_shared_ones_line_up() {
    // Pure logic: the set difference the cross-build summary prints, over hand-made tallies.
    let xbox: BTreeMap<u8, usize> = BTreeMap::from([(11, 100), (0, 20)]); // Opus, PCM16
    let ps5: BTreeMap<u8, usize> = BTreeMap::from([(8, 90), (0, 18)]); // a new codec 8, PCM16
    let only = |a: &BTreeMap<u8, usize>, b: &BTreeMap<u8, usize>| -> Vec<u8> {
        a.keys().filter(|k| !b.contains_key(*k)).copied().collect()
    };
    assert_eq!(only(&xbox, &ps5), vec![11], "Opus is Xbox-only here");
    assert_eq!(
        only(&ps5, &xbox),
        vec![8],
        "codec 8 is PlayStation 5-only here"
    );
    assert_eq!(label(8), "unknown(8)");
    assert_eq!(label(11), "Opus");
    assert_eq!(label(2), "codec 2 (Spectra)");
}

#[test]
fn every_codec_both_builds_state_is_decoded_or_refused_by_name() {
    let what = "GTA VI audio census over the builds";
    let Some(snapshot) = env_path(GTA6_SNAPSHOT_ENV) else {
        skip_install(
            what,
            format_args!("set {GTA6_SNAPSHOT_ENV} and {GAME_ROOT_ENV}"),
        );
        return;
    };
    let roots = game_roots();
    if roots.is_empty() {
        skip_install(what, format_args!("{GAME_ROOT_ENV} names no GTA VI folder"));
        return;
    }
    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");

    let mut by_platform: BTreeMap<&str, (Build, Vec<String>)> = BTreeMap::new();
    let mut any = false;
    for root in &roots {
        let archives = build_archives(root);
        if archives.is_empty() {
            continue;
        }
        let platform = platform_of(root);
        let build = census_of(&archives, &keys);
        any |= build.containers > 0;
        let wrong = report(platform, &build);
        by_platform
            .entry(platform.label())
            .or_insert((build, wrong));
    }

    if !any {
        skip_install(
            what,
            format_args!("no `.awc` container under the folders {GAME_ROOT_ENV} names"),
        );
        return;
    }

    // The codecs each build states, side by side, for judgement.
    let codec_set = |b: &Build| {
        b.codecs
            .keys()
            .map(|k| label(*k))
            .collect::<Vec<_>>()
            .join(", ")
    };
    for (name, (build, _)) in &by_platform {
        eprintln!("{name} states: {}", codec_set(build));
    }
    if let (Some((xbox, _)), Some((ps5, _))) = (
        by_platform.get(Platform::XboxSeries.label()),
        by_platform.get(Platform::Ps5.label()),
    ) {
        let only = |a: &Build, b: &Build| {
            a.codecs
                .keys()
                .filter(|k| !b.codecs.contains_key(*k))
                .map(|k| label(*k))
                .collect::<Vec<_>>()
        };
        let xbox_only = only(xbox, ps5);
        let ps5_only = only(ps5, xbox);
        if !xbox_only.is_empty() {
            eprintln!("codecs only Xbox states: {}", xbox_only.join(", "));
        }
        if !ps5_only.is_empty() {
            eprintln!("codecs only PlayStation 5 states: {}", ps5_only.join(", "));
        }
    } else {
        eprintln!(
            "only {} measured; name both builds in {GAME_ROOT_ENV} for a cross-build comparison",
            by_platform
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(" and "),
        );
    }

    let wrong: Vec<String> = by_platform.into_values().flat_map(|(_, w)| w).collect();
    assert!(
        wrong.is_empty(),
        "{} codec(s) produced wrong or unnamed output; a codec with no decoder must refuse by name:\n{}",
        wrong.len(),
        wrong.join("\n"),
    );
}
