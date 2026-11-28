//! The textures a scanned GTA VI drawable's shaders bind, as the Textures panel lists a GTA V
//! drawable's: none is embedded, so each is shown from the open document its key resolves to,
//! and replaced and exported there.

use std::hash::{Hash, Hasher};
use std::path::Path;

use or_io::authoring::{FormatChoice, TextureBuildOptions};
use or_io::codec::texture::GrcTextureFormat;

use super::document::DocPayload;
use super::gta6_resource::{drawable_of, texture_name};
use super::textures::{DocumentTexture, Texture, TextureListing, TextureResolution, TextureSource};
use super::toasts::ToastAction;
use crate::ui::panels::selection_ids::{
    dict_entry_part_id, dict_entry_unpack_part, is_dict_entry_selection, name_hash, SECT_MASK,
    SECT_TEXTURE, YPT_MODEL_KIND_TEXTURE,
};

/// What a selection that is no texture of the drawable says.
const PICK_A_TEXTURE: &str = "Select one of its textures, under Textures";

/// The textures the drawable's shaders bind, by the names their keys are known by; the size and
/// format only of one its own dictionary holds.
pub(crate) fn list(payload: &DocPayload, selection: Option<u64>) -> TextureListing {
    use or_io::gta6::headerless::{Entry, Resource};
    let DocPayload::Gta6Drawable(resource) = payload else {
        return TextureListing::Nothing("Active document has no texture dictionary");
    };
    let held = |name: &str| match &**resource {
        Resource::Dictionary(dict) => dict.entries.iter().find_map(|(_, entry)| match entry {
            Ok(Entry::Texture(t)) if t.name.eq_ignore_ascii_case(name) => t.texture.as_ref().ok(),
            _ => None,
        }),
        _ => None,
    };
    if matches!(**resource, Resource::Bound(_) | Resource::Cloth(_)) {
        return TextureListing::Nothing("Active document has no texture dictionary");
    }
    let Some(drawable) = drawable_of(resource, selection) else {
        return TextureListing::Nothing("Select one of its drawables");
    };
    let entry =
        (selection.filter(|s| is_dict_entry_selection(*s))).map(|s| dict_entry_unpack_part(s).0);
    let mut names: Vec<String> = Vec::new();
    for key in (drawable.shaders.iter()).flat_map(|s| s.texture_keys.iter().flatten()) {
        let name = texture_name(*key);
        if !names.contains(&name) {
            names.push(name);
        }
    }
    let textures = (names.into_iter().enumerate())
        .map(|(i, name)| {
            let held = held(&name);
            DocumentTexture {
                selection_id: match entry {
                    Some(hash) => dict_entry_part_id(hash, YPT_MODEL_KIND_TEXTURE, i as u32),
                    None => SECT_TEXTURE | (name_hash(&name) & !SECT_MASK),
                },
                name,
                width: held.map_or(0, |t| t.width),
                height: held.map_or(0, |t| t.height),
                format: held.map_or(GrcTextureFormat::None, |t| t.format.format()),
            }
        })
        .collect();
    TextureListing::Textures {
        textures,
        source: entry.map_or(TextureSource::Root, |hash| TextureSource::Entry { hash }),
    }
}

/// `listing` of `doc`, a scanned GTA VI drawable, with each texture its own dictionary does not
/// hold given the size and format the directory of an open pack states for its key, else those of
/// the texture it resolves to in another open document.
pub fn sized_for_gta6(
    state: &super::AppState,
    doc: super::DocumentId,
    mut listing: TextureListing,
) -> TextureListing {
    let drawable = state.document(doc).map(|d| &d.payload);
    let TextureListing::Textures { textures, .. } = &mut listing else {
        return listing;
    };
    if !matches!(drawable, Some(DocPayload::Gta6Drawable(_))) {
        return listing;
    }
    // ⚠ Asked every frame by the Outline and the Gallery, and a resolve reads what it finds: the
    // sizes are found again only when the names asked or an open document change.
    let mut hasher = std::hash::DefaultHasher::new();
    (doc, super::gta6_resource::drawable_lines_stamp(state)).hash(&mut hasher);
    // Where each open document is, which tells one session's documents from another's.
    (state.documents.iter()).for_each(|d| std::ptr::from_ref(&d.payload).hash(&mut hasher));
    textures
        .iter()
        .for_each(|t| (&t.name, t.width).hash(&mut hasher));
    let asked = hasher.finish();
    let mut kept = SIZED.lock().unwrap_or_else(|p| p.into_inner());
    if let Some((_, sizes)) = kept.as_ref().filter(|(at, _)| *at == asked) {
        for (t, (width, height, format)) in textures.iter_mut().zip(sizes) {
            (t.width, t.height, t.format) = (*width, *height, *format);
        }
        return listing;
    }
    for t in textures.iter_mut().filter(|t| t.width == 0) {
        if let Some(entry) = open_pack_entry(state, &t.name) {
            (t.width, t.height) = (entry.width, entry.height);
            t.format = entry
                .format()
                .map_or(GrcTextureFormat::None, |f| f.format());
        // Packs first: their directory states the size without reading the texture from disk.
        } else if let Some(found) = super::texture_resolution::resolve(state, None, None, &t.name) {
            let texture = found.texture;
            (t.width, t.height) = (texture.width().into(), texture.height().into());
            t.format = texture.format();
        }
    }
    let sizes = textures
        .iter()
        .map(|t| (t.width, t.height, t.format))
        .collect();
    *kept = Some((asked, sizes));
    listing
}

