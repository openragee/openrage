use or_io::gta5::PtxFxList;

use crate::state::particle_target::{on_particle_target, ParticleTarget};
use crate::state::textures::Lookup;
use crate::state::DocPayload;

pub const DOC_ROOT_SEL: u64 = u64::MAX;

pub const YPT_TAG_MASK: u64 = 0xF << 60;
pub const YPT_PARTICLE: u64 = 0x0 << 60;
pub const YPT_EFFECT: u64 = 0x6 << 60;
pub const YPT_EMITTER: u64 = 0x7 << 60;
pub const YPT_EMITTER_LIB: u64 = 0x8 << 60;
pub const YPT_PARTICLE_LIB: u64 = 0x9 << 60;
pub const YPT_TEXTURE: u64 = 0xA << 60;
pub const YPT_MODEL: u64 = 0xB << 60;
pub const YPT_DOMAIN: u64 = 0xC << 60;
pub const YPT_BEHAVIOUR: u64 = 0xD << 60;

pub const YPT_TEXTURE_DICT: u64 = 0xE << 60;

pub const YPT_DOMAIN_CREATION: u64 = 0;
pub const YPT_DOMAIN_TARGET: u64 = 1;
pub const YPT_DOMAIN_ATTRACTOR: u64 = 2;

pub fn ypt_effect_id(effect_idx: usize) -> u64 {
    YPT_EFFECT | (effect_idx as u64)
}

pub fn ypt_emitter_id(effect_idx: usize, emitter_idx: usize) -> u64 {
    YPT_EMITTER | ((effect_idx as u64) << 16) | (emitter_idx as u64)
}

pub fn ypt_tag_of(id: u64) -> u64 {
    id & YPT_TAG_MASK
}

pub fn ypt_unpack_emitter(id: u64) -> (usize, usize) {
    let payload = id & !YPT_TAG_MASK;
    let effect = ((payload >> 16) & 0xFFFF) as usize;
    let emitter = (payload & 0xFFFF) as usize;
    (effect, emitter)
}

pub fn ypt_unpack_effect(id: u64) -> usize {
    (id & !YPT_TAG_MASK) as usize
}

pub fn ypt_unpack_particle(id: u64) -> (usize, usize) {
    let payload = id & !YPT_TAG_MASK;
    ((payload / 1000) as usize, ((payload % 1000) / 10) as usize)
}

pub fn ypt_emitter_lib_id(rule_idx: usize) -> u64 {
    YPT_EMITTER_LIB | (rule_idx as u64)
}

pub fn ypt_unpack_emitter_lib(id: u64) -> usize {
    (id & !YPT_TAG_MASK) as usize
}

pub fn ypt_particle_lib_id(rule_idx: usize) -> u64 {
    YPT_PARTICLE_LIB | (rule_idx as u64)
}

pub fn ypt_unpack_particle_lib(id: u64) -> usize {
    (id & !YPT_TAG_MASK) as usize
}

pub fn ypt_texture_id(name: &str) -> u64 {
    YPT_TEXTURE | (name_hash(name) & !YPT_TAG_MASK)
}

const YPT_MODEL_HASH_MASK: u64 = 0x0000_0000_FFFF_FFFF;
const YPT_MODEL_KIND_SHIFT: u32 = 32;
const YPT_MODEL_KIND_MASK: u64 = 0xF << YPT_MODEL_KIND_SHIFT;
const YPT_MODEL_SUB_INDEX_SHIFT: u32 = 36;
const YPT_MODEL_SUB_INDEX_MASK: u64 = 0x00FF_FFFF << YPT_MODEL_SUB_INDEX_SHIFT;

pub const YPT_MODEL_KIND_MODEL: u8 = 0;
pub const YPT_MODEL_KIND_LOD: u8 = 1;
pub const YPT_MODEL_KIND_SHADER: u8 = 2;
pub const YPT_MODEL_KIND_BONE: u8 = 3;
pub const YPT_MODEL_KIND_LIGHT: u8 = 4;
pub const YPT_MODEL_KIND_BOUND: u8 = 5;
pub const YPT_MODEL_KIND_TEXTURE: u8 = 6;

pub const YPT_MODEL_TEXTURES_SECTION_SUB: u32 = 0x00FF_FFFF;

pub fn ypt_model_id(dict_hash: u32) -> u64 {
    YPT_MODEL | (dict_hash as u64)
}

