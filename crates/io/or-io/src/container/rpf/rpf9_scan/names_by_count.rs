//! The names of a nested archive the scan left without them, placed by counting the entries its
//! header states over the finds after it; never where a find among them may be no entry.

use std::collections::HashMap;
use std::io::{Read, Seek};

use super::{
    archive_head, names_find, Reader, Rpf9BodyKind, Rpf9Content, Rpf9Scan, Rpf9ScanError,
    Rpf9ScanFind, PIECE, RPF9_HEADER_SIZE, RPF9_SLOT,
};

/// Entries an archive may hold that the scan lists no find for: bodies it has no row for.
const MOST_UNLISTED: u32 = 64;
/// The longest gap whose bytes are read for names; a longer one places none.
const GAP_LIMIT: u64 = 16 << 20;
/// How many times every header is tried: an inner archive placed lets an outer one count.
const PASSES: usize = 4;
/// The most finds a count from one header keeps before it gives up.
const MOST_ROWS: usize = 1 << 22;

#[derive(Debug, Clone, Copy)]
enum Kind {
    Archive {
        entries: u32,
        names: u32,
    },
    Names {
        archive: u64,
    },
    /// An entry; `unbounded` for a first entry or an unknown zlib body whose end is not stated,
    /// which the count cannot tell from bytes of no entry, and for a body stored as it is whose
    /// end is not stated, whose bytes may read as further finds.
    Entry {
        unbounded: bool,
    },
    /// A chunk an audio container's table states, which is no entry of the archive.
    InContainer,
}

#[derive(Debug, Clone, Copy)]
struct Row {
    offset: u64,
    /// Where the find ends, where the scan states it; for names, where they end their archive.
    end: Option<u64>,
    kind: Kind,
}

/// The finds of one scan in the order it found them, kept small, for names placed by count.
#[derive(Debug, Default)]
pub struct Rpf9NamesByCount {
    rows: Vec<Row>,
    /// The offset of the names of each archive that has them, by its header's offset.
    names: HashMap<u64, u64>,
}

impl Rpf9NamesByCount {
    /// Keep what the count needs of `find`, the next find of the scan.
    pub fn push(&mut self, find: &Rpf9ScanFind) {
        let stated = find.stored_len.zip(find.inflated_len);
        let end = stated.map(|(stored, _)| find.offset + stored);
        let kind = match find.kind {
            Rpf9BodyKind::StoredRsc8 { .. } if find.container.is_some() => Kind::InContainer,
            Rpf9BodyKind::Archive(header) => Kind::Archive {
                entries: header.entry_count,
                names: header.names_size,
            },
            Rpf9BodyKind::Names { archive } => {
                self.names.insert(archive, find.offset);
                Kind::Names { archive }
            }
            Rpf9BodyKind::FirstEntry { .. } => Kind::Entry {
                unbounded: end.is_none(),
            },
            Rpf9BodyKind::Zlib(run) => Kind::Entry {
                unbounded: end.is_none()
                    && ((run.content == Rpf9Content::Other && run.signature.is_none())
                        || (run.chunks == 0 && run.resource.is_none())),
            },
            _ => Kind::Entry { unbounded: false },
        };
        let end = match kind {
            Kind::Names { .. } => Some(find.offset + find.span),
            _ => end,
        };
        self.rows.push(Row {
            offset: find.offset,
            end,
            kind,
        });
    }

    /// The names placed by count, in offset order, for the nested archives these finds give none.
    pub fn place<R: Read + Seek>(&mut self, file: R) -> Result<Vec<Rpf9ScanFind>, Rpf9ScanError> {
        let mut reader = Reader::new(file)?;
        let head = archive_head(reader.get(0, RPF9_HEADER_SIZE)?);
        let top = head.map(|header| u64::from(header.names_size));
        let mut placed = Vec::new();
        for _ in 0..PASSES {
            let before = placed.len();
            let mut i = 0;
            while i < self.rows.len() {
                if let Some(names) = self.try_header(&mut reader, i, top)? {
                    placed.push(names);
                }
                i += 1;
            }
            if placed.len() == before {
                break;
            }
        }
        placed.sort_by_key(|find| find.offset);
        Ok(placed)
    }

