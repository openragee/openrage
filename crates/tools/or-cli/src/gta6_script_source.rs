//! Where a compiled GTA VI script's bytes come from: a file told by its first bytes, one entry of
//! an archive, or every program of a build folder, an archive or a folder.
//!
//! ⚠ A file is told by its first 64 bytes and never read to be told: an archive is tens of
//! gigabytes. Eight zero bytes only send a file to the program reader, whose refusal stands.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use or_io::container::rpf::{Gta6Entry, Gta6Mount, GtaViKeys, NameResolver, RPF9_MAGIC};
use or_io::gta6::hash::name_hash64;
use or_io::gta6::script::program::Program;

use crate::rpf9::{build_archives, entry_matches, load_keys, load_resolver};

/// The bytes of a file that say what it is.
const HEAD: u64 = 64;

/// The nested archives followed below an archive and the folders below a folder: a ring of either must end.
const MAX_DEPTH: usize = 8;

/// What a file of text is told.
pub const TEXT: &str =
    "text, not a compiled script; if it is a lifted listing, `script lifted` reads it";

/// What the first bytes of a file say it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Head {
    /// Eight zero bytes: perhaps a deciphered GTA VI program.
    Program,
    /// A GTA VI archive.
    Archive,
    Text,
    /// A GTA V resource, which GTA V's reader keeps.
    Resource,
    /// Nothing this command knows: GTA V's reader keeps it, with its refusal of it.
    Other,
}

/// What opens an archive: the key snapshot `--keys` names, and the executable `--exe` names, which a masked snapshot opens with.
#[derive(Clone, Copy, Default)]
pub struct Keys<'a> {
    pub snapshot: Option<&'a Path>,
    pub exe: Option<&'a Path>,
}

/// What the first 64 bytes of `reader` say a file is; nothing past them is read.
pub fn head_of(reader: impl Read) -> std::io::Result<Head> {
    let mut head = Vec::new();
    reader.take(HEAD).read_to_end(&mut head)?;
    let control = |b: &u8| (*b < 0x20 && !matches!(*b, b'\t' | b'\n' | b'\r')) || *b == 0x7F;
    Ok(if head.starts_with(b"RSC7") {
        Head::Resource
    } else if head.starts_with(&RPF9_MAGIC.to_le_bytes()) {
        Head::Archive
    } else if head.starts_with(&[0; 8]) {
        Head::Program
    } else if !head.is_empty() && !head.iter().any(control) {
        Head::Text
    } else {
        Head::Other
    })
}

/// The bytes one command reads as a program, and where they came from.
pub struct Source {
    pub bytes: Vec<u8>,
    /// The file, or the archive and the entry.
    pub read_from: String,
}

/// One body that may be a program, from an archive or a folder.
pub struct Body {
    /// The archive it lies in, from the root, with `/`; `None` for a file of a folder.
    pub archive: Option<String>,
    /// Its entry in that archive.
    pub entry: Option<Gta6Entry>,
    /// What a line about it starts with.
    pub label: String,
    pub bytes: Vec<u8>,
}

/// The bodies a walk found, and what it could not read.
#[derive(Default)]
pub struct Found {
    pub bodies: Vec<Body>,
    /// The enciphered entries the walk met.
    pub enciphered: usize,
    /// Those that did not decipher, and the archives that did not open.
    pub unread: usize,
    /// The files of a folder that do not start as a program does, and so were not read.
    pub passed_over: usize,
}

/// The file `paths` starts with, or with `entry` the entry of the archive it names down the rest of `paths`.
pub fn open(paths: &[String], entry: Option<&str>, keys: Keys<'_>) -> Result<Source, String> {
    let Some((path, inner)) = paths.split_first() else {
        return Err("no file was named".to_owned());
    };
    let head = File::open(path).and_then(head_of);
    match (head.map_err(|e| format!("read {path}: {e}"))?, entry) {
        (Head::Archive, Some(entry)) => Reader::new(keys)?.entry(Path::new(path), inner, entry),
        (Head::Archive, None) => Err(format!(
            "{path} is a GTA VI archive: name one entry of it with --entry, as `script list` \
             prints them"
        )),
        _ if entry.is_some() || !inner.is_empty() => Err(format!(
            "{path} is no archive, so it has no entry and no inner path"
        )),
        (Head::Program, _) => Ok(Source {
            bytes: std::fs::read(path).map_err(|e| format!("read {path}: {e}"))?,
            read_from: path.clone(),
        }),
        (Head::Text, _) => Err(format!("{path}: {TEXT}")),
        (Head::Resource | Head::Other, _) => Err(format!(
            "{path} is neither a GTA VI archive nor a deciphered GTA VI program"
        )),
    }
}

