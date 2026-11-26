//! Go and fetch the two tables a shader definition database is assembled from.

use std::sync::Arc;

use or_io::container::rpf::crypto::RpfCryptoProvider;
use or_io::gta5::shaders::fxc::GrcEffect;
use or_io::gta5::shaders::sga::SgaFile;
use or_io::gta5::shaders::shader_defs::{ShaderDefinitions, UnresolvedPreset};
use or_io::gta5::shaders::sps::SpsPreset;
use or_io::Generation;

use crate::install::materials::GtaInstall;
use crate::install::shaders::{
    extract_entries_from_install, ShaderExtractError, DEFAULT_SHADER_DIR,
};
use crate::vfs::{build_vfs_for_install, Vfs, VfsBuildError};

/// Where a Legacy install keeps its `.sps` presets.
pub const DEFAULT_PRESET_DIR: &str = "shaders/db";

/// Where an Enhanced install keeps its shader containers, which hold disjoint effects.
const SGA_DIR: &str = "common:/shaders/fxdb";
const SGA_CONTAINERS: [&str; 2] = ["sga_win32_60_final.awc", "sga_win32_60_final_init.awc"];
/// Where an Enhanced install keeps its `.sps` presets.
const SGA_PRESET_DIR: &str = "common:/shaders/db";

/// What one install yielded, and what it could not resolve.
#[derive(Debug)]
pub struct ShaderDefinitionsLoad {
    pub definitions: ShaderDefinitions,
    /// Presets naming an effect the shader folder does not hold. Retail Legacy
    /// leaves 5 of these even through its update archives; they are reported
    /// rather than defined empty.
    pub unresolved: Vec<UnresolvedPreset>,
    /// Effects whose container would not parse, with the reason.
    pub unreadable: Vec<(String, String)>,
    /// Effects the parse did not read to the end of, with the bytes left over.
    /// ⚠ **This is the check that the reader is right**, not a warning: a
    /// `.fxc` has no table of contents, so a table read at the wrong offset
    /// yields plausible-looking garbage and only the tail says so.
    pub incomplete: Vec<(String, usize)>,
}

/// Assemble the database an install states: `.fxc` and `.sps` for Gen8, the shader containers and `.sps` for Gen9.
///
/// ⚠ An Enhanced install ships no `.fxc` and only 26 of the 411 presets its
/// own preset list names, all in `update/update2.rpf`, so a Gen9 database
/// describes a missing preset only where an effect shares its name. Measured
/// against retail installs of both editions.
pub fn load_shader_definitions(
    install: &GtaInstall,
    crypto: Arc<dyn RpfCryptoProvider>,
    title: &str,
    target: &str,
) -> Result<ShaderDefinitionsLoad, ShaderExtractError> {
    if target == Generation::Gen9.slug() {
        return load_from_containers(install, crypto, title, target);
    }
    let effects_raw =
        extract_entries_from_install(install, Arc::clone(&crypto), DEFAULT_SHADER_DIR, "fxc")?;
    let presets_raw = extract_entries_from_install(install, crypto, DEFAULT_PRESET_DIR, "sps")
        .unwrap_or_default();

    let mut effects = Vec::with_capacity(effects_raw.len());
    let mut unreadable = Vec::new();
    let mut incomplete = Vec::new();
    for (name, bytes) in effects_raw {
        match GrcEffect::parse_counting(&bytes) {
            Ok((effect, consumed)) => {
                if consumed < bytes.len() {
                    incomplete.push((name.clone(), bytes.len() - consumed));
                }
                effects.push((name, effect));
            }
            Err(e) => unreadable.push((name, e.to_string())),
        }
    }

    let presets: Vec<SpsPreset> = presets_raw
        .iter()
        .map(|(name, bytes)| SpsPreset::parse(name, &String::from_utf8_lossy(bytes)))
        .collect();

    let (definitions, unresolved) = ShaderDefinitions::assemble(title, target, &effects, &presets);
    Ok(ShaderDefinitionsLoad {
        definitions,
        unresolved,
        unreadable,
        incomplete,
    })
}

fn mounted(
    install: &GtaInstall,
    crypto: Arc<dyn RpfCryptoProvider>,
) -> Result<Vfs, ShaderExtractError> {
    build_vfs_for_install(install, crypto).map_err(|e| match e {
        VfsBuildError::MissingCommon { path } => ShaderExtractError::MissingArchive { path },
        VfsBuildError::OpenRpf { path, source } => ShaderExtractError::Rpf { path, source },
        other @ VfsBuildError::LevelNotListed { .. } => ShaderExtractError::Mount(other),
    })
}

/// Each shader container of an Enhanced install, as `(file name, bytes)`.
fn sga_containers(
    vfs: &Vfs,
    install: &GtaInstall,
) -> Result<Vec<(&'static str, Vec<u8>)>, ShaderExtractError> {
    SGA_CONTAINERS
        .into_iter()
        .map(|name| {
            vfs.open(&format!("{SGA_DIR}/{name}"))
                .map(|bytes| (name, bytes))
                .ok_or_else(|| ShaderExtractError::NoShaders {
                    archive: install.root.join("update").join("update2.rpf"),
                    folder: format!("common/shaders/fxdb/{name}"),
                })
        })
        .collect()
}

/// The raw shader containers an Enhanced install ships in place of `.fxc`.
pub fn extract_sga_containers(
    install: &GtaInstall,
    crypto: Arc<dyn RpfCryptoProvider>,
) -> Result<Vec<(String, Vec<u8>)>, ShaderExtractError> {
    let vfs = mounted(install, crypto)?;
    Ok(sga_containers(&vfs, install)?
        .into_iter()
        .map(|(name, bytes)| (name.to_string(), bytes))
        .collect())
}

fn load_from_containers(
    install: &GtaInstall,
    crypto: Arc<dyn RpfCryptoProvider>,
    title: &str,
    target: &str,
) -> Result<ShaderDefinitionsLoad, ShaderExtractError> {
    let vfs = mounted(install, crypto)?;

    let mut effects = Vec::new();
    let mut unreadable = Vec::new();
    for (name, bytes) in sga_containers(&vfs, install)? {
        match SgaFile::parse(&bytes) {
            Ok(file) => effects.extend(file.effects),
            Err(e) => unreadable.push((name.to_string(), e.to_string())),
        }
    }

    let presets: Vec<SpsPreset> = (vfs.list(SGA_PRESET_DIR).iter())
        .filter_map(|file| {
            let stem = file.to_ascii_lowercase().strip_suffix(".sps")?.to_string();
            let bytes = vfs.open(&format!("{SGA_PRESET_DIR}/{file}"))?;
            Some(SpsPreset::parse(&stem, &String::from_utf8_lossy(&bytes)))
        })
        .collect();

    let (definitions, unresolved) =
        ShaderDefinitions::assemble_sga(title, target, &effects, &presets);
    Ok(ShaderDefinitionsLoad {
        definitions,
        unresolved,
        unreadable,
        incomplete: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::container::rpf::crypto::NoOpCryptoProvider;
    use std::path::PathBuf;

    #[test]
    fn a_missing_install_is_refused_before_anything_is_assembled() {
        let install = GtaInstall {
            root: PathBuf::from("/nonexistent/gta-install"),
        };
        let crypto: Arc<dyn RpfCryptoProvider> = Arc::new(NoOpCryptoProvider::new());
        let err = load_shader_definitions(&install, crypto, "gta5", "gen8").unwrap_err();
        assert!(matches!(err, ShaderExtractError::MissingArchive { .. }));
    }
}
