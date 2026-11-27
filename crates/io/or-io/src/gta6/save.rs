//! GTA VI's save containers: `JSAV`, a story save's index over one JSON text, opened, edited and
//! saved as an owned value; `ASAV`, a header before a payload, opened, edited and saved — the
//! checksum at +0x160 is the RAGE joaat of the header (see [`asav_checksum`]), recovered from the
//! Xbox build's `AsavHeader::ReadAndVerify` at 0x1428d0410 in `game_scarlett_master.exe`; and
//! `SLAYER1`, a packed copy whose clear head is read and whose body is refused.

use std::ops::Range;

use serde_json::Value;

/// The one `JSAV` version measured.
pub const JSAV_VERSION: u32 = 3;
/// The `ASAV` version measured on real bytes: a GTA VI build's settings, profile and story
/// containers all state 2. `SLAYER1` shares it, unmeasured on real bytes.
pub const ASAV_VERSION: u32 = 2;
/// The bytes of an `ASAV` header, and where its payload was measured to start.
pub const ASAV_HEADER: usize = 0x170;
/// The seed the RAGE joaat computes an ASAV header's checksum with, read from the Xbox build's
/// `AsavHeader::ComputeChecksum` at 0x1428d0410: `MOV R8D, 0x4BF672ED` before `CALL joaat(hdr, 0x170, seed)`.
pub const ASAV_CHECKSUM_SEED: u32 = 0x4BF6_72ED;
/// The bytes of a story save's index entry.
const ENTRY: usize = 20;
const SLAYER_TAG: &[u8; 8] = b"SLAYER1\0";

/// Why a save is not read, or not written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SaveError {
    #[error("the bytes start no save container: neither JSAV, ASAV nor SLAYER1")]
    NotASave,
    #[error("the file ends at {len} bytes, before the end of {what}")]
    Truncated { what: &'static str, len: usize },
    #[error("{container} version {found} is not read: only version {read} was measured")]
    Version {
        container: &'static str,
        found: u32,
        read: u32,
    },
    #[error(
        "block {index} is stated at {offset:#X} for {size} bytes, which is outside the file or \
         before the end of the block ahead of it"
    )]
    BlockPlacement {
        index: usize,
        offset: u32,
        size: u32,
    },
    #[error("block {index} is not UTF-8 text")]
    BlockText { index: usize },
    #[error("the bytes from {at:#X} are not the zero padding to a 16-byte boundary a story save holds there")]
    Padding { at: usize },
    #[error("no value lies at {path} in the block")]
    NoValue { path: String },
    #[error("the text is not one JSON value: {0}")]
    NotJson(String),
    #[error("nothing is added at {path} in the block: {why}")]
    NotAdded { path: String, why: &'static str },
    #[error("the payload is stated at {offset:#X}, which is inside the header or past the file's {len} bytes")]
    Payload { offset: u32, len: usize },
    #[error(
        "a packed copy (SLAYER1) of the save \"{title}\" in slot {slot}: its body is compressed \
         or encrypted and is not decoded; the plain JSAV beside it holds the same save"
    )]
    Packed { title: String, slot: String },
    #[error("the save passes what a 32-bit index states")]
    TooLarge,
    #[error(
        "the ASAV header's checksum at 0x160 is {found:#010x}, the RAGE joaat over the 0x170 \
         header with that slot zeroed is {computed:#010x}; the game refuses the file"
    )]
    Checksum { found: u32, computed: u32 },
}

/// Which container some bytes start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveKind {
    /// `JSAV`: a story save, read by [`Jsav`].
    Story,
    /// `ASAV`: a header and a payload, read by [`Asav`].
    Record,
    /// `SLAYER1`: a packed copy, whose head [`Slayer1`] reads.
    Packed,
}

impl SaveKind {
    pub fn of(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(b"JSAV") {
            Some(Self::Story)
        } else if bytes.starts_with(b"ASAV") {
            Some(Self::Record)
        } else if bytes.starts_with(SLAYER_TAG) {
            Some(Self::Packed)
        } else {
            None
        }
    }
}

/// A save that reads: a story save or a plain record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Save {
    Story(Jsav),
    Record(Asav),
}

