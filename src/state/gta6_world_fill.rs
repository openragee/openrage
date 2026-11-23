//! The world of a whole GTA VI archive read on a worker for the Map screen: every map's entity
//! rows, each archetype's box, every light placed in the world and the lane graph, a body at a
//! time with progress and a stop, kept in the configuration folder while the archive's size and
//! modification time stand. Where each world body starts and the ground a map covers are kept
//! beside it, so a region reads only the maps that meet it.
//!
//! ⚠ An archetype's lights stand once at each entity that places it, and an interior's once at each
//! `CMloInstanceDef` that places the interior, so they are placed when every body is read.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Instant, UNIX_EPOCH};

use or_io::container::rpf::rpf9_scan::Rpf9BodyReader;
use or_io::container::rpf::{Rpf9BodyKind, Rpf9Scan, Rpf9ScanCarry, Rpf9ScanFind, Rpf9ZlibRun};
use or_io::gta6::map::lights::{LightEffect, LightOwner, Pose};
use or_io::gta6::map::world::{read_area_body, AreaBody, IndexedMap, PlacementRow};
use or_io::gta6::map::{Aabb, MapBody, LOD_LEVELS};
use or_job::{Cancel, Progress};

use super::{Area, Filled, Lanes, Light};
use crate::state::jobs::{file_name, JobKind, Outcome, RunningJob, Then};

/// What one pass of a scan for the bodies reads before it says how far it is.
const SLICE: u64 = 64 << 20;
/// The bytes a body is read to: more than any map body was measured to hold.
const BODY_LIMIT: u64 = 256 << 20;
/// The version of what is kept of a fill: a change to it, or to what a fill reads, takes another.
const VERSION: u64 = 1;
const MAGIC: &[u8; 8] = b"ORWORLD\0";

/// A rectangle of the ground plane, its lowest corner and its highest.
pub type Region = ([f32; 2], [f32; 2]);

fn meets(region: &Region, b: &Aabb) -> bool {
    (0..2).all(|i| b.min[i] <= region.1[i] && b.max[i] >= region.0[i])
}

/// What a world body of an archive is, as a fill of a region asks before it reads the body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Extent {
    /// A map that places entities, and the box they are streamed in.
    Map(Aabb),
    /// Archetypes or a lane list, which a fill of any region reads.
    Everywhere,
}

/// The offsets among `bodies` a fill of `region` reads: each map that meets it, and every body
/// that is no map.
pub(super) fn bodies_of(bodies: &[(u64, Extent)], region: &Region) -> Vec<u64> {
    let reads = |extent: &Extent| match extent {
        Extent::Map(streamed) => meets(region, streamed),
        Extent::Everywhere => true,
    };
    let wanted = bodies.iter().filter(|(_, extent)| reads(extent));
    wanted.map(|(offset, _)| *offset).collect()
}

/// Whether `find` starts a headerless resource with no signature, as a map body and a lane list do.
pub(super) fn starts_a_resource(find: &Rpf9ScanFind) -> bool {
    let unsigned = matches!(
        find.kind,
        Rpf9BodyKind::Zlib(Rpf9ZlibRun {
            resource: Some(_),
            signature: None,
            ..
        })
    );
    unsigned && find.stored_len.is_some()
}

fn place(out: &mut Vec<Light>, pose: &Pose, effect: &LightEffect) {
    for light in effect.lights() {
        let at = pose.place(effect, light).position;
        out.push(Light {
            at: [at[0], at[1]],
            colour: light.colour,
            range: light.falloff,
        });
    }
}

/// What a fill has read so far, a body at a time.
#[derive(Default)]
pub(super) struct Gathered {
    filled: Filled,
    /// The light effects of each archetype, by its word.
    archetype_lights: HashMap<u64, Vec<LightEffect>>,
    /// The light effects inside each interior, by the interior's word, each with its entity's pose.
    interior_lights: HashMap<u64, Vec<(Pose, LightEffect)>>,
    lanes: Lanes,
    /// Where each body that gives a fill something starts, and what it is, whatever the region.
    bodies: Vec<(u64, Extent)>,
}

