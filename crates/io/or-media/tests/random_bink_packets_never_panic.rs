//! Bytes that are no Bink packet reach the decoders as a file's corruption would: each is decoded
//! or refused, and none panics, whatever the picture's size, revision or flags.

use or_media::bink::{audio, video};
use or_media::bink2;

/// A small deterministic generator, so a failure repeats.
struct Noise(u64);

impl Noise {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| (self.next() >> 24) as u8).collect()
    }
}

#[test]
fn a_random_picture_packet_is_decoded_or_refused_and_never_panics() {
    const ALPHA: u32 = 0x0010_0000;
    let mut noise = Noise(0x9e37_79b9_7f4a_7c15);
    let mut decoded = 0;
    for revision in *b"fghik" {
        for (width, height, flags) in [(16, 16, 0), (40, 24, 0), (33, 17, ALPHA)] {
            let mut decoder = video::Decoder::new(revision, flags, width, height).expect("a size");
            for _ in 0..200 {
                let len = (noise.next() % 1500) as usize;
                let packet = noise.bytes(len);
                if decoder.decode(&packet).is_ok() {
                    decoded += 1;
                }
            }
            decoder.reset();
        }
    }
    eprintln!("{decoded} of 3000 random picture packets decoded");
}

#[test]
fn a_random_sound_packet_is_decoded_or_refused_and_never_panics() {
    let mut noise = Noise(0x2545_f491_4f6c_dd1d);
    let mut decoded = 0;
    // The DCT coding, the RDFT one, and the DCT coding of a `KB2n` movie.
    for (dct, revised) in [(true, false), (false, false), (true, true)] {
        for (rate, channels) in [(11_025, 1), (22_050, 2), (44_100, 2), (48_000, 1)] {
            let mut decoder = audio::Decoder::new(rate, channels, dct).expect("a track");
            if revised {
                decoder = decoder.revised();
            }
            for _ in 0..200 {
                let len = (noise.next() % 1500) as usize;
                let packet = noise.bytes(len);
                if decoder.decode_packet(&packet).is_ok() {
                    decoded += 1;
                }
            }
            decoder.reset();
        }
    }
    eprintln!("{decoded} of 2400 random sound packets decoded");
}

#[test]
fn a_random_bink2_packet_is_decoded_or_refused_and_never_panics() {
    const ALPHA: u32 = 0x0010_0000;
    let mut noise = Noise(0x1357_9bdf_2468_ace0);
    let mut decoded = 0;
    for (revision, flags) in [(b'g', 0), (b'j', 0x10), (b'j', 0x12), (b'k', 0x13)] {
        for (width, height) in [(32, 32), (72, 40), (60, 102), (130, 130)] {
            for alpha in [0, ALPHA] {
                let mut decoder =
                    bink2::video::Decoder::new(revision, flags | alpha, width, height)
                        .expect("a size");
                for n in 0..150 {
                    // A frame in the coding the decoder reads, whatever its other flags say, then noise.
                    let len = (noise.next() % 1500) as usize + 4;
                    let mut packet = noise.bytes(len);
                    let stated = 0x2000 | (noise.next() as u32 & 0x00DF_C0FF);
                    packet[..4].copy_from_slice(&stated.to_le_bytes());
                    if decoder.decode(&packet, n % 7 == 0).is_ok() {
                        decoded += 1;
                    }
                }
                decoder.reset();
            }
        }
    }
    eprintln!("{decoded} of 4800 random Bink 2 packets decoded");
}
