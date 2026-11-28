//! The first edit of an object the game placed imports its map into the project as a source that keeps the game's file as its base,
//! applies the edit, and is one record that is taken back whole.
//!
//! The install is built by hand as `or-depends` builds one for its world index: map files written as the bytes the game ships them
//! in, put in an archive and mounted with no game present. The project is made in a temporary folder with one maps content, and the
//! mount answers `first_edit::Game` through the world index, as the command line answers it.
//!
//! ⚠ The files are the crate's samples, so the clean checkout runs this. No sample fails its XML fixed point while it reads as a
//! map file, so the import's own refusal is held on a map file whose name no source folder can take.

mod common;
#[path = "../../or-world-gta5/tests/world_fixture/mod.rs"]
mod world_fixture;

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use or_io::meta::{MetaDocument, MetaInstance};
use or_maps::changes::{self, EntityChange, SourceChanges};
use or_maps::first_edit::{
    self, apply, plan, take_back, told, Change, Edit, Place, Record, Refusal, Which,
};
use or_maps::import::{add_files, Incoming};
use or_maps::operation::apply_all;
use or_maps::sample::{child_map_file, map_file, Entity};
use or_maps::Maps;
use or_project::{Container, Module, Output, Project, Target};
use or_title::MountedGame;
use or_world_gta5::execute::{identity, links_by_identity, Key};
use or_world_gta5::{MapData, MapGroup};
use or_world_gta5::{MountedWorld, WorldIndex};

const BOX: ([f32; 3], [f32; 3]) = ([0.0; 3], [100.0; 3]);

/// The entities of `m_town`: a bench, two lamps that state one guid, and two posts at one pose.
const BENCH: usize = 0;
const BENCH_GUID: u32 = 11;
const LAMP_GUID: u32 = 12;
const POST: usize = 3;
const POST_TWIN: usize = 4;
/// The entities of `block_lod`, and of `block` under it.
const LOD_SHOP: usize = 1;
const SHOP: usize = 0;

fn under<'a>(archetype: &'a str, position: [f32; 3], guid: u32, parent: usize) -> Entity<'a> {
    Entity {
        parent: parent as i32,
        in_parent_map: true,
        ..Entity::at(archetype, position, guid)
    }
}

fn counting<'a>(archetype: &'a str, position: [f32; 3], guid: u32, children: u32) -> Entity<'a> {
    Entity {
        children,
        ..Entity::at(archetype, position, guid)
    }
}

/// An install of one archive.
///
/// `m_town` stands alone. `shed` holds a shed under its far model in `shed_lod`, which stands for nothing else. `block_lod` holds the
/// far models of a row, a shop and a yard, in that order: `block` holds the shop, a house of the row and a fence of the yard,
/// `block_b` a house of the row and the yard, whose links name the far model before the shop's and the one after it, and `block_c`
/// a house of the row, whose link names the one before it. `garbage` is no map file, `ruin` holds a wall under a far model in
/// `ruin_lod`, which is no map file either, and `aux` is a map file whose name no source folder can take, with `a_hut` under it,
/// whose source is made before it. `void` is a map file that holds no entity.
fn install(dir: &Path) -> PathBuf {
    let root = dir.join("install");
    std::fs::create_dir(&root).unwrap();
    let town = map_file(
        "m_town",
        &[
            Entity::at("bench", [10.0, 20.0, 1.0], BENCH_GUID),
            Entity::at("lamp", [30.0, 20.0, 1.0], LAMP_GUID),
            Entity::at("lamp", [40.0, 20.0, 1.0], LAMP_GUID),
            Entity::at("post", [50.0, 20.0, 1.0], 13),
            Entity::at("post", [50.0, 20.0, 1.0], 14),
        ],
        BOX,
    );
    let shed_lod = map_file(
        "shed_lod",
        &[counting("lod_shed", [60.0, 60.0, 0.0], 500, 1)],
        BOX,
    );
    let shed = child_map_file(
        "shed",
        "shed_lod",
        &[
            under("shed", [60.0, 60.0, 0.0], 501, 0),
            Entity::at("barrel", [62.0, 60.0, 0.0], 502),
        ],
        BOX,
    );
    let block_lod = map_file(
        "block_lod",
        &[
            counting("lod_row", [20.0, 10.0, 0.0], 100, 3),
            counting("lod_shop", [10.0, 12.0, 3.0], 101, 1),
            counting("lod_yard", [90.0, 10.0, 0.0], 102, 2),
        ],
        BOX,
    );
    let block = child_map_file(
        "block",
        "block_lod",
        &[
            under("shop", [10.0, 12.0, 3.0], 201, LOD_SHOP),
            under("house", [18.0, 10.0, 0.0], 202, 0),
            Entity::at("crate", [5.0, 5.0, 0.0], 203),
            under("fence", [88.0, 10.0, 0.0], 204, 2),
        ],
        BOX,
    );
    let block_b = child_map_file(
        "block_b",
        "block_lod",
        &[
            under("house", [22.0, 10.0, 0.0], 301, 0),
            under("yard", [90.0, 10.0, 0.0], 302, 2),
        ],
        BOX,
    );
    let block_c = child_map_file(
        "block_c",
        "block_lod",
        &[under("house", [26.0, 10.0, 0.0], 401, 0)],
        BOX,
    );
    let ruin = child_map_file(
        "ruin",
        "ruin_lod",
        &[under("wall", [70.0, 70.0, 0.0], 601, 0)],
        BOX,
    );
    let aux = map_file(
        "aux",
        &[counting("lod_hut", [80.0, 80.0, 0.0], 700, 1)],
        BOX,
    );
    let a_hut = child_map_file(
        "a_hut",
        "aux",
        &[under("hut", [80.0, 80.0, 0.0], 701, 0)],
        BOX,
    );
    // A mount asks for the archive of common data, which holds no map.
    world_fixture::write_archive(&root, "common.rpf", &[("data/empty.txt", Vec::new())]);
    world_fixture::write_archive(
        &root,
        "x64a.rpf",
        &[
            ("levels/m_town.ymap", town),
            ("levels/shed_lod.ymap", shed_lod),
            ("levels/shed.ymap", shed),
            ("levels/block_lod.ymap", block_lod),
            ("levels/block.ymap", block),
            ("levels/block_b.ymap", block_b),
            ("levels/block_c.ymap", block_c),
            ("levels/garbage.ymap", b"not a map file".to_vec()),
            ("levels/ruin.ymap", ruin),
            ("levels/ruin_lod.ymap", b"PSINbroken".to_vec()),
            ("levels/aux.ymap", aux),
            ("levels/a_hut.ymap", a_hut),
            ("levels/void.ymap", map_file("void", &[], BOX)),
            ("levels/notes.txt", b"no map".to_vec()),
        ],
    );
    root
}

/// The mount as a first edit asks it, counting the files it gives.
struct Mounted<'a> {
    game: &'a MountedGame,
    index: WorldIndex,
    edition: Target,
    read: Cell<usize>,
}

impl<'a> Mounted<'a> {
    fn new(game: &'a MountedGame, root: &Path) -> Self {
        let (index, _) = WorldIndex::build(&MountedWorld::new(game, root), None);
        Self {
            game,
            index,
            edition: Target::Legacy,
            read: Cell::new(0),
        }
    }

    /// The logical path of the map called `name`.
    fn logical(&self, name: &str) -> String {
        let map = self.index.map(world_fixture::hash(name));
        map.unwrap_or_else(|| panic!("the install holds {name}"))
            .logical
            .clone()
    }
}

impl first_edit::Game for Mounted<'_> {
    fn edition(&self) -> Target {
        self.edition
    }

    fn read(&self, logical: &str) -> Option<Vec<u8>> {
        self.read.set(self.read.get() + 1);
        self.game.vfs.open(logical)
    }

    fn lies_at(&self, logical: &str) -> Option<String> {
        self.game.vfs.locate(logical)
    }

    fn map_file(&self, name: u32) -> Option<String> {
        self.index.map(name).map(|map| map.logical.clone())
    }

    fn maps_under(&self, parent: u32) -> Vec<String> {
        let under = self.index.maps().iter().filter(|map| map.parent == parent);
        under.map(|map| map.logical.clone()).collect()
    }
}

/// A project in `dir` with one maps content, in the folder `maps`, and the builds `outputs`.
struct Made {
    file: PathBuf,
    container: Container,
}

