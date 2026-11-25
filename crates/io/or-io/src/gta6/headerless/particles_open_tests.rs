//! The parts of a particle list read beyond its records' fields: evolved keyframes, shader words,
//! texture slots, frames and the behaviours labelled by id, place and size.

use super::*;

/// The section offset the pointer word at `at` addresses.
fn target(v: &[u8], at: usize) -> usize {
    let word = u64::from_le_bytes(std::array::from_fn(|i| v[at + i]));
    (word & 0x0FFF_FFFF) as usize
}

type Keys<'a> = &'a [([f32; 4], [f32; 4])];

const LATE: [([f32; 4], [f32; 4]); 2] = [
    ([0.0; 4], [1.0, 2.0, 3.0, 4.0]),
    ([1.0, 1.0, 0.0, 0.0], [5.0, 6.0, 7.0, 8.0]),
];

fn entries(keys: Keys) -> Vec<KfpEntry> {
    (keys.iter())
        .map(|&(time, value)| KfpEntry { time, value })
        .collect()
}

/// The body with its effect's evolutions named `names` and its one evolved value's keyframes,
/// each (evolution index, kind, flags, keys); the value's place and each keyframe's.
fn evolved(names: &[&str], keyframes: &[(u32, u8, [u8; 2], Keys)]) -> (Vec<u8>, usize, Vec<usize>) {
    let (v, _) = body();
    let effect = read_list(&v).expect("it reads").effects[0].at as usize;
    let mut b = Body { v };
    let list = target(&b.v, effect + 0x48);
    let evolutions = b.alloc(0x18 * names.len());
    for (i, name) in names.iter().enumerate() {
        let text = b.text(name);
        b.ptr(evolutions + 0x18 * i, text);
        b.u32(evolutions + 0x18 * i + 8, hash(name));
    }
    let n = names.len() as u16;
    b.array(list, evolutions, n, n);
    let prop = target(&b.v, list + 0x10);
    let records = b.alloc(0x30 * keyframes.len());
    let mut places = Vec::new();
    for (k, (index, kind, flags, keys)) in keyframes.iter().enumerate() {
        let e = records + 0x30 * k;
        // Its keys' array sits where a keyframed value's does at +0x08; the ids written go below.
        b.kfp(e - 8, 0, 0, keys);
        b.u32(e + 0x20, *index);
        b.put(e + 0x24, &[*kind, flags[0], flags[1]]);
        places.push(e);
    }
    let count = keyframes.len() as u16;
    b.array(prop, records, count, count);
    (b.v, prop, places)
}

#[test]
fn an_evolved_value_reads_its_keyframe_for_each_evolution() {
    let (v, _, _) = evolved(
        &["camTopView", "LOD"],
        &[(0, 3, [1, 0], &LATE[..]), (1, 1, [0, 1], &LATE[..1])],
    );
    let list = read_list(&v).expect("it reads");
    let evolved = list.effects[0].evolution_list.as_ref().expect("a list");
    assert_eq!(
        evolved.props[0].keyframes,
        [
            EvolvedKeyframe {
                entries: entries(&LATE),
                evolution: 0,
                kind: 3,
                flags: [1, 0],
            },
            EvolvedKeyframe {
                entries: entries(&LATE[..1]),
                evolution: 1,
                kind: 1,
                flags: [0, 1],
            },
        ]
    );
}

#[test]
fn an_evolved_keyframes_kind_is_its_evolutions_camera_view_by_name_in_any_case() {
    let (v, _, _) = evolved(
        &["camForwardView", "camtopview", "CamSideView"],
        &[
            (0, 2, [0, 0], &LATE[..]),
            (1, 3, [0, 0], &LATE[..]),
            (2, 4, [0, 0], &LATE[..]),
        ],
    );
    let list = read_list(&v).expect("it reads");
    let evolved = list.effects[0].evolution_list.as_ref().expect("a list");
    let kinds: Vec<u8> = evolved.props[0].keyframes.iter().map(|k| k.kind).collect();
    assert_eq!(kinds, [2, 3, 4]);
}

