//! The dressed ped in the viewport: the parts the Wardrobe hands over, drawn in place of a document's meshes.

use std::sync::Arc;

use glam::Mat4;
use or_renderer::{LineVertex, MeshDraw, MeshVertex};
use wgpu::util::DeviceExt;

use super::texture_upload::upload_rgba;
use super::RenderBridge;
use crate::render::debug_channel::DebugChannel;
use crate::render::dressed::{Aux, DressedScene, ScenePart};

/// What the bridge keeps of the dressed ped besides the draws it made of it.
#[derive(Default)]
pub(super) struct Held {
    /// The part and the mesh of each draw, in the order of the draws.
    meshes: Vec<(usize, usize)>,
    /// What each draw's vertex buffer holds now.
    vertices: Vec<Vec<MeshVertex>>,
    /// The edges of each draw, each once.
    edges: Vec<Vec<[u32; 2]>>,
    channel: Option<DebugChannel>,
    /// The pose the vertices are in, none for the rest pose.
    pose: Option<u64>,
    /// The scene the game's passes hold was prepared from this scene and this pack epoch.
    pub(super) game_key: Option<(u64, u64)>,
    /// The pose the game's passes hold for this ped: its number and how many bones it has.
    pub(super) skin_key: Option<(u64, usize)>,
}

/// The colour of the dressed ped's edges in the wireframe view.
const WIRE: [f32; 4] = [0.55, 0.78, 0.95, 0.85];

/// Each edge of the triangles `indices` lists once, smaller end first.
pub(crate) fn edges_of(indices: &[u32]) -> Vec<[u32; 2]> {
    let mut edges: Vec<[u32; 2]> = indices
        .as_chunks::<3>()
        .0
        .iter()
        .flat_map(|[a, b, c]| [[*a, *b], [*b, *c], [*c, *a]])
        .filter(|[a, b]| a != b)
        .map(|[a, b]| [a.min(b), a.max(b)])
        .collect();
    edges.sort_unstable();
    edges.dedup();
    edges
}

/// The colour of a vertex that a debug view of vertex data has none to show: neutral grey, which neither a normal nor a colour of nothing is.
const NO_DATA: [f32; 4] = [0.5, 0.5, 0.5, 1.0];

/// `source` as the part is drawn: raised and tinted, or in the colour of the channel asked for; `aux` holds the colours of the channels the plain vertex does not carry.
fn styled(
    part: &ScenePart,
    source: &[MeshVertex],
    channel: Option<DebugChannel>,
    aux: Option<&Aux>,
) -> Vec<MeshVertex> {
    let [r, g, b] = part.tint;
    source
        .iter()
        .enumerate()
        .map(|(at, vertex)| {
            let mut vertex = *vertex;
            vertex.position[2] += part.lift;
            vertex.colour = match channel {
                Some(DebugChannel::Texcoord(_)) => {
                    DebugChannel::Texcoord(0).visualise([vertex.uv[0], vertex.uv[1], 0.0, 1.0])
                }
                Some(shown @ (DebugChannel::Colour(_) | DebugChannel::Tangent(_))) => aux
                    .and_then(|aux| aux.of(shown))
                    .and_then(|colours| colours.get(at))
                    .copied()
                    .unwrap_or(NO_DATA),
                Some(channel) => {
                    channel.visualise([vertex.normal[0], vertex.normal[1], vertex.normal[2], 1.0])
                }
                None => [
                    vertex.colour[0] * r,
                    vertex.colour[1] * g,
                    vertex.colour[2] * b,
                    vertex.colour[3],
                ],
            };
            vertex
        })
        .collect()
}

