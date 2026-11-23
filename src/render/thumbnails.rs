use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{channel, Receiver, Sender};

use or_renderer::{MeshDraw, OrbitCamera, ShadingMode, ViewportRenderer};

use crate::render::meshes::{MeshUpload, TileJob};
use crate::state::DocumentId;

pub const TILE_PX: u32 = 256;

const CAPACITY: usize = 256;

/// A tile's meshes as a worker sends them.
type Built = (ThumbKey, Vec<MeshUpload>);

/// How many tiles' meshes are built at once.
const BUILDING: usize = 4;

/// The yaw a tile is drawn from, which sees the front of most props.
pub const TILE_YAW: f32 = -std::f32::consts::FRAC_PI_3;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct ThumbKey {
    pub doc: DocumentId,
    pub selection: u64,
    pub payload_epoch: u64,
    /// ⚠ The look as well as the contents: a tile is drawn in the viewport's mode, with its
    /// channel and its level of detail, and is kept until its key changes.
    mode: ShadingMode,
    channel: Option<crate::render::debug_channel::DebugChannel>,
    lod: usize,
}

impl ThumbKey {
    /// The tile of `selection` in `doc` as it stands.
    pub fn of(doc: &crate::state::Document, selection: u64) -> Self {
        Self {
            doc: doc.id,
            selection,
            payload_epoch: doc.payload_epoch(),
            mode: ShadingMode::default(),
            channel: None,
            lod: 0,
        }
    }

    /// The same tile as a viewport in `mode` draws it.
    pub fn drawn_by(self, state: &crate::state::AppState, mode: ShadingMode) -> Self {
        Self {
            mode: tile_mode(mode),
            channel: state.preview_scene.debug_channel,
            lod: state.preview_scene.lod,
            ..self
        }
    }
}

pub struct Thumbnail {
    pub id: Option<egui::TextureId>,

    pub triangles: usize,

    _texture: Option<wgpu::Texture>,
}

impl Thumbnail {
    pub(crate) fn empty() -> Self {
        Self {
            id: None,
            triangles: 0,
            _texture: None,
        }
    }

    pub(crate) fn rendered(id: egui::TextureId, triangles: usize, texture: wgpu::Texture) -> Self {
        Self {
            id: Some(id),
            triangles,
            _texture: Some(texture),
        }
    }
}

pub struct ThumbnailRenderer {
    scratch: Option<ViewportRenderer>,
    colour_format: wgpu::TextureFormat,

    cache: HashMap<ThumbKey, Thumbnail>,
    lru: VecDeque<ThumbKey>,
    pending: Vec<ThumbKey>,
    /// Geometry handed in with its request, for a model no open document holds.
    supplied: HashMap<ThumbKey, Vec<MeshUpload>>,
    /// The tiles whose meshes a worker is building, and where it sends them.
    building: HashSet<ThumbKey>,
    built: (Sender<Built>, Receiver<Built>),
}

impl ThumbnailRenderer {
    pub fn new(colour_format: wgpu::TextureFormat) -> Self {
        Self {
            scratch: None,
            colour_format,
            cache: HashMap::new(),
            lru: VecDeque::new(),
            pending: Vec::new(),
            supplied: HashMap::new(),
            building: HashSet::new(),
            built: channel(),
        }
    }

    /// The cached tiles and everything the scratch renderer holds.
    pub fn bytes(&self) -> u64 {
        let tiles: u64 = self
            .cache
            .values()
            .filter_map(|thumb| thumb._texture.as_ref())
            .map(or_renderer::viewport::texture_bytes)
            .sum();
        tiles
            + self.scratch.as_ref().map_or(0, |scratch| {
                let r = scratch.resident_bytes();
                scratch.target_bytes()
                    + r.line_buffers
                    + r.particle_instance_buffers
                    + r.uniform_buffers
                    + r.default_textures
            })
    }

    pub fn get(&mut self, key: &ThumbKey) -> Option<&Thumbnail> {
        if !self.cache.contains_key(key) {
            return None;
        }
        if let Some(at) = self.lru.iter().position(|k| k == key) {
            let k = self.lru.remove(at).expect("index came from position");
            self.lru.push_back(k);
        }
        self.cache.get(key)
    }

