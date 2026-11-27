//! A terrain tile's buffers and default textures, as the game binds them for a tile whose own
//! record is not ready: the binder names them, and this gives each its bytes or its texel.

use std::sync::Arc;

use or_cli::gta6_terrain::{self as terrain, DefaultTexture, Terrain};
use or_cli::{Gta6Feed as Filled, Gta6ShadingPass};
use or_renderer::game_shaders::mesh::Image;
use or_renderer::game_shaders::meshlets::Feed;

/// The one texel of `texture`.
fn image(texture: DefaultTexture) -> Image {
    Image {
        width: 1,
        height: 1,
        rgba: texture.texel().to_vec(),
        srgb: false,
    }
}

/// What fills a binding the binder gives the terrain's `part`, after the header a buffer read by
/// address states; `None` binds nothing.
pub(super) fn feed(part: Terrain, header: Option<u32>) -> Option<Feed> {
    let words = match part {
        Terrain::Record => terrain::default_record(DefaultTexture::entry).words(),
        Terrain::Layers => terrain::default_layer(DefaultTexture::entry).words(),
        Terrain::Constants => terrain::default_constants(DefaultTexture::entry).words(),
        Terrain::Texture(texture) => {
            return Some(Feed::Textures(vec![Some(Arc::new(image(texture)))]))
        }
        Terrain::Feedback => return None,
    };
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    Some(Feed::Bytes(super::gta6_meshlets::headed(header, &bytes)))
}

/// Put the default textures into `heap` at the entries the terrain's buffers state them by,
/// where `pass` is fed any of the terrain's: a stage reads an image's size there too.
pub(super) fn extend_heap(pass: &Gta6ShadingPass, heap: &mut Vec<Option<Arc<Image>>>) {
    let mut feeds = pass.mesh.feeds.iter().chain(&pass.pixel.feeds);
    if !feeds.any(|fed| matches!(fed.feed, Filled::Terrain(_))) {
        return;
    }
    for texture in DefaultTexture::ALL {
        let entry = texture.entry() as usize;
        if heap.len() <= entry {
            heap.resize(entry + 1, None);
        }
        heap[entry] = Some(Arc::new(image(texture)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_cli::{Gta6Fed as Fed, Gta6Stage as Stage};

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

    #[test]
    fn a_terrain_pass_s_heap_holds_the_defaults_at_their_entries_and_another_s_does_not() {
        let fed = Fed {
            group: 1,
            binding: 3,
            feed: Filled::Terrain(Terrain::Record),
            header: Some(372),
        };
        let mut heap = vec![None; 4];
        extend_heap(&pass(vec![fed.clone()]), &mut heap);
        for texture in DefaultTexture::ALL {
            let image = heap[texture.entry() as usize].as_ref().expect("an image");
            assert_eq!(image.rgba, texture.texel());
        }
        let other = Fed {
            feed: Filled::Zero(Some(terrain::CELLS)),
            ..fed
        };
        let mut heap = vec![None; 4];
        extend_heap(&pass(vec![other]), &mut heap);
        assert_eq!(heap.len(), 4);
    }
}
