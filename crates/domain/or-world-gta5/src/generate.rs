//! Archetypes derived from the drawables they declare: batch authoring of a `.ytyp`.
//!
//! This crate holds no drawable reader and must not grow one, so the input is
//! [`DrawableFacts`] — what a caller that can read `.ydr`, `.ydd` and `.yft`
//! extracted — and the output is a `CBaseArchetypeDef` value tree.
//!
//! ⚠ **Which fields follow from the drawable is a measurement, not a
//! convention.** `what_an_archetype_takes_from_its_drawable` compares every
//! shipped archetype whose drawable the mount can find against that
//! drawable, and [`Agreement`] is the instrument. Everything the files do not
//! show to be derived is an [`Authoring`] decision the caller states.

use glam::Vec3;
use or_io::meta::instance::HashedString;
use or_io::meta::{MetaInstance, Value};

use crate::field;
use crate::keys;
use crate::map_types::Archetype;

/// Which asset an archetype names, as `CBaseArchetypeDef::assetType` spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AssetKind {
    Fragment,
    Drawable,
    DrawableDictionary,
    Assetless,
}

impl AssetKind {
    /// The `rage::fwArchetypeDef::eAssetType` variant.
    pub fn variant(self) -> &'static str {
        match self {
            AssetKind::Fragment => "ASSET_TYPE_FRAGMENT",
            AssetKind::Drawable => "ASSET_TYPE_DRAWABLE",
            AssetKind::DrawableDictionary => "ASSET_TYPE_DRAWABLEDICTIONARY",
            AssetKind::Assetless => "ASSET_TYPE_ASSETLESS",
        }
    }

    /// The kind an archetype states, whichever way the reader spelt the enum.
    ///
    /// ⚠ With no schema snapshot an enum reads as its number, so the
    /// numeric spellings are accepted too: 1 to 4 in declaration order.
    pub fn of(arch: &Archetype<'_>) -> Option<Self> {
        let variant = match field::get(arch.instance(), keys::ASSET_TYPE)? {
            Value::Enum { variant, .. } => variant.clone(),
            Value::UInt(n) => n.to_string(),
            Value::Int(n) => n.to_string(),
            _ => return None,
        };
        let numeric = variant
            .strip_prefix("0x")
            .and_then(|h| u32::from_str_radix(h, 16).ok())
            .or_else(|| variant.parse().ok());
        match (variant.as_str(), numeric) {
            ("ASSET_TYPE_FRAGMENT", _) | (_, Some(1)) => Some(AssetKind::Fragment),
            ("ASSET_TYPE_DRAWABLE", _) | (_, Some(2)) => Some(AssetKind::Drawable),
            ("ASSET_TYPE_DRAWABLEDICTIONARY", _) | (_, Some(3)) => {
                Some(AssetKind::DrawableDictionary)
            }
            ("ASSET_TYPE_ASSETLESS", _) | (_, Some(4)) => Some(AssetKind::Assetless),
            _ => None,
        }
    }
}

/// What a drawable says about itself that an archetype repeats.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawableFacts {
    /// The file stem, or the dictionary entry's name.
    pub name: String,
    pub kind: AssetKind,
    /// The `.ydd` stem, for an entry of one.
    pub dictionary: Option<String>,
    pub bb_min: Vec3,
    pub bb_max: Vec3,
    pub bs_centre: Vec3,
    pub bs_radius: f32,
}

/// What an archetype needs that its drawable does not record.
///
/// ⚠ The defaults are a starting point, not a derivation. Over both retail
/// installs `lodDist` equals one of a drawable's LOD distances on 10 of
/// 308,537 non-fragment archetypes, and `textureDictionary` names the
/// archetype's own asset on 7,445 of 154,848 drawables and 125 of 153,689
/// dictionary entries — shipped content almost always shares a texture
/// dictionary, which only its author knows.
#[derive(Debug, Clone, PartialEq)]
pub struct Authoring {
    pub lod_dist: f32,
    pub hd_texture_dist: f32,
    pub flags: u32,
    pub special_attribute: u32,
    /// `None` names the archetype's own asset, which is the common case.
    pub texture_dictionary: Option<String>,
    pub physics_dictionary: Option<String>,
    pub clip_dictionary: Option<String>,
}

impl Default for Authoring {
    fn default() -> Self {
        Self {
            lod_dist: 100.0,
            hd_texture_dist: 15.0,
            flags: 0,
            special_attribute: 0,
            texture_dictionary: None,
            physics_dictionary: None,
            clip_dictionary: None,
        }
    }
}

/// ⚠ A dictionary entry's name survives only as its hash, so `0xHHHHHHHH`
/// is taken as the number rather than hashed as text.
fn hashed(text: &str) -> Value {
    if text.is_empty() {
        Value::Hash(HashedString::from_hash(0))
    } else {
        Value::Hash(HashedString::from_xml_text(text))
    }
}

