//! Struct and enum definitions recovered from the type tables PSO and RBM
//! documents carry, for an XML import with no schema snapshot.
//!
//! ⚠ **A table states a layout, not a declaration.** An embedded `char[N]`
//! comes back as a heap string and a `Matrix34` as `Mat34V`, so a document
//! typed by its own table may move bytes on its first canonical cycle and
//! none after.

use std::borrow::Cow;
use std::sync::OnceLock;

use crate::meta::binary::hash_index::default_hash_index;
use crate::meta::binary::layout::{ContainerLayout, MetaLayout};
use crate::meta::binary::pso::schema::{
    string_subtype, struct_subtype, PsoDataType, PsoSchemaItem, PsoSchemaSection, PsoStructEntry,
    PsoStructInfo, ARRAY_INFO_NAME_HASH,
};
use crate::meta::binary::rbm::schema::{PsoType, RbmClass, RbmClassEntry, RbmModel};
use crate::meta::binary::synth::VTABLE_BYTES;
use crate::meta::schema::{
    ArrayKind, BitsetStorage, EnumDef, EnumValue, Field, FieldHint, FieldType, PointerPolicy,
    SchemaLibrary, StringKind, StructDef,
};

/// What an install's type tables state, read once for the process.
static INSTALLED: OnceLock<SchemaLibrary> = OnceLock::new();

/// Install `library`, built from an install's documents, as the source an import
/// with no donor falls back to; the first installation stands.
pub fn install(library: SchemaLibrary) {
    let _ = INSTALLED.set(library);
}

/// The library [`install`] installed.
pub fn installed() -> Option<&'static SchemaLibrary> {
    INSTALLED.get()
}

/// Every struct and enum the type table `layout` was read with states.
pub fn of_layout(layout: &MetaLayout) -> SchemaLibrary {
    match &layout.container {
        ContainerLayout::Pso(p) => of_pso(&p.schema),
        ContainerLayout::Rbm(r) => of_rbm(&r.model),
        ContainerLayout::Rbf(_) => SchemaLibrary::default(),
    }
}

/// Add every struct and enum `from` states that `into` lacks.
pub fn fill(into: &mut SchemaLibrary, from: &SchemaLibrary) {
    for (name, def) in &from.structs {
        into.structs
            .entry(name.clone())
            .or_insert_with(|| def.clone());
    }
    for (name, def) in &from.enums {
        into.enums
            .entry(name.clone())
            .or_insert_with(|| def.clone());
    }
}

/// `library` with what `tables` states that it lacks.
///
/// ⚠ **A type a table only names is declared empty.** A table describes the
/// types its document contains, so an empty `atArray<T>` leaves `T`
/// undeclared, in 2,660 of the 3,527 PSO documents staged; an element that
/// reaches one carries only members no table declares, which the import
/// refuses by name.
pub fn extend<'a>(library: &'a SchemaLibrary, tables: &SchemaLibrary) -> Cow<'a, SchemaLibrary> {
    let added: Vec<&StructDef> = tables
        .structs
        .values()
        .filter(|d| library.struct_(&d.name).is_none())
        .collect();
    let enums_missing = tables.enums.keys().any(|n| library.enum_(n).is_none());
    if added.is_empty() && !enums_missing {
        return Cow::Borrowed(library);
    }
    let mut lib = library.clone();
    fill(&mut lib, tables);
    let mut structs = Vec::new();
    let mut enums = Vec::new();
    for def in &added {
        for f in &def.fields {
            referenced(&f.ty, &mut structs, &mut enums);
        }
    }
    for name in structs {
        if lib.struct_(&name).is_none() {
            lib.insert_struct(struct_def(name, true, Vec::new()));
        }
    }
    for name in enums {
        if !name.is_empty() && lib.enum_(&name).is_none() {
            lib.insert_enum(enum_def(name, Vec::new()));
        }
    }
    Cow::Owned(lib)
}

