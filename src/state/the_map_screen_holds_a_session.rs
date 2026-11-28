//! The Map screen holds the pack it edits open in a session: an edit, Undo and Redo change what the session holds and write no file,
//! saving the project writes what the bare operations would have written, a deletion takes the chain of far models as one step, a
//! refusal changes nothing, a new edit drops what Undo took back, a project closed without saving writes nothing, and a source
//! changed on disk under the session is read again or asked about.
//!
//! ⚠ Built with `AppState::from_prefs` and a project in a scratch folder, with map files made of the crate's samples: no window,
//! no install, and a worker for each import and scan that a case waits on.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use glam::Vec3;
use or_io::decl::File;
use or_io::meta::typed::CMapData;
use or_maps::facts::Inside;
use or_maps::operation::{apply, At, Operation, PoseData};
use or_maps::sample::{
    child_map_file, map_file, map_file_with_container_lods, types_file, Archetype, Entity,
};
use or_maps::{delete, Name};
use or_world::transform::Pose;

use super::front_door::{NewProject, Template};
use super::the_wardrobe_workspace::{bare, until, write};
use super::workbench::{Pick, PickKind};
use super::{AppState, CloseAnswer, ToastType};
use crate::ui::dock::Workspace;

const SLOD: &str = "city_slod.metadata.src";
const LOD: &str = "city_lod.metadata.src";
const CITY: &str = "city.metadata.src";
pub(super) const HARBOUR: &str = "harbour.metadata.src";
const TYPES: &str = "harbour_types.metadata.src";

const EXTENTS: ([f32; 3], [f32; 3]) = ([0.0; 3], [100.0; 3]);

/// A map project in `dir` that holds `files`, opened on the Map screen once the scan has read each as a source.
fn a_pack(dir: &Path, files: &[(&str, Vec<u8>)]) -> (AppState, PathBuf) {
    a_pack_within(dir, files, Duration::from_secs(30))
}

/// [`a_pack`], with `long` for the import and the first scan to end: a large map takes more than a test of the default tier has.
fn a_pack_within(dir: &Path, files: &[(&str, Vec<u8>)], long: Duration) -> (AppState, PathBuf) {
    let mut draft = NewProject::new(Template::Maps, dir.to_path_buf());
    draft.set_name("city".into());
    let project = draft.create().expect("the template makes a project").file;
    let mut state = bare();
    state.load_project_from(project.clone());
    let dropped = dir.join("dropped");
    let paths = (files.iter())
        .map(|(name, bytes)| write(&dropped.join(name), bytes))
        .collect();
    state.add_maps(paths);
    state.active_workspace = Workspace::Map;
    let sources: Vec<String> = (files.iter())
        .map(|(name, _)| format!("{}.metadata.src", name.split('.').next().unwrap()))
        .collect();
    let read = |state: &AppState| {
        let read = state.maps_pack().and_then(|pack| pack.read.as_ref().ok());
        read.is_some_and(|read| {
            sources.iter().all(|source| {
                (read.facts.source(source)).is_some_and(|fact| matches!(fact.inside, Some(Ok(_))))
            })
        })
    };
    let deadline = Instant::now() + long;
    while !read(&state) {
        assert!(Instant::now() < deadline, "the maps were never read");
        state.poll_workbench();
        std::thread::sleep(Duration::from_millis(5));
    }
    (state, project)
}

/// Two crates, one turned, and the type file that declares them.
pub(super) fn harbour(dir: &Path) -> (AppState, PathBuf) {
    let map = map_file(
        "harbour",
        &[
            Entity::at("prop_crate_a", [10.0, 20.0, 0.5], 1),
            Entity {
                yaw: 0.6,
                ..Entity::at("prop_crate_b", [16.0, 20.0, 0.5], 2)
            },
        ],
        ([8.0, 18.0, -1.0], [18.0, 22.0, 2.0]),
    );
    let types = types_file(
        "harbour_types",
        &[
            Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3]),
            Archetype::new("prop_crate_b", [-1.0; 3], [1.0; 3]),
        ],
    );
    a_pack(dir, &[("harbour.ymap", map), ("harbour_types.ytyp", types)])
}

/// An entity that names the entity `parent` of its parent map as its far model.
fn under<'a>(archetype: &'a str, position: [f32; 3], guid: u32, parent: i32) -> Entity<'a> {
    Entity {
        parent,
        in_parent_map: true,
        ..Entity::at(archetype, position, guid)
    }
}

