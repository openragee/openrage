//! The rules of a maps module fire on the map files that break them, each with the place it is at, and stay quiet on a map that does not;
//! a waiver with a reason sets a finding aside and one without a reason is a finding.
//!
//! ⚠ The files are the crate's samples, so the clean checkout runs this; `what_the_rules_say_of_the_install` holds the rules to the
//! files of a mounted game.

use std::collections::BTreeSet;
use std::path::Path;

use or_maps::import::{add_files, Incoming};
use or_maps::model::{Maps, Waiver};
use or_maps::sample::{map_file, types_file, Archetype, Entity};
use or_maps::{check, inspect, Depth, Finding, Severity};

fn add(folder: &Path, name: &str, bytes: &[u8]) {
    let added = add_files(
        folder,
        &[Incoming {
            name,
            bytes,
            from_game: false,
            virtual_path: None,
        }],
        None,
    )
    .unwrap();
    assert_eq!(added.refused, Vec::<(String, String)>::new(), "{name}");
}

/// The findings of the module in `folder`, as `code` and the source each is at.
fn findings(folder: &Path, known: Option<&dyn Fn(u32) -> bool>) -> Vec<Finding> {
    let model = Maps::read(folder).unwrap();
    let facts = inspect(folder, &model, Depth::Open);
    check(&model, &facts, known)
}

fn codes(found: &[Finding]) -> BTreeSet<&'static str> {
    found.iter().map(|f| f.code).collect()
}

const CRATE: &str = "prop_crate_a";

fn clean_module(dir: &Path) -> std::path::PathBuf {
    let folder = dir.join("maps");
    add(
        &folder,
        "harbour.ymap",
        &map_file(
            "harbour",
            &[
                Entity::at(CRATE, [1.0, 1.0, 0.0], 1),
                Entity::at(CRATE, [5.0, 1.0, 0.0], 2),
            ],
            ([-1.0, -1.0, -1.0], [7.0, 3.0, 1.0]),
        ),
    );
    add(
        &folder,
        "harbour_types.ytyp",
        &types_file(
            "harbour_types",
            &[Archetype::new(CRATE, [-1.0; 3], [1.0; 3])],
        ),
    );
    folder
}

#[test]
fn a_map_that_breaks_nothing_is_met_with_nothing_that_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let folder = clean_module(dir.path());
    let found = findings(&folder, None);
    assert!(
        !found.iter().any(|f| f.severity != Severity::Info),
        "{:#?}",
        found
            .iter()
            .map(|f| (f.code, &f.message))
            .collect::<Vec<_>>()
    );
    // The archetype names a model the set does not hold: said, as a note, because the game or another pack may have it.
    assert!(codes(&found).contains("archetype-without-model"));
}

