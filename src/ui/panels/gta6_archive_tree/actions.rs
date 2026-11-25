//! What an explorer row does beyond opening: extract its body to a file, and replace it with a
//! file in a new copy of the archive, which the game refuses because only its maker signs a table.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use or_io::container::rpf::{Gta6Entry, Gta6Mount, GtaViKeys};

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
pub(super) fn spawn_extract(
    row: Row,
    source: crate::state::rpf9::entry::EntrySource,
    out: PathBuf,
) {
    std::thread::spawn(move || {
        let said = extract_source(&row, &source, &out);
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

fn extract_source(
    row: &Row,
    source: &crate::state::rpf9::entry::EntrySource,
    out: &Path,
) -> Result<(), String> {
    never_the_archive(out, &row.disk)?;
    let parent = out
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    source.extract(temporary.as_file_mut())?;
    temporary.persist(out).map_err(|e| e.to_string())?;
    Ok(())
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

/// Replace a supported entry in a new archive; headerless resources remain read-only.
fn replace(row: &Row, with: &Path, out: &Path, keys: &GtaViKeys) -> Result<(bool, bool), String> {
    let mut file = File::open(&row.disk).map_err(|e| format!("{}: {e}", row.disk.display()))?;
    let (_, _, headerless) = mount_row(&mut file, row, keys)?;
    if headerless {
        return Err(crate::i18n::tr(crate::state::rpf9::entry::READ_ONLY).to_string());
    }
    drop(file);
    let body = std::fs::read(with).map_err(|e| format!("{}: {e}", with.display()))?;
    let path: Vec<u32> = row.node.iter().copied().chain([row.entry]).collect();
    write_entry(&row.disk, &path, headerless, body, out, keys)
}

#[cfg(test)]
#[path = "actions_tests.rs"]
mod tests;
