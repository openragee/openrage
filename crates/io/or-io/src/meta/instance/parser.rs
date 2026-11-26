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
    normalize_member_name, ArrayKind, EnumDef, Field as SchemaField, FieldType, ResolvedStruct,
    SchemaLibrary, StringKind, StructDef, TypeName,
};

use super::value::{HashedString, MetaInstance, UnknownNode, Value};

/// What a read is given: the library, and whether an element typed as a structure the library
/// lacks is kept (as the structure it derives from, else as text) rather than refused.
#[derive(Clone, Copy)]
struct Cx<'a> {
    library: &'a SchemaLibrary,
    keep_unknown: bool,
}

pub fn parse_meta(library: &SchemaLibrary, bytes: &[u8]) -> Result<MetaInstance> {
    parse_root(
        Cx {
            library,
            keep_unknown: false,
        },
        bytes,
    )
}

fn parse_root(cx: Cx<'_>, bytes: &[u8]) -> Result<MetaInstance> {
    let library = cx.library;
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
                    cx,
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
                apply_attribute_fields(&mut inst, cx, &resolved, e.attributes())?;
                return Ok(inst);
            }
            _ => {}
        }
        buf.clear();
    }
}

/// XML `bytes` typed by `library` where it can, else untyped whole with the structure it lacks.
pub fn parse_typed_or_tree(
    library: &SchemaLibrary,
    bytes: &[u8],
) -> Result<(MetaInstance, Option<String>)> {
    // A `type` naming a structure the library lacks is read as the structure it derives from,
    // with the members that one lacks kept as text, so the document stays typed. A document
    // whose root the library lacks is the tree it states, or kept as text where no tree holds it.
    let cx = Cx {
        library,
        keep_unknown: true,
    };
    match parse_root(cx, bytes) {
        Err(MetaError::UnknownType(name)) => {
            let tree = super::schemaless::parse_schemaless(bytes).or_else(|_| kept_whole(bytes))?;
            Ok((tree, Some(name)))
        }
        read => read.map(|typed| (typed, None)),
    }
}

/// A document as one element kept whole under an instance with no type, which a save writes as it came.
fn kept_whole(bytes: &[u8]) -> Result<MetaInstance> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let root = loop {
        match reader.read_event_into(&mut buf)? {
            Event::Eof => {
                return Err(MetaError::Other(
                    "no root element found in XML document".into(),
                ))
            }
            Event::Start(e) => {
                break read_unknown_open(&mut reader, &local_name(e.name()), e.attributes())?
            }
            Event::Empty(e) => break read_unknown_empty(&local_name(e.name()), e.attributes())?,
            _ => {}
        }
        buf.clear();
    };
    let mut inst = MetaInstance::new("");
    inst.extras.push(root);
    Ok(inst)
}

fn read_struct_body(
    reader: &mut Reader<&[u8]>,
    cx: Cx<'_>,
    resolved: &ResolvedStruct<'_>,
    close_tag: &str,
    type_name: &str,
    attrs: Attributes,
) -> Result<MetaInstance> {
    let mut inst = MetaInstance::new(type_name);
    apply_attribute_fields(&mut inst, cx, resolved, attrs)?;

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
                    let value = read_field_open(reader, cx, &field.ty, &tag, e.attributes())?;
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
                    let value = read_field_empty(cx, &field.ty, e.attributes())?;
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
    cx: Cx<'_>,
    ty: &FieldType,
    tag: &str,
    attrs: Attributes,
) -> Result<Value> {
    let library = cx.library;
    match ty {
        FieldType::Array { elem, .. } => read_array(reader, cx, elem.as_ref(), tag),
        FieldType::Map { key, value } => read_map(reader, cx, key.as_ref(), value.as_ref(), tag),
        FieldType::Struct(name) => {
            let resolved = library
                .resolve_any(name)
                .ok_or_else(|| MetaError::UnknownType(name.clone()))?;
            let inner = read_struct_body(reader, cx, &resolved, tag, &resolved.def.name, attrs)?;
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
            let inner = match library.resolve_any(&concrete) {
                Some(resolved) => {
                    let kept = kept_name(&resolved, &concrete);
                    read_struct_body(reader, cx, &resolved, tag, &kept, attrs)?
                }
                None if cx.keep_unknown => match library.resolve_any(base) {
                    // An attribute the base has no member for would be lost: refused instead.
                    Some(resolved) if !attributes_fit(&resolved, &attrs)? => {
                        return Err(MetaError::UnknownType(concrete))
                    }
                    Some(resolved) => {
                        read_struct_body(reader, cx, &resolved, tag, &concrete, attrs)?
                    }
                    None => kept_as_text(reader, tag, &concrete, attrs)?,
                },
                None => return Err(MetaError::UnknownType(concrete)),
            };
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
            scalar_value(cx, ty, &raw)
        }
    }
}

