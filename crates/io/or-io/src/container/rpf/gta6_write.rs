//! The encrypt direction of RPF9's (GTA VI) white-box TOC cipher: the exact stage-by-stage inverse
//! of [`rpf9_toc`](super::rpf9_toc)'s `block` primitive, and an [`Rpf9Builder`] that writes a decoded
//! table of contents back to ciphertext.
//!
//! ⚠ The round tables, input and output substitutions and the CBC chaining are the algorithm, read
//! from the same snapshot [`rpf9_toc`](super::rpf9_toc) decrypts with; no key byte is committed.
//!
//! Scope and honesty: only the *byte-preserving* round trip is a supported claim — decrypt an on-disk
//! TOC, rebuild, and the ciphertext is byte-identical. A *structurally modified* archive is not
//! supported, and the running game refuses one: a type-6 (and legacy type-2) descriptor is an
//! RSA-shaped signature over the whole plaintext TOC, verified at first mount, so an edited TOC
//! cannot mount without the private key. The per-entry `0xb6f5` content hash's algorithm is also
//! unidentified. [`Rpf9Toc::set_cell`] therefore writes a raw cell for the round-trip test only; the
//! archive writer offers no structural edit. Whether to attempt structural edits at all is an open
//! owner question (O1).

use std::collections::HashMap;

use byteorder::{ByteOrder, LE};

use super::rpf9::Rpf9Header;
use super::rpf9_toc::{GtaViKeys, Rpf9Toc};
use crate::error::{IoError, Result};

/// The four output lanes of one T-box round and the input state bytes each consumes. `GROUPS[j]`
/// lists, for output word `j`, the four round tables feeding it, in the order the forward round XORs
/// them; a table's index is the state byte it reads (`T[k]` reads big-endian state byte `k`).
const GROUPS: [[usize; 4]; 4] = [[0, 13, 10, 7], [1, 14, 4, 11], [5, 2, 8, 15], [9, 6, 12, 3]];

/// The shared inverses derived once from a snapshot's white-box tables: enough to encrypt any block
/// under any key the snapshot holds. Per-archive material (round-key words, output maps) stays in the
/// key and is applied per region.
pub struct Rpf9CipherCore {
    /// Inverse of each shared input byte table.
    input_inv: Box<[[u8; 256]; 16]>,
    /// `T[k][0]`, the affine constant of each round table.
    tbox_const: [u32; 16],
    /// Inverse of each round table's internal byte permutation: `sinv[k][s]` is the state byte whose
    /// coordinate in the table's difference-subspace basis is `s`.
    sinv: Box<[[u8; 256]; 16]>,
    /// Row form of each output lane's inverse 32×32 GF(2) linear layer; `apply_inv` reads it.
    round_inv: [[u32; 32]; 4],
}

fn write_bad(why: impl Into<String>) -> IoError {
    IoError::Invalid {
        what: "an RPF9 TOC encrypt".into(),
        why: why.into(),
    }
}

/// Insert `v` into a greedy XOR basis keyed by highest set bit; `true` when it was independent of the
/// vectors already there, so the caller keeps it as a basis column.
fn xor_basis_insert(basis: &mut [u32; 32], mut v: u32) -> bool {
    while v != 0 {
        let high = 31 - v.leading_zeros() as usize;
        if basis[high] == 0 {
            basis[high] = v;
            return true;
        }
        v ^= basis[high];
    }
    false
}

/// The image of the 8-bit coordinate `s` under the 8-column GF(2) map `cols` (bit `i` of `s` selects
/// column `i`).
fn apply_cols8(cols: &[u32; 8], s: u8) -> u32 {
    let mut out = 0u32;
    for (i, col) in cols.iter().enumerate() {
        if (s >> i) & 1 == 1 {
            out ^= col;
        }
    }
    out
}

/// `M⁻¹ · v` where `rows` is the inverse in row form (`rows[r]` is row `r` of the inverse): bit `r`
/// of the result is the parity of `rows[r] & v`.
fn apply_inv(rows: &[u32; 32], v: u32) -> u32 {
    let mut out = 0u32;
    for (r, row) in rows.iter().enumerate() {
        if (row & v).count_ones() & 1 == 1 {
            out |= 1 << r;
        }
    }
    out
}

