//! What every wgpu instance OpenRage opens is given: Vulkan, Direct3D 12 or Metal, never OpenGL, and
//! DXC as Direct3D 12's shader compiler where a `dxcompiler.dll` is found.

use std::path::{Path, PathBuf};

/// The backends OpenRage draws through. OpenGL is not one: it indexes no table of textures at a
/// computed place and updates no texture in a step, which a game's shading passes do.
pub const BACKENDS: wgpu::Backends = wgpu::Backends::PRIMARY;

/// What a person is told where a machine or `WGPU_BACKEND` offers none of [`BACKENDS`].
pub const UNSUPPORTED: &str =
    "OpenRage draws through Vulkan, Direct3D 12 or Metal; OpenGL is not supported";

/// [`BACKENDS`], or those of them `WGPU_BACKEND` names.
///
/// # Errors
/// [`UNSUPPORTED`], where the variable names none of them.
pub fn backends_from_env() -> Result<wgpu::Backends, &'static str> {
    supported(wgpu::Backends::from_env())
}

fn supported(named: Option<wgpu::Backends>) -> Result<wgpu::Backends, &'static str> {
    match named {
        None => Ok(BACKENDS),
        Some(named) if named.intersects(BACKENDS) => Ok(named & BACKENDS),
        Some(_) => Err(UNSUPPORTED),
    }
}

const DXC: &str = "dxcompiler.dll";
/// What a compiler older than 1.8.2502 signs its output with, loaded by name: without it
/// Direct3D 12 opens no device on that compiler.
const DXIL: &str = "dxil.dll";

/// The newest `dxcompiler.dll` a Windows SDK under `program_files` holds for this machine's
/// kind, with the `dxil.dll` an SDK's compiler needs beside it.
fn sdk_dxc(program_files: &Path) -> Option<PathBuf> {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        _ => "x64",
    };
    let version = |dir: &PathBuf| -> Vec<u32> {
        let name = dir.file_name().map(|name| name.to_string_lossy());
        (name.iter().flat_map(|name| name.split('.')))
            .map(|part| part.parse().unwrap_or(0))
            .collect()
    };
    let bin = program_files.join("Windows Kits").join("10").join("bin");
    let mut versions: Vec<PathBuf> = (std::fs::read_dir(bin).ok()?)
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|dir| {
            [DXC, DXIL]
                .iter()
                .all(|file| dir.join(arch).join(file).is_file())
        })
        .collect();
    versions.sort_by_key(version);
    Some(versions.pop()?.join(arch).join(DXC))
}

/// The `dxcompiler.dll` Direct3D 12 compiles with: the one beside the program, else the first on
/// `PATH`, else the newest Windows SDK's.
pub fn find_dxc() -> Option<PathBuf> {
    let beside = (std::env::current_exe().ok())
        .and_then(|program| Some(program.parent()?.join(DXC)))
        .filter(|file| file.is_file());
    let on_path = || {
        let path = std::env::var_os("PATH")?;
        let found = std::env::split_paths(&path)
            .map(|dir| dir.join(DXC))
            .find(|file| file.is_file());
        found
    };
    let in_sdk = || {
        ["ProgramFiles(x86)", "ProgramFiles"]
            .iter()
            .filter_map(std::env::var_os)
            .find_map(|root| sdk_dxc(Path::new(&root)))
    };
    beside.or_else(on_path).or_else(in_sdk)
}

/// Puts the folder of `dxc` on `PATH` where it holds a `dxil.dll`, which the compiler loads by name.
fn expose_dxil(dxc: &Path) {
    let Some(dir) = dxc.parent().filter(|dir| dir.join(DXIL).is_file()) else {
        return;
    };
    let path = std::env::var_os("PATH").unwrap_or_default();
    if std::env::split_paths(&path).any(|held| held == dir) {
        return;
    }
    let dirs = std::iter::once(dir.to_path_buf()).chain(std::env::split_paths(&path));
    if let Ok(joined) = std::env::join_paths(dirs) {
        std::env::set_var("PATH", joined);
    }
}

