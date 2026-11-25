//! The Map screen as it is drawn: a map pack opens on it, it shows in Content, Open in and the start page through the table of task screens
//! alone, its empty state offers the one action, its list shows what the files place and declare, and an entity moved with the manipulator
//! (or typed, or nudged from the keyboard) is written to its map file as one step of Undo that puts it back.
//!
//! Read through AccessKit from whole frames of the shell, with the map files this file makes from the crate's samples.

use std::path::{Path, PathBuf};

use glam::{Quat, Vec3};
use or_maps::gizmo::{Axis, Gizmo, Mode, View};
use or_maps::sample::{map_file, types_file, Archetype, Entity};

use super::frames_of_the_shell::{key, Shell};
use super::the_front_door_on_screen::{
    a_first_launch, a_project, click_on, names, shows, waits_for,
};
use super::the_project_screens_on_screen::{centre_of, until};
use crate::state::front_door::cards::read_card;
use crate::state::front_door::Template;
use crate::state::workbench::{Pick, PickKind};
use crate::ui::dock::Workspace;

const TALL: (u32, u32) = (1600, 1200);

/// A map file placing two crates, one turned, and the type file declaring them.
pub(super) fn the_files() -> (Vec<u8>, Vec<u8>) {
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
    (map, types)
}

pub(super) fn write(path: &Path, body: &[u8]) -> PathBuf {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
    path.to_path_buf()
}

/// A shell on a map pack `harbour`, which holds the map file and the type file of `the_files` once the scan has read them.
pub(super) fn a_maps_shell(dir: &Path) -> Shell {
    let file = a_project(dir, "harbour", Template::Maps);
    let mut shell = Shell::new(crate::state::ThemeChoice::Dark);
    shell.state.load_project_from(file);
    let (map, types) = the_files();
    let drop = dir.join("drop");
    shell.state.add_maps(vec![
        write(&drop.join("harbour.ymap"), &map),
        write(&drop.join("harbour_types.ytyp"), &types),
    ]);
    shell.state.active_workspace = Workspace::Map;
    // The names the install gives are known to the index; a map file keeps only their hashes.
    for name in ["prop_crate_a", "prop_crate_b"] {
        shell
            .state
            .name_index
            .add(crate::state::name_index::NameIndexCategory::Model, name);
    }
    shell.settled();
    until(&mut shell, "the maps", |s| {
        s.state
            .shown_scene()
            .is_some_and(|scene| scene.placed.len() == 2)
    });
    shell
}

fn the_map_source(shell: &Shell) -> PathBuf {
    shell
        .state
        .maps_pack()
        .expect("a pack")
        .folder
        .join("harbour.metadata.src")
}

fn the_map_file(shell: &Shell) -> or_io::meta::typed::CMapData {
    use or_io::decl::File;
    or_io::meta::typed::CMapData::from_bytes(
        &or_maps::facts::cooked(&the_map_source(shell)).unwrap(),
    )
    .unwrap()
}

fn select_entity(shell: &mut Shell, index: usize) {
    let (module, set) = {
        let (pack, set) = shell.state.shown_set().expect("a set");
        (pack.module.clone(), set.id.clone())
    };
    shell.state.select_in_maps(
        &module,
        &set,
        Some(Pick {
            source: "harbour.metadata.src".to_string(),
            kind: PickKind::Entity(index),
        }),
    );
    shell.settled_sized(TALL);
}

#[test]
fn a_map_pack_opens_on_the_map_screen_and_shows_in_content_the_strip_and_the_start_page_through_the_table(
) {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let file = a_project(dir.path(), "harbour", Template::Maps);
    let card = read_card(&file);
    assert_eq!(
        card.content,
        ["Maps · 0 map sets"],
        "the card counts in the row's word"
    );

    let mut shell = Shell::new(crate::state::ThemeChoice::Dark);
    shell.state.load_project_from(file);
    shell.settled();
    assert_eq!(shell.state.active_workspace, Workspace::Map);
    assert!(shell.state.workspaces_offered().contains(&Workspace::Map));
    // ⚠ On the overview: Content counts the pack, and the Project panel is not repeated on a content's own screen.
    shell.state.active_workspace = Workspace::Project;
    until(&mut shell, "the pack to be counted", |s| {
        shows(s, "0 map sets")
    });
}

