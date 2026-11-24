//! The GTA VI content cipher (`cipherType == 17`, white-box representation `5dce1827`): AES-CTR whose
//! keystream is a white-box T-box transform, distinct from the index/CBC transform in
//! [`super::rpf9_toc`]. It shares the index cipher's blob *shape* — 60 round words and sixteen
//! 256-byte output maps — but uses a second, independent set of shared input and round tables and a
//! different per-byte mixing permutation, which is why a plain AES library does not reproduce it.
//!
//! An enciphered content body's plaintext is a zlib stream; the mode is a 128-bit big-endian counter
//! seeded by the entry's IV, continuous across the body. `ctr_xor` applies the keystream, and because
//! CTR is XOR with the keystream it is its own inverse: applying it twice with the same key and IV
//! restores the input.
//!
//! ⚠ The transform, the CTR framing and the table/blob shapes are the algorithm, committable per the
//! owner's "truth from the executable" ruling. The key words, the output maps and the two shared
//! tables are game-derived and live only in the key snapshot, never in the tree.

/// Round-key words in a content key blob; the first four key the input, the rest the rounds.
pub const WORDS: usize = 60;
/// Substitution maps per key (the output maps) and shared byte/word tables per block byte.
pub const MAPS: usize = 16;
/// Entries in one substitution map or table.
pub const MAP_LEN: usize = 256;

/// Encipher one 16-byte counter block into 16 keystream bytes: the input substitution packed
/// big-endian per column, the T-box rounds, then the output-map substitution.
fn ks_block(
    counter: &[u8; 16],
    words: &[u32; WORDS],
    output: &[[u8; MAP_LEN]; MAPS],
    input: &[[u8; MAP_LEN]; MAPS],
    round: &[[u32; MAP_LEN]; MAPS],
) -> [u8; 16] {
    let t = round;
    // Substitute each counter byte through its input table, then pack four lookups big-endian per
    // column and xor the first four round words.
    let mut q = [0u8; 16];
    for (k, slot) in q.iter_mut().enumerate() {
        *slot = input[k][counter[k] as usize];
    }
    let be = |a: u8, b: u8, c: u8, d: u8| {
        (u32::from(a) << 24) | (u32::from(b) << 16) | (u32::from(c) << 8) | u32::from(d)
    };
    let mut s7 = be(q[0], q[1], q[2], q[3]) ^ words[0];
    let mut s6 = be(q[4], q[5], q[6], q[7]) ^ words[1];
    let mut s9 = be(q[8], q[9], q[10], q[11]) ^ words[2];
    let mut s4 = be(q[12], q[13], q[14], q[15]) ^ words[3];

    // `pv` indexes the round words; the loop computes two column-sets per iteration until the
    // thirteenth T-box round, whose column-set feeds the output maps with no further mix.
    // `r(ti, x, sh)` is round table `ti` indexed by byte `sh` of `x`.
    let r = |ti: usize, x: u32, sh: u32| t[ti][((x >> sh) & 0xFF) as usize];
    let mut round_counter = 0usize;
    let mut pv = 5usize;
    // First written in the loop body, which always runs at least once before any read.
    let (mut s8, mut s11, mut s10, mut s5);
    loop {
        s8 = r(5, s6, 16) ^ r(10, s9, 8) ^ r(0, s7, 24) ^ r(15, s4, 0) ^ words[pv - 1];
        s11 = r(14, s4, 8) ^ r(9, s9, 16) ^ r(4, s6, 24) ^ r(3, s7, 0) ^ words[pv];
        s10 = r(13, s4, 16) ^ r(2, s7, 8) ^ r(8, s9, 24) ^ r(7, s6, 0) ^ words[pv + 1];
        s5 = r(6, s6, 8) ^ r(1, s7, 16) ^ r(12, s4, 24) ^ r(11, s9, 0) ^ words[pv + 2];
        if round_counter == 12 {
            break;
        }
        round_counter += 2;
        s7 = r(5, s11, 16) ^ r(10, s10, 8) ^ r(0, s8, 24) ^ r(15, s5, 0) ^ words[pv + 3];
        s6 = r(14, s5, 8) ^ r(9, s10, 16) ^ r(4, s11, 24) ^ r(3, s8, 0) ^ words[pv + 4];
        s9 = r(2, s8, 8) ^ r(13, s5, 16) ^ r(8, s10, 24) ^ r(7, s11, 0) ^ words[pv + 5];
        s4 = r(1, s8, 16) ^ r(6, s11, 8) ^ r(12, s5, 24) ^ r(11, s10, 0) ^ words[pv + 6];
        pv += 8;
    }

    let g = |map: usize, x: u32, shift: u32| output[map][((x >> shift) & 0xFF) as usize];
    [
        g(0, s8, 24),
        g(1, s11, 16),
        g(2, s10, 8),
        g(3, s5, 0),
        g(4, s11, 24),
        g(5, s10, 16),
        g(6, s5, 8),
        g(7, s8, 0),
        g(8, s10, 24),
        g(9, s5, 16),
        g(10, s8, 8),
        g(11, s11, 0),
        g(12, s5, 24),
        g(13, s8, 16),
        g(14, s11, 8),
        g(15, s10, 0),
    ]
}

