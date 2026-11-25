//! Adding game files to a pack of weapons: each becomes a source in the module's folder, and the pack learns which weapons they are.
//!
//! A text `.meta` becomes a metadata source, which is the one row this content adds to what a project imports (a project keeps a `.meta`
//! it imports as a loose file, since it states no folder a data source ships at); a model, a fragment or a texture dictionary becomes
//! the source its format has. The weapons a metadata file defines are added to the model, and a model or a texture is given to the weapon
//! whose metadata names it, by the name it cooks under. What names none is kept in the folder and said, never dropped.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use or_source::game_import::{self, Facts as Origin};
use or_source::models::Extraction;

use crate::facts::{inspect, Depth};
use crate::model::{Weapon, Weapons};
use crate::rules::stem;

/// A game file to add.
#[derive(Debug, Clone)]
pub struct Incoming<'a> {
    /// The name it has in the game, with its extension.
    pub name: &'a str,
    pub bytes: &'a [u8],
    pub from_game: bool,
    /// Its path inside the game's archives, when it came out of one.
    pub virtual_path: Option<&'a str>,
}

/// What became of the files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Added {
    /// The source folders made.
    pub made: Vec<PathBuf>,
    /// The names of the weapons the metadata defined that the pack did not hold.
    pub weapons: Vec<String>,
    /// The files that were not added, each with why.
    pub refused: Vec<(String, String)>,
    /// The sources made that no weapon names yet.
    pub unplaced: Vec<String>,
}

/// The extension of a file name, lower case.
fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .map_or(String::new(), |(_, e)| e.to_ascii_lowercase())
}

/// Make a source of one game file in `folder`, and return its folder; the pack is not told of it ([`learn`] does that).
pub fn add_one(folder: &Path, file: &Incoming<'_>) -> Result<PathBuf, String> {
    std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let stem_of = file.name.rsplit_once('.').map_or(file.name, |(s, _)| s);
    match extension(file.name).as_str() {
        "meta" => or_source::metadata::extract(
            file.bytes,
            stem_of,
            "meta",
            folder,
            &Extraction {
                from_game: file.from_game,
                virtual_path: file.virtual_path,
                textures: false,
                skeleton: None,
            },
        )
        .map_err(|problems| {
            problems
                .iter()
                .map(|p| p.message.clone())
                .collect::<Vec<_>>()
                .join("; ")
        }),
        _ => game_import::decompile(
            file.bytes,
            file.name,
            folder,
            &Origin {
                from_game: file.from_game,
                virtual_path: file.virtual_path.map(str::to_string),
                ..Origin::default()
            },
        ),
    }
}

/// What the pack learned of the sources it was told of.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Learned {
    /// The names of the weapons the metadata defined that the pack did not hold.
    pub weapons: Vec<String>,
    /// The sources that no weapon names yet.
    pub unplaced: Vec<String>,
}

/// Tell the pack of the source folders `made` in `folder`: the weapons their metadata defines join it, and a model or a texture goes to the weapon whose metadata names it.
pub fn learn(folder: &Path, made: &[PathBuf]) -> Result<Learned, String> {
    let mut model = if folder.join(crate::model::FILE).is_file() {
        Weapons::read(folder).map_err(|e| e.to_string())?
    } else {
        Weapons::new()
    };
    let mut learned = Learned::default();
    let relative = |dir: &Path| {
        dir.strip_prefix(folder)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| dir.display().to_string())
    };
    // The metadata first: what it defines is what the other sources are given to.
    let facts = inspect(
        folder,
        &Weapons {
            weapons: vec![Weapon {
                id: "scan".into(),
                label: None,
                name: String::new(),
                sources: made.iter().map(|d| relative(d)).collect(),
            }],
            ..Weapons::new()
        },
        Depth::Open,
    );
    for dir in made {
        let path = relative(dir);
        let Some(source) = facts.source(&path).filter(|s| s.is_metadata()) else {
            continue;
        };
        let Some(Ok(defined)) = &source.defined else {
            continue;
        };
        for defined in &defined.weapons {
            match model
                .weapons
                .iter_mut()
                .find(|w| w.name.eq_ignore_ascii_case(&defined.name))
            {
                Some(weapon) => {
                    if !weapon.sources.contains(&path) {
                        weapon.sources.push(path.clone());
                    }
                }
                None => {
                    let mut weapon = Weapon::new(&defined.name);
                    weapon.sources.push(path.clone());
                    learned.weapons.push(defined.name.clone());
                    model.weapons.push(weapon);
                }
            }
        }
    }
    give_to_their_weapons(folder, &mut model, made, &relative, &mut learned.unplaced);
    model.write(folder).map_err(|e| e.to_string())?;
    Ok(learned)
}

