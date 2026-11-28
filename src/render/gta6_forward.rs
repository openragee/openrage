//! A GTA VI surface drawn by a forward pass, for the renderer: how its pass is laid over the lit
//! frame, and where it stands.

use or_cli::{Gta6Blend as Blend, Gta6Faces as Faces, Gta6Factor as Factor};
use or_io::gta6::headerless::Geometry;
use or_renderer::game_shaders::meshlets::Forward;

fn factor(factor: Factor) -> wgpu::BlendFactor {
    match factor {
        Factor::Zero => wgpu::BlendFactor::Zero,
        Factor::One => wgpu::BlendFactor::One,
        Factor::OneMinusSourceAlpha => wgpu::BlendFactor::OneMinusSrcAlpha,
        Factor::SecondColour => wgpu::BlendFactor::Src1,
        Factor::SecondAlpha => wgpu::BlendFactor::Src1Alpha,
    }
}

/// The faces a pass that draws `faces` culls, the front being those whose corners run counter-clockwise.
fn culled(faces: Faces) -> wgpu::Face {
    match faces {
        Faces::Front => wgpu::Face::Back,
        Faces::Back => wgpu::Face::Front,
    }
}

/// `blend` as the renderer lays a surface of `geometry` on by it.
pub(super) fn forward_of(blend: &Blend, geometry: &Geometry) -> Forward {
    let summed = |[source, destination]: [Factor; 2]| wgpu::BlendComponent {
        src_factor: factor(source),
        dst_factor: factor(destination),
        operation: wgpu::BlendOperation::Add,
    };
    Forward {
        blend: wgpu::BlendState {
            color: summed(blend.colour),
            alpha: summed(blend.alpha),
        },
        writes: wgpu::ColorWrites::from_bits_truncate(u32::from(blend.writes)),
        cull: Some(culled(blend.faces)),
        centre: centre_of(geometry),
    }
}

/// The middle of the box `geometry`'s positions fill; the origin where none reads.
fn centre_of(geometry: &Geometry) -> glam::Vec3 {
    let positions = geometry.positions.as_deref().unwrap_or_default();
    let corners = positions.iter().fold(None, |corners, at| {
        let at = glam::Vec3::from(*at);
        Some(
            corners.map_or((at, at), |(least, most): (glam::Vec3, glam::Vec3)| {
                (least.min(at), most.max(at))
            }),
        )
    });
    corners.map_or(glam::Vec3::ZERO, |(least, most)| (least + most) / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(positions: Vec<[f32; 3]>) -> Geometry {
        Geometry {
            shader: 0,
            vertex_count: 0,
            channels: Vec::new(),
            triangles: Vec::new(),
            positions: Ok(positions),
            vertices: Default::default(),
            vertex_data: Vec::new(),
            index_data: Vec::new(),
        }
    }

    #[test]
    fn a_forward_surface_is_laid_on_by_its_effect_s_blend_from_where_it_stands() {
        let blend = Blend {
            colour: [Factor::One, Factor::OneMinusSourceAlpha],
            alpha: [Factor::Zero, Factor::One],
            writes: 0b0111,
            faces: Faces::Front,
        };
        let forward = forward_of(&blend, &geometry(vec![[0.0, 0.0, 0.0], [2.0, 4.0, -6.0]]));
        assert_eq!(forward.blend.color.src_factor, wgpu::BlendFactor::One);
        assert_eq!(
            forward.blend.color.dst_factor,
            wgpu::BlendFactor::OneMinusSrcAlpha
        );
        assert_eq!(forward.blend.alpha.src_factor, wgpu::BlendFactor::Zero);
        assert_eq!(forward.blend.alpha.dst_factor, wgpu::BlendFactor::One);
        assert_eq!(forward.writes, wgpu::ColorWrites::COLOR);
        assert_eq!(forward.cull, Some(wgpu::Face::Back));
        assert_eq!(forward.centre, glam::Vec3::new(1.0, 2.0, -3.0));
        assert_eq!(
            forward_of(&blend, &geometry(Vec::new())).centre,
            glam::Vec3::ZERO
        );
    }

    #[test]
    fn a_lit_surface_is_laid_on_by_its_second_colour_and_a_glass_s_inside_culls_its_front() {
        let lit = Blend {
            colour: [Factor::One, Factor::SecondColour],
            alpha: [Factor::One, Factor::SecondAlpha],
            writes: 0b1111,
            faces: Faces::Back,
        };
        let forward = forward_of(&lit, &geometry(Vec::new()));
        assert_eq!(forward.blend.color.dst_factor, wgpu::BlendFactor::Src1);
        assert_eq!(forward.blend.alpha.dst_factor, wgpu::BlendFactor::Src1Alpha);
        assert_eq!(forward.writes, wgpu::ColorWrites::ALL);
        assert_eq!(forward.cull, Some(wgpu::Face::Front));
    }
}
