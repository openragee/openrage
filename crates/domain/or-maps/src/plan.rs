//! The planner: what a module of map sets hands to the plan of a build.
//!
//! [`plan`] is a pure function of the model and what the disk said of its sources. [`contribution`] reads both for a module of a
//! project and hands the result over as the [`Contribution`] the plan merges, so maps are one more kind of content going through the
//! one path: each source cooks to `stream/<name>.<extension>`, a type file is registered by the line that tells the game to read it,
//! the manifest of the maps is written beside them, and a line declares the resource a map. For a singleplayer DLC the same files go
//! into the nested archives the packs of the game keep theirs in ([`Shape::Pack`], `crate::layout`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use or_project::{
    ArchiveContents, Claim, Container, Contribution, Figure, Item, ManifestEntry, Module,
    ModuleKind, Origin, Output, OutputKind, PlannedFile,
};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::facts::{inspect, Depth, Facts, Role, SourceFact};
use crate::layout::{
    pack_archive, pack_path, pack_type_request, props_path, stream_path, MAP_DATA, MAP_LINE, PROPS,
    TYPE_FILE,
};
use crate::manifest;
use crate::model::{self, MapSet, Maps};
use crate::rules::{check, Finding, Known};

/// The kind of name the game resolves once that an archetype's is.
pub const ARCHETYPE_NAME: &str = or_project::server::ARCHETYPE_CLAIM;
/// The kind of name the game resolves once that a map file's is.
pub const MAP_FILE: &str = "map file";

/// A module planned.
#[derive(Debug, Clone)]
pub struct Planned {
    pub contribution: Contribution,
    pub findings: Vec<Finding>,
}

/// The sentence a finding that stops a build becomes in a plan's problems.
pub fn problem(finding: &Finding) -> String {
    format!("{}: {}", finding.code, finding.message)
}

/// Where the source `source` lands inside the resource.
pub fn place_of(source: &SourceFact) -> String {
    stream_path(&source.file_name())
}

/// What a module's files are laid out for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// A FiveM resource, and any build that takes the files where a resource has them: at `stream/`.
    Resource,
    /// A singleplayer DLC, and the pack an `.oiv` installs: the maps inside the nested archive the pack registers as map data, and each type file inside an archive of its own with its models.
    Pack,
}

impl Shape {
    /// The shape a build of `kind` takes a module's files in.
    pub fn of(kind: &OutputKind) -> Self {
        match kind {
            OutputKind::SingleplayerDlc | OutputKind::OivPackage => Self::Pack,
            _ => Self::Resource,
        }
    }

    /// Where a map file or the manifest called `file` of the content `module` lands.
    fn place(self, module: &str, file: &str) -> String {
        match self {
            Self::Resource => stream_path(file),
            Self::Pack => pack_path(module, file),
        }
    }
}

/// What `model` hands to the plan of module `module`, whose folder is `folder`, given what the disk said of its sources.
pub fn plan(model: &Maps, facts: &Facts, module: &str, folder: &Path, known: Known<'_>) -> Planned {
    plan_in(model, facts, module, folder, known, Shape::Resource)
}

