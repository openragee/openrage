//! The index of a GTA VI install the world viewer streams from: the maps the level registers with
//! their boxes, each archetype by its word, and where each model's body lies. It is built by one
//! pass over the level's archives on every core, and kept in the configuration folder while the
//! archives' sizes and modification times stand.
//!
//! ⚠ A map's entities are not in it: a map's body is read when the map streams in.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use or_io::container::rpf::{Rpf9BodyKind, Rpf9Role, Rpf9Scan, Rpf9ScanFind, Rpf9ZlibRun};
use or_io::gta6::hash::name_hash64;
use or_io::gta6::headerless::{read_dictionary, stated_model_word, Entry};
use or_io::gta6::map::{self, MapBody};
use or_job::{Cancel, Progress};
use or_title::gta6_mount::{self, HeldBack, MapFact, Registry};
use or_world::streaming::MapBox;

/// The bytes of an archive one scan reads before the next worker takes over.
const RANGE: u64 = 256 << 20;
/// The inflated bytes kept of a body: more than any map or dictionary was measured to hold.
const KEEP: usize = 96 << 20;
/// How far past the end of a range's last body the scan after it is checked against a fresh one.
const STITCH: u64 = 32 << 20;
/// An archive larger than this holds no registration document.
const SMALL: u64 = 1 << 30;
/// The fewest archives a device's folder must be named for to be scanned as the level's.
const NAMED: usize = 16;
const MAGIC: &[u8; 8] = b"ORWIDX\0\0";
/// The version of the kept index: a change to it, or to what a build reads, takes another.
const VERSION: u64 = 1;

/// A map of the index.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct MapRow {
    /// Which archive of the index it lies in.
    pub archive: u32,
    pub name: u32,
    pub block: u32,
    pub flags: u32,
    pub content_flags: u32,
    /// Why it does not stream at rest: 0 where it does, 1 not registered, 2 script managed, 3 empty.
    pub held: u32,
    /// The names of the maps its level-of-detail parents lie in: a run of the index's parent names.
    pub parents_from: u32,
    pub parents_len: u32,
    pub offset: u64,
    /// The streaming box, lowest corner then highest, then the entities' box.
    pub boxes: [f32; 12],
}

impl MapRow {
    pub fn boxes(&self) -> MapBox {
        let at = |k: usize| Vec3::from_slice(&self.boxes[3 * k..3 * k + 3]);
        MapBox {
            streaming: (at(0), at(1)),
            entities: (at(2), at(3)),
        }
    }
}

/// An archetype of the index, by its word.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct ArchetypeRow {
    pub word: u64,
    /// The word of the drawable or fragment that draws it; 0 for an interior or a dictionary's.
    pub model: u64,
    /// The word of the drawable dictionary that holds it under its name; 0 for none.
    pub dictionary: u64,
    pub name: u32,
    pub lod_dist: f32,
    pub centre: [f32; 3],
    pub radius: f32,
    pub flags: u32,
    pub pad: u32,
}

/// Where a model's body lies, by the word that names it.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Pod, Zeroable)]
pub struct BodyRow {
    pub word: u64,
    pub offset: u64,
    pub archive: u32,
    pub pad: u32,
}

/// The body that draws an archetype, and the entry of it where the body is a dictionary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModelRef {
    pub archive: u32,
    pub offset: u64,
    pub entry: Option<u32>,
}

/// What the mount and the build came to, for the screen to say.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct Report {
    pub packs: u64,
    pub sets_read: u64,
    pub sets_unread: u64,
    pub named_archives: u64,
    pub bound_archives: u64,
    pub maps: u64,
    pub registered: u64,
    pub at_rest: u64,
    pub script_managed: u64,
    pub empty: u64,
    pub archetypes: u64,
    pub models: u64,
    /// Bodies shaped as a map, a map types or a dictionary that did not read.
    pub unread: u64,
}

/// The index of one install.
#[derive(Debug, Default)]
pub struct WorldIndex {
    pub archives: Vec<PathBuf>,
    pub maps: Vec<MapRow>,
    pub parents: Vec<u32>,
    archetypes: Vec<ArchetypeRow>,
    models: Vec<BodyRow>,
    dictionaries: Vec<BodyRow>,
    pub report: Report,
    by_name: HashMap<u32, u32>,
}

