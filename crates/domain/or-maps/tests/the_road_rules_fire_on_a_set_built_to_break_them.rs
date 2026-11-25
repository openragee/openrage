//! Each rule of a set of path regions fires on a set built to break it and on no other rule's set, a clean set is clean, and a rule
//! that was not decided is said.
//!
//! ⚠ The regions are built by hand, so the clean checkout runs this.

#[path = "common/roads.rs"]
mod roads;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use or_maps::roads::rules::{check, most_links, Ground, Report, Severity, RULES, UNDECIDED};
use or_maps::roads::{read, sources, Address};
use roads::{extract, node, region, Node};
use serde_json::{json, Value};

/// Two nodes linked both ways: a set no rule fires on.
fn pair() -> Vec<Node> {
    vec![
        node([0, 0, 0], &[(7, 1, 0x11)]),
        node([100, 0, 0], &[(7, 0, 0x21)]),
    ]
}

fn seven(folder: &Path, nodes: &[Node]) {
    extract(folder, "nodes7", &region(7, nodes.len(), nodes, &[]));
}

fn edit_document(folder: &Path, name: &str, edit: impl FnOnce(&mut Value)) {
    let document = folder.join(name).join("paths.json");
    let mut stated: Value = serde_json::from_slice(&std::fs::read(&document).unwrap()).unwrap();
    edit(&mut stated);
    std::fs::write(&document, serde_json::to_string_pretty(&stated).unwrap()).unwrap();
}

/// The ground at height zero everywhere, with five units of slack.
fn flat(_: i16, _: i16) -> Option<i16> {
    Some(0)
}

fn report(folder: &Path, with_ground: bool) -> Report {
    let set = read(folder).unwrap();
    let ground = Ground {
        height: &flat,
        slack: 5,
    };
    check(&set, with_ground.then_some(&ground))
}

/// A set built to break one rule: what to build in the folder, and whether the rules are given the ground.
type Broken = (&'static str, fn(&Path), bool);

/// ⚠ A rule with no set here, that is not one of [`UNDECIDED`], fails `each_rule_fires_on_a_set_built_to_break_it_and_on_no_other`.
const BROKEN: &[Broken] = &[
    (
        "region-unreadable",
        |folder| {
            seven(folder, &pair());
            extract(folder, "nodes8", &region(8, 2, &shifted(8), &[]));
            std::fs::write(
                folder.join("nodes8.paths.src/paths.json"),
                "{ \"nodes\": 3 }",
            )
            .unwrap();
        },
        false,
    ),
    (
        "region-twice",
        |folder| {
            seven(folder, &pair());
            extract(folder, "nodes7_again", &region(7, 2, &pair(), &[]));
        },
        false,
    ),
    (
        "links-uncountable",
        |folder| {
            seven(folder, &pair());
            // A count a region cannot state, so the document is typed.
            edit_document(folder, "nodes7.paths.src", |doc| {
                let links = doc["nodes"][0]["links"].as_array_mut().unwrap();
                let link = links[0].clone();
                links.resize(most_links() + 1, link);
            });
        },
        false,
    ),
    (
        "link-node-missing",
        |folder| {
            let mut nodes = pair();
            nodes[0].links.push((8, 5, 0x12));
            seven(folder, &nodes);
            extract(folder, "nodes8", &region(8, 2, &shifted(8), &[]));
        },
        false,
    ),
    (
        "link-region-absent",
        |folder| {
            let mut nodes = pair();
            nodes[0].links.push((9, 0, 0x12));
            seven(folder, &nodes);
        },
        false,
    ),
    (
        "link-one-way",
        |folder| {
            // A ring: every node is reached and no link is returned.
            seven(
                folder,
                &[
                    node([0, 0, 0], &[(7, 1, 1)]),
                    node([100, 0, 0], &[(7, 2, 2)]),
                    node([200, 0, 0], &[(7, 0, 3)]),
                ],
            );
        },
        false,
    ),
    (
        "link-road-to-footpath",
        |folder| {
            // The pair, its second node the one pedestrian node of the region.
            extract(folder, "nodes7", &region(7, 1, &pair(), &[]));
        },
        false,
    ),
    (
        "node-unreached",
        |folder| {
            let mut nodes = pair();
            nodes.push(node([200, 0, 0], &[]));
            seven(folder, &nodes);
        },
        false,
    ),
    (
        "nodes-coincide",
        |folder| {
            let mut nodes = pair();
            nodes[1].coors = nodes[0].coors;
            seven(folder, &nodes);
        },
        false,
    ),
    (
        "node-in-air",
        |folder| {
            let mut nodes = pair();
            nodes[1].coors[2] = 6;
            seven(folder, &nodes);
        },
        true,
    ),
    (
        "node-buried",
        |folder| {
            let mut nodes = pair();
            nodes[1].coors[2] = -6;
            seven(folder, &nodes);
        },
        true,
    ),
];

/// The clean pair as the nodes of `region`, away from region 7's.
fn shifted(region: u16) -> Vec<Node> {
    vec![
        node([1000, 0, 0], &[(region, 1, 0x11)]),
        node([1100, 0, 0], &[(region, 0, 0x21)]),
    ]
}

fn codes(report: &Report) -> BTreeSet<&'static str> {
    report.findings.iter().map(|f| f.code).collect()
}