/// What [`sized_for_gta6`] last found, by what it was asked: each texture's size and format.
type Sized = Option<(u64, Vec<(u32, u32, GrcTextureFormat)>)>;
static SIZED: std::sync::Mutex<Sized> = std::sync::Mutex::new(None);

/// The directory entry an open pack holds for texture `name`, a GTA VI drawable's key by the name it is known by.
pub fn open_pack_entry(state: &super::AppState, name: &str) -> Option<or_io::gta6::Entry> {
    (state.documents.iter())
        .find_map(|d| match &d.payload {
            DocPayload::Ttpf(pack) => pack.index_of(name).and_then(|i| pack.pack.entries.get(i)),
            _ => None,
        })
        .copied()
}

/// The row of [`list`] the selection is.
pub(crate) fn selected(payload: &DocPayload, selection: u64) -> TextureResolution {
    let TextureListing::Textures { textures, source } = list(payload, Some(selection)) else {
        return TextureResolution::NotATexture(PICK_A_TEXTURE);
    };
    match textures.into_iter().find(|t| t.selection_id == selection) {
        Some(t) => TextureResolution::Found {
            name: t.name,
            source,
        },
        None => TextureResolution::NotATexture(PICK_A_TEXTURE),
    }
}

/// The texture `name` of a scanned dictionary that holds it beside its drawables; a drawable alone
/// holds none, and its keys resolve in other documents through [`resolved_for_gta6`].
pub(crate) fn at<'p>(payload: &'p DocPayload, _: TextureSource, name: &str) -> Option<Texture<'p>> {
    use or_io::gta6::headerless::{Entry, Resource};
    let DocPayload::Gta6Drawable(resource) = payload else {
        return None;
    };
    let Resource::Dictionary(dict) = &**resource else {
        return None;
    };
    let texture = dict.entries.iter().find_map(|(_, entry)| match entry {
        Ok(Entry::Texture(t)) if t.name.eq_ignore_ascii_case(name) => {
            let pixels = t.texture.as_ref().ok()?.clone();
            super::ttpf_document::grc(t.name.clone(), pixels).ok()
        }
        _ => None,
    })?;
    Some(Texture::Owned(std::borrow::Cow::Owned(texture)))
}

/// The texture `name` that a key of `doc`, a scanned GTA VI drawable, resolves to in another
/// document, as the model view binds it; `None` for any other document.
pub fn resolved_for_gta6(
    state: &super::AppState,
    doc: super::DocumentId,
    selection: Option<u64>,
    name: &str,
) -> Option<Texture<'static>> {
    if !matches!(state.document(doc)?.payload, DocPayload::Gta6Drawable(_)) {
        return None;
    }
    let resolved = super::texture_resolution::resolve(state, None, selection, name)?;
    Some(resolved.texture.into_owned())
}

/// Said of a texture a drawable binds by a key that no open document holds.
pub(super) const UNRESOLVED: &str = tr_key!(
    "This texture is unresolved: no open texture pack and no dictionary the app has read holds it."
);

impl super::AppState {
    /// `listing` of `doc`, a scanned GTA VI drawable, without the textures no pack of its install holds: there is no picture to show for one.
    pub fn without_unheld_textures(
        &self,
        doc: super::DocumentId,
        mut listing: TextureListing,
    ) -> TextureListing {
        if let TextureListing::Textures { textures, .. } = &mut listing {
            // One of no size is not in the drawable's own dictionary.
            textures.retain(|t| t.width != 0 || self.held_by_no_pack(doc, &t.name).is_none());
        }
        listing
    }

