//! The skin tones of a GTA VI install, read on a worker apart from the ped index: from the body
//! kept as theirs where the archive is as it was kept, else found by a scan of the archives,
//! smallest first, and kept.

use std::collections::BTreeMap;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use or_io::container::rpf::rpf9_scan::Rpf9BodyReader;
use or_io::container::rpf::{Rpf9BodyKind, Rpf9Content};
use or_io::gta6::hash::name_hash64;
use or_io::gta6::skin_tones::{self, SkinTones};

use crate::state::gta6_texture_index::{put, stat, stated};
use crate::state::rpf9_scan::{Ended, Row as ScanRow, Rpf9ScanDocument};

/// The largest body read.
const BODY_LIMIT: u64 = 64 << 20;
/// An archive past this is not scanned, as the ped index leaves it.
const MOST_ARCHIVE: u64 = 8 << 30;
/// How often the worker looks at the scan of the archive in hand.
const LOOK: Duration = Duration::from_millis(50);
/// The version of what is kept: a change to it takes another.
const VERSION: u64 = 1;

/// Where the skin tones body was found: its archive under the install, the archive's size and
/// modification time then, and the body's offset.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Place {
    archive: PathBuf,
    stat: (u64, u64),
    offset: u64,
}

/// What is kept of `place` for the install at `root`.
fn encode(root: &Path, place: &Place) -> Vec<u8> {
    let mut plain = VERSION.to_le_bytes().to_vec();
    put(&mut plain, root.to_string_lossy().as_bytes());
    put(&mut plain, place.archive.to_string_lossy().as_bytes());
    for word in [place.stat.0, place.stat.1, place.offset] {
        plain.extend(word.to_le_bytes());
    }
    plain
}

/// The place `bytes` keep for the install at `root`; `None` for another version or install, or
/// bytes that do not read.
fn decode(root: &Path, bytes: &[u8]) -> Option<Place> {
    let bytes = &mut bytes.strip_prefix(VERSION.to_le_bytes().as_slice())?;
    let word = |bytes: &mut &[u8]| -> io::Result<u64> {
        let (head, rest) = bytes
            .split_first_chunk::<8>()
            .ok_or(io::ErrorKind::UnexpectedEof)?;
        *bytes = rest;
        Ok(u64::from_le_bytes(*head))
    };
    let mut read = || -> io::Result<Option<Place>> {
        if stated(bytes)? != root.to_string_lossy().as_bytes() {
            return Ok(None);
        }
        let archive = PathBuf::from(&*String::from_utf8_lossy(stated(bytes)?));
        let place = Place {
            archive,
            stat: (word(bytes)?, word(bytes)?),
            offset: word(bytes)?,
        };
        Ok(bytes.is_empty().then_some(place))
    };
    read().ok().flatten()
}

/// The place kept for `root` where its archive's size and modification time are still `now`.
fn fresh(root: &Path, bytes: &[u8], now: impl Fn(&Path) -> Option<(u64, u64)>) -> Option<Place> {
    decode(root, bytes).filter(|place| now(&root.join(&place.archive)) == Some(place.stat))
}

/// The file the place is kept in for `root`, beside the install's kept texture indexes.
fn kept_file(root: &Path) -> Option<PathBuf> {
    let name = name_hash64(root.to_string_lossy().as_bytes());
    let folder = crate::state::prefs::config_root()?.join("gta6_texture_indexes");
    Some(folder.join(format!("{name:016x}.skin_tones")))
}

/// The skin tones of the body at `offset` of `archive`; `None` where it is no such body.
fn read_at(archive: &Path, offset: u64) -> Option<SkinTones> {
    let mut reader = Rpf9BodyReader::new(File::open(archive).ok()?).ok()?;
    let (_, bytes) = reader.read(offset, BODY_LIMIT).ok()?;
    skin_tones::is_skin_tones(&bytes).then_some(())?;
    let root =
        or_io::meta::binary::parse_meta_binary(crate::state::document::meta_schema(), &bytes);
    skin_tones::read_tree(&root.ok()?)
}

/// The metadata bodies the whole scan of the archive at `path` finds; none where it does not
/// read the archive whole.
fn metadata_bodies(path: &Path) -> Vec<u64> {
    let Ok(Some(scan)) = Rpf9ScanDocument::open(path) else {
        return Vec::new();
    };
    loop {
        let found = scan.found();
        match &found.ended {
            Some(Ended::Whole) => {
                return (found.rows.iter())
                    .filter_map(|row| match row {
                        ScanRow::Body(find) => Some(find),
                        ScanRow::Missing { .. } => None,
                    })
                    .filter(|find| find.inflated_len.is_none_or(|len| len <= BODY_LIMIT))
                    .filter(|find| {
                        matches!(find.kind, Rpf9BodyKind::Zlib(run) if run.content == Rpf9Content::Pso)
                    })
                    .map(|find| find.offset)
                    .collect();
            }
            Some(_) => return Vec::new(),
            None => {}
        }
        drop(found);
        std::thread::sleep(LOOK);
    }
}