/// The save `bytes` hold; a packed copy is refused as [`SaveError::Packed`], with its title and slot.
pub fn open(bytes: &[u8]) -> Result<Save, SaveError> {
    match SaveKind::of(bytes).ok_or(SaveError::NotASave)? {
        SaveKind::Story => Jsav::open(bytes).map(Save::Story),
        SaveKind::Record => Asav::open(bytes).map(Save::Record),
        SaveKind::Packed => {
            let packed = Slayer1::read(bytes)?;
            Err(SaveError::Packed {
                title: packed.header.title,
                slot: packed.header.slot,
            })
        }
    }
}

/// The hash a save writes for the name of a block or a field: case is kept.
pub const fn key_hash(name: &str) -> u32 {
    crate::hash::literal_string_hash(name.as_bytes())
}

/// The key a save writes for `name`: `$0x` and its hash in eight lower-case hex digits.
pub fn key_text(name: &str) -> String {
    format!("$0x{:08x}", key_hash(name))
}

/// The hash a key of the form `$0x%08x` states; `None` for a reserved key such as `$x` or `$data`.
pub fn parse_key(key: &str) -> Option<u32> {
    let digits = key.strip_prefix("$0x").filter(|d| d.len() == 8)?;
    u32::from_str_radix(digits, 16).ok()
}

/// Field names of a settings record that a brute-force against the hashes of a real save read
/// back: `audio`, `musicLevel`, `camera`, `fov`, `subtitles`, and the rest a Rockstar settings
/// file holds. The hash is the [`key_hash`] of the name on the right, so both forms round-trip.
///
/// Not every key the save holds is in this table — the brute reached 25 of 219 unique hashes in
/// a real `SettingsProfileSettings` record. Adding a candidate here is cheap: compute its hash
/// and, if the hash matches one the save states, commit the pair.
pub const KNOWN_SETTINGS_FIELD_NAMES: &[(u32, &str)] = &[
    (0x00ba555f, "closedCaptions"),
    (0x17d82a47, "display"),
    (0x2202fbe1, "autoLevel"),
    (0x23683a0d, "controls"),
    (0x25584db6, "camera"),
    (0x2af4b66c, "colorblindMode"),
    (0x3300c23e, "musicLevel"),
    (0x3301e662, "vibration"),
    (0x4b62d3b2, "language"),
    (0x6293d9ce, "autoSave"),
    (0x6a510681, "fov"),
    (0x75953d64, "radarMode"),
    (0x8372d0df, "radioStation"),
    (0x8f12d59c, "thirdPerson"),
    (0x944fa9d9, "hud"),
    (0x9c002b5e, "cinematic"),
    (0xa5341017, "targeting"),
    (0xb80c6ae8, "audio"),
    (0xbf1a0b77, "aimMode"),
    (0xc63c7f4b, "dialogueLevel"),
    (0xc7fbada1, "compassMode"),
    (0xc89952df, "subtitles"),
    (0xcb20b040, "accessibility"),
    (0xe76cda01, "firstPerson"),
    (0xee190b4e, "sfxLevel"),
];

/// The known name a save's `$0x%08x` key stands for; `None` for a key no name has been matched
/// to yet. Looks the hash up in [`KNOWN_SETTINGS_FIELD_NAMES`].
pub fn known_field_name(hash: u32) -> Option<&'static str> {
    KNOWN_SETTINGS_FIELD_NAMES
        .iter()
        .find(|(h, _)| *h == hash)
        .map(|(_, name)| *name)
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(std::array::from_fn(|k| bytes[at + k]))
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(std::array::from_fn(|k| bytes[at + k]))
}

fn align16(n: usize) -> usize {
    n.next_multiple_of(16)
}

/// RAGE's joaat over `bytes` seeded with `seed`. Each byte is sign-extended before it joins the
/// accumulator, the way the game's `AccumulateBytes` reads a `char` rather than a `uchar`, then
/// the `FinalizeAndHashBytes` wrapper runs the standard Jenkins one-at-a-time tail: `*9`, `^>>11`,
/// `*0x8001`. This is the hash the game writes to an ASAV header at +0x160.
pub fn rage_joaat_signed(bytes: &[u8], seed: u32) -> u32 {
    let mut h = seed;
    for &b in bytes {
        let signed = (b as i8) as i32 as u32;
        h = h.wrapping_add(signed);
        h = h.wrapping_mul(0x401); // == h + (h << 10)
        h ^= h >> 6;
    }
    h = h.wrapping_mul(9); // == h + (h << 3)
    h ^= h >> 11;
    h = h.wrapping_mul(0x8001); // == h + (h << 15)
    h
}

