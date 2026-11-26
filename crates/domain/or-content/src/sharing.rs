//! Check before sharing: what a project folder holds that is not the person's to hand to another machine or a repository.
//!
//! The editor's Check before sharing and `openrage-cli project check-sharing` read the same things with the same words.

use std::path::{Path, PathBuf};

use or_project::check::{machine_paths, Why};
use or_project::paths::find_files;
use or_project::snapshot::PRIVATE_DIR;
use or_project::{find_sources, project_folder, relative_inside};
use or_source::asset::{AssetManifest, MANIFEST};

/// What a line of the check is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A path in the project file that another machine cannot use.
    MachinePath,
    /// A source whose content is the game's own file.
    GameContent,
    /// The `.openrage` folder: this project's history and the files only this machine can name.
    PrivateFolder,
    /// The project has changes the saved file does not hold, so the file read is not the one in the window.
    Unsaved,
    /// The search for sources stopped at its bounds.
    Bounded,
}

/// One thing to take out before the folder is sent, or to know about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Concern {
    pub kind: Kind,
    pub text: String,
}

/// Why a path in a file that is sent is no use to the machine it is sent to.
fn in_words(why: &Why) -> &'static str {
    match why {
        Why::Absolute => "a place on this machine, which another one does not have",
        Why::LeavesTheFolder => {
            "above the project folder, so a zip of the folder alone does not carry it"
        }
    }
}

/// Whether the source folder `dir` says in its manifest that it is the game's own file.
pub fn is_the_games(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join(MANIFEST))
        .ok()
        .and_then(|text| AssetManifest::parse(&text).ok())
        .is_some_and(|manifest| manifest.origin.from_game)
}

/// The source folders of the project at `project` that are the game's own files, and whether the search for sources stopped at its bounds.
pub fn game_sources(project: &Path) -> (Vec<PathBuf>, bool) {
    let (sources, bounded) = find_sources(&project_folder(project), &[]);
    (
        sources
            .into_iter()
            .filter(|dir| is_the_games(dir))
            .collect(),
        bounded,
    )
}

/// What sharing the folder of the project file `project` would hand over.
///
/// ⚠ The saved file is what gets sent, so it is the file that is read. `game_sources` are the source folders that are the game's own, and `packs` the
/// folders of the clothing packs the project writes, each of which keeps its earlier versions in a private folder of its own.
pub fn concerns(
    project: &Path,
    game_sources: &[PathBuf],
    packs: &[PathBuf],
    bounded: bool,
) -> Vec<Concern> {
    let folder = project_folder(project);
    let mut found = Vec::new();
    let mut say = |kind, text: String| found.push(Concern { kind, text });

    match std::fs::read(project)
        .map_err(|e| e.to_string())
        .and_then(|bytes| machine_paths(&bytes))
    {
        Ok(paths) => {
            for path in paths {
                say(
                    Kind::MachinePath,
                    format!("{} is {}: {}", path.at, path.path, in_words(&path.why)),
                );
            }
        }
        Err(why) => say(
            Kind::MachinePath,
            format!("The project file could not be read: {why}"),
        ),
    }
    // ⚠ A reference is a file of its own in a content's folder, and it travels with that content: a path of this machine in one is as much a path of this machine as one in the project file.
    let (references, _) = find_files(&folder, or_project::reference::SUFFIX);
    for file in references {
        let Some(inside) = relative_inside(&file, &folder) else {
            continue;
        };
        let paths = std::fs::read(&file)
            .map_err(|e| e.to_string())
            .and_then(|bytes| machine_paths(&bytes));
        match paths {
            Ok(paths) => {
                for path in paths {
                    say(
                        Kind::MachinePath,
                        format!(
                            "{inside} leads through {} ({}): {}",
                            path.path,
                            path.at,
                            in_words(&path.why)
                        ),
                    );
                }
            }
            Err(why) => say(
                Kind::MachinePath,
                format!("{inside} could not be read: {why}"),
            ),
        }
    }
    for source in game_sources {
        if let Some(inside) = relative_inside(source, &folder) {
            say(
                Kind::GameContent,
                format!("{inside} is the game's own file; leave it out of what you send"),
            );
        }
    }
    // ⚠ A maps content keeps manifests of the game whole (`or_maps::manifest::keep`): compiled files of the game that are no source,
    // so no `asset.json` says whose they are, and where they lie is what says it.
    let (manifests, _) = find_files(&folder, ".ymf");
    for file in manifests {
        let kept = (file.parent().and_then(Path::file_name))
            .is_some_and(|dir| dir == or_maps::manifest::KEPT);
        if let Some(inside) = relative_inside(&file, &folder).filter(|_| kept) {
            say(
                Kind::GameContent,
                format!(
                    "{inside} is a manifest of the game, kept whole; leave it out of what you send"
                ),
            );
        }
    }
    if folder.join(PRIVATE_DIR).is_dir() {
        say(
            Kind::PrivateFolder,
            format!(
                "{PRIVATE_DIR}/ keeps this project's history and what only this machine can name. Leave it out when you zip the folder; Git already ignores it."
            ),
        );
    }
    // ⚠ A clothing pack keeps its own private folder: its earlier versions travel with a zip of the folder.
    for pack in packs {
        let private = pack.join(PRIVATE_DIR);
        if *pack == folder || !private.is_dir() {
            continue;
        }
        if let Some(inside) = relative_inside(&private, &folder) {
            say(
                Kind::PrivateFolder,
                format!(
                    "{inside}/ keeps the earlier versions of this pack. Leave it out when you zip the folder; Git already ignores it."
                ),
            );
        }
    }
    if bounded {
        say(
            Kind::Bounded,
            "The search for source folders stopped at its bounds, so one further down may be missing from this list"
                .to_string(),
        );
    }
    found
}
