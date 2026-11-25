//! A scanned GTA VI particle effects list's edits saved: the rules its GTA V form no longer holds
//! taken out, the effects, rules, events and behaviours it copied and the keys it added written
//! past the section's end, then the values the two forms share and each event's rules written
//! over the body's sections in place on the save's worker, and any edit with no place in the list
//! refused.

use std::collections::HashSet;
use std::sync::Arc;

use or_io::container::rpf::Rpf9Sections;
use or_io::decl::{Bool8, Gen8, Hash32, Ref};
use or_io::gta5::particle::*;
use or_io::gta5::{Vec3V, Vec4V};
use or_io::gta6::headerless::{
    add_particle_keys, duplicate_particle_effect, read_particle_effects,
    remove_from_particle_effects, write_particle_effects, EffectRule, EmitterRule, Kfp, KfpEntry,
    ParticleEffects, SpawnedScalars,
};
use or_particle::list::{
    canonical, effect_rules, effect_rules_mut, emitter_rules, events, events_mut, particle_rules,
};

#[path = "gta6_particles_remove.rs"]
pub(super) mod remove;

#[path = "gta6_particles_copies.rs"]
mod copies;

use super::document::Document;
use super::gta6_edit::EditSections;
use super::gta6_resource_save::EditedSave;
use super::DocPayload;

/// The Ypt row's `edited_sections` where `document` is a scanned GTA VI list holding an edit: its
/// body read again on the worker and its values written over it; `None` for any other document.
pub(super) fn edited_save(document: &Document) -> Option<Result<EditedSave, String>> {
    if !document.history.can_undo() {
        return None;
    }
    let path = document.path.as_deref()?;
    super::gta6_particles::scanned_lines(path)?;
    let DocPayload::Resource(or_io::Asset::Particles(ypt)) = &document.payload else {
        return None;
    };
    let (archive, offset) = super::rpf9_scan::scanned_body(path)?;
    let ypt = ypt.clone();
    let edit: Arc<EditSections> = Arc::new(move |_, sections: &mut Rpf9Sections| {
        // Every write goes to a copy, so a refused one leaves the sections as they were.
        let mut s = sections.clone();
        let read = |s: &Rpf9Sections| {
            read_particle_effects(&s.virtual_section, None).map_err(|e| e.to_string())
        };
        let removals = remove::removals(&read(&s)?, &ypt);
        if !removals.is_empty() {
            remove_from_particle_effects(&mut s.virtual_section, &removals)
                .map_err(|e| e.to_string())?;
        }
        for (from, name) in duplicates(&read(&s)?, &ypt)? {
            let stored = read(&s)?;
            let Some(source) = stored.effects.iter().position(|e| e.name == from) else {
                return Err(not_a_copy());
            };
            duplicate_particle_effect(&mut s, source, &name).map_err(|e| e.to_string())?;
        }
        copies::written(&mut s, &ypt)?;
        let ypt = copies::ordered(&read(&s)?, &ypt);
        let keyed = from_ypt(&read(&s)?, &ypt)?;
        add_particle_keys(&mut s, &keyed).map_err(|e| e.to_string())?;
        let edited = from_ypt(&read(&s)?, &ypt)?;
        write_particle_effects(&mut s.virtual_section, &edited).map_err(|e| e.to_string())?;
        *sections = s;
        Ok(())
    });
    Some(Ok(EditedSave::Body {
        archive: archive.to_path_buf(),
        offset,
        edit,
    }))
}

fn moved() -> String {
    tr!("A rule, an event or a behaviour was added, removed or renamed: a GTA VI particle list is written in place, so its rules stay as they are stored.").to_string()
}

/// `stored` given the values its GTA V form `ypt` shows; refused where an edit of `ypt` has no
/// place in the GTA VI list.
pub(super) fn from_ypt(
    stored: &ParticleEffects,
    ypt: &PtxFxList,
) -> Result<ParticleEffects, String> {
    let mut edited = ypt.clone();
    canonical(&mut edited);
    unbound_dropped(&mut edited);
    let mut list = stored.clone();
    let effects: Vec<&PtxEffectRule<Gen8>> = effect_rules(&edited).collect();
    let emitters: Vec<&PtxEmitterRule> = emitter_rules(&edited).collect();
    let particles: Vec<&PtxParticleRule<Gen8>> = particle_rules(&edited).collect();
    let same = effects.len() == list.effects.len()
        && emitters.len() == list.emitters.len()
        && particles.len() == list.particles.len();
    if !same {
        return Err(moved());
    }
    let names = Names {
        emitters: list.emitters.iter().map(|r| r.name.clone()).collect(),
        particles: list.particles.iter().map(|r| r.name.clone()).collect(),
    };
    for (to, from) in list.effects.iter_mut().zip(effects) {
        effect(to, from, &names)?;
    }
    for (to, from) in list.emitters.iter_mut().zip(emitters) {
        emitter(to, from);
    }
    for (i, from) in particles.into_iter().enumerate() {
        particle(&mut list, i, from)?;
    }
    let mut back = super::gta6_particles::as_ypt(&list);
    canonical(&mut back);
    if back != edited {
        return Err(tr!("An edit has no place in the GTA VI particle list, which keeps only the values both games share: undo it to save the others.").to_string());
    }
    Ok(list)
}

