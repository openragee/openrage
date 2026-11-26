//! `.rel` — the audio metadata container: a data block of items, a name
//! table, an index of each item's span, and two tables of offsets into the data
//! block.
//!
//! Hand-written because no `.psc` describes it. Each item is kept as its bytes
//! in the data block; what an item's fields are depends on the file's
//! `rel_type`: [`speech`] and [`config`] type `rel_type` 4's, [`synth`] 10's,
//! [`dynamic_mix`] 15's, [`curve`] 16's, [`category`] 22's, [`sound`] 54's and
//! [`game_object`] 149's, 150's and 151's. [`interior`] adds, edits and removes
//! the interiors of a game data file and their rooms.
//!
//! Every length the file states is derived from what it carries and checked at
//! open, so a save is byte-identical with the file it opened.

pub mod category;
pub mod config;
pub mod curve;
pub mod dynamic_mix;
pub mod game_object;
pub mod interior;
pub mod layout;
pub mod sound;
pub mod speech;
pub mod synth;

use crate::error::{IoError, Result};

/// Where the data block starts in the file, which the hash and pack tables count from.
const DATA_START: u32 = 8;

/// One `.rel` file, as it states itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelFile {
    /// Which item taxonomy the data block holds.
    pub rel_type: u32,
    /// The items and every byte around them, verbatim.
    // ⚠ Not every byte is in an indexed item: in one shipped `rel_type` 54
    // file the first indexed item starts 339,922 bytes in.
    pub data: Vec<u8>,
    /// The name table, by position.
    pub names: Vec<String>,
    /// Each item's key and span in [`RelFile::data`], in the file's order.
    pub index: RelIndex,
    /// Offsets into [`RelFile::data`] that the hash table lists.
    // ⚠ The file states these and the pack table's from its own start, 8 bytes
    // before the data block; read that way, every one in both installs lands
    // inside an indexed item.
    pub hash_offsets: Vec<u32>,
    /// Offsets into [`RelFile::data`] that the pack table lists.
    pub pack_offsets: Vec<u32>,
}

/// The item index: keyed by hash in most files, by name in a few.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelIndex {
    /// Each item keyed by a 32-bit hash.
    Hashed(Vec<RelIndexHash>),
    /// Each item keyed by its name, at most 255 bytes.
    // ⚠ An empty one saves as an empty `Hashed`: with no entry, nothing in the
    // bytes tells the two forms apart.
    Named(Vec<RelIndexName>),
}

/// An index entry keyed by hash; `offset` counts from the data block's start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelIndexHash {
    pub hash: u32,
    pub offset: u32,
    pub length: u32,
}

/// An index entry keyed by name; `offset` counts from the data block's start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelIndexName {
    pub name: String,
    pub offset: u32,
    pub length: u32,
}

/// An index entry's key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelKey<'a> {
    Hash(u32),
    Name(&'a str),
}

/// One indexed item: its key and its bytes in the data block, never empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelItem<'a> {
    key: RelKey<'a>,
    body: &'a [u8],
}

impl<'a> RelItem<'a> {
    /// The key the index lists it under.
    pub fn key(&self) -> RelKey<'a> {
        self.key
    }

    /// Its bytes, the type id first.
    pub fn body(&self) -> &'a [u8] {
        self.body
    }

    /// The item's type within its file's `rel_type`: its first byte.
    // ⚠ Not a type in a `rel_type` 4 file keyed by hash: there the first byte
    // takes almost every value from 0 to 255 across both installs.
    pub fn type_id(&self) -> u8 {
        self.body[0]
    }
}

/// One indexed item read as a type of its file's `rel_type`.
#[derive(Debug, Clone, PartialEq)]
pub enum TypedItem {
    Speech(speech::SpeechItem),
    Config(config::ConfigItem),
    Synth(synth::SynthItem),
    DynamicMix(dynamic_mix::DynamicMixItem),
    Curve(curve::CurveItem),
    Category(category::CategoryItem),
    Sound(sound::SoundItem),
    GameObject(game_object::GameObjectItem),
}

