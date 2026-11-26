//! Replacing a stream's samples from PCM16, each channel encoded in the codec it already has.

use super::codec::{self, Accumulator, Codec, ADPCM_BLOCK_PAYLOAD_BYTES};
use super::opus::{is_measured_toc, OpusStream, OPUS_FRAME};
use super::{crypto, AwcChunkKind, AwcFile, AwcKey, MULTI_CHANNEL_PACKET};

mod gta6_records;
mod in_extent;
mod opus_blocks;

pub use in_extent::NotInExtent;

/// Samples in one 2,048-byte PCM16 packet.
const PCM16_PACKET_SAMPLES: usize = MULTI_CHANNEL_PACKET / 2;

/// Samples in one 2,048-byte ADPCM packet, which is one ADPCM block.
const ADPCM_PACKET_SAMPLES: usize = ADPCM_BLOCK_PAYLOAD_BYTES * 2;

/// A multi-channel block's `u32` for a channel that states no first packet.
const NO_FIRST_PACKET: u32 = u32::MAX;

/// The average bitrate, in kbps, a replaced MP3 stream is encoded at: the shipped streams' own
/// average is 82.5.
const MP3_KBPS: f32 = 96.0;

/// Why a stream was not replaced, in words a modder reads.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct Refused(String);

fn refuse<T>(why: String) -> Result<T, Refused> {
    Err(Refused(why))
}

impl AwcFile {
    /// This container with stream `stream`'s samples replaced by interleaved PCM16 in each channel's own codec.
    ///
    /// ⚠ **A single-channel stream keeps its `format` chunk's other fields and
    /// its peaks true; a multi-channel one has its shared blocks rebuilt**, so
    /// the file reads back as the shipped containers' geometry does (see
    /// [`AwcFile::decode_channels`]). Nothing is resampled, and an MP3
    /// channel of a multi-channel stream has no encoder here.
    pub fn with_stream_replaced(
        &self,
        stream: usize,
        pcm: &[u8],
        rate: u32,
        channels: u16,
        key: AwcKey,
    ) -> Result<AwcFile, Refused> {
        let Some(s) = self.streams.get(stream) else {
            return refuse("no such stream".to_string());
        };
        if s.format().is_some() {
            self.replaced_single(stream, pcm, rate, channels, key)
        } else if s.stream_format().is_some() {
            self.replaced_multi(stream, pcm, rate, channels, key)
        } else if s.chunk(AwcChunkKind::OpusFormat).is_some() {
            self.replaced_record(stream, pcm, rate, channels)
        } else {
            refuse(format!("stream {stream} has no format to replace it by"))
        }
    }

