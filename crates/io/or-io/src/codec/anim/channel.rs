use crate::error::{IoError, Result};

use super::packed::{PackedArray, PackedSequence};
use super::{format_type, AnimationSource, BlockSource};

pub const FORMAT_COUNT: usize = 9;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChannelFormat {
    ConstantQuat = 0,
    ConstantVec = 1,
    Constant = 2,
    Raw = 3,
    Quantize = 4,
    Indirect = 5,
    Linear = 6,
    Reconstruct = 7,
    Normalize = 8,
}

impl ChannelFormat {
    pub const ALL: [ChannelFormat; FORMAT_COUNT] = [
        Self::ConstantQuat,
        Self::ConstantVec,
        Self::Constant,
        Self::Raw,
        Self::Quantize,
        Self::Indirect,
        Self::Linear,
        Self::Reconstruct,
        Self::Normalize,
    ];

    pub fn engine_name(self) -> &'static str {
        match self {
            Self::ConstantQuat => "kConstantQuat",
            Self::ConstantVec => "kConstantVec",
            Self::Constant => "kConstant",
            Self::Raw => "kRaw",
            Self::Quantize => "kQuantize",
            Self::Indirect => "kIndirect",
            Self::Linear => "kLinear",
            Self::Reconstruct => "kReconstruct",
            Self::Normalize => "kNormalize",
        }
    }

    pub fn is_post_pass(self) -> bool {
        matches!(self, Self::Reconstruct | Self::Normalize)
    }

    fn from_index(index: usize) -> Self {
        Self::ALL[index]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedChannel {
    pub format: ChannelFormat,
    pub dof_index: u32,
    pub component: u8,
    pub constant_offset: usize,
    pub bit_address: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoverChannelRef {
    pub is_position: bool,
    pub raw: [u8; 6],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DofSamples {
    pub components: [f32; 4],
    pub count: usize,
}

impl DofSamples {
    pub fn zeroed(count: usize) -> Self {
        Self {
            components: [0.0; 4],
            count,
        }
    }

    pub fn as_slice(&self) -> &[f32] {
        &self.components[..self.count.min(4)]
    }
}

#[derive(Debug, Clone)]
pub struct DecodedDof {
    pub samples: DofSamples,
    pub channels: Vec<DecodedChannel>,
}

#[derive(Debug)]
pub struct ChannelWalk<'a> {
    counts: [u16; FORMAT_COUNT],
    indices: Vec<Vec<u16>>,
    constants: Vec<u32>,
    variable: &'a [u8],
    frame_size: usize,
    num_frames: usize,
    segment_size: u32,
}

impl<'a> ChannelWalk<'a> {
    pub fn new<B: BlockSource + ?Sized>(block: &'a B) -> Result<Self> {
        if block.is_compact() {
            return Err(IoError::InvalidData {
                context: format!(
                    "crBlockStream is compact (compact_size = {}), so its payload is compressed. \
                     Expanding it needs the crAnimCache path, which OpenRage does not implement. \
                     The block still reads and writes verbatim; only decoding is unavailable.",
                    block.compact_size()
                ),
            });
        }

        let data = block.data();
        let constant_size = block.constant_size() as usize;
        let frame_size = usize::from(block.frame_size());
        let num_frames = usize::from(block.num_frames());

        if constant_size > data.len() {
            return Err(IoError::InvalidData {
                context: format!(
                    "crBlockStream constant section is {constant_size} bytes but the payload is \
                     only {} bytes",
                    data.len()
                ),
            });
        }

        let variable_end = constant_size.saturating_add(frame_size.saturating_mul(num_frames));
        if variable_end > data.len() {
            return Err(IoError::InvalidData {
                context: format!(
                    "crBlockStream needs {variable_end} bytes for {num_frames} frames of \
                     {frame_size} bytes after a {constant_size}-byte constant section, but the \
                     payload is only {} bytes — the block is truncated",
                    data.len()
                ),
            });
        }

        let directory_start = block.channel_directory_offset();
        debug_assert_eq!(directory_start, variable_end);

        let directory = &data[directory_start..];
        let directory_u16: Vec<u16> = directory
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        if directory_u16.len() < FORMAT_COUNT {
            return Err(IoError::InvalidData {
                context: format!(
                    "crBlockStream channel directory needs {FORMAT_COUNT} counts but only {} u16s \
                     are present after offset {directory_start}",
                    directory_u16.len()
                ),
            });
        }

        let mut counts = [0u16; FORMAT_COUNT];
        counts.copy_from_slice(&directory_u16[..FORMAT_COUNT]);

        let mut indices: Vec<Vec<u16>> = Vec::with_capacity(FORMAT_COUNT);
        let mut cursor = FORMAT_COUNT;
        for &count in &counts {
            let count = usize::from(count);
            let end = cursor + count;
            if end > directory_u16.len() {
                return Err(IoError::InvalidData {
                    context: format!(
                        "crBlockStream channel directory declares a section of {count} entries at \
                         u16 {cursor} but only {} are present — the block is truncated",
                        directory_u16.len()
                    ),
                });
            }
            indices.push(directory_u16[cursor..end].to_vec());
            cursor += (count + 3) & !3;
        }

        let constants: Vec<u32> = data[..constant_size]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect();

        Ok(Self {
            counts,
            indices,
            constants,
            variable: &data[constant_size..variable_end],
            frame_size,
            num_frames,
            segment_size: u32::from(block.segment_size()),
        })
    }

    pub fn count(&self, format: ChannelFormat) -> usize {
        usize::from(self.counts[format as usize])
    }

    pub fn total_channels(&self) -> usize {
        self.counts.iter().map(|&c| usize::from(c)).sum()
    }

    pub fn num_frames(&self) -> usize {
        self.num_frames
    }

    pub fn channels(&self) -> Result<Vec<DecodedChannel>> {
        let mut found = Vec::with_capacity(self.total_channels());
        self.walk(|channel| {
            found.push(channel);
            Ok(())
        })?;
        Ok(found)
    }

    pub fn channels_for_dof(&self, dof_index: u32) -> Result<Vec<DecodedChannel>> {
        let mut found = Vec::new();
        self.walk(|channel| {
            if channel.dof_index == dof_index {
                found.push(channel);
            }
            Ok(())
        })?;
        Ok(found)
    }

    fn walk(&self, mut visit: impl FnMut(DecodedChannel) -> Result<()>) -> Result<()> {
        let mut constant_offset = 0usize;
        let mut bit_address = 0u32;

        for (section, &count) in self.counts.iter().enumerate() {
            let format = ChannelFormat::from_index(section);
            for j in 0..usize::from(count) {
                let idx = self.indices[section][j];
                visit(DecodedChannel {
                    format,
                    dof_index: u32::from(idx) >> 2,
                    component: (idx & 3) as u8,
                    constant_offset,
                    bit_address,
                })?;

                match format {
                    ChannelFormat::ConstantQuat | ChannelFormat::ConstantVec => {
                        constant_offset += 3;
                    }
                    ChannelFormat::Constant => constant_offset += 1,
                    ChannelFormat::Raw => bit_address += 32,
                    ChannelFormat::Quantize => {
                        bit_address += self.constant_word(constant_offset)? & 31;
                        constant_offset += 3;
                    }
                    ChannelFormat::Indirect => {
                        bit_address += self.constant_word(constant_offset)?;
                        let num_values = self.constant_word(constant_offset + 2)? as usize;
                        constant_offset += 5 + num_values;
                    }
                    ChannelFormat::Linear => {
                        let stride = self.constant_word(constant_offset)? as usize;
                        if stride == 0 {
                            return Err(IoError::InvalidData {
                                context: format!(
                                    "kLinear channel at constant word {constant_offset} declares a \
                                     zero stride, which would loop forever"
                                ),
                            });
                        }
                        constant_offset += stride;
                    }
                    ChannelFormat::Reconstruct | ChannelFormat::Normalize => {}
                }
            }
        }
        Ok(())
    }

    fn constant_word(&self, offset: usize) -> Result<u32> {
        self.constants
            .get(offset)
            .copied()
            .ok_or_else(|| IoError::InvalidData {
                context: format!(
                    "channel header reads constant word {offset} but the constant section is only \
                     {} words — the block payload is truncated",
                    self.constants.len()
                ),
            })
    }

    fn constant_f32(&self, offset: usize) -> Result<f32> {
        Ok(f32::from_bits(self.constant_word(offset)?))
    }

    pub(crate) fn frame_words(&self, frame: usize) -> Result<Vec<u32>> {
        if self.frame_size == 0 {
            return Ok(Vec::new());
        }
        let frame = frame.min(self.num_frames.saturating_sub(1));
        let start = frame * self.frame_size;
        let bytes = self
            .variable
            .get(start..start + self.frame_size)
            .ok_or_else(|| IoError::InvalidData {
                context: format!(
                    "frame {frame} is past the variable section ({} bytes, {} per frame)",
                    self.variable.len(),
                    self.frame_size
                ),
            })?;
        let mut words: Vec<u32> = bytes
            .chunks(4)
            .map(|w| {
                let mut buf = [0u8; 4];
                buf[..w.len()].copy_from_slice(w);
                u32::from_le_bytes(buf)
            })
            .collect();
        words.push(0);
        Ok(words)
    }

    pub fn evaluate_dof(
        &self,
        dof_index: u32,
        frame: usize,
        tr: f32,
        component_count: usize,
    ) -> Result<Option<DecodedDof>> {
        let channels = self.channels_for_dof(dof_index)?;
        if channels.is_empty() {
            return Ok(None);
        }
        let samples = self.evaluate_channels(&channels, frame, tr, component_count)?;
        Ok(Some(DecodedDof { samples, channels }))
    }

    /// One DOF's value from its channels, as [`ChannelWalk::channels_for_dof`] lists them.
    pub fn evaluate_channels(
        &self,
        channels: &[DecodedChannel],
        frame: usize,
        tr: f32,
        component_count: usize,
    ) -> Result<DofSamples> {
        let prev = self.frame_words(frame)?;
        let next = self.frame_words(frame + 1)?;
        self.evaluate_between(channels, &prev, &next, frame, tr, component_count)
    }

    /// [`ChannelWalk::evaluate_channels`] with the two frames' words already read.
    pub(crate) fn evaluate_between(
        &self,
        channels: &[DecodedChannel],
        prev: &[u32],
        next: &[u32],
        frame: usize,
        tr: f32,
        component_count: usize,
    ) -> Result<DofSamples> {
        let mut samples = DofSamples::zeroed(component_count.clamp(1, 4));

        for channel in channels {
            match channel.format {
                ChannelFormat::ConstantQuat => {
                    let x = self.constant_f32(channel.constant_offset)?;
                    let y = self.constant_f32(channel.constant_offset + 1)?;
                    let z = self.constant_f32(channel.constant_offset + 2)?;
                    samples.components = [x, y, z, reconstruct_component(x, y, z)];
                    samples.count = 4;
                }
                ChannelFormat::ConstantVec => {
                    for (c, slot) in samples.components.iter_mut().enumerate().take(3) {
                        *slot = self.constant_f32(channel.constant_offset + c)?;
                    }
                    samples.count = samples.count.max(3);
                }
                ChannelFormat::Constant => {
                    self.store(
                        &mut samples,
                        channel.component,
                        self.constant_f32(channel.constant_offset)?,
                    );
                }
                ChannelFormat::Raw => {
                    let a = f32::from_bits(PackedArray::new(prev).get(channel.bit_address, 32)?);
                    let b = f32::from_bits(PackedArray::new(next).get(channel.bit_address, 32)?);
                    self.store(&mut samples, channel.component, lerp(a, b, tr));
                }
                ChannelFormat::Quantize => {
                    let bits = self.constant_word(channel.constant_offset)? & 31;
                    let scale = self.constant_f32(channel.constant_offset + 1)?;
                    let offset = self.constant_f32(channel.constant_offset + 2)?;
                    let a = PackedArray::new(prev).get(channel.bit_address, bits)? as f32;
                    let b = PackedArray::new(next).get(channel.bit_address, bits)? as f32;
                    self.store(
                        &mut samples,
                        channel.component,
                        scale * lerp(a, b, tr) + offset,
                    );
                }
                ChannelFormat::Indirect => {
                    let index_bits = self.constant_word(channel.constant_offset)?;
                    let value_bits = self.constant_word(channel.constant_offset + 1)?;
                    let scale = self.constant_f32(channel.constant_offset + 3)?;
                    let offset = self.constant_f32(channel.constant_offset + 4)?;
                    let table = PackedArray::new(&self.constants)
                        .slice_from(channel.constant_offset + 5)?;

                    let a_index = PackedArray::new(prev).get(channel.bit_address, index_bits)?;
                    let b_index = PackedArray::new(next).get(channel.bit_address, index_bits)?;
                    let a = table.get(a_index * value_bits, value_bits)? as f32;
                    let b = table.get(b_index * value_bits, value_bits)? as f32;
                    self.store(
                        &mut samples,
                        channel.component,
                        scale * lerp(a, b, tr) + offset,
                    );
                }
                ChannelFormat::Linear => {
                    let value = self.evaluate_linear(channel, frame, tr)?;
                    self.store(&mut samples, channel.component, value);
                }
                ChannelFormat::Reconstruct => {
                    let [a, b, c, _] = samples.components;
                    let w = reconstruct_component(a, b, c);
                    samples.components = match channel.component {
                        0 => [w, a, b, c],
                        1 => [a, w, b, c],
                        2 => [a, b, w, c],
                        _ => [a, b, c, w],
                    };
                    samples.count = 4;
                }
                ChannelFormat::Normalize => {
                    let [x, y, z, w] = samples.components;
                    let length = (x * x + y * y + z * z + w * w).sqrt();
                    if length > f32::MIN_POSITIVE {
                        samples.components = [x / length, y / length, z / length, w / length];
                    }
                    samples.count = 4;
                }
            }
        }

        Ok(samples)
    }

    fn evaluate_linear(&self, channel: &DecodedChannel, frame: usize, tr: f32) -> Result<f32> {
        let segment_size = self.segment_size;
        if segment_size == 0 {
            return Err(IoError::InvalidData {
                context: "kLinear channel needs segment_size > 0, but the block declares 0"
                    .to_string(),
            });
        }

        let sequence = PackedArray::new(&self.constants).slice_from(channel.constant_offset)?;
        let packed = sequence
            .words()
            .get(1)
            .copied()
            .ok_or_else(|| IoError::InvalidData {
                context: "kLinear channel header is truncated".to_string(),
            })?;
        let scale = f32::from_bits(sequence.words()[2]);
        let offset = f32::from_bits(sequence.words()[3]);

        let addr_bits = packed & 0xFF;
        let initial_bits = (packed >> 8) & 0xFF;
        let divisor = (packed >> 16) & 0xFF;

        const HEADER_BITS: u32 = 4 * 4 * 8;
        let segment_mask = segment_size - 1;
        let num_segments = (self.num_frames as u32 + segment_mask) / segment_size;
        let frame = frame as u32;
        let value_index = frame % segment_size;
        let segment_index = frame / segment_size;

        let address_offset = sequence.get(HEADER_BITS + segment_index * addr_bits, addr_bits)?;
        let mut address = HEADER_BITS + num_segments * addr_bits;

        let segment_addr = address + segment_index * initial_bits;
        let prev_segment = sequence.get(segment_addr, initial_bits)? as i64;
        let next_segment = sequence.get(segment_addr + initial_bits, initial_bits)? as i64;

        address += num_segments * initial_bits + address_offset;

        let mut stream = PackedSequence::new(sequence.words(), address);
        let mut velocity: i64 = 0;
        let mut previous: i64 = 0;
        let mut current: i64 = prev_segment;
        for _ in 0..=value_index {
            velocity += i64::from(stream.next_value(divisor)?);
            previous = current;
            current += velocity;
        }
        if value_index == segment_mask {
            current = next_segment;
        }

        Ok(scale * lerp(previous as f32, current as f32, tr) + offset)
    }

    fn store(&self, samples: &mut DofSamples, component: u8, value: f32) {
        let index = usize::from(component).min(3);
        samples.components[index] = value;
        samples.count = samples.count.max(index + 1);
    }
}

pub fn reconstruct_component(x: f32, y: f32, z: f32) -> f32 {
    let squared = 1.0 - (x * x + y * y + z * z);
    if squared > f32::MIN_POSITIVE {
        squared.sqrt()
    } else {
        0.0
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Where one DOF is sampled at a time: its block, and the frame and the
/// fraction of a frame into that block.
struct Sample<'a, B> {
    components: usize,
    block_index: usize,
    block: &'a B,
    tq: usize,
    tr: f32,
}

fn locate<A: AnimationSource + ?Sized>(
    animation: &A,
    dof_index: usize,
    time: f32,
) -> Result<Option<Sample<'_, A::Block>>> {
    let Some(format) = animation.dof_format(dof_index) else {
        return Ok(None);
    };

    let internal_frame = animation.time_to_internal_frame(time);
    let block_index = animation.internal_frame_to_block_index(internal_frame);
    let Some(block) = animation.block(block_index) else {
        return Ok(None);
    };
    if block.data().is_empty() {
        return Err(IoError::InvalidData {
            context: format!(
                "animation {:?} block {block_index} has no payload — it was read in reference \
                 mode, which drops block payloads.",
                animation.name()
            ),
        });
    }

    let block_time = animation.internal_frame_to_block_time(internal_frame, block_index);
    let tq = (block_time as usize).min(usize::from(block.num_frames()).saturating_sub(2));
    let tr = (block_time - tq as f32).clamp(0.0, 1.0);
    Ok(Some(Sample {
        components: format_type::component_count(format),
        block_index,
        block,
        tq,
        tr,
    }))
}

/// DOF `dof_index`'s value at `time` seconds, or `None` where no channel drives it.
pub fn decode_dof<A: AnimationSource + ?Sized>(
    animation: &A,
    dof_index: usize,
    time: f32,
) -> Result<Option<DofSamples>> {
    let Some(at) = locate(animation, dof_index, time)? else {
        return Ok(None);
    };
    let walk = ChannelWalk::new(at.block)?;
    Ok(walk
        .evaluate_dof(dof_index as u32, at.tq, at.tr, at.components)?
        .map(|decoded| decoded.samples))
}

/// [`decode_dof`] for many samples of one animation, each block's walk and
/// its channels by DOF built once rather than once a sample.
///
/// ⚠ A block whose walk does not build takes [`decode_dof`]'s own path on
/// every sample, so a refusal is the same error it always was.
pub struct DofDecoder<'a, A: AnimationSource + ?Sized> {
    animation: &'a A,
    /// Per block: not yet built, built, or refused to build.
    blocks: Vec<Option<Option<BlockChannels<'a>>>>,
    words: Option<FrameWords>,
}

/// The last `(block, frame)` sampled, and that frame's and the next one's words.
struct FrameWords {
    at: (usize, usize),
    prev: Vec<u32>,
    next: Vec<u32>,
}

struct BlockChannels<'a> {
    walk: ChannelWalk<'a>,
    by_dof: Vec<Vec<DecodedChannel>>,
}

