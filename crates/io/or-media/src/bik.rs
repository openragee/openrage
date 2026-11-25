//! A Bink movie as a source: the fields at the head of the file, its frame index and its sound
//! tracks, with the picture and the first sound track opened through the table's rows.
//!
//! Origin of the layout it reads: FFmpeg n7.0 (commit 083443d67cb159ce469e5d902346b8d0c2cd1c93),
//! libavformat, the file bink.c, Copyright (c) 2008-2010 Peter Ross, LGPL-2.1-or-later: the field
//! offsets, the frame index and the packets of a frame. Listed in LICENSE-third-party.md.

use std::sync::Arc;

use crate::bink::{audio, video};
use crate::bink2;
use crate::{
    AudioInfo, AudioStream, Codec, Decoders, Frame, MediaError, MediaInfo, Source, VideoInfo,
    VideoStream, Yuv420,
};

/// Bytes whose fields are the same in `BIKi`, `KB2g` and `KB2j`.
const HEAD_BYTES: usize = 44;

/// The most frames and sound tracks a movie may state.
const MAX_FRAMES: usize = 1_000_000;
const MAX_TRACKS: usize = 256;

/// A track flag: the stream is stereo, and the stream is coded with the DCT rather than the RDFT.
const AUDIO_STEREO: u16 = 0x2000;
const AUDIO_DCT: u16 = 0x1000;

/// Whether the bytes start as a Bink 1 (`BIK`) or Bink 2 (`KB2`) movie.
pub(crate) fn is_bink(bytes: &[u8]) -> bool {
    bytes.starts_with(b"BIK") || bytes.starts_with(b"KB2")
}

/// A sound track as the header states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Track {
    sample_rate: u32,
    channels: u16,
    dct: bool,
    /// Whether the track is a `KB2n` movie's DCT one, whose blocks are coded another way.
    revised: bool,
}

impl Track {
    fn codec(self) -> Codec {
        if self.revised {
            Codec::Bink2Audio
        } else if self.dct {
            Codec::BinkAudioDct
        } else {
            Codec::BinkAudioRdft
        }
    }
}

/// Where a frame's packets lie in the file, and whether it needs no frame before it.
#[derive(Debug, Clone, Copy)]
struct Span {
    start: usize,
    end: usize,
    key: bool,
}

/// A movie's header and frame index, read once.
struct Layout {
    revision: u8,
    video_flags: u32,
    width: u32,
    height: u32,
    tracks: Vec<Track>,
    index: Vec<Span>,
}

/// What a frame holds: a packet per sound track, empty where the track has none, and the picture's.
struct Parts<'a> {
    audio: Vec<&'a [u8]>,
    video: &'a [u8],
}

