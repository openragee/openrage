use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use or_renderer::ShadingMode;
use serde::{Deserialize, Serialize};

use super::drivers::{Drivers, ExposureDriver, WorldBackgroundDriver};
use super::explorer::Place;

/// 5 added `appearance`, which reads as its default when absent; 6 moved the
/// recent files to `recents`, each with where it is and when it was opened.
const PREFS_SCHEMA: u32 = 6;
const MAX_RECENT: usize = 10;

pub use or_title::{Game, GamePaths};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RpfOutputMode {
    #[default]
    Open,
    Aes,
    Ng,
}

impl RpfOutputMode {
    pub const ALL: [RpfOutputMode; 3] = [Self::Open, Self::Aes, Self::Ng];

    pub fn label(self) -> &'static str {
        match self {
            Self::Open => tr!("OPEN (plaintext)"),
            Self::Aes => tr!("AES (retail)"),
            Self::Ng => tr!("NG (retail)"),
        }
    }
}

/// What an export does where OpenRage's canonical form alters a file's bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ExportFixes {
    #[default]
    Ask,
    Apply,
    Keep,
}

impl ExportFixes {
    pub const ALL: [ExportFixes; 3] = [Self::Ask, Self::Apply, Self::Keep];

    pub fn label(self) -> &'static str {
        match self {
            Self::Ask => tr!("Ask each time"),
            Self::Apply => tr!("Apply OpenRage export fixes"),
            Self::Keep => tr!("Keep the original bytes"),
        }
    }
}

/// How much of the game the user has said they are willing to write.
///
/// | mode | outside an install | the `mods` overlay | the game as shipped |
/// |---|---|---|---|
/// | [`ReadOnly`](Self::ReadOnly) — **default** | yes | no | no |
/// | [`Overlay`](Self::Overlay) | yes | yes | no |
/// | [`Everything`](Self::Everything) | yes | yes | yes, once a write is confirmed |
///
/// ⚠ **Writing outside an install is always allowed**, in every mode including
/// `ReadOnly`. The gate is about the *game*, and a mode that stopped a user
/// saving a `.ydr` to their desktop would be read as the application being
/// broken, which is how a safety feature gets switched off permanently.
///
/// ⚠ `Everything` asks before a session's first base-game write, and the Yes
/// lasts until the mode changes or OpenRage closes. The earlier `YesAlways`
/// never asked again, and a persisted "never warn me again" on the one
/// operation that can cost a user their install is a decision they made once
/// and cannot see.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditMode {
    #[default]
    ReadOnly,
    Overlay,
    Everything,
}

impl EditMode {
    pub const ALL: [EditMode; 3] = [Self::ReadOnly, Self::Overlay, Self::Everything];

    pub fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => tr!("Read only"),
            Self::Overlay => tr!("Edit the mods overlay"),
            Self::Everything => tr!("Edit base game files"),
        }
    }

    /// One sentence a user can act on.
    pub fn semantics(self) -> &'static str {
        match self {
            Self::ReadOnly => {
                tr!(
                    "Nothing inside a game install can be written. Files outside one \
                 save normally."
                )
            }
            Self::Overlay => {
                tr!(
                    "Files under the install's `mods` directory can be written. The \
                 game as shipped cannot."
                )
            }
            Self::Everything => {
                tr!(
                    "The game as shipped can be written: the first write asks, and that \
                 Yes lasts until the mode changes or OpenRage closes. A one-time \
                 `.bak` is kept beside anything overwritten."
                )
            }
        }
    }

    pub fn permits_overlay(self) -> bool {
        matches!(self, Self::Overlay | Self::Everything)
    }

    pub fn permits_shipped_game(self) -> bool {
        matches!(self, Self::Everything)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EditorWindowPrefs {
    #[serde(default)]
    pub size: Option<(u32, u32)>,
    #[serde(default)]
    pub position: Option<(i32, i32)>,
}

/// How the viewport looks, as the user last left it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ViewportPrefs {
    pub show_grid: bool,
    pub show_axis: bool,
    #[serde(serialize_with = "shading_key")]
    pub shading: ShadingMode,
    /// The colour the viewport is cleared to.
    pub background: [f32; 4],
    /// Whether Shaded draws the game's sky where nothing else is, in place of the background colour.
    pub sky: bool,
    /// Whether Shaded lays a ground under a particle effect, for its light to land on.
    pub ground: bool,
    pub exposure: f32,
}

/// Each shading mode a restart opens in; Channel shows one file's channel and is not one. `game` is what the mode that draws with the game's shaders was saved as before it took the name `shaded`.
const SHADING_KEYS: [(ShadingMode, &str); 5] = [
    (ShadingMode::Wireframe, "wireframe"),
    (ShadingMode::Solid, "solid"),
    (ShadingMode::Material, "material"),
    (ShadingMode::Shaded, "shaded"),
    (ShadingMode::Shaded, "game"),
];