impl<'a> BlockChannels<'a> {
    fn build<B: BlockSource + ?Sized>(block: &'a B) -> Option<Self> {
        let walk = ChannelWalk::new(block).ok()?;
        let mut by_dof: Vec<Vec<DecodedChannel>> = Vec::new();
        walk.walk(|channel| {
            let dof = channel.dof_index as usize;
            if by_dof.len() <= dof {
                by_dof.resize_with(dof + 1, Vec::new);
            }
            by_dof[dof].push(channel);
            Ok(())
        })
        .ok()?;
        Some(Self { walk, by_dof })
    }
}

impl<'a, A: AnimationSource + ?Sized> DofDecoder<'a, A> {
    pub fn new(animation: &'a A) -> Self {
        Self {
            animation,
            blocks: std::iter::repeat_with(|| None)
                .take(animation.block_count())
                .collect(),
            words: None,
        }
    }

    /// Exactly what [`decode_dof`] returns for the same arguments.
    pub fn decode(&mut self, dof_index: usize, time: f32) -> Result<Option<DofSamples>> {
        let animation = self.animation;
        let Some(at) = locate(animation, dof_index, time)? else {
            return Ok(None);
        };
        let slot = &mut self.blocks[at.block_index];
        let built = slot.get_or_insert_with(|| BlockChannels::build(at.block));
        let Some(block) = built else {
            return decode_dof(animation, dof_index, time);
        };
        let channels = block.by_dof.get(dof_index).map_or(&[][..], Vec::as_slice);
        if channels.is_empty() {
            return Ok(None);
        }
        // Every DOF sampled at one time reads the same two frames.
        let key = (at.block_index, at.tq);
        if self.words.as_ref().map(|w| w.at) != Some(key) {
            match (
                block.walk.frame_words(at.tq),
                block.walk.frame_words(at.tq + 1),
            ) {
                (Ok(prev), Ok(next)) => {
                    self.words = Some(FrameWords {
                        at: key,
                        prev,
                        next,
                    })
                }
                _ => return decode_dof(animation, dof_index, time),
            }
        }
        let words = self.words.as_ref().expect("read above");
        block
            .walk
            .evaluate_between(
                channels,
                &words.prev,
                &words.next,
                at.tq,
                at.tr,
                at.components,
            )
            .map(Some)
    }
}

