//! Each open document's viewport camera, so a switch away and back finds the
//! view where it was left.

use std::collections::HashMap;

use or_renderer::OrbitCamera;

use super::{AppState, DocumentId};
use crate::render::viewport::{
    compute_active_doc_bounds, compute_selection_bounds, selected_dictionary_entry, CameraInputs,
};

// ⚠ Keyed by document and never by workspace. A workspace is a layout preset
// over the one document list, so a camera per workspace would give one
// document several views and still lose them all on a document switch.
//
// ⚠ Session-only, deliberately. A `DocumentId` names nothing after a restart,
// and `docs/direction/Project_Context.md` records why `prefs.json` is not the place to
// key one by path instead.
#[derive(Debug, Default)]
pub struct DocumentCameras {
    views: HashMap<DocumentId, View>,
    /// What the viewport shows with no document active: the grid on its own.
    idle: OrbitCamera,
    /// What the World screen's flying camera sees, as the viewport renders through it.
    world: OrbitCamera,
}

#[derive(Debug)]
struct View {
    camera: OrbitCamera,
    /// The dictionary entry the camera was last framed on; choosing another reframes.
    framed_entry: Option<u32>,
}

impl AppState {
    /// Apply one frame of viewport input and return the camera to draw through.
    ///
    /// A document seen for the first time is framed on its own bounds; after
    /// that its camera moves only by input, by a frame key, or by choosing
    /// another dictionary entry inside it.
    pub fn step_viewport_camera(&mut self, inputs: CameraInputs) -> &OrbitCamera {
        // The Wardrobe dresses a ped that is no document, with a camera of its own.
        if self.active_workspace == crate::ui::dock::Workspace::Wardrobe {
            return self.step_dressed_camera(inputs);
        }
        // So does the Weapons screen, which assembles a weapon that is no document.
        if self.active_workspace == crate::ui::dock::Workspace::Weapons {
            return self.step_weapon_camera(inputs);
        }
        // The World screen flies: its camera is the viewer's, moved by its own panel.
        if self.active_workspace == crate::ui::dock::Workspace::World {
            self.cameras.world = super::gta6_world::worldview::viewer().camera.as_orbit();
            return &self.cameras.world;
        }
        // And the Map screen, which places a set of maps that is no document.
        if self.active_workspace == crate::ui::dock::Workspace::Map {
            return self.step_map_camera(inputs);
        }
        let documents = &self.documents;
        self.cameras
            .views
            .retain(|id, _| documents.iter().any(|d| d.id == *id));

        let Some(active) = self
            .active_document
            .filter(|id| self.document(*id).is_some())
        else {
            inputs.apply_to(&mut self.cameras.idle);
            return &self.cameras.idle;
        };
        let entry = selected_dictionary_entry(self, active);
        let reframe = inputs.frame_active
            || inputs.frame_selection
            || self
                .cameras
                .views
                .get(&active)
                .is_none_or(|view| view.framed_entry != entry);
        let bounds = if inputs.frame_selection {
            compute_selection_bounds(self)
        } else if reframe {
            compute_active_doc_bounds(self)
        } else {
            None
        };

        let view = self.cameras.views.entry(active).or_insert_with(|| View {
            camera: OrbitCamera::default(),
            framed_entry: entry,
        });
        inputs.apply_to(&mut view.camera);
        if reframe {
            if let Some((centre, radius)) = bounds {
                view.camera.frame_sphere(centre, radius);
            }
            view.framed_entry = entry;
        }
        &view.camera
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use glam::Vec3;
    use or_io::gta5::{SpdSphere, Vec4V};
    use or_renderer::OrbitCamera;

    use crate::render::viewport::CameraInputs;
    use crate::state::{AppState, DocPayload, DocumentId};
    use crate::ui::dock::Workspace;

    fn drawable_at(centre: Vec3, radius: f32) -> DocPayload {
        let mut drawable = or_io::gta5::GtaDrawable::default();
        drawable.base.lod_group.bounds_sphere = SpdSphere {
            center_and_radius: Vec4V {
                x: centre.x,
                y: centre.y,
                z: centre.z,
                w: radius,
                ..Vec4V::default()
            },
            ..SpdSphere::default()
        };
        DocPayload::Resource(or_io::Asset::Drawable(drawable))
    }

    fn open(state: &mut AppState, name: &str, centre: Vec3, radius: f32) -> DocumentId {
        state.insert_document(PathBuf::from(name), drawable_at(centre, radius), None, None)
    }

    fn frame(state: &mut AppState, inputs: CameraInputs) -> OrbitCamera {
        state.step_viewport_camera(inputs).clone()
    }

    fn idle(state: &mut AppState) -> OrbitCamera {
        frame(state, CameraInputs::default())
    }

    /// Every field, by bit pattern: "close to where it was left" is not the claim.
    fn bits(camera: &OrbitCamera) -> [u32; 9] {
        [
            camera.target.x,
            camera.target.y,
            camera.target.z,
            camera.yaw,
            camera.pitch,
            camera.distance,
            camera.fov_y,
            camera.near,
            camera.far,
        ]
        .map(f32::to_bits)
    }

    fn a_drag(orbit: (f32, f32), pan: (f32, f32), zoom: f32) -> CameraInputs {
        CameraInputs {
            orbit: egui::vec2(orbit.0, orbit.1),
            pan: egui::vec2(pan.0, pan.1),
            zoom,
            ..CameraInputs::default()
        }
    }

    fn fresh_state() -> AppState {
        let mut state = AppState::new();
        state.documents.clear();
        state.active_document = None;
        state
    }

    #[test]
    fn a_documents_camera_survives_switching_documents_and_workspaces() {
        let mut state = fresh_state();
        let a_centre = Vec3::new(10.0, 0.0, 0.0);
        let a = open(&mut state, "a.ydr", a_centre, 2.0);
        let framed_a = idle(&mut state);
        assert_eq!(
            framed_a.target, a_centre,
            "a document seen for the first time is framed on its own bounds"
        );

        frame(&mut state, a_drag((120.0, -40.0), (30.0, 15.0), 1.5));
        let left_a = idle(&mut state);
        assert_ne!(
            bits(&left_a),
            bits(&framed_a),
            "the drag has to move A's camera, or the test proves nothing"
        );

        let b = open(&mut state, "b.ydr", Vec3::new(-50.0, 20.0, 5.0), 8.0);
        assert_eq!(state.active_document, Some(b));
        idle(&mut state);
        frame(&mut state, a_drag((-200.0, 60.0), (0.0, 0.0), -1.0));
        let left_b = idle(&mut state);

        let started = state.active_workspace;
        let mut visited = 0;
        for workspace in Workspace::ALL {
            state.active_workspace = workspace;
            idle(&mut state);
            visited += 1;
        }
        assert_eq!(visited, Workspace::ALL.len());
        // ⚠ The Wardrobe has a camera of its own, for a ped that is no document: the one a document owns is read again where documents are drawn.
        state.active_workspace = started;
        assert_eq!(
            bits(&idle(&mut state)),
            bits(&left_b),
            "switching workspace moved the active document's camera"
        );

        state.set_active_document(a);
        assert_eq!(
            bits(&idle(&mut state)),
            bits(&left_a),
            "A's camera was not where the user left it"
        );

        state.set_active_document(b);
        assert_eq!(
            bits(&idle(&mut state)),
            bits(&left_b),
            "B's camera was not where the user left it"
        );
    }

    #[test]
    fn the_frame_key_reframes_the_active_document_and_no_other() {
        let mut state = fresh_state();
        let a = open(&mut state, "a.ydr", Vec3::new(10.0, 0.0, 0.0), 2.0);
        idle(&mut state);
        frame(&mut state, a_drag((80.0, 10.0), (40.0, -20.0), 2.0));
        let left_a = idle(&mut state);

        let b_centre = Vec3::new(-50.0, 20.0, 5.0);
        open(&mut state, "b.ydr", b_centre, 8.0);
        let framed_b = idle(&mut state);
        frame(&mut state, a_drag((0.0, 0.0), (90.0, 90.0), -1.5));
        assert_ne!(idle(&mut state).target, b_centre);
        let reframed_b = frame(
            &mut state,
            CameraInputs {
                frame_active: true,
                ..CameraInputs::default()
            },
        );
        assert_eq!(reframed_b.target, framed_b.target);
        assert_eq!(reframed_b.distance, framed_b.distance);

        state.set_active_document(a);
        assert_eq!(bits(&idle(&mut state)), bits(&left_a));
    }

    #[test]
    fn closing_a_document_forgets_its_camera() {
        let mut state = fresh_state();
        let a = open(&mut state, "a.ydr", Vec3::new(1.0, 2.0, 3.0), 1.0);
        idle(&mut state);
        let b = open(&mut state, "b.ydr", Vec3::new(4.0, 5.0, 6.0), 1.0);
        idle(&mut state);
        assert_eq!(state.cameras.views.len(), 2);

        state.close_document(b);
        idle(&mut state);
        assert!(!state.cameras.views.contains_key(&b));
        assert!(state.cameras.views.contains_key(&a));

        state.close_document(a);
        let with_nothing_open = idle(&mut state);
        assert!(state.cameras.views.is_empty());
        assert_eq!(
            bits(&with_nothing_open),
            bits(&OrbitCamera::default()),
            "with nothing open the viewport falls back to its own camera"
        );
    }

    const WHOLE: (Vec3, f32) = (Vec3::new(-20.0, 0.0, 0.0), 50.0);
    const FIRST: (Vec3, Vec3) = (Vec3::ZERO, Vec3::splat(2.0));
    const SECOND: (Vec3, Vec3) = (Vec3::new(10.0, 0.0, 0.0), Vec3::new(12.0, 2.0, 2.0));

    fn both() -> (Vec3, Vec3) {
        (FIRST.0.min(SECOND.0), FIRST.1.max(SECOND.1))
    }

    fn centre((min, max): (Vec3, Vec3)) -> Vec3 {
        (min + max) * 0.5
    }

    /// A drawable bounded by `WHOLE` whose one model holds the geometries `FIRST` and `SECOND`.
    fn two_geometries() -> or_io::gta5::GtaDrawable {
        let corner = |p: Vec3| Vec4V {
            x: p.x,
            y: p.y,
            z: p.z,
            ..Vec4V::default()
        };
        let aabb = |(min, max): (Vec3, Vec3)| or_io::gta5::SpdAABB {
            min: corner(min),
            max: corner(max),
            ..Default::default()
        };
        let model = or_io::gta5::GrmModel::<or_io::decl::Gen8> {
            geometries: vec![Box::default(), Box::default()],
            // Above one geometry, the model's own box comes first.
            aabbs: vec![aabb(both()), aabb(FIRST), aabb(SECOND)],
            ..Default::default()
        };
        let DocPayload::Resource(or_io::Asset::Drawable(mut drawable)) =
            drawable_at(WHOLE.0, WHOLE.1)
        else {
            unreachable!("drawable_at makes a drawable")
        };
        drawable.base.lod_group.lods[0].models.push(Box::new(model));
        drawable
    }

    #[test]
    fn frame_selection_frames_the_selected_geometry_or_model_and_otherwise_everything() {
        use crate::ui::panels::selection_ids::pack_selection;

        let mut state = fresh_state();
        let doc = state.insert_document(
            PathBuf::from("two.ydr"),
            DocPayload::Resource(or_io::Asset::Drawable(two_geometries())),
            None,
            None,
        );
        assert_eq!(idle(&mut state).target, WHOLE.0);
        let selection = CameraInputs {
            frame_selection: true,
            ..CameraInputs::default()
        };
        let all = CameraInputs {
            frame_active: true,
            ..CameraInputs::default()
        };

        state.selection.select(doc, pack_selection(0, 0, Some(1)));
        assert_eq!(frame(&mut state, selection).target, centre(SECOND));
        state.selection.select(doc, pack_selection(0, 0, None));
        assert_eq!(frame(&mut state, selection).target, centre(both()));
        assert_eq!(
            frame(&mut state, all).target,
            WHOLE.0,
            "Frame all frames everything whatever is selected"
        );
        state.selection.clear(doc);
        assert_eq!(
            frame(&mut state, selection).target,
            WHOLE.0,
            "with nothing selected, Frame selection frames everything"
        );
    }

    #[test]
    fn frame_selection_frames_a_geometry_inside_a_dictionary_entry() {
        use crate::ui::panels::selection_ids::{
            dict_entry_id, dict_entry_part_id, pack_lod_sub_index, YPT_MODEL_KIND_LOD,
        };
        const KEY: u32 = 0x00C0_FFEE;

        let mut state = fresh_state();
        let dictionary = or_io::gta5::PgDictionary {
            codes: vec![KEY],
            entries: vec![Some(Box::new(two_geometries()))],
            ..Default::default()
        };
        let doc = state.insert_document(
            PathBuf::from("two.ydd"),
            DocPayload::Resource(or_io::Asset::DrawableDict(dictionary)),
            None,
            None,
        );
        state.selection.select(doc, dict_entry_id(KEY));
        assert_eq!(idle(&mut state).target, WHOLE.0, "an entry frames whole");

        let first = dict_entry_part_id(KEY, YPT_MODEL_KIND_LOD, pack_lod_sub_index(0, 0, Some(0)));
        state.selection.select(doc, first);
        let selection = CameraInputs {
            frame_selection: true,
            ..CameraInputs::default()
        };
        assert_eq!(frame(&mut state, selection).target, centre(FIRST));
    }
}
