//! A GTA VI resource as a loose file, which a read at offset 0 returns as the sections it was
//! written from: a block-mapped resource or a stored RSC8.
//!
//! Every block is deflated, as the measured layout allows: the game stores a section's last block
//! raw where its encoder does not shrink it, and no reader needs that choice.

use std::io::Write;

use byteorder::{ByteOrder, LE};
use flate2::write::ZlibEncoder;
use flate2::Compression;

use super::rpf9_scan::{
    Rpf9BodyKind, Rpf9ScanFind, Rpf9ZlibRun, RPF9_SLOT, STREAM_CAP, STREAM_CAPS,
};
use super::version::RSC8_MAGIC;

/// A section is cut into a head of whole pieces and a tail of the rest.
const PIECE: usize = 0x1_0000;
/// The bytes of a section a pointer reaches: its low 28 bits.
pub(super) const SECTION_LIMIT: usize = 1 << 28;
/// A stored RSC8's version, compressor and key bytes, the same in every measured head.
const RSC8_STORED: [u8; 3] = [0x41, 0x1F, 0];

/// How a resource is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rpf9Form {
    /// Blocks of zlib streams, counted by the block map the virtual section points at from 0x08.
    BlockMapped,
    /// A 16-byte RSC8 head, then the virtual section stored; `config` is the head's byte 7,
    /// carried as read, since what it selects is not known.
    StoredRsc8 { config: u8 },
}

/// A resource's virtual and physical sections, and the form they are written in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rpf9Sections {
    pub form: Rpf9Form,
    pub virtual_section: Vec<u8>,
    pub physical: Vec<u8>,
}

impl Rpf9Sections {
    /// The sections of the body a read returned as `bytes`; `None` for a body that is no resource.
    pub fn of(find: &Rpf9ScanFind, bytes: &[u8]) -> Option<Self> {
        match find.kind {
            Rpf9BodyKind::Zlib(Rpf9ZlibRun {
                resource: Some(resource),
                ..
            }) => {
                let (virtual_section, physical) = resource.sections(bytes);
                Some(Self {
                    form: Rpf9Form::BlockMapped,
                    virtual_section: virtual_section.to_vec(),
                    physical: physical.to_vec(),
                })
            }
            Rpf9BodyKind::StoredRsc8 { .. } => Some(Self {
                form: Rpf9Form::StoredRsc8 {
                    config: *bytes.get(7)?,
                },
                virtual_section: bytes.get(16..)?.to_vec(),
                physical: Vec::new(),
            }),
            _ => None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Rpf9WriteError {
    #[error("the virtual section's first 16 bytes state no block map: eight zero bytes, then a 0x5000_0000 pointer")]
    NoBlockMap,
    #[error("the block map at {at:#X} is not 16-aligned, or its {len} bytes do not lie in the {virtual_len}-byte virtual section")]
    MapOutside { at: u64, len: u64, virtual_len: u64 },
    #[error("the block map at {at:#X} holds a non-zero byte at {byte:#X} beside its two counts, so the section is laid out for a map of fewer blocks")]
    MapNotZero { at: u64, byte: u64 },
    #[error(
        "the {section} section of {len} bytes passes the {SECTION_LIMIT} bytes a pointer reaches"
    )]
    TooLong { section: &'static str, len: u64 },
    #[error("the virtual section of {len} bytes is one block of more than one stream with physical blocks after it, which a read cannot tell from the physical head")]
    Ambiguous { len: u64 },
    #[error("a stored RSC8 has no physical section, and this one has {len} bytes")]
    PhysicalInRsc8 { len: u64 },
    #[error("an RSC8 config byte is 0 or 1 in every measured head, not {0}")]
    Config(u8),
    #[error(
        "a block's streams stop at 8 MiB or 32 MiB in every measured archive, not at {0} bytes"
    )]
    StreamCap(u64),
    #[error("deflating a block: {0}")]
    Deflate(#[from] std::io::Error),
}

/// `sections` as a loose file: each section padded with zeros to 16 bytes, and for a block-mapped
/// resource the block map's counts and slots written from the sections' lengths.
pub fn write_rpf9_resource(sections: &Rpf9Sections) -> Result<Vec<u8>, Rpf9WriteError> {
    write_rpf9_resource_capped(sections, STREAM_CAP)
}

/// As [`write_rpf9_resource`], a block's streams cut at `cap`, one of the two caps a read takes.
// Which cap an archive's resources use is per nested archive and not known from the sections.
pub fn write_rpf9_resource_capped(
    sections: &Rpf9Sections,
    cap: u64,
) -> Result<Vec<u8>, Rpf9WriteError> {
    if !STREAM_CAPS.contains(&cap) {
        return Err(Rpf9WriteError::StreamCap(cap));
    }
    let mut v = sections.virtual_section.clone();
    let mut p = sections.physical.clone();
    for (section, bytes) in [("virtual", &mut v), ("physical", &mut p)] {
        if bytes.len() > SECTION_LIMIT {
            return Err(Rpf9WriteError::TooLong {
                section,
                len: bytes.len() as u64,
            });
        }
        bytes.resize(bytes.len().next_multiple_of(RPF9_SLOT as usize), 0);
    }
    match sections.form {
        Rpf9Form::StoredRsc8 { config } => stored_rsc8(v, &p, config),
        Rpf9Form::BlockMapped => block_mapped(v, &p, cap as usize),
    }
}

/// A block-mapped loose file, with the chunks and padded section lengths a row is authored from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rpf9WrittenChunks {
    /// What [`write_rpf9_resource_capped`] writes: the chunks, each 16-aligned.
    pub bytes: Vec<u8>,
    /// Each chunk's inflated and stored sizes, in the order written; every one deflated.
    pub chunks: Vec<super::gta6_chunk_table::Chunk>,
    pub virtual_len: u32,
    pub physical_len: u32,
}

