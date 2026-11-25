//! The elements of a state's run that are not placed in the world: a row per element by its key,
//! opening into its value's members and everything below them, as the body declares them.

use std::sync::{Arc, Mutex};

use or_io::gta6::mission_other::{Declared, OtherElement};
use or_io::gta6::mission_tables::StateRecord;
use or_io::gta6::names::Vocabulary;
use or_io::meta::binary::hash_index::default_hash_index;
use or_io::meta::Value;

use super::*;
use crate::state::gta6_names;

/// An element with its key label computed once, so the per-frame draw neither decodes nor looks a
/// name up.
struct OtherRow {
    key: String,
    element: OtherElement,
}

type Decoded = Arc<Result<Vec<OtherRow>, String>>;

/// The one run last opened, decoded once with its key labels; re-decoded only when the open run,
/// the document's form (its bytes) or the vocabulary served changes. One run is open at a time, so
/// a single slot suffices.
// ponytail: single-entry cache keyed by the form's bytes, the record and the vocabulary; two runs
// open at once re-decode by turns, as CURRENT does for the form.
#[allow(clippy::type_complexity)]
static DECODED: Mutex<Option<((usize, usize, u32, Option<(usize, usize)>, usize), Decoded)>> =
    Mutex::new(None);

/// What an element's key is shown as: the name `vocabulary` holds for it, else the metadata names'
/// for a narrow key, else its digits.
fn key_label(vocabulary: &Vocabulary, wide: bool, key: u64) -> String {
    if wide {
        return vocabulary
            .name64(key)
            .map_or_else(|| format!("{key:016X}"), str::to_owned);
    }
    // A narrow key came from four bytes, so it is whole as a u32.
    let narrow = key as u32;
    let held = vocabulary.name32(narrow).map(str::to_owned);
    held.unwrap_or_else(|| default_hash_index().name_or_literal(narrow))
}

/// `record`'s run decoded, from the cache where the form, the record and the vocabulary are the
/// ones last decoded with.
fn decoded(bytes: &[u8], body: &Body, record: &StateRecord) -> Decoded {
    let vocabulary = gta6_names::vocabulary();
    let key = (
        bytes.as_ptr() as usize,
        bytes.len(),
        record.key,
        record.run,
        Arc::as_ptr(&vocabulary) as usize,
    );
    let mut kept = DECODED.lock().unwrap_or_else(|p| p.into_inner());
    if let Some((cached_key, decoded)) = kept.as_ref() {
        if *cached_key == key {
            return decoded.clone();
        }
    }
    let rows = Declared::open(bytes)
        .and_then(|declared| declared.elements(body, record))
        .map(|elements| {
            (elements.into_iter())
                .map(|element| OtherRow {
                    key: key_label(&vocabulary, element.wide, element.key),
                    element,
                })
                .collect()
        })
        .map_err(|e| e.to_string());
    let decoded: Decoded = Arc::new(rows);
    *kept = Some((key, decoded.clone()));
    decoded
}

/// A row per element of `record`'s run in `bytes`, or why the run does not read.
pub(super) fn rows(
    ui: &mut egui::Ui,
    id: egui::Id,
    bytes: &[u8],
    body: &Body,
    record: &StateRecord,
) {
    let decoded = decoded(bytes, body, record);
    let rows = match decoded.as_ref() {
        Ok(rows) => rows,
        Err(e) => {
            ui.add(detail_label(tr!("The elements do not read: {e}", e = e)));
            return;
        }
    };
    for (k, row) in rows.iter().enumerate() {
        let element = &row.element;
        let id = id.with(("other", k));
        collapsing_row(
            ui,
            id,
            false,
            |ui, _| {
                ui.label(row.key.as_str());
                ui.add(detail_label(tr!(
                    "at {at}: {class}",
                    at = format!("{:#X}", element.at),
                    class = element.value.type_name
                )));
            },
            |ui| {
                for (n, (name, value)) in element.value.fields.iter().enumerate() {
                    value_row(ui, id.with(n), name, value);
                }
            },
        );
    }
}

