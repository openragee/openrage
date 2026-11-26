//! A scanned GTA VI drawable offers what a GTA V drawable's view offers: its levels of detail with
//! their distances, Collision over its own bound, its bones, its rows in the Outline, the textures
//! its shaders bind, each vertex channel it carries, and effects kept across runs; its row adds
//! the export to glTF a GTA V drawable has in its menus.

use std::sync::Arc;

use or_io::gta6::headerless::{
    Bone, Bound, BoundChild, BoundGeometry, ChildBound, Drawable, Geometry, Model, Palette,
    Primitive, Resource, Shader, Skeleton, Vertices,
};

use super::capabilities::{self, GeometryRequest};
use super::textures::{TextureListing, TextureResolution, TextureSource};
use super::{AppState, DocPayload, DocumentId};
use crate::render::debug_channel::DebugChannel;
use crate::ui::panels::selection_ids::{pack_selection, SECT_TEXTURE};

const KEY: u64 = 0x0123_4567_89AB_CDEF;

fn triangle(at: f32, vertices: Vertices) -> Geometry {
    Geometry {
        vertex_data: Vec::new(),
        index_data: Vec::new(),
        shader: 0,
        vertex_count: 3,
        channels: Vec::new(),
        triangles: vec![[0, 1, 2]],
        positions: Ok(vec![[at, 0.0, 0.0], [at + 1.0, 0.0, 0.0], [at, 1.0, 0.0]]),
        vertices,
    }
}

fn level(geometry: Geometry) -> Option<Vec<Model>> {
    Some(vec![Model {
        geometries: vec![Ok(geometry)],
        palette: Ok(Palette::Own),
        matrix_index: 0,
    }])
}

/// A composite of one geometry child: a triangle 10 m along X.
fn bound() -> Bound {
    let geometry = BoundGeometry {
        box_min: [0.0; 3],
        box_max: [1.0; 3],
        vertices: vec![[10.0, 0.0, 0.0], [11.0, 0.0, 0.0], [10.0, 1.0, 0.0]],
        primitives: vec![Primitive::Triangle {
            vertices: [0, 1, 2],
            neighbours: [0xFFFF; 3],
            edges: [0xFF; 3],
        }],
        material_ids: Vec::new(),
        primitive_materials: Vec::new(),
        bvh: None,
        unknown_shapes: Vec::new(),
    };
    let mut matrix = [[0.0; 4]; 4];
    for (r, row) in matrix.iter_mut().enumerate().take(3) {
        row[r] = 1.0;
    }
    Bound {
        radius: 1.0,
        box_min: [10.0, 0.0, 0.0],
        box_max: [11.0, 1.0, 0.0],
        centroid: [0.0; 3],
        children: vec![BoundChild {
            matrix,
            flags: None,
            bound: Some(Ok(ChildBound::Geometry(geometry))),
            material_id: None,
        }],
        bvh: None,
    }
}

/// A drawable of levels 0 and 2, drawn to 25 m and 140 m, whose level 0 carries a normal and two
/// texcoords and whose one shader binds [`KEY`] twice.
fn drawable(with_bound: bool) -> Drawable {
    let channels = Vertices {
        normals: vec![[0.0, 0.0, -1.0]; 3],
        uvs: vec![
            (28, vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]),
            (29, vec![[0.75, 0.75], [0.25, 0.5], [0.75, 0.25]]),
        ],
        ..Vertices::default()
    };
    Drawable {
        name: "prop_test.#dr".to_string(),
        box_min: [0.0; 3],
        box_max: [6.0, 1.0, 0.0],
        shaders: vec![Shader {
            name_hash: 0x1234_5678,
            texture_keys: vec![Some(KEY), None, Some(KEY)],
            parameters: Vec::new(),
        }],
        levels: [
            level(triangle(0.0, channels)),
            None,
            level(triangle(5.0, Vertices::default())),
            Some(Vec::new()),
        ],
        lod_distances: [25.0, 60.0, 140.0, 9998.0],
        bound: with_bound.then(bound),
        embedded: None,
        extra_skeletons: Vec::new(),
    }
}

