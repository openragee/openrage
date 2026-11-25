use std::collections::HashMap;

use or_io::gta6::headerless::{Parameter, ParameterValue, Shader};

use super::*;

/// A shader whose texture records state `records`, each with its key in that order, then `constants`.
fn shader(records: &[(u32, Option<u64>)], constants: &[(u32, &[f32])]) -> Shader {
    let textures = (records.iter().zip(0u8..)).map(|((hash, _), slot)| Parameter {
        hash: *hash,
        value: ParameterValue::Texture { slot },
    });
    let constants = constants.iter().map(|(hash, values)| Parameter {
        hash: *hash,
        value: ParameterValue::Constant {
            buffer: 0,
            offset: 0,
            words: values.iter().map(|v| v.to_bits()).collect(),
        },
    });
    Shader {
        name_hash: 1,
        texture_keys: records.iter().map(|(_, key)| *key).collect(),
        parameters: textures.chain(constants).collect(),
    }
}

/// The material of `shader` in the bucket of `byte`, with no pack indexed.
fn of(shader: &Shader, byte: u8) -> CpuMaterial {
    material_of(shader, byte, &|_| None)
}

fn h(name: &str) -> u32 {
    name_hash32(name.as_bytes())
}

fn layer(colour: u64, normal: u64, material_a: u64, material_b: Option<u64>) -> Layer {
    Layer {
        colour: Some(colour),
        normal: Some(normal),
        material_a: Some(material_a),
        material_b,
    }
}

/// A one-layer material's four records, keys 0xC, 0xB, 0xA, 0xA2.
fn one_layer() -> Vec<(u32, Option<u64>)> {
    vec![
        (h("DiffuseTex"), Some(0xC)),
        (h("BumpTex"), Some(0xB)),
        (h("SpecularTex"), Some(0xA)),
        (h("SpecularTex2"), Some(0xA2)),
    ]
}

/// A two-layer material's records: layer i's maps 0x1i0 to 0x1i3, then its control, dirt and palette maps.
fn two_layer() -> Vec<(u32, Option<u64>)> {
    let mut records = Vec::new();
    for i in 0..2u64 {
        for (k, map) in ["DiffuseTex", "NormalTex", "MaterialATex", "MaterialBTex"]
            .iter()
            .enumerate()
        {
            records.push((h(&format!("lyr{i}{map}")), Some(0x100 + 16 * i + k as u64)));
        }
    }
    records.extend([
        (h("controlTextureTex"), Some(0x1C)),
        (h("dirtDiffuseTex"), Some(0x1D)),
        (h("TintPaletteTex"), Some(0x1E)),
    ]);
    records
}

#[test]
fn a_map_joins_by_the_annotation_hash_its_record_states_in_the_drawables_own_order() {
    // A decal's drawable states its records in an order of its own: material A first.
    let records = [
        (h("SpecularTex"), Some(0xA)),
        (h("DiffuseTex"), Some(0xC)),
        (h("BumpTex"), Some(0xB)),
        (h("SpecularTex2"), Some(0xA2)),
        (h("TintPaletteTex"), Some(0x9)),
    ];
    let decal = of(&shader(&records, &[]), 2);
    assert_eq!(decal.layers, [layer(0xC, 0xB, 0xA, Some(0xA2))]);
    assert_eq!(decal.palette, Some(0x9));

    // The effect's variable hashes name no record a drawable states.
    let variables = [
        (0xC1C0_1CC4, Some(0xA)),
        (0x0149_D3C3, Some(0xA2)),
        (h("DiffuseTex"), Some(0xC)),
    ];
    let plain = of(&shader(&variables, &[]), 0);
    assert_eq!(
        (plain.layers[0].material_a, plain.layers[0].material_b),
        (None, None)
    );
    // A constant's too: the annotation `Fresnel`, never the variable `specularFresnel`.
    let variable = of(&shader(&[], &[(h("specularFresnel"), &[0.5])]), 0);
    assert_eq!(variable.constants.fresnel[0], 0.0);
    let annotation = of(&shader(&[], &[(h("Fresnel"), &[0.5])]), 0);
    assert_eq!(annotation.constants.fresnel[0], 0.5);
    // `bumpTexture` is a terrain LOD's normal record, and a one-layer one's where it states it.
    let bump = of(&shader(&[(h("bumpTexture"), Some(0xB))], &[]), 0);
    assert_eq!(bump.layers[0].normal, Some(0xB));
}

