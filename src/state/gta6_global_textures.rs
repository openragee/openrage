//! The textures of an install's texture archive that the game binds by name whatever is drawn:
//! each found by a scan of the archive and read once a session on a worker, slice by slice.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use or_io::container::rpf::rpf9_scan::Rpf9BodyReader;
use or_io::container::rpf::{Rpf9BodyKind, Rpf9Role, Rpf9Scan, Rpf9ZlibRun};
use or_io::gta6::headerless::{read_dictionary, Entry};
use or_renderer::game_shaders::mesh::Image;

/// The install's texture archive, whose dictionaries the game holds whatever is drawn.
const ARCHIVE: &str = "textures.rpf";
/// The bytes of a dictionary body kept to read its entries' names: more than any virtual section holds.
const DICTIONARY_HEAD: usize = 1 << 20;
/// The most the body of the dictionary that holds a texture inflates to.
const BODY_LIMIT: u64 = 1 << 30;

/// A texture's slices, the first level of each: one for a plain texture, a volume's depth of them.
pub(crate) type Slices = Arc<Vec<Option<Arc<Image>>>>;

type Slot = Arc<Mutex<Option<Option<Slices>>>>;

static READ: Mutex<BTreeMap<(PathBuf, u32), Slot>> = Mutex::new(BTreeMap::new());

/// The texture whose name hashes to `name` in the archive under `root`: `Err` while a worker reads
/// it, which the first ask starts; `None` where no dictionary of the archive holds it.
pub(crate) fn under(root: &Path, name: u32) -> Result<Option<Slices>, ()> {
    let mut read = READ.lock().unwrap_or_else(|p| p.into_inner());
    let key = (root.to_path_buf(), name);
    if let Some(slot) = read.get(&key) {
        return slot
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or(());
    }
    let slot = Slot::default();
    read.insert(key, Arc::clone(&slot));
    let archive = root.join(ARCHIVE);
    // On the maps' workers: the count of maps in moves once it is read, so the scene binds it.
    crate::render::gta6_maps::decode(move || {
        let found = slices_in(&archive, name);
        if let Some(slices) = &found {
            tracing::info!(
                name = format!("0x{name:08X}"),
                slices = slices.len(),
                "read the texture the game binds by this name from {}",
                archive.display()
            );
        } else {
            tracing::warn!(
                name = format!("0x{name:08X}"),
                "no dictionary of {} holds the texture the game binds by this name, so it reads as noughts",
                archive.display()
            );
        }
        *slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(found.map(Arc::new));
    });
    Err(())
}

/// The texture named `name` for the active GTA VI drawable, from the archive of its install.
pub(crate) fn worn(state: &crate::state::AppState, name: u32) -> Result<Option<Slices>, ()> {
    let doc = state.active_document().ok_or(())?;
    match crate::state::gta6_texture_index::root_of(state, doc) {
        Some(root) => under(&root, name),
        None => Ok(None),
    }
}

/// The slices of the texture named `name` in the first dictionary of `archive` that holds it.
fn slices_in(archive: &Path, name: u32) -> Option<Vec<Option<Arc<Image>>>> {
    let file = File::open(archive).ok()?;
    let len = file.metadata().ok()?.len();
    let mut scan = Rpf9Scan::new(&file, 0..len).ok()?.keeping(DICTIONARY_HEAD);
    let holds = |entries: &[(u32, _)]| entries.iter().any(|(code, _)| *code == name);
    let offset = loop {
        let find = scan.next()?.ok()?;
        let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
            resource: Some(resource),
            signature: Some(signature),
            ..
        }) = find.kind
        else {
            continue;
        };
        if signature.role() != Some(Rpf9Role::Dictionary) {
            continue;
        }
        let (virtual_section, _) = resource.sections(scan.kept(&find));
        if read_dictionary(virtual_section, None).is_ok_and(|d| holds(&d.entries)) {
            break find.offset;
        }
    };
    let mut reader = Rpf9BodyReader::new(File::open(archive).ok()?).ok()?;
    let (find, bytes) = reader.read(offset, BODY_LIMIT).ok()?;
    let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(resource),
        ..
    }) = find.kind
    else {
        return None;
    };
    let (virtual_section, physical) = resource.sections(&bytes);
    let dictionary = read_dictionary(virtual_section, Some(physical)).ok()?;
    let texture = (dictionary.entries.into_iter()).find_map(|(code, entry)| match entry {
        Ok(Entry::Texture(t)) if code == name => t.texture.ok(),
        _ => None,
    })?;
    let first_level = or_renderer::game_shaders::drawable::first_level;
    let slices = (0..texture.faces).map(|face| texture.face(face).and_then(|f| first_level(&f)));
    Some(slices.map(|slice| slice.map(Arc::new)).collect())
}
