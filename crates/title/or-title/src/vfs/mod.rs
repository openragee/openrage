//! RAGE-shaped virtual file system for `common:/`-style path lookups.
//!
//! A [`Vfs`] keeps an ordered stack of [`FileDevice`]s per **mount name**,
//! picks the longest-matching mount name and walks its stack
//! **newest-first**, so a later mount shadows the same path in an older one,
//! as the engine's device priority does. [`install`] mounts `common.rpf` on
//! `common:/`, then `update.rpf` twice: on `common:/` with sub-root `common`,
//! shadowing `common.rpf`, and whole on `update:/`; then Enhanced's
//! `update2.rpf` on `common:/` with sub-root `common`.

mod archive_device;
mod device;
mod folder_device;
pub mod game_mount;
pub mod glob;
pub mod install;
pub mod keys;
mod mount;
pub mod overlay;
mod overlay_base;
mod path;
mod shadow;

pub use archive_device::{ArchiveDevice, ArchiveEntry, ArchiveEntryKind};
pub use device::FileDevice;
pub use folder_device::FolderDevice;
pub use game_mount::{
    build_mounted_game, build_mounted_game_with, DlcCutoff, EntryFacts, FileStamp, MountRoot,
    MountedGame, MountedPack,
};
pub use glob::Pattern;
pub use install::{build_vfs_for_install, build_vfs_for_install_with, VfsBuildError};
pub use mount::{GlobResult, Vfs};
pub use overlay::{
    install_path_for, is_under_overlay, overlay_path_for, overlay_root, ArchiveOrigin,
    MountOptions, OverlayResolution, OVERLAY_DIR,
};
pub use overlay_base::{
    record_file, refresh_plan, stale_copies, BaseRecord, Change, Discard, EntryMark, RefreshPlan,
    StaleCopy, Staleness,
};
pub use path::{is_archive_segment, VfsPath, ARCHIVE_EXTENSION};
pub use shadow::{mount_shadow, ShadowReport};
