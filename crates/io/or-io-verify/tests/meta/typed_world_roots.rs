//! The world family opened as typed roots: every staged `.ymap` and `.ytyp`
//! against the value tree it was read from, and one position edit that must
//! reach the file as that edit and nothing else.

use crate::support;

use std::path::{Path, PathBuf};

use or_io::decl::File;
use or_io::decl::{Vec3, Vec4};
use or_io::hash::literal_string_hash;
use or_io::meta::binary::pso::schema::ARRAY_INFO_NAME_HASH;
use or_io::meta::binary::rbm::schema::{PsoType, RbmClass, RbmClassEntry, RbmModel};
use or_io::meta::binary::synth::{name_hash, rbm_layout_for, rbm_layout_for_instance};
use or_io::meta::binary::{ContainerLayout, MetaLayout};
use or_io::meta::instance::HashedString;
use or_io::meta::{
    write_meta_binary, BinaryFormat, BoxOccluder, CBaseArchetypeDef, CBaseArchetypeDefAny, CCarGen,
    CDistantLODLight, CEntityDef, CEntityDefAny, CLODLight, CMapData, CMapTypes, CMloArchetypeDef,
    CMloEntitySet, CMloInstanceDef, CMloPortalDef, CMloRoomDef, CMloTimeCycleModifier,
    CTimeArchetypeDef, CTimeCycleModifier, FloatXYZ, MetaDocument, MetaInstance, OccludeModel,
    Value,
};
use or_io_verify::gates::{corpus_root, name};
use or_test_support::ledger::{self, SkipReason};
use or_test_support::Corpus;
use std::ops::Deref;

fn staged(ext: &str, what: &str) -> Option<Vec<PathBuf>> {
    let Some(root) = corpus_root() else {
        ledger::record(SkipReason::Corpus, what, "no corpus is staged");
        return None;
    };
    Corpus::with_root(root, ext)
        .all_targets()
        .find_or_skip(what)
}

fn field_mut<'a>(inst: &'a mut MetaInstance, name: &str) -> Option<&'a mut Value> {
    let key = literal_string_hash(name.as_bytes());
    inst.fields
        .iter_mut()
        .find(|(n, _)| name_hash(n) == key)
        .map(|(_, v)| v)
}

fn struct_mut(value: &mut Value) -> Option<&mut MetaInstance> {
    match value {
        Value::Struct(inner) | Value::Pointer(Some(inner)) => Some(inner),
        _ => None,
    }
}

/// Raises the `index`th entity of a `CMapData` tree by a metre, as the typed
/// edit does, and reports whether the tree had that entity.
fn raise_in_tree(root: &mut MetaInstance, index: usize) -> bool {
    let Some(Value::Array(entities)) = field_mut(root, "entities") else {
        return false;
    };
    let Some(entity) = entities.get_mut(index).and_then(struct_mut) else {
        return false;
    };
    match field_mut(entity, "position") {
        Some(Value::Vec3(p)) => p[2] += 1.0,
        Some(Value::Vec4(p)) => p[2] += 1.0,
        _ => return false,
    }
    true
}

