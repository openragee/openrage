use std::path::PathBuf;
use std::sync::Arc;

use crate::install::materials::GtaInstall;
use crate::vfs::{build_vfs_for_install, VfsBuildError};
use or_io::container::rpf::crypto::RpfCryptoProvider;
use or_io::error::IoError;

pub const DEFAULT_SHADER_DIR: &str = "shaders/win32_40_final";
#[derive(Debug, thiserror::Error)]
pub enum ShaderExtractError {
    #[error("`{path}` not found in GTA install")]
    MissingArchive { path: PathBuf },
    #[error("Failed to open RPF `{path}`: {source}")]
    Rpf {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("No shaders found under `{archive}/{folder}` (is this the right install?)")]
    NoShaders { archive: PathBuf, folder: String },
    #[error("The install did not mount: {0}")]
    Mount(#[source] VfsBuildError),
    #[error("Failed to extract `{entry}` from `{archive}`: {source}")]
    Extract {
        archive: PathBuf,
        entry: String,
        #[source]
        source: IoError,
    },
}

/// Every file directly under `folder` with this extension, as `(stem, bytes)` sorted by stem, as the game's mount resolves it: `update.rpf` and `update2.rpf` replace and add to what `common.rpf` holds.
pub fn extract_entries_from_install(
    install: &GtaInstall,
    crypto: Arc<dyn RpfCryptoProvider>,
    folder: &str,
    extension: &str,
) -> Result<Vec<(String, Vec<u8>)>, ShaderExtractError> {
    let vfs = build_vfs_for_install(install, crypto).map_err(|e| match e {
        VfsBuildError::MissingCommon { path } => ShaderExtractError::MissingArchive { path },
        VfsBuildError::OpenRpf { path, source } => ShaderExtractError::Rpf { path, source },
        other @ VfsBuildError::LevelNotListed { .. } => ShaderExtractError::Mount(other),
    })?;
    let folder_norm = normalise(folder);
    let suffix = format!(".{}", extension.to_ascii_lowercase());

    let mut hits: Vec<String> = vfs
        .list(&format!("common:/{folder_norm}"))
        .into_iter()
        .map(|name| normalise(&name))
        .filter(|name| name.ends_with(&suffix))
        .collect();
    hits.sort();

    if hits.is_empty() {
        return Err(ShaderExtractError::NoShaders {
            archive: install.common_rpf(),
            folder: folder.to_string(),
        });
    }

    let mut out = Vec::with_capacity(hits.len());
    for name in hits {
        let path = format!("{folder_norm}/{name}");
        let bytes =
            vfs.open(&format!("common:/{path}"))
                .ok_or_else(|| ShaderExtractError::Extract {
                    archive: install.common_rpf(),
                    entry: path.clone(),
                    source: IoError::FileNotFound(PathBuf::from(&path)),
                })?;
        out.push((stem_from_path(&path, &suffix), bytes));
    }
    Ok(out)
}

fn stem_from_path(path: &str, suffix: &str) -> String {
    let last = path.rsplit('/').next().unwrap_or(path);
    let stem = last.strip_suffix(suffix).unwrap_or(last);
    stem.to_ascii_lowercase()
}

fn normalise(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::container::rpf::crypto::NoOpCryptoProvider;

    fn write_archive(path: &std::path::Path, files: &[(&str, &[u8])]) {
        let mut builder = or_io::container::rpf::RpfBuilder::default();
        for (inner, body) in files {
            builder.insert_with(inner, *body, false).unwrap();
        }
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        builder.save(path).unwrap();
    }

    #[test]
    fn an_effect_an_update_archive_replaces_or_adds_is_read_from_the_update() {
        let root = tempfile::tempdir().unwrap();
        write_archive(
            &root.path().join("common.rpf"),
            &[
                ("shaders/win32_40_final/a.fxc", b"common-a"),
                ("shaders/win32_40_final/b.fxc", b"common-b"),
            ],
        );
        write_archive(
            &root.path().join("update").join("update2.rpf"),
            &[
                ("common/shaders/win32_40_final/B.fxc", b"update2-b"),
                ("common/shaders/win32_40_final/c.fxc", b"update2-c"),
                ("common/shaders/win32_40_lq_final/d.fxc", b"lq-d"),
            ],
        );
        let install = GtaInstall {
            root: root.path().to_path_buf(),
        };
        let provider: Arc<dyn RpfCryptoProvider> = Arc::new(NoOpCryptoProvider::new());
        let found = extract_entries_from_install(&install, provider, DEFAULT_SHADER_DIR, "fxc")
            .expect("the folder is found");
        assert_eq!(
            found,
            vec![
                ("a".to_string(), b"common-a".to_vec()),
                ("b".to_string(), b"update2-b".to_vec()),
                ("c".to_string(), b"update2-c".to_vec()),
            ],
            "a replaced effect takes the update's bytes, an added one is found, a sibling folder stays out"
        );
    }

    #[test]
    fn a_folder_no_layer_holds_is_refused_naming_it() {
        let root = tempfile::tempdir().unwrap();
        write_archive(&root.path().join("common.rpf"), &[("data/x.dat", b"x")]);
        let install = GtaInstall {
            root: root.path().to_path_buf(),
        };
        let provider: Arc<dyn RpfCryptoProvider> = Arc::new(NoOpCryptoProvider::new());
        let err = extract_entries_from_install(&install, provider, DEFAULT_SHADER_DIR, "fxc")
            .unwrap_err();
        assert!(matches!(err, ShaderExtractError::NoShaders { .. }), "{err}");
    }

    #[test]
    fn missing_install_returns_missing_archive() {
        let install = GtaInstall {
            root: PathBuf::from("/nonexistent/gta-install"),
        };
        let provider: Arc<dyn RpfCryptoProvider> = Arc::new(NoOpCryptoProvider::new());
        let err = extract_entries_from_install(&install, provider, DEFAULT_SHADER_DIR, "fxc")
            .unwrap_err();
        assert!(matches!(err, ShaderExtractError::MissingArchive { .. }));
    }

    #[test]
    fn a_stem_drops_its_folder_and_its_extension() {
        assert_eq!(
            stem_from_path("shaders/win32_40_final/default.fxc", ".fxc"),
            "default"
        );
        assert_eq!(
            stem_from_path("shaders/win32_40_final/Vehicle_Paint.fxc", ".fxc"),
            "vehicle_paint"
        );
        assert_eq!(stem_from_path("foo.fxc", ".fxc"), "foo");
        assert_eq!(stem_from_path("shaders/db/cutout.sps", ".sps"), "cutout");
    }
}
