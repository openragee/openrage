use egui_phosphor::regular as ph;

use crate::state::particle_target::{on_particle_target, ParticleTarget};
use crate::state::{capabilities, AppState, DocPayload, DocumentId, DocumentKind};
use crate::ui::panels::selection_ids::{
    dict_entry_unpack_part, pack_selection, payload_index, root_selection_id, section_of,
    texture_name_for_selection, unpack_lod_sub_index, unpack_selection, ydd_selected_entry_index,
    yld_selected_entry_index, ypt_model_texture_name_for_selection, ypt_tag_of,
    ypt_texture_name_for_selection, ypt_unpack_behaviour, ypt_unpack_domain, ypt_unpack_effect,
    ypt_unpack_emitter, ypt_unpack_emitter_lib, ypt_unpack_model_part, ypt_unpack_particle,
    ypt_unpack_particle_lib, DOC_ROOT_SEL, SECT_BOUND, SECT_LIGHT, SECT_LOD, SECT_SHADER,
    SECT_TEXTURE, YPT_BEHAVIOUR, YPT_DOMAIN, YPT_DOMAIN_ATTRACTOR, YPT_DOMAIN_CREATION,
    YPT_DOMAIN_TARGET, YPT_EFFECT, YPT_EMITTER, YPT_EMITTER_LIB, YPT_MODEL, YPT_MODEL_DICT,
    YPT_MODEL_KIND_BONE, YPT_MODEL_KIND_BOUND, YPT_MODEL_KIND_LIGHT, YPT_MODEL_KIND_LOD,
    YPT_MODEL_KIND_MODEL, YPT_MODEL_KIND_SHADER, YPT_MODEL_KIND_TEXTURE, YPT_PARTICLE,
    YPT_PARTICLE_LIB, YPT_TEXTURE,
};
mod awc;
mod bik;
mod bone;
#[cfg(test)]
pub(crate) use bone::PHYSICS_STAYS;
mod dat_tables;
mod drawable;
mod field_menu;
mod fxc;
mod gen9;
mod gfx;
mod gta6_anim;
pub(crate) use gta6_anim::posed as gta6_posed;
pub(crate) use gta6_anim::{
    choice_count as gta6_choice_count, choice_label as gta6_choice_label,
    draw_timeline as gta6_timeline, drawn as gta6_drawn, model_drawn as gta6_model_drawn,
};
mod gta6_dialogue;
mod gta6_drawable;
mod gta6_meta_ids;
mod gta6_move;
mod gta6_particles;
pub(crate) mod gta6_text;
mod hex;
mod material;
mod move_network;
mod rel;
mod rpf;
mod rpf9;
pub(crate) use rpf9::row_kind_label as rpf9_row_kind_label;
mod rpf9_entry;
mod structure;
mod summary;
mod text;
mod ttpf;
mod world_dat;
mod ybn;
mod ycd;
mod ydd;
mod ydr;
mod yft;
mod yld;
mod ypdb;
mod ypt;
mod ypt_behaviour;
mod ypt_model;
mod ypt_selected;
pub(crate) mod ypt_steps;
mod ysc;
mod ytd;

use crate::ui::panels::drag_value;
use crate::ui::theme;
use awc::*;
use bik::*;
use bone::*;
use dat_tables::*;
use drawable::*;
use fxc::*;
use gen9::*;
use gfx::*;
use hex::*;
use move_network::*;
use rel::*;
use rpf::*;
use rpf9::*;
use structure::*;
use summary::*;
use text::*;
use ttpf::*;
use world_dat::*;
use ybn::*;
use ycd::*;
use ydd::*;
use ydr::*;
use yft::*;
use yld::*;
use ypdb::*;
use ypt::*;
use ypt_behaviour::*;
use ypt_model::*;
use ypt_selected::*;
use ysc::*;
use ytd::*;

#[cfg(test)]
pub(crate) use structure::summary_at;
pub(crate) use structure::{declared_for, summary_in, Declared};
pub(crate) use summary::plural;

#[derive(Debug, Clone, Default)]
pub struct InspectorUiState {
    pub pinned: Option<(DocumentId, u64)>,
    pub filter: String,
}

impl crate::ui::panel_state::PanelState for InspectorUiState {
    const KEY: &'static str = "inspector";
}