/// [`write_rpf9_resource_capped`] of a block-mapped resource, with its chunks and section lengths.
pub fn write_rpf9_resource_chunks(
    virtual_section: &[u8],
    physical: &[u8],
    cap: u64,
) -> Result<Rpf9WrittenChunks, Rpf9WriteError> {
    let sections = Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section: virtual_section.to_vec(),
        physical: physical.to_vec(),
    };
    let bytes = write_rpf9_resource_capped(&sections, cap)?;
    let lost = |why: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, why.to_owned());
    let (virtual_len, physical_len) = (
        virtual_section.len().next_multiple_of(RPF9_SLOT as usize) as u32,
        physical.len().next_multiple_of(RPF9_SLOT as usize) as u32,
    );
    let mut chunks = Vec::new();
    let mut at = 0usize;
    for inflated in super::gta6_form_h::chunk_sizes(virtual_len, physical_len, cap) {
        // A stream's stored length is where inflating it ends.
        let rest = bytes
            .get(at..)
            .ok_or_else(|| lost("a chunk starts past the written bytes"))?;
        let mut z = flate2::bufread::ZlibDecoder::new(rest);
        if std::io::copy(&mut z, &mut std::io::sink())? != u64::from(inflated) {
            return Err(lost("a written stream does not inflate to its block's piece").into());
        }
        let stored = z.total_in() as u32;
        chunks.push(super::gta6_chunk_table::Chunk {
            inflated,
            stored,
            zlib: true,
        });
        at = (at + stored as usize).next_multiple_of(RPF9_SLOT as usize);
    }
    Ok(Rpf9WrittenChunks {
        bytes,
        chunks,
        virtual_len,
        physical_len,
    })
}

