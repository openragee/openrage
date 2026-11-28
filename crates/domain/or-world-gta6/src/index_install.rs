//! A GTA VI install as the kept file every title shares reads it: every archive under the root is
//! an input, a world archive's rows lie in its own section, and what the mount, the world states
//! and the far stand-ins make across archives lies in the one derived section.
//!
//! ⚠ Which archives hold the world is the mount's word, read from the registration documents of
//! every archive. Nothing of the mount is read until rows or the derived section are asked for:
//! an open the kept file serves opens no archive.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use or_io::container::rpf::GtaViKeys;
use or_io::gta6::map::stream::grass::slod::{self, SlodClaim};
use or_io::gta6::world_state::Place;
use or_job::{Cancel, Progress};
use or_title::gta6_mount::MapFact;
use or_world::contract::{WorldError, WorldIndexer};
use or_world::kept::{Input, Inputs, Section, Stamp};
use or_world::model::{Gate, Index, MapId, PlacementRow, SwitchId};

use super::mount::{self, Mount};
use super::rows::{pod, put_pod, Rows};
use super::scan::{self, Shard};
use super::states::{self, Placed};
use super::{counted, dictionaries, place, BodyRow, GroupRow, PlaceRow, Report, WorldIndex};

/// The version of what a section holds and of what a scan reads of an archive: a change to either takes another, and every kept index is then built again.
pub const RULES: u32 = 1;

/// What the other input is named: the file of what a fresh game is taken to have.
pub(super) const FRESH: &str = "fresh_game.json";

/// What reads the archives of an install: the disk's own readers, or a test's.
pub(super) trait Disk: Sync {
    /// The registration documents of `every`; `None` once `stopped` says so.
    fn mount(
        &self,
        every: &[PathBuf],
        keys: Option<&GtaViKeys>,
        stopped: &dyn Fn() -> bool,
    ) -> Option<Mount>;
    /// Everything the archive at `path` gives the index; `None` once `cancel` is set.
    fn scan(
        &self,
        path: &Path,
        cancel: &Cancel,
        reached: &(dyn Fn(u64) + Sync),
    ) -> io::Result<Option<Shard>>;
    /// The world-state selectors and places of the game-shape archives, and whether each of them opened; `None` once `cancel` is set.
    fn selectors(
        &self,
        mounted: &Mount,
        every: &[PathBuf],
        cancel: &Cancel,
    ) -> Option<(Vec<Placed>, Vec<Place>, bool)>;
}

/// The archives as they lie on disk.
pub(super) struct OnDisk;

impl Disk for OnDisk {
    fn mount(
        &self,
        every: &[PathBuf],
        keys: Option<&GtaViKeys>,
        stopped: &dyn Fn() -> bool,
    ) -> Option<Mount> {
        mount::read(every, keys, stopped)
    }

    fn scan(
        &self,
        path: &Path,
        cancel: &Cancel,
        reached: &(dyn Fn(u64) + Sync),
    ) -> io::Result<Option<Shard>> {
        scan::scan_archive(path, cancel, reached)
    }

    fn selectors(
        &self,
        mounted: &Mount,
        every: &[PathBuf],
        cancel: &Cancel,
    ) -> Option<(Vec<Placed>, Vec<Place>, bool)> {
        states::selectors(mounted, every, cancel)
    }
}

/// The mount of an install, with the archives it names the world's.
struct Mounted {
    mount: Mount,
    /// The archives the index is built from, each by its place among the inputs.
    world: Vec<usize>,
    /// The same, by path.
    paths: Vec<PathBuf>,
    /// The bytes of them, which the progress of a build is counted against.
    total: u64,
}

