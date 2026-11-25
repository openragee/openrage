//! The Bink audio decoders, the DCT one and the RDFT one: one packet in, floating-point samples out.
//!
//! Origin: FFmpeg n7.0 (commit 083443d67cb159ce469e5d902346b8d0c2cd1c93), libavcodec, the file
//! binkaudio.c, Copyright (c) 2007-2011 Peter Ross and (c) 2009 Daniel Verkamp, with the band edges
//! of wma_freqs.c, LGPL-2.1-or-later. Ported to safe Rust; the transforms are OpenRage's own, in
//! `crate::transform`. Mono and stereo, which is all the container's flags can state. Listed in
//! LICENSE-third-party.md.
//!
//! A `KB2n` movie's DCT blocks differ in two things, measured on such movies and written here
//! from that: a band's step is seven bits, and a span's signs follow all of its values.

use crate::bits::BitReader;
use crate::transform::{Dct3, Rdft};
use crate::{AudioStream, MediaError, Samples};

/// The critical band edges, in hertz, the coefficients' step sizes are grouped by.
const CRITICAL_FREQS: [u32; 25] = [
    100, 200, 300, 400, 510, 630, 770, 920, 1080, 1270, 1480, 1720, 2000, 2320, 2700, 3150, 3700,
    4400, 5300, 6400, 7700, 9500, 12000, 15500, 24500,
];

/// Run lengths, in groups of eight coefficients, a four-bit code stands for.
const RLE_LENGTHS: [usize; 16] = [2, 3, 4, 5, 6, 8, 9, 10, 11, 12, 13, 14, 15, 16, 32, 64];

fn bad(what: &str) -> MediaError {
    MediaError::Decode(format!("this Bink audio packet is corrupt: {what}"))
}

enum Transform {
    Dct(Dct3),
    Rdft(Rdft),
}

pub struct Decoder {
    /// Channels the container states: what a block interleaves to.
    channels: usize,
    /// Channels one block holds: all of them for the DCT, one interleaved stream for the RDFT.
    block_channels: usize,
    frame_len: usize,
    overlap_len: usize,
    num_bands: usize,
    root: f32,
    bands: [usize; 26],
    quant_table: [f32; 96],
    /// The tail of each channel's last block, which the next one is blended with.
    previous: [Vec<f32>; 2],
    first: bool,
    transform: Transform,
    /// Whether blocks are in the coding of a `KB2n` movie.
    revised: bool,
}

impl Decoder {
    /// A decoder of the DCT form (`dct`) or the RDFT one, for a track of `channels` (1 or 2).
    pub fn new(sample_rate: u32, channels: u16, dct: bool) -> Result<Self, MediaError> {
        if !(1..=2).contains(&channels) {
            return Err(MediaError::Decode(format!(
                "Bink audio of {channels} channels is not one OpenRage decodes"
            )));
        }
        let channels = usize::from(channels);
        let mut frame_len_bits = match sample_rate {
            0..=22049 => 9,
            22050..=44099 => 10,
            _ => 11,
        };
        let mut rate = u64::from(sample_rate);
        let block_channels = if dct {
            channels
        } else {
            rate *= channels as u64;
            frame_len_bits += channels.trailing_zeros();
            1
        };
        let frame_len = 1usize << frame_len_bits;
        let overlap_len = frame_len / 16;
        let rate_half = rate.div_ceil(2) as usize;
        let root = if dct {
            frame_len as f64 / ((frame_len as f64).sqrt() * 32768.0)
        } else {
            2.0 / ((frame_len as f64).sqrt() * 32768.0)
        } as f32;
        let quant_table = std::array::from_fn(|i| (i as f32 * 0.152_891_65_f32).exp() * root);

        let mut num_bands = 1;
        while num_bands < 25 && rate_half > CRITICAL_FREQS[num_bands - 1] as usize {
            num_bands += 1;
        }
        let mut bands = [0usize; 26];
        bands[0] = 2;
        for i in 1..num_bands {
            bands[i] = (CRITICAL_FREQS[i - 1] as usize * frame_len / rate_half) & !1;
        }
        bands[num_bands] = frame_len;

        let transform = if dct {
            Transform::Dct(Dct3::new(frame_len))
        } else {
            Transform::Rdft(Rdft::new(frame_len))
        };
        Ok(Self {
            channels,
            block_channels,
            frame_len,
            overlap_len,
            num_bands,
            root,
            bands,
            quant_table,
            previous: [vec![0.0; overlap_len], vec![0.0; overlap_len]],
            first: true,
            transform,
            revised: false,
        })
    }

    /// The same decoder for the blocks of a `KB2n` movie.
    pub fn revised(mut self) -> Self {
        self.revised = true;
        self
    }

    /// Forgets the last block, as at the start of a track.
    pub fn reset(&mut self) {
        self.first = true;
    }

