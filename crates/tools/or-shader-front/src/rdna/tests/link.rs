//! The two stages of a pass joined: what the pixel stage reads crosses in as few locations as hold it.

use naga::{Binding, BuiltIn, Expression as E, Interpolation, Statement, TypeInner};

use super::super::link::{link, pack, Carried, Options as LinkOptions};
use super::*;

/// The 16-bit interpolation of attribute `attr`'s component `chan`, its high half or its low, into `dst`.
fn interp_half(dst: u32, attr: u32, chan: u32, high: bool) -> [u32; 2] {
    let place = attr | chan << 6 | u32::from(high) << 8;
    vop3(op3("v_interp_p2_f16"), dst, [place, V + 1, V], 0, 0)
}

/// A pixel program of `attributes` inputs, attribute `n` being parameter slot `n + 1`.
fn pixel(attributes: u8, words: Vec<u32>) -> naga::Module {
    let mut program = program(ShaderKind::Pixel, words);
    program.inputs = (1..=attributes)
        .map(|slot| PixelInput { slot, flags: 2 })
        .collect();
    let module = translate(&program).unwrap().module;
    validate(&module);
    module
}

/// A vertex program that gives the position and each parameter of `parameters`, the ones of `halves` as halves.
fn vertex(parameters: &[u32], halves: &[u8]) -> naga::Module {
    let mut words = vec![vop1(V_MOV, 0, F_ONE)];
    words.extend(exp(POS0, [0, 0, 0, 0]));
    for parameter in parameters {
        words.extend(exp(32 + parameter, [0, 0, 0, 0]));
    }
    words.push(S_ENDPGM);
    let options = Options {
        half_slots: halves.iter().copied().collect(),
        ..Options::default()
    };
    let module = translate_with(&program(ShaderKind::Vertex, words), &options)
        .unwrap()
        .module;
    validate(&module);
    module
}

/// The locations `module`'s entry point takes or gives, with the interpolation of each.
fn locations(module: &naga::Module) -> Vec<(u32, Option<Interpolation>)> {
    let entry = &module.entry_points[0];
    let of = |binding: &Option<Binding>| match binding {
        Some(Binding::Location {
            location,
            interpolation,
            ..
        }) => Some((*location, *interpolation)),
        _ => None,
    };
    if entry.stage == naga::ShaderStage::Fragment {
        let arguments = entry.function.arguments.iter();
        return arguments.filter_map(|a| of(&a.binding)).collect();
    }
    let result = entry.function.result.as_ref().unwrap();
    let TypeInner::Struct { members, .. } = &module.types[result.ty].inner else {
        panic!("the vertex stage returns no structure");
    };
    let mut given: Vec<_> = members.iter().filter_map(|m| of(&m.binding)).collect();
    given.sort_by_key(|(location, _)| *location);
    given
}

/// The built-in values `module`'s vertex stage gives.
fn built_ins(module: &naga::Module) -> Vec<BuiltIn> {
    let result = module.entry_points[0].function.result.as_ref().unwrap();
    let TypeInner::Struct { members, .. } = &module.types[result.ty].inner else {
        panic!("the vertex stage returns no structure");
    };
    (members.iter())
        .filter_map(|member| match &member.binding {
            Some(Binding::BuiltIn(built_in)) => Some(*built_in),
            _ => None,
        })
        .collect()
}

#[test]
fn the_halves_a_pixel_stage_reads_of_a_parameter_cross_in_one_location() {
    // Slot 1's x as its high half and its y as its low half: two locations as lowered, 1 and 33.
    let (high, low) = (interp_half(4, 0, 0, true), interp_half(5, 0, 1, false));
    let colour = exp(0, [4, 5, 4, 5]);
    let mut words = Vec::new();
    words.extend(high);
    words.extend(low);
    words.extend(colour);
    words.push(S_ENDPGM);
    let mut fragment = pixel(3, words);
    assert_eq!(locations(&fragment).len(), 2);
    let mut geometry = vertex(&[1], &[1]);
    assert_eq!(locations(&geometry).len(), 2);

    let linked = link(&mut geometry, &mut fragment, &LinkOptions::default()).unwrap();
    validate(&geometry);
    validate(&fragment);
    let one = [(0, Some(Interpolation::Perspective))];
    assert_eq!(locations(&fragment), one);
    assert_eq!(locations(&geometry), one);
    assert_eq!(linked.locations, 1);
    let carried = |component, high, at| Carried {
        slot: 1,
        high,
        component,
        flat: false,
        location: 0,
        at,
        fed: true,
    };
    assert_eq!(linked.carried, [carried(1, false, 0), carried(0, true, 1)]);
}

