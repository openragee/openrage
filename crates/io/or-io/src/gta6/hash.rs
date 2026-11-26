//! GTA VI's two data hashes: a 64-bit one for string values and texture keys, and the 32-bit
//! name hash. Both hash the text with `A` to `Z` folded to lower case.

/// The low half of MurmurHash3's x64 128-bit hash, seed 0, of `text` with ASCII case folded.
pub fn name_hash64(text: &[u8]) -> u64 {
    murmur3_x64_128_low(text, true)
}

/// The same 64-bit hash of `text` as it is spelled, no case folded: what names a script's types and fields.
pub fn literal_hash64(text: &[u8]) -> u64 {
    murmur3_x64_128_low(text, false)
}

/// The same hash of `bytes` as they are, which tells one file's bytes from another's.
pub(crate) fn bytes_hash64(bytes: &[u8]) -> u64 {
    murmur3_x64_128_low(bytes, false)
}

/// The 32-bit hash of `text`: `atStringHash`, which folds case before it hashes.
pub const fn name_hash32(text: &[u8]) -> u32 {
    crate::hash::string_hash(text)
}

const C1: u64 = 0x87C3_7B91_1142_53D5;
const C2: u64 = 0x4CF5_AD43_2745_937F;

/// MurmurHash3's 64-bit finaliser.
pub(crate) fn fmix64(mut k: u64) -> u64 {
    k ^= k >> 33;
    k = k.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    k ^= k >> 33;
    k = k.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    k ^ (k >> 33)
}

fn murmur3_x64_128_low(text: &[u8], fold: bool) -> u64 {
    let byte = |b: u8| if fold { b.to_ascii_lowercase() } else { b };
    let word = |bytes: &[u8]| {
        bytes
            .iter()
            .rev()
            .fold(0u64, |w, b| (w << 8) | u64::from(byte(*b)))
    };
    let mix1 = |k: u64| k.wrapping_mul(C1).rotate_left(31).wrapping_mul(C2);
    let mix2 = |k: u64| k.wrapping_mul(C2).rotate_left(33).wrapping_mul(C1);
    let (mut h1, mut h2) = (0u64, 0u64);
    let (blocks, tail) = text.as_chunks::<16>();
    for block in blocks {
        h1 ^= mix1(word(&block[..8]));
        h1 = h1
            .rotate_left(27)
            .wrapping_add(h2)
            .wrapping_mul(5)
            .wrapping_add(0x52DC_E729);
        h2 ^= mix2(word(&block[8..]));
        h2 = h2
            .rotate_left(31)
            .wrapping_add(h1)
            .wrapping_mul(5)
            .wrapping_add(0x3849_5AB5);
    }
    if tail.len() > 8 {
        h2 ^= mix2(word(&tail[8..]));
    }
    if !tail.is_empty() {
        h1 ^= mix1(word(&tail[..tail.len().min(8)]));
    }
    let len = text.len() as u64;
    h1 ^= len;
    h2 ^= len;
    h1 = h1.wrapping_add(h2);
    h2 = h2.wrapping_add(h1);
    h1 = fmix64(h1);
    h2 = fmix64(h2);
    h1.wrapping_add(h2)
}

/// [`name_hash64`] of a text given in parts: the hash of many texts that start alike costs each
/// only its own end.
#[derive(Debug, Clone, Copy, Default)]
pub struct NameHasher {
    h1: u64,
    h2: u64,
    len: u64,
    /// The bytes after the last whole block of 16, case folded.
    tail: [u8; 16],
    held: usize,
}

impl NameHasher {
    /// The state with `text` after what it holds.
    pub fn with(mut self, text: &[u8]) -> Self {
        for byte in text {
            self.tail[self.held] = byte.to_ascii_lowercase();
            self.held += 1;
            if self.held == self.tail.len() {
                let (k1, k2) = (word(&self.tail[..8]), word(&self.tail[8..]));
                self.h1 ^= mix1(k1);
                self.h1 = (self.h1.rotate_left(27))
                    .wrapping_add(self.h2)
                    .wrapping_mul(5)
                    .wrapping_add(0x52DC_E729);
                self.h2 ^= mix2(k2);
                self.h2 = (self.h2.rotate_left(31))
                    .wrapping_add(self.h1)
                    .wrapping_mul(5)
                    .wrapping_add(0x3849_5AB5);
                self.held = 0;
            }
        }
        self.len += text.len() as u64;
        self
    }