#[test]
fn a_set_with_nothing_wrong_has_no_finding_with_the_ground_or_without() {
    let scratch = tempfile::tempdir().unwrap();
    seven(scratch.path(), &pair());
    extract(scratch.path(), "nodes8", &region(8, 2, &shifted(8), &[]));
    for with_ground in [false, true] {
        let found = report(scratch.path(), with_ground);
        assert_eq!(found.findings, [], "with the ground: {with_ground}");
        assert!(!found.blocks());
    }
    // A node within the slack of the ground is on it.
    let scratch = tempfile::tempdir().unwrap();
    let mut nodes = pair();
    (nodes[0].coors[2], nodes[1].coors[2]) = (5, -5);
    seven(scratch.path(), &nodes);
    assert_eq!(report(scratch.path(), true).findings, []);
}

#[test]
fn each_rule_fires_on_a_set_built_to_break_it_and_on_no_other() {
    let mut covered = BTreeSet::new();
    for (code, build, with_ground) in BROKEN {
        let scratch = tempfile::tempdir().unwrap();
        build(scratch.path());
        let found = report(scratch.path(), *with_ground);
        assert_eq!(
            codes(&found),
            BTreeSet::from([*code]),
            "the set built to break {code}: {:#?}",
            found.findings
        );
        let rule = RULES.iter().find(|r| r.code == *code).expect("a rule");
        assert!(
            found.findings.iter().all(|f| f.severity == rule.severity),
            "{code}"
        );
        assert_eq!(found.blocks(), rule.severity == Severity::Error, "{code}");
        covered.insert(*code);
    }
    let undecided: BTreeSet<&str> = UNDECIDED.iter().map(|(code, _)| *code).collect();
    let rules: BTreeSet<&str> = RULES.iter().map(|r| r.code).collect();
    assert_eq!(rules.len(), RULES.len(), "a code names one rule");
    assert!(covered.is_disjoint(&undecided));
    assert_eq!(
        covered.union(&undecided).copied().collect::<BTreeSet<_>>(),
        rules,
        "every rule has a set built to break it, or is one this build cannot decide"
    );
}