#[test]
fn a_four_layer_material_gives_each_layer_its_maps_and_the_lookup_and_tint_maps() {
    let mut records = Vec::new();
    for i in 0..4u64 {
        records.extend([
            (h(&format!("DiffuseTexture_layer{i}")), Some(0x100 + i)),
            (h(&format!("BumpTexture_layer{i}")), Some(0x200 + i)),
            (h(&format!("MaterialMapATexture_layer{i}")), Some(0x300 + i)),
        ]);
    }
    records.extend([
        (0x07BC_8533, Some(0x7)),
        (h("lookupTexture"), Some(0x4)),
        (h("tintTexture"), Some(0x5)),
        (h("tintBumpTexture"), Some(0x6)),
    ]);
    let material = of(&shader(&records, &[]), 0x80);
    assert_eq!(material.family, Family::FourLayer);
    let layers: Vec<Layer> = (0..4)
        .map(|i| layer(0x100 + i, 0x200 + i, 0x300 + i, None))
        .collect();
    assert_eq!(material.layers, layers);
    assert_eq!(
        (
            material.lookup,
            material.tint,
            material.tint_normal,
            material.palette
        ),
        (Some(0x4), Some(0x5), Some(0x6), None)
    );
    assert_eq!(material.emissive, None);
}

#[test]
fn a_two_layer_material_gives_both_layers_and_its_control_dirt_and_palette_maps() {
    let material = of(&shader(&two_layer(), &[]), 0);
    assert_eq!(material.family, Family::TwoLayer);
    assert_eq!(
        material.layers,
        [
            layer(0x100, 0x101, 0x102, Some(0x103)),
            layer(0x110, 0x111, 0x112, Some(0x113))
        ]
    );
    assert_eq!(
        (material.lookup, material.dirt, material.palette),
        (Some(0x1C), Some(0x1D), Some(0x1E))
    );
    // A two-layer decal is a decal, its layers as its records state them.
    let decal = of(&shader(&two_layer(), &[]), 0x82);
    assert_eq!(decal.family, Family::Decal);
    assert_eq!(decal.layers, material.layers);
}

#[test]
fn each_family_is_classed_by_its_bucket_byte_then_its_records_and_constants() {
    let family = |records: &[(u32, Option<u64>)], constants: &[(u32, &[f32])], byte: u8| {
        of(&shader(records, constants), byte).family
    };
    let one = one_layer();
    assert_eq!(family(&one, &[], 0), Family::Standard);
    assert_eq!(family(&one, &[], 3), Family::Standard);
    assert_eq!(family(&one, &[], 2), Family::Decal);
    assert_eq!(family(&one, &[], 1), Family::Glass);
    assert_eq!(family(&one, &[], 0x81), Family::Glass);
    assert_eq!(family(&one, &[], 6), Family::WaterMask);
    assert_eq!(
        family(&one, &[(h("gtintensity"), &[0.45])], 0x80),
        Family::Grass
    );
    assert_eq!(family(&[], &[], 0), Family::Other);

    // `standard_dirt`'s second colour is its dirt.
    let mut dirt = one.clone();
    dirt.push((h("DiffuseTex2"), Some(0xD)));
    assert_eq!(of(&shader(&dirt, &[]), 0).dirt, Some(0xD));

    // An emissive map bound makes a one-layer material emissive; a layered one keeps its family.
    let mut glowing = one.clone();
    glowing.push((h("EmissiveTex"), Some(0xE)));
    let emissive = of(&shader(&glowing, &[]), 0);
    assert_eq!(emissive.family, Family::Emissive);
    assert_eq!(emissive.emissive.map(|e| e.map), Some(0xE));
    let mut unbound = one.clone();
    unbound.push((h("EmissiveTex"), None));
    let plain = of(&shader(&unbound, &[]), 0);
    assert_eq!((plain.family, plain.emissive), (Family::Standard, None));
    let mut interior = two_layer();
    interior.push((h("EmissiveTex"), Some(0xE)));
    let interior = of(&shader(&interior, &[]), 0);
    assert_eq!(interior.family, Family::TwoLayer);
    assert_eq!(interior.emissive.map(|e| e.map), Some(0xE));

    // A terrain LOD: its own records for the normal and the material maps.
    let terrain = [
        (h("SpecularTexture"), Some(0xA)),
        (h("DiffuseTex"), Some(0xC)),
        (h("bumpTexture"), Some(0xB)),
        (h("MaterialBTexture"), Some(0xA2)),
    ];
    let building = [terrain.as_slice(), &[(h("TintPaletteTex"), Some(0xD))]].concat();
    let terrain = of(&shader(&terrain, &[]), 0);
    assert_eq!(terrain.family, Family::TerrainLod);
    assert_eq!(terrain.layers, [layer(0xC, 0xB, 0xA, Some(0xA2))]);
    // A building's far model states the same records and a palette: a one-layer material
    let building = of(&shader(&building, &[]), 0);
    assert_eq!(building.family, Family::Standard);
    assert_eq!(building.layers, terrain.layers);
    assert_eq!(building.palette, Some(0xD));
}

