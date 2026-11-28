//! What a GTA VI ped part's stages are fed of its component: the buffer whose dword 8 decides
//! whether the pixel program samples the textures it picks, and those textures, by the join.

use std::collections::BTreeMap;
use std::sync::Arc;

use or_cli::gta6_ped_layers::{component_map, ComponentBuffer, Map, ObjectData};
use or_cli::gta6_ped_skin::{Loaded, SkinPreset, SkinTone, SkinToneEntry, FALLBACK_PRESET};
use or_cli::{Gta6Feed as Filled, Gta6ShadingPass};
use or_io::gta6::ped_assets::PedTextures;
use or_io::gta6::skin_tones::SkinTones;
use or_renderer::game_shaders::mesh::Image;
use or_renderer::game_shaders::meshlets::Feed;

use crate::state::gta6_shaders::ped_assets::Worn;

/// Whether `pass` is fed anything of a ped's component.
pub(crate) fn reads_a_component(pass: &Gta6ShadingPass) -> bool {
    let mut feeds = pass.mesh.feeds.iter().chain(&pass.pixel.feeds);
    feeds.any(|fed| matches!(fed.feed, Filled::PedComponent | Filled::PedTexture(_)))
}

/// The join a surface `pass` draws is fed from: `asked`, kept in `held` from the first pass fed of
/// a component on. While a worker reads the index `decoding` is set and no join is given.
pub(crate) fn joined_for<'h>(
    pass: &Gta6ShadingPass,
    held: &'h mut Option<Option<Worn>>,
    asked: &dyn Fn() -> Option<Worn>,
    decoding: &std::cell::Cell<bool>,
) -> Option<&'h PedTextures> {
    if !reads_a_component(pass) {
        return None;
    }
    match held.get_or_insert_with(asked) {
        Some(Worn::Reading) => {
            decoding.set(true);
            None
        }
        Some(Worn::Joined(joined)) => joined.as_ref(),
        None => None,
    }
}

/// Fill `feeds`, the bindings of a surface `pass` draws, with what its stages are fed of the
/// component `joined` gives, or of one no state wears; `map` finds a texture by its key. `Err`
/// says why the surface draws as Material.
// A picked texture the join gives no key of, or no pack holds, is left unfed: it reads as
// noughts, as the game's own default for it does, an array of layers of nought. A material
// texture the component writes takes the join's map, else the drawable's own, else the texel
// the game resets the component's record with.
pub(crate) fn fill(
    pass: &Gta6ShadingPass,
    joined: Option<&PedTextures>,
    map: &mut dyn FnMut(u64) -> Option<Arc<Image>>,
    feeds: &mut BTreeMap<(u32, u32), Feed>,
) -> Result<(), String> {
    if let Some(joined) = joined.filter(|joined| joined.wrinkled) {
        return Err(tr!(
            "its part's state {state} has a wrinkle map, and the number the game then sets to have its pixel program sample the map is not read",
            state = format!("0x{:08X}", joined.state),
        ));
    }
    let ped = reads_a_component(pass);
    for stage in [&pass.mesh, &pass.pixel] {
        for fed in &stage.feeds {
            let at = (fed.group, fed.binding);
            let feed = match &fed.feed {
                Filled::PedComponent => Feed::Bytes(ComponentBuffer::AT_REST.bytes()),
                Filled::PedTexture(name) => {
                    let image = joined
                        .and_then(|joined| joined.key(*name))
                        .and_then(&mut *map);
                    match image {
                        Some(image) => Feed::Textures(vec![Some(image)]),
                        None => continue,
                    }
                }
                Filled::Texture(number) if ped => {
                    let stated = (stage.textures.iter())
                        .find_map(|(n, hash)| (n == number).then_some(*hash));
                    let Some((wears, texel)) = stated.and_then(component_map) else {
                        continue;
                    };
                    let key = joined.and_then(|joined| match wears {
                        Map::Albedo => Some(joined.albedo),
                        Map::Normal => Some(joined.normal),
                        Map::Named(name) => joined.key(name),
                    });
                    let own = matches!(feeds.get(&at), Some(Feed::Textures(images))
                        if images.first().is_some_and(Option::is_some));
                    match key.and_then(&mut *map) {
                        Some(image) => Feed::Textures(vec![Some(image)]),
                        None if own => continue,
                        None => Feed::Textures(vec![Some(Arc::new(Image {
                            width: 1,
                            height: 1,
                            rgba: texel.to_vec(),
                            srgb: false,
                        }))]),
                    }
                }
                _ => continue,
            };
            feeds.insert(at, feed);
        }
    }
    Ok(())
}

