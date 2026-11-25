use super::*;

#[test]
fn every_pass_has_one_place_and_one_name() {
    for (n, pass) in Pass::ALL.into_iter().enumerate() {
        // The place in a set is the number, so `ALL` must be in the order of the variants.
        assert_eq!(pass as usize, n, "{pass:?}");
        assert_eq!(Pass::from_name(pass.name()), Some(pass));
    }
    let mut names: Vec<&str> = Pass::ALL.into_iter().map(Pass::name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), Pass::ALL.len());
    assert_eq!(Pass::from_name("no-such-pass"), None);
    for level in Level::ALL {
        assert_eq!(Level::from_name(level.name()), Some(level));
    }
}

#[test]
fn a_pass_is_switched_off_by_name_and_the_others_stay() {
    let all = Options::default();
    assert_eq!(all.level, Level::Source);
    assert!(Pass::ALL.into_iter().all(|pass| all.on(pass)));

    let named = Pass::from_name("foreach").expect("a pass of that name");
    let without = all.without(named);
    assert!(!without.on(Pass::Foreach));
    assert_eq!(without.passes.iter().count(), Pass::ALL.len() - 1);
    assert!(without.any(&[Pass::Foreach, Pass::Using]));
    assert!(!without
        .without(Pass::Using)
        .any(&[Pass::Foreach, Pass::Using]));
    assert_eq!(without.with(Pass::Foreach), all);
}

#[test]
fn a_level_runs_its_own_passes_and_those_below() {
    assert!(Options::at(Level::Lifted).passes.is_empty());

    let structured = Options::at(Level::Structured);
    assert!(structured.on(Pass::Values) && structured.on(Pass::RoleNames));
    // No source construct is rebuilt below `Source`.
    assert!(!structured.on(Pass::Fields) && !structured.on(Pass::Tuples));

    let baseline = Options::baseline();
    assert_eq!(baseline.level, Level::Structured);
    assert_eq!(
        baseline.passes.iter().collect::<Vec<_>>(),
        Pass::BASELINE.to_vec()
    );
    assert!(!baseline.on(Pass::Forward));
}