#[test]
fn a_four_layer_materials_constants_are_read_per_layer() {
    let constants: [(u32, &[f32]); 11] = [
        (h("Fresnel"), &[0.1, 0.2, 0.3, 0.4]),
        (h("Bumpiness"), &[1.5, 2.0, 2.5, 3.0]),
        (h("HBBRangeScale1234"), &[2.0, 3.0, 4.0, 5.0]),
        (h("HBBRangeBias1234"), &[0.1, 0.0, 0.2, 0.0]),
        (h("blendSoftness"), &[0.4, 0.75, 0.5, 0.6]),
        (h("tintIntensity"), &[0.0, 1.0, 2.0, 3.0]),
        (h("TintBlendAmount"), &[1.0, 0.0, 0.5, 0.0]),
        (0x81BC_5BCA, &[4.0, 3.0, 2.0, 1.0]),
        (0x896D_1E47, &[0.25, 0.5, 0.75, 1.0]),
        (0x3FB2_B8F7, &[1.0]),
        (0x7A09_0C2A, &[0.0]),
    ];
    let records = [(h("DiffuseTexture_layer0"), Some(0x100))];
    let c = of(&shader(&records, &constants), 0).constants;
    assert_eq!(c.fresnel, [0.1, 0.2, 0.3, 0.4]);
    assert_eq!(c.bumpiness, [1.5, 2.0, 2.5, 3.0]);
    assert_eq!(c.hbb_range_scale, [2.0, 3.0, 4.0, 5.0]);
    assert_eq!(c.hbb_range_bias, [0.1, 0.0, 0.2, 0.0]);
    assert_eq!(c.blend_softness, [0.4, 0.75, 0.5, 0.6]);
    assert_eq!(c.tint_intensity, [0.0, 1.0, 2.0, 3.0]);
    assert_eq!(c.tint_blend_amount, [1.0, 0.0, 0.5, 0.0]);
    assert_eq!(c.vertex_tint_scale, [4.0, 3.0, 2.0, 1.0]);
    assert_eq!(c.vertex_tint_amount, [0.25, 0.5, 0.75, 1.0]);
    assert!(c.vertex_tint);
    assert!(!c.mask_is_alpha);
    // What a four-layer material does not state is the default.
    assert_eq!(c.palette_tint, [1.0; 4]);
    assert_eq!(c.decal_blend_opacity, [1.0; 4]);
}

#[test]
fn a_layers_coordinate_set_and_transform_are_read_whether_stated_as_floats_or_integers() {
    let four: [(u32, &[f32]); 4] = [
        (0x4344_D2CC, &[128.0, 64.0, 0.5, 0.25]),
        (0x9ECA_AA1E, &[2.0, 2.0, 0.0, 0.0]),
        (h("uvSet"), &[1.0, 2.0, 1.0, 2.0]),
        (h("lookupUVSet"), &[3.0]),
    ];
    let records = [(h("DiffuseTexture_layer0"), Some(0x100))];
    let c = of(&shader(&records, &four), 0).constants;
    assert_eq!(c.uv0_transform, [128.0, 64.0, 0.5, 0.25]);
    assert_eq!(c.uv1_transform, [2.0, 2.0, 0.0, 0.0]);
    assert_eq!(c.uv_sets, [1.0, 2.0, 1.0, 2.0]);
    assert_eq!(c.lookup_uv_set, 3.0);
    // A two-layer material states its sets as integers, its control's set under its own name
    let mut two = shader(&two_layer(), &[(h("lyr1UVScale"), &[2.0, 3.0])]);
    for (name, set) in [("lyr0UVSet", 2u32), ("lyr1UVSet", 1), ("controlUVSet", 2)] {
        two.parameters.push(Parameter {
            hash: h(name),
            value: ParameterValue::Constant {
                buffer: 0,
                offset: 0,
                words: vec![set],
            },
        });
    }
    let c = of(&two, 0).constants;
    assert_eq!(c.uv_sets, [2.0, 1.0, 1.0, 1.0]);
    assert_eq!((c.lookup_uv_set, c.layer1_uv_scale), (2.0, [2.0, 3.0]));
    // A material that states none samples every map by its first coordinate, untransformed
    let d = Constants::default();
    assert_eq!((d.uv_sets, d.lookup_uv_set), ([1.0; 4], 1.0));
    assert_eq!(d.uv0_transform, [1.0, 1.0, 0.0, 0.0]);
}

