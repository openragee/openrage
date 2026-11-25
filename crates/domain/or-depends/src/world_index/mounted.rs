//! A mounted install as a world index reads it: each root archive on its own, stamped by the file
//! on disk that holds it as the mount found that file.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use or_title::vfs::{ArchiveDevice, EntryFacts, FileDevice};
use or_title::MountedGame;

use super::{Archive, Stamp, WorldSource};

/// A mounted install and the folder it was mounted from.
pub struct MountedWorld<'a> {
    game: &'a MountedGame,
    root: PathBuf,
    /// One device a root archive, so a file is read from the archive that lists it and not from
    /// whichever archive answers its path last on a shared device.
    devices: Vec<ArchiveDevice>,
}

impl<'a> MountedWorld<'a> {
    /// `game`, which was mounted from the install at `install_root`.
    pub fn new(game: &'a MountedGame, install_root: &Path) -> Self {
        let devices = (game.roots.iter())
            .map(|root| ArchiveDevice::new(Arc::clone(&root.archive)))
            .collect();
        Self {
            game,
            root: install_root.to_path_buf(),
            devices,
        }
    }

    /// The path of `logical` inside root archive `archive`, which names it under its device.
    fn inside<'p>(&self, archive: usize, logical: &'p str) -> Option<&'p str> {
        let device = &self.game.roots.get(archive)?.device;
        logical.strip_prefix(device.as_str())?.strip_prefix(":/")
    }
}

impl WorldSource for MountedWorld<'_> {
    fn identity(&self) -> String {
        self.root.to_string_lossy().into_owned()
    }

    fn archives(&self) -> Vec<Archive> {
        (self.game.roots.iter())
            .map(|root| Archive {
                device: root.device.clone(),
                container: root.container.clone(),
                // ⚠ The mount's stamp and never the file's as it is now: an archive written since
                // the mount was made would be kept as what the mount read of the one before it.
                // And none where a level of DLC keeps folders out of the walk: the file is the same
                // at every level, so its stamp would keep what one level walked for another.
                stamp: (root.stamp.filter(|_| root.left_out.is_empty())).map(|stamp| Stamp {
                    size: stamp.size,
                    modified: stamp.modified,
                }),
            })
            .collect()
    }

    fn entries(&self, archive: usize, visit: &mut dyn FnMut(&EntryFacts)) {
        if let Some(root) = self.game.roots.get(archive) {
            root.for_each_entry(|facts| visit(facts));
        }
    }

    fn open(&self, archive: usize, logical: &str) -> Option<Vec<u8>> {
        self.devices
            .get(archive)?
            .open(self.inside(archive, logical)?)
    }

    fn open_head(&self, archive: usize, logical: &str, limit: usize) -> Option<Vec<u8>> {
        (self.devices.get(archive)?).open_head(self.inside(archive, logical)?, limit)
    }

    fn patch_of(&self, archive: usize) -> Option<String> {
        self.game.roots.get(archive)?.patch.clone()
    }
}
