//! The dictionary: ascending codes and the entries they key, each the 32-bit hash of its name where
//! the entry carries one.

use super::bytes::{f32_at, sections, u32_at, Bytes};
use super::drawable::{drawable_at, is_drawable_root, Drawable};
use super::texture::{is_texture, texture_at, DictionaryTexture};
use super::{HeaderlessError, Result};
use crate::gta6::hash::name_hash32;

#[derive(Debug)]
pub struct Dictionary {
    /// Each entry with its code; an entry of no kind read is refused by name.
    pub entries: Vec<(u32, Result<Entry>)>,
}

/// One entry of a dictionary, of the kinds whose layout was measured.
#[derive(Debug)]
pub enum Entry {
    Drawable(Box<Drawable>),
    Texture(DictionaryTexture),
    /// A frame filter, named `pack:/<stem>.iff`, and the tracks it weights.
    FrameFilter {
        name: String,
        tracks: Vec<FilterTrack>,
    },
    /// A 0x70-byte animation record: a name and one record per skeleton bone.
    AnimationRecord {
        name: String,
        bones: Vec<BoneRecord>,
    },
    /// A 0x20-byte record holding one 64-bit word whose role is not known.
    Word64(u64),
}

/// One 48-byte element of a frame filter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilterTrack {
    /// The name hash of a bone or track; 0 and 0xFFFFFFFF name none.
    pub track: u32,
    pub weight: f32,
    /// A name hash the tracks of one group share.
    pub group: u32,
    /// The word at 0x2C, whose role is not known.
    pub word_2c: u32,
}

/// One 144-byte element of an animation record: the settings of one skeleton bone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoneRecord {
    /// The name hash of a skeleton bone.
    pub bone: u32,
    pub kind: u32,
    pub flags: u32,
    /// The floats at 0x0C to 0x77, whose roles are not known.
    pub values: [f32; 27],
}

impl Entry {
    /// The text the entry's code hashes; `None` for a word, which carries no name.
    fn hashed(&self) -> Option<&str> {
        match self {
            Self::Drawable(d) => d.name.split('.').next(),
            Self::Texture(t) => Some(&t.name),
            Self::FrameFilter { name, .. } => name.rsplit('/').next()?.split('.').next(),
            Self::AnimationRecord { name, .. } => Some(name),
            Self::Word64(_) => None,
        }
    }
}

pub fn read_dictionary(virtual_section: &[u8], physical: Option<&[u8]>) -> Result<Dictionary> {
    sections(virtual_section, physical, dictionary_at)
}

fn dictionary_at(v: Bytes, p: Bytes) -> Result<Dictionary> {
    let count = v.u16("code count", 0x28)?;
    let entry_count = v.u16("entry count", 0x38)?;
    if count != entry_count {
        return Err(HeaderlessError::Disagree {
            what: "dictionary's code and entry counts",
            first: count.into(),
            second: entry_count.into(),
        });
    }
    // An empty dictionary has null arrays, parent 0 and ref count 1.
    if count == 0 {
        for (at, expected) in [(0x10, 0), (0x18, 1), (0x20, 0), (0x30, 0)] {
            let found = v.u64("empty dictionary's word", at)?;
            if found != expected {
                return Err(HeaderlessError::Constant {
                    what: "empty dictionary's parent, ref count or array pointer",
                    at: at as u64,
                    found,
                });
            }
        }
        return Ok(Dictionary {
            entries: Vec::new(),
        });
    }
    let codes = v.array("codes", v.to("codes", 0x20)?, count.into(), 4)?;
    let entries = v.to("entries", 0x30)?;
    v.array("entries", entries, count.into(), 8)?;
    let codes: Vec<u32> = codes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_le_bytes(*c))
        .collect();
    if codes.windows(2).any(|w| w[0] >= w[1]) {
        return Err(HeaderlessError::CodesNotAscending);
    }
    let mut read = Vec::with_capacity(codes.len());
    for (i, code) in codes.into_iter().enumerate() {
        let entry = v
            .to("entry", entries + 8 * i)
            .and_then(|at| entry_at(v, p, at))
            .and_then(|entry| match entry.hashed() {
                Some(name) if name_hash32(name.as_bytes()) != code => {
                    Err(HeaderlessError::Disagree {
                        what: "entry's code and the hash of its name",
                        first: code.into(),
                        second: name_hash32(name.as_bytes()).into(),
                    })
                }
                _ => Ok(entry),
            });
        read.push((code, entry));
    }
    Ok(Dictionary { entries: read })
}