impl RenderBridge {
    /// Draw `scene`, or nothing for none, until a document's meshes are asked for again, with the vertex colours of `channel` where one is asked for; the pose is forgotten.
    ///
    /// ⚠ **A document's cache is let go here and this one's in `refresh_meshes`**: both fill the one
    /// list of draws, so each must make the other build again when its turn comes.
    pub fn refresh_dressed_as(
        &mut self,
        scene: Option<&DressedScene>,
        channel: Option<DebugChannel>,
    ) {
        self.mesh_cache_key = None;
        self.cached_mesh_uploads.clear();
        let Some(scene) = scene else {
            self.cached_mesh_draws.clear();
            self.dressed_key = None;
            self.held = Held::default();
            return;
        };
        // ⚠ The channel is part of what is drawn: its colours are packed into the vertices.
        let key = scene.key
            ^ channel
                .map_or(0, |c| 1 + c.channel() as u64)
                .rotate_left(48);
        if self.dressed_key == Some(key) {
            return;
        }
        self.dressed_key = Some(key);
        let mut draws = Vec::new();
        let mut held = Held {
            channel,
            ..Held::default()
        };
        let mut kept = std::collections::HashSet::new();
        for (at, part) in scene.parts.iter().enumerate() {
            let bind_group = self.dressed_texture(part, &mut kept);
            for (mesh, geometry) in part.geometry.iter().enumerate() {
                if geometry.vertices.is_empty() || geometry.indices.is_empty() {
                    continue;
                }
                let vertices = styled(part, &geometry.vertices, channel, scene.aux(at, mesh));
                draws.push(self.dressed_draw(&vertices, &geometry.indices, bind_group.clone()));
                held.edges.push(edges_of(&geometry.indices));
                held.vertices.push(vertices);
                held.meshes.push((at, mesh));
            }
        }
        // A texture no part draws with any more is let go of, so reading one again does not grow the cache.
        self.dressed_textures.retain(|id, _| kept.contains(id));
        self.cached_mesh_draws = draws;
        self.held = held;
        self.skinned_revision = None;
    }