#[test]
fn a_two_layer_materials_constants_go_to_their_own_layer() {
    let constants: [(u32, &[f32]); 12] = [
        (h("lyr0Fresnel"), &[0.1]),
        (h("lyr1Fresnel"), &[0.2]),
        (h("lyr0Bumpiness"), &[2.0]),
        (h("lyr1Bumpiness"), &[3.0]),
        (h("lyr0Tint"), &[0.5]),
        (h("lyr1Tint"), &[0.25]),
        (h("controlFilter"), &[1.0, 0.0, 0.0, 0.0]),
        (h("invControl"), &[1.0]),
        (h("blendSoftness"), &[0.4]),
        (0x2926_5D05, &[0.3]),
        (h("dirtIntensity"), &[0.7]),
        (h("bUseHardLight"), &[1.0]),
    ];
    let c = of(&shader(&two_layer(), &constants), 0).constants;
    assert_eq!(c.fresnel, [0.1, 0.2, 0.0, 0.0]);
    assert_eq!(c.bumpiness, [2.0, 3.0, 1.0, 1.0]);
    assert_eq!(c.palette_tint, [0.5, 0.25, 1.0, 1.0]);
    assert_eq!(c.control_filter, [1.0, 0.0, 0.0, 0.0]);
    assert!(c.inv_control && c.hard_light);
    // A scalar stated where four may be: the first, the rest the default.
    assert_eq!(c.blend_softness, [0.4, 0.5, 0.5, 0.5]);
    assert_eq!((c.layer_weight_bias, c.dirt_intensity), (0.3, 0.7));
}

#[test]
fn a_one_layer_decal_grass_and_emissive_materials_constants_are_read() {
    let standard: [(u32, &[f32]); 3] = [
        (h("translucencyAsTintMask"), &[1.0]),
        (h("tintPaletteSelector"), &[3.0, 0.0]),
        (h("Bumpiness"), &[2.0]),
    ];
    let c = of(&shader(&one_layer(), &standard), 0).constants;
    assert_eq!(
        (c.translucency_as_tint_mask, c.tint_palette_selector),
        (1.0, 3.0)
    );
    assert_eq!(c.bumpiness, [2.0, 1.0, 1.0, 1.0]);

    let opacity = [1.0, 0.0, 0.5, 0.0];
    let decal: Vec<(u32, &[f32])> = (0..4)
        .map(|i| (h(&format!("DecalBlendOpacityGBuffer{i}")), &opacity[i..=i]))
        .collect();
    let c = of(&shader(&one_layer(), &decal), 2).constants;
    assert_eq!(c.decal_blend_opacity, [1.0, 0.0, 0.5, 0.0]);

    let grass = of(&shader(&one_layer(), &[(h("gtintensity"), &[0.45])]), 0);
    assert_eq!(grass.constants.gtintensity, 0.45);

    let mut glowing = one_layer();
    glowing.push((h("EmissiveTex"), Some(0xE)));
    let stated: [(u32, &[f32]); 6] = [
        (h("emissivemultiplier"), &[5.0]),
        (0x823D_5749, &[1.0]),
        (0x7AB0_FD74, &[2.0, 2.0, 2.0]),
        (0x5FCB_8723, &[1.0, 1.0, 2.0]),
        (0x7FBC_292C, &[3.0]),
        (0xC0FE_E36D, &[0.0]),
    ];
    let emissive = of(&shader(&glowing, &stated), 0).emissive;
    let expected = Emissive {
        map: 0xE,
        multiplier: 5.0,
        night: 1.0,
        scale: [2.0; 3],
        power: [1.0, 1.0, 2.0],
        facing: 3.0,
        diffuse: 0.0,
    };
    assert_eq!(emissive, Some(expected));
    // An emissive material that states none of them glows by its map alone, all day.
    let bare = of(&shader(&glowing, &[]), 0).emissive;
    let defaults = Emissive {
        map: 0xE,
        multiplier: 1.0,
        night: -1.0,
        scale: [1.0; 3],
        power: [1.0; 3],
        facing: 0.0,
        diffuse: 1.0,
    };
    assert_eq!(bare, Some(defaults));

    // The two-layer interior effect states its night strength under an annotation of its own.
    let mut interior = two_layer();
    interior.push((h("EmissiveTex"), Some(0xE)));
    let night: [(u32, &[f32]); 2] = [(h("emissivemultiplier"), &[5.0]), (0x605D_899E, &[1.0])];
    let interior = of(&shader(&interior, &night), 0).emissive;
    assert_eq!(interior.map(|e| e.night), Some(1.0));
}

