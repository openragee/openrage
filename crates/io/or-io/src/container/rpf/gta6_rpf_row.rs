//! What the row of one RPF9 file states, for a view that shows the entry a tree has selected.

use std::borrow::Cow;

use super::{Leaf, Rpf9State, RpfArchive};
use crate::container::rpf::gta6_archive::{Gta6Entry, Gta6EntryExtra};
use crate::container::rpf::gta6_chunk_table::{entry_chunks, Chunk};
use crate::container::rpf::gta6_kinds::kind_label;

/// What the row of an RPF9 file states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rpf9Row {
    /// The kind its 24-byte row's class byte or its 20-byte row's type code names.
    pub kind: Cow<'static, str>,
    /// The bytes it is stored in.
    pub stored: u64,
    /// The bytes it holds once read.
    pub real: u64,
    pub ciphered: bool,
    /// The length of its row: 24, or 20 for a headerless resource's; `None` where it has none.
    pub row_bytes: Option<u8>,
    /// The hash of its path, where no name is recovered.
    pub hash: Option<u64>,
    /// The hash of its path as its table states it, named or not.
    pub name64: u64,
}

impl RpfArchive {
    /// What the row of file `entry_index` of an RPF9 states; `None` for a folder or another generation.
    pub fn rpf9_row(&self, entry_index: u32) -> Option<Rpf9Row> {
        let (state, at) = self.rpf9_at(entry_index)?;
        let extra = state.mount.archive.entry_extra(at);
        Some(row_of(
            state.leaves.get(at)?,
            state.mount.archive.entries.get(at)?,
            extra,
        ))
    }

    /// Whether file `entry_index` of an RPF9 is stored deflated; `None` for a folder or another generation.
    pub fn rpf9_deflated(&self, entry_index: u32) -> Option<bool> {
        let (state, at) = self.rpf9_at(entry_index)?;
        let entry = state.mount.archive.entries.get(at)?;
        let leaf = state.leaves.get(at)?;
        // Asked of every row a folder's total counts, so a row's chunk table and head are read once.
        let once = state.deflated.get(at)?;
        Some(*once.get_or_init(|| {
            // ⚠ A malformed chunk table is refused by the read; the label falls back to the body.
            let chunks = entry_chunks(&state.mount.archive, entry.index as usize)
                .ok()
                .flatten();
            deflated(leaf, chunks.as_deref(), || self.rpf9_head(entry_index))
        }))
    }

    /// The first bytes of file `entry_index`'s stored body, read from its file on disk.
    fn rpf9_head(&self, entry_index: u32) -> Option<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};
        let place = self.rpf9_place(entry_index)?;
        let mut file = std::fs::File::open(&place.file).ok()?;
        file.seek(SeekFrom::Start(place.offset)).ok()?;
        let mut head = Vec::with_capacity(HEAD);
        file.take(HEAD as u64).read_to_end(&mut head).ok()?;
        Some(head)
    }

    fn rpf9_at(&self, entry_index: u32) -> Option<(&Rpf9State, usize)> {
        let state = self.rpf9_state()?;
        let at =
            (state.entry_of.get(entry_index as usize).copied()).filter(|at| *at != u32::MAX)?;
        Some((state, at as usize))
    }
}

/// The bytes of a body's head read to tell a zlib stream from stored bytes.
const HEAD: usize = 64;

/// A nested archive is kept as it is; a chunk table says per chunk; with none, a body whose stored
/// size is not its real size padded to 16 is deflated, and one whose size is may still be one zlib
/// stream, which its clear `head` tells, as the mount's walk tells it; where the head cannot be
/// read, the sizes say stored.
fn deflated(leaf: &Leaf, chunks: Option<&[Chunk]>, head: impl FnOnce() -> Option<Vec<u8>>) -> bool {
    if leaf.archive {
        return false;
    }
    if let Some(chunks) = chunks {
        return chunks.iter().any(|c| c.zlib);
    }
    if !(leaf.real..=leaf.real.next_multiple_of(16)).contains(&leaf.stored) {
        return true;
    }
    let head = (!leaf.ciphered).then(head).flatten();
    head.is_some_and(|head| zlib_starts(&head))
}

/// Whether `head` starts a zlib stream: a deflate header whose check holds, with no preset
/// dictionary, whose first bytes inflate.
fn zlib_starts(head: &[u8]) -> bool {
    let [cmf, flg, ..] = *head else {
        return false;
    };
    let header = (u16::from(cmf) << 8) | u16::from(flg);
    if cmf & 0x0F != 8 || cmf >> 4 > 7 || header % 31 != 0 || flg & 0x20 != 0 {
        return false;
    }
    let mut out = [0u8; 4 * HEAD];
    let mut inflate = flate2::Decompress::new(true);
    inflate
        .decompress(head, &mut out, flate2::FlushDecompress::None)
        .is_ok()
}

