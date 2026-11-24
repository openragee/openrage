#![cfg(feature = "gta6")]

use or_io::container::rpf::{
    open_at_bounded, Gta6Archive, Gta6EntryRead, GtaViKeys, Rpf9Builder, Rpf9Toc,
};
use or_test_support::rpf9;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::ops::Range;

struct BodyOnly {
    source: Cursor<Vec<u8>>,
    allowed: Range<u64>,
    reads: usize,
}
impl Read for BodyOnly {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let at = self.source.position();
        if at < self.allowed.start || at + buf.len() as u64 > self.allowed.end {
            return Err(io::Error::other("read outside the table's body extent"));
        }
        self.reads += 1;
        self.source.read(buf)
    }
}
impl Seek for BodyOnly {
    fn seek(&mut self, at: SeekFrom) -> io::Result<u64> {
        self.source.seek(at)
    }
}

fn fixture(plain: Vec<u8>, body: &[u8]) -> (Gta6Archive, BodyOnly, GtaViKeys) {
    let keys = GtaViKeys::parse(&rpf9::keys()).unwrap();
    let mut bytes = Rpf9Builder::serialize(
        &Rpf9Toc {
            bytes: plain,
            key_id: 0,
            registry_id: 0,
            entry_count: 0,
            groups: Vec::new(),
        },
        &keys,
    )
    .unwrap();
    bytes.resize(0x1010, 0x78);
    bytes.extend_from_slice(body);
    let end = bytes.len() as u64;
    let mut source = Cursor::new(bytes);
    let archive = open_at_bounded(&mut source, 0..end, &keys).unwrap();
    (
        archive,
        BodyOnly {
            source,
            allowed: 0x1010..end,
            reads: 0,
        },
        keys,
    )
}