/// [`plan`], with the files laid out for `shape`.
pub fn plan_in(
    model: &Maps,
    facts: &Facts,
    module: &str,
    folder: &Path,
    known: Known<'_>,
    shape: Shape,
) -> Planned {
    let findings = check(model, facts, known);
    let mut contribution = Contribution::of(module);
    contribution.pack = shape == Shape::Pack;
    contribution.problems = findings
        .iter()
        .filter(|f| f.blocks())
        .map(problem)
        .collect();

    let homes = match shape {
        Shape::Pack => props_homes(model, facts, module),
        Shape::Resource => BTreeMap::new(),
    };
    let mut shipped: Vec<&str> = Vec::new();
    let mut archetypes_claimed: Vec<u32> = Vec::new();
    let (mut entities, mut archetypes, mut interiors) = (0u64, 0u64, 0u64);
    let mut maps = 0u64;
    for set in &model.maps {
        for path in &set.sources {
            let Some(source) = facts.source(path) else {
                continue;
            };
            // A source several sets name ships once.
            if shipped.contains(&path.as_str()) {
                continue;
            }
            shipped.push(path);
            // ⚠ In a pack only a map file goes into the archive of maps: the packs of the game keep models out of it and ask for no type file inside it.
            let at = match homes.get(path.as_str()) {
                Some(home) if source.role() != Some(Role::Map) => {
                    props_path(home, &source.file_name())
                }
                _ => shape.place(module, &source.file_name()),
            };
            contribution.files.push(PlannedFile {
                path: at.clone(),
                origin: Origin::Cook {
                    source: folder.join(path.replace('/', std::path::MAIN_SEPARATOR_STR)),
                    kind: source.kind.clone(),
                    name: source.name.clone(),
                    bytes: source.bytes,
                    game: source.game.clone(),
                },
            });
            match source.role() {
                Some(Role::Types) => {
                    let interior = (source.types())
                        .is_some_and(|types| types.archetypes.iter().any(|a| a.interior.is_some()));
                    if shape == Shape::Pack && interior {
                        // ⚠ Never asked for: no pack of the game asks for a type file that declares an interior. A lock, so the
                        // cell of the build says what its plan says.
                        contribution.locks.push(format!(
                            "The type file {} declares an interior, and the game's own packs ask for no such type file by name: they state it in the manifest of its archive, in a list a build does not write yet, so a singleplayer DLC cannot ship it.",
                            source.file_name()
                        ));
                    } else {
                        // ⚠ A pack asks for a type file by an `.ityp` name of the folder its archive is in, never by the file inside the archive, and states props of it.
                        let asked = match shape {
                            Shape::Resource => at,
                            Shape::Pack => {
                                let asked = pack_type_request(&source.name);
                                contribution.archives.push(ArchiveContents {
                                    path: asked.clone(),
                                    contents: PROPS.to_string(),
                                });
                                asked
                            }
                        };
                        contribution.manifest.push(ManifestEntry {
                            key: "data_file".to_string(),
                            values: vec![TYPE_FILE.to_string(), asked],
                        });
                    }
                    if let Some(types) = source.types() {
                        for archetype in &types.archetypes {
                            archetypes += 1;
                            interiors += u64::from(archetype.interior.is_some());
                            if !archetypes_claimed.contains(&archetype.name.hash) {
                                archetypes_claimed.push(archetype.name.hash);
                                contribution.claims.push(Claim {
                                    kind: ARCHETYPE_NAME.to_string(),
                                    name: archetype.name.shown(),
                                });
                            }
                        }
                    }
                }
                Some(Role::Map) => {
                    maps += 1;
                    entities += source.map().map_or(0, |m| m.entities.len() as u64);
                    contribution.claims.push(Claim {
                        kind: MAP_FILE.to_string(),
                        name: source.name.clone(),
                    });
                }
                _ => {}
            }
        }
        contribution.items.push(item(set, facts));
    }
    let stated = manifest::of(model, facts);
    contribution
        .notes
        .extend((stated.notes.iter()).map(|note| format!("content `{module}`: {note}")));
    if let Some(bytes) = stated.file {
        contribution.files.push(PlannedFile {
            path: shape.place(module, manifest::FILE),
            origin: Origin::Generated(bytes),
        });
    }
    if maps > 0 {
        match shape {
            Shape::Resource => contribution.manifest.push(ManifestEntry {
                key: MAP_LINE.0.to_string(),
                values: vec![MAP_LINE.1.to_string()],
            }),
            Shape::Pack => contribution.archives.push(ArchiveContents {
                path: pack_archive(module),
                contents: MAP_DATA.to_string(),
            }),
        }
    }
    contribution.figures = vec![
        Figure {
            name: "map_sets".to_string(),
            amount: model.maps.len() as u64,
        },
        Figure {
            name: "map_files".to_string(),
            amount: maps,
        },
        Figure {
            name: "entities".to_string(),
            amount: entities,
        },
        Figure {
            name: "archetypes".to_string(),
            amount: archetypes,
        },
        Figure {
            name: "interiors".to_string(),
            amount: interiors,
        },
        Figure {
            name: "map_sources".to_string(),
            amount: shipped.len() as u64,
        },
    ];
    Planned {
        contribution,
        findings,
    }
}

/// The props archive of a pack each source of `model` that is no map goes into, by the source's path: a type file into the one named
/// for it, a model or a texture dictionary into that of the first type file with an archetype that names it, and what none names
/// into the one named for the content `module`.
fn props_homes<'a>(model: &'a Maps, facts: &Facts, module: &str) -> BTreeMap<&'a str, String> {
    let sources = || {
        (model.maps.iter())
            .flat_map(|set| set.sources.iter())
            .filter_map(|path| Some((path.as_str(), facts.source(path)?)))
    };
    // What the archetypes of each type file name: a model by its asset or its own name, a dictionary of drawables and one of textures.
    let mut named: Vec<(&str, BTreeSet<u32>)> = Vec::new();
    for (_, source) in sources() {
        let Some(types) = source.types() else {
            continue;
        };
        let mut names = BTreeSet::new();
        for archetype in &types.archetypes {
            names.insert(match archetype.asset_name.hash {
                0 => archetype.name.hash,
                asset => asset,
            });
            names.insert(archetype.drawable_dictionary.hash);
            names.insert(archetype.texture_dictionary.hash);
        }
        names.remove(&0);
        named.push((source.name.as_str(), names));
    }
    let mut homes = BTreeMap::new();
    for (path, source) in sources() {
        let home = match source.role() {
            Some(Role::Map) => continue,
            Some(Role::Types) => source.name.as_str(),
            _ => {
                let name = or_io::string_hash(source.name.as_bytes());
                (named.iter())
                    .find(|(_, names)| names.contains(&name))
                    .map_or(module, |(types, _)| *types)
            }
        };
        homes.insert(path, home.to_string());
    }
    homes
}