pub fn draw(ui: &mut egui::Ui, state: &mut AppState) {
    let filter = state.panels.get::<InspectorUiState>().filter.clone();
    write_filter_to_ctx(ui.ctx(), &filter);

    let pinned = state.panels.get::<InspectorUiState>().pinned;
    if pinned.is_none() && super::empty_state::has_nothing_to_show(state) {
        if super::empty_state::drawn_by_the_document_panel(state) {
            empty(ui, super::empty_state::heading(state.active_workspace));
        } else {
            super::empty_state::draw(ui, state);
        }
        return;
    }

    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            draw_header(ui, state);

            let (doc_id, body, selection) = if let Some((doc_id, sel)) = pinned {
                match state.document(doc_id) {
                    Some(doc) => (doc.id, body_for(doc), Some(sel)),
                    None => {
                        ui.colored_label(
                            theme::WARNING,
                            "Pinned document closed — clear the pin to follow the active selection",
                        );
                        return;
                    }
                }
            } else {
                let Some(doc) = state.active_document() else {
                    return;
                };
                (doc.id, body_for(doc), state.selection.selected(doc.id))
            };

            if super::document_view::draws_the_body_of(state, doc_id) {
                draw_facts(ui, state, doc_id);
            } else {
                draw_document_body(ui, state, doc_id, body, selection);
            }
            draw_scan_origin(ui, state, doc_id);
        });
}

/// The Files panel's list of what the scan of the GTA VI archive `doc` found.
pub fn draw_scan_list(ui: &mut egui::Ui, state: &mut AppState, doc: DocumentId) {
    draw_scan_rows(ui, state, doc);
}

/// The active document's body without the inspector's header: the Document
/// panel's view of a kind with no editor of its own.
pub fn draw_body(ui: &mut egui::Ui, state: &mut AppState) {
    let Some((doc_id, body)) = state.active_document().map(|doc| (doc.id, body_for(doc))) else {
        return;
    };
    let selection = state.selection.selected(doc_id);
    egui::ScrollArea::both()
        .id_salt("document_body")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            draw_document_body(ui, state, doc_id, body, selection)
        });
}

fn draw_document_body(
    ui: &mut egui::Ui,
    state: &mut AppState,
    doc_id: DocumentId,
    body: Option<capabilities::InspectorFn>,
    selection: Option<u64>,
) {
    match body {
        Some(draw) => draw(ui, state, doc_id, selection),
        None => {
            // ⚠ The material editor first: a Shader node is a
            // selection the texture surface does not recognise, so
            // the order decides nothing, but the material panel
            // needs the payload mutably and the described root does
            // not.
            if !draw_enhanced_material(ui, state, doc_id, selection)
                && !draw_enhanced_texture(ui, state, doc_id, selection)
                && !draw_selected_bone(ui, state, doc_id, selection)
            {
                draw_described_root(ui, state, doc_id);
            }
        }
    }
}

/// What the inspector shows of a document the Document panel draws.
fn draw_facts(ui: &mut egui::Ui, state: &AppState, doc_id: DocumentId) {
    let Some(doc) = state.document(doc_id) else {
        return;
    };
    ui.heading(doc.kind.info().noun);
    ui.separator();
    grid(ui, "document_facts", |ui| {
        row(ui, "Name", &doc.name);
        if let Some(path) = &doc.path {
            row(ui, "Path", &path.display().to_string());
        }
    });
}

/// The kind's own Inspector body, which an Enhanced file has only where the body reads both targets.
fn body_for(doc: &crate::state::Document) -> Option<capabilities::InspectorFn> {
    if doc.payload.is_enhanced() && doc.kind != DocumentKind::Ypt {
        return None;
    }
    capabilities::for_payload(&doc.payload).and_then(|row| row.inspector)
}