impl TypedItem {
    /// The item's type, as the engine names it.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Speech(item) => item.type_name(),
            Self::Config(item) => item.value.type_name(),
            Self::Synth(item) => item.type_name(),
            Self::DynamicMix(item) => item.type_name(),
            Self::Curve(item) => item.type_name(),
            Self::Category(item) => item.type_name(),
            Self::Sound(item) => item.type_name(),
            Self::GameObject(item) => item.type_name(),
        }
    }
}

impl RelFile {
    /// Every indexed item in index order, typed by the file's `rel_type`; refuses a `rel_type` with no types or an item no type fits.
    pub fn typed_items(&self) -> Result<Vec<TypedItem>> {
        fn each<T>(
            file: &RelFile,
            decode: fn(&[u8]) -> Result<T>,
            wrap: fn(T) -> TypedItem,
        ) -> Result<Vec<TypedItem>> {
            file.items()?
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    decode(item.body())
                        .map(wrap)
                        .map_err(|e| refuse(format!("item {i} ({:?}): {e}", item.key())))
                })
                .collect()
        }
        match (self.rel_type, &self.index) {
            (4, RelIndex::Hashed(_)) => Ok(self
                .speech_items()?
                .into_iter()
                .map(TypedItem::Speech)
                .collect()),
            (4, RelIndex::Named(_)) => Ok(self
                .config_items()?
                .into_iter()
                .map(TypedItem::Config)
                .collect()),
            (10, _) => each(self, synth::SynthItem::decode, TypedItem::Synth),
            (15, _) => each(
                self,
                dynamic_mix::DynamicMixItem::decode,
                TypedItem::DynamicMix,
            ),
            (16, _) => each(self, curve::CurveItem::decode, TypedItem::Curve),
            (22, _) => each(self, category::CategoryItem::decode, TypedItem::Category),
            (54, _) => each(self, sound::SoundItem::decode, TypedItem::Sound),
            (149..=151, _) => each(
                self,
                game_object::GameObjectItem::decode,
                TypedItem::GameObject,
            ),
            (other, _) => Err(refuse(format!("rel_type {other} has no typed items"))),
        }
    }

    /// Reads a whole `.rel`, refusing any length or offset the bytes do not bear out.
    pub fn open(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader { bytes, at: 0 };
        let rel_type = r.u32("the header")?;
        let data_len = r.u32("the header")? as usize;
        let data = r.take(data_len, "the data block")?.to_vec();
        let names = read_names(&mut r)?;
        // ⚠ Nothing marks the index's form: a named index is recognised by the
        // word after its count, which states its names' bytes exactly.
        let (index, tail) = match read_named_index(bytes, r.at) {
            Some(named) => named,
            None => read_hashed_index(bytes, r.at)?,
        };
        let mut r = Reader { bytes, at: tail };
        let hash_offsets = r.offsets("the hash table")?;
        let pack_offsets = r.offsets("the pack table")?;
        if r.at != bytes.len() {
            return Err(refuse(format!(
                "{} bytes follow the pack table",
                bytes.len() - r.at
            )));
        }
        let file = Self {
            rel_type,
            data,
            names,
            index,
            hash_offsets,
            pack_offsets,
        };
        file.check_offsets()?;
        file.items()?;
        Ok(file)
    }

    /// Writes the file, refusing an index entry or table offset outside the data block.
    pub fn save(&self) -> Result<Vec<u8>> {
        self.items()?;
        self.check_offsets()?;
        let mut out = Vec::with_capacity(self.data.len() + 64);
        put(&mut out, self.rel_type);
        put(&mut out, len32(self.data.len(), "the data block")?);
        out.extend_from_slice(&self.data);

        let strings: usize = self.names.iter().map(|n| n.len() + 1).sum();
        let table = 4 + 4 * self.names.len() + strings;
        put(&mut out, len32(table, "the name table")?);
        put(&mut out, len32(self.names.len(), "the name table")?);
        let mut at = 0;
        for name in &self.names {
            put(&mut out, len32(at, "the name table")?);
            at += name.len() + 1;
        }
        for name in &self.names {
            out.extend_from_slice(name.as_bytes());
            out.push(0);
        }

        match &self.index {
            RelIndex::Hashed(entries) => {
                put(&mut out, len32(entries.len(), "the item table")?);
                for e in entries {
                    put(&mut out, e.hash);
                    put(&mut out, e.offset);
                    put(&mut out, e.length);
                }
            }
            RelIndex::Named(entries) => {
                put(&mut out, len32(entries.len(), "the item table")?);
                if !entries.is_empty() {
                    let names: usize = entries.iter().map(|e| 1 + e.name.len()).sum();
                    put(&mut out, len32(names, "the item table")?);
                }
                for e in entries {
                    let len = u8::try_from(e.name.len()).map_err(|_| {
                        refuse(format!(
                            "the item table names `{}`, longer than 255 bytes",
                            e.name
                        ))
                    })?;
                    out.push(len);
                    out.extend_from_slice(e.name.as_bytes());
                    put(&mut out, e.offset);
                    put(&mut out, e.length);
                }
            }
        }

        for table in [&self.hash_offsets, &self.pack_offsets] {
            put(&mut out, len32(table.len(), "an offset table")?);
            for &offset in table {
                let stated = offset as usize + DATA_START as usize;
                put(&mut out, len32(stated, "an offset table")?);
            }
        }
        Ok(out)
    }

    /// Every indexed item in index order, refusing an entry that is empty or outside the data block.
    pub fn items(&self) -> Result<Vec<RelItem<'_>>> {
        let spans: Vec<(RelKey<'_>, u32, u32)> = match &self.index {
            RelIndex::Hashed(entries) => entries
                .iter()
                .map(|e| (RelKey::Hash(e.hash), e.offset, e.length))
                .collect(),
            RelIndex::Named(entries) => entries
                .iter()
                .map(|e| (RelKey::Name(e.name.as_str()), e.offset, e.length))
                .collect(),
        };
        spans
            .into_iter()
            .enumerate()
            .map(|(i, (key, offset, length))| {
                let start = offset as usize;
                let body = start
                    .checked_add(length as usize)
                    .and_then(|end| self.data.get(start..end))
                    .filter(|body| !body.is_empty())
                    .ok_or_else(|| {
                        refuse(format!(
                            "the item table's entry {i} ({key:?}) spans {length} bytes at \
                             {offset:#x}, which is empty or outside the {}-byte data block",
                            self.data.len()
                        ))
                    })?;
                Ok(RelItem { key, body })
            })
            .collect()
    }

    fn check_offsets(&self) -> Result<()> {
        for (table, offsets) in [
            ("hash table", &self.hash_offsets),
            ("pack table", &self.pack_offsets),
        ] {
            if let Some(bad) = offsets.iter().find(|&&o| o as usize + 4 > self.data.len()) {
                return Err(refuse(format!(
                    "the {table} points at {bad:#x} in the data block, past its {} bytes",
                    self.data.len()
                )));
            }
        }
        Ok(())
    }
}

