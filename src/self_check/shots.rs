//! Draws the real shell with no window and saves frames as PNGs, driven by a
//! script of one step per line; `docs/guides/Development_Workflow.md` lists the steps.
//!
//! ⚠ A frame is `render_editor`'s frame: the same polls, `ui::draw`, overlays
//! and render bridge. App shortcuts are dispatched from winit, so no step
//! reaches them.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use or_renderer::ShadingMode;

mod png;
use png::{paint, save_png, target_texture};

use crate::render::RenderBridge;
use crate::state::front_door::{Offer, Step as DialogStep, Template};
use crate::state::prefs::Game;
use crate::state::{AfterClose, AppState, AppearancePrefs, LoadingState, ThemeChoice};
use crate::ui::dock::Workspace;
use crate::ui::panels::settings::{SettingsCategory, SettingsUiState};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Frames drawn before a capture: egui sizes some widgets on a second pass.
const SETTLE_FRAMES: usize = 3;

const OPEN_DEADLINE: Duration = Duration::from_secs(180);

/// Discovery runs on a worker, so a `game` step with no folder waits for it.
const DISCOVERY_DEADLINE: Duration = Duration::from_secs(30);

/// How long the `picker` step draws the start page before its shot.
const LOGO_SETTLE: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq)]
enum Step {
    Size(f32, f32),
    Game(Game, Option<PathBuf>),
    Wait(Duration),
    Open(PathBuf),
    Project(PathBuf),
    Extract(PathBuf),
    Gltf(PathBuf),
    Pick(PathBuf),
    /// A file or folder dropped from outside the window onto the pointer, as Explorer drops one.
    Drop(PathBuf),
    /// A file dropped from outside the window onto the tile of a texture variation of the garment selected in the Wardrobe, by its place in the garment.
    DropTile(usize, PathBuf),
    /// A file or folder held over the window by a drag that has not let go, as Explorer holds one; it stays until `hoverend` or a `drop`.
    HoverFile(PathBuf),
    HoverEnd,
    /// Every build of the open project started, as Build all starts them, without waiting for them.
    Build,
    /// The pollers that take a worker's result stop, so a build or a cook that has finished still reads as running, until `release`.
    Hold,
    Release,
    /// A project asked for as a recent project's card asks for it: it asks where when one is open, and nothing waits for it.
    AskProject(PathBuf),
    /// The status bar's project menu open.
    ProjectMenu,
    Workspace(Workspace),
    Reset,
    /// The shading mode the viewport draws in, as a pill picks it.
    Mode(ShadingMode),
    Click {
        at: egui::Pos2,
        button: egui::PointerButton,
        times: usize,
        modifiers: egui::Modifiers,
    },
    Hover(egui::Pos2),
    Scroll {
        at: egui::Pos2,
        dy: f32,
    },
    Type(String),
    Key {
        key: egui::Key,
        modifiers: egui::Modifiers,
    },
    Palette,
    Frames(usize),
    CloseAll,
    /// The start page raised over the shell, as View ▸ Start page does.
    StartPage,
    /// The open project closed, as Project ▸ Close project does.
    CloseProject,
    /// The New project dialog open on a template.
    NewProject(Template, DialogStep, Option<PathBuf>),
    Theme(ThemeChoice),
    Scale(u16),
    /// The interface language, as its setting chooses it: `None` follows the system's.
    Language(Option<String>),
    Shot(String),
    Picker(String),
    Settings {
        name: String,
        category: SettingsCategory,
    },
}

/// Every step with its line number and text, or the first line that is wrong.
fn parse(text: &str) -> Result<Vec<(usize, String, Step)>, String> {
    let mut steps = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let number = index + 1;
        let step = parse_step(line).map_err(|why| format!("line {number}: {why}: `{line}`"))?;
        steps.push((number, line.to_string(), step));
    }
    Ok(steps)
}

