//! The Build tab: the plan of the chosen build, what changed since the last one, what the last build wrote and read back, and what is never claimed.
//!
//! ⚠ **Sentences a person reads.** The engine's names for a figure (`clothing_drawables`) and for a step
//! never reach the screen; `figure_words` and `STEPS` say them.

use crate::state::front_door::KindWords as _;
use std::str::FromStr;

use or_project::acceptance::Sheet;
use or_project::{Container, Diff, Item, Origin, Output};
use or_wardrobe::Slot;

use super::build_part::built_part;
use super::{command_button, severity_colour, size, REFRESH};
use crate::state::front_door::{counted, spoken, GuideDocument};
use crate::state::workbench::{
    verifies, BuildDot, BuiltPart, Counts, LastBuild, Resource, Scope, Severity, STEPS,
};
use crate::state::AppState;
use crate::ui::commands::Action;
use crate::ui::theme;

/// What each step of a build does, beside its name in `STEPS`, as a hover says it.
const STEP_SENTENCES: [&str; 5] = [
    tr_key!("List every file the build copies or cooks, and the document it writes"),
    tr_key!("Each source folder becomes the game file it stands for, before anything is written"),
    tr_key!("Write the plan to a fresh folder beside the last build"),
    tr_key!("Read every file back, byte for byte and through the reader that opens its kind"),
    tr_key!("Swap it in, keeping the last build as the previous one"),
];

/// How many reasons a build that stopped lists under its buttons: Problems has the rest.
const STOPPED_LISTED: usize = 3;

/// How many of a long list the tab names before it counts the rest.
pub(super) const LISTED: usize = 20;

/// How tall a row of a list that scrolls is.
pub(super) const ROW: f32 = 18.0;

/// What stands before each item of a list of reasons: not the dot of a state, which a status is drawn with.
pub(super) const LIST_MARK: &str = "•";