#[test]
fn a_material_that_states_no_constant_has_the_defaults() {
    assert_eq!(
        of(&shader(&one_layer(), &[]), 0).constants,
        Constants::default()
    );
    let d = Constants::default();
    assert_eq!((d.bumpiness, d.hbb_range_scale), ([1.0; 4], [1.0; 4]));
    assert_eq!(d.control_filter, [0.0, 0.0, 1.0, 0.0]);
    assert!(d.mask_is_alpha && !d.vertex_tint);
    // Fewer words than the member holds: those stated, then the default.
    let short = of(&shader(&[], &[(h("HBBRangeScale1234"), &[3.0, 4.0])]), 0);
    assert_eq!(short.constants.hbb_range_scale, [3.0, 4.0, 1.0, 1.0]);
}

#[test]
fn the_colour_of_a_material_with_no_colour_record_is_a_key_no_record_names() {
    let (normal, specular, other, data) = (0xB0B_u64, 0x5EC_u64, 0x07E_u64, 0xDA7A_u64);
    let unknown = 0x1234_5678;
    // An unbound colour record gives way to the first key a pack states as sRGB.
    let unbound = shader(&[(h("DiffuseTex"), None), (unknown, Some(other))], &[]);
    let packed = |key: u64| Some(key == other || key == specular);
    assert_eq!(
        material_of(&unbound, 0, &packed).layers[0].colour,
        Some(other)
    );
    // A normal and a material map are never the colour, whatever a pack states of them.
    let bumped = shader(
        &[
            (h("BumpTex"), Some(normal)),
            (h("SpecularTex"), Some(specular)),
        ],
        &[],
    );
    let maps = material_of(&bumped, 0, &packed).layers[0];
    assert_eq!(
        (maps.colour, maps.normal, maps.material_a),
        (None, Some(normal), Some(specular))
    );
    assert_eq!(of(&bumped, 0).layers[0].colour, None);
    // A key no record names and no pack is known to hold is the one asked for as the colour.
    let more = shader(&[(h("BumpTex"), Some(normal)), (unknown, Some(data))], &[]);
    assert_eq!(of(&more, 0).layers[0].colour, Some(data));
    assert_eq!(
        material_of(&more, 0, &|_| Some(false)).layers[0].colour,
        None
    );
}

#[test]
fn a_materials_keys_are_each_named_once_and_equal_materials_are_one_key_of_a_map() {
    let mut records = one_layer();
    records.extend([
        (h("TintPaletteTex"), Some(0x9)),
        (h("DiffuseTex2"), Some(0xC)),
        (h("EmissiveTex"), Some(0xE)),
    ]);
    let material = of(&shader(&records, &[]), 0);
    assert_eq!(material.keys(), [0xC, 0xB, 0xA, 0xA2, 0x9, 0xE]);

    let fresnel = |value: f32| of(&shader(&records, &[(h("Fresnel"), &[value])]), 0);
    let mut seen = HashMap::new();
    for (at, material) in [fresnel(0.0), fresnel(0.0), fresnel(0.5)]
        .into_iter()
        .enumerate()
    {
        seen.entry(material).or_insert(at);
    }
    assert_eq!(seen.len(), 2);
}

