//! The neutral world verbs over the fake title, which names no game: each answers through the
//! contract alone, and a verb whose capability the world lacks is refused with what it lacks.

use or_world::contract::{
    GameId, Keys, MapReader, ModelReader, Opening, WorldError, WorldIndexer, WorldInstall,
    WorldLook, WorldSpec, WorldTitle,
};
use or_world::fake::{
    FakeTitle, FAR_EYE, HELD_EYE, INTERIOR_NODES, LIST_INSTANCES, NEAR_EYE, NIGHT_EYE, NOWHERE,
    WEATHER, YARD_EYE,
};
use or_world::model::Moment;

use super::*;

fn opened(config: &Path) -> Opened {
    open(&FakeTitle::new(), &FakeTitle::spec(), Keys::None, config).expect("the fake world opens")
}

fn at(hour: f32) -> Moment {
    Moment {
        hour,
        weather: WEATHER,
    }
}

/// The lines of `lines` that begin, past their indent, with `word`.
fn starting<'a>(lines: &'a [String], word: &str) -> Vec<&'a str> {
    let lines = lines.iter().map(|line| line.trim_start());
    lines.filter(|line| line.starts_with(word)).collect()
}

#[test]
fn the_index_of_any_title_is_built_kept_and_said_with_its_options() {
    let config = tempfile::tempdir().expect("a scratch folder");
    let (title, world) = (FakeTitle::new(), FakeTitle::spec());
    let first = index_lines(&title, &world, &opened(config.path()));
    assert!(first[0].contains("(fake, fake-world)"), "{first:?}");
    assert!(first[1].ends_with("none was kept, so every archive was read"));
    assert_eq!(first[2], "  2 archive(s), 2 read");
    assert_eq!(
        first[3],
        "  5 map(s): 0 that state no box, 1 only a request lets in, 1 by the clock or the weather"
    );
    assert_eq!(starting(&first, "option").len(), title.options().len());
    let second = index_lines(&title, &world, &opened(config.path()));
    assert!(second[1].ends_with("of the world as it stands, so nothing was read"));
    assert_eq!(second[2], "  2 archive(s), 0 read");
    assert_eq!(first[3..], second[3..]);
}

#[test]
fn what_streams_at_a_place_is_said_with_whether_its_gate_lets_it_in() {
    let config = tempfile::tempdir().expect("a scratch folder");
    let opened = opened(config.path());
    let night = at_lines(&opened, NIGHT_EYE, &at(22.0));
    assert_eq!(
        starting(&night, "map"),
        ["map       night  during the hours 0xf0003f, under any weather (in)"]
    );
    let noon = at_lines(&opened, NIGHT_EYE, &at(12.0));
    assert!(noon[1].ends_with("(held back)"), "{noon:?}");
    let held = at_lines(&opened, HELD_EYE, &at(12.0));
    assert_eq!(
        starting(&held, "map"),
        ["map       held  only when asked for (held back)"]
    );
    assert_eq!(at_lines(&opened, NOWHERE, &at(12.0)).len(), 1);
}

#[test]
fn a_view_is_streamed_and_selected_through_the_driver_for_any_title() {
    let config = tempfile::tempdir().expect("a scratch folder");
    let opened = opened(config.path());
    let far = select_lines(&opened, FAR_EYE, &at(12.0));
    assert_eq!(starting(&far, "instance").len(), 1, "{far:?}");
    let near = select_lines(&opened, NEAR_EYE, &at(12.0));
    assert_eq!(starting(&near, "instance").len(), 2, "{near:?}");
    assert_eq!(
        starting(&near, "map"),
        [
            "map       far  the view is within its box",
            "map       near  the view is within its box"
        ]
    );
    let yard = select_lines(&opened, YARD_EYE, &at(12.0));
    assert_eq!(starting(&yard, "interior").len(), 1, "{yard:?}");
    assert_eq!(
        starting(&yard, "instance").len(),
        INTERIOR_NODES + LIST_INSTANCES,
        "{yard:?}"
    );
    let nowhere = select_lines(&opened, NOWHERE, &at(12.0));
    assert!(nowhere[0].contains(": 0 map(s) wanted"), "{nowhere:?}");
}

