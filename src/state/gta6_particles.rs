//! A scanned GTA VI particle effects list in the Particle workspace: a GTA V list built from the
//! fields both games share, and lines for what GTA VI states that GTA V's list has no place for.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use or_io::container::rpf::{Rpf9Resource, Rpf9Role, Rpf9Signature};
use or_io::decl::{Bool8, Gen8, Hash32, Owned, Record, Ref};
use or_io::gta5::particle::*;
use or_io::gta5::{PgDictionary, Vec3V, Vec4V};
use or_io::gta6::headerless::{
    class_of_bit, BehaviourFields, ClassNamedBy, KfpEntry, COLOUR_RGBA_KFPS,
};
use or_io::gta6::headerless::{
    read_particle_effects, Behaviour, Domain, EffectRule, EmitterRule, Event, EvolutionList, Kfp,
    ParticleEffects, ParticleRule, SpawnedScalars, Spawner, DOMAIN_KFPS, EFFECT_KFPS, EMITTER_KFPS,
};

use super::DocPayload;

#[path = "gta6_particle_sprites.rs"]
pub(crate) mod sprites;

#[path = "gta6_particle_raw.rs"]
mod raw;

#[path = "gta6_particle_spawners.rs"]
pub(crate) mod spawners;

#[path = "gta6_particle_colour.rs"]
pub(crate) mod colour_max;

/// The highest alignment mode GTA V's sprite names.
const GTA5_ALIGNMENTS: u32 = 4;

/// The highest `ptxu_ZCull` mode GTA V names.
const GTA5_ZCULL_MODES: i32 = 3;

/// The size of the behaviour that opens most rules' lists, which no keyframed value classes.
const FIRST_BEHAVIOUR: u64 = 0x10;

// ponytail: one entry per scanned list opened, never emptied, as the scanned cells are.
static SCANNED: Mutex<BTreeMap<PathBuf, Arc<Vec<String>>>> = Mutex::new(BTreeMap::new());

fn scanned() -> MutexGuard<'static, BTreeMap<PathBuf, Arc<Vec<String>>>> {
    SCANNED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A scanned particle effects list as GTA V's, where it reads; its own lines are kept for `path`.
pub(super) fn opened(
    path: &Path,
    signature: Rpf9Signature,
    resource: &Rpf9Resource,
    bytes: &[u8],
) -> Option<DocPayload> {
    if signature.role() != Some(Rpf9Role::ParticleEffects) {
        return None;
    }
    let (virtual_section, physical) = resource.sections(bytes);
    from_sections(path, virtual_section, physical).ok()
}

/// The particle preview and its caches, read only from the two supplied sections.
pub(super) fn from_sections(
    path: &Path,
    virtual_section: &[u8],
    physical: &[u8],
) -> Result<DocPayload, or_io::gta6::headerless::HeaderlessError> {
    let list = read_particle_effects(virtual_section, Some(physical))?;
    let mut ypt = as_ypt(&list);
    or_particle::list::canonical(&mut ypt);
    scanned().insert(path.to_path_buf(), Arc::new(lines(&list)));
    sprites::keep(path, &list);
    spawners::keep(path, &list);
    super::gta6_particle_models::keep(path, &list);
    Ok(DocPayload::Resource(or_io::Asset::Particles(ypt)))
}

/// The lines of the GTA VI list the document at `path` opened from; `None` for any other.
pub(crate) fn scanned_lines(path: &Path) -> Option<Arc<Vec<String>>> {
    scanned().get(path).cloned()
}

/// Whether `doc` is a scanned GTA VI list, saved in place, so no rule is made in it from nothing.
pub(crate) fn written_in_place(state: &super::AppState, doc: super::DocumentId) -> bool {
    let path = state.document(doc).and_then(|d| d.path.as_deref());
    path.is_some_and(|p| scanned_lines(p).is_some())
}

/// The list as GTA V's, its rules in their dictionaries and named by the events and spawners.
pub(super) fn as_ypt(list: &ParticleEffects) -> PtxFxList {
    PtxFxList {
        name: list.name.clone(),
        effect_rule_dict: dictionary(
            (list.effects.iter())
                .map(|e| (e.code, effect_rule(list, e)))
                .collect(),
        ),
        emitter_rule_dict: dictionary(
            (list.emitters.iter())
                .map(|e| (e.code, emitter_rule(e)))
                .collect(),
        ),
        particle_rule_dict: dictionary(
            (list.particles.iter())
                .map(|p| (p.code, particle_rule(list, p)))
                .collect(),
        ),
        ..Default::default()
    }
}

