//! The body of `deleting_an_entity_of_the_install_keeps_every_link`, run where no install is: over map files built by hand and
//! read by path, as the suite reads the files of a mount.
//!
//! ⚠ This is the run that says the install suite's own checks can pass and can fail. It says nothing of a shipped map, which only
//! the install suite reads.

mod common;

use std::collections::BTreeMap;

use common::chain::{delete_one, survey, Report};
use or_maps::sample::{child_map_file, map_file, Entity};

const BOX: ([f32; 3], [f32; 3]) = ([0.0; 3], [100.0; 3]);

/// An entity under the entity `parent` of the parent map.
fn under<'a>(archetype: &'a str, position: [f32; 3], guid: u32, parent: i32) -> Entity<'a> {
    Entity {
        parent,
        in_parent_map: true,
        ..Entity::at(archetype, position, guid)
    }
}

/// Four map files by the path a mount would give each: `town_slod`, `town_lod` under it and `town` under that, each entity of the
/// two lower ones under a far model of the map above, and `field`, which has no parent map and one far model of its own entities,
/// listed after the first of them.
fn tree() -> BTreeMap<String, Vec<u8>> {
    let stands = |archetype: &'static str, position: [f32; 3], guid: u32, children: u32| Entity {
        children,
        ..Entity::at(archetype, position, guid)
    };
    let slod = map_file(
        "town_slod",
        &[
            Entity::at("slod_sign", [1.0, 2.0, 3.0], 100),
            stands("slod_block", [10.0, 10.0, 0.0], 101, 2),
        ],
        BOX,
    );
    let lod = child_map_file(
        "town_lod",
        "town_slod",
        &[
            Entity {
                children: 1,
                ..under("lod_hall", [10.0, 10.0, 0.0], 201, 1)
            },
            Entity {
                children: 2,
                ..under("lod_row", [20.0, 10.0, 0.0], 202, 1)
            },
        ],
        BOX,
    );
    let town = child_map_file(
        "town",
        "town_lod",
        &[
            under("hall", [10.0, 10.0, 0.0], 301, 0),
            under("house", [18.0, 10.0, 0.0], 302, 1),
            under("house", [22.0, 10.0, 0.0], 303, 1),
        ],
        BOX,
    );
    let field = map_file(
        "field",
        &[
            Entity {
                parent: 2,
                ..Entity::at("hedge", [60.0, 60.0, 0.0], 402)
            },
            Entity::at("scarecrow", [50.0, 50.0, 0.0], 400),
            stands("hedge_far", [60.0, 60.0, 0.0], 401, 2),
            Entity {
                parent: 2,
                ..Entity::at("hedge", [64.0, 60.0, 0.0], 403)
            },
        ],
        BOX,
    );
    BTreeMap::from([
        ("x64/levels/town_slod.ymap".to_string(), slod),
        ("x64/levels/town_lod.ymap".to_string(), lod),
        ("x64/levels/town.ymap".to_string(), town),
        ("x64/levels/field.ymap".to_string(), field),
    ])
}

#[test]
fn the_checks_of_the_install_suite_hold_on_a_tree_of_maps_built_by_hand() {
    let tree = tree();
    let open = |path: &str| tree.get(path).cloned();
    let paths: Vec<String> = tree.keys().cloned().collect();
    let (facts, children) = survey(&paths, &open);
    assert_eq!(
        facts.len(),
        tree.len(),
        "each file is a map of its own name"
    );

    let mut report = Report {
        maps: facts.len(),
        ..Report::default()
    };
    for name in facts.keys() {
        report.sampled += 1;
        delete_one(&open, *name, &facts, &children, &mut report);
    }
    let text = report.text(1);
    assert!(report.failures.is_empty(), "{text}");
    assert_eq!(report.not_made, BTreeMap::new(), "{text}");
    // Every entity of `town_lod` stands for another, so the suite finds none of it to delete.
    assert_eq!(
        report.refused,
        BTreeMap::from([("every entity stands for another", 1)]),
        "{text}"
    );
    // The hall of `town` with its far model, the sign of `town_slod`, and a length of hedge of `field`.
    assert_eq!((report.deleted, report.ambiguous), (3, 0), "{text}");
    assert_eq!((report.chains, report.across_files), (1, 2), "{text}");
    assert_eq!(report.entities_gone, 4, "{text}");
    // The two houses name the far model after the hall's; the two far models of `town_lod` name the block after the sign.
    assert_eq!(report.renumbered_into_parent, 2 + 2, "{text}");
    // The second length of hedge names its far model, which stands after the first length.
    assert_eq!(report.renumbered, 1, "{text}");
    // The block counts the hall's far model no more, and the far hedge counts one length fewer.
    assert_eq!(report.lowered, 2, "{text}");
    assert_eq!((report.notes, report.rewritten), (0, 0), "{text}");
}
