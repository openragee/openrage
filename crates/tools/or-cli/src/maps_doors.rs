//! `openrage-cli maps doors --game`: the doors of one map file of a mounted install, surveyed as those of a map set of a content
//! are (`or_maps::doors`). It reads the install and writes nothing but the world index it keeps.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use or_io::meta::MetaDocument;
use or_io::HashFlavour;
use or_maps::doors::{survey, SetDoors};
use or_maps::{MapSet, Maps};
use or_world::collision::CollisionSet;
use or_world_gta5::doors::{NamePatterns, Names};
use or_world_gta5::find::names_of;
use or_world_gta5::{MapData, MapTypes};
use or_world_gta5::{MountedWorld, WorldIndex, WorldSource};
use serde_json::json;

use crate::project::Failure;
use Failure::{Run, Usage};

/// The flags of `maps doors` that say something of an install, and so go with `--game` alone.
pub(crate) const OF_AN_INSTALL: &[&str] = &["--map", "--keys", "--kept"];

/// A metadata file of root archive `archive`, or why it gave none.
fn document(
    source: &dyn WorldSource,
    archive: usize,
    logical: &str,
) -> Result<MetaDocument, String> {
    let bytes = (source.open(archive, logical))
        .ok_or_else(|| "the archive that lists it does not open it".to_string())?;
    MetaDocument::parse(or_io::meta::default_library(), bytes)
        .map_err(|e| format!("does not read as a metadata document: {e}"))
}

/// Open each type file of `wanted` that is not in `opened`, and say in `unread` which gave no document.
fn open_types(
    source: &dyn WorldSource,
    index: &WorldIndex,
    wanted: impl IntoIterator<Item = u32>,
    opened: &mut HashMap<u32, MetaDocument>,
    unread: &mut Vec<(String, String)>,
) {
    for hash in wanted.into_iter().collect::<BTreeSet<u32>>() {
        let Some(file) = index
            .type_file(hash)
            .filter(|_| !opened.contains_key(&hash))
        else {
            continue;
        };
        // A file that gave no document is asked for once, and said once.
        if unread.iter().any(|(logical, _)| *logical == file.logical) {
            continue;
        }
        match document(source, file.archive, &file.logical) {
            Ok(read) => {
                opened.insert(hash, read);
            }
            Err(why) => unread.push((file.logical.clone(), why)),
        }
    }
}

/// The doors of the map file of the mount that `map` names by its logical path, by the type files `index` says declare what it places: the path as the index spells it, and the doors.
///
/// ⚠ An arc is held against the boxes of the map's other entities alone: no bound of the mount is streamed for it.
pub(crate) fn doors_of_map(
    source: &dyn WorldSource,
    index: &WorldIndex,
    map: &str,
    names: Names<'_>,
    patterns: &NamePatterns,
) -> Result<(String, SetDoors), String> {
    let listed = (index.maps().iter())
        .find(|listed| listed.logical.eq_ignore_ascii_case(map))
        .ok_or_else(|| {
            format!(
                "the install holds no map file `{map}`: name one by its logical path, as `world index`, `world at` and `world find` print it"
            )
        })?;
    let held = document(source, listed.archive, &listed.logical)
        .map_err(|why| format!("`{}` {why}", listed.logical))?;
    let view = MapData::new(&held.instance)
        .ok_or_else(|| format!("`{}` does not read as a map file", listed.logical))?;

    let file_of = |archetype: u32| {
        index
            .archetype(archetype)
            .map(|declared| declared.type_file)
    };
    let mut opened: HashMap<u32, MetaDocument> = HashMap::new();
    let mut unread: Vec<(String, String)> = Vec::new();
    let placed = view
        .entities()
        .filter_map(|entity| file_of(entity.archetype()));
    open_types(source, index, placed, &mut opened, &mut unread);
    // What an interior the map places holds is declared by type files of its own; an interior holds no interior.
    let interiors: BTreeSet<u32> = (view.entities())
        .filter(|entity| entity.is_interior_instance())
        .map(|entity| entity.archetype())
        .collect();
    let within: Vec<u32> = (opened.values())
        .filter_map(|read| MapTypes::new(&read.instance))
        .flat_map(|file| file.archetypes())
        .filter(|archetype| interiors.contains(&archetype.name()))
        .filter_map(|archetype| archetype.interior())
        .flat_map(|interior| interior.entities())
        .filter_map(|entity| file_of(entity.archetype()))
        .collect();
    open_types(source, index, within, &mut opened, &mut unread);

    // ⚠ Of two type files that declare one name the search believes the last it is handed, as the index does: in mount order.
    let mut types: Vec<MapTypes<'_>> = Vec::new();
    for file in index.type_files() {
        let Some(read) = opened.get(&file.hash) else {
            continue;
        };
        match MapTypes::new(&read.instance) {
            Some(view) => types.push(view),
            None => unread.push((
                file.logical.clone(),
                "its root is not the one its extension says".to_string(),
            )),
        }
    }
    let set = MapSet::new(&listed.name);
    let nothing: CollisionSet<String> = CollisionSet::new();
    let maps = [(listed.logical.as_str(), view)];
    let mut doors = survey(&Maps::new(), &set, &maps, &types, names, patterns, &nothing);
    doors.unread = unread;
    Ok((listed.logical.clone(), doors))
}