fn dictionary<T: Owned<Raw: Record>>(entries: Vec<(u32, T)>) -> Option<Box<PgDictionary<T>>>
where
    PgDictionary<T>: Default,
{
    let (codes, entries) = (entries.into_iter())
        .map(|(code, entry)| (code, Some(Box::new(entry))))
        .unzip();
    Some(Box::new(PgDictionary {
        codes,
        entries,
        ..Default::default()
    }))
}

fn flag(byte: u8) -> Bool8 {
    Bool8::new(byte != 0)
}

fn vec3([x, y, z]: [f32; 3]) -> Vec3V {
    let mut v = Vec3V::default();
    (v.x, v.y, v.z) = (x, y, z);
    v
}

fn vec4([x, y, z, w]: [f32; 4]) -> Vec4V {
    let mut v = Vec4V::default();
    (v.x, v.y, v.z, v.w) = (x, y, z, w);
    v
}

fn entries(keys: &[KfpEntry]) -> Vec<PtxKeyframeEntry> {
    (keys.iter())
        .map(|e| {
            let mut entry = PtxKeyframeEntry::default();
            (entry.time, entry.value) = (vec4(e.time), vec4(e.value));
            entry
        })
        .collect()
}

/// `k`'s keys under the GTA V property id `id`.
fn kfp(k: &Kfp, id: u32) -> PtxKeyframeProp {
    let mut prop = PtxKeyframeProp {
        property_id: Hash32::from_raw(id),
        ..Default::default()
    };
    prop.keyframe.entries = entries(&k.entries);
    prop
}

/// The GTA V form's id of the keyframed value GTA VI keys by `id`.
fn gta5_id(id: u32) -> u32 {
    match id {
        _ if id == COLOUR_RGBA_KFPS[0] => KFP_COLOUR[0],
        _ if id == COLOUR_RGBA_KFPS[1] => KFP_COLOUR[1],
        _ if id == Hash32::of("ptxu_Attractor:m_strengthKFP").get() => KFP_ATTRACTOR[0],
        _ => id,
    }
}

/// The behaviour's value of id `id` under the GTA V id `gta5`, with no keys where it holds none.
fn kfp_of(b: &Behaviour, id: u32, gta5: u32) -> PtxKeyframeProp {
    match b.kfps.iter().find(|k| k.id == id) {
        Some(k) => kfp(k, gta5),
        None => PtxKeyframeProp {
            property_id: Hash32::from_raw(gta5),
            ..Default::default()
        },
    }
}

fn evolution_list(l: &EvolutionList) -> Box<PtxEvolutionList> {
    let evolutions = (l.evolutions.iter())
        .map(|e| PtxEvolution {
            name: e.name.clone(),
            name_hash: Hash32::of(&e.name),
            override_value: e.override_value,
            is_overriden: Bool8::new(e.is_overridden),
            ..Default::default()
        })
        .collect();
    let evolved_keyframe_props = (l.props.iter())
        .map(|p| PtxEvolvedKeyframeProp {
            evolved_keyframes: (p.keyframes.iter())
                .map(|k| {
                    let mut keyframe = PtxEvolvedKeyframe {
                        evolution_idx: k.evolution as i32,
                        is_lod_evolution: Bool8::new(k.kind == 1),
                        ..Default::default()
                    };
                    keyframe.keyframe.entries = entries(&k.entries);
                    keyframe
                })
                .collect(),
            property_id: Hash32::from_raw(gta5_id(p.id)),
            blend_mode: p.blend_mode,
            ..Default::default()
        })
        .collect();
    Box::new(PtxEvolutionList {
        evolutions,
        evolved_keyframe_props,
        ..Default::default()
    })
}