    pub fn request(&mut self, key: ThumbKey) {
        if self.cache.contains_key(&key) || self.pending.contains(&key) {
            return;
        }
        self.pending.push(key);
    }

    /// Ask for a tile of `uploads`, which no document has to build.
    pub fn request_with(&mut self, key: ThumbKey, uploads: Vec<MeshUpload>) {
        if self.cache.contains_key(&key) || self.pending.contains(&key) {
            return;
        }
        self.supplied.insert(key, uploads);
        self.pending.push(key);
    }

    /// Whether the tile under `key` is waiting to be drawn.
    pub fn is_pending(&self, key: &ThumbKey) -> bool {
        self.pending.contains(key)
    }

    /// The tile to draw this frame, the first asked whose meshes are here, with them; `None` as meshes when it is built from the state, on this thread.
    ///
    /// ⚠ One a frame: every tile of a frame is recorded into one encoder, and each writes the
    /// scratch renderer's camera through the queue, which lands before any of them runs, so a
    /// second tile drew with the last tile's camera and showed nothing of its own model.
    ///
    /// `job` makes what builds a tile's meshes on a worker. ⚠ A tile waits its turn behind no
    /// other's build: building each on the UI thread, a tile a frame, stuttered every frame a
    /// scroll brought models into view.
    pub(crate) fn next_to_draw(
        &mut self,
        mut job: impl FnMut(&ThumbKey) -> Option<TileJob>,
    ) -> Option<(ThumbKey, Option<Vec<MeshUpload>>)> {
        for (key, uploads) in self.built.1.try_iter() {
            self.building.remove(&key);
            // A tile forgotten while it was built is of a document that closed.
            if self.pending.contains(&key) {
                self.supplied.insert(key, uploads);
            }
        }
        for at in 0..self.pending.len() {
            let key = self.pending[at];
            if let Some(uploads) = self.supplied.remove(&key) {
                self.pending.remove(at);
                return Some((key, Some(uploads)));
            }
            if self.building.contains(&key) || self.building.len() >= BUILDING {
                continue;
            }
            let Some(build) = job(&key) else {
                self.pending.remove(at);
                return Some((key, None));
            };
            self.building.insert(key);
            let tx = self.built.0.clone();
            std::thread::spawn(move || {
                let _ = tx.send((key, build()));
            });
        }
        None
    }

