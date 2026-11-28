//! The sun's shadow on a GTA VI model: its surfaces drawn again as the light sees them, into one
//! tile fitted to the model, which the resolve reads a pixel's share of the light from.

use glam::{Mat4, Vec3};

use super::super::shadow::{sphere_of, Fit};
use super::super::targets::render_target;
use super::Sky;

/// The texels a side of the tile.
pub(super) const TILE: u32 = 1024;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// How far behind itself a surface is held, in the tile's depth: a texel and a half of its width.
pub(super) const BIAS: f32 = 1.5 / TILE as f32;
/// The tangent of half the angle the sun's disc is seen under, where no cycle states one: how
/// far across the tile a shadow is searched for, since a penumbra opens by it.
pub(super) const DISC: f32 = 0.008_726_9;

/// The tile the surfaces are drawn into: nought is the side the light enters by.
pub(super) fn tile(device: &wgpu::Device) -> wgpu::TextureView {
    render_target(
        device,
        "meshlet-shadow",
        (TILE, TILE),
        FORMAT,
        wgpu::TextureUsages::empty(),
    )
}

/// The matrix from the world to the tile for light that comes from `towards`, its shadow fitted
/// to the box of `bounds`: across and down the tile from -1 to 1, and in depth from nought, where
/// the light enters the box's sphere, to one.
pub fn to_tile(towards: Vec3, bounds: (Vec3, Vec3)) -> Mat4 {
    let (centre, radius) = sphere_of(bounds.0, bounds.1);
    let fit = Fit::of(-towards, centre, radius).caster();
    fit.projection * fit.view
}

/// The matrix a frame lit by `sky` casts its shadow with; none where it states no box, or no
/// light to cast one.
pub(super) fn cast_of(sky: &Sky) -> Option<Mat4> {
    let bounds = sky.scene.bounds?;
    let towards = sky.towards.try_normalize()?;
    let casts = sky.sun.max_element() > 0.0 && (bounds.1 - bounds.0).max_element() > 0.0;
    casts.then(|| to_tile(towards, bounds))
}

/// What the resolve reads of the shadow, after the matrix: whether one is cast, how far across
/// the tile it is searched for, the bias, and the penumbra a unit of depth opens. The tile is
/// as deep as it is wide, so a penumbra that opens by the disc's tangent reaches the whole search
/// over the tile's whole depth.
pub(super) fn words(sky: &Sky, casts: bool) -> [f32; 4] {
    let reach = sky.tone.map_or(DISC, |tone| tone.disc);
    [f32::from(u8::from(casts)), reach, BIAS, 1.0]
}

/// How far a surface is held clear of what stands before it towards the light, in the world's units: [`BIAS`] of the tile's depth, which is the diameter of the sphere about the box `sky` states; nought where it states none.
pub(super) fn held_off(sky: &Sky) -> f32 {
    sky.scene.bounds.map_or(0.0, |(min, max)| {
        BIAS * 2.0 * sphere_of(min, max).1.max(1e-3)
    })
}

/// The pass that draws a surface's vertex stage into the tile, and no colour.
pub(super) fn caster(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    vertex: &wgpu::ShaderModule,
    entry: &str,
    compiled: Option<&wgpu::PipelineCache>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("meshlet-shadow"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: vertex,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: None,
        multiview_mask: None,
        cache: compiled,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tile_holds_the_box_with_the_side_the_light_enters_by_at_nought() {
        let bounds = (Vec3::new(-1.0, -2.0, 0.0), Vec3::new(1.0, 2.0, 3.0));
        let towards = Vec3::new(0.3, -0.2, 0.9).normalize();
        let matrix = to_tile(towards, bounds);
        let at = |p: Vec3| matrix.project_point3(p);
        let centre = (bounds.0 + bounds.1) * 0.5;
        assert!((at(centre) - Vec3::new(0.0, 0.0, 0.5)).length() < 1e-5);
        // Towards the light is nearer, and a step across the light's way leaves the depth alone.
        assert!(at(centre + towards).z < 0.5 && at(centre - towards).z > 0.5);
        let across = towards.cross(Vec3::Z).normalize();
        assert!((at(centre + across).z - 0.5).abs() < 1e-5);
        // Every corner of the box is in the tile.
        for corner in 0..8 {
            let pick = |bit: u32, lo: f32, hi: f32| if corner >> bit & 1 == 0 { lo } else { hi };
            let p = at(Vec3::new(
                pick(0, bounds.0.x, bounds.1.x),
                pick(1, bounds.0.y, bounds.1.y),
                pick(2, bounds.0.z, bounds.1.z),
            ));
            assert!(p.abs().max_element() <= 1.0 + 1e-5 && p.z >= -1e-5, "{p:?}");
        }
    }

    #[test]
    fn a_shadow_is_cast_where_a_box_and_a_light_are_stated_and_searched_as_far_as_the_disc_opens() {
        let mut sky = Sky::at(12.0, 0.0);
        assert_eq!(cast_of(&sky), None, "no box");
        sky.scene.bounds = Some((Vec3::ZERO, Vec3::ONE));
        assert!(cast_of(&sky).is_some());
        assert_eq!(words(&sky, true), [1.0, DISC, BIAS, 1.0]);
        assert_eq!(words(&sky, false)[0], 0.0);
        sky.sun = Vec3::ZERO;
        assert_eq!(cast_of(&sky), None, "no light");
    }
}