/// Feed the skin tone table and the object record of a pass that reads the table with `tones`
/// as the game builds them for a ped that names no preset: the fallback preset's record.
/// `None` where no skin tones are loaded leaves the records at rest. The preset it is drawn
/// with is said once a session.
pub(crate) fn fill_skin(
    pass: &Gta6ShadingPass,
    tones: Option<&SkinTones>,
    feeds: &mut BTreeMap<(u32, u32), Feed>,
) {
    let stages = || pass.mesh.feeds.iter().chain(&pass.pixel.feeds);
    let Some(tones) = tones.filter(|_| stages().any(|fed| fed.feed == Filled::SkinTones)) else {
        return;
    };
    let loaded_tones: Vec<(u32, SkinTone)> = (tones.tones.iter())
        .map(|t| {
            let tone = SkinTone {
                colour: t.colour,
                at: t.place,
                extra: t.extra,
                depth_weight: t.depth_weight,
            };
            (t.name, tone)
        })
        .collect();
    let presets: Vec<SkinPreset> = (tones.presets.iter())
        .map(|p| SkinPreset {
            name: p.name,
            tone: p.tone,
            entry: SkinToneEntry {
                colour: p.colour,
                offset: p.offset,
            },
        })
        .collect();
    let loaded = Loaded {
        tones: &loaded_tones,
        presets: &presets,
    };
    // A ped that names no preset is looked up by a name no preset has.
    let slot = loaded.slot(0);
    let record = ObjectData {
        skin_tone: slot,
        depth_weight: loaded.depth_weight(slot),
        ..ObjectData::AT_REST
    };
    static SAID: std::sync::Once = std::sync::Once::new();
    SAID.call_once(|| {
        tracing::info!(
            slot,
            "a ped's part is drawn with the engine's fallback skin tone preset 0x{FALLBACK_PRESET:08X}, as a ped that names none"
        );
    });
    for fed in stages() {
        let bytes = match fed.feed {
            Filled::SkinTones => loaded.table(),
            Filled::ObjectData => record.bytes(),
            _ => continue,
        };
        let filled = Feed::Bytes(super::gta6_meshlets::headed(fed.header, &bytes));
        feeds.insert((fed.group, fed.binding), filled);
    }
}

#[cfg(test)]
mod tests {
    use or_cli::{Gta6Fed as Fed, Gta6Stage as Stage};
    use or_io::gta6::ped_assets::{WRINKLE, WRINKLE_MASK};

    use super::*;

    fn pass(feeds: Vec<Fed>) -> Gta6ShadingPass {
        let stage = |feeds| Stage {
            approximations: Default::default(),
            module: Arc::default(),
            feeds,
            textures: Vec::new(),
            streams: Default::default(),
            rows: Vec::new(),
        };
        Gta6ShadingPass {
            mesh: stage(Vec::new()),
            pixel: stage(feeds),
            linked: Default::default(),
            forward: None,
            then: None,
        }
    }

    fn fed(binding: u32, feed: Filled) -> Fed {
        Fed {
            group: 1,
            binding,
            feed,
            header: None,
        }
    }

    fn joined(wrinkled: bool) -> PedTextures {
        PedTextures {
            kit: 1,
            state: 0x44E2_1C90,
            sets: 1,
            albedo: 0,
            normal: 0,
            listed: true,
            wrinkled,
            named: vec![(WRINKLE_MASK, 0xF3)],
        }
    }

    #[test]
    fn a_component_with_no_wrinkle_map_is_fed_minus_one_and_the_textures_the_join_keys() {
        let pass = pass(vec![
            fed(0, Filled::PedComponent),
            fed(1, Filled::PedTexture(WRINKLE_MASK)),
            fed(2, Filled::PedTexture(WRINKLE)),
            fed(3, Filled::Sampler),
        ]);
        assert!(reads_a_component(&pass));
        let image = Arc::new(Image {
            width: 1,
            height: 1,
            rgba: vec![1, 2, 3, 4],
            srgb: false,
        });
        let mut asked = Vec::new();
        let mut map = |key: u64| {
            asked.push(key);
            Some(Arc::clone(&image))
        };
        let mut feeds = BTreeMap::new();
        assert_eq!(
            fill(&pass, Some(&joined(false)), &mut map, &mut feeds),
            Ok(())
        );
        assert_eq!(asked, [0xF3], "the one texture the join keys");
        let Some(Feed::Bytes(component)) = feeds.get(&(1, 0)) else {
            panic!("the component's buffer is not fed");
        };
        assert_eq!(*component, ComponentBuffer::AT_REST.bytes());
        assert_eq!(component[32..36], (-1i32).to_le_bytes());
        assert!(matches!(feeds.get(&(1, 1)), Some(Feed::Textures(images)) if images.len() == 1));
        assert!(!feeds.contains_key(&(1, 2)) && !feeds.contains_key(&(1, 3)));

        // A part no state wears is fed its component's reset state; one with a wrinkle map is refused.
        let mut reset = BTreeMap::new();
        assert_eq!(fill(&pass, None, &mut |_| None, &mut reset), Ok(()));
        assert_eq!(reset.len(), 1);
        let why = fill(
            &pass,
            Some(&joined(true)),
            &mut |_| None,
            &mut BTreeMap::new(),
        );
        assert!(why.is_err_and(|why| why.contains("0x44E21C90")));
        assert!(!reads_a_component(&self::pass(vec![fed(
            0,
            Filled::Sampler
        )])));
    }