fn zlib(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

#[test]
fn twenty_byte_rows_read_mixed_chunks_at_exact_table_offsets() {
    let raw = b"\x78\xda9FPRxxxxxxxxxx";
    let expected = vec![0xa5; 80_000];
    let packed = zlib(&expected);
    let table = rpf9::resource_table(&[
        (16, 16, false),
        (expected.len() as u32, packed.len() as u32, true),
    ]);
    let mut body = raw.to_vec();
    body.extend(&packed);
    body.resize(body.len().next_multiple_of(16), 0x78);
    let (archive, mut source, keys) = fixture(table, &body);
    assert!(
        source.read(&mut [0]).is_err(),
        "the guard rejects table/header reads"
    );
    let read = Gta6EntryRead::from_table(&archive, 0..source.allowed.end, 0, 100_000).unwrap();
    let expected = [raw.as_slice(), expected.as_slice()].concat();
    assert_eq!(read.read(&mut source, &keys).unwrap(), expected);
    assert_eq!(source.reads, 1);
}

#[test]
fn resource_rows_require_valid_mode_two_descriptors_before_body_reads() {
    use or_io::container::rpf::gta6_chunk_table::cell_offset;
    for case in 0..8 {
        let (mut archive, mut source, _) =
            fixture(rpf9::resource_table(&[(16, 16, false)]), &[0; 16]);
        let at = cell_offset(&archive, 0).unwrap();
        let heap =
            at + u32::from_le_bytes(archive.toc.bytes[at..at + 4].try_into().unwrap()) as usize;
        match case {
            0 => archive.toc.groups.retain(|g| g.data.start != at),
            1 => archive.toc.bytes[at + 7] = 3,
            2 => archive.toc.bytes[at + 6] = 0,
            3 => archive.toc.bytes[at + 4..at + 6].fill(0),
            4 => archive.toc.bytes[at..at + 4].fill(0xff),
            5 => archive.toc.bytes[heap + 3] = 1,
            6 => archive.toc.bytes[heap..heap + 8].fill(0),
            7 => archive.entries[0].stored = Some(32),
            _ => unreachable!(),
        }
        assert!(
            Gta6EntryRead::from_table(&archive, 0..source.allowed.end, 0, 100_000).is_err(),
            "case {case}"
        );
        assert_eq!(source.reads, 0);
        source.source.set_position(source.allowed.start - 1);
        assert!(source.read(&mut [0]).is_err());
    }
}

#[test]
fn resource_sizes_limits_and_parent_bounds_are_checked_before_reading() {
    use or_io::container::rpf::gta6_archive::GROUP_ENTRY_ROW_20;
    let (mut archive, source, _) = fixture(rpf9::resource_table(&[(32, 16, true)]), &[0; 16]);
    for (parent, limit) in [
        (0..source.allowed.end - 1, 100_000),
        (0..source.allowed.end, 31),
        (0..16, 100_000),
    ] {
        assert!(Gta6EntryRead::from_table(&archive, parent, 0, limit).is_err());
    }
    let row = archive
        .toc
        .groups
        .iter()
        .find(|g| g.name_hash == GROUP_ENTRY_ROW_20)
        .unwrap()
        .data
        .start;
    for pages in [0u32, 1, 3, u32::MAX] {
        archive.toc.bytes[row + 8..row + 12].copy_from_slice(&pages.to_le_bytes());
        assert!(Gta6EntryRead::from_table(&archive, 0..source.allowed.end, 0, 100_000).is_err());
    }
    archive.toc.bytes[row + 8..row + 12].copy_from_slice(&2u32.to_le_bytes());
    for start in [0, archive.toc.bytes.len() as u64 - 16, u64::MAX - 15] {
        archive.entries[0].start = Some(start);
        assert!(Gta6EntryRead::from_table(&archive, 0..source.allowed.end, 0, 100_000).is_err());
    }
    assert_eq!(source.reads, 0);
    let (archive, source, _) = fixture(rpf9::resource_table(&[(80_000, 16, true)]), &[0; 16]);
    assert!(Gta6EntryRead::from_table(&archive, 0..source.allowed.end, 0, 50_000).is_err());
}

#[test]
fn each_compressed_chunk_must_consume_and_produce_its_exact_sizes() {
    let packed = zlib(&[0x55; 32]);
    for (decoded, stored) in [
        (16, packed.len()),
        (48, packed.len()),
        (32, packed.len() - 1),
        (32, packed.len() + 1),
    ] {
        let mut body = packed.clone();
        body.resize(stored.next_multiple_of(16), 0);
        let (archive, mut source, keys) = fixture(
            rpf9::resource_table(&[(decoded, stored as u32, true)]),
            &body,
        );
        let read = Gta6EntryRead::from_table(&archive, 0..source.allowed.end, 0, 100_000).unwrap();
        assert!(
            read.read(&mut source, &keys).is_err(),
            "decoded {decoded}, stored {stored}"
        );
    }
}

#[test]
fn invalid_compression_never_scans_for_a_later_valid_stream() {
    let packed = zlib(&[0x55; 32]);
    let mut body = vec![0; 16];
    body.extend(&packed);
    body.resize(body.len().next_multiple_of(16), 0);
    let (archive, mut source, keys) = fixture(
        rpf9::resource_table(&[(32, 16, true), (32, packed.len() as u32, true)]),
        &body,
    );
    let read = Gta6EntryRead::from_table(&archive, 0..source.allowed.end, 0, 100_000).unwrap();
    assert!(read.read(&mut source, &keys).is_err());
}

#[test]
fn a_resource_in_a_nested_archive_uses_the_parent_base_and_skips_the_header_slot() {
    let (archive, mut source, keys) = fixture(
        rpf9::resource_table(&[(16, 16, false)]),
        b"table only bytes",
    );
    let base = 0x2000;
    let mut bytes = vec![0x78; base];
    bytes.extend(source.source.into_inner());
    source.source = Cursor::new(bytes);
    source.allowed.start += base as u64;
    source.allowed.end += base as u64;
    let read =
        Gta6EntryRead::from_table(&archive, base as u64..source.allowed.end, 0, 100_000).unwrap();
    assert_eq!(read.read(&mut source, &keys).unwrap(), b"table only bytes");
    source.source.get_mut().pop();
    source.reads = 0;
    assert!(read.read(&mut source, &keys).is_err());
    assert_eq!(source.reads, 0);
}

#[test]
fn resource_content_keys_are_required_before_reading_and_decode_the_body() {
    use or_io::container::rpf::Gta6Cipher;
    let mut snapshot = rpf9::keys();
    snapshot.extend(0x5243u32.to_le_bytes());
    for size in [4096u32, 16384] {
        snapshot.extend(size.to_le_bytes());
        snapshot.extend(vec![0; size as usize]);
    }
    snapshot.extend(1u32.to_le_bytes());
    snapshot.extend(0x0000_0001_0200_0000u64.to_le_bytes());
    snapshot.extend((4u32 + 60 * 4 + 16 * 256).to_le_bytes());
    snapshot.extend([0x5d, 0xce, 0x18, 0x27]);
    snapshot.extend([0; 60 * 4]);
    for lane in 0..16u8 {
        snapshot.extend([0x80 + lane; 256]);
    }
    let content_keys = GtaViKeys::parse(&snapshot).unwrap();
    let expected = [0x55; 32];
    let mut body = zlib(&expected);
    let table = rpf9::resource_table(&[(32, body.len() as u32, true)]);
    body.resize(body.len().next_multiple_of(16), 0);
    for (i, byte) in body.iter_mut().enumerate() {
        *byte ^= 0x80 + (i % 16) as u8;
    }
    let (mut archive, mut source, keys) = fixture(table, &body);
    archive.entries[0].cipher = Some(Gta6Cipher {
        key_id: 1,
        cipher_type: 17,
        iv: [0; 16],
        framing: [0, 0x400, 0, 0, 4],
    });
    let read = Gta6EntryRead::from_table(&archive, 0..source.allowed.end, 0, 100_000).unwrap();
    assert!(read.read(&mut source, &keys).is_err());
    assert_eq!(source.reads, 0);
    assert_eq!(read.read(&mut source, &content_keys).unwrap(), expected);
    archive.entries[0].cipher.as_mut().unwrap().framing[4] = 5;
    assert!(Gta6EntryRead::from_table(&archive, 0..source.allowed.end, 0, 100_000).is_err());
}
