//! A `.bik` opens as its own kind, and its preview is decoded in OpenRage's own code, picture and
//! sound, whether it is Bink 1 or Bink 2, or says what is wrong and why it cannot play.
//!
//! ⚠ Install-gated where it decodes a real picture: a shipped movie is the only honest input,
//! because neither decoder comes with an encoder. A small Bink 2 movie written by hand, a key frame
//! that codes nothing and frames that repeat it, plays everywhere. On Windows an install's own
//! `bink2w64.dll` holds the Bink 2 decoder to the game's pictures.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
use or_io::container::rpf::RpfArchive;
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::game::Game;

use super::{AppState, DocPayload, DocumentId, DocumentKind, LoadingState};
use crate::audio::{AudioOut, NullSink};
use crate::bink;
use crate::video::{PREVIEW_HEIGHT, PREVIEW_WIDTH, ROUTE};

/// The smallest Bink 1 movie a Legacy install ships, and the archive holding it.
const SHIPPED_BIK: (&str, &str) = ("x64d.rpf", "movies/2secondsblack.bik");
/// A small Bink 1 movie with audible stereo sound, in the same archive; many ship a silent track.
const SHIPPED_BIK_WITH_SOUND: (&str, &str) = ("x64d.rpf", "movies/cnt_lottery.bik");
/// The Bink 2 movie both installs' update archive ships.
const SHIPPED_BINK2: (&str, &str) = ("update/update.rpf", "x64/movies/rockstar_logos.bik");

fn open(state: &mut AppState, path: PathBuf) -> DocumentId {
    state.load_file(path);
    let deadline = Instant::now() + Duration::from_secs(30);
    while state.load_receiver.is_some() && Instant::now() < deadline {
        state.poll_load_result();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        matches!(state.loading, LoadingState::Idle),
        "the load did not finish"
    );
    let doc = state
        .active_document
        .expect("the .bik opened as a document");
    assert_eq!(state.document(doc).map(|d| d.kind), Some(DocumentKind::Bik));
    doc
}

fn shipped_bik(root: &Path, (rpf, path): (&str, &str)) -> Vec<u8> {
    let exe = Game::detect_from_root(root)
        .and_then(|game| game.find_executable(root))
        .expect("the install's executable");
    let keys = Arc::new(extract_keys(&exe, &|_| {}).expect("keys from the install's executable"));
    let provider = Arc::new(Gta5RetailCryptoProvider::new(keys));
    let archive = RpfArchive::open_with(root.join(rpf), provider).expect("the archive opens");
    archive.read(path).expect("the movie extracts")
}

#[test]
fn a_shipped_bik_opens_and_its_first_frame_decodes_at_the_preview_size() {
    const WHAT: &str = "a shipped .bik's first frame decodes in OpenRage's own code";
    let Some(root) = game_roots()
        .into_iter()
        .find(|root| root.join("GTA5.exe").is_file() && root.join(SHIPPED_BIK.0).is_file())
    else {
        skip_install(
            WHAT,
            format_args!("no Legacy install root; set {GAME_ROOT_ENV}"),
        );
        return;
    };

    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("2secondsblack.bik");
    std::fs::write(&path, shipped_bik(&root, SHIPPED_BIK)).expect("the movie is written");

    let mut state = AppState::default();
    let doc = open(&mut state, path);
    let mut preview = state.video_preview(doc).expect("a .bik has a preview");

    assert!(
        preview.wait_for_frame(Duration::from_secs(30)),
        "no frame arrived: {:?}",
        preview.problem()
    );
    assert_eq!(preview.decoder(), Some(ROUTE));
    let frame = preview.frame().expect("a frame is shown");
    assert_eq!(frame.len(), (PREVIEW_WIDTH * PREVIEW_HEIGHT * 4) as usize);
    assert!(
        frame.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 255),
        "a movie with no alpha plane decodes opaque; a frame that is not was never decoded"
    );
    drop(preview);

    let Some(DocPayload::Bik(bik)) = state.document(doc).map(|d| d.payload.clone()) else {
        unreachable!("the document is a .bik");
    };
    let mut preview = state.video_preview(doc).expect("a .bik has a preview");
    preview.play(&bik.bytes);
    let deadline = Instant::now() + Duration::from_secs(30);
    while preview.position < 0.5 && preview.playing && Instant::now() < deadline {
        preview.tick(0.01);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        preview.position >= 0.5,
        "playing reached {}s: {:?}",
        preview.position,
        preview.problem()
    );
}