/// Three map files, each the parent map of the next: `city` holds a tower, two houses and a shed; `city_lod` the far model of the
/// tower, one for the two houses and one for the shed; `city_slod` a block that stands for the first two of those, and a far model
/// of the shed's far model.
fn city(dir: &Path) -> (AppState, PathBuf) {
    let slod = map_file(
        "city_slod",
        &[
            Entity {
                children: 2,
                ..Entity::at("slod_block", [10.0, 10.0, 0.0], 101)
            },
            Entity {
                children: 1,
                ..Entity::at("slod_far", [90.0, 10.0, 0.0], 102)
            },
        ],
        EXTENTS,
    );
    let lod = child_map_file(
        "city_lod",
        "city_slod",
        &[
            Entity {
                children: 1,
                ..under("lod_tower", [10.0, 10.0, 0.0], 201, 0)
            },
            Entity {
                children: 2,
                ..under("lod_row", [20.0, 10.0, 0.0], 202, 0)
            },
            Entity {
                children: 1,
                ..under("lod_shed", [90.0, 10.0, 0.0], 203, 1)
            },
        ],
        EXTENTS,
    );
    let near = child_map_file(
        "city",
        "city_lod",
        &[
            under("tower", [10.0, 12.0, 3.0], 301, 0),
            under("house", [18.0, 10.0, 0.0], 302, 1),
            under("house", [22.0, 10.0, 0.0], 303, 1),
            under("shed", [90.0, 10.0, 0.0], 304, 2),
        ],
        EXTENTS,
    );
    a_pack(
        dir,
        &[
            ("city_slod.ymap", slod),
            ("city_lod.ymap", lod),
            ("city.ymap", near),
        ],
    )
}

pub(super) fn folder(state: &AppState) -> PathBuf {
    state.maps_pack().expect("a pack").folder.clone()
}

/// Every file of the sources of the pack in `folder`, by its path inside it: the pack's own file, which remembers the names a person types, is not one.
pub(super) fn sources(folder: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, into: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, into);
            } else if path.parent() != Some(root) {
                let inside = path.strip_prefix(root).unwrap().to_path_buf();
                into.insert(inside, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut found = BTreeMap::new();
    walk(folder, folder, &mut found);
    found
}

/// The sources of `one` that `other` does not hold with the same bytes, and the ones only `other` holds.
pub(super) fn differing(
    one: &BTreeMap<PathBuf, Vec<u8>>,
    other: &BTreeMap<PathBuf, Vec<u8>>,
) -> Vec<PathBuf> {
    (one.iter())
        .filter(|(path, bytes)| other.get(*path) != Some(bytes))
        .map(|(path, _)| path.clone())
        .chain(other.keys().filter(|p| !one.contains_key(*p)).cloned())
        .collect()
}

/// A copy of the pack in `folder`, file for file, beside it.
fn a_copy(folder: &Path, dir: &Path) -> PathBuf {
    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let path = entry.unwrap().path();
            let into = to.join(path.file_name().unwrap());
            if path.is_dir() {
                copy(&path, &into);
            } else {
                std::fs::copy(&path, &into).unwrap();
            }
        }
    }
    let twin = dir.join("twin");
    copy(folder, &twin);
    twin
}

fn select(state: &mut AppState, source: &str, kind: PickKind) {
    let (module, set) = {
        let (pack, set) = state.shown_set().expect("a set");
        (pack.module.clone(), set.id.clone())
    };
    let pick = Pick {
        source: source.to_string(),
        kind,
    };
    state.select_in_maps(&module, &set, Some(pick));
}

/// Move the entity `index` of `source` to `x`, as the manipulator does when it is let go.
pub(super) fn move_to(state: &mut AppState, source: &str, index: usize, x: f32) {
    select(state, source, PickKind::Entity(index));
    let pose = state.selected_pose().expect("the entity selected stands");
    let moved = Pose {
        position: Vec3::new(x, pose.position.y, pose.position.z),
        ..pose
    };
    state.place_selected_entity(moved, "Moved".to_string());
}

/// Where the entities of `source` stand along X, as the session of the pack holds them.
fn held_xs(state: &mut AppState, source: &str) -> Vec<f32> {
    let map = state.held_map_file(source).expect("the session holds it");
    map.entities.iter().map(|e| e.position.x).collect()
}

/// The same, as the screen shows them: what the last scan read, with what the session holds laid over it.
fn shown_xs(state: &AppState, source: &str) -> Vec<f32> {
    let read = state.maps_pack().and_then(|pack| pack.read.as_ref().ok());
    let map = read.and_then(|read| read.facts.source(source)?.map());
    (map.expect("a map file").entities.iter())
        .map(|e| e.position[0])
        .collect()
}

/// What the screen shows of every source: what the files state, or why one does not read.
fn shown(state: &AppState) -> BTreeMap<String, Option<Result<Inside, String>>> {
    let (pack, set) = state.shown_set().expect("a set");
    let read = pack.read.as_ref().expect("the pack reads");
    (set.sources.iter())
        .map(|s| {
            (
                s.clone(),
                read.facts.source(s).and_then(|f| f.inside.clone()),
            )
        })
        .collect()
}

