//! What GTA VI's day keeps in its executable rather than its data files: the timecycle's variable
//! table (each variable's name hash, default and modifier mode) and the minutes its solar noon moves
//! by each day of the year, both found in the user's own executable by their shape.

use std::collections::HashMap;
use std::path::Path;

use crate::gta6_day_frame::{TimecycleFrame, FRAME_FLOATS};

/// The bytes of one entry of the variable table: index, name hash, default, kind, a flag, mode.
const ENTRY: usize = 0x18;
/// The entries of the table: one fewer than the frame's floats, whose last float has none.
const ENTRIES: usize = FRAME_FLOATS - 1;
/// The days of the noon table, one byte each, nought past them.
const NOON_DAYS: usize = 365;
/// The most minutes a day of the noon table moves noon by, and the least it ranges over in a year.
const NOON_MOST: u8 = 60;
const NOON_LEAST_RANGE: u8 = 20;

/// One variable of the timecycle, as its table entry states it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VarEntry {
    pub hash: u32,
    pub default: f32,
    /// 1 a scalar; 2, 4 or 7 the first of a colour; 6 the first of a pair; 0 the rest of one.
    pub kind: u32,
    /// How a modifier meets the variable, 1 to 6.
    pub mode: u32,
}

/// The timecycle's variable table, variable N at entry N.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VarTable {
    pub entries: Vec<VarEntry>,
    by_hash: HashMap<u32, usize>,
}

impl VarTable {
    /// The table whose first entry is at the start of `bytes`; `None` unless every entry states its own index, a flag of 0 or 1, a known kind and a mode of 1 to 6.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let word = |at: usize| {
            let b = bytes.get(at..at + 4)?;
            Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let mut entries = Vec::with_capacity(ENTRIES);
        for index in 0..ENTRIES {
            let at = index * ENTRY;
            let [own, hash, default, kind, flag, mode] =
                [0, 4, 8, 12, 16, 20].map(|off| word(at + off));
            let fits = own? as usize == index
                && flag? <= 1
                && matches!(kind?, 0 | 1 | 2 | 4 | 6 | 7)
                && (1..=6).contains(&mode?);
            if !fits {
                return None;
            }
            entries.push(VarEntry {
                hash: hash?,
                default: f32::from_bits(default?),
                kind: kind?,
                mode: mode?,
            });
        }
        let by_hash = (entries.iter().enumerate())
            .map(|(i, e)| (e.hash, i))
            .collect();
        Some(Self { entries, by_hash })
    }

    /// The table found in the executable image `image`: the one place its entries begin, aligned to four bytes.
    pub fn find(image: &[u8]) -> Option<Self> {
        let index_at = |at: usize| {
            (image.get(at..at + 4)).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let mut found = None;
        let last = image.len().checked_sub(ENTRIES * ENTRY)?;
        for at in (0..=last).step_by(4) {
            if index_at(at) != Some(0)
                || index_at(at + ENTRY) != Some(1)
                || index_at(at + 2 * ENTRY) != Some(2)
            {
                continue;
            }
            if let Some(table) = Self::parse(&image[at..]) {
                if found.is_some() {
                    return None;
                }
                found = Some(table);
            }
        }
        found
    }

    /// The variable whose name hashes to `hash`.
    pub fn index_of(&self, hash: u32) -> Option<usize> {
        self.by_hash.get(&hash).copied()
    }

    /// A frame of every variable at its default; the last float, which no entry states, at nought.
    pub fn defaults(&self) -> Box<TimecycleFrame> {
        let mut frame = Box::new([0.0; FRAME_FLOATS]);
        for (value, entry) in frame.iter_mut().zip(&self.entries) {
            *value = entry.default;
        }
        frame
    }
}

/// The minutes solar noon moves by on each day of the year, found in `image` by its shape: a year of bytes from 1 to 60 moving at most two from day to day over a range of at least 20, nought after; `None` unless exactly one such run is there.
pub fn find_noon_table(image: &[u8]) -> Option<Vec<u8>> {
    let mut found = None;
    let mut run = 0usize;
    let mut previous = 0u8;
    for (at, &byte) in image.iter().enumerate() {
        let smooth = (1..=NOON_MOST).contains(&byte) && (run == 0 || byte.abs_diff(previous) <= 2);
        if smooth {
            run += 1;
            previous = byte;
            continue;
        }
        if byte == 0 && run >= NOON_DAYS {
            let days = &image[at - NOON_DAYS..at];
            let (low, high) = (days.iter().min()?, days.iter().max()?);
            if high - low >= NOON_LEAST_RANGE {
                if found.is_some() {
                    return None;
                }
                found = Some(days.to_vec());
            }
        }
        run = usize::from((1..=NOON_MOST).contains(&byte));
        previous = byte;
    }
    found
}

/// What the day reads from the executable beside the install's archive `archive`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExecutableDay {
    pub variables: Option<VarTable>,
    pub noon_minutes: Vec<u8>,
}

/// The variable table and the noon table of the first executable in the folder of `archive` that holds them; each is left out where none does.
pub fn read_executable_day(archive: &Path) -> ExecutableDay {
    let mut day = ExecutableDay::default();
    let Some(folder) = archive.parent() else {
        return day;
    };
    let Ok(listing) = std::fs::read_dir(folder) else {
        return day;
    };
    let exes = (listing.flatten().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe")));
    for exe in exes {
        let Ok(file) = std::fs::File::open(&exe) else {
            continue;
        };
        // SAFETY: the map is read only and dropped before this returns; a file changed under it gives wrong bytes, which the shape tests refuse.
        let Ok(image) = (unsafe { memmap2::Mmap::map(&file) }) else {
            continue;
        };
        day.variables = day.variables.or_else(|| VarTable::find(&image));
        if day.noon_minutes.is_empty() {
            day.noon_minutes = find_noon_table(&image).unwrap_or_default();
        }
        if day.variables.is_some() && !day.noon_minutes.is_empty() {
            break;
        }
    }
    if day.variables.is_none() {
        tracing::warn!(folder = %folder.display(), "no executable beside the archive holds GTA VI's timecycle variable table");
    }
    if day.noon_minutes.is_empty() {
        tracing::warn!(folder = %folder.display(), "no executable beside the archive holds GTA VI's noon table");
    }
    day
}

#[cfg(test)]
#[path = "gta6_day_vars_tests.rs"]
mod tests;