fn stored_rsc8(v: Vec<u8>, p: &[u8], config: u8) -> Result<Vec<u8>, Rpf9WriteError> {
    if !p.is_empty() {
        return Err(Rpf9WriteError::PhysicalInRsc8 {
            len: p.len() as u64,
        });
    }
    if config > 1 {
        return Err(Rpf9WriteError::Config(config));
    }
    let mut out = vec![0u8; 16];
    LE::write_u32(&mut out, RSC8_MAGIC);
    out[4..7].copy_from_slice(&RSC8_STORED);
    out[7] = config;
    // The virtual flags are the size with nibble 4, the physical ones a size of 0 with nibble 1.
    LE::write_u32(&mut out[8..], v.len() as u32 | 4);
    LE::write_u32(&mut out[12..], 1);
    out.extend(v);
    Ok(out)
}

/// The head and tail a section of `len` bytes is cut into: whole pieces, then the rest.
pub(super) fn cuts(len: usize) -> Vec<usize> {
    let head = len - len % PIECE;
    [head, len % PIECE].into_iter().filter(|n| *n > 0).collect()
}

fn block_mapped(mut v: Vec<u8>, p: &[u8], cap: usize) -> Result<Vec<u8>, Rpf9WriteError> {
    let word = |at: usize| v.get(at..at + 8).map(LE::read_u64);
    let (Some(0), Some(pointer)) = (word(0), word(8)) else {
        return Err(Rpf9WriteError::NoBlockMap);
    };
    if pointer >> 28 != 5 {
        return Err(Rpf9WriteError::NoBlockMap);
    }
    let at = (pointer & 0x0FFF_FFFF) as usize;
    let (virtual_blocks, physical_blocks) = (cuts(v.len()), cuts(p.len()));
    // A section of whole pieces whose map counted an empty tail keeps that count, as stored; a
    // virtual one only where a read stops after its head's one stream, as under 8 MiB.
    let kept = |count: usize, len: usize, byte: usize| match v.get(at + byte).copied() {
        Some(2) if count == 1 && len.is_multiple_of(PIECE) => 2,
        _ => count,
    };
    let virtual_count = if p.is_empty() || v.len() < (8 << 20) {
        kept(virtual_blocks.len(), v.len(), 8)
    } else {
        virtual_blocks.len()
    };
    let physical_count = kept(physical_blocks.len(), p.len(), 9);
    let len = 16 + 8 * (virtual_count + physical_count);
    if !at.is_multiple_of(16) || at + len > v.len() {
        return Err(Rpf9WriteError::MapOutside {
            at: at as u64,
            len: len as u64,
            virtual_len: v.len() as u64,
        });
    }
    if let Some(byte) = (at..at + len).find(|i| ![at + 8, at + 9].contains(i) && v[*i] != 0) {
        return Err(Rpf9WriteError::MapNotZero {
            at: at as u64,
            byte: byte as u64,
        });
    }
    // ⚠ A read never goes on in a one-block virtual section's next stream when physical blocks
    // follow, since that stream may be the physical head.
    if virtual_blocks.len() == 1 && !p.is_empty() && v.len() > cap {
        return Err(Rpf9WriteError::Ambiguous {
            len: v.len() as u64,
        });
    }
    v[at + 8] = virtual_count as u8;
    v[at + 9] = physical_count as u8;
    let mut out = Vec::new();
    for (section, blocks) in [(&v[..], virtual_blocks), (p, physical_blocks)] {
        let mut from = 0;
        for block in blocks {
            // Every stream of a block but its last holds the cap, so a read goes on past it.
            for stream in section[from..from + block].chunks(cap) {
                let mut z = ZlibEncoder::new(Vec::new(), Compression::best());
                z.write_all(stream)?;
                out.extend(z.finish()?);
                out.resize(out.len().next_multiple_of(RPF9_SLOT as usize), 0);
            }
            from += block;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::super::rpf9_scan::{extract_rpf9_body, read_rpf9_body, Rpf9Resource, Rpf9Walk};
    use super::*;

    /// A virtual section of `len` bytes with its block map at `map`, the rest a pattern with no
    /// zero byte and no pointer word.
    fn virtual_section(len: usize, map: usize) -> Vec<u8> {
        let mut v: Vec<u8> = (0..len).map(|i| (i % 251) as u8 | 0x80).collect();
        v[..8].fill(0);
        v[8..16].copy_from_slice(&(0x5000_0000 + map as u64).to_le_bytes());
        v[map..map + 48].fill(0);
        v
    }

    fn physical(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 7 % 253) as u8 | 1).collect()
    }

    fn mapped(v: Vec<u8>, p: Vec<u8>) -> Rpf9Sections {
        Rpf9Sections {
            form: Rpf9Form::BlockMapped,
            virtual_section: v,
            physical: p,
        }
    }

    /// What a read of `file` at offset 0 returns, and the resource it walked.
    fn read(file: &[u8]) -> (Rpf9Sections, Option<Rpf9Resource>) {
        let (find, bytes) = read_rpf9_body(Cursor::new(file), 0, u64::MAX).expect("it reads");
        let resource = match find.kind {
            Rpf9BodyKind::Zlib(run) => run.resource,
            _ => None,
        };
        (
            Rpf9Sections::of(&find, &bytes).expect("a resource"),
            resource,
        )
    }

    /// `bytes` deflated by another encoder than the writer's, under the head a scan starts a body on.
    fn deflate_fast(bytes: &[u8]) -> Vec<u8> {
        let mut z = ZlibEncoder::new(Vec::new(), Compression::fast());
        z.write_all(bytes).expect("deflated");
        let mut out = z.finish().expect("finished");
        // The head's second byte states a level and a check of the two bytes, never the data.
        out[1] = 0xDA;
        out
    }

    #[test]
    fn a_block_mapped_resource_reads_back_to_its_sections_and_writes_again_byte_identical() {
        // Sections of one block, of a head and a tail, of a whole head alone, and a map that
        // lies in the virtual tail.
        let shapes = [
            (0x1F0, 0x40, 0),
            (0x1_0000 + 0x2F5, 0x80, 2 * 0x1_0000 + 0x1234),
            (3 * 0x1_0000, 0x1_0000 + 0x100, 0x2C3),
            (0x1_0000 + 0x400, 0x1_0000 + 0x200, 0x1_0000),
        ];
        for (len, map, physical_len) in shapes {
            let sections = mapped(virtual_section(len, map), physical(physical_len));
            let written = write_rpf9_resource(&sections).expect("written");
            let (again, resource) = read(&written);
            let resource = resource.expect("a block-mapped resource");
            assert_eq!(resource.walk, Rpf9Walk::Whole, "{len:#X}");
            let (vc, pc) = (
                cuts(len.next_multiple_of(16)),
                cuts(physical_len.next_multiple_of(16)),
            );
            assert_eq!(
                [resource.virtual_blocks, resource.physical_blocks],
                [vc.len() as u8, pc.len() as u8],
                "{len:#X}"
            );
            let blocks: Vec<usize> = (resource.blocks.iter().flatten())
                .map(|b| b.inflated_len as usize)
                .collect();
            assert_eq!(blocks, [&vc[..], &pc[..]].concat(), "{len:#X}");
            let mut canonical = sections.virtual_section.clone();
            canonical.resize(len.next_multiple_of(16), 0);
            (canonical[map + 8], canonical[map + 9]) = (vc.len() as u8, pc.len() as u8);
            let mut padded = sections.physical.clone();
            padded.resize(physical_len.next_multiple_of(16), 0);
            assert_eq!(again, mapped(canonical, padded), "{len:#X}");
            assert_eq!(
                write_rpf9_resource(&again).expect("again"),
                written,
                "{len:#X}"
            );
        }
    }

    #[test]
    fn a_block_past_the_stream_cap_is_written_as_streams_a_read_takes_as_one_block() {
        let cap = STREAM_CAP as usize;
        let sections = mapped(
            virtual_section(0x200, 0x40),
            physical(cap + 3 * 0x1_0000 + 0x50),
        );
        let written = write_rpf9_resource(&sections).expect("written");
        let (again, resource) = read(&written);
        let blocks: Vec<u64> = (resource.expect("a resource").blocks.iter().flatten())
            .map(|b| b.inflated_len)
            .collect();
        assert_eq!(blocks, [0x200, cap as u64 + 0x3_0000, 0x50]);
        assert_eq!(again.physical, sections.physical);
        assert_eq!(write_rpf9_resource(&again).expect("again"), written);
    }

    #[test]
    fn a_block_is_cut_into_streams_at_the_cap_it_is_written_with() {
        use super::super::rpf9_scan::rpf9_block_streams;
        let small: u64 = 8 << 20;
        let head = small as usize + 3 * 0x1_0000;
        let sections = mapped(virtual_section(0x200, 0x40), physical(head + 0x50));
        let streams = |file: &[u8]| -> Vec<Vec<u64>> {
            let (_, resource) = read(file);
            (resource.expect("a resource").blocks.iter().flatten())
                .map(|b| rpf9_block_streams(Cursor::new(file), b).expect("walked"))
                .collect()
        };
        let cut = write_rpf9_resource_capped(&sections, small).expect("written");
        let (again, _) = read(&cut);
        assert_eq!(again.physical, sections.physical);
        assert_eq!(
            streams(&cut),
            [vec![0x200], vec![small, 0x3_0000], vec![0x50]]
        );
        assert_eq!(
            write_rpf9_resource_capped(&again, small).expect("again"),
            cut
        );
        // The default cap writes the same block as one stream.
        let whole = write_rpf9_resource(&sections).expect("written");
        assert_eq!(streams(&whole)[1], [head as u64]);
        let lone = virtual_section(small as usize + 0x1_0000, 0x40);
        assert!(matches!(
            write_rpf9_resource_capped(&mapped(lone, physical(0x10)), small),
            Err(Rpf9WriteError::Ambiguous { .. })
        ));
        assert!(matches!(
            write_rpf9_resource_capped(&sections, 1 << 20),
            Err(Rpf9WriteError::StreamCap(0x10_0000))
        ));
    }

    #[test]
    fn a_resource_extracted_as_it_is_stored_and_one_written_open_through_the_same_read() {
        // As an archive stores it, another encoder's streams after a zero slot at 16-byte ends.
        let mut v = virtual_section(0x1_0000 + 0x300, 0x60);
        (v[0x68], v[0x69]) = (2, 1);
        let p = physical(0x700);
        let mut archive = vec![0u8; 16];
        for part in [&v[..0x1_0000], &v[0x1_0000..], &p[..]] {
            archive.resize(archive.len().next_multiple_of(16), 0);
            archive.extend(deflate_fast(part));
        }
        archive.resize(archive.len().next_multiple_of(0x1000) + 0x1000, 0);
        let (_, extracted) =
            extract_rpf9_body(Cursor::new(&archive), 16, u64::MAX).expect("extracted");
        let (from_stored, _) = read(&extracted);
        assert_eq!(from_stored, mapped(v, p));
        let written = write_rpf9_resource(&from_stored).expect("written");
        assert_ne!(written, extracted);
        assert_eq!(read(&written).0, from_stored);
    }

    #[test]
    fn a_stored_rsc8_head_is_derived_from_its_size_and_carried_config() {
        let mut v = virtual_section(0x2F4, 0x40);
        v[0x48] = 1;
        let sections = Rpf9Sections {
            form: Rpf9Form::StoredRsc8 { config: 1 },
            virtual_section: v.clone(),
            physical: Vec::new(),
        };
        let written = write_rpf9_resource(&sections).expect("written");
        let mut head = vec![0u8; 16];
        LE::write_u32(&mut head, RSC8_MAGIC);
        head[4..8].copy_from_slice(&[0x41, 0x1F, 0, 1]);
        LE::write_u32(&mut head[8..], 0x300 | 4);
        LE::write_u32(&mut head[12..], 1);
        assert_eq!(written[..16], head[..]);
        let (again, resource) = read(&written);
        assert!(resource.is_none());
        v.resize(0x300, 0);
        assert_eq!(again.virtual_section, v);
        assert_eq!(again.form, Rpf9Form::StoredRsc8 { config: 1 });
        assert_eq!(write_rpf9_resource(&again).expect("again"), written);
    }

    #[test]
    fn sections_no_read_could_take_back_are_refused() {
        let refused = |sections: Rpf9Sections| write_rpf9_resource(&sections).map(|_| ());
        let mut no_map = virtual_section(0x100, 0x40);
        no_map[8..16].copy_from_slice(&0x6000_0040u64.to_le_bytes());
        assert!(matches!(
            refused(mapped(no_map, Vec::new())),
            Err(Rpf9WriteError::NoBlockMap)
        ));
        let mut past = virtual_section(0x100, 0x40);
        past[8..16].copy_from_slice(&0x5000_00F0u64.to_le_bytes());
        assert!(matches!(
            refused(mapped(past, Vec::new())),
            Err(Rpf9WriteError::MapOutside {
                at: 0xF0,
                len: 0x18,
                virtual_len: 0x100
            })
        ));
        // A map laid out for one block, where the sections need three.
        let mut tight = virtual_section(0x1_0000 + 0x100, 0x40);
        tight[0x58..0x70].fill(0x55);
        assert!(matches!(
            refused(mapped(tight, physical(0x10))),
            Err(Rpf9WriteError::MapNotZero {
                at: 0x40,
                byte: 0x58
            })
        ));
        let lone = virtual_section(STREAM_CAP as usize + 0x1_0000, 0x40);
        assert!(matches!(
            refused(mapped(lone, physical(0x10))),
            Err(Rpf9WriteError::Ambiguous { .. })
        ));
        let rsc8 = |config, physical| Rpf9Sections {
            form: Rpf9Form::StoredRsc8 { config },
            virtual_section: virtual_section(0x100, 0x40),
            physical,
        };
        assert!(matches!(
            refused(rsc8(2, Vec::new())),
            Err(Rpf9WriteError::Config(2))
        ));
        assert!(matches!(
            refused(rsc8(0, vec![1; 0x10])),
            Err(Rpf9WriteError::PhysicalInRsc8 { len: 0x10 })
        ));
    }

    #[test]
    fn a_section_of_whole_pieces_keeps_a_stored_count_of_an_empty_tail() {
        // A virtual section of two whole pieces whose map counts two virtual blocks, and a
        // physical one of one whole piece whose map counts two physical blocks.
        let mut v = virtual_section(2 * 0x1_0000, 0x40);
        v[0x48] = 2;
        let mut w = virtual_section(0x1F0, 0x40);
        (w[0x48], w[0x49]) = (1, 2);
        for (sections, counts, written_blocks) in [
            (mapped(v, Vec::new()), [2u8, 0], 1),
            (mapped(w, physical(0x1_0000)), [1u8, 2], 2),
        ] {
            let written = write_rpf9_resource(&sections).expect("written");
            let (again, resource) = read(&written);
            let resource = resource.expect("a block-mapped resource");
            assert_eq!(resource.walk, Rpf9Walk::Whole);
            assert_eq!([resource.virtual_blocks, resource.physical_blocks], counts);
            assert_eq!(resource.blocks.iter().flatten().count(), written_blocks);
            assert_eq!(again, sections);
            assert_eq!(write_rpf9_resource(&again).expect("again"), written);
        }
        // A section off whole pieces is written with the count its length gives.
        let mut short = virtual_section(0x1F0, 0x40);
        short[0x48] = 2;
        let written = write_rpf9_resource(&mapped(short, Vec::new())).expect("written");
        assert_eq!(read(&written).1.expect("a resource").virtual_blocks, 1);
    }
}
