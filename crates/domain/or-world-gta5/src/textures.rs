//! The textures the surfaces of GTA V's models ask for, in batches: one named in a texture
//! dictionary is looked for in that dictionary and then in each it falls through to, as the
//! install's relationship files state, and one a model carries is read from the model's own
//! dictionary. Each file is opened once a batch.
//!
//! ⚠ A level's bytes are the file's: nothing is decoded, so a texture in a format the core's
//! texture has no word for does not read and says so.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};

use crate::WorldSource;
use or_io::codec::texture::{GrcTextureFormat, TextureSource};
use or_io::decl::{File, Gen8, Gen9};
use or_io::gta5::gen8::GrcTexture;
use or_io::gta5::gen9::GrcTextureG9;
use or_io::gta5::retarget::layout_of;
use or_io::gta5::PgDictionary;
use or_world::model::{AssetKey, Index, Texture, TextureAsk, TextureFormat};
use or_world::sentence;
use or_world::sentence::Sentence;

use crate::index::{linked, Holding};
use crate::install::Gta5World;
use crate::models::{drawables_of, held_at, NoModel, Opened, Target};

/// The bit of a scope that says it names the dictionary a model carries, by the name of the model's archetype in its low 32 bits; without it a scope is the name of a texture dictionary.
pub const EMBEDDED: u64 = 1 << 33;

/// Why a texture does not read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NoTexture {
    #[error("its scope names no texture dictionary")]
    NoDictionary,
    #[error("none of the {walked} dictionaries of its chain holds it: the mount holds no file of {absent} of them and {unread} do not read")]
    NotInChain {
        walked: u32,
        absent: u32,
        unread: u32,
    },
    #[error("the model that carries it does not read: {0}")]
    Model(NoModel),
    #[error("the model's own dictionary holds no such texture")]
    NotEmbedded,
    #[error("its format, {0}, is one the texture has no word for")]
    Format(String),
    #[error("it is a cube or a volume")]
    NotFlat,
    #[error("its pixels are stored in a tiled order")]
    Tiled,
    #[error("its bytes hold no whole level")]
    PartLevel,
}

impl NoTexture {
    /// The reason as a sentence the catalogs hold, without what a file said of itself.
    pub fn said(&self) -> Sentence {
        match self {
            Self::NoDictionary => sentence!("Its scope names no texture dictionary"),
            Self::NotInChain { unread: 1.., .. } => {
                sentence!("A dictionary of its chain does not read")
            }
            Self::NotInChain { absent: 1.., .. } => {
                sentence!("The install lacks a dictionary of its chain")
            }
            Self::NotInChain { .. } => sentence!("No dictionary of its chain holds it"),
            Self::Model(_) => sentence!("The model that carries it does not read"),
            Self::NotEmbedded => sentence!("The model's own dictionary lacks it"),
            Self::Format(_) => sentence!("Its format is one the texture has no word for"),
            Self::NotFlat => sentence!("It is a cube or a volume"),
            Self::Tiled => sentence!("Its pixels are stored in a tiled order"),
            Self::PartLevel => sentence!("Its bytes hold no whole level"),
        }
    }

    /// [`Self::said`] as its English text, which a count of reasons is kept by.
    pub fn kind(&self) -> &'static str {
        self.said().key()
    }
}

