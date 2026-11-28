//! The tile a GTA VI terrain drawable is a cell of: its bounds and material bodies, which the
//! archive beside the drawable's own holds, read once for each such archive on a worker.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use or_io::container::rpf::{Gta6Mount, RpfArchive};
use or_io::gta6::hash::name_hash64;
use or_io::gta6::terrain_tile::{self, TileBounds, TileMaterial};

use crate::state::{AppState, ArchiveSource, DocPayload, Document, RpfNodeBacking, RpfNodeId};

/// A terrain drawable's tile, and which of its cells the drawable is.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Tile {
    pub bounds: TileBounds,
    pub material: TileMaterial,
    /// The drawable's cell, from 1, counted from the south-west.
    pub cell: u32,
}

impl Tile {
    /// The layers whose cell mask names the drawn cell, its bit counted from the south-west or
    /// from the north-west, as body A counts its cells' boxes.
    // ⚠ Which way the masks count is not read; the layers the cell draws come from its index map.
    pub fn masked_layers(&self) -> Vec<usize> {
        let at = self.cell.saturating_sub(1);
        let (column, row) = (at % 8, at / 8);
        let bits = (1u64 << at) | (1u64 << ((7 - row) * 8 + column));
        (self.material.masks.iter().enumerate())
            .filter(|(_, mask)| *mask & bits != 0)
            .map(|(layer, _)| layer)
            .collect()
    }
}

/// Every tile whose bodies one archive of images holds.
type Tiles = Arc<Vec<(TileBounds, TileMaterial)>>;

type Slot = Arc<Mutex<Option<Tiles>>>;

/// The tiles of each archive of images asked for, by where it was asked from: `None` in a slot
/// while a worker reads it.
static READ: Mutex<BTreeMap<String, Slot>> = Mutex::new(BTreeMap::new());

/// How many reads of an archive of images have ended, which a drawable drawn without its tile is
/// prepared again by.
static ENDED: AtomicU64 = AtomicU64::new(0);

/// The tiles said found, so each is logged once.
static SAID: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

const STACK: usize = 8 << 20;

/// How many reads of a tile's archive of images have ended.
pub(crate) fn reads_ended() -> u64 {
    ENDED.load(Ordering::Acquire)
}

/// Where a tile's bodies are read from.
enum Source {
    /// A nested archive's row of the tree: the archive around it, and the images archive's path.
    Tree(Arc<RpfArchive>, String),
    /// A body opened from a scan of the archive file at this path, whose tile's archive is found
    /// by the tile's name.
    Scanned(PathBuf),
}

/// The tile of the active GTA VI drawable, where it is a terrain cell, opened from its archive's
/// tree or from a scan of the archive, and the archive beside its tile's holds its two bodies;
/// `None` too while a worker reads them, which the first ask starts.
pub(crate) fn worn(state: &AppState) -> Option<Arc<Tile>> {
    let doc = state.active_document()?;
    let DocPayload::Gta6Drawable(resource) = &doc.payload else {
        return None;
    };
    let drawable =
        crate::state::gta6_resource::drawable_of(resource, state.selection.selected(doc.id))?;
    let (tile, cell) = terrain_tile::cell_of(&drawable.name)?;
    let (key, source) = source_of(state, doc, tile)?;
    let tiles = tiles_of(key, source)?;
    let (bounds, material) = tiles.iter().find(|(b, _)| b.name == tile)?;
    let said = (SAID.lock().unwrap_or_else(|p| p.into_inner())).insert(tile.to_owned());
    if said {
        tracing::info!(
            tile,
            cell,
            cells = bounds.cells.len(),
            layers = material.layers.len(),
            slot_keys = ?material.slot_keys,
            "found a terrain drawable's tile"
        );
    }
    Some(Arc::new(Tile {
        bounds: bounds.clone(),
        material: material.clone(),
        cell,
    }))
}

/// The keys of the textures the active GTA VI drawable's tile binds for its cell: its slots, and
/// the drawn cell's sub-tiles with the first one, by which the game tells a sub-tiled tile.
pub(in crate::state) fn texture_keys(state: &AppState) -> Vec<u64> {
    let Some(tile) = worn(state) else {
        return Vec::new();
    };
    keys_of(&tile)
}

fn keys_of(tile: &Tile) -> Vec<u64> {
    let name = tile.bounds.name.as_str();
    let slots = (0..=12).map(|slot| terrain_tile::slot_key(&tile.material, name, slot));
    let sub_tile = terrain_tile::sub_tile_of(tile.cell);
    let kinds = terrain_tile::KINDS.len();
    let sub_tiles = (0..kinds).map(|kind| terrain_tile::sub_tile_key(name, sub_tile, kind));
    let first = terrain_tile::sub_tile_key(name, 0, 0);
    let layers = (tile.masked_layers().into_iter())
        .filter_map(|layer| tile.material.textures.get(layer))
        .flat_map(|textures| textures.keys);
    let keys = slots.chain(sub_tiles).chain([first]).chain(layers);
    keys.filter(|key| *key != 0).collect()
}

