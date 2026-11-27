//! The conformance kit refuses a broken title by the name of the rule it breaks: the fake title
//! built with each of its faults breaks one rule and no other, and every rule has its fault.

use std::collections::BTreeSet;

use or_world::fake::{fixture, FakeTitle, Fault};
use or_world::kit::{self, Rule};

/// The rule the fake title breaks when built with `fault`.
fn breaks(fault: Fault) -> Rule {
    match fault {
        Fault::LinkForgets => Rule::KeptLinks,
        Fault::ParentUnlinked => Rule::ChildNamesParent,
        Fault::ChildrenMiscounted => Rule::ParentUntilChildren,
        Fault::HoursIgnored => Rule::TimedMap,
        Fault::HeldStreams => Rule::HeldMap,
        Fault::InteriorUnread => Rule::Interior,
        Fault::ListLaysOne => Rule::List,
        Fault::WordsShort => Rule::Columns,
        Fault::LightsDropped => Rule::Lit,
        Fault::CapsLie => Rule::Caps,
        Fault::SessionHidesBody => Rule::ApplyShows,
        Fault::GuardIgnored => Rule::StaleGuard,
        Fault::InverseWrong => Rule::Inverse,
        Fault::SwitchTwice => Rule::Slots,
        Fault::StreamsEverywhere => Rule::Streams,
        Fault::BandOfNought => Rule::Policy,
        Fault::IndexPastVertices => Rule::Models,
    }
}

#[test]
fn each_broken_fake_breaks_the_rule_it_was_built_to_break_and_no_other() {
    let mut wrong = Vec::new();
    for fault in Fault::ALL {
        let broken = kit::broken(&fixture(FakeTitle::broken(fault)));
        let rules: Vec<Rule> = broken.iter().map(|(rule, _)| *rule).collect();
        if rules != [breaks(fault)] {
            wrong.push(format!(
                "{fault:?} should break {:?} alone and breaks {broken:#?}",
                breaks(fault)
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn every_rule_has_a_fake_built_to_break_it() {
    let broken: BTreeSet<Rule> = Fault::ALL.into_iter().map(breaks).collect();
    let rules: BTreeSet<Rule> = Rule::ALL.into_iter().collect();
    assert_eq!(broken, rules);
    assert_eq!(
        Fault::ALL.len(),
        Rule::ALL.len(),
        "a fault a rule, and no two for one"
    );
}

#[test]
fn the_whole_fake_breaks_no_rule() {
    assert_eq!(kit::broken(&fixture(FakeTitle::new())), []);
}

#[test]
#[should_panic(expected = "the title `fake` breaks the rule `a_stale_guard_refuses_the_operation`")]
fn a_broken_rule_fails_its_test_by_the_rules_name() {
    kit::hold(
        &fixture(FakeTitle::broken(Fault::GuardIgnored)),
        Rule::StaleGuard,
    );
}
