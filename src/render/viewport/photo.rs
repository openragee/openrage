//! The thumbnails and the photographs the bridge draws of a model, each in a render of its own.

use super::*;

/// How bright the fill light of a photograph is, as a share of the sun.
pub(super) const FILL: f32 = 0.5;

/// The sRGB bytes in linear light, and the bytes of the linear values at sixty-five thousand steps of light.
///
/// ⚠ The exact formula cost a power per channel of every pixel, which was most of a picture's time; a linear value is looked up at a
/// step of 1/65535, which moves a byte by a fortieth of a level at the most.
struct Srgb {
    linear: [f32; 256],
    encoded: Vec<u8>,
}

fn srgb() -> &'static Srgb {
    static TABLES: std::sync::OnceLock<Srgb> = std::sync::OnceLock::new();
    TABLES.get_or_init(|| Srgb {
        linear: std::array::from_fn(|byte| exact_linear(byte as u8)),
        encoded: (0..=65535u32)
            .map(|step| exact_encoded(step as f32 / 65535.0))
            .collect(),
    })
}

fn exact_linear(c: u8) -> f32 {
    let c = f32::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn exact_encoded(l: f32) -> u8 {
    let l = l.clamp(0.0, 1.0);
    let c = if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    };
    (c * 255.0).round() as u8
}

/// `fill`'s colour added to `key`'s as light adds, in linear values; `key`'s alpha kept.
pub(super) fn add_light(mut key: image::RgbaImage, fill: image::RgbaImage) -> image::RgbaImage {
    let tables = srgb();
    for (k, f) in key.pixels_mut().zip(fill.pixels()) {
        for channel in 0..3 {
            let sum =
                tables.linear[usize::from(k.0[channel])] + tables.linear[usize::from(f.0[channel])];
            k.0[channel] = tables.encoded[(sum.clamp(0.0, 1.0) * 65535.0).round() as usize];
        }
    }
    key
}

impl RenderBridge {
    pub fn thumbnail(
        &mut self,
        key: crate::render::thumbnails::ThumbKey,
    ) -> Option<&crate::render::thumbnails::Thumbnail> {
        self.thumbnails.get(&key)
    }

    pub fn request_thumbnail(&mut self, key: crate::render::thumbnails::ThumbKey) {
        self.thumbnails.request(key);
    }

    /// Ask for a tile of geometry no open document holds, such as a model inside an archive.
    pub fn request_thumbnail_with(
        &mut self,
        key: crate::render::thumbnails::ThumbKey,
        uploads: Vec<MeshUpload>,
    ) {
        self.thumbnails.request_with(key, uploads);
    }

    pub fn thumbnail_pending(&self, key: crate::render::thumbnails::ThumbKey) -> bool {
        self.thumbnails.is_pending(&key)
    }

    pub fn thumbnail_key(
        &self,
        state: &AppState,
        doc: &crate::state::Document,
        selection: u64,
    ) -> crate::render::thumbnails::ThumbKey {
        crate::render::thumbnails::ThumbKey::of(doc, selection).drawn_by(state, self.mode)
    }

    pub fn forget_document_thumbnails(
        &mut self,
        doc: DocumentId,
        egui_renderer: &mut egui_wgpu::Renderer,
    ) {
        self.thumbnails.forget_document(doc, egui_renderer);
    }

    pub fn render_thumbnails(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        state: &AppState,
        egui_renderer: &mut egui_wgpu::Renderer,
    ) {
        use crate::render::thumbnails::{camera_for, geometry_for, tile_mode, Thumbnail};

        if !self.thumbnails.has_pending() {
            return;
        }
        let device = self.device.clone();
        let queue = self.queue.clone();
        let mode = tile_mode(self.mode);

        let (lod, channel) = (state.preview_scene.lod, state.preview_scene.debug_channel);
        let next = self.thumbnails.next_to_draw(|key| {
            let payload = &state.document(key.doc)?.payload;
            crate::render::meshes::tile_job(payload, key.selection, lod, channel)
        });
        if let Some((key, built)) = next {
            let uploads = match built {
                Some(uploads) => uploads,
                None => {
                    let Some(doc) = state.document(key.doc) else {
                        return;
                    };
                    geometry_for(state, doc, key.selection)
                }
            };
            let triangles: usize = uploads.iter().map(|u| u.indices.len() / 3).sum();

            let Some(camera) = camera_for(&uploads) else {
                self.thumbnails
                    .insert(key, Thumbnail::empty(), egui_renderer);
                return;
            };

            let draws = self.upload_meshes(&uploads, state, Some(key.selection));
            let texture = self
                .thumbnails
                .render_one(&device, &queue, encoder, draws, &camera, mode);
            let view = texture.create_view(&Default::default());
            let id =
                egui_renderer.register_native_texture(&device, &view, wgpu::FilterMode::Linear);
            self.thumbnails.insert(
                key,
                Thumbnail::rendered(id, triangles, texture),
                egui_renderer,
            );
        }
    }