#[test]
fn the_empty_state_of_a_project_without_maps_offers_the_one_action_and_it_gives_the_project_a_pack()
{
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let file = a_project(dir.path(), "gadgets", Template::Files);
    let mut shell = a_first_launch();
    shell.state.load_project_from(file);
    shell.state.active_workspace = Workspace::Map;
    shell.settled();
    assert!(!shell.state.has_maps());
    assert!(
        shows(&shell, "No maps in this project"),
        "{:?}",
        names(&shell)
    );
    click_on(&mut shell, "Add a map to this project");
    assert!(shell.state.has_maps(), "the action gave the project a pack");
    waits_for(&mut shell, "Add map files…");
}

#[test]
fn the_list_shows_what_the_files_place_and_declare_and_a_click_selects_one() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_maps_shell(dir.path());
    for text in ["#0  prop_crate_a", "#1  prop_crate_b"] {
        assert!(shows(&shell, text), "{text}: {:?}", names(&shell));
    }
    let at = centre_of(&shell, "Entity #1  prop_crate_b");
    shell.click(at);
    shell.settled();
    let selected = shell.state.selected_placed().map(|p| (p.index, p.guid));
    assert_eq!(selected, Some((1, 2)));
    assert!(
        shows(&shell, "Entity position X"),
        "the selection offers the number to type: {:?}",
        names(&shell)
    );
}

