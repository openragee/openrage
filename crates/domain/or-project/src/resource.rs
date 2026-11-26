//! A resource folder checked as a server will read it, whoever made it: its manifest, the files the manifest names and every file under its stream folder.
//!
//! A rule is a row of [`RULES`] and a finding is one place a rule applies, with the file it is about. The check writes nothing, runs
//! nothing of the manifest, and opens a streamed file through the reader a build reloads its own files through.
//!
//! ⚠ It reads the files and not the game: nothing here says that a server starts the resource or that GTA V loads a file of it. A
//! name the resource does not hold may be the game's or another resource's, so those findings are never errors.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use or_io::container::resource::{ResourceEnvelope, Rsc7};
use or_io::decl::File;
use or_io::meta::instance::HashedString;
use or_io::meta::typed::{CBaseArchetypeDefAny, CMapData, CMapTypes};
use serde::Serialize;

use crate::content::rule::{Rule, Severity};

pub mod manifest;

use manifest::{is_glob, Manifest};

/// The folder of a resource whose files a server streams, with every folder inside it.
pub const STREAM_DIR: &str = "stream";

/// The bytes a file locked by a server's asset escrow opens with, in place of the header of the file it holds.
pub const ESCROW_MARKER: &[u8; 4] = b"FXAP";

/// The `limits` entry that holds the ceiling for the memory one streamed RSC7 file takes; no file is weighed while none is stated.
pub const FILE_MEMORY_LIMIT: &str = "file_memory";

/// The extensions a server streams: the paged resources the tree declares, then maps, type files, metadata, manifests, scripts, movies, cutscenes and pose matchers.
// ⚠ Read from no server: a file of another extension is reported as a warning and never as an error.
pub const STREAMED: &[&str] = &[
    "ybd", "ybn", "ycd", "ydd", "ydr", "yed", "yfd", "yft", "yld", "ynd", "ynv", "ypt", "ytd",
    "yvr", "ywr", "ymap", "ytyp", "ymt", "ymf", "ysc", "gfx", "cut", "ypdb",
];

/// The extensions of a model an archetype names as its asset or its drawable dictionary.
const MODELS: [&str; 3] = ["ydr", "yft", "ydd"];

/// How many names a finding about several lists before it counts the rest.
const NAMED: usize = 8;

const fn rule(code: &'static str, severity: Severity, summary: &'static str) -> Rule {
    Rule::new(code, severity, false, summary)
}

use Severity::{Error, Info, Warning};

/// Every rule, in the order a report lists them.
pub const RULES: &[Rule] = &[
    rule(
        "manifest-missing",
        Error,
        "the folder has no manifest, so a server does not start it as a resource",
    ),
    rule(
        "version-missing",
        Error,
        "the manifest has no version line, without which a server rejects the resource",
    ),
    rule(
        "manifest-line-unread",
        Info,
        "lines of the manifest are not statements this check reads, so what they state is not known",
    ),
    rule(
        "path-escapes",
        Warning,
        "the manifest names a file by a path that leaves the resource, which is not followed",
    ),
    rule(
        "file-missing",
        Error,
        "a file the manifest lists, or one of its data-file lines names, is not in the resource",
    ),
    rule(
        "file-twice",
        Warning,
        "two files under the stream folder have one name, and a server loads one of them",
    ),
    rule(
        "file-not-streamed",
        Warning,
        "a file under the stream folder is of a type a server does not stream",
    ),
    rule(
        "file-locked",
        Info,
        "a file is locked by a server's asset escrow, so it is counted and not opened",
    ),
    rule(
        "file-unreadable",
        Error,
        "a streamed file does not open through the reader of its type",
    ),
    rule(
        "file-oversized",
        Warning,
        "a streamed file takes more memory than the limit stated for one file",
    ),
    rule(
        "map-not-declared",
        Warning,
        "the resource streams maps and its manifest does not declare it a map",
    ),
    rule(
        "model-missing",
        Warning,
        "an archetype of a type file names a model that neither the resource nor the game holds",
    ),
    rule(
        "archetype-without-model",
        Info,
        "an archetype of a type file names a model the resource does not hold, so the game or another resource has to",
    ),
    rule(
        "archetype-missing",
        Warning,
        "a map places an archetype that no type file of the resource declares and the game does not know",
    ),
    rule(
        "archetype-undeclared",
        Info,
        "a map places an archetype that no type file of the resource declares, so the game or another resource has to",
    ),
    // ⚠ A note and no warning: asked of the game's own type files it fires on more than half of them, so it says of a resource what
    // the game says of itself (`how_many_texture_dictionaries_the_type_files_of_an_install_name_and_no_file_holds`).
    rule(
        "texture-dictionary-missing",
        Info,
        "an archetype of a type file names a texture dictionary that neither the resource nor the game holds, as more than half the type files of the game do",
    ),
    rule(
        "texture-dictionary-elsewhere",
        Info,
        "an archetype of a type file names a texture dictionary the resource does not hold, so the game or another resource has to",
    ),
];

