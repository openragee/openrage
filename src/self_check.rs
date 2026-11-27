//! The real startup path and a bounded number of real frames, with no window.
//!
//! ⚠ **A render-only check proves that drawing is side-effect-free and
//! nothing else.** So this constructs the real [`AppState`] — real
//! preferences, real install discovery — builds the
//! same `egui::Context` a window would (`app::gpu::configured_egui_context`),
//! and runs the real [`crate::ui::draw`] through `begin_pass` / `tessellate` /
//! `end_pass`. Tessellation is deliberate: it is where a missing glyph or an
//! unregistered image loader turns into a panic.
//!
//! ⚠ **It does not reach the event loop or a surface.** `winit` is never
//! constructed, so window creation, input translation and resize are
//! uncovered; `or_renderer::RenderContext` takes a
//! window, so the swap chain and `egui_wgpu`'s paint path are too.
//! `RenderBridge` needs a device but not a surface, so `--viewport` builds one
//! headlessly; without it every panel draws with `bridge: None`.

use std::panic::AssertUnwindSafe;
use std::sync::Mutex;

use crate::state::AppState;
use crate::ui::dock::{Tab, Workspace};
use crate::ui::panel_registry;

mod front_door;
mod gta6_script;
#[cfg(test)]
mod judged_without_an_install;
#[cfg(test)]
mod leaves_nothing;
mod project;
pub(crate) mod shots;
mod units;
mod wardrobe;

/// Wide enough that the default layouts are not degenerate.
const SCREEN: (f32, f32) = (1600.0, 900.0);

pub struct Options {
    /// Frames per unit. More than one because the first frame of an egui
    /// surface allocates its ids and the second is the one that reuses them.
    pub frames: usize,
    pub viewport: bool,
    /// A file to open through the real load path and draw in its workspace.
    pub open: Option<std::path::PathBuf>,
    /// A script of steps and the directory its PNGs go to.
    pub shots: Option<(std::path::PathBuf, std::path::PathBuf)>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            frames: 3,
            viewport: false,
            open: None,
            shots: None,
        }
    }
}

/// `None` unless `--self-check` is present, so the editor's own argument
/// handling is untouched.
pub fn options_from(args: impl IntoIterator<Item = String>) -> Option<Options> {
    let args: Vec<String> = args.into_iter().collect();
    if !args.iter().any(|a| a == "--self-check") {
        return None;
    }
    let mut options = Options::default();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--viewport" => options.viewport = true,
            "--open" => options.open = rest.next().map(std::path::PathBuf::from),
            "--shots" => {
                let script = rest.next().map(std::path::PathBuf::from);
                let out = rest.next().map(std::path::PathBuf::from);
                options.shots = Some((script.unwrap_or_default(), out.unwrap_or_default()));
            }
            "--frames" => {
                if let Some(n) = rest.next().and_then(|n| n.parse().ok()) {
                    options.frames = n;
                }
            }
            _ => {}
        }
    }
    Some(options)
}

/// What one frame put on the screen.
///
/// ⚠ **Vertices rather than primitives.** egui batches a whole clip rect into
/// one `ClippedPrimitive`, so a panel that paints a single label and a panel
/// that paints a full property grid both score one — the count moves with the
/// number of *regions*, not with what is in them. A glyph is a quad, so
/// vertices move with the content.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Painted {
    primitives: usize,
    vertices: usize,
}

impl std::fmt::Display for Painted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} vertices in {} region(s)",
            self.vertices, self.primitives
        )
    }
}

/// `Err` is the reason it failed, already formatted.
type Outcome = Result<Painted, String>;

struct Report {
    rows: Vec<(String, Outcome)>,
    notes: Vec<String>,
}

impl Report {
    fn record(&mut self, what: impl Into<String>, outcome: Outcome) {
        self.rows.push((what.into(), outcome));
    }

    fn note(&mut self, note: impl Into<String>) {
        self.notes.push(note.into());
    }

    fn failures(&self) -> usize {
        self.rows.iter().filter(|(_, o)| o.is_err()).count()
    }

    fn print(&self) {
        println!();
        for note in &self.notes {
            println!("  {note}");
        }
        println!();
        for (what, outcome) in &self.rows {
            match outcome {
                Ok(painted) => println!("  ok    {what}  ({painted})"),
                Err(why) => println!("  FAIL  {what}\n          {why}"),
            }
        }
        println!();
        println!("{} unit(s), {} failed", self.rows.len(), self.failures());
    }
}

const BLANK_ID: crate::ui::panel_registry::PanelId = "self-check-blank";

/// The control panel: registered for real, and paints nothing on purpose.
struct BlankPanel;