pub fn ypt_unpack_model_hash(id: u64) -> u32 {
    (id & YPT_MODEL_HASH_MASK) as u32
}

pub fn ypt_model_part_id(model_hash: u32, kind: u8, sub_index: u32) -> u64 {
    let kind = (kind as u64 & 0xF) << YPT_MODEL_KIND_SHIFT;
    let sub = (sub_index as u64 & 0x00FF_FFFF) << YPT_MODEL_SUB_INDEX_SHIFT;
    YPT_MODEL | (model_hash as u64) | kind | sub
}

pub fn ypt_unpack_model_part(id: u64) -> (u32, u8, u32) {
    let hash = (id & YPT_MODEL_HASH_MASK) as u32;
    let kind = ((id & YPT_MODEL_KIND_MASK) >> YPT_MODEL_KIND_SHIFT) as u8;
    let sub_index = ((id & YPT_MODEL_SUB_INDEX_MASK) >> YPT_MODEL_SUB_INDEX_SHIFT) as u32;
    (hash, kind, sub_index)
}

pub fn pack_lod_sub_index(lod_idx: u8, model_idx: u8, geom_idx: Option<u16>) -> u32 {
    let g = geom_idx.unwrap_or(0xFFF) as u32;
    ((lod_idx as u32 & 0xF) << 20) | ((model_idx as u32 & 0xFF) << 12) | (g & 0xFFF)
}

pub fn unpack_lod_sub_index(sub: u32) -> (u8, u8, Option<u16>) {
    let lod = ((sub >> 20) & 0xF) as u8;
    let model = ((sub >> 12) & 0xFF) as u8;
    let geom_raw = (sub & 0xFFF) as u16;
    let geom = if geom_raw == 0xFFF {
        None
    } else {
        Some(geom_raw)
    };
    (lod, model, geom)
}

pub const YPT_MODEL_DICT: u64 = 0xF << 60;

pub const YDR_TEXTURE_DICT: u64 = SECT_TEXTURE | (u32::MAX as u64);

pub fn ypt_domain_id(effect_idx: usize, emitter_idx: usize, domain_kind: u64) -> u64 {
    YPT_DOMAIN
        | ((effect_idx as u64 & 0xFFFF_FFFF) << 20)
        | ((emitter_idx as u64 & 0xFFFF) << 4)
        | (domain_kind & 0xF)
}

pub fn ypt_unpack_domain(id: u64) -> (usize, usize, u64) {
    let payload = id & !YPT_TAG_MASK;
    let effect = ((payload >> 20) & 0xFFFF_FFFF) as usize;
    let emitter = ((payload >> 4) & 0xFFFF) as usize;
    let kind = payload & 0xF;
    (effect, emitter, kind)
}

pub fn ypt_behaviour_id(particle_lib_idx: usize, behaviour_idx: usize) -> u64 {
    YPT_BEHAVIOUR
        | ((particle_lib_idx as u64 & 0xFFFF_FFFF) << 16)
        | (behaviour_idx as u64 & 0xFFFF)
}

pub fn ypt_unpack_behaviour(id: u64) -> (usize, usize) {
    let payload = id & !YPT_TAG_MASK;
    let particle = ((payload >> 16) & 0xFFFF_FFFF) as usize;
    let behaviour = (payload & 0xFFFF) as usize;
    (particle, behaviour)
}

pub const DICT_ENTRY: u64 = YPT_MODEL;

pub fn dict_entry_id(key: u32) -> u64 {
    ypt_model_id(key)
}

pub fn dict_entry_part_id(key: u32, kind: u8, sub_index: u32) -> u64 {
    ypt_model_part_id(key, kind, sub_index)
}

pub fn dict_entry_unpack_part(id: u64) -> (u32, u8, u32) {
    ypt_unpack_model_part(id)
}

/// The hash a fragment's physics level `level`, 0 the highest, holds its bound's Outline row under.
pub fn physics_bound_hash(level: usize) -> u32 {
    (level as u32).wrapping_add(1)
}

pub fn is_dict_entry_selection(id: u64) -> bool {
    id != DOC_ROOT_SEL && ypt_tag_of(id) == DICT_ENTRY
}