/// The joaat checksum the game writes to an ASAV header at +0x160: hashed over the whole 0x170
/// bytes of the header with the four bytes at +0x160 zeroed. The payload is not part of the hash.
///
/// Call it on the 0x170 header bytes you are about to write; the bytes at +0x160..+0x164 inside
/// `header` are ignored (they are zeroed internally for the hash) so the caller can leave them
/// as they stand.
pub fn asav_checksum(header: &[u8; ASAV_HEADER]) -> u32 {
    let mut buf = *header;
    buf[0x160..0x164].fill(0);
    rage_joaat_signed(&buf, ASAV_CHECKSUM_SEED)
}

fn checked_version(container: &'static str, found: u32, read: u32) -> Result<(), SaveError> {
    if found == read {
        return Ok(());
    }
    Err(SaveError::Version {
        container,
        found,
        read,
    })
}

/// One block of a story save: an index entry and the JSON text it points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// The block's id, which is also the key inside its group.
    pub hash: u32,
    /// 1 on every save measured.
    pub flags: u32,
    /// 0 on every save measured.
    pub reserved: u32,
    /// The block's JSON, as the file writes it: a number keeps its digits only while its text is kept.
    pub text: String,
    /// The bytes between the block before, or the body's start, and this block.
    lead: Vec<u8>,
}

impl Block {
    /// The block's JSON, parsed.
    pub fn value(&self) -> Result<Value, SaveError> {
        serde_json::from_str(&self.text).map_err(|e| SaveError::NotJson(e.to_string()))
    }

    /// The group hash and the inner hash the block's two outer keys state.
    pub fn keys(&self) -> Option<(u32, u32)> {
        let text = self.text.as_bytes();
        let key_at = |at: usize| {
            let end = string_end(text, at)?;
            parse_key(self.text.get(at + 1..end - 1)?)
        };
        let outer = member_start(text, skip_space(text, 0))?;
        let inner = member_start(text, value_after_key(text, outer)?)?;
        Some((key_at(outer)?, key_at(inner)?))
    }

    /// The text of the value `path` reaches: an object's member by its key as written, an array's element by its index.
    pub fn get(&self, path: &[&str]) -> Option<&str> {
        self.text.get(value_span(&self.text, path)?)
    }

    /// Put the JSON `json` in place of the value `path` reaches, and leave every other byte of the block.
    pub fn set(&mut self, path: &[&str], json: &str) -> Result<(), SaveError> {
        let json = json.trim();
        serde_json::from_str::<Value>(json).map_err(|e| SaveError::NotJson(e.to_string()))?;
        let span = value_span(&self.text, path).ok_or_else(|| SaveError::NoValue {
            path: path.join("/"),
        })?;
        self.text.replace_range(span, json);
        Ok(())
    }

    /// Add the JSON `json` after the last member of the object or element of the array `path`
    /// reaches: under `key` in an object that holds no such key, with no key in an array.
    pub fn insert(
        &mut self,
        path: &[&str],
        key: Option<&str>,
        json: &str,
    ) -> Result<(), SaveError> {
        let json = json.trim();
        serde_json::from_str::<Value>(json).map_err(|e| SaveError::NotJson(e.to_string()))?;
        let no = |why| SaveError::NotAdded {
            path: path.join("/"),
            why,
        };
        let text = self.text.as_bytes();
        let at = (value_span(&self.text, path))
            .ok_or(no("no value lies there"))?
            .start;
        let held = items(text, at).ok_or(no("the value there is no object and no array"))?;
        let piece = match (text[at] == b'{', key) {
            (true, Some(key)) if held.iter().any(|item| key_of(text, item) == Some(key)) => {
                return Err(no("the object already holds the key"))
            }
            (true, Some(key)) => format!("{}:{json}", Value::from(key)),
            (false, None) => json.to_owned(),
            (true, None) => return Err(no("a member of an object needs a key")),
            (false, Some(_)) => return Err(no("an element of an array takes no key")),
        };
        match held.last() {
            Some(last) => self.text.insert_str(last.end, &format!(",{piece}")),
            None => self.text.insert_str(at + 1, &piece),
        }
        Ok(())
    }

