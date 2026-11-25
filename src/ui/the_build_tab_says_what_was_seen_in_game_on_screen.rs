//! The Build tab's reading of the acceptance sheet, as it is drawn after a build: the kinds of file the build wrote that nobody
//! recorded seeing the game load, the day all of them were seen, the link to the steps that check a build, and the guide it opens.
//!
//! ⚠ The map and the type file are `or_maps::sample`'s and hold no game content, and the sheet with a result is the test's own copy:
//! nothing here ran the game, and no result is written into the sheet the tree ships.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

use egui::epaint::ClippedShape;
use or_maps::sample::{map_file, types_file, Archetype, Entity};
use or_project::acceptance::{Seen, Sheet, SHEET};
use or_project::{OutputKind, Target};

use super::frames_of_the_shell::{key, Shell};
use super::the_build_tab_on_screen::{a_files_project, a_part};
use super::the_front_door_on_screen::{a_first_launch, click_on};
use super::the_project_screens_on_screen::{names, shows, until, write};
use crate::state::front_door::GuideDocument;
use crate::state::workbench::{BuiltPart, BuiltSummary, LastBuild};
use crate::ui::commands::{self, Action};
use crate::ui::panels::guide;
use crate::ui::panels::project::in_game::{sentence, IN_STORY_MODE, LINK, ON_A_SERVER};

/// The id of the build the project of these tests declares.
const OUTPUT: &str = "fivem";
const MAP: &str = "harbour.ymap";
const TYPES: &str = "harbour_types.ytyp";
const EDITOR: (u32, u32) = crate::app::EDITOR_SIZE;
/// A window narrower than the smallest the screens are held to, where the tab has the least room a window leaves it.
const NARROW: (u32, u32) = (700, 600);

/// The files of [`a_files_project`] with a map and a type file beside its script, and the build written for `edition`.
fn a_map_project(dir: &Path, edition: Option<Target>) -> Shell {
    let mut shell = a_files_project(dir);
    let file = shell.state.project_path.clone().expect("a project");
    let folder = or_project::project_folder(&file);
    let map = map_file(
        "harbour",
        &[Entity::at("prop_crate_a", [10.0, 20.0, 0.5], 1)],
        ([8.0, 18.0, -1.0], [16.0, 22.0, 2.0]),
    );
    let types = types_file(
        "harbour_types",
        &[Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3])],
    );
    let added = shell.state.add_files_to_project(vec![
        write(&folder.join("harbour.ymap"), &map),
        write(&folder.join("harbour_types.ytyp"), &types),
    ]);
    assert_eq!((added.added, &added.refused), (2, &Vec::new()));
    until(&mut shell, "the plan with the map and the type file", |s| {
        (s.state.workbench.resources_of(OUTPUT).first()).is_some_and(|resource| {
            [MAP, TYPES]
                .iter()
                .all(|path| resource.plan.files.iter().any(|file| file.path == *path))
        })
    });
    written_for(&mut shell, edition);
    shell
}

/// State the edition the build ships for, and wait for the plan that says it.
fn written_for(shell: &mut Shell, edition: Option<Target>) {
    shell
        .state
        .edit_build(OUTPUT, "the edition".into(), |output| {
            output.target = edition
        })
        .expect("the build takes an edition");
    until(shell, "the plan for the edition", |s| {
        (s.state.workbench.resources_of(OUTPUT).first())
            .is_some_and(|resource| resource.plan.target == edition)
    });
}