/// What reads an archive: the key snapshot, and the entry names recovered so far.
pub struct Reader {
    keys: GtaViKeys,
    resolver: NameResolver,
}

impl Reader {
    /// With the snapshot `keys` names, else the one a session supplies for the install.
    pub fn new(keys: Keys<'_>) -> Result<Reader, String> {
        Ok(Reader {
            keys: load_keys(keys.snapshot, keys.exe)?,
            resolver: load_resolver(None),
        })
    }

    /// Opens the archive at `path` and each nested archive `inner` names in turn; the last is returned under its label.
    fn mount(&self, path: &Path, inner: &[String]) -> Result<(File, Gta6Mount, String), String> {
        let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut mount = Gta6Mount::open_root(&mut file, &self.keys).map_err(|e| e.to_string())?;
        mount.archive.resolve_names(&self.resolver);
        let mut label = name_of(path);
        let parts = inner.iter().flat_map(|part| part.split(['/', '\\']));
        for part in parts.filter(|part| !part.is_empty()) {
            let mut entries = mount.archive.entries.iter();
            let named = entries.find(|entry| entry_matches(entry, part)).cloned();
            let entry = named.ok_or_else(|| format!("no entry {part:?} in {label}"))?;
            let nested = mount.entry_begins_archive(&mut file, &entry);
            if !nested.map_err(|e| e.to_string())? {
                return Err(format!(
                    "{part:?} is an entry of {label}, not a nested archive"
                ));
            }
            let opened = mount.open_nested(&mut file, &entry, &self.keys);
            mount = opened.map_err(|e| e.to_string())?;
            mount.archive.resolve_names(&self.resolver);
            label = format!("{label}/{}", entry.display_name());
        }
        Ok((file, mount, label))
    }

    /// The body of the entry `selector` names in the archive at `path`, down `inner`.
    fn entry(&self, path: &Path, inner: &[String], selector: &str) -> Result<Source, String> {
        let (mut file, mount, label) = self.mount(path, inner)?;
        let entries = &mount.archive.entries;
        // A name the table does not hold is hashed as an entry's name is, and matched.
        let hashed = name_hash64(selector.as_bytes());
        let named = entries.iter().find(|entry| entry_matches(entry, selector));
        let found = named.or_else(|| entries.iter().find(|entry| entry.name64 == hashed));
        let Some(entry) = found else {
            return Err(format!("no entry {selector:?} in {label}"));
        };
        let read = match entry.cipher {
            Some(_) => mount.read_entry_decrypted(&mut file, entry, &self.keys),
            None => {
                let nested = mount.entry_begins_archive(&mut file, entry);
                if nested.map_err(|e| e.to_string())? {
                    return Err(format!(
                        "{selector:?} is a nested archive of {label}: name it in the inner path, \
                         and an entry of it with --entry"
                    ));
                }
                mount.read_entry(&mut file, entry)
            }
        };
        Ok(Source {
            bytes: read.map_err(|e| format!("{label}, entry {selector:?}: {e}"))?,
            read_from: format!("{label}, entry 0x{:016X}", entry.name64),
        })
    }

    /// Every enciphered entry of the archive at `path`, down `inner`, whose deciphered body reads as a program, with those of the archives nested in it to `depth` levels; the archive goes by `label`, else by its file's name and `inner`.
    // ponytail: every enciphered body is deciphered and read whole to tell; a ranged decipher and
    // a reader of the head alone if listing shows the cost.
    pub fn programs(
        &self,
        path: &Path,
        inner: &[String],
        depth: usize,
        label: Option<String>,
    ) -> Result<Found, String> {
        if !self.keys.has_content_keys() {
            return Err(
                "the key snapshot holds no content keys, so no enciphered entry, and so no \
                 program, can be read"
                    .to_owned(),
            );
        }
        let (mut file, mount, named) = self.mount(path, inner)?;
        let label = label.unwrap_or(named);
        let mut found = Found::default();
        self.walk(&mut file, &mount, &label, depth.min(MAX_DEPTH), &mut found);
        Ok(found)
    }