/// A drawable dictionary's entry hashes, at either target.
fn ydd_codes(payload: &DocPayload) -> Option<&[u32]> {
    match payload {
        DocPayload::Resource(or_io::Asset::DrawableDict(dict)) => Some(&dict.codes),
        DocPayload::Resource(or_io::Asset::DrawableDictGen9(dict)) => Some(&dict.codes),
        _ => None,
    }
}

pub fn ydd_selected_entry_hash(payload: &DocPayload, selection_id: u64) -> Option<u32> {
    let codes = ydd_codes(payload)?;
    if !is_dict_entry_selection(selection_id) {
        return None;
    }
    let (hash, _, _) = dict_entry_unpack_part(selection_id);
    codes.contains(&hash).then_some(hash)
}

/// The hash of a drawable dictionary's first entry, at either target, or the code of a GTA VI dictionary's one drawable.
pub fn ydd_first_entry_hash(payload: &DocPayload) -> Option<u32> {
    let held: Vec<bool> = match payload {
        DocPayload::Resource(or_io::Asset::DrawableDict(dict)) => {
            dict.entries.iter().map(Option::is_some).collect()
        }
        DocPayload::Resource(or_io::Asset::DrawableDictGen9(dict)) => {
            dict.entries.iter().map(Option::is_some).collect()
        }
        // A GTA VI dictionary of several drawables waits for one to be chosen from its list.
        DocPayload::Gta6Drawable(resource) => {
            let mut drawables = crate::state::gta6_resource::dictionary_drawables(resource);
            let (code, _) = drawables.next()?;
            return drawables.next().is_none().then_some(code);
        }
        _ => return None,
    };
    (ydd_codes(payload)?.iter().zip(held)).find_map(|(code, held)| held.then_some(*code))
}

pub fn ydd_selected_entry_index(payload: &DocPayload, selection_id: u64) -> Option<usize> {
    let hash = ydd_selected_entry_hash(payload, selection_id)?;
    ydd_codes(payload)?.iter().position(|h| *h == hash)
}

pub fn ycd_selected_clip_key(payload: &DocPayload, selection_id: u64) -> Option<u32> {
    let DocPayload::Resource(or_io::Asset::Clips(dict)) = payload else {
        return None;
    };
    if !is_dict_entry_selection(selection_id) {
        return None;
    }
    let (key, _, _) = dict_entry_unpack_part(selection_id);
    or_anim::clip::clip(dict, key).map(|_| key)
}

pub fn yld_selected_entry_hash(payload: &DocPayload, selection_id: u64) -> Option<u32> {
    let DocPayload::Resource(or_io::Asset::ClothDict(dict)) = payload else {
        return None;
    };
    if !is_dict_entry_selection(selection_id) {
        return None;
    }
    let (hash, _, _) = ypt_unpack_model_part(selection_id);
    dict.codes.contains(&hash).then_some(hash)
}

pub fn yld_selected_entry_index(payload: &DocPayload, selection_id: u64) -> Option<usize> {
    let hash = yld_selected_entry_hash(payload, selection_id)?;
    let DocPayload::Resource(or_io::Asset::ClothDict(dict)) = payload else {
        return None;
    };
    dict.codes.iter().position(|h| *h == hash)
}

/// The cloth a selection names; `None` for an empty slot.
pub fn yld_selected_cloth(
    payload: &DocPayload,
    selection_id: u64,
) -> Option<&or_io::gta5::CharacterCloth> {
    let index = yld_selected_entry_index(payload, selection_id)?;
    let DocPayload::Resource(or_io::Asset::ClothDict(dict)) = payload else {
        return None;
    };
    dict.entries.get(index)?.as_deref()
}

pub fn name_hash(name: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut h);
    h.finish()
}

pub fn texture_name_for_selection(payload: &DocPayload, selection_id: u64) -> Option<String> {
    crate::state::textures::legacy_dictionary(payload)?
        .entries
        .iter()
        .flatten()
        .find(|t| name_hash(&t.name) == selection_id)
        .map(|t| t.name.clone())
}

pub fn ypt_texture_name_for_selection(payload: &DocPayload, selection_id: u64) -> Option<String> {
    on_particle_target!(payload, T => {
        let dict = T::list(payload)?.texture_dict.as_deref()?;
        let target = selection_id & !YPT_TAG_MASK;
        dict.entries
            .iter()
            .flatten()
            .find(|t| (name_hash(T::name_of(t)) & !YPT_TAG_MASK) == target)
            .map(|t| T::name_of(t).to_string())
    }, else None)
}