impl Made {
    fn new(dir: &Path, outputs: Vec<Output>) -> Self {
        let folder = dir.join("project");
        std::fs::create_dir(&folder).unwrap();
        let container = Container {
            modules: vec![Module::maps("maps", "maps")],
            outputs,
        };
        let file = folder.join("world.openrage-proj");
        Project::create(&file, &container, &[]).unwrap();
        or_maps::adopt::start_module(&folder, &container.modules[0]).unwrap();
        Self { file, container }
    }

    fn place(&self) -> Place<'_> {
        Place {
            project: &self.file,
            container: &self.container,
            module: &self.container.modules[0],
        }
    }

    fn folder(&self) -> PathBuf {
        self.file.parent().unwrap().to_path_buf()
    }

    fn maps(&self) -> PathBuf {
        self.folder().join("maps")
    }

    /// The source folders of the maps content, by name.
    fn sources(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.maps())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".src"))
            .collect();
        names.sort();
        names
    }
}

/// Every file under `folder`, by its path inside it with `/`.
fn files(folder: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, into: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, into);
            } else {
                let inside = path.strip_prefix(root).unwrap().to_string_lossy();
                into.insert(inside.replace('\\', "/"), std::fs::read(&path).unwrap());
            }
        }
    }
    let mut found = BTreeMap::new();
    walk(folder, folder, &mut found);
    found
}

/// The paths at which two readings of a folder differ.
fn differing(one: &BTreeMap<String, Vec<u8>>, other: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
    let paths: BTreeSet<&String> = one.keys().chain(other.keys()).collect();
    (paths.into_iter())
        .filter(|path| one.get(*path) != other.get(*path))
        .cloned()
        .collect()
}

fn moved(map: &str, entity: Which, position: [f32; 3]) -> Edit {
    Edit {
        map: map.to_string(),
        entity,
        change: Change::Move { position },
    }
}

fn deleted(map: &str, index: usize) -> Edit {
    Edit {
        map: map.to_string(),
        entity: Which::Index(index),
        change: Change::Delete,
    }
}

/// The lines of the managed block of the project's ignore rules.
fn ignored(project: &Made) -> Vec<String> {
    let text = std::fs::read_to_string(project.folder().join(".gitignore")).unwrap_or_default();
    (text.lines().map(str::to_string))
        .skip_while(|line| line != or_source::git::BEGIN)
        .collect()
}

/// The world as the game would load it with the project over it: the value tree of every map file of the install, from the
/// project's source where it holds one of that name.
fn world(project: &Made, mounted: &Mounted<'_>) -> BTreeMap<String, MetaInstance> {
    let mut trees = BTreeMap::new();
    // A map whose parent map does not read is left out, as nothing can say what its links name.
    let whole = |map: &&or_world_gta5::world_index::Map| {
        let parent = mounted.index.parent_of(map);
        map.read && parent.map_or(map.parent == 0, |parent| parent.read)
    };
    for map in mounted.index.maps().iter().filter(whole) {
        let source = project.maps().join(format!("{}.metadata.src", map.name));
        let bytes = if source.is_dir() {
            or_maps::facts::cooked(&source).expect("the source cooks")
        } else {
            mounted.game.vfs.open(&map.logical).expect("the file reads")
        };
        let document = MetaDocument::parse(or_io::meta::default_library(), bytes).unwrap();
        trees.insert(map.name.clone(), document.instance);
    }
    trees
}

/// Every LOD link of `trees` by the identity of its two ends, once the maps are held to the checks of one file and of the group.
fn links(trees: &BTreeMap<String, MetaInstance>) -> BTreeMap<Key, Key> {
    let mut group = MapGroup::new();
    for (name, tree) in trees {
        let map = MapData::new(tree).expect("a map file");
        assert_eq!(or_world_gta5::validate::check(&map), Vec::new(), "{name}");
        group.add_from(&map, name);
    }
    assert_eq!(group.check(), Vec::new(), "the group holds");
    assert_eq!(
        group.child_count_disagreements(),
        Vec::new(),
        "every count is the number of entities that name it"
    );
    let (inside, outside) = group.unresolved_links();
    assert_eq!(outside, 0, "of {inside} links into a parent map");
    links_by_identity(trees.values())
}

/// The identity of every entity of `trees`.
fn entities(trees: &BTreeMap<String, MetaInstance>) -> BTreeSet<Key> {
    (trees.values())
        .filter_map(MapData::new)
        .flat_map(|map| {
            let keys: Vec<Key> = (map.entities().map(|e| identity(e.instance()))).collect();
            keys
        })
        .collect()
}

#[test]
fn moving_an_entity_the_game_placed_imports_exactly_its_map_with_the_games_file_as_its_base() {
    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    let game = world_fixture::mount(&root);
    let mounted = Mounted::new(&game, &root);
    let project = Made::new(scratch.path(), Vec::new());
    let town = mounted.logical("m_town");

    let record = apply(
        &project.place(),
        &mounted,
        &moved(&town, Which::Index(BENCH), [12.0, 21.0, 1.5]),
    )
    .unwrap();

    // Exactly its map came in, and the content names it.
    assert!(record.done);
    let imported: Vec<(&str, &str)> = (record.imported.iter())
        .map(|i| (i.map.as_str(), i.source.as_str()))
        .collect();
    assert_eq!(imported, [(town.as_str(), "m_town.metadata.src")]);
    assert_eq!(project.sources(), ["m_town.metadata.src"]);
    let model = Maps::read(&project.maps()).unwrap();
    let named: Vec<&str> = or_maps::facts::named_paths(&model).collect();
    assert_eq!(named, ["m_town.metadata.src"]);

    // The source keeps the game's file as its base, byte for byte, and says where it was taken from.
    let dir = project.maps().join("m_town.metadata.src");
    let shipped = game.vfs.open(&town).unwrap();
    assert_eq!(std::fs::read(dir.join("base.ymap")).unwrap(), shipped);
    assert_eq!(
        record.imported[0].base_sha256,
        or_source::asset::sha256_hex(&shipped)
    );
    let manifest = or_source::asset::Source::open(&dir).unwrap().manifest;
    assert!(manifest.origin.from_game);
    // ⚠ The path inside the install's archives, which is what the check against the game reads the original at, and not the
    // path the mount loads it by.
    let lies_at = game
        .vfs
        .locate(&town)
        .expect("the mount says where it lies");
    assert_ne!(lies_at, town);
    assert!(
        lies_at.ends_with("x64a.rpf/levels/m_town.ymap"),
        "{lies_at}"
    );
    assert_eq!(
        manifest.origin.virtual_path.as_deref(),
        Some(lies_at.as_str())
    );

    // What changed of the base is that one move.
    let SourceChanges::Map(found) = changes::source_changes(&dir).unwrap() else {
        panic!("a map file");
    };
    assert!(found.added.is_empty() && found.deleted.is_empty() && found.ambiguous.is_empty());
    assert_eq!(found.unchanged, 4);
    let [kept] = found.changed.as_slice() else {
        panic!("one entity changed: {:#?}", found.changed);
    };
    assert_eq!((kept.base.index, kept.now.index), (BENCH, BENCH));
    let [EntityChange::Moved { from, to }] = kept.changes.as_slice() else {
        panic!("one move: {:#?}", kept.changes);
    };
    assert_eq!((*from, *to), ([10.0, 20.0, 1.0], [12.0, 21.0, 1.5]));

    // The one operation, and what the person is told: the map by name, and what it is to a repository.
    assert_eq!(record.operations.len(), 1);
    assert_eq!(record.operations[0].name(), "place_entity");
    assert_eq!(record.inverses.len(), 1);
    assert_eq!(record.told.line.key, told::MOVED);
    assert_eq!(record.told.line.text(), "Moved an object of m_town.ymap");
    let brought = record.told.imported.as_ref().unwrap();
    assert_eq!(brought.key, told::IMPORTED.0);
    assert_eq!(brought.text(), "m_town.ymap is now a map of this project");
    let sharing = record.told.sharing.as_ref().unwrap();
    assert_eq!(sharing.key, told::LEFT_OUT.0);
    assert_eq!(sharing.text(), told::LEFT_OUT.0);
    assert_eq!(record.told.undo.key, told::UNDO);

    // What the sentence says of the ignore rules is so: they name the source, and the check before sharing would find it.
    assert!(
        ignored(&project).contains(&"/maps/m_town.metadata.src/".to_string()),
        "{:?}",
        ignored(&project)
    );
    let rewrote: Vec<&str> = record.rewritten.iter().map(|r| r.file.as_str()).collect();
    assert_eq!(rewrote, ["maps/maps.json", ".gitignore", ".gitattributes"]);
}

