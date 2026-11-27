//! The device's own ray query under the game's walk: whether a device offers one, its structures
//! built from the arena's own triangles and instances in the numbers a walk answered through the
//! query reads, the arena that walk is given, and the binding of the top level.
//!
//! An instance is the device's slot `i` where it is the `i`th instance node of the top level, and
//! a bottom level is one geometry whose triangle `2 * unit + place` is the one the arena holds at
//! that unit, a triangle of no size where the unit holds none: so a candidate's numbers lead to
//! its nodes with no table.

use std::collections::BTreeMap;

use or_io::gta6::accel::layout::{
    offset, INSTANCE_BOTTOM, INSTANCE_NO_ROOT, INSTANCE_OBJECT_TO_WORLD, INSTANCE_ROOT, QUERY_TOP,
};
use or_io::gta6::accel::{device_triangles, instance_nodes, word, Arena, Structure};
use wgpu::util::DeviceExt;

/// The feature a module whose test of a node asks the device's query needs of its device.
pub const FEATURES: wgpu::Features = wgpu::Features::EXPERIMENTAL_RAY_QUERY;
/// Every instance whatever its mask: the walk tests the mask itself.
const EVERY_MASK: u8 = 0xFF;
/// The bytes of a corner: three floats.
const CORNER: u64 = 12;

/// Why the device's structures were not built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum QueryError {
    #[error("the device was not opened with ray queries")]
    NotOffered,
    #[error("the top level holds no instance")]
    Empty,
    #[error("instance {0} of the top level places no bottom level")]
    NoBottom(usize),
}

/// Whether a device of `backend` with `features` runs a module whose test of a node asks its query: Vulkan's and Direct3D 12's, not Metal's, whose looped proceed wgpu 29 never ends.
pub fn offered(backend: wgpu::Backend, features: wgpu::Features) -> bool {
    matches!(backend, wgpu::Backend::Vulkan | wgpu::Backend::Dx12) && features.contains(FEATURES)
}

/// What a device is asked for where `adapter` [`offered`] its query: the feature, the token experimental features take, and `limits` raised to the adapter's for structures; nothing otherwise, so a plain device is opened.
pub fn device_needs(
    adapter: &wgpu::Adapter,
    limits: &mut wgpu::Limits,
) -> (wgpu::Features, wgpu::ExperimentalFeatures) {
    if !offered(adapter.get_info().backend, adapter.features()) {
        return (
            wgpu::Features::empty(),
            wgpu::ExperimentalFeatures::default(),
        );
    }
    *limits = limits
        .clone()
        .using_acceleration_structure_values(adapter.limits());
    // SAFETY: the one experimental feature asked for is the ray query, whose structures are built
    // here from bytes checked by the arena's builder, and whose modules naga validates with its
    // initialisation tracking left on.
    (FEATURES, unsafe { wgpu::ExperimentalFeatures::enabled() })
}

/// Whether a device opened from an adapter of `backend` with `features` is asked for the query: only where the aid asks for it (`aided`) and the adapter [`offered`] it.
pub fn asked(aided: bool, backend: wgpu::Backend, features: wgpu::Features) -> bool {
    aided && offered(backend, features)
}

/// [`device_needs`] where [`super::query_aid`] asks for the query; nothing otherwise, so a session's device is opened exactly as it is without the aid.
pub fn aided_device_needs(
    adapter: &wgpu::Adapter,
    limits: &mut wgpu::Limits,
) -> (wgpu::Features, wgpu::ExperimentalFeatures) {
    if !asked(
        super::query_aid(),
        adapter.get_info().backend,
        adapter.features(),
    ) {
        return (
            wgpu::Features::empty(),
            wgpu::ExperimentalFeatures::default(),
        );
    }
    device_needs(adapter, limits)
}

/// The arena a walk answered through the device's query is given: `arena` with the first of `top`'s two query boxes stated as its root and `top`'s address at [`QUERY_TOP`]; `None` where `top` is no top level of it.
// ponytail: a copy of the whole arena; a section's arena would state the root in place.
pub fn asked_words(arena: &Arena, top: &Structure) -> Option<Vec<u32>> {
    let mut asking = arena.clone();
    if !asking.state_query_root(top, true) {
        return None;
    }
    let mut words = asking.words().to_vec();
    *words.get_mut((QUERY_TOP / 4) as usize)? = top.base;
    Some(words)
}

