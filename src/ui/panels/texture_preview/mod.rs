use or_io::codec::texture::{Channel, ChannelView};

use crate::state::textures::TextureSource;
use crate::state::{textures, AppState, CachedTexture, DocPayload, TextureResolution};
use crate::ui::accessible_name::named;
use crate::ui::dock::Workspace;
use crate::ui::panels::selection_ids::DOC_ROOT_SEL;
use crate::ui::theme;

mod edit;

use edit::draw_encoding_bar;
pub(crate) use edit::{export, replace_from_image, selected_texture};

/// Why Apply, Replace from image, Embed and Unembed are greyed over an Enhanced texture: the texture editor writes the Legacy layout.
pub(crate) const ENHANCED_TEXTURES: &str =
    "Enhanced textures cannot be edited yet: the texture editor writes only the Legacy layout.";

fn channel_picker(ui: &mut egui::Ui, default: ChannelView, view: &mut ChannelView) {
    let label = match view {
        ChannelView::Rgba => "RGB",
        ChannelView::Grey(Channel::R) => "R",
        ChannelView::Grey(Channel::G) => "G",
        ChannelView::Grey(Channel::B) => "B",
        ChannelView::Grey(Channel::A) | ChannelView::AlphaOnly => "A",
        ChannelView::NormalMapRg => "N",
    };

    let selected = egui::RichText::new(label).small();
    combo_box(ui, "texture_preview_channel", 56.0, selected, true, |ui| {
        ui.selectable_value(view, ChannelView::Rgba, "RGB (as stored)");
        ui.selectable_value(view, ChannelView::Grey(Channel::R), "Red");
        ui.selectable_value(view, ChannelView::Grey(Channel::G), "Green");
        ui.selectable_value(view, ChannelView::Grey(Channel::B), "Blue");
        ui.selectable_value(view, ChannelView::AlphaOnly, "Alpha");
        ui.selectable_value(view, ChannelView::NormalMapRg, "Normal (rebuild Z)");
        ui.separator();
        if ui.button("Reset to format default").clicked() {
            *view = default;
            ui.close();
        }
    })
    .on_hover_text("Isolate a channel. Decoded pixels are spec-exact, so packed masks show up in whichever channel really holds them.");
}

/// A combo box at least `width` wide, in a slot of its own size: egui lays a
/// combo box out in what is left of a row, so in a wrapping row it would run
/// past the panel's edge rather than move to the next row.
fn combo_box(
    ui: &mut egui::Ui,
    id_salt: &str,
    width: f32,
    selected: egui::RichText,
    enabled: bool,
    menu: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    let spacing = ui.spacing();
    // As egui sizes the button: the text, the arrow and the padding.
    let chrome = spacing.icon_spacing + spacing.icon_width + 2.0 * spacing.button_padding.x;
    let height = spacing.interact_size.y;
    let text = egui::WidgetText::from(selected.clone()).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    let size = egui::vec2(width.max(text.size().x + chrome), height);
    ui.allocate_ui(size, |ui| {
        ui.add_enabled_ui(enabled, |ui| {
            egui::ComboBox::from_id_salt(id_salt)
                .selected_text(selected)
                .width(width)
                .show_ui(ui, menu)
                .response
        })
        .inner
    })
    .inner
}

/// `add`, told whether a separator leads the unit, as one unit of a wrapping row: egui wraps a
/// row one widget at a time, which would leave a separator ending a row or starting one. The unit
/// starts the next row whole when what is left of this one is narrower than it was drawn last
/// frame, and a separator only follows something in its row.
fn kept_together(ui: &mut egui::Ui, id_salt: &str, add: impl FnOnce(&mut egui::Ui, bool)) {
    let id = ui.make_persistent_id(id_salt);
    // What the unit takes with its separator, which is its own width and the gap after it.
    let wanted: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(0.0);
    let separator = ui.spacing().item_spacing.x * 2.0;
    let room = ui.available_size_before_wrap().x;
    let starts_row = room >= ui.max_rect().width() - 0.5;
    if !starts_row && room < wanted {
        ui.end_row();
    }
    let leads = !starts_row && room >= wanted;
    let used = ui.horizontal(|ui| add(ui, leads)).response.rect.width();
    let unit = if leads { used } else { used + separator };
    ui.data_mut(|d| d.insert_temp(id, unit));
    if (unit - wanted).abs() > 0.5 {
        ui.ctx().request_repaint();
    }
}