/// Where each entry of the dictionary that is a drawable lies.
pub(super) fn drawable_entries(v: Bytes) -> Result<Vec<usize>> {
    let count = usize::from(v.u16("entry count", 0x38)?);
    if count == 0 {
        return Ok(Vec::new());
    }
    let entries = v.to("entries", 0x30)?;
    v.array("entries", entries, count, 8)?;
    let mut drawables = Vec::new();
    for i in 0..count {
        if let Some(at) = v.ptr("entry", entries + 8 * i)? {
            if !is_texture(v, at) && is_drawable_root(v, at, false) {
                drawables.push(at);
            }
        }
    }
    Ok(drawables)
}

/// Where the entry of code `code` lies; `None` where no code is `code`.
pub(super) fn entry_of(v: Bytes, code: u32) -> Result<Option<usize>> {
    let count = v.u16("code count", 0x28)?;
    let codes = v.array("codes", v.to("codes", 0x20)?, count.into(), 4)?;
    let codes = codes.as_chunks::<4>().0.iter();
    let Some(i) = codes
        .map(|c| u32::from_le_bytes(*c))
        .position(|c| c == code)
    else {
        return Ok(None);
    };
    v.to("entry", v.to("entries", 0x30)? + 8 * i).map(Some)
}

/// The entry at `at`, of the first kind whose test it meets.
fn entry_at(v: Bytes, p: Bytes, at: usize) -> Result<Entry> {
    if is_texture(v, at) {
        return texture_at(v, p, at).map(Entry::Texture);
    }
    match drawable_at(v, p, at, true) {
        Err(HeaderlessError::NotADrawable { .. }) => {}
        read => return read.map(|d| Entry::Drawable(Box::new(d))),
    }
    let word = |o| v.u64("dictionary record", at + o).ok();
    let half = |o| v.u32("dictionary record", at + o).ok();
    let pointer = |o| v.ptr("dictionary record", at + o).ok().flatten();
    let zero = word(0) == Some(0);
    if zero && half(0x08) == Some(1) && half(0x10) == Some(14) {
        if let Some(name) = pointer(0x18) {
            let name = v.c_str("frame filter name", name)?;
            let count = v.u16("frame filter track count", at + 0x30)?;
            let tracks = elements(v, "frame filter tracks", at + 0x28, count.into(), 48)?;
            let tracks = (tracks.as_chunks::<48>().0.iter())
                .map(|t| FilterTrack {
                    track: u32_at(t, 0x00),
                    weight: f32_at(t, 0x04),
                    group: u32_at(t, 0x28),
                    word_2c: u32_at(t, 0x2C),
                })
                .collect();
            return Ok(Entry::FrameFilter { name, tracks });
        }
    }
    if zero && word(0x08) == Some(0) && half(0x10) == Some(1) && pointer(0x20).is_some() {
        let count = v.u32("animation record count", at + 0x18)?;
        let first = v.to("animation record bones", at + 0x20)?;
        let bones = v.array("animation record bones", first, count as usize, 144)?;
        let mut records = Vec::with_capacity(bones.len() / 144);
        for (i, b) in bones.as_chunks::<144>().0.iter().enumerate() {
            if let Some(k) = (0..0x0C).chain(0x84..0x90).find(|k| b[*k] != 0) {
                return Err(HeaderlessError::Constant {
                    what: "bone record's zero bytes",
                    at: (first + 144 * i + k) as u64,
                    found: b[k].into(),
                });
            }
            records.push(BoneRecord {
                bone: u32_at(b, 0x80),
                kind: u32_at(b, 0x78),
                flags: u32_at(b, 0x7C),
                values: std::array::from_fn(|f| f32_at(b, 0x0C + 4 * f)),
            });
        }
        let name = name16(v, "animation record name", at + 0x58)?;
        return Ok(Entry::AnimationRecord {
            name,
            bones: records,
        });
    }
    if zero && word(0x08) == Some(0) && word(0x18) == Some(0) {
        if let Some(value) = word(0x10).filter(|&w| w != 0) {
            return Ok(Entry::Word64(value));
        }
    }
    Err(HeaderlessError::UnknownEntry { at: at as u64 })
}

/// The `count` elements of `stride` bytes the pointer at `at` addresses; none for a count of 0.
fn elements<'a>(
    v: Bytes<'a>,
    what: &'static str,
    at: usize,
    count: usize,
    stride: usize,
) -> Result<&'a [u8]> {
    if count == 0 {
        return Ok(&[]);
    }
    v.array(what, v.to(what, at)?, count, stride)
}

/// The 16-byte name at `at`: inline, its length in byte 15 with the top bit set, or a pointer
/// and a u16 length.
fn name16(v: Bytes, what: &'static str, at: usize) -> Result<String> {
    let field = v.get(what, at, 16)?;
    let (from, len) = if field[15] & 0x80 != 0 {
        (at, usize::from(field[15] & 0x7F).min(15))
    } else {
        (v.to(what, at)?, usize::from(v.u16(what, at + 8)?))
    };
    Ok(String::from_utf8_lossy(v.get(what, from, len)?).into_owned())
}
