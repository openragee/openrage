//! Editing a texture that is already in a document: the document's half of
//! `or_io::authoring` and of the owned drawable's embedding.
//!
//! ⚠ **Gen8 only, as a type constraint.** A Legacy `.ytd` and a Legacy
//! drawable hold the owned `GrcTexture`, and an Enhanced dictionary holds
//! `GrcTextureG9` — a different type with a different field set.

use or_io::authoring::{FormatChoice, TextureBuildOptions};
use or_io::codec::texture::{decode_level, GrcTextureFormat, RgbaImage};
use or_io::decl::{Gen8, Heap};
use or_io::gta5::{gen8, GrmShaderGroup, PgDictionary};

use super::document::{DocPayload, DocumentId};
use super::edit_history::{Change, Swap};
use super::memory::vec_bytes;
use super::textures::{self, TextureSource};
use super::AppState;

/// New pixels over `texture`, which keeps its name and usage classification; the mip count it now has.
fn replace(
    texture: &mut gen8::GrcTexture,
    format: GrcTextureFormat,
    image: &RgbaImage,
    options: &TextureBuildOptions,
) -> Result<u32, String> {
    texture
        .set_pixels(format, image, options)
        .map_err(|e| e.to_string())?;
    Ok(u32::from(texture.mip_count))
}

/// What an edit changed, for the toast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextureEdit {
    Reencoded {
        name: String,
        format: GrcTextureFormat,
        mips: u32,
    },
    Imported {
        name: String,
        format: GrcTextureFormat,
        mips: u32,
        /// It went over a texture that was already there.
        replaced: bool,
    },
    Embedded {
        name: String,
        replaced: bool,
    },
    Unembedded {
        name: String,
        /// Materials that now need an external dictionary.
        orphaned: usize,
    },
}

impl TextureEdit {
    pub fn describe(&self) -> String {
        match self {
            Self::Reencoded { name, format, mips } => {
                tr!(
                    "{name} is now {format} with {mips} mip level(s)",
                    name = name,
                    format = format!("{format:?}"),
                    mips = mips
                )
            }
            Self::Imported {
                name,
                format,
                mips,
                replaced,
            } => {
                let format = format!("{format:?}");
                if *replaced {
                    tr!(
                        "{name} replaced as {format} with {mips} mip level(s)",
                        name = name,
                        format = format,
                        mips = mips
                    )
                } else {
                    tr!(
                        "{name} added as {format} with {mips} mip level(s)",
                        name = name,
                        format = format,
                        mips = mips
                    )
                }
            }
            Self::Embedded {
                name,
                replaced: true,
            } => {
                tr!("{name} replaced the embedded copy", name = name)
            }
            Self::Embedded { name, .. } => tr!("{name} is embedded in this drawable", name = name),
            Self::Unembedded { name, orphaned: 0 } => {
                tr!("{name} is no longer carried by this drawable", name = name)
            }
            Self::Unembedded { name, orphaned } => tr!(
                "{name} is no longer carried; {orphaned} material(s) now need an \
                 external dictionary",
                name = name,
                orphaned = orphaned
            ),
        }
    }
}

/// Every Legacy dictionary a document holds or embeds that can be edited in place.
fn dictionary_mut(
    payload: &mut DocPayload,
    source: TextureSource,
) -> Option<&mut PgDictionary<gen8::GrcTexture>> {
    match payload {
        DocPayload::Resource(or_io::Asset::TextureDict(dict)) => Some(dict),
        DocPayload::Resource(or_io::Asset::Drawable(_) | or_io::Asset::Fragment(_)) => {
            shader_group_mut(payload)?.texture_dictionary.as_deref_mut()
        }
        DocPayload::Resource(or_io::Asset::DrawableDict(dict)) => {
            // ⚠ A dictionary document's entries each carry their own embedded
            // dictionary, so the entry has to be named. `TextureSource::Root`
            // would be ambiguous across entries that share a texture name.
            let TextureSource::Entry { hash } = source else {
                return None;
            };
            let at = dict.codes.iter().position(|h| *h == hash)?;
            (dict.entries.get_mut(at)?.as_deref_mut()?)
                .shader_group
                .as_deref_mut()?
                .texture_dictionary
                .as_deref_mut()
        }
        _ => None,
    }
}