fn draw_described_root(ui: &mut egui::Ui, state: &mut AppState, doc_id: crate::state::DocumentId) {
    let (summary, described, noun, route) = {
        let Some(doc) = state.document(doc_id) else {
            return;
        };
        let selected = state
            .selection
            .described(doc_id)
            .filter(|path| !path.is_root());
        let node = selected.and_then(|path| {
            crate::ui::panels::hierarchy::described_outline(doc)
                .and_then(|outline| outline.find(path).cloned())
        });
        let summary = match node
            .as_ref()
            .and_then(|n| n.type_name.as_deref().map(|name| (name, n.at)))
        {
            Some((name, at)) => summary_at(doc, name, at),
            None => pack_root_summary(state, doc),
        };
        (
            summary,
            or_io::gta5::formats::format(doc.kind.extension()).is_some(),
            doc.kind.info().noun.to_ascii_lowercase(),
            selected.map(|p| p.to_string()),
        )
    };

    if let Some(summary) = summary {
        match route {
            Some(route) => {
                ui.label(
                    egui::RichText::new(route)
                        .small()
                        .color(theme::TEXT_SECONDARY),
                );
                draw_editable_summary(ui, "described_root", state, doc_id, &summary);
            }
            None => {
                if let Some(doc) = state.document(doc_id) {
                    draw_summary(ui, doc, Vec::new());
                }
                ui.add_space(theme::SPACING_DEFAULT);
                draw_editable_summary(ui, "described_root", state, doc_id, &summary);
                ui.add_space(theme::SPACING_DEFAULT);
                select_in(ui, state, doc_id, tr_key!("an item"));
            }
        }
        return;
    }

    let why = if !described {
        format!("OpenRage cannot read the fields of a {noun}.")
    } else {
        format!("OpenRage cannot read the fields of this version of a {noun}.")
    };
    empty(ui, &why);
}

/// Nothing is selected in `doc_id`: what to select, naming the file rather
/// than a panel, since the tree holds every open file.
fn select_in(ui: &mut egui::Ui, state: &AppState, doc_id: DocumentId, what: &'static str) {
    let name = state
        .document(doc_id)
        .map(|doc| doc.file_name())
        .unwrap_or_default();
    empty(
        ui,
        &tr!(
            "Select {what} in {name}",
            what = crate::i18n::tr(what),
            name = name
        ),
    );
}

pub(crate) mod rows {
    use super::*;

    pub(crate) fn ypt(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        selection: Option<u64>,
    ) {
        draw_ypt(ui, state, doc_id, selection);
        if let Some(sel) = selection {
            // ⚠ The live sliders drive the preview, which plays the active document's effect.
            if ypt_tag_of(sel) == YPT_EFFECT && state.active_document == Some(doc_id) {
                draw_ypt_evolution_sliders(ui, state);
            }
        }
        super::gta6_particles::draw_scanned(ui, state, doc_id);
    }

    pub(crate) fn ytd(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        selection: Option<u64>,
    ) {
        draw_ytd(ui, state, doc_id, selection);
    }

    pub(crate) fn ydr(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        selection: Option<u64>,
    ) {
        draw_ydr(ui, state, doc_id, selection);
    }

    pub(crate) fn ydd(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        selection: Option<u64>,
    ) {
        draw_ydd(ui, state, doc_id, selection);
    }

    pub(crate) fn yft(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        selection: Option<u64>,
    ) {
        draw_yft(ui, state, doc_id, selection);
    }

    pub(crate) fn ybn(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        selection: Option<u64>,
    ) {
        draw_ybn(ui, state, doc_id, selection);
    }

    pub(crate) fn yld(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        selection: Option<u64>,
    ) {
        draw_yld(ui, state, doc_id, selection);
    }

    pub(crate) fn ypdb(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        selection: Option<u64>,
    ) {
        draw_ypdb(ui, state, doc_id, selection);
    }

    pub(crate) fn fxc(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        selection: Option<u64>,
    ) {
        draw_fxc(ui, state, doc_id, selection);
    }

    pub(crate) fn ysc(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        _selection: Option<u64>,
    ) {
        draw_ysc(ui, state, doc_id);
    }

    pub(crate) fn ycd(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        selection: Option<u64>,
    ) {
        draw_ycd(ui, state, doc_id, selection);
    }

    pub(crate) fn rpf(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        selection: Option<u64>,
    ) {
        draw_rpf(ui, state, doc_id, selection);
    }

    pub(crate) fn bik(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        _selection: Option<u64>,
    ) {
        draw_bik(ui, state, doc_id);
    }

    pub(crate) fn awc(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        _selection: Option<u64>,
    ) {
        draw_awc(ui, state, doc_id);
    }

    pub(crate) fn height_map(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        _selection: Option<u64>,
    ) {
        draw_height_map(ui, state, doc_id);
    }

    pub(crate) fn distant_lights(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        _selection: Option<u64>,
    ) {
        draw_distant_lights(ui, state, doc_id);
    }

    pub(crate) fn water_map(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        _selection: Option<u64>,
    ) {
        draw_water_map(ui, state, doc_id);
    }

    pub(crate) fn world_blanket(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        _selection: Option<u64>,
    ) {
        draw_world_blanket(ui, state, doc_id);
    }

    pub(crate) fn nav_precalc(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        _selection: Option<u64>,
    ) {
        draw_nav_precalc(ui, state, doc_id);
    }

