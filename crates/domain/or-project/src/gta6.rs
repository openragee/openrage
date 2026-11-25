//! A GTA VI mod as a project: edits against an install's RPF9 archives, built into copies of the
//! archives they touch in a new output folder, which reverting deletes.
//!
//! ⚠ Every written archive is structural: the game refuses one whose table changed ([`STRUCTURAL`]).

use std::collections::BTreeMap;
use std::fmt;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

use or_io::container::rpf::gta6_chunk_table::cell_offset;
use or_io::container::rpf::gta6_container_write::Rpf9NewBody;
use or_io::container::rpf::{Gta6Entry, Gta6Mount, GtaViKeys, Rpf9ArchiveWriter, RPF9_MAGIC};
use or_io::gta6::hash::name_hash64;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The `format` a GTA VI project file states.
pub const GTA6_FORMAT: &str = "openrage-gta6-project";
/// The `format` of the manifest a build writes.
pub const GTA6_BUILD_FORMAT: &str = "openrage-gta6-build";
/// The manifest's file name inside the output folder.
pub const GTA6_MANIFEST: &str = "openrage-gta6-build.json";
/// What every build says of the archives it wrote.
pub const STRUCTURAL: &str =
    "Every archive here is a structural copy in a new folder, and the game \
refuses one whose table changed, because it checks a signature over each table at mount and only \
its maker can sign one.";

/// A GTA VI project: a name and the edits it makes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Gta6Project {
    pub name: String,
    pub edits: Vec<Gta6Edit>,
}

/// One edit of one entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Gta6Edit {
    /// The archive, relative to the install root.
    pub archive: PathBuf,
    /// The nested archives the entry is inside, outermost first, each named as `entry` is.
    #[serde(default)]
    pub within: Vec<String>,
    /// The entry: its path, or its name64 hash as `0x` and 16 hex digits; an added entry's path.
    pub entry: String,
    #[serde(flatten)]
    pub op: Gta6Op,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Gta6Op {
    /// The entry's body becomes the bytes of `with`; a relative path is the project file's folder's.
    Replace {
        with: PathBuf,
    },
    Remove,
    Rename {
        name: String,
    },
    /// A new entry of `with`'s bytes: a loose resource of `type`, else chunked or a nested archive.
    Add {
        with: PathBuf,
        #[serde(default, rename = "type")]
        type_code: Option<u32>,
    },
}

impl Gta6Op {
    fn word(&self) -> &'static str {
        match self {
            Self::Replace { .. } => "replace",
            Self::Remove => "remove",
            Self::Rename { .. } => "rename",
            Self::Add { .. } => "add",
        }
    }
}

/// Two edits of one entry, by the projects that make them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gta6Conflict {
    pub archive: PathBuf,
    pub within: Vec<String>,
    pub entry: String,
    /// The project that edits the entry first, then the one that edits it again.
    pub projects: [String; 2],
}

impl fmt::Display for Gta6Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b] = &self.projects;
        write!(
            f,
            "{a} and {b} both edit {} in {}",
            self.entry,
            self.archive.display()
        )?;
        for name in &self.within {
            write!(f, "/{name}")?;
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Gta6ProjectError {
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{}: not a GTA VI project: {why}", path.display())]
    NotAProject { path: PathBuf, why: String },
    #[error("{}", conflict_text(.0))]
    Conflicts(Vec<Gta6Conflict>),
    #[error("{}: the archive path must be relative to the install and stay inside it", .0.display())]
    OutsideInstall(PathBuf),
    #[error("{}: the output folder must not be inside the install, which is never written", .0.display())]
    OutputInInstall(PathBuf),
    #[error("{}: the output folder already exists; revert the last build by deleting it, then build again", .0.display())]
    OutputExists(PathBuf),
    #[error("{}: no build manifest there, so it is not a build output and is not deleted", .0.display())]
    NotABuild(PathBuf),
    #[error("{}: {why}", archive.display())]
    Archive { archive: PathBuf, why: String },
}

fn conflict_text(conflicts: &[Gta6Conflict]) -> String {
    let each: Vec<String> = conflicts.iter().map(ToString::to_string).collect();
    format!("conflicting edits: {}", each.join("; "))
}

fn io_at(path: &Path) -> impl FnOnce(std::io::Error) -> Gta6ProjectError + '_ {
    move |source| Gta6ProjectError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// What a build wrote: one change per edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Gta6Manifest {
    pub format: String,
    pub project: String,
    pub note: String,
    /// Tables the edits changed; each such table's signature is stale.
    pub tables_changed: usize,
    pub changes: Vec<Gta6Change>,
}

