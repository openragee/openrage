//! Roads: the path regions a folder holds as sources, the operations that edit them and the rules that say what is wrong in them.
//!
//! A set is every `<name>.paths.src` directly inside one folder, each the source of one `.ynd` (`or_source::paths`). A node is named
//! by its address, `region:index`, as a region's document names it, and a link by its node and its place among that node's links.
//! [`operation`] edits the regions of a set as owned values and writes each back as the document it cooks from, and [`rules`] reads
//! what the documents state, so a document a compile refuses is still judged.
//!
//! ⚠ **A link names its far node by region and index.** Taking a node out, or putting a vehicle node in before the pedestrian ones,
//! renumbers the later nodes of its region, so every link and junction reference of the set that names one is rewritten in the same
//! step, and the step is refused while a region that may name one is not in the set.
//!
//! ⚠ **Only what the declaration names is named here.** `CPathNodeLink` states its far node and one word, and `CPathNode` two words of
//! flags with a height and a count of links: nothing in `or-io` names the lanes of a link or a state that disables a node, so a link's
//! word is set whole and a request to disable a node is refused.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use or_source::paths::{Stated, KIND, RULES};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub mod operation;
pub mod rules;

/// A node by its address, written `region:index` as a path region's document writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Address {
    pub region: u16,
    /// The node's place among the nodes of its region, which is how the game finds it.
    pub index: u16,
}

impl Address {
    pub fn new(region: u16, index: u16) -> Self {
        Self { region, index }
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.region, self.index)
    }
}

impl FromStr for Address {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        text.split_once(':')
            .and_then(|(region, index)| Some(Self::new(region.parse().ok()?, index.parse().ok()?)))
            .ok_or_else(|| {
                format!("`{text}` is not a node address, which is written `region:index`")
            })
    }
}

impl Serialize for Address {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Address {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// A 32-bit word of a node or a link, written `"0x…"` as a path region's document writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Word(pub u32);

impl Serialize for Word {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&format_args!("{:#010x}", self.0))
    }
}

impl<'de> Deserialize<'de> for Word {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.strip_prefix("0x")
            .and_then(|digits| u32::from_str_radix(digits, 16).ok())
            .map(Self)
            .ok_or_else(|| {
                serde::de::Error::custom(format!(
                    "`{text}` is not a 32-bit word, which is written `0x` and its hexadecimal digits"
                ))
            })
    }
}

/// Why a folder of path regions was not read.
#[derive(Debug, thiserror::Error)]
#[error("{0}: {1}")]
pub struct Unread(pub String, pub std::io::Error);

/// The paths sources directly inside `folder`, each by the name of its folder, in the order of their names.
pub fn sources(folder: &Path) -> Result<Vec<(String, PathBuf)>, Unread> {
    let unread = |e| Unread(folder.display().to_string(), e);
    let mut found = Vec::new();
    for entry in std::fs::read_dir(folder).map_err(unread)? {
        let path = entry.map_err(unread)?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let is_paths =
            or_source::asset::parse_folder_name(name).is_some_and(|(_, kind)| kind == KIND);
        if is_paths && path.is_dir() {
            found.push((name.to_string(), path));
        }
    }
    found.sort();
    Ok(found)
}

/// One paths source of a set, as a listing and the rules read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// The folder of the source, by its name.
    pub source: String,
    /// What its document states, or why it does not read.
    pub stated: Result<Stated, String>,
}

/// What the document of every paths source directly inside `folder` states.
pub fn read(folder: &Path) -> Result<Vec<Listed>, Unread> {
    let listed = sources(folder)?.into_iter().map(|(source, dir)| {
        let stated = std::fs::read(dir.join(RULES))
            .map_err(|e| format!("{RULES}: cannot read: {e}"))
            .and_then(|rules| or_source::paths::stated(&rules).map_err(|p| p.to_string()));
        Listed { source, stated }
    });
    Ok(listed.collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_and_a_word_are_written_as_a_document_writes_them_and_read_back() {
        let address = Address::new(7, 40);
        let text = serde_json::to_string(&address).unwrap();
        assert_eq!(text, "\"7:40\"");
        assert_eq!(serde_json::from_str::<Address>(&text).unwrap(), address);
        for bad in ["\"7\"", "\"7:\"", "\"7:70000\"", "\"a:1\"", "7"] {
            assert!(serde_json::from_str::<Address>(bad).is_err(), "{bad}");
        }

        let word = Word(0x0040_0101);
        let text = serde_json::to_string(&word).unwrap();
        assert_eq!(text, "\"0x00400101\"");
        assert_eq!(serde_json::from_str::<Word>(&text).unwrap(), word);
        for bad in ["\"400101\"", "\"0x1ffffffff\"", "\"0xzz\"", "17"] {
            assert!(serde_json::from_str::<Word>(bad).is_err(), "{bad}");
        }
    }
}