    pub(crate) fn dat_table(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        _selection: Option<u64>,
    ) {
        draw_dat_table(ui, state, doc_id);
    }

    pub(crate) fn rel(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        _selection: Option<u64>,
    ) {
        draw_rel(ui, state, doc_id);
    }

    pub(crate) fn gfx(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        _selection: Option<u64>,
    ) {
        draw_gfx(ui, state, doc_id);
    }

    pub(crate) fn move_network(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        _selection: Option<u64>,
    ) {
        draw_move_network(ui, state, doc_id);
    }

    pub(crate) fn dds(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        _selection: Option<u64>,
    ) {
        draw_dds(ui, state, doc_id);
    }

    pub(crate) fn ttpf(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        selection: Option<u64>,
    ) {
        draw_ttpf(ui, state, doc_id, selection);
    }

    pub(crate) fn rpf9(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        _selection: Option<u64>,
    ) {
        draw_rpf9(ui, state, doc_id);
    }

    pub(crate) fn gta6_anim(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        _selection: Option<u64>,
    ) {
        super::gta6_anim::draw_gta6_anim(ui, state, doc_id);
    }

    pub(crate) fn gta6_drawable(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        selection: Option<u64>,
    ) {
        super::gta6_drawable::draw_gta6_drawable(ui, state, doc_id, selection);
    }

    /// A navigation mesh's described root, after what a scanned GTA VI cell states beside it.
    pub(crate) fn ynv(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        selection: Option<u64>,
    ) {
        let cell = super::gta6_drawable::draw_gta6_cell(ui, state, doc_id, selection);
        // A cell's model is GTA V's, built from it: its root holds a GTA V edition and a header the
        // cell does not fill, so only a node of it is described.
        let node = (state.selection.described(doc_id)).is_some_and(|path| !path.is_root());
        if !cell || node {
            super::draw_described_root(ui, state, doc_id);
        }
    }

    pub(crate) fn hex(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        _selection: Option<u64>,
    ) {
        draw_hex(ui, state, doc_id);
    }

    pub(crate) fn gxt2(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        _selection: Option<u64>,
    ) {
        draw_gxt2(ui, state, doc_id);
    }

    pub(crate) fn gta6_text(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        _selection: Option<u64>,
    ) {
        super::gta6_text::draw_text_table(ui, state, doc_id);
    }

    pub(crate) fn gta6_audio_data(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        _selection: Option<u64>,
    ) {
        super::gta6_text::draw_audio_data(ui, state, doc_id);
    }

    pub(crate) fn text(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        _selection: Option<u64>,
    ) {
        draw_text(ui, state, doc_id);
    }

    pub(crate) fn name_table(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        _selection: Option<u64>,
    ) {
        draw_name_table(ui, state, doc_id);
    }

    pub(crate) fn meta(
        ui: &mut egui::Ui,
        state: &mut AppState,
        doc_id: crate::state::DocumentId,
        selection: Option<u64>,
    ) {
        draw_meta(ui, state, doc_id, selection);
    }
}

fn draw_header(ui: &mut egui::Ui, state: &mut AppState) {
    ui.horizontal(|ui| {
        let mut internals = state.prefs.show_internals;
        let shown = ui
            .checkbox(&mut internals, tr!("Show internals"))
            .on_hover_text(tr!("Also list the engine's bookkeeping: the words it fills in at run time, counts, reference counts, padding, addresses, binary format codes and vertex strides."));
        if shown.changed() {
            state.set_show_internals(internals);
        }

        let pinned = state.panels.get::<InspectorUiState>().pinned;
        let pin_text = if pinned.is_some() {
            tr!("📌 Unpin")
        } else {
            tr!("📌 Pin")
        };
        if ui
            .small_button(pin_text)
            .on_hover_text(tr!("Pin the current selection so it stays visible while you click around the hierarchy. Click again to unpin."))
            .clicked()
        {
            if pinned.is_some() {
                state.panels.get::<InspectorUiState>().pinned = None;
            } else if let Some(pin) = pin_target(state) {
                state.panels.get::<InspectorUiState>().pinned = Some(pin);
            }
        }

        let focus_target = focus_target_for_header(state);
        let focus_label =
            format!("{} {}", ph::CROSSHAIR_SIMPLE, tr!("Focus"));
        let focus_resp = ui.add_enabled(
            focus_target.is_some(),
            egui::Button::new(focus_label).small(),
        );
        let focus_resp = focus_resp.on_hover_text(tr!("Reveal the inspected element in the Outline — expands every enclosing section and scrolls the row into view."));
        if focus_resp.clicked() {
            if let Some((doc_id, sel)) = focus_target {
                super::hierarchy::ui_state(state).scroll_to = Some((doc_id, sel));
                let ws = state.active_workspace;
                state
                    .workspace_docks
                    .reveal(ws, super::registry::ids::OUTLINE);
                ui.ctx().request_repaint();
            }
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let edit =
                egui::TextEdit::singleline(&mut state.panels.get::<InspectorUiState>().filter)
                    .id_salt("inspector_property_filter_input")
                    .hint_text(tr!("Filter properties…"))
                    .desired_width(160.0);
            ui.add(edit);
        });
    });
    ui.separator();
}

