//! The world of a mounted GTA V install, by name and by position.
//!
//! Every map, type file, archetype and static bound the mount lists, each read from its own file
//! and held a root archive at a time, so [`refresh`] can keep the index on disk and a later start
//! reads again only the archives whose file changed.
//!
//! ⚠ Every copy an archive holds is read, not only the one the mount would load: which copy wins is
//! settled when the archives are put together, last in mount order as [`crate::Catalogue`] has it,
//! so an archive that changes cannot leave the index without the copy it used to shadow.
//!
//! ⚠ A file in the folder that patches a pack ([`WorldSource::patch_of`]) lies in the title update,
//! which mounts before every pack, and is the copy the mount opens at the pack's own path: it takes
//! its place in the order right after the pack's archive, and keeps the logical path it is listed at.

mod kept;
mod mounted;
pub(crate) mod tree;

use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};

use glam::Vec3;
use or_io::meta::binary::parse_meta_binary_full;
use or_io::meta::{Diagnostics, ErrorMode, MetaInstance};
use or_title::vfs::{EntryFacts, Vfs};
use or_world::{MapData, MapTypes, StateGroups};

use crate::catalogue::{map_of_bound_stem, Kind};
use crate::contents::ModelContents;
use crate::resolve::Via;
use crate::tables::{stated_bounds, InstallTables, NamedBounds};

pub use kept::{refresh, Kept, KeptError, RULES, VERSION};
pub use mounted::MountedWorld;
use tree::BoxTree;

/// A box, as `(min, max)`.
pub type Span = (Vec3, Vec3);

/// What tells a changed archive from the one read before: the file on disk that holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    /// Bytes.
    pub size: u64,
    /// Nanoseconds since the Unix epoch.
    pub modified: u64,
}

/// One root archive of a mount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archive {
    /// The device its files answer to, without the colon.
    pub device: String,
    /// Where it sits, relative to the install.
    pub container: String,
    /// `None` where no file on disk could be asked, and such an archive is read at every build.
    pub stamp: Option<Stamp>,
}

/// What an index reads a mount through: its root archives, their entries and their files.
pub trait WorldSource {
    /// What tells this mount from another install's, as the kept index states it.
    fn identity(&self) -> String;

    /// Each root archive, in mount order.
    fn archives(&self) -> Vec<Archive>;

    /// Every file of root archive `archive`, its nested archives descended, in its own order.
    fn entries(&self, archive: usize, visit: &mut dyn FnMut(&EntryFacts));

    /// The whole of the file `logical` that root archive `archive` holds.
    fn open(&self, archive: usize, logical: &str) -> Option<Vec<u8>>;

    /// The front of it, of at most `limit` bytes where the archive can read less than the whole.
    fn open_head(&self, archive: usize, logical: &str, limit: usize) -> Option<Vec<u8>>;

    /// The folder whose files the mount stacks over root archive `archive`'s on its device, as the logical path they start with (`update:/dlc_patch/pack/`); none where nothing patches it.
    fn patch_of(&self, _archive: usize) -> Option<String> {
        None
    }
}

/// What a `.ymap` states of itself; `None` in its row where the file did not read as a map.
#[derive(Debug, Clone, PartialEq)]
struct MapFacts {
    parent: u32,
    flags: u32,
    content_flags: u32,
    entities: Option<Span>,
    streaming: Option<Span>,
    /// The archetype of each interior the map places, each once, lowest first.
    interiors: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq)]
struct MapRow {
    logical: String,
    facts: Option<MapFacts>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ArchetypeRow {
    name: u32,
    span: Option<Span>,
    lod_dist: f32,
    asset_name: u32,
    drawable_dictionary: u32,
    texture_dictionary: u32,
}

#[derive(Debug, Clone, PartialEq)]
struct TypeFileRow {
    logical: String,
    /// `None` where the file did not read as a type file.
    archetypes: Option<Vec<ArchetypeRow>>,
}

/// What switches one map group of a manifest, as the group states it.
#[derive(Debug, Clone, PartialEq)]
struct GroupGate {
    name: u32,
    flags: u32,
    /// One bit an hour of the day, bit 0 for midnight.
    hours: u32,
    weather_types: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq)]
struct ManifestRow {
    logical: String,
    read: bool,
    groups: Vec<NamedBounds>,
    interiors: Vec<NamedBounds>,
    gates: Vec<GroupGate>,
}

#[derive(Debug, Clone, PartialEq)]
struct BoundRow {
    logical: String,
    /// `None` where no root could be read of the file.
    span: Option<Span>,
}

