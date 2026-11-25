use std::sync::Arc;

use anyhow::Result;
use winit::{event_loop::ActiveEventLoop, window::Window};

use super::App;
use crate::state::{AppearancePrefs, FontChoice};
use crate::ui::theme;

pub(super) struct WindowSurface {
    pub(super) window: Arc<Window>,
    pub(super) surface: wgpu::Surface<'static>,
    pub(super) surface_config: wgpu::SurfaceConfiguration,
    pub(super) egui_ctx: egui::Context,
    pub(super) egui_state: egui_winit::State,
    pub(super) egui_renderer: egui_wgpu::Renderer,
    pub(super) appearance: AppearancePrefs,
    /// The code of the language the fonts were ordered for.
    pub(super) language: &'static str,
    /// The frames until the fonts are set again, after a new device: see `on_new_device`.
    pub(super) refont: u8,
}

impl WindowSurface {
    /// Restyle this window if the appearance changed since it was last styled.
    ///
    /// ⚠ Per window, because Settings is its own window with its own context:
    /// a choice made there has to reach the editor's context too.
    pub(super) fn sync_appearance(&mut self, appearance: &AppearancePrefs) {
        // The language too: it orders the fallback fonts.
        let language = crate::i18n::active().code.as_str();
        let again = self.refont == 1;
        self.refont = self.refont.saturating_sub(1);
        if again || &self.appearance != appearance || self.language != language {
            apply_appearance(&self.egui_ctx, appearance);
            self.appearance = appearance.clone();
            self.language = language;
        }
    }
}

impl WindowSurface {
    /// This window's surface and painter on `gpu`, a device made after the one they were on
    /// was lost.
    // The new painter holds none of the context's textures, and a context sends its font atlas
    // whole only for fonts it has not laid out yet: it is given the same fonts with one more
    // entry now, and `sync_appearance` sets its own again the frame after.
    pub(super) fn on_new_device(
        &mut self,
        gpu: &SharedGpu,
        appearance: &AppearancePrefs,
    ) -> Result<()> {
        self.surface = gpu.instance.create_surface(self.window.clone())?;
        self.surface_config.format = gpu.surface_format;
        self.surface.configure(&gpu.device, &self.surface_config);
        self.egui_renderer = egui_wgpu::Renderer::new(
            &gpu.device,
            gpu.surface_format,
            egui_wgpu::RendererOptions::default(),
        );
        let (mut fonts, _) = font_definitions(&appearance.font);
        if let Some(data) = fonts.font_data.values().next().cloned() {
            fonts.font_data.insert("sent again".to_owned(), data);
        }
        self.egui_ctx.set_fonts(fonts);
        self.refont = 2;
        Ok(())
    }
}

/// A system font OpenRage knows how to find, by name and file.
const SYSTEM_FONTS: &[(&str, &str)] = &[
    ("Segoe UI", "segoeui.ttf"),
    ("Arial", "arial.ttf"),
    ("Tahoma", "tahoma.ttf"),
    ("Verdana", "verdana.ttf"),
    ("Calibri", "calibri.ttf"),
    ("DejaVu Sans", "DejaVuSans.ttf"),
    ("Noto Sans", "NotoSans-Regular.ttf"),
];

fn system_font_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Some(windows) = std::env::var_os("WINDIR").or_else(|| std::env::var_os("SystemRoot")) {
        dirs.push(std::path::PathBuf::from(windows).join("Fonts"));
    }
    for unix in [
        "/usr/share/fonts/truetype/dejavu",
        "/usr/share/fonts/TTF",
        "/usr/share/fonts/noto",
        "/usr/share/fonts/truetype/noto",
        "/usr/share/fonts/opentype/noto",
        "/usr/share/fonts/noto-cjk",
        "/Library/Fonts",
        "/System/Library/Fonts/Supplemental",
    ] {
        dirs.push(std::path::PathBuf::from(unix));
    }
    dirs
}

fn system_font_path(name: &str) -> Option<std::path::PathBuf> {
    let (_, file) = SYSTEM_FONTS.iter().find(|(n, _)| *n == name)?;
    system_font_dirs()
        .into_iter()
        .map(|dir| dir.join(file))
        .find(|path| path.is_file())
}