/// Where the bodies of `tile`, whose cell `doc` is, are read from, and the key they are kept by.
fn source_of(state: &AppState, doc: &Document, tile: &str) -> Option<(String, Source)> {
    let tree = || {
        let binding = doc.rpf_entry_binding.as_ref()?;
        let DocPayload::Rpf(rpf) = &state.document(binding.parent_doc_id)?.payload else {
            return None;
        };
        let (outer, at) = nested_around(&rpf.session, binding.entry_id)?;
        let beside = terrain_tile::images_beside(&outer.entry_path(at)?)?;
        let key = format!(
            "{:x}|{}",
            Arc::as_ptr(&outer) as usize,
            beside.to_lowercase()
        );
        Some((key, Source::Tree(outer, beside)))
    };
    let scanned = || {
        let (archive, _) = crate::state::rpf9_scan::scanned_body(doc.path.as_deref()?)?;
        let key = format!("{}|{tile}", archive.display()).to_lowercase();
        Some((key, Source::Scanned(archive.to_path_buf())))
    };
    tree().or_else(scanned)
}

/// The archive that holds the nested archive the row `row` sits in, and that one's entry in it.
fn nested_around(
    session: &crate::state::RpfEditSession,
    row: RpfNodeId,
) -> Option<(Arc<RpfArchive>, u32)> {
    let mut node = session.node(row)?;
    loop {
        if let RpfNodeBacking::Archive {
            source:
                ArchiveSource::Nested {
                    parent_archive_id,
                    parent_entry_index,
                    ..
                },
            ..
        } = &node.backing
        {
            let parent = session.archive(*parent_archive_id)?;
            return Some((Arc::clone(&parent.archive), *parent_entry_index));
        }
        node = session.node(node.parent?)?;
    }
}

/// The tiles `source` holds, kept by `key`: `None` while a worker reads them, which the first
/// ask starts.
fn tiles_of(key: String, source: Source) -> Option<Tiles> {
    let mut read = READ.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(slot) = read.get(&key) {
        return slot.lock().unwrap_or_else(|p| p.into_inner()).clone();
    }
    let slot = Slot::default();
    read.insert(key.clone(), Arc::clone(&slot));
    drop(read);
    static POOL: OnceLock<or_job::Pool> = OnceLock::new();
    let pool = POOL.get_or_init(|| or_job::Pool::new(2, STACK, || {}));
    let started = pool.run(move || {
        let tiles = Arc::new(match source {
            Source::Tree(outer, beside) => read_tiles(&outer, &beside),
            Source::Scanned(archive) => scanned_tiles(&archive, &key),
        });
        tracing::info!(
            key,
            tiles = tiles.len(),
            "read the terrain tiles of an archive of images"
        );
        *slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(tiles);
        ENDED.fetch_add(1, Ordering::Release);
    });
    if let Err(why) = started {
        tracing::warn!(%why, "no worker reads a terrain tile's bodies");
    }
    None
}

/// The tiles beside the tile named in `key` (after its `|`) of the archive file `archive`, whose
/// table the app's keys open and whose rows the app's names name.
fn scanned_tiles(archive: &Path, key: &str) -> Vec<(TileBounds, TileMaterial)> {
    let tile = key.rsplit('|').next().unwrap_or_default();
    let read = || -> Result<Vec<(TileBounds, TileMaterial)>, String> {
        let keys = crate::state::rpf9_scan::rpf9_table::keys()?;
        let mut file = File::open(archive).map_err(|e| e.to_string())?;
        let top = Gta6Mount::open_root(&mut file, &keys).map_err(|e| e.to_string())?;
        let names = crate::state::rpf9_scan::rpf9_table::resolver();
        let paths = (top.archive.entries.iter()).filter_map(|e| names.lookup(e.name64));
        let beside = (tile_archive(paths, tile).and_then(|p| terrain_tile::images_beside(&p)))
            .ok_or("no named archive of the tile")?;
        let hash = name_hash64(beside.as_bytes());
        let entry = (top.archive.entries.iter().find(|e| e.name64 == hash))
            .ok_or("no archive of images beside the tile's")?;
        let images = (top.open_nested(&mut file, entry, &keys)).map_err(|e| e.to_string())?;
        let bodies = images
            .archive
            .entries
            .iter()
            .filter_map(|entry| match entry.cipher {
                Some(_) => images.read_entry_decrypted(&mut file, entry, &keys).ok(),
                None => images.read_entry(&mut file, entry).ok(),
            });
        Ok(paired(bodies))
    };
    read().unwrap_or_else(|why| {
        tracing::info!(why, tile, "a terrain tile's bodies were not read");
        Vec::new()
    })
}

