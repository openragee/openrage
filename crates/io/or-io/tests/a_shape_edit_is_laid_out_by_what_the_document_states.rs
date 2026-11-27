//! A world document edited in shape is laid out by the classes its own type table states, in a process that installed
//! a schema snapshot whose classes state other members, and the saves are the bytes a process with no snapshot writes.
//!
//! ⚠ **Every test of this binary runs with the snapshot below installed.** An installed snapshot is state of the whole
//! process, set once and never removed, and a gate runs the tests of one binary as threads of one process: a test that
//! wants none installed belongs in another binary. The process with no snapshot is this binary run again as a child.

use std::process::Command;
use std::sync::OnceLock;

use or_io::decl::File;
use or_io::meta::binary::hash_index::default_hash_index;
use or_io::meta::binary::pso::schema::{PsoEnumInfo, PsoSchemaItem};
use or_io::meta::binary::synth::name_hash;
use or_io::meta::binary::type_tables;
use or_io::meta::binary::ContainerLayout;
use or_io::meta::instance::value::HashedString;
use or_io::meta::typed::world_types::WORLD_TYPES;
use or_io::meta::{
    build_library, canonical, detect_binary_format, write_meta_binary, ArrayKind, BinaryFormat,
    CBaseArchetypeDefAny, CCarGen, CEntityDef, CEntityDefAny, CMapData, CMapTypes,
    CMloArchetypeDef, CMloRoomDef, CScenarioChain, CScenarioPoint, CScenarioPointRegion, FieldType,
    MetaDocument, MetaInstance, SchemaLibrary, StringKind, Value,
};

const CHILD_TEST: &str = "save_for_a_parent_with_no_snapshot";
const CHILD_ENV: &str = "OPENRAGE_SHAPE_EDIT_CHILD";
/// What a line the child prints holds: a save or a sentence the two processes share, or an outcome they part on.
const SAME: &str = "same";
const APART: &str = "apart";
const BEGIN: &str = "\u{1e}saves begin\u{1e}";
const END: &str = "\u{1e}saves end\u{1e}";

const FORMATS: [BinaryFormat; 2] = [BinaryFormat::Pso, BinaryFormat::Rbm];

/// A member the snapshot declares for each class below and no file here states.
const LATER: &str = r#"<u32 name="aMemberOfALaterBuild"/>"#;
/// A member a file here states and the snapshot does not declare.
const OWN: &str = r#"<u32 name="aMemberOnlyTheFileStates"/>"#;

const ASSET_NAME: &str = r#"<string name="assetName" type="atHashString"/>"#;
const TINT: &str = r#"<u32 name="tintValue" init="0"/>"#;
const DICTIONARIES: &str =
    r#"<array name="physicsDictionaries" type="atArray"><string type="atHashString"/></array>"#;
const CAR_GENERATORS: &str =
    r#"<array name="carGenerators" type="atArray"><struct type="CCarGen"/></array>"#;
const LIVERY: &str = r#"<s8 name="livery" init="-1"/>"#;

/// A parked vehicle's class with the members its record types, and `MEMBERS` to state more.
const CAR_GEN: &str = r#"
  <structdef type="CCarGen">
    <Vector3 name="position"/>
    <float name="orientX"/>
    <float name="orientY"/>
    <float name="perpendicularLength"/>
    <string name="carModel" type="atHashString"/>
    <u32 name="flags"/>
    <s32 name="bodyColorRemap1"/>
    <s32 name="bodyColorRemap2"/>
    <s32 name="bodyColorRemap3"/>
    <s32 name="bodyColorRemap4"/>
    <string name="popGroup" type="atHashString"/>
    <s8 name="livery" init="-1"/>MEMBERS
  </structdef>
</ParserSchema>"#;

/// The classes of an interior, with `ROOM` to state more of a room.
const INTERIOR: &str = r#"
  <structdef type="CMloRoomDef">
    <string name="name" type="atString"/>
    <Vector3 name="bbMin"/>
    <Vector3 name="bbMax"/>
    <float name="blend"/>
    <string name="timecycleName" type="atHashString"/>
    <string name="secondaryTimecycleName" type="atHashString"/>
    <u32 name="flags"/>
    <u32 name="portalCount"/>
    <s32 name="floorId"/>
    <s32 name="exteriorVisibiltyDepth" init="-1"/>
    <array name="attachedObjects" type="atArray"><u32/></array>ROOM
  </structdef>
  <structdef type="CMloPortalDef">
    <u32 name="roomFrom"/>
    <u32 name="roomTo"/>
    <u32 name="flags"/>
    <u32 name="mirrorPriority"/>
    <u32 name="opacity"/>
    <u32 name="audioOcclusion"/>
    <array name="corners" type="atArray"><Vector3/></array>
    <array name="attachedObjects" type="atArray"><u32/></array>
  </structdef>
  <structdef type="CMloArchetypeDef" base="CBaseArchetypeDef">
    <u32 name="mloFlags"/>
    <array name="entities" type="atArray"><pointer type="rage::fwEntityDef" policy="owner"/></array>
    <array name="rooms" type="atArray"><struct type="CMloRoomDef"/></array>
    <array name="portals" type="atArray"><struct type="CMloPortalDef"/></array>
  </structdef>
</ParserSchema>"#;

/// `psc` with the classes of an interior, whose room states `room` besides the members its record types.
fn with_interior(psc: &str, room: &str) -> String {
    psc.replace("</ParserSchema>", &INTERIOR.replace("ROOM", room))
}

/// The classes of a scenario region, with `POINT` to state more of a point and `CHAIN` what a chain states.
const REGION: &str = r#"
  <structdef type="CScenarioPoint">
    <u8 name="iType"/>
    <u8 name="ModelSetId"/>
    <u8 name="iInterior"/>
    <u8 name="iRequiredIMapId"/>
    <u8 name="iProbability"/>
    <u8 name="uAvailableInMpSp"/>
    <u8 name="iTimeStartOverride"/>
    <u8 name="iTimeEndOverride"/>
    <u8 name="iRadius"/>
    <u8 name="iTimeTillPedLeaves"/>
    <u16 name="iScenarioGroup"/>
    <u32 name="Flags"/>
    <Vector4 name="vPositionAndDirection"/>POINT
  </structdef>
  <structdef type="CScenarioPointContainer">
    <array name="MyPoints" type="atArray"><struct type="CScenarioPoint"/></array>
  </structdef>
  <structdef type="CScenarioChainingGraph">
    <array name="Chains" type="atArray"><struct type="CScenarioChain"/></array>
  </structdef>CHAIN
  <structdef type="CScenarioPointLookUps">
    <array name="TypeNames" type="atArray"><string type="atHashString"/></array>
  </structdef>
  <structdef type="CScenarioPointRegion">
    <s32 name="VersionNumber"/>
    <struct name="Points" type="CScenarioPointContainer"/>
    <struct name="ChainingGraph" type="CScenarioChainingGraph"/>
    <struct name="LookUps" type="CScenarioPointLookUps"/>
  </structdef>
</ParserSchema>"#;

const CHAIN: &str = r#"
  <structdef type="CScenarioChain">
    <u8 name="MaxUsers" init="1"/>
    <array name="EdgeIds" type="atArray"><u16/></array>
  </structdef>"#;

/// The classes of a region whose point states `point` besides the members its record types; `chain` is what the
/// classes state of a chain, which a region that holds none only names.
fn region_psc(point: &str, chain: &str) -> String {
    let region = REGION.replace("POINT", point).replace("CHAIN", chain);
    r#"<ParserSchema>"#.to_string() + region.trim_start()
}

