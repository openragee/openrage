//! A parser dump, the JSON a viewer of an executable's reflection tables carries, read into a
//! schema library. A name the dump has only as a hash is spelled `hash_XXXXXXXX`.
//!
//! ⚠ A library holds no offset and no size, so the dump's are dropped: a binary body states its
//! own, and the library is read for names and for what a member is.

use serde::Deserialize;

use crate::meta::error::{MetaError, Result};
use crate::meta::schema::{
    ArrayKind, BitsetStorage, EnumDef, EnumValue, Field, FieldHint, FieldType, PointerPolicy,
    SchemaLibrary, StringKind, StructDef,
};

#[derive(Deserialize)]
struct Tree {
    #[serde(default)]
    structs: Vec<Structure>,
    #[serde(default)]
    enums: Vec<Enumeration>,
}

#[derive(Deserialize)]
struct Structure {
    name: String,
    markup: String,
    #[serde(default)]
    fields: Vec<Member>,
    /// The structures derived from this one, each with its own members only.
    #[serde(default)]
    children: Vec<Structure>,
}

#[derive(Deserialize)]
struct Member {
    name: String,
    offset: u32,
    subtype: String,
}

#[derive(Deserialize)]
struct Enumeration {
    name: String,
    markup: String,
}

/// The library the parser dump `json` declares.
pub fn library_from_par_tree(json: &[u8]) -> Result<SchemaLibrary> {
    let tree: Tree = serde_json::from_slice(json)
        .map_err(|e| MetaError::Other(format!("not a parser dump: {e}")))?;
    let mut library = SchemaLibrary::new();
    for structure in &tree.structs {
        insert(&mut library, structure)?;
    }
    for enumeration in &tree.enums {
        let values = (enumeration.markup.lines())
            .filter_map(|line| line.trim().strip_suffix(',')?.split_once(" = "))
            .map(|(name, value)| EnumValue {
                name: spelled(name),
                description: None,
                value: value.parse().ok(),
            })
            .collect();
        library.insert_enum(EnumDef {
            name: spelled(&enumeration.name),
            values,
            generate_bitset: false,
            source_path: String::new(),
        });
    }
    Ok(library)
}

fn insert(library: &mut SchemaLibrary, structure: &Structure) -> Result<()> {
    let mut lines = structure.markup.lines();
    // `$struct$ =@Name@ : @Base@`, the base only where there is one.
    let base = (lines.next())
        .and_then(|head| head.split_once(" : "))
        .and_then(|(_, base)| named(base))
        .map(|(name, _)| spelled(name));
    let mut fields = Vec::new();
    // The markup is in the order of declaration, which the member list does not keep.
    let members = lines.map(str::trim).filter(|line| line.starts_with('$'));
    for line in members.filter_map(|line| line.strip_suffix(';')) {
        let (stated, name) = line.rsplit_once(' ').ok_or_else(|| {
            MetaError::Other(format!("{}: `{line}` declares no member", structure.name))
        })?;
        let subtype = (structure.fields.iter())
            .find(|m| m.name == name)
            .map_or("", |m| m.subtype.as_str());
        fields.push(Field {
            name: spelled(name),
            ty: field_type(stated, subtype).0,
            init: None,
            min: None,
            max: None,
            step: None,
            ui_key: false,
            hide_widgets: false,
            description: None,
            hint: FieldHint::None,
        });
    }
    library.insert_struct(StructDef {
        name: spelled(&structure.name),
        base: base.clone(),
        abstract_: false,
        // No member below 8 means a virtual table's pointer stands there.
        simple: base.is_none() && structure.fields.iter().any(|m| m.offset < 8),
        fields,
        source_path: String::new(),
    });
    structure
        .children
        .iter()
        .try_for_each(|c| insert(library, c))
}

/// `_0xXXXXXXXX` as the tree spells a hash, and any other name as it is.
fn spelled(name: &str) -> String {
    match name.strip_prefix("_0x") {
        Some(hex) if hex.len() == 8 && hex.bytes().all(|b| b.is_ascii_hexdigit()) => {
            format!("hash_{}", hex.to_ascii_uppercase())
        }
        _ => name.to_string(),
    }
}

/// The name between the first two `@` of `text`, and what follows it.
fn named(text: &str) -> Option<(&str, &str)> {
    text.split_once('@')?.1.split_once('@')
}

