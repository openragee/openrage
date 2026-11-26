//! Importing a game file: its bytes become a source folder under `sources/`, or a loose file under `files/`.
//!
//! ⚠ **An import only adds.** It never writes into a folder that exists, never changes the file it
//! reads, and builds a source in a hidden staging folder that it renames into place only when the
//! extraction has succeeded, so a failure leaves nothing behind. What it made is reported with a
//! digest, so that a caller can take back exactly that while nobody has touched it.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::asset::{
    folder_name, new_id, parse_folder_name, sha256_hex, unsafe_file_name, AssetManifest, How,
    Source, MANIFEST,
};
use crate::{bounds, cook, fragment, git, metadata, models, particles, paths, textures, Problem};

/// Where an import writes its sources, beside the project file.
pub const SOURCES_DIR: &str = "sources";
/// Where an import keeps a compiled file that has no source form yet.
pub const FILES_DIR: &str = "files";
/// The folders of a project a walk leaves alone, as `or-project` names them.
pub const BUILD_DIR: &str = "build";
pub const SHADOW_DIR: &str = "vfs";
/// The extension of a project file, which a walk never imports.
pub const PROJECT_EXTENSION: &str = "openrage-proj";
/// How many of the files or folders a bound leaves out are listed one by one.
const MAX_LISTED: usize = 1_000;
/// How many folders of a project are read to find the ids its sources hold.
const MAX_PROJECT_FOLDERS: usize = 5_000;

/// The bones a ped component's skin is bound to, which are in another file than the component: the ped's own.
#[derive(Debug, Clone, Default)]
pub struct Skeleton(Option<std::sync::Arc<Vec<or_interchange::Bone>>>);

impl Skeleton {
    pub fn of(bones: Vec<or_interchange::Bone>) -> Self {
        Self(Some(std::sync::Arc::new(bones)))
    }

    pub fn bones(&self) -> Option<&[or_interchange::Bone]> {
        self.0.as_deref().map(Vec::as_slice)
    }
}

/// Two skeletons are equal where they are the same bones read once, which is all a report needs to know.
impl PartialEq for Skeleton {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (None, None) => true,
            (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Eq for Skeleton {}

/// What the caller knows of where a file came from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    pub from_game: bool,
    /// The file's path inside the game's archives, when it came out of one.
    pub virtual_path: Option<String>,
    pub game_build: Option<String>,
    /// The skeleton of the ped a ped component is for, so that its glb has its skin; none for a file that is not one.
    pub skeleton: Skeleton,
}

/// A thing to import.
#[derive(Debug, Clone)]
pub enum Input<'a> {
    /// A file already in memory, such as an archive entry, under the name it has in the game.
    Bytes {
        name: &'a str,
        bytes: &'a [u8],
        facts: Facts,
    },
    /// A file, or a folder walked recursively. A path on disk has no archive path, so `virtual_path` is not read.
    Path { path: &'a Path, facts: Facts },
}

/// How far a walk of a folder goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    /// How many levels of folders below the walked one are read.
    pub depth: usize,
    /// How many files one walk takes.
    pub files: usize,
}

impl Default for Bounds {
    fn default() -> Self {
        Self {
            depth: 8,
            files: 10_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Copy a file with no source form under `files/`; off, it is only reported.
    pub keep: bool,
    pub bounds: Bounds,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            keep: true,
            bounds: Bounds::default(),
        }
    }
}

/// What became of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Made { folder: PathBuf },
    AlreadyASource { folder: PathBuf },
    Kept { copied_to: Option<PathBuf> },
    Refused,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReport {
    pub input: String,
    pub verdict: Verdict,
    /// One sentence a person can read.
    pub reason: String,
}

/// A path the import made, and the digest it had when it was made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    pub path: PathBuf,
    pub digest: String,
}

/// Something the import left out: past a bound of the walk, an archive, or not reached before it was stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeftOut {
    /// Inside the walked folder, with `/` on every host.
    pub path: String,
    pub why: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub files: Vec<FileReport>,
    pub created: Vec<Created>,
    pub left_out: Vec<LeftOut>,
    /// What else a person should be told: a bound that cut a walk, a file that could not be updated.
    pub notes: Vec<String>,
    /// The bounds a folder was walked within, none when no folder was walked.
    pub bounds: Option<Bounds>,
    /// Stopped before the last file: what it made stays, and `left_out` lists the files it did not reach.
    pub cancelled: bool,
}

/// Told how many files are done, how many there are in all, and the name of the file about to be done.
pub type Say<'a> = &'a dyn Fn(usize, usize, &str);

/// What a caller sees and says while an import runs.
#[derive(Default)]
pub struct Watch<'a> {
    /// Asked before each file; once it says true the import stops and keeps what it made.
    pub cancelled: Option<&'a dyn Fn() -> bool>,
    /// Told before each file.
    pub progress: Option<Say<'a>>,
}

type Extract = fn(&[u8], &str, &Path, &Facts) -> Result<PathBuf, Vec<Problem>>;

/// One row of the kind table: a game extension, the kind of source it becomes, and how.
pub struct Row {
    pub extension: &'static str,
    pub kind: &'static str,
    extract: Extract,
    /// A file the extraction refuses is kept as a compiled file with the reason, where it was kept before this kind existed; off, it is refused.
    keeps_what_it_cannot: bool,
}

