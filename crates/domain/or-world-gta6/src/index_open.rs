//! The index of an install as a caller opens it: from the kept file every title shares where its
//! inputs stand, read again where they do not, and what the kept file of an earlier build left
//! in the configuration folder put away first.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use or_io::container::rpf::GtaViKeys;
use or_io::gta6::hash::name_hash64;
use or_job::{Cancel, Progress};
use or_world::contract::{GameId, WorldError, WorldIndexer, WorldSpec};
use or_world::kept::{self, Report, Section};

use super::install::{Disk, Install, Linked, OnDisk, FRESH};
use super::WorldIndex;

/// What names the title in a kept file's head and in the kept file's name.
pub const TITLE: &str = "gta6";
/// The game, by the id its profile registers under.
pub const GAME: GameId = GameId("gta-vi");
/// The folder of the configuration folder that held the index before the kept file every title shares.
const OLD_FOLDER: &str = "gta6_world";

fn world(root: &Path) -> WorldSpec {
    WorldSpec::of(GAME, root)
}

/// Where the index of the install at `root` is kept under the configuration folder `config`.
pub fn kept_at(config: &Path, root: &Path) -> PathBuf {
    kept::file_for(config, TITLE, &world(root))
}

/// Where what a fresh game is taken to have is written under the configuration folder `config`: beside the kept index of every GTA VI install.
pub fn fresh_at(config: &Path) -> PathBuf {
    config.join("worlds").join(TITLE).join(FRESH)
}

/// Where the index of the install at `root` was kept before.
fn old_kept_at(config: &Path, root: &Path) -> PathBuf {
    let name = name_hash64(root.to_string_lossy().as_bytes());
    (config.join(OLD_FOLDER)).join(format!("{name:016x}.index"))
}

/// Delete the index kept of `root` before, and move the file of a fresh game from beside it: where that file is read from at this open.
// ⚠ The folder is left: the Map screen keeps its own files in it.
fn settle_old(config: &Path, root: &Path) -> PathBuf {
    let old = old_kept_at(config, root);
    match std::fs::remove_file(&old) {
        Ok(()) => {
            tracing::info!(file = %old.display(), "the index kept before was deleted: it is built again")
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            tracing::warn!(file = %old.display(), %error, "the index kept before was not deleted")
        }
    }
    let (was, now) = (old.with_file_name(FRESH), fresh_at(config));
    if !was.is_file() {
        return now;
    }
    if now.is_file() {
        tracing::warn!(left = %was.display(), read = %now.display(), "two files state what a fresh game is taken to have");
        return now;
    }
    let moved = (now.parent().map_or(Ok(()), std::fs::create_dir_all))
        .and_then(|()| std::fs::rename(&was, &now))
        // Another volume takes no rename: the file is copied, and then deleted.
        .or_else(|_| std::fs::copy(&was, &now).and_then(|_| std::fs::remove_file(&was)));
    match moved {
        Ok(()) => now,
        Err(error) => {
            tracing::warn!(from = %was.display(), to = %now.display(), %error, "what a fresh game is taken to have was not moved, and is read where it lies");
            was
        }
    }
}

/// The index of an install, and what its kept file served of it.
pub struct Opened {
    pub index: Arc<WorldIndex>,
    /// `None` where there is no configuration folder to keep an index in.
    pub kept: Option<Report>,
}

/// The index of the install at `root`: from the file kept under `config` where its inputs stand, with only the archives that changed read again, or with every archive read where `afresh`; `None` once `cancel` is set.
pub fn open(
    root: &Path,
    config: Option<&Path>,
    keys: Option<&GtaViKeys>,
    afresh: bool,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<Option<Opened>, String> {
    open_through(&OnDisk, root, config, keys, afresh, (cancel, progress))
}

pub(super) fn open_through(
    disk: &dyn Disk,
    root: &Path,
    config: Option<&Path>,
    keys: Option<&GtaViKeys>,
    afresh: bool,
    job: (&Cancel, &Progress),
) -> Result<Option<Opened>, String> {
    let Some(config) = config else {
        let built = build_through(disk, root, None, keys, job)?;
        return Ok(built.map(|index| Opened {
            index: Arc::new(index),
            kept: None,
        }));
    };
    let file = kept_at(config, root);
    let fresh = settle_old(config, root);
    let install = Install::new(root, Some(&fresh), keys, job, disk);
    let mut afresh = afresh;
    let (index, report) = loop {
        if afresh {
            match std::fs::remove_file(&file) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => {
                    tracing::warn!(file = %file.display(), %error, "the kept index was not deleted")
                }
                _ => {}
            }
        }
        match kept::refresh_with(TITLE, &world(root), install.inputs(), &install, &file) {
            Ok(read) => break read,
            Err(WorldError::Cancelled) => return Ok(None),
            // ⚠ A kept file can read whole and hold sections that do not link, which no later
            // open would mend: it is deleted and every archive read, once.
            Err(error) if !afresh && file.is_file() => {
                tracing::warn!(file = %file.display(), %error, "the kept index does not link and is built again");
                afresh = true;
            }
            Err(error) => return Err(error.to_string()),
        }
    };
    if let Some(error) = &report.not_kept {
        tracing::warn!(%error, "the world index was not kept");
    }
    let linked = (index.title::<Linked>()).ok_or("the link of a GTA VI index made no tables")?;
    let index = Arc::clone(&linked.world);
    // ⚠ A folder with no game links an index of nothing, which is no world: it is not kept.
    if index.report.maps == 0 {
        let _ = std::fs::remove_file(&file);
    }
    Ok(Some(Opened {
        index,
        kept: Some(report),
    }))
}

/// The index of the install at `root` read from its archives alone, with what the file `fresh` takes of a fresh game where there is one, and nothing kept; `None` once `cancel` is set.
pub fn build(
    root: &Path,
    fresh: Option<&Path>,
    snapshot: Option<&GtaViKeys>,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<Option<WorldIndex>, String> {
    build_through(&OnDisk, root, fresh, snapshot, (cancel, progress))
}

fn build_through(
    disk: &dyn Disk,
    root: &Path,
    fresh: Option<&Path>,
    keys: Option<&GtaViKeys>,
    job: (&Cancel, &Progress),
) -> Result<Option<WorldIndex>, String> {
    let install = Install::new(root, fresh, keys, job, disk);
    let built = || -> Result<WorldIndex, WorldError> {
        let mut archives = Vec::new();
        for archive in 0..install.inputs().archives.len() {
            let mut section = Section::new();
            install.rows(archive, &mut section)?;
            archives.push(section);
        }
        let mut whole = Section::new();
        install.whole(&archives, &mut whole)?;
        Ok(install.linked(&archives, &whole)?.0)
    };
    match built() {
        Ok(index) => Ok(Some(index)),
        Err(WorldError::Cancelled) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
#[path = "index_open_tests.rs"]
mod tests;