pub fn draw(ui: &mut egui::Ui, state: &mut AppState) {
    // ⚠ Whatever the Texture workspace handles, in whichever workspace this panel is docked: the
    // Particle workspace has it beside a `.ypt`, and a `.ytd` active there is still a texture dictionary.
    let shows =
        (state.active_document()).is_some_and(|doc| Workspace::Texture.handles_kind(doc.kind));
    let Some(doc_id) = state.active_document.filter(|_| shows) else {
        empty(
            ui,
            "Open a texture dictionary, a model or particle effects to preview their textures",
        );
        return;
    };

    if let Some(label) = state
        .document(doc_id)
        .and_then(|d| textures::image_label(&d.payload))
    {
        draw_document_image(ui, state, doc_id, label);
        return;
    }

    let selection_id = state
        .selection
        .selected(doc_id)
        .filter(|&s| s != DOC_ROOT_SEL);

    let (texture_name, source) = {
        let Some(doc) = state.document(doc_id) else {
            empty(ui, "No document");
            return;
        };
        let resolution = match (selection_id, textures::sole_texture(&doc.payload)) {
            (_, Some(name)) => TextureResolution::Found {
                name,
                source: TextureSource::Document,
            },
            (Some(selection_id), None) => {
                textures::texture_for_selection(&doc.payload, selection_id)
            }
            (None, None) => {
                empty(ui, &format!("Select a texture in {}", doc.file_name()));
                return;
            }
        };
        match resolution {
            TextureResolution::Found { name, source } => (name, source),
            TextureResolution::NotATexture(hint) => {
                empty(ui, hint);
                return;
            }
            TextureResolution::Missing => {
                empty(ui, "Selected texture is no longer available");
                return;
            }
        }
    };

    // A pack's cube, array or volume is shown one slice at a time, each cached under its own name.
    let slice_id = ui.make_persistent_id(("texture_preview_slice", doc_id, &texture_name));
    let mut slice: u32 = ui.data(|d| d.get_temp::<u32>(slice_id)).unwrap_or(0);
    let mut slices = match state.document(doc_id).map(|d| &d.payload) {
        Some(DocPayload::Ttpf(ttpf)) => ttpf.index_of(&texture_name).map_or(1, |index| {
            let slices = ttpf.slices(index);
            slice = slice.min(slices - 1);
            ttpf.show_slice(index, slice);
            slices
        }),
        _ => 1,
    };
    // A cube's slices are its faces, six per cube.
    let mut cube = match state.document(doc_id).map(|d| &d.payload) {
        Some(DocPayload::Ttpf(ttpf)) => ttpf
            .index_of(&texture_name)
            .and_then(|index| ttpf.pack.entries.get(index))
            .is_some_and(|entry| entry.dimension == 2),
        _ => false,
    };

    let Some(doc) = state.document(doc_id) else {
        empty(ui, "No document");
        return;
    };
    let refused = crate::state::ttpf_document::refused(doc, &texture_name);
    let texture = textures::texture_at(&doc.payload, source, &texture_name)
        .map(textures::Texture::into_owned)
        .filter(|_| refused.is_none());
    // A scanned dictionary's texture holds its slices one after another.
    let texture = match texture {
        Some(textures::Texture::Owned(t))
            if slices == 1 && crate::state::ttpf_document::scanned_slices(doc, &t) > 1 =>
        {
            slices = crate::state::ttpf_document::scanned_slices(doc, &t);
            slice = slice.min(slices - 1);
            crate::state::ttpf_document::show_scanned_slice(doc, &texture_name, slice);
            cube = t.image_type == 1;
            let t = crate::state::ttpf_document::scanned_slice(t.into_owned(), slice, slices);
            Some(textures::Texture::Owned(std::borrow::Cow::Owned(t)))
        }
        texture => texture,
    };
    let shown_name = match slices {
        1 => texture_name.clone(),
        _ => format!("{texture_name}@slice{slice}"),
    };
    let texture =
        texture.or_else(|| textures::resolved_for_gta6(state, doc_id, selection_id, &texture_name));
    let Some(texture) = texture else {
        // A pack's texture, or a scanned dictionary's, is there and may not read; it says why.
        let why = match state.document(doc_id).map(|d| &d.payload) {
            Some(DocPayload::Ttpf(ttpf)) => ttpf
                .index_of(&texture_name)
                .and_then(|index| ttpf.refusal(index)),
            _ => refused,
        };
        match why {
            Some(why) => empty(
                ui,
                &tr!("{name} is not shown: {why}", name = texture_name, why = why),
            ),
            None => empty(ui, "Texture not found"),
        }
        return;
    };

    let channel_id = ui.make_persistent_id(("texture_preview_channels", doc_id, &texture_name));
    let mut channel_view: ChannelView = ui
        .data(|d| d.get_temp::<ChannelView>(channel_id))
        .unwrap_or_else(|| ChannelView::preview_default(texture.format()));

    let levels = or_io::codec::texture::TextureSource::stored_levels(&texture).max(1);
    let mip_id = ui.make_persistent_id(("texture_preview_mip", doc_id, &texture_name));
    let mut mip: u32 = ui
        .data(|d| d.get_temp::<u32>(mip_id))
        .unwrap_or(0)
        .min(levels - 1);

    let cached = if mip == 0 {
        state.texture_cache.get_or_decode(
            ui.ctx(),
            (doc_id, shown_name.clone(), channel_view),
            &texture,
        )
    } else {
        // Cached under the texture's name and the level, so forgetting the document forgets it too.
        state.texture_cache.get_or_insert_image(
            ui.ctx(),
            (doc_id, format!("{shown_name}@mip{mip}"), channel_view),
            egui::TextureOptions::LINEAR,
            || or_io::codec::texture::decode_level(&texture, mip).ok(),
        )
    };

    let view_id = ui.make_persistent_id(("texture_preview_view", doc_id, &texture_name));
    let mut view: ViewState = ui
        .data(|d| d.get_temp::<ViewState>(view_id))
        .unwrap_or_default();

    let (estimate, doubt) = match state.document(doc_id).map(|d| &d.payload) {
        Some(DocPayload::Ttpf(ttpf)) => match ttpf.index_of(&texture_name) {
            Some(index) => (ttpf.estimate_label(index), ttpf.doubt_label(index)),
            None => (None, None),
        },
        _ => (None, None),
    };
    egui::Panel::top(ui.make_persistent_id("texture_preview_meta"))
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 4)))
        .show_inside(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                // ⚠ Truncated rather than flowed across rows, so each part of a
                // wrapped header keeps a rectangle of its own.
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                ui.label(
                    egui::RichText::new(&texture_name)
                        .color(theme::ACCENT_TEXT)
                        .strong(),
                );
                let mip_text = if texture.mip_count() > 1 {
                    format!(" • {} mips", texture.mip_count())
                } else {
                    String::new()
                };
                ui.label(
                    egui::RichText::new(format!(
                        "{}×{} • {:?}{}",
                        texture.width(),
                        texture.height(),
                        texture.format(),
                        mip_text
                    ))
                    .small()
                    .color(theme::TEXT_SECONDARY),
                );
                // A picture through fitted tables is not the one the game decodes, and says so.
                if let Some(how) = &estimate {
                    let said = tr!("An estimate, not the exact picture: {how}", how = how);
                    named(ui.label(egui::RichText::new(&said).small().strong()), &said);
                }
                // So does one through supplied tables that a block speaks against, or that
                // nothing has been able to speak against yet.
                if let Some(how) = &doubt {
                    let said = tr!("Not called exact: {how}", how = how);
                    named(ui.label(egui::RichText::new(&said).small().strong()), &said);
                }
                if levels > 1 {
                    mip_picker(ui, &texture, levels, &mut mip);
                }
                if slices > 1 {
                    let picker = egui::DragValue::new(&mut slice)
                        .range(0..=slices - 1)
                        .prefix(format!("{} ", tr!("slice")))
                        .suffix(format!(" / {slices}"));
                    named(ui.add(picker), tr!("Slice")).on_hover_text(tr!(
                        "Show one slice: a face of a cube, or a slice of an array or a volume"
                    ));
                    if cube {
                        let face = cube_face(slice, slices);
                        let shown = ui.label(
                            egui::RichText::new(&face)
                                .small()
                                .color(theme::TEXT_SECONDARY),
                        );
                        named(shown, &face);
                    }
                }
                view_controls(
                    ui,
                    &mut view,
                    &mut channel_view,
                    ChannelView::preview_default(texture.format()),
                );
            });
        });

    egui::Panel::top(ui.make_persistent_id("texture_preview_encoding"))
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 4)))
        .show_inside(ui, |ui| {
            draw_encoding_bar(ui, state, doc_id, source, &texture, mip);
        });

    // The first things done to a texture, on the picture itself as on its tile.
    let mut asked = None;
    let menu = |ui: &mut egui::Ui| {
        use crate::ui::commands::{offer_plain, Action};
        for action in [Action::ReplaceTextureFromImage, Action::ExportTexture] {
            offer_plain(ui, state, action, &mut asked);
        }
    };
    canvas(
        ui,
        cached,
        &mut view,
        || {
            format!(
                "Cannot preview {:?}\n(format not yet decodable for {}×{})",
                texture.format(),
                texture.width(),
                texture.height()
            )
        },
        Some(menu),
    );
    if let Some(command) = asked {
        crate::ui::commands::run(state, ui.ctx(), &command);
    }

    ui.data_mut(|d| {
        d.insert_temp(view_id, view);
        d.insert_temp(channel_id, channel_view);
        d.insert_temp(mip_id, mip);
        d.insert_temp(slice_id, slice);
    });
}

