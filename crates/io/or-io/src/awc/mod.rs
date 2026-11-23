//! `.awc` — the audio wave container.
//!
//! Not an RSC7 resource and not compressed: a 16-byte header, a table of streams, a table of
//! chunks with absolute file offsets, and the chunk payloads. Some files are
//! scrambled whole with [`crypto`]'s XXTEA variant before any of that is
//! readable.
//!
//! A chunk payload this does not interpret — `markers`, `animation`,
//! `gesture`, the granular pair, `mid` — is carried verbatim and round-trips,
//! and [`AwcChunkKind`] names it so a census can count it.
//!
//! Every structural claim the file makes about itself — a chunk that runs
//! past the end, a stream count that will not fit, a chunk table that
//! overlaps its own header — is a refusal. There is no lenient mode: a
//! truncated `.awc` has no partial reading worth having.

pub mod codec;
pub mod crypto;
#[cfg(feature = "gta6")]
mod gta6_chunks;
mod gta6_codecs;
pub mod ogg;
pub mod opus;
pub mod replace;
pub mod wav;

use crate::error::{IoError, Result};

pub use codec::{Accumulator, Codec};
pub use crypto::AwcKey;
#[cfg(feature = "gta6")]
pub use gta6_chunks::{markers, read_markers, track_ids, AwcLoudness, AwcMarker};

/// `ADAT`, little-endian.
pub const AWC_MAGIC_LE: u32 = 0x5441_4441;
/// `TADA` — the same four characters from a big-endian console build.
pub const AWC_MAGIC_BE: u32 = 0x4144_4154;

/// Bytes of file header before the stream table.
const HEADER_BYTES: usize = 16;

/// The chunk table's offset and size fields are 28 bits, so nothing beyond
/// this is addressable.
const MAX_ADDRESSABLE: usize = 0x0FFF_FFFF;

/// The unit a multi-channel block is laid out in: its table is padded to it
/// and each channel's run is a whole number of them.
const MULTI_CHANNEL_PACKET: usize = 2048;

/// Which chunk this is, by the last byte of the hash of its name.
///
/// ⚠ **The discriminant is a truncated hash, not an index.** Two chunk names
/// whose hashes ended in the same byte would collide and the format would
/// have no way to tell them apart, so this list is closed by the engine
/// rather than extensible by us. A byte outside it is carried as
/// [`AwcChunkKind::Other`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AwcChunkKind {
    Data,
    Format,
    Animation,
    Peak,
    Midi,
    Gesture,
    GranularGrains,
    GranularLoops,
    Markers,
    StreamFormat,
    SeekTable,
    /// GTA VI's 32-byte format record of an Opus stream; a role name, as the engine's is not known.
    OpusFormat,
    /// GTA VI's {`u32` position, `f32` value} records; a role name, as their being loudness is inferred.
    Loudness,
    Other(u8),
}

impl AwcChunkKind {
    pub fn from_byte(value: u8) -> Self {
        match value {
            0x55 => Self::Data,
            0xFA => Self::Format,
            0x5C => Self::Animation,
            0x36 => Self::Peak,
            0x68 => Self::Midi,
            0x2B => Self::Gesture,
            0x5A => Self::GranularGrains,
            0xD9 => Self::GranularLoops,
            0xBD => Self::Markers,
            0x48 => Self::StreamFormat,
            0xA3 => Self::SeekTable,
            0x76 => Self::OpusFormat,
            0xB2 => Self::Loudness,
            other => Self::Other(other),
        }
    }

    pub fn as_byte(self) -> u8 {
        match self {
            Self::Data => 0x55,
            Self::Format => 0xFA,
            Self::Animation => 0x5C,
            Self::Peak => 0x36,
            Self::Midi => 0x68,
            Self::Gesture => 0x2B,
            Self::GranularGrains => 0x5A,
            Self::GranularLoops => 0xD9,
            Self::Markers => 0xBD,
            Self::StreamFormat => 0x48,
            Self::SeekTable => 0xA3,
            Self::OpusFormat => 0x76,
            Self::Loudness => 0xB2,
            Self::Other(other) => other,
        }
    }

    pub fn label(self) -> String {
        match self {
            Self::Data => "data".into(),
            Self::Format => "format".into(),
            Self::Animation => "animation".into(),
            Self::Peak => "peak".into(),
            Self::Midi => "mid".into(),
            Self::Gesture => "gesture".into(),
            Self::GranularGrains => "granulargrains".into(),
            Self::GranularLoops => "granularloops".into(),
            Self::Markers => "markers".into(),
            Self::StreamFormat => "streamformat".into(),
            Self::SeekTable => "seektable".into(),
            Self::OpusFormat => "opusformat".into(),
            Self::Loudness => "loudness".into(),
            Self::Other(other) => format!("unknown(0x{other:02X})"),
        }
    }
}

/// One chunk: its kind, where the file put it, and its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwcChunk {
    pub kind: AwcChunkKind,
    /// Absolute file offset, as the chunk table states it.
    pub offset: u32,
    pub payload: Vec<u8>,
}

/// One stream: a 29-bit name hash and the chunks belonging to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwcStream {
    /// The stream's name hash. Zero marks the multi-channel source stream,
    /// which carries the interleaved data for every other stream in the file
    /// and is not itself playable.
    pub id: u32,
    pub chunks: Vec<AwcChunk>,
}

/// A `format` chunk: everything about one single-channel stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AwcFormat {
    pub samples: u32,
    pub loop_point: i32,
    pub samples_per_second: u16,
    pub headroom: i16,
    pub loop_begin: u16,
    pub loop_end: u16,
    pub play_end: u16,
    pub play_begin: u8,
    pub codec: Codec,
    /// Present only when the chunk is 24 bytes rather than 20. The low half
    /// is the first window of the peak envelope; the high half is unnamed.
    pub peak: Option<u32>,
}