impl panel_registry::Panel for BlankPanel {
    fn id(&self) -> panel_registry::PanelId {
        BLANK_ID
    }

    // ⚠ Longer than any panel's title: every tab bar shows its title, so a
    // body that paints nothing must still paint less than the control.
    fn title(&self, _state: &AppState) -> String {
        "(blank control panel)".to_string()
    }

    fn draw(&self, _ui: &mut egui::Ui, _ctx: &mut panel_registry::PanelContext<'_>) {
        #[cfg(test)]
        assert!(
            !CONTROL_FAILS.load(std::sync::atomic::Ordering::Relaxed),
            "the control panel was made to fail"
        );
    }
}

static BLANK: BlankPanel = BlankPanel;

/// Set by the one test that runs a self-check whose control panel fails, in a process of its own.
#[cfg(test)]
static CONTROL_FAILS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

const GUEST_WIDGETS_ID: crate::ui::panel_registry::PanelId = "self-check-guest-widgets";

/// A guest's panel as the host draws it: every widget a plugin can describe, through the host's own drawing.
///
/// ⚠ The self-check has no wasm32 module to run, so the widgets are written here; what a guest
/// describes and what the user does to it reach the guest in `a_plugin_adds_to_the_editor`.
struct GuestWidgetsPanel;

impl panel_registry::Panel for GuestWidgetsPanel {
    fn id(&self) -> panel_registry::PanelId {
        GUEST_WIDGETS_ID
    }

    fn title(&self, _state: &AppState) -> String {
        "Guest widgets".to_string()
    }

    fn draw(&self, ui: &mut egui::Ui, _ctx: &mut panel_registry::PanelContext<'_>) {
        use or_plugin::Widget;
        let widgets = [
            Widget::Label {
                text: "Every widget a plugin can describe".into(),
            },
            Widget::Button {
                id: "press".into(),
                text: "Press".into(),
            },
            Widget::Slider {
                id: "weight".into(),
                label: "Weight".into(),
                min: 0.0,
                max: 1.0,
                value: 0.5,
            },
            Widget::Checkbox {
                id: "pin".into(),
                label: "Pin".into(),
                checked: true,
            },
            Widget::TextField {
                id: "name".into(),
                label: "Name".into(),
                text: "left".into(),
            },
            Widget::List {
                id: "choice".into(),
                items: vec!["Front".into(), "Back".into()],
                selected: Some(1),
            },
        ];
        crate::ui::panels::plugin_panel::draw_widgets(ui, &widgets);
    }
}

static GUEST_WIDGETS: GuestWidgetsPanel = GuestWidgetsPanel;

/// The folder a `--self-check` run made for its preferences, removed when this is dropped.
struct ScratchToRemove(Option<std::path::PathBuf>);

impl Drop for ScratchToRemove {
    fn drop(&mut self) {
        if let Some(scratch) = self.0.take() {
            let _ = std::fs::remove_dir_all(scratch);
        }
    }
}

/// The exit code.
pub fn run(options: Options) -> i32 {
    run_removing(options, crate::state::prefs::self_check_scratch_to_clean())
}

