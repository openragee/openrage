#![allow(
    clippy::manual_pattern_char_comparison,
    clippy::manual_strip,
    clippy::collapsible_if,
    clippy::collapsible_match
)]

use quick_xml::escape::{resolve_predefined_entity, unescape};
use quick_xml::events::attributes::Attributes;
use quick_xml::events::{BytesRef, Event};
use quick_xml::name::QName;
use quick_xml::Reader;

use crate::meta::error::{MetaError, Result};
use crate::meta::schema::library::hash_spelling;
use crate::meta::schema::{
    normalize_member_name, ArrayKind, BitsetStorage, EnumDef, Field as SchemaField, FieldType,
    ResolvedStruct, SchemaLibrary, StringKind, StructDef, TypeName,
};

use super::value::{HashedString, MetaInstance, UnknownNode, Value};

pub fn parse_meta(library: &SchemaLibrary, bytes: &[u8]) -> Result<MetaInstance> {
    let mut reader = Reader::from_reader(bytes);
    let cfg = reader.config_mut();
    cfg.trim_text(false);
    cfg.expand_empty_elements = false;

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => {
                return Err(MetaError::Other(
                    "no root element found in XML document".into(),
                ));
            }
            Event::Start(e) => {
                let tag = local_name(e.name());
                let resolved = library
                    .resolve_any(&tag)
                    .ok_or_else(|| MetaError::UnknownType(tag.clone()))?;
                let canonical = kept_name(&resolved, &tag);
                return read_struct_body(
                    &mut reader,
                    library,
                    &resolved,
                    &tag,
                    &canonical,
                    e.attributes(),
                );
            }
            Event::Empty(e) => {
                let tag = local_name(e.name());
                let resolved = library
                    .resolve_any(&tag)
                    .ok_or_else(|| MetaError::UnknownType(tag.clone()))?;
                let mut inst = MetaInstance::new(kept_name(&resolved, &tag));
                apply_attribute_fields(&mut inst, &resolved, e.attributes())?;
                return Ok(inst);
            }
            _ => {}
        }
        buf.clear();
    }
}

/// XML `bytes` typed by `library`, or as the tree they state with the structure `library` lacks.
pub fn parse_typed_or_tree(
    library: &SchemaLibrary,
    bytes: &[u8],
) -> Result<(MetaInstance, Option<String>)> {
    match parse_meta(library, bytes) {
        Err(MetaError::UnknownType(name)) => {
            super::schemaless::parse_schemaless(bytes).map(|tree| (tree, Some(name)))
        }
        read => read.map(|typed| (typed, None)),
    }
}

fn read_struct_body(
    reader: &mut Reader<&[u8]>,
    library: &SchemaLibrary,
    resolved: &ResolvedStruct<'_>,
    close_tag: &str,
    type_name: &str,
    attrs: Attributes,
) -> Result<MetaInstance> {
    let mut inst = MetaInstance::new(type_name);
    apply_attribute_fields(&mut inst, resolved, attrs)?;

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::End(e) if local_name(e.name()) == close_tag => break,
            Event::Start(e) => {
                let tag = local_name(e.name());
                if let Some((field, name)) =
                    lookup_field(resolved, &tag).filter(|_| !is_a_reference(e.attributes()))
                {
                    let value = read_field_open(reader, library, &field.ty, &tag, e.attributes())?;
                    inst.fields.push((name, value));
                } else {
                    let extra = read_unknown_open(reader, &tag, e.attributes())?;
                    inst.extras.push(extra);
                }
            }
            Event::Empty(e) => {
                let tag = local_name(e.name());
                if let Some((field, name)) =
                    lookup_field(resolved, &tag).filter(|_| !is_a_reference(e.attributes()))
                {
                    let value = read_field_empty(library, &field.ty, e.attributes())?;
                    inst.fields.push((name, value));
                } else {
                    let extra = read_unknown_empty(&tag, e.attributes())?;
                    inst.extras.push(extra);
                }
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(inst)
}

/// Whether an element names an item by `ref` (the pointer of an `external_named` policy), which the value tree has no variant for and an unknown node keeps whole.
fn is_a_reference(attrs: Attributes) -> bool {
    attrs.flatten().any(|a| local_name(a.key) == "ref")
}

/// The member `xml_tag` names and the name the instance keeps for it: the library's, or the XML's
/// spelling where the library holds the member only as its hash, which a save must write.
fn lookup_field<'a>(
    resolved: &'a ResolvedStruct<'_>,
    xml_tag: &str,
) -> Option<(&'a SchemaField, String)> {
    let key = normalize_member_name(xml_tag);
    let mut fields = resolved.fields.iter().copied();
    if let Some(f) = fields.clone().find(|f| f.name == key || f.name == xml_tag) {
        return Some((f, f.name.clone()));
    }
    let hashed = hash_spelling(xml_tag);
    fields
        .find(|f| f.name == hashed)
        .map(|f| (f, xml_tag.to_string()))
}