/// How a file's class differs from the snapshot's: it states fewer members, more, or others.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Differs {
    /// The class as the engine's names give it, without the snapshot's later member.
    Fewer,
    /// Every member the snapshot declares, and one of its own.
    More,
    /// One member of the snapshot's missing, and one of its own.
    Other,
}

const DIFFERENCES: [Differs; 3] = [Differs::Fewer, Differs::More, Differs::Other];

/// The world classes with `member` put after `after` in the class that holds it.
fn with_member(psc: &str, after: &str, member: &str) -> String {
    assert!(psc.contains(after), "the classes state `{after}`");
    psc.replace(after, &format!("{after}{member}"))
}

fn without_member(psc: &str, member: &str) -> String {
    assert!(psc.contains(member), "the classes state `{member}`");
    psc.replace(member, "")
}

/// The world classes with a map that holds parked vehicles, whose class states `members` besides those its record types.
fn with_car_gens(psc: &str, members: &str) -> String {
    with_member(psc, DICTIONARIES, CAR_GENERATORS)
        .replace("</ParserSchema>", &CAR_GEN.replace("MEMBERS", members))
}

/// The classes the snapshot of this process declares: an archetype, an entity and a parked vehicle each with a member
/// of a later build, as a class grows between two versions of the game.
fn snapshot_psc() -> String {
    let psc = with_member(WORLD_TYPES, ASSET_NAME, LATER);
    let psc = with_interior(&psc, LATER);
    let psc = with_member(
        &psc,
        TINT,
        r#"<u32 name="lodLevel"/><u32 name="priorityLevel"/>"#,
    );
    let region = region_psc(LATER, CHAIN).replace("<ParserSchema>", "");
    with_car_gens(&psc, LATER).replace("</ParserSchema>", &region)
}

fn library(psc: &str) -> SchemaLibrary {
    build_library([("classes.psc", psc.as_bytes())]).expect("the classes parse")
}

/// Installs the snapshot, unless this process is the child that installs none; says whether one is installed.
fn snapshot() -> bool {
    static INSTALLED: OnceLock<bool> = OnceLock::new();
    *INSTALLED.get_or_init(|| {
        if std::env::var_os(CHILD_ENV).is_some() {
            assert!(!or_io::meta::has_default_library());
            return false;
        }
        or_io::meta::install_default_library(library(&snapshot_psc()));
        let installed = or_io::meta::default_library();
        let declares = |class: &str, member: &str| {
            let class = installed.resolve_any(class).expect("the snapshot's class");
            class.fields.iter().any(|f| f.name == member)
        };
        assert!(declares("CBaseArchetypeDef", "aMemberOfALaterBuild"));
        assert!(declares("CEntityDef", "lodLevel"));
        assert!(declares("CCarGen", "aMemberOfALaterBuild"));
        assert!(declares("CMloArchetypeDef", "aMemberOfALaterBuild"));
        assert!(declares("CMloRoomDef", "aMemberOfALaterBuild"));
        assert!(declares("CScenarioPoint", "aMemberOfALaterBuild"));
        assert!(declares("CScenarioChain", "EdgeIds"));
        true
    })
}

/// The name the index of names gives `name`, which is how a writer looks a member up.
fn spelt(name: &str) -> String {
    default_hash_index().name_or_literal(name_hash(name))
}

fn empty(library: &SchemaLibrary, ty: &FieldType) -> Value {
    match ty {
        FieldType::Bool => Value::Bool(false),
        FieldType::U8 | FieldType::U16 | FieldType::U32 => Value::UInt(0),
        FieldType::S8 | FieldType::S16 | FieldType::S32 => Value::Int(0),
        FieldType::Float => Value::Float(0.0),
        FieldType::Vector3 => Value::Vec3([0.0; 3]),
        FieldType::Vector4 => Value::Vec4([0.0; 4]),
        FieldType::String(StringKind::AtHashString) => Value::Hash(HashedString::default()),
        FieldType::String(StringKind::AtString) => Value::String(String::new()),
        FieldType::Array {
            kind: ArrayKind::AtArray,
            ..
        } => Value::Array(Vec::new()),
        FieldType::Struct(class) => Value::Struct(Box::new(record(library, class, &[]))),
        other => panic!("no class built here holds a {other:?}"),
    }
}

/// A `class` record of `library` with `members` set, every other member at its empty value.
fn record(library: &SchemaLibrary, class: &str, members: &[(&str, Value)]) -> MetaInstance {
    let resolved = library
        .resolve_any(class)
        .unwrap_or_else(|| panic!("`{class}` is a class of the file"));
    let mut inst = MetaInstance::new(class);
    for field in &resolved.fields {
        let set = members.iter().find(|(m, _)| *m == field.name);
        let value = match set {
            Some((_, value)) => value.clone(),
            None => empty(library, &field.ty),
        };
        inst.fields.push((spelt(&field.name), value));
    }
    for (member, _) in members {
        let stated = resolved.fields.iter().any(|f| f.name == *member);
        assert!(stated, "`{class}` states `{member}`");
    }
    inst
}

fn encoded(library: &SchemaLibrary, root: &MetaInstance, format: BinaryFormat) -> Vec<u8> {
    let layout = canonical::layout_for(library, root, format, None).expect("lays out");
    write_meta_binary(format, Some(&layout), root).expect("encodes")
}

/// `root` encoded as a file whose table names `class` for an array that holds nothing, and states it nowhere.
///
/// ⚠ A table states the classes its document holds, so a shipped file names the class of an empty array and states it
/// nowhere. A layout made here states it all the same, so the statement is taken out before the file is written.
fn encoded_naming_only(
    library: &SchemaLibrary,
    root: &MetaInstance,
    format: BinaryFormat,
    class: &str,
) -> Vec<u8> {
    let key = name_hash(class);
    let other = name_hash("AClassNoFileStates");
    let mut layout = canonical::layout_for(library, root, format, None).expect("lays out");
    match &mut layout.container {
        ContainerLayout::Rbm(rbm) => {
            // ⚠ The writer starts from the image the layout carries, so the class is given another name there: a
            // class record opens with its name, and the first one met from the start of the class table is its own.
            let table =
                usize::try_from(rbm.header.class_refs_ptr & 0x0FFF_FFFF).expect("an offset");
            let named = |at: &usize| rbm.decompressed[*at..*at + 4] == key.to_le_bytes();
            let ends = rbm.decompressed.len() - 4;
            let at = (table..ends).step_by(4).find(named);
            let at = at.expect("the table stated the class");
            rbm.decompressed[at..at + 4].copy_from_slice(&other.to_le_bytes());
        }
        ContainerLayout::Pso(pso) => {
            let is_class = |item: &&mut PsoSchemaItem| matches!(item, PsoSchemaItem::Struct(info) if info.name_hash == key);
            let stated = pso.schema.items.iter_mut().find(is_class);
            // An enum in its place, so the table holds no struct under the name.
            *stated.expect("the table stated the class") = PsoSchemaItem::Enum(PsoEnumInfo {
                name_hash: name_hash("AnEnumNoFileStates"),
                flags: 0,
                entries: Vec::new(),
            });
        }
        ContainerLayout::Rbf(_) => panic!("no file here is an RBF"),
    }
    let file = write_meta_binary(format, Some(&layout), root).expect("encodes");

    // The file reads as it did, and its table states every class it stated but that one.
    let stating = encoded(library, root, format);
    let stated = |bytes: &[u8]| {
        let doc = MetaDocument::parse(&SchemaLibrary::default(), bytes.to_vec()).expect("reads");
        let layout = doc.layout.expect("a binary document carries its layout");
        let tables = type_tables::of_layout(&layout);
        let mut stated: Vec<u32> = tables.structs.keys().map(|name| name_hash(name)).collect();
        stated.sort_unstable();
        stated
    };
    assert_eq!(
        tree(&file),
        tree(&stating),
        "{format:?}: the file reads as it did"
    );
    let (mut was, mut now) = (stated(&stating), stated(&file));
    assert!(
        was.contains(&key) && !now.contains(&key),
        "{format:?}: {class}"
    );
    was.retain(|stated| *stated != key);
    now.retain(|stated| *stated != other);
    assert_eq!(now, was, "{format:?}: every other class is still stated");
    file
}