    fn replaced_single(
        &self,
        stream: usize,
        pcm: &[u8],
        rate: u32,
        channels: u16,
        key: AwcKey,
    ) -> Result<AwcFile, Refused> {
        let s = &self.streams[stream];
        let (Some(format), Some(chunk)) = (s.format(), s.chunk(AwcChunkKind::Format)) else {
            return refuse(format!("stream {stream} has no format to replace it by"));
        };
        if !matches!(format.codec, Codec::Pcm16 | Codec::ImaAdpcm | Codec::Mp3) {
            let codec = format.codec.label();
            return refuse(format!(
                "this stream is {codec} and OpenRage has no {codec} encoder: only a PCM16, \
                 ADPCM or MP3 stream is replaced"
            ));
        }
        if channels != 1 {
            return refuse(format!(
                "the WAV has {channels} channels and this stream has one: save it as mono"
            ));
        }
        let stream_rate = u32::from(format.samples_per_second);
        if format.codec != Codec::Pcm16 && rate != stream_rate {
            let label = format.codec.label();
            return refuse(format!(
                "the WAV is {rate} Hz and this {label} stream is {stream_rate} Hz, and OpenRage \
                 resamples nothing: save it at {stream_rate} Hz"
            ));
        }
        let rate = rate_field(rate)?;
        let mut samples = pcm.len() / 2;

        // What the stream stores, what it decodes to, and its seek table when it keeps one.
        let (mut data, heard, seek) =
            match format.codec {
                Codec::ImaAdpcm => {
                    let adpcm = codec::encode_adpcm(pcm);
                    let heard = codec::decode_adpcm(&adpcm, samples, Accumulator::Clamped);
                    (adpcm, heard, None)
                }
                Codec::Mp3 => {
                    let Some(old) = self.stream_data(s, key) else {
                        return refuse(
                            "this stream is scrambled and no AWC key is loaded: extract keys from \
                         your game executable"
                                .to_string(),
                        );
                    };
                    // ⚠ The frames' own rate, not the format chunk's: 36,237 of Enhanced's 213,875
                    // streams state a rate the game plays at and frames at 32,000 Hz.
                    let frames_at = codec::mpeg_frame_rate(&old)
                        .unwrap_or(if stream_rate > 40_000 { 48_000 } else { 32_000 });
                    let mp3 = codec::encode_mp3(pcm, frames_at, MP3_KBPS).map_err(Refused)?;
                    samples = mp3.samples;
                    let heard = codec::decode(&mp3.data, samples, Codec::Mp3, Accumulator::Clamped)
                        .ok_or_else(|| Refused("the new MP3 frames do not decode".to_string()))?;
                    let sizes = mp3
                        .frame_sizes
                        .iter()
                        .flat_map(|s| s.to_le_bytes())
                        .collect();
                    (mp3.data, heard, Some(sizes))
                }
                _ => (pcm.to_vec(), pcm.to_vec(), None),
            };
        let envelope = codec::peak_envelope(&heard, 1 + codec::peak_window_count(samples));

        let mut header = chunk.payload.clone();
        header[0..4].copy_from_slice(&(samples as u32).to_le_bytes());
        header[8..10].copy_from_slice(&rate.to_le_bytes());
        if header.len() >= 24 {
            header[20..22].copy_from_slice(&envelope[0].to_le_bytes());
        }
        let peaks = s.chunk(AwcChunkKind::Peak).map(|_| {
            envelope[1..]
                .iter()
                .flat_map(|p| p.to_le_bytes())
                .collect::<Vec<u8>>()
        });
        if self.single_channel_encrypted() && !self.whole_file_encrypted {
            if !key.is_usable() {
                return refuse(
                    "this file scrambles each stream and no AWC key is loaded to scramble the \
                     new one: extract keys from your game executable"
                        .to_string(),
                );
            }
            crypto::encrypt(key, &mut data);
        }

        let mut edited = self.clone();
        let mut set = |kind, payload| {
            edited
                .set_chunk_payload(stream, kind, payload)
                .map_err(|e| Refused(e.to_string()))
        };
        set(AwcChunkKind::Format, header)?;
        if let Some(peaks) = peaks {
            set(AwcChunkKind::Peak, peaks)?;
        }
        if let (Some(seek), Some(_)) = (seek, s.chunk(AwcChunkKind::SeekTable)) {
            set(AwcChunkKind::SeekTable, seek)?;
        }
        set(AwcChunkKind::Data, data)?;
        Ok(edited)
    }