fn parse_step(line: &str) -> Result<Step, String> {
    let (word, rest) = split_word(line);
    let args: Vec<&str> = rest.split_whitespace().collect();
    let no_args = |step: Step| {
        if rest.is_empty() {
            Ok(step)
        } else {
            Err(format!("`{word}` takes no arguments"))
        }
    };
    match word {
        "size" => {
            let [w, h] = numbers::<2>(&args)?;
            if w < 320.0 || h < 240.0 {
                return Err("a screen smaller than 320×240 lays nothing out".into());
            }
            Ok(Step::Size(w, h))
        }
        "game" => {
            let (edition, folder) = split_word(rest);
            let game = match edition {
                "legacy" => Game::GtaVLegacy,
                "enhanced" => Game::GtaVEnhanced,
                other => return Err(format!("no game `{other}`; use legacy or enhanced")),
            };
            Ok(Step::Game(
                game,
                (!folder.is_empty()).then(|| PathBuf::from(folder)),
            ))
        }
        "wait" => {
            let [seconds] = numbers::<1>(&args)?;
            if seconds < 0.0 {
                return Err("a wait cannot be negative".into());
            }
            Ok(Step::Wait(Duration::from_secs_f32(seconds)))
        }
        "open" => Ok(Step::Open(PathBuf::from(non_empty(rest, "a file path")?))),
        "project" => Ok(Step::Project(PathBuf::from(non_empty(
            rest,
            "a .openrage-proj file",
        )?))),
        "extract" => Ok(Step::Extract(PathBuf::from(non_empty(rest, "a folder")?))),
        "gltf" => Ok(Step::Gltf(PathBuf::from(non_empty(
            rest,
            "a .glb or .gltf",
        )?))),
        "pick" => Ok(Step::Pick(PathBuf::from(non_empty(rest, "a path")?))),
        "drop" => Ok(Step::Drop(PathBuf::from(non_empty(
            rest,
            "a file or folder",
        )?))),
        "droptile" => {
            let (index, path) = split_word(rest);
            let index = index.parse().map_err(|_| {
                "droptile takes the number of a tile, counting from 0, and a file".to_string()
            })?;
            Ok(Step::DropTile(
                index,
                PathBuf::from(non_empty(path, "an image")?),
            ))
        }
        "hoverfile" => Ok(Step::HoverFile(PathBuf::from(non_empty(
            rest,
            "a file or folder",
        )?))),
        "hoverend" => no_args(Step::HoverEnd),
        "build" => no_args(Step::Build),
        "hold" => no_args(Step::Hold),
        "release" => no_args(Step::Release),
        "askproject" => Ok(Step::AskProject(PathBuf::from(non_empty(
            rest,
            "a .openrage-proj file",
        )?))),
        "projectmenu" => no_args(Step::ProjectMenu),
        "workspace" => Workspace::ALL
            .into_iter()
            .find(|w| w.label().eq_ignore_ascii_case(rest))
            .map(Step::Workspace)
            .ok_or_else(|| format!("no workspace `{rest}`")),
        "reset" => no_args(Step::Reset),
        "mode" => [
            ("wireframe", ShadingMode::Wireframe),
            ("solid", ShadingMode::Solid),
            ("material", ShadingMode::Material),
            ("shaded", ShadingMode::Shaded),
        ]
        .into_iter()
        .find(|(name, _)| *name == rest)
        .map(|(_, mode)| Step::Mode(mode))
        .ok_or_else(|| format!("no mode `{rest}`; use wireframe, solid, material or shaded")),
        "click" | "rclick" | "dclick" => {
            let (position, held) = args.split_at(args.len().min(2));
            let [x, y] = numbers::<2>(position)?;
            Ok(Step::Click {
                at: egui::pos2(x, y),
                button: if word == "rclick" {
                    egui::PointerButton::Secondary
                } else {
                    egui::PointerButton::Primary
                },
                times: if word == "dclick" { 2 } else { 1 },
                modifiers: modifiers(held)?,
            })
        }
        "hover" => {
            let [x, y] = numbers::<2>(&args)?;
            Ok(Step::Hover(egui::pos2(x, y)))
        }
        "scroll" => {
            let [x, y, dy] = numbers::<3>(&args)?;
            Ok(Step::Scroll {
                at: egui::pos2(x, y),
                dy,
            })
        }
        "type" => Ok(Step::Type(non_empty(rest, "text")?.to_string())),
        "key" => {
            let (name, held) = args.split_first().ok_or("needs a key name")?;
            let key = egui::Key::from_name(name).ok_or_else(|| format!("no key `{name}`"))?;
            Ok(Step::Key {
                key,
                modifiers: modifiers(held)?,
            })
        }
        "palette" => no_args(Step::Palette),
        "frames" => {
            let [n] = numbers::<1>(&args)?;
            if !(1.0..=10_000.0).contains(&n) || n.fract() != 0.0 {
                return Err("a frame count is a whole number from 1 to 10000".into());
            }
            Ok(Step::Frames(n as usize))
        }
        "closeall" => no_args(Step::CloseAll),
        "startpage" => no_args(Step::StartPage),
        "closeproject" => no_args(Step::CloseProject),
        "newproject" => {
            let (template, after) = split_word(rest);
            let (step, folder) = split_word(after);
            let template = Template::ALL
                .into_iter()
                // ⚠ `adopt` is the template's name in code and in older scripts, which its label no longer starts with.
                .find(|t| {
                    t.label().to_lowercase().starts_with(template)
                        || (template == "adopt" && *t == Template::Adopt)
                })
                .filter(|_| !template.is_empty())
                .ok_or_else(|| {
                    "use newproject empty, files, clothing, weapon, particle, map or adopt"
                        .to_string()
                })?;
            match step {
                "" if folder.is_empty() => {
                    Ok(Step::NewProject(template, DialogStep::Gallery, None))
                }
                "details" => Ok(Step::NewProject(
                    template,
                    DialogStep::Details,
                    (!folder.is_empty()).then(|| PathBuf::from(folder)),
                )),
                other => Err(format!(
                    "`{other}` is not a step; use details, then an optional folder"
                )),
            }
        }
        "theme" => match rest {
            "dark" => Ok(Step::Theme(ThemeChoice::Dark)),
            "light" => Ok(Step::Theme(ThemeChoice::Light)),
            other => Err(format!("no theme `{other}`; use dark or light")),
        },
        "language" => match rest {
            "system" => Ok(Step::Language(None)),
            code if crate::i18n::languages()
                .iter()
                .any(|l| l.code.eq_ignore_ascii_case(code)) =>
            {
                Ok(Step::Language(Some(code.to_string())))
            }
            other => Err(format!(
                "no language `{other}`; use system or a code such as en or fr"
            )),
        },
        "scale" => {
            let percent: u16 = rest
                .parse()
                .map_err(|_| format!("`{rest}` is not a percentage"))?;
            if !AppearancePrefs::SCALE_STEPS.contains(&percent) {
                return Err(format!(
                    "the interface scales in steps of {:?}",
                    AppearancePrefs::SCALE_STEPS
                ));
            }
            Ok(Step::Scale(percent))
        }
        "shot" => Ok(Step::Shot(file_stem(rest)?)),
        "picker" => Ok(Step::Picker(file_stem(rest)?)),
        "settings" => {
            let (name, label) = split_word(rest);
            let category = SettingsCategory::ALL
                .into_iter()
                .find(|c| c.label().eq_ignore_ascii_case(label))
                .ok_or_else(|| format!("no settings page `{label}`"))?;
            Ok(Step::Settings {
                name: file_stem(name)?,
                category,
            })
        }
        other => Err(format!("no step `{other}`")),
    }
}

