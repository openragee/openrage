mod defrag;
mod dispatch;
mod drawable;
mod files;
mod files_keys;
mod files_menu;
mod files_view;
mod folder_view;
mod fxc;
mod gen9;
mod gta6_anim;
mod gta6_anim_scene;
mod gta6_bound;
mod gta6_drawable;
pub(crate) mod gta6_expressions;
pub(crate) mod gta6_filters;
mod gta6_fragment;
mod gta6_install;
mod gta6_mission;
pub(crate) mod gta6_save;
mod gta6_save_text;
mod navigation;
mod outline_panel;
pub(crate) mod pictures;
mod rows;
mod rpf;
mod rpf_tree;
mod ttpf;
mod uses;
mod ybn;
mod ycd;
mod ydd;
mod ydr;
mod yed;
mod yfd;
mod yft;
mod yld;
mod ynd;
mod ynv;
mod ypdb;
mod ypt;
mod ytd;

pub(crate) use dispatch::{children, outline};
pub(crate) use drawable::LOD_LABELS;
use drawable::*;
pub use files::draw;
pub(crate) use files_menu::{draw_properties, new_folder};
pub(crate) use files_view::{draw_files_view, go_into, FilesViewUi};
use fxc::*;
use gen9::*;
use navigation::escalate_button;
pub(crate) use navigation::go_to_file;
pub use outline_panel::draw as draw_outline;
use rpf::*;
use ttpf::*;
use ybn::*;
pub(crate) use ybn::{bound_geometry, bound_type_name};
use ycd::*;
use ydd::*;
use ydr::*;
pub(crate) use yed::short_name;
use yft::*;
use yld::*;
pub(crate) use yld::{cloth_levels, cloth_name, primary_cloth};
use ypt::*;
pub(crate) use ypt::{delete_row as delete_particle_row, deletes as particle_row_deletes};
use ytd::*;

use or_io::codec::texture::GrcTextureFormat;
use or_io::gta5::PhBoundAny;
use or_particle::domain::PtxDomainShape;

use crate::platform::file_dialogs;
use crate::state::explorer::{self, SortKey, SortOrder};
use crate::state::navigation::Location;
use crate::state::{
    capabilities, AppState, DocPayload, Document, DocumentId, DocumentKind, RpfNodeBacking,
    RpfNodeId, RpfSessionNode,
};
use crate::ui::commands::{self, Action, Command, Target};
use crate::ui::dock::Workspace;
use crate::ui::icons::{self, YdrSection, YptItem};
use crate::ui::panels::selection_ids::{
    dict_entry_id, dict_entry_part_id, is_dict_entry_selection, name_hash, pack_index,
    pack_lod_sub_index, pack_selection, root_selection_id, ypt_behaviour_id, ypt_domain_id,
    ypt_effect_id, ypt_emitter_id, ypt_emitter_lib_id, ypt_model_id, ypt_model_part_id,
    ypt_particle_lib_id, ypt_tag_of, ypt_texture_id, ypt_unpack_behaviour, ypt_unpack_domain,
    ypt_unpack_effect, ypt_unpack_emitter, ypt_unpack_model_hash, DOC_ROOT_SEL, SECT_BOUND,
    SECT_LIGHT, SECT_SHADER, SECT_SKEL, YPT_BEHAVIOUR, YPT_DOMAIN, YPT_DOMAIN_ATTRACTOR,
    YPT_DOMAIN_CREATION, YPT_DOMAIN_TARGET, YPT_EFFECT, YPT_EMITTER, YPT_EMITTER_LIB, YPT_MODEL,
    YPT_MODEL_DICT, YPT_MODEL_KIND_BONE, YPT_MODEL_KIND_BOUND, YPT_MODEL_KIND_LIGHT,
    YPT_MODEL_KIND_LOD, YPT_MODEL_KIND_SHADER, YPT_MODEL_KIND_TEXTURE, YPT_PARTICLE_LIB,
    YPT_TEXTURE, YPT_TEXTURE_DICT,
};
use crate::ui::theme;
use egui_phosphor::regular as ph;

pub(crate) fn ui_state(state: &mut AppState) -> &mut HierarchyUiState {
    state.panels.get::<HierarchyUiState>()
}

pub(crate) fn ui_state_ref(state: &AppState) -> Option<&HierarchyUiState> {
    state.panels.peek::<HierarchyUiState>()
}