impl Gathered {
    fn body(&mut self, bodies: &mut Rpf9BodyReader<&File>, offset: u64, region: Option<&Region>) {
        let Ok((find, bytes)) = bodies.read(offset, BODY_LIMIT) else {
            return;
        };
        let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
            resource: Some(resource),
            ..
        }) = find.kind
        else {
            return;
        };
        let (virtual_section, physical) = resource.sections(&bytes);
        if let Some(body) = read_area_body(virtual_section, physical) {
            self.add(offset, body, region);
        }
    }

    /// Keep what `body`, found at `offset`, gives; a map whose streaming box is outside `region`
    /// gives nothing.
    pub(super) fn add(&mut self, offset: u64, body: AreaBody, region: Option<&Region>) {
        let extent = match &body {
            // A map that places no entity gives a fill nothing.
            AreaBody::Map(MapBody::Data(map), _) if map.entities.is_empty() => None,
            AreaBody::Map(MapBody::Data(map), _) => Some(Extent::Map(map.streaming_extents)),
            _ => Some(Extent::Everywhere),
        };
        self.bodies.extend(extent.map(|extent| (offset, extent)));
        match body {
            AreaBody::Map(MapBody::Data(map), lights) => {
                if region.is_some_and(|r| !meets(r, &map.streaming_extents)) {
                    return;
                }
                self.filled.index.add(offset, &map);
                for (owner, effect) in lights.iter().flat_map(|l| &l.effects) {
                    let LightOwner::Entity(k) = owner else {
                        continue;
                    };
                    if let Some(entity) = map.entities.get(*k) {
                        place(&mut self.filled.lights, &Pose::of(entity), effect);
                    }
                }
            }
            AreaBody::Map(MapBody::Types(types), lights) => {
                for a in &types.archetypes {
                    (self.filled.boxes.entry(a.name_word)).or_insert([a.bb_min, a.bb_max]);
                }
                let word = |k: usize| types.archetypes.get(k).map(|a| a.name_word);
                let mut inside: HashMap<u64, Vec<(Pose, LightEffect)>> = HashMap::new();
                for (owner, effect) in lights.into_iter().flat_map(|l| l.effects) {
                    match owner {
                        LightOwner::Archetype(k) => {
                            let Some(word) = word(k) else {
                                continue;
                            };
                            let held = self.archetype_lights.entry(word).or_default();
                            // An archetype two bodies state adds its effect once.
                            if !held.contains(&effect) {
                                held.push(effect);
                            }
                        }
                        LightOwner::InteriorEntity {
                            archetype, pose, ..
                        }
                        | LightOwner::EntitySetEntity {
                            archetype, pose, ..
                        } => {
                            if let Some(word) = word(archetype) {
                                inside.entry(word).or_default().push((pose, effect));
                            }
                        }
                        LightOwner::Entity(_) => {}
                    }
                }
                // The first body to state an interior holds its lights.
                for (word, lights) in inside {
                    self.interior_lights.entry(word).or_insert(lights);
                }
            }
            AreaBody::Lanes(body) => self.lanes.keep(body),
        }
    }

    /// Where each body read so far that gives a fill something starts, and what it is.
    pub(super) fn bodies(&self) -> &[(u64, Extent)] {
        &self.bodies
    }

    /// What was read, with each archetype's lights at every entity that places it and each
    /// interior's at every entity that places the interior.
    pub(super) fn finish(self) -> Filled {
        let Self {
            mut filled,
            archetype_lights,
            interior_lights,
            lanes,
            bodies: _,
        } = self;
        let lights = &mut filled.lights;
        for row in &filled.index.rows {
            let pose = row.pose();
            for effect in archetype_lights
                .get(&row.archetype_word)
                .into_iter()
                .flatten()
            {
                place(lights, &pose, effect);
            }
            if !row.interior {
                continue;
            }
            for (inside, effect) in interior_lights
                .get(&row.archetype_word)
                .into_iter()
                .flatten()
            {
                place(lights, &inside.within(&pose), effect);
            }
        }
        // A box no entity read places is not drawn: most archetypes of an archive, for a region.
        let placed: HashSet<u64> = (filled.index.rows.iter())
            .map(|r| r.archetype_word)
            .collect();
        filled.boxes.retain(|word, _| placed.contains(word));
        filled.lane_points = lanes.points;
        filled.lane_segments = lanes.segments;
        filled.lane_signals = lanes.signals;
        filled
    }
}

fn put_u64(out: &mut impl Write, value: u64) -> io::Result<()> {
    out.write_all(&value.to_le_bytes())
}

