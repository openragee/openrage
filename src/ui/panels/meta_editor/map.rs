//! A map's pairs in the form: each pair's key and value, a pair removed, and a pair added under a
//! typed key with its value's default, each a step of undo that holds the whole map.
//!
//! ⚠ A place in the tree reaches no pair, so an edit inside a pair's value is recorded as the
//! whole map it changed.

use std::cell::RefCell;
use std::rc::Rc;

use egui_phosphor::regular as ph;
use or_io::meta::instance::HashedString;
use or_io::meta::{Field, FieldType, StringKind, Value};

use super::ctx::EditorCtx;
use super::widgets::{default_value, draw_field};
use crate::ui::theme;

/// The key `text` spells for a map whose keys are `key`; none where it spells none.
pub(super) fn typed_key(key: &FieldType, text: &str) -> Option<Value> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let number = || match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => text.parse::<u64>().ok(),
    };
    match key {
        FieldType::String(StringKind::Plain | StringKind::AtString) => {
            Some(Value::String(text.to_string()))
        }
        FieldType::String(_) => Some(Value::Hash(HashedString::from_xml_text(text))),
        FieldType::U8 | FieldType::U16 | FieldType::U32 | FieldType::U64 => {
            number().map(Value::UInt)
        }
        FieldType::S8 | FieldType::S16 | FieldType::S32 | FieldType::S64 => {
            text.parse::<i64>().ok().map(Value::Int)
        }
        FieldType::Enum(name) => Some(Value::Enum {
            type_name: name.clone(),
            variant: text.to_string(),
        }),
        _ => None,
    }
}

/// A key as the form shows it.
fn key_text(key: &Value) -> String {
    match key {
        Value::Hash(h) => h.to_xml_text(),
        Value::String(s) => s.clone(),
        Value::UInt(v) => v.to_string(),
        Value::Int(v) => v.to_string(),
        Value::Enum { variant, .. } => variant.clone(),
        other => format!("{other:?}"),
    }
}

/// `pairs` with a pair of key `key` and value `of`'s default added last; the map as it was, or
/// none where a pair already holds that key.
pub(super) fn add_pair(
    pairs: &mut Vec<(Value, Value)>,
    key: Value,
    of: &FieldType,
) -> Option<Value> {
    if pairs.iter().any(|(k, _)| *k == key) {
        return None;
    }
    let before = Value::Map(pairs.clone());
    pairs.push((key, default_value(of)));
    Some(before)
}