fn key_of<T: PartialEq + Copy>(keys: &[(T, &'static str)], value: T) -> &'static str {
    keys.iter()
        .find(|(v, _)| *v == value)
        .map_or(keys[0].1, |(_, key)| key)
}

fn shading_key<S: serde::Serializer>(mode: &ShadingMode, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(key_of(&SHADING_KEYS, *mode))
}

impl Default for ViewportPrefs {
    fn default() -> Self {
        Self {
            show_grid: true,
            show_axis: true,
            shading: ShadingMode::Material,
            background: WorldBackgroundDriver::default().colour,
            sky: WorldBackgroundDriver::default().sky,
            ground: WorldBackgroundDriver::default().ground,
            exposure: ExposureDriver::default().value,
        }
    }
}

impl ViewportPrefs {
    /// ⚠ Field by field, as `AppearancePrefs` reads: a value that does not read costs only itself.
    fn from_value(value: &serde_json::Value) -> Self {
        let mut out = Self::default();
        let Some(map) = value.as_object() else {
            return out;
        };
        let flag = |key: &str| map.get(key).and_then(serde_json::Value::as_bool);
        let keyed = |key: &str| map.get(key).and_then(serde_json::Value::as_str);
        let number = |key: &str| {
            map.get(key)
                .and_then(serde_json::Value::as_f64)
                .map(|v| v as f32)
                .filter(|v| v.is_finite())
        };
        let colour = |key: &str| {
            map.get(key)
                .and_then(|v| serde_json::from_value::<[f32; 4]>(v.clone()).ok())
                .filter(|c| c.iter().all(|v| v.is_finite()))
        };
        out.show_grid = flag("show_grid").unwrap_or(out.show_grid);
        out.show_axis = flag("show_axis").unwrap_or(out.show_axis);
        if let Some((mode, _)) = SHADING_KEYS
            .iter()
            .find(|(_, k)| Some(*k) == keyed("shading"))
        {
            out.shading = *mode;
        }
        out.background = colour("background").unwrap_or(out.background);
        out.sky = flag("sky").unwrap_or(out.sky);
        out.ground = flag("ground").unwrap_or(out.ground);
        out.exposure = number("exposure").map_or(out.exposure, |v| v.clamp(0.0, 1.0));
        out
    }

    /// Keep the background and exposure the viewport's drivers hold now.
    pub fn keep(&mut self, drivers: &Drivers) {
        self.background = drivers.world_background.colour;
        self.sky = drivers.world_background.sky;
        self.ground = drivers.world_background.ground;
        self.exposure = drivers.exposure.value;
    }
}

fn readable_viewport<'de, D: serde::Deserializer<'de>>(d: D) -> Result<ViewportPrefs, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(ViewportPrefs::from_value(&value))
}

impl super::AppState {
    /// Keep the viewport's background and exposure, and save them with its other preferences.
    pub fn persist_viewport_prefs(&mut self) {
        self.prefs.viewport.keep(&self.drivers);
        save_prefs(&self.prefs);
    }

    /// Remember the shading mode a restart opens the viewport in.
    pub fn remember_shading_mode(&mut self, mode: ShadingMode) {
        let kept = SHADING_KEYS.iter().any(|(m, _)| *m == mode);
        if kept && self.prefs.viewport.shading != mode {
            self.prefs.viewport.shading = mode;
            save_prefs(&self.prefs);
        }
    }

    /// Draw or hide the skeleton source's own model in the Animation preview, and keep the choice
    /// for a restart.
    pub fn set_draw_skeleton_model(&mut self, on: bool) {
        self.preview_scene.draw_skeleton_model = on;
        if self.prefs.draw_skeleton_model != on {
            self.prefs.draw_skeleton_model = on;
            save_prefs(&self.prefs);
        }
    }

    /// Keep the inspector's Show internals as the user left it, for a restart.
    pub fn set_show_internals(&mut self, on: bool) {
        if self.prefs.show_internals != on {
            self.prefs.show_internals = on;
            save_prefs(&self.prefs);
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub enum ThemeChoice {
    #[default]
    Dark,
    Light,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 2] = [Self::Dark, Self::Light];

    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => tr!("Dark"),
            Self::Light => tr!("Light"),
        }
    }

    /// The theme `Serialize` writes under `key`: the variant's own name, never its label, which is in the language in use.
    fn from_key(key: &str) -> Option<Self> {
        match key {
            "Dark" => Some(Self::Dark),
            "Light" => Some(Self::Light),
            _ => None,
        }
    }
}

/// The interface font. A system font is named, and found on disk when applied.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FontChoice {
    #[default]
    Default,
    Monospace,
    System(String),
}

impl FontChoice {
    const SYSTEM_PREFIX: &'static str = "system:";

    pub fn key(&self) -> String {
        match self {
            Self::Default => "default".to_string(),
            Self::Monospace => "monospace".to_string(),
            Self::System(name) => format!("{}{name}", Self::SYSTEM_PREFIX),
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        match key {
            "default" => Some(Self::Default),
            "monospace" => Some(Self::Monospace),
            _ => key
                .strip_prefix(Self::SYSTEM_PREFIX)
                .filter(|name| !name.trim().is_empty())
                .map(|name| Self::System(name.to_string())),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Default => tr!("Default (Inter)").to_string(),
            Self::Monospace => tr!("Monospace (Hack)").to_string(),
            Self::System(name) => name.clone(),
        }
    }
}

/// How the editor looks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppearancePrefs {
    pub theme: ThemeChoice,
    #[serde(serialize_with = "hex_rgb")]
    pub accent: [u8; 3],
    /// ⚠ **A whole percentage, not an `f32`.** `serde_json` writes a
    /// non-finite float as `null` and then refuses to read it back, which once
    /// cost every preference in the file. An integer has no such value.
    pub ui_scale_percent: u16,
    #[serde(serialize_with = "font_key")]
    pub font: FontChoice,
    /// Whether what egui animates changes at once instead.
    pub reduce_motion: bool,
}

impl AppearancePrefs {
    pub const DEFAULT_ACCENT: [u8; 3] = [0x00, 0x78, 0xd4];
    pub const SCALE_STEPS: [u16; 8] = [75, 90, 100, 110, 125, 150, 175, 200];

    /// The zoom factor egui multiplies the platform's own scale by.
    pub fn scale(&self) -> f32 {
        f32::from(self.ui_scale_percent) / 100.0
    }

    fn scale_is_allowed(percent: u64) -> bool {
        let steps = Self::SCALE_STEPS;
        (u64::from(steps[0])..=u64::from(steps[steps.len() - 1])).contains(&percent)
    }

    /// ⚠ **Field by field, so an unreadable setting costs only itself.** A
    /// derived `Deserialize` fails the whole `Prefs` on one bad value, and
    /// `load_prefs_from` answers a failed parse with defaults for everything —
    /// game paths, shortcuts, layout. Here a value that does not read is
    /// logged and replaced by its own default.
    fn from_value(value: &serde_json::Value) -> Self {
        let mut out = Self::default();
        let Some(map) = value.as_object() else {
            if !value.is_null() {
                tracing::warn!(found = %value, "appearance settings are not an object; using defaults");
            }
            return out;
        };
        let refuse = |field: &str, found: &serde_json::Value| {
            tracing::warn!(field, found = %found, "unreadable appearance setting; using its default");
        };
        if let Some(v) = map.get("theme") {
            match v.as_str().and_then(ThemeChoice::from_key) {
                Some(theme) => out.theme = theme,
                None => refuse("theme", v),
            }
        }
        if let Some(v) = map.get("accent") {
            match v.as_str().and_then(parse_hex_rgb) {
                Some(rgb) => out.accent = rgb,
                None => refuse("accent", v),
            }
        }
        if let Some(v) = map.get("ui_scale_percent") {
            match v.as_u64().filter(|p| Self::scale_is_allowed(*p)) {
                Some(p) => out.ui_scale_percent = p as u16,
                None => refuse("ui_scale_percent", v),
            }
        }
        if let Some(v) = map.get("font") {
            match v.as_str().and_then(FontChoice::from_key) {
                Some(font) => out.font = font,
                None => refuse("font", v),
            }
        }
        if let Some(v) = map.get("reduce_motion") {
            match v.as_bool() {
                Some(reduce) => out.reduce_motion = reduce,
                None => refuse("reduce_motion", v),
            }
        }
        out
    }
}

impl Default for AppearancePrefs {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::default(),
            accent: Self::DEFAULT_ACCENT,
            ui_scale_percent: 100,
            font: FontChoice::default(),
            reduce_motion: false,
        }
    }
}

fn hex_rgb<S: serde::Serializer>(rgb: &[u8; 3], s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]))
}

fn parse_hex_rgb(text: &str) -> Option<[u8; 3]> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

fn font_key<S: serde::Serializer>(font: &FontChoice, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&font.key())
}

fn readable_appearance<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<AppearancePrefs, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(AppearancePrefs::from_value(&value))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedBinding {
    /// The platform's primary modifier: Command on a Mac, Control elsewhere.
    #[serde(default)]
    pub ctrl: bool,
    /// The Control key itself on a Mac, where it is not the primary modifier; never set elsewhere.
    #[serde(default)]
    pub control: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
    pub key: String,
}