#[test]
fn twenty_parameters_read_one_component_each_cross_in_five_locations() {
    let mut words = Vec::new();
    for attr in 0..20 {
        words.push(vintrp(1, 4, attr, 0, 1));
    }
    words.extend(exp(0, [4, 4, 4, 4]));
    words.push(S_ENDPGM);
    let mut fragment = pixel(20, words);
    assert_eq!(locations(&fragment).len(), 20);
    let parameters: Vec<u32> = (1..=20).collect();
    let mut geometry = vertex(&parameters, &[]);

    let linked = link(&mut geometry, &mut fragment, &LinkOptions::default()).unwrap();
    validate(&geometry);
    validate(&fragment);
    assert_eq!(locations(&fragment).len(), 5);
    assert_eq!(locations(&geometry), locations(&fragment));
    assert!(linked.carried.iter().all(|carried| carried.fed));
    // Alone, the pixel stage packs the same way.
    let mut alone = pixel(20, {
        let mut words: Vec<u32> = (0..20).map(|attr| vintrp(1, 4, attr, 0, 1)).collect();
        words.extend(exp(0, [4, 4, 4, 4]));
        words.push(S_ENDPGM);
        words
    });
    assert_eq!(pack(&mut alone).unwrap().locations, 5);
    validate(&alone);
    assert_eq!(locations(&alone), locations(&fragment));
}

#[test]
fn a_parameter_is_joined_to_the_slot_the_pass_reads_it_at_and_what_nothing_reads_is_dropped() {
    // The pixel stage reads slot 2; the vertex stage gives parameters 0, 1 and 2.
    let words = vec![vintrp(1, 4, 1, 0, 1)];
    let fragment = |words: &[u32]| {
        let mut words = words.to_vec();
        words.extend(exp(0, [4, 4, 4, 4]));
        words.push(S_ENDPGM);
        pixel(3, words)
    };
    let (mut by_number, mut by_slot) = (fragment(&words), fragment(&words));
    let (mut plain, mut shifted) = (vertex(&[0, 1, 2], &[]), vertex(&[0, 1], &[]));

    // Parameter 2 feeds slot 2 where nothing says otherwise.
    let linked = link(&mut plain, &mut by_number, &LinkOptions::default()).unwrap();
    assert!(linked.carried[0].fed);
    assert_eq!(locations(&plain).len(), 1);
    validate(&plain);

    // Parameter `n` read at slot `n + 1`: parameter 1 feeds it, and without parameter 2.
    let options = LinkOptions {
        slots: vec![1, 2],
        ..Default::default()
    };
    let linked = link(&mut shifted, &mut by_slot, &options).unwrap();
    assert!(linked.carried[0].fed);
    validate(&shifted);

    // A slot nothing writes is carried as nought, and says so.
    let (mut starved, mut reader) = (vertex(&[0], &[]), fragment(&words));
    let linked = link(&mut starved, &mut reader, &LinkOptions::default()).unwrap();
    assert!(!linked.carried[0].fed);
    assert_eq!(locations(&starved), locations(&reader));
    validate(&starved);
}

