//! What a module hands to an output: the contract that content plugs into a plan through.
//!
//! A contribution is data in and data out. A kind of content builds one for each of its
//! modules and the plan merges them, so the Files module is the first contributor and no
//! other path leads into a plan.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::paths::{is_component, is_under, relative_inside, BUILD_DIR};
use crate::plan::{Origin, PlannedFile, Presence, Source};

/// What one module hands to the output that takes it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Contribution {
    /// The id of the module it comes from.
    pub module: String,
    /// The files it copies and the documents it writes, at their paths inside the resource.
    pub files: Vec<PlannedFile>,
    /// Lines it adds to the resource's manifest.
    pub manifest: Vec<ManifestEntry>,
    /// Names the game resolves once, which no two contents may hold.
    pub claims: Vec<Claim>,
    /// Amounts of the named budgets that an output may declare a ceiling for.
    pub figures: Vec<Figure>,
    /// Things with a stable identity, which the diff of two builds follows.
    pub items: Vec<Item>,
    /// What is wrong with the module's own content, one sentence each.
    pub problems: Vec<String>,
    /// What a person is told about the build that does not stop it, one sentence each: a file it names and does not ship.
    pub notes: Vec<String>,
    /// Whether its paths are those of a singleplayer DLC pack, which an `.oiv` installs inside the `dlc.rpf` it adds.
    pub pack: bool,
    /// What a singleplayer DLC states that an archive its files go into holds, where the game's packs state it.
    pub archives: Vec<ArchiveContents>,
}

/// What a pack states a nested archive holds, such as `CONTENTS_DLC_MAP_DATA` for the archive of its maps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveContents {
    /// The archive, by its path in the pack.
    pub path: String,
    pub contents: String,
}

/// One line of the manifest: `key 'value' 'value'`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    pub key: String,
    pub values: Vec<String>,
}

/// A name of one kind that the game resolves once, such as a clothing collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub kind: String,
    pub name: String,
}

/// An amount of a named budget, such as the metadata entries a module adds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Figure {
    pub name: String,
    pub amount: u64,
}

/// A thing a build ships that keeps its `id` when it is renamed, renumbered or edited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Stable for as long as the thing exists, and never reused.
    pub id: String,
    /// What the game sees, which a rule may change when the thing moves.
    pub name: String,
    /// Equal for equal content, whatever the module hashes to say so.
    pub digest: String,
}

/// A `<name>.<kind>.src` folder of the project, as the caller read its `asset.json`.
///
/// This crate knows no kind of source: the caller says what a folder cooks to, and the cooker
/// it hands the build makes the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFolder {
    pub folder: PathBuf,
    /// What a build makes of the folder, or the one sentence that says why it cannot.
    pub cooks: Result<Cooks, String>,
}

/// What a source folder cooks to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cooks {
    /// The word that chooses the compiler, such as `textures`.
    pub kind: String,
    /// The asset's own name, which names the cooked file. The folder may be numbered and the name is not.
    pub name: String,
    /// The game extension the cooked file has, such as `ytd`: the file ships at `stream/<name>.<extension>`.
    pub extension: String,
    /// What the plan counts the file as: the payload it is made from, since the file is not made yet.
    pub bytes: u64,
    /// The SHA-256 of each file that is the game's own, which a cook that makes one ships nothing of; empty for any other source.
    pub game: Vec<String>,
}

impl Contribution {
    /// A contribution of `module` that hands over nothing.
    pub fn of(module: &str) -> Self {
        Self {
            module: module.to_string(),
            ..Self::default()
        }
    }

    /// What a Files module hands over: every source, at the path it has inside the project folder.
    pub fn files(module: &str, folder: &Path, sources: &[Source]) -> Self {
        let mut contribution = Self::of(module);
        for source in sources {
            contribution.place(source, folder);
        }
        contribution
    }