/// What one root archive holds of the world, each list in the archive's own order.
#[derive(Debug, Clone, PartialEq)]
struct ArchiveRows {
    archive: Archive,
    maps: Vec<MapRow>,
    type_files: Vec<TypeFileRow>,
    manifests: Vec<ManifestRow>,
    bounds: Vec<BoundRow>,
}

/// One map the mount would load.
#[derive(Debug, Clone, PartialEq)]
pub struct Map {
    /// The file's stem in lower case, which is the name the game keeps the map under.
    pub name: String,
    /// `string_hash` of [`Self::name`], which a parent and a manifest name it by.
    pub hash: u32,
    pub logical: String,
    /// The root archive that lists it, by its place in mount order, which is where [`WorldSource::open`] reads it from.
    pub archive: usize,
    /// Whether the file read as a map; every field below is empty where it did not.
    pub read: bool,
    /// The map's `parent`, zero where it has none.
    pub parent: u32,
    pub flags: u32,
    pub content_flags: u32,
    pub entities: Option<Span>,
    pub streaming: Option<Span>,
    /// The archetype of each interior the map places.
    pub interiors: Vec<u32>,
}

/// One type file the mount would load.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeFile {
    /// The file's stem in lower case.
    pub name: String,
    pub hash: u32,
    pub logical: String,
    /// The root archive that lists it, by its place in mount order, which is where [`WorldSource::open`] reads it from.
    pub archive: usize,
    /// Whether the file read as a type file.
    pub read: bool,
    /// How many archetypes it declares.
    pub archetypes: usize,
}

/// One archetype, as the last type file to declare it does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Archetype {
    pub name: u32,
    /// The [`TypeFile::hash`] of the file that declares it.
    pub type_file: u32,
    /// `bbMin` and `bbMax`.
    pub span: Option<Span>,
    /// Metres.
    pub lod_dist: f32,
    /// `assetName`: the drawable or the fragment that draws it, or its entry in [`Self::drawable_dictionary`]; zero where it states none.
    pub asset_name: u32,
    /// Zero where it states none.
    pub drawable_dictionary: u32,
    /// Zero where it states none.
    pub texture_dictionary: u32,
}

/// One `.ybn` the mount would load.
#[derive(Debug, Clone, PartialEq)]
pub struct Bound {
    /// The file's stem in lower case.
    pub name: String,
    pub hash: u32,
    pub logical: String,
    /// The root archive that lists it, by its place in mount order, which is where [`WorldSource::open`] reads it from.
    pub archive: usize,
    /// The box its header states, as the file has it.
    pub span: Option<Span>,
}

/// What carries a bound, and why the index says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Carrier {
    /// The map group or the map, or for [`Via::ManifestInterior`] the interior archetype.
    pub name: u32,
    pub via: Via,
}

/// One bound a map's collision is in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Carried<'a> {
    /// The bound's name, as the manifest or the filename gives it.
    pub name: u32,
    pub via: Via,
    /// `None` where the mount holds no such file.
    pub bound: Option<&'a Bound>,
}

/// One bound whose box holds a position, and what carries it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundAt<'a> {
    pub bound: &'a Bound,
    pub carriers: &'a [Carrier],
}

/// Whether a kept index served a build.
#[derive(Debug)]
pub enum KeptState {
    /// None was given.
    Absent,
    /// It was of this mount as it stands: nothing was read.
    Current,
    /// It was of this install with other archives: each changed one was read again.
    Changed,
    /// It was of another install, so nothing of it was taken.
    Foreign,
    /// It could not be read, and why; every archive was read.
    Refused(KeptError),
}

/// What a build took from a kept index and what it read.
#[derive(Debug)]
pub struct Report {
    pub kept: KeptState,
    /// How many root archives the mount lists.
    pub archives: usize,
    /// The container of each archive whose entries were read, in mount order.
    pub read: Vec<String>,
    /// How many files were opened to read them; a bound read whole after its front counts twice.
    pub opened: u64,
    /// Why the index was not written back, where a write was tried and failed.
    pub not_kept: Option<std::io::Error>,
}

