use super::*;
pub(super) fn draw_ypt_behaviour_mut<T: ParticleTarget>(
    ui: &mut egui::Ui,
    state: &mut AppState,
    doc_id: DocumentId,
    particle_idx: usize,
    behaviour_idx: usize,
) {
    let gta6 = crate::state::gta6_particles::written_in_place(state, doc_id);
    let mut before = None;
    {
        let Some(doc) = state.document_mut(doc_id) else {
            return;
        };
        let Some(asset) = T::list_mut(&mut doc.payload) else {
            return;
        };
        let Some(rule) = or_particle::list::particle_rules_mut(asset).nth(particle_idx) else {
            empty(ui, tr!("Selection is no longer valid"));
            return;
        };
        let rule_name = rule.name.clone();
        let mut behaviours = or_particle::list::behaviours(rule);
        let Some(behaviour) = behaviours.get_mut(behaviour_idx) else {
            empty(ui, tr!("Selection is no longer valid"));
            return;
        };
        ui.label(
            egui::RichText::new(format!(
                "Behaviour: {} ({})",
                behaviour_type_name(behaviour),
                rule_name
            ))
            .strong()
            .color(theme::TEXT_PRIMARY),
        );
        ui.add_space(crate::ui::theme::SPACING_TIGHT);
        if members(ui, behaviour, gta6) {
            before = Some(rule.clone());
            or_particle::list::set_behaviours(rule, &behaviours);
        }
    }
    if let Some(before) = before {
        super::ypt_steps::particle(state, doc_id, before);
    }
}

/// The members a scanned GTA VI list writes back, drawn editable above the rest.
fn written_members(ui: &mut egui::Ui, behaviour: &mut or_particle::Behaviour, dirty: &mut bool) {
    use or_particle::Behaviour;
    match behaviour {
        Behaviour::Colour(d) => check_bool(ui, "rgba_max_enable", &mut d.rgba_max_enable, dirty),
        Behaviour::DrawSprite(d) => {
            drag_f32(ui, "flip_chance_u", &mut d.flip_chance_u, 0.01, dirty);
            drag_f32(ui, "flip_chance_v", &mut d.flip_chance_v, 0.01, dirty);
        }
        Behaviour::ZCulling(d) => drag_u32(ui, "mode", &mut d.mode, dirty),
        _ => {}
    }
}

