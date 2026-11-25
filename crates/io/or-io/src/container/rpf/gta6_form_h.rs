//! The 20-byte nested-table row of a headerless resource and the stored body it describes (a
//! 16-byte zero slot, then the chunks, each 16-aligned), from one write's sections and chunks.
//! `w0` and `w4` are the padded section lengths with the version's high and low nibble, `w8` the
//! body in 16-byte units, `w12` its place (the container writer's) and `w16` the type code.

use super::gta6_chunk_table::Chunk;
use super::rpf9_scan::{RPF9_SLOT, STREAM_CAPS};
use super::rpf9_write::{cuts, SECTION_LIMIT};
use crate::error::{IoError, Result};

/// The resource version of each headerless type code, the same on every row of that code.
pub const VERSIONS: [(u32, u8); 36] = [
    (0x02, 0xB1), // drawable
    (0x03, 0x62), // fragment
    (0x04, 0xB1), // drawable dictionary
    (0x05, 0x03), // texture dictionary, second type
    (0x06, 0x2F), // bound
    (0x0B, 0x03), // headerless metadata
    (0x0F, 0x06), // acceleration structure
    (0x10, 0x07), // dictionary-shaped, acceleration archives
    (0x11, 0x06), // acceleration structure
    (0x12, 0x17),
    (0x43, 0x41), // clip dictionary
    (0x45, 0x20),
    (0x46, 0x4C), // animation
    (0x48, 0x6A), // particles
    (0x49, 0x03), // map data
    (0x4A, 0x03), // map types
    (0x4D, 0x06),
    (0x4E, 0x27), // animation
    (0x4F, 0x06), // navigation mesh
    (0x56, 0x06), // dictionary-shaped, animation archive
    (0x57, 0x03), // headerless metadata
    (0x58, 0x6A), // cloth
    (0x59, 0x31), // script program
    (0x5F, 0x1A), // keyed-blob descriptor
    (0x63, 0x1A), // dictionary-shaped
    (0x64, 0x01), // navigation octree
    (0x65, 0x0B), // neighbour table
    (0x66, 0x24), // animation
    (0x67, 0x0B), // morph target
    (0x6C, 0x03), // headerless metadata
    (0x6D, 0x01), // animation
    (0x6E, 0x6A), // asset index
    (0x6F, 0x03), // text table
    (0x71, 0x03), // headerless metadata
    (0x72, 0x1A), // keyed blob
    (0x73, 0x03), // navigation metadata
];

fn bad(why: String) -> IoError {
    IoError::Invalid {
        what: "a headerless resource's table row".into(),
        why,
    }
}

/// The resource version of `type_code`, `None` for a code no measured row carries.
pub fn version(type_code: u32) -> Option<u8> {
    VERSIONS
        .iter()
        .find(|(code, _)| *code == type_code)
        .map(|(_, version)| *version)
}

/// The inflated sizes of the chunks of two sections of these lengths, blocks split at `cap`.
pub(super) fn chunk_sizes(virtual_len: u32, physical_len: u32, cap: u64) -> Vec<u32> {
    let cap = cap as usize;
    [virtual_len, physical_len]
        .into_iter()
        .flat_map(|len| cuts(len as usize))
        .flat_map(|block| {
            (0..block)
                .step_by(cap)
                .map(move |at| (block - at).min(cap) as u32)
        })
        .collect()
}

/// The 20-byte row of a `type_code` resource of these padded sections and chunks, `w12` left 0.
pub fn row(
    type_code: u32,
    virtual_len: u32,
    physical_len: u32,
    chunks: &[Chunk],
) -> Result<[u8; 20]> {
    let version = version(type_code).ok_or_else(|| {
        bad(format!(
            "type code {type_code:#x} is no headerless resource kind with a measured version"
        ))
    })?;
    for (section, len) in [("virtual", virtual_len), ("physical", physical_len)] {
        if !len.is_multiple_of(16) || len as usize > SECTION_LIMIT {
            return Err(bad(format!(
                "the {section} section of {len} bytes is not padded to 16 within the \
                 {SECTION_LIMIT} bytes a pointer reaches"
            )));
        }
    }
    let inflated: Vec<u32> = chunks.iter().map(|c| c.inflated).collect();
    if !STREAM_CAPS
        .iter()
        .any(|&cap| inflated == chunk_sizes(virtual_len, physical_len, cap))
    {
        return Err(bad(format!(
            "its {} chunks are not the cuts of a {virtual_len}-byte virtual section then a \
             {physical_len}-byte physical one, split at a stream cap",
            chunks.len()
        )));
    }
    if let Some(i) = chunks
        .iter()
        .position(|c| c.zlib == (c.stored == c.inflated))
    {
        return Err(bad(format!(
            "chunk {i} is {} though its stored size {} its inflated size, and a chunk is stored \
             exactly when the two are equal",
            if chunks[i].zlib { "deflated" } else { "stored" },
            if chunks[i].stored == chunks[i].inflated {
                "equals"
            } else {
                "differs from"
            },
        )));
    }
    let units = chunks
        .iter()
        .try_fold(1u32, |units, c| units.checked_add(c.stored.div_ceil(16)))
        .ok_or_else(|| bad("its stored chunks overflow the 32-bit unit count".into()))?;
    let mut out = [0u8; 20];
    out[0..4].copy_from_slice(&(virtual_len | u32::from(version >> 4)).to_le_bytes());
    out[4..8].copy_from_slice(&(physical_len | u32::from(version & 0xF)).to_le_bytes());
    out[8..12].copy_from_slice(&units.to_le_bytes());
    out[16..20].copy_from_slice(&type_code.to_le_bytes());
    Ok(out)
}