/// Every map, type file, archetype and bound of a mount, and what lies at a position.
#[derive(Debug)]
pub struct WorldIndex {
    identity: String,
    archives: Vec<ArchiveRows>,
    maps: Vec<Map>,
    map_by_hash: HashMap<u32, usize>,
    type_files: Vec<TypeFile>,
    type_file_by_hash: HashMap<u32, usize>,
    archetypes: HashMap<u32, Archetype>,
    redefined: usize,
    bounds: Vec<Bound>,
    bound_by_hash: HashMap<u32, usize>,
    /// The bounds whose stem names each map, by the map's hash.
    stem_bounds: HashMap<u32, Vec<u32>>,
    tables: InstallTables,
    state_groups: StateGroups,
    carriers: HashMap<u32, Vec<Carrier>>,
    /// By a map's hash, the places in `bounds` of what it carries whose file states no box.
    boxless: HashMap<u32, Vec<u32>>,
    map_tree: BoxTree,
    bound_tree: BoxTree,
}

/// The lower-case leaf and stem of a logical path.
fn leaf_and_stem(logical: &str) -> (String, String) {
    let leaf = logical.rsplit('/').next().unwrap_or(logical);
    let leaf = leaf.to_ascii_lowercase();
    let stem = leaf
        .rsplit_once('.')
        .map_or(leaf.as_str(), |(stem, _)| stem);
    let stem = stem.to_string();
    (leaf, stem)
}

/// Where a file stands in the mount's order: its root archive, or for a file of a patch folder the last archive that folder
/// patches, and above that archive's own files; then the archive that lists it and the place of its row there.
type Rank = (usize, bool, usize, usize);

/// The rows `rows` picks of each archive, each with its logical path, in the mount's order.
fn ranked<'a>(
    archives: &'a [ArchiveRows],
    patches: &[(usize, String)],
    rows: impl Fn(&'a ArchiveRows) -> Vec<&'a str>,
) -> Vec<(Rank, &'a str)> {
    let mut all = Vec::new();
    for (a, archive) in archives.iter().enumerate() {
        for (r, logical) in rows(archive).into_iter().enumerate() {
            let under = |folder: &str| {
                (logical.get(..folder.len()))
                    .is_some_and(|front| front.eq_ignore_ascii_case(folder))
            };
            let patched =
                (patches.iter().filter(|(_, folder)| under(folder))).map(|(root, _)| *root);
            let rank = match patched.max() {
                Some(root) => (root, true, a, r),
                None => (a, false, a, r),
            };
            all.push((rank, logical));
        }
    }
    all.sort_unstable();
    all
}

/// The archive and the row of the last copy of each leaf of `ranked`, by the leaf in lower case.
fn winners(ranked: &[(Rank, &str)]) -> HashMap<String, (usize, usize)> {
    let last = |&((_, _, a, r), logical): &(Rank, &str)| (leaf_and_stem(logical).0, (a, r));
    ranked.iter().map(last).collect()
}

/// A metadata file as its value tree, read as the dependency tables read one.
pub(crate) fn parse(bytes: &[u8]) -> Option<MetaInstance> {
    let mut diags = Diagnostics::new(ErrorMode::Lenient);
    parse_meta_binary_full(or_io::meta::default_library(), bytes, &mut diags)
        .ok()
        .map(|(root, _)| root)
}

fn map_facts(bytes: &[u8]) -> Option<MapFacts> {
    let root = parse(bytes)?;
    let map = MapData::new(&root)?;
    let mut interiors: Vec<u32> = map
        .entities()
        .filter(|entity| entity.is_interior_instance())
        .map(|entity| entity.archetype())
        .filter(|archetype| *archetype != 0)
        .collect();
    interiors.sort_unstable();
    interiors.dedup();
    Some(MapFacts {
        parent: map.parent(),
        flags: map.flags(),
        content_flags: map.content_flags(),
        entities: map.stored_entities_extents(),
        streaming: map.stored_streaming_extents(),
        interiors,
    })
}

fn archetype_rows(bytes: &[u8]) -> Option<Vec<ArchetypeRow>> {
    let root = parse(bytes)?;
    let types = MapTypes::new(&root)?;
    let rows = types
        .archetypes()
        .filter(|archetype| archetype.name() != 0)
        .map(|archetype| ArchetypeRow {
            name: archetype.name(),
            span: archetype.bounds().map(|bounds| (bounds.min, bounds.max)),
            lod_dist: archetype.lod_dist(),
            asset_name: archetype.asset_name(),
            drawable_dictionary: archetype.drawable_dictionary(),
            texture_dictionary: archetype.texture_dictionary(),
        });
    Some(rows.collect())
}