    /// The open document that holds the texture `name` a key of the drawable `doc` resolves to; `Err` says none does.
    pub fn bound_texture_holder(
        &self,
        doc: super::DocumentId,
        name: &str,
    ) -> Result<super::DocumentId, &'static str> {
        let from = super::preview_scene::MeshSource {
            document: doc,
            entry: None,
        };
        let found = super::texture_resolution::probe(self, Some(from), None, name).found;
        (found.map(|origin| origin.document)).ok_or(crate::i18n::tr(UNRESOLVED))
    }

    /// The document that takes a replacement of that texture; `Err` says why none does: no document holds it, or one no view may change does.
    pub fn bound_texture_pack(
        &self,
        doc: super::DocumentId,
        name: &str,
    ) -> Result<super::DocumentId, &'static str> {
        // A texture the drawable's own shader group embeds is replaced in the drawable.
        if super::gta6_embedded_edit::embeds(self, doc, name) {
            return Ok(doc);
        }
        let holder = self.bound_texture_holder(doc, name)?;
        let Some(held) = self.document(holder) else {
            return Err(crate::i18n::tr(UNRESOLVED));
        };
        // A scanned dictionary's texture is replaced as its own tab replaces it, and saved as that tab saves.
        match held.read_only {
            Some(_) if super::gta6_edit::is_scanned_dictionary(held) => Ok(holder),
            Some(why) => Err(crate::i18n::tr(why)),
            None => Ok(holder),
        }
    }

    /// Replace that texture from the image at `image`, in the pack that holds it, and offer that pack's save: the drawable on screen saves nothing.
    pub fn replace_bound_texture(
        &mut self,
        doc: super::DocumentId,
        name: &str,
        image: &Path,
        format: FormatChoice,
        options: &TextureBuildOptions,
    ) {
        if let Some(outcome) = self.replace_embedded_texture(doc, name, image, format, options) {
            match outcome {
                Ok(edit) => self.show_toast(edit.describe()),
                Err(why) => self.show_error_toast(why),
            }
            return;
        }
        let replaced =
            (self.bound_texture_pack(doc, name).map_err(str::to_string)).and_then(|pack| {
                let pixels = or_io::authoring::texture_import::load(image)
                    .map_err(|e| format!("{}: {e}", image.display()))?;
                let held = TextureSource::Document;
                self.import_texture_over(pack, held, name, &pixels, format, options)?;
                Ok(pack)
            });
        match replaced {
            Ok(pack) => {
                let held = self.document(pack);
                let file = held.map_or_else(String::new, |held| held.file_name());
                // ⚠ A body read out of an archive is never saved over it: Save As in its tab writes a file of its own.
                let in_archive = (held.and_then(|d| d.path.as_deref())).is_some_and(|path| {
                    super::rpf9_scan::scanned_body(path).is_some_and(|(at, _)| at != path)
                });
                if in_archive {
                    self.show_info_toast(tr!(
                        "Replaced {name} in {pack}. Save As in its tab writes the dictionary as a file of its own.",
                        name = name,
                        pack = file
                    ));
                } else {
                    self.show_info_toast_with_action(
                        tr!("Replaced {name} in {pack}.", name = name, pack = file),
                        ToastAction::SavePack(pack),
                    );
                }
            }
            Err(why) => self.show_error_toast(why),
        }
    }

    /// Save the pack `pack` as Save does the document in front, which stays in front.
    pub(super) fn save_pack(&mut self, pack: super::DocumentId) {
        // ⚠ Save writes the active document and asks the edit gate for it, so the pack is that while its save starts.
        let in_front = self.active_document.replace(pack);
        self.save_active_document();
        self.active_document = in_front;
    }

    /// Write that texture to `path`: every mip for a `.dds`, else its top mip as a PNG.
    pub fn export_bound_texture(&mut self, doc: super::DocumentId, name: &str, path: &Path) {
        let written = resolved_for_gta6(self, doc, None, name)
            .ok_or_else(|| {
                tr!(
                    "{name} does not read from the document that holds it",
                    name = name
                )
            })
            .and_then(|texture| super::textures::export_texture(&texture, 0, path));
        match written {
            Ok(()) => self.show_toast(tr!("Wrote {path}", path = path.display())),
            Err(why) => self.show_error_toast(why),
        }
    }
}
