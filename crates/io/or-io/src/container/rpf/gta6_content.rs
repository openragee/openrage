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

/// The five content-framing tuples `(F0, F1, F2, F3, F4)` the whole install uses, read from the
/// cipher record's bytes 24..64 as five little-endian u64. A record whose tuple is not one of these is
/// refused rather than guessed ([`framing_reject_reason`]). `F4` is 4 in every one; its meaning is not
/// established.
pub const KNOWN_FRAMINGS: [[u64; 5]; 5] = [
    [0x400, 0x400, 0x1C00, 0, 4],
    [0x1000, 0x1000, 0x31000, 0x1000, 4],
    [0x1000, 0x1000, 0, 0x1000, 4],
    [0, 0x1000, 0, 0, 4],
    [0, 0x400, 0, 0, 4],
];

/// Refuse unknown framing or an unaligned tail; known head and tail windows are clipped to the body.
pub fn framing_reject_reason(framing: &[u64; 5], len: u64) -> Option<String> {
    if framing[4] != 4 {
        return Some(format!("its framing word F4 is {}, not 4", framing[4]));
    }
    if !KNOWN_FRAMINGS.contains(framing) {
        return Some(format!(
            "its framing tuple {framing:x?} is not one of the five the install uses"
        ));
    }
    // The absolute counter is `offset / 16`, so every enciphered range must start 16-aligned. A tail
    // (F3 > 0) starts at `len - F3`; with the five known tuples F3 is a 16-multiple, so an unaligned
    // body would mis-key that range. Refuse rather than silently truncate the counter.
    if framing[3] > 0 && !len.is_multiple_of(16) {
        return Some(format!(
            "its framing tail F3 is set but the {len:#x}-byte body is not 16-aligned"
        ));
    }
    None
}

/// The byte ranges of a body of `len` bytes that are enciphered, under a framing tuple: the head
/// `[0, F0)`, then (when `F2 == 0`) the whole `[F0, len)` or else the window `[s, s + F1)` repeating
/// every `F1 + F2` from `F0`, plus the tail `[len - F3, len)`. The ranges are returned merged,
/// disjoint and sorted, so applying each one XORs no byte twice. Every boundary is 16-aligned for the
/// five known tuples, which the caller relies on for the absolute counter.
pub fn enciphered_ranges(framing: &[u64; 5], len: u64) -> Vec<(u64, u64)> {
    let [f0, f1, f2, f3, _] = *framing;
    let mut ranges: Vec<(u64, u64)> = Vec::new();
    let mut pos = 0u64;
    if f0 > 0 {
        ranges.push((0, f0.min(len)));
        pos = f0;
    }
    if f1 > 0 {
        if f2 == 0 {
            if pos < len {
                ranges.push((pos, len));
            }
        } else {
            // Saturating arithmetic so an out-of-range framing refuses (empty cover) rather than
            // panicking under overflow checks; the validated tuples never reach these bounds.
            let period = f1.saturating_add(f2);
            while pos < len {
                ranges.push((pos, pos.saturating_add(f1).min(len)));
                pos = pos.saturating_add(period);
            }
        }
    }
    if f3 > 0 {
        ranges.push((len.saturating_sub(f3), len));
    }
    merge_ranges(ranges)
}