/// The inverse of the 32×32 GF(2) matrix whose column `c` is `cols[c]`, returned in row form for
/// [`apply_inv`]; `None` when the matrix is singular.
fn invert32(cols: &[u32; 32]) -> Option<[u32; 32]> {
    // Row form: bit c of mat[r] is bit r of cols[c] (entry M[r][c]). Augment with the identity and
    // run Gauss-Jordan; the identity half becomes the inverse's rows.
    let mut mat = [0u32; 32];
    for (c, col) in cols.iter().enumerate() {
        for (r, m) in mat.iter_mut().enumerate() {
            if (col >> r) & 1 == 1 {
                *m |= 1 << c;
            }
        }
    }
    let mut aug = [0u32; 32];
    for (r, a) in aug.iter_mut().enumerate() {
        *a = 1 << r;
    }
    for col in 0..32 {
        let pivot = (col..32).find(|&r| (mat[r] >> col) & 1 == 1)?;
        mat.swap(col, pivot);
        aug.swap(col, pivot);
        for r in 0..32 {
            if r != col && (mat[r] >> col) & 1 == 1 {
                mat[r] ^= mat[col];
                aug[r] ^= aug[col];
            }
        }
    }
    Some(aug)
}

/// The inverse of a byte table, refused when the table is not a permutation of `0..=255`.
fn invert_byte_table(table: &[u8; 256], what: &str) -> Result<[u8; 256]> {
    let mut inv = [0u8; 256];
    let mut seen = [false; 256];
    for (x, &y) in table.iter().enumerate() {
        if seen[y as usize] {
            return Err(write_bad(format!("{what} is not a byte permutation")));
        }
        seen[y as usize] = true;
        inv[y as usize] = x as u8;
    }
    Ok(inv)
}

impl Rpf9CipherCore {
    /// Derive the encrypt-side inverses from `keys`' shared white-box tables. Refused, never
    /// panicking, when a table is not the clean affine-permutation shape the inverse needs.
    pub fn new(keys: &GtaViKeys) -> Result<Self> {
        let (input_tables, round_tables, _) = keys.shared_tables();

        let mut input_inv = Box::new([[0u8; 256]; 16]);
        for (i, table) in input_tables.iter().enumerate() {
            input_inv[i] = invert_byte_table(table, "an input byte table")?;
        }

        let mut tbox_const = [0u32; 16];
        let mut a_cols = [[0u32; 8]; 16];
        let mut sinv = Box::new([[0u8; 256]; 16]);
        for (k, table) in round_tables.iter().enumerate() {
            let constant = table[0];
            tbox_const[k] = constant;

            // The difference set; a clean round table has 256 distinct differences forming an
            // 8-dimensional GF(2) subspace.
            let mut diff_to_byte: HashMap<u32, u8> = HashMap::with_capacity(256);
            for (x, &word) in table.iter().enumerate() {
                if diff_to_byte.insert(word ^ constant, x as u8).is_some() {
                    return Err(write_bad(
                        "a round table's differences are not distinct (not a clean cipher table)",
                    ));
                }
            }

            // A basis of that subspace: the first eight independent differences, in input order.
            let mut basis = [0u32; 32];
            let mut columns = [0u32; 8];
            let mut found = 0usize;
            for word in table.iter().skip(1) {
                if found == 8 {
                    break;
                }
                let v = word ^ constant;
                if xor_basis_insert(&mut basis, v) {
                    columns[found] = v;
                    found += 1;
                }
            }
            if found != 8 {
                return Err(write_bad(
                    "a round table's difference set is not 8-dimensional",
                ));
            }
            a_cols[k] = columns;

            // sinv[k][s] inverts the table's internal permutation: the byte whose basis coordinate is
            // s is the preimage of A_k·s in the difference set.
            for (s, slot) in sinv[k].iter_mut().enumerate() {
                let image = apply_cols8(&columns, s as u8);
                *slot = *diff_to_byte.get(&image).ok_or_else(|| {
                    write_bad("a round table's basis does not span its differences")
                })?;
            }
        }

        // Per output lane, the 32×32 linear layer is the four feeding tables' 8-column maps side by
        // side; slot 0 occupies the top byte of the packed coordinate vector (bits 24..=31).
        let mut round_inv = [[0u32; 32]; 4];
        for (j, group) in GROUPS.iter().enumerate() {
            let mut lane = [0u32; 32];
            for (slot, &k) in group.iter().enumerate() {
                for i in 0..8 {
                    lane[(3 - slot) * 8 + i] = a_cols[k][i];
                }
            }
            round_inv[j] = invert32(&lane).ok_or_else(|| {
                write_bad("a round's linear layer is singular, so it cannot invert")
            })?;
        }

        Ok(Self {
            input_inv,
            tbox_const,
            sinv,
            round_inv,
        })
    }

