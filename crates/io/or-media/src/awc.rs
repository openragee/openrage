//! The streams of an `.awc` as sources: `or-io` parses the container and holds its codecs'
//! decode, and this lists the streams and opens one through the table.
//!
//! ⚠ A multi-channel stream's blocks are `or-io`'s to read: it de-interleaves them and runs each
//! channel's codec itself, so the table is asked only whether it decodes every codec the channels
//! use, and a row added for one codec does not change how a block is laid out.

use std::sync::Arc;

use or_io::awc::opus::{split_packets, OpusStream};
use or_io::awc::wav::interleave_pcm16;
use or_io::awc::{AwcChunkKind, AwcFile, AwcKey};

use crate::{
    opus, AudioInfo, AudioStream, Codec, Decoders, MediaError, MediaInfo, Pcm, Samples, Source,
};

/// The rate every Opus stream decodes at, whatever rate its record states.
const OPUS_RATE: u32 = 48_000;

/// One playable stream: a single-channel stream, or a multi-channel container's interleaved source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stream {
    /// Index into [`AwcFile::streams`].
    pub stream: usize,
    /// The stream's name hash; the file stores no name.
    pub id: u32,
    pub info: AudioInfo,
}

/// Every playable stream of the container, read from its `format` and `streamformat` chunks alone.
pub fn streams(file: &AwcFile) -> Vec<Stream> {
    file.streams
        .iter()
        .enumerate()
        .filter_map(|(stream, s)| {
            if let Ok(Some(format)) = s.opus_format() {
                return Some(Stream {
                    stream,
                    id: s.id,
                    info: AudioInfo {
                        codec: Codec::Opus,
                        channels: u16::from(format.channels),
                        sample_rate: OPUS_RATE,
                        frames: u64::from(format.samples_per_channel),
                    },
                });
            }
            // GTA VI's mono PCM16 and XMA decode as GTA V's do, and codec 2 by its own row, at the
            // rate the record states.
            if let Ok(Some(format)) = s.coded_format() {
                return Some(Stream {
                    stream,
                    id: s.id,
                    info: AudioInfo {
                        codec: match format.codec {
                            spectra if spectra == or_io::awc::Codec::SPECTRA => Codec::Spectra,
                            other => other.into(),
                        },
                        channels: 1,
                        sample_rate: u32::from(format.stated_rate),
                        frames: u64::from(format.samples_per_channel),
                    },
                });
            }
            if let Some(format) = s.format() {
                return Some(Stream {
                    stream,
                    id: s.id,
                    info: AudioInfo {
                        codec: format.codec.into(),
                        channels: 1,
                        sample_rate: u32::from(format.samples_per_second),
                        frames: u64::from(format.samples),
                    },
                });
            }
            // A GTA VI 32-byte record (chunk `OpusFormat`) stating a codec none of the readers above
            // know: list it as that codec so it is named and refused, never dropped from the panel.
            // A PlayStation 5 build may carry a codec Xbox does not (ATRAC9, say); codec 2 is Spectra,
            // read above, so only a genuinely unnamed byte reaches here.
            if let Some(record) = s.chunk(or_io::awc::AwcChunkKind::OpusFormat) {
                let stated = Codec::from(or_io::awc::Codec::from_byte(*record.payload.get(0x1C)?));
                // Codec 2 is Spectra, read above; a malformed one stays dropped. Any other unnamed byte
                // is surfaced so it refuses by name instead of vanishing from the panel.
                if let Codec::Unknown(byte) = stated {
                    if byte != 2 {
                        let u16_at = |o: usize| {
                            record
                                .payload
                                .get(o..o + 2)
                                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                        };
                        let u32_at = |o: usize| {
                            (record.payload.get(o..o + 4))
                                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                        };
                        return Some(Stream {
                            stream,
                            id: s.id,
                            info: AudioInfo {
                                codec: Codec::Unknown(byte),
                                channels: 1,
                                sample_rate: u32::from(u16_at(0x08).unwrap_or(0)),
                                frames: u64::from(u32_at(0x0C).unwrap_or(0)),
                            },
                        });
                    }
                }
            }
            let format = s.stream_format()?;
            let first = format.channels.first()?;
            if first.codec == or_io::awc::Codec::Opus {
                // A channel may be stereo, which only the Opus reading of the records states.
                let groups = file.opus_streams(stream).ok()?;
                return Some(Stream {
                    stream,
                    id: s.id,
                    info: AudioInfo {
                        codec: Codec::Opus,
                        channels: groups.iter().map(|g| u16::from(g.channels)).sum(),
                        sample_rate: OPUS_RATE,
                        frames: u64::from(groups.first()?.samples),
                    },
                });
            }
            Some(Stream {
                stream,
                id: s.id,
                info: AudioInfo {
                    codec: first.codec.into(),
                    channels: format.channels.len() as u16,
                    sample_rate: u32::from(first.samples_per_second),
                    frames: u64::from(first.samples),
                },
            })
        })
        .collect()
}