pub fn rule_of(code: &str) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.code == code)
}

/// The texture dictionaries the archetypes of `types` name that `held` says nothing holds, each as its archetype names it.
pub fn unheld_texture_dictionaries(
    types: &CMapTypes,
    held: impl Fn(u32) -> bool,
) -> Vec<&HashedString> {
    (types.archetypes.iter())
        .map(|archetype| &archetype.texture_dictionary)
        .filter(|textures| textures.hash != 0 && !held(textures.hash))
        .collect()
}

/// What an install is asked about a name the resource does not hold, each by the hash the game keys it with.
pub trait Install {
    /// Whether a type file of the game declares the archetype.
    fn declares_archetype(&self, name: u32) -> bool;
    /// Whether the game holds a drawable, a fragment or a drawable dictionary of the name.
    fn holds_model(&self, name: u32) -> bool;
    /// Whether the game holds a texture dictionary of the name.
    fn holds_texture_dictionary(&self, name: u32) -> bool;
}

/// What a check is given beside the folder.
#[derive(Default)]
pub struct Options<'a> {
    /// The ceilings a person states, by name, as a build's `limits` are; [`FILE_MEMORY_LIMIT`] is the one read here.
    pub limits: BTreeMap<String, u64>,
    /// The install asked about a name the resource does not hold; none when no install was given.
    pub install: Option<&'a dyn Install>,
}

/// One place a rule applies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub code: &'static str,
    pub severity: Severity,
    /// The file it is about, inside the resource with `/`; none when it is about the folder.
    pub file: Option<String>,
    pub message: String,
}

/// What the check read of a resource folder, and what the rules found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    pub folder: PathBuf,
    /// The manifest read, by its name; none when the folder has none.
    pub manifest: Option<String>,
    /// The value of the manifest's version line.
    pub version: Option<String>,
    /// The games the manifest names.
    pub games: Vec<String>,
    /// Whether the manifest declares the resource a map.
    pub map: bool,
    /// The files under the stream folder.
    pub streamed: usize,
    /// The streamed files a reader opened.
    pub opened: usize,
    /// The streamed files locked by an asset escrow, which were not opened.
    pub locked: usize,
    pub findings: Vec<Finding>,
}

/// What a person reads for a severity.
pub fn word(severity: Severity) -> &'static str {
    match severity {
        Error => "error",
        Warning => "warning",
        Info => "note",
    }
}

impl Report {
    /// Whether a rule found an error, which a command answers with a failing exit code.
    pub fn has_error(&self) -> bool {
        self.count(Error) > 0
    }

    pub fn count(&self, severity: Severity) -> usize {
        (self.findings.iter())
            .filter(|f| f.severity == severity)
            .count()
    }

    /// The line of the text form that states `finding`.
    pub fn line_of(finding: &Finding) -> String {
        let at = (finding.file.as_deref()).map_or_else(String::new, |file| format!("{file}: "));
        format!(
            "  {:<7} {:<28} {at}{}",
            word(finding.severity),
            finding.code,
            finding.message
        )
    }

    /// The text form: what was read, one line for each finding in the order of [`Report::findings`], the counts, and what the check cannot see.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![format!("resource {}", self.folder.display())];
        lines.push(match &self.manifest {
            Some(name) => format!(
                "  manifest {name}: version {}, game {}, {}",
                self.version.as_deref().unwrap_or("not stated"),
                match self.games.as_slice() {
                    [] => "not stated".to_string(),
                    games => games.join(" and "),
                },
                if self.map {
                    "declared a map"
                } else {
                    "not declared a map"
                }
            ),
            None => "  manifest: none".to_string(),
        });
        lines.push(format!(
            "  streamed: {} file(s), {} opened by a reader, {} locked by an asset escrow and not opened",
            self.streamed, self.opened, self.locked
        ));
        lines.extend(self.findings.iter().map(Self::line_of));
        lines.push(format!(
            "{} error(s), {} warning(s), {} note(s)",
            self.count(Error),
            self.count(Warning),
            self.count(Info)
        ));
        lines.push(
            "This reads the files as a server finds them. It cannot see whether the game loads a file."
                .to_string(),
        );
        lines
    }
}

