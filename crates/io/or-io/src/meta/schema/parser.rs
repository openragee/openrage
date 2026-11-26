#![allow(
    clippy::manual_pattern_char_comparison,
    clippy::manual_strip,
    clippy::collapsible_if,
    clippy::collapsible_match
)]

use quick_xml::events::attributes::Attributes;
use quick_xml::events::Event;
use quick_xml::name::QName;
use quick_xml::Reader;

use super::normalize::{convert_datatype_to_friendly_name, normalize_member_name};
use super::types::*;

#[derive(Debug, Clone)]
pub enum SchemaItem {
    Struct(StructDef),
    Enum(EnumDef),
    Skipped,
}

pub fn parse_psc(bytes: &[u8], source_path: &str) -> Result<Vec<SchemaItem>, String> {
    let mut reader = Reader::from_reader(bytes);
    let cfg = reader.config_mut();
    cfg.trim_text(true);
    cfg.expand_empty_elements = false;

    let mut items: Vec<SchemaItem> = Vec::new();
    let mut buf = Vec::new();

    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| format!("xml: {e}"))?
        {
            Event::Eof => break,
            Event::Start(e) => match local_name(e.name()).as_str() {
                "structdef" => {
                    let s = parse_structdef(&mut reader, e.attributes(), source_path)?;
                    items.push(SchemaItem::Struct(s));
                }
                "enumdef" => {
                    let e = parse_enumdef(&mut reader, e.attributes(), source_path)?;
                    items.push(SchemaItem::Enum(e));
                }
                _ => {}
            },
            Event::Empty(e) => match local_name(e.name()).as_str() {
                "structdef" => {
                    let mut s = StructDef {
                        name: String::new(),
                        base: None,
                        abstract_: false,
                        simple: false,
                        fields: Vec::new(),
                        source_path: source_path.to_string(),
                    };
                    apply_structdef_attrs(&mut s, e.attributes())?;
                    items.push(SchemaItem::Struct(s));
                }
                "enumdef" => {
                    let mut ed = EnumDef {
                        name: String::new(),
                        values: Vec::new(),
                        generate_bitset: false,
                        source_path: source_path.to_string(),
                    };
                    apply_enumdef_attrs(&mut ed, e.attributes())?;
                    items.push(SchemaItem::Enum(ed));
                }
                _ => {}
            },
            _ => {}
        }
        buf.clear();
    }

    Ok(items)
}

