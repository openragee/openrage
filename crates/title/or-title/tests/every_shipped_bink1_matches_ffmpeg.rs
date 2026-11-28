//! Every `.bik` an install ships, decoded by OpenRage's own decoders and by FFmpeg, compared: the
//! picture of each Bink 1 movie, the sound of every movie, Bink 1 or 2.
//!
//! Install-gated (`OPENRAGE_GAME_ROOT`, `;`-separated install roots) and FFmpeg-gated: this is the
//! one place FFmpeg runs, as the reference the ported decoders are held to, and a machine without
//! one records a skip. FFmpeg 7.0's `binkvideo`, `binkaudio_dct` and `binkaudio_rdft` decode each
//! movie to raw frames and raw samples; the picture is compared exactly, plane by plane, in the
//! YUV 4:2:0 both decode to, before any colour conversion, and the sound sample by sample within
//! `AUDIO_TOLERANCE`. A seek to three frames of each movie is held to the pictures decoded in order.
//!
//! `OPENRAGE_BINK_ONLY` narrows the run to the movies whose path holds that text, to debug one; a
//! narrowed run is not the check and says so.
//!
//! ⚠ **Every assertion of the full run is over an exercised count.** A walk that found no movie, or
//! movies none of which had sound, would leave every comparison passing.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};

use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
use or_io::container::RpfCryptoProvider;
use or_media::bik::{self, Pictures};
use or_media::Decoders;
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::game::Game;
use or_title::install::crawl::{self, ControlFlow};

const WHAT: &str = "every shipped Bink 1 movie decodes as FFmpeg decodes it";

/// The environment variable that narrows a run to the movies whose path holds its text.
const ONLY_ENV: &str = "OPENRAGE_BINK_ONLY";

/// The most a sample of sound may differ from FFmpeg's, on a scale where full volume is 1.0:
/// both decode to 32-bit floats, and ours is computed in double precision where FFmpeg's FFT runs
/// in single, so the two differ by rounding alone.
const AUDIO_TOLERANCE: f32 = 1.0e-4;

/// Worker threads comparing movies at once, each running its own FFmpeg.
const WORKERS: usize = 4;

fn ffmpeg() -> Option<PathBuf> {
    let program = PathBuf::from("ffmpeg");
    let runs = Command::new(&program)
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    runs.then_some(program)
}

/// What one movie's comparison saw.
#[derive(Default, Debug, Clone)]
struct Seen {
    pictures: u64,
    picture_bytes: u64,
    tracks: usize,
    samples: u64,
    max_audio_diff: f32,
}

/// A running FFmpeg, killed and reaped when a comparison fails before it has read it to the end.
struct Run(std::process::Child);