/// The path the world index lists the map file `map` names at: `map` where the index lists it, and the copy the install loads of the map of its name where `map` lies where that copy does. A path that lies nowhere is given back, for the search to refuse.
///
/// ⚠ By where each lies and not by its path: the copy a title update patches is listed at its own path and answers at the pack's too, which is the path the mount lists the pack's file at.
pub(crate) fn listed_path(
    index: &WorldIndex,
    lies_at: &dyn Fn(&str) -> Option<String>,
    map: &str,
) -> Result<String, String> {
    let listed =
        |listed: &&or_world_gta5::world_index::Map| listed.logical.eq_ignore_ascii_case(map);
    if let Some(listed) = index.maps().iter().find(listed) {
        return Ok(listed.logical.clone());
    }
    let leaf = map.rsplit(['/', '\\']).next().unwrap_or(map);
    let stem = leaf.rsplit_once('.').map_or(leaf, |(stem, _)| stem);
    let loaded = index.map(or_io::string_hash(stem.to_ascii_lowercase().as_bytes()));
    match (loaded, lies_at(map)) {
        (Some(loaded), Some(there)) if lies_at(&loaded.logical).as_ref() == Some(&there) => {
            Ok(loaded.logical.clone())
        }
        (Some(loaded), Some(_)) => Err(format!(
            "`{map}` is not the copy of that map the install loads: a later archive holds the map again, at `{}`, and its doors are the ones the game places. Name that file",
            loaded.logical
        )),
        _ => Ok(map.to_string()),
    }
}

/// The names a file lists, a line each, by the hash a map file keeps of each.
pub(crate) fn names_listed(file: Option<&String>) -> Result<HashMap<u32, String>, Failure> {
    let Some(file) = file else {
        return Ok(HashMap::new());
    };
    Ok(std::fs::read_to_string(file)
        .map_err(|e| Run(format!("{file}: {e}")))?
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| (or_io::string_hash(name.as_bytes()), name.to_string()))
        .collect())
}

/// What `maps doors --game` prints of one map file: its lines, and the same as data.
pub(crate) fn report(root: &Path, doors: &SetDoors, map: &str) -> (Vec<String>, serde_json::Value) {
    let mut lines = vec![format!("install {}: map {map}", root.display())];
    lines.extend(doors.lines().into_iter().map(|line| format!("  {line}")));
    let data = json!([{ "game": root.display().to_string(), "map": map, "sets": [doors] }]);
    (lines, data)
}