/// The path of the archive of the tile named `tile` among `paths`: the one whose last name it is.
fn tile_archive<'p>(mut paths: impl Iterator<Item = &'p str>, tile: &str) -> Option<String> {
    let named = |path: &&str| {
        let last = path.rsplit(['/', '\\']).next().unwrap_or(path);
        last.eq_ignore_ascii_case(tile)
    };
    paths.find(named).map(str::to_owned)
}

fn read_tiles(outer: &Arc<RpfArchive>, path: &str) -> Vec<(TileBounds, TileMaterial)> {
    let Some(nested) = outer.find(path).and_then(|at| outer.open_nested(at).ok()) else {
        return Vec::new();
    };
    paired((0..nested.entries.len() as u32).filter_map(|at| nested.read_entry(at).ok()))
}

/// Each tile's bounds among `bodies`, with the material body that is of the same tile.
fn paired(bodies: impl Iterator<Item = Vec<u8>>) -> Vec<(TileBounds, TileMaterial)> {
    let (mut bounds, mut materials) = (Vec::new(), Vec::new());
    for bytes in bodies {
        if let Ok(read) = TileBounds::open(&bytes) {
            bounds.push(read);
        } else if let Ok(read) = TileMaterial::open(&bytes) {
            materials.push(read);
        }
    }
    let paired = |b: TileBounds| {
        let material = materials.iter().find(|m| m.is_of(&b.name))?.clone();
        Some((b, material))
    };
    bounds.into_iter().filter_map(paired).collect()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn a_tile_s_archive_is_the_one_its_name_ends_whatever_the_case() {
        let paths = [
            "levels/a/terrain/g1_g2/n04_w18",
            "levels/a/terrain/g1_g2/g1_g2_terrain_images_merged",
            "levels/a/terrain/g1_g2/N04_W19",
            "levels/a/terrain/g1_g2/n04_w19_metadata",
        ];
        let found = tile_archive(paths.into_iter(), "n04_w19");
        assert_eq!(found.as_deref(), Some("levels/a/terrain/g1_g2/N04_W19"));
        let beside = found.and_then(|p| terrain_tile::images_beside(&p));
        assert_eq!(
            beside.as_deref(),
            Some("levels/a/terrain/g1_g2/g1_g2_terrain_images_merged")
        );
        assert_eq!(tile_archive(paths.into_iter(), "n05_w19"), None);
    }

    #[test]
    fn a_cell_asks_for_its_slots_and_its_own_sub_tiles_packs() {
        let tile = Tile {
            bounds: TileBounds {
                name: "n04_w19".into(),
                corner: [0.0; 3],
                place_second: [0.0; 3],
                bounds: Default::default(),
                cells: Vec::new(),
            },
            material: TileMaterial {
                slot_names: [0; terrain_tile::SLOTS],
                slot_keys: [7; terrain_tile::SLOTS],
                layers: Vec::new(),
                textures: Vec::new(),
                masks: Vec::new(),
            },
            cell: 27,
        };
        let keys = keys_of(&tile);
        let named = |name: &str| name_hash64(name.as_bytes());
        // Cell 27 is column 2 of row 3 from the south-west: the sub-tile two by two cells hold.
        for kind in terrain_tile::KINDS {
            assert!(
                keys.contains(&named(&format!("n04_w19_lod_006_{kind}"))),
                "{kind}"
            );
        }
        assert!(keys.contains(&named("n04_w19_lod_001_ab")));
        assert!(keys.contains(&named("n04_w19_grass_mask")));
        assert!(keys.contains(&7));
    }

    #[test]
    fn a_read_on_a_worker_ends_counted_and_is_kept_for_the_next_ask() {
        let before = reads_ended();
        let key = "made-up archive|none".to_owned();
        let source = || Source::Scanned(PathBuf::from("Z:/no such/archive.rpf"));
        assert_eq!(
            tiles_of(key.clone(), source()),
            None,
            "a worker reads it first"
        );
        let until = Instant::now() + Duration::from_secs(30);
        let kept = loop {
            if let Some(kept) = tiles_of(key.clone(), source()) {
                break kept;
            }
            assert!(Instant::now() < until, "the read never ended");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(kept.is_empty(), "no archive holds no tile");
        assert!(reads_ended() > before);
    }
}