/// Coalesce `ranges` into a sorted, disjoint cover: empties dropped, overlapping or byte-adjacent
/// ranges joined. Adjacency matters — the head and the first periodic window abut at `F0`.
fn merge_ranges(mut ranges: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    ranges.retain(|(a, b)| a < b);
    ranges.sort_unstable();
    let mut out: Vec<(u64, u64)> = Vec::with_capacity(ranges.len());
    for (a, b) in ranges {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// A 128-bit big-endian counter advanced by `blocks`: the keystream for body offset `a` starts from
/// the record IV advanced by `a / 16`, so a range at an absolute offset gets the same keystream the
/// game would, never a per-range restart. Overflow wraps, as the block increment does.
pub fn advance_counter_be(iv: &[u8; 16], blocks: u64) -> [u8; 16] {
    u128::from_be_bytes(*iv)
        .wrapping_add(blocks as u128)
        .to_be_bytes()
}

/// Apply the content cipher over `data`'s enciphered extent (`framing`), calling `decrypt_range` for
/// each enciphered range with the slice and the IV already advanced to that range's absolute block.
/// The caller supplies the actual CTR pass (which holds the key material); this owns the extent rule
/// and the absolute-counter advance, so the two are tested together without the keys.
pub fn decrypt_extent<F>(data: &mut [u8], framing: &[u64; 5], iv: &[u8; 16], mut decrypt_range: F)
where
    F: FnMut(&mut [u8], &[u8; 16]),
{
    let len = data.len() as u64;
    for (a, b) in enciphered_ranges(framing, len) {
        let advanced = advance_counter_be(iv, a / 16);
        decrypt_range(&mut data[a as usize..b as usize], &advanced);
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

    #[test]
    fn advance_by_a_block_equals_one_increment() {
        let iv = [0x5Au8; 16];
        let mut once = iv;
        inc_be(&mut once);
        assert_eq!(advance_counter_be(&iv, 1), once);
        // Advancing by n and then once equals advancing by n + 1.
        let mut n_then_one = advance_counter_be(&iv, 40);
        inc_be(&mut n_then_one);
        assert_eq!(advance_counter_be(&iv, 41), n_then_one);
        // The low word carries into the next byte up.
        let mut iv = [0u8; 16];
        iv[15] = 0xFF;
        assert_eq!(advance_counter_be(&iv, 1)[14], 1);
    }

    #[test]
    fn the_head_and_first_window_merge_and_the_period_follows() {
        // Tuple 1 on a 0x5000 body: [0, 0x800) (head + first window), then 0x400 every 0x2000.
        assert_eq!(
            enciphered_ranges(&[0x400, 0x400, 0x1C00, 0, 4], 0x5000),
            vec![(0, 0x800), (0x2400, 0x2800), (0x4400, 0x4800)],
        );
        // F2 == 0 enciphers the whole body from F0.
        assert_eq!(
            enciphered_ranges(&[0, 0x400, 0, 0, 4], 0x1000),
            vec![(0, 0x1000)],
        );
        // The 0x1000 tuple: head + first window merge to [0, 0x2000), the tail closes the body.
        let r = enciphered_ranges(&[0x1000, 0x1000, 0x31000, 0x1000, 4], 0x34000);
        assert_eq!(r.first(), Some(&(0, 0x2000)));
        assert_eq!(r.last(), Some(&(0x33000, 0x34000)));
    }

    #[test]
    fn framing_validation_refuses_what_it_cannot_recognise() {
        assert!(framing_reject_reason(&[0x400, 0x400, 0x1C00, 0, 4], 0x10000).is_none());
        assert!(
            framing_reject_reason(&[0x400, 0x400, 0x1C00, 0, 5], 0x10000).is_some(),
            "F4 != 4 is refused"
        );
        assert!(
            framing_reject_reason(&[0x123, 0, 0, 0, 4], 0x10000).is_some(),
            "an unknown tuple is refused"
        );
        assert!(
            framing_reject_reason(&[0x1000, 0x1000, 0, 0x1000, 4], 0x100).is_none(),
            "the known tail covers a shorter body in full"
        );
    }

    #[test]
    fn the_extent_uses_an_absolute_counter_not_a_per_range_restart() {
        let (w, o, i, r) = synthetic();
        let iv = [0x5Au8; 16];
        // Tuple 1 over a body long enough for several periodic windows past the first 0x800.
        let framing = [0x400, 0x400, 0x1C00, 0, 4];
        let clear: Vec<u8> = (0..0x5000u32).map(|x| (x ^ (x >> 5)) as u8).collect();

        let mut enc = clear.clone();
        decrypt_extent(&mut enc, &framing, &iv, |slice, ctr| {
            ctr_xor(slice, ctr, &w, &o, &i, &r)
        });
        assert_ne!(
            enc, clear,
            "the enciphered extent differs from the plaintext"
        );
        // The clear regions (e.g. [0x800, 0x2400)) are untouched.
        assert_eq!(&enc[0x800..0x2400], &clear[0x800..0x2400]);

        // Applying the same absolute-counter pass again restores the plaintext (CTR is symmetric).
        let mut back = enc.clone();
        decrypt_extent(&mut back, &framing, &iv, |slice, ctr| {
            ctr_xor(slice, ctr, &w, &o, &i, &r)
        });
        assert_eq!(
            back, clear,
            "the extent round-trips under the absolute counter"
        );

        // A pass that restarts the counter at the base IV for every range (ignoring the advance) must
        // NOT restore the body: the windows past the first are keyed at the wrong counter.
        let mut wrong = enc.clone();
        decrypt_extent(&mut wrong, &framing, &iv, |slice, _advanced| {
            ctr_xor(slice, &iv, &w, &o, &i, &r)
        });
        assert_ne!(wrong, clear, "a per-range counter restart decrypts wrong");
    }
}