impl AwcFormat {
    pub fn peak_value(&self) -> Option<u16> {
        self.peak.map(|p| (p & 0xFFFF) as u16)
    }
}

/// One channel of a multi-channel stream's `streamformat` chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AwcChannelFormat {
    pub id: u32,
    pub samples: u32,
    pub headroom: i16,
    pub samples_per_second: u16,
    pub codec: Codec,
}

/// A `streamformat` chunk: the block geometry of an interleaved stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwcStreamFormat {
    pub block_count: u32,
    pub block_size: u32,
    pub channels: Vec<AwcChannelFormat>,
}

impl AwcStream {
    pub fn chunk(&self, kind: AwcChunkKind) -> Option<&AwcChunk> {
        self.chunks.iter().find(|c| c.kind == kind)
    }

    pub fn format(&self) -> Option<AwcFormat> {
        let chunk = self.chunk(AwcChunkKind::Format)?;
        let b = &chunk.payload;
        if b.len() < 20 {
            return None;
        }
        let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let u16_at = |o: usize| u16::from_le_bytes(b[o..o + 2].try_into().unwrap());
        Some(AwcFormat {
            samples: u32_at(0),
            loop_point: u32_at(4) as i32,
            samples_per_second: u16_at(8),
            headroom: u16_at(10) as i16,
            loop_begin: u16_at(12),
            loop_end: u16_at(14),
            play_end: u16_at(16),
            play_begin: b[18],
            codec: Codec::from_byte(b[19]),
            peak: (b.len() >= 24).then(|| u32_at(20)),
        })
    }

    pub fn stream_format(&self) -> Option<AwcStreamFormat> {
        let chunk = self.chunk(AwcChunkKind::StreamFormat)?;
        let b = &chunk.payload;
        if b.len() < 12 {
            return None;
        }
        let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let count = u32_at(8) as usize;
        if b.len() < 12 + count * 16 {
            return None;
        }
        let channels = (0..count)
            .map(|i| {
                let o = 12 + i * 16;
                AwcChannelFormat {
                    id: u32_at(o),
                    samples: u32_at(o + 4),
                    headroom: u16::from_le_bytes(b[o + 8..o + 10].try_into().unwrap()) as i16,
                    samples_per_second: u16::from_le_bytes(b[o + 10..o + 12].try_into().unwrap()),
                    codec: Codec::from_byte(b[o + 12]),
                }
            })
            .collect();
        Some(AwcStreamFormat {
            block_count: u32_at(0),
            block_size: u32_at(4),
            channels,
        })
    }

    /// The `peak` chunk's envelope: one `u16` per 4,096 samples after the
    /// first window.
    pub fn peak_envelope(&self) -> Option<Vec<u16>> {
        let chunk = self.chunk(AwcChunkKind::Peak)?;
        Some(
            chunk
                .payload
                .as_chunks::<2>()
                .0
                .iter()
                .map(|w| u16::from_le_bytes([w[0], w[1]]))
                .collect(),
        )
    }
}

/// Flag bits 0 and 5, set together in every GTA VI container that has a first-chunk index.
// Which of the two adds the extra words is not known; GTA V's layout reads a container with
// bit 0 alone, as before.
pub const GTA6_TABLES: u16 = 0x21;

/// Flag bit 6 of a [`GTA6_TABLES`] container: one word before the first-chunk index.
const GTA6_BOUNDARY: u16 = 0x40;

/// Words a [`GTA6_TABLES`] container states beside its first-chunk index, carried as they are.
// What they mean is not known, so they are written back only beside the streams they were read with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwcExtraTables {
    /// The word flag bit 6 adds; a chunk starts there, and every stream lies on one side of it.
    pub boundary: Option<u32>,
    /// `streamCount + 1` words between the first-chunk index and the stream table.
    pub words: Vec<u32>,
}

impl AwcExtraTables {
    fn len(&self) -> usize {
        (usize::from(self.boundary.is_some()) + self.words.len()) * 4
    }
}

/// Where the stream table of a container with these flags and this many streams starts.
pub fn stream_table_offset(flags: u16, stream_count: usize) -> usize {
    let mut at = HEADER_BYTES;
    if flags & GTA6_TABLES == GTA6_TABLES {
        at += (stream_count + 1) * 4 + usize::from(flags & GTA6_BOUNDARY != 0) * 4;
    }
    if flags & 1 != 0 {
        at += stream_count * 2;
    }
    at
}

/// A parsed `.awc`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwcFile {
    pub version: u16,
    pub flags: u16,
    /// Whether the whole file was scrambled and had to be unscrambled first.
    pub whole_file_encrypted: bool,
    /// Present only when [`AwcFile::has_chunk_indices`]: the index of each
    /// stream's first chunk.
    pub chunk_indices: Vec<u16>,
    pub streams: Vec<AwcStream>,
    /// GTA VI's tables around the first-chunk index, present when [`GTA6_TABLES`] are both set.
    extra: Option<AwcExtraTables>,
    /// Bytes between the end of one chunk and the start of the next, kept so
    /// a write reproduces the file's own padding rather than inventing it.
    padding: Vec<(u32, Vec<u8>)>,
    /// Length of the plaintext the parse saw, which is the encrypted form's
    /// length rounded up to a word.
    plaintext_len: usize,
}

impl AwcFile {
    pub fn has_chunk_indices(&self) -> bool {
        self.flags & 1 != 0
    }
    pub fn single_channel_encrypted(&self) -> bool {
        self.flags & 2 != 0
    }
    pub fn multi_channel(&self) -> bool {
        self.flags & 4 != 0
    }
    pub fn multi_channel_encrypted(&self) -> bool {
        self.flags & 8 != 0
    }