#[test]
fn a_project_that_commits_the_games_sources_is_told_so() {
    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    let game = world_fixture::mount(&root);
    let mounted = Mounted::new(&game, &root);
    let project = Made::new(scratch.path(), Vec::new());
    let commit = or_source::git::Commit {
        game_sources: true,
        ..Default::default()
    };
    or_source::git::choose(&project.folder(), commit).unwrap();

    let town = mounted.logical("m_town");
    let edit = moved(&town, Which::Index(BENCH), [12.0, 21.0, 1.5]);
    let record = apply(&project.place(), &mounted, &edit).unwrap();
    assert_eq!(record.told.sharing.unwrap().key, told::COMMITTED.0);
    assert!(
        !ignored(&project).iter().any(|line| line.contains("m_town")),
        "{:?}",
        ignored(&project)
    );
}

#[test]
fn taking_a_first_edit_back_leaves_no_source_and_the_projects_files_as_they_were() {
    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    let game = world_fixture::mount(&root);
    let mounted = Mounted::new(&game, &root);
    let project = Made::new(scratch.path(), Vec::new());
    let before = files(&project.folder());

    let town = mounted.logical("m_town");
    let edit = moved(&town, Which::Guid(BENCH_GUID), [12.0, 21.0, 1.5]);
    let record = apply(&project.place(), &mounted, &edit).unwrap();
    assert_ne!(
        differing(&before, &files(&project.folder())),
        Vec::<String>::new()
    );

    // ⚠ Through its JSON, as a step kept on disk is read after the project is opened again.
    let kept = Record::from_json(record.to_json().unwrap().as_bytes()).unwrap();
    assert_eq!(kept, record);
    let taken = take_back(&project.place(), &kept).unwrap();
    assert_eq!(taken.removed, ["m_town.metadata.src"]);
    assert_eq!(taken.restored, Vec::<String>::new());
    assert_eq!(
        taken.rewritten,
        [".gitattributes", ".gitignore", "maps/maps.json"]
    );
    assert_eq!(taken.notes, Vec::<String>::new());
    assert_eq!(project.sources(), Vec::<String>::new());
    assert_eq!(
        differing(&before, &files(&project.folder())),
        Vec::<String>::new()
    );

    // A record of a plan wrote nothing and takes nothing back, and a record of another format is not read as one.
    let planned = plan(&project.place(), &mounted, &edit).unwrap().record;
    assert!(matches!(
        take_back(&project.place(), &planned),
        Err(Refusal::NotDone)
    ));
    let other = record.to_json().unwrap().replace(first_edit::FORMAT, "x");
    assert!(matches!(
        Record::from_json(other.as_bytes()),
        Err(Refusal::Format(_))
    ));
}

#[test]
fn deleting_a_building_whose_far_model_is_in_the_parent_map_imports_both_map_files() {
    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    let game = world_fixture::mount(&root);
    let mounted = Mounted::new(&game, &root);
    let project = Made::new(scratch.path(), Vec::new());
    let before = files(&project.folder());
    let (links_before, entities_before) = {
        let trees = world(&project, &mounted);
        (links(&trees), entities(&trees))
    };

    let record = apply(
        &project.place(),
        &mounted,
        &deleted(&mounted.logical("shed"), 0),
    )
    .unwrap();
    assert_eq!(
        project.sources(),
        ["shed.metadata.src", "shed_lod.metadata.src"]
    );
    let imported: BTreeSet<&str> = record.imported.iter().map(|i| i.map.as_str()).collect();
    let both = [mounted.logical("shed"), mounted.logical("shed_lod")];
    assert_eq!(imported, both.iter().map(String::as_str).collect());
    assert_eq!(
        record.told.line.text(),
        "Deleted an object of shed.ymap with its far models: 2 entities"
    );
    assert_eq!(
        record.told.imported.as_ref().unwrap().text(),
        "2 maps of the game are now maps of this project"
    );
    assert_eq!(record.told.sharing.as_ref().unwrap().key, told::LEFT_OUT.0);
    assert_eq!(
        record.told.sharing.as_ref().unwrap().text(),
        told::LEFT_OUT.1
    );

    // The world with the project over it validates: the shed and its far model are gone, and every other link joins what it joined.
    let trees = world(&project, &mounted);
    let gone: BTreeSet<Key> = (entities_before.difference(&entities(&trees)).copied()).collect();
    assert_eq!(gone.len(), 2);
    let mut expected = links_before.clone();
    expected.retain(|child, parent| !gone.contains(child) && !gone.contains(parent));
    assert_eq!(links(&trees), expected);
    assert_eq!(links_before.len() - expected.len(), 1);
    // The rules of the content find nothing that stops a build.
    let model = Maps::read(&project.maps()).unwrap();
    let facts = or_maps::inspect(&project.maps(), &model, or_maps::Depth::Open);
    let blocking: Vec<_> = (or_maps::check(&model, &facts, None).into_iter())
        .filter(|finding| finding.blocks())
        .collect();
    assert!(blocking.is_empty(), "{blocking:#?}");

    take_back(&project.place(), &record).unwrap();
    assert_eq!(
        differing(&before, &files(&project.folder())),
        Vec::<String>::new()
    );
}

#[test]
fn a_deletion_imports_each_map_file_it_changes_and_no_map_file_it_only_read() {
    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    let game = world_fixture::mount(&root);
    let mounted = Mounted::new(&game, &root);
    let project = Made::new(scratch.path(), Vec::new());
    let before = files(&project.folder());
    let (links_before, entities_before) = {
        let trees = world(&project, &mounted);
        (links(&trees), entities(&trees))
    };

    // The shop goes with its far model, the second of `block_lod`: the yard of `block_b` names the third, which becomes the second,
    // and the house of `block_c` names the first, which stays where it is.
    let edit = deleted(&mounted.logical("block"), SHOP);
    let planned = plan(&project.place(), &mounted, &edit).unwrap();
    assert_eq!(
        differing(&before, &files(&project.folder())),
        Vec::<String>::new(),
        "a plan writes nothing"
    );
    let record = apply(&project.place(), &mounted, &edit).unwrap();
    let sources = [
        "block.metadata.src",
        "block_b.metadata.src",
        "block_lod.metadata.src",
    ];
    assert_eq!(project.sources(), sources);
    // What the plan said is what was done.
    assert!(!planned.record.done && planned.record.inverses.is_empty());
    assert_eq!(planned.record.operations, record.operations);
    let said = |record: &Record| -> Vec<(String, String, String)> {
        (record.imported.iter())
            .map(|i| (i.map.clone(), i.source.clone(), i.base_sha256.clone()))
            .collect()
    };
    assert_eq!(said(&planned.record), said(&record));
    assert_eq!(planned.record.told, record.told);
    let goes = &planned.deletion.as_ref().unwrap().goes;
    assert_eq!(goes.len(), 2);
    // Two files lose an entity, and two hold a link into `block_lod` past the far model that goes: `block` is both.
    let mut done: Vec<(&str, &str)> = (record.operations.iter())
        .map(|operation| (operation.source(), operation.name()))
        .collect();
    done.sort_unstable();
    assert_eq!(
        done,
        [
            ("block.metadata.src", "remove_entities"),
            ("block.metadata.src", "renumber_for_removed"),
            ("block_b.metadata.src", "renumber_for_removed"),
            ("block_lod.metadata.src", "remove_entities"),
        ]
    );

    let trees = world(&project, &mounted);
    let gone: BTreeSet<Key> = (entities_before.difference(&entities(&trees)).copied()).collect();
    assert_eq!(gone.len(), 2);
    let mut expected = links_before.clone();
    expected.retain(|child, parent| !gone.contains(child) && !gone.contains(parent));
    assert_eq!(links(&trees), expected);

    // The inverses alone put every byte of every source back to what the import made, which is what lets the sources go.
    let imported = files(&project.folder());
    apply_all(&project.maps(), &record.inverses, false).unwrap();
    for made in &record.imported {
        let dir = project.maps().join(&made.source);
        assert_eq!(
            or_source::game_import::tree_digest(&dir).unwrap(),
            made.digest,
            "{}",
            made.source
        );
        let shipped = game.vfs.open(&made.map).unwrap();
        assert_eq!(std::fs::read(dir.join("base.ymap")).unwrap(), shipped);
        assert!(changes::source_changes(&dir).unwrap().is_empty());
    }
    let changed = differing(&imported, &files(&project.folder()));
    let xml = sources.map(|source| format!("maps/{source}/metadata.xml"));
    assert_eq!(changed, xml, "the inverses write the XML of each source");
}