pub fn draw(ui: &mut egui::Ui, state: &mut AppState) {
    if state.project_path.is_none() {
        centred(ui, tr!("No project is open, so there is nothing to build"));
        return;
    }
    ui.ctx().request_repaint_after(REFRESH);
    let Some(output) = state
        .active_output_id()
        .and_then(|id| state.project_carried.container.output(&id).cloned())
    else {
        ui.vertical_centered(|ui| {
            ui.add_space(theme::SPACING_SECTION);
            ui.label(
                egui::RichText::new(tr!("The project declares no build"))
                    .size(16.0)
                    .color(theme::TEXT_SECONDARY),
            );
            ui.add_space(theme::SPACING_DEFAULT);
            command_button(ui, state, Action::AddBuild);
        });
        return;
    };

    let mut go_to_problems = false;
    let mut how_to_check = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let dot = state.workbench.dot(&output.id);
            let last = state.workbench.builds().last(&output.id).cloned();
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&output.name).strong().size(16.0));
                let edition = output
                    .target
                    .map(|t| tr!(", for {edition_label}", edition_label = t.generation().edition_label()))
                    .unwrap_or_default();
                ui.label(
                    egui::RichText::new(tr!("{kind}{edition}, written to build/{name}", edition = edition, kind = output.kind.words(), name = output.name))
                    .color(theme::TEXT_SECONDARY),
                );
            });
            let building = state.workbench.builds().is_running();
            ui.horizontal(|ui| {
                // ⚠ A build is cancelled where it was started: while it runs the buttons that start one are replaced by the one that stops it.
                if building {
                    if ui
                        .button(tr!("Cancel build"))
                        .on_hover_text(tr!("Stop the build being written: the folder it was writing is removed and the last good build is as it was"))
                        .clicked()
                    {
                        state.cancel_job(crate::state::jobs::JobKind::ProjectBuild);
                    }
                } else {
                    command_button(ui, state, Action::BuildProject);
                    command_button(ui, state, Action::BuildAll);
                    command_button(ui, state, Action::RevealBuild);
                }
                command_button(ui, state, Action::EditBuild);
                ui.label(egui::RichText::new(format!("● {}", dot.label())).color(dot_colour(dot)));
            });
            if let Some(job) = state.project_build_job().filter(|_| building) {
                ui.add(
                    egui::ProgressBar::new(job.fraction.unwrap_or(0.0))
                        .text(job.detail.clone())
                        .desired_width(ui.available_width().min(480.0)),
                );
            }
            ui.horizontal(|ui| {
                verify_option(ui, state, &output);
                super::help_link(ui, state, super::GUIDE_BUILD);
            });
            let resources = state.workbench.resources_of(&output.id);
            let stopped_at = state.workbench.builds().stopped_at(&output.id);
            match &last {
                Some(LastBuild::Failed(reasons)) if !building => {
                    go_to_problems = stopped(ui, reasons, stopped_at);
                }
                Some(LastBuild::Built(done)) if !building => verdict(ui, done.files(), &done.parts),
                _ => {}
            }
            let counts = state.workbench.counts(Scope::Build(&output.id));
            let line = header_line(resources, counts);
            let colour = match counts.errors {
                0 => theme::TEXT_PRIMARY,
                _ => severity_colour(Severity::Error),
            };
            ui.add(egui::Label::new(egui::RichText::new(line).color(colour)).wrap());
            let changes = changes_of(resources, last.as_ref());
            if let Some(since) = since_line(&changes) {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(since)
                            .small()
                            .color(theme::TEXT_SECONDARY),
                    )
                    .wrap(),
                );
            }
            let running = (state.workbench.builds().running())
                .is_some_and(|(current, _)| current == output.id)
                .then(|| state.workbench.builds().step())
                .flatten();
            steps(ui, running, dot, stopped_at);
            ui.add_space(theme::SPACING_DEFAULT);

            // ⚠ The answer to "did it work" is open after a build and the plan, which is reference, is folded: an open plan shows before one.
            let built = matches!(dot, BuildDot::Built | BuildDot::Unchecked);
            section(ui, tr!("Plan"), !built, |ui| {
                if resources.is_empty() {
                    ui.label(
                        egui::RichText::new(tr!("Reading the project…")).color(theme::TEXT_SECONDARY),
                    );
                }
                for resource in resources {
                    plan_table(ui, resource, resources.len());
                }
            });
            let changed = changes.diffs.iter().flatten().any(|d| !d.is_empty());
            section(ui, tr!("What changed"), !built || changed, |ui| {
                for (resource, diff) in resources.iter().zip(&changes.diffs) {
                    changes_of_one(ui, resource, resources.len(), *diff, changes.by_the_build);
                }
            });
            section(ui, tr!("Verification"), true, |ui| {
                let builds = state.workbench.builds();
                how_to_check = verification(
                    ui,
                    (&state.project_carried.container, &output, builds.sheet()),
                    builds.last(&output.id),
                );
            });
            section(ui, tr!("Size map"), !built, |ui| {
                super::size_map::draw(ui, state, &output, resources);
            });
            // ⚠ Never ticked by the tool: nobody here can run GTA V, and a tester records it per kind of build.
            ui.label(
                egui::RichText::new(
                    tr!("☐  Loads in game: not tested. A build says that every file it wrote reads \
                     back, never that the game loads it"),
                )
                .color(theme::TEXT_SECONDARY),
            );
        });
    if let Some(section) = how_to_check {
        state
            .front_door
            .show_document(GuideDocument::Acceptance, section);
    }
    if go_to_problems {
        state.workspace_docks.reveal(
            crate::ui::dock::Workspace::Project,
            crate::ui::panels::registry::ids::PROBLEMS,
        );
    }
}

/// The build's option to be built a second time and compared, which a build that does not ask for it says it did not run.
fn verify_option(ui: &mut egui::Ui, state: &mut AppState, output: &or_project::Output) {
    let said = tr!("Verify with a second build");
    let mut on = verifies(output);
    let response = ui.checkbox(&mut on, said).on_hover_text(
        tr!("After the build, build it a second time into a scratch folder and compare the two byte for byte. It cooks every source again, so it takes about as long again."),
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, on, said));
    if on != verifies(output) {
        let _ = state.verify_build(&output.id, on);
    }
}