/// A key pressed, drawn and let go, as a keyboard does it: egui reads a second press of a key that was never let go as the first one repeating.
fn tap(shell: &mut Shell, pressed: egui::Key) {
    pointer(shell, vec![key(pressed)]);
    shell.settled_sized(TALL);
    pointer(
        shell,
        vec![egui::Event::Key {
            key: pressed,
            physical_key: Some(pressed),
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
}

/// The scene's rectangle as it was drawn, and the camera that looks at `target` from the south.
fn look_at(shell: &mut Shell, target: Vec3) -> View {
    let work = &mut shell.state.workbench.maps;
    work.camera.target = target;
    work.camera.yaw = -std::f32::consts::FRAC_PI_2;
    work.camera.pitch = 0.5;
    work.camera.distance = 30.0;
    work.needs_frame = false;
    shell.settled_sized(TALL);
    let bounds = shell
        .described
        .iter()
        .find(|w| w.name.as_deref() == Some("Scene of the map set"))
        .expect("the scene is drawn")
        .bounds;
    View {
        view_proj: shell
            .state
            .workbench
            .maps
            .camera
            .view_proj(bounds.width() / bounds.height()),
        origin: glam::Vec2::new(bounds.min.x, bounds.min.y),
        size: glam::Vec2::new(bounds.width(), bounds.height()),
    }
}

fn pointer(shell: &mut Shell, events: Vec<egui::Event>) {
    shell.draw_sized(TALL, events);
}

fn button(at: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn at(p: glam::Vec2) -> egui::Pos2 {
    egui::pos2(p.x, p.y)
}

#[test]
fn moving_an_entity_with_the_manipulator_writes_the_new_position_and_one_undo_restores_it() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_maps_shell(dir.path());
    shell.state.workbench.maps.tool = Mode::Move;
    shell.state.workbench.maps.snap.grid_on = true;
    shell.state.workbench.maps.snap.grid = 0.5;
    select_entity(&mut shell, 0);
    let before = std::fs::read(the_map_source(&shell).join("metadata.xml")).unwrap();
    let origin = Vec3::new(10.0, 20.0, 0.5);
    let view = look_at(&mut shell, origin);
    let gizmo =
        Gizmo::at(Mode::Move, origin, Quat::IDENTITY, &view).expect("the manipulator is on screen");

    // Take the X arrow a little way from its root, and drag it three metres east.
    let grab = origin + Vec3::X * gizmo.reach * 0.6;
    let from = at(view.project(grab).unwrap());
    let to = at(view.project(grab + Vec3::X * 3.0).unwrap());
    pointer(&mut shell, vec![egui::Event::PointerMoved(from)]);
    pointer(&mut shell, vec![button(from, true)]);
    for step in 1..=6 {
        let t = step as f32 / 6.0;
        let mid = egui::pos2(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
        pointer(&mut shell, vec![egui::Event::PointerMoved(mid)]);
        if step == 3 {
            // Mid-drag the file still holds the old place, and the screen shows the new one.
            assert_eq!(
                the_map_file(&shell).entities[0].position.x,
                10.0,
                "nothing is written until the drag is let go"
            );
            assert!(shell.state.workbench.maps.preview.is_some());
        }
    }
    pointer(&mut shell, vec![button(to, false)]);
    shell.settled_sized(TALL);

    let moved = the_map_file(&shell).entities[0].position;
    assert!(
        (moved.x - 13.0).abs() <= 0.5,
        "three metres east, held to the grid: {moved:?}"
    );
    assert_eq!(moved.x % 0.5, 0.0, "on a line of the grid: {}", moved.x);
    assert_eq!(
        (moved.y, moved.z),
        (20.0, 0.5),
        "the other axes are where they were"
    );
    assert_eq!(
        shell.state.workbench.maps.undo.len(),
        1,
        "a drag is one step, however long"
    );
    assert!(shell.state.workbench.maps.preview.is_none());
    let other = the_map_file(&shell).entities[1].position;
    assert_eq!(other.x, 16.0, "the other entity is where it was");

    shell.state.undo_maps_change();
    assert_eq!(
        the_map_file(&shell).entities[0].position.x,
        10.0,
        "one Undo puts it back"
    );
    assert_eq!(
        std::fs::read(the_map_source(&shell).join("metadata.xml")).unwrap(),
        before,
        "the source is as it was"
    );
    shell.state.redo_maps_change();
    assert!((the_map_file(&shell).entities[0].position.x - 13.0).abs() <= 0.5);
}

#[test]
fn a_ring_turns_and_the_centre_scales_and_each_is_one_step() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_maps_shell(dir.path());
    select_entity(&mut shell, 0);
    let origin = Vec3::new(10.0, 20.0, 0.5);

    // A ring about the vertical axis, taken at the east and drawn a quarter of the way round.
    shell.state.workbench.maps.tool = Mode::Rotate;
    let view = look_at(&mut shell, origin);
    let gizmo = Gizmo::at(Mode::Rotate, origin, Quat::IDENTITY, &view).unwrap();
    let ring = |angle: f32| {
        let p = origin + (Vec3::X * angle.cos() + Vec3::Y * angle.sin()) * gizmo.reach * 0.8;
        at(view.project(p).unwrap())
    };
    let (from, to) = (ring(0.0), ring(std::f32::consts::FRAC_PI_2));
    pointer(&mut shell, vec![egui::Event::PointerMoved(from)]);
    pointer(&mut shell, vec![button(from, true)]);
    for step in 1..=8 {
        let a = std::f32::consts::FRAC_PI_2 * step as f32 / 8.0;
        pointer(&mut shell, vec![egui::Event::PointerMoved(ring(a))]);
    }
    pointer(&mut shell, vec![button(to, false)]);
    shell.settled_sized(TALL);
    let turned = or_maps::edit::pose_of(&the_map_file(&shell).entities[0]);
    let (_, _, about_z) = turned.rotation.to_euler(glam::EulerRot::XYZ);
    assert!(
        (about_z.to_degrees() - 90.0).abs() < 16.0,
        "turned about a quarter, held to a step: {}",
        about_z.to_degrees()
    );
    assert_eq!(shell.state.workbench.maps.undo.len(), 1);

    // The centre of the scale tool.
    shell.state.workbench.maps.tool = Mode::Scale;
    let view = look_at(&mut shell, origin);
    let gizmo = Gizmo::at(Mode::Scale, origin, Quat::IDENTITY, &view).unwrap();
    let centre = at(view.project(origin).unwrap());
    let out = at(view.project(origin + Vec3::Z * gizmo.reach * 0.7).unwrap());
    let _ = Axis::Z;
    pointer(&mut shell, vec![egui::Event::PointerMoved(centre)]);
    pointer(&mut shell, vec![button(centre, true)]);
    for step in 1..=6 {
        let t = step as f32 / 6.0;
        let mid = egui::pos2(
            centre.x + (out.x - centre.x) * t,
            centre.y + (out.y - centre.y) * t,
        );
        pointer(&mut shell, vec![egui::Event::PointerMoved(mid)]);
    }
    pointer(&mut shell, vec![button(out, false)]);
    shell.settled_sized(TALL);
    let scaled = or_maps::edit::pose_of(&the_map_file(&shell).entities[0]);
    assert!(scaled.scale_xy > 1.2 && scaled.scale_z > 1.2, "{scaled:?}");
    assert_eq!(
        scaled.scale_xy, scaled.scale_z,
        "the centre scales every axis together"
    );
    assert_eq!(shell.state.workbench.maps.undo.len(), 2);
}

#[test]
fn a_typed_position_and_the_arrow_keys_do_what_the_handles_do() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_maps_shell(dir.path());
    select_entity(&mut shell, 0);

    // The number typed in the selection.
    let at_x = centre_of(&shell, "Entity position X");
    pointer(
        &mut shell,
        vec![egui::Event::PointerMoved(at_x), button(at_x, true)],
    );
    pointer(&mut shell, vec![button(at_x, false)]);
    pointer(
        &mut shell,
        vec![egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }],
    );
    pointer(&mut shell, vec![egui::Event::Text("42.5".to_string())]);
    pointer(&mut shell, vec![key(egui::Key::Enter)]);
    shell.settled_sized(TALL);
    assert_eq!(the_map_file(&shell).entities[0].position.x, 42.5);
    assert_eq!(shell.state.workbench.maps.undo.len(), 1);

    // A key moves it a step of the grid from the scene, which a click gives the keyboard to.
    shell.state.workbench.maps.snap.grid_on = true;
    shell.state.workbench.maps.snap.grid = 0.5;
    let view = look_at(&mut shell, Vec3::new(42.5, 20.0, 0.5));
    let empty = at(view.origin + glam::Vec2::new(view.size.x * 0.9, view.size.y * 0.9));
    pointer(
        &mut shell,
        vec![egui::Event::PointerMoved(empty), button(empty, true)],
    );
    pointer(&mut shell, vec![button(empty, false)]);
    select_entity(&mut shell, 0);
    let scene_at = centre_of(&shell, "Scene of the map set");
    let _ = scene_at;
    tap(&mut shell, egui::Key::ArrowRight);
    assert_eq!(
        the_map_file(&shell).entities[0].position.x,
        43.0,
        "a step east of the grid"
    );
    // Every other arrow, and the two keys for height, do the same along their own axis.
    for (keys, expect) in [
        (vec![egui::Key::ArrowUp], Vec3::new(43.0, 20.5, 0.5)),
        (
            vec![egui::Key::ArrowDown, egui::Key::ArrowDown],
            Vec3::new(43.0, 19.5, 0.5),
        ),
        (vec![egui::Key::ArrowLeft], Vec3::new(42.5, 19.5, 0.5)),
        (vec![egui::Key::PageUp], Vec3::new(42.5, 19.5, 1.0)),
        (
            vec![egui::Key::PageDown, egui::Key::PageDown],
            Vec3::new(42.5, 19.5, 0.0),
        ),
    ] {
        for each in keys {
            tap(&mut shell, each);
        }
        let now = the_map_file(&shell).entities[0].position;
        assert_eq!(
            Vec3::new(now.x, now.y, now.z),
            expect,
            "after the keys {:?}",
            now
        );
    }

    // The rotation is typed as the ring would turn it: ninety degrees about Z.
    let at_z = centre_of(&shell, "Entity rotation about Z in degrees");
    pointer(
        &mut shell,
        vec![egui::Event::PointerMoved(at_z), button(at_z, true)],
    );
    pointer(&mut shell, vec![button(at_z, false)]);
    pointer(
        &mut shell,
        vec![egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }],
    );
    pointer(&mut shell, vec![egui::Event::Text("90".to_string())]);
    pointer(&mut shell, vec![key(egui::Key::Enter)]);
    shell.settled_sized(TALL);
    let turned = or_maps::edit::pose_of(&the_map_file(&shell).entities[0]).rotation;
    let ninety = Quat::from_rotation_z(90f32.to_radians());
    assert!(
        turned.dot(ninety).abs() > 0.9999,
        "ninety degrees about Z: {turned:?}"
    );
}

