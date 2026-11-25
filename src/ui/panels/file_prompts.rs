//! The questions a delete and an extraction ask before they destroy anything
//! on disk, the ones a conversion and a glTF export ask before they write and
//! a glTF import before it changes a model, and the Properties window a row's
//! menu opens.

use super::question_within;
use crate::platform::file_dialogs;
use crate::state::extract_job::OnClash;
use crate::state::gltf::{Asked, Export, Import};
use crate::state::jobs::file_name;
use crate::state::AppState;

/// How wide the conversion's question is.
pub(crate) const QUESTION_WIDTH: f32 = 760.0;

pub fn draw(ctx: &egui::Context, state: &mut AppState) {
    draw_delete(ctx, state);
    draw_clash(ctx, state);
    draw_conversion(ctx, state);
    super::dlc_pack::draw(ctx, state);
    draw_gltf(ctx, state);
    super::batch::draw(ctx, state);
    super::import_report::draw(ctx, state);
    super::reopen_report::draw(ctx, state);
}

enum Answer {
    /// Export…, offering this name.
    Export(String),
    Import,
    Cancel,
}

/// What a glTF export carries before it is written, and what an import replaces before the model changes.
fn draw_gltf(ctx: &egui::Context, state: &mut AppState) {
    let mut answer = None;
    let rerun = match &mut state.gltf.asked {
        Some(Asked::Export(asked)) => {
            let title = tr!("Export {name} to glTF", name = asked.name);
            question_within(ctx, &title, 640.0, |ui| export_body(ui, asked, &mut answer))
        }
        Some(Asked::Import(asked)) => {
            let title = tr!(
                "Replace {name} from {mesh}",
                name = asked.name,
                mesh = file_name(&asked.mesh)
            );
            question_within(ctx, &title, 640.0, |ui| import_body(ui, asked, &mut answer))
        }
        None => return,
    };
    if rerun {
        state.rerun_gltf();
    }
    match answer {
        Some(Answer::Export(name)) => {
            if let Some(dest) = file_dialogs::pick_export_path(&name) {
                state.write_gltf(dest);
            }
        }
        Some(Answer::Import) => state.apply_gltf_import(),
        Some(Answer::Cancel) => {
            state.gltf.cancel();
        }
        None => {}
    }
}

fn weak(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).weak());
}

/// The drawable choice of a `.ydd` or `.yft`, with every drawable at once where `every` is given; whether it changed.
fn drawable_choice<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    label: &str,
    drawables: &[(usize, String)],
    chosen: &mut T,
    every: Option<T>,
    of: impl Fn(usize) -> T,
) -> bool {
    let named = |value: T| {
        (drawables.iter())
            .find(|(entry, _)| of(*entry) == value)
            .map_or_else(
                || tr!("Every drawable").to_string(),
                |(e, name)| format!("{e}  {name}"),
            )
    };
    let mut changed = false;
    egui::ComboBox::from_label(label)
        .selected_text(named(*chosen))
        .show_ui(ui, |ui| {
            if let Some(every) = every {
                changed |= ui
                    .selectable_value(chosen, every, tr!("Every drawable"))
                    .changed();
            }
            for (entry, name) in drawables {
                let text = format!("{entry}  {name}");
                changed |= ui.selectable_value(chosen, of(*entry), text).changed();
            }
        });
    changed
}

/// The buttons under a glTF question, the first enabled once its work is done and holds no refusal.
fn buttons(ui: &mut egui::Ui, verb: &str, ready: Result<(), &str>) -> Option<bool> {
    let mut answer = None;
    ui.add_space(crate::ui::theme::SPACING_LOOSE);
    ui.horizontal(|ui| {
        let go = ui.add_enabled(ready.is_ok(), egui::Button::new(verb));
        if go
            .on_disabled_hover_text(ready.err().unwrap_or(""))
            .clicked()
        {
            answer = Some(true);
        }
        if ui.button(tr!("Cancel")).clicked() {
            answer = Some(false);
        }
    });
    answer
}