/// The compiler Direct3D 12 is given: the one `WGPU_DX12_COMPILER` names, else DXC where
/// [`find_dxc`] finds it, else FXC, which refuses shaders that are valid (a gradient inside a
/// loop, a barrier in a branch) and has no 64-bit integers.
pub fn dx12_compiler() -> wgpu::Dx12Compiler {
    if !cfg!(windows) {
        return wgpu::Dx12Compiler::default();
    }
    if let Some(named) = wgpu::Dx12Compiler::from_env() {
        return named;
    }
    match find_dxc() {
        Some(file) => {
            expose_dxil(&file);
            tracing::info!("Direct3D 12 compiles shaders with DXC: {}", file.display());
            wgpu::Dx12Compiler::DynamicDxc {
                dxc_path: file.to_string_lossy().into_owned(),
            }
        }
        None => {
            tracing::warn!(
                "Direct3D 12 compiles shaders with FXC, which refuses some of a game's own: no \
                 {DXC} beside the program, on PATH or in a Windows SDK. Put {DXC} and dxil.dll \
                 beside the program, or set WGPU_BACKEND=vulkan"
            );
            wgpu::Dx12Compiler::Fxc
        }
    }
}

/// `flags` as an instance takes them: without `DEBUG` where the software adapter is asked for.
// ⚠ `DEBUG`, which a build with debug assertions sets, has Direct3D 12's compiler leave a shader
// unoptimised, and the software adapter then takes about four times as long for twice the code:
// a skinned stage it runs in 10 to 20 s optimised did not finish in 150 s. Validation stays.
fn flags_for(flags: wgpu::InstanceFlags, software: bool) -> wgpu::InstanceFlags {
    match software {
        true => flags.difference(wgpu::InstanceFlags::DEBUG),
        false => flags,
    }
}

/// Whether a device of `backend` compiles with FXC for want of a compiler: none was `named` by
/// `WGPU_DX12_COMPILER` and no `dxcompiler.dll` was `found`.
fn lacks_dxc(backend: wgpu::Backend, named: bool, found: bool) -> bool {
    backend == wgpu::Backend::Dx12 && !named && !found
}

/// Whether a device of `backend` compiles with FXC because [`find_dxc`] found no DXC, which the
/// app tells a person when it starts: [`dx12_compiler`] only logs it.
pub fn fell_back_to_fxc(backend: wgpu::Backend) -> bool {
    let named = wgpu::Dx12Compiler::from_env().is_some();
    lacks_dxc(backend, named, find_dxc().is_some())
}

/// `descriptor` with [`dx12_compiler`] as Direct3D 12's compiler, and optimised shaders where
/// [`SOFTWARE_ADAPTER`] is set.
pub fn with_dxc(mut descriptor: wgpu::InstanceDescriptor) -> wgpu::InstanceDescriptor {
    descriptor.backend_options.dx12.shader_compiler = dx12_compiler();
    descriptor.flags = flags_for(descriptor.flags, software_asked());
    descriptor
}

/// Whether `adapter` updates a texture of 64-bit texels in a step. Direct3D 12 offers
/// `TEXTURE_INT64_ATOMIC` and then refuses the layout of such a texture, so the feature alone
/// does not say.
pub fn updates_64_bit_textures(adapter: &wgpu::Adapter) -> bool {
    adapter
        .features()
        .contains(wgpu::Features::TEXTURE_INT64_ATOMIC)
        && (adapter
            .get_texture_format_features(wgpu::TextureFormat::R64Uint)
            .flags)
            .contains(wgpu::TextureFormatFeatureFlags::STORAGE_ATOMIC)
}

/// The variable that, set to anything but `0`, sends every device opened with no surface to the
/// machine's software adapter: a frame a hardware adapter might never finish is drawn there first.
pub const SOFTWARE_ADAPTER: &str = "OPENRAGE_SOFTWARE_ADAPTER";

/// Whether [`SOFTWARE_ADAPTER`] asks for the software adapter.
pub fn software_asked() -> bool {
    std::env::var_os(SOFTWARE_ADAPTER).is_some_and(|value| !value.is_empty() && value != "0")
}