fn hash(name: &str) -> Value {
    Value::Hash(HashedString::new(name))
}

/// What a file's class states of `own`, the member only the file states: a value, where the class has the member.
fn own(differs: Differs, value: u64) -> Vec<(&'static str, Value)> {
    match differs {
        Differs::Fewer => Vec::new(),
        Differs::More | Differs::Other => vec![("aMemberOnlyTheFileStates", Value::UInt(value))],
    }
}

/// The classes of a type file whose archetype class differs from the snapshot's as `differs` says.
fn type_file_psc(differs: Differs) -> String {
    match differs {
        Differs::Fewer => WORLD_TYPES.to_string(),
        Differs::More => with_member(&with_member(WORLD_TYPES, ASSET_NAME, LATER), LATER, OWN),
        Differs::Other => with_member(WORLD_TYPES, ASSET_NAME, OWN).replace(ASSET_NAME, ""),
    }
}

/// A type file of two archetypes, each holding its own value of the member only the file states.
fn type_file(differs: Differs, format: BinaryFormat) -> Vec<u8> {
    let library = library(&type_file_psc(differs));
    let archetype = |name: &str, lod_dist: f64, held: u64| {
        let mut members = vec![
            ("name", hash(name)),
            ("lodDist", Value::Float(lod_dist)),
            ("bbMax", Value::Vec3([1.0, 2.0, 3.0])),
            ("hdTextureDist", Value::Float(5.0)),
        ];
        members.extend(own(differs, held));
        Value::Pointer(Some(Box::new(record(
            &library,
            "CBaseArchetypeDef",
            &members,
        ))))
    };
    let archetypes = Value::Array(vec![
        archetype("prop_bench_01a", 80.0, 9),
        archetype("prop_lamp_02b", 120.0, 11),
    ]);
    let root = record(
        &library,
        "CMapTypes",
        &[("name", hash("hills_types")), ("archetypes", archetypes)],
    );
    encoded(&library, &root, format)
}

/// The classes of a map whose entity class differs from the snapshot's as `differs` says.
fn map_psc(differs: Differs) -> String {
    let later = r#"<u32 name="lodLevel"/><u32 name="priorityLevel"/>"#;
    match differs {
        Differs::Fewer => WORLD_TYPES.to_string(),
        Differs::More => with_member(&with_member(WORLD_TYPES, TINT, later), later, OWN),
        Differs::Other => with_member(WORLD_TYPES, TINT, OWN).replace(TINT, ""),
    }
}

/// The tree of a map of three entities, each holding its own value of the member only a file of `library` states.
fn map_tree(library: &SchemaLibrary, differs: Differs) -> MetaInstance {
    let entity = |guid: u64, x: f32| {
        let mut members = vec![
            ("archetypeName", hash("prop_bench_01a")),
            ("guid", Value::UInt(guid)),
            ("position", Value::Vec3([x, -40.25, 30.0])),
            ("rotation", Value::Vec4([0.0, 0.0, 0.0, 1.0])),
            ("scaleXY", Value::Float(1.0)),
            ("scaleZ", Value::Float(1.0)),
            ("parentIndex", Value::Int(-1)),
            ("lodDist", Value::Float(60.0)),
            ("ambientOcclusionMultiplier", Value::Int(255)),
            ("artificialAmbientOcclusion", Value::Int(255)),
        ];
        members.extend(own(differs, 20 + guid));
        Value::Pointer(Some(Box::new(record(library, "CEntityDef", &members))))
    };
    let entities = Value::Array(vec![entity(1, 10.0), entity(2, 20.0), entity(3, 30.0)]);
    record(
        library,
        "CMapData",
        &[("name", hash("hills")), ("entities", entities)],
    )
}

/// A map of three entities, each holding its own value of the member only the file states.
fn map_file(differs: Differs, format: BinaryFormat) -> Vec<u8> {
    let library = library(&map_psc(differs));
    encoded(&library, &map_tree(&library, differs), format)
}

/// A map of three entities whose table names a parked vehicle's class, for an array that holds nothing, and states it
/// nowhere.
fn map_naming_a_parked_vehicle(format: BinaryFormat) -> Vec<u8> {
    let library = library(&with_car_gens(WORLD_TYPES, ""));
    let root = map_tree(&library, Differs::Fewer);
    encoded_naming_only(&library, &root, format, "CCarGen")
}

/// A map that holds no entity, whose table neither states nor names an entity's class.
fn map_of_no_entity(format: BinaryFormat) -> Vec<u8> {
    let library = library(WORLD_TYPES);
    let root = record(&library, "CMapData", &[("name", hash("empty_hills"))]);
    encoded(&library, &root, format)
}

/// A type file of one interior of two rooms, a portal between them and two entities, whose archetype class differs
/// from the snapshot's as `differs` says.
fn interior_file(differs: Differs, format: BinaryFormat) -> Vec<u8> {
    let library = library(&with_interior(&type_file_psc(differs), ""));
    let entity = |guid: u64| {
        let members = [
            ("archetypeName", hash("prop_bench_01a")),
            ("guid", Value::UInt(guid)),
            ("rotation", Value::Vec4([0.0, 0.0, 0.0, 1.0])),
            ("scaleXY", Value::Float(1.0)),
            ("scaleZ", Value::Float(1.0)),
            ("parentIndex", Value::Int(-1)),
        ];
        Value::Pointer(Some(Box::new(record(&library, "CEntityDef", &members))))
    };
    let room = |name: &str, floor: i64| {
        let members = [
            ("name", Value::String(name.to_string())),
            ("bbMax", Value::Vec3([4.0, 3.0, 2.5])),
            ("floorId", Value::Int(floor)),
            ("attachedObjects", Value::Array(vec![Value::UInt(0)])),
        ];
        Value::Struct(Box::new(record(&library, "CMloRoomDef", &members)))
    };
    let corners = [
        [1.0, 3.0, 0.0],
        [-1.0, 3.0, 0.0],
        [-1.0, 3.0, 2.0],
        [1.0, 3.0, 2.0],
    ];
    let portal = record(
        &library,
        "CMloPortalDef",
        &[
            ("roomTo", Value::UInt(1)),
            ("corners", Value::Array(corners.map(Value::Vec3).to_vec())),
        ],
    );
    let mut members = vec![
        ("name", hash("v_int_hills")),
        ("lodDist", Value::Float(120.0)),
        ("mloFlags", Value::UInt(1)),
        ("entities", Value::Array(vec![entity(1), entity(2)])),
        (
            "rooms",
            Value::Array(vec![room("limbo", 0), room("hall", 1)]),
        ),
        (
            "portals",
            Value::Array(vec![Value::Struct(Box::new(portal))]),
        ),
    ];
    members.extend(own(differs, 9));
    let interior = record(&library, "CMloArchetypeDef", &members);
    let archetypes = Value::Array(vec![Value::Pointer(Some(Box::new(interior)))]);
    let root = record(
        &library,
        "CMapTypes",
        &[
            ("name", hash("hills_interiors")),
            ("archetypes", archetypes),
        ],
    );
    encoded(&library, &root, format)
}

