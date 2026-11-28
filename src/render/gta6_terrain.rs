//! A terrain tile's buffers and textures: the defaults the game binds for a tile whose own record
//! is not ready, and a tile's own once the editor has read it. The binder names each binding.

use std::collections::BTreeMap;
use std::sync::Arc;

use or_cli::gta6_terrain::{self as terrain, DefaultTexture, Terrain};
use or_cli::{Gta6Fed as Fed, Gta6Feed as Filled, Gta6ShadingPass};
use or_io::gta6::terrain_tile as tile_data;

use crate::state::gta6_shaders::terrain_tile::Tile;
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
        Terrain::Texture(texture) => return Some(texel(texture)),
        Terrain::Slot(offset) => {
            return Some(texel(
                terrain::default_at(offset).unwrap_or(DefaultTexture::Black),
            ))
        }
        Terrain::SubTile(_) => return Some(texel(DefaultTexture::Clear)),
        Terrain::Layer(kind) => return Some(texel(unheld_texture(kind))),
        // Bound only with a tile of the model's own, by [`fill`].
        Terrain::Cells | Terrain::Feedback => return None,
    };
    Some(bytes(header, &words))
}

fn texel(texture: DefaultTexture) -> Feed {
    Feed::Textures(vec![Some(Arc::new(image(texture)))])
}

/// The texture a layer's texture of `kind` stands at until it streams in.
fn unheld_texture(kind: u8) -> DefaultTexture {
    match kind {
        1 => DefaultTexture::Flat,
        _ => DefaultTexture::Black,
    }
}

/// The side every slice of a layer array is made to.
const LAYER_SIDE: u32 = 512;

/// The layers the pixels of cell `cell` (from 1, from the south-west) select: each index the
/// index map states there whose weight in the weight map is above nought, the heaviest first;
/// `None` without both maps.
// The maps cover the tile, north at the top; the stage blends a cell's four indices by the three
// weights and what they leave of one.
fn cell_layers(indices: Option<&Image>, weights: Option<&Image>, cell: u32) -> Option<Vec<usize>> {
    let (indices, weights) = (indices?, weights?);
    let whole = |i: &Image| i.rgba.len() == (i.width * i.height * 4) as usize && i.width > 0;
    if !whole(indices) || !whole(weights) {
        return None;
    }
    let at = cell.saturating_sub(1);
    let (column, row) = (at % 8, 7 - at / 8);
    let texel = |image: &Image, u: f32, v: f32| {
        let x = ((u * image.width as f32) as u32).min(image.width - 1);
        let y = ((v * image.height as f32) as u32).min(image.height - 1);
        let k = ((y * image.width + x) * 4) as usize;
        [0, 1, 2, 3].map(|c| image.rgba[k + c])
    };
    let steps = (indices.width / 8).clamp(1, 256);
    let mut used = BTreeMap::<usize, u64>::new();
    for j in 0..steps {
        for i in 0..steps {
            let u = (column as f32 + (i as f32 + 0.5) / steps as f32) / 8.0;
            let v = (row as f32 + (j as f32 + 0.5) / steps as f32) / 8.0;
            let [r, g, b, _] = texel(weights, u, v);
            let rest = 255u32.saturating_sub(u32::from(r) + u32::from(g) + u32::from(b));
            let picked = texel(indices, u, v);
            for (index, weight) in picked.into_iter().zip([r.into(), g.into(), b.into(), rest]) {
                *used.entry(usize::from(index)).or_default() += u64::from(weight);
            }
        }
    }
    // The layers that weigh most first, so that those past the arrays' slices weigh least.
    let mut used: Vec<(usize, u64)> = used.into_iter().filter(|(_, w)| *w > 0).collect();
    used.sort_by_key(|(layer, weight)| (std::cmp::Reverse(*weight), *layer));
    Some(used.into_iter().map(|(layer, _)| layer).collect())
}