pub(super) fn said(state: &AppState, kind: ToastType) -> Vec<String> {
    (state.toasts.iter())
        .filter(|toast| toast.toast_type == kind)
        .map(|toast| toast.message.clone())
        .collect()
}

pub(super) fn save(state: &mut AppState) {
    assert!(
        state.save_active_project(|_| None),
        "{:?}",
        said(state, ToastType::Error)
    );
}

#[test]
fn an_edit_is_held_until_the_project_is_saved_and_the_save_writes_what_the_bare_operations_leave() {
    const STEPS: usize = 7;
    let _english = crate::i18n::pin("en");
    let dir = tempfile::tempdir().unwrap();
    let (mut state, _) = harbour(dir.path());
    let pack = folder(&state);
    let before = sources(&pack);
    let twin = a_copy(&pack, dir.path());
    assert!(!state.map_edits_unwritten());
    assert!(!state.window_title().contains('●'));

    // A move, an entity added past the extents, a rename, an archetype added and changed, an entity taken out, and an entity
    // turned, scaled and moved past the extents again.
    move_to(&mut state, HARBOUR, 0, 12.5);
    let pose = state.selected_pose().expect("a pose");
    let far = Pose {
        position: Vec3::new(300.0, 20.0, 0.5),
        ..pose
    };
    state.add_entity_at(Name::from_text("prop_crate_a"), far);
    select(&mut state, HARBOUR, PickKind::Entity(1));
    state.rename_selected_entity_archetype("prop_crate_a");
    select(&mut state, TYPES, PickKind::Archetype(1));
    state.add_archetype_named("prop_new_crate", None);
    state.edit_selected_archetype("Set the box".to_string(), &[], |archetype| {
        archetype.bb_max.z = 9.5;
    });
    select(&mut state, HARBOUR, PickKind::Entity(1));
    state.remove_selected_entity();
    // ⚠ Once the scan has read the names the edits made the pack remember: the last move is then said in place, in what the
    // screen keeps of the file, and what is shown below is compared with what the files state once they are saved.
    until(&mut state, "the scan of the names", |s| {
        s.workbench.scan_is_current()
    });
    select(&mut state, HARBOUR, PickKind::Entity(0));
    let turned = Pose {
        position: Vec3::new(150.0, -60.0, 0.5),
        rotation: glam::Quat::from_rotation_z(0.4),
        scale_xy: 1.5,
        scale_z: 2.0,
    };
    state.place_selected_entity(turned, "Turned".to_string());
    assert_eq!(said(&state, ToastType::Error), Vec::<String>::new());
    assert_eq!(state.workbench.maps.undo.len(), STEPS);

    // The session holds them and the screen shows them, and no source was written.
    assert_eq!(held_xs(&mut state, HARBOUR), [150.0, 300.0]);
    assert_eq!(shown_xs(&state, HARBOUR), [150.0, 300.0]);
    assert_eq!(state.shown_scene().expect("a scene").placed.len(), 2);
    assert_eq!(
        differing(&before, &sources(&pack)),
        Vec::<PathBuf>::new(),
        "an edit writes no source"
    );
    assert!(state.map_edits_unwritten());
    assert!(
        state.window_title().contains("city ●"),
        "{}",
        state.window_title()
    );
    let held = shown(&state);

    // The same operations, one by one, each written as it is done.
    let done: Vec<Operation> = (state.workbench.maps.undo.iter())
        .flat_map(|step| step.redo.clone())
        .collect();
    assert_eq!(done.len(), STEPS);
    for operation in &done {
        apply(&twin, operation).unwrap_or_else(|e| panic!("{} one by one: {e}", operation.name()));
    }

    // Save, as Ctrl+S runs it, has the edits to write though the project itself has no change.
    assert!(!state.project_carried.changed);
    let ctx = egui::Context::default();
    crate::ui::commands::run_action(&mut state, &ctx, crate::ui::commands::Action::Save);
    assert_eq!(said(&state, ToastType::Error), Vec::<String>::new());
    assert!(!state.map_edits_unwritten());
    assert!(
        !state.window_title().contains('●'),
        "{}",
        state.window_title()
    );
    let saved = sources(&pack);
    assert_ne!(differing(&before, &saved), Vec::<PathBuf>::new());
    assert_eq!(
        differing(&sources(&twin), &saved),
        Vec::<PathBuf>::new(),
        "the save writes the XML the bare operations write"
    );
    for source in [HARBOUR, TYPES] {
        assert!(
            or_maps::facts::cooked(&pack.join(source))
                == or_maps::facts::cooked(&twin.join(source)),
            "{source} cooks to the file the bare operations leave"
        );
    }

    // The scan that reads the save shows what the session showed before it.
    until(&mut state, "the scan of the save", |s| {
        s.workbench.scan_is_current()
    });
    assert!(
        shown(&state) == held,
        "what was shown before the save is what the files state after it"
    );
    assert_eq!(
        state.workbench.maps.undo.len(),
        STEPS,
        "a save keeps the steps"
    );

    // Undo still goes back after the save, in the session, and the next save writes that.
    state.undo_maps_change();
    assert_eq!(shown_xs(&state, HARBOUR), [12.5, 300.0]);
    for _ in 1..STEPS {
        state.undo_maps_change();
    }
    assert_eq!(said(&state, ToastType::Error), Vec::<String>::new());
    assert_eq!(held_xs(&mut state, HARBOUR), [10.0, 16.0]);
    assert_eq!(differing(&saved, &sources(&pack)), Vec::<PathBuf>::new());
    let held = shown(&state);
    save(&mut state);
    assert_eq!(
        differing(&before, &sources(&pack)),
        Vec::<PathBuf>::new(),
        "every step back and saved, the sources hold the bytes they held"
    );
    until(&mut state, "the scan of the second save", |s| {
        s.workbench.scan_is_current()
    });
    assert!(
        shown(&state) == held,
        "and the screen showed them so before that save"
    );
}