/// The type name an instance of `resolved` keeps: the library's, or `spelling` where the library
/// holds the structure only as its hash.
fn kept_name(resolved: &ResolvedStruct<'_>, spelling: &str) -> String {
    if resolved.def.name == hash_spelling(spelling) {
        spelling.to_string()
    } else {
        resolved.def.name.clone()
    }
}

/// The members an instance's tree holds typed, and the elements it keeps untyped, as text.
pub fn member_counts(inst: &MetaInstance) -> (usize, usize) {
    fn value(v: &Value, counts: &mut (usize, usize)) {
        match v {
            Value::Struct(inner) | Value::Pointer(Some(inner)) => instance(inner, counts),
            Value::Array(items) => items.iter().for_each(|i| value(i, counts)),
            Value::Map(pairs) => pairs.iter().for_each(|(k, v)| {
                value(k, counts);
                value(v, counts);
            }),
            _ => {}
        }
    }
    fn instance(inst: &MetaInstance, counts: &mut (usize, usize)) {
        counts.0 += inst.fields.len();
        counts.1 += inst.extras.len();
        inst.fields.iter().for_each(|(_, v)| value(v, counts));
    }
    let mut counts = (0, 0);
    instance(inst, &mut counts);
    counts
}

fn read_field_open(
    reader: &mut Reader<&[u8]>,
    library: &SchemaLibrary,
    ty: &FieldType,
    tag: &str,
    attrs: Attributes,
) -> Result<Value> {
    match ty {
        FieldType::Array { elem, .. } => read_array(reader, library, elem.as_ref(), tag),
        FieldType::Map { key, value } => {
            read_map(reader, library, key.as_ref(), value.as_ref(), tag)
        }
        FieldType::Struct(name) => {
            let resolved = library
                .resolve_any(name)
                .ok_or_else(|| MetaError::UnknownType(name.clone()))?;
            let inner =
                read_struct_body(reader, library, &resolved, tag, &resolved.def.name, attrs)?;
            Ok(Value::Struct(Box::new(inner)))
        }
        FieldType::Pointer { base, .. } => {
            // ⚠ The attribute is taken as written. Converting `A__B` to `A::B`
            // here and then asking `resolve_any` is a one-way trip: its own
            // fallback converts in the same direction, so a library that keys
            // the type under the spelling the writer emitted is never reached.
            let mut concrete = base.clone();
            for a in attrs.clone() {
                let a = a?;
                if local_name(a.key) == "type" {
                    concrete = decode_attr(&a.value);
                }
            }
            if concrete.is_empty() {
                consume_to_close(reader, tag)?;
                return Ok(Value::Pointer(None));
            }
            let resolved = library
                .resolve_any(&concrete)
                .ok_or_else(|| MetaError::UnknownType(concrete.clone()))?;
            let kept = kept_name(&resolved, &concrete);
            let inner = read_struct_body(reader, library, &resolved, tag, &kept, attrs)?;
            Ok(Value::Pointer(Some(Box::new(inner))))
        }
        _ => {
            let mut text = String::new();
            let attr_value = attrs_value_attr(&attrs)?;
            let mut buf = Vec::new();
            loop {
                match reader.read_event_into(&mut buf)? {
                    Event::Eof => break,
                    Event::End(e) if local_name(e.name()) == tag => break,
                    Event::Text(t) => text.push_str(&decode_utf8(t.as_ref())),
                    Event::CData(t) => text.push_str(&decode_utf8(t.as_ref())),
                    Event::GeneralRef(t) => text.push_str(&resolve_general_ref(&t)),
                    _ => {}
                }
                buf.clear();
            }
            let raw = if !text.trim().is_empty() {
                text
            } else {
                attr_value.unwrap_or_default()
            };
            scalar_value(library, ty, &raw)
        }
    }
}