#[test]
fn the_ground_and_a_search_are_answered_where_the_world_states_them() {
    let config = tempfile::tempdir().expect("a scratch folder");
    let opened = opened(config.path());
    let ground = ground_lines(&opened, 3.0, 4.0, Some(5.0)).expect("the fake states its ground");
    assert_eq!(
        ground,
        ["Ground under (3, 4): 0 (the surface faces 0, 0, 1)"]
    );
    let found = find_lines(&opened, "YAR").expect("the fake states placement rows");
    assert_eq!(
        found,
        ["`YAR`: 1 map(s)", "  map       yard  0xfa05  places 2"]
    );
    let by_number = find_lines(&opened, "0xFA05").expect("a name as a number");
    assert_eq!(by_number[1], found[1]);
    assert_eq!(
        find_lines(&opened, "no such map"),
        Ok(vec!["`no such map`: 0 map(s)".to_string()])
    );
}

/// A world that states no capability, over the fake's.
struct Bare(Arc<dyn WorldInstall>);

impl WorldInstall for Bare {
    fn caps(&self) -> or_world::caps::WorldCaps {
        or_world::caps::WorldCaps::none()
    }
    fn policy(&self) -> or_world::policy::StreamPolicy {
        self.0.policy()
    }
    fn extras(&self) -> or_world::slots::Extras {
        self.0.extras()
    }
    fn inputs(&self) -> or_world::kept::Inputs {
        self.0.inputs()
    }
    fn indexer(&self) -> &dyn WorldIndexer {
        self.0.indexer()
    }
    fn maps(&self) -> &dyn MapReader {
        self.0.maps()
    }
    fn models(&self) -> &dyn ModelReader {
        self.0.models()
    }
    fn look(&self) -> &dyn WorldLook {
        self.0.look()
    }
}

struct BareTitle(FakeTitle);

impl WorldTitle for BareTitle {
    fn id(&self) -> &'static str {
        "bare"
    }
    fn games(&self) -> &'static [GameId] {
        self.0.games()
    }
    fn options(&self) -> Vec<or_world::slots::OptionKind> {
        Vec::new()
    }
    fn open(
        &self,
        world: &WorldSpec,
        with: &Opening<'_>,
    ) -> Result<Arc<dyn WorldInstall>, WorldError> {
        Ok(Arc::new(Bare(self.0.open(world, with)?)))
    }
}

#[test]
fn a_verb_whose_capability_the_world_lacks_is_refused_with_what_it_lacks() {
    let config = tempfile::tempdir().expect("a scratch folder");
    let title = BareTitle(FakeTitle::new());
    let opened = open(&title, &FakeTitle::spec(), Keys::None, config.path()).expect("it opens");
    assert_eq!(
        ground_lines(&opened, 0.0, 0.0, None),
        Err("this world states no height of the ground".to_string())
    );
    assert_eq!(
        find_lines(&opened, "yard"),
        Err("this world states no placement rows".to_string())
    );
    let said = index_lines(&title, &FakeTitle::spec(), &opened);
    assert!(
        said.contains(&"  it states: no capability".to_string()),
        "{said:?}"
    );
    // What needs no capability is answered all the same.
    assert_eq!(
        starting(&select_lines(&opened, NEAR_EYE, &at(12.0)), "instance").len(),
        2
    );
}

#[test]
fn an_option_is_read_as_a_toggle_a_choice_or_a_name_and_a_moment_from_its_flags() {
    let args = |words: &[&str]| -> Vec<String> { words.iter().map(|w| w.to_string()).collect() };
    let set = options_of(&args(&[
        "--option", "1=on", "x", "--option", "2=7", "--option", "3=a pack",
    ]));
    assert_eq!(
        set,
        Ok(vec![
            (OptionId(1), OptionValue::On(true)),
            (OptionId(2), OptionValue::Choice(7)),
            (OptionId(3), OptionValue::Named("a pack".to_string())),
        ])
    );
    assert!(options_of(&args(&["--option", "level"])).is_err());
    assert!(options_of(&args(&["--option"])).is_err());
    let moment = moment_of(&args(&["--hour", "22", "--weather", "9"])).expect("a moment");
    assert_eq!((moment.hour, moment.weather), (22.0, 9));
    assert!(moment_of(&args(&["--hour", "24"])).is_err());
    assert!(moment_of(&args(&["--weather", "RAIN"])).is_err());
}

#[test]
fn an_install_no_title_opens_a_world_of_and_a_flag_no_verb_knows_are_refused() {
    let args = |words: &[&str]| -> Vec<String> { words.iter().map(|w| w.to_string()).collect() };
    let dir = tempfile::tempdir().expect("a scratch folder");
    let install = dir.path().to_string_lossy().into_owned();
    assert_eq!(
        said("index", &args(&[&install, "--neutral", "--levle", "x"])),
        Err("`--levle` is no flag of this verb".to_string())
    );
    assert_eq!(
        said("index", &args(&[&install, "--neutral", "--keys"])),
        Err("`--keys` takes a value".to_string())
    );
}