/// The face slice `slice` of a cube's `slices` shows, and its cube where there are several, cube after cube.
fn cube_face(slice: u32, slices: u32) -> String {
    let face = ["+X", "-X", "+Y", "-Y", "+Z", "-Z"][(slice % 6) as usize];
    match slices {
        6 => tr!("face {face}", face = face),
        _ => tr!("cube {cube}, face {face}", cube = slice / 6, face = face),
    }
}

/// A picture a document offers that is not a texture: the same view, with nothing to encode.
fn draw_document_image(
    ui: &mut egui::Ui,
    state: &mut AppState,
    doc_id: crate::state::DocumentId,
    label: &'static str,
) {
    let channel_id = ui.make_persistent_id(("texture_preview_image_channels", doc_id));
    let mut channel_view: ChannelView = ui
        .data(|d| d.get_temp::<ChannelView>(channel_id))
        .unwrap_or(ChannelView::Rgba);
    // The picture is built only on a miss: a height map is never edited in place.
    let key = (doc_id, label.to_string(), channel_view);
    let cached = match state.texture_cache.get(&key).cloned() {
        Some(cached) => Some(cached),
        None => {
            let made = state
                .document(doc_id)
                .and_then(|d| textures::document_image(&d.payload));
            state.texture_cache.get_or_insert_image(
                ui.ctx(),
                key,
                egui::TextureOptions::NEAREST,
                || made,
            )
        }
    };

    let view_id = ui.make_persistent_id(("texture_preview_image_view", doc_id));
    let mut view: ViewState = ui
        .data(|d| d.get_temp::<ViewState>(view_id))
        .unwrap_or_default();

    egui::Panel::top(ui.make_persistent_id("texture_preview_image_meta"))
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 4)))
        .show_inside(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                ui.label(
                    egui::RichText::new(label)
                        .color(theme::ACCENT_TEXT)
                        .strong(),
                );
                if let Some(cached) = &cached {
                    ui.label(
                        egui::RichText::new(format!("{}×{}", cached.width, cached.height))
                            .small()
                            .color(theme::TEXT_SECONDARY),
                    );
                }
                view_controls(ui, &mut view, &mut channel_view, ChannelView::Rgba);
            });
        });

    canvas(
        ui,
        cached,
        &mut view,
        || "This document has no picture to show".to_string(),
        None::<fn(&mut egui::Ui)>,
    );

    ui.data_mut(|d| {
        d.insert_temp(view_id, view);
        d.insert_temp(channel_id, channel_view);
    });
}

