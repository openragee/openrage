//! The preview pane: what the selected archive row is and holds, without
//! opening it as a document; a texture as its image and a model as a render.

use crate::render::RenderBridge;
use crate::state::entry_preview::{self, EntryPreview, PreviewBody};
use crate::state::files_view::{row_facts, RowFacts};
use crate::state::{AppState, DocPayload, DocumentId, RpfNodeId};
use crate::ui::panels::hierarchy::pictures::{picture, Shown};
use crate::ui::theme;

/// The longest side the pane's picture is made at, in pixels.
const PICTURE_SIDE: u32 = 512;

/// A row, its size and name, and how many rows a folder holds: a change to any rebuilds its preview.
type PreviewKey = (DocumentId, RpfNodeId, Option<u64>, String, usize);

/// The last preview built, kept until the selection or the entry changes so a
/// frame does not re-read the bytes.
#[derive(Default)]
pub struct PreviewCache {
    key: Option<PreviewKey>,
    preview: Option<EntryPreview>,
    texture: Option<egui::TextureHandle>,
}

impl crate::ui::panel_state::PanelState for PreviewCache {
    const KEY: &'static str = "file-preview";
}

fn key_of(state: &AppState, doc: DocumentId, node: RpfNodeId) -> Option<PreviewKey> {
    let DocPayload::Rpf(rpf) = &state.document(doc)?.payload else {
        return None;
    };
    let row = rpf.session.node(node)?;
    Some((
        doc,
        node,
        rpf.session.size_for(node),
        row.name.clone(),
        row.children.len(),
    ))
}

/// The preview of `target`, rebuilt only when that or the row changed.
fn current_for(
    state: &mut AppState,
    target: Option<(DocumentId, RpfNodeId)>,
) -> Option<EntryPreview> {
    let (doc, node) = target?;
    let key = key_of(state, doc, node)?;
    let mut cache = state.panels.take::<PreviewCache>();
    if cache.key.as_ref() != Some(&key) {
        cache.preview = match state.document(doc).map(|d| &d.payload) {
            Some(DocPayload::Rpf(rpf)) => entry_preview::preview(&rpf.session, node),
            _ => None,
        };
        cache.texture = None;
        cache.key = Some(key);
    }
    let out = cache.preview.clone();
    state.panels.put(cache);
    out
}

pub fn draw(ui: &mut egui::Ui, state: &mut AppState, bridge: Option<&mut RenderBridge>) {
    let target = state.preview_target();
    draw_for(ui, state, bridge, target);
}

/// The pane for the row `target`, which the Files view makes its folder when no row is selected.
pub fn draw_for(
    ui: &mut egui::Ui,
    state: &mut AppState,
    bridge: Option<&mut RenderBridge>,
    target: Option<(DocumentId, RpfNodeId)>,
) {
    let Some(preview) = current_for(state, target) else {
        ui.add_space(theme::SPACING_RELAXED);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new(egui_phosphor::regular::EYE)
                    .size(28.0)
                    .color(theme::TEXT_DISABLED),
            );
            ui.label(
                egui::RichText::new(tr!("Select a file or folder in an archive to preview it"))
                    .color(theme::TEXT_SECONDARY),
            );
        });
        return;
    };
    let facts = target.and_then(
        |(doc, node)| match state.document(doc).map(|d| &d.payload) {
            Some(DocPayload::Rpf(rpf)) => row_facts(&rpf.session, node),
            _ => None,
        },
    );

    ui.label(egui::RichText::new(&preview.name).strong());
    egui::Grid::new("file_preview_facts")
        .num_columns(2)
        .spacing([10.0, 2.0])
        .show(ui, |ui| {
            for (label, value) in facts_of(&preview, facts.as_ref()) {
                ui.label(
                    egui::RichText::new(label)
                        .small()
                        .color(theme::TEXT_SECONDARY),
                );
                ui.add(egui::Label::new(egui::RichText::new(value).small()).truncate());
                ui.end_row();
            }
        });
    ui.separator();

    egui::ScrollArea::both()
        .id_salt("file_preview_body")
        .auto_shrink([false, false])
        .show(ui, |ui| draw_body(ui, state, bridge, target, &preview));
}

/// The facts the pane lists over the body: the row's columns that apply, and where it is.
fn facts_of(preview: &EntryPreview, facts: Option<&RowFacts>) -> Vec<(&'static str, String)> {
    let mut out = vec![(tr!("Type"), preview.kind.clone())];
    if let Some(size) = preview.size {
        out.push((
            tr!("Size"),
            tr!(
                "{human} ({bytes} bytes)",
                human = or_job::human_bytes(size),
                bytes = size
            ),
        ));
    }
    if let Some(facts) = facts {
        if let Some(stored) = facts.stored.filter(|&s| Some(s) != preview.size) {
            out.push((tr!("Stored size"), or_job::human_bytes(stored)));
        }
        if let Some(compression) = facts.compression {
            out.push((tr!("Compression"), compression.to_string()));
        }
        if let Some(encryption) = facts.encryption {
            out.push((tr!("Encryption"), encryption.to_string()));
        }
        match (facts.edition, facts.version) {
            (Some(edition), Some(version)) => out.push((
                tr!("Edition"),
                tr!(
                    "{edition}, version {version}",
                    edition = edition,
                    version = version
                ),
            )),
            (None, Some(version)) => out.push((tr!("Version"), version.to_string())),
            _ => {}
        }
    }
    out.push((tr!("Path"), preview.path.clone()));
    if !preview.stored_in.is_empty() {
        out.push((tr!("Stored"), preview.stored_in.clone()));
    }
    out
}