#[test]
fn a_finding_says_where_it_is_and_what_it_found() {
    let at = |code: &str| -> (Report, tempfile::TempDir) {
        let scratch = tempfile::tempdir().unwrap();
        let (_, build, with_ground) = BROKEN.iter().find(|(c, ..)| *c == code).unwrap();
        build(scratch.path());
        (report(scratch.path(), *with_ground), scratch)
    };
    let place = |report: &Report| {
        let f = &report.findings[0];
        (f.place.source.clone().unwrap(), f.place.node, f.place.link)
    };
    let seven = "nodes7.paths.src".to_string();

    let (found, _held) = at("link-node-missing");
    assert_eq!(
        place(&found),
        (seven.clone(), Some(Address::new(7, 0)), Some(1))
    );
    assert_eq!(
        found.findings[0].message,
        "link 1 of node 7:0 names 8:5, and region 8 holds 2 node(s)"
    );
    let (found, _held) = at("link-region-absent");
    assert_eq!(
        place(&found),
        (seven.clone(), Some(Address::new(7, 0)), Some(1))
    );
    assert!(found.findings[0]
        .message
        .contains("region 9 is not in the set"));
    let (found, _held) = at("link-one-way");
    assert_eq!(found.findings.len(), 3, "each link of the ring");
    assert_eq!(
        place(&found),
        (seven.clone(), Some(Address::new(7, 0)), Some(0))
    );
    let (found, _held) = at("link-road-to-footpath");
    let said: Vec<&str> = found.findings.iter().map(|f| f.message.as_str()).collect();
    assert_eq!(
        said,
        [
            "link 0 of node 7:0, a vehicle node, names 7:1, a pedestrian node",
            "link 0 of node 7:1, a pedestrian node, names 7:0, a vehicle node"
        ]
    );
    assert_eq!(
        place(&found),
        (seven.clone(), Some(Address::new(7, 0)), Some(0))
    );
    let (found, _held) = at("node-unreached");
    assert_eq!(
        place(&found),
        (seven.clone(), Some(Address::new(7, 2)), None)
    );
    let (found, _held) = at("nodes-coincide");
    assert_eq!(found.findings.len(), 1, "one finding for the place");
    assert_eq!(
        found.findings[0].message,
        "node 7:0 and 7:1 stand at one place, (0, 0, 0)"
    );
    let (found, _held) = at("links-uncountable");
    assert_eq!(
        found.findings[0].message,
        format!(
            "node 7:0 holds {} links, and its word counts {} at most",
            most_links() + 1,
            most_links()
        )
    );
    let (found, _held) = at("node-in-air");
    assert_eq!(
        place(&found),
        (seven.clone(), Some(Address::new(7, 1)), None)
    );
    assert!(found.findings[0].message.contains("6 above the ground"));
    let (found, _held) = at("node-buried");
    assert!(found.findings[0].message.contains("6 under the ground"));
    let (found, _held) = at("region-twice");
    assert_eq!(
        place(&found),
        ("nodes7_again.paths.src".to_string(), None, None)
    );
    let (found, _held) = at("region-unreadable");
    assert_eq!(place(&found), ("nodes8.paths.src".to_string(), None, None));
    assert!(found.findings[0].message.contains("nodes"), "{found:?}");

    // A finding is data, with its node as a document writes it.
    let (found, _held) = at("link-one-way");
    assert_eq!(
        serde_json::to_value(&found.findings[0]).unwrap(),
        json!({
            "code": "link-one-way",
            "severity": "warning",
            "place": { "source": "nodes7.paths.src", "node": "7:0", "link": 0 },
            "message": "link 0 of node 7:0 names 7:1, which states no link back to 7:0",
        })
    );
}

#[test]
fn a_link_past_the_nodes_of_its_own_region_and_a_node_whose_link_leaves_the_set_are_judged() {
    // A link a compile refuses, which only a typed document states.
    let scratch = tempfile::tempdir().unwrap();
    seven(scratch.path(), &pair());
    edit_document(scratch.path(), "nodes7.paths.src", |doc| {
        doc["nodes"][0]["links"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "as_integer1": "0x00000001", "other_node": "7:9" }));
    });
    assert_eq!(
        codes(&report(scratch.path(), false)),
        BTreeSet::from(["link-node-missing"])
    );

    // A node nothing of the set reaches is not said unreached while one of its own links leaves the set.
    let scratch = tempfile::tempdir().unwrap();
    let mut nodes = pair();
    nodes.push(node([200, 0, 0], &[(9, 4, 1)]));
    seven(scratch.path(), &nodes);
    assert_eq!(
        codes(&report(scratch.path(), false)),
        BTreeSet::from(["link-region-absent"])
    );
}

#[test]
fn a_rule_that_was_not_decided_is_said_with_its_reason() {
    let scratch = tempfile::tempdir().unwrap();
    seven(scratch.path(), &pair());
    let skipped = |with_ground| -> BTreeMap<&'static str, String> {
        (report(scratch.path(), with_ground).skipped.into_iter())
            .map(|s| (s.code, s.why))
            .collect()
    };
    let without = skipped(false);
    assert_eq!(
        without.keys().copied().collect::<Vec<_>>(),
        ["link-without-lane", "node-buried", "node-in-air"]
    );
    assert!(without["node-in-air"].contains("no ground was given"));
    assert!(without["link-without-lane"].contains("no lane in that word"));
    // With the ground, only what this build cannot decide is left.
    assert_eq!(
        skipped(true).keys().copied().collect::<Vec<_>>(),
        ["link-without-lane"]
    );
    for (code, _) in UNDECIDED {
        assert!(RULES.iter().any(|r| r.code == *code), "{code}");
    }
}