/// One stream of the container, ready to decode; `key` unscrambles it when the file scrambles it.
pub fn open(file: &AwcFile, key: AwcKey, stream: usize) -> Result<Box<dyn Source>, MediaError> {
    let row = streams(file)
        .into_iter()
        .find(|row| row.stream == stream)
        .ok_or_else(|| {
            MediaError::Container(format!("stream {stream} has no format to play it by"))
        })?;
    // GTA VI's format record marks data stored in the clear, whatever flag bit 1 says.
    let scrambled = !file.whole_file_encrypted
        && (file.single_channel_encrypted() || file.multi_channel_encrypted())
        && row.info.codec != Codec::Opus
        && file.streams[stream]
            .chunk(AwcChunkKind::OpusFormat)
            .is_none();
    if scrambled && !key.is_usable() {
        return Err(MediaError::Container(
            "this stream is scrambled and no AWC key is loaded: extract keys from your game \
             executable"
                .to_string(),
        ));
    }
    let info = MediaInfo {
        audio: Some(row.info),
        video: None,
    };
    let single = file.streams[stream].opus_format().ok().flatten().is_some();
    if row.info.codec == Codec::Opus && !single {
        let groups = file
            .opus_streams(stream)
            .map_err(|e| MediaError::Container(e.to_string()))?;
        return Ok(Box::new(OpusChannels { info, groups }));
    }
    // A single Opus stream, stereo too, is decoded from its data chunk by the table's row.
    if row.info.channels == 1 || row.info.codec == Codec::Opus {
        let data = file
            .stream_data(&file.streams[stream], key)
            .ok_or_else(|| MediaError::Container("this stream has no data chunk".to_string()))?;
        return Ok(Box::new(Single {
            info,
            data: data.into(),
        }));
    }
    let codecs = file
        .streams
        .iter()
        .find_map(|s| s.stream_format())
        .map(|format| format.channels.iter().map(|c| c.codec.into()).collect())
        .unwrap_or_default();
    Ok(Box::new(Interleaved {
        info,
        codecs,
        file: file.clone(),
        key,
    }))
}

struct Single {
    info: MediaInfo,
    data: Arc<[u8]>,
}

impl Source for Single {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn audio(&self, decoders: &Decoders) -> Result<Box<dyn AudioStream>, MediaError> {
        let audio = self.info.audio.ok_or(MediaError::NoAudio)?;
        let open = decoders.audio_decoder(audio.codec)?;
        open(&audio, &self.data)
    }
}

/// A multi-channel container, held whole because its blocks are decoded together.
struct Interleaved {
    info: MediaInfo,
    codecs: Vec<Codec>,
    file: AwcFile,
    key: AwcKey,
}