/// The name table: its byte length, a count, an offset per name, then the names, each NUL-terminated.
fn read_names(r: &mut Reader<'_>) -> Result<Vec<String>> {
    let len = r.u32("the name table")? as usize;
    let table = r.take(len, "the name table")?;
    let mut t = Reader {
        bytes: table,
        at: 0,
    };
    let count = t.u32("the name table")? as usize;
    if count > table.len() / 4 {
        return Err(refuse(format!(
            "the name table states {count} names in {} bytes",
            table.len()
        )));
    }
    let offsets = (0..count)
        .map(|_| t.u32("the name table"))
        .collect::<Result<Vec<_>>>()?;
    let strings = &table[t.at..];
    let mut names = Vec::with_capacity(count);
    let mut at = 0;
    for (i, &offset) in offsets.iter().enumerate() {
        // ⚠ A save derives each offset as where the names before it end, so
        // any other offset is refused rather than lost.
        if offset as usize != at {
            return Err(refuse(format!(
                "the name table puts name {i} at {offset:#x}, not at {at:#x} where the \
                 names before it end"
            )));
        }
        let end = strings[at..]
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| refuse(format!("the name table's name {i} has no terminator")))?;
        let name = std::str::from_utf8(&strings[at..at + end])
            .map_err(|_| refuse(format!("the name table's name {i} is not UTF-8")))?;
        names.push(name.to_string());
        at += end + 1;
    }
    if at != strings.len() {
        return Err(refuse(format!(
            "the name table holds {} bytes of names and its names take {at}",
            strings.len()
        )));
    }
    Ok(names)
}