/// Build, draw frames at `size` until a build that ended after `after` is there, and say when it ended.
fn build(
    shell: &mut Shell,
    size: (u32, u32),
    after: Option<std::time::SystemTime>,
) -> std::time::SystemTime {
    shell.state.build_selected();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        shell.state.poll_workbench();
        shell.draw_sized(size, Vec::new());
        match shell.state.workbench.builds().last(OUTPUT) {
            Some(LastBuild::Built(done)) if after.is_none_or(|at| done.at > at) => {
                let at = done.at;
                shell.settled_sized(size);
                return at;
            }
            Some(LastBuild::Failed(why)) if after.is_none() => panic!("the build stopped: {why:?}"),
            _ => {}
        }
        assert!(Instant::now() < deadline, "the build never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The kinds of file the last build wrote, as the worker read them from its folder.
fn kinds_built(shell: &Shell) -> BTreeSet<String> {
    match shell.state.workbench.builds().last(OUTPUT) {
        Some(LastBuild::Built(done)) => done.parts[0].kinds.clone(),
        _ => panic!("no build went through"),
    }
}

/// The test's own copy of the sheet, with each line about a map or a type file seen to load for `edition`, a day apart; and the last of those days.
fn a_sheet_seen_for(edition: Target) -> (Sheet, String) {
    let mut own: serde_json::Value = serde_json::from_str(SHEET).unwrap();
    let mut days = Vec::new();
    for (at, line) in own["lines"].as_array_mut().unwrap().iter_mut().enumerate() {
        if line["kind"] == "ymap" || line["kind"] == "ytyp" {
            days.push(format!("2026-12-{:02}", at + 1));
            line["results"] = serde_json::json!([{
                "edition": edition.word(), "build": "3258", "where": "fivem-server",
                "date": days.last(), "verdict": "loaded",
            }]);
        }
    }
    let newest = days
        .into_iter()
        .max()
        .expect("the sheet has a line about a map");
    let sheet = Sheet::parse(&own.to_string()).expect("the copy is a sheet");
    (sheet, newest)
}

fn unseen(kinds: &[&str]) -> Seen {
    Seen::Not(kinds.iter().map(ToString::to_string).collect())
}

/// How many widgets the screen reader calls exactly `name`.
fn named(shell: &Shell, name: &str) -> usize {
    (shell.described.iter())
        .filter(|w| w.name.as_deref() == Some(name))
        .count()
}

/// Turn the wheel over the Build tab until the widget called `name` is inside the window, as a person does when the tab is longer than its panel; where it is then.
fn scrolled_to(shell: &mut Shell, name: &str) -> egui::Rect {
    // The status bar is under the tab, so the tab ends a row above the window's edge.
    let (in_the_tab, in_sight) = (EDITOR.1 as f32 - 80.0, EDITOR.1 as f32 - 40.0);
    for _ in 0..40 {
        let bounds = (shell.described.iter())
            .find(|w| w.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("nothing is named `{name}`: {:?}", names(shell)))
            .bounds;
        if bounds.max.y < in_sight {
            return bounds;
        }
        shell.draw(vec![
            egui::Event::PointerMoved(egui::pos2(bounds.center().x, in_the_tab)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0.0, -3.0),
                modifiers: egui::Modifiers::NONE,
                phase: egui::TouchPhase::Move,
            },
        ]);
        shell.settled();
    }
    panic!("`{name}` never came into the window");
}

/// Holds that the line of the open guide that begins `line` stands at the top of its page, under the row of buttons that name the sections.
fn assert_stands_at_the_top(shell: &Shell, line: &str) {
    let heading = (shell.described.iter())
        .filter(|w| w.role == egui::accesskit::Role::Label)
        .find(|w| w.name.as_deref().is_some_and(|name| name.starts_with(line)))
        .unwrap_or_else(|| panic!("no line begins `{line}`: {:?}", names(shell)))
        .bounds;
    let ways = (shell.described.iter())
        .filter(|w| {
            w.name
                .as_deref()
                .is_some_and(|n| n.starts_with("Guide section: "))
        })
        .map(|w| w.bounds.max.y)
        .fold(0.0, f32::max);
    assert!(
        heading.min.y >= ways && heading.min.y - ways < 40.0,
        "`{line}` is at {heading:?}, and the buttons over the page end at {ways}"
    );
}

/// The marks of Markdown a line of the guide's window still holds, which a person would read as noise.
fn markup(shell: &Shell) -> Vec<String> {
    names(shell)
        .into_iter()
        .filter(|name| {
            // ⚠ A glob of the recipe's commands holds two stars before a slash, which is the command and no mark.
            let stars = name.replace("**/", "");
            name.contains("](")
                || stars.contains("**")
                || name.contains("```")
                || name.starts_with('|')
                || (!name.is_empty() && name.chars().all(|c| c == '-' || c == ':'))
        })
        .collect()
}

#[test]
fn a_build_that_wrote_a_map_and_a_type_file_names_both_kinds_nobody_saw_loading() {
    let _serial = crate::ui::theme::serial();
    let _english = crate::i18n::pin("en");
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_map_project(dir.path(), None);
    assert!(
        !shows(&shell, "seen loading") && named(&shell, LINK) == 0,
        "nothing is built yet, so nothing is said of the sheet: {:?}",
        names(&shell)
    );

    build(&mut shell, EDITOR, None);
    let kinds = kinds_built(&shell);
    assert!(
        kinds.contains("ymap") && kinds.contains("ytyp"),
        "the build wrote a map and a type file: {kinds:?}"
    );
    // The build states no edition, so the sheet is read for each of the two, and it ships with no result.
    let read = shell.state.workbench.builds().sheet().seen(&kinds, None);
    assert_eq!(
        read,
        [
            (Target::Legacy, unseen(&["ymap", "ytyp"])),
            (Target::Enhanced, unseen(&["ymap", "ytyp"])),
        ]
    );
    for said in [
        "2 kinds of file this build wrote were not seen loading in GTA V Legacy: maps, type files",
        "2 kinds of file this build wrote were not seen loading in GTA V Enhanced: maps, type files",
    ] {
        assert_eq!(named(&shell, said), 1, "`{said}`: {:?}", names(&shell));
    }
    assert_eq!(
        named(&shell, LINK),
        1,
        "one link to the steps for the resource: {:?}",
        names(&shell)
    );
    let link = (shell.described.iter())
        .find(|w| w.name.as_deref() == Some(LINK))
        .unwrap();
    assert_eq!(link.role, egui::accesskit::Role::Link);
    // ⚠ The walk of the control inventory counts buttons and fields and no link, so the link's row is held here.
    let design = include_str!("../../docs/direction/Workbench_UX_Design.md");
    let row = format!("| `{LINK}` | Build tab, Verification");
    assert!(
        design.lines().any(|line| line.starts_with(&row)),
        "the control inventory has no row that begins `{row}`"
    );
    assert!(
        shows(&shell, "Loads in game: not tested"),
        "the line that is never ticked stays: {:?}",
        names(&shell)
    );
}

#[test]
fn a_result_of_loaded_for_legacy_is_said_with_its_day_and_an_enhanced_build_stays_unchecked() {
    let _serial = crate::ui::theme::serial();
    let _english = crate::i18n::pin("en");
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_map_project(dir.path(), Some(Target::Legacy));
    let (sheet, newest) = a_sheet_seen_for(Target::Legacy);
    shell.state.workbench.read_builds_against(sheet);

    let first = build(&mut shell, EDITOR, None);
    let checked = format!(
        "Every kind of file this build wrote that the acceptance sheet has a line about was seen loading in GTA V Legacy; the newest result is dated {newest}"
    );
    assert_eq!(
        named(&shell, &checked),
        1,
        "`{checked}`: {:?}",
        names(&shell)
    );
    assert!(
        !shows(&shell, "not seen loading") && !shows(&shell, "GTA V Enhanced"),
        "a Legacy build is read for Legacy alone: {:?}",
        names(&shell)
    );
    assert_eq!(
        named(&shell, LINK),
        0,
        "a build whose kinds were all seen is sent to no steps: {:?}",
        names(&shell)
    );

    let unchecked =
        "2 kinds of file this build wrote were not seen loading in GTA V Enhanced: maps, type files";
    // ⚠ The build on the disk was written for Legacy, and stating another edition for the next one changes nothing of what is said of it.
    written_for(&mut shell, Some(Target::Enhanced));
    shell.settled();
    assert_eq!(
        (named(&shell, &checked), named(&shell, unchecked)),
        (1, 0),
        "the build that is there was written for Legacy: {:?}",
        names(&shell)
    );
    build(&mut shell, EDITOR, Some(first));
    assert_eq!(
        named(&shell, unchecked),
        1,
        "`{unchecked}`: {:?}",
        names(&shell)
    );
    assert!(
        !shows(&shell, "was seen loading") && !shows(&shell, "GTA V Legacy"),
        "a result for Legacy says nothing of an Enhanced build: {:?}",
        names(&shell)
    );
    assert_eq!(named(&shell, LINK), 1, "{:?}", names(&shell));
}

#[test]
fn a_build_of_files_no_line_is_about_says_nothing_of_the_sheet() {
    let _serial = crate::ui::theme::serial();
    let _english = crate::i18n::pin("en");
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_files_project(dir.path());
    build(&mut shell, EDITOR, None);

    let kinds = kinds_built(&shell);
    assert!(
        kinds.contains("lua"),
        "the worker read the kinds the build wrote: {kinds:?}"
    );
    assert!(
        !shows(&shell, "seen loading") && named(&shell, LINK) == 0,
        "the sheet has no line about a script: {:?}",
        names(&shell)
    );
    assert!(shows(&shell, "Loads in game: not tested"));
}

#[test]
fn the_link_opens_the_acceptance_guide_at_the_steps_that_load_the_build() {
    let _serial = crate::ui::theme::serial();
    let _english = crate::i18n::pin("en");
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_map_project(dir.path(), Some(Target::Legacy));
    build(&mut shell, EDITOR, None);
    assert!(shell.state.front_door.guide.is_none());

    let link = scrolled_to(&mut shell, LINK).center();
    shell.click(link);
    // The window scrolls to the section over a few frames, as it does for a person.
    for _ in 0..30 {
        shell.draw(Vec::new());
    }

    let open = shell
        .state
        .front_door
        .guide
        .clone()
        .expect("the guide is open");
    assert_eq!(
        (open.document, open.section.as_str()),
        (GuideDocument::Acceptance, ON_A_SERVER),
        "a FiveM resource is sent to the steps for a server"
    );
    assert!(
        guide::headings_of(GuideDocument::Acceptance).contains(&ON_A_SERVER),
        "`{ON_A_SERVER}` is a heading of the acceptance guide"
    );
    assert!(open.scrolled, "the window scrolled to the section");
    assert_stands_at_the_top(&shell, ON_A_SERVER);
    for drawn in [
        "In-game acceptance guide",
        &format!("Guide section: {ON_A_SERVER}"),
        "Copy the folder ",
    ] {
        assert!(shows(&shell, drawn), "`{drawn}`: {:?}", names(&shell));
    }
    assert_eq!(markup(&shell), [] as [&str; 0]);

    shell.draw(vec![key(egui::Key::Escape)]);
    shell.settled();
    assert!(shell.state.front_door.guide.is_none(), "Escape closes it");

    // Help then opens the same document from its top, and not where the link left it.
    commands::run_action(
        &mut shell.state,
        &shell.ctx.clone(),
        Action::OpenAcceptanceGuide,
    );
    for _ in 0..30 {
        shell.draw(Vec::new());
    }
    assert_stands_at_the_top(&shell, "What a person looks for in a running game");
}

#[test]
fn help_opens_the_acceptance_guide_from_the_top_with_its_table_and_its_commands_as_text() {
    let _serial = crate::ui::theme::serial();
    let _english = crate::i18n::pin("en");
    let mut shell = a_first_launch();
    commands::run_action(
        &mut shell.state,
        &shell.ctx.clone(),
        Action::OpenAcceptanceGuide,
    );
    shell.settled();

    let open = shell
        .state
        .front_door
        .guide
        .clone()
        .expect("the guide is open");
    assert_eq!(
        (open.document, open.section.as_str()),
        (GuideDocument::Acceptance, "")
    );
    for heading in guide::headings_of(GuideDocument::Acceptance) {
        let way = format!("Guide section: {heading}");
        assert!(
            shows(&shell, &way),
            "no way to `{heading}`: {:?}",
            names(&shell)
        );
    }
    // A cell of the table of lines, a word of a sentence with a link in it, and a command between fences.
    let sheet = Sheet::shipped().unwrap();
    for drawn in [
        sheet.lines[0].id.as_str(),
        sheet.lines[0].look_for.as_str(),
        "who runs the game",
    ] {
        assert!(shows(&shell, drawn), "`{drawn}`: {:?}", names(&shell));
    }
    assert_eq!(markup(&shell), [] as [&str; 0]);
    assert!(
        !shows(&shell, "Drop the files or the folder on the Wardrobe"),
        "the window shows one document"
    );

    click_on(&mut shell, "Guide section: Make the sample");
    // ⚠ The command stands under a numbered step, indented in the document: the line is the command alone, as a person copies it.
    assert_eq!(
        named(
            &shell,
            "openrage-cli project acceptance <scratch>/acceptance/build/or_acceptance"
        ),
        1,
        "a command of the recipe is a line of its own: {:?}",
        names(&shell)
    );
    assert_eq!(markup(&shell), [] as [&str; 0]);

    // The other guide is still the one Help ▸ Guide opens.
    commands::run_action(&mut shell.state, &shell.ctx.clone(), Action::OpenGuide);
    shell.settled();
    assert!(shows(
        &shell,
        "Drop the files or the folder on the Wardrobe"
    ));
    assert!(!shows(&shell, sheet.lines[0].look_for.as_str()));
}

#[test]
fn the_link_under_a_story_pack_opens_the_steps_for_story_mode() {
    let _serial = crate::ui::theme::serial();
    let _english = crate::i18n::pin("en");
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_files_project(dir.path());
    let pack = (shell.state)
        .add_build_taking(
            "shopcase_dlc",
            OutputKind::SingleplayerDlc,
            Some(Target::Legacy),
            &["files".into()],
        )
        .expect("a build");
    shell.state.workbench.selected_output = Some(pack.clone());
    // What the worker keeps of a pack it wrote: the archive is a kind of its own, beside the kinds of the files in it.
    let part = BuiltPart {
        kinds: ["rpf", "ytd"].map(str::to_string).into(),
        target: Some(Target::Legacy),
        ..a_part(0)
    };
    shell.state.workbench.keep_a_build(
        &pack,
        LastBuild::Built(BuiltSummary {
            at: std::time::SystemTime::now(),
            parts: vec![part],
            frozen: Vec::new(),
        }),
    );
    shell.settled();

    let said =
        "1 kind of file this build wrote was not seen loading in GTA V Legacy: singleplayer DLC packs";
    assert_eq!(named(&shell, said), 1, "`{said}`: {:?}", names(&shell));
    let link = scrolled_to(&mut shell, LINK).center();
    shell.click(link);
    for _ in 0..30 {
        shell.draw(Vec::new());
    }
    let open = (shell.state.front_door.guide.clone()).expect("the guide is open");
    assert_eq!(
        (open.document, open.section.as_str()),
        (GuideDocument::Acceptance, IN_STORY_MODE),
        "a pack is loaded in story mode, and on no server"
    );
    assert_stands_at_the_top(&shell, IN_STORY_MODE);
}

/// A text or a filled rectangle of a frame, in the order they were painted.
enum Painted {
    Text {
        said: String,
        at: egui::Rect,
        /// The part of the window the text may be painted in.
        clip: egui::Rect,
    },
    Fill(egui::Rect),
}

/// Every text and every filled rectangle of `shapes`, in the order they were painted.
fn painted(shapes: &[ClippedShape]) -> Vec<Painted> {
    fn walk(shape: &egui::Shape, clip: egui::Rect, out: &mut Vec<Painted>) {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, clip, out)),
            egui::Shape::Rect(rect) if rect.fill.a() > 0 => {
                out.push(Painted::Fill(rect.rect.intersect(clip)));
            }
            egui::Shape::Text(text) => out.push(Painted::Text {
                said: text.galley.text().to_string(),
                at: text.visual_bounding_rect(),
                clip,
            }),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(&clipped.shape, clipped.clip_rect, &mut out);
    }
    out
}

