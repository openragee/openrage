//! The game's deferred frame of a viewport: what it is given to light and draw, and the day, the sky and the tone it is drawn with.

use super::*;

impl ViewportRenderer {
    /// Prepare `surfaces` for the game's deferred passes, which Shaded draws in place of the scene [`Self::set_game_scene`] sets; how many are drawn, and the effect of each left out with why.
    ///
    /// The passes that light and map the frame are built the first time and kept for the scenes that follow.
    pub fn prepare_deferred(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pack: &crate::game_shaders::pack::GamePack,
        surfaces: &[(
            crate::game_shaders::mesh::Surface,
            crate::game_shaders::mesh::Material,
        )],
    ) -> Result<(usize, Vec<(String, String)>), String> {
        use crate::game_shaders::deferred::{
            hdr_target, ldr_target, DeferredScene, DepthViews, FramePasses, GBuffer, Inputs,
            ReflectionTarget, HDR_FORMAT,
        };
        if self.deferred.is_none() {
            let gbuffer = GBuffer::new(device, self.width, self.height);
            let depth = DepthViews::of(self.depth_view.texture());
            let hdr = hdr_target(device, self.width, self.height);
            let ldr = ldr_target(
                device,
                self.colour_format.remove_srgb_suffix(),
                self.width,
                self.height,
            );
            let reflection = ReflectionTarget::new(device, REFLECTION_WIDTH, REFLECTION_LEVELS);
            let passes = FramePasses::new(
                device,
                queue,
                pack,
                HDR_FORMAT,
                self.colour_format.remove_srgb_suffix(),
                &Inputs {
                    gbuffer: &gbuffer,
                    depth: &depth,
                    hdr: &hdr,
                    reflection: reflection.view(),
                    ldr: &ldr,
                },
            )
            .map_err(|e| e.to_string())?;
            let mut passes = passes;
            passes.set_sky_textures(self.sky_textures.clone());
            self.deferred = Some(DeferredFrame {
                passes,
                gbuffer,
                depth,
                hdr,
                ldr,
                reflection,
                reflection_of: None,
                stale: false,
                refraction: None,
                fog_sink: None,
                meter: super::meter::Meter::new(device),
                post: super::post::Post::new(device, self.colour_format.remove_srgb_suffix()),
                grade: crate::game_shaders::grade::GradePass::new(device),
            });
        }
        self.rebind_deferred(device, queue);
        // Water reads the lit frame behind it and how far that lies, which are made as the frame is drawn
        let has_water = surfaces.iter().any(|(_, m)| is_water(m));
        self.refracts = has_water;
        self.refraction_stale = false;
        let dressed;
        let surfaces = if has_water {
            dressed = self.dress_water(device, queue, surfaces);
            dressed.as_slice()
        } else {
            surfaces
        };
        let scene = DeferredScene::prepare(
            device,
            queue,
            pack,
            &mut self.deferred_pipelines,
            HDR_FORMAT,
            surfaces,
        );
        let report = scene.report();
        self.deferred_scene = Some(scene);
        Ok(report)
    }