    /// A stream's `data` payload, descrambled when the file's flags say it
    /// is scrambled.
    ///
    /// ⚠ **There are two independent scramblings and only one applies at a
    /// time.** A whole-file-encrypted container is already plaintext by the
    /// time it is parsed, and its data chunks are *not* scrambled again; a
    /// container that is not encrypted whole may still scramble each data
    /// chunk under the same key. Descrambling both, or neither, decodes to
    /// noise that still has the right length and the right sample count —
    /// which is why this is checked against the peaks the encoder recorded
    /// rather than against the reader.
    ///
    /// Descrambling lives here rather than in the parse because stored
    /// payloads stay exactly as the file has them, which is what keeps
    /// [`AwcFile::to_bytes`] byte-identical.
    pub fn stream_data(&self, stream: &AwcStream, key: AwcKey) -> Option<Vec<u8>> {
        let chunk = stream.chunk(AwcChunkKind::Data)?;
        // GTA VI sets flag bit 1 and stores its Opus data in the clear.
        if self.whole_file_encrypted || opus::is_opus(stream) {
            return Some(chunk.payload.clone());
        }
        if self.multi_channel() {
            if !self.multi_channel_encrypted() {
                return Some(chunk.payload.clone());
            }
            // Each block is its own cipher message, so a run of them cannot
            // be descrambled as one.
            let format = stream.stream_format()?;
            let block = format.block_size as usize;
            if block == 0 || !key.is_usable() {
                return None;
            }
            let mut out = chunk.payload.clone();
            for start in (0..out.len()).step_by(block) {
                let end = (start + block).min(out.len());
                crypto::decrypt(key, &mut out[start..end]);
            }
            return Some(out);
        }
        if !self.single_channel_encrypted() {
            return Some(chunk.payload.clone());
        }
        if !key.is_usable() {
            return None;
        }
        let mut out = chunk.payload.clone();
        crypto::decrypt(key, &mut out);
        Some(out)
    }

    /// PCM16 for a single-channel stream, or `None` when the stream has no
    /// `format` + `data` pair, its data cannot be descrambled, or its codec
    /// has no decode.
    pub fn decode_stream(
        &self,
        stream: &AwcStream,
        key: AwcKey,
        accumulator: Accumulator,
    ) -> Option<Vec<u8>> {
        let format = stream.format()?;
        let data = self.stream_data(stream, key)?;
        codec::decode(&data, format.samples as usize, format.codec, accumulator)
    }

    /// PCM16 for each channel of a multi-channel container, in `streamformat`
    /// order, or `None` when there is no `streamformat` + `data` pair, the
    /// data cannot be descrambled, a channel's codec has no decode, or a
    /// block's layout does not fit the block.
    ///
    /// ⚠ **Each block holds every channel's run whole, not samples
    /// alternating.** A block opens with one 24-byte header per channel
    /// (`u32` at +4 is its count of 2,048-byte packets, `u32` at +12 its
    /// sample count), then each channel's `u32` per-packet sample offsets,
    /// padded to 2,048; the channels' packets follow in `streamformat` order.
    ///
    /// ⚠ **An MP3 channel's run is not `packetCount` packets.** It is the
    /// `u32` at +20 bytes of frames, each channel's run starting on 16 bytes;
    /// the block's samples are those its frames decode to from the `u32` at
    /// +8 to the one at +12, the first being a repeat of the previous block's
    /// last. The +12 count is signed: one container's last block states
    /// −450, taking back what the block before it overran. Measured over
    /// Enhanced, those runs end on a frame boundary and the kept samples sum
    /// to the channel's count, which is a refusal otherwise.
    pub fn decode_channels(&self, key: AwcKey, accumulator: Accumulator) -> Option<Vec<Vec<u8>>> {
        let source = self.streams.iter().find(|s| s.stream_format().is_some())?;
        let format = source.stream_format()?;
        let data = self.stream_data(source, key)?;
        let block_size = format.block_size as usize;
        let count = format.channels.len();
        if block_size == 0 || count == 0 {
            return None;
        }
        let mut channels = vec![Vec::new(); count];
        let mut mp3 = vec![nanomp3::Decoder::new(); count];
        for block in data.chunks(block_size) {
            let u32_at = |o: usize| -> Option<usize> {
                Some(u32::from_le_bytes(block.get(o..o + 4)?.try_into().ok()?) as usize)
            };
            let mut heads = Vec::with_capacity(count);
            for c in 0..count {
                heads.push((u32_at(c * 24 + 4)?, u32_at(c * 24 + 12)?));
            }
            let table: usize =
                count * 24 + heads.iter().map(|(packets, _)| packets * 4).sum::<usize>();
            let mut at = table.div_ceil(MULTI_CHANNEL_PACKET) * MULTI_CHANNEL_PACKET;
            for (c, (packets, samples)) in heads.into_iter().enumerate() {
                if format.channels[c].codec == Codec::Mp3 {
                    let (repeat, bytes) = (u32_at(c * 24 + 8)?, u32_at(c * 24 + 20)?);
                    let pcm = codec::decode_mp3_frames(&mut mp3[c], block.get(at..at + bytes)?)?;
                    // A count below the repeat takes samples back from the
                    // previous block, which overran the channel's end.
                    match (samples as u32 as i32).checked_sub(repeat as i32)? {
                        keep @ 0.. => channels[c]
                            .extend_from_slice(pcm.get(repeat * 2..(repeat + keep as usize) * 2)?),
                        back => {
                            let len = channels[c]
                                .len()
                                .checked_sub(back.unsigned_abs() as usize * 2)?;
                            channels[c].truncate(len);
                        }
                    }
                    at = (at + bytes).next_multiple_of(16);
                    continue;
                }
                let end = at + packets * MULTI_CHANNEL_PACKET;
                let pcm = codec::decode(
                    block.get(at..end)?,
                    samples,
                    format.channels[c].codec,
                    accumulator,
                )?;
                channels[c].extend_from_slice(&pcm);
                at = end;
            }
        }
        for (out, channel) in channels.iter_mut().zip(&format.channels) {
            let stated = channel.samples as usize * 2;
            if channel.codec == Codec::Mp3 && out.len() != stated {
                return None;
            }
            out.resize(stated, 0);
        }
        Some(channels)
    }