#[test]
fn a_deletion_takes_the_far_models_as_one_step_and_one_undo_puts_the_chain_back() {
    let _english = crate::i18n::pin("en");
    let dir = tempfile::tempdir().unwrap();
    let (mut state, _) = city(dir.path());
    let pack = folder(&state);
    let before = sources(&pack);
    let twin = a_copy(&pack, dir.path());
    let counts = |state: &mut AppState| {
        [CITY, LOD, SLOD].map(|source| state.held_map_file(source).unwrap().entities.len())
    };
    let as_read: Vec<Vec<u8>> = [CITY, LOD, SLOD]
        .map(|source| state.held_map_file(source).unwrap().to_bytes().unwrap())
        .to_vec();
    assert_eq!(counts(&mut state), [4, 3, 2]);
    assert_eq!(state.shown_scene().expect("a scene").placed.len(), 9);

    // The shed goes with its far model and the far model of that one.
    select(&mut state, CITY, PickKind::Entity(3));
    state.remove_selected_entity();
    assert_eq!(said(&state, ToastType::Error), Vec::<String>::new());
    assert_eq!(
        said(&state, ToastType::Warning),
        ["2 far models that stood for nothing else went with it"]
    );
    assert_eq!(state.workbench.maps.undo.len(), 1, "the chain is one step");
    let step = state.workbench.maps.undo[0].clone();
    let written: Vec<&str> = step.redo.iter().map(Operation::source).collect();
    assert_eq!(written, [CITY, LOD, SLOD], "one step across three files");
    assert_eq!(counts(&mut state), [3, 2, 1]);
    assert_eq!(state.shown_scene().expect("a scene").placed.len(), 6);
    assert_eq!(state.workbench.maps.selected, None);
    assert_eq!(differing(&before, &sources(&pack)), Vec::<PathBuf>::new());

    // One Undo puts the three back, and nothing is left to write.
    state.undo_maps_change();
    assert_eq!(counts(&mut state), [4, 3, 2]);
    let back: Vec<Vec<u8>> = [CITY, LOD, SLOD]
        .map(|source| state.held_map_file(source).unwrap().to_bytes().unwrap())
        .to_vec();
    assert!(back == as_read, "each file is the file it was");
    assert_eq!(state.shown_scene().expect("a scene").placed.len(), 9);
    assert!(state.workbench.maps.undo.is_empty());
    assert_eq!(state.workbench.maps.redo.len(), 1);
    assert!(!state.map_edits_unwritten());

    // Made again and saved, the files are the ones the plan leaves when it is applied to the sources.
    state.redo_maps_change();
    assert_eq!(counts(&mut state), [3, 2, 1]);
    save(&mut state);
    let shed = [At {
        source: CITY.to_string(),
        index: 3,
    }];
    let planned = delete::plan(&twin, &shed).unwrap();
    assert_eq!(planned.operations, step.redo);
    delete::apply(&twin, &planned).unwrap();
    assert_eq!(
        differing(&sources(&twin), &sources(&pack)),
        Vec::<PathBuf>::new()
    );
    let codes: Vec<&str> = {
        let model = or_maps::Maps::read(&pack).unwrap();
        let facts = or_maps::inspect(&pack, &model, or_maps::Depth::Open);
        (or_maps::check(&model, &facts, None).iter())
            .map(|finding| finding.code)
            .collect()
    };
    for rule in ["parent-link-broken", "lod-children-uncounted"] {
        assert!(!codes.contains(&rule), "{rule}: {codes:?}");
    }
    state.undo_maps_change();
    save(&mut state);
    assert_eq!(differing(&before, &sources(&pack)), Vec::<PathBuf>::new());

    // A far model taken out takes what it stood for, and the block above it stays with one child fewer.
    state.clear_toasts();
    select(&mut state, LOD, PickKind::Entity(1));
    state.remove_selected_entity();
    assert_eq!(
        said(&state, ToastType::Warning),
        ["2 entities it was the far model of went with it"]
    );
    assert_eq!(counts(&mut state), [2, 2, 2]);
    assert_eq!(
        state.held_map_file(SLOD).unwrap().entities[0].num_children,
        1
    );
    // ⚠ `city` takes two operations of this step, its houses out and the link of the shed renumbered: a step keeps the operations
    // of one file in the order each applies in, which three files of one operation each do not show.
    let step = state.workbench.maps.undo.last().expect("a step").clone();
    let of_city =
        |operations: &[Operation]| operations.iter().filter(|o| o.source() == CITY).count();
    assert_eq!((of_city(&step.redo), of_city(&step.undo)), (2, 2));
    state.undo_maps_change();
    assert_eq!(counts(&mut state), [4, 3, 2]);
    assert_eq!(
        state.held_map_file(SLOD).unwrap().entities[0].num_children,
        2
    );
    state.redo_maps_change();
    assert_eq!(counts(&mut state), [2, 2, 2]);
    assert_eq!(
        state.held_map_file(CITY).unwrap().entities[1].parent_index,
        1
    );
    state.undo_maps_change();
    assert_eq!(said(&state, ToastType::Error), Vec::<String>::new());
    assert_eq!(counts(&mut state), [4, 3, 2]);
    let back: Vec<Vec<u8>> = [CITY, LOD, SLOD]
        .map(|source| state.held_map_file(source).unwrap().to_bytes().unwrap())
        .to_vec();
    assert!(
        back == as_read,
        "made again and taken back, each file is the file it was"
    );

    // The tower goes with the one far model that stood for it alone.
    state.clear_toasts();
    select(&mut state, CITY, PickKind::Entity(0));
    state.remove_selected_entity();
    assert_eq!(
        said(&state, ToastType::Warning),
        ["1 far model that stood for nothing else went with it"]
    );
    assert_eq!(counts(&mut state), [3, 2, 2]);

    // The shed is entity 2 of `city` now, in the session and not on the disk, where that index is a house: the deletion is planned
    // over the files as the session holds them.
    state.clear_toasts();
    select(&mut state, CITY, PickKind::Entity(2));
    state.remove_selected_entity();
    assert_eq!(said(&state, ToastType::Error), Vec::<String>::new());
    assert_eq!(
        said(&state, ToastType::Warning),
        ["2 far models that stood for nothing else went with it"]
    );
    assert_eq!(counts(&mut state), [2, 1, 1]);
    let houses: Vec<u32> = (state.held_map_file(CITY).unwrap().entities.iter())
        .map(|entity| entity.guid)
        .collect();
    assert_eq!(houses, [302, 303]);
}

