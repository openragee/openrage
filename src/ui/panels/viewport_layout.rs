//! Where the viewport's overlays go: down its right edge the shading pills, the drawable's row,
//! the mode's bar and the axis gizmo, each below the last; and what the box at its bottom left says.

use crate::state::AppState;
use crate::ui::dock::Workspace;

pub(crate) const GIZMO_SIZE: f32 = 84.0;

/// The tops of the viewport's right-hand column, and the axis gizmo's box below them.
pub(crate) struct Column {
    pub row: f32,
    pub bar: f32,
    pub gizmo: egui::Rect,
}

/// The column in `rect`, under the pills, with a row `row_h` and a bar `bar_h` tall, each gap included.
pub(crate) fn column(rect: egui::Rect, row_h: f32, bar_h: f32) -> Column {
    let row = rect.top() + 8.0 + super::viewport::SHADING_PILL_H + 6.0;
    let bar = row + row_h;
    let top = bar + bar_h + 2.0;
    // A short viewport under a wrapped bar has less room than the gizmo's size: it shrinks to stay inside.
    let size = GIZMO_SIZE.min(rect.bottom() - top).max(0.0);
    let gizmo = egui::Rect::from_min_size(
        egui::pos2(rect.right() - 10.0 - size, top),
        egui::vec2(size, size),
    );
    Column { row, bar, gizmo }
}

/// What the box at the viewport's bottom left says, and which of its lines names a pick.
pub(crate) struct Overlay {
    pub text: String,
    /// The line of `text`, counting from 0, that names what a click last picked in the Animation preview.
    pub picked: Option<usize>,
}

/// What the box says, as a string.
#[cfg(test)]
pub(crate) fn overlay_text(
    state: &AppState,
    retarget: Option<&or_anim::PaletteRetarget>,
) -> String {
    overlay(state, retarget, None).text
}

/// What the box at the viewport's bottom left says: what it shows, in words, what Shaded says of a
/// GTA VI drawable, such as why it draws as Material, and how to move.
pub(crate) fn overlay(
    state: &AppState,
    retarget: Option<&or_anim::PaletteRetarget>,
    shaded: Option<&str>,
) -> Overlay {
    let mut picked = None;
    let mut lines = match state.active_document() {
        Some(_) if state.active_workspace == Workspace::Animation => {
            let (lines, names_a_pick) = preview_lines(state);
            picked = names_a_pick.then(|| lines.len() - 1);
            lines
        }
        Some(doc) => vec![format!("{} · {}", doc.file_name(), doc.kind.info().noun)],
        None => vec![tr!("Nothing open").to_string()],
    };
    // ⚠ A mode that draws as another says so where it is read: the pill's tooltip alone is not seen.
    lines.extend(shaded.map(str::to_string));
    let sim = &state.sim;
    if state.active_workspace == Workspace::Particle && sim.last_built.is_some() {
        let (t, length) = (sim.manager.current_time(), sim.manager.duration());
        let live = sim.manager.live_count();
        lines.push(tr!(
            "{t} of {length} s · {live} particles",
            t = format!("{t:.2}"),
            length = format!("{length:.2}"),
            live = live
        ));
    } else {
        lines.push(
            tr!("Drag to orbit · Shift+drag to pan · Click to select · Alt+click for a model")
                .to_string(),
        );
        lines.push(super::viewport::framing_hint(&state.prefs));
    }
    // ⚠ A retarget that says nothing is worse than none: the mesh has moved
    // onto another skeleton's indices and a reader has no way to tell. Only
    // shown where a palette carried the tags to move it by, which is also
    // the only case where anything moved.
    lines.extend(retarget.map(or_anim::PaletteRetarget::summary));
    lines.retain(|line| !line.is_empty());
    Overlay {
        text: lines.join("\n"),
        picked,
    }
}

/// What the Animation preview draws, a line for each drawable and one for the skeleton whose
/// bones it shows, and what a click last picked, which is the last line when the flag is set: the
/// active document is a clip dictionary, which it draws none of.
fn preview_lines(state: &AppState) -> (Vec<String>, bool) {
    const NAMED: usize = 3;
    let scene = &state.preview_scene;
    let sources = scene.drawn_sources();
    let mut lines: Vec<String> = (sources.iter())
        .filter_map(|(doc, entry)| {
            let doc = state.document(*doc)?;
            let entry = entry.map(|index| format!(" [{index}]")).unwrap_or_default();
            Some(format!(
                "{}{entry} · {}",
                doc.file_name(),
                doc.kind.info().noun
            ))
        })
        .take(NAMED)
        .collect();
    if sources.len() > NAMED {
        lines.push(format!("and {} more", sources.len() - NAMED));
    }
    // A GTA VI animation draws the model its Inspector picked, which no Preview source names.
    let gta6_model = crate::ui::panels::inspector::gta6_model_drawn(state);
    lines.extend(
        (gta6_model.and_then(|(doc, _)| state.document(doc)))
            .map(|doc| format!("{} · {}", doc.file_name(), doc.kind.info().noun)),
    );
    let skeleton = crate::render::viewport::bones::drawn_from(state);
    let bones = skeleton.and_then(|doc| state.document(doc));
    lines.extend(bones.map(|doc| format!("Bones of {}", doc.file_name())));
    if lines.is_empty() {
        // A skeleton is chosen, so what is missing is what draws it: its own model, or its bones
        lines.push(if scene.skeleton_source.is_some() {
            "Nothing to draw · the skeleton file's model is hidden and Show bones is off"
                .to_string()
        } else {
            "Nothing to draw · choose a mesh or a skeleton in Preview".to_string()
        });
    }
    // The pick is held apart from the selection, which the clip dictionary keeps; a pick in a
    // document the preview no longer draws is not worth saying.
    let picked = (scene.picked.as_ref()).filter(|picked| {
        skeleton == Some(picked.document) || sources.iter().any(|(doc, _)| *doc == picked.document)
    });
    let picked = picked.and_then(|picked| {
        let doc = state.document(picked.document)?;
        Some(format!("Picked: {} · {}", picked.label, doc.file_name()))
    });
    let names_a_pick = picked.is_some();
    lines.extend(picked);
    (lines, names_a_pick)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_box_says_what_shaded_says_of_a_drawable_that_draws_as_material() {
        let state = AppState::from_prefs(crate::state::Prefs::default());
        let why = "Shaded draws as Material: a made-up reason";
        assert!(overlay(&state, None, Some(why)).text.contains(why));
        assert!(!overlay(&state, None, None).text.contains("Shaded"));
    }
}