#[test]
fn an_evolved_keyframe_is_refused_where_a_measured_rule_breaks() {
    let (v, prop, places) = evolved(
        &["strength", "lod"],
        &[(0, 0, [0, 0], &LATE[..]), (1, 1, [0, 0], &LATE[..])],
    );
    assert!(read_list(&v).is_ok());
    let refused = |change: &dyn Fn(&mut Vec<u8>)| {
        let mut w = v.clone();
        change(&mut w);
        is_refused(read_list(&w))
    };
    let (first, second) = (places[0], places[1]);
    // An index past the evolutions, and one not above the one before.
    assert!(refused(&|w| w[second + 0x20] = 2));
    assert!(refused(&|w| w[second + 0x20] = 0));
    // A kind above 4, a lod or camera kind on another evolution, a lod one of another kind.
    assert!(refused(&|w| w[second + 0x24] = 5));
    assert!(refused(&|w| w[first + 0x24] = 1));
    assert!(refused(&|w| w[first + 0x24] = 2));
    assert!(refused(&|w| w[second + 0x24] = 0));
    assert!(refused(&|w| w[second + 0x24] = 4));
    // A flag of 2, a pad byte, a runtime word.
    assert!(refused(&|w| w[second + 0x25] = 2));
    assert!(refused(&|w| w[second + 0x2F] = 1));
    assert!(refused(&|w| w[second + 0x18] = 1));
    // A key's time past 1.
    let keys = target(&v, second);
    let (past, time) = (2.0f32.to_le_bytes(), keys + 0x20);
    assert!(refused(&|w| w[time..time + 4].copy_from_slice(&past)));
    // A blend mode above 3, and the value's pad.
    assert!(refused(&|w| w[prop + 0x14] = 4));
    assert!(refused(&|w| w[prop + 0x15] = 1));
    let mut nan = v.clone();
    nan[keys + 0x10..keys + 0x14].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(
        read_list(&nan).is_ok(),
        "a value lane that is not finite is read"
    );
}

#[test]
fn an_events_evolved_value_is_one_of_the_lists_keyframed_values() {
    let read = |id: u32| {
        let (v, at) = body();
        let mut b = Body { v };
        let list = b.alloc(0x38);
        let prop = b.alloc(0x18);
        b.u32(prop + 0x10, id);
        b.v[prop + 0x14] = 1;
        b.array(list + 0x10, prop, 1, 1);
        b.atmap(list + 0x20, &[(id, Some(prop))]);
        b.ptr(at.event + 0x30, list);
        read_list(&b.v)
    };
    let held = read(EMITTER_KFPS[2]).expect("an emitter rule's value");
    let event = &held.effects[0].events[0];
    let ids: Vec<u32> = (event.evolution_list.iter())
        .flat_map(|l| l.props.iter().map(|p| p.id))
        .collect();
    assert_eq!(ids, [EMITTER_KFPS[2]]);
    assert!(read(DOMAIN_KFPS[1][0]).is_ok(), "a domain's value");
    assert!(is_refused(read(0x0BAD_F00D)));
}

fn word(variable: &str, value: &str) -> (u32, u32) {
    let value = match value.is_empty() {
        true => 0,
        false => hash(value),
    };
    (hash(variable), value)
}

/// The body with its rule holding `words`.
fn worded(words: &[(u32, u32)]) -> Result<ParticleEffects> {
    let (v, at) = body();
    let mut b = Body { v };
    let to = b.alloc(8 * words.len());
    for (w, (variable, value)) in words.iter().enumerate() {
        b.u32(to + 8 * w, *variable);
        b.u32(to + 8 * w + 4, *value);
    }
    let n = words.len() as u16;
    b.array(at.shader_words, to, n, n);
    read_list(&b.v)
}