/// [`run`] with the folder to remove named, which a test does since it cannot start the process with `--self-check`.
fn run_removing(options: Options, scratch: Option<std::path::PathBuf>) -> i32 {
    // ⚠ Held first and dropped on every way out: this run returns early when its control panel
    // fails or a setting is refused, and a panic unwinds through it.
    let _scratch = ScratchToRemove(scratch);
    // ⚠ First: a click in a script reaches every command the app has.
    crate::platform::stand_in_for_the_desktop();
    if let Some((script, out)) = &options.shots {
        if script.as_os_str().is_empty() || out.as_os_str().is_empty() {
            println!("--shots takes a script and the directory its PNGs go to");
            return 1;
        }
        if let Err(message) = checked_config_root() {
            println!("{message}");
            return 1;
        }
        return shots::run(script, out);
    }
    let mut report = Report {
        rows: Vec::new(),
        notes: Vec::new(),
    };

    if !panel_registry::register(&BLANK) {
        println!("something already provides `{BLANK_ID}`, so the control is not a control");
        return 1;
    }

    if !panel_registry::register(&GUEST_WIDGETS) {
        println!(
            "something already provides `{GUEST_WIDGETS_ID}`, so the guest panel is not drawn"
        );
        return 1;
    }
    // A plugin's panel with no plugin loaded behind it: the real `GuestPanel`, which says so.
    crate::ui::panels::plugin_panel::register("self-check", "not-loaded");

    let config_root = match checked_config_root() {
        Ok(root) => root,
        Err(message) => {
            println!("{message}");
            return 1;
        }
    };

    // ⚠ Constructed before the hook and outside `catch_unwind`: a panic here
    // is the startup path failing, and there are no frames to report about.
    // It should look like an ordinary crash.
    let mut state = AppState::new();
    // ⚠ The editor's frame records the installs discovery found; no frame here polls it.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while state.discovery.is_running() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
        state.poll_game_discovery();
    }
    // ⚠ The write this run is judged by. Discovery writes only when it finds an install, so a
    // machine with none (every CI runner, every Mac) failed a run that drew every panel. One save
    // through the editor's own function reaches the same redirected folder, with or without one.
    crate::state::prefs::save_prefs(&state.prefs);
    describe_environment(&state, &mut report);

    install_quiet_panic_hook();

    let mut bridge = if options.viewport {
        match headless_bridge() {
            Ok(bridge) => {
                report.note(
                    "viewport: a headless wgpu device was created, so the \
                             viewport and gallery draw against a real GPU",
                );
                Some(bridge)
            }
            Err(e) => {
                report.record("viewport: headless wgpu device", Err(e));
                None
            }
        }
    } else {
        report.note(
            "viewport: not exercised — pass --viewport to build a surfaceless \
             wgpu device and draw through RenderBridge",
        );
        None
    };

    // ⚠ A docked panel whose body does nothing still costs the leaf's
    // background and tab bar, so measuring against an *empty* dock would
    // credit every panel with chrome it did not draw. `BLANK` is a real
    // registered panel with an empty `draw`.
    let blank = match draw_unit(&mut state, None, options.frames, |state| {
        state.active_workspace = Workspace::Particle;
        *state.workspace_docks.get_mut(Workspace::Particle) =
            egui_dock::DockState::new(vec![Tab(BLANK_ID)]);
    }) {
        Ok(painted) => {
            report.record("control: a docked panel that paints nothing", Ok(painted));
            painted
        }
        Err(e) => {
            report.record("control: a docked panel that paints nothing", Err(e));
            report.print();
            return 1;
        }
    };

    for panel in panel_registry::all() {
        let id = panel.id();
        if id == BLANK_ID {
            continue;
        }
        let outcome = draw_unit(&mut state, bridge.as_mut(), options.frames, move |state| {
            state.active_workspace = Workspace::Particle;
            *state.workspace_docks.get_mut(Workspace::Particle) =
                egui_dock::DockState::new(vec![Tab(id)]);
        })
        .and_then(|painted| {
            if painted.vertices > blank.vertices {
                Ok(painted)
            } else {
                Err(format!(
                    "painted {} vertices, no more than the {} an empty panel in the \
                     same leaf costs — this panel drew nothing of its own",
                    painted.vertices, blank.vertices
                ))
            }
        });
        report.record(format!("panel `{id}`"), outcome);
    }

    units::kind_units(
        &mut state,
        bridge.as_mut(),
        options.frames,
        &blank,
        &mut report,
    );

    for ws in Workspace::ALL {
        let outcome = draw_unit(&mut state, bridge.as_mut(), options.frames, move |state| {
            state.active_workspace = ws;
            state.workspace_docks.reset(ws);
        });
        report.record(
            format!("workspace `{}` default layout", ws.label()),
            outcome,
        );
    }

    project::project_units(&mut state, options.frames, &blank, &mut report);
    wardrobe::wardrobe_units(
        &mut state,
        options.frames,
        &blank,
        bridge.as_mut(),
        &mut report,
    );
    front_door::front_door_units(
        &mut state,
        options.frames,
        &blank,
        &mut report,
        &config_root,
    );

    // Kinds nothing above opens: each draws its workspace with a blank document active.
    for kind in [
        crate::state::DocumentKind::Ynd,
        crate::state::DocumentKind::Ynv,
    ] {
        let doc = state.new_empty_document(kind);
        let ws = Workspace::for_kind(kind);
        let outcome = draw_unit(&mut state, bridge.as_mut(), options.frames, move |state| {
            state.set_active_document(doc);
            state.active_workspace = ws;
            state.workspace_docks.reset(ws);
        });
        report.record(
            format!(
                "workspace `{}` with a blank .{}",
                ws.label(),
                kind.extension()
            ),
            outcome,
        );
        discard(&mut state, doc);
    }

    if let Some(path) = &options.open {
        open_unit(
            &mut state,
            bridge.as_mut(),
            options.frames,
            path,
            &mut report,
        );
    }

    report.record(
        "start page: the Document panel with nothing open",
        draw_start_page(&mut state, options.frames, &blank),
    );
    report.record(
        "status bar: the game switcher open",
        draw_game_switcher(&mut state, options.frames),
    );
    let install = state.open_game_root(config_root.clone());
    let outcome = draw_unit(&mut state, bridge.as_mut(), options.frames, move |state| {
        state.set_active_document(install);
        state.workspace_docks.reset(Workspace::Browse);
    });
    discard(&mut state, install);
    report.record("workspace `Browse` with an install open", outcome);
    state.plugins.scanned = Some(vec![synthetic_plugin()]);
    for category in crate::ui::panels::settings::SettingsCategory::ALL {
        state
            .panels
            .get::<crate::ui::panels::settings::SettingsUiState>()
            .category = category;
        report.record(
            format!("settings window: {}", category.label()),
            draw_settings(&mut state, options.frames),
        );
    }
    state
        .panels
        .put(crate::ui::panels::settings::SettingsUiState::default());
    report.record(
        "crash report window",
        draw_crash_notice(&mut state, options.frames),
    );
    report.record(
        "plugin consent prompt",
        draw_consent_prompt(&mut state, options.frames),
    );
    report.record(
        "unsaved changes prompt",
        draw_question(
            &mut state,
            options.frames,
            |state| {
                let doc = state.new_empty_document(crate::state::DocumentKind::Text);
                state.close_document(doc);
            },
            |state| state.answer_close(crate::state::CloseAnswer::DontSave),
        ),
    );
    report.record(
        "changed-on-disk prompt",
        draw_question(
            &mut state,
            options.frames,
            |state| {
                let file = std::path::PathBuf::from("C:/mods/smoke.ypt");
                state.reload_conflicts.push(file);
            },
            |state| state.reload_conflicts.clear(),
        ),
    );
    report.record(
        "delete question",
        draw_file_prompt(&mut state, options.frames, |state| {
            state.pending_delete = Some(crate::state::delete_prompt::PendingDelete {
                doc_id: 0,
                node_id: 0,
                name: "props".into(),
                files: 2,
                folders: 1,
                bytes: 15,
                from: crate::state::delete_prompt::DeleteFrom::Disk,
            });
        }),
    );
    report.record(
        "extraction clash question",
        draw_file_prompt(&mut state, options.frames, |state| {
            state.extract_jobs.clash = Some(crate::state::extract_job::ExtractClash {
                doc_id: 0,
                nodes: Vec::new(),
                dest: "out".into(),
                form: or_io::container::rpf::BodyForm::Decoded,
                filter: crate::state::TypeFilter::All,
                fixes: false,
                clashes: 2,
                planned: 3,
                first: "a.ytd".into(),
            });
        }),
    );
    report.record(
        "refused write prompt",
        draw_file_prompt(&mut state, options.frames, |state| {
            state.refused_write = Some("C:/Games/GTAV/update/update.rpf".into());
        }),
    );

    // ⚠ Last, because the palette is process-wide: every unit above draws
    // the appearance the user saved, and this one draws the other theme, a
    // non-default accent, scale and font through the same shell.
    let saved = state.prefs.appearance.clone();
    let other = crate::state::AppearancePrefs {
        theme: match saved.theme {
            crate::state::ThemeChoice::Dark => crate::state::ThemeChoice::Light,
            crate::state::ThemeChoice::Light => crate::state::ThemeChoice::Dark,
        },
        accent: [0xd0, 0x50, 0x20],
        ui_scale_percent: 150,
        font: crate::state::FontChoice::Monospace,
        reduce_motion: !saved.reduce_motion,
    };
    let label = format!(
        "workspace `Particle` in the {} theme at 150%, monospace",
        other.theme.label()
    );
    state.prefs.appearance = other;
    let outcome = draw_unit(&mut state, bridge.as_mut(), options.frames, |state| {
        state.active_workspace = Workspace::Particle;
        state.workspace_docks.reset(Workspace::Particle);
    });
    state.prefs.appearance = saved;
    report.record(label, outcome);

    report.print();
    let wrote_preferences = report_config_root(&config_root);
    let failures = report.failures() > 0;

    // ⚠ Reported rather than assumed. A run that stopped constructing
    // `AppState` would leave the owner's `prefs.json` untouched too, and that
    // is the wrong reason to be safe — this line is what tells the two apart.
    if !wrote_preferences {
        println!(
            "\n⚠ no prefs.json was written, so this run did not reach the code the \
             redirect above exists to contain"
        );
    }

    i32::from(failures || !wrote_preferences)
}

