#![cfg(feature = "gta6")]

use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::ops::Range;

use or_io::container::rpf::{open_at_bounded, GtaViKeys, Rpf9Builder, Rpf9Toc};
use or_test_support::rpf9;

fn encrypt(bytes: Vec<u8>, keys: &GtaViKeys) -> Vec<u8> {
    Rpf9Builder::serialize(
        &Rpf9Toc {
            bytes,
            key_id: 0,
            registry_id: 0,
            entry_count: 0,
            groups: Vec::new(),
        },
        keys,
    )
    .expect("encrypt a synthetic table")
}

struct TableOnly {
    source: Cursor<Vec<u8>>,
    allowed: Range<u64>,
    read: usize,
}

impl Read for TableOnly {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let at = self.source.position();
        if at < self.allowed.start || at + buf.len() as u64 > self.allowed.end {
            return Err(io::Error::other("read outside the table"));
        }
        let count = self.source.read(buf)?;
        self.read += count;
        Ok(count)
    }
}

impl Seek for TableOnly {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.source.seek(position)
    }
}

#[test]
fn listing_reads_the_table_alone_at_the_requested_base() {
    let keys = GtaViKeys::parse(&rpf9::keys()).expect("synthetic keys");
    let name = or_io::gta6::hash::name_hash64(b"child");
    let plain = rpf9::table(&[(name, 0x1000, 0x1000, 0)]);
    let table = encrypt(plain, &keys);
    let base = 0x1000;
    let mut bytes = vec![0x78; 1 << 20];
    bytes[base..base + table.len()].copy_from_slice(&table);
    let mut source = TableOnly {
        source: Cursor::new(bytes),
        allowed: base as u64..(base + table.len()) as u64,
        read: 0,
    };
    let archive =
        open_at_bounded(&mut source, base as u64..1 << 20, &keys).expect("table-only listing");
    assert_eq!(source.read, table.len());
    assert_eq!(archive.entries.len(), 1);
    assert_eq!(archive.entries[0].name64, name);
    assert_eq!(archive.entries[0].start, Some(0x1000));
    assert_eq!(archive.entries[0].name, None);
    assert!(
        source.read(&mut [0; 1]).is_err(),
        "the read guard must reject body access"
    );
}

#[test]
fn a_table_outside_its_parent_is_refused_before_reading_it() {
    let keys = GtaViKeys::parse(&rpf9::keys()).expect("synthetic keys");
    let table = encrypt(rpf9::table(&[(1, 0x1000, 16, 0xff)]), &keys);
    let end = table.len() as u64;
    let mut source = TableOnly {
        source: Cursor::new(table),
        allowed: 0..64,
        read: 0,
    };
    assert!(open_at_bounded(&mut source, 0..end - 16, &keys).is_err());
    assert_eq!(source.read, 64);
}

#[test]
fn a_table_end_before_the_header_is_refused_without_panicking() {
    let keys = GtaViKeys::parse(&rpf9::keys()).expect("synthetic keys");
    let mut table = rpf9::table(&[(1, 0x1000, 16, 0xff)]);
    table[4..8].fill(0);
    let end = table.len() as u64;
    let mut source = TableOnly {
        source: Cursor::new(table),
        allowed: 0..64,
        read: 0,
    };
    assert!(open_at_bounded(&mut source, 0..end, &keys).is_err());
    assert_eq!(source.read, 64);
}

#[test]
fn invalid_column_indices_are_not_silently_missing_entries() {
    let keys = GtaViKeys::parse(&rpf9::keys()).expect("synthetic keys");
    for count in [2, 256] {
        let mut plain = rpf9::table(&vec![(1, 0x1000, 16, 0xff); count]);
        let width = plain[0x2c] as usize;
        let split = u32::from_le_bytes(plain[8..12].try_into().unwrap()) as usize;
        plain[split..split + width].copy_from_slice(&(count as u16).to_le_bytes()[..width]);
        let broken = encrypt(plain.clone(), &keys);
        assert!(
            Rpf9Toc::decrypt(&broken, &keys).is_err(),
            "out-of-range row for width {width}"
        );
        plain[split..split + width].fill(0xff);
        assert!(
            Rpf9Toc::decrypt(&encrypt(plain, &keys), &keys).is_ok(),
            "the absent sentinel is valid"
        );
    }
}

#[test]
fn only_the_bounded_opener_applies_its_table_memory_budget() {
    use or_io::container::rpf::{open_at, Gta6Entry, Rpf9Header};

    struct HeaderOnly {
        header: Cursor<Vec<u8>>,
        end: u64,
        body_attempted: bool,
    }
    impl Read for HeaderOnly {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.header.position() >= 64 {
                self.body_attempted = true;
                return Err(io::Error::other("only the synthetic header is stored"));
            }
            self.header.read(buf)
        }
    }
    impl Seek for HeaderOnly {
        fn seek(&mut self, at: SeekFrom) -> io::Result<u64> {
            match at {
                SeekFrom::End(0) => Ok(self.end),
                at => self.header.seek(at),
            }
        }
    }

    let keys = GtaViKeys::parse(&rpf9::keys()).unwrap();
    let template = encrypt(rpf9::table(&[(1, 0x1000, 16, 255)]), &keys);
    let budget = 64usize << 20;
    let last_toc = budget - std::mem::size_of::<Gta6Entry>().next_multiple_of(16);
    for (bounded, toc_end, body_attempted) in [
        (true, last_toc, true),
        (true, budget, false),
        (false, budget, true),
    ] {
        let mut header = template[..64].to_vec();
        header[4..8].copy_from_slice(&(toc_end as u32).to_le_bytes());
        header[8..12].copy_from_slice(&((toc_end - 16) as u32).to_le_bytes());
        Rpf9Header::parse(&header).expect("the synthetic header reaches the budget check");
        let end = toc_end as u64;
        let mut source = HeaderOnly {
            header: Cursor::new(header),
            end,
            body_attempted: false,
        };
        let result = if bounded {
            open_at_bounded(&mut source, 0..end, &keys)
        } else {
            open_at(&mut source, 0, &keys)
        };
        assert!(result.is_err());
        assert_eq!(
            source.body_attempted, body_attempted,
            "bounded={bounded}, toc_end={toc_end}"
        );
    }
}

#[test]
fn a_nested_mount_refuses_a_table_larger_than_its_entry() {
    use or_io::container::rpf::Gta6Mount;
    let keys = GtaViKeys::parse(&rpf9::keys()).unwrap();
    let child = encrypt(rpf9::table(&[(2, 0x1000, 16, 255)]), &keys);
    let mut parent = encrypt(rpf9::table(&[(1, 0x1000, 64, 0)]), &keys);
    parent.resize(0x1000, 0);
    parent.extend(child);
    let mut source = Cursor::new(parent);
    let mount = Gta6Mount::open_root(&mut source, &keys).unwrap();
    assert!(mount
        .open_nested(&mut source, &mount.archive.entries[0], &keys)
        .is_err());
}