/// The model a particle list of either target keys by the selection's hash.
pub fn ypt_drawable_for_selection<T: or_io::gta5::DrawableTarget>(
    list: &or_io::gta5::PtxFxList<T>,
    selection_id: u64,
) -> Option<&or_io::gta5::drawable::RmcDrawable<T>> {
    or_particle::list::model(list, ypt_unpack_model_hash(selection_id))
}

pub fn ydr_drawable_texture_name_for_selection(
    payload: &DocPayload,
    selection_id: u64,
) -> Option<String> {
    fn named<T: Lookup>(
        drawable: &or_io::gta5::RmcDrawable<T>,
        selection_id: u64,
    ) -> Option<String> {
        let dict = crate::state::textures::drawable_textures(drawable)?;
        let target = selection_id & !SECT_MASK;
        (dict.entries.iter().flatten())
            .find(|t| (name_hash(T::name_of(t)) & !SECT_MASK) == target)
            .map(|t| T::name_of(t).to_string())
    }
    match payload {
        DocPayload::Resource(or_io::Asset::Drawable(drawable)) => named(drawable, selection_id),
        DocPayload::Resource(or_io::Asset::DrawableGen9(drawable)) => named(drawable, selection_id),
        _ => None,
    }
}

pub fn ypt_model_texture_name_for_selection(
    payload: &DocPayload,
    selection_id: u64,
) -> Option<String> {
    let (model_hash, kind, idx) = ypt_unpack_model_part(selection_id);
    if kind != YPT_MODEL_KIND_TEXTURE {
        return None;
    }
    on_particle_target!(payload, T => {
        let names = ypt_model_texture_names(T::list(payload)?, model_hash);
        names.into_iter().nth(idx as usize)
    }, else None)
}

/// The names a model's textures go by, its own dictionary's first and then each its shaders bind, once each.
pub fn ypt_model_texture_names<T: ParticleTarget>(
    asset: &PtxFxList<T>,
    model_hash: u32,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let Some(mesh) = or_particle::list::model(asset, model_hash) else {
        return out;
    };
    let Some(group) = mesh.shader_group.as_deref() else {
        return out;
    };
    let dictionary = (group.texture_dictionary.iter())
        .flat_map(|td| td.entries.iter().flatten())
        .map(|tex| T::name_of(tex));
    let bound = (group.shaders.iter()).flat_map(|shader| T::bound_names(shader, group));
    for name in dictionary.chain(bound) {
        if !name.is_empty() && seen.insert(name.to_ascii_lowercase()) {
            out.push(name.to_string());
        }
    }
    out
}

pub fn ypt_model_textures_section_id(model_hash: u32) -> u64 {
    ypt_model_part_id(
        model_hash,
        YPT_MODEL_KIND_TEXTURE,
        YPT_MODEL_TEXTURES_SECTION_SUB,
    )
}

pub fn ypt_active_model_for_textures(payload: &DocPayload, selection: Option<u64>) -> Option<u32> {
    if !matches!(
        payload,
        DocPayload::Resource(or_io::Asset::Particles(_) | or_io::Asset::ParticlesGen9(_))
    ) {
        return None;
    }
    let sel = selection?;
    if ypt_tag_of(sel) != YPT_MODEL {
        return None;
    }
    let (model_hash, kind, _sub) = ypt_unpack_model_part(sel);
    if kind == YPT_MODEL_KIND_TEXTURE {
        Some(model_hash)
    } else {
        None
    }
}

/// The texture `name` names for a model: its own dictionary's, else the list's, else another model's.
pub fn ypt_model_texture_lookup<'a, T: Lookup>(
    asset: &'a PtxFxList<T>,
    model_hash: u32,
    name: &str,
) -> Option<&'a T::Texture> {
    let lookup = |td: Option<&'a or_io::gta5::PgDictionary<T::Texture>>| {
        td?.entries
            .iter()
            .flatten()
            .find(|t| T::name_of(t).eq_ignore_ascii_case(name))
            .map(|t| &**t)
    };

    let of_model = |model: &'a or_io::gta5::drawable::RmcDrawable<T>| {
        lookup(
            model
                .shader_group
                .as_deref()
                .and_then(|sg| sg.texture_dictionary.as_deref()),
        )
    };
    if let Some(tex) = or_particle::list::model(asset, model_hash).and_then(of_model) {
        return Some(tex);
    }
    if let Some(tex) = lookup(asset.texture_dict.as_deref()) {
        return Some(tex);
    }
    or_particle::list::models(asset)
        .filter(|(h, _)| *h != model_hash)
        .find_map(|(_, model)| of_model(model))
}

