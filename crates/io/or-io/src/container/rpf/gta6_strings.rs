//! The GTA VI PSO string cipher (`cipherType == 0x13`, white-box representation `504f865e`): a
//! white-box block cipher chained like CBC, which deciphers the enciphered string runs of a PSO
//! document's `LSNS`/`STNS`/`STFN` sections at load. It is a third representation, distinct from the
//! index cipher ([`super::rpf9_toc`]) and the content cipher ([`super::gta6_content`]): the block
//! primitive is the index cipher's transform shape (input substitution, thirteen T-box rounds, output
//! substitution) but keyed from a second, independent set of shared tables, and the mode is CBC
//! decryption seeded by a fixed 16-byte IV held in the executable, restarted for each run.
//!
//! ⚠ The transform, the CBC framing, the whole-block extent and the table/blob shapes are the
//! algorithm, committable per the owner's "truth from the executable" ruling. The key words, the
//! output maps, the two shared tables and the IV are game-derived and live only in the key snapshot,
//! never in the tree.

use byteorder::{BigEndian, ByteOrder};

/// Round-key words in a string key blob; the first four key the input, the rest the thirteen rounds.
pub const WORDS: usize = 60;
/// Substitution maps per key (the output maps) and shared byte/word tables per block byte.
pub const MAPS: usize = 16;
/// Entries in one substitution map or table.
pub const MAP_LEN: usize = 256;
/// Rounds of the T-box transform between the input and output substitutions.
const ROUNDS: usize = 13;

/// The string cipher's key material and its own shared tables, read from the key snapshot.
pub struct StringCipher {
    /// The fixed 16-byte IV, the CBC seed for every run (the same constant for the whole install).
    pub iv: [u8; 16],
    /// The round-key words; the first four key the input, the rest the rounds.
    pub words: [u32; WORDS],
    /// The output byte maps, one per block byte.
    pub output: Box<[[u8; MAP_LEN]; MAPS]>,
    /// The shared input byte tables, one per block byte.
    pub input: Box<[[u8; MAP_LEN]; MAPS]>,
    /// The shared round word tables.
    pub round: Box<[[u32; MAP_LEN]; MAPS]>,
}

