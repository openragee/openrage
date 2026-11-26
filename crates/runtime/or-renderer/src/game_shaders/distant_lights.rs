//! Lights seen from afar drawn as the glow of their source: each one a camera-facing sprite of the pack's `DistantLights` pass, added into the lit frame where nothing nearer stands.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use glam::Vec3;
use or_io::gta5::shaders::spirv_pack::SignatureElement;
use wgpu::util::DeviceExt;

use super::deferred::{lighting_values, Screen, DEPTH_STENCIL_FORMAT};
use super::engine::EngineConstants;
use super::light_volumes::record_bytes;
use super::pack::GamePack;
use super::pipeline::{DrawState, Target};
use super::screen::{vertex_inputs, ScreenError, ScreenPass, Space};
use super::stand_in::Provided;

const EFFECT: &str = "im";
const TECHNIQUE: &str = "DistantLights";
/// The pass that measures the distance in three dimensions.
const PASS: usize = 0;

/// How many sprites are drawn at once: more than the distant lights the lists around a camera over the city hold.
pub const MOST: usize = 1 << 18;

/// The side of the texture a sprite's shape is read from, in texels.
const DOT: u32 = 32;

/// One distant light: where it stands, its colour, and its sprite's strength before the terms of the view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DistantLight {
    pub position: Vec3,
    pub colour: Vec3,
    /// Its stated intensity times the settings' gain and its fade.
    pub strength: f32,
    /// The way a spot shines, a unit vector; nought for a light that shines all round.
    pub direction: Vec3,
}

/// The height in pixels of the screen GTA VI renders, which a sprite's radiance grows with.
// ⚠ The program reads the render height, so the same light is three times brighter at 2160 rows than at 720: a viewer drawn in a window of its own size takes the game's height instead, or a small window dims every distant light.
// ponytail: the console's 2160 rows at a render scale of one, the default of the scale's setting; a scale read from the install would replace it.
pub const GAME_HEIGHT: f32 = 2160.0;

/// How GTA VI draws a distant light: a sprite of a constant share of the screen's height, whose radiance at its middle is its strength times the game's screen height in pixels and `area`, over the tangent of half the lens's upright angle and the distance along the view, times what the air lets through; a spot fades as the eye leaves its cone and every light twinkles with the distance.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SpriteLook {
    /// The sprite's side, as a share of half the screen's height.
    pub coverage: f32,
    pub area: f32,
    /// What the air takes a metre along the view, as an exponent.
    pub extinction: f32,
    /// What every sprite's strength is multiplied by for the frame's units, which the pass applies, so that a change of it writes no sprite again.
    pub exposure: f32,
    /// How far a spot's sprite is moved the way it shines, in metres.
    pub offset: f32,
    /// The cosines of a spot's inner and outer fade angles near and far, and the distances from the eye that are near and far, in metres.
    pub cones: [f32; 4],
    pub cone_distances: [f32; 2],
    /// The distances along the view over which a twinkle grows to its full amount, and the amount.
    pub twinkle: [f32; 3],
}

impl SpriteLook {
    /// The radiance at its middle of a sprite of `strength` `z` metres along the view, seen through a lens whose half upright angle has tangent `tan_half`.
    pub fn radiance(&self, strength: f32, z: f32, tan_half: f32) -> f32 {
        if !(z > 0.0 && tan_half > 0.0) {
            return 0.0;
        }
        strength * GAME_HEIGHT * self.area / (tan_half * z) * (-self.extinction * z).exp()
    }

    /// How much of a spot's sprite an eye `toward_eye` from it (a unit vector) `distance` metres off sees: the cone it fades across widens from the near angles to the far ones with the distance, and goes on widening past it.
    pub fn cone_fade(&self, direction: Vec3, toward_eye: Vec3, distance: f32) -> f32 {
        if direction == Vec3::ZERO {
            return 1.0;
        }
        let [near, far] = self.cone_distances;
        let t = (distance - near) / (far - near).max(1e-3);
        let [inner_near, outer_near, inner_far, outer_far] = self.cones;
        let inner = inner_near + (inner_far - inner_near) * t;
        let outer = outer_near + (outer_far - outer_near) * t;
        ((direction.dot(toward_eye) - outer) / (inner - outer).max(1e-4)).clamp(0.0, 1.0)
    }