fn put_floats(out: &mut impl Write, values: &[f32]) -> io::Result<()> {
    values
        .iter()
        .try_for_each(|v| out.write_all(&v.to_le_bytes()))
}

fn get<const N: usize>(from: &mut impl Read) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    from.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn get_u64(from: &mut impl Read) -> io::Result<u64> {
    get(from).map(u64::from_le_bytes)
}

fn get_floats<const N: usize>(from: &mut impl Read) -> io::Result<[f32; N]> {
    let mut out = [0.0; N];
    for slot in &mut out {
        *slot = f32::from_le_bytes(get(from)?);
    }
    Ok(out)
}

/// The next count of `from`, and `read` called that many times: a count the bytes do not bear out
/// fails at their end, and nothing is reserved by it.
fn list<R: Read, T>(
    from: &mut R,
    mut read: impl FnMut(&mut R) -> io::Result<T>,
) -> io::Result<Vec<T>> {
    let count = get_u64(from)?;
    let mut out = Vec::new();
    for _ in 0..count {
        out.push(read(from)?);
    }
    Ok(out)
}

fn not_kept(what: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what)
}

impl Filled {
    /// The bytes its lists hold, less what a map and an allocation add around them.
    fn held_bytes(&self) -> usize {
        size_of_val(&self.index.maps[..])
            + size_of_val(&self.index.rows[..])
            + self.boxes.len() * size_of::<(u64, [[f32; 3]; 2])>()
            + size_of_val(&self.lights[..])
            + size_of_val(&self.lane_points[..])
            + size_of_val(&self.lane_segments[..])
            + size_of_val(&self.lane_signals[..])
    }

    /// Write everything a fill read, as [`Filled::read`] reads it.
    pub(super) fn write(&self, out: &mut impl Write) -> io::Result<()> {
        put_u64(out, self.index.maps.len() as u64)?;
        for m in &self.index.maps {
            put_u64(out, m.offset)?;
            out.write_all(&m.name.to_le_bytes())?;
            out.write_all(&m.parent.to_le_bytes())?;
            put_floats(out, &m.streaming_extents.min)?;
            put_floats(out, &m.streaming_extents.max)?;
            put_u64(out, m.rows.start as u64)?;
            put_u64(out, m.rows.end as u64)?;
        }
        put_u64(out, self.index.rows.len() as u64)?;
        for r in &self.index.rows {
            out.write_all(&r.archetype_name.to_le_bytes())?;
            put_u64(out, r.archetype_word)?;
            put_floats(out, &r.position)?;
            put_floats(out, &r.rotation)?;
            put_floats(out, &[r.scale_xy, r.scale_z, r.lod_dist])?;
            out.write_all(&r.lod_parent.to_le_bytes())?;
            out.write_all(&[r.lod_level, u8::from(r.interior)])?;
        }
        put_u64(out, self.boxes.len() as u64)?;
        for (word, [min, max]) in &self.boxes {
            put_u64(out, *word)?;
            put_floats(out, min)?;
            put_floats(out, max)?;
        }
        put_u64(out, self.lights.len() as u64)?;
        for l in &self.lights {
            put_floats(out, &[l.at[0], l.at[1], l.range])?;
            out.write_all(&l.colour)?;
        }
        put_u64(out, self.lane_points.len() as u64)?;
        (self.lane_points.iter()).try_for_each(|p| put_floats(out, p))?;
        put_u64(out, self.lane_segments.len() as u64)?;
        for [a, b] in &self.lane_segments {
            put_floats(out, a)?;
            put_floats(out, b)?;
        }
        put_u64(out, self.lane_signals.len() as u64)?;
        (self.lane_signals.iter()).try_for_each(|p| put_floats(out, p))
    }

