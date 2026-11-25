//! The names of GTA VI's map bodies that follow from the name of another map: a map's occlusion
//! and level-of-detail maps, its instance maps by range and detail, and its area's imposter maps.
//!
//! ⚠ A rule only makes candidates. A candidate names a map only where its 64-bit hash is the word
//! a map body states, so a rule can miss and cannot name wrongly short of a 64-bit collision.

use std::collections::{HashMap, HashSet};

use super::hash::name_hash64;
use super::names::Vocabulary;

/// What follows a map's name in the name of a map that goes with it whole: its occlusion and its
/// levels of detail, and the states of a map an event swaps.
const WHOLE: [&str; 6] = ["_occl", "_lod", "_slod", "_before", "_aftermath", "_end"];
/// What stands between a map's name and `_instances`.
const BASES: [&str; 2] = ["", "_placement"];
/// The range of an instance map, after `_instances`.
const RANGES: [&str; 3] = ["", "_med", "_long"];
/// The detail of an instance map, after its range.
const DETAILS: [&str; 3] = ["", "_hd", "_lod"];
/// The range of an area's imposter map, after `_ip_slod_`.
const IMPOSTER_RANGES: [&str; 3] = ["near", "med", "far"];
// ponytail: a numbered run is taken to end at this many numbers in a row that name no map, so a
// run with a wider gap is named only up to the gap; raise it if a census shows such a run.
const GAP: u32 = 4;

/// The start of each numbered run of names that go with the map named `stem`, to which a number
/// from 0 is added.
fn runs(stem: &str) -> Vec<String> {
    let mut runs = Vec::new();
    for base in BASES {
        for range in RANGES {
            for detail in DETAILS {
                runs.push(format!("{stem}{base}_instances{range}{detail}_"));
            }
        }
    }
    // A map `ap_01` or `ap_02b` is of the area `ap`, which has imposter maps of its own.
    let area = stem.rsplit_once('_').filter(|(_, last)| {
        let digits = last.trim_end_matches(|c: char| c.is_ascii_lowercase());
        !digits.is_empty()
            && last.len() - digits.len() <= 1
            && digits.bytes().all(|b| b.is_ascii_digit())
    });
    if let Some((area, _)) = area {
        for range in IMPOSTER_RANGES {
            runs.push(format!("{area}_ip_slod_{range}_"));
        }
    }
    runs
}

/// The maps of `wanted`, by their 64-bit words, that the rules name from the map names `stems`,
/// a name the rules give being a stem in its turn.
pub fn name_maps<'a>(
    stems: impl IntoIterator<Item = &'a str>,
    wanted: &HashSet<u64>,
) -> HashMap<u64, String> {
    let mut named: HashMap<u64, String> = HashMap::new();
    let mut stems: Vec<String> = stems.into_iter().map(str::to_ascii_lowercase).collect();
    let mut seen: HashSet<String> = stems.iter().cloned().collect();
    while let Some(stem) = stems.pop() {
        for suffix in WHOLE {
            let name = format!("{stem}{suffix}");
            let word = name_hash64(name.as_bytes());
            if wanted.contains(&word) && seen.insert(name.clone()) {
                named.insert(word, name.clone());
                stems.push(name);
            }
        }
        for run in runs(&stem) {
            let mut missed = 0;
            let mut number = 0u32;
            while missed < GAP {
                let name = format!("{run}{number}");
                let word = name_hash64(name.as_bytes());
                match wanted.contains(&word) {
                    true => {
                        named.insert(word, name);
                        missed = 0;
                    }
                    false => missed += 1,
                }
                number += 1;
            }
        }
    }
    named
}