/// Refuses an adapter of `kind` named `name` where the software one was asked for and it is not a CPU's.
fn software_or_not_asked(software: bool, kind: wgpu::DeviceType, name: &str) -> Result<(), String> {
    match software && kind != wgpu::DeviceType::Cpu {
        true => Err(format!(
            "{SOFTWARE_ADAPTER} is set and {name} is no software adapter ({kind:?})"
        )),
        false => Ok(()),
    }
}

/// The adapter a device with no surface is opened on: the machine's fastest, or its software one
/// where [`SOFTWARE_ADAPTER`] is set.
///
/// # Errors
/// Where the machine has none, or the software one was asked for and what it offers is no CPU's.
pub fn headless_adapter(instance: &wgpu::Instance) -> Result<wgpu::Adapter, String> {
    let software = software_asked();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: software,
    }))
    .map_err(|e| format!("no GPU adapter without a surface: {e}"))?;
    let info = adapter.get_info();
    software_or_not_asked(software, info.device_type, &info.name)?;
    Ok(adapter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_software_adapter_is_handed_optimised_shaders_and_no_other_is() {
        use wgpu::InstanceFlags as Flags;
        let debug = Flags::DEBUG | Flags::VALIDATION;
        assert_eq!(flags_for(debug, true), Flags::VALIDATION);
        assert_eq!(flags_for(debug, false), debug);
        assert_eq!(flags_for(Flags::empty(), true), Flags::empty());
    }

    #[test]
    fn only_a_cpu_adapter_passes_where_the_software_one_is_asked_for() {
        use wgpu::DeviceType::{Cpu, DiscreteGpu};
        assert!(software_or_not_asked(true, Cpu, "a").is_ok());
        assert!(software_or_not_asked(true, DiscreteGpu, "a").is_err());
        assert!(software_or_not_asked(false, DiscreteGpu, "a").is_ok());
    }

    #[test]
    fn fxc_is_a_fallback_only_under_direct3d_12_with_no_compiler_named_or_found() {
        use wgpu::Backend::{Dx12, Vulkan};
        assert!(lacks_dxc(Dx12, false, false));
        assert!(!lacks_dxc(Vulkan, false, false));
        // FXC that was asked for is no fallback, and a compiler found is used.
        assert!(!lacks_dxc(Dx12, true, false));
        assert!(!lacks_dxc(Dx12, false, true));
    }

    #[test]
    fn a_backend_named_that_is_not_supported_is_refused_and_a_supported_one_is_kept() {
        assert_eq!(supported(None), Ok(BACKENDS));
        assert_eq!(supported(Some(wgpu::Backends::GL)), Err(UNSUPPORTED));
        let both = wgpu::Backends::GL | wgpu::Backends::VULKAN;
        assert_eq!(supported(Some(both)), Ok(wgpu::Backends::VULKAN));
    }

    #[test]
    fn the_newest_sdk_that_holds_a_compiler_is_the_one_found() {
        let held_root = tempfile::tempdir().unwrap();
        let root = held_root.path().join("sdk");
        let arch = match std::env::consts::ARCH {
            "aarch64" => "arm64",
            _ => "x64",
        };
        let bin = root.join("Windows Kits").join("10").join("bin");
        // 10.0.9 sorts after 10.0.10 as text, and the newest directory holds no compiler.
        for (version, held) in [
            ("10.0.9.0", true),
            ("10.0.10.0", true),
            ("10.0.11.0", false),
        ] {
            let dir = bin.join(version).join(arch);
            std::fs::create_dir_all(&dir).unwrap();
            if held {
                std::fs::write(dir.join(DXC), b"").unwrap();
                std::fs::write(dir.join(DXIL), b"").unwrap();
            }
        }
        let found = sdk_dxc(&root);
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(found, Some(bin.join("10.0.10.0").join(arch).join(DXC)));
        assert_eq!(sdk_dxc(&root), None);
    }
}