/// The system fonts that draw Chinese, Japanese and Korean, which no bundled font does: the language each is for, its file, its
/// face in the file, and how far down its glyphs are moved, in font sizes. The first file a machine has for a language is used.
///
/// ⚠ egui centres a fallback font on the row by its whole height, line gap included. Yu Gothic's line gap is half a font size,
/// so its glyphs sit a quarter of one above the Latin beside them unless they are moved back down; the other fonts have none.
// ponytail: off Windows one pan-CJK face draws all three, so Japanese takes Chinese shapes there; add a face per language if it matters.
const CJK_FONTS: &[(&str, &str, u32, f32)] = &[
    ("zh", "msyh.ttc", 0, 0.0),
    ("ja", "YuGothR.ttc", 0, 0.28),
    ("ja", "msgothic.ttc", 0, 0.0),
    ("ko", "malgun.ttf", 0, 0.0),
    ("zh", "NotoSansCJK-Regular.ttc", 2, 0.0),
    ("zh", "Arial Unicode.ttf", 0, 0.0),
];

/// The fonts of [`CJK_FONTS`] this machine has, read once: each is over ten megabytes and every restyle asks for them.
/// How much smaller than its own size Inter is set, so that it takes the room the screens give their text.
const INTER_SCALE: f32 = 0.94;

fn cjk_fonts() -> &'static [(&'static str, Arc<egui::FontData>)] {
    static FOUND: std::sync::OnceLock<Vec<(&'static str, Arc<egui::FontData>)>> =
        std::sync::OnceLock::new();
    FOUND.get_or_init(|| {
        let mut found: Vec<(&'static str, Arc<egui::FontData>)> = Vec::new();
        for &(language, file, index, down) in CJK_FONTS {
            if found.iter().any(|(of, _)| *of == language) {
                continue;
            }
            let read = system_font_dirs()
                .into_iter()
                .find_map(|dir| std::fs::read(dir.join(file)).ok());
            if let Some(bytes) = read {
                let mut font = egui::FontData::from_owned(bytes);
                font.index = index;
                font.tweak.y_offset_factor = down;
                found.push((language, Arc::new(font)));
            }
        }
        found
    })
}

/// Every font the Appearance page can offer on this machine.
pub(crate) fn available_fonts() -> Vec<FontChoice> {
    let mut fonts = vec![FontChoice::Default, FontChoice::Monospace];
    fonts.extend(
        SYSTEM_FONTS
            .iter()
            .filter(|(name, _)| system_font_path(name).is_some())
            .map(|(name, _)| FontChoice::System((*name).to_string())),
    );
    fonts
}

/// The font set for `choice`, and why it fell back if it did.
///
/// ⚠ **A font that cannot be found is a fallback, not a failure.** The choice
/// is stored by name and read at every start, so a font uninstalled since —
/// or a `prefs.json` carried to another machine — must still start the editor.
pub(crate) fn font_definitions(choice: &FontChoice) -> (egui::FontDefinitions, Option<String>) {
    let mut fonts = egui::FontDefinitions::default();
    // ⚠ Hack, the bundled monospace font, is the interface font's last fallback:
    // Ubuntu Light has no arrows or geometric shapes, and without it "File ▸ Open"
    // draws a box.
    let symbols = fonts
        .families
        .get(&egui::FontFamily::Monospace)
        .and_then(|family| family.first().cloned());
    if let (Some(symbols), Some(proportional)) = (
        symbols,
        fonts.families.get_mut(&egui::FontFamily::Proportional),
    ) {
        proportional.push(symbols);
    }
    // Inter leads the interface; egui's Ubuntu Light stays behind it for what Inter lacks.
    // ⚠ The bundled file maps nothing in the private-use area, where the icons live: stock Inter draws glyphs of its own there.
    // ⚠ Drawn unhinted: epaint reruns a hinted font's TrueType program each time a glyph comes at another size than the
    // last, and Inter's is heavy. Measured: hinted, the self-check (a fresh context per unit) took 27 s, unhinted 10 s.
    let inter = egui::FontData::from_static(include_bytes!("../../assets/fonts/Inter-Regular.ttf"));
    fonts.font_data.insert(
        "inter".into(),
        // ⚠ Inter sets wider and a pixel taller than the font the screens were laid out in: at its own size a row of controls wraps and a table hides a column.
        Arc::new(inter.tweak(egui::FontTweak {
            scale: INTER_SCALE,
            hinting_override: Some(false),
            ..Default::default()
        })),
    );
    if let Some(proportional) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        proportional.insert(0, "inter".into());
    }
    let mut fallback = None;
    let lead: Option<String> = match choice {
        FontChoice::Default => None,
        FontChoice::Monospace => fonts
            .families
            .get(&egui::FontFamily::Monospace)
            .and_then(|family| family.first().cloned()),
        FontChoice::System(name) => {
            match system_font_path(name).map(|path| (std::fs::read(&path), path)) {
                Some((Ok(bytes), _)) => {
                    let key = format!("system:{name}");
                    fonts.font_data.insert(
                        key.clone(),
                        std::sync::Arc::new(egui::FontData::from_owned(bytes)),
                    );
                    Some(key)
                }
                Some((Err(e), path)) => {
                    fallback = Some(format!("{}: {e}", path.display()));
                    None
                }
                None => {
                    fallback = Some(format!("{name} is not installed on this machine"));
                    None
                }
            }
        }
    };
    if let (Some(lead), Some(proportional)) = (
        lead,
        fonts.families.get_mut(&egui::FontFamily::Proportional),
    ) {
        proportional.retain(|f| f != &lead);
        proportional.insert(0, lead);
    }
    // ⚠ Regular only: Phosphor's other variants reuse its code points, so one
    // added beside it under any name in the same family draws every icon.
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    if let Some(mono) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
        if !mono.iter().any(|f| f == "phosphor") {
            mono.insert(1.min(mono.len()), "phosphor".into());
        }
    }
    // Chinese, Japanese and Korean share code points that each font draws in its own country's shapes, so the font of the
    // language in use comes first; the others stay, for the name of each language in Settings.
    let language = crate::i18n::active().code.as_str();
    let mut cjk: Vec<_> = cjk_fonts().iter().collect();
    cjk.sort_by_key(|(of, _)| *of != language);
    for (of, font) in cjk {
        let key = format!("cjk:{of}");
        fonts.font_data.insert(key.clone(), font.clone());
        for family in fonts.families.values_mut() {
            family.push(key.clone());
        }
    }
    (fonts, fallback)
}