/// List the doors of the map file `--map` names in the install `--game` names.
pub(crate) fn run(
    flags: &HashMap<String, String>,
    fresh: bool,
    json_out: bool,
) -> Result<ExitCode, Failure> {
    let game = (flags.get("--game"))
        .ok_or_else(|| Usage("--game <install root> names the install".into()))?;
    let map = (flags.get("--map")).ok_or_else(|| {
        Usage(
            "--map <logical path> names the map file of the install whose doors are listed".into(),
        )
    })?;
    let listed = names_listed(flags.get("--names"))?;
    let patterns = or_maps::doors::built_in_patterns().map_err(|e| Run(e.to_string()))?;
    // The path as the file system spells it, so one install is one kept index however it is named.
    let root = std::fs::canonicalize(game).map_err(|e| Run(format!("{game}: {e}")))?;
    let mount = crate::keys::mount_all(&root, flags.get("--keys").map(Path::new)).map_err(Run)?;
    let kept = match flags.get("--kept") {
        Some(file) => PathBuf::from(file),
        None => crate::world_index::default_kept(&root, None).ok_or_else(|| {
            Run("no configuration directory to keep the world index in: name a file with --kept <file>".into())
        })?,
    };
    let (index, built) = crate::world_index::indexed(&mount, &root, &kept, fresh);
    if let Some(error) = &built.not_kept {
        eprintln!("warning: the index was not kept: {error}");
    }
    let source = MountedWorld::new(&mount, &root);
    // A model's name is the stem of a file the mount lists, in the case its hash folds it to.
    let (_, stated) = names_of(&source);
    let named = |hash: u32| {
        listed.get(&hash).cloned().or_else(|| {
            let name = stated.resolve_with(HashFlavour::CaseFolding, hash);
            name.map(str::to_ascii_lowercase)
        })
    };
    let map = listed_path(&index, &|path| mount.vfs.locate(path), map).map_err(Run)?;
    let (map, doors) = doors_of_map(&source, &index, &map, &named, &patterns).map_err(Run)?;
    let (lines, data) = report(&root, &doors, &map);
    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&data).map_err(|e| Run(e.to_string()))?
        );
    } else {
        for line in lines {
            println!("{line}");
        }
        println!("{}", crate::maps::DOORS_READ_FROM);
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world_fixture::{
        cube, hash, interior_types_bytes, manifest_bytes, mount, placing_map_bytes, write_archive,
        MapSpec, Placed,
    };
    use or_maps::sample::{types_file, Archetype};
    use or_title::MountedGame;

    const DOOR: &str = "vault_door";
    const BANK: &str = "platform:/levels/bank.ymap";

    /// A leaf `width` metres wide.
    fn leaf(width: f32) -> Archetype<'static> {
        Archetype::new(DOOR, [0.0, -0.02, 0.0], [width, 0.02, 2.0])
    }

    fn sofa(name: &str) -> Archetype<'_> {
        Archetype::new(name, [-0.5, -0.5, 0.0], [0.5, 0.5, 1.0])
    }

    /// The install at `root` with `files` in its archive of levels, mounted, and its index.
    fn indexed(root: &Path, files: &[(&str, Vec<u8>)]) -> (MountedGame, WorldIndex) {
        let manifest = manifest_bytes(&[], &[]);
        write_archive(root, "common.rpf", &[("data/_manifest.ymf", manifest)]);
        write_archive(root, "x64a.rpf", files);
        let game = mount(root);
        let (index, _) = WorldIndex::build(&MountedWorld::new(&game, root), None);
        (game, index)
    }

    /// The doors of the bank's map, read through `source`, the door's model named.
    fn doors_of_the_bank(source: &dyn WorldSource, index: &WorldIndex) -> SetDoors {
        let patterns = or_maps::doors::built_in_patterns().expect("the patterns read");
        let named = |name: u32| (name == hash(DOOR)).then(|| DOOR.to_string());
        let (map, doors) = doors_of_map(source, index, BANK, &named, &patterns).expect("the map");
        assert_eq!(map, BANK);
        doors
    }

    #[test]
    fn a_door_two_type_files_declare_has_the_box_of_the_one_the_index_believes() {
        let square = MapSpec::around(cube([0.0; 3], 50.0));
        let placed = [
            Placed::new(DOOR, [10.0, 10.0, 0.0], 0.0, 1),
            Placed::new("sofa_a", [20.0, 20.0, 0.0], 0.0, 2),
            Placed::new("sofa_b", [30.0, 30.0, 0.0], 0.0, 3),
        ];
        // ⚠ Each file declares a model the other does not, so the map has both opened whichever the index believes of the door,
        // and the widths change files so the one believed is the narrow leaf once and the wide one once.
        let mut believed = Vec::new();
        for (in_a, in_b) in [(1.0, 1.5), (1.5, 1.0)] {
            let dir = tempfile::tempdir().expect("a scratch directory");
            let files = [
                (
                    "levels/bank.ymap",
                    placing_map_bytes("bank", &square, &placed),
                ),
                (
                    "types/a_types.ytyp",
                    types_file("a_types", &[leaf(in_a), sofa("sofa_a")]),
                ),
                (
                    "types/b_types.ytyp",
                    types_file("b_types", &[leaf(in_b), sofa("sofa_b")]),
                ),
            ];
            let (game, index) = indexed(dir.path(), &files);
            assert_eq!(index.redefined(), 1, "the door is declared twice");
            let declared = index.archetype(hash(DOOR)).expect("the door is declared");
            let width = if declared.type_file == hash("a_types") {
                in_a
            } else {
                assert_eq!(declared.type_file, hash("b_types"));
                in_b
            };
            let doors = doors_of_the_bank(&MountedWorld::new(&game, dir.path()), &index);
            assert!(doors.unread.is_empty(), "{:?}", doors.unread);
            let [door] = &doors.doors[..] else {
                panic!("one door: {:?}", doors.doors);
            };
            let read = door.leaf.as_ref().map(|leaf| leaf.width);
            assert_eq!(read, Some(width), "{:?}", door.unread);
            believed.push(width);
        }
        assert_ne!(believed[0], believed[1]);
    }

    #[test]
    fn a_path_that_lies_where_the_copy_the_install_loads_does_names_it_and_another_copy_is_refused()
    {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let square = MapSpec::around(cube([0.0; 3], 50.0));
        let files = [("levels/bank.ymap", placing_map_bytes("bank", &square, &[]))];
        let (game, index) = indexed(dir.path(), &files);
        // The pack's own path answers the file the title update patches it with, and an older pack holds another copy.
        let (packs, older) = ("dlc_heist:/levels/BANK.ymap", "dlc_old:/levels/bank.ymap");
        let lies_at = |path: &str| match path {
            BANK | "dlc_heist:/levels/BANK.ymap" => Some("update.rpf/levels/bank.ymap".to_string()),
            "dlc_old:/levels/bank.ymap" => Some("old.rpf/levels/bank.ymap".to_string()),
            _ => None,
        };
        assert_eq!(listed_path(&index, &lies_at, BANK).as_deref(), Ok(BANK));
        let spelled = BANK.to_ascii_uppercase();
        assert_eq!(
            listed_path(&index, &|_| None, &spelled).as_deref(),
            Ok(BANK)
        );
        assert_eq!(listed_path(&index, &lies_at, packs).as_deref(), Ok(BANK));
        let refused = listed_path(&index, &lies_at, older).expect_err("another copy");
        assert!(
            refused.starts_with(&format!(
                "`{older}` is not the copy of that map the install loads"
            )) && refused.contains(&format!("at `{BANK}`")),
            "{refused}"
        );
        // A path that lies nowhere, and one of a map the index does not hold, are left for the search to refuse.
        for nowhere in ["dlc_gone:/levels/bank.ymap", "dlc_old:/levels/vault.ymap"] {
            assert_eq!(
                listed_path(&index, &lies_at, nowhere).as_deref(),
                Ok(nowhere)
            );
            let patterns = or_maps::doors::built_in_patterns().expect("the patterns read");
            let source = MountedWorld::new(&game, dir.path());
            let refused = doors_of_map(&source, &index, nowhere, &|_| None, &patterns)
                .expect_err("no such map file");
            assert!(refused.contains("holds no map file"), "{refused}");
        }
    }

    /// A mount but for one file, which it does not open.
    struct Without<'a> {
        whole: &'a dyn WorldSource,
        lacks: &'a str,
    }

    impl WorldSource for Without<'_> {
        fn identity(&self) -> String {
            self.whole.identity()
        }

        fn archives(&self) -> Vec<or_world_gta5::world_index::Archive> {
            self.whole.archives()
        }

        fn entries(&self, archive: usize, visit: &mut dyn FnMut(&or_title::vfs::EntryFacts)) {
            self.whole.entries(archive, visit);
        }

        fn open(&self, archive: usize, logical: &str) -> Option<Vec<u8>> {
            if logical == self.lacks {
                return None;
            }
            self.whole.open(archive, logical)
        }

        fn open_head(&self, archive: usize, logical: &str, limit: usize) -> Option<Vec<u8>> {
            self.whole.open_head(archive, logical, limit)
        }
    }

    #[test]
    fn a_type_file_that_does_not_open_is_said_once_and_its_doors_are_listed_with_no_leaf() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        // The map places the door and an interior that holds the door too, so the type file of the door is wanted twice.
        let bank = MapSpec {
            interiors: vec!["bank_interior"],
            ..MapSpec::around(cube([0.0; 3], 50.0))
        };
        let placed = [Placed::new(DOOR, [10.0, 10.0, 0.0], 0.0, 1)];
        let files = [
            (
                "levels/bank.ymap",
                placing_map_bytes("bank", &bank, &placed),
            ),
            (
                "types/bank_interior.ytyp",
                interior_types_bytes("bank_interior", "bank_interior", DOOR, [1.0, 2.0, 0.5]),
            ),
            (
                "types/bank_props.ytyp",
                types_file("bank_props", &[leaf(1.0)]),
            ),
        ];
        let (game, index) = indexed(dir.path(), &files);
        let whole = MountedWorld::new(&game, dir.path());
        let leaves = |doors: &SetDoors| -> Vec<bool> {
            (doors.doors.iter())
                .map(|door| door.leaf.is_some())
                .collect()
        };
        let read = doors_of_the_bank(&whole, &index);
        assert_eq!((leaves(&read), read.unread), (vec![true, true], vec![]));

        let props = index.type_file(hash("bank_props")).expect("indexed");
        let lacking = Without {
            whole: &whole,
            lacks: &props.logical,
        };
        let unread = doors_of_the_bank(&lacking, &index);
        // Both doors are still found, by their name, and neither has a box to read a leaf from.
        assert_eq!(leaves(&unread), [false, false]);
        assert_eq!(
            unread.unread,
            [(
                props.logical.clone(),
                "the archive that lists it does not open it".to_string()
            )]
        );
    }
}
