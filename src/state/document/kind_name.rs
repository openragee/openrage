//! How a `DocumentKind` is spelled in preferences: its variant's name, or a
//! registered resource's extension behind a prefix.

use serde::{Deserialize, Serialize};

use super::DocumentKind;

impl Serialize for DocumentKind {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Registered(resource) => {
                s.serialize_str(&format!("{}{}", REGISTERED_PREFIX, resource.extension()))
            }
            named => s.serialize_str(named.variant_name()),
        }
    }
}

impl<'de> Deserialize<'de> for DocumentKind {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        if let Some(extension) = raw.strip_prefix(REGISTERED_PREFIX) {
            return or_io::container::resource::ResourceType::from_extension(extension)
                .map(Self::Registered)
                .ok_or_else(|| serde::de::Error::custom(format!("unknown resource .{extension}")));
        }
        Self::from_variant_name(&raw)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown document kind {raw}")))
    }
}

const REGISTERED_PREFIX: &str = "registered:";

impl DocumentKind {
    pub(super) fn variant_name(self) -> &'static str {
        match self {
            Self::Ypt => "Ypt",
            Self::Ytd => "Ytd",
            Self::Ydr => "Ydr",
            Self::Ydd => "Ydd",
            Self::Yft => "Yft",
            Self::Ybn => "Ybn",
            Self::Yld => "Yld",
            Self::Ycd => "Ycd",
            Self::Yed => "Yed",
            Self::Yfd => "Yfd",
            Self::Ynd => "Ynd",
            Self::Ynv => "Ynv",
            Self::Ypdb => "Ypdb",
            Self::Fxc => "Fxc",
            Self::Gxt2 => "Gxt2",
            Self::Bik => "Bik",
            Self::Awc => "Awc",
            Self::HeightMap => "HeightMap",
            Self::DistantLights => "DistantLights",
            Self::WaterMap => "WaterMap",
            Self::WorldBlanket => "WorldBlanket",
            Self::NavPrecalc => "NavPrecalc",
            Self::DatTable => "DatTable",
            Self::Rel => "Rel",
            Self::NameTable => "NameTable",
            Self::Gfx => "Gfx",
            Self::MoveNetwork => "MoveNetwork",
            Self::Dds => "Dds",
            Self::Ttpf => "Ttpf",
            Self::Rpf => "Rpf",
            Self::Rpf9 => "Rpf9",
            Self::Gta6Anim => "Gta6Anim",
            Self::Gta6Drawable => "Gta6Drawable",
            Self::Gta6Text => "Gta6Text",
            Self::Gta6AudioData => "Gta6AudioData",
            Self::Gta6Expressions => "Gta6Expressions",
            Self::Gta6Save => "Gta6Save",
            Self::Gta6Script => "Gta6Script",
            Self::GameRoot => "GameRoot",
            Self::Folder => "Folder",
            Self::Text => "Text",
            Self::Meta => "Meta",
            Self::Hex => "Hex",
            Self::Compare => "Compare",
            Self::Registered(_) => "",
        }
    }

    pub(super) fn from_variant_name(name: &str) -> Option<Self> {
        Self::NAMED
            .iter()
            .copied()
            .find(|kind| kind.variant_name() == name)
    }
}