fn opened(resource: Resource) -> (AppState, DocumentId, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let mut state = AppState::new();
    let payload = DocPayload::Gta6Drawable(Arc::new(resource));
    let doc = state.insert_document(dir.path().join("0x10.drawable"), payload, None, None);
    state.active_document = Some(doc);
    (state, doc, dir)
}

fn meshes(state: &AppState, doc: DocumentId) -> Vec<crate::render::meshes::MeshUpload> {
    let request = GeometryRequest {
        state,
        doc: state.document(doc).expect("open"),
        selection: state.selection.selected(doc),
        entry: None,
    };
    crate::render::gta6_drawable::meshes(&request)
}

#[test]
fn the_switch_offers_each_level_that_holds_models_at_its_distance_and_draws_the_one_picked() {
    let (mut state, doc, _dir) = opened(Resource::Drawable(drawable(false)));
    let levels = crate::render::viewport::pick::levels(&state);
    assert_eq!(levels, [(0, 25.0), (2, 140.0)]);
    state.preview_scene.lod = 2;
    let x = meshes(&state, doc)[0].vertices[0].position[0];
    assert_eq!(x, 5.0, "level 2's triangle");
    state.preview_scene.lod = 1;
    let x = meshes(&state, doc)[0].vertices[0].position[0];
    assert_eq!(x, 0.0, "a level with no models draws level 0");
}

#[test]
fn collision_draws_the_drawable_s_own_bound_and_else_its_fragment_s() {
    use crate::render::gta6_drawable::embedded_bound;
    use crate::render::lines::document_lines;
    let (mut state, _, _dir) = opened(Resource::Drawable(drawable(false)));
    assert!(embedded_bound(&state).is_none(), "no bound, no toggle");

    let (mut state_with, _, _dir2) = opened(Resource::Drawable(drawable(true)));
    assert!(embedded_bound(&state_with).is_some());
    state_with.preview_scene.show_collision = false;
    assert!(document_lines(&state_with, false).is_empty());
    state_with.preview_scene.show_collision = true;
    let lines = document_lines(&state_with, false);
    assert_eq!(lines.len(), 6, "the bound triangle's three edges");
    assert!(
        lines.iter().all(|v| v.position[0] >= 10.0),
        "placed 10 m along X"
    );

    let fragment = or_io::gta6::headerless::DrawableWithBound {
        name: "prop_test".to_string(),
        drawable: drawable(false),
        bound: Some(bound()),
        fragment: Default::default(),
    };
    let (fragment_state, _, _dir3) = opened(Resource::DrawableWithBound(fragment));
    assert!(embedded_bound(&fragment_state).is_some());
    state.preview_scene.show_collision = true;
    assert!(document_lines(&state, false).is_empty());
}

#[test]
fn a_scanned_skeleton_is_offered_as_bones_and_drawn_while_the_box_is_ticked() {
    use crate::render::viewport::bones;
    let (mut state, doc, _dir) = opened(Resource::Drawable(drawable(false)));
    assert!(!bones::can_draw(&state), "no skeleton was read with it");
    let bone = |name: &str, parent, y: f32| Bone {
        name: name.to_string(),
        hash: or_io::gta6::hash::name_hash32(name.as_bytes()),
        parent,
        rotation: [0.0, 0.0, 0.0, 1.0],
        translation: [0.0, y, 0.0],
        scale: [1.0; 3],
        inverse_bind: [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, -y, 0.0],
        ],
    };
    let skeleton = Skeleton {
        bones: vec![
            bone("SKEL_ROOT", None, 0.0),
            bone("SKEL_Head", Some(0), 1.5),
        ],
    };
    let path = state
        .document(doc)
        .and_then(|d| d.path.clone())
        .expect("a path");
    super::rpf9_scan::keep_scanned_skeleton(&path, skeleton);
    assert!(bones::can_draw(&state));
    state.preview_scene.show_rest_bones = false;
    assert!(bones::lines(&state).is_empty());
    state.preview_scene.show_rest_bones = true;
    assert_eq!(bones::drawn_from(&state), Some(doc));
    let lines = bones::lines(&state);
    assert!(
        lines.iter().any(|v| (v.position[1] - 1.5).abs() < 1e-4),
        "the head is drawn at its bind: {lines:?}"
    );
}