/// Every game format that becomes a source. Any other extension is kept as it is.
pub const KINDS: &[Row] = &[
    Row {
        extension: "ytd",
        kind: textures::KIND,
        extract: |bytes, name, parent, facts| {
            textures::extract(
                bytes,
                name,
                parent,
                &textures::Extraction {
                    from_game: facts.from_game,
                    virtual_path: facts.virtual_path.as_deref(),
                },
            )
        },
        keeps_what_it_cannot: false,
    },
    Row {
        extension: "ydr",
        kind: models::DRAWABLE,
        extract: |bytes, name, parent, facts| {
            models::extract_drawable(bytes, name, parent, &models_how(facts))
        },
        keeps_what_it_cannot: false,
    },
    Row {
        extension: "ydd",
        kind: models::DRAWABLES,
        extract: |bytes, name, parent, facts| {
            models::extract_drawables(bytes, name, parent, &models_how(facts))
        },
        keeps_what_it_cannot: false,
    },
    Row {
        extension: "yft",
        kind: fragment::KIND,
        extract: |bytes, name, parent, facts| {
            fragment::extract(bytes, name, parent, &models_how(facts))
        },
        keeps_what_it_cannot: true,
    },
    Row {
        extension: "ymt",
        kind: metadata::KIND,
        extract: |bytes, name, parent, facts| metadata_source(bytes, name, "ymt", parent, facts),
        keeps_what_it_cannot: true,
    },
    Row {
        extension: "ymap",
        kind: metadata::KIND,
        extract: |bytes, name, parent, facts| metadata_source(bytes, name, "ymap", parent, facts),
        keeps_what_it_cannot: true,
    },
    Row {
        extension: "ytyp",
        kind: metadata::KIND,
        extract: |bytes, name, parent, facts| metadata_source(bytes, name, "ytyp", parent, facts),
        keeps_what_it_cannot: true,
    },
    Row {
        extension: "ypt",
        kind: particles::KIND,
        extract: |bytes, name, parent, facts| {
            particles::extract(
                bytes,
                name,
                parent,
                &textures::Extraction {
                    from_game: facts.from_game,
                    virtual_path: facts.virtual_path.as_deref(),
                },
            )
        },
        keeps_what_it_cannot: false,
    },
    Row {
        extension: "ybn",
        kind: bounds::KIND,
        extract: |bytes, name, parent, facts| {
            bounds::extract(
                bytes,
                name,
                parent,
                &textures::Extraction {
                    from_game: facts.from_game,
                    virtual_path: facts.virtual_path.as_deref(),
                },
            )
        },
        keeps_what_it_cannot: true,
    },
    Row {
        extension: "ynd",
        kind: paths::KIND,
        extract: |bytes, name, parent, facts| {
            paths::extract(
                bytes,
                name,
                parent,
                &textures::Extraction {
                    from_game: facts.from_game,
                    virtual_path: facts.virtual_path.as_deref(),
                },
            )
        },
        keeps_what_it_cannot: true,
    },
];

/// A metadata file as a source.
fn metadata_source(
    bytes: &[u8],
    name: &str,
    extension: &str,
    parent: &Path,
    facts: &Facts,
) -> Result<PathBuf, Vec<Problem>> {
    metadata::extract(bytes, name, extension, parent, &models_how(facts))
}

/// Whether the extraction of a source of `kind` binds a skin to the skeleton it is given: a model does, and a texture or a
/// metadata file has no skin to bind.
fn takes_a_skeleton(kind: &str) -> bool {
    matches!(kind, models::DRAWABLE | models::DRAWABLES | fragment::KIND)
}

fn models_how(facts: &Facts) -> models::Extraction<'_> {
    models::Extraction {
        from_game: facts.from_game,
        virtual_path: facts.virtual_path.as_deref(),
        textures: false,
        skeleton: facts.skeleton.bones(),
    }
}

/// The row for a game extension, in any case.
pub fn kind_of(extension: &str) -> Option<&'static Row> {
    KINDS
        .iter()
        .find(|row| row.extension.eq_ignore_ascii_case(extension))
}

/// Decompiles the game file `name` whose bytes are `bytes` into a source folder inside `parent`, and says where.
///
/// This is the one rule of the door: every file with a source form becomes a source, whichever door it came in by. A file with
/// none, or one the reader of its format refuses, is an error with the reason, and nothing is left behind.
pub fn decompile(
    bytes: &[u8],
    name: &str,
    parent: &Path,
    facts: &Facts,
) -> Result<PathBuf, String> {
    let (stem, extension) = name.rsplit_once('.').unwrap_or((name, ""));
    let row = kind_of(extension)
        .ok_or_else(|| format!("a .{extension} file has no source form to decompile into"))?;
    decompile_with(row, stem, bytes, parent, facts)
}