    /// The names placed by count for the header at `i`, kept among the rows; `None` where it is
    /// no nested header without names, or no count places them.
    fn try_header<R: Read + Seek>(
        &mut self,
        reader: &mut Reader<R>,
        i: usize,
        top: Option<u64>,
    ) -> Result<Option<Rpf9ScanFind>, Rpf9ScanError> {
        let row = self.rows[i];
        let Kind::Archive { entries, names } = row.kind else {
            return Ok(None);
        };
        if row.offset == 0 || self.names.contains_key(&row.offset) || !self.counts(i, entries) {
            return Ok(None);
        }
        let Some((start, end)) = self.place_one(reader, i, entries, names, top)? else {
            return Ok(None);
        };
        let at = self.rows.partition_point(|r| r.offset <= start);
        let listed = Row {
            offset: start,
            end: Some(end),
            kind: Kind::Names {
                archive: row.offset,
            },
        };
        self.rows.insert(at, listed);
        self.names.insert(row.offset, start);
        Ok(Some(names_find(start, end - start, row.offset)))
    }

    /// Whether the count applies to the header at `i`: none of its first `entries` entries, walked
    /// as [`Self::nth_entry`] walks them, is a first entry or an unknown zlib body whose end is not
    /// stated, and every inner archive among them has its names.
    fn counts(&self, i: usize, entries: u32) -> bool {
        let (mut count, mut j) = (0, i + 1);
        while j < self.rows.len() && count < entries {
            match self.rows[j].kind {
                Kind::Entry { unbounded: true } => return false,
                Kind::Entry { .. } => count += 1,
                Kind::Archive { .. } => {
                    count += 1;
                    let Some(names) = self.names_index(self.rows[j].offset) else {
                        return false;
                    };
                    j = names + 1;
                    continue;
                }
                Kind::Names { .. } | Kind::InContainer => {}
            }
            j += 1;
        }
        true
    }

    /// The index of the names of the archive whose header is at `archive`.
    fn names_index(&self, archive: u64) -> Option<usize> {
        let offset = *self.names.get(&archive)?;
        let from = self.rows.partition_point(|r| r.offset < offset);
        let own = |j: &usize| match self.rows[*j].kind {
            Kind::Names { archive: of } => of == archive,
            _ => false,
        };
        (from..self.rows.len())
            .take_while(|j| self.rows[*j].offset == offset)
            .find(own)
    }

    /// The `n`-th entry after the header at `i`, an inner archive counting one and the count
    /// going on after its names, and the index of the next entry or header after it.
    fn nth_entry(&self, i: usize, n: u32) -> Option<(usize, usize)> {
        let (mut count, mut j, mut last) = (0, i + 1, None);
        while j < self.rows.len() && count < n {
            match self.rows[j].kind {
                Kind::Archive { .. } => {
                    count += 1;
                    last = Some(j);
                    j = self.names_index(self.rows[j].offset)? + 1;
                    continue;
                }
                Kind::Entry { .. } => {
                    count += 1;
                    last = Some(j);
                }
                Kind::Names { .. } | Kind::InContainer => {}
            }
            j += 1;
        }
        let last = last.filter(|_| count == n)?;
        let skipped = |r: &Row| matches!(r.kind, Kind::Names { .. } | Kind::InContainer);
        while self.rows.get(j).is_some_and(skipped) {
            j += 1;
        }
        Some((last, j))
    }

    /// Where the entry at `last` ends as the scan states it; an inner archive ends with its names.
    fn entry_end(&self, last: usize) -> Option<u64> {
        let row = self.rows[last];
        match row.kind {
            Kind::Archive { .. } => self.rows[self.names_index(row.offset)?].end,
            _ => row.end,
        }
    }

    /// The `len` bytes of names of the header at `i` of `entries` entries, for the fewest entries
    /// the scan does not list that leave a gap holding them; `top` is the top archive's names size.
    fn place_one<R: Read + Seek>(
        &self,
        reader: &mut Reader<R>,
        i: usize,
        entries: u32,
        len: u32,
        top: Option<u64>,
    ) -> Result<Option<(u64, u64)>, Rpf9ScanError> {
        let len = u64::from(len);
        let Some(most) = entries.checked_sub(1) else {
            return Ok(None);
        };
        if len < RPF9_SLOT {
            return Ok(None);
        }
        let unlisted = 0..=most.min(MOST_UNLISTED);
        for k in unlisted.clone() {
            if let Some(names) = self.in_gap(reader, i, entries - k, len, top)? {
                return Ok(Some(names));
            }
        }
        for k in unlisted {
            if let Some(names) = self.after_end(reader, i, entries - k, len)? {
                return Ok(Some(names));
            }
        }
        Ok(None)
    }