#[test]
fn a_refused_operation_changes_nothing_and_says_why() {
    let _english = crate::i18n::pin("en");
    let dir = tempfile::tempdir().unwrap();
    let stands = Entity::at("prop_crate_a", [10.0, 20.0, 0.5], 1);
    let twin = Entity::at("prop_crate_a", [40.0, 20.0, 0.5], 3);
    let carrying = map_file_with_container_lods(
        "yard",
        &[Entity::at("prop_crate_b", [60.0, 20.0, 0.5], 9)],
        &[0],
        EXTENTS,
    );
    let harbour = map_file(
        "harbour",
        &[stands, twin, Entity { guid: 4, ..twin }],
        EXTENTS,
    );
    let types = types_file(
        "harbour_types",
        &[
            Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3]),
            Archetype::new("prop_crate_b", [-1.0; 3], [1.0; 3]),
        ],
    );
    let (mut state, _) = a_pack(
        dir.path(),
        &[
            ("harbour.ymap", harbour),
            ("yard.ymap", carrying),
            ("harbour_types.ytyp", types),
        ],
    );
    let pack = folder(&state);
    let before = sources(&pack);

    // One edit that goes through, so that the session holds something a refusal could lose.
    move_to(&mut state, HARBOUR, 0, 12.5);
    let held = shown(&state);
    let as_it_was = |state: &mut AppState, what: &str| {
        assert_eq!(state.workbench.maps.undo.len(), 1, "{what}");
        assert!(state.workbench.maps.redo.is_empty(), "{what}");
        assert!(
            shown(state) == held,
            "{what}: the screen shows what it showed"
        );
        assert_eq!(held_xs(state, HARBOUR), [12.5, 40.0, 40.0], "{what}");
        assert!(
            state.map_edits_unwritten(),
            "{what}: the edit before it is still to write"
        );
        assert_eq!(
            differing(&before, &sources(&pack)),
            Vec::<PathBuf>::new(),
            "{what}"
        );
        state.clear_toasts();
    };

    // An archetype given the name another holds.
    select(&mut state, TYPES, PickKind::Archetype(1));
    state.edit_selected_archetype("Renamed".to_string(), &[], |archetype| {
        archetype.name = or_io::meta::instance::HashedString::new("prop_crate_a");
    });
    let refused = said(&state, ToastType::Error);
    assert!(
        matches!(refused.as_slice(), [why] if why.contains("already declares")),
        "{refused:?}"
    );
    as_it_was(&mut state, "a name another archetype holds");

    // An entity of a map that carries container LODs, whose indexes a deletion does not renumber.
    select(&mut state, "yard.metadata.src", PickKind::Entity(0));
    state.remove_selected_entity();
    let refused = said(&state, ToastType::Error);
    assert!(
        matches!(refused.as_slice(), [why] if why.contains("container LODs")),
        "{refused:?}"
    );
    as_it_was(&mut state, "a map that carries container LODs");

    // An entity that another cannot be told from is left, with a sentence that says so.
    select(&mut state, HARBOUR, PickKind::Entity(1));
    state.remove_selected_entity();
    assert_eq!(
        said(&state, ToastType::Warning),
        ["Entity 1 was left: 1 other entity places the same model at the same pose, and a deletion would not know which of them goes"]
    );
    assert_eq!(said(&state, ToastType::Error), Vec::<String>::new());
    as_it_was(&mut state, "an entity with a twin");

    // A pose that is no number.
    select(&mut state, HARBOUR, PickKind::Entity(0));
    let pose = state.selected_pose().expect("a pose");
    let nowhere = Pose {
        position: Vec3::new(f32::NAN, 0.0, 0.0),
        ..pose
    };
    state.place_selected_entity(nowhere, "Moved".to_string());
    let refused = said(&state, ToastType::Error);
    assert!(
        matches!(refused.as_slice(), [why] if why.contains("not a finite number")),
        "{refused:?}"
    );
    as_it_was(&mut state, "a pose that is no number");

    // A field left as it was is no step and nothing to save.
    move_to(&mut state, HARBOUR, 0, 12.5);
    as_it_was(&mut state, "a move to where it stands");

    // What went through before the refusals is what the save writes.
    save(&mut state);
    assert_eq!(
        or_maps::edit::read_entity(&pack.join(HARBOUR), 0)
            .unwrap()
            .position
            .x,
        12.5
    );
}