pub type ShortcutBindings = BTreeMap<String, PersistedBinding>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GtaKeysPrefs {
    #[serde(default)]
    pub gta5_path: Option<PathBuf>,
    #[serde(default)]
    pub gta5_sha1_hex: Option<String>,
    #[serde(default)]
    pub gta5_size: Option<u64>,
    #[serde(default)]
    pub extracted_at: Option<i64>,
}

impl GtaKeysPrefs {
    pub fn forget(&mut self) {
        self.gta5_sha1_hex = None;
        self.gta5_size = None;
        self.extracted_at = None;
    }
}

/// A file opened lately, on disk or inside what holds it, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentFile {
    /// The file on disk, or the archive, install or folder holding it.
    pub source: String,
    /// `/`-separated within `source`; empty for a file of its own.
    #[serde(default)]
    pub inner: String,
    /// Seconds since the Unix epoch; `None` for a file remembered before this was kept.
    #[serde(default)]
    pub opened: Option<u64>,
}

impl RecentFile {
    /// The file's own name.
    pub fn name(&self) -> &str {
        match self
            .inner
            .rsplit('/')
            .next()
            .filter(|leaf| !leaf.is_empty())
        {
            Some(leaf) => leaf,
            None => Path::new(&self.source)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&self.source),
        }
    }

    /// Where it is, in full.
    pub fn location(&self) -> String {
        if self.inner.is_empty() {
            self.source.clone()
        } else {
            format!("{}/{}", self.source, self.inner)
        }
    }

    /// Whether what holds it is still on disk.
    pub fn exists(&self) -> bool {
        Path::new(&self.source).exists()
    }

    /// How long ago it was opened, as "5m ago".
    pub fn opened_ago(&self) -> Option<String> {
        let opened = std::time::UNIX_EPOCH + std::time::Duration::from_secs(self.opened?);
        Some(ago(opened)).filter(|ago| !ago.is_empty())
    }
}

#[path = "prefs_ago.rs"]
mod prefs_ago;
pub use prefs_ago::ago;

/// ⚠ Entry by entry, so one that does not read is one fewer; a path alone is a file on disk remembered before 6.
fn readable_recent_files<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<RecentFile>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    let entries = value.as_array().map(Vec::as_slice).unwrap_or_default();
    Ok(entries
        .iter()
        .filter_map(|entry| match entry.as_str() {
            Some(path) => Some(RecentFile {
                source: path.to_string(),
                inner: String::new(),
                opened: None,
            }),
            None => serde_json::from_value(entry.clone()).ok(),
        })
        .collect())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prefs {
    #[serde(default = "default_schema")]
    pub schema: u32,
    /// ⚠ Not under `recent_files`, the paths a build before 6 kept: an object
    /// there would fail such a build's whole read of this file.
    #[serde(
        default,
        rename = "recents",
        deserialize_with = "readable_recent_files"
    )]
    pub recent_files: Vec<RecentFile>,
    /// What a build before 6 kept, read once into `recent_files`.
    #[serde(
        default,
        rename = "recent_files",
        skip_serializing,
        deserialize_with = "readable_recent_files"
    )]
    pub remembered_before_6: Vec<RecentFile>,
    #[serde(default)]
    pub recent_projects: Vec<PathBuf>,
    #[serde(default)]
    pub recent_folders: Vec<PathBuf>,
    #[serde(default)]
    pub gta_keys: GtaKeysPrefs,
    #[serde(default)]
    pub rpf_output_mode: RpfOutputMode,
    #[serde(default)]
    pub export_fixes: ExportFixes,
    #[serde(default)]
    pub active_game: Option<Game>,
    #[serde(default)]
    pub game_paths: GamePaths,
    #[serde(default)]
    pub editor_window: EditorWindowPrefs,
    #[serde(default)]
    pub edit_mode: EditMode,
    /// Read the `mods` overlay's copy of an archive where it has one.
    ///
    /// ⚠ **`default_true`.** A missing field would otherwise deserialise to
    /// `false` and silently stop reading an overlay the user can see in
    /// Explorer.
    #[serde(default = "default_true")]
    pub read_overlay_first: bool,
    /// Folders searched for a `.ytd` a drawable names and no open document
    /// holds, most preferred first.
    ///
    /// ⚠ **Order is the whole feature.** Two folders can hold a dictionary of
    /// the same name — a mod's and the base game's are the usual pair — and
    /// which one wins is the question the user is answering by sorting this
    /// list. A set would lose it.
    #[serde(default)]
    pub texture_search_paths: Vec<PathBuf>,
    #[serde(default, deserialize_with = "readable_viewport")]
    pub viewport: ViewportPrefs,
    #[serde(default)]
    pub shortcut_bindings: ShortcutBindings,
    /// The preset of keys under `shortcut_bindings`, by its id; empty, or an id no preset has, is OpenRage's own.
    #[serde(default)]
    pub shortcut_preset: String,
    /// ⚠ Not kept: what the keyboard in use types, learnt from the presses of this run.
    #[serde(skip)]
    pub keys_seen: crate::ui::shortcuts::KeysSeen,
    /// The dock arrangement per workspace, as the user last left it.
    ///
    /// ⚠ **Versioned separately from `schema` above**, because this one is a
    /// third party's format: `egui_dock` decides what a `DockState` looks
    /// like and can change it on a minor bump, at which point every other
    /// preference in this file is still perfectly readable. See
    /// `src/ui/dock_layout.rs`.
    #[serde(default, deserialize_with = "crate::ui::dock_layout::readable")]
    pub dock_layouts: crate::ui::dock_layout::SavedLayouts,
    #[serde(default, deserialize_with = "readable_appearance")]
    pub appearance: AppearancePrefs,
    #[serde(default, deserialize_with = "crate::state::explorer::readable")]
    pub explorer: crate::state::explorer::ExplorerPrefs,
    /// The decoded texture preview cache's eviction threshold.
    #[serde(
        default = "default_texture_cache_mib",
        deserialize_with = "readable_texture_cache_mib"
    )]
    pub texture_cache_mib: u32,
    /// The viewport's GPU texture eviction threshold.
    #[serde(
        default = "default_gpu_texture_mib",
        deserialize_with = "readable_gpu_texture_mib"
    )]
    pub gpu_texture_mib: u32,
    /// What the user answered each plugin, by plugin id.
    #[serde(default, deserialize_with = "crate::state::plugins::readable_consent")]
    pub plugin_consent: std::collections::BTreeMap<String, crate::state::plugins::Consent>,
    /// What the user chose for the settings each plugin declares, by plugin id and setting id.
    #[serde(
        default,
        deserialize_with = "crate::state::plugins::surface::readable_plugin_settings"
    )]
    pub plugin_settings:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, serde_json::Value>>,
    /// The load order of each install's mods, by `or_title::mods::install_key`.
    #[serde(
        default,
        deserialize_with = "crate::state::mod_manager::readable_mod_orders"
    )]
    pub mod_orders: std::collections::BTreeMap<String, Vec<String>>,
    /// The Animation preview's Draw the skeleton file's own model, as the user last left it.
    ///
    /// ⚠ **`default_true`**, as `read_overlay_first` is: a missing field would otherwise hide the
    /// model of every file a user opens.
    #[serde(default = "default_true")]
    pub draw_skeleton_model: bool,
    /// The inspector's Show internals, as the user last left it.
    #[serde(default)]
    pub show_internals: bool,
    /// The project that was open when OpenRage last ran; none once it was closed.
    #[serde(default)]
    pub last_project: Option<PathBuf>,
    /// Whether a launch reopens [`Self::last_project`].
    ///
    /// ⚠ **`default_true`**: a missing field would otherwise turn the owner's ruling off.
    #[serde(default = "default_true")]
    pub reopen_last_project: bool,
    /// When each recent project was opened, in seconds since the epoch, by the path as text.
    #[serde(default)]
    pub project_opened: BTreeMap<String, u64>,
    /// The code of the interface language the person chose in Settings; none follows the system's own language.
    #[serde(default, deserialize_with = "readable_language")]
    pub language: Option<String>,
    /// The World screen's hour, weather, modes, level-of-detail scale and speed, as the person last left them.
    #[serde(default)]
    pub world_viewer: crate::ui::panels::world_viewer_prefs::WorldViewerPrefs,
}

