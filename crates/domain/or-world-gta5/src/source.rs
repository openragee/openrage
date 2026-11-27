//! A mounted install as the world index reads it, owned by the opened world: each root archive on
//! its own, stamped by the file on disk that holds it as the mount found that file.

use std::path::PathBuf;
use std::sync::Arc;

use or_depends::world_index::{Archive, Stamp};
use or_depends::WorldSource;
use or_title::vfs::{ArchiveDevice, EntryFacts, FileDevice};
use or_title::MountedGame;

/// A mount and the folder it was made from.
pub(crate) struct Source {
    pub game: MountedGame,
    pub root: PathBuf,
    /// One device a root archive, so a file is read from the archive that lists it.
    devices: Vec<ArchiveDevice>,
}

impl Source {
    pub fn new(game: MountedGame, root: PathBuf) -> Self {
        let devices = (game.roots.iter())
            .map(|root| ArchiveDevice::new(Arc::clone(&root.archive)))
            .collect();
        Self {
            game,
            root,
            devices,
        }
    }

    /// The path of `logical` inside root archive `archive`, which names it under its device.
    fn inside<'p>(&self, archive: usize, logical: &'p str) -> Option<&'p str> {
        let device = &self.game.roots.get(archive)?.device;
        logical.strip_prefix(device.as_str())?.strip_prefix(":/")
    }
}

impl WorldSource for Source {
    fn identity(&self) -> String {
        self.root.to_string_lossy().into_owned()
    }

    fn archives(&self) -> Vec<Archive> {
        (self.game.roots.iter())
            .map(|root| Archive {
                device: root.device.clone(),
                container: root.container.clone(),
                // ⚠ The mount's stamp, where a level leaves folders of the archive out too: a
                // world at a level has a kept file of its own, so what one level walked is never
                // kept for another.
                stamp: root.stamp.map(|stamp| Stamp {
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

/// One root archive of a source as a mount of its own, which is how its rows are read apart from the others'.
pub(crate) struct One<'a> {
    pub source: &'a Source,
    pub archive: usize,
}

impl WorldSource for One<'_> {
    fn identity(&self) -> String {
        self.source.identity()
    }

    fn archives(&self) -> Vec<Archive> {
        (self.source.archives().into_iter())
            .skip(self.archive)
            .take(1)
            .collect()
    }

    fn entries(&self, archive: usize, visit: &mut dyn FnMut(&EntryFacts)) {
        if archive == 0 {
            self.source.entries(self.archive, visit);
        }
    }

    fn open(&self, archive: usize, logical: &str) -> Option<Vec<u8>> {
        (archive == 0).then(|| self.source.open(self.archive, logical))?
    }

    fn open_head(&self, archive: usize, logical: &str, limit: usize) -> Option<Vec<u8>> {
        (archive == 0).then(|| self.source.open_head(self.archive, logical, limit))?
    }
}