/// The named form at `at`, or `None` when the bytes there are not one.
fn read_named_index(bytes: &[u8], at: usize) -> Option<(RelIndex, usize)> {
    let mut r = Reader { bytes, at };
    let count = r.u32("").ok()? as usize;
    let stated = r.u32("").ok()? as usize;
    if count == 0 || count > (bytes.len() - r.at) / 9 {
        return None;
    }
    let mut entries = Vec::with_capacity(count);
    let mut names = 0usize;
    for _ in 0..count {
        let len = r.take(1, "").ok()?[0] as usize;
        let name = std::str::from_utf8(r.take(len, "").ok()?).ok()?.to_string();
        names += 1 + len;
        let offset = r.u32("").ok()?;
        let length = r.u32("").ok()?;
        entries.push(RelIndexName {
            name,
            offset,
            length,
        });
    }
    (names == stated).then_some((RelIndex::Named(entries), r.at))
}

fn read_hashed_index(bytes: &[u8], at: usize) -> Result<(RelIndex, usize)> {
    let mut r = Reader { bytes, at };
    let count = r.u32("the item table")? as usize;
    if count > (bytes.len() - r.at) / 12 {
        return Err(refuse(format!(
            "the item table states {count} entries, more than the file holds"
        )));
    }
    let entries = (0..count)
        .map(|_| {
            Ok(RelIndexHash {
                hash: r.u32("the item table")?,
                offset: r.u32("the item table")?,
                length: r.u32("the item table")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((RelIndex::Hashed(entries), r.at))
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize, what: &str) -> Result<&'a [u8]> {
        let slice = self
            .at
            .checked_add(len)
            .and_then(|end| self.bytes.get(self.at..end))
            .ok_or_else(|| {
                refuse(format!(
                    "{what} needs {len} bytes at {:#x} and the file ends at {:#x}",
                    self.at,
                    self.bytes.len()
                ))
            })?;
        self.at += len;
        Ok(slice)
    }

    fn u32(&mut self, what: &str) -> Result<u32> {
        let b = self.take(4, what)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// A counted table of offsets the file states from its own start, as data-block offsets.
    fn offsets(&mut self, what: &str) -> Result<Vec<u32>> {
        let count = self.u32(what)? as usize;
        let words = self.take(count.saturating_mul(4), what)?;
        words
            .as_chunks::<4>()
            .0
            .iter()
            .map(|w| {
                let stated = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
                stated.checked_sub(DATA_START).ok_or_else(|| {
                    refuse(format!("{what} points at {stated:#x}, inside the header"))
                })
            })
            .collect()
    }
}

fn put(out: &mut Vec<u8>, word: u32) {
    out.extend_from_slice(&word.to_le_bytes());
}

fn len32(len: usize, what: &str) -> Result<u32> {
    u32::try_from(len).map_err(|_| refuse(format!("{what} is {len} bytes, past a u32")))
}

fn refuse(why: String) -> IoError {
    IoError::InvalidData {
        context: format!("rel: {why}"),
    }
}