/// The item of a set: its id, its name and a digest of everything it ships.
fn item(set: &MapSet, facts: &Facts) -> Item {
    let sources: Vec<_> = set
        .sources
        .iter()
        .map(|path| match facts.source(path) {
            Some(source) => {
                json!({ "at": place_of(source), "content": source.digest.clone().unwrap_or_default() })
            }
            None => json!({ "at": path, "content": "" }),
        })
        .collect();
    let text = serde_json::to_string(&json!({ "name": set.name.trim(), "sources": sources }))
        .expect("a JSON value serialises");
    Item {
        id: set.id.clone(),
        name: set.name.trim().to_string(),
        digest: Sha256::digest(text.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Model(#[from] model::Error),
    #[error("the folder of content `{module}` is {why}")]
    Folder { module: String, why: String },
    #[error("{0}")]
    Linked(String),
}

/// The folder a maps module's content is in: inside `project_folder`, or where its link leads.
pub fn module_folder(project_folder: &Path, module: &Module) -> Result<PathBuf, Error> {
    use or_project::content::folder::{module_folder, FolderError};
    module_folder(project_folder, module, "a maps module").map_err(|e| match e {
        FolderError::Folder { module, why } => Error::Folder { module, why },
        FolderError::Linked(why) => Error::Linked(why),
    })
}

/// The folder a maps module is edited in: [`module_folder`], except that a link is refused.
///
/// ⚠ The project that holds a linked folder is the only one that writes it.
pub fn module_folder_to_edit(
    project: &Path,
    container: &Container,
    module: &Module,
) -> Result<PathBuf, Error> {
    let folder = module_folder(&or_project::project_folder(project), module)?;
    or_project::links::refuse_write(container, project, &folder.join(model::FILE))
        .map_err(Error::Linked)?;
    Ok(folder)
}

/// A module read from its folder in the project in `project_folder`, and planned.
pub fn planned(
    project_folder: &Path,
    module: &Module,
    depth: Depth,
    known: Known<'_>,
) -> Result<(Maps, Facts, Planned), Error> {
    planned_in(project_folder, module, depth, known, Shape::Resource)
}

/// [`planned`], with the files laid out for `shape`.
pub fn planned_in(
    project_folder: &Path,
    module: &Module,
    depth: Depth,
    known: Known<'_>,
    shape: Shape,
) -> Result<(Maps, Facts, Planned), Error> {
    let folder = module_folder(project_folder, module)?;
    let model = Maps::read(&folder)?;
    let facts = inspect(&folder, &model, depth);
    let planned = plan_in(&model, &facts, &module.id, &folder, known, shape);
    Ok((model, facts, planned))
}

/// What module `module` hands over; a module whose folder or `maps.json` cannot be read hands over the reason.
pub fn contribution(project_folder: &Path, module: &Module) -> Contribution {
    contribution_in(project_folder, module, Shape::Resource)
}

/// What module `module` hands to the build `output`: its files where a build of that kind has them.
pub fn contribution_for(project_folder: &Path, module: &Module, output: &Output) -> Contribution {
    contribution_in(project_folder, module, Shape::of(&output.kind))
}

fn contribution_in(project_folder: &Path, module: &Module, shape: Shape) -> Contribution {
    match planned_in(project_folder, module, Depth::Open, None, shape) {
        Ok((.., planned)) => planned.contribution,
        Err(e) => {
            let mut contribution = Contribution::of(&module.id);
            contribution.problems.push(e.to_string());
            contribution
        }
    }
}

/// What every maps module of `container` hands over.
pub fn contributions(container: &Container, project_folder: &Path) -> Vec<Contribution> {
    container
        .modules
        .iter()
        .filter(|m| m.kind == ModuleKind::Maps)
        .map(|m| contribution(project_folder, m))
        .collect()
}
