//! The ped catalogues and asset lists of the install a GTA VI drawable is in, and what the active
//! drawable joins to through them: the bodies found in the scans of the install's archives and
//! read once a session on a worker.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use or_io::container::rpf::{rpf9_scan::Rpf9BodyReader, Rpf9BodyKind, Rpf9Content};
use or_io::gta6::ped_assets::{self, PedAssetList, PedTextures};
use or_io::gta6::ped_catalogue::{self, PedCatalogue};

use crate::state::rpf9_scan::{Ended, Row as ScanRow, Rpf9ScanDocument};

/// The largest body read.
const BODY_LIMIT: u64 = 64 << 20;
/// An archive past this is not read for the bodies, as the layered materials' index leaves it.
const MOST_ARCHIVE: u64 = 8 << 30;
/// How often the worker looks at the scan of the archive in hand.
const LOOK: Duration = Duration::from_millis(50);

/// The catalogues and asset lists found, in the order the archives and their bodies are found.
#[derive(Debug, Default)]
pub(crate) struct Index {
    pub catalogues: Vec<PedCatalogue>,
    pub lists: Vec<PedAssetList>,
}

/// What the active GTA VI drawable joins to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Worn {
    /// A worker still reads the index of its install.
    Reading,
    /// The state that wears it and its textures' keys; `None` where no state wears it.
    Joined(Option<PedTextures>),
}

/// What the archive at `path` holds of the index, added to `out`, from its scan; nothing where
/// the scan does not read the archive whole.
// ponytail: every resource-shaped and metadata body is read each session to find the few that
// are the index's; keep their offsets as the layered materials' index does when the first ask
// of a session is seen to take long.
fn read_archive(path: &Path, out: &mut Index) {
    let Ok(Some(scan)) = Rpf9ScanDocument::open(path) else {
        return;
    };
    let bodies: Vec<(u64, bool)> = loop {
        let found = scan.found();
        match &found.ended {
            Some(Ended::Whole) => {
                break (found.rows.iter())
                    .filter_map(|row| match row {
                        ScanRow::Body(find) => Some(find),
                        ScanRow::Missing { .. } => None,
                    })
                    .filter(|find| find.inflated_len.is_none_or(|len| len <= BODY_LIMIT))
                    .filter_map(|find| match find.kind {
                        Rpf9BodyKind::Zlib(run) if run.content == Rpf9Content::Pso => {
                            Some((find.offset, true))
                        }
                        Rpf9BodyKind::Zlib(run)
                            if run.content == Rpf9Content::ResourceShaped
                                && run.resource.is_some() =>
                        {
                            Some((find.offset, false))
                        }
                        _ => None,
                    })
                    .collect()
            }
            Some(_) => return,
            None => {}
        }
        drop(found);
        std::thread::sleep(LOOK);
    };
    let Some(mut reader) = File::open(path)
        .ok()
        .and_then(|f| Rpf9BodyReader::new(f).ok())
    else {
        return;
    };
    let schema = crate::state::document::meta_schema();
    for (offset, pso) in bodies {
        let Ok((find, bytes)) = reader.read(offset, BODY_LIMIT) else {
            continue;
        };
        if pso {
            let list = (ped_assets::is_asset_list(&bytes))
                .then(|| or_io::meta::binary::parse_meta_binary(schema, &bytes).ok())
                .flatten()
                .and_then(|root| ped_assets::read_tree(&root).ok());
            out.lists.extend(list);
            continue;
        }
        let Rpf9BodyKind::Zlib(run) = find.kind else {
            continue;
        };
        let Some(resource) = run.resource else {
            continue;
        };
        let (virtual_section, physical) = resource.sections(&bytes);
        if ped_catalogue::is_ped_catalogue(virtual_section) {
            out.catalogues
                .extend(ped_catalogue::open(virtual_section, physical).ok());
        }
    }
}