    /// `surfaces` with what water reads given to the materials that are water: a copy of the lit frame behind it, how far each pixel of that lies, and a sky for what it reflects.
    pub(super) fn dress_water(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surfaces: &[(
            crate::game_shaders::mesh::Surface,
            crate::game_shaders::mesh::Material,
        )],
    ) -> Vec<(
        crate::game_shaders::mesh::Surface,
        crate::game_shaders::mesh::Material,
    )> {
        use crate::game_shaders::stand_in::constant_view;
        let size = (self.width, self.height);
        let frame = self
            .deferred
            .as_mut()
            .expect("the frame is made before its water is dressed");
        let refraction = frame
            .refraction
            .get_or_insert_with(|| {
                crate::game_shaders::targets::render_target(
                    device,
                    "refraction",
                    size,
                    crate::game_shaders::deferred::HDR_FORMAT,
                    // Copied into by the frame, and out of by a test that reads what the water reads
                    wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
                )
            })
            .clone();
        frame.fog_sink.get_or_insert_with(|| {
            crate::game_shaders::targets::render_target(
                device,
                "water-fog-sink",
                size,
                crate::game_shaders::deferred::GBUFFER_FORMAT,
                wgpu::TextureUsages::empty(),
            )
        });
        // Where the pack has no pass that makes the depth, everything lies fifty metres off
        let depth = (frame.passes.half_depth().cloned())
            .unwrap_or_else(|| constant_view(device, queue, [50.0, 0.0, 0.0, 0.0]));
        // What the water reflects is the map of the sky the frame is lit by, which a day writes again into the same texture
        let sky = constant_view(device, queue, [2.0, 3.0, 4.5, 1.0]);
        let mut inputs = vec![
            ("RefractionSampler", refraction),
            ("RefractionDepthSampler", depth),
            ("ReflectionSampler", frame.reflection.view().clone()),
            ("PlanarReflectionSampler", sky),
            // What the fog pass writes beside the colour: how thick the water lies over what is opaque
            (
                "LightingSampler",
                frame.fog_sink.as_ref().expect("the sink is made").clone(),
            ),
            // How much a ray meets the surface at an angle reflects, and the foam that shows: a curve the game draws once
            (
                "BlendSampler",
                crate::game_shaders::stand_in::row_view(
                    device,
                    queue,
                    &crate::game_shaders::water::blend_row(),
                ),
            ),
        ];
        // The ripples and the foam of the install's own dictionary; the game reads the second bump for both where the water is small on the screen
        let uploaded = |image: &crate::game_shaders::mesh::Image| {
            crate::game_shaders::stand_in::uploaded_view(
                device,
                queue,
                image.width,
                image.height,
                &image.rgba,
                image.srgb,
            )
        };
        let maps = &self.water_textures;
        if let Some(bump) = &maps.bump {
            inputs.push(("StaticBumpSampler", uploaded(bump)));
        }
        if let Some(bump) = &maps.bump2 {
            inputs.push(("WaterBumpSampler", uploaded(bump)));
            inputs.push(("WaterBumpSampler2", uploaded(bump)));
        }
        if let Some(foam) = &maps.foam {
            inputs.push(("StaticFoamSampler", uploaded(foam)));
        }
        surfaces
            .iter()
            .map(|(surface, material)| {
                let mut material = material.clone();
                if is_water(&material) {
                    for (name, view) in &inputs {
                        material.views.insert((*name).to_string(), view.clone());
                    }
                }
                (surface.clone(), material)
            })
            .collect()
    }

    /// Whether the scene holds water that reads a copy of the frame made before the targets were made anew, so that it is to be prepared again.
    pub fn deferred_needs_prepare(&self) -> bool {
        self.refraction_stale
    }

    /// Take the deferred scene away.
    pub fn clear_deferred(&mut self) {
        self.refracts = false;
        self.refraction_stale = false;
        self.deferred_scene = None;
        let (gbuffer, blended, shadow, fog) = &mut self.deferred_pipelines;
        for cache in [gbuffer, blended, shadow, fog] {
            cache.forget_textures();
        }
    }

    pub fn has_deferred(&self) -> bool {
        self.deferred_scene.is_some()
    }

    /// The targets of the G-buffer the deferred frame fills, for a test to read what its passes wrote.
    #[cfg(test)]
    pub(crate) fn gbuffer_textures(&self) -> Vec<&wgpu::Texture> {
        let frame = self.deferred.iter();
        frame.flat_map(|frame| frame.gbuffer.textures()).collect()
    }

    /// Forget what was built for the deferred passes, such as pipelines the device refused or ones made for another pack.
    pub fn reset_deferred_pipelines(&mut self) {
        self.refracts = false;
        self.refraction_stale = false;
        self.deferred_pipelines = Default::default();
        self.deferred = None;
        self.deferred_scene = None;
    }

    /// Make the deferred passes read the targets again where they were made anew.
    fn rebind_deferred(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let Some(frame) = self.deferred.as_mut().filter(|frame| frame.stale) else {
            return;
        };
        frame.passes.bind(
            device,
            queue,
            &crate::game_shaders::deferred::Inputs {
                gbuffer: &frame.gbuffer,
                depth: &frame.depth,
                hdr: &frame.hdr,
                reflection: frame.reflection.view(),
                ldr: &frame.ldr,
            },
        );
        frame.stale = false;
    }

