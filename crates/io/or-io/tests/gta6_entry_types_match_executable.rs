//! The streamed type suffixes agree with the three suffix tables in the user's executable.

#![cfg(feature = "gta6")]

use std::io::Read;
use std::path::PathBuf;

use or_io::container::rpf::{
    Gta6Entry, Gta6EntryExtra, Gta6EntryType, Gta6NestedRow, GTA6_EXE_ENV,
};

#[derive(Clone, Copy)]
struct Section {
    name: [u8; 8],
    rva: u64,
    offset: usize,
    size: usize,
}

struct Executable<'a> {
    bytes: &'a [u8],
    base: u64,
    sections: Vec<Section>,
}

fn word<const N: usize>(bytes: &[u8], at: usize) -> [u8; N] {
    bytes
        .get(at..at.checked_add(N).expect("field end"))
        .expect("complete executable field")
        .try_into()
        .unwrap()
}

impl<'a> Executable<'a> {
    fn open(bytes: &'a [u8]) -> Self {
        assert_eq!(&word::<2>(bytes, 0), b"MZ");
        let pe = u32::from_le_bytes(word(bytes, 0x3C)) as usize;
        assert_eq!(&word::<4>(bytes, pe), b"PE\0\0");
        assert_eq!(u16::from_le_bytes(word(bytes, pe + 4)), 0x8664);
        let count = u16::from_le_bytes(word(bytes, pe + 6)) as usize;
        let optional_size = u16::from_le_bytes(word(bytes, pe + 20)) as usize;
        let optional = pe + 24;
        assert_eq!(u16::from_le_bytes(word(bytes, optional)), 0x20B);
        let base = u64::from_le_bytes(word(bytes, optional + 24));
        let table = optional + optional_size;
        let sections = (0..count)
            .map(|i| {
                let row = table + i * 40;
                Section {
                    name: word(bytes, row),
                    rva: u64::from(u32::from_le_bytes(word(bytes, row + 12))),
                    size: u32::from_le_bytes(word(bytes, row + 16)) as usize,
                    offset: u32::from_le_bytes(word(bytes, row + 20)) as usize,
                }
            })
            .collect();
        Self {
            bytes,
            base,
            sections,
        }
    }

    fn contents(&self, section: Section) -> &'a [u8] {
        &self.bytes[section.offset..section.offset.checked_add(section.size).unwrap()]
    }

    fn text(&self, address: u64) -> &'a str {
        let rva = address.checked_sub(self.base).expect("an image address");
        let section = self
            .sections
            .iter()
            .find(|s| rva >= s.rva && rva - s.rva < s.size as u64)
            .expect("the string lies in a file-backed section");
        let offset = usize::try_from(rva - section.rva).unwrap();
        let bytes = &self.contents(*section)[offset..];
        let end = bytes
            .iter()
            .take(64)
            .position(|b| *b == 0)
            .expect("a short suffix");
        std::str::from_utf8(&bytes[..end]).expect("an ASCII suffix")
    }

    fn streamed_suffixes(&self) -> Vec<String> {
        let section = *self
            .sections
            .iter()
            .find(|s| &s.name == b".rdata\0\0")
            .unwrap();
        let data = self.contents(section);
        let addresses = |name: &[u8]| {
            data.windows(name.len())
                .enumerate()
                .filter(|(_, bytes)| *bytes == name)
                .map(|(offset, _)| self.base + section.rva + offset as u64)
                .collect::<Vec<_>>()
        };
        let (mrf, cut, gfx) = (
            addresses(b"mrf\0"),
            addresses(b"cut\0"),
            addresses(b"gfx\0"),
        );
        let mut starts = Vec::new();
        for a in &mrf {
            for b in &cut {
                for c in &gfx {
                    let anchor: Vec<u8> = [a, b, c]
                        .into_iter()
                        .flat_map(|v| v.to_le_bytes())
                        .collect();
                    starts.extend(
                        data.windows(anchor.len())
                            .enumerate()
                            .filter(|(_, bytes)| *bytes == anchor)
                            .map(|(offset, _)| offset),
                    );
                }
            }
        }
        starts.sort_unstable();
        starts.dedup();
        assert_eq!(starts.len(), 3, "three platform suffix tables");
        let stride = starts[1] - starts[0];
        assert_eq!(starts[2] - starts[1], stride);
        assert_eq!(stride % std::mem::size_of::<u64>(), 0);
        let count = stride / std::mem::size_of::<u64>();
        (0..count)
            .map(|slot| {
                let variants: Vec<&str> = starts
                    .iter()
                    .map(|start| self.text(u64::from_le_bytes(word(data, start + slot * 8))))
                    .collect();
                if variants.iter().all(|name| *name == variants[0]) {
                    variants[0].to_owned()
                } else {
                    let tails: Vec<&str> = variants
                        .iter()
                        .zip(['z', 'i', '#'])
                        .map(|(name, prefix)| {
                            name.strip_prefix(prefix).expect("the platform prefix")
                        })
                        .collect();
                    assert!(tails.iter().all(|tail| *tail == tails[0]), "slot {slot}");
                    format!("y{}", tails[0])
                }
            })
            .collect()
    }
}

#[test]
fn streamed_type_extensions_are_derived_from_the_executable_tables() {
    let Some(path) = std::env::var_os(GTA6_EXE_ENV).map(PathBuf::from) else {
        or_test_support::skip_install(
            "GTA VI resource extensions",
            format_args!("set {GTA6_EXE_ENV} to the user's executable"),
        );
        return;
    };
    const LIMIT: u64 = 512 << 20;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .expect("open the executable")
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .expect("read the executable");
    assert!(
        bytes.len() as u64 <= LIMIT,
        "executable exceeds the test's byte budget"
    );
    let suffixes = Executable::open(&bytes).streamed_suffixes();
    let entry = Gta6Entry {
        index: 0,
        name64: 0,
        name32: None,
        name: None,
        start: None,
        stored: None,
        real: None,
        class: None,
        cipher: None,
    };
    for (slot, expected) in suffixes.iter().enumerate() {
        let code = 64 + u32::try_from(slot).unwrap();
        let extra = Gta6EntryExtra {
            nested: Some(Gta6NestedRow {
                resource_type: code,
                virtual_size: 0,
                flags: 3,
                stored_pages: 0,
            }),
            ..Default::default()
        };
        assert_eq!(
            Gta6EntryType::hint(&entry, &extra).extension(),
            expected,
            "type {code}"
        );
    }
    for code in [106usize, 112] {
        assert_eq!(suffixes[code - 64], "ynmd", "the two slots are intentional");
    }
}