#[cfg(all(test, feature = "gta5"))]
mod tests {
    use super::*;
    use crate::gta5::CrBlockStream;

    fn constant_float_block(value: f32, dof: u32, component: u8) -> CrBlockStream {
        let mut data = Vec::new();
        data.extend_from_slice(&value.to_bits().to_le_bytes());
        let constant_size = data.len() as u32;
        let channel_offset = data.len() as u32;
        let mut counts = [0u16; FORMAT_COUNT];
        counts[ChannelFormat::Constant as usize] = 1;
        for count in counts {
            data.extend_from_slice(&count.to_le_bytes());
        }
        let idx = ((dof << 2) | u32::from(component)) as u16;
        for entry in [idx, 0, 0, 0] {
            data.extend_from_slice(&entry.to_le_bytes());
        }

        CrBlockStream {
            constant_size,
            channel_offset,
            num_frames: 1,
            frame_size: 0,
            segment_size: 64,
            data,
            ..CrBlockStream::default()
        }
    }

    #[test]
    fn format_enum_matches_engine_order() {
        assert_eq!(ChannelFormat::ConstantQuat as usize, 0);
        assert_eq!(ChannelFormat::ConstantVec as usize, 1);
        assert_eq!(ChannelFormat::Constant as usize, 2);
        assert_eq!(ChannelFormat::Raw as usize, 3);
        assert_eq!(ChannelFormat::Quantize as usize, 4);
        assert_eq!(ChannelFormat::Indirect as usize, 5);
        assert_eq!(ChannelFormat::Linear as usize, 6);
        assert_eq!(ChannelFormat::Reconstruct as usize, 7);
        assert_eq!(ChannelFormat::Normalize as usize, 8);
        assert_eq!(ChannelFormat::ALL.len(), FORMAT_COUNT);
    }