    pub fn parse(data: &[u8], key: AwcKey) -> Result<Self> {
        if data.len() < HEADER_BYTES {
            return Err(refuse(format!(
                "{} bytes is shorter than the {HEADER_BYTES}-byte header",
                data.len()
            )));
        }

        let (plain, whole_file_encrypted) = match magic_of(data) {
            Some(AWC_MAGIC_BE) => {
                return Err(refuse(
                    "big-endian `TADA`: this is a console build's container, and only \
                     the little-endian PC form is read"
                        .to_string(),
                ))
            }
            Some(_) => (data.to_vec(), false),
            None => {
                if !key.is_usable() {
                    return Err(refuse(
                        "no `ADAT` magic, so the file is scrambled, and no AWC key is \
                         available to unscramble it — re-extract keys from your game \
                         executable, which carries it"
                            .to_string(),
                    ));
                }
                if !data.len().is_multiple_of(4) {
                    return Err(refuse(format!(
                        "no `ADAT` magic and {} bytes is not a whole number of cipher \
                         words, so this is neither a plain nor a scrambled container",
                        data.len()
                    )));
                }
                let mut plain = data.to_vec();
                crypto::decrypt(key, &mut plain);
                if magic_of(&plain) != Some(AWC_MAGIC_LE) {
                    return Err(refuse(
                        "unscrambling did not produce `ADAT`; either this is not an \
                         `.awc` or the AWC key does not describe this build"
                            .to_string(),
                    ));
                }
                (plain, true)
            }
        };

        let mut file = Self::parse_plain(&plain)?;
        file.whole_file_encrypted = whole_file_encrypted;
        Ok(file)
    }

    fn parse_plain(data: &[u8]) -> Result<Self> {
        let u16_at = |o: usize| u16::from_le_bytes(data[o..o + 2].try_into().unwrap());
        let u32_at = |o: usize| u32::from_le_bytes(data[o..o + 4].try_into().unwrap());

        let version = u16_at(4);
        let flags = u16_at(6);
        let stream_count = u32_at(8) as usize;
        let declared_data_offset = u32_at(12) as usize;

        // A stream costs 4 bytes of table plus at least one chunk's 8, so a
        // count the file cannot hold is a refusal before any allocation.
        let floor = HEADER_BYTES + stream_count * 4;
        if stream_count > data.len() / 4 || floor > data.len() {
            return Err(refuse(format!(
                "{stream_count} stream(s) will not fit in {} bytes",
                data.len()
            )));
        }

        let gta6 = flags & GTA6_TABLES == GTA6_TABLES;
        let mut at = HEADER_BYTES;
        let mut extra = None;
        if gta6 {
            let boundary = flags & GTA6_BOUNDARY != 0;
            if at + usize::from(boundary) * 4 + stream_count * 6 + 4 > data.len() {
                return Err(refuse(
                    "the tables before the stream table run past the end".into(),
                ));
            }
            extra = Some(AwcExtraTables {
                boundary: boundary.then(|| u32_at(at)),
                words: Vec::new(),
            });
            at += usize::from(boundary) * 4;
        }
        let mut chunk_indices = Vec::new();
        if flags & 1 != 0 {
            if at + stream_count * 2 > data.len() {
                return Err(refuse("the chunk-index table runs past the end".into()));
            }
            chunk_indices = (0..stream_count).map(|i| u16_at(at + i * 2)).collect();
            at += stream_count * 2;
        }
        if let Some(extra) = &mut extra {
            extra.words = (0..=stream_count).map(|i| u32_at(at + i * 4)).collect();
            at += (stream_count + 1) * 4;
        }

        if at + stream_count * 4 > data.len() {
            return Err(refuse("the stream table runs past the end".into()));
        }
        let mut heads = Vec::with_capacity(stream_count);
        for i in 0..stream_count {
            let raw = u32_at(at + i * 4);
            heads.push(((raw >> 29) as usize, raw & 0x1FFF_FFFF));
        }
        at += stream_count * 4;

        let total_chunks: usize = heads.iter().map(|(n, _)| n).sum();
        if at + total_chunks * 8 > data.len() {
            return Err(refuse(format!(
                "the chunk table needs {} bytes and only {} remain",
                total_chunks * 8,
                data.len() - at
            )));
        }

        let mut streams = Vec::with_capacity(stream_count);
        let mut spans: Vec<(usize, usize)> = Vec::with_capacity(total_chunks);
        for (chunk_count, id) in heads {
            let mut chunks = Vec::with_capacity(chunk_count);
            for _ in 0..chunk_count {
                let raw = u64::from_le_bytes(data[at..at + 8].try_into().unwrap());
                at += 8;
                let kind = AwcChunkKind::from_byte((raw >> 56) as u8);
                let size = ((raw >> 28) & 0x0FFF_FFFF) as usize;
                let offset = (raw & 0x0FFF_FFFF) as usize;
                if offset > MAX_ADDRESSABLE || offset + size > data.len() {
                    return Err(refuse(format!(
                        "a {} chunk claims {size} bytes at {offset:#x}, past the end of \
                         a {}-byte file",
                        kind.label(),
                        data.len()
                    )));
                }
                spans.push((offset, size));
                chunks.push(AwcChunk {
                    kind,
                    offset: offset as u32,
                    payload: data[offset..offset + size].to_vec(),
                });
            }
            streams.push(AwcStream { id, chunks });
        }

        if at > declared_data_offset {
            return Err(refuse(format!(
                "the tables end at {at:#x}, past the declared data offset \
                 {declared_data_offset:#x}"
            )));
        }
        if gta6 {
            if at != declared_data_offset {
                return Err(refuse(format!(
                    "the GTA VI tables end at {at:#x}, not at the declared data offset \
                     {declared_data_offset:#x}"
                )));
            }
            let mut first = 0usize;
            for (i, (index, stream)) in chunk_indices.iter().zip(&streams).enumerate() {
                if usize::from(*index) != first {
                    return Err(refuse(format!(
                        "stream {i}'s first-chunk index is {index}, not the {first} chunks \
                         before it"
                    )));
                }
                first += stream.chunks.len();
            }
        }

        Ok(Self {
            version,
            flags,
            whole_file_encrypted: false,
            chunk_indices,
            extra,
            streams,
            padding: gaps_between(data, at, &mut spans),
            plaintext_len: data.len(),
        })
    }

