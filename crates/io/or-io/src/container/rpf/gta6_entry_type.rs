//! Content hints and strict texture previews of bytes read from an RPF9 table entry.

use super::{
    Gta6Class, Gta6Entry, Gta6EntryExtra, Rpf9Header, Rpf9Role, Rpf9Signature, RPF9_MAGIC,
};
use crate::error::{IoError, Result};
use crate::gta6::headerless::{read_dictionary, Dictionary, Entry};
use crate::gta6::{other_bodies::OtherBody, ui_tree::is_rbf};

/// A table's content hint, refined by a decoded entry's leading bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gta6EntryType {
    Unknown,
    Archive,
    TextureDictionary,
    Awc,
    Pso,
    File(&'static str),
    Resource(Rpf9Role),
    Body(OtherBody),
    Rbf,
}

impl Gta6EntryType {
    /// Identify a container from its leading header without validating its complete contents.
    pub fn from_header(bytes: &[u8]) -> Option<Self> {
        let head = bytes.get(..16)?;
        if crate::gta6::audio_data::stamped(head) {
            return Some(Self::File("rel"));
        }
        let text = head.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(head);
        if (text.starts_with(b"<?xml") && text.get(5).is_some_and(u8::is_ascii_whitespace))
            || starts_xml_element(text)
        {
            return Some(Self::File("xml"));
        }
        match &head[..4] {
            b"ADAT" | b"TADA" => Some(Self::Awc),
            b"PSIN" => Some(Self::Pso),
            b"KB2j" | b"KB2n" => Some(Self::File("bk2")),
            b"RBF0" => Some(Self::Rbf),
            b"PAMH" => Some(Self::Body(OtherBody::HeightMap)),
            b"MHIU" => Some(Self::Body(OtherBody::HeightGrid)),
            _ => None,
        }
    }

    /// The file family stated by either row layout, without reading its body.
    pub fn hint(entry: &Gta6Entry, extra: &Gta6EntryExtra) -> Self {
        let code = if let Some(row) = extra.nested {
            if entry.class.is_some() || entry.real.is_some() || entry.stored.is_some() {
                return Self::Unknown;
            }
            row.resource_type
        } else {
            if entry.kind() == Some(Gta6Class::NestedArchive) {
                return Self::Archive;
            }
            let Some(class) = entry.class else {
                return Self::Unknown;
            };
            u32::from(class)
        };
        match code {
            2 => Self::Resource(Rpf9Role::Drawable),
            3 => Self::Resource(Rpf9Role::DrawableWithBound),
            4 => Self::Resource(Rpf9Role::Dictionary),
            5 => Self::TextureDictionary,
            6 => Self::Resource(Rpf9Role::Bound),
            68 => Self::Resource(Rpf9Role::Cloth),
            72 => Self::Resource(Rpf9Role::ParticleEffects),
            79 => Self::Resource(Rpf9Role::NavMesh),
            _ => table_extension(code)
                .map(Self::File)
                .unwrap_or(Self::Unknown),
        }
    }

    /// The display suffix, independent of the name hash and of decoder availability.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Archive => "rpf",
            Self::TextureDictionary => "ytd",
            Self::Awc => "awc",
            Self::Pso => "psin",
            Self::File(extension) => extension,
            Self::Rbf => "rbf",
            Self::Resource(role) => match role {
                Rpf9Role::Drawable => "ydr",
                Rpf9Role::Dictionary => "ydd",
                Rpf9Role::DrawableWithBound => "yft",
                Rpf9Role::Bound => "ybn",
                Rpf9Role::NavMesh => "ynv",
                Rpf9Role::Cloth => "yld",
                Rpf9Role::ParticleEffects => "ypt",
                _ => "bin",
            },
            Self::Body(body) => match body {
                OtherBody::Gfx => "gfx",
                OtherBody::HeightMap | OtherBody::HeightGrid => "dat",
                OtherBody::MoveNetwork => "mrf",
                _ => "bin",
            },
            Self::Unknown => "bin",
        }
    }
}

fn starts_xml_element(head: &[u8]) -> bool {
    let Some(name) = head.strip_prefix(b"<") else {
        return false;
    };
    if !name
        .first()
        .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
    {
        return false;
    }
    let end = name
        .iter()
        .position(|b| !b.is_ascii_alphanumeric() && !b"_:-.".contains(b));
    end.is_none_or(|end| name[end] == b'>' || name[end] == b'/' || name[end].is_ascii_whitespace())
}