    /// What a light of noise `noise` (0 to 1) `z` metres along the view is multiplied by as it twinkles: more with the distance, held between 0.15 and 1.85.
    pub fn twinkled(&self, noise: f32, z: f32) -> f32 {
        let [near, far, amount] = self.twinkle;
        let grown = (z - near) / (far - near).max(1e-3);
        (1.0 + grown * (noise - 0.5) * amount).clamp(0.15, 1.85)
    }

    /// What the pass is given for that radiance: it lays a sprite out as large as its side times its distance along the view, divides its colour by that distance squared, and gains with the distance `distance`.
    fn given(&self, strength: f32, z: f32, distance: f32, tan_half: f32) -> f32 {
        self.radiance(strength, z, tan_half) * z.max(1.0).powi(2) / distance_gain(distance)
    }

    /// The fields a sprite's record depends on, all but the exposure, as bits.
    fn bits(&self) -> [u32; 13] {
        let [a, b, c, d] = self.cones;
        let [e, f] = self.cone_distances;
        let [g, h, i] = self.twinkle;
        [
            self.coverage,
            self.area,
            self.extinction,
            self.offset,
            a,
            b,
            c,
            d,
            e,
            f,
            g,
            h,
            i,
        ]
        .map(f32::to_bits)
    }
}

