//! A click in the viewport selects the geometry under it in the Outline, `F` frames it and `Home`
//! frames everything, and a restart keeps the shading mode; the viewport draws each level of detail
//! and a drawable's bound on request, and its overlays, tiles and framing show what they say.

use std::path::PathBuf;

use glam::{Mat4, Vec3};
use or_io::gta5::{GtaDrawable, PgDictionary, RmcDrawable};
use or_renderer::{OrbitCamera, ShadingMode};
use or_test_support::{skip_corpus, Corpus, TARGET_GEN8, TARGET_GEN9};

use super::prefs::{load_prefs_from, save_prefs_to, Prefs};
use super::{AppState, DocPayload, DocumentId};
use crate::render::meshes::{drawable_parts, Material, MeshUpload};
use crate::render::viewport::pick::{select_under, Ray};
use crate::render::viewport::CameraInputs;
use crate::ui::commands::Action;
use crate::ui::panels::selection_ids::{
    dict_entry_id, dict_entry_part_id, pack_lod_sub_index, pack_selection, YPT_MODEL_KIND_LOD,
};
use crate::ui::shortcuts::{action_for, ShortcutMods};
use crate::ui::theme;

/// How far from the face it looks at the camera stands, beyond the near plane's 0.05.
const AWAY: f32 = 0.1;

/// A geometry the viewport draws, by its model's and its own index, and its mesh.
type Part = ((usize, usize), MeshUpload);

/// The first staged file of `extension` and `target` that `drawn` finds something in.
pub(super) fn staged<T>(
    extension: &str,
    target: &str,
    drawn: impl Fn(&or_io::Asset) -> Option<T>,
) -> Option<(PathBuf, or_io::Asset, T)> {
    let files = Corpus::new(env!("CARGO_MANIFEST_DIR"), extension)
        .target(target)
        .find();
    files.into_iter().find_map(|path| {
        let asset = or_io::open(&path).ok()?;
        let found = drawn(&asset)?;
        Some((path, asset, found))
    })
}

/// What a drawable's first level of detail draws, where that is two geometries or more.
pub(super) fn two_or_more(asset: &or_io::Asset) -> Option<Vec<Part>> {
    let parts = match asset {
        or_io::Asset::Drawable(d) => drawable_parts(&d.base, 0, None),
        or_io::Asset::DrawableGen9(d) => drawable_parts(&d.base, 0, None),
        _ => return None,
    };
    (parts.len() >= 2).then_some(parts)
}

/// A dictionary's first entry with a face in sight, by its hash, and what it draws.
fn an_entry<T: Material>(dict: &PgDictionary<GtaDrawable<T>>) -> Option<(u32, Vec<Part>)> {
    (dict.codes.iter().zip(&dict.entries)).find_map(|(code, entry)| {
        let parts = drawable_parts(&entry.as_deref()?.base, 0, None);
        the_last_in_sight(&parts).map(|_| (*code, parts))
    })
}

fn triangles(upload: &MeshUpload) -> Vec<[Vec3; 3]> {
    let corner = |i: u32| Vec3::from(upload.vertices[i as usize].position);
    (upload.indices.as_chunks::<3>().0.iter())
        .map(|t| [corner(t[0]), corner(t[1]), corner(t[2])])
        .collect()
}

/// Whether the segment `p q` passes through the triangle, by the signs of the volumes they span.
fn crosses(p: Vec3, q: Vec3, [a, b, c]: [Vec3; 3]) -> bool {
    let volume = |w: Vec3, x: Vec3, y: Vec3, z: Vec3| (x - w).cross(y - w).dot(z - w);
    if volume(a, b, c, p).signum() == volume(a, b, c, q).signum() {
        return false;
    }
    let sides = [volume(p, q, a, b), volume(p, q, b, c), volume(p, q, c, a)];
    sides.iter().all(|s| *s >= 0.0) || sides.iter().all(|s| *s <= 0.0)
}