/// A `CBaseArchetypeDef` for `facts`, every schema field present in schema order.
///
/// `asset_type_enum` is the enum's type name as the destination document
/// spells it; an archetype carried from another archetype in that document
/// reads it back, and a mismatched spelling is a write refusal rather than a
/// wrong byte.
pub fn archetype(
    facts: &DrawableFacts,
    authoring: &Authoring,
    asset_type_enum: &str,
) -> MetaInstance {
    let mut inst = MetaInstance::new(keys::types::C_BASE_ARCHETYPE_DEF);
    inst.set("lodDist", Value::Float(authoring.lod_dist as f64));
    inst.set("flags", Value::UInt(authoring.flags as u64));
    inst.set(
        "specialAttribute",
        Value::UInt(authoring.special_attribute as u64),
    );
    inst.set("bbMin", Value::Vec3(facts.bb_min.to_array()));
    inst.set("bbMax", Value::Vec3(facts.bb_max.to_array()));
    inst.set("bsCentre", Value::Vec3(facts.bs_centre.to_array()));
    inst.set("bsRadius", Value::Float(facts.bs_radius as f64));
    inst.set(
        "hdTextureDist",
        Value::Float(authoring.hd_texture_dist as f64),
    );
    inst.set("name", hashed(&facts.name));
    inst.set(
        "textureDictionary",
        hashed(
            authoring
                .texture_dictionary
                .as_deref()
                .unwrap_or(&facts.name),
        ),
    );
    inst.set(
        "clipDictionary",
        hashed(authoring.clip_dictionary.as_deref().unwrap_or("")),
    );
    inst.set(
        "drawableDictionary",
        hashed(facts.dictionary.as_deref().unwrap_or("")),
    );
    inst.set(
        "physicsDictionary",
        hashed(authoring.physics_dictionary.as_deref().unwrap_or("")),
    );
    inst.set(
        "assetType",
        Value::Enum {
            type_name: asset_type_enum.to_string(),
            variant: facts.kind.variant().to_string(),
        },
    );
    inst.set("assetName", hashed(&facts.name));
    inst.set("extensions", Value::Array(Vec::new()));
    inst
}

/// Why a type file could not be generated from a donor.
#[derive(Debug, thiserror::Error)]
pub enum GenerateError {
    #[error("the donor is not a CMapTypes")]
    NotATypeFile,
    /// ⚠ A class's `signature` word is derived by nothing in this tree, so
    /// synthesis copies it from a donor that already names the class, and a
    /// donor holding no `CBaseArchetypeDef` would leave it zero — a value no
    /// shipped file carries.
    #[error(
        "the donor declares no CBaseArchetypeDef, so the archetypes would be written with a \
         zero signature no shipped file carries; use a donor that holds one"
    )]
    NoArchetypeClass,
}

/// A copy of `donor` holding `archetypes` and nothing else, named `name`.
///
/// Every other array the donor carries — dependencies, composite entity
/// types, type-file extensions — is emptied, so nothing of the donor's
/// content leaks into the new file; what survives is its type table.
pub fn type_file(
    donor: &MetaInstance,
    name: u32,
    archetypes: Vec<MetaInstance>,
) -> Result<MetaInstance, GenerateError> {
    let types = crate::map_types::MapTypes::new(donor).ok_or(GenerateError::NotATypeFile)?;
    if !types
        .archetypes()
        .any(|a| field::is_type(a.instance(), keys::types::C_BASE_ARCHETYPE_DEF))
    {
        return Err(GenerateError::NoArchetypeClass);
    }
    let mut out = donor.clone();
    for (slot_name, value) in &mut out.fields {
        let key = field::name_hash(slot_name);
        if key == keys::NAME {
            *value = Value::Hash(HashedString::from_hash(name));
        } else if key == keys::ARCHETYPES {
            *value = Value::Array(
                archetypes
                    .iter()
                    .cloned()
                    .map(|a| Value::Pointer(Some(Box::new(a))))
                    .collect(),
            );
        } else if key == keys::DEPENDENCIES
            || key == keys::COMPOSITE_ENTITY_TYPES
            || key == keys::EXTENSIONS
        {
            *value = Value::Array(Vec::new());
        }
    }
    Ok(out)
}

/// The asset-type enum's type name as `donor`'s archetypes spell it.
pub fn asset_type_enum_of(donor: &MetaInstance) -> Option<String> {
    let types = crate::map_types::MapTypes::new(donor)?;
    types
        .archetypes()
        .find_map(|a| match field::get(a.instance(), keys::ASSET_TYPE) {
            Some(Value::Enum { type_name, .. }) => Some(type_name.clone()),
            _ => None,
        })
}