    #[test]
    fn constant_float_channel_decodes() {
        let block = constant_float_block(1.25, 3, 1);
        let walk = ChannelWalk::new(&block).expect("parse directory");

        assert_eq!(walk.count(ChannelFormat::Constant), 1);
        assert_eq!(walk.total_channels(), 1);

        let channels = walk.channels().expect("walk");
        assert_eq!(channels.len(), 1);
        assert_eq!(channels[0].dof_index, 3);
        assert_eq!(channels[0].component, 1);

        let decoded = walk
            .evaluate_dof(3, 0, 0.0, 3)
            .expect("evaluate")
            .expect("dof present");
        assert_eq!(decoded.samples.components[1], 1.25);
    }

    #[test]
    fn dof_lookup_ignores_other_dofs() {
        let block = constant_float_block(1.25, 3, 1);
        let walk = ChannelWalk::new(&block).unwrap();
        assert!(walk.channels_for_dof(4).unwrap().is_empty());
        assert!(walk.evaluate_dof(4, 0, 0.0, 3).unwrap().is_none());
    }

    #[test]
    fn compact_block_is_refused() {
        let mut block = constant_float_block(1.0, 0, 0);
        block.compact_size = 64;
        let err = ChannelWalk::new(&block).expect_err("must refuse");
        assert!(err.to_string().contains("compact"), "{err}");
        assert!(err.to_string().contains("crAnimCache"), "{err}");
    }

