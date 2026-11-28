//! Reading a metadata field by the hash it is stored under.
//!
//! ⚠ **A field's spelling in a [`MetaInstance`] depends on whether a schema
//! snapshot is staged**, so no world type may key on one. The reader resolves
//! a name where it can and falls back to the literal `hash_HHHHHHHH`, which
//! means the same `.ymap` presents `entities` on one machine and
//! `hash_CCAB9307` on another. Everything here keys on the number both spellings
//! stand for.

use or_io::hash::literal_string_hash;
use or_io::meta::instance::value::HashedString;
use or_io::meta::{MetaInstance, Value};

/// The hash a field name stands for, whichever way the reader spelt it.
pub fn name_hash(field_name: &str) -> u32 {
    or_io::meta::binary::synth::name_hash(field_name)
}

/// The value stored under `hash`, whatever the field is called.
pub fn get(inst: &MetaInstance, hash: u32) -> Option<&Value> {
    inst.fields
        .iter()
        .find(|(name, _)| name_hash(name) == hash)
        .map(|(_, value)| value)
}

/// The value stored under `hash`, for a caller about to change it.
pub fn get_mut(inst: &mut MetaInstance, hash: u32) -> Option<&mut Value> {
    inst.fields
        .iter_mut()
        .find(|(name, _)| name_hash(name) == hash)
        .map(|(_, value)| value)
}

/// Whether this instance is of the named type, whichever way it was spelt.
pub fn is_type(inst: &MetaInstance, name: &str) -> bool {
    name_hash(&inst.type_name) == literal_string_hash(name.as_bytes())
}

pub fn f32_at(inst: &MetaInstance, hash: u32) -> Option<f32> {
    match get(inst, hash)? {
        Value::Float(v) => Some(*v as f32),
        Value::Int(v) => Some(*v as f32),
        Value::UInt(v) => Some(*v as f32),
        _ => None,
    }
}

pub fn u32_at(inst: &MetaInstance, hash: u32) -> Option<u32> {
    match get(inst, hash)? {
        Value::UInt(v) => Some(*v as u32),
        Value::Int(v) => Some(*v as u32),
        Value::Bitset { bits, .. } => Some(*bits as u32),
        Value::Color(v) => Some(*v),
        _ => None,
    }
}

pub fn i32_at(inst: &MetaInstance, hash: u32) -> Option<i32> {
    match get(inst, hash)? {
        Value::Int(v) => Some(*v as i32),
        Value::UInt(v) => Some(*v as i32),
        _ => None,
    }
}

pub fn vec3_at(inst: &MetaInstance, hash: u32) -> Option<[f32; 3]> {
    match get(inst, hash)? {
        Value::Vec3(v) => Some(*v),
        Value::Vec4(v) => Some([v[0], v[1], v[2]]),
        _ => None,
    }
}

pub fn vec4_at(inst: &MetaInstance, hash: u32) -> Option<[f32; 4]> {
    match get(inst, hash)? {
        Value::Vec4(v) => Some(*v),
        Value::Vec3(v) => Some([v[0], v[1], v[2], 0.0]),
        _ => None,
    }
}

/// The hash a `Hash` or `String` field carries.
///
/// ⚠ A `HashedString`'s `raw` is filled from the name index, so it is not a
/// reliable preimage of its own number and must not be re-hashed. Read
/// [`HashedString::hash`].
pub fn hash_at(inst: &MetaInstance, hash: u32) -> Option<u32> {
    match get(inst, hash)? {
        Value::Hash(h) => Some(h.hash),
        Value::UInt(v) => Some(*v as u32),
        Value::String(s) if !s.is_empty() => Some(or_io::string_hash(s.as_bytes())),
        _ => None,
    }
}

/// The text of a `String` field, or a named `Hash`'s name.
pub fn string_at(inst: &MetaInstance, hash: u32) -> Option<String> {
    match get(inst, hash)? {
        Value::String(s) => Some(s.clone()),
        Value::Hash(HashedString { raw, .. }) if !raw.is_empty() => Some(raw.clone()),
        _ => None,
    }
}