#[test]
fn a_second_edit_in_a_map_the_project_holds_imports_nothing_and_is_taken_back_first() {
    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    let game = world_fixture::mount(&root);
    let mounted = Mounted::new(&game, &root);
    let project = Made::new(scratch.path(), Vec::new());
    let before = files(&project.folder());

    // The far model of the shop is moved, which imports `block_lod` alone.
    let lod = mounted.logical("block_lod");
    let first = apply(
        &project.place(),
        &mounted,
        &moved(&lod, Which::Index(LOD_SHOP), [11.0, 12.0, 3.0]),
    )
    .unwrap();
    assert_eq!(project.sources(), ["block_lod.metadata.src"]);
    let after_first = files(&project.folder());

    // The shop is deleted: its far model is in a source of the project now, which is edited there, and two more map files come in.
    let second = apply(
        &project.place(),
        &mounted,
        &deleted(&mounted.logical("block"), SHOP),
    )
    .unwrap();
    let imported: Vec<&str> = second.imported.iter().map(|i| i.source.as_str()).collect();
    assert_eq!(imported, ["block.metadata.src", "block_b.metadata.src"]);
    assert!(
        (second.operations.iter()).any(|op| op.source() == "block_lod.metadata.src"),
        "{:#?}",
        second.operations
    );

    // A third, of an entity named by the game's path in a map the project holds, imports nothing, and reads of the game the one
    // file its place was given in.
    let read = mounted.read.get();
    let third = apply(
        &project.place(),
        &mounted,
        &moved(&lod, Which::Index(0), [21.0, 10.0, 0.0]),
    )
    .unwrap();
    assert_eq!(third.imported, Vec::new());
    assert_eq!(third.told.imported, None);
    assert_eq!(third.told.sharing, None);
    assert_eq!(third.rewritten, Vec::new());
    assert_eq!(third.entity.source, "block_lod.metadata.src");
    assert_eq!(
        mounted.read.get(),
        read + 1,
        "the game's file was read once, to carry the place, and no other"
    );
    assert_eq!(project.sources().len(), 3);

    // An earlier record is refused while a later edit of one of its sources stands, and nothing is changed by the refusal.
    let standing = files(&project.folder());
    for earlier in [&first, &second] {
        let refused = take_back(&project.place(), earlier).unwrap_err();
        assert!(matches!(refused, Refusal::Stale(_)), "{refused}");
        assert!(refused.to_string().contains("nothing was taken back"));
        assert_eq!(
            differing(&standing, &files(&project.folder())),
            Vec::<String>::new()
        );
    }

    // Newest first, each leaves what the one before it left: the source the second changed and did not make is put back, not removed.
    let taken = take_back(&project.place(), &third).unwrap();
    assert_eq!(taken.removed, Vec::<String>::new());
    assert_eq!(taken.restored, ["block_lod.metadata.src"]);
    let taken = take_back(&project.place(), &second).unwrap();
    assert_eq!(
        taken.removed,
        ["block.metadata.src", "block_b.metadata.src"]
    );
    assert_eq!(taken.restored, ["block_lod.metadata.src"]);
    assert_eq!(
        differing(&after_first, &files(&project.folder())),
        Vec::<String>::new()
    );
    let taken = take_back(&project.place(), &first).unwrap();
    assert_eq!(taken.removed, ["block_lod.metadata.src"]);
    assert_eq!(
        differing(&before, &files(&project.folder())),
        Vec::<String>::new()
    );
}

#[test]
fn an_edit_of_an_object_the_project_added_imports_nothing() {
    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    let game = world_fixture::mount(&root);
    let mounted = Mounted::new(&game, &root);
    let project = Made::new(scratch.path(), Vec::new());
    // A map a person made, which the game has none of.
    let mine = map_file(
        "mine",
        &[
            Entity::at("statue", [1.0, 2.0, 3.0], 1),
            Entity::at("statue", [4.0, 5.0, 6.0], 2),
        ],
        BOX,
    );
    let added = add_files(
        &project.maps(),
        &[Incoming {
            name: "mine.ymap",
            bytes: &mine,
            from_game: false,
            virtual_path: None,
        }],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::new());
    let before = files(&project.folder());

    for edit in [
        moved("mine.metadata.src", Which::Guid(2), [7.0, 8.0, 9.0]),
        deleted("mine.metadata.src", 0),
    ] {
        let record = apply(&project.place(), &mounted, &edit).unwrap();
        assert_eq!(record.imported, Vec::new());
        assert_eq!(record.rewritten, Vec::new());
        assert_eq!(record.told.imported, None);
        assert_eq!(record.told.sharing, None);
        assert_eq!(project.sources(), ["mine.metadata.src"]);
        let changed = differing(&before, &files(&project.folder()));
        assert_eq!(changed, ["maps/mine.metadata.src/metadata.xml"]);
        let taken = take_back(&project.place(), &record).unwrap();
        assert_eq!(taken.removed, Vec::<String>::new());
        assert_eq!(taken.restored, ["mine.metadata.src"]);
        assert_eq!(
            differing(&before, &files(&project.folder())),
            Vec::<String>::new()
        );
    }
    assert_eq!(mounted.read.get(), 0, "no file of the game was read");
}

#[test]
fn a_map_that_cannot_become_a_source_refuses_the_whole_edit_by_name_and_writes_nothing() {
    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    let game = world_fixture::mount(&root);
    let mounted = Mounted::new(&game, &root);
    let project = Made::new(scratch.path(), Vec::new());
    let before = files(&project.folder());
    let unchanged = |what: &str| {
        let changed = differing(&before, &files(&project.folder()));
        assert_eq!(changed, Vec::<String>::new(), "{what}");
    };

    // The hut's far model is in a map file that reads and that the import refuses: the hut's own map, which it takes, is not kept.
    let aux = mounted.logical("aux");
    let edit = deleted(&mounted.logical("a_hut"), 0);
    for refused in [
        plan(&project.place(), &mounted, &edit).map(drop),
        apply(&project.place(), &mounted, &edit).map(drop),
    ] {
        let refused = refused.unwrap_err();
        let Refusal::NotASource { file, why } = &refused else {
            panic!("{refused}");
        };
        assert_eq!(file, &aux);
        assert!(why.contains("device name"), "{why}");
        assert!(refused.to_string().contains(&aux), "{refused}");
        unchanged("a map file the import refuses");
    }

    // A map file that does not read, the entity's own and the one its chain leads into.
    let garbage = mounted.logical("garbage");
    for (edit, file) in [
        (moved(&garbage, Which::Index(0), [1.0; 3]), garbage.clone()),
        (
            deleted(&mounted.logical("ruin"), 0),
            mounted.logical("ruin_lod"),
        ),
    ] {
        for refused in [
            plan(&project.place(), &mounted, &edit).map(drop),
            apply(&project.place(), &mounted, &edit).map(drop),
        ] {
            let refused = refused.unwrap_err();
            let Refusal::Unread { file: named, .. } = &refused else {
                panic!("{refused}");
            };
            assert_eq!(named, &file);
            unchanged("a map file that does not read");
        }
    }

    // A file that is no map file, and a path the install holds nothing at.
    let refused = apply(
        &project.place(),
        &mounted,
        &moved(
            &garbage.replace("garbage.ymap", "notes.txt"),
            Which::Index(0),
            [1.0; 3],
        ),
    );
    assert!(
        matches!(refused, Err(Refusal::NotAMap { .. })),
        "{refused:?}"
    );
    let refused = apply(
        &project.place(),
        &mounted,
        &moved(
            &garbage.replace("garbage", "nowhere"),
            Which::Index(0),
            [1.0; 3],
        ),
    );
    assert!(
        matches!(refused, Err(Refusal::NoSuchMap { .. })),
        "{refused:?}"
    );
    // An entity that is not there, and a position no file stores.
    let town = mounted.logical("m_town");
    let refused = apply(
        &project.place(),
        &mounted,
        &moved(&town, Which::Index(5), [1.0; 3]),
    );
    assert!(
        matches!(
            refused,
            Err(Refusal::NoSuchEntity {
                index: 5,
                len: 5,
                ..
            })
        ),
        "{refused:?}"
    );
    let refused = apply(
        &project.place(),
        &mounted,
        &moved(&town, Which::Index(0), [f32::NAN, 0.0, 0.0]),
    );
    assert!(matches!(refused, Err(Refusal::NotFinite)), "{refused:?}");
    unchanged("an edit refused for what it names");

    // A map of the content's own under the name of the game's stands in its place.
    let own = map_file("m_town", &[Entity::at("statue", [1.0, 2.0, 3.0], 1)], BOX);
    let incoming = Incoming {
        name: "m_town.ymap",
        bytes: &own,
        from_game: false,
        virtual_path: None,
    };
    add_files(&project.maps(), &[incoming], None).unwrap();
    let with_own = files(&project.folder());
    // ⚠ The entity named is past what the content's own map holds: it is not looked for there.
    for edit in [moved(&town, Which::Index(4), [1.0; 3]), deleted(&town, 4)] {
        let refused = apply(&project.place(), &mounted, &edit).unwrap_err();
        assert!(matches!(refused, Refusal::InTheWay { .. }), "{refused}");
        assert_eq!(
            differing(&with_own, &files(&project.folder())),
            Vec::<String>::new()
        );
    }
}