/// ⚠ **The refusal is in the binary, not the recipe**: a guarantee spelled in a
/// justfile does not survive somebody typing
/// `cargo run -p openrage --bin openrage -- --self-check`.
/// [`crate::state::prefs::config_root`] redirects a `--self-check` process on
/// its own; this checks that it did.
fn checked_config_root() -> Result<std::path::PathBuf, String> {
    let asked_for = std::env::var_os("OPENRAGE_CONFIG_DIR").is_some_and(|d| !d.is_empty());
    let root = crate::state::prefs::config_root()
        .ok_or_else(|| "this platform has no configuration directory".to_string())?;

    if !asked_for && Some(&root) == crate::state::prefs::real_config_root().as_ref() {
        return Err(format!(
            "refusing to run: --self-check resolved to the real configuration \
             directory\n\n  {}\n\nA self-check constructs the real AppState, which saves \
             preferences, so this run\nwould rewrite the machine owner's prefs.json. Set \
             OPENRAGE_CONFIG_DIR to say that\nis what you meant.",
            root.display()
        ));
    }
    Ok(root)
}

/// Says where this run's state went, and whether any arrived.
fn report_config_root(root: &std::path::Path) -> bool {
    // Where the editor's own save puts it: the root's `prefs.json`, and in a test build a folder
    // of that thread's own, which is the only reason this does not read the root.
    let wrote = crate::state::prefs::prefs_path().is_some_and(|path| path.is_file());
    println!(
        "\nconfig: {}  (prefs.json written: {})",
        root.display(),
        if wrote { "yes" } else { "no" }
    );
    wrote
}