pub fn bool_at(inst: &MetaInstance, hash: u32) -> Option<bool> {
    match get(inst, hash)? {
        Value::Bool(b) => Some(*b),
        Value::UInt(v) => Some(*v != 0),
        Value::Int(v) => Some(*v != 0),
        _ => None,
    }
}

/// The elements of an array field, or an empty slice when it is absent.
///
/// Absent and empty are the same answer: a `.ymap` whose `carGenerators` the
/// reader could not resolve has no car generators, like one with none.
pub fn array_at(inst: &MetaInstance, hash: u32) -> &[Value] {
    match get(inst, hash) {
        Some(Value::Array(items)) => items,
        _ => &[],
    }
}

/// The struct instances in an array field, following pointers.
pub fn structs_at(inst: &MetaInstance, hash: u32) -> impl Iterator<Item = &MetaInstance> {
    array_at(inst, hash).iter().filter_map(as_struct)
}

/// Whether every item of an array field is a struct, so a place among [`structs_at`]'s is a place in the array.
// ⚠ A lenient read leaves a pointer it could not follow null and a member it could not decode raw.
pub fn structs_fill(inst: &MetaInstance, hash: u32) -> bool {
    match get(inst, hash) {
        Some(Value::Array(items)) => items.iter().all(|item| as_struct(item).is_some()),
        Some(Value::Raw(_)) => false,
        _ => true,
    }
}

/// The instance a value holds, whether inline or behind a pointer.
pub fn as_struct(value: &Value) -> Option<&MetaInstance> {
    match value {
        Value::Struct(inner) => Some(inner),
        Value::Pointer(Some(inner)) => Some(inner),
        _ => None,
    }
}

/// The instance a struct-valued field holds.
pub fn struct_at(inst: &MetaInstance, hash: u32) -> Option<&MetaInstance> {
    as_struct(get(inst, hash)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_field_reads_under_either_spelling() {
        let entities = literal_string_hash(b"entities");

        let mut named = MetaInstance::new("CMapData");
        named.set("entities", Value::Array(vec![Value::Int(1)]));

        let mut numbered = MetaInstance::new("hash_D3593FA6");
        numbered.set(
            format!("hash_{entities:08X}"),
            Value::Array(vec![Value::Int(1)]),
        );

        assert_eq!(array_at(&named, entities).len(), 1);
        assert_eq!(array_at(&numbered, entities).len(), 1);
        assert!(is_type(&named, "CMapData"));
        assert!(is_type(&numbered, "CMapData"));
    }

    #[test]
    fn a_literal_spelling_is_not_rehashed() {
        assert_eq!(name_hash("0xCCAB9307"), 0xCCAB_9307);
        assert_ne!(
            literal_string_hash(b"0xCCAB9307"),
            0xCCAB_9307,
            "if these agreed the test could not tell the two paths apart"
        );
    }

    #[test]
    fn only_an_eight_digit_literal_is_read_as_a_number() {
        assert_eq!(
            name_hash("0xCCAB93070"),
            literal_string_hash(b"0xCCAB93070")
        );
        assert_eq!(name_hash("0xnothex12"), literal_string_hash(b"0xnothex12"));
    }

    #[test]
    fn a_missing_array_reads_as_empty_rather_than_absent() {
        let inst = MetaInstance::new("CMapData");
        assert!(array_at(&inst, literal_string_hash(b"carGenerators")).is_empty());
    }

    #[test]
    fn a_hash_field_reports_its_stored_number_not_its_texts() {
        let mut inst = MetaInstance::new("CEntityDef");
        inst.set(
            "archetypeName",
            Value::Hash(HashedString {
                raw: "position".into(),
                hash: 0x1234_5678,
            }),
        );
        let key = literal_string_hash(b"archetypeName");
        assert_eq!(hash_at(&inst, key), Some(0x1234_5678));
    }
}