    /// Take out the member or element `path` reaches, with the comma that set it apart.
    pub fn remove(&mut self, path: &[&str]) -> Result<(), SaveError> {
        let no = || SaveError::NoValue {
            path: path.join("/"),
        };
        let (step, parent) = path.split_last().ok_or_else(no)?;
        let text = self.text.as_bytes();
        let at = value_span(&self.text, parent).ok_or_else(no)?.start;
        let held = items(text, at).ok_or_else(no)?;
        let k = match text[at] {
            b'{' => (held.iter()).position(|item| key_of(text, item) == Some(*step)),
            _ => step.parse().ok().filter(|k| *k < held.len()),
        }
        .ok_or_else(no)?;
        let cut = match (held.get(k + 1), k.checked_sub(1)) {
            (Some(next), _) => held[k].start..next.start,
            (None, Some(before)) => held[before].end..held[k].end,
            (None, None) => held[k].clone(),
        };
        self.text.replace_range(cut, "");
        Ok(())
    }
}

/// A story save: its code blocks and its script blocks, in the file's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jsav {
    pub code: Vec<Block>,
    pub script: Vec<Block>,
    /// The text after the last block, without the zero padding.
    tail: Vec<u8>,
}

impl Jsav {
    pub fn open(bytes: &[u8]) -> Result<Self, SaveError> {
        let truncated = |what| SaveError::Truncated {
            what,
            len: bytes.len(),
        };
        if !bytes.starts_with(b"JSAV") {
            return Err(SaveError::NotASave);
        }
        if bytes.len() < 16 {
            return Err(truncated("a story save's head"));
        }
        checked_version("JSAV", u32_at(bytes, 4), JSAV_VERSION)?;
        let code = u32_at(bytes, 8) as usize;
        let count = code.saturating_add(u32_at(bytes, 12) as usize);
        let index_end = (count.checked_mul(ENTRY))
            .and_then(|n| n.checked_add(16))
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| truncated("a story save's index"))?;
        let body = align16(index_end);
        zero_padding(bytes, index_end, body)?;
        let mut at = body;
        let mut blocks = Vec::with_capacity(count);
        for index in 0..count {
            let entry = 16 + index * ENTRY;
            let [hash, flags, reserved, offset, size] =
                std::array::from_fn(|k| u32_at(bytes, entry + 4 * k));
            let start = offset as usize;
            let end = start.saturating_add(size as usize);
            if start < at || end > bytes.len() {
                return Err(SaveError::BlockPlacement {
                    index,
                    offset,
                    size,
                });
            }
            let text = std::str::from_utf8(&bytes[start..end])
                .map_err(|_| SaveError::BlockText { index })?;
            blocks.push(Block {
                hash,
                flags,
                reserved,
                text: text.to_owned(),
                lead: bytes[at..start].to_vec(),
            });
            at = end;
        }
        let rest = &bytes[at..];
        let tail = &rest[..rest
            .iter()
            .rposition(|b| *b != 0)
            .map_or(0, |last| last + 1)];
        // The padding is checked, not kept: a save states none, so a writer can only compute it.
        if bytes.len() != align16(at + tail.len()) {
            return Err(SaveError::Padding {
                at: at + tail.len(),
            });
        }
        let script = blocks.split_off(code);
        Ok(Self {
            code: blocks,
            script,
            tail: tail.to_vec(),
        })
    }

    /// Every block, the code blocks first, as the index lists them.
    pub fn blocks(&self) -> impl Iterator<Item = &Block> {
        self.code.iter().chain(&self.script)
    }

    /// The block of id `hash`.
    pub fn block_mut(&mut self, hash: u32) -> Option<&mut Block> {
        (self.code.iter_mut().chain(&mut self.script)).find(|b| b.hash == hash)
    }

    /// The file: the index rebuilt over the blocks' texts, which are written as they are held.
    pub fn save(&self) -> Result<Vec<u8>, SaveError> {
        let word = |n: usize| u32::try_from(n).map_err(|_| SaveError::TooLarge);
        let count = self.code.len() + self.script.len();
        let mut out = Vec::new();
        out.extend_from_slice(b"JSAV");
        out.extend_from_slice(&JSAV_VERSION.to_le_bytes());
        out.extend_from_slice(&word(self.code.len())?.to_le_bytes());
        out.extend_from_slice(&word(self.script.len())?.to_le_bytes());
        out.resize(align16(16 + count * ENTRY), 0);
        for (index, block) in self.blocks().enumerate() {
            out.extend_from_slice(&block.lead);
            let entry = [
                block.hash,
                block.flags,
                block.reserved,
                word(out.len())?,
                word(block.text.len())?,
            ];
            for (k, field) in entry.into_iter().enumerate() {
                let at = 16 + index * ENTRY + 4 * k;
                out[at..at + 4].copy_from_slice(&field.to_le_bytes());
            }
            out.extend_from_slice(block.text.as_bytes());
        }
        out.extend_from_slice(&self.tail);
        out.resize(align16(out.len()), 0);
        Ok(out)
    }
}

