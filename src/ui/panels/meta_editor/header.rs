use crate::state::{AppState, DocPayload, DocumentId};
use crate::ui::theme;

/// What the header's "no schema" mark says on hover.
const NO_SCHEMA: &str = "No schema snapshot was found, so field names show as hashes rather \
                         than names. Nothing is broken — metadata still reads, edits and saves \
                         byte-identically, because the container carries its own type table and \
                         the schema only supplies names.";

pub fn draw(ui: &mut egui::Ui, state: &AppState, doc_id: DocumentId) {
    let Some(doc) = state.document(doc_id) else {
        return;
    };
    let DocPayload::Meta(meta) = &doc.payload else {
        return;
    };

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SPACING_TIGHT;
        let internals = crate::ui::panels::inspector::shows_internals(state);
        let format = meta.source_format();
        if internals && format.is_binary() {
            ui.label(
                egui::RichText::new(format.label())
                    .small()
                    .strong()
                    .color(theme::ACCENT_TEXT),
            )
            .on_hover_text(format!(
                "Loaded as {} binary metadata. Saving re-encodes your edits into the same \
                 container; an unedited document saves byte-identically.",
                format.label()
            ));
            ui.separator();
        }
        let type_name = meta.type_name();
        ui.label(
            egui::RichText::new(if type_name.is_empty() {
                "(unbound)"
            } else {
                type_name
            })
            .monospace()
            .color(theme::TEXT_PRIMARY),
        );
        let read_only = doc
            .read_only
            .filter(|why| *why != crate::state::rpf9_scan::READ_ONLY_BODY);
        if let Some(why) = read_only {
            let mark = egui::RichText::new(tr!("read-only")).small().strong();
            ui.label(mark.color(theme::WARNING))
                .on_hover_text(crate::i18n::tr(why));
        }
        if meta.parse_error.is_some() {
            ui.label(
                egui::RichText::new("⚠ parse error")
                    .small()
                    .strong()
                    .color(theme::WARNING),
            )
            .on_hover_text(
                "The Text tab's XML failed to parse against the installed schema. \
                 Fix it there; the visual view will reload automatically.",
            );
        }
        if let Some(why) = meta.why_it_will_not_save() {
            // Disabled with the reason rather than hidden: a hidden control
            // reads as a missing feature.
            ui.add_enabled(false, egui::Button::new("Save").small())
                .on_disabled_hover_text(format!(
                    "This document will not encode, so saving it would either fail or write \
                     something the game cannot read:\n\n{why}\n\nFix the field named above, \
                     or edit the document in the Text tab."
                ));
            ui.label(
                egui::RichText::new("will not save")
                    .small()
                    .strong()
                    .color(theme::WARNING),
            )
            .on_hover_text(why);
            ui.separator();
        }
        let lib = meta.schema();
        if !or_io::meta::has_default_library() {
            ui.label(
                egui::RichText::new("⚠ no schema")
                    .small()
                    .strong()
                    .color(theme::WARNING),
            )
            .on_hover_text(NO_SCHEMA);
        }
        if let Some(s) = (lib.resolve_any(type_name).map(|r| r.def)).filter(|_| internals) {
            ui.label(
                egui::RichText::new(format!("← {}", s.source_path))
                    .small()
                    .color(theme::TEXT_DIM),
            )
            .on_hover_text(format!(
                "Its fields are named as {} defines them",
                s.source_path
            ));
        }
        if doc.dirty {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new("● unsaved")
                        .small()
                        .color(theme::ACCENT_TEXT),
                );
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_no_schema_hover_names_no_path_in_the_repository() {
        for token in NO_SCHEMA.split_whitespace() {
            assert!(
                !token.contains('/') && !token.ends_with(".md") && !token.ends_with(".rs"),
                "the hover sends the reader to `{token}`, a path in the repository"
            );
        }
    }
}