/// The members a parked vehicle's class states besides those its record types, in a file whose class is `stating`.
#[derive(Debug, Clone, Copy, PartialEq)]
enum CarGenOf {
    /// The members the record types, and no other.
    Typed,
    /// Those and one no record types.
    WithOwn,
    /// Those without `livery`.
    WithoutLivery,
}

/// A map of one parked vehicle, whose class states what `stating` says.
fn car_gen_file(stating: CarGenOf, format: BinaryFormat, model: &str) -> Vec<u8> {
    let psc = match stating {
        CarGenOf::Typed => with_car_gens(WORLD_TYPES, ""),
        CarGenOf::WithOwn => with_car_gens(WORLD_TYPES, OWN),
        CarGenOf::WithoutLivery => without_member(&with_car_gens(WORLD_TYPES, ""), LIVERY),
    };
    let library = library(&psc);
    let mut members = vec![
        ("position", Value::Vec3([120.5, -40.25, 30.0])),
        ("orientX", Value::Float(0.5)),
        ("orientY", Value::Float(-0.75)),
        ("perpendicularLength", Value::Float(2.5)),
        ("carModel", hash(model)),
        ("flags", Value::UInt(0x0400_0680)),
        ("bodyColorRemap1", Value::Int(12)),
        ("bodyColorRemap2", Value::Int(-1)),
        ("bodyColorRemap3", Value::Int(3)),
        ("bodyColorRemap4", Value::Int(-1)),
        ("popGroup", hash("a_pop_group")),
    ];
    match stating {
        CarGenOf::Typed => members.push(("livery", Value::Int(4))),
        CarGenOf::WithOwn => {
            members.push(("livery", Value::Int(4)));
            members.push(("aMemberOnlyTheFileStates", Value::UInt(9)));
        }
        CarGenOf::WithoutLivery => {}
    }
    let parked = Value::Struct(Box::new(record(&library, "CCarGen", &members)));
    let root = record(
        &library,
        "CMapData",
        &[
            ("name", hash("car_park")),
            ("carGenerators", Value::Array(vec![parked])),
        ],
    );
    encoded(&library, &root, format)
}

const RADIUS: &str = r#"<u8 name="iRadius"/>"#;

/// The classes of a region whose point class differs from the snapshot's as `differs` says, and which names a chain's
/// class and states none.
fn region_file_psc(differs: Differs) -> String {
    match differs {
        Differs::Fewer => region_psc("", CHAIN),
        Differs::More => region_psc(&format!("{LATER}{OWN}"), CHAIN),
        Differs::Other => without_member(&region_psc(OWN, CHAIN), RADIUS),
    }
}

/// A region of two points and no chain, each point holding its own value of the member only the file states.
fn region_file(differs: Differs, format: BinaryFormat) -> Vec<u8> {
    let library = library(&region_file_psc(differs));
    let point = |x: f32, held: u64| {
        let mut members = vec![
            ("iProbability", Value::UInt(40)),
            ("iScenarioGroup", Value::UInt(3)),
            ("Flags", Value::UInt(0x0000_0240)),
            ("vPositionAndDirection", Value::Vec4([x, -40.25, 30.0, 1.5])),
        ];
        members.extend(own(differs, held));
        Value::Struct(Box::new(record(&library, "CScenarioPoint", &members)))
    };
    let points = Value::Array(vec![point(10.0, 9), point(20.0, 11)]);
    let container = record(&library, "CScenarioPointContainer", &[("MyPoints", points)]);
    let names = Value::Array(vec![hash("a_scenario_type")]);
    let look_ups = record(&library, "CScenarioPointLookUps", &[("TypeNames", names)]);
    let root = record(
        &library,
        "CScenarioPointRegion",
        &[
            ("VersionNumber", Value::Int(80)),
            ("Points", Value::Struct(Box::new(container))),
            ("LookUps", Value::Struct(Box::new(look_ups))),
        ],
    );
    encoded_naming_only(&library, &root, format, "CScenarioChain")
}

/// The points a region's tree states.
fn points(root: &MetaInstance) -> Vec<&MetaInstance> {
    match held(root, "Points") {
        Some(Value::Struct(container)) => elements(container, "MyPoints"),
        _ => Vec::new(),
    }
}

/// The tree a file states, read with no schema.
fn tree(bytes: &[u8]) -> MetaInstance {
    MetaDocument::parse(&SchemaLibrary::default(), bytes.to_vec())
        .expect("the file reads")
        .instance
}

