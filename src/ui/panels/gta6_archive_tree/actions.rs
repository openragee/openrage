//! What an explorer row does beyond opening: extract its body to a file, and replace it with a
//! file in a new copy of the archive, which the game refuses because only its maker signs a table.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use or_io::container::rpf::gta6_kinds::entry_kinds;
use or_io::container::rpf::{
    open_at, read_rpf9_body, write_rpf9_resource, Gta6Entry, Gta6Mount, GtaViKeys, Rpf9Sections,
    RPF9_SLOT,
};

use crate::state::rpf9_scan::rpf9_table::{keys, write_entry};
use crate::state::AppState;

/// How a row action on a worker ended: the sentence to show, or why it was refused.
type Ended = Result<String, String>;

// ponytail: one list for the process, emptied by the next frame the panel draws.
static ENDED: Mutex<Vec<Ended>> = Mutex::new(Vec::new());

fn ended() -> MutexGuard<'static, Vec<Ended>> {
    ENDED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Show how each row action ended.
pub(super) fn poll(state: &mut AppState) {
    for said in std::mem::take(&mut *ended()) {
        match said {
            Ok(said) => state.show_toast(said),
            Err(why) => state.show_error_toast(why),
        }
    }
}

/// The entry a row action acts on: the chain of nested-archive indices to its table, its index
/// there and its name.
pub(super) struct Row {
    pub disk: PathBuf,
    pub node: Vec<u32>,
    pub entry: u32,
    pub name: String,
}

/// Extract the row's body to `out` on a worker.
pub(super) fn spawn_extract(row: Row, out: PathBuf) {
    std::thread::spawn(move || {
        let said = keys().and_then(|keys| extract(&row, &out, &keys));
        ended().push(match said {
            Ok(()) => Ok(tr!(
                "Wrote {entry} to {file}.",
                entry = row.name,
                file = crate::state::jobs::file_name(&out)
            )),
            Err(why) => Err(tr!(
                "Cannot extract {entry}: {reason}",
                entry = row.name,
                reason = why
            )),
        });
    });
}

/// Write a new archive `out` with the row's body replaced by the file `with`, on a worker.
pub(super) fn spawn_replace(row: Row, with: PathBuf, out: PathBuf) {
    std::thread::spawn(move || {
        let said = keys().and_then(|keys| replace(&row, &with, &out, &keys));
        let file = crate::state::jobs::file_name(&out);
        ended().push(match said {
            Ok((true, _)) => Ok(tr!(
                "{file} is a new archive with {entry} replaced, which the game refuses because only its maker can sign the table this changed.",
                file = file,
                entry = row.name
            )),
            Ok((false, true)) => Ok(tr!(
                "{file} is a new archive with the edit in its entry's place and its table as it was read.",
                file = file
            )),
            Ok((false, false)) => Ok(tr!(
                "{file} is a new archive the same as the one it was read from: the file holds the bytes {entry} holds.",
                file = file,
                entry = row.name
            )),
            Err(why) => Err(tr!(
                "Cannot replace {entry}: {reason}",
                entry = row.name,
                reason = why
            )),
        });
    });
}

/// Each entry's kind label of the table whose header is at `base` in `disk`.
pub(super) fn kinds(disk: &Path, base: u64, keys: &GtaViKeys) -> Result<Vec<String>, String> {
    let mut file = File::open(disk).map_err(|e| format!("{}: {e}", disk.display()))?;
    let archive = open_at(&mut file, base, keys).map_err(|e| e.to_string())?;
    Ok(entry_kinds(&archive)
        .into_iter()
        .map(|k| k.into_owned())
        .collect())
}

/// Whether two paths name one file: as written, or once the system has resolved both.
fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (std::fs::canonicalize(a), std::fs::canonicalize(b)),
            (Ok(a), Ok(b)) if a == b
        )
}

fn never_the_archive(out: &Path, disk: &Path) -> Result<(), String> {
    if same_file(out, disk) {
        return Err(tr!(
            "the file written is a new one, and the archive it is read from is never written"
        )
        .to_string());
    }
    Ok(())
}

