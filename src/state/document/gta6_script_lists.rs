//! What a decompile reads beside a program: the user's lists in the names folder, and the project
//! an export wrote, from the folder of the program's file or from the names folder.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use or_io::gta5::script::natives::NativeDb;
use or_io::gta6::script::ir::TypeKey;
use or_io::gta6::script::program::Program;
use or_io::gta6::script::project::index;
use or_io::gta6::script::{Names, Project};

/// The name of the file an export writes its project to.
pub const PROJECT_FILE: &str = "project.json";

/// The refused project files remembered: a file that is no project is not read again at every open.
const REFUSED_KEPT: usize = 8;

/// The lists of a folder as a decompile takes them.
#[derive(Clone, Debug, Default)]
pub struct Supplied {
    /// What the names lists name; its `natives` is the natives list in use.
    pub names: Names,
    /// The natives lists read, in the order of their files' names.
    pub natives: Vec<String>,
    /// Each file no reader took, with the reason of the reader tried first.
    pub refused: Vec<String>,
    /// Each names list in the order read, with how many hashes, cells, indices, functions or words it named that no list before it did.
    pub named: Vec<(String, usize)>,
    /// The files passed over for the guesses their names say they hold.
    pub candidates: Vec<String>,
    /// The files of native labels, passed over unless the user asks for them.
    pub labels: Vec<String>,
    /// Each labels file read, with how many natives it labelled.
    pub labelled: Vec<(String, usize)>,
}

/// What a file's name holds, in any case, that makes it a file of guesses the folder never reads.
// ⚠ A guessed row of a 32-bit hash proves itself as a true one does: only the file's name tells them apart.
const CANDIDATES: &str = "candidate";

/// What a file's name holds, in any case, that makes it a file of native labels, read only when asked.
// ⚠ A label is a guess at a native's name, and a natives list in form: read as one it would name natives.
const LABELS: &str = "labels";

/// The lists of the folder `dir`, in the order of their names, those whose every row proves itself first: each file is a names list where it reads as one, else a natives list.
// ⚠ The same files are names lists here and on the command line, which reads this folder too. The
// natives list in use is the one naming most of the hashes the names lists give handlers'
// addresses: a compiled program states an address, so with no such hash no natives list names anything.
// ponytail: read again for every script opened; cache them if opening scripts in bulk shows the cost.
pub fn supplied(dir: Option<&Path>) -> Supplied {
    supplied_over(None, dir)
}

/// [`supplied`] over the lists a package ships in `shipped` and the folder `dir`, a file of `dir` in place of a shipped one of its name.
pub fn supplied_over(shipped: Option<&Path>, dir: Option<&Path>) -> Supplied {
    let mut out = Supplied::default();
    let mut lists: Vec<NativeDb> = Vec::new();
    let mut by_name: std::collections::BTreeMap<std::ffi::OsString, PathBuf> = Default::default();
    for dir in [shipped, dir].into_iter().flatten() {
        let entries = std::fs::read_dir(dir).into_iter().flatten().flatten();
        for path in entries
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
        {
            if let Some(name) = path.file_name() {
                by_name.insert(name.to_owned(), path);
            }
        }
    }
    let files = by_name.into_values();
    // Each file's text, with whether every row of it proves itself by its hash.
    let mut held: Vec<(String, Result<String, String>, bool)> = Vec::new();
    for path in files {
        let label =
            (path.file_name()).map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        // A file manager leaves files of its own in a folder it has shown, named with a leading dot.
        if label == PROJECT_FILE || label.starts_with('.') {
            continue;
        }
        if label.to_ascii_lowercase().contains(CANDIDATES) {
            out.candidates.push(label);
            continue;
        }
        if label.to_ascii_lowercase().contains(LABELS) {
            out.labels.push(label);
            continue;
        }
        let text = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| String::from_utf8(bytes).map_err(|e| e.to_string()));
        let proved = text.as_ref().is_ok_and(|text| proves_alone(&label, text));
        held.push((label, text, proved));
    }
    // ⚠ Of a proof and a label on trust for one hash, the list read second is refused: the proof goes first.
    let (proofs, others): (Vec<_>, Vec<_>) = held.into_iter().partition(|(_, _, proved)| *proved);
    for (label, text, _) in proofs.into_iter().chain(others) {
        let read = text.and_then(|text| taken(&mut out.names, &label, &text));
        match read {
            Ok(Took::Natives(list)) => lists.push(list),
            Ok(Took::Names(new)) => out.named.push((label, new)),
            Err(why) => {
                tracing::warn!(list = %label, "{why}");
                out.refused.push(format!("{label}: {why}"));
            }
        }
    }
    let hashes: BTreeSet<u64> = out.names.native_hashes.values().copied().collect();
    out.names.natives = Names::best_natives(&lists, &hashes).map(|(list, _)| list.clone());
    out.natives = lists.iter().map(|list| list.label().to_owned()).collect();
    out
}

/// Reads each labels file of the folder `dir` that `supplied` passed over as a list of labels, as `--labels` reads one: a native no natives list names is written by its label, a later file's in place of an earlier's.
// ⚠ A file that reads as no list of labels is refused with the reason, as any file of the folder is.
pub fn labelled(supplied: &mut Supplied, dir: Option<&Path>) {
    let Some(dir) = dir else {
        return;
    };
    for label in supplied.labels.clone() {
        let text = std::fs::read_to_string(dir.join(&label)).map_err(|e| e.to_string());
        let names = &mut supplied.names;
        let read =
            text.and_then(|text| names.read_labels(&label, &text).map_err(|e| e.to_string()));
        match read {
            Ok(_) => {
                let new = names.labelled_by(names.label_lists.len().saturating_sub(1));
                supplied.labelled.push((label, new));
            }
            Err(why) => {
                tracing::warn!(list = %label, "{why}");
                supplied.refused.push(format!("{label}: {why}"));
            }
        }
    }
}