/// The centre of one of `target`'s faces, and the side of it a camera `AWAY` sees it from with no
/// other geometry between.
fn a_face_in_sight(parts: &[Part], target: (usize, usize)) -> Option<(Vec3, Vec3)> {
    let (_, upload) = parts.iter().find(|(part, _)| *part == target)?;
    let others: Vec<[Vec3; 3]> = (parts.iter())
        .filter(|(part, _)| *part != target)
        .flat_map(|(_, upload)| triangles(upload))
        .collect();
    let area = |[a, b, c]: &[Vec3; 3]| (*b - *a).cross(*c - *a).length();
    // ⚠ A camera looking straight down the world's up has no view matrix, so it leans.
    let lean = |side: Vec3| match side.z.abs() > 0.99 {
        true => (side + Vec3::X * 0.3).normalize(),
        false => side,
    };
    let mut faces = triangles(upload);
    faces.sort_by(|x, y| area(y).total_cmp(&area(x)));
    faces.into_iter().find_map(|face| {
        let normal = (face[1] - face[0])
            .cross(face[2] - face[0])
            .try_normalize()?;
        let centre = (face[0] + face[1] + face[2]) / 3.0;
        [lean(normal), lean(-normal)]
            .into_iter()
            .find(|side| {
                let (eye, short) = (centre + *side * AWAY, centre + *side * 1e-3);
                !others.iter().any(|other| crosses(eye, short, *other))
            })
            .map(|side| (centre, side))
    })
}

/// The last geometry drawn, and one of its faces in sight.
pub(super) fn the_last_in_sight(parts: &[Part]) -> Option<((usize, usize), Vec3, Vec3)> {
    let target = parts.last()?.0;
    let (centre, normal) = a_face_in_sight(parts, target)?;
    Some((target, centre, normal))
}

/// A click at the middle of the viewport, drawn through a camera `AWAY` from `at` towards `eye`.
fn click(state: &mut AppState, at: Vec3, eye: Vec3) {
    let camera = OrbitCamera {
        target: at,
        yaw: eye.y.atan2(eye.x),
        pitch: eye.z.asin(),
        distance: AWAY,
        ..OrbitCamera::default()
    };
    let rect = egui::Rect::from_min_size(egui::pos2(300.0, 40.0), egui::vec2(800.0, 500.0));
    let ray = Ray::at(&camera, rect, (1200, 750), rect.center()).expect("a ray");
    select_under(state, &ray);
}

/// The centre of the box the drawable ships for one geometry.
fn box_centre<T: Material>(
    drawable: &or_io::gta5::RmcDrawable<T>,
    (model, geometry): (usize, usize),
) -> Vec3 {
    let model = &drawable.lod_group.lods[0].models[model];
    let slot = or_io::codec::mesh::aabb_slot_of(model.geometries.len(), geometry).expect("a box");
    let b = &model.aabbs[slot];
    (Vec3::new(b.min.x, b.min.y, b.min.z) + Vec3::new(b.max.x, b.max.y, b.max.z)) * 0.5
}

/// Whether the Outline, drawn too short to show every row unscrolled, shows a row labelled `label`
/// once it has settled.
pub(super) fn the_outline_shows(state: &mut AppState, label: &str) -> bool {
    the_outline_settles_with(state, |screen, output| {
        let update = output.platform_output.accesskit_update?;
        Some(update.nodes.iter().any(|(_, node)| {
            let centre = node
                .bounds()
                .map(|b| egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32));
            node.label().is_some_and(|text| text.contains(label))
                && centre.is_some_and(|at| screen.contains(at))
        }))
    })
}

/// Whether the Outline, settled as above, shows a row labelled `label` in the colour of a selected
/// row; egui does not tell a screen reader which row of a list is selected.
pub(super) fn the_outline_selects(state: &mut AppState, label: &str) -> bool {
    // The palette it compares with is the process's: a theme test must not flip it between the draw and the read
    let _serial = crate::ui::theme::serial();
    /// The colour of the last section of the text `shape` paints that holds `label`, which is the
    /// row's label after its icon, where the text is on the screen.
    fn colour_of(shape: &egui::Shape, label: &str, screen: egui::Rect) -> Option<egui::Color32> {
        match shape {
            egui::Shape::Vec(shapes) => shapes
                .iter()
                .find_map(|shape| colour_of(shape, label, screen)),
            egui::Shape::Text(text)
                if text.galley.text().contains(label) && screen.contains(text.pos) =>
            {
                text.galley.job.sections.last().map(|s| s.format.color)
            }
            _ => None,
        }
    }
    the_outline_settles_with(state, |screen, output| {
        let colour = (output.shapes.iter()).find_map(|c| colour_of(&c.shape, label, screen));
        Some(colour == Some(theme::ACCENT_TEXT.get()))
    })
}

