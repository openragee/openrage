//! A model's material as the renderer shades it: its family, the key of the map in each slot, and its constants.

use or_renderer::world::{slot, WorldConstants, WorldFamily, WORLD_MAPS};

use crate::state::gta6_world::worldview::{CpuMaterial, Family, RegionTint};

/// The renderer's family of `family`; a water mask is never drawn, and is shaded as standard where one is.
pub(super) fn family_of(family: Family) -> WorldFamily {
    match family {
        Family::Standard | Family::WaterMask => WorldFamily::Standard,
        Family::TwoLayer => WorldFamily::TwoLayer,
        Family::FourLayer => WorldFamily::FourLayer,
        Family::Decal => WorldFamily::Decal,
        Family::Glass => WorldFamily::Glass,
        Family::Emissive => WorldFamily::Emissive,
        Family::Grass => WorldFamily::Grass,
        Family::TerrainLod => WorldFamily::TerrainLod,
        Family::Other => WorldFamily::Other,
    }
}

/// The key of the map each slot of `material` samples.
pub(super) fn slots_of(material: &CpuMaterial) -> [Option<u64>; WORLD_MAPS] {
    let mut slots = [None; WORLD_MAPS];
    let four = material.family == Family::FourLayer;
    // A four-layer material's fourth layer takes the slots of material B and the emissive map
    let layers = if four { 4 } else { 2 };
    for (at, layer) in material.layers.iter().take(layers).enumerate() {
        slots[slot::colour(at)] = layer.colour;
        slots[slot::normal(at)] = layer.normal;
        slots[slot::material_a(at)] = layer.material_a;
        if !four {
            slots[slot::MATERIAL_B[at]] = layer.material_b;
        }
    }
    slots[slot::WEIGHTS] = material.lookup;
    if four {
        (slots[slot::TINT], slots[slot::SECOND]) = (material.tint, material.tint_normal);
    } else {
        (slots[slot::TINT], slots[slot::SECOND]) = (material.palette, material.dirt);
        slots[slot::EMISSIVE] = material.emissive.map(|emissive| emissive.map);
    }
    if let Some(interior) = &material.interior {
        slots[slot::ROOM] = interior.room;
        slots[slot::ROOM_LIGHT] = interior.light;
        slots[slot::BACK_PLANE] = interior.plane;
    }
    slots
}

/// Whether the map in `at` is a layer's colour, which is a mid grey while it is waited for.
pub(super) fn is_colour(material: &CpuMaterial, at: usize) -> bool {
    let layers = if material.family == Family::FourLayer {
        4
    } else {
        2
    };
    (0..layers).any(|layer| slot::colour(layer) == at)
}

/// Whether the map in `at` is read texel by texel, so that it is wanted whole whatever the screen needs of it.
pub(super) fn read_whole(material: &CpuMaterial, at: usize) -> bool {
    at == slot::TINT && material.family != Family::FourLayer
}