/// A file under the stream folder.
pub(crate) struct Streamed {
    /// Inside the resource, with `/`.
    pub(crate) path: String,
    pub(crate) at: PathBuf,
    /// The file's name in lower case, which is what a server keys a streamed file by.
    pub(crate) name: String,
    pub(crate) extension: String,
    /// The hash of the name without its extension, as an archetype names a model or a dictionary.
    stem: u32,
}

/// What the type files and the maps of a resource state by name.
#[derive(Default)]
struct Named {
    /// A type file was locked or did not open, so the archetypes the resource declares are not all known.
    types_unknown: bool,
    types: Vec<(String, CMapTypes)>,
    /// Each map by the archetypes it places: a resource may stream thousands of maps, and nothing else of one is asked.
    maps: Vec<(String, Vec<HashedString>)>,
}

/// Read `folder` as a server would and apply every rule.
pub fn check(folder: &Path, options: &Options<'_>) -> Report {
    let mut report = Report {
        folder: folder.to_path_buf(),
        ..Report::default()
    };
    let (streamed, unread) = stream_files(folder);
    report.streamed = streamed.len();
    for (path, why) in unread {
        report.say(
            "file-unreadable",
            Some(&path),
            format!("the folder cannot be read, so the files in it are not known: {why}"),
        );
    }

    let manifest = manifest::NAMES
        .into_iter()
        .find(|name| folder.join(name).is_file());
    match manifest {
        None => report.say(
            "manifest-missing",
            None,
            format!(
                "the folder holds no `{}` and no `{}`",
                manifest::NAMES[0],
                manifest::NAMES[1]
            ),
        ),
        Some(name) => {
            let has_maps = streamed.iter().any(|f| f.extension == "ymap");
            report.read_manifest(folder, name, has_maps);
        }
    }

    for held in twice(&streamed) {
        let paths: Vec<&str> = held.iter().map(|f| f.path.as_str()).collect();
        report.say(
            "file-twice",
            Some(paths[0]),
            format!(
                "`{}` is under the stream folder {} times, and a server loads one of them: `{}`",
                held[0].name,
                held.len(),
                paths.join("`, `")
            ),
        );
    }

    let limit = options.limits.get(FILE_MEMORY_LIMIT).copied();
    let mut named = Named::default();
    for file in &streamed {
        report.open(file, limit, &mut named);
    }
    report.names(&streamed, &named, options.install);

    let order = |code: &str| RULES.iter().position(|r| r.code == code);
    report.findings.sort_by_key(|f| order(f.code));
    report
}

impl Report {
    fn say(&mut self, code: &'static str, file: Option<&str>, message: String) {
        self.findings.push(Finding {
            code,
            severity: rule_of(code).map_or(Error, |r| r.severity),
            file: file.map(str::to_string),
            message,
        });
    }