/// Refuse `bytes[from..to]` unless it is there and all zero.
fn zero_padding(bytes: &[u8], from: usize, to: usize) -> Result<(), SaveError> {
    match bytes.get(from..to) {
        Some(pad) if pad.iter().all(|b| *b == 0) => Ok(()),
        _ => Err(SaveError::Padding { at: from }),
    }
}

fn skip_space(text: &[u8], mut at: usize) -> usize {
    while matches!(text.get(at), Some(b' ' | b'\t' | b'\r' | b'\n')) {
        at += 1;
    }
    at
}

/// The index past the string that opens at `at`.
fn string_end(text: &[u8], at: usize) -> Option<usize> {
    let mut i = at + 1;
    loop {
        match text.get(i)? {
            b'"' => return Some(i + 1),
            b'\\' => i += 2,
            _ => i += 1,
        }
    }
}

/// The index past the value that starts at `at`.
fn value_end(text: &[u8], at: usize) -> Option<usize> {
    match text.get(at)? {
        b'"' => string_end(text, at),
        b'{' | b'[' => {
            let (mut depth, mut i) = (0usize, at);
            loop {
                match text.get(i)? {
                    b'"' => {
                        i = string_end(text, i)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' if depth == 1 => return Some(i + 1),
                    b'}' | b']' => depth -= 1,
                    _ => {}
                }
                i += 1;
            }
        }
        _ => {
            let ends = |c: &u8| matches!(c, b',' | b'}' | b']' | b' ' | b'\t' | b'\r' | b'\n');
            let len = text[at..].iter().position(ends).unwrap_or(text.len() - at);
            (len > 0).then_some(at + len)
        }
    }
}

/// Where the first key of the object that opens at `at` starts.
fn member_start(text: &[u8], at: usize) -> Option<usize> {
    if *text.get(at)? != b'{' {
        return None;
    }
    let key = skip_space(text, at + 1);
    (*text.get(key)? == b'"').then_some(key)
}

/// Where the value of the member whose key starts at `key` starts.
fn value_after_key(text: &[u8], key: usize) -> Option<usize> {
    let colon = skip_space(text, string_end(text, key)?);
    (*text.get(colon)? == b':').then(|| skip_space(text, colon + 1))
}

/// Where the next member or element starts after the value at `value`; `None` at the container's end.
fn next_item(text: &[u8], value: usize) -> Option<usize> {
    let comma = skip_space(text, value_end(text, value)?);
    (*text.get(comma)? == b',').then(|| skip_space(text, comma + 1))
}

/// The bytes of each member, from its key, or element of the object or array that opens at `at`.
fn items(text: &[u8], at: usize) -> Option<Vec<Range<usize>>> {
    let object = match text.get(at)? {
        b'{' => true,
        b'[' => false,
        _ => return None,
    };
    let mut held = Vec::new();
    let mut item = skip_space(text, at + 1);
    if matches!(text.get(item)?, b'}' | b']') {
        return Some(held);
    }
    loop {
        let value = match object {
            true if *text.get(item)? == b'"' => value_after_key(text, item)?,
            true => return None,
            false => item,
        };
        held.push(item..value_end(text, value)?);
        match next_item(text, value) {
            Some(next) => item = next,
            None => return Some(held),
        }
    }
}

/// The key, as it is written, of the member whose bytes are `item`.
fn key_of<'a>(text: &'a [u8], item: &Range<usize>) -> Option<&'a str> {
    let end = string_end(text, item.start)?;
    std::str::from_utf8(text.get(item.start + 1..end - 1)?).ok()
}