fn split_word(text: &str) -> (&str, &str) {
    text.split_once(char::is_whitespace)
        .map_or((text, ""), |(word, rest)| (word, rest.trim()))
}

fn numbers<const N: usize>(args: &[&str]) -> Result<[f32; N], String> {
    if args.len() != N {
        return Err(format!("needs {N} number(s), found {}", args.len()));
    }
    let mut out = [0.0; N];
    for (slot, arg) in out.iter_mut().zip(args) {
        *slot = arg
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| format!("`{arg}` is not a number"))?;
    }
    Ok(out)
}

fn non_empty<'a>(text: &'a str, what: &str) -> Result<&'a str, String> {
    if text.is_empty() {
        Err(format!("needs {what}"))
    } else {
        Ok(text)
    }
}

fn modifiers(words: &[&str]) -> Result<egui::Modifiers, String> {
    let mut held = egui::Modifiers::NONE;
    for word in words {
        match *word {
            "ctrl" => {
                held.ctrl = true;
                held.command = true;
            }
            "shift" => held.shift = true,
            "alt" => held.alt = true,
            other => return Err(format!("no modifier `{other}`; use ctrl, shift or alt")),
        }
    }
    Ok(held)
}

/// ⚠ A name, not a path, so every PNG lands in the output directory.
fn file_stem(name: &str) -> Result<String, String> {
    let ok = !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(name.to_string())
    } else {
        Err(format!(
            "`{name}` is not a file name: use letters, digits, '-', '_' and '.'"
        ))
    }
}

/// The exit code.
pub fn run(script: &Path, out: &Path) -> i32 {
    match run_script(script, out) {
        Ok(()) => 0,
        Err(why) => {
            println!("{why}");
            1
        }
    }
}

fn run_script(script: &Path, out: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(script)
        .map_err(|e| format!("cannot read {}: {e}", script.display()))?;
    // Parsed whole before anything is drawn, so a typo on the last line
    // fails before a long run rather than after it.
    let steps = parse(&text)?;
    std::fs::create_dir_all(out).map_err(|e| format!("cannot create {}: {e}", out.display()))?;

    let mut state = AppState::new();
    let (device, queue) = super::headless_gpu()?;
    let mut shell = Shell::on(&state, device, queue);
    for (number, line, step) in &steps {
        println!("> {line}");
        shell
            .step(&mut state, step, out)
            .map_err(|why| format!("line {number}: {why}"))?;
    }
    Ok(())
}

/// What a script is told once the device its frames are drawn on is lost.
const DEVICE_LOST: &str = "the graphics device was lost: nothing more is drawn in this run";