/// Whether every attribute in `attrs` but `type` and a map entry's `key` is a member of `resolved`.
fn attributes_fit(resolved: &ResolvedStruct<'_>, attrs: &Attributes) -> Result<bool> {
    for a in attrs.clone() {
        let name = local_name(a?.key);
        let key = normalize_member_name(&name);
        let member = (resolved.fields.iter()).any(|f| f.name == key || f.name == name);
        if !member && name != "type" && name != "key" {
            return Ok(false);
        }
    }
    Ok(true)
}

/// An element typed as a structure neither its `type` nor its base names in the library: its
/// elements kept whole under `type_name`, or refused where it holds text, which no instance keeps.
fn kept_as_text(
    reader: &mut Reader<&[u8]>,
    tag: &str,
    type_name: &str,
    attrs: Attributes,
) -> Result<MetaInstance> {
    let node = read_unknown_open(reader, tag, attrs)?;
    // An instance keeps no text and no attribute of its own but its type: refused, the document
    // falls back to a reader that keeps them.
    let own_attribute = node.attrs.iter().any(|(k, _)| k != "type" && k != "key");
    if !node.text.trim().is_empty() || own_attribute {
        return Err(MetaError::UnknownType(type_name.to_string()));
    }
    let mut inst = MetaInstance::new(type_name);
    inst.extras = node.children;
    Ok(inst)
}

fn read_field_empty(cx: Cx<'_>, ty: &FieldType, attrs: Attributes) -> Result<Value> {
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
            scalar_value(cx, ty, &raw)
        }
    }
}

fn read_array(
    reader: &mut Reader<&[u8]>,
    cx: Cx<'_>,
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
                let value = read_field_open(reader, cx, elem, &tag, e.attributes())?;
                items.push(value);
            }
            Event::Empty(e) => {
                let value = read_field_empty(cx, elem, e.attributes())?;
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
            items.push(scalar_value(cx, elem, token)?);
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

/// ⚠ A map is written two ways: a `<Key>` then a `<Value>` element per entry, or one
/// `<Item key="…">` per entry whose element is the value. Pairing the keyed form's elements as
/// key and value read every other entry as a key and lost the rest. The keyed form's `type` may
/// name the key's type (`atHashString`), not a structure.
fn read_map(
    reader: &mut Reader<&[u8]>,
    cx: Cx<'_>,
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
                match (pending_key.take(), attribute(&e.attributes(), "key")?) {
                    (None, Some(key)) => {
                        let key = scalar_value(cx, key_ty, &key)?;
                        let typed = attribute(&e.attributes(), "type")?;
                        let value = match val_ty {
                            // A `type` that spells the key's type names no structure: the value
                            // reads as its base, under the base's name.
                            FieldType::Pointer { base, .. }
                                if typed.as_deref().is_some_and(|t| names_the_type(key_ty, t)) =>
                            {
                                let resolved = (cx.library.resolve_any(base))
                                    .ok_or_else(|| MetaError::UnknownType(base.clone()))?;
                                let kept = kept_name(&resolved, base);
                                let inner = read_struct_body(
                                    reader,
                                    cx,
                                    &resolved,
                                    &tag,
                                    &kept,
                                    e.attributes(),
                                )?;
                                Value::Pointer(Some(Box::new(inner)))
                            }
                            _ => read_field_open(reader, cx, val_ty, &tag, e.attributes())?,
                        };
                        pairs.push((key, value));
                    }
                    (None, None) => {
                        pending_key =
                            Some(read_field_open(reader, cx, key_ty, &tag, e.attributes())?);
                    }
                    (Some(key), _) => {
                        let value = read_field_open(reader, cx, val_ty, &tag, e.attributes())?;
                        pairs.push((key, value));
                    }
                }
            }
            Event::Empty(e) => match (pending_key.take(), attribute(&e.attributes(), "key")?) {
                (None, Some(key)) => {
                    let key = scalar_value(cx, key_ty, &key)?;
                    pairs.push((key, read_field_empty(cx, val_ty, e.attributes())?));
                }
                (None, None) => pending_key = Some(read_field_empty(cx, key_ty, e.attributes())?),
                (Some(key), _) => pairs.push((key, read_field_empty(cx, val_ty, e.attributes())?)),
            },
            _ => {}
        }
        buf.clear();
    }
    Ok(Value::Map(pairs))
}