    /// Read the stages the device finished timing, where it can, and whether it read a frame; `prepare_game_frame` does as well.
    pub fn collect_gpu_times(&self, device: &wgpu::Device) -> bool {
        self.gpu_timer
            .as_ref()
            .is_some_and(|timer| timer.collect(device))
    }

    /// Make the deferred passes read the targets again where they were made anew, and the map of the sky show the sky of the day; call it before the frame is drawn.
    pub fn prepare_game_frame(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let _ = self.collect_gpu_times(device);
        self.rebind_deferred(device, queue);
        let sky = self.sky();
        let revision = self.day_revision;
        let Some(frame) = self.deferred.as_mut() else {
            return;
        };
        frame.meter.collect(device);
        frame.meter.bind(device, &frame.hdr);
        match self.day.as_ref().is_some_and(|day| day.post.is_some()) {
            true => {
                let depth = (&frame.depth.depth, &frame.depth.stencil);
                (frame.post).prepare(device, &frame.hdr, depth, &frame.ldr);
                frame.grade.prepare(device, &frame.hdr);
            }
            false => frame.post.forget(),
        }
        if frame.reflection_of != Some(revision) {
            use crate::game_shaders::sky::ReflectionMap;
            let (width, levels) = (REFLECTION_WIDTH, REFLECTION_LEVELS as usize);
            let map = match self.day.as_ref().and_then(|d| d.sky_view.as_ref()) {
                Some(view) => ReflectionMap::of_colours(
                    &|d| sky.colour_over(d, view.sample(d)),
                    width,
                    levels,
                ),
                None => ReflectionMap::of(&sky, width, levels),
            };
            frame.reflection.write(queue, &map);
            frame.reflection_of = Some(revision);
        }
        self.world.bind_sky(device, frame.reflection.view());
        // The world's water reads a copy of the lit frame, which is made when the frame is
        if self.world.water.drawn() {
            let size = (self.width, self.height);
            let behind = frame.refraction.get_or_insert_with(|| {
                crate::game_shaders::targets::render_target(
                    device,
                    "refraction",
                    size,
                    crate::game_shaders::deferred::HDR_FORMAT,
                    wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
                )
            });
            let water = &mut self.world.water;
            water.upload(queue);
            water.bind(
                device,
                behind,
                frame.passes.half_depth(),
                frame.reflection.view(),
                frame.passes.shadow_map().as_ref(),
            );
        }
    }

    /// The lit frame's luminance as last read back, where that frame was drawn under the day set now.
    pub fn metered(&self) -> Option<Metered> {
        (self.deferred.as_ref()?.meter).last(self.day_revision)
    }

    /// Forget the temporal history and start its camera offsets from their first.
    pub fn restart_temporal(&self) {
        if let Some(frame) = &self.deferred {
            frame.post.restart();
        }
    }

    /// End the stage `label` for the GPU timer, where the device has one.
    pub(super) fn gpu_mark(&self, encoder: &mut wgpu::CommandEncoder, label: &'static str) {
        if let Some(timer) = &self.gpu_timer {
            timer.mark(encoder, label);
        }
    }

    /// The sky of the day the frame is lit by, which the surfaces reflect; a black one where there is no day.
    pub(super) fn sky(&self) -> crate::game_shaders::sky::Sky {
        use crate::game_shaders::lighting::View;
        use crate::game_shaders::sky::Sky;
        let Some(day) = &self.day else {
            return Sky::new(&Default::default(), Vec3::Z, Vec3::ZERO);
        };
        let lighting = day.lighting(&View {
            camera_height: 0.0,
            projection: Mat4::perspective_rh(1.0, 1.0, 0.1, 1000.0),
            near: 0.1,
        });
        // The light travels away from the sun, and the ground gives back some of what falls on it
        let toward_sun = -lighting.directional_light.truncate();
        let sun = lighting.directional_colour.truncate() * toward_sun.z.max(0.0);
        let ground = (sun + lighting.natural_ambient[1].truncate()) * 0.25;
        let (sun_sprite, moon_sprite) = lighting.sun_and_moon();
        Sky::new(&day.keyframe, toward_sun, ground)
            .placed(sun_sprite, moon_sprite)
            .with_time(day.cycle_time)
    }