fn manifest_row(logical: String, bytes: Option<&[u8]>) -> ManifestRow {
    let root = bytes.and_then(parse);
    let manifest = root.as_ref().and_then(or_world::Manifest::new);
    let (groups, interiors) = manifest.map(stated_bounds).unwrap_or_default();
    let gates = manifest.into_iter().flat_map(|m| m.state_groups());
    let gates = gates.map(|group| GroupGate {
        name: group.name(),
        flags: group.flags(),
        hours: group.hours_on_off(),
        weather_types: group.weather_types(),
    });
    ManifestRow {
        logical,
        read: manifest.is_some(),
        groups,
        interiors,
        gates: gates.collect(),
    }
}

/// Read what root archive `index` of `source` holds of the world, counting each file opened.
///
/// ⚠ One value tree at a time: a retail install's maps and type files do not fit in memory
/// together, so each is folded into its row and dropped before the next is opened.
fn read_archive(
    source: &dyn WorldSource,
    index: usize,
    archive: Archive,
    opened: &Cell<u64>,
) -> ArchiveRows {
    let mut wanted: Vec<(Kind, String)> = Vec::new();
    source.entries(index, &mut |facts| {
        let kind = Kind::of_leaf(&facts.leaf);
        if matches!(
            kind,
            Kind::MapData | Kind::MapTypes | Kind::Manifest | Kind::Bounds
        ) {
            wanted.push((kind, facts.logical.clone()));
        }
    });
    let unopened = Cell::new(false);
    let whole = |logical: &str| {
        opened.set(opened.get() + 1);
        let bytes = source.open(index, logical);
        unopened.set(unopened.get() || bytes.is_none());
        bytes
    };
    let mut rows = ArchiveRows {
        archive,
        maps: Vec::new(),
        type_files: Vec::new(),
        manifests: Vec::new(),
        bounds: Vec::new(),
    };
    for (kind, logical) in wanted {
        match kind {
            Kind::MapData => {
                let facts = whole(&logical).as_deref().and_then(map_facts);
                rows.maps.push(MapRow { logical, facts });
            }
            Kind::MapTypes => {
                let archetypes = whole(&logical).as_deref().and_then(archetype_rows);
                rows.type_files.push(TypeFileRow {
                    logical,
                    archetypes,
                });
            }
            Kind::Manifest => {
                let bytes = whole(&logical);
                rows.manifests.push(manifest_row(logical, bytes.as_deref()));
            }
            _ => {
                let mut read = ModelContents::new();
                read.read_bound_from(
                    0,
                    || {
                        opened.set(opened.get() + 1);
                        source.open_head(index, &logical, Vfs::BOUND_FRONT)
                    },
                    || whole(&logical),
                );
                let span = read.bound_box_of(0);
                rows.bounds.push(BoundRow { logical, span });
            }
        }
    }
    // ⚠ A file the mount lists and could not open is no fact about the file: an archive held open, a failed read, a key not yet
    // loaded. Its rows say unread all the same, so the archive is kept with no stamp and the next build reads it again.
    if unopened.get() {
        rows.archive.stamp = None;
    }
    rows
}

impl WorldIndex {
    /// The index of `source`: an archive `kept` holds as the mount now has it is taken from it, and every other is read.
    pub fn build(source: &dyn WorldSource, kept: Option<Kept>) -> (Self, Report) {
        let identity = source.identity();
        let listed = source.archives();
        let mut state = KeptState::Absent;
        // ⚠ A queue: a mount can list one archive twice, and each listing takes rows of its own.
        let mut carried: HashMap<(String, String), VecDeque<ArchiveRows>> = HashMap::new();
        if let Some(kept) = kept {
            if kept.identity != identity {
                state = KeptState::Foreign;
            } else {
                state = if kept.key == kept::key_of(&identity, listed.iter()) {
                    KeptState::Current
                } else {
                    KeptState::Changed
                };
                for rows in kept.archives {
                    let at = (rows.archive.device.clone(), rows.archive.container.clone());
                    carried.entry(at).or_default().push_back(rows);
                }
            }
        }
        let opened = Cell::new(0);
        let mut read = Vec::new();
        let mut archives = Vec::with_capacity(listed.len());
        let patches: Vec<(usize, String)> = (0..listed.len())
            .filter_map(|archive| Some((archive, source.patch_of(archive)?)))
            .collect();
        for (index, archive) in listed.into_iter().enumerate() {
            let at = (archive.device.clone(), archive.container.clone());
            // ⚠ An archive with no stamp is never taken from a kept index: nothing says it stands.
            let same = (carried.get_mut(&at).and_then(VecDeque::pop_front))
                .filter(|rows| archive.stamp.is_some() && rows.archive == archive);
            archives.push(match same {
                Some(rows) => rows,
                None => {
                    read.push(archive.container.clone());
                    read_archive(source, index, archive, &opened)
                }
            });
        }
        // ⚠ The key cannot tell an archive with no stamp from itself, and it is read all the same.
        if matches!(state, KeptState::Current) && !read.is_empty() {
            state = KeptState::Changed;
        }
        let report = Report {
            kept: state,
            archives: archives.len(),
            read,
            opened: opened.get(),
            not_kept: None,
        };
        (Self::of_rows(identity, archives, &patches), report)
    }

