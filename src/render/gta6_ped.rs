//! What a GTA VI ped part's stages are fed of its component: the buffer whose dword 8 decides
//! whether the pixel program samples the textures it picks, and those textures, by the join.

use std::collections::BTreeMap;
use std::sync::Arc;

use or_cli::{Gta6Feed as Filled, Gta6ShadingPass};
use or_io::gta6::ped_assets::PedTextures;
use or_renderer::game_shaders::mesh::Image;
use or_renderer::game_shaders::meshlets::Feed;

use crate::state::gta6_shaders::ped_assets::Worn;

/// The bytes of a ped component's buffer, and the byte of its dword 8.
const COMPONENT_BYTES: usize = 48;
const SAMPLES_AT: usize = 32;
/// What the game sets dword 8 to for a component with no wrinkle map: the pixel program then
/// samples none of the three textures it picks.
const NO_WRINKLE: i32 = -1;

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
// A texture the join gives no key of, or no pack holds, is left unfed: it reads as noughts, as
// the game's own default for it does, an array of layers of nought.
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
    for fed in pass.mesh.feeds.iter().chain(&pass.pixel.feeds) {
        let feed = match &fed.feed {
            Filled::PedComponent => {
                let mut bytes = vec![0u8; COMPONENT_BYTES];
                bytes[SAMPLES_AT..SAMPLES_AT + 4].copy_from_slice(&NO_WRINKLE.to_le_bytes());
                Feed::Bytes(bytes)
            }
            Filled::PedTexture(name) => {
                let image = joined
                    .and_then(|joined| joined.key(*name))
                    .and_then(&mut *map);
                match image {
                    Some(image) => Feed::Textures(vec![Some(image)]),
                    None => continue,
                }
            }
            _ => continue,
        };
        feeds.insert((fed.group, fed.binding), feed);
    }
    Ok(())
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
        assert_eq!(component.len(), COMPONENT_BYTES);
        assert_eq!(component[32..36], (-1i32).to_le_bytes());
        assert!(component[..32]
            .iter()
            .chain(&component[36..])
            .all(|b| *b == 0));
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
}