fn by_word<T>(rows: &[T], word: u64, of: impl Fn(&T) -> u64) -> Option<&T> {
    let at = rows.binary_search_by_key(&word, of).ok()?;
    rows.get(at)
}

impl WorldIndex {
    pub fn archetype(&self, word: u64) -> Option<&ArchetypeRow> {
        by_word(&self.archetypes, word, |a| a.word)
    }

    /// The body that draws `archetype`; `None` for an interior, or a model no body of the index names.
    pub fn model(&self, archetype: &ArchetypeRow) -> Option<ModelRef> {
        let (rows, word, entry) = match archetype.dictionary {
            0 => (&self.models, archetype.model, None),
            word => (&self.dictionaries, word, Some(archetype.name)),
        };
        let body = by_word(rows, word, |b| b.word).filter(|_| word != 0)?;
        Some(ModelRef {
            archive: body.archive,
            offset: body.offset,
            entry,
        })
    }

    /// The map of the name `name`, by its place among the maps.
    pub fn map_named(&self, name: u32) -> Option<usize> {
        self.by_name.get(&name).map(|at| *at as usize)
    }

    pub fn parents_of(&self, map: &MapRow) -> &[u32] {
        let from = map.parents_from as usize;
        (self.parents.get(from..from + map.parents_len as usize)).unwrap_or_default()
    }

    /// The middle of the boxes of the maps that stream at rest, where a camera starts.
    pub fn middle(&self) -> Option<Vec3> {
        let mut sum = glam::DVec3::ZERO;
        let mut count = 0u32;
        for map in self.maps.iter().filter(|m| m.held == 0) {
            let b = map.boxes();
            let centre = (b.entities.0 + b.entities.1) / 2.0;
            if centre.is_finite() {
                sum += centre.as_dvec3();
                count += 1;
            }
        }
        (count > 0).then(|| (sum / f64::from(count)).as_vec3())
    }

    fn finish(mut self) -> Self {
        self.archetypes.sort_unstable_by_key(|a| a.word);
        // Of two bodies that state one archetype the later one stands.
        self.archetypes.reverse();
        self.archetypes.dedup_by_key(|a| a.word);
        self.archetypes.reverse();
        for rows in [&mut self.models, &mut self.dictionaries] {
            rows.sort_unstable_by_key(|b| (b.word, b.archive, b.offset));
            rows.dedup_by_key(|b| b.word);
        }
        self.by_name = (self.maps.iter().enumerate())
            .map(|(at, m)| (m.name, at as u32))
            .collect();
        self.report.archetypes = self.archetypes.len() as u64;
        self.report.models = (self.models.len() + self.dictionaries.len()) as u64;
        self
    }
}

/// What one scan of a range of an archive found.
#[derive(Default)]
struct Shard {
    /// Each map with the names of its parents' maps and whether it holds something to draw.
    maps: Vec<(MapRow, Vec<u32>, bool)>,
    archetypes: Vec<ArchetypeRow>,
    /// The word each drawable or fragment body names itself by, with its offset.
    models: Vec<(u64, u64)>,
    /// The codes of the drawables each dictionary body holds, with its offset.
    keys: Vec<(u64, Vec<u32>)>,
    /// Each nested archive's header, and where the names that end one end.
    archives: Vec<u64>,
    ends: Vec<(u64, u64)>,
    unread: u64,
    /// Every offset the scan found a body at, in order.
    offsets: Vec<u64>,
}