    /// The deferred passes and the scene Shaded draws with, where both are made.
    pub(super) fn deferred_in(
        &self,
        mode: ShadingMode,
    ) -> Option<(
        &DeferredFrame,
        &crate::game_shaders::deferred::DeferredScene,
    )> {
        match mode {
            ShadingMode::Shaded => self.deferred.as_ref().zip(self.deferred_scene.as_ref()),
            _ => None,
        }
    }

    /// The colour of the sun and of the ambient light the game's passes are lit by; a clear day until set.
    pub fn set_game_light(&mut self, sun_colour: Vec3, ambient: Vec3) {
        self.game_light = (sun_colour, ambient);
    }

    /// The colour of the sun and of the ambient light the game's passes are lit by.
    pub fn game_light(&self) -> (Vec3, Vec3) {
        self.game_light
    }

    /// The way towards the sun the game's passes are lit from; none puts it over the camera's shoulder.
    /// Draw the game's own sky of the moment of the day where nothing else is drawn, in place of the background colour; it needs the passes of the game and a moment of the day.
    pub fn set_sky_background(&mut self, on: bool) {
        self.sky_background = on;
    }

    /// Whether the sun's shadow is cast and received by the game's passes; on until set.
    pub fn set_shadows(&mut self, on: bool) {
        self.shadows = on;
    }

    /// Whether the sun's shadow is cast and received by the game's passes.
    pub fn shadows(&self) -> bool {
        self.shadows
    }

    /// Whether the sky is what shows where nothing is drawn.
    pub(super) fn shows_sky(&self, deferred: &DeferredFrame) -> bool {
        self.sky_background && self.day.is_some() && deferred.passes.has_sky()
    }

    pub fn set_game_sun(&mut self, towards: Option<Vec3>) {
        self.game_sun = towards;
    }

    /// The moment of the day the game's passes are lit by, with the sun, the ambient light and the fog the game's own day gives it; none lights them by the colours and the sun set above.
    pub fn set_day(&mut self, day: Option<crate::game_shaders::lighting::Day>) {
        self.day = day;
        self.day_revision += 1;
    }

    /// How far ahead of the camera the sun's shadow of a world is cast by the day's cycle for what the camera rides, in metres.
    pub fn shadow_reach(&self) -> f32 {
        crate::world::shadow_reach(self.shadow_multiplier(), self.world.focus)
    }

    /// What the day's cycle multiplies the ends of the sun's cascades by; one with no day.
    fn shadow_multiplier(&self) -> f32 {
        let of = |day: &crate::game_shaders::lighting::Day| {
            day.keyframe.get_or(crate::world::DISTANCE_MULTIPLIER, 1.0)
        };
        self.day.as_ref().map_or(1.0, of)
    }

    /// The skin matrices of the pose, one for each bone of the palette of the scene; none where there is no pose or no skinned surface.
    fn posed_bones(&self) -> Option<Vec<Mat4>> {
        let scene = self.deferred_scene.as_ref()?;
        if self.skin.is_empty() || scene.palette().is_empty() {
            return None;
        }
        Some(
            scene
                .palette()
                .iter()
                .map(|bone| {
                    self.skin
                        .get(usize::from(*bone))
                        .copied()
                        .unwrap_or(Mat4::IDENTITY)
                })
                .collect(),
        )
    }

    /// Draw the skinned surfaces of the game's passes in the pose `skin` gives, one skin matrix for each bone of the skeleton; an empty pose draws them at rest.
    ///
    /// ⚠ Not a change of the targets: `take_dirty` says the colour texture was made again, which its owner registers anew, and a pose changes every frame of a clip.
    pub fn set_skin(&mut self, skin: Vec<Mat4>) {
        self.skin = skin;
    }

