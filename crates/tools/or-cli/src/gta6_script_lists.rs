//! The lists a user supplies to name what a compiled GTA VI script only numbers: names lists and
//! natives lists, named on the command line or kept in one folder of the configuration directory.
//!
//! ⚠ **The folder is read by one rule by every verb of the command line.** The app reads the same
//! folder and tries a file that holds JSON as a natives list first. Its files are taken in the order of
//! their names. A file whose name starts with `natives` is a natives list where it reads as one,
//! else a names list; any other file is a names list where it reads as one, else a natives list.
//! So a table of `hash` and `rva` gives the handlers' hashes whatever it is called. A file that
//! reads as neither is passed over and said. A file whose name starts with a dot, and
//! `project.json`, are no lists and are not read; one whose name holds `candidate` holds guesses
//! and is read only where `--names` names it. A list each row of which proves itself by its hash
//! is read before every other, so that a label another file gives on trust never displaces it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use or_io::gta5::script::natives::NativeDb;
use or_io::gta6::script::native_addresses::NativeAddresses;
use or_io::gta6::script::Names;

use super::PROJECT_FILE;

/// The folder of the configuration directory whose files are lists, read when no names list is named.
pub const NAMES_FOLDER: &str = "gta6_script_names";

/// What a file's name starts with, in any case, that is tried as a natives list first.
const NATIVES_PREFIX: &str = "natives";

/// What a file's name holds, in any case, that makes it a file of guesses the folder never reads.
// ⚠ A guessed row of a 32-bit hash proves itself as a true one does: only the file's name tells them apart.
pub const CANDIDATES: &str = "candidate";

/// Why natives lists name nothing where no names list is a table of handlers.
const NO_HANDLERS: &str = "a natives list names by hash, and no names list gives the hash of a \
     handler by its address (a table of `hash` and `rva`)";

/// Why natives lists name nothing where none holds a hash of a handler.
const NO_HASH_HELD: &str = "none of them holds a hash the names lists give a handler";

/// The user's lists as a decompile takes them, and what is to be said of those that named nothing.
#[derive(Default)]
pub struct Supplied {
    pub names: Names,
    /// The natives lists read, by label, where none of them names a native.
    pub idle: Vec<String>,
    /// Why none of them does.
    pub idle_why: &'static str,
    /// Each file of the folder that read as no list, with why.
    pub refused: Vec<String>,
    /// Each names list in the order read, with how many hashes, cells and indices it named that no list before it did.
    pub named: Vec<(String, usize)>,
    /// The files of the folder passed over for the guesses their names say they hold.
    pub candidates: Vec<String>,
}

impl Supplied {
    /// What a source's header says of the lists beyond what its printer says of them.
    pub fn said(&self) -> Vec<String> {
        let quoted = |labels: &[String]| -> String {
            let labels: Vec<String> = labels.iter().map(|label| format!("`{label}`")).collect();
            labels.join(", ")
        };
        let mut said = Vec::new();
        if !self.named.is_empty() {
            let each: Vec<String> = (self.named.iter())
                .map(|(label, new)| format!("`{label}` {new}"))
                .collect();
            said.push(format!(
                "lists read, in order, each with how many hashes, cells, indices, functions or \
                 words it named that no list before it did: {}",
                each.join(", ")
            ));
        }
        if !self.candidates.is_empty() {
            said.push(format!(
                "candidates: {} of the names folder passed over, as a file whose name holds \
                 `{CANDIDATES}` holds guesses; --names reads it",
                quoted(&self.candidates)
            ));
        }
        if !self.idle.is_empty() {
            said.push(format!(
                "natives lists: {} read, and none names a native here: {}",
                quoted(&self.idle),
                self.idle_why
            ));
        }
        said
    }
}

/// What the files of a folder gave.
#[derive(Default)]
pub struct Folder {
    pub names: Names,
    /// In the order of their files' names.
    pub natives: Vec<NativeDb>,
    /// Each file that read as no list, with why.
    pub refused: Vec<String>,
    /// Each names list in the order read, with what it newly named.
    pub named: Vec<(String, usize)>,
    /// The files passed over for their names, in the order of those names.
    pub candidates: Vec<String>,
}

/// Whether a file of the folder is called as a file of guesses is.
pub fn guessed(label: &str) -> bool {
    label.to_ascii_lowercase().contains(CANDIDATES)
}

/// Whether `bytes`, read alone as a names list, read whole and every row of them proved itself by its hash.
// ⚠ A row taken on trust and a word are evidence of nothing: a list holding either is no proof.
fn proves_alone(label: &str, bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let mut alone = Names::default();
    if alone.read_list(label, text).is_err() {
        return false;
    }
    let words = alone.candidates.iter().sum::<usize>();
    alone
        .rows
        .iter()
        .all(|(proved, trusted)| *proved > 0 && *trusted == 0)
        && words == 0
}

/// Where lists are read from when none is named, under the configuration directory the app also uses.
pub fn names_dir(config_root: &Path) -> PathBuf {
    config_root.join(NAMES_FOLDER)
}