    #[test]
    fn truncated_directory_is_rejected() {
        let mut block = constant_float_block(1.0, 0, 0);
        block.data.truncate(block.data.len() - 8);
        let err = ChannelWalk::new(&block).expect_err("must reject");
        assert!(err.to_string().contains("truncated"), "{err}");
    }

    #[test]
    fn a_constant_section_past_the_payload_is_rejected() {
        let mut block = constant_float_block(1.0, 0, 0);
        block.constant_size = 0xFFFF;
        let err = ChannelWalk::new(&block).expect_err("must reject");
        assert!(err.to_string().contains("payload"), "{err}");
    }

    #[test]
    fn a_channel_offset_past_the_payload_is_not_an_error() {
        let mut block = constant_float_block(1.0, 0, 0);
        block.channel_offset = 0xFFFF;
        ChannelWalk::new(&block).expect("channel_offset must not affect the walk");
    }

    #[test]
    fn variable_section_overrun_is_rejected() {
        let mut block = constant_float_block(1.0, 0, 0);
        block.frame_size = 64;
        block.num_frames = 1000;
        let err = ChannelWalk::new(&block).expect_err("must reject");
        assert!(err.to_string().contains("truncated"), "{err}");
    }

    #[test]
    fn reconstruct_component_is_clamped_not_nan() {
        assert_eq!(reconstruct_component(1.0, 0.0, 0.0), 0.0);
        assert_eq!(reconstruct_component(1.0, 1.0, 1.0), 0.0);
        assert!(!reconstruct_component(2.0, 0.0, 0.0).is_nan());
        let w = reconstruct_component(0.5, 0.5, 0.5);
        assert!((w - 0.25f32.sqrt()).abs() < 1e-6, "w = {w}");
    }

