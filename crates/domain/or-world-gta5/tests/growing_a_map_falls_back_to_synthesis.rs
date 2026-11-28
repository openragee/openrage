//! Adding an entity to a `.ymap` and getting bytes out, which is the whole
//! reason the RBM synthesiser exists.
//!
//! The map is assembled from the installed schema's own `CMapData` rather than
//! staged, so this measures the wiring in [`or_world_gta5::write::encode`] and
//! says nothing about a retail file; the metadata suites'
//! `an_array_that_grew_encodes_and_one_that_did_not_is_untouched` runs on
//! shipped bytes.

mod support;

use or_io::meta::binary::layout::{ContainerLayout, MetaLayout};
use or_io::meta::binary::rbm::schema::{PsoType, RbmClass, RbmClassEntry, RbmModel};
use or_io::meta::binary::synth::{rbm_layout_for, rbm_layout_for_instance};
use or_io::meta::binary::{parse_meta_binary_full, write_meta_binary};
use or_io::meta::instance::value::HashedString;
use or_io::meta::{BinaryFormat, Diagnostics, MetaInstance, Value};

const ARRAY_INFO_NAME_HASH: u32 = 0x100;

/// One value per member, so a synthesised layout is asked to hold something
/// in every slot it declared.
fn sample_instance(model: &RbmModel, class: &RbmClass) -> MetaInstance {
    let idx = support::hash_index();
    let mut inst = MetaInstance::new(idx.name_or_literal(class.name_hash));
    for entry in &class.entries {
        if entry.name_hash == ARRAY_INFO_NAME_HASH {
            continue;
        }
        inst.set(
            idx.name_or_literal(entry.name_hash),
            sample_for(model, class, entry),
        );
    }
    inst
}

fn sample_for(model: &RbmModel, owner: &RbmClass, entry: &RbmClassEntry) -> Value {
    use PsoType as T;
    match entry.data_type {
        T::Bool | T::BoolV => Value::Bool(false),
        T::S8 | T::S16 | T::S32 | T::S64 => Value::Int(0),
        T::U8 | T::U16 | T::U32 | T::U64 => Value::UInt(0),
        T::Float | T::ScalarV | T::Double | T::Float16 => Value::Float(0.0),
        T::Vec2 | T::Vec2V => Value::Vec2([0.0; 2]),
        T::Vec3 | T::Vec3V => Value::Vec3([0.0; 3]),
        T::Vec4V | T::VecBoolV => Value::Vec4([0.0; 4]),
        T::Mat33V | T::Mat43 | T::Mat34V | T::Mat44V => Value::Mat34([[0.0; 4]; 3]),
        T::StringHash
        | T::LiteralStringHash
        | T::PartialStringHash
        | T::NsStringHash
        | T::StructId => Value::Hash(HashedString::new("sample")),
        T::StringMember
        | T::WideStringMember
        | T::StringPointer32
        | T::StringPointer64
        | T::AtString32
        | T::AtString64 => Value::String("sample".into()),
        T::AtWideString32 | T::AtWideString64 | T::WideStringPointer32 | T::WideStringPointer64 => {
            Value::String(String::new())
        }
        T::Enum8 | T::Enum16 | T::Enum32 => Value::Enum {
            type_name: String::new(),
            variant: "0x00000000".into(),
        },
        T::Bitset8 | T::Bitset16 | T::Bitset32 | T::AtBitset32 | T::AtBitset64 => Value::Bitset {
            type_name: String::new(),
            bits: 0,
        },
        T::Struct => match model.class_for(entry.referent_hash()) {
            Some(inner) => Value::Struct(Box::new(sample_instance(model, inner))),
            None => Value::Null,
        },
        T::Pointer32 | T::Pointer64 => Value::Pointer(None),
        T::ArrayMember | T::AtFixedArray => {
            let elem = owner.entry(entry.contents_index()).copied();
            Value::Array(
                (0..entry.array_count())
                    .map(|_| match elem {
                        Some(e) => sample_for(model, owner, &e),
                        None => Value::Null,
                    })
                    .collect(),
            )
        }
        T::AtArray32 | T::AtArray64 | T::AtArray32With32BitIdx | T::AtArray64With32BitIdx => {
            Value::Array(Vec::new())
        }
        T::AtBinaryMap32 | T::AtBinaryMap64 => Value::Map(Vec::new()),
        T::ArrayPointer32
        | T::ArrayPointer64
        | T::ArrayPointer32WithCount
        | T::ArrayPointer64WithCount => Value::Null,
        T::Invalid | T::Unknown(_) => Value::Null,
    }
}