/// How closely a stored value matched its derived one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Closeness {
    Exact,
    /// Within a millimetre on every component.
    Millimetre,
    /// Within a centimetre.
    Centimetre,
    Further,
}

impl Closeness {
    pub fn of(stored: Vec3, derived: Vec3) -> Self {
        let d = (stored - derived).abs().max_element();
        if stored == derived {
            Closeness::Exact
        } else if d <= 0.001 {
            Closeness::Millimetre
        } else if d <= 0.01 {
            Closeness::Centimetre
        } else {
            Closeness::Further
        }
    }
}

/// A shipped archetype held against the drawable it names.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Agreement {
    pub bb_min: Closeness,
    pub bb_max: Closeness,
    pub bs_centre: Closeness,
    pub bs_radius: Closeness,
    /// Whether `name` equals `assetName`.
    pub name_is_asset: bool,
    /// Whether `textureDictionary` names the archetype's own asset.
    pub txd_is_asset: bool,
}

impl Agreement {
    pub fn of(arch: &Archetype<'_>, facts: &DrawableFacts) -> Option<Self> {
        let bounds = arch.bounds()?;
        let scalar = |v: f32| Vec3::splat(v);
        Some(Self {
            bb_min: Closeness::of(bounds.min, facts.bb_min),
            bb_max: Closeness::of(bounds.max, facts.bb_max),
            bs_centre: Closeness::of(bounds.centre, facts.bs_centre),
            bs_radius: Closeness::of(scalar(bounds.radius), scalar(facts.bs_radius)),
            name_is_asset: arch.name() == arch.asset_name(),
            txd_is_asset: arch.texture_dictionary() == arch.asset_name(),
        })
    }

    /// Every bound within a centimetre of the drawable's.
    pub fn bounds_agree(&self) -> bool {
        [self.bb_min, self.bb_max, self.bs_centre, self.bs_radius]
            .iter()
            .all(|c| *c <= Closeness::Centimetre)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> DrawableFacts {
        DrawableFacts {
            name: "prop_box".into(),
            kind: AssetKind::Drawable,
            dictionary: None,
            bb_min: Vec3::new(-1.0, -2.0, 0.0),
            bb_max: Vec3::new(1.0, 2.0, 3.0),
            bs_centre: Vec3::new(0.0, 0.0, 1.5),
            bs_radius: 2.5,
        }
    }

    #[test]
    fn a_generated_archetype_agrees_with_the_facts_it_came_from() {
        let facts = facts();
        let inst = archetype(
            &facts,
            &Authoring::default(),
            "rage__fwArchetypeDef__eAssetType",
        );
        let arch = Archetype::new(&inst);
        let agreement = Agreement::of(&arch, &facts).unwrap();
        assert!(agreement.bounds_agree());
        assert_eq!(agreement.bb_min, Closeness::Exact);
        assert!(agreement.name_is_asset);
        assert!(agreement.txd_is_asset);
        assert_eq!(AssetKind::of(&arch), Some(AssetKind::Drawable));
        assert_eq!(arch.name(), or_io::string_hash(b"prop_box"));
    }

    #[test]
    fn a_moved_bound_is_not_agreement() {
        let facts = facts();
        let inst = archetype(&facts, &Authoring::default(), "");
        let mut moved = facts.clone();
        moved.bb_max.x += 0.02;
        let agreement = Agreement::of(&Archetype::new(&inst), &moved).unwrap();
        assert_eq!(agreement.bb_max, Closeness::Further);
        assert!(!agreement.bounds_agree());
    }

    #[test]
    fn every_field_the_schema_declares_is_present() {
        let lib = crate::test_schema();
        let Some(resolved) = lib.resolve("CBaseArchetypeDef") else {
            or_test_support::skip_corpus(
                "every_field_the_schema_declares_is_present",
                "the installed schema library has no CBaseArchetypeDef",
            );
            return;
        };
        let inst = archetype(&facts(), &Authoring::default(), "");
        let ours: Vec<&str> = inst.fields.iter().map(|(n, _)| n.as_str()).collect();
        let schema: Vec<&str> = resolved.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(ours, schema);
    }

    #[test]
    fn an_asset_kind_reads_from_its_number_as_well_as_its_name() {
        for (n, kind) in [
            (1, AssetKind::Fragment),
            (2, AssetKind::Drawable),
            (3, AssetKind::DrawableDictionary),
            (4, AssetKind::Assetless),
        ] {
            let mut inst = MetaInstance::new("CBaseArchetypeDef");
            inst.set(
                "assetType",
                Value::Enum {
                    type_name: String::new(),
                    variant: format!("0x{n:08X}"),
                },
            );
            assert_eq!(AssetKind::of(&Archetype::new(&inst)), Some(kind));
        }
    }
}