#[derive(Debug, Clone, Default)]
pub struct HierarchyUiState {
    pub filter: String,
    /// Read the filter as a path glob through the VFS rather than as a
    /// substring over the rows already on screen.
    pub deep: bool,
    pub pulse_expand_all: bool,
    pub pulse_collapse_all: bool,
    pub focus_filter_next_frame: bool,
    /// Go to File is finding under the field: what the words name is listed there, with the keys of the palette, until a file is chosen or Escape leaves it.
    pub open_matches: bool,
    /// Whether the field had the keys in the frame last drawn: it was focused, or lost the focus to an Enter typed in it.
    pub filter_keys: bool,
    pub renaming: Option<RenameState>,
    pub pending_rename: Option<PendingRename>,
    pub multi_click_throttle: Option<MultiClickThrottle>,
    pub last_click: Option<LastClick>,
    pub scroll_to: Option<(DocumentId, u64)>,
    /// The row the Outline has kept `scroll_to` on, and for how many frames.
    pub outline_reveal: Option<((DocumentId, u64), u32)>,
    /// Archive rows in the order this frame drew them: what a shift-click
    /// spans, since a range is a stretch of the screen and not of ids.
    pub visible_rows: std::collections::HashMap<DocumentId, Vec<u64>>,
    /// A shift-click waiting for the frame's rows to be known.
    pub pending_range: Option<(DocumentId, u64)>,
    /// Folders to open this frame on the way down to a revealed row.
    pub reveal_ancestors: std::collections::HashSet<u64>,
    /// Frames a reveal has waited for its row to be drawn.
    pub reveal_frames: u32,
    pub filter_counts: Option<FilterCounts>,
    /// The folder a Details or Icons view shows, per archive.
    pub open_folder: std::collections::HashMap<DocumentId, u64>,
    /// Where each archive row landed this frame.
    pub row_rects: Vec<(DocumentId, u64, egui::Rect)>,
    /// The pass `row_rects` was drawn in.
    pub rows_pass: u64,
}

/// The filter readout, kept until the filter or the tree changes so a large
/// archive is not recounted every frame.
#[derive(Debug, Clone)]
pub struct FilterCounts {
    pub doc_id: DocumentId,
    pub filter: String,
    pub nodes: usize,
    pub shown: usize,
    pub total: usize,
}

/// Frames a reveal keeps trying before it gives up on a row that never drew.
///
/// ⚠ **More than one is needed**: a collapsing header forced open this frame
/// animates from closed, so its children first draw a frame or two later.
const REVEAL_PATIENCE: u32 = 30;

#[derive(Debug, Clone, Copy)]
pub struct LastClick {
    pub doc_id: DocumentId,
    pub key: u64,
    pub at: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct MultiClickThrottle {
    pub doc_id: DocumentId,
    pub key: u64,
    pub at: f64,
}

pub(crate) const MULTI_CLICK_THROTTLE: f64 = 0.6;

pub(crate) const RENAME_TRIGGER_DELAY: f64 = 0.5;

#[derive(Debug, Clone)]
pub struct PendingRename {
    pub doc_id: DocumentId,
    pub target: RpfNodeId,
    pub current_name: String,
    pub scheduled_at: f64,
}

#[derive(Debug, Clone)]
pub struct RenameState {
    pub doc_id: DocumentId,
    pub target: RpfNodeId,
    pub buffer: String,
    pub focus_pending: bool,
    /// Edited in the Files view in the centre rather than in the tree: one row
    /// drawn in both would hold two editors under one id.
    pub in_centre: bool,
}

impl crate::ui::panel_state::PanelState for HierarchyUiState {
    const KEY: &'static str = "hierarchy";
}

impl HierarchyUiState {
    pub fn start_rename_rpf(&mut self, doc_id: DocumentId, node_id: RpfNodeId, current_name: &str) {
        self.renaming = Some(RenameState {
            doc_id,
            target: node_id,
            buffer: current_name.to_string(),
            focus_pending: true,
            in_centre: false,
        });
    }

    /// Rename a row in place in the Files view in the centre.
    pub fn start_rename_in_centre(&mut self, doc_id: DocumentId, node_id: RpfNodeId, name: &str) {
        self.start_rename_rpf(doc_id, node_id, name);
        if let Some(rename) = self.renaming.as_mut() {
            rename.in_centre = true;
        }
    }

    /// Whether the tree edits `node_id`'s name now.
    pub fn is_renaming_rpf(&self, doc_id: DocumentId, node_id: RpfNodeId) -> bool {
        matches!(
            &self.renaming,
            Some(r) if r.doc_id == doc_id && r.target == node_id && !r.in_centre
        )
    }

    /// Whether the Files view in the centre edits `node_id`'s name now.
    pub fn is_renaming_in_centre(&self, doc_id: DocumentId, node_id: RpfNodeId) -> bool {
        matches!(
            &self.renaming,
            Some(r) if r.doc_id == doc_id && r.target == node_id && r.in_centre
        )
    }

    pub fn schedule_rename(
        &mut self,
        doc_id: DocumentId,
        target: RpfNodeId,
        current_name: String,
        now: f64,
    ) {
        self.pending_rename = Some(PendingRename {
            doc_id,
            target,
            current_name,
            scheduled_at: now,
        });
    }

    pub fn cancel_pending_rename(&mut self) {
        self.pending_rename = None;
    }

    pub fn allow_multi_click(&self, doc_id: DocumentId, key: u64, now: f64) -> bool {
        match self.multi_click_throttle {
            Some(t) if t.doc_id == doc_id && t.key == key => now - t.at >= MULTI_CLICK_THROTTLE,
            _ => true,
        }
    }

    pub fn note_multi_click(&mut self, doc_id: DocumentId, key: u64, now: f64) {
        self.multi_click_throttle = Some(MultiClickThrottle {
            doc_id,
            key,
            at: now,
        });
    }

