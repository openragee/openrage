//! The names of GTA VI's map bodies that follow from the name of another map: a map's occlusion
//! and level-of-detail maps, its instance maps by range and detail, and its area's imposter maps.
//!
//! ⚠ A rule only makes candidates. A candidate names a map only where its 64-bit hash is the word
//! a map body states, so a rule can miss and cannot name wrongly short of a 64-bit collision.

use std::collections::{HashMap, HashSet};

use super::hash::name_hash64;

/// What follows a map's name in the name of a map that goes with it whole.
const WHOLE: [&str; 3] = ["_occl", "_lod", "_slod"];
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