    fn replaced_multi(
        &self,
        stream: usize,
        pcm: &[u8],
        rate: u32,
        channels: u16,
        key: AwcKey,
    ) -> Result<AwcFile, Refused> {
        let s = &self.streams[stream];
        let (Some(format), Some(chunk)) = (s.stream_format(), s.chunk(AwcChunkKind::StreamFormat))
        else {
            return refuse(format!("stream {stream} has no format to replace it by"));
        };
        let n = format.channels.len();
        if n == 0 {
            return refuse("this stream states no channels".to_string());
        }
        if usize::from(channels) != n {
            return refuse(format!(
                "the WAV has {channels} channel(s) and this stream has {n}: save it with {n}"
            ));
        }
        let codec = format.channels[0].codec;
        if format.channels.iter().any(|c| c.codec != codec) {
            return refuse(
                "this stream's channels are in different codecs, and OpenRage rebuilds only \
                 blocks whose channels share one"
                    .to_string(),
            );
        }
        let per_packet = match codec {
            Codec::Pcm16 => PCM16_PACKET_SAMPLES,
            Codec::ImaAdpcm => ADPCM_PACKET_SAMPLES,
            Codec::Mp3 => {
                return refuse(
                    "this multi-channel stream is MP3, and OpenRage encodes MP3 only into a \
                     single-channel stream: each block of a multi-channel one repeats the last \
                     frame of the block before it, which is not rebuilt"
                        .to_string(),
                );
            }
            other => {
                let codec = other.label();
                return refuse(format!(
                    "this stream is {codec} and OpenRage has no {codec} encoder: only a PCM16 \
                     or ADPCM stream is replaced"
                ));
            }
        };
        if codec == Codec::ImaAdpcm {
            if let Some(other) = format
                .channels
                .iter()
                .map(|c| u32::from(c.samples_per_second))
                .find(|stream_rate| *stream_rate != rate)
            {
                return refuse(format!(
                    "the WAV is {rate} Hz and this ADPCM stream is {other} Hz, and OpenRage \
                     resamples nothing: save it at {other} Hz"
                ));
            }
        }
        let rate = rate_field(rate)?;
        let samples = pcm.len() / (2 * n);
        if samples == 0 {
            return refuse("the WAV holds no samples".to_string());
        }
        let block_size = format.block_size as usize;
        let layout = Layout::new(n, block_size)?;

        let Some(old) = self.stream_data(s, key) else {
            return refuse(
                "this stream's blocks are scrambled and no AWC key is loaded: extract keys \
                 from your game executable"
                    .to_string(),
            );
        };
        let words_unset = (0..n).all(|c| word(&old, c * 24) == Some(NO_FIRST_PACKET));

        // ⚠ An ADPCM channel the WAV leaves exactly as it decodes keeps its shipped packets:
        // re-encoding a decode is close, not exact, so an edit to one channel would otherwise
        // cost every other channel a generation.
        let shipped = (codec == Codec::ImaAdpcm)
            .then(|| {
                let decoded = self.decode_channels(key, Accumulator::Clamped)?;
                Some((decoded, channel_runs(&old, block_size, n)?))
            })
            .flatten();
        let runs: Vec<Vec<u8>> = (0..n)
            .map(|c| {
                let own: Vec<u8> = pcm
                    .chunks_exact(2 * n)
                    .flat_map(|frame| frame[c * 2..c * 2 + 2].to_vec())
                    .collect();
                if let Some((decoded, runs)) = &shipped {
                    let packets = samples.div_ceil(per_packet) * MULTI_CHANNEL_PACKET;
                    if decoded[c] == own && runs[c].len() == packets {
                        return runs[c].clone();
                    }
                }
                let mut run = match codec {
                    Codec::ImaAdpcm => codec::encode_adpcm(&own),
                    _ => own,
                };
                run.resize(run.len().next_multiple_of(MULTI_CHANNEL_PACKET), 0);
                run
            })
            .collect();
        let (mut data, starts) = layout.blocks(&runs, samples, per_packet, words_unset);

        if self.multi_channel_encrypted() && !self.whole_file_encrypted {
            if !key.is_usable() {
                return refuse(
                    "this file scrambles each block and no AWC key is loaded to scramble the \
                     new ones: extract keys from your game executable"
                        .to_string(),
                );
            }
            for block in data.chunks_mut(block_size) {
                crypto::encrypt(key, block);
            }
        }

        let mut header = chunk.payload.clone();
        header[0..4].copy_from_slice(&(starts.len() as u32).to_le_bytes());
        for c in 0..n {
            let at = 12 + c * 16;
            header[at + 4..at + 8].copy_from_slice(&(samples as u32).to_le_bytes());
            header[at + 10..at + 12].copy_from_slice(&rate.to_le_bytes());
        }

        let mut edited = self.clone();
        let mut set = |kind, payload| {
            edited
                .set_chunk_payload(stream, kind, payload)
                .map_err(|e| Refused(e.to_string()))
        };
        set(AwcChunkKind::StreamFormat, header)?;
        if s.chunk(AwcChunkKind::SeekTable).is_some() {
            set(
                AwcChunkKind::SeekTable,
                starts.iter().flat_map(|s| s.to_le_bytes()).collect(),
            )?;
        }
        set(AwcChunkKind::Data, data)?;
        Ok(edited)
    }
}

/// Samples of each channel one peak window covers.
const PEAK_WINDOW: u32 = 4096;

/// Samples of each channel from one loudness record to the next.
pub const LOUDNESS_STEP: u32 = 4800;

/// Samples of each channel one loudness record measures: 400 ms.
pub const LOUDNESS_WINDOW: u32 = 19_200;

/// What frames × 960 − samples is in every shipped single Opus stream: the delay and the padding.
const OPUS_SPARE: std::ops::Range<u64> = 1920..2880;