#[test]
fn a_ground_decal_states_the_heights_its_blend_spans_and_whether_its_tint_multiplies() {
    let ground = shader(
        &one_layer(),
        &[
            (0x1BCE_EA11, &[0.25]),
            (0xAED6_191D, &[0.75]),
            (0x40D6_CCD6, &[1.0]),
            (0x236B_91CA, &[1.0]),
        ],
    );
    let c = of(&ground, 2).constants;
    assert_eq!(
        (c.ground_heights, c.tint_multiplies, c.tint_masked),
        (Some([0.25, 0.75]), true, true)
    );
    // A decal that states neither height blends by its coverage alone
    let plain = of(&shader(&one_layer(), &[]), 2).constants;
    assert_eq!(
        (
            plain.ground_heights,
            plain.tint_multiplies,
            plain.tint_masked
        ),
        (None, false, false)
    );
}

#[test]
fn a_tree_is_foliage_by_the_wind_thresholds_its_effect_states_and_a_plain_material_is_not() {
    let tree = shader(&one_layer(), &[(h("WindMinMaxThreshholds0"), &[0.8, 1.1])]);
    assert!(of(&tree, 0).constants.foliage);
    assert!(!of(&shader(&one_layer(), &[]), 0).constants.foliage);
    // Grass, told by the tint intensity it states even at 0, is foliage as well
    let grass = shader(&one_layer(), &[(h("gtintensity"), &[0.0])]);
    assert!(of(&grass, 0).constants.foliage);
}

#[test]
fn only_the_region_tint_effects_take_their_colour_from_the_instance_and_one_lays_the_vertex_over_it(
) {
    // A region-tint list's material names material B alone, as a material of any effect may
    let records = [(h("SpecularTex2"), Some(0xA2))];
    let with = |effect: &str| {
        let shader = Shader {
            name_hash: h(effect),
            ..shader(&records, &[])
        };
        of(&shader, 0).constants.region_tint
    };
    assert_eq!(
        with("prop_batch_slod_regiontint"),
        Some(RegionTint::Instance)
    );
    assert_eq!(
        with("prop_batch_slod_regiontint_cpv"),
        Some(RegionTint::OverInstance)
    );
    assert_eq!(
        with("signs"),
        None,
        "a material naming no colour of another effect"
    );
    assert_eq!(with("prop_batch"), None);
}

#[test]
fn materials_equal_in_all_but_a_constant_are_two_keys_and_equal_ones_one() {
    let colour = |key| CpuMaterial {
        layers: vec![Layer {
            colour: Some(key),
            ..Layer::default()
        }],
        ..CpuMaterial::default()
    };
    let mut bumpier = colour(7);
    bumpier.constants.bumpiness[0] = 2.0;
    let mut kept = HashMap::new();
    for (material, id) in [
        (colour(7), 0),
        (bumpier.clone(), 1),
        (colour(8), 2),
        (colour(7), 9),
    ] {
        kept.entry(material).or_insert(id);
    }
    assert_eq!(kept.len(), 3);
    assert_eq!(kept[&colour(7)], 0);
    assert_eq!(kept[&bumpier], 1);
}

#[test]
fn an_interior_window_names_its_room_cubes_and_back_plane_and_reads_its_room_from_its_constants() {
    let mut records = two_layer();
    records.extend([
        (0x17D3_B5C2, Some(0x5A)),
        (0x633A_EF4D, Some(0x5B)),
        (0x6994_EF3F, Some(0x5C)),
    ]);
    let constants: &[(u32, &[f32])] = &[
        (0x6AD7_14D8, &[1.0]),
        (0x2881_A0FF, &[2.5]),
        (0xC096_32A5, &[0.5]),
        (0x1C25_0A16, &[15.0, 12.5, 12.5, 0.0]),
        (0x0F2E_7960, &[0.02]),
    ];
    let material = of(&shader(&records, constants), 0);
    let interior = material
        .interior
        .expect("the room cube's record makes it an interior window");
    assert_eq!(
        (interior.room, interior.light, interior.plane),
        (Some(0x5A), Some(0x5B), Some(0x5C))
    );
    assert!(interior.by_vertex);
    assert_eq!(interior.size[1], 2.5);
    // The back plane's alpha is divided by its cut, as a scale
    assert_eq!(interior.plane_of[2], 2.0);
    assert_eq!(interior.glass, [15.0, 12.5, 12.5, 0.02]);
    let keys = material.keys();
    assert!([0x5A, 0x5B, 0x5C].iter().all(|k| keys.contains(k)));
    // A two-layer material that states no room cube has no room behind it
    assert_eq!(of(&shader(&two_layer(), &[]), 0).interior, None);
}