/// Holds in `vocabulary` the name the rules give each map of `words`, the words map bodies state
/// of their names, that it names none of, from the maps it names; how many names were new.
pub fn hold_map_names(vocabulary: &mut Vocabulary, words: &HashSet<u64>) -> usize {
    let stems: Vec<String> = (words.iter())
        .filter_map(|word| vocabulary.name64(*word).map(str::to_string))
        .collect();
    let wanted: HashSet<u64> = (words.iter().copied())
        .filter(|word| vocabulary.name64(*word).is_none())
        .collect();
    let named = name_maps(stems.iter().map(String::as_str), &wanted);
    named
        .values()
        .filter(|name| vocabulary.add(name, None))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(names: &[&str]) -> HashSet<u64> {
        names.iter().map(|n| name_hash64(n.as_bytes())).collect()
    }

    #[test]
    fn a_maps_name_names_the_maps_that_go_with_it_and_no_other() {
        let made_up = [
            "zq_07_occl",
            "zq_07_lod",
            "zq_07_lod_instances_0",
            "zq_07_instances_0",
            "zq_07_instances_1",
            "zq_07_instances_long_hd_0",
            "zq_07_placement_instances_med_lod_2",
            "zq_ip_slod_far_0",
            // Five numbers past the last one found: beyond the gap a run is followed over.
            "zq_07_instances_6",
            // Of another map, which no stem names.
            "zr_01_instances_0",
        ];
        let named = name_maps(["ZQ_07"], &words(&made_up));
        let mut got: Vec<&str> = named.values().map(String::as_str).collect();
        got.sort_unstable();
        assert_eq!(
            got,
            [
                "zq_07_instances_0",
                "zq_07_instances_1",
                "zq_07_instances_long_hd_0",
                "zq_07_lod",
                "zq_07_lod_instances_0",
                "zq_07_occl",
                "zq_07_placement_instances_med_lod_2",
                "zq_ip_slod_far_0",
            ]
        );
        for (word, name) in &named {
            assert_eq!(*word, name_hash64(name.as_bytes()));
        }
    }

    #[test]
    fn a_vocabulary_holds_the_names_the_rules_give_the_maps_a_harvest_saw() {
        let mut vocabulary = Vocabulary::default();
        vocabulary.add("zq_07", None);
        vocabulary.add("zq_08_ec_fair", None);
        let saw = [
            "zq_07",
            "zq_07_instances_0",
            "zq_08_ec_fair",
            // The states of a map an event swaps, and the levels of detail of one of them.
            "zq_08_ec_fair_before",
            "zq_08_ec_fair_aftermath",
            "zq_08_ec_fair_aftermath_lod",
            "zq_08_ec_fair_end",
            // A map no rule reaches from a name held.
            "zq_09_other",
        ];
        assert_eq!(hold_map_names(&mut vocabulary, &words(&saw)), 5);
        let held = |v: &Vocabulary, name: &str| v.name64(name_hash64(name.as_bytes())).is_some();
        assert!(saw[..7].iter().all(|name| held(&vocabulary, name)));
        assert!(!held(&vocabulary, "zq_09_other"));
        // A second harvest of the same maps adds none, and no name of a map no body states.
        assert_eq!(hold_map_names(&mut vocabulary, &words(&saw)), 0);
        assert!(!held(&vocabulary, "zq_07_occl") && !held(&vocabulary, "zq_07_instances_1"));
    }

    #[test]
    fn an_area_is_a_stem_without_its_number_and_a_name_with_none_has_no_area() {
        let has = |stem: &str, run: &str| runs(stem).iter().any(|r| r == run);
        assert!(has("zq_02b", "zq_ip_slod_med_"));
        assert!(has("zq_lel_14", "zq_lel_ip_slod_near_"));
        assert!(!runs("zq_metadata").iter().any(|r| r.contains("_ip_slod_")));
        assert!(!runs("zq").iter().any(|r| r.contains("_ip_slod_")));
        // The stems reversed, as a census's control spells them, give other names.
        let named = name_maps(["70_qz"], &words(&["zq_07_instances_0", "zq_07_occl"]));
        assert!(named.is_empty());
    }
}
