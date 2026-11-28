//! Every world-state group a retail install declares, held to the maps and
//! bounds it switches and to the flag word that says what switches it.
//!
//! Point `OPENRAGE_GAME_ROOT` at one or more install roots, `;`-separated.
//!
//! ⚠ A group's `Bounds` reads like its member maps and names a held map 2
//! times in 2,549; it names a held `.ybn` every time. A group's `Name` is a
//! held map every time, and that map sets `flags` bit 0 every time — the bit
//! that holds a map back until something asks. So a group switches the map
//! it is named for and the collision files it lists.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};
use or_io::container::RpfCryptoProvider;
use or_io::meta::binary::parse_meta_binary_full;
use or_io::meta::Diagnostics;
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};
use or_title::game::Game;
use or_world::map_data::MapData;
use or_world::state::{weather_name, Gate, StateGroups, WorldState, WEATHER_NAMES};
use or_world::Manifest;

const WHAT: &str = "what_every_world_state_group_holds";

#[derive(Default)]
struct Install {
    label: String,
    manifests: usize,
    groups: usize,
    redeclared: u64,
    gates: BTreeMap<&'static str, u64>,
    problems: Vec<String>,
    bounds: u64,
    bounds_are_maps: u64,
    bounds_are_ybn: u64,
    weather: BTreeSet<u32>,
    maps: usize,
    held_back: usize,
    held_back_ungrouped: usize,
    toggles: u64,
    toggles_exact: u64,
    transitions: u64,
    transitions_exact: u64,
}

impl Install {
    fn report(&self) -> String {
        let named = self
            .weather
            .iter()
            .filter(|h| weather_name(**h).is_some())
            .count();
        format!(
            "{}\n  {} manifests declare {} groups ({} restated by a later manifest); gates {:?}\n  \
             {} problems between the groups and the maps\n  \
             `Bounds`: {} entries, {} name a held map, {} name a held .ybn\n  \
             {} distinct weather types, {} named\n  \
             {} maps held, {} set flags bit 0, {} of them in no group\n  \
             {} requests and releases, {} streamed exactly the map and its bounds\n  \
             {} clock and weather transitions, {} deltas equal to two full answers",
            self.label,
            self.manifests,
            self.groups,
            self.redeclared,
            self.gates,
            self.problems.len(),
            self.bounds,
            self.bounds_are_maps,
            self.bounds_are_ybn,
            self.weather.len(),
            named,
            self.maps,
            self.held_back,
            self.held_back_ungrouped,
            self.toggles,
            self.toggles_exact,
            self.transitions,
            self.transitions_exact,
        )
    }
}

fn crypto_for(root: &Path) -> Option<Arc<dyn RpfCryptoProvider>> {
    let exe = Game::detect_from_root(root)?.find_executable(root)?;
    let keys = extract_keys(&exe, &|_| {}).ok()?;
    Some(Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys))))
}