#[test]
fn the_most_links_a_node_counts_is_what_the_declaration_takes() {
    let most = most_links();
    let mut node = or_io::gta5::CPathNode::default();
    assert!(node.set_num_links(most).is_ok());
    assert!(node.set_num_links(most + 1).is_err());
    assert!(most > 0);
}

#[test]
fn each_rule_is_listed_with_the_severity_its_brief_gives_it() {
    use Severity::{Error, Info, Warning};
    let stated: Vec<(&str, Severity)> = RULES.iter().map(|r| (r.code, r.severity)).collect();
    // A link into a region the set does not hold is a note, and a link past the nodes of a region it holds a warning.
    assert_eq!(
        stated,
        [
            ("region-unreadable", Error),
            ("region-twice", Error),
            ("links-uncountable", Error),
            ("link-node-missing", Warning),
            ("link-region-absent", Info),
            ("link-one-way", Warning),
            ("link-without-lane", Warning),
            ("link-road-to-footpath", Warning),
            ("node-unreached", Warning),
            ("nodes-coincide", Warning),
            ("node-in-air", Warning),
            ("node-buried", Warning),
        ]
    );
}

#[test]
fn a_report_lists_its_findings_in_the_order_of_the_rules() {
    // A node nothing reaches stands before a node whose link leaves the set: found in that order, and listed in the other.
    let scratch = tempfile::tempdir().unwrap();
    seven(
        scratch.path(),
        &[
            node([300, 0, 0], &[]),
            node([0, 0, 0], &[(7, 2, 1), (9, 0, 2)]),
            node([100, 0, 0], &[(7, 1, 3)]),
        ],
    );
    let found = report(scratch.path(), false);
    let listed: Vec<&str> = found.findings.iter().map(|f| f.code).collect();
    assert_eq!(listed, ["link-region-absent", "node-unreached"]);
    let order = |code: &str| RULES.iter().position(|r| r.code == code).unwrap();
    assert!(order(listed[0]) < order(listed[1]));
}

#[test]
fn a_node_only_its_own_link_reaches_is_not_reached() {
    let scratch = tempfile::tempdir().unwrap();
    let mut nodes = pair();
    nodes.push(node([200, 0, 0], &[(7, 2, 1)]));
    seven(scratch.path(), &nodes);
    let found = report(scratch.path(), false);
    assert_eq!(codes(&found), BTreeSet::from(["node-unreached"]));
    assert_eq!(found.findings[0].place.node, Some(Address::new(7, 2)));
}

#[test]
fn the_first_source_that_states_a_region_is_the_one_the_rules_read() {
    let ring = [
        node([0, 0, 0], &[(7, 1, 1)]),
        node([100, 0, 0], &[(7, 2, 2)]),
        node([200, 0, 0], &[(7, 0, 3)]),
    ];
    // The pair first by name: the ring is passed by, and nothing of it is found.
    let scratch = tempfile::tempdir().unwrap();
    seven(scratch.path(), &pair());
    extract(scratch.path(), "nodes7_ring", &region(7, 3, &ring, &[]));
    let found = report(scratch.path(), false);
    assert_eq!(codes(&found), BTreeSet::from(["region-twice"]));
    assert_eq!(
        found.findings[0].place.source.as_deref(),
        Some("nodes7_ring.paths.src")
    );
    // The ring first by name: it is the one read.
    let scratch = tempfile::tempdir().unwrap();
    seven(scratch.path(), &ring);
    extract(scratch.path(), "nodes7_pair", &region(7, 2, &pair(), &[]));
    assert_eq!(
        codes(&report(scratch.path(), false)),
        BTreeSet::from(["region-twice", "link-one-way"])
    );
}

