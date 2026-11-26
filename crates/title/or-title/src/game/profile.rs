//! What OpenRage needs to know about a title in order to open its files.
//!
//! ⚠ A title, its container family and a file's asset target are separate
//! axes: GTA V is RPF7/RSC7 in both editions, and an Enhanced install ships
//! plenty of Gen8 files, so a file's target is read from its own header and
//! never from the profile.

use std::path::{Path, PathBuf};

use or_io::Generation;

use super::overlay_loader::OverlayLoaders;

pub type GameId = &'static str;

/// The archive and resource container family, named for the container rather than the title.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContainerFamily {
    // `RPF7` archives, `RSC7` resources. GTA V, both editions.
    Rpf7,
    // `RPF8` archives, `RSC8` resources. RDR2. Not a variation on RPF7: its
    // table of contents stores hashes instead of names, and its default payload
    // compression is Oodle Kraken rather than DEFLATE.
    Rpf8,
    // `RPF9` archives (magic `9FPR`). GTA VI. Only the 64-byte header is
    // plaintext: the entry table and the names blob are ciphertext, and no key
    // source for them exists. Its resource container is not established.
    Rpf9,
}

impl ContainerFamily {
    pub fn is_implemented(self) -> bool {
        matches!(self, ContainerFamily::Rpf7 | ContainerFamily::Rpf8)
    }

    pub fn label(self) -> &'static str {
        match self {
            ContainerFamily::Rpf7 => "RPF7 / RSC7",
            ContainerFamily::Rpf8 => "RPF8 / RSC8",
            ContainerFamily::Rpf9 => "RPF9",
        }
    }
}

/// The machine a build of a title was made for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    Pc,
    Ps4,
    XboxOne,
    Ps5,
    XboxSeries,
}

impl Platform {
    pub fn label(self) -> &'static str {
        match self {
            Platform::Pc => "PC",
            Platform::Ps4 => "PlayStation 4",
            Platform::XboxOne => "Xbox One",
            Platform::Ps5 => "PlayStation 5",
            Platform::XboxSeries => "Xbox Series",
        }
    }
}

/// Where a PlayStation package states its title id: an SFO on a PlayStation 4, JSON on a PlayStation 5.
pub const PLAYSTATION_PARAMS: &[&str] = &["sce_sys/param.sfo", "sce_sys/param.json"];

/// Whether a parameter file of `root` holds one of `title_ids`, which both forms store as plain ASCII.
pub(crate) fn playstation_params_name(root: &Path, title_ids: &[&str]) -> bool {
    use std::io::Read;

    PLAYSTATION_PARAMS.iter().any(|name| {
        let Ok(file) = std::fs::File::open(root.join(name)) else {
            return false;
        };
        let mut bytes = Vec::new();
        if file
            .take(GDK_CONFIG_MAX_BYTES)
            .read_to_end(&mut bytes)
            .is_err()
        {
            return false;
        }
        title_ids
            .iter()
            .any(|id| !id.is_empty() && bytes.windows(id.len()).any(|w| w == id.as_bytes()))
    })
}

/// The GDK title configuration an Xbox package carries at its root.
pub const GDK_CONFIG: &str = "MicrosoftGame.Config";

// A cap on what is read: a file this large at that name is not a title configuration.
const GDK_CONFIG_MAX_BYTES: u64 = 64 * 1024;