/// The bytes of the value `path` reaches in the JSON `text`; a key is compared as it is written.
fn value_span(text: &str, path: &[&str]) -> Option<Range<usize>> {
    let bytes = text.as_bytes();
    let mut at = skip_space(bytes, 0);
    for step in path {
        at = match bytes.get(at)? {
            b'{' => {
                let mut key = member_start(bytes, at)?;
                loop {
                    let value = value_after_key(bytes, key)?;
                    if &bytes[key + 1..string_end(bytes, key)? - 1] == step.as_bytes() {
                        break value;
                    }
                    key = next_item(bytes, value)?;
                }
            }
            b'[' => {
                let mut element = skip_space(bytes, at + 1);
                for _ in 0..step.parse::<usize>().ok()? {
                    element = next_item(bytes, element)?;
                }
                (*bytes.get(element)? != b']').then_some(element)?
            }
            _ => return None,
        };
    }
    Some(at..value_end(bytes, at)?)
}

/// The header before an `ASAV` payload, each field as the file states it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsavHeader {
    /// 1 on a packed story save, 2 on settings and 3 on a profile record, of those measured.
    pub kind: u32,
    /// The word at 0x0C: 1 on a plain record and 3 on a packed one, as the real containers state it;
    /// what it counts is not proven.
    pub word_0c: u32,
    /// The word at 0x10: 0 on a plain record and 0x180 on a packed one, as the real containers state
    /// it; what it counts is not proven.
    pub word_10: u32,
    pub payload_offset: u32,
    /// The word at 0x18: 0 on a plain record, the stored size on a packed one.
    pub stored_size: u32,
    /// The file's size on a plain record, the unpacked size on a packed one.
    pub size: u32,
    /// Seconds since 1970, UTC.
    pub time: u64,
    pub title: String,
    pub slot: String,
    /// The RAGE joaat of the 0x170-byte header with the four bytes at +0x160 zeroed, seeded with
    /// [`ASAV_CHECKSUM_SEED`]. The payload is not part of the hash, so a payload change writes a
    /// file the game still accepts. On write, [`Asav::save`] computes this field from the final
    /// header and the caller can leave it as it stands.
    pub checksum: u32,
    /// The word at 0x164, zero where measured.
    pub word_164: u32,
    /// The file's size again, on a plain record.
    pub size_again: u64,
}

/// How many bytes each clear field of an `ASAV` header takes: the title is UTF-16 up to the slot,
/// and the slot is UTF-8 up to the checksum.
const ASAV_TITLE_BYTES: usize = 0x128 - 0x28;
const ASAV_SLOT_BYTES: usize = 0x160 - 0x128;

impl AsavHeader {
    pub fn read(bytes: &[u8]) -> Result<Self, SaveError> {
        if !bytes.starts_with(b"ASAV") {
            return Err(SaveError::NotASave);
        }
        let head = bytes.get(..ASAV_HEADER).ok_or(SaveError::Truncated {
            what: "an ASAV header",
            len: bytes.len(),
        })?;
        checked_version("ASAV", u32_at(head, 4), ASAV_VERSION)?;
        // The game refuses an ASAV whose joaat does not match: the Xbox build's
        // `AsavHeader::ReadAndVerify` ends with `iVar6 == *(u32*)(header+0x160)`.
        let head_array: [u8; ASAV_HEADER] = head.try_into().expect("head is ASAV_HEADER bytes");
        let computed = asav_checksum(&head_array);
        let found = u32_at(head, 0x160);
        if computed != found {
            return Err(SaveError::Checksum { found, computed });
        }
        let title: Vec<u16> = (head[0x28..0x128].as_chunks::<2>().0.iter())
            .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
            .take_while(|unit| *unit != 0)
            .collect();
        let slot = &head[0x128..0x160];
        let slot = &slot[..slot.iter().position(|b| *b == 0).unwrap_or(slot.len())];
        Ok(Self {
            kind: u32_at(head, 0x08),
            word_0c: u32_at(head, 0x0C),
            word_10: u32_at(head, 0x10),
            payload_offset: u32_at(head, 0x14),
            stored_size: u32_at(head, 0x18),
            size: u32_at(head, 0x1C),
            time: u64_at(head, 0x20),
            title: String::from_utf16_lossy(&title),
            slot: String::from_utf8_lossy(slot).into_owned(),
            checksum: u32_at(head, 0x160),
            word_164: u32_at(head, 0x164),
            size_again: u64_at(head, 0x168),
        })
    }