impl Source for Interleaved {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn audio(&self, decoders: &Decoders) -> Result<Box<dyn AudioStream>, MediaError> {
        let audio = self.info.audio.ok_or(MediaError::NoAudio)?;
        for codec in &self.codecs {
            decoders.audio_decoder(*codec)?;
        }
        let Some(channels) = self.file.decode_channels(self.key, decoders.accumulator) else {
            return Err(MediaError::Decode(match audio.codec {
                Codec::Mp3 => "this stream is MP3 and its frames do not decode to the samples \
                               its format states"
                    .to_string(),
                _ => "this stream's blocks do not fit the layout its format states".to_string(),
            }));
        };
        let samples = interleave_pcm16(&channels)
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        Ok(Box::new(Pcm::new(Samples::I16(samples), audio.channels)))
    }
}

/// A multi-channel Opus source: each channel's packets, the repeats already dropped.
struct OpusChannels {
    info: MediaInfo,
    groups: Vec<OpusStream>,
}

impl Source for OpusChannels {
    fn info(&self) -> &MediaInfo {
        &self.info
    }

    fn audio(&self, decoders: &Decoders) -> Result<Box<dyn AudioStream>, MediaError> {
        let audio = self.info.audio.ok_or(MediaError::NoAudio)?;
        decoders.audio_decoder(Codec::Opus)?;
        let samples = opus_pcm(&self.groups, &|_, _| true)?;
        Ok(Box::new(Pcm::new(Samples::I16(samples), audio.channels)))
    }
}

/// Stream `stream`'s Opus packets as [`open`] splits them; empty for a stream that holds no Opus.
pub fn opus_groups(file: &AwcFile, stream: usize) -> Result<Vec<OpusStream>, MediaError> {
    let s = (file.streams.get(stream))
        .ok_or_else(|| MediaError::Container(format!("there is no stream {stream}")))?;
    let format = (s.opus_format()).map_err(|e| MediaError::Container(e.to_string()))?;
    let Some(format) = format else {
        return (file.opus_streams(stream)).map_err(|e| MediaError::Container(e.to_string()));
    };
    // Opus data is stored in the clear, so no key is asked for.
    let data = (file.stream_data(s, AwcKey::default()))
        .ok_or_else(|| MediaError::Container("this stream has no data chunk".to_string()))?;
    let packets =
        split_packets(&data, format.channels).map_err(|e| MediaError::Decode(e.to_string()))?;
    Ok(vec![OpusStream {
        id: s.id,
        channels: format.channels,
        samples: format.samples_per_channel,
        packets,
    }])
}