fn decompile_with(
    row: &Row,
    stem: &str,
    bytes: &[u8],
    parent: &Path,
    facts: &Facts,
) -> Result<PathBuf, String> {
    let said = |problems: Vec<Problem>| {
        problems
            .iter()
            .map(Problem::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    };
    // A skeleton the skin cannot be bound to is no reason to refuse the file: it comes in as it did, and its source says why.
    let skeletal = takes_a_skeleton(row.kind) && facts.skeleton.bones().is_some();
    let folder = parent.join(folder_name(stem, row.kind));
    let in_the_way = folder.exists();
    let (extracted, skeleton) = match (row.extract)(bytes, stem, parent, facts) {
        Ok(folder) => (folder, skeletal.then(|| "embedded".to_string())),
        Err(first) if skeletal => {
            // ⚠ Only a folder this call made can be taken out of the way: one that was there is somebody's source.
            if !in_the_way {
                let _ = std::fs::remove_dir_all(&folder);
            }
            let plain = Facts {
                skeleton: Skeleton::default(),
                ..facts.clone()
            };
            match (row.extract)(bytes, stem, parent, &plain) {
                Ok(folder) => (folder, Some(format!("not embedded: {}", said(first)))),
                Err(_) => return Err(said(first)),
            }
        }
        Err(problems) => return Err(said(problems)),
    };
    let mut source = Source::open(&extracted).map_err(|p| p.to_string())?;
    if let Some(skeleton) = skeleton {
        source
            .manifest
            .origin
            .extra
            .insert("reference_skeleton".into(), skeleton.into());
    }
    source.manifest.origin.how = if facts.from_game {
        How::Extracted
    } else {
        How::Imported
    };
    source.manifest.origin.game_build = facts.game_build.clone();
    source.write_manifest().map_err(|p| p.to_string())?;
    // ⚠ A cook writes OpenRage's form of the game's file, not its bytes, so a build can tell
    // the file is still the game's only by the hash of this first cook. One that fails records
    // nothing and fails again, with its reason, when a build cooks the source.
    if facts.from_game {
        if let Ok(cooked) = cook::cooked_sha256(&extracted) {
            source.manifest.origin.cooked_sha256 = Some(cooked);
            source.write_manifest().map_err(|p| p.to_string())?;
        }
    }
    Ok(extracted)
}

/// Imports `inputs` into the project whose file is in `project_root`, and says what became of each.
pub fn import(project_root: &Path, inputs: &[Input<'_>], options: &Options) -> Report {
    import_watched(project_root, inputs, options, &Watch::default())
}

/// As [`import`], telling `watch` how far it has got and stopping, keeping what it made, when `watch` says so.
pub fn import_watched(
    project_root: &Path,
    inputs: &[Input<'_>],
    options: &Options,
    watch: &Watch<'_>,
) -> Report {
    let sources = project_root.join(SOURCES_DIR);
    let mut run = Run {
        index: Index::read(project_root, &sources),
        sources,
        files: project_root.join(FILES_DIR),
        options,
        report: Report::default(),
    };
    // ⚠ Every file is found before the first is made, so that progress has its total and a stop can list the rest.
    let todo = run.expand(inputs);
    for (done, item) in todo.iter().enumerate() {
        if watch.cancelled.is_some_and(|stop| stop()) {
            run.stopped(&todo[done..], done);
            break;
        }
        if let Some(say) = watch.progress {
            say(done, todo.len(), item.name());
        }
        run.item(item);
    }
    if let Some(note) = kept_note(&run.report.files) {
        run.report.notes.push(note);
    }
    let made = run
        .report
        .files
        .iter()
        .any(|f| matches!(f.verdict, Verdict::Made { .. }));
    if made {
        if let Err(e) = git::update(project_root) {
            run.report.notes.push(format!(
                "the ignore rules of the project are not updated: {e}"
            ));
        }
    }
    run.report
}

/// How the note that names the kinds of file an import kept begins.
pub const KEPT_NOTE: &str = "Kept as compiled files, not sources: ";

/// Names the kinds of file an import kept as compiled files, so a report says which formats still have no source: `Kept as compiled files, not sources: 2 .ycd, 1 .yft`.
fn kept_note(files: &[FileReport]) -> Option<String> {
    let mut kinds: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for file in files
        .iter()
        .filter(|f| matches!(f.verdict, Verdict::Kept { .. }))
    {
        let name = last_component(&file.input);
        let kind = match name.rsplit_once('.') {
            Some((_, extension)) => format!(".{}", extension.to_ascii_lowercase()),
            None => "no extension".to_string(),
        };
        *kinds.entry(kind).or_default() += 1;
    }
    if kinds.is_empty() {
        return None;
    }
    let listed: Vec<String> = kinds
        .iter()
        .map(|(kind, n)| format!("{n} {kind}"))
        .collect();
    Some(format!("{KEPT_NOTE}{}", listed.join(", ")))
}

/// One file to import, found by [`Run::expand`].
enum Todo<'i> {
    Bytes {
        name: &'i str,
        bytes: &'i [u8],
        facts: &'i Facts,
    },
    /// A file on disk, read when its turn comes; `keep_as` is its path below the walked folder.
    Disk {
        shown: String,
        name: String,
        keep_as: String,
        path: PathBuf,
        facts: Facts,
    },
    /// A source folder, taken as it is.
    Source {
        shown: String,
        folder: PathBuf,
    },
    Refuse {
        shown: String,
        why: String,
    },
}

impl Todo<'_> {
    /// The file's own name.
    fn name(&self) -> &str {
        match self {
            Self::Bytes { name, .. } => name,
            Self::Disk { name, .. } => name,
            Self::Source { folder, .. } => folder
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default(),
            Self::Refuse { shown, .. } => last_component(shown),
        }
    }

    /// Where a report places the file: below the walked folder when it was in one.
    fn place(&self) -> &str {
        match self {
            Self::Bytes { name, .. } => name,
            Self::Disk { keep_as, .. } => keep_as,
            Self::Source { shown, .. } | Self::Refuse { shown, .. } => shown,
        }
    }
}

struct Run<'a> {
    sources: PathBuf,
    files: PathBuf,
    options: &'a Options,
    index: Index,
    report: Report,
}

