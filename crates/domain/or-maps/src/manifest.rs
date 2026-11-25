//! The manifest of a pack's maps: the `_manifest.ymf` that tells the game which type files each map needs before it loads.
//!
//! The lists are `or-world`'s derivation over what the sources place and declare, and the document is written by
//! `or_world::manifest_build` from the manifests of the game the module keeps in its `manifests` folder. A manifest is not made from
//! nothing: the tree declares none of its classes and derives no flags for an entry, so those come from a kept manifest, and with none
//! kept, or for a map none of them states, nothing is written and the plan says so by name. A map taken from the game keeps the entry
//! the game states for it, because the derivation and the shipped manifests agree for most maps and not all. A map that is not the
//! game's and places an archetype no type file of the content declares keeps the type files the game states for a map of its name
//! beside the ones found, since the derivation sees the content's own type files and no install.
//!
//! ⚠ A kept manifest is the game's own file, as a source's base file is: it is in the person's project and never in this repository.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use or_world::manifest::{dependencies_of_placed, resident_type_files};
use or_world::manifest_build::{self, BuildError, Wanted};
use sha2::{Digest, Sha256};

use crate::facts::{named_paths, Facts, Name, Role};
use crate::model::Maps;

/// The name the game reads a manifest by.
pub const FILE: &str = "_manifest.ymf";
/// The folder of a module that keeps the manifests of the game.
pub const KEPT: &str = "manifests";

/// A manifest of the game a module keeps: its file in [`KEPT`], and its bytes or why they cannot be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    pub file: String,
    pub bytes: Result<Vec<u8>, String>,
}

/// The manifests `folder` keeps, in the order of their names on every host.
pub fn kept(folder: &Path) -> Vec<Kept> {
    let mut names: Vec<String> = std::fs::read_dir(folder.join(KEPT))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.to_ascii_lowercase().ends_with(".ymf"))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|file| Kept {
            bytes: std::fs::read(folder.join(KEPT).join(&file)).map_err(|e| e.to_string()),
            file,
        })
        .collect()
}