fn effect_rule(list: &ParticleEffects, e: &EffectRule) -> PtxEffectRule<Gen8> {
    let [pre_time, pre_interval, duration_min, duration_max, rate_min, rate_max] = e.times;
    let [viewport, render_v, update_v, emit_v, distance, render_d, update_d, emit_d] =
        e.culling_modes;
    let [radius, fade_dist, cull_dist, lod_min, lod_max] = e.culling;
    let k = |i: usize| kfp(&e.kfps[i], EFFECT_KFPS[i]);
    let events = (e.events.iter().enumerate())
        .filter_map(|(i, ev)| event(list, i, ev).map(|ev| Some(Box::new(ev))))
        .collect();
    PtxEffectRule {
        file_version: 4.2,
        name: e.name.clone(),
        timeline: PtxTimeLine {
            events,
            ..Default::default()
        },
        evolution_list: e.evolution_list.as_ref().map(evolution_list),
        num_loops: e.num_loops,
        sort_events_by_distance: flag(e.flags[0]),
        draw_list_id: e.flags[1],
        is_short_lived: flag(e.flags[2]),
        has_no_shadows: flag(e.flags[3]),
        rand_offset_pos: vec3(e.rand_offset_pos),
        pre_update_time: pre_time,
        pre_update_time_interval: pre_interval,
        duration_min,
        duration_max,
        playback_rate_scalar_min: rate_min,
        playback_rate_scalar_max: rate_max,
        viewport_culling_mode: viewport,
        render_when_viewport_culled: flag(render_v),
        update_when_viewport_culled: flag(update_v),
        emit_when_viewport_culled: flag(emit_v),
        distance_culling_mode: distance,
        render_when_distance_culled: flag(render_d),
        update_when_distance_culled: flag(update_d),
        emit_when_distance_culled: flag(emit_d),
        viewport_culling_sphere_offset: vec3(e.culling_sphere_offset),
        viewport_culling_sphere_radius: radius,
        distance_culling_fade_dist: fade_dist,
        distance_culling_cull_dist: cull_dist,
        lod_evo_dist_min: lod_min,
        lod_evo_dist_max: lod_max,
        coln_range: e.coln_range,
        coln_probe_dist: e.coln_probe_dist,
        colour_tint_min_kfp: k(0),
        colour_tint_max_kfp: k(1),
        zoom_scalar_kfp: k(2),
        data_sphere_kfp: k(3),
        data_capsule_kfp: k(4),
        colour_tint_max_enable: flag(e.data_volume[0]),
        use_data_volume: flag(e.data_volume[1]),
        data_volume_type: e.data_volume[2],
        zoom_level: 100.0,
        ..Default::default()
    }
}

/// An event naming its rules, whose keys the list's canonical form gives.
fn event(list: &ParticleEffects, index: usize, ev: &Event) -> Option<PtxEventEmitter<Gen8>> {
    let (emitter, particle) = (
        list.emitters.get(ev.emitter)?,
        list.particles.get(ev.particle)?,
    );
    Some(PtxEventEmitter {
        base: PtxEvent {
            index: i32::try_from(index).ok()?,
            start_ratio: ev.start_ratio,
            end_ratio: ev.end_ratio,
            evolution_list: ev.evolution_list.as_ref().map(evolution_list),
            ..Default::default()
        },
        emitter_rule_name: emitter.name.clone(),
        particle_rule_name: particle.name.clone(),
        playback_rate_scalar_min: ev.playback_rate_scalar[0],
        playback_rate_scalar_max: ev.playback_rate_scalar[1],
        zoom_scalar_min: ev.zoom_scalar[0],
        zoom_scalar_max: ev.zoom_scalar[1],
        colour_tint_min: ev.colour_tint[0],
        colour_tint_max: ev.colour_tint[1],
        ..Default::default()
    })
}

fn emitter_rule(e: &EmitterRule) -> PtxEmitterRule {
    let k = |i: usize| kfp(&e.kfps[i], EMITTER_KFPS[i]);
    let held = |kind: usize| PtxDomainObj {
        domain: e.domains[kind].as_ref().map(|d| Box::new(domain(d, kind))),
        ..Default::default()
    };
    PtxEmitterRule {
        file_version: 4.1,
        name: e.name.clone(),
        creation_domain_obj: held(0),
        target_domain_obj: held(1),
        attractor_domain_obj: held(2),
        spawn_rate_over_time_kfp: k(0),
        spawn_rate_over_dist_kfp: k(1),
        particle_life_kfp: k(2),
        playback_rate_scalar_kfp: k(3),
        speed_scalar_kfp: k(4),
        size_scalar_kfp: k(5),
        accn_scalar_kfp: k(6),
        dampening_scalar_kfp: k(7),
        matrix_weight_scalar_kfp: k(8),
        inherit_velocity_kfp: k(9),
        ..Default::default()
    }
}