/// A GTA VI install whose index is read: the archives under its root, and what reads them.
pub(super) struct Install<'a> {
    pub(super) root: &'a Path,
    /// Every archive under the root, the inputs in their order.
    every: Vec<PathBuf>,
    /// The file of what a fresh game is taken to have.
    pub(super) fresh: Option<&'a Path>,
    pub(super) keys: Option<&'a GtaViKeys>,
    pub(super) cancel: &'a Cancel,
    pub(super) progress: &'a Progress,
    disk: &'a dyn Disk,
    /// `None` inside for a mount that was cancelled.
    mounted: OnceLock<Option<Mounted>>,
    done: AtomicU64,
}

fn unread(why: &str) -> WorldError {
    WorldError::Unread {
        what: "the kept index of a GTA VI install".into(),
        why: why.into(),
    }
}

/// What is derived across the archives' rows, as the derived section holds it.
#[derive(Debug, Default, PartialEq)]
struct Whole {
    /// The archives the index is built from, each by its place among the inputs.
    world: Vec<u32>,
    /// The rows of each world archive whose own section holds none, by the same place.
    late: Vec<(u32, Rows)>,
    /// For each map, in the link's order: the bits of the modes whose packs register it.
    modes: Vec<u8>,
    slod: Vec<u64>,
    switches: Vec<GroupRow>,
    places: Vec<PlaceRow>,
    outlines: Vec<[f32; 2]>,
    dictionaries: Vec<BodyRow>,
    report: Report,
}

impl Whole {
    fn put(&self, out: &mut Section) {
        put_pod(out, &self.world);
        out.put_u64(self.late.len() as u64);
        for (archive, rows) in &self.late {
            out.put_u32(*archive);
            rows.put_body(out);
        }
        out.put_bytes(&self.modes);
        put_pod(out, &self.slod);
        put_pod(out, &self.switches);
        put_pod(out, &self.places);
        put_pod(out, &self.outlines);
        put_pod(out, &self.dictionaries);
        put_pod(out, std::slice::from_ref(&self.report));
    }

    fn read(section: &Section) -> Result<Self, WorldError> {
        let mut from = section.reader();
        let world = pod(&mut from)?;
        let mut late = Vec::new();
        for _ in 0..from.u64()? {
            late.push((from.u32()?, Rows::read_body(&mut from)?));
        }
        let whole = Self {
            world,
            late,
            modes: from.bytes()?.to_vec(),
            slod: pod(&mut from)?,
            switches: pod(&mut from)?,
            places: pod(&mut from)?,
            outlines: pod(&mut from)?,
            dictionaries: pod(&mut from)?,
            report: match pod::<Report>(&mut from)?[..] {
                [report] => report,
                _ => return Err(unread("the derived section states no one report")),
            },
        };
        from.end()?;
        Ok(whole)
    }
}

/// The rows of the world's archives laid end to end, as a build of the index gathers them.
#[derive(Default)]
struct Gathered {
    /// The index with the tables an archive's rows fill, each map in its archive and nothing yet placed.
    index: WorldIndex,
    facts: Vec<MapFact>,
    /// Where each map-types body lies: its archive and the nested archive that holds it.
    types_at: HashSet<(u32, u64)>,
    keys: HashMap<u32, Vec<BodyRow>>,
    unread: u64,
    claims: Vec<SlodClaim>,
    words: Vec<(u32, u64)>,
}

impl Gathered {
    /// Take the rows of the world archive at `archive` among the index's.
    // ⚠ The order rows are taken in is the order the later of two bodies that state one
    // archetype is told by: archive by archive, each in the order of its scan.
    fn take(&mut self, archive: u32, rows: Rows) {
        let index = &mut self.index;
        let body = |(word, offset)| BodyRow {
            word,
            offset,
            archive,
            pad: 0,
        };
        (self.types_at).extend(rows.types.into_iter().map(|container| (archive, container)));
        for map in rows.maps {
            let mut row = map.row;
            row.archive = archive;
            row.parents_from = index.parents.len() as u32;
            row.parents_len = map.parents.len() as u32;
            index.parents.extend(map.parents);
            self.facts.push(MapFact {
                archive,
                container: map.container,
                name: row.name,
                block: row.block,
                flags: row.flags,
                holds: map.holds,
            });
            index.maps.push(row);
        }
        index.archetypes.extend(rows.archetypes);
        index.lights.extend(rows.lights);
        self.words.extend(rows.words);
        index.models.extend(rows.models.into_iter().map(body));
        index.interiors.extend(rows.interiors.into_iter().map(body));
        for (offset, codes) in rows.keys {
            for code in codes {
                (self.keys.entry(code).or_default()).push(body((0, offset)));
            }
        }
        self.unread += rows.unread;
        self.claims.extend(rows.slod);
    }
}