fn elements<'a>(root: &'a MetaInstance, array: &str) -> Vec<&'a MetaInstance> {
    let key = name_hash(array);
    let found = root.fields.iter().find(|(n, _)| name_hash(n) == key);
    match found {
        Some((_, Value::Array(items))) => items
            .iter()
            .filter_map(|v| match v {
                Value::Struct(inst) | Value::Pointer(Some(inst)) => Some(&**inst),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The members `inst` states, by name hash, in the order it states them.
fn stated(inst: &MetaInstance) -> Vec<u32> {
    inst.fields.iter().map(|(n, _)| name_hash(n)).collect()
}

fn held<'a>(inst: &'a MetaInstance, member: &str) -> Option<&'a Value> {
    let key = name_hash(member);
    let found = inst.fields.iter().find(|(n, _)| name_hash(n) == key);
    found.map(|(_, v)| v)
}

/// A save that reads back as `edited` and saves again to the same bytes.
fn saved<T: File + PartialEq + std::fmt::Debug>(edited: &T, what: &str) -> Vec<u8> {
    let bytes = edited
        .to_bytes()
        .unwrap_or_else(|e| panic!("{what} saves: {e}"));
    let back = T::from_bytes(&bytes).unwrap_or_else(|e| panic!("{what} reads back: {e}"));
    assert_eq!(&back, edited, "{what} reads back as it was saved");
    let again = back
        .to_bytes()
        .unwrap_or_else(|e| panic!("{what} saves again: {e}"));
    assert_eq!(again, bytes, "{what} is at its fixed point");
    bytes
}

/// A type file takes an archetype added by copy, then removed; every archetype of each save states what the file's own
/// class states. What each save wrote is returned.
fn an_archetype_added_and_removed(differs: Differs, format: BinaryFormat) -> Vec<Vec<u8>> {
    let what = format!("{differs:?} {format:?}");
    let file = type_file(differs, format);
    let class = stated(elements(&tree(&file), "archetypes")[0]);
    let types = CMapTypes::from_bytes(&file).expect("the type file opens");
    assert_eq!(types.archetypes.len(), 2);

    let mut added = types.clone();
    added.archetypes.push(types.archetypes[0].clone());
    let with = saved(&added, &format!("{what}: the archetype added"));
    assert_eq!(detect_binary_format(&with), Some(format));
    let root = tree(&with);
    let archetypes = elements(&root, "archetypes");
    assert_eq!(archetypes.len(), 3, "{what}");
    for archetype in &archetypes {
        assert_eq!(
            stated(archetype),
            class,
            "{what}: an archetype states what the file's own class states"
        );
    }
    if differs != Differs::Fewer {
        let own = |i: usize| held(archetypes[i], "aMemberOnlyTheFileStates").cloned();
        assert_eq!(
            [own(0), own(1), own(2)],
            [
                Some(Value::UInt(9)),
                Some(Value::UInt(11)),
                Some(Value::UInt(9))
            ],
            "{what}: the member the snapshot lacks is carried"
        );
    }

    let mut removed = CMapTypes::from_bytes(&with).expect("reopens");
    removed.archetypes.remove(0);
    let without = saved(&removed, &format!("{what}: an archetype removed"));
    let root = tree(&without);
    let archetypes = elements(&root, "archetypes");
    assert_eq!(archetypes.len(), 2, "{what}");
    for archetype in &archetypes {
        assert_eq!(stated(archetype), class, "{what}");
    }
    vec![file, with, without]
}

/// A map has an entity taken out, then one built here put in; every entity of each save states what the file's own
/// class states. What each save wrote is returned.
fn an_entity_taken_out_and_one_built(differs: Differs, format: BinaryFormat) -> Vec<Vec<u8>> {
    let what = format!("{differs:?} {format:?}");
    let file = map_file(differs, format);
    let class = stated(elements(&tree(&file), "entities")[0]);
    let map = CMapData::from_bytes(&file).expect("the map opens");
    assert_eq!(map.entities.len(), 3);

    let mut taken = map.clone();
    taken.entities.remove(1);
    let without = saved(&taken, &format!("{what}: an entity taken out"));
    assert_eq!(detect_binary_format(&without), Some(format));
    let root = tree(&without);
    let entities = elements(&root, "entities");
    assert_eq!(entities.len(), 2, "{what}");
    for entity in &entities {
        assert_eq!(
            stated(entity),
            class,
            "{what}: an entity states what the file's own class states"
        );
    }
    if differs != Differs::Fewer {
        let own = |i: usize| held(entities[i], "aMemberOnlyTheFileStates").cloned();
        assert_eq!(
            [own(0), own(1)],
            [Some(Value::UInt(21)), Some(Value::UInt(23))],
            "{what}: the member the snapshot lacks is carried"
        );
    }

    // ⚠ A record built here holds no tree, so it is conformed to a class: the file's own, whatever the snapshot declares.
    let mut built = CMapData::from_bytes(&without).expect("reopens");
    let mut lamp = CEntityDef::default();
    lamp.archetype_name = HashedString::new("prop_lamp_02b");
    (lamp.guid, lamp.parent_index, lamp.lod_dist) = (7, -1, 45.0);
    (lamp.scale_xy, lamp.scale_z) = (1.0, 1.0);
    built.entities.push(CEntityDefAny::Base(lamp));
    let with = built
        .to_bytes()
        .unwrap_or_else(|e| panic!("{what}: an entity built here saves: {e}"));
    let root = tree(&with);
    let entities = elements(&root, "entities");
    assert_eq!(entities.len(), 3, "{what}");
    for entity in &entities {
        assert_eq!(
            stated(entity),
            class,
            "{what}: an entity built here states what the file's own class states"
        );
    }
    if differs != Differs::Fewer {
        assert_eq!(
            held(entities[2], "aMemberOnlyTheFileStates"),
            Some(&Value::UInt(0)),
            "{what}: at its empty value"
        );
    }
    let back = CMapData::from_bytes(&with).expect("the entity built here reads back");
    assert_eq!(back.entities[2].guid, 7);
    assert_eq!(back.entities[2].lod_dist, 45.0);
    assert_eq!(
        back.to_bytes().expect("saves again"),
        with,
        "{what}: at its fixed point"
    );
    vec![file, without, with]
}

/// A parked vehicle copied into a file whose class states a member the copy lacks takes that member: the typed one at
/// the value the record holds, the one no record types at its empty value. What each save wrote is returned.
fn a_parked_vehicle_copied_into_a_file_that_states_more(format: BinaryFormat) -> Vec<Vec<u8>> {
    let what = format!("{format:?}");
    let donor_file = car_gen_file(CarGenOf::WithoutLivery, format, "a_donor_model");
    let donor = CMapData::from_bytes(&donor_file).expect("the donor opens");
    assert_eq!(
        donor.car_generators[0].livery, -1,
        "a member the donor's class lacks reads as the engine's init"
    );

    let file = car_gen_file(CarGenOf::WithOwn, format, "a_model");
    let class = stated(elements(&tree(&file), "carGenerators")[0]);
    let mut map = CMapData::from_bytes(&file).expect("the map opens");
    map.car_generators.push(donor.car_generators[0].clone());
    let with = map
        .to_bytes()
        .unwrap_or_else(|e| panic!("{what}: the copy saves: {e}"));
    assert_eq!(detect_binary_format(&with), Some(format));
    let root = tree(&with);
    let parked = elements(&root, "carGenerators");
    assert_eq!(parked.len(), 2, "{what}");
    for vehicle in &parked {
        assert_eq!(
            stated(vehicle),
            class,
            "{what}: a parked vehicle states what the file's own class states"
        );
    }
    assert_eq!(
        held(parked[1], "aMemberOnlyTheFileStates"),
        Some(&Value::UInt(0)),
        "{what}: the member no record types, at its empty value"
    );
    let back = CMapData::from_bytes(&with).expect("the copy reads back");
    assert_eq!(back.car_generators.len(), 2);
    assert_eq!(back.car_generators[0], map.car_generators[0], "{what}");
    let copy = &back.car_generators[1];
    assert!(copy.car_model == "a_donor_model", "{what}");
    assert_eq!(
        (copy.livery, copy.flags, copy.body_color_remap1),
        (-1, 0x0400_0680, 12),
        "{what}: the typed member the copy lacked, at the engine's init the record read"
    );
    assert_eq!(
        back.to_bytes().expect("saves again"),
        with,
        "{what}: at its fixed point"
    );
    vec![donor_file, file, with]
}

/// The one interior of a type file.
fn interior(types: &CMapTypes) -> &CMloArchetypeDef {
    match &types.archetypes[0] {
        CBaseArchetypeDefAny::CMloArchetypeDef(mlo) => mlo,
        other => panic!("the archetype is the interior, and it read as {other:?}"),
    }
}

fn interior_mut(types: &mut CMapTypes) -> &mut CMloArchetypeDef {
    match &mut types.archetypes[0] {
        CBaseArchetypeDefAny::CMloArchetypeDef(mlo) => mlo,
        other => panic!("the archetype is the interior, and it read as {other:?}"),
    }
}

/// An interior takes a room as a copy, a room built here, a portal as a copy and an attachment, then loses each; the
/// interior and its rooms state what the file's own classes state at every save. What each save wrote is returned.
fn an_interior_takes_rooms_a_portal_and_an_attachment(
    differs: Differs,
    format: BinaryFormat,
) -> Vec<Vec<u8>> {
    let what = format!("{differs:?} {format:?}");
    let file = interior_file(differs, format);
    let stated_in = |bytes: &[u8]| {
        let root = tree(bytes);
        let interior = elements(&root, "archetypes")[0];
        let rooms: Vec<Vec<u32>> = (elements(interior, "rooms").iter())
            .map(|room| stated(room))
            .collect();
        let own = held(interior, "aMemberOnlyTheFileStates").cloned();
        (stated(interior), rooms, own)
    };
    let (class, rooms, own) = stated_in(&file);
    let room_class = &rooms[0];
    assert_eq!(own.is_some(), differs != Differs::Fewer);
    let types = CMapTypes::from_bytes(&file).expect("the type file opens");
    assert_eq!(interior(&types).rooms.len(), 2);

    let mut saves = vec![file.clone()];
    let mut keep = |edited: &CMapTypes, step: &str, rooms: usize| {
        let bytes = saved(edited, &format!("{what}: {step}"));
        assert_eq!(detect_binary_format(&bytes), Some(format));
        let (interior, held_rooms, held_own) = stated_in(&bytes);
        assert_eq!(
            interior, class,
            "{what}: {step}: the interior states its own class"
        );
        assert_eq!(held_rooms.len(), rooms, "{what}: {step}");
        for room in &held_rooms {
            assert_eq!(
                room, room_class,
                "{what}: {step}: a room states its own class"
            );
        }
        assert_eq!(
            held_own, own,
            "{what}: {step}: the member the snapshot lacks is carried"
        );
        saves.push(bytes);
    };

    let mut edited = types.clone();
    let hall = interior(&types).rooms[1].clone();
    interior_mut(&mut edited).rooms.push(hall);
    keep(&edited, "a room added as a copy", 3);

    let mut cellar = CMloRoomDef::default();
    (cellar.name, cellar.floor_id) = ("cellar".to_string(), -1);
    interior_mut(&mut edited).rooms.push(cellar);
    keep(&edited, "a room built here", 4);

    let mut door = interior(&types).portals[0].clone();
    (door.room_from, door.room_to) = (1, 3);
    interior_mut(&mut edited).portals.push(door);
    keep(&edited, "a portal added as a copy", 4);

    interior_mut(&mut edited).rooms[0].attached_objects.push(1);
    keep(&edited, "an attachment added", 4);

    interior_mut(&mut edited).portals.pop();
    interior_mut(&mut edited).rooms.truncate(2);
    interior_mut(&mut edited).rooms[0].attached_objects.pop();
    keep(&edited, "each taken out again", 2);
    let back = CMapTypes::from_bytes(saves.last().expect("the last save")).expect("reopens");
    assert_eq!(back, types, "{what}: the file reads as it was first read");
    saves
}

/// A region takes a point as a copy and loses a point; every point of each save states what the file's own class
/// states. What each save wrote is returned.
fn a_point_added_by_copy_and_removed(differs: Differs, format: BinaryFormat) -> Vec<Vec<u8>> {
    let what = format!("{differs:?} {format:?}");
    let file = region_file(differs, format);
    let class = stated(points(&tree(&file))[0]);
    let region = CScenarioPointRegion::from_bytes(&file).expect("the region opens");
    assert_eq!(region.points.my_points.len(), 2);

    let mut added = region.clone();
    let first = region.points.my_points[0].clone();
    added.points.my_points.push(first);
    let with = saved(&added, &format!("{what}: the point added"));
    assert_eq!(detect_binary_format(&with), Some(format));
    let root = tree(&with);
    let held_points = points(&root);
    assert_eq!(held_points.len(), 3, "{what}");
    for point in &held_points {
        assert_eq!(
            stated(point),
            class,
            "{what}: a point states what the file's own class states"
        );
    }
    if differs != Differs::Fewer {
        let own = |i: usize| held(held_points[i], "aMemberOnlyTheFileStates").cloned();
        assert_eq!(
            [own(0), own(1), own(2)],
            [
                Some(Value::UInt(9)),
                Some(Value::UInt(11)),
                Some(Value::UInt(9))
            ],
            "{what}: the member the snapshot lacks is carried"
        );
    }

    let mut removed = CScenarioPointRegion::from_bytes(&with).expect("reopens");
    removed.points.my_points.remove(0);
    let without = saved(&removed, &format!("{what}: a point removed"));
    let root = tree(&without);
    assert_eq!(points(&root).len(), 2, "{what}");
    for point in points(&root) {
        assert_eq!(stated(point), class, "{what}");
    }
    vec![file, with, without]
}

/// A point built in a region takes the members the region's own class states, the one no record types at its empty
/// value. What the save wrote is returned.
fn a_point_built_in_a_region(differs: Differs, format: BinaryFormat) -> Vec<u8> {
    let what = format!("{differs:?} {format:?}");
    let file = region_file(differs, format);
    let class = stated(points(&tree(&file))[0]);
    let mut region = CScenarioPointRegion::from_bytes(&file).expect("the region opens");
    let mut built = CScenarioPoint::default();
    (built.i_probability, built.i_scenario_group, built.flags) = (70, 2, 5);
    region.points.my_points.push(built);
    let with = saved(&region, &format!("{what}: a point built here"));
    assert_eq!(detect_binary_format(&with), Some(format));
    let root = tree(&with);
    let held_points = points(&root);
    assert_eq!(held_points.len(), 3, "{what}");
    for point in &held_points {
        assert_eq!(
            stated(point),
            class,
            "{what}: a point built here states what the file's own class states"
        );
    }
    if differs != Differs::Fewer {
        assert_eq!(
            held(held_points[2], "aMemberOnlyTheFileStates"),
            Some(&Value::UInt(0)),
            "{what}: at its empty value"
        );
    }
    let back = CScenarioPointRegion::from_bytes(&with).expect("reads back");
    let point = &back.points.my_points[2];
    assert_eq!(
        (point.i_probability, point.i_scenario_group, point.flags),
        (70, 2, 5),
        "{what}"
    );
    with
}

/// Why a save is refused, which is a refusal of the save and never the writer's own words.
fn refusal<T: File>(edited: &T, what: &str) -> String {
    let refused = match edited.to_bytes() {
        Ok(_) => panic!("{what} is refused"),
        Err(refused) => refused,
    };
    assert!(
        matches!(refused, or_io::Error::Save { .. }),
        "{what}: {refused}"
    );
    let why = refused.to_string();
    assert!(!why.contains("its schema declares"), "{what}: {why}");
    why
}

/// A point built in a region whose class lacks a member is refused the value it holds of that member, by name.
fn a_built_point_that_sets_a_member_the_regions_class_lacks(format: BinaryFormat) -> String {
    let file = region_file(Differs::Other, format);
    let mut region = CScenarioPointRegion::from_bytes(&file).expect("the region opens");
    let mut built = CScenarioPoint::default();
    built.i_radius = 5;
    region.points.my_points.push(built);
    let why = refusal(&region, "a point that sets a member its class lacks here");
    assert!(
        why.contains("`CScenarioPoint::iRadius` was set")
            && why.contains("has no such member to hold it"),
        "{format:?}: {why}"
    );
    why
}

/// How a record of a class the file only names is refused: a PSO has no layout for it, an RBM no signature.
fn names_the_class_alone(why: &str, class: &str, how: &str, format: BinaryFormat) {
    let stated = match (format, how) {
        (BinaryFormat::Rbm, _) => why.contains("signature"),
        (_, "built here") => {
            why.contains(&format!("names `{class}` and states none of its members"))
        }
        _ => why.contains(&format!("states no `{class}`")),
    };
    assert!(
        stated && why.contains(&format!("a `{class}` {how} cannot be saved into this file")),
        "{format:?}: {why}"
    );
}

/// A chain built in a region whose table only names a chain's class is refused by name, whatever the snapshot declares
/// of that class. Each sentence is returned.
fn a_chain_built_in_a_region_that_only_names_its_class(format: BinaryFormat) -> Vec<String> {
    let file = region_file(Differs::Fewer, format);
    let region = CScenarioPointRegion::from_bytes(&file).expect("the region opens");
    let mut stating = CScenarioChain::default();
    (stating.max_users, stating.edge_ids) = (3, vec![4, 5]);
    // ⚠ Every member at its init: a class of no member would hold it, and say nothing.
    let chains = [stating, CScenarioChain::default()];
    let refuse = |chain: &CScenarioChain| {
        let mut region = region.clone();
        region.chaining_graph.chains.push(chain.clone());
        let what = format!(
            "{format:?}: a chain of {} users built where the table only names its class",
            chain.max_users
        );
        let why = refusal(&region, &what);
        names_the_class_alone(&why, "CScenarioChain", "built here", format);
        why
    };
    chains.iter().map(refuse).collect()
}

/// A map whose table only names a parked vehicle's class has an entity taken out, and saves what it did before the
/// snapshot declared that class; a parked vehicle built in it, or copied into it, is refused by name. The save and
/// each sentence are returned.
fn a_map_that_only_names_a_parked_vehicles_class(format: BinaryFormat) -> (Vec<u8>, Vec<String>) {
    let what = format!("{format:?}");
    let file = map_naming_a_parked_vehicle(format);
    let map = CMapData::from_bytes(&file).expect("the map opens");
    assert!(map.car_generators.is_empty(), "{what}");

    let mut taken = map.clone();
    taken.entities.remove(1);
    let without = saved(&taken, &format!("{what}: an entity taken out"));
    assert_eq!(elements(&tree(&without), "entities").len(), 2, "{what}");

    let mut built = map.clone();
    let mut parked = CCarGen::default();
    parked.car_model = HashedString::new("a_model");
    built.car_generators.push(parked);
    let built = refusal(
        &built,
        "a parked vehicle built where the table only names its class",
    );
    names_the_class_alone(&built, "CCarGen", "built here", format);

    let donor = CMapData::from_bytes(&car_gen_file(CarGenOf::Typed, format, "a_donor_model"))
        .expect("the donor opens");
    let mut copied = map.clone();
    copied.car_generators.push(donor.car_generators[0].clone());
    let copied = refusal(
        &copied,
        "a parked vehicle copied where the table only names its class",
    );
    names_the_class_alone(&copied, "CCarGen", "copied here", format);
    (without, vec![built, copied])
}

/// What an entity copied into a map that holds none comes to: `saved`, where it reads back and is at its fixed point,
/// or the sentence it is refused with.
fn an_entity_copied_into_a_map_that_holds_none(differs: Differs, format: BinaryFormat) -> String {
    let donor = CMapData::from_bytes(&map_file(differs, format)).expect("the donor opens");
    let mut map = CMapData::from_bytes(&map_of_no_entity(format)).expect("the map opens");
    map.entities.push(donor.entities[0].clone());
    match map.to_bytes() {
        Ok(bytes) => {
            let back = CMapData::from_bytes(&bytes).expect("the copy reads back");
            assert_eq!(back.entities.len(), 1);
            assert_eq!(back.entities[0].guid, 1);
            assert_eq!(back.to_bytes().expect("saves again"), bytes);
            // ⚠ Laid out by a class no file gave: as RBM it carries the zero signature no shipped file carries.
            let doc = MetaDocument::parse(&SchemaLibrary::default(), bytes).expect("reads");
            if let Some(ContainerLayout::Rbm(rbm)) = doc.layout.as_ref().map(|l| &l.container) {
                let class = rbm.model.class_for(name_hash("CEntityDef"));
                assert_eq!(class.map(|class| class.signature), Some(0));
            }
            "saved".to_string()
        }
        Err(refused) => {
            assert!(matches!(refused, or_io::Error::Save { .. }), "{refused}");
            refused.to_string()
        }
    }
}

/// Every save of every edit above and every sentence an edit is refused with, in one order, with what it is: what a
/// process that installed a snapshot and one that installed none write and say alike.
fn every_save() -> Vec<(String, Vec<u8>)> {
    let mut saves = Vec::new();
    let mut keep = |what: String, written: Vec<Vec<u8>>| {
        for (step, bytes) in written.into_iter().enumerate() {
            saves.push((format!("{what} save {step}"), bytes));
        }
    };
    let said = |sentences: Vec<String>| sentences.into_iter().map(String::into_bytes).collect();
    for format in FORMATS {
        for differs in DIFFERENCES {
            keep(
                format!("archetype {differs:?} {format:?}"),
                an_archetype_added_and_removed(differs, format),
            );
            keep(
                format!("entity {differs:?} {format:?}"),
                an_entity_taken_out_and_one_built(differs, format),
            );
            keep(
                format!("interior {differs:?} {format:?}"),
                an_interior_takes_rooms_a_portal_and_an_attachment(differs, format),
            );
            keep(
                format!("point {differs:?} {format:?}"),
                a_point_added_by_copy_and_removed(differs, format),
            );
            keep(
                format!("built point {differs:?} {format:?}"),
                vec![a_point_built_in_a_region(differs, format)],
            );
        }
        keep(
            format!("parked vehicle {format:?}"),
            a_parked_vehicle_copied_into_a_file_that_states_more(format),
        );
        keep(
            format!("built point refused {format:?}"),
            said(vec![
                a_built_point_that_sets_a_member_the_regions_class_lacks(format),
            ]),
        );
        keep(
            format!("chain refused {format:?}"),
            said(a_chain_built_in_a_region_that_only_names_its_class(format)),
        );
        let (without, refused) = a_map_that_only_names_a_parked_vehicles_class(format);
        keep(format!("map that names a class {format:?}"), vec![without]);
        keep(format!("parked vehicle refused {format:?}"), said(refused));
    }
    // ⚠ A PSO map that holds no entity takes one as the class declared in code, with a snapshot and with none.
    for differs in DIFFERENCES {
        let came_to = an_entity_copied_into_a_map_that_holds_none(differs, BinaryFormat::Pso);
        keep(
            format!("entity into no entity {differs:?} Pso"),
            said(vec![came_to]),
        );
    }
    saves
}

/// Where the two processes part: an entity copied into an RBM map that holds none, whose table neither states nor names
/// its class, for each difference.
fn where_the_two_part() -> Vec<(String, String)> {
    let came_to = |differs: Differs| {
        let came_to = an_entity_copied_into_a_map_that_holds_none(differs, BinaryFormat::Rbm);
        (format!("{differs:?}"), came_to)
    };
    DIFFERENCES.into_iter().map(came_to).collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn a_type_file_takes_an_archetype_added_and_removed_whatever_the_snapshot_declares_of_its_class() {
    assert!(snapshot(), "this binary's tests run with a snapshot");
    for format in FORMATS {
        for differs in DIFFERENCES {
            an_archetype_added_and_removed(differs, format);
        }
    }
}

#[test]
fn an_interior_takes_rooms_a_portal_and_an_attachment_whatever_the_snapshot_declares_of_its_classes(
) {
    assert!(snapshot(), "this binary's tests run with a snapshot");
    for format in FORMATS {
        for differs in DIFFERENCES {
            an_interior_takes_rooms_a_portal_and_an_attachment(differs, format);
        }
    }
}

#[test]
fn a_map_has_an_entity_taken_out_and_one_built_whatever_the_snapshot_declares_of_its_class() {
    assert!(snapshot(), "this binary's tests run with a snapshot");
    for format in FORMATS {
        for differs in DIFFERENCES {
            an_entity_taken_out_and_one_built(differs, format);
        }
    }
}

#[test]
fn a_region_takes_a_point_copied_and_a_point_built_whatever_the_snapshot_declares_of_its_class() {
    assert!(snapshot(), "this binary's tests run with a snapshot");
    for format in FORMATS {
        for differs in DIFFERENCES {
            a_point_added_by_copy_and_removed(differs, format);
            a_point_built_in_a_region(differs, format);
        }
        a_built_point_that_sets_a_member_the_regions_class_lacks(format);
    }
}

#[test]
fn a_record_of_a_class_the_table_only_names_is_refused_by_name_though_the_snapshot_declares_it() {
    assert!(snapshot(), "this binary's tests run with a snapshot");
    for format in FORMATS {
        a_chain_built_in_a_region_that_only_names_its_class(format);
        a_map_that_only_names_a_parked_vehicles_class(format);
    }
}

#[test]
fn a_parked_vehicle_copied_between_files_takes_the_members_the_file_it_enters_states() {
    assert!(snapshot(), "this binary's tests run with a snapshot");
    for format in FORMATS {
        a_parked_vehicle_copied_into_a_file_that_states_more(format);
    }
}

#[test]
fn a_parked_vehicle_that_holds_a_member_the_file_it_enters_lacks_is_refused_by_name() {
    assert!(snapshot(), "this binary's tests run with a snapshot");
    let member = spelt("aMemberOnlyTheFileStates");
    for format in FORMATS {
        let donor = CMapData::from_bytes(&car_gen_file(CarGenOf::WithOwn, format, "a_donor_model"))
            .expect("the donor opens");
        let file = car_gen_file(CarGenOf::Typed, format, "a_model");
        let mut map = CMapData::from_bytes(&file).expect("the map opens");
        map.car_generators.push(donor.car_generators[0].clone());
        let why = refusal(&map, "a copy that holds a member the save would drop");
        assert!(
            why.contains(&format!("a `CCarGen` here holds `{member}`"))
                && why.contains("states `CCarGen` without it"),
            "{format:?}: {why}"
        );

        // The copy taken out, the file is the one it was read as.
        map.car_generators.pop();
        saved(&map, "the file with the copy taken out");
    }
}

/// Not a test of its own: the saves a parent asks of a child that installs no snapshot.
#[test]
fn save_for_a_parent_with_no_snapshot() {
    use std::io::Write;
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    assert!(!snapshot());
    let saves = every_save();
    let apart = where_the_two_part();
    assert!(
        !or_io::meta::has_default_library(),
        "the child installed no snapshot"
    );
    let same = (saves.iter()).map(|(what, bytes)| format!("{SAME}\t{what}\t{}", hex(bytes)));
    let apart = (apart.iter()).map(|(what, came_to)| format!("{APART}\t{what}\t{came_to}"));
    let lines: Vec<String> = same.chain(apart).collect();
    let mut out = std::io::stdout();
    let _ = write!(out, "{BEGIN}{}{END}", lines.join("\n"));
    let _ = out.flush();
}

/// What the child that installed no snapshot printed under `kind`, a line for each save or outcome.
fn printed_by_a_child_with_no_snapshot(kind: &str) -> Vec<String> {
    let output = Command::new(std::env::current_exe().expect("this test binary"))
        .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, "1")
        .output()
        .expect("this test binary runs as a child");
    let text = String::from_utf8_lossy(&output.stdout);
    let error = String::from_utf8_lossy(&output.stderr);
    let (_, rest) = text
        .split_once(BEGIN)
        .unwrap_or_else(|| panic!("the child printed no start marker: {text}{error}"));
    let (printed, _) = rest
        .split_once(END)
        .unwrap_or_else(|| panic!("the child panicked: {rest}{error}"));
    let of_kind = |line: &str| {
        let (held, rest) = line.split_once('\t')?;
        (held == kind).then(|| rest.to_string())
    };
    printed.lines().filter_map(of_kind).collect()
}

#[test]
fn every_shape_edit_saves_the_bytes_a_process_with_no_snapshot_saves() {
    assert!(snapshot(), "this binary's tests run with a snapshot");
    let with: Vec<String> = (every_save().iter())
        .map(|(what, bytes)| format!("{what}\t{}", hex(bytes)))
        .collect();
    let without = printed_by_a_child_with_no_snapshot(SAME);

    // Three saves of an archetype, an entity and a point and six of an interior, a point built, for each difference;
    // three of a parked vehicle, a map that names a class, and five sentences, for each format; three for a PSO map.
    let of_a_difference = 3 + 3 + 6 + 3 + 1;
    let of_a_format = DIFFERENCES.len() * of_a_difference + 3 + 1 + 5;
    assert_eq!(with.len(), FORMATS.len() * of_a_format + DIFFERENCES.len());
    assert_eq!(with.len(), without.len(), "the same saves both times");
    for (with, without) in with.iter().zip(&without) {
        let what = with.split('\t').next().unwrap_or_default();
        assert!(
            with == without,
            "{what}: the snapshot changed what was written or said"
        );
    }
}

/// ⚠ The one place the two processes part, held here so that closing it is a change somebody makes on purpose: a class
/// the file's table neither states nor names is still laid out by an installed snapshot.
#[test]
fn a_copy_of_a_class_an_rbm_file_neither_states_nor_names_is_laid_out_by_the_snapshot_and_refused_with_none(
) {
    assert!(snapshot(), "this binary's tests run with a snapshot");
    let member = spelt("aMemberOnlyTheFileStates");
    let with = where_the_two_part();
    let without = printed_by_a_child_with_no_snapshot(APART);
    assert_eq!(with.len(), DIFFERENCES.len());
    assert_eq!(with.len(), without.len(), "the same edits both times");
    for (differs, (what, came_to)) in DIFFERENCES.into_iter().zip(&with) {
        match differs {
            // Every member the copy holds is one of the snapshot's class.
            Differs::Fewer => assert_eq!(came_to, "saved", "{what}"),
            Differs::More | Differs::Other => assert!(
                came_to.contains(&format!("a `CEntityDef` here holds `{member}`"))
                    && came_to.contains("the class as the installed schema snapshot declares it"),
                "{what}: {came_to}"
            ),
        }
    }
    for line in &without {
        let (what, came_to) = line.split_once('\t').unwrap_or_default();
        assert!(
            came_to.contains("a `CEntityDef` copied here cannot be saved into this file")
                && came_to.contains("signature"),
            "{what}: {came_to}"
        );
    }
}

#[test]
fn the_archetype_of_a_type_file_built_from_nothing_is_taken_out_whatever_the_snapshot_declares() {
    assert!(snapshot(), "this binary's tests run with a snapshot");
    let bench = |name: &str| {
        let mut bench = or_io::meta::CBaseArchetypeDef::default();
        bench.lod_dist = 80.0;
        bench.name = HashedString::new(name);
        CBaseArchetypeDefAny::Base(bench)
    };
    let mut built = CMapTypes::default();
    built.name = HashedString::new("hills_types");
    built.archetypes = vec![bench("prop_bench_01a"), bench("prop_lamp_02b")];
    let file = built
        .to_bytes()
        .expect("a type file built from nothing saves");
    let mut types = CMapTypes::from_bytes(&file).expect("reopens");
    types.archetypes.remove(0);
    let without = saved(&types, "the type file with an archetype taken out");
    let root = tree(&without);
    assert_eq!(elements(&root, "archetypes").len(), 1);
    assert_eq!(
        held(elements(&root, "archetypes")[0], "aMemberOfALaterBuild"),
        None,
        "the snapshot's later member is not stated"
    );
}