/// A heading that folds what is under it away, open or closed to start with as `open` says.
///
/// ⚠ Keyed by `open` too: what a person folded before a build is not what they chose for the sections of a build that went through.
fn section(ui: &mut egui::Ui, title: &str, open: bool, body: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(egui::RichText::new(title).strong())
        .id_salt(("build_tab_section", title, open))
        .default_open(open)
        .show(ui, body);
}

fn centred(ui: &mut egui::Ui, text: &str) {
    theme::empty_message(ui, text);
}

fn dot_colour(dot: BuildDot) -> theme::Themed {
    match dot {
        BuildDot::Never => theme::TEXT_SECONDARY,
        BuildDot::Running => theme::INFO,
        BuildDot::Built => theme::SUCCESS,
        BuildDot::Failed => theme::ERROR,
        BuildDot::Blocked | BuildDot::Unchecked => theme::WARNING,
    }
}

/// The five steps of a build in one row: a tick before the step it is on, an arrow on the one running or a cross on the one it stopped at, as well as a colour.
fn steps(ui: &mut egui::Ui, running: Option<usize>, dot: BuildDot, stopped_at: Option<usize>) {
    let built = matches!(dot, BuildDot::Built | BuildDot::Unchecked);
    let stopped_at = stopped_at.filter(|_| dot == BuildDot::Failed);
    ui.horizontal_wrapped(|ui| {
        for (at, name) in STEPS.iter().map(|name| crate::i18n::tr(name)).enumerate() {
            let on_it = running.or(stopped_at);
            let done = built || on_it.is_some_and(|on| at < on);
            let text = if running == Some(at) {
                egui::RichText::new(format!("▶ {} {name}", at + 1))
                    .strong()
                    .color(theme::INFO)
            } else if stopped_at == Some(at) {
                egui::RichText::new(format!("{} {} {name}", egui_phosphor::regular::X, at + 1))
                    .strong()
                    .color(severity_colour(Severity::Error))
            } else if done {
                egui::RichText::new(format!(
                    "{} {} {name}",
                    egui_phosphor::regular::CHECK,
                    at + 1
                ))
                .color(theme::SUCCESS)
            } else {
                egui::RichText::new(format!("{} {name}", at + 1)).color(theme::TEXT_SECONDARY)
            };
            ui.label(text)
                .on_hover_text(crate::i18n::tr(STEP_SENTENCES[at]));
        }
    });
}

/// One line for the whole of what the build would write: its files, its size and what stops it, as the status bar and the Builds list say it.
fn header_line(resources: &[Resource], counts: Counts) -> String {
    let files: u64 = resources.iter().map(|r| r.plan.totals.files).sum();
    let bytes: u64 = resources.iter().map(|r| r.plan.totals.bytes).sum();
    format!(
        "{}, {}, {}",
        counted(files as usize, "file"),
        size(bytes),
        counts.stopping()
    )
}

/// What differs, resource by resource, and whose difference it is.
struct Changes<'a> {
    diffs: Vec<Option<&'a Diff>>,
    /// What the last build changed from the one it replaced, and not what the next would.
    by_the_build: bool,
}

/// What the next build would change when anything is edited since the last, else what the last build changed.
///
/// ⚠ Not the plan's difference alone: a plan compared with the build just written is empty by design, and saying so beside the removal that build made says the opposite of the verdict.
fn changes_of<'a>(resources: &'a [Resource], last: Option<&'a LastBuild>) -> Changes<'a> {
    let edited = resources
        .iter()
        .any(|r| r.since.as_ref().is_some_and(|d| !d.is_empty()));
    if let (false, Some(LastBuild::Built(done))) = (edited, last) {
        let diffs = (resources.iter())
            .map(|r| {
                let part = done.parts.iter().find(|p| p.resource == r.plan.resource);
                part.and_then(|p| p.since.as_ref())
            })
            .collect();
        return Changes {
            diffs,
            by_the_build: true,
        };
    }
    Changes {
        diffs: resources.iter().map(|r| r.since.as_ref()).collect(),
        by_the_build: false,
    }
}

