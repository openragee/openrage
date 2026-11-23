//! `rpf scan-rdna --backends`: a validated module through each writer wgpu hands it to, Vulkan's
//! SPIR-V, Direct3D 12's HLSL, Metal's MSL and WGSL, with the options the shader tool's own
//! check of a pack writes with.

use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};

use naga::valid::ModuleInfo;

pub(crate) const SPIRV: &str = "SPIR-V (Vulkan)";
pub(crate) const HLSL: &str = "HLSL (Direct3D 12)";
pub(crate) const MSL: &str = "MSL (Metal)";
pub(crate) const WGSL: &str = "WGSL";
/// The order the writers run in: the one known to panic last, so its death costs no other's answer.
pub(crate) const ORDER: [&str; 4] = [SPIRV, MSL, WGSL, HLSL];

/// What the worker is doing to its block, for the line a panic leaves.
static DOING: std::sync::Mutex<&str> = std::sync::Mutex::new("starting");

pub(crate) fn set_doing(what: &'static str) {
    *DOING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = what;
}

pub(crate) fn doing() -> &'static str {
    *DOING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A message with its numbers taken out and cut to a line, so the same fault of two programs counts once.
pub(crate) fn fold(text: &str) -> String {
    let mut out = String::new();
    let mut in_number = false;
    for c in text.lines().next().unwrap_or_default().chars().take(160) {
        if c.is_ascii_digit() {
            if !in_number {
                out.push('N');
            }
            in_number = true;
        } else {
            in_number = false;
            out.push(c);
        }
    }
    out
}

/// What a panic carried, as text.
pub(crate) fn panic_text(panic: &(dyn std::any::Any + Send)) -> String {
    (panic.downcast_ref::<String>().cloned())
        .or_else(|| panic.downcast_ref::<&str>().map(|text| (*text).to_owned()))
        .unwrap_or_else(|| "a panic with no message".to_owned())
}

/// The capabilities every Vulkan device offers wgpu, which a module needs no feature for.
const COMMON: [naga::back::spv::Capability; 11] = {
    use naga::back::spv::Capability as C;
    [
        C::Shader,
        C::Matrix,
        C::Sampled1D,
        C::Image1D,
        C::ImageQuery,
        C::DerivativeControl,
        C::StorageImageExtendedFormats,
        C::SampledCubeArray,
        C::SampleRateShading,
        C::ClipDistance,
        C::StorageImageWriteWithoutFormat,
    ]
};

/// The SPIR-V capabilities the module needs past the common ones.
fn spirv(module: &naga::Module, info: &ModuleInfo) -> Result<Vec<String>, String> {
    let options = naga::back::spv::Options {
        lang_version: (1, 3),
        ..Default::default()
    };
    let mut writer = naga::back::spv::Writer::new(&options).map_err(|e| e.to_string())?;
    (writer.write(module, info, None, &None, &mut Vec::new())).map_err(|e| e.to_string())?;
    Ok((writer.get_capabilities_used().iter())
        .filter(|capability| !COMMON.contains(capability))
        .map(|capability| format!("{capability:?}"))
        .collect())
}