/// The lists of the folder `dir`, each file read by the one rule; a folder that is not there holds none.
pub fn folder(dir: &Path) -> Folder {
    let entries = std::fs::read_dir(dir).into_iter().flatten().flatten();
    let mut files: Vec<PathBuf> = entries
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    let mut found = Folder::default();
    // Each file read once, with whether it is a list every row of which proves itself.
    let mut held: Vec<(String, Vec<u8>, bool)> = Vec::new();
    for path in files {
        let label = path.file_name().map(|name| name.to_string_lossy());
        let label = label.unwrap_or_default().into_owned();
        // ⚠ A project is hundreds of megabytes of JSON, which a natives list also is: it is never tried as one.
        if label.starts_with('.') || label == PROJECT_FILE {
            continue;
        }
        if guessed(&label) {
            found.candidates.push(label);
            continue;
        }
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                let refused = format!("`{label}` of the names folder was not read: {e}");
                found.refused.push(refused);
                continue;
            }
        };
        let natives_first = label.to_ascii_lowercase().starts_with(NATIVES_PREFIX);
        let proved = !natives_first && proves_alone(&label, &bytes);
        held.push((label, bytes, proved));
    }
    // ⚠ Of a proof and a label on trust for one hash, the list read second is refused: the proof goes first.
    let (proofs, others): (Vec<_>, Vec<_>) = held.into_iter().partition(|(_, _, proved)| *proved);
    for (label, bytes, _) in proofs.into_iter().chain(others) {
        let as_natives = |found: &mut Folder| {
            let list = NativeDb::parse(label.as_str(), &bytes).map_err(|e| e.to_string())?;
            found.natives.push(list);
            Ok::<(), String>(())
        };
        let as_names = |found: &mut Folder| {
            let text = std::str::from_utf8(&bytes)
                .map_err(|e| format!("names list `{label}` is not UTF-8: {e}"))?;
            // ⚠ A list that is refused leaves the lines before its bad one read, so it is tried on a copy.
            let mut tried = found.names.clone();
            let new = tried.read_list(&label, text).map_err(|e| e.to_string())?;
            found.names = tried;
            found.named.push((label.clone(), new));
            Ok::<(), String>(())
        };
        let natives_first = label.to_ascii_lowercase().starts_with(NATIVES_PREFIX);
        let read: Result<(), (String, String)> = match natives_first {
            true => as_natives(&mut found)
                .or_else(|first| as_names(&mut found).map_err(|second| (first, second))),
            false => as_names(&mut found)
                .or_else(|first| as_natives(&mut found).map_err(|second| (first, second))),
        };
        if let Err((first, second)) = read {
            found.refused.push(format!(
                "`{label}` of the names folder is no list and was passed over: {first}; {second}"
            ));
        }
    }
    found
}

/// The lists a decompile takes. Names lists: those named, each of which must read, else the folder's under `config`. Natives lists: those named, each of which must read, else the folder's with those of the variable and of a download; of them the one naming most of the hashes the names lists give handlers.
pub fn supplied(
    named: &[PathBuf],
    natives: &[PathBuf],
    config: Option<&Path>,
) -> Result<Supplied, String> {
    let read = match named.is_empty() {
        true => config.map_or_else(Folder::default, |root| folder(&names_dir(root))),
        false => {
            let (names, counted) = super::super::lifted::names_counted(named)?;
            Folder {
                names,
                named: counted,
                ..Folder::default()
            }
        }
    };
    let Folder {
        mut names,
        natives: kept,
        refused,
        named: counted,
        candidates,
    } = read;
    // ⚠ Like the folder, the table the listings left in the configuration directory is read only
    // where no names list is named: a run that names its lists takes nothing it was not given.
    // It states a handler no list states, and names natives only until a natives list is chosen.
    if let (true, Some(root)) = (named.is_empty(), config) {
        if let Some(cache) = NativeAddresses::cached(root) {
            cache.supply(&mut names);
        }
    }
    let hashes: BTreeSet<u64> = names.native_hashes.values().copied().collect();
    let mut lists = match natives.is_empty() {
        true => kept,
        // A natives list that is named must read, whatever it then names.
        false => super::super::load_lists(natives.to_vec())?.dbs().to_vec(),
    };
    // ⚠ The lists of the variable and of a download are looked for only where a hash could be
    // named: they are read whole on every run, and they name GTA V's natives.
    if natives.is_empty() && !hashes.is_empty() {
        let ambient = super::super::load_lists(Vec::new())?;
        lists.extend(ambient.dbs().iter().cloned());
    }
    let labels = || -> Vec<String> { lists.iter().map(|list| list.label().to_owned()).collect() };
    // A natives list names by hash: with no hash of any handler it names nothing.
    let (idle, idle_why) = match hashes.is_empty() {
        true => (labels(), NO_HANDLERS),
        false => match Names::best_natives(&lists, &hashes) {
            Some((list, _)) => {
                names.natives = Some(list.clone());
                (Vec::new(), "")
            }
            None => (labels(), NO_HASH_HELD),
        },
    };
    Ok(Supplied {
        names,
        idle,
        idle_why,
        refused,
        named: counted,
        candidates,
    })
}

#[cfg(test)]
#[path = "gta6_script_lists_tests.rs"]
mod tests;