/// The entry named `name` in the dictionary an edit reaches; `None` when the document has none, `Err` when it lacks the name.
fn texture_mut<'a>(
    payload: &'a mut DocPayload,
    source: TextureSource,
    name: &str,
) -> Option<Result<&'a mut gen8::GrcTexture, String>> {
    if let DocPayload::Dds(texture) = payload {
        return Some(Ok(texture));
    }
    Some(
        dictionary_mut(payload, source)?
            .get_mut(name)
            .ok_or_else(|| tr!("{name} is not in this dictionary", name = name)),
    )
}

const NOT_EDITABLE: &str = tr_key!(
    "This document has no editable Legacy texture dictionary. Enhanced \
                            textures use a different layout and cannot be re-encoded here."
);

pub(crate) const PACK_KEEPS_ITS_FORMAT: &str = tr_key!(
    "A GTA VI pack's texture keeps its format, size and mips; replace it from an image instead"
);

const NOT_A_LEGACY_DICTIONARY: &str = tr_key!(
    "Only a Legacy .ytd can take a new texture. An Enhanced \
                                       dictionary holds GrcTextureG9, which this encoder does \
                                       not produce."
);

/// A texture and every byte it stores.
fn texture_bytes(texture: &gen8::GrcTexture) -> usize {
    std::mem::size_of::<gen8::GrcTexture>() + texture.heap_bytes()
}

/// A `.ytd` entry an import added, held while it is out of the dictionary.
struct NewEntry {
    name: String,
    other: Option<gen8::GrcTexture>,
}

impl Swap for NewEntry {
    fn held_bytes(&self) -> usize {
        self.name.capacity() + self.other.as_ref().map_or(0, texture_bytes)
    }

    fn swap(&mut self, payload: &mut DocPayload) -> bool {
        let Some(dictionary) = textures::legacy_dictionary_mut(payload) else {
            return false;
        };
        if let Some(texture) = self.other.take() {
            // ⚠ A name in use is not replaced: a part that says false has changed nothing
            if dictionary.get(&texture.name).is_some() {
                self.other = Some(texture);
                return false;
            }
            return dictionary.insert(texture).is_none();
        }
        self.other = take(dictionary, &self.name);
        self.other.is_some()
    }
}

/// The entry `name`, taken out of `dictionary`.
fn take(dictionary: &mut PgDictionary<gen8::GrcTexture>, name: &str) -> Option<gen8::GrcTexture> {
    let code = or_io::hash::string_hash_str(name);
    let at = dictionary.codes.iter().position(|c| *c == code)?;
    let texture = dictionary.entries.get_mut(at)?.take();
    dictionary.codes.remove(at);
    dictionary.entries.remove(at);
    texture.map(|texture| *texture)
}

/// What a drawable carried under one name, and its materials, before an embed or an unembed rebound them.
struct Carried {
    name: String,
    /// Whether it carried a dictionary at all: an unembed of its last texture drops it.
    dictionary: bool,
    texture: Option<gen8::GrcTexture>,
    shaders: Vec<gen8::GrcInstanceData>,
}

impl Carried {
    fn of(group: &GrmShaderGroup<Gen8>, name: &str) -> Self {
        let dictionary = group.texture_dictionary.as_deref();
        Self {
            name: name.to_string(),
            dictionary: dictionary.is_some(),
            texture: dictionary.and_then(|d| d.get(name)).cloned(),
            shaders: group
                .shaders
                .iter()
                .map(|shader| (**shader).clone())
                .collect(),
        }
    }
}