#[test]
fn an_entity_named_by_a_guid_two_entities_of_its_map_state_is_refused_as_ambiguous() {
    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    let game = world_fixture::mount(&root);
    let mounted = Mounted::new(&game, &root);
    let project = Made::new(scratch.path(), Vec::new());
    let before = files(&project.folder());
    let town = mounted.logical("m_town");

    for change in [
        Change::Move {
            position: [1.0, 2.0, 3.0],
        },
        Change::Delete,
    ] {
        let edit = Edit {
            map: town.clone(),
            entity: Which::Guid(LAMP_GUID),
            change,
        };
        for refused in [
            plan(&project.place(), &mounted, &edit).map(drop),
            apply(&project.place(), &mounted, &edit).map(drop),
        ] {
            let refused = refused.unwrap_err();
            let Refusal::AmbiguousGuid { guid, entities, .. } = &refused else {
                panic!("{refused}");
            };
            assert_eq!((*guid, entities.as_slice()), (LAMP_GUID, [1, 2].as_slice()));
            assert!(
                refused.to_string().contains("entities 1 and 2"),
                "{refused}"
            );
        }
    }
    let edit = moved(&town, Which::Guid(999), [1.0; 3]);
    let refused = apply(&project.place(), &mounted, &edit).unwrap_err();
    assert!(
        matches!(refused, Refusal::NoSuchGuid { guid: 999, .. }),
        "{refused}"
    );
    assert_eq!(
        differing(&before, &files(&project.folder())),
        Vec::<String>::new()
    );

    // An entity that another of its archetype stands at the pose of is not deleted alone, and is moved by its index.
    let refused = apply(&project.place(), &mounted, &deleted(&town, POST)).unwrap_err();
    let Refusal::Twin { index, twins, .. } = &refused else {
        panic!("{refused}");
    };
    assert_eq!(*index, POST);
    let twin: Vec<usize> = twins.iter().map(|twin| twin.index).collect();
    assert_eq!(twin, [POST_TWIN]);
    assert_eq!(
        differing(&before, &files(&project.folder())),
        Vec::<String>::new()
    );

    // The guid one entity states names it.
    let edit = moved(&town, Which::Guid(BENCH_GUID), [1.0; 3]);
    let record = apply(&project.place(), &mounted, &edit).unwrap();
    assert_eq!(record.entity.index, BENCH);
}

#[test]
fn a_map_of_an_install_of_the_other_edition_than_a_build_targets_is_refused_with_the_way_out() {
    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    let game = world_fixture::mount(&root);
    let mounted = Mounted::new(&game, &root);
    let enhanced = Output {
        target: Some(Target::Enhanced),
        ..Output::fivem_resource("server", "server", &["maps"])
    };
    // A build that states no edition, and one of the other edition that does not take the content, refuse nothing.
    let any = Output::fivem_resource("any", "any", &["maps"]);
    let elsewhere = Output {
        target: Some(Target::Enhanced),
        ..Output::fivem_resource("other", "other", &["clothes"])
    };
    let project = Made::new(scratch.path(), vec![any, elsewhere, enhanced]);
    let before = files(&project.folder());
    let town = mounted.logical("m_town");

    for edit in [
        moved(&town, Which::Index(BENCH), [1.0, 2.0, 3.0]),
        deleted(&mounted.logical("shed"), 0),
    ] {
        for refused in [
            plan(&project.place(), &mounted, &edit).map(drop),
            apply(&project.place(), &mounted, &edit).map(drop),
        ] {
            let refused = refused.unwrap_err();
            let Refusal::OtherEdition { build, .. } = &refused else {
                panic!("{refused}");
            };
            assert_eq!(build, "server");
            let said = refused.to_string();
            assert!(said.contains("a map of a GTA V Legacy install"), "{said}");
            assert!(
                said.contains("edit this object from an install of GTA V Enhanced"),
                "{said}"
            );
            assert_eq!(
                differing(&before, &files(&project.folder())),
                Vec::<String>::new()
            );
        }
    }

    // From an install of the edition the build targets, the edit goes through.
    let of_the_target = Mounted {
        edition: Target::Enhanced,
        ..Mounted::new(&game, &root)
    };
    let edit = moved(&town, Which::Index(BENCH), [1.0, 2.0, 3.0]);
    apply(&project.place(), &of_the_target, &edit).unwrap();
}

#[cfg(unix)]
#[test]
fn an_edit_the_disk_refuses_after_the_import_leaves_the_project_as_it_was() {
    use std::os::unix::fs::PermissionsExt;

    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    let game = world_fixture::mount(&root);
    let mounted = Mounted::new(&game, &root);
    let project = Made::new(scratch.path(), Vec::new());
    let lod = mounted.logical("block_lod");
    let edit = moved(&lod, Which::Index(LOD_SHOP), [11.0, 12.0, 3.0]);
    apply(&project.place(), &mounted, &edit).unwrap();
    let before = files(&project.folder());

    // The source the deletion changes and does not import cannot be written, which only the flush finds out, after two imports.
    let held = project.maps().join("block_lod.metadata.src");
    let writable = std::fs::metadata(&held).unwrap().permissions();
    let xml = held.join("metadata.xml");
    let xml_writable = std::fs::metadata(&xml).unwrap().permissions();
    std::fs::set_permissions(&xml, std::fs::Permissions::from_mode(0o444)).unwrap();
    std::fs::set_permissions(&held, std::fs::Permissions::from_mode(0o555)).unwrap();
    let refused = apply(
        &project.place(),
        &mounted,
        &deleted(&mounted.logical("block"), SHOP),
    );
    std::fs::set_permissions(&held, writable).unwrap();
    std::fs::set_permissions(&xml, xml_writable).unwrap();

    let refused = refused.unwrap_err();
    assert!(matches!(refused, Refusal::Operation(_)), "{refused}");
    assert_eq!(project.sources(), ["block_lod.metadata.src"]);
    assert_eq!(
        differing(&before, &files(&project.folder())),
        Vec::<String>::new()
    );
}

#[test]
fn a_source_changed_in_more_than_the_edit_wrote_is_not_removed_and_the_edit_stands() {
    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    let game = world_fixture::mount(&root);
    let mounted = Mounted::new(&game, &root);
    let project = Made::new(scratch.path(), Vec::new());
    let town = mounted.logical("m_town");
    let edit = moved(&town, Which::Index(BENCH), [12.0, 21.0, 1.5]);
    let record = apply(&project.place(), &mounted, &edit).unwrap();

    // A person's file beside the XML is theirs: the source is not removed over it, and the edit is put back.
    let notes = project.maps().join("m_town.metadata.src/notes.txt");
    std::fs::write(&notes, "mine").unwrap();
    let standing = files(&project.folder());
    let refused = take_back(&project.place(), &record).unwrap_err();
    let Refusal::Touched { sources, added } = &refused else {
        panic!("{refused}");
    };
    assert_eq!(sources, &["m_town.metadata.src"]);
    // The file at fault is named, so the person knows what to move away.
    assert_eq!(added, &["m_town.metadata.src/notes.txt"]);
    assert!(refused
        .to_string()
        .contains("the edit did not make `m_town.metadata.src/notes.txt`"));
    assert_eq!(
        differing(&standing, &files(&project.folder())),
        Vec::<String>::new()
    );

    // Once it is as the edit left it, the record is taken back; a `maps.json` changed meanwhile keeps what was added to it.
    // ⚠ What a file browser left in the folder is no person's work and holds nothing back.
    std::fs::remove_file(&notes).unwrap();
    std::fs::write(notes.with_file_name(".DS_Store"), b"\0\0\0\x01Bud1").unwrap();
    let mut model = Maps::read(&project.maps()).unwrap();
    model.maps.push(or_maps::MapSet::new("later"));
    model.write(&project.maps()).unwrap();
    let taken = take_back(&project.place(), &record).unwrap();
    assert_eq!(taken.removed, ["m_town.metadata.src"]);
    assert_eq!(taken.rewritten, [".gitattributes", ".gitignore"]);
    assert_eq!(taken.notes.len(), 1, "{:?}", taken.notes);
    assert!(
        taken.notes[0].contains("maps/maps.json"),
        "{:?}",
        taken.notes
    );
    let after = Maps::read(&project.maps()).unwrap();
    let names: Vec<&str> = after.maps.iter().map(|set| set.name.as_str()).collect();
    assert_eq!(names, ["later"]);
}