#[test]
fn the_table_of_lines_is_painted_whole_with_no_row_over_another_and_none_past_the_page() {
    let _serial = crate::ui::theme::serial();
    let _english = crate::i18n::pin("en");
    let mut shell = a_first_launch();
    let the_lines = guide::headings_of(GuideDocument::Acceptance)[1];
    (shell.state.front_door).show_document(GuideDocument::Acceptance, the_lines);
    let mut shapes = Vec::new();
    for _ in 0..30 {
        shapes = shell.draw(Vec::new());
    }
    assert_stands_at_the_top(&shell, the_lines);

    let sheet = Sheet::shipped().unwrap();
    let cells: Vec<&str> = (sheet.lines.iter())
        .flat_map(|line| [&line.id, &line.action, &line.look_for])
        .map(String::as_str)
        .collect();
    let painted = painted(&shapes);
    let mut found = Vec::new();
    for (place, shape) in painted.iter().enumerate() {
        let Painted::Text { said, at, clip } = shape else {
            continue;
        };
        if !cells.contains(&said.as_str()) {
            continue;
        }
        found.push(said.as_str());
        assert!(
            at.min.x >= clip.min.x - 0.5 && at.max.x <= clip.max.x + 0.5,
            "`{said}` runs past the page, which shows {clip:?} of it: {at:?}"
        );
        assert!(
            at.min.y >= clip.min.y && at.max.y <= clip.max.y,
            "`{said}` is not whole in the window, so this checks nothing of it: {at:?} in {clip:?}"
        );
        // ⚠ A band painted after a cell's text is painted on it: the band of a row belongs under the row.
        for later in &painted[place + 1..] {
            let Painted::Fill(fill) = later else {
                continue;
            };
            let over = fill.intersect(*at);
            assert!(
                !(over.width() > 1.0 && over.height() > 1.0),
                "`{said}` at {at:?} is painted over by {fill:?}"
            );
        }
    }
    found.sort_unstable();
    let mut cells = cells;
    cells.sort_unstable();
    assert_eq!(found, cells, "every cell of every line is painted once");
}