    /// Decodes a packet into interleaved samples: the blocks it holds, one after another.
    pub fn decode_packet(&mut self, packet: &[u8]) -> Result<Vec<f32>, MediaError> {
        if packet.len() < 4 {
            return Err(bad("it is too small"));
        }
        let mut gb = BitReader::new(packet);
        gb.skip(32);
        let mut out = Vec::new();
        loop {
            match self.decode_block(&mut gb) {
                Ok(block) => out.extend(block),
                // A packet's padding is not a block: what decoded before it stands.
                Err(_) if !out.is_empty() => break,
                Err(e) => return Err(e),
            }
            gb.align32();
            if gb.bits_left() <= 0 {
                break;
            }
        }
        Ok(out)
    }

    fn get_float(gb: &mut BitReader) -> f32 {
        let power = gb.get(5) as i32;
        let mantissa = gb.get(23) as f32;
        let exp2 = f32::from_bits(((power - 23 + 127) as u32) << 23);
        let f = mantissa * exp2;
        if gb.get1() != 0 {
            -f
        } else {
            f
        }
    }

    /// One block: `block_channels` channels' worth of coefficients, transformed and blended with
    /// the block before, returned interleaved.
    fn decode_block(&mut self, gb: &mut BitReader) -> Result<Vec<f32>, MediaError> {
        let frame_len = self.frame_len;
        let channels = self.block_channels;
        let dct = matches!(self.transform, Transform::Dct(_));
        if dct {
            gb.skip(2);
        }
        let mut out: Vec<Vec<f32>> = vec![vec![0.0; frame_len]; channels];
        let mut coeffs = vec![0.0f32; frame_len + 2];
        for channel in out.iter_mut() {
            if gb.bits_left() < 58 {
                return Err(bad("a block runs past the end"));
            }
            coeffs[0] = Self::get_float(gb) * self.root;
            coeffs[1] = Self::get_float(gb) * self.root;
            let step_bits = if self.revised { 7 } else { 8 };
            if gb.bits_left() < (self.num_bands as u32 * step_bits) as i64 {
                return Err(bad("the band table runs past the end"));
            }
            let mut quant = [0.0f32; 26];
            for q in quant.iter_mut().take(self.num_bands) {
                *q = self.quant_table[(gb.get(step_bits) as usize).min(95)];
            }

            let mut k = 0;
            let mut q = quant[0];
            let mut i = 2;
            while i < frame_len {
                let span = if gb.get1() != 0 {
                    RLE_LENGTHS[gb.get(4) as usize] * 8
                } else {
                    8
                };
                let j = (i + span).min(frame_len);
                let width = gb.get(4);
                if width == 0 {
                    coeffs[i..j].fill(0.0);
                    i = j;
                    while self.bands[k] < i {
                        q = quant[k];
                        k += 1;
                    }
                } else {
                    let span = i..j;
                    while i < j {
                        if self.bands[k] == i {
                            q = quant[k];
                            k += 1;
                        }
                        let coeff = gb.get(width);
                        coeffs[i] = if coeff == 0 {
                            0.0
                        } else if !self.revised && gb.get1() != 0 {
                            -q * coeff as f32
                        } else {
                            q * coeff as f32
                        };
                        i += 1;
                    }
                    if self.revised {
                        // A step is never zero, so a value that is not zero has a sign.
                        for coeff in coeffs[span].iter_mut().filter(|c| **c != 0.0) {
                            if gb.get1() != 0 {
                                *coeff = -*coeff;
                            }
                        }
                    }
                }
            }

            match &self.transform {
                Transform::Dct(dct) => {
                    dct.apply(&coeffs[..frame_len], channel);
                    // The reference doubles x[0] to undo its DCT-III halving that term, and scales
                    // by 1 / N over a transform that weights the other terms by 2: 2 / N in all.
                    let scale = 2.0 / frame_len as f32;
                    channel.iter_mut().for_each(|s| *s *= scale);
                }
                Transform::Rdft(rdft) => {
                    for i in (2..frame_len).step_by(2) {
                        coeffs[i + 1] = -coeffs[i + 1];
                    }
                    coeffs[frame_len] = coeffs[1];
                    coeffs[frame_len + 1] = 0.0;
                    coeffs[1] = 0.0;
                    rdft.apply(&coeffs, 0.5, channel);
                }
            }
        }

        let count = self.overlap_len * channels;
        for (ch, channel) in out.iter_mut().enumerate() {
            if !self.first {
                for (i, sample) in channel.iter_mut().enumerate().take(self.overlap_len) {
                    let j = ch + i * channels;
                    *sample = (self.previous[ch][i] * (count - j) as f32 + *sample * j as f32)
                        / count as f32;
                }
            }
            self.previous[ch].copy_from_slice(&channel[frame_len - self.overlap_len..]);
        }
        self.first = false;

        let keep = frame_len - self.overlap_len;
        if channels == 1 {
            let mut only = out.swap_remove(0);
            only.truncate(keep);
            return Ok(only);
        }
        Ok((0..keep)
            .flat_map(|i| out.iter().map(move |channel| channel[i]))
            .collect())
    }