fn draw_body(
    ui: &mut egui::Ui,
    state: &mut AppState,
    bridge: Option<&mut RenderBridge>,
    target: Option<(DocumentId, RpfNodeId)>,
    preview: &EntryPreview,
) {
    let dim = |ui: &mut egui::Ui, text: &str| {
        ui.label(
            egui::RichText::new(text)
                .small()
                .color(theme::TEXT_SECONDARY),
        );
    };
    match &preview.body {
        PreviewBody::Folder {
            files,
            folders,
            bytes,
            census,
        } => {
            dim(
                ui,
                &tr!(
                    "{files} file(s) and {folders} folder(s) loaded, {size}",
                    files = files,
                    folders = folders,
                    size = or_job::human_bytes(*bytes)
                ),
            );
            for (ext, count) in census {
                let ext = if ext.is_empty() { tr!("(none)") } else { ext };
                ui.label(
                    egui::RichText::new(format!(".{ext}  {count}"))
                        .small()
                        .monospace(),
                );
            }
        }
        PreviewBody::Text { text, truncated } => {
            ui.add(
                egui::Label::new(egui::RichText::new(text).small().monospace()).selectable(true),
            );
            if *truncated {
                dim(ui, tr!("… the rest is in the file; open it to read it all"));
            }
        }
        PreviewBody::Image {
            width,
            height,
            rgba,
        } => {
            let mut cache = state.panels.take::<PreviewCache>();
            let texture = cache.texture.get_or_insert_with(|| {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [*width as usize, *height as usize],
                    rgba,
                );
                ui.ctx()
                    .load_texture("file-preview", image, egui::TextureOptions::LINEAR)
            });
            let size = egui::vec2(*width as f32, *height as f32);
            let fit = (ui.available_width() / size.x).min(1.0);
            ui.image((texture.id(), size * fit));
            state.panels.put(cache);
            dim(ui, &format!("{width} × {height}"));
        }
        PreviewBody::Picture => {
            let Some((doc, node)) = target else {
                return;
            };
            match picture(ui.ctx(), state, bridge, doc, node, PICTURE_SIDE) {
                Shown::Ready { id, size } => {
                    let fit = (ui.available_width() / size.x).min(2.0);
                    ui.image((id, size * fit));
                }
                Shown::Waiting => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        dim(ui, tr!("Drawing it…"));
                    });
                }
                Shown::None => dim(
                    ui,
                    tr!("OpenRage could not draw it here; open it to see it in its own view"),
                ),
            }
        }
        PreviewBody::Hex { head, total } => {
            let font = egui::TextStyle::Monospace.resolve(ui.style());
            let glyph = ui.ctx().fonts_mut(|f| f.glyph_width(&font, '0'));
            let per_line = entry_preview::bytes_per_line(ui.available_width(), glyph);
            for line in entry_preview::hex_lines(head, per_line) {
                ui.label(egui::RichText::new(line).small().monospace());
            }
            if head.len() < *total {
                dim(
                    ui,
                    &tr!(
                        "first {count} of {total} bytes",
                        total = total,
                        count = head.len()
                    ),
                );
            }
        }
        PreviewBody::TooLarge => dim(
            ui,
            &tr!(
                "Larger than {size} — open it or extract it to look inside",
                size = or_job::human_bytes(entry_preview::READ_LIMIT)
            ),
        ),
        PreviewBody::Unreadable => dim(ui, tr!("Its bytes could not be read")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::DocumentKind;

    fn current(state: &mut AppState) -> Option<EntryPreview> {
        let target = state.preview_target();
        current_for(state, target)
    }

    #[test]
    fn the_pane_follows_the_selection() {
        let mut state = AppState::default();
        let doc = state.new_empty_document(DocumentKind::Rpf);
        let DocPayload::Rpf(rpf) = &mut state.document_mut(doc).unwrap().payload else {
            panic!("an empty RPF document holds an RPF payload");
        };
        let root = rpf.session.root_id();
        let a = rpf
            .session
            .add_file_copy(root, "a.meta", b"<Item />".to_vec())
            .unwrap();
        let b = rpf
            .session
            .add_file_copy(root, "b.bin", vec![0, 1, 2])
            .unwrap();
        state.set_active_document(doc);
        assert!(
            current(&mut state).is_none(),
            "nothing selected, nothing shown"
        );

        state.selection.select(doc, a);
        let shown = current(&mut state).expect("a selected entry previews");
        assert!(matches!(shown.body, PreviewBody::Text { .. }));
        assert_eq!(shown.path, "/a.meta");

        state.selection.select(doc, b);
        let shown = current(&mut state).unwrap();
        assert!(matches!(shown.body, PreviewBody::Hex { total: 3, .. }));

        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(600.0, 600.0),
            )),
            ..Default::default()
        };
        let out = ctx.run_ui(input, |ui| draw(ui, &mut state, None));
        let painted: usize = ctx
            .tessellate(out.shapes, out.pixels_per_point)
            .iter()
            .map(|p| match &p.primitive {
                egui::epaint::Primitive::Mesh(m) => m.vertices.len(),
                _ => 0,
            })
            .sum();
        assert!(painted > 0);
    }
}