    /// GTA VI's passes after the light, where the day states them and Shaded draws the game's frame.
    fn gta6_post(&self) -> Option<&crate::game_shaders::tone::Gta6Post> {
        self.deferred.as_ref()?;
        self.day.as_ref()?.post.as_ref()
    }

    /// The camera's offset of this frame, as a change of the clip space: none unless `mode` draws GTA VI's frame, which is anti-aliased over time.
    pub(super) fn jitter(&self, mode: ShadingMode) -> Mat4 {
        match (mode, self.gta6_post(), &self.deferred) {
            (ShadingMode::Shaded, Some(_), Some(frame)) => {
                frame.post.jitter((self.width, self.height))
            }
            _ => Mat4::IDENTITY,
        }
    }

    /// The constants the game's shaders read, from the camera of this frame.
    pub(super) fn game_constants(
        &self,
        camera: &OrbitCamera,
        aspect: f32,
    ) -> crate::game_shaders::engine::EngineConstants {
        let projection = self.jitter(ShadingMode::Shaded)
            * perspective_reverse_z_rh(camera.fov_y, aspect.max(0.0001), camera.near, camera.far);
        let lighting = self.day.as_ref().map(|day| {
            day.lighting(&crate::game_shaders::lighting::View {
                camera_height: camera.eye().z,
                projection,
                near: camera.near,
            })
        });
        crate::game_shaders::engine::EngineConstants {
            world: Mat4::IDENTITY,
            view: camera.view(),
            projection,
            sun_direction: self.game_sun.unwrap_or_else(|| {
                crate::game_shaders::mesh::light_towards(
                    camera.eye(),
                    camera.target,
                    OrbitCamera::WORLD_UP,
                )
            }),
            sun_colour: self.game_light.0,
            ambient: self.game_light.1,
            lighting,
            bones: self.posed_bones(),
            screen: [self.width as f32, self.height as f32],
        }
    }

    /// The background as a target written through [`Self::colour_display_view`] takes it: such a view does not encode what is written, so a colour of an sRGB target is encoded here.
    pub(super) fn display_background(&self) -> wgpu::Color {
        if !self.colour_format.is_srgb() {
            return self.background;
        }
        let encode = |linear: f64| {
            if linear <= 0.003_130_8 {
                linear * 12.92
            } else {
                1.055 * linear.powf(1.0 / 2.4) - 0.055
            }
        };
        wgpu::Color {
            r: encode(self.background.r),
            g: encode(self.background.g),
            b: encode(self.background.b),
            a: self.background.a,
        }
    }

    /// The background as the target the frame is mapped to takes it, which holds the luminance of a pixel in its alpha for the anti-aliasing to read.
    pub(super) fn ldr_background(&self) -> wgpu::Color {
        let colour = self.display_background();
        wgpu::Color {
            a: 0.2125 * colour.r + 0.7154 * colour.g + 0.0721 * colour.b,
            ..colour
        }
    }

    /// What the day gives the colour of the fog the particles fill the air with.
    pub(super) fn fog_sky(
        &self,
        engine: &crate::game_shaders::engine::EngineConstants,
    ) -> crate::game_shaders::fog_volumes::FogSky {
        let (sun, height) = match &engine.lighting {
            Some(lighting) => (
                lighting.directional_colour.truncate(),
                -lighting.directional_light.z,
            ),
            None => (engine.sun_colour, engine.sun_direction.z),
        };
        let (day_fog, ground) = self.day.as_ref().map_or((1.0, glam::Vec3::ONE), |day| {
            (
                day.keyframe.get_or("fog_hdr", 1.0),
                glam::Vec3::new(
                    day.keyframe.get_or("fog_near_col_r", 1.0),
                    day.keyframe.get_or("fog_near_col_g", 1.0),
                    day.keyframe.get_or("fog_near_col_b", 1.0),
                ),
            )
        });
        crate::game_shaders::fog_volumes::FogSky {
            sun,
            sun_height: height,
            day_fog,
            ground,
        }
    }