/// What changed in a line of its own: nothing at all is not a line, since a build with no earlier one and a build that changed nothing say it in the fold.
fn since_line(changes: &Changes) -> Option<String> {
    let diffs: Vec<&Diff> = changes.diffs.iter().flatten().copied().collect();
    let count = |of: fn(&Diff) -> usize| diffs.iter().map(|d| of(d)).sum::<usize>();
    let counts = [
        count(|d| d.added.len()),
        count(|d| d.changed.len()),
        count(|d| d.moved.len()),
        count(|d| d.removed.len()),
    ];
    if counts.iter().sum::<usize>() == 0 {
        return None;
    }
    let heading = if changes.by_the_build {
        tr!("This build changed")
    } else {
        tr!("Since the last build")
    };
    Some(tr!(
        "{heading}: {added} added, {changed} changed, {moved} moved, {removed} removed",
        heading = heading,
        added = counts[0],
        changed = counts[1],
        moved = counts[2],
        removed = counts[3]
    ))
}

/// What a build that stopped says beside its state: where it stopped, why, and that the last good build is as it was; whether the person asked for Problems.
///
/// ⚠ Here, under the buttons, and not three screens down in Verification: the cause of a failure belongs next to the word Failed.
fn stopped(ui: &mut egui::Ui, reasons: &[String], at: Option<usize>) -> bool {
    let error = severity_colour(Severity::Error);
    let mut go = false;
    egui::Frame::group(ui.style())
        .stroke(egui::Stroke::new(1.0_f32, error.get()))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let where_it_stopped =
                match at.and_then(|at| STEPS.get(at).map(|name| (at, crate::i18n::tr(name)))) {
                    Some((at, name)) => tr!(
                        "The build stopped at step {step}, {name}.",
                        step = at + 1,
                        name = name
                    ),
                    None => tr!("The build stopped.").to_string(),
                };
            ui.add(
                egui::Label::new(
                    egui::RichText::new(tr!(
                        "{where_it_stopped} The last good build is as it was.",
                        where_it_stopped = where_it_stopped
                    ))
                    .strong()
                    .color(error),
                )
                .wrap(),
            );
            for reason in reasons.iter().take(STOPPED_LISTED) {
                ui.add(egui::Label::new(format!("{LIST_MARK} {}", spoken(reason))).wrap());
            }
            if reasons.len() > STOPPED_LISTED {
                ui.label(tr!(
                    "… and {count} more",
                    count = reasons.len() - STOPPED_LISTED
                ));
            }
            go = ui
                .button(tr!("Show in Problems"))
                .on_hover_text(tr!(
                    "Problems lists the build that stopped, and what to do about each reason"
                ))
                .clicked();
        });
    go
}

/// The verdict of a build that went through, in one line: what was written and read back, whether a second build agreed, and what is never claimed.
fn verdict(ui: &mut egui::Ui, files: u64, parts: &[BuiltPart]) {
    use or_project::Reproducible;

    let second = if parts
        .iter()
        .any(|p| matches!(p.reproducible, Reproducible::No { .. }))
    {
        (
            tr!(
                "{icon} a second build wrote different bytes",
                icon = egui_phosphor::regular::X
            ),
            true,
        )
    } else if !parts.is_empty()
        && parts
            .iter()
            .all(|p| matches!(p.reproducible, Reproducible::Yes { .. }))
    {
        (
            tr!(
                "{icon} a second build wrote the same bytes",
                icon = egui_phosphor::regular::CHECK
            ),
            false,
        )
    } else {
        (tr!("a second build was not run").to_string(), false)
    };
    let files = counted(files as usize, "file");
    let line = tr!(
        "{icon} {files} written and read back · {second} · ☐ loads in game: not tested",
        icon = egui_phosphor::regular::CHECK,
        files = files,
        second = second.0
    );
    let colour = if second.1 {
        severity_colour(Severity::Error)
    } else {
        theme::TEXT_PRIMARY
    };
    ui.add(egui::Label::new(egui::RichText::new(line).strong().color(colour)).wrap());
}