    /// Bytes the chunk table and the header do not account for.
    ///
    /// ⚠ **The only verification a container with no checksum admits of.**
    /// Every byte of the plaintext is either header, table, a chunk payload
    /// or inter-chunk padding, so a reader that reaches zero here has read
    /// the whole file rather than the part it understood.
    pub fn unaccounted_bytes(&self) -> usize {
        let tables = HEADER_BYTES
            + self.extra.as_ref().map_or(0, AwcExtraTables::len)
            + self.chunk_indices.len() * 2
            + self.streams.len() * 4
            + self.streams.iter().map(|s| s.chunks.len()).sum::<usize>() * 8;
        let payloads: usize = self
            .streams
            .iter()
            .flat_map(|s| &s.chunks)
            .map(|c| c.payload.len())
            .sum();
        let padding: usize = self.padding.iter().map(|(_, bytes)| bytes.len()).sum();
        self.plaintext_len
            .saturating_sub(tables + payloads + padding)
    }

    /// The plaintext length the parse saw.
    pub fn plaintext_len(&self) -> usize {
        self.plaintext_len
    }

    /// Replace one chunk's payload, moving every byte after it so the file stays whole.
    ///
    /// ⚠ What follows moves by a multiple of 16 bytes and the gap is zeroed,
    /// so whatever alignment the file kept for later chunks survives; the
    /// payload is stored as given, so a scrambled stream's caller scrambles it.
    pub fn set_chunk_payload(
        &mut self,
        stream: usize,
        kind: AwcChunkKind,
        payload: Vec<u8>,
    ) -> Result<()> {
        let index = self
            .streams
            .get(stream)
            .and_then(|s| s.chunks.iter().position(|c| c.kind == kind))
            .ok_or_else(|| refuse(format!("stream {stream} has no {} chunk", kind.label())))?;
        let chunk = &self.streams[stream].chunks[index];
        let start = chunk.offset as usize;
        let old_end = start + chunk.payload.len();
        let new_end = start + payload.len();
        let shift = (new_end as i64 - old_end as i64 + 15).div_euclid(16) * 16;
        let moved = |offset: usize| (offset as i64 + shift) as usize;

        for (s, other) in self.streams.iter().enumerate() {
            for (c, other) in other.chunks.iter().enumerate() {
                let at = other.offset as usize;
                if (s, c) != (stream, index) && at > start && at < old_end {
                    return Err(refuse(format!(
                        "a {} chunk at {at:#x} lies inside the {} chunk being replaced",
                        other.kind.label(),
                        kind.label()
                    )));
                }
                let end = if (s, c) == (stream, index) {
                    new_end
                } else if at >= old_end {
                    moved(at) + other.payload.len()
                } else {
                    at + other.payload.len()
                };
                if end > MAX_ADDRESSABLE {
                    return Err(refuse(format!(
                        "a {} chunk would end at {end:#x}, past the 28-bit offset field",
                        other.kind.label()
                    )));
                }
            }
        }

        for (s, other) in self.streams.iter_mut().enumerate() {
            for (c, other) in other.chunks.iter_mut().enumerate() {
                if (s, c) != (stream, index) && other.offset as usize >= old_end {
                    other.offset = moved(other.offset as usize) as u32;
                }
            }
        }
        self.streams[stream].chunks[index].payload = payload;
        for (offset, _) in &mut self.padding {
            if *offset as usize >= old_end {
                *offset = moved(*offset as usize) as u32;
            }
        }
        let gap_end = moved(old_end);
        if gap_end > new_end {
            self.padding
                .push((new_end as u32, vec![0; gap_end - new_end]));
        }
        self.padding.sort_unstable_by_key(|(offset, _)| *offset);
        let mut merged: Vec<(u32, Vec<u8>)> = Vec::with_capacity(self.padding.len());
        for (offset, bytes) in self.padding.drain(..) {
            match merged.last_mut() {
                Some((at, run)) if *at as usize + run.len() == offset as usize => {
                    run.extend_from_slice(&bytes)
                }
                _ => merged.push((offset, bytes)),
            }
        }
        self.padding = merged;
        self.plaintext_len = moved(self.plaintext_len);
        Ok(())
    }