    pub(crate) fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub(crate) fn render_one(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        draws: Vec<MeshDraw>,
        camera: &OrbitCamera,
        mode: ShadingMode,
    ) -> wgpu::Texture {
        let scratch = self.scratch.get_or_insert_with(|| {
            ViewportRenderer::new(device, queue, self.colour_format, TILE_PX, TILE_PX)
        });
        scratch.resize(device, TILE_PX, TILE_PX);
        scratch.set_meshes(draws);
        scratch.render(encoder, queue, camera, mode);

        let tile = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gallery-model-thumbnail"),
            size: wgpu::Extent3d {
                width: TILE_PX,
                height: TILE_PX,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.colour_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        encoder.copy_texture_to_texture(
            scratch.colour_texture().as_image_copy(),
            tile.as_image_copy(),
            wgpu::Extent3d {
                width: TILE_PX,
                height: TILE_PX,
                depth_or_array_layers: 1,
            },
        );
        tile
    }

    pub(crate) fn insert(
        &mut self,
        key: ThumbKey,
        thumb: Thumbnail,
        egui_renderer: &mut egui_wgpu::Renderer,
    ) {
        while self.lru.len() >= CAPACITY {
            let Some(oldest) = self.lru.pop_front() else {
                break;
            };
            if let Some(evicted) = self.cache.remove(&oldest) {
                if let Some(id) = evicted.id {
                    egui_renderer.free_texture(&id);
                }
            }
        }
        self.cache.insert(key, thumb);
        self.lru.push_back(key);
    }

    pub fn forget_document(&mut self, doc: DocumentId, egui_renderer: &mut egui_wgpu::Renderer) {
        self.cache.retain(|key, thumb| {
            if key.doc == doc {
                if let Some(id) = thumb.id {
                    egui_renderer.free_texture(&id);
                }
                false
            } else {
                true
            }
        });
        self.lru.retain(|key| key.doc != doc);
        self.pending.retain(|key| key.doc != doc);
        self.supplied.retain(|key, _| key.doc != doc);
        self.building.retain(|key| key.doc != doc);
    }
}

pub(crate) fn bounding_sphere(uploads: &[MeshUpload]) -> Option<(glam::Vec3, f32)> {
    let mut min = glam::Vec3::splat(f32::INFINITY);
    let mut max = glam::Vec3::splat(f32::NEG_INFINITY);
    let mut any = false;
    for upload in uploads {
        for vertex in &upload.vertices {
            let p = glam::Vec3::from(vertex.position);
            if !p.is_finite() {
                continue;
            }
            min = min.min(p);
            max = max.max(p);
            any = true;
        }
    }
    if !any {
        return None;
    }
    let centre = (min + max) * 0.5;
    let radius = ((max - min) * 0.5).length().max(0.01);
    Some((centre, radius))
}

fn tile_camera(centre: glam::Vec3, radius: f32) -> OrbitCamera {
    let mut camera = OrbitCamera {
        yaw: TILE_YAW,
        pitch: std::f32::consts::FRAC_PI_8,
        ..OrbitCamera::default()
    };
    camera.frame_sphere(centre, radius);
    camera
}

pub fn geometry_for(
    state: &crate::state::AppState,
    doc: &crate::state::Document,
    selection: u64,
) -> Vec<MeshUpload> {
    use crate::state::capabilities::{self, GeometryRequest};

    let request = GeometryRequest {
        state,
        doc,
        selection: Some(selection),
        entry: None,
    };
    capabilities::for_payload(&doc.payload)
        .and_then(|row| row.meshes)
        .map(|build| build(&request))
        .unwrap_or_default()
}

pub fn camera_for(uploads: &[MeshUpload]) -> Option<OrbitCamera> {
    let (centre, radius) = bounding_sphere(uploads)?;
    Some(tile_camera(centre, radius))
}

pub fn tile_mode(viewport_mode: ShadingMode) -> ShadingMode {
    match viewport_mode {
        ShadingMode::Wireframe => ShadingMode::Solid,
        ShadingMode::Shaded => ShadingMode::Material,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_renderer::MeshVertex;

    fn upload(positions: &[[f32; 3]]) -> MeshUpload {
        MeshUpload {
            bump: false,
            vertices: positions
                .iter()
                .map(|&p| MeshVertex::new(p, [0.0, 0.0, 1.0], [0.0, 0.0]))
                .collect(),
            indices: (0..positions.len() as u32).collect(),
            channels: Vec::new(),
            texture_name: None,
            skinning: None,
            palette_bones: Vec::new(),
            source: None,
        }
    }

    #[test]
    fn the_bounding_sphere_encloses_the_whole_box() {
        let meshes = [upload(&[[-2.0, -1.0, 0.0], [2.0, 1.0, 0.0]])];
        let (centre, radius) = bounding_sphere(&meshes).expect("some geometry");
        assert_eq!(centre, glam::Vec3::ZERO);
        assert!((radius - 5f32.sqrt()).abs() < 1e-5, "radius was {radius}");
    }

    #[test]
    fn empty_geometry_has_no_camera() {
        assert!(bounding_sphere(&[]).is_none());
        assert!(camera_for(&[]).is_none());
    }

    #[test]
    fn a_non_finite_vertex_is_skipped() {
        let meshes = [upload(&[
            [-1.0, -1.0, -1.0],
            [f32::NAN, 0.0, 0.0],
            [1.0, 1.0, 1.0],
        ])];
        let (centre, radius) = bounding_sphere(&meshes).expect("the finite vertices still bound");
        assert_eq!(centre, glam::Vec3::ZERO);
        assert!((radius - 3f32.sqrt()).abs() < 1e-5, "radius was {radius}");
    }

    #[test]
    fn a_cached_tile_is_not_re_requested() {
        let mut thumbs = ThumbnailRenderer::new(wgpu::TextureFormat::Rgba8UnormSrgb);
        let key = ThumbKey {
            doc: 1,
            selection: 7,
            payload_epoch: 0,
            ..ThumbKey::default()
        };

        thumbs.request(key);
        thumbs.request(key);
        assert_eq!(thumbs.pending.len(), 1, "a duplicate request is dropped");

        thumbs.cache.insert(key, Thumbnail::empty());
        thumbs.pending.clear();
        thumbs.request(key);
        assert!(
            thumbs.pending.is_empty(),
            "a cached tile is not re-requested"
        );
    }

    #[test]
    fn a_tile_no_worker_builds_is_handed_back_at_once_and_one_a_frame() {
        let mut thumbs = ThumbnailRenderer::new(wgpu::TextureFormat::Rgba8UnormSrgb);
        for selection in 0..4 {
            thumbs.request(ThumbKey {
                doc: 1,
                selection,
                payload_epoch: 0,
                ..ThumbKey::default()
            });
        }
        let (key, meshes) = thumbs.next_to_draw(|_| None).expect("the first tile asked");
        assert_eq!(key.selection, 0);
        assert!(meshes.is_none(), "its meshes are built from the state");
        assert_eq!(thumbs.pending.len(), 3, "the rest stay queued");
        assert!(thumbs.has_pending());
    }

    #[test]
    fn a_tile_a_worker_builds_is_drawn_once_its_meshes_arrive_and_holds_no_other_back() {
        let mut thumbs = ThumbnailRenderer::new(wgpu::TextureFormat::Rgba8UnormSrgb);
        let key = |selection| ThumbKey {
            doc: 1,
            selection,
            payload_epoch: 0,
            ..ThumbKey::default()
        };
        thumbs.request(key(0));
        thumbs.request(key(1));
        // The first tile's build waits to be let go; the second has none and is drawn meanwhile.
        let (go, wait) = channel::<()>();
        let mut wait = Some(wait);
        let mut job = |key: &ThumbKey| -> Option<TileJob> {
            let wait = wait.take().filter(|_| key.selection == 0)?;
            Some(Box::new(move || {
                let _ = wait.recv();
                Vec::new()
            }))
        };
        let (drawn, meshes) = thumbs.next_to_draw(&mut job).expect("the tile behind it");
        assert_eq!((drawn.selection, meshes.is_none()), (1, true));
        assert!(
            thumbs.next_to_draw(&mut job).is_none(),
            "its meshes are not here yet"
        );

        go.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let (drawn, meshes) = loop {
            if let Some(next) = thumbs.next_to_draw(&mut job) {
                break next;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the worker never sent its meshes"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        assert_eq!((drawn.selection, meshes.is_some()), (0, true));
        assert!(!thumbs.has_pending());
    }

    #[test]
    fn closing_a_document_forgets_its_queued_tiles() {
        let mut thumbs = ThumbnailRenderer::new(wgpu::TextureFormat::Rgba8UnormSrgb);
        for doc in [1u64, 2] {
            thumbs.request(ThumbKey {
                doc,
                selection: 1,
                payload_epoch: 0,
                ..ThumbKey::default()
            });
        }
        thumbs.pending.retain(|key| key.doc != 1);
        assert_eq!(thumbs.pending.len(), 1);
        assert_eq!(thumbs.pending[0].doc, 2);
    }

    #[test]
    #[ignore = "needs a GPU adapter: acquisition can hang, not just fail"]
    fn a_rendered_tile_and_the_scratch_targets_are_counted() {
        let Some((device, queue)) =
            crate::render::test_device("a_rendered_tile_and_the_scratch_targets_are_counted")
        else {
            return;
        };
        let mut thumbs = ThumbnailRenderer::new(wgpu::TextureFormat::Rgba8UnormSrgb);
        assert_eq!(thumbs.bytes(), 0);

        let mut encoder = device.create_command_encoder(&Default::default());
        let tile = thumbs.render_one(
            &device,
            &queue,
            &mut encoder,
            Vec::new(),
            &OrbitCamera::default(),
            ShadingMode::Solid,
        );
        queue.submit(std::iter::once(encoder.finish()));
        let scratch = thumbs.bytes();
        assert!(scratch > 0, "the scratch renderer's targets are counted");

        let tile_bytes = or_renderer::viewport::texture_bytes(&tile);
        let key = ThumbKey {
            doc: 1,
            selection: 1,
            payload_epoch: 0,
            ..ThumbKey::default()
        };
        thumbs
            .cache
            .insert(key, Thumbnail::rendered(egui::TextureId::User(0), 0, tile));
        assert_eq!(thumbs.bytes(), scratch + tile_bytes);
    }
}