/// One edited entry: its archive, its name64 hash, and the SHA-256 of its content before and after.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Gta6Change {
    pub archive: PathBuf,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub within: Vec<String>,
    pub entry: String,
    pub op: String,
    /// Empty for an added entry.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub old: String,
    /// `None` for a removed entry.
    pub new: Option<String>,
    /// A renamed entry's new name64 hash.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub renamed_to: Option<String>,
}

impl Gta6Project {
    /// Read a project file, resolving each relative `with` against its folder.
    pub fn load(path: &Path) -> Result<Self, Gta6ProjectError> {
        let refused = |why: String| Gta6ProjectError::NotAProject {
            path: path.to_path_buf(),
            why,
        };
        let bytes = std::fs::read(path).map_err(io_at(path))?;
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| refused(e.to_string()))?;
        if value.get("format").and_then(serde_json::Value::as_str) != Some(GTA6_FORMAT) {
            return Err(refused(format!("its `format` is not {GTA6_FORMAT:?}")));
        }
        let mut project: Self =
            serde_json::from_value(value).map_err(|e| refused(e.to_string()))?;
        let folder = path.parent().unwrap_or(Path::new(""));
        for edit in &mut project.edits {
            if let Gta6Op::Replace { with } | Gta6Op::Add { with, .. } = &mut edit.op {
                if with.is_relative() {
                    *with = folder.join(&*with);
                }
            }
        }
        Ok(project)
    }
}

/// The name64 hash a selector names: `0x` and hex digits, else the hash of the path it is.
fn selector_hash(selector: &str) -> u64 {
    selector
        .strip_prefix("0x")
        .and_then(|hex| u64::from_str_radix(hex, 16).ok())
        .unwrap_or_else(|| name_hash64(selector.as_bytes()))
}

fn hash_text(hash: u64) -> String {
    format!("{hash:#018x}")
}

/// Every second edit of an entry an earlier edit already edits, across `projects` and within each.
pub fn conflicts(projects: &[Gta6Project]) -> Vec<Gta6Conflict> {
    let mut first: BTreeMap<(String, Vec<u64>, u64), &str> = BTreeMap::new();
    let mut found = Vec::new();
    for project in projects {
        for edit in &project.edits {
            let archive = archive_key(&edit.archive);
            let within = edit.within.iter().map(|w| selector_hash(w)).collect();
            let key = (archive, within, selector_hash(&edit.entry));
            match first.get(&key) {
                Some(&owner) => found.push(Gta6Conflict {
                    archive: edit.archive.clone(),
                    within: edit.within.clone(),
                    entry: edit.entry.clone(),
                    projects: [owner.to_owned(), project.name.clone()],
                }),
                None => {
                    first.insert(key, &project.name);
                }
            }
        }
    }
    found
}

/// Apply `project` to copies of the archives it touches under `install`, written into `out`, a
/// folder that must not exist yet; nothing under `install` is written.
pub fn build(
    project: &Gta6Project,
    install: &Path,
    out: &Path,
    keys: &GtaViKeys,
) -> Result<Gta6Manifest, Gta6ProjectError> {
    let clashes = conflicts(std::slice::from_ref(project));
    if !clashes.is_empty() {
        return Err(Gta6ProjectError::Conflicts(clashes));
    }
    // Keyed as `conflicts` keys an archive, so two spellings of one archive share one writer.
    let mut by_archive: BTreeMap<String, (&Path, Vec<&Gta6Edit>)> = BTreeMap::new();
    for edit in &project.edits {
        let inside = !edit.archive.as_os_str().is_empty()
            && edit
                .archive
                .components()
                .all(|c| matches!(c, Component::Normal(_)));
        if !inside {
            return Err(Gta6ProjectError::OutsideInstall(edit.archive.clone()));
        }
        by_archive
            .entry(archive_key(&edit.archive))
            .or_insert_with(|| (edit.archive.as_path(), Vec::new()))
            .1
            .push(edit);
    }
    let install_at = install.canonicalize().map_err(io_at(install))?;
    if is_within(&resolved(out).map_err(io_at(out))?, &install_at) {
        return Err(Gta6ProjectError::OutputInInstall(out.to_path_buf()));
    }
    if out.exists() {
        return Err(Gta6ProjectError::OutputExists(out.to_path_buf()));
    }
    std::fs::create_dir_all(out).map_err(io_at(out))?;
    let built = write_all(project, &by_archive, install, out, keys);
    if built.is_err() {
        // The folder did not exist before this build, so everything in it is this build's.
        let _ = std::fs::remove_dir_all(out);
    }
    built
}

fn archive_key(archive: &Path) -> String {
    archive.to_string_lossy().replace('\\', "/").to_lowercase()
}