#[test]
fn a_flat_input_keeps_a_location_of_its_own() {
    let mut words = vec![vintrp(1, 4, 0, 0, 1), vintrp(2, 5, 1, 0, 1)];
    words.extend(exp(0, [4, 5, 4, 5]));
    words.push(S_ENDPGM);
    let mut fragment = pixel(3, words);
    let mut geometry = vertex(&[1, 2], &[]);
    link(&mut geometry, &mut fragment, &LinkOptions::default()).unwrap();
    validate(&geometry);
    validate(&fragment);
    let both = [
        (0, Some(Interpolation::Perspective)),
        (1, Some(Interpolation::Flat)),
    ];
    assert_eq!(locations(&fragment), both);
    assert_eq!(locations(&geometry), both);
}

#[test]
fn clip_distances_become_a_discard_where_the_device_has_none() {
    let stages = || {
        let mut words = vec![vintrp(1, 4, 0, 0, 1)];
        words.extend(exp(0, [4, 4, 4, 4]));
        words.push(S_ENDPGM);
        let fragment = pixel(3, words);
        let mut words = vec![vop1(V_MOV, 0, F_ONE)];
        for target in [POS0, POS0 + 1, 33] {
            words.extend(exp(target, [0, 0, 0, 0]));
        }
        words.push(S_ENDPGM);
        let geometry = translate(&program(ShaderKind::Vertex, words)).unwrap();
        (geometry.module, fragment)
    };
    let kills = |module: &naga::Module| {
        let body = &module.entry_points[0].function.body;
        holds(body, &|statement| matches!(statement, Statement::Kill))
    };

    let (mut geometry, mut fragment) = stages();
    assert!(built_ins(&geometry).contains(&BuiltIn::ClipDistance));
    let options = LinkOptions {
        clip_distances: true,
        ..Default::default()
    };
    let linked = link(&mut geometry, &mut fragment, &options).unwrap();
    assert_eq!((linked.clip, linked.locations), (None, 1));
    assert!(built_ins(&geometry).contains(&BuiltIn::ClipDistance));
    assert!(!kills(&fragment));

    let (mut geometry, mut fragment) = stages();
    let linked = link(&mut geometry, &mut fragment, &LinkOptions::default()).unwrap();
    validate(&geometry);
    validate(&fragment);
    assert_eq!((linked.clip, linked.locations), (Some(1), 2));
    assert!(!built_ins(&geometry).contains(&BuiltIn::ClipDistance));
    assert!(kills(&fragment));
    let both = [
        (0, Some(Interpolation::Perspective)),
        (1, Some(Interpolation::Perspective)),
    ];
    assert_eq!(locations(&geometry), both);
    assert_eq!(locations(&fragment), both);
}

#[test]
fn a_carried_component_is_the_one_the_stage_before_wrote() {
    // Slot 1's z, alone: the vertex stage's location 0 holds parameter 1's third component first.
    let mut words = vec![vintrp(1, 4, 0, 2, 1)];
    words.extend(exp(0, [4, 4, 4, 4]));
    words.push(S_ENDPGM);
    let mut fragment = pixel(3, words);
    let mut geometry = vertex(&[1], &[]);
    link(&mut geometry, &mut fragment, &LinkOptions::default()).unwrap();
    let function = &geometry.entry_points[0].function;
    let Some(Statement::Return { value: Some(value) }) = function.body.last() else {
        panic!("the vertex stage returns nothing");
    };
    let E::Compose { components, .. } = &function.expressions[*value] else {
        panic!("the result is not composed");
    };
    let E::Compose { components, .. } = &function.expressions[components[1]] else {
        panic!("the carried vector is not composed");
    };
    assert!(matches!(
        function.expressions[components[0]],
        E::AccessIndex { index: 2, .. }
    ));
    // And the pixel stage puts the first carried component back in its parameter's z.
    let function = &fragment.entry_points[0].function;
    let stored = function.body.iter().find_map(|statement| match statement {
        Statement::Store { value, .. } => Some(*value),
        _ => None,
    });
    let E::Compose { components, .. } = &function.expressions[stored.unwrap()] else {
        panic!("the parameter is not composed");
    };
    assert!(matches!(
        function.expressions[components[2]],
        E::AccessIndex { index: 0, .. }
    ));
    assert!(matches!(function.expressions[components[0]], E::Literal(_)));
}
