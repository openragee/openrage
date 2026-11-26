use std::path::PathBuf;
use std::sync::Arc;

use thiserror::Error;

use super::archive_device::ArchiveDevice;
use super::mount::Vfs;
use super::overlay::{resolve_archive, MountOptions};
use crate::install::materials::GtaInstall;
use or_io::container::rpf::crypto::RpfCryptoProvider;
use or_io::container::rpf::RpfArchive;
use or_io::error::IoError;

#[derive(Error, Debug)]
pub enum VfsBuildError {
    #[error("`common.rpf` not found at `{path}`")]
    MissingCommon { path: PathBuf },
    #[error("Failed to open `{path}`: {source}")]
    OpenRpf {
        path: PathBuf,
        #[source]
        source: IoError,
    },
    #[error("the DLC list names no pack `{level}` to stop at, among the {listed} it names")]
    LevelNotListed { level: String, listed: usize },
}

pub fn build_vfs_for_install(
    install: &GtaInstall,
    crypto: Arc<dyn RpfCryptoProvider>,
) -> Result<Vfs, VfsBuildError> {
    build_vfs_for_install_with(install, crypto, MountOptions::default())
}

/// The same archives, with the overlay resolution said out loud.
///
/// ⚠ This builder has no diagnostics to report through, so a caller that
/// needs to know *which* copy it read wants
/// [`build_mounted_game_with`](super::build_mounted_game_with) instead.
pub fn build_vfs_for_install_with(
    install: &GtaInstall,
    crypto: Arc<dyn RpfCryptoProvider>,
    options: MountOptions,
) -> Result<Vfs, VfsBuildError> {
    let mut vfs = Vfs::new();
    let common_path = install.common_rpf();
    if !common_path.exists() {
        return Err(VfsBuildError::MissingCommon { path: common_path });
    }
    let (common_path, _) = resolve_archive(&install.root, &common_path, options);
    let common_archive = open_rpf(&common_path, Arc::clone(&crypto))?;
    vfs.mount("common:", Arc::new(ArchiveDevice::new(common_archive)));

    let update_path = install.update_rpf();
    if update_path.exists() {
        let (update_path, _) = resolve_archive(&install.root, &update_path, options);
        let update_archive = open_rpf(&update_path, Arc::clone(&crypto))?;
        vfs.mount(
            "common:",
            Arc::new(ArchiveDevice::with_sub_root(
                Arc::clone(&update_archive),
                "common",
            )),
        );
        vfs.mount("update:", Arc::new(ArchiveDevice::new(update_archive)));
    }

    // ⚠ Enhanced's `update2.rpf` holds the only `common/shaders/` it ships:
    // its shader container and presets.
    let update2_path = install.root.join("update").join("update2.rpf");
    if update2_path.exists() {
        let (update2_path, _) = resolve_archive(&install.root, &update2_path, options);
        let update2_archive = open_rpf(&update2_path, crypto)?;
        vfs.mount(
            "common:",
            Arc::new(ArchiveDevice::with_sub_root(update2_archive, "common")),
        );
    }
    Ok(vfs)
}

fn open_rpf(
    path: &std::path::Path,
    crypto: Arc<dyn RpfCryptoProvider>,
) -> Result<Arc<RpfArchive>, VfsBuildError> {
    RpfArchive::open_with(path, crypto)
        .map(Arc::new)
        .map_err(|source| VfsBuildError::OpenRpf {
            path: path.to_path_buf(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::container::rpf::crypto::NoOpCryptoProvider;

    fn write_archive(path: &std::path::Path, inner: &str, body: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        or_io::container::rpf::RpfBuilder::default()
            .insert_with(inner, body, false)
            .unwrap()
            .save(path)
            .unwrap();
    }

    #[test]
    fn common_reaches_the_shaders_enhanced_keeps_in_update2() {
        let root = tempfile::tempdir().unwrap();
        write_archive(&root.path().join("common.rpf"), "data/x.dat", b"common");
        write_archive(
            &root.path().join("update").join("update2.rpf"),
            "common/shaders/db/x.sps",
            b"update2",
        );
        let install = GtaInstall {
            root: root.path().to_path_buf(),
        };
        let provider: Arc<dyn RpfCryptoProvider> = Arc::new(NoOpCryptoProvider::new());
        let vfs = build_vfs_for_install(&install, provider).unwrap();
        assert_eq!(
            vfs.open("common:/shaders/db/x.sps").as_deref(),
            Some(&b"update2"[..])
        );
        assert_eq!(
            vfs.open("common:/data/x.dat").as_deref(),
            Some(&b"common"[..])
        );
    }

    #[test]
    fn missing_common_rpf_returns_typed_error() {
        let install = GtaInstall {
            root: PathBuf::from("/nowhere/this-folder-does-not-exist"),
        };
        let provider: Arc<dyn RpfCryptoProvider> = Arc::new(NoOpCryptoProvider::new());
        match build_vfs_for_install(&install, provider) {
            Err(VfsBuildError::MissingCommon { path }) => {
                assert!(path.ends_with("common.rpf"));
            }
            other => panic!("expected MissingCommon, got {other:?}"),
        }
    }
}