    /// Recover the 16-byte plaintext input of one `block`: the exact inverse of
    /// [`rpf9_toc`](super::rpf9_toc)'s `block`, so `block(block_encrypt(x)) == x`.
    fn block_encrypt(
        &self,
        out: &[u8; 16],
        words: &[u32; 60],
        output_inv: &[[u8; 256]; 16],
    ) -> [u8; 16] {
        // Undo the output substitution to recover the shuffle indices.
        let mut idx = [0u8; 16];
        for (i, slot) in idx.iter_mut().enumerate() {
            *slot = output_inv[i][out[i] as usize];
        }
        // Undo the shuffle: reassemble the four big-endian state lanes from the index bytes.
        let word = |b0: usize, b1: usize, b2: usize, b3: usize| {
            u32::from_be_bytes([idx[b0], idx[b1], idx[b2], idx[b3]])
        };
        let mut state = [
            word(0, 5, 10, 15),
            word(4, 9, 14, 3),
            word(8, 13, 2, 7),
            word(12, 1, 6, 11),
        ];
        // Undo the thirteen rounds, last first.
        for r in (1..=13).rev() {
            state = self.inv_round(state, words, r);
        }
        // Undo the initial AddRoundKey, then the input substitution.
        let mut encoded = [0u8; 16];
        for (i, lane) in state.iter().enumerate() {
            encoded[i * 4..i * 4 + 4].copy_from_slice(&(lane ^ words[i]).to_be_bytes());
        }
        let mut data = [0u8; 16];
        for (i, slot) in data.iter_mut().enumerate() {
            *slot = self.input_inv[i][encoded[i] as usize];
        }
        data
    }

    /// Invert one T-box round: from the output state recover the input state bytes each lane consumed.
    fn inv_round(&self, out: [u32; 4], words: &[u32; 60], r: usize) -> [u32; 4] {
        let mut bytes = [0u8; 16];
        for (j, group) in GROUPS.iter().enumerate() {
            let constants = group.iter().fold(0u32, |acc, &k| acc ^ self.tbox_const[k]);
            let value = out[j] ^ words[4 * r + j] ^ constants;
            let coords = apply_inv(&self.round_inv[j], value);
            let packed = coords.to_be_bytes();
            for (slot, &k) in group.iter().enumerate() {
                bytes[k] = self.sinv[k][packed[slot] as usize];
            }
        }
        let mut state = [0u32; 4];
        for (i, lane) in state.iter_mut().enumerate() {
            *lane = u32::from_be_bytes([
                bytes[i * 4],
                bytes[i * 4 + 1],
                bytes[i * 4 + 2],
                bytes[i * 4 + 3],
            ]);
        }
        state
    }

    /// Encrypt `data` in place under `registry_id`, CBC-chained from `iv`: `data[i]` is XORed with the
    /// previous ciphertext block (the IV first), run through [`Self::block_encrypt`], and that
    /// ciphertext becomes the next chaining value — the exact inverse of the reader's CBC decrypt.
    pub fn cbc_encrypt(
        &self,
        keys: &GtaViKeys,
        registry_id: u64,
        iv: &[u8; 16],
        data: &mut [u8],
    ) -> Result<()> {
        if !data.len().is_multiple_of(16) {
            return Err(write_bad(format!(
                "a cipher region of {} bytes is not a whole number of blocks",
                data.len()
            )));
        }
        let (words, output) = keys.block_key(registry_id)?;
        let mut output_inv = [[0u8; 256]; 16];
        for (i, table) in output.iter().enumerate() {
            output_inv[i] = invert_byte_table(table, "an output byte table")?;
        }
        let mut prev = *iv;
        for chunk in data.chunks_mut(16) {
            let mut block: [u8; 16] = chunk.try_into().expect("chunks_mut(16) yields 16 bytes");
            for (b, p) in block.iter_mut().zip(prev.iter()) {
                *b ^= p;
            }
            let cipher = self.block_encrypt(&block, words, &output_inv);
            chunk.copy_from_slice(&cipher);
            prev = cipher;
        }
        Ok(())
    }
}