    /// `uploads` of `selection` drawn alone, textured, from `camera`, on a transparent `px`-square target, read back.
    ///
    /// ⚠ Draws with this bridge's own renderer, so it is for a bridge that has no viewport to show.
    pub fn photograph(
        &mut self,
        state: &AppState,
        uploads: &[MeshUpload],
        selection: u64,
        camera: &OrbitCamera,
        px: u32,
        game: Option<&[(Surface, GameMaterial)]>,
    ) -> Result<image::RgbaImage, String> {
        let draws = self.upload_meshes(uploads, state, Some(selection));
        self.shoot(draws, camera, px, Alpha::Drawn, game)
    }

    /// `draws` alone, textured, from `camera`, on a transparent `px`-square target, read back; its colour drawn by the game's own passes from `game` where there is a pack of them.
    ///
    /// ⚠ The alpha is taken as `alpha` says, and never from Material alone: Material writes a texture's
    /// own alpha, which a garment's diffuse uses for something else, and left a whole shirt half clear.
    pub(super) fn shoot(
        &mut self,
        draws: Vec<MeshDraw>,
        camera: &OrbitCamera,
        px: u32,
        alpha: Alpha,
        game: Option<&[(Surface, GameMaterial)]>,
    ) -> Result<image::RgbaImage, String> {
        self.renderer.resize(&self.device, px, px);
        self.renderer.set_background([0.0; 4]);
        self.renderer.set_meshes(draws);
        let colour = match (game, self.game_pack.clone()) {
            (Some(surfaces), Some(pack)) if !surfaces.is_empty() => {
                // A pass the device refuses is reported by its error scope and not by a panic
                let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
                let scene = crate::photo_timing::time("30 prepare game scene", || {
                    MeshScene::prepare_with(
                        &mut self.game_pipelines,
                        &self.device,
                        &self.queue,
                        &pack,
                        &self.renderer.game_target(),
                        surfaces,
                    )
                });
                if let Some(error) = pollster::block_on(validation.pop()) {
                    self.game_pipelines = Default::default();
                    return Err(format!("the game's passes did not prepare: {error}"));
                }
                self.renderer.set_game_scene(Some(scene));
                ShadingMode::Shaded
            }
            _ => ShadingMode::Material,
        };
        let mut picture =
            crate::photo_timing::time("31 key pass", || self.render_pass(camera, colour, px));
        if colour == ShadingMode::Shaded {
            picture = picture.and_then(|key| {
                let fill =
                    crate::photo_timing::time("32 fill pass", || self.fill_pass(camera, px))?;
                Ok(crate::photo_timing::time("33 add light", || {
                    add_light(key, fill)
                }))
            });
        }
        let covered = crate::photo_timing::time("34 solid pass", || {
            self.render_pass(camera, ShadingMode::Solid, px)
        });
        // ⚠ A cut is the textures' alpha as Material draws it: the game's passes for a ped write none.
        let kept = match (alpha, game) {
            (Alpha::Cut, _) if colour == ShadingMode::Shaded => {
                Some(crate::photo_timing::time("35 kept pass", || {
                    self.render_pass(camera, ShadingMode::Material, px)
                }))
            }
            (Alpha::Worn, Some(surfaces)) => {
                let draws =
                    crate::photo_timing::time("35 coverage draws", || self.coverage_of(surfaces));
                self.renderer.set_meshes(draws);
                Some(crate::photo_timing::time("35 kept pass", || {
                    self.render_pass(camera, ShadingMode::Material, px)
                }))
            }
            _ => None,
        };
        self.renderer.set_game_scene(None);
        let (mut picture, covered, kept) = (picture?, covered?, kept.transpose()?);
        for (at, (pixel, cover)) in picture.pixels_mut().zip(covered.pixels()).enumerate() {
            let drawn = pixel.0[3];
            let kept = kept.as_ref().map_or(drawn, |k| k.as_raw()[at * 4 + 3]);
            pixel.0[3] = match alpha {
                Alpha::Drawn if colour == ShadingMode::Shaded => drawn.min(cover.0[3]),
                Alpha::Cut => kept.min(cover.0[3]),
                Alpha::Worn if game.is_some() => kept,
                Alpha::Drawn | Alpha::Worn => cover.0[3],
            };
        }
        Ok(picture)
    }