#[test]
fn an_edit_made_after_an_undo_drops_what_redo_would_have_made_again() {
    let dir = tempfile::tempdir().unwrap();
    let (mut state, _) = harbour(dir.path());
    move_to(&mut state, HARBOUR, 0, 12.0);
    move_to(&mut state, HARBOUR, 0, 14.0);
    state.undo_maps_change();
    assert_eq!(held_xs(&mut state, HARBOUR), [12.0, 16.0]);
    let work = &state.workbench.maps;
    assert_eq!((work.undo.len(), work.redo.len()), (1, 1));

    move_to(&mut state, HARBOUR, 1, 20.0);
    assert_eq!(state.workbench.maps.undo.len(), 2);
    assert!(
        state.workbench.maps.redo.is_empty(),
        "the move to 14 is gone"
    );
    state.redo_maps_change();
    assert_eq!(
        held_xs(&mut state, HARBOUR),
        [12.0, 20.0],
        "Redo has nothing to make again"
    );
    assert_eq!(shown_xs(&state, HARBOUR), [12.0, 20.0]);

    // The two steps that are left go back in their order, to the file as it was read.
    state.undo_maps_change();
    assert_eq!(held_xs(&mut state, HARBOUR), [12.0, 16.0]);
    state.undo_maps_change();
    assert_eq!(held_xs(&mut state, HARBOUR), [10.0, 16.0]);
    assert_eq!(shown_xs(&state, HARBOUR), [10.0, 16.0]);
    assert!(state.workbench.maps.undo.is_empty());
    assert!(!state.map_edits_unwritten());
    state.redo_maps_change();
    state.redo_maps_change();
    assert_eq!(held_xs(&mut state, HARBOUR), [12.0, 20.0]);
}