/// `image` at [`LAYER_SIDE`] texels a side: its level of that size where it has one, else its
/// texels taken nearest. Kept by the image's place in memory, so a surface prepared again binds
/// the same.
fn layer_slice(image: &Arc<Image>) -> Arc<Image> {
    type Kept = std::collections::HashMap<usize, (Arc<Image>, Arc<Image>)>;
    static KEPT: std::sync::Mutex<Option<Kept>> = std::sync::Mutex::new(None);
    let place = Arc::as_ptr(image) as usize;
    let mut kept = KEPT.lock().unwrap_or_else(|p| p.into_inner());
    let kept = kept.get_or_insert_with(Default::default);
    if let Some((_, slice)) = kept.get(&place) {
        return Arc::clone(slice);
    }
    let side = LAYER_SIDE;
    let level = match (image.width, image.height) {
        (w, h) if (w, h) == (side, side) => None,
        (w, h) if w > side && w == h && w.is_power_of_two() => {
            let levels = or_renderer::game_shaders::mips::chain(w, h, &image.rgba, image.srgb);
            levels
                .into_iter()
                .find(|(w, _, _)| *w == side)
                .map(|(_, _, rgba)| rgba)
        }
        _ => None,
    };
    let rgba = level.unwrap_or_else(|| {
        let (w, h) = (image.width.max(1), image.height.max(1));
        let mut out = Vec::with_capacity((side * side * 4) as usize);
        for y in 0..side {
            for x in 0..side {
                let k = (((y * h / side) * w + x * w / side) * 4) as usize;
                out.extend_from_slice(image.rgba.get(k..k + 4).unwrap_or(&[0, 0, 0, 255]));
            }
        }
        out
    });
    let slice = Arc::new(Image {
        width: side,
        height: side,
        rgba,
        srgb: image.srgb,
    });
    // The source is kept with its slice: no other image comes to lie where it does meanwhile.
    kept.insert(place, (Arc::clone(image), Arc::clone(&slice)));
    slice
}

/// The slices of one kind's layer array.
type Slices = Vec<Option<Arc<Image>>>;

/// The drawn cell's layers' textures, an array a kind, and each layer's three numbers: a slice's
/// where its texture is held, else the default's, as the game numbers a layer not streamed in.
fn layer_arrays(
    tile: &Tile,
    used: &[usize],
    found: &mut dyn FnMut(u64) -> Option<Arc<Image>>,
) -> ([Slices; 3], Vec<[u32; 3]>) {
    let mut arrays: [Slices; 3] = Default::default();
    let mut numbers = vec![terrain::unheld_layer(); tile.material.layers.len()];
    for layer in used.iter().copied() {
        let (Some(textures), Some(number)) =
            (tile.material.textures.get(layer), numbers.get_mut(layer))
        else {
            continue;
        };
        for (kind, array) in arrays.iter_mut().enumerate() {
            let full = array.len() as u32 >= terrain::LAYER_SLICES;
            let Some(image) = found(textures.keys[kind]).filter(|_| !full) else {
                continue;
            };
            let slice = layer_slice(&image);
            // An array has one colour space, the first slice's: another stays at the default.
            if let Some(Some(first)) = array.first() {
                if first.srgb != slice.srgb {
                    continue;
                }
            }
            number[kind] = terrain::layer_entry(kind as u8, array.len() as u32);
            array.push(Some(slice));
        }
    }
    (arrays, numbers)
}

fn bytes(header: Option<u32>, words: &[u32]) -> Feed {
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    Feed::Bytes(super::gta6_meshlets::headed(header, &bytes))
}

/// Whether `pass` is fed any of a terrain tile's buffers or textures.
pub(super) fn reads_a_tile(pass: &Gta6ShadingPass) -> bool {
    let mut feeds = pass.mesh.feeds.iter().chain(&pass.pixel.feeds);
    feeds.any(|fed| matches!(fed.feed, Filled::Terrain(_)))
}

