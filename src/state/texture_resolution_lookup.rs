//! Texture lookup using the name key of each game, shared by probing and fetching.

use super::*;

pub(super) fn holds(asker: Option<&Document>, doc: &Document, name: &str) -> bool {
    let gta6 = crate::state::ttpf_document::is_gta6;
    if gta6(Some(doc)) != gta6(asker) {
        return false;
    }
    match &doc.payload {
        DocPayload::Resource(or_io::Asset::TextureDict(_)) => {
            dictionary_texture(doc, name).is_some()
        }
        DocPayload::Resource(or_io::Asset::TextureDictGen9(dict)) => {
            textures::find_in_dictionary_g9(Some(dict), name).is_some_and(textures::decodes_g9)
        }
        payload => crate::state::ttpf_document::holds(asker, payload, name),
    }
}

fn dictionary_texture<'a>(
    doc: &'a Document,
    name: &str,
) -> Option<&'a or_io::gta5::gen8::GrcTexture> {
    let DocPayload::Resource(or_io::Asset::TextureDict(dict)) = &doc.payload else {
        return None;
    };
    if !crate::state::ttpf_document::is_gta6(Some(doc)) {
        return dict.get(name);
    }
    let key = match name.strip_prefix("hash_") {
        Some(hex) => u64::from_str_radix(hex, 16).ok()?,
        None => or_io::gta6::hash::name_hash64(name.as_bytes()),
    };
    // The Legacy projection's 32-bit codes do not identify GTA VI shader bindings.
    let texture = dict
        .entries
        .iter()
        .flatten()
        .find(|texture| or_io::gta6::hash::name_hash64(texture.name.as_bytes()) == key)?;
    crate::state::ttpf_document::refused(doc, &texture.name)
        .is_none()
        .then_some(texture.as_ref())
}

pub(super) fn fetch<'a>(doc: &'a Document, name: &str) -> Option<textures::Texture<'a>> {
    match &doc.payload {
        DocPayload::Resource(or_io::Asset::TextureDict(_)) => {
            dictionary_texture(doc, name).map(|t| textures::Texture::Owned(Cow::Borrowed(t)))
        }
        DocPayload::Resource(or_io::Asset::TextureDictGen9(dict)) => {
            textures::find_in_dictionary_g9(Some(dict), name)
                .filter(|t| textures::decodes_g9(t))
                .map(|t| textures::Texture::Enhanced(Cow::Borrowed(t)))
        }
        payload => crate::state::ttpf_document::fetch(payload, name),
    }
}