/// Take the source folders `gone` (names of the module's folder, with `/`) out of every weapon that names them, and the weapons left with no source because of it out of the pack.
pub fn forget(folder: &Path, gone: &[String]) -> Result<bool, String> {
    let mut model = Weapons::read(folder).map_err(|e| e.to_string())?;
    let before = model.clone();
    for weapon in &mut model.weapons {
        weapon.sources.retain(|source| !gone.contains(source));
    }
    model.weapons.retain(|weapon| {
        let named_a_gone_one = before
            .weapon(&weapon.id)
            .is_some_and(|was| was.sources.iter().any(|source| gone.contains(source)));
        !(weapon.sources.is_empty() && named_a_gone_one)
    });
    if model == before {
        return Ok(false);
    }
    model.write(folder).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Make a source of each of `incoming` in `folder`, and add the weapons they define and give the rest to their weapons.
pub fn add_files(folder: &Path, incoming: &[Incoming<'_>]) -> Result<Added, String> {
    std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let mut added = Added::default();
    for file in incoming {
        match add_one(folder, file) {
            Ok(dir) => added.made.push(dir),
            Err(why) => added.refused.push((file.name.to_string(), why)),
        }
    }
    let learned = learn(folder, &added.made)?;
    added.weapons = learned.weapons;
    added.unplaced = learned.unplaced;
    Ok(added)
}

/// Give each of the sources `made` that is a model or a texture dictionary to the weapons whose metadata names it.
fn give_to_their_weapons(
    folder: &Path,
    model: &mut Weapons,
    made: &[PathBuf],
    relative: &dyn Fn(&Path) -> String,
    unplaced: &mut Vec<String>,
) {
    let facts = inspect(folder, model, Depth::Open);
    // For each weapon, the names its pack's metadata says its sources cook under.
    let wanted: Vec<BTreeSet<String>> = model
        .weapons
        .iter()
        .map(|weapon| {
            let assembly = crate::assemble::assemble(weapon, &facts);
            let mut names: BTreeSet<String> =
                assembly.parts.iter().map(|p| stem(&p.model_name)).collect();
            if let Some(defined) = crate::rules::defined_by(weapon, &facts) {
                names.insert(stem(&defined.model));
                for archetype in weapon
                    .sources
                    .iter()
                    .filter_map(|p| facts.source(p)?.defined.as_ref()?.as_ref().ok())
                    .flat_map(|d| &d.archetypes)
                    .filter(|a| stem(&a.model_name) == stem(&defined.model))
                {
                    names.insert(stem(&archetype.txd_name));
                }
            }
            names.remove("");
            names
        })
        .collect();
    let named = inspect(
        folder,
        &Weapons {
            weapons: vec![Weapon {
                id: "scan".into(),
                label: None,
                name: String::new(),
                sources: made.iter().map(|d| relative(d)).collect(),
            }],
            ..Weapons::new()
        },
        Depth::Stat,
    );
    for dir in made {
        let path = relative(dir);
        let Some(source) = named.source(&path) else {
            continue;
        };
        if !(source.is_model() || source.is_textures()) {
            continue;
        }
        let mut given = false;
        for (weapon, names) in model.weapons.iter_mut().zip(&wanted) {
            if names.contains(&stem(&source.name)) && !weapon.sources.contains(&path) {
                weapon.sources.push(path.clone());
                given = true;
            }
        }
        if !given && !model.weapons.iter().any(|w| w.sources.contains(&path)) {
            unplaced.push(path);
        }
    }
}

/// A weapon called `name` in a metadata source of its own, a `CWeaponInfo` with every member its declaration names: the id the pack gives it, and the source folder.
pub fn new_weapon(folder: &Path, name: &str) -> Result<(String, PathBuf), String> {
    use or_io::decl::File;
    use or_io::meta::typed::weapon::{CWeaponInfo, CWeaponInfoBlob, CWeaponInfoBlobInfos, InfoAny};

    std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let mut model = if folder.join(crate::model::FILE).is_file() {
        Weapons::read(folder).map_err(|e| e.to_string())?
    } else {
        Weapons::new()
    };
    if model
        .weapons
        .iter()
        .any(|w| w.name.eq_ignore_ascii_case(name))
    {
        return Err(format!("the pack already holds a weapon called {name}"));
    }
    let stem = name.to_ascii_lowercase();
    let mut info = CWeaponInfo::default();
    info.name = name.to_string();
    info.model = stem.clone();
    let mut group = CWeaponInfoBlobInfos::default();
    group.infos.push(InfoAny::CWeaponInfo(info));
    let mut blob = CWeaponInfoBlob::default();
    blob.name = stem.clone();
    blob.infos.push(group);
    let bytes = blob.to_bytes().map_err(|e| e.to_string())?;
    let dir = or_source::metadata::extract(
        &bytes,
        &stem,
        "meta",
        folder,
        &Extraction {
            from_game: false,
            virtual_path: None,
            textures: false,
            skeleton: None,
        },
    )
    .map_err(|problems| {
        problems
            .iter()
            .map(|p| p.message.clone())
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    let mut weapon = Weapon::new(name);
    weapon.sources.push(
        dir.file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
    );
    let id = weapon.id.clone();
    model.weapons.push(weapon);
    model.write(folder).map_err(|e| e.to_string())?;
    Ok((id, dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_weapon_is_a_metadata_source_of_its_own_that_reads_back_and_its_name_is_held_once() {
        let dir = tempfile::tempdir().unwrap();
        let (id, source) = new_weapon(dir.path(), "WEAPON_NEW").unwrap();
        assert!(source.ends_with("weapon_new.metadata.src"));
        let model = Weapons::read(dir.path()).unwrap();
        assert_eq!(model.weapons.len(), 1);
        assert_eq!(
            (model.weapons[0].id.as_str(), model.weapons[0].name.as_str()),
            (id.as_str(), "WEAPON_NEW")
        );
        let facts = inspect(dir.path(), &model, Depth::Open);
        let defined = crate::rules::defined_by(&model.weapons[0], &facts);
        assert_eq!(defined.map(|d| d.name.as_str()), Some("WEAPON_NEW"));
        assert!(new_weapon(dir.path(), "weapon_new").is_err());
    }

    #[test]
    fn a_file_with_no_source_form_is_refused_with_the_reason_and_nothing_is_lost() {
        let dir = tempfile::tempdir().unwrap();
        let added = add_files(
            dir.path(),
            &[Incoming {
                name: "hull.yld",
                bytes: b"x",
                from_game: false,
                virtual_path: None,
            }],
        )
        .unwrap();
        assert!(added.made.is_empty());
        assert_eq!(added.refused.len(), 1);
        assert!(
            added.refused[0].1.contains("source form"),
            "{:?}",
            added.refused
        );
        // The module has its model after the call, empty.
        assert_eq!(Weapons::read(dir.path()).unwrap(), Weapons::new());
    }
}