/// A value from 0 to 1 that stays with a light's place, in place of the program's noise texture.
// ⚠ The game reads it from a texture that moves with the time; this one is the texture still, as one frame of the game.
pub fn noise_at(position: Vec3) -> f32 {
    let [x, y, z] = position.to_array().map(f32::to_bits);
    let mut h = x ^ y.rotate_left(11) ^ z.rotate_left(22);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^= h >> 16;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// What the lights were last written for: the list, held so that no other list takes its place, the view and the look.
type WrittenFor = (Arc<[DistantLight]>, [u32; 16], u32, [u32; 13]);

/// What the pass multiplies a sprite `distance` metres from the eye by, on top of its strength: one to 500 m, rising to sixteen at 2,500 m.
pub fn distance_gain(distance: f32) -> f32 {
    1.0 + 15.0 * ((distance - 500.0) * 0.0005).clamp(0.0, 1.0)
}

/// The texel at `x, y` of the shape a sprite is drawn with: the pass takes its red to the fourth power, which falls from one in the middle to nought at the edge of the circle the square holds.
fn dot_texel(x: u32, y: u32) -> u8 {
    let at = |i: u32| (i as f32 + 0.5) / DOT as f32 * 2.0 - 1.0;
    let r = (at(x).hypot(at(y))).min(1.0);
    ((1.0 - r) * 255.0).round() as u8
}

/// How much of a sprite's square its shape fills, as the pass reads it: the mean of the red to the fourth power.
pub fn dot_coverage() -> f32 {
    let mut sum = 0.0;
    for y in 0..DOT {
        for x in 0..DOT {
            sum += (f32::from(dot_texel(x, y)) / 255.0).powi(4);
        }
    }
    sum / (DOT * DOT) as f32
}

pub(super) fn dot() -> Provided {
    let rgba = (0..DOT * DOT)
        .flat_map(|i| {
            let v = dot_texel(i % DOT, i / DOT);
            [v, v, v, 255]
        })
        .collect();
    Provided::Texture2d {
        width: DOT,
        height: DOT,
        rgba,
        srgb: false,
    }
}

/// What the input `semantic` of the vertex stage reads of a light.
fn light_value(semantic: &str, light: &DistantLight) -> [f32; 4] {
    let (p, c) = (light.position, light.colour);
    match semantic {
        "POSITION" => [p.x, p.y, p.z, 1.0],
        "COLOR" => [c.x, c.y, c.z, light.strength],
        _ => [0.0; 4],
    }
}

/// The six corners of a sprite, the pass laying each at its texture coordinate less a half.
const CORNERS: [[f32; 2]; 6] = [
    [0.0, 0.0],
    [1.0, 0.0],
    [1.0, 1.0],
    [0.0, 0.0],
    [1.0, 1.0],
    [0.0, 1.0],
];

/// The sprites of the distant lights, drawn into the lit frame.
pub struct DistantLights {
    pass: ScreenPass,
    inputs: Vec<SignatureElement>,
    corners: wgpu::Buffer,
    instances: wgpu::Buffer,
    drawn: AtomicU32,
    written: Mutex<Option<WrittenFor>>,
}

impl DistantLights {
    /// The pass, drawn additively into colour targets of `colour`, tested against the depth of the frame and writing none.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pack: &GamePack,
        colour: wgpu::TextureFormat,
    ) -> Result<Self, ScreenError> {
        let inputs =
            vertex_inputs(pack, EFFECT, TECHNIQUE, PASS).ok_or_else(|| ScreenError::NoPass {
                effect: EFFECT.into(),
                technique: TECHNIQUE.into(),
                pass: PASS,
            })?;
        let add = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let target = Target {
            colour,
            depth: DEPTH_STENCIL_FORMAT,
            depth_compare: wgpu::CompareFunction::GreaterEqual,
            blend: Some(wgpu::BlendState {
                color: add,
                alpha: add,
            }),
            colour_attachments: Some(1),
        };
        let draw = DrawState {
            instance_locations: (inputs.iter())
                .filter(|i| i.semantic != "TEXCOORD")
                .map(|i| i.register)
                .collect(),
            depth_attachment: true,
            depth_tested: true,
            depth_write: Some(false),
            write_mask: Some(wgpu::ColorWrites::COLOR),
            ..DrawState::default()
        };
        let pass = ScreenPass::new(
            device,
            queue,
            pack,
            EFFECT,
            TECHNIQUE,
            PASS,
            &target,
            &draw,
            Space::Unit,
            &mut |name, _| match name {
                "DiffuseSampler" => Some(dot()),
                _ => None,
            },
        )?;
        let streams = pass.streams();
        let corners: Vec<u8> =
            (CORNERS.iter())
                .flat_map(|&[u, v]| {
                    record_bytes(&streams.vertex, streams.vertex_stride, &inputs, &|input| {
                        match input.semantic.as_str() {
                            "TEXCOORD" => [u, v, 0.0, 0.0],
                            _ => [0.0; 4],
                        }
                    })
                })
                .collect();
        let corners = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("distant-light-corners"),
            contents: &corners,
            usage: wgpu::BufferUsages::VERTEX,
        });
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("distant-lights"),
            size: streams.instance_stride.max(4) * MOST as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            pass,
            inputs,
            corners,
            instances,
            drawn: AtomicU32::new(0),
            written: Mutex::new(None),
        })
    }

    /// Write the constants of this frame and the first [`MOST`] of `lights`, drawn as `look` says.
    pub fn write(
        &self,
        queue: &wgpu::Queue,
        engine: &EngineConstants,
        screen: &Screen,
        look: SpriteLook,
        lights: &Arc<[DistantLight]>,
    ) {
        let all = lights;
        let lights = &lights[..lights.len().min(MOST)];
        self.drawn.store(lights.len() as u32, Ordering::Relaxed);
        if lights.is_empty() {
            return;
        }
        let tan_half = 1.0 / engine.projection.y_axis.y;
        let mut values = engine.values();
        values.extend(lighting_values(engine, screen));
        // The pass's own fade by distance is off, and every light is drawn from where it stands: the caller fades each one.
        values.insert("refMipBlurParams", vec![0.0, 0.0, 1.0, 0.0]);
        // A side of the share of the screen at one metre along the view, which the pass grows by the distance along it.
        let side = look.coverage * tan_half;
        values.insert("GeneralParams0", vec![side, 0.0, 0.0, look.exposure]);
        // The noise is a white stand-in, so a twinkle of one takes nothing away: each light's twinkle is in its strength.
        values.insert("GeneralParams1", vec![1.0, 1.0, 0.0, 1.0]);
        self.pass.write(queue, &values);
        // ⚠ A quarter of a million lights, each worked out and written: only when they or the view changed.
        let view = engine.view.to_cols_array().map(f32::to_bits);
        let key: WrittenFor = (Arc::clone(all), view, tan_half.to_bits(), look.bits());
        let mut written = self.written.lock().unwrap_or_else(|p| p.into_inner());
        let same = |was: &WrittenFor| {
            Arc::ptr_eq(&was.0, &key.0) && (was.1, was.2, was.3) == (key.1, key.2, key.3)
        };
        if written.as_ref().is_some_and(same) {
            return;
        }
        *written = Some(key);
        let camera = engine.view.inverse();
        let (eye, forward) = (camera.w_axis.truncate(), -camera.z_axis.truncate());
        let streams = self.pass.streams();
        let stride = streams.instance_stride as usize;
        // Each attribute's place in a record and the semantic it reads, found once.
        let slots: Vec<(usize, usize, &str)> = (streams.instance.iter())
            .filter_map(|a| {
                let input = (self.inputs.iter()).find(|i| i.register == a.shader_location)?;
                Some((
                    a.offset as usize,
                    (a.format.size() / 4) as usize,
                    input.semantic.as_str(),
                ))
            })
            .collect();
        let mut bytes = vec![0u8; stride * lights.len()];
        for (record, light) in bytes.chunks_exact_mut(stride.max(1)).zip(lights) {
            let toward = light.position - eye;
            let (distance, z) = (toward.length(), toward.dot(forward));
            let seen = look.cone_fade(light.direction, -toward / distance.max(1e-3), distance);
            let strength = light.strength * seen * look.twinkled(noise_at(light.position), z);
            let position = light.position + light.direction * look.offset;
            let toward = position - eye;
            let given = DistantLight {
                position,
                strength: look.given(strength, toward.dot(forward), toward.length(), tan_half),
                ..*light
            };
            for &(offset, words, semantic) in &slots {
                let values = light_value(semantic, &given);
                for (i, v) in values.iter().take(words).enumerate() {
                    let at = offset + 4 * i;
                    record[at..at + 4].copy_from_slice(&v.to_le_bytes());
                }
            }
        }
        queue.write_buffer(&self.instances, 0, &bytes);
    }

    /// Draw the lights the frame was written with into `pass`: its colour target of the format this was built for, and the depth of the frame, which it only reads.
    pub fn record(&self, pass: &mut wgpu::RenderPass<'_>) {
        let drawn = self.drawn.load(Ordering::Relaxed);
        if drawn == 0 {
            return;
        }
        self.pass.record_instanced(
            pass,
            &self.corners,
            CORNERS.len() as u32,
            &self.instances,
            0..drawn,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shape_is_a_disc_bright_in_the_middle_and_dark_at_its_edge() {
        let middle = dot_texel(DOT / 2, DOT / 2);
        assert!(middle > 240, "{middle}");
        assert_eq!(dot_texel(0, 0), 0);
        // The fourth power of a cone over a disc fills a sixtieth of pi of its square.
        let coverage = dot_coverage();
        let exact = std::f32::consts::PI / 60.0;
        assert!((coverage - exact).abs() < 0.1 * exact, "{coverage} {exact}");
    }

    #[test]
    fn a_sprite_falls_with_the_distance_along_the_view_and_the_pass_s_own_terms_are_taken_out() {
        let look = SpriteLook {
            coverage: 0.011,
            area: 0.05,
            extinction: 0.0007,
            exposure: 1.0,
            ..SpriteLook::default()
        };
        let tan_half = 0.5;
        let at = |z: f32| look.radiance(10.0, z, tan_half);
        // The game's screen height, whatever the window's.
        let wanted = 10.0 * 2160.0 * 0.05 / (0.5 * 400.0) * (-0.28f32).exp();
        assert!((at(400.0) - wanted).abs() < 1e-4 * wanted, "{}", at(400.0));
        assert!((at(200.0) / at(400.0) - 2.0 * (0.14f32).exp()).abs() < 1e-4);
        assert_eq!(at(-1.0), 0.0, "behind the eye");
        // The pass multiplies by its gain and divides by the distance along the view squared.
        let given = look.given(10.0, 1200.0, 1300.0, tan_half);
        let seen = given * distance_gain(1300.0) / (1200.0f32 * 1200.0);
        assert!((seen - at(1200.0)).abs() < 1e-4 * at(1200.0));
    }

    fn settled() -> SpriteLook {
        let cos = |d: f32| d.to_radians().cos();
        SpriteLook {
            cones: [cos(0.0), cos(90.0), cos(0.0), cos(120.0)],
            cone_distances: [100.0, 1000.0],
            twinkle: [100.0, 5000.0, 100.0],
            ..SpriteLook::default()
        }
    }

    #[test]
    fn a_spot_is_seen_from_within_its_cone_and_its_cone_widens_with_the_distance() {
        let look = settled();
        let down = Vec3::NEG_Z;
        // Under the lamp, beside it, and above it.
        assert_eq!(look.cone_fade(down, Vec3::NEG_Z, 100.0), 1.0);
        assert!(look.cone_fade(down, Vec3::X, 100.0) < 1e-6);
        assert_eq!(look.cone_fade(down, Vec3::Z, 1000.0), 0.0);
        // The program widens the cone on past the far distance: from straight above at 2,000 m a little shows.
        assert!((look.cone_fade(down, Vec3::Z, 2000.0) - 0.027).abs() < 1e-3);
        // Beside it, far off: the outer angle is 120° at 1,000 m, so a right angle is a third across.
        let beside = look.cone_fade(down, Vec3::X, 1000.0);
        assert!((beside - 1.0 / 3.0).abs() < 1e-4, "{beside}");
        assert_eq!(look.cone_fade(Vec3::ZERO, Vec3::Z, 50.0), 1.0, "a point");
    }

    #[test]
    fn a_light_twinkles_more_with_the_distance_and_within_its_bounds() {
        let look = settled();
        assert_eq!(look.twinkled(0.9, 100.0), 1.0, "none at the near distance");
        // A tenth of the way to the far distance, a twentieth above the middle of the noise.
        assert!((look.twinkled(0.55, 590.0) - 1.5).abs() < 1e-4);
        assert_eq!(look.twinkled(0.0, 5000.0), 0.15);
        assert_eq!(look.twinkled(1.0, 5000.0), 1.85);
        assert_eq!(
            SpriteLook::default().twinkled(0.0, 3000.0),
            1.0,
            "no amount"
        );
        let spread: Vec<f32> = (0..64)
            .map(|i| noise_at(Vec3::new(i as f32, 7.0, 3.0)))
            .collect();
        assert!(spread.iter().all(|n| (0.0..1.0).contains(n)));
        let mean = spread.iter().sum::<f32>() / 64.0;
        assert!((mean - 0.5).abs() < 0.12, "{mean}");
    }

    #[test]
    fn a_light_gives_the_pass_its_place_and_its_colour_times_its_strength() {
        let light = DistantLight {
            position: Vec3::new(1.0, 2.0, 3.0),
            colour: Vec3::new(0.5, 0.25, 1.0),
            strength: 8.0,
            direction: Vec3::ZERO,
        };
        assert_eq!(light_value("POSITION", &light), [1.0, 2.0, 3.0, 1.0]);
        assert_eq!(light_value("COLOR", &light), [0.5, 0.25, 1.0, 8.0]);
        assert_eq!(light_value("TEXCOORD", &light), [0.0; 4]);
    }
}
