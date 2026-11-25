//! Adding game files to a pack of maps: each becomes a source in the module's folder, and the pack learns which set holds it.
//!
//! A `.ymap` or `.ytyp` becomes a metadata source, which is `or_source::metadata`'s form for them (a file whose XML would not come
//! back to a fixed point with every value is refused with the reason, and nothing of it is kept); a model, a fragment or a texture
//! dictionary becomes the source its format has. The sources join one map set: the one asked for, else the first, else a new one named
//! for the first map file. What no source form holds is refused with the reason and never dropped, and so is a file whose source
//! a set gives no role: a bound, a path region, a particle list.

use std::path::{Path, PathBuf};

use or_source::game_import::{self, Facts as Origin};
use or_source::models::Extraction;

use crate::model::{MapSet, Maps};

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
    /// The files that were not added, each with why.
    pub refused: Vec<(String, String)>,
    /// The name of the set that was made for them, when none held them.
    pub set_made: Option<String>,
}

/// The extension of a file name, lower case.
fn extension(name: &str) -> String {
    name.rsplit_once('.')
        .map_or(String::new(), |(_, e)| e.to_ascii_lowercase())
}

/// Make a source of one game file in `folder`, and return its folder; the pack is not told of it ([`learn`] does that).
pub fn add_one(folder: &Path, file: &Incoming<'_>) -> Result<PathBuf, String> {
    std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let stem = file.name.rsplit_once('.').map_or(file.name, |(s, _)| s);
    match extension(file.name).as_str() {
        ext @ ("ymap" | "ytyp") => or_source::metadata::extract(
            file.bytes,
            stem,
            ext,
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
        ext => {
            // ⚠ A source of a kind the module holds no role for would join a set whose rules then call it unreadable.
            if let Some(row) = game_import::kind_of(ext) {
                if crate::facts::role_of(row.kind, ext).is_none() {
                    return Err(format!(
                        "becomes a {} source, and a maps module holds map files, type files, models and textures",
                        row.kind
                    ));
                }
            }
            game_import::decompile(
                file.bytes,
                file.name,
                folder,
                &Origin {
                    from_game: file.from_game,
                    virtual_path: file.virtual_path.map(str::to_string),
                    ..Origin::default()
                },
            )
        }
    }
}

/// The folders of `made` as the names the model gives sources: a folder of the module's own, with `/`.
fn relative(folder: &Path, dir: &Path) -> String {
    dir.strip_prefix(folder)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| dir.display().to_string())
}

/// Tell the pack of the source folders `made` in `folder`: they join the set `into` names, else the first set, else a new set named for the first map file among them (or `maps`). Returns the id of the set and whether it was made here, none when there was nothing to tell.
pub fn learn(
    folder: &Path,
    made: &[PathBuf],
    into: Option<&str>,
) -> Result<Option<(String, bool)>, String> {
    let mut model = if folder.join(crate::model::FILE).is_file() {
        Maps::read(folder).map_err(|e| e.to_string())?
    } else {
        Maps::new()
    };
    if made.is_empty() {
        if !folder.join(crate::model::FILE).is_file() {
            model.write(folder).map_err(|e| e.to_string())?;
        }
        return Ok(None);
    }
    let named = into.and_then(|id| model.map(id)).map(|set| set.id.clone());
    let (id, new) = match named.or_else(|| model.maps.first().map(|set| set.id.clone())) {
        Some(id) => (id, false),
        None => {
            let first_map = made.iter().find_map(|dir| {
                let name = dir.file_name()?.to_string_lossy().into_owned();
                let stem = name.split('.').next()?.to_string();
                name.contains(".metadata.").then_some(stem)
            });
            let set = MapSet::new(first_map.as_deref().unwrap_or("maps"));
            let id = set.id.clone();
            model.maps.push(set);
            (id, true)
        }
    };
    if let Some(set) = model.map_mut(&id) {
        for dir in made {
            let path = relative(folder, dir);
            if !set.sources.contains(&path) {
                set.sources.push(path);
            }
        }
    }
    model.write(folder).map_err(|e| e.to_string())?;
    Ok(Some((id, new)))
}

