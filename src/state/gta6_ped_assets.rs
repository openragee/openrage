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

/// What the index makes of a body its scan found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Taken {
    /// A metadata body, which may be an asset list.
    Metadata,
    /// A resource the scan states no signature of, which may be a catalogue.
    Resource,
    /// A resource the scan states a signature of, which no catalogue has: left unread.
    Signed,
    /// Anything else.
    Not,
}

/// Which bodies the index reads, by what the scan states of one.
// Measured over a whole build: every catalogue is a resource whose virtual section the scan
// states no signature of; the navigation meshes, octrees, acceleration structures, dictionaries,
// cloth and drawables that make up most of an archive's bodies all have one.
fn taken(content: Rpf9Content, signed: bool, resource: bool) -> Taken {
    match content {
        Rpf9Content::Pso => Taken::Metadata,
        Rpf9Content::ResourceShaped if resource && signed => Taken::Signed,
        Rpf9Content::ResourceShaped if resource => Taken::Resource,
        _ => Taken::Not,
    }
}

/// What the archive at `path` holds of the index, from its scan; `None` where the scan does not
/// read the archive whole.
fn read_archive(path: &Path) -> Option<Index> {
    let mut out = Index::default();
    let Ok(Some(scan)) = Rpf9ScanDocument::open(path) else {
        return None;
    };
    let mut signed = 0usize;
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
                    .filter_map(|find| {
                        let Rpf9BodyKind::Zlib(run) = find.kind else {
                            return None;
                        };
                        let signature = run.signature.is_some();
                        match taken(run.content, signature, run.resource.is_some()) {
                            Taken::Metadata => Some((find.offset, true)),
                            Taken::Resource => Some((find.offset, false)),
                            Taken::Signed => {
                                signed += 1;
                                None
                            }
                            Taken::Not => None,
                        }
                    })
                    .collect()
            }
            Some(_) => return None,
            None => {}
        }
        drop(found);
        std::thread::sleep(LOOK);
    };
    tracing::info!(
        archive = %path.display(),
        read = bodies.len(),
        signed,
        "the ped index reads these bodies of an archive and leaves the resources the scan states a signature of"
    );
    let mut reader = File::open(path)
        .ok()
        .and_then(|f| Rpf9BodyReader::new(f).ok())?;
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
    Some(out)
}

/// The version of what is kept of a ped index: a change to it takes another.
const KEPT_VERSION: u32 = 1;
/// The most a kept ped index inflates to.
const KEPT_BYTES: usize = 1 << 30;

/// What one archive gave the index, with its size and modification time when it was read.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct KeptArchive {
    stat: (u64, u64),
    catalogues: Vec<PedCatalogue>,
    lists: Vec<PedAssetList>,
}

/// The ped index of one install as kept, each archive by its path under the install.
#[derive(serde::Serialize, serde::Deserialize)]
struct Kept {
    version: u32,
    root: String,
    archives: BTreeMap<String, KeptArchive>,
}

/// The file the index of `root` is kept in, beside its kept texture indexes.
fn kept_file(root: &Path) -> Option<PathBuf> {
    let name = or_io::gta6::hash::name_hash64(root.to_string_lossy().as_bytes());
    let folder = crate::state::prefs::config_root()?.join("gta6_texture_indexes");
    Some(folder.join(format!("{name:016x}.ped_index")))
}

/// What `archives` keep for the install at `root`, sealed.
fn encode(root: &Path, archives: BTreeMap<String, KeptArchive>) -> Option<Vec<u8>> {
    let kept = Kept {
        version: KEPT_VERSION,
        root: root.to_string_lossy().into_owned(),
        archives,
    };
    let plain = serde_json::to_vec(&kept).ok()?;
    or_io::container::rpf::seal_rpf9_scan(&plain).ok()
}

/// The archives `sealed` keeps for the install at `root`; none for another version or install,
/// or bytes that do not read.
fn decode(root: &Path, sealed: &[u8]) -> BTreeMap<String, KeptArchive> {
    let plain = or_io::container::rpf::unseal_rpf9_scan(sealed, KEPT_BYTES).ok();
    let kept: Option<Kept> = plain.and_then(|plain| serde_json::from_slice(&plain).ok());
    (kept.filter(|k| k.version == KEPT_VERSION && k.root == root.to_string_lossy()))
        .map_or_else(BTreeMap::new, |k| k.archives)
}

/// What is kept of the archive `name` while its size and modification time are still `now`.
fn fresh<'k>(
    kept: &'k BTreeMap<String, KeptArchive>,
    name: &str,
    now: Option<(u64, u64)>,
) -> Option<&'k KeptArchive> {
    kept.get(name).filter(|archive| Some(archive.stat) == now)
}