/// The behaviour's members drawn, where `gta6` only those its list writes back editable; whether
/// one was edited.
fn members(ui: &mut egui::Ui, behaviour: &mut or_particle::Behaviour, gta6: bool) -> bool {
    use or_particle::Behaviour;
    let mut dirty = false;
    if gta6 {
        written_members(ui, behaviour, &mut dirty);
    }
    ui.scope(|ui| {
        if gta6 {
            ui.disable();
        }
        match behaviour {
            Behaviour::Acceleration(d) => {
                drag_vec3(ui, "xyz_min", &mut d.xyz_min, &mut dirty);
                drag_vec3(ui, "xyz_max", &mut d.xyz_max, &mut dirty);
                check_bool(
                    ui,
                    "is_affected_by_zoom",
                    &mut d.is_affected_by_zoom,
                    &mut dirty,
                );
                check_bool(ui, "enable_gravity", &mut d.enable_gravity, &mut dirty);
            }
            Behaviour::Velocity(d) => {
                drag_vec3(ui, "xyz_min", &mut d.xyz_min, &mut dirty);
                drag_vec3(ui, "xyz_max", &mut d.xyz_max, &mut dirty);
            }
            Behaviour::Rotation(d) => {
                check_bool(ui, "accumulate_angle", &mut d.accumulate_angle, &mut dirty);
                check_bool(
                    ui,
                    "rotate_angle_axes",
                    &mut d.rotate_angle_axes,
                    &mut dirty,
                );
                check_bool(
                    ui,
                    "rotate_init_angle_axes",
                    &mut d.rotate_init_angle_axes,
                    &mut dirty,
                );
                drag_f32(
                    ui,
                    "speed_fade_thresh",
                    &mut d.speed_fade_thresh,
                    0.01,
                    &mut dirty,
                );
            }
            Behaviour::RotationSpeed(d) => {
                drag_f32(ui, "speed_min", &mut d.speed_min, 0.01, &mut dirty);
                drag_f32(ui, "speed_max", &mut d.speed_max, 0.01, &mut dirty);
                drag_vec3(ui, "axis", &mut d.axis, &mut dirty);
            }
            Behaviour::Colour(d) => {
                if !gta6 {
                    check_bool(ui, "rgba_max_enable", &mut d.rgba_max_enable, &mut dirty);
                }
                check_bool(
                    ui,
                    "rgba_proportional",
                    &mut d.rgba_proportional,
                    &mut dirty,
                );
                check_bool(ui, "rgb_can_tint", &mut d.rgb_can_tint, &mut dirty);
                drag_f32(ui, "rgba_min.r", &mut d.rgba_min.r, 0.01, &mut dirty);
                drag_f32(ui, "rgba_min.g", &mut d.rgba_min.g, 0.01, &mut dirty);
                drag_f32(ui, "rgba_min.b", &mut d.rgba_min.b, 0.01, &mut dirty);
                drag_f32(ui, "rgba_min.a", &mut d.rgba_min.a, 0.01, &mut dirty);
                drag_f32(ui, "rgba_max.r", &mut d.rgba_max.r, 0.01, &mut dirty);
                drag_f32(ui, "rgba_max.g", &mut d.rgba_max.g, 0.01, &mut dirty);
                drag_f32(ui, "rgba_max.b", &mut d.rgba_max.b, 0.01, &mut dirty);
                drag_f32(ui, "rgba_max.a", &mut d.rgba_max.a, 0.01, &mut dirty);
                drag_f32(
                    ui,
                    "emissive_intensity_min",
                    &mut d.emissive_intensity_min,
                    0.01,
                    &mut dirty,
                );
                drag_f32(
                    ui,
                    "emissive_intensity_max",
                    &mut d.emissive_intensity_max,
                    0.01,
                    &mut dirty,
                );
            }
            Behaviour::Size(d) => {
                check_bool(ui, "is_proportional", &mut d.is_proportional, &mut dirty);
            }
            Behaviour::AnimateTexture(d) => {
                drag_f32(ui, "anim_rate_min", &mut d.anim_rate_min, 0.1, &mut dirty);
                drag_f32(ui, "anim_rate_max", &mut d.anim_rate_max, 0.1, &mut dirty);
                check_bool(ui, "is_randomised", &mut d.is_randomised, &mut dirty);
                check_bool(
                    ui,
                    "is_held_on_last_frame",
                    &mut d.is_held_on_last_frame,
                    &mut dirty,
                );
                check_bool(
                    ui,
                    "do_frame_blending",
                    &mut d.do_frame_blending,
                    &mut dirty,
                );
                check_bool(
                    ui,
                    "is_scaled_over_particle_life",
                    &mut d.is_scaled_over_particle_life,
                    &mut dirty,
                );
            }
            Behaviour::Noise(d) => {
                check_bool(
                    ui,
                    "keep_constant_speed",
                    &mut d.keep_constant_speed,
                    &mut dirty,
                );
            }
            Behaviour::Wind(d) => {
                drag_f32(ui, "high_lod_range", &mut d.high_lod_range, 0.1, &mut dirty);
                drag_f32(ui, "low_lod_range", &mut d.low_lod_range, 0.1, &mut dirty);
                check_bool(
                    ui,
                    "ignore_mtx_weight",
                    &mut d.ignore_mtx_weight,
                    &mut dirty,
                );
            }
            Behaviour::Dampening(d) => {
                drag_vec3(ui, "xyz_min", &mut d.xyz_min, &mut dirty);
                drag_vec3(ui, "xyz_max", &mut d.xyz_max, &mut dirty);
                check_bool(
                    ui,
                    "enable_air_resistance",
                    &mut d.enable_air_resistance,
                    &mut dirty,
                );
            }
            Behaviour::Gravity(d) => {
                drag_f32(ui, "strength", &mut d.strength, 0.1, &mut dirty);
                drag_vec3(ui, "direction", &mut d.direction, &mut dirty);
            }
            Behaviour::Attractor(d) => {
                drag_f32(ui, "strength_min", &mut d.strength_min, 0.1, &mut dirty);
                drag_f32(ui, "strength_max", &mut d.strength_max, 0.1, &mut dirty);
            }
            Behaviour::FadeIn(d) => {
                drag_f32(ui, "duration", &mut d.duration, 0.01, &mut dirty);
            }
            Behaviour::FadeOut(d) => {
                drag_f32(ui, "duration", &mut d.duration, 0.01, &mut dirty);
            }
            Behaviour::DrawSprite(d) => {
                drag_f32(ui, "near_clip_dist", &mut d.near_clip_dist, 0.1, &mut dirty);
                drag_f32(ui, "far_clip_dist", &mut d.far_clip_dist, 0.1, &mut dirty);
                if !gta6 {
                    drag_f32(ui, "flip_chance_u", &mut d.flip_chance_u, 0.01, &mut dirty);
                    drag_f32(ui, "flip_chance_v", &mut d.flip_chance_v, 0.01, &mut dirty);
                }
                check_bool(ui, "is_screen_space", &mut d.is_screen_space, &mut dirty);
                check_bool(ui, "near_clip", &mut d.near_clip, &mut dirty);
                check_bool(ui, "far_clip", &mut d.far_clip, &mut dirty);
                check_bool(ui, "uv_clip", &mut d.uv_clip, &mut dirty);
                check_bool(ui, "is_hi_res", &mut d.is_hi_res, &mut dirty);
                check_bool(ui, "disable_draw", &mut d.disable_draw, &mut dirty);
                drag_f32(
                    ui,
                    "projection_depth",
                    &mut d.projection_depth,
                    0.01,
                    &mut dirty,
                );
                drag_f32(
                    ui,
                    "shadow_cast_intensity",
                    &mut d.shadow_cast_intensity,
                    0.01,
                    &mut dirty,
                );
            }
            Behaviour::DrawTrail(d) => {
                drag_u32(ui, "tessellation_u", &mut d.tessellation_u, &mut dirty);
                drag_u32(ui, "tessellation_v", &mut d.tessellation_v, &mut dirty);
                drag_f32(ui, "smoothness_x", &mut d.smoothness_x, 0.01, &mut dirty);
                drag_f32(ui, "smoothness_y", &mut d.smoothness_y, 0.01, &mut dirty);
                drag_f32(
                    ui,
                    "projection_depth",
                    &mut d.projection_depth,
                    0.01,
                    &mut dirty,
                );
                drag_f32(
                    ui,
                    "shadow_cast_intensity",
                    &mut d.shadow_cast_intensity,
                    0.01,
                    &mut dirty,
                );
                check_bool(ui, "flip_u", &mut d.flip_u, &mut dirty);
                check_bool(ui, "flip_v", &mut d.flip_v, &mut dirty);
                check_bool(
                    ui,
                    "wrap_texture_over_particle_life",
                    &mut d.wrap_texture_over_particle_life,
                    &mut dirty,
                );
                check_bool(ui, "disable_draw", &mut d.disable_draw, &mut dirty);
                drag_vec3(ui, "align_axis", &mut d.align_axis, &mut dirty);
            }
            Behaviour::DrawModel(d) => {
                drag_u32(
                    ui,
                    "colour_control_shader_id",
                    &mut d.colour_control_shader_id,
                    &mut dirty,
                );
                drag_f32(ui, "camera_shrink", &mut d.camera_shrink, 0.01, &mut dirty);
                drag_f32(
                    ui,
                    "shadow_cast_intensity",
                    &mut d.shadow_cast_intensity,
                    0.01,
                    &mut dirty,
                );
                check_bool(ui, "disable_draw", &mut d.disable_draw, &mut dirty);
            }
            Behaviour::Collision(d) => {
                drag_f32(ui, "radius_mult", &mut d.radius_mult, 0.01, &mut dirty);
                drag_f32(ui, "rest_speed", &mut d.rest_speed, 0.01, &mut dirty);
                drag_u32(ui, "collision_chance", &mut d.collision_chance, &mut dirty);
                drag_u32(ui, "kill_chance", &mut d.kill_chance, &mut dirty);
                drag_f32(
                    ui,
                    "override_min_radius",
                    &mut d.override_min_radius,
                    0.01,
                    &mut dirty,
                );
                check_bool(ui, "debug_draw", &mut d.debug_draw, &mut dirty);
            }
            Behaviour::MatrixWeight(d) => {
                ui.label(
                    egui::RichText::new(format!("reference_space: {:?}", d.reference_space))
                        .small()
                        .color(theme::TEXT_SECONDARY),
                );
                let count = d.kfp_meta.len();
                ui.label(
                    egui::RichText::new(format!(
                        "{} KFP meta entr{}",
                        count,
                        if count == 1 { "y" } else { "ies" }
                    ))
                    .small()
                    .color(theme::TEXT_SECONDARY),
                );
            }
            Behaviour::Pause(d) => {
                drag_f32(
                    ui,
                    "pause_life_ratio",
                    &mut d.pause_life_ratio,
                    0.01,
                    &mut dirty,
                );
                drag_f32(ui, "unpause_time", &mut d.unpause_time, 0.01, &mut dirty);
                drag_f32(
                    ui,
                    "unpause_effect_dist",
                    &mut d.unpause_effect_dist,
                    0.1,
                    &mut dirty,
                );
                drag_f32(
                    ui,
                    "unpause_cam_dist",
                    &mut d.unpause_cam_dist,
                    0.1,
                    &mut dirty,
                );
            }
            Behaviour::Age(d) => {
                drag_f32(ui, "age_rate", &mut d.age_rate, 0.01, &mut dirty);
            }
            Behaviour::ZCulling(d) => {
                if !gta6 {
                    drag_u32(ui, "mode", &mut d.mode, &mut dirty);
                }
                ui.label(
                    egui::RichText::new(format!("reference_space: {:?}", d.reference_space))
                        .small()
                        .color(theme::TEXT_SECONDARY),
                );
            }
            Behaviour::Decal(d) => {
                drag_u32(ui, "decal_id", &mut d.decal_id, &mut dirty);
                drag_f32(
                    ui,
                    "velocity_thresh",
                    &mut d.velocity_thresh,
                    0.01,
                    &mut dirty,
                );
                drag_f32(ui, "total_life", &mut d.total_life, 0.01, &mut dirty);
                drag_f32(ui, "fade_in_time", &mut d.fade_in_time, 0.01, &mut dirty);
                drag_f32(ui, "uv_mult_start", &mut d.uv_mult_start, 0.01, &mut dirty);
                drag_f32(ui, "uv_mult_end", &mut d.uv_mult_end, 0.01, &mut dirty);
                drag_f32(ui, "uv_mult_time", &mut d.uv_mult_time, 0.01, &mut dirty);
                drag_f32(
                    ui,
                    "duplicate_reject_dist",
                    &mut d.duplicate_reject_dist,
                    0.01,
                    &mut dirty,
                );
                drag_f32(
                    ui,
                    "projection_depth",
                    &mut d.projection_depth,
                    0.01,
                    &mut dirty,
                );
                drag_f32(
                    ui,
                    "distance_scale",
                    &mut d.distance_scale,
                    0.01,
                    &mut dirty,
                );
                check_bool(ui, "flip_u", &mut d.flip_u, &mut dirty);
                check_bool(ui, "flip_v", &mut d.flip_v, &mut dirty);
                check_bool(
                    ui,
                    "proportional_size",
                    &mut d.proportional_size,
                    &mut dirty,
                );
                check_bool(ui, "use_complex_coln", &mut d.use_complex_coln, &mut dirty);
                check_bool(ui, "is_directional", &mut d.is_directional, &mut dirty);
            }
            Behaviour::Light(d) => {
                drag_u32(ui, "light_type", &mut d.light_type, &mut dirty);
                drag_f32(ui, "corona_z_bias", &mut d.corona_z_bias, 0.01, &mut dirty);
                check_bool(
                    ui,
                    "corona_use_light_col",
                    &mut d.corona_use_light_col,
                    &mut dirty,
                );
                check_bool(
                    ui,
                    "colour_from_particle",
                    &mut d.colour_from_particle,
                    &mut dirty,
                );
                check_bool(ui, "colour_per_frame", &mut d.colour_per_frame, &mut dirty);
                check_bool(
                    ui,
                    "intensity_per_frame",
                    &mut d.intensity_per_frame,
                    &mut dirty,
                );
                check_bool(ui, "range_per_frame", &mut d.range_per_frame, &mut dirty);
                check_bool(ui, "casts_shadows", &mut d.casts_shadows, &mut dirty);
                check_bool(
                    ui,
                    "corona_not_in_reflection",
                    &mut d.corona_not_in_reflection,
                    &mut dirty,
                );
                check_bool(
                    ui,
                    "corona_only_in_reflection",
                    &mut d.corona_only_in_reflection,
                    &mut dirty,
                );
            }
            Behaviour::FogVolume(d) => {
                drag_f32(ui, "falloff", &mut d.falloff, 0.01, &mut dirty);
                drag_f32(ui, "hdr_mult", &mut d.hdr_mult, 0.01, &mut dirty);
                drag_u32(ui, "lighting_type", &mut d.lighting_type, &mut dirty);
                check_bool(
                    ui,
                    "colour_tint_from_particle",
                    &mut d.colour_tint_from_particle,
                    &mut dirty,
                );
                check_bool(
                    ui,
                    "sort_with_particles",
                    &mut d.sort_with_particles,
                    &mut dirty,
                );
                check_bool(
                    ui,
                    "use_ground_fog_colour",
                    &mut d.use_ground_fog_colour,
                    &mut dirty,
                );
                check_bool(
                    ui,
                    "use_effect_evo_values",
                    &mut d.use_effect_evo_values,
                    &mut dirty,
                );
            }
            Behaviour::Liquid(d) => {
                drag_u32(ui, "liquid_type", &mut d.liquid_type, &mut dirty);
                drag_f32(
                    ui,
                    "velocity_thresh",
                    &mut d.velocity_thresh,
                    0.01,
                    &mut dirty,
                );
                drag_f32(
                    ui,
                    "pool_start_size",
                    &mut d.pool_start_size,
                    0.01,
                    &mut dirty,
                );
                drag_f32(ui, "pool_end_size", &mut d.pool_end_size, 0.01, &mut dirty);
                drag_f32(
                    ui,
                    "pool_growth_rate",
                    &mut d.pool_growth_rate,
                    0.01,
                    &mut dirty,
                );
                drag_f32(
                    ui,
                    "trail_width_min",
                    &mut d.trail_width_min,
                    0.01,
                    &mut dirty,
                );
                drag_f32(
                    ui,
                    "trail_width_max",
                    &mut d.trail_width_max,
                    0.01,
                    &mut dirty,
                );
            }
            Behaviour::DecalPool(d) => {
                drag_u32(ui, "liquid_type", &mut d.liquid_type, &mut dirty);
                drag_u32(ui, "decal_id", &mut d.decal_id, &mut dirty);
                drag_f32(
                    ui,
                    "velocity_thresh",
                    &mut d.velocity_thresh,
                    0.01,
                    &mut dirty,
                );
                drag_f32(ui, "start_size", &mut d.start_size, 0.01, &mut dirty);
                drag_f32(ui, "end_size", &mut d.end_size, 0.01, &mut dirty);
                drag_f32(ui, "growth_rate", &mut d.growth_rate, 0.01, &mut dirty);
            }
            Behaviour::River(d) => {
                drag_f32(ui, "velocity_mult", &mut d.velocity_mult, 0.01, &mut dirty);
                drag_f32(ui, "influence", &mut d.influence, 0.01, &mut dirty);
            }
            Behaviour::Unknown { type_id, data } => {
                ui.label(
                    egui::RichText::new(format!("type_id: {:#X}", type_id))
                        .small()
                        .color(theme::TEXT_SECONDARY),
                );
                ui.label(
                    egui::RichText::new(format!("data: {} bytes", data.len()))
                        .small()
                        .color(theme::TEXT_SECONDARY),
                );
            }
        }
    });
    dirty
}
fn behaviour_type_name(behaviour: &or_particle::Behaviour) -> &'static str {
    use or_particle::Behaviour;
    match behaviour {
        Behaviour::Acceleration(_) => "Acceleration",
        Behaviour::Velocity(_) => "Velocity",
        Behaviour::Rotation(_) => "Rotation",
        Behaviour::RotationSpeed(_) => "Rotation Speed",
        Behaviour::Colour(_) => "Colour",
        Behaviour::Size(_) => "Size",
        Behaviour::AnimateTexture(_) => "Animate Texture",
        Behaviour::FadeIn(_) => "Fade In",
        Behaviour::FadeOut(_) => "Fade Out",
        Behaviour::Noise(_) => "Noise",
        Behaviour::Wind(_) => "Wind",
        Behaviour::Dampening(_) => "Dampening",
        Behaviour::Gravity(_) => "Gravity",
        Behaviour::Collision(_) => "Collision",
        Behaviour::Attractor(_) => "Attractor",
        Behaviour::DrawSprite(_) => "Draw Sprite",
        Behaviour::DrawTrail(_) => "Draw Trail",
        Behaviour::DrawModel(_) => "Draw Model",
        Behaviour::Decal(_) => "Decal",
        Behaviour::FogVolume(_) => "Fog Volume",
        Behaviour::Light(_) => "Light",
        Behaviour::Liquid(_) => "Liquid",
        Behaviour::River(_) => "River",
        Behaviour::DecalPool(_) => "Decal Pool",
        Behaviour::Age(_) => "Age",
        Behaviour::Pause(_) => "Pause",
        Behaviour::ZCulling(_) => "Z Culling",
        Behaviour::MatrixWeight(_) => "Matrix Weight",
        Behaviour::Unknown { .. } => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::epaint::ClippedShape;
    use or_particle::Behaviour;

    /// The members of a scanned GTA VI list's behaviour drawn frame by frame.
    #[derive(Default)]
    struct Frames {
        ctx: egui::Context,
        time: f64,
    }

    impl Frames {
        /// A frame of `events`: whether it edited the behaviour, and what it painted.
        fn frame(
            &mut self,
            b: &mut Behaviour,
            events: Vec<egui::Event>,
        ) -> (bool, Vec<ClippedShape>) {
            // ⚠ A press held past egui's click duration is a drag, not a click.
            self.time += 0.1;
            let input = egui::RawInput {
                events,
                time: Some(self.time),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 1200.0),
                )),
                ..Default::default()
            };
            let mut dirty = false;
            let output = self.ctx.run_ui(input, |ui| dirty |= members(ui, b, true));
            (dirty, output.shapes)
        }

        /// A drag 40 points to the right across the value beside `label`; whether it edited.
        fn drag(&mut self, b: &mut Behaviour, label: &str) -> bool {
            let (_, shapes) = self.frame(b, Vec::new());
            let at = value_beside(&shapes, label).expect(label);
            let to = at + egui::vec2(40.0, 0.0);
            let press = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            let pressed = vec![egui::Event::PointerMoved(at), press(at, true)];
            let mut dirty = self.frame(b, pressed).0;
            dirty |= self.frame(b, vec![egui::Event::PointerMoved(to)]).0;
            dirty | self.frame(b, vec![press(to, false)]).0
        }
    }

    /// Where egui painted the first text to the right of `label`, on its row.
    fn value_beside(shapes: &[ClippedShape], label: &str) -> Option<egui::Pos2> {
        fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
            match shape {
                egui::Shape::Text(t) => {
                    let rect = t.galley.rect.translate(t.pos.to_vec2());
                    out.push((t.galley.text().to_string(), rect));
                }
                egui::Shape::Vec(inner) => inner.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut texts = Vec::new();
        shapes.iter().for_each(|c| walk(&c.shape, &mut texts));
        let (_, at) = texts.iter().find(|(text, _)| text == label)?;
        (texts.iter())
            .filter(|(_, r)| r.min.x >= at.max.x && (r.center().y - at.center().y).abs() < 8.0)
            .min_by(|a, b| a.1.min.x.total_cmp(&b.1.min.x))
            .map(|(_, r)| r.center())
    }

    #[test]
    fn a_gta_vi_sprite_takes_a_flip_chance_edit_and_no_edit_of_a_member_it_does_not_write() {
        let mut sprite = Behaviour::DrawSprite(Default::default());
        let mut frames = Frames::default();
        assert!(!frames.drag(&mut sprite, "near_clip_dist"), "not written");
        assert!(frames.drag(&mut sprite, "flip_chance_u"), "written");
        let Behaviour::DrawSprite(d) = &sprite else {
            unreachable!("a sprite");
        };
        assert_eq!(d.near_clip_dist, 0.0);
        assert!(d.flip_chance_u > 0.0, "{}", d.flip_chance_u);
    }
}
