//! The textures of an install's texture archive that the game binds by name whatever is drawn:
//! each found by a scan of the archive and read once a session on a worker, slice by slice.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use or_io::codec::texture::DxgiFormat;
use or_io::container::rpf::rpf9_scan::Rpf9BodyReader;
use or_io::container::rpf::{Rpf9BodyKind, Rpf9Role, Rpf9Scan, Rpf9ZlibRun};
use or_io::gta6::headerless::{read_dictionary, Entry};
use or_io::gta6::ttpf::Texture;
use or_renderer::game_shaders::meshlets::Volume;

/// The install's texture archive, whose dictionaries the game holds whatever is drawn.
const ARCHIVE: &str = "textures.rpf";
/// The bytes of a dictionary body kept to read its entries' names: more than any virtual section holds.
const DICTIONARY_HEAD: usize = 1 << 20;
/// The most the body of the dictionary that holds a texture inflates to.
const BODY_LIMIT: u64 = 1 << 30;

/// A texture read: the first level of each of its slices, as a volume.
pub(crate) type Slices = Arc<Volume>;

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
        let found = volume_in(&archive, name);
        if let Some(volume) = &found {
            tracing::info!(
                name = format!("0x{name:08X}"),
                slices = volume.depth,
                half = volume.half,
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

/// The texture named `name` in the first dictionary of `archive` that holds it, as a volume.
fn volume_in(archive: &Path, name: u32) -> Option<Volume> {
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
    volume_of(&texture)
}

/// The first level of each slice of `texture` as one volume: the half floats as they are stored,
/// else each slice decoded to bytes; `None` where a slice will not read.
fn volume_of(texture: &Texture) -> Option<Volume> {
    let (width, height, depth) = (texture.width, texture.height, texture.faces);
    let half = texture.format.dxgi() == DxgiFormat::R16G16B16A16Float;
    let mut texels = Vec::new();
    for slice in 0..depth {
        let slice = texture.face(slice)?;
        match half {
            true => texels.extend_from_slice(slice.pixels.get(..(width * height * 8) as usize)?),
            false => {
                let image = or_renderer::game_shaders::drawable::first_level(&slice)?;
                texels.extend_from_slice(&image.rgba);
            }
        }
    }
    Some(Volume {
        width,
        height,
        depth,
        half,
        texels,
    })
}

#[cfg(test)]
mod tests {
    use or_io::gta6::ttpf::Format;

    use super::*;

    #[test]
    fn a_volume_of_half_floats_keeps_them_as_stored_slice_after_slice() {
        // Two slices of 2 by 1 texels, eight bytes a texel, each byte its own place.
        let texture = Texture {
            width: 2,
            height: 1,
            format: Format::of_dxgi(10).expect("16-bit float RGBA"),
            levels: 1,
            faces: 2,
            pixels: (0u8..32).collect(),
        };
        let volume = volume_of(&texture).expect("it reads");
        assert_eq!((volume.width, volume.height, volume.depth), (2, 1, 2));
        assert!(volume.half);
        assert_eq!(volume.texels, (0u8..32).collect::<Vec<_>>());
        // A slice short of its first level does not read.
        let short = Texture {
            pixels: vec![0; 24],
            ..texture
        };
        assert_eq!(volume_of(&short), None);
    }
}
