//! A `.bik` or `.bk2` preview: `or_media` opens the movie and decodes it through its table, which holds the
//! picture and sound of Bink 1 and Bink 2 in OpenRage's own Rust, on every platform. Nothing here
//! runs an external program or loads a library.

use std::sync::mpsc::{sync_channel, Receiver, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use or_media::{Decoders, Frame, MediaError, VideoStream};

use crate::bink;
use crate::state::{AppState, DocPayload, DocumentId};

pub const PREVIEW_WIDTH: u32 = 640;
pub const PREVIEW_HEIGHT: u32 = 360;
/// Frames per second a movie that states no rate plays at.
pub const PREVIEW_FPS: f64 = 30.0;
const FRAME_BYTES: usize = (PREVIEW_WIDTH * PREVIEW_HEIGHT * 4) as usize;

/// A Bink video document: the file's bytes, decoded afresh on every seek, and its header.
#[derive(Clone)]
pub struct BikDocument {
    pub bytes: Arc<[u8]>,
    pub header: Option<bink::Header>,
    pub preview: Arc<Mutex<Preview>>,
}

impl BikDocument {
    pub fn new(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.into(),
            header: bink::Header::parse(bytes),
            preview: Arc::default(),
        }
    }
}

impl AppState {
    /// The document's preview, started on first use.
    pub fn video_preview(&self, doc: DocumentId) -> Option<MutexGuard<'_, Preview>> {
        let DocPayload::Bik(bik) = &self.document(doc)?.payload else {
            return None;
        };
        let mut preview = bik.preview.lock().unwrap_or_else(PoisonError::into_inner);
        preview.prepare(&bik.bytes);
        Some(preview)
    }

    /// Gives the speaker what `doc`'s preview owes it: the movie's sound started where the picture
    /// stands, or stopped.
    pub fn sound_the_movie(&mut self, doc: DocumentId) {
        let owed = self
            .video_preview(doc)
            .map(|mut preview| (preview.take_cue(), preview.position));
        match owed {
            Some((Some(SoundCue::Play(sound)), position)) => {
                if let Err(why) = self.audio.play_movie(doc, &sound, position) {
                    if let Some(mut preview) = self.video_preview(doc) {
                        preview.set_sound_problem(why);
                    }
                }
            }
            Some((Some(SoundCue::Stop), _)) => self.audio.stop_movie(doc),
            _ => {}
        }
    }
}

/// What decodes a movie, as the preview names it.
pub const ROUTE: &str = "OpenRage's Bink decoder";

/// One decoding thread, from a start frame to the end of the movie.
struct Decoder {
    /// What decodes, as the preview names it.
    name: &'static str,
    frames: Receiver<Result<Vec<u8>, MediaError>>,
}

impl Decoder {
    fn start(name: &'static str, mut stream: Box<dyn VideoStream>) -> Self {
        let (sender, frames) = sync_channel(4);
        std::thread::spawn(move || loop {
            let sent = match stream.next_frame() {
                Ok(Some(frame)) => sender.send(Ok(sized(frame))).is_ok(),
                Ok(None) => false,
                Err(why) => {
                    let _ = sender.send(Err(why));
                    false
                }
            };
            if !sent {
                break;
            }
        });
        Self { name, frames }
    }
}

/// A frame as RGBA at the preview size, scaled only when it is not that size already.
fn sized(frame: Frame) -> Vec<u8> {
    if (frame.width, frame.height) == (PREVIEW_WIDTH, PREVIEW_HEIGHT) {
        frame.rgba
    } else {
        fit(&frame.rgba, frame.width, frame.height)
    }
}

/// An RGBA frame scaled to the preview size, keeping its aspect ratio, on opaque black.
// ponytail: nearest-neighbour, so a 4K movie aliases at 640×360; a box filter if that matters.
fn fit(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut out = [0, 0, 0, 255].repeat(FRAME_BYTES / 4);
    if width == 0 || height == 0 || rgba.len() < width as usize * height as usize * 4 {
        return out;
    }
    let scale = f64::min(
        f64::from(PREVIEW_WIDTH) / f64::from(width),
        f64::from(PREVIEW_HEIGHT) / f64::from(height),
    );
    let w = ((f64::from(width) * scale).round() as u32).clamp(1, PREVIEW_WIDTH);
    let h = ((f64::from(height) * scale).round() as u32).clamp(1, PREVIEW_HEIGHT);
    let (left, top) = ((PREVIEW_WIDTH - w) / 2, (PREVIEW_HEIGHT - h) / 2);
    for y in 0..h {
        let source_row = (y * height / h) * width;
        for x in 0..w {
            let s = ((source_row + x * width / w) * 4) as usize;
            let d = (((top + y) * PREVIEW_WIDTH + left + x) * 4) as usize;
            out[d..d + 4].copy_from_slice(&rgba[s..s + 4]);
        }
    }
    out
}