    pub fn previous_click_was_on(&self, doc_id: DocumentId, key: u64, now: f64) -> bool {
        match self.last_click {
            Some(c) if c.doc_id == doc_id && c.key == key => now - c.at < MULTI_CLICK_THROTTLE,
            _ => false,
        }
    }

    pub fn note_click(&mut self, doc_id: DocumentId, key: u64, now: f64) {
        self.last_click = Some(LastClick {
            doc_id,
            key,
            at: now,
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InlineRenameAction {
    Editing,
    Commit,
    Cancel,
}

/// What a frame's document rows asked for, done once every row is drawn.
#[derive(Default)]
struct RowOutcome {
    create: Option<DocumentKind>,
    /// A header clicked, which makes its document active only where the
    /// workspace on screen shows it.
    clicked: Option<DocumentId>,
    activate: Option<DocumentId>,
    /// A command a document's menu offers.
    command: Option<Command>,
}

impl RowOutcome {
    fn apply(self, state: &mut AppState, ctx: &egui::Context) {
        if let Some(kind) = self.create {
            create_new(state, kind);
        }
        if let Some(id) = self.clicked {
            state.activate_if_shown(id);
        }
        if let Some(id) = self.activate {
            state.set_active_document(id);
        }
        if let Some(command) = self.command {
            commands::run(state, ctx, &command);
        }
    }
}

/// One open document's header, with its tree under it.
fn document_row(
    ui: &mut egui::Ui,
    state: &mut AppState,
    doc_id: DocumentId,
    outcome: &mut RowOutcome,
) {
    let (title, kind, is_active, dirty) = match state.document(doc_id) {
        Some(d) => (
            d.title(),
            d.kind,
            state.active_document == Some(d.id),
            d.dirty,
        ),
        None => return,
    };

    let header_color = if is_active {
        theme::ACCENT_TEXT
    } else {
        theme::TEXT_PRIMARY
    };
    let icon = icons::doc_kind_icon(kind, dirty);
    let icon_tooltip = icon.tooltip();
    let header_job = icons::icon_label(&icon, &title, header_color.into());
    let selected = state.selection.is_selected(doc_id, DOC_ROOT_SEL);

    let is_leaf = !doc_has_outline_children(state, doc_id);
    let header_id = ui.make_persistent_id(("hierarchy_doc", doc_id));
    let reveal_here = state.navigation.reveal.filter(|l| l.doc == doc_id);
    if reveal_here.is_some() && !is_leaf {
        force_open_collapsing(ui.ctx(), header_id);
    }

    let header_resp = if is_leaf {
        leaf_row(ui, |ui| {
            selectable_header(ui, selected, header_job).on_hover_text(icon_tooltip)
        })
    } else {
        collapsing_row(
            ui,
            header_id,
            is_active,
            |ui, _open| selectable_header(ui, selected, header_job).on_hover_text(icon_tooltip),
            |ui| draw_doc_body(ui, state, doc_id),
        )
    };

    if reveal_here.is_some_and(|l| l.node == DOC_ROOT_SEL) {
        header_resp.scroll_to_me(Some(egui::Align::Center));
        state.navigation.reveal = None;
    }
    if header_resp.clicked() {
        let now = ui.ctx().input(|i| i.time);
        let real_multi = is_multi_click(&header_resp)
            && ui_state(state).previous_click_was_on(doc_id, DOC_ROOT_SEL, now);
        if real_multi {
            if ui_state(state).allow_multi_click(doc_id, DOC_ROOT_SEL, now) {
                ui_state(state).note_multi_click(doc_id, DOC_ROOT_SEL, now);
                state.selection.select(doc_id, DOC_ROOT_SEL);
                if !is_active {
                    outcome.clicked = Some(doc_id);
                }
                if !is_leaf {
                    toggle_collapsing(ui.ctx(), header_id, is_active);
                }
            }
        } else {
            state.selection.select(doc_id, DOC_ROOT_SEL);
            if !is_active {
                outcome.clicked = Some(doc_id);
            }
        }
        state.navigation.visit(Location::document(doc_id));
        ui_state(state).note_click(doc_id, DOC_ROOT_SEL, now);
    }
    header_resp.context_menu(|ui| {
        crate::ui::popup_of_a_document(ui.ctx(), doc_id);
        new_document_menu(ui, &mut outcome.create);
        ui.separator();
        if !is_active && ui.button(tr!("Activate")).clicked() {
            outcome.activate = Some(doc_id);
            ui.close_kind(egui::UiKind::Menu);
        }
        let target = Target::Document(doc_id);
        if !matches!(kind, DocumentKind::GameRoot) {
            commands::offer(
                ui,
                state,
                Action::CloseDocument,
                &target,
                &mut outcome.command,
            );
        }
        ui.separator();
        let favourite = files_menu::favourite_action(state, Location::document(doc_id));
        commands::offer(ui, state, favourite, &target, &mut outcome.command);
        if matches!(kind, DocumentKind::Folder) {
            for action in [
                Action::RevealOnDisk,
                Action::CopyPath,
                Action::CreateDlcPack,
            ] {
                commands::offer(ui, state, action, &target, &mut outcome.command);
            }
            ui.separator();
            commands::offer(ui, state, Action::Properties, &target, &mut outcome.command);
        }
        if state.offers_gltf(crate::state::gltf::Model::Document(doc_id)) {
            ui.separator();
            for action in [Action::ExportGltf, Action::ImportGltf] {
                commands::offer(ui, state, action, &target, &mut outcome.command);
            }
        }
    });
}

fn create_new(state: &mut AppState, kind: DocumentKind) {
    let id = state.new_empty_document(kind);
    let name = state
        .document(id)
        .map(|d| d.name.clone())
        .unwrap_or_default();
    state.show_toast(tr!("Created {name}", name = name));
    state.set_active_document(id);
}

fn new_document_menu(ui: &mut egui::Ui, out: &mut Option<DocumentKind>) {
    crate::ui::commands::submenu(ui, tr!("New"), |ui| {
        new_document_items(ui, out);
    });
}

/// The create-new rows, with the ones this build cannot author disabled.
///
/// ⚠ **Disabled and explained rather than hidden.** A row that vanishes when no
/// pack is staged teaches nothing; a greyed one saying why sends the user to stage
/// a pack. Neither is an offer: `add_enabled(false, ..)` cannot be clicked.
pub(crate) fn new_document_items(ui: &mut egui::Ui, out: &mut Option<DocumentKind>) {
    for row in crate::state::kinds::creatable() {
        let kind = row.info.kind;
        let response = ui.add_enabled(row.available(), egui::Button::new(row.info.menu_label()));
        let response = match &row.unavailable {
            Some(because) => response.on_disabled_hover_text(because),
            None => response,
        };
        if response.clicked() {
            *out = Some(kind);
            ui.close_kind(egui::UiKind::Menu);
        }
    }
}

fn draw_doc_body(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId) {
    let outline = state
        .document(doc_id)
        .and_then(|doc| capabilities::for_payload(&doc.payload))
        .and_then(|row| row.outline);
    match outline {
        Some(draw) => draw(ui, state, doc_id),
        None => {
            if !gta6_anim_scene::draw_gta6_anim_scene(ui, state, doc_id)
                && !gta6_mission::draw(ui, state, doc_id)
            {
                draw_described_outline(ui, state, doc_id);
            }
        }
    }
}

fn doc_has_outline_children(state: &AppState, doc_id: DocumentId) -> bool {
    let Some(doc) = state.document(doc_id) else {
        return false;
    };
    if let Some(has) = capabilities::for_payload(&doc.payload).and_then(|r| r.has_outline_children)
    {
        return has(&doc.payload);
    }
    if crate::ui::panels::gta6_anim_scene::holds_one(doc) || gta6_mission::has_tables(&doc.payload)
    {
        return true;
    }
    described_outline(doc).is_some_and(|outline| outline.root.has_children())
}

pub(crate) fn described_outline(doc: &Document) -> Option<or_editor::Outline> {
    let image = doc.resource.as_ref()?;
    let (pack, root) = crate::state::described_root(doc)?;
    or_editor::Outline::of(&pack, &root, image.view(), 0)
}

fn draw_described_outline(ui: &mut egui::Ui, state: &mut AppState, doc_id: DocumentId) {
    let Some(outline) = state.document(doc_id).and_then(described_outline) else {
        return;
    };
    let selected = state.selection.described(doc_id).cloned();
    let root = or_editor::PackPath::default();
    let mut clicked: Option<or_editor::PackPath> = None;
    for node in &outline.root.children {
        draw_described_node(ui, node, &root, selected.as_ref(), &mut clicked);
    }
    if let Some(path) = clicked {
        state.selection.select_described(doc_id, path);
    }
}

fn draw_described_node(
    ui: &mut egui::Ui,
    node: &or_editor::Node,
    parent: &or_editor::PackPath,
    selected: Option<&or_editor::PackPath>,
    clicked: &mut Option<or_editor::PackPath>,
) {
    let path = match &node.hop {
        Some(hop) => parent.then(hop.clone()),
        None => parent.clone(),
    };
    let label = match &node.note {
        Some(note) => format!("{} — {note}", node.label),
        None => node.label.clone(),
    };

    if node.has_children() {
        let header = egui::CollapsingHeader::new(label)
            .id_salt(("described", &path))
            .show(ui, |ui| {
                for child in &node.children {
                    draw_described_node(ui, child, &path, selected, clicked);
                }
            });
        if header.header_response.clicked() {
            *clicked = Some(path);
        }
        return;
    }

    let text = egui::RichText::new(label);
    let text = if selected == Some(&path) {
        text.color(theme::ACCENT_TEXT)
    } else {
        text
    };
    if node.type_name.is_some() {
        if ui
            .add(egui::Label::new(text).sense(egui::Sense::click()))
            .clicked()
        {
            *clicked = Some(path);
        }
    } else {
        ui.label(text);
    }
}

/// The row the Outline is revealing in `doc_id`.
fn revealing(state: &AppState, doc_id: DocumentId) -> Option<u64> {
    let (doc, row) = ui_state_ref(state)?.scroll_to?;
    (doc == doc_id).then_some(row)
}

/// Open the row `target_sel` names while the Outline reveals it, or a part of the entry it is.
fn consume_scroll_to(
    state: &AppState,
    doc_id: DocumentId,
    target_sel: u64,
    ctx: &egui::Context,
    row_id: egui::Id,
) {
    let Some(k) = revealing(state, doc_id) else {
        return;
    };
    let entry = |id: u64| is_dict_entry_selection(id).then(|| ypt_unpack_model_hash(id));
    if k != target_sel && (entry(k).is_none() || entry(k) != entry(target_sel)) {
        return;
    }
    force_open_collapsing(ctx, row_id);
    ctx.request_repaint();
}

fn force_open_collapsing(ctx: &egui::Context, id: egui::Id) {
    let mut s = egui::collapsing_header::CollapsingState::load_with_default_open(ctx, id, false);
    s.set_open(true);
    s.store(ctx);
}

const REVEALED_KEY: &str = "hierarchy_reveal_reached";

fn pulse_scroll_into_view(
    state: &AppState,
    doc_id: DocumentId,
    target_sel: u64,
    resp: &egui::Response,
) {
    if revealing(state, doc_id) == Some(target_sel) {
        resp.scroll_to_me(Some(egui::Align::Center));
        resp.ctx
            .data_mut(|d| d.insert_temp(egui::Id::new(REVEALED_KEY), true));
    }
}

fn pulse_open_for_target(
    state: &AppState,
    doc_id: DocumentId,
    ctx: &egui::Context,
    id_root: egui::Id,
) {
    let Some((d, k)) = ui_state_ref(state).and_then(|s| s.scroll_to) else {
        return;
    };
    if d != doc_id {
        return;
    }
    match ypt_tag_of(k) {
        YPT_EFFECT => {
            force_open_collapsing(ctx, id_root.with("effects_section"));
            let eff_idx = ypt_unpack_effect(k);
            force_open_collapsing(ctx, id_root.with(("effect", eff_idx)));
        }
        YPT_EMITTER => {
            let (eff_idx, _em_idx) = ypt_unpack_emitter(k);
            force_open_collapsing(ctx, id_root.with("effects_section"));
            force_open_collapsing(ctx, id_root.with(("effect", eff_idx)));
        }
        YPT_DOMAIN => {
            let (eff_idx, em_idx, _kind) = ypt_unpack_domain(k);
            force_open_collapsing(ctx, id_root.with("effects_section"));
            let effect_id = id_root.with(("effect", eff_idx));
            force_open_collapsing(ctx, effect_id);
            force_open_collapsing(ctx, effect_id.with(("emitter", em_idx)));
        }
        YPT_EMITTER_LIB => {
            force_open_collapsing(ctx, id_root.with("emitter_rules_section"));
        }
        YPT_PARTICLE_LIB => {
            force_open_collapsing(ctx, id_root.with("particle_rules_section"));
        }
        YPT_BEHAVIOUR => {
            force_open_collapsing(ctx, id_root.with("particle_rules_section"));
            let (rule_idx, _b_idx) = ypt_unpack_behaviour(k);
            force_open_collapsing(ctx, id_root.with(("part_rule", rule_idx)));
        }
        YPT_TEXTURE => {
            force_open_collapsing(ctx, id_root.with("textures_section"));
        }
        YPT_MODEL => {
            force_open_collapsing(ctx, id_root.with("models_section"));
        }
        _ => {}
    }
    ctx.request_repaint();
}

fn handle_select_click(
    state: &mut AppState,
    ctx: &egui::Context,
    resp: &egui::Response,
    doc_id: DocumentId,
    sel: u64,
    row_id: egui::Id,
    default_open: bool,
) {
    if !resp.clicked() {
        return;
    }
    let modifiers = ctx.input(|i| i.modifiers);
    let multi_pin_modifier = modifiers.command || modifiers.ctrl;
    if multi_pin_modifier {
        use crate::ui::panels::selection_ids::{ypt_tag_of, ypt_unpack_effect, YPT_EFFECT};
        if ypt_tag_of(sel) == YPT_EFFECT {
            let key = crate::state::sim::SimBuildKey {
                doc_id,
                target: crate::state::sim::SerialisableTarget::Effect(ypt_unpack_effect(sel)),
            };
            state.sim.toggle_pin(key);
            return;
        }
    }
    let now = ctx.input(|i| i.time);
    let real_multi =
        is_multi_click(resp) && ui_state(state).previous_click_was_on(doc_id, sel, now);
    if real_multi {
        if ui_state(state).allow_multi_click(doc_id, sel, now) {
            ui_state(state).note_multi_click(doc_id, sel, now);
            state.selection.select(doc_id, sel);
            toggle_collapsing(ctx, row_id, default_open);
        }
    } else {
        state.selection.select(doc_id, sel);
    }
    ui_state(state).note_click(doc_id, sel, now);
}

fn empty_centered(ui: &mut egui::Ui, message: &str) {
    empty_centered_with_button(ui, message, None);
}

/// [`empty_centered`] with a button under the message; whether it was clicked.
fn empty_centered_with_button(ui: &mut egui::Ui, message: &str, button: Option<&str>) -> bool {
    let glyph = empty_state_glyph(message);
    ui.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(theme::SPACING_DEFAULT);
            ui.label(
                egui::RichText::new(glyph)
                    .size(32.0)
                    .color(theme::TEXT_DISABLED),
            );
            ui.add_space(theme::SPACING_DEFAULT);
            ui.label(egui::RichText::new(message).color(theme::TEXT_SECONDARY));
            button.is_some_and(|text| ui.button(text).clicked())
        })
        .inner
    })
    .inner
}

fn empty_state_glyph(message: &str) -> &'static str {
    let lower = message.to_ascii_lowercase();
    if lower.contains("nothing open") {
        ph::FOLDER_OPEN
    } else if lower.contains("no effects") {
        ph::LIGHTNING
    } else if lower.contains("no textures") {
        ph::IMAGE_SQUARE
    } else if lower.contains("archive") {
        ph::ARCHIVE
    } else {
        ph::FILE_DASHED
    }
}