/// The rows of each archive of `world`, in its order: those of its own section, else those `late` holds for it.
fn gathered(
    world: &[u32],
    archives: &[Section],
    late: Vec<(u32, Rows)>,
) -> Result<Gathered, WorldError> {
    let mut late: HashMap<u32, Rows> = late.into_iter().collect();
    let mut all = Gathered::default();
    for (archive, at) in (0u32..).zip(world) {
        let section = (archives.get(*at as usize))
            .ok_or_else(|| unread("the derived section names an archive past the last"))?;
        let rows = match Rows::read(section)? {
            Some(rows) => rows,
            None => (late.remove(at))
                .ok_or_else(|| unread("the rows of an archive of the world were never read"))?,
        };
        all.take(archive, rows);
    }
    Ok(all)
}

/// What the link of GTA VI made beside the rows: the index its readers stream from.
pub struct Linked {
    pub world: Arc<WorldIndex>,
    /// The place among the index's maps of each row of the linked index.
    places: Vec<u32>,
}

impl Linked {
    /// Where the map of the row `id` lies among the maps of [`Self::world`].
    pub fn place(&self, id: MapId) -> Option<usize> {
        self.places.get(id.0 as usize).map(|at| *at as usize)
    }
}

/// The rows of the linked index over `world`, and each one's place among the maps.
// ⚠ A row's modes are not in its gate: a map of a mode that is off is left out by whoever
// selects, as `Enabled::allows` does. Of two maps of one name the index answers for the last,
// and that one alone is a row.
fn rows_of(world: &WorldIndex, inputs: &[u32]) -> (Vec<or_world::model::MapRow>, Vec<u32>) {
    let places: Vec<u32> = (0u32..)
        .zip(&world.maps)
        .filter(|(at, map)| world.map_named(map.name) == Some(*at as usize))
        .map(|(at, _)| at)
        .collect();
    let id_of: HashMap<u32, MapId> = (0u32..)
        .zip(&places)
        .map(|(id, at)| (world.maps[*at as usize].name, MapId(id)))
        .collect();
    let rows = (places.iter())
        .map(|at| {
            let map = &world.maps[*at as usize];
            let mut parents: Vec<MapId> = (world.parents_of(map).iter())
                .filter(|name| **name != map.name)
                .filter_map(|name| id_of.get(name).copied())
                .collect();
            parents.sort_unstable();
            parents.dedup();
            or_world::model::MapRow {
                name: u64::from(map.name),
                label: format!("{:#010x}", map.name),
                boxes: Some(map.boxes()),
                parents,
                gate: match map.held {
                    0 => Gate::Always,
                    2 => Gate::Held {
                        switch: Some(SwitchId(map.group)),
                    },
                    _ => Gate::Held { switch: None },
                },
                archive: (inputs.get(map.archive as usize)).map_or(u32::MAX, |at| *at),
            }
        })
        .collect();
    (rows, places)
}