    /// Re-encode. Chunks are written back at the offsets the file stated, so
    /// a file that parsed reproduces itself byte for byte.
    pub fn to_bytes(&self, key: AwcKey) -> Result<Vec<u8>> {
        if let Some(extra) = &self.extra {
            let mut first = 0;
            let indices_hold = self.chunk_indices.len() == self.streams.len()
                && (self.chunk_indices.iter().zip(&self.streams)).all(|(index, stream)| {
                    let holds = usize::from(*index) == first;
                    first += stream.chunks.len();
                    holds
                });
            if extra.words.len() != self.streams.len() + 1 || !indices_hold {
                return Err(refuse(
                    "a GTA VI container is written only with the streams and chunks it was \
                     read with: what the words beside its first-chunk index mean is not known"
                        .to_string(),
                ));
            }
        }
        let mut out = vec![0u8; self.plaintext_len];
        out[0..4].copy_from_slice(&AWC_MAGIC_LE.to_le_bytes());
        out[4..6].copy_from_slice(&self.version.to_le_bytes());
        out[6..8].copy_from_slice(&self.flags.to_le_bytes());
        out[8..12].copy_from_slice(&(self.streams.len() as u32).to_le_bytes());

        let mut at = HEADER_BYTES;
        if let Some(boundary) = self.extra.as_ref().and_then(|e| e.boundary) {
            out[at..at + 4].copy_from_slice(&boundary.to_le_bytes());
            at += 4;
        }
        if self.has_chunk_indices() {
            for index in &self.chunk_indices {
                out[at..at + 2].copy_from_slice(&index.to_le_bytes());
                at += 2;
            }
        }
        for word in self.extra.iter().flat_map(|e| &e.words) {
            out[at..at + 4].copy_from_slice(&word.to_le_bytes());
            at += 4;
        }
        for stream in &self.streams {
            let raw = (stream.id & 0x1FFF_FFFF) | ((stream.chunks.len() as u32) << 29);
            out[at..at + 4].copy_from_slice(&raw.to_le_bytes());
            at += 4;
        }
        for chunk in self.streams.iter().flat_map(|s| &s.chunks) {
            let raw = (chunk.offset as u64 & 0x0FFF_FFFF)
                | ((chunk.payload.len() as u64 & 0x0FFF_FFFF) << 28)
                | ((chunk.kind.as_byte() as u64) << 56);
            out[at..at + 8].copy_from_slice(&raw.to_le_bytes());
            at += 8;
        }
        out[12..16].copy_from_slice(&(self.declared_data_offset().to_le_bytes()));

        for chunk in self.streams.iter().flat_map(|s| &s.chunks) {
            let start = chunk.offset as usize;
            let end = start + chunk.payload.len();
            if end > out.len() {
                return Err(refuse(format!(
                    "a {} chunk at {start:#x} does not fit the {}-byte file it came from",
                    chunk.kind.label(),
                    out.len()
                )));
            }
            out[start..end].copy_from_slice(&chunk.payload);
        }
        for (offset, bytes) in &self.padding {
            let start = *offset as usize;
            out[start..start + bytes.len()].copy_from_slice(bytes);
        }

        if self.whole_file_encrypted {
            if !key.is_usable() {
                return Err(refuse(
                    "this file was scrambled and no AWC key is available to scramble it \
                     again"
                        .to_string(),
                ));
            }
            crypto::encrypt(key, &mut out);
        }
        Ok(out)
    }

    /// The `dataOffset` field, reproduced from where the tables end.
    fn declared_data_offset(&self) -> u32 {
        (HEADER_BYTES
            + self.extra.as_ref().map_or(0, AwcExtraTables::len)
            + self.chunk_indices.len() * 2
            + self.streams.len() * 4
            + self.streams.iter().map(|s| s.chunks.len()).sum::<usize>() * 8) as u32
    }
}

/// The magic word, when the first four bytes are one.
fn magic_of(data: &[u8]) -> Option<u32> {
    if data.len() < 4 {
        return None;
    }
    let magic = u32::from_le_bytes(data[0..4].try_into().unwrap());
    (magic == AWC_MAGIC_LE || magic == AWC_MAGIC_BE).then_some(magic)
}

/// The runs of bytes between the end of the tables and the end of the file
/// that no chunk covers.
fn gaps_between(
    data: &[u8],
    tables_end: usize,
    spans: &mut [(usize, usize)],
) -> Vec<(u32, Vec<u8>)> {
    spans.sort_unstable();
    let mut gaps = Vec::new();
    let mut cursor = tables_end;
    for (offset, size) in spans.iter() {
        if *offset > cursor {
            gaps.push((cursor as u32, data[cursor..*offset].to_vec()));
        }
        cursor = cursor.max(offset + size);
    }
    if cursor < data.len() {
        gaps.push((cursor as u32, data[cursor..].to_vec()));
    }
    gaps
}

