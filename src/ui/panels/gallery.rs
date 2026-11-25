use or_io::codec::texture::ChannelView;

use crate::render::RenderBridge;
use crate::state::{
    models, textures, AppState, ClothingVariant, DocPayload, DocumentId, DocumentModel,
    DocumentTexture, ModelListing, RpfNodeId, TextureListing, TextureSource, Tile,
};
use crate::ui::commands::{self, Action, Target};
use crate::ui::dock::Workspace;
use crate::ui::panels::hierarchy::pictures::{picture, Shown};
use crate::ui::panels::inspector::plural;
use crate::ui::theme;

const DEFAULT_CELL_SIZE: f32 = 128.0;
const MIN_CELL_SIZE: f32 = 64.0;
const MAX_CELL_SIZE: f32 = 256.0;

const CAPTION_HEIGHT: f32 = 32.0;

const CELL_GAP: f32 = 8.0;

/// The longest side a texture tile's picture is decoded to: the largest tile, whatever size the tiles are, so the slider decodes none again.
const TILE_PICTURE_SIDE: u32 = MAX_CELL_SIZE as u32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum GalleryMode {
    Textures,
    Models,
}

/// The texture tiles the gallery drew, so that an image held over one or dropped on it finds its texture.
#[derive(Default)]
pub struct Tiles {
    /// ⚠ The pass the tiles were drawn in: a gallery that is not on screen leaves an earlier pass's tiles behind, and a drop must not land on one of those.
    pass: u64,
    drawn: Vec<(egui::Rect, DocumentId, String, TextureSource)>,
    /// The tile an image is held over, which the drop replaces.
    aimed: Option<(egui::Rect, DocumentId, String, TextureSource)>,
}

impl crate::ui::panel_state::PanelState for Tiles {
    const KEY: &'static str = "gallery-tiles";
}

/// Whether a texture tile drawn this pass is under `p`; the tile is remembered as the one a drop replaces.
pub fn aim_at_tile(ctx: &egui::Context, state: &mut AppState, p: egui::Pos2) -> bool {
    let tiles = state.panels.get::<Tiles>();
    let under = (tiles.pass == ctx.cumulative_pass_nr())
        .then(|| {
            tiles
                .drawn
                .iter()
                .find(|(rect, ..)| rect.contains(p))
                .cloned()
        })
        .flatten();
    tiles.aimed = under;
    tiles.aimed.is_some()
}

/// The rectangle of the tile an image is held over.
pub fn aimed_tile_rect(state: &mut AppState) -> Option<egui::Rect> {
    (state.panels.get::<Tiles>().aimed.as_ref()).map(|(rect, ..)| *rect)
}