fn mip_picker(ui: &mut egui::Ui, texture: &textures::Texture<'_>, levels: u32, mip: &mut u32) {
    let (width, height) = or_io::codec::texture::TextureSource::stored_size(texture);
    let format = texture.format();
    let label = |level: u32| {
        let (w, h) = format.mip_dimensions(width, height, level);
        format!("mip {level} • {w}×{h}")
    };
    let width = ui.spacing().combo_width;
    let selected = egui::RichText::new(label(*mip)).small();
    combo_box(ui, "texture_preview_mip", width, selected, true, |ui| {
        for level in 0..levels {
            ui.selectable_value(mip, level, label(level));
        }
    })
    .on_hover_text("Show one stored mip level as the file holds it");
}

/// Zoom, fit, the alpha checker and the channel, over whatever the canvas shows.
fn view_controls(
    ui: &mut egui::Ui,
    view: &mut ViewState,
    channel_view: &mut ChannelView,
    default: ChannelView,
) {
    let reset = ui
        .small_button("Reset")
        .on_hover_text("Recenter and reset zoom (or middle-double-click)");
    if reset.clicked() {
        *view = ViewState::default();
    }
    if ui
        .small_button("100%")
        .on_hover_text("Reset to native pixel scale (1:1)")
        .clicked()
    {
        view.zoom = 1.0;
        view.exact_pixels = true;
    }
    if ui
        .small_button("Fit")
        .on_hover_text("Fit the texture to the panel")
        .clicked()
    {
        *view = ViewState::default();
    }
    kept_together(ui, "texture_view_alpha", |ui, separated| {
        if separated {
            ui.separator();
        }
        ui.checkbox(&mut view.show_alpha_checker, "α");
        ui.label(
            egui::RichText::new(format!("{:.0}%", view.zoom * 100.0))
                .small()
                .color(theme::TEXT_SECONDARY),
        );
    });
    kept_together(ui, "texture_view_channel", |ui, separated| {
        if separated {
            ui.separator();
        }
        channel_picker(ui, default, channel_view);
    });
}