fn bad(what: impl std::fmt::Display) -> MediaError {
    MediaError::Container(format!("this Bink movie is unreadable: {what}"))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

impl Layout {
    fn parse(bytes: &[u8]) -> Result<Self, MediaError> {
        if bytes.len() < HEAD_BYTES {
            return Err(bad("its header is cut short"));
        }
        let field = |at: usize| u32_at(bytes, at).unwrap_or(0);
        let revision = bytes[3];
        let file_size = field(4) as usize + 8;
        let frames = field(8) as usize;
        if frames > MAX_FRAMES {
            return Err(bad("it states more than 1,000,000 frames"));
        }
        if field(12) as usize > file_size {
            return Err(bad("its largest frame is bigger than the file"));
        }
        let tracks_stated = field(40) as usize;
        if tracks_stated > MAX_TRACKS {
            return Err(bad("it states more than 256 sound tracks"));
        }
        let new_field = (bytes.starts_with(b"BIK") && revision == b'k')
            || (bytes.starts_with(b"KB2") && matches!(revision, b'i' | b'j' | b'k' | b'n'));
        let mut at = HEAD_BYTES + if new_field { 4 } else { 0 };
        let mut tracks = Vec::with_capacity(tracks_stated);
        if tracks_stated > 0 {
            at += 4 * tracks_stated;
            for _ in 0..tracks_stated {
                let head = bytes
                    .get(at..at + 4)
                    .ok_or_else(|| bad("its sound track table is cut short"))?;
                let flags = u16::from_le_bytes([head[2], head[3]]);
                let dct = flags & AUDIO_DCT != 0;
                tracks.push(Track {
                    sample_rate: u32::from(u16::from_le_bytes([head[0], head[1]])),
                    channels: if flags & AUDIO_STEREO != 0 { 2 } else { 1 },
                    dct,
                    revised: dct && bytes.starts_with(b"KB2") && revision == b'n',
                });
                at += 4;
            }
            at += 4 * tracks_stated;
        }

        let mut index = Vec::with_capacity(frames);
        let mut next = u32_at(bytes, at).ok_or_else(|| bad("its frame index is cut short"))?;
        for i in 0..frames {
            let raw = next;
            next = if i + 1 == frames {
                file_size as u32
            } else {
                u32_at(bytes, at + 4 * (i + 1))
                    .ok_or_else(|| bad("its frame index is cut short"))?
            };
            let (start, end) = ((raw & !1) as usize, (next & !1) as usize);
            if end <= start {
                return Err(bad("its frame index does not rise"));
            }
            if end > bytes.len() {
                return Err(bad(format!("frame {i} lies beyond the end of the file")));
            }
            index.push(Span {
                start,
                end,
                key: i == 0 || raw & 1 != 0,
            });
        }
        Ok(Self {
            revision,
            video_flags: field(36),
            width: field(20),
            height: field(24),
            tracks,
            index,
        })
    }

    fn parts<'a>(&self, bytes: &'a [u8], frame: usize) -> Result<Parts<'a>, MediaError> {
        let span = self.index[frame];
        let mut rest = &bytes[span.start..span.end];
        let mut audio = Vec::with_capacity(self.tracks.len());
        for _ in &self.tracks {
            let size =
                u32_at(rest, 0).ok_or_else(|| bad(format!("frame {frame} is cut short")))? as usize;
            if size > rest.len() - 4 {
                return Err(bad(format!(
                    "frame {frame}: a sound packet of {size} bytes is bigger than what is left of it"
                )));
            }
            audio.push(if size >= 4 {
                &rest[4..4 + size]
            } else {
                &[][..]
            });
            rest = &rest[4 + size..];
        }
        Ok(Parts { audio, video: rest })
    }

    fn audio_info(&self, bytes: &[u8], track: usize) -> AudioInfo {
        let t = self.tracks[track];
        AudioInfo {
            codec: t.codec(),
            channels: t.channels,
            sample_rate: t.sample_rate,
            frames: self.audio_frames(bytes, track),
        }
    }

    /// How many sample frames track `track` decodes to, by what its packets say.
    fn audio_frames(&self, bytes: &[u8], track: usize) -> u64 {
        let channels = u64::from(self.tracks[track].channels);
        (0..self.index.len())
            .filter_map(|frame| self.parts(bytes, frame).ok())
            .filter_map(|parts| u32_at(parts.audio[track], 0))
            .map(|decoded| u64::from(decoded) / (2 * channels))
            .sum()
    }
}

struct Movie {
    info: MediaInfo,
    bytes: Arc<[u8]>,
    layout: Option<Arc<Layout>>,
}

impl Source for Movie {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn video(&self, decoders: &Decoders) -> Result<Box<dyn VideoStream>, MediaError> {
        let video = self.info.video.ok_or(MediaError::NoVideo)?;
        let open = decoders.video_decoder(video.codec)?;
        open(&video, Arc::clone(&self.bytes))
    }

    fn audio(&self, decoders: &Decoders) -> Result<Box<dyn AudioStream>, MediaError> {
        let audio = self.info.audio.ok_or(MediaError::NoAudio)?;
        let layout = self.layout.as_ref().ok_or(MediaError::NoAudio)?;
        let open = decoders.audio_decoder(audio.codec)?;
        open(&audio, &track_packets(layout, &self.bytes, 0))
    }
}

/// A track's packets, each a little-endian `u32` length and then its bytes: what an audio row is given.
fn track_packets(layout: &Layout, bytes: &[u8], track: usize) -> Vec<u8> {
    let mut packets = Vec::new();
    for frame in 0..layout.index.len() {
        let Ok(parts) = layout.parts(bytes, frame) else {
            break;
        };
        let packet = parts.audio[track];
        if !packet.is_empty() {
            packets.extend((packet.len() as u32).to_le_bytes());
            packets.extend_from_slice(packet);
        }
    }
    packets
}