    // Recursion ends at `depth`, which no caller sets above `MAX_DEPTH`.
    fn walk(&self, file: &mut File, mount: &Gta6Mount, label: &str, depth: usize, to: &mut Found) {
        for entry in &mount.archive.entries {
            if entry.cipher.is_some() {
                to.enciphered += 1;
                let read = mount.read_entry_decrypted(&mut *file, entry, &self.keys);
                match read.map(|bytes| (kept(&bytes), bytes)) {
                    Ok((Kept::Program, bytes)) => {
                        to.bodies.push(Body {
                            archive: Some(label.to_owned()),
                            entry: Some(entry.clone()),
                            label: format!("{label}: {}", entry.display_name()),
                            bytes,
                        });
                    }
                    Ok((Kept::Other, _)) => {}
                    Ok((Kept::Unread, _)) | Err(_) => to.unread += 1,
                }
            } else if depth > 0 && mount.entry_begins_archive(file, entry).unwrap_or(false) {
                match mount.open_nested(file, entry, &self.keys) {
                    Ok(mut nested) => {
                        nested.archive.resolve_names(&self.resolver);
                        let label = format!("{label}/{}", entry.display_name());
                        self.walk(file, &nested, &label, depth - 1, to);
                    }
                    Err(_) => to.unread += 1,
                }
            }
        }
    }
}

/// What a deciphered entry is to the walk.
#[derive(Debug, PartialEq, Eq)]
enum Kept {
    Program,
    /// It starts as a program does and does not read as one: what its code names is not known.
    Unread,
    Other,
}

fn kept(bytes: &[u8]) -> Kept {
    match (bytes.starts_with(&[0; 8]), Program::read(bytes).is_ok()) {
        (true, true) => Kept::Program,
        (true, false) => Kept::Unread,
        (false, _) => Kept::Other,
    }
}

/// The bodies of `root`, and what it was read as: a build folder's archives, one archive, or a folder of deciphered programs.
// ponytail: every body is held until the export is done; read a body again in each pass if a
// build outgrows memory.
pub fn gathered(root: &Path, keys: Keys<'_>) -> Result<(&'static str, Found), String> {
    if !root.is_dir() {
        let head = File::open(root).and_then(head_of);
        return match head.map_err(|e| format!("read {}: {e}", root.display()))? {
            Head::Archive => {
                let found = Reader::new(keys)?.programs(root, &[], MAX_DEPTH, None)?;
                Ok(("an archive", found))
            }
            _ => Err(format!(
                "{} is neither a folder nor a GTA VI archive",
                root.display()
            )),
        };
    }
    let mut found = Found::default();
    let archives = build_archives(root);
    if archives.is_empty() {
        let (files, passed_over) = folder_files(root)?;
        found.passed_over = passed_over;
        for path in files {
            let bytes =
                std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
            found.bodies.push(Body {
                archive: None,
                entry: None,
                label: relative(root, &path),
                bytes,
            });
        }
        return Ok(("a folder of deciphered programs", found));
    }
    let reader = Reader::new(keys)?;
    for path in &archives {
        // In a build an archive goes by its path from the root: two packs hold files of one name.
        match reader.programs(path, &[], MAX_DEPTH, Some(relative(root, path))) {
            Ok(mut of_archive) => {
                found.bodies.append(&mut of_archive.bodies);
                found.enciphered += of_archive.enciphered;
                found.unread += of_archive.unread;
            }
            // Without content keys no archive reads: that is said, not counted an archive at a time.
            Err(why) if !reader.keys.has_content_keys() => return Err(why),
            Err(_) => found.unread += 1,
        }
    }
    Ok(("a build folder", found))
}

/// Every file under `folder` that starts with eight zero bytes, as a deciphered program does, in the order of their paths.
// A deciphered program has no extension to be found by.
pub fn program_files(folder: &Path) -> Result<Vec<PathBuf>, String> {
    folder_files(folder).map(|(found, _)| found)
}

/// The files [`program_files`] finds under `folder`, and how many files there start otherwise.
fn folder_files(folder: &Path) -> Result<(Vec<PathBuf>, usize), String> {
    let (mut found, mut others) = (Vec::new(), 0);
    let mut pending = vec![(folder.to_path_buf(), 0)];
    while let Some((dir, depth)) = pending.pop() {
        let entries =
            std::fs::read_dir(&dir).map_err(|e| format!("read {}: {e}", dir.display()))?;
        for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
            if path.is_dir() {
                if depth < MAX_DEPTH {
                    pending.push((path, depth + 1));
                }
            } else if File::open(&path).and_then(head_of).ok() == Some(Head::Program) {
                found.push(path);
            } else {
                others += 1;
            }
        }
    }
    found.sort();
    Ok((found, others))
}

fn name_of(path: &Path) -> String {
    let name = path.file_name().map(|name| name.to_string_lossy());
    name.map_or_else(|| path.display().to_string(), |name| name.into_owned())
}

/// `path` from `root`, with `/`.
fn relative(root: &Path, path: &Path) -> String {
    let from_root = path.strip_prefix(root).unwrap_or(path);
    from_root.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
#[path = "gta6_script_source_tests.rs"]
mod tests;