fn collapsing_row<HeaderRet, BodyRet>(
    ui: &mut egui::Ui,
    id: egui::Id,
    default_open: bool,
    add_header: impl FnOnce(&mut egui::Ui, bool) -> HeaderRet,
    add_body: impl FnOnce(&mut egui::Ui) -> BodyRet,
) -> HeaderRet {
    if let Some(force_open) = pulse_state_from_ctx(ui.ctx()) {
        let mut s = egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            id,
            default_open,
        );
        s.set_open(force_open);
        s.store(ui.ctx());
    }
    let state = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        id,
        default_open,
    );
    let is_open = state.is_open();
    let (toggle, header_inner, _body_inner) = state
        .show_header(ui, |ui| add_header(ui, is_open))
        .body(add_body);
    crate::ui::accessible_name::named(
        toggle,
        if is_open {
            tr!("Collapse")
        } else {
            tr!("Expand")
        },
    );
    header_inner.inner
}

fn leaf_row<HeaderRet>(
    ui: &mut egui::Ui,
    add_header: impl FnOnce(&mut egui::Ui) -> HeaderRet,
) -> HeaderRet {
    ui.horizontal(|ui| {
        ui.add_space(ui.spacing().indent);
        add_header(ui)
    })
    .inner
}

const PULSE_KEY: &str = "hierarchy_pulse_force_open";