impl Swap for Carried {
    fn held_bytes(&self) -> usize {
        let shaders: usize = self.shaders.iter().map(Heap::heap_bytes).sum();
        let texture = self.texture.as_ref().map_or(0, texture_bytes);
        self.name.capacity() + texture + vec_bytes(&self.shaders) + shaders
    }

    fn swap(&mut self, payload: &mut DocPayload) -> bool {
        let Some(group) = shader_group_mut(payload) else {
            return false;
        };
        let dictionary = group.texture_dictionary.is_some();
        let texture = (group.texture_dictionary.as_deref_mut()).and_then(|d| take(d, &self.name));
        if let Some(held) = self.texture.take() {
            group
                .texture_dictionary
                .get_or_insert_with(Default::default)
                .insert(held);
        }
        let empty = (group.texture_dictionary.as_deref()).is_some_and(|d| d.codes.is_empty());
        if self.dictionary {
            group
                .texture_dictionary
                .get_or_insert_with(Default::default);
        } else if empty {
            group.texture_dictionary = None;
        }
        let held = std::mem::take(&mut self.shaders).into_iter().map(Box::new);
        let now = std::mem::replace(&mut group.shaders, held.collect());
        self.shaders = now.into_iter().map(|shader| *shader).collect();
        (self.dictionary, self.texture) = (dictionary, texture);
        true
    }
}

/// The step that puts back `before`, the texture `name` held until an edit replaced its pixels.
fn replaced(source: TextureSource, name: &str, before: gen8::GrcTexture) -> (String, Change) {
    let place = name.to_string();
    let change = Change::part(format!("texture {name}"), before, move |p| {
        texture_mut(p, source, &place)?.ok()
    });
    (tr!("Replace {name}", name = name), change)
}

impl AppState {
    fn after_texture_edit(&mut self, doc: DocumentId, (label, change): (String, Change)) {
        self.record(doc, label, change);
        // ⚠ The decoded preview is keyed by name and the pixels behind it have
        // just changed, so leaving it cached shows the old encoding until the
        // document is closed.
        self.texture_cache.forget_document(doc);
    }

    /// Whether an in-place edit can reach this document's dictionary at all.
    pub fn texture_dictionary_is_editable(
        &mut self,
        doc: DocumentId,
        source: TextureSource,
    ) -> bool {
        self.document_mut(doc).is_some_and(|document| {
            let scanned = super::gta6_edit::is_scanned_dictionary(document);
            scanned
                || document.read_only.is_none()
                    && (matches!(document.payload, DocPayload::Dds(_) | DocPayload::Ttpf(_))
                        || dictionary_mut(&mut document.payload, source).is_some())
        })
    }

    /// Change a texture's pixel format, compression quality and mip count
    /// without moving it out of the dictionary it is in.
    pub fn reencode_texture_in_place(
        &mut self,
        doc: DocumentId,
        source: TextureSource,
        name: &str,
        format: FormatChoice,
        options: &TextureBuildOptions,
    ) -> Result<TextureEdit, String> {
        let pack = (self.document_mut(doc)).and_then(|d| {
            super::ttpf_document::reshape(&mut d.payload, name, None, format, options)
        });
        if let Some(reshaped) = pack {
            let (step, edit) = reshaped?;
            self.after_texture_edit(doc, step);
            return Ok(edit);
        }
        let scanned = (self.document_mut(doc))
            .and_then(|d| super::gta6_dictionary_write::reencode(d, name, format, options));
        if let Some(outcome) = scanned {
            let (before, edit) = outcome?;
            self.after_texture_edit(doc, replaced(source, name, before));
            return Ok(edit);
        }
        let Some(texture) = self.texture_for_edit(doc, source, name) else {
            return Err(crate::i18n::tr(NOT_EDITABLE).to_string());
        };
        let texture = texture?;
        let before = texture.clone();

        let rgba = decode_level(&*texture, 0).map_err(|e| e.to_string())?;
        let format = format.resolve(&rgba);
        let mips = replace(texture, format, &rgba, options)?;

        self.after_texture_edit(doc, replaced(source, name, before));
        Ok(TextureEdit::Reencoded {
            name: name.to_string(),
            format,
            mips,
        })
    }