#[test]
fn a_rules_shader_words_read_as_named_modes_and_switches() {
    let base = [
        word("lightingmode", "normalspec"),
        word("projectionmode", "water"),
        word("diffusemode", "rgb"),
    ];
    let mut words = base.to_vec();
    words.extend([word("soft", ""), (0xC391_FD51, 0), word("soft", "")]);
    let list = worded(&words).expect("it reads");
    let shading = list.particles[0].shading().expect("named");
    assert_eq!(
        shading,
        Shading {
            lighting: LightingMode::NormalSpec,
            projection: ProjectionMode::Water,
            diffuse: DiffuseMode::Rgb,
            switches: vec![
                ShaderSwitch::Soft,
                ShaderSwitch::HashC391FD51,
                ShaderSwitch::Soft,
            ],
        }
    );
    assert_eq!(shading.lighting.name(), "normalspec");
    assert_eq!(ShaderSwitch::HashC391FD51.name(), "hash_C391FD51");
    let unnamed = worded(&[(hash("lightingmode"), 0x6F22_62CB), base[1], base[2]]);
    let lighting = unnamed.ok().and_then(|l| l.particles[0].shading().ok());
    assert_eq!(
        lighting.map(|s| s.lighting),
        Some(LightingMode::Hash6F2262CB)
    );

    let refused = |words: &[(u32, u32)]| is_refused(worded(words));
    assert!(refused(&[base[0], base[1]]), "diffusemode missing");
    assert!(
        refused(&[base[0], base[1], base[2], base[2]]),
        "an enumeration twice"
    );
    let other = (hash("lightingmode"), hash("rgba"));
    assert!(
        refused(&[other, base[1], base[2]]),
        "a value of another enumeration"
    );
    let valued = (hash("refract"), 1);
    assert!(
        refused(&[base[0], base[1], base[2], valued]),
        "a switch holding a value"
    );
    assert!(
        refused(&[base[0], base[1], base[2], (0x7300_0001, 0)]),
        "a variable not measured"
    );
    assert!(
        refused(&[base[0], base[1], base[2], word("trail", "")]),
        "a trail switch on a sprite rule"
    );
}

/// The body with its rule's texture keys `keys` by slot and the rest 0.
fn keyed(keys: &[(usize, u64)]) -> Result<ParticleEffects> {
    let (mut v, at) = body();
    v[at.particle + 0x2F0..at.particle + 0x358].fill(0);
    for &(slot, key) in keys {
        let k = at.particle + 0x2F0 + 8 * slot;
        v[k..k + 8].copy_from_slice(&key.to_le_bytes());
    }
    read_list(&v)
}

#[test]
fn a_sprite_is_the_first_set_of_slots_0_6_5_and_12_and_paired_slots_are_set_together() {
    let slot = |keys: &[(usize, u64)]| keyed(keys).expect("it reads").particles[0].sprite_slot();
    assert_eq!(slot(&[(12, 1), (5, 2), (0, 3)]), Some(0));
    assert_eq!(slot(&[(12, 1), (5, 2), (6, 4), (7, 5)]), Some(6));
    assert_eq!(slot(&[(12, 1), (5, 2), (1, 6)]), Some(5));
    assert_eq!(slot(&[(12, 1), (8, 7)]), Some(12));
    assert_eq!(slot(&[(8, 7), (2, 8), (3, 9)]), None);
    for alone in [2, 3, 6, 7] {
        assert!(is_refused(keyed(&[(0, 3), (alone, 8)])), "slot {alone}");
    }
}

#[test]
fn a_rules_frame_range_is_refused_backwards_and_its_count_is_its_last_frame_and_one() {
    let (v, at) = body();
    let list = read_list(&v).expect("it reads");
    assert_eq!(list.frame_count(&list.particles[0]), 8);
    let mut w = v.clone();
    w[at.particle + 0x170] = 9;
    assert!(is_refused(read_list(&w)), "a first frame above the last");
    w[at.particle + 0x170..at.particle + 0x174].copy_from_slice(&(-1i32).to_le_bytes());
    assert!(is_refused(read_list(&w)), "a first frame below 0");
}

/// The body with behaviours of the given sizes and class bits, each above Size's, listed after
/// its rule's own, and their places.
fn listing(sized: &[(usize, u8)]) -> (Body, Vec<usize>) {
    let (v, at) = body();
    let list = read_list(&v).expect("it reads");
    let held: Vec<usize> = (list.particles[0].behaviours.iter())
        .map(|&b| list.behaviours[b].at as usize)
        .collect();
    let mut b = Body { v };
    let listed = b.alloc(8 * (held.len() + sized.len()));
    let added: Vec<usize> = sized.iter().map(|&(s, _)| b.alloc(s)).collect();
    for (i, place) in held.iter().chain(&added).enumerate() {
        b.ptr(listed + 8 * i, *place);
    }
    let n = (held.len() + added.len()) as u16;
    b.array(at.particle + 0x188, listed, n, n);
    let bits = (sized.iter()).fold(AGE_AND_SIZE, |word, &(_, bit)| word | (1 << bit));
    class_words(&mut b.v, at.particle, bits);
    (b, added)
}