/// Fill `feeds`, the bindings of a surface `pass` draws, with what the game binds for a tile of
/// its own: `tile`'s record, layers and cells, and the textures its slots and sub-tiles name,
/// which `map` finds by key. `Err` says why the surface draws as Material: it reads the cells
/// and the model is no cell of a tile the editor read. With no tile, the defaults stay.
// The layers a cell's pixels select are given their textures in three arrays, one a kind, whose
// slices their records number; every other layer keeps the defaults, as the game numbers a layer
// that has not streamed in.
pub(super) fn fill(
    pass: &Gta6ShadingPass,
    tile: Option<&Tile>,
    map: &mut dyn FnMut(u64) -> Option<Arc<Image>>,
    feeds: &mut BTreeMap<(u32, u32), Feed>,
) -> Result<(), String> {
    let fed: Vec<_> = (pass.mesh.feeds.iter().chain(&pass.pixel.feeds))
        .filter(|fed| matches!(fed.feed, Filled::Terrain(_) | Filled::Heap { .. }))
        .collect();
    let terrain_of = |fed: &&Fed| match fed.feed {
        Filled::Terrain(part) => Some(part),
        _ => None,
    };
    if !fed.iter().any(|fed| terrain_of(fed).is_some()) {
        return Ok(());
    }
    let Some(tile) = tile else {
        return match fed
            .iter()
            .any(|fed| terrain_of(fed) == Some(Terrain::Cells))
        {
            true => Err(terrain::no_tile()),
            false => Ok(()),
        };
    };
    let name = tile.bounds.name.as_str();
    let mut found = |key: u64| if key == 0 { None } else { map(key) };
    // The game takes sixteen sub-tiles unless slot 0's texture is there and the first sub-tile's is not.
    let sub_tiled = !(found(tile_data::slot_key(&tile.material, name, 0)).is_some()
        && found(tile_data::sub_tile_key(name, 0, 0)).is_none());
    let sub_tile = tile_data::sub_tile_of(tile.cell);
    let mut slots: [Option<Arc<Image>>; 14] = Default::default();
    for (slot, held) in slots.iter_mut().enumerate() {
        let kind = terrain::KIND_SLOTS.iter().position(|s| *s == slot);
        let key = match kind {
            Some(kind) if sub_tiled => tile_data::sub_tile_key(name, sub_tile, kind),
            _ => tile_data::slot_key(&tile.material, name, slot),
        };
        *held = found(key);
    }
    let record = terrain::tile_record(tile.bounds.corner, sub_tiled, sub_tile, |slot| {
        slots.get(slot).is_some_and(Option::is_some)
    });
    // The layers the cell's pixels select; the masks' where the maps are not in yet.
    let used = cell_layers(slots[4].as_deref(), slots[3].as_deref(), tile.cell)
        .unwrap_or_else(|| tile.masked_layers());
    let (arrays, numbers) = layer_arrays(tile, &used, &mut found);
    let layers: Vec<u32> = (tile.material.layers.iter().zip(&numbers))
        .flat_map(|(params, numbers)| terrain::tile_layer(*params, *numbers).words())
        .collect();
    let kind_image = |kind: u8| {
        slots
            .get(*terrain::KIND_SLOTS.get(usize::from(kind))?)?
            .clone()
    };
    for fed in fed {
        let at = (fed.group, fed.binding);
        let filled = match fed.feed {
            Filled::Terrain(Terrain::Record) => bytes(fed.header, &record.words()),
            Filled::Terrain(Terrain::Layers) => bytes(fed.header, &layers),
            Filled::Terrain(Terrain::Cells) => {
                bytes(fed.header, &[terrain::UNMAPPED; tile_data::CELLS])
            }
            Filled::Terrain(Terrain::Slot(offset)) => {
                let slot = terrain::slot_at(offset);
                match slot.and_then(|slot| slots[slot].clone()) {
                    Some(image) => Feed::Textures(vec![Some(image)]),
                    None => match slot.and_then(terrain::unheld) {
                        Some(texture) => texel(texture),
                        // Number nought picks the heap's blank entry.
                        None => Feed::Textures(vec![None]),
                    },
                }
            }
            Filled::Terrain(Terrain::SubTile(kind)) => match kind_image(kind) {
                Some(image) => Feed::Textures(vec![Some(image)]),
                None => continue,
            },
            Filled::Terrain(Terrain::Layer(kind)) => match arrays.get(usize::from(kind)) {
                Some(array) if !array.is_empty() => Feed::Textures(array.clone()),
                _ => texel(unheld_texture(kind)),
            },
            // The heap bound whole: a tile's sub-tile textures at their entries.
            Filled::Heap { .. } => {
                let Some(Feed::Textures(heap)) = feeds.get_mut(&at) else {
                    continue;
                };
                for kind in 0..terrain::SUB_TILE_KINDS {
                    let (entry, image) = (terrain::sub_tile_entry(kind) as usize, kind_image(kind));
                    if image.is_some() && heap.len() > entry {
                        heap[entry] = image;
                    }
                }
                continue;
            }
            _ => continue,
        };
        feeds.insert(at, filled);
    }
    Ok(())
}