/// A texture of either target as the contract's, its levels the file's own bytes; a level above `pixels` a side is passed over while a smaller one is stored.
pub(crate) fn texture_of<S: TextureSource>(texture: &S, pixels: u32) -> Result<Texture, NoTexture> {
    let stated = texture.pixel_format();
    let format = match stated {
        GrcTextureFormat::DXT1 => TextureFormat::Bc1,
        GrcTextureFormat::DXT5 => TextureFormat::Bc3,
        GrcTextureFormat::DXT5A => TextureFormat::Bc4,
        GrcTextureFormat::DXN => TextureFormat::Bc5,
        GrcTextureFormat::BC7 => TextureFormat::Bc7,
        GrcTextureFormat::A8B8G8R8 => TextureFormat::Rgba8,
        other => return Err(NoTexture::Format(format!("{other:?}"))),
    };
    let (width, height) = texture.stored_size();
    let payload = texture.payload();
    let count = texture.stored_levels().max(1);
    let level = |at: u32| {
        let held = payload.get(stated.mip_level_range(width, height, at));
        held.filter(|_| at < count)
    };
    let (mut levels, mut first) = (Vec::new(), (0, 0));
    for at in 0..count {
        let Some(bytes) = level(at) else { break };
        let (wide, high) = stated.mip_dimensions(width, height, at);
        if levels.is_empty() {
            if wide.max(high) > pixels && level(at + 1).is_some() {
                continue;
            }
            first = (wide, high);
        }
        levels.push(bytes.to_vec());
    }
    let whole = |bytes: &Vec<u8>| bytes.len() == format.level_bytes(first.0, first.1);
    if !levels.first().is_some_and(whole) {
        return Err(NoTexture::PartLevel);
    }
    Ok(Texture {
        width: first.0,
        height: first.1,
        format,
        srgb: texture.is_srgb(),
        levels,
    })
}

/// The texture named `name` of `dictionary`, where it holds one.
fn held_in<T: Target>(
    dictionary: &PgDictionary<T::Texture>,
    name: u64,
    pixels: u32,
) -> Option<Result<Texture, NoTexture>> {
    let at = (dictionary.codes.iter()).position(|code| u64::from(*code) == name)?;
    let texture = dictionary.entries.get(at)?.as_deref()?;
    Some(T::texture(texture, pixels))
}

/// A texture dictionary that is a file of its own, of either target.
enum Dictionary {
    Gen8(PgDictionary<GrcTexture>),
    Gen9(PgDictionary<GrcTextureG9>),
}

impl Dictionary {
    /// `bytes` read by the layout its own RSC7 version names.
    fn read(bytes: &[u8]) -> Result<Self, String> {
        let refused = |e: or_io::Error| e.to_string();
        match layout_of("ytd", bytes)?.is_gen9() {
            true => <PgDictionary<GrcTextureG9> as File>::from_bytes(bytes)
                .map(Self::Gen9)
                .map_err(refused),
            false => <PgDictionary<GrcTexture> as File>::from_bytes(bytes)
                .map(Self::Gen8)
                .map_err(refused),
        }
    }

    fn texture(&self, name: u64, pixels: u32) -> Option<Result<Texture, NoTexture>> {
        match self {
            Self::Gen8(dictionary) => held_in::<Gen8>(dictionary, name, pixels),
            Self::Gen9(dictionary) => held_in::<Gen9>(dictionary, name, pixels),
        }
    }
}

/// The asks of the textures models carry, by the file that holds each model, each with the name its model has in a dictionary.
type Carried<'a> = BTreeMap<(usize, &'a str), (Holding, Vec<(TextureAsk, u32)>)>;