impl Drop for Run {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn run_ffmpeg(ffmpeg: &Path, file: &Path, args: &[&str]) -> Run {
    let child = Command::new(ffmpeg)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(file)
        .args(args)
        .arg("pipe:1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("FFmpeg starts");
    Run(child)
}

fn finish(mut child: Run, what: &str) -> Result<(), String> {
    let mut said = String::new();
    if let Some(mut stderr) = child.0.stderr.take() {
        let _ = stderr.read_to_string(&mut said);
    }
    let status = child.0.wait().map_err(|e| e.to_string())?;
    if !status.success() || !said.trim().is_empty() {
        return Err(format!("FFmpeg {what}: {status}: {}", said.trim()));
    }
    Ok(())
}

/// Where the seek check starts: a third of the way, half way and the last frame.
fn seek_starts(frames: usize) -> Vec<usize> {
    let mut starts = vec![frames / 3, frames / 2, frames.saturating_sub(1)];
    starts.dedup();
    starts
}

/// Every picture, plane by plane, against FFmpeg's, then a seek to a few frames against decoding
/// in order.
fn compare_pictures(
    ffmpeg: &Path,
    file: &Path,
    bytes: &Arc<[u8]>,
    seen: &mut Seen,
) -> Result<(), String> {
    let mut ours = Pictures::open(Arc::clone(bytes)).map_err(|e| e.to_string())?;
    let alpha = ours
        .next_picture()
        .map_err(|e| e.to_string())?
        .is_some_and(|p| p.alpha.is_some());
    ours.seek(0).map_err(|e| e.to_string())?;

    let mut child = run_ffmpeg(
        ffmpeg,
        file,
        &[
            "-map",
            "0:v:0",
            "-f",
            "rawvideo",
            "-pix_fmt",
            if alpha { "yuva420p" } else { "yuv420p" },
            "-fps_mode",
            "passthrough",
        ],
    );
    let mut stdout = child.0.stdout.take().expect("FFmpeg's stdout");
    // The frames a seek is checked on, and what decoding in order made of them.
    let frames = ours.frames();
    let watched: HashSet<usize> = seek_starts(frames)
        .into_iter()
        .flat_map(|start| [start, start + 1])
        .collect();
    let mut in_order = HashMap::new();
    let mut n = 0u64;
    while let Some(picture) = ours.next_picture().map_err(|e| format!("frame {n}: {e}"))? {
        let mut planes = vec![("Y", picture.y), ("U", picture.u), ("V", picture.v)];
        if let Some(a) = picture.alpha {
            planes.push(("A", a));
        }
        let mut hash = DefaultHasher::new();
        for (name, plane) in planes {
            let want = {
                let mut buf = vec![0u8; plane.width * plane.height];
                stdout
                    .read_exact(&mut buf)
                    .map_err(|e| format!("frame {n}: FFmpeg gave no {name} plane: {e}"))?;
                buf
            };
            let got = plane.packed();
            if got != want {
                let at = got.iter().zip(&want).position(|(a, b)| a != b).unwrap_or(0);
                return Err(format!(
                    "frame {n}, plane {name}: first difference at ({}, {}): ours {} and FFmpeg's {}",
                    at % plane.width,
                    at / plane.width,
                    got[at],
                    want[at]
                ));
            }
            if watched.contains(&(n as usize)) {
                want.hash(&mut hash);
            }
            seen.picture_bytes += want.len() as u64;
        }
        if watched.contains(&(n as usize)) {
            in_order.insert(n as usize, hash.finish());
        }
        n += 1;
    }
    let mut extra = Vec::new();
    stdout.read_to_end(&mut extra).map_err(|e| e.to_string())?;
    if !extra.is_empty() {
        return Err(format!(
            "FFmpeg gave {} bytes more than our {n} pictures",
            extra.len()
        ));
    }
    seen.pictures += n;
    finish(child, "decoding the picture")?;

    for start in seek_starts(frames) {
        ours.seek(start as u64).map_err(|e| e.to_string())?;
        for frame in start..(start + 2).min(frames) {
            let picture = ours
                .next_picture()
                .map_err(|e| format!("frame {frame} after a seek to {start}: {e}"))?
                .ok_or_else(|| format!("no frame {frame} after a seek to {start}"))?;
            let mut hash = DefaultHasher::new();
            for plane in [picture.y, picture.u, picture.v]
                .into_iter()
                .chain(picture.alpha)
            {
                plane.packed().hash(&mut hash);
            }
            if in_order.get(&frame) != Some(&hash.finish()) {
                return Err(format!(
                    "frame {frame} after a seek to {start} differs from the one decoded in order"
                ));
            }
        }
    }
    Ok(())
}

/// Every sound track, sample by sample, against FFmpeg's.
fn compare_sound(
    ffmpeg: &Path,
    file: &Path,
    bytes: &Arc<[u8]>,
    seen: &mut Seen,
) -> Result<(), String> {
    let tracks = bik::audio_tracks(bytes).map_err(|e| e.to_string())?;
    for (index, info) in tracks.iter().enumerate() {
        let map = format!("0:a:{index}");
        let mut child = run_ffmpeg(
            ffmpeg,
            file,
            &["-map", &map, "-f", "f32le", "-acodec", "pcm_f32le"],
        );
        let mut stdout = child.0.stdout.take().expect("FFmpeg's stdout");
        let mut stream =
            bik::audio_track(bytes, index, &Decoders::new()).map_err(|e| e.to_string())?;
        let what = format!(
            "track {index} ({}, {} channels, {} Hz)",
            info.codec.label(),
            info.channels,
            info.sample_rate
        );

        // A block at a time, so a long movie's sound is never held whole.
        let mut compared = 0u64;
        let mut diff = 0.0f32;
        while let Some(block) = stream.read().map_err(|e| format!("{what}: {e}"))? {
            let ours = block.into_f32();
            let mut raw = vec![0u8; ours.len() * 4];
            stdout.read_exact(&mut raw).map_err(|e| {
                format!("{what}: FFmpeg's sound ends before ours, at sample {compared}: {e}")
            })?;
            for (at, (a, b)) in ours.iter().zip(raw.as_chunks::<4>().0.iter()).enumerate() {
                let theirs = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                let d = (a - theirs).abs();
                if d > AUDIO_TOLERANCE || d.is_nan() {
                    return Err(format!(
                        "{what}: sample {} differs by {d}: ours {a} and FFmpeg's {theirs}",
                        compared + at as u64
                    ));
                }
                diff = diff.max(d);
            }
            compared += ours.len() as u64;
        }
        let mut extra = Vec::new();
        stdout.read_to_end(&mut extra).map_err(|e| e.to_string())?;
        if !extra.is_empty() {
            return Err(format!(
                "{what}: FFmpeg's sound goes on for {} samples after ours ends at {compared}",
                extra.len() / 4
            ));
        }
        finish(child, "decoding the sound")?;
        seen.tracks += 1;
        seen.samples += compared;
        seen.max_audio_diff = seen.max_audio_diff.max(diff);
    }
    Ok(())
}

fn compare_movie(ffmpeg: &Path, name: &str, bytes: Vec<u8>) -> Result<Seen, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let file = dir.path().join("movie.bik");
    std::fs::write(&file, &bytes).map_err(|e| e.to_string())?;
    let bytes: Arc<[u8]> = bytes.into();
    let mut seen = Seen::default();
    // A Bink 2 movie's picture is another decoder's; its sound is the same codecs as Bink 1's.
    let pictures = if bytes.starts_with(b"BIK") {
        compare_pictures(ffmpeg, &file, &bytes, &mut seen)
    } else {
        Ok(())
    };
    pictures
        .and_then(|()| compare_sound(ffmpeg, &file, &bytes, &mut seen))
        .map_err(|e| format!("{name}: {e}"))?;
    Ok(seen)
}

fn crypto_for(root: &Path) -> Option<Arc<dyn RpfCryptoProvider>> {
    let exe = Game::detect_from_root(root)?.find_executable(root)?;
    let keys = extract_keys(&exe, &|_| {}).ok()?;
    Some(Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys))))
}

