//! A GTA VI hash value no table names, shown beside it by the name the harvested text holds.

use or_io::meta::Value;

use super::ctx::EditorCtx;
use crate::ui::theme;

/// The document's own string for `value`, else the vocabulary's marked name in a GTA VI form.
pub(super) fn draw(ui: &mut egui::Ui, ctx: &EditorCtx, value: &Value) {
    if let Some(name) = shown_name(ctx, value) {
        ui.label(egui::RichText::new(name).small().color(theme::TEXT_DIM));
    }
}

/// The name shown beside `value`: a 32-bit hash's, or the document's own string a 64-bit number
/// hashes from, as the Text panel names a `value="N"`.
fn shown_name(ctx: &EditorCtx, value: &Value) -> Option<String> {
    match value {
        Value::Hash(hash) => {
            let own = (hash.raw.is_empty())
                .then(|| ctx.strings.name(hash.hash).map(str::to_owned))
                .flatten();
            own.or_else(|| {
                (ctx.text_names)
                    .then(|| crate::state::meta_text_name(hash))
                    .flatten()
            })
        }
        Value::UInt(wide) => (ctx.strings.name64(*wide).map(str::to_owned)).or_else(|| {
            (ctx.text_names)
                .then(|| wide_text_name(&crate::state::gta6_names::vocabulary(), *wide))
                .flatten()
        }),
        _ => None,
    }
}

/// The vocabulary's name for the 64-bit `wide`, marked as from the text.
fn wide_text_name(vocabulary: &or_io::gta6::names::Vocabulary, wide: u64) -> Option<String> {
    let name = vocabulary.name64(wide)?;
    Some(tr!("{name} (from the text)", name = name))
}

/// The sentence a document shows where its strings are enciphered and no key snapshot reads them.
pub(super) fn draw_needs_keys(ui: &mut egui::Ui, strings: &crate::state::DocumentStrings) {
    if strings.needs_keys {
        let said = crate::i18n::tr(crate::state::STRING_NAMES_NEED_KEYS).to_string();
        ui.label(egui::RichText::new(said).small().color(theme::TEXT_DIM));
    }
}

#[cfg(test)]
pub(super) mod tests {
    use std::sync::Arc;

    use super::*;

    /// A PSO whose `LSNS` holds the one clear category-1 run naming `text`.
    pub(in super::super) fn document(text: &str) -> Vec<u8> {
        let mut data = text.as_bytes().to_vec();
        data.resize(text.len().div_ceil(16) * 16 + 16, 0);
        let mut blob = 1u32.to_le_bytes().to_vec();
        blob.extend_from_slice(&0x01u32.to_le_bytes());
        blob.extend_from_slice(&(data.len() as u32).to_le_bytes());
        blob.extend_from_slice(&data);
        let mut doc = Vec::new();
        for (ident, content) in [(&b"PSIN"[..], &[0u8; 8][..]), (b"LSNS", &blob)] {
            doc.extend_from_slice(ident);
            doc.extend_from_slice(&(content.len() as u32 + 8).to_be_bytes());
            doc.extend_from_slice(content);
        }
        doc
    }

    #[test]
    fn a_64_bit_number_the_documents_own_string_hashes_to_shows_that_string() {
        let text = "zq_inspector_wide";
        let mut ctx = EditorCtx::for_test();
        ctx.strings = Arc::new(crate::state::DocumentStrings::of(&document(text), || None));
        let wide = or_io::gta6::hash::name_hash64(text.as_bytes());
        assert_eq!(shown_name(&ctx, &Value::UInt(wide)), Some(text.into()));
        assert_eq!(shown_name(&ctx, &Value::UInt(wide ^ 1)), None);
        assert_eq!(shown_name(&ctx, &Value::Int(wide as i64)), None);
        // The 32-bit hash of the same string still names it.
        let narrow =
            or_io::meta::instance::HashedString::from_hash(or_io::string_hash(text.as_bytes()));
        assert_eq!(shown_name(&ctx, &Value::Hash(narrow)), Some(text.into()));
    }

    #[test]
    fn a_64_bit_number_no_own_string_names_takes_the_vocabulary_s_marked_name() {
        let text = "zq_form_vocabulary_wide";
        let mut vocabulary = or_io::gta6::names::Vocabulary::default();
        vocabulary.add(text, None);
        let wide = or_io::gta6::hash::name_hash64(text.as_bytes());
        assert_eq!(
            wide_text_name(&vocabulary, wide),
            Some(format!("{text} (from the text)"))
        );
        assert_eq!(wide_text_name(&vocabulary, wide ^ 1), None);
        // A form that is no GTA VI one asks no vocabulary.
        assert_eq!(shown_name(&EditorCtx::for_test(), &Value::UInt(wide)), None);
    }
}