/// Whether `spelling`, a keyed map entry's `type`, names the map's key type `ty`.
fn names_the_type(ty: &FieldType, spelling: &str) -> bool {
    match ty {
        FieldType::Enum(name) => {
            name == spelling
                || name.replace("::", "__") == spelling
                || name.replace("__", "::") == spelling
        }
        _ => crate::meta::schema::parser::map_key_type(spelling, None) == *ty,
    }
}

/// The value of the attribute `name`, decoded.
fn attribute(attrs: &Attributes, name: &str) -> Result<Option<String>> {
    for a in attrs.clone() {
        let a = a?;
        if local_name(a.key) == name {
            return Ok(Some(decode_attr(&a.value)));
        }
    }
    Ok(None)
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
    cx: Cx<'_>,
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
            let value = scalar_value(cx, &field.ty, &raw).unwrap_or(Value::String(raw));
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

fn scalar_value(cx: Cx<'_>, ty: &FieldType, raw: &str) -> Result<Value> {
    let library = cx.library;
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
        FieldType::Bitset { values, .. } => Ok(match parse_bitset(library, values, trimmed) {
            Some(bits) => Value::Bitset {
                type_name: values.clone(),
                bits,
            },
            None if cx.keep_unknown => Value::Raw(trimmed.to_string()),
            // ⚠ A strict read, as GTA V's, keeps the bits its tokens name and drops the rest, as
            // it always has: a binary writer refuses a raw bitset.
            None => Value::Bitset {
                type_name: values.clone(),
                bits: named_bits(library, values, trimmed),
            },
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

/// The bits `raw` states, or `None` where a token names no member of the enum, a bit it cannot
/// hold, or a number it cannot be, so the text is kept as written rather than saved as no bits.
fn parse_bitset(library: &SchemaLibrary, enum_name: &str, raw: &str) -> Option<u128> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Some(0);
    }
    if let Some(stripped) = trimmed.strip_prefix("0x").or(trimmed.strip_prefix("0X")) {
        return u128::from_str_radix(stripped, 16).ok();
    }
    if let Ok(n) = trimmed.parse::<u128>() {
        return Some(n);
    }
    let enum_def = library.enum_(enum_name)?;
    let mut bits: u128 = 0;
    for token in trimmed.split(|c: char| matches!(c, ' ' | ',' | '\t' | '\n' | '\r' | '|')) {
        let t = token.trim();
        if t.is_empty() {
            continue;
        }
        let v = enum_def.variant_value(enum_member(enum_def, t)?);
        if !(0..128).contains(&v) {
            return None;
        }
        bits |= 1u128 << v;
    }
    Some(bits)
}

/// The bits of `raw` the library names, the rest dropped: `0` for bad hex or an enum it lacks.
fn named_bits(library: &SchemaLibrary, enum_name: &str, raw: &str) -> u128 {
    if let Some(stripped) = raw.strip_prefix("0x").or(raw.strip_prefix("0X")) {
        return u128::from_str_radix(stripped, 16).unwrap_or(0);
    }
    let Some(enum_def) = library.enum_(enum_name) else {
        return 0;
    };
    (raw.split(|c: char| c.is_ascii_whitespace() || matches!(c, ',' | '|')))
        .filter_map(|token| enum_member(enum_def, token.trim()))
        .map(|idx| enum_def.variant_value(idx))
        .filter(|v| (0..128).contains(v))
        .fold(0, |bits, v| bits | 1u128 << v)
}

/// The member `spelling` names: by name, or by the literal hash of the spelling where the library
/// holds the member only as its hash (`hash_XXXXXXXX`).
fn enum_member(enum_def: &EnumDef, spelling: &str) -> Option<usize> {
    let values = &enum_def.values;
    (values.iter().position(|v| v.name == spelling)).or_else(|| {
        let hashed = hash_spelling(spelling);
        values.iter().position(|v| v.name == hashed)
    })
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

/// An element of XML text with the byte span of each attribute's value and of what it holds.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpannedElement {
    pub tag: String,
    /// Each attribute's name and the span of its value as written, escapes included.
    pub attrs: Vec<(String, std::ops::Range<usize>)>,
    /// The bytes between its start and end tags; `None` for an element written as one empty tag.
    pub content: Option<std::ops::Range<usize>>,
    pub children: Vec<SpannedElement>,
}

/// The root element of XML `bytes`, with the byte span of every attribute value and element body.
pub fn spanned_root(bytes: &[u8]) -> Result<SpannedElement> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(false);
    loop {
        match reader.read_event()? {
            Event::Eof => {
                return Err(MetaError::Other(
                    "no root element found in XML document".into(),
                ))
            }
            Event::Start(e) => {
                let mut root = spanned(bytes, &e)?;
                spanned_body(&mut reader, bytes, &mut root)?;
                return Ok(root);
            }
            Event::Empty(e) => return spanned(bytes, &e),
            _ => {}
        }
    }
}

fn spanned(bytes: &[u8], start: &quick_xml::events::BytesStart<'_>) -> Result<SpannedElement> {
    let mut element = SpannedElement {
        tag: local_name(start.name()),
        ..SpannedElement::default()
    };
    for a in start.attributes() {
        let a = a?;
        // A slice reader borrows every value from `bytes`, so its address is its offset.
        let at = (a.value.as_ptr() as usize)
            .checked_sub(bytes.as_ptr() as usize)
            .filter(|&at| at <= bytes.len() && a.value.len() <= bytes.len() - at)
            .ok_or_else(|| MetaError::Other("an attribute value is not in the text".into()))?;
        element
            .attrs
            .push((local_name(a.key), at..at + a.value.len()));
    }
    Ok(element)
}

fn spanned_body<'a>(
    reader: &mut Reader<&'a [u8]>,
    bytes: &'a [u8],
    element: &mut SpannedElement,
) -> Result<()> {
    let start = reader.buffer_position() as usize;
    loop {
        let before = reader.buffer_position() as usize;
        match reader.read_event()? {
            Event::Eof => {
                return Err(MetaError::Other(format!(
                    "the XML document ends inside <{}>",
                    element.tag
                )))
            }
            Event::End(_) => {
                element.content = Some(start..before);
                return Ok(());
            }
            Event::Start(e) => {
                let mut child = spanned(bytes, &e)?;
                spanned_body(reader, bytes, &mut child)?;
                element.children.push(child);
            }
            Event::Empty(e) => element.children.push(spanned(bytes, &e)?),
            _ => {}
        }
    }
}

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

    /// A made-up dump with a bitset and an enum over an enum one of whose members it holds only
    /// as the literal hash of its spelling, a keyed map and an array of pointers.
    fn flags_dump() -> SchemaLibrary {
        let hidden = format!("_0x{:08x}", crate::literal_string_hash(b"ZQ_HIDDEN"));
        let json = format!(
            r#"{{"structs":[
              {{"name":"CZqFlags","markup":"$struct$ =@CZqFlags@\r\n{{\r\n\t$bitset$<$enum$ @ZqFlag@> Flags;\r\n\t$enum$ @ZqFlag@ Kind;\r\n\t$map$<$string$, $struct$ @ZqPart@*> Parts;\r\n\t$array$<$struct$ @ZqPart@*> Items;\r\n\t$struct$ $void$* Any;\r\n}};\r\n"}},
              {{"name":"ZqPart","markup":"$struct$ =@ZqPart@\r\n{{\r\n\t$float$ Weight;\r\n}};\r\n"}}
            ],"enums":[
              {{"name":"ZqFlag","markup":"$enum$ =@ZqFlag@\r\n{{\r\n\tZQ_A = 0,\r\n\t{hidden} = 1,\r\n\tZQ_C = 2,\r\n}};\r\n"}}
            ]}}"#
        );
        library_from_par_tree(json.as_bytes()).expect("the dump reads")
    }

    fn flags(body: &str) -> String {
        format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<CZqFlags>\n{body}</CZqFlags>\n")
    }

    /// Typed with nothing untyped, and saved at a fixed point; the first save.
    fn saved_typed(library: &SchemaLibrary, xml: &str) -> (MetaInstance, String) {
        let (inst, untyped) = parse_typed_or_tree(library, xml.as_bytes()).expect("reads");
        assert_eq!(untyped, None, "the root is typed");
        let saved = crate::meta::write_meta(&inst).expect("writes");
        let (again, _) = parse_typed_or_tree(library, saved.as_bytes()).expect("reads again");
        assert_eq!(crate::meta::write_meta(&again).expect("writes"), saved);
        (inst, saved)
    }

    #[test]
    fn a_bitset_token_held_only_as_a_hash_survives_a_save() {
        let library = flags_dump();
        let xml = flags("  <Flags>ZQ_A ZQ_HIDDEN</Flags>\n");
        let (inst, saved) = saved_typed(&library, &xml);
        let set = Value::Bitset {
            type_name: "ZqFlag".into(),
            bits: 0b11,
        };
        assert_eq!(inst.get("Flags"), Some(&set));
        assert!(saved.contains("<Flags>0x3</Flags>"), "{saved}");
        let strict = parse_meta(&library, xml.as_bytes()).expect("reads strict");
        assert_eq!(strict.get("Flags"), Some(&set));
    }

    #[test]
    fn a_bitset_token_no_member_names_is_kept_as_written() {
        let library = flags_dump();
        for written in ["ZQ_A ZQ_NOWHERE", "0xZZ"] {
            let (inst, saved) =
                saved_typed(&library, &flags(&format!("  <Flags>{written}</Flags>\n")));
            assert_eq!(inst.get("Flags"), Some(&Value::Raw(written.into())));
            assert!(
                saved.contains(&format!("<Flags>{written}</Flags>")),
                "{saved}"
            );
        }
        // A strict read, GTA V's, reads as it always has: the bits named, the rest dropped, so a
        // binary save still writes a bitset.
        for (written, bits) in [("ZQ_A ZQ_NOWHERE", 0b1), ("0xZZ", 0)] {
            let xml = flags(&format!("  <Flags>{written}</Flags>\n"));
            let strict = parse_meta(&library, xml.as_bytes()).expect("reads strict");
            let set = Value::Bitset {
                type_name: "ZqFlag".into(),
                bits,
            };
            assert_eq!(strict.get("Flags"), Some(&set), "{written}");
        }
    }

    #[test]
    fn an_enum_value_held_only_as_a_hash_keeps_the_xmls_spelling() {
        let library = flags_dump();
        let def = library.enum_("ZqFlag").expect("the enum");
        assert_eq!(enum_member(def, "ZQ_HIDDEN"), Some(1));
        assert_eq!(enum_member(def, "ZQ_C"), Some(2));
        assert_eq!(enum_member(def, "ZQ_NOWHERE"), None);
        let (inst, saved) = saved_typed(&library, &flags("  <Kind>ZQ_HIDDEN</Kind>\n"));
        let kind = Value::Enum {
            type_name: "ZqFlag".into(),
            variant: "ZQ_HIDDEN".into(),
        };
        assert_eq!(inst.get("Kind"), Some(&kind));
        assert!(saved.contains("<Kind>ZQ_HIDDEN</Kind>"), "{saved}");
        assert!(!saved.contains("hash_"), "{saved}");
    }

    #[test]
    fn a_keyed_map_whose_type_names_the_keys_type_opens_typed() {
        let library = flags_dump();
        let xml = flags(concat!(
            "  <Parts>\n",
            "    <Item key=\"zqLeft\" type=\"atHashString\">\n      <Weight value=\"0.5\"/>\n    </Item>\n",
            "    <Item key=\"zqRight\" type=\"ZqPart\">\n      <Weight value=\"2.0\"/>\n    </Item>\n",
            "  </Parts>\n",
        ));
        parse_meta(&library, xml.as_bytes()).expect("a strict read takes the key's type too");
        let (inst, saved) = saved_typed(&library, &xml);
        parse_meta(&library, saved.as_bytes()).expect("a strict read reads the save");
        let Some(Value::Map(pairs)) = inst.get("Parts") else {
            panic!("Parts reads as a map: {:?}", inst.get("Parts"));
        };
        let entry = |(key, value): &(Value, Value)| {
            let Value::Pointer(Some(part)) = value else {
                panic!("a pointer: {value:?}");
            };
            (key.clone(), part.type_name.clone(), part.fields.clone())
        };
        let weight = |w| vec![("Weight".to_string(), Value::Float(w))];
        assert_eq!(
            pairs.iter().map(entry).collect::<Vec<_>>(),
            [
                (
                    Value::Hash(HashedString::new("zqLeft")),
                    "ZqPart".to_string(),
                    weight(0.5)
                ),
                (
                    Value::Hash(HashedString::new("zqRight")),
                    "ZqPart".to_string(),
                    weight(2.0)
                ),
            ]
        );
    }

    #[test]
    fn an_item_type_the_library_lacks_reads_as_its_base_and_keeps_the_rest() {
        let library = flags_dump();
        let xml = flags(concat!(
            "  <Items>\n",
            "    <Item type=\"ZqPart\">\n      <Weight value=\"1.0\"/>\n    </Item>\n",
            "    <Item type=\"CZqMissing__Tunables\">\n      <Weight value=\"3.0\"/>\n      <zqOwn>kept</zqOwn>\n    </Item>\n",
            "  </Items>\n",
            "  <Any type=\"CZqMissing\">\n    <zqOwn value=\"7\"/>\n  </Any>\n",
        ));
        let (inst, saved) = saved_typed(&library, &xml);
        assert_eq!(saved, xml, "every element is written back as it came");
        let Some(Value::Array(items)) = inst.get("Items") else {
            panic!("Items reads as an array");
        };
        let Value::Pointer(Some(missing)) = &items[1] else {
            panic!("a pointer: {:?}", items[1]);
        };
        assert_eq!(missing.type_name, "CZqMissing__Tunables");
        assert_eq!(missing.fields, [("Weight".to_string(), Value::Float(3.0))]);
        assert_eq!(missing.extras[0].tag, "zqOwn");
        let Some(Value::Pointer(Some(any))) = inst.get("Any") else {
            panic!("Any reads as a pointer");
        };
        assert!(any.fields.is_empty(), "no base names its members");
        assert_eq!(
            any.extras[0].attrs,
            [("value".to_string(), "7".to_string())]
        );
        assert_eq!(member_counts(&inst), (4, 2));
    }

    #[test]
    fn an_attribute_no_member_keeps_is_not_lost_by_a_lenient_read() {
        let library = flags_dump();
        for element in [
            "  <Any type=\"CZqMissing\" zqA=\"1\">\n    <zqOwn value=\"7\"/>\n  </Any>\n",
            concat!(
                "  <Items>\n    <Item type=\"CZqMissing__Tunables\" zqA=\"1\">\n",
                "      <Weight value=\"3.0\"/>\n    </Item>\n  </Items>\n",
            ),
        ] {
            let xml = flags(element);
            let (inst, _) = parse_typed_or_tree(&library, xml.as_bytes()).expect("opens");
            let saved = crate::meta::write_meta(&inst).expect("writes");
            assert!(saved.contains("zqA=\"1\""), "{saved}");
        }
    }

    #[test]
    fn a_document_no_structure_or_tree_holds_is_kept_whole_and_named_untyped() {
        let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<zq_cycle_data version=\"2\">\n  <Item name=\"a\">text and attributes</Item>\n</zq_cycle_data>\n";
        assert!(super::super::schemaless::parse_schemaless(xml.as_bytes()).is_err());
        let (inst, untyped) = parse_typed_or_tree(&flags_dump(), xml.as_bytes()).expect("opens");
        assert_eq!(untyped.as_deref(), Some("zq_cycle_data"));
        assert_eq!(member_counts(&inst), (0, 1));
        assert_eq!(crate::meta::write_meta(&inst).expect("writes"), xml);
    }

    #[test]
    fn every_attribute_value_and_element_body_is_spanned_as_written() {
        let xml = "<?xml version=\"1.0\"?>\r\n<!-- c -->\r\n<zqRoot a=\"1&amp;2\">\r\n  <id>zq_one</id>\r\n  <n value=\"12\"/>\r\n  <e></e>\r\n</zqRoot>\r\n";
        let root = spanned_root(xml.as_bytes()).expect("spans");
        let text = |span: &std::ops::Range<usize>| &xml[span.clone()];
        assert_eq!(root.tag, "zqRoot");
        assert_eq!(text(&root.attrs[0].1), "1&amp;2");
        let [id, n, e] = root.children.as_slice() else {
            panic!("three children: {:?}", root.children);
        };
        assert_eq!(id.content.as_ref().map(text), Some("zq_one"));
        assert_eq!((n.content.as_ref(), text(&n.attrs[0].1)), (None, "12"));
        assert_eq!(e.content.as_ref().map(text), Some(""));
        assert!(text(root.content.as_ref().expect("a body")).ends_with("<e></e>\r\n"));
        assert!(spanned_root(b"<zqRoot><id>").is_err());
    }
}