#[test]
fn every_shipped_bink1_matches_ffmpeg() {
    let roots = game_roots();
    if roots.is_empty() {
        skip_install(
            WHAT,
            format_args!(
                "no install root; set {GAME_ROOT_ENV} to a ;-separated list of install directories"
            ),
        );
        return;
    }
    let Some(ffmpeg) = ffmpeg() else {
        skip_install(WHAT, "no FFmpeg on the PATH to decode the reference with");
        return;
    };

    // One movie waits beside the ones being compared: the largest are hundreds of megabytes.
    let (sender, receiver) = sync_channel::<(String, Vec<u8>)>(1);
    let receiver = Arc::new(Mutex::new(receiver));
    let results = Arc::new(Mutex::new(Vec::<Result<Seen, String>>::new()));
    let workers: Vec<_> = (0..WORKERS)
        .map(|_| {
            let (receiver, results, ffmpeg) =
                (Arc::clone(&receiver), Arc::clone(&results), ffmpeg.clone());
            std::thread::spawn(move || loop {
                let job = receiver.lock().expect("the queue").recv();
                let Ok((name, bytes)) = job else { return };
                let outcome = compare_movie(&ffmpeg, &name, bytes);
                results.lock().expect("the results").push(outcome);
            })
        })
        .collect();

    let only = std::env::var(ONLY_ENV).ok();
    let mut installs = 0;
    for root in &roots {
        let Some(provider) = crypto_for(root) else {
            continue;
        };
        installs += 1;
        let mut warnings = Vec::new();
        let _ = crawl::crawl(root, provider, &mut warnings, &mut |found| {
            let name = format!("{} {}::{}", root.display(), found.container, found.path);
            let wanted = only.as_deref().is_none_or(|text| name.contains(text));
            if !found.leaf.to_ascii_lowercase().ends_with(".bik") || !wanted {
                return ControlFlow::Continue(());
            }
            let Ok(bytes) = found.read() else {
                results
                    .lock()
                    .expect("the results")
                    .push(Err(format!("{}: did not extract", found.path)));
                return ControlFlow::Continue(());
            };
            sender.send((name, bytes)).expect("a worker is listening");
            ControlFlow::Continue(())
        });
        assert!(warnings.is_empty(), "{}: {warnings:?}", root.display());
    }
    drop(sender);
    for worker in workers {
        worker.join().expect("a worker finished");
    }
    if installs == 0 {
        skip_install(
            WHAT,
            "no install root yielded an executable to take keys from",
        );
        return;
    }

    let results = results.lock().expect("the results");
    let failures: Vec<&String> = results.iter().filter_map(|r| r.as_ref().err()).collect();
    let seen: Vec<&Seen> = results.iter().filter_map(|r| r.as_ref().ok()).collect();
    let total = Seen {
        pictures: seen.iter().map(|s| s.pictures).sum(),
        picture_bytes: seen.iter().map(|s| s.picture_bytes).sum(),
        tracks: seen.iter().map(|s| s.tracks).sum(),
        samples: seen.iter().map(|s| s.samples).sum(),
        max_audio_diff: seen.iter().map(|s| s.max_audio_diff).fold(0.0, f32::max),
    };
    eprintln!(
        "{} movies compared, {} failed; {} pictures ({} bytes), {} sound tracks ({} samples), \
         largest sound difference {}",
        results.len(),
        failures.len(),
        total.pictures,
        total.picture_bytes,
        total.tracks,
        total.samples,
        total.max_audio_diff
    );
    assert!(
        failures.is_empty(),
        "{} of {} movies differ from FFmpeg:\n{:#?}",
        failures.len(),
        results.len(),
        failures
    );
    if only.is_some() {
        eprintln!("{ONLY_ENV} narrowed this run: it is not the check");
        return;
    }
    assert!(
        results.len() >= 100 && total.pictures > 0 && total.tracks > 0,
        "{} movies, {} pictures and {} sound tracks were compared, so this check exercised too little",
        results.len(),
        total.pictures,
        total.tracks
    );
}