fn read_field_empty(library: &SchemaLibrary, ty: &FieldType, attrs: Attributes) -> Result<Value> {
    match ty {
        FieldType::Array { .. } => Ok(Value::Array(Vec::new())),
        FieldType::Map { .. } => Ok(Value::Map(Vec::new())),
        FieldType::Struct(_) => {
            let mut inst = MetaInstance::new("");
            if let FieldType::Struct(name) = ty {
                inst.type_name.clone_from(name);
            }
            Ok(Value::Struct(Box::new(inst)))
        }
        FieldType::Pointer { .. } => Ok(Value::Pointer(None)),
        _ => {
            let raw = attrs_value_attr(&attrs)?.unwrap_or_default();
            scalar_value(library, ty, &raw)
        }
    }
}

fn read_array(
    reader: &mut Reader<&[u8]>,
    library: &SchemaLibrary,
    elem: &FieldType,
    end_tag: &str,
) -> Result<Value> {
    let mut items = Vec::new();
    let mut text = String::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::End(e) if local_name(e.name()) == end_tag => break,
            Event::Start(e) => {
                let tag = local_name(e.name());
                let value = read_field_open(reader, library, elem, &tag, e.attributes())?;
                items.push(value);
            }
            Event::Empty(e) => {
                let value = read_field_empty(library, elem, e.attributes())?;
                items.push(value);
            }
            Event::Text(t) => text.push_str(&decode_utf8(t.as_ref())),
            _ => {}
        }
        buf.clear();
    }
    // ⚠ **A byte array is exported as text**, `<verts content="char_array">2 0
    // 140 …</verts>`, so an array with no child elements takes its items from
    // its text. Ignoring it emptied every `OccludeModel::verts` on re-import.
    if items.is_empty() && is_scalar(elem) {
        for token in text.split_ascii_whitespace() {
            items.push(scalar_value(library, elem, token)?);
        }
    }
    Ok(Value::Array(items))
}

fn is_scalar(ty: &FieldType) -> bool {
    matches!(
        ty,
        FieldType::Bool
            | FieldType::U8
            | FieldType::U16
            | FieldType::U32
            | FieldType::U64
            | FieldType::S8
            | FieldType::S16
            | FieldType::S32
            | FieldType::S64
            | FieldType::Float
            | FieldType::Float16
            | FieldType::Double
    )
}

