//! The navigation cells an RPF9's table holds a row of, through every archive nested in it and
//! ciphered rows among them: what a byte scan, which reads only clear bodies, cannot vouch for.

use std::collections::BTreeSet;
use std::io::{Cursor, Read, Seek};

use super::gta6_archive::{Gta6Class, Gta6Entry};
use super::gta6_mount::Gta6Mount;
use super::rpf9_scan::{read_rpf9_body, Rpf9BodyKind, Rpf9ZlibRun, RPF9_SLOT};
use super::rpf9_toc::GtaViKeys;
use crate::error::{IoError, Result};

/// The type code of a navigation cell's 20-byte row.
pub const NAVIGATION_MESH: u32 = 0x4F;
/// Where a navigation cell's root states its cell.
const CELL: usize = 0xC8;
/// The most a navigation body inflates to.
const BODY_LIMIT: u64 = 1 << 30;
/// How deep nested archives are mounted before the table is refused as malformed.
const DEPTH: usize = 16;

fn bad(entry: &Gta6Entry, why: impl std::fmt::Display) -> IoError {
    IoError::Invalid {
        what: "an RPF9 navigation row".into(),
        why: format!("entry {}: {why}", entry.index),
    }
}

/// Every cell a navigation row of `mount` or of an archive nested in it states.
pub fn navigation_cells<R: Read + Seek>(
    src: &mut R,
    mount: &Gta6Mount,
    keys: &GtaViKeys,
) -> Result<BTreeSet<u32>> {
    // A row whose cell does not read is an error, as it may be any cell.
    let mut cells = BTreeSet::new();
    let mut stack = vec![(mount.clone(), 0usize)];
    while let Some((mount, depth)) = stack.pop() {
        let extras = mount.archive.entry_extras();
        for (at, entry) in mount.archive.entries.iter().enumerate() {
            if entry.kind() == Some(Gta6Class::NestedArchive) {
                if !mount.entry_begins_archive(src, entry)? {
                    // A ciphered archive's header is not in the clear, and its rows are not read.
                    if entry.cipher.is_some() {
                        return Err(bad(entry, "a ciphered archive row is not read through"));
                    }
                    continue;
                }
                if depth + 1 >= DEPTH {
                    return Err(bad(entry, format!("archives nest past {DEPTH} levels")));
                }
                stack.push((mount.open_nested(src, entry, keys)?, depth + 1));
                continue;
            }
            let code = extras
                .get(at)
                .and_then(|x| x.nested)
                .map(|r| r.resource_type);
            if code == Some(NAVIGATION_MESH) {
                cells.insert(navigation_cell(src, &mount, entry, keys)?);
            }
        }
    }
    Ok(cells)
}

/// The cell the navigation row `entry` of `mount` states, its body deciphered where it is ciphered.
pub fn navigation_cell<R: Read + Seek>(
    src: &mut R,
    mount: &Gta6Mount,
    entry: &Gta6Entry,
    keys: &GtaViKeys,
) -> Result<u32> {
    let start = mount
        .absolute_start(entry)
        .ok_or_else(|| bad(entry, "it has no start"))?;
    // A 20-byte row's body follows a 16-byte slot; a ciphered one is deciphered without it.
    let read = match entry.cipher {
        None => read_rpf9_body(&mut *src, start + RPF9_SLOT, BODY_LIMIT),
        Some(_) => {
            let mut slotted = vec![0u8; RPF9_SLOT as usize];
            slotted.extend(mount.read_entry_deciphered_stored(&mut *src, entry, keys)?);
            read_rpf9_body(Cursor::new(slotted), RPF9_SLOT, BODY_LIMIT)
        }
    };
    let (find, bytes) = read.map_err(|e| bad(entry, e))?;
    let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(resource),
        ..
    }) = find.kind
    else {
        return Err(bad(entry, "its body is no resource"));
    };
    let word = (resource.sections(&bytes).0.get(CELL..CELL + 4))
        .ok_or_else(|| bad(entry, "its body is too short to state a cell"))?;
    Ok(u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
}

#[cfg(test)]
mod tests {
    use super::super::gta6_kinds::kind_label;
    use super::*;

    #[test]
    fn the_navigation_type_code_is_the_one_the_kinds_name_a_navigation_mesh() {
        assert_eq!(kind_label(None, Some(NAVIGATION_MESH)), "navigation mesh");
    }
}
