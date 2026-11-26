//! A mapping that comes in as a folder or as a zip: what it holds, what each file would become and what the map rules say of it,
//! before anything is written, and its adoption as a new project.
//!
//! [`analyse`] reads a place (`or_project::place`) in memory and writes nothing. It finds the resources (a folder with a manifest) and
//! the loose stream folders, says of each file what an adoption makes of it ([`Becomes`]), of each line of a manifest whether a build
//! carries it, and applies [`crate::rules`] to the map files, type files and models of each as one set. [`adopt`] then makes a project
//! of one of them in a folder that is new or empty: the place is staged inside that folder, the map files, type files, models and
//! textures become sources of a maps content through [`crate::import`], and every other file is kept whole at its place.
//!
//! ⚠ **A file an asset escrow locks is told by its first four bytes and nothing else of it is read**: it is counted, kept whole and
//! ships as it came. A model or a texture is judged by its name until the adoption, where the reader of its format is asked; one it
//! refuses is kept whole with the reason, and the report an adoption returns says so.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use or_project::place::{
    extension_of, is_archive_name, Place, Refusal, Wanted, ESCROW_ENTITLEMENT,
};
use or_project::resource::manifest::{self, Manifest};
use or_project::resource::{ESCROW_MARKER, STREAM_DIR};
use or_project::{Container, Module, Output, Project, PROJECT_EXTENSION};
use or_source::game_import;
use serde::Serialize;
use serde_json::{json, Value};

use crate::facts::{inside_of, role_of, Fact, Facts, SourceFact};
use crate::import::{add_one, learn, Incoming};
use crate::layout::TYPE_FILE;
use crate::model::{MapSet, Maps};
use crate::rules::{self, Finding};

/// The largest map file, type file or manifest read in memory to be looked into; a larger one is judged by its name.
pub const READ_AT_MOST: u64 = 64 << 20;

/// The folder of the maps content an adoption makes, unless the mapping has one of that name.
pub const MODULE_FOLDER: &str = "maps";

/// What is true of a file kept whole where a source was expected: the build that ships it reads it back first.
const KEPT_AS_IT_CAME: &str =
    "it is kept whole: a build ships it as it came, and stops at it by name if it cannot read it back";

/// What an adoption makes of one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "as", rename_all = "kebab-case")]
pub enum Becomes {
    /// A source of the maps content, of the kind that names its compiler: `metadata`, `drawable`, `drawables`, `fragment` or `textures`.
    Source { kind: String },
    /// A manifest of the game's maps, which the maps content keeps and writes the manifest of its own maps from.
    MapManifest,
    /// The manifest of the resource, whose lines are listed one by one.
    Manifest,
    /// Kept whole at its place: it ships as it came.
    Kept,
    /// Locked by a server's asset escrow: kept whole, counted and never opened.
    Locked,
    /// Kept in the folder of the project and no file of it, with the reason: a build does not ship it.
    KeptBeside,
    /// Not adopted, with the reason: a map file or a type file that does not read.
    Refused,
    /// Not adopted, with the reason: an archive inside the place, which is listed and not opened, or a file a build would write over.
    LeftOut,
}

impl Becomes {
    /// The word a line of the report begins with.
    pub fn word(&self) -> &'static str {
        match self {
            Self::Source { .. } => "source",
            Self::MapManifest => "map manifest",
            Self::Manifest => "manifest",
            Self::Kept => "kept whole",
            Self::Locked => "locked",
            Self::KeptBeside => "kept beside",
            Self::Refused => "refused",
            Self::LeftOut => "left out",
        }
    }

    /// Whether an adoption brings the file into the project.
    pub fn is_adopted(&self) -> bool {
        !matches!(self, Self::Refused | Self::LeftOut)
    }
}

/// One file of a resource or of a stream folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileLine {
    /// Inside the resource, with `/`.
    pub path: String,
    pub bytes: u64,
    pub becomes: Becomes,
    /// One sentence a person can read, empty where the word says it all.
    pub why: String,
}

/// One statement of a resource's manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManifestLine {
    /// The line it is on, from 1.
    pub line: usize,
    pub key: String,
    pub values: Vec<String>,
    /// Whether a build of the adopted project states it.
    pub carried: bool,
    /// Who states it, or why nothing does.
    pub how: String,
}

/// What a root is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RootKind {
    /// A folder with a manifest a server starts it by.
    Resource,
    /// A folder named `stream` that no resource holds.
    StreamFolder,
}

/// A resource or a loose stream folder, with what an adoption makes of it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Root {
    /// The folder inside the place, with `/`; empty for the place itself.
    pub path: String,
    pub kind: RootKind,
    /// The name of its manifest, for a resource.
    pub manifest: Option<String>,
    pub files: Vec<FileLine>,
    pub lines: Vec<ManifestLine>,
    /// The runs of lines of the manifest that are no statement this reads, first and last, from 1.
    pub unread: Vec<(usize, usize)>,
    /// How many files an asset escrow locks.
    pub locked: usize,
    /// What the map rules say of the files that become sources, read as one set.
    pub findings: Vec<Finding>,
}

impl Root {
    /// What to call the root in a sentence.
    pub fn shown(&self) -> &str {
        match self.path.as_str() {
            "" => ".",
            path => path,
        }
    }

    /// How many files become what `word` names ([`Becomes::word`]).
    pub fn count(&self, word: &str) -> usize {
        (self.files.iter())
            .filter(|file| file.becomes.word() == word)
            .count()
    }

    /// Whether an adoption makes a maps content of it.
    pub fn has_maps(&self) -> bool {
        (self.files.iter())
            .any(|file| matches!(file.becomes, Becomes::Source { .. } | Becomes::MapManifest))
    }
}

/// A file of the place that is in no root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Outside {
    pub path: String,
    pub why: String,
}

/// What a place holds, as an adoption would take it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    /// The name of the folder or the archive.
    pub from: String,
    /// Whether the place is an archive, read without unpacking it.
    pub archive: bool,
    pub roots: Vec<Root>,
    pub outside: Vec<Outside>,
    pub notes: Vec<String>,
}

/// The name of the document a [`Report`] is as JSON.
pub const FORMAT: &str = "openrage-mapping-report";
pub const SCHEMA: u32 = 1;

impl Report {
    /// Whether there is anything to adopt.
    pub fn adoptable(&self) -> bool {
        !self.roots.is_empty()
    }

