//! The mount of an install for the index: its registration documents, read by the archives'
//! tables where a key snapshot deciphers them and by a scan where none does, which archives hold
//! the world, and which packs register each map.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use or_io::container::rpf::GtaViKeys;
use or_title::dlc::RegisteredFile;
use or_title::gta6_mount::table::Table;
use or_title::gta6_mount::{self, HeldBack, Listed, MapFact, Mode, Registry};

/// An archive larger than this is not scanned for registration documents.
const SMALL: u64 = 1 << 30;
/// The fewest archives a device's folder must be named for to be scanned as the level's.
const NAMED: usize = 16;

/// What the registration documents of an install came to.
pub(super) struct Mount {
    pub(super) registry: Registry,
    /// The table of each archive that deciphered, with the archive's place among those under the root.
    pub(super) tables: Vec<(usize, Table)>,
}

/// The key snapshot of this run, where one is found under the configuration folder `config` and reads.
pub fn keys(config: Option<&Path>) -> Option<GtaViKeys> {
    GtaViKeys::for_install(config)
        .inspect_err(|error| tracing::warn!(%error, "the GTA VI key snapshot is not read"))
        .ok()
        .flatten()
}

/// Read the registration documents of the archives `every`: through an archive's table where `keys` deciphers it, else by a scan of an archive that is small; `None` once `stopped` says so.
pub(super) fn read(
    every: &[PathBuf],
    keys: Option<&GtaViKeys>,
    stopped: &dyn Fn() -> bool,
) -> Option<Mount> {
    let mut tables = Vec::new();
    let mut documents = Vec::new();
    for (at, path) in every.iter().enumerate() {
        if stopped() {
            return None;
        }
        let table = keys.and_then(|keys| {
            Table::open(path, keys)
                .inspect_err(
                    |error| tracing::info!(path = %path.display(), %error, "no table read"),
                )
                .ok()
        });
        let found = match &table {
            Some(table) => gta6_mount::read_documents_by_table(path, table, stopped),
            None if std::fs::metadata(path).is_ok_and(|m| m.len() <= SMALL) => {
                gta6_mount::read_documents(path, stopped)
            }
            None => Ok(Vec::new()),
        };
        match found {
            Ok(found) if !found.is_empty() => documents.push((
                at,
                found.into_iter().map(|f| f.document).collect::<Vec<_>>(),
            )),
            Ok(_) => {}
            Err(error) => tracing::info!(path = %path.display(), %error, "no document read"),
        }
        tables.extend(table.map(|table| (at, table)));
    }
    if stopped() {
        return None;
    }
    let table_of = |at: usize| {
        tables
            .iter()
            .find(|(t, _)| *t == at)
            .map(|(_, table)| table)
    };
    let registry = Registry::listed(
        (documents.into_iter()).map(|(at, documents)| (every[at].clone(), documents, table_of(at))),
    );
    Some(Mount { registry, tables })
}

impl Mount {
    /// The files the sets of the level register, of the level pack alone.
    pub(super) fn level_files(&self) -> Vec<&RegisteredFile> {
        (self.registry.level_pack().iter())
            .flat_map(|pack| pack.sets.iter().filter(|set| set.level))
            .flat_map(|set| set.files.iter())
            .collect()
    }

