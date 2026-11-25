//! Which copy of an archive a mount opens: the install's, or the user's overlay.
//!
//! ⚠ **An overlay is not a folder mounted over the install.** It holds
//! *archives* — `mods/update/update.rpf` — which have to be opened as archives
//! and mounted where the install's copy mounts. A [`FolderDevice`] over `mods`
//! would serve the `.rpf` as bytes and never descend into it, so the resolution
//! is per archive path and happens before anything is opened.
//!
//! [`FolderDevice`]: super::FolderDevice

use std::path::{Path, PathBuf};

use crate::dlc::MapMode;

/// The directory inside an install that a user is *expected* to write.
///
/// OpenIV's convention, and the one the game-side loader redirects reads to.
pub const OVERLAY_DIR: &str = "mods";

/// Whether a mount reads the overlay's copy of an archive when there is one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverlayResolution {
    /// The overlay's copy where it has one, the install's everywhere else.
    #[default]
    OverlayFirst,
    /// The install's own archives, whatever the overlay holds.
    InstallOnly,
}

/// Which of the two copies of an archive a mount actually opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveOrigin {
    Install,
    Overlay,
}

impl ArchiveOrigin {
    pub fn is_overlay(self) -> bool {
        matches!(self, Self::Overlay)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Overlay => "overlay",
        }
    }
}

/// What a caller can say about how a mount is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MountOptions {
    pub overlay: OverlayResolution,
    /// The map as story or as multiplayer; `None` applies every change set of every pack.
    pub mode: Option<MapMode>,
}

impl MountOptions {
    /// Read the install's own archives and ignore the overlay.
    pub fn install_only() -> Self {
        Self {
            overlay: OverlayResolution::InstallOnly,
            ..Self::default()
        }
    }

    pub fn reads_the_overlay(self) -> bool {
        self.overlay == OverlayResolution::OverlayFirst
    }
}

/// The overlay directory of an install, whether or not it exists.
pub fn overlay_root(install_root: &Path) -> PathBuf {
    install_root.join(OVERLAY_DIR)
}

/// Is this path the overlay's own — the first component below `install_root`
/// being the overlay directory?
///
/// ⚠ The component is matched **whole**, so `mods_`, `modsbackup` and
/// `mods.rpf` are not the overlay — `mods_` is how a user parks one. And it is
/// **anchored** to the root, so `<install>/x64/mods` is not the overlay either:
/// the game reads no such folder, and the edit gate calling it writable is a
/// permission granted over a directory that does nothing.
pub fn is_under_overlay(install_root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(install_root) else {
        return false;
    };
    relative
        .components()
        .next()
        .and_then(|c| c.as_os_str().to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case(OVERLAY_DIR))
}

/// Where the overlay would hold its copy of an archive the install holds at
/// `install_path`.
///
/// `None` when the path is not inside the install, or is already the overlay's
/// own copy — there is no `mods/mods`.
pub fn overlay_path_for(install_root: &Path, install_path: &Path) -> Option<PathBuf> {
    let relative = install_path.strip_prefix(install_root).ok()?;
    if is_under_overlay(install_root, install_path) {
        return None;
    }
    Some(install_root.join(OVERLAY_DIR).join(relative))
}

/// Where the install holds what the overlay's copy at `overlay_path` stands in for; `None` outside the overlay.
pub fn install_path_for(install_root: &Path, overlay_path: &Path) -> Option<PathBuf> {
    if !is_under_overlay(install_root, overlay_path) {
        return None;
    }
    let below: PathBuf = (overlay_path.strip_prefix(install_root).ok()?.components())
        .skip(1)
        .collect();
    (!below.as_os_str().is_empty()).then(|| install_root.join(below))
}