    /// The report as lines of text: each root with its files, the lines of its manifest and its findings, then what is outside.
    pub fn lines(&self) -> Vec<String> {
        let resources = (self.roots.iter())
            .filter(|root| root.kind == RootKind::Resource)
            .count();
        let mut lines = vec![format!(
            "mapping {}: {} resource(s), {} loose stream folder(s)",
            self.from,
            resources,
            self.roots.len() - resources
        )];
        for root in &self.roots {
            let what = match (&root.kind, &root.manifest) {
                (RootKind::Resource, Some(manifest)) => {
                    format!("resource {} ({manifest})", root.shown())
                }
                _ => format!("stream folder of {}", root.shown()),
            };
            let sources = root.count("source");
            lines.push(format!(
                "{what}: {} file(s): {sources} become source(s), {} kept whole, {} locked by an asset escrow and not opened, {} refused, {} left out",
                root.files.len(),
                root.count("kept whole")
                    + root.count("manifest")
                    + root.count("map manifest")
                    + root.count("kept beside"),
                root.locked,
                root.count("refused"),
                root.count("left out"),
            ));
            for file in &root.files {
                lines.push(format!("  {}", file_line(file)));
            }
            for line in &root.lines {
                lines.push(format!("  {}", manifest_line(line)));
            }
            for (first, last) in &root.unread {
                lines.push(format!(
                    "  manifest lines {first} to {last}: not read, they are no statement of a key and its text"
                ));
            }
            for finding in &root.findings {
                lines.push(format!("  {}", finding_line(finding)));
            }
        }
        for outside in &self.outside {
            lines.push(format!("outside {}: {}", outside.path, outside.why));
        }
        for note in &self.notes {
            lines.push(format!("note: {note}"));
        }
        lines
    }

    /// The report as a JSON document, which states what [`Report::lines`] states.
    pub fn document(&self) -> Value {
        let mut document = serde_json::to_value(self).unwrap_or(Value::Null);
        if let Value::Object(top) = &mut document {
            top.insert("format".into(), FORMAT.into());
            top.insert("schema".into(), SCHEMA.into());
        }
        document
    }
}

/// One file as a line of the report.
pub fn file_line(file: &FileLine) -> String {
    let word = match &file.becomes {
        Becomes::Source { kind } => format!("source ({kind})"),
        other => other.word().to_string(),
    };
    match file.why.as_str() {
        "" => format!("{word} {}", file.path),
        why => format!("{word} {}: {why}", file.path),
    }
}

/// One statement of a manifest as a line of the report.
pub fn manifest_line(line: &ManifestLine) -> String {
    let values: Vec<String> = line.values.iter().map(|v| format!("'{v}'")).collect();
    let carried = if line.carried {
        "carried"
    } else {
        "not carried"
    };
    format!(
        "manifest line {} {} {}: {carried}: {}",
        line.line,
        line.key,
        values.join(" "),
        line.how
    )
}

/// One finding of the map rules as a line of the report.
pub fn finding_line(finding: &Finding) -> String {
    format!(
        "{} [{}] {}",
        or_project::resource::word(finding.severity),
        finding.code,
        finding.message
    )
}

fn leaf_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

/// `path` inside the folder `root`, none when it is not in it.
fn inside<'a>(path: &'a str, root: &str) -> Option<&'a str> {
    if root.is_empty() {
        return Some(path);
    }
    path.strip_prefix(root)?.strip_prefix('/')
}

/// Whether `path`, inside a root, is under its stream folder.
fn is_streamed(path: &str) -> bool {
    path.split_once('/')
        .is_some_and(|(first, _)| first.eq_ignore_ascii_case(STREAM_DIR))
}

/// Where an adoption places the file at `path` of a root, and where a build ships it from: its path, with the stream folder spelled
/// as a build spells it.
///
/// ⚠ A maps content cooks into `stream`, and on a disk that does not tell case apart `Stream` is that folder: a file kept under
/// another spelling would be one the build finds in its folder and not in its plan.
fn placed(path: &str) -> String {
    match path.split_once('/') {
        Some((first, rest)) if first.eq_ignore_ascii_case(STREAM_DIR) => {
            format!("{STREAM_DIR}/{rest}")
        }
        _ => path.to_string(),
    }
}

/// Whether `text` is what `pattern` names, where an item of the pattern that `any` takes stands for any run of items.
///
/// ⚠ A pattern is somebody else's input: this goes back only to the last such item, so its cost is the two lengths multiplied,
/// where trying every run for every one of them is exponential in their number.
fn fits<P, T>(
    pattern: &[P],
    text: &[T],
    any: impl Fn(&P) -> bool,
    same: impl Fn(&P, &T) -> bool,
) -> bool {
    let (mut p, mut t) = (0, 0);
    let mut last: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && any(&pattern[p]) {
            last = Some((p, t));
            p += 1;
        } else if p < pattern.len() && same(&pattern[p], &text[t]) {
            p += 1;
            t += 1;
        } else if let Some((at, taken)) = last {
            last = Some((at, taken + 1));
            p = at + 1;
            t = taken + 1;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(any)
}

/// Whether the names of a path are ones the names of a pattern name: `*` in a name is any run of characters, a name that is `**`
/// is any run of folders, and case is not told apart.
fn names_are(pattern: &[&str], path: &[&str]) -> bool {
    let name_is = |pattern: &&str, name: &&str| {
        fits(
            pattern.as_bytes(),
            name.as_bytes(),
            |p| *p == b'*',
            |p, n| p.eq_ignore_ascii_case(n),
        )
    };
    fits(pattern, path, |name| *name == "**", name_is)
}

/// How many paths of one manifest that name files by a pattern are searched for; one past it is said to name no one file.
///
/// ⚠ A pattern is tried on every file of its root, so a manifest of nothing but patterns would cost its lines times the files.
const PATTERNS_AT_MOST: usize = 256;

/// The files of a root that a path a manifest states names: the one file of a plain path, and every file a pattern names; none
/// when the path leaves the resource, or is a pattern past the `patterns` still searched for.
fn named<'a>(
    stated: &str,
    files: &'a [FileLine],
    by_path: &Held<'a>,
    patterns: &Cell<usize>,
) -> Option<Vec<&'a FileLine>> {
    if let Some(inside) = stated_path(stated) {
        let held = by_path.get(&inside.to_ascii_lowercase()).copied();
        return Some(held.into_iter().collect());
    }
    patterns.set(patterns.get().checked_sub(1)?);
    let stated = stated.replace('\\', "/");
    let stated = stated.strip_prefix("./").unwrap_or(&stated);
    let pattern: Vec<&str> = stated.split('/').collect();
    let plain = |name: &&str| or_project::paths::is_component(&name.replace('*', "x"));
    if !manifest::is_glob(stated) || !pattern.iter().all(plain) {
        return None;
    }
    let matches = |file: &&FileLine| {
        let path: Vec<&str> = file.path.split('/').collect();
        names_are(&pattern, &path)
    };
    Some(files.iter().filter(matches).collect())
}

