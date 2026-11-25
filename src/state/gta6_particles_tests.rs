use std::collections::BTreeSet;

use or_io::decl::OwnedOrKeyed;
use or_io::gta6::headerless::{Evolution, KfpEntry};

use super::*;

fn hash(name: &str) -> u32 {
    Hash32::of(name).get()
}

const RATE: [([f32; 4], [f32; 4]); 1] = [([0.0; 4], [10.0, 12.0, 0.0, 0.0])];

fn value(id: u32, entries: &[([f32; 4], [f32; 4])]) -> Kfp {
    Kfp {
        id,
        base_id: id,
        entries: (entries.iter())
            .map(|&(time, value)| KfpEntry { time, value })
            .collect(),
    }
}

fn scalars(zoom: f32) -> SpawnedScalars {
    SpawnedScalars {
        duration: 1.5,
        playback_rate: 0.5,
        colour_tint: 0xFF20_4080,
        zoom,
        flags: 8,
    }
}

fn spawner(effect: Option<usize>, name: &str) -> Spawner {
    Spawner {
        scalars_min: scalars(90.0),
        scalars_max: scalars(110.0),
        effect,
        name: name.to_string(),
        trigger_info: 0.25,
        flags: [1, 0, 1, 0],
    }
}

fn domain(kind: usize, shape: u32, model: Option<&str>) -> Domain {
    Domain {
        shape,
        flags: [1, 0, 0, 1],
        file_version: 2.1,
        kfps: std::array::from_fn(|i| value(DOMAIN_KFPS[kind][i], &[])),
        model: model.map(|m| (m.to_string(), hash(m))),
    }
}

fn behaviour(class: Option<&'static str>, size: u64, kfps: Vec<Kfp>) -> Behaviour {
    Behaviour {
        at: 0,
        class,
        class_bit: Some(0),
        size,
        kfps,
        fields: Default::default(),
    }
}

/// One effect firing one emitter (a mesh creation domain, a sphere target) at one particle rule
/// that lists a first 0x10 behaviour, a `ptxu_Size`, a behaviour of no known class and a
/// `ptxu_Attractor`, and starts the effect from its first and third spawners.
fn list() -> ParticleEffects {
    let event = Event {
        start_ratio: 0.25,
        end_ratio: 0.75,
        emitter: 0,
        particle: 0,
        evolution_list: None,
        playback_rate_scalar: [1.0, 2.0],
        zoom_scalar: [3.0, 4.0],
        colour_tint: [0xFF00_FF00, 0xFFFF_FFFF],
    };
    let effect = EffectRule {
        at: 0x100,
        code: hash("fx_a"),
        name: "fx_a".into(),
        events: vec![event],
        evolution_list: Some(EvolutionList {
            evolutions: vec![Evolution {
                name: "speed".into(),
                override_value: 0.5,
                is_overridden: true,
            }],
            props: vec![],
        }),
        num_loops: -1,
        flags: [1, 2, 0, 1],
        rand_offset_pos: [0.5, 1.5, 2.5],
        times: [0.1, 0.2, 1.25, 2.0, 0.5, 1.5],
        culling_modes: [4, 0, 1, 0, 3, 1, 0, 1],
        culling_sphere_offset: [0.0, 0.0, 0.5],
        culling: [1.0, 20.0, 40.0, 10.0, 30.0],
        coln_range: 3.0,
        coln_probe_dist: 360.0,
        kfps: std::array::from_fn(|i| value(EFFECT_KFPS[i], &[])),
        data_volume: [0, 1, 2],
    };
    let emitter = EmitterRule {
        at: 0x400,
        code: hash("emit_a"),
        name: "emit_a".into(),
        domains: [
            Some(domain(0, 4, Some("mesh_a"))),
            Some(domain(1, 1, None)),
            None,
        ],
        kfps: std::array::from_fn(|i| value(EMITTER_KFPS[i], &RATE[..usize::from(i == 0)])),
        flags: 1,
    };
    let mut keys = [0u64; 13];
    keys[4] = 0x0A0B_0C0D_0E0F_1011;
    let particle = ParticleRule {
        at: 0x800,
        code: hash("part_a"),
        name: "part_a".into(),
        spawners: [
            spawner(Some(0), "fx_a"),
            spawner(None, ""),
            spawner(Some(0), "fx_a"),
        ],
        tex_frame_id: [2, 5],
        behaviours: vec![0, 1, 2, 3],
        class_bits: 0,
        texture_keys: keys,
        shader_words: vec![(0x73A7_7B34, 0)],
        shader_hash: 0x3FEE_018B,
        drawables: vec![],
        sort_type: 1,
        draw_type: 0,
    };
    let whd = [([0.0; 4], [1.0, 2.0, 3.0, 0.0])];
    let strength = [([0.0; 4], [7.0, 0.0, 0.0, 0.0])];
    let mut unclassed = behaviour(None, 0x50, vec![]);
    unclassed.class_bit = Some(26);
    let behaviours = vec![
        behaviour(None, 0x10, vec![]),
        behaviour(Some("ptxu_Size"), 0x118, vec![value(KFP_SIZE[1], &whd)]),
        unclassed,
        behaviour(
            Some("ptxu_Attractor"),
            0x90,
            vec![value(hash("ptxu_Attractor:m_strengthKFP"), &strength)],
        ),
    ];
    ParticleEffects {
        name: "test_list".into(),
        registry: vec![],
        effects: vec![effect],
        emitters: vec![emitter],
        particles: vec![particle],
        behaviours,
        drawable_hashes: BTreeSet::new(),
        mesh_hashes: BTreeSet::from([hash("mesh_a")]),
    }
}