/// What one file of the folder was taken as.
enum Took {
    Natives(NativeDb),
    /// A names list, with how many hashes, cells, indices, functions or words it named that no list before it did.
    Names(usize),
}

/// Whether a file is tried as a natives list first: by its name, or by the JSON it holds.
fn natives_first(label: &str, text: &str) -> bool {
    let json = text.trim_start_matches('\u{feff}').trim_start();
    label.to_ascii_lowercase().starts_with("natives") || json.starts_with('{')
}

/// Whether `text`, read alone as a names list, reads whole and every row of it proves itself by its hash.
// ⚠ A row taken on trust and a word are evidence of nothing: a list holding either is no proof.
fn proves_alone(label: &str, text: &str) -> bool {
    if natives_first(label, text) {
        return false;
    }
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

/// Reads one file of the folder: into `names` as a names list, else returned as the natives list it is.
// ⚠ A names list refused part-way has named things already, so it is read into a copy that is kept only whole.
// A natives list written as `0xHASH NAME` lines reads as a names list too: a name that starts
// with `natives` says which it is meant as, and JSON is never a names list.
fn taken(names: &mut Names, label: &str, text: &str) -> Result<Took, String> {
    let as_names = |names: &mut Names| {
        let mut tried = names.clone();
        let read = tried.read_list(label, text).map_err(|e| e.to_string());
        read.map(|new| {
            *names = tried;
            Took::Names(new)
        })
    };
    let as_natives = || NativeDb::parse(label, text.as_bytes()).map_err(|e| e.to_string());
    if natives_first(label, text) {
        return match as_natives() {
            Ok(list) => Ok(Took::Natives(list)),
            Err(why) => as_names(names).map_err(|_| why),
        };
    }
    match as_names(names) {
        Ok(took) => Ok(took),
        Err(why) => as_natives().map(Took::Natives).map_err(|_| why),
    }
}

/// Names each field and method record of `program` that a list names by the hash the record holds, and each entry function a list's text proves.
pub fn bind(names: &mut Names, program: &Program) {
    for ty in &program.types {
        for field in &ty.fields {
            names.bind_field(field.name, TypeKey(ty.name), field.offset);
        }
    }
    for method in &program.methods {
        let owner = program.types.get(method.owner as usize);
        if let (Some(owner), Some(slot)) = (owner, method.slot) {
            names.bind_method(method.name, TypeKey(owner.name), slot);
        }
    }
    names.bind_entries(program.program, &program.entries);
}

/// A project file as it was when read: another file under its name is another project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub file: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
}

/// The project a decompile is given, and the project file that was found and not read.
#[derive(Clone, Debug, Default)]
pub struct Found {
    pub held: Option<(Stamp, Arc<Project>)>,
    /// The file and the reason.
    pub refused: Option<String>,
}

impl Found {
    /// The project to decompile with: the one found, else the empty one a program decompiled alone is given.
    pub fn project(&self) -> &Project {
        let held = self.held.as_ref();
        held.map_or(Project::empty(), |(_, project)| project.as_ref())
    }

    pub fn stamp(&self) -> Option<&Stamp> {
        self.held.as_ref().map(|(stamp, _)| stamp)
    }
}

/// What is kept of the project files read.
struct Kept {
    project: Option<(Stamp, Arc<Project>)>,
    refused: Vec<(Stamp, String)>,
}

/// The project read last, kept for the next script: a whole install's is hundreds of megabytes of text, read in seconds.
static KEPT: Mutex<Kept> = Mutex::new(Kept {
    project: None,
    refused: Vec::new(),
});

/// The project of a file as its bytes read.
fn read_project(file: &Path) -> Result<Project, String> {
    let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
    index::load(&bytes).map_err(|e| e.to_string())
}

/// The project of the first of `folders` that holds a `project.json` that reads; the one read last is not read again.
// ⚠ A worker's call, never the UI thread's: the lock is held while a file is read, so that two
// scripts opened together read it once.
pub fn project(folders: &[Option<&Path>]) -> Found {
    let mut found = Found::default();
    let mut kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
    for folder in folders.iter().flatten() {
        let file = folder.join(PROJECT_FILE);
        let Ok(meta) = std::fs::metadata(&file) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let stamp = Stamp {
            file,
            len: meta.len(),
            modified: meta.modified().ok(),
        };
        let known = (kept.project.as_ref())
            .filter(|(held, _)| *held == stamp)
            .map(|(_, project)| Arc::clone(project));
        if let Some(project) = known {
            found.held = Some((stamp, project));
            break;
        }
        let refused = kept.refused.iter().find(|(held, _)| *held == stamp);
        let read = match refused.map(|(_, why)| why.clone()) {
            Some(why) => Err(why),
            None => {
                // ⚠ The one kept goes first: two projects of an install at once are over a gigabyte.
                kept.project = None;
                read_project(&stamp.file)
            }
        };
        match read {
            Ok(project) => {
                let project = Arc::new(project);
                kept.project = Some((stamp.clone(), Arc::clone(&project)));
                found.held = Some((stamp, project));
                break;
            }
            Err(why) => {
                let said = format!("{}: {why}", stamp.file.display());
                if found.refused.is_none() {
                    found.refused = Some(said);
                }
                if !kept.refused.iter().any(|(held, _)| *held == stamp) {
                    if kept.refused.len() >= REFUSED_KEPT {
                        kept.refused.remove(0);
                    }
                    kept.refused.push((stamp, why));
                }
            }
        }
    }
    found
}