#[test]
fn a_node_is_measured_against_the_ground_under_it_and_passed_over_where_none_is_known() {
    let scratch = tempfile::tempdir().unwrap();
    seven(
        scratch.path(),
        &[
            node([0, 0, 0], &[(7, 1, 1), (7, 2, 2)]),
            // On the ground where it rises.
            node([100, 0, 100], &[(7, 0, 3), (7, 3, 4)]),
            // High over a place whose ground nothing knows.
            node([0, 50, 77], &[(7, 0, 5)]),
            // At the foot of ground that has risen two hundred.
            node([200, 0, 0], &[(7, 1, 6)]),
        ],
    );
    // The ground rises with x along y = 0, and is known nowhere else.
    let slope = |x: i16, y: i16| (y == 0).then_some(x);
    let ground = Ground {
        height: &slope,
        slack: 5,
    };
    let found = check(&read(scratch.path()).unwrap(), Some(&ground));
    let said: Vec<&str> = found.findings.iter().map(|f| f.message.as_str()).collect();
    assert_eq!(
        said,
        ["node 7:3 stands at height 0, 200 under the ground above it at 200"]
    );
    assert_eq!(found.findings[0].code, "node-buried");
}

#[test]
fn only_the_paths_sources_directly_inside_the_folder_are_a_set() {
    let scratch = tempfile::tempdir().unwrap();
    let folder = scratch.path();
    seven(folder, &pair());
    // A source of another kind, a file named as a paths source is, and a paths source one folder down.
    std::fs::create_dir(folder.join("props.textures.src")).unwrap();
    std::fs::write(folder.join("nodes1.paths.src"), b"").unwrap();
    extract(
        &folder.join("deeper"),
        "nodes8",
        &region(8, 2, &shifted(8), &[]),
    );
    let named = |folder: &Path| -> Vec<String> {
        (sources(folder).unwrap().into_iter())
            .map(|(name, _)| name)
            .collect()
    };
    assert_eq!(named(folder), ["nodes7.paths.src"]);
    assert_eq!(report(folder, false).findings, []);

    // A paths source with no document is of the set, and is said not to read.
    std::fs::create_dir(folder.join("nodes9.paths.src")).unwrap();
    assert_eq!(named(folder), ["nodes7.paths.src", "nodes9.paths.src"]);
    let set = read(folder).unwrap();
    let why = set[1].stated.as_ref().expect_err("no document");
    assert!(why.starts_with("paths.json: cannot read: "), "{why}");
    assert_eq!(
        codes(&report(folder, false)),
        BTreeSet::from(["region-unreadable"])
    );
    assert!(read(&folder.join("no such folder")).is_err());
}

#[test]
fn a_link_is_judged_by_the_kind_the_region_of_each_node_states() {
    // Region 7 holds a vehicle node and a pedestrian one, region 8 two pedestrian nodes: each node links to its like in the other.
    let scratch = tempfile::tempdir().unwrap();
    let folder = scratch.path();
    let build = |vehicles_of_eight| {
        let seven = [
            node([0, 0, 0], &[(8, 0, 1)]),
            node([100, 0, 0], &[(8, 1, 2)]),
        ];
        let eight = [
            node([1000, 0, 0], &[(7, 0, 3)]),
            node([1100, 0, 0], &[(7, 1, 4)]),
        ];
        extract(folder, "nodes7", &region(7, 1, &seven, &[]));
        extract(folder, "nodes8", &region(8, vehicles_of_eight, &eight, &[]));
    };
    build(1);
    assert_eq!(report(folder, false).findings, []);
    // The same links, with both nodes of region 8 pedestrian: the vehicle node of region 7 is linked to a pedestrian node.
    for name in ["nodes7.paths.src", "nodes8.paths.src"] {
        std::fs::remove_dir_all(folder.join(name)).unwrap();
    }
    build(0);
    let found = report(folder, false);
    let said: Vec<&str> = found.findings.iter().map(|f| f.message.as_str()).collect();
    assert_eq!(
        said,
        [
            "link 0 of node 7:0, a vehicle node, names 8:0, a pedestrian node",
            "link 0 of node 8:0, a pedestrian node, names 7:0, a vehicle node"
        ]
    );
    assert_eq!(codes(&found), BTreeSet::from(["link-road-to-footpath"]));
}
