//! The Build tab, the Build button and the job a build is, as they are drawn: the header line, the three sections in their
//! order, the five steps with the one running marked, plain sentences with no internal name, and the list of every file reloaded.

use std::path::{Path, PathBuf};

use super::frames_of_the_shell::Shell;
use super::the_project_screens_on_screen::{a_project_shell, names, shows, until, write};
use crate::state::front_door::{NewProject, Template};
use crate::state::workbench::{BuiltPart, BuiltSummary, LastBuild};
use crate::state::ThemeChoice;
use crate::ui::dock::Workspace;
use crate::ui::panels::registry::ids;

/// The id of the build every project of these tests declares, whatever its resource is called.
const OUTPUT: &str = "fivem";

/// A project of loose files with one build, `shopcase`, read and with the Build tab on top of the bottom panels.
pub(super) fn a_files_project(dir: &Path) -> Shell {
    let mut draft = NewProject::new(Template::Files, dir.to_path_buf());
    draft.set_name("shopcase".into());
    let file = draft.create().expect("a files project").file;
    let folder = file.parent().unwrap().to_path_buf();
    let mut shell = Shell::new(ThemeChoice::Dark);
    shell.state.load_project_from(file);
    shell
        .state
        .add_files_to_project(vec![write(&folder.join("client.lua"), b"print('hello')")]);
    shell.state.active_workspace = Workspace::Project;
    shell
        .state
        .workspace_docks
        .reveal(Workspace::Project, ids::BUILD);
    until(&mut shell, "the plan", |s| {
        !s.state.workbench.resources_of(OUTPUT).is_empty()
    });
    shell.settled();
    shell
}

fn the_build_folder(shell: &Shell) -> PathBuf {
    let file = shell.state.project_path.clone().expect("a project");
    or_project::project_folder(&file).join("build/shopcase")
}

/// The y of the widget named `name` that is nearest the top of the screen.
fn y_of(shell: &Shell, name: &str) -> f32 {
    (shell.described.iter())
        .filter(|w| w.name.as_deref() == Some(name))
        .map(|w| w.bounds.center().y)
        .min_by(f32::total_cmp)
        .unwrap_or_else(|| panic!("nothing is named `{name}`: {:?}", names(shell)))
}

#[test]
fn the_tab_opens_with_one_line_of_figures_and_three_sections_in_the_order_of_a_build() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let shell = a_files_project(dir.path());

    let plan = &shell.state.workbench.resources_of(OUTPUT)[0].plan;
    let line = format!(
        "{}, {}, nothing stops this build",
        crate::state::front_door::counted(plan.totals.files as usize, "file"),
        or_job::human_bytes(plan.totals.bytes)
    );
    assert!(
        shows(&shell, &line),
        "the header line `{line}` is not drawn: {:?}",
        names(&shell)
    );
    let (plan_y, changed_y, verified_y) = (
        y_of(&shell, "Plan"),
        y_of(&shell, "What changed"),
        y_of(&shell, "Verification"),
    );
    assert!(
        plan_y < changed_y && changed_y < verified_y,
        "the sections are not in the order plan, what changed, verification: {plan_y} {changed_y} {verified_y}"
    );
}

#[test]
fn the_steps_are_listed_and_the_one_running_is_marked_in_words_and_not_in_colour_alone() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_files_project(dir.path());
    let marked = |shell: &Shell| -> Vec<String> {
        names(shell)
            .into_iter()
            .filter(|name| name.starts_with("▶"))
            .collect()
    };
    assert!(
        marked(&shell).is_empty(),
        "no build runs, so no step is: {:?}",
        marked(&shell)
    );
    for step in ["Plan", "Cook", "Write", "Read", "Swap"] {
        assert!(
            shows(&shell, &format!("{step} ")) || shows(&shell, step),
            "the list of steps has none for `{step}`"
        );
    }

    // ⚠ Not polled: what the worker said stays on screen, as a build that is still running reads.
    shell.state.build_selected();
    let live = the_build_folder(&shell);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !live.join("fxmanifest.lua").is_file() {
        assert!(
            std::time::Instant::now() < deadline,
            "the build never swapped in"
        );
        shell.draw(Vec::new());
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    shell.settled();
    let running = marked(&shell);
    assert_eq!(running.len(), 1, "one step is running: {:?}", names(&shell));
    assert!(
        running[0].starts_with("▶ 5") && running[0].contains("Swap"),
        "the last step the build reported is the one marked: {running:?}"
    );

    let job = (shell.state.running_jobs().into_iter())
        .find(|job| job.kind == crate::state::jobs::JobKind::ProjectBuild)
        .expect("the build is a job");
    assert!(
        job.detail.starts_with("Step 5 of 5") && job.fraction.is_some_and(|f| f > 0.0 && f < 1.0),
        "the job names its step and how far it is: {job:?}"
    );

    while shell.state.workbench.builds().last(OUTPUT).is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "the build never ended"
        );
        shell.state.poll_workbench();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    shell.settled();
    assert!(marked(&shell).is_empty(), "{:?}", names(&shell));
}