/// The type the markup `stated` spells, and what of `stated` follows it. `subtype` is the
/// member's own and says nothing of an element's.
fn field_type<'a>(stated: &'a str, subtype: &str) -> (FieldType, &'a str) {
    let stated = stated.trim_start();
    let Some((word, rest)) = (stated.strip_prefix('$')).and_then(|s| s.split_once('$')) else {
        return (FieldType::Unknown(stated.to_string()), "");
    };
    let scalar = |ty| (ty, rest);
    match word {
        "bool" => scalar(FieldType::Bool),
        "char" => scalar(FieldType::S8),
        "uchar" => scalar(FieldType::U8),
        "short" => scalar(FieldType::S16),
        "ushort" => scalar(FieldType::U16),
        "int" => scalar(FieldType::S32),
        "uint" if subtype == "COLOR" => scalar(FieldType::Color32),
        "uint" => scalar(FieldType::U32),
        "int64" | "ptrdiff_t" => scalar(FieldType::S64),
        "uint64" | "size_t" => scalar(FieldType::U64),
        "float" => scalar(FieldType::Float),
        "float16" => scalar(FieldType::Float16),
        "double" => scalar(FieldType::Double),
        "Vec2f" => scalar(FieldType::Vector2),
        "Vec2V" => scalar(FieldType::Vec2V),
        "Vec3V" => scalar(FieldType::Vec3V),
        "Vec4V" | "QuatV" => scalar(FieldType::Vec4V),
        "Mat33V" => scalar(FieldType::Mat33V),
        "Mat34V" => scalar(FieldType::Mat34V),
        "Mat44V" => scalar(FieldType::Mat44V),
        "string" => scalar(match subtype {
            "ATFINALHASHSTRING" => FieldType::String(StringKind::AtFinalHashString),
            "ATHASHVALUE" | "ATPARTIALHASHVALUE" | "ATNSHASHVALUE" => {
                FieldType::String(StringKind::AtHashValue)
            }
            "ATSTRING" => FieldType::String(StringKind::AtString),
            "CONST_STRING" | "POINTER" | "MEMBER" => FieldType::String(StringKind::Plain),
            // ponytail: an element's kind is not stated; a hash string is by far the commonest.
            "" | "ATNONFINALHASHSTRING" | "ATNSHASHSTRING" => {
                FieldType::String(StringKind::AtHashString)
            }
            other => FieldType::Unknown(other.to_string()),
        }),
        "enum" => match named(rest) {
            Some((name, rest)) => (FieldType::Enum(spelled(name)), rest),
            None => scalar(FieldType::Unknown(word.to_string())),
        },
        "struct" => {
            // `@Name@`, or `$void$` for a pointer to anything, then `*` for a pointer.
            let (name, rest) = named(rest)
                .filter(|_| rest.trim_start().starts_with('@'))
                .or_else(|| rest.trim_start().strip_prefix('$')?.split_once('$'))
                .unwrap_or(("void", rest));
            let (pointer, rest) = match rest.strip_prefix('*') {
                Some(rest) => (true, rest),
                None => (false, rest),
            };
            let ty = match (pointer, subtype) {
                (false, "" | "STRUCTURE") => FieldType::Struct(spelled(name)),
                (true, "EXTERNAL_NAMED") => FieldType::Pointer {
                    base: spelled(name),
                    policy: PointerPolicy::ExternalNamed,
                },
                (true, "" | "POINTER" | "SIMPLE_POINTER") => FieldType::Pointer {
                    base: spelled(name),
                    policy: PointerPolicy::Owner,
                },
                (_, other) => FieldType::Unknown(other.to_string()),
            };
            (ty, rest)
        }
        "bitset" => {
            let (inner, rest) = field_type(rest.strip_prefix('<').unwrap_or(rest), "");
            let storage = match subtype {
                "_8BIT" => BitsetStorage::Fixed8,
                "_16BIT" => BitsetStorage::Fixed16,
                "_64BIT" => BitsetStorage::Fixed64,
                "ATBITSET" => BitsetStorage::Variable,
                _ => BitsetStorage::Fixed32,
            };
            let values = match inner {
                FieldType::Enum(name) => name,
                _ => String::new(),
            };
            (FieldType::Bitset { storage, values }, closed(rest))
        }
        "array" => {
            let (elem, rest) = field_type(rest.strip_prefix('<').unwrap_or(rest), "");
            // `, N` before the closing bracket where the count is the type's own.
            let (count, rest) = match after_comment(rest).strip_prefix(", ") {
                Some(counted) => {
                    let digits = counted.len()
                        - counted
                            .trim_start_matches(|c: char| c.is_ascii_digit())
                            .len();
                    (counted[..digits].parse().unwrap_or(0), &counted[digits..])
                }
                None => (0, rest),
            };
            let kind = match subtype {
                "ATFIXEDARRAY" => ArrayKind::AtFixedArray(count),
                "MEMBER" | "ATRANGEARRAY" => ArrayKind::Member(count),
                "POINTER" | "POINTER_WITH_COUNT" => ArrayKind::Pointer {
                    size_var: None,
                    align: 0,
                },
                _ if count > 0 => ArrayKind::Member(count),
                // ponytail: every counted array of another header reads as an `atArray`; the
                // model has no kind for them and a body states its own.
                _ => ArrayKind::AtArray,
            };
            let elem = Box::new(elem);
            (FieldType::Array { elem, kind }, closed(rest))
        }
        "map" => {
            let (key, rest) = field_type(rest.strip_prefix('<').unwrap_or(rest), "");
            let rest = after_comment(rest);
            let (value, rest) = field_type(rest.strip_prefix(", ").unwrap_or(rest), "");
            let (key, value) = (Box::new(key), Box::new(value));
            (FieldType::Map { key, value }, closed(rest))
        }
        other => scalar(FieldType::Unknown(other.to_string())),
    }
}

