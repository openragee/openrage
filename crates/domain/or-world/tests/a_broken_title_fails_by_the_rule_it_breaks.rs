//! The conformance kit refuses a broken title by the name of the rule it breaks: the fake title
//! built with each of its faults breaks one rule and no other, and every rule has its fault.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, PoisonError};

use or_world::contract::{
    Address, Applied, Base, EditError, EditPlace, EditSession, EditTitle, Envelope, Guard, MapBody,
    OperationKind, Written,
};
use or_world::fake::{fixture, FakeEdits, FakeTitle, Fault};
use or_world::kit::{self, Rule};
use or_world::model::MapId;

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
        Fault::CapsLie | Fault::BaseStated | Fault::StoreUnstated => Rule::Caps,
        Fault::SessionHidesBody => Rule::ApplyShows,
        Fault::GuardIgnored => Rule::StaleGuard,
        Fault::InverseWrong => Rule::Inverse,
        Fault::SwitchTwice => Rule::Slots,
        Fault::StreamsEverywhere => Rule::Streams,
        Fault::BandOfNought => Rule::Policy,
        Fault::IndexPastVertices
        | Fault::ModelUnread
        | Fault::TextureUnread
        | Fault::ElementPastStride => Rule::Models,
        Fault::SunTwiceAUnit => Rule::Frame,
        Fault::DrawsShort => Rule::Draws,
        Fault::SessionBodyAfresh => Rule::SessionBody,
    }
}

#[test]
fn each_broken_fake_breaks_the_rule_it_was_built_to_break_and_no_other() {
    let mut wrong = Vec::new();
    let scratch = tempfile::tempdir().expect("a scratch folder");
    for fault in Fault::ALL {
        let under = scratch.path().join(format!("{fault:?}"));
        let broken = kit::broken(&fixture(FakeTitle::broken(fault)), &under);
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
}

#[test]
fn the_whole_fake_breaks_no_rule() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    assert_eq!(kit::broken(&fixture(FakeTitle::new()), scratch.path()), []);
}

#[test]
#[should_panic(expected = "the title `fake` breaks the rule `a_stale_guard_refuses_the_operation`")]
fn a_broken_rule_fails_its_test_by_the_rules_name() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    kit::hold(
        &fixture(FakeTitle::broken(Fault::GuardIgnored)),
        Rule::StaleGuard,
        scratch.path(),
    );
}

#[test]
fn every_broken_fake_fails_the_test_of_its_rule_and_the_failure_names_the_rule() {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    for fault in Fault::ALL {
        let (rule, under) = (breaks(fault), scratch.path().join(format!("{fault:?}")));
        // What the test the macro writes for the rule runs, over the broken title.
        let held = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            kit::hold(&fixture(FakeTitle::broken(fault)), rule, &under)
        }));
        let said = match held {
            Ok(()) => panic!("{fault:?} passes the test of the rule {rule:?}"),
            Err(panic) => match panic.downcast::<String>() {
                Ok(said) => *said,
                Err(_) => panic!("{fault:?} fails the test of {rule:?} with no words"),
            },
        };
        let names = format!("the title `fake` breaks the rule `{}`: ", rule.name());
        assert!(said.starts_with(&names), "{fault:?} fails with: {said}");
    }
}

/// The fake title's edit store, its sessions answering of a map the first body they answered of it.
struct Pinning(FakeEdits);

struct Pinned<'w> {
    inner: Box<dyn EditSession + 'w>,
    first: Mutex<HashMap<MapId, Arc<dyn MapBody>>>,
}

impl EditTitle for Pinning {
    fn operations(&self) -> Vec<OperationKind> {
        self.0.operations()
    }

    fn open<'w>(&self, place: &EditPlace<'_, 'w>) -> Result<Box<dyn EditSession + 'w>, EditError> {
        Ok(Box::new(Pinned {
            inner: self.0.open(place)?,
            first: Mutex::default(),
        }))
    }
}

impl EditSession for Pinned<'_> {
    fn guard(&self, at: &Address) -> Guard {
        self.inner.guard(at)
    }

    fn apply(&mut self, op: &Envelope) -> Result<Applied, EditError> {
        self.inner.apply(op)
    }

    fn body(&self, map: MapId) -> Option<Arc<dyn MapBody>> {
        let body = self.inner.body(map)?;
        let mut first = self.first.lock().unwrap_or_else(PoisonError::into_inner);
        Some(Arc::clone(first.entry(map).or_insert(body)))
    }

    fn flush(&mut self) -> Result<Vec<Written>, EditError> {
        self.inner.flush()
    }

    fn base(&self, at: &Address) -> Option<Base> {
        self.inner.base(at)
    }
}

#[test]
fn a_session_that_answers_an_old_body_after_an_apply_breaks_the_rule_of_its_body() {
    let title = FakeTitle::new();
    let mut fixture = fixture(title.clone());
    let edit = fixture.edit.as_mut().expect("the fake has an edit store");
    edit.title = Box::new(Pinning(title.edits()));
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let broke = kit::check(&fixture, Rule::SessionBody, scratch.path())
        .expect_err("a body that outlives the apply that changed its map breaks the rule");
    assert!(
        broke.contains("answers the body it answered before"),
        "{broke}"
    );
}

#[test]
fn a_world_with_no_edit_store_breaks_the_rule_of_its_capabilities_where_it_states_one_or_a_base() {
    let broken = |title: FakeTitle| {
        let scratch = tempfile::tempdir().expect("a scratch folder");
        let mut fixture = fixture(title);
        fixture.edit = None;
        kit::broken(&fixture, scratch.path())
    };
    let stated = broken(FakeTitle::new());
    assert_eq!(stated.len(), 1, "{stated:#?}");
    assert_eq!(stated[0].0, Rule::Caps);
    assert!(stated[0].1.starts_with("an edit store is stated: true"));

    let based = broken(FakeTitle::broken(Fault::StoreUnstated));
    assert_eq!(based.len(), 1, "{based:#?}");
    assert_eq!(
        based[0],
        (
            Rule::Caps,
            "a base is stated of a world with no edit store".to_owned()
        )
    );
}

#[test]
fn a_map_that_gives_other_placement_rows_than_its_fixture_states_breaks_the_rule_of_its_capabilities(
) {
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let mut fixture = fixture(FakeTitle::new());
    fixture.scene.placed.1 += 1;
    let broken = kit::broken(&fixture, scratch.path());
    assert_eq!(broken.len(), 1, "{broken:#?}");
    assert_eq!(broken[0].0, Rule::Caps);
}