fn pulse_state_from_ctx(ctx: &egui::Context) -> Option<bool> {
    ctx.data(|d| d.get_temp::<bool>(egui::Id::new(PULSE_KEY)))
}

fn is_multi_click(resp: &egui::Response) -> bool {
    resp.double_clicked() || resp.triple_clicked()
}

fn toggle_collapsing(ctx: &egui::Context, id: egui::Id, default_open: bool) {
    let mut s =
        egui::collapsing_header::CollapsingState::load_with_default_open(ctx, id, default_open);
    let new_open = !s.is_open();
    s.set_open(new_open);
    s.store(ctx);
}

fn promote_pending_rename(ctx: &egui::Context, state: &mut AppState) {
    let now = ctx.input(|i| i.time);
    let ui = ui_state(state);
    let due = |p: &mut PendingRename| now - p.scheduled_at >= RENAME_TRIGGER_DELAY;
    if let Some(pending) = ui.pending_rename.take_if(due) {
        ui.start_rename_rpf(pending.doc_id, pending.target, &pending.current_name);
    } else if let Some(pending) = &ui.pending_rename {
        let remaining = (RENAME_TRIGGER_DELAY - (now - pending.scheduled_at)).max(0.01);
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(remaining));
    }
}

fn write_pulse_to_ctx(ctx: &egui::Context, value: Option<bool>) {
    ctx.data_mut(|d| match value {
        Some(v) => {
            d.insert_temp(egui::Id::new(PULSE_KEY), v);
        }
        None => {
            d.remove::<bool>(egui::Id::new(PULSE_KEY));
        }
    });
}