/// `path` made absolute with `..` folded, its deepest existing ancestor resolved through links and
/// junctions, and the components below that ancestor appended.
fn resolved(path: &Path) -> std::io::Result<PathBuf> {
    let mut folded = PathBuf::new();
    for c in std::path::absolute(path)?.components() {
        match c {
            Component::ParentDir => {
                folded.pop();
            }
            Component::CurDir => {}
            other => folded.push(other),
        }
    }
    let mut below = Vec::new();
    let mut at = folded.as_path();
    loop {
        if let Ok(real) = at.canonicalize() {
            return Ok(below.iter().rev().fold(real, |p, c| p.join(c)));
        }
        match (at.file_name(), at.parent()) {
            (Some(name), Some(parent)) => {
                below.push(name.to_os_string());
                at = parent;
            }
            _ => return Ok(folded),
        }
    }
}

/// Whether `path` is `root` or below it, ignoring case on Windows, where the file system does.
fn is_within(path: &Path, root: &Path) -> bool {
    let fold = |p: &Path| -> Vec<String> {
        p.components()
            .map(|c| {
                let s = c.as_os_str().to_string_lossy().into_owned();
                if cfg!(windows) {
                    s.to_lowercase()
                } else {
                    s
                }
            })
            .collect()
    };
    fold(path).starts_with(&fold(root))
}

fn write_all(
    project: &Gta6Project,
    by_archive: &BTreeMap<String, (&Path, Vec<&Gta6Edit>)>,
    install: &Path,
    out: &Path,
    keys: &GtaViKeys,
) -> Result<Gta6Manifest, Gta6ProjectError> {
    let mut manifest = Gta6Manifest {
        format: GTA6_BUILD_FORMAT.to_owned(),
        project: project.name.clone(),
        note: STRUCTURAL.to_owned(),
        tables_changed: 0,
        changes: Vec::new(),
    };
    for &(archive, ref edits) in by_archive.values() {
        let failed = |why: String| Gta6ProjectError::Archive {
            archive: archive.to_path_buf(),
            why,
        };
        let source = install.join(archive);
        let mut src = File::open(&source).map_err(io_at(&source))?;
        let root = Gta6Mount::open_root(&mut src, keys).map_err(|e| failed(e.to_string()))?;
        let mut writer = Rpf9ArchiveWriter::new();
        for edit in edits {
            if let Gta6Op::Add { with, type_code } = &edit.op {
                let (_, path) = within(&mut src, &root, &edit.within, keys).map_err(failed)?;
                let name = &edit.entry;
                if selector_hash(name) != name_hash64(name.as_bytes()) {
                    return Err(failed(format!(
                        "{name:?}: an added entry is named by its path, not a hash"
                    )));
                }
                let bytes = std::fs::read(with).map_err(io_at(with))?;
                let new = digest(&bytes);
                let body = match type_code {
                    Some(code) => Rpf9NewBody::Resource {
                        type_code: *code,
                        loose: bytes,
                    },
                    None if bytes.starts_with(&RPF9_MAGIC.to_le_bytes()) => {
                        Rpf9NewBody::Archive(bytes)
                    }
                    None => Rpf9NewBody::Chunked(bytes),
                };
                writer.add(&path, name, body);
                manifest.changes.push(Gta6Change {
                    archive: archive.to_path_buf(),
                    within: edit.within.clone(),
                    entry: hash_text(name_hash64(name.as_bytes())),
                    op: edit.op.word().to_owned(),
                    old: String::new(),
                    new: Some(new),
                    renamed_to: None,
                });
                continue;
            }
            let (mount, entry, path) = locate(&mut src, &root, edit, keys).map_err(failed)?;
            let old = digest(&content(&mount, &mut src, &entry, keys).map_err(failed)?);
            let (new, renamed_to) = match &edit.op {
                Gta6Op::Replace { with } => {
                    let body = std::fs::read(with).map_err(io_at(with))?;
                    let new = digest(&body);
                    // A 20-byte row holds a headerless resource, which only its loose form replaces.
                    let headerless = (mount.archive.entry_extra(entry.index as usize))
                        .is_some_and(|x| x.nested.is_some());
                    if headerless {
                        writer.replace_resource(&path, body, None);
                    } else {
                        writer.replace(&path, body);
                    }
                    (Some(new), None)
                }
                Gta6Op::Remove => {
                    writer.remove(&path);
                    (None, None)
                }
                Gta6Op::Rename { name } => {
                    writer.rename(&path, name);
                    (
                        Some(old.clone()),
                        Some(hash_text(name_hash64(name.as_bytes()))),
                    )
                }
                // Written above, as it names no entry to locate.
                Gta6Op::Add { .. } => continue,
            };
            manifest.changes.push(Gta6Change {
                archive: archive.to_path_buf(),
                within: edit.within.clone(),
                entry: hash_text(entry.name64),
                op: edit.op.word().to_owned(),
                old,
                new,
                renamed_to,
            });
        }
        let plan = writer
            .plan(&mut src, keys)
            .map_err(|e| failed(e.to_string()))?;
        manifest.tables_changed += plan.tables_changed;
        let target = out.join(archive);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(io_at(parent))?;
        }
        let created = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(io_at(&target))?;
        let mut sink = BufWriter::new(created);
        plan.write_to(&mut src, &mut sink)
            .map_err(|e| failed(e.to_string()))?;
        sink.flush().map_err(io_at(&target))?;
    }
    let mut text = serde_json::to_string_pretty(&manifest).expect("a manifest serialises");
    text.push('\n');
    let at = out.join(GTA6_MANIFEST);
    std::fs::write(&at, text).map_err(io_at(&at))?;
    Ok(manifest)
}