    /// Put the archives' rows together: the copy the mount would load of each file, the tables,
    /// what carries each bound and the two trees. `patches` names the folder that patches a root archive, by its place.
    fn of_rows(identity: String, archives: Vec<ArchiveRows>, patches: &[(usize, String)]) -> Self {
        let maps = ranked(&archives, patches, |rows| {
            rows.maps.iter().map(|row| row.logical.as_str()).collect()
        });
        let type_files = ranked(&archives, patches, |rows| {
            (rows.type_files.iter().map(|row| row.logical.as_str())).collect()
        });
        let manifests = ranked(&archives, patches, |rows| {
            (rows.manifests.iter().map(|row| row.logical.as_str())).collect()
        });
        let bounds = ranked(&archives, patches, |rows| {
            rows.bounds.iter().map(|row| row.logical.as_str()).collect()
        });
        // The last copy of a leaf wins, and a file takes its winner's place in the mount's order,
        // so a later pack's type file is the one a redefined archetype is taken from.
        let (map_winner, types_winner, bound_winner) =
            (winners(&maps), winners(&type_files), winners(&bounds));
        // ⚠ Every manifest is called the same, so a patched one replaces by its path: the path under the archive's device that a
        // file of the folder patching it answers at, in lower case.
        let mut patched_manifests: HashSet<String> = HashSet::new();
        for &((root, over, ..), logical) in &manifests {
            let folder = patches.iter().find(|(patched, _)| over && *patched == root);
            if let Some((_, folder)) = folder {
                let rest = &logical[folder.len()..];
                let under = format!("{}:/{rest}", archives[root].archive.device);
                patched_manifests.insert(under.to_ascii_lowercase());
            }
        }

        let mut index = Self {
            identity,
            archives: Vec::new(),
            maps: Vec::new(),
            map_by_hash: HashMap::new(),
            type_files: Vec::new(),
            type_file_by_hash: HashMap::new(),
            archetypes: HashMap::new(),
            redefined: 0,
            bounds: Vec::new(),
            bound_by_hash: HashMap::new(),
            stem_bounds: HashMap::new(),
            tables: InstallTables::new(),
            state_groups: StateGroups::new(),
            carriers: HashMap::new(),
            boxless: HashMap::new(),
            map_tree: BoxTree::default(),
            bound_tree: BoxTree::default(),
        };
        for &((_, _, a, r), _) in &maps {
            let row = &archives[a].maps[r];
            let (leaf, name) = leaf_and_stem(&row.logical);
            if map_winner.get(&leaf) != Some(&(a, r)) {
                continue;
            }
            let hash = or_io::string_hash(name.as_bytes());
            let facts = row.facts.clone();
            index.map_by_hash.insert(hash, index.maps.len());
            index.maps.push(Map {
                name,
                hash,
                logical: row.logical.clone(),
                archive: a,
                read: facts.is_some(),
                parent: facts.as_ref().map_or(0, |f| f.parent),
                flags: facts.as_ref().map_or(0, |f| f.flags),
                content_flags: facts.as_ref().map_or(0, |f| f.content_flags),
                entities: facts.as_ref().and_then(|f| f.entities),
                streaming: facts.as_ref().and_then(|f| f.streaming),
                interiors: facts.map(|f| f.interiors).unwrap_or_default(),
            });
        }
        for &((_, _, a, r), _) in &type_files {
            let row = &archives[a].type_files[r];
            let (leaf, name) = leaf_and_stem(&row.logical);
            if types_winner.get(&leaf) != Some(&(a, r)) {
                continue;
            }
            let hash = or_io::string_hash(name.as_bytes());
            for archetype in row.archetypes.iter().flatten() {
                let declared = Archetype {
                    name: archetype.name,
                    type_file: hash,
                    span: archetype.span,
                    lod_dist: archetype.lod_dist,
                    asset_name: archetype.asset_name,
                    drawable_dictionary: archetype.drawable_dictionary,
                    texture_dictionary: archetype.texture_dictionary,
                };
                if index.archetypes.insert(archetype.name, declared).is_some() {
                    index.redefined += 1;
                }
            }
            (index.type_file_by_hash).insert(hash, index.type_files.len());
            index.type_files.push(TypeFile {
                name,
                hash,
                logical: row.logical.clone(),
                archive: a,
                read: row.archetypes.is_some(),
                archetypes: row.archetypes.as_ref().map_or(0, Vec::len),
            });
        }
        for &((_, over, a, r), _) in &manifests {
            let row = &archives[a].manifests[r];
            if !over && patched_manifests.contains(&row.logical.to_ascii_lowercase()) {
                continue;
            }
            (index.tables).add_bounds(row.groups.clone(), row.interiors.clone());
            for gate in &row.gates {
                let bounds = row.groups.iter().find(|(name, _)| *name == gate.name);
                index.state_groups.add_group(
                    gate.name,
                    (gate.flags, gate.hours),
                    gate.weather_types.clone(),
                    bounds.map(|(_, bounds)| bounds.clone()).unwrap_or_default(),
                    &row.logical,
                );
            }
        }
        for &((_, _, a, r), _) in &bounds {
            let row = &archives[a].bounds[r];
            let (leaf, name) = leaf_and_stem(&row.logical);
            if bound_winner.get(&leaf) != Some(&(a, r)) {
                continue;
            }
            let hash = or_io::string_hash(name.as_bytes());
            if let Some(map) = map_of_bound_stem(&name) {
                let map = or_io::string_hash(map.as_bytes());
                index.stem_bounds.entry(map).or_default().push(hash);
            }
            index.bound_by_hash.insert(hash, index.bounds.len());
            index.bounds.push(Bound {
                name,
                hash,
                logical: row.logical.clone(),
                archive: a,
                span: row.span,
            });
        }
        for map in &index.maps {
            index.state_groups.add_map_flags(map.hash, map.flags);
        }
        index.archives = archives;
        index.carry();
        index
    }