/// A movie, whose signature picks the codec.
///
/// ⚠ A head shorter than its 44 bytes states nothing: its fields read as zero, and the decoder
/// the table holds says what is wrong with the file. The same goes for a file whose frame index
/// or sound tracks do not read: it opens with its head's fields and no sound.
pub fn open(bytes: Arc<[u8]>) -> Result<Box<dyn Source>, MediaError> {
    let codec = if bytes.starts_with(b"BIK") {
        Codec::Bink1
    } else if bytes.starts_with(b"KB2") {
        Codec::Bink2
    } else {
        return Err(MediaError::Container(
            "this is not a Bink movie".to_string(),
        ));
    };
    let field = |at: usize| {
        u32_at(&bytes, at)
            .filter(|_| bytes.len() >= HEAD_BYTES)
            .unwrap_or(0)
    };
    let video = VideoInfo {
        codec,
        width: field(20),
        height: field(24),
        frame_rate: (field(28), field(32)),
        frames: u64::from(field(8)),
    };
    let layout = Layout::parse(&bytes).ok().map(Arc::new);
    let audio = layout
        .as_ref()
        .filter(|layout| !layout.tracks.is_empty())
        .map(|layout| layout.audio_info(&bytes, 0));
    Ok(Box::new(Movie {
        info: MediaInfo {
            audio,
            video: Some(video),
        },
        bytes,
        layout,
    }))
}

/// The picture decoder of a movie: the `BIK` or the `KB2` one, as its signature says.
enum PictureDecoder {
    One(Box<video::Decoder>),
    Two(Box<bink2::video::Decoder>),
}

impl PictureDecoder {
    fn decode(&mut self, packet: &[u8], key: bool) -> Result<Yuv420<'_>, MediaError> {
        match self {
            Self::One(decoder) => decoder.decode(packet),
            Self::Two(decoder) => decoder.decode(packet, key),
        }
    }

    fn reset(&mut self) {
        match self {
            Self::One(decoder) => decoder.reset(),
            Self::Two(decoder) => decoder.reset(),
        }
    }
}

/// A Bink movie's pictures before they become RGBA: planar 4:2:0, a frame at a time.
pub struct Pictures {
    layout: Layout,
    bytes: Arc<[u8]>,
    decoder: PictureDecoder,
    next: usize,
}

impl Pictures {
    pub fn open(bytes: Arc<[u8]>) -> Result<Self, MediaError> {
        let layout = Layout::parse(&bytes)?;
        let (revision, flags) = (layout.revision, layout.video_flags);
        let decoder = if bytes.starts_with(b"KB2") {
            PictureDecoder::Two(Box::new(bink2::video::Decoder::new(
                revision,
                flags,
                layout.width,
                layout.height,
            )?))
        } else {
            PictureDecoder::One(Box::new(video::Decoder::new(
                revision,
                flags,
                layout.width,
                layout.height,
            )?))
        };
        Ok(Self {
            layout,
            bytes,
            decoder,
            next: 0,
        })
    }

    /// Frames the movie holds.
    pub fn frames(&self) -> usize {
        self.layout.index.len()
    }

    /// The next picture, or `None` after the last; it lives until the next call.
    pub fn next_picture(&mut self) -> Result<Option<Yuv420<'_>>, MediaError> {
        if self.next >= self.frames() {
            return Ok(None);
        }
        let parts = self.layout.parts(&self.bytes, self.next)?;
        let key = self.layout.index[self.next].key;
        self.next += 1;
        self.decoder.decode(parts.video, key).map(Some)
    }

    /// Makes `frame` the next picture, decoding from the key frame before it.
    pub fn seek(&mut self, frame: u64) -> Result<(), MediaError> {
        let frames = self.frames();
        if frames == 0 {
            return Ok(());
        }
        let target = usize::try_from(frame).unwrap_or(usize::MAX).min(frames);
        let key = (0..=target.min(frames - 1))
            .rev()
            .find(|&f| self.layout.index[f].key)
            .unwrap_or(0);
        self.decoder.reset();
        self.next = key;
        while self.next < target {
            let parts = self.layout.parts(&self.bytes, self.next)?;
            self.decoder
                .decode(parts.video, self.layout.index[self.next].key)?;
            self.next += 1;
        }
        Ok(())
    }
}

/// A Bink movie's picture as RGBA frames.
struct BinkVideo {
    pictures: Pictures,
    frame_rate: (u32, u32),
}

impl VideoStream for BinkVideo {
    fn next_frame(&mut self) -> Result<Option<Frame>, MediaError> {
        Ok(self.pictures.next_picture()?.map(|picture| Frame {
            width: picture.width as u32,
            height: picture.height as u32,
            rgba: picture.to_rgba(),
        }))
    }