/// The folder that holds the first folder named `stream` on the way to `path`, none when there is none.
fn stream_parent(path: &str) -> Option<&str> {
    let mut at = 0;
    for name in parent_of(path).split('/') {
        if name.eq_ignore_ascii_case(STREAM_DIR) {
            return Some(path[..at].trim_end_matches('/'));
        }
        at += name.len() + 1;
    }
    None
}

/// The place read as an adoption would take it; nothing is written.
pub fn analyse(place: &mut Place) -> Report {
    let from = (place.at().file_name()).map_or_else(
        || place.at().display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let mut report = Report {
        from,
        archive: place.is_archive(),
        roots: Vec::new(),
        outside: Vec::new(),
        notes: Vec::new(),
    };
    for left in place.left() {
        report.outside.push(Outside {
            path: left.path.clone(),
            why: left.why.clone(),
        });
    }
    let held: Vec<(String, u64)> = (place.files().iter())
        .map(|file| (file.path.clone(), file.bytes))
        .collect();

    // The folders with a manifest are the resources; a file belongs to the deepest one above it.
    let mut resources: BTreeMap<String, String> = BTreeMap::new();
    for (path, _) in &held {
        let leaf = leaf_of(path).to_ascii_lowercase();
        if let Some(rank) = manifest::NAMES.iter().position(|name| *name == leaf) {
            let known = resources
                .entry(parent_of(path).to_string())
                .or_insert_with(|| path.clone());
            // A folder with both names is started by the first of them.
            let rank_of = |path: &str| {
                let leaf = leaf_of(path).to_ascii_lowercase();
                manifest::NAMES.iter().position(|name| *name == leaf)
            };
            if rank_of(known).is_some_and(|held| rank < held) {
                *known = path.clone();
            }
        }
    }
    let resource_of = |path: &str| {
        let mut folder = parent_of(path);
        loop {
            if resources.contains_key(folder) {
                return Some(folder.to_string());
            }
            if folder.is_empty() {
                return None;
            }
            folder = parent_of(folder);
        }
    };

    let mut rooted: BTreeMap<String, (RootKind, Vec<(String, u64)>)> = BTreeMap::new();
    for (path, bytes) in &held {
        let (root, kind) = match (resource_of(path), stream_parent(path)) {
            (Some(resource), _) => (resource, RootKind::Resource),
            (None, Some(parent)) => (parent.to_string(), RootKind::StreamFolder),
            (None, None) => {
                report.outside.push(Outside {
                    path: path.clone(),
                    why: format!(
                        "{}, in no resource and under no stream folder",
                        what_is(path)
                    ),
                });
                continue;
            }
        };
        let entry = rooted.entry(root).or_insert_with(|| (kind, Vec::new()));
        entry.1.push((path.clone(), *bytes));
    }

    for (path, (kind, files)) in rooted {
        let manifest = resources.get(&path).map(|at| leaf_of(at).to_string());
        let root = read_root(place, path, kind, manifest, &files);
        report.roots.push(root);
    }
    if report.roots.is_empty() {
        report.notes.push(format!(
            "no resource and no stream folder, so there is nothing to adopt: {}",
            found_by_kind(&held)
        ));
    }
    for root in &report.roots {
        let locked_types = (root.files.iter())
            .any(|file| file.becomes == Becomes::Locked && extension_of(&file.path) == "ytyp");
        if locked_types {
            report.notes.push(format!(
                "a type file of {} is locked, so the archetypes it declares are not known and the map rules may name one as undeclared",
                root.shown()
            ));
        }
        let entitled = root
            .files
            .iter()
            .any(|file| file.path == ESCROW_ENTITLEMENT);
        if root.locked > 0 && !entitled {
            report.notes.push(format!(
                "{} file(s) of {} are locked by an asset escrow and it holds no `{ESCROW_ENTITLEMENT}`, the file an escrow writes beside what it locks, so a server may not open them",
                root.locked,
                root.shown()
            ));
        }
        let kept_types = (root.files.iter()).any(|file| {
            file.becomes == Becomes::Kept
                && is_streamed(&file.path)
                && extension_of(&file.path) == "ytyp"
        });
        if kept_types {
            report.notes.push(format!(
                "a type file of {} is kept whole and is no source, so the map rules do not read the archetypes it declares and may name one as undeclared",
                root.shown()
            ));
        }
    }
    report
}

/// What a file is called by its extension, for a sentence about one that is in no root.
fn what_is(path: &str) -> String {
    match extension_of(path).as_str() {
        "ymap" => "a map file".into(),
        "ytyp" => "a type file".into(),
        "" => "a file with no extension".into(),
        extension => match game_import::kind_of(extension) {
            Some(row) => format!("a .{extension} file, which becomes a {} source", row.kind),
            None => format!("a .{extension} file"),
        },
    }
}

/// How many files of each extension `held` has, as `2 .ymap, 1 .txt`.
fn found_by_kind(held: &[(String, u64)]) -> String {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for (path, _) in held {
        *counts.entry(extension_of(path)).or_default() += 1;
    }
    if counts.is_empty() {
        return "it holds no file".into();
    }
    let said: Vec<String> = (counts.iter())
        .map(|(extension, count)| match extension.as_str() {
            "" => format!("{count} with no extension"),
            extension => format!("{count} .{extension}"),
        })
        .collect();
    format!("it holds {}", said.join(", "))
}

/// A hash to the text of a name the files of a root carry in their own names, which a map file states as a number.
fn stems_of(files: &[(String, u64)]) -> HashMap<u32, String> {
    (files.iter())
        .filter_map(|(path, _)| {
            let stem = leaf_of(path).rsplit_once('.')?.0.to_ascii_lowercase();
            Some((or_io::string_hash(stem.as_bytes()), stem))
        })
        .collect()
}

fn read_root(
    place: &mut Place,
    path: String,
    kind: RootKind,
    manifest: Option<String>,
    files: &[(String, u64)],
) -> Root {
    let mut root = Root {
        path,
        kind,
        manifest,
        files: Vec::new(),
        lines: Vec::new(),
        unread: Vec::new(),
        locked: 0,
        findings: Vec::new(),
    };
    let stems = stems_of(files);
    let names = |hash: u32| stems.get(&hash).cloned();
    let mut read = Sources {
        facts: Facts::default(),
        set: MapSet {
            // ⚠ A fixed id: a finding names its set by it, and two readings of one mapping have to say the same.
            id: "mapping".into(),
            name: leaf_of(root.shown()).to_string(),
            sources: Vec::new(),
        },
        made: HashMap::new(),
    };
    let mut stated: Option<Manifest> = None;
    for (at, bytes) in files {
        let rel = inside(at, &root.path).unwrap_or(at).to_string();
        let mut line = FileLine {
            path: rel.clone(),
            bytes: *bytes,
            becomes: Becomes::Kept,
            why: String::new(),
        };
        let extension = extension_of(&rel);
        let is_manifest = root
            .manifest
            .as_deref()
            .is_some_and(|m| rel.eq_ignore_ascii_case(m));
        if is_manifest {
            line.becomes = Becomes::Manifest;
            match read_within(place, at, *bytes) {
                Ok(text) => stated = Some(Manifest::read(&String::from_utf8_lossy(&text))),
                Err(why) => line.why = format!("its lines are not read: {why}"),
            }
        } else if rel == ESCROW_ENTITLEMENT {
            line.why = "the file an asset escrow writes beside the files it locks: kept whole, and a build ships it at its root and lists it nowhere".into();
        } else if is_archive_name(&rel) {
            line.becomes = Becomes::LeftOut;
            line.why =
                "an archive: it is listed and not opened, and nothing in it is adopted".into();
        } else {
            match place.head(at, ESCROW_MARKER.len()) {
                Err(refusal) => {
                    line.becomes = Becomes::Refused;
                    line.why = refusal.to_string();
                }
                Ok(head) if head == *ESCROW_MARKER => {
                    root.locked += 1;
                    line.becomes = Becomes::Locked;
                    line.why =
                        "locked by a server's asset escrow: kept whole, counted and not opened"
                            .into();
                }
                Ok(_) if is_streamed(&rel) => {
                    streamed(place, at, &extension, &mut line, &mut read, &names);
                }
                Ok(_) => {}
            }
        }
        root.files.push(line);
    }
    written_over(&mut root.files);
    if let Some(stated) = &stated {
        if root.has_maps() {
            scripts_of_the_server(stated, &mut root.files);
        }
        root.lines = lines_of(stated, &root);
        root.unread = stated.unread.clone();
    }
    if !read.set.sources.is_empty() {
        let mut model = Maps::new();
        model.maps.push(read.set);
        root.findings = rules::check(&model, &read.facts, None);
    }
    root
}

/// Leave out each file kept whole at the path a source of the root cooks to.
///
/// ⚠ A source cooks into the stream folder by its name alone, whatever folder its file came from, so a second file of that name
/// that stands directly in the stream folder would ship at the same path, and a plan of both is refused.
fn written_over(files: &mut [FileLine]) {
    let cooked: HashMap<String, String> = (files.iter())
        .filter(|file| matches!(file.becomes, Becomes::Source { .. }))
        .map(|file| {
            let at = format!("{STREAM_DIR}/{}", leaf_of(&file.path));
            (at.to_lowercase(), file.path.clone())
        })
        .collect();
    for file in files {
        if !matches!(file.becomes, Becomes::Kept | Becomes::Locked) {
            continue;
        }
        if let Some(source) = cooked.get(&placed(&file.path).to_lowercase()) {
            file.becomes = Becomes::LeftOut;
            file.why = format!(
                "`{source}` becomes a source, which a build cooks to this file's path, and two files do not ship at one path, so this one is left out"
            );
        }
    }
}

/// The keys of a manifest that name the scripts only a server runs.
const SERVER_SCRIPTS: [&str; 2] = ["server_script", "server_scripts"];

/// Keep beside the project each file a line of `stated` names as a script of the server.
///
/// ⚠ A build writes its own manifest, which lists every file it ships outside the stream folder under `files`, and a server sends
/// those to every client: a script of the server shipped that way would be published, and no line would run it.
fn scripts_of_the_server(stated: &Manifest, files: &mut [FileLine]) {
    let mut scripts: BTreeSet<String> = BTreeSet::new();
    {
        let by_path = by_path(files);
        let patterns = Cell::new(PATTERNS_AT_MOST);
        let lines = (stated.directives.iter())
            .filter(|directive| SERVER_SCRIPTS.contains(&directive.key.as_str()));
        for line in lines {
            for value in &line.values {
                for file in named(value, files, &by_path, &patterns).unwrap_or_default() {
                    scripts.insert(file.path.clone());
                }
            }
        }
    }
    for file in files {
        let kept = file.becomes == Becomes::Kept && !is_streamed(&file.path);
        if kept && scripts.contains(&file.path) {
            file.becomes = Becomes::KeptBeside;
            file.why = "a `server_script` line names it, and a build writes no script line and lists every file it ships outside the stream folder under `files`, which a server sends to every client: it is kept beside the project and no build ships it".into();
        }
    }
}

/// The bytes of the file at `at`, when it is small enough to be looked into in memory.
fn read_within(place: &mut Place, at: &str, bytes: u64) -> Result<Vec<u8>, String> {
    if bytes > READ_AT_MOST {
        return Err(format!(
            "it states {bytes} bytes, and a file is read in memory up to {READ_AT_MOST}"
        ));
    }
    place.read(at).map_err(|refusal| refusal.to_string())
}

/// The files of a root that become sources, as the map rules read them.
struct Sources {
    facts: Facts,
    set: MapSet,
    /// The source folder each makes, in lower case, with the file that makes it.
    made: HashMap<String, String>,
}

/// What an adoption makes of the streamed file at `at`, which no escrow locks.
fn streamed(
    place: &mut Place,
    at: &str,
    extension: &str,
    line: &mut FileLine,
    read: &mut Sources,
    names: &dyn Fn(u32) -> Option<String>,
) {
    let stem = leaf_of(&line.path)
        .rsplit_once('.')
        .map_or("", |(stem, _)| stem);
    let mut fact = SourceFact {
        kind: String::new(),
        name: stem.to_string(),
        extension: extension.to_string(),
        bytes: line.bytes,
        game: Vec::new(),
        digest: None,
        inside: None,
    };
    match extension {
        "ymf" => {
            line.becomes = Becomes::MapManifest;
            line.why =
                "kept by the maps content, which writes the manifest of its own maps from it"
                    .into();
            return;
        }
        "ymap" | "ytyp" => {
            fact.kind = or_source::metadata::KIND.to_string();
            match read_within(place, at, line.bytes) {
                Ok(bytes) => match inside_of(&bytes, extension, Some(names)) {
                    Ok(read) => fact.inside = Some(Ok(read)),
                    Err(why) => {
                        line.becomes = Becomes::Refused;
                        line.why = format!("it {why}");
                        return;
                    }
                },
                Err(why) => {
                    line.why = format!("not looked into before it is adopted: {why}");
                }
            }
        }
        extension => match game_import::kind_of(extension) {
            Some(row) if role_of(row.kind, extension).is_some() => fact.kind = row.kind.to_string(),
            Some(row) => {
                line.why = format!(
                    "it becomes a {} source where a project takes one, and a maps content holds map files, type files, models and textures",
                    row.kind
                );
                return;
            }
            None => return,
        },
    }
    // ⚠ A source is a folder named for the file's stem and its kind, and the content keeps every source in one folder: a map file
    // and a type file of one name are one name there, and so are two models of one name in two folders. Case is not told apart,
    // as on the disks of Windows and macOS, so a mapping is adopted the same on every host.
    let folder = or_source::asset::folder_name(stem, &fact.kind);
    if let Some(first) = read.made.get(&folder.to_lowercase()) {
        line.why = format!(
            "the source `{folder}` is made of `{first}`, and one folder is one source, so {KEPT_AS_IT_CAME}"
        );
        return;
    }
    read.made.insert(folder.to_lowercase(), line.path.clone());
    line.becomes = Becomes::Source {
        kind: fact.kind.clone(),
    };
    // A model and a texture dictionary are not looked into: the rules read the map files and the type files and know the rest by name.
    read.facts.insert(&line.path, Fact::Source(Box::new(fact)));
    read.set.sources.push(line.path.clone());
}

/// A path a manifest states, as the path inside the resource it names; none when it leaves the resource or names files by a pattern.
fn stated_path(stated: &str) -> Option<String> {
    let stated = stated.replace('\\', "/");
    let stated = stated.strip_prefix("./").unwrap_or(&stated);
    let plain =
        !manifest::is_glob(stated) && stated.split('/').all(or_project::paths::is_component);
    plain.then(|| stated.to_string())
}

/// The files of a root by their path in lower case.
type Held<'a> = HashMap<String, &'a FileLine>;

/// The files of a root by their path in lower case, which is how a manifest finds the one it names.
///
/// ⚠ A manifest states a line for each of thousands of files, so the file a line names is looked up and never searched for.
fn by_path(files: &[FileLine]) -> Held<'_> {
    (files.iter())
        .map(|file| (file.path.to_ascii_lowercase(), file))
        .collect()
}