#[test]
fn every_corpus_ymap_opens_typed_and_saves_to_its_fixed_point() {
    const WHAT: &str = "every_corpus_ymap_opens_typed_and_saves_to_its_fixed_point";
    let Some(files) = staged("ymap", WHAT) else {
        return;
    };
    let library = support::schema();
    let mut failures = Vec::new();
    for path in &files {
        let file = name(path);
        let bytes = std::fs::read(path).expect("a corpus file reads");
        let tree = match MetaDocument::parse(library, bytes.clone()) {
            Ok(tree) => tree,
            Err(e) => {
                failures.push(format!("{file}: the value tree does not read: {e}"));
                continue;
            }
        };
        let map = match CMapData::from_bytes(&bytes) {
            Ok(map) => map,
            Err(e) => {
                failures.push(format!("{file}: does not open as CMapData: {e}"));
                continue;
            }
        };
        let once = match map.to_bytes() {
            Ok(b) => b,
            Err(e) => {
                failures.push(format!("{file}: an untouched save failed: {e}"));
                continue;
            }
        };
        if tree.to_bytes().ok().as_ref() != Some(&once) {
            failures.push(format!(
                "{file}: an untouched save is not the value tree's own save"
            ));
            continue;
        }
        match CMapData::from_bytes(&once).and_then(|m| m.to_bytes()) {
            Ok(twice) if twice == once => {}
            Ok(_) => {
                failures.push(format!("{file}: a second save differs from the first"));
                continue;
            }
            Err(e) => {
                failures.push(format!("{file}: the save does not reopen: {e}"));
                continue;
            }
        }
        if map.entities.is_empty() {
            continue;
        }
        let mut edited = map.clone();
        edited.entities[0].position.z += 1.0;
        let mut expected = tree.clone();
        assert!(raise_in_tree(&mut expected.instance, 0), "{file}");
        match (edited.to_bytes(), expected.to_bytes()) {
            (Ok(saved), Ok(want)) if saved == want => match CMapData::from_bytes(&saved) {
                Ok(back) if back == edited => {}
                Ok(_) => failures.push(format!("{file}: the edit does not reopen as saved")),
                Err(e) => failures.push(format!("{file}: the edit does not reopen: {e}")),
            },
            (Ok(_), Ok(_)) => failures.push(format!(
                "{file}: the position edit changed more than the position"
            )),
            (Err(e), _) => failures.push(format!("{file}: the edit does not save: {e}")),
            (_, Err(e)) => failures.push(format!("{file}: the edited tree does not save: {e}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} .ymap files:\n  {}",
        failures.len(),
        files.len(),
        failures
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

#[test]
fn every_corpus_ytyp_opens_typed_and_saves_to_its_fixed_point() {
    const WHAT: &str = "every_corpus_ytyp_opens_typed_and_saves_to_its_fixed_point";
    let Some(files) = staged("ytyp", WHAT) else {
        return;
    };
    let library = support::schema();
    let mut failures = Vec::new();
    for path in &files {
        let file = name(path);
        let bytes = std::fs::read(path).expect("a corpus file reads");
        let result = MetaDocument::parse(library, bytes.clone())
            .map_err(|e| e.to_string())
            .and_then(|tree| {
                let types = CMapTypes::from_bytes(&bytes).map_err(|e| e.to_string())?;
                let once = types.to_bytes().map_err(|e| e.to_string())?;
                if tree.to_bytes().ok().as_ref() != Some(&once) {
                    return Err("an untouched save is not the value tree's own save".into());
                }
                let twice = CMapTypes::from_bytes(&once)
                    .and_then(|t| t.to_bytes())
                    .map_err(|e| e.to_string())?;
                if twice != once {
                    return Err("a second save differs from the first".into());
                }
                Ok(())
            });
        if let Err(e) = result {
            failures.push(format!("{file}: {e}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} .ytyp files:\n  {}",
        failures.len(),
        files.len(),
        failures
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

/// One value per member, so a synthesised layout has something for every slot.
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

/// Every member the installed schema declares for `type_name`, sampled.
fn sampled(type_name: &str) -> MetaInstance {
    let shape = rbm_layout_for(support::schema(), type_name).expect("the type lays out");
    let class = shape
        .model
        .class_for(literal_string_hash(type_name.as_bytes()))
        .expect("the type is in its own table")
        .clone();
    sample_instance(&shape.model, &class)
}

fn set(inst: &mut MetaInstance, name: &str, value: Value) {
    match field_mut(inst, name) {
        Some(slot) => *slot = value,
        None => inst.set(name, value),
    }
}

fn entity(
    start: fn(&str) -> MetaInstance,
    type_name: &str,
    archetype: &str,
    guid: u32,
    position: [f32; 3],
) -> MetaInstance {
    let mut e = start(type_name);
    set(
        &mut e,
        "archetypeName",
        Value::Hash(HashedString::new(archetype)),
    );
    set(&mut e, "flags", Value::UInt(0x0010_0000));
    set(&mut e, "guid", Value::UInt(guid as u64));
    set(&mut e, "position", Value::Vec3(position));
    set(&mut e, "rotation", Value::Vec4([0.0, 0.0, 0.0, 1.0]));
    set(&mut e, "scaleXY", Value::Float(1.0));
    set(&mut e, "scaleZ", Value::Float(1.0));
    set(&mut e, "parentIndex", Value::Int(-1));
    set(&mut e, "lodDist", Value::Float(120.0));
    set(&mut e, "childLodDist", Value::Float(0.0));
    set(&mut e, "numChildren", Value::UInt(0));
    set(&mut e, "ambientOcclusionMultiplier", Value::Int(255));
    set(&mut e, "artificialAmbientOcclusion", Value::Int(255));
    set(&mut e, "tintValue", Value::UInt(0));
    e
}

/// A `CMapData` with a bench, a bin and an interior, and every member its
/// schema declares, most of which the typed root carries rather than names.
fn hills(start: fn(&str) -> MetaInstance) -> MetaInstance {
    let mut interior = entity(start, "CMloInstanceDef", "v_int_hills", 3, [5.0, 6.0, 7.0]);
    set(&mut interior, "groupId", Value::UInt(0));
    set(&mut interior, "floorId", Value::UInt(0));
    set(
        &mut interior,
        "defaultEntitySets",
        Value::Array(vec![Value::Hash(HashedString::new("set_a"))]),
    );
    set(&mut interior, "numExitPortals", Value::UInt(2));
    set(&mut interior, "MLOInstflags", Value::UInt(0));

    let mut root = start("CMapData");
    set(&mut root, "name", Value::Hash(HashedString::new("hills")));
    set(
        &mut root,
        "parent",
        Value::Hash(HashedString::new("hills_lod")),
    );
    set(&mut root, "flags", Value::UInt(0));
    set(&mut root, "contentFlags", Value::UInt(1));
    set(
        &mut root,
        "streamingExtentsMin",
        Value::Vec3([-10.0, -10.0, -10.0]),
    );
    set(
        &mut root,
        "streamingExtentsMax",
        Value::Vec3([10.0, 10.0, 10.0]),
    );
    set(
        &mut root,
        "entitiesExtentsMin",
        Value::Vec3([-5.0, -5.0, -5.0]),
    );
    set(
        &mut root,
        "entitiesExtentsMax",
        Value::Vec3([5.0, 5.0, 5.0]),
    );
    let bench = entity(start, "CEntityDef", "prop_bench_01a", 1, [1.0, 2.0, 3.0]);
    let bin = entity(start, "CEntityDef", "prop_bin_01a", 2, [4.0, 5.0, 6.0]);
    set(
        &mut root,
        "entities",
        Value::Array(
            [bench, bin, interior]
                .into_iter()
                .map(|e| Value::Pointer(Some(Box::new(e))))
                .collect(),
        ),
    );
    set(&mut root, "physicsDictionaries", Value::Array(Vec::new()));
    root
}

/// The target's metadata lines, on a map built here rather than staged.
fn run_the_target_lines(path: &Path) -> Result<(), or_io::Error> {
    let mut map = CMapData::open(path)?;
    for e in &mut map.entities {
        if e.archetype_name == "prop_bench_01a" {
            e.position.z += 1.0;
        }
    }
    map.save()?;
    Ok(())
}

/// The map `hills` builds, RBM-encoded against the installed schema, which it needs.
fn built_hills(what: &str) -> Option<Vec<u8>> {
    let library = support::schema_or_skip(what)?;
    let root = &hills(sampled);
    let layout = rbm_layout_for_instance(library, root, None).expect("the built map lays out");
    let layout = MetaLayout {
        format: BinaryFormat::Rbm,
        container: ContainerLayout::Rbm(Box::new(layout)),
    };
    Some(write_meta_binary(BinaryFormat::Rbm, Some(&layout), root).expect("the built map encodes"))
}

/// A map with no entities, which RBF can hold without a schema.
fn bare_map() -> MetaInstance {
    let mut root = hills(|t| MetaInstance::new(t));
    root.set("entities", Value::Array(Vec::new()));
    root
}

#[test]
fn a_bench_raised_by_the_target_lines_is_the_only_change_saved() {
    const WHAT: &str = "a_bench_raised_by_the_target_lines_is_the_only_change_saved";
    let Some(bytes) = built_hills(WHAT) else {
        return;
    };
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("typed_world_roots");
    std::fs::create_dir_all(&dir).expect("scratch directory");
    let path = dir.join("hills.ymap");
    std::fs::write(&path, &bytes).expect("scratch file");

    let before = MetaDocument::parse(support::schema(), bytes.clone()).expect("reads");
    let opened = CMapData::open(&path).expect("opens typed");
    assert_eq!(opened.entities.len(), 3);
    assert!(opened.entities[2].as_mlo_instance().is_some());
    assert_eq!(
        opened.to_bytes().expect("an untouched save"),
        bytes,
        "an untouched save changed the file"
    );

    run_the_target_lines(&path).expect("the target lines run");

    let mut expected = before.clone();
    assert!(raise_in_tree(&mut expected.instance, 0));
    let saved = std::fs::read(&path).expect("saved");
    assert_ne!(saved, bytes, "the edit did not reach the file");
    assert_eq!(
        saved,
        expected.to_bytes().expect("the tree encodes"),
        "the save holds a change besides the bench's height"
    );

    let reopened = CMapData::open(&path).expect("reopens");
    let mut want = opened.clone();
    want.entities[0].position.z += 1.0;
    assert_eq!(reopened, want);
    assert_ne!(reopened, opened);
    assert_eq!(reopened.entities[0].position.z, 4.0);
    assert_eq!(reopened.entities[1].position.z, 6.0);
}

#[test]
fn a_document_of_another_root_is_refused_at_open() {
    const WHAT: &str = "a_document_of_another_root_is_refused_at_open";
    let Some(bytes) = built_hills(WHAT) else {
        return;
    };
    CMapData::from_bytes(&bytes).expect("the map opens as what it is");
    let err = CMapTypes::from_bytes(&bytes).expect_err("a map is not a type file");
    assert!(err.to_string().contains("CMapTypes"), "{err}");
}

#[test]
fn a_member_its_type_table_lacks_opens_as_its_default_and_refuses_a_change() {
    let mut root = bare_map();
    root.fields.retain(|(n, _)| n != "entitiesExtentsMax");
    let bytes = write_meta_binary(BinaryFormat::Rbf, None, &root).expect("encodes");
    let mut map = CMapData::from_bytes(&bytes).expect("an older type table opens");
    assert_eq!(map.entities_extents_max, Vec3::default());
    assert_eq!(map.to_bytes().expect("an untouched save"), bytes);

    map.entities_extents_max.z = 5.0;
    let err = map
        .to_bytes()
        .expect_err("the file has nowhere to hold the change");
    assert!(err.to_string().contains("entitiesExtentsMax"), "{err}");
}

fn opened_hills(what: &str) -> Option<CMapData> {
    let bytes = built_hills(what)?;
    Some(CMapData::from_bytes(&bytes).expect("opens typed"))
}

/// `map` with every name held as its hash alone, which is all an RBM file stores of it.
fn as_stored(map: &CMapData) -> CMapData {
    let unnamed = |h: &mut HashedString| h.raw.clear();
    let mut map = map.clone();
    unnamed(&mut map.name);
    unnamed(&mut map.parent);
    map.physics_dictionaries.iter_mut().for_each(unnamed);
    for e in &mut map.entities {
        unnamed(&mut e.archetype_name);
        if let CEntityDefAny::CMloInstanceDef(mlo) = e {
            mlo.default_entity_sets.iter_mut().for_each(unnamed);
        }
    }
    map
}

fn round_trip(map: &CMapData) -> CMapData {
    let saved = map.to_bytes().expect("saves");
    let back = CMapData::from_bytes(&saved).expect("reopens");
    assert_eq!(back, as_stored(map));
    assert_eq!(back.to_bytes().expect("saves again"), saved);
    back
}

#[test]
fn an_entity_moved_to_another_variant_reopens_as_that_variant() {
    const WHAT: &str = "an_entity_moved_to_another_variant_reopens_as_that_variant";
    let Some(mut map) = opened_hills(WHAT) else {
        return;
    };
    let interior = map.entities[2]
        .as_mlo_instance()
        .expect("an interior")
        .clone();
    map.entities[2] = CEntityDefAny::Base(interior.base.clone());
    let CEntityDefAny::Base(bench) = map.entities[0].clone() else {
        panic!("the bench is a plain entity");
    };
    map.entities[0] = CEntityDefAny::CMloInstanceDef(CMloInstanceDef {
        base: bench,
        group_id: 5,
        default_entity_sets: vec![HashedString::new("set_b")],
        ..Default::default()
    });

    let back = round_trip(&map);
    assert!(back.entities[2].as_mlo_instance().is_none());
    let moved = back.entities[0]
        .as_mlo_instance()
        .expect("saved as an interior");
    assert_eq!(moved.group_id, 5);
    assert!(moved.default_entity_sets[0] == "set_b");
    assert!(back.entities[0].archetype_name == "prop_bench_01a");
}

#[test]
fn a_fresh_entity_pushed_onto_an_opened_map_saves_and_reopens() {
    const WHAT: &str = "a_fresh_entity_pushed_onto_an_opened_map_saves_and_reopens";
    let Some(mut map) = opened_hills(WHAT) else {
        return;
    };
    let mut e = CEntityDef::default();
    e.archetype_name = HashedString::new("prop_bin_01a");
    e.guid = 9;
    e.position = Vec3 {
        x: -4.0,
        y: 5.0,
        z: 6.0,
    };
    e.parent_index = -1;
    map.entities.push(CEntityDefAny::Base(e));

    let back = round_trip(&map);
    assert_eq!(back.entities.len(), 4);
    assert_eq!(
        back.entities[3].position,
        Vec3 {
            x: -4.0,
            y: 5.0,
            z: 6.0
        }
    );
}

#[test]
fn a_map_built_from_nothing_saves_and_reopens() {
    const WHAT: &str = "a_map_built_from_nothing_saves_and_reopens";
    if support::schema_or_skip(WHAT).is_none() {
        return;
    }
    let mut map = CMapData::default();
    map.name = HashedString::new("hills");
    let mut e = CEntityDef::default();
    e.archetype_name = HashedString::new("prop_bench_01a");
    e.position = Vec3 {
        x: 1.0,
        y: 2.0,
        z: 3.0,
    };
    e.parent_index = -1;
    map.entities.push(CEntityDefAny::Base(e));

    let back = round_trip(&map);
    assert!(back.entities[0].archetype_name == "prop_bench_01a");
}

/// The name hash of each member `inst` states.
fn stated(inst: &MetaInstance) -> Vec<u32> {
    let mut stated: Vec<u32> = inst.fields.iter().map(|(n, _)| name_hash(n)).collect();
    stated.sort_unstable();
    stated
}

fn hashed(members: &[&str]) -> Vec<u32> {
    let mut hashed: Vec<u32> = members.iter().map(|m| name_hash(m)).collect();
    hashed.sort_unstable();
    hashed
}

/// The name hash of each member `or_io::meta::typed::world_types` declares for `class`, sorted.
fn declared_members(class: &str) -> Vec<u32> {
    let declared = or_io::meta::typed::world_types::declared();
    let class = declared.resolve(class).expect("the class is declared");
    let mut members: Vec<u32> = class.fields.iter().map(|f| name_hash(&f.name)).collect();
    members.sort_unstable();
    members
}

/// ⚠ A snapshot declares more of each class than the typed records name, so a root that took its classes from the snapshot
/// would state those members too, and a machine with none could not write the same file.
#[test]
fn a_root_built_from_nothing_states_the_declared_members_whatever_snapshot_is_installed() {
    const WHAT: &str =
        "a_root_built_from_nothing_states_the_declared_members_whatever_snapshot_is_installed";
    let Some(library) = support::schema_or_skip(WHAT) else {
        return;
    };
    let declares_more = |class: &str, typed: &[&str]| {
        library
            .resolve_any(class)
            .is_some_and(|resolved| resolved.fields.len() > typed.len())
    };
    assert!(
        declares_more("CEntityDef", CEntityDef::MEMBERS)
            && declares_more("CMapData", CMapData::MEMBERS),
        "the installed snapshot declares members no record types, so this can fail"
    );

    let mut map = CMapData::default();
    map.name = HashedString::new("hills");
    let mut e = CEntityDef::default();
    e.archetype_name = HashedString::new("prop_bench_01a");
    e.parent_index = -1;
    map.entities.push(CEntityDefAny::Base(e));
    let saved = map.to_bytes().expect("saves");
    assert_eq!(
        or_io::meta::detect_binary_format(&saved),
        Some(BinaryFormat::Pso)
    );
    let root = saved_tree(saved);
    // ⚠ The members that hold a map's parked vehicles, occluders, modifiers and far lights are typed and not declared.
    let of_a_map = declared_members("CMapData");
    assert!(of_a_map.len() < CMapData::MEMBERS.len());
    assert_eq!(stated(&root), of_a_map);
    let Some(Value::Array(entities)) = root
        .fields
        .iter()
        .find_map(|(n, v)| (name_hash(n) == name_hash("entities")).then_some(v))
    else {
        panic!("the map holds its entities");
    };
    let Some(Value::Struct(entity) | Value::Pointer(Some(entity))) = entities.first() else {
        panic!("the map holds its entity");
    };
    assert_eq!(stated(entity), hashed(CEntityDef::MEMBERS));

    let mut types = CMapTypes::default();
    types.archetypes = vec![CBaseArchetypeDefAny::Base(CBaseArchetypeDef::default())];
    let root = saved_tree(types.to_bytes().expect("saves"));
    assert_eq!(stated(&root), hashed(CMapTypes::MEMBERS));
}

/// ⚠ A snapshot lays an interior instance out over the entity class a root built from nothing takes from the declaration, which
/// has fewer members than the entity a shipped interior is, so a copy of one would be saved without them.
#[test]
fn a_copy_of_a_class_derived_from_a_declared_one_is_refused_rather_than_narrowed() {
    const WHAT: &str =
        "a_copy_of_a_class_derived_from_a_declared_one_is_refused_rather_than_narrowed";
    let Some(donor) = opened_hills(WHAT) else {
        return;
    };
    let interior = donor.entities[2].clone();
    assert!(interior.as_mlo_instance().is_some());
    let mut map = CMapData::default();
    map.name = HashedString::new("hills");
    map.entities.push(interior);
    let refused = map
        .to_bytes()
        .expect_err("the copy holds members of the entity class to drop");
    let why = refused.to_string();
    assert!(
        why.contains("would drop") && why.contains("CMloInstanceDef"),
        "{why}"
    );
}

/// Each member `or_io::meta::typed::world_types` declares, held to the class the installed snapshot resolves: the same type, the
/// same init, and the same order among the members declared.
#[test]
fn every_declared_world_member_is_typed_as_the_engine_declares_it() {
    const WHAT: &str = "every_declared_world_member_is_typed_as_the_engine_declares_it";
    let Some(library) = support::schema_or_skip(WHAT) else {
        return;
    };
    let declared = or_io::meta::typed::world_types::declared();
    let mut checked = 0;
    for (class, def) in &declared.structs {
        let engine = library
            .resolve_any(class)
            .unwrap_or_else(|| panic!("the snapshot declares `{class}`"));
        let mut last = None;
        for member in &def.fields {
            let at = engine
                .fields
                .iter()
                .position(|f| name_hash(&f.name) == name_hash(&member.name))
                .unwrap_or_else(|| panic!("the engine's `{class}` has a `{}`", member.name));
            let stated = &engine.fields[at];
            assert_eq!(member.ty, stated.ty, "`{class}::{}`", member.name);
            assert_eq!(member.init, stated.init, "`{class}::{}`", member.name);
            assert!(
                last < Some(at),
                "`{class}::{}` is declared in the engine's order",
                member.name
            );
            last = Some(at);
            checked += 1;
        }
    }
    // ⚠ A map types six members no class is declared for: the ones that hold the records of `typed::map_records`.
    let typed = [
        ("CEntityDef", CEntityDef::MEMBERS),
        ("CMapData", CMapData::MEMBERS),
        ("CBaseArchetypeDef", CBaseArchetypeDef::MEMBERS),
        ("CMapTypes", CMapTypes::MEMBERS),
    ];
    let both = |(class, typed): &(&str, &[&str])| {
        let declared = declared_members(class);
        let is_declared = |member: &&&str| declared.contains(&name_hash(member));
        typed.iter().filter(is_declared).count()
    };
    assert_eq!(checked, typed.iter().map(both).sum::<usize>());
    assert_eq!(
        typed.iter().map(|(_, typed)| typed.len()).sum::<usize>(),
        checked + 6
    );
}

#[test]
fn a_map_whose_parent_was_removed_is_refused_at_save() {
    const WHAT: &str = "a_map_whose_parent_was_removed_is_refused_at_save";
    let Some(mut map) = opened_hills(WHAT) else {
        return;
    };
    map.entities[1].parent_index = 2;
    map.to_bytes().expect("a parent in range saves");

    map.entities.remove(2);
    let err = map.to_bytes().expect_err("the parent is gone");
    assert!(matches!(err, or_io::Error::Refused { .. }), "{err}");
    assert!(err.to_string().contains("entity 1"), "{err}");

    map.entities[1].parent_index = 1;
    let err = map.to_bytes().expect_err("an entity is not its own parent");
    assert!(err.to_string().contains("its own LOD parent"), "{err}");

    map.entities[1].flags |= 1 << 3;
    map.to_bytes()
        .expect("a parent in the parent map is not this file's to check");
}

/// A `CMapTypes` with a timed archetype and an interior whose one room attaches its one entity.
fn built_types(what: &str) -> Option<Vec<u8>> {
    let library = support::schema_or_skip(what)?;
    let mut timed = sampled("CTimeArchetypeDef");
    set(
        &mut timed,
        "name",
        Value::Hash(HashedString::new("prop_timed")),
    );
    set(&mut timed, "timeFlags", Value::UInt(7));
    let mut room = sampled("CMloRoomDef");
    set(
        &mut room,
        "attachedObjects",
        Value::Array(vec![Value::UInt(0)]),
    );
    let mut interior = sampled("CMloArchetypeDef");
    set(
        &mut interior,
        "name",
        Value::Hash(HashedString::new("v_int_hills")),
    );
    set(
        &mut interior,
        "entities",
        Value::Array(vec![Value::Pointer(Some(Box::new(entity(
            sampled,
            "CEntityDef",
            "prop_bench_01a",
            1,
            [1.0, 2.0, 3.0],
        ))))]),
    );
    set(
        &mut interior,
        "rooms",
        Value::Array(vec![Value::Struct(Box::new(room))]),
    );
    let mut root = sampled("CMapTypes");
    set(
        &mut root,
        "archetypes",
        Value::Array(
            [timed, interior]
                .into_iter()
                .map(|a| Value::Pointer(Some(Box::new(a))))
                .collect(),
        ),
    );
    let layout = rbm_layout_for_instance(library, &root, None).expect("the built types lay out");
    let layout = MetaLayout {
        format: BinaryFormat::Rbm,
        container: ContainerLayout::Rbm(Box::new(layout)),
    };
    Some(
        write_meta_binary(BinaryFormat::Rbm, Some(&layout), &root).expect("the built types encode"),
    )
}

#[test]
fn an_interior_whose_attached_entity_was_removed_is_refused_at_save() {
    const WHAT: &str = "an_interior_whose_attached_entity_was_removed_is_refused_at_save";
    let Some(bytes) = built_types(WHAT) else {
        return;
    };
    let mut types = CMapTypes::from_bytes(&bytes).expect("opens typed");
    assert_eq!(types.to_bytes().expect("an untouched save"), bytes);
    let CBaseArchetypeDefAny::CMloArchetypeDef(interior) = &mut types.archetypes[1] else {
        panic!("the second archetype is the interior");
    };
    interior.entities.clear();
    let err = types
        .to_bytes()
        .expect_err("the room attaches a removed entity");
    assert!(matches!(err, or_io::Error::Refused { .. }), "{err}");
    assert!(err.to_string().contains("room 0"), "{err}");
}

#[test]
fn a_timed_archetype_moved_to_the_base_reopens_as_the_base() {
    const WHAT: &str = "a_timed_archetype_moved_to_the_base_reopens_as_the_base";
    let Some(bytes) = built_types(WHAT) else {
        return;
    };
    let mut types = CMapTypes::from_bytes(&bytes).expect("opens typed");
    assert!(matches!(
        types.archetypes[0],
        CBaseArchetypeDefAny::CTimeArchetypeDef(_)
    ));
    types.archetypes[0] = CBaseArchetypeDefAny::Base(types.archetypes[0].deref().clone());
    let saved = types.to_bytes().expect("saves");
    let back = CMapTypes::from_bytes(&saved).expect("reopens");
    assert_eq!(back, types);
    assert!(matches!(back.archetypes[0], CBaseArchetypeDefAny::Base(_)));
    assert!(back.archetypes[0].name == "prop_timed");
    let tree = MetaDocument::parse(support::schema(), saved).expect("reads");
    let Some(Value::Array(archetypes)) = tree.instance.get("archetypes") else {
        panic!("no archetypes");
    };
    let Value::Pointer(Some(first)) = &archetypes[0] else {
        panic!("an archetype is held by pointer");
    };
    assert!(first.get("timeFlags").is_none(), "{first:?}");
}

#[test]
fn an_entity_pushed_onto_a_map_reaches_the_file() {
    const WHAT: &str = "an_entity_pushed_onto_a_map_reaches_the_file";
    let Some(bytes) = built_hills(WHAT) else {
        return;
    };
    let mut map = CMapData::from_bytes(&bytes).expect("opens typed");
    let mut copy = map.entities[1].clone();
    copy.guid = 4;
    copy.position.x = -40.0;
    map.entities.push(copy);

    let saved = map.to_bytes().expect("the grown map saves");
    let back = CMapData::from_bytes(&saved).expect("the grown map reopens");
    assert_eq!(back, map);
    let guids: Vec<u32> = back.entities.iter().map(|e| e.guid).collect();
    assert_eq!(guids, [1, 2, 3, 4]);
    assert_eq!(back.entities[3].position.x, -40.0);
    assert!(back.entities[3].archetype_name == "prop_bin_01a");
    assert_eq!(back.to_bytes().expect("saves again"), saved);
}

/// Where `name`'s distinct value comes from: its name hash, kept small enough for an `f32`.
fn distinct_seed(name: &str) -> u32 {
    name_hash(name) % 50_000 + 1
}

fn distinct_hash(name: &str) -> HashedString {
    HashedString::from_hash(distinct_seed(name))
}

fn distinct_f32(name: &str) -> f32 {
    distinct_seed(name) as f32
}

fn distinct_vec3(name: &str) -> Vec3 {
    let a = distinct_f32(name);
    Vec3 {
        x: a,
        y: a + 1.0,
        z: a + 2.0,
    }
}

fn distinct_vec4(name: &str) -> Vec4 {
    let a = distinct_f32(name);
    Vec4 {
        x: a,
        y: a + 1.0,
        z: a + 2.0,
        w: a + 3.0,
    }
}

/// Every scalar and hash slot of `type_name`, as the installed schema declares
/// it, holding a value derived from the slot's own name hash and no default.
fn distinct(type_name: &str) -> MetaInstance {
    distinct_of(sampled(type_name))
}

/// `inst` with every scalar and hash slot holding a value derived from the slot's own name hash and no default.
fn distinct_of(mut inst: MetaInstance) -> MetaInstance {
    for (name, slot) in &mut inst.fields {
        let seed = distinct_seed(name);
        match slot {
            Value::UInt(v) => *v = u64::from(seed),
            Value::Int(v) => *v = i64::from(seed),
            Value::Float(v) => *v = f64::from(distinct_f32(name)),
            Value::Bool(v) => *v = true,
            Value::Bitset { bits, .. } => *bits = u128::from(seed),
            Value::Color(v) => *v = seed,
            Value::Hash(h) => *h = distinct_hash(name),
            Value::Vec3(v) => {
                let d = distinct_vec3(name);
                *v = [d.x, d.y, d.z];
            }
            Value::Vec4(v) => {
                let d = distinct_vec4(name);
                *v = [d.x, d.y, d.z, d.w];
            }
            _ => {}
        }
    }
    inst
}

fn distinct_hashes(name: &str) -> Value {
    Value::Array(vec![Value::Hash(distinct_hash(name))])
}

fn behind_pointers(items: Vec<MetaInstance>) -> Value {
    Value::Array(
        items
            .into_iter()
            .map(|i| Value::Pointer(Some(Box::new(i))))
            .collect(),
    )
}

fn encoded(root: &MetaInstance) -> Vec<u8> {
    let layout = rbm_layout_for_instance(support::schema(), root, None).expect("lays out");
    let layout = MetaLayout {
        format: BinaryFormat::Rbm,
        container: ContainerLayout::Rbm(Box::new(layout)),
    };
    write_meta_binary(BinaryFormat::Rbm, Some(&layout), root).expect("encodes")
}

fn assert_distinct_entity(e: &CEntityDef) {
    assert_eq!(e.archetype_name.hash, distinct_hash("archetypeName").hash);
    assert_eq!(e.flags, distinct_seed("flags"));
    assert_eq!(e.guid, distinct_seed("guid"));
    assert_eq!(e.position, distinct_vec3("position"));
    assert_eq!(e.rotation, distinct_vec4("rotation"));
    assert_eq!(e.scale_xy, distinct_f32("scaleXY"));
    assert_eq!(e.scale_z, distinct_f32("scaleZ"));
    assert_eq!(e.parent_index, distinct_seed("parentIndex") as i32);
    assert_eq!(e.lod_dist, distinct_f32("lodDist"));
    assert_eq!(e.child_lod_dist, distinct_f32("childLodDist"));
    assert_eq!(e.num_children, distinct_seed("numChildren"));
    assert_eq!(
        e.ambient_occlusion_multiplier,
        distinct_seed("ambientOcclusionMultiplier") as i32
    );
    assert_eq!(
        e.artificial_ambient_occlusion,
        distinct_seed("artificialAmbientOcclusion") as i32
    );
    assert_eq!(e.tint_value, distinct_seed("tintValue"));
}

fn assert_distinct_archetype(a: &CBaseArchetypeDef) {
    assert_eq!(a.lod_dist, distinct_f32("lodDist"));
    assert_eq!(a.flags, distinct_seed("flags"));
    assert_eq!(a.special_attribute, distinct_seed("specialAttribute"));
    assert_eq!(a.bb_min, distinct_vec3("bbMin"));
    assert_eq!(a.bb_max, distinct_vec3("bbMax"));
    assert_eq!(a.bs_centre, distinct_vec3("bsCentre"));
    assert_eq!(a.bs_radius, distinct_f32("bsRadius"));
    assert_eq!(a.hd_texture_dist, distinct_f32("hdTextureDist"));
    for (hash, name) in [
        (&a.name, "name"),
        (&a.texture_dictionary, "textureDictionary"),
        (&a.clip_dictionary, "clipDictionary"),
        (&a.drawable_dictionary, "drawableDictionary"),
        (&a.physics_dictionary, "physicsDictionary"),
        (&a.asset_name, "assetName"),
    ] {
        assert_eq!(hash.hash, distinct_hash(name).hash, "{name}");
    }
}

/// ⚠ A typed member whose name the engine does not declare for its type reads
/// as its init in every file, so only a slot the schema names catches the misspelling.
#[test]
fn every_typed_member_reads_the_slot_the_engine_declares_for_it() {
    const WHAT: &str = "every_typed_member_reads_the_slot_the_engine_declares_for_it";
    if support::schema_or_skip(WHAT).is_none() {
        return;
    }
    let mut interior = distinct("CMloInstanceDef");
    set(
        &mut interior,
        "defaultEntitySets",
        distinct_hashes("defaultEntitySets"),
    );
    let mut root = distinct("CMapData");
    set(
        &mut root,
        "entities",
        behind_pointers(vec![distinct("CEntityDef"), interior]),
    );
    set(
        &mut root,
        "physicsDictionaries",
        distinct_hashes("physicsDictionaries"),
    );
    let map = CMapData::from_bytes(&encoded(&root)).expect("opens typed");
    assert_eq!(map.name.hash, distinct_hash("name").hash);
    assert_eq!(map.parent.hash, distinct_hash("parent").hash);
    assert_eq!(map.flags, distinct_seed("flags"));
    assert_eq!(map.content_flags, distinct_seed("contentFlags"));
    assert_eq!(
        map.streaming_extents_min,
        distinct_vec3("streamingExtentsMin")
    );
    assert_eq!(
        map.streaming_extents_max,
        distinct_vec3("streamingExtentsMax")
    );
    assert_eq!(
        map.entities_extents_min,
        distinct_vec3("entitiesExtentsMin")
    );
    assert_eq!(
        map.entities_extents_max,
        distinct_vec3("entitiesExtentsMax")
    );
    assert_eq!(map.physics_dictionaries.len(), 1);
    assert_eq!(
        map.physics_dictionaries[0].hash,
        distinct_hash("physicsDictionaries").hash
    );
    assert_eq!(map.entities.len(), 2);
    assert_distinct_entity(&map.entities[0]);
    let mlo = map.entities[1].as_mlo_instance().expect("an interior");
    assert_distinct_entity(&mlo.base);
    assert_eq!(mlo.group_id, distinct_seed("groupId"));
    assert_eq!(mlo.floor_id, distinct_seed("floorId"));
    assert_eq!(mlo.default_entity_sets.len(), 1);
    assert_eq!(
        mlo.default_entity_sets[0].hash,
        distinct_hash("defaultEntitySets").hash
    );
    assert_eq!(mlo.num_exit_portals, distinct_seed("numExitPortals"));
    assert_eq!(mlo.mlo_instflags, distinct_seed("MLOInstflags"));

    let mut interior = distinct("CMloArchetypeDef");
    set(
        &mut interior,
        "entities",
        behind_pointers(vec![distinct("CEntityDef")]),
    );
    let mut root = distinct("CMapTypes");
    set(
        &mut root,
        "archetypes",
        behind_pointers(vec![
            distinct("CBaseArchetypeDef"),
            distinct("CTimeArchetypeDef"),
            interior,
        ]),
    );
    set(&mut root, "dependencies", distinct_hashes("dependencies"));
    let types = CMapTypes::from_bytes(&encoded(&root)).expect("opens typed");
    assert_eq!(types.name.hash, distinct_hash("name").hash);
    assert_eq!(types.dependencies.len(), 1);
    assert_eq!(
        types.dependencies[0].hash,
        distinct_hash("dependencies").hash
    );
    let [base, timed, interior] = &types.archetypes[..] else {
        panic!("three archetypes: {:?}", types.archetypes);
    };
    assert!(matches!(base, CBaseArchetypeDefAny::Base(_)));
    assert_distinct_archetype(base);
    let CBaseArchetypeDefAny::CTimeArchetypeDef(timed) = timed else {
        panic!("the second archetype is timed");
    };
    assert_distinct_archetype(&timed.base);
    assert_eq!(timed.time_flags, distinct_seed("timeFlags"));
    let CBaseArchetypeDefAny::CMloArchetypeDef(interior) = interior else {
        panic!("the third archetype is an interior");
    };
    assert_distinct_archetype(&interior.base);
    assert_eq!(interior.mlo_flags, distinct_seed("mloFlags"));
    assert_eq!(interior.entities.len(), 1);
    assert_distinct_entity(&interior.entities[0]);
}

/// Collects each numeric member of `inst` and its structs that does not hold
/// its schema `init`, and the name hash of every type visited.
fn inits_missed(inst: &MetaInstance, seen: &mut Vec<u32>, missed: &mut Vec<String>) {
    use or_io::meta::schema::Literal;
    let Some(resolved) = support::schema().resolve_any(&inst.type_name) else {
        return;
    };
    seen.push(name_hash(&inst.type_name));
    for f in &resolved.fields {
        let key = name_hash(&f.name);
        let slot = inst
            .fields
            .iter()
            .find(|(n, _)| name_hash(n) == key)
            .map(|(_, v)| v);
        let holds = match (&f.init, slot) {
            (Some(Literal::Int(want)), Some(Value::Int(v))) => v == want,
            (Some(Literal::Int(want)), Some(Value::UInt(v))) => i64::try_from(*v) == Ok(*want),
            (Some(Literal::Float(want)), Some(Value::Float(v))) => v == want,
            (Some(Literal::Bool(want)), Some(Value::Bool(v))) => v == want,
            (Some(Literal::Int(_) | Literal::Float(_) | Literal::Bool(_)), _) => false,
            _ => true,
        };
        if !holds {
            missed.push(format!(
                "`{}::{}` holds {slot:?}; the engine's init is {:?}",
                inst.type_name, f.name, f.init
            ));
        }
    }
    let mut visit = |v: &Value| {
        if let Value::Struct(i) | Value::Pointer(Some(i)) = v {
            inits_missed(i, seen, missed);
        }
    };
    for (_, v) in &inst.fields {
        match v {
            Value::Array(items) => items.iter().for_each(&mut visit),
            other => visit(other),
        }
    }
}

fn saved_tree(bytes: Vec<u8>) -> MetaInstance {
    MetaDocument::parse(support::schema(), bytes)
        .expect("the save reads")
        .instance
}

#[test]
fn a_record_built_here_saves_the_engines_inits() {
    const WHAT: &str = "a_record_built_here_saves_the_engines_inits";
    if support::schema_or_skip(WHAT).is_none() {
        return;
    }
    let mut unparented = CEntityDef::default();
    unparented.parent_index = -1;
    let mut map = CMapData::default();
    map.entities = vec![
        CEntityDefAny::Base(unparented.clone()),
        CEntityDefAny::CMloInstanceDef(CMloInstanceDef {
            base: unparented.clone(),
            ..Default::default()
        }),
    ];
    let mut types = CMapTypes::default();
    types.archetypes = vec![
        CBaseArchetypeDefAny::Base(CBaseArchetypeDef::default()),
        CBaseArchetypeDefAny::CTimeArchetypeDef(CTimeArchetypeDef::default()),
        CBaseArchetypeDefAny::CMloArchetypeDef(CMloArchetypeDef {
            entities: vec![CEntityDefAny::Base(unparented)],
            ..Default::default()
        }),
    ];

    let (mut seen, mut missed) = (Vec::new(), Vec::new());
    inits_missed(
        &saved_tree(map.to_bytes().expect("the built map saves")),
        &mut seen,
        &mut missed,
    );
    inits_missed(
        &saved_tree(types.to_bytes().expect("the built types save")),
        &mut seen,
        &mut missed,
    );
    for typed in [
        "CEntityDef",
        "CMloInstanceDef",
        "CBaseArchetypeDef",
        "CTimeArchetypeDef",
        "CMloArchetypeDef",
    ] {
        assert!(seen.contains(&name_hash(typed)), "no `{typed}` was checked");
    }
    assert!(missed.is_empty(), "{}", missed.join("\n"));
}

fn by_value(items: Vec<MetaInstance>) -> Value {
    Value::Array(
        items
            .into_iter()
            .map(|i| Value::Struct(Box::new(i)))
            .collect(),
    )
}

fn distinct_indexes(name: &str) -> Value {
    Value::Array(vec![Value::UInt(u64::from(distinct_seed(name)))])
}

/// ⚠ The same trap as `every_typed_member_reads_the_slot_the_engine_declares_for_it`, for the four records of an
/// interior: a member misspelt reads as its init in every file, and one read as the wrong type opens nothing.
#[test]
fn every_typed_member_of_an_interiors_records_reads_the_slot_the_engine_declares_for_it() {
    const WHAT: &str =
        "every_typed_member_of_an_interiors_records_reads_the_slot_the_engine_declares_for_it";
    let Some(library) = support::schema_or_skip(WHAT) else {
        return;
    };
    for (class, typed) in [
        ("CMloRoomDef", CMloRoomDef::MEMBERS),
        ("CMloPortalDef", CMloPortalDef::MEMBERS),
        ("CMloEntitySet", CMloEntitySet::MEMBERS),
        ("CMloTimeCycleModifier", CMloTimeCycleModifier::MEMBERS),
    ] {
        let engine = library
            .resolve_any(class)
            .unwrap_or_else(|| panic!("the snapshot declares `{class}`"));
        let declared: Vec<u32> = engine.fields.iter().map(|f| name_hash(&f.name)).collect();
        assert_eq!(
            hashed(typed),
            {
                let mut declared = declared.clone();
                declared.sort_unstable();
                declared
            },
            "`{class}` types the members the engine declares for it, no member more and none fewer"
        );
    }

    let mut room = distinct("CMloRoomDef");
    set(&mut room, "name", Value::String("a room's name".into()));
    set(&mut room, "attachedObjects", Value::Array(Vec::new()));
    let mut portal = distinct("CMloPortalDef");
    // The interior holds one room and no entity, so the portal names room 0 and attaches nothing.
    set(&mut portal, "roomFrom", Value::UInt(0));
    set(&mut portal, "roomTo", Value::UInt(0));
    let corner = distinct_vec3("corners");
    set(
        &mut portal,
        "corners",
        Value::Array(vec![Value::Vec3([corner.x, corner.y, corner.z])]),
    );
    set(&mut portal, "attachedObjects", Value::Array(Vec::new()));
    let mut entity_set = distinct("CMloEntitySet");
    set(&mut entity_set, "locations", distinct_indexes("locations"));
    set(
        &mut entity_set,
        "entities",
        behind_pointers(vec![distinct("CEntityDef")]),
    );
    let modifier = distinct("CMloTimeCycleModifier");
    let mut interior = distinct("CMloArchetypeDef");
    set(&mut interior, "entities", Value::Array(Vec::new()));
    set(&mut interior, "rooms", by_value(vec![room]));
    set(&mut interior, "portals", by_value(vec![portal]));
    set(&mut interior, "entitySets", by_value(vec![entity_set]));
    set(
        &mut interior,
        "timeCycleModifiers",
        by_value(vec![modifier]),
    );
    let mut root = sampled("CMapTypes");
    set(&mut root, "archetypes", behind_pointers(vec![interior]));
    let bytes = encoded(&root);

    let types = CMapTypes::from_bytes(&bytes).expect("opens typed");
    let CBaseArchetypeDefAny::CMloArchetypeDef(mlo) = &types.archetypes[0] else {
        panic!("the archetype is the interior");
    };
    let room = &mlo.rooms[0];
    assert_eq!(room.name, "a room's name");
    assert_eq!(room.bb_min, distinct_vec3("bbMin"));
    assert_eq!(room.bb_max, distinct_vec3("bbMax"));
    assert_eq!(room.blend, distinct_f32("blend"));
    assert_eq!(
        room.timecycle_name.hash,
        distinct_hash("timecycleName").hash
    );
    assert_eq!(
        room.secondary_timecycle_name.hash,
        distinct_hash("secondaryTimecycleName").hash
    );
    assert_eq!(room.flags, distinct_seed("flags"));
    assert_eq!(room.portal_count, distinct_seed("portalCount"));
    assert_eq!(room.floor_id, distinct_seed("floorId") as i32);
    assert_eq!(
        room.exterior_visibilty_depth,
        distinct_seed("exteriorVisibiltyDepth") as i32
    );

    let portal = &mlo.portals[0];
    assert_eq!((portal.room_from, portal.room_to), (0, 0));
    assert_eq!(portal.flags, distinct_seed("flags"));
    assert_eq!(portal.mirror_priority, distinct_seed("mirrorPriority"));
    assert_eq!(portal.opacity, distinct_seed("opacity"));
    assert_eq!(portal.audio_occlusion, distinct_seed("audioOcclusion"));
    assert_eq!(portal.corners, [distinct_vec3("corners")]);

    let entity_set = &mlo.entity_sets[0];
    assert_eq!(entity_set.name.hash, distinct_hash("name").hash);
    assert_eq!(entity_set.locations, [distinct_seed("locations")]);
    assert_distinct_entity(&entity_set.entities[0]);

    let modifier = &mlo.time_cycle_modifiers[0];
    assert_eq!(modifier.name.hash, distinct_hash("name").hash);
    assert_eq!(modifier.sphere, distinct_vec4("sphere"));
    assert_eq!(modifier.percentage, distinct_f32("percentage"));
    assert_eq!(modifier.range, distinct_f32("range"));
    assert_eq!(modifier.start_hour, distinct_seed("startHour"));
    assert_eq!(modifier.end_hour, distinct_seed("endHour"));

    assert_eq!(types.to_bytes().expect("an untouched save"), bytes);

    // A member a table lacks reads as the engine's init, which is what a record built here states.
    let engine = library
        .resolve_any("CMloRoomDef")
        .expect("the snapshot declares the room");
    let depth = engine
        .fields
        .iter()
        .find(|f| name_hash(&f.name) == name_hash("exteriorVisibiltyDepth"))
        .expect("the room has the member");
    assert_eq!(
        depth.init,
        Some(or_io::meta::Literal::Int(i64::from(
            CMloRoomDef::default().exterior_visibilty_depth
        )))
    );
}

/// The class the installed schema declares for what a map holds under `member`, an array's elements or a struct, sampled.
fn sampled_under(member: &str) -> MetaInstance {
    let shape = rbm_layout_for(support::schema(), "CMapData").expect("a map lays out");
    let root = shape
        .model
        .class_for(literal_string_hash(b"CMapData"))
        .expect("a map is in its own table");
    let entry = (root.entries.iter())
        .find(|e| e.name_hash == name_hash(member))
        .unwrap_or_else(|| panic!("a map has `{member}`"));
    let held = match entry.data_type {
        PsoType::Struct => *entry,
        _ => *root
            .entry(entry.contents_index())
            .expect("an array states its element"),
    };
    let class = shape
        .model
        .class_for(held.referent_hash())
        .unwrap_or_else(|| panic!("the class `{member}` holds is in the table"));
    sample_instance(&shape.model, class)
}

fn distinct_point(name: &str) -> MetaInstance {
    let point = distinct_vec3(name);
    let mut inst = MetaInstance::new("FloatXYZ");
    inst.set("x", Value::Float(f64::from(point.x)));
    inst.set("y", Value::Float(f64::from(point.y)));
    inst.set("z", Value::Float(f64::from(point.z)));
    inst
}

fn distinct_floats(name: &str) -> Value {
    Value::Array(vec![Value::Float(f64::from(distinct_f32(name)))])
}

/// ⚠ A member narrower than the seed takes the seed's low bits, which its own name still tells apart.
fn narrow_seed(name: &str, below: u32) -> u32 {
    distinct_seed(name) % below
}

fn distinct_bytes(name: &str) -> Value {
    Value::Array(vec![Value::UInt(u64::from(narrow_seed(name, 256)))])
}

/// ⚠ The same trap as `every_typed_member_reads_the_slot_the_engine_declares_for_it`, for the records a map holds besides
/// entities: a member misspelt reads as its init in every file, and one read as the wrong type opens nothing.
#[test]
fn every_typed_member_of_a_maps_records_reads_the_slot_the_engine_declares_for_it() {
    const WHAT: &str =
        "every_typed_member_of_a_maps_records_reads_the_slot_the_engine_declares_for_it";
    let Some(library) = support::schema_or_skip(WHAT) else {
        return;
    };
    // ⚠ A file names an occluder by its bare name, where the snapshot keys it under its namespace.
    for (class, typed) in [
        ("CCarGen", CCarGen::MEMBERS),
        ("rage::BoxOccluder", BoxOccluder::MEMBERS),
        ("rage::OccludeModel", OccludeModel::MEMBERS),
        ("CTimeCycleModifier", CTimeCycleModifier::MEMBERS),
        ("FloatXYZ", FloatXYZ::MEMBERS),
        ("CLODLight", CLODLight::MEMBERS),
        ("CDistantLODLight", CDistantLODLight::MEMBERS),
    ] {
        let engine = library
            .resolve_any(class)
            .unwrap_or_else(|| panic!("the snapshot declares `{class}`"));
        let mut declared: Vec<u32> = engine.fields.iter().map(|f| name_hash(&f.name)).collect();
        declared.sort_unstable();
        assert_eq!(
            hashed(typed),
            declared,
            "`{class}` types the members the engine declares for it, no member more and none fewer"
        );
    }

    const BOX: [&str; 8] = [
        "iCenterX", "iCenterY", "iCenterZ", "iCosZ", "iLength", "iWidth", "iHeight", "iSinZ",
    ];
    let mut car = distinct_of(sampled_under("carGenerators"));
    set(
        &mut car,
        "livery",
        Value::Int(i64::from(narrow_seed("livery", 100))),
    );
    let mut occluder = distinct_of(sampled_under("boxOccluders"));
    for member in BOX {
        let low = i64::from(narrow_seed(member, 30_000));
        set(&mut occluder, member, Value::Int(low));
    }
    let verts: Vec<u8> = (0..24).map(|b| b ^ 0x5A).collect();
    let mut model = distinct_of(sampled_under("occludeModels"));
    set(&mut model, "dataSize", Value::UInt(verts.len() as u64));
    set(&mut model, "verts", Value::ByteArray(verts.clone()));
    let modifier = distinct_of(sampled_under("timeCycleModifiers"));
    let mut lights = distinct_of(sampled_under("LODLightsSOA"));
    set(
        &mut lights,
        "direction",
        by_value(vec![distinct_point("direction")]),
    );
    for member in ["falloff", "falloffExponent"] {
        set(&mut lights, member, distinct_floats(member));
    }
    for member in ["timeAndStateFlags", "hash"] {
        set(&mut lights, member, distinct_indexes(member));
    }
    for member in [
        "coneInnerAngle",
        "coneOuterAngleOrCapExt",
        "coronaIntensity",
    ] {
        set(&mut lights, member, distinct_bytes(member));
    }
    let mut distant = distinct_of(sampled_under("DistantLODLightsSOA"));
    set(
        &mut distant,
        "position",
        by_value(vec![distinct_point("position")]),
    );
    set(&mut distant, "RGBI", distinct_indexes("RGBI"));

    let mut root = sampled("CMapData");
    set(&mut root, "carGenerators", by_value(vec![car]));
    set(&mut root, "boxOccluders", by_value(vec![occluder]));
    set(&mut root, "occludeModels", by_value(vec![model]));
    set(&mut root, "timeCycleModifiers", by_value(vec![modifier]));
    set(&mut root, "LODLightsSOA", Value::Struct(Box::new(lights)));
    set(
        &mut root,
        "DistantLODLightsSOA",
        Value::Struct(Box::new(distant)),
    );
    let bytes = encoded(&root);

    let map = CMapData::from_bytes(&bytes).expect("opens typed");
    let car = &map.car_generators[0];
    assert_eq!(car.position, distinct_vec3("position"));
    assert_eq!(car.orient_x, distinct_f32("orientX"));
    assert_eq!(car.orient_y, distinct_f32("orientY"));
    assert_eq!(
        car.perpendicular_length,
        distinct_f32("perpendicularLength")
    );
    assert_eq!(car.car_model.hash, distinct_hash("carModel").hash);
    assert_eq!(car.flags, distinct_seed("flags"));
    let remaps = [
        (car.body_color_remap1, "bodyColorRemap1"),
        (car.body_color_remap2, "bodyColorRemap2"),
        (car.body_color_remap3, "bodyColorRemap3"),
        (car.body_color_remap4, "bodyColorRemap4"),
    ];
    for (remap, member) in remaps {
        assert_eq!(remap, distinct_seed(member) as i32, "{member}");
    }
    assert_eq!(car.pop_group.hash, distinct_hash("popGroup").hash);
    assert_eq!(i32::from(car.livery), narrow_seed("livery", 100) as i32);

    let b = &map.box_occluders[0];
    let read = [
        b.i_center_x,
        b.i_center_y,
        b.i_center_z,
        b.i_cos_z,
        b.i_length,
        b.i_width,
        b.i_height,
        b.i_sin_z,
    ];
    for (read, member) in read.into_iter().zip(BOX) {
        assert_eq!(
            i32::from(read),
            narrow_seed(member, 30_000) as i32,
            "{member}"
        );
    }

    let model = &map.occlude_models[0];
    assert_eq!(model.bmin, distinct_vec3("bmin"));
    assert_eq!(model.bmax, distinct_vec3("bmax"));
    assert_eq!(model.data_size as usize, verts.len());
    assert_eq!(model.verts, verts);
    assert_eq!(
        u32::from(model.num_verts_in_bytes),
        distinct_seed("numVertsInBytes")
    );
    assert_eq!(u32::from(model.num_tris), distinct_seed("numTris"));
    assert_eq!(model.flags, distinct_seed("flags"));

    let modifier = &map.time_cycle_modifiers[0];
    assert_eq!(modifier.name.hash, distinct_hash("name").hash);
    assert_eq!(modifier.min_extents, distinct_vec3("minExtents"));
    assert_eq!(modifier.max_extents, distinct_vec3("maxExtents"));
    assert_eq!(modifier.percentage, distinct_f32("percentage"));
    assert_eq!(modifier.range, distinct_f32("range"));
    assert_eq!(modifier.start_hour, distinct_seed("startHour"));
    assert_eq!(modifier.end_hour, distinct_seed("endHour"));

    let point = |p: &FloatXYZ| Vec3 {
        x: p.x,
        y: p.y,
        z: p.z,
    };
    let lights = &map.lod_lights_soa;
    assert_eq!(point(&lights.direction[0]), distinct_vec3("direction"));
    assert_eq!(lights.falloff, [distinct_f32("falloff")]);
    assert_eq!(lights.falloff_exponent, [distinct_f32("falloffExponent")]);
    assert_eq!(
        lights.time_and_state_flags,
        [distinct_seed("timeAndStateFlags")]
    );
    assert_eq!(lights.hash, [distinct_seed("hash")]);
    let byte = |member: &str| narrow_seed(member, 256) as u8;
    assert_eq!(lights.cone_inner_angle, [byte("coneInnerAngle")]);
    assert_eq!(
        lights.cone_outer_angle_or_cap_ext,
        [byte("coneOuterAngleOrCapExt")]
    );
    assert_eq!(lights.corona_intensity, [byte("coronaIntensity")]);

    let distant = &map.distant_lod_lights_soa;
    assert_eq!(point(&distant.position[0]), distinct_vec3("position"));
    assert_eq!(distant.rgbi, [distinct_seed("RGBI")]);
    assert_eq!(
        u32::from(distant.num_street_lights),
        distinct_seed("numStreetLights")
    );
    assert_eq!(u32::from(distant.category), distinct_seed("category"));

    assert_eq!(map.to_bytes().expect("an untouched save"), bytes);
}