/// Draws the Outline for forty frames on a screen too short to show every row unscrolled, and
/// answers what `judge` said of the last frame it judged.
fn the_outline_settles_with(
    state: &mut AppState,
    mut judge: impl FnMut(egui::Rect, egui::FullOutput) -> Option<bool>,
) -> bool {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 70.0));
    let mut shown = false;
    for frame in 0..40 {
        let input = egui::RawInput {
            screen_rect: Some(screen),
            time: Some(f64::from(frame) / 30.0),
            ..Default::default()
        };
        let output = ctx.run_ui(input, |ui| {
            crate::ui::panels::hierarchy::draw_outline(ui, state)
        });
        if let Some(judged) = judge(screen, output) {
            shown = judged;
        }
    }
    shown
}

pub(super) fn opened(path: &std::path::Path, asset: or_io::Asset) -> (AppState, DocumentId) {
    let mut state = AppState::from_prefs(Prefs::default());
    let doc = state.insert_document(path.to_path_buf(), DocPayload::Resource(asset), None, None);
    (state, doc)
}

fn entry_geometry(hash: u32, (model, geometry): (usize, usize)) -> u64 {
    let sub = pack_lod_sub_index(0, model as u8, Some(geometry as u16));
    dict_entry_part_id(hash, YPT_MODEL_KIND_LOD, sub)
}

#[test]
fn a_click_selects_the_geometry_under_the_cursor_in_the_outline_f_frames_it_and_a_restart_keeps_the_mode(
) {
    const WHAT: &str = "a_click_selects_the_geometry_under_the_cursor_in_the_outline_f_frames_it_and_a_restart_keeps_the_mode";
    for target in [TARGET_GEN8, TARGET_GEN9] {
        let Some((path, asset, parts)) = staged("ydr", target, two_or_more) else {
            skip_corpus(WHAT, format_args!("no {target} .ydr draws two geometries"));
            continue;
        };
        let ((model, geometry), centre, normal) = the_last_in_sight(&parts)
            .unwrap_or_else(|| panic!("{}: no face of its last geometry in sight", path.display()));
        let its_box = match &asset {
            or_io::Asset::Drawable(d) => box_centre(&d.base, (model, geometry)),
            or_io::Asset::DrawableGen9(d) => box_centre(&d.base, (model, geometry)),
            _ => unreachable!("staged as a drawable"),
        };
        let (mut state, doc) = opened(&path, asset);
        let all = state.step_viewport_camera(CameraInputs::default()).target;

        click(&mut state, centre, normal);
        let geometry_row = pack_selection(0, model as u16, Some(geometry as u32));
        assert_eq!(
            state.selection.selected(doc),
            Some(geometry_row),
            "{}: a click on geometry {geometry} of model {model} selected another row",
            path.display()
        );
        assert!(
            the_outline_shows(&mut state, &format!("Geometry {geometry} \u{2022}")),
            "{}: the Outline does not show the row of geometry {geometry}",
            path.display()
        );

        let key = |key| action_for(ShortcutMods::none(), key, &state.prefs);
        assert_eq!(key(egui::Key::F), Some(Action::FrameSelection));
        assert_eq!(key(egui::Key::Home), Some(Action::FrameAll));
        let framed = state
            .step_viewport_camera(CameraInputs {
                frame_selection: true,
                ..CameraInputs::default()
            })
            .target;
        assert_eq!(
            framed,
            its_box,
            "{}: F framed something other than the geometry clicked",
            path.display()
        );
        assert_ne!(framed, all, "{}: F framed everything", path.display());
    }

    let mut state = AppState::from_prefs(Prefs::default());
    state.remember_shading_mode(ShadingMode::Shaded);
    let dir = tempfile::tempdir().expect("a scratch folder");
    let file = dir.path().join("prefs.json");
    save_prefs_to(&file, &state.prefs);
    let restarted = AppState::from_prefs(load_prefs_from(&file));
    assert_eq!(restarted.prefs.viewport.shading, ShadingMode::Shaded);
}