pub fn draw(ui: &mut egui::Ui, state: &mut AppState, mut bridge: Option<&mut RenderBridge>) {
    // An image dropped on a tile replaces that tile's texture, as the tile's own menu does from a file the person picks.
    if let Some(image) = (state.panels)
        .get::<crate::state::os_drop::DroppedOnATile>()
        .0
        .take()
    {
        if let Some((_, doc, name, source)) = state.panels.get::<Tiles>().aimed.take() {
            super::texture_preview::replace_from_image(state, doc, source, &name, Some(image));
        }
    }
    {
        let pass = ui.ctx().cumulative_pass_nr();
        let tiles = state.panels.get::<Tiles>();
        if tiles.pass != pass {
            tiles.pass = pass;
            tiles.drawn.clear();
        }
    }
    let shows = !super::empty_state::has_nothing_to_show(state);
    let Some(doc_id) = state.active_document.filter(|_| shows) else {
        empty(
            ui,
            tr!(
                "Open a texture dictionary, a model or particle effects to browse their textures \
             and models"
            ),
        );
        return;
    };

    state.ensure_garment_dressed();
    let variants = state.garment_variants(doc_id).to_vec();
    let Some(doc) = state.document(doc_id) else {
        empty(ui, tr!("No active document"));
        return;
    };
    let selection = state.selection.selected(doc_id);
    let textures = textures::textures_in(&doc.payload, selection);
    let textures = textures::sized_for_gta6(state, doc_id, textures);
    let models = models::models_in(&doc.payload);
    // A GTA VI drawable's textures lie in other documents, where a tile's menu acts.
    let bound = matches!(doc.payload, DocPayload::Gta6Drawable(_));
    let textures = match bound {
        true => state.without_unheld_textures(doc_id, textures),
        false => textures,
    };

    let has_embedded =
        matches!(&textures, TextureListing::Textures { textures, .. } if !textures.is_empty());
    let has_variants = !variants.is_empty();
    // A garment's own dictionaries are its textures whether or not it embeds any.
    let has_textures = has_embedded || has_variants;
    let has_models = matches!(&models, ModelListing::Models(m) if !m.is_empty());

    if !has_textures && !has_models {
        empty(
            ui,
            match (&textures, &models) {
                (TextureListing::Nothing(why), _) => why,
                (_, ModelListing::Nothing(why)) => why,
                _ => tr!("Nothing to show"),
            },
        );
        return;
    }

    let view_id = ui.make_persistent_id(("gallery_view", doc_id));
    let mut view: ViewState = ui
        .data(|d| d.get_temp::<ViewState>(view_id))
        .unwrap_or_else(|| ViewState::initial(has_textures));

    if view.mode == GalleryMode::Textures && !has_textures {
        view.mode = GalleryMode::Models;
    } else if view.mode == GalleryMode::Models && !has_models {
        view.mode = GalleryMode::Textures;
    }

    let count = match view.mode {
        GalleryMode::Textures => {
            variants.len()
                + match &textures {
                    TextureListing::Textures { textures, .. } => textures.len(),
                    TextureListing::Nothing(_) => 0,
                }
        }
        GalleryMode::Models => match &models {
            ModelListing::Models(m) => m.len(),
            ModelListing::Nothing(_) => 0,
        },
    };

    egui::Panel::top(ui.make_persistent_id(("gallery_toolbar", doc_id)))
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 4)))
        .show_inside(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.add_enabled_ui(has_textures, |ui| {
                    if ui
                        .selectable_label(view.mode == GalleryMode::Textures, tr!("Textures"))
                        .clicked()
                    {
                        view.mode = GalleryMode::Textures;
                    }
                });
                ui.add_enabled_ui(has_models, |ui| {
                    if ui
                        .selectable_label(view.mode == GalleryMode::Models, tr!("Models"))
                        .clicked()
                    {
                        view.mode = GalleryMode::Models;
                    }
                });
                ui.separator();
                ui.label(
                    egui::RichText::new(match view.mode {
                        GalleryMode::Textures => plural(count, "texture"),
                        GalleryMode::Models => plural(count, "model"),
                    })
                    .small()
                    .color(theme::TEXT_SECONDARY),
                );
                ui.separator();
                ui.label(egui::RichText::new(tr!("Size")).small());
                ui.add(
                    egui::Slider::new(&mut view.cell_size, MIN_CELL_SIZE..=MAX_CELL_SIZE)
                        .show_value(false),
                );
                if view.mode == GalleryMode::Textures {
                    ui.separator();
                    ui.checkbox(&mut view.show_alpha_checker, "α")
                        .on_hover_text(tr!("Show alpha checkerboard behind transparent textures"));
                }
            });
        });

    let mut interaction = TileInteraction::default();

    egui::CentralPanel::default()
        .frame(
            egui::Frame::NONE
                .fill(theme::BG_L0.get())
                .inner_margin(egui::Margin::same(8)),
        )
        .show_inside(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let avail_width = ui.available_width().max(view.cell_size + CELL_GAP);
                    let tile_size = egui::vec2(view.cell_size, view.cell_size + CAPTION_HEIGHT);
                    let stride = tile_size.x + CELL_GAP;
                    let per_row = ((avail_width + CELL_GAP) / stride).floor().max(1.0) as usize;

                    let prev_spacing = ui.spacing().item_spacing;
                    ui.spacing_mut().item_spacing = egui::vec2(CELL_GAP, CELL_GAP);

                    match view.mode {
                        GalleryMode::Textures => {
                            if let TextureListing::Textures {
                                textures, source, ..
                            } = &textures
                            {
                                if has_variants {
                                    section_header(
                                        ui,
                                        tr!("Embedded in this model"),
                                        textures.len(),
                                    );
                                }
                                let stride = tile_size.y + CELL_GAP;
                                let rows = textures.len().div_ceil(per_row);
                                let (first, end) = rows_in_view(ui, rows, stride);
                                ui.add_space(first as f32 * stride);
                                let shown = textures.chunks(per_row).skip(first).take(end - first);
                                for chunk in shown {
                                    ui.horizontal(|ui| {
                                        for tex in chunk {
                                            let id = tex.selection_id;
                                            let response = draw_tile(
                                                ui,
                                                doc_id,
                                                tex,
                                                *source,
                                                &view,
                                                selection == Some(id),
                                                tile_size,
                                                state,
                                            );
                                            (state.panels.get::<Tiles>().drawn).push((
                                                response.rect,
                                                doc_id,
                                                tex.name.clone(),
                                                *source,
                                            ));
                                            if response.double_clicked() {
                                                interaction.open_in_preview = Some(id);
                                            } else if response.clicked()
                                                || response.secondary_clicked()
                                            {
                                                // A right-click selects the tile its menu is about.
                                                interaction.select = Some(id);
                                            }
                                            response.context_menu(|ui| {
                                                let asked = &mut interaction.command;
                                                // A GTA VI model's texture is in another document, which its commands name.
                                                if bound {
                                                    let name = tex.name.clone();
                                                    let on = Target::Texture { doc: doc_id, name };
                                                    let replace = Action::ReplaceTextureFromImage;
                                                    commands::offer(ui, state, replace, &on, asked);
                                                    let export = Action::ExportTexture;
                                                    commands::offer(ui, state, export, &on, asked);
                                                    return;
                                                }
                                                // What the menu offers acts on the tile selected, which the right-click that opened it made this one.
                                                for action in [
                                                    Action::ReplaceTextureFromImage,
                                                    Action::ExportTexture,
                                                ] {
                                                    commands::offer_plain(ui, state, action, asked);
                                                }
                                            });
                                        }
                                    });
                                }
                                ui.add_space((rows - end) as f32 * stride);
                            }
                            if let Some(parent) = state
                                .document(doc_id)
                                .and_then(|d| d.rpf_entry_binding)
                                .map(|b| b.parent_doc_id)
                                .filter(|_| has_variants)
                            {
                                if has_embedded {
                                    ui.add_space(CELL_GAP);
                                }
                                section_header(ui, tr!("Clothing textures"), variants.len());
                                let chosen = state.garment_variant(doc_id);
                                for chunk in variants.chunks(per_row) {
                                    ui.horizontal(|ui| {
                                        for variant in chunk {
                                            let response = draw_variant_tile(
                                                ui,
                                                state,
                                                parent,
                                                variant,
                                                &view,
                                                chosen == Some(variant.node),
                                                tile_size,
                                            );
                                            if response.clicked() {
                                                interaction.dress = Some(variant.node);
                                            }
                                        }
                                    });
                                }
                            }
                        }
                        GalleryMode::Models => {
                            let ModelListing::Models(models) = &models else {
                                return;
                            };
                            let stride = tile_size.y + CELL_GAP;
                            let rows = models.len().div_ceil(per_row);
                            let (first, end) = rows_in_view(ui, rows, stride);
                            ui.add_space(first as f32 * stride);
                            for chunk in models.chunks(per_row).skip(first).take(end - first) {
                                ui.horizontal(|ui| {
                                    for model in chunk {
                                        let id = model.selection_id;
                                        let response = draw_model_tile(
                                            ui,
                                            doc_id,
                                            model,
                                            &view,
                                            selection == Some(id),
                                            tile_size,
                                            state,
                                            bridge.as_deref_mut(),
                                        );
                                        if response.double_clicked() {
                                            interaction.open_in_geometry = Some(id);
                                        } else if response.clicked() {
                                            interaction.select = Some(id);
                                        }
                                    }
                                });
                            }
                            ui.add_space((rows - end) as f32 * stride);
                        }
                    }

                    ui.spacing_mut().item_spacing = prev_spacing;
                });
        });

    if let Some(id) = interaction.select {
        state.selection.select(doc_id, id);
    }
    if let Some(id) = interaction.open_in_preview {
        state.selection.select(doc_id, id);
        state.active_workspace = Workspace::Texture;
    }
    if let Some(id) = interaction.open_in_geometry {
        state.selection.select(doc_id, id);
        state.active_workspace = Workspace::Model;
    }
    if let Some(node) = interaction.dress {
        if let Err(why) = state.choose_garment_variant(doc_id, node) {
            state.show_error_toast(why);
        }
    }
    if let Some(command) = interaction.command {
        commands::run(state, ui.ctx(), &command);
    }

    ui.data_mut(|d| d.insert_temp(view_id, view));
}

