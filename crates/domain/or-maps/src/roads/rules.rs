//! The rules of a set of path regions: what is wrong in its nodes and links, as data with a code and a severity.
//!
//! A rule is a row of [`RULES`] and a finding is one place a rule applies. The rules read what the documents state
//! (`or_source::paths::stated`) and not what they cook to, so a link to a node its region does not hold and a node of more links than
//! its word counts, which a compile refuses, are findings here.
//!
//! ⚠ **A rule that was not decided is said, never passed over.** The two that measure a node against the ground need the ground, which
//! [`check`] is handed as a closure, and are [`Skipped`] when it is handed none. The rows of [`UNDECIDED`] are rules this build cannot
//! decide at all, each with the reason, and are skipped in every report.
//!
//! ⚠ A position is the integers the file states: nothing in a path region states the divisor of its fixed point, so the ground is asked
//! in those units and none is converted here.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use or_io::gta5::CPathNode;
use or_source::paths::Stated;
use serde::Serialize;

use super::{Address, Listed};

pub use or_project::content::rule::{Rule, Severity};

use Severity::{Error, Info, Warning};

const fn rule(code: &'static str, severity: Severity, summary: &'static str) -> Rule {
    // No file of a set holds a waiver, so no finding can be set aside.
    Rule::new(code, severity, false, summary)
}

/// Every rule, in the order a report lists them.
pub const RULES: &[Rule] = &[
    rule(
        "region-unreadable",
        Error,
        "the document of a path region does not read",
    ),
    rule(
        "region-twice",
        Error,
        "two sources state the nodes of one region, and the game finds a node in one file",
    ),
    rule(
        "links-uncountable",
        Error,
        "a node holds more links than its word can count",
    ),
    rule(
        "link-node-missing",
        Warning,
        "a link names a node past the nodes its region holds",
    ),
    rule(
        "link-region-absent",
        Info,
        "a link names a node of a region the set does not hold, so nothing says the node is there",
    ),
    rule(
        "link-one-way",
        Warning,
        "a link names a node that states no link back",
    ),
    rule(
        "link-without-lane",
        Warning,
        "a link has no lane either way",
    ),
    rule(
        "link-road-to-footpath",
        Warning,
        "a link joins a vehicle node and a pedestrian node",
    ),
    rule(
        "node-unreached",
        Warning,
        "no link of the set reaches a node, and none of its own leaves the set",
    ),
    rule("nodes-coincide", Warning, "two nodes stand at one place"),
    rule(
        "node-in-air",
        Warning,
        "a node stands above the ground under it",
    ),
    rule(
        "node-buried",
        Warning,
        "a node stands under the ground above it",
    ),
];

/// The rules of [`RULES`] this build cannot decide, each with the reason a report gives.
pub const UNDECIDED: &[(&str, &str)] = &[(
    "link-without-lane",
    "the declaration of a link names its far node and one word, and no lane in that word, so nothing here can count the lanes of a link",
)];

/// The two rules that need the ground.
const GROUND_RULES: [&str; 2] = ["node-in-air", "node-buried"];

pub fn rule_of(code: &str) -> Option<&'static Rule> {
    RULES.iter().find(|r| r.code == code)
}

/// The most links a node's word counts: the last count the declaration takes.
pub fn most_links() -> usize {
    static MOST: OnceLock<usize> = OnceLock::new();
    *MOST.get_or_init(|| {
        let mut probe = CPathNode::default();
        (0..=u8::MAX as usize)
            .take_while(|count| probe.set_num_links(*count).is_ok())
            .last()
            .unwrap_or(0)
    })
}

/// Which kind the node at `index` is: a region holds its vehicle nodes first, and nothing in a node says which it is.
fn kind(stated: &Stated, index: u16) -> &'static str {
    if i32::from(index) < stated.num_nodes_car_nodes {
        "vehicle"
    } else {
        "pedestrian"
    }
}

/// The ground a node is measured against.
pub struct Ground<'a> {
    /// The height of the ground under a node's `coors_x` and `coors_y`, in the units of its `coors_z`; none where no ground is known.
    pub height: &'a dyn Fn(i16, i16) -> Option<i16>,
    /// How far above or under the ground a node may stand, in the units of `coors_z`.
    pub slack: u16,
}

/// Where a finding is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Place {
    /// The folder of the source, by its name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<Address>,
    /// The place of the link among the links of its node.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<usize>,
}

/// One place a rule applies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub code: &'static str,
    pub severity: Severity,
    pub place: Place,
    pub message: String,
}

/// A rule a report did not decide, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Skipped {
    pub code: &'static str,
    pub why: String,
}

/// What the rules say of a set.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    pub findings: Vec<Finding>,
    pub skipped: Vec<Skipped>,
}

impl Report {
    /// Whether a finding is an error.
    pub fn blocks(&self) -> bool {
        self.findings.iter().any(|f| f.severity == Error)
    }
}