    /// Read what [`Filled::write`] wrote, refused where a map's rows are not rows of the index or
    /// a row's level of detail is none.
    pub(super) fn read(from: &mut impl Read) -> io::Result<Self> {
        let mut filled = Self::default();
        filled.index.maps = list(from, |from| {
            Ok(IndexedMap {
                offset: get_u64(from)?,
                name: u32::from_le_bytes(get(from)?),
                parent: u32::from_le_bytes(get(from)?),
                streaming_extents: Aabb {
                    min: get_floats(from)?,
                    max: get_floats(from)?,
                },
                rows: get_u64(from)? as usize..get_u64(from)? as usize,
            })
        })?;
        filled.index.rows = list(from, |from| {
            let archetype_name = u32::from_le_bytes(get(from)?);
            let archetype_word = get_u64(from)?;
            let (position, rotation) = (get_floats(from)?, get_floats(from)?);
            let [scale_xy, scale_z, lod_dist] = get_floats(from)?;
            let lod_parent = i16::from_le_bytes(get(from)?);
            let [lod_level, interior] = get(from)?;
            if usize::from(lod_level) >= LOD_LEVELS.len() {
                return Err(not_kept("a kept row of no level of detail"));
            }
            Ok(PlacementRow {
                archetype_name,
                archetype_word,
                position,
                rotation,
                scale_xy,
                scale_z,
                lod_parent,
                lod_dist,
                lod_level,
                interior: interior != 0,
            })
        })?;
        let rows = filled.index.rows.len();
        if (filled.index.maps.iter()).any(|m| m.rows.start > m.rows.end || m.rows.end > rows) {
            return Err(not_kept("a kept map whose rows are not rows of the index"));
        }
        let boxes = list(from, |from| {
            Ok((get_u64(from)?, [get_floats(from)?, get_floats(from)?]))
        })?;
        filled.boxes = boxes.into_iter().collect();
        filled.lights = list(from, |from| {
            let [x, y, range] = get_floats(from)?;
            Ok(Light {
                at: [x, y],
                colour: get(from)?,
                range,
            })
        })?;
        filled.lane_points = list(from, get_floats)?;
        filled.lane_segments = list(from, |from| Ok([get_floats(from)?, get_floats(from)?]))?;
        filled.lane_signals = list(from, get_floats)?;
        Ok(filled)
    }
}

/// Where the fill of one archive is kept, and what the kept file starts with: this file's version,
/// then the archive's size, modification time and path.
pub(super) struct Kept {
    file: PathBuf,
    head: Vec<u8>,
}

impl Kept {
    /// Where the fill of the archive at `archive` is kept under the configuration folder `root`;
    /// `None` where the archive states no size or modification time to tell a changed one by.
    pub(super) fn of(root: &Path, archive: &Path) -> Option<Self> {
        let meta = std::fs::metadata(archive).ok()?;
        let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
        let path = archive.to_string_lossy();
        let mut head = MAGIC.to_vec();
        // Nanoseconds, which a u64 holds for some centuries yet.
        for word in [
            VERSION,
            meta.len(),
            modified.as_nanos() as u64,
            path.len() as u64,
        ] {
            head.extend(word.to_le_bytes());
        }
        head.extend(path.as_bytes());
        let name = or_io::gta6::hash::name_hash64(path.as_bytes());
        let file = root.join("gta6_world").join(format!("{name:016x}.world"));
        Some(Self { file, head })
    }

    /// `file` past its head, where it was kept of the archive as it is now.
    fn open(&self, file: &Path) -> Option<BufReader<File>> {
        let mut from = BufReader::new(File::open(file).ok()?);
        let mut head = vec![0; self.head.len()];
        from.read_exact(&mut head).ok()?;
        (head == self.head).then_some(from)
    }

    /// Write `file` whole beside itself, the head and then what `write` writes, and move it there.
    fn write(
        &self,
        file: &Path,
        write: impl FnOnce(&mut BufWriter<File>) -> io::Result<()>,
    ) -> io::Result<()> {
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let beside = file.with_extension("tmp");
        let mut out = BufWriter::new(File::create(&beside)?);
        out.write_all(&self.head)?;
        write(&mut out)?;
        out.flush()?;
        drop(out);
        std::fs::rename(beside, file)
    }

    /// The fill kept of the archive as it is now; `None` where none is, or it does not read.
    pub(super) fn read(&self) -> Option<Filled> {
        Filled::read(&mut self.open(&self.file)?)
            .inspect_err(|e| tracing::info!("{}: {e}", self.file.display()))
            .ok()
    }

    /// Keep `filled` in place of any fill kept before.
    pub(super) fn keep(&self, filled: &Filled) -> io::Result<()> {
        self.write(&self.file, |out| filled.write(out))
    }

    /// Where the archive's world bodies are kept: each one's offset, and a map's streaming box.
    fn bodies_file(&self) -> PathBuf {
        self.file.with_extension("bodies")
    }

