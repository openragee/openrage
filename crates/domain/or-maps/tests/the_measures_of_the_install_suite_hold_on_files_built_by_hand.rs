//! What the install suite of the doors, bounds and path regions counts and compares, run where no install is: over maps, type
//! files, a bound and a path region built by hand.
//!
//! ⚠ This is the run that says the suite's own counts can be wrong and are seen to be. It says nothing of a shipped file, which
//! only the install suite reads.

#[path = "common/install_measures.rs"]
mod install_measures;

use std::collections::HashMap;

use install_measures::{
    canonical_save, cooked_back, cooked_to, doors_of, doors_stated_in, winners_of, Cooked,
    DoorCount,
};
use or_io::decl::File;
use or_io::gta5::bounds::build::{self, BoundMesh};
use or_io::gta5::bounds::PhBoundAny;
use or_io::gta5::{CNodeAddress, CPathNode, CPathRegion, PhMaterialMgrId};
use or_io::meta::instance::HashedString;
use or_io::meta::{MetaInstance, Value};
use or_world::ExtensionKind;

fn pointers(items: Vec<MetaInstance>) -> Value {
    let boxed = |inst| Value::Pointer(Some(Box::new(inst)));
    Value::Array(items.into_iter().map(boxed).collect())
}

fn named(type_name: &str, member: &str, name: &str) -> MetaInstance {
    let mut inst = MetaInstance::new(type_name);
    let hash = or_io::string_hash(name.as_bytes());
    inst.set(member, Value::Hash(HashedString::from_hash(hash)));
    inst
}

fn archetype(name: &str, door: bool) -> MetaInstance {
    let mut inst = named("CBaseArchetypeDef", "name", name);
    inst.set("bbMin", Value::Vec3([0.0, -0.02, 0.0]));
    inst.set("bbMax", Value::Vec3([1.0, 0.02, 2.0]));
    if door {
        let extension = ExtensionKind::Door.template().expect("a named kind");
        inst.set("extensions", pointers(vec![extension]));
    }
    inst
}

fn file(type_name: &str, member: &str, items: Vec<MetaInstance>) -> MetaInstance {
    let mut root = MetaInstance::new(type_name);
    root.set(member, pointers(items));
    root
}

/// An entity that places the model `name`; a name marked `+` states the door extension itself, and one ending in `@`
/// places an interior.
fn entity(name: &&str) -> MetaInstance {
    let model = name.trim_end_matches(['+', '@']);
    let type_name = match name.ends_with('@') {
        true => "CMloInstanceDef",
        false => "CEntityDef",
    };
    let mut inst = named(type_name, "archetypeName", model);
    inst.set("position", Value::Vec3([0.0; 3]));
    inst.set("rotation", Value::Vec4([0.0, 0.0, 0.0, 1.0]));
    inst.set("scaleXY", Value::Float(1.0));
    inst.set("scaleZ", Value::Float(1.0));
    if name.contains('+') {
        let extension = ExtensionKind::Door.template().expect("a named kind");
        inst.set("extensions", pointers(vec![extension]));
    }
    inst
}

/// A map that places `models`, each as [`entity`] reads it.
fn map(models: &[&str]) -> MetaInstance {
    file("CMapData", "entities", models.iter().map(entity).collect())
}

/// An interior named `name` that holds `models`, each as [`entity`] reads it.
fn interior(name: &str, models: &[&str]) -> MetaInstance {
    let mut inst = archetype(name, false);
    inst.type_name = "CMloArchetypeDef".into();
    inst.set("entities", pointers(models.iter().map(entity).collect()));
    inst
}

#[test]
fn the_doors_of_two_maps_are_counted_by_what_found_each_and_a_model_with_no_name_once() {
    let types = file(
        "CMapTypes",
        "archetypes",
        vec![
            archetype("yard_latch", true),
            archetype("shop_door_l", false),
            archetype("side_gate", false),
            archetype("shop_doorframe", false),
            archetype("crate_a", false),
            // Its hatch states the door extension, as the entities of the installs' interiors do.
            interior("yard_interior", &["hatch+", "crate_a"]),
        ],
    );
    let maps = [
        map(&[
            "yard_latch",
            "shop_door_l",
            "crate_a+",
            "nothing_names_a",
            "yard_interior@",
        ]),
        map(&[
            "yard_latch",
            "side_gate",
            "shop_doorframe",
            "nothing_names_a",
            "nothing_names_b",
            // A placed interior is no door, whatever it states.
            "yard_interior+@",
        ]),
    ];
    let known = ["shop_door_l", "side_gate", "shop_doorframe", "crate_a"];
    let names: HashMap<u32, String> = (known.iter())
        .map(|name| (or_io::string_hash(name.as_bytes()), name.to_string()))
        .collect();
    let patterns = or_maps::doors::built_in_patterns().expect("the patterns of the tree");

    // One search of both maps, and one of each merged, count the same.
    let whole = doors_of(&maps, std::slice::from_ref(&types), &names, &patterns);
    let mut merged = DoorCount::default();
    for one in &maps {
        let alone = std::slice::from_ref(one);
        merged.merge(doors_of(
            alone,
            std::slice::from_ref(&types),
            &names,
            &patterns,
        ));
    }
    assert_eq!(whole, merged);

    let hash = |name: &str| or_io::string_hash(name.as_bytes());
    // The latch twice by its archetype, the crate once by the entity that states it, and the hatch where each map
    // places its interior. The placed interior is looked at and is no model placed, though nothing names it.
    let expected = DoorCount {
        entities: 15,
        by_extension: 5,
        by_name: [("*door*".to_string(), 1), ("*gate*".to_string(), 1)].into(),
        unnamed_entities: 3,
        unnamed_models: [hash("nothing_names_a"), hash("nothing_names_b")].into(),
        models: (known.iter().copied())
            .chain(["yard_latch", "nothing_names_a", "nothing_names_b", "hatch"])
            .map(hash)
            .collect(),
        stated_by_the_entity: 1,
    };
    assert_eq!(whole, expected);
    assert!(whole.by_extension >= whole.stated_by_the_entity);
    // The latch among the archetypes, and the hatch among the entities of the interiors, once however often it is placed.
    assert_eq!(doors_stated_in(std::slice::from_ref(&types)), (1, 1));
}