/// The layout entry of the device's top level at `binding`, read by a compute stage.
pub fn layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::AccelerationStructure {
            vertex_return: false,
        },
        count: None,
    }
}

/// The device's structures of one top level of an arena; each of its instances holds its bottom level.
pub struct Structures {
    pub tlas: wgpu::Tlas,
}

impl Structures {
    /// The structures of the top level at byte `top` of `words`, built by one submission to `queue`.
    ///
    /// # Errors
    ///
    /// Where `device` has no ray query, or the top level holds no instance or one that places no
    /// bottom level.
    pub fn build(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        words: &[u32],
        top: u32,
    ) -> Result<Self, QueryError> {
        // ponytail: wgpu refits no structure, so an arena refitted for a pose is built again
        // whole; a top level alone where only instances move, once a section needs it.
        if !device.features().contains(FEATURES) {
            return Err(QueryError::NotOffered);
        }
        let nodes = instance_nodes(words, top);
        if nodes.is_empty() {
            return Err(QueryError::Empty);
        }
        // Each bottom level once, by its address, and each instance's level and transform.
        let (mut levels, mut bottoms) = (BTreeMap::new(), Vec::new());
        let mut placed = Vec::with_capacity(nodes.len());
        for (number, node) in nodes.iter().enumerate() {
            let at = top.wrapping_add(offset(*node));
            if word(words, at, INSTANCE_ROOT) & INSTANCE_NO_ROOT == INSTANCE_NO_ROOT {
                return Err(QueryError::NoBottom(number));
            }
            let bottom = word(words, at, INSTANCE_BOTTOM) << 8;
            let level = *levels.entry(bottom).or_insert_with(|| {
                bottoms.push(bottom);
                bottoms.len() - 1
            });
            // Three rows of four floats, as a device's instance takes them.
            let transform: [f32; 12] = std::array::from_fn(|k| {
                f32::from_bits(word(words, at, INSTANCE_OBJECT_TO_WORLD + 4 * k as u32))
            });
            placed.push((level, transform));
        }

        let mut sizes = Vec::with_capacity(bottoms.len());
        let mut buffers = Vec::with_capacity(bottoms.len());
        for bottom in &bottoms {
            let corners: Vec<f32> = (device_triangles(words, *bottom).iter())
                .flat_map(|triangle| triangle.iter().flatten().copied())
                .collect();
            sizes.push(wgpu::BlasTriangleGeometrySizeDescriptor {
                vertex_format: wgpu::VertexFormat::Float32x3,
                vertex_count: (corners.len() / 3) as u32,
                index_format: None,
                index_count: None,
                // A triangle is handed over once: the walk would see a candidate twice otherwise.
                flags: wgpu::AccelerationStructureGeometryFlags::NO_DUPLICATE_ANY_HIT_INVOCATION,
            });
            buffers.push(
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("query-corners"),
                    contents: bytemuck::cast_slice(&corners),
                    usage: wgpu::BufferUsages::BLAS_INPUT,
                }),
            );
        }
        let blases: Vec<wgpu::Blas> = (sizes.iter())
            .map(|size| {
                device.create_blas(
                    &wgpu::CreateBlasDescriptor {
                        label: Some("query-bottom"),
                        flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
                        update_mode: wgpu::AccelerationStructureUpdateMode::Build,
                    },
                    wgpu::BlasGeometrySizeDescriptors::Triangles {
                        descriptors: vec![size.clone()],
                    },
                )
            })
            .collect();
        let mut tlas = device.create_tlas(&wgpu::CreateTlasDescriptor {
            label: Some("query-top"),
            max_instances: placed.len() as u32,
            flags: wgpu::AccelerationStructureFlags::PREFER_FAST_TRACE,
            update_mode: wgpu::AccelerationStructureUpdateMode::Build,
        });
        for (slot, (level, transform)) in placed.into_iter().enumerate() {
            tlas[slot] = Some(wgpu::TlasInstance::new(
                &blases[level],
                transform,
                0,
                EVERY_MASK,
            ));
        }
        let entries: Vec<wgpu::BlasBuildEntry> = (blases.iter().zip(&sizes).zip(&buffers))
            .map(|((blas, size), buffer)| wgpu::BlasBuildEntry {
                blas,
                geometry: wgpu::BlasGeometries::TriangleGeometries(vec![
                    wgpu::BlasTriangleGeometry {
                        size,
                        vertex_buffer: buffer,
                        first_vertex: 0,
                        vertex_stride: CORNER,
                        index_buffer: None,
                        first_index: None,
                        transform_buffer: None,
                        transform_buffer_offset: None,
                    },
                ]),
            })
            .collect();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("query-structures"),
        });
        encoder.build_acceleration_structures(entries.iter(), [&tlas]);
        queue.submit([encoder.finish()]);
        Ok(Self { tlas })
    }

    /// The bind group entry of the top level at `binding`.
    pub fn entry(&self, binding: u32) -> wgpu::BindGroupEntry<'_> {
        wgpu::BindGroupEntry {
            binding,
            resource: self.tlas.as_binding(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::gta6::accel::{stated_boxes, Instance, Triangle};

    #[test]
    fn a_query_is_offered_on_vulkan_and_direct3d_12_with_the_feature_and_nowhere_else() {
        use wgpu::Backend::{Dx12, Metal, Vulkan};
        let none = wgpu::Features::empty();
        assert!(offered(Vulkan, FEATURES));
        assert!(offered(
            Dx12,
            FEATURES | wgpu::Features::TEXTURE_COMPRESSION_BC
        ));
        assert!(!offered(Metal, FEATURES));
        assert!(!offered(Vulkan, none) && !offered(Dx12, none));
    }

    #[test]
    fn a_device_is_asked_for_the_query_only_under_the_aid_and_where_its_adapter_offers_it() {
        use wgpu::Backend::{Dx12, Metal, Vulkan};
        let made_up = [
            wgpu::Features::empty(),
            FEATURES,
            FEATURES | wgpu::Features::TEXTURE_COMPRESSION_BC,
            wgpu::Features::all(),
        ];
        // Without the aid nothing is asked, whatever the adapter has: the device is today's.
        for features in made_up {
            for backend in [Vulkan, Dx12, Metal] {
                assert!(!asked(false, backend, features), "{backend:?} {features:?}");
            }
        }
        assert!(asked(true, Vulkan, FEATURES) && asked(true, Dx12, wgpu::Features::all()));
        assert!(!asked(true, Metal, FEATURES) && !asked(true, Vulkan, wgpu::Features::empty()));
        // The aid's spellings: `traced` asks; `no-query` and `plain-device` keep body S.
        use super::super::query_of;
        assert!(query_of("traced") && query_of("target=3, traced=C:/w.walk"));
        assert!(!query_of("traced,no-query") && !query_of("plain-device, traced"));
        assert!(!query_of("no-query") && !query_of(""));
    }

    #[test]
    fn a_walk_through_the_query_is_given_the_arena_with_its_boxes_stated_and_its_top_level_named() {
        let mut arena = Arena::new();
        let triangle = Triangle {
            corners: [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            geometry: 0,
            primitive: 0,
            opaque: true,
        };
        let bottom = arena.bottom_level(&[triangle]).unwrap();
        let instance = Instance {
            bottom,
            object_to_world: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 2.0],
            ],
            id: 0,
            mask: 0xFF,
            flags: 0,
            contribution: 0,
        };
        let top = arena.top_level(&[instance]).unwrap();
        let words = asked_words(&arena, &top).expect("a top level");
        assert_eq!(stated_boxes(arena.words(), top.base), None);
        assert!(stated_boxes(&words, top.base).is_some());
        assert_eq!(words[(QUERY_TOP / 4) as usize], top.base);
        // Nothing else of the arena moves: the header's root and the dword at the top's address.
        let moved = (arena.words().iter().zip(&words))
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(moved, 2);
        // A bottom level is no top level.
        assert_eq!(asked_words(&arena, &bottom), None);
        // The device is given one instance, of the bottom level the arena places.
        let nodes = instance_nodes(&words, top.base);
        assert_eq!(nodes.len(), 1);
        let at = top.base + offset(nodes[0]);
        assert_eq!(word(&words, at, INSTANCE_BOTTOM) << 8, bottom.base);
    }
}