#[test]
fn the_lines_of_a_record_say_what_its_json_states() {
    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    let game = world_fixture::mount(&root);
    let mounted = Mounted::new(&game, &root);
    let project = Made::new(scratch.path(), Vec::new());
    let edit = deleted(&mounted.logical("block"), SHOP);
    let planned = plan(&project.place(), &mounted, &edit).unwrap().record;
    let done = apply(&project.place(), &mounted, &edit).unwrap();

    for record in [&planned, &done] {
        let lines = record.lines();
        let json: serde_json::Value = serde_json::from_str(&record.to_json().unwrap()).unwrap();
        // Each entry of the document is one line of the text that no other entry is.
        let mut matched = vec![false; lines.len()];
        let mut claim = |is: &dyn Fn(&str) -> bool, what: String| {
            let at: Vec<usize> = (0..lines.len())
                .filter(|at| !matched[*at] && is(&lines[*at]))
                .collect();
            assert_eq!(at.len(), 1, "{what} is one line of:\n{lines:#?}");
            matched[at[0]] = true;
        };
        let text = |value: &serde_json::Value| value.as_str().unwrap().to_string();
        let (imported, did) = match record.done {
            true => ("imported", "did"),
            false => ("would import", "would do"),
        };
        let entity = format!(
            "entity {} of {}",
            json["entity"]["index"],
            text(&json["entity"]["source"])
        );
        claim(
            &|line| line.contains(&text(&json["edit"]["map"])) && line.ends_with(&entity),
            entity.clone(),
        );
        assert_eq!(json["imported"].as_array().unwrap().len(), 3);
        for import in json["imported"].as_array().unwrap() {
            let begins = format!(
                "{imported} {} as {}:",
                text(&import["map"]),
                text(&import["source"])
            );
            let sha = text(&import["base_sha256"]);
            claim(
                &|line| line.starts_with(&begins) && line.contains(&sha),
                begins.clone(),
            );
        }
        for (at, operation) in json["operations"].as_array().unwrap().iter().enumerate() {
            let whole = format!(
                "{did} {}. {} of {}",
                at + 1,
                text(&operation["op"]),
                text(&operation["source"])
            );
            claim(&|line| line == whole, whole.clone());
        }
        for note in json["notes"].as_array().unwrap() {
            let whole = format!("note: {}", text(note));
            claim(&|line| line == whole, whole.clone());
        }
        for rewritten in json["rewritten"].as_array().unwrap() {
            let whole = format!("rewrote {}", text(&rewritten["file"]));
            claim(&|line| line == whole, whole.clone());
        }
        for message in [&record.told.line]
            .into_iter()
            .chain(&record.told.imported)
            .chain(&record.told.sharing)
        {
            let whole = format!("told: {}", message.text());
            claim(&|line| line == whole, whole.clone());
        }
        if record.done {
            let begins = format!("{}: ", text(&json["told"]["undo"]["key"]));
            let inverses = json["inverses"].as_array().unwrap().len();
            let counts =
                format!("by {inverses} inverse operation(s) and the removal of 3 source(s)");
            claim(
                &|line| line.starts_with(&begins) && line.ends_with(&counts),
                begins.clone(),
            );
            assert_eq!(json["rewritten"].as_array().unwrap().len(), 3);
        } else {
            assert_eq!(json["inverses"].as_array().unwrap().len(), 0);
            assert_eq!(json["rewritten"].as_array().unwrap().len(), 0);
        }
        assert!(
            matched.iter().all(|matched| *matched),
            "a line no entry states: {lines:#?}"
        );
    }
}

#[test]
fn a_deletion_in_a_map_the_project_holds_imports_nothing_of_an_install_of_the_other_edition() {
    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    let game = world_fixture::mount(&root);
    let enhanced = Output {
        target: Some(Target::Enhanced),
        ..Output::fivem_resource("server", "server", &["maps"])
    };
    let project = Made::new(scratch.path(), vec![enhanced]);
    let of_the_target = Mounted {
        edition: Target::Enhanced,
        ..Mounted::new(&game, &root)
    };
    let lod = of_the_target.logical("block_lod");
    let edit = moved(&lod, Which::Index(0), [21.0, 10.0, 0.0]);
    apply(&project.place(), &of_the_target, &edit).unwrap();
    let before = files(&project.folder());

    // The far model of the shop is in a source of the project, and the maps under it that the deletion changes are the install's.
    let other = Mounted::new(&game, &root);
    let edit = deleted("block_lod.metadata.src", LOD_SHOP);
    for refused in [
        plan(&project.place(), &other, &edit).map(drop),
        apply(&project.place(), &other, &edit).map(drop),
    ] {
        let refused = refused.unwrap_err();
        let Refusal::OtherEdition { file, build, .. } = &refused else {
            panic!("{refused}");
        };
        assert_eq!(build, "server");
        assert!(file.ends_with(".ymap"), "{file}");
        assert_eq!(
            differing(&before, &files(&project.folder())),
            Vec::<String>::new()
        );
    }
    // An edit that imports nothing asks nothing of the install's edition.
    let edit = moved("block_lod.metadata.src", Which::Index(0), [22.0, 10.0, 0.0]);
    apply(&project.place(), &other, &edit).unwrap();
}

#[test]
fn a_folder_in_the_place_a_map_of_the_game_would_take_refuses_the_plan_as_it_refuses_the_edit() {
    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    let game = world_fixture::mount(&root);
    let mounted = Mounted::new(&game, &root);
    let project = Made::new(scratch.path(), Vec::new());

    // A folder no set names lies where the far model's map would come in: the shed's own map, which would come in first, does not.
    let orphan = project.maps().join("shed_lod.metadata.src");
    std::fs::create_dir(&orphan).unwrap();
    std::fs::write(orphan.join("notes.txt"), "mine").unwrap();
    let before = files(&project.folder());
    let edit = deleted(&mounted.logical("shed"), 0);
    for refused in [
        plan(&project.place(), &mounted, &edit).map(drop),
        apply(&project.place(), &mounted, &edit).map(drop),
    ] {
        let refused = refused.expect_err("a folder is in the way");
        let Refusal::InTheWay { file, source_there } = &refused else {
            panic!("{refused}");
        };
        assert_eq!(file, &mounted.logical("shed_lod"));
        assert_eq!(source_there, "shed_lod.metadata.src");
        assert_eq!(
            differing(&before, &files(&project.folder())),
            Vec::<String>::new()
        );
    }
    std::fs::remove_dir_all(&orphan).unwrap();

    // A source of the game's map that a folder of the content keeps, taken with no word of where from, stands for it: a second
    // source of the one map is not made beside it.
    let town = mounted.logical("m_town");
    let shipped = game.vfs.open(&town).unwrap();
    let incoming = Incoming {
        name: "m_town.ymap",
        bytes: &shipped,
        from_game: true,
        virtual_path: None,
    };
    let dir = or_maps::import::add_one(&project.maps().join("city"), &incoming).unwrap();
    or_maps::import::learn(&project.maps(), &[dir], None).unwrap();
    let before = files(&project.folder());
    let edit = moved(&town, Which::Index(BENCH), [12.0, 21.0, 1.5]);
    for refused in [
        plan(&project.place(), &mounted, &edit).map(drop),
        apply(&project.place(), &mounted, &edit).map(drop),
    ] {
        let refused = refused.expect_err("the content holds the map under a folder");
        let Refusal::InTheWay { source_there, .. } = &refused else {
            panic!("{refused}");
        };
        assert_eq!(source_there, "city/m_town.metadata.src");
        assert!(
            refused.to_string().contains("city/m_town.metadata.src"),
            "{refused}"
        );
        assert_eq!(
            differing(&before, &files(&project.folder())),
            Vec::<String>::new()
        );
    }
    // Named as the content names it, the object is edited there.
    let edit = moved(
        "city/m_town.metadata.src",
        Which::Index(BENCH),
        [12.0, 21.0, 1.5],
    );
    let record = apply(&project.place(), &mounted, &edit).unwrap();
    assert_eq!(record.imported, Vec::new());
}