impl Shell {
    /// The editor's frames on `device`, as a script drives them.
    fn on(state: &AppState, device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Self {
        let mut bridge = RenderBridge::new(device.clone(), queue.clone(), FORMAT);
        // A pack baked elsewhere is named by `OPENRAGE_GAME_PACK`; without one the mode is not offered
        bridge.set_game_pack_path(state.game_pack.root());
        Shell {
            bridge,
            renderer: egui_wgpu::Renderer::new(
                &device,
                FORMAT,
                egui_wgpu::RendererOptions::default(),
            ),
            ctx: crate::app::gpu::configured_egui_context(Some(8192), &state.prefs.appearance),
            device,
            queue,
            size: egui::vec2(1600.0, 900.0),
            time: 0.0,
            target: None,
            dropped: Vec::new(),
            hovering: Vec::new(),
            held: false,
        }
    }
}

struct Shell {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    bridge: RenderBridge,
    renderer: egui_wgpu::Renderer,
    ctx: egui::Context,
    size: egui::Vec2,
    time: f64,
    /// Files the next frame receives as dropped from outside the window.
    dropped: Vec<PathBuf>,
    /// Files held over the window by a drag, which every frame reports until the drag ends.
    hovering: Vec<PathBuf>,
    /// The pollers that take a worker's result do not run: a `hold`.
    held: bool,
    target: Option<(u32, u32, wgpu::Texture)>,
}

impl Shell {
    fn step(&mut self, state: &mut AppState, step: &Step, out: &Path) -> Result<(), String> {
        match step {
            Step::Size(w, h) => self.size = egui::vec2(*w, *h),
            Step::Game(game, folder) => {
                let root = match folder {
                    Some(folder) => folder.clone(),
                    None => self.discovered(state, *game)?,
                };
                activate_game_and_wait(state, *game, &root, |state| self.idle(state, 1))?;
            }
            Step::Wait(how_long) => {
                let until = Instant::now() + *how_long;
                while Instant::now() < until {
                    self.idle(state, 1)?;
                    std::thread::sleep(Duration::from_millis(30));
                }
            }
            Step::Open(path) => self.open(state, path)?,
            Step::Project(path) => self.project(state, path)?,
            // The folder picker is the platform's, so the script names the folder it would return.
            Step::Extract(dest) => {
                let doc = state
                    .active_document
                    .ok_or("nothing is open to extract from")?;
                let rows = state.selection.selected_ids(doc);
                if rows.is_empty() {
                    return Err("no row is selected to extract".into());
                }
                state.extract_rpf_nodes(
                    doc,
                    rows,
                    dest.clone(),
                    or_io::container::rpf::BodyForm::Decoded,
                    crate::state::TypeFilter::All,
                );
            }
            // The file picker is the platform's, so the script names the glTF it would return.
            Step::Gltf(mesh) => {
                let doc = state
                    .active_document
                    .ok_or("no model is open to import into")?;
                state.import_gltf(doc, mesh.clone());
            }
            Step::Pick(path) => crate::platform::file_dialogs::answer_next(path.clone()),
            Step::Drop(path) => {
                // On the panel of the screen that takes drops when it is drawn, else where the pointer last was.
                let panel = match state.active_workspace {
                    Workspace::Weapons => crate::ui::panels::weapons::drop_rect(state),
                    Workspace::Map => crate::ui::panels::maps::drop_rect(state),
                    Workspace::Particle => crate::ui::panels::particles::drop_rect(state),
                    _ => crate::ui::panels::wardrobe::drop_rect(state),
                };
                let at = panel.map_or(egui::pos2(400.0, 300.0), |rect| rect.center());
                self.frame(state, vec![egui::Event::PointerMoved(at)], None)?;
                // A drop ends the drag that held it over the window.
                self.hovering.clear();
                self.dropped.push(path.clone());
                self.idle(state, 1)?;
            }
            Step::DropTile(index, path) => {
                let garment = state
                    .workbench
                    .wardrobe
                    .selected
                    .as_ref()
                    .map(|(_, id)| id.clone());
                let at = garment
                    .and_then(|id| crate::ui::panels::wardrobe::variant_rect(state, &id, *index))
                    .map(|rect| rect.center())
                    .ok_or_else(|| {
                        format!(
                            "no tile {index} is on screen: select a garment in the Wardrobe first"
                        )
                    })?;
                self.frame(state, vec![egui::Event::PointerMoved(at)], None)?;
                self.hovering.clear();
                self.dropped.push(path.clone());
                self.idle(state, 1)?;
            }
            Step::HoverFile(path) => {
                let at = crate::ui::panels::wardrobe::drop_rect(state)
                    .map_or(egui::pos2(400.0, 300.0), |rect| rect.center());
                self.hovering = vec![path.clone()];
                self.frame(state, vec![egui::Event::PointerMoved(at)], None)?;
            }
            Step::HoverEnd => self.hovering.clear(),
            Step::Build => state.build_all(),
            Step::Hold => self.held = true,
            Step::Release => self.held = false,
            Step::AskProject(path) => state.open_project_asking(path.clone()),
            Step::ProjectMenu => {
                state
                    .panels
                    .get::<crate::ui::panels::status_bar::ProjectMenu>()
                    .open = true;
            }
            Step::Workspace(ws) => state.active_workspace = *ws,
            Step::Reset => {
                let ws = state.active_workspace;
                state.workspace_docks.reset(ws);
            }
            Step::Mode(mode) => {
                self.bridge.set_mode(*mode);
                state.remember_shading_mode(*mode);
            }
            Step::Click {
                at,
                button,
                times,
                modifiers,
            } => {
                for _ in 0..*times {
                    let moved = vec![egui::Event::PointerMoved(*at)];
                    self.frame_with(state, moved, *modifiers, None)?;
                    for pressed in [true, false] {
                        let event = egui::Event::PointerButton {
                            pos: *at,
                            button: *button,
                            pressed,
                            modifiers: *modifiers,
                        };
                        self.frame_with(state, vec![event], *modifiers, None)?;
                    }
                }
            }
            Step::Hover(at) => {
                self.frame(state, vec![egui::Event::PointerMoved(*at)], None)?;
                // Long enough for a tooltip to appear.
                self.idle(state, 30)?;
            }
            Step::Scroll { at, dy } => {
                self.frame(state, vec![egui::Event::PointerMoved(*at)], None)?;
                let wheel = egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, *dy),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::NONE,
                };
                self.frame(state, vec![wheel], None)?;
            }
            Step::Type(text) => self.frame(state, vec![egui::Event::Text(text.clone())], None)?,
            Step::Key { key, modifiers } => {
                for pressed in [true, false] {
                    let event = egui::Event::Key {
                        key: *key,
                        physical_key: Some(*key),
                        pressed,
                        repeat: false,
                        modifiers: *modifiers,
                    };
                    self.frame_with(state, vec![event], *modifiers, None)?;
                }
            }
            Step::Palette => crate::ui::panels::command_palette::open(state),
            Step::Frames(n) => self.idle(state, *n)?,
            Step::CloseAll => {
                let ids: Vec<_> = state.documents.iter().map(|d| d.id).collect();
                state.request_close(ids, AfterClose::Nothing);
            }
            Step::StartPage => state.show_start_page(),
            Step::CloseProject => state.close_project(),
            Step::NewProject(template, step, folder) => {
                state.start_new_project_on(*template, *step);
                // A folder to adopt is chosen as the picker would choose it.
                if let (Some(folder), Some(draft)) = (folder, state.front_door.new_project.as_mut())
                {
                    draft.pick_place(folder);
                }
            }
            Step::Theme(theme) => {
                state.prefs.appearance.theme = *theme;
                crate::app::gpu::apply_appearance(&self.ctx, &state.prefs.appearance);
            }
            Step::Language(code) => crate::app::gpu::use_language(&self.ctx, state, code.clone()),
            Step::Scale(percent) => {
                state.prefs.appearance.ui_scale_percent = *percent;
                crate::app::gpu::apply_appearance(&self.ctx, &state.prefs.appearance);
            }
            Step::Shot(name) => {
                self.idle(state, SETTLE_FRAMES)?;
                let path = out.join(format!("{name}.png"));
                self.frame(state, Vec::new(), Some(&path))?;
                println!("shot  {}", path.display());
            }
            Step::Picker(name) => {
                let path = out.join(format!("{name}.png"));
                // The start page is what Browse's Document panel draws while nothing is open.
                let open = std::mem::take(&mut state.documents);
                let workspace = std::mem::replace(&mut state.active_workspace, Workspace::Browse);
                // ⚠ Drawn for time, not frames: its logos load from disk on a
                // worker, and a few frames shoot their spinners instead.
                let until = Instant::now() + LOGO_SETTLE;
                let mut shot = Ok(());
                while shot.is_ok() && Instant::now() < until {
                    shot = self.idle(state, 1);
                    std::thread::sleep(Duration::from_millis(30));
                }
                let shot = shot.and_then(|()| self.frame(state, Vec::new(), Some(&path)));
                state.active_workspace = workspace;
                let opened = std::mem::replace(&mut state.documents, open);
                state.documents.extend(opened);
                shot?;
                println!("shot  {}", path.display());
            }
            Step::Settings { name, category } => {
                state.panels.get::<SettingsUiState>().category = *category;
                let path = out.join(format!("{name}.png"));
                self.window_shot(
                    state,
                    crate::app::SETTINGS_WINDOW_SIZE,
                    &path,
                    crate::ui::panels::settings::draw_settings_window_body,
                )?;
                println!("shot  {}", path.display());
            }
        }
        // Every step ends on drawn frames, so the next one acts on a settled screen.
        self.idle(state, 2)
    }

    fn discovered(&mut self, state: &mut AppState, game: Game) -> Result<PathBuf, String> {
        let until = Instant::now() + DISCOVERY_DEADLINE;
        loop {
            if let Some(root) = state.prefs.game_paths.get(game) {
                return Ok(root.clone());
            }
            if Instant::now() > until {
                return Err(format!(
                    "no {} install was found; give its folder after the game",
                    game.label()
                ));
            }
            self.idle(state, 1)?;
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    fn open(&mut self, state: &mut AppState, path: &Path) -> Result<(), String> {
        open_and_wait(state, path, |state| self.idle(state, 1))?;
        // The banner over an opened folder says what the folder holds once a worker has read it.
        let until = Instant::now() + DISCOVERY_DEADLINE;
        while let Some(Offer::Make(folder)) = state.adoption_offer() {
            if state.front_door.previews.of(&folder).is_some() || Instant::now() > until {
                break;
            }
            self.idle(state, 1)?;
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    /// Opens a project as Open Project does once its file is picked, and waits for its files to be read.
    fn project(&mut self, state: &mut AppState, path: &Path) -> Result<(), String> {
        state.load_project_from(path.to_path_buf());
        let until = Instant::now() + OPEN_DEADLINE;
        while state.project_path.as_deref() != Some(path)
            || !state.workbench.is_scanned()
            || matches!(state.loading, LoadingState::Loading)
        {
            if Instant::now() > until {
                return Err(format!(
                    "{} was still opening after three minutes",
                    path.display()
                ));
            }
            self.idle(state, 1)?;
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    fn idle(&mut self, state: &mut AppState, frames: usize) -> Result<(), String> {
        for _ in 0..frames {
            self.frame(state, Vec::new(), None)?;
        }
        Ok(())
    }

    fn frame(
        &mut self,
        state: &mut AppState,
        events: Vec<egui::Event>,
        capture: Option<&Path>,
    ) -> Result<(), String> {
        self.frame_with(state, events, egui::Modifiers::NONE, capture)
    }

    /// ⚠ Polls as the editor's redraw does, except the shader-pack bake: a
    /// bake downloads its tools and runs for minutes, so a script never starts one.
    fn frame_with(
        &mut self,
        state: &mut AppState,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
        capture: Option<&Path>,
    ) -> Result<(), String> {
        // ⚠ Before anything is drawn: the painter panics on a buffer a lost device does not write.
        if self.bridge.device_lost() {
            state.graphics_device_lost(self.bridge.mode());
            self.bridge.set_mode(ShadingMode::Material);
            return Err(DEVICE_LOST.to_owned());
        }
        state.poll_load_result();
        state.poll_save_result();
        // No dialog opens in a script, so a document never saved stays unsaved and its close is called off.
        state.drive_close(|_, _| None);
        state.poll_key_extraction();
        state.poll_material_palette_load();
        state.poll_clip_regions_load();
        state.poll_shader_definitions();
        state.poll_game_discovery();
        state.poll_expansions();
        state.tick_navigation();
        state.poll_extract_job();
        // ⚠ A hold skips every poller of `poll_jobs`: the workbench's takes the build's and the cook's results.
        if !self.held {
            state.poll_jobs();
        }
        state.update_toasts();

        self.bridge.begin_frame();
        state.tick_previews();
        self.time += 1.0 / 60.0;
        self.ctx.begin_pass(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
            max_texture_side: Some(8192),
            time: Some(self.time),
            predicted_dt: 1.0 / 60.0,
            modifiers,
            events,
            focused: true,
            hovered_files: (self.hovering.iter())
                .map(|path| egui::HoveredFile {
                    path: Some(path.clone()),
                    mime: String::new(),
                })
                .collect(),
            dropped_files: std::mem::take(&mut self.dropped)
                .into_iter()
                .map(|path| egui::DroppedFile {
                    path: Some(path),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        });
        crate::ui::draw(&self.ctx, state, Some(&mut self.bridge));
        crate::app::modals::draw_editor_overlays(&self.ctx, state);
        let output = self.ctx.end_pass();
        // The editor's window puts copied text on the system clipboard; a script only says it.
        for command in &output.platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                tracing::info!("the clipboard's stand-in was asked to copy {text:?}");
            }
        }
        let ppp = output.pixels_per_point;
        let primitives = self.ctx.tessellate(output.shapes, ppp);
        let (device, queue) = (self.device.clone(), self.queue.clone());
        for (id, delta) in &output.textures_delta.set {
            self.renderer.update_texture(&device, &queue, *id, delta);
        }

        let w = (self.size.x * ppp).round() as u32;
        let h = (self.size.y * ppp).round() as u32;
        if !matches!(&self.target, Some((tw, th, _)) if (*tw, *th) == (w, h)) {
            self.target = Some((w, h, target_texture(&device, w, h)));
        }
        let Some((_, _, target)) = &self.target else {
            return Err("no render target".into());
        };
        let view = target.create_view(&Default::default());
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [w, h],
            pixels_per_point: ppp,
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        self.renderer
            .update_buffers(&device, &queue, &mut encoder, &primitives, &screen);
        let dirty = self.bridge.end_frame(&mut encoder, state);
        if dirty || self.bridge.texture_id().is_none() {
            if let Some(previous) = self.bridge.texture_id() {
                self.renderer.free_texture(&previous);
            }
            let id = self.renderer.register_native_texture(
                &device,
                self.bridge.colour_texture_view(),
                wgpu::FilterMode::Linear,
            );
            self.bridge.set_texture_id(id);
        }
        for closed in state.closed_documents.drain(..) {
            self.bridge
                .forget_document_thumbnails(closed, &mut self.renderer);
        }
        self.bridge
            .render_thumbnails(&mut encoder, state, &mut self.renderer);
        paint(
            &mut self.renderer,
            &mut encoder,
            &view,
            crate::app::CLEAR_COLOR,
            &primitives,
            &screen,
        );
        queue.submit(std::iter::once(encoder.finish()));
        let saved = match capture {
            Some(path) => save_png(&device, &queue, target, w, h, path),
            None => Ok(()),
        };
        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
        saved
    }

    /// A window the editor opens on its own surface, Settings, drawn with its
    /// own context and renderer as that surface has.
    fn window_shot(
        &mut self,
        state: &mut AppState,
        size: (u32, u32),
        path: &Path,
        mut draw: impl FnMut(&egui::Context, &mut AppState),
    ) -> Result<(), String> {
        if self.bridge.device_lost() {
            return Err(DEVICE_LOST.to_owned());
        }
        let ctx = crate::app::gpu::configured_egui_context(Some(8192), &state.prefs.appearance);
        let mut renderer =
            egui_wgpu::Renderer::new(&self.device, FORMAT, egui_wgpu::RendererOptions::default());
        let points = egui::vec2(size.0 as f32, size.1 as f32);
        for frame in 0..=SETTLE_FRAMES {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, points)),
                max_texture_side: Some(8192),
                time: Some(frame as f64 / 60.0),
                focused: true,
                ..Default::default()
            });
            draw(&ctx, state);
            let output = ctx.end_pass();
            let ppp = output.pixels_per_point;
            let primitives = ctx.tessellate(output.shapes, ppp);
            for (id, delta) in &output.textures_delta.set {
                renderer.update_texture(&self.device, &self.queue, *id, delta);
            }
            let w = (points.x * ppp).round() as u32;
            let h = (points.y * ppp).round() as u32;
            let target = target_texture(&self.device, w, h);
            let view = target.create_view(&Default::default());
            let screen = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [w, h],
                pixels_per_point: ppp,
            };
            let mut encoder = self.device.create_command_encoder(&Default::default());
            renderer.update_buffers(
                &self.device,
                &self.queue,
                &mut encoder,
                &primitives,
                &screen,
            );
            paint(
                &mut renderer,
                &mut encoder,
                &view,
                crate::app::SETTINGS_CLEAR_COLOR,
                &primitives,
                &screen,
            );
            self.queue.submit(std::iter::once(encoder.finish()));
            if frame == SETTLE_FRAMES {
                save_png(&self.device, &self.queue, &target, w, h, path)?;
            }
            for id in &output.textures_delta.free {
                renderer.free_texture(id);
            }
        }
        Ok(())
    }
}

