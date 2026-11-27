//! The surfaces of a GTA VI geometry whose mesh stage places its instances by records: one surface
//! an instance, since a lowered mesh stage draws its meshlets as the draw's instances.

use or_cli::{Gta6Feed as Filled, Gta6InstanceRecord, Gta6ShadingPass, GTA6_IN_PLACE};
use or_renderer::game_shaders::meshlets::{Feed, MeshletSurface};

/// The records a binding of them holds: one instance's, repeated, so that a stage reads that
/// instance's record at any number it picks one by below this.
const RECORDS: usize = 256;

/// Where the model view places the one instance of a model opened alone.
pub(crate) const IN_PLACE: [[f32; 12]; 1] = [GTA6_IN_PLACE];

/// The bytes of a binding of records for the instance placed by `placement`, a row-major 3x4 matrix.
pub(crate) fn records(placement: [f32; 12]) -> Vec<u8> {
    Gta6InstanceRecord::at_rest(placement)
        .bytes()
        .repeat(RECORDS)
}

/// `surface`, drawn by `pass`, once for each of `placements` where its mesh stage reads records of
/// instances, and as it is where it reads none.
pub(crate) fn placed(
    pass: &Gta6ShadingPass,
    surface: MeshletSurface,
    placements: &[[f32; 12]],
) -> Vec<MeshletSurface> {
    let fed: Vec<_> = (pass.mesh.feeds.iter())
        .filter(|fed| fed.feed == Filled::InstanceRecords)
        .collect();
    if fed.is_empty() {
        return vec![surface];
    }
    (placements.iter())
        .map(|placement| {
            let mut surface = surface.clone();
            for fed in &fed {
                let bytes = super::gta6_meshlets::headed(fed.header, &records(*placement));
                surface
                    .feeds
                    .insert((fed.group, fed.binding), Feed::Bytes(bytes));
            }
            surface
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use or_cli::{Gta6Fed as Fed, Gta6Stage as Stage};

    use super::*;

    fn stage(feeds: Vec<Fed>) -> Stage {
        Stage {
            approximations: Default::default(),
            module: Arc::default(),
            feeds,
            textures: Vec::new(),
            streams: Default::default(),
            rows: Vec::new(),
        }
    }

    fn surface() -> MeshletSurface {
        MeshletSurface {
            vertex: Arc::default(),
            fragment: Arc::default(),
            feeds: BTreeMap::new(),
            meshlets: 2,
            primitives: 5,
            forward: None,
        }
    }

    #[test]
    fn three_placed_instances_are_three_surfaces_each_with_its_own_record() {
        let records = Fed {
            group: 0,
            binding: 3,
            feed: Filled::InstanceRecords,
            header: Some(128),
        };
        let pass = Gta6ShadingPass {
            mesh: stage(vec![records]),
            pixel: stage(Vec::new()),
            linked: Default::default(),
            forward: None,
        };
        let at = |x: f32| {
            let mut placement = GTA6_IN_PLACE;
            placement[3] = x;
            placement
        };
        let drawn = placed(&pass, surface(), &[at(1.0), at(2.0), at(3.0)]);
        assert_eq!(drawn.len(), 3);
        for (surface, x) in drawn.iter().zip([1.0f32, 2.0, 3.0]) {
            let Some(Feed::Bytes(bytes)) = surface.feeds.get(&(0, 3)) else {
                panic!("the records are not bound");
            };
            // The header, then the record: its first row's fourth float is the place along x.
            assert_eq!(bytes.len(), 16 + 128 * RECORDS);
            assert_eq!(bytes[16 + 12..16 + 16], x.to_le_bytes());
            assert_eq!((surface.meshlets, surface.primitives), (2, 5));
        }
        // A stage that reads no record draws once, whatever is placed.
        let plain = Gta6ShadingPass {
            mesh: stage(Vec::new()),
            ..pass
        };
        assert_eq!(placed(&plain, surface(), &[at(1.0), at(2.0)]).len(), 1);
    }
}