#[test]
fn a_shipped_bik_with_sound_hands_the_speaker_its_samples_and_pausing_silences_them() {
    const WHAT: &str =
        "a shipped .bik's sound decodes in OpenRage's own code and reaches the speaker";
    let Some(root) = game_roots().into_iter().find(|root| {
        root.join("GTA5.exe").is_file() && root.join(SHIPPED_BIK_WITH_SOUND.0).is_file()
    }) else {
        skip_install(
            WHAT,
            format_args!("no Legacy install root; set {GAME_ROOT_ENV}"),
        );
        return;
    };
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("news.bik");
    std::fs::write(&path, shipped_bik(&root, SHIPPED_BIK_WITH_SOUND))
        .expect("the movie is written");

    let mut state = AppState::default();
    let sink = NullSink::default();
    let heard = Arc::clone(&sink.heard);
    state.audio = AudioOut::new(Box::new(sink));
    let doc = open(&mut state, path);
    let Some(DocPayload::Bik(bik)) = state.document(doc).map(|d| d.payload.clone()) else {
        unreachable!("the document is a .bik");
    };
    let header = bik.header.clone().expect("OpenRage reads the header");
    assert_eq!(header.audio_tracks, 1);
    state
        .video_preview(doc)
        .expect("a .bik has a preview")
        .play(&bik.bytes);

    // The sound decodes on its own thread, so the start waits for it.
    let deadline = Instant::now() + Duration::from_secs(30);
    while heard.lock().expect("the sound heard").is_empty() && Instant::now() < deadline {
        state.sound_the_movie(doc);
        std::thread::sleep(Duration::from_millis(5));
    }
    let played = heard.lock().expect("the sound heard");
    let (rate, channels, samples) = played.first().expect("the speaker was given the sound");
    assert_eq!(*channels, 2, "the track is stereo");
    assert!(matches!(*rate, 22050 | 48000), "{rate} Hz");
    let seconds = samples.len() as f64 / (f64::from(*rate) * f64::from(*channels));
    let movie = f64::from(header.frames) / header.fps().expect("the header states a rate");
    assert!(
        (seconds - movie).abs() < 1.0,
        "{seconds} s of sound under {movie} s of picture"
    );
    assert!(
        samples.iter().any(|&s| s != 0),
        "decoded silence is not the sound"
    );
    drop(played);
    assert_eq!(state.audio.playing(), Some((doc, usize::MAX)));

    state
        .video_preview(doc)
        .expect("a .bik has a preview")
        .pause();
    state.sound_the_movie(doc);
    assert_eq!(state.audio.playing(), None, "pausing silences the movie");
}

/// A small `KB2g` movie of `frames` frames of 32 × 32 grey: a key frame that codes nothing, which
/// decodes to the first DC of a picture, then frames that repeat it. The packets are the hand-written
/// ones of `or_media`'s own decoder tests, in the container every shipped movie has: a frame starts
/// at an even byte, the low bit of its offset saying whether it is a key frame.
fn grey_bink2_movie(frames: u32) -> Vec<u8> {
    // The frame flags `0xE000`, then the bits of a block that codes nothing, or of a skip, then a pad.
    const KEY: [u8; 6] = [0x00, 0xE0, 0x00, 0x00, 0x53, 0x00];
    const SKIP: [u8; 6] = [0x00, 0xE0, 0x00, 0x00, 0x00, 0x00];
    let head = 44 + 4 * frames as usize;
    let mut movie = b"KB2g".to_vec();
    let fields = [
        (head + 6 * frames as usize - 8) as u32,
        frames,
        6,
        frames,
        32,
        32,
        30,
        1,
        0x10,
        0,
    ];
    for field in fields {
        movie.extend(field.to_le_bytes());
    }
    for frame in 0..frames {
        let key = u32::from(frame == 0);
        movie.extend(((head + 6 * frame as usize) as u32 | key).to_le_bytes());
    }
    for frame in 0..frames {
        movie.extend(if frame == 0 { KEY } else { SKIP });
    }
    movie
}

#[test]
fn a_bink2_header_opens_with_its_stated_fields_and_says_why_a_head_alone_cannot_play() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("movie.bik");
    std::fs::write(&path, bink::synthetic_bink2()).expect("the file is written");

    let mut state = AppState::default();
    let doc = open(&mut state, path);
    let Some(DocPayload::Bik(bik)) = state.document(doc).map(|d| d.payload.clone()) else {
        unreachable!("the document is a .bik");
    };
    let header = bik.header.expect("OpenRage reads the header");
    assert!(header.is_bink2());
    assert_eq!(
        (
            header.width,
            header.height,
            header.frames,
            header.audio_tracks
        ),
        (640, 360, 10, 1)
    );

    let preview = state.video_preview(doc).expect("a .bik has a preview");
    assert_eq!(
        preview.problem(),
        Some("this Bink movie is unreadable: its sound track table is cut short")
    );
    assert_eq!(preview.decoder(), None, "no decoder ran");
    assert!(preview.frame().is_none());
}