impl Run<'_> {
    fn push(&mut self, input: String, verdict: Verdict, reason: String) {
        self.report.files.push(FileReport {
            input,
            verdict,
            reason,
        });
    }

    fn refuse(&mut self, input: String, reason: impl Into<String>) {
        self.push(input, Verdict::Refused, reason.into());
    }

    /// Every file `inputs` name, a folder walked, in the order they are imported.
    fn expand<'i>(&mut self, inputs: &'i [Input<'_>]) -> Vec<Todo<'i>> {
        let mut todo = Vec::new();
        for input in inputs {
            match input {
                Input::Bytes { name, bytes, facts } => todo.push(Todo::Bytes {
                    name: last_component(name),
                    bytes,
                    facts,
                }),
                Input::Path { path, facts } => self.expand_path(path, facts, &mut todo),
            }
        }
        todo
    }

    fn expand_path(&mut self, path: &Path, facts: &Facts, todo: &mut Vec<Todo<'_>>) {
        let facts = Facts {
            virtual_path: None,
            ..facts.clone()
        };
        let shown = path.display().to_string();
        let refuse = |why: &str| Todo::Refuse {
            shown: shown.clone(),
            why: why.to_string(),
        };
        match std::fs::metadata(path) {
            Err(e) => todo.push(refuse(&format!("cannot be read: {e}"))),
            Ok(meta) if meta.is_dir() && path.join(MANIFEST).is_file() => todo.push(Todo::Source {
                shown: shown.clone(),
                folder: path.to_path_buf(),
            }),
            Ok(meta) if meta.is_dir() => self.expand_folder(path, &facts, todo),
            // An `asset.json` names the source it is the marker of.
            Ok(_)
                if path
                    .file_name()
                    .is_some_and(|n| n.eq_ignore_ascii_case(MANIFEST)) =>
            {
                match path.parent().filter(|p| !p.as_os_str().is_empty()) {
                    Some(folder) => todo.push(Todo::Source {
                        shown: shown.clone(),
                        folder: folder.to_path_buf(),
                    }),
                    None => todo.push(refuse("is an asset.json with no folder around it")),
                }
            }
            Ok(_) => {
                let name = path
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                todo.push(Todo::Disk {
                    shown,
                    keep_as: name.clone(),
                    name,
                    path: path.to_path_buf(),
                    facts,
                });
            }
        }
    }

    fn item(&mut self, item: &Todo<'_>) {
        match item {
            Todo::Bytes { name, bytes, facts } => {
                self.file(name.to_string(), name, name, bytes, facts)
            }
            Todo::Disk {
                shown,
                name,
                keep_as,
                path,
                facts,
            } => match std::fs::read(path) {
                Ok(bytes) => self.file(shown.clone(), name, keep_as, &bytes, facts),
                Err(e) => self.refuse(shown.clone(), format!("cannot be read: {e}")),
            },
            Todo::Source { shown, folder } => self.source_as_it_is(shown.clone(), folder),
            Todo::Refuse { shown, why } => self.refuse(shown.clone(), why.as_str()),
        }
    }

    /// Records that the import stopped with `rest` not reached after `done` files, and lists the first [`MAX_LISTED`] of them.
    fn stopped(&mut self, rest: &[Todo<'_>], done: usize) {
        let listed = rest.len().min(MAX_LISTED);
        self.report.cancelled = true;
        self.report.notes.push(format!(
            "Stopped after {done} of {} file(s): the {} not reached were not imported; the first {listed} are listed",
            done + rest.len(),
            rest.len()
        ));
        self.report
            .left_out
            .extend(rest[..listed].iter().map(|item| LeftOut {
                path: item.place().to_string(),
                why: "was not reached before the import was stopped".to_string(),
            }));
    }

    fn expand_folder(&mut self, folder: &Path, facts: &Facts, todo: &mut Vec<Todo<'_>>) {
        let bounds = self.options.bounds;
        self.report.bounds = Some(bounds);
        let mut walked = walk(folder, &bounds, &[&self.files, &self.sources]);
        let found_sources = std::mem::take(&mut walked.sources);
        let over = if walked.files.len() > bounds.files {
            walked.files.split_off(bounds.files)
        } else {
            Vec::new()
        };
        self.leave_out(
            folder,
            &walked.too_deep,
            "folder",
            format!("more than {} levels down", bounds.depth),
            "it is deeper than an import reads",
        );
        self.leave_out(
            folder,
            &over,
            "file",
            format!("past the {} an import takes at once", bounds.files),
            "it is past the files an import takes at once",
        );
        // ⚠ An archive is gigabytes and `files/` keeps a file whole, so it is listed and not copied: the way in is its files.
        let (archives, files): (Vec<PathBuf>, Vec<PathBuf>) =
            walked.files.into_iter().partition(|file| is_archive(file));
        self.leave_out(
            folder,
            &archives,
            "archive",
            "in the folder".to_string(),
            "is an archive, which an import does not keep whole; open it and import the files in it",
        );
        // ⚠ A source the walk met is taken whole and first: its files are its own, never game files of the walk.
        for source in found_sources {
            todo.push(Todo::Source {
                shown: source.display().to_string(),
                folder: source,
            });
        }
        for file in files {
            let relative = relative_to(&file, folder);
            todo.push(Todo::Disk {
                shown: file.display().to_string(),
                name: last_component(&relative).to_string(),
                keep_as: relative,
                path: file,
                facts: facts.clone(),
            });
        }
    }

    /// Records `cut` as left out of the walk of `folder`: a note with the count, and the first [`MAX_LISTED`] by path.
    fn leave_out(&mut self, folder: &Path, cut: &[PathBuf], what: &str, place: String, why: &str) {
        if cut.is_empty() {
            return;
        }
        let listed = cut.len().min(MAX_LISTED);
        self.report.notes.push(format!(
            "{} {what}(s) {place} are not imported; the first {listed} are listed",
            cut.len()
        ));
        self.report
            .left_out
            .extend(cut[..listed].iter().map(|path| LeftOut {
                path: relative_to(path, folder),
                why: why.to_string(),
            }));
    }

    /// One file named `name` whose bytes are `bytes`; `keep_as` is where it goes under `files/` when it has no source form.
    fn file(&mut self, input: String, name: &str, keep_as: &str, bytes: &[u8], facts: &Facts) {
        let (stem, extension) = name.rsplit_once('.').unwrap_or((name, ""));
        match kind_of(extension) {
            Some(row) => {
                if let Err(why) = self.source(input.clone(), row, stem, bytes, facts) {
                    if row.keeps_what_it_cannot {
                        self.keep(input, extension, keep_as, bytes, Some(&why));
                    } else {
                        self.refuse(input, why);
                    }
                }
            }
            None => self.keep(input, extension, keep_as, bytes, None),
        }
    }

    /// Makes the source of `bytes`, or says why the extraction could not; a refusal is the caller's to report.
    fn source(
        &mut self,
        input: String,
        row: &Row,
        stem: &str,
        bytes: &[u8],
        facts: &Facts,
    ) -> Result<(), String> {
        let sha = sha256_hex(bytes);
        if let Some(folder) = self.index.same_origin(row.kind, &sha, facts) {
            let folder = folder.clone();
            let reason = format!(
                "is already {} in the project: the same {} and SHA-256",
                self.shown(&folder),
                if facts.virtual_path.is_some() {
                    "archive path"
                } else {
                    "file"
                }
            );
            self.push(input, Verdict::AlreadyASource { folder }, reason);
            return Ok(());
        }
        let sources_were_there = self.sources.is_dir();
        let stage = self
            .sources
            .join(format!(".import-{}", new_id().unwrap_or_default()));
        let outcome = match std::fs::create_dir_all(&stage) {
            Ok(()) => self.extract_into(&stage, row, stem, bytes, facts, &sha),
            Err(e) => Err(format!("cannot make the staging folder: {e}")),
        };
        let _ = std::fs::remove_dir_all(&stage);
        if !sources_were_there {
            // Only an empty folder goes, so a source that was made stays.
            let _ = std::fs::remove_dir(&self.sources);
        }
        let folder = outcome?;
        let numbered = folder
            .file_name()
            .is_some_and(|n| n.to_string_lossy() != folder_name(stem, row.kind));
        let mut reason = if numbered {
            format!(
                "became {}; the name {stem} is taken there by a file with another origin",
                self.shown(&folder)
            )
        } else {
            format!("became {}", self.shown(&folder))
        };
        // ⚠ A compile writes the canonical form, so a person is told when the first export is not the file they gave.
        if row.kind == metadata::KIND {
            match metadata::differs_from_the_original(&folder) {
                Some(true) => reason.push_str(
                    "; its XML is OpenRage's canonical form, so the first export differs from the original's bytes",
                ),
                Some(false) => reason.push_str("; its first export is the original's bytes"),
                None => {}
            }
        }
        self.push(input, Verdict::Made { folder }, reason);
        Ok(())
    }

    /// A source folder taken as it is: copied whole under `sources/` at the free name of its kind and name, keeping its id.
    ///
    /// ⚠ One asset has one id. The folder of the project that holds the id already is reported as already a source when it is
    /// the same tree, and the incoming copy of an edited one is given an id of its own, since two sources of one id cook into one file.
    fn source_as_it_is(&mut self, input: String, folder: &Path) {
        let source = match Source::open(folder) {
            Ok(source) => source,
            Err(problem) => return self.refuse(input, problem.to_string()),
        };
        let (kind, name) = (source.manifest.kind.clone(), source.manifest.name.clone());
        let held = self.index.ids.get(&source.manifest.id).cloned();
        let inside = std::fs::canonicalize(folder).ok().and_then(|found| {
            let sources = std::fs::canonicalize(&self.sources).ok()?;
            (found.parent() == Some(sources.as_path())).then_some(found)
        });
        // The project's own source, or the tree it holds already.
        let same = match (&inside, &held) {
            (Some(_), _) => held.clone().or_else(|| Some(folder.to_path_buf())),
            (None, Some(there)) => {
                (tree_digest(there).ok() == tree_digest(folder).ok()).then(|| there.clone())
            }
            (None, None) => None,
        };
        if let Some(there) = same {
            let reason = format!("is already {} in the project", self.shown(&there));
            return self.push(input, Verdict::AlreadyASource { folder: there }, reason);
        }
        let fresh_id = held.is_some();
        match self.copy_source(folder, &name, &kind, fresh_id) {
            Ok(dest) => {
                let mut reason = format!(
                    "a {kind} source, taken as it is: became {}",
                    self.shown(&dest)
                );
                if fresh_id {
                    reason.push_str("; the project holds another source with its id, so this copy has an id of its own");
                }
                self.push(input, Verdict::Made { folder: dest }, reason);
            }
            Err(why) => self.refuse(input, why),
        }
    }

    /// Copies the source `folder` into a free place of `sources/`, in a hidden staging folder that is renamed into place when the copy is whole.
    fn copy_source(
        &mut self,
        folder: &Path,
        name: &str,
        kind: &str,
        fresh_id: bool,
    ) -> Result<PathBuf, String> {
        let sources_were_there = self.sources.is_dir();
        let stage = self
            .sources
            .join(format!(".import-{}", new_id().unwrap_or_default()));
        let staged = stage.join(folder_name(name, kind));
        let outcome = copy_tree(folder, &staged)
            .map_err(|e| format!("cannot copy it: {e}"))
            .and_then(|()| match fresh_id {
                true => crate::asset::reid(&staged)
                    .map(|_| ())
                    .map_err(|p| p.to_string()),
                false => Ok(()),
            })
            .and_then(|()| {
                let dest = self.index.free(&self.sources, name, kind);
                match dest.exists() {
                    true => Err(format!("{} appeared while it was copied", dest.display())),
                    false => std::fs::rename(&staged, &dest)
                        .map(|()| dest.clone())
                        .map_err(|e| format!("cannot move it into {}: {e}", dest.display())),
                }
            });
        let _ = std::fs::remove_dir_all(&stage);
        if !sources_were_there {
            let _ = std::fs::remove_dir(&self.sources);
        }
        let dest = outcome?;
        let digest =
            tree_digest(&dest).map_err(|e| format!("cannot read {}: {e}", dest.display()))?;
        let id = Source::open(&dest).map_err(|p| p.to_string())?.manifest.id;
        self.index.add_source(&dest, id);
        self.report.created.push(Created {
            path: dest.clone(),
            digest,
        });
        Ok(dest)
    }

    /// Extracts into `stage` and moves the folder to its free place in `sources/`.
    fn extract_into(
        &mut self,
        stage: &Path,
        row: &Row,
        stem: &str,
        bytes: &[u8],
        facts: &Facts,
        sha: &str,
    ) -> Result<PathBuf, String> {
        let extracted = decompile_with(row, stem, bytes, stage, facts)?;
        let dest = self.index.free(&self.sources, stem, row.kind);
        if dest.exists() {
            return Err(format!(
                "{} appeared while the file was imported",
                dest.display()
            ));
        }
        std::fs::rename(&extracted, &dest)
            .map_err(|e| format!("cannot move the source into {}: {e}", dest.display()))?;
        let digest =
            tree_digest(&dest).map_err(|e| format!("cannot read {}: {e}", dest.display()))?;
        self.index.add(&dest, row.kind, sha, facts);
        self.report.created.push(Created {
            path: dest.clone(),
            digest,
        });
        Ok(dest)
    }

    /// A file with no source form: copied under `files/` unless the same file is there, and never over one.
    /// `why` is what stopped a source from being made of it; none when its kind has no source form at all.
    fn keep(
        &mut self,
        input: String,
        extension: &str,
        keep_as: &str,
        bytes: &[u8],
        why: Option<&str>,
    ) {
        let what = match extension {
            "" => "a file with no extension".to_string(),
            e => format!("a .{e}"),
        };
        if !self.options.keep {
            let reason = match why {
                None => format!(
                    "{what} is a compiled file this import has no source form for, so it was not copied"
                ),
                Some(why) => format!(
                    "{what} has no source form this import could make ({why}), so it was not copied"
                ),
            };
            return self.push(input, Verdict::Kept { copied_to: None }, reason);
        }
        if let Some(bad) = keep_as.split('/').find_map(unsafe_file_name) {
            return self.refuse(input, bad);
        }
        let mut number = 1;
        let candidate = loop {
            let candidate = self.files.join(numbered(keep_as, number));
            match std::fs::read(&candidate) {
                Ok(there) if there == bytes => {
                    let reason = format!("{what} is already kept as {}", self.shown(&candidate));
                    let copied_to = Some(candidate);
                    return self.push(input, Verdict::Kept { copied_to }, reason);
                }
                Ok(_) => number += 1,
                Err(e) if e.kind() == io::ErrorKind::NotFound => break candidate,
                Err(e) => return self.refuse(input, format!("cannot look under files/: {e}")),
            }
        };
        let written = candidate
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&candidate, bytes))
            .and_then(|()| tree_digest(&candidate));
        match written {
            Ok(digest) => {
                self.report.created.push(Created {
                    path: candidate.clone(),
                    digest,
                });
                let reason = match why {
                    None => format!(
                        "{what} is a compiled file this import has no source form for yet, so it was kept as it is in {}",
                        self.shown(&candidate)
                    ),
                    Some(why) => format!(
                        "{what} has no source form this import could make ({why}), so it was kept as it is in {}",
                        self.shown(&candidate)
                    ),
                };
                let copied_to = Some(candidate);
                self.push(input, Verdict::Kept { copied_to }, reason);
            }
            Err(e) => {
                let _ = std::fs::remove_file(&candidate);
                self.refuse(input, format!("cannot write {}: {e}", candidate.display()));
            }
        }
    }

    /// `path` as the project spells it: from the folder that holds `sources/` and `files/`.
    fn shown(&self, path: &Path) -> String {
        let root = self.sources.parent().unwrap_or(Path::new(""));
        relative_to(path, root)
    }
}