/// A domain of type `kind`; a mesh (shape 4) is a shape GTA V's has not, which it takes as a point.
fn domain(d: &Domain, kind: usize) -> PtxDomain {
    let k = |i: usize| kfp(&d.kfps[i], DOMAIN_KFPS[kind][i]);
    PtxDomain {
        r#type: kind as i32,
        shape: d.shape as i32,
        is_world_space: flag(d.flags[0]),
        is_point_relative: flag(d.flags[1]),
        is_creation_relative: flag(d.flags[2]),
        is_target_relative: flag(d.flags[3]),
        position_kfp: k(0),
        rotation_kfp: k(1),
        size_outer_kfp: k(2),
        size_inner_kfp: k(3),
        file_version: d.file_version,
        ..Default::default()
    }
}

fn scalars(s: &SpawnedScalars) -> PtxSpawnedEffectScalars {
    PtxSpawnedEffectScalars {
        duration_scalar: s.duration,
        playback_rate_scalar: s.playback_rate,
        colour_tint_scalar: s.colour_tint,
        zoom_scalar: s.zoom,
        flags: s.flags,
        ..Default::default()
    }
}

/// A spawner naming its effect, whose key the list's canonical form gives.
fn spawner(s: &Spawner) -> PtxEffectSpawner<Gen8> {
    PtxEffectSpawner {
        spawned_effect_scalars_min: scalars(&s.scalars_min),
        spawned_effect_scalars_max: scalars(&s.scalars_max),
        name: s.name.clone(),
        trigger_info: s.trigger_info,
        inherits_point_life: flag(s.flags[0]),
        tracks_point_pos: flag(s.flags[1]),
        tracks_point_dir: flag(s.flags[2]),
        tracks_point_neg_dir: flag(s.flags[3]),
        ..Default::default()
    }
}

fn particle_rule(list: &ParticleEffects, p: &ParticleRule) -> PtxParticleRule<Gen8> {
    let all_behaviours = (p.behaviours.iter().enumerate())
        .filter_map(|(place, b)| behaviour(list.behaviours.get(*b)?, place))
        .map(|b| Ref::Own(Box::new(b)))
        .collect();
    let drawables = (p.drawables.iter())
        .map(|d| PtxDrawable {
            name: d.name.clone(),
            hash_name: Hash32::from_raw(d.hash),
            ..Default::default()
        })
        .collect();
    PtxParticleRule {
        effect_spawner_at_ratio: spawner(&p.spawners[0]),
        effect_spawner_on_coln: spawner(&p.spawners[1]),
        tex_frame_id_min: p.tex_frame_id[0],
        tex_frame_id_max: p.tex_frame_id[1],
        name: p.name.clone(),
        all_behaviours,
        drawables,
        sort_type: p.sort_type,
        draw_type: p.draw_type,
        ..Default::default()
    }
}

fn base(hash_name: u32) -> PtxBehaviour {
    PtxBehaviour {
        hash_name,
        ..Default::default()
    }
}

/// Whether the behaviour at `place` of its rule is taken as `ptxu_Age` by its place and size.
fn aged(b: &Behaviour, place: usize) -> bool {
    b.class.is_none() && place == 0 && b.size == FIRST_BEHAVIOUR
}