/// How wide `said` is on one line of the tab's text, and how tall that line is.
fn on_one_line(shell: &Shell, said: &str) -> egui::Vec2 {
    let font = egui::TextStyle::Body.resolve(&shell.ctx.global_style());
    shell.ctx.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(said.to_string(), font, egui::Color32::WHITE)
            .size()
    })
}

/// A build that states no edition, read against a sheet with Legacy seen and Enhanced not, so that it says both sentences; and the two readings.
fn a_build_that_says_both_sentences(dir: &Path) -> (Shell, Vec<(Target, Seen)>) {
    let mut shell = a_map_project(dir, None);
    let (sheet, _) = a_sheet_seen_for(Target::Legacy);
    shell.state.workbench.read_builds_against(sheet);
    build(&mut shell, EDITOR, None);
    let kinds = kinds_built(&shell);
    let readings = shell.state.workbench.builds().sheet().seen(&kinds, None);
    assert!(
        matches!(readings.as_slice(), [(_, Seen::All(_)), (_, Seen::Not(_))]),
        "{readings:?}"
    );
    (shell, readings)
}

/// The code of the language whose catalog says `reading` widest on one line, and every language by that width.
fn widest_language(shell: &Shell, reading: &Seen, edition: Target) -> (String, Vec<(f32, String)>) {
    let mut widths: Vec<(f32, String)> = (crate::i18n::languages().iter())
        .map(|language| {
            let _pin = crate::i18n::pin(&language.code);
            let said = sentence(reading, edition);
            (on_one_line(shell, &said).x, language.code.clone())
        })
        .collect();
    widths.sort_by(|a, b| a.0.total_cmp(&b.0));
    let widest = widths.last().expect("there are languages").1.clone();
    (widest, widths)
}