    /// The 0x170 bytes of the header as a file states them, every field from this struct and
    /// `ASAV` + [`ASAV_VERSION`] in the clear ahead of the title. A title longer than fits in
    /// the title slot or a slot longer than fits in its own slot are refused.
    pub fn write(&self) -> Result<[u8; ASAV_HEADER], SaveError> {
        let mut out = [0u8; ASAV_HEADER];
        out[..4].copy_from_slice(b"ASAV");
        out[4..8].copy_from_slice(&ASAV_VERSION.to_le_bytes());
        out[0x08..0x0C].copy_from_slice(&self.kind.to_le_bytes());
        out[0x0C..0x10].copy_from_slice(&self.word_0c.to_le_bytes());
        out[0x10..0x14].copy_from_slice(&self.word_10.to_le_bytes());
        out[0x14..0x18].copy_from_slice(&self.payload_offset.to_le_bytes());
        out[0x18..0x1C].copy_from_slice(&self.stored_size.to_le_bytes());
        out[0x1C..0x20].copy_from_slice(&self.size.to_le_bytes());
        out[0x20..0x28].copy_from_slice(&self.time.to_le_bytes());
        // Title: UTF-16 LE, zero-terminated, must leave room for the terminator.
        let title_units: Vec<u16> = self.title.encode_utf16().collect();
        if title_units.len() * 2 >= ASAV_TITLE_BYTES {
            return Err(SaveError::TooLarge);
        }
        for (k, unit) in title_units.into_iter().enumerate() {
            out[0x28 + 2 * k..0x28 + 2 * k + 2].copy_from_slice(&unit.to_le_bytes());
        }
        // Slot: UTF-8, zero-terminated, must leave room for the terminator.
        let slot = self.slot.as_bytes();
        if slot.len() >= ASAV_SLOT_BYTES {
            return Err(SaveError::TooLarge);
        }
        out[0x128..0x128 + slot.len()].copy_from_slice(slot);
        out[0x160..0x164].copy_from_slice(&self.checksum.to_le_bytes());
        out[0x164..0x168].copy_from_slice(&self.word_164.to_le_bytes());
        out[0x168..0x170].copy_from_slice(&self.size_again.to_le_bytes());
        Ok(out)
    }
}

/// A plain `ASAV` record: its header and the payload after it, which is not decoded here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asav {
    pub header: AsavHeader,
    pub payload: Vec<u8>,
}

impl Asav {
    pub fn open(bytes: &[u8]) -> Result<Self, SaveError> {
        let header = AsavHeader::read(bytes)?;
        let offset = header.payload_offset;
        let payload = (bytes.get(offset as usize..))
            .filter(|_| offset as usize >= ASAV_HEADER)
            .ok_or(SaveError::Payload {
                offset,
                len: bytes.len(),
            })?;
        Ok(Self {
            payload: payload.to_vec(),
            header,
        })
    }

    /// The payload as the JSON text a settings record holds; `None` where it is not UTF-8.
    pub fn text(&self) -> Option<&str> {
        let text = std::str::from_utf8(&self.payload).ok()?;
        Some(text.trim_end_matches('\0'))
    }