/// Theme, accent, scale and font, applied to one context.
pub(crate) fn apply_appearance(ctx: &egui::Context, appearance: &AppearancePrefs) {
    theme::apply_theme(ctx, appearance);
    ctx.set_zoom_factor(appearance.scale());
    // `apply_theme` has just set a whole style, so this is the only place the time is shortened.
    if appearance.reduce_motion {
        ctx.global_style_mut(|style| style.animation_time = 0.0);
    }
    let (fonts, fallback) = font_definitions(&appearance.font);
    if let Some(why) = fallback {
        tracing::warn!(font = %appearance.font.label(), reason = %why, "Using the default font");
    }
    ctx.set_fonts(fonts);
}

pub(super) struct SharedGpu {
    pub(super) instance: wgpu::Instance,
    pub(super) adapter: wgpu::Adapter,
    pub(super) device: Arc<wgpu::Device>,
    pub(super) queue: Arc<wgpu::Queue>,
    pub(super) surface_format: wgpu::TextureFormat,
}

/// Uses the language `code` and restyles `ctx` for it, as a window does on the frame after the setting changes: the language orders the fallback fonts.
pub(crate) fn use_language(
    ctx: &egui::Context,
    state: &mut crate::state::AppState,
    code: Option<String>,
) {
    state.set_language(code);
    apply_appearance(ctx, &state.prefs.appearance);
}

/// The egui context this application runs, theme, fonts, loaders and all.
///
/// ⚠ Every window builds its context here rather than inline, so a run
/// without a window (`--self-check`) exercises the same theme, the same
/// phosphor fallback and the same image loaders: a missing glyph or an
/// unregistered loader is a panic inside `tessellate`.
pub(crate) fn configured_egui_context(
    max_texture_side: Option<usize>,
    appearance: &AppearancePrefs,
) -> egui::Context {
    let egui_ctx = egui::Context::default();
    apply_appearance(&egui_ctx, appearance);
    egui_extras::install_image_loaders(&egui_ctx);
    if let Some(side) = max_texture_side {
        egui_ctx.input_mut(|i| i.raw.max_texture_side = Some(side));
    }
    egui_ctx
}