#[test]
fn each_sentence_wraps_inside_a_narrow_column_in_every_language() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let (mut shell, readings) = a_build_that_says_both_sentences(dir.path());
    let sheet = shell.state.workbench.builds().sheet().clone();
    let output = (shell.state.project_carried.container.output(OUTPUT))
        .cloned()
        .expect("the build");
    let part = match shell.state.workbench.builds().last(OUTPUT) {
        Some(LastBuild::Built(done)) => done.parts[0].clone(),
        _ => panic!("no build went through"),
    };

    // The tab's own lines, drawn alone in a column narrower than any of them is on one line.
    const COLUMN: f32 = 320.0;
    for language in crate::i18n::languages() {
        let code = language.code.as_str();
        let _pin = crate::i18n::pin(code);
        shell.scratch(|ui| {
            ui.allocate_ui(egui::vec2(COLUMN, 0.0), |ui| {
                ui.set_max_width(COLUMN);
                crate::ui::panels::project::in_game::seen_in_game(ui, &sheet, &output, &part);
            });
        });
        let mut drawn: Vec<String> = (readings.iter())
            .map(|(edition, reading)| sentence(reading, *edition))
            .collect();
        drawn.push(crate::i18n::tr(LINK).to_string());
        for said in &drawn {
            let bounds = (shell.described.iter())
                .find(|w| w.name.as_deref() == Some(said.as_str()))
                .unwrap_or_else(|| panic!("{code}: `{said}` is not drawn: {:?}", names(&shell)))
                .bounds;
            assert!(
                bounds.width() <= COLUMN + 0.5,
                "{code}: `{said}` is wider than its column: {bounds:?}"
            );
            let line = on_one_line(&shell, said);
            assert_eq!(
                bounds.height() > line.y * 1.5,
                line.x > COLUMN,
                "{code}: `{said}` is {} wide on one line and drawn as {bounds:?}",
                line.x
            );
        }
        let sentences = &drawn[..readings.len()];
        assert!(
            sentences
                .iter()
                .all(|said| on_one_line(&shell, said).x > COLUMN),
            "{code}: a sentence fits the column on one line, so nothing here wraps: {sentences:?}"
        );
    }
}