#[test]
fn a_project_closed_without_saving_writes_no_edit_and_one_closed_with_save_writes_them() {
    let dir = tempfile::tempdir().unwrap();
    let (mut state, project) = harbour(dir.path());
    let pack = folder(&state);
    let before = sources(&pack);

    move_to(&mut state, HARBOUR, 0, 12.5);
    state.close_project();
    assert!(
        state.pending_close.is_some(),
        "an edit of the maps is asked about"
    );
    state.answer_close(CloseAnswer::Cancel);
    assert!(state.project_path.is_some());
    assert_eq!(
        held_xs(&mut state, HARBOUR),
        [12.5, 16.0],
        "a cancelled close keeps the edit"
    );

    state.close_project();
    state.answer_close(CloseAnswer::DontSave);
    assert!(state.project_path.is_none());
    state.poll_workbench();
    assert_eq!(
        differing(&before, &sources(&pack)),
        Vec::<PathBuf>::new(),
        "the session was let go with nothing written"
    );

    // Opened again, the project holds the files, no step and nothing to save.
    state.load_project_from(project.clone());
    state.active_workspace = Workspace::Map;
    until(&mut state, "the maps", |s| {
        s.shown_scene().is_some_and(|scene| scene.placed.len() == 2)
    });
    assert!(state.workbench.maps.undo.is_empty());
    assert!(!state.map_edits_unwritten());
    assert_eq!(held_xs(&mut state, HARBOUR), [10.0, 16.0]);

    move_to(&mut state, HARBOUR, 1, 20.0);
    state.close_project();
    state.answer_close(CloseAnswer::Save);
    state.drive_close(|_, _| None);
    assert!(state.project_path.is_none());
    let saved = or_maps::edit::read_entity(&pack.join(HARBOUR), 1).unwrap();
    assert_eq!(
        saved.position.x, 20.0,
        "Save wrote the edit before the project closed"
    );
}

/// Move the entity 1 of `harbour` on disk to `x`, as something that is not this window does.
fn moved_outside(pack: &Path, x: f32) {
    let operation = Operation::PlaceEntity {
        source: HARBOUR.to_string(),
        expects: None,
        entity: 1,
        pose: PoseData {
            position: [x, 41.0, 0.5],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale_xy: 1.0,
            scale_z: 1.0,
        },
    };
    apply(pack, &operation).unwrap();
}

#[test]
fn a_source_changed_on_disk_under_the_session_is_read_again_or_asked_about_as_a_tab_is() {
    let dir = tempfile::tempdir().unwrap();
    let (mut state, _) = harbour(dir.path());
    let pack = folder(&state);
    let file = pack.join(HARBOUR).join(or_source::metadata::XML);
    let scanned = |state: &mut AppState| {
        state.workbench.want_scan();
        until(state, "the scan", |s| s.workbench.scan_is_current());
    };

    // With nothing unsaved, the source is read again with no question, and the steps that held its old states go.
    move_to(&mut state, HARBOUR, 0, 12.5);
    save(&mut state);
    assert_eq!(state.workbench.maps.undo.len(), 1);
    moved_outside(&pack, 40.0);
    scanned(&mut state);
    assert_eq!(state.reload_conflicts, Vec::<PathBuf>::new());
    assert_eq!(held_xs(&mut state, HARBOUR), [12.5, 40.0]);
    assert_eq!(shown_xs(&state, HARBOUR), [12.5, 40.0]);
    assert!(
        state.workbench.maps.undo.is_empty(),
        "the step of the old contents is gone"
    );

    // With an edit unsaved, a save writes nothing over the file, and the next scan asks.
    move_to(&mut state, HARBOUR, 0, 14.0);
    moved_outside(&pack, 50.0);
    let outside = std::fs::read(&file).unwrap();
    assert!(!state.save_active_project(|_| None));
    let refused = said(&state, ToastType::Error);
    assert!(
        matches!(refused.as_slice(), [why] if why.contains("was changed on disk since the session read it")),
        "{refused:?}"
    );
    assert!(
        std::fs::read(&file).unwrap() == outside,
        "a refused save writes nothing"
    );
    scanned(&mut state);
    assert_eq!(state.reload_conflicts, std::slice::from_ref(&file));
    assert_eq!(
        shown_xs(&state, HARBOUR),
        [14.0, 40.0],
        "the screen keeps showing what it holds"
    );

    // Kept, the edit is written over the file at the next save, and what was done outside is gone.
    state.answer_reload_conflicts(false);
    assert!(state.reload_conflicts.is_empty());
    scanned(&mut state);
    assert!(
        state.reload_conflicts.is_empty(),
        "the question is asked once"
    );
    save(&mut state);
    let saved =
        CMapData::from_bytes(&or_maps::facts::cooked(&pack.join(HARBOUR)).unwrap()).unwrap();
    let xs: Vec<f32> = saved.entities.iter().map(|e| e.position.x).collect();
    assert_eq!(xs, [14.0, 40.0]);
    assert_eq!(state.workbench.maps.undo.len(), 1, "and its step is kept");

    // Read again, the edit is lost with its step, and the session holds what the disk holds.
    move_to(&mut state, HARBOUR, 0, 18.0);
    moved_outside(&pack, 60.0);
    scanned(&mut state);
    assert_eq!(state.reload_conflicts, std::slice::from_ref(&file));
    state.answer_reload_conflicts(true);
    assert!(state.reload_conflicts.is_empty());
    assert!(state.workbench.maps.undo.is_empty());
    assert!(!state.map_edits_unwritten());
    assert_eq!(held_xs(&mut state, HARBOUR), [14.0, 60.0]);
    scanned(&mut state);
    assert_eq!(shown_xs(&state, HARBOUR), [14.0, 60.0]);
    assert!(state.reload_conflicts.is_empty());
}