/// The GTA V behaviour of `b`'s class with the values GTA V names; `None` for a class not known.
pub(super) fn behaviour(b: &Behaviour, place: usize) -> Option<PtxBehaviourAny> {
    use PtxBehaviourAny as B;
    let k = |id: u32| kfp_of(b, id, id);
    if aged(b, place) {
        return Some(B::PtxuAge(PtxuAge {
            base: base(BEHAVIOUR_HASH_AGE),
            ..Default::default()
        }));
    }
    if let BehaviourFields::Sprite {
        flip_chance: [flip_u, flip_v],
        alignment,
        ..
    } = b.fields
    {
        return Some(B::PtxdSprite(PtxdSprite {
            base: base(BEHAVIOUR_HASH_SPRITE),
            alignment_mode: alignment as i32,
            flip_chance_u: flip_u,
            flip_chance_v: flip_v,
            ..Default::default()
        }));
    }
    Some(match b.class? {
        "ptxu_Acceleration" => B::PtxuAcceleration(PtxuAcceleration {
            base: base(BEHAVIOUR_HASH_ACCELERATION),
            xyz_min_kfp: k(KFP_ACCELERATION[0]),
            xyz_max_kfp: k(KFP_ACCELERATION[1]),
            ..Default::default()
        }),
        "ptxu_Velocity" => B::PtxuVelocity(PtxuVelocity {
            base: base(BEHAVIOUR_HASH_VELOCITY),
            ..Default::default()
        }),
        "ptxu_Rotation" => B::PtxuRotation(PtxuRotation {
            base: base(BEHAVIOUR_HASH_ROTATION),
            initial_angle_min_kfp: k(KFP_ROTATION[0]),
            initial_angle_max_kfp: k(KFP_ROTATION[1]),
            angle_min_kfp: k(KFP_ROTATION[2]),
            angle_max_kfp: k(KFP_ROTATION[3]),
            ..Default::default()
        }),
        "ptxu_Size" => B::PtxuSize(PtxuSize {
            base: base(BEHAVIOUR_HASH_SIZE),
            whd_min_kfp: k(KFP_SIZE[0]),
            whd_max_kfp: k(KFP_SIZE[1]),
            tblr_scalar_kfp: k(KFP_SIZE[2]),
            tblr_vel_scalar_kfp: k(KFP_SIZE[3]),
            ..Default::default()
        }),
        "ptxu_Colour" => B::PtxuColour(PtxuColour {
            base: base(BEHAVIOUR_HASH_COLOUR),
            rgba_min_kfp: kfp_of(b, COLOUR_RGBA_KFPS[0], KFP_COLOUR[0]),
            rgba_max_kfp: kfp_of(b, COLOUR_RGBA_KFPS[1], KFP_COLOUR[1]),
            emissive_intensity_kfp: k(KFP_COLOUR[2]),
            rgba_max_enable: colour_max::max_enable(b),
            ..Default::default()
        }),
        "ptxu_AnimateTexture" => {
            let mut animated = PtxuAnimateTexture {
                base: base(BEHAVIOUR_HASH_ANIMATE_TEXTURE),
                anim_rate_kfp: k(KFP_ANIMATE_TEXTURE[0]),
                ..Default::default()
            };
            if let BehaviourFields::AnimateTexture {
                keyframe_mode,
                last_frame_id,
                loop_mode,
                ..
            } = b.fields
            {
                // A last frame id of -1 is a one-frame atlas.
                animated.keyframe_mode = keyframe_mode;
                animated.last_frame_id = last_frame_id.max(0);
                animated.loop_mode = loop_mode;
            }
            B::PtxuAnimateTexture(animated)
        }
        "ptxu_Dampening" => B::PtxuDampening(PtxuDampening {
            base: base(BEHAVIOUR_HASH_DAMPENING),
            xyz_min_kfp: k(KFP_DAMPENING[0]),
            xyz_max_kfp: k(KFP_DAMPENING[1]),
            ..Default::default()
        }),
        "ptxu_MatrixWeight" => B::PtxuMatrixWeight(PtxuMatrixWeight {
            base: base(BEHAVIOUR_HASH_MATRIX_WEIGHT),
            mtx_weight_kfp: k(KFP_MATRIX_WEIGHT[0]),
            ..Default::default()
        }),
        "ptxu_Collision" => B::PtxuCollision(PtxuCollision {
            base: base(BEHAVIOUR_HASH_COLLISION),
            bounciness_kfp: k(KFP_COLLISION[0]),
            bounce_dir_var_kfp: k(KFP_COLLISION[1]),
            ..Default::default()
        }),
        "ptxu_Wind" => B::PtxuWind(PtxuWind {
            base: base(BEHAVIOUR_HASH_WIND),
            influence_kfp: k(KFP_WIND[0]),
            ..Default::default()
        }),
        "ptxu_Noise" => B::PtxuNoise(PtxuNoise {
            base: base(BEHAVIOUR_HASH_NOISE),
            pos_noise_min_kfp: k(KFP_NOISE[0]),
            pos_noise_max_kfp: k(KFP_NOISE[1]),
            vel_noise_min_kfp: k(KFP_NOISE[2]),
            vel_noise_max_kfp: k(KFP_NOISE[3]),
            ..Default::default()
        }),
        // ⚠ GTA VI keys the strength under its own class; GTA V under ptxu_Acceleration's.
        "ptxu_Attractor" => B::PtxuAttractor(PtxuAttractor {
            base: base(BEHAVIOUR_HASH_ATTRACTOR),
            strength_kfp: kfp_of(
                b,
                Hash32::of("ptxu_Attractor:m_strengthKFP").get(),
                KFP_ATTRACTOR[0],
            ),
            ..Default::default()
        }),
        "ptxd_Trail" => B::PtxdTrail(PtxdTrail {
            base: base(BEHAVIOUR_HASH_TRAIL),
            tex_info_kfp: k(KFP_TRAIL[0]),
            ..Default::default()
        }),
        "ptxu_Light" => B::PtxuLight(PtxuLight {
            base: base(BEHAVIOUR_HASH_LIGHT),
            rgb_min_kfp: k(KFP_LIGHT[0]),
            rgb_max_kfp: k(KFP_LIGHT[1]),
            intensity_kfp: k(KFP_LIGHT[2]),
            range_kfp: k(KFP_LIGHT[3]),
            corona_rgb_min_kfp: k(KFP_LIGHT[4]),
            corona_rgb_max_kfp: k(KFP_LIGHT[5]),
            corona_intensity_kfp: k(KFP_LIGHT[6]),
            corona_size_kfp: k(KFP_LIGHT[7]),
            corona_flare_kfp: k(KFP_LIGHT[8]),
            ..Default::default()
        }),
        "ptxu_Decal" => B::PtxuDecal(PtxuDecal {
            base: base(BEHAVIOUR_HASH_DECAL),
            dimensions_kfp: k(KFP_DECAL[0]),
            alpha_kfp: k(KFP_DECAL[1]),
            ..Default::default()
        }),
        "ptxu_ZCull" => {
            let mut zcull = PtxuZCull {
                base: base(BEHAVIOUR_HASH_ZCULL),
                height_kfp: k(KFP_ZCULL[0]),
                fade_dist_kfp: k(KFP_ZCULL[1]),
                ..Default::default()
            };
            if let BehaviourFields::ZCull {
                mode,
                reference_space,
            } = b.fields
            {
                (zcull.mode, zcull.reference_space) = (mode, reference_space);
            }
            B::PtxuZCull(zcull)
        }
        "ptxu_FogVolume" => B::PtxuFogVolume(PtxuFogVolume {
            base: base(BEHAVIOUR_HASH_FOG_VOLUME),
            rgb_tint_min_kfp: k(KFP_FOG_VOLUME[0]),
            rgb_tint_max_kfp: k(KFP_FOG_VOLUME[1]),
            density_range_kfp: k(KFP_FOG_VOLUME[2]),
            scale_min_kfp: k(KFP_FOG_VOLUME[3]),
            scale_max_kfp: k(KFP_FOG_VOLUME[4]),
            rotation_min_kfp: k(KFP_FOG_VOLUME[5]),
            rotation_max_kfp: k(KFP_FOG_VOLUME[6]),
            ..Default::default()
        }),
        "ptxu_CameraEvolution" => B::PtxuCameraEvolution(PtxuCameraEvolution {
            base: base(Hash32::of("ptxu_CameraEvolution").get()),
            evolution_curve: k(Hash32::of("ptxu_CameraEvolution:m_evolutionCurve").get()),
            ..Default::default()
        }),
        _ => return None,
    })
}

