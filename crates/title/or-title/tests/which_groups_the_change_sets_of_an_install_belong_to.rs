//! Which group each change set of a mounted install belongs to, as the setup document of its pack states it, for every pack and for
//! the title update, and what the three group names the mode rule knows cover of them (`SetupData::change_sets_in`). It bounds the
//! rule that takes the map as story or as multiplayer: the rule is applied to every pack, and what it leaves out is counted.
//!
//! Point `OPENRAGE_GAME_ROOT` at an install root. Without one this skips, which is not a pass.
//!
//! ⚠ The install is opened read-only and nothing here writes into one. It prints counts and the names of groups, which are words of
//! the format, and never the name of a pack or of a change set.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider, RpfCryptoProvider};
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::dlc::{ContentChangeSet, ContentXml, ListItem, MapMode, SetupData};
use or_title::{build_mounted_game, DlcCutoff, Game, GtaInstall, MountedGame};

const WHAT: &str = "which_groups_the_change_sets_of_an_install_belong_to";

/// The three group names the mode rule knows.
const STARTUP: &str = "GROUP_STARTUP";
const MAP: &str = "GROUP_MAP";
const MAP_SP: &str = "GROUP_MAP_SP";

/// Every group name a setup of the installs measured states: the two of the three that are stated, and nine that say no mode.
const STATED: [&str; 11] = [
    "GROUP_EARLY_ON",
    MAP,
    "GROUP_ON_DEMAND",
    "GROUP_POST_DLC_PATCH",
    STARTUP,
    "GROUP_TITLEUPDATE_STARTUP",
    "GROUP_UPDATE_DLC_METADATA",
    "GROUP_UPDATE_DLC_PATCH",
    "GROUP_UPDATE_STREAMING",
    "GROUP_UPDATE_TEXT",
    "GROUP_UPDATE_WEAPON_PATCH",
];

/// The name the mount gives the title update, whose own setup document it does not read.
const TITLE_UPDATE: &str = "TitleUpdate";

fn mount(root: &Path) -> Option<MountedGame> {
    let exe = Game::detect_from_root(root).find_executable(root)?;
    let keys = extract_keys(&exe, &|_| {}).ok()?;
    let crypto: Arc<dyn RpfCryptoProvider> =
        Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys)));
    build_mounted_game(&GtaInstall::from_path(root)?, crypto, DlcCutoff::All).ok()
}

fn bump<K: Ord>(counts: &mut BTreeMap<K, usize>, key: K) {
    *counts.entry(key).or_default() += 1;
}

fn names(items: &[ListItem], change_set: &ContentChangeSet) -> bool {
    (items.iter()).any(|item| item.trim().eq_ignore_ascii_case(change_set.name.trim()))
}

/// The change sets of a pack's content document and of the documents it includes, none when the document does not read.
fn change_sets_of(game: &MountedGame, device: &str, dat: &str) -> Option<Vec<ContentChangeSet>> {
    let dat = if dat.is_empty() { "content.xml" } else { dat };
    let content = ContentXml::parse(&game.vfs.open(&format!("{device}:/{dat}"))?).ok()?;
    let mut sets = content.content_change_sets.clone();
    let included = (content.included_data_files.iter())
        .map(|item| item.value.clone())
        .chain(content.included_xml_files.iter().cloned());
    for path in included {
        if let Some(inner) = (game.vfs.open(&path)).and_then(|b| ContentXml::parse(&b).ok()) {
            sets.extend(inner.content_change_sets);
        }
    }
    Some(sets)
}