    /// Put an image file's pixels over a texture already in the dictionary.
    ///
    /// ⚠ The entry keeps its name **and its usage classification**: importing
    /// pixels is not a route to declaring a texture something the game treats
    /// differently.
    pub fn import_texture_over(
        &mut self,
        doc: DocumentId,
        source: TextureSource,
        name: &str,
        image: &RgbaImage,
        format: FormatChoice,
        options: &TextureBuildOptions,
    ) -> Result<TextureEdit, String> {
        let pack = (self.document_mut(doc)).and_then(|d| {
            super::ttpf_document::import(&mut d.payload, name, image, format, options)
        });
        if let Some(replaced) = pack {
            let (step, edit) = replaced?;
            self.after_texture_edit(doc, step);
            return Ok(edit);
        }
        let scanned = (self.document_mut(doc))
            .and_then(|d| super::gta6_dictionary_write::import(d, name, image, format, options));
        if let Some(outcome) = scanned {
            let (before, edit) = outcome?;
            self.after_texture_edit(doc, replaced(source, name, before));
            return Ok(edit);
        }
        let Some(texture) = self.texture_for_edit(doc, source, name) else {
            return Err(crate::i18n::tr(NOT_EDITABLE).to_string());
        };
        let texture = texture?;
        let before = texture.clone();

        let format = format.resolve(image);
        let mips = replace(texture, format, image, options)?;

        self.after_texture_edit(doc, replaced(source, name, before));
        Ok(TextureEdit::Imported {
            name: name.to_string(),
            format,
            mips,
            replaced: true,
        })
    }

    /// Add an image file to a `.ytd` as a new entry.
    ///
    /// ⚠ Refuses a name the dictionary already holds rather than replacing it.
    /// A `PgDictionary` will happily carry two entries under one hash, and the
    /// second is unreachable by name — [`Self::import_texture_over`] is the
    /// operation that means *replace*.
    pub fn import_texture_as_new(
        &mut self,
        doc: DocumentId,
        name: &str,
        image: &RgbaImage,
        format: FormatChoice,
        options: &TextureBuildOptions,
    ) -> Result<TextureEdit, String> {
        if name.trim().is_empty() {
            return Err(tr!("A texture needs a name").to_string());
        }
        let Some(document) = self.document_mut(doc) else {
            return Err(tr!("That document is no longer open").to_string());
        };
        let scanned = super::gta6_edit::is_scanned_dictionary(document);
        if !scanned {
            if let Some(why) = document.read_only {
                return Err(crate::i18n::tr(why).to_string());
            }
        }
        let Some(dictionary) = textures::legacy_dictionary_mut(&mut document.payload) else {
            return Err(crate::i18n::tr(NOT_A_LEGACY_DICTIONARY).to_string());
        };
        if dictionary.get(name).is_some() {
            return Err(tr!(
                "{name} is already in this dictionary; replace it from an image instead",
                name = name
            ));
        }

        let format = format.resolve(image);
        let mut texture = gen8::GrcTexture {
            name: name.to_string(),
            resource_type_and_conversion_flags: or_io::gta5::texture_dict::gen8::PROCESSED,
            ..Default::default()
        };
        texture
            .set_pixels(format, image, options)
            .map_err(|e| e.to_string())?;
        if scanned {
            super::gta6_dictionary_write::can_add(&texture)?;
        }
        let mips = u32::from(texture.mip_count);
        dictionary.insert(texture);

        let added = NewEntry {
            name: name.to_string(),
            other: None,
        };
        let change = Change::Part {
            key: None,
            parts: vec![Box::new(added)],
        };
        self.after_texture_edit(doc, (tr!("Add texture {name}", name = name), change));
        Ok(TextureEdit::Imported {
            name: name.to_string(),
            format,
            mips,
            replaced: false,
        })
    }