/// The picture, panned with the middle button and zoomed with Ctrl and the wheel.
fn canvas<M: FnOnce(&mut egui::Ui)>(
    ui: &mut egui::Ui,
    cached: Option<CachedTexture>,
    view: &mut ViewState,
    undecodable: impl FnOnce() -> String,
    menu: Option<M>,
) {
    egui::CentralPanel::default()
        .frame(
            egui::Frame::NONE
                .fill(theme::BG_L0.get())
                .inner_margin(egui::Margin::same(8)),
        )
        .show_inside(ui, |ui| match cached {
            Some(cached) => {
                let avail_rect = ui.available_rect_before_wrap();
                let (rect, response) =
                    ui.allocate_exact_size(avail_rect.size(), egui::Sense::click_and_drag());
                // The right button moves nothing here, so it is the menu's.
                if let Some(menu) = menu {
                    response.context_menu(menu);
                }

                let img_w = cached.width as f32;
                let img_h = cached.height as f32;
                let base_scale = (rect.width() / img_w)
                    .min(rect.height() / img_h)
                    .clamp(0.01, 1.0);

                if response.dragged_by(egui::PointerButton::Middle) {
                    view.pan += response.drag_delta();
                }
                if response.double_clicked_by(egui::PointerButton::Middle) {
                    *view = ViewState::default();
                }

                if response.hovered() {
                    let (scroll_y, ctrl, hover_pos) = ui.input(|i| {
                        let mut delta = 0.0_f32;
                        for ev in &i.events {
                            if let egui::Event::MouseWheel { unit, delta: d, .. } = ev {
                                let line_height = match unit {
                                    egui::MouseWheelUnit::Point => 1.0,
                                    egui::MouseWheelUnit::Line => 50.0,
                                    egui::MouseWheelUnit::Page => 400.0,
                                };
                                delta += d.y * line_height;
                            }
                        }
                        (
                            delta,
                            i.modifiers.ctrl || i.modifiers.command,
                            i.pointer.hover_pos(),
                        )
                    });
                    if ctrl && scroll_y != 0.0 {
                        let factor = (scroll_y * 0.0015).exp();
                        let new_zoom = (view.zoom * factor).clamp(0.05, 50.0);
                        let actual = new_zoom / view.zoom;
                        if let Some(cursor) = hover_pos {
                            let cursor_off = cursor - rect.center();
                            view.pan = (view.pan - cursor_off) * actual + cursor_off;
                        }
                        view.zoom = new_zoom;
                    }
                }

                let scale = if view.exact_pixels {
                    view.zoom
                } else {
                    base_scale * view.zoom
                };
                let size = egui::vec2(img_w * scale, img_h * scale);
                let img_rect = egui::Rect::from_center_size(rect.center() + view.pan, size);

                let prev_clip = ui.clip_rect();
                ui.set_clip_rect(rect.intersect(prev_clip));
                if view.show_alpha_checker {
                    paint_alpha_checker(ui.painter(), img_rect.intersect(rect));
                }
                egui::Image::new(&cached.handle)
                    .maintain_aspect_ratio(true)
                    .paint_at(ui, img_rect);
                ui.set_clip_rect(prev_clip);
            }
            None => empty_central(ui, &undecodable()),
        });
}