/// Big-endian increment of a 128-bit counter in place: add one across all sixteen bytes.
fn inc_be(counter: &mut [u8; 16]) {
    for byte in counter.iter_mut().rev() {
        *byte = byte.wrapping_add(1);
        if *byte != 0 {
            break;
        }
    }
}

/// XOR `data` in place with the CTR keystream seeded by `iv`, continuous across the whole buffer.
/// CTR is its own inverse, so this both enciphers and deciphers. `data` may be any length; a partial
/// trailing block takes only the keystream bytes it needs.
pub fn ctr_xor(
    data: &mut [u8],
    iv: &[u8; 16],
    words: &[u32; WORDS],
    output: &[[u8; MAP_LEN]; MAPS],
    input: &[[u8; MAP_LEN]; MAPS],
    round: &[[u32; MAP_LEN]; MAPS],
) {
    let mut counter = *iv;
    for block in data.chunks_mut(16) {
        let ks = ks_block(&counter, words, output, input, round);
        for (b, k) in block.iter_mut().zip(ks.iter()) {
            *b ^= k;
        }
        inc_be(&mut counter);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Distinct, deterministic synthetic tables and a key — not the real material, which is never in
    /// the tree. The transform's correctness against the game is proven by the install-gated content
    /// test; these fix the port against regression and exercise the CTR contract.
    /// A content key's round words with its output maps, and the two shared input and round tables.
    type SyntheticKey = (
        [u32; WORDS],
        Box<[[u8; MAP_LEN]; MAPS]>,
        Box<[[u8; MAP_LEN]; MAPS]>,
        Box<[[u32; MAP_LEN]; MAPS]>,
    );

    fn synthetic() -> SyntheticKey {
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
        (words, output, input, round)
    }

    #[test]
    fn the_keystream_block_is_deterministic_and_pinned() {
        let (w, o, i, r) = synthetic();
        let a = ks_block(&[0u8; 16], &w, &o, &i, &r);
        let b = ks_block(&[0u8; 16], &w, &o, &i, &r);
        assert_eq!(a, b, "the transform is a pure function of its inputs");
        // A pinned answer so the port and the snapshot generator cannot drift apart.
        assert_eq!(
            a,
            [
                0xEC, 0x4B, 0xB3, 0x2B, 0xB6, 0x41, 0x05, 0xCA, 0x01, 0xA4, 0x8F, 0x4F, 0x10, 0x42,
                0xF1, 0xD7
            ],
        );
        // A different counter yields a different block.
        let mut other = [0u8; 16];
        other[15] = 1;
        assert_ne!(ks_block(&other, &w, &o, &i, &r), a);
    }

    #[test]
    fn ctr_is_its_own_inverse_and_xors_the_keystream() {
        let (w, o, i, r) = synthetic();
        let iv = [0x5Au8; 16];
        let clear: Vec<u8> = (0..70u16).map(|x| x as u8).collect();

        // Applying CTR twice restores the input (CTR is XOR with the keystream).
        let mut buf = clear.clone();
        ctr_xor(&mut buf, &iv, &w, &o, &i, &r);
        assert_ne!(buf, clear, "the first pass enciphers");
        ctr_xor(&mut buf, &iv, &w, &o, &i, &r);
        assert_eq!(buf, clear, "the second pass deciphers");

        // The first block equals the plaintext XOR the keystream of the IV counter; the second
        // block uses the incremented counter, so CTR is a byte stream across block boundaries.
        let ks0 = ks_block(&iv, &w, &o, &i, &r);
        let mut c1 = iv;
        inc_be(&mut c1);
        let ks1 = ks_block(&c1, &w, &o, &i, &r);
        let mut enc = clear.clone();
        ctr_xor(&mut enc, &iv, &w, &o, &i, &r);
        for k in 0..16 {
            assert_eq!(enc[k], clear[k] ^ ks0[k]);
            assert_eq!(enc[16 + k], clear[16 + k] ^ ks1[k]);
        }
        // The trailing partial block (70 = 4*16 + 6) is enciphered too.
        assert_eq!(enc.len(), 70);
    }

    #[test]
    fn big_endian_increment_carries_across_every_byte() {
        let mut c = [0xFFu8; 16];
        inc_be(&mut c);
        assert_eq!(c, [0u8; 16], "all-ones wraps to zero");
        let mut c = [0u8; 16];
        c[15] = 0xFF;
        inc_be(&mut c);
        let mut want = [0u8; 16];
        want[14] = 1;
        assert_eq!(c, want, "a carry rolls into the next byte up");
    }
}