    /// The tables as the binding takes them, each archive by its place among `archives`.
    pub(super) fn listed<'a>(
        &'a self,
        every: &'a [PathBuf],
        archives: &[PathBuf],
    ) -> Vec<Listed<'a>> {
        (self.tables.iter())
            .map(|(at, table)| Listed {
                // An archive the index is not built from holds no map of it.
                archive: (archives.iter().position(|p| *p == every[*at]))
                    .map_or(u32::MAX, |i| i as u32),
                path: &every[*at],
                table,
            })
            .collect()
    }

    /// The archives of `every` the index is built from: with tables, those that list a registered
    /// metadata archive; with none, the archive of a folder the level names many archives under;
    /// and every archive where neither names one.
    pub(super) fn archives(&self, root: &Path, every: &[PathBuf]) -> Vec<PathBuf> {
        let listed = self.listed(every, every);
        let mut archives: Vec<PathBuf> = gta6_mount::world_archives(&self.registry.packs, &listed)
            .into_iter()
            .filter_map(|at| every.get(at as usize).cloned())
            .collect();
        archives.sort();
        if archives.is_empty() {
            let folders = level_folders(&self.level_files());
            let named = |path: &Path| {
                let stem = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_ascii_lowercase());
                path.parent() == Some(root)
                    && stem.is_some_and(|stem| folders.get(&stem).is_some_and(|n| *n >= NAMED))
            };
            archives = every.iter().filter(|p| named(p)).cloned().collect();
        }
        match archives.is_empty() {
            true => every.to_vec(),
            false => archives,
        }
    }

    /// Which packs register each of `maps`, which lie in the archives `archives` of `every`.
    pub(super) fn bind(&self, every: &[PathBuf], archives: &[PathBuf], maps: &[MapFact]) -> Bound {
        let by_blocks = gta6_mount::bind(&self.level_files(), maps);
        let base = Mode::Base.bit();
        let scanned: Vec<u8> = (by_blocks.held.iter())
            .map(|held| match held {
                Some(HeldBack::NotRegistered) => 0,
                _ => base,
            })
            .collect();
        let by_table =
            gta6_mount::bind_by_table(&self.registry.packs, &self.listed(every, archives), maps);
        // Tables that bind no archive, as where the level's own does not decipher, leave the block
        // maps to bind it whole.
        if self.tables.is_empty() || by_table.bound_archives == 0 {
            return Bound {
                modes: scanned,
                named_archives: by_blocks.report.named_archives,
                bound_archives: by_blocks.report.bound_archives,
                ..Bound::default()
            };
        }
        // The block maps bind the level's archive whole: where the two disagree on a map of the
        // base game, the table stands and the count says so.
        let disagree = (by_table.modes.iter().zip(&scanned))
            .filter(|(table, scan)| (**table & base) != **scan)
            .count();
        if disagree > 0 {
            tracing::info!(
                disagree,
                "maps the tables and the block maps bind differently"
            );
        }
        Bound {
            modes: by_table.modes,
            named_archives: by_table.named_archives,
            bound_archives: by_table.bound_archives,
            by_table: true,
            disagree,
            permanent: by_table.permanent.into_iter().collect(),
            permanent_named: by_table.permanent_named,
        }
    }
}

/// Which packs register each map, and what the binding counted.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Bound {
    /// For each map: the bits of the modes whose packs register it.
    pub(super) modes: Vec<u8>,
    pub(super) named_archives: usize,
    pub(super) bound_archives: usize,
    /// Whether the tables bound the maps, and how many maps the block maps bind otherwise.
    pub(super) by_table: bool,
    pub(super) disagree: usize,
    /// The nested archive of each permanent map-types file, by its archive and offset.
    pub(super) permanent: HashSet<(u32, u64)>,
    pub(super) permanent_named: usize,
}

/// The first folder each registered archive of the level is named under on the platform device,
/// lower-cased, with how many are.
pub(super) fn level_folders(files: &[&RegisteredFile]) -> HashMap<String, usize> {
    let mut folders = HashMap::new();
    for file in files {
        let name = file.filename.to_ascii_lowercase();
        let Some(rest) = name.strip_prefix("platform:/") else {
            continue;
        };
        if let (Some((first, _)), true) = (rest.split_once('/'), name.ends_with(".rpf")) {
            *folders.entry(first.to_string()).or_default() += 1;
        }
    }
    folders
}

/// Every archive under `root`, through the one lister, the folder's own first; a walk cut short by a limit is logged.
pub(super) fn archives_under(root: &Path) -> Vec<PathBuf> {
    let listed = or_title::install::gta6_files::list(root);
    if listed.truncated {
        tracing::warn!(root = %root.display(), "the walk stopped at a limit: archives past it are not indexed");
    }
    listed.archive_paths()
}

#[cfg(test)]
#[path = "index_mount_tests.rs"]
pub(super) mod tests;