    /// The hash of the text given so far.
    pub fn finish(&self) -> u64 {
        let tail = &self.tail[..self.held];
        let (mut h1, mut h2) = (self.h1, self.h2);
        if tail.len() > 8 {
            h2 ^= mix2(word(&tail[8..]));
        }
        if !tail.is_empty() {
            h1 ^= mix1(word(&tail[..tail.len().min(8)]));
        }
        h1 ^= self.len;
        h2 ^= self.len;
        h1 = h1.wrapping_add(h2);
        h2 = h2.wrapping_add(h1);
        fmix64(h1).wrapping_add(fmix64(h2))
    }
}

/// The little-endian word of up to eight bytes.
fn word(bytes: &[u8]) -> u64 {
    (bytes.iter().rev()).fold(0, |w, b| (w << 8) | u64::from(*b))
}

fn mix1(k: u64) -> u64 {
    k.wrapping_mul(C1).rotate_left(31).wrapping_mul(C2)
}

fn mix2(k: u64) -> u64 {
    k.wrapping_mul(C2).rotate_left(33).wrapping_mul(C1)
}

/// For each group of a linear-probing table, how many groups directly before it, cyclically, are full.
pub(crate) fn full_before(full: &[bool]) -> Vec<usize> {
    let n = full.len();
    let Some(open) = full.iter().position(|f| !f) else {
        return vec![n; n];
    };
    let mut run = vec![0; n];
    for k in 1..=n {
        let (g, before) = ((open + k) % n, (open + k - 1) % n);
        run[g] = if full[before] { run[before] + 1 } else { 0 };
    }
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_before_counts_the_full_groups_a_probe_crosses() {
        for full in [
            &[true, false, true, true, false, true][..],
            &[false; 4],
            &[true; 3],
            &[true, true, false],
        ] {
            let n = full.len();
            let walked: Vec<usize> = (0..n)
                .map(|g| (1..=n).take_while(|d| full[(g + n - d) % n]).count())
                .collect();
            assert_eq!(full_before(full), walked, "{full:?}");
        }
    }

    #[test]
    fn the_64_bit_hash_is_murmur3_s_published_low_half() {
        // MurmurHash3_x64_128, seed 0, as its reference implementation prints them.
        assert_eq!(murmur3_x64_128_low(b"", false), 0);
        assert_eq!(murmur3_x64_128_low(b"hello", false), 0xCBD8_A7B3_41BD_9B02);
        assert_eq!(
            murmur3_x64_128_low(b"The quick brown fox jumps over the lazy dog", false),
            0xE34B_BC7B_BC07_1B6C
        );
    }

    #[test]
    fn a_text_hashed_in_parts_hashes_as_the_whole() {
        // Longer than three blocks, so a cut falls before, on and after each block's end.
        let text = b"Zq_Shed_07_Lod_c1_012_abal and the rest of a made-up name";
        for len in 0..=text.len() {
            let whole = name_hash64(&text[..len]);
            for cut in 0..=len {
                let start = NameHasher::default().with(&text[..cut]);
                assert_eq!(
                    start.with(&text[cut..len]).finish(),
                    whole,
                    "{cut} of {len}"
                );
                // A start is kept to end many texts.
                assert_eq!(start.finish(), name_hash64(&text[..cut]), "{cut}");
            }
        }
        let parts = NameHasher::default()
            .with(b"zq")
            .with(b"")
            .with(b"_")
            .with(b"AB");
        assert_eq!(parts.finish(), name_hash64(b"zq_ab"));
    }

    #[test]
    fn both_hashes_fold_case_and_nothing_else() {
        let sentence = b"The quick brown fox jumps over the lazy dog";
        let lower = sentence.to_ascii_lowercase();
        assert_eq!(name_hash64(sentence), murmur3_x64_128_low(&lower, false));
        assert_ne!(name_hash64(sentence), murmur3_x64_128_low(sentence, false));
        assert_eq!(name_hash64(b"HELLO"), 0xCBD8_A7B3_41BD_9B02);
        // Jenkins' one-at-a-time hash of "a", as its author publishes it.
        assert_eq!(name_hash32(b"A"), 0xCA2E_9442);
        assert_eq!(name_hash32(b"a"), crate::hash::literal_string_hash(b"a"));
    }
}
