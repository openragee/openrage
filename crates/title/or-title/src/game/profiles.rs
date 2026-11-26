use or_io::Generation;

use super::overlay_loader::{OverlayLoaders, Project, Redirect};
use super::profile::{
    Brand, ContainerFamily, DlcPolicy, GameProfile, InstallLayout, Platform, StoreHints,
};

const OPEN_IV: Project = Project {
    name: "OpenIV",
    page: "https://openiv.com",
};
const OPEN_RPF: Project = Project {
    name: "OpenRPF",
    page: "https://www.gta5-mods.com/tools/openrpf-openiv-asi-for-gta-v-enhanced",
};
const RAGE_OPEN_V: Project = Project {
    name: "RageOpenV",
    page: "https://github.com/Chiheb-Bacha/RageOpenV",
};
const ULTIMATE_ASI_LOADER: Project = Project {
    name: "Ultimate ASI Loader",
    page: "https://github.com/ThirteenAG/Ultimate-ASI-Loader",
};
const SCRIPT_HOOK_V: Project = Project {
    name: "ScriptHookV",
    page: "https://www.dev-c.com/gtav/scripthookv/",
};

// Open source and one binary for both editions, so it leads.
const RAGE_OPEN_V_PLUGIN: Redirect = Redirect {
    file: "RageOpenV.asi",
    project: RAGE_OPEN_V,
    loads_itself: false,
};

const LEGACY_OVERLAY: OverlayLoaders = OverlayLoaders {
    redirects: &[
        RAGE_OPEN_V_PLUGIN,
        Redirect {
            file: "OpenIV.asi",
            project: OPEN_IV,
            loads_itself: false,
        },
    ],
    loader_files: &["dinput8.dll"],
    loader_projects: &[ULTIMATE_ASI_LOADER, SCRIPT_HOOK_V],
};

// OpenRPF carries its own ASI loader from 0.2; its 0.1 needed a `dsound.dll`, so a bare 0.1 reads as working.
const ENHANCED_OVERLAY: OverlayLoaders = OverlayLoaders {
    redirects: &[
        RAGE_OPEN_V_PLUGIN,
        Redirect {
            file: "OpenRPF.asi",
            project: OPEN_RPF,
            loads_itself: true,
        },
    ],
    loader_files: &["xinput1_4.dll", "dsound.dll"],
    loader_projects: &[ULTIMATE_ASI_LOADER],
};

const GTA_V_LAYOUT: InstallLayout = InstallLayout {
    base_archive: "common.rpf",
    update_archive: Some("update/update.rpf"),
    dlc_directories: &["update/x64/dlcpacks", "x64/dlcpacks"],
    platform_archive_prefix: "x64",
};

const GEN9_PACK_TOKEN: &str = "g9ec";

#[derive(Debug, Clone, Copy, Default)]
pub struct GtaVLegacy;

impl GameProfile for GtaVLegacy {
    fn id(&self) -> &'static str {
        "gta-v-legacy"
    }

    fn label(&self) -> &'static str {
        "GTA V (Legacy)"
    }

    fn description(&self) -> &'static str {
        "Original / classic build of GTA V."
    }

    fn title(&self) -> &'static str {
        "GTA V"
    }

    fn container(&self) -> ContainerFamily {
        ContainerFamily::Rpf7
    }

    fn native_target(&self) -> Generation {
        Generation::Gen8
    }

    fn asset_targets(&self) -> &'static [Generation] {
        &[Generation::Gen8]
    }

    fn executable_candidates(&self) -> &'static [&'static str] {
        // Launcher executables are deliberately absent: key extraction needs the
        // game binary, and a launcher would satisfy detection while failing
        // extraction.
        &["GTA5.exe"]
    }

    fn icon_executable_candidates(&self) -> &'static [&'static str] {
        &["PlayGTAV.exe", "GTAVLauncher.exe", "GTA5.exe"]
    }

    fn store_hints(&self) -> StoreHints {
        StoreHints {
            steam_folders: &["Grand Theft Auto V"],
            launcher_registry_keys: &[r"SOFTWARE\WOW6432Node\Rockstar Games\Grand Theft Auto V"],
            epic_display_names: &["Grand Theft Auto V", "Grand Theft Auto V: Premium Edition"],
        }
    }

    fn brand(&self) -> Brand {
        Brand {
            background: ([0x1f, 0x1f, 0x1f], [0x36, 0x36, 0x36]),
            accent: [0xfb, 0xc8, 0x04],
            caption_color: [0xfb, 0xc8, 0x04],
            caption: "V",
            caption_scale: 0.55,
        }
    }

    fn install_layout(&self) -> InstallLayout {
        GTA_V_LAYOUT
    }

    fn dlc_policy(&self) -> DlcPolicy {
        DlcPolicy::ExcludeNameToken(GEN9_PACK_TOKEN)
    }

    fn update2_archive(&self) -> Option<&'static str> {
        Some("update/update2.rpf")
    }

    fn overlay_loaders(&self) -> OverlayLoaders {
        LEGACY_OVERLAY
    }

    fn supports_key_extraction(&self) -> bool {
        true
    }

    fn keys_cache_name(&self) -> Option<&'static str> {
        Some("gta5_keys.bin")
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct GtaVEnhanced;