/// ⚠ A value that is not a text is no choice, and costs only itself.
fn readable_language<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(value.as_str().map(str::to_owned))
}

fn default_schema() -> u32 {
    PREFS_SCHEMA
}

/// A 2048×2048 RGBA8 upload is 16 MiB, so this holds a few dozen of the largest.
pub const DEFAULT_GPU_TEXTURE_MIB: u32 = 1024;

fn default_gpu_texture_mib() -> u32 {
    DEFAULT_GPU_TEXTURE_MIB
}

fn readable_gpu_texture_mib<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(value
        .as_u64()
        .and_then(|mib| u32::try_from(mib).ok())
        .filter(|&mib| mib > 0)
        .unwrap_or(DEFAULT_GPU_TEXTURE_MIB))
}

fn default_texture_cache_mib() -> u32 {
    crate::state::DEFAULT_BUDGET_MIB
}

fn readable_texture_cache_mib<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(value
        .as_u64()
        .and_then(|mib| u32::try_from(mib).ok())
        .filter(|&mib| mib > 0)
        .unwrap_or_else(default_texture_cache_mib))
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            schema: PREFS_SCHEMA,
            recent_files: Vec::new(),
            remembered_before_6: Vec::new(),
            recent_projects: Vec::new(),
            recent_folders: Vec::new(),
            gta_keys: GtaKeysPrefs::default(),
            rpf_output_mode: RpfOutputMode::default(),
            export_fixes: ExportFixes::default(),
            active_game: None,
            game_paths: GamePaths::default(),
            editor_window: EditorWindowPrefs::default(),
            edit_mode: EditMode::default(),
            read_overlay_first: true,
            texture_search_paths: Vec::new(),
            viewport: ViewportPrefs::default(),
            shortcut_bindings: ShortcutBindings::default(),
            shortcut_preset: String::new(),
            keys_seen: Default::default(),
            dock_layouts: crate::ui::dock_layout::SavedLayouts::default(),
            appearance: AppearancePrefs::default(),
            explorer: crate::state::explorer::ExplorerPrefs::default(),
            texture_cache_mib: default_texture_cache_mib(),
            gpu_texture_mib: DEFAULT_GPU_TEXTURE_MIB,
            plugin_consent: Default::default(),
            plugin_settings: Default::default(),
            mod_orders: Default::default(),
            draw_skeleton_model: true,
            show_internals: false,
            last_project: None,
            reopen_last_project: true,
            project_opened: BTreeMap::new(),
            language: None,
            world_viewer: Default::default(),
        }
    }
}

impl Prefs {
    /// Put `place` first among the recent files, opened now, and drop those gone from disk.
    pub fn touch_recent_file(&mut self, place: Place) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
        self.recent_files
            .retain(|r| (r.source != place.source || r.inner != place.inner) && r.exists());
        self.recent_files.insert(
            0,
            RecentFile {
                source: place.source,
                inner: place.inner,
                opened: now.ok().map(|d| d.as_secs()),
            },
        );
        self.recent_files.truncate(MAX_RECENT);
    }

    /// Put `path` first among the recent projects, opened now, and remember it as the one to reopen.
    pub fn touch_recent_project(&mut self, path: PathBuf) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
        if let Ok(now) = now {
            self.project_opened
                .insert(path.to_string_lossy().into_owned(), now.as_secs());
        }
        self.last_project = Some(path.clone());
        push_mru(&mut self.recent_projects, path);
        self.forget_when_opened_of_the_forgotten();
    }

    /// Drop `path` from the recent projects, and from the one to reopen; the project's files are not touched.
    pub fn forget_recent_project(&mut self, path: &Path) {
        self.recent_projects.retain(|p| p != path);
        if self.last_project.as_deref() == Some(path) {
            self.last_project = None;
        }
        self.forget_when_opened_of_the_forgotten();
    }

    /// When `path` was opened, as "3d ago".
    pub fn project_opened_ago(&self, path: &Path) -> Option<String> {
        let at = *self.project_opened.get(&*path.to_string_lossy())?;
        let opened = std::time::UNIX_EPOCH + std::time::Duration::from_secs(at);
        Some(ago(opened)).filter(|ago| !ago.is_empty())
    }

    fn forget_when_opened_of_the_forgotten(&mut self) {
        let kept: std::collections::HashSet<String> = self
            .recent_projects
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        self.project_opened.retain(|path, _| kept.contains(path));
    }

    pub fn touch_recent_folder(&mut self, path: PathBuf) {
        push_mru(&mut self.recent_folders, path);
    }

    pub fn clear_recent_files(&mut self) {
        self.recent_files.clear();
    }

    pub fn clear_recent_projects(&mut self) {
        self.recent_projects.clear();
        self.project_opened.clear();
    }

    pub fn clear_recent_folders(&mut self) {
        self.recent_folders.clear();
    }

    pub fn active_game_root(&self) -> Option<&PathBuf> {
        self.active_game.and_then(|g| self.game_paths.get(g))
    }

    /// How every mount this application builds should resolve its archives.
    pub fn mount_options(&self) -> or_title::MountOptions {
        if self.read_overlay_first {
            or_title::MountOptions::default()
        } else {
            or_title::MountOptions::install_only()
        }
    }

    pub fn texture_cache_budget(&self) -> usize {
        self.texture_cache_mib as usize * 1024 * 1024
    }

    pub fn gpu_texture_budget(&self) -> usize {
        self.gpu_texture_mib as usize * 1024 * 1024
    }

    /// Bring a file written by any schema up to this one.
    ///
    /// ⚠ **A newer schema is read, not refused**, and saved back as this one:
    /// refusing would reset the user's game paths for having run a newer
    /// build once, and keeping the newer number would tell that build this
    /// file carries fields it no longer does.
    fn migrate(&mut self) {
        if self.schema > PREFS_SCHEMA {
            tracing::warn!(
                found = self.schema,
                understood = PREFS_SCHEMA,
                "prefs.json was written by a newer OpenRage; settings this build does not know are kept as they are on the next save"
            );
        }
        self.migrate_from_gta5_path();
        if self.recent_files.is_empty() {
            self.recent_files = std::mem::take(&mut self.remembered_before_6);
        }
        self.schema = PREFS_SCHEMA;
    }

    fn migrate_from_gta5_path(&mut self) {
        if self.active_game.is_some() && self.game_paths.get(Game::GtaVLegacy).is_some() {
            return;
        }
        let Some(exe) = self.gta_keys.gta5_path.as_ref() else {
            return;
        };
        let Some(parent) = exe.parent() else {
            return;
        };
        if self.game_paths.get(Game::GtaVLegacy).is_none() {
            self.game_paths
                .set(Game::GtaVLegacy, Some(parent.to_path_buf()));
        }
        if self.active_game.is_none() {
            self.active_game = Some(Game::GtaVLegacy);
        }
    }
}