/// What a line of a manifest is judged against: the files of its root.
struct OfRoot<'a> {
    files: &'a [FileLine],
    by_path: Held<'a>,
    /// Whether a map file becomes a source, which is when the maps content declares the resource a map.
    cooks_a_map: bool,
    /// How many more paths that name files by a pattern are searched for.
    patterns: Cell<usize>,
}

impl<'a> OfRoot<'a> {
    fn of(files: &'a [FileLine]) -> Self {
        Self {
            files,
            by_path: by_path(files),
            cooks_a_map: (files.iter()).any(|file| {
                matches!(file.becomes, Becomes::Source { .. }) && extension_of(&file.path) == "ymap"
            }),
            patterns: Cell::new(PATTERNS_AT_MOST),
        }
    }
}

/// Each statement of `stated` with whether a build of the adopted project carries it.
fn lines_of(stated: &Manifest, root: &Root) -> Vec<ManifestLine> {
    let has_maps = root.has_maps();
    let of_root = OfRoot::of(&root.files);
    (stated.directives.iter())
        .map(|directive| {
            let values: Vec<&str> = directive.values.iter().map(String::as_str).collect();
            let (carried, how) = if !has_maps {
                (true, "the resource makes no maps content, so its manifest is kept as it is and ships with every line".to_string())
            } else {
                carried(&directive.key, &values, &of_root)
            };
            ManifestLine {
                line: directive.line,
                key: directive.key.clone(),
                values: directive.values.clone(),
                carried,
                how,
            }
        })
        .collect()
}