impl GameProfile for GtaVEnhanced {
    fn id(&self) -> &'static str {
        "gta-v-enhanced"
    }

    fn label(&self) -> &'static str {
        "GTA V (Enhanced)"
    }

    fn description(&self) -> &'static str {
        "GTA V Enhanced edition (2024 relaunch)."
    }

    fn title(&self) -> &'static str {
        "GTA V"
    }

    fn container(&self) -> ContainerFamily {
        // Still RPF7 and still RSC7. Enhanced renumbered the per-format resource
        // versions in both directions — `.ytd` went 13 to 5 while `.yft` went
        // 162 to 171 — so no threshold rule can identify a generation.
        ContainerFamily::Rpf7
    }

    fn native_target(&self) -> Generation {
        Generation::Gen9
    }

    fn asset_targets(&self) -> &'static [Generation] {
        &[Generation::Gen8, Generation::Gen9]
    }

    fn executable_candidates(&self) -> &'static [&'static str] {
        &["GTA5_Enhanced.exe", "GTA5.exe"]
    }

    fn icon_executable_candidates(&self) -> &'static [&'static str] {
        &["PlayGTAV.exe", "GTA5_Enhanced.exe", "GTA5.exe"]
    }

    fn store_hints(&self) -> StoreHints {
        StoreHints {
            steam_folders: &[
                "Grand Theft Auto V Enhanced",
                "Grand Theft Auto V_Enhanced",
                "GTA V Enhanced",
            ],
            launcher_registry_keys: &[
                r"SOFTWARE\WOW6432Node\Rockstar Games\Grand Theft Auto V Enhanced",
                r"SOFTWARE\WOW6432Node\Rockstar Games\GTAV Enhanced",
            ],
            epic_display_names: &[
                "Grand Theft Auto V Enhanced",
                "Grand Theft Auto V (Enhanced)",
            ],
        }
    }

    fn brand(&self) -> Brand {
        Brand {
            background: ([0x10, 0x18, 0x2a], [0x1d, 0x2c, 0x4d]),
            accent: [0x4e, 0xc4, 0xff],
            caption_color: [0xff, 0xff, 0xff],
            caption: "V+",
            caption_scale: 0.45,
        }
    }

    fn install_layout(&self) -> InstallLayout {
        GTA_V_LAYOUT
    }

    fn dlc_policy(&self) -> DlcPolicy {
        DlcPolicy::MountAll
    }

    fn update2_archive(&self) -> Option<&'static str> {
        Some("update/update2.rpf")
    }

    fn overlay_loaders(&self) -> OverlayLoaders {
        ENHANCED_OVERLAY
    }

    fn supports_key_extraction(&self) -> bool {
        true
    }

    fn keys_cache_name(&self) -> Option<&'static str> {
        Some("gta5_enhanced_keys.bin")
    }

    fn matches_install(&self, root: &std::path::Path) -> bool {
        root.join("GTA5_Enhanced.exe").is_file()
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Rdr2;

/// RDR2's own asset target.
///
/// ⚠ **Not [`Generation::Gen8`]**, which names GTA V's eighth-generation
/// *layouts*, none of which RDR2 shares.
///
/// ⚠ **Naming a target is not describing one.** Nothing in this workspace reads
/// an RSC8 layout.
const RDR2_TARGET: Generation = Generation::from_id("rdr2");

impl GameProfile for Rdr2 {
    fn id(&self) -> &'static str {
        "rdr2"
    }

    fn label(&self) -> &'static str {
        "Red Dead Redemption 2"
    }

    fn description(&self) -> &'static str {
        "Red Dead Redemption 2. Its RPF8 archives open; its resources (RSC8) do not decode yet."
    }

    fn container(&self) -> ContainerFamily {
        ContainerFamily::Rpf8
    }

    fn native_target(&self) -> Generation {
        RDR2_TARGET
    }

    fn asset_targets(&self) -> &'static [Generation] {
        &[RDR2_TARGET]
    }

    fn target_labels(&self) -> &'static [or_io::TargetDescriptor] {
        &[or_io::TargetDescriptor {
            id: "rdr2",
            label: "Red Dead Redemption 2",
        }]
    }

    fn executable_candidates(&self) -> &'static [&'static str] {
        &["RDR2.exe"]
    }

    fn icon_executable_candidates(&self) -> &'static [&'static str] {
        &["RDR2.exe", "Launcher.exe"]
    }

    fn store_hints(&self) -> StoreHints {
        StoreHints {
            steam_folders: &["Red Dead Redemption 2"],
            launcher_registry_keys: &[r"SOFTWARE\WOW6432Node\Rockstar Games\Red Dead Redemption 2"],
            epic_display_names: &["Red Dead Redemption 2"],
        }
    }

    fn brand(&self) -> Brand {
        Brand {
            background: ([0x3a, 0x0e, 0x0a], [0x6c, 0x18, 0x10]),
            accent: [0xf2, 0xc1, 0x6b],
            caption_color: [0xf2, 0xc1, 0x6b],
            caption: "RDR2",
            caption_scale: 0.26,
        }
    }

    fn install_layout(&self) -> InstallLayout {
        InstallLayout {
            base_archive: "common_0.rpf",
            update_archive: None,
            dlc_directories: &[],
            platform_archive_prefix: "",
        }
    }

    fn dlc_policy(&self) -> DlcPolicy {
        DlcPolicy::MountAll
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Gta6;

/// GTA VI's own asset target; nothing in this workspace reads a layout of it.
const GTA6_TARGET: Generation = Generation::from_id("gta-vi");

impl GameProfile for Gta6 {
    fn id(&self) -> &'static str {
        "gta-vi"
    }

    fn label(&self) -> &'static str {
        "GTA VI"
    }

    fn description(&self) -> &'static str {
        "GTA VI. Its RPF9 archives' tables are encrypted: with a key snapshot an archive is browsed as a folder of its entries, and with no key it opens as a scan of the bodies stored in the clear. The install opens as a plain folder."
    }

    // The only build read so far is an Xbox package.
    fn platform(&self) -> Platform {
        Platform::XboxSeries
    }

    fn container(&self) -> ContainerFamily {
        ContainerFamily::Rpf9
    }

    fn native_target(&self) -> Generation {
        GTA6_TARGET
    }

    fn asset_targets(&self) -> &'static [Generation] {
        &[GTA6_TARGET]
    }

    fn target_labels(&self) -> &'static [or_io::TargetDescriptor] {
        &[or_io::TargetDescriptor {
            id: "gta-vi",
            label: "GTA VI",
        }]
    }

    // The executable an Xbox package's title configuration names.
    fn executable_candidates(&self) -> &'static [&'static str] {
        &["game_scarlett_master.exe"]
    }

    fn gdk_title_id(&self) -> Option<&'static str> {
        Some("7952987A")
    }

    // ⚠ Empty on purpose: a GTA VI install is mounted by the archives and
    // documents its registration documents name, never by a base archive and an
    // update layered over it, so naming `common.rpf` here would give it a role
    // it does not have.
    fn install_layout(&self) -> InstallLayout {
        InstallLayout {
            base_archive: "",
            update_archive: None,
            dlc_directories: &[],
            platform_archive_prefix: "",
        }
    }

    fn dlc_policy(&self) -> DlcPolicy {
        DlcPolicy::MountAll
    }
}
