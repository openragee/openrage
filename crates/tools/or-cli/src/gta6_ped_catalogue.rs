//! GTA VI's ped catalogues on the command line: `rpf scan-open` on one prints its kits, their
//! states and the assets each names, by the names kept where they hold one. Nothing is written.

use std::collections::HashSet;

use or_io::gta6::names::Vocabulary;
use or_io::gta6::ped_catalogue::{self, PedCatalogue};
use or_io::gta6::texture_names::{self, Named};

/// The kits of each kind `scan-open` lists of a catalogue before it counts the rest.
const ROWS: usize = 8;

/// Print the ped catalogue a headerless resource's sections hold, or say why its reader refuses it.
pub(crate) fn print_body(virtual_section: &[u8], physical: &[u8]) -> Result<(), String> {
    if !ped_catalogue::is_ped_catalogue(virtual_section) {
        return Ok(());
    }
    let body = ped_catalogue::open(virtual_section, physical)
        .map_err(|e| format!("GTA VI's ped-catalogue reader refuses this body: {e}"))?;
    let vocabulary = super::gta6_names::vocabulary();
    (lines(&body, vocabulary, ROWS).iter()).for_each(|l| println!("{l}"));
    Ok(())
}

fn said32(vocabulary: &Vocabulary, hash: u32) -> String {
    match (hash, vocabulary.name32(hash)) {
        (0, _) => "none".to_string(),
        (_, Some(name)) => name.to_string(),
        (_, None) => format!("hash_{hash:08X}"),
    }
}

/// A 64-bit name word by the name kept for it, else by the one the texture naming rules give it
/// from a drawable of the catalogue, else by its digits.
fn said64(vocabulary: &Vocabulary, ruled: &Named, word: u64) -> String {
    match (word, vocabulary.name64(word), ruled.get(&word)) {
        (0, ..) => "none".to_string(),
        (_, Some(name), _) => name.to_string(),
        (_, None, Some((name, rule))) => format!("{name} (by rule {})", rule.label()),
        (_, None, None) => format!("hash_{word:016X}"),
    }
}

/// What a catalogue states, with at most `rows` kits of each kind listed.
fn lines(body: &PedCatalogue, vocabulary: &Vocabulary, rows: usize) -> Vec<String> {
    let states = || body.asset_kits().flat_map(|kit| &kit.states);
    let members: usize = body.set_kits().map(|kit| kit.members.len()).sum();
    let words: Vec<u64> = states()
        .flat_map(|s| [s.drawable, s.albedo, s.normal, s.material])
        .filter(|word| *word != 0)
        .collect();
    let named = words.iter().filter(|w| vocabulary.name64(**w).is_some());
    // A ped's maps are named as a model's textures are: by the rules, from its drawable's name.
    let unnamed: HashSet<u64> = (words.iter().copied())
        .filter(|word| vocabulary.name64(*word).is_none())
        .collect();
    let drawables = states().filter_map(|s| vocabulary.name64(s.drawable));
    let ruled = texture_names::name_keys(drawables, &unnamed);
    let by_rule = words.iter().filter(|w| ruled.contains_key(w)).count();
    let mut out = vec![
        format!(
            "  ped catalogue {} list(s) of {} asset kits with {} states; {} list(s) of {} set kits with {} members, {} of them a kit and a state of this catalogue",
            body.asset_lists.len(),
            body.asset_kits().count(),
            states().count(),
            body.set_lists.len(),
            body.set_kits().count(),
            members,
            body.members_resolved()
        ),
        format!(
            "  assets        {} drawable, albedo, normal and material words stated, {} of them named by the names kept and {by_rule} more by the texture naming rules from a drawable's name",
            words.len(),
            named.count()
        ),
    ];
    for (k, kit) in body.asset_kits().enumerate().take(rows) {
        out.push(format!(
            "    asset kit {k}  {}, category {}, template {}, {} states",
            said32(vocabulary, kit.id),
            said32(vocabulary, kit.category),
            said32(vocabulary, kit.template_name),
            kit.states.len()
        ));
        for state in kit.states.iter().take(rows) {
            out.push(format!(
                "      state {}: drawable {}, albedo {}, normal {}, material {}; {} more assets",
                said32(vocabulary, state.name),
                said64(vocabulary, &ruled, state.drawable),
                said64(vocabulary, &ruled, state.albedo),
                said64(vocabulary, &ruled, state.normal),
                said64(vocabulary, &ruled, state.material),
                state.more_assets
            ));
        }
    }
    for (k, kit) in body.set_kits().enumerate().take(rows) {
        let members: Vec<String> = (kit.members.iter().take(rows))
            .map(|m| {
                format!(
                    "{}/{}",
                    said32(vocabulary, m.kit),
                    said32(vocabulary, m.state)
                )
            })
            .collect();
        out.push(format!(
            "    set kit {k}  {}, scale {}, {} members: {}",
            said32(vocabulary, kit.id),
            kit.scale,
            kit.members.len(),
            members.join(", ")
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::gta6::hash::{name_hash32, name_hash64};
    use or_io::gta6::ped_catalogue::{AssetKit, AssetState, SetKit, SetMember};

    #[test]
    fn a_catalogue_prints_its_counts_and_names_what_the_vocabulary_holds() {
        let mut vocabulary = Vocabulary::default();
        vocabulary.add("zq_hat_000", None);
        vocabulary.add("base", None);
        let state = AssetState {
            name: name_hash32(b"base"),
            drawable: name_hash64(b"zq_hat_000"),
            albedo: 0x55,
            normal: 0,
            material: 0,
            more_assets: 2,
        };
        let body = PedCatalogue {
            asset_lists: vec![vec![AssetKit {
                id: 0xA1,
                template_name: 0,
                category: 0xA3,
                states: vec![state],
            }]],
            set_lists: vec![vec![SetKit {
                id: 0xC1,
                template_name: 0,
                scale: 1.0,
                members: vec![SetMember {
                    kit: 0xA1,
                    state: state.name,
                }],
            }]],
        };
        let said = lines(&body, &vocabulary, 8);
        assert!(
            said[0].contains("1 list(s) of 1 asset kits with 1 states")
                && said[0].contains("1 set kits with 1 members, 1 of them"),
            "{}",
            said[0]
        );
        assert!(said[1].contains("2 drawable") && said[1].contains(", 1 of them named"));
        assert!(
            said[1].contains("and 0 more by the texture naming rules"),
            "{}",
            said[1]
        );
        assert_eq!(
            said[3],
            "      state base: drawable zq_hat_000, albedo hash_0000000000000055, normal none, material none; 2 more assets"
        );
        assert_eq!(
            said[4],
            "    set kit 0  hash_000000C1, scale 1, 1 members: hash_000000A1/base"
        );

        // A map whose name a rule makes from the drawable's is named by that rule.
        let mut ruled_body = body.clone();
        ruled_body.asset_lists[0][0].states[0].albedo = name_hash64(b"zq_hat_000_ab");
        let said = lines(&ruled_body, &vocabulary, 8);
        assert!(
            said[1].contains("and 1 more by the texture naming rules"),
            "{}",
            said[1]
        );
        assert!(
            said[3].contains("albedo zq_hat_000_ab (by rule"),
            "{}",
            said[3]
        );
    }
}
