//! A game install: which edition it is, where its archives are, what order
//! they stack in, and how a path resolves through them.
//!
//! ⚠ An edition is not a generation: an Enhanced install ships plenty of Gen8
//! resources, so layout selection comes from a file's own RSC7 version and
//! never from the edition of the install it came out of.

pub mod archives;
pub mod config;
pub mod dlc;
pub mod game;
pub mod gta5;
pub mod gta6_cloud;
pub mod gta6_day;
pub mod gta6_day_frame;
pub mod gta6_day_sun;
pub mod gta6_day_vars;
pub mod gta6_day_weather;
pub mod gta6_day_wind;
pub mod gta6_exposure;
pub mod gta6_mount;
pub mod gta6_sky;
pub mod install;
pub mod mods;
pub mod oiv;
pub mod schema;
pub mod vfs;

pub use archives::{format_recovery_hint, KnownArchiveNames};

pub use dlc::{
    ContentXml, DataFileEntry, DlcList, ExtraTitleUpdateData, MapMode, SetupContentType, SetupData,
    TitleUpdateMount,
};
pub use game::{ContainerFamily, DlcPolicy, Game, GameId, GamePaths, GameProfile, InstallLayout};
pub use gta5::{Gta5, InstallError};
pub use install::shaders as shader_extract;
pub use install::{
    classify_leaf, load_clip_regions,
    load_from_gta_install as load_material_palette_from_gta_install, load_timecycle,
    read_sky_textures, read_textures_files, read_water_textures, scan_install_names,
    timecycle_of_documents, ClipRegionLoadError, GtaInstall, InstalledNameKind,
    MaterialPaletteLoadError, TimecycleData, CLIP_REGIONS_VFS_PATH, SKY_TEXTURES_FILE,
    WATER_TEXTURES_FILE,
};
pub use shader_extract::{ShaderExtractError, DEFAULT_SHADER_DIR};
pub use vfs::{
    build_mounted_game, build_mounted_game_with, build_vfs_for_install, build_vfs_for_install_with,
    is_under_overlay, mount_shadow, record_file, refresh_plan, stale_copies, ArchiveDevice,
    ArchiveOrigin, BaseRecord, Change, Discard, DlcCutoff, FileDevice, FolderDevice, ListedPack,
    ListedPackState, MountOptions, MountRoot, MountedGame, MountedPack, OverlayResolution,
    RefreshPlan, ShadowReport, StaleCopy, Staleness, Vfs, VfsBuildError, VfsPath, OVERLAY_DIR,
};