/// The constants the renderer reads for `material`.
pub(super) fn constants_of(material: &CpuMaterial) -> WorldConstants {
    let c = &material.constants;
    let flag = |on: bool, bit: u32| if on { bit } else { 0 };
    let flags = flag(c.vertex_tint, WorldConstants::VERTEX_TINT)
        | flag(c.mask_is_alpha, WorldConstants::MASK_FROM_ALPHA)
        | flag(c.inv_control, WorldConstants::INVERT_CONTROL)
        | flag(c.hard_light, WorldConstants::HARD_LIGHT)
        | flag(c.foliage, WorldConstants::FOLIAGE)
        | flag(c.region_tint.is_some(), WorldConstants::INSTANCE_COLOUR)
        | flag(
            c.region_tint == Some(RegionTint::OverInstance),
            WorldConstants::VERTEX_OVER_TINT,
        )
        | WorldConstants::MODEL_VERTEX;
    let mut constants = WorldConstants {
        fresnel: c.fresnel,
        bumpiness: c.bumpiness,
        height_scale: c.hbb_range_scale,
        height_bias: c.hbb_range_bias,
        blend_softness: c.blend_softness,
        tint_intensity: c.tint_intensity,
        tint_blend: c.tint_blend_amount,
        vertex_tint_scale: c.vertex_tint_scale,
        vertex_tint_amount: c.vertex_tint_amount,
        control_filter: c.control_filter,
        opacity: c.decal_blend_opacity,
        uv0_transform: c.uv0_transform,
        uv1_transform: c.uv1_transform,
        uv_sets: c.uv_sets,
        uv_extra: [
            c.lookup_uv_set,
            0.0,
            c.layer1_uv_scale[0],
            c.layer1_uv_scale[1],
        ],
        flags,
        palette_row: c.tint_palette_selector,
        translucency_as_tint_mask: c.translucency_as_tint_mask,
        // A one-layer material's fresnel is its first layer's
        specular_fresnel: c.fresnel[0],
        layer0_tint: c.palette_tint[0],
        layer1_tint: c.palette_tint[1],
        control_softness: c.blend_softness[0],
        control_offset: c.layer_weight_bias,
        dirt_intensity: c.dirt_intensity,
        grass_tint: c.gtintensity,
        ..WorldConstants::default()
    };
    // A ground decal's height span reaches the shader as the first layer's scale and bias
    if let Some([low, high]) = c.ground_heights {
        constants.flags |= WorldConstants::GROUND_BLEND
            | flag(c.tint_multiplies, WorldConstants::TINT_MULTIPLY)
            | flag(c.tint_masked, WorldConstants::TINT_MASKED);
        constants.height_scale[0] = 1.0 / (high - low).max(1e-4);
        constants.height_bias[0] = low;
    }
    if let Some(emissive) = &material.emissive {
        let [r, g, b] = emissive.scale;
        let [pr, pg, pb] = emissive.power;
        constants.emissive_scale = [r, g, b, 1.0];
        constants.emissive_power = [pr, pg, pb, 1.0];
        constants.emissive_multiplier = emissive.multiplier;
        constants.emissive_night = emissive.night;
        constants.emissive_view_power = emissive.facing;
        constants.emissive_diffuse_tint = emissive.diffuse;
    }
    if let Some(interior) = &material.interior {
        constants.flags |=
            WorldConstants::INTERIOR | flag(interior.by_vertex, WorldConstants::ROOM_BY_VERTEX);
        constants.interior_room = interior.size;
        constants.interior_plane = interior.plane_of;
        constants.interior_glass = interior.glass;
        let [most, slant, glow] = interior.slant;
        constants.interior_slant = [most, slant, glow, 0.0];
    }
    constants
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::gta6_world::worldview::{Constants, Emissive, Interior, Layer};

    fn layer(base: u64) -> Layer {
        Layer {
            colour: Some(base),
            normal: Some(base + 1),
            material_a: Some(base + 2),
            material_b: Some(base + 3),
        }
    }

    #[test]
    fn a_four_layer_material_fills_every_layer_slot_and_its_tint_maps_and_states_no_material_b() {
        let material = CpuMaterial {
            family: Family::FourLayer,
            layers: vec![layer(10), layer(20), layer(30), layer(40)],
            lookup: Some(1),
            palette: Some(2),
            tint: Some(3),
            tint_normal: Some(4),
            ..CpuMaterial::default()
        };
        let slots = slots_of(&material);
        for (at, base) in [10, 20, 30, 40].into_iter().enumerate() {
            assert_eq!(slots[slot::colour(at)], Some(base));
            assert_eq!(slots[slot::normal(at)], Some(base + 1));
            assert_eq!(slots[slot::material_a(at)], Some(base + 2));
        }
        assert_eq!(slots[slot::WEIGHTS], Some(1));
        assert_eq!(slots[slot::TINT], Some(3));
        assert_eq!(slots[slot::SECOND], Some(4));
        assert!(!read_whole(&material, slot::TINT));
        assert!(is_colour(&material, slot::colour(3)));
        assert!(!is_colour(&material, slot::normal(0)));
        assert_eq!(family_of(material.family), WorldFamily::FourLayer);
    }

    #[test]
    fn a_two_layer_material_puts_its_material_b_maps_its_palette_dirt_and_glow_beside_its_layers() {
        let glow = Emissive {
            map: 99,
            multiplier: 5.0,
            night: -1.0,
            scale: [1.0, 0.5, 0.25],
            power: [1.0; 3],
            facing: 2.0,
            diffuse: 0.0,
        };
        let material = CpuMaterial {
            family: Family::TwoLayer,
            layers: vec![layer(10), layer(20)],
            lookup: Some(1),
            palette: Some(2),
            dirt: Some(5),
            emissive: Some(glow),
            constants: Constants {
                palette_tint: [0.25, 0.75, 1.0, 1.0],
                blend_softness: [0.3, 0.5, 0.5, 0.5],
                inv_control: true,
                hard_light: true,
                ..Constants::default()
            },
            ..CpuMaterial::default()
        };
        let slots = slots_of(&material);
        assert_eq!(slots[slot::colour(1)], Some(20));
        assert_eq!(slot::MATERIAL_B.map(|at| slots[at]), [Some(13), Some(23)]);
        assert_eq!(slots[slot::EMISSIVE], Some(99));
        assert_eq!(slots[slot::TINT], Some(2));
        assert_eq!(slots[slot::SECOND], Some(5));
        assert_eq!(slots[slot::colour(2)], None);
        assert!(read_whole(&material, slot::TINT));
        // Its material B of the first layer sits where a fourth layer's colour would
        assert!(!is_colour(&material, slot::MATERIAL_B[0]));
        let constants = constants_of(&material);
        assert_eq!((constants.layer0_tint, constants.layer1_tint), (0.25, 0.75));
        assert_eq!(constants.control_softness, 0.3);
        assert_eq!(constants.emissive_multiplier, 5.0);
        assert_eq!(constants.emissive_scale, [1.0, 0.5, 0.25, 1.0]);
        assert_eq!(constants.emissive_view_power, 2.0);
        let wanted = WorldConstants::MASK_FROM_ALPHA
            | WorldConstants::INVERT_CONTROL
            | WorldConstants::HARD_LIGHT
            | WorldConstants::MODEL_VERTEX;
        assert_eq!(constants.flags, wanted);
    }

    #[test]
    fn an_interior_window_puts_its_room_cubes_and_back_plane_in_the_third_layers_slots_and_its_room_in_its_constants(
    ) {
        let interior = Interior {
            room: Some(61),
            light: Some(62),
            plane: Some(63),
            by_vertex: true,
            size: [1.0, 2.5, 0.8, 1.0],
            plane_of: [1.0, 0.2, 2.0, 7.0],
            glass: [15.0, 12.5, 12.5, 0.02],
            slant: [0.04, 0.5, 1.0],
        };
        let material = CpuMaterial {
            family: Family::TwoLayer,
            layers: vec![layer(10), layer(20)],
            interior: Some(interior),
            ..CpuMaterial::default()
        };
        let slots = slots_of(&material);
        assert_eq!(
            [slot::ROOM, slot::ROOM_LIGHT, slot::BACK_PLANE].map(|at| slots[at]),
            [Some(61), Some(62), Some(63)]
        );
        let constants = constants_of(&material);
        let both = WorldConstants::INTERIOR | WorldConstants::ROOM_BY_VERTEX;
        assert_eq!(constants.flags & both, both);
        assert_eq!(constants.interior_room, interior.size);
        assert_eq!(constants.interior_glass, interior.glass);
        assert_eq!(constants.interior_slant, [0.04, 0.5, 1.0, 0.0]);
        // A two-layer material with no room leaves those slots to no map and sets neither flag
        let plain = CpuMaterial {
            interior: None,
            ..material
        };
        assert_eq!(slots_of(&plain)[slot::ROOM], None);
        assert_eq!(constants_of(&plain).flags & both, 0);
    }

    #[test]
    fn a_ground_decal_hands_the_shader_its_height_span_as_a_scale_and_bias_and_its_flags() {
        let material = CpuMaterial {
            family: Family::Decal,
            constants: Constants {
                ground_heights: Some([0.25, 0.75]),
                tint_multiplies: true,
                tint_masked: true,
                ..Constants::default()
            },
            ..CpuMaterial::default()
        };
        let constants = constants_of(&material);
        assert_eq!(
            (constants.height_scale[0], constants.height_bias[0]),
            (2.0, 0.25)
        );
        let both = WorldConstants::GROUND_BLEND
            | WorldConstants::TINT_MULTIPLY
            | WorldConstants::TINT_MASKED;
        assert_eq!(constants.flags & both, both);
        // Any other material blends no ground
        let plain = constants_of(&CpuMaterial::default());
        assert_eq!(plain.flags & both, 0);
    }

    #[test]
    fn foliage_reaches_the_shader_as_its_flag_and_nothing_else_does() {
        let tree = CpuMaterial {
            constants: Constants {
                foliage: true,
                ..Constants::default()
            },
            ..CpuMaterial::default()
        };
        assert_ne!(constants_of(&tree).flags & WorldConstants::FOLIAGE, 0);
        assert_eq!(
            constants_of(&CpuMaterial::default()).flags & WorldConstants::FOLIAGE,
            0
        );
    }

    #[test]
    fn a_region_tint_material_hands_the_shader_the_instance_s_colour_and_whether_the_vertex_lies_over_it(
    ) {
        let with = |region_tint| {
            let material = CpuMaterial {
                constants: Constants {
                    region_tint,
                    ..Constants::default()
                },
                ..CpuMaterial::default()
            };
            let flags = constants_of(&material).flags;
            let own = WorldConstants::INSTANCE_COLOUR;
            (
                flags & own != 0,
                flags & WorldConstants::VERTEX_OVER_TINT != 0,
            )
        };
        assert_eq!(with(Some(RegionTint::Instance)), (true, false));
        assert_eq!(with(Some(RegionTint::OverInstance)), (true, true));
        // A material that names no colour map of any other effect keeps its instance's tint off
        assert_eq!(with(None), (false, false));
    }

    #[test]
    fn a_material_with_no_glow_keeps_the_renderers_default_glow_and_every_family_has_its_namesake()
    {
        let constants = constants_of(&CpuMaterial::default());
        let neutral = WorldConstants::default();
        assert_eq!(constants.emissive_night, neutral.emissive_night);
        assert_eq!(constants.emissive_multiplier, neutral.emissive_multiplier);
        assert_eq!(constants.opacity, [1.0; 4]);
        let pairs = [
            (Family::Standard, WorldFamily::Standard),
            (Family::TwoLayer, WorldFamily::TwoLayer),
            (Family::FourLayer, WorldFamily::FourLayer),
            (Family::Decal, WorldFamily::Decal),
            (Family::Glass, WorldFamily::Glass),
            (Family::Emissive, WorldFamily::Emissive),
            (Family::Grass, WorldFamily::Grass),
            (Family::TerrainLod, WorldFamily::TerrainLod),
            (Family::Other, WorldFamily::Other),
        ];
        for (cpu, gpu) in pairs {
            assert_eq!(family_of(cpu), gpu);
        }
    }
}