    #[test]
    fn reconstruct_splices_at_the_component_slot() {
        let mut samples = DofSamples {
            components: [0.1, 0.2, 0.3, 0.0],
            count: 3,
        };
        let w = reconstruct_component(0.1, 0.2, 0.3);

        for (component, expected) in [
            (0u8, [w, 0.1, 0.2, 0.3]),
            (1, [0.1, w, 0.2, 0.3]),
            (2, [0.1, 0.2, w, 0.3]),
            (3, [0.1, 0.2, 0.3, w]),
        ] {
            let [a, b, c, _] = [0.1f32, 0.2, 0.3, 0.0];
            samples.components = match component {
                0 => [w, a, b, c],
                1 => [a, w, b, c],
                2 => [a, b, w, c],
                _ => [a, b, c, w],
            };
            assert_eq!(samples.components, expected, "component {component}");
        }
    }

    #[test]
    fn lerp_endpoints_are_exact() {
        assert_eq!(lerp(1.0, 3.0, 0.0), 1.0);
        assert_eq!(lerp(1.0, 3.0, 1.0), 3.0);
        assert_eq!(lerp(1.0, 3.0, 0.5), 2.0);
    }

    struct BitWriter {
        bits: Vec<bool>,
    }

    impl BitWriter {
        fn new() -> Self {
            Self { bits: Vec::new() }
        }