pub fn root_selection_id() -> u64 {
    0
}

pub const SECT_MASK: u64 = 0xF << 60;
pub const SECT_LOD: u64 = 0x1 << 60;
pub const SECT_SHADER: u64 = 0x2 << 60;
pub const SECT_SKEL: u64 = 0x3 << 60;
pub const SECT_LIGHT: u64 = 0x4 << 60;
pub const SECT_BOUND: u64 = 0x5 << 60;
pub const SECT_TEXTURE: u64 = 0x6 << 60;
/// A `.ynv`'s polygon, numbered as the quadtree's polygon array holds them.
pub const SECT_NAV_POLYGON: u64 = 0x7 << 60;

pub fn pack_selection(lod: u8, model: u16, geom: Option<u32>) -> u64 {
    let g = geom.unwrap_or(u32::MAX) as u64;
    SECT_LOD | ((lod as u64) << 48) | ((model as u64) << 32) | g
}

pub fn unpack_selection(id: u64) -> (u8, u16, Option<u32>) {
    let payload = id & !SECT_MASK;
    let lod = ((payload >> 48) & 0xFF) as u8;
    let model = ((payload >> 32) & 0xFFFF) as u16;
    let g = (payload & 0xFFFF_FFFF) as u32;
    let geom = if g == u32::MAX { None } else { Some(g) };
    (lod, model, geom)
}

pub fn section_of(id: u64) -> u64 {
    id & SECT_MASK
}

pub fn pack_index(section: u64, index: u32) -> u64 {
    section | (index as u64)
}

pub fn payload_index(id: u64) -> u32 {
    (id & !SECT_MASK) as u32
}

/// The row a `.ynv`'s polygon `index` has in the Outline.
pub fn nav_polygon_id(index: usize) -> u64 {
    pack_index(SECT_NAV_POLYGON, index as u32)
}