/// What the run started with.
fn describe_environment(state: &AppState, report: &mut Report) {
    report.note(format!(
        "documents open at startup: {}",
        state.documents.len()
    ));
    report.note(format!("start page shown: {}", state.documents.is_empty()));
    report.note(format!(
        "panels registered: {}",
        panel_registry::all().len()
    ));
}

/// Run `frames` frames of the real shell and return what the last one painted.
///
/// ⚠ **A fresh `egui::Context` per unit, and that is not tidiness.** A panic
/// inside a panel unwinds out of `begin_pass` with the context still inside a
/// pass; the next `begin_pass` on it then panics *while* that state is
/// broken, and the process aborts without printing the report. Measured: one
/// panicking panel took the whole run down after reporting eight rows. Each
/// unit owning its context means one broken panel costs one row.
fn draw_unit(
    state: &mut AppState,
    bridge: Option<&mut crate::render::RenderBridge>,
    frames: usize,
    arrange: impl FnOnce(&mut AppState),
) -> Outcome {
    let mut bridge = bridge;
    let ctx = crate::app::gpu::configured_egui_context(Some(2048), &state.prefs.appearance);
    std::panic::catch_unwind(AssertUnwindSafe(move || {
        // ⚠ A toast expires by the wall clock, so on a slow run one unit drew the toasts a step before it left and the next drew none, and a unit judged against another drawn a moment earlier failed at random; a unit that wants a toast asks for it in `arrange`.
        state.toasts.clear();
        arrange(state);
        let mut painted = Painted {
            primitives: 0,
            vertices: 0,
        };
        for frame in 0..frames.max(1) {
            ctx.begin_pass(raw_input(frame));
            crate::ui::draw(&ctx, state, bridge.as_deref_mut());
            crate::app::modals::draw_editor_overlays(&ctx, state);
            let output = ctx.end_pass();
            painted = measure(ctx.tessellate(output.shapes, output.pixels_per_point));
        }
        painted
    }))
    .map_err(describe_panic)
}