fn read_map(
    reader: &mut Reader<&[u8]>,
    library: &SchemaLibrary,
    key_ty: &FieldType,
    val_ty: &FieldType,
    end_tag: &str,
) -> Result<Value> {
    let mut pairs: Vec<(Value, Value)> = Vec::new();
    let mut buf = Vec::new();
    let mut pending_key: Option<Value> = None;
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::End(e) if local_name(e.name()) == end_tag => break,
            Event::Start(e) => {
                let tag = local_name(e.name());
                let value = if pending_key.is_none() {
                    read_field_open(reader, library, key_ty, &tag, e.attributes())?
                } else {
                    read_field_open(reader, library, val_ty, &tag, e.attributes())?
                };
                if let Some(k) = pending_key.take() {
                    pairs.push((k, value));
                } else {
                    pending_key = Some(value);
                }
            }
            Event::Empty(e) => {
                let value = if pending_key.is_none() {
                    read_field_empty(library, key_ty, e.attributes())?
                } else {
                    read_field_empty(library, val_ty, e.attributes())?
                };
                if let Some(k) = pending_key.take() {
                    pairs.push((k, value));
                } else {
                    pending_key = Some(value);
                }
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(Value::Map(pairs))
}

fn read_unknown_open(
    reader: &mut Reader<&[u8]>,
    tag: &str,
    attrs: Attributes,
) -> Result<UnknownNode> {
    let mut node = UnknownNode {
        tag: tag.to_string(),
        ..UnknownNode::default()
    };
    for a in attrs {
        let a = a?;
        node.attrs.push((local_name(a.key), decode_attr(&a.value)));
    }
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            // ⚠ The elements inside are the node's own, kept in their order: reading them as its text would write them as text.
            Event::Start(e) => {
                let inner = local_name(e.name());
                node.children
                    .push(read_unknown_open(reader, &inner, e.attributes())?);
            }
            Event::Empty(e) => node
                .children
                .push(read_unknown_empty(&local_name(e.name()), e.attributes())?),
            Event::End(_) => break,
            Event::Text(t) => node.text.push_str(&decode_utf8(t.as_ref())),
            Event::CData(t) => node.text.push_str(&decode_utf8(t.as_ref())),
            Event::GeneralRef(t) => node.text.push_str(&resolve_general_ref(&t)),
            _ => {}
        }
        buf.clear();
    }
    // The white space between the elements is layout, not text.
    if !node.children.is_empty() && node.text.trim().is_empty() {
        node.text.clear();
    }
    Ok(node)
}

fn read_unknown_empty(tag: &str, attrs: Attributes) -> Result<UnknownNode> {
    let mut node = UnknownNode {
        tag: tag.to_string(),
        ..UnknownNode::default()
    };
    for a in attrs {
        let a = a?;
        node.attrs.push((local_name(a.key), decode_attr(&a.value)));
    }
    Ok(node)
}