#[derive(Default)]
struct TileInteraction {
    select: Option<u64>,
    open_in_preview: Option<u64>,
    open_in_geometry: Option<u64>,
    dress: Option<RpfNodeId>,
    /// The command a tile's menu asked for, run once the tiles are drawn.
    command: Option<commands::Command>,
}

/// The rows in view, first and one past the last, of `rows` each `stride` tall from the cursor down.
///
/// ⚠ Rows out of view take their space and are not laid out: a texture pack lists ten thousand.
fn rows_in_view(ui: &egui::Ui, rows: usize, stride: f32) -> (usize, usize) {
    let top = ui.cursor().top();
    let clip = ui.clip_rect();
    let at = |y: f32| (((y - top) / stride).max(0.0) as usize).min(rows);
    let end = (at(clip.bottom()) + 1).min(rows);
    (at(clip.top()).min(end), end)
}

/// A tile's tooltip, whose words are made only while the pointer rests on the tile.
fn hover(response: egui::Response, text: impl FnOnce() -> String) -> egui::Response {
    response.on_hover_ui(|ui| {
        ui.set_max_width(ui.spacing().tooltip_width);
        ui.label(text());
    })
}

/// A group's title and its count over a rule, so groups of tiles read as apart.
fn section_header(ui: &mut egui::Ui, title: &str, count: usize) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).strong());
        ui.label(egui::RichText::new(count.to_string()).color(theme::TEXT_SECONDARY));
    });
    ui.separator();
}