#[test]
fn the_row_offers_export_to_gltf_which_asks_what_the_menu_s_command_asks() {
    use super::gltf::{Asked, ExportChoice, Model};
    use super::viewport_basics_for_inspecting_assets::{click_in_the_row, the_row_shows};
    use crate::ui::commands::{self, Action, Target};
    use crate::ui::panels::viewport_drawable_row::height;
    use or_io::gta6::headerless::{Dictionary, Entry};
    const EXPORT: &str = "Export to glTF…";
    /// The GTA VI export that waits on an answer: its model's name and what is chosen of it.
    fn asked(state: &AppState) -> Option<(String, ExportChoice)> {
        match &state.gltf.asked {
            Some(Asked::Export(export)) if export.gta6.is_some() => {
                Some((export.name.clone(), export.choice.clone()))
            }
            _ => None,
        }
    }

    let (mut state, doc, _dir) = opened(Resource::Drawable(drawable(false)));
    assert!(
        click_in_the_row(&mut state, EXPORT),
        "the row has no export"
    );
    let by_the_row = asked(&state);
    assert!(by_the_row.is_some(), "the click asked for no export");
    state.gltf.cancel();
    let command = commands::on(&state, Action::ExportGltf, Target::Document(doc));
    commands::run(
        &mut state,
        &egui::Context::default(),
        &command.expect("the menu's command"),
    );
    assert_eq!(asked(&state), by_the_row, "the menu asks for another");

    // A dictionary of two's is drawn before a drawable is chosen, and runs once one is; a dictionary
    // of one opens on its drawable.
    let dictionary = Resource::Dictionary(Dictionary {
        entries: vec![
            (0x20, Ok(Entry::Drawable(Box::new(drawable(false))))),
            (0x21, Ok(Entry::Drawable(Box::new(drawable(false))))),
        ],
    });
    let (mut state, doc, _dir) = opened(dictionary);
    assert!(height(&state) > 0.0, "the row is given no room");
    let refused = commands::on(&state, Action::ExportGltf, Target::Document(doc))
        .and_then(|command| command.enabled.err())
        .expect("the command waits on a drawable");
    assert!(click_in_the_row(&mut state, EXPORT));
    assert!(
        !state.toasts.iter().any(|toast| toast.message == refused),
        "the button ran what the command refuses"
    );
    (state.selection).select(doc, crate::ui::panels::selection_ids::dict_entry_id(0x20));
    assert!(click_in_the_row(&mut state, EXPORT));
    assert!(asked(&state).is_some(), "it did not run on the one chosen");

    let (mut state, _, _dir) = opened(Resource::Bound(bound()));
    assert!(!the_row_shows(&mut state, EXPORT), "a bound's row has it");
    let gta_v = DocPayload::Resource(or_io::Asset::Drawable(Default::default()));
    let doc = state.insert_document("prop.ydr".into(), gta_v, None, None);
    assert!(state.offers_gltf(Model::Document(doc)));
    assert!(!the_row_shows(&mut state, EXPORT), "a GTA V row has it");
}

#[test]
fn the_outline_summary_and_textures_panel_list_what_a_gta_v_drawable_s_do() {
    let (state, doc, _dir) = opened(Resource::Drawable(drawable(false)));
    let payload = &state.document(doc).expect("open").payload;
    let row = capabilities::for_payload(payload).expect("a row");
    assert!((row.has_outline_children.expect("an outline"))(payload));
    let summarise = row.selection_summary.expect("a summary");
    let root = crate::ui::panels::selection_ids::DOC_ROOT_SEL;
    assert_eq!(summarise(payload, root), "Drawable: prop_test.#dr");
    // A model the Outline selects is named by its place in the drawable.
    let summary = summarise(payload, pack_selection(0, 0, None));
    assert_eq!(summary, "prop_test.#dr: model 0 of level 0");

    let TextureListing::Textures { textures, source } = super::textures::textures_in(payload, None)
    else {
        panic!("a listing");
    };
    assert_eq!(source, TextureSource::Root);
    let name = super::gta6_resource::texture_name(KEY);
    assert_eq!(textures.len(), 1, "a key bound twice is one texture");
    assert_eq!(textures[0].name, name);
    assert_eq!(textures[0].selection_id & SECT_TEXTURE, SECT_TEXTURE);
    match super::textures::texture_for_selection(payload, textures[0].selection_id) {
        TextureResolution::Found { name: found, .. } => assert_eq!(found, name),
        other => panic!("the row's texture, not {other:?}"),
    }
    assert!(matches!(
        super::textures::texture_for_selection(payload, pack_selection(0, 0, Some(0))),
        TextureResolution::NotATexture(_)
    ));
    assert!(
        super::textures::resolved_for_gta6(&state, doc, None, &name).is_none(),
        "nothing open holds it"
    );
}