/// The export's report and choices; whether a choice changed what it makes.
fn export_body(ui: &mut egui::Ui, asked: &mut Export, answer: &mut Option<Answer>) -> bool {
    let ready = match &asked.made {
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(tr!("Working out what glTF carries of it…"));
            });
            Err(tr!("Still working out what it carries"))
        }
        Some(Ok(scene)) => {
            let counted = |n: usize, one: &str, many: &str| match n {
                1 => format!("1 {one}"),
                n => format!("{n} {many}"),
            };
            let meshes = scene.objects.iter().map(|o| o.meshes.len()).sum();
            ui.label(
                egui::RichText::new(tr!(
                    "{meshes}, {vertex_count}, {triangle_count}, {count} and {count_2}",
                    meshes = counted(meshes, "mesh", "meshes"),
                    vertex_count = counted(scene.vertex_count(), "vertex", "vertices"),
                    triangle_count = counted(scene.triangle_count(), "triangle", "triangles"),
                    count = counted(scene.materials.len(), "material", "materials"),
                    count_2 = counted(scene.images.len(), "texture", "textures")
                ))
                .strong(),
            );
            let notes = or_interchange::report_lines(&scene.report);
            if notes.is_empty() {
                ui.label(tr!("Nothing it reads is left out."));
            }
            for note in notes {
                ui.label(note);
            }
            Ok(())
        }
        Some(Err(why)) => {
            ui.label(egui::RichText::new(tr!("It cannot be exported:")).strong());
            ui.label(why.as_str());
            Err(tr!("Nothing here can be exported"))
        }
    };
    ui.add_space(crate::ui::theme::SPACING_DEFAULT);
    let choice = &mut asked.choice;
    let mut changed = false;
    if !asked.drawables.is_empty() {
        let drawables = &asked.drawables;
        changed |= drawable_choice(
            ui,
            tr!("Drawable to export"),
            drawables,
            &mut choice.entry,
            Some(None),
            Some,
        );
        weak(
            ui,
            tr!(
                "An import replaces one drawable, so a mesh meant to come back goes out one \
             drawable at a time."
            ),
        );
    }
    changed |= ui
        .checkbox(&mut choice.every_lod, tr!("Every level of detail"))
        .changed();
    weak(
        ui,
        tr!(
            "Off, only the most detailed level goes out, and importing it leaves the other \
         levels as they are."
        ),
    );
    changed |= ui
        .checkbox(&mut choice.textures, tr!("Its own textures, as PNG"))
        .changed();
    ui.horizontal_wrapped(|ui| {
        ui.radio_value(&mut choice.glb, true, tr!("One .glb"));
        ui.radio_value(
            &mut choice.glb,
            false,
            tr!(".gltf beside a .bin and a .png per texture"),
        );
    });
    match buttons(ui, tr!("Export…"), ready) {
        Some(true) => {
            let stem = std::path::Path::new(&asked.name)
                .file_stem()
                .map_or_else(|| "mesh".to_string(), |s| s.to_string_lossy().into_owned());
            let ext = if choice.glb { "glb" } else { "gltf" };
            *answer = Some(Answer::Export(format!("{stem}.{ext}")));
        }
        Some(false) => *answer = Some(Answer::Cancel),
        None => {}
    }
    changed
}

