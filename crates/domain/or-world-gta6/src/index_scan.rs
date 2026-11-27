//! One pass over an archive for the index: every map, every map-types body, every model and
//! every drawable dictionary a scan finds, read a range a worker.

use std::collections::HashSet;
use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use or_io::container::rpf::{Rpf9BodyKind, Rpf9Role, Rpf9Scan, Rpf9ScanFind, Rpf9ZlibRun};
use or_io::gta6::headerless::{read_dictionary, stated_model_word, Entry};
use or_io::gta6::map::stream::grass::slod::SlodClaim;
use or_io::gta6::map::{self, MapBody};
use or_io::meta::binary::rbm::gta6::read_model;
use or_job::Cancel;

use super::types::{self, ALL_HOURS};
use super::{ArchetypeRow, MapRow};

/// The bytes of an archive one scan reads before the next worker takes over.
const RANGE: u64 = 256 << 20;
/// The inflated bytes kept of a body: more than any map or dictionary was measured to hold.
const KEEP: usize = 96 << 20;
/// How far past the end of a range's last body the scan after it is checked against a fresh one.
const STITCH: u64 = 32 << 20;

/// What one scan of a range of an archive found.
#[derive(Default)]
pub(super) struct Shard {
    /// Each map with the names of its parents' maps and whether it holds something to draw.
    pub(super) maps: Vec<(MapRow, Vec<u32>, bool)>,
    pub(super) archetypes: Vec<ArchetypeRow>,
    /// The lights the archetypes of each map-types body hold.
    pub(super) lights: Vec<super::lights_index::LightRow>,
    /// The group a script-managed map states, with the 64-bit word the same name hashes to.
    pub(super) words: Vec<(u32, u64)>,
    /// Where each map-types body that read lies.
    pub(super) types: Vec<u64>,
    /// The word each drawable or fragment body names itself by, with its offset.
    pub(super) models: Vec<(u64, u64)>,
    /// The word of each interior a map-types body states, with the body's offset.
    pub(super) interiors: Vec<(u64, u64)>,
    /// The codes of the drawables each dictionary body holds, with its offset.
    pub(super) keys: Vec<(u64, Vec<u32>)>,
    /// Each nested archive's header, and where the names that end one end.
    pub(super) archives: Vec<u64>,
    pub(super) ends: Vec<(u64, u64)>,
    pub(super) unread: u64,
    /// What each far stand-in list of each map says of its ids.
    pub(super) slod: Vec<SlodClaim>,
    /// Every offset the scan found a body at, in order.
    pub(super) offsets: Vec<u64>,
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
            None => {
                let kind = map::root_type(virtual_section);
                if matches!(kind, Some(map::C_MAP_DATA | map::C_MAP_TYPES)) {
                    let read = whole
                        .then(|| self.map_body(find.offset, virtual_section, physical))
                        .flatten();
                    if read.is_none() {
                        self.unread += 1;
                    }
                }
            }
        }
    }

    /// Keep the map or the map types of these two sections; `None` where the body does not read.
    fn map_body(&mut self, offset: u64, virtual_section: &[u8], physical: &[u8]) -> Option<()> {
        let model = read_model(virtual_section, physical).ok()?;
        if map::root_type(virtual_section) == Some(map::C_MAP_DATA) {
            let m = map::stream::read(&model).ok()?;
            // A body whose class table moves the group reads as a map of no group.
            let group = types::group_of(&model).unwrap_or(0);
            let word = types::word_of(&model).unwrap_or(0);
            if group != 0 && word != 0 {
                self.words.push((group, word));
            }
            self.map(offset, &m, group);
            self.slod.extend(m.grass.iter().filter_map(SlodClaim::of));
            return Some(());
        }
        let MapBody::Types(body) = map::read(&model).ok()? else {
            return None;
        };
        // ⚠ A body whose hours do not read keeps its archetypes, on at every hour.
        let hours = types::hours_of(&model).filter(|h| h.len() == body.archetypes.len());
        if hours.is_none() {
            tracing::debug!(offset, "the hours of a map-types body are not read");
        }
        // ⚠ A body whose load flags do not read keeps its archetypes, of no kind.
        let loads = types::load_flags_of(&model).filter(|l| l.len() == body.archetypes.len());
        self.types.push(offset);
        self.lights.extend(super::lights_index::rows_of(&model));
        (self.interiors).extend(
            (body.archetypes.iter())
                .filter(|a| a.class == map::ArchetypeClass::Interior)
                .map(|a| (a.name_word, offset)),
        );
        (self.archetypes).extend(
            body.archetypes
                .iter()
                .enumerate()
                .map(|(k, a)| ArchetypeRow {
                    word: a.name_word,
                    model: a.drawable | a.fragment,
                    dictionary: a.drawable_dictionary,
                    name: a.name,
                    lod_dist: a.lod_dist,
                    centre: a.bs_centre,
                    radius: a.bs_radius,
                    flags: a.flags,
                    hours: hours.as_ref().map_or(ALL_HOURS, |h| h[k]),
                    load_flags: loads.as_ref().map_or(0, |l| l[k]),
                    pad: 0,
                }),
        );
        Some(())
    }

    fn map(&mut self, offset: u64, m: &map::stream::StreamMap, group: u32) {
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
            group,
            modes: 0,
            offset,
            boxes,
        };
        let holds = !m.entities.is_empty() || !m.prop_lists.is_empty() || m.grass_lists > 0;
        self.maps.push((row, m.lod_parents.clone(), holds));
    }

    /// Drop what was found before `offset`, which lies inside a body of the range before.
    pub(super) fn cut_before(&mut self, offset: u64) {
        self.maps.retain(|(m, ..)| m.offset >= offset);
        self.types.retain(|at| *at >= offset);
        self.models.retain(|(_, at)| *at >= offset);
        self.interiors.retain(|(_, at)| *at >= offset);
        self.keys.retain(|(at, _)| *at >= offset);
        self.archives.retain(|at| *at >= offset);
        self.ends.retain(|(_, end)| *end > offset);
        self.offsets.retain(|at| *at >= offset);
        // ⚠ Archetypes and group words carry no offset: those of a body cut here stay, and the
        // later body's stand.
    }

    fn append(&mut self, mut more: Shard) {
        self.maps.append(&mut more.maps);
        self.archetypes.append(&mut more.archetypes);
        self.lights.append(&mut more.lights);
        self.words.append(&mut more.words);
        self.types.append(&mut more.types);
        self.models.append(&mut more.models);
        self.interiors.append(&mut more.interiors);
        self.keys.append(&mut more.keys);
        self.archives.append(&mut more.archives);
        self.ends.append(&mut more.ends);
        self.unread += more.unread;
        self.slod.append(&mut more.slod);
        self.offsets.append(&mut more.offsets);
    }

    /// Each nested archive's header with where its names end it, where they were found, in order.
    pub(super) fn nested(&self) -> Vec<(u64, Option<u64>)> {
        let mut nested: Vec<(u64, Option<u64>)> = (self.archives.iter())
            .map(|start| {
                let end = (self.ends.iter())
                    .find(|(a, _)| a == start)
                    .map(|(_, end)| *end);
                (*start, end)
            })
            .collect();
        nested.sort_unstable();
        nested
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
pub(super) fn scan_archive(
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
pub(super) fn container_of(archives: &[(u64, Option<u64>)], offset: u64) -> Option<u64> {
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