impl Gta5World {
    /// The texture of each of `asks`, or why it does not read; each file that holds any is opened once.
    pub fn read_textures(
        &self,
        index: &Index,
        asks: &[TextureAsk],
        out: &mut Vec<(TextureAsk, Result<Texture, NoTexture>)>,
    ) -> Opened {
        let mut opened = Opened::default();
        let Ok(linked) = linked(index) else {
            out.extend(asks.iter().map(|ask| (*ask, Err(NoTexture::NoDictionary))));
            return opened;
        };
        // Each dictionary of the batch, read once: `Err(true)` for one the mount lacks and `Err(false)` for one that does not read.
        let mut dictionaries: HashMap<u32, Result<Dictionary, bool>> = HashMap::new();
        let mut carried: Carried<'_> = BTreeMap::new();
        for ask in asks {
            if ask.scope.0 & EMBEDDED != 0 {
                match held_at(linked, AssetKey(ask.scope.0 & !EMBEDDED)) {
                    Ok(at) => {
                        let file = carried.entry((at.archive, at.logical));
                        let (_, wanted) = file.or_insert_with(|| (at.holding, Vec::new()));
                        wanted.push((*ask, at.entry));
                    }
                    Err(why) => out.push((*ask, Err(NoTexture::Model(why)))),
                }
                continue;
            }
            let Some(first) = u32::try_from(ask.scope.0).ok().filter(|name| *name != 0) else {
                out.push((*ask, Err(NoTexture::NoDictionary)));
                continue;
            };
            let (mut walked, mut absent, mut unread, mut found) = (Vec::new(), 0, 0, None);
            let mut next = Some(first);
            // ⚠ The relationship files are data: a chain that comes back to a dictionary stops there.
            while let Some(name) = next.filter(|name| !walked.contains(name)) {
                walked.push(name);
                if let Entry::Vacant(unread_yet) = dictionaries.entry(name) {
                    let file = linked.texture_file(name).ok_or(true);
                    let read = file.and_then(|(archive, logical)| {
                        let bytes = self.source.open(archive, logical).ok_or(true)?;
                        opened.files += 1;
                        opened.bytes += bytes.len() as u64;
                        Dictionary::read(&bytes).map_err(|why| {
                            tracing::debug!(file = logical, %why, "a texture dictionary does not read");
                            false
                        })
                    });
                    unread_yet.insert(read);
                }
                match dictionaries.get(&name) {
                    Some(Ok(dictionary)) => found = dictionary.texture(ask.name, ask.pixels),
                    Some(Err(false)) => unread += 1,
                    _ => absent += 1,
                }
                if found.is_some() {
                    break;
                }
                next = linked.texture_parent(name);
            }
            let walked = walked.len() as u32;
            out.push((
                *ask,
                found.unwrap_or(Err(NoTexture::NotInChain {
                    walked,
                    absent,
                    unread,
                })),
            ));
        }
        for ((archive, logical), (holding, wanted)) in carried {
            let Some(bytes) = self.source.open(archive, logical) else {
                let absent =
                    |(ask, _): &(TextureAsk, u32)| (*ask, Err(NoTexture::Model(NoModel::Absent)));
                out.extend(wanted.iter().map(absent));
                continue;
            };
            opened.files += 1;
            opened.bytes += bytes.len() as u64;
            let target = match layout_of(holding.extension(), &bytes) {
                Ok(target) => target,
                Err(why) => {
                    let version = |(ask, _): &(TextureAsk, u32)| {
                        (*ask, Err(NoTexture::Model(NoModel::Version(why.clone()))))
                    };
                    out.extend(wanted.iter().map(version));
                    continue;
                }
            };
            let entry = |(_, entry): &(TextureAsk, u32)| *entry;
            if target.is_gen9() {
                drawables_of::<Gen9, _>(holding, &bytes, &wanted, entry, |(ask, _), drawable| {
                    out.push((*ask, carried_by::<Gen9>(drawable, ask)));
                });
            } else {
                drawables_of::<Gen8, _>(holding, &bytes, &wanted, entry, |(ask, _), drawable| {
                    out.push((*ask, carried_by::<Gen8>(drawable, ask)));
                });
            }
        }
        opened
    }
}

/// The texture `ask` names in the dictionary `drawable` carries.
fn carried_by<T: Target>(
    drawable: Result<&or_io::gta5::RmcDrawable<T>, NoModel>,
    ask: &TextureAsk,
) -> Result<Texture, NoTexture> {
    let group = drawable.map_err(NoTexture::Model)?.shader_group.as_deref();
    let dictionary = group.and_then(|group| group.texture_dictionary.as_deref());
    let held = dictionary.and_then(|held| held_in::<T>(held, ask.name, ask.pixels));
    held.unwrap_or(Err(NoTexture::NotEmbedded))
}
