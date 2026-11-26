//! The kind of content that is maps: how a project hands a build what a module holds, how a folder of them is recognised when a
//! project adopts it, and how an empty module is started.
//!
//! A folder is recognised by its `maps.json`, and a folder without one is not maps however many `.ymap` files it holds: a project holds
//! sources, and a file is imported into one by the doors.

use std::path::{Path, PathBuf};

use or_project::adopt::{Recognised, Recogniser};
use or_project::content::Content;
use or_project::{Contribution, Module, ModuleKind, Output};

use crate::model::{Maps, FILE};
use crate::plan::{contribution, module_folder};

/// The id of the maps content an adoption makes.
const ID: &str = "maps";

/// The kind of content that is maps.
pub struct MapPack;

impl Content for MapPack {
    fn contribution(&self, project_folder: &Path, module: &Module) -> Contribution {
        contribution(project_folder, module)
    }

    fn contribution_for(
        &self,
        project_folder: &Path,
        module: &Module,
        output: &Output,
    ) -> Contribution {
        crate::plan::contribution_for(project_folder, module, output)
    }

    fn start(&self, project_folder: &Path, module: &Module, _name: &str) -> Result<(), String> {
        start_module(project_folder, module)
    }
}

impl Recogniser for MapPack {
    fn kind(&self) -> ModuleKind {
        ModuleKind::Maps
    }

    fn holds(&self, folder: &Path) -> bool {
        folder.join(FILE).is_file()
    }

    fn recognise(&self, folder: &Path, files: &[PathBuf]) -> Result<Option<Recognised>, String> {
        if !self.holds(folder) {
            return Ok(None);
        }
        let model = Maps::read(folder).map_err(|e| e.to_string())?;
        let sources: Vec<PathBuf> = model
            .maps
            .iter()
            .flat_map(|m| m.sources.iter())
            .map(|s| folder.join(s.replace('/', std::path::MAIN_SEPARATOR_STR)))
            .collect();
        // The file of the model, everything inside a source it names, and the manifests of the game it keeps.
        let kept = folder.join(crate::manifest::KEPT);
        let claimed: Vec<PathBuf> = files
            .iter()
            .filter(|file| {
                **file == folder.join(FILE)
                    || file.starts_with(&kept)
                    || sources.iter().any(|s| file.starts_with(s))
            })
            .cloned()
            .collect();
        let names: Vec<&str> = model.maps.iter().map(|m| m.name.as_str()).collect();
        let summary = format!("{} map set(s): {}", model.maps.len(), names.join(", "));
        Ok(Some(Recognised {
            module: Module::maps(ID, "."),
            summary,
            claimed,
            unplaced: Vec::new(),
            writes: Vec::new(),
        }))
    }
}

/// Make the folder of the maps module `module`: the folder, and an empty `maps.json` in it unless it has one.
pub fn start_module(project_folder: &Path, module: &Module) -> Result<(), String> {
    let folder = module_folder(project_folder, module).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    if folder.join(FILE).exists() {
        return Ok(());
    }
    Maps::new().write(&folder).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MapSet;

    #[test]
    fn an_empty_module_is_started_once_and_a_folder_with_a_model_is_recognised_by_it() {
        let dir = tempfile::tempdir().unwrap();
        let module = Module::maps("maps", "maps");
        start_module(dir.path(), &module).unwrap();
        let folder = dir.path().join("maps");
        assert!(MapPack.holds(&folder));
        assert_eq!(Maps::read(&folder).unwrap(), Maps::new());

        let mut model = Maps::new();
        let mut set = MapSet::new("harbour");
        set.sources.push("harbour.metadata.src".into());
        model.maps.push(set);
        model.write(&folder).unwrap();
        start_module(dir.path(), &module).unwrap();
        assert_eq!(
            Maps::read(&folder).unwrap(),
            model,
            "a started module keeps what is there"
        );

        let inside = folder.join("harbour.metadata.src").join("metadata.xml");
        let outside = folder.join("notes.txt");
        let files = [folder.join(FILE), inside.clone(), outside.clone()];
        let found = MapPack.recognise(&folder, &files).unwrap().unwrap();
        assert_eq!(found.module.kind, ModuleKind::Maps);
        assert!(found.claimed.contains(&inside) && !found.claimed.contains(&outside));
        assert!(MapPack.recognise(dir.path(), &[]).unwrap().is_none());
    }
}