fn selectable_header(
    ui: &mut egui::Ui,
    selected: bool,
    text: impl Into<egui::WidgetText>,
) -> egui::Response {
    ui.selectable_label(selected, text)
}

fn inline_rename_text_edit(ui: &mut egui::Ui, rename: &mut RenameState) -> egui::Response {
    let id = egui::Id::new(("hierarchy_inline_rename", rename.doc_id, rename.target));
    let edit = egui::TextEdit::singleline(&mut rename.buffer)
        .id(id)
        .desired_width(ui.available_width().clamp(120.0, 260.0))
        .margin(egui::vec2(2.0, 0.0));
    let resp = ui.add(edit);
    if rename.focus_pending {
        resp.request_focus();
        if let Some(mut s) = egui::TextEdit::load_state(ui.ctx(), id) {
            let len = rename.buffer.chars().count();
            let range = egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(len),
            );
            s.cursor.set_char_range(Some(range));
            s.store(ui.ctx(), id);
        }
        rename.focus_pending = false;
    }
    resp
}

fn detail_label(text: impl Into<String>) -> egui::Label {
    egui::Label::new(
        egui::RichText::new(text.into())
            .small()
            .color(theme::TEXT_SECONDARY),
    )
    .selectable(false)
    .truncate()
}

#[cfg(test)]
mod scroll_to_tests {

    use super::*;