impl<'a> Install<'a> {
    /// The install at `root`, read through `disk`, with what the file `fresh` takes of a fresh game where it names one.
    pub(super) fn new(
        root: &'a Path,
        fresh: Option<&'a Path>,
        keys: Option<&'a GtaViKeys>,
        (cancel, progress): (&'a Cancel, &'a Progress),
        disk: &'a dyn Disk,
    ) -> Self {
        Self {
            root,
            every: mount::archives_under(root),
            fresh,
            keys,
            cancel,
            progress,
            disk,
            mounted: OnceLock::new(),
            done: AtomicU64::new(0),
        }
    }

    /// Every archive under the root with its size and time, and the file of a fresh game where there is one: no archive is opened to say so.
    pub(super) fn inputs(&self) -> Inputs {
        let name = |path: &Path| {
            let under = path.strip_prefix(self.root).unwrap_or(path);
            under.to_string_lossy().replace('\\', "/")
        };
        let archives = (self.every.iter()).map(|path| Input {
            name: name(path),
            stamp: Stamp::of(path),
        });
        // ⚠ A file that is not there is no input: one listed with no stamp would derive the
        // section across archives again at every open.
        let fresh = self.fresh.and_then(Stamp::of).map(|stamp| Input {
            name: FRESH.into(),
            stamp: Some(stamp),
        });
        Inputs {
            archives: archives.collect(),
            others: fresh.into_iter().collect(),
        }
    }

    /// The mount, read at the first call.
    fn mounted(&self) -> Result<&Mounted, WorldError> {
        let read = || {
            let stopped = || self.cancel.is_set();
            let mount = self.disk.mount(&self.every, self.keys, &stopped)?;
            let paths = mount.archives(self.root, &self.every);
            let world = (paths.iter())
                .filter_map(|path| self.every.iter().position(|every| every == path))
                .collect();
            let total = (paths.iter())
                .filter_map(|path| std::fs::metadata(path).ok())
                .map(|meta| meta.len())
                .sum();
            Some(Mounted {
                mount,
                world,
                paths,
                total,
            })
        };
        (self.mounted.get_or_init(read).as_ref()).ok_or(WorldError::Cancelled)
    }

    /// The rows of the input archive `archive`; `None` where it did not read.
    fn scanned(&self, archive: usize) -> Result<Option<Rows>, WorldError> {
        let total = self.mounted()?.total;
        let path = (self.every.get(archive)).ok_or(WorldError::Lacks("such archive"))?;
        let reached = |step: u64| {
            let now = self.done.fetch_add(step, Ordering::Relaxed) + step;
            self.progress.reached(now.min(total), total);
        };
        match self.disk.scan(path, self.cancel, &reached) {
            Ok(Some(shard)) => Ok(Some(Rows::of(shard))),
            Ok(None) => Err(WorldError::Cancelled),
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "an archive was not indexed");
                Ok(None)
            }
        }
    }

    /// The index the sections link to, and the rows of the linked index with their places.
    pub(super) fn linked(
        &self,
        archives: &[Section],
        whole: &Section,
    ) -> Result<(WorldIndex, Vec<or_world::model::MapRow>, Vec<u32>), WorldError> {
        let whole = Whole::read(whole)?;
        let Gathered {
            mut index, facts, ..
        } = gathered(&whole.world, archives, whole.late)?;
        for at in &whole.world {
            let path = (self.every.get(*at as usize))
                .ok_or_else(|| unread("the derived section names an archive past the last"))?;
            index.archives.push(path.clone());
        }
        if whole.modes.len() != index.maps.len() {
            return Err(unread(
                "the derived section counts other maps than the rows",
            ));
        }
        place(&mut index.maps, &facts, &whole.modes);
        index.slod = whole.slod;
        index.switches = whole.switches;
        index.places = whole.places;
        index.outlines = whole.outlines;
        index.dictionaries = whole.dictionaries;
        index.report = whole.report;
        let index = index.finish();
        let (rows, places) = rows_of(&index, &whole.world);
        Ok((index, rows, places))
    }
}