fn behaviour_at(list: &ParticleEffects, at: usize) -> &Behaviour {
    (list.behaviours.iter())
        .find(|b| b.at as usize == at)
        .expect("listed")
}

#[test]
fn a_ptxu_animate_texture_states_the_frame_count_and_a_last_frame_below_minus_1_is_refused() {
    let (mut b, added) = listing(&[(0x78, 17)]);
    let anim = added[0];
    let rate = hash("ptxu_AnimateTexture:m_animRateKFP");
    b.kfp(anim + 8, rate, rate, &[]);
    b.u32(anim + 0x48, 1);
    b.u32(anim + 0x4C, 31);
    b.u32(anim + 0x50, 2);
    b.put(anim + 0x54, &[0, 2, 1, 0]);
    let list = read_list(&b.v).expect("it reads");
    let held = behaviour_at(&list, anim);
    assert_eq!(held.class, Some("ptxu_AnimateTexture"));
    let fields = BehaviourFields::AnimateTexture {
        keyframe_mode: 1,
        last_frame_id: 31,
        loop_mode: 2,
        flags: [0, 2, 1, 0],
    };
    assert_eq!(held.fields, fields);
    assert_eq!(list.frame_count(&list.particles[0]), 32);
    b.u32(anim + 0x4C, -1i32 as u32);
    let list = read_list(&b.v).expect("a last frame of -1 reads");
    assert_eq!(list.frame_count(&list.particles[0]), 1);
    b.u32(anim + 0x4C, -2i32 as u32);
    assert!(is_refused(read_list(&b.v)));
}

#[test]
fn a_ptxu_zcull_states_its_mode_and_space_and_lets_the_third_spawner_start_an_effect() {
    let (mut b, added) = listing(&[(0xA0, 12)]);
    let zcull = added[0];
    let height = hash("ptxu_ZCull:m_heightKFP");
    b.kfp(zcull + 8, height, height, &[]);
    b.u32(zcull + 0x94, 1);
    b.u32(zcull + 0x98, 5);
    let list = read_list(&b.v).expect("it reads");
    let rule = list.particles[0].at as usize;
    let effect = list.effects[0].at;
    let held = behaviour_at(&list, zcull);
    assert_eq!((held.class, held.class_bit), (Some("ptxu_ZCull"), Some(12)));
    let fields = BehaviourFields::ZCull {
        mode: 1,
        reference_space: 5,
    };
    assert_eq!(held.fields, fields);
    b.ptr(rule + 0x148, effect as usize);
    let list = read_list(&b.v).expect("a third spawner beside a ptxu_ZCull reads");
    assert_eq!(list.particles[0].spawners[2].effect, Some(0));
}

#[test]
fn an_edited_flip_chance_and_zcull_mode_are_written_in_place_and_read_back() {
    let (mut b, added) = listing(&[(0xA0, 12), (0x50, 27)]);
    let [zcull, sprite] = added[..] else {
        panic!("two behaviours added");
    };
    let height = hash("ptxu_ZCull:m_heightKFP");
    b.kfp(zcull + 8, height, height, &[]);
    b.u32(zcull + 0x94, 1);
    b.f32(sprite + 8, 0.5);
    b.f32(sprite + 0xC, 0.25);
    let mut list = read_list(&b.v).expect("it reads");
    for held in &mut list.behaviours {
        match &mut held.fields {
            BehaviourFields::Sprite { flip_chance, .. } => *flip_chance = [0.125, 1.0],
            BehaviourFields::ZCull { mode, .. } => *mode = 4,
            _ => {}
        }
    }
    let mut v = b.v.clone();
    write_particle_effects(&mut v, &list).expect("it writes");
    let back = read_list(&v).expect("it reads back");
    let drawn = BehaviourFields::Sprite {
        flip_chance: [0.125, 1.0],
        alignment: 0,
        bytes: [0; 16],
    };
    assert_eq!(behaviour_at(&back, sprite).fields, drawn);
    let culled = BehaviourFields::ZCull {
        mode: 4,
        reference_space: 0,
    };
    assert_eq!(behaviour_at(&back, zcull).fields, culled);

    // A mode no file holds is refused, writing nothing.
    let mut wrong = back.clone();
    for held in &mut wrong.behaviours {
        if let BehaviourFields::ZCull { mode, .. } = &mut held.fields {
            *mode = 9;
        }
    }
    let before = v.clone();
    assert!(write_particle_effects(&mut v, &wrong).is_err());
    assert_eq!(v, before);
}