/// `path` below `base`, with `/` between the names, or `path` itself when it is not below it.
fn relative_to(path: &Path, base: &Path) -> String {
    path.strip_prefix(base).map_or_else(
        |_| path.display().to_string(),
        |rest| {
            rest.components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        },
    )
}

/// The part of `name` after its last `/` or `\`.
fn last_component(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

/// `a/b.ybn` as `a/b_2.ybn` for 2, and as it is for 1.
fn numbered(relative: &str, number: u32) -> PathBuf {
    if number == 1 {
        return relative.split('/').collect();
    }
    let (dir, file) = relative.rsplit_once('/').unwrap_or(("", relative));
    let (stem, extension) = file
        .rsplit_once('.')
        .map_or((file, String::new()), |(s, e)| (s, format!(".{e}")));
    let file = format!("{stem}_{number}{extension}");
    dir.split('/')
        .filter(|part| !part.is_empty())
        .chain([file.as_str()])
        .collect()
}

/// A source folder, and the archive path it states.
type Held = (PathBuf, Option<String>);

/// The sources already in `sources/`, read once: which names are taken and which origins are there.
struct Index {
    /// Lower case, because the file systems this runs on do not tell `A` from `a`.
    taken: HashSet<String>,
    /// By kind and SHA-256.
    origins: HashMap<(String, String), Vec<Held>>,
    /// The folder each asset id is in, so that a source taken as it is is not a second asset of one id.
    ids: HashMap<String, PathBuf>,
}

impl Index {
    fn read(project_root: &Path, sources: &Path) -> Self {
        let mut index = Self {
            taken: HashSet::new(),
            origins: HashMap::new(),
            ids: HashMap::new(),
        };
        for entry in std::fs::read_dir(sources).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            index.taken.insert(name.to_lowercase());
            if parse_folder_name(&name).is_none() {
                continue;
            }
            let manifest = std::fs::read_to_string(entry.path().join(MANIFEST))
                .ok()
                .and_then(|text| AssetManifest::parse(&text).ok());
            if let Some(AssetManifest {
                kind, origin, id, ..
            }) = manifest
            {
                index.ids.insert(id, entry.path());
                if let Some(sha) = origin.sha256 {
                    index
                        .origins
                        .entry((kind, sha))
                        .or_default()
                        .push((entry.path(), origin.virtual_path));
                }
            }
        }
        // ⚠ A source of the project may live anywhere in it, such as a clothing pack's `models/`, and its id is held all the same.
        for folder in source_folders(project_root) {
            if folder.parent() == Some(sources) {
                continue;
            }
            let id = std::fs::read_to_string(folder.join(MANIFEST))
                .ok()
                .and_then(|text| AssetManifest::parse(&text).ok())
                .map(|manifest| manifest.id);
            if let Some(id) = id {
                index.ids.entry(id).or_insert(folder);
            }
        }
        index
    }

    /// The source that holds this file already: the same bytes, and the same archive path when the file has one.
    fn same_origin(&self, kind: &str, sha: &str, facts: &Facts) -> Option<&PathBuf> {
        self.origins
            .get(&(kind.to_string(), sha.to_string()))?
            .iter()
            .find(|(_, path)| facts.virtual_path.is_none() || *path == facts.virtual_path)
            .map(|(folder, _)| folder)
    }

    /// The first folder of `<stem>`, `<stem>_2`, `<stem>_3` and so on that is not there.
    fn free(&self, sources: &Path, stem: &str, kind: &str) -> PathBuf {
        let folder = (1u32..)
            .map(|n| match n {
                1 => folder_name(stem, kind),
                n => folder_name(&format!("{stem}_{n}"), kind),
            })
            .find(|folder| !self.taken.contains(&folder.to_lowercase()))
            .unwrap_or_else(|| folder_name(stem, kind));
        sources.join(folder)
    }

    /// A source taken as it is: its name is taken, and its id is held.
    fn add_source(&mut self, folder: &Path, id: String) {
        let name = folder.file_name().unwrap_or_default().to_string_lossy();
        self.taken.insert(name.to_lowercase());
        self.ids.insert(id, folder.to_path_buf());
    }

    fn add(&mut self, folder: &Path, kind: &str, sha: &str, facts: &Facts) {
        let name = folder.file_name().unwrap_or_default().to_string_lossy();
        self.taken.insert(name.to_lowercase());
        self.origins
            .entry((kind.to_string(), sha.to_string()))
            .or_default()
            .push((folder.to_path_buf(), facts.virtual_path.clone()));
    }
}