#[derive(Clone, Copy)]
struct ViewState {
    pan: egui::Vec2,
    zoom: f32,
    show_alpha_checker: bool,
    exact_pixels: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            pan: egui::Vec2::ZERO,
            zoom: 1.0,
            show_alpha_checker: true,
            exact_pixels: false,
        }
    }
}

fn paint_alpha_checker(painter: &egui::Painter, rect: egui::Rect) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let tile = 8.0;
    let light = theme::BG_L3.get();
    let dark = theme::BG_L2.get();
    painter.rect_filled(rect, egui::CornerRadius::ZERO, dark);
    let mut y = rect.top();
    let mut row = 0u32;
    while y < rect.bottom() {
        let mut x = rect.left();
        let mut col = row;
        while x < rect.right() {
            if col & 1 == 0 {
                let cell = egui::Rect::from_min_max(
                    egui::pos2(x, y),
                    egui::pos2((x + tile).min(rect.right()), (y + tile).min(rect.bottom())),
                );
                painter.rect_filled(cell, egui::CornerRadius::ZERO, light);
            }
            x += tile;
            col = col.wrapping_add(1);
        }
        y += tile;
        row = row.wrapping_add(1);
    }
}

fn empty(ui: &mut egui::Ui, message: &str) {
    theme::empty_message(ui, message);
}

fn empty_central(ui: &mut egui::Ui, message: &str) {
    theme::empty_message(ui, message);
}