/// What Pin holds: the active document's selection, or a GTA VI animation whole, which has nothing
/// to select and is pinned so its pose plays on a model in the Model view.
fn pin_target(state: &AppState) -> Option<(DocumentId, u64)> {
    let doc = state.active_document()?;
    let whole = (doc.kind == DocumentKind::Gta6Anim).then_some(DOC_ROOT_SEL);
    let sel = state.selection.selected(doc.id).or(whole)?;
    Some((doc.id, sel))
}

fn focus_target_for_header(state: &AppState) -> Option<(DocumentId, u64)> {
    if let Some((doc_id, sel)) = state
        .panels
        .peek::<InspectorUiState>()
        .and_then(|s| s.pinned)
    {
        if state.document(doc_id).is_some() {
            return Some((doc_id, sel));
        }
    }
    let doc = state.active_document()?;
    let sel = state.selection.selected(doc.id)?;
    Some((doc.id, sel))
}

fn empty(ui: &mut egui::Ui, message: &str) {
    theme::empty_message(ui, message);
}

fn drag_f32(ui: &mut egui::Ui, label: &str, value: &mut f32, speed: f32, dirty: &mut bool) {
    egui::Grid::new(format!("dv_{label}_grid"))
        .num_columns(2)
        .show(ui, |ui| {
            ui.label(label);
            if drag_value(ui, value, |d| d.speed(speed as f64)).inner {
                *dirty = true;
            }
            ui.end_row();
        });
}

fn drag_u8(ui: &mut egui::Ui, label: &str, value: &mut u8, max: u8, dirty: &mut bool) {
    egui::Grid::new(format!("dv_{label}_grid"))
        .num_columns(2)
        .show(ui, |ui| {
            ui.label(label);
            if drag_value(ui, value, |d| d.range(0..=max).speed(1)).inner {
                *dirty = true;
            }
            ui.end_row();
        });
}

fn drag_u32(ui: &mut egui::Ui, label: &str, value: &mut u32, dirty: &mut bool) {
    egui::Grid::new(format!("dv_{label}_grid"))
        .num_columns(2)
        .show(ui, |ui| {
            ui.label(label);
            if drag_value(ui, value, |d| d.speed(1)).inner {
                *dirty = true;
            }
            ui.end_row();
        });
}

fn drag_i32(ui: &mut egui::Ui, label: &str, value: &mut i32, dirty: &mut bool) {
    egui::Grid::new(format!("dv_{label}_grid"))
        .num_columns(2)
        .show(ui, |ui| {
            ui.label(label);
            if drag_value(ui, value, |d| d.speed(1)).inner {
                *dirty = true;
            }
            ui.end_row();
        });
}

fn check_bool(ui: &mut egui::Ui, label: &str, value: &mut bool, dirty: &mut bool) {
    if ui.checkbox(value, label).changed() {
        *dirty = true;
    }
}

fn check_bool8(ui: &mut egui::Ui, label: &str, value: &mut or_io::decl::Bool8, dirty: &mut bool) {
    let mut b = value.get();
    if ui.checkbox(&mut b, label).changed() {
        *value = or_io::decl::Bool8::new(b);
        *dirty = true;
    }
}

fn drag_vec3(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut or_particle::domain::Vec3,
    dirty: &mut bool,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        if drag_value(ui, &mut value.x, |d| d.speed(0.01).prefix("x ")).inner {
            *dirty = true;
        }
        if drag_value(ui, &mut value.y, |d| d.speed(0.01).prefix("y ")).inner {
            *dirty = true;
        }
        if drag_value(ui, &mut value.z, |d| d.speed(0.01).prefix("z ")).inner {
            *dirty = true;
        }
    });
}