impl Shard {
    /// Keep what the body `find`, whose first bytes are `bytes`, gives the index.
    fn take(&mut self, find: &Rpf9ScanFind, bytes: &[u8]) {
        self.offsets.push(find.offset);
        let run = match find.kind {
            Rpf9BodyKind::Archive(_) => return self.archives.push(find.offset),
            Rpf9BodyKind::Names { archive } => {
                let end = find.offset + find.stored_len.unwrap_or(find.span);
                return self.ends.push((archive, end));
            }
            Rpf9BodyKind::Zlib(run) => run,
            _ => return,
        };
        let Rpf9ZlibRun {
            resource: Some(resource),
            signature,
            ..
        } = run
        else {
            return;
        };
        let whole = bytes.len() as u64 >= resource.virtual_len + resource.physical_len;
        let (virtual_section, physical) = resource.sections(bytes);
        match signature.and_then(|s| s.role()) {
            Some(Rpf9Role::Drawable | Rpf9Role::DrawableWithBound) => {
                if let Ok(word) = stated_model_word(virtual_section) {
                    self.models.push((word, find.offset));
                }
            }
            Some(Rpf9Role::Dictionary) if whole => {
                match read_dictionary(virtual_section, Some(physical)) {
                    Ok(dictionary) => {
                        let codes: Vec<u32> = (dictionary.entries.iter())
                            .filter(|(_, e)| matches!(e, Ok(Entry::Drawable(_))))
                            .map(|(code, _)| *code)
                            .collect();
                        if !codes.is_empty() {
                            self.keys.push((find.offset, codes));
                        }
                    }
                    Err(_) => self.unread += 1,
                }
            }
            Some(_) => {}
            // A map body states no signature: its class table stands where one would be read.
            None => match map::root_type(virtual_section) {
                Some(map::C_MAP_DATA) if whole => {
                    match map::stream::open(virtual_section, physical) {
                        Ok(m) => self.map(find.offset, &m),
                        Err(_) => self.unread += 1,
                    }
                }
                Some(map::C_MAP_TYPES) if whole => match map::open(virtual_section, physical) {
                    Ok(MapBody::Types(types)) => {
                        (self.archetypes).extend(types.archetypes.iter().map(|a| ArchetypeRow {
                            word: a.name_word,
                            model: a.drawable | a.fragment,
                            dictionary: a.drawable_dictionary,
                            name: a.name,
                            lod_dist: a.lod_dist,
                            centre: a.bs_centre,
                            radius: a.bs_radius,
                            flags: a.flags,
                            pad: 0,
                        }))
                    }
                    _ => self.unread += 1,
                },
                Some(map::C_MAP_DATA | map::C_MAP_TYPES) => self.unread += 1,
                _ => {}
            },
        }
    }

    fn map(&mut self, offset: u64, m: &map::stream::StreamMap) {
        let mut boxes = [0.0; 12];
        let corners = [
            m.streaming_extents.min,
            m.streaming_extents.max,
            m.entities_extents.min,
            m.entities_extents.max,
        ];
        for (slot, corner) in boxes.chunks_mut(3).zip(corners) {
            slot.copy_from_slice(&corner);
        }
        let row = MapRow {
            archive: 0,
            name: m.name,
            block: m.block,
            flags: m.flags,
            content_flags: m.content_flags,
            held: 0,
            parents_from: 0,
            parents_len: 0,
            offset,
            boxes,
        };
        let holds = !m.entities.is_empty() || !m.prop_lists.is_empty() || m.grass_lists > 0;
        self.maps.push((row, m.lod_parents.clone(), holds));
    }

    /// Drop what was found before `offset`, which lies inside a body of the range before.
    fn cut_before(&mut self, offset: u64) {
        self.maps.retain(|(m, ..)| m.offset >= offset);
        self.models.retain(|(_, at)| *at >= offset);
        self.keys.retain(|(at, _)| *at >= offset);
        self.archives.retain(|at| *at >= offset);
        self.ends.retain(|(_, end)| *end > offset);
        self.offsets.retain(|at| *at >= offset);
        // ⚠ Archetypes carry no offset: those of a body cut here stay, and the later body's stand.
    }

    fn append(&mut self, mut more: Shard) {
        self.maps.append(&mut more.maps);
        self.archetypes.append(&mut more.archetypes);
        self.models.append(&mut more.models);
        self.keys.append(&mut more.keys);
        self.archives.append(&mut more.archives);
        self.ends.append(&mut more.ends);
        self.unread += more.unread;
        self.offsets.append(&mut more.offsets);
    }
}

/// One range of an archive as a worker scanned it: its place, where its scan ended, and its finds.
struct Ranged {
    index: u64,
    end: u64,
    shard: Shard,
}