/// The longest side a variant's picture is made for, whatever size the tiles are, so the slider asks for none again.
const VARIANT_PICTURE_SIDE: u32 = 256;

/// A garment's texture variant: its dictionary's first texture, read off the UI thread, that clicking dresses the garment with.
fn draw_variant_tile(
    ui: &mut egui::Ui,
    state: &mut AppState,
    parent: DocumentId,
    variant: &ClothingVariant,
    view: &ViewState,
    is_selected: bool,
    size: egui::Vec2,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());

    let img_rect = egui::Rect::from_min_size(rect.min, egui::vec2(size.x, view.cell_size));
    let caption_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x, img_rect.max.y),
        egui::pos2(rect.max.x, rect.max.y),
    );

    if !ui.is_rect_visible(rect) {
        return response;
    }

    if view.show_alpha_checker {
        paint_alpha_checker(ui.painter(), img_rect);
    } else {
        ui.painter()
            .rect_filled(img_rect, egui::CornerRadius::ZERO, theme::BG_L1);
    }

    let ctx = ui.ctx().clone();
    match picture(
        &ctx,
        state,
        None,
        parent,
        variant.node,
        VARIANT_PICTURE_SIDE,
    ) {
        Shown::Ready { id, size: pixels } => {
            let scale = (img_rect.width() / pixels.x).min(img_rect.height() / pixels.y);
            let painted = egui::Rect::from_center_size(img_rect.center(), pixels * scale);
            egui::Image::new((id, pixels))
                .maintain_aspect_ratio(true)
                .paint_at(ui, painted);
        }
        shown => {
            ui.painter().text(
                img_rect.center(),
                egui::Align2::CENTER_CENTER,
                if shown == Shown::Waiting {
                    "\u{2026}"
                } else {
                    tr!("no preview")
                },
                egui::FontId::proportional(10.0),
                theme::TEXT_SECONDARY.get(),
            );
        }
    }

    if response.hovered() && !is_selected {
        let wash = theme::ACCENT.gamma_multiply(0.10);
        ui.painter()
            .rect_filled(img_rect, egui::CornerRadius::ZERO, wash);
    }
    let stroke = if is_selected {
        egui::Stroke::new(2.0_f32, theme::ACCENT)
    } else if response.hovered() {
        egui::Stroke::new(1.0_f32, theme::ACCENT_HOVER)
    } else {
        egui::Stroke::new(1.0_f32, theme::BORDER)
    };
    ui.painter().rect_stroke(
        img_rect,
        egui::CornerRadius::ZERO,
        stroke,
        egui::StrokeKind::Inside,
    );

    let caption_painter = ui.painter().with_clip_rect(caption_rect);
    let name_color = if is_selected {
        theme::ACCENT_TEXT.get()
    } else {
        ui.visuals().text_color()
    };
    let name_galley = caption_painter.layout(
        variant.label(),
        egui::FontId::proportional(11.0),
        name_color,
        caption_rect.width() - 4.0,
    );
    caption_painter.galley(
        egui::pos2(caption_rect.min.x + 2.0, caption_rect.min.y + 4.0),
        name_galley,
        name_color,
    );

    hover(response, || {
        tr!(
            "{file}\nClick: draw the garment with this texture",
            file = variant.file
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn draw_tile(
    ui: &mut egui::Ui,
    doc_id: DocumentId,
    tex: &DocumentTexture,
    source: TextureSource,
    view: &ViewState,
    is_selected: bool,
    size: egui::Vec2,
    state: &mut AppState,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let response = crate::ui::accessible_name::named(response, &tex.name);

    let img_rect = egui::Rect::from_min_size(rect.min, egui::vec2(size.x, view.cell_size));
    let caption_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x, img_rect.max.y),
        egui::pos2(rect.max.x, rect.max.y),
    );

    if !ui.is_rect_visible(rect) {
        return response;
    }

    if view.show_alpha_checker {
        paint_alpha_checker(ui.painter(), img_rect);
    } else {
        ui.painter()
            .rect_filled(img_rect, egui::CornerRadius::ZERO, theme::BG_L1);
    }

    let ctx = ui.ctx().clone();
    let picture = tile_picture(state, &ctx, doc_id, source, tex);
    if let Tile::Ready(cached) = &picture {
        let img_w = cached.width as f32;
        let img_h = cached.height as f32;
        let scale = (img_rect.width() / img_w)
            .min(img_rect.height() / img_h)
            .min(1.0);
        let drawn = egui::vec2(img_w * scale, img_h * scale);
        let painted = egui::Rect::from_center_size(img_rect.center(), drawn);
        egui::Image::new(&cached.handle)
            .maintain_aspect_ratio(true)
            .paint_at(ui, painted);
    } else {
        let text = format!("{}\u{00d7}{}", tex.width, tex.height);
        let painter = ui.painter();
        painter.text(
            img_rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(10.0),
            theme::TEXT_SECONDARY.get(),
        );
    }

    if response.hovered() && !is_selected {
        let wash = theme::ACCENT.gamma_multiply(0.10);
        ui.painter()
            .rect_filled(img_rect, egui::CornerRadius::ZERO, wash);
    }
    let stroke = if is_selected {
        egui::Stroke::new(2.0_f32, theme::ACCENT)
    } else if response.hovered() {
        egui::Stroke::new(1.0_f32, theme::ACCENT_HOVER)
    } else {
        egui::Stroke::new(1.0_f32, theme::BORDER)
    };
    ui.painter().rect_stroke(
        img_rect,
        egui::CornerRadius::ZERO,
        stroke,
        egui::StrokeKind::Inside,
    );

    let caption_painter = ui.painter().with_clip_rect(caption_rect);
    let name_color = if is_selected {
        theme::ACCENT_TEXT.get()
    } else {
        ui.visuals().text_color()
    };
    // A GTA VI pack's texture read through fitted tables is an estimate, which its tile says; one
    // with no name is captioned by the model that uses it.
    let (estimate, used) = match state.document(doc_id).map(|d| &d.payload) {
        Some(DocPayload::Ttpf(ttpf)) => {
            let index = tex.selection_id as usize;
            (ttpf.estimate_label(index), ttpf.use_label(index))
        }
        _ => (None, None),
    };
    let dim_text = match &estimate {
        Some(_) => format!("{} {}\u{00d7}{}", tr!("calibrated"), tex.width, tex.height),
        None => format!("{}\u{00d7}{}", tex.width, tex.height),
    };
    let dim_galley = caption_painter.layout_no_wrap(
        dim_text,
        egui::FontId::proportional(10.0),
        theme::TEXT_SECONDARY.get(),
    );
    // The name wraps short of the size, which sits at the bottom right under its second line.
    let mut name_job = egui::text::LayoutJob::simple(
        used.clone().unwrap_or_else(|| tex.name.clone()),
        egui::FontId::proportional(11.0),
        name_color,
        caption_rect.width() - 8.0 - dim_galley.size().x,
    );
    name_job.wrap.break_anywhere = true;
    name_job.wrap.max_rows = 2;
    caption_painter.galley(
        egui::pos2(caption_rect.min.x + 2.0, caption_rect.min.y + 2.0),
        caption_painter.layout_job(name_job),
        name_color,
    );
    caption_painter.galley(
        caption_rect.max - dim_galley.size() - egui::vec2(2.0, 2.0),
        dim_galley,
        theme::TEXT_SECONDARY.get(),
    );

    hover(response, || {
        format!(
            "{}\n{}\u{00d7}{} {:?}{}{}\nDouble-click: open in Texture workspace",
            tex.name,
            tex.width,
            tex.height,
            tex.format,
            estimate.map(|how| format!("\n{how}")).unwrap_or_default(),
            used.map(|used| format!("\n{used}")).unwrap_or_default()
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn draw_model_tile(
    ui: &mut egui::Ui,
    doc_id: DocumentId,
    model: &DocumentModel,
    view: &ViewState,
    is_selected: bool,
    size: egui::Vec2,
    state: &AppState,
    bridge: Option<&mut RenderBridge>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());

    let img_rect = egui::Rect::from_min_size(rect.min, egui::vec2(size.x, view.cell_size));
    let caption_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x, img_rect.max.y),
        egui::pos2(rect.max.x, rect.max.y),
    );

    if !ui.is_rect_visible(rect) {
        return response;
    }

    ui.painter()
        .rect_filled(img_rect, egui::CornerRadius::ZERO, theme::BG_L1);

    // A tile whose render is on its way is looked at again shortly: its meshes are built off this thread.
    let waits = bridge.is_some();
    let thumb: Option<(Option<egui::TextureId>, usize)> = bridge.and_then(|bridge| {
        let doc = state.document(doc_id)?;
        let key = bridge.thumbnail_key(state, doc, model.selection_id);
        bridge.request_thumbnail(key);
        bridge
            .thumbnail(key)
            .map(|thumb| (thumb.id, thumb.triangles))
    });

    if waits && thumb.is_none() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(30));
    }

    let mut detail: Option<String> = None;
    match thumb {
        Some((Some(id), triangles)) => {
            ui.painter().image(
                id,
                img_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            detail = Some(format!("{} tri", triangles));
        }
        Some((None, _)) => {
            ui.painter().text(
                img_rect.center(),
                egui::Align2::CENTER_CENTER,
                tr!("no geometry"),
                egui::FontId::proportional(10.0),
                theme::TEXT_SECONDARY.get(),
            );
        }
        None => {
            ui.painter().text(
                img_rect.center(),
                egui::Align2::CENTER_CENTER,
                "…",
                egui::FontId::proportional(14.0),
                theme::TEXT_SECONDARY.get(),
            );
        }
    }

    if response.hovered() && !is_selected {
        let wash = theme::ACCENT.gamma_multiply(0.10);
        ui.painter()
            .rect_filled(img_rect, egui::CornerRadius::ZERO, wash);
    }
    let stroke = if is_selected {
        egui::Stroke::new(2.0_f32, theme::ACCENT)
    } else if response.hovered() {
        egui::Stroke::new(1.0_f32, theme::ACCENT_HOVER)
    } else {
        egui::Stroke::new(1.0_f32, theme::BORDER)
    };
    ui.painter().rect_stroke(
        img_rect,
        egui::CornerRadius::ZERO,
        stroke,
        egui::StrokeKind::Inside,
    );

    let caption_painter = ui.painter().with_clip_rect(caption_rect);
    let name_color = if is_selected {
        theme::ACCENT_TEXT.get()
    } else {
        ui.visuals().text_color()
    };
    let name_galley = caption_painter.layout(
        model.name.clone(),
        egui::FontId::proportional(11.0),
        name_color,
        caption_rect.width() - 4.0,
    );
    caption_painter.galley(
        egui::pos2(caption_rect.min.x + 2.0, caption_rect.min.y + 4.0),
        name_galley,
        name_color,
    );
    if let Some(detail) = &detail {
        caption_painter.text(
            egui::pos2(caption_rect.max.x - 2.0, caption_rect.max.y - 2.0),
            egui::Align2::RIGHT_BOTTOM,
            detail,
            egui::FontId::proportional(10.0),
            theme::TEXT_SECONDARY.get(),
        );
    }

    hover(response, || {
        tr!(
            "{name}{detail}\nDouble-click: open in Model workspace",
            name = model.name,
            detail = detail.map(|d| format!("\n{d}")).unwrap_or_default()
        )
    })
}

/// The tile's picture of `tex`, asked for off the UI thread the first time the tile is in view.
fn tile_picture(
    state: &mut AppState,
    ctx: &egui::Context,
    doc_id: DocumentId,
    source: TextureSource,
    tex: &DocumentTexture,
) -> Tile {
    let view = ChannelView::preview_default(tex.format);
    let key = (doc_id, tex.name.clone(), view);
    if let Some(tile) = state.texture_cache.tile(ctx, &key) {
        return tile;
    }
    let held = state.document(doc_id).and_then(|d| {
        textures::texture_at(&d.payload, source, &tex.name).map(textures::Texture::into_owned)
    });
    let texture = match held {
        Some(texture) => {
            debug_assert_eq!(
                texture.format(),
                tex.format,
                "gallery metadata format drift"
            );
            Some(texture)
        }
        // A GTA VI drawable's listing knows no format: its texture lies in another document.
        None => textures::resolved_for_gta6(state, doc_id, None, &tex.name),
    };
    (state.texture_cache).ask_tile(ctx, key, TILE_PICTURE_SIDE, texture)
}

#[derive(Clone, Copy)]
struct ViewState {
    mode: GalleryMode,
    cell_size: f32,
    show_alpha_checker: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            mode: GalleryMode::Textures,
            cell_size: DEFAULT_CELL_SIZE,
            show_alpha_checker: true,
        }
    }
}