    /// The names after the `n`-th entry: ending at the end of the last non-zero slot before the
    /// next find, or before the top archive's names where none follows, their first and last
    /// slots non-zero and only zero bytes from the entry's stated end to their start.
    fn in_gap<R: Read + Seek>(
        &self,
        reader: &mut Reader<R>,
        i: usize,
        n: u32,
        len: u64,
        top: Option<u64>,
    ) -> Result<Option<(u64, u64)>, Rpf9ScanError> {
        let Some((last, next)) = self.nth_entry(i, n) else {
            return Ok(None);
        };
        let stated = self.entry_end(last);
        let lo = stated.unwrap_or(self.rows[last].offset + RPF9_SLOT);
        let hi = match self.rows.get(next) {
            Some(row) => row.offset,
            None => {
                let file_end = reader.src.len - reader.src.len % RPF9_SLOT;
                let (Some(end), Some(top)) = (nonzero_end(reader, lo, file_end)?, top) else {
                    return Ok(None);
                };
                let Some(hi) = end.checked_sub(top) else {
                    return Ok(None);
                };
                hi
            }
        };
        if hi.saturating_sub(lo) > GAP_LIMIT {
            return Ok(None);
        }
        let Some(end) = nonzero_end(reader, lo, hi - hi % RPF9_SLOT)? else {
            return Ok(None);
        };
        let Some(start) = end.checked_sub(len) else {
            return Ok(None);
        };
        if start < lo || !start.is_multiple_of(RPF9_SLOT) {
            return Ok(None);
        }
        if zero_slot(reader, start)? || zero_slot(reader, end - RPF9_SLOT)? {
            return Ok(None);
        }
        if stated.is_some() && !all_zero(reader, lo.next_multiple_of(RPF9_SLOT), start)? {
            return Ok(None);
        }
        Ok(Some((start, end)))
    }

    /// The names at the slot the `n`-th entry's stated end reaches, ending by the next find.
    fn after_end<R: Read + Seek>(
        &self,
        reader: &mut Reader<R>,
        i: usize,
        n: u32,
        len: u64,
    ) -> Result<Option<(u64, u64)>, Rpf9ScanError> {
        let Some((last, next)) = self.nth_entry(i, n) else {
            return Ok(None);
        };
        let Some(stated) = self.entry_end(last) else {
            return Ok(None);
        };
        let start = stated.next_multiple_of(RPF9_SLOT);
        let end = start + len;
        if self.rows.get(next).is_some_and(|row| end > row.offset) {
            return Ok(None);
        }
        if zero_slot(reader, start)? || zero_slot(reader, end - RPF9_SLOT)? {
            return Ok(None);
        }
        Ok(Some((start, end)))
    }
}

/// Whether the slot at `at` holds only zero bytes; a slot past the file's end does.
fn zero_slot<R: Read + Seek>(reader: &mut Reader<R>, at: u64) -> Result<bool, Rpf9ScanError> {
    Ok(reader.get(at, RPF9_SLOT as usize)?.iter().all(|b| *b == 0))
}

/// Whether `from..to` holds only zero bytes; bytes past the file's end are zero.
fn all_zero<R: Read + Seek>(
    reader: &mut Reader<R>,
    from: u64,
    to: u64,
) -> Result<bool, Rpf9ScanError> {
    let mut at = from;
    while at < to {
        let bytes = reader.get(at, (to - at).min(PIECE) as usize)?;
        if bytes.is_empty() {
            break;
        }
        if bytes.iter().any(|b| *b != 0) {
            return Ok(false);
        }
        at += bytes.len() as u64;
    }
    Ok(true)
}