    /// Add each source folder a build cooks, at `stream/<name>.<extension>`; the folder itself never ships.
    ///
    /// A folder this build cannot cook, one in the build folder, and a second that cooks a file
    /// another already cooks are problems that name the folders, and only the first of two is planned.
    pub fn cooked(mut self, project: &Path, folders: &[SourceFolder]) -> Self {
        for found in folders {
            let shown = found.folder.display();
            let cooks = match &found.cooks {
                Ok(cooks) => cooks,
                Err(why) => {
                    self.problems.push(format!(
                        "`{shown}` is a source folder this build cannot cook: {why}"
                    ));
                    continue;
                }
            };
            let file = format!("{}.{}", cooks.name, cooks.extension);
            let in_builds = relative_inside(&found.folder, project)
                .is_some_and(|inside| is_under(&inside, BUILD_DIR));
            if in_builds {
                self.problems.push(format!(
                    "`{shown}` is in the build folder, which is never read back as content"
                ));
                continue;
            }
            if !is_component(&file) {
                self.problems.push(format!(
                    "`{shown}` cooks `{file}`, a name that Windows or another host refuses, so the resource could not be unpacked there"
                ));
                continue;
            }
            let path = format!("stream/{file}");
            let twin = self.files.iter().find_map(|f| match &f.origin {
                Origin::Cook { source, .. } if f.path.eq_ignore_ascii_case(&path) => Some(source),
                _ => None,
            });
            if let Some(first) = twin {
                self.problems.push(format!(
                    "`{}` and `{shown}` both cook `{path}`; keep one, or give one a name of its own in its asset.json",
                    first.display()
                ));
                continue;
            }
            self.files.push(PlannedFile {
                path,
                origin: Origin::Cook {
                    source: found.folder.clone(),
                    kind: cooks.kind.clone(),
                    name: cooks.name.clone(),
                    bytes: cooks.bytes,
                    game: cooks.game.clone(),
                },
            });
        }
        self
    }

    /// Add the types a Files content states for its files, as the `data_file` lines a manifest holds.
    ///
    /// `stated` is the `file_types` key of the content's entry: a map from a path inside the
    /// project to a type such as `HANDLING_FILE`. A FiveM resource lists them in its manifest and a
    /// singleplayer DLC registers each file with its type, which no extension but two settles.
    pub fn with_file_types(mut self, stated: Option<&Value>) -> Self {
        match stated {
            None | Some(Value::Null) => {}
            Some(Value::Object(types)) => {
                for (path, file_type) in types {
                    match file_type.as_str().filter(|text| is_type(text)) {
                        Some(file_type) => self.manifest.push(ManifestEntry {
                            key: "data_file".to_string(),
                            values: vec![file_type.to_string(), path.clone()],
                        }),
                        None => self.problems.push(format!(
                            "`file_types` gives `{path}` the type {file_type}, which is not a name such as HANDLING_FILE"
                        )),
                    }
                }
            }
            Some(v) => self.problems.push(format!(
                "`file_types` is {v}, not a map from a path to its type"
            )),
        }
        self
    }

    /// One source, copied to its path in the project or refused with the reason.
    fn place(&mut self, source: &Source, folder: &Path) {
        // ⚠ The name the project knows it by: a path of the person's disk says nothing a name in the project does not, and runs to a line of its own.
        let shown = relative_inside(&source.path, folder)
            .unwrap_or_else(|| source.path.display().to_string());
        let Presence::File { bytes } = source.presence else {
            let why = match source.presence {
                Presence::Folder => "a folder, and the Files content holds files",
                _ => "missing",
            };
            self.problems.push(format!("`{shown}` is {why}"));
            return;
        };
        match relative_inside(&source.path, folder) {
            None => self.problems.push(format!(
                "`{shown}` is outside the project folder, so its place in the resource is not defined: copy it into the folder, or link the folder it is in if that holds content"
            )),
            Some(path) if is_under(&path, BUILD_DIR) => self.problems.push(format!(
                "`{shown}` is in the build folder, which is never read back as content"
            )),
            Some(path) if !path.split('/').all(is_component) => self.problems.push(format!(
                "`{shown}` has a name that Windows or another host refuses, so the resource could not be unpacked there"
            )),
            Some(path) => self.files.push(PlannedFile {
                path,
                origin: Origin::Copy {
                    from: source.path.clone(),
                    bytes,
                },
            }),
        }
    }
}

/// Whether `text` is spelled as the game spells a data file's type: capitals, digits and underscores.
pub(crate) fn is_type(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}