    /// Read the manifest called `name` and say what it lacks and which of the files it names are not there.
    fn read_manifest(&mut self, folder: &Path, name: &'static str, has_maps: bool) {
        self.manifest = Some(name.to_string());
        let text = match std::fs::read(folder.join(name)) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(e) => {
                self.say(
                    "manifest-line-unread",
                    Some(name),
                    format!("the manifest cannot be read, so nothing it states is known: {e}"),
                );
                return;
            }
        };
        let manifest = Manifest::read(&text);
        self.version = manifest.version().map(str::to_string);
        self.games = manifest.games().into_iter().map(str::to_string).collect();
        self.games.dedup();
        self.map = manifest.is_map();
        for &(first, last) in &manifest.unread {
            let lines = if first == last {
                format!("line {first} is")
            } else {
                format!("lines {first} to {last} are")
            };
            self.say(
                "manifest-line-unread",
                Some(name),
                format!("{lines} not read: this check reads a key followed by text or by a table of text, and runs nothing"),
            );
        }
        // ⚠ That a line is absent is known only of a manifest read whole: an unread line may be the one.
        if manifest.unread.is_empty() {
            if self.version.is_none() {
                self.say(
                    "version-missing",
                    Some(name),
                    "the manifest states no `fx_version` and no `resource_manifest_version`"
                        .to_string(),
                );
            }
            if has_maps && !self.map {
                self.say(
                    "map-not-declared",
                    Some(name),
                    "the resource streams map files and the manifest has no `this_is_a_map` line"
                        .to_string(),
                );
            }
        }
        for (line, stated) in manifest.data_files_unread() {
            self.say(
                "manifest-line-unread",
                Some(name),
                format!("line {line} is not read: a data-file line states a type and a path, and this one states {stated} text(s)"),
            );
        }
        let listed = (manifest.files().into_iter()).map(|(line, path)| (line, path, false));
        let data = (manifest.data_files().into_iter()).map(|d| (d.line, d.path, true));
        for (line, path, data_file) in listed.chain(data) {
            let what = if data_file {
                "a data-file line names"
            } else {
                "the manifest lists"
            };
            match inside(path) {
                None => self.say(
                    "path-escapes",
                    Some(name),
                    format!("line {line}: {what} `{path}`, which leaves the resource and is not followed"),
                ),
                // A pattern names no one file whose absence is known.
                Some(_) if is_glob(path) => {}
                Some(rel) if !is_there(&folder.join(&rel), data_file) => self.say(
                    "file-missing",
                    Some(&rel.to_string_lossy().replace('\\', "/")),
                    format!("line {line} of `{name}`: {what} `{path}`, which is not in the resource"),
                ),
                Some(_) => {}
            }
        }
    }

    /// Open one streamed file through the reader of its type, unless a server does not stream it or an escrow locks it.
    fn open(&mut self, file: &Streamed, limit: Option<u64>, named: &mut Named) {
        let at = Some(file.path.as_str());
        if !STREAMED.contains(&file.extension.as_str()) {
            let what = match file.extension.as_str() {
                "" => "a file with no extension".to_string(),
                extension => format!("a `.{extension}` file"),
            };
            self.say(
                "file-not-streamed",
                at,
                format!(
                    "{what} is under the stream folder, and a server does not stream that type"
                ),
            );
            return;
        }
        let types = file.extension == "ytyp";
        match head(&file.at) {
            Ok(head) if head == *ESCROW_MARKER => {
                self.locked += 1;
                named.types_unknown |= types;
                self.say(
                    "file-locked",
                    at,
                    "locked by a server's asset escrow: counted, and not opened".to_string(),
                );
                return;
            }
            Ok(_) => {}
            Err(e) => {
                named.types_unknown |= types;
                self.say("file-unreadable", at, format!("cannot be read: {e}"));
                return;
            }
        }
        let bytes = match std::fs::read(&file.at) {
            Ok(bytes) => bytes,
            Err(e) => {
                named.types_unknown |= types;
                self.say("file-unreadable", at, format!("cannot be read: {e}"));
                return;
            }
        };
        if let (Some(limit), Ok(seal)) = (limit, Rsc7::peek(&bytes)) {
            let memory = u64::from(seal.total_size());
            if memory > limit {
                self.say(
                    "file-oversized",
                    at,
                    format!("takes {memory} bytes of streaming memory by its own header, over the {limit} that `{FILE_MEMORY_LIMIT}` allows one file"),
                );
            }
        }
        // ⚠ The readers of metadata also take the text a source is kept as, which no game streams.
        if matches!(file.extension.as_str(), "ymap" | "ytyp" | "ymt")
            && or_io::meta::detect_binary_format(&bytes).is_none()
        {
            named.types_unknown |= types;
            self.say(
                "file-unreadable",
                at,
                "is not metadata in a binary form, which is the only form of it the game streams"
                    .to_string(),
            );
            return;
        }
        let opened = match file.extension.as_str() {
            "ymap" => CMapData::from_bytes(&bytes)
                .map(|map| {
                    let placed = map.entities.iter().map(|e| e.archetype_name.clone());
                    named.maps.push((file.path.clone(), placed.collect()));
                })
                .map_err(|e| e.to_string()),
            "ytyp" => CMapTypes::from_bytes(&bytes)
                .map(|types| named.types.push((file.path.clone(), types)))
                .map_err(|e| e.to_string()),
            _ => match crate::readers::reload(&file.name, &bytes) {
                Some(read) => read.map(drop),
                // A type nothing here reads is streamed and not opened.
                None => return,
            },
        };
        match opened {
            Ok(()) => self.opened += 1,
            Err(why) => {
                named.types_unknown |= types;
                self.say("file-unreadable", at, format!("does not open: {why}"));
            }
        }
    }

    /// Say which names the type files and the maps state that nothing holds.
    fn names(&mut self, streamed: &[Streamed], named: &Named, install: Option<&dyn Install>) {
        let stems = |extensions: &[&str]| -> BTreeSet<u32> {
            (streamed.iter())
                .filter(|f| extensions.contains(&f.extension.as_str()))
                .map(|f| f.stem)
                .collect()
        };
        let models = stems(&MODELS);
        let dictionaries = stems(&["ytd"]);
        let mut declared: BTreeSet<u32> = BTreeSet::new();
        for (path, types) in &named.types {
            let mut without_model = Vec::new();
            for archetype in &types.archetypes {
                declared.insert(archetype.name.hash);
                // An interior is drawn by the entities it holds and has no model of its own.
                let interior = matches!(archetype, CBaseArchetypeDefAny::CMloArchetypeDef(_));
                let asset = match archetype.asset_name.hash {
                    0 => &archetype.name,
                    _ => &archetype.asset_name,
                };
                let dictionary = archetype.drawable_dictionary.hash;
                let held = |name: u32| {
                    models.contains(&name) || install.is_some_and(|game| game.holds_model(name))
                };
                if !interior && !held(asset.hash) && !(dictionary != 0 && held(dictionary)) {
                    without_model.push(shown(asset));
                }
            }
            let held = |name: u32| {
                dictionaries.contains(&name)
                    || install.is_some_and(|game| game.holds_texture_dictionary(name))
            };
            let without_textures: Vec<String> = (unheld_texture_dictionaries(types, held).iter())
                .map(|name| shown(name))
                .collect();
            let (code, tail) = match install {
                Some(_) => ("model-missing", "and the game holds none either"),
                None => (
                    "archetype-without-model",
                    "and no install was given to ask the game",
                ),
            };
            self.list(
                code,
                path,
                without_model,
                "archetype(s) name a model the resource does not hold",
                tail,
            );
            let (code, tail) = match install {
                Some(_) => (
                    "texture-dictionary-missing",
                    "and the game holds none either",
                ),
                None => (
                    "texture-dictionary-elsewhere",
                    "and no install was given to ask the game",
                ),
            };
            self.list(
                code,
                path,
                without_textures,
                "texture dictionary(ies) its archetypes name are not in the resource",
                tail,
            );
        }
        // ⚠ What the resource declares is known only when every type file of it opened.
        if named.types_unknown {
            return;
        }
        for (path, placed) in &named.maps {
            let placed: Vec<String> = (placed.iter())
                .filter(|name| !declared.contains(&name.hash))
                .filter(|name| !install.is_some_and(|game| game.declares_archetype(name.hash)))
                .map(shown)
                .collect();
            let (code, tail) = match install {
                Some(_) => ("archetype-missing", "and the game does not know them"),
                None => (
                    "archetype-undeclared",
                    "and no install was given to ask the game",
                ),
            };
            self.list(
                code,
                path,
                placed,
                "archetype(s) it places are declared by no type file of the resource",
                tail,
            );
        }
    }

    /// One finding for `file` that counts `names`, each once, and lists the first of them.
    fn list(&mut self, code: &'static str, file: &str, names: Vec<String>, what: &str, tail: &str) {
        let names: BTreeSet<String> = names.into_iter().collect();
        if names.is_empty() {
            return;
        }
        let listed: Vec<&str> = names.iter().take(NAMED).map(String::as_str).collect();
        let more = match names.len() - listed.len() {
            0 => String::new(),
            more => format!(" and {more} more"),
        };
        self.say(
            code,
            Some(file),
            format!(
                "{} {what}, {tail}: `{}`{more}",
                names.len(),
                listed.join("`, `")
            ),
        );
    }
}