fn push_mru(list: &mut Vec<PathBuf>, path: PathBuf) {
    list.retain(|p| p != &path);
    list.insert(0, path);
    list.truncate(MAX_RECENT);
}

/// Where everything this application remembers lives.
///
/// ⚠ **`OPENRAGE_CONFIG_DIR` redirects it.** Running the editor writes
/// preferences, so an unredirected run edits the machine owner's game paths,
/// shortcuts and recent files. Every path below goes through here so one
/// variable covers them all.
pub fn config_root() -> Option<PathBuf> {
    if let Some(scratch) = test_config_root() {
        return Some(scratch);
    }
    if let Some(dir) = or_title::config::config_dir_override() {
        return Some(dir);
    }
    self_check_scratch().or_else(real_config_root)
}

/// Where a real run of the editor keeps its state.
pub fn real_config_root() -> Option<PathBuf> {
    or_title::config::user_config_dir()
}

/// ⚠ **A `--self-check` run must not write the machine owner's preferences.**
/// The check constructs the real [`crate::state::AppState`], which saves
/// preferences, so without this every run rewrites `prefs.json`.
///
/// Keyed on this process's own arguments rather than exported by the justfile
/// recipe, because a bare `cargo run -p openrage --bin openrage -- --self-check` never passes
/// through the recipe.
///
/// Ranked **below** `OPENRAGE_CONFIG_DIR`, so a run can still be pointed at a
/// directory somebody wants to read afterwards: the scratch is removed with the
/// process, an asked-for directory is not.
fn self_check_scratch() -> Option<PathBuf> {
    if !std::env::args_os().any(|a| a == "--self-check") {
        return None;
    }
    Some(self_check_scratch_at(
        std::env::temp_dir(),
        std::process::id(),
    ))
}

/// Named by process id rather than at random: a run that is killed before its
/// own cleanup leaves one directory behind instead of one per attempt.
fn self_check_scratch_at(temp: PathBuf, pid: u32) -> PathBuf {
    temp.join(format!("openrage-self-check-{pid}"))
}

/// The directory a `--self-check` run resolved to, for the run to report and
/// then take down. `None` when the run was pointed somewhere on purpose.
pub fn self_check_scratch_to_clean() -> Option<PathBuf> {
    self_check_scratch().filter(|scratch| config_root().as_ref() == Some(scratch))
}

#[cfg(test)]
thread_local! {
    static CONFIG_ROOT_HERE: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// A test that holds the configuration folder, to seed its preferences or read them back, calls this
/// first: from then on this thread's configuration is `dir`'s. A test runs on a thread of its own,
/// so nothing is undone afterwards.
#[cfg(test)]
pub fn keep_prefs_in(dir: &Path) {
    CONFIG_ROOT_HERE.with(|here| *here.borrow_mut() = Some(dir.to_path_buf()));
}

/// ⚠ **The test binary must not write the developer's preferences.**
/// `AppState::default()` reaches `save_prefs` and dozens of tests construct
/// one, so without this the suite rewrites the real `prefs.json`. One
/// directory per test process, removed with it.
#[cfg(test)]
fn test_config_root() -> Option<PathBuf> {
    if let Some(here) = CONFIG_ROOT_HERE.with(|here| here.borrow().clone()) {
        return Some(here);
    }
    super::test_scratch::root()
}

#[cfg(not(test))]
fn test_config_root() -> Option<PathBuf> {
    None
}

pub fn prefs_path() -> Option<PathBuf> {
    prefs_root().map(|d| d.join("prefs.json"))
}

/// Where `prefs.json` lives: the configuration root, and in a test build a folder of this thread's own.
///
/// ⚠ **Every test of the binary shares one scratch configuration, and `AppState::default()` reads
/// `prefs.json` from it**: a test that saved a preference, or finished a save, decided what every
/// later test started with.
#[cfg(test)]
fn prefs_root() -> Option<PathBuf> {
    if let Some(here) = CONFIG_ROOT_HERE.with(|here| here.borrow().clone()) {
        return Some(here);
    }
    let thread = std::thread::current().id();
    super::test_scratch::root().map(|scratch| scratch.join(format!("prefs-{thread:?}")))
}

#[cfg(not(test))]
fn prefs_root() -> Option<PathBuf> {
    config_root()
}

pub fn keys_cache_path_for(game: Game) -> Option<PathBuf> {
    let name = game.profile().keys_cache_name().unwrap_or("gta5_keys.bin");
    config_root().map(|d| d.join(name))
}

pub fn keys_cache_path_for_exe(exe: &std::path::Path) -> Option<PathBuf> {
    // These are GTA V's keys: an executable in a folder no profile claims keeps Legacy's cache.
    let game = exe
        .parent()
        .and_then(Game::detect_from_root)
        .unwrap_or(Game::GtaVLegacy);
    keys_cache_path_for(game)
}

pub fn encrypt_cache_path() -> Option<PathBuf> {
    config_root().map(|d| d.join("gta5_ng_encrypt.bin"))
}

/// The user's own key snapshot beside the keys cache, or else the bundled one.
pub fn user_snapshot() -> or_io::container::rpf::crypto::SnapshotSource {
    or_io::container::rpf::crypto::SnapshotSource::discover(config_root().as_deref()).or_bundled()
}

pub fn load_prefs() -> Prefs {
    let path = match prefs_path() {
        Some(p) => p,
        None => return Prefs::default(),
    };
    load_prefs_from(&path)
}

pub fn load_prefs_from(path: &Path) -> Prefs {
    let (prefs, parsed) = read_prefs(path);
    // ⚠ Defaults that stand for a file this build could not parse are not what the file holds:
    // noted as known, they would leave the bad key on disk at every save and never heal it.
    if parsed {
        note_known(path, &prefs);
    }
    prefs
}

/// The preferences in the file, and whether that is what the file holds: false when it is there and does not parse.
pub(super) fn read_prefs(path: &Path) -> (Prefs, bool) {
    match std::fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<Prefs>(without_bom(&bytes)) {
            Ok(mut prefs) => {
                prefs.migrate();
                (prefs, true)
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "Failed to parse prefs; using defaults");
                (Prefs::default(), false)
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Prefs::default(), true),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "Failed to read prefs; using defaults");
            (Prefs::default(), true)
        }
    }
}