/// `rest` without a `/* … */` it starts with.
fn after_comment(rest: &str) -> &str {
    let trimmed = rest.trim_start();
    match trimmed.strip_prefix("/*").and_then(|c| c.split_once("*/")) {
        Some((_, after)) => after.trim_start(),
        None => rest,
    }
}

/// `rest` after the `>` that closes a bracket, with any comment before it.
fn closed(rest: &str) -> &str {
    let rest = after_comment(rest);
    rest.strip_prefix('>').unwrap_or(rest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::binary::hash_index::HashIndex;

    /// A made-up dump: a base with a virtual table, a derived structure, and an enum.
    const DUMP: &str = r#"{
      "structs": [
        { "name": "ZqBase", "hash": "_0x00000001", "size": 16, "align": 8,
          "markup": "$struct$ =@ZqBase@\r\n{\r\n\t$float$ Reach;\r\n};\r\n",
          "fields": [ { "name": "Reach", "offset": 8, "size": 4, "align": 4, "type": "FLOAT", "subtype": "NONE" } ],
          "children": [
            { "name": "_0x1234abcd", "hash": "_0x1234ABCD", "size": 96, "align": 8,
              "markup": "$struct$ =@_0x1234abcd@ : @ZqBase@\r\n{\r\n\t$string$ Label;\r\n\t$enum$ @ZqKind@ _0x0BADF00D;\r\n\t$array$<$struct$ @ZqBase@*> Parts;\r\n\t$array$<$float$, 3> Weights;\r\n\t$map$<$string$, $array$<$struct$ @ZqBase@>> ByName;\r\n\t$map$<$enum$ @ZqKind@, $struct$ @ZqBase@ /* note */> ByKind;\r\n\t$bitset$<$enum$ @ZqKind@> Flags;\r\n\t$struct$ $void$* Any;\r\n\t$array$<$bitset$<$enum$ @ZqKind@> /* Flags */> Masks;\r\n\t$UnkThing$ Odd;\r\n\t$uint$ Tint;\r\n};\r\n",
              "fields": [
                { "name": "_0x0BADF00D", "offset": 20, "size": 4, "align": 4, "type": "ENUM", "subtype": "_32BIT" },
                { "name": "Label", "offset": 16, "size": 4, "align": 4, "type": "STRING", "subtype": "ATFINALHASHSTRING" },
                { "name": "Parts", "offset": 24, "size": 16, "align": 8, "type": "ARRAY", "subtype": "ATARRAY" },
                { "name": "Weights", "offset": 40, "size": 12, "align": 4, "type": "ARRAY", "subtype": "MEMBER" },
                { "name": "Flags", "offset": 88, "size": 2, "align": 2, "type": "BITSET", "subtype": "_16BIT" },
                { "name": "Tint", "offset": 92, "size": 4, "align": 4, "type": "UINT", "subtype": "COLOR" }
              ] }
          ] }
      ],
      "enums": [
        { "name": "ZqKind", "hash": "_0x00000002",
          "markup": "$enum$ =@ZqKind@\r\n{\r\n\tZQ_None = 0,\r\n\t_0xcafe0001 = -2,\r\n};\r\n" }
      ]
    }"#;

    #[test]
    fn a_dump_reads_as_a_library_with_hashed_names_spelled_as_hashes() {
        let library = library_from_par_tree(DUMP.as_bytes()).expect("the dump reads");
        let base = library.struct_("ZqBase").expect("the base");
        assert!(
            !base.simple,
            "its first member stands after a virtual table"
        );
        assert_eq!(base.fields[0].ty, FieldType::Float);

        let derived = library.struct_("hash_1234ABCD").expect("a hashed name");
        assert_eq!(derived.base.as_deref(), Some("ZqBase"));
        let ty = |name: &str| {
            let field = derived.fields.iter().find(|f| f.name == name);
            field.unwrap_or_else(|| panic!("{name}")).ty.clone()
        };
        let base_struct = || FieldType::Struct("ZqBase".into());
        let kind = || FieldType::Enum("ZqKind".into());
        let hash_string = || FieldType::String(StringKind::AtHashString);
        let array = |elem, kind| FieldType::Array {
            elem: Box::new(elem),
            kind,
        };
        // The markup's order, not the member list's.
        assert_eq!(derived.fields[0].name, "Label");
        assert_eq!(
            ty("Label"),
            FieldType::String(StringKind::AtFinalHashString)
        );
        assert_eq!(ty("hash_0BADF00D"), kind());
        let owner = FieldType::Pointer {
            base: "ZqBase".into(),
            policy: PointerPolicy::Owner,
        };
        assert_eq!(ty("Parts"), array(owner, ArrayKind::AtArray));
        assert_eq!(ty("Weights"), array(FieldType::Float, ArrayKind::Member(3)));
        assert_eq!(
            ty("ByName"),
            FieldType::Map {
                key: Box::new(hash_string()),
                value: Box::new(array(base_struct(), ArrayKind::AtArray)),
            }
        );
        assert_eq!(
            ty("ByKind"),
            FieldType::Map {
                key: Box::new(kind()),
                value: Box::new(base_struct()),
            }
        );
        let bitset = |storage| FieldType::Bitset {
            storage,
            values: "ZqKind".into(),
        };
        assert_eq!(ty("Flags"), bitset(BitsetStorage::Fixed16));
        assert_eq!(
            ty("Any"),
            FieldType::Pointer {
                base: "void".into(),
                policy: PointerPolicy::Owner,
            }
        );
        assert_eq!(
            ty("Masks"),
            array(bitset(BitsetStorage::Fixed32), ArrayKind::AtArray)
        );
        assert_eq!(ty("Odd"), FieldType::Unknown("UnkThing".into()));
        assert_eq!(ty("Tint"), FieldType::Color32);

        let values = &library.enum_("ZqKind").expect("the enum").values;
        assert_eq!(values[0].name, "ZQ_None");
        assert_eq!(
            (values[1].name.as_str(), values[1].value),
            ("hash_CAFE0001", Some(-2))
        );
    }

    #[test]
    fn a_converted_library_names_its_hashes_and_survives_the_snapshot() {
        let library = library_from_par_tree(DUMP.as_bytes()).expect("the dump reads");
        let bytes = crate::meta::encode_snapshot(&library).expect("encodes");
        let back = crate::meta::decode_snapshot(&bytes).expect("decodes");
        let index = HashIndex::from_library(&back);
        for name in ["ZqBase", "Reach", "ByKind", "ZqKind", "ZQ_None"] {
            let hash = crate::literal_string_hash(name.as_bytes());
            assert_eq!(index.lookup(hash), Some(name));
        }
        // A name that is only a hash names nothing, under its own hash or the hash of its spelling.
        assert_eq!(index.lookup(0x1234_ABCD), None);
        assert_eq!(
            index.lookup(crate::literal_string_hash(b"hash_1234ABCD")),
            None
        );
        assert_eq!(index.name_or_literal(0x1234_ABCD), "hash_1234ABCD");
    }

    #[test]
    fn what_is_not_a_dump_is_refused() {
        assert!(library_from_par_tree(b"<xml/>").is_err());
        let no_member =
            r#"{"structs":[{"name":"Zq","markup":"$struct$ =@Zq@\r\n{\r\n\t$float$;\r\n};\r\n"}]}"#;
        assert!(library_from_par_tree(no_member.as_bytes()).is_err());
    }
}