    #[test]
    fn a_ped_s_material_textures_take_the_join_s_maps_else_the_drawable_s_else_the_reset_texel() {
        use or_cli::gta6_ped_layers::MB_DEFAULT;
        // Number 0 is the albedo's slot, 1 the material map mb's, 2 one the component never writes.
        let mut part = pass(vec![
            fed(0, Filled::PedComponent),
            fed(1, Filled::Texture(0)),
            fed(2, Filled::Texture(1)),
            fed(3, Filled::Texture(2)),
        ]);
        part.pixel.textures = vec![(0, 0xC8E8_C282), (1, 0x739E_3D0D), (2, 0x0CD2_7763)];
        let image = |red| {
            Arc::new(Image {
                width: 1,
                height: 1,
                rgba: vec![red, 0, 0, 255],
                srgb: false,
            })
        };
        let mut state = joined(false);
        state.albedo = 0xA1;
        let mut map = |key: u64| (key == 0xA1).then(|| image(7));
        let texel = |feeds: &BTreeMap<(u32, u32), Feed>, binding| match feeds.get(&(1, binding)) {
            Some(Feed::Textures(images)) => images[0].as_ref().map(|i| i.rgba.clone()),
            _ => None,
        };
        // What the drawable's own slots gave: nothing at all three.
        let empty = || -> BTreeMap<(u32, u32), Feed> {
            (1..4)
                .map(|b| ((1, b), Feed::Textures(vec![None])))
                .collect()
        };
        let mut feeds: BTreeMap<(u32, u32), Feed> = empty();
        assert_eq!(fill(&part, Some(&state), &mut map, &mut feeds), Ok(()));
        assert_eq!(
            texel(&feeds, 1),
            Some(vec![7, 0, 0, 255]),
            "the join's albedo"
        );
        assert_eq!(
            texel(&feeds, 2),
            Some(MB_DEFAULT.to_vec()),
            "mb's reset texel"
        );
        assert_eq!(
            texel(&feeds, 3),
            None,
            "a slot the component does not write"
        );
        // The drawable's own map stands where the join gives none.
        let mut own: BTreeMap<(u32, u32), Feed> = empty();
        own.insert((1, 2), Feed::Textures(vec![Some(image(9))]));
        assert_eq!(fill(&part, None, &mut |_| None, &mut own), Ok(()));
        assert_eq!(texel(&own, 2), Some(vec![9, 0, 0, 255]));
        // A surface fed of no component keeps its material textures as they are.
        part.pixel.feeds.remove(0);
        let mut building: BTreeMap<(u32, u32), Feed> = empty();
        assert_eq!(fill(&part, None, &mut |_| None, &mut building), Ok(()));
        assert_eq!(texel(&building, 2), None);
    }

    #[test]
    fn a_surface_fed_of_no_component_asks_nothing_of_the_ped_index() {
        let building = pass(vec![fed(0, Filled::Sampler)]);
        let decoding = std::cell::Cell::new(false);
        let asked = || -> Option<Worn> { panic!("a building asked the ped index") };
        let mut held = None;
        assert_eq!(joined_for(&building, &mut held, &asked, &decoding), None);
        assert!(held.is_none() && !decoding.get());
        // A ped's part asks once, and waits for the index without holding the frame.
        let part = pass(vec![fed(0, Filled::PedComponent)]);
        let reading = || Some(Worn::Reading);
        assert_eq!(joined_for(&part, &mut held, &reading, &decoding), None);
        assert!(decoding.get() && held.is_some());
    }

    #[test]
    fn a_skin_pass_is_fed_the_loaded_table_and_the_fallback_preset_s_record() {
        use or_cli::gta6_ped_skin::FALLBACK_TONE;
        use or_io::gta6::skin_tones::{SkinPreset as Preset, SkinTone as Tone};
        let tones = SkinTones {
            tones: vec![Tone {
                name: FALLBACK_TONE,
                colour: [0.5; 3],
                place: [0.25, 0.0, 0.5],
                depth_weight: 0.75,
                extra: [0.0; 3],
            }],
            presets: vec![Preset {
                name: FALLBACK_PRESET,
                colour: [0.5; 3],
                tone: FALLBACK_TONE,
                offset: [0.0; 3],
            }],
        };
        let skin = pass(vec![fed(0, Filled::SkinTones), fed(1, Filled::ObjectData)]);
        let mut feeds = BTreeMap::new();
        fill_skin(&skin, Some(&tones), &mut feeds);
        let bytes = |binding| match feeds.get(&(1, binding)) {
            Some(Feed::Bytes(bytes)) => bytes.clone(),
            _ => Vec::new(),
        };
        assert_eq!(bytes(0).len(), 2 * 60, "record nought, then the preset's");
        let record = bytes(1);
        assert_eq!(
            u32::from_le_bytes(record[0x164..0x168].try_into().unwrap()),
            1
        );
        assert_eq!(
            f32::from_le_bytes(record[0x19C..0x1A0].try_into().unwrap()),
            0.75
        );
        // No skin tones loaded, or a pass that reads no table: nothing is fed.
        let mut none = BTreeMap::new();
        fill_skin(&skin, None, &mut none);
        fill_skin(
            &pass(vec![fed(1, Filled::ObjectData)]),
            Some(&tones),
            &mut none,
        );
        assert!(none.is_empty());
    }
}
