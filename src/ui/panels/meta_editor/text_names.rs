//! A GTA VI hash value no table names, shown beside it by the name the harvested text holds.

use or_io::meta::Value;

use super::ctx::EditorCtx;
use crate::ui::theme;

/// The document's own string for `value`, else the vocabulary's marked name in a GTA VI form.
pub(super) fn draw(ui: &mut egui::Ui, ctx: &EditorCtx, value: &Value) {
    let Value::Hash(hash) = value else {
        return;
    };
    let own = (hash.raw.is_empty())
        .then(|| ctx.strings.name(hash.hash).map(str::to_owned))
        .flatten();
    let text = (ctx.text_names)
        .then(|| crate::state::meta_text_name(hash))
        .flatten();
    if let Some(name) = own.or(text) {
        ui.label(egui::RichText::new(name).small().color(theme::TEXT_DIM));
    }
}

/// The sentence a document shows where its strings are enciphered and no key snapshot reads them.
pub(super) fn draw_needs_keys(ui: &mut egui::Ui, strings: &crate::state::DocumentStrings) {
    if strings.needs_keys {
        let said = crate::i18n::tr(crate::state::STRING_NAMES_NEED_KEYS).to_string();
        ui.label(egui::RichText::new(said).small().color(theme::TEXT_DIM));
    }
}