#[test]
fn a_record_that_names_a_folder_outside_the_content_removes_nothing() {
    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    let game = world_fixture::mount(&root);
    let mounted = Mounted::new(&game, &root);
    let project = Made::new(scratch.path(), Vec::new());
    let town = mounted.logical("m_town");
    let edit = moved(&town, Which::Index(BENCH), [12.0, 21.0, 1.5]);
    let done = apply(&project.place(), &mounted, &edit).unwrap();
    let standing = files(&project.folder());

    // A record is a file anyone can write: one that states the digest of a folder elsewhere must not remove that folder.
    let victim = scratch.path().join("victim");
    std::fs::create_dir(&victim).unwrap();
    std::fs::write(victim.join("mine.txt"), "mine").unwrap();
    let digest = or_source::game_import::tree_digest(&victim).unwrap();
    for named in [
        "../../victim".to_string(),
        victim.to_string_lossy().into_owned(),
    ] {
        let mut record = done.clone();
        record.inverses.clear();
        record.rewritten.clear();
        record.imported[0].source = named.clone();
        record.imported[0].digest = digest.clone();
        let refused = take_back(&project.place(), &record).expect_err(&named);
        assert!(matches!(refused, Refusal::Outside(_)), "{refused}");
        assert!(refused.to_string().contains(&named), "{refused}");
        assert!(victim.join("mine.txt").is_file(), "{named}");
        assert_eq!(
            differing(&standing, &files(&project.folder())),
            Vec::<String>::new()
        );
    }
    // The record as it was made is still taken back.
    take_back(&project.place(), &done).unwrap();
}

#[test]
fn the_census_of_an_install_counts_the_map_files_a_first_edit_refuses_each_with_its_sentence() {
    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    let game = world_fixture::mount(&root);
    let mounted = Mounted::new(&game, &root);

    let census = common::census::census(&game, 1);
    // Every map file the mount loads was asked, the ones that do not read among them.
    assert_eq!(census.maps, mounted.index.maps().len());
    assert_eq!(census.asked, census.maps);
    assert_eq!(census.empty, 1, "`void` holds no entity");
    let refused: Vec<&str> = census
        .refused
        .iter()
        .map(|(file, _)| file.as_str())
        .collect();
    let expected = ["aux", "garbage", "ruin_lod"].map(|name| mounted.logical(name));
    assert_eq!(refused, expected);
    for ((file, why), reason) in census.refused.iter().zip([
        "cannot become a source",
        "does not read as a map file",
        "does not read as a map file",
    ]) {
        assert!(why.contains(file.as_str()) && why.contains(reason), "{why}");
    }
    let text = census.text();
    assert!(
        text.starts_with(&format!(
            "3 of {} map files asked refuse a first edit",
            census.maps
        )),
        "{text}"
    );
    assert_eq!(text.lines().count(), 4, "{text}");

    // Every other one is asked.
    let sampled = common::census::census(&game, 2);
    assert_eq!(sampled.asked, census.maps.div_ceil(2));
}

/// Entities, each as its index in the game's file and its guid.
type Named = Vec<(usize, u32)>;

/// The entities `changes::source_changes` says the source `name` of `project` changed of its base, and those it deleted.
fn changed_and_deleted(project: &Made, name: &str) -> (Named, Named) {
    let dir = project.maps().join(format!("{name}.metadata.src"));
    let SourceChanges::Map(found) = changes::source_changes(&dir).unwrap() else {
        panic!("a map file");
    };
    let changed = found
        .changed
        .iter()
        .map(|kept| (kept.base.index, kept.base.guid));
    let deleted = found
        .deleted
        .iter()
        .map(|entity| (entity.index, entity.guid));
    (changed.collect(), deleted.collect())
}

/// ⚠ `world find` and `world select` read the install and go on printing an entity's place in the game's file, whatever the project
/// has done to its source of that file since.
#[test]
fn a_place_in_the_games_file_names_the_same_object_after_an_earlier_edit_took_one_out_of_the_source(
) {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("install");
    std::fs::create_dir(&root).unwrap();
    let row: Vec<Entity<'_>> = (["bin", "bench", "lamp", "sign", "post"].iter().enumerate())
        .map(|(i, name)| Entity::at(name, [10.0 * (i as f32 + 1.0), 20.0, 1.0], 100 + i as u32))
        .collect();
    world_fixture::write_archive(&root, "common.rpf", &[("data/empty.txt", Vec::new())]);
    let files = [("levels/m_row.ymap", map_file("m_row", &row, BOX))];
    world_fixture::write_archive(&root, "x64a.rpf", &files);
    let game = world_fixture::mount(&root);
    let mounted = Mounted::new(&game, &root);
    let project = Made::new(scratch.path(), Vec::new());
    let logical = mounted.logical("m_row");

    // The bench, entity 1 of the game's file, goes. The game still numbers the lamp 2, and that is what is asked to move.
    apply(&project.place(), &mounted, &deleted(&logical, 1)).unwrap();
    let lamp = moved(&logical, Which::Index(2), [30.0, 25.0, 1.0]);
    let record = apply(&project.place(), &mounted, &lamp).unwrap();
    assert_eq!(
        changed_and_deleted(&project, "m_row"),
        (vec![(2, 102)], vec![(1, 101)]),
        "the lamp moved, and no other object"
    );
    assert_eq!(
        (record.entity.source.as_str(), record.entity.index),
        ("m_row.metadata.src", 1)
    );
    assert!(
        (record.notes.iter()).any(|note| note
            .contains("entity 2 of the game's `m_row.ymap` is entity 1 of `m_row.metadata.src`")),
        "{:?}",
        record.notes
    );

    // The same place given with the source's own name is a place in the source: entity 2 there is the sign.
    let sign = moved("m_row.metadata.src", Which::Index(2), [40.0, 25.0, 1.0]);
    apply(&project.place(), &mounted, &sign).unwrap();
    assert_eq!(
        changed_and_deleted(&project, "m_row").0,
        [(2, 102), (3, 103)]
    );

    // An entity the source no longer holds is refused by name, and one past the end of the game's file by the game's count.
    let gone = plan(&project.place(), &mounted, &deleted(&logical, 1));
    assert!(
        matches!(&gone, Err(Refusal::GoneHere { index: 1, source_there, .. }) if source_there == "m_row.metadata.src"),
        "{:?}",
        gone.map(|planned| planned.record.lines())
    );
    let past = plan(&project.place(), &mounted, &deleted(&logical, 5));
    assert!(
        matches!(
            past,
            Err(Refusal::NoSuchEntity {
                index: 5,
                len: 5,
                ..
            })
        ),
        "the end is the game's file's, which holds five"
    );
}

/// ⚠ Half the map names of a retail install are held by more than one archive, and a path can name a copy a later one replaces.
#[test]
fn a_copy_of_a_map_that_a_later_archive_replaces_is_refused_with_the_copy_the_install_loads() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("install");
    std::fs::create_dir(&root).unwrap();
    let bench = Entity::at("bench", [10.0, 20.0, 1.0], 11);
    let lamp = Entity::at("lamp", [30.0, 20.0, 1.0], 12);
    let first = map_file("m_town", std::slice::from_ref(&bench), BOX);
    let later = map_file("m_town", &[lamp, bench], BOX);
    world_fixture::write_archive(&root, "common.rpf", &[("data/empty.txt", Vec::new())]);
    world_fixture::write_archive(&root, "x64a.rpf", &[("levels/m_town.ymap", first)]);
    world_fixture::write_archive(&root, "x64b.rpf", &[("later/m_town.ymap", later)]);
    let game = world_fixture::mount(&root);
    let mounted = Mounted::new(&game, &root);
    let project = Made::new(scratch.path(), Vec::new());
    let before = files(&project.folder());

    let loaded = mounted.logical("m_town");
    let replaced = "platform:/levels/m_town.ymap";
    assert_eq!(loaded, "platform:/later/m_town.ymap");
    assert!(
        game.vfs.open(replaced).is_some(),
        "the replaced copy still opens at its path"
    );
    let edit = moved(replaced, Which::Index(0), [11.0, 21.0, 2.0]);
    for refused in [
        plan(&project.place(), &mounted, &edit).map(drop),
        apply(&project.place(), &mounted, &edit).map(drop),
    ] {
        assert!(
            matches!(&refused, Err(Refusal::NotTheCopyLoaded { file, loaded: named }) if file == replaced && *named == loaded),
            "{refused:?}"
        );
    }
    assert_eq!(
        differing(&before, &files(&project.folder())),
        Vec::<String>::new()
    );

    // The copy the install loads comes in, with every entity it holds.
    apply(
        &project.place(),
        &mounted,
        &moved(&loaded, Which::Index(0), [31.0, 21.0, 2.0]),
    )
    .unwrap();
    assert_eq!(project.sources(), ["m_town.metadata.src"]);
    assert_eq!(changed_and_deleted(&project, "m_town").0, [(0, 12)]);
}