/// What the list states that its GTA V form has no place for, and what is not known.
pub(super) fn lines(list: &ParticleEffects) -> Vec<String> {
    let mut lines = vec![
        tr!(
            "A GTA VI particle effects list named {name}: {effects} effect rules, {emitters} emitter rules and {particles} particle rules, shown in the Particle workspace as a GTA V list built from them.",
            name = list.name,
            effects = list.effects.len(),
            emitters = list.emitters.len(),
            particles = list.particles.len(),
        ),
        tr!("A sprite is drawn from its rule's texture slot 0, else 6, else 5, else 12, once a texture pack open in the app holds its key. Slot 6 is coloured from the ramp slot 7 holds, whose four rows are one row, at the place its value gives, which is inferred; it is drawn grey until a pack holding the ramp is open, and where slot 7 is not a BC6H texture 4 texels high, which is no ramp. Slot 5 is drawn opaque. Most slot 4 and 9 textures are single-channel masks, slot 10's two-channel vector maps and slot 12's grey layers; what the shader does with them is not known.").to_string(),
        tr!("A rule's atlas grid is not stated: it is cut into the square cells that hold exactly its frames where such a grid exists, else guessed as GTA V guesses it.").to_string(),
        tr!("Keyframed values GTA V's records have no member for are listed at the end, raw: a velocity's angle, a colour's last three unnamed values and a domain's fifth and sixth. A colour's first two unnamed values are shown as its minimum and maximum, which is inferred; the preview draws the maximum, as GTA V does, where the colour's word at +0x19C counts two or more colours in use, which is inferred too.").to_string(),
        tr!("An evolution's evolved keyframes are read and the evolution sliders play them. A keyframe's kind is its evolution's source: 1 the LOD distance, 2 to 4 the camera's forward, top and side views, whose levels the sliders set, since how the game computes them is not stated. Each keyframe's two flags are listed at the end; what they select is not known.").to_string(),
        tr!("The GPU buffers in the physical section are checked: the one at +0x390 copies the rule's floats at +0x1E0, and the one at +0x388 holds two tables of 16 rows whose values are not derived from the rule, so they are kept as stored and may disagree with an edited keyframed value. The preview needs neither.").to_string(),
        tr!("Save As writes the values the two forms share over the GTA VI list in place, and with them a rule removed or an event bound to another rule. A key added, an effect, an emitter rule, a particle rule or an event duplicated, and a behaviour copied to another rule are written past the list's end, a rule's copy found by the name Duplicate gives it, and a particle rule's copy given GPU buffers of its own. A rule made from nothing has no stated value, so the workspace does not offer it.").to_string(),
    ];
    for e in &list.emitters {
        lines.push(tr!(
            "Emitter rule {name}: flags {flags}. Byte 0 reads as GTA V's one-shot flag, which is not established, so the preview does not use it.",
            name = e.name,
            flags = format!("{:#010X}", e.flags),
        ));
        for (slot, d) in e.domains.iter().enumerate() {
            if let Some(Domain {
                model: Some((model, _)),
                ..
            }) = d
            {
                lines.push(tr!(
                    "Its domain {slot} is a mesh of the model \"{model}\": once a search of the open scans finds the model, the preview takes its surface as the domain, and the domain's box before.",
                    slot = slot,
                    model = model,
                ));
            }
        }
    }
    for p in &list.particles {
        rule_lines(list, p, &mut lines);
    }
    raw::lines(list, &mut lines);
    lines
}