fn consume_to_close(reader: &mut Reader<&[u8]>, end_tag: &str) -> Result<()> {
    let mut buf = Vec::new();
    let mut depth: i32 = 1;
    while depth > 0 {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => break,
            Event::Start(e) if local_name(e.name()) == end_tag => depth += 1,
            Event::End(e) if local_name(e.name()) == end_tag => depth -= 1,
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

fn apply_attribute_fields(
    inst: &mut MetaInstance,
    resolved: &ResolvedStruct<'_>,
    attrs: Attributes,
) -> Result<()> {
    for a in attrs {
        let a = a?;
        let name = local_name(a.key);
        let key = normalize_member_name(&name);
        if let Some(field) = resolved
            .fields
            .iter()
            .find(|f| f.name == key || f.name == name)
        {
            let raw = decode_attr(&a.value);
            let value = scalar_value(&SchemaLibrary::default(), &field.ty, &raw)
                .unwrap_or(Value::String(raw));
            inst.fields.push((field.name.clone(), value));
        }
    }
    Ok(())
}

fn attrs_value_attr(attrs: &Attributes) -> Result<Option<String>> {
    const COMPONENTS: [&str; 8] = ["x", "y", "z", "w", "r", "g", "b", "a"];
    let mut components: [Option<String>; 8] = Default::default();
    let mut found_component = false;

    for a in attrs.clone() {
        let a = a?;
        let name = local_name(a.key);
        if name == "value" {
            return Ok(Some(decode_attr(&a.value)));
        }
        if let Some(i) = COMPONENTS.iter().position(|c| *c == name) {
            components[i] = Some(decode_attr(&a.value));
            found_component = true;
        }
    }

    if !found_component {
        return Ok(None);
    }
    let joined = components
        .iter()
        .flatten()
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    Ok(Some(joined))
}

fn scalar_value(library: &SchemaLibrary, ty: &FieldType, raw: &str) -> Result<Value> {
    let trimmed = raw.trim();
    match ty {
        FieldType::Bool => Ok(Value::Bool(matches!(
            trimmed.to_ascii_lowercase().as_str(),
            "true" | "1" | "yes"
        ))),
        FieldType::U8 | FieldType::U16 | FieldType::U32 | FieldType::U64 => {
            Ok(Value::UInt(parse_u64(trimmed)))
        }
        FieldType::S8 | FieldType::S16 | FieldType::S32 | FieldType::S64 => {
            Ok(Value::Int(parse_i64(trimmed)))
        }
        FieldType::Float | FieldType::Float16 | FieldType::Double => {
            Ok(Value::Float(parse_f64(trimmed)))
        }
        FieldType::String(StringKind::Plain) | FieldType::String(StringKind::AtString) => {
            Ok(Value::String(raw.to_string()))
        }
        FieldType::String(_) => Ok(Value::Hash(HashedString::from_xml_text(raw))),
        FieldType::Vector2 | FieldType::Vec2V => Ok(Value::Vec2(parse_vec_n::<2>(trimmed))),
        FieldType::Vector3 | FieldType::Vec3V => Ok(Value::Vec3(parse_vec_n::<3>(trimmed))),
        FieldType::Vector4 | FieldType::Vec4V => Ok(Value::Vec4(parse_vec_n::<4>(trimmed))),
        FieldType::Mat34V | FieldType::Matrix34 => {
            let v = parse_floats(trimmed);
            let mut m = [[0.0f32; 4]; 3];
            for (i, f) in v.into_iter().take(12).enumerate() {
                m[i / 4][i % 4] = f;
            }
            Ok(Value::Mat34(m))
        }
        FieldType::Color32 => Ok(Value::Color(parse_color(trimmed))),
        FieldType::Enum(name) => Ok(Value::Enum {
            type_name: name.clone(),
            variant: trimmed.to_string(),
        }),
        FieldType::Bitset { values, storage } => Ok(Value::Bitset {
            type_name: values.clone(),
            bits: parse_bitset(library, values, *storage, trimmed),
        }),
        FieldType::Pad(_) => Ok(Value::Null),
        FieldType::Unknown(_) => Ok(Value::Raw(raw.to_string())),
        FieldType::Array { .. } | FieldType::Map { .. } => Ok(Value::Raw(raw.to_string())),
        FieldType::Struct(name) => {
            let mut inst = MetaInstance::new(name.clone());
            if !raw.trim().is_empty() {
                inst.extras.push(UnknownNode {
                    tag: "value".into(),
                    text: raw.to_string(),
                    ..UnknownNode::default()
                });
            }
            Ok(Value::Struct(Box::new(inst)))
        }
        FieldType::Pointer { .. } => Ok(Value::Pointer(None)),
        FieldType::Mat33V | FieldType::Mat44V | FieldType::Matrix33 | FieldType::Matrix44 => {
            Ok(Value::Raw(raw.to_string()))
        }
    }
}

fn parse_u64(s: &str) -> u64 {
    if let Some(stripped) = s.strip_prefix("0x").or(s.strip_prefix("0X")) {
        u64::from_str_radix(stripped, 16).unwrap_or(0)
    } else {
        s.parse::<u64>()
            .unwrap_or_else(|_| s.parse::<i64>().map(|v| v as u64).unwrap_or(0))
    }
}

fn parse_i64(s: &str) -> i64 {
    if let Some(stripped) = s.strip_prefix("0x").or(s.strip_prefix("0X")) {
        i64::from_str_radix(stripped, 16).unwrap_or(0)
    } else {
        s.parse::<i64>().unwrap_or(0)
    }
}

fn parse_f64(s: &str) -> f64 {
    let t = s.trim_end_matches(|c: char| matches!(c, 'f' | 'F' | 'd' | 'D'));
    t.parse::<f64>().unwrap_or(0.0)
}

fn parse_floats(s: &str) -> Vec<f32> {
    s.split(|c: char| matches!(c, ' ' | ',' | '\t' | '\n' | '\r'))
        .filter(|p| !p.is_empty())
        .map(parse_f64)
        .map(|v| v as f32)
        .collect()
}

fn parse_vec_n<const N: usize>(s: &str) -> [f32; N] {
    let mut out = [0.0f32; N];
    for (i, v) in parse_floats(s).into_iter().take(N).enumerate() {
        out[i] = v;
    }
    out
}

fn parse_color(s: &str) -> u32 {
    if let Some(stripped) = s.strip_prefix("0x").or(s.strip_prefix("0X")) {
        return u32::from_str_radix(stripped, 16).unwrap_or(0);
    }
    if s.starts_with('#') {
        return u32::from_str_radix(&s[1..], 16).unwrap_or(0);
    }
    let parts: Vec<u32> = s
        .split(|c: char| matches!(c, ' ' | ',' | '\t'))
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.parse::<u32>().ok())
        .collect();
    // ⚠ A lone number is the packed word itself. A binary container stores a
    // `Color32` as a plain `u32`, so that is how a colour read from one is
    // exported, and reading it as anything else zeroes it on re-import.
    match parts.as_slice() {
        [r, g, b, a] => (a << 24) | (r << 16) | (g << 8) | b,
        [r, g, b] => 0xFF00_0000 | (r << 16) | (g << 8) | b,
        [packed] => *packed,
        _ => 0,
    }
}

fn parse_bitset(
    library: &SchemaLibrary,
    enum_name: &str,
    storage: BitsetStorage,
    raw: &str,
) -> u128 {
    let _ = storage;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return 0;
    }
    if let Some(stripped) = trimmed.strip_prefix("0x").or(trimmed.strip_prefix("0X")) {
        return u128::from_str_radix(stripped, 16).unwrap_or(0);
    }
    if let Ok(n) = trimmed.parse::<u128>() {
        return n;
    }
    let Some(enum_def) = library.enum_(enum_name) else {
        return 0;
    };
    let mut bits: u128 = 0;
    for token in trimmed.split(|c: char| matches!(c, ' ' | ',' | '\t' | '\n' | '\r' | '|')) {
        let t = token.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(idx) = enum_def.values.iter().position(|v| v.name == t) {
            bits |= bit_for(enum_def, idx);
        }
    }
    bits
}