    /// The file: the header rebuilt over the payload, written at the offset the header states, with
    /// a fresh checksum at +0x160. The writer reconciles `size`, `size_again` and
    /// `payload_offset + payload.len()` from the payload it holds, and the checksum at +0x160 is
    /// computed by [`asav_checksum`] over the 0x170-byte header with that slot zeroed — the same
    /// routine the Xbox build's `AsavHeader::ReadAndVerify` runs when it loads a save.
    ///
    /// So a record whose payload was not changed round-trips to the same bytes, and a record whose
    /// payload *was* changed writes a file the game accepts: the payload is NOT part of the hash,
    /// so no payload edit can disagree with the header's checksum.
    pub fn save(&self) -> Result<Vec<u8>, SaveError> {
        let offset = self.header.payload_offset as usize;
        if offset < ASAV_HEADER {
            return Err(SaveError::Payload {
                offset: self.header.payload_offset,
                len: self.payload.len(),
            });
        }
        let total = offset
            .checked_add(self.payload.len())
            .ok_or(SaveError::TooLarge)?;
        let size = u32::try_from(total).map_err(|_| SaveError::TooLarge)?;
        let mut header = self.header.clone();
        // A plain record states its total length in `size` and `size_again`; `stored_size` is zero.
        header.size = size;
        header.size_again = u64::from(size);
        // Zero the checksum slot ahead of the write so `write()` lays the header out with that
        // slot zeroed, which is what [`asav_checksum`] expects to see. The final value goes in
        // once the hash is known.
        header.checksum = 0;
        let mut head_bytes = header.write()?;
        let checksum = asav_checksum(&head_bytes);
        head_bytes[0x160..0x164].copy_from_slice(&checksum.to_le_bytes());
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(&head_bytes);
        out.resize(offset, 0);
        out.extend_from_slice(&self.payload);
        Ok(out)
    }
}

/// The clear head of a `SLAYER1` packed copy: its name, and the header of the save it packs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slayer1 {
    pub name: String,
    pub payload_size: u64,
    /// The packed save's header: its title, slot and time are clear, its body is not.
    pub header: AsavHeader,
}

impl Slayer1 {
    /// A `SLAYER1` packed copy: the 8-byte tag, [`ASAV_VERSION`], the name's byte length and its
    /// bytes, the payload size and the packed `inner` body. Reading the written bytes gives a
    /// [`Slayer1`] whose header fields match `inner`'s own header; the payload beyond that is kept
    /// as given, since its encoding is not known.
    pub fn write(name: &str, inner: &[u8]) -> Result<Vec<u8>, SaveError> {
        let name_len = u32::try_from(name.len()).map_err(|_| SaveError::TooLarge)?;
        let payload_size = u64::try_from(inner.len()).map_err(|_| SaveError::TooLarge)?;
        let head_len = 16usize
            .checked_add(name.len())
            .and_then(|n| n.checked_add(8))
            .ok_or(SaveError::TooLarge)?;
        let total = head_len
            .checked_add(inner.len())
            .ok_or(SaveError::TooLarge)?;
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(SLAYER_TAG);
        out.extend_from_slice(&ASAV_VERSION.to_le_bytes());
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&payload_size.to_le_bytes());
        out.extend_from_slice(inner);
        Ok(out)
    }

    pub fn read(bytes: &[u8]) -> Result<Self, SaveError> {
        let truncated = SaveError::Truncated {
            what: "a SLAYER1 head",
            len: bytes.len(),
        };
        if !bytes.starts_with(SLAYER_TAG) {
            return Err(SaveError::NotASave);
        }
        if bytes.len() < 16 {
            return Err(truncated);
        }
        checked_version("SLAYER1", u32_at(bytes, 8), ASAV_VERSION)?;
        let name_end = 16usize.saturating_add(u32_at(bytes, 12) as usize);
        let (Some(name), Some(_)) = (
            bytes.get(16..name_end),
            bytes.get(name_end..name_end.saturating_add(8)),
        ) else {
            return Err(truncated);
        };
        Ok(Self {
            name: String::from_utf8_lossy(name).into_owned(),
            payload_size: u64_at(bytes, name_end),
            header: AsavHeader::read(&bytes[name_end + 8..])?,
        })
    }
}

#[cfg(test)]
#[path = "save_tests.rs"]
mod tests;