fn rule_lines(list: &ParticleEffects, p: &ParticleRule, lines: &mut Vec<String>) {
    lines.push(tr!(
        "Particle rule {name}: draw type {draw}, sort type {sort}, shader hash_{shader}, whose name is not known.",
        name = p.name,
        draw = p.draw_type,
        sort = p.sort_type,
        shader = format!("{:08X}", p.shader_hash),
    ));
    shading_lines(list, p, lines);
    for (place, b) in p.behaviours.iter().enumerate() {
        let Some(b) = list.behaviours.get(*b) else {
            continue;
        };
        match b.class {
            Some(class) => lines.push(tr!(
                "Behaviour {place}: {class}, {size} bytes to the next behaviour.",
                place = place,
                class = class,
                size = b.size,
            )),
            None if aged(b, place) => lines.push(tr!(
                "Behaviour {place}: {size} bytes and first in its rule, so shown as ptxu_Age by its place and size; no keyframed value names its class.",
                place = place,
                size = b.size,
            )),
            None => unclassed(place, b, lines),
        }
        if let BehaviourFields::Sprite { alignment, .. } = b.fields {
            if b.class.is_none() && alignment > GTA5_ALIGNMENTS {
                lines.push(tr!(
                    "Its alignment mode {alignment} is not known, so the preview faces it to the camera.",
                    alignment = alignment,
                ));
            }
        }
        if let BehaviourFields::ZCull { mode, .. } = b.fields {
            if mode > GTA5_ZCULL_MODES {
                lines.push(tr!(
                    "Its ptxu_ZCull's mode {mode} is not one GTA V has, so the preview culls under its height.",
                    mode = mode,
                ));
            }
        }
        colour_max::lines(b, lines);
    }
    for (slot, key) in p.texture_keys.iter().enumerate().filter(|(_, k)| **k != 0) {
        lines.push(tr!(
            "Texture slot {slot}: key {key}.",
            slot = slot,
            key = format!("{key:016X}"),
        ));
    }
    if let Some(effect) = p.spawners[2].effect.and_then(|i| list.effects.get(i)) {
        lines.push(tr!(
            "Its third spawner starts {effect} where its ptxu_ZCull culls a point, and so does the preview; that its trigger value is a speed the point must reach, as the collision spawner's is, is inferred.",
            effect = effect.name,
        ));
    }
    for d in &p.drawables {
        lines.push(tr!(
            "It names the drawable {drawable}, a resource of its own, which the preview draws once a search of the open scans finds it, with its materials' base-colour textures once a texture pack open in the app holds their keys.",
            drawable = d.name,
        ));
    }
}