fn measure(root: &Path) -> Option<Install> {
    let provider = crypto_for(root)?;
    let install = or_title::GtaInstall::from_path(root)?;
    let game = or_title::build_mounted_game(&install, provider, or_title::DlcCutoff::All).ok()?;

    let mut manifest_paths = Vec::new();
    let mut winners: BTreeMap<String, String> = BTreeMap::new();
    let mut ybn: BTreeSet<u32> = BTreeSet::new();
    game.for_each_file(|logical, _container, leaf| {
        let lower = leaf.to_ascii_lowercase();
        if lower.ends_with(".ymf") {
            manifest_paths.push(logical.to_string());
        } else if lower.ends_with(".ymap") {
            winners.insert(lower, logical.to_string());
        } else if let Some(stem) = lower.strip_suffix(".ybn") {
            ybn.insert(or_io::string_hash(stem.as_bytes()));
        }
    });

    let library = support::schema();
    let mut out = Install {
        label: root.display().to_string(),
        ..Install::default()
    };

    let mut groups = StateGroups::new();
    let mut declared_bounds: Vec<u32> = Vec::new();
    for path in &manifest_paths {
        let Some(bytes) = game.vfs.open(path) else {
            continue;
        };
        let mut diags = Diagnostics::lenient();
        let Ok((tree, _)) = parse_meta_binary_full(library, &bytes, &mut diags) else {
            continue;
        };
        let Some(manifest) = Manifest::new(&tree) else {
            continue;
        };
        out.manifests += 1;
        for group in manifest.state_groups() {
            declared_bounds.extend(group.bounds());
            out.weather.extend(group.weather_types());
        }
        groups.add_from(manifest, path);
    }
    out.groups = groups.len();
    out.redeclared = groups.redeclared();
    for name in groups.names() {
        let gate = match groups.gate(name) {
            Some(Gate::Scripted) => "scripted",
            Some(Gate::Timed { .. }) => "timed",
            Some(Gate::Weather { .. }) => "weather",
            Some(Gate::TimedWeather { .. }) => "timed+weather",
            None => "none",
        };
        *out.gates.entry(gate).or_default() += 1;
    }

    // A fold: only a map's name and one bit survive its tree.
    let mut map_names: BTreeSet<u32> = BTreeSet::new();
    for path in winners.values() {
        let Some(bytes) = game.vfs.open(path) else {
            continue;
        };
        let mut diags = Diagnostics::lenient();
        let Ok((tree, _)) = parse_meta_binary_full(library, &bytes, &mut diags) else {
            continue;
        };
        let Some(map) = MapData::new(&tree) else {
            continue;
        };
        map_names.insert(map.name());
        groups.add_map(&map);
    }
    out.maps = map_names.len();
    out.problems = groups.check().iter().map(ToString::to_string).collect();

    out.bounds = declared_bounds.len() as u64;
    out.bounds_are_maps = declared_bounds
        .iter()
        .filter(|b| map_names.contains(b))
        .count() as u64;
    out.bounds_are_ybn = declared_bounds.iter().filter(|b| ybn.contains(b)).count() as u64;

    let held: Vec<u32> = groups.held_back().collect();
    out.held_back = held.len();
    out.held_back_ungrouped = held.iter().filter(|m| !groups.contains(**m)).count();

    // The delta's claim on every map a script could ask for: from a state
    // with nothing requested, requesting it streams it and its bounds unless
    // a gate already holds it on, and releasing it undoes exactly that.
    let base = WorldState::at(0, "EXTRASUNNY");
    for map in held.iter().copied() {
        out.toggles += 1;
        let already = groups.is_active(map, &base);
        let bounds: BTreeSet<u32> = groups.bounds(map).iter().copied().collect();
        let mut state = base.clone();
        let on = groups.toggle(&mut state, map, true);
        let off = groups.toggle(&mut state, map, false);
        let exact = if already {
            on.is_empty() && off.is_empty()
        } else {
            on.load == BTreeSet::from([map])
                && on.unload.is_empty()
                && on.bounds_load == bounds
                && off.unload == on.load
                && off.bounds_unload == on.bounds_load
                && off.load.is_empty()
        };
        out.toggles_exact += exact as u64;
    }

    // And on every hour boundary under every weather a group names: the
    // incremental answer has to be the difference of two full recomputations.
    let mut weathers: Vec<u32> = out.weather.iter().copied().collect();
    weathers.extend(
        WEATHER_NAMES
            .iter()
            .map(|w| or_io::string_hash(w.as_bytes())),
    );
    weathers.sort_unstable();
    weathers.dedup();
    for weather in &weathers {
        for hour in 0..24u8 {
            let before = WorldState {
                hour,
                weather: *weather,
                requested: BTreeSet::new(),
            };
            let after = WorldState {
                hour: (hour + 1) % 24,
                ..before.clone()
            };
            let delta = groups.transition(&before, &after);
            let (mut load, mut unload) = (BTreeSet::new(), BTreeSet::new());
            for map in &held {
                match (
                    groups.is_active(*map, &before),
                    groups.is_active(*map, &after),
                ) {
                    (false, true) => {
                        load.insert(*map);
                    }
                    (true, false) => {
                        unload.insert(*map);
                    }
                    _ => {}
                }
            }
            out.transitions += 1;
            out.transitions_exact += (delta.load == load && delta.unload == unload) as u64;
        }
    }
    Some(out)
}

#[test]
fn what_every_world_state_group_holds() {
    let roots = game_roots();
    if roots.is_empty() {
        skip_install(
            WHAT,
            format_args!(
                "no install root; set {GAME_ROOT_ENV} to a ;-separated list of install \
                 directories. Manifests live inside encrypted archives."
            ),
        );
        return;
    }
    let installs: Vec<Install> = roots.iter().filter_map(|r| measure(r)).collect();
    if installs.is_empty() {
        skip_install(WHAT, "no install root mounted");
        return;
    }
    for install in &installs {
        let report = install.report();
        eprintln!("{report}");
        // Floors well under one edition, so an empty mount cannot pass.
        assert!(install.groups >= 100, "{report}");
        assert!(install.bounds >= 1_000, "{report}");
        assert!(install.held_back >= 1_000, "{report}");
        assert!(
            install.problems.is_empty(),
            "{report}\n  {}",
            install.problems.join("\n  ")
        );
        assert_eq!(install.bounds_are_ybn, install.bounds, "{report}");
        assert_eq!(install.toggles_exact, install.toggles, "{report}");
        assert_eq!(install.transitions_exact, install.transitions, "{report}");
    }
}