#[test]
fn a_click_inside_a_dictionary_entry_opens_it_in_the_outline_and_a_click_on_nothing_keeps_only_what_is_drawn(
) {
    const WHAT: &str = "a_click_inside_a_dictionary_entry_opens_it_in_the_outline_and_a_click_on_nothing_keeps_only_what_is_drawn";
    let entry = |asset: &or_io::Asset| match asset {
        or_io::Asset::DrawableDict(dict) => an_entry(dict),
        or_io::Asset::DrawableDictGen9(dict) => an_entry(dict),
        _ => None,
    };
    for target in [TARGET_GEN8, TARGET_GEN9] {
        let Some((path, asset, (hash, parts))) = staged("ydd", target, entry) else {
            skip_corpus(
                WHAT,
                format_args!("no {target} .ydd entry draws a geometry"),
            );
            continue;
        };
        let (part, centre, normal) = the_last_in_sight(&parts)
            .unwrap_or_else(|| panic!("{}: no face of its entry in sight", path.display()));
        let (mut state, doc) = opened(&path, asset);
        state.selection.select(doc, dict_entry_id(hash));

        click(&mut state, centre, normal);
        assert_eq!(
            state.selection.selected(doc),
            Some(entry_geometry(hash, part)),
            "{}: the click did not select the geometry under it",
            path.display()
        );
        assert!(
            the_outline_shows(&mut state, &format!("Geometry {} \u{2022}", part.1)),
            "{}: the Outline does not show the entry's geometry",
            path.display()
        );

        click(&mut state, centre + normal * 100.0, -normal);
        assert_eq!(
            state.selection.selected(doc),
            Some(dict_entry_id(hash)),
            "{}: a click on nothing did not keep the entry the viewport draws",
            path.display()
        );
    }

    let Some((path, asset, parts)) = staged("ydr", TARGET_GEN8, two_or_more) else {
        skip_corpus(WHAT, "no Legacy .ydr draws two geometries");
        return;
    };
    let (part, centre, normal) = the_last_in_sight(&parts).expect("a face in sight");
    let (mut state, doc) = opened(&path, asset);
    click(&mut state, centre, normal);
    assert_eq!(
        state.selection.selected(doc),
        Some(pack_selection(0, part.0 as u16, Some(part.1 as u32)))
    );
    click(&mut state, centre + normal * 100.0, -normal);
    assert_eq!(
        state.selection.selected(doc),
        None,
        "{}: a click on nothing kept the geometry selected",
        path.display()
    );
}

/// Each level of detail a drawable draws, with the distance the game draws it to and its meshes.
fn levels_drawn<T: Material>(drawable: &RmcDrawable<T>) -> Vec<(usize, f32, Vec<Part>)> {
    (0..drawable.lod_group.lods.len())
        .map(|level| {
            let distance = drawable.lod_group.lod_distances[level];
            (level, distance, drawable_parts(drawable, level, None))
        })
        .filter(|(_, _, parts)| !parts.is_empty())
        .collect()
}

/// Whether the viewport's row of controls, drawn at the default window size, shows `text`.
pub(super) fn the_row_shows(state: &mut AppState, text: &str) -> bool {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    (the_row(&ctx, state, Vec::new()).iter()).any(|(shown, _)| shown.contains(text))
}

/// One frame of the viewport's row of controls at the default window size, given `events`: what
/// it shows, and where.
fn the_row(
    ctx: &egui::Context,
    state: &mut AppState,
    events: Vec<egui::Event>,
) -> Vec<(String, egui::Pos2)> {
    let rect = egui::Rect::from_min_size(egui::pos2(300.0, 40.0), egui::vec2(800.0, 500.0));
    let input = egui::RawInput {
        events,
        ..Default::default()
    };
    let output = ctx.run_ui(input, |ui| {
        crate::ui::panels::viewport_drawable_row::draw(ui, rect, rect.top() + 36.0, state)
    });
    let nodes = output.platform_output.accesskit_update.map(|u| u.nodes);
    (nodes.unwrap_or_default().iter())
        .filter_map(|(_, node)| {
            let shown = node.value().or(node.label())?;
            let b = node.bounds()?;
            let at = egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32);
            Some((shown.to_string(), at))
        })
        .collect()
}

/// Click the control the viewport's row labels `label`, as a pointer does; whether there is one.
pub(super) fn click_in_the_row(state: &mut AppState, label: &str) -> bool {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let shown = the_row(&ctx, state, Vec::new());
    let Some(at) = (shown.into_iter()).find_map(|(text, at)| text.contains(label).then_some(at))
    else {
        return false;
    };
    let press = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    the_row(
        &ctx,
        state,
        vec![egui::Event::PointerMoved(at), press(true)],
    );
    the_row(&ctx, state, vec![press(false)]);
    true
}