/// `end`, a slot boundary, moved back over the zero slots at or after `lo`; `None` where that
/// passes [`GAP_LIMIT`].
fn nonzero_end<R: Read + Seek>(
    reader: &mut Reader<R>,
    lo: u64,
    end: u64,
) -> Result<Option<u64>, Rpf9ScanError> {
    let floor = lo.next_multiple_of(RPF9_SLOT);
    let mut e = end;
    while e >= floor + RPF9_SLOT {
        if end - e > GAP_LIMIT {
            return Ok(None);
        }
        let from = e.saturating_sub(PIECE).max(floor);
        let bytes = reader.get(from, (e - from) as usize)?;
        if let Some(at) = bytes.iter().rposition(|b| *b != 0) {
            return Ok(Some(from + (at as u64 / RPF9_SLOT + 1) * RPF9_SLOT));
        }
        e = from;
    }
    Ok(Some(e))
}

/// The names of the archive nested at `offset` of `source`: the scan's own, else placed by count.
pub fn rpf9_names_by_count_at<R: Read + Seek>(
    mut source: R,
    offset: u64,
) -> Result<Option<Rpf9ScanFind>, Rpf9ScanError> {
    let own = |find: &Rpf9ScanFind| find.kind == (Rpf9BodyKind::Names { archive: offset });
    let mut count = Rpf9NamesByCount::default();
    {
        let mut scan = Rpf9Scan::new(&mut source, offset..u64::MAX)?;
        let entries = match scan.next().transpose()? {
            Some(find) if find.offset == offset => match find.kind {
                Rpf9BodyKind::Archive(header) => {
                    count.push(&find);
                    u64::from(header.entry_count)
                }
                _ => return Ok(None),
            },
            _ => return Ok(None),
        };
        // As many finds as the count reads after the header, and the one after its last entry.
        let (mut after, mut counted) = (0u64, 0u64);
        for find in scan.by_ref() {
            let find = find?;
            if own(&find) {
                return Ok(Some(find));
            }
            count.push(&find);
            after += 1;
            let listed = count.rows.last().map(|row| row.kind);
            let entry = !matches!(listed, Some(Kind::Names { .. } | Kind::InContainer));
            counted += u64::from(entry);
            if (after > entries && counted > entries) || count.rows.len() >= MOST_ROWS {
                break;
            }
        }
    }
    let placed = count.place(&mut source)?;
    Ok(placed.into_iter().find(own))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(offset: u64, kind: Kind) -> Row {
        Row {
            offset,
            end: Some(offset + RPF9_SLOT),
            kind,
        }
    }

    const BOUNDED: Kind = Kind::Entry { unbounded: false };
    const UNBOUNDED: Kind = Kind::Entry { unbounded: true };

    fn archive(entries: u32) -> Kind {
        Kind::Archive {
            entries,
            names: 0x40,
        }
    }

    #[test]
    fn the_count_tests_the_rows_of_the_entries_it_counts_and_not_the_first_rows() {
        // Two chunks of an audio container stand before the header's two entries.
        let count = Rpf9NamesByCount {
            rows: vec![
                row(0x1000, archive(2)),
                row(0x1100, Kind::InContainer),
                row(0x1110, Kind::InContainer),
                row(0x1200, BOUNDED),
                row(0x1300, UNBOUNDED),
            ],
            names: HashMap::new(),
        };
        assert!(!count.counts(0, 2), "the second entry's end is not stated");
        assert!(count.counts(0, 1), "the first entry's end is stated");
    }

    #[test]
    fn an_inner_archive_counts_one_entry_and_its_own_rows_are_not_the_outer_one_s() {
        let rows = vec![
            row(0x1000, archive(2)),
            row(0x2000, archive(1)),
            row(0x2100, UNBOUNDED),
            row(0x2200, Kind::Names { archive: 0x2000 }),
            row(0x2300, BOUNDED),
        ];
        let named = Rpf9NamesByCount {
            rows: rows.clone(),
            names: HashMap::from([(0x2000, 0x2200)]),
        };
        assert!(named.counts(0, 2));
        assert_eq!(named.nth_entry(0, 2), Some((4, 5)));
        let unnamed = Rpf9NamesByCount {
            rows,
            names: HashMap::new(),
        };
        assert!(!unnamed.counts(0, 2), "an inner archive with no names");
    }
}