/// Scan `from..to` of `file` into `shard`, up to the first body at an offset of `known`: where the
/// scan ended, and that offset where it stopped at one.
fn scan_into(
    file: &File,
    (from, to): (u64, u64),
    shard: &mut Shard,
    stop: &Cancel,
    known: Option<&HashSet<u64>>,
) -> io::Result<(u64, Option<u64>)> {
    let mut scan = (Rpf9Scan::new(file, from..to).map_err(io::Error::other)?).keeping(KEEP);
    while let Some(find) = scan.next() {
        // A scan that fails says so once and ends: the rest of its range is not read.
        let (Ok(find), false) = (find, stop.is_set()) else {
            break;
        };
        if known.is_some_and(|known| known.contains(&find.offset)) {
            return Ok((find.offset, Some(find.offset)));
        }
        let bytes = scan.kept(&find);
        shard.take(&find, bytes);
    }
    Ok((scan.position(), None))
}

/// Everything one archive gives the index, read a range a worker; `None` once `cancel` is set.
fn scan_archive(
    path: &Path,
    cancel: &Cancel,
    reached: &(dyn Fn(u64) + Sync),
) -> io::Result<Option<Shard>> {
    let len = std::fs::metadata(path)?.len();
    let ranges = len.div_ceil(RANGE).max(1);
    let workers = (std::thread::available_parallelism().map_or(4, usize::from) as u64).min(ranges);
    let next = AtomicU64::new(0);
    let work = || -> io::Result<Vec<Ranged>> {
        let file = File::open(path)?;
        let mut out = Vec::new();
        loop {
            let index = next.fetch_add(1, Ordering::Relaxed);
            if index >= ranges || cancel.is_set() {
                return Ok(out);
            }
            let (start, to) = (index * RANGE, ((index + 1) * RANGE).min(len));
            let mut shard = Shard::default();
            let (end, _) = scan_into(&file, (start, to), &mut shard, cancel, None)?;
            out.push(Ranged { index, end, shard });
            reached(to - start);
        }
    };
    let mut ranged = Vec::new();
    std::thread::scope(|scope| -> io::Result<()> {
        let running: Vec<_> = (0..workers).map(|_| scope.spawn(work)).collect();
        for worker in running {
            let done = worker
                .join()
                .map_err(|_| io::Error::other("a scan panicked"))?;
            ranged.extend(done?);
        }
        Ok(())
    })?;
    if cancel.is_set() {
        return Ok(None);
    }
    ranged.sort_unstable_by_key(|r| r.index);
    let file = File::open(path)?;
    let mut whole = Shard::default();
    let mut ended = 0u64;
    for Ranged { mut shard, end, .. } in ranged {
        // ⚠ A range's scan may start inside the last body of the range before, and read its
        // streams as bodies of their own until it finds its feet: what it found before that body's
        // end is dropped, and a scan from that end stands in for it up to where the two agree.
        shard.cut_before(ended);
        if ended > 0 && ended < len {
            let mut bridge = Shard::default();
            let known: HashSet<u64> = shard.offsets.iter().copied().collect();
            let window = (ended, (ended + STITCH).min(len));
            let (_, agreed) = scan_into(&file, window, &mut bridge, cancel, Some(&known))?;
            if let Some(agreed) = agreed {
                shard.cut_before(agreed);
                whole.append(bridge);
            }
        }
        ended = ended.max(end);
        whole.append(shard);
    }
    Ok(Some(whole))
}

/// The nested archive that holds the body at `offset`: the last header before it whose names have
/// not ended it; `None` at the archive's own level.
fn container_of(archives: &[(u64, Option<u64>)], offset: u64) -> Option<u64> {
    let mut at = archives.partition_point(|(start, _)| *start <= offset);
    while at > 0 {
        at -= 1;
        let (start, end) = archives[at];
        if end.is_none_or(|end| offset < end) {
            return Some(start).filter(|start| *start != 0);
        }
    }
    None
}

