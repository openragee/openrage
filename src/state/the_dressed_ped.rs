//! The dressed ped of the Wardrobe: what the preview reads and draws of a pack, the tools that change it, a texture made from an
//! image dropped on its tile or saved beside it, and the two ways a recook reaches what is open.
//!
//! ⚠ Built with `AppState::from_prefs` and a clothing project in a scratch folder, whose garments are real
//! models and textures from the corpus (the preview reads files, and a dummy one would draw nothing).
//! The ped's own drawables come from an install, so those cases are install-gated.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use or_io::codec::texture::{save_png, RgbaImage};
use or_source::cook::Recook;
use or_test_support::{game_roots, skip_corpus, skip_install, Corpus, TARGET_GEN8};
use or_wardrobe::edit::Edit;

use super::front_door::{NewProject, Template};
use super::os_drop::DropTarget;
use super::{AppState, Prefs};
use crate::ui::dock::Workspace;

pub(super) const WHAT: &str = "the_dressed_ped";
pub(super) const C: &str = "mp_m_freemode_01_mp_m_t";

pub(super) fn until(state: &mut AppState, what: &str, done: impl Fn(&AppState) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        state.poll_workbench();
        if done(state) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what} never happened; the preview says {:?}; the history says:\n{}",
            state.workbench.wardrobe.dressed.notes,
            state
                .workbench
                .history()
                .iter()
                .map(|e| e.text.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The smallest staged files of `extension` that open and, for a model, hold geometry.
pub(super) fn staged(extension: &str, count: usize) -> Option<Vec<PathBuf>> {
    let mut files = Corpus::new(env!("CARGO_MANIFEST_DIR"), extension)
        .target(TARGET_GEN8)
        .find_or_skip(WHAT)?;
    files.sort_by_key(|p| std::fs::metadata(p).map_or(u64::MAX, |m| m.len()));
    let usable: Vec<PathBuf> = files
        .into_iter()
        .take(150)
        .filter(|path| {
            let Ok(asset) = or_io::open(path) else {
                return false;
            };
            match extension {
                "ydd" => !crate::render::meshes::meshes_of_asset(&asset).is_empty(),
                _ => true,
            }
        })
        .take(count)
        .collect();
    if usable.len() < count {
        skip_corpus(
            WHAT,
            format!("fewer than {count} usable .{extension} files"),
        );
        return None;
    }
    Some(usable)
}

fn png(path: &Path, shade: u8) {
    let data: Vec<u8> = (0..64 * 64)
        .flat_map(|i| [shade, (i % 64) as u8 * 4, (i / 64) as u8 * 4, 255])
        .collect();
    save_png(
        path,
        &RgbaImage {
            width: 64,
            height: 64,
            data,
            is_srgb: true,
        },
    )
    .unwrap();
}

/// A clothing project holding a jacket (two textures) and a pair of shoes, open on the Wardrobe.
pub(super) struct Fitted {
    pub(super) state: AppState,
    pub(super) jacket: String,
    pub(super) shoes: String,
    /// Last, so the workers and the watchers are gone before their folder is.
    _dir: tempfile::TempDir,
}

fn fitted() -> Option<Fitted> {
    fitted_from(staged("ydd", 2)?)
}

/// What the first mesh of the first part is made of, which another model does not share.
pub(super) fn first_mesh(f: &Fitted) -> (usize, usize) {
    let mesh = &f.scene_parts()[0].geometry[0];
    (mesh.vertices.len(), mesh.indices.len())
}

/// The smallest staged model whose medium level has fewer triangles than its high one, and the smallest other one.
pub(super) fn staged_with_levels() -> Option<Vec<PathBuf>> {
    let mut files = Corpus::new(env!("CARGO_MANIFEST_DIR"), "ydd")
        .target(TARGET_GEN8)
        .find_or_skip(WHAT)?;
    files.sort_by_key(|p| std::fs::metadata(p).map_or(u64::MAX, |m| m.len()));
    let triangles = |asset: &or_io::Asset, lod| -> usize {
        (crate::render::meshes::meshes_of_asset_at(asset, lod).iter())
            .map(|m| m.indices.len() / 3)
            .sum()
    };
    let opened: Vec<(PathBuf, or_io::Asset)> = files
        .into_iter()
        .take(150)
        .filter_map(|path| Some((path.clone(), or_io::open(&path).ok()?)))
        .collect();
    let many = opened.iter().find(|(_, asset)| {
        (1..triangles(asset, 0)).contains(&triangles(asset, 1)) && triangles(asset, 1) > 0
    });
    let other = opened
        .iter()
        .find(|(path, asset)| Some(path) != many.map(|m| &m.0) && triangles(asset, 0) > 0);
    let (Some(many), Some(other)) = (many, other) else {
        skip_corpus(WHAT, "no staged model has a lighter medium level");
        return None;
    };
    Some(vec![many.0.clone(), other.0.clone()])
}

pub(super) fn fitted_from(models: Vec<PathBuf>) -> Option<Fitted> {
    fitted_dropping(models, false)
}

/// The same pack with a hat in it as well, which is a third garment and the second model worn on the head.
pub(super) fn fitted_dropping(models: Vec<PathBuf>, with_hat: bool) -> Option<Fitted> {
    let textures = staged("ytd", 3)?;
    let dir = tempfile::tempdir().unwrap();
    let mut draft = NewProject::new(Template::Clothing, dir.path().to_path_buf());
    draft.set_name("tshirts".into());
    let created = draft.create().expect("the template makes a project");
    let mut state = AppState::from_prefs(Prefs::default());
    state.documents.clear();
    state.active_document = None;
    state.load_project_from(created.file.clone());
    until(&mut state, "the project's scan", |s| {
        s.workbench.is_scanned()
    });

    let drop = dir.path().join("drop");
    std::fs::create_dir_all(&drop).unwrap();
    let copy = |from: &Path, name: String| {
        let to = drop.join(name);
        std::fs::copy(from, &to).unwrap();
        to
    };
    let mut dropped = vec![
        copy(&models[0], format!("{C}^jbib_000_u.ydd")),
        copy(&textures[0], format!("{C}^jbib_diff_000_a_uni.ytd")),
        copy(&textures[1], format!("{C}^jbib_diff_000_b_uni.ytd")),
        copy(&models[1], format!("{C}^feet_000_u.ydd")),
        copy(&textures[2], format!("{C}^feet_diff_000_a_uni.ytd")),
    ];
    if with_hat {
        dropped.push(copy(&models[1], format!("{C}^p_head_000.ydd")));
        dropped.push(copy(&textures[2], format!("{C}^p_head_diff_000_a.ytd")));
    }
    let garments = 2 + usize::from(with_hat);
    state.handle_os_drop(dropped, DropTarget::Wardrobe);
    until(&mut state, "the drop", |s| {
        s.wardrobe_pack()
            .and_then(|p| p.read.as_ref().ok())
            .is_some_and(|read| read.wardrobe.garments.len() == garments)
    });
    let pack = state.wardrobe_pack().unwrap();
    let wardrobe = &pack.read.as_ref().unwrap().wardrobe;
    let id_of = |slot: &str| {
        wardrobe
            .garments
            .iter()
            .find(|g| g.model.as_deref().is_some_and(|m| m.contains(slot)))
            .unwrap()
            .id
            .clone()
    };
    let (jacket, shoes) = (id_of("jbib"), id_of("feet"));
    state.active_workspace = Workspace::Wardrobe;
    Some(Fitted {
        state,
        jacket,
        shoes,
        _dir: dir,
    })
}

impl Fitted {
    pub(super) fn select(&mut self, id: &str) {
        let module = self.state.wardrobe_module_id().unwrap();
        self.state.select_garment(&module, Some(id.to_string()));
    }

    /// Poll until the preview has drawn `parts` parts of a scene it is not waiting to replace.
    pub(super) fn shows(&mut self, parts: usize) {
        until(&mut self.state, "the preview", |s| {
            let d = &s.workbench.wardrobe.dressed;
            !d.is_reading() && d.scene().is_some_and(|scene| scene.parts.len() == parts)
        });
    }

    pub(super) fn scene_parts(&self) -> &[crate::render::dressed::ScenePart] {
        &self
            .state
            .workbench
            .wardrobe
            .dressed
            .scene()
            .expect("a scene")
            .parts
    }
}

#[test]
fn switching_the_level_of_detail_changes_what_is_drawn_and_what_is_counted() {
    let Some(models) = staged_with_levels() else {
        return;
    };
    // The levels the file holds models at, which the scene says the garment holds.
    let expected: [usize; 4] = std::array::from_fn(|lod| {
        let asset = or_io::open(&models[0]).expect("a staged model opens");
        usize::from(!crate::render::meshes::meshes_of_asset_at(&asset, lod).is_empty())
    });
    let Some(mut f) = fitted_from(models) else {
        return;
    };
    let jacket = f.jacket.clone();
    f.select(&jacket);
    f.shows(1);
    let scene = f.state.workbench.wardrobe.dressed.scene().unwrap();
    let (high, levels) = (scene.drawn(), scene.levels);
    assert_eq!(high, levels[0], "the high level is drawn to begin with");
    assert_eq!(scene.garments(), 1);
    assert_eq!(
        scene.held, expected,
        "the levels the garment holds models at"
    );
    for (cost, _) in levels.iter().zip(expected).filter(|(_, held)| *held == 0) {
        assert_eq!(
            *cost, levels[0],
            "a level it lacks costs its first, as it is drawn"
        );
    }
    assert!(
        levels[1].triangles > 0 && levels[1].triangles < levels[0].triangles,
        "the staged model has a medium level with fewer triangles: {levels:?}"
    );

    f.state.workbench.wardrobe.dressed.lod = 1;
    until(&mut f.state, "the medium level", |s| {
        let d = &s.workbench.wardrobe.dressed;
        !d.is_reading() && d.scene().is_some_and(|scene| scene.drawn() != high)
    });
    let scene = f.state.workbench.wardrobe.dressed.scene().unwrap();
    assert_eq!(scene.drawn().triangles, levels[1].triangles);
    assert_eq!(
        scene.levels, levels,
        "the table is the same whichever level is drawn"
    );

    f.state.workbench.wardrobe.dressed.lod = 0;
    until(&mut f.state, "the high level again", |s| {
        let d = &s.workbench.wardrobe.dressed;
        !d.is_reading() && d.scene().is_some_and(|scene| scene.drawn() == high)
    });
}

#[test]
fn a_replaced_model_is_drawn_without_reopening_and_its_undo_draws_the_old_one_again() {
    let Some(mut f) = fitted() else { return };
    let Some(spare) = staged("ydd", 6) else {
        return;
    };
    let jacket = f.jacket.clone();
    f.select(&jacket);
    f.shows(1);
    let before = first_mesh(&f);
    // The smallest staged models the fixture did not use: one that is not the jacket's own mesh.
    let replacement = spare
        .into_iter()
        .skip(2)
        .find(|path| {
            let Ok(asset) = or_io::open(path) else {
                return false;
            };
            crate::render::meshes::meshes_of_asset(&asset)
                .first()
                .is_some_and(|m| (m.vertices.len(), m.indices.len()) != before)
        })
        .expect("a staged model with another mesh");

    f.state.replace_model(jacket.clone(), replacement);
    until(&mut f.state, "the replaced model", |s| {
        let d = &s.workbench.wardrobe.dressed;
        !d.is_reading()
            && d.scene().is_some_and(|scene| {
                let Some(mesh) = scene.parts.first().and_then(|p| p.geometry.first()) else {
                    return false;
                };
                (mesh.vertices.len(), mesh.indices.len()) != before
            })
    });

    f.state.undo_last_change();
    until(&mut f.state, "the old model again", |s| {
        let d = &s.workbench.wardrobe.dressed;
        !d.is_reading()
            && d.scene().is_some_and(|scene| {
                let Some(mesh) = scene.parts.first().and_then(|p| p.geometry.first()) else {
                    return false;
                };
                (mesh.vertices.len(), mesh.indices.len()) == before
            })
    });
}

#[test]
fn a_selected_garment_is_worn_alone_when_no_game_is_mounted_and_the_preview_says_so() {
    let Some(mut f) = fitted() else { return };
    let jacket = f.jacket.clone();
    f.select(&jacket);
    f.shows(1);

    let part = &f.scene_parts()[0];
    assert!(
        part.geometry.iter().any(|g| !g.indices.is_empty()),
        "its model was read"
    );
    assert!(part.diffuse.is_some(), "its first texture was read");
    assert_eq!(part.lift, 0.0);
    let notes = f.state.workbench.wardrobe.dressed.notes.join(" | ");
    assert!(notes.contains("No game install is chosen"), "{notes}");
    assert!(f.state.workbench.wardrobe.dressed.scene().unwrap().radius > 0.0);
}

#[test]
fn an_outfit_wears_several_garments_and_the_selected_one_goes_on_top() {
    let Some(mut f) = fitted() else { return };
    let (jacket, shoes) = (f.jacket.clone(), f.shoes.clone());
    f.select(&jacket);
    f.state.workbench.wardrobe.dressed.toggle_in_outfit(&jacket);
    f.select(&shoes);
    f.shows(1);

    // With Outfit off the selected garment is worn alone, whatever the outfit holds.
    f.state.workbench.wardrobe.dressed.outfit_on = true;
    f.shows(2);
    f.state.workbench.wardrobe.dressed.toggle_in_outfit(&jacket);
    f.shows(1);
}

#[test]
fn the_heel_flag_raises_every_part_of_the_ped_by_a_tenth_of_its_value_and_the_heels_button_takes_it_off(
) {
    let Some(mut f) = fitted() else { return };
    let (jacket, shoes) = (f.jacket.clone(), f.shoes.clone());
    f.state.workbench.wardrobe.dressed.outfit = vec![jacket.clone()];
    f.state.workbench.wardrobe.dressed.outfit_on = true;
    f.select(&shoes);
    f.shows(2);
    assert!(
        f.scene_parts().iter().all(|p| p.lift == 0.0),
        "shoes with no heels raise nothing"
    );

    f.state.edit_garment(shoes.clone(), Edit::Heels(Some(1.5)));
    until(&mut f.state, "the heels in the preview", |s| {
        s.workbench.wardrobe.dressed.scene().is_some_and(|scene| {
            scene.parts.len() == 2
                && scene
                    .parts
                    .iter()
                    .all(|p| p.lift == 1.5 * or_wardrobe::dress::METRES_PER_HEEL_UNIT)
        })
    });

    f.state.workbench.wardrobe.dressed.heels = false;
    until(&mut f.state, "the heels taken off", |s| {
        s.workbench
            .wardrobe
            .dressed
            .scene()
            .is_some_and(|scene| scene.parts.iter().all(|p| p.lift == 0.0))
    });
}

#[test]
fn a_and_b_show_the_ped_without_the_pack_and_with_it() {
    let Some(mut f) = fitted() else { return };
    let jacket = f.jacket.clone();
    f.select(&jacket);
    f.shows(1);
    f.state.workbench.wardrobe.dressed.original = true;
    until(&mut f.state, "A", |s| {
        s.workbench
            .wardrobe
            .dressed
            .scene()
            .is_some_and(|scene| scene.parts.is_empty())
    });
    f.state.workbench.wardrobe.dressed.original = false;
    f.shows(1);
}

#[test]
fn the_texture_variation_chosen_is_the_one_drawn() {
    let Some(mut f) = fitted() else { return };
    let jacket = f.jacket.clone();
    f.select(&jacket);
    f.shows(1);
    let first = f.scene_parts()[0].diffuse.as_ref().unwrap().rgba.clone();

    f.state
        .workbench
        .wardrobe
        .dressed
        .variation
        .insert(jacket.clone(), 1);
    until(&mut f.state, "the second texture", |s| {
        s.workbench.wardrobe.dressed.scene().is_some_and(|scene| {
            scene
                .parts
                .first()
                .and_then(|p| p.diffuse.as_ref())
                .is_some_and(|d| d.rgba != first)
        })
    });
    f.state
        .workbench
        .wardrobe
        .dressed
        .variation
        .insert(jacket, 0);
    until(&mut f.state, "the first texture again", |s| {
        s.workbench.wardrobe.dressed.scene().is_some_and(|scene| {
            scene
                .parts
                .first()
                .and_then(|p| p.diffuse.as_ref())
                .is_some_and(|d| d.rgba == first)
        })
    });
}

#[test]
fn an_image_dropped_on_a_tile_makes_that_variations_texture_and_the_preview_shows_it() {
    let Some(mut f) = fitted() else { return };
    let jacket = f.jacket.clone();
    f.select(&jacket);
    f.state
        .workbench
        .wardrobe
        .dressed
        .variation
        .insert(jacket.clone(), 1);
    f.shows(1);
    let before = f.scene_parts()[0].diffuse.as_ref().unwrap().rgba.clone();
    let (a, b) = (texture_file(&f, 0), texture_file(&f, 1));

    let image = f._dir.path().join("my_edit.png");
    png(&image, 220);
    f.state.workbench.wardrobe.dressed.drop_tile = Some((jacket, 1));
    f.state.handle_os_drop(vec![image], DropTarget::Variant);

    until(&mut f.state, "the preview to show the image", |s| {
        s.workbench.wardrobe.dressed.scene().is_some_and(|scene| {
            scene
                .parts
                .first()
                .and_then(|p| p.diffuse.as_ref())
                .is_some_and(|d| d.rgba != before)
        })
    });
    // The image is the source of that variation, and no other variation changed.
    assert_eq!(texture_file(&f, 0), a);
    assert_ne!(texture_file(&f, 1), b);
    assert!(
        texture_file(&f, 1).ends_with(".textures.src"),
        "{}",
        texture_file(&f, 1)
    );
}

/// The file name of variation `i` of the jacket.
fn texture_file(f: &Fitted, i: usize) -> String {
    let pack = f.state.wardrobe_pack().unwrap();
    let read = pack.read.as_ref().unwrap();
    let g = read.wardrobe.garment(&f.jacket).unwrap();
    g.variations[i]
        .texture
        .as_deref()
        .and_then(|t| t.rsplit('/').next())
        .unwrap()
        .to_string()
}

#[test]
fn the_ped_wears_its_own_drawables_from_the_install_in_every_slot_nothing_selects() {
    let Some(root) = game_roots().into_iter().next() else {
        skip_install(WHAT, "OPENRAGE_GAME_ROOT names no install");
        return;
    };
    let Some(mut f) = fitted() else { return };
    let mut with_keys = super::opening_a_ydd::with_keys(&root);
    std::mem::swap(&mut f.state.keys, &mut with_keys.keys);
    let game = crate::state::prefs::Game::detect_from_root(&root).expect("a game's install");
    f.state.prefs.game_paths.set(game, Some(root));
    f.state.prefs.active_game = Some(game);
    let jacket = f.jacket.clone();
    f.select(&jacket);
    until(&mut f.state, "the install's drawables", |s| {
        let d = &s.workbench.wardrobe.dressed;
        !d.is_reading() && d.scene().is_some_and(|scene| scene.parts.len() > 3)
    });
    let scene = f.state.workbench.wardrobe.dressed.scene().unwrap();
    assert!(scene.parts.iter().filter(|p| p.diffuse.is_some()).count() > 3);
    let notes = f.state.workbench.wardrobe.dressed.notes.join(" | ");
    assert!(!notes.contains("No game install is chosen"), "{notes}");
}

#[test]
fn a_cook_that_lands_lets_a_model_ask_the_project_for_its_textures_again() {
    let mut state = AppState::from_prefs(Prefs::default());
    state.attempted_texture_project.insert(7);
    state.took_recook(Recook::Cooked {
        source: PathBuf::from("logo.textures.src"),
        output: PathBuf::from("vfs/id/logo.ytd"),
    });
    assert!(
        state.attempted_texture_project.is_empty(),
        "a model opened before its textures were first cooked asks again"
    );
}

#[test]
fn a_file_a_recook_wrote_is_asked_about_as_refreshed_and_not_as_changed_by_somebody_else() {
    use super::document::{DocPayload, Document};
    let dir = tempfile::tempdir().unwrap();
    let cooked = dir.path().join("logo.ytd");
    std::fs::write(&cooked, b"x").unwrap();
    let mut state = AppState::from_prefs(Prefs::default());
    let mut tab = Document::new(
        1,
        Some(cooked.clone()),
        DocPayload::Resource(or_io::Asset::TextureDict(Default::default())),
    );
    tab.mark_edited();
    state.documents.push(tab);

    state.took_recook(Recook::Cooked {
        source: dir.path().join("logo.textures.src"),
        output: cooked.clone(),
    });
    assert_eq!(
        state.reload_conflicts,
        std::slice::from_ref(&cooked),
        "a tab with edits asks first"
    );
    assert!(state.workbench.refreshed.contains(&cooked));

    // Somebody else changing a file is not a refresh.
    let other = dir.path().join("other.ytd");
    state.reload_conflicts.push(other.clone());
    assert!(!state.workbench.refreshed.contains(&other));

    state.answer_reload_conflicts(false);
    assert!(state.reload_conflicts.is_empty());
    assert!(
        state.workbench.refreshed.is_empty(),
        "the answer forgets what was asked"
    );
}

/// A of A/B with the ped's own drawables: the same slots the pack's garment took, now worn by the ped.
#[test]
fn a_shows_the_peds_own_drawable_in_the_slot_the_garment_took() {
    let Some(root) = game_roots().into_iter().next() else {
        skip_install(WHAT, "OPENRAGE_GAME_ROOT names no install");
        return;
    };
    let Some(mut f) = fitted() else { return };
    let mut with_keys = super::opening_a_ydd::with_keys(&root);
    std::mem::swap(&mut f.state.keys, &mut with_keys.keys);
    let game = crate::state::prefs::Game::detect_from_root(&root).expect("a game's install");
    f.state.prefs.game_paths.set(game, Some(root));
    f.state.prefs.active_game = Some(game);
    let jacket = f.jacket.clone();
    f.select(&jacket);
    until(&mut f.state, "the ped with the jacket", |s| {
        let d = &s.workbench.wardrobe.dressed;
        !d.is_reading() && d.scene().is_some_and(|scene| scene.parts.len() > 3)
    });
    let dressed = f.scene_parts().len();
    let jacket_pixels = f
        .scene_parts()
        .iter()
        .filter_map(|p| p.diffuse.as_ref())
        .map(|d| d.id)
        .collect::<Vec<_>>();

    f.state.workbench.wardrobe.dressed.original = true;
    until(&mut f.state, "A", |s| {
        let d = &s.workbench.wardrobe.dressed;
        !d.is_reading()
            && d.scene().is_some_and(|scene| {
                let ids: Vec<u64> = scene
                    .parts
                    .iter()
                    .filter_map(|p| p.diffuse.as_ref())
                    .map(|d| d.id)
                    .collect();
                ids != jacket_pixels
            })
    });
    // The ped's own drawable takes the slot, so there are as many parts, and one of them is another one.
    assert_eq!(f.scene_parts().len(), dressed);
}

#[test]
fn the_wardrobe_has_a_camera_of_its_own_that_is_framed_on_the_ped_orbits_and_frames_again() {
    use crate::render::viewport::CameraInputs;
    let Some(mut f) = fitted() else { return };
    let jacket = f.jacket.clone();
    f.select(&jacket);
    f.shows(1);
    let centre = f.state.workbench.wardrobe.dressed.scene().unwrap().centre;

    let framed = f
        .state
        .step_viewport_camera(CameraInputs::default())
        .clone();
    assert!(
        (framed.target - centre).length() < 1e-3,
        "framed on the ped, not on a document"
    );

    let orbit = CameraInputs {
        orbit: egui::vec2(60.0, 10.0),
        ..CameraInputs::default()
    };
    let turned = f.state.step_viewport_camera(orbit).clone();
    assert_ne!(turned.yaw, framed.yaw, "the drag turned it");
    assert!((turned.target - centre).length() < 1e-3, "about the ped");

    f.state.workbench.wardrobe.dressed.camera.target = glam::Vec3::splat(40.0);
    let again = CameraInputs {
        frame_active: true,
        ..CameraInputs::default()
    };
    let back = f.state.step_viewport_camera(again).clone();
    assert!(
        (back.target - centre).length() < 1e-3,
        "Frame puts it back on the ped"
    );
}

#[test]
fn a_turntable_batch_draws_one_picture_of_each_garment_and_no_two_pictures_are_the_same() {
    let Some(mut f) = fitted() else { return };
    if let Err(why) = crate::self_check::headless_bridge() {
        skip_corpus(WHAT, &why);
        return;
    }
    for turns in [1u32, 4] {
        let folder = f._dir.path().join(format!("pictures-{turns}"));
        f.state
            .start_catalogue(folder.clone(), turns)
            .expect("the pack has two garments with models");
        until(&mut f.state, "the catalogue", |s| {
            (s.workbench.wardrobe.dressed.catalogue.as_ref()).is_some_and(|c| c.is_finished())
        });
        let job = f.state.workbench.wardrobe.dressed.catalogue.take().unwrap();
        assert!(job.failed.is_empty(), "{:?}", job.failed);
        assert_eq!(job.written, 2 * turns as usize);

        let mut files: Vec<PathBuf> = std::fs::read_dir(&folder)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        files.sort();
        assert_eq!(
            files.len(),
            2 * turns as usize,
            "one picture of each garment for each turn: {files:?}"
        );
        let bytes: Vec<Vec<u8>> = files.iter().map(|p| std::fs::read(p).unwrap()).collect();
        let distinct: std::collections::HashSet<&Vec<u8>> = bytes.iter().collect();
        assert_eq!(distinct.len(), bytes.len(), "no two pictures are the same");
        for file in &files {
            let picture = image::open(file).expect("a picture").to_rgba8();
            let drawn = picture.pixels().filter(|p| p.0[3] > 0).count();
            assert!(drawn > 0, "{} drew nothing", file.display());
        }
    }
}

#[test]
fn each_preset_of_the_animation_is_a_clip_of_the_installs_default_movement_and_drives_the_whole_ped(
) {
    let Some(root) = game_roots().into_iter().next() else {
        skip_install(WHAT, "OPENRAGE_GAME_ROOT names no install");
        return;
    };
    let Some(mut f) = fitted() else { return };
    let mut with_keys = super::opening_a_ydd::with_keys(&root);
    std::mem::swap(&mut f.state.keys, &mut with_keys.keys);
    let game = crate::state::prefs::Game::detect_from_root(&root).expect("a game's install");
    f.state.prefs.game_paths.set(game, Some(root));
    f.state.prefs.active_game = Some(game);
    let jacket = f.jacket.clone();
    f.select(&jacket);
    until(&mut f.state, "the ped's own drawables", |s| {
        let d = &s.workbench.wardrobe.dressed;
        !d.is_reading() && d.skeleton(s.dressed_ped()).is_some()
    });

    for (at, preset) in super::workbench::PRESETS.iter().enumerate() {
        f.state.workbench.wardrobe.dressed.stop_clip();
        f.state.play_preset(at);
        until(&mut f.state, preset.label, |s| {
            let d = &s.workbench.wardrobe.dressed;
            !d.is_animating() || d.motion.is_some() || d.clip_note.is_some()
        });
        let dressed = &f.state.workbench.wardrobe.dressed;
        let motion = dressed
            .motion
            .as_ref()
            .unwrap_or_else(|| panic!("{} did not play: {:?}", preset.label, dressed.clip_note));
        assert!(motion.transport.playing && motion.transport.is_looping());
        assert!(
            motion.report.resolved > 0,
            "{} drives no bone of the ped",
            preset.label
        );
    }
}

#[test]
fn a_hat_that_states_the_hide_hair_scale_takes_the_peds_own_hair_off_and_says_so() {
    use or_wardrobe::behaviour::Behaviour;
    let Some(root) = game_roots().into_iter().next() else {
        skip_install(WHAT, "OPENRAGE_GAME_ROOT names no install");
        return;
    };
    let Some(models) = staged("ydd", 2) else {
        return;
    };
    let Some(mut f) = fitted_dropping(models, true) else {
        return;
    };
    let mut with_keys = super::opening_a_ydd::with_keys(&root);
    std::mem::swap(&mut f.state.keys, &mut with_keys.keys);
    let game = crate::state::prefs::Game::detect_from_root(&root).expect("a game's install");
    f.state.prefs.game_paths.set(game, Some(root));
    f.state.prefs.active_game = Some(game);
    let hat = (f.state.wardrobe_pack())
        .and_then(|p| p.read.as_ref().ok())
        .and_then(|read| {
            (read.wardrobe.garments.iter())
                .find(|g| g.slot == Some(or_wardrobe::Slot::Prop(or_wardrobe::Prop::Head)))
                .map(|g| g.id.clone())
        })
        .expect("the hat is a head prop");
    f.select(&hat);
    until(&mut f.state, "the ped with the hat", |s| {
        let d = &s.workbench.wardrobe.dressed;
        !d.is_reading() && d.scene().is_some_and(|scene| scene.parts.len() > 3)
    });
    let before = f.scene_parts().len();
    assert!(
        !f.state
            .workbench
            .wardrobe
            .dressed
            .notes
            .iter()
            .any(|n| n.contains("hair is off")),
        "a hat that states no scale leaves the hair"
    );

    f.state.edit_garment(
        hat,
        Edit::Behaviour(Behaviour::HairScale(Some(or_wardrobe::hair::HIDES))),
    );
    until(&mut f.state, "the hair off", |s| {
        let d = &s.workbench.wardrobe.dressed;
        !d.is_reading() && d.scene().is_some_and(|scene| scene.parts.len() < before)
    });
    assert_eq!(f.scene_parts().len(), before - 1, "only the hair went");
    let notes = f.state.workbench.wardrobe.dressed.notes.join(" | ");
    assert!(notes.contains("not verified in game"), "{notes}");
}

#[test]
fn a_slot_of_the_peds_own_taken_off_or_changed_is_drawn_so_and_the_game_dresses_it_again_when_asked(
) {
    use or_wardrobe::dress::BaseEdit;
    use or_wardrobe::Component;
    let Some(root) = game_roots().into_iter().next() else {
        skip_install(WHAT, "OPENRAGE_GAME_ROOT names no install");
        return;
    };
    let Some(mut f) = fitted() else { return };
    let mut with_keys = super::opening_a_ydd::with_keys(&root);
    std::mem::swap(&mut f.state.keys, &mut with_keys.keys);
    let game = crate::state::prefs::Game::detect_from_root(&root).expect("a game's install");
    f.state.prefs.game_paths.set(game, Some(root));
    f.state.prefs.active_game = Some(game);
    let jacket = f.jacket.clone();
    f.select(&jacket);
    until(&mut f.state, "the ped with the jacket", |s| {
        let d = &s.workbench.wardrobe.dressed;
        !d.is_reading() && d.scene().is_some_and(|scene| scene.parts.len() > 3)
    });
    let dressed = f.scene_parts().len();
    let ped = f.state.dressed_ped();
    let drawables = f
        .state
        .workbench
        .wardrobe
        .dressed
        .base_drawables(ped, Component::Hair);
    assert!(
        drawables > 1,
        "the ped has drawables of its own to choose from"
    );
    let drawn = |f: &Fitted| -> Vec<(usize, usize)> {
        (f.scene_parts().iter())
            .map(|p| (p.geometry[0].vertices.len(), p.geometry[0].indices.len()))
            .collect()
    };
    let as_the_game = drawn(&f);

    f.state
        .workbench
        .wardrobe
        .dressed
        .base_edits
        .insert(Component::Hair, BaseEdit::Off);
    until(&mut f.state, "the hair off", |s| {
        let d = &s.workbench.wardrobe.dressed;
        !d.is_reading() && d.scene().is_some_and(|scene| scene.parts.len() < dressed)
    });
    assert_eq!(f.scene_parts().len(), dressed - 1);

    f.state
        .workbench
        .wardrobe
        .dressed
        .base_edits
        .insert(Component::Hair, BaseEdit::Drawable(drawables as u32 - 1));
    until(&mut f.state, "another hair", |s| {
        let d = &s.workbench.wardrobe.dressed;
        !d.is_reading() && d.scene().is_some_and(|scene| scene.parts.len() == dressed)
    });
    assert_ne!(
        drawn(&f),
        as_the_game,
        "another drawable of the ped's own is drawn"
    );

    f.state.workbench.wardrobe.dressed.base_edits.clear();
    until(&mut f.state, "the game's own hair", |s| {
        let d = &s.workbench.wardrobe.dressed;
        !d.is_reading() && d.scene().is_some()
    });
    until(&mut f.state, "the ped as the game has it", |s| {
        let d = &s.workbench.wardrobe.dressed;
        !d.is_reading() && d.scene().is_some_and(|scene| scene.parts.len() == dressed)
    });
    assert_eq!(drawn(&f), as_the_game);
}

#[test]
fn a_head_of_the_peds_own_that_the_menu_offers_is_drawn_and_no_file_of_it_is_missing() {
    use or_wardrobe::dress::BaseEdit;
    use or_wardrobe::Component;
    let Some(root) = game_roots().into_iter().next() else {
        skip_install(WHAT, "OPENRAGE_GAME_ROOT names no install");
        return;
    };
    let Some(mut f) = fitted() else { return };
    let mut with_keys = super::opening_a_ydd::with_keys(&root);
    std::mem::swap(&mut f.state.keys, &mut with_keys.keys);
    let game = crate::state::prefs::Game::detect_from_root(&root).expect("a game's install");
    f.state.prefs.game_paths.set(game, Some(root));
    f.state.prefs.active_game = Some(game);
    let jacket = f.jacket.clone();
    f.select(&jacket);
    until(&mut f.state, "the ped with the jacket", |s| {
        s.dressed_is_settled()
            && s.workbench
                .wardrobe
                .dressed
                .scene()
                .is_some_and(|x| x.parts.len() > 3)
    });
    let dressed = f.scene_parts().len();
    let ped = f.state.dressed_ped();
    let heads = (f.state.workbench.wardrobe.dressed).base_drawables(ped, Component::Head);
    assert!(
        heads > 12,
        "the ped has heads beyond the base head of each tone"
    );
    // Heads that are not the first of a tone, which a white tone does not give the name of.
    for n in [2, 3, 5, 7, 11, heads as u32 - 1] {
        f.state
            .workbench
            .wardrobe
            .dressed
            .base_edits
            .insert(Component::Head, BaseEdit::Drawable(n));
        until(&mut f.state, "the head", |s| s.dressed_is_settled());
        let notes = f.state.workbench.wardrobe.dressed.notes.join(" | ");
        assert!(
            !notes.contains("is not in the install"),
            "head {n}: {notes}"
        );
        assert_eq!(f.scene_parts().len(), dressed, "head {n} is drawn: {notes}");
    }
}