impl App {
    pub(super) fn init_gpu(&mut self, first_window: &Arc<Window>) -> Result<()> {
        if self.gpu.is_some() {
            return Ok(());
        }

        let instance =
            wgpu::Instance::new(or_renderer::instance::with_dxc(wgpu::InstanceDescriptor {
                backends: or_renderer::instance::BACKENDS,
                flags: wgpu::InstanceFlags::default(),
                memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
                backend_options: wgpu::BackendOptions::default(),
                display: None,
            }));

        let probe = instance.create_surface(first_window.clone())?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&probe),
            force_fallback_adapter: false,
        }))
        .map_err(|e| {
            let needs = or_renderer::instance::UNSUPPORTED;
            anyhow::anyhow!("Failed to find suitable GPU adapter: {e}. {needs}")
        })?;

        tracing::info!("Using GPU: {}", adapter.get_info().name);

        let adapter_features = adapter.features();
        let mut required_features = wgpu::Features::empty();
        if adapter_features.contains(wgpu::Features::CLIP_DISTANCES) {
            required_features |= wgpu::Features::CLIP_DISTANCES;
        }
        if adapter_features.contains(or_renderer::gpu_timer::FEATURES) {
            required_features |= or_renderer::gpu_timer::FEATURES;
        }
        for wanted in [
            wgpu::Features::TEXTURE_COMPRESSION_BC,
            wgpu::Features::INDIRECT_FIRST_INSTANCE,
        ] {
            if adapter_features.contains(wanted) {
                required_features |= wanted;
            }
        }
        let mut required_limits = wgpu::Limits::default();
        required_limits.max_bind_groups = required_limits.max_bind_groups.max(5);
        // A GTA VI drawable's own programs bind arrays of textures and more buffers than a device gives unasked
        required_features |= or_renderer::game_shaders::meshlets::device_needs(
            adapter_features,
            &adapter.limits(),
            &mut required_limits,
        );
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("OpenRage device"),
                required_features,
                required_limits,
                experimental_features: wgpu::ExperimentalFeatures::default(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            }))?;

        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let surface_caps = probe.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        drop(probe);

        self.gpu = Some(SharedGpu {
            instance,
            adapter,
            device,
            queue,
            surface_format,
        });

        Ok(())
    }

    /// A window's surface, with its AccessKit adapter made and the window shown.
    ///
    /// ⚠ `window` must still be hidden: the adapter refuses a window that has been shown.
    pub(super) fn build_window_surface(
        &self,
        event_loop: &ActiveEventLoop,
        window: Arc<Window>,
    ) -> Result<WindowSurface> {
        let gpu = self
            .gpu
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("GPU not initialised"))?;

        let surface = gpu.instance.create_surface(window.clone())?;
        let size = window.inner_size();
        let surface_caps = surface.get_capabilities(&gpu.adapter);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: gpu.surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&gpu.device, &surface_config);

        let max_texture_side = gpu.device.limits().max_texture_dimension_2d as usize;
        let appearance = self.app_state.prefs.appearance.clone();
        let egui_ctx = configured_egui_context(None, &appearance);
        let mut egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui_ctx.viewport_id(),
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(max_texture_side),
        );
        egui_state.init_accesskit(event_loop, &window, self.proxy.clone());
        window.set_visible(true);

        let egui_renderer = egui_wgpu::Renderer::new(
            &gpu.device,
            gpu.surface_format,
            egui_wgpu::RendererOptions::default(),
        );

        Ok(WindowSurface {
            window,
            surface,
            surface_config,
            egui_ctx,
            egui_state,
            egui_renderer,
            appearance,
            language: crate::i18n::active().code.as_str(),
            refont: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lead(fonts: &egui::FontDefinitions) -> String {
        fonts.families[&egui::FontFamily::Proportional][0].clone()
    }

    #[test]
    fn a_font_that_is_not_installed_falls_back_and_says_why() {
        let (fonts, fallback) = font_definitions(&FontChoice::System("No Such Font".into()));
        let (default, none) = font_definitions(&FontChoice::Default);
        assert!(none.is_none());
        assert_eq!(lead(&fonts), lead(&default));
        let why = fallback.expect("a missing font is reported");
        assert!(why.contains("No Such Font"), "{why}");
    }

    /// ⚠ Falsified by adding Phosphor's Fill font beside its Regular one: the
    /// two share their code points, so whichever a family reaches first draws
    /// every icon, and a regular icon draws filled.
    #[test]
    fn every_regular_icon_draws_from_phosphors_regular_font() {
        let (fonts, _) = font_definitions(&FontChoice::Default);
        let phosphor = fonts.font_data.get("phosphor").expect("the icon font");
        assert!(
            *phosphor.font == *egui_phosphor::Variant::Regular.font_bytes(),
            "the font named phosphor is not Phosphor's regular one"
        );
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let listed = fonts.families[&family]
                .iter()
                .filter(|name| *name == "phosphor")
                .count();
            assert_eq!(listed, 1, "{family:?} lists the icon font {listed} times");
        }
    }

    #[test]
    fn monospace_leads_the_interface_font() {
        let (fonts, _) = font_definitions(&FontChoice::Monospace);
        assert_eq!(
            lead(&fonts),
            fonts.families[&egui::FontFamily::Monospace][0]
        );
        assert!(
            fonts.families[&egui::FontFamily::Proportional]
                .iter()
                .any(|f| f == "phosphor"),
            "the icon fallback must survive a font change"
        );
    }

    /// ⚠ Machine-dependent by nature: it checks every system font this
    /// machine has, and says how many that was.
    #[test]
    fn every_offered_system_font_loads_and_leads() {
        let mut checked = 0;
        for font in available_fonts() {
            let FontChoice::System(name) = &font else {
                continue;
            };
            let (fonts, fallback) = font_definitions(&font);
            assert!(fallback.is_none(), "{name}: {fallback:?}");
            assert_eq!(lead(&fonts), format!("system:{name}"));
            checked += 1;
        }
        eprintln!("system fonts checked: {checked}");
        #[cfg(target_os = "windows")]
        assert!(
            checked > 0,
            "a Windows install ships at least Arial and Segoe UI"
        );
    }

    /// The characters a literal spells as written in the source, its `\u{…}` escapes decoded.
    fn spelled(literal: &str) -> Vec<char> {
        let mut out = Vec::new();
        let mut chars = literal.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
            } else if chars.next() == Some('u') {
                let hex: String = chars.by_ref().skip(1).take_while(|&c| c != '}').collect();
                out.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
            }
        }
        out
    }

    /// ⚠ Falsified by dropping the symbol fallback from `font_definitions`: the
    /// status bar's "File ▸ Open" draws its arrow as a box again.
    #[test]
    fn every_symbol_the_shell_writes_is_in_its_fonts() {
        let (definitions, _) = font_definitions(&FontChoice::Default);
        let mut fonts = egui::epaint::text::Fonts::new(Default::default(), definitions);
        let (scanned, literals) = crate::edition_strings::user_facing_literals(&["src"]);
        let mut symbols = std::collections::BTreeMap::new();
        for (file, line, text) in &literals {
            for c in spelled(text).into_iter().filter(|c| !c.is_ascii()) {
                symbols.entry(c).or_insert_with(|| format!("{file}:{line}"));
            }
        }
        assert!(
            scanned > 200 && symbols.contains_key(&'\u{25B8}') && symbols.contains_key(&'\u{25A4}'),
            "{scanned} files read and {} symbols found, without the status bar's ▸ or its \\u{{25A4}}",
            symbols.len()
        );
        let missing: Vec<String> = symbols
            .iter()
            .filter(|(c, _)| {
                [egui::FontFamily::Proportional, egui::FontFamily::Monospace]
                    .into_iter()
                    .any(|family| !fonts.has_glyph(&egui::FontId::new(14.0, family), **c))
            })
            .map(|(c, at)| format!("U+{:04X} {c}, first at {at}", u32::from(*c)))
            .collect();
        assert!(
            missing.is_empty(),
            "{} of {} symbols the shell writes draw as a box:\n{}",
            missing.len(),
            symbols.len(),
            missing.join("\n")
        );
    }

    #[test]
    fn the_scale_reaches_the_context() {
        let _serial = theme::serial();
        let ctx = egui::Context::default();
        let appearance = AppearancePrefs {
            ui_scale_percent: 150,
            ..AppearancePrefs::default()
        };
        apply_appearance(&ctx, &appearance);
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        assert_eq!(ctx.zoom_factor(), 1.5, "zoom lands on the next pass");
        apply_appearance(&ctx, &AppearancePrefs::default());
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        assert_eq!(ctx.zoom_factor(), 1.0);
    }
}
