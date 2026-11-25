//! The elements of a state's run that are not placed in the world: a line each of its key and
//! its value's members, everything below them included, as the body declares them.

use or_io::gta6::mission_other::Declared;
use or_io::gta6::mission_tables::{Body, StateRecord};
use or_io::gta6::names::Vocabulary;
use or_io::meta::Value;

use super::said32;

/// A line per element of `record`'s run, or one that says why the run does not read.
pub(super) fn lines(
    bytes: &[u8],
    body: &Body,
    record: &StateRecord,
    vocabulary: &Vocabulary,
) -> Vec<String> {
    match Declared::open(bytes).and_then(|declared| declared.elements(body, record)) {
        Ok(elements) => (elements.iter())
            .map(|e| {
                let width: usize = if e.wide { 18 } else { 10 };
                format!(
                    "    element at {:#X}: key {:#0width$X}{}, {} {}",
                    e.at,
                    e.key,
                    key_said(vocabulary, e.wide, e.key),
                    e.value.type_name,
                    members(vocabulary, &e.value.fields)
                )
            })
            .collect(),
        Err(e) => vec![format!("    the elements do not read: {e}")],
    }
}

/// ` (name)` where `vocabulary` holds the name an element's key states, else nothing.
fn key_said(vocabulary: &Vocabulary, wide: bool, key: u64) -> String {
    match wide {
        true => (vocabulary.name64(key)).map_or_else(String::new, |name| format!(" ({name})")),
        // A narrow key came from four bytes, so it is whole as a u32.
        false => said32(vocabulary, key as u32),
    }
}

fn members(vocabulary: &Vocabulary, fields: &[(String, Value)]) -> String {
    let shown: Vec<String> = (fields.iter())
        .map(|(name, value)| format!("{name}={}", show(vocabulary, value)))
        .collect();
    format!("{{{}}}", shown.join(", "))
}

fn floats(lanes: &[f32]) -> String {
    (lanes.iter())
        .map(|c| format!("{c:.3}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A value as the line writes it, everything below it included.
fn show(vocabulary: &Vocabulary, value: &Value) -> String {
    let show = |value: &Value| show(vocabulary, value);
    match value {
        Value::Null | Value::Pointer(None) => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Int(n) => n.to_string(),
        Value::UInt(n) => n.to_string(),
        Value::Float(f) => format!("{f:.3}"),
        Value::String(s) | Value::Raw(s) => s.clone(),
        Value::Hash(h) if h.raw.is_empty() => {
            format!("{:#010X}{}", h.hash, said32(vocabulary, h.hash))
        }
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
        Value::Struct(inner) | Value::Pointer(Some(inner)) => members(vocabulary, &inner.fields),
        Value::Array(items) => {
            format!(
                "[{}]",
                items.iter().map(show).collect::<Vec<_>>().join(", ")
            )
        }
        Value::Map(pairs) => {
            let shown: Vec<String> = (pairs.iter())
                .map(|(key, v)| format!("{}: {}", show(key), show(v)))
                .collect();
            format!("{{{}}}", shown.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::meta::MetaInstance;

    #[test]
    fn a_value_is_written_with_everything_below_it() {
        let mut inner = MetaInstance::new("hash_11110000");
        inner.fields.push(("a".into(), Value::UInt(7)));
        let list = vec![Value::Bool(true), Value::Vec3([1.0, 2.5, 0.0])];
        inner.fields.push(("b".into(), Value::Array(list)));
        inner.fields.push(("c".into(), Value::Pointer(None)));
        let none = Vocabulary::default();
        assert_eq!(
            show(&none, &Value::Struct(Box::new(inner))),
            "{a=7, b=[true, 1.000 2.500 0.000], c=null}"
        );
        assert_eq!(show(&none, &Value::ByteArray(vec![0xAB, 0x01])), "AB01");
        assert_eq!(show(&none, &Value::Map(Vec::new())), "{}");
    }

    #[test]
    fn a_key_and_a_hash_are_named_by_the_vocabulary_beside_their_digits() {
        use or_io::gta6::hash::{name_hash32, name_hash64};
        let mut vocabulary = Vocabulary::default();
        assert!(vocabulary.add("zq_mission_key", None));
        let narrow = name_hash32(b"zq_mission_key");
        let wide = name_hash64(b"zq_mission_key");
        assert_eq!(
            key_said(&vocabulary, false, u64::from(narrow)),
            " (zq_mission_key)"
        );
        assert_eq!(key_said(&vocabulary, true, wide), " (zq_mission_key)");
        assert_eq!(key_said(&Vocabulary::default(), true, wide), "");
        let hash = Value::Hash(or_io::meta::instance::HashedString {
            hash: narrow,
            raw: String::new(),
        });
        assert_eq!(
            show(&vocabulary, &hash),
            format!("{narrow:#010X} (zq_mission_key)")
        );
    }
}