fn parse_structdef(
    reader: &mut Reader<&[u8]>,
    attrs: Attributes,
    source_path: &str,
) -> Result<StructDef, String> {
    let mut def = StructDef {
        name: String::new(),
        base: None,
        abstract_: false,
        simple: false,
        fields: Vec::new(),
        source_path: source_path.to_string(),
    };
    apply_structdef_attrs(&mut def, attrs)?;

    let mut buf = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| format!("xml: {e}"))?
        {
            Event::Eof => break,
            Event::End(e) if local_name(e.name()) == "structdef" => break,
            Event::Start(e) => {
                let tag = local_name(e.name());
                if let Some(field) = parse_field_start(reader, &tag, e.attributes())? {
                    def.fields.push(field);
                } else {
                    skip_to_close(reader, &tag)?;
                }
            }
            Event::Empty(e) => {
                let tag = local_name(e.name());
                if let Some(field) = parse_field_empty(&tag, e.attributes())? {
                    def.fields.push(field);
                }
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(def)
}

fn apply_structdef_attrs(def: &mut StructDef, attrs: Attributes) -> Result<(), String> {
    for a in attrs {
        let a = a.map_err(|e| format!("attr: {e}"))?;
        let key = local_name(a.key);
        let value = decode_value(&a.value);
        match key.as_str() {
            "type" => def.name = convert_datatype_to_friendly_name(&value),
            "base" => {
                let b = convert_datatype_to_friendly_name(&value);
                def.base = if b.is_empty() { None } else { Some(b) };
            }
            "constructable" => def.abstract_ = value.eq_ignore_ascii_case("false"),
            "simple" => def.simple = value.eq_ignore_ascii_case("true"),
            _ => {}
        }
    }
    Ok(())
}

fn parse_enumdef(
    reader: &mut Reader<&[u8]>,
    attrs: Attributes,
    source_path: &str,
) -> Result<EnumDef, String> {
    let mut def = EnumDef {
        name: String::new(),
        values: Vec::new(),
        generate_bitset: false,
        source_path: source_path.to_string(),
    };
    apply_enumdef_attrs(&mut def, attrs)?;

    let mut buf = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| format!("xml: {e}"))?
        {
            Event::Eof => break,
            Event::End(e) if local_name(e.name()) == "enumdef" => break,
            Event::Empty(e) if local_name(e.name()) == "enumval" => {
                let ev = read_enumval(e.attributes())?;
                def.values.push(ev);
            }
            Event::Start(e) if local_name(e.name()) == "enumval" => {
                let ev = read_enumval(e.attributes())?;
                def.values.push(ev);
                skip_to_close(reader, "enumval")?;
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(def)
}

fn read_enumval(attrs: Attributes) -> Result<EnumValue, String> {
    let mut ev = EnumValue {
        name: String::new(),
        description: None,
        value: None,
    };
    for a in attrs {
        let a = a.map_err(|err| format!("attr: {err}"))?;
        let key = local_name(a.key);
        let value = decode_value(&a.value);
        match key.as_str() {
            "name" => ev.name = value,
            "description" => ev.description = Some(value),
            "value" => ev.value = parse_int_lit(&value),
            _ => {}
        }
    }
    Ok(ev)
}

fn apply_enumdef_attrs(def: &mut EnumDef, attrs: Attributes) -> Result<(), String> {
    for a in attrs {
        let a = a.map_err(|e| format!("attr: {e}"))?;
        let key = local_name(a.key);
        let value = decode_value(&a.value);
        match key.as_str() {
            "type" => def.name = value,
            "generate" => {
                if value.eq_ignore_ascii_case("bitset") {
                    def.generate_bitset = true;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn parse_field_start(
    reader: &mut Reader<&[u8]>,
    tag: &str,
    attrs: Attributes,
) -> Result<Option<Field>, String> {
    match tag {
        "array" => {
            let mut field = unnamed_field(FieldType::Pad(0));
            let mut shape = ArrayAttrs::default();
            for a in attrs {
                let a = a.map_err(|e| format!("attr: {e}"))?;
                let key = local_name(a.key);
                let value = decode_value(&a.value);
                match key.as_str() {
                    "name" => field.name = normalize_member_name(&value),
                    "min" => field.min = parse_float_lit(&value),
                    "max" => field.max = parse_float_lit(&value),
                    "step" => field.step = parse_float_lit(&value),
                    "init" => field.init = Some(Literal::Other(value)),
                    "ui_key" => field.ui_key = value.eq_ignore_ascii_case("true"),
                    "hideWidgets" => field.hide_widgets = value.eq_ignore_ascii_case("true"),
                    "description" => field.description = Some(value),
                    other => shape.take(other, value),
                }
            }
            field.ty = shape.finish(parse_array_element(reader, "array")?);
            Ok(Some(field))
        }
        "map" => {
            let mut field = unnamed_field(FieldType::Pad(0));
            let mut shape = MapAttrs::default();
            for a in attrs {
                let a = a.map_err(|e| format!("attr: {e}"))?;
                let key = local_name(a.key);
                let value = decode_value(&a.value);
                match key.as_str() {
                    "name" => field.name = normalize_member_name(&value),
                    "hideWidgets" => field.hide_widgets = value.eq_ignore_ascii_case("true"),
                    "description" => field.description = Some(value),
                    other => shape.take(other, value),
                }
            }
            field.ty = shape.finish(reader)?;
            Ok(Some(field))
        }
        _ => {
            let result = parse_field_empty(tag, attrs)?;
            if result.is_some() {
                skip_to_close(reader, tag)?;
            }
            Ok(result)
        }
    }
}

fn parse_field_empty(tag: &str, attrs: Attributes) -> Result<Option<Field>, String> {
    let ty = match tag {
        "bool" => FieldType::Bool,
        "u8" => FieldType::U8,
        "u16" => FieldType::U16,
        "u32" => FieldType::U32,
        "u64" => FieldType::U64,
        "s8" | "char" => FieldType::S8,
        "s16" | "short" => FieldType::S16,
        "s32" | "int" => FieldType::S32,
        "s64" => FieldType::S64,
        "float" => FieldType::Float,
        "Float16" | "float16" => FieldType::Float16,
        "double" => FieldType::Double,
        "Vector2" | "vector2" => FieldType::Vector2,
        "Vector3" | "vector3" => FieldType::Vector3,
        "Vector4" | "vector4" => FieldType::Vector4,
        "Vec2V" => FieldType::Vec2V,
        "Vec3V" => FieldType::Vec3V,
        "Vec4V" => FieldType::Vec4V,
        "Mat33V" => FieldType::Mat33V,
        "Mat34V" => FieldType::Mat34V,
        "Mat44V" => FieldType::Mat44V,
        "Matrix33" => FieldType::Matrix33,
        "Matrix34" => FieldType::Matrix34,
        "Matrix44" => FieldType::Matrix44,
        "Color32" | "Color" | "color32" | "color" => FieldType::Color32,
        "string" => FieldType::String(StringKind::default()),
        "enum" => FieldType::Enum(String::new()),
        "bitset" => FieldType::Bitset {
            storage: BitsetStorage::Fixed32,
            values: String::new(),
        },
        "struct" => FieldType::Struct(String::new()),
        "pointer" => FieldType::Pointer {
            base: String::new(),
            policy: PointerPolicy::default(),
        },
        "pad" => FieldType::Pad(0),
        "hinclude" | "hinsert" | "const" | "size_t" => return Ok(None),
        _ => FieldType::Unknown(tag.to_string()),
    };

    let mut field = unnamed_field(ty);
    apply_field_attrs(&mut field, attrs)?;
    Ok(Some(field))
}

fn unnamed_field(ty: FieldType) -> Field {
    Field {
        name: String::new(),
        ty,
        init: None,
        min: None,
        max: None,
        step: None,
        ui_key: false,
        hide_widgets: false,
        description: None,
        hint: FieldHint::None,
    }
}

/// The attributes that say what shape an `<array>` is.
///
/// ⚠ Collected and resolved only once every attribute is read: `size=`
/// precedes `type=` in objects_simple.psc, so a kind built inside the
/// attribute loop takes the size of the previous member.
#[derive(Default)]
struct ArrayAttrs {
    kind: String,
    size: String,
    size_var: Option<String>,
    align: u32,
}

impl ArrayAttrs {
    fn take(&mut self, key: &str, value: String) {
        match key {
            "type" => self.kind = value,
            "size" => self.size = value,
            "sizeVar" => self.size_var = Some(normalize_member_name(&value)),
            "align" => self.align = parse_int_lit(&value).map_or(0, |n| n as u32),
            _ => {}
        }
    }

    fn finish(self, elem: FieldType) -> FieldType {
        let kind = if self.kind == "pointer" {
            ArrayKind::Pointer {
                size_var: self.size_var,
                align: self.align,
            }
        } else {
            parse_array_kind(&self.kind, &self.size)
        };
        FieldType::Array {
            elem: Box::new(elem),
            kind,
        }
    }
}

/// The attributes that say what a `<map>` is keyed by.
///
/// ⚠ **The key is an attribute and the one child is the value.** All 267
/// maps in the 1,134-file corpus spell it `key="atHashString"` and so on;
/// reading the child as the key typed every map's value `Unknown`.
#[derive(Default)]
struct MapAttrs {
    key: Option<String>,
    enum_key_type: Option<String>,
}

impl MapAttrs {
    fn take(&mut self, key: &str, value: String) {
        match key {
            "key" => self.key = Some(value),
            "enumKeyType" => self.enum_key_type = Some(value),
            _ => {}
        }
    }

    fn finish(self, reader: &mut Reader<&[u8]>) -> Result<FieldType, String> {
        let (first, second) = parse_map_body(reader)?;
        let (key, value) = match self.key {
            Some(spelling) => (map_key_type(&spelling, self.enum_key_type), first),
            None => (first, second),
        };
        Ok(FieldType::Map {
            key: Box::new(key),
            value: Box::new(value),
        })
    }
}

/// The type a map's `key=` spelling names.
pub(crate) fn map_key_type(spelling: &str, enum_key_type: Option<String>) -> FieldType {
    match spelling {
        "enum" => FieldType::Enum(enum_key_type.unwrap_or_default()),
        "u8" => FieldType::U8,
        "u16" => FieldType::U16,
        "u32" => FieldType::U32,
        "u64" => FieldType::U64,
        "s8" => FieldType::S8,
        "s16" => FieldType::S16,
        "s32" => FieldType::S32,
        "s64" => FieldType::S64,
        other => match hash_string_kind(other) {
            Some(kind) => FieldType::String(kind),
            None => FieldType::Unknown(format!("map_key {other}")),
        },
    }
}

/// The string kind a hashed-string spelling names, or `None` for one that
/// is not a hash.
///
/// ⚠ **Every `atHash*` spelling is a 32-bit hash**, whichever string bank or
/// finalisation it names. Five of the eight that string members in the corpus
/// use fell through to a pointer string, which is not what any file stores.
fn hash_string_kind(spelling: &str) -> Option<StringKind> {
    Some(match spelling {
        "atHashString"
        | "atHashWithStringBank"
        | "atHashWithStringDev"
        | "atHashWithStringNotFinal"
        | "atNonFinalHashString"
        | "atDiagHashString" => StringKind::AtHashString,
        "atFinalHashString" | "atFinalHashStringValue" => StringKind::AtFinalHashString,
        "atHashValue" | "atPartialHashValue" => StringKind::AtHashValue,
        _ => return None,
    })
}

fn apply_field_attrs(field: &mut Field, attrs: Attributes) -> Result<(), String> {
    for a in attrs {
        let a = a.map_err(|e| format!("attr: {e}"))?;
        let key = local_name(a.key);
        let value = decode_value(&a.value);
        match key.as_str() {
            "name" => field.name = normalize_member_name(&value),
            "init" => field.init = Some(literal_from(&value, &field.ty)),
            "min" => field.min = parse_float_lit(&value),
            "max" => field.max = parse_float_lit(&value),
            "step" => field.step = parse_float_lit(&value),
            "ui_key" => field.ui_key = value.eq_ignore_ascii_case("true"),
            "hideWidgets" => field.hide_widgets = value.eq_ignore_ascii_case("true"),
            "description" => field.description = Some(value),
            "type" => apply_type_attr(&mut field.ty, &value),
            "values" => {
                if let FieldType::Bitset { values, .. } = &mut field.ty {
                    *values = value;
                }
            }
            "policy" => {
                if let FieldType::Pointer { policy, .. } = &mut field.ty {
                    *policy = match value.as_str() {
                        "owner" => PointerPolicy::Owner,
                        "ref" | "link" | "reference" => PointerPolicy::Reference,
                        "external_named" | "external" => PointerPolicy::ExternalNamed,
                        _ => PointerPolicy::Owner,
                    };
                }
            }
            "size" | "bytes" => {
                if let FieldType::Pad(n) = &mut field.ty {
                    if let Some(v) = parse_int_lit(&value) {
                        *n = v as u32;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn apply_type_attr(ty: &mut FieldType, value: &str) {
    match ty {
        FieldType::String(kind) => {
            *kind = match value {
                "atString" => StringKind::AtString,
                other => hash_string_kind(other).unwrap_or(StringKind::Plain),
            }
        }
        FieldType::Enum(name) => *name = value.to_string(),
        FieldType::Struct(name) => *name = convert_datatype_to_friendly_name(value),
        FieldType::Pointer { base, .. } => *base = convert_datatype_to_friendly_name(value),
        FieldType::Bitset { storage, .. } => {
            *storage = match value {
                "fixed8" | "atFixedBitset8" => BitsetStorage::Fixed8,
                "fixed16" | "atFixedBitset16" => BitsetStorage::Fixed16,
                "fixed32" | "atFixedBitset32" => BitsetStorage::Fixed32,
                "fixed64" | "atFixedBitset64" => BitsetStorage::Fixed64,
                _ => BitsetStorage::Variable,
            }
        }
        _ => {}
    }
}

fn parse_array_kind(kind: &str, size: &str) -> ArrayKind {
    // Sizes above what a PSO count field holds are as unusable as a symbolic
    // one, and saying so here keeps every consumer's zero meaning one thing.
    let count = parse_int_lit(size)
        .filter(|n| (1..=i64::from(u16::MAX)).contains(n))
        .map(|n| n as u32)
        .unwrap_or(0);
    if kind.starts_with("atFixedArray") {
        return ArrayKind::AtFixedArray(count);
    }
    // ⚠ `atRangeArray<T,N>` is a run of N, not a heap array, and falling
    // through to the default here called four of them heap. The shipped type
    // tables say so: `CJunctionTemplate::Entrances` is 16 in every corpus
    // document that carries it, which is the `size=` the PSC states.
    if kind.starts_with("atRangeArray") {
        return ArrayKind::Member(count);
    }
    if kind.starts_with("atArray") {
        return ArrayKind::AtArray;
    }
    if kind.starts_with("atPtr") {
        return ArrayKind::AtPtrArray;
    }
    if kind == "member" {
        return ArrayKind::Member(count);
    }
    ArrayKind::AtArray
}

fn parse_array_element(reader: &mut Reader<&[u8]>, end_tag: &str) -> Result<FieldType, String> {
    let mut elem: Option<FieldType> = None;
    let mut buf = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| format!("xml: {e}"))?
        {
            Event::Eof => break,
            Event::End(e) if local_name(e.name()) == end_tag => break,
            // ⚠ An element can itself be an array or a map, and reading it as
            // a leaf typed it `Unknown`: `CVehicleModelInfoVarGlobal::Wheels`
            // is a run of `atArray`s, and 4 corpus documents hold one.
            Event::Start(e)
                if elem.is_none() && matches!(local_name(e.name()).as_str(), "array" | "map") =>
            {
                let tag = local_name(e.name());
                elem = parse_field_start(reader, &tag, e.attributes())?.map(|f| f.ty);
            }
            Event::Start(e) => {
                let tag = local_name(e.name());
                if elem.is_none() {
                    if let Some(field) = parse_field_empty(&tag, e.attributes())? {
                        elem = Some(field.ty);
                    }
                }
                skip_to_close(reader, &tag)?;
            }
            Event::Empty(e) => {
                let tag = local_name(e.name());
                if elem.is_none() {
                    if let Some(field) = parse_field_empty(&tag, e.attributes())? {
                        elem = Some(field.ty);
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(elem.unwrap_or(FieldType::Unknown("array_elem".to_string())))
}

fn parse_map_body(reader: &mut Reader<&[u8]>) -> Result<(FieldType, FieldType), String> {
    let mut key_ty = FieldType::Unknown("map_key".to_string());
    let mut val_ty = FieldType::Unknown("map_value".to_string());
    let mut buf = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| format!("xml: {e}"))?
        {
            Event::Eof => break,
            Event::End(e) if local_name(e.name()) == "map" => break,
            Event::Start(e) => {
                let tag = local_name(e.name());
                let nested = matches!(tag.as_str(), "array" | "map");
                let parsed = if nested {
                    parse_field_start(reader, &tag, e.attributes())?
                } else {
                    parse_field_empty(&tag, e.attributes())?
                };
                if let Some(field) = parsed {
                    if key_ty == FieldType::Unknown("map_key".to_string()) {
                        key_ty = field.ty;
                    } else {
                        val_ty = field.ty;
                    }
                }
                if !nested {
                    skip_to_close(reader, &tag)?;
                }
            }
            Event::Empty(e) => {
                let tag = local_name(e.name());
                if let Some(field) = parse_field_empty(&tag, e.attributes())? {
                    if key_ty == FieldType::Unknown("map_key".to_string()) {
                        key_ty = field.ty;
                    } else {
                        val_ty = field.ty;
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }
    Ok((key_ty, val_ty))
}

fn skip_to_close(reader: &mut Reader<&[u8]>, end_tag: &str) -> Result<(), String> {
    let mut depth: i32 = 1;
    let mut buf = Vec::new();
    while depth > 0 {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| format!("xml: {e}"))?
        {
            Event::Eof => break,
            Event::Start(e) if local_name(e.name()) == end_tag => depth += 1,
            Event::End(e) if local_name(e.name()) == end_tag => depth -= 1,
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

fn local_name(name: QName) -> String {
    let raw = name.into_inner();
    let s = std::str::from_utf8(raw).unwrap_or("");
    match s.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => s.to_string(),
    }
}

fn decode_value(bytes: &[u8]) -> String {
    let s = std::str::from_utf8(bytes).unwrap_or("");
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

fn parse_int_lit(s: &str) -> Option<i64> {
    let trimmed = s.trim().trim_end_matches(|c: char| matches!(c, 'L' | 'l'));
    if let Some(stripped) = trimmed.strip_prefix("0x").or(trimmed.strip_prefix("0X")) {
        i64::from_str_radix(stripped, 16).ok()
    } else {
        trimmed.parse::<i64>().ok()
    }
}

fn parse_float_lit(s: &str) -> Option<f64> {
    let t = s
        .trim()
        .trim_end_matches(|c: char| matches!(c, 'f' | 'F' | 'd' | 'D'));
    t.parse::<f64>()
        .ok()
        .or_else(|| parse_int_lit(t).map(|v| v as f64))
}

fn literal_from(value: &str, ty: &FieldType) -> Literal {
    match ty {
        FieldType::Bool => match value.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Literal::Bool(true),
            "false" | "0" | "no" => Literal::Bool(false),
            _ => Literal::Other(value.to_string()),
        },
        t if t.is_integer() => parse_int_lit(value)
            .map(Literal::Int)
            .unwrap_or_else(|| Literal::Other(value.to_string())),
        t if t.is_float() => parse_float_lit(value)
            .map(Literal::Float)
            .unwrap_or_else(|| Literal::Other(value.to_string())),
        FieldType::String(_) => Literal::Str(value.to_string()),
        FieldType::Enum(_) => Literal::Enum(value.to_string()),
        _ => Literal::Other(value.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ambient_model_set() {
        let xml = r#"<?xml version="1.0"?>
<ParserSchema xmlns="x">
  <structdef type="CAmbientModel">
    <string name="m_Name" type="atHashString" ui_key="true"/>
    <float name="m_Probability" init="1" min="0" max="1"/>
  </structdef>
  <enumdef type="eAnimSceneFlags" generate="bitset">
    <enumval name="ASF_LOOPING" description="Loop"/>
    <enumval name="ASF_HOLD"/>
  </enumdef>
</ParserSchema>"#;
        let items = parse_psc(xml.as_bytes(), "test.psc").unwrap();
        assert_eq!(items.len(), 2);
        match &items[0] {
            SchemaItem::Struct(s) => {
                assert_eq!(s.name, "CAmbientModel");
                assert_eq!(s.fields.len(), 2);
                assert_eq!(s.fields[0].name, "Name");
                assert!(matches!(
                    s.fields[0].ty,
                    FieldType::String(StringKind::AtHashString)
                ));
                assert!(s.fields[0].ui_key);
                assert_eq!(s.fields[1].min, Some(0.0));
                assert_eq!(s.fields[1].max, Some(1.0));
                assert!(matches!(s.fields[1].init, Some(Literal::Float(_))));
            }
            _ => panic!(),
        }
        match &items[1] {
            SchemaItem::Enum(e) => {
                assert_eq!(e.name, "eAnimSceneFlags");
                assert!(e.generate_bitset);
                assert_eq!(e.values.len(), 2);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parses_array_with_struct_element() {
        let xml = r#"<?xml version="1.0"?>
<ParserSchema>
  <structdef type="Outer">
    <array name="m_Items" type="atArray">
      <struct type="rage__game__Item"/>
    </array>
  </structdef>
</ParserSchema>"#;
        let items = parse_psc(xml.as_bytes(), "test.psc").unwrap();
        match &items[0] {
            SchemaItem::Struct(s) => {
                assert_eq!(s.fields.len(), 1);
                match &s.fields[0].ty {
                    FieldType::Array { elem, kind } => {
                        assert!(matches!(kind, ArrayKind::AtArray));
                        assert_eq!(s.fields[0].name, "Items");
                        match elem.as_ref() {
                            FieldType::Struct(name) => assert_eq!(name, "rage::game::Item"),
                            other => panic!("unexpected elem: {other:?}"),
                        }
                    }
                    other => panic!("expected array, got {other:?}"),
                }
            }
            _ => panic!(),
        }
    }

    fn array_kinds(xml: &str) -> Vec<(String, ArrayKind)> {
        let items = parse_psc(xml.as_bytes(), "t.psc").unwrap();
        let SchemaItem::Struct(s) = &items[0] else {
            panic!("expected a structdef");
        };
        s.fields
            .iter()
            .map(|f| match &f.ty {
                FieldType::Array { kind, .. } => (f.name.clone(), kind.clone()),
                other => panic!("{} is {other:?} rather than an array", f.name),
            })
            .collect()
    }

    /// The four array spellings the corpus uses, and the size each carries.
    ///
    /// ⚠ `atRangeArray` is a run of N and not a heap array. It used to fall
    /// through to the default arm, which described four members of
    /// `CJunctionTemplate` as heap arrays against shipped type tables that
    /// state 16, 16, 8 and 8.
    #[test]
    fn a_fixed_array_carries_the_size_the_psc_declares() {
        let xml = r#"<?xml version="1.0"?>
<ParserSchema>
  <structdef type="X">
    <array name="m_Heap" type="atArray"><float/></array>
    <array name="m_Fixed" type="atFixedArray" size="10"><float/></array>
    <array name="m_Inline" type="member" size="3"><float/></array>
    <array name="m_Ranged" type="atRangeArray" size="16"><float/></array>
  </structdef>
</ParserSchema>"#;
        assert_eq!(
            array_kinds(xml),
            vec![
                ("Heap".to_string(), ArrayKind::AtArray),
                ("Fixed".to_string(), ArrayKind::AtFixedArray(10)),
                ("Inline".to_string(), ArrayKind::Member(3)),
                ("Ranged".to_string(), ArrayKind::Member(16)),
            ]
        );
    }

    /// ⚠ `size=` precedes `type=` in objects_simple.psc. A kind resolved
    /// inside the attribute loop takes whichever it met first, which is the
    /// previous member's size or none at all.
    #[test]
    fn the_size_is_read_whichever_side_of_the_type_it_sits() {
        let xml = r#"<?xml version="1.0"?>
<ParserSchema>
  <structdef type="X">
    <array name="m_After" type="atFixedArray" size="4"><float/></array>
    <array name="m_Before" size="4" type="atFixedArray"><float/></array>
  </structdef>
</ParserSchema>"#;
        let kinds = array_kinds(xml);
        assert_eq!(kinds[0].1, ArrayKind::AtFixedArray(4));
        assert_eq!(kinds[1].1, kinds[0].1, "attribute order changed the size");
    }

    /// ⚠ A size the PSC states as a C++ constant is **not** a size, and the
    /// parser must not invent one. 164 of the 486 `size=` attributes in the
    /// corpus are expressions no `.psc` defines; synthesis refuses those by
    /// name rather than laying out a guess.
    #[test]
    fn a_size_that_is_not_a_number_stays_unstated() {
        let xml = r#"<?xml version="1.0"?>
<ParserSchema>
  <structdef type="X">
    <array name="m_Symbolic" type="atFixedArray" size="CWeaponInfo::MAX_ATTACH_POINTS"><float/></array>
    <array name="m_Product" type="atFixedArray" size="A::B*C::D"><float/></array>
    <array name="m_Missing" type="member"><float/></array>
    <array name="m_Zero" type="atFixedArray" size="0"><float/></array>
    <array name="m_TooMany" type="atFixedArray" size="65536"><float/></array>
  </structdef>
</ParserSchema>"#;
        for (name, kind) in array_kinds(xml) {
            let count = match kind {
                ArrayKind::AtFixedArray(n) | ArrayKind::Member(n) => n,
                other => panic!("{name} is {other:?}"),
            };
            assert_eq!(count, 0, "{name} took a size it should not have");
        }
    }

    /// Hexadecimal, and the `L` suffix `parse_int_lit` already tolerates.
    #[test]
    fn a_size_is_read_the_way_every_other_integer_literal_is() {
        let xml = r#"<?xml version="1.0"?>
<ParserSchema>
  <structdef type="X">
    <array name="m_Hex" type="atFixedArray" size="0x10"><float/></array>
    <array name="m_Long" type="atFixedArray" size="12L"><float/></array>
    <array name="m_Spaced" type="atFixedArray" size=" 7 "><float/></array>
  </structdef>
</ParserSchema>"#;
        assert_eq!(
            array_kinds(xml)
                .into_iter()
                .map(|(_, k)| k)
                .collect::<Vec<_>>(),
            vec![
                ArrayKind::AtFixedArray(16),
                ArrayKind::AtFixedArray(12),
                ArrayKind::AtFixedArray(7),
            ]
        );
    }

    fn only_struct(xml: &str) -> StructDef {
        match parse_psc(xml.as_bytes(), "t.psc").unwrap().remove(0) {
            SchemaItem::Struct(s) => s,
            other => panic!("expected a structdef, got {other:?}"),
        }
    }

    /// ⚠ `type="pointer"` is a `T*` counted by another member, and the file
    /// stores it that way: `OccludeModel::verts` is an
    /// `ArrayPointer64WithCount` in all 670 occluder maps. It used to fall
    /// through to `atArray`, a header this member does not have.
    #[test]
    fn a_pointer_array_names_the_member_that_counts_it() {
        let s = only_struct(
            r#"<ParserSchema>
  <structdef type="rage::OccludeModel">
    <u32 name="m_dataSize"/>
    <array name="m_verts" type="pointer" sizeVar="m_dataSize" align="16"><u8/></array>
    <array name="m_Fixed" size="4" type="pointer"><u8/></array>
  </structdef>
</ParserSchema>"#,
        );
        assert_eq!(
            s.fields[1].ty,
            FieldType::Array {
                elem: Box::new(FieldType::U8),
                kind: ArrayKind::Pointer {
                    size_var: Some("dataSize".into()),
                    align: 16,
                },
            }
        );
        assert_eq!(
            s.fields[2].ty,
            FieldType::Array {
                elem: Box::new(FieldType::U8),
                kind: ArrayKind::Pointer {
                    size_var: None,
                    align: 0,
                },
            },
            "a pointer array with no sizeVar states no count, whatever its size="
        );
    }

    /// ⚠ An array of arrays: reading the inner `<array>` as a leaf typed it
    /// `Unknown("array")`, and synthesis refused 4 corpus documents on it.
    #[test]
    fn an_array_element_can_be_an_array() {
        let s = only_struct(
            r#"<ParserSchema>
  <structdef type="CVehicleModelInfoVarGlobal">
    <array name="m_Wheels" type="member" size="VWT_MAX">
      <array type="atArray"><struct type="CVehicleWheel"/></array>
    </array>
    <float name="m_After"/>
  </structdef>
</ParserSchema>"#,
        );
        assert_eq!(
            s.fields[0].ty,
            FieldType::Array {
                elem: Box::new(FieldType::Array {
                    elem: Box::new(FieldType::Struct("CVehicleWheel".into())),
                    kind: ArrayKind::AtArray,
                }),
                kind: ArrayKind::Member(0),
            }
        );
        assert_eq!(
            s.fields[1].name, "After",
            "the nested close ended the struct"
        );
    }

    /// ⚠ All 267 maps in the corpus state the key as an attribute and hold
    /// one child, the value. Reading the child as the key typed every map's
    /// value `Unknown("map_value")`, and 14 corpus documents refused on it.
    #[test]
    fn a_map_takes_its_key_from_the_attribute_and_its_value_from_the_child() {
        let s = only_struct(
            r#"<ParserSchema>
  <structdef type="X">
    <map name="m_Sets" type="atBinaryMap" key="atHashString"><struct type="CSet"/></map>
    <map name="m_ByEnum" type="atMap" key="enum" enumKeyType="eKey"><pointer type="Base" policy="owner"/></map>
    <map name="m_ByU32" type="atBinaryMap" key="u32"><array type="atArray"><float/></array></map>
    <map name="m_Bank" type="atBinaryMap" key="atHashWithStringBank"><int/></map>
  </structdef>
</ParserSchema>"#,
        );
        let map = |k: FieldType, v: FieldType| FieldType::Map {
            key: Box::new(k),
            value: Box::new(v),
        };
        let types: Vec<&FieldType> = s.fields.iter().map(|f| &f.ty).collect();
        assert_eq!(
            types,
            [
                &map(
                    FieldType::String(StringKind::AtHashString),
                    FieldType::Struct("CSet".into())
                ),
                &map(
                    FieldType::Enum("eKey".into()),
                    FieldType::Pointer {
                        base: "Base".into(),
                        policy: PointerPolicy::Owner
                    }
                ),
                &map(
                    FieldType::U32,
                    FieldType::Array {
                        elem: Box::new(FieldType::Float),
                        kind: ArrayKind::AtArray
                    }
                ),
                &map(FieldType::String(StringKind::AtHashString), FieldType::S32),
            ]
        );
    }

    #[test]
    fn every_hashed_string_spelling_is_a_hash() {
        let s = only_struct(
            r#"<ParserSchema>
  <structdef type="X">
    <string name="a" type="atHashWithStringBank"/>
    <string name="b" type="atHashWithStringDev"/>
    <string name="c" type="atHashWithStringNotFinal"/>
    <string name="d" type="atDiagHashString"/>
    <string name="e" type="atPartialHashValue"/>
    <string name="f" type="ConstString"/>
  </structdef>
</ParserSchema>"#,
        );
        let kinds: Vec<&FieldType> = s.fields.iter().map(|f| &f.ty).collect();
        assert_eq!(
            kinds,
            [
                &FieldType::String(StringKind::AtHashString),
                &FieldType::String(StringKind::AtHashString),
                &FieldType::String(StringKind::AtHashString),
                &FieldType::String(StringKind::AtHashString),
                &FieldType::String(StringKind::AtHashValue),
                &FieldType::String(StringKind::Plain),
            ]
        );
    }

    #[test]
    fn a_lower_case_spelling_reaches_the_same_type() {
        let xml = r#"<?xml version="1.0"?>
<ParserSchema>
  <structdef type="X">
    <color32 name="m_A"/>
    <Color32 name="m_B"/>
    <vector3 name="m_C"/>
    <Vector3 name="m_D"/>
  </structdef>
</ParserSchema>"#;
        let items = parse_psc(xml.as_bytes(), "t.psc").unwrap();
        let SchemaItem::Struct(s) = &items[0] else {
            panic!()
        };
        let types: Vec<&FieldType> = s.fields.iter().map(|f| &f.ty).collect();
        assert_eq!(types[0], types[1], "color32 and Color32 differ");
        assert_eq!(types[2], types[3], "vector3 and Vector3 differ");
        assert_eq!(*types[0], FieldType::Color32);
        assert_eq!(*types[2], FieldType::Vector3);
    }

    #[test]
    fn parses_bitset_field() {
        let xml = r#"<?xml version="1.0"?>
<ParserSchema>
  <structdef type="X">
    <bitset name="m_Flags" type="fixed32" values="MyEnum"/>
  </structdef>
</ParserSchema>"#;
        let items = parse_psc(xml.as_bytes(), "t.psc").unwrap();
        if let SchemaItem::Struct(s) = &items[0] {
            match &s.fields[0].ty {
                FieldType::Bitset { storage, values } => {
                    assert_eq!(*storage, BitsetStorage::Fixed32);
                    assert_eq!(values, "MyEnum");
                }
                _ => panic!(),
            }
        } else {
            panic!();
        }
    }

    #[test]
    fn parses_pointer_with_policy() {
        let xml = r#"<?xml version="1.0"?>
<ParserSchema>
  <structdef type="X">
    <pointer name="p" type="Base" policy="owner"/>
  </structdef>
</ParserSchema>"#;
        let items = parse_psc(xml.as_bytes(), "t.psc").unwrap();
        if let SchemaItem::Struct(s) = &items[0] {
            match &s.fields[0].ty {
                FieldType::Pointer { base, policy } => {
                    assert_eq!(base, "Base");
                    assert_eq!(*policy, PointerPolicy::Owner);
                }
                _ => panic!(),
            }
        } else {
            panic!();
        }
    }

    #[test]
    fn struct_inheritance_recorded() {
        let xml = r#"<?xml version="1.0"?>
<ParserSchema>
  <structdef type="Base" constructable="false"/>
  <structdef type="Derived" base="Base">
    <int name="m_X"/>
  </structdef>
</ParserSchema>"#;
        let items = parse_psc(xml.as_bytes(), "t.psc").unwrap();
        match &items[0] {
            SchemaItem::Struct(s) => {
                assert_eq!(s.name, "Base");
                assert!(s.abstract_);
            }
            _ => panic!(),
        }
        match &items[1] {
            SchemaItem::Struct(s) => {
                assert_eq!(s.name, "Derived");
                assert_eq!(s.base.as_deref(), Some("Base"));
                assert_eq!(s.fields[0].name, "X");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn enum_type_attribute_left_raw_for_cross_ref() {
        let xml = r#"<?xml version="1.0"?>
<ParserSchema>
  <structdef type="Z">
    <enum name="m_flags" type="eAnimSceneFlags"/>
  </structdef>
</ParserSchema>"#;
        let items = parse_psc(xml.as_bytes(), "t.psc").unwrap();
        if let SchemaItem::Struct(s) = &items[0] {
            match &s.fields[0].ty {
                FieldType::Enum(ty) => assert_eq!(ty, "eAnimSceneFlags"),
                t => panic!("{t:?}"),
            }
        } else {
            panic!();
        }
    }
}