/// Every folder below `root` that holds an `asset.json`, as far as `MAX_PROJECT_FOLDERS` folders read.
///
/// ⚠ A source is not entered, `build/`, `vfs/` and a hidden folder are left alone, and a link is not followed, so
/// no walk loops and nothing OpenRage wrote is read back as a source.
pub(crate) fn source_folders(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut visited = 0;
    let mut stack = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        visited += 1;
        if visited > MAX_PROJECT_FOLDERS {
            break;
        }
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !entry.file_type().is_ok_and(|kind| kind.is_dir())
                || name.starts_with('.')
                || name.eq_ignore_ascii_case(BUILD_DIR)
                || name.eq_ignore_ascii_case(SHADOW_DIR)
            {
                continue;
            }
            if entry.path().join(MANIFEST).is_file() {
                found.push(entry.path());
            } else if depth < Bounds::default().depth {
                stack.push((entry.path(), depth + 1));
            }
        }
    }
    found
}

struct Walked {
    files: Vec<PathBuf>,
    /// The folders that hold an `asset.json`: sources, which are taken whole and not walked into.
    sources: Vec<PathBuf>,
    too_deep: Vec<PathBuf>,
}

/// The files of `folder` an import takes, in path order, and the folders left unread for being too deep.
///
/// ⚠ Not `build/` and `vfs/`, a hidden file or folder, a link, a project file, or one of `own` (the
/// project's `files/` and `sources/`, wherever the walk meets them): what a build writes or reads, and
/// what OpenRage wrote itself, are not game files, and walking `files/` nests its copies. A source
/// folder is found and not walked into.
fn walk(folder: &Path, bounds: &Bounds, own: &[&Path]) -> Walked {
    let own: Vec<PathBuf> = own
        .iter()
        .filter_map(|path| std::fs::canonicalize(path).ok())
        .collect();
    let mut walked = Walked {
        files: Vec::new(),
        sources: Vec::new(),
        too_deep: Vec::new(),
    };
    let mut stack = vec![(folder.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            if kind.is_dir() {
                let left_alone = name.eq_ignore_ascii_case(BUILD_DIR)
                    || name.eq_ignore_ascii_case(SHADOW_DIR)
                    || (!own.is_empty()
                        && std::fs::canonicalize(entry.path()).is_ok_and(|p| own.contains(&p)));
                if left_alone {
                    continue;
                }
                if entry.path().join(MANIFEST).is_file() {
                    walked.sources.push(entry.path());
                    continue;
                }
                // A `.src` folder with no marker is not a source, and its files are not game files.
                if name.ends_with(".src") {
                    continue;
                }
                if depth < bounds.depth {
                    stack.push((entry.path(), depth + 1));
                } else {
                    walked.too_deep.push(entry.path());
                }
            } else if kind.is_file() && !is_project_file(&entry.path()) {
                walked.files.push(entry.path());
            }
        }
    }
    walked.files.sort();
    walked.sources.sort();
    walked.too_deep.sort();
    walked
}

