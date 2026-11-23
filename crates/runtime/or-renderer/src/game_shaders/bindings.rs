//! What a module asks its bind groups to hold, read from the module itself: a pipeline's layout is made from this and so cannot disagree with the shader.

use std::num::NonZeroU32;

use thiserror::Error;

use naga::{AddressSpace, ImageClass, ImageDimension, ScalarKind, StorageAccess, TypeInner};

/// One thing a shader reads or writes through a binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resource {
    Uniform {
        size: u64,
    },
    Storage {
        size: u64,
        read_only: bool,
    },
    Texture {
        dimension: wgpu::TextureViewDimension,
        sample: wgpu::TextureSampleType,
        multisampled: bool,
    },
    StorageTexture {
        dimension: wgpu::TextureViewDimension,
        format: wgpu::TextureFormat,
        access: wgpu::StorageTextureAccess,
    },
    Sampler {
        comparison: bool,
    },
}

/// A resource at a group and binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    pub group: u32,
    pub binding: u32,
    pub resource: Resource,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BindingError {
    #[error("a binding array")]
    BindingArray,
    #[error("a storage texture format with no format on the device")]
    StorageFormat,
    #[error("a resource of a kind this does not take")]
    Unsupported,
}

fn dimension(dim: ImageDimension, arrayed: bool) -> wgpu::TextureViewDimension {
    use wgpu::TextureViewDimension as D;
    match (dim, arrayed) {
        (ImageDimension::D1, _) => D::D1,
        (ImageDimension::D2, false) => D::D2,
        (ImageDimension::D2, true) => D::D2Array,
        (ImageDimension::D3, _) => D::D3,
        (ImageDimension::Cube, false) => D::Cube,
        (ImageDimension::Cube, true) => D::CubeArray,
    }
}

fn storage_format(format: naga::StorageFormat) -> Option<wgpu::TextureFormat> {
    use naga::StorageFormat as S;
    use wgpu::TextureFormat as F;
    Some(match format {
        S::R8Unorm => F::R8Unorm,
        S::R8Snorm => F::R8Snorm,
        S::R8Uint => F::R8Uint,
        S::R8Sint => F::R8Sint,
        S::R16Uint => F::R16Uint,
        S::R16Sint => F::R16Sint,
        S::R16Float => F::R16Float,
        S::Rg8Unorm => F::Rg8Unorm,
        S::Rg8Snorm => F::Rg8Snorm,
        S::Rg8Uint => F::Rg8Uint,
        S::Rg8Sint => F::Rg8Sint,
        S::R32Uint => F::R32Uint,
        S::R32Sint => F::R32Sint,
        S::R32Float => F::R32Float,
        S::Rg16Uint => F::Rg16Uint,
        S::Rg16Sint => F::Rg16Sint,
        S::Rg16Float => F::Rg16Float,
        S::Rgba8Unorm => F::Rgba8Unorm,
        S::Rgba8Snorm => F::Rgba8Snorm,
        S::Rgba8Uint => F::Rgba8Uint,
        S::Rgba8Sint => F::Rgba8Sint,
        S::Rg32Uint => F::Rg32Uint,
        S::Rg32Sint => F::Rg32Sint,
        S::Rg32Float => F::Rg32Float,
        S::Rgba16Uint => F::Rgba16Uint,
        S::Rgba16Sint => F::Rgba16Sint,
        S::Rgba16Float => F::Rgba16Float,
        S::Rgba32Uint => F::Rgba32Uint,
        S::Rgba32Sint => F::Rgba32Sint,
        S::Rgba32Float => F::Rgba32Float,
        _ => return None,
    })
}

fn storage_access(access: StorageAccess) -> wgpu::StorageTextureAccess {
    match (
        access.contains(StorageAccess::LOAD),
        access.contains(StorageAccess::STORE),
    ) {
        (true, true) => wgpu::StorageTextureAccess::ReadWrite,
        (false, true) => wgpu::StorageTextureAccess::WriteOnly,
        _ => wgpu::StorageTextureAccess::ReadOnly,
    }
}