#[test]
fn each_sentence_wraps_as_the_tabs_other_lines_do_in_a_narrow_window_in_its_widest_language() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let (mut shell, readings) = a_build_that_says_both_sentences(dir.path());

    for (edition, reading) in &readings {
        let (widest, widths) = widest_language(&shell, reading, *edition);
        let _pin = crate::i18n::pin(&widest);
        let said = sentence(reading, *edition);
        shell.settled_sized(NARROW);
        let mine: Vec<String> = (readings.iter())
            .map(|(edition, reading)| sentence(reading, *edition))
            .collect();
        let label = (shell.described.iter())
            .find(|w| w.name.as_deref() == Some(said.as_str()))
            .unwrap_or_else(|| panic!("{widest}: `{said}` is not drawn: {:?}", names(&shell)))
            .bounds;
        // The lines the Verification part wrapped before this one was added start where it starts.
        let beside = (shell.described.iter())
            .filter(|w| w.role == egui::accesskit::Role::Label && w.bounds.min.x == label.min.x)
            .filter(|w| !w.name.as_ref().is_some_and(|name| mine.contains(name)))
            .map(|w| w.bounds.width())
            .fold(0.0, f32::max);
        let line = on_one_line(&shell, &said);
        assert!(
            line.x > beside * 1.25,
            "{widest}: `{said}` is not wider than the lines beside it, so nothing here wraps: {widths:?}"
        );
        assert!(
            label.height() > line.y * 1.5 && label.width() <= beside * 1.25,
            "{widest}: `{said}` is {} wide on one line, drawn as {label:?} beside lines {beside} wide",
            line.x
        );
    }
}