/// A `CEntityDef` the validator accepts: no LOD parent, no children.
///
/// ⚠ Built from its **own** shape rather than from the map's. A pointer's
/// target type comes from the value tree, so an empty `CMapData` does not
/// reach `CEntityDef` and its class is not in that table at all.
fn an_entity(lib: &or_io::meta::SchemaLibrary) -> MetaInstance {
    let shape = rbm_layout_for(lib, "CEntityDef").expect("CEntityDef lays out");
    let hash = or_io::meta::binary::synth::name_hash("CEntityDef");
    let class = shape
        .model
        .class_for(hash)
        .expect("the root is in its own table")
        .clone();
    let mut ent = sample_instance(&shape.model, &class);
    ent.set("parentIndex", Value::Int(-1));
    ent.set("numChildren", Value::UInt(0));
    ent
}

fn entities_of(inst: &MetaInstance) -> usize {
    match inst.get("entities") {
        Some(Value::Array(items)) => items.len(),
        _ => 0,
    }
}

#[test]
fn adding_an_entity_to_a_map_encodes_and_reads_back() {
    let lib = support::schema();
    if lib.struct_("CMapData").is_none() || lib.struct_("CEntityDef").is_none() {
        or_test_support::skip_corpus(
            "adding_an_entity_to_a_map_encodes_and_reads_back",
            "the installed schema library has no CMapData; see crates/io/or-io/src/meta/README.md",
        );
        return;
    }

    // A shape first, so the sample can be built against the very type table
    // that will describe it.
    let shape = rbm_layout_for(lib, "CMapData").expect("CMapData lays out");
    let root_hash = or_io::meta::binary::synth::name_hash("CMapData");
    let root_class = shape
        .model
        .class_for(root_hash)
        .expect("the root is in its own table")
        .clone();
    let mut map = sample_instance(&shape.model, &root_class);
    let entity = an_entity(lib);
    map.set(
        "entities",
        Value::Array(vec![Value::Pointer(Some(Box::new(entity.clone())))]),
    );

    let laid_out = rbm_layout_for_instance(lib, &map, None).expect("the map lays out");
    let layout = MetaLayout {
        format: BinaryFormat::Rbm,
        container: ContainerLayout::Rbm(Box::new(laid_out)),
    };
    let bytes = write_meta_binary(BinaryFormat::Rbm, Some(&layout), &map).expect("encodes");

    let mut diags = Diagnostics::lenient();
    let (mut document, donor) = parse_meta_binary_full(lib, &bytes, &mut diags).expect("re-reads");
    assert_eq!(document.type_name, "CMapData");
    assert_eq!(
        entities_of(&document),
        1,
        "the map was built with one entity"
    );

    // Nothing has changed yet, so the writer's own path has to be enough and
    // the bytes have to come back the same.
    let unchanged = or_world_gta5::write::encode(&document, &donor).expect("re-encodes");
    assert_eq!(
        unchanged[..16],
        bytes[..16],
        "the RSC7 header moved for a document that did not change"
    );

    // Now grow it. The document's own allocation holds one entity, so this
    // is the case that has to reach synthesis.
    let Some(Value::Array(items)) = document.get("entities").cloned() else {
        panic!("entities is not an array");
    };
    let mut items = items;
    items.push(Value::Pointer(Some(Box::new(entity))));
    document.set("entities", Value::Array(items));

    let refused = write_meta_binary(BinaryFormat::Rbm, Some(&donor), &document)
        .expect_err("the document's own layout holds one entity, not two");
    assert!(
        refused.outgrew_its_layout(),
        "the writer refused for some other reason, so the fallback would never fire: {refused}"
    );

    let grown = or_world_gta5::write::encode(&document, &donor).expect("the fallback encodes");
    let mut diags = Diagnostics::lenient();
    let (back, _) = parse_meta_binary_full(lib, &grown, &mut diags).expect("the grown map reads");
    assert_eq!(
        entities_of(&back),
        2,
        "the added entity did not reach the file"
    );
}

/// ⚠ **A refusal that is not about the allocation must not be retried.**
/// Synthesis rewrites every byte of a document, so reaching it for the wrong
/// reason turns a failed save into a silently different file.
#[test]
fn a_refusal_that_is_not_about_the_allocation_is_not_retried() {
    let err = or_io::meta::MetaError::Other("something else went wrong".into());
    assert!(!err.outgrew_its_layout());
    let outgrown = or_io::meta::MetaError::OutgrewItsLayout("an array grew".into());
    assert!(outgrown.outgrew_its_layout());
    // And saying where it happened must not change the answer.
    assert!(outgrown
        .in_context("CMapData.entities")
        .outgrew_its_layout());
    assert!(!err.in_context("CMapData.entities").outgrew_its_layout());
}