        fn pad_to(&mut self, bit: usize) {
            while self.bits.len() < bit {
                self.bits.push(false);
            }
        }

        fn push_bits(&mut self, value: u32, bits: u32) {
            for i in 0..bits {
                self.bits.push((value >> i) & 1 == 1);
            }
        }

        fn push_rice(&mut self, value: i32, divisor: u32) {
            let magnitude = value.unsigned_abs();
            let remainder = magnitude & super::super::packed::MASK_TABLE[divisor as usize];
            let quotient = magnitude >> divisor;
            self.push_bits(remainder, divisor);
            for _ in 0..quotient {
                self.bits.push(false);
            }
            self.bits.push(true);
            if magnitude != 0 {
                self.bits.push(value < 0);
            }
        }

        fn into_words(self) -> Vec<u32> {
            let mut words = vec![0u32; self.bits.len().div_ceil(32) + 2];
            for (i, &bit) in self.bits.iter().enumerate() {
                if bit {
                    words[i / 32] |= 1 << (i % 32);
                }
            }
            words
        }
    }

    struct LinearFixture {
        scale: f32,
        offset: f32,
        prev_segment: u32,
        next_segment: u32,
        velocities: [i32; 4],
        segment_size: u8,
        num_frames: u16,
    }

    fn linear_block(fixture: &LinearFixture) -> CrBlockStream {
        const ADDR_BITS: u32 = 8;
        const INITIAL_BITS: u32 = 16;
        const DIVISOR: u32 = 2;
        const HEADER_BITS: usize = 4 * 4 * 8;

        let num_segments = u32::from(fixture.num_frames).div_ceil(u32::from(fixture.segment_size));

        let mut writer = BitWriter::new();
        writer.pad_to(HEADER_BITS);

        // One address offset per segment.
        for _ in 0..num_segments {
            writer.push_bits(INITIAL_BITS, ADDR_BITS);
        }
        writer.push_bits(fixture.prev_segment, INITIAL_BITS);
        writer.push_bits(fixture.next_segment, INITIAL_BITS);
        for _ in 0..num_segments.saturating_sub(1) {
            writer.push_bits(0, INITIAL_BITS);
        }
        // The Rice-coded velocity deltas.
        for &velocity in &fixture.velocities {
            writer.push_rice(velocity, DIVISOR);
        }

        let mut words = writer.into_words();
        words[0] = words.len() as u32;
        words[1] = ADDR_BITS | (INITIAL_BITS << 8) | (DIVISOR << 16);
        words[2] = fixture.scale.to_bits();
        words[3] = fixture.offset.to_bits();

        let mut data = Vec::new();
        for word in &words {
            data.extend_from_slice(&word.to_le_bytes());
        }
        let constant_size = data.len() as u32;
        let channel_offset = data.len() as u32;

        let mut counts = [0u16; FORMAT_COUNT];
        counts[ChannelFormat::Linear as usize] = 1;
        for count in counts {
            data.extend_from_slice(&count.to_le_bytes());
        }
        for entry in [0u16, 0, 0, 0] {
            data.extend_from_slice(&entry.to_le_bytes());
        }

        CrBlockStream {
            constant_size,
            channel_offset,
            num_frames: fixture.num_frames,
            frame_size: 0,
            segment_size: fixture.segment_size,
            data,
            ..CrBlockStream::default()
        }
    }