/// The stored body of these chunks' stored bytes: the zero slot, then each chunk padded to 16.
pub fn stored_body(chunks: &[&[u8]]) -> Vec<u8> {
    let mut body = vec![0u8; RPF9_SLOT as usize];
    for chunk in chunks {
        body.extend_from_slice(chunk);
        body.resize(body.len().next_multiple_of(RPF9_SLOT as usize), 0);
    }
    body
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::super::rpf9_scan::STREAM_CAP;
    use super::super::rpf9_write::{
        write_rpf9_resource, write_rpf9_resource_chunks, Rpf9Form, Rpf9Sections,
    };
    use super::*;

    const MIB32: u32 = 32 << 20;

    fn zlib(inflated: u32, stored: u32) -> Chunk {
        Chunk {
            inflated,
            stored,
            zlib: true,
        }
    }

    fn raw(len: u32) -> Chunk {
        Chunk {
            inflated: len,
            stored: len,
            zlib: false,
        }
    }

    fn words(row: &[u8; 20]) -> [u32; 5] {
        std::array::from_fn(|i| {
            u32::from_le_bytes(row[4 * i..4 * i + 4].try_into().expect("4 bytes"))
        })
    }

    #[test]
    fn a_row_states_each_section_with_a_nibble_of_the_version_and_the_body_in_units() {
        // A drawable (0xB1) of a head and a tail virtual block and one physical block.
        let chunks = [zlib(0x1_0000, 0x3001), zlib(0x300, 0x95), raw(0x1240)];
        let drawable = row(0x02, 0x1_0300, 0x1240, &chunks).expect("a row");
        // The slot, then 0x3010, 0xA0 and 0x1240 bytes of chunks.
        let units = 1 + (0x3010 + 0xA0 + 0x1240) / 16;
        assert_eq!(
            words(&drawable),
            [0x1_0300 | 0xB, 0x1240 | 0x1, units, 0, 0x02]
        );
        // A cloth (0x6A) has no physical section: `w4` is the nibble alone.
        let cloth = row(0x58, 0x200, 0, &[zlib(0x200, 0x41)]).expect("a row");
        assert_eq!(words(&cloth), [0x206, 0xA, 1 + 0x50 / 16, 0, 0x58]);
    }

    #[test]
    fn the_chunks_are_the_cuts_of_the_virtual_then_the_physical_section_split_at_a_stream_cap() {
        let p = MIB32 + 0x3_0000 + 0x50;
        let split = [
            zlib(0x200, 0x40),
            zlib(MIB32, 0x100),
            zlib(0x3_0000, 0x80),
            zlib(0x50, 0x20),
        ];
        assert!(row(0x05, 0x200, p, &split).is_ok());
        // The block past the cap left whole, the sections swapped, a size off by 16.
        let whole = [
            zlib(0x200, 0x40),
            zlib(MIB32 + 0x3_0000, 0x180),
            zlib(0x50, 0x20),
        ];
        assert!(row(0x05, 0x200, p, &whole).is_err());
        let swapped = [
            zlib(0x50, 0x20),
            zlib(0x3_0000, 0x80),
            zlib(MIB32, 0x100),
            zlib(0x200, 0x40),
        ];
        assert!(row(0x05, p, 0x200, &swapped).is_err());
        assert!(row(0x05, 0x210, p, &split).is_err());
        // The smaller cap's split is a cap a read takes too.
        let small = (8u32 << 20, 0x1_0000);
        let at_8 = [
            zlib(small.0, 0x100),
            zlib(small.0, 0x100),
            zlib(small.1, 0x40),
        ];
        assert!(row(0x05, 2 * small.0 + small.1, 0, &at_8).is_ok());
        assert_eq!(chunk_sizes(0x2_0000, 0, STREAM_CAP), [0x2_0000]);
        assert_eq!(chunk_sizes(0, 0x1_0010, STREAM_CAP), [0x1_0000, 0x10]);
        assert!(chunk_sizes(0, 0, STREAM_CAP).is_empty());
    }

    #[test]
    fn rows_no_table_holds_are_refused() {
        let one = [zlib(0x200, 0x41)];
        // A type code with no measured version, an unpadded section, one no pointer reaches.
        assert!(row(0x01, 0x200, 0, &one).is_err());
        assert!(row(0x7F, 0x200, 0, &one).is_err());
        assert!(row(0x02, 0x1F8, 0, &[zlib(0x1F8, 0x41)]).is_err());
        let past = (SECTION_LIMIT + 16) as u32;
        assert!(row(0x02, past, 0, &[zlib(past, 0x41)]).is_err());
        // A deflated chunk the size it inflates to, and a stored one of another size.
        assert!(row(0x02, 0x200, 0, &[zlib(0x200, 0x200)]).is_err());
        assert!(row(
            0x02,
            0x200,
            0,
            &[Chunk {
                inflated: 0x200,
                stored: 0x100,
                zlib: false
            }]
        )
        .is_err());
        assert!(row(0x02, 0x200, 0, &[raw(0x200)]).is_ok());
    }

    #[test]
    fn every_type_code_has_one_version() {
        for (i, (code, _)) in VERSIONS.iter().enumerate() {
            assert!(
                VERSIONS[i + 1..].iter().all(|(other, _)| other != code),
                "{code:#x}"
            );
        }
        assert_eq!(version(0x43), Some(0x41));
        assert_eq!(version(0x45), Some(0x20));
        assert_eq!(version(0x00), None);
    }

    #[test]
    fn a_stored_body_is_the_zero_slot_then_each_chunk_16_aligned() {
        let body = stored_body(&[&[1; 5], &[2; 16], &[3; 17]]);
        assert_eq!(body.len(), 16 + 16 + 16 + 32);
        assert!(body[..16].iter().all(|b| *b == 0));
        assert_eq!(body[16..21], [1; 5]);
        assert!(body[21..32].iter().all(|b| *b == 0));
        assert_eq!(body[32..48], [2; 16]);
        assert_eq!(body[48..65], [3; 17]);
        assert!(body[65..].iter().all(|b| *b == 0));
        assert_eq!(stored_body(&[]), [0; 16]);
    }

    #[test]
    fn one_write_authors_the_row_and_the_body_that_inflates_back_to_its_sections() {
        // A virtual section of a head and a tail, its block map at 0x80, and a physical section.
        let mut v: Vec<u8> = (0..0x1_02F5).map(|i| (i % 251) as u8 | 0x80).collect();
        v[..8].fill(0);
        v[8..16].copy_from_slice(&0x5000_0080u64.to_le_bytes());
        v[0x80..0xB0].fill(0);
        let p: Vec<u8> = (0..0x1234).map(|i| (i * 7 % 253) as u8 | 1).collect();
        let written = write_rpf9_resource_chunks(&v, &p, STREAM_CAP).expect("written");
        let sections = Rpf9Sections {
            form: Rpf9Form::BlockMapped,
            virtual_section: v,
            physical: p.clone(),
        };
        assert_eq!(
            written.bytes,
            write_rpf9_resource(&sections).expect("written")
        );
        assert_eq!(
            [written.virtual_len, written.physical_len],
            [0x1_0300, 0x1240]
        );
        let sizes: Vec<u32> = written.chunks.iter().map(|c| c.inflated).collect();
        assert_eq!(sizes, [0x1_0000, 0x300, 0x1240]);

        let row = row(
            0x02,
            written.virtual_len,
            written.physical_len,
            &written.chunks,
        )
        .expect("a row");
        let body = stored_body(&[&written.bytes]);
        assert_eq!(words(&row)[2] as usize * 16, body.len());
        assert!(body[..16].iter().all(|b| *b == 0));
        // Each chunk inflates from exactly its stored bytes, 16-aligned after the slot.
        let (mut at, mut inflated) = (16usize, Vec::new());
        for chunk in &written.chunks {
            let stored = &body[at..at + chunk.stored as usize];
            let mut z = flate2::read::ZlibDecoder::new(stored);
            let mut out = Vec::new();
            z.read_to_end(&mut out).expect("inflates");
            assert_eq!(out.len(), chunk.inflated as usize);
            inflated.extend(out);
            at = (at + chunk.stored as usize).next_multiple_of(16);
        }
        assert_eq!(at, body.len());
        let mut canonical = sections.virtual_section.clone();
        canonical.resize(0x1_0300, 0);
        (canonical[0x88], canonical[0x89]) = (2, 1);
        let mut padded = p;
        padded.resize(0x1240, 0);
        assert_eq!(inflated, [canonical, padded].concat());
    }
}