#[test]
fn a_finished_build_says_what_it_did_in_plain_sentences_with_no_internal_name() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_project_shell(dir.path(), ThemeChoice::Dark);
    shell
        .state
        .workspace_docks
        .reveal(Workspace::Project, ids::BUILD);
    shell.settled();

    // The plan of a clothing pack carries figures named for the engine, and the tab says them as a person does.
    let plan = &shell.state.workbench.resources_of(OUTPUT)[0].plan;
    assert!(
        plan.budgets.iter().any(|b| b.name == "clothing_drawables"),
        "{:?}",
        plan.budgets
    );
    assert!(
        shows(&shell, "Clothing drawables"),
        "the figure has no words of its own: {:?}",
        names(&shell)
    );
    for internal in [
        "clothing_drawables",
        "clothing_textures",
        "module",
        "output",
    ] {
        assert!(
            !names(&shell).iter().any(|name| name.contains(internal)),
            "the tab says `{internal}`: {:?}",
            names(&shell)
        );
    }
    // Per slot, in the words of the Wardrobe.
    assert!(
        shows(&shell, "Tops: 3"),
        "the garments are not counted by slot: {:?}",
        names(&shell)
    );
}

pub(super) fn a_part(reloaded: usize) -> BuiltPart {
    BuiltPart {
        resource: "shopcase".into(),
        folder: PathBuf::from("build/shopcase"),
        files: 40,
        bytes: 1 << 20,
        checked: 40,
        reloaded: (0..reloaded)
            .map(|n| {
                (
                    format!("stream/part_{n:02}.ydd"),
                    "a drawable reader".into(),
                )
            })
            .collect(),
        metadata: vec![or_project::MetadataCount {
            reader: "CPedVariationInfo (RBF)".into(),
            documents: 2,
            entries: 31,
        }],
        names: Default::default(),
        memory: or_project::Memory {
            files: 3,
            bytes: 14 << 20,
            ceiling: Some(64 << 20),
        },
        reproducible: or_project::Reproducible::No {
            differences: vec!["stream/a.ytd differs".into()],
        },
        kept_previous: true,
        since: None,
        losses: Vec::new(),
        cooked: Vec::new(),
        unchanged: Vec::new(),
        dlclist_entry: None,
        pack_checks: Vec::new(),
        sizes: Vec::new(),
        kinds: Default::default(),
        target: None,
    }
}

#[test]
fn the_verification_says_every_figure_it_has_and_lists_every_file_it_reloaded() {
    let _serial = crate::ui::theme::serial();
    let dir = tempfile::tempdir().unwrap();
    let mut shell = a_files_project(dir.path());
    shell.state.workbench.keep_a_build(
        OUTPUT,
        LastBuild::Built(BuiltSummary {
            at: std::time::SystemTime::now(),
            parts: vec![a_part(30)],
            frozen: Vec::new(),
        }),
    );
    shell.settled();

    let size = or_job::human_bytes;
    for wanted in [
        "40 files were read back and equal the plan",
        "30 files were opened again to check them",
        "2 documents of clothing variation metadata with 31 entries",
        &format!(
            "Streaming memory: {} of {} allowed",
            size(14 << 20),
            size(64 << 20)
        ),
        "A second build wrote different bytes in 1 file",
        "not tested",
    ] {
        assert!(
            shows(&shell, wanted),
            "the verification does not say `{wanted}`: {:?}",
            names(&shell)
        );
    }
    assert!(
        !names(&shell).iter().any(|name| name.contains("more")),
        "a list that stops and counts the rest is not in full: {:?}",
        names(&shell)
    );
}