#[derive(Default)]
struct Figures {
    packs: usize,
    unread: usize,
    change_sets: usize,
    /// For each group name: the setups that state it, the setups that name a change set under it, and the change sets it names.
    groups: BTreeMap<String, (usize, usize, usize)>,
    /// The change sets by the groups that name them, each with whether it holds the list of a map.
    by_groups: BTreeMap<(String, bool), usize>,
    /// The setups that name a change set outside every group, and the change sets so named.
    setups_with_their_own: usize,
    named_by_the_setup_alone: usize,
    /// The names a group lists that are no change set of the pack's content.
    dangling: usize,
    /// What the rule makes of every change set, as the mount applies it.
    left_out_of_story: usize,
    left_out_of_multiplayer: usize,
    unstated: usize,
    /// The packs that name a change set under the multiplayer map group: by the type their setup states, and how many are named as multiplayer packs.
    map_packs: BTreeMap<String, usize>,
    map_packs_named_mp: usize,
    /// The change sets that state conditions, by the groups that name them.
    conditional: BTreeMap<String, usize>,
}

fn measure(game: &MountedGame) -> Figures {
    let mut figures = Figures::default();
    // ⚠ Each device once: a mount can list one pack twice, and its change sets are then counted twice.
    let mut counted: BTreeSet<String> = BTreeSet::new();
    for pack in &game.packs {
        if !counted.insert(pack.device_name.to_ascii_lowercase()) {
            continue;
        }
        let title_update = pack.name_hash == TITLE_UPDATE;
        // ⚠ The mount gives the title update a setup of its own making; the document is read here.
        let setup = match title_update {
            true => (game.vfs.open("update:/setup2.xml"))
                .and_then(|bytes| SetupData::parse(&bytes).ok())
                .unwrap_or_else(|| pack.setup.clone()),
            false => pack.setup.clone(),
        };
        figures.packs += 1;
        let Some(sets) = change_sets_of(game, &pack.device_name, &setup.dat_file) else {
            figures.unread += 1;
            continue;
        };
        figures.change_sets += sets.len();
        let held: BTreeSet<String> = (sets.iter())
            .map(|set| set.name.trim().to_ascii_uppercase())
            .collect();
        for group in &setup.content_change_set_groups {
            let slot = (figures.groups)
                .entry(group.name_hash.trim().to_ascii_uppercase())
                .or_default();
            slot.0 += 1;
            slot.1 += usize::from(!group.content_change_sets.is_empty());
            slot.2 += group.content_change_sets.len();
            figures.dangling += (group.content_change_sets.iter())
                .filter(|item| !held.contains(&item.trim().to_ascii_uppercase()))
                .count();
        }
        figures.setups_with_their_own += usize::from(!setup.content_change_sets.is_empty());
        for set in &sets {
            let mut groups: Vec<String> = (setup.content_change_set_groups.iter())
                .filter(|group| names(&group.content_change_sets, set))
                .map(|group| group.name_hash.trim().to_ascii_uppercase())
                .collect();
            groups.sort();
            if groups.is_empty() && names(&setup.content_change_sets, set) {
                figures.named_by_the_setup_alone += 1;
            }
            let spelled = match groups.is_empty() {
                true => "no group".to_string(),
                false => groups.join(" and "),
            };
            if set.execution_conditions.is_some() {
                bump(&mut figures.conditional, spelled.clone());
            }
            let of_a_map = !set.map_change_set_data.is_empty();
            bump(&mut figures.by_groups, (spelled, of_a_map));
        }
        let story = setup.change_sets_in(&sets, Some(MapMode::Story));
        let multiplayer = setup.change_sets_in(&sets, Some(MapMode::Multiplayer));
        figures.left_out_of_story += story.left_out.len();
        figures.left_out_of_multiplayer += multiplayer.left_out.len();
        assert_eq!(
            story.unstated, multiplayer.unstated,
            "what no group states a mode for is the same in both"
        );
        figures.unstated += story.unstated.len();
        let names_a_map_set = (setup.content_change_set_groups.iter()).any(|g| {
            g.name_hash.trim().eq_ignore_ascii_case(MAP) && !g.content_change_sets.is_empty()
        });
        if names_a_map_set {
            bump(&mut figures.map_packs, setup.type_name().to_string());
            figures.map_packs_named_mp +=
                usize::from(pack.name_hash.to_ascii_lowercase().starts_with("mp"));
        }
    }
    figures
}