#[test]
fn a_map_file_added_after_the_first_edit_is_edited_through_the_same_session() {
    let dir = tempfile::tempdir().unwrap();
    let (mut state, _) = harbour(dir.path());
    move_to(&mut state, HARBOUR, 0, 12.5);
    let pier = map_file(
        "pier",
        &[Entity::at("prop_crate_a", [30.0, 20.0, 0.5], 21)],
        EXTENTS,
    );
    state.add_maps(vec![write(
        &dir.path().join("dropped").join("pier.ymap"),
        &pier,
    )]);
    let source = "pier.metadata.src";
    until(&mut state, "the pier", |s| {
        let read = s.maps_pack().and_then(|pack| pack.read.as_ref().ok());
        read.is_some_and(|read| read.facts.source(source).is_some_and(|f| f.map().is_some()))
    });
    assert_eq!(
        shown_xs(&state, HARBOUR),
        [12.5, 16.0],
        "the scan of the import keeps the edit shown"
    );
    move_to(&mut state, source, 0, 33.0);
    assert_eq!(said(&state, ToastType::Error), Vec::<String>::new());
    assert_eq!(held_xs(&mut state, source), [33.0]);
    assert_eq!(state.workbench.maps.undo.len(), 2);
}

/// What a move costs on the Map screen on a large map: the operation in the session, and what the session then holds laid over
/// the scan (the map file saved from its root and read as a scan reads one, the scene and the rules), on the thread of the frame;
/// then the save, which is the flush.
///
/// ⚠ A timing, so the test is ignored and run by name:
/// `cargo nextest run --profile all --bin openrage --run-ignored all --success-output final -E 'test(one_move_on_the_map_screen_is_timed)'`.
#[test]
#[ignore]
fn one_move_on_the_map_screen_is_timed_on_a_synthetic_map_of_twenty_thousand_entities() {
    const ENTITIES: usize = 20_000;
    let dir = tempfile::tempdir().unwrap();
    let placed: Vec<Entity<'_>> = (0..ENTITIES)
        .map(|at| Entity {
            yaw: at as f32 * 0.01,
            ..Entity::at(
                ["prop_crate_a", "prop_crate_b"][at % 2],
                [(at % 200) as f32 * 4.0, (at / 200) as f32 * 4.0, 0.5],
                at as u32 + 1,
            )
        })
        .collect();
    let map = map_file("harbour", &placed, ([0.0; 3], [800.0, 400.0, 10.0]));
    let types = types_file(
        "harbour_types",
        &[
            Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3]),
            Archetype::new("prop_crate_b", [-2.0; 3], [2.0; 3]),
        ],
    );
    let (mut state, _) = a_pack_within(
        dir.path(),
        &[("harbour.ymap", map), ("harbour_types.ytyp", types)],
        Duration::from_secs(600),
    );
    let timed = |state: &mut AppState, x: f32| {
        let started = Instant::now();
        move_to(state, HARBOUR, 7, x);
        started.elapsed()
    };
    let first = timed(&mut state, 3.5);
    let next: Vec<_> = (0..5)
        .map(|step| timed(&mut state, 4.0 + step as f32))
        .collect();
    assert_eq!(state.workbench.maps.undo.len(), 6);
    assert_eq!(shown_xs(&state, HARBOUR)[7], 8.0);
    // The parts of what is laid over the scan, each timed apart on what the session holds.
    let part = |what: &str, started: Instant| eprintln!("  {what}: {:?}", started.elapsed());
    let started = Instant::now();
    let held = state.held_map_file(HARBOUR).unwrap();
    part("the root copied", started);
    let started = Instant::now();
    let bytes = held.to_bytes().unwrap();
    part("the root saved", started);
    let started = Instant::now();
    or_maps::facts::inside_of(&bytes, "ymap", None).unwrap();
    part("the saved file read as a scan reads one", started);
    {
        let (pack, set) = state.shown_set().expect("a set");
        let read = pack.read.as_ref().expect("the pack reads");
        let started = Instant::now();
        or_maps::scene(set, &read.facts);
        part("the scene", started);
        let started = Instant::now();
        or_maps::check(&read.model, &read.facts, None);
        part("the rules", started);
    }
    let started = Instant::now();
    state.flush_map_sessions().unwrap();
    part("the flush", started);
    move_to(&mut state, HARBOUR, 7, 3.0);
    let started = Instant::now();
    save(&mut state);
    let saved = started.elapsed();
    eprintln!(
        "{ENTITIES} entities: the first move, which opens the source, {first:?}; the least of five moves after it {:?}; the save {saved:?}",
        next.iter().min().unwrap()
    );
}