/// Keep `bytes`, a manifest of the game, in the module at `folder`, and return its file; one that is no manifest is refused with the reason.
pub fn keep(folder: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    manifest_build::stated_in(bytes).map_err(|e| e.to_string())?;
    // The file is named for its content, so a manifest kept twice is kept once.
    let digest: String = Sha256::digest(bytes)
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect();
    let dir = folder.join(KEPT);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(format!("{digest}.ymf"));
    if !path.is_file() {
        or_project::replace_file(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(path)
}

/// What the maps of a module come to as a manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The document, none when nothing could be written.
    pub file: Option<Vec<u8>>,
    /// The type files it states for each map it states, by the hash of each name.
    pub stated: BTreeMap<u32, BTreeSet<u32>>,
    /// What a person is told: why there is no document, a map it states nothing for, a map whose kept entry lacks a type file it places.
    pub notes: Vec<String>,
}

/// The manifest of the maps `model` ships, from what `facts` says its sources place and declare and the manifests it keeps.
pub fn of(model: &Maps, facts: &Facts) -> Outcome {
    let sources: Vec<_> = named_paths(model)
        .filter_map(|path| facts.source(path))
        .collect();
    // ⚠ Later wins, as a later type file does in the game: the order is the order the sets name their sources in.
    let mut owner: BTreeMap<u32, u32> = BTreeMap::new();
    let mut ours: BTreeMap<u32, Name> = BTreeMap::new();
    for source in &sources {
        let Some(types) = source.types().filter(|types| !types.name.is_empty()) else {
            continue;
        };
        for archetype in &types.archetypes {
            owner.insert(archetype.name.hash, types.name.hash);
        }
        // A type file taken from the game is one the game's own manifests could state.
        if source.game.is_empty() {
            ours.insert(types.name.hash, named(&types.name, &source.name));
        }
    }
    let resident = resident_type_files();
    let mut names: BTreeMap<u32, Name> = BTreeMap::new();
    let mut wanted = Vec::new();
    for source in &sources {
        if source.role() != Some(Role::Map) {
            continue;
        }
        let Some(map) = source.map().filter(|map| !map.name.is_empty()) else {
            continue;
        };
        let derived =
            dependencies_of_placed(&map.placed, |archetype| owner.get(&archetype).copied());
        names.insert(map.name.hash, named(&map.name, &source.name));
        wanted.push(Wanted {
            name: map.name.hash,
            keep_stated: !source.game.is_empty(),
            needs: derived.types.difference(&resident).copied().collect(),
            // ⚠ A plan has no install, so an archetype of the game is one no type file here declares: the list is then short, and the writer adds it to the stated one.
            incomplete: derived.unresolved > 0,
        });
    }
    if wanted.is_empty() {
        return Outcome::default();
    }

    let mut outcome = Outcome::default();
    let mut bytes: Vec<&[u8]> = Vec::new();
    for kept in &facts.manifests {
        match &kept.bytes {
            Ok(held) => bytes.push(held),
            Err(why) => {
                outcome.notes.push(format!(
                    "no `{FILE}` is written: the kept manifest `{KEPT}/{}` cannot be read: {why}",
                    kept.file
                ));
                return outcome;
            }
        }
    }
    let built = match manifest_build::build(&bytes, &wanted) {
        Ok(built) => built,
        Err(e) => {
            outcome.notes.push(refusal(&e, facts, &names));
            return outcome;
        }
    };
    let shown = |hashes: &mut dyn Iterator<Item = u32>, table: &BTreeMap<u32, Name>| {
        hashes
            .map(|hash| {
                format!(
                    "`{}`",
                    table.get(&hash).cloned().unwrap_or(Name::of(hash)).shown()
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    if !built.unstated.is_empty() {
        outcome.notes.push(format!(
            "`{FILE}` states nothing for {}: no manifest of the game kept in `{KEPT}` states a map of that name, so nothing says what flags its entry takes",
            shown(&mut built.unstated.iter().copied(), &names)
        ));
    }
    if !built.pairless.is_empty() {
        outcome.notes.push(format!(
            "`{FILE}` states nothing for {}: the manifests of the game kept in `{KEPT}` state it only by pairs of a map and a type file, and it needs no type file here, so there is no pair to write",
            shown(&mut built.pairless.iter().copied(), &names)
        ));
    }
    if !built.disagreeing.is_empty() {
        outcome.notes.push(format!(
            "the manifests of the game kept in `{KEPT}` state {} differently from one another, and nothing kept says which the game mounts last: the entry written is taken from them in the order of their file names, so check its flags and keep only the manifest the game reads last",
            shown(&mut built.disagreeing.iter().copied(), &names)
        ));
    }
    for map in &wanted {
        if !built.kept.contains(&map.name) {
            continue;
        }
        let stated = built.stated.get(&map.name).cloned().unwrap_or_default();
        let mut lacking = (map.needs.iter().copied())
            .filter(|needed| ours.contains_key(needed) && !stated.contains(needed))
            .peekable();
        if lacking.peek().is_some() {
            outcome.notes.push(format!(
                "map {} keeps the entry the game states for it, which does not name {} that it now places from this content; the build registers that type file itself",
                shown(&mut std::iter::once(map.name), &names),
                shown(&mut lacking, &ours)
            ));
        }
    }
    outcome.stated = built.stated;
    outcome.file = Some(built.bytes);
    outcome
}

/// `name` with the text of the file's own name where the file states a hash alone and that name hashes to it.
fn named(name: &Name, file: &str) -> Name {
    match &name.text {
        None if or_io::string_hash(file.as_bytes()) == name.hash => Name::from_text(file),
        _ => name.clone(),
    }
}

/// The sentence a manifest that was not written becomes, with the kept file named where one is at fault.
fn refusal(error: &BuildError, facts: &Facts, names: &BTreeMap<u32, Name>) -> String {
    let file = |at: &usize| {
        facts
            .manifests
            .get(at.saturating_sub(1))
            .map_or_else(String::new, |kept| kept.file.clone())
    };
    match error {
        BuildError::NoDonor => format!(
            "no `{FILE}` is written: the content keeps no manifest of the game in `{KEPT}`, and that document is typed by one and takes the flags of each entry from one; `openrage-cli maps keep-manifest` keeps one"
        ),
        BuildError::NoEntry(_) => {
            let maps: Vec<String> = names.values().map(|name| format!("`{}`", name.shown())).collect();
            format!(
                "no `{FILE}` is written: no manifest of the game kept in `{KEPT}` states {}, so nothing says what flags an entry for it takes",
                maps.join(", ")
            )
        }
        BuildError::NoPair(_) => {
            let maps: Vec<String> = names.values().map(|name| format!("`{}`", name.shown())).collect();
            format!(
                "no `{FILE}` is written: the manifests of the game kept in `{KEPT}` state {} only by pairs of a map and a type file, and no map here needs a type file, so there is no pair to write",
                maps.join(", ")
            )
        }
        BuildError::Unreadable(at, why) => format!(
            "no `{FILE}` is written: the kept manifest `{KEPT}/{}` does not read: {why}",
            file(at)
        ),
        BuildError::NotPso(at, _) | BuildError::NotAManifest(at, _) => format!(
            "no `{FILE}` is written: the kept manifest `{KEPT}/{}` cannot be used: {error}",
            file(at)
        ),
        other => format!("no `{FILE}` is written: {other}"),
    }
}