fn report(f: &Figures) {
    println!(
        "{} packs with the title update, {} whose content document did not read; {} change sets",
        f.packs, f.unread, f.change_sets
    );
    println!("the groups the setup documents state (setups stating it, setups naming a change set under it, change sets it names):");
    for (group, (stating, naming, sets)) in &f.groups {
        println!("  {stating:4} {naming:4} {sets:5}  {group}");
    }
    println!("the change sets by the groups that name them:");
    for ((groups, of_a_map), n) in &f.by_groups {
        let list = match of_a_map {
            true => "with the list of a map",
            false => "with no list of a map",
        };
        println!("  {n:5}  {groups}, {list}");
    }
    println!("  of them state conditions: {:?}", f.conditional);
    println!(
        "{} setups name a change set outside every group, {} change sets are named that way and by no group; {} names a group lists are no change set of the content",
        f.setups_with_their_own, f.named_by_the_setup_alone, f.dangling
    );
    println!(
        "the rule leaves {} change sets out of the story and {} out of multiplayer, and names {} as stated for no mode",
        f.left_out_of_story, f.left_out_of_multiplayer, f.unstated
    );
    println!(
        "{} packs name a change set under {MAP}: by the type of their setup {:?}; the name of {} starts with mp",
        f.map_packs.values().sum::<usize>(),
        f.map_packs,
        f.map_packs_named_mp
    );
}

#[test]
fn no_pack_states_the_group_of_the_story_and_the_group_of_multiplayer_is_stated_by_packs_alone() {
    let Some(root) = game_roots().into_iter().next() else {
        skip_install(WHAT, format!("{GAME_ROOT_ENV} names no install"));
        return;
    };
    let game = mount(&root).expect("the install mounts");
    let figures = measure(&game);
    report(&figures);
    let f = &figures;
    let named_under = |group: &str| f.groups.get(group).map_or(0, |(_, _, sets)| *sets);
    let alone = |group: &str, of_a_map: bool| {
        (f.by_groups.get(&(group.to_string(), of_a_map)).copied()).unwrap_or(0)
    };

    // Enough was read for the figures to mean something.
    assert!(f.packs >= 20 && f.unread == 0 && f.change_sets >= 100);
    assert!(named_under(STARTUP) > 0 && named_under(MAP) > 0);
    // ⚠ The story's group is stated by no setup of the install, so nothing shipped shows the rule's story branch at work. When a
    // pack states it, this fails, and the rule has a document to be held to.
    assert!(
        !f.groups.contains_key(MAP_SP),
        "a setup states {MAP_SP}: {:?}",
        f.groups[MAP_SP]
    );
    // The names are the ones the rule's own test is written over: a name stated here and not there is one nothing holds the rule to.
    let stated: Vec<&str> = f.groups.keys().map(String::as_str).collect();
    assert_eq!(stated, STATED);
    // What the rule then does on the install: the story loses exactly what the multiplayer map group names alone, and multiplayer nothing.
    assert_eq!(f.left_out_of_story, alone(MAP, true) + alone(MAP, false));
    assert_eq!(f.left_out_of_multiplayer, 0);
    // No change set is in two groups, so no mode is ever a choice between two statements.
    assert!(
        f.by_groups
            .keys()
            .all(|(groups, _)| !groups.contains(" and ")),
        "a change set is named by two groups"
    );
    // Every list of a map is under the multiplayer map group or under no group.
    for (groups, of_a_map) in f.by_groups.keys() {
        assert!(
            !of_a_map || groups == MAP || groups == "no group",
            "a change set under {groups} holds the list of a map"
        );
    }
    // A setup names its change sets under groups and never outside them.
    assert_eq!(f.setups_with_their_own, 0);
    // What the three names cover is a part: the rest is applied in both modes and named.
    let stated = named_under(STARTUP) + named_under(MAP) + named_under(MAP_SP);
    assert!(f.unstated > 0 && stated < f.change_sets + f.dangling);
}
