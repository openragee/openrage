//! The GTA VI story-save payload cipher: the keystream generator recovered from the Xbox build's
//! `game_scarlett_master.exe`, and the payload decrypt and encrypt built on it.
//!
//! A story save's `ASAV` payload (a `SARC00xx` container's data blob, header mode 1 at +8) is
//! XORed with a keystream before it is stored; settings and player records (modes 2 and 3) are
//! plain. The keystream is the output of a table-driven AES-shaped block function measured at
//! `0x145645D70` (`KeystreamBlock`), run in counter mode: the 16-byte counter starts at a fixed
//! seed and counts up little-endian from its last byte, and every block's output is 16 bytes of
//! keystream. The block function's tables, its round keys and its final sixteen S-boxes are static
//! data of the executable; the round keys and S-boxes travel together in one 0x10F4-byte key blob
//! that starts with the magic `AA 38 AA 57` (`0x57AA38AA`).
//!
//! Because the seed and the key material are fixed, every story save ever written by this build of
//! the game starts with the same keystream: two saves' payloads XORed together show the two-time
//! pad (a 61-byte zero run where the two `JSAV` heads match, ~7% zeros overall). Recovering the
//! generator once — by reading the tables out of one's own executable, the way this module does —
//! decrypts and re-encrypts every save of that build, of any length.
//!
//! Measured on `GTA6.Xbox.v1.0.2.1` (`game_scarlett_master.exe`, SHA-verified by the loader):
//! the key blob at RVA `0x66FC060`, the counter seed at RVA `0x62FDA90`, the sixteen byte tables
//! of the initial state at RVA `0x60C0BE0` and the sixteen T-tables of the rounds at the RVAs
//! [`T_TABLE_RVAS`] lists, all read only. The block function was verified against the same
//! executable's own code emulated instruction by instruction: the two reference vectors of
//! [`Self::keystream_block`] reproduce the emulated output exactly, and every story save of the
//! profile decrypts to a `JSAV` version 3 container through it.

/// The bytes of the key blob the round keys and the final S-boxes are read from.
pub const KEY_BLOB: usize = 0x10F4;
/// The key blob's first four bytes, `AA 38 AA 57` read little-endian.
pub const KEY_MAGIC: u32 = 0x57AA_38AA;
/// The RVA of the key blob in `game_scarlett_master.exe` 1.0.2.1.
pub const KEY_BLOB_RVA: u32 = 0x66FC_060;
/// The RVA of the 16-byte counter seed in `game_scarlett_master.exe` 1.0.2.1.
pub const IV_RVA: u32 = 0x62FD_A90;
/// The RVA of the initial state's sixteen byte tables, 256 bytes each.
pub const INIT_TABLES_RVA: u32 = 0x60C_0BE0;
/// The RVAs of the round function's sixteen T-tables, 1024 bytes each, in the order the block
/// function reads them: the first four are the first half-step's `u8` quadruple, the next four
/// its `u11`, then `u10` and `u5`; the second half-step reuses them in the order its own four
/// state words list.
pub const T_TABLE_RVAS: [u32; 16] = [
    0x60C_2FE0, 0x60C_43E0, 0x60C_1BE0, 0x60C_57E0, 0x60C_53E0, 0x60C_3FE0, 0x60C_2BE0, 0x60C_27E0,
    0x60C_4FE0, 0x60C_23E0, 0x60C_3BE0, 0x60C_37E0, 0x60C_33E0, 0x60C_1FE0, 0x60C_4BE0, 0x60C_47E0,
];
/// The counter seed measured at `IV_RVA` of the 1.0.2.1 build, kept as the fallback a build whose
/// sections move needs.
pub const MEASURED_IV: [u8; 16] = [
    0x63, 0x70, 0xED, 0xC3, 0xC5, 0x8A, 0xA5, 0x6E, 0xAA, 0xC4, 0x55, 0x0B, 0x68, 0x55, 0x6E, 0x06,
];
/// The keystream block the measured seed must produce, the first half of the reference vector the
/// emulated executable's own block function gave.
pub const REFERENCE_BLOCK: [u8; 16] = [
    0x89, 0xA9, 0x5E, 0xD1, 0x64, 0xB3, 0x8E, 0x9D, 0x4B, 0x3E, 0x97, 0xB2, 0xD2, 0xF5, 0x20, 0xAD,
];