    #[test]
    fn linear_channel_integrates_velocities() {
        let fixture = LinearFixture {
            scale: 0.5,
            offset: 1.0,
            prev_segment: 10,
            next_segment: 20,
            velocities: [3, 1, 1, 1],
            segment_size: 4,
            num_frames: 4,
        };
        let block = linear_block(&fixture);
        let walk = ChannelWalk::new(&block).expect("parse directory");
        assert_eq!(walk.count(ChannelFormat::Linear), 1);

        let value_at = |frame: usize| {
            walk.evaluate_dof(0, frame, 0.0, 1)
                .expect("evaluate")
                .expect("dof present")
                .samples
                .components[0]
        };

        assert!(
            (value_at(0) - 6.0).abs() < 1e-5,
            "frame 0 = {}",
            value_at(0)
        );

        assert!(
            (value_at(1) - 7.5).abs() < 1e-5,
            "frame 1 = {}",
            value_at(1)
        );

        assert!(
            (value_at(2) - 9.5).abs() < 1e-5,
            "frame 2 = {}",
            value_at(2)
        );

        assert!(
            (value_at(3) - 12.0).abs() < 1e-5,
            "frame 3 = {}",
            value_at(3)
        );
    }

    #[test]
    fn linear_channel_blends_between_frames() {
        let fixture = LinearFixture {
            scale: 1.0,
            offset: 0.0,
            prev_segment: 10,
            next_segment: 20,
            velocities: [2, 2, 2, 2],
            segment_size: 4,
            num_frames: 4,
        };
        let block = linear_block(&fixture);
        let walk = ChannelWalk::new(&block).unwrap();

        let at = |frame: usize, tr: f32| {
            walk.evaluate_dof(0, frame, tr, 1)
                .unwrap()
                .unwrap()
                .samples
                .components[0]
        };

        assert!((at(0, 0.0) - 10.0).abs() < 1e-5);
        assert!((at(0, 1.0) - 12.0).abs() < 1e-5);
        assert!((at(0, 0.5) - 11.0).abs() < 1e-5);
    }

    #[test]
    fn linear_channel_rejects_zero_segment_size() {
        let fixture = LinearFixture {
            scale: 1.0,
            offset: 0.0,
            prev_segment: 1,
            next_segment: 2,
            velocities: [1, 1, 1, 1],
            segment_size: 4,
            num_frames: 4,
        };
        let mut block = linear_block(&fixture);
        block.segment_size = 0;
        let walk = ChannelWalk::new(&block).unwrap();
        let err = walk.evaluate_dof(0, 0, 0.0, 1).expect_err("must reject");
        assert!(err.to_string().contains("segment_size"), "{err}");
    }

    #[test]
    fn linear_channel_rejects_zero_stride() {
        let fixture = LinearFixture {
            scale: 1.0,
            offset: 0.0,
            prev_segment: 1,
            next_segment: 2,
            velocities: [1, 1, 1, 1],
            segment_size: 4,
            num_frames: 4,
        };
        let mut block = linear_block(&fixture);
        block.data[0..4].copy_from_slice(&0u32.to_le_bytes());
        let walk = ChannelWalk::new(&block).unwrap();
        let err = walk.channels().expect_err("must reject");
        assert!(err.to_string().contains("zero stride"), "{err}");
    }

    #[test]
    fn dof_samples_slice_respects_count() {
        let samples = DofSamples {
            components: [1.0, 2.0, 3.0, 4.0],
            count: 3,
        };
        assert_eq!(samples.as_slice(), &[1.0, 2.0, 3.0]);
        assert_eq!(DofSamples::zeroed(4).as_slice().len(), 4);
    }
}