    /// The game's passes lit by the fill alone: a light from the camera's other side and a little below, a half of the sun, and no ambient.
    ///
    /// ⚠ The passes are lit by one sun, and what the key leaves in shadow, a ring's inside and a
    /// watch's crown, was black: the fill is a second frame, added to the first as light adds.
    pub(super) fn fill_pass(
        &mut self,
        camera: &OrbitCamera,
        px: u32,
    ) -> Result<image::RgbaImage, String> {
        let (sun, ambient) = self.renderer.game_light();
        let back = (camera.eye() - camera.target).normalize_or(glam::Vec3::Y);
        let right = OrbitCamera::WORLD_UP
            .cross(back)
            .normalize_or(glam::Vec3::X);
        let fill = (back - right * 0.8 - OrbitCamera::WORLD_UP * 0.15).normalize_or(back);
        self.renderer.set_game_light(sun * FILL, glam::Vec3::ZERO);
        self.renderer.set_game_sun(Some(fill));
        let frame = self.render_pass(camera, ShadingMode::Shaded, px);
        self.renderer.set_game_sun(None);
        self.renderer.set_game_light(sun, ambient);
        frame
    }

    /// One frame of what is set, drawn in `mode` and read back.
    pub(super) fn render_pass(
        &self,
        camera: &OrbitCamera,
        mode: ShadingMode,
        px: u32,
    ) -> Result<image::RgbaImage, String> {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.renderer
            .render(&mut encoder, &self.queue, camera, mode);
        self.queue.submit(std::iter::once(encoder.finish()));
        crate::render::read_back::rgba(
            &self.device,
            &self.queue,
            self.renderer.colour_texture(),
            px,
            px,
        )
    }

    /// `surfaces` as meshes whose texture's alpha is what each covers: its diffuse's where its material blends or cuts it out, whole where the material is opaque.
    pub(super) fn coverage_of(&self, surfaces: &[(Surface, GameMaterial)]) -> Vec<MeshDraw> {
        use or_renderer::game_shaders::mesh::Coverage;
        let mut draws = Vec::new();
        for (surface, material) in surfaces {
            let uvs = surface.uvs.get(&0);
            let vertices: Vec<or_renderer::MeshVertex> = (surface.positions.iter().enumerate())
                .map(|(at, p)| {
                    let normal = surface.normals.get(at).copied().unwrap_or([0.0, 0.0, 1.0]);
                    let uv = uvs.and_then(|u| u.get(at)).copied().unwrap_or([0.0; 2]);
                    or_renderer::MeshVertex::new(*p, normal, uv)
                })
                .collect();
            if vertices.is_empty() || surface.indices.is_empty() {
                continue;
            }
            let texture = material.textures.get("DiffuseSampler").map(|image| {
                let mut rgba = image.rgba.clone();
                if material.coverage == Coverage::Opaque {
                    for texel in rgba.as_chunks_mut::<4>().0 {
                        texel[3] = u8::MAX;
                    }
                }
                let (group, _) = texture_upload::upload_rgba(
                    &self.device,
                    &self.queue,
                    &self.material_bgl,
                    (image.width, image.height),
                    &rgba,
                    image.srgb,
                );
                Arc::new(group)
            });
            draws.push(MeshDraw {
                bump: false,
                vertex_buffer: self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("photo-coverage-vb"),
                        contents: bytemuck::cast_slice(&vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                index_buffer: self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("photo-coverage-ib"),
                        contents: bytemuck::cast_slice(&surface.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    }),
                index_count: surface.indices.len() as u32,
                texture_bind_group: texture,
            });
        }
        draws
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_light_table_is_the_formula_to_within_a_level() {
        let tables = srgb();
        let mut worst = 0i32;
        for key in (0..=255u16).step_by(5) {
            for fill in (0..=255u16).step_by(5) {
                let sum = exact_linear(key as u8) + exact_linear(fill as u8);
                let by_table = tables.encoded[(sum.clamp(0.0, 1.0) * 65535.0).round() as usize];
                worst = worst.max((i32::from(by_table) - i32::from(exact_encoded(sum))).abs());
            }
        }
        assert!(worst <= 1, "the table is {worst} levels from the formula");
    }

    #[test]
    fn a_fill_adds_its_light_as_light_adds_and_keeps_the_key_alpha() {
        let pixel = |rgba: [u8; 4]| image::RgbaImage::from_pixel(1, 1, image::Rgba(rgba));
        let lit = add_light(pixel([188, 0, 255, 77]), pixel([188, 0, 255, 200]));
        let [r, g, b, a] = lit.get_pixel(0, 0).0;
        // Half the light twice is all of it: 188 is a half of linear light in sRGB
        assert!(r >= 254, "two halves of light are white: {r}");
        assert_eq!(
            (g, b, a),
            (0, 255, 77),
            "none adds none, white stays white, and alpha is the key's"
        );
    }
}