impl WorldIndexer for Install<'_> {
    fn rules(&self) -> u32 {
        RULES
    }

    fn rows(&self, archive: usize, out: &mut Section) -> Result<(), WorldError> {
        if !self.mounted()?.world.contains(&archive) {
            Rows::put_none(out);
            return Ok(());
        }
        match self.scanned(archive)? {
            Some(rows) => rows.put(out),
            // An archive that did not open gives the world nothing this once, and is asked again.
            None => {
                Rows::default().put(out);
                out.in_part();
            }
        }
        Ok(())
    }

    fn whole(&self, archives: &[Section], out: &mut Section) -> Result<(), WorldError> {
        let mounted = self.mounted()?;
        // ⚠ An archive the mount names the world's since its section was written holds no rows
        // there, and its stamp stands: its rows are read here and kept in this section.
        let (mut late, mut opened) = (Vec::new(), true);
        for at in &mounted.world {
            let section = archives.get(*at).ok_or(WorldError::Lacks("such archive"))?;
            if Rows::read(section)?.is_none() {
                let rows = self.scanned(*at)?;
                opened &= rows.is_some();
                late.push((*at as u32, rows.unwrap_or_default()));
            }
        }
        let world: Vec<u32> = mounted.world.iter().map(|at| *at as u32).collect();
        let Gathered {
            mut index,
            facts,
            types_at,
            keys,
            unread,
            claims,
            words,
        } = gathered(&world, archives, late.clone())?;
        let (selectors, places, states_opened) = (self.disk)
            .selectors(&mounted.mount, &self.every, self.cancel)
            .ok_or(WorldError::Cancelled)?;
        index.lay_places(places);
        let (assumed, _) = self.fresh.map_or_else(Default::default, states::assumed);
        let bound = mounted.mount.bind(&self.every, &mounted.paths, &facts);
        place(&mut index.maps, &facts, &bound.modes);
        let mut dictionaries = dictionaries(&index.archetypes, &keys);
        // Found in no order: sorted, the section is the same bytes at every build.
        dictionaries.sort_unstable_by_key(|b| (b.word, b.archive, b.offset));
        let (sets_read, sets_unread, sets_ciphered) = (mounted.mount.registry.level_pack())
            .map_or((0, 0, 0), |pack| {
                (pack.sets.len(), pack.unread.len(), pack.ciphered.len())
            });
        let report = Report {
            packs: mounted.mount.registry.packs.len() as u64,
            sets_read: sets_read as u64,
            sets_unread: sets_unread as u64,
            sets_ciphered: sets_ciphered as u64,
            named_archives: bound.named_archives as u64,
            bound_archives: bound.bound_archives as u64,
            by_table: u64::from(bound.by_table),
            disagree: bound.disagree as u64,
            permanent_named: bound.permanent_named as u64,
            permanent_found: bound.permanent.intersection(&types_at).count() as u64,
            selectors: selectors.len() as u64,
            unread,
            ..counted(&index.maps)
        };
        let whole = Whole {
            world,
            late,
            modes: bound.modes,
            slod: slod::solve(&claims),
            switches: states::switches(&words, &selectors, &assumed),
            places: index.places,
            outlines: index.outlines,
            dictionaries,
            report,
        };
        whole.put(out);
        // An archive that did not open here gave this section nothing this once: the section is
        // made again at the next open, since no stamp of its own says it stands.
        if !(opened && states_opened) {
            out.in_part();
        }
        Ok(())
    }

    fn link(&self, archives: &[Section], whole: &Section) -> Result<Index, WorldError> {
        let (world, rows, places) = self.linked(archives, whole)?;
        let linked = Linked {
            world: Arc::new(world),
            places,
        };
        Ok(Index::new(rows)?.with_title(Arc::new(linked)))
    }

    fn placements(
        &self,
        _index: &Index,
        _map: MapId,
        _out: &mut Vec<PlacementRow>,
    ) -> Result<(), WorldError> {
        Err(WorldError::Lacks("placement rows"))
    }
}