/// `groups` decoded and interleaved, each channel as long as the first; `watch` hears each packet.
pub fn opus_pcm(groups: &[OpusStream], watch: opus::Watch<'_>) -> Result<Vec<i16>, MediaError> {
    let tick = opus::ticks(watch, groups.iter().map(|g| g.packets.len() as u64).sum());
    // A stereo channel is two of the output; the first group's decode bounds what it states.
    let frames = groups.first().map_or(0, |g| g.samples as usize);
    let mut decoded = Vec::with_capacity(groups.len());
    for group in groups {
        let mut pcm = opus::decode_ticked(group, &tick)?;
        pcm.resize(frames * usize::from(group.channels), 0);
        decoded.push((pcm, usize::from(group.channels)));
    }
    let width: usize = decoded.iter().map(|(_, k)| k).sum();
    let mut samples = Vec::with_capacity(frames * width);
    for t in 0..frames {
        for (pcm, k) in &decoded {
            samples.extend_from_slice(&pcm[t * k..(t + 1) * k]);
        }
    }
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A GTA VI container of one stereo Opus stream, flag bit 1 set: its format record, one
    /// 2,048-byte packet holding one Opus packet, and a seek table.
    fn stereo_opus() -> Vec<u8> {
        let mut record = vec![0u8; 32];
        record[0..4].copy_from_slice(&1000u32.to_le_bytes());
        record[4..8].copy_from_slice(&(-1i32).to_le_bytes());
        record[12..16].copy_from_slice(&500u32.to_le_bytes());
        record[0x1C] = 11;
        record[0x1D] = 2;
        let mut data = vec![0u8; 2048];
        data[0..2].copy_from_slice(&3u16.to_le_bytes());
        data[2..5].copy_from_slice(&[0xFC, 0xFF, 0xFE]);
        single(&[(0x76u64, record), (0x55, data), (0xA3, vec![0; 8])])
    }

    /// A GTA VI container of one stream 0x5150 holding `chunks`, flag bit 1 set.
    fn single(chunks: &[(u64, Vec<u8>)]) -> Vec<u8> {
        let tables = 16 + 4 + chunks.len() * 8;
        let mut out = b"ADAT".to_vec();
        let count = chunks.len() as u32;
        for w in [0xFF02_0001u32, 1, tables as u32, count << 29 | 0x5150] {
            out.extend(w.to_le_bytes());
        }
        let mut at = out.len() + chunks.len() * 8;
        for (kind, payload) in chunks {
            out.extend((kind << 56 | (payload.len() as u64) << 28 | at as u64).to_le_bytes());
            at += payload.len();
        }
        chunks.iter().for_each(|(_, payload)| out.extend(payload));
        out
    }

    /// A mono PCM16 record of `samples` at 44,100 Hz, codec byte 0.
    fn pcm_record(samples: u32) -> Vec<u8> {
        let mut record = vec![0u8; 32];
        record[0..4].copy_from_slice(&samples.to_le_bytes());
        record[4..8].copy_from_slice(&(-1i32).to_le_bytes());
        record[8..10].copy_from_slice(&44_100u16.to_le_bytes());
        record[12..16].copy_from_slice(&samples.to_le_bytes());
        (record[0x1C], record[0x1D]) = (0, 1);
        record
    }

    #[test]
    fn a_gta_vi_pcm16_stream_is_listed_and_plays_its_samples_with_flag_bit_1_set() {
        let pcm: Vec<i16> = (0..700).map(|i| (i * 41 % 3000 - 1500) as i16).collect();
        let data: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
        let bytes = single(&[(0x76, pcm_record(700)), (0x55, data)]);
        let file = AwcFile::parse(&bytes, AwcKey::default()).expect("parses");
        let want = AudioInfo {
            codec: Codec::Pcm16,
            channels: 1,
            sample_rate: 44_100,
            frames: 700,
        };
        assert_eq!(
            streams(&file).iter().map(|r| r.info).collect::<Vec<_>>(),
            [want]
        );
        let source = open(&file, AwcKey::default(), 0).expect("opens without a key");
        let played = (source.audio(&Decoders::new()))
            .and_then(|mut audio| audio.read_all_i16())
            .expect("it decodes");
        assert_eq!(played, pcm);
        assert!(opus_groups(&file, 0).expect("not refused").is_empty());
    }

    #[test]
    fn a_gta_vi_xma_stream_is_listed_and_a_codec_2_stream_its_blocks_contradict_is_not() {
        let mut xma = pcm_record(1000);
        xma[0..4].copy_from_slice(&(512u32 * 4).to_le_bytes());
        xma[8..10].copy_from_slice(&32_000u16.to_le_bytes());
        xma[0x1C] = 5;
        let mut packet = vec![0u8; 2048];
        packet[0] = 4 << 2;
        let file = AwcFile::parse(&single(&[(0x76, xma), (0x55, packet)]), AwcKey::default());
        let file = file.expect("parses");
        let want = AudioInfo {
            codec: Codec::Xma,
            channels: 1,
            sample_rate: 32_000,
            frames: 1000,
        };
        assert_eq!(
            streams(&file).iter().map(|r| r.info).collect::<Vec<_>>(),
            [want]
        );
        let source = open(&file, AwcKey::default(), 0).expect("opens");
        let refused = source.audio(&Decoders::empty()).err().expect("no row");
        assert_eq!(refused, MediaError::NoDecoder(Codec::Xma));

        let mut spectra = pcm_record(400);
        spectra[0..4].copy_from_slice(&(2048u32 * 20).to_le_bytes());
        spectra[0x1C] = 2;
        let bytes = single(&[(0x76, spectra), (0x55, vec![0; 8192 * 20])]);
        let file = AwcFile::parse(&bytes, AwcKey::default()).expect("parses");
        assert!(streams(&file).is_empty());
        let why = opus_groups(&file, 0).expect_err("refused by name");
        assert!(why.to_string().contains("codec 2"), "{why}");
        assert!(open(&file, AwcKey::default(), 0).is_err());
    }

    /// A GTA VI container of one codec-2 stream holding `signal`: its blocks, the zero blocks and a
    /// trailer, 20 blocks in all.
    fn spectra_container(signal: &[f64]) -> Vec<u8> {
        let mut data = crate::spectra::encode(signal);
        let mut zero = vec![0u8; 8192];
        (1025..2048).step_by(2).for_each(|f| zero[4 * f + 3] = 0x80);
        while data.len() < 19 * 8192 {
            data.extend_from_slice(&zero);
        }
        let mut trailer = vec![0u8; 8192];
        trailer[0..4].copy_from_slice(&0xFFFF_ACE1u32.to_le_bytes());
        data.extend(trailer);
        let mut record = pcm_record(signal.len() as u32);
        record[0..4].copy_from_slice(&(2048u32 * 20).to_le_bytes());
        record[8..10].copy_from_slice(&48_000u16.to_le_bytes());
        record[0x1C] = 2;
        single(&[(0x76, record), (0x55, data)])
    }

    /// Every sample `source` plays, as `f32`.
    fn played(source: &dyn Source) -> Vec<f32> {
        let mut audio = source.audio(&Decoders::new()).expect("a row decodes it");
        let mut all = Vec::new();
        while let Some(block) = audio.read().expect("it decodes") {
            all.extend(block.into_f32());
        }
        all
    }

    #[test]
    fn a_gta_vi_codec_2_stream_is_listed_plays_its_signal_and_is_replaced_from_a_wav() {
        let signal: Vec<f64> = (0..3_000)
            .map(|n| (n as f64 / 23.0).sin() * (-(n as f64) / 900.0).exp() * 0.7)
            .collect();
        let file = AwcFile::parse(&spectra_container(&signal), AwcKey::default());
        let file = file.expect("parses");
        let want = AudioInfo {
            codec: Codec::Spectra,
            channels: 1,
            sample_rate: 48_000,
            frames: 3_000,
        };
        assert_eq!(
            streams(&file).iter().map(|r| r.info).collect::<Vec<_>>(),
            [want]
        );
        assert!(opus_groups(&file, 0).expect("not refused").is_empty());
        let source = open(&file, AwcKey::default(), 0).expect("opens without a key");
        let heard = played(source.as_ref());
        assert_eq!(heard.len(), signal.len());
        let off = |a: &[f64], b: &[f32]| {
            (a.iter().zip(b))
                .map(|(a, b)| (a - f64::from(*b)).abs())
                .fold(0.0, f64::max)
        };
        let worst = off(&signal[..], &heard[..]);
        assert!(worst < 1e-5, "off by {worst}");

        let pcm: Vec<i16> = (0..2_500)
            .map(|n| ((n * 53) % 9_000 - 4_500) as i16)
            .collect();
        let wav: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
        let edited = crate::spectra::replaced(&file, 0, &wav, 48_000, 1).expect("replaced");
        let bytes = edited.to_bytes(AwcKey::default()).expect("written");
        let read = AwcFile::parse(&bytes, AwcKey::default()).expect("it reads back");
        let source = open(&read, AwcKey::default(), 0).expect("opens");
        let heard = played(source.as_ref());
        let want: Vec<f64> = pcm.iter().map(|s| f64::from(*s) / 32_768.0).collect();
        assert_eq!(heard.len(), want.len());
        let worst = off(&want[..], &heard[..]);
        assert!(worst < 1e-5, "off by {worst}");
        let stated = read.streams[0]
            .coded_format()
            .expect("read")
            .expect("codec 2");
        assert_eq!(stated.peak, 52_056, "the loudest window states 52,056");
        let why = crate::spectra::replaced(&file, 0, &wav, 44_100, 1).expect_err("refused");
        assert!(why.to_string().contains("save it at 48000 Hz"), "{why}");
    }

    #[test]
    fn an_opus_stream_is_listed_and_decodes_to_the_samples_its_record_states() {
        let file = AwcFile::parse(&stereo_opus(), AwcKey::default()).expect("parses");
        assert!(file.single_channel_encrypted());
        let rows = streams(&file);
        assert_eq!(rows.len(), 1);
        let want = AudioInfo {
            codec: Codec::Opus,
            channels: 2,
            sample_rate: 48_000,
            frames: 500,
        };
        assert_eq!(rows[0].info, want);
        let source = open(&file, AwcKey::default(), 0).expect("opens without a key");
        assert_eq!(source.info().audio, Some(want));
        let samples = (source.audio(&Decoders::new()))
            .and_then(|mut audio| audio.read_all_i16())
            .expect("the one packet decodes");
        assert_eq!(samples.len(), 2 * 500);
        let refused = source.audio(&Decoders::empty()).err().expect("refused");
        assert_eq!(refused, MediaError::NoDecoder(Codec::Opus));
        assert_eq!(
            refused.to_string(),
            "this is Opus, which OpenRage has no decoder for"
        );
    }

    #[test]
    fn an_opus_stream_decodes_as_it_plays_telling_each_packet_and_stops_when_told() {
        let file = AwcFile::parse(&stereo_opus(), AwcKey::default()).expect("parses");
        let groups = opus_groups(&file, 0).expect("the packets split");
        let listed: Vec<_> = (groups.iter())
            .map(|g| (g.id, g.channels, g.samples, g.packets.len()))
            .collect();
        assert_eq!(listed, [(0x5150, 2, 500, 1)]);
        let told = std::cell::RefCell::new(Vec::new());
        let pcm = opus_pcm(&groups, &|done, total| {
            told.borrow_mut().push((done, total));
            true
        })
        .expect("it decodes");
        let source = open(&file, AwcKey::default(), 0).expect("opens");
        let played = (source.audio(&Decoders::new()))
            .and_then(|mut audio| audio.read_all_i16())
            .expect("it plays");
        assert_eq!(pcm, played);
        assert_eq!(told.into_inner(), [(1, 1)]);
        assert!(opus_pcm(&groups, &|_, _| false).is_err());
        assert!(opus_groups(&file, 1).is_err());
    }

    #[test]
    fn a_gta_vi_stream_of_an_unknown_codec_is_listed_and_refused_by_name() {
        // A GTA VI 32-byte record (chunk 0x76) stating codec 8 — one no reader knows, as a
        // PlayStation 5 build may carry. It is listed, not dropped, and refuses to decode by name.
        let mut record = vec![0u8; 32];
        record[0x00..0x04].copy_from_slice(&400u32.to_le_bytes());
        record[0x08..0x0A].copy_from_slice(&48_000u16.to_le_bytes());
        record[0x0C..0x10].copy_from_slice(&400u32.to_le_bytes());
        record[0x1C] = 8;
        record[0x1D] = 1;
        let bytes = single(&[(0x76, record), (0x55, vec![0u8; 8192])]);
        let file = AwcFile::parse(&bytes, AwcKey::default()).expect("parses");
        let rows = streams(&file);
        assert_eq!(
            rows.len(),
            1,
            "the unknown-codec stream is listed, not dropped"
        );
        assert_eq!(rows[0].info.codec, Codec::Unknown(8));
        assert_eq!(rows[0].info.codec.label(), "unknown(8)");
        assert_eq!(
            (
                rows[0].info.channels,
                rows[0].info.sample_rate,
                rows[0].info.frames
            ),
            (1, 48_000, 400)
        );
        let source = open(&file, AwcKey::default(), 0).expect("opens");
        let refused = source.audio(&Decoders::new()).err().expect("no decoder");
        assert_eq!(refused, MediaError::NoDecoder(Codec::Unknown(8)));
    }
}