fn bit_for(enum_def: &EnumDef, idx: usize) -> u128 {
    let v = enum_def.variant_value(idx);
    if (0..128).contains(&v) {
        1u128 << v
    } else {
        0
    }
}

pub(crate) fn local_name(name: QName) -> String {
    let raw = name.into_inner();
    let s = std::str::from_utf8(raw).unwrap_or("");
    match s.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => s.to_string(),
    }
}

pub(crate) fn decode_utf8(bytes: &[u8]) -> String {
    std::str::from_utf8(bytes).unwrap_or("").to_string()
}

/// ⚠ **An entity reference is a separate event, not part of the text.**
/// quick-xml 0.38 splits `&amp;` out of `Event::Text` into `Event::GeneralRef`
/// carrying only the name, so a loop that handles `Text` and `CData` receives
/// the surrounding characters and never the entity. `BytesText` has no
/// `unescape` in this version precisely because text can no longer hold one.
///
/// An entity neither quick-xml's predefined table nor a character reference
/// resolves is kept as its own source spelling: the writer can emit only the
/// four it escapes, so the case reaches here from hand-authored input, where
/// preserving `&nbsp;` costs a re-escape on the next write and dropping it
/// costs the value.
pub(crate) fn resolve_general_ref(entity: &BytesRef<'_>) -> String {
    let Ok(name) = entity.decode() else {
        return String::new();
    };
    if let Ok(Some(ch)) = entity.resolve_char_ref() {
        return ch.to_string();
    }
    match resolve_predefined_entity(&name) {
        Some(text) => text.to_string(),
        None => format!("&{name};"),
    }
}

/// ⚠ **Attribute values are the one place quick-xml leaves the escaping in**,
/// so they need the opposite treatment to text. An unresolvable entity fails
/// the whole value rather than itself, and a lenient reader keeps the source
/// text over losing the rest of the string with it.
pub(crate) fn decode_attr(bytes: &[u8]) -> String {
    let s = decode_utf8(bytes);
    match unescape(&s) {
        Ok(decoded) => decoded.into_owned(),
        Err(_) => s,
    }
}

