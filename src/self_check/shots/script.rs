//! A shots script as its steps: one a line, read whole before anything is drawn.

use std::path::PathBuf;
use std::time::Duration;

use or_renderer::ShadingMode;

use crate::state::front_door::{Step as DialogStep, Template};
use crate::state::prefs::Game;
use crate::state::{AppearancePrefs, ThemeChoice};
use crate::ui::dock::Workspace;
use crate::ui::panels::settings::SettingsCategory;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Step {
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
    /// The pointer pressed at one place and let go at another, as a drag in the viewport orbits or, with shift, pans.
    Drag {
        from: egui::Pos2,
        to: egui::Pos2,
        modifiers: egui::Modifiers,
    },
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
    /// A step of the World screen: a camera, an hour, a clean frame or a wait for what streams in.
    World(super::world::WorldStep),
    /// A shot measured against a reference image.
    Compare(super::compare::Compare),
    /// What this many frames of the open GTA VI model cost at rest, and as many with the camera turned once round it, written as JSON to this file beside the shots; with `true`, every pass drawn every frame, whatever moved.
    ModelPerf(usize, String, bool),
    /// Build the passes of the open GTA VI model again from nothing, its maps decoded already.
    ModelPasses,
}

/// Every step with its line number and text, or the first line that is wrong.
pub(super) fn parse(text: &str) -> Result<Vec<(usize, String, Step)>, String> {
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
                "gta6" => Game::GtaVI,
                other => return Err(format!("no game `{other}`; use legacy, enhanced or gta6")),
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
        "drag" => {
            let (places, held) = args.split_at(args.len().min(4));
            let [x, y, to_x, to_y] = numbers::<4>(places)?;
            Ok(Step::Drag {
                from: egui::pos2(x, y),
                to: egui::pos2(to_x, to_y),
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
            // `ctrl+s` is `s ctrl`; with nothing before its last `+` the name is the key, so `+` is Plus.
            let (joined, name) = match name.rsplit_once('+') {
                Some((joined, key)) if !joined.is_empty() => (joined, key),
                _ => ("", *name),
            };
            let held: Vec<&str> = (joined.split('+').filter(|word| !word.is_empty()))
                .chain(held.iter().copied())
                .collect();
            let key = egui::Key::from_name(name).ok_or_else(|| format!("no key `{name}`"))?;
            Ok(Step::Key {
                key,
                modifiers: modifiers(&held)?,
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
        "world" => super::world::parse(rest).map(Step::World),
        "compare" => super::compare::parse(rest).map(Step::Compare),
        "model" => {
            let (frames, name, in_full) = match args[..] {
                ["passes"] => return Ok(Step::ModelPasses),
                ["perf", frames, name] => (frames, name, false),
                ["perf", frames, name, "full"] => (frames, name, true),
                _ => return Err("use `model perf FRAMES FILE [full]` or `model passes`".into()),
            };
            let frames = (frames.parse::<usize>().ok())
                .filter(|n| (1..=10_000).contains(n))
                .ok_or_else(|| format!("`{frames}` is not a number of frames from 1 to 10000"))?;
            Ok(Step::ModelPerf(frames, file_stem(name)?, in_full))
        }
        other => Err(format!("no step `{other}`")),
    }
}

pub(super) fn split_word(text: &str) -> (&str, &str) {
    text.split_once(char::is_whitespace)
        .map_or((text, ""), |(word, rest)| (word, rest.trim()))
}

pub(super) fn numbers<const N: usize>(args: &[&str]) -> Result<[f32; N], String> {
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
pub(super) fn file_stem(name: &str) -> Result<String, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn one(line: &str) -> Result<Step, String> {
        parse(line).map(|mut steps| steps.remove(0).2)
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
    fn a_drag_takes_two_places_and_the_modifiers_held() {
        let dragged = Step::Drag {
            from: egui::pos2(10.0, 20.0),
            to: egui::pos2(30.0, 40.0),
            modifiers: egui::Modifiers::SHIFT,
        };
        assert_eq!(one("drag 10 20 30 40 shift"), Ok(dragged));
        assert!(one("drag 10 20 30").is_err());
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

    #[test]
    fn a_model_s_cost_is_asked_for_by_a_number_of_frames_and_a_file_s_name() {
        assert_eq!(
            one("model perf 120 cube.json"),
            Ok(Step::ModelPerf(120, "cube.json".into(), false))
        );
        assert_eq!(
            one("model perf 3 cube-full.json full"),
            Ok(Step::ModelPerf(3, "cube-full.json".into(), true))
        );
        assert_eq!(one("model passes"), Ok(Step::ModelPasses));
        for bad in [
            "model",
            "model perf",
            "model perf 120",
            "model perf cube.json 120",
            "model perf 0 cube.json",
            "model perf 120 ../cube.json",
            "model perf 120 cube.json whole",
            "model cost 120 cube.json",
            "model passes again",
        ] {
            assert!(one(bad).is_err(), "`{bad}` should be refused");
        }
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

    #[test]
    fn a_script_reads_the_steps_of_a_repeatable_frame_of_the_world() {
        use super::super::compare::Compare;
        use super::super::world::WorldStep;

        let script = "world clean on\nworld hour 9\nworld camera 1 2 3 45 -10\nworld settle 30\n\
                      shot city\ncompare city C:\\ref frames\\city.png 2";
        let steps: Vec<Step> = (parse(script).expect("valid").into_iter())
            .map(|(_, _, step)| step)
            .collect();
        assert_eq!(steps[0], Step::World(WorldStep::Clean(true)));
        assert_eq!(steps[1], Step::World(WorldStep::Hour(9.0)));
        assert!(matches!(steps[2], Step::World(WorldStep::Camera(_))));
        assert_eq!(
            steps[3],
            Step::World(WorldStep::Settle(Duration::from_secs(30)))
        );
        assert_eq!(steps[4], Step::Shot("city".into()));
        assert_eq!(
            steps[5],
            Step::Compare(Compare {
                shot: "city".into(),
                reference: PathBuf::from("C:\\ref frames\\city.png"),
                tolerance: 2.0,
            })
        );
        let wrong = parse("shot a\nworld hour 24").expect_err("no such hour");
        assert!(wrong.starts_with("line 2:"), "{wrong}");
    }

    #[test]
    fn a_key_joined_to_its_modifiers_is_the_key_with_them_held() {
        assert_eq!(one("key ctrl+shift+s"), one("key s ctrl shift"));
        assert_eq!(one("key ctrl+s alt"), one("key S ctrl alt"));
        let Ok(Step::Key { key, modifiers }) = one("key +") else {
            panic!("`+` alone is a key");
        };
        assert_eq!((key, modifiers), (egui::Key::Plus, egui::Modifiers::NONE));
        assert!(one("key ctrl+nothing").is_err());
        assert!(one("key meta+s").is_err());
    }
}