    fn frame_rate(&self) -> (u32, u32) {
        self.frame_rate
    }

    fn seek(&mut self, frame: u64) -> Result<(), MediaError> {
        self.pictures.seek(frame)
    }
}

/// The table's Bink 1 and Bink 2 rows: a movie's picture, decoded in OpenRage's own code.
pub(crate) fn open_video(
    info: &VideoInfo,
    bytes: Arc<[u8]>,
) -> Result<Box<dyn VideoStream>, MediaError> {
    Ok(Box::new(BinkVideo {
        pictures: Pictures::open(bytes)?,
        frame_rate: info.frame_rate,
    }))
}

/// The sound tracks a Bink movie's header states.
pub fn audio_tracks(bytes: &[u8]) -> Result<Vec<AudioInfo>, MediaError> {
    let layout = Layout::parse(bytes)?;
    Ok((0..layout.tracks.len())
        .map(|i| layout.audio_info(bytes, i))
        .collect())
}

/// Sound track `index` of a Bink movie, decoded through the row `decoders` holds for its codec.
pub fn audio_track(
    bytes: &[u8],
    index: usize,
    decoders: &Decoders,
) -> Result<Box<dyn AudioStream>, MediaError> {
    let layout = Layout::parse(bytes)?;
    if index >= layout.tracks.len() {
        return Err(MediaError::NoAudio);
    }
    let info = layout.audio_info(bytes, index);
    decoders.audio_decoder(info.codec)?(&info, &track_packets(&layout, bytes, index))
}