fn first<T: Owned<Raw: Record>>(dict: &Option<Box<PgDictionary<T>>>) -> &T {
    let dict = dict.as_deref().expect("a dictionary");
    dict.entries[0].as_deref().expect("an entry")
}

#[test]
fn a_gta_vi_list_becomes_a_gta_v_list_whose_events_and_spawners_name_their_rules() {
    let mut ypt = as_ypt(&list());
    or_particle::list::canonical(&mut ypt);
    let effects = ypt.effect_rule_dict.as_deref().expect("effects");
    assert_eq!(effects.codes, [hash("fx_a")]);
    let effect = first(&ypt.effect_rule_dict);
    assert_eq!(effect.name, "fx_a");
    assert_eq!(effect.num_loops, -1);
    assert_eq!((effect.duration_min, effect.duration_max), (1.25, 2.0));
    assert_eq!(effect.draw_list_id, 2);
    assert_eq!(
        (effect.viewport_culling_mode, effect.distance_culling_mode),
        (4, 3)
    );
    assert!(effect.emit_when_distance_culled.get());
    assert!(!effect.update_when_distance_culled.get());
    assert_eq!(effect.lod_evo_dist_max, 30.0);
    assert_eq!(effect.data_volume_type, 2);
    let evolutions = effect.evolution_list.as_deref().expect("an evolution list");
    assert_eq!(evolutions.evolutions[0].name, "speed");
    assert!(evolutions.evolutions[0].is_overriden.get());
    let event = effect.timeline.events[0].as_deref().expect("an event");
    assert_eq!(event.emitter_rule_name, "emit_a");
    assert_eq!(event.particle_rule_name, "part_a");
    assert_eq!(event.emitter_rule, OwnedOrKeyed::Key(hash("emit_a")));
    assert_eq!(event.particle_rule, OwnedOrKeyed::Key(hash("part_a")));
    assert_eq!((event.base.start_ratio, event.base.end_ratio), (0.25, 0.75));
    assert_eq!(event.colour_tint_min, 0xFF00_FF00);

    let emitter = first(&ypt.emitter_rule_dict);
    let creation = emitter
        .creation_domain_obj
        .domain
        .as_deref()
        .expect("a creation domain");
    assert_eq!((creation.r#type, creation.shape), (0, 4));
    assert!(creation.is_target_relative.get());
    let target = emitter
        .target_domain_obj
        .domain
        .as_deref()
        .expect("a target domain");
    assert_eq!((target.r#type, target.shape), (1, 1));
    assert_eq!(target.position_kfp.property_id.get(), DOMAIN_KFPS[1][0]);
    assert!(emitter.attractor_domain_obj.domain.is_none());
    let rate = &emitter.spawn_rate_over_time_kfp;
    assert_eq!(rate.property_id.get(), EMITTER_KFPS[0]);
    assert_eq!(rate.keyframe.entries[0].value.y, 12.0);
    assert!(emitter.inherit_velocity_kfp.keyframe.entries.is_empty());
}

#[test]
fn a_particle_rule_keeps_its_named_behaviours_and_labels_the_first_by_place_and_size() {
    let mut ypt = as_ypt(&list());
    or_particle::list::canonical(&mut ypt);
    let rule = first(&ypt.particle_rule_dict);
    assert_eq!((rule.tex_frame_id_min, rule.tex_frame_id_max), (2, 5));
    assert_eq!((rule.sort_type, rule.draw_type), (1, 0));
    assert_eq!(
        rule.effect_spawner_at_ratio.effect_rule,
        OwnedOrKeyed::Key(hash("fx_a"))
    );
    assert_eq!(
        rule.effect_spawner_at_ratio
            .spawned_effect_scalars_max
            .zoom_scalar,
        110.0
    );
    assert!(rule.effect_spawner_at_ratio.tracks_point_dir.get());
    let behaviours: Vec<&PtxBehaviourAny> = (rule.all_behaviours.iter())
        .filter_map(|b| match b {
            Ref::Own(b) => Some(&**b),
            Ref::To(_) => None,
        })
        .collect();
    assert_eq!(
        behaviours.len(),
        3,
        "the behaviour of no known class is left out"
    );
    assert!(matches!(behaviours[0], PtxBehaviourAny::PtxuAge(_)));
    let PtxBehaviourAny::PtxuSize(size) = behaviours[1] else {
        panic!("a ptxu_Size");
    };
    let PtxBehaviourAny::PtxuAttractor(pull) = behaviours[2] else {
        panic!("a ptxu_Attractor");
    };
    assert_eq!(size.base.hash_name, BEHAVIOUR_HASH_SIZE);
    assert_eq!(size.whd_max_kfp.keyframe.entries[0].value.y, 2.0);
    assert!(size.whd_min_kfp.keyframe.entries.is_empty());
    assert_eq!(pull.strength_kfp.property_id.get(), KFP_ATTRACTOR[0]);
    assert_eq!(pull.strength_kfp.keyframe.entries[0].value.x, 7.0);
}

#[test]
fn the_lines_name_what_the_gta_v_list_has_no_place_for() {
    let lines = lines(&list());
    let has = |part: &str| lines.iter().any(|l| l.contains(part));
    assert!(has(
        "Behaviour 2: 80 bytes, of class bit 26, a class GTA V does not have"
    ));
    assert!(has("Behaviour 0: 16 bytes and first in its rule"));
    assert!(has("Behaviour 1: ptxu_Size, 280 bytes"));
    assert!(has("Texture slot 4: key 0A0B0C0D0E0F1011."));
    assert!(has(
        "third spawner starts fx_a where its ptxu_ZCull culls a point"
    ));
    assert!(has("mesh of the model \"mesh_a\""));
    assert!(has("Emitter rule emit_a: flags 0x00000001."));
}

#[test]
fn a_rules_third_spawner_is_kept_naming_its_effect_and_a_zcull_keeps_its_mode() {
    let mut list = list();
    let kept = spawners::of_list(&list);
    let third = kept.get("part_a").expect("its third spawner");
    assert_eq!((third.name.as_str(), third.trigger_info), ("fx_a", 0.25));
    list.particles[0].spawners[2].effect = None;
    assert!(spawners::of_list(&list).is_empty());

    let mut zcull = behaviour(Some("ptxu_ZCull"), 0xA0, vec![]);
    zcull.fields = BehaviourFields::ZCull {
        mode: 4,
        reference_space: 5,
    };
    let Some(PtxBehaviourAny::PtxuZCull(z)) = super::behaviour(&zcull, 2) else {
        panic!("a ptxu_ZCull");
    };
    assert_eq!((z.mode, z.reference_space), (4, 5));
    list.behaviours.push(zcull);
    list.particles[0].behaviours.push(4);
    let said = lines(&list);
    assert!(said
        .iter()
        .any(|l| l.contains("mode 4 is not one GTA V has")));
}

#[test]
fn an_unedited_gta_v_form_gives_the_gta_vi_list_back() {
    let stored = list();
    let mut ypt = as_ypt(&stored);
    or_particle::list::canonical(&mut ypt);
    let back = crate::state::gta6_particles_save::from_ypt(&stored, &ypt);
    assert_eq!(back, Ok(stored));
}

#[test]
fn edits_of_the_shared_values_reach_the_gta_vi_list_and_any_other_is_refused() {
    use crate::state::gta6_particles_save::from_ypt;
    use or_particle::list::{effect_rules_mut, emitter_rules_mut, events_mut, particle_rules_mut};
    let stored = list();
    let mut ypt = as_ypt(&stored);
    or_particle::list::canonical(&mut ypt);
    let effect = effect_rules_mut(&mut ypt).next().expect("an effect");
    effect.duration_max = 9.0;
    effect.sort_events_by_distance = Bool8::new(false);
    events_mut(effect).next().expect("an event").colour_tint_max = 0x1020_3040;
    let emitter = emitter_rules_mut(&mut ypt).next().expect("an emitter");
    emitter.spawn_rate_over_time_kfp.keyframe.entries[0].value.y = 15.0;
    let rule = particle_rules_mut(&mut ypt)
        .next()
        .expect("a particle rule");
    rule.tex_frame_id_max = 6;
    for b in &mut rule.all_behaviours {
        let Ref::Own(b) = b else {
            continue;
        };
        let keys = match &mut **b {
            PtxBehaviourAny::PtxuSize(s) => &mut s.whd_max_kfp.keyframe.entries,
            PtxBehaviourAny::PtxuAttractor(a) => &mut a.strength_kfp.keyframe.entries,
            _ => continue,
        };
        keys[0].value.x += 8.0;
    }
    let edited = from_ypt(&stored, &ypt).expect("every edit has a place");
    let effect = &edited.effects[0];
    assert_eq!(effect.times[3], 9.0);
    assert_eq!(effect.flags, [0, 2, 0, 1], "only the flag edited changes");
    assert_eq!(effect.events[0].colour_tint, [0xFF00_FF00, 0x1020_3040]);
    let rate = &edited.emitters[0].kfps[0].entries[0];
    assert_eq!(rate.value, [10.0, 15.0, 0.0, 0.0]);
    assert_eq!(edited.particles[0].tex_frame_id, [2, 6]);
    let size = &edited.behaviours[1].kfps[0].entries[0];
    assert_eq!(size.value, [9.0, 2.0, 3.0, 0.0]);
    let strength = &edited.behaviours[3].kfps[0].entries[0];
    assert_eq!(strength.value, [15.0, 0.0, 0.0, 0.0]);
    assert_eq!(edited.emitters[0].domains, stored.emitters[0].domains);

    // A value the GTA VI list has no place for, and a rule renamed.
    let mut zoomed = ypt.clone();
    effect_rules_mut(&mut zoomed)
        .next()
        .expect("an effect")
        .zoom_level = 50.0;
    assert!(from_ypt(&stored, &zoomed).is_err());
    let mut renamed = ypt.clone();
    particle_rules_mut(&mut renamed)
        .next()
        .expect("a rule")
        .name = "part_b".into();
    assert!(from_ypt(&stored, &renamed).is_err());
}

#[test]
fn a_list_names_the_model_its_creation_mesh_domain_emits_from() {
    let named = crate::state::gta6_particle_models::named(&list());
    let mesh = or_io::gta6::hash::name_hash32(b"mesh_a");
    assert_eq!(named.domains.get("emit_a"), Some(&[Some(mesh), None, None]));
    assert_eq!(named.domains.len(), 1, "the sphere target names no model");
    assert!(named.wanted.contains(&mesh));
}

/// The behaviours the first particle rule of `ypt` owns.
fn owned(ypt: &mut PtxFxList) -> Vec<&mut PtxBehaviourAny> {
    let rule = or_particle::list::particle_rules_mut(ypt)
        .next()
        .expect("a particle rule");
    (rule.all_behaviours.iter_mut())
        .filter_map(|b| match b {
            Ref::Own(b) => Some(&mut **b),
            Ref::To(_) => None,
        })
        .collect()
}

#[test]
fn an_edited_flip_chance_and_zcull_mode_reach_the_gta_vi_list_and_another_member_is_refused() {
    use crate::state::gta6_particles_save::from_ypt;
    let mut stored = list();
    let mut zcull = behaviour(Some("ptxu_ZCull"), 0xA0, vec![]);
    zcull.fields = BehaviourFields::ZCull {
        mode: 1,
        reference_space: 5,
    };
    let mut sprite = behaviour(None, 0x50, vec![]);
    let drawn = |flip_chance| BehaviourFields::Sprite {
        flip_chance,
        alignment: 2,
        bytes: [0; 16],
    };
    sprite.fields = drawn([0.5, 0.25]);
    stored.behaviours.extend([zcull, sprite]);
    stored.particles[0].behaviours.extend([4, 5]);
    let mut ypt = as_ypt(&stored);
    or_particle::list::canonical(&mut ypt);
    for b in owned(&mut ypt) {
        match b {
            PtxBehaviourAny::PtxdSprite(s) => s.flip_chance_v = 0.75,
            PtxBehaviourAny::PtxuZCull(z) => z.mode = 3,
            _ => {}
        }
    }
    let edited = from_ypt(&stored, &ypt).expect("both have a place");
    assert_eq!(edited.behaviours[5].fields, drawn([0.5, 0.75]));
    let culled = BehaviourFields::ZCull {
        mode: 3,
        reference_space: 5,
    };
    assert_eq!(edited.behaviours[4].fields, culled);

    // A member the GTA VI list does not write.
    let mut clipped = ypt.clone();
    for b in owned(&mut clipped) {
        if let PtxBehaviourAny::PtxdSprite(s) = b {
            s.near_clip_dist = 2.0;
        }
    }
    assert!(from_ypt(&stored, &clipped).is_err());
}

#[path = "gta6_particles_open_tests.rs"]
mod open;

#[path = "gta6_particles_remove_tests.rs"]
mod remove;

#[path = "gta6_particle_colour_tests.rs"]
mod colour;