/// The skin tones of the install at `root` and where they were found, from the first archive
/// that holds them, smallest first.
fn find(root: &Path) -> Option<(Place, SkinTones)> {
    let (archives, _) = crate::state::gta6_install_scan::archives_under(root);
    let mut sized: Vec<(u64, PathBuf)> = (archives.into_iter())
        .filter_map(|path| Some((stat(&path)?.0, path)))
        .filter(|(len, _)| *len <= MOST_ARCHIVE)
        .collect();
    sized.sort();
    for (_, path) in sized {
        for offset in metadata_bodies(&path) {
            if let Some(tones) = read_at(&path, offset) {
                let archive = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
                let place = Place {
                    archive,
                    stat: stat(&path)?,
                    offset,
                };
                return Some((place, tones));
            }
        }
    }
    None
}

/// The skin tones of `root`: from the kept place where it is fresh and its body reads as them,
/// else found and kept.
fn read(root: &Path) -> Option<SkinTones> {
    let file = kept_file(root);
    let kept = (file.as_ref())
        .and_then(|file| std::fs::read(file).ok())
        .and_then(|bytes| fresh(root, &bytes, stat));
    if let Some(place) = kept {
        if let Some(tones) = read_at(&root.join(&place.archive), place.offset) {
            return Some(tones);
        }
    }
    let (place, tones) = find(root)?;
    if let Some(file) = file {
        let kept = (file.parent().map_or(Ok(()), std::fs::create_dir_all))
            .and_then(|()| std::fs::write(&file, encode(root, &place)));
        if let Err(error) = kept {
            tracing::warn!(%error, "where the skin tones were found was not kept");
        }
    }
    Some(tones)
}

type Slot = Arc<Mutex<Option<Option<Arc<SkinTones>>>>>;

static READ: Mutex<BTreeMap<PathBuf, Slot>> = Mutex::new(BTreeMap::new());

/// The skin tones of the install at `root`: `Err` while a worker reads them, which the first ask
/// starts; `None` where no archive holds them.
fn under(root: &Path) -> Result<Option<Arc<SkinTones>>, ()> {
    let mut read_ = READ.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(slot) = read_.get(root) {
        return slot
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or(());
    }
    let slot = Slot::default();
    read_.insert(root.to_path_buf(), Arc::clone(&slot));
    let root = root.to_path_buf();
    // On the maps' workers: the count of maps in moves once they are read, so the scene binds them.
    crate::render::gta6_maps::decode(move || {
        let started = std::time::Instant::now();
        let tones = read(&root);
        tracing::info!(
            presets = tones.as_ref().map_or(0, |t| t.presets.len()),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "read the skin tones of {}",
            root.display()
        );
        *slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(tones.map(Arc::new));
    });
    Err(())
}

/// The skin tones of the active document's install; `Err` while a worker reads them.
pub(crate) fn worn(state: &crate::state::AppState) -> Result<Option<Arc<SkinTones>>, ()> {
    let Some(doc) = state.active_document() else {
        return Ok(None);
    };
    match crate::state::gta6_texture_index::root_of(state, doc) {
        Some(root) => under(&root),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kept_place_is_used_only_while_its_archive_is_as_it_was_kept() {
        let root = Path::new("K:/made/up/install");
        let place = Place {
            archive: PathBuf::from("data.rpf"),
            stat: (4096, 77),
            offset: 0x2000,
        };
        let bytes = encode(root, &place);
        assert_eq!(decode(root, &bytes), Some(place.clone()));
        let as_kept = |_: &Path| Some((4096, 77));
        assert_eq!(fresh(root, &bytes, as_kept), Some(place));
        // A size or a time that moved, or an archive gone: the kept place is refused.
        assert_eq!(fresh(root, &bytes, |_: &Path| Some((4096, 78))), None);
        assert_eq!(fresh(root, &bytes, |_: &Path| Some((4097, 77))), None);
        assert_eq!(fresh(root, &bytes, |_: &Path| None), None);
        // Another install's, another version's, or bytes cut short.
        assert_eq!(decode(Path::new("K:/other"), &bytes), None);
        let mut other_version = bytes.clone();
        other_version[0] ^= 1;
        assert_eq!(decode(root, &other_version), None);
        assert_eq!(decode(root, &bytes[..bytes.len() - 1]), None);
    }
}
