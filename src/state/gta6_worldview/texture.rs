//! A texture as a worker read it: the level the screen asks for and every level under it, as the
//! block rows its pack stores where a device takes them as they are, else as 8-bit RGBA.

use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use or_io::codec::texture::{decode_level, DxgiFormat, TextureSource};
use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9ZlibRun};
use or_io::gta6::hash::name_hash64;
use or_io::gta6::headerless::{read_dictionary, Entry};
use or_io::gta6::{Texture, TexturePack};
use wgpu::TextureFormat;

use crate::state::gta6_texture_index::DictionaryPlaces;

/// The most a dictionary body read for one of its textures inflates to.
const BODY_LIMIT: u64 = 1 << 28;

/// A texture on the CPU: its levels, the largest first, each half the last.
pub struct CpuTexture {
    pub width: u32,
    pub height: u32,
    /// `Rgba8Unorm` or `Rgba8UnormSrgb` for decoded levels; a `Bc*` format for levels of whole block rows.
    pub format: TextureFormat,
    pub levels: Vec<Vec<u8>>,
}

impl CpuTexture {
    /// The bytes of its levels.
    pub fn bytes(&self) -> u64 {
        self.levels.iter().map(|level| level.len() as u64).sum()
    }
}

/// The texture packs of the install, and the pack each key is in.
pub(super) struct Places {
    pub(super) packs: Vec<PathBuf>,
    pub(super) keys: HashMap<u64, u32>,
    /// The format code each key's texture states.
    pub(super) formats: HashMap<u64, u16>,
    /// The install's texture archive, and the dictionary body in it that holds each key no pack holds.
    pub(super) dictionaries: DictionaryPlaces,
}

impl Places {
    /// Whether the texture of `key` states an sRGB format, as a colour map does; `false` for a key no pack holds.
    pub(super) fn is_colour(&self, key: u64) -> bool {
        let srgb = |code: &u16| DxgiFormat::from_raw((code >> 3).into()).is_srgb();
        self.formats.get(&key).is_some_and(srgb)
    }
}

/// A pack opened, with the place of each key in it; `None` for one that did not open.
pub(super) type OpenPack = Option<(TexturePack, HashMap<u64, usize>)>;

/// The packs the readers opened, by their place among the install's, shared by every reader.
// ⚠ A pack's table is opened once for all the readers: one copy a reader held a pack's table as many times as there were readers.
pub(super) type OpenPacks = Mutex<HashMap<u32, Arc<OpenPack>>>;

/// The pack at `at` among `places`, opened by the first reader that needs it.
fn pack_at(open: &OpenPacks, places: &Places, at: u32) -> Arc<OpenPack> {
    let held = open
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&at)
        .cloned();
    if let Some(pack) = held {
        return pack;
    }
    // Opened outside the lock, so that the other readers read on meanwhile; two that open it at once keep the first.
    let opened = (places.packs.get(at as usize))
        .and_then(|path| TexturePack::open(path).ok())
        .map(|pack| {
            let entries = (pack.entries.iter().enumerate())
                .filter(|(_, e)| e.on_disk)
                .map(|(place, e)| (e.key, place))
                .collect();
            (pack, entries)
        });
    let mut open = open.lock().unwrap_or_else(|p| p.into_inner());
    Arc::clone(open.entry(at).or_insert_with(|| Arc::new(opened)))
}

/// The format a device samples the blocks of `format` as they are stored; `None` for one it decodes.
fn block_format(format: DxgiFormat) -> Option<TextureFormat> {
    Some(match format {
        DxgiFormat::Bc1Unorm => TextureFormat::Bc1RgbaUnorm,
        DxgiFormat::Bc1UnormSrgb => TextureFormat::Bc1RgbaUnormSrgb,
        DxgiFormat::Bc2Unorm => TextureFormat::Bc2RgbaUnorm,
        DxgiFormat::Bc3Unorm => TextureFormat::Bc3RgbaUnorm,
        DxgiFormat::Bc3UnormSrgb => TextureFormat::Bc3RgbaUnormSrgb,
        DxgiFormat::Bc4Unorm => TextureFormat::Bc4RUnorm,
        DxgiFormat::Bc5Unorm => TextureFormat::Bc5RgUnorm,
        // ⚠ Halves past one, as the interior effect's cube of a room's light holds: decoded to eight bits they clip at one.
        DxgiFormat::Bc6hUf16 => TextureFormat::Bc6hRgbUfloat,
        DxgiFormat::Bc6hSf16 => TextureFormat::Bc6hRgbFloat,
        DxgiFormat::Bc7Unorm => TextureFormat::Bc7RgbaUnorm,
        DxgiFormat::Bc7UnormSrgb => TextureFormat::Bc7RgbaUnormSrgb,
        _ => return None,
    })
}

/// The mip of a `width` by `height` texture a screen size of `pixels` asks for: the smallest whose
/// longer side is at least that, the top where none is.
pub(super) fn level_for(width: u32, height: u32, pixels: u32) -> u32 {
    // Each mip halves the side: as many halvings as keep it at or over `pixels`.
    (width.max(height) / pixels.max(1))
        .checked_ilog2()
        .unwrap_or(0)
}