/// ⚠ A rotation the game stores is not always at length one, and a move states the rotation the entity has.
#[test]
fn a_move_alone_leaves_the_rotation_of_an_object_as_the_game_stores_it() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("install");
    std::fs::create_dir(&root).unwrap();
    // Thirty objects, each turned a little more about the vertical axis than the last.
    let turned: Vec<Entity<'_>> = (0..30u32)
        .map(|i| Entity {
            yaw: 0.05 + i as f32 * 0.1,
            ..Entity::at("bench", [10.0 + i as f32, 20.0, 1.0], 100 + i)
        })
        .collect();
    world_fixture::write_archive(&root, "common.rpf", &[("data/empty.txt", Vec::new())]);
    let files = [("levels/m_turn.ymap", map_file("m_turn", &turned, BOX))];
    world_fixture::write_archive(&root, "x64a.rpf", &files);
    let game = world_fixture::mount(&root);
    let mounted = Mounted::new(&game, &root);
    let project = Made::new(scratch.path(), Vec::new());
    let logical = mounted.logical("m_turn");
    // The check is of rotations a reading at length one would change, so the map holds some.
    use or_io::decl::File as _;
    let shipped = or_io::meta::typed::CMapData::from_bytes(&game.vfs.open(&logical).unwrap());
    let off_length = (shipped.unwrap().entities.iter())
        .filter(|entity| {
            let r = &entity.rotation;
            let stored = glam::Quat::from_xyzw(r.x, r.y, r.z, r.w);
            stored.normalize().to_array().map(f32::to_bits) != stored.to_array().map(f32::to_bits)
        })
        .count();
    assert!(
        off_length > 0,
        "every rotation of the fixture is at length one to the bit"
    );

    for i in 0..30usize {
        let to = [10.0 + i as f32, 25.0, 1.0];
        apply(
            &project.place(),
            &mounted,
            &moved(&logical, Which::Index(i), to),
        )
        .unwrap();
    }
    let dir = project.maps().join("m_turn.metadata.src");
    let SourceChanges::Map(found) = changes::source_changes(&dir).unwrap() else {
        panic!("a map file");
    };
    assert_eq!(found.changed.len(), 30);
    for kept in &found.changed {
        assert!(
            matches!(kept.changes.as_slice(), [EntityChange::Moved { .. }]),
            "entity {} moved and nothing else: {:?}",
            kept.base.index,
            kept.changes
        );
    }
}

/// ⚠ A move writes one entity of one map file, and a deletion writes map files alone: what is left where it was is said.
#[test]
fn a_first_edit_says_what_it_leaves_where_it_was_the_far_model_on_a_move_and_the_collision_on_both()
{
    let scratch = tempfile::tempdir().unwrap();
    let root = install(scratch.path());
    let game = world_fixture::mount(&root);
    let mounted = Mounted::new(&game, &root);
    let project = Made::new(scratch.path(), Vec::new());
    let says = |notes: &[String], what: &str| notes.iter().any(|note| note.contains(what));
    let collision = "the static collision of the area is not changed";

    // The shop of `block` is the one child of the far model `lod_shop` of `block_lod`, which a move of the shop leaves.
    let shop = moved(
        &mounted.logical("block"),
        Which::Index(SHOP),
        [40.0, 40.0, 3.0],
    );
    let record = plan(&project.place(), &mounted, &shop).unwrap().record;
    let far = format!(
        "the far model of the object, entity {LOD_SHOP} of `block_lod.ymap`, stays where it was"
    );
    assert!(says(&record.notes, &far), "{:?}", record.notes);
    assert!(says(&record.notes, collision), "{:?}", record.notes);
    // A far model that is moved leaves the models it stands for.
    let lod = moved(
        &mounted.logical("block_lod"),
        Which::Index(0),
        [21.0, 10.0, 0.0],
    );
    let record = plan(&project.place(), &mounted, &lod).unwrap().record;
    assert!(
        says(
            &record.notes,
            "stands for 3 nearer model(s), which stay where they were"
        ),
        "{:?}",
        record.notes
    );
    // An object with no far model is said to leave none, and a deletion says the collision stays too.
    let bench = moved(
        &mounted.logical("m_town"),
        Which::Index(BENCH),
        [11.0, 21.0, 2.0],
    );
    let record = plan(&project.place(), &mounted, &bench).unwrap().record;
    assert!(!says(&record.notes, "far model"), "{:?}", record.notes);
    let record = plan(
        &project.place(),
        &mounted,
        &deleted(&mounted.logical("shed"), 0),
    )
    .unwrap()
    .record;
    assert!(says(&record.notes, collision), "{:?}", record.notes);
}

/// Map files by their logical path, answered as the world index answers: a map by the hash of its file's name, and the maps under it by the parent each states.
struct Listed(BTreeMap<String, Vec<u8>>);

impl first_edit::Game for Listed {
    fn edition(&self) -> Target {
        Target::Legacy
    }

    fn read(&self, logical: &str) -> Option<Vec<u8>> {
        self.0.get(logical).cloned()
    }

    fn lies_at(&self, logical: &str) -> Option<String> {
        self.0.contains_key(logical).then(|| logical.to_string())
    }

    fn map_file(&self, name: u32) -> Option<String> {
        let called = |logical: &&String| {
            or_io::string_hash(logical.trim_end_matches(".ymap").as_bytes()) == name
        };
        self.0.keys().find(called).cloned()
    }

    fn maps_under(&self, parent: u32) -> Vec<String> {
        use or_io::decl::File;
        let under = |bytes: &Vec<u8>| {
            or_io::meta::typed::CMapData::from_bytes(bytes)
                .is_ok_and(|map| map.parent.hash == parent)
        };
        let listed = self.0.iter().filter(|(_, bytes)| under(bytes));
        listed.map(|(logical, _)| logical.clone()).collect()
    }
}

#[test]
fn a_deletion_in_a_map_file_that_states_another_maps_name_takes_the_maps_under_the_file() {
    // `f_lod.ymap` states the name `x_lod`: the houses of `f` are under it, and the shops of `x` under the file `x_lod.ymap`.
    let far =
        |archetype: &'static str, x: f32, guid: u32| counting(archetype, [x, 10.0, 0.0], guid, 1);
    let f_lod = [
        counting("f_solo", [1.0, 1.0, 0.0], 10, 0),
        far("f_far_a", 10.0, 11),
        far("f_far_b", 20.0, 12),
    ];
    let x_lod = [
        counting("x_solo", [51.0, 51.0, 0.0], 30, 0),
        far("x_far_a", 60.0, 31),
        far("x_far_b", 70.0, 32),
    ];
    let houses = [
        under("house_a", [10.0, 10.0, 0.0], 21, 1),
        under("house_b", [20.0, 10.0, 0.0], 22, 2),
    ];
    let shops = [
        under("shop_a", [60.0, 10.0, 0.0], 41, 1),
        under("shop_b", [70.0, 10.0, 0.0], 42, 2),
    ];
    let game = Listed(BTreeMap::from([
        ("f_lod.ymap".to_string(), map_file("x_lod", &f_lod, BOX)),
        (
            "f.ymap".to_string(),
            child_map_file("f", "f_lod", &houses, BOX),
        ),
        ("x_lod.ymap".to_string(), map_file("x_lod", &x_lod, BOX)),
        (
            "x.ymap".to_string(),
            child_map_file("x", "x_lod", &shops, BOX),
        ),
    ]));
    let scratch = tempfile::tempdir().unwrap();
    let project = Made::new(scratch.path(), Vec::new());

    // The far model that stands for nothing goes: every entity of `f_lod.ymap` after it moves one place up.
    let record = apply(&project.place(), &game, &deleted("f_lod.ymap", 0)).unwrap();
    let mut imported: Vec<&str> = record
        .imported
        .iter()
        .map(|made| made.map.as_str())
        .collect();
    imported.sort_unstable();
    assert_eq!(
        imported,
        ["f.ymap", "f_lod.ymap"],
        "the map under the file, and no map under the name it states"
    );
    let links = |source: &str| -> Vec<i32> {
        (0..)
            .map_while(|index| or_maps::edit::read_entity(&project.maps().join(source), index).ok())
            .map(|entity| entity.parent_index)
            .collect()
    };
    assert_eq!(links("f.metadata.src"), [0, 1]);
}
