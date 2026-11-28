//! What a dependency resolver can and cannot derive, per format, as a table.
//!
//! Every extension [`crate::Kind`] classifies must carry a ruling, and a
//! [`Derivable::No`] its reason; the suite fails otherwise.

use or_world_gta5::Kind;

/// Where a reference is written down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stated {
    /// A field of the file itself.
    InTheFile(&'static str),
    /// Another file in the install states it about this one.
    Elsewhere(&'static str),
    /// Nothing states it; a filename says it.
    ByFilename,
}

/// Whether a resolver can produce the edge, and what it needs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Derivable {
    /// The file names it and the install holds the answer.
    Yes,
    /// Derivable, but only with the whole install in hand.
    NeedsTheInstall,
    /// Applied as a convention rather than read, so it can be wrong in a way
    /// nothing in the bytes contradicts.
    ByConvention,
    /// Not derivable, with the reason.
    No(&'static str),
}

/// One outbound reference a format can carry.
#[derive(Debug, Clone, Copy)]
pub struct Rule {
    pub from: Kind,
    pub to: Kind,
    pub what: &'static str,
    pub stated: Stated,
    pub derivable: Derivable,
}

/// The ruling, per format.
///
/// ⚠ A drawable names the textures it wants and never where they come from;
/// only a `.ytyp` that places it says which dictionary supplies them, so a
/// resolver rooted at a loose file has a ceiling no amount of parsing raises.
pub const RULES: &[Rule] = &[
    Rule {
        from: Kind::MapData,
        to: Kind::MapTypes,
        what: "the type files declaring the archetypes it places",
        stated: Stated::Elsewhere("_manifest.ymf imapDependencies_2"),
        derivable: Derivable::NeedsTheInstall,
    },
    Rule {
        from: Kind::MapData,
        to: Kind::Bounds,
        what: "static bounds, for a map that is an imap group",
        stated: Stated::Elsewhere("_manifest.ymf MapDataGroups"),
        derivable: Derivable::NeedsTheInstall,
    },
    Rule {
        from: Kind::MapData,
        to: Kind::Bounds,
        what: "static bounds, for a map no imap group names: the `<map>_N`, `hi@<map>_N` \
               and `ma@<map>_N` `.ybn`, flagged where one leaves the map's streaming box",
        stated: Stated::ByFilename,
        derivable: Derivable::ByConvention,
    },
    Rule {
        from: Kind::MapData,
        to: Kind::MapData,
        what: "the parent map its LOD chain continues into",
        stated: Stated::InTheFile("parent"),
        derivable: Derivable::Yes,
    },
    Rule {
        from: Kind::MapTypes,
        to: Kind::Drawable,
        what: "assetName, when assetType is a drawable",
        stated: Stated::InTheFile("assetName"),
        derivable: Derivable::Yes,
    },
    Rule {
        from: Kind::MapTypes,
        to: Kind::Fragment,
        what: "assetName, when assetType is a fragment",
        stated: Stated::InTheFile("assetName"),
        derivable: Derivable::Yes,
    },
    Rule {
        from: Kind::MapTypes,
        to: Kind::Bounds,
        what: "assetName, when the install holds it only as a `.ybn`: a collision-only, \
               assetless archetype",
        stated: Stated::InTheFile("assetName"),
        derivable: Derivable::Yes,
    },
    Rule {
        from: Kind::MapTypes,
        to: Kind::DrawableDict,
        what: "drawableDictionary, which holds assetName as an entry",
        stated: Stated::InTheFile("drawableDictionary"),
        derivable: Derivable::Yes,
    },
    Rule {
        from: Kind::MapTypes,
        to: Kind::TextureDict,
        what: "textureDictionary, the only statement of where a model's textures live",
        stated: Stated::InTheFile("textureDictionary"),
        derivable: Derivable::Yes,
    },
    Rule {
        from: Kind::MapTypes,
        to: Kind::Bounds,
        what: "physicsDictionary: a `.ybn` or `.ybd` of that name where one exists, else \
               the bound the archetype's own `.ydr` carries, which is where nearly every \
               one points, else the `.ybn` that is the archetype's asset",
        stated: Stated::InTheFile("physicsDictionary"),
        derivable: Derivable::Yes,
    },
    Rule {
        from: Kind::MapTypes,
        to: Kind::Bounds,
        what: "an interior archetype's collision, which no field of the archetype states",
        stated: Stated::Elsewhere("_manifest.ymf Interiors"),
        derivable: Derivable::NeedsTheInstall,
    },
    Rule {
        from: Kind::MapTypes,
        to: Kind::ClipDict,
        what: "clipDictionary",
        stated: Stated::InTheFile("clipDictionary"),
        derivable: Derivable::Yes,
    },
    Rule {
        from: Kind::TextureDict,
        to: Kind::TextureDict,
        what: "the parent dictionary a lookup falls through to",
        stated: Stated::Elsewhere("gtxd.meta txdRelationships"),
        derivable: Derivable::NeedsTheInstall,
    },
    Rule {
        from: Kind::Drawable,
        to: Kind::TextureDict,
        what: "which dictionary supplies a shader's texture",
        stated: Stated::Elsewhere("the .ytyp that places it"),
        derivable: Derivable::No(
            "a drawable names the texture it wants and never where it comes from, \
             so a resolver rooted at the file cannot answer and one rooted at an \
             archetype can",
        ),
    },
    Rule {
        from: Kind::Fragment,
        to: Kind::TextureDict,
        what: "which dictionary supplies a shader's texture",
        stated: Stated::Elsewhere("the .ytyp that places it"),
        derivable: Derivable::No("as for a drawable: the name is in the file, the source is not"),
    },
    Rule {
        from: Kind::Fragment,
        to: Kind::Fragment,
        what: "the `_hi` companion holding the high-detail LODs",
        stated: Stated::ByFilename,
        derivable: Derivable::ByConvention,
    },
    Rule {
        from: Kind::TextureDict,
        to: Kind::TextureDict,
        what: "the `+hi` companion holding the same texture names at higher resolution",
        stated: Stated::ByFilename,
        derivable: Derivable::ByConvention,
    },
    Rule {
        from: Kind::Fragment,
        to: Kind::ClipDict,
        what: "a skeleton to animate it",
        stated: Stated::Elsewhere("the .ytyp that places it"),
        derivable: Derivable::No(
            "a fragment carries its own crSkeletonData, so the skeleton is not an \
             outbound edge at all; what is outbound is the clip dictionary, and only \
             an archetype names one",
        ),
    },
    Rule {
        from: Kind::Bounds,
        to: Kind::Other,
        what: "the surface material each polygon uses",
        stated: Stated::Elsewhere("materials.dat"),
        derivable: Derivable::No(
            "a bound stores a material index into an install-global palette, not a \
             file reference, so there is no per-file edge to report",
        ),
    },
    Rule {
        from: Kind::BoundsDict,
        to: Kind::Other,
        what: "the surface material each polygon uses",
        stated: Stated::Elsewhere("materials.dat"),
        derivable: Derivable::No(
            "a bounds dictionary is a container of bounds and inherits their ceiling: \
             an index into the material palette is not a reference to a file",
        ),
    },
    Rule {
        from: Kind::DrawableDict,
        to: Kind::TextureDict,
        what: "which dictionary supplies an entry's textures",
        stated: Stated::Elsewhere("the .ytyp that places the entry"),
        derivable: Derivable::No(
            "each entry is a drawable and carries a drawable's ceiling, and the \
             dictionary adds one of its own: its entries are placed by different \
             archetypes, so there is no one answer for the file",
        ),
    },
    Rule {
        from: Kind::DrawableDict,
        to: Kind::DrawableDict,
        what: "the `_hi` companion",
        stated: Stated::ByFilename,
        derivable: Derivable::ByConvention,
    },
    Rule {
        from: Kind::ClipDict,
        to: Kind::Fragment,
        what: "the skeleton its channels animate",
        stated: Stated::Elsewhere("the .ytyp that names this clip dictionary"),
        derivable: Derivable::No(
            "a clip addresses bones by tag rather than by skeleton, so a dictionary \
             fits every skeleton carrying those tags and names none of them; the edge \
             runs the other way, from an archetype's clipDictionary",
        ),
    },
    Rule {
        from: Kind::Manifest,
        to: Kind::MapTypes,
        what: "the type files it states each map needs",
        stated: Stated::InTheFile("imapDependencies_2"),
        derivable: Derivable::Yes,
    },
    Rule {
        from: Kind::Manifest,
        to: Kind::MapData,
        what: "the maps it states dependencies for",
        stated: Stated::InTheFile("imapDependencies_2"),
        derivable: Derivable::Yes,
    },
    Rule {
        from: Kind::Manifest,
        to: Kind::TextureDict,
        what: "HDTxdBindingArray, binding a high-detail dictionary to an asset",
        stated: Stated::InTheFile("HDTxdBindingArray"),
        derivable: Derivable::Yes,
    },
];

/// Every kind a rule reads a reference out of.
pub fn kinds_with_a_ruling() -> Vec<Kind> {
    let mut out: Vec<Kind> = Vec::new();
    for rule in RULES {
        if !out.contains(&rule.from) {
            out.push(rule.from);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_classified_format_carries_a_ruling() {
        let ruled = kinds_with_a_ruling();
        // `Other` is the catch-all the catalogue puts everything unclassified
        // in, so it is not a format and cannot be ruled on as one.
        let unruled: Vec<Kind> = Kind::ALL
            .iter()
            .copied()
            .filter(|k| *k != Kind::Other && !ruled.contains(k))
            .collect();
        assert!(
            unruled.is_empty(),
            "no rule reads a reference out of {unruled:?}; add one, or a \
             Derivable::No saying why there is nothing to read"
        );
    }

    #[test]
    fn every_refusal_says_why() {
        for rule in RULES {
            if let Derivable::No(reason) = rule.derivable {
                assert!(
                    reason.len() > 40,
                    "{:?} -> {:?} refuses with too little to act on: {reason:?}",
                    rule.from,
                    rule.to
                );
            }
        }
    }

    #[test]
    fn neither_drawable_family_can_source_its_own_textures() {
        for from in [Kind::Drawable, Kind::Fragment] {
            let rule = RULES
                .iter()
                .find(|r| r.from == from && r.to == Kind::TextureDict)
                .unwrap_or_else(|| panic!("{from:?} has no texture ruling"));
            assert!(matches!(rule.derivable, Derivable::No(_)));
        }
    }
}