/// A figure of a plan as a person says it: `clothing_drawables` is "Clothing drawables".
pub(super) fn figure_words(name: &str) -> String {
    match name {
        "clothing_drawables" => tr!("Clothing drawables").to_string(),
        "clothing_textures" => tr!("Clothing textures").to_string(),
        "clothing_slots" => tr!("Component drawables").to_string(),
        "clothing_props" => tr!("Prop drawables").to_string(),
        "clothing_outfit_memory" => tr!("Heaviest outfit memory").to_string(),
        "addon_peds" => tr!("Add-on peds").to_string(),
        "addon_ped_files" => tr!("Add-on ped files").to_string(),
        "archive_entries" => tr!("Entries in the fullest archive").to_string(),
        other => {
            let spaced = other.replace('_', " ");
            let mut chars = spaced.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

/// Whether a figure counts bytes, which a person reads in the units the sizes of the same tab are in.
pub(super) fn figure_is_bytes(name: &str) -> bool {
    name.ends_with("_memory") || name.ends_with("_bytes")
}

/// A figure's number as the tab says it: bytes as a size, a count as a count.
pub(super) fn figure_value(name: &str, n: u64) -> String {
    if figure_is_bytes(name) {
        size(n)
    } else {
        n.to_string()
    }
}

/// `used` against its ceiling, or on its own where none is set (a plan with no limit at all says so once), with what it counts in front.
pub(super) fn against(what: &str, used: String, ceiling: Option<String>) -> String {
    match ceiling {
        Some(max) => tr!(
            "{what}: {used} of {max} allowed",
            what = what,
            used = used,
            max = max
        ),
        None => tr!("{what}: {used}", what = what, used = used),
    }
}

/// How many garments of `items` are in each slot, in the order the slots come; an item whose name holds no slot, such as a ped, is not counted.
///
/// ⚠ An item's name is `<collection>^<slot>_<position>`: the slot is read from what the plan wrote.
fn by_slot(items: &[Item]) -> Vec<(Slot, usize)> {
    let mut counted: std::collections::BTreeMap<Slot, usize> = Default::default();
    for item in items {
        let slot = (item.name.rsplit_once('^'))
            .and_then(|(_, place)| place.rsplit_once('_'))
            .and_then(|(slot, _)| Slot::from_str(slot).ok());
        if let Some(slot) = slot {
            *counted.entry(slot).or_default() += 1;
        }
    }
    counted.into_iter().collect()
}

fn plan_table(ui: &mut egui::Ui, resource: &Resource, resources: usize) {
    let plan = &resource.plan;
    if resources > 1 {
        ui.label(
            egui::RichText::new(tr!("Resource {resource}", resource = plan.resource)).strong(),
        );
    }
    ui.label(against(
        tr!("Files"),
        plan.totals.files.to_string(),
        plan.limits.files.map(|n| n.to_string()),
    ));
    ui.label(against(
        tr!("Size"),
        size(plan.totals.bytes),
        plan.limits.bytes.map(size),
    ));
    for budget in &plan.budgets {
        ui.label(against(
            &figure_words(&budget.name),
            figure_value(&budget.name, budget.used),
            budget.ceiling.map(|n| figure_value(&budget.name, n)),
        ));
    }
    if plan.limits.files.is_none()
        && plan.limits.bytes.is_none()
        && plan.budgets.iter().all(|budget| budget.ceiling.is_none())
    {
        ui.label(
            egui::RichText::new(tr!("No limits set. Set them in Edit build…"))
                .color(theme::TEXT_SECONDARY),
        );
    }
    let slots = by_slot(&plan.items);
    if !slots.is_empty() {
        let parts: Vec<String> = (slots.iter())
            .map(|(slot, n)| format!("{}: {n}", crate::ui::panels::wardrobe::slot_words(*slot)))
            .collect();
        ui.add(egui::Label::new(tr!("By slot: {parts}", parts = parts.join(", "))).wrap());
    }
    if !plan.items.is_empty() {
        ui.label(tr_n!(
            "{count} item keeps a stable id, so the next build can tell what moved",
            "{count} items keep a stable id, so the next build can tell what moved",
            plan.items.len()
        ));
    }
    for problem in &plan.problems {
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!("{LIST_MARK} {}", spoken(problem)))
                    .color(severity_colour(Severity::Error)),
            )
            .wrap(),
        );
    }
    // What the build goes ahead with and a person should know: a file it names by its id and does not ship.
    for note in &plan.notes {
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!("○ {}", spoken(note)))
                    .small()
                    .color(theme::TEXT_SECONDARY),
            )
            .wrap(),
        );
    }
    ui.add_space(theme::SPACING_TIGHT);
    ui.label(
        egui::RichText::new(tr!(
            "Files this build writes ({count})",
            count = plan.files.len()
        ))
        .strong(),
    );
    egui::Frame::group(ui.style()).show(ui, |ui| {
        egui::ScrollArea::vertical()
            .id_salt(("build_plan_files", &plan.resource))
            .max_height(180.0)
            .show_rows(ui, ROW, plan.files.len(), |ui, range| {
                for file in &plan.files[range] {
                    let (how, bytes) = match &file.origin {
                        Origin::Copy { bytes, .. } => ("copied", *bytes),
                        Origin::Retarget { bytes, .. } => ("converted", *bytes),
                        Origin::Cook { bytes, .. } => (tr!("built from source"), *bytes),
                        Origin::Document(_) | Origin::Generated(_) => ("written", file.bytes()),
                    };
                    file_row(
                        ui,
                        &file.path,
                        &format!("{}  ·  {how}", size(bytes)),
                        &file.path,
                    );
                }
            });
    });
    ui.add_space(theme::SPACING_DEFAULT);
}

