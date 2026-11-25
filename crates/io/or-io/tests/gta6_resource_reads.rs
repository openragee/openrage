#![cfg(feature = "gta6")]

use or_io::container::rpf::{
    open_at_bounded, Gta6Archive, Gta6EntryRead, GtaViKeys, Rpf9Builder, Rpf9Toc,
};
use or_test_support::rpf9;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::ops::Range;

fn reader_plan(
    archive: &Gta6Archive,
    extent: Range<u64>,
    index: u32,
    limit: u64,
) -> or_io::Result<Gta6EntryRead> {
    or_io::container::rpf::Gta6Mount {
        base: extent.start,
        archive: archive.clone(),
    }
    .read_entry_plan(index, extent.end, limit)
}

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
    fixture_at(plain, body, 0x1010)
}

fn fixture_at(plain: Vec<u8>, body: &[u8], start: usize) -> (Gta6Archive, BodyOnly, GtaViKeys) {
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
    bytes.resize(start, 0x78);
    bytes.extend_from_slice(body);
    let end = bytes.len() as u64;
    let mut source = Cursor::new(bytes);
    let archive = open_at_bounded(&mut source, 0..end, &keys).unwrap();
    (
        archive,
        BodyOnly {
            source,
            allowed: start as u64..end,
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
    let read = reader_plan(&archive, 0..source.allowed.end, 0, 100_000).unwrap();
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
            reader_plan(&archive, 0..source.allowed.end, 0, 100_000).is_err(),
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
        assert!(reader_plan(&archive, parent, 0, limit).is_err());
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
        assert!(reader_plan(&archive, 0..source.allowed.end, 0, 100_000).is_err());
    }
    archive.toc.bytes[row + 8..row + 12].copy_from_slice(&2u32.to_le_bytes());
    for start in [0, archive.toc.bytes.len() as u64 - 16, u64::MAX - 15] {
        archive.entries[0].start = Some(start);
        assert!(reader_plan(&archive, 0..source.allowed.end, 0, 100_000).is_err());
    }
    assert_eq!(source.reads, 0);
    let (archive, source, _) = fixture(rpf9::resource_table(&[(80_000, 16, true)]), &[0; 16]);
    assert!(reader_plan(&archive, 0..source.allowed.end, 0, 50_000).is_err());
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
        let read = reader_plan(&archive, 0..source.allowed.end, 0, 100_000).unwrap();
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
    let read = reader_plan(&archive, 0..source.allowed.end, 0, 100_000).unwrap();
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
    let read = reader_plan(&archive, base as u64..source.allowed.end, 0, 100_000).unwrap();
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
    let read = reader_plan(&archive, 0..source.allowed.end, 0, 100_000).unwrap();
    assert!(read.read(&mut source, &keys).is_err());
    assert_eq!(source.reads, 0);
    assert_eq!(read.read(&mut source, &content_keys).unwrap(), expected);
    archive.entries[0].cipher.as_mut().unwrap().framing[4] = 5;
    assert!(reader_plan(&archive, 0..source.allowed.end, 0, 100_000).is_err());
}

#[test]
fn media_and_resource_rows_follow_mixed_mode_three_chunks() {
    let expected = b"compressed chunk";
    let packed = zlib(expected);
    let raw = b"\x78\xda9FPRxxxxxxxxxx";
    for class in [0xff, 11] {
        let table = rpf9::chunked_table(
            class,
            &[
                (expected.len() as u32, packed.len() as u32, true),
                (raw.len() as u32, raw.len() as u32, false),
            ],
        );
        let mut body = packed.clone();
        body.resize(body.len().next_multiple_of(16), 0x78);
        body.extend(raw);
        let (archive, mut source, keys) = fixture_at(table, &body, 0x1000);
        let read = reader_plan(&archive, 0..source.allowed.end, 0, 100_000).unwrap();
        assert_eq!(
            read.read(&mut source, &keys).unwrap(),
            [expected.as_slice(), raw.as_slice()].concat()
        );
        assert_eq!(source.reads, 1);
    }
}

#[test]
fn malformed_mode_three_tables_never_fall_back_to_raw_or_counted_streams() {
    use or_io::container::rpf::{gta6_chunk_table::cell_offset, GROUP_ENTRY_ROW};
    for case in 0..6 {
        let (mut archive, mut source, keys) = fixture_at(
            rpf9::chunked_table(255, &[(16, 16, false)]),
            b"table only bytes",
            0x1000,
        );
        let at = cell_offset(&archive, 0).unwrap();
        match case {
            0 => archive.toc.bytes[at + 7] = 2,
            1 => archive.toc.bytes[at + 6] = 0,
            2 => archive.toc.bytes[at..at + 4].fill(0xff),
            3 => {
                let row = archive
                    .toc
                    .groups
                    .iter()
                    .find(|g| g.name_hash == GROUP_ENTRY_ROW)
                    .unwrap()
                    .data
                    .start;
                archive.toc.bytes[row + 16..row + 24].copy_from_slice(&17u64.to_le_bytes());
                archive.entries[0].real = Some(17);
            }
            4 => archive.entries[0].stored = Some(32),
            5 => archive.toc.bytes[at + 4..at + 6].fill(0),
            _ => unreachable!(),
        }
        let result = reader_plan(&archive, 0..source.allowed.end, 0, 100_000)
            .and_then(|read| read.read(&mut source, &keys));
        assert!(result.is_err(), "case {case}");
        assert_eq!(source.reads, 0);
    }
}

#[test]
fn mode_four_sizes_work_on_both_row_shapes() {
    use or_io::container::rpf::gta6_chunk_table::cell_offset;
    for resource in [false, true] {
        let plain = if resource {
            rpf9::resource_table(&[(16, 16, false)])
        } else {
            rpf9::chunked_table(64, &[(16, 16, false)])
        };
        let (mut archive, mut source, keys) = fixture_at(
            plain,
            b"table only bytes",
            if resource { 0x1010 } else { 0x1000 },
        );
        let at = cell_offset(&archive, 0).unwrap();
        let heap =
            at + u32::from_le_bytes(archive.toc.bytes[at..at + 4].try_into().unwrap()) as usize;
        archive.toc.bytes[at + 7] = 4;
        archive.toc.bytes[heap..heap + 8].copy_from_slice(&(16u64 | (1u64 << 28)).to_le_bytes());
        let read = reader_plan(&archive, 0..source.allowed.end, 0, 100_000).unwrap();
        assert_eq!(read.read(&mut source, &keys).unwrap(), b"table only bytes");
    }
}

#[test]
fn a_resource_without_a_chunk_record_is_one_bounded_raw_deflate_stream() {
    let expected = b"a single headerless DEFLATE stream";
    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(expected).unwrap();
    let mut packed = encoder.finish().unwrap();
    packed.resize(packed.len().next_multiple_of(16), 0x78);
    let (mut archive, mut source, keys) = fixture_at(
        rpf9::table(&[(1, 0x1000, packed.len() as u64, 11)]),
        &packed,
        0x1000,
    );
    archive.entries[0].real = Some(expected.len() as u64);
    let read = reader_plan(&archive, 0..source.allowed.end, 0, 100_000).unwrap();
    assert_eq!(read.read(&mut source, &keys).unwrap(), expected);
    source.source.get_mut()[0x1000..0x1010].fill(0);
    assert!(read.read(&mut source, &keys).is_err());
}

#[test]
fn mode_four_streams_across_buffers_and_refuses_bad_flags_sizes_and_padding() {
    use or_io::container::rpf::gta6_chunk_table::cell_offset;
    let expected: Vec<u8> = (0..200_003u32)
        .map(|i| (i.wrapping_mul(1103515245).rotate_left(i % 31) >> 24) as u8)
        .collect();
    let compressed = zlib(&expected);
    assert!(compressed.len() > 65536);
    let aligned = compressed.len().next_multiple_of(16);
    let mut body = compressed.clone();
    body.resize(aligned, 0x78);
    for resource in [false, true] {
        let template = if resource {
            rpf9::resource_table(&[(16, 16, false)])
        } else {
            rpf9::chunked_table(105, &[(16, 16, false)])
        };
        let (mut archive, mut source, keys) =
            fixture_at(template, &body, if resource { 0x1010 } else { 0x1000 });
        let at = cell_offset(&archive, 0).unwrap();
        let heap =
            at + u32::from_le_bytes(archive.toc.bytes[at..at + 4].try_into().unwrap()) as usize;
        archive.toc.bytes[at + 7] = 4;
        let word = expected.len() as u64
            | ((aligned as u64 / 16) << 28)
            | (((aligned - compressed.len()) as u64) << 52)
            | (3 << 56);
        archive.toc.bytes[heap..heap + 8].copy_from_slice(&word.to_le_bytes());
        if resource {
            use or_io::container::rpf::GROUP_ENTRY_ROW_20;
            let row = archive
                .toc
                .groups
                .iter()
                .find(|g| g.name_hash == GROUP_ENTRY_ROW_20)
                .unwrap()
                .data
                .start;
            archive.toc.bytes[row + 8..row + 12]
                .copy_from_slice(&(((aligned + 16) / 16) as u32).to_le_bytes());
        } else {
            archive.entries[0].stored = Some(aligned as u64);
            archive.entries[0].real = Some(expected.len() as u64);
        }
        let plan = reader_plan(&archive, 0..source.allowed.end, 0, 1 << 20).unwrap();
        let mut decoded = Vec::new();
        assert_eq!(
            plan.extract(&mut source, &keys, &mut decoded).unwrap(),
            expected.len() as u64
        );
        assert_eq!(decoded, expected);
        assert!(source.reads > 1);
        struct Refuse;
        impl Write for Refuse {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("destination full"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        assert!(plan
            .extract(&mut source, &keys, &mut Refuse)
            .unwrap_err()
            .to_string()
            .contains("destination full"));
        for invalid in [
            word | (1 << 63),
            (word & !(0xff << 56)) | (1 << 56),
            (1 << 52) | (3 << 56),
        ] {
            archive.toc.bytes[heap..heap + 8].copy_from_slice(&invalid.to_le_bytes());
            source.reads = 0;
            assert!(reader_plan(&archive, 0..source.allowed.end, 0, 1 << 20).is_err());
            assert_eq!(source.reads, 0);
        }
        source.source.get_mut()[source.allowed.start as usize..source.allowed.start as usize + 16]
            .fill(0);
        assert!(plan.extract(&mut source, &keys, &mut Vec::new()).is_err());
    }
}

#[test]
fn mount_reports_clear_chunk_counts_and_exact_consumption() {
    use or_io::container::rpf::Gta6Mount;
    let (archive, mut source, keys) = fixture_at(
        rpf9::chunked_table(255, &[(16, 16, false), (16, 16, false)]),
        b"first raw chunk!second raw chunk",
        0x1000,
    );
    let mount = Gta6Mount { base: 0, archive };
    let report = mount
        .read_entry_decrypted_report(&mut source, &mount.archive.entries[0], &keys)
        .unwrap();
    assert_eq!(report.data, b"first raw chunk!second raw chunk");
    assert_eq!(report.chunk_count, 2);
    assert_eq!(report.stored_consumed, 32);
}

#[test]
fn a_mount_refuses_resource_bytes_without_chunk_instructions() {
    use or_io::container::rpf::{gta6_archive::GROUP_CHUNK_RECORD, Gta6Mount};
    let packed = zlib(&[0x55; 32]);
    let mut body = packed.clone();
    body.resize(body.len().next_multiple_of(16), 0);
    let (mut archive, source, _) = fixture(
        rpf9::resource_table(&[(32, packed.len() as u32, true)]),
        &body,
    );
    archive
        .toc
        .groups
        .retain(|g| g.name_hash != GROUP_CHUNK_RECORD);
    let mount = Gta6Mount { base: 0, archive };
    let error = mount
        .inflate_deciphered_stored(&body, &mount.archive.entries[0])
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("20-byte resource row requires a chunk table"));
    assert_eq!(source.reads, 0);
}