#[test]
fn a_bink2_movie_plays_to_its_end_through_or_media_whatever_the_platform() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("grey.bik");
    std::fs::write(&path, grey_bink2_movie(12)).expect("the file is written");

    let mut state = AppState::default();
    let doc = open(&mut state, path);
    let Some(DocPayload::Bik(bik)) = state.document(doc).map(|d| d.payload.clone()) else {
        unreachable!("the document is a .bik");
    };
    let mut preview = state.video_preview(doc).expect("a .bik has a preview");
    assert_eq!(preview.problem(), None);
    assert_eq!(preview.decoder(), Some(ROUTE));
    assert_eq!(preview.duration, Some(12.0 / 30.0));
    assert!(
        preview.wait_for_frame(Duration::from_secs(30)),
        "no frame arrived: {:?}",
        preview.problem()
    );
    let frame = preview.frame().expect("a frame is shown").to_vec();
    assert_eq!(frame.len(), (PREVIEW_WIDTH * PREVIEW_HEIGHT * 4) as usize);
    let pixel = |x: u32, y: u32| {
        let at = ((y * PREVIEW_WIDTH + x) * 4) as usize;
        [frame[at], frame[at + 1], frame[at + 2], frame[at + 3]]
    };
    // The picture is square, so it sits in the middle of the preview between bars of black, and
    // what it holds is the grey the key frame decodes to, full range.
    assert_eq!(
        pixel(PREVIEW_WIDTH / 2, PREVIEW_HEIGHT / 2),
        [128, 128, 128, 255]
    );
    assert_eq!(pixel(2, PREVIEW_HEIGHT / 2), [0, 0, 0, 255]);

    // A seek decodes from the key frame, and playing from the middle reaches the end.
    preview.seek(&bik.bytes, 6.0 / 30.0);
    preview.play(&bik.bytes);
    let deadline = Instant::now() + Duration::from_secs(30);
    while preview.playing && Instant::now() < deadline {
        preview.tick(1.0 / 30.0);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!preview.playing, "playing stops at the end of the movie");
    assert_eq!(preview.problem(), None, "an end is not a failure");
    assert!(
        (preview.position - 11.0 / 30.0).abs() < 1e-9,
        "it reached {}s",
        preview.position
    );
    assert_eq!(preview.sound_note(), "This movie has no sound.");
}

#[test]
fn a_shipped_bink2_plays_with_its_sound_through_or_media() {
    const WHAT: &str = "a shipped Bink 2 movie plays with its sound in OpenRage's own code";
    let Some(root) = game_roots().into_iter().find(|root| {
        root.join(SHIPPED_BINK2.0).is_file()
            && Game::detect_from_root(root)
                .and_then(|game| game.find_executable(root))
                .is_some()
    }) else {
        skip_install(WHAT, format_args!("no install root; set {GAME_ROOT_ENV}"));
        return;
    };
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = dir.path().join("rockstar_logos.bik");
    std::fs::write(&path, shipped_bik(&root, SHIPPED_BINK2)).expect("the movie is written");

    let mut state = AppState::default();
    let sink = NullSink::default();
    let heard = Arc::clone(&sink.heard);
    state.audio = AudioOut::new(Box::new(sink));
    let doc = open(&mut state, path);
    let Some(DocPayload::Bik(bik)) = state.document(doc).map(|d| d.payload.clone()) else {
        unreachable!("the document is a .bik");
    };
    let header = bik.header.clone().expect("OpenRage reads the header");
    assert!(header.is_bink2());
    assert_eq!(header.audio_tracks, 1);

    let mut preview = state.video_preview(doc).expect("a .bik has a preview");
    assert_eq!(preview.decoder(), Some(ROUTE), "{:?}", preview.problem());
    assert!(
        preview.wait_for_frame(Duration::from_secs(60)),
        "no frame arrived: {:?}",
        preview.problem()
    );
    let frame = preview.frame().expect("a frame is shown");
    assert_eq!(frame.len(), (PREVIEW_WIDTH * PREVIEW_HEIGHT * 4) as usize);
    assert!(frame.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 255));
    let end = preview.duration.expect("a Bink 2 movie states its length");
    drop(preview);

    // Played, its sound reaches the speaker as long as its picture, and it is not silence.
    state
        .video_preview(doc)
        .expect("a .bik has a preview")
        .play(&bik.bytes);
    let deadline = Instant::now() + Duration::from_secs(30);
    while heard.lock().expect("the sound heard").is_empty() && Instant::now() < deadline {
        state.sound_the_movie(doc);
        std::thread::sleep(Duration::from_millis(5));
    }
    {
        let played = heard.lock().expect("the sound heard");
        let (rate, channels, samples) = played.first().expect("the speaker was given the sound");
        assert_eq!(*channels, 2, "the track is stereo");
        let seconds = samples.len() as f64 / (f64::from(*rate) * f64::from(*channels));
        assert!(
            (seconds - end).abs() < 1.0,
            "{seconds} s of sound under {end} s of picture"
        );
        assert!(
            samples.iter().any(|&s| s != 0),
            "decoded silence is not the sound"
        );
    }

    let mut preview = state.video_preview(doc).expect("a .bik has a preview");
    preview.seek(&bik.bytes, end - 1.0);
    preview.play(&bik.bytes);
    let deadline = Instant::now() + Duration::from_secs(60);
    while preview.playing && Instant::now() < deadline {
        preview.tick(0.05);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!preview.playing, "playing stops at the end of the movie");
    assert!(
        preview.position > end - 0.1,
        "it reached {}s of {end}s",
        preview.position
    );
    assert_eq!(preview.problem(), None);
}