/// Every rule applied to `set`, measuring each node against `ground` when one is given.
pub fn check(set: &[Listed], ground: Option<&Ground<'_>>) -> Report {
    let mut report = Report::default();
    let mut say = |code: &'static str, place: Place, message: String| {
        report.findings.push(Finding {
            code,
            severity: rule_of(code).map_or(Error, |r| r.severity),
            place,
            message,
        });
    };
    let at = |source: &str, node: Option<Address>, link: Option<usize>| Place {
        source: Some(source.to_string()),
        node,
        link,
    };

    // The source that states each region: the first by name, which is the one the other rules read.
    let mut states: BTreeMap<u16, (&str, &Stated)> = BTreeMap::new();
    for listed in set {
        let source = listed.source.as_str();
        match &listed.stated {
            Err(why) => say(
                "region-unreadable",
                at(source, None, None),
                format!("`{source}` does not read: {why}"),
            ),
            Ok(stated) => {
                let Some(region) = stated.region else {
                    continue;
                };
                match states.get(&region) {
                    Some((first, _)) => say(
                        "region-twice",
                        at(source, None, None),
                        format!(
                            "`{source}` states the nodes of region {region}, which `{first}` states: the rules read `{first}` and pass `{source}` by"
                        ),
                    ),
                    None => {
                        states.insert(region, (source, stated));
                    }
                }
            }
        }
    }

    // ⚠ A node is its place: an index past what an address names is no node a link can reach.
    let nodes = |stated: &'_ Stated| {
        let count = stated.nodes.len().min(usize::from(u16::MAX) + 1);
        (0..count).map(|index| index as u16)
    };
    let mut reached: BTreeSet<Address> = BTreeSet::new();
    for (region, (_, stated)) in &states {
        for index in nodes(stated) {
            for (to_region, to_index) in &stated.nodes[usize::from(index)].links {
                let to = Address::new(*to_region, *to_index);
                if to != Address::new(*region, index) {
                    reached.insert(to);
                }
            }
        }
    }

    let mut stand: BTreeMap<[i16; 3], Vec<(Address, &str)>> = BTreeMap::new();
    for (region, (source, stated)) in &states {
        for index in nodes(stated) {
            let here = Address::new(*region, index);
            let node = &stated.nodes[usize::from(index)];
            let coors = [node.coors_x, node.coors_y, node.coors_z];
            stand.entry(coors).or_default().push((here, source));

            if node.links.len() > most_links() {
                say(
                    "links-uncountable",
                    at(source, Some(here), None),
                    format!(
                        "node {here} holds {} links, and its word counts {} at most",
                        node.links.len(),
                        most_links()
                    ),
                );
            }
            let mut leaves_the_set = false;
            for (place, (to_region, to_index)) in node.links.iter().enumerate() {
                let to = Address::new(*to_region, *to_index);
                let link = at(source, Some(here), Some(place));
                let Some((_, far)) = states.get(to_region) else {
                    leaves_the_set = true;
                    say(
                        "link-region-absent",
                        link,
                        format!(
                            "link {place} of node {here} names {to}, and region {to_region} is not in the set"
                        ),
                    );
                    continue;
                };
                let Some(named) = far.nodes.get(usize::from(*to_index)) else {
                    say(
                        "link-node-missing",
                        link,
                        format!(
                            "link {place} of node {here} names {to}, and region {to_region} holds {} node(s)",
                            far.nodes.len()
                        ),
                    );
                    continue;
                };
                if to != here && !named.links.contains(&(*region, index)) {
                    say(
                        "link-one-way",
                        link.clone(),
                        format!(
                            "link {place} of node {here} names {to}, which states no link back to {here}"
                        ),
                    );
                }
                let (from_kind, to_kind) = (kind(stated, index), kind(far, *to_index));
                if from_kind != to_kind {
                    say(
                        "link-road-to-footpath",
                        link,
                        format!(
                            "link {place} of node {here}, a {from_kind} node, names {to}, a {to_kind} node"
                        ),
                    );
                }
            }
            if !reached.contains(&here) && !leaves_the_set {
                say(
                    "node-unreached",
                    at(source, Some(here), None),
                    format!("no link of the set reaches node {here}"),
                );
            }
            let under = ground.and_then(|ground| {
                let height = (ground.height)(node.coors_x, node.coors_y)?;
                Some((
                    i32::from(node.coors_z) - i32::from(height),
                    i32::from(ground.slack),
                    height,
                ))
            });
            if let Some((above, slack, height)) = under {
                if above > slack {
                    say(
                        "node-in-air",
                        at(source, Some(here), None),
                        format!(
                            "node {here} stands at height {}, {above} above the ground under it at {height}",
                            node.coors_z
                        ),
                    );
                } else if -above > slack {
                    say(
                        "node-buried",
                        at(source, Some(here), None),
                        format!(
                            "node {here} stands at height {}, {} under the ground above it at {height}",
                            node.coors_z, -above
                        ),
                    );
                }
            }
        }
    }
    for (coors, standing) in &stand {
        let [(first, source), rest @ ..] = standing.as_slice() else {
            continue;
        };
        if rest.is_empty() {
            continue;
        }
        let others: Vec<String> = rest.iter().map(|(node, _)| node.to_string()).collect();
        say(
            "nodes-coincide",
            at(source, Some(*first), None),
            format!(
                "node {first} and {} stand at one place, ({}, {}, {})",
                others.join(", "),
                coors[0],
                coors[1],
                coors[2]
            ),
        );
    }

    // A report lists its findings in the order of the rules, each rule's in the order they were found.
    let order = |code: &str| RULES.iter().position(|r| r.code == code);
    report.findings.sort_by_key(|f| order(f.code));
    for (code, why) in UNDECIDED {
        report.skipped.push(Skipped {
            code,
            why: (*why).to_string(),
        });
    }
    if ground.is_none() {
        for code in GROUND_RULES {
            report.skipped.push(Skipped {
                code,
                why: "no ground was given to measure a node against".to_string(),
            });
        }
    }
    report
}