/// The mount of the table the row is in, the row's entry and whether it is a headerless resource's.
fn mount_row(
    file: &mut File,
    row: &Row,
    keys: &GtaViKeys,
) -> Result<(Gta6Mount, Gta6Entry, bool), String> {
    let entry_of = |mount: &Gta6Mount, index: u32| {
        (mount.archive.entries.iter())
            .find(|e| e.index == index)
            .cloned()
            .ok_or_else(|| tr!("entry {index} is not in its table", index = index).to_string())
    };
    let mut mount = Gta6Mount::open_root(&mut *file, keys).map_err(|e| e.to_string())?;
    for &index in &row.node {
        let nested = entry_of(&mount, index)?;
        mount = (mount.open_nested(&mut *file, &nested, keys)).map_err(|e| e.to_string())?;
    }
    let entry = entry_of(&mount, row.entry)?;
    let headerless =
        (mount.archive.entry_extra(row.entry as usize)).is_some_and(|x| x.nested.is_some());
    Ok((mount, entry, headerless))
}

/// Write the row's body to `out`: a nested archive as its row's extent, an enciphered body
/// deciphered and inflated, a clear headerless resource in the loose form a replace takes back, and
/// any other body inflated.
fn extract(row: &Row, out: &Path, keys: &GtaViKeys) -> Result<(), String> {
    never_the_archive(out, &row.disk)?;
    let mut file = File::open(&row.disk).map_err(|e| format!("{}: {e}", row.disk.display()))?;
    let (mount, entry, headerless) = mount_row(&mut file, row, keys)?;
    let start = mount.absolute_start(&entry).ok_or_else(|| {
        tr!("the entry's row states no start, so its body is not known").to_string()
    })?;
    let written =
        |bytes: &[u8]| std::fs::write(out, bytes).map_err(|e| format!("{}: {e}", out.display()));
    if entry.cipher.is_some() {
        let body = mount.read_entry_decrypted(&mut file, &entry, keys);
        return written(&body.map_err(|e| e.to_string())?);
    }
    if mount
        .entry_begins_archive(&mut file, &entry)
        .map_err(|e| e.to_string())?
    {
        let stored = entry
            .stored
            .ok_or_else(|| tr!("a nested archive whose row states no stored size").to_string())?;
        file.seek(SeekFrom::Start(start))
            .map_err(|e| e.to_string())?;
        let mut sink = File::create(out).map_err(|e| format!("{}: {e}", out.display()))?;
        let copied = std::io::copy(&mut (&mut file).take(stored), &mut sink)
            .map_err(|e| format!("{}: {e}", out.display()))?;
        if copied != stored {
            drop(sink);
            let _ = std::fs::remove_file(out);
            return Err(tr!(
                "the archive ends {copied} bytes into the {stored}-byte nested archive",
                copied = copied,
                stored = stored
            )
            .to_string());
        }
        return Ok(());
    }
    if headerless {
        let read = read_rpf9_body(&mut file, start + RPF9_SLOT, u64::MAX);
        if let Some(loose) = (read.ok())
            .and_then(|(find, body)| Rpf9Sections::of(&find, &body))
            .and_then(|sections| write_rpf9_resource(&sections).ok())
        {
            return written(&loose);
        }
    }
    let body = mount
        .read_entry(&mut file, &entry)
        .map_err(|e| e.to_string())?;
    written(&body)
}

/// Write a new archive `out` with the row's body replaced by the file `with` (a headerless
/// resource's by its loose form); whether a table and whether a body changed. `out` must not exist.
fn replace(row: &Row, with: &Path, out: &Path, keys: &GtaViKeys) -> Result<(bool, bool), String> {
    let body = std::fs::read(with).map_err(|e| format!("{}: {e}", with.display()))?;
    let mut file = File::open(&row.disk).map_err(|e| format!("{}: {e}", row.disk.display()))?;
    let (_, _, headerless) = mount_row(&mut file, row, keys)?;
    drop(file);
    let path: Vec<u32> = row.node.iter().copied().chain([row.entry]).collect();
    write_entry(&row.disk, &path, headerless, body, out, keys)
}

#[cfg(test)]
#[path = "actions_tests.rs"]
mod tests;
