//! A project built from every program of the GTA VI archive a script was read from, and written
//! as the names folder's `project.json`, which every script opened after it is decompiled with.

use std::fs::File;
use std::io::Read as _;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use or_io::container::rpf::{Gta6Mount, GtaViKeys, RPF9_MAGIC};
use or_io::gta6::script::lift::Progress;
use or_io::gta6::script::program::Program;
use or_io::gta6::script::project::index;
use or_io::gta6::script::Project;

use super::lists::PROJECT_FILE;
use super::STACK;

/// The nested archives followed below the archive: a ring of them must end.
const MAX_DEPTH: usize = 8;

/// The threads a build reads programs on at most.
const THREADS: usize = 4;

/// The file a project is written to before it takes the place of the one there; the folder's reader passes over a name with a leading dot.
const PART_FILE: &str = ".project.json.part";

/// Held while a project is written: two builds at once would write one part file.
static WRITING: Mutex<()> = Mutex::new(());

/// How far a build has come, as the thread doing it says.
#[derive(Debug, Default)]
pub struct Building {
    found: AtomicUsize,
    read: AtomicUsize,
    all: AtomicUsize,
    built: AtomicBool,
}

impl Building {
    /// The programs found in the archive so far.
    pub fn found(&self) -> usize {
        self.found.load(Ordering::Relaxed)
    }

    /// The reads of a program for the project done so far.
    pub fn read(&self) -> usize {
        self.read.load(Ordering::Relaxed)
    }

    /// The reads of a program the project takes, two for each program found; 0 until every program is found.
    pub fn all(&self) -> usize {
        self.all.load(Ordering::Relaxed)
    }

    /// Whether the project is written.
    pub fn built(&self) -> bool {
        self.built.load(Ordering::Relaxed)
    }
}

/// Whether the file at `path` starts as a GTA VI archive does.
pub(super) fn is_archive(path: &Path) -> bool {
    let mut head = [0u8; 4];
    let read = File::open(path).and_then(|mut file| file.read_exact(&mut head));
    read.is_ok() && head == RPF9_MAGIC.to_le_bytes()
}

/// Builds the project of every program of the archive at `archive` and writes it into `folder` as `project.json`, in place of the one there; the file written.
// ⚠ A worker's call, never the UI thread's: every enciphered entry is deciphered to tell.
pub(super) fn build(
    archive: &Path,
    folder: &Path,
    building: &Building,
    progress: &Progress,
) -> Result<PathBuf, String> {
    let keys = crate::state::rpf9_scan::rpf9_table::keys()?;
    let bodies = programs(archive, &keys, building, progress)?;
    let project = project_of(&bodies, building, progress)?;
    // ⚠ A stop asked while the project settles or is saved still writes nothing.
    if progress.stopped() {
        return Err(stopped());
    }
    let bytes = index::save(&project).map_err(|e| e.to_string())?;
    if progress.stopped() {
        return Err(stopped());
    }
    let written = put(folder, &bytes)?;
    building.built.store(true, Ordering::Relaxed);
    Ok(written)
}

/// What a build stopped by its user ends with.
fn stopped() -> String {
    tr!("the build was stopped").to_string()
}

/// A walk of an archive and those nested in it, and the programs it found.
struct Walk<'a> {
    file: File,
    keys: &'a GtaViKeys,
    building: &'a Building,
    progress: &'a Progress,
    bodies: Vec<Vec<u8>>,
}

impl Walk<'_> {
    /// Keeps each enciphered entry of `mount` that deciphers to a program, and walks each archive nested in it to `depth` levels.
    // Recursion ends at `depth`, which starts at `MAX_DEPTH`.
    fn walk(&mut self, mount: &Gta6Mount, depth: usize) {
        for entry in &mount.archive.entries {
            if self.progress.stopped() {
                return;
            }
            if entry.cipher.is_some() {
                // An entry that does not decipher, or is no program, is no part of the project.
                let read = mount.read_entry_decrypted(&mut self.file, entry, self.keys);
                if let Ok(bytes) = read {
                    if bytes.starts_with(&[0; 8]) && Program::read(&bytes).is_ok() {
                        self.building.found.fetch_add(1, Ordering::Relaxed);
                        self.bodies.push(bytes);
                    }
                }
            } else if depth > 0 {
                let nested = mount.entry_begins_archive(&mut self.file, entry);
                if !nested.unwrap_or(false) {
                    continue;
                }
                if let Ok(nested) = mount.open_nested(&mut self.file, entry, self.keys) {
                    self.walk(&nested, depth - 1);
                }
            }
        }
    }
}