fn not_a_copy() -> String {
    tr!("A GTA VI particle list takes a new effect only as a copy of one it holds: use Duplicate, and keep the copy's name the name of the effect it copies followed by _copy.").to_string()
}

/// Each effect `ypt` holds that `stored` does not, with the stored effect it copies: the one whose
/// name followed by `_copy` begins its name, the longest where several do.
pub(super) fn duplicates(
    stored: &ParticleEffects,
    ypt: &PtxFxList,
) -> Result<Vec<(String, String)>, String> {
    let held: HashSet<&str> = stored.effects.iter().map(|e| e.name.as_str()).collect();
    let mut out = Vec::new();
    for rule in effect_rules(ypt).filter(|r| !held.contains(r.name.as_str())) {
        let from = (stored.effects.iter())
            .filter(|e| rule.name.starts_with(&format!("{}_copy", e.name)))
            .max_by_key(|e| e.name.len());
        let Some(from) = from else {
            return Err(not_a_copy());
        };
        out.push((from.name.clone(), rule.name.clone()));
    }
    Ok(out)
}

fn unbound() -> String {
    tr!("An event names a rule the GTA VI particle list does not hold: bind it to one of the list's rules.").to_string()
}

/// `ypt` without the events a rule's deletion left naming no rule, which the list removes with
/// the rule, each effect's other events numbered by their place.
fn unbound_dropped(ypt: &mut PtxFxList) {
    let bound = |e: &Option<Box<PtxEventEmitter<Gen8>>>| {
        e.as_deref()
            .is_none_or(|e| !e.emitter_rule_name.is_empty() && !e.particle_rule_name.is_empty())
    };
    for rule in effect_rules_mut(ypt) {
        let held = rule.timeline.events.len();
        rule.timeline.events.retain(bound);
        if rule.timeline.events.len() == held {
            continue;
        }
        for (place, ev) in events_mut(rule).enumerate() {
            ev.base.index = i32::try_from(place).unwrap_or(i32::MAX);
        }
    }
}

/// The list's emitter and particle rule names, each at its rule's index.
struct Names {
    emitters: Vec<String>,
    particles: Vec<String>,
}

fn index_of(names: &[String], name: &str) -> Result<usize, String> {
    names.iter().position(|n| n == name).ok_or_else(unbound)
}

/// `held` where it already reads as `b`, so a flag not edited keeps its byte.
fn byte(held: u8, b: Bool8) -> u8 {
    match (held != 0) == b.get() {
        true => held,
        false => u8::from(b.get()),
    }
}

fn xyz(v: &Vec3V) -> [f32; 3] {
    [v.x, v.y, v.z]
}

fn lanes(v: &Vec4V) -> [f32; 4] {
    [v.x, v.y, v.z, v.w]
}

fn keys(k: &mut Kfp, prop: &PtxKeyframeProp) {
    k.entries = (prop.keyframe.entries.iter())
        .map(|e| KfpEntry {
            time: lanes(&e.time),
            value: lanes(&e.value),
        })
        .collect();
}