/// The most OpenRage's colour conversion may differ from the DLL's, on average a sample, in levels of
/// 255: the worst of the 3,524 shipped pictures compared measured 2.74 against Legacy's older DLL and
/// 0.84 against Enhanced's, though the planes it converts equal both DLLs' exactly.
#[cfg(windows)]
const COLOUR_TOLERANCE: f64 = 3.0;

/// Frames between the pictures whose colour is compared, so that a 4K movie is not converted twice
/// in every frame.
#[cfg(windows)]
const COLOUR_EVERY: usize = 50;

/// Movies compared at once, each by its own DLL handle and its own decoder.
#[cfg(windows)]
const WORKERS: usize = 4;

/// What comparing one movie with the DLL saw.
#[cfg(windows)]
#[derive(Debug, Default, Clone, Copy)]
struct Compared {
    frames: u64,
    colour_checked: u64,
    worst_colour: f64,
}

/// One movie decoded by the DLL and by OpenRage, frame by frame: every plane of every frame must be
/// the DLL's, byte for byte, the in-loop filter included.
#[cfg(windows)]
fn compare_with_dll(dll: &Path, bytes: Arc<[u8]>) -> Result<Compared, String> {
    let header = bink::Header::parse(&bytes).ok_or("no Bink header")?;
    let mut movie = bink::oracle::Movie::open(dll, Arc::clone(&bytes))?;
    let stated = (
        header.width,
        header.height,
        header.frames,
        (header.fps_num, header.fps_den),
    );
    let opened = (movie.width(), movie.height(), movie.frames(), movie.fps());
    if opened != stated {
        return Err(format!("stated {stated:?}, the DLL opened {opened:?}"));
    }
    let mut pictures = or_media::bik::Pictures::open(bytes).map_err(|why| why.to_string())?;
    let (w, h) = (header.width as usize, header.height as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let mut done = Compared::default();
    for frame in 0..pictures.frames() {
        movie.decode();
        let picture = pictures
            .next_picture()
            .map_err(|why| format!("frame {frame}: {why}"))?
            .ok_or_else(|| format!("frame {frame}: the movie ended"))?;
        // The DLL's planes are luma, red chroma and blue chroma; OpenRage's, luma, blue and red.
        for (name, ours, theirs, width, height) in [
            ("luma", picture.y, movie.plane(0), w, h),
            ("blue chroma", picture.u, movie.plane(2), cw, ch),
            ("red chroma", picture.v, movie.plane(1), cw, ch),
        ] {
            for y in 0..height {
                let (a, b) = (&ours.row(y)[..width], theirs.row(y, width));
                if let Some(x) = a.iter().zip(b).position(|(a, b)| a != b) {
                    return Err(format!(
                        "frame {frame}: the {name} differs from the DLL's at ({x}, {y}): {} against {}",
                        a[x], b[x]
                    ));
                }
            }
        }
        done.frames += 1;
        if frame % COLOUR_EVERY == 0 {
            let (theirs, ours) = (movie.rgba(), picture.to_rgba());
            let mean = ours
                .as_chunks::<4>()
                .0
                .iter()
                .zip(theirs.as_chunks::<4>().0.iter())
                .flat_map(|(a, b)| (0..3).map(move |c| f64::from(a[c].abs_diff(b[c]))))
                .sum::<f64>()
                / (w * h * 3) as f64;
            done.colour_checked += 1;
            done.worst_colour = done.worst_colour.max(mean);
            if mean > COLOUR_TOLERANCE {
                return Err(format!(
                    "frame {frame}: the colours differ from the DLL's by {mean:.2} levels on average"
                ));
            }
        }
        movie.next();
    }
    Ok(done)
}

/// Every Bink 2 movie each install ships, decoded by OpenRage and by that install's DLL.
#[cfg(windows)]
#[test]
fn every_shipped_bink2_matches_its_installs_dll_plane_for_plane() {
    use std::sync::mpsc::sync_channel;
    use std::sync::Mutex;

    use or_io::container::RpfCryptoProvider;
    use or_title::install::crawl::{self, ControlFlow};

    const WHAT: &str = "every shipped Bink 2 movie matches its install's DLL";
    let roots: Vec<PathBuf> = game_roots()
        .into_iter()
        .filter(|root| root.join(bink::oracle::DLL).is_file())
        .collect();
    if roots.is_empty() {
        skip_install(
            WHAT,
            format_args!(
                "no install root with {}; set {GAME_ROOT_ENV}",
                bink::oracle::DLL
            ),
        );
        return;
    }
    for root in &roots {
        let exe = Game::detect_from_root(root)
            .and_then(|game| game.find_executable(root))
            .expect("the install's executable");
        let keys = extract_keys(&exe, &|_| {}).expect("keys from the install's executable");
        let provider: Arc<dyn RpfCryptoProvider> =
            Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys)));
        let dll = root.join(bink::oracle::DLL);

        let (sender, receiver) = sync_channel::<(String, Arc<[u8]>)>(2);
        let receiver = Mutex::new(receiver);
        let results: Mutex<Vec<(String, Result<Compared, String>)>> = Mutex::default();
        let mut walked = (0usize, 0usize);
        let mut failures = Vec::new();
        let mut warnings = Vec::new();
        std::thread::scope(|scope| {
            for _ in 0..WORKERS {
                scope.spawn(|| loop {
                    let next = receiver.lock().expect("the queue").recv();
                    let Ok((name, bytes)) = next else { break };
                    let compared = compare_with_dll(&dll, bytes);
                    results.lock().expect("the results").push((name, compared));
                });
            }
            let _ = crawl::crawl(root, Arc::clone(&provider), &mut warnings, &mut |found| {
                if !found.leaf.to_ascii_lowercase().ends_with(".bik") {
                    return ControlFlow::Continue(());
                }
                let name = format!("{}::{}", found.container, found.path);
                let Ok(bytes) = found.read() else {
                    failures.push(format!("{name}: did not extract"));
                    return ControlFlow::Continue(());
                };
                match bink::Header::parse(&bytes) {
                    None => failures.push(format!("{name}: no Bink header")),
                    Some(header) if !header.is_bink2() => walked.0 += 1,
                    Some(_) => {
                        walked.1 += 1;
                        let _ = sender.send((name, bytes.into()));
                    }
                }
                ControlFlow::Continue(())
            });
            drop(sender);
        });

        let mut total = Compared::default();
        for (name, compared) in results.into_inner().expect("the results") {
            match compared {
                Err(why) => failures.push(format!("{name}: {why}")),
                Ok(done) => {
                    total.frames += done.frames;
                    total.colour_checked += done.colour_checked;
                    total.worst_colour = total.worst_colour.max(done.worst_colour);
                }
            }
        }
        assert!(warnings.is_empty(), "{}: {warnings:?}", root.display());
        assert!(failures.is_empty(), "{}: {failures:#?}", root.display());
        assert!(
            walked.1 > 0 && total.frames > 0 && total.colour_checked > 0,
            "{}: {} Bink 2 movies of {} frames, {} colours checked: this check exercised nothing",
            root.display(),
            walked.1,
            total.frames,
            total.colour_checked
        );
        tracing::info!(
            root = %root.display(),
            movies = walked.1,
            bink1 = walked.0,
            frames = total.frames,
            worst_colour = total.worst_colour,
            "every shipped Bink 2 movie compared"
        );
    }
}