/// A file opened through the real load path and drawn in its kind's workspace,
/// judged by the geometry the viewport built for it rather than by egui's
/// vertices, which count the viewport as one image whatever it shows, and
/// failed if drawing marked it modified.
fn open_unit(
    state: &mut AppState,
    bridge: Option<&mut crate::render::RenderBridge>,
    frames: usize,
    path: &std::path::Path,
    report: &mut Report,
) {
    let what = format!("viewport: {}", path.display());
    let Some(bridge) = bridge else {
        report.record(what, Err("needs --viewport to build geometry".into()));
        return;
    };
    state.load_file(path.to_path_buf());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while !matches!(state.loading, crate::state::LoadingState::Idle) {
        if std::time::Instant::now() > deadline {
            report.record(what, Err("still loading after two minutes".into()));
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        state.poll_load_result();
    }
    let Some(doc) = state
        .documents
        .iter()
        .find(|d| d.path.as_deref() == Some(path))
        .map(|d| (d.id, d.kind))
    else {
        report.record(
            what,
            Err(format!("did not open: {}", state.error_dialog_message)),
        );
        return;
    };
    let (id, kind) = doc;
    let ws = Workspace::for_kind(kind);
    let outcome = draw_unit(state, Some(&mut *bridge), frames, move |state| {
        state.set_active_document(id);
        state.active_workspace = ws;
        state.workspace_docks.reset(ws);
    });
    let (meshes, lines) = bridge.document_geometry();
    let modified = state.document(id).is_some_and(|d| d.dirty);
    discard(state, id);
    report.record(
        format!("{what} ({meshes} mesh draws, {lines} line vertices)"),
        outcome.and_then(|painted| {
            if modified {
                Err("drawing it marked it modified".into())
            } else if meshes + lines > 0 {
                Ok(painted)
            } else {
                Err("the viewport built no geometry for it".into())
            }
        }),
    );
}

fn measure(primitives: Vec<egui::ClippedPrimitive>) -> Painted {
    let vertices = primitives
        .iter()
        .map(|p| match &p.primitive {
            egui::epaint::Primitive::Mesh(mesh) => mesh.vertices.len(),
            egui::epaint::Primitive::Callback(_) => 0,
        })
        .sum();
    Painted {
        primitives: primitives.len(),
        vertices,
    }
}

/// The Document panel alone with nothing open, which draws the start page, judged against the blank panel.
fn draw_start_page(state: &mut AppState, frames: usize, blank: &Painted) -> Outcome {
    let open = std::mem::take(&mut state.documents);
    let drawn = draw_unit(state, None, frames, |state| {
        state.active_workspace = Workspace::Particle;
        *state.workspace_docks.get_mut(Workspace::Particle) =
            egui_dock::DockState::new(vec![Tab::DOCUMENT]);
    });
    state.documents = open;
    let drawn = drawn?;
    if drawn.vertices > blank.vertices {
        Ok(drawn)
    } else {
        Err(format!(
            "painted {} vertices, no more than the {} an empty panel costs",
            drawn.vertices, blank.vertices
        ))
    }
}

/// The shell with the status bar's game switcher open, judged against the shell with it shut.
fn draw_game_switcher(state: &mut AppState, frames: usize) -> Outcome {
    use crate::ui::panels::status_bar::GameSwitcher;

    let shut = draw_unit(state, None, frames, |_| {})?;
    let open = draw_unit(state, None, frames, |state| {
        state.panels.get::<GameSwitcher>().open = true;
    });
    state.panels.get::<GameSwitcher>().open = false;
    let open = open?;
    if open.vertices > shut.vertices {
        Ok(open)
    } else {
        Err(format!(
            "painted {} vertices open and {} shut — the switcher drew nothing",
            open.vertices, shut.vertices
        ))
    }
}

fn draw_settings(state: &mut AppState, frames: usize) -> Outcome {
    let ctx = crate::app::gpu::configured_egui_context(Some(2048), &state.prefs.appearance);
    std::panic::catch_unwind(AssertUnwindSafe(move || {
        let mut painted = Painted {
            primitives: 0,
            vertices: 0,
        };
        for frame in 0..frames.max(1) {
            ctx.begin_pass(raw_input(frame));
            crate::ui::panels::settings::draw_settings_window_body(&ctx, state);
            let output = ctx.end_pass();
            painted = measure(ctx.tessellate(output.shapes, output.pixels_per_point));
        }
        painted
    }))
    .map_err(describe_panic)
}

/// The overlays with a synthetic report pending, judged against the same
/// overlays with none — a window that drew nothing would tie.
fn draw_crash_notice(state: &mut AppState, frames: usize) -> Outcome {
    use crate::diagnostics::crash;

    let without = draw_overlays(state, frames)?;
    crash::publish(crash::CrashNotice {
        path: None,
        report: "OpenRage crash report\n\nPanic:    a self-check's synthetic report\n".into(),
        from_last_session: true,
    });
    let with = draw_overlays(state, frames);
    crash::dismiss();
    let with = with?;
    if with.vertices > without.vertices {
        Ok(with)
    } else {
        Err(format!(
            "painted {} vertices with a report pending and {} without — the window drew nothing",
            with.vertices, without.vertices
        ))
    }
}

/// The overlays with a plugin's consent pending, judged against the same
/// overlays with none.
fn draw_consent_prompt(state: &mut AppState, frames: usize) -> Outcome {
    let without = draw_overlays(state, frames)?;
    state.plugins.pending_consent = Some(synthetic_plugin());
    let with = draw_overlays(state, frames);
    state.plugins.pending_consent = None;
    let with = with?;
    if with.vertices > without.vertices {
        Ok(with)
    } else {
        Err(format!(
            "painted {} vertices with a consent pending and {} without — the prompt drew nothing",
            with.vertices, without.vertices
        ))
    }
}

/// The overlays with the question `ask` raises, judged against the same
/// overlays with none; `answer` takes it down again.
fn draw_question(
    state: &mut AppState,
    frames: usize,
    ask: impl FnOnce(&mut AppState),
    answer: impl FnOnce(&mut AppState),
) -> Outcome {
    let without = draw_overlays(state, frames)?;
    ask(state);
    let with = draw_overlays(state, frames);
    answer(state);
    let with = with?;
    if with.vertices > without.vertices {
        Ok(with)
    } else {
        Err(format!(
            "painted {} vertices with the question asked and {} without — the prompt drew nothing",
            with.vertices, without.vertices
        ))
    }
}

/// The shell with the question a delete, an extraction or a refused write raises, judged
/// against the shell with none.
fn draw_file_prompt(state: &mut AppState, frames: usize, ask: fn(&mut AppState)) -> Outcome {
    let without = draw_unit(state, None, frames, |_| {})?;
    let with = draw_unit(state, None, frames, ask);
    state.pending_delete = None;
    state.extract_jobs.clash = None;
    state.refused_write = None;
    let with = with?;
    if with.vertices > without.vertices {
        Ok(with)
    } else {
        Err(format!(
            "painted {} vertices with the question up and {} without — it drew nothing",
            with.vertices, without.vertices
        ))
    }
}

/// Close a document the self-check opened for itself, saved or not.
fn discard(state: &mut AppState, doc: crate::state::DocumentId) {
    state.close_document(doc);
    state.answer_close(crate::state::CloseAnswer::DontSave);
}

/// A plugin that wants every capability, so the prompt and the page draw every line.
fn synthetic_plugin() -> crate::state::plugins::InstalledPlugin {
    crate::state::plugins::InstalledPlugin {
        manifest: or_plugin::Manifest {
            id: "self-check".into(),
            name: "A self-check's plugin".into(),
            abi: or_plugin::ABI_VERSION,
            wants: or_plugin::Capability::ALL.to_vec(),
        },
        wasm: std::path::PathBuf::new(),
        refused: Some("a self-check's plugin has no module".into()),
    }
}

fn draw_overlays(state: &mut AppState, frames: usize) -> Outcome {
    let ctx = crate::app::gpu::configured_egui_context(Some(2048), &state.prefs.appearance);
    std::panic::catch_unwind(AssertUnwindSafe(|| {
        let mut painted = Painted {
            primitives: 0,
            vertices: 0,
        };
        for frame in 0..frames.max(1) {
            ctx.begin_pass(raw_input(frame));
            crate::app::modals::draw_editor_overlays(&ctx, state);
            let output = ctx.end_pass();
            painted = measure(ctx.tessellate(output.shapes, output.pixels_per_point));
        }
        painted
    }))
    .map_err(describe_panic)
}

/// ⚠ `time` advances, because a panel that animates reads it and a fixed
/// clock hides an animation that never settles.
fn raw_input(frame: usize) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(SCREEN.0, SCREEN.1),
        )),
        max_texture_side: Some(2048),
        time: Some(frame as f64 / 60.0),
        ..Default::default()
    }
}