/// The bytes of a 2,048-byte packet the game's encoder fills: a frame enters while what the packet
/// holds, its `u16` length and the frame fit in this many.
const PACKET_ROOM: usize = MULTI_CHANNEL_PACKET - 2;

/// The longest Opus frame a packet takes beside its `u16` length.
const LONGEST_FRAME: usize = PACKET_ROOM - 2;

/// The peak windows an Opus stream of `samples` per channel states, window 0 included.
pub fn opus_peak_windows(samples: u32) -> usize {
    (samples / PEAK_WINDOW).max(1) as usize
}

/// The loudness records an Opus stream of `samples` per channel states.
pub fn loudness_windows(samples: u32) -> usize {
    samples
        .checked_sub(LOUDNESS_WINDOW)
        .map_or(0, |rest| (rest / LOUDNESS_STEP + 1) as usize)
}

impl AwcFile {
    /// This container with single Opus stream `stream` storing `opus`'s packets instead.
    pub fn with_opus_replaced(
        &self,
        stream: usize,
        opus: &OpusStream,
        peaks: &[u16],
        loudness: &[f32],
    ) -> Result<AwcFile, Refused> {
        // ⚠ `peaks` and `loudness` come from the caller's decode of the new packets: this crate
        // decodes no Opus. The record's 0x08 and 0x0A words are carried, and nothing can check
        // that the game accepts what is written.
        let Some(s) = self.streams.get(stream) else {
            return refuse("no such stream".to_string());
        };
        let (Ok(Some(format)), Some(chunk)) = (s.opus_format(), s.chunk(AwcChunkKind::OpusFormat))
        else {
            return refuse(format!(
                "stream {stream} has no Opus format record to replace it by"
            ));
        };
        let k = format.channels;
        if opus.channels != k {
            return refuse(format!(
                "the new packets are {} channel(s) and this stream has {k}",
                opus.channels
            ));
        }
        let fits = |p: &[u8]| !p.is_empty() && p.len() <= LONGEST_FRAME && is_measured_toc(p[0], k);
        if let Some(p) = opus.packets.iter().find(|p| !fits(p.as_slice())) {
            return refuse(format!(
                "an Opus packet of {} bytes is not one 20 ms frame of {k} channel(s) that a \
                 2,048-byte packet holds",
                p.len()
            ));
        }
        let frames = opus.packets.len() as u64;
        let step = u64::from(OPUS_FRAME) * u64::from(k);
        let held = frames * u64::from(OPUS_FRAME);
        let spare = held.checked_sub(u64::from(opus.samples));
        let in_range = spare.is_some_and(|s| OPUS_SPARE.contains(&s));
        let (Some(whole), true) = (opus.samples.checked_mul(u32::from(k)), in_range) else {
            return refuse(format!(
                "{frames} Opus frames hold {held} samples per channel, which is not {} to {} \
                 more than the {} stated, as in every shipped stream",
                OPUS_SPARE.start, OPUS_SPARE.end, opus.samples
            ));
        };
        if step * frames > u64::from(u32::MAX) {
            return refuse(format!(
                "{frames} Opus frames do not fit the seek table's words"
            ));
        }
        let windows = opus_peak_windows(opus.samples);
        let levels = s.chunk(AwcChunkKind::Loudness).is_some();
        let records = if levels {
            loudness_windows(opus.samples)
        } else {
            0
        };
        if peaks.len() != windows || loudness.len() != records {
            return refuse(format!(
                "{} peak windows and {} loudness records were given for {} samples, which state \
                 {windows} and {records}",
                peaks.len(),
                loudness.len(),
                opus.samples
            ));
        }

        let (mut data, mut seek) = (Vec::new(), Vec::new());
        let mut at = MULTI_CHANNEL_PACKET;
        for (i, p) in opus.packets.iter().enumerate() {
            if at + 2 + p.len() > PACKET_ROOM {
                data.resize(data.len() + MULTI_CHANNEL_PACKET, 0);
                seek.extend(((step * i as u64) as u32).to_le_bytes());
                at = 0;
            }
            let base = data.len() - MULTI_CHANNEL_PACKET + at;
            data[base..base + 2].copy_from_slice(&(p.len() as u16).to_le_bytes());
            data[base + 2..base + 2 + p.len()].copy_from_slice(p);
            at += 2 + p.len();
        }
        seek.extend(((step * (frames - 1)) as u32).to_le_bytes());

        let samples = opus.samples;
        let looped = u32::try_from(format.loop_point)
            .ok()
            .filter(|l| *l < samples);
        let mut record = chunk.payload.clone();
        record[0x00..0x04].copy_from_slice(&whole.to_le_bytes());
        record[0x04..0x08].copy_from_slice(&looped.map_or(-1, |l| l as i32).to_le_bytes());
        record[0x0C..0x10].copy_from_slice(&samples.to_le_bytes());
        record[0x16..0x18].copy_from_slice(&looped.map_or(0xFFFF, |l| l as u16).to_le_bytes());
        // 0x18 states the last sample or 0xFFFF, and which an unlooped stream takes is not known:
        // the stream's own choice is kept.
        if format.word_18 == format.samples_per_channel.wrapping_sub(1) as u16 {
            record[0x18..0x1A].copy_from_slice(&(samples.wrapping_sub(1) as u16).to_le_bytes());
        }
        record[0x1A..0x1C].copy_from_slice(&peaks[0].to_le_bytes());

        let mut edited = self.clone();
        let mut set = |kind, payload| {
            edited
                .set_chunk_payload(stream, kind, payload)
                .map_err(|e| Refused(e.to_string()))
        };
        set(AwcChunkKind::OpusFormat, record)?;
        if s.chunk(AwcChunkKind::Peak).is_some() {
            set(
                AwcChunkKind::Peak,
                peaks[1..].iter().flat_map(|p| p.to_le_bytes()).collect(),
            )?;
        }
        if levels {
            let mut payload = Vec::with_capacity(loudness.len() * 8);
            for (i, level) in loudness.iter().enumerate() {
                payload.extend((i as u32 * LOUDNESS_STEP).to_le_bytes());
                payload.extend(level.to_le_bytes());
            }
            set(AwcChunkKind::Loudness, payload)?;
        }
        set(AwcChunkKind::SeekTable, seek)?;
        set(AwcChunkKind::Data, data)?;
        Ok(edited)
    }
}