/// Whether `path` is a RAGE archive, which holds game files and is none itself.
fn is_archive(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("rpf"))
}

fn is_project_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(PROJECT_EXTENSION))
}

/// Copies the folder `from` to `to`, which is not there yet; a link inside it is left out, since it may lead out of the folder or back into it.
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// What a file browser writes into a folder a person opens, which is no part of what a tool made there or a person changed.
pub const LEFT_BY_A_FILE_BROWSER: [&str; 3] = [".DS_Store", "Thumbs.db", "desktop.ini"];

/// A digest of a file, or of a folder's names and the bytes of every file in it but those of [`LEFT_BY_A_FILE_BROWSER`].
pub fn tree_digest(path: &Path) -> io::Result<String> {
    let mut hash = Sha256::new();
    digest_into(&mut hash, path, "")?;
    Ok(format!("{:x}", hash.finalize()))
}

fn digest_into(hash: &mut Sha256, path: &Path, name: &str) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    let kind: &[u8] = if meta.is_dir() {
        b"d"
    } else if meta.is_file() {
        b"f"
    } else {
        b"o"
    };
    hash.update(kind);
    hash.update((name.len() as u64).to_le_bytes());
    hash.update(name.as_bytes());
    if meta.is_file() {
        hash.update(meta.len().to_le_bytes());
        io::copy(&mut std::fs::File::open(path)?, hash)?;
    } else if meta.is_dir() {
        let mut names: Vec<String> = std::fs::read_dir(path)?
            .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
            .collect::<io::Result<_>>()?;
        names.retain(|child| !LEFT_BY_A_FILE_BROWSER.contains(&child.as_str()));
        names.sort();
        for child in names {
            digest_into(hash, &path.join(&child), &format!("{name}/{child}"))?;
        }
    }
    Ok(())
}