/// The three cipher regions and index key of a decrypted TOC, read from its plaintext metadata so a
/// rebuild reproduces exactly what the reader consumed.
struct TocRegions {
    bootstrap_end: usize,
    split: usize,
    toc_end: usize,
    registry_id: u64,
    index_iv: [u8; 16],
}

/// The index cipher type the reader opens; others are not a supported write target.
const INDEX_CIPHER_TYPE: u16 = 16;

/// Read the cipher regions and index key from a plaintext TOC, mirroring the reader's boundary math
/// and its single type-1 index descriptor.
fn toc_regions(bytes: &[u8]) -> Result<TocRegions> {
    let header = Rpf9Header::read(bytes)?;
    let n = u64::from(u16::from_le_bytes([header.raw[0x1A], header.raw[0x1B]]));
    let m = u64::from(header.part_c_offset);
    let e = u64::from(header.part_b_offset);
    let split = u64::from(header.index_offset);
    let toc_end = u64::from(header.toc_end);
    let metadata_start = 0x40 + (8 * n).next_multiple_of(16);
    let bootstrap_end = metadata_start
        .checked_add(m)
        .and_then(|v| v.checked_sub(e))
        .ok_or_else(|| write_bad("the metadata size and its exclusion do not form a range"))?;
    if !(0x40 <= metadata_start && metadata_start <= bootstrap_end && bootstrap_end <= split) {
        return Err(write_bad("the metadata region is out of order"));
    }
    if split > toc_end || (bytes.len() as u64) < toc_end {
        return Err(write_bad("the table of contents runs past the plaintext"));
    }

    // The one type-1 descriptor in the plaintext metadata names the index key id, cipher type and IV.
    let mut descriptor: Option<u64> = None;
    for i in 0..n {
        let at = (0x40 + 8 * i) as usize;
        let kind = LE::read_u16(&bytes[at..]);
        let offset = u64::from(LE::read_u32(&bytes[at + 4..]));
        if kind == 1 {
            if descriptor.is_some() {
                return Err(write_bad("more than one index cipher descriptor"));
            }
            if offset >= m {
                return Err(write_bad("the index descriptor points past the metadata"));
            }
            descriptor = Some(metadata_start + offset);
        }
    }
    let at = descriptor.ok_or_else(|| write_bad("no index cipher descriptor"))? as usize;
    if bytes.len() < at + 22 {
        return Err(write_bad("the index descriptor runs past the metadata"));
    }
    let key_id = LE::read_u32(&bytes[at..]);
    let cipher_type = LE::read_u16(&bytes[at + 4..]);
    if cipher_type != INDEX_CIPHER_TYPE {
        return Err(IoError::RpfEncryptionUnsupported { key_id });
    }
    let mut index_iv = [0u8; 16];
    index_iv.copy_from_slice(&bytes[at + 6..at + 22]);
    let registry_id = (u64::from(key_id) << 32) | 0x0200_0000;
    Ok(TocRegions {
        bootstrap_end: bootstrap_end as usize,
        split: split as usize,
        toc_end: toc_end as usize,
        registry_id,
        index_iv,
    })
}

/// Writes a decoded (possibly edited) [`Rpf9Toc`] back to the ciphertext the game reads.
pub struct Rpf9Builder;

impl Rpf9Builder {
    /// Serialize `toc`'s plaintext `[0, toc_end)` back to ciphertext: the header stays plaintext and
    /// the three cipher regions are re-encrypted in the reader's split — the metadata under the
    /// bootstrap key and IV, then the two index regions under the archive key, each from the index IV.
    ///
    /// For an unedited `toc` the result is byte-identical to the on-disk TOC — the only supported
    /// claim. Re-encrypting an edited `toc` is well defined, but the running game refuses the result
    /// (the type-6 signature; see the module header), so it serves the round-trip test alone.
    pub fn serialize(toc: &Rpf9Toc, keys: &GtaViKeys) -> Result<Vec<u8>> {
        let core = Rpf9CipherCore::new(keys)?;
        // Read the regions and index key from the plaintext before the metadata is enciphered over.
        let regions = toc_regions(&toc.bytes)?;
        let (_, _, bootstrap_iv) = keys.shared_tables();
        let bootstrap_iv = *bootstrap_iv;
        let bootstrap = keys.bootstrap_registry_id();

        let mut out = toc.bytes.clone();
        core.cbc_encrypt(
            keys,
            bootstrap,
            &bootstrap_iv,
            &mut out[0x40..regions.bootstrap_end],
        )?;
        core.cbc_encrypt(
            keys,
            regions.registry_id,
            &regions.index_iv,
            &mut out[regions.bootstrap_end..regions.split],
        )?;
        core.cbc_encrypt(
            keys,
            regions.registry_id,
            &regions.index_iv,
            &mut out[regions.split..regions.toc_end],
        )?;
        Ok(out)
    }
}

