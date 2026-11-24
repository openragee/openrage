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
    Resource(Rpf9Role),
    Body(OtherBody),
    Rbf,
}

impl Gta6EntryType {
    /// The measured content family in the table, without reading the entry's body.
    pub fn hint(entry: &Gta6Entry, extra: &Gta6EntryExtra) -> Self {
        if let Some(row) = extra.nested {
            if entry.class.is_some() || entry.real.is_some() || entry.stored.is_some() {
                return Self::Unknown;
            }
            return match row.resource_type {
                2 => Self::Resource(Rpf9Role::Drawable),
                3 => Self::Resource(Rpf9Role::DrawableWithBound),
                4 => Self::Resource(Rpf9Role::Dictionary),
                5 => Self::TextureDictionary,
                6 => Self::Resource(Rpf9Role::Bound),
                _ => Self::Unknown,
            };
        }
        match entry.kind() {
            Some(Gta6Class::NestedArchive) => Self::Archive,
            _ => Self::Unknown,
        }
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