/// The polygon of a `.ynv` a selection names.
pub fn nav_polygon_of(id: u64) -> Option<usize> {
    (id != DOC_ROOT_SEL && section_of(id) == SECT_NAV_POLYGON).then(|| payload_index(id) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ypt_effect_id_round_trips() {
        for &idx in &[0usize, 1, 7, 255, 4096] {
            let id = ypt_effect_id(idx);
            assert_eq!(ypt_tag_of(id), YPT_EFFECT);
            assert_eq!(ypt_unpack_effect(id), idx);
        }
    }

    #[test]
    fn ypt_emitter_id_round_trips() {
        for &(eff, em) in &[(0usize, 0usize), (1, 2), (7, 15), (255, 65535)] {
            let id = ypt_emitter_id(eff, em);
            assert_eq!(ypt_tag_of(id), YPT_EMITTER);
            assert_eq!(ypt_unpack_emitter(id), (eff, em));
        }
    }

    #[test]
    fn ypt_emitter_lib_id_round_trips() {
        for &idx in &[0usize, 3, 100, 4096] {
            let id = ypt_emitter_lib_id(idx);
            assert_eq!(ypt_tag_of(id), YPT_EMITTER_LIB);
            assert_eq!(ypt_unpack_emitter_lib(id), idx);
        }
    }

    #[test]
    fn ypt_particle_lib_id_round_trips() {
        for &idx in &[0usize, 3, 100, 4096] {
            let id = ypt_particle_lib_id(idx);
            assert_eq!(ypt_tag_of(id), YPT_PARTICLE_LIB);
            assert_eq!(ypt_unpack_particle_lib(id), idx);
        }
    }

    #[test]
    fn ypt_domain_id_round_trips() {
        for &(eff, em, kind) in &[
            (0usize, 0usize, YPT_DOMAIN_CREATION),
            (1, 2, YPT_DOMAIN_TARGET),
            (7, 15, YPT_DOMAIN_ATTRACTOR),
            (4096, 65535, YPT_DOMAIN_CREATION),
        ] {
            let id = ypt_domain_id(eff, em, kind);
            assert_eq!(ypt_tag_of(id), YPT_DOMAIN);
            assert_eq!(ypt_unpack_domain(id), (eff, em, kind));
        }
    }

    #[test]
    fn ypt_behaviour_id_round_trips() {
        for &(part, beh) in &[(0usize, 0usize), (1, 5), (12, 65535), (4096, 200)] {
            let id = ypt_behaviour_id(part, beh);
            assert_eq!(ypt_tag_of(id), YPT_BEHAVIOUR);
            assert_eq!(ypt_unpack_behaviour(id), (part, beh));
        }
    }

    #[test]
    fn ypt_model_part_id_round_trips() {
        for &(hash, kind, sub) in &[
            (0u32, YPT_MODEL_KIND_MODEL, 0u32),
            (0xDEAD_BEEF, YPT_MODEL_KIND_SHADER, 7),
            (0x1234_5678, YPT_MODEL_KIND_BONE, 0x00FF_FFFF),
            (0xAAAA_BBBB, YPT_MODEL_KIND_BOUND, 42),
            (0xFFFF_FFFF, YPT_MODEL_KIND_LIGHT, 3),
            (0xCAFE_BABE, YPT_MODEL_KIND_TEXTURE, 12),
        ] {
            let id = ypt_model_part_id(hash, kind, sub);
            assert_eq!(ypt_tag_of(id), YPT_MODEL);
            assert_eq!(ypt_unpack_model_part(id), (hash, kind, sub));
            if kind == YPT_MODEL_KIND_MODEL && sub == 0 {
                assert_eq!(id, ypt_model_id(hash));
            }
            assert_eq!(ypt_unpack_model_hash(id), hash);
        }
    }

    #[test]
    fn ypt_lod_sub_index_round_trips() {
        for &(lod, model, geom) in &[
            (0u8, 0u8, None),
            (1, 2, Some(0u16)),
            (3, 17, Some(42)),
            (15, 255, Some(0xFFE)), // 0xFFF is the "no geom" sentinel — keep below it
            (0, 0, None),
        ] {
            let sub = pack_lod_sub_index(lod, model, geom);
            assert_eq!(unpack_lod_sub_index(sub), (lod, model, geom));
        }
    }

    #[test]
    fn ypt_texture_and_model_ids_carry_correct_tag() {
        let tex = ypt_texture_id("smoke_alpha");
        assert_eq!(ypt_tag_of(tex), YPT_TEXTURE);
        let model = ypt_model_id(0xDEAD_BEEF);
        assert_eq!(ypt_tag_of(model), YPT_MODEL);
        assert_eq!(ypt_unpack_model_hash(model), 0xDEAD_BEEF);
    }

    #[test]
    fn an_entry_selected_in_a_dictionary_of_either_target_resolves() {
        let codes = vec![0x1111_1111, 0x2222_2222];
        let legacy = or_io::gta5::PgDictionary {
            codes: codes.clone(),
            entries: vec![Some(Box::default()), Some(Box::default())],
            ..Default::default()
        };
        let enhanced = or_io::gta5::PgDictionary {
            codes,
            entries: vec![Some(Box::default()), Some(Box::default())],
            ..Default::default()
        };
        for (target, payload) in [
            ("Legacy", or_io::Asset::DrawableDict(legacy)),
            ("Enhanced", or_io::Asset::DrawableDictGen9(enhanced)),
        ] {
            let payload = DocPayload::Resource(payload);
            let part = dict_entry_part_id(0x2222_2222, YPT_MODEL_KIND_LOD, 0);
            assert_eq!(
                ydd_selected_entry_index(&payload, part),
                Some(1),
                "{target}"
            );
            assert_eq!(ydd_selected_entry_hash(&payload, part), Some(0x2222_2222));
            assert_eq!(
                ydd_selected_entry_index(&payload, dict_entry_id(0x3333_3333)),
                None,
                "{target}: a hash the dictionary does not hold"
            );
        }
    }

    #[test]
    fn ypt_tags_are_distinct() {
        let tags = [
            YPT_EFFECT,
            YPT_EMITTER,
            YPT_EMITTER_LIB,
            YPT_PARTICLE_LIB,
            YPT_TEXTURE,
            YPT_MODEL,
            YPT_DOMAIN,
            YPT_BEHAVIOUR,
        ];
        for (i, a) in tags.iter().enumerate() {
            for b in tags.iter().skip(i + 1) {
                assert_ne!(a, b, "Duplicate YPT tag");
            }
        }
    }
}