#[cfg(test)]
mod tests {
    use egui::accesskit::Role;
    use or_io::codec::texture::GrcTextureFormat;
    use or_io::gta5::{gen8, PgDictionary};

    use super::*;
    use crate::state::{Document, DocumentId, Prefs, TextureListing};
    use crate::ui::dock::Workspace;

    type Drawn = Vec<(Role, String, egui::Pos2)>;

    /// Every widget a frame drew: its role, its label and its value (one entry each), and its centre.
    fn widgets(output: egui::FullOutput) -> Drawn {
        output
            .platform_output
            .accesskit_update
            .map(|update| {
                update
                    .nodes
                    .iter()
                    .flat_map(|(_, node)| {
                        let b = node.bounds();
                        let centre = b.map(|b| {
                            egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
                        });
                        [node.label(), node.value()]
                            .into_iter()
                            .flatten()
                            .filter_map(move |text| Some((node.role(), text.to_string(), centre?)))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 800.0),
            )),
            events,
            ..Default::default()
        }
    }

    fn click(at: egui::Pos2) -> [Vec<egui::Event>; 2] {
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        [
            vec![egui::Event::PointerMoved(at), press(true)],
            vec![press(false)],
        ]
    }

    fn find(drawn: &Drawn, role: Role, text: &str) -> Option<egui::Pos2> {
        drawn
            .iter()
            .find(|(r, t, _)| *r == role && t == text)
            .map(|(_, _, at)| *at)
    }

    #[test]
    fn one_hundred_percent_stays_until_another_zoom_is_chosen() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut view = ViewState::default();
        let mut channel = ChannelView::Rgba;
        let mut frame = |view: &mut ViewState, events| {
            widgets(ctx.run_ui(input(events), |ui| {
                ui.horizontal(|ui| view_controls(ui, view, &mut channel, ChannelView::Rgba));
            }))
        };
        let drawn = frame(&mut view, Vec::new());
        let hundred = find(&drawn, Role::Button, "100%").expect("the 100% button");
        for events in click(hundred) {
            frame(&mut view, events);
        }
        assert!(view.exact_pixels, "100% did not take");
        frame(&mut view, Vec::new());
        frame(&mut view, Vec::new());
        assert!(
            view.exact_pixels && view.zoom == 1.0,
            "100% went back to fitting the panel on the next frame"
        );