/// Every `.rpf` under `root`, the folder's own first.
fn archives_under(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut folders = vec![(root.to_path_buf(), 0)];
    while let Some((folder, depth)) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && depth < 4 {
                folders.push((path, depth + 1));
            } else if (path.extension()).is_some_and(|e| e.eq_ignore_ascii_case("rpf")) {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// The first folder each registered archive of the level is named under on the platform device,
/// lower-cased, with how many are.
fn level_folders(files: &[&or_title::dlc::RegisteredFile]) -> HashMap<String, usize> {
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

/// Read the registration documents under `root`, bind the level they register to the bodies of its
/// archives and index them; `None` once `cancel` is set.
pub fn build(
    root: &Path,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<Option<WorldIndex>, String> {
    let stopped = || cancel.is_set();
    let every = archives_under(root);
    let mut documents = Vec::new();
    for path in &every {
        let small = std::fs::metadata(path).is_ok_and(|m| m.len() <= SMALL);
        if !small {
            continue;
        }
        if stopped() {
            return Ok(None);
        }
        match gta6_mount::read_documents(path, &stopped) {
            Ok(found) if !found.is_empty() => documents.push((
                path.clone(),
                found.into_iter().map(|f| f.document).collect(),
            )),
            Ok(_) => {}
            Err(error) => tracing::info!(path = %path.display(), %error, "no document read"),
        }
    }
    let registry = Registry::of(documents);
    let level = registry.level_pack();
    let files: Vec<&or_title::dlc::RegisteredFile> = (level.iter())
        .flat_map(|pack| pack.sets.iter().filter(|set| set.level))
        .flat_map(|set| set.files.iter())
        .collect();
    let folders = level_folders(&files);
    // The archive of a folder the level names many archives under is the level's; with none so
    // named every archive is read.
    let named = |path: &Path| {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_ascii_lowercase());
        path.parent() == Some(root)
            && stem.is_some_and(|stem| folders.get(&stem).is_some_and(|n| *n >= NAMED))
    };
    let mut archives: Vec<PathBuf> = every.iter().filter(|p| named(p)).cloned().collect();
    if archives.is_empty() {
        archives = every.clone();
    }
    let total: u64 = (archives.iter())
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();
    let done = AtomicU64::new(0);
    let reached = |step: u64| {
        let now = done.fetch_add(step, Ordering::Relaxed) + step;
        progress.reached(now.min(total), total);
    };

    let mut index = WorldIndex {
        archives: archives.clone(),
        ..WorldIndex::default()
    };
    let mut facts = Vec::new();
    let mut keys: HashMap<u32, Vec<BodyRow>> = HashMap::new();
    for (at, path) in archives.iter().enumerate() {
        let archive = at as u32;
        let shard = match scan_archive(path, cancel, &reached) {
            Ok(Some(shard)) => shard,
            Ok(None) => return Ok(None),
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "an archive was not indexed");
                continue;
            }
        };
        let mut nested: Vec<(u64, Option<u64>)> = (shard.archives.iter())
            .map(|start| {
                let end = (shard.ends.iter())
                    .find(|(a, _)| a == start)
                    .map(|(_, end)| *end);
                (*start, end)
            })
            .collect();
        nested.sort_unstable();
        for (mut row, parents, holds) in shard.maps {
            row.archive = archive;
            row.parents_from = index.parents.len() as u32;
            row.parents_len = parents.len() as u32;
            index.parents.extend(parents);
            facts.push(MapFact {
                archive,
                container: container_of(&nested, row.offset),
                name: row.name,
                block: row.block,
                flags: row.flags,
                holds,
            });
            index.maps.push(row);
        }
        index.archetypes.extend(shard.archetypes);
        (index.models).extend(shard.models.into_iter().map(|(word, offset)| BodyRow {
            word,
            offset,
            archive,
            pad: 0,
        }));
        for (offset, codes) in shard.keys {
            for code in codes {
                keys.entry(code).or_default().push(BodyRow {
                    word: 0,
                    offset,
                    archive,
                    pad: 0,
                });
            }
        }
        index.report.unread += shard.unread;
    }

    let binding = gta6_mount::bind(&files, &facts);
    for (row, held) in index.maps.iter_mut().zip(&binding.held) {
        row.held = match held {
            None => 0,
            Some(HeldBack::NotRegistered) => 1,
            Some(HeldBack::ScriptManaged) => 2,
            Some(HeldBack::Empty) => 3,
        };
    }
    index.dictionaries = dictionaries(&index.archetypes, &keys);
    let sets = |read: bool| {
        (level.iter())
            .map(|pack| match read {
                true => pack.sets.len(),
                false => pack.unread.len(),
            })
            .sum::<usize>() as u64
    };
    index.report = Report {
        packs: registry.packs.len() as u64,
        sets_read: sets(true),
        sets_unread: sets(false),
        named_archives: binding.report.named_archives as u64,
        bound_archives: binding.report.bound_archives as u64,
        maps: binding.report.maps as u64,
        registered: binding.report.registered as u64,
        at_rest: binding.report.at_rest as u64,
        script_managed: binding.report.script_managed as u64,
        empty: binding.report.empty as u64,
        unread: index.report.unread,
        ..Report::default()
    };
    Ok(Some(index.finish()))
}

/// The one dictionary body that holds a drawable under every name the archetypes of a dictionary
/// word bear, for each such word.
fn dictionaries(archetypes: &[ArchetypeRow], keys: &HashMap<u32, Vec<BodyRow>>) -> Vec<BodyRow> {
    let mut names: HashMap<u64, Vec<u32>> = HashMap::new();
    for a in archetypes.iter().filter(|a| a.dictionary != 0) {
        names.entry(a.dictionary).or_default().push(a.name);
    }
    let mut found = Vec::new();
    for (word, names) in names {
        let mut holders: Option<HashSet<(u32, u64)>> = None;
        for name in &names {
            let bodies: HashSet<(u32, u64)> = (keys.get(name).into_iter().flatten())
                .map(|b| (b.archive, b.offset))
                .collect();
            holders = Some(match holders {
                None => bodies,
                Some(held) => held.intersection(&bodies).copied().collect(),
            });
        }
        let holders = holders.unwrap_or_default();
        if let (1, Some((archive, offset))) = (holders.len(), holders.iter().next()) {
            found.push(BodyRow {
                word,
                offset: *offset,
                archive: *archive,
                pad: 0,
            });
        }
    }
    found
}

/// The size and modification time of the archive at `path`, in nanoseconds.
fn stat(path: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((meta.len(), modified.as_nanos() as u64))
}

fn put_rows<T: Pod>(out: &mut impl Write, rows: &[T]) -> io::Result<()> {
    out.write_all(&(rows.len() as u64).to_le_bytes())?;
    out.write_all(bytemuck::cast_slice(rows))
}

fn word(from: &mut impl Read) -> io::Result<u64> {
    let mut bytes = [0; 8];
    from.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

/// The rows the next count of `from` states, read a block at a time so a count the bytes do not
/// bear out fails at their end and reserves nothing.
fn rows<T: Pod>(from: &mut impl Read) -> io::Result<Vec<T>> {
    let count = word(from)?;
    let mut out = Vec::new();
    let mut left = count;
    let mut block = [T::zeroed(); 1024];
    while left > 0 {
        let take = left.min(block.len() as u64) as usize;
        from.read_exact(bytemuck::cast_slice_mut(&mut block[..take]))?;
        out.extend_from_slice(&block[..take]);
        left -= take as u64;
    }
    Ok(out)
}

/// Where the index of the install at `root` is kept under the configuration folder `config`.
pub fn kept_at(config: &Path, root: &Path) -> PathBuf {
    let name = name_hash64(root.to_string_lossy().as_bytes());
    config.join("gta6_world").join(format!("{name:016x}.index"))
}

impl WorldIndex {
    /// What a kept index starts with: its version, then each archive's path, size and time.
    fn head(archives: &[PathBuf]) -> Vec<u8> {
        let mut head = MAGIC.to_vec();
        head.extend(VERSION.to_le_bytes());
        head.extend((archives.len() as u64).to_le_bytes());
        for path in archives {
            let (len, time) = stat(path).unwrap_or_default();
            let text = path.to_string_lossy();
            for value in [len, time, text.len() as u64] {
                head.extend(value.to_le_bytes());
            }
            head.extend(text.as_bytes());
        }
        head
    }

    /// Keep the index at `file`, written whole beside it and then moved there.
    pub fn keep(&self, file: &Path) -> io::Result<()> {
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let beside = file.with_extension("tmp");
        let mut out = BufWriter::new(File::create(&beside)?);
        let head = Self::head(&self.archives);
        out.write_all(&(head.len() as u64).to_le_bytes())?;
        out.write_all(&head)?;
        put_rows(&mut out, &self.maps)?;
        put_rows(&mut out, &self.parents)?;
        put_rows(&mut out, &self.archetypes)?;
        put_rows(&mut out, &self.models)?;
        put_rows(&mut out, &self.dictionaries)?;
        put_rows(&mut out, std::slice::from_ref(&self.report))?;
        out.flush()?;
        drop(out);
        std::fs::rename(beside, file)
    }

    /// The index kept at `file`, where the archives it was built from are as they were.
    pub fn kept(file: &Path) -> Option<Self> {
        let mut from = BufReader::new(File::open(file).ok()?);
        let len = usize::try_from(word(&mut from).ok()?).ok()?;
        let mut head = vec![0; len.min(1 << 20)];
        from.read_exact(&mut head).ok()?;
        let archives = archives_of(&head)?;
        if head != Self::head(&archives) {
            return None;
        }
        let read = |from: &mut BufReader<File>| -> io::Result<Self> {
            let mut index = WorldIndex {
                archives,
                maps: rows(from)?,
                parents: rows(from)?,
                archetypes: rows(from)?,
                models: rows(from)?,
                dictionaries: rows(from)?,
                ..WorldIndex::default()
            };
            let report: Vec<Report> = rows(from)?;
            index.report = report.first().copied().unwrap_or_default();
            // A kept run past the names is a file cut short or of another build.
            let parents = index.parents.len() as u64;
            let runs = (index.maps.iter())
                .all(|m| u64::from(m.parents_from) + u64::from(m.parents_len) <= parents);
            let archives = index.archives.len() as u32;
            let placed = index.maps.iter().all(|m| m.archive < archives);
            match runs && placed {
                true => Ok(index),
                false => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "a kept index out of step",
                )),
            }
        };
        read(&mut from)
            .inspect_err(|e| tracing::info!("{}: {e}", file.display()))
            .ok()
            .map(Self::finish)
    }
}