#[test]
fn a_collision_toggle_draws_a_drawables_embedded_bound() {
    use crate::render::lines::{build_bound_lines, document_lines};
    use crate::render::viewport::LinesCacheKey;
    const WHAT: &str = "a_collision_toggle_draws_a_drawables_embedded_bound";
    let bound = |asset: &or_io::Asset| {
        let bound = match asset {
            or_io::Asset::Drawable(d) => d.bound.as_deref(),
            or_io::Asset::DrawableGen9(d) => d.bound.as_deref(),
            _ => None,
        }?;
        Some(build_bound_lines(bound).len()).filter(|lines| *lines > 0)
    };
    for target in [TARGET_GEN8, TARGET_GEN9] {
        let Some((path, asset, bound)) = staged("ydr", target, bound) else {
            skip_corpus(WHAT, format_args!("no {target} .ydr embeds a bound"));
            continue;
        };
        let (mut state, doc) = opened(&path, asset);
        let wireframe = document_lines(&state, true).len();
        assert!(document_lines(&state, false).is_empty());
        let hidden = LinesCacheKey::of(&state, doc);

        assert!(
            click_in_the_row(&mut state, "Collision"),
            "{}: the viewport offers no collision toggle",
            path.display()
        );
        assert_eq!(
            document_lines(&state, false).len(),
            bound,
            "{}: the toggle did not draw the embedded bound",
            path.display()
        );
        assert_eq!(document_lines(&state, true).len(), wireframe + bound);
        assert_ne!(
            LinesCacheKey::of(&state, doc),
            hidden,
            "{}: the toggle does not build the lines again",
            path.display()
        );
    }
}

#[test]
fn a_level_of_detail_switch_draws_each_level_and_shows_its_distance() {
    const WHAT: &str = "a_level_of_detail_switch_draws_each_level_and_shows_its_distance";
    let two_levels = |asset: &or_io::Asset| {
        let levels = match asset {
            or_io::Asset::Drawable(d) => levels_drawn(&d.base),
            or_io::Asset::DrawableGen9(d) => levels_drawn(&d.base),
            or_io::Asset::Fragment(f) => levels_drawn(&f.common_drawable.as_deref()?.base),
            or_io::Asset::FragmentGen9(f) => levels_drawn(&f.common_drawable.as_deref()?.base),
            _ => return None,
        };
        (levels.len() >= 2).then_some(levels)
    };
    let positions = |uploads: &mut dyn Iterator<Item = &MeshUpload>| -> Vec<[f32; 3]> {
        uploads
            .flat_map(|u| u.vertices.iter().map(|v| v.position))
            .collect()
    };
    for target in [TARGET_GEN8, TARGET_GEN9] {
        let staged = ["ydr", "yft"]
            .into_iter()
            .find_map(|extension| staged(extension, target, two_levels));
        let Some((path, asset, levels)) = staged else {
            skip_corpus(
                WHAT,
                format_args!("no {target} drawable draws two levels of detail"),
            );
            continue;
        };
        // How the Outline names a geometry of a level: in a drawable's tree, or a fragment's.
        let row: fn(usize, (usize, usize)) -> u64 = match &asset {
            or_io::Asset::Drawable(_) | or_io::Asset::DrawableGen9(_) => {
                |level, (m, g)| pack_selection(level as u8, m as u16, Some(g as u32))
            }
            _ => |level, (m, g)| {
                let sub = pack_lod_sub_index(level as u8, m as u8, Some(g as u16));
                let hash = crate::state::textures::FRAGMENT_ENTRY_HASH;
                dict_entry_part_id(hash, YPT_MODEL_KIND_LOD, sub)
            },
        };
        let (mut state, doc) = opened(&path, asset);
        let (mut meshes, mut wireframes) = (Vec::new(), Vec::new());
        for (level, distance, parts) in &levels {
            state.preview_scene.lod = *level;
            let drawn = crate::render::meshes::build_meshes_for_active_document(&state);
            // The body's meshes come first; a vehicle's wheels follow, which no click selects
            assert!(
                positions(&mut drawn.iter().take(parts.len()))
                    == positions(&mut parts.iter().map(|(_, u)| u)),
                "{}: level {level} drew another level's meshes",
                path.display()
            );
            assert!(
                the_row_shows(&mut state, &format!("to {distance:.0} m")),
                "{}: the switch does not show level {level}'s distance, {distance} m",
                path.display()
            );
            let (part, centre, side) = the_last_in_sight(parts).expect("a face in sight");
            click(&mut state, centre, side);
            assert_eq!(
                state.selection.selected(doc),
                Some(row(*level, part)),
                "{}: a click on level {level} selected a row of another level",
                path.display()
            );
            let vertices = parts
                .iter()
                .find(|(p, _)| *p == part)
                .map(|(_, u)| u.vertices.len());
            let label = format!(
                "Geometry {} \u{2022} {} verts",
                part.1,
                vertices.unwrap_or(0)
            );
            assert!(
                the_outline_shows(&mut state, &label),
                "{}: the Outline does not show level {level}'s geometry",
                path.display()
            );
            meshes.push(format!(
                "{:?}",
                crate::render::viewport::MeshCacheKey::of(&state, doc, 0)
            ));
            wireframes.push(format!(
                "{:?}",
                crate::render::viewport::LinesCacheKey::of(&state, doc)
            ));
        }
        for (what, mut keys) in [("meshes", meshes), ("wireframe", wireframes)] {
            keys.sort();
            keys.dedup();
            assert_eq!(
                keys.len(),
                levels.len(),
                "{}: switching level does not build the {what} again",
                path.display()
            );
        }
    }
}