/// A name as a person reads it: its text where the file keeps one that hashes to it, else the hash as a source's XML spells an unknown name.
pub(crate) fn shown(name: &HashedString) -> String {
    if !name.raw.is_empty() && or_io::string_hash(name.raw.as_bytes()) == name.hash {
        name.raw.clone()
    } else {
        name.to_literal()
    }
}

/// Every file under the stream folder of `folder`, in path order, and each folder under it that could not be read with the reason; a
/// hidden entry and a link are left out, so the walk ends.
pub(crate) fn stream_files(folder: &Path) -> (Vec<Streamed>, Vec<(String, std::io::Error)>) {
    let mut found = Vec::new();
    let mut unread = Vec::new();
    let root = folder.join(STREAM_DIR);
    // A resource with no stream folder streams nothing, which is no finding; neither does one whose stream folder is a link.
    let is_folder = std::fs::symlink_metadata(&root).is_ok_and(|found| found.is_dir());
    let mut stack = Vec::from_iter(is_folder.then_some(root));
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(why) => {
                let path = crate::paths::relative_inside(&dir, folder);
                unread.push((path.unwrap_or_else(|| STREAM_DIR.to_string()), why));
                continue;
            }
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            if kind.is_dir() {
                stack.push(entry.path());
            } else if kind.is_file() {
                let Some(path) = crate::paths::relative_inside(&entry.path(), folder) else {
                    continue;
                };
                let name = name.to_lowercase();
                let (stem, extension) = match name.rsplit_once('.') {
                    Some((stem, extension)) => (stem, extension.to_string()),
                    None => (name.as_str(), String::new()),
                };
                found.push(Streamed {
                    path,
                    at: entry.path(),
                    stem: or_io::string_hash(stem.as_bytes()),
                    extension,
                    name,
                });
            }
        }
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    unread.sort_by(|a, b| a.0.cmp(&b.0));
    (found, unread)
}