    /// The world bodies kept of the archive as it is now; `None` where none are, or they do not
    /// read.
    pub(super) fn read_bodies(&self) -> Option<Vec<(u64, Extent)>> {
        let mut from = self.open(&self.bodies_file())?;
        let body = |from: &mut BufReader<File>| {
            let offset = get_u64(from)?;
            let extent = match get(from)? {
                [0u8] => Extent::Everywhere,
                [1] => Extent::Map(Aabb {
                    min: get_floats(from)?,
                    max: get_floats(from)?,
                }),
                _ => return Err(not_kept("a kept body of no kind")),
            };
            Ok((offset, extent))
        };
        list(&mut from, body).ok()
    }

    /// Keep `bodies`, the world bodies a pass over every body of the archive found.
    pub(super) fn keep_bodies(&self, bodies: &[(u64, Extent)]) -> io::Result<()> {
        self.write(&self.bodies_file(), |out| {
            put_u64(out, bodies.len() as u64)?;
            for (offset, extent) in bodies {
                put_u64(out, *offset)?;
                match extent {
                    Extent::Everywhere => out.write_all(&[0])?,
                    Extent::Map(streamed) => {
                        out.write_all(&[1])?;
                        put_floats(out, &streamed.min)?;
                        put_floats(out, &streamed.max)?;
                    }
                }
            }
            Ok(())
        })
    }
}

/// Read the world of `archive`: the fill kept of it where every map is asked for and one stands,
/// else its bodies at `offsets`, or those a scan of it finds; `None` once `cancel` is set. A
/// region reads only the bodies that meet it once a pass over every body has kept where they are.
fn read(
    archive: &Path,
    offsets: Option<Vec<u64>>,
    region: Option<Region>,
    cancel: &Cancel,
    progress: &Progress,
) -> Result<Option<Filled>, String> {
    // ponytail: only a fill of every map is kept whole; a region reads its own bodies each time.
    // Key the file by the region too if one region's fill is asked for often.
    let kept = crate::state::prefs::config_root().and_then(|root| Kept::of(&root, archive));
    let whole = (region.is_none()).then_some(()).and(kept.as_ref());
    if let Some(filled) = whole.and_then(Kept::read) {
        return Ok(Some(filled));
    }
    let began = Instant::now();
    let met = (region.as_ref()).zip(kept.as_ref().and_then(Kept::read_bodies));
    let met = met.map(|(region, bodies)| bodies_of(&bodies, region));
    // What is read is every body unless the kept bodies say which meet the region.
    let every = met.is_none();
    let offsets = met.or(offsets);
    let file = File::open(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    // One window for every body: the offsets rise, so bodies close together share what is read.
    let mut bodies = Rpf9BodyReader::new(&file).map_err(|e| e.to_string())?;
    let mut gathered = Gathered::default();
    match offsets {
        Some(offsets) => {
            let total = offsets.len() as u64;
            for (done, offset) in offsets.iter().enumerate() {
                if cancel.is_set() {
                    return Ok(None);
                }
                progress.reached(done as u64, total);
                gathered.body(&mut bodies, *offset, region.as_ref());
            }
        }
        None => {
            let (mut pos, mut carry) = (0, Rpf9ScanCarry::default());
            let len = file.metadata().map_err(|e| e.to_string())?.len();
            while pos < len {
                let mut scan = Rpf9Scan::new(&file, pos..pos.saturating_add(SLICE))
                    .map_err(|e| e.to_string())?
                    .carrying(carry);
                for find in scan.by_ref() {
                    if cancel.is_set() {
                        return Ok(None);
                    }
                    let find = find.map_err(|e| e.to_string())?;
                    if starts_a_resource(&find) {
                        gathered.body(&mut bodies, find.offset, region.as_ref());
                    }
                }
                let end = scan.position().min(scan.file_len());
                carry = scan.carry();
                progress.reached(end, len);
                if end <= pos {
                    break;
                }
                pos = end;
            }
        }
    }
    if let (true, Some(kept)) = (every, &kept) {
        if let Err(error) = kept.keep_bodies(gathered.bodies()) {
            tracing::warn!(%error, path = %archive.display(), "the world's bodies were not kept");
        }
    }
    let bodies = gathered.bodies().len();
    let filled = gathered.finish();
    if let Some(kept) = whole {
        if let Err(error) = kept.keep(&filled) {
            tracing::warn!(%error, path = %archive.display(), "the world read was not kept");
        }
    }
    tracing::info!(
        seconds = began.elapsed().as_secs_f64(),
        every,
        bodies,
        maps = filled.index.maps.len(),
        entities = filled.index.rows.len(),
        lights = filled.lights.len(),
        held_bytes = filled.held_bytes(),
        path = %archive.display(),
        "the world was read"
    );
    Ok(Some(filled))
}

/// How a fill ended.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ended {
    /// How many maps, entities and lights it read.
    Read {
        maps: usize,
        entities: usize,
        lights: usize,
    },
    Stopped,
    Failed(String),
}