/// `texture`, of one face, as a device takes it: its block rows as they are where a device samples
/// its format and its top level is whole blocks, else every level decoded to RGBA.
pub(super) fn cpu_texture(texture: &Texture) -> Option<CpuTexture> {
    let (width, height) = (texture.width, texture.height);
    // ⚠ A device refuses a block-compressed texture whose first level is not whole blocks.
    let whole = width.is_multiple_of(4) && height.is_multiple_of(4);
    if let Some(format) = block_format(texture.format.dxgi()).filter(|_| whole) {
        let (stored, payload) = (texture.pixel_format(), texture.payload());
        let levels: Vec<Vec<u8>> = (0..texture.levels)
            .map_while(|level| payload.get(stored.mip_level_range(width, height, level)))
            .map(<[u8]>::to_vec)
            .collect();
        return (!levels.is_empty()).then_some(CpuTexture {
            width,
            height,
            format,
            levels,
        });
    }
    let top = decode_level(texture, 0).ok()?;
    let (width, height, srgb) = (top.width, top.height, top.is_srgb);
    let mut levels = vec![top.data];
    for under in 1..texture.levels {
        let Ok(image) = decode_level(texture, under) else {
            break;
        };
        let k = levels.len() as u32;
        if (image.width, image.height) != ((width >> k).max(1), (height >> k).max(1)) {
            break;
        }
        levels.push(image.data);
    }
    Some(CpuTexture {
        width,
        height,
        format: match srgb {
            true => TextureFormat::Rgba8UnormSrgb,
            false => TextureFormat::Rgba8Unorm,
        },
        levels,
    })
}

/// The texture of the key `key` at the level a screen size of `pixels` asks for and every level
/// under it, read from the rows of its pack those levels lie in; where the rows of that level do
/// not decode, from the largest level under it whose rows do; a key no pack holds, from the texture archive's dictionary that holds it. `None` where none holds it or
/// none of it reads.
pub(super) fn read_texture(
    open: &OpenPacks,
    places: &Places,
    key: u64,
    pixels: u32,
) -> Option<CpuTexture> {
    let Some(&at) = places.keys.get(&key) else {
        return dictionary_texture(&places.dictionaries, key);
    };
    let pack = pack_at(open, places, at);
    let (pack, entries) = pack.as_ref().as_ref()?;
    let place = *entries.get(&key)?;
    let entry = pack.entries.get(place)?;
    let level = level_for(entry.width, entry.height, pixels);
    let (_, texture) = pack.texture_from(place, level).ok()?;
    of_faces(&texture)
}

/// `texture` as a device takes it: a cube's six faces stacked down one image in the order a pack
/// stores them, as the interior effect samples them; an array or a volume its first slice.
// ponytail: an array or a volume is read whole for its first slice.
fn of_faces(texture: &Texture) -> Option<CpuTexture> {
    match texture.faces {
        1 => return cpu_texture(texture),
        6 => {}
        _ => return cpu_texture(&texture.face(0)?),
    }
    let faces: Vec<CpuTexture> = (0..6)
        .map(|face| cpu_texture(&texture.face(face)?))
        .collect::<Option<_>>()?;
    let first = &faces[0];
    // ⚠ A block-compressed level under a block a side is stored as a whole block: six of them do not stack into one level.
    let (block, _) = first.format.block_dimensions();
    let levels: Vec<Vec<u8>> = (0..first.levels.len())
        .take_while(|&k| {
            let side = first.width >> k;
            side > 0
                && side.is_multiple_of(block)
                && faces
                    .iter()
                    .all(|f| f.levels.get(k).map(Vec::len) == Some(first.levels[k].len()))
        })
        .map(|k| {
            faces
                .iter()
                .flat_map(|f| f.levels[k].iter().copied())
                .collect()
        })
        .collect();
    (!levels.is_empty()).then(|| CpuTexture {
        width: first.width,
        height: first.height * 6,
        format: first.format,
        levels,
    })
}

/// The texture of the key `key` from the dictionary body of the texture archive that holds it,
/// every level; `None` where none holds it or it does not read.
// ponytail: the body is read again for each key; these dictionaries are few and small.
fn dictionary_texture((archive, bodies): &DictionaryPlaces, key: u64) -> Option<CpuTexture> {
    let offset = *bodies.get(&key)?;
    let file = File::open(archive).ok()?;
    let (find, bytes) = read_rpf9_body(&file, offset, BODY_LIMIT).ok()?;
    let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(resource),
        ..
    }) = find.kind
    else {
        return None;
    };
    let (virtual_section, physical) = resource.sections(&bytes);
    let dictionary = read_dictionary(virtual_section, Some(physical)).ok()?;
    let texture = dictionary
        .entries
        .iter()
        .find_map(|(_, entry)| match entry {
            Ok(Entry::Texture(t)) if name_hash64(t.name.as_bytes()) == key => {
                t.texture.as_ref().ok()
            }
            _ => None,
        })?;
    of_faces(texture)
}

#[cfg(test)]
#[path = "texture_tests.rs"]
mod tests;