    /// Say what carries each bound, and build the two trees.
    fn carry(&mut self) {
        let mut carriers: HashMap<u32, Vec<Carrier>> = HashMap::new();
        let mut carry = |bound: u32, name: u32, via: Via| {
            carriers
                .entry(bound)
                .or_default()
                .push(Carrier { name, via });
        };
        for group in self.tables.bounded_groups() {
            for &bound in self.tables.static_bounds_of(group) {
                carry(bound, group, Via::ManifestMapDataGroup);
            }
        }
        for interior in self.tables.bounded_interiors() {
            for &bound in self.tables.interior_bounds_of(interior) {
                carry(bound, interior, Via::ManifestInterior);
            }
        }
        for map in &self.maps {
            for found in self.stem_bounds_of(map) {
                carry(found.name, map.hash, found.via);
            }
        }
        for list in carriers.values_mut() {
            list.sort_by_key(|carrier| carrier.name);
        }
        self.carriers = carriers;

        self.map_tree = BoxTree::build(self.maps.iter().enumerate().filter_map(|(at, map)| {
            let (min, max) = map.streaming?;
            Some((min, max, at as u32))
        }));
        // ⚠ Only a bound a map group or a map's stem carries is asked by position. A `.ybn` that
        // is an archetype's own collision states its box about the model's origin, and one an
        // interior alone carries about the interior's: neither box is a place in the world.
        let in_the_world = |bound: &Bound| {
            let mut carriers = self.carriers.get(&bound.hash).into_iter().flatten();
            carriers.any(|by| by.via != Via::ManifestInterior)
        };
        self.bound_tree =
            BoxTree::build(self.bounds.iter().enumerate().filter_map(|(at, bound)| {
                let (min, max) = bound.span?;
                in_the_world(bound).then_some((min, max, at as u32))
            }));
        // ⚠ A carried bound with no box the tree takes is in no column by its own word, so it is
        // kept under each map that carries it and asked where that map streams.
        let mut boxless: HashMap<u32, Vec<u32>> = HashMap::new();
        for (at, bound) in self.bounds.iter().enumerate() {
            if bound
                .span
                .is_some_and(|(min, max)| tree::is_a_box(min, max))
            {
                continue;
            }
            for carrier in self.carriers.get(&bound.hash).into_iter().flatten() {
                if !matches!(carrier.via, Via::ManifestInterior) {
                    boxless.entry(carrier.name).or_default().push(at as u32);
                }
            }
        }
        self.boxless = boxless;
    }