/// Opening a project that already holds a map pack frames the camera on its set, as importing the files does.
#[test]
fn a_project_opened_with_maps_in_it_frames_the_camera_on_the_set() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    drop(a_maps_shell(dir.path()));
    let mut shell = Shell::new(crate::state::ThemeChoice::Dark);
    shell
        .state
        .load_project_from(dir.path().join("harbour").join("harbour.openrage-proj"));
    shell.state.active_workspace = Workspace::Map;
    until(&mut shell, "the maps", |s| {
        s.state
            .shown_scene()
            .is_some_and(|scene| scene.placed.len() == 2)
    });
    let camera = shell
        .state
        .step_viewport_camera(crate::render::viewport::CameraInputs::default())
        .clone();
    // The crates stand at (10, 20) and (16, 20): the camera looks between them.
    let away = (camera.target - Vec3::new(13.0, 20.0, 0.5)).length();
    assert!(
        away < 3.0,
        "the camera looks {away} m from the set: {camera:?}"
    );
}

/// The codes of the rules that fire on the sources as the disk holds them now, without waiting for a scan.
fn findings_now(shell: &Shell) -> Vec<&'static str> {
    let folder = shell.state.maps_pack().expect("a pack").folder.clone();
    let model = or_maps::Maps::read(&folder).expect("the model reads");
    let facts = or_maps::inspect(&folder, &model, or_maps::Depth::Open);
    or_maps::check(&model, &facts, None)
        .into_iter()
        .filter(|f| f.waived.is_none())
        .map(|f| f.code)
        .collect()
}

