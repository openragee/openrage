//! What one pose edit costs on a large map: the measure the world editor's plan asks for before tools are built on the operations.
//!
//! ⚠ A timing, so the test is ignored and run by name, under the profile that gives a test more than thirty seconds, and with
//! `--cargo-profile release` for the figure of a release build:
//! `cargo nextest run --profile all -p or-maps --test what_one_pose_edit_costs --run-ignored all --success-output final`.

use std::time::{Duration, Instant};

use or_maps::import::{add_files, Incoming};
use or_maps::operation::{apply, Operation, Operations, PoseData};
use or_maps::sample::{map_file, types_file, Archetype, Entity};

const SOURCE: &str = "large.metadata.src";

/// A pack whose one map file places `entities` crates on a grid, and the size of the XML its source holds.
fn a_large_map(dir: &std::path::Path, entities: usize) -> (std::path::PathBuf, u64) {
    let folder = dir.join("maps");
    let placed: Vec<Entity<'_>> = (0..entities)
        .map(|at| Entity {
            yaw: at as f32 * 0.01,
            ..Entity::at(
                ["prop_crate_a", "prop_crate_b"][at % 2],
                [(at % 200) as f32 * 4.0, (at / 200) as f32 * 4.0, 0.5],
                at as u32 + 1,
            )
        })
        .collect();
    let map = map_file("large", &placed, ([0.0; 3], [800.0, 400.0, 10.0]));
    let types = types_file(
        "large_types",
        &[
            Archetype::new("prop_crate_a", [-1.0; 3], [1.0; 3]),
            Archetype::new("prop_crate_b", [-2.0; 3], [2.0; 3]),
        ],
    );
    let incoming = |name: &'static str, bytes: &'static [u8]| Incoming {
        name,
        bytes,
        from_game: false,
        virtual_path: None,
    };
    let added = add_files(
        &folder,
        &[
            incoming("large.ymap", map.leak()),
            incoming("large_types.ytyp", types.leak()),
        ],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new());
    let xml = std::fs::metadata(folder.join(SOURCE).join("metadata.xml")).unwrap();
    (folder, xml.len())
}

fn least(times: &[Duration]) -> Duration {
    times.iter().copied().min().unwrap_or_default()
}

/// One `place_entity` through `apply`, on a synthetic map: the source cooked, the entity moved past the extents, the extents grown, the
/// XML written and the inverse proven; then the inverse, which puts the XML back to the byte.
///
/// Measured 2026-11-25 on an Apple M5 Ultra (30 cores, 256 GB of memory), the least of three edits, each on one thread, while other
/// lanes built on the machine (a load average near 50); a quieter run of the dev profile gave 2.8 s for the larger map.
///
/// | Entities | XML of the source | dev profile | release profile |
/// |---|---|---|---|
/// | 2,000 | 1.2 MB | 0.31 s | 0.25 s |
/// | 20,000 | 11.7 MB | 3.3 s | 2.5 s |
///
/// A timer placed by hand around the steps of the larger edit, in the dev profile, gave about 2.1 s of it to the XML
/// (`or_source::metadata::Inputs::xml_of`: exported, compiled back and exported again), 0.2 s to cooking the source, 0.3 s to
/// opening and saving the typed file, and 0.4 s to proving the inverse.
///
/// What a step of History would keep is measured beside it: the operation and its inverse as the JSON of a list, against the XML
/// before and after that a step of the Map screen's Undo keeps today. For the larger map that is 3,023 bytes against 23.4 MB, and the
/// test fails when the two operations are not under a hundredth of one copy of the XML.
///
/// ⚠ The sample's entity states the members the typed family reads and no more, so a game map costs more for each entity.
#[test]
#[ignore = "a timing; run by name"]
fn one_pose_edit_is_timed_on_synthetic_maps_of_two_thousand_and_twenty_thousand_entities() {
    for entities in [2_000, 20_000] {
        let scratch = tempfile::tempdir().unwrap();
        let (folder, xml) = a_large_map(scratch.path(), entities);
        let file = folder.join(SOURCE).join("metadata.xml");
        let place = |x: f32| Operation::PlaceEntity {
            source: SOURCE.into(),
            entity: entities / 2,
            pose: PoseData {
                position: [x, 150.0, 0.5],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale_xy: 1.0,
                scale_z: 1.0,
            },
        };
        let (mut edits, mut inverses) = (Vec::new(), Vec::new());
        let mut kept = 0;
        for step in 0..3 {
            let before = std::fs::read(&file).unwrap();
            let operation = place(900.0 + step as f32);
            let started = Instant::now();
            let applied = apply(&folder, &operation).unwrap();
            edits.push(started.elapsed());
            assert!(applied.edited.changed());
            let step_of_history = Operations::new(vec![operation, applied.inverse.clone()]);
            kept = step_of_history.to_json().unwrap().len() as u64;
            assert!(
                kept < xml / 100,
                "a step that holds the operation and its inverse keeps {kept} bytes of {xml}"
            );
            assert_eq!(applied.inverse.name(), "replace_entity");

            let started = Instant::now();
            apply(&folder, &applied.inverse).unwrap();
            inverses.push(started.elapsed());
            assert!(
                std::fs::read(&file).unwrap() == before,
                "the inverse puts {entities} entities back to the byte"
            );
        }
        println!(
            "{entities} entities, {xml} bytes of XML: one pose edit in {:?} (the least of {edits:?}), its inverse in {:?}; \
             the operation and its inverse are {kept} bytes of JSON",
            least(&edits),
            least(&inverses)
        );
    }
}