#[test]
fn a_dictionary_s_own_texture_is_fetched_by_the_name_its_drawable_s_key_is_listed_by() {
    use or_io::gta6::headerless::{Dictionary, Entry};
    let name = super::gta6_resource::texture_name(KEY);
    let texture = super::gta6_textures_resolve_and_save::dictionary_texture(&name, 1, true);
    let dictionary = Resource::Dictionary(Dictionary {
        entries: vec![
            (0x20, Ok(Entry::Drawable(Box::new(drawable(false))))),
            texture,
        ],
    });
    let (state, doc, _dir) = opened(dictionary);
    let payload = &state.document(doc).expect("open").payload;
    let fetched = super::textures::texture_at(payload, TextureSource::Root, &name);
    assert!(matches!(&fetched, Some(super::textures::Texture::Owned(t)) if t.width == 4));
    // The row lists what the fetch returns, as the gallery holds it to.
    let selection = crate::ui::panels::selection_ids::dict_entry_id(0x20);
    let TextureListing::Textures { textures, .. } =
        super::textures::textures_in(payload, Some(selection))
    else {
        panic!("the drawable's textures are listed");
    };
    let listed = textures.iter().find(|t| t.name == name).expect("listed");
    let fetched = fetched.expect("fetched");
    assert_eq!((listed.format, listed.width), (fetched.format(), 4));
    assert_ne!(listed.format, or_io::codec::texture::GrcTextureFormat::None);
    let other = super::textures::texture_at(payload, TextureSource::Root, "hash_0000000000000001");
    assert!(other.is_none());

    let (alone, doc, _dir) = opened(Resource::Drawable(drawable(false)));
    let payload = &alone.document(doc).expect("open").payload;
    assert!(super::textures::texture_at(payload, TextureSource::Root, &name).is_none());
}

#[test]
fn the_outline_draws_a_drawable_s_rows_and_a_dictionary_s_drawables() {
    use or_io::gta6::headerless::{Dictionary, Entry};
    let dictionary = Resource::Dictionary(Dictionary {
        entries: vec![(0x20, Ok(Entry::Drawable(Box::new(drawable(false)))))],
    });
    // Each kind's rows, and not another kind's: a bound lists its children, a drawable its levels.
    let cases = [
        (Resource::Drawable(drawable(true)), "Geometry 0", "Child 0"),
        (dictionary, "prop_test.#dr", "Child 0"),
        (Resource::Bound(bound()), "Child 0", "Geometry 0"),
    ];
    for (resource, row, other) in cases {
        let (mut state, doc, _dir) = opened(resource);
        let outline = (capabilities::for_payload(&state.document(doc).expect("open").payload))
            .and_then(|row| row.outline)
            .expect("an outline");
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| outline(ui, &mut state, doc));
        let labels: Vec<String> = (output.platform_output.accesskit_update.into_iter())
            .flat_map(|update| update.nodes)
            .filter_map(|(_, node)| node.label().map(str::to_string))
            .collect();
        assert!(labels.iter().any(|l| l.contains(row)), "{labels:?}");
        assert!(!labels.iter().any(|l| l.contains(other)), "{labels:?}");
        let payload = &state.document(doc).expect("open").payload;
        let children = capabilities::for_payload(payload).and_then(|r| r.has_outline_children);
        assert!(children.expect("a chevron")(payload), "{row}");
    }
}