/// An entity moved or added past the extents the map states grows them in the same step, so the source never states extents short of
/// what it places, and one Undo takes the growth back with the placement.
#[test]
fn an_entity_moved_or_added_past_the_extents_grows_them_in_the_same_step() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_maps_shell(dir.path());
    select_entity(&mut shell, 0);
    let stated = the_map_file(&shell);
    assert!(stated.entities_extents_max.x < 20.0, "{stated:?}");
    assert!(
        !findings_now(&shell).contains(&"extents-short"),
        "the files start inside their extents"
    );

    let pose = shell.state.selected_pose().expect("a pose");
    shell.state.place_selected_entity(
        or_maps::edit::Pose {
            position: Vec3::new(150.0, 20.0, 0.5),
            ..pose
        },
        "Moved".to_string(),
    );
    let moved = the_map_file(&shell);
    assert_eq!(moved.entities[0].position.x, 150.0);
    assert!(
        moved.entities_extents_max.x >= 151.0,
        "{:?}",
        moved.entities_extents_max
    );
    assert!(
        moved.streaming_extents_max.x >= 151.0,
        "{:?}",
        moved.streaming_extents_max
    );
    assert_eq!(
        moved.entities_extents_min.x, stated.entities_extents_min.x,
        "the minimum the file stated stays"
    );
    assert!(
        !findings_now(&shell).contains(&"extents-short"),
        "{:?}",
        findings_now(&shell)
    );

    shell.state.add_entity_at(
        or_maps::Name::from_text("prop_crate_a"),
        or_maps::edit::Pose {
            position: Vec3::new(300.0, 20.0, 0.5),
            ..pose
        },
    );
    assert_eq!(the_map_file(&shell).entities.len(), 3);
    assert!(
        the_map_file(&shell).entities_extents_max.x >= 301.0,
        "{:?}",
        the_map_file(&shell).entities_extents_max
    );
    assert!(!findings_now(&shell).contains(&"extents-short"));

    shell.state.undo_maps_change();
    assert_eq!(the_map_file(&shell).entities.len(), 2);
    assert!(
        (151.0..200.0).contains(&the_map_file(&shell).entities_extents_max.x),
        "one Undo takes back the growth of the add and no more"
    );
    shell.state.undo_maps_change();
    assert_eq!(
        the_map_file(&shell).entities_extents_max.x,
        stated.entities_extents_max.x,
        "one more puts the extents back with the move"
    );
}