/// ⚠ **A UTF-8 BOM makes `serde_json` reject the whole file**, and the whole
/// file is every preference the user has — so a text editor that added three
/// bytes on Windows silently reset their game paths, their shortcuts and
/// their layout at once. Measured by hand-editing `prefs.json` with
/// PowerShell's `Set-Content -Encoding utf8`, which writes one.
fn without_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)
}

// ⚠ **Per thread, because the test binary runs tests on parallel threads**;
// a process-wide count would read every other test's writes as this one's.
#[cfg(test)]
thread_local! {
    static WRITES_ON_THIS_THREAD: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many times this thread has asked for `prefs.json` to be written.
#[cfg(test)]
pub fn writes_on_this_thread() -> usize {
    WRITES_ON_THIS_THREAD.with(|w| w.get())
}

pub fn save_prefs(prefs: &Prefs) {
    #[cfg(test)]
    WRITES_ON_THIS_THREAD.with(|w| w.set(w.get() + 1));
    let path = match prefs_path() {
        Some(p) => p,
        None => return,
    };
    save_prefs_to(&path, prefs);
}

/// What this process last read from, or wrote to, each preferences file: the base of the merge a save makes.
///
/// ⚠ **A second window is a second process on the same file.** A save that wrote this process's whole
/// state would undo whatever the other one saved since this one read it, so a save reads the file
/// again and writes over it only the keys this process changed since it last read or wrote.
static KNOWN: std::sync::Mutex<BTreeMap<PathBuf, serde_json::Map<String, serde_json::Value>>> =
    std::sync::Mutex::new(BTreeMap::new());

fn as_object(prefs: &Prefs) -> Option<serde_json::Map<String, serde_json::Value>> {
    match serde_json::to_value(prefs) {
        Ok(serde_json::Value::Object(keys)) => Some(keys),
        _ => None,
    }
}

fn note_known(path: &Path, prefs: &Prefs) {
    if let (Some(keys), Ok(mut known)) = (as_object(prefs), KNOWN.lock()) {
        known.insert(path.to_path_buf(), keys);
    }
}

/// `mine`, with every key this process did not change since it last read or wrote the file taken from the file as it is now.
fn merged_with_the_file(
    path: &Path,
    mine: serde_json::Map<String, serde_json::Value>,
) -> serde_json::Map<String, serde_json::Value> {
    let known = KNOWN.lock().ok().and_then(|known| known.get(path).cloned());
    let on_disk = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(without_bom(&bytes)).ok());
    let (Some(known), Some(serde_json::Value::Object(mut merged))) = (known, on_disk) else {
        return mine;
    };
    for (key, value) in mine {
        // ⚠ The schema is always this build's: a file saved back under a newer number would tell that build it carries fields it no longer does.
        if key == "schema" || known.get(&key) != Some(&value) {
            merged.insert(key, value);
        }
    }
    merged
}

pub fn save_prefs_to(path: &Path, prefs: &Prefs) {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!(path = %parent.display(), error = %e, "Failed to create prefs dir");
            return;
        }
    }
    let Some(mine) = as_object(prefs) else {
        tracing::warn!("Failed to serialize prefs");
        return;
    };
    let merged = merged_with_the_file(path, mine.clone());
    match serde_json::to_vec_pretty(&merged) {
        Ok(bytes) => match atomic_write(path, &bytes) {
            Ok(()) => {
                if let Ok(mut known) = KNOWN.lock() {
                    known.insert(path.to_path_buf(), mine);
                }
            }
            Err(e) => tracing::warn!(path = %path.display(), error = %e, "Failed to write prefs"),
        },
        Err(e) => {
            tracing::warn!(error = %e, "Failed to serialize prefs");
        }
    }
}