/// Take the source folders `gone` (names of the module's folder, with `/`) out of every set that names them, and the sets left with no source because of it out of the pack. Whether the model changed.
pub fn forget(folder: &Path, gone: &[String]) -> Result<bool, String> {
    let mut model = Maps::read(folder).map_err(|e| e.to_string())?;
    let before = model.clone();
    for set in &mut model.maps {
        set.sources.retain(|source| !gone.contains(source));
    }
    model.maps.retain(|set| {
        let named_a_gone_one = before
            .map(&set.id)
            .is_some_and(|was| was.sources.iter().any(|source| gone.contains(source)));
        !(set.sources.is_empty() && named_a_gone_one)
    });
    if model == before {
        return Ok(false);
    }
    model.write(folder).map_err(|e| e.to_string())?;
    Ok(true)
}

/// Make a source of each of `incoming` in `folder` and tell the pack of them.
pub fn add_files(
    folder: &Path,
    incoming: &[Incoming<'_>],
    into: Option<&str>,
) -> Result<Added, String> {
    std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let mut added = Added::default();
    for file in incoming {
        match add_one(folder, file) {
            Ok(dir) => added.made.push(dir),
            Err(why) => added.refused.push((file.name.to_string(), why)),
        }
    }
    if let Some((id, true)) = learn(folder, &added.made, into)? {
        added.set_made = Maps::read(folder)
            .ok()
            .and_then(|m| m.map(&id).map(|s| s.name.clone()));
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

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
            None,
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
        assert_eq!(Maps::read(dir.path()).unwrap(), Maps::new());
    }

    #[test]
    fn a_bound_and_a_path_region_are_refused_at_the_door_and_join_no_set() {
        use or_io::decl::File;
        use or_io::gta5::bounds::build::{self, BoundMesh};
        use or_io::gta5::bounds::PhBoundAny;
        use or_io::gta5::CPathRegion;

        let mesh = BoundMesh::new(
            vec![[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [4.0, 4.0, 1.0]],
            vec![[0, 1, 2]],
        );
        let bound = PhBoundAny::PhBoundBVH(build::bvh(&mesh).unwrap())
            .to_bytes()
            .unwrap();
        let region = CPathRegion::default().to_bytes().unwrap();
        // Both become sources where a project takes them, so the refusal is the module's and not the format's.
        for (name, bytes) in [("hull.ybn", &bound), ("nodes7.ynd", &region)] {
            let project = tempfile::tempdir().unwrap();
            let made = game_import::decompile(bytes, name, project.path(), &Origin::default());
            assert!(made.is_ok(), "{name}: {made:?}");
        }

        let dir = tempfile::tempdir().unwrap();
        let incoming: Vec<Incoming<'_>> = [("hull.ybn", &bound), ("nodes7.ynd", &region)]
            .into_iter()
            .map(|(name, bytes)| Incoming {
                name,
                bytes,
                from_game: true,
                virtual_path: None,
            })
            .collect();
        let added = add_files(dir.path(), &incoming, None).unwrap();
        assert!(added.made.is_empty(), "{:?}", added.made);
        let why: Vec<&str> = added.refused.iter().map(|(_, why)| why.as_str()).collect();
        assert_eq!(why.len(), 2);
        assert!(why[0].contains("a bounds source"), "{why:?}");
        assert!(why[1].contains("a paths source"), "{why:?}");
        assert!(
            why.iter().all(|why| why.contains("a maps module holds")),
            "{why:?}"
        );
        assert_eq!(Maps::read(dir.path()).unwrap(), Maps::new());
        let left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .filter(|name| name.to_string_lossy().ends_with(".src"))
            .collect();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn what_a_pack_forgets_of_a_taken_back_source_leaves_the_sets_that_still_hold_something() {
        let dir = tempfile::tempdir().unwrap();
        let mut model = Maps::new();
        let mut kept = MapSet::new("kept");
        kept.sources = vec!["a.metadata.src".into(), "b.metadata.src".into()];
        let mut lost = MapSet::new("lost");
        lost.sources = vec!["c.metadata.src".into()];
        let empty = MapSet::new("empty");
        model.maps = vec![kept, lost, empty];
        model.write(dir.path()).unwrap();

        assert!(forget(
            dir.path(),
            &["b.metadata.src".into(), "c.metadata.src".into()]
        )
        .unwrap());
        let after = Maps::read(dir.path()).unwrap();
        let names: Vec<&str> = after.maps.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            names,
            ["kept", "empty"],
            "a set with nothing left because of it goes; one that never had a source stays"
        );
        assert_eq!(after.maps[0].sources, ["a.metadata.src"]);
        assert!(!forget(dir.path(), &["zzz.metadata.src".into()]).unwrap());
    }
}