fn effect(to: &mut EffectRule, from: &PtxEffectRule<Gen8>, names: &Names) -> Result<(), String> {
    to.num_loops = from.num_loops;
    let f = &mut to.flags;
    f[0] = byte(f[0], from.sort_events_by_distance);
    f[1] = from.draw_list_id;
    f[2] = byte(f[2], from.is_short_lived);
    f[3] = byte(f[3], from.has_no_shadows);
    to.rand_offset_pos = xyz(&from.rand_offset_pos);
    to.times = [
        from.pre_update_time,
        from.pre_update_time_interval,
        from.duration_min,
        from.duration_max,
        from.playback_rate_scalar_min,
        from.playback_rate_scalar_max,
    ];
    let m = &mut to.culling_modes;
    m[0] = from.viewport_culling_mode;
    m[1] = byte(m[1], from.render_when_viewport_culled);
    m[2] = byte(m[2], from.update_when_viewport_culled);
    m[3] = byte(m[3], from.emit_when_viewport_culled);
    m[4] = from.distance_culling_mode;
    m[5] = byte(m[5], from.render_when_distance_culled);
    m[6] = byte(m[6], from.update_when_distance_culled);
    m[7] = byte(m[7], from.emit_when_distance_culled);
    to.culling_sphere_offset = xyz(&from.viewport_culling_sphere_offset);
    to.culling = [
        from.viewport_culling_sphere_radius,
        from.distance_culling_fade_dist,
        from.distance_culling_cull_dist,
        from.lod_evo_dist_min,
        from.lod_evo_dist_max,
    ];
    to.coln_range = from.coln_range;
    to.coln_probe_dist = from.coln_probe_dist;
    let props = [
        &from.colour_tint_min_kfp,
        &from.colour_tint_max_kfp,
        &from.zoom_scalar_kfp,
        &from.data_sphere_kfp,
        &from.data_capsule_kfp,
    ];
    for (k, prop) in to.kfps.iter_mut().zip(props) {
        keys(k, prop);
    }
    let d = &mut to.data_volume;
    d[0] = byte(d[0], from.colour_tint_max_enable);
    d[1] = byte(d[1], from.use_data_volume);
    d[2] = from.data_volume_type;
    let fired: Vec<&PtxEventEmitter<Gen8>> = events(from).collect();
    if fired.len() != to.events.len() {
        return Err(unbound());
    }
    for (ev, from) in to.events.iter_mut().zip(fired) {
        ev.emitter = index_of(&names.emitters, &from.emitter_rule_name)?;
        ev.particle = index_of(&names.particles, &from.particle_rule_name)?;
        (ev.start_ratio, ev.end_ratio) = (from.base.start_ratio, from.base.end_ratio);
        ev.playback_rate_scalar = [from.playback_rate_scalar_min, from.playback_rate_scalar_max];
        ev.zoom_scalar = [from.zoom_scalar_min, from.zoom_scalar_max];
        ev.colour_tint = [from.colour_tint_min, from.colour_tint_max];
    }
    Ok(())
}

fn emitter(to: &mut EmitterRule, from: &PtxEmitterRule) {
    let props = [
        &from.spawn_rate_over_time_kfp,
        &from.spawn_rate_over_dist_kfp,
        &from.particle_life_kfp,
        &from.playback_rate_scalar_kfp,
        &from.speed_scalar_kfp,
        &from.size_scalar_kfp,
        &from.accn_scalar_kfp,
        &from.dampening_scalar_kfp,
        &from.matrix_weight_scalar_kfp,
        &from.inherit_velocity_kfp,
    ];
    for (k, prop) in to.kfps.iter_mut().zip(props) {
        keys(k, prop);
    }
    let held = [
        &from.creation_domain_obj,
        &from.target_domain_obj,
        &from.attractor_domain_obj,
    ];
    for (slot, obj) in to.domains.iter_mut().zip(held) {
        let (Some(d), Some(from)) = (slot.as_mut(), obj.domain.as_deref()) else {
            continue;
        };
        let f = &mut d.flags;
        f[0] = byte(f[0], from.is_world_space);
        f[1] = byte(f[1], from.is_point_relative);
        f[2] = byte(f[2], from.is_creation_relative);
        f[3] = byte(f[3], from.is_target_relative);
        let props = [
            &from.position_kfp,
            &from.rotation_kfp,
            &from.size_outer_kfp,
            &from.size_inner_kfp,
        ];
        // The fifth and sixth have no GTA V member and stay as stored.
        for (k, prop) in d.kfps.iter_mut().zip(props) {
            keys(k, prop);
        }
    }
}

fn scalars(to: &mut SpawnedScalars, from: &PtxSpawnedEffectScalars) {
    to.duration = from.duration_scalar;
    to.playback_rate = from.playback_rate_scalar;
    to.colour_tint = from.colour_tint_scalar;
    to.zoom = from.zoom_scalar;
    to.flags = from.flags;
}