    fn run_with_pulse(target_sel: u64, query_ids: &[egui::Id]) -> Vec<bool> {
        let ctx = egui::Context::default();
        let id_root = egui::Id::new("hierarchy_ypt");
        let doc_id: DocumentId = 1;
        let mut state = AppState::default();
        ui_state(&mut state).scroll_to = Some((doc_id, target_sel));
        pulse_open_for_target(&state, doc_id, &ctx, id_root);
        query_ids
            .iter()
            .map(|id| {
                egui::collapsing_header::CollapsingState::load_with_default_open(&ctx, *id, false)
                    .is_open()
            })
            .collect()
    }

    #[test]
    fn pulse_opens_emitter_rules_section_for_lib_target() {
        let target = ypt_emitter_lib_id(3);
        let id_root = egui::Id::new("hierarchy_ypt");
        let section = id_root.with("emitter_rules_section");
        let other = id_root.with("particle_rules_section");
        let opens = run_with_pulse(target, &[section, other]);
        assert!(opens[0], "emitter_rules_section must be force-opened");
        assert!(
            !opens[1],
            "unrelated sections must not be opened by an emitter-lib pulse"
        );
    }

    #[test]
    fn pulse_opens_particle_rules_section_for_lib_target() {
        let target = ypt_particle_lib_id(7);
        let id_root = egui::Id::new("hierarchy_ypt");
        let section = id_root.with("particle_rules_section");
        let other = id_root.with("emitter_rules_section");
        let opens = run_with_pulse(target, &[section, other]);
        assert!(opens[0], "particle_rules_section must be force-opened");
        assert!(!opens[1]);
    }

    #[test]
    fn pulse_opens_particle_rule_row_and_section_for_behaviour_target() {
        let particle_idx = 4;
        let target = ypt_behaviour_id(particle_idx, 2);
        let id_root = egui::Id::new("hierarchy_ypt");
        let section = id_root.with("particle_rules_section");
        let part_row = id_root.with(("part_rule", particle_idx));
        let opens = run_with_pulse(target, &[section, part_row]);
        assert!(opens[0], "particle_rules_section must open");
        assert!(opens[1], "the parent part_rule row must also open");
    }

    #[test]
    fn pulse_opens_effect_chain_for_emitter_target() {
        let effect_idx = 5;
        let emitter_idx = 1;
        let target = ypt_emitter_id(effect_idx, emitter_idx);
        let id_root = egui::Id::new("hierarchy_ypt");
        let effects = id_root.with("effects_section");
        let effect = id_root.with(("effect", effect_idx));
        let opens = run_with_pulse(target, &[effects, effect]);
        assert!(opens[0], "effects_section must open");
        assert!(opens[1], "parent effect row must open");
    }

    #[test]
    fn pulse_opens_full_chain_for_domain_target() {
        let effect_idx = 2;
        let emitter_idx = 4;
        let target = ypt_domain_id(effect_idx, emitter_idx, YPT_DOMAIN_TARGET);
        let id_root = egui::Id::new("hierarchy_ypt");
        let effects = id_root.with("effects_section");
        let effect = id_root.with(("effect", effect_idx));
        let emitter = effect.with(("emitter", emitter_idx));
        let opens = run_with_pulse(target, &[effects, effect, emitter]);
        assert!(opens[0]);
        assert!(opens[1]);
        assert!(opens[2]);
    }

    #[test]
    fn pulse_opens_textures_section_for_texture_target() {
        let target = ypt_texture_id("smoke_diffuse");
        let id_root = egui::Id::new("hierarchy_ypt");
        let section = id_root.with("textures_section");
        let opens = run_with_pulse(target, &[section]);
        assert!(opens[0]);
    }

    #[test]
    fn no_pulse_means_no_open() {
        let ctx = egui::Context::default();
        let id_root = egui::Id::new("hierarchy_ypt");
        let doc_id: DocumentId = 1;
        let mut state = AppState::default();
        assert!(ui_state(&mut state).scroll_to.is_none());
        pulse_open_for_target(&state, doc_id, &ctx, id_root);
        let section = id_root.with("emitter_rules_section");
        assert!(
            !egui::collapsing_header::CollapsingState::load_with_default_open(&ctx, section, false)
                .is_open(),
            "no pulse must leave collapsibles untouched"
        );
    }

    #[test]
    fn pulse_for_other_document_is_ignored() {
        let ctx = egui::Context::default();
        let id_root = egui::Id::new("hierarchy_ypt");
        let active_doc: DocumentId = 1;
        let other_doc: DocumentId = 2;
        let mut state = AppState::default();
        ui_state(&mut state).scroll_to = Some((other_doc, ypt_emitter_lib_id(0)));
        pulse_open_for_target(&state, active_doc, &ctx, id_root);
        let section = id_root.with("emitter_rules_section");
        assert!(
            !egui::collapsing_header::CollapsingState::load_with_default_open(&ctx, section, false)
                .is_open(),
            "cross-document pulse must not open the active doc's sections"
        );
    }
}

#[cfg(test)]
mod pending_rename_tests {
    use super::*;

    /// The frame at `time` that opens a rename waiting out the pause after a second click.
    fn promote_at(ctx: &egui::Context, state: &mut AppState, time: f64) {
        let input = egui::RawInput {
            time: Some(time),
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| promote_pending_rename(ui.ctx(), state));
    }