/// The next 64-bit word of `rest`, which moves past it.
fn next_word(rest: &mut &[u8]) -> Option<u64> {
    let (value, after) = rest.split_first_chunk::<8>()?;
    *rest = after;
    Some(u64::from_le_bytes(*value))
}

/// The archives a kept index's head names.
fn archives_of(head: &[u8]) -> Option<Vec<PathBuf>> {
    let mut rest = head.strip_prefix(MAGIC.as_slice())?;
    if next_word(&mut rest)? != VERSION {
        return None;
    }
    let count = next_word(&mut rest)?;
    let mut archives = Vec::new();
    for _ in 0..count {
        let (_len, _time) = (next_word(&mut rest)?, next_word(&mut rest)?);
        let text = usize::try_from(next_word(&mut rest)?).ok()?;
        let (path, after) = rest.split_at_checked(text)?;
        rest = after;
        archives.push(PathBuf::from(String::from_utf8_lossy(path).into_owned()));
    }
    Some(archives)
}

/// Indexes made up for the tests of this folder.
#[cfg(test)]
pub(super) mod tests_support {
    use super::*;

    /// An index of `archives` with an archetype for each word, the distance it draws to and the
    /// offset of its model where it has one, and the maps `maps` with the parent names `parents`.
    pub fn index_with(
        archives: Vec<PathBuf>,
        archetypes: &[(u64, f32, Option<u64>)],
        maps: Vec<MapRow>,
        parents: Vec<u32>,
    ) -> WorldIndex {
        let mut index = WorldIndex {
            archives,
            maps,
            parents,
            ..WorldIndex::default()
        };
        for (word, lod_dist, model) in archetypes {
            index.archetypes.push(ArchetypeRow {
                word: *word,
                model: model.map_or(0, |_| word + 1),
                dictionary: 0,
                name: *word as u32,
                lod_dist: *lod_dist,
                centre: [0.0; 3],
                radius: 1.0,
                flags: 0,
                pad: 0,
            });
            index.models.extend(model.map(|offset| BodyRow {
                word: word + 1,
                offset,
                archive: 0,
                pad: 0,
            }));
        }
        index.finish()
    }

    pub fn index_of(archives: Vec<PathBuf>, archetypes: &[(u64, f32, Option<u64>)]) -> WorldIndex {
        index_with(archives, archetypes, Vec::new(), Vec::new())
    }
}

#[cfg(test)]
#[path = "index_tests.rs"]
mod tests;