#[test]
fn the_copy_a_mount_would_load_is_the_last_of_its_leaf_at_that_copys_place() {
    let listed: Vec<(String, String)> = [
        ("a:/types/one.ytyp", "one.ytyp"),
        ("a:/types/two.ytyp", "Two.YTYP"),
        ("a:/maps/one.ymap", "one.ymap"),
        ("b:/patch/one.ytyp", "ONE.ytyp"),
        ("b:/patch/three.ytyp", "three.ytyp"),
    ]
    .map(|(logical, leaf)| (logical.to_string(), leaf.to_string()))
    .into();
    assert_eq!(
        winners_of(&listed, ".ytyp"),
        [
            "a:/types/two.ytyp",
            "b:/patch/one.ytyp",
            "b:/patch/three.ytyp"
        ]
    );
    assert_eq!(winners_of(&listed, ".ymap"), ["a:/maps/one.ymap"]);
    assert!(winners_of(&listed, ".ybn").is_empty());
}

#[test]
fn a_bound_and_a_path_region_built_here_cook_back_and_what_is_neither_is_said_unopened() {
    let mesh = BoundMesh {
        positions: vec![
            [0.0, 0.0, 0.0],
            [4.0, 0.0, 0.0],
            [4.0, 4.0, 0.0],
            [0.0, 4.0, 1.0],
        ],
        triangles: vec![[0, 1, 2], [0, 2, 3]],
        materials: vec![PhMaterialMgrId::default()],
        triangle_materials: vec![0, 0],
    };
    let geometry = build::geometry(&mesh).expect("a mesh a bound can hold");
    let bound = PhBoundAny::PhBoundGeometry(geometry)
        .to_bytes()
        .expect("a bound");
    // A node states its place in its region, which a source refuses to be told otherwise.
    let node = |index: u16, x: i16, y: i16| CPathNode {
        address: CNodeAddress {
            index,
            ..Default::default()
        },
        coors_x: x,
        coors_y: y,
        ..Default::default()
    };
    let region = CPathRegion {
        nodes: vec![node(0, 0, 0), node(1, 40, 0), node(2, 40, 40)],
        num_nodes_car_nodes: 2,
        num_nodes_ped_nodes: 1,
        ..Default::default()
    }
    .to_bytes()
    .expect("a path region");

    let work = tempfile::tempdir().unwrap();
    let at = |name: &str| work.path().join(name);
    assert_eq!(cooked_back("ybn", &bound, &at("bound")), Cooked::Same);
    assert_eq!(cooked_back("ynd", &region, &at("region")), Cooked::Same);
    for (extension, name) in [("ybn", "no_bound"), ("ynd", "no_region")] {
        let unopened = cooked_back(extension, b"neither a bound nor a region", &at(name));
        assert!(matches!(unopened, Cooked::Unopened(_)), "{unopened:?}");
    }
    // A bound handed in as a path region is not read as one.
    let crossed = cooked_back("ynd", &bound, &at("crossed"));
    assert!(matches!(crossed, Cooked::Unopened(_)), "{crossed:?}");

    // A source that cooks to other bytes than the save it is held to is said to differ, and a file no source is made of
    // is said refused.
    let moved = CPathRegion {
        nodes: vec![node(0, 0, 0), node(1, 44, 0), node(2, 40, 40)],
        num_nodes_car_nodes: 2,
        num_nodes_ped_nodes: 1,
        ..Default::default()
    }
    .to_bytes()
    .expect("a path region");
    let other = canonical_save("ynd", &moved).expect("it opens and saves");
    assert_ne!(Ok(&other), canonical_save("ynd", &region).as_ref());
    let differs = cooked_to("ynd", &region, &at("differs"), &other);
    assert_eq!(differs, Cooked::Differs);
    let refused = cooked_to("ybn", &region, &at("refused"), &other);
    assert!(matches!(refused, Cooked::Refused(_)), "{refused:?}");
}