/// One line for a file: its own name first, and the folder after it in grey, since the end of a path is where two files differ and a long one is cut from the right.
///
/// ⚠ One line each, whatever the path: a row that wrapped would overprint the next, which the list counts as one line tall.
pub(super) fn file_row(ui: &mut egui::Ui, path: &str, what: &str, hover: &str) {
    let (folder, leaf) = path.rsplit_once('/').unwrap_or(("", path));
    let mut job = egui::text::LayoutJob::default();
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    job.append(
        leaf,
        0.0,
        egui::TextFormat::simple(font.clone(), theme::TEXT_PRIMARY.get()),
    );
    job.append(
        &format!("   {what}"),
        0.0,
        egui::TextFormat::simple(font.clone(), theme::TEXT_SECONDARY.get()),
    );
    if !folder.is_empty() {
        job.append(
            &format!("   {folder}"),
            0.0,
            egui::TextFormat::simple(font, theme::TEXT_SECONDARY.get()),
        );
    }
    ui.add(egui::Label::new(job).truncate())
        .on_hover_text(hover);
}

/// What changed in one resource, by stable id.
fn changes_of_one(
    ui: &mut egui::Ui,
    resource: &Resource,
    resources: usize,
    diff: Option<&Diff>,
    by_the_build: bool,
) {
    if resources > 1 {
        ui.label(
            egui::RichText::new(tr!(
                "Resource {resource}",
                resource = resource.plan.resource
            ))
            .strong(),
        );
    }
    match diff {
        None => {
            ui.label(
                egui::RichText::new(tr!(
                    "No earlier build recorded its items, so nothing is compared"
                ))
                .color(theme::TEXT_SECONDARY),
            );
        }
        Some(diff) if diff.is_empty() => {
            ui.label(
                egui::RichText::new(if by_the_build {
                    tr!("This build holds the same as the one it replaced")
                } else {
                    tr!("Nothing differs from the last build")
                })
                .color(theme::TEXT_SECONDARY),
            );
        }
        Some(diff) => diff_view(ui, diff),
    }
}

/// What was added, changed, moved and removed, by stable id, and the redirect map of what moved.
fn diff_view(ui: &mut egui::Ui, diff: &Diff) {
    ui.label(tr!(
        "{count} added, {count_2} changed, {count_3} moved, {count_4} removed",
        count = diff.added.len(),
        count_2 = diff.changed.len(),
        count_3 = diff.moved.len(),
        count_4 = diff.removed.len()
    ));
    let mut named = |what: &str, names: Vec<String>| {
        for name in names.iter().take(LISTED) {
            ui.label(egui::RichText::new(format!("{what}  {name}")).monospace());
        }
        if names.len() > LISTED {
            ui.label(tr!(
                "… and {count} more {what}",
                count = names.len() - LISTED,
                what = what
            ));
        }
    };
    named(
        tr!("added"),
        diff.added.iter().map(|i| i.name.clone()).collect(),
    );
    named(
        tr!("changed"),
        diff.changed.iter().map(|i| i.name.clone()).collect(),
    );
    named(
        tr!("moved"),
        diff.moved
            .iter()
            .map(|m| format!("{} → {}", m.from, m.to))
            .collect(),
    );
    named(
        "removed",
        diff.removed.iter().map(|i| i.name.clone()).collect(),
    );
    if !diff.moved.is_empty() {
        ui.add(
            egui::Label::new(
                egui::RichText::new(
                    tr!("⚠ Something kept its id and changed place. What holds the old name, such as a \
                     saved outfit or a shop table, needs the redirect map."),
                )
                .color(severity_colour(Severity::Warning)),
            )
            .wrap(),
        );
        if ui.button(tr!("Copy the redirect map")).clicked() {
            ui.ctx().copy_text(diff.redirect_map());
        }
    }
}