/// A movie's first sound track decoded whole: what the app hands the speaker.
pub struct Sound {
    pub rate: u32,
    pub channels: u16,
    /// Interleaved by `channels`.
    pub samples: Vec<i16>,
}

impl Sound {
    /// The samples from `seconds` in, as the speaker is given them.
    pub fn from(&self, seconds: f64) -> Vec<i16> {
        let frames = (seconds.max(0.0) * f64::from(self.rate)).round() as usize;
        let at = (frames * usize::from(self.channels)).min(self.samples.len());
        self.samples[at..].to_vec()
    }
}

/// What the preview asks of the speaker, which only the app can reach.
pub enum SoundCue {
    /// Start the sound at the movie's position now, replacing what plays.
    Play(Arc<Sound>),
    Stop,
}

/// The movie's sound, as far as it is decoded.
#[derive(Default)]
enum Track {
    /// No sound track, or none that decoded.
    #[default]
    None,
    Decoding(Receiver<Result<Sound, String>>),
    Ready(Arc<Sound>),
}

/// The first sound track of `bytes` decoded on a thread, or `None` when the movie has none.
fn decode_sound(bytes: &Arc<[u8]>) -> Option<Receiver<Result<Sound, String>>> {
    let source = or_media::open(Arc::clone(bytes)).ok()?;
    let audio = source.info().audio?;
    let (sender, receiver) = sync_channel(1);
    std::thread::spawn(move || {
        let samples = source
            .audio(&Decoders::new())
            .and_then(|mut stream| stream.read_all_i16())
            .map(|samples| Sound {
                rate: audio.sample_rate,
                channels: audio.channels,
                samples,
            });
        let _ = sender.send(samples.map_err(|why| why.to_string()));
    });
    Some(receiver)
}

/// The preview's playback: the frame shown, the clock, and the decoder feeding it.
#[derive(Default)]
pub struct Preview {
    /// Whether the preview has started: the sound is decoding and the first frame is asked for.
    prepared: bool,
    /// `or_media`'s table, which decodes the movie.
    decoders: Decoders,
    /// What opened this movie, once something did.
    route: Option<&'static str>,
    /// Frames per second of the frames the decoder hands over.
    fps: f64,
    decoder: Option<Decoder>,
    frame: Option<Vec<u8>>,
    frames_since_seek: u64,
    shown: u64,
    texture: Option<(u64, egui::TextureHandle)>,
    clock: f64,
    next_at: f64,
    /// Seconds into the movie of the frame shown.
    pub position: f64,
    pub duration: Option<f64>,
    pub playing: bool,
    problem: Option<String>,
    sound: Track,
    /// The speaker is owed a start (`true`) or a stop (`false`).
    cue: Option<bool>,
    /// Why the sound did not decode or play, if it did not.
    sound_problem: Option<String>,
}

impl Preview {
    fn prepare(&mut self, bytes: &Arc<[u8]>) {
        if self.prepared {
            return;
        }
        *self = Self {
            prepared: true,
            fps: PREVIEW_FPS,
            sound: decode_sound(bytes).map_or(Track::None, Track::Decoding),
            ..Self::default()
        };
        self.seek(bytes, 0.0);
    }

    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// Why the movie's sound is not heard, if the movie has sound and it is not.
    pub fn sound_problem(&self) -> Option<&str> {
        self.sound_problem.as_deref()
    }

    pub fn set_sound_problem(&mut self, why: String) {
        self.sound_problem = Some(why);
    }

    #[cfg(test)]
    pub fn frame(&self) -> Option<&[u8]> {
        self.frame.as_deref()
    }