fn refuse(why: String) -> IoError {
    IoError::InvalidData {
        context: format!("awc: {why}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_key() -> AwcKey {
        AwcKey([0xA1B2_C3D4, 0x1122_3344, 0x5566_7788, 0x99AA_BBCC])
    }

    /// One stream, a 20-byte `format` and a PCM `data` chunk, built by hand
    /// so nothing in the test came out of the reader.
    fn synthetic(samples: u32, codec: u8) -> Vec<u8> {
        let stream_count = 1usize;
        let tables = 16 + stream_count * 4 + 2 * 8;
        let format_at = tables;
        let data_at = format_at + 20;
        let data_len = samples as usize * 2;
        let mut out = vec![0u8; data_at + data_len];

        out[0..4].copy_from_slice(&AWC_MAGIC_LE.to_le_bytes());
        out[4..6].copy_from_slice(&1u16.to_le_bytes());
        out[6..8].copy_from_slice(&0xFF00u16.to_le_bytes());
        out[8..12].copy_from_slice(&(stream_count as u32).to_le_bytes());
        out[12..16].copy_from_slice(&(tables as u32).to_le_bytes());
        out[16..20].copy_from_slice(&(0x0BAD_F00Du32 & 0x1FFF_FFFF | (2 << 29)).to_le_bytes());

        let pack = |kind: u8, size: usize, offset: usize| -> u64 {
            (offset as u64 & 0x0FFF_FFFF)
                | ((size as u64 & 0x0FFF_FFFF) << 28)
                | ((kind as u64) << 56)
        };
        out[20..28].copy_from_slice(&pack(0xFA, 20, format_at).to_le_bytes());
        out[28..36].copy_from_slice(&pack(0x55, data_len, data_at).to_le_bytes());

        out[format_at..format_at + 4].copy_from_slice(&samples.to_le_bytes());
        out[format_at + 8..format_at + 10].copy_from_slice(&48000u16.to_le_bytes());
        out[format_at + 19] = codec;

        for i in 0..samples as usize {
            let sample = ((i as i32 * 137) % 20000 - 10000) as i16;
            let at = data_at + i * 2;
            out[at..at + 2].copy_from_slice(&sample.to_le_bytes());
        }
        out
    }

    #[test]
    fn a_hand_built_container_reads_back_what_was_put_in_it() {
        let bytes = synthetic(64, 0);
        let awc = AwcFile::parse(&bytes, AwcKey::default()).expect("parse");

        assert_eq!(awc.version, 1);
        assert_eq!(awc.streams.len(), 1);
        assert!(!awc.whole_file_encrypted);
        let stream = &awc.streams[0];
        assert_eq!(stream.id, 0x0BAD_F00D & 0x1FFF_FFFF);
        assert_eq!(stream.chunks.len(), 2);

        let format = stream.format().expect("format chunk");
        assert_eq!(format.samples, 64);
        assert_eq!(format.samples_per_second, 48000);
        assert_eq!(format.codec, Codec::Pcm16);
        assert_eq!(format.peak, None);
        assert_eq!(awc.unaccounted_bytes(), 0);
    }

    /// A file the reader never touched.
    #[test]
    fn a_container_written_from_scratch_reloads_and_matches() {
        let awc = AwcFile {
            version: 1,
            flags: 0xFF00,
            whole_file_encrypted: false,
            chunk_indices: Vec::new(),
            extra: None,
            streams: vec![AwcStream {
                id: 0x0012_3456,
                chunks: vec![AwcChunk {
                    kind: AwcChunkKind::Format,
                    offset: 28,
                    payload: {
                        let mut p = vec![0u8; 20];
                        p[0..4].copy_from_slice(&16u32.to_le_bytes());
                        p[8..10].copy_from_slice(&24000u16.to_le_bytes());
                        p[19] = 4;
                        p
                    },
                }],
            }],
            padding: Vec::new(),
            plaintext_len: 48,
        };
        let bytes = awc.to_bytes(AwcKey::default()).expect("write");
        let back = AwcFile::parse(&bytes, AwcKey::default()).expect("reload");
        assert_eq!(back, awc);
        assert_eq!(back.streams[0].format().unwrap().codec, Codec::ImaAdpcm);
    }

    #[test]
    fn a_scrambled_container_round_trips_through_the_cipher() {
        let plain = synthetic(32, 0);
        let mut scrambled = plain.clone();
        crypto::encrypt(a_key(), &mut scrambled);
        assert_ne!(magic_of(&scrambled), Some(AWC_MAGIC_LE));

        let awc = AwcFile::parse(&scrambled, a_key()).expect("parse scrambled");
        assert!(awc.whole_file_encrypted);
        assert_eq!(awc.streams.len(), 1);
        assert_eq!(awc.to_bytes(a_key()).expect("rewrite"), scrambled);
    }

    /// ⚠ Without the key the refusal must name the key, not the file. A
    /// scrambled `.awc` and a corrupt one look identical from outside.
    #[test]
    fn a_scrambled_container_without_a_key_says_which_is_missing() {
        let mut scrambled = synthetic(32, 0);
        crypto::encrypt(a_key(), &mut scrambled);
        let err = AwcFile::parse(&scrambled, AwcKey::default()).expect_err("no key");
        let message = format!("{err}");
        assert!(message.contains("key"), "{message}");
        assert!(message.contains("executable"), "{message}");

        // And the wrong key is a different message from no key at all.
        let wrong = AwcFile::parse(&scrambled, AwcKey([1, 2, 3, 4])).expect_err("wrong key");
        assert!(format!("{wrong}").contains("did not produce"), "{wrong}");
    }

    #[test]
    fn a_console_container_is_refused_by_name_rather_than_misread() {
        let mut bytes = synthetic(16, 0);
        bytes[0..4].copy_from_slice(&AWC_MAGIC_BE.to_le_bytes());
        let err = AwcFile::parse(&bytes, a_key()).expect_err("big-endian");
        assert!(format!("{err}").contains("console"), "{err}");
    }

    /// Every structural lie a file can tell about itself is a refusal, and
    /// each produces a different sentence.
    #[test]
    fn a_container_that_does_not_fit_itself_is_refused() {
        let good = synthetic(32, 0);

        let mut truncated = good.clone();
        truncated.truncate(30);
        assert!(AwcFile::parse(&truncated, a_key()).is_err());

        let mut wild_count = good.clone();
        wild_count[8..12].copy_from_slice(&0x00FF_FFFFu32.to_le_bytes());
        let err = AwcFile::parse(&wild_count, a_key()).expect_err("stream count");
        assert!(format!("{err}").contains("will not fit"), "{err}");

        let mut wild_chunk = good.clone();
        let raw = (0x0FFF_FFF0u64) | ((64u64) << 28) | ((0x55u64) << 56);
        wild_chunk[28..36].copy_from_slice(&raw.to_le_bytes());
        let err = AwcFile::parse(&wild_chunk, a_key()).expect_err("chunk span");
        assert!(format!("{err}").contains("past the end"), "{err}");

        assert!(AwcFile::parse(&[], a_key()).is_err());
        assert!(AwcFile::parse(&[0u8; 8], a_key()).is_err());
    }

    /// ⚠ Padding is preserved rather than zeroed, because a write that
    /// zeroes a byte it did not understand is not a round-trip.
    #[test]
    fn bytes_no_chunk_covers_survive_a_rewrite() {
        let mut bytes = synthetic(16, 0);
        // Push the data chunk four bytes later and fill the gap with
        // something a zeroing writer would lose.
        let tables = 16 + 4 + 16;
        let data_at = tables + 20;
        let data_len = 32;
        let mut moved = bytes[..data_at].to_vec();
        moved.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        moved.extend_from_slice(&bytes[data_at..]);
        let raw =
            ((data_at + 4) as u64 & 0x0FFF_FFFF) | ((data_len as u64) << 28) | ((0x55u64) << 56);
        moved[28..36].copy_from_slice(&raw.to_le_bytes());
        bytes = moved;

        let awc = AwcFile::parse(&bytes, a_key()).expect("parse");
        assert_eq!(awc.unaccounted_bytes(), 0);
        assert_eq!(awc.to_bytes(a_key()).expect("rewrite"), bytes);
    }

    /// A grown or shrunk chunk moves what follows by whole 16-byte steps, and
    /// what the write produces parses back to the edited value.
    #[test]
    fn a_replaced_chunk_moves_what_follows_it_and_the_file_reparses() {
        // The format chunk first, so the data chunk after it has to move.
        let original = synthetic(16, 0);
        for len in [20usize, 21, 37, 4, 0] {
            let mut awc = AwcFile::parse(&original, a_key()).expect("parse");
            let data_at = awc.streams[0].chunk(AwcChunkKind::Data).unwrap().offset;
            let payload: Vec<u8> = (0..len as u8).collect();
            awc.set_chunk_payload(0, AwcChunkKind::Format, payload.clone())
                .expect("replace");

            let moved = awc.streams[0].chunk(AwcChunkKind::Data).unwrap().offset;
            assert_eq!((moved as i64 - data_at as i64).rem_euclid(16), 0, "{len}");
            assert!(moved as usize >= 36 + len, "{len}: the data chunk overlaps");
            let bytes = awc.to_bytes(a_key()).expect("write");
            let back = AwcFile::parse(&bytes, a_key()).expect("reparse");
            assert_eq!(back, awc, "{len}");
            assert_eq!(back.unaccounted_bytes(), 0);
            assert_eq!(
                back.streams[0].chunk(AwcChunkKind::Format).unwrap().payload,
                payload
            );
            assert_eq!(
                back.streams[0].chunk(AwcChunkKind::Data).unwrap().payload,
                original[56..].to_vec()
            );
        }
        let mut awc = AwcFile::parse(&original, a_key()).expect("parse");
        assert!(awc
            .set_chunk_payload(0, AwcChunkKind::Peak, Vec::new())
            .is_err());
    }

    #[test]
    fn a_chunk_byte_the_table_does_not_name_keeps_its_number() {
        assert_eq!(AwcChunkKind::from_byte(0x55), AwcChunkKind::Data);
        assert_eq!(AwcChunkKind::from_byte(0x11), AwcChunkKind::Other(0x11));
        for byte in [0x55u8, 0xFA, 0xA3, 0x11, 0x00] {
            assert_eq!(AwcChunkKind::from_byte(byte).as_byte(), byte);
        }
        assert!(AwcChunkKind::from_byte(0x11).label().contains("0x11"));
    }

    /// ⚠ The two scramblings are mutually exclusive and a reader that
    /// applies the wrong one produces noise of exactly the right length.
    /// This is the in-process half of the check the corpus gate makes.
    #[test]
    fn a_stream_scrambled_on_its_own_is_descrambled_and_a_whole_file_one_is_not() {
        // Flag 2 set, whole file plain: the data chunk is scrambled.
        let mut bytes = synthetic(64, 0);
        bytes[6..8].copy_from_slice(&(0xFF00u16 | 2).to_le_bytes());
        let data_at = 16 + 4 + 16 + 20;
        crypto::encrypt(a_key(), &mut bytes[data_at..]);

        let awc = AwcFile::parse(&bytes, a_key()).expect("parse");
        assert!(awc.single_channel_encrypted());
        let pcm = awc
            .decode_stream(&awc.streams[0], a_key(), Accumulator::Clamped)
            .expect("pcm");
        for i in 0..64usize {
            let want = ((i as i32 * 137) % 20000 - 10000) as i16;
            assert_eq!(
                i16::from_le_bytes([pcm[i * 2], pcm[i * 2 + 1]]),
                want,
                "sample {i}"
            );
        }
        // Without the key the refusal is a `None`, not noise.
        assert!(awc
            .decode_stream(&awc.streams[0], AwcKey::default(), Accumulator::Clamped)
            .is_none());
        // And the stored payload is still the file's own bytes.
        assert_eq!(awc.to_bytes(a_key()).expect("rewrite"), bytes);

        // Flag 2 set *and* whole-file encrypted: the data chunk is plain
        // once the file is, so descrambling again would be the defect.
        let mut whole = bytes.clone();
        crypto::decrypt(a_key(), &mut whole[data_at..]);
        crypto::encrypt(a_key(), &mut whole);
        let awc = AwcFile::parse(&whole, a_key()).expect("parse");
        assert!(awc.whole_file_encrypted && awc.single_channel_encrypted());
        let pcm = awc
            .decode_stream(&awc.streams[0], a_key(), Accumulator::Clamped)
            .expect("pcm");
        assert_eq!(i16::from_le_bytes([pcm[0], pcm[1]]), -10000);
    }

    #[test]
    fn a_pcm_stream_decodes_to_the_samples_it_was_built_from() {
        let bytes = synthetic(64, 0);
        let awc = AwcFile::parse(&bytes, a_key()).expect("parse");
        let pcm = awc
            .decode_stream(&awc.streams[0], a_key(), Accumulator::Clamped)
            .expect("pcm");
        assert_eq!(pcm.len(), 128);
        for i in 0..64usize {
            let want = ((i as i32 * 137) % 20000 - 10000) as i16;
            let got = i16::from_le_bytes([pcm[i * 2], pcm[i * 2 + 1]]);
            assert_eq!(got, want, "sample {i}");
        }
    }
}