/// The lines of a behaviour no keyframed value classes, by the class its bit gives it.
fn unclassed(place: usize, b: &Behaviour, lines: &mut Vec<String>) {
    let size = b.size;
    // Read only by the arms after the first, where a rule lists the behaviour.
    let bit = b.class_bit.unwrap_or_default();
    lines.push(match (b.fields, b.class_bit.and_then(class_of_bit)) {
        _ if b.class_bit.is_none() => tr!(
            "Behaviour {place}: {size} bytes, which no rule lists, so its class bit is not known and the preview leaves it out.",
            place = place,
            size = size,
        ),
        (BehaviourFields::Age, _) => tr!(
            "Behaviour {place}: {size} bytes, labelled ptxu_Age by its class bit, which the preview needs nothing from.",
            place = place,
            size = size,
        ),
        (
            BehaviourFields::Sprite {
                flip_chance: [flip_u, flip_v],
                alignment,
                ..
            },
            _,
        ) => tr!(
            "Behaviour {place}: {size} bytes, shown as ptxd_Sprite by its class bit: alignment mode {alignment}, flip chances {flip_u} and {flip_v}; its other bytes are not known.",
            place = place,
            size = size,
            alignment = alignment,
            flip_u = flip_u,
            flip_v = flip_v,
        ),
        (_, Some((Some(class), ClassNamedBy::Order))) => tr!(
            "Behaviour {place}: {size} bytes, of class bit {bit}, which GTA V's update order names {class}; the preview does not play it, since its members are not laid out.",
            place = place,
            size = size,
            bit = bit,
            class = class,
        ),
        (_, Some((Some(class), _))) => tr!(
            "Behaviour {place}: {size} bytes, of class bit {bit}, {class}'s, though no keyframed value it holds has an id the reader knows; the preview leaves it out.",
            place = place,
            size = size,
            bit = bit,
            class = class,
        ),
        _ => tr!(
            "Behaviour {place}: {size} bytes, of class bit {bit}, a class GTA V does not have, so the preview leaves it out.",
            place = place,
            size = size,
            bit = bit,
        ),
    });
    if let BehaviourFields::Unclassed(id) = b.fields {
        lines.push(tr!(
            "Its keyframed value's id is hash_{id}, whose member name is not known.",
            id = format!("{id:08X}"),
        ));
    }
}

/// What `p`'s shader words select, and the texture slot its sprite is drawn from.
fn shading_lines(list: &ParticleEffects, p: &ParticleRule, lines: &mut Vec<String>) {
    if let Ok(shading) = p.shading() {
        lines.push(tr!(
            "Its shader words select lightingmode {lighting}, projectionmode {projection} and diffusemode {diffuse}.",
            lighting = shading.lighting.name(),
            projection = shading.projection.name(),
            diffuse = shading.diffuse.name(),
        ));
        if !shading.switches.is_empty() {
            let switches: Vec<&str> = shading.switches.iter().map(|s| s.name()).collect();
            lines.push(tr!(
                "Its shader switches: {switches}.",
                switches = switches.join(", "),
            ));
        }
    }
    if p.draw_type == 1 {
        return;
    }
    lines.push(match p.sprite_slot() {
        Some(slot) => tr_n!(
            "Its sprite is texture slot {slot}, of {count} frame.",
            "Its sprite is texture slot {slot}, of {count} frames.",
            list.frame_count(p),
            slot = slot,
        ),
        None => tr!("It sets none of the sprite's texture slots 0, 6, 5 and 12.").to_string(),
    });
}

#[cfg(test)]
#[path = "gta6_particles_tests.rs"]
mod tests;