/// The map `value` of `field`, whose keys are `key` and whose values are `of`.
pub(super) fn draw_map(
    ui: &mut egui::Ui,
    ctx: &EditorCtx,
    field: &Field,
    (key, of): (&FieldType, &FieldType),
    value: &mut Value,
    dirty: &mut bool,
) {
    let Value::Map(pairs) = value else {
        ui.label(
            egui::RichText::new(tr!("(the file holds no map here)"))
                .small()
                .color(theme::TEXT_DIM),
        );
        return;
    };
    let typed_id = ui.id().with(("meta_map_new_key", field.name.as_str()));
    let mut text: String = ui.data(|d| d.get_temp(typed_id)).unwrap_or_default();
    let typed = typed_key(key, &text);
    let free = typed
        .as_ref()
        .is_some_and(|k| pairs.iter().all(|(held, _)| held != k));
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(tr!("Pairs: {count}", count = pairs.len()))
                    .small()
                    .color(theme::TEXT_DIM),
            );
            ui.label(tr!("New pair's key"));
            let response = ui.add(
                egui::TextEdit::singleline(&mut text)
                    .desired_width(160.0)
                    .font(egui::TextStyle::Monospace),
            );
            if response.changed() {
                ui.data_mut(|d| d.insert_temp(typed_id, text.clone()));
            }
            let add = format!("{} {}", ph::PLUS, tr!("Add"));
            if ui.add_enabled(free, egui::Button::new(add)).clicked() {
                if let Some(before) = typed.and_then(|k| add_pair(pairs, k, of)) {
                    ctx.changed(before);
                    ui.data_mut(|d| d.remove::<String>(typed_id));
                    *dirty = true;
                }
            }
        });
        let value_field = Field {
            ty: of.clone(),
            ..field.clone()
        };
        // Edits inside a pair are kept apart, and recorded as the whole map they changed.
        let inner = EditorCtx {
            edits: Rc::new(RefCell::new(Vec::new())),
            ..ctx.clone()
        };
        let mut removed = None;
        let mut edited = None;
        for (i, (k, v)) in pairs.iter_mut().enumerate() {
            ui.push_id(("meta_map_pair", i), |ui| {
                let header_id = ui.make_persistent_id(("meta_map_hdr", i));
                let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
                    ui.ctx(),
                    header_id,
                    false,
                );
                let header = ui.horizontal(|ui| {
                    state.show_toggle_button(ui, egui::collapsing_header::paint_default_icon);
                    ui.label(egui::RichText::new(key_text(k)).monospace());
                    super::text_names::draw(ui, ctx, k);
                    let summary = super::widgets::item_summary(v);
                    if !summary.is_empty() {
                        ui.label(egui::RichText::new(summary).small().color(theme::TEXT_DIM));
                    }
                    let delete = ui
                        .small_button(egui::RichText::new(ph::TRASH))
                        .on_hover_text(tr!("Delete this pair"));
                    if delete.clicked() {
                        removed = Some(i);
                    }
                });
                state.show_body_indented(&header.response, ui, |ui| {
                    let held = v.clone();
                    let mut changed = false;
                    draw_field(ui, &inner, &value_field, v, &mut changed);
                    if changed {
                        edited = Some((i, held));
                    }
                });
            });
        }
        if let Some((i, held)) = edited {
            let mut before = pairs.clone();
            before[i].1 = held;
            ctx.changed(Value::Map(before));
            *dirty = true;
        }
        if let Some(i) = removed {
            ctx.changed(Value::Map(pairs.clone()));
            pairs.remove(i);
            *dirty = true;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_key_is_a_hash_a_name_a_number_or_nothing_by_the_map_s_key_type() {
        let hash = FieldType::String(StringKind::AtHashValue);
        assert_eq!(
            typed_key(&hash, " hash_0000BEEF "),
            Some(Value::Hash(HashedString::from_hash(0xBEEF)))
        );
        assert_eq!(
            typed_key(&hash, "player"),
            Some(Value::Hash(HashedString::new("player")))
        );
        assert_eq!(typed_key(&FieldType::U32, "0x10"), Some(Value::UInt(16)));
        assert_eq!(typed_key(&FieldType::U32, "12"), Some(Value::UInt(12)));
        assert_eq!(typed_key(&FieldType::U32, "twelve"), None);
        assert_eq!(typed_key(&hash, "   "), None);
        assert_eq!(typed_key(&FieldType::Vector3, "1"), None);
    }

    #[test]
    fn a_pair_is_added_last_with_its_value_s_default_and_a_held_key_is_refused() {
        let key = |h: u32| Value::Hash(HashedString::from_hash(h));
        let mut pairs = vec![(key(1), Value::UInt(5))];
        let before = add_pair(&mut pairs, key(2), &FieldType::U32).expect("added");
        assert_eq!(before, Value::Map(vec![(key(1), Value::UInt(5))]));
        assert_eq!(pairs, [(key(1), Value::UInt(5)), (key(2), Value::UInt(0))]);
        assert_eq!(add_pair(&mut pairs, key(1), &FieldType::U32), None);
        assert_eq!(pairs.len(), 2);
    }

    fn map_field() -> Field {
        Field {
            name: "entities".into(),
            ty: FieldType::Map {
                key: Box::new(FieldType::String(StringKind::AtHashValue)),
                value: Box::new(FieldType::U32),
            },
            init: None,
            min: None,
            max: None,
            step: None,
            ui_key: false,
            hide_widgets: false,
            description: None,
            hint: or_io::meta::FieldHint::None,
        }
    }

    /// The shapes the form draws of a map of one pair whose key hashes `key`, and whether it changed.
    fn drawn(ctx: &EditorCtx, key: u32) -> (String, bool) {
        let field = map_field();
        let mut value = Value::Map(vec![(
            Value::Hash(HashedString::from_hash(key)),
            Value::UInt(3),
        )]);
        let mut dirty = false;
        let output = egui::Context::default().run_ui(Default::default(), |ui| {
            let FieldType::Map { key, value: of } = &field.ty else {
                return;
            };
            draw_map(ui, ctx, &field, (&**key, &**of), &mut value, &mut dirty);
        });
        (format!("{:?}", output.shapes), dirty)
    }

    #[test]
    fn the_form_draws_a_map_s_count_and_each_pair_s_key_and_records_nothing_unclicked() {
        let _serial = crate::ui::theme::serial();
        let ctx = EditorCtx::for_test();
        let (shapes, dirty) = drawn(&ctx, 0xABCD_0001);
        assert!(shapes.contains("Pairs: 1"), "{shapes}");
        assert!(shapes.contains("hash_ABCD0001"), "{shapes}");
        assert!(!dirty);
        assert!(ctx.edits.borrow().is_empty());
    }

    #[test]
    fn a_pair_s_hash_key_the_document_s_strings_name_shows_that_name() {
        let _serial = crate::ui::theme::serial();
        let text = "zq_map_pair_key";
        let mut ctx = EditorCtx::for_test();
        let document = super::super::text_names::tests::document(text);
        ctx.strings = std::sync::Arc::new(crate::state::DocumentStrings::of(&document, || None));
        let (shapes, _) = drawn(&ctx, or_io::string_hash(text.as_bytes()));
        assert!(shapes.contains(text), "{shapes}");
        let (shapes, _) = drawn(&ctx, or_io::string_hash(text.as_bytes()) ^ 1);
        assert!(!shapes.contains(text), "{shapes}");
    }
}