/// What the last build wrote and read back; the section of the acceptance guide a link of it asked for.
fn verification(
    ui: &mut egui::Ui,
    of: (&Container, &Output, &Sheet),
    last: Option<&LastBuild>,
) -> Option<&'static str> {
    let mut asked = None;
    match last {
        None => {
            ui.label(
                egui::RichText::new(tr!("Not built in this session")).color(theme::TEXT_SECONDARY),
            );
        }
        Some(LastBuild::Built(done)) => {
            for sentence in &done.frozen {
                said(ui, theme::TEXT_SECONDARY, sentence.clone()).on_hover_text(tr!(
                    "Kept in wardrobe.lock.json, beside the pack's wardrobe.json"
                ));
            }
            for part in &done.parts {
                asked = built_part(ui, of, done.at, part).or(asked);
            }
        }
        // The reasons are under the buttons, beside the word Failed; nothing was read back.
        Some(LastBuild::Failed(_)) => {
            ui.label(
                egui::RichText::new(tr!(
                    "Nothing was read back: the build stopped before it was done"
                ))
                .color(theme::TEXT_SECONDARY),
            );
        }
    }
    asked
}

/// One line of what a build found, and how it reads: said in words, the colour only agreeing.
///
/// ⚠ Said through [`spoken`]: a sentence another crate wrote for a document carries backticks and `(s)`.
pub(super) fn said(
    ui: &mut egui::Ui,
    colour: theme::Themed,
    text: impl Into<String>,
) -> egui::Response {
    let text: String = text.into();
    ui.add(egui::Label::new(egui::RichText::new(spoken(&text)).color(colour)).wrap())
}

#[cfg(test)]
mod tests {
    use or_wardrobe::plan::place_name;
    use or_wardrobe::{Component, Ped, Placed, Prop, Wardrobe};

    use super::*;

    fn an_item(name: String) -> Item {
        Item {
            id: name.clone(),
            name,
            digest: String::new(),
        }
    }

    #[test]
    fn a_garments_slot_is_read_from_the_name_the_plan_gives_it() {
        let wardrobe = Wardrobe::new("pack");
        let named = |slot: Slot, position: u32| {
            an_item(place_name(
                &wardrobe,
                &Placed {
                    garment: 0,
                    ped: Ped::Male,
                    slot,
                    position,
                    part: 0,
                    place: position,
                },
            ))
        };
        let tops = Slot::Component(Component::Jbib);
        let hats = Slot::Prop(Prop::Head);
        let items = vec![
            named(tops, 0),
            named(hats, 4),
            named(tops, 1),
            an_item("a_ped_with_no_slot".to_string()),
        ];
        assert_eq!(by_slot(&items), [(tops, 2), (hats, 1)]);
    }

    #[test]
    fn a_figure_is_said_as_a_person_says_it() {
        assert_eq!(figure_words("clothing_drawables"), "Clothing drawables");
        assert_eq!(figure_words("addon_peds"), "Add-on peds");
        // A figure no one has words for yet is still a sentence, not a name.
        assert_eq!(figure_words("weapon_slots_used"), "Weapon slots used");
        assert_eq!(
            against("Files", "2".into(), Some("100".into())),
            "Files: 2 of 100 allowed"
        );
        assert_eq!(against("Files", "2".into(), None), "Files: 2");
    }
}