// Resource suffixes use the same display prefix as the other RAGE titles; it is not hashed.
fn table_extension(code: u32) -> Option<&'static str> {
    const BASE: &[&str] = &[
        "rpf", "ymf", "ydr", "yft", "ydd", "ytd", "ybn", "ybd", "ypd", "ybs", "ysd", "ymt", "ysc",
        "ycs", "ydatb", "ybdr", "ybdd", "ybft", "yvert",
    ];
    const STREAMED: &[&str] = &[
        "mrf", "cut", "gfx", "ycd", "yld", "ypmd", "ypm", "yed", "ypt", "ymap", "ytyp", "ych",
        "yldb", "yjd", "yad", "ynv", "ynvs", "ypl", "ynd", "yvr", "ywr", "ynh", "yfd", "yas",
        "ysm", "yscript", "yascript", "ysma", "iasf", "yfc", "ylfc", "ycg", "yasd", "rdb", "yfbs",
        "ycps", "ysp", "ytsl", "ymmd", "yfid", "ymsk", "yhdat", "ynmd", "ysmm", "yascd", "yrd",
        "ysmam", "yldbmt", "ynmd", "ykit", "ycgc", "ycov",
    ];
    if code < BASE.len() as u32 {
        BASE.get(code as usize).copied()
    } else {
        code.checked_sub(64)
            .and_then(|index| STREAMED.get(index as usize).copied())
    }
}

/// A decoded entry's content family and a completely readable texture dictionary, where present.
#[derive(Debug)]
pub struct Gta6EntryContent {
    pub kind: Gta6EntryType,
    pub textures: Option<Dictionary>,
}

impl Gta6EntryContent {
    /// Identify bytes decoded from the table and validate every texture before offering a preview.
    pub fn detect(entry: &Gta6Entry, extra: &Gta6EntryExtra, bytes: &[u8]) -> Result<Self> {
        let hint = Gta6EntryType::hint(entry, extra);
        if let Some(row) = extra.nested.filter(|row| row.resource_type == 5) {
            if hint != Gta6EntryType::TextureDictionary {
                return Err(bad("the texture dictionary has conflicting entry rows"));
            }
            if row.virtual_size & 15 != 0 || row.flags & 15 != 3 {
                return Err(bad("the texture dictionary has unsupported section flags"));
            }
            let virtual_len = (row.virtual_size & !15) as usize;
            let physical_len = (row.flags & !15) as usize;
            if virtual_len.checked_add(physical_len) != Some(bytes.len()) {
                return Err(bad(
                    "the texture dictionary's section sizes do not match its decoded bytes",
                ));
            }
            let (virtual_section, physical) = bytes.split_at(virtual_len);
            if Rpf9Signature::of(virtual_section, virtual_len as u64).and_then(Rpf9Signature::role)
                != Some(Rpf9Role::Dictionary)
            {
                return Err(bad("the texture dictionary's root is not a dictionary"));
            }
            let dictionary = read_dictionary(virtual_section, Some(physical))
                .map_err(|error| bad(format!("texture dictionary: {error}")))?;
            for (code, result) in &dictionary.entries {
                match result {
                    Ok(Entry::Texture(texture)) => {
                        if let Err(error) = &texture.texture {
                            return Err(bad(format!("texture {code:08X} pixels: {error}")));
                        }
                    }
                    Ok(_) => {
                        return Err(bad(format!(
                            "texture dictionary entry {code:08X} is not a texture"
                        )))
                    }
                    Err(error) => {
                        return Err(bad(format!("texture dictionary entry {code:08X}: {error}")))
                    }
                }
            }
            return Ok(Self {
                kind: Gta6EntryType::TextureDictionary,
                textures: Some(dictionary),
            });
        }

        let kind = if bytes.starts_with(&RPF9_MAGIC.to_le_bytes()) {
            Rpf9Header::parse(bytes)?;
            Gta6EntryType::Archive
        } else if hint == Gta6EntryType::Archive {
            return Err(bad("the nested archive has no RPF9 header"));
        } else if let Some(kind) = Gta6EntryType::from_header(bytes) {
            kind
        } else if is_rbf(bytes) {
            Gta6EntryType::Rbf
        } else if let Some(body) = OtherBody::of(bytes, bytes.len() as u64) {
            Gta6EntryType::Body(body)
        } else {
            // Only a table-validated split supplies a headerless resource's virtual extent.
            let role = extra.nested.and_then(|row| {
                let virtual_len = (row.virtual_size & !15) as usize;
                let physical_len = (row.flags & !15) as usize;
                if virtual_len.checked_add(physical_len) != Some(bytes.len()) {
                    return None;
                }
                Rpf9Signature::of(&bytes[..virtual_len], virtual_len as u64)?.role()
            });
            role.map(Gta6EntryType::Resource)
                .unwrap_or(Gta6EntryType::Unknown)
        };
        Ok(Self {
            kind,
            textures: None,
        })
    }
}

fn bad(why: impl Into<String>) -> IoError {
    IoError::Invalid {
        what: "an RPF9 entry's content".into(),
        why: why.into(),
    }
}

#[cfg(test)]
#[path = "gta6_entry_type_tests.rs"]
mod tests;