#[test]
fn the_channel_view_offers_each_channel_a_geometry_carries_and_colours_the_one_chosen() {
    let (mut state, doc, _dir) = opened(Resource::Drawable(drawable(false)));
    let offered = &meshes(&state, doc)[0].channels;
    assert_eq!(
        offered,
        &[
            DebugChannel::Normal,
            DebugChannel::Texcoord(0),
            DebugChannel::Texcoord(1)
        ]
    );
    let plain = meshes(&state, doc)[0].vertices[1];
    state.preview_scene.debug_channel = Some(DebugChannel::Texcoord(1));
    let shown = meshes(&state, doc)[0].vertices[1];
    assert_ne!(plain.colour, shown.colour);
    assert_eq!(
        shown.colour,
        [0.25, 0.5, 0.0, 1.0],
        "texcoord 1 of vertex 1"
    );
    state.preview_scene.debug_channel = Some(DebugChannel::Normal);
    let normal = meshes(&state, doc)[0].vertices[1].colour;
    assert_eq!(normal, [0.5, 0.5, 0.0, 1.0], "the normal down Z");
    state.preview_scene.debug_channel = Some(DebugChannel::Colour(0));
    assert_eq!(
        meshes(&state, doc)[0].vertices[1].colour,
        plain.colour,
        "a channel it does not carry is not drawn"
    );
}

#[test]
fn collision_draws_each_fragment_child_at_its_bone_and_the_model_draws_its_surfaces_over_it() {
    use or_io::gta6::headerless::{DrawableWithBound, Fragment, FragmentChild, Surface};
    use or_renderer::LineVertex;
    let placed = |at: [f32; 3]| {
        let mut m = [[0.0; 4]; 4];
        for (r, row) in m.iter_mut().enumerate().take(3) {
            row[r] = 1.0;
        }
        m[3] = [at[0], at[1], at[2], 0.0];
        m
    };
    let child = |composite, bound, at| FragmentChild {
        group: 0,
        rotation: [0.0, 0.0, 0.0, 1.0],
        translation: [0.0; 3],
        angular_inertia: [1.0; 3],
        mass: 1.0,
        composite,
        bound,
        flags: [0; 2],
        bound_word: 0,
        matrix: placed(at),
    };
    let own = ChildBound::Sphere {
        centre: [0.0; 3],
        radius: 0.25,
    };
    let fragment = Fragment {
        children: vec![
            child(Some(0), None, [0.0, 5.0, 0.0]),
            child(None, Some(Ok(own)), [0.0, 0.0, 3.0]),
        ],
        ..Fragment::default()
    };
    let surface = Surface {
        code: 7,
        triangles: vec![[[0.0, 0.0, 2.0], [1.0, 0.0, 2.0], [0.0, 1.0, 2.0]]],
        weights: vec![1.0],
        neighbours: false,
        word: 0x101,
    };
    let mut drawn = drawable(false);
    drawn.embedded = Some(Box::new(Ok(vec![surface])));
    let resource = Resource::DrawableWithBound(DrawableWithBound {
        name: "prop_test".to_string(),
        drawable: drawn,
        bound: Some(bound()),
        fragment: Box::new(fragment),
    });
    let (mut state, doc, _dir) = opened(resource);
    state.preview_scene.show_collision = true;
    let lines = crate::render::lines::document_lines(&state, false);
    let (listed, own): (Vec<&LineVertex>, Vec<&LineVertex>) =
        lines.iter().partition(|v| v.position[1] >= 4.0);
    assert_eq!(
        listed.len(),
        6,
        "the composite's triangle, at its child's matrix"
    );
    assert!(listed
        .iter()
        .all(|v| v.position[0] >= 10.0 && v.position[1] >= 5.0));
    assert!(!own.is_empty(), "the second-set child's sphere");
    for v in own {
        let from = glam::Vec3::from(v.position).distance(glam::Vec3::new(0.0, 0.0, 3.0));
        assert!((from - 0.25).abs() < 1e-3, "{v:?}");
    }
    let meshes = meshes(&state, doc);
    let last = meshes.last().expect("the surfaces' mesh");
    assert_eq!(meshes.len(), 2, "level 0's triangle, then the surface");
    let at: Vec<[f32; 3]> = last.vertices.iter().map(|v| v.position).collect();
    assert_eq!(at, [[0.0, 0.0, 2.0], [1.0, 0.0, 2.0], [0.0, 1.0, 2.0]]);
    assert_ne!(last.vertices[0].colour, meshes[0].vertices[0].colour);
}