/// The copy of `install_path` a mount should open, and which one it is.
///
/// ⚠ **Per archive.** A partial overlay — one archive copied, the rest not — is
/// the normal state of a modded install, so resolving a whole tree to one root
/// or the other describes an install nobody has.
pub fn resolve_archive(
    install_root: &Path,
    install_path: &Path,
    options: MountOptions,
) -> (PathBuf, ArchiveOrigin) {
    if is_under_overlay(install_root, install_path) {
        return (install_path.to_path_buf(), ArchiveOrigin::Overlay);
    }
    if !options.reads_the_overlay() {
        return (install_path.to_path_buf(), ArchiveOrigin::Install);
    }
    match overlay_path_for(install_root, install_path) {
        Some(candidate) if candidate.is_file() => (candidate, ArchiveOrigin::Overlay),
        _ => (install_path.to_path_buf(), ArchiveOrigin::Install),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder removed when the first of the pair drops, so hold it as long as the second is used.
    fn temp_dir(name: &str) -> (tempfile::TempDir, PathBuf) {
        let scratch = tempfile::Builder::new()
            .prefix(&format!("or-title-overlay-{name}-"))
            .tempdir()
            .unwrap();
        let dir = scratch.path().to_path_buf();
        (scratch, dir)
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"x").unwrap();
    }

    #[test]
    fn an_overlay_copy_is_preferred_and_a_missing_one_falls_through() {
        let (_scratch, root) = temp_dir("prefer");
        touch(&root.join("update/update.rpf"));
        touch(&root.join("common.rpf"));
        touch(&root.join("mods/update/update.rpf"));

        let (path, origin) = resolve_archive(
            &root,
            &root.join("update/update.rpf"),
            MountOptions::default(),
        );
        assert_eq!(origin, ArchiveOrigin::Overlay);
        assert_eq!(path, root.join("mods/update/update.rpf"));

        // A partial overlay: `common.rpf` has no copy, so it falls through.
        let (path, origin) =
            resolve_archive(&root, &root.join("common.rpf"), MountOptions::default());
        assert_eq!(origin, ArchiveOrigin::Install);
        assert_eq!(path, root.join("common.rpf"));
    }

    #[test]
    fn install_only_reads_the_install_even_with_a_copy_present() {
        let (_scratch, root) = temp_dir("toggle");
        touch(&root.join("update/update.rpf"));
        touch(&root.join("mods/update/update.rpf"));

        let (path, origin) = resolve_archive(
            &root,
            &root.join("update/update.rpf"),
            MountOptions::install_only(),
        );
        assert_eq!(origin, ArchiveOrigin::Install);
        assert_eq!(path, root.join("update/update.rpf"));
    }

    /// ⚠ A directory that only *looks* like the overlay is not it, and neither
    /// is a `mods` at any other depth.
    #[test]
    fn the_overlay_is_one_whole_component_directly_below_the_root() {
        let root = Path::new("C:/Games/GTAV");

        assert!(is_under_overlay(root, &root.join("mods/update/update.rpf")));
        assert!(is_under_overlay(root, &root.join("MODS/x64a.rpf")));

        for not_the_overlay in [
            "mods_/gunskins.rpf",
            "modsbackup/x64a.rpf",
            "mods.rpf",
            // An overlay one directory too deep, which the game ignores.
            "x64/mods/foo.rpf",
            "update/mods/y.ydr",
        ] {
            assert!(
                !is_under_overlay(root, &root.join(not_the_overlay)),
                "`{not_the_overlay}` is not the overlay"
            );
        }
        assert!(!is_under_overlay(
            root,
            Path::new("D:/elsewhere/mods/x.rpf")
        ));

        assert_eq!(
            overlay_path_for(root, &root.join("update/update.rpf")),
            Some(root.join("mods/update/update.rpf"))
        );
        assert_eq!(
            overlay_path_for(root, &root.join("mods/update/update.rpf")),
            None,
            "an archive already in the overlay has no overlay copy of its own"
        );
        assert_eq!(
            overlay_path_for(root, &root.join("mods_/x64a.rpf")),
            Some(root.join("mods/mods_/x64a.rpf"))
        );
        assert_eq!(
            overlay_path_for(root, Path::new("D:/elsewhere/update.rpf")),
            None
        );
    }

    /// ⚠ Resolution is idempotent, and it has to be: several callers resolve a
    /// path and then hand the result to something that resolves again.
    #[test]
    fn resolving_twice_does_not_reach_for_mods_mods() {
        let (_scratch, root) = temp_dir("twice");
        touch(&root.join("common.rpf"));
        touch(&root.join("mods/common.rpf"));

        let (once, first) =
            resolve_archive(&root, &root.join("common.rpf"), MountOptions::default());
        assert_eq!(first, ArchiveOrigin::Overlay);
        let (twice, again) = resolve_archive(&root, &once, MountOptions::default());
        assert_eq!(twice, once, "the second resolve is the identity");
        assert_eq!(
            again,
            ArchiveOrigin::Overlay,
            "and it still says where the file is, rather than reporting the \
             overlay's own copy as the install's"
        );
    }

    #[test]
    fn the_install_holds_the_original_of_an_overlay_copy_where_the_mount_reads_it() {
        let root = Path::new("C:/Games/GTAV");
        for archive in [
            "update/update.rpf",
            "x64a.rpf",
            "update/x64/dlcpacks/a/dlc.rpf",
        ] {
            let shipped = root.join(archive);
            let copy = overlay_path_for(root, &shipped).expect("an install path has a copy");
            assert_eq!(install_path_for(root, &copy), Some(shipped), "{archive}");
        }
        assert_eq!(
            install_path_for(root, &root.join("MODS/x64a.rpf")),
            Some(root.join("x64a.rpf"))
        );
        for not_a_copy in [
            "update/update.rpf",
            "mods_/x64a.rpf",
            "mods",
            "x64/mods/a.rpf",
        ] {
            assert_eq!(
                install_path_for(root, &root.join(not_a_copy)),
                None,
                "{not_a_copy}"
            );
        }
        assert_eq!(
            install_path_for(root, Path::new("D:/elsewhere/mods/x.rpf")),
            None
        );
    }

    /// ⚠ The toggle does not lie about a path that is already in the overlay.
    #[test]
    fn install_only_still_reports_an_overlay_path_as_the_overlay() {
        let (_scratch, root) = temp_dir("honest");
        touch(&root.join("mods/common.rpf"));

        let (path, origin) = resolve_archive(
            &root,
            &root.join("mods/common.rpf"),
            MountOptions::install_only(),
        );
        assert_eq!(path, root.join("mods/common.rpf"));
        assert_eq!(
            origin,
            ArchiveOrigin::Overlay,
            "`InstallOnly` stops the mount *redirecting* to the overlay. A \
             caller that names an overlay path itself is still reading the \
             overlay, and a report saying otherwise is the thing this enum is \
             for."
        );
    }
}