/// The words a build states about itself in its manifest, which an adoption takes from the mapping's.
const WORDS: [&str; 3] = ["author", "description", "version"];

/// Whether a build states the type `file_type` for the file `held` of a root that makes a maps content, and who states it or why nothing does.
fn typed(file_type: &str, held: &FileLine) -> (bool, String) {
    let inside = &held.path;
    match &held.becomes {
        Becomes::Source { .. } if file_type == TYPE_FILE && extension_of(inside) == "ytyp" => {
            (true, "the maps content registers the type file it cooks".into())
        }
        Becomes::Source { .. } | Becomes::MapManifest => (
            false,
            format!("`{inside}` becomes a source, and the maps content states no {file_type} line for what it cooks"),
        ),
        Becomes::Kept | Becomes::Locked => {
            let mut files = Module::files("files");
            match files.set_file_type(&placed(inside), Some(file_type)) {
                Ok(()) => (true, "the Files content states the type of the file".into()),
                Err(why) => (false, why),
            }
        }
        Becomes::Manifest | Becomes::KeptBeside | Becomes::Refused | Becomes::LeftOut => {
            (false, format!("`{inside}` is not adopted as a file of the project"))
        }
    }
}

/// Whether a build carries the statement `key values` of a resource that makes a maps content, and who states it or why nothing does.
fn carried(key: &str, values: &[&str], root: &OfRoot<'_>) -> (bool, String) {
    match (key, values) {
        ("fx_version" | "resource_manifest_version" | "game" | "games", _) => {
            (true, "a build writes its own line of this kind".into())
        }
        // ⚠ The maps content declares the resource a map only with a map file it cooks: a pack of props states no such line.
        (manifest::MAP_KEY, _) if root.cooks_a_map => (true, "the maps content states it".into()),
        (manifest::MAP_KEY, _) => (
            false,
            "no map file of the resource becomes a source, and the maps content states the line only with a map file it cooks".into(),
        ),
        // ⚠ The reader keeps a backslash as it is written, and Lua reads one as an escape: what the text means is not known here.
        (word, [text]) if WORDS.contains(&word) && text.contains('\\') => (
            false,
            format!("its text holds a backslash, which Lua may read as an escape and which is read here as it is written, so a build would state other text: state `{word}` in the build by hand"),
        ),
        (word, [_]) if WORDS.contains(&word) => (true, "the build states it".into()),
        ("file" | "files", _) => (
            true,
            "a build lists every file it ships outside the stream folder".into(),
        ),
        ("data_file", [file_type, path]) => {
            let Some(named) = named(path, root.files, &root.by_path, &root.patterns) else {
                let why = match root.patterns.get() {
                    0 if manifest::is_glob(path) => format!("more than {PATTERNS_AT_MOST} paths of the manifest name files by a pattern, and the files of one past that are not searched for"),
                    _ => format!("`{path}` is no one file inside the resource, and a type is stated for one file"),
                };
                return (false, why);
            };
            match (named.as_slice(), stated_path(path)) {
                ([], Some(inside)) => (false, format!("the resource holds no `{inside}`")),
                ([], None) => (false, format!("`{path}` names no file of the resource")),
                ([held], Some(_)) => typed(file_type, held),
                // A pattern is carried once every file it names is stated, by whoever ships that file.
                (several, _) => {
                    let said: Vec<(bool, String)> =
                        several.iter().map(|held| typed(file_type, held)).collect();
                    match said.iter().filter(|(carried, _)| !carried).count() {
                        0 => {
                            let how: BTreeSet<&str> = said.iter().map(|(_, how)| how.as_str()).collect();
                            let how: Vec<&str> = how.into_iter().collect();
                            (true, format!("it names {} file(s), each stated for itself: {}", said.len(), how.join("; ")))
                        }
                        unstated => {
                            let first = said.iter().find(|(carried, _)| !carried).map_or("", |(_, why)| why);
                            (false, format!("of the {} file(s) it names, {unstated} are not stated; the first: {first}", said.len()))
                        }
                    }
                }
            }
        }
        _ => (
            false,
            format!("a build writes its own manifest and states no `{key}` line, so add it by hand if the resource needs it"),
        ),
    }
}