fn hlsl(module: &naga::Module, info: &ModuleInfo) -> Result<(), String> {
    let options = naga::back::hlsl::Options {
        shader_model: naga::back::hlsl::ShaderModel::V5_1,
        fake_missing_bindings: true,
        // wgpu's Direct3D 12 backend gives a first vertex and a first instance this way.
        special_constants_binding: Some(naga::back::hlsl::BindTarget {
            space: 0,
            register: 0,
            binding_array_size: None,
            dynamic_storage_buffer_offsets_index: None,
            restrict_indexing: false,
        }),
        ..Default::default()
    };
    let pipeline = naga::back::hlsl::PipelineOptions::default();
    let mut text = String::new();
    (naga::back::hlsl::Writer::new(&mut text, &options, &pipeline).write(module, info, None))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn msl(module: &naga::Module, info: &ModuleInfo) -> Result<(), String> {
    let options = naga::back::msl::Options {
        lang_version: (3, 0),
        fake_missing_bindings: true,
        ..Default::default()
    };
    let pipeline = naga::back::msl::PipelineOptions::default();
    (naga::back::msl::write_string(module, info, &options, &pipeline))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// `module` as WGSL, which is also what `--program` prints.
pub(crate) fn wgsl(module: &naga::Module, info: &ModuleInfo) -> Result<String, String> {
    naga::back::wgsl::write_string(module, info, naga::back::wgsl::WriterFlags::empty())
        .map_err(|e| e.to_string())
}

/// A writer's answer, a panic of it counted as its refusal.
fn guarded<T>(write: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(write))
        .unwrap_or_else(|panic| Err(format!("the writer panicked: {}", panic_text(&*panic))))
}

/// The writers that refuse `module`, each with why, and the SPIR-V capabilities it needs.
/// `before` hears each writer as it starts, with what the ones before it answered.
pub(crate) fn check(
    module: &naga::Module,
    info: &ModuleInfo,
    mut before: impl FnMut(&str, &BTreeMap<String, String>, &[String]),
) -> (BTreeMap<String, String>, Vec<String>) {
    let mut failures = BTreeMap::new();
    let mut capabilities = Vec::new();
    for backend in ORDER {
        before(backend, &failures, &capabilities);
        let result = match backend {
            SPIRV => {
                set_doing("writing SPIR-V");
                guarded(|| spirv(module, info)).map(|needed| capabilities = needed)
            }
            MSL => {
                set_doing("writing MSL");
                guarded(|| msl(module, info))
            }
            WGSL => {
                set_doing("writing WGSL");
                guarded(|| wgsl(module, info)).map(|_| ())
            }
            _ => {
                set_doing("writing HLSL");
                guarded(|| hlsl(module, info))
            }
        };
        if let Err(why) = result {
            failures.insert(backend.to_owned(), fold(&why));
        }
    }
    (failures, capabilities)
}

/// What `module` asks of a device past the baseline of Vulkan, Direct3D 12 and Metal, read from
/// its types and globals: binding arrays, storage textures, 64-bit scalars, storage in the vertex stage.
pub(crate) fn requires(module: &naga::Module) -> Vec<String> {
    use naga::{AddressSpace, ArraySize, ImageClass, ScalarKind, StorageAccess, TypeInner};
    let mut needs = std::collections::BTreeSet::new();
    let access = |access: StorageAccess| match access.contains(StorageAccess::STORE) {
        true if access.contains(StorageAccess::LOAD) => "read and written",
        true => "written",
        false => "read",
    };
    for (_, ty) in module.types.iter() {
        match &ty.inner {
            TypeInner::BindingArray { base, size } => {
                let of = match module.types[*base].inner {
                    TypeInner::Image { .. } => "textures",
                    TypeInner::Sampler { .. } => "samplers",
                    _ => "buffers",
                };
                needs.insert(match size {
                    ArraySize::Constant(count) => format!("a binding array of {count} {of}"),
                    _ => format!("a binding array of {of} of no stated size"),
                });
            }
            TypeInner::Image {
                class:
                    ImageClass::Storage {
                        format,
                        access: how,
                    },
                ..
            } => {
                needs.insert(format!("a storage texture of {format:?}, {}", access(*how)));
            }
            TypeInner::Scalar(scalar)
            | TypeInner::Atomic(scalar)
            | TypeInner::Vector { scalar, .. }
                if scalar.width == 8 =>
            {
                needs.insert(match scalar.kind {
                    ScalarKind::Float => "64-bit floats".to_owned(),
                    _ => "64-bit integers".to_owned(),
                });
            }
            _ => {}
        }
    }
    let vertex = (module.entry_points.iter()).any(|entry| entry.stage == naga::ShaderStage::Vertex);
    for (_, global) in module.global_variables.iter() {
        if let (true, AddressSpace::Storage { access: how }) = (vertex, global.space) {
            needs.insert(format!(
                "a storage buffer {} in the vertex stage",
                access(how)
            ));
        }
    }
    needs.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_folds_to_its_class() {
        assert_eq!(
            fold("Function [12] 'f' at 340:7\nmore"),
            "Function [N] 'f' at N:N"
        );
        assert_eq!(fold(""), "");
    }

    #[test]
    fn an_empty_module_passes_every_writer() {
        let module = naga::Module::default();
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let mut started = Vec::new();
        let (failures, capabilities) = check(&module, &info, |writer, _, _| {
            started.push(writer.to_owned());
        });
        assert_eq!(failures, BTreeMap::new());
        assert_eq!(capabilities, Vec::<String>::new());
        assert_eq!(started, ORDER);
        assert_eq!(requires(&module), Vec::<String>::new());
    }

    #[test]
    fn a_module_s_types_say_what_it_asks_of_a_device() {
        use naga::{Scalar, Span, Type, TypeInner};
        let mut module = naga::Module::default();
        let mut add = |inner| {
            module
                .types
                .insert(Type { name: None, inner }, Span::UNDEFINED)
        };
        add(TypeInner::Scalar(Scalar::U64));
        let image = add(TypeInner::Image {
            dim: naga::ImageDimension::D2,
            arrayed: false,
            class: naga::ImageClass::Storage {
                format: naga::StorageFormat::Rgba8Unorm,
                access: naga::StorageAccess::STORE,
            },
        });
        add(TypeInner::BindingArray {
            base: image,
            size: naga::ArraySize::Constant(std::num::NonZeroU32::new(8).unwrap()),
        });
        assert_eq!(
            requires(&module),
            [
                "64-bit integers",
                "a binding array of 8 textures",
                "a storage texture of Rgba8Unorm, written"
            ]
        );
    }
}