fn rate_field(rate: u32) -> Result<u16, Refused> {
    u16::try_from(rate).or_else(|_| {
        refuse(format!(
            "{rate} Hz does not fit the stream's 16-bit rate field"
        ))
    })
}

/// Each channel's packets end to end over the blocks, as `data` lays them out.
fn channel_runs(data: &[u8], block_size: usize, n: usize) -> Option<Vec<Vec<u8>>> {
    let mut runs = vec![Vec::new(); n];
    for block in data.chunks(block_size) {
        let packets: Vec<usize> = (0..n)
            .map(|c| word(block, c * 24 + 4).map(|p| p as usize))
            .collect::<Option<_>>()?;
        let table = n * 24 + packets.iter().sum::<usize>() * 4;
        let mut at = table.next_multiple_of(MULTI_CHANNEL_PACKET);
        for (run, packets) in runs.iter_mut().zip(packets) {
            let end = at + packets * MULTI_CHANNEL_PACKET;
            run.extend_from_slice(block.get(at..end)?);
            at = end;
        }
    }
    Some(runs)
}

/// The packets a block's table takes for `n` channels of `per` packets each.
fn table_packets(n: usize, per: usize) -> usize {
    (n * 24 + n * per * 4).div_ceil(MULTI_CHANNEL_PACKET)
}