/// The table's Bink audio rows: a track's framed packets, decoded in OpenRage's own code.
pub(crate) fn open_audio(
    info: &AudioInfo,
    packets: &[u8],
) -> Result<Box<dyn AudioStream>, MediaError> {
    let dct = info.codec != Codec::BinkAudioRdft;
    let mut decoder = audio::Decoder::new(info.sample_rate, info.channels, dct)?;
    if info.codec == Codec::Bink2Audio {
        decoder = decoder.revised();
    }
    Ok(Box::new(audio::Stream::new(decoder, packets.to_vec())))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::Decoder;

    /// The 44-byte head both revisions share, with the fields at the offsets the format fixes.
    fn head(signature: &[u8; 4]) -> Vec<u8> {
        let mut head = vec![0u8; HEAD_BYTES];
        head[..4].copy_from_slice(signature);
        for (at, value) in [(8, 90u32), (20, 640), (24, 360), (28, 30), (32, 1), (40, 1)] {
            head[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        head
    }

    /// What a test frame holds: a packet per track, empty where the track has none, and the picture.
    type Held = (Vec<Vec<u8>>, Vec<u8>);

    /// A whole small movie with a track per entry of `tracks` (a rate and flags).
    fn movie(signature: &[u8; 4], tracks: &[(u16, u16)], frames: &[Held]) -> Vec<u8> {
        let new_field = signature == b"BIKk" || signature == b"KB2n";
        let table = HEAD_BYTES + if new_field { 4 } else { 0 } + 12 * tracks.len();
        let mut body = Vec::new();
        let mut offsets = Vec::new();
        let start = table + 4 * frames.len();
        for (audio, video) in frames {
            offsets.push((start + body.len()) as u32);
            for packet in audio {
                body.extend((packet.len() as u32).to_le_bytes());
                body.extend_from_slice(packet);
            }
            body.extend_from_slice(video);
        }
        let mut file = vec![0u8; HEAD_BYTES];
        file[..4].copy_from_slice(signature);
        let total = (start + body.len()) as u32;
        let fields = [
            (4, total - 8),
            (8, frames.len() as u32),
            (12, 16),
            (16, frames.len() as u32),
            (20, 64),
            (24, 48),
            (28, 30),
            (32, 1),
            (40, tracks.len() as u32),
        ];
        for (at, value) in fields {
            file[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        if new_field {
            file.extend([0; 4]);
        }
        file.extend(std::iter::repeat_n(0, 4 * tracks.len()));
        for &(rate, flags) in tracks {
            file.extend(rate.to_le_bytes());
            file.extend(flags.to_le_bytes());
        }
        file.extend(std::iter::repeat_n(0, 4 * tracks.len()));
        for (i, offset) in offsets.iter().enumerate() {
            // Every frame after the first but the third is a key frame.
            let key = u32::from(i != 0 && i != 2);
            file.extend((offset | key).to_le_bytes());
        }
        assert_eq!(file.len(), start);
        file.extend(body);
        file
    }

    /// Frames of one colour each, so a seek shows in which colour comes next.
    struct Colours {
        next: u8,
        count: u8,
    }

    impl VideoStream for Colours {
        fn next_frame(&mut self) -> Result<Option<Frame>, MediaError> {
            if self.next >= self.count {
                return Ok(None);
            }
            let shade = self.next;
            self.next += 1;
            Ok(Some(Frame {
                width: 1,
                height: 1,
                rgba: vec![shade, shade, shade, 255],
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

    #[test]
    fn a_head_states_its_codec_size_rate_and_length() {
        for (signature, codec) in [(b"BIKi", Codec::Bink1), (b"KB2g", Codec::Bink2)] {
            let source = open(head(signature).into()).expect("a Bink head opens");
            let video = source.info().video.expect("a picture track");
            assert_eq!(
                (video.codec, video.width, video.height, video.frames),
                (codec, 640, 360, 90)
            );
            assert_eq!(video.fps(), Some(30.0));
            assert_eq!(
                source.info().audio,
                None,
                "a head with no frame index states no sound track"
            );
            assert_eq!(source.info().duration(), Some(Duration::from_secs(3)));
            assert_eq!(
                source.audio(&Decoders::new()).err(),
                Some(MediaError::NoAudio)
            );
        }
    }

    #[test]
    fn a_movie_states_its_sound_track_and_its_length_from_what_its_packets_say() {
        // Track 0 is stereo DCT at 44,100 Hz, track 1 mono RDFT; the first frame's packet states
        // 8,192 bytes decoded, which is 2,048 frames of two 16-bit channels.
        let mut packet = vec![0u8; 8];
        packet[..4].copy_from_slice(&8192u32.to_le_bytes());
        let frames = [
            (vec![packet.clone(), vec![]], b"v0".to_vec()),
            (vec![vec![], vec![]], b"v1".to_vec()),
            (vec![packet, vec![]], b"v2".to_vec()),
        ];
        let bytes = movie(b"BIKi", &[(44100, 0x2000 | 0x1000), (22050, 0)], &frames);
        let source = open(bytes.into()).expect("it opens");
        let audio = source.info().audio.expect("a sound track");
        assert_eq!(
            audio,
            AudioInfo {
                codec: Codec::BinkAudioDct,
                channels: 2,
                sample_rate: 44100,
                frames: 2 * 2048,
            }
        );
    }

    #[test]
    fn a_kb2n_head_states_its_track_after_the_extra_field_and_its_sound_has_a_row() {
        // One stereo DCT track at 48,000 Hz, after an extra field no reader takes as a count.
        let mut packet = vec![0u8; 8];
        packet[..4].copy_from_slice(&7680u32.to_le_bytes());
        let frames = [
            (vec![packet.clone()], b"v0".to_vec()),
            (vec![packet], b"v1".to_vec()),
        ];
        let mut bytes = movie(b"KB2n", &[(48000, 0x4000 | 0x2000 | 0x1000)], &frames);
        bytes[HEAD_BYTES..HEAD_BYTES + 4].copy_from_slice(&[0xA5; 4]);
        let source = open(bytes.clone().into()).expect("it opens");
        let expected = AudioInfo {
            codec: Codec::Bink2Audio,
            channels: 2,
            sample_rate: 48000,
            frames: 2 * 7680 / 4,
        };
        assert_eq!(source.info().audio, Some(expected));
        assert_eq!(audio_tracks(&bytes), Ok(vec![expected]));

        assert!(source.audio(&Decoders::new()).is_ok());
        assert!(audio_track(&bytes, 0, &Decoders::new()).is_ok());
        // The same flags in a revision before it are the coding Bink 1 has.
        let older = movie(
            b"KB2g",
            &[(48000, 0x1000)],
            &[(vec![vec![]], b"v0".to_vec())],
        );
        let stated = audio_tracks(&older).expect("its tracks");
        assert_eq!(stated[0].codec, Codec::BinkAudioDct);
    }

    #[test]
    fn a_frame_index_that_does_not_rise_or_leaves_the_file_leaves_the_movie_without_sound() {
        let frames = [
            (vec![vec![]], b"aaaa".to_vec()),
            (vec![vec![]], b"bbbb".to_vec()),
        ];
        let mut bytes = movie(b"BIKi", &[(44100, 0x1000)], &frames);
        let at = HEAD_BYTES + 12;
        let first = u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
        // The second frame starts where the first does.
        bytes[at + 4..at + 8].copy_from_slice(&first.to_le_bytes());
        assert!(Layout::parse(&bytes).is_err());
        let source = open(bytes.into()).expect("the head still reads");
        assert_eq!(source.info().audio, None);
        assert!(source.info().video.is_some());

        let mut short = movie(b"BIKi", &[(44100, 0x1000)], &frames);
        short.truncate(short.len() - 3);
        assert!(Layout::parse(&short).is_err(), "a frame past the end");
    }

    #[test]
    fn a_frame_is_split_into_a_packet_per_track_and_the_picture() {
        let frames = [
            (vec![b"track0".to_vec(), vec![]], b"picture0".to_vec()),
            (vec![vec![], b"track1".to_vec()], b"picture1".to_vec()),
        ];
        let bytes = movie(b"BIKk", &[(22050, 0), (22050, 0)], &frames);
        let layout = Layout::parse(&bytes).expect("a k movie with its extra field");
        assert_eq!(
            (layout.revision, layout.tracks.len(), layout.index.len()),
            (b'k', 2, 2)
        );
        let one = layout.parts(&bytes, 1).expect("frame 1");
        assert_eq!(
            (one.audio[0], one.audio[1], one.video),
            (&b""[..], &b"track1"[..], &b"picture1"[..])
        );
        let keys: Vec<bool> = layout.index.iter().map(|s| s.key).collect();
        assert_eq!(keys, [true, true], "the first frame is always a key frame");
    }

    #[test]
    fn a_bink2_movie_is_refused_by_name_where_no_row_decodes_it() {
        let source = open(head(b"KB2g").into()).expect("its info reads");
        let Err(why) = source.video(&Decoders::empty()) else {
            panic!("an empty table has no Bink 2 row");
        };
        assert_eq!(why, MediaError::NoDecoder(Codec::Bink2));
        assert!(why.to_string().contains("Bink 2"), "{why}");

        // The table that decodes it says what is wrong with a head that holds no frames.
        let Err(why) = source.video(&Decoders::new()) else {
            panic!("a head with no frame index decodes nothing");
        };
        assert!(matches!(why, MediaError::Container(_)), "{why}");
    }

    #[test]
    fn an_added_row_is_handed_the_movie_and_decodes_it_frame_by_frame_and_seeks() {
        let bytes = head(b"KB2g");
        let size = bytes.len();
        let source = open(bytes.into()).expect("its info reads");
        let table = Decoders::empty().with(
            Codec::Bink2,
            Decoder::video(move |info, movie| {
                assert_eq!((info.codec, movie.len()), (Codec::Bink2, size));
                Ok(Box::new(Colours { next: 0, count: 5 }))
            }),
        );

        let mut video = source.video(&table).expect("the row opens it");
        assert_eq!(video.frame_rate(), (30, 1));
        let shades = |video: &mut Box<dyn VideoStream>| {
            let mut seen = Vec::new();
            while let Some(frame) = video.next_frame().expect("a frame") {
                seen.push(frame.rgba[0]);
            }
            seen
        };
        assert_eq!(shades(&mut video), vec![0, 1, 2, 3, 4]);
        video.seek(3).expect("this row seeks");
        assert_eq!(shades(&mut video), vec![3, 4]);
    }

    #[test]
    fn a_short_head_opens_with_no_fields_and_a_file_that_is_not_bink_is_refused() {
        let source = open(Arc::from(&b"BIKi"[..])).expect("the signature picks the codec");
        let video = source.info().video.expect("a picture track");
        assert_eq!(
            (video.codec, video.width, video.frames, video.fps()),
            (Codec::Bink1, 0, 0, None)
        );
        assert_eq!(source.info().duration(), None);
        let Err(why) = source.video(&Decoders::new()) else {
            panic!("a head with no frames decodes nothing");
        };
        assert!(matches!(why, MediaError::Container(_)), "{why}");

        let Err(why) = open(Arc::from(&b"RIFF"[..])) else {
            panic!("RIFF is not Bink");
        };
        assert!(matches!(why, MediaError::Container(_)));
    }
}