fn grid<R>(ui: &mut egui::Ui, id: &str, content: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let mut value: Option<R> = None;
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([8.0, 4.0])
        .striped(true)
        .show(ui, |ui| {
            value = Some(content(ui));
        });
    value.expect("grid closure always runs once")
}

fn row(ui: &mut egui::Ui, label: &str, value: &str) {
    row_with_note(ui, label, value, None);
}

fn row_with_note(ui: &mut egui::Ui, label: &str, value: &str, note: Option<&str>) {
    if !filter_matches(ui.ctx(), label) {
        return;
    }
    let label_widget = ui.label(
        egui::RichText::new(label)
            .small()
            .color(theme::TEXT_SECONDARY),
    );
    if let Some(note) = note {
        label_widget.on_hover_text(note);
    }
    let value_label = ui
        .add(egui::Label::new(egui::RichText::new(value).monospace()).sense(egui::Sense::click()))
        .on_hover_text("Click to copy value");
    if value_label.clicked() {
        ui.ctx().copy_text(value.to_string());
    }
    ui.end_row();
}

/// Usage classification, conversion state and streaming, as rows in an open grid.
///
/// ⚠ The first two read **one byte**: the resource type is the low two
/// bits of `resource_type` and the conversion flags are the other six, so a
/// caller that renders the byte itself shows neither.
fn usage_rows(ui: &mut egui::Ui, resource_type: u8, cut_mip_levels: u8) {
    const USAGE: [&str; 4] = ["Normal", "RenderTarget", "Reference", "DictionaryReference"];
    row_with_note(
        ui,
        "Usage",
        USAGE[usize::from(resource_type & 0x03)],
        Some("What kind of texture object the header declares this to be."),
    );

    let conversion = conversion_flags(resource_type);
    row_with_note(
        ui,
        "Conversion",
        if conversion.is_empty() {
            "processed"
        } else {
            &conversion
        },
        Some("How the texture pipeline treated this texture when the file was built."),
    );

    row_with_note(
        ui,
        "Mips cut",
        &cut_mip_levels.to_string(),
        Some("Mip levels dropped when the texture was built, for streaming."),
    );
}

/// Empty for processed alone, `SKIPPED`, `FAILED`, `BADMETA`, or one letter per set flag in `[PSFION]` order.
fn conversion_flags(flags: u8) -> String {
    match flags {
        0x80 => String::new(),
        0x40 => "SKIPPED".to_string(),
        0x20 => "FAILED".to_string(),
        0x90 => "BADMETA".to_string(),
        _ => {
            let letters: String = [
                (0x80, 'P'),
                (0x40, 'S'),
                (0x20, 'F'),
                (0x10, 'I'),
                (0x08, 'O'),
                (0x60, 'N'),
            ]
            .into_iter()
            .filter(|(bits, _)| flags & bits != 0)
            .map(|(_, letter)| letter)
            .collect();
            format!("[{letters}]")
        }
    }
}

fn pack_root_summary(
    _state: &AppState,
    doc: &crate::state::Document,
) -> Option<or_editor::Summary> {
    let (_, root) = crate::state::described_root(doc)?;
    summary_at(doc, &root, 0)
}

/// Whether "Show internals" is on, as the user last left it.
pub(crate) fn shows_internals(state: &AppState) -> bool {
    state.prefs.show_internals
}

/// `doc_id`'s summary, then `extra`, then its declared structure where one is described.
fn draw_root_summary(
    ui: &mut egui::Ui,
    state: &mut AppState,
    doc_id: DocumentId,
    id: &str,
    extra: Facts,
) {
    let structure = {
        let Some(doc) = state.document(doc_id) else {
            return;
        };
        draw_summary(ui, doc, extra);
        pack_root_summary(state, doc)
    };
    if let Some(structure) = structure {
        ui.add_space(theme::SPACING_DEFAULT);
        draw_editable_summary(ui, id, state, doc_id, &structure);
    }
}

const FILTER_KEY: &str = "inspector_property_filter";

fn write_filter_to_ctx(ctx: &egui::Context, filter: &str) {
    if filter.is_empty() {
        ctx.data_mut(|d| {
            d.remove::<String>(egui::Id::new(FILTER_KEY));
        });
    } else {
        ctx.data_mut(|d| {
            d.insert_temp(egui::Id::new(FILTER_KEY), filter.to_ascii_lowercase());
        });
    }
}