    #[test]
    fn a_second_clicks_rename_opens_once_the_pause_has_passed() {
        let ctx = egui::Context::default();
        let mut state = AppState::default();
        ui_state(&mut state).schedule_rename(1, 7, "notes.txt".into(), 10.0);
        promote_at(&ctx, &mut state, 10.0 + RENAME_TRIGGER_DELAY / 2.0);
        assert!(
            ui_state(&mut state).renaming.is_none(),
            "the name opened before the pause passed"
        );
        promote_at(&ctx, &mut state, 10.0 + RENAME_TRIGGER_DELAY);
        assert!(
            ui_state(&mut state).is_renaming_rpf(1, 7),
            "the name never opened"
        );
        assert!(ui_state(&mut state).pending_rename.is_none());
    }
}

#[cfg(test)]
mod new_menu_tests {
    use super::*;

    #[test]
    fn the_new_menu_names_each_kind_in_words() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut chosen = None;
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            new_document_items(ui, &mut chosen);
        });
        let labels: Vec<String> = output
            .platform_output
            .accesskit_update
            .map(|update| {
                update
                    .nodes
                    .iter()
                    .filter_map(|(_, node)| node.label().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        for expected in ["Particle effects (.ypt)", "Archive (.rpf)"] {
            assert!(
                labels.iter().any(|label| label == expected),
                "the New menu offers no `{expected}`: {labels:?}"
            );
        }
        for label in &labels {
            let name = label.split(" (").next().unwrap_or_default();
            assert!(
                name.starts_with(char::is_uppercase) && name.contains(char::is_lowercase),
                "`{label}` names a kind by its code, not in words"
            );
        }
    }
}

#[cfg(test)]
pub(crate) mod described_tests {
    use std::sync::Arc;

    use super::*;
    use crate::state::ResourceImage;

    /// A document of `ext` over a zeroed image under an RSC7 header at `version`.
    pub(crate) fn zeroed_document(ext: &str, version: u32) -> Document {
        let kind = crate::state::DocumentKind::from_path(std::path::Path::new(&format!("x.{ext}")))
            .unwrap_or_else(|| panic!(".{ext} names no kind"));
        let resource = kind.resource_type().expect("a resource kind");
        let payload = crate::state::DocPayload::Registered {
            format: crate::state::format_handlers::FormatId::new(resource, or_io::Generation::Gen8),
            data: Arc::new(()),
        };
        let mut doc = Document::new(1, None, payload);
        doc.kind = kind;
        let virtual_size = 0x4000;
        doc.resource = Some(Arc::new(ResourceImage {
            data: vec![0; virtual_size * 2],
            virtual_size,
            seal: or_io::container::resource::Rsc7::invent_single_chunk_seal(version, virtual_size)
                .expect("a seal"),
        }));
        doc
    }

    #[test]
    fn a_registered_resource_gets_a_described_outline_without_a_pack() {
        for ext in ["ynd", "ybd", "ynv"] {
            let format = or_io::gta5::formats::format(ext).expect("declared");
            let version = or_io::container::resource::ResourceType::from_extension(ext)
                .expect("a resource type")
                .read_versions()
                .into_iter()
                .find(|v| format.target_of_version(*v).is_ok())
                .expect("a declared version");
            let doc = zeroed_document(ext, version);
            assert!(
                described_outline(&doc).is_some(),
                ".{ext} at version {version} has a built-in description and no outline"
            );
        }
    }

    fn painted_text(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => out.push(text.galley.text().to_string()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| painted_text(s, out)),
            _ => {}
        }
    }

    #[test]
    fn a_registered_resource_draws_its_described_outline_in_the_hierarchy() {
        let declared: Vec<&str> = DocumentKind::registered()
            .into_iter()
            .map(|kind| kind.extension())
            .filter(|ext| or_io::gta5::formats::format(ext).is_some())
            .collect();
        assert!(!declared.is_empty(), "no registered resource is declared");
        for ext in declared {
            let format = or_io::gta5::formats::format(ext).expect("declared");
            let version = or_io::container::resource::ResourceType::from_extension(ext)
                .expect("a resource type")
                .read_versions()
                .into_iter()
                .find(|v| format.target_of_version(*v).is_ok())
                .expect("a declared version");
            let doc = zeroed_document(ext, version);
            assert!(
                matches!(doc.kind, crate::state::DocumentKind::Registered(_)),
                ".{ext} is a registered kind"
            );
            let first = described_outline(&doc)
                .and_then(|outline| outline.root.children.first().map(|n| n.label.clone()))
                .unwrap_or_else(|| panic!(".{ext} has no described child"));
            let doc_id = doc.id;
            let mut state = AppState::default();
            state.documents.push(doc);

            let ctx = egui::Context::default();
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                draw_doc_body(ui, &mut state, doc_id);
            });
            let mut texts = Vec::new();
            for clipped in &output.shapes {
                painted_text(&clipped.shape, &mut texts);
            }
            assert!(
                texts.iter().any(|t| t.starts_with(&first)),
                ".{ext}: the hierarchy body drew {texts:?}, not the described child {first:?}"
            );
        }
    }
}