/// `declared`, then what `tables` states that it lacks, then `snapshot` for a type neither holds nor names.
pub fn layered(
    declared: &SchemaLibrary,
    tables: &SchemaLibrary,
    snapshot: Option<&SchemaLibrary>,
) -> SchemaLibrary {
    // ⚠ A document's own table outranks an installed snapshot: a snapshot is of one build of the game and a class
    // grows between builds, so a class laid out by the snapshot demands members the document's records do not hold
    // and leaves out the ones only they hold.
    // ⚠ A type `tables` only names stays declared empty, as `extend` declares it, whatever the snapshot declares of
    // it: the element class of an array that holds nothing is written into a synthesised table, so the snapshot's
    // class in its place would make a process that installed one write other bytes than a process with none.
    let own = extend(declared, tables).into_owned();
    let Some(snapshot) = snapshot else {
        return own;
    };
    let mut library = snapshot.clone();
    library.structs.extend(own.structs);
    library.enums.extend(own.enums);
    library
}

fn referenced(ty: &FieldType, structs: &mut Vec<String>, enums: &mut Vec<String>) {
    match ty {
        FieldType::Struct(name) => structs.push(name.clone()),
        FieldType::Enum(name) | FieldType::Bitset { values: name, .. } => enums.push(name.clone()),
        FieldType::Array { elem, .. } => referenced(elem, structs, enums),
        FieldType::Map { key, value } => {
            referenced(key, structs, enums);
            referenced(value, structs, enums);
        }
        _ => {}
    }
}

/// Every struct and enum a PSO type table states.
pub fn of_pso(schema: &PsoSchemaSection) -> SchemaLibrary {
    let idx = default_hash_index();
    let mut library = SchemaLibrary::new();
    for item in &schema.items {
        match item {
            PsoSchemaItem::Enum(info) => library.insert_enum(enum_def(
                idx.name_or_literal(info.name_hash),
                info.entries
                    .iter()
                    .map(|e| (e.name_hash, e.value))
                    .collect(),
            )),
            PsoSchemaItem::Struct(info) => library.insert_struct(struct_def(
                idx.name_or_literal(info.name_hash),
                true,
                info.entries
                    .iter()
                    .filter(|e| e.name_hash != ARRAY_INFO_NAME_HASH)
                    .map(|e| field(idx.name_or_literal(e.name_hash), pso_type(info, e)))
                    .collect(),
            )),
        }
    }
    library
}

/// Every class and enum an RBM type table states.
pub fn of_rbm(model: &RbmModel) -> SchemaLibrary {
    let idx = default_hash_index();
    let mut library = SchemaLibrary::new();
    for info in model.enum_order.iter().filter_map(|h| model.enum_for(*h)) {
        library.insert_enum(enum_def(
            idx.name_or_literal(info.name_hash),
            info.values.iter().map(|v| (v.name_hash, v.value)).collect(),
        ));
    }
    for class in model.class_order.iter().filter_map(|h| model.class_for(*h)) {
        // ⚠ A first member below the vtable word is the only sign a table gives
        // that a class has none.
        let first = class
            .entries
            .iter()
            .filter(|e| e.name_hash != ARRAY_INFO_NAME_HASH)
            .map(|e| e.offset as usize)
            .min();
        library.insert_struct(struct_def(
            idx.name_or_literal(class.name_hash),
            first.is_none_or(|o| o < VTABLE_BYTES),
            class
                .entries
                .iter()
                .filter(|e| e.name_hash != ARRAY_INFO_NAME_HASH)
                .map(|e| field(idx.name_or_literal(e.name_hash), rbm_type(class, e)))
                .collect(),
        ));
    }
    library
}

fn struct_def(name: String, simple: bool, fields: Vec<Field>) -> StructDef {
    StructDef {
        name,
        base: None,
        abstract_: false,
        simple,
        fields,
        source_path: String::new(),
    }
}