/// The index of the install or folder at `root`, read from its archives.
fn build(root: &Path) -> Index {
    let archives = or_title::install::gta6_files::list(root).archive_paths();
    let mut index = Index::default();
    for path in archives {
        let small =
            crate::state::gta6_texture_index::stat(&path).is_some_and(|s| s.0 <= MOST_ARCHIVE);
        if small {
            read_archive(&path, &mut index);
        }
    }
    tracing::info!(
        catalogues = index.catalogues.len(),
        lists = index.lists.len(),
        "read the ped catalogues and asset lists of {}",
        root.display()
    );
    index
}

type Slot = Arc<Mutex<Option<Arc<Index>>>>;

static BUILT: Mutex<BTreeMap<PathBuf, Slot>> = Mutex::new(BTreeMap::new());

/// The index of the install or folder at `root`: `None` while a worker reads it, which the first
/// ask starts.
fn under(root: &Path) -> Option<Arc<Index>> {
    let mut built = BUILT.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(slot) = built.get(root) {
        return slot.lock().unwrap_or_else(|p| p.into_inner()).clone();
    }
    let slot = Slot::default();
    built.insert(root.to_path_buf(), Arc::clone(&slot));
    let root = root.to_path_buf();
    // On the maps' workers: the count of maps in moves once it is read, so a scene prepared
    // while it was read binds what it gives.
    crate::render::gta6_maps::decode(move || {
        let index = Arc::new(build(&root));
        *slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(index);
    });
    None
}

/// What the drawable named `name` joins to in `index`.
pub(crate) fn joined(index: &Index, name: &str) -> Option<PedTextures> {
    let word = ped_assets::drawable_word(name);
    ped_assets::join(word, &index.catalogues, &index.lists)
}

/// What the active GTA VI drawable joins to; `None` where the document shows no such drawable
/// in a folder. The first ask has a worker read the index of its install.
pub(crate) fn worn(state: &crate::state::AppState) -> Option<Worn> {
    let doc = state.active_document()?;
    let crate::state::DocPayload::Gta6Drawable(resource) = &doc.payload else {
        return None;
    };
    let selection = state.selection.selected(doc.id);
    let drawable = crate::state::gta6_resource::drawable_of(resource, selection)?;
    let root = crate::state::gta6_texture_index::root_of(state, doc)?;
    Some(match under(&root) {
        Some(index) => Worn::Joined(joined(&index, &drawable.name)),
        None => Worn::Reading,
    })
}

// Declared here: the state module is at its line budget.
#[path = "gta6_skin_tones.rs"]
pub(crate) mod skin_tones;

#[cfg(test)]
mod tests {
    use or_io::gta6::ped_assets::{TextureEntry, BASE, WRINKLE_MASK};
    use or_io::gta6::ped_catalogue::{AssetKit, AssetState};

    use super::*;

    #[test]
    fn a_drawable_joins_by_its_name_to_the_state_that_wears_it() {
        let drawable = or_io::gta6::hash::name_hash64(b"zq_hat_000");
        let state = AssetState {
            name: BASE,
            drawable,
            albedo: 0xA1,
            normal: 0,
            material: 0,
            more: vec![(0xE125_85D1, 0xF2)],
        };
        let mut list = PedAssetList::default();
        list.albedos.insert(
            0xA1,
            TextureEntry {
                word: 0xA1,
                names: Vec::new(),
            },
        );
        let mask = TextureEntry {
            word: 0xF2,
            names: vec![(WRINKLE_MASK, 0xF3)],
        };
        list.wrinkles.insert(0xF2, mask);
        let index = Index {
            catalogues: vec![PedCatalogue {
                asset_lists: vec![vec![AssetKit {
                    id: 1,
                    template_name: 0,
                    category: 0,
                    states: vec![state],
                }]],
                set_lists: Vec::new(),
            }],
            lists: vec![list],
        };
        let joined = joined(&index, "ZQ_Hat_000.#dr").expect("worn");
        assert_eq!((joined.state, joined.key(WRINKLE_MASK)), (BASE, Some(0xF3)));
        assert!(joined.listed && !joined.wrinkled);
        assert_eq!(super::joined(&index, "zq_hat_001.#dr"), None);
    }
}
