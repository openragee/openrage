//! What the row of one RPF9 file states, for a view that shows the entry a tree has selected.

use std::borrow::Cow;

use super::{Leaf, RpfArchive};
use crate::container::rpf::gta6_archive::{Gta6Entry, Gta6EntryExtra};
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
        let state = self.rpf9.as_deref()?;
        let at =
            (state.entry_of.get(entry_index as usize).copied()).filter(|at| *at != u32::MAX)?;
        let at = at as usize;
        let extra = state.mount.archive.entry_extra(at);
        Some(row_of(
            state.leaves.get(at)?,
            state.mount.archive.entries.get(at)?,
            extra,
        ))
    }
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
        assert_eq!(row.kind, "media");
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

    #[test]
    fn an_entry_with_no_row_says_so() {
        let row = row_of(&leaf(false), &entry(None, None), None);
        assert_eq!(row.kind, "no row");
        assert_eq!(row.row_bytes, None);
    }
}