fn row_of(leaf: &Leaf, entry: &Gta6Entry, extra: Option<Gta6EntryExtra>) -> Rpf9Row {
    let nested = extra.and_then(|x| x.nested);
    Rpf9Row {
        kind: kind_label(entry.class, nested.map(|row| row.resource_type)),
        stored: leaf.stored,
        real: leaf.real,
        ciphered: leaf.ciphered,
        row_bytes: match (entry.class, nested) {
            (Some(_), _) => Some(24),
            (None, Some(_)) => Some(20),
            (None, None) => None,
        },
        hash: entry.name.is_none().then_some(entry.name64),
        name64: entry.name64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::rpf::gta6_archive::Gta6NestedRow;

    fn entry(class: Option<u8>, name: Option<&str>) -> Gta6Entry {
        Gta6Entry {
            index: 0,
            name64: 0x0123_4567_89AB_CDEF,
            name32: None,
            name: name.map(str::to_owned),
            start: Some(0x1000),
            stored: Some(0x20),
            real: Some(0x30),
            class,
            cipher: None,
        }
    }

    fn leaf(ciphered: bool) -> Leaf {
        Leaf {
            segments: vec!["x".into()],
            offset: Some(0x1000),
            stored: 0x20,
            real: 0x30,
            archive: false,
            slot: false,
            ciphered,
            kind: None,
        }
    }

    #[test]
    fn a_24_byte_row_is_named_by_its_class_and_a_named_entry_shows_no_hash() {
        let row = row_of(&leaf(false), &entry(Some(0xFF), Some("a/b.xml")), None);
        assert_eq!(row.kind, "file");
        assert_eq!(row.row_bytes, Some(24));
        assert_eq!((row.stored, row.real, row.ciphered), (0x20, 0x30, false));
        assert_eq!(row.hash, None);
    }

    #[test]
    fn a_20_byte_row_is_named_by_its_type_code_and_an_unnamed_entry_shows_its_hash() {
        let extra = Gta6EntryExtra {
            nested: Some(Gta6NestedRow {
                resource_type: 0x02,
                ..Gta6NestedRow::default()
            }),
            ..Gta6EntryExtra::default()
        };
        let row = row_of(&leaf(true), &entry(None, None), Some(extra));
        assert_eq!(row.kind, "drawable");
        assert_eq!(row.row_bytes, Some(20));
        assert!(row.ciphered);
        assert_eq!(row.hash, Some(0x0123_4567_89AB_CDEF));
    }

    fn sized(stored: u64, real: u64) -> Leaf {
        Leaf {
            stored,
            real,
            ..leaf(false)
        }
    }

    /// No head to read, as a body of an archive held in memory has.
    fn unread() -> Option<Vec<u8>> {
        None
    }

    fn deflated(leaf: &Leaf, chunks: Option<&[Chunk]>) -> bool {
        super::deflated(leaf, chunks, unread)
    }

    #[test]
    fn a_body_stored_at_its_real_size_padded_to_16_reads_as_stored() {
        assert!(!deflated(&sized(5_189_568, 5_189_564), None));
        assert!(!deflated(&sized(0x30, 0x30), None));
        // Past the padding, or short of the real size, it is deflated.
        assert!(deflated(&sized(5_189_584, 5_189_564), None));
        assert!(deflated(&sized(5_189_563, 5_189_564), None));
    }

    #[test]
    fn a_body_with_no_chunk_table_is_told_by_its_head_within_the_padding() {
        use std::io::Write;
        // 100 bytes no deflater shrinks: zlib stores them, 2 + 5 + 100 + 4 = 111 bytes.
        let mut x = 0x9E37_79B9u32;
        let content: Vec<u8> = (0..100)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect();
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        z.write_all(&content).expect("deflates");
        let stream = z.finish().expect("deflates");
        assert_eq!(stream.len(), 111);
        let leaf = sized(112, 100);
        assert!(super::deflated(&leaf, None, || Some(stream.clone())));
        assert!(!super::deflated(&leaf, None, || Some(content.clone())));
        let ciphered = Leaf {
            ciphered: true,
            ..sized(112, 100)
        };
        assert!(
            !super::deflated(&ciphered, None, || Some(stream.clone())),
            "a ciphered head is not read in the clear"
        );
    }

    #[test]
    fn a_deflated_row_reads_as_deflated_and_its_chunk_table_decides_over_its_sizes() {
        assert!(deflated(&sized(0x20, 0x30), None));
        let chunk = |zlib| Chunk {
            inflated: 0x30,
            stored: 0x30,
            zlib,
        };
        assert!(deflated(
            &sized(0x30, 0x30),
            Some(&[chunk(false), chunk(true)][..])
        ));
        assert!(!deflated(&sized(0x20, 0x30), Some(&[chunk(false)][..])));
        let nested = Leaf {
            archive: true,
            ..sized(0x20, 0x30)
        };
        assert!(
            !deflated(&nested, None),
            "a nested archive is kept as it is"
        );
    }

    #[test]
    fn an_entry_with_no_row_says_so() {
        let row = row_of(&leaf(false), &entry(None, None), None);
        assert_eq!(row.kind, "no row");
        assert_eq!(row.row_bytes, None);
    }
}