/// The import's report and choices; whether a choice changed what it makes.
fn import_body(ui: &mut egui::Ui, asked: &mut Import, answer: &mut Option<Answer>) -> bool {
    let ready = match &asked.made {
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(tr!("Reading the mesh and fitting it to the model…"));
            });
            Err(tr!("Still reading the mesh"))
        }
        Some(Ok((_, report))) => {
            ui.label(
                egui::RichText::new(tr!("{name} changes as one step you can undo; nothing is written until you save it.", name = asked.name))
                .strong(),
            );
            // A GTA VI import's lines are `rpf scan-import`'s, geometry by geometry.
            let lines = match asked.is_gta6() {
                true => report.splits.clone(),
                false => report.lines(),
            };
            for line in lines {
                ui.label(line);
            }
            Ok(())
        }
        Some(Err(why)) => {
            let refused = tr!(
                "It cannot replace {name}, which is unchanged:",
                name = asked.name
            );
            ui.label(egui::RichText::new(refused).strong());
            ui.label(why.as_str());
            Err(tr!("The mesh cannot replace the model as it is"))
        }
    };
    ui.add_space(crate::ui::theme::SPACING_DEFAULT);
    // A GTA VI geometry keeps the channels the mesh does not replace at its own vertex count, so
    // only the fill, which acts at another count, is asked.
    if asked.is_gta6() {
        let filled = ui
            .checkbox(
                &mut asked.choice.fill_missing,
                tr!("Fill a channel the mesh leaves out"),
            )
            .changed();
        weak(ui, tr!("Each geometry the mesh names takes its positions, normals, tangents, texcoords, colours and triangles, and its weights and blend indices from the mesh's joints, adding to its palette a bone it lacks. A channel the mesh leaves out is kept as the geometry holds it at its own vertex count and refused at another, unless the box above is ticked: then a normal is filled with +Z, a tangent with +X, a colour with white, a UV with 0 and a skin with a whole weight on its first bone."));
        let filled = filled
            | ui.checkbox(
                &mut asked.choice.lower_lods,
                tr!("Replace the lower levels of detail too"),
            )
            .changed();
        weak(ui, tr!("Off, a level of detail the mesh names no geometry of is left as the model holds it. On, each such geometry takes the level-0 mesh at its model and place where both draw with one shader."));
        match buttons(ui, tr!("Replace"), ready) {
            Some(true) => *answer = Some(Answer::Import),
            Some(false) => *answer = Some(Answer::Cancel),
            None => {}
        }
        return filled;
    }
    let choice = &mut asked.choice;
    let mut changed = false;
    if !asked.drawables.is_empty() {
        let drawables = &asked.drawables;
        changed |= drawable_choice(
            ui,
            tr!("Drawable it replaces"),
            drawables,
            &mut choice.entry,
            None,
            |e| e,
        );
    }
    changed |= ui
        .checkbox(
            &mut choice.fill_missing,
            tr!("Fill a channel the mesh leaves out"),
        )
        .changed();
    weak(
        ui,
        tr!(
            "A channel the model's vertices hold and the mesh does not state is filled with a \
         default: a normal with +Z, a tangent with +X, a colour with white, a UV with 0, a \
         skin with a whole weight on its first bone. Off, the import is refused instead, \
         since a filled normal or colour is data the source never stated, and it renders."
        ),
    );
    changed |= ui
        .checkbox(
            &mut choice.keep_skinning,
            tr!("Keep the model's own skinning"),
        )
        .changed();
    weak(
        ui,
        tr!(
            "The bone weights already in the model are carried across when the mesh states \
         none, as a ped component's cannot, its skeleton being in another file. Refused if \
         the vertex count moved, because then the old weights name other vertices."
        ),
    );
    match buttons(ui, tr!("Replace"), ready) {
        Some(true) => *answer = Some(Answer::Import),
        Some(false) => *answer = Some(Answer::Cancel),
        None => {}
    }
    changed
}

/// What a conversion loses, or why it cannot be made, before anything is written.
///
/// ⚠ One file is written to a file its Convert dialog names, as Export as…
/// writes a row; several go into a folder, each under its own name, since
/// there is no one name to give them. A folder that holds some of them asks
/// Replace, Skip or Keep both instead, which hides this question until it is
/// answered.
fn draw_conversion(ctx: &egui::Context, state: &mut AppState) {
    let Some(done) = &state.convert.done else {
        return;
    };
    if state.convert.clash.is_some() {
        return;
    }
    let (title, summary, report) = (done.title(), done.summary(), done.report());
    let made: Vec<String> = done.made().map(|(name, _)| name.to_string()).collect();
    let (without_database, of_document) = (done.without_database, done.of_document);
    let mut answer: Option<bool> = None;
    // ⚠ Fixed, so a value that names a long texture wraps inside it rather than widening it.
    question_within(ctx, &title, QUESTION_WIDTH, |ui| {
        ui.label(egui::RichText::new(summary).strong());
        ui.add_space(crate::ui::theme::SPACING_TIGHT);
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.visuals_mut().indent_has_left_vline = false;
                for (file, lines) in report.iter().enumerate() {
                    report_line(ui, &lines.line);
                    for (value, words) in lines.dropped.iter().enumerate() {
                        ui.indent((file, value), |ui| report_line(ui, words));
                    }
                }
            });
        ui.add_space(crate::ui::theme::SPACING_SNUG);
        ui.label(
            egui::RichText::new(tr!(
                "Nothing is invented: each value listed is dropped, and what only the \
                 other edition states is left at its default."
            ))
            .weak(),
        );
        if of_document {
            weak(
                ui,
                tr!("It converts the open document as it stands, unsaved edits included."),
            );
        }
        if without_database {
            ui.label(
                egui::RichText::new(tr!(
                    "No shader database is loaded, so each material parameter is dropped \
                     rather than respelled. Mounting an install loads one."
                ))
                .weak(),
            );
        }
        ui.add_space(crate::ui::theme::SPACING_LOOSE);
        ui.horizontal(|ui| {
            let convert = ui
                .add_enabled(!made.is_empty(), egui::Button::new(tr!("Convert…")))
                .on_hover_text(tr!("Choose where it is written"))
                .on_disabled_hover_text(tr!("Nothing here can be converted"));
            if convert.clicked() {
                answer = Some(true);
            }
            if ui.button(tr!("Cancel")).clicked() {
                answer = Some(false);
            }
        });
    });
    match answer {
        Some(true) => {
            let dest = match &made[..] {
                [one] => file_dialogs::pick_conversion_path(one),
                _ => file_dialogs::pick_conversion_folder(),
            };
            if let Some(dest) = dest {
                state.write_conversion(dest, None);
            }
        }
        Some(false) => state.convert.done = None,
        None => {}
    }
}