/// Why a mapping is not adopted.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Refused(#[from] Refusal),
    #[error("there is no resource and no stream folder in it, so nothing is adopted")]
    Nothing,
    #[error("it holds {} resources or stream folders, and one project is made of one: name it ({})", .0.len(), .0.join(", "))]
    Several(Vec<String>),
    #[error("it holds no resource and no stream folder at `{0}`")]
    NoSuchRoot(String),
    #[error("{0} is not a new or an empty folder, and a mapping is adopted into one")]
    Held(String),
    #[error("{0} is inside the folder adopted from, which is read and never changed")]
    Inside(String),
    #[error("{0}")]
    Write(String),
}

/// What an adoption is asked.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    /// The folder the project is made in: not there yet, or empty.
    pub into: &'a Path,
    /// The name of the project, which names its file.
    pub name: &'a str,
    /// The name of the resource its build writes.
    pub resource: &'a str,
    /// The root to adopt, as [`Root::path`]; needed only where the place holds more than one.
    pub root: Option<&'a str>,
}

/// What an adoption made.
#[derive(Debug, Clone, PartialEq)]
pub struct Adopted {
    /// The project file.
    pub project: PathBuf,
    /// The root adopted, with what each file became in the end.
    pub root: Root,
    /// The source folders made.
    pub sources: Vec<PathBuf>,
    /// The files kept whole, which are the documents of the project.
    pub kept: Vec<PathBuf>,
}

/// The root of `report` an adoption takes: the one `named`, else the only one.
pub fn chosen<'r>(report: &'r Report, named: Option<&str>) -> Result<&'r Root, Error> {
    match (named, report.roots.as_slice()) {
        (_, []) => Err(Error::Nothing),
        (Some(named), roots) => {
            let named = named.trim_matches('/');
            let named = if named == "." { "" } else { named };
            (roots.iter())
                .find(|root| root.path == named)
                .ok_or_else(|| Error::NoSuchRoot(named.to_string()))
        }
        (None, [only]) => Ok(only),
        (None, several) => Err(Error::Several(
            several
                .iter()
                .map(|root| root.shown().to_string())
                .collect(),
        )),
    }
}

/// Where in the place each file of `root` that an adoption brings in is, in the order of its files.
fn staged_from(root: &Root) -> Vec<String> {
    (root.files.iter())
        .filter(|file| file.becomes.is_adopted())
        .map(|file| match root.path.as_str() {
            "" => file.path.clone(),
            folder => format!("{folder}/{}", file.path),
        })
        .collect()
}

/// The files of `root` to stage, each from its place in `from` ([`staged_from`]) to its path inside the root.
fn wanted_of<'a>(root: &'a Root, from: &'a [String]) -> Vec<Wanted<'a>> {
    (root.files.iter())
        .filter(|file| file.becomes.is_adopted())
        .zip(from)
        .map(|(file, from)| Wanted {
            from,
            to: &file.path,
        })
        .collect()
}

/// What stops the adoption `request` asks for before it writes anything, which is the first thing [`adopt`] asks; nothing is written.
pub fn check(place: &Place, report: &Report, request: &Request<'_>) -> Result<(), Error> {
    let root = chosen(report, request.root)?;
    let into = request.into;
    let there = into.exists();
    let empty = || std::fs::read_dir(into).is_ok_and(|mut entries| entries.next().is_none());
    if there && !(into.is_dir() && empty()) {
        return Err(Error::Held(into.display().to_string()));
    }
    let absolute = |path: &Path| std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    if !place.is_archive() && absolute(into).starts_with(absolute(place.at())) {
        return Err(Error::Inside(into.display().to_string()));
    }
    let from = staged_from(root);
    place.fits(into, &wanted_of(root, &from))?;
    Ok(())
}