/// Every bone moved straight up and far, so no ray towards a face where the bind pose put it meets
/// the moved mesh.
const MOVED: Vec3 = Vec3::new(0.0, 0.0, 1000.0);

fn posed() -> Vec<Mat4> {
    vec![Mat4::from_translation(MOVED); 256]
}

/// A dictionary's first entry with a skinned geometry in sight once posed: its hash, what it
/// draws posed, that geometry, and its face.
fn a_posed_entry<T: Material>(
    dict: &PgDictionary<GtaDrawable<T>>,
) -> Option<(u32, (usize, usize), Vec3, Vec3)> {
    (dict.codes.iter().zip(&dict.entries)).find_map(|(code, entry)| {
        let mut parts = drawable_parts(&entry.as_deref()?.base, 0, None);
        for (_, upload) in &mut parts {
            upload.apply_pose(&posed(), &mut Vec::new(), &mut Vec::new());
        }
        let (part, (centre, side)) = (parts.iter().rev())
            .filter(|(_, upload)| upload.is_skinned())
            .find_map(|(part, _)| Some((*part, a_face_in_sight(&parts, *part)?)))?;
        Some((*code, part, centre, side))
    })
}

#[test]
fn a_click_lands_on_a_skinned_mesh_where_its_pose_draws_it() {
    const WHAT: &str = "a_click_lands_on_a_skinned_mesh_where_its_pose_draws_it";
    let skinned = |asset: &or_io::Asset| match asset {
        or_io::Asset::DrawableDict(dict) => a_posed_entry(dict),
        or_io::Asset::DrawableDictGen9(dict) => a_posed_entry(dict),
        _ => None,
    };
    for target in [TARGET_GEN8, TARGET_GEN9] {
        let Some((path, asset, (hash, part, centre, side))) = staged("ydd", target, skinned) else {
            skip_corpus(
                WHAT,
                format_args!("no {target} .ydd entry draws a skinned mesh"),
            );
            continue;
        };
        let (mut state, doc) = opened(&path, asset);
        state.selection.select(doc, dict_entry_id(hash));
        state.anim.skin_matrices = posed();

        click(&mut state, centre, side);
        assert_eq!(
            state.selection.selected(doc),
            Some(entry_geometry(hash, part)),
            "{}: a click where the pose draws the mesh missed it",
            path.display()
        );
        click(&mut state, centre - MOVED, side);
        assert_eq!(
            state.selection.selected(doc),
            Some(dict_entry_id(hash)),
            "{}: a click where the bind pose would put the mesh found it",
            path.display()
        );
    }
}

#[test]
fn material_is_the_mode_of_a_preview_never_set() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let file = dir.path().join("prefs.json");
    for (json, mode) in [
        (serde_json::json!({}), ShadingMode::Material),
        (
            serde_json::json!({ "viewport": { "show_grid": false } }),
            ShadingMode::Material,
        ),
        (
            serde_json::json!({ "viewport": { "shading": "solid" } }),
            ShadingMode::Solid,
        ),
    ] {
        std::fs::write(&file, json.to_string()).expect("prefs.json written");
        let state = AppState::from_prefs(load_prefs_from(&file));
        assert_eq!(state.prefs.viewport.shading, mode, "{json}");
    }
}