impl StringCipher {
    /// Decipher one 16-byte ciphertext block: the input substitution, thirteen T-box rounds, then the
    /// output substitution. This is the white-box block primitive the CBC decryption runs per block.
    fn block(&self, data: &[u8; 16]) -> [u8; 16] {
        let input = &self.input;
        let mut encoded = [0u8; 16];
        for (i, slot) in encoded.iter_mut().enumerate() {
            *slot = input[i][data[i] as usize];
        }
        let w = &self.words;
        let mut state = [0u32; 4];
        for (i, lane) in state.iter_mut().enumerate() {
            *lane = BigEndian::read_u32(&encoded[i * 4..]) ^ w[i];
        }
        let t = &self.round;
        for r in 1..=ROUNDS {
            let [a, b, c, d] = state;
            state = [
                t[0][(a >> 24) as usize]
                    ^ t[13][((d >> 16) & 0xFF) as usize]
                    ^ t[10][((c >> 8) & 0xFF) as usize]
                    ^ t[7][(b & 0xFF) as usize]
                    ^ w[4 * r],
                t[1][((a >> 16) & 0xFF) as usize]
                    ^ t[14][((d >> 8) & 0xFF) as usize]
                    ^ t[4][(b >> 24) as usize]
                    ^ t[11][(c & 0xFF) as usize]
                    ^ w[4 * r + 1],
                t[5][((b >> 16) & 0xFF) as usize]
                    ^ t[2][((a >> 8) & 0xFF) as usize]
                    ^ t[8][(c >> 24) as usize]
                    ^ t[15][(d & 0xFF) as usize]
                    ^ w[4 * r + 2],
                t[9][((c >> 16) & 0xFF) as usize]
                    ^ t[6][((b >> 8) & 0xFF) as usize]
                    ^ t[12][(d >> 24) as usize]
                    ^ t[3][(a & 0xFF) as usize]
                    ^ w[4 * r + 3],
            ];
        }
        let [a, b, c, d] = state;
        let indices = [
            a >> 24,
            d >> 16,
            c >> 8,
            b,
            b >> 24,
            a >> 16,
            d >> 8,
            c,
            c >> 24,
            b >> 16,
            a >> 8,
            d,
            d >> 24,
            c >> 16,
            b >> 8,
            a,
        ];
        let out_maps = &self.output;
        let mut out = [0u8; 16];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = out_maps[i][(indices[i] & 0xFF) as usize];
        }
        out
    }

    /// Decipher `data` in place, CBC-chained from the fixed IV: each 16-byte block is run through
    /// [`Self::block`] and XORed with the previous ciphertext (the IV for the first). Only whole
    /// 16-byte blocks are deciphered; a trailing `< 16` bytes is left clear, as the game leaves it.
    /// The counter is the IV at every call, so a caller deciphers each run with its own call.
    pub fn cbc_decrypt(&self, data: &mut [u8]) {
        let mut prev = self.iv;
        let whole = data.len() & !0xF;
        for chunk in data[..whole].chunks_mut(16) {
            let cipher: [u8; 16] = chunk.try_into().expect("chunks_mut(16) yields 16 bytes");
            let plain = self.block(&cipher);
            for (out, (p, pv)) in chunk.iter_mut().zip(plain.iter().zip(prev.iter())) {
                *out = p ^ pv;
            }
            prev = cipher;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Distinct, deterministic synthetic tables and a key — not the real material, which is never in
    /// the tree. The transform's correctness against the game is proven by the install-gated string
    /// test; these fix the port against regression and exercise the CBC contract.
    fn synthetic() -> StringCipher {
        let mut words = [0u32; WORDS];
        for (i, w) in words.iter_mut().enumerate() {
            *w = (i as u32).wrapping_mul(0x9E37_79B1);
        }
        let mut output = Box::new([[0u8; MAP_LEN]; MAPS]);
        let mut input = Box::new([[0u8; MAP_LEN]; MAPS]);
        for m in 0..MAPS {
            for x in 0..MAP_LEN {
                output[m][x] = ((x * 7 + m * 3) % 256) as u8;
                input[m][x] = ((x * 5 + m * 11 + 1) % 256) as u8;
            }
        }
        let mut round = Box::new([[0u32; MAP_LEN]; MAPS]);
        for (k, table) in round.iter_mut().enumerate() {
            for (j, word) in table.iter_mut().enumerate() {
                *word = ((k * 256 + j) as u32).wrapping_mul(0x0100_1001) ^ 0xA5A5_A5A5;
            }
        }
        StringCipher {
            iv: [0x5Au8; 16],
            words,
            output,
            input,
            round,
        }
    }

    #[test]
    fn the_block_is_deterministic_and_pinned() {
        let c = synthetic();
        let a = c.block(&[0u8; 16]);
        assert_eq!(a, c.block(&[0u8; 16]), "the transform is a pure function");
        // A pinned answer so the port and the snapshot generator cannot drift apart.
        assert_eq!(
            a,
            [
                0x89, 0x85, 0xC3, 0xF4, 0x3B, 0x07, 0xF5, 0x20, 0xE4, 0x6A, 0x5F, 0x58, 0xD1, 0x44,
                0x01, 0xB5
            ],
        );
        let mut other = [0u8; 16];
        other[15] = 1;
        assert_ne!(c.block(&other), a);
    }

    #[test]
    fn cbc_xors_the_block_with_the_previous_cipher() {
        let c = synthetic();
        let mut data: Vec<u8> = (0u8..32).collect();
        let first: [u8; 16] = data[..16].try_into().unwrap();
        let second: [u8; 16] = data[16..].try_into().unwrap();
        c.cbc_decrypt(&mut data);
        let mut want0 = c.block(&first);
        for (wv, iv) in want0.iter_mut().zip(c.iv.iter()) {
            *wv ^= iv;
        }
        let mut want1 = c.block(&second);
        for (wv, pv) in want1.iter_mut().zip(first.iter()) {
            *wv ^= pv;
        }
        assert_eq!(&data[..16], &want0);
        assert_eq!(&data[16..], &want1);
    }

    #[test]
    fn a_trailing_partial_block_is_left_clear() {
        let c = synthetic();
        // 20 bytes = one whole block then four clear.
        let mut data: Vec<u8> = (0u8..20).collect();
        let tail_before = data[16..].to_vec();
        c.cbc_decrypt(&mut data);
        assert_ne!(
            &data[..16],
            &(0u8..16).collect::<Vec<_>>()[..],
            "the block deciphers"
        );
        assert_eq!(
            &data[16..],
            &tail_before[..],
            "the partial tail is untouched"
        );
    }
}
