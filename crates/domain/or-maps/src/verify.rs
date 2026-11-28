//! The verifier: what a built resource has to hold for the map sets that were planned, read back.
//!
//! A build already says that every file it wrote is the plan's and reloads through a reader. This says what only a module of maps
//! knows: that each source reached the place the plan gave it, that each map file and type file reloads as the root it is and holds
//! what its source states, that the resource's manifest registers each type file and declares the resource a map, and that the
//! manifest of the maps is there and states what the sources and the kept manifests come to.
//!
//! ⚠ It checks the files and not the game. Whether GTA V streams a map is a thing nobody working on this repository can run.

use std::path::Path;

use or_io::meta::typed::{CMapData, CMapTypes};

use crate::facts::{Facts, Role, SourceFact};
use crate::layout::{stream_path, MAP_LINE, TYPE_FILE};
use crate::manifest;
use crate::model::Maps;
use crate::plan::place_of;

pub use or_project::content::check::{Check, Verification};

/// Read `resource`, the folder a build of `model` made, against `model` whose sources `facts` looked at.
pub fn verify(model: &Maps, facts: &Facts, resource: &Path) -> Verification {
    let mut found = Verification::default();
    let manifest = std::fs::read_to_string(resource.join("fxmanifest.lua")).unwrap_or_default();
    let mut seen: Vec<&str> = Vec::new();
    let mut maps_shipped = false;
    for set in &model.maps {
        let mut problems = Vec::new();
        let mut shipped = Vec::new();
        for path in &set.sources {
            let Some(source) = facts.source(path) else {
                continue;
            };
            let at = place_of(source);
            // A source of an unchanged original is left out by a build that is told to.
            if !resource.join(&at).is_file() {
                if !source.game.is_empty() {
                    continue;
                }
                problems.push(format!("`{at}` is not in the resource"));
                continue;
            }
            shipped.push(at.clone());
            maps_shipped |= source.role() == Some(Role::Map);
            if seen.contains(&path.as_str()) {
                continue;
            }
            seen.push(path);
            problems.extend(file(resource, &at, source, &manifest));
        }
        found.add(
            format!("map {}", set.name.trim()),
            problems,
            format!(
                "{} is shipped as `{}`",
                set.name.trim(),
                shipped.join("`, `")
            ),
        );
    }
    if maps_shipped {
        let line = format!("{} '{}'", MAP_LINE.0, MAP_LINE.1);
        // ⚠ Read as the statement `project check-resource` reads, never looked for as text: a line in a comment declares nothing.
        let read = or_project::resource::manifest::Manifest::read(&manifest);
        let stated =
            (read.directives.iter()).any(|d| d.key == MAP_LINE.0 && d.values == [MAP_LINE.1]);
        let problems = match stated {
            true => Vec::new(),
            false => vec![format!("the manifest does not state `{line}`")],
        };
        found.add(
            "map resource".to_string(),
            problems,
            format!("the manifest declares the resource a map with `{line}`"),
        );
    }
    let stated = manifest::of(model, facts);
    if stated.file.is_some() {
        let at = stream_path(manifest::FILE);
        found.add(
            "manifest of the maps".to_string(),
            manifest_problems(resource, &at, &stated),
            format!(
                "`{at}` states the type files of {} map(s)",
                stated.stated.len()
            ),
        );
    }
    found
}

/// What is wrong with the manifest of the maps at `at` of `resource`: it is not there, does not read, or states other than `stated`.
fn manifest_problems(resource: &Path, at: &str, stated: &manifest::Outcome) -> Vec<String> {
    let bytes = match std::fs::read(resource.join(at)) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return vec![format!("`{at}` is not in the resource")]
        }
        Err(e) => return vec![format!("`{at}` cannot be read: {e}")],
    };
    match or_world_gta5::manifest_build::stated_in(&bytes) {
        Ok(held) if held != stated.stated => vec![format!(
            "`{at}` states other type files than the sources and the kept manifests come to"
        )],
        // ⚠ The document is the same bytes on every build, so one that states the same type files and differs states something else of an entry, such as its flags.
        Ok(_) if stated.file.as_deref() != Some(bytes.as_slice()) => vec![format!(
            "`{at}` states the type files the sources and the kept manifests come to, and is not the document they come to"
        )],
        Ok(_) => Vec::new(),
        Err(why) => vec![format!("`{at}` does not read as a manifest: {why}")],
    }
}

/// What is wrong with the file at `at` of `resource`: it does not reload as the root it is, holds other than its source states, or is a type file the manifest does not register.
fn file(resource: &Path, at: &str, source: &SourceFact, manifest: &str) -> Vec<String> {
    let bytes = match std::fs::read(resource.join(at)) {
        Ok(bytes) => bytes,
        Err(e) => return vec![format!("`{at}` cannot be read: {e}")],
    };
    let mut problems = Vec::new();
    // A model and a texture dictionary are reloaded by the build itself, through the readers of their formats.
    if !matches!(source.role(), Some(Role::Map | Role::Types)) {
        return problems;
    }
    match reload(source, &bytes) {
        Ok(held) => {
            let stated = match (source.map(), source.types()) {
                (Some(map), _) => Some(map.entities.len()),
                (_, Some(types)) => Some(types.archetypes.len()),
                _ => None,
            };
            if stated.is_some_and(|stated| stated != held) {
                problems.push(format!(
                    "`{at}` reloads with {held} entries and its source states {}",
                    stated.unwrap_or_default()
                ));
            }
        }
        Err(why) => problems.push(format!("`{at}` does not reload: {why}")),
    }
    if source.role() == Some(Role::Types) {
        let line = format!("data_file '{TYPE_FILE}' '{at}'");
        if !manifest.contains(&line) {
            problems.push(format!("the manifest does not state `{line}`"));
        }
    }
    problems
}

/// Open `bytes` as the typed root the source's file is, and say how many entities or archetypes it holds.
fn reload(source: &SourceFact, bytes: &[u8]) -> Result<usize, String> {
    use or_io::decl::File;
    match source.role() {
        Some(Role::Map) => CMapData::from_bytes(bytes)
            .map(|map| map.entities.len())
            .map_err(|e| e.to_string()),
        _ => CMapTypes::from_bytes(bytes)
            .map(|types| types.archetypes.len())
            .map_err(|e| e.to_string()),
    }
}