/// Why a cipher is not read from an executable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CryptoError {
    #[error(
        "the bytes are not a PE image: no MZ or no PE\\0\\0 at the offset the DOS header states"
    )]
    NotAnImage,
    #[error("RVA {rva:#X} lies in no section of the image")]
    NoSection { rva: u32 },
    #[error("the key blob at {at:#X} starts {found:#010X}, not the magic {KEY_MAGIC:#010X}")]
    BadMagic { at: usize, found: u32 },
    #[error("the image ends at {len} bytes, before the {what} it should hold ends")]
    Truncated { what: &'static str, len: usize },
    #[error(
        "the keystream block of the measured seed is {found:02X?}, not the reference \
         {REFERENCE_BLOCK:02X?}: the tables do not belong to the measured build"
    )]
    ReferenceMismatch { found: [u8; 16] },
}

/// The story-save payload cipher of one build: the block function's tables, its key blob's round
/// keys and final S-boxes, and the counter seed every payload's keystream starts from.
#[derive(Debug, Clone)]
pub struct SaveCipher {
    init_tables: [[u8; 256]; 16],
    t_tables: [[u32; 256]; 16],
    round_keys: [u32; 60],
    sboxes: [[u8; 256]; 16],
    seed: [u8; 16],
}

impl SaveCipher {
    /// The cipher of an Xbox build's `game_scarlett_master.exe`, read from its own bytes: the
    /// tables, the key blob and the counter seed, then checked against the reference vector so a
    /// build the tables were not measured for is refused, not misread.
    pub fn from_exe(exe: &[u8]) -> Result<Self, CryptoError> {
        if &exe[0..2] != b"MZ" {
            return Err(CryptoError::NotAnImage);
        }
        let dos = u16::from_le_bytes([exe[0x3C], exe[0x3D]]) as usize;
        if exe.get(dos..dos + 4) != Some(b"PE\0\0") {
            return Err(CryptoError::NotAnImage);
        }
        let count = u16::from_le_bytes([exe[dos + 6], exe[dos + 7]]) as usize;
        let table = dos + 0x18 + u16::from_le_bytes([exe[dos + 0x14], exe[dos + 0x15]]) as usize;
        let offset = |rva: u32| -> Result<usize, CryptoError> {
            for i in 0..count {
                let s = table + i * 40;
                let va = u32::from_le_bytes(exe[s + 12..s + 16].try_into().unwrap());
                let raw = u32::from_le_bytes(exe[s + 16..s + 20].try_into().unwrap());
                let size = u32::from_le_bytes(exe[s + 20..s + 24].try_into().unwrap());
                if va <= rva && rva < va + size.max(raw) {
                    return Ok(rva as usize - va as usize + raw as usize);
                }
            }
            Err(CryptoError::NoSection { rva })
        };
        let read = |rva: u32, len: usize, what: &'static str| -> Result<&[u8], CryptoError> {
            let at = offset(rva)?;
            exe.get(at..at + len).ok_or(CryptoError::Truncated {
                what,
                len: exe.len(),
            })
        };
        let mut init_tables = [[0u8; 256]; 16];
        for (i, table) in init_tables.iter_mut().enumerate() {
            let rva = INIT_TABLES_RVA + (i as u32) * 0x100;
            table.copy_from_slice(read(rva, 0x100, "an initial state table")?);
        }
        let mut t_tables = [[0u32; 256]; 16];
        for (i, table) in t_tables.iter_mut().enumerate() {
            let bytes = read(T_TABLE_RVAS[i], 0x400, "a round table")?;
            for (j, word) in table.iter_mut().enumerate() {
                *word = u32::from_le_bytes(bytes[j * 4..j * 4 + 4].try_into().unwrap());
            }
        }
        let blob = read(KEY_BLOB_RVA, KEY_BLOB, "the key blob")?;
        let magic = u32::from_le_bytes(blob[0..4].try_into().unwrap());
        if magic != KEY_MAGIC {
            return Err(CryptoError::BadMagic {
                at: KEY_BLOB_RVA as usize,
                found: magic,
            });
        }
        let mut round_keys = [0u32; 60];
        for (i, word) in round_keys.iter_mut().enumerate() {
            *word = u32::from_le_bytes(blob[4 + i * 4..8 + i * 4].try_into().unwrap());
        }
        let mut sboxes = [[0u8; 256]; 16];
        for (i, sbox) in sboxes.iter_mut().enumerate() {
            sbox.copy_from_slice(&blob[4 + 240 + i * 256..4 + 240 + (i + 1) * 256]);
        }
        let seed = match read(IV_RVA, 16, "the counter seed") {
            Ok(bytes) => bytes.try_into().unwrap(),
            Err(CryptoError::NoSection { .. }) => MEASURED_IV,
            Err(e) => return Err(e),
        };
        let cipher = Self {
            init_tables,
            t_tables,
            round_keys,
            sboxes,
            seed,
        };
        if cipher.keystream_block(&cipher.seed) != REFERENCE_BLOCK {
            return Err(CryptoError::ReferenceMismatch {
                found: cipher.keystream_block(&cipher.seed),
            });
        }
        Ok(cipher)
    }

    /// The counter seed every payload's keystream starts from.
    pub fn seed(&self) -> &[u8; 16] {
        &self.seed
    }

    /// The 16 bytes of keystream one counter block yields: the initial state from the seed's bytes
    /// through the sixteen byte tables, then six and a half rounds of the T-tables' quarter
    /// lookups against the round keys, then the sixteen final S-boxes. The layout is the
    /// executable's own, kept exactly: the order of the table reads is what makes the keystream.
    pub fn keystream_block(&self, counter: &[u8; 16]) -> [u8; 16] {
        let [t0, t1, t2, t3, t4, t5, t6, t7, t8, t9, t10, t11, t12, t13, t14, t15] = &self.t_tables;
        let [i0, i1, i2, i3, i4, i5, i6, i7, i8, i9, i10, i11, i12, i13, i14, i15] =
            &self.init_tables;
        let rk = &self.round_keys;
        let mut s0 = (i0[counter[0] as usize] as u32) << 24
            | (i1[counter[1] as usize] as u32) << 16
            | (i2[counter[2] as usize] as u32) << 8
            | i3[counter[3] as usize] as u32;
        s0 ^= rk[0];
        let mut s1 = (i4[counter[4] as usize] as u32) << 24
            | (i5[counter[5] as usize] as u32) << 16
            | (i6[counter[6] as usize] as u32) << 8
            | i7[counter[7] as usize] as u32;
        s1 ^= rk[1];
        let mut s2 = (i8[counter[8] as usize] as u32) << 24
            | (i9[counter[9] as usize] as u32) << 16
            | (i10[counter[10] as usize] as u32) << 8
            | i11[counter[11] as usize] as u32;
        s2 ^= rk[2];
        let mut s3 = (i12[counter[12] as usize] as u32) << 24
            | (i13[counter[13] as usize] as u32) << 16
            | (i14[counter[14] as usize] as u32) << 8
            | i15[counter[15] as usize] as u32;
        s3 ^= rk[3];
        let (mut u8_, mut u11, mut u10, mut u5) = (0u32, 0u32, 0u32, 0u32);
        let mut ki = 4usize;
        let mut rounds = 0u32;
        loop {
            u8_ = t0[(s1 >> 16) as usize & 255]
                ^ t1[(s2 >> 8) as usize & 255]
                ^ t2[(s0 >> 24) as usize & 255]
                ^ t3[s3 as usize & 255]
                ^ rk[ki];
            u11 = t4[(s3 >> 8) as usize & 255]
                ^ t5[(s2 >> 16) as usize & 255]
                ^ t6[(s1 >> 24) as usize & 255]
                ^ t7[s0 as usize & 255]
                ^ rk[ki + 1];
            u10 = t8[(s3 >> 16) as usize & 255]
                ^ t9[(s0 >> 8) as usize & 255]
                ^ t10[(s2 >> 24) as usize & 255]
                ^ t11[s1 as usize & 255]
                ^ rk[ki + 2];
            u5 = t12[(s1 >> 8) as usize & 255]
                ^ t13[(s0 >> 16) as usize & 255]
                ^ t14[(s3 >> 24) as usize & 255]
                ^ t15[s2 as usize & 255]
                ^ rk[ki + 3];
            rounds += 2;
            if rounds == 14 {
                break;
            }
            let b1 = (u10 >> 16) as usize & 255;
            let b2 = (u8_ >> 8) as usize & 255;
            let b3 = (u5 >> 16) as usize & 255;
            s0 = t0[(u11 >> 16) as usize & 255]
                ^ t1[(u10 >> 8) as usize & 255]
                ^ t2[(u8_ >> 24) as usize & 255]
                ^ t3[u5 as usize & 255]
                ^ rk[ki + 4];
            s1 = t4[(u5 >> 8) as usize & 255]
                ^ t5[b1]
                ^ t6[(u11 >> 24) as usize & 255]
                ^ t7[u8_ as usize & 255]
                ^ rk[ki + 5];
            s2 = t9[b2]
                ^ t8[b3]
                ^ t10[(u10 >> 24) as usize & 255]
                ^ t11[u11 as usize & 255]
                ^ rk[ki + 6];
            s3 = t13[(u8_ >> 16) as usize & 255]
                ^ t12[(u11 >> 8) as usize & 255]
                ^ t14[(u5 >> 24) as usize & 255]
                ^ t15[u10 as usize & 255]
                ^ rk[ki + 7];
            ki += 8;
        }
        [
            self.sboxes[0][(u8_ >> 24) as usize & 255],
            self.sboxes[1][(u11 >> 16) as usize & 255],
            self.sboxes[2][(u10 >> 8) as usize & 255],
            self.sboxes[3][u5 as usize & 255],
            self.sboxes[4][(u11 >> 24) as usize & 255],
            self.sboxes[5][(u10 >> 16) as usize & 255],
            self.sboxes[6][(u5 >> 8) as usize & 255],
            self.sboxes[7][u8_ as usize & 255],
            self.sboxes[8][(u10 >> 24) as usize & 255],
            self.sboxes[9][(u5 >> 16) as usize & 255],
            self.sboxes[10][(u8_ >> 8) as usize & 255],
            self.sboxes[11][u11 as usize & 255],
            self.sboxes[12][(u5 >> 24) as usize & 255],
            self.sboxes[13][(u8_ >> 16) as usize & 255],
            self.sboxes[14][(u11 >> 8) as usize & 255],
            self.sboxes[15][u10 as usize & 255],
        ]
    }

    /// The next counter, the little-endian increment of the last byte that carries: the executable
    /// counts `FF FF .. FF` to `00 00 .. 00` through, one block per keystream.
    pub fn next_counter(counter: &[u8; 16]) -> [u8; 16] {
        let mut next = *counter;
        for byte in next.iter_mut().rev() {
            *byte = byte.wrapping_add(1);
            if *byte != 0 {
                break;
            }
        }
        next
    }

    /// A payload decrypted, or a plaintext encrypted: the cipher is a counter-mode XOR, so the
    /// one operation serves both directions, and a payload of any length takes as many counter
    /// blocks as it needs from the fixed seed.
    pub fn crypt(&self, payload: &[u8]) -> Vec<u8> {
        let mut counter = self.seed;
        let mut out = Vec::with_capacity(payload.len());
        for chunk in payload.chunks(16) {
            let keystream = self.keystream_block(&counter);
            counter = Self::next_counter(&counter);
            for (byte, key) in chunk.iter().zip(keystream) {
                out.push(*byte ^ key);
            }
        }
        out
    }
}

#[cfg(test)]
#[path = "save_crypto_tests.rs"]
mod tests;