#[test]
fn behaviours_no_value_classes_are_labelled_by_id_and_class_bit() {
    let (mut b, added) = listing(&[(0x60, 11), (0x10, 20), (0x50, 27)]);
    let [wind, fire, sprite] = added[..] else {
        panic!("three behaviours added");
    };
    b.kfp(wind + 8, 0x89DC_E430, 0x89DC_E430, &[]);
    b.f32(sprite + 8, 0.5);
    b.f32(sprite + 0xC, 0.25);
    b.u32(sprite + 0x1C, 10);
    b.put(sprite + 0x38, &[1, 0, 1, 0, 0, 0, 0, 0xFF]);
    let list = read_list(&b.v).expect("it reads");
    let first = list.behaviours[list.particles[0].behaviours[0]].at as usize;
    let fields = |at: usize| behaviour_at(&list, at).fields;
    assert_eq!(fields(first), BehaviourFields::Age, "bit 1");
    assert_eq!(fields(wind), BehaviourFields::Unclassed(0x89DC_E430));
    assert_eq!(
        fields(fire),
        BehaviourFields::None,
        "0x10 bytes at Fire's bit, not Age's"
    );
    assert_eq!(behaviour_at(&list, fire).class_bit, Some(20));
    let mut bytes = [0; 16];
    bytes[..8].copy_from_slice(&[1, 0, 1, 0, 0, 0, 0, 0xFF]);
    let drawn = BehaviourFields::Sprite {
        flip_chance: [0.5, 0.25],
        alignment: 10,
        bytes,
    };
    assert_eq!(fields(sprite), drawn);
    // An alignment above 11, or a word at +0x30 or +0x48 set, leaves it unlabelled.
    for (at, word) in [(0x1C, 0x3F80_0000u32), (0x30, 1), (0x48, 1)] {
        let mut w = b.v.clone();
        w[sprite + at..sprite + at + 4].copy_from_slice(&word.to_le_bytes());
        let list = read_list(&w).expect("it reads");
        assert_eq!(
            behaviour_at(&list, sprite).fields,
            BehaviourFields::None,
            "+{at:#X}"
        );
    }
}

#[test]
fn a_colours_first_unnamed_value_reads_with_a_word_at_0x30_and_another_base_id() {
    let (mut b, added) = listing(&[(0x1E0, 19)]);
    let colour = added[0];
    let emissive = hash("ptxu_Colour:m_emissiveIntensityKFP");
    b.kfp(colour + 8, emissive, emissive, &[]);
    let rgba = [0.5, 0.25, 1.0, 1.0];
    let keys = [([0.0; 4], rgba)];
    b.kfp(colour + 0x48, COLOUR_RGBA_KFPS[0], 0x1357_9BDF, &keys);
    b.u32(colour + 0x48 + 0x30, 1);
    b.kfp(colour + 0x88, COLOUR_RGBA_KFPS[1], COLOUR_RGBA_KFPS[1], &[]);
    let list = read_list(&b.v).expect("it reads");
    let held = behaviour_at(&list, colour);
    assert_eq!(held.class, Some("ptxu_Colour"));
    let min = (held.kfps.iter())
        .find(|k| k.id == COLOUR_RGBA_KFPS[0])
        .expect("its minimum");
    assert_eq!((min.base_id, min.entries[0].value), (0x1357_9BDF, rgba));
    assert!(held.kfps.iter().any(|k| k.id == COLOUR_RGBA_KFPS[1]));
}