impl Created {
    /// Whether the path is as the import left it.
    pub fn untouched(&self) -> bool {
        tree_digest(&self.path).is_ok_and(|digest| digest == self.digest)
    }

    /// Copies what was made to `to`, which is not there yet, so a Redo can make it again after it is taken back.
    pub fn copy_to(&self, to: &Path) -> io::Result<()> {
        if self.path.is_dir() {
            return copy_tree(&self.path, to);
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&self.path, to).map(drop)
    }

    /// Puts what [`Self::copy_to`] kept at `from` back at its path, which is not there now; a place somebody took since is theirs and is refused.
    pub fn restore_from(&self, from: &Path) -> io::Result<()> {
        if self.path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} is there already", self.path.display()),
            ));
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if std::fs::rename(from, &self.path).is_ok() {
            return Ok(());
        }
        if from.is_dir() {
            copy_tree(from, &self.path)?;
            std::fs::remove_dir_all(from)
        } else {
            std::fs::copy(from, &self.path)?;
            std::fs::remove_file(from)
        }
    }

    /// Removes the path when it is untouched; false, and nothing removed, when somebody changed it.
    pub fn undo(&self) -> io::Result<bool> {
        if !self.untouched() {
            return Ok(false);
        }
        if self.path.is_dir() {
            std::fs::remove_dir_all(&self.path)?;
        } else {
            std::fs::remove_file(&self.path)?;
        }
        Ok(true)
    }
}