#[test]
fn a_dictionary_opens_on_its_first_entry() {
    const WHAT: &str = "a_dictionary_opens_on_its_first_entry";
    let first = |asset: &or_io::Asset| {
        fn first<T: Material>(dict: &PgDictionary<GtaDrawable<T>>) -> Option<u32> {
            (dict.codes.iter().zip(&dict.entries))
                .find_map(|(code, entry)| entry.as_ref().map(|_| *code))
        }
        match asset {
            or_io::Asset::DrawableDict(dict) => first(dict),
            or_io::Asset::DrawableDictGen9(dict) => first(dict),
            _ => None,
        }
    };
    for target in [TARGET_GEN8, TARGET_GEN9] {
        let Some((path, _, hash)) = staged("ydd", target, first) else {
            skip_corpus(WHAT, format_args!("no {target} .ydd holds an entry"));
            continue;
        };
        let mut state = AppState::from_prefs(Prefs::default());
        // ⚠ Of the other edition, a file waits on the question whether to switch to it.
        state.prefs.active_game = Some(match target {
            TARGET_GEN9 => super::prefs::Game::GtaVEnhanced,
            _ => super::prefs::Game::GtaVLegacy,
        });
        state.load_file(path.clone());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            state.poll_load_result();
            if state.load_receiver.is_none() && state.active_document.is_some() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the open never finished"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let doc = state.active_document.expect("the dictionary is open");
        assert_eq!(
            state.selection.selected(doc),
            Some(dict_entry_id(hash)),
            "{}: the dictionary did not open on its first entry",
            path.display()
        );
        assert!(
            !crate::render::meshes::build_meshes_for_active_document(&state).is_empty(),
            "{}: the viewport draws nothing of the dictionary just opened",
            path.display()
        );
    }
}

/// Every rectangle a frame painted or laid a labelled widget out in.
fn drawn_rects(output: &egui::FullOutput) -> Vec<(String, egui::Rect)> {
    let painted = (output.shapes.iter()).filter_map(|clipped| match &clipped.shape {
        egui::Shape::Rect(shape) => Some(("a box".to_string(), shape.rect)),
        egui::Shape::Text(text) => Some((
            text.galley.text().to_string(),
            text.galley.rect.translate(text.pos.to_vec2()),
        )),
        _ => None,
    });
    let nodes = output.platform_output.accesskit_update.as_ref();
    let widgets =
        (nodes.map(|u| u.nodes.as_slice()).unwrap_or_default().iter()).filter_map(|(_, node)| {
            let b = node.bounds()?;
            let rect = egui::Rect::from_min_max(
                egui::pos2(b.x0 as f32, b.y0 as f32),
                egui::pos2(b.x1 as f32, b.y1 as f32),
            );
            Some((node.label().unwrap_or("a widget").to_string(), rect))
        });
    painted.chain(widgets).collect()
}

#[test]
fn the_overlays_leave_the_axis_gizmo_clear_and_fit_a_narrow_viewport() {
    use crate::render::debug_channel::DebugChannel;
    use crate::ui::panels::viewport::{
        draw_channel_selector, draw_shaded_hud_bar, draw_viewport_overlay, mode_bar_height,
    };
    use crate::ui::panels::viewport_layout::column;
    let mut state = AppState::from_prefs(Prefs::default());
    let offered = [DebugChannel::Normal];
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 900.0));
    let sizes = [
        (800.0, 500.0),
        (220.0, 500.0),
        (360.0, 230.0),
        (220.0, 300.0),
    ];
    for size in sizes {
        let rect = egui::Rect::from_min_size(egui::pos2(300.0, 40.0), size.into());
        for mode in [ShadingMode::Shaded, ShadingMode::Channel] {
            let col = column(rect, 28.0, mode_bar_height(mode, rect, true));
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let input = egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            };
            let output = ctx.run_ui(input, |ui| {
                match mode {
                    ShadingMode::Shaded => draw_shaded_hud_bar(ui, rect, col.bar, &mut state),
                    _ => draw_channel_selector(ui, rect, col.bar, &offered, &mut state),
                }
                draw_viewport_overlay(ui, rect, &mut state, None, None, col.gizmo);
            });
            assert!(
                rect.contains_rect(col.gizmo),
                "{mode:?} in {size:?}: the gizmo leaves the viewport"
            );
            for (what, drawn) in drawn_rects(&output) {
                let shared = drawn.intersect(col.gizmo);
                assert!(
                    shared.width() <= 0.01 || shared.height() <= 0.01,
                    "{mode:?} in {size:?}: `{what}` at {drawn:?} covers the gizmo at {:?}",
                    col.gizmo
                );
                assert!(
                    rect.expand(0.01).contains_rect(drawn),
                    "{mode:?} in {size:?}: `{what}` at {drawn:?} leaves the viewport"
                );
            }
        }
    }
}