fn filter_matches(ctx: &egui::Context, label: &str) -> bool {
    let filter = ctx.data(|d| d.get_temp::<String>(egui::Id::new(FILTER_KEY)));
    match filter {
        None => true,
        Some(f) if f.is_empty() => true,
        Some(f) => label.to_ascii_lowercase().contains(&f),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_with(payload: DocPayload) -> crate::state::Document {
        crate::state::Document::new(0, None, payload)
    }

    #[test]
    fn pin_holds_a_gta_vi_animation_whole_and_any_other_document_only_by_its_selection() {
        use or_io::gta6::anim::{AnimResource, RootKind};
        let animation = AnimResource {
            root: RootKind::ClipDictionary,
            root_word: 2,
            name: "dict_a".to_string(),
            key: 0,
            block_map: None,
            word_at_60: 0,
            word_at_98: None,
            sets: Vec::new(),
            external_sets: Vec::new(),
            animation_keys: Vec::new(),
            clips: Vec::new(),
            motions: Vec::new(),
            matchers: Vec::new(),
            dictionaries: Vec::new(),
            end_region: None,
            sections: None,
            edited: false,
        };
        let mut state = AppState::new();
        state.documents = vec![
            doc_with(DocPayload::Gta6Anim(std::sync::Arc::new(animation))),
            crate::state::Document::new(
                1,
                None,
                DocPayload::Resource(or_io::Asset::Drawable(Default::default())),
            ),
        ];
        state.active_document = Some(0);
        assert_eq!(pin_target(&state), Some((0, DOC_ROOT_SEL)));
        state.active_document = Some(1);
        assert_eq!(
            pin_target(&state),
            None,
            "nothing is selected in the drawable"
        );
    }

    #[test]
    fn a_conversion_byte_reads_as_the_engine_spells_it() {
        for (flags, spelled) in [
            (0x80, ""),
            (0x40, "SKIPPED"),
            (0x20, "FAILED"),
            (0x90, "BADMETA"),
            (0x88, "[PO]"),
            (0xC0, "[PSN]"),
            (0x00, "[]"),
        ] {
            assert_eq!(conversion_flags(flags), spelled, "{flags:#04x}");
        }
    }

    #[test]
    fn an_enhanced_payload_takes_the_generic_body() {
        for payload in [
            DocPayload::Resource(or_io::Asset::DrawableGen9(Default::default())),
            DocPayload::Resource(or_io::Asset::TextureDictGen9(Default::default())),
            DocPayload::Resource(or_io::Asset::DrawableDictGen9(Default::default())),
            DocPayload::Resource(or_io::Asset::FragmentGen9(Default::default())),
        ] {
            let kind = payload.kind();
            assert!(
                body_for(&doc_with(payload)).is_none(),
                "an Enhanced {kind:?} should take the described-root body, not a Legacy one"
            );
        }
    }

    #[test]
    fn every_legacy_format_reaches_a_body_of_its_own() {
        for payload in [
            DocPayload::Resource(or_io::Asset::Particles(Default::default())),
            DocPayload::Resource(or_io::Asset::TextureDict(Default::default())),
            DocPayload::Resource(or_io::Asset::Drawable(Default::default())),
            DocPayload::Resource(or_io::Asset::DrawableDict(Default::default())),
            DocPayload::Resource(or_io::Asset::Bounds(
                or_io::gta5::PhBoundAny::PhBoundSphere(Default::default()),
            )),
            DocPayload::Resource(or_io::Asset::ClothDict(Default::default())),
            DocPayload::Gxt2(Default::default()),
        ] {
            let kind = payload.kind();
            assert!(
                body_for(&doc_with(payload)).is_some(),
                "{kind:?} lost its own inspector body"
            );
        }
    }

    #[test]
    fn an_enhanced_body_is_never_confused_with_its_legacy_twin() {
        assert!(
            body_for(&doc_with(DocPayload::Resource(or_io::Asset::Drawable(
                Default::default()
            ))))
            .is_some()
        );
        assert!(
            body_for(&doc_with(DocPayload::Resource(or_io::Asset::DrawableGen9(
                Default::default()
            ))))
            .is_none()
        );
    }

    #[test]
    fn every_declared_format_and_target_gets_a_described_outline_and_summary() {
        let state = AppState::default();
        let mut checked = 0usize;
        for format in or_io::gta5::formats::FORMATS {
            let resource = or_io::container::resource::ResourceType::from_extension(format.ext)
                .unwrap_or_else(|| panic!(".{} has no resource type", format.ext));
            for target in format.targets() {
                let version = resource
                    .read_versions()
                    .into_iter()
                    .find(|v| format.target_of_version(*v).ok() == Some(target))
                    .unwrap_or_else(|| {
                        panic!("no read version of .{} names {}", format.ext, target.slug())
                    });
                let doc = crate::ui::panels::hierarchy::described_tests::zeroed_document(
                    format.ext, version,
                );
                assert!(
                    crate::ui::panels::hierarchy::described_outline(&doc).is_some(),
                    ".{} for {} has no described outline",
                    format.ext,
                    target.slug()
                );
                assert!(
                    pack_root_summary(&state, &doc).is_some(),
                    ".{} for {} has no described summary",
                    format.ext,
                    target.slug()
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "no declared format was checked");
    }

    #[test]
    fn the_paged_kinds_with_a_panel_of_their_own_are_the_declared_ones() {
        let declared: Vec<DocumentKind> = crate::state::kinds::all()
            .map(|info| info.kind)
            .filter(|kind| !matches!(kind, DocumentKind::Registered(_)))
            .filter(|kind| or_io::gta5::formats::format(kind.extension()).is_some())
            .collect();
        assert_eq!(
            declared,
            vec![
                DocumentKind::Ypt,
                DocumentKind::Ytd,
                DocumentKind::Ydr,
                DocumentKind::Ydd,
                DocumentKind::Yft,
                DocumentKind::Ybn,
                DocumentKind::Yld,
                DocumentKind::Ycd,
                DocumentKind::Yed,
                DocumentKind::Yfd,
                DocumentKind::Ynd,
                DocumentKind::Ynv,
            ],
            "a named kind gains or loses its described root grid"
        );
    }

    /// egui turns a NaN into an infinity, and clamps a value past its range,
    /// with no input; neither is the user's edit.
    #[test]
    fn a_value_drawn_as_the_file_holds_it_is_not_written_back() {
        let ctx = egui::Context::default();
        let (mut nan, mut beyond, mut dirty) = (f32::NAN, 7u8, false);
        let row = or_editor::Row {
            label: "radius".into(),
            value: "NaN".into(),
            edit: Some(or_editor::Edit {
                offset: 0,
                kind: or_editor::EditKind::F32,
            }),
            held: Some(or_io::desc::ScalarValue::F32(f32::NAN)),
            ..Default::default()
        };
        let mut written = None;
        for _ in 0..2 {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                drag_f32(ui, "nan", &mut nan, 0.1, &mut dirty);
                drag_u8(ui, "beyond", &mut beyond, 3, &mut dirty);
                egui::Grid::new("row").show(ui, |ui| {
                    written = written.take().or(field_row(ui, &row, None, |_| {}));
                });
            });
        }
        assert!(
            !dirty && written.is_none(),
            "drawing reported an edit: {written:?}"
        );
        assert!(
            nan.is_nan() && beyond == 7,
            "drawing wrote {nan} and {beyond}"
        );
    }

    fn zeroed(ext: &str, payload: DocPayload) -> crate::state::Document {
        let format = or_io::gta5::formats::format(ext).expect("declared");
        let version = or_io::container::resource::ResourceType::from_extension(ext)
            .expect("a resource type")
            .read_versions()
            .into_iter()
            .find(|v| format.target_of_version(*v).is_ok())
            .expect("a declared version");
        let mut doc = crate::ui::panels::hierarchy::described_tests::zeroed_document(ext, version);
        doc.payload = payload;
        doc
    }

    #[test]
    fn every_field_of_a_drawable_says_whether_it_is_written_here() {
        let state = AppState::default();
        let drawable = zeroed(
            "ydr",
            DocPayload::Resource(or_io::Asset::Drawable(Default::default())),
        );
        let rows = pack_root_summary(&state, &drawable)
            .expect("a described drawable")
            .rows;
        let vft = rows
            .iter()
            .find(|r| r.label == "Base > Base > Vft")
            .expect("the vft word is declared");
        assert!(vft.internal && vft.edit.is_none(), "{vft:?}");
        assert!(
            rows.iter().any(|r| !r.internal && r.edit.is_some()),
            "nothing a drawable shows is written here"
        );
        let silent: Vec<&str> = (rows.iter())
            .filter(|r| r.edit.is_none() && r.fixed.is_none())
            .map(|r| r.label.as_str())
            .collect();
        assert!(silent.is_empty(), "read-only with no reason: {silent:?}");
    }
}