        let fit = find(&frame(&mut view, Vec::new()), Role::Button, "Fit").expect("Fit");
        for events in click(fit) {
            frame(&mut view, events);
        }
        assert!(!view.exact_pixels, "Fit leaves 100% on");
    }

    /// A four-by-four texture named `name` in `format`.
    fn texture(name: &str, format: GrcTextureFormat, bytes: usize) -> gen8::GrcTexture {
        gen8::GrcTexture {
            name: name.to_string(),
            width: 4,
            height: 4,
            depth: 1,
            format: format.to_raw_format(),
            mip_count: 1,
            backing_store: vec![0; bytes],
            ..Default::default()
        }
    }

    /// A Legacy dictionary of an uncompressed `paint` and a DXT1 `dirt`, on screen in Texture.
    fn two_textures() -> (AppState, DocumentId) {
        let mut dictionary = PgDictionary::default();
        dictionary.insert(texture("paint", GrcTextureFormat::A8R8G8B8, 4 * 4 * 4));
        dictionary.insert(texture("dirt", GrcTextureFormat::DXT1, 8));
        let mut state = AppState::from_prefs(Prefs::default());
        state.documents.clear();
        let id = 4242;
        state.documents.push(Document::new(
            id,
            Some(std::path::PathBuf::from("C:/mods/props.ytd")),
            DocPayload::Resource(or_io::Asset::TextureDict(dictionary)),
        ));
        state.active_document = Some(id);
        state.active_workspace = Workspace::Texture;
        (state, id)
    }

    fn select(state: &mut AppState, doc: DocumentId, name: &str) {
        let payload = &state.document(doc).expect("open").payload;
        let TextureListing::Textures { textures, .. } = textures::textures_in(payload, None) else {
            panic!("the dictionary lists no textures");
        };
        let id = textures
            .iter()
            .find(|t| t.name == name)
            .map(|t| t.selection_id)
            .expect("the texture is listed");
        state.selection.select(doc, id);
    }

    fn preview(ctx: &egui::Context, state: &mut AppState, events: Vec<egui::Event>) -> Drawn {
        widgets(ctx.run_ui(input(events), |ui| draw(ui, state)))
    }

    /// The format the encoding bar offers for the texture on screen.
    fn offered_format(ctx: &egui::Context, state: &mut AppState) -> String {
        preview(ctx, state, Vec::new());
        let drawn = preview(ctx, state, Vec::new());
        drawn
            .iter()
            .filter(|(role, _, _)| *role == Role::ComboBox)
            .map(|(_, text, _)| text.clone())
            .find(|text| text.starts_with("DXT") || text == "A8R8G8B8")
            .unwrap_or_else(|| panic!("no format is offered: {drawn:?}"))
    }

    #[test]
    fn each_texture_keeps_its_own_encode_options() {
        let (mut state, doc) = two_textures();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();

        select(&mut state, doc, "paint");
        assert_eq!(offered_format(&ctx, &mut state), "A8R8G8B8");
        let drawn = preview(&ctx, &mut state, Vec::new());
        let list = find(&drawn, Role::ComboBox, "A8R8G8B8").expect("the format list");
        for events in click(list) {
            preview(&ctx, &mut state, events);
        }
        let drawn = preview(&ctx, &mut state, Vec::new());
        let dxt5 = find(&drawn, Role::Button, "DXT5").expect("DXT5 in the open list");
        for events in click(dxt5) {
            preview(&ctx, &mut state, events);
        }
        assert_eq!(
            offered_format(&ctx, &mut state),
            "DXT5",
            "the choice did not take"
        );

        select(&mut state, doc, "dirt");
        assert_eq!(
            offered_format(&ctx, &mut state),
            "DXT1",
            "dirt shows the options chosen for paint"
        );
        select(&mut state, doc, "paint");
        assert_eq!(
            offered_format(&ctx, &mut state),
            "DXT5",
            "paint forgot its choice"
        );
    }

    #[test]
    fn re_encoding_a_compressed_texture_warns_that_it_loses_detail() {
        let (mut state, doc) = two_textures();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let warned = |ctx: &egui::Context, state: &mut AppState| {
            preview(ctx, state, Vec::new());
            preview(ctx, state, Vec::new())
                .iter()
                .any(|(_, text, _)| text.contains("loses detail"))
        };

        select(&mut state, doc, "dirt");
        assert!(
            warned(&ctx, &mut state),
            "DXT1 is re-encoded with no word of the loss"
        );
        select(&mut state, doc, "paint");
        assert!(
            !warned(&ctx, &mut state),
            "an uncompressed texture loses nothing"
        );
    }

    #[test]
    fn the_texture_views_empty_text_names_no_extension() {
        let (mut state, _) = two_textures();
        state.documents.clear();
        state.active_document = None;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let drawn = preview(&ctx, &mut state, Vec::new());
        let named: Vec<&String> = drawn
            .iter()
            .map(|(_, text, _)| text)
            .filter(|text| text.contains(".y") || text.contains(".dds"))
            .collect();
        assert!(named.is_empty(), "{named:?}");
    }

    #[test]
    fn a_cube_array_s_slice_names_its_cube_and_its_face() {
        assert_eq!(cube_face(4, 6), "face +Z");
        assert_eq!(cube_face(9, 156), "cube 1, face -Y");
        assert_eq!(cube_face(155, 156), "cube 25, face -Z");
    }
}