/// Delete a build's output folder; refused unless it holds a build manifest.
pub fn revert(out: &Path) -> Result<(), Gta6ProjectError> {
    if !out.join(GTA6_MANIFEST).is_file() {
        return Err(Gta6ProjectError::NotABuild(out.to_path_buf()));
    }
    std::fs::remove_dir_all(out).map_err(io_at(out))
}

/// The mount holding `edit`'s entry, the entry, and its entry indices from the top-level table.
fn locate(
    src: &mut File,
    root: &Gta6Mount,
    edit: &Gta6Edit,
    keys: &GtaViKeys,
) -> Result<(Gta6Mount, Gta6Entry, Vec<u32>), String> {
    let (mount, mut path) = within(src, root, &edit.within, keys)?;
    let entry = find(&mount, &edit.entry)?;
    path.push(entry.index);
    Ok((mount, entry, path))
}

/// The nested archive the names `nesting` walk to from `root`, and its entry indices from there.
fn within(
    src: &mut File,
    root: &Gta6Mount,
    nesting: &[String],
    keys: &GtaViKeys,
) -> Result<(Gta6Mount, Vec<u32>), String> {
    let mut mount = root.clone();
    let mut path = Vec::with_capacity(nesting.len() + 1);
    for name in nesting {
        let entry = find(&mount, name)?;
        if !mount
            .entry_begins_archive(src, &entry)
            .map_err(|e| e.to_string())?
        {
            return Err(format!("{name:?} is an entry, not a nested archive"));
        }
        path.push(entry.index);
        mount = mount
            .open_nested(src, &entry, keys)
            .map_err(|e| e.to_string())?;
    }
    Ok((mount, path))
}

fn find(mount: &Gta6Mount, selector: &str) -> Result<Gta6Entry, String> {
    let hash = selector_hash(selector);
    mount
        .archive
        .entries
        .iter()
        .find(|e| e.name64 == hash)
        .cloned()
        .ok_or_else(|| format!("no entry {selector:?}"))
}

/// The entry's content as the writer compares it: inflated and deciphered where it is chunked,
/// enciphered or sized by the reader, else the `real` bytes at its start.
fn content(
    mount: &Gta6Mount,
    src: &mut File,
    entry: &Gta6Entry,
    keys: &GtaViKeys,
) -> Result<Vec<u8>, String> {
    let chunked = cell_offset(&mount.archive, entry.index as usize).is_some();
    let (Some(at), Some(real), false, None) = (
        mount.absolute_start(entry),
        entry.real,
        chunked,
        entry.cipher,
    ) else {
        return mount
            .read_entry_decrypted(&mut *src, entry, keys)
            .map_err(|e| e.to_string());
    };
    let len = src.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
    if at.checked_add(real).is_none_or(|end| end > len) {
        return Err(format!(
            "entry {} states {real} bytes at {at:#x}, past the archive's end",
            hash_text(entry.name64)
        ));
    }
    let mut body = vec![0u8; real as usize];
    src.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
    src.read_exact(&mut body).map_err(|e| e.to_string())?;
    Ok(body)
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_output_reached_by_dots_or_another_case_is_inside_the_install() {
        let install = tempfile::tempdir().expect("an install");
        let root = install.path().canonicalize().expect("canonical");
        let dotted = install.path().join("a").join("..").join("out");
        assert!(is_within(&resolved(&dotted).expect("resolved"), &root));
        let beside = install.path().join("..").join("elsewhere");
        assert!(!is_within(&resolved(&beside).expect("resolved"), &root));
        if cfg!(windows) {
            let upper = PathBuf::from(install.path().to_string_lossy().to_uppercase()).join("m");
            assert!(is_within(&resolved(&upper).expect("resolved"), &root));
        }
    }
}