/// Make a project of one root of `report`, which is what [`analyse`] said of `place`, in the folder the request names.
pub fn adopt(place: &mut Place, report: &Report, request: &Request<'_>) -> Result<Adopted, Error> {
    // ⚠ Nothing is written unless the place stages whole, and a failure after that removes everything this call made.
    check(place, report, request)?;
    let root = chosen(report, request.root)?;
    let into = request.into;
    let from = staged_from(root);
    let wanted = wanted_of(root, &from);
    // The highest folder this call makes, which is what a failure takes away again.
    let made_from = (into.ancestors())
        .take_while(|folder| !folder.as_os_str().is_empty() && !folder.exists())
        .last()
        .map(Path::to_path_buf);
    std::fs::create_dir_all(into).map_err(|e| Error::Write(format!("{}: {e}", into.display())))?;
    let made = fill(place, root, request, &wanted);
    if made.is_err() {
        match &made_from {
            Some(folder) => {
                let _ = std::fs::remove_dir_all(folder);
            }
            None => {
                for entry in std::fs::read_dir(into).into_iter().flatten().flatten() {
                    let path = entry.path();
                    let _ = if path.is_dir() {
                        std::fs::remove_dir_all(&path)
                    } else {
                        std::fs::remove_file(&path)
                    };
                }
            }
        }
    }
    made
}

/// Move the staged file `from` to `to`, which is not there.
fn place_at(from: &Path, to: &Path) -> Result<(), Error> {
    let said = |e: std::io::Error| Error::Write(format!("{}: {e}", to.display()));
    if to.exists() {
        return Err(Error::Write(format!("{} is there already", to.display())));
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(said)?;
    }
    std::fs::rename(from, to).map_err(said)
}