/// The streamed files that share a name, each group in path order.
fn twice(streamed: &[Streamed]) -> Vec<Vec<&Streamed>> {
    let mut by_name: BTreeMap<&str, Vec<&Streamed>> = BTreeMap::new();
    for file in streamed {
        by_name.entry(&file.name).or_default().push(file);
    }
    (by_name.into_values())
        .filter(|held| held.len() > 1)
        .collect()
}

/// The first bytes of a file, as many as the escrow marker has; fewer when the file is shorter.
fn head(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut head = Vec::with_capacity(ESCROW_MARKER.len());
    std::fs::File::open(path)?
        .take(ESCROW_MARKER.len() as u64)
        .read_to_end(&mut head)?;
    Ok(head)
}

/// A path a manifest states, as the path inside the resource it names; none when it is absolute or climbs out.
pub(crate) fn inside(stated: &str) -> Option<PathBuf> {
    let stated = stated.replace('\\', "/");
    // A drive letter is a path of its own on Windows and an ordinary name anywhere else.
    if stated.as_bytes().get(1) == Some(&b':') {
        return None;
    }
    let mut inside = PathBuf::new();
    for component in Path::new(&stated).components() {
        match component {
            Component::Normal(part) => inside.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                if !inside.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(inside)
}

/// Whether the file a manifest names is in the resource.
///
/// ⚠ A data-file line may name a folder, and an audio line names `x.dat` for a file called `x.dat<number>.rel`.
fn is_there(at: &Path, data_file: bool) -> bool {
    if at.is_file() {
        return true;
    }
    if !data_file {
        return false;
    }
    if at.is_dir() {
        return true;
    }
    let Some(leaf) = at.file_name().map(|n| n.to_string_lossy().to_lowercase()) else {
        return false;
    };
    leaf.ends_with(".dat")
        && (at.parent().into_iter())
            .flat_map(std::fs::read_dir)
            .flatten()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().to_lowercase())
            .any(|name| name.starts_with(&leaf) && name.ends_with(".rel"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rule_has_one_code_and_a_sentence() {
        for (index, rule) in RULES.iter().enumerate() {
            assert!(!rule.summary.is_empty(), "{}", rule.code);
            assert!(
                RULES[..index].iter().all(|r| r.code != rule.code),
                "{} is a rule twice",
                rule.code
            );
            assert!(
                !rule.waivable,
                "{}: a check of a folder keeps no waiver",
                rule.code
            );
        }
    }

    #[test]
    fn every_paged_resource_the_tree_declares_is_a_type_a_server_streams() {
        for format in or_io::gta5::formats::FORMATS {
            assert!(
                STREAMED.contains(&format.ext),
                ".{} is not listed",
                format.ext
            );
        }
        for model in MODELS {
            assert!(STREAMED.contains(&model), ".{model}");
        }
    }

    #[test]
    fn a_stated_path_is_inside_the_resource_or_is_not_followed() {
        let inside_of =
            |stated: &str| inside(stated).map(|p| p.to_string_lossy().replace('\\', "/"));
        assert_eq!(inside_of("data/a.meta").as_deref(), Some("data/a.meta"));
        assert_eq!(inside_of("./data\\a.meta").as_deref(), Some("data/a.meta"));
        assert_eq!(inside_of("data/../a.meta").as_deref(), Some("a.meta"));
        for out in [
            "../a.meta",
            "data/../../a.meta",
            "/etc/passwd",
            "C:\\Windows\\win.ini",
            "c:/x",
            "\\\\server\\share\\x",
        ] {
            assert_eq!(inside(out), None, "{out}");
        }
    }
}