/// Put the default textures into `heap` at the entries the terrain's buffers state them by,
/// where `pass` is fed any of the terrain's: a stage reads an image's size there too.
pub(super) fn extend_heap(pass: &Gta6ShadingPass, heap: &mut Vec<Option<Arc<Image>>>) {
    let mut feeds = pass.mesh.feeds.iter().chain(&pass.pixel.feeds);
    if !feeds.any(|fed| matches!(fed.feed, Filled::Terrain(_))) {
        return;
    }
    let sub_tiles =
        (0..terrain::SUB_TILE_KINDS).map(|k| (terrain::sub_tile_entry(k), DefaultTexture::Clear));
    let entries = DefaultTexture::ALL
        .map(|t| (t.entry(), t))
        .into_iter()
        .chain(sub_tiles);
    for (entry, texture) in entries {
        let entry = entry as usize;
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
            then: None,
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

    fn tile() -> Tile {
        let cell = tile_data::Bounds::default();
        let mut slot_keys = [0u64; tile_data::SLOTS];
        slot_keys[3] = 0x33;
        Tile {
            bounds: tile_data::TileBounds {
                name: "n04_w19".into(),
                corner: [-9728.0, 1536.0, -299.75],
                place_second: [0.0; 3],
                bounds: cell,
                cells: vec![cell; tile_data::CELLS],
            },
            material: tile_data::TileMaterial {
                slot_names: [0; tile_data::SLOTS],
                slot_keys,
                layers: vec![[0.5; tile_data::LAYER_PARAMS]; 2],
                textures: vec![Default::default(); 2],
                masks: vec![1, 2],
            },
            // Cell 27 is in sub-tile 5.
            cell: 27,
        }
    }

    fn fed(binding: u32, part: Terrain, header: Option<u32>) -> Fed {
        Fed {
            group: 1,
            binding,
            feed: Filled::Terrain(part),
            header,
        }
    }

    #[test]
    fn a_tile_s_own_record_layers_cells_and_textures_are_fed_at_their_bindings() {
        let feeds = vec![
            fed(0, Terrain::Record, Some(372)),
            fed(1, Terrain::Layers, Some(128)),
            fed(2, Terrain::Cells, Some(4)),
            fed(3, Terrain::Slot(320), None),
            fed(4, Terrain::Slot(332), None),
            fed(5, Terrain::SubTile(0), None),
        ];
        let colour = |rgba: u8| {
            Arc::new(Image {
                width: 1,
                height: 1,
                rgba: vec![rgba; 4],
                srgb: false,
            })
        };
        // The weights (slot 3) and sub-tile 5's albedo are held; nothing else is.
        let sub_tile = tile_data::sub_tile_key("n04_w19", 5, 0);
        let mut map = |key: u64| match key {
            0x33 => Some(colour(3)),
            key if key == sub_tile => Some(colour(9)),
            _ => None,
        };
        let mut out = BTreeMap::new();
        fill(&pass(feeds), Some(&tile()), &mut map, &mut out).expect("the tile is fed");
        let words = |at: u32| match out.get(&(1, at)) {
            Some(Feed::Bytes(bytes)) => (bytes[16..].chunks(4))
                .map(|w| u32::from_le_bytes(w.try_into().expect("a dword")))
                .collect::<Vec<_>>(),
            other => panic!("binding {at} is {other:?}"),
        };
        let record = words(0);
        assert_eq!(record.len(), 93);
        assert_eq!(f32::from_bits(record[12]), -9728.0, "the tile's corner");
        assert_eq!(
            record[16 + 5],
            terrain::sub_tile_entry(0),
            "sub-tile 5 of the first block"
        );
        assert_eq!(record[90], 15, "sixteen sub-tiles");
        assert_eq!(words(1).len(), 2 * 32, "both layers");
        assert_eq!(words(2), [terrain::UNMAPPED; 64]);
        let rgba = |at: u32| match out.get(&(1, at)) {
            Some(Feed::Textures(images)) => images[0].as_ref().map(|i| i.rgba.clone()),
            other => panic!("binding {at} is {other:?}"),
        };
        assert_eq!(rgba(3), Some(vec![3; 4]), "the held weight map");
        assert_eq!(
            rgba(4),
            Some(DefaultTexture::Clear.texel().to_vec()),
            "a slot not held"
        );
        assert_eq!(rgba(5), Some(vec![9; 4]), "the drawn sub-tile's albedo");
    }

    #[test]
    fn a_stage_that_reads_the_cells_of_a_model_with_no_tile_is_refused_and_another_keeps_the_defaults(
    ) {
        let mut map = |_: u64| -> Option<Arc<Image>> { None };
        let mut out = BTreeMap::new();
        let cells = pass(vec![fed(2, Terrain::Cells, Some(4))]);
        assert_eq!(
            fill(&cells, None, &mut map, &mut out),
            Err(terrain::no_tile())
        );
        let record = pass(vec![fed(0, Terrain::Record, Some(372))]);
        assert_eq!(fill(&record, None, &mut map, &mut out), Ok(()));
        assert!(
            out.is_empty(),
            "the defaults are what the binder's feeds give"
        );
    }

    #[test]
    fn the_layers_a_cell_s_maps_select_take_slices_of_the_layer_arrays_and_the_rest_the_defaults() {
        let mut tile = tile();
        tile.material.slot_keys[4] = 0x44;
        tile.material.layers = vec![[0.5; tile_data::LAYER_PARAMS]; 3];
        tile.material.masks = vec![0; 3];
        tile.material.textures = (0..3u64)
            .map(|layer| tile_data::LayerTextures {
                names: [0; 3],
                keys: [0x100 + layer, 0x200 + layer, 0x300 + layer],
            })
            .collect();
        let map_of = |rgba: [u8; 4], side: u32| {
            Arc::new(Image {
                width: side,
                height: side,
                rgba: rgba.repeat((side * side) as usize),
                srgb: false,
            })
        };
        // Every texel of the cell indexes layers 2 and 0, weighted 1 and nought: layer 2 alone.
        let (indices, weights) = (map_of([2, 0, 1, 1], 8), map_of([255, 0, 0, 0], 8));
        let mut map = |key: u64| match key {
            0x44 => Some(Arc::clone(&indices)),
            0x33 => Some(Arc::clone(&weights)),
            0x102 | 0x202 | 0x302 => Some(map_of([9; 4], 1024)),
            _ => None,
        };
        let feeds = vec![
            fed(1, Terrain::Layers, Some(128)),
            fed(2, Terrain::Layer(0), None),
            fed(3, Terrain::Layer(1), None),
        ];
        let mut out = BTreeMap::new();
        fill(&pass(feeds), Some(&tile), &mut map, &mut out).expect("the tile is fed");
        let Some(Feed::Bytes(bytes)) = out.get(&(1, 1)) else {
            panic!("the layers are bytes");
        };
        let words: Vec<u32> = (bytes[16..].chunks(4))
            .map(|w| u32::from_le_bytes(w.try_into().expect("a dword")))
            .collect();
        let albedo = |layer: usize| words[layer * 32 + 28];
        assert_eq!(
            albedo(2),
            terrain::layer_entry(0, 0),
            "layer 2's albedo, slice nought"
        );
        assert_eq!(words[2 * 32 + 29], terrain::layer_entry(1, 0), "its normal");
        assert_eq!(
            albedo(0),
            DefaultTexture::Black.entry(),
            "a layer weighted nought"
        );
        assert_eq!(
            albedo(1),
            DefaultTexture::Black.entry(),
            "a layer not indexed"
        );
        let Some(Feed::Textures(slices)) = out.get(&(1, 2)) else {
            panic!("the albedo array is textures");
        };
        let slice = slices[0].as_ref().expect("a slice");
        assert_eq!(
            (slices.len(), slice.width, slice.height),
            (1, LAYER_SIDE, LAYER_SIDE)
        );
        assert_eq!(&slice.rgba[..4], &[9; 4]);
    }
}