    /// The bounds a map's own stem names, where no manifest group names any for it.
    fn stem_bounds_of<'a>(&'a self, map: &'a Map) -> impl Iterator<Item = Carried<'a>> + 'a {
        let stated = !self.tables.static_bounds_of(map.hash).is_empty();
        let named = (self.stem_bounds.get(&map.hash)).filter(|_| !stated);
        named.into_iter().flatten().map(move |&name| {
            let bound = self.bound(name);
            let inside = bound.and_then(|b| b.span).zip(map.streaming).map(
                |((min, max), (map_min, map_max))| {
                    min.cmpge(map_min).all() && max.cmple(map_max).all()
                },
            );
            Carried {
                name,
                via: Via::MapStemConvention {
                    inside_streaming_box: inside,
                },
                bound,
            }
        })
    }

    /// Every map the mount would load, in the mount's order.
    pub fn maps(&self) -> &[Map] {
        &self.maps
    }

    /// The map whose stem hashes to `name`.
    pub fn map(&self, name: u32) -> Option<&Map> {
        self.map_by_hash.get(&name).map(|at| &self.maps[*at])
    }

    /// The place in [`Self::maps`] of the map whose stem hashes to `name`.
    pub(crate) fn place_of_map(&self, name: u32) -> Option<u32> {
        self.map_by_hash.get(&name).map(|at| *at as u32)
    }

    /// The map `map` names as its parent, where the mount holds it.
    pub fn parent_of(&self, map: &Map) -> Option<&Map> {
        (map.parent != 0).then(|| self.map(map.parent)).flatten()
    }

    /// Every type file the mount would load, in the mount's order.
    pub fn type_files(&self) -> &[TypeFile] {
        &self.type_files
    }

    /// The type file whose stem hashes to `name`.
    pub fn type_file(&self, name: u32) -> Option<&TypeFile> {
        (self.type_file_by_hash.get(&name)).map(|at| &self.type_files[*at])
    }

    /// The archetype named `name`, as the last type file to declare it does.
    pub fn archetype(&self, name: u32) -> Option<&Archetype> {
        self.archetypes.get(&name)
    }

    /// Every archetype, each name once, in no order.
    pub fn archetypes(&self) -> impl Iterator<Item = &Archetype> {
        self.archetypes.values()
    }

    /// How many archetypes the type files declare, each name once.
    pub fn archetype_count(&self) -> usize {
        self.archetypes.len()
    }

    /// How often a later type file declared an archetype an earlier one had.
    pub fn redefined(&self) -> usize {
        self.redefined
    }

    /// Every `.ybn` the mount would load, in the mount's order.
    pub fn bounds(&self) -> &[Bound] {
        &self.bounds
    }

    /// The `.ybn` whose stem hashes to `name`.
    pub fn bound(&self, name: u32) -> Option<&Bound> {
        self.bound_by_hash.get(&name).map(|at| &self.bounds[*at])
    }

    /// The place in [`Self::bounds`] of the `.ybn` whose stem hashes to `name`.
    pub(crate) fn place_of_bound(&self, name: u32) -> Option<u32> {
        self.bound_by_hash.get(&name).map(|at| *at as u32)
    }

    /// How many bounds a map group, a map's stem or an interior carries.
    pub fn carried_bounds(&self) -> usize {
        self.carriers.len()
    }

    /// The manifests read, and how many of them did not read as one.
    pub fn manifests(&self) -> (usize, usize) {
        let all = self.archives.iter().flat_map(|rows| &rows.manifests);
        let (mut count, mut unread) = (0, 0);
        for manifest in all {
            count += 1;
            unread += usize::from(!manifest.read);
        }
        (count, unread)
    }

    /// The tables the manifests fill: a map group's bounds and an interior's.
    pub fn tables(&self) -> &InstallTables {
        &self.tables
    }

    /// What switches each map group of the manifests, and which maps wait to be asked for.
    // ⚠ The last manifest in mount order to state a group keeps it here, and the first keeps its
    // bounds in [`Self::tables`]: each as the module that reads it has it.
    pub fn state_groups(&self) -> &StateGroups {
        &self.state_groups
    }

    /// The bounds of the map whose stem hashes to `map`: a manifest group's, else its stem's, then its interiors'.
    pub fn bounds_of_map(&self, map: u32) -> Vec<Carried<'_>> {
        let Some(map) = self.map(map) else {
            return Vec::new();
        };
        let group = (self.tables.static_bounds_of(map.hash).iter()).map(|&name| Carried {
            name,
            via: Via::ManifestMapDataGroup,
            bound: self.bound(name),
        });
        let interiors = map.interiors.iter().flat_map(|&interior| {
            (self.tables.interior_bounds_of(interior).iter()).map(|&name| Carried {
                name,
                via: Via::ManifestInterior,
                bound: self.bound(name),
            })
        });
        group
            .chain(self.stem_bounds_of(map))
            .chain(interiors)
            .collect()
    }

    /// The maps whose streaming extents hold `at`, in the mount's order.
    pub fn maps_at(&self, at: Vec3) -> Vec<&Map> {
        let found = self.map_tree.holding(at);
        found.iter().map(|&at| &self.maps[at as usize]).collect()
    }

    /// The bounds a map group or a map's stem carries whose box holds `at`, in the mount's order.
    pub fn bounds_at(&self, at: Vec3) -> Vec<BoundAt<'_>> {
        self.carried(&self.bound_tree.holding(at))
    }

    /// The maps whose streaming extents hold `x` and `y` at any height, in the mount's order.
    pub fn maps_in_column(&self, x: f32, y: f32) -> Vec<&Map> {
        self.maps_near_column(x, y, 0.0)
    }

    /// The maps whose streaming extents come within `radius` metres of the upright line through `x` and `y`, in the mount's order.
    pub fn maps_near_column(&self, x: f32, y: f32, radius: f32) -> Vec<&Map> {
        let found = self.map_tree.in_column(x, y, radius);
        found.iter().map(|&at| &self.maps[at as usize]).collect()
    }

    /// The bounds a map group or a map's stem carries whose box holds `x` and `y` at any height, in the mount's order.
    pub fn bounds_in_column(&self, x: f32, y: f32) -> Vec<BoundAt<'_>> {
        self.bounds_near_column(x, y, 0.0)
    }

    /// The bounds a map group or a map's stem carries whose box comes within `radius` metres of the upright line through `x` and `y`, in the mount's order.
    pub fn bounds_near_column(&self, x: f32, y: f32, radius: f32) -> Vec<BoundAt<'_>> {
        self.carried(&self.bound_tree.in_column(x, y, radius))
    }

    /// The bounds whose file states no box, carried by a map whose streaming extents come within `radius` metres of the upright line through `x` and `y`, in the mount's order.
    pub fn boxless_bounds_near_column(&self, x: f32, y: f32, radius: f32) -> Vec<BoundAt<'_>> {
        let maps = self.map_tree.in_column(x, y, radius);
        let carried = |at: &u32| self.boxless.get(&self.maps[*at as usize].hash);
        let mut found: Vec<u32> = maps.iter().filter_map(carried).flatten().copied().collect();
        found.sort_unstable();
        found.dedup();
        self.carried(&found)
    }

    /// The bounds a manifest names for the interiors placed by a map whose streaming extents come within `radius` metres of the upright line through `x` and `y`, in the mount's order.
    pub fn interior_bounds_near_column(&self, x: f32, y: f32, radius: f32) -> Vec<BoundAt<'_>> {
        let maps = self.map_tree.in_column(x, y, radius);
        let interiors = maps
            .iter()
            .flat_map(|at| &self.maps[*at as usize].interiors);
        let named = interiors.flat_map(|interior| self.tables.interior_bounds_of(*interior));
        let mut found: Vec<u32> = named
            .filter_map(|name| self.place_of_bound(*name))
            .collect();
        found.sort_unstable();
        found.dedup();
        self.carried(&found)
    }

    /// Each bound of `found`, by its place in [`Self::bounds`], with what carries it.
    fn carried(&self, found: &[u32]) -> Vec<BoundAt<'_>> {
        let bound_at = |&at: &u32| {
            let bound = &self.bounds[at as usize];
            let carriers = self
                .carriers
                .get(&bound.hash)
                .map_or(&[][..], Vec::as_slice);
            BoundAt { bound, carriers }
        };
        found.iter().map(bound_at).collect()
    }

    /// The root archives the index was read from, in mount order.
    pub fn archives(&self) -> impl Iterator<Item = &Archive> {
        self.archives.iter().map(|rows| &rows.archive)
    }

    /// How many copies of a map, a type file or a bound a later archive's copy replaced.
    pub fn shadowed(&self) -> usize {
        let held: usize = (self.archives.iter())
            .map(|rows| rows.maps.len() + rows.type_files.len() + rows.bounds.len())
            .sum();
        held - self.maps.len() - self.type_files.len() - self.bounds.len()
    }
}