    /// What decodes this movie, once one was found.
    #[cfg(test)]
    pub fn decoder(&self) -> Option<&'static str> {
        self.route
    }

    /// Restart decoding at `seconds`; the frame there shows once it is decoded.
    pub fn seek(&mut self, bytes: &Arc<[u8]>, seconds: f64) {
        self.decoder = None;
        let opened = or_media::open(Arc::clone(bytes)).and_then(|source| {
            let info = *source.info();
            source.video(&self.decoders).map(|stream| (info, stream))
        });
        let (info, mut stream) = match opened {
            Ok(opened) => opened,
            Err(why) => {
                self.problem = Some(why.to_string());
                return;
            }
        };
        self.fps = match stream.frame_rate() {
            (numerator @ 1.., denominator @ 1..) => f64::from(numerator) / f64::from(denominator),
            _ => PREVIEW_FPS,
        };
        let mut first = (seconds * self.fps).round() as u64;
        if let Some(last) = info.video.and_then(|video| video.frames.checked_sub(1)) {
            first = first.min(last);
        }
        let seconds = first as f64 / self.fps;
        if first > 0 {
            if let Err(why) = stream.seek(first) {
                self.problem = Some(why.to_string());
                return;
            }
        }
        self.route = Some(ROUTE);
        self.decoder = Some(Decoder::start(ROUTE, stream));
        self.duration = self
            .duration
            .or_else(|| info.duration().map(|d| d.as_secs_f64()));
        self.problem = None;
        self.frames_since_seek = 0;
        self.position = seconds;
        self.clock = seconds;
        self.next_at = seconds;
        // A seek while playing moves the sound with the picture.
        if self.playing {
            self.cue = Some(true);
        }
    }

    /// Play from where the preview stands, or from the start once the movie has ended.
    pub fn play(&mut self, bytes: &Arc<[u8]>) {
        if self.decoder.is_none() {
            self.seek(bytes, 0.0);
        }
        self.playing = self.decoder.is_some();
        if self.playing {
            self.cue = Some(true);
        }
    }

    pub fn pause(&mut self) {
        self.playing = false;
        self.cue = Some(false);
    }

    /// Advance the clock by `dt` seconds while playing, and show each decoded frame now due.
    pub fn tick(&mut self, dt: f64) {
        if self.playing {
            self.clock += dt;
        }
        while self.next_at <= self.clock {
            let Some(decoder) = &self.decoder else {
                break;
            };
            match decoder.frames.try_recv() {
                Ok(Ok(frame)) => {
                    self.frame = Some(frame);
                    self.shown += 1;
                    self.frames_since_seek += 1;
                    self.position = self.next_at;
                    self.next_at += 1.0 / self.fps;
                }
                Ok(Err(why)) => {
                    self.problem = Some(why.to_string());
                    self.playing = false;
                    self.decoder = None;
                    break;
                }
                // The decoder is behind: hold the clock rather than skip frames.
                Err(TryRecvError::Empty) => {
                    self.clock = self.next_at;
                    break;
                }
                Err(TryRecvError::Disconnected) => {
                    if self.frames_since_seek == 0 {
                        self.problem = Some(format!("{} decoded no frame", decoder.name));
                    }
                    self.playing = false;
                    self.decoder = None;
                    break;
                }
            }
        }
    }

    /// Whether the decoder is still decoding, or the sound still decoding for a start, so the UI
    /// keeps asking for frames.
    pub fn busy(&self) -> bool {
        let sound_due = self.cue == Some(true) && matches!(self.sound, Track::Decoding(_));
        sound_due || self.decoder.is_some() && (self.playing || self.frames_since_seek == 0)
    }

    /// The sound, once its decoding thread has finished with it.
    fn sound(&mut self) -> Option<Arc<Sound>> {
        if let Track::Decoding(receiver) = &self.sound {
            match receiver.try_recv() {
                Ok(Ok(sound)) => self.sound = Track::Ready(Arc::new(sound)),
                Ok(Err(why)) => {
                    self.sound_problem = Some(why);
                    self.sound = Track::None;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.sound = Track::None,
            }
        }
        match &self.sound {
            Track::Ready(sound) => Some(Arc::clone(sound)),
            _ => None,
        }
    }

    /// What the speaker is owed, taken once; a start waits for the sound to finish decoding.
    pub fn take_cue(&mut self) -> Option<SoundCue> {
        if self.cue? {
            let sound = self.sound();
            if sound.is_some() || !matches!(self.sound, Track::Decoding(_)) {
                self.cue = None;
            }
            sound.map(SoundCue::Play)
        } else {
            self.cue = None;
            Some(SoundCue::Stop)
        }
    }

    /// The line under the controls that says what the movie's sound is.
    pub fn sound_note(&self) -> String {
        match &self.sound {
            Track::Ready(sound) => {
                let channels = if sound.channels == 1 {
                    "mono"
                } else {
                    "stereo"
                };
                format!("Sound: {channels}, {} Hz.", sound.rate)
            }
            Track::Decoding(_) => "Decoding the sound…".to_string(),
            Track::None if self.sound_problem.is_some() => "The sound did not decode.".to_string(),
            Track::None => "This movie has no sound.".to_string(),
        }
    }

    /// The shown frame as a texture, uploaded again only when the frame changed.
    pub fn texture(&mut self, ctx: &egui::Context) -> Option<&egui::TextureHandle> {
        let frame = self.frame.as_deref()?;
        let size = [PREVIEW_WIDTH as usize, PREVIEW_HEIGHT as usize];
        let image = || egui::ColorImage::from_rgba_unmultiplied(size, frame);
        match &mut self.texture {
            Some((shown, _)) if *shown == self.shown => {}
            Some((shown, handle)) => {
                handle.set(image(), egui::TextureOptions::LINEAR);
                *shown = self.shown;
            }
            None => {
                let handle = ctx.load_texture("bik_preview", image(), egui::TextureOptions::LINEAR);
                self.texture = Some((self.shown, handle));
            }
        }
        self.texture.as_ref().map(|(_, handle)| handle)
    }

    #[cfg(test)]
    pub fn wait_for_frame(&mut self, timeout: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        while self.frames_since_seek == 0
            && self.problem.is_none()
            && std::time::Instant::now() < deadline
        {
            self.tick(0.0);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        self.frames_since_seek > 0
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::sync_channel;
    use std::sync::Arc;
    use std::time::Duration;

    use or_media::{Codec, Decoders, Frame, MediaError, VideoStream};

    use super::{
        Preview, Sound, SoundCue, Track, FRAME_BYTES, PREVIEW_FPS, PREVIEW_HEIGHT, PREVIEW_WIDTH,
        ROUTE,
    };

    /// A `KB2g` head stating `frames` frames of 640×360 at 30 a second, which is all `or_media`
    /// reads of a movie before a decoder gets it, and which no real decoder can decode.
    fn bink2_head(frames: u32) -> Arc<[u8]> {
        let mut head = vec![0u8; 44];
        head[..4].copy_from_slice(b"KB2g");
        for (at, value) in [(8, frames), (20, 640), (24, 360), (28, 30), (32, 1)] {
            head[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        head.into()
    }

    /// A decoder of frames each filled with its own number, which fails at `fail_at` when told to.
    struct Shades {
        next: u8,
        count: u8,
        fail_at: Option<u8>,
    }

    impl VideoStream for Shades {
        fn next_frame(&mut self) -> Result<Option<Frame>, MediaError> {
            if self.fail_at == Some(self.next) {
                return Err(MediaError::Decode("the decoder stopped".to_string()));
            }
            if self.next >= self.count {
                return Ok(None);
            }
            self.next += 1;
            Ok(Some(Frame {
                width: PREVIEW_WIDTH,
                height: PREVIEW_HEIGHT,
                rgba: vec![self.next - 1; FRAME_BYTES],
            }))
        }

        fn frame_rate(&self) -> (u32, u32) {
            (30, 1)
        }

        fn seek(&mut self, frame: u64) -> Result<(), MediaError> {
            self.next = u8::try_from(frame).unwrap_or(u8::MAX);
            Ok(())
        }
    }

    /// A preview whose only row is a Bink 2 one of `Shades` of `count` frames, so nothing here
    /// decodes a real picture.
    fn preview_of_shades(count: u8, fail_at: Option<u8>) -> Preview {
        Preview {
            decoders: Decoders::empty().with(
                Codec::Bink2,
                or_media::Decoder::video(move |_, _| {
                    Ok(Box::new(Shades {
                        next: 0,
                        count,
                        fail_at,
                    }))
                }),
            ),
            fps: PREVIEW_FPS,
            ..Preview::default()
        }
    }

    #[test]
    fn a_preview_plays_a_movie_to_its_end_through_whatever_row_decodes_it() {
        let movie = bink2_head(6);
        let mut preview = preview_of_shades(6, None);
        preview.seek(&movie, 0.0);
        assert_eq!(preview.problem(), None);
        assert_eq!(
            preview.duration,
            Some(6.0 / 30.0),
            "the header's frames over its rate"
        );
        assert!(preview.wait_for_frame(Duration::from_secs(10)));

        preview.play(&movie);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while preview.playing && std::time::Instant::now() < deadline {
            preview.tick(1.0 / 30.0);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!preview.playing, "playing stops when the movie ends");
        assert_eq!(preview.problem(), None, "an end is not a failure");
        assert_eq!(
            preview.frame().map(|f| f[0]),
            Some(5),
            "the last frame stays up"
        );
        assert!(
            (preview.position - 5.0 / 30.0).abs() < 1e-9,
            "{}",
            preview.position
        );
    }

    #[test]
    fn a_seek_starts_the_stream_at_the_frame_the_time_names() {
        let movie = bink2_head(8);
        let mut preview = preview_of_shades(8, None);
        preview.seek(&movie, 4.0 / 30.0);
        assert!(preview.wait_for_frame(Duration::from_secs(10)));
        assert_eq!(preview.frame().map(|f| f[0]), Some(4));
        assert_eq!(preview.position, 4.0 / 30.0);
    }

    #[test]
    fn a_seek_at_the_end_shows_the_last_frame() {
        let movie = bink2_head(8);
        let mut preview = preview_of_shades(8, None);
        preview.seek(&movie, 8.0 / 30.0);
        assert!(
            preview.wait_for_frame(Duration::from_secs(10)),
            "{:?}",
            preview.problem()
        );
        assert_eq!(preview.frame().map(|f| f[0]), Some(7));
        assert_eq!(preview.position, 7.0 / 30.0);
        assert_eq!(preview.problem(), None);
    }

    #[test]
    fn a_stream_that_fails_is_reported_with_its_own_words() {
        let movie = bink2_head(8);
        let mut preview = preview_of_shades(8, Some(0));
        preview.seek(&movie, 0.0);
        assert!(!preview.wait_for_frame(Duration::from_secs(10)));
        assert_eq!(preview.problem(), Some("the decoder stopped"));
        assert!(!preview.busy(), "a failed decoder is not waited for");

        // A decoder that ends before its first frame says which one it was.
        let mut empty = preview_of_shades(0, None);
        empty.seek(&movie, 0.0);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while empty.problem().is_none() && std::time::Instant::now() < deadline {
            empty.tick(0.0);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            empty.problem(),
            Some(format!("{ROUTE} decoded no frame").as_str())
        );
    }

    #[test]
    fn a_file_that_is_not_bink_is_refused_before_any_decoder_is_asked() {
        let mut preview = preview_of_shades(1, None);
        preview.seek(&Arc::from(&b"RIFF"[..]), 0.0);
        assert!(
            preview.problem().is_some(),
            "the preview says it cannot open the file"
        );
        assert!(preview.decoder().is_none());
    }

    #[test]
    fn a_bink1_movie_that_does_not_read_is_named_by_its_decoder_and_asks_for_no_install() {
        let mut head = vec![0u8; 44];
        head[..4].copy_from_slice(b"BIKi");
        for (at, value) in [(8, 6u32), (20, 640), (24, 360), (28, 30), (32, 1)] {
            head[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        let mut preview = Preview {
            decoders: Decoders::new(),
            fps: PREVIEW_FPS,
            ..Preview::default()
        };
        preview.seek(&head.into(), 0.0);
        let problem = preview
            .problem()
            .expect("a head with no frames cannot play");
        assert_eq!(
            problem,
            "this Bink movie is unreadable: its frame index is cut short"
        );
        assert!(!problem.to_lowercase().contains("ffmpeg"), "{problem}");
        assert!(preview.decoder().is_none());
    }

    /// A preview over eight frames whose sound is `track`.
    fn preview_with_sound(track: Track) -> (Preview, Arc<[u8]>) {
        let movie = bink2_head(8);
        let mut preview = preview_of_shades(8, None);
        preview.sound = track;
        preview.seek(&movie, 0.0);
        (preview, movie)
    }

    fn two_seconds_of_stereo_at_4_hz() -> Sound {
        Sound {
            rate: 4,
            channels: 2,
            samples: (0..16).collect(),
        }
    }

    #[test]
    fn playing_cues_the_sound_from_where_the_preview_stands_and_pausing_stops_it() {
        let (mut preview, movie) =
            preview_with_sound(Track::Ready(Arc::new(two_seconds_of_stereo_at_4_hz())));
        assert!(
            preview.take_cue().is_none(),
            "nothing is owed before it plays"
        );

        preview.play(&movie);
        let Some(SoundCue::Play(sound)) = preview.take_cue() else {
            panic!("playing owes the speaker a start");
        };
        assert_eq!(
            sound.from(1.5),
            (12..16).collect::<Vec<i16>>(),
            "frame 6 of 4 a second"
        );
        assert!(preview.take_cue().is_none(), "a cue is taken once");

        preview.pause();
        assert!(!preview.playing);
        assert!(matches!(preview.take_cue(), Some(SoundCue::Stop)));

        // A seek while playing moves the sound with the picture; one while paused leaves it be.
        preview.seek(&movie, 4.0 / 30.0);
        assert!(preview.take_cue().is_none());
        preview.play(&movie);
        let _ = preview.take_cue();
        preview.seek(&movie, 2.0 / 30.0);
        assert!(matches!(preview.take_cue(), Some(SoundCue::Play(_))));
    }

    #[test]
    fn a_start_waits_for_the_sound_to_decode_and_a_movie_with_none_owes_the_speaker_nothing() {
        let (sender, receiver) = sync_channel(1);
        let (mut preview, movie) = preview_with_sound(Track::Decoding(receiver));
        preview.play(&movie);
        assert!(preview.take_cue().is_none(), "the sound is not decoded yet");
        assert!(preview.busy(), "the UI keeps asking while a start waits");
        sender
            .send(Ok(two_seconds_of_stereo_at_4_hz()))
            .expect("the preview listens");
        assert!(matches!(preview.take_cue(), Some(SoundCue::Play(_))));
        assert_eq!(preview.sound_note(), "Sound: stereo, 4 Hz.");

        let (mut silent, movie) = preview_with_sound(Track::None);
        silent.play(&movie);
        assert!(silent.take_cue().is_none());
        assert!(silent.take_cue().is_none(), "and it does not keep waiting");
        assert_eq!(silent.sound_note(), "This movie has no sound.");

        let (sender, receiver) = sync_channel(1);
        let (mut broken, movie) = preview_with_sound(Track::Decoding(receiver));
        broken.play(&movie);
        sender
            .send(Err("a block is cut short".to_string()))
            .expect("the preview listens");
        assert!(broken.take_cue().is_none());
        assert_eq!(broken.sound_problem(), Some("a block is cut short"));
    }

    #[test]
    fn a_sound_starts_at_the_frame_a_time_names_and_never_past_its_end() {
        let sound = two_seconds_of_stereo_at_4_hz();
        assert_eq!(sound.from(0.0), (0..16).collect::<Vec<i16>>());
        assert_eq!(sound.from(0.5), (4..16).collect::<Vec<i16>>());
        assert!(sound.from(99.0).is_empty());
        assert_eq!(sound.from(-1.0).len(), 16, "before the start is the start");
    }

    #[test]
    fn a_frame_at_the_preview_size_is_handed_on_as_it_is() {
        let rgba = [7u8; super::FRAME_BYTES];
        let frame = or_media::Frame {
            width: super::PREVIEW_WIDTH,
            height: super::PREVIEW_HEIGHT,
            rgba: rgba.to_vec(),
        };
        assert_eq!(super::sized(frame), rgba);
        let small = or_media::Frame {
            width: 1,
            height: 1,
            rgba: vec![9, 9, 9, 9],
        };
        let out = super::sized(small);
        assert_eq!(out.len(), super::FRAME_BYTES);
        assert_eq!(&out[..4], [0, 0, 0, 255], "letterboxed, not stretched");
    }

    #[test]
    fn a_frame_fits_the_preview_letterboxed_on_opaque_black() {
        let red_blue = [255, 0, 0, 255, 0, 0, 255, 255];
        let out = super::fit(&red_blue, 2, 1);
        let pixel = |x: u32, y: u32| {
            let at = ((y * super::PREVIEW_WIDTH + x) * 4) as usize;
            &out[at..at + 4]
        };
        assert_eq!(pixel(0, 0), [0, 0, 0, 255], "the bar above the frame");
        assert_eq!(pixel(0, 20), [255, 0, 0, 255]);
        assert_eq!(pixel(639, 20), [0, 0, 255, 255]);
        assert_eq!(pixel(639, 339), [0, 0, 255, 255]);
        assert_eq!(pixel(639, 340), [0, 0, 0, 255], "the bar below the frame");
    }
}