const _: fn() = || {
    let _ = StructDef {
        name: String::new(),
        base: None,
        abstract_: false,
        simple: true,
        fields: Vec::new(),
        source_path: String::new(),
    };
    let _: TypeName = String::new();
    let _ = ArrayKind::AtArray;
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::par_tree::library_from_par_tree;

    /// A made-up dump in the form a parser dump has, whose root, two members and a member's
    /// structure it holds only as the hash of a spelling.
    fn dump(root: &str) -> SchemaLibrary {
        let hash = |name: &str| format!("_0x{:08x}", crate::literal_string_hash(name.as_bytes()));
        let root = hash(root);
        let (spot, part, kind) = (hash("zqSpot"), hash("zqPart"), hash("ZqPart"));
        let json = format!(
            r#"{{"structs":[
              {{"name":"{root}","markup":"$struct$ =@{root}@\r\n{{\r\n\t$float$ Reach;\r\n\t$UnkVec3F$ {spot};\r\n\t$struct$ @{kind}@ {part};\r\n}};\r\n"}},
              {{"name":"{kind}","markup":"$struct$ =@{kind}@\r\n{{\r\n\t$float$ Weight;\r\n}};\r\n"}}
            ]}}"#
        );
        library_from_par_tree(json.as_bytes()).expect("the dump reads")
    }

    /// In the writer's own form, so a save that keeps every element gives these bytes back.
    const XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<CZqRoot>\n  <Reach value=\"2.5\"/>\n  <zqSpot x=\"1.0\" y=\"2.0\" z=\"3.0\"/>\n  <zqPart>\n    <Weight value=\"0.5\"/>\n  </zqPart>\n  <zqUnheard>kept as written</zqUnheard>\n</CZqRoot>\n";

    #[test]
    fn a_root_and_members_held_only_by_hash_read_typed_under_the_xmls_names() {
        let inst = parse_meta(&dump("CZqRoot"), XML.as_bytes()).expect("the root resolves by hash");
        assert_eq!(inst.type_name, "CZqRoot");
        let names: Vec<&str> = inst.fields.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["Reach", "zqSpot", "zqPart"]);
        assert_eq!(inst.fields[1].1, Value::Vec3([1.0, 2.0, 3.0]));
        let Value::Struct(part) = &inst.fields[2].1 else {
            panic!("zqPart reads as its structure: {:?}", inst.fields[2].1);
        };
        assert_eq!(part.fields, [("Weight".to_string(), Value::Float(0.5))]);
        assert_eq!(member_counts(&inst), (4, 1));
    }

    #[test]
    fn a_member_the_library_lacks_is_kept_as_text_and_saves_byte_identical() {
        let inst = parse_meta(&dump("CZqRoot"), XML.as_bytes()).expect("reads");
        assert_eq!(inst.extras.len(), 1);
        assert_eq!(inst.extras[0].tag, "zqUnheard");
        assert_eq!(inst.extras[0].text, "kept as written");
        let saved = crate::meta::write_meta(&inst).expect("writes");
        assert_eq!(saved, XML, "no hash_ spelling and no element lost");
    }

    #[test]
    fn a_root_whose_hash_is_of_another_spelling_is_not_found() {
        match parse_meta(&dump("CZqOther"), XML.as_bytes()) {
            Err(MetaError::UnknownType(name)) => assert_eq!(name, "CZqRoot"),
            other => panic!("the root must not resolve: {other:?}"),
        }
        let (tree, untyped) =
            parse_typed_or_tree(&dump("CZqOther"), XML.as_bytes()).expect("reads as its tree");
        assert_eq!(tree.type_name, "CZqRoot");
        assert_eq!(untyped.as_deref(), Some("CZqRoot"));
        let (_, untyped) = parse_typed_or_tree(&dump("CZqRoot"), XML.as_bytes()).expect("typed");
        assert_eq!(untyped, None);
    }
}