/// Make `game`'s install at `root` active as the start page does, drawing a frame through `frame` until its rows are read.
pub(crate) fn activate_game_and_wait(
    state: &mut AppState,
    game: Game,
    root: &Path,
    frame: impl FnMut(&mut AppState) -> Result<(), String>,
) -> Result<(), String> {
    state.set_active_game(game, root.to_path_buf());
    wait_for_rows(state, root, frame)
}

/// Open `path` as the app does, drawing a frame through `frame` until it has loaded and its rows are read.
pub(crate) fn open_and_wait(
    state: &mut AppState,
    path: &Path,
    frame: impl FnMut(&mut AppState) -> Result<(), String>,
) -> Result<(), String> {
    if path.is_dir() {
        state.open_folder(path.to_path_buf());
    } else {
        state.load_file(path.to_path_buf());
    }
    wait_for_rows(state, path, frame)?;
    match state
        .documents
        .iter()
        .find(|d| d.path.as_deref() == Some(path))
    {
        Some(doc) => println!("  opened as {:?}", doc.kind),
        // Not an error: a script may open a broken file to draw the dialog.
        None => println!("  did not open: {}", state.error_dialog_message),
    }
    Ok(())
}

/// Draw frames through `frame` until `path`'s document is no longer loading or being listed.
///
/// ⚠ Open Folder and an install's root return with their rows still being read on a worker, and
/// `loading` does not say so: a click straight after would land on "Reading this folder…".
fn wait_for_rows(
    state: &mut AppState,
    path: &Path,
    mut frame: impl FnMut(&mut AppState) -> Result<(), String>,
) -> Result<(), String> {
    let until = Instant::now() + OPEN_DEADLINE;
    loop {
        let opened = state
            .documents
            .iter()
            .find(|d| d.path.as_deref() == Some(path));
        let reading = opened.is_some_and(|doc| state.expander.is_listing(doc.id));
        if !reading && !matches!(state.loading, LoadingState::Loading) {
            return Ok(());
        }
        if Instant::now() > until {
            return Err(format!(
                "{} was still loading after three minutes",
                path.display()
            ));
        }
        frame(state)?;
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(line: &str) -> Result<Step, String> {
        parse(line).map(|mut steps| steps.remove(0).2)
    }

    #[test]
    fn a_script_ends_with_a_reason_and_no_panic_once_its_device_is_lost() {
        const WHAT: &str = "a_script_ends_with_a_reason_and_no_panic_once_its_device_is_lost";
        let Some((device, queue)) = crate::render::test_device(WHAT) else {
            return;
        };
        let _language = crate::i18n::pin("en");
        let mut state = AppState::new();
        let device = Arc::new(device);
        let mut shell = Shell::on(&state, device.clone(), Arc::new(queue));
        shell
            .idle(&mut state, 2)
            .expect("frames on a device that is there");
        assert!(!shell.bridge.device_lost());
        shell.bridge.set_mode(ShadingMode::Shaded);
        // A device destroyed is a device lost: its next poll tells whoever watches it.
        device.destroy();
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        assert!(shell.bridge.device_lost());
        assert_eq!(shell.idle(&mut state, 1), Err(DEVICE_LOST.to_owned()));
        assert_eq!(shell.bridge.mode(), ShadingMode::Material);
        let told = |toast: &crate::state::toasts::Toast| toast.message.contains("device was lost");
        assert!(state.toasts.iter().any(told));
    }

    #[test]
    fn a_script_reads_every_step_with_its_line() {
        let script = "# a comment\n\nsize 1280 720\ngame legacy D:\\Games\\GTA V\nopen C:\\a b\\x.ydr\n\
                      workspace model\ndclick 14 144\nkey Escape ctrl shift\nsettings games Games\nshot first\n\
                      extract C:\\out dir";
        let steps = parse(script).expect("valid");
        let lines: Vec<usize> = steps.iter().map(|(n, _, _)| *n).collect();
        assert_eq!(lines, [3, 4, 5, 6, 7, 8, 9, 10, 11]);
        assert_eq!(steps[0].2, Step::Size(1280.0, 720.0));
        assert_eq!(
            steps[1].2,
            Step::Game(Game::GtaVLegacy, Some(PathBuf::from("D:\\Games\\GTA V")))
        );
        assert_eq!(steps[2].2, Step::Open(PathBuf::from("C:\\a b\\x.ydr")));
        assert_eq!(steps[3].2, Step::Workspace(Workspace::Model));
        assert_eq!(
            steps[4].2,
            Step::Click {
                at: egui::pos2(14.0, 144.0),
                button: egui::PointerButton::Primary,
                times: 2,
                modifiers: egui::Modifiers::NONE
            }
        );
        let Step::Key { key, modifiers } = &steps[5].2 else {
            panic!("a key step: {:?}", steps[5].2);
        };
        assert_eq!(*key, egui::Key::Escape);
        assert!(modifiers.ctrl && modifiers.shift && !modifiers.alt);
        assert_eq!(
            steps[6].2,
            Step::Settings {
                name: "games".into(),
                category: SettingsCategory::Games
            }
        );
        assert_eq!(steps[7].2, Step::Shot("first".into()));
        assert_eq!(steps[8].2, Step::Extract(PathBuf::from("C:\\out dir")));
    }

    #[test]
    fn a_mode_step_names_a_shading_mode_and_a_wrong_one_is_refused() {
        for (name, mode) in [
            ("wireframe", ShadingMode::Wireframe),
            ("solid", ShadingMode::Solid),
            ("material", ShadingMode::Material),
            ("shaded", ShadingMode::Shaded),
        ] {
            assert_eq!(one(&format!("mode {name}")), Ok(Step::Mode(mode)));
        }
        assert!(
            one("mode channel").is_err(),
            "a channel is not a mode to ask for"
        );
        assert!(one("mode").is_err());
    }

    #[test]
    fn a_click_takes_the_modifiers_a_key_does() {
        for word in ["click", "rclick", "dclick"] {
            let Ok(Step::Click { at, modifiers, .. }) = one(&format!("{word} 10 20 alt shift"))
            else {
                panic!("`{word}` with Alt and Shift held");
            };
            assert_eq!(at, egui::pos2(10.0, 20.0));
            assert!(
                modifiers.alt && modifiers.shift && !modifiers.ctrl,
                "{word}"
            );
        }
        assert!(
            one("click 10 20").is_ok_and(|step| matches!(
                step,
                Step::Click { modifiers, .. } if modifiers == egui::Modifiers::NONE
            )),
            "a click names no modifier unless it is given one"
        );
        for bad in ["click 10 alt", "click alt 10 20", "click 10 20 meta"] {
            assert!(one(bad).is_err(), "`{bad}` should be refused");
        }
    }

    #[test]
    fn the_front_door_steps_name_a_template_and_a_template_that_does_not_exist_is_refused() {
        assert_eq!(one("startpage"), Ok(Step::StartPage));
        assert_eq!(one("closeproject"), Ok(Step::CloseProject));
        for (word, template) in [
            ("empty", Template::Empty),
            ("files", Template::Files),
            ("clothing", Template::Clothing),
            ("map", Template::Maps),
            ("adopt", Template::Adopt),
        ] {
            assert_eq!(
                one(&format!("newproject {word}")),
                Ok(Step::NewProject(template, DialogStep::Gallery, None))
            );
        }
        for bad in [
            "newproject",
            "newproject vehicle",
            "newproject files soon",
            "startpage now",
        ] {
            assert!(one(bad).is_err(), "`{bad}` should be refused");
        }
        assert_eq!(
            one("newproject clothing details"),
            Ok(Step::NewProject(
                Template::Clothing,
                DialogStep::Details,
                None
            ))
        );
        assert_eq!(
            one("newproject adopt details C:\\a b"),
            Ok(Step::NewProject(
                Template::Adopt,
                DialogStep::Details,
                Some(PathBuf::from("C:\\a b"))
            ))
        );
    }

    #[test]
    fn the_steps_that_draw_a_drag_a_running_build_and_a_question_are_read() {
        assert_eq!(
            one("hoverfile C:\\a b\\p.openrage-proj"),
            Ok(Step::HoverFile(PathBuf::from("C:\\a b\\p.openrage-proj")))
        );
        assert_eq!(
            one("askproject C:\\a b\\p.openrage-proj"),
            Ok(Step::AskProject(PathBuf::from("C:\\a b\\p.openrage-proj")))
        );
        for (word, step) in [
            ("hoverend", Step::HoverEnd),
            ("build", Step::Build),
            ("hold", Step::Hold),
            ("release", Step::Release),
            ("projectmenu", Step::ProjectMenu),
        ] {
            assert_eq!(one(word), Ok(step));
            assert!(
                one(&format!("{word} now")).is_err(),
                "`{word}` takes no argument"
            );
        }
        for bad in ["hoverfile", "askproject"] {
            assert!(one(bad).is_err(), "`{bad}` names a path");
        }
    }

    #[test]
    fn a_pick_step_names_the_file_the_next_dialog_answers() {
        assert_eq!(
            one("pick C:\\a b\\out.glb"),
            Ok(Step::Pick(PathBuf::from("C:\\a b\\out.glb")))
        );
        assert!(one("pick").is_err(), "a pick names a path");
    }

    #[test]
    fn a_game_step_may_leave_the_folder_to_discovery() {
        assert_eq!(
            one("game enhanced"),
            Ok(Step::Game(Game::GtaVEnhanced, None))
        );
        assert!(
            one("game rdr2").is_err(),
            "the step names GTA V's editions only"
        );
    }

    #[test]
    fn a_wrong_line_is_named_before_anything_runs() {
        let err = parse("size 800 600\nshot ok\nzoom 2").expect_err("no such step");
        assert!(err.starts_with("line 3:"), "{err}");
        assert!(err.contains("zoom"), "{err}");
    }

    #[test]
    fn arguments_are_checked() {
        for bad in [
            "size 800",
            "size 800 wide",
            "size 100 100",
            "click 1 inf",
            "wait -1",
            "frames 0",
            "frames 2.5",
            "open",
            "extract",
            "gltf",
            "reset now",
            "key Nope",
            "key A meta",
            "theme blue",
            "scale 130",
            "workspace Nowhere",
            "settings x Nowhere",
        ] {
            assert!(one(bad).is_err(), "`{bad}` should be refused");
        }
        assert_eq!(one("scale 150"), Ok(Step::Scale(150)));
        assert_eq!(one("wait 1.5"), Ok(Step::Wait(Duration::from_millis(1500))));
    }

    /// ⚠ The name is joined to the output directory, so a separator or a
    /// leading dot would write somewhere else.
    #[test]
    fn a_shot_is_named_not_pathed() {
        for bad in ["", "../escape", "a/b", "a\\b", ".hidden", "C:x"] {
            assert!(file_stem(bad).is_err(), "`{bad}` should be refused");
        }
        assert_eq!(file_stem("02-ws-texture.v2"), Ok("02-ws-texture.v2".into()));
    }
}
