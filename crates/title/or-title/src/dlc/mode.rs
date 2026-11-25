//! Which of a pack's change sets apply to the map as story and as multiplayer,
//! by the group its `setup2.xml` names each under.

use super::model::{ContentChangeSet, ListItem, SetupData};

/// The group whose change sets run when the pack starts, whatever the mode.
const GROUP_STARTUP: &str = "GROUP_STARTUP";
/// The group of map change sets multiplayer runs.
const GROUP_MAP: &str = "GROUP_MAP";
/// The group of map change sets the story runs.
const GROUP_MAP_SP: &str = "GROUP_MAP_SP";

/// The map as the story has it or as multiplayer has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapMode {
    Story,
    Multiplayer,
}

impl MapMode {
    /// The word a person reads and types for this mode.
    pub fn label(self) -> &'static str {
        match self {
            Self::Story => "story",
            Self::Multiplayer => "multiplayer",
        }
    }

    /// The mode `word` names, by its [`label`](Self::label) in any case.
    pub fn from_label(word: &str) -> Option<Self> {
        [Self::Story, Self::Multiplayer]
            .into_iter()
            .find(|mode| mode.label().eq_ignore_ascii_case(word.trim()))
    }
}

/// When a group's change sets run; `None` for a group whose name states no mode.
fn group_runs_in(group: &str, mode: MapMode) -> Option<bool> {
    let group = group.trim();
    if group.eq_ignore_ascii_case(GROUP_STARTUP) {
        Some(true)
    } else if group.eq_ignore_ascii_case(GROUP_MAP) {
        Some(mode == MapMode::Multiplayer)
    } else if group.eq_ignore_ascii_case(GROUP_MAP_SP) {
        Some(mode == MapMode::Story)
    } else {
        None
    }
}

/// What a mode makes of the change sets of one pack.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeSetsInMode {
    /// Every file the change sets that apply enable, map-scoped ones included.
    pub files_to_enable: Vec<ListItem>,
    /// The change sets every group naming them states for the other mode.
    pub left_out: Vec<String>,
    /// The change sets no group states a mode for, which apply as they do with no mode.
    pub unstated: Vec<String>,
}

impl SetupData {
    /// Which of `change_sets` apply in `mode`, by the groups of this setup that name each; every one applies with no mode.
    pub fn change_sets_in(
        &self,
        change_sets: &[ContentChangeSet],
        mode: Option<MapMode>,
    ) -> ChangeSetsInMode {
        let mut out = ChangeSetsInMode::default();
        for change_set in change_sets {
            let named = |items: &[ListItem]| {
                items
                    .iter()
                    .any(|item| item.trim().eq_ignore_ascii_case(change_set.name.trim()))
            };
            // A setup's own `contentChangeSets` run with `GROUP_STARTUP`.
            let mut runs = mode.is_none() || named(&self.content_change_sets);
            let mut other_mode = false;
            let mut says_nothing = false;
            for group in &self.content_change_set_groups {
                let Some(mode) = mode else { break };
                if !named(&group.content_change_sets) {
                    continue;
                }
                match group_runs_in(&group.name_hash, mode) {
                    Some(true) => runs = true,
                    Some(false) => other_mode = true,
                    None => says_nothing = true,
                }
            }
            // ⚠ Left out only when every group naming it is the other mode's. One no
            // group names, or one a group of another name holds, applies as it does
            // with no mode: the documents do not say when it runs.
            if !runs {
                if other_mode && !says_nothing {
                    out.left_out.push(change_set.name.clone());
                    continue;
                }
                out.unstated.push(change_set.name.clone());
            }
            out.files_to_enable
                .extend(change_set.files_to_enable.iter().cloned());
            for map in &change_set.map_change_set_data {
                out.files_to_enable
                    .extend(map.files_to_enable.iter().cloned());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dlc::ContentChangeSetGroup;

    /// A change set enabling one file named for it.
    fn change_set(name: &str) -> ContentChangeSet {
        ContentChangeSet {
            name: name.into(),
            files_to_enable: vec![format!("{name}.rpf").into()],
            ..Default::default()
        }
    }

    fn setup(groups: &[(&str, &[&str])]) -> SetupData {
        SetupData {
            content_change_set_groups: groups
                .iter()
                .map(|(group, sets)| ContentChangeSetGroup {
                    name_hash: group.to_string(),
                    content_change_sets: sets.iter().map(|s| s.to_string().into()).collect(),
                })
                .collect(),
            ..Default::default()
        }
    }

    const BOTH: [MapMode; 2] = [MapMode::Story, MapMode::Multiplayer];

    #[test]
    fn a_change_set_both_map_groups_name_applies_in_both_modes() {
        let setup = setup(&[("GROUP_MAP", &["SHARED"]), ("GROUP_MAP_SP", &["SHARED"])]);
        for mode in BOTH {
            let verdict = setup.change_sets_in(&[change_set("SHARED")], Some(mode));
            assert_eq!(verdict.files_to_enable, ["SHARED.rpf"], "{mode:?}");
            assert!(verdict.left_out.is_empty() && verdict.unstated.is_empty());
        }
    }

    #[test]
    fn a_setups_own_change_sets_apply_in_both_modes_and_are_not_named_as_unstated() {
        let setup = SetupData {
            content_change_sets: vec!["OWN".to_string().into()],
            ..Default::default()
        };
        for mode in BOTH {
            let verdict = setup.change_sets_in(&[change_set("OWN")], Some(mode));
            assert_eq!(verdict.files_to_enable, ["OWN.rpf"], "{mode:?}");
            assert!(verdict.left_out.is_empty() && verdict.unstated.is_empty());
        }
    }

    #[test]
    fn a_group_and_a_change_set_are_matched_whatever_their_case() {
        let setup = setup(&[("group_map_sp", &["Story_Set"])]);
        let sets = [change_set("STORY_SET")];
        let multiplayer = setup.change_sets_in(&sets, Some(MapMode::Multiplayer));
        assert_eq!(multiplayer.left_out, ["STORY_SET"]);
        assert!(multiplayer.files_to_enable.is_empty());
        let story = setup.change_sets_in(&sets, Some(MapMode::Story));
        assert_eq!(story.files_to_enable, ["STORY_SET.rpf"]);
    }

    /// ⚠ One group states the other mode and one states none: the documents do not settle it, so it applies.
    #[test]
    fn a_change_set_a_group_of_another_name_also_holds_is_applied_and_named() {
        let setup = setup(&[("GROUP_MAP", &["MIXED"]), ("GROUP_ON_DEMAND", &["MIXED"])]);
        let verdict = setup.change_sets_in(&[change_set("MIXED")], Some(MapMode::Story));
        assert_eq!(verdict.files_to_enable, ["MIXED.rpf"]);
        assert_eq!(verdict.unstated, ["MIXED"]);
        assert!(verdict.left_out.is_empty());
    }

    #[test]
    fn a_mode_is_read_from_the_word_it_is_shown_as() {
        for mode in BOTH {
            assert_eq!(MapMode::from_label(mode.label()), Some(mode));
        }
        assert_eq!(MapMode::from_label(" Story "), Some(MapMode::Story));
        assert_eq!(
            MapMode::from_label("MULTIPLAYER"),
            Some(MapMode::Multiplayer)
        );
        assert_eq!(MapMode::from_label("online"), None);
    }
}