    /// The dictionary entry an edit is about, or why it cannot be reached.
    ///
    /// The outer `None` is "no editable dictionary here", the inner `Err` is
    /// "this dictionary does not hold that name".
    fn texture_for_edit(
        &mut self,
        doc: DocumentId,
        source: TextureSource,
        name: &str,
    ) -> Option<Result<&mut gen8::GrcTexture, String>> {
        let document = self.document_mut(doc).filter(|d| d.read_only.is_none())?;
        texture_mut(&mut document.payload, source, name)
    }

    /// Carry a texture in the drawable itself, under its own name.
    pub fn embed_texture(
        &mut self,
        doc: DocumentId,
        texture: gen8::GrcTexture,
    ) -> Result<TextureEdit, String> {
        let Some(document) = self.document_mut(doc) else {
            return Err(tr!("That document is no longer open").to_string());
        };
        let Some(group) = shader_group_mut(&mut document.payload) else {
            return Err(tr!("Only a drawable or fragment can carry textures").to_string());
        };
        let name = texture.name.clone();
        let before = Carried::of(group, &name);
        let replaced = group.embed(texture).is_some();

        let change = Change::Part {
            key: None,
            parts: vec![Box::new(before)],
        };
        self.after_texture_edit(doc, (tr!("Embed {name}", name = name), change));
        Ok(TextureEdit::Embedded { name, replaced })
    }

    /// Stop carrying a texture, leaving every material's reference to it.
    pub fn unembed_texture(&mut self, doc: DocumentId, name: &str) -> Result<TextureEdit, String> {
        let Some(document) = self.document_mut(doc) else {
            return Err(tr!("That document is no longer open").to_string());
        };
        let Some(group) = shader_group_mut(&mut document.payload) else {
            return Err(tr!("Only a drawable or fragment can carry textures").to_string());
        };
        let before = Carried::of(group, name);
        if group.unembed(name).is_none() {
            return Err(tr!("{name} is not carried by this drawable", name = name));
        }
        let orphaned = group.users(name).len();

        let change = Change::Part {
            key: None,
            parts: vec![Box::new(before)],
        };
        self.after_texture_edit(doc, (tr!("Stop carrying {name}", name = name), change));
        Ok(TextureEdit::Unembedded {
            name: name.to_string(),
            orphaned,
        })
    }
}

