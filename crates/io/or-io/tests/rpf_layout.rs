use std::io::Cursor;

use or_io::container::{
    Compressor, RpfBinaryEntry, RpfDirectoryEntry, RpfEncryption, RpfEntry, RpfHeader,
    RpfResourceEntry,
};

const RPF_HEADER_SIZE: usize = 16;
const RPF_ENTRY_SIZE: usize = 16;

fn entry_roundtrip(e: RpfEntry) -> RpfEntry {
    let mut buf = Vec::new();
    e.write(&mut buf).expect("write");
    assert_eq!(buf.len(), RPF_ENTRY_SIZE);
    RpfEntry::read(&mut Cursor::new(&buf)).expect("read")
}

fn header_roundtrip(h: RpfHeader) -> RpfHeader {
    let mut buf = Vec::new();
    h.write(&mut buf).expect("write");
    assert_eq!(buf.len(), RPF_HEADER_SIZE);
    RpfHeader::read(&mut Cursor::new(&buf)).expect("read")
}

#[test]
fn header_open_archive_roundtrip() {
    let h = RpfHeader {
        entry_count: 42,
        name_heap_size: 0x80,
        name_shift: 0,
        xcompress: false,
        encryption: RpfEncryption::Open,
        platform: None,
    };
    assert_eq!(header_roundtrip(h), h);
}

#[test]
fn header_with_max_name_shift_and_xcompress() {
    let h = RpfHeader {
        entry_count: 65535,
        name_heap_size: 0x8_0000,
        name_shift: 3,
        xcompress: true,
        encryption: RpfEncryption::None,
        platform: None,
    };
    assert_eq!(header_roundtrip(h), h);
}

#[test]
fn header_preserves_unknown_encryption_key() {
    let h = RpfHeader {
        entry_count: 1,
        name_heap_size: 16,
        name_shift: 0,
        xcompress: false,
        encryption: RpfEncryption::Other(0xCAFE_BABE),
        platform: None,
    };
    assert_eq!(header_roundtrip(h), h);
}

#[test]
fn entry_directory_roundtrip() {
    let e = RpfEntry::Directory(RpfDirectoryEntry {
        name_offset: 0x12,
        first_entry_index: 4,
        entry_count: 7,
    });
    assert_eq!(entry_roundtrip(e.clone()), e);
}

#[test]
fn entry_compressed_binary_roundtrip() {
    let e = RpfEntry::Binary(RpfBinaryEntry {
        name_offset: 0x40,
        consumed_size: 0x4321,
        file_offset: 0x10_0000,
        uncompressed_size: 0x9876,
        encrypted: true,
        encrypted_raw: 1,
        compressor: Compressor::Deflate,
        tag: 0,
    });
    assert_eq!(entry_roundtrip(e.clone()), e);
}

#[test]
fn entry_uncompressed_binary_roundtrip() {
    let e = RpfEntry::Binary(RpfBinaryEntry {
        name_offset: 0xAB,
        consumed_size: 0,
        file_offset: 0x80_0000,
        uncompressed_size: 0x12345,
        encrypted: false,
        encrypted_raw: 0,
        compressor: Compressor::None,
        tag: 0,
    });
    assert_eq!(entry_roundtrip(e.clone()), e);
}

#[test]
fn entry_resource_roundtrip() {
    let e = RpfEntry::Resource(RpfResourceEntry {
        name_offset: 0x55,
        consumed_size: 0xABCDE,
        file_offset: 0x40_0000,
        system_info: 0x9001_0203,
        graphics_info: 0xA002_0304,
        compressor: Compressor::None,
        tag: 0,
    });
    assert_eq!(entry_roundtrip(e.clone()), e);
}

#[test]
fn entry_big_resource_roundtrip() {
    let e = RpfEntry::Resource(RpfResourceEntry {
        name_offset: 0xAA,
        consumed_size: 0xFF_FFFF,
        file_offset: 0x100_0000,
        system_info: 0,
        graphics_info: 0,
        compressor: Compressor::None,
        tag: 0,
    });
    let r = entry_roundtrip(e.clone());
    assert_eq!(r, e);
    if let RpfEntry::Resource(r) = r {
        assert!(r.is_big_resource());
    } else {
        panic!("expected Resource");
    }
}