/// A screen reader speaks the name of a row of the list, so it is French in the French interface.
#[test]
fn the_rows_of_the_list_are_named_in_french_in_the_french_interface() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut shell = {
        let _english = crate::i18n::pin("en");
        a_maps_shell(dir.path())
    };
    let _french = crate::i18n::pin("fr");
    shell.settled();
    for french in ["Entité #1  prop_crate_b", "Archétype #0  prop_crate_a"] {
        assert!(shows(&shell, french), "{french}: {:?}", names(&shell));
    }
    for english in ["Entity #1  prop_crate_b", "Archetype #0  prop_crate_a"] {
        assert!(
            !shows(&shell, english),
            "{english} is still English: {:?}",
            names(&shell)
        );
    }
}

/// The edits the manipulator does not make are operations too: an entity renamed or taken out and an archetype added, changed or
/// taken out are each one step, and Undo puts each source back to the bytes it held.
#[test]
fn an_entity_renamed_or_removed_and_an_archetype_added_changed_or_removed_are_each_one_step_undo_puts_back(
) {
    use or_io::decl::File;
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_maps_shell(dir.path());
    let folder = shell.state.maps_pack().expect("a pack").folder.clone();
    let types_source = "harbour_types.metadata.src";
    let xml = |source: &str| std::fs::read(folder.join(source).join("metadata.xml")).unwrap();
    let before = (xml("harbour.metadata.src"), xml(types_source));
    let types = || {
        or_io::meta::typed::CMapTypes::from_bytes(
            &or_maps::facts::cooked(&folder.join(types_source)).unwrap(),
        )
        .unwrap()
    };

    select_entity(&mut shell, 1);
    shell.state.rename_selected_entity_archetype("prop_crate_a");
    assert!(the_map_file(&shell).entities[1].archetype_name == "prop_crate_a");
    assert_eq!(the_map_file(&shell).entities[1].guid, 2);
    shell.state.remove_selected_entity();
    let map = the_map_file(&shell);
    assert_eq!(map.entities.len(), 1);
    assert_eq!(map.entities[0].guid, 1);
    assert_eq!(shell.state.workbench.maps.undo.len(), 2);

    let (module, set) = {
        let (pack, set) = shell.state.shown_set().expect("a set");
        (pack.module.clone(), set.id.clone())
    };
    shell.state.select_in_maps(
        &module,
        &set,
        Some(Pick {
            source: types_source.to_string(),
            kind: PickKind::Archetype(1),
        }),
    );
    shell
        .state
        .add_archetype_named("prop_new_crate", Some("prop_new_crate_lod"));
    assert_eq!(types().archetypes.len(), 3);
    assert!(types().archetypes[2].asset_name == "prop_new_crate_lod");
    assert_eq!(
        shell.state.workbench.maps.selected,
        Some(Pick {
            source: types_source.to_string(),
            kind: PickKind::Archetype(2),
        }),
        "the archetype added is the one selected"
    );
    shell
        .state
        .edit_selected_archetype("Set the box".to_string(), &[], |archetype| {
            archetype.bb_max.z = 9.5;
        });
    assert_eq!(types().archetypes[2].bb_max.z, 9.5);
    assert!(types().archetypes[2].name == "prop_new_crate");
    // A name another archetype holds is refused, and the source stays as it was.
    let held = xml(types_source);
    shell
        .state
        .edit_selected_archetype("Renamed".to_string(), &[], |archetype| {
            archetype.name = or_io::meta::instance::HashedString::new("prop_crate_a");
        });
    assert_eq!(xml(types_source), held);
    assert_eq!(shell.state.workbench.maps.undo.len(), 4);
    shell.state.remove_selected_archetype();
    assert_eq!(types().archetypes.len(), 2);
    assert_eq!(shell.state.workbench.maps.undo.len(), 5);

    for _ in 0..5 {
        shell.state.undo_maps_change();
    }
    assert!(shell.state.workbench.maps.undo.is_empty());
    assert!(
        (xml("harbour.metadata.src"), xml(types_source)) == before,
        "five steps back, the sources hold the bytes they held"
    );
}