    /// Put the skinned meshes of `scene` in the pose `skin` stands for, named by `revision`, or at rest for none; a pose already held is not written again.
    pub fn pose_dressed(&mut self, scene: &DressedScene, pose: Option<(u64, &[Mat4])>) {
        let wanted = pose.map(|(revision, _)| revision);
        if self.held.pose == wanted {
            return;
        }
        self.held.pose = wanted;
        for (at, &(part, mesh)) in self.held.meshes.iter().enumerate() {
            if !scene.is_skinned_mesh(part, mesh) {
                continue;
            }
            let Some(source) = scene.parts.get(part) else {
                continue;
            };
            let posed = pose.and_then(|(_, skin)| scene.posed(part, mesh, skin));
            let rest = &source.geometry[mesh].vertices;
            let vertices = styled(
                source,
                posed.as_deref().unwrap_or(rest),
                self.held.channel,
                scene.aux(part, mesh),
            );
            if let Some(draw) = self.cached_mesh_draws.get(at) {
                self.queue
                    .write_buffer(&draw.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
            }
            self.held.vertices[at] = vertices;
        }
    }

    /// The edges of the dressed ped where it stands now, for the wireframe view.
    pub(super) fn dressed_wire_lines(&self) -> Vec<LineVertex> {
        let at = |draw: usize, end: u32| self.held.vertices[draw][end as usize].position;
        (self.held.edges.iter().enumerate())
            .flat_map(|(draw, edges)| {
                edges.iter().flat_map(move |[a, b]| {
                    [
                        LineVertex::new(at(draw, *a), WIRE),
                        LineVertex::new(at(draw, *b), WIRE),
                    ]
                })
            })
            .collect()
    }

    /// The parts of `scene` garment `id` wears, alone, as [`Self::refresh_dressed_as`] draws them, from `camera` on a transparent `px`-square target, read back, its alpha as `alpha` says and its colour drawn by the game's own passes from `game` where there is a pack of them.
    ///
    /// ⚠ Draws with this bridge's own renderer, so it is for a bridge that has no viewport to show.
    pub fn photograph_garment(
        &mut self,
        scene: &DressedScene,
        id: &str,
        camera: &or_renderer::OrbitCamera,
        px: u32,
        alpha: super::Alpha,
        game: Option<
            &[(
                or_renderer::game_shaders::mesh::Surface,
                or_renderer::game_shaders::mesh::Material,
            )],
        >,
    ) -> Result<image::RgbaImage, String> {
        let mut draws = Vec::new();
        let mut kept = std::collections::HashSet::new();
        let worn = (scene.parts.iter()).filter(|part| part.garment.as_deref() == Some(id));
        for part in worn {
            let bind_group = self.dressed_texture(part, &mut kept);
            for geometry in
                (part.geometry.iter()).filter(|g| !g.vertices.is_empty() && !g.indices.is_empty())
            {
                let vertices = styled(part, &geometry.vertices, None, None);
                draws.push(self.dressed_draw(&vertices, &geometry.indices, bind_group.clone()));
            }
        }
        self.dressed_textures.retain(|id, _| kept.contains(id));
        self.shoot(draws, camera, px, alpha, game)
    }

    /// The bind group of the part's texture, uploaded once for as long as the scene draws with it.
    fn dressed_texture(
        &mut self,
        part: &ScenePart,
        kept: &mut std::collections::HashSet<u64>,
    ) -> Option<Arc<wgpu::BindGroup>> {
        let diffuse = part.diffuse.as_ref()?;
        kept.insert(diffuse.id);
        if let Some(held) = self.dressed_textures.get(&diffuse.id) {
            return Some(held.clone());
        }
        let (group, _) = upload_rgba(
            &self.device,
            &self.queue,
            &self.material_bgl,
            (diffuse.width, diffuse.height),
            &diffuse.rgba,
            diffuse.srgb,
        );
        let group = Arc::new(group);
        self.dressed_textures.insert(diffuse.id, group.clone());
        Some(group)
    }

    /// One draw of a mesh whose vertices are already as the part draws them.
    fn dressed_draw(
        &self,
        vertices: &[MeshVertex],
        indices: &[u32],
        texture_bind_group: Option<Arc<wgpu::BindGroup>>,
    ) -> MeshDraw {
        let vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("dressed-mesh-vb"),
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            });
        let index_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("dressed-mesh-ib"),
                contents: bytemuck::cast_slice(indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        MeshDraw {
            bump: false,
            vertex_buffer,
            index_buffer,
            index_count: indices.len() as u32,
            texture_bind_group,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_renderer::{OrbitCamera, ShadingMode};

    #[test]
    fn every_edge_of_a_quad_is_listed_once_smaller_end_first_and_a_degenerate_triangle_adds_none() {
        // Two triangles over a quad share the diagonal.
        assert_eq!(
            edges_of(&[0, 1, 2, 0, 2, 3]),
            vec![[0, 1], [0, 2], [0, 3], [1, 2], [2, 3]]
        );
        assert_eq!(edges_of(&[3, 3, 3]), Vec::<[u32; 2]>::new());
        assert_eq!(edges_of(&[2, 1, 0]), vec![[0, 1], [0, 2], [1, 2]]);
    }

    #[test]
    fn a_channel_colours_the_vertex_by_its_normal_or_its_uv_and_leaves_the_tint_out() {
        let part = ScenePart {
            geometry: Arc::new(Vec::new()),
            diffuse: None,
            tint: [0.5, 0.5, 0.5],
            lift: 0.25,
            garment: None,
        };
        let vertex = MeshVertex::new([1.0, 2.0, 3.0], [0.0, 0.0, -1.0], [2.0, 0.25]);
        let tinted = styled(&part, &[vertex], None, None);
        assert_eq!(tinted[0].position, [1.0, 2.0, 3.25]);
        assert_eq!(tinted[0].colour, [0.5, 0.5, 0.5, 1.0]);
        let normal = styled(&part, &[vertex], Some(DebugChannel::Normal), None);
        assert_eq!(normal[0].colour, [0.5, 0.5, 0.0, 1.0]);
        let uv = styled(&part, &[vertex], Some(DebugChannel::Texcoord(0)), None);
        assert_eq!(uv[0].colour, [1.0, 0.25, 0.0, 1.0], "a uv past 1 saturates");
    }

    #[test]
    fn the_vertex_colour_and_the_tangent_come_from_the_data_of_the_mesh_and_a_mesh_without_them_is_grey(
    ) {
        let part = ScenePart {
            geometry: Arc::new(Vec::new()),
            diffuse: None,
            tint: [0.5, 0.5, 0.5],
            lift: 0.0,
            garment: None,
        };
        let vertices = [
            MeshVertex::new([0.0; 3], [0.0, 0.0, 1.0], [0.0, 0.0]),
            MeshVertex::new([1.0; 3], [0.0, 0.0, 1.0], [0.0, 0.0]),
        ];
        let aux = Aux {
            colour: Some(vec![[1.0, 0.0, 0.0, 1.0], [0.0, 0.5, 1.0, 1.0]]),
            tangent: None,
        };
        let colours = styled(&part, &vertices, Some(DebugChannel::Colour(0)), Some(&aux));
        assert_eq!(
            colours[0].colour,
            [1.0, 0.0, 0.0, 1.0],
            "the vertex's own colour, unlit and untinted"
        );
        assert_eq!(colours[1].colour, [0.0, 0.5, 1.0, 1.0]);
        let tangents = styled(&part, &vertices, Some(DebugChannel::Tangent(0)), Some(&aux));
        assert_eq!(tangents[0].colour, NO_DATA, "this mesh has no tangent");
        let bare = styled(&part, &vertices, Some(DebugChannel::Colour(0)), None);
        assert_eq!(bare[1].colour, NO_DATA, "a scene made without the data");
        let short = Aux {
            colour: Some(vec![[1.0; 4]]),
            tangent: None,
        };
        assert_eq!(
            styled(
                &part,
                &vertices,
                Some(DebugChannel::Colour(0)),
                Some(&short)
            )[1]
            .colour,
            NO_DATA,
            "a vertex past the end of the data has none"
        );
    }

    use crate::render::dressed::{Diffuse, DressedScene, Geometry, ScenePart};
    use or_renderer::game_shaders::mesh::bone_palette;

    const PX: u32 = 128;

    /// A bridge in Shaded, with the pack `OPENRAGE_GAME_PACK` names, on a device; none where there is no adapter or no pack.
    fn shaded_bridge(what: &str) -> Option<(Arc<wgpu::Device>, RenderBridge)> {
        let (device, queue) = crate::render::test_device(what)?;
        let Ok(pack) = std::env::var("OPENRAGE_GAME_PACK") else {
            or_test_support::skip_install(what, format_args!("OPENRAGE_GAME_PACK names no pack"));
            return None;
        };
        let device = Arc::new(device);
        let mut bridge = RenderBridge::new(
            device.clone(),
            Arc::new(queue),
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        assert!(bridge.set_game_pack_path(std::path::Path::new(&pack)));
        bridge.mode = ShadingMode::Shaded;
        bridge.renderer.resize(&device, PX, PX);
        Some((device, bridge))
    }

    /// Every ped component of the corpus, the smallest first.
    fn components() -> Vec<std::path::PathBuf> {
        let mut files = or_test_support::Corpus::new(env!("CARGO_MANIFEST_DIR"), "ydd")
            .target(or_test_support::TARGET_GEN8)
            .find();
        files.sort_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(u64::MAX));
        files
    }

    fn texture(id: u64, rgba: [u8; 4]) -> Arc<Diffuse> {
        Arc::new(Diffuse {
            id,
            width: 2,
            height: 2,
            rgba: rgba.repeat(4),
            srgb: true,
        })
    }

    /// `asset` worn with `diffuse` as the Wardrobe makes a part of it, none for a file with no drawable.
    fn scene_with(asset: &or_io::Asset, diffuse: Arc<Diffuse>) -> Option<DressedScene> {
        let shaded = crate::render::game_scene::surfaces_of_asset(asset, |_, _| None)?;
        let geometry: Vec<Geometry> = crate::render::meshes::meshes_of_asset_in(asset, 0, None)
            .into_iter()
            .map(|m| Geometry {
                vertices: m.vertices,
                indices: m.indices,
            })
            .collect();
        let part = ScenePart {
            geometry: Arc::new(geometry),
            diffuse: Some(diffuse),
            tint: [1.0; 3],
            lift: 0.0,
            garment: None,
        };
        // The key is what tells the bridge the scene changed, as the texture id does in the app
        let key = part.diffuse.as_ref().map_or(0, |d| d.id);
        Some(DressedScene::new(key, vec![part]).with_shaded(vec![Some(Arc::new(shaded))]))
    }

    #[test]
    #[ignore = "needs a GPU adapter: acquisition can hang, not just fail"]
    fn a_dressed_ped_shown_in_shaded_is_drawn_by_the_game_with_the_texture_it_is_worn_with() {
        const WHAT: &str = "a_dressed_ped_shown_in_shaded_is_drawn_by_the_game";
        let Some((device, mut bridge)) = shaded_bridge(WHAT) else {
            return;
        };
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let (mut drew, mut changed) = (None, None);
        for path in components() {
            let Ok(asset) = or_io::open(&path) else {
                continue;
            };
            let (Some(red), Some(blue)) = (
                scene_with(&asset, texture(1, [255, 0, 0, 255])),
                scene_with(&asset, texture(2, [0, 0, 255, 255])),
            ) else {
                continue;
            };
            let mut camera = OrbitCamera::default();
            camera.frame_sphere(red.centre, red.radius);
            let mut picture = |scene: &DressedScene| {
                bridge.refresh_dressed_game(Some(scene));
                bridge
                    .render_pass(&camera, ShadingMode::Shaded, PX)
                    .unwrap()
            };
            let (red, blue) = (picture(&red), picture(&blue));
            let first = red.get_pixel(0, 0);
            if red.pixels().all(|pixel| pixel == first) {
                continue;
            }
            drew.get_or_insert(path.clone());
            if red != blue {
                changed = Some(path);
                break;
            }
        }
        let error = pollster::block_on(validation.pop());
        assert!(error.is_none(), "{error:?}");
        let Some(drew) = drew else {
            or_test_support::skip_corpus(WHAT, "no ped component the game passes draw");
            return;
        };
        assert!(
            changed.is_some(),
            "{}: the texture it is worn with is not what it is drawn with, nor on any other drawable that shows",
            drew.display()
        );
    }

    #[test]
    #[ignore = "needs a GPU adapter: acquisition can hang, not just fail"]
    fn a_dressed_ped_shown_in_shaded_follows_the_pose_a_clip_gives_its_skeleton() {
        const WHAT: &str = "a_dressed_ped_shown_in_shaded_follows_the_pose";
        let Some((device, mut bridge)) = shaded_bridge(WHAT) else {
            return;
        };
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        // The first skinned ped component of the corpus that shows something at rest
        let mut shown = None;
        for path in components() {
            let Ok(asset) = or_io::open(&path) else {
                continue;
            };
            let Some(scene) = scene_with(&asset, texture(1, [200, 200, 200, 255])) else {
                continue;
            };
            if bone_palette(&scene.shaded_surfaces()).is_empty() {
                continue;
            }
            let mut camera = OrbitCamera::default();
            camera.frame_sphere(scene.centre, scene.radius);
            bridge.refresh_dressed_game(Some(&scene));
            bridge.pose_dressed_game(None);
            let rest = bridge
                .render_pass(&camera, ShadingMode::Shaded, PX)
                .unwrap();
            let first = rest.get_pixel(0, 0);
            if rest.pixels().any(|pixel| pixel != first) {
                shown = Some((path, camera, rest));
                break;
            }
        }
        let Some((path, camera, rest)) = shown else {
            or_test_support::skip_corpus(WHAT, "no skinned ped component that shows");
            return;
        };
        let mut picture = |pose: (u64, Vec<glam::Mat4>)| {
            bridge.pose_dressed_game(Some((pose.0, &pose.1)));
            bridge
                .render_pass(&camera, ShadingMode::Shaded, PX)
                .unwrap()
        };
        // Every bone where it is leaves the ped as it stands; a pose that carries every bone far off carries the ped with it
        let same = picture((1, vec![glam::Mat4::IDENTITY; 512]));
        assert_eq!(same, rest, "{}: a pose at rest moved it", path.display());
        let moved = picture((
            2,
            vec![glam::Mat4::from_translation(glam::Vec3::splat(1.0e4)); 512],
        ));
        let error = pollster::block_on(validation.pop());
        assert!(error.is_none(), "{error:?}");
        assert_ne!(moved, rest, "{}: the pose did not move it", path.display());
    }
}