impl ViewState {
    fn initial(has_textures: bool) -> Self {
        Self {
            mode: if has_textures {
                GalleryMode::Textures
            } else {
                GalleryMode::Models
            },
            ..Self::default()
        }
    }
}

/// The side of one square of the checkerboard, in points.
const CHECKER_SQUARE: f32 = 8.0;

/// The checkerboard behind a see-through texture: one quad of a two-by-two picture, repeated.
///
/// ⚠ One shape a tile: a rectangle a square was 128 shapes for a tile, and thousands a frame.
fn paint_alpha_checker(painter: &egui::Painter, rect: egui::Rect) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    // Keyed on the theme, whose colours the squares are.
    let id = egui::Id::new(("gallery_alpha_checker", theme::generation()));
    let ctx = painter.ctx();
    let kept = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id));
    let texture = kept.unwrap_or_else(|| {
        let light = theme::BG_L3.get();
        let dark = theme::BG_L2.get();
        let image = egui::ColorImage::new([2, 2], vec![light, dark, dark, light]);
        let options = egui::TextureOptions {
            wrap_mode: egui::TextureWrapMode::Repeat,
            ..egui::TextureOptions::NEAREST
        };
        let texture = ctx.load_texture("gallery-alpha-checker", image, options);
        ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
        texture
    });
    let squares = rect.size() / (2.0 * CHECKER_SQUARE);
    let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, squares.to_pos2());
    painter.image(texture.id(), rect, uv, egui::Color32::WHITE);
}

fn empty(ui: &mut egui::Ui, message: &str) {
    theme::empty_message(ui, message);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gallerys_empty_text_names_no_extension() {
        let mut state = AppState::from_prefs(crate::state::Prefs::default());
        state.documents.clear();
        state.active_document = None;
        state.active_workspace = Workspace::Model;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| draw(ui, &mut state, None));
        let said: Vec<String> = output
            .platform_output
            .accesskit_update
            .map(|update| {
                update
                    .nodes
                    .iter()
                    .filter_map(|(_, node)| {
                        node.label().or_else(|| node.value()).map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();
        assert!(!said.is_empty(), "the gallery drew nothing");
        let named: Vec<&String> = said.iter().filter(|text| text.contains(".y")).collect();
        assert!(named.is_empty(), "{named:?}");
    }
}