/// ⚠ A temporary file of its own for each process, or two windows saving at once write one file.
fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, data)?;
    // A rename replaces the file in one step where the platform allows it, so another process
    // starting now never finds none.
    if std::fs::rename(&tmp, path).is_ok() {
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Falsified by deleting the `cfg(test)` arm: the suite then writes the
    /// real file.
    #[test]
    fn the_suite_cannot_reach_the_developers_own_preferences() {
        let root = config_root().expect("a scratch root");
        let real = dirs::config_dir().map(|d| d.join("openrage"));
        assert_ne!(
            Some(root.clone()),
            real,
            "the test binary is pointed at the real configuration directory"
        );
        assert!(
            !root.starts_with(real.unwrap_or_else(|| PathBuf::from("\0"))),
            "{} is inside the real configuration directory",
            root.display()
        );
    }

    /// ⚠ The suite cannot reach this through [`config_root`], because the
    /// `cfg(test)` arm above outranks it — so the placement is asserted on
    /// the helper, and `self_check::checked_config_root` is what stands at
    /// the point of harm in a real `--self-check` process.
    #[test]
    fn a_self_check_writes_somewhere_that_is_not_the_owners_configuration() {
        let scratch = self_check_scratch_at(std::env::temp_dir(), 4242);
        assert!(scratch.ends_with("openrage-self-check-4242"));

        let real = real_config_root().expect("a configuration directory");
        assert_ne!(scratch, real);
        assert!(
            !scratch.starts_with(&real),
            "{} is inside {}",
            scratch.display(),
            real.display()
        );
    }

    /// Threads, as libtest runs two tests at once in one process.
    #[test]
    fn a_test_that_saves_a_preference_leaves_the_next_default_state_as_it_was() {
        let asked_for = Prefs::default().appearance.ui_scale_percent + 50;
        std::thread::spawn(move || {
            let mut state = crate::state::AppState::default();
            state.prefs.appearance.ui_scale_percent = asked_for;
            save_prefs(&state.prefs);
            assert_eq!(
                crate::state::AppState::default()
                    .prefs
                    .appearance
                    .ui_scale_percent,
                asked_for
            );
        })
        .join()
        .expect("the saving test ran");

        let next = std::thread::spawn(|| crate::state::AppState::default().prefs.appearance)
            .join()
            .expect("the next test ran");
        assert_eq!(
            next.ui_scale_percent,
            Prefs::default().appearance.ui_scale_percent,
            "the next test started with what an earlier one saved"
        );
    }

    fn prefs_with_a_game() -> Prefs {
        let mut prefs = Prefs::default();
        prefs.touch_recent_file(Place::new(Path::new("weapons.ytd"), "").expect("a path"));
        prefs.active_game = Some(Game::GtaVLegacy);
        prefs
            .game_paths
            .set(Game::GtaVLegacy, Some(PathBuf::from("D:/Games/GTA V")));
        prefs
    }

    fn recent_sources(prefs: &Prefs) -> Vec<&str> {
        prefs
            .recent_files
            .iter()
            .map(|r| r.source.as_str())
            .collect()
    }

    #[test]
    fn recent_files_kept_before_6_read_and_are_written_where_that_build_does_not_look() {
        let read = load_json(&serde_json::json!({ "schema": 5, "recent_files": ["a.ypt"] }));
        assert_eq!(recent_sources(&read), ["a.ypt"]);
        let written = serde_json::to_value(&read).expect("prefs serialise");
        assert!(
            written.get("recent_files").is_none(),
            "a build before 6 reads that key as paths and would refuse the whole file"
        );
        assert_eq!(written["recents"][0]["source"], "a.ypt");
    }

    fn load_json(json: &serde_json::Value) -> Prefs {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("prefs.json");
        std::fs::write(&path, serde_json::to_vec(json).expect("json")).expect("write");
        load_prefs_from(&path)
    }

    fn assert_the_game_survived(prefs: &Prefs) {
        assert_eq!(prefs.active_game, Some(Game::GtaVLegacy));
        assert_eq!(
            prefs.game_paths.get(Game::GtaVLegacy),
            Some(&PathBuf::from("D:/Games/GTA V")),
            "the game path was reset along with the setting"
        );
        assert_eq!(recent_sources(prefs), ["weapons.ytd"]);
    }

    #[test]
    fn an_appearance_survives_a_restart() {
        let mut prefs = prefs_with_a_game();
        prefs.appearance = AppearancePrefs {
            theme: ThemeChoice::Light,
            accent: [0xd0, 0x4a, 0x9a],
            ui_scale_percent: 150,
            font: FontChoice::System("Segoe UI".into()),
            reduce_motion: true,
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("prefs.json");
        save_prefs_to(&path, &prefs);

        let written: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("written")).expect("json");
        assert_eq!(
            written["appearance"],
            serde_json::json!({
                "theme": "Light",
                "accent": "#d04a9a",
                "ui_scale_percent": 150,
                "font": "system:Segoe UI",
                "reduce_motion": true,
            })
        );

        let read = load_prefs_from(&path);
        assert_eq!(read.appearance, prefs.appearance);
        assert_the_game_survived(&read);
    }

    /// ⚠ The file names a theme by a key of its own, never by the label of the language in use: a launch reads the file before
    /// the language is chosen, and a French label would never match `Light`.
    #[test]
    fn a_saved_theme_reads_back_whatever_language_is_in_use() {
        let _french = crate::i18n::pin("fr");
        for theme in ThemeChoice::ALL {
            let mut prefs = Prefs::default();
            prefs.appearance.theme = theme;
            let dir = tempfile::tempdir().expect("tempdir");
            let path = dir.path().join("prefs.json");
            save_prefs_to(&path, &prefs);
            assert_eq!(
                load_prefs_from(&path).appearance.theme,
                theme,
                "{theme:?} read back in French"
            );
        }
    }

    /// ⚠ Falsified by reading `appearance` with a derived `Deserialize`: the
    /// `null` then fails the whole file and the game path goes with it.
    #[test]
    fn an_unreadable_appearance_setting_costs_only_that_setting() {
        let mut json = serde_json::to_value(prefs_with_a_game()).expect("json");
        json["appearance"] = serde_json::json!({
            "theme": "Light",
            "accent": "not a colour",
            "ui_scale_percent": null,
            "font": 42,
        });
        let read = load_json(&json);

        assert_the_game_survived(&read);
        assert_eq!(
            read.appearance.theme,
            ThemeChoice::Light,
            "the readable field was kept"
        );
        let defaults = AppearancePrefs::default();
        assert_eq!(read.appearance.accent, defaults.accent);
        assert_eq!(read.appearance.ui_scale_percent, defaults.ui_scale_percent);
        assert_eq!(read.appearance.font, defaults.font);
    }

    #[test]
    fn a_scale_outside_the_offered_range_is_refused_not_applied() {
        for bad in [
            serde_json::json!(0),
            serde_json::json!(5000),
            serde_json::json!(1.5),
        ] {
            let mut json = serde_json::to_value(prefs_with_a_game()).expect("json");
            json["appearance"] = serde_json::json!({ "ui_scale_percent": bad });
            let read = load_json(&json);
            assert_eq!(read.appearance.ui_scale_percent, 100, "{bad} was applied");
            assert_the_game_survived(&read);
        }
    }

    #[test]
    fn an_appearance_that_is_not_an_object_leaves_everything_else() {
        for bad in [
            serde_json::json!("garbage"),
            serde_json::json!(42),
            serde_json::json!(null),
            serde_json::json!([1, 2]),
        ] {
            let mut json = serde_json::to_value(prefs_with_a_game()).expect("json");
            json["appearance"] = bad.clone();
            let read = load_json(&json);
            assert_eq!(read.appearance, AppearancePrefs::default(), "{bad}");
            assert_the_game_survived(&read);
        }
    }

    #[test]
    fn a_schema_4_file_reads_with_the_default_appearance_and_is_brought_forward() {
        let mut json = serde_json::to_value(prefs_with_a_game()).expect("json");
        let map = json.as_object_mut().expect("an object");
        map.remove("appearance");
        map.insert("schema".into(), serde_json::json!(4));

        let read = load_json(&json);
        assert_eq!(read.schema, PREFS_SCHEMA);
        assert_eq!(read.appearance, AppearancePrefs::default());
        assert_the_game_survived(&read);
    }

    #[test]
    fn an_absent_or_unreadable_texture_cache_budget_costs_only_itself() {
        let mut json = serde_json::to_value(prefs_with_a_game()).expect("json");
        json.as_object_mut()
            .expect("an object")
            .remove("texture_cache_mib");
        assert_eq!(
            load_json(&json).texture_cache_mib,
            crate::state::DEFAULT_BUDGET_MIB
        );

        for bad in [serde_json::json!("big"), serde_json::json!(0)] {
            json["texture_cache_mib"] = bad;
            let read = load_json(&json);
            assert_eq!(read.texture_cache_mib, crate::state::DEFAULT_BUDGET_MIB);
            assert_the_game_survived(&read);
        }
    }

    #[test]
    fn an_absent_or_unreadable_gpu_texture_budget_costs_only_itself() {
        let mut json = serde_json::to_value(prefs_with_a_game()).expect("json");
        json.as_object_mut()
            .expect("an object")
            .remove("gpu_texture_mib");
        assert_eq!(load_json(&json).gpu_texture_mib, DEFAULT_GPU_TEXTURE_MIB);

        for bad in [serde_json::json!("big"), serde_json::json!(0)] {
            json["gpu_texture_mib"] = bad;
            let read = load_json(&json);
            assert_eq!(read.gpu_texture_mib, DEFAULT_GPU_TEXTURE_MIB);
            assert_the_game_survived(&read);
        }
    }

    #[test]
    fn a_file_from_a_newer_build_is_read_rather_than_reset() {
        let mut json = serde_json::to_value(prefs_with_a_game()).expect("json");
        json["schema"] = serde_json::json!(PREFS_SCHEMA + 3);
        json["a_setting_from_the_future"] = serde_json::json!({ "x": 1 });

        let read = load_json(&json);
        assert_the_game_survived(&read);
        assert_eq!(
            read.schema, PREFS_SCHEMA,
            "saved back as what this build writes"
        );
    }

    #[test]
    fn a_byte_order_mark_does_not_reset_every_preference() {
        let mut prefs = Prefs::default();
        prefs.touch_recent_file(Place::new(Path::new("a.ypt"), "").expect("a path"));
        let json = serde_json::to_vec(&prefs).expect("prefs serialise");

        let mut with_bom = vec![0xEF, 0xBB, 0xBF];
        with_bom.extend_from_slice(&json);

        assert!(
            serde_json::from_slice::<Prefs>(&with_bom).is_err(),
            "if serde starts tolerating a BOM, `without_bom` is doing nothing \
             and should go"
        );
        let read: Prefs =
            serde_json::from_slice(without_bom(&with_bom)).expect("a BOM is not a parse failure");
        assert_eq!(recent_sources(&read), ["a.ypt"]);
    }

    #[test]
    fn a_favourite_and_the_sort_order_survive_a_restart() {
        use crate::state::explorer::{Place, SortKey, SortOrder};

        let dir = tempfile::tempdir().expect("a scratch directory");
        let path = dir.path().join("prefs.json");
        let mut prefs = Prefs::default();
        let place = Place::new(Path::new("C:/game/x64a.rpf"), "levels/gta5").unwrap();
        prefs.explorer.toggle_favourite(place.clone());
        prefs.explorer.sort = SortOrder {
            key: SortKey::Size,
            descending: true,
        };
        save_prefs_to(&path, &prefs);

        let read = load_prefs_from(&path);
        assert_eq!(read.explorer.favourites, vec![place]);
        assert_eq!(read.explorer.sort, prefs.explorer.sort);
    }

    /// Falsified by deriving `Deserialize` on `ExplorerPrefs` in place of
    /// `deserialize_with`: a bad section then fails the whole document.
    #[test]
    fn an_unreadable_explorer_section_costs_only_itself() {
        for explorer in [
            serde_json::json!(42),
            serde_json::json!({ "favourites": "not a list", "sort": [1, 2] }),
            serde_json::json!({ "favourites": [{ "source": null }] }),
        ] {
            let json = serde_json::json!({
                "recents": ["a.ypt"],
                "explorer": explorer,
            });
            let read: Prefs = serde_json::from_value(json)
                .expect("the explorer section cannot fail the document");
            assert_eq!(recent_sources(&read), ["a.ypt"]);
            assert!(read.explorer.favourites.is_empty());
        }
    }

    /// The second window is a second process: it is simulated by writing the file without
    /// `save_prefs_to`, which is the only thing that remembers what this process read.
    fn another_window_saves(path: &Path, change: impl FnOnce(&mut Prefs)) {
        let mut theirs = read_prefs(path).0;
        change(&mut theirs);
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&theirs).expect("prefs serialise"),
        )
        .expect("the other window's write");
    }

    #[test]
    fn a_save_leaves_what_a_second_window_saved_since_this_one_read_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("prefs.json");
        save_prefs_to(&path, &prefs_with_a_game());
        let mut mine = load_prefs_from(&path);

        another_window_saves(&path, |theirs| {
            theirs.touch_recent_project(PathBuf::from("D:/mods/police/police.openrage-proj"));
        });
        mine.appearance.ui_scale_percent = 150;
        save_prefs_to(&path, &mine);

        let read = load_prefs_from(&path);
        assert_eq!(
            read.appearance.ui_scale_percent, 150,
            "this window's change"
        );
        assert_eq!(
            read.recent_projects,
            [PathBuf::from("D:/mods/police/police.openrage-proj")],
            "the other window's project was written over"
        );
        assert_the_game_survived(&read);
    }

    #[test]
    fn a_setting_both_windows_changed_takes_the_save_made_last() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("prefs.json");
        save_prefs_to(&path, &prefs_with_a_game());
        let mut mine = load_prefs_from(&path);

        another_window_saves(&path, |theirs| theirs.appearance.ui_scale_percent = 125);
        mine.appearance.ui_scale_percent = 150;
        save_prefs_to(&path, &mine);

        assert_eq!(load_prefs_from(&path).appearance.ui_scale_percent, 150);
    }

    #[test]
    fn a_save_keeps_a_setting_from_a_newer_build_and_writes_this_builds_schema() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("prefs.json");
        let mut json = serde_json::to_value(prefs_with_a_game()).expect("json");
        json["schema"] = serde_json::json!(PREFS_SCHEMA + 3);
        json["a_setting_from_the_future"] = serde_json::json!({ "x": 1 });
        std::fs::write(&path, serde_json::to_vec(&json).expect("json")).expect("write");

        let mut mine = load_prefs_from(&path);
        mine.appearance.ui_scale_percent = 150;
        save_prefs_to(&path, &mine);

        let written: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("written")).expect("json");
        assert_eq!(
            written["a_setting_from_the_future"],
            serde_json::json!({ "x": 1 })
        );
        assert_eq!(written["schema"], serde_json::json!(PREFS_SCHEMA));
    }

    /// Falsified by noting the defaults a failed parse returned as what the file holds: the
    /// merge then takes the bad key back from the disk at every save.
    #[test]
    fn a_file_this_build_cannot_parse_is_replaced_on_the_next_save() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("prefs.json");
        let mut json = serde_json::to_value(prefs_with_a_game()).expect("json");
        json["recent_projects"] = serde_json::json!("not a list");
        std::fs::write(&path, serde_json::to_vec(&json).expect("json")).expect("write");

        let mut mine = load_prefs_from(&path);
        assert_eq!(mine.active_game, None, "the file does not parse: defaults");
        mine.show_internals = true;
        save_prefs_to(&path, &mine);

        let read: Prefs = serde_json::from_slice(&std::fs::read(&path).expect("written"))
            .expect("the save left a file this build cannot parse");
        assert!(read.show_internals, "this session's change");
        // The next save of the session merges again, and still leaves the file whole.
        mine.appearance.ui_scale_percent = 150;
        save_prefs_to(&path, &mine);
        assert_eq!(load_prefs_from(&path).appearance.ui_scale_percent, 150);
    }

    #[test]
    fn a_project_opened_is_remembered_with_when_and_forgetting_it_deletes_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = dir.path().join("police.openrage-proj");
        std::fs::write(&project, b"{}").expect("a project");
        let mut prefs = Prefs::default();
        assert!(prefs.reopen_last_project, "reopening is on by default");

        prefs.touch_recent_project(project.clone());
        assert_eq!(prefs.last_project.as_deref(), Some(project.as_path()));
        assert_eq!(
            prefs.project_opened_ago(&project).as_deref(),
            Some("just now")
        );

        prefs.forget_recent_project(&project);
        assert!(prefs.recent_projects.is_empty());
        assert_eq!(
            prefs.last_project, None,
            "a forgotten project is not reopened"
        );
        assert_eq!(prefs.project_opened_ago(&project), None);
        assert!(
            project.is_file(),
            "forgetting a project from a list deletes no file"
        );
    }

    #[test]
    fn a_preferences_file_with_no_project_keys_reads_as_reopening_the_last_project() {
        let read = load_json(&serde_json::json!({ "schema": 6 }));
        assert!(read.reopen_last_project);
        assert_eq!(read.last_project, None);
        assert!(read.project_opened.is_empty());
    }
}