fn shader_group_mut(payload: &mut DocPayload) -> Option<&mut GrmShaderGroup<Gen8>> {
    match payload {
        DocPayload::Resource(or_io::Asset::Drawable(d)) => d.base.shader_group.as_deref_mut(),
        DocPayload::Resource(or_io::Asset::Fragment(frag)) => {
            (frag.common_drawable.as_deref_mut()?)
                .base
                .shader_group
                .as_deref_mut()
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Document;
    use or_io::codec::texture::RgbaImage;

    /// The low two bits of `resource_type_and_conversion_flags`: 0 a normal texture, 1 a render target.
    const RESOURCE_TYPE: u8 = 0x03;
    const RENDER_TARGET: u8 = 1;

    /// A four-by-four with one transparent pixel, so `Automatic` has
    /// something to decide.
    fn source(alpha: u8) -> RgbaImage {
        let mut image = RgbaImage::new(4, 4);
        for y in 0..4 {
            for x in 0..4 {
                image.set_pixel(x, y, [(x * 60) as u8, (y * 60) as u8, 8, 255]);
            }
        }
        image.set_pixel(0, 0, [255, 0, 0, alpha]);
        image
    }

    /// A four-by-four A8R8G8B8 texture named `paint`.
    fn paint() -> gen8::GrcTexture {
        gen8::GrcTexture {
            name: "paint".to_string(),
            width: 4,
            height: 4,
            depth: 1,
            format: GrcTextureFormat::A8R8G8B8.to_raw_format(),
            mip_count: 1,
            backing_store: vec![0; 4 * 4 * 4],
            ..Default::default()
        }
    }

    fn dictionary_document(state: &mut AppState, texture: gen8::GrcTexture) -> DocumentId {
        let mut dictionary = or_io::gta5::PgDictionary::default();
        dictionary.insert(texture);
        let id = state.documents.len() as DocumentId + 1;
        state.documents.push(Document::new(
            id,
            Some(std::path::PathBuf::from("C:/a/paint.ytd")),
            DocPayload::Resource(or_io::Asset::TextureDict(dictionary)),
        ));
        id
    }

    fn resource_type_of(state: &AppState, doc: DocumentId, name: &str) -> u8 {
        let dictionary = state
            .document(doc)
            .and_then(|d| textures::legacy_dictionary(&d.payload))
            .expect("the document is a Legacy .ytd");
        dictionary
            .get(name)
            .expect("the entry")
            .resource_type_and_conversion_flags
    }

    /// ⚠ A `.ytd` entry that declares itself a render target writes a file the
    /// game reads as one, so replacing pixels must neither set nor clear that
    /// classification.
    #[test]
    fn importing_pixels_leaves_the_usage_classification_alone() {
        let mut state = AppState::new();
        let mut texture = paint();
        texture.resource_type_and_conversion_flags = RENDER_TARGET;
        let declared = texture.resource_type_and_conversion_flags;
        let doc = dictionary_document(&mut state, texture);

        state
            .import_texture_over(
                doc,
                TextureSource::Document,
                "paint",
                &source(0x40),
                FormatChoice::Automatic,
                &TextureBuildOptions::default(),
            )
            .expect("the entry is there and 4x4 encodes");

        assert_eq!(
            resource_type_of(&state, doc, "paint"),
            declared,
            "importing pixels rewrote what the header declares this texture is"
        );
    }

    #[test]
    fn table_texture_previews_refuse_import_and_gta5_conversion() {
        let mut state = AppState::from_prefs(Default::default());
        let id = dictionary_document(&mut state, paint());
        state.document_mut(id).expect("document").read_only =
            Some(super::super::document::rpf9::entry::READ_ONLY);
        assert!(!state.texture_dictionary_is_editable(id, TextureSource::Document));
        assert!(state
            .import_texture_as_new(
                id,
                "decal",
                &source(0x40),
                FormatChoice::Automatic,
                &TextureBuildOptions::default(),
            )
            .is_err());
        assert!(state
            .import_texture_over(
                id,
                TextureSource::Document,
                "paint",
                &source(0x40),
                FormatChoice::Automatic,
                &TextureBuildOptions::default(),
            )
            .is_err());
        assert!(state
            .reencode_texture_in_place(
                id,
                TextureSource::Document,
                "paint",
                FormatChoice::Automatic,
                &TextureBuildOptions::default(),
            )
            .is_err());
        assert!(state.document_conversion_to(id).is_err());
        state.convert_document(id);
        assert!(state.convert.job().is_none());
        let document = state.document(id).expect("document");
        assert!(!document.dirty);
        let dictionary = textures::legacy_dictionary(&document.payload).expect("dictionary");
        assert!(dictionary.get("paint").is_some());
        assert!(dictionary.get("decal").is_none());
    }

    #[test]
    fn an_imported_texture_is_a_normal_one() {
        let mut state = AppState::new();
        let doc = dictionary_document(&mut state, paint());

        state
            .import_texture_as_new(
                doc,
                "decal",
                &source(0x40),
                FormatChoice::Automatic,
                &TextureBuildOptions::default(),
            )
            .expect("a new name in a Legacy dictionary");

        assert_eq!(resource_type_of(&state, doc, "decal") & RESOURCE_TYPE, 0);
    }

    #[test]
    fn automatic_picks_the_block_format_the_alpha_channel_asks_for() {
        let mut state = AppState::new();
        let doc = dictionary_document(&mut state, paint());

        let import = |state: &mut AppState, name: &str, alpha: u8| {
            state
                .import_texture_as_new(
                    doc,
                    name,
                    &source(alpha),
                    FormatChoice::Automatic,
                    &TextureBuildOptions::default(),
                )
                .expect("a new name")
        };

        assert!(matches!(
            import(&mut state, "opaque", 0xFF),
            TextureEdit::Imported {
                format: GrcTextureFormat::DXT1,
                ..
            }
        ));
        assert!(matches!(
            import(&mut state, "masked", 0x00),
            TextureEdit::Imported {
                format: GrcTextureFormat::DXT5,
                ..
            }
        ));
    }

    #[test]
    fn adding_a_name_the_dictionary_already_holds_is_refused() {
        let mut state = AppState::new();
        let doc = dictionary_document(&mut state, paint());

        let error = state
            .import_texture_as_new(
                doc,
                "PAINT",
                &source(0xFF),
                FormatChoice::Automatic,
                &TextureBuildOptions::default(),
            )
            .unwrap_err();
        assert!(
            error.contains("replace it from an image"),
            "unhelpful error: {error}"
        );
    }

    #[test]
    fn an_enhanced_dictionary_refuses_with_a_reason() {
        let mut state = AppState::new();
        let id = 1 as DocumentId;
        state.documents.push(Document::new(
            id,
            Some(std::path::PathBuf::from("C:/a/paint.ytd")),
            DocPayload::Resource(or_io::Asset::TextureDictGen9(Default::default())),
        ));

        assert!(!state.texture_dictionary_is_editable(id, TextureSource::Document));
        let error = state
            .import_texture_as_new(
                id,
                "decal",
                &source(0xFF),
                FormatChoice::Automatic,
                &TextureBuildOptions::default(),
            )
            .unwrap_err();
        assert!(error.contains("GrcTextureG9"), "unhelpful error: {error}");
    }

    /// A texture authored into a blank Legacy dictionary saves with no pack and reopens with the pixels it was given.
    #[test]
    fn a_texture_authored_into_a_blank_dictionary_saves_and_reopens_with_no_pack() {
        use or_io::gta5::File as _;

        let mut state = AppState::new();
        let id = 1 as DocumentId;
        state.documents.push(Document::new(
            id,
            Some(std::path::PathBuf::from("C:/a/new.ytd")),
            DocPayload::Resource(or_io::Asset::TextureDict(Default::default())),
        ));
        let options = TextureBuildOptions::default();
        state
            .import_texture_as_new(
                id,
                "decal",
                &source(0x00),
                FormatChoice::Automatic,
                &options,
            )
            .expect("a new name in a blank dictionary");

        let payload = &state.document(id).expect("still open").payload;
        let saved = crate::state::document_io::serialize_payload(
            payload,
            crate::state::prefs::RpfOutputMode::Open,
            std::sync::Arc::new(or_io::container::NoOpCryptoProvider::new()),
        )
        .expect("a Legacy .ytd saves with no pack");
        let reopened = or_io::gta5::PgDictionary::<gen8::GrcTexture>::from_bytes(&saved)
            .expect("the saved file reopens");

        let authored = textures::legacy_dictionary(payload)
            .and_then(|d| d.get("decal"))
            .expect("the authored entry");
        let read = reopened.get("decal").expect("the entry survives the save");
        assert_eq!(read.format, GrcTextureFormat::DXT5.to_raw_format());
        assert_eq!(
            decode_level(read, 0).expect("decodes").data,
            decode_level(authored, 0).expect("decodes").data,
            "the saved pixels are not the ones authored"
        );
    }

    /// What saving `doc` would write now.
    fn written(state: &AppState, doc: DocumentId) -> Vec<u8> {
        crate::state::document_io::serialize_payload(
            &state.document(doc).expect("open").payload,
            crate::state::prefs::RpfOutputMode::Open,
            std::sync::Arc::new(or_io::container::NoOpCryptoProvider::new()),
        )
        .expect("a Legacy .ytd saves")
    }

    #[test]
    fn a_reencode_and_an_import_undo_to_the_dictionary_as_it_opened() {
        let mut state = AppState::new();
        state.documents.clear();
        let doc = dictionary_document(&mut state, paint());
        state.active_document = Some(doc);
        let opened = written(&state, doc);
        let options = TextureBuildOptions::default();

        let dxt1 = FormatChoice::Fixed(GrcTextureFormat::DXT1);
        state
            .reencode_texture_in_place(doc, TextureSource::Document, "paint", dxt1, &options)
            .expect("the texture re-encodes");
        state
            .import_texture_as_new(
                doc,
                "decal",
                &source(0xFF),
                FormatChoice::Automatic,
                &options,
            )
            .expect("a new name");
        let edited = written(&state, doc);

        assert_eq!(state.undo().as_deref(), Some("Add texture decal"));
        assert_eq!(state.undo().as_deref(), Some("Replace paint"));
        assert_eq!(
            written(&state, doc),
            opened,
            "undone, it writes other bytes"
        );
        assert!(!state.document(doc).expect("open").dirty);
        while state.redo().is_some() {}
        assert_eq!(written(&state, doc), edited, "redone, an edit is lost");
    }

    #[test]
    fn embedding_and_unembedding_undo_to_the_drawable_as_it_opened() {
        let mut drawable = or_io::gta5::GtaDrawable::<Gen8>::default();
        drawable.base.shader_group = Some(Box::default());
        let mut state = AppState::new();
        state.documents.clear();
        let payload = DocPayload::Resource(or_io::Asset::Drawable(drawable));
        state
            .documents
            .push(Document::new(1, Some("probe.ydr".into()), payload));
        state.active_document = Some(1);
        let shown = |state: &AppState| match &state.document(1).expect("open").payload {
            DocPayload::Resource(or_io::Asset::Drawable(d)) => format!("{d:?}"),
            _ => panic!("not a drawable"),
        };
        let opened = shown(&state);

        let decal = gen8::GrcTexture {
            name: "decal".to_string(),
            ..paint()
        };
        state.embed_texture(1, paint()).expect("embeds");
        state.embed_texture(1, decal).expect("embeds");
        state.unembed_texture(1, "paint").expect("unembeds");
        let edited = shown(&state);
        assert_ne!(edited, opened);

        while state.undo().is_some() {}
        assert_eq!(shown(&state), opened, "undone, it is not as it opened");
        assert!(!state.document(1).expect("open").dirty);
        while state.redo().is_some() {}
        assert_eq!(shown(&state), edited, "redone, an edit is lost");
    }

    #[test]
    fn a_new_entry_that_cannot_come_back_because_its_name_is_taken_changes_nothing() {
        let mut taken = paint();
        taken.width = 8;
        let mut dictionary = or_io::gta5::PgDictionary::default();
        dictionary.insert(taken);
        let mut payload = DocPayload::Resource(or_io::Asset::TextureDict(dictionary));
        let mut step = NewEntry {
            name: "paint".to_string(),
            other: Some(paint()),
        };
        assert!(
            !step.swap(&mut payload),
            "a name already in use cannot take it"
        );
        let held = textures::legacy_dictionary(&payload).and_then(|d| d.get("paint"));
        assert_eq!(
            held.map(|t| t.width),
            Some(8),
            "a swap that said false replaced the entry that was there"
        );
        assert!(
            step.other.is_some(),
            "the step lost the entry it was to put back"
        );
    }
}