    /// How the frame is mapped to the screen: by the day's own curves and exposure, or by the game's where there is no day.
    pub(super) fn tone_of(
        &self,
        engine: &crate::game_shaders::engine::EngineConstants,
    ) -> crate::game_shaders::tone::Tone {
        use crate::game_shaders::tone::{Tone, ToneSettings};
        match (&self.day, &engine.lighting) {
            (Some(day), Some(lighting)) => day.tone(lighting),
            _ => {
                let lighting = crate::game_shaders::lighting::Lighting {
                    directional_colour: engine.sun_colour.extend(0.0),
                    natural_ambient: [engine.ambient.extend(1.0); 2],
                    ..Default::default()
                };
                Tone::new(&ToneSettings::default(), None, &lighting)
            }
        }
    }

    /// The frame Shaded draws of a prepared scene: the surfaces into the G-buffer, the lighting passes over them, the sky, what blends, and the post chain.
    pub(super) fn render_deferred(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        camera: &OrbitCamera,
        aspect: f32,
        deferred: &DeferredFrame,
        scene: &crate::game_shaders::deferred::DeferredScene,
    ) {
        let engine = self.game_constants(camera, aspect);
        let occlusion = self.day.as_ref().map_or(0.0, |day| day.occlusion());
        scene.update(queue, &engine);
        let screen = crate::game_shaders::deferred::Screen {
            width: self.width,
            height: self.height,
            near: camera.near,
            far: camera.far,
            reflection_levels: deferred.reflection.levels(),
        };
        // What blends over the lit frame is lit by the frame's own sun and ambient light
        self.world.write_light(queue, &engine);
        // A world is shaded by cascades about what the camera sees, and a scene's own surfaces are then cast into the nearest of them
        let cascades = (self.shadows && deferred.passes.has_shadow())
            .then(|| {
                let multiplier = self.shadow_multiplier();
                (self.world).cascades(queue, &engine, camera, aspect, multiplier)
            })
            .flatten();
        // With no world, where the sun shines the drawable shades itself: its surfaces are cast from the sun, and the G-buffer keeps what reaches each pixel
        let shadow = scene
            .sphere()
            .filter(|_| self.shadows && cascades.is_none())
            .and_then(|sphere| deferred.passes.shadow_fit(&engine, sphere));
        if let Some(cascades) = &cascades {
            scene.update_shadow(queue, &cascades.caster());
            deferred.passes.write_shadow_with(
                queue,
                &engine,
                &screen,
                cascades.values(camera.eye()),
            );
        }
        if let Some(fit) = &shadow {
            scene.update_shadow(queue, &fit.caster());
            deferred.passes.write_shadow(queue, &engine, &screen, fit);
        }
        let tone = self.tone_of(&engine);
        deferred
            .passes
            .write(queue, &engine, &screen, &tone, occlusion);
        deferred
            .passes
            .write_point_lights(queue, &engine, &screen, &self.point_lights);
        deferred
            .passes
            .write_spot_lights(queue, &engine, &screen, &self.spot_lights);
        deferred
            .passes
            .write_capsule_lights(queue, &engine, &screen, &self.capsule_lights);
        let (look, distant) = &self.distant_lights;
        (deferred.passes).write_distant_lights(queue, &engine, &screen, (*look, distant));
        let coronas = (self.coronas.as_ref()).map(|(look, c)| (*look, c.as_slice()));
        let coronas = coronas.filter(|(_, c)| !c.is_empty());
        let none = (
            crate::CoronaLook {
                occlusion_share: 0.0,
                occlusion_pixels: 0.0,
                screen_edge: 0.0,
            },
            &[][..],
        );
        let has_coronas = coronas.is_some();
        (deferred.passes).write_coronas(queue, &engine, &screen, coronas.unwrap_or(none));
        deferred.passes.write_fog(
            queue,
            &engine,
            &screen,
            &self.fog_sky(&engine),
            &self.fog_volumes,
        );
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport-gbuffer-pass"),
                color_attachments: &deferred.gbuffer.attachments(),
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: Some(crate::game_shaders::deferred::stencil_clear()),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            scene.record_gbuffer(&mut pass);
            self.world.draw_gbuffer(&mut pass, &self.bind_group);
        }
        self.gpu_mark(encoder, "g-buffer");
        if shadow.is_some() || cascades.is_some() {
            deferred.passes.record_shadow_cast(encoder, |pass| {
                scene.record_shadow(pass);
                if cascades.is_some() {
                    self.world.draw_shadow(pass);
                }
            });
            deferred
                .passes
                .record_shadow_receive(encoder, &deferred.gbuffer, &self.depth_view);
            self.gpu_mark(encoder, "shadows");
        }
        // Past the cascades the terrain alone shades, as the game's frame does.
        if let (Some(terrain), true) = (&self.world.terrain, self.shadows) {
            let fade = (cascades.as_ref()).map_or([0.0; 2], |c| c.fade());
            let day = |name: &str, default: f32| {
                (self.day.as_ref()).map_or(default, |day| day.keyframe.get_or(name, default))
            };
            terrain.record(
                queue,
                encoder,
                &engine,
                (&self.bind_group, &self.depth_view),
                deferred.gbuffer.self_shadows(),
                (fade, &day),
                (self.width, self.height),
            );
            self.gpu_mark(encoder, "terrain shadow");
        }
        // Water reads how far the opaque surfaces lie, where the pass that works that out has no occlusion to give
        let refracts = self.refracts || self.world.water.drawn();
        if self.world.water.drawn() {
            self.world.water.write(
                queue,
                &engine,
                (self.width, self.height),
                cascades.as_ref().map(|c| c.views()),
            );
        }
        // The coronas read how far the opaque surfaces lie as well
        let depth_wanted = refracts || has_coronas;
        deferred
            .passes
            .record_ambient(encoder, occlusion, depth_wanted);
        self.gpu_mark(encoder, "ambient occlusion");
        {
            let mut pass = over_the_frame(
                encoder,
                "viewport-lighting-pass",
                &deferred.hdr,
                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                &self.depth_view,
                None,
            );
            deferred.passes.record_lighting(&mut pass, &engine);
        }
        self.gpu_mark(encoder, "lighting");
        let sky_shown = self.shows_sky(deferred);
        if sky_shown {
            // A day that states its sky as a table has it drawn over the dome's clouds and stars alone
            let view = self.day.as_ref().and_then(|d| d.sky_view.as_ref());
            let sky = match view {
                Some(_) => self.sky().without_atmosphere(),
                None => self.sky(),
            };
            deferred
                .passes
                .write_sky(queue, &engine, &sky, camera.far, view);
            deferred.passes.record_sky_noise(encoder);
            let mut pass = over_the_frame(
                encoder,
                "viewport-sky-pass",
                &deferred.hdr,
                wgpu::LoadOp::Load,
                &self.depth_view,
                None,
            );
            deferred.passes.record_sky(&mut pass);
        }
        self.gpu_mark(encoder, "sky");
        // The water's fog gives it its colour before the copy is taken: its own pass lays the colour of the water into the frame where it lies in front of what is opaque, scaled by the light the water takes
        if let (true, true, Some(sink)) = (self.refracts, scene.has_water(), &deferred.fog_sink) {
            let attachment = |view, load| {
                Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport-water-fog-pass"),
                color_attachments: &[
                    attachment(&deferred.hdr, wgpu::LoadOp::Load),
                    attachment(sink, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: None,
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let light = engine.water_light();
            pass.set_blend_constant(wgpu::Color {
                r: f64::from(light.x),
                g: f64::from(light.y),
                b: f64::from(light.z),
                a: 1.0,
            });
            scene.record_water_fog(&mut pass);
        }
        // What glows, and the distant lights and coronas, are laid in before the water's copy is taken, as the game's own emissive target is in its frame, so that the water mirrors them
        let lights_first = refracts && deferred.refraction.is_some();
        {
            let mut pass = over_the_frame(
                encoder,
                "viewport-glow-pass",
                &deferred.hdr,
                wgpu::LoadOp::Load,
                &self.depth_view,
                Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
            );
            self.world.draw_glow(&mut pass, &self.bind_group);
            if lights_first {
                deferred.passes.record_distant_lights(&mut pass);
            }
        }
        // Water is drawn over a copy of the frame as it stands, since it cannot read what it writes
        if let (true, Some(refraction)) = (refracts, &deferred.refraction) {
            encoder.copy_texture_to_texture(
                deferred.hdr.texture().as_image_copy(),
                refraction.texture().as_image_copy(),
                wgpu::Extent3d {
                    width: self.width,
                    height: self.height,
                    depth_or_array_layers: 1,
                },
            );
        }
        self.gpu_mark(encoder, "water");
        {
            // What blends is drawn over the light, testing the depth and marking the stencil where it lands
            let mut pass = over_the_frame(
                encoder,
                "viewport-blended-pass",
                &deferred.hdr,
                wgpu::LoadOp::Load,
                &self.depth_view,
                Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
            );
            scene.record_blended(&mut pass);
            self.world.draw_lit(&mut pass, &self.bind_group);
            if !lights_first {
                deferred.passes.record_distant_lights(&mut pass);
            }
        }
        self.gpu_mark(encoder, "blended");
        // The fog the particles fill the air with is laid over what blends, testing the depth and leaving the stencil be
        if !self.fog_volumes.is_empty() {
            let mut pass = over_the_frame(
                encoder,
                "viewport-fog-volume-pass",
                &deferred.hdr,
                wgpu::LoadOp::Load,
                &self.depth_view,
                None,
            );
            deferred.passes.record_fog(&mut pass);
        }
        self.gpu_mark(encoder, "fog volumes");
        // GTA VI's frame is laid over its history before it is metered and bloomed
        let post = self.gta6_post();
        let exposure = (self.day.as_ref().and_then(|d| d.stated)).map_or(1.0, |s| s.exposure);
        if let Some(post) = post {
            let unjittered = camera.view_proj(aspect);
            let jittered = engine.projection * engine.view;
            let views = (unjittered, jittered, camera.eye());
            let at = (post, exposure);
            (deferred.post).record_temporal(encoder, queue, &deferred.hdr, at, views);
            self.gpu_mark(encoder, "temporal");
        }
        (deferred.meter).record(encoder, queue, (self.day_revision, exposure));
        match post {
            Some(post) => {
                (deferred.grade).record(encoder, queue, &deferred.hdr, &post.grade);
                (deferred.post).record_bloom(encoder, queue, &deferred.hdr, post, exposure);
            }
            // The bright part of the lit frame is blurred and put back over it, before the frame is mapped
            None => deferred.passes.record_bloom(encoder, &deferred.hdr, &tone),
        }
        self.gpu_mark(encoder, "bloom");
        // Where the frame is anti-aliased or sharpened it is mapped to a target of its own first, with the background around it
        let antialiased = post.is_some() || deferred.passes.antialiases();
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport-resolve-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: if antialiased {
                        &deferred.ldr
                    } else {
                        &self.colour_display_view
                    },
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(if antialiased {
                            self.ldr_background()
                        } else {
                            self.display_background()
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: None,
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            deferred.passes.record_resolve(&mut pass);
            if sky_shown {
                deferred.passes.record_background(&mut pass);
            } else if post.is_none() && deferred.passes.blooms(&tone) {
                deferred.passes.record_halo(&mut pass);
            }
        }
        self.gpu_mark(encoder, "resolve");
        // GTA VI's frame has no edge blending of its own after the history: it is sharpened onto the screen
        if let Some(post) = post {
            let background = self.display_background();
            let screen = &self.colour_display_view;
            let shift = (post.sharpen, &post.scotopic);
            (deferred.post).record_sharpen(encoder, queue, screen, background, shift);
        } else if antialiased {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport-antialias-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.colour_display_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.display_background()),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            deferred.passes.record_antialias(&mut pass);
        }
        self.gpu_mark(encoder, "anti-aliasing");
    }
}

/// Whether a material is water, which the game draws over a copy of the frame.
fn is_water(material: &crate::game_shaders::mesh::Material) -> bool {
    material.effect.to_ascii_lowercase().starts_with("water")
}