/// The body of each program of the archive at `archive` and of the archives nested in it, in the order of their entries.
fn programs(
    archive: &Path,
    keys: &GtaViKeys,
    building: &Building,
    progress: &Progress,
) -> Result<Vec<Vec<u8>>, String> {
    let mut file = File::open(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let mount = Gta6Mount::open_root(&mut file, keys).map_err(|e| e.to_string())?;
    let mut walk = Walk {
        file,
        keys,
        building,
        progress,
        bodies: Vec::new(),
    };
    walk.walk(&mount, MAX_DEPTH);
    match progress.stopped() {
        true => Err(stopped()),
        false => Ok(walk.bodies),
    }
}

/// The project of the programs `bodies` are, as an export builds it: what they state of each other, then what their code shows once that is settled.
// ⚠ A program whose part does not read, or faults, gives the project only that its code may call
// anything; the rest are read.
fn project_of(
    bodies: &[Vec<u8>],
    building: &Building,
    progress: &Progress,
) -> Result<Project, String> {
    if bodies.is_empty() {
        return Err(tr!("no entry of this archive is a GTA VI program").to_string());
    }
    building.all.store(bodies.len() * 2, Ordering::Relaxed);
    let tables = index::tables(bodies.iter().map(Vec::as_slice));
    let parts = each(bodies, building, progress, |bytes| {
        index::of_program_in(bytes, &tables).ok()
    });
    if progress.stopped() {
        return Err(stopped());
    }
    let mut whole = Vec::with_capacity(parts.len());
    for (part, bytes) in parts.into_iter().zip(bodies) {
        let part = part.or_else(|| {
            catch_unwind(AssertUnwindSafe(|| index::unread_part(bytes)))
                .ok()
                .flatten()
        });
        whole.extend(part);
    }
    let mut project = index::merge(whole);
    let typed = each(bodies, building, progress, |bytes| {
        index::typed_part(bytes, &tables, &project).ok()
    });
    if progress.stopped() {
        return Err(stopped());
    }
    index::refine(&mut project, typed.into_iter().flatten().collect());
    Ok(project)
}

/// What `work` gives of each body, in their order, on up to [`THREADS`] threads of a deep stack; a body whose work faults gives `None`, and so does each body left once a stop is asked.
fn each<T: Send>(
    bodies: &[Vec<u8>],
    building: &Building,
    progress: &Progress,
    work: impl Fn(&[u8]) -> Option<T> + Sync,
) -> Vec<Option<T>> {
    let next = AtomicUsize::new(0);
    let done: Mutex<Vec<(usize, Option<T>)>> = Mutex::new(Vec::with_capacity(bodies.len()));
    let run = || loop {
        let n = next.fetch_add(1, Ordering::Relaxed);
        let Some(bytes) = bodies.get(n) else {
            break;
        };
        let made = match progress.stopped() {
            true => None,
            false => catch_unwind(AssertUnwindSafe(|| work(bytes)))
                .ok()
                .flatten(),
        };
        building.read.fetch_add(1, Ordering::Relaxed);
        let mut done = done.lock().unwrap_or_else(PoisonError::into_inner);
        done.push((n, made));
    };
    let threads = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .clamp(1, THREADS)
        .min(bodies.len());
    std::thread::scope(|scope| {
        let mut started = 0;
        for _ in 0..threads {
            let thread = std::thread::Builder::new().stack_size(STACK);
            started += usize::from(thread.spawn_scoped(scope, run).is_ok());
        }
        // With no thread to be had, the bodies are read here, on the worker's own deep stack.
        if started == 0 {
            run();
        }
    });
    let mut done = done.into_inner().unwrap_or_else(PoisonError::into_inner);
    done.sort_by_key(|(n, _)| *n);
    done.into_iter().map(|(_, made)| made).collect()
}

/// Writes `bytes` into `folder` as `project.json` in place of the one there, through a file the folder's reader passes over, so that no reader sees half of it.
fn put(folder: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    let _writing = WRITING.lock().unwrap_or_else(PoisonError::into_inner);
    let failed = |at: &Path, e: std::io::Error| format!("{}: {e}", at.display());
    std::fs::create_dir_all(folder).map_err(|e| failed(folder, e))?;
    let (part, file) = (folder.join(PART_FILE), folder.join(PROJECT_FILE));
    std::fs::write(&part, bytes).map_err(|e| failed(&part, e))?;
    if let Err(e) = std::fs::rename(&part, &file) {
        let _ = std::fs::remove_file(&part);
        return Err(failed(&file, e));
    }
    Ok(file)
}

#[cfg(test)]
#[path = "gta6_script_build_tests.rs"]
mod tests;