/// The index of the install or folder at `root`: each archive's part as kept while the archive
/// is as it was kept, else read from it; what changed is kept again.
fn build(root: &Path) -> Index {
    let started = std::time::Instant::now();
    let archives = or_title::install::gta6_files::list(root).archive_paths();
    let file = kept_file(root);
    let kept = (file.as_ref())
        .and_then(|file| std::fs::read(file).ok())
        .map_or_else(BTreeMap::new, |sealed| decode(root, &sealed));
    let (mut index, mut keep) = (Index::default(), BTreeMap::new());
    let (mut read, mut reused) = (0, 0);
    for path in archives {
        let stat = crate::state::gta6_texture_index::stat(&path);
        if stat.is_none_or(|s| s.0 > MOST_ARCHIVE) {
            continue;
        }
        let name = path.strip_prefix(root).unwrap_or(&path);
        let name = name.to_string_lossy().into_owned();
        let part = match (fresh(&kept, &name, stat), stat) {
            (Some(part), _) => {
                reused += 1;
                part.clone()
            }
            (None, Some(stat)) => {
                read += 1;
                let Some(found) = read_archive(&path) else {
                    continue;
                };
                KeptArchive {
                    stat,
                    catalogues: found.catalogues,
                    lists: found.lists,
                }
            }
            (None, None) => continue,
        };
        index.catalogues.extend(part.catalogues.iter().cloned());
        index.lists.extend(part.lists.iter().cloned());
        keep.insert(name, part);
    }
    if let (Some(file), true) = (file, read > 0) {
        let sealed = encode(root, keep).ok_or_else(|| std::io::Error::other("not encoded"));
        let kept = sealed.and_then(|sealed| {
            (file.parent().map_or(Ok(()), std::fs::create_dir_all))
                .and_then(|()| std::fs::write(&file, sealed))
        });
        if let Err(error) = kept {
            tracing::warn!(%error, "the ped index was not kept");
        }
    }
    tracing::info!(
        catalogues = index.catalogues.len(),
        lists = index.lists.len(),
        archives_read = read,
        archives_kept = reused,
        elapsed_ms = started.elapsed().as_millis() as u64,
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

#[cfg(test)]
mod tests {
    use or_io::gta6::ped_assets::{TextureEntry, BASE, WRINKLE_MASK};
    use or_io::gta6::ped_catalogue::{AssetKit, AssetState};

    use super::*;

    #[test]
    fn a_kept_archive_s_part_is_used_only_while_the_archive_is_as_it_was_kept() {
        let root = Path::new("K:/made/up/install");
        let catalogue = PedCatalogue {
            asset_lists: vec![vec![AssetKit {
                id: 7,
                template_name: 0,
                category: 2,
                states: vec![AssetState {
                    name: BASE,
                    drawable: 0xD1,
                    albedo: 0xA1,
                    normal: 0xB1,
                    material: 0xC1,
                    more: vec![(3, 0xF3)],
                }],
            }]],
            set_lists: Vec::new(),
        };
        let mut list = PedAssetList::default();
        let entry = TextureEntry {
            word: 0xF2,
            names: vec![(WRINKLE_MASK, 0xF3)],
        };
        list.wrinkles.insert(0xF2, entry);
        let part = KeptArchive {
            stat: (4096, 77),
            catalogues: vec![catalogue],
            lists: vec![list],
        };
        let archives = BTreeMap::from([("packs.rpf".to_owned(), part.clone())]);
        let sealed = encode(root, archives).expect("encoded");
        let kept = decode(root, &sealed);
        assert_eq!(fresh(&kept, "packs.rpf", Some((4096, 77))), Some(&part));
        // A size or a time that moved, an archive gone or not kept: read again.
        assert_eq!(fresh(&kept, "packs.rpf", Some((4096, 78))), None);
        assert_eq!(fresh(&kept, "packs.rpf", Some((4097, 77))), None);
        assert_eq!(fresh(&kept, "packs.rpf", None), None);
        assert_eq!(fresh(&kept, "data.rpf", Some((4096, 77))), None);
        // Another install's, or bytes that do not read, keep nothing.
        assert!(decode(Path::new("K:/other"), &sealed).is_empty());
        assert!(decode(root, &sealed[..sealed.len() / 2]).is_empty());
    }

    #[test]
    fn the_index_reads_metadata_and_unsigned_resources_and_leaves_signed_ones() {
        use Rpf9Content::{Pso, ResourceShaped};
        // A catalogue: a resource the scan states no signature of.
        assert_eq!(taken(ResourceShaped, false, true), Taken::Resource);
        // A navigation mesh, an acceleration structure, a dictionary: each has a signature.
        assert_eq!(taken(ResourceShaped, true, true), Taken::Signed);
        assert_eq!(taken(Pso, false, false), Taken::Metadata);
        assert_eq!(taken(ResourceShaped, false, false), Taken::Not);
    }

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

/// The skin tones of an install, read apart from this index.
#[path = "gta6_skin_tones.rs"]
pub(crate) mod skin_tones;
