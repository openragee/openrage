//! The judgements the install suite of a map's records makes of its census, run where no install is: over records, maps of far
//! lights and signatures built by hand.
//!
//! ⚠ This is the run that says the suite's own judgements can fail and are seen to. It says nothing of a shipped file, which only
//! the install suite (`what_a_map_holds_besides_entities`) reads.

mod support;

#[path = "support/map_records.rs"]
mod map_records;

use map_records::{census_of_lights, key, light_map, Census, CAR_GEN, LOD_LIGHT, RECORDS};
use or_io::meta::{MetaInstance, Value};

/// The census of the members over records built here, so the install suite's own judgement is seen to fail.
#[test]
fn the_census_of_a_maps_records_names_one_typed_and_never_stated_and_one_stated_and_not_typed() {
    let mut census = Census::default();
    for (class, (name, typed)) in RECORDS.iter().enumerate() {
        let mut record = MetaInstance::new(*name);
        for member in *typed {
            record.set(*member, Value::UInt(0));
        }
        census.state(class, &record);
    }
    assert_eq!(census.members_that_differ(), Vec::<String>::new());

    census.stated[CAR_GEN].remove(&key("livery"));
    let mut wider = MetaInstance::new("CLODLight");
    wider.set("aMemberNoRecordTypes", Value::UInt(0));
    census.state(LOD_LIGHT, &wider);
    assert_eq!(
        census.members_that_differ(),
        [
            "CCarGen::livery is typed and never stated",
            "CLODLight::aMemberNoRecordTypes is stated and not typed"
        ]
    );
}

/// The pairing of the far lights over maps named here, so its judgement is seen to fail before a map is read.
#[test]
fn the_pairing_names_a_map_of_far_lights_with_no_parent_in_its_folder_and_one_out_of_step() {
    let mut census = census_of_lights(vec![
        light_map("a:/town/lights.ymap", "lights", "distant", 3, 0),
        light_map("a:/city/distant.ymap", "distant", "", 0, 3),
    ]);
    census.pair_the_lights();
    assert_eq!(census.pairs, 1);
    let said =
        "a:/town/lights.ymap: 0 maps of its folder are the parent it names and hold distant lights";
    assert_eq!(census.problems[0], said);
    assert!(census.problems[1].starts_with("1 maps hold distant lights, 0 are named as a parent"));

    let mut census = census_of_lights(vec![
        light_map("a:/town/lights.ymap", "lights", "distant", 3, 0),
        light_map("a:/town/distant.ymap", "distant", "", 0, 2),
    ]);
    census.pair_the_lights();
    let said = "a:/town/lights.ymap: holds 3 far lights and its parent 2 distant ones";
    assert_eq!(census.problems, [said]);
    assert_eq!((census.pairs, census.pairs_saved), (1, 0));

    let mut census = census_of_lights(vec![light_map("a:/town/both.ymap", "both", "", 3, 3)]);
    census.pair_the_lights();
    assert_eq!(
        census.problems[0],
        "a:/town/both.ymap: holds both families of far lights"
    );
}

/// The judgement of a class a file states otherwise than its record types, over signatures written here.
#[test]
fn a_class_stated_otherwise_counts_only_where_a_file_holds_a_record_of_it() {
    let mut census = Census::default();
    let narrower = "position: Vector3\n        flags: U32";
    let stated = census.signatures.entry("CCarGen").or_default();
    let narrow = stated.entry(narrower.to_string()).or_default();
    narrow.by_format.insert("PSO", (2, 0));
    assert_eq!(census.signatures_that_differ(), Vec::<String>::new());
    let stated = census.signatures.entry("CCarGen").or_default();
    let narrow = stated.entry(narrower.to_string()).or_default();
    narrow.by_format.insert("RBM", (1, 2));
    let differ = census.signatures_that_differ();
    assert_eq!(differ.len(), 1);
    assert!(differ[0].starts_with("CCarGen: 2 records are held by files that state it as"));
}