struct Fill {
    archive: PathBuf,
    cancel: Cancel,
    progress: Progress,
    ended: Arc<Mutex<Option<Ended>>>,
    /// Whether the job list has been told how it ended.
    said: bool,
}

impl Drop for Fill {
    fn drop(&mut self) {
        self.cancel.set();
    }
}

// ponytail: one fill for the whole app, of the archive it was last started for.
static FILL: Mutex<Option<Fill>> = Mutex::new(None);

fn held() -> MutexGuard<'static, Option<Fill>> {
    FILL.lock().unwrap_or_else(|p| p.into_inner())
}

fn ended_of(fill: &Fill) -> Option<Ended> {
    fill.ended.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Read the world of `archive` on a worker, in place of any fill before; the plan draws what it
/// read once it ends.
pub(super) fn start(archive: &Path, offsets: Option<Vec<u64>>, region: Option<Region>) {
    let fill = Fill {
        archive: archive.to_path_buf(),
        cancel: Cancel::new(),
        progress: Progress::new(),
        ended: Arc::default(),
        said: false,
    };
    let (archive, cancel, progress, ended) = (
        fill.archive.clone(),
        fill.cancel.clone(),
        fill.progress.clone(),
        Arc::clone(&fill.ended),
    );
    std::thread::spawn(move || {
        let outcome = match read(&archive, offsets, region, &cancel, &progress) {
            Ok(Some(filled)) => {
                let read = Ended::Read {
                    maps: filled.index.maps.len(),
                    entities: filled.index.rows.len(),
                    lights: filled.lights.len(),
                };
                // The grids are built here, off the UI thread.
                let area = Arc::new(Area::new(archive, filled));
                super::world().area = Some(area);
                read
            }
            Ok(None) => Ended::Stopped,
            Err(why) => Ended::Failed(why),
        };
        *ended.lock().unwrap_or_else(|p| p.into_inner()) = Some(outcome);
    });
    *held() = Some(fill);
}

/// Whether a fill is running.
pub(crate) fn running() -> bool {
    held().as_ref().is_some_and(|fill| ended_of(fill).is_none())
}

pub(crate) fn stop() {
    if let Some(fill) = held().as_ref() {
        fill.cancel.set();
    }
}

/// The fill while it runs, as the job list draws it.
pub(crate) fn job() -> Option<RunningJob> {
    let held = held();
    let fill = held.as_ref().filter(|fill| ended_of(fill).is_none())?;
    Some(RunningJob {
        kind: JobKind::ArchetypeSearch,
        doing: tr!(
            "Reading the world of {archive}",
            archive = file_name(&fill.archive)
        ),
        fraction: Some(fill.progress.snapshot().fraction()),
        detail: String::new(),
        stopping: fill.cancel.is_set(),
    })
}

/// How the fill ended, as the job list says it, once.
pub(crate) fn ended() -> Option<(Outcome, String)> {
    let mut held = held();
    let fill = held.as_mut().filter(|fill| !fill.said)?;
    let told = match ended_of(fill)? {
        Ended::Read {
            maps,
            entities,
            lights,
        } => (
            Outcome::Done(Then::Nothing),
            tr!(
                "{maps} maps, {entities} entities and {lights} lights read of {archive}",
                maps = maps,
                entities = entities,
                lights = lights,
                archive = file_name(&fill.archive)
            ),
        ),
        Ended::Stopped => (
            Outcome::Stopped(Then::Nothing),
            tr!("Stopped before it finished").to_string(),
        ),
        Ended::Failed(why) => (
            Outcome::Failed,
            tr!("The world was not read: {why}", why = why),
        ),
    };
    fill.said = true;
    Some(told)
}