fn enum_def(name: String, values: Vec<(u32, i32)>) -> EnumDef {
    let idx = default_hash_index();
    EnumDef {
        name,
        values: values
            .into_iter()
            .map(|(hash, value)| EnumValue {
                name: idx.name_or_literal(hash),
                description: None,
                value: Some(i64::from(value)),
            })
            .collect(),
        generate_bitset: false,
        source_path: String::new(),
    }
}

fn field(name: String, ty: FieldType) -> Field {
    Field {
        name,
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

fn named(hash: u32) -> String {
    default_hash_index().name_or_literal(hash)
}

fn pso_type(owner: &PsoStructInfo, entry: &PsoStructEntry) -> FieldType {
    let sidecar = |index: Option<u16>, what: &str| {
        index
            .and_then(|i| owner.entry(i))
            .map(|e| pso_type(owner, e))
            .unwrap_or_else(|| FieldType::Unknown(what.into()))
    };
    match entry.data_type {
        PsoDataType::Bool | PsoDataType::BoolV => FieldType::Bool,
        PsoDataType::SByte => FieldType::S8,
        PsoDataType::UByte => FieldType::U8,
        PsoDataType::SShort => FieldType::S16,
        PsoDataType::UShort => FieldType::U16,
        PsoDataType::SInt => FieldType::S32,
        PsoDataType::UInt => FieldType::U32,
        PsoDataType::Int64 | PsoDataType::PtrDiffT => FieldType::S64,
        PsoDataType::UInt64 | PsoDataType::SizeT => FieldType::U64,
        PsoDataType::Float | PsoDataType::ScalarV => FieldType::Float,
        PsoDataType::Double => FieldType::Double,
        PsoDataType::Float16 => FieldType::Float16,
        PsoDataType::Vector2 | PsoDataType::Vec2V => FieldType::Vector2,
        PsoDataType::Vector3 | PsoDataType::Vec3V => FieldType::Vector3,
        PsoDataType::Vector4 | PsoDataType::Vec4V | PsoDataType::VecBoolV => FieldType::Vector4,
        PsoDataType::Mat33V => FieldType::Mat33V,
        PsoDataType::Matrix34 | PsoDataType::Mat34V => FieldType::Mat34V,
        PsoDataType::Matrix44 | PsoDataType::Mat44V => FieldType::Mat44V,
        PsoDataType::String => match entry.subtype() {
            string_subtype::HASH | string_subtype::HASH_ALT => {
                FieldType::String(StringKind::AtHashString)
            }
            string_subtype::CHAR_ARRAY => FieldType::String(StringKind::AtString),
            _ => FieldType::String(StringKind::Plain),
        },
        PsoDataType::Structure => match entry.subtype() {
            struct_subtype::EMBEDDED => FieldType::Struct(named(entry.referent_hash())),
            _ => FieldType::Pointer {
                base: named(entry.referent_hash()),
                policy: PointerPolicy::Owner,
            },
        },
        PsoDataType::Enum => FieldType::Enum(named(entry.referent_hash())),
        // ⚠ No enum: PSO synthesis writes a bitset with no member naming
        // one, so a name taken here would be gone from the first cycle's table
        // and the second would lay out another. The XML spells the bits as a
        // number, which needs none.
        PsoDataType::Bitset => FieldType::Bitset {
            storage: match entry.count() {
                0..=8 => BitsetStorage::Fixed8,
                9..=16 => BitsetStorage::Fixed16,
                17..=32 => BitsetStorage::Fixed32,
                _ => BitsetStorage::Fixed64,
            },
            values: String::new(),
        },
        PsoDataType::Array => FieldType::Array {
            elem: Box::new(sidecar(entry.member_index(), "array element")),
            kind: match entry.count() {
                0 => ArrayKind::AtArray,
                n => ArrayKind::AtFixedArray(u32::from(n)),
            },
        },
        PsoDataType::Map => FieldType::Map {
            key: Box::new(sidecar(Some(entry.key_index()), "map key")),
            value: Box::new(sidecar(Some(entry.value_index()), "map value")),
        },
        PsoDataType::Unknown(b) => FieldType::Unknown(format!("pso type {b:#04X}")),
    }
}

fn rbm_type(owner: &RbmClass, entry: &RbmClassEntry) -> FieldType {
    let sidecar = |index: u16, what: &str| {
        owner
            .entry(index)
            .map(|e| rbm_type(owner, e))
            .unwrap_or_else(|| FieldType::Unknown(what.into()))
    };
    let bitset = |storage| FieldType::Bitset {
        storage,
        values: named(entry.bitset_enum_hash()),
    };
    match entry.data_type {
        PsoType::Bool | PsoType::BoolV => FieldType::Bool,
        PsoType::S8 => FieldType::S8,
        PsoType::U8 => FieldType::U8,
        PsoType::S16 => FieldType::S16,
        PsoType::U16 => FieldType::U16,
        PsoType::S32 => FieldType::S32,
        PsoType::U32 => FieldType::U32,
        PsoType::S64 => FieldType::S64,
        PsoType::U64 => FieldType::U64,
        PsoType::Float | PsoType::ScalarV => FieldType::Float,
        PsoType::Double => FieldType::Double,
        PsoType::Float16 => FieldType::Float16,
        PsoType::Vec2 | PsoType::Vec2V => FieldType::Vector2,
        PsoType::Vec3 | PsoType::Vec3V => FieldType::Vector3,
        PsoType::Vec4V | PsoType::VecBoolV => FieldType::Vector4,
        PsoType::Mat33V | PsoType::Mat43 => FieldType::Mat33V,
        PsoType::Mat34V | PsoType::Mat44V => FieldType::Mat34V,
        PsoType::StringHash
        | PsoType::LiteralStringHash
        | PsoType::PartialStringHash
        | PsoType::NsStringHash
        | PsoType::StructId => FieldType::String(StringKind::AtHashString),
        PsoType::AtString32 | PsoType::AtString64 => FieldType::String(StringKind::AtString),
        PsoType::StringMember
        | PsoType::WideStringMember
        | PsoType::StringPointer32
        | PsoType::StringPointer64
        | PsoType::AtWideString32
        | PsoType::AtWideString64
        | PsoType::WideStringPointer32
        | PsoType::WideStringPointer64 => FieldType::String(StringKind::Plain),
        PsoType::Struct => FieldType::Struct(named(entry.referent_hash())),
        PsoType::Pointer32 | PsoType::Pointer64 => FieldType::Pointer {
            base: named(entry.referent_hash()),
            policy: PointerPolicy::Owner,
        },
        PsoType::Enum8 | PsoType::Enum16 | PsoType::Enum32 => {
            FieldType::Enum(named(entry.referent_hash()))
        }
        PsoType::Bitset8 => bitset(BitsetStorage::Fixed8),
        PsoType::Bitset16 => bitset(BitsetStorage::Fixed16),
        PsoType::Bitset32 => bitset(BitsetStorage::Fixed32),
        PsoType::AtBitset32 | PsoType::AtBitset64 => bitset(BitsetStorage::Fixed64),
        PsoType::ArrayMember | PsoType::AtFixedArray => FieldType::Array {
            elem: Box::new(sidecar(entry.contents_index(), "array element")),
            kind: ArrayKind::AtFixedArray(entry.array_count()),
        },
        PsoType::AtArray32
        | PsoType::AtArray64
        | PsoType::AtArray32With32BitIdx
        | PsoType::AtArray64With32BitIdx => FieldType::Array {
            elem: Box::new(sidecar(entry.contents_index(), "array element")),
            kind: ArrayKind::AtArray,
        },
        PsoType::AtBinaryMap32 | PsoType::AtBinaryMap64 => FieldType::Map {
            key: Box::new(sidecar(entry.map_key_index(), "map key")),
            value: Box::new(sidecar(entry.map_value_index(), "map value")),
        },
        // ⚠ A run another member counts is stated as the table states it, so a document typed by its own table lays
        // its occluder vertices out again; a run nothing counts has no length to state.
        PsoType::ArrayPointer32WithCount | PsoType::ArrayPointer64WithCount => {
            let counted_by = u16::try_from(entry.array_count())
                .ok()
                .and_then(|index| owner.entry(index));
            match counted_by {
                Some(count) => FieldType::Array {
                    elem: Box::new(sidecar(entry.contents_index(), "array element")),
                    kind: ArrayKind::Pointer {
                        size_var: Some(named(count.name_hash)),
                        align: match entry.array_align_power() {
                            0 => 0,
                            power => 1 << power,
                        },
                    },
                },
                None => FieldType::Unknown("raw array pointer".into()),
            }
        }
        PsoType::ArrayPointer32 | PsoType::ArrayPointer64 => {
            FieldType::Unknown("raw array pointer".into())
        }
        PsoType::Invalid => FieldType::Unknown("invalid".into()),
        PsoType::Unknown(b) => FieldType::Unknown(format!("rbm type {b:#04X}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::build_library;

    fn library(psc: &str) -> SchemaLibrary {
        let psc = format!("<ParserSchema>{psc}</ParserSchema>");
        build_library([("classes.psc", psc.as_bytes())]).expect("the classes parse")
    }

    /// The members `library` gives `class`, or `None` where it holds no such class.
    fn members(library: &SchemaLibrary, class: &str) -> Option<Vec<String>> {
        let def = library.struct_(class)?;
        Some(def.fields.iter().map(|f| f.name.clone()).collect())
    }

    #[test]
    fn a_documents_own_table_outranks_a_snapshot_and_a_type_it_only_names_stays_empty() {
        // The file states `Held` with a member of its own and names `Element` for an array that holds nothing.
        let tables = library(
            r#"<structdef type="Held">
                 <u32 name="shared"/>
                 <u32 name="onlyTheFileStates"/>
                 <array name="elements" type="atArray"><struct type="Element"/></array>
               </structdef>"#,
        );
        assert_eq!(members(&tables, "Element"), None, "the file only names it");
        let snapshot = library(
            r#"<structdef type="Held"><u32 name="shared"/><u32 name="ofALaterBuild"/></structdef>
               <structdef type="Element"><u32 name="ofTheSnapshot"/></structdef>
               <structdef type="Family"><u32 name="ofTheSnapshot"/></structdef>
               <structdef type="NoFileHolds"><u32 name="ofTheSnapshot"/></structdef>"#,
        );
        let declared =
            library(r#"<structdef type="Family"><u32 name="declaredInCode"/></structdef>"#);

        let with = layered(&declared, &tables, Some(&snapshot));
        let without = layered(&declared, &tables, None);
        let names = |names: &[&str]| Some(names.iter().map(|n| n.to_string()).collect::<Vec<_>>());
        assert_eq!(
            members(&with, "Held"),
            names(&["shared", "onlyTheFileStates", "elements"]),
            "as the file's own table states it"
        );
        assert_eq!(
            members(&with, "Element"),
            names(&[]),
            "a class the table only names is declared empty, whatever the snapshot declares of it"
        );
        assert_eq!(members(&with, "Family"), names(&["declaredInCode"]));
        assert_eq!(
            members(&with, "NoFileHolds"),
            names(&["ofTheSnapshot"]),
            "the snapshot gives a class the table does not hold"
        );
        // Whatever a process with no snapshot lays out, one that installed a snapshot lays out the same.
        assert_eq!(without.structs.len(), 3);
        for class in without.structs.keys() {
            assert_eq!(members(&with, class), members(&without, class), "{class}");
        }
        assert_eq!(members(&without, "NoFileHolds"), None);
    }
}