/// Whether `root`'s `MicrosoftGame.Config` is a `<Game>` whose `<TitleId>` is `title_id`, compared without case.
pub(crate) fn gdk_config_names(root: &Path, title_id: &str) -> bool {
    use std::io::Read;

    let Ok(file) = std::fs::File::open(root.join(GDK_CONFIG)) else {
        return false;
    };
    let mut bytes = Vec::new();
    if file
        .take(GDK_CONFIG_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > GDK_CONFIG_MAX_BYTES
    {
        return false;
    }
    match crate::dlc::tree::parse_tree(&bytes, GDK_CONFIG) {
        Ok(game) => {
            game.name.eq_ignore_ascii_case("Game")
                && game
                    .child("TitleId")
                    .is_some_and(|id| id.text.trim().eq_ignore_ascii_case(title_id))
        }
        Err(_) => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallLayout {
    pub base_archive: &'static str,
    pub update_archive: Option<&'static str>,
    pub dlc_directories: &'static [&'static str],
    pub platform_archive_prefix: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DlcPolicy {
    MountAll,
    ExcludeNameToken(&'static str),
}

impl DlcPolicy {
    pub fn accepts(self, pack_name: &str) -> bool {
        match self {
            DlcPolicy::MountAll => true,
            DlcPolicy::ExcludeNameToken(token) => !pack_name.to_ascii_lowercase().contains(token),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StoreHints {
    pub steam_folders: &'static [&'static str],
    pub launcher_registry_keys: &'static [&'static str],
    pub epic_display_names: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
pub struct Brand {
    pub background: ([u8; 3], [u8; 3]),
    pub accent: [u8; 3],
    pub caption_color: [u8; 3],
    pub caption: &'static str,
    pub caption_scale: f32,
}

impl Default for Brand {
    fn default() -> Self {
        Self {
            background: ([0x1f, 0x1f, 0x1f], [0x36, 0x36, 0x36]),
            accent: [0x8a, 0x8a, 0x8a],
            caption_color: [0xd0, 0xd0, 0xd0],
            caption: "",
            caption_scale: 0.4,
        }
    }
}

pub trait GameProfile: Send + Sync + 'static {
    fn id(&self) -> GameId;

    fn label(&self) -> &'static str;

    fn description(&self) -> &'static str;

    /// The game this is a build of, which every build of it names alike.
    fn title(&self) -> &'static str {
        self.label()
    }

    fn platform(&self) -> Platform {
        Platform::Pc
    }

    fn container(&self) -> ContainerFamily;

    fn native_target(&self) -> Generation;

    fn asset_targets(&self) -> &'static [Generation];

    fn target_labels(&self) -> &'static [or_io::TargetDescriptor] {
        &[]
    }

    fn executable_candidates(&self) -> &'static [&'static str];

    // ⚠ Separate from `executable_candidates`, which identifies an install and
    // must stay the game binary: key extraction needs it, so a launcher there
    // would satisfy detection and then fail extraction. An icon prefers the
    // opposite: GTA V's `PlayGTAV.exe` ships a 256x256 resource `GTA5.exe` lacks.
    fn icon_executable_candidates(&self) -> &'static [&'static str] {
        self.executable_candidates()
    }

    fn store_hints(&self) -> StoreHints {
        StoreHints::default()
    }

    fn brand(&self) -> Brand {
        Brand::default()
    }

    fn install_layout(&self) -> InstallLayout;

    fn dlc_policy(&self) -> DlcPolicy;

    /// A second title-update archive the app's mount layers over the update archive, install-relative.
    fn update2_archive(&self) -> Option<&'static str> {
        None
    }

    /// What the game needs beside its executable to read an install's `mods` overlay; nothing is known by default.
    fn overlay_loaders(&self) -> OverlayLoaders {
        OverlayLoaders::NONE
    }

    fn supports_key_extraction(&self) -> bool {
        false
    }

    fn keys_cache_name(&self) -> Option<&'static str> {
        None
    }

    /// The `TitleId` a `MicrosoftGame.Config` at the install root names, which marks an install whose executable is absent.
    fn gdk_title_id(&self) -> Option<&'static str> {
        None
    }

    /// The title ids a PlayStation package of this build states, one per region.
    fn playstation_title_ids(&self) -> &'static [&'static str] {
        &[]
    }

    fn matches_install(&self, root: &Path) -> bool {
        self.executable_candidates()
            .iter()
            .any(|name| root.join(name).is_file())
            || self
                .gdk_title_id()
                .is_some_and(|id| gdk_config_names(root, id))
            || playstation_params_name(root, self.playstation_title_ids())
    }

    fn is_supported(&self) -> bool {
        self.container().is_implemented()
    }

    fn find_executable(&self, root: &Path) -> Option<PathBuf> {
        self.executable_candidates()
            .iter()
            .map(|name| root.join(name))
            .find(|p| p.is_file())
    }

    fn executable_name(&self) -> &'static str {
        self.executable_candidates()
            .first()
            .copied()
            .unwrap_or("(unknown)")
    }
}