/// Where the last panic was, which the unwind payload does not carry.
static LAST_PANIC_SITE: Mutex<Option<String>> = Mutex::new(None);

/// ⚠ **Replaces the default hook rather than adding to it**, because a unit
/// that panics is a reported row and the default hook would print the same
/// thing again for every frame of every workspace that contains the panel —
/// eight copies for one broken panel, measured.
fn install_quiet_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let site = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "an unknown location".to_string());
        *LAST_PANIC_SITE.lock().expect("panic site poisoned") = Some(site);
    }));
}

fn describe_panic(payload: Box<dyn std::any::Any + Send>) -> String {
    let message = if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "a payload this cannot describe".to_string()
    };
    match LAST_PANIC_SITE.lock().expect("panic site poisoned").take() {
        Some(site) => format!("panicked at {site}: {message}"),
        None => format!("panicked: {message}"),
    }
}

/// A `RenderBridge` over a `wgpu` device with no surface behind it.
pub(crate) fn headless_bridge() -> Result<crate::render::RenderBridge, String> {
    let (device, queue) = headless_gpu()?;
    Ok(crate::render::RenderBridge::new(
        device,
        queue,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ))
}

/// A `wgpu` device and queue with no surface behind them.
fn headless_gpu() -> Result<(std::sync::Arc<wgpu::Device>, std::sync::Arc<wgpu::Queue>), String> {
    use std::sync::Arc;

    // `WGPU_BACKEND` picks the backend, so the same frames are drawn through each one a machine has.
    let instance = wgpu::Instance::new(or_renderer::instance::with_dxc(wgpu::InstanceDescriptor {
        backends: or_renderer::instance::backends_from_env()?,
        flags: wgpu::InstanceFlags::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        backend_options: wgpu::BackendOptions::default(),
        display: None,
    }));
    let adapter = or_renderer::instance::headless_adapter(&instance)?;
    tracing::info!(backend = ?adapter.get_info().backend, adapter = %adapter.get_info().name, "the self-check's device");

    let adapter_features = adapter.features();
    let mut required_features = wgpu::Features::empty();
    if adapter_features.contains(wgpu::Features::CLIP_DISTANCES) {
        required_features |= wgpu::Features::CLIP_DISTANCES;
    }
    for wanted in [
        wgpu::Features::TEXTURE_COMPRESSION_BC,
        wgpu::Features::INDIRECT_FIRST_INSTANCE,
        // The stages of a frame on the device, which a shots script's `world perf` reads.
        or_renderer::gpu_timer::FEATURES,
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
    let (query_features, experimental_features) =
        or_renderer::game_shaders::trace::query::aided_device_needs(&adapter, &mut required_limits);
    required_features |= query_features;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("OpenRage self-check device"),
        required_features,
        required_limits,
        experimental_features,
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
    }))
    .map_err(|e| format!("could not create a headless device: {e}"))?;

    Ok((Arc::new(device), Arc::new(queue)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Option<Options> {
        options_from(args.iter().map(|a| (*a).to_string()))
    }

    /// ⚠ The editor's own argument handling must be untouched, or a file
    /// passed on the command line stops opening the editor.
    #[test]
    fn nothing_but_the_flag_asks_for_a_self_check() {
        assert!(parse(&[]).is_none());
        assert!(parse(&["some/file.ypt"]).is_none());
        assert!(parse(&["--viewport"]).is_none());
        assert!(parse(&["--self-check"]).is_some());
    }

    #[test]
    fn the_defaults_draw_more_than_one_frame_and_no_gpu() {
        let options = parse(&["--self-check"]).expect("the flag is present");
        assert!(
            options.frames > 1,
            "one frame never reuses an id it allocated, which is where a \
             duplicate-id panic shows"
        );
        assert!(
            !options.viewport,
            "the GPU half is opt-in, because CI has no adapter"
        );
    }

    #[test]
    fn the_frame_count_and_the_gpu_can_be_asked_for() {
        let options = parse(&["--self-check", "--frames", "7", "--viewport"]).expect("present");
        assert_eq!(options.frames, 7);
        assert!(options.viewport);
    }

    #[test]
    fn shots_take_a_script_and_a_directory() {
        let options = parse(&["--self-check", "--shots", "steps.txt", "out"]).expect("present");
        assert_eq!(
            options.shots,
            Some(("steps.txt".into(), "out".into())),
            "both paths are kept, in order"
        );
        let missing = parse(&["--self-check", "--shots", "steps.txt"]).expect("present");
        assert_eq!(
            missing.shots,
            Some(("steps.txt".into(), std::path::PathBuf::new())),
            "a missing directory is kept empty so the run refuses it, rather than \
             silently running the whole self-check instead"
        );
    }

    #[test]
    fn a_frame_count_that_is_not_a_number_is_ignored() {
        let options = parse(&["--self-check", "--frames", "lots"]).expect("present");
        assert_eq!(options.frames, Options::default().frames);
    }

    /// ⚠ A callback primitive carries no vertices, so a viewport drawn
    /// through one must not be counted as having painted.
    #[test]
    fn a_callback_contributes_no_vertices() {
        let clip = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(10.0, 10.0));
        let mut mesh = egui::epaint::Mesh::default();
        mesh.add_colored_rect(clip, egui::Color32::RED);
        let measured = measure(vec![egui::ClippedPrimitive {
            clip_rect: clip,
            primitive: egui::epaint::Primitive::Mesh(mesh),
        }]);
        assert_eq!(measured.primitives, 1);
        assert_eq!(measured.vertices, 4, "one rect is one quad");
    }
}

#[cfg(test)]
mod toasts_leave_a_unit_alone {
    use super::*;

    /// A unit is judged against another drawn a moment earlier, so a toast left from before, which goes by the wall clock, must not be drawn in it.
    #[test]
    fn a_toast_left_from_before_is_not_drawn_in_the_next_unit() {
        let mut state = AppState::new();
        let without = draw_unit(&mut state, None, 2, |_| {}).expect("a unit draws");
        state.toasts.success("Folder opened");
        let after = draw_unit(&mut state, None, 2, |_| {}).expect("a unit draws");
        assert_eq!(after.vertices, without.vertices);
        assert!(
            state.toasts.iter().next().is_none(),
            "the unit left the toast on screen"
        );
        // A unit that asks for a toast draws it
        let asked = draw_unit(&mut state, None, 2, |state| {
            state.toasts.success("Folder opened");
        })
        .expect("a unit draws");
        assert!(asked.vertices > without.vertices);
    }
}