/// What a variable of type `inner` in address space `space` binds.
fn resource_of(
    module: &naga::Module,
    space: &AddressSpace,
    inner: &TypeInner,
) -> Result<Resource, BindingError> {
    let size = || u64::from(inner.size(module.to_ctx()));
    Ok(match (space, inner) {
        (AddressSpace::Uniform, _) => Resource::Uniform { size: size() },
        (AddressSpace::Storage { access }, _) => Resource::Storage {
            size: size(),
            read_only: !access.contains(StorageAccess::STORE),
        },
        (AddressSpace::Handle, TypeInner::Sampler { comparison }) => Resource::Sampler {
            comparison: *comparison,
        },
        (
            AddressSpace::Handle,
            TypeInner::Image {
                dim,
                arrayed,
                class,
            },
        ) => {
            let view = dimension(*dim, *arrayed);
            match class {
                ImageClass::Sampled { kind, multi } => Resource::Texture {
                    dimension: view,
                    sample: match kind {
                        ScalarKind::Sint => wgpu::TextureSampleType::Sint,
                        ScalarKind::Uint => wgpu::TextureSampleType::Uint,
                        _ => wgpu::TextureSampleType::Float { filterable: !multi },
                    },
                    multisampled: *multi,
                },
                ImageClass::Depth { multi } => Resource::Texture {
                    dimension: view,
                    sample: wgpu::TextureSampleType::Depth,
                    multisampled: *multi,
                },
                ImageClass::Storage { format, access } => Resource::StorageTexture {
                    dimension: view,
                    format: storage_format(*format).ok_or(BindingError::StorageFormat)?,
                    access: storage_access(*access),
                },
                _ => return Err(BindingError::Unsupported),
            }
        }
        (_, TypeInner::BindingArray { .. }) => return Err(BindingError::BindingArray),
        _ => return Err(BindingError::Unsupported),
    })
}

/// Every resource `module` binds, by group and binding, with the entries of each that is a
/// binding array of a constant length.
pub fn arrayed(
    module: &naga::Module,
) -> Result<Vec<(Requirement, Option<NonZeroU32>)>, BindingError> {
    let mut found = Vec::new();
    for (_, variable) in module.global_variables.iter() {
        let Some(binding) = &variable.binding else {
            continue;
        };
        let (inner, count) = match &module.types[variable.ty].inner {
            TypeInner::BindingArray {
                base,
                size: naga::ArraySize::Constant(count),
            } => (&module.types[*base].inner, Some(*count)),
            inner => (inner, None),
        };
        found.push((
            Requirement {
                group: binding.group,
                binding: binding.binding,
                resource: resource_of(module, &variable.space, inner)?,
            },
            count,
        ));
    }
    found.sort_by_key(|(r, _)| (r.group, r.binding));
    Ok(found)
}

/// Every resource `module` binds, by group and binding.
pub fn requirements(module: &naga::Module) -> Result<Vec<Requirement>, BindingError> {
    arrayed(module)?
        .into_iter()
        .map(|(requirement, count)| match count {
            None => Ok(requirement),
            Some(_) => Err(BindingError::BindingArray),
        })
        .collect()
}

impl Resource {
    /// The layout entry that holds this resource at `binding` for the stages in `visibility`.
    pub fn layout_entry(
        &self,
        binding: u32,
        visibility: wgpu::ShaderStages,
    ) -> wgpu::BindGroupLayoutEntry {
        let ty = match self {
            Resource::Uniform { .. } => wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            Resource::Storage { read_only, .. } => wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage {
                    read_only: *read_only,
                },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            Resource::Texture {
                dimension,
                sample,
                multisampled,
            } => wgpu::BindingType::Texture {
                sample_type: *sample,
                view_dimension: *dimension,
                multisampled: *multisampled,
            },
            Resource::StorageTexture {
                dimension,
                format,
                access,
            } => wgpu::BindingType::StorageTexture {
                access: *access,
                format: *format,
                view_dimension: *dimension,
            },
            Resource::Sampler { comparison } => wgpu::BindingType::Sampler(if *comparison {
                wgpu::SamplerBindingType::Comparison
            } else {
                wgpu::SamplerBindingType::Filtering
            }),
        };
        wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty,
            count: None,
        }
    }
}

#[cfg(test)]
mod tests;