#[test]
fn the_overlay_says_what_the_viewport_shows_in_words() {
    use crate::ui::panels::viewport_layout::overlay_text;
    let mut state = AppState::from_prefs(Prefs::default());
    assert_eq!(
        overlay_text(&state, None).lines().next(),
        Some("Nothing open")
    );
    let drawable = or_io::Asset::Drawable(Default::default());
    state.insert_document(
        PathBuf::from("C:/mods/bench.ydr"),
        DocPayload::Resource(drawable),
        None,
        None,
    );
    let text = overlay_text(&state, None);
    assert!(text.starts_with("bench.ydr · Drawable\n"), "{text}");
    for code in ["Doc:", "YDR", "fps"] {
        assert!(!text.contains(code), "`{code}` in {text}");
    }
}

#[test]
fn a_gallery_tiles_key_holds_the_mode_and_the_channel() {
    use crate::render::debug_channel::DebugChannel;
    use crate::render::thumbnails::ThumbKey;
    use crate::ui::panels::selection_ids::DOC_ROOT_SEL;
    let mut state = AppState::from_prefs(Prefs::default());
    let drawable = or_io::Asset::Drawable(Default::default());
    let doc = state.insert_document(
        PathBuf::from("C:/mods/bench.ydr"),
        DocPayload::Resource(drawable),
        None,
        None,
    );
    let key = |state: &AppState, mode| {
        let document = state.document(doc).expect("open");
        ThumbKey::of(document, DOC_ROOT_SEL).drawn_by(state, mode)
    };
    let solid = key(&state, ShadingMode::Solid);
    assert_ne!(solid, key(&state, ShadingMode::Material), "the mode");
    assert_eq!(
        solid,
        key(&state, ShadingMode::Wireframe),
        "a wireframe viewport's tiles draw solid"
    );
    state.preview_scene.debug_channel = Some(DebugChannel::Normal);
    let normals = key(&state, ShadingMode::Channel);
    state.preview_scene.debug_channel = Some(DebugChannel::Texcoord(0));
    assert_ne!(normals, key(&state, ShadingMode::Channel), "the channel");
    let high = key(&state, ShadingMode::Channel);
    state.preview_scene.lod = 1;
    assert_ne!(
        high,
        key(&state, ShadingMode::Channel),
        "the level of detail"
    );
}

#[test]
fn a_particle_model_frames_from_a_sphere_that_holds_it() {
    use crate::ui::panels::selection_ids::ypt_model_id;
    const WHAT: &str = "a_particle_model_frames_from_a_sphere_that_holds_it";
    let models = |asset: &or_io::Asset| -> Option<Vec<u32>> {
        let hashes: Vec<u32> = match asset {
            or_io::Asset::Particles(list) => {
                or_particle::list::models(list).map(|(h, _)| h).collect()
            }
            or_io::Asset::ParticlesGen9(list) => {
                or_particle::list::models(list).map(|(h, _)| h).collect()
            }
            _ => return None,
        };
        (!hashes.is_empty()).then_some(hashes)
    };
    for target in [TARGET_GEN8, TARGET_GEN9] {
        let Some((path, asset, hashes)) = staged("ypt", target, models) else {
            skip_corpus(WHAT, format_args!("no {target} .ypt holds a model"));
            continue;
        };
        let (mut state, doc) = opened(&path, asset);
        state.active_workspace = crate::ui::dock::Workspace::Model;
        for hash in hashes {
            state.selection.select(doc, ypt_model_id(hash));
            let drawn = crate::render::meshes::build_meshes_for_active_document(&state);
            let Some((centre, radius)) = crate::render::viewport::compute_active_doc_bounds(&state)
            else {
                panic!("{}: model {hash:08x} frames from nothing", path.display());
            };
            for vertex in drawn.iter().flat_map(|upload| &upload.vertices) {
                let away = Vec3::from(vertex.position).distance(centre);
                assert!(
                    away <= radius * 1.0001 + 1e-4,
                    "{}: model {hash:08x} has a vertex {away} from the centre of a sphere of {radius}",
                    path.display()
                );
            }
        }
    }
}

/// How many rows of the Outline, drawn on a screen tall enough to show every one, are labelled `label`.
pub(super) fn the_outline_rows_labelled(state: &mut AppState, label: &str) -> usize {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 4000.0));
    let mut rows = 0;
    for frame in 0..6 {
        let input = egui::RawInput {
            screen_rect: Some(screen),
            time: Some(f64::from(frame) / 30.0),
            ..Default::default()
        };
        let output = ctx.run_ui(input, |ui| {
            crate::ui::panels::hierarchy::draw_outline(ui, state)
        });
        let nodes = output.platform_output.accesskit_update.map(|u| u.nodes);
        rows = (nodes.into_iter().flatten())
            .filter(|(_, node)| node.label().is_some_and(|text| text.contains(label)))
            .count();
    }
    rows
}