fn particle(
    list: &mut ParticleEffects,
    index: usize,
    from: &PtxParticleRule<Gen8>,
) -> Result<(), String> {
    let Some(rule) = list.particles.get_mut(index) else {
        return Err(moved());
    };
    let spawners = [&from.effect_spawner_at_ratio, &from.effect_spawner_on_coln];
    // The third spawner has no GTA V member and stays as stored.
    for (s, from) in rule.spawners.iter_mut().zip(spawners) {
        scalars(&mut s.scalars_min, &from.spawned_effect_scalars_min);
        scalars(&mut s.scalars_max, &from.spawned_effect_scalars_max);
        s.trigger_info = from.trigger_info;
        let f = &mut s.flags;
        f[0] = byte(f[0], from.inherits_point_life);
        f[1] = byte(f[1], from.tracks_point_pos);
        f[2] = byte(f[2], from.tracks_point_dir);
        f[3] = byte(f[3], from.tracks_point_neg_dir);
    }
    rule.tex_frame_id = [from.tex_frame_id_min, from.tex_frame_id_max];
    rule.sort_type = from.sort_type;
    let listed = rule.behaviours.clone();
    // The GTA V form holds the behaviours of a class it names, in the rule's order.
    let mut shown = Vec::new();
    for (place, &b) in listed.iter().enumerate() {
        let held = list.behaviours.get(b);
        let shows = held.and_then(|held| super::gta6_particles::behaviour(held, place));
        if shows.is_some() {
            shown.push(b);
        }
    }
    let owned: Vec<&PtxBehaviourAny> = (from.all_behaviours.iter())
        .filter_map(|b| match b {
            Ref::Own(b) => Some(&**b),
            Ref::To(_) => None,
        })
        .collect();
    if owned.len() != shown.len() {
        return Err(moved());
    }
    for (at, b) in shown.into_iter().zip(owned) {
        let Some(held) = list.behaviours.get_mut(at) else {
            continue;
        };
        // Only an edit of the enable shown changes the count, which may read on and be shown off.
        if let PtxBehaviourAny::PtxuColour(c) = b {
            if c.rgba_max_enable != super::gta6_particles::colour_max::max_enable(held) {
                held.fields = held.fields.with_colour_max_enabled(c.rgba_max_enable.get());
            }
        }
        super::gta6_particles::written_fields(b, held);
        for (id, prop) in behaviour_kfps(b) {
            if let Some(k) = held.kfps.iter_mut().find(|k| k.id == id) {
                keys(k, prop);
            }
        }
    }
    Ok(())
}

/// The keyframed values the GTA V form of a GTA VI behaviour holds, by the GTA VI id each is read
/// from.
fn behaviour_kfps(b: &PtxBehaviourAny) -> Vec<(u32, &PtxKeyframeProp)> {
    use PtxBehaviourAny as B;
    let props: Vec<&PtxKeyframeProp> = match b {
        B::PtxuAcceleration(x) => vec![&x.xyz_min_kfp, &x.xyz_max_kfp],
        B::PtxuRotation(x) => vec![
            &x.initial_angle_min_kfp,
            &x.initial_angle_max_kfp,
            &x.angle_min_kfp,
            &x.angle_max_kfp,
        ],
        B::PtxuSize(x) => vec![
            &x.whd_min_kfp,
            &x.whd_max_kfp,
            &x.tblr_scalar_kfp,
            &x.tblr_vel_scalar_kfp,
        ],
        B::PtxuColour(x) => vec![&x.emissive_intensity_kfp],
        B::PtxuAnimateTexture(x) => vec![&x.anim_rate_kfp],
        B::PtxuDampening(x) => vec![&x.xyz_min_kfp, &x.xyz_max_kfp],
        B::PtxuMatrixWeight(x) => vec![&x.mtx_weight_kfp],
        B::PtxuCollision(x) => vec![&x.bounciness_kfp, &x.bounce_dir_var_kfp],
        B::PtxuWind(x) => vec![&x.influence_kfp],
        B::PtxuNoise(x) => vec![
            &x.pos_noise_min_kfp,
            &x.pos_noise_max_kfp,
            &x.vel_noise_min_kfp,
            &x.vel_noise_max_kfp,
        ],
        // ⚠ GTA VI keys the strength under its own class; GTA V under ptxu_Acceleration's.
        B::PtxuAttractor(x) => {
            let id = Hash32::of("ptxu_Attractor:m_strengthKFP").get();
            return vec![(id, &x.strength_kfp)];
        }
        B::PtxdTrail(x) => vec![&x.tex_info_kfp],
        B::PtxuLight(x) => vec![
            &x.rgb_min_kfp,
            &x.rgb_max_kfp,
            &x.intensity_kfp,
            &x.range_kfp,
            &x.corona_rgb_min_kfp,
            &x.corona_rgb_max_kfp,
            &x.corona_intensity_kfp,
            &x.corona_size_kfp,
            &x.corona_flare_kfp,
        ],
        B::PtxuDecal(x) => vec![&x.dimensions_kfp, &x.alpha_kfp],
        B::PtxuZCull(x) => vec![&x.height_kfp, &x.fade_dist_kfp],
        B::PtxuFogVolume(x) => vec![
            &x.rgb_tint_min_kfp,
            &x.rgb_tint_max_kfp,
            &x.density_range_kfp,
            &x.scale_min_kfp,
            &x.scale_max_kfp,
            &x.rotation_min_kfp,
            &x.rotation_max_kfp,
        ],
        B::PtxuCameraEvolution(x) => vec![&x.evolution_curve],
        _ => Vec::new(),
    };
    (props.into_iter())
        .map(|k| (k.property_id.get(), k))
        .collect()
}