#[test]
fn a_map_built_to_break_the_rules_is_met_with_each_of_them_at_its_place() {
    let dir = tempfile::tempdir().unwrap();
    let folder = clean_module(dir.path());
    add(
        &folder,
        "duplicates.ymap",
        &map_file(
            "duplicates",
            &[
                Entity::at(CRATE, [0.0, 0.0, 0.0], 7),
                Entity::at(CRATE, [0.0, 0.0, 0.0], 7),
                Entity::at("prop_ghost", [5.0, 0.0, 0.0], 9),
            ],
            ([-1.0; 3], [6.0, 1.0, 1.0]),
        ),
    );
    add(
        &folder,
        "wide.ymap",
        &map_file(
            "wide",
            &[Entity::at(CRATE, [500.0, 0.0, 0.0], 1)],
            ([-1.0; 3], [2.0; 3]),
        ),
    );
    add(
        &folder,
        "orphan.ymap",
        &map_file(
            "orphan",
            &[Entity {
                parent: 9,
                ..Entity::at(CRATE, [0.0, 0.0, 0.0], 1)
            }],
            ([-1.0; 3], [1.0; 3]),
        ),
    );
    add(
        &folder,
        "twice.ytyp",
        &types_file(
            "twice",
            &[
                Archetype::new(CRATE, [-1.0; 3], [1.0; 3]),
                Archetype::new("prop_upside_down", [1.0; 3], [-1.0; 3]),
            ],
        ),
    );

    let found = findings(&folder, None);
    let said = codes(&found);
    for code in [
        "entity-twice",
        "guid-twice",
        "archetype-undeclared",
        "extents-short",
        "hierarchy-broken",
        "archetype-twice",
        "bounds-inverted",
    ] {
        assert!(said.contains(code), "{code} did not fire: {said:?}");
    }
    assert!(
        !said.contains("archetype-missing"),
        "nobody was asked whether the game knows the name"
    );

    let at = |code: &str| {
        let f = found.iter().find(|f| f.code == code).unwrap();
        (
            f.place.source.clone().unwrap_or_default(),
            f.place.entity,
            f.place.archetype,
        )
    };
    assert_eq!(
        at("entity-twice"),
        ("duplicates.metadata.src".to_string(), Some(1), None)
    );
    assert_eq!(at("guid-twice").0, "duplicates.metadata.src");
    assert_eq!(at("archetype-undeclared").1, Some(2));
    assert_eq!(at("extents-short").0, "wide.metadata.src");
    assert_eq!(at("hierarchy-broken").0, "orphan.metadata.src");
    assert_eq!(
        at("bounds-inverted"),
        ("twice.metadata.src".to_string(), None, Some(1))
    );
    assert!(
        found
            .iter()
            .any(|f| f.code == "hierarchy-broken" && f.blocks()),
        "a parent past the entities blocks a build"
    );
    assert!(found
        .iter()
        .filter(|f| f.code == "entity-twice")
        .all(|f| !f.blocks()));

    // Asked, the game says it does not know the ghost: the note becomes the warning.
    let nothing_known = |_: u32| false;
    let asked = findings(&folder, Some(&nothing_known));
    let missing = asked
        .iter()
        .find(|f| f.code == "archetype-missing")
        .expect("fires");
    let unnamed = format!("hash_{:08X}", or_io::string_hash(b"prop_ghost"));
    assert!(
        missing.message.contains(&unnamed),
        "a name the files give no text for is spelled as a source's XML spells it: {}",
        missing.message
    );
    // Told the text of the name, the finding says it.
    let model = Maps::read(&folder).unwrap();
    let names =
        |hash: u32| (hash == or_io::string_hash(b"prop_ghost")).then(|| "prop_ghost".to_string());
    let facts = or_maps::inspect_named(&folder, &model, Depth::Open, Some(&names));
    let told = check(&model, &facts, Some(&nothing_known));
    let told = told.iter().find(|f| f.code == "archetype-missing").unwrap();
    assert!(told.message.contains("prop_ghost"), "{}", told.message);
    assert!(!codes(&asked).contains("archetype-undeclared"));
    let knows_ghost = |hash: u32| hash == or_io::string_hash(b"prop_ghost");
    assert!(!codes(&findings(&folder, Some(&knows_ghost))).contains("archetype-missing"));
}

#[test]
fn a_waiver_with_a_reason_sets_a_finding_aside_and_one_without_is_a_finding() {
    let dir = tempfile::tempdir().unwrap();
    let folder = clean_module(dir.path());
    add(
        &folder,
        "orphan.ymap",
        &map_file(
            "orphan",
            &[Entity {
                parent: 9,
                ..Entity::at(CRATE, [0.0, 0.0, 0.0], 1)
            }],
            ([-1.0; 3], [1.0; 3]),
        ),
    );
    add(
        &folder,
        "dup.ymap",
        &map_file(
            "dup",
            &[
                Entity::at(CRATE, [0.0; 3], 3),
                Entity::at(CRATE, [0.0; 3], 4),
            ],
            ([-1.0; 3], [1.0; 3]),
        ),
    );
    let mut model = Maps::read(&folder).unwrap();
    model.waivers = vec![
        Waiver {
            code: "entity-twice".into(),
            map: None,
            reason: "the doubled crate is a stack".into(),
        },
        // An error that no reason could mend is not waived.
        Waiver {
            code: "hierarchy-broken".into(),
            map: None,
            reason: "it is fine".into(),
        },
        Waiver {
            code: "guid-twice".into(),
            map: None,
            reason: "  ".into(),
        },
        Waiver {
            code: "no-such-rule".into(),
            map: None,
            reason: "x".into(),
        },
    ];
    model.write(&folder).unwrap();

    let found = findings(&folder, None);
    let twice = found.iter().find(|f| f.code == "entity-twice").unwrap();
    assert_eq!(
        twice.waived.as_deref(),
        Some("the doubled crate is a stack")
    );
    assert!(
        found
            .iter()
            .any(|f| f.code == "hierarchy-broken" && f.blocks()),
        "an error that cannot be waived still blocks"
    );
    assert!(found
        .iter()
        .any(|f| f.code == "waiver-without-reason" && f.blocks()));
    assert!(found.iter().any(|f| f.code == "waiver-unknown"));
}

/// The road rules and operations over the staged path regions: a corpus walk, held here because this binary is of the slow tier.
#[path = "common/roads_over_the_corpus.rs"]
mod roads_over_the_corpus;