/// One line of a conversion's report, wrapped at the question's edge and selected on its own.
fn report_line(ui: &mut egui::Ui, text: &str) {
    let text = egui::RichText::new(text).monospace();
    ui.add(egui::Label::new(text).wrap().selectable(true));
}

fn draw_delete(ctx: &egui::Context, state: &mut AppState) {
    let Some(asked) = state.pending_delete.clone() else {
        return;
    };
    let mut answer: Option<bool> = None;
    question_within(ctx, tr!("Delete"), 380.0, |ui| {
        ui.label(egui::RichText::new(asked.question()).strong());
        ui.add_space(crate::ui::theme::SPACING_TIGHT);
        ui.label(asked.contents());
        ui.add_space(crate::ui::theme::SPACING_SNUG);
        ui.label(asked.consequence());
        ui.add_space(crate::ui::theme::SPACING_LOOSE);
        ui.horizontal(|ui| {
            if ui.button(asked.verb()).clicked() {
                answer = Some(true);
            }
            if ui.button(tr!("Cancel")).clicked() {
                answer = Some(false);
            }
        });
    });
    if let Some(confirmed) = answer {
        state.answer_delete(confirmed);
    }
}

/// The files an extraction or a conversion finds already in its folder, asked about once for the whole job.
fn draw_clash(ctx: &egui::Context, state: &mut AppState) {
    let converting = state.extract_jobs.clash.is_none();
    let (asked, dest, made) = match (state.extract_jobs.clash.as_ref(), &state.convert.clash) {
        (Some(clash), _) => (clash.question(), &clash.dest, "extracted"),
        (None, Some(clash)) => (clash.question(), &clash.dest, "converted"),
        (None, None) => return,
    };
    let dest = dest.display().to_string();
    let mut answer: Option<Option<OnClash>> = None;
    question_within(ctx, tr!("Files already there"), 420.0, |ui| {
        ui.label(egui::RichText::new(asked).strong());
        ui.label(egui::RichText::new(dest).monospace());
        ui.add_space(crate::ui::theme::SPACING_SNUG);
        ui.label(
            egui::RichText::new(tr!(
                "The answer holds for every one of them. Keep both writes the \
                 {made} copy beside the old one as “name (2).ext”.",
                made = made
            ))
            .weak(),
        );
        ui.add_space(crate::ui::theme::SPACING_LOOSE);
        ui.horizontal(|ui| {
            for (label, choice) in [
                (tr!("Replace"), OnClash::Replace),
                (tr!("Skip"), OnClash::Skip),
                (tr!("Keep both"), OnClash::KeepBoth),
            ] {
                if ui.button(label).clicked() {
                    answer = Some(Some(choice));
                }
            }
            if ui.button(tr!("Cancel")).clicked() {
                answer = Some(None);
            }
        });
    });
    match answer {
        Some(choice) if converting => state.answer_conversion_clash(choice),
        Some(choice) => state.answer_extract_clash(choice),
        None => {}
    }
}