/// A member's row: a leaf with its value, or a row that opens into what the value holds.
fn value_row(ui: &mut egui::Ui, id: egui::Id, name: &str, value: &Value) {
    let below: Vec<(String, &Value)> = match value {
        Value::Struct(inner) | Value::Pointer(Some(inner)) => {
            (inner.fields.iter()).map(|(n, v)| (n.clone(), v)).collect()
        }
        Value::Array(items) => (items.iter().enumerate())
            .map(|(i, v)| (format!("[{i}]"), v))
            .collect(),
        Value::Map(pairs) => (pairs.iter()).map(|(key, v)| (text(key), v)).collect(),
        _ => Vec::new(),
    };
    let header = |ui: &mut egui::Ui| {
        ui.label(name);
        ui.add(detail_label(text(value)));
    };
    if below.is_empty() {
        leaf_row(ui, header);
        return;
    }
    collapsing_row(
        ui,
        id,
        false,
        |ui, _| header(ui),
        |ui| {
            for (k, (name, value)) in below.iter().enumerate() {
                value_row(ui, id.with(k), name, value);
            }
        },
    );
}

/// Lanes as the outline writes them.
fn floats(lanes: &[f32]) -> String {
    (lanes.iter())
        .map(|c| format!("{c:.3}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A value as one line: a structure by its type, an array or a map by its count.
fn text(value: &Value) -> String {
    match value {
        Value::Null | Value::Pointer(None) => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Int(n) => n.to_string(),
        Value::UInt(n) => n.to_string(),
        Value::Float(f) => format!("{f:.3}"),
        Value::String(s) | Value::Raw(s) => s.clone(),
        Value::Hash(h) if h.raw.is_empty() => gta6_names::said32(h.hash),
        Value::Hash(h) => h.raw.clone(),
        Value::Vec2(v) => floats(v),
        Value::Vec3(v) => floats(v),
        Value::Vec4(v) => floats(v),
        Value::Mat34(rows) => (rows.iter())
            .map(|row| floats(&row[..]))
            .collect::<Vec<_>>()
            .join("; "),
        Value::Color(argb) => format!("{argb:#010X}"),
        Value::Enum { variant, .. } => variant.clone(),
        Value::Bitset { bits, .. } => format!("{bits:#X}"),
        Value::ByteArray(bytes) => bytes.iter().map(|b| format!("{b:02X}")).collect(),
        Value::Struct(inner) | Value::Pointer(Some(inner)) => inner.type_name.clone(),
        Value::Array(items) => tr_n!("{count} item", "{count} items", items.len()),
        Value::Map(pairs) => tr_n!("{count} pair", "{count} pairs", pairs.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::meta::MetaInstance;

    #[test]
    fn a_value_reads_as_one_line_and_a_structure_opens_into_its_members() {
        let mut inner = MetaInstance::new("hash_11110000");
        inner.fields.push(("hash_22220000".into(), Value::UInt(7)));
        let structure = Value::Struct(Box::new(inner));
        assert_eq!(text(&Value::Vec3([1.0, 2.5, -3.0])), "1.000 2.500 -3.000");
        assert_eq!(text(&Value::Bool(true)), "true");
        assert_eq!(text(&Value::ByteArray(vec![0xAB, 0x01])), "AB01");
        assert_eq!(text(&Value::Pointer(None)), "null");
        assert_eq!(text(&structure), "hash_11110000");
        assert_eq!(
            text(&Value::Bitset {
                type_name: String::new(),
                bits: 0x30
            }),
            "0x30"
        );
    }

    #[test]
    fn an_element_s_key_is_named_by_the_vocabulary_else_shown_by_its_digits() {
        let mut vocabulary = Vocabulary::default();
        assert!(vocabulary.add("zq_mission_marker", None));
        let narrow = u64::from(or_io::gta6::hash::name_hash32(b"zq_mission_marker"));
        let wide = or_io::gta6::hash::name_hash64(b"zq_mission_marker");
        assert_eq!(key_label(&vocabulary, false, narrow), "zq_mission_marker");
        assert_eq!(key_label(&vocabulary, true, wide), "zq_mission_marker");
        let none = Vocabulary::default();
        assert_eq!(key_label(&none, true, 0xAB), "00000000000000AB");
        assert_eq!(
            key_label(&none, false, narrow),
            default_hash_index().name_or_literal(narrow as u32)
        );
    }
}