impl Rpf9Toc {
    /// Overwrite the cell at physical row `row` of the column named `group_hash` in the plaintext TOC,
    /// so a following [`Rpf9Builder::serialize`] writes it back as ciphertext. `cell` must be the
    /// column's exact width, and `row` is a record index within the column's own deduplicated rows —
    /// it is not resolved through the entry-major index, so this writes that one raw cell and no more.
    ///
    /// It exists for the byte-preserving round-trip test (edit a cell, re-encrypt, re-decrypt, compare
    /// the same cell). The archive writer offers no structural edit: the game refuses an edited TOC
    /// (the type-6 signature), and the `0xb6f5` content hash it would need is unidentified — see the
    /// module header.
    pub fn set_cell(&mut self, group_hash: u64, row: u32, cell: &[u8]) -> Result<()> {
        let group = self
            .groups
            .iter()
            .find(|g| g.name_hash == group_hash)
            .ok_or_else(|| write_bad(format!("no column with name hash {group_hash:#018X}")))?;
        if row >= group.count {
            return Err(write_bad(format!(
                "row {row} is past the {}-row column",
                group.count
            )));
        }
        if cell.len() != usize::from(group.width) {
            return Err(write_bad(format!(
                "a {}-byte cell for a {}-byte column",
                cell.len(),
                group.width
            )));
        }
        let at = group.data.start + row as usize * usize::from(group.width);
        self.bytes[at..at + cell.len()].copy_from_slice(cell);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::rpf::Rpf9Group;

    // Snapshot container lengths, mirrored from rpf9_toc so a valid synthetic key can be built here.
    const WORDS: usize = 60;
    const MAPS: usize = 16;
    const MAP_LEN: usize = 256;
    const BLOB_LEN: usize = 4 + WORDS * 4 + MAPS * MAP_LEN;
    const BOOTSTRAP_REGISTRY_ID: u64 = 0x1000_0001_0200_0000;

    /// Append a distinct byte permutation (a rotation by `k`), used for the input and output maps.
    fn push_perm(out: &mut Vec<u8>, k: usize) {
        for x in 0..256usize {
            out.push(((x + k * 7 + 1) % 256) as u8);
        }
    }

    /// A dense invertible 8×8 GF(2) map: output bit `i` is the XOR of input bits `i..=7` (its inverse
    /// is `b_i = y_i ^ y_{i+1}`). Not a bit permutation, so the per-lane matrix inverse has real work.
    fn mix8(b: u8) -> u8 {
        let mut y = 0u8;
        let mut acc = 0u8;
        for i in (0..8).rev() {
            acc ^= (b >> i) & 1;
            y |= acc << i;
        }
        y
    }

    /// A clean synthetic white-box snapshot: the input and output maps are byte permutations, and each
    /// round table is `const ^ (mix8(S_k(byte)) << slot*8)` for a nontrivial inner byte permutation
    /// `S_k` (a bit rotation and a per-table XOR) and the dense invertible mix [`mix8`]. Its difference
    /// set is still an exact 8-dim subspace in the lane's byte — the shape [`Rpf9CipherCore::new`]
    /// inverts — but the derived `sinv` and the per-lane 32×32 inverse are now nontrivial, so the round
    /// trip exercises them rather than an identity. The bootstrap and one archive registry are present.
    fn clean_snapshot(registry_id: u64) -> Vec<u8> {
        // Which slot (0..4) within its lane each of the 16 round tables occupies.
        let mut slot_of = [0usize; 16];
        for group in GROUPS.iter() {
            for (slot, &k) in group.iter().enumerate() {
                slot_of[k] = slot;
            }
        }

        let mut out = Vec::new();
        out.extend_from_slice(b"ORGTAVIK");
        out.extend_from_slice(&1u32.to_le_bytes()); // version
        out.extend_from_slice(&0x4342u32.to_le_bytes()); // CBC mode tag
        out.extend_from_slice(&[0u8; 32]); // exe sha256
        out.extend_from_slice(&[0x11u8; 16]); // bootstrap iv

        out.extend_from_slice(&((MAPS * MAP_LEN) as u32).to_le_bytes());
        for k in 0..MAPS {
            push_perm(&mut out, k);
        }

        out.extend_from_slice(&((MAPS * MAP_LEN * 4) as u32).to_le_bytes());
        for (k, &slot) in slot_of.iter().enumerate() {
            let shift = slot * 8;
            let constant = 0x0A0B_0C0Du32.wrapping_mul(k as u32 + 1);
            for x in 0..256u32 {
                // A nontrivial inner permutation then a dense GF(2) mix, kept inside the lane byte so
                // the four subspaces of a lane are a direct sum and the lane matrix stays invertible.
                let s = (x as u8).rotate_left(3) ^ (k as u8).wrapping_mul(0x5B);
                let mixed = u32::from(mix8(s));
                out.extend_from_slice(&(constant ^ (mixed << shift)).to_le_bytes());
            }
        }

        out.extend_from_slice(&2u32.to_le_bytes()); // two keys
        for &id in &[BOOTSTRAP_REGISTRY_ID, registry_id] {
            out.extend_from_slice(&id.to_le_bytes());
            out.extend_from_slice(&(BLOB_LEN as u32).to_le_bytes());
            out.extend_from_slice(&[0x8e, 0x9b, 0x3b, 0x4e]); // blob magic
            for w in 0..WORDS as u32 {
                out.extend_from_slice(&(w.wrapping_mul(2_654_435_761) ^ id as u32).to_le_bytes());
            }
            for k in 0..MAPS {
                push_perm(&mut out, k + 3);
            }
        }
        out
    }

    #[test]
    fn encrypt_is_the_exact_inverse_of_the_production_decrypt() {
        let registry_id = 0xDEAD_BEEF_0200_0000u64;
        let snapshot = clean_snapshot(registry_id);
        let keys = GtaViKeys::parse(&snapshot).expect("a clean synthetic snapshot parses");
        let core = Rpf9CipherCore::new(&keys).expect("the clean tables invert");

        let iv = [0x5Au8; 16];
        let mut data: Vec<u8> = (0..16 * 7u32)
            .map(|i| (i.wrapping_mul(2_246_822_519) >> 13) as u8)
            .collect();
        let original = data.clone();

        core.cbc_encrypt(&keys, registry_id, &iv, &mut data)
            .expect("encrypt");
        assert_ne!(data, original, "ciphertext should differ from plaintext");

        // The production reader's decrypt must recover the plaintext exactly.
        keys.decrypt_region(registry_id, &iv, &mut data)
            .expect("decrypt");
        assert_eq!(data, original, "decrypt(encrypt(x)) must be x");
    }

    #[test]
    fn a_region_that_is_not_whole_blocks_is_refused() {
        let registry_id = 0x1111_2222_0200_0000u64;
        let keys = GtaViKeys::parse(&clean_snapshot(registry_id)).unwrap();
        let core = Rpf9CipherCore::new(&keys).unwrap();
        let mut data = [0u8; 17];
        assert!(matches!(
            core.cbc_encrypt(&keys, registry_id, &[0; 16], &mut data),
            Err(IoError::Invalid { .. })
        ));
    }

    #[test]
    fn set_cell_overwrites_one_cell_and_guards_its_bounds() {
        // A tiny TOC: a header-sized prefix, then two 4-byte cells for one group of two rows.
        let group = Rpf9Group {
            name_hash: 0xABCD,
            count: 2,
            width: 4,
            external: false,
            data: 0x40..0x48,
        };
        let mut toc = Rpf9Toc {
            bytes: vec![0u8; 0x48],
            key_id: 0,
            registry_id: 0,
            entry_count: 2,
            groups: vec![group],
        };
        toc.set_cell(0xABCD, 1, &[1, 2, 3, 4]).expect("write row 1");
        assert_eq!(&toc.bytes[0x44..0x48], &[1, 2, 3, 4]);
        assert_eq!(&toc.bytes[0x40..0x44], &[0, 0, 0, 0], "row 0 untouched");

        assert!(
            toc.set_cell(0xABCD, 2, &[0; 4]).is_err(),
            "row past the column"
        );
        assert!(toc.set_cell(0xABCD, 0, &[0; 3]).is_err(), "wrong width");
        assert!(toc.set_cell(0x9999, 0, &[0; 4]).is_err(), "unknown column");
    }
}