/// What is added is a copy of what is selected, and not of the first record its file holds.
#[test]
fn an_entity_and_an_archetype_added_are_copies_of_the_one_selected() {
    use or_io::decl::File;
    use or_maps::operation::{apply, Operation, Record};
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_maps_shell(dir.path());
    let folder = shell.state.maps_pack().expect("a pack").folder.clone();
    let types_source = "harbour_types.metadata.src";

    // The second entity is given a distance of its own, which the first does not state.
    let mut second = the_map_file(&shell).entities[1].clone();
    let first = the_map_file(&shell).entities[0].lod_dist;
    second.lod_dist = first + 23.5;
    apply(
        &folder,
        &Operation::ReplaceEntity {
            source: "harbour.metadata.src".to_string(),
            entity: 1,
            record: Record::entity(&second).unwrap(),
            extents: None,
        },
    )
    .unwrap();
    select_entity(&mut shell, 1);
    let pose = shell.state.selected_pose().expect("a pose");
    shell
        .state
        .add_entity_at(or_maps::Name::from_text("prop_crate_a"), pose);
    let map = the_map_file(&shell);
    assert_eq!(map.entities.len(), 3);
    assert_eq!(map.entities[2].lod_dist, first + 23.5);
    assert!(map.entities[2].archetype_name == "prop_crate_a");

    let (module, set) = {
        let (pack, set) = shell.state.shown_set().expect("a set");
        (pack.module.clone(), set.id.clone())
    };
    shell.state.select_in_maps(
        &module,
        &set,
        Some(Pick {
            source: types_source.to_string(),
            kind: PickKind::Archetype(1),
        }),
    );
    shell
        .state
        .edit_selected_archetype("Set the box".to_string(), &[], |archetype| {
            archetype.bb_max.z = 9.5;
        });
    shell.state.add_archetype_named("prop_new_crate", None);
    let types = or_io::meta::typed::CMapTypes::from_bytes(
        &or_maps::facts::cooked(&folder.join(types_source)).unwrap(),
    )
    .unwrap();
    assert_eq!(types.archetypes.len(), 3);
    assert_eq!(types.archetypes[0].bb_max.z, 1.0);
    assert_eq!(types.archetypes[2].bb_max.z, 9.5);
    assert!(types.archetypes[2].name == "prop_new_crate");
}