    /// How many channels the samples this decoder returns are interleaved by.
    pub fn channels(&self) -> usize {
        self.channels
    }
}

/// A Bink sound track as a stream: its packets, framed by a length each, decoded one at a time.
pub(crate) struct Stream {
    decoder: Decoder,
    /// Every packet as a little-endian `u32` length then its bytes.
    packets: Vec<u8>,
    at: usize,
    /// Sample frames still to drop after a seek.
    skip: usize,
}

impl Stream {
    pub(crate) fn new(decoder: Decoder, packets: Vec<u8>) -> Self {
        Self {
            decoder,
            packets,
            at: 0,
            skip: 0,
        }
    }
}

impl AudioStream for Stream {
    fn read(&mut self) -> Result<Option<Samples>, MediaError> {
        loop {
            let Some(head) = self.packets.get(self.at..self.at + 4) else {
                return Ok(None);
            };
            let len = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
            let start = self.at + 4;
            let packet = self
                .packets
                .get(start..start + len)
                .ok_or_else(|| bad("a packet's length runs past the track"))?;
            let mut samples = self.decoder.decode_packet(packet)?;
            self.at = start + len;
            let channels = self.decoder.channels();
            let drop = (self.skip * channels).min(samples.len());
            samples.drain(..drop);
            self.skip -= drop / channels;
            if !samples.is_empty() {
                return Ok(Some(Samples::F32(samples)));
            }
        }
    }

    fn seek(&mut self, frame: u64) -> Result<(), MediaError> {
        self.decoder.reset();
        self.at = 0;
        self.skip = usize::try_from(frame).unwrap_or(usize::MAX);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bits written lowest first, as the reader takes them.
    #[derive(Default)]
    struct Bits {
        bytes: Vec<u8>,
        len: usize,
    }

    impl Bits {
        fn put(&mut self, value: u32, n: usize) {
            for i in 0..n {
                if self.len.is_multiple_of(8) {
                    self.bytes.push(0);
                }
                let last = self.bytes.len() - 1;
                self.bytes[last] |= (((value >> i) & 1) as u8) << (self.len % 8);
                self.len += 1;
            }
        }
    }

    /// A span's values, each with whether it is negative.
    type Span = (bool, u32, u32, Vec<(u32, bool)>);

    /// One stereo block of 2,048 coefficients a channel in either coding: two floats, 25 steps,
    /// two spans of values, then runs of zeros to the end.
    fn block(revised: bool) -> Vec<u8> {
        let spans: [Span; 2] = [
            (
                false,
                0,
                3,
                [1, 0, 5, 0, 0, 7, 2, 0].map(|v| (v, v > 2)).to_vec(),
            ),
            (true, 1, 2, (0..24).map(|i| (i % 4, i > 9)).collect()),
        ];
        let mut bits = Bits::default();
        bits.put(0, 2);
        for channel in 0..2u32 {
            // A power, a mantissa and a sign, twice.
            for float in 0..2 {
                bits.put(10 + channel, 5);
                bits.put(0x40_0000 | (0x1234 << float), 23);
                bits.put(float, 1);
            }
            for band in 0..25 {
                bits.put(20 + band + channel, if revised { 7 } else { 8 });
            }
            for (run, code, width, values) in &spans {
                bits.put(u32::from(*run), 1);
                if *run {
                    bits.put(*code, 4);
                }
                bits.put(*width, 4);
                for (value, negative) in values {
                    bits.put(*value, *width as usize);
                    if !revised && *value != 0 {
                        bits.put(u32::from(*negative), 1);
                    }
                }
                if revised {
                    for (_, negative) in values.iter().filter(|v| v.0 != 0) {
                        bits.put(u32::from(*negative), 1);
                    }
                }
            }
            // Four runs of 512 zeros pass the 2,014 coefficients left.
            for _ in 0..4 {
                bits.put(1, 1);
                bits.put(15, 4);
                bits.put(0, 4);
            }
        }
        let mut packet = 7680u32.to_le_bytes().to_vec();
        packet.extend(bits.bytes);
        packet.resize(packet.len().next_multiple_of(4), 0);
        packet
    }

    #[test]
    fn a_kb2n_block_decodes_to_what_the_same_values_do_in_the_older_coding() {
        let mut older = Decoder::new(48_000, 2, true).expect("a track");
        let mut newer = Decoder::new(48_000, 2, true).expect("a track").revised();
        let (old, new) = (block(false), block(true));
        assert!(new.len() < old.len(), "25 bits fewer a channel");
        let want = older.decode_packet(&old).expect("the older coding");
        assert_eq!(want.len(), 2 * 1920);
        assert!(want.iter().any(|s| *s != 0.0));
        assert_eq!(newer.decode_packet(&new).expect("the newer coding"), want);
        // Each coding read as the other is another signal.
        let mut crossed = Decoder::new(48_000, 2, true).expect("a track").revised();
        assert_ne!(crossed.decode_packet(&old).ok(), Some(want));
    }
}