fn fill(
    place: &mut Place,
    root: &Root,
    request: &Request<'_>,
    wanted: &[Wanted<'_>],
) -> Result<Adopted, Error> {
    let into = request.into;
    let staged = place.stage(into, wanted)?;
    let mut root = root.clone();
    // The maps content has a folder of its own, beside whatever the mapping calls its own folders.
    let top: BTreeSet<String> = (root.files.iter())
        .map(|file| {
            file.path
                .split('/')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase()
        })
        .collect();
    let mut module_folder = MODULE_FOLDER.to_string();
    let mut number = 1;
    while top.contains(&module_folder) {
        number += 1;
        module_folder = format!("{MODULE_FOLDER}-{number}");
    }
    let maps_folder = into.join(&module_folder);
    let has_maps = root.has_maps();
    let mut sources = Vec::new();
    let mut kept = Vec::new();
    let mut manifest_kept = false;
    for file in &mut root.files {
        let at = staged.path(&file.path);
        let to = into.join(placed(&file.path).replace('/', std::path::MAIN_SEPARATOR_STR));
        let made = match &file.becomes {
            Becomes::Refused | Becomes::LeftOut => continue,
            Becomes::Kept | Becomes::Locked => None,
            Becomes::KeptBeside => {
                place_at(&at, &to)?;
                continue;
            }
            // ⚠ A build writes the manifest of a maps content, so the mapping's own is kept beside the project and is no document of it.
            Becomes::Manifest if has_maps => {
                place_at(&at, &to)?;
                continue;
            }
            Becomes::Manifest => {
                manifest_kept = true;
                None
            }
            Becomes::Source { .. } | Becomes::MapManifest => {
                let bytes = std::fs::read(&at)
                    .map_err(|e| Error::Write(format!("{}: {e}", at.display())))?;
                let made = if file.becomes == Becomes::MapManifest {
                    crate::manifest::keep(&maps_folder, &bytes).map(|_| None)
                } else {
                    let incoming = Incoming {
                        name: leaf_of(&file.path),
                        bytes: &bytes,
                        from_game: false,
                        virtual_path: None,
                    };
                    add_one(&maps_folder, &incoming).map(Some)
                };
                match made {
                    Ok(made) => Some(made),
                    Err(why) => {
                        file.becomes = Becomes::Kept;
                        file.why = format!("no source is made of it ({why}), so {KEPT_AS_IT_CAME}");
                        None
                    }
                }
            }
        };
        match made {
            Some(Some(folder)) => sources.push(folder),
            Some(None) => {}
            None => {
                place_at(&at, &to)?;
                kept.push(to);
            }
        }
    }
    drop(staged);

    let mut modules = Vec::new();
    if has_maps {
        learn(&maps_folder, &sources, None).map_err(Error::Write)?;
        modules.push(Module::maps("maps", &module_folder));
    }
    if !kept.is_empty() || modules.is_empty() {
        let mut files = Module::files("files");
        let by_path = by_path(&root.files);
        // The lines the report said are carried, which are no more patterns than it searched for.
        let patterns = Cell::new(PATTERNS_AT_MOST);
        let carried = (root.lines.iter()).filter(|line| line.carried && line.key == "data_file");
        for line in carried {
            let [file_type, path] = line.values.as_slice() else {
                continue;
            };
            for held in named(path, &root.files, &by_path, &patterns).unwrap_or_default() {
                if matches!(held.becomes, Becomes::Kept | Becomes::Locked) {
                    files
                        .set_file_type(&placed(&held.path), Some(file_type))
                        .map_err(Error::Write)?;
                }
            }
        }
        modules.push(files);
    }
    let take: Vec<&str> = modules.iter().map(|module| module.id.as_str()).collect();
    let mut output = Output::fivem_resource("fivem", request.resource, &take);
    if !manifest_kept {
        for line in root.lines.iter().filter(|line| line.carried) {
            if let (true, [text]) = (WORDS.contains(&line.key.as_str()), line.values.as_slice()) {
                output.extra.insert(line.key.clone(), text.clone().into());
            }
        }
    }
    let container = Container {
        modules,
        outputs: vec![output],
    };
    let mut extra = serde_json::Map::new();
    if let Some(record) = record_of(&root, manifest_kept) {
        extra.insert(or_project::adopt::ADOPTED_KEY.into(), record);
    }
    let project = into.join(format!("{}.{PROJECT_EXTENSION}", request.name));
    Project::create_with(&project, &container, &kept, extra)
        .map_err(|e| Error::Write(format!("{}: {e}", project.display())))?;
    Ok(Adopted {
        project,
        root,
        sources,
        kept,
    })
}

/// What the project file records of the adoption of `root`: the files and the lines of the manifest that did not come along.
fn record_of(root: &Root, manifest_kept: bool) -> Option<Value> {
    let mut unplaced: Vec<Value> = (root.files.iter())
        .filter(|file| !file.becomes.is_adopted() || file.becomes == Becomes::KeptBeside)
        .map(|file| json!({ "path": file.path, "why": file.why }))
        .collect();
    if let (Some(manifest), false) = (&root.manifest, manifest_kept) {
        unplaced.push(json!({
            "path": manifest,
            "why": "a build writes its own manifest, so this one is kept beside the project and is no file of it",
        }));
    }
    let mut notes: Vec<String> = (root.lines.iter())
        .filter(|line| !line.carried)
        .map(manifest_line)
        .collect();
    if root.locked > 0 {
        notes.push(format!(
            "{} file(s) are locked by a server's asset escrow: kept whole and not opened",
            root.locked
        ));
    }
    if unplaced.is_empty() && notes.is_empty() {
        return None;
    }
    Some(json!({ "unplaced": unplaced, "notes": notes }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_under_the_stream_folder_whatever_the_case_and_a_root_is_the_folder_above_it() {
        assert!(is_streamed("stream/a.ymap") && is_streamed("Stream/sub/a.ymap"));
        assert!(!is_streamed("data/stream/a.ymap") && !is_streamed("stream"));
        assert_eq!(stream_parent("stream/a.ymap"), Some(""));
        assert_eq!(stream_parent("pack/STREAM/sub/a.ymap"), Some("pack"));
        assert_eq!(stream_parent("a/b/stream/stream/c.ydr"), Some("a/b"));
        assert_eq!(stream_parent("pack/streams/a.ymap"), None);
        assert_eq!(
            stream_parent("stream"),
            None,
            "a file called stream is no folder"
        );
        assert_eq!(inside("pack/stream/a.ymap", "pack"), Some("stream/a.ymap"));
        assert_eq!(inside("packed/a.ymap", "pack"), None);
        assert_eq!(inside("a.ymap", ""), Some("a.ymap"));
    }

    #[test]
    fn a_line_of_a_manifest_is_carried_by_whoever_states_it_or_said_not_to_be() {
        let line = |path: &str, becomes: Becomes| FileLine {
            path: path.into(),
            bytes: 1,
            becomes,
            why: String::new(),
        };
        let files = [
            line(
                "stream/props.ytyp",
                Becomes::Source {
                    kind: "metadata".into(),
                },
            ),
            line("data/handling.meta", Becomes::Kept),
            line("stream/old.zip", Becomes::LeftOut),
        ];
        let with_a_map = [
            files[0].clone(),
            line(
                "stream/sub/harbour.ymap",
                Becomes::Source {
                    kind: "metadata".into(),
                },
            ),
            line("Stream/kept.ytyp", Becomes::Kept),
        ];
        let of_root = OfRoot::of(&files);
        let said = |key: &str, values: &[&str]| carried(key, values, &of_root);

        assert!(said("fx_version", &["cerulean"]).0);
        let (is_map, why) = said(manifest::MAP_KEY, &["yes"]);
        assert!(
            !is_map && why.contains("no map file of the resource becomes a source"),
            "a pack of props is declared no map by its build: {why}"
        );
        let mapped = OfRoot::of(&with_a_map);
        assert!(carried(manifest::MAP_KEY, &["yes"], &mapped).0);
        assert!(said("author", &["Ann"]).0);
        let (escaped, why) = said("description", &["Line one\\nLine two"]);
        assert!(!escaped && why.contains("backslash"), "{why}");
        assert!(!said("author", &["Ann", "Ben"]).0, "a word is one text");
        assert!(said("data_file", &[TYPE_FILE, "stream/props.ytyp"]).0);
        assert!(!said("data_file", &["HANDLING_FILE", "stream/props.ytyp"]).0);
        assert!(said("data_file", &["HANDLING_FILE", "data\\handling.meta"]).0);
        assert!(
            !said("data_file", &["handling", "data/handling.meta"]).0,
            "a type is capitals"
        );
        // A pattern is carried when every file it names is stated, each by whoever ships it.
        let (pattern, how) = said("data_file", &["HANDLING_FILE", "data/*.meta"]);
        assert!(
            pattern && how.contains("1 file(s)") && how.contains("the Files content"),
            "{how}"
        );
        for types in ["stream/*.ytyp", "stream/**/*.ytyp", "**/*.YTYP"] {
            let (carried, how) = said("data_file", &[TYPE_FILE, types]);
            assert!(
                carried && how.contains("the maps content registers"),
                "{types}: {how}"
            );
        }
        let (both, how) = carried("data_file", &[TYPE_FILE, "stream/*.ytyp"], &mapped);
        assert!(
            both && how.contains("2 file(s)")
                && how.contains("the maps content registers")
                && how.contains("the Files content"),
            "a type file kept whole and one cooked are both named: {how}"
        );
        assert!(
            !said("data_file", &[TYPE_FILE, "stream/*.ymap"]).0,
            "it names no file"
        );
        let (left, how) = said("data_file", &["X_FILE", "stream/*"]);
        assert!(
            !left && how.contains("of the 2 file(s) it names, 2 are not stated"),
            "{how}"
        );
        assert!(!said("data_file", &["HANDLING_FILE", "../*/handling.meta"]).0);

        // ⚠ A pattern of many `*` on a name it does not fit ends: nothing is tried for every run of every one of them.
        let stars = "*a".repeat(40);
        let name = "a".repeat(80);
        assert!(names_are(&[stars.as_str()], &[name.as_str()]));
        assert!(!names_are(&[&format!("{stars}b")], &[name.as_str()]));
        let folders = vec!["**"; 40];
        let deep = vec!["x"; 80];
        assert!(names_are(&folders, &deep));
        assert!(!names_are(&[&folders[..], &["y"]].concat(), &deep));
        assert!(names_are(
            &["stream", "**", "*.ytyp"],
            &["stream", "a.ytyp"]
        ));
        assert!(names_are(&["*"], &["a.lua"]) && !names_are(&["*"], &["a", "b.lua"]));
        // Past the patterns a manifest is searched for, one is said to name no one file.
        for _ in 0..PATTERNS_AT_MOST {
            said("data_file", &["HANDLING_FILE", "data/*.meta"]);
        }
        let (past, why) = said("data_file", &["HANDLING_FILE", "data/*.meta"]);
        assert!(!past && why.contains("not searched for"), "{why}");
        assert!(said("data_file", &["HANDLING_FILE", "data/handling.meta"]).0);
        assert!(!said("data_file", &["HANDLING_FILE", "../other/handling.meta"]).0);
        assert!(!said("data_file", &["HANDLING_FILE", "data/gone.meta"]).0);
        assert!(!said("data_file", &["X_FILE", "stream/old.zip"]).0);
        let (carried, how) = said("client_script", &["client.lua"]);
        assert!(
            !carried && how.contains("client_script") && how.contains("by hand"),
            "{how}"
        );
    }
}