fn word(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// How many 2,048-byte packets of each channel a block carries, for `n` channels in blocks of `block_size`.
struct Layout {
    n: usize,
    block_packets: usize,
    per_block: usize,
}

impl Layout {
    /// ⚠ **The most each channel can have while its table and one spare packet fit.** Measured over
    /// both installs, every multi-channel container's full block is this size: 63 packets of each of
    /// four channels in 256, 127 of two, 31 of sixteen in 504.
    fn new(n: usize, block_size: usize) -> Result<Self, Refused> {
        if block_size == 0 || !block_size.is_multiple_of(MULTI_CHANNEL_PACKET) {
            return refuse(format!(
                "this stream's blocks are {block_size} bytes, which is not a whole number of \
                 2,048-byte packets"
            ));
        }
        let block_packets = block_size / MULTI_CHANNEL_PACKET;
        let Some(per_block) = (1..=block_packets / n)
            .rev()
            .find(|per| n * per + table_packets(n, *per) < block_packets)
        else {
            return refuse(format!(
                "this stream's {block_packets}-packet blocks are too small to hold {n} channels"
            ));
        };
        Ok(Self {
            n,
            block_packets,
            per_block,
        })
    }

    /// The blocks holding every channel's run, and the sample each block starts on.
    ///
    /// ⚠ **A block's `u32` at +0 is where the channel would start if the spare packets were dealt out
    /// to the first channels, 0 to `n` in the last block's case, and −1 in a container that never
    /// states it.** Nothing here reads it; it is written as the shipped containers carry it. The
    /// table's offsets are each packet's first sample in the whole channel; a block is zero-padded
    /// to the block size, except that the last ends where its last run does.
    fn blocks(
        &self,
        runs: &[Vec<u8>],
        samples: usize,
        per_packet: usize,
        words_unset: bool,
    ) -> (Vec<u8>, Vec<u32>) {
        let n = self.n;
        let packets = samples.div_ceil(per_packet);
        let blocks = packets.div_ceil(self.per_block);
        let spare = self.block_packets - table_packets(n, self.per_block) - 1;
        let (each, extra) = (spare / n, spare % n);
        let mut data = Vec::new();
        let mut starts = Vec::with_capacity(blocks);
        for b in 0..blocks {
            let first = b * self.per_block;
            let here = (packets - first).min(self.per_block);
            let from = first * per_packet;
            let held = (samples - from).min(here * per_packet);
            starts.push(from as u32);
            let last = b + 1 == blocks;
            let mut block = vec![0u8; table_packets(n, here) * MULTI_CHANNEL_PACKET];
            let mut offsets = n * 24;
            for c in 0..n {
                let start = match (words_unset, last) {
                    (true, _) => NO_FIRST_PACKET,
                    (false, true) => (c * here) as u32,
                    (false, false) => (c * each + c.min(extra)) as u32,
                };
                for (i, value) in [start, here as u32, 0, held as u32, 0, 0]
                    .into_iter()
                    .enumerate()
                {
                    block[c * 24 + i * 4..c * 24 + i * 4 + 4].copy_from_slice(&value.to_le_bytes());
                }
                for p in 0..here {
                    let first_sample = (from + p * per_packet) as u32;
                    block[offsets..offsets + 4].copy_from_slice(&first_sample.to_le_bytes());
                    offsets += 4;
                }
            }
            for run in runs {
                block.extend_from_slice(
                    &run[first * MULTI_CHANNEL_PACKET..(first + here) * MULTI_CHANNEL_PACKET],
                );
            }
            if !last {
                block.resize(self.block_packets * MULTI_CHANNEL_PACKET, 0);
            }
            data.extend_from_slice(&block);
        }
        (data, starts)
    }
}

#[cfg(test)]
mod tests {
    use super::super::opus::tests::{container, mono, packet, single_stream};
    use super::*;

    /// A container of one mono Opus stream of 40 packets, looped at `loop_point`, with a peak chunk
    /// and, when `levels`, a loudness chunk.
    fn shipped(loop_point: i32, levels: bool) -> AwcFile {
        let mut chunks = single_stream(&mono(40, 1), 1);
        chunks[2].1[0x04..0x08].copy_from_slice(&loop_point.to_le_bytes());
        if levels {
            chunks.push((0xB2, vec![0; 16]));
        }
        let bytes = container(0xFF02, &[], &[(0x0ABC, chunks)]);
        AwcFile::parse(&bytes, AwcKey::default()).expect("the container parses")
    }

    /// `frames` packets for `samples` per channel that lie 2,000 samples short of the frames.
    fn replacement(frames: usize) -> OpusStream {
        OpusStream {
            id: 0x0ABC,
            channels: 1,
            samples: OPUS_FRAME * frames as u32 - 2000,
            packets: mono(frames, 7),
        }
    }

    #[test]
    fn an_opus_stream_replaced_reads_back_its_packets_peaks_and_levels() {
        let new = replacement(70);
        let windows = opus_peak_windows(new.samples);
        let records = loudness_windows(new.samples);
        assert_eq!((windows, records), (new.samples as usize / 4096, 10));
        let peaks: Vec<u16> = (0..windows).map(|w| 100 + 3 * w as u16).collect();
        let levels: Vec<f32> = (0..records).map(|i| -30.0 + i as f32 / 4.0).collect();
        let file = shipped(-1, true);
        let edited = file
            .with_opus_replaced(0, &new, &peaks, &levels)
            .expect("replaced");
        let bytes = edited.to_bytes(AwcKey::default()).expect("written");
        let read = AwcFile::parse(&bytes, AwcKey::default()).expect("it reads back");
        assert_eq!(read.opus_streams(0).expect("split"), vec![new.clone()]);
        let s = &read.streams[0];
        let format = s.opus_format().expect("read").expect("present");
        let last = (new.samples - 1) as u16;
        assert_eq!(
            (format.samples, format.loop_point, format.peak),
            (new.samples, -1, 100)
        );
        assert_eq!((format.word_16, format.word_18), (0xFFFF, last));
        assert_eq!((format.stated_rate, format.word_0a), (32_000, -41));
        assert_eq!(s.peak_envelope(), Some(peaks[1..].to_vec()));
        let mut want = Vec::new();
        for (i, level) in levels.iter().enumerate() {
            want.extend((i as u32 * LOUDNESS_STEP).to_le_bytes());
            want.extend(level.to_le_bytes());
        }
        let loudness = s.chunk(AwcChunkKind::Loudness).expect("kept");
        assert_eq!(loudness.payload, want);
    }

    #[test]
    fn a_loop_point_is_kept_only_inside_the_new_samples() {
        let new = replacement(20);
        let peaks = vec![9; opus_peak_windows(new.samples)];
        for (stated, kept) in [(5000, Some(5000u32)), (new.samples as i32, None)] {
            let file = shipped(stated, false);
            let edited = file
                .with_opus_replaced(0, &new, &peaks, &[])
                .expect("replaced");
            let format = edited.streams[0]
                .opus_format()
                .expect("read")
                .expect("present");
            let word = kept.map_or(0xFFFF, |l| l as u16);
            assert_eq!(
                (format.loop_point, format.word_16),
                (kept.map_or(-1, |l| l as i32), word)
            );
        }
    }

    #[test]
    fn a_replaced_opus_stream_packs_a_frame_while_its_packet_holds_2046_bytes() {
        // Three frames of 510 bytes and their lengths fill 1,536; a fourth would fill 2,048.
        let new = OpusStream {
            id: 0x0ABC,
            channels: 1,
            samples: OPUS_FRAME * 7 - 2000,
            packets: (0..7).map(|i| packet(0xF8, 510, i)).collect(),
        };
        let peaks = vec![1; opus_peak_windows(new.samples)];
        let edited = shipped(-1, false)
            .with_opus_replaced(0, &new, &peaks, &[])
            .expect("replaced");
        let s = &edited.streams[0];
        let seek = &s.chunk(AwcChunkKind::SeekTable).expect("kept").payload;
        let starts: Vec<u32> = seek
            .chunks(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect();
        assert_eq!(starts, [0, 3, 6, 6].map(|f| f * OPUS_FRAME));
    }

    fn refusal(file: &AwcFile, new: &OpusStream, peaks: &[u16]) -> String {
        let refused = file.with_opus_replaced(0, new, peaks, &[]);
        format!("{}", refused.expect_err("refused"))
    }

    #[test]
    fn packets_the_stream_cannot_hold_are_refused() {
        let file = shipped(-1, false);
        let good = replacement(20);
        let peaks = vec![1; opus_peak_windows(good.samples)];
        let mut stereo_toc = good.clone();
        stereo_toc.packets[3] = packet(0xFC, 40, 1);
        let mut long = good.clone();
        long.packets[0] = packet(0xF8, MULTI_CHANNEL_PACKET - 1, 1);
        let mut short = good.clone();
        short.samples += 1000;
        let mut stereo = good.clone();
        stereo.channels = 2;
        for (new, want) in [
            (&stereo_toc, "not one 20 ms frame"),
            (&long, "2047 bytes"),
            (&short, "which is not 1920 to 2880 more"),
            (&stereo, "this stream has 1"),
        ] {
            let why = refusal(&file, new, &peaks);
            assert!(why.contains(want), "{want}: {why}");
        }
        let why = refusal(&file, &good, &peaks[1..]);
        assert!(why.contains("peak windows"), "{why}");
        let streams = [(1, vec![(0x48, vec![0; 12])])];
        let multi =
            AwcFile::parse(&container(0xFF00, &[], &streams), AwcKey::default()).expect("parses");
        let why = refusal(&multi, &good, &peaks);
        assert!(why.contains("no Opus format record"), "{why}");
    }
}
