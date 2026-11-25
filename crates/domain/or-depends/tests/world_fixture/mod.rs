//! A GTA V install built by hand: maps, type files, a manifest and bounds written as the bytes the
//! game ships them in, put in archives and mounted with no game present.

#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

use or_io::container::rpf::crypto::NoOpCryptoProvider;
use or_io::container::rpf::RpfBuilder;
use or_io::decl::File as _;
use or_io::gta5::bounds::build::{self, BoundMesh};
use or_io::gta5::shared::Vec4V;
use or_io::gta5::{
    CPathNode, CPathRegion, PhBoundAny, PhBoundGeometry, PhBoundSphere, PhMaterialMgrId,
};
use or_io::meta::canonical;
use or_io::meta::instance::HashedString;
use or_io::meta::{
    build_library, write_meta_binary, BinaryFormat, MetaInstance, SchemaLibrary, Value,
};
use or_title::vfs::{build_mounted_game, DlcCutoff};
use or_title::{GtaInstall, MountedGame};

/// The members the world index and a search of it read, and no other: a file written from this
/// reads back to the tree it was written from, whatever schema snapshot the machine has.
const PSC: &str = r#"<?xml version="1.0"?>
<ParserSchema>
  <structdef type="CEntityDef">
    <string name="archetypeName" type="atHashString"/>
    <u32 name="guid"/>
    <Vector3 name="position"/>
    <Vector4 name="rotation"/>
  </structdef>
  <structdef type="CMloInstanceDef" base="CEntityDef">
    <u32 name="groupId"/>
  </structdef>
  <structdef type="CMapData">
    <string name="name" type="atHashString"/>
    <string name="parent" type="atHashString"/>
    <u32 name="flags"/>
    <u32 name="contentFlags"/>
    <Vector3 name="streamingExtentsMin"/>
    <Vector3 name="streamingExtentsMax"/>
    <Vector3 name="entitiesExtentsMin"/>
    <Vector3 name="entitiesExtentsMax"/>
    <array name="entities" type="atArray"><pointer type="CEntityDef" policy="owner"/></array>
  </structdef>
  <structdef type="CBaseArchetypeDef">
    <float name="lodDist"/>
    <Vector3 name="bbMin"/>
    <Vector3 name="bbMax"/>
    <string name="name" type="atHashString"/>
  </structdef>
  <structdef type="CMapTypes">
    <array name="archetypes" type="atArray"><pointer type="CBaseArchetypeDef" policy="owner"/></array>
    <string name="name" type="atHashString"/>
  </structdef>
  <structdef type="CMapDataGroup">
    <string name="Name" type="atHashString"/>
    <array name="Bounds" type="atArray"><string type="atHashString"/></array>
  </structdef>
  <structdef type="CInteriorBoundsFiles">
    <string name="Name" type="atHashString"/>
    <array name="Bounds" type="atArray"><string type="atHashString"/></array>
  </structdef>
  <structdef type="CPackFileMetaData">
    <array name="MapDataGroups" type="atArray"><struct type="CMapDataGroup"/></array>
    <array name="Interiors" type="atArray"><struct type="CInteriorBoundsFiles"/></array>
  </structdef>
</ParserSchema>"#;

fn library() -> SchemaLibrary {
    build_library([("world_fixture.psc", PSC.as_bytes())]).expect("the schema parses")
}

fn encoded(root: &MetaInstance, format: BinaryFormat) -> Vec<u8> {
    let layout = canonical::layout_for(&library(), root, format, None).expect("lays out");
    write_meta_binary(format, Some(&layout), root).expect("encodes")
}

pub fn hash(name: &str) -> u32 {
    or_io::string_hash(name.as_bytes())
}

/// A box, as `(min, max)`.
pub type Span = ([f32; 3], [f32; 3]);

/// A box `side` metres wide whose lowest corner is at `corner`.
pub fn cube(corner: [f32; 3], side: f32) -> Span {
    (corner, corner.map(|v| v + side))
}

/// What a map of the fixture states.
#[derive(Clone)]
pub struct MapSpec {
    pub parent: &'static str,
    pub flags: u32,
    pub content_flags: u32,
    pub streaming: Span,
    pub entities: Span,
    /// The archetype of each interior it places.
    pub interiors: Vec<&'static str>,
}

impl MapSpec {
    /// A map with no parent whose entities stand in `entities` and that streams from ten metres
    /// around them.
    pub fn around(entities: Span) -> Self {
        Self {
            parent: "",
            flags: 0,
            content_flags: 0,
            streaming: (entities.0.map(|v| v - 10.0), entities.1.map(|v| v + 10.0)),
            entities,
            interiors: Vec::new(),
        }
    }
}

/// One entity a map of the fixture places.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub archetype: String,
    pub position: [f32; 3],
    /// The turn about the vertical axis that stands it, in degrees.
    pub heading: f32,
    pub guid: u32,
}

impl Placed {
    pub fn new(archetype: &str, position: [f32; 3], heading: f32, guid: u32) -> Self {
        Self {
            archetype: archetype.to_string(),
            position,
            heading,
            guid,
        }
    }
}

/// A `.ymap`'s bytes: a `CMapData` named `name`, as an RSC7 resource.
pub fn map_bytes(name: &str, spec: &MapSpec) -> Vec<u8> {
    let placed = Placed::new("prop_a", spec.entities.0, 0.0, 0);
    placing_map_bytes(name, spec, &[placed])
}

/// A `.ymap`'s bytes: a `CMapData` named `name` whose entities are `placed`, in that order, and then its interiors.
pub fn placing_map_bytes(name: &str, spec: &MapSpec, placed: &[Placed]) -> Vec<u8> {
    let mut entities: Vec<Value> = placed
        .iter()
        .map(|placed| {
            let turn = glam::Quat::from_rotation_z(placed.heading.to_radians());
            let mut entity = MetaInstance::new("CEntityDef");
            entity.set(
                "archetypeName",
                Value::Hash(HashedString::new(placed.archetype.as_str())),
            );
            entity.set("guid", Value::UInt(u64::from(placed.guid)));
            entity.set("position", Value::Vec3(placed.position));
            entity.set(
                "rotation",
                Value::Vec4(or_world::Placement::Entity.store(turn)),
            );
            Value::Pointer(Some(Box::new(entity)))
        })
        .collect();
    for interior in &spec.interiors {
        let mut placed = MetaInstance::new("CMloInstanceDef");
        placed.set("archetypeName", Value::Hash(HashedString::new(*interior)));
        placed.set("guid", Value::UInt(0));
        placed.set("position", Value::Vec3(spec.entities.0));
        placed.set("rotation", Value::Vec4([0.0, 0.0, 0.0, 1.0]));
        placed.set("groupId", Value::UInt(0));
        entities.push(Value::Pointer(Some(Box::new(placed))));
    }
    let mut root = MetaInstance::new("CMapData");
    root.set("name", Value::Hash(HashedString::new(name)));
    root.set("parent", Value::Hash(HashedString::new(spec.parent)));
    root.set("flags", Value::UInt(u64::from(spec.flags)));
    root.set("contentFlags", Value::UInt(u64::from(spec.content_flags)));
    root.set("streamingExtentsMin", Value::Vec3(spec.streaming.0));
    root.set("streamingExtentsMax", Value::Vec3(spec.streaming.1));
    root.set("entitiesExtentsMin", Value::Vec3(spec.entities.0));
    root.set("entitiesExtentsMax", Value::Vec3(spec.entities.1));
    root.set("entities", Value::Array(entities));
    encoded(&root, BinaryFormat::Rbm)
}

/// A `.ytyp`'s bytes: a `CMapTypes` named `name` declaring `(archetype, box, lodDist)`.
pub fn types_bytes(name: &str, archetypes: &[(&str, Span, f32)]) -> Vec<u8> {
    let items = archetypes
        .iter()
        .map(|(archetype, (min, max), lod_dist)| {
            let mut arch = MetaInstance::new("CBaseArchetypeDef");
            arch.set("lodDist", Value::Float(f64::from(*lod_dist)));
            arch.set("bbMin", Value::Vec3(*min));
            arch.set("bbMax", Value::Vec3(*max));
            arch.set("name", Value::Hash(HashedString::new(*archetype)));
            Value::Pointer(Some(Box::new(arch)))
        })
        .collect();
    let mut root = MetaInstance::new("CMapTypes");
    root.set("archetypes", Value::Array(items));
    root.set("name", Value::Hash(HashedString::new(name)));
    encoded(&root, BinaryFormat::Rbm)
}

/// A `_manifest.ymf`'s bytes: the bounds each map group and each interior archetype is named.
pub fn manifest_bytes(groups: &[(&str, &[&str])], interiors: &[(&str, &[&str])]) -> Vec<u8> {
    let items = |type_name: &str, named: &[(&str, &[&str])]| {
        let items = named.iter().map(|(name, bounds)| {
            let mut item = MetaInstance::new(type_name);
            item.set("Name", Value::Hash(HashedString::new(*name)));
            let bounds = bounds.iter().map(|b| Value::Hash(HashedString::new(*b)));
            item.set("Bounds", Value::Array(bounds.collect()));
            Value::Struct(Box::new(item))
        });
        Value::Array(items.collect())
    };
    let mut root = MetaInstance::new("CPackFileMetaData");
    root.set("MapDataGroups", items("CMapDataGroup", groups));
    root.set("Interiors", items("CInteriorBoundsFiles", interiors));
    encoded(&root, BinaryFormat::Pso)
}

/// A `.ybn`'s bytes: a bound whose header box is `span`.
pub fn bound_bytes(span: Span) -> Vec<u8> {
    sphere_bytes(span, 0.0)
}

/// A `.ybn`'s bytes: a sphere whose header states the box `span` and the radius `radius`.
pub fn sphere_bytes(span: Span, radius: f32) -> Vec<u8> {
    let (min, max) = span;
    let mut sphere = PhBoundSphere::default();
    sphere.base.radius_around_centroid = radius;
    sphere.base.bounding_box_max_xyz_margin_w = Vec4V {
        x: max[0],
        y: max[1],
        z: max[2],
        ..Default::default()
    };
    sphere.base.set_bounding_box_min(min);
    PhBoundAny::PhBoundSphere(sphere)
        .to_bytes()
        .expect("a bound")
}

/// A geometry of `triangles` over `positions`, each with the material `materials` lists for it by index.
pub fn mesh_bound(
    positions: &[[f32; 3]],
    triangles: &[[u16; 3]],
    materials: &[u64],
    triangle_materials: &[u8],
) -> PhBoundGeometry {
    let mesh = BoundMesh {
        positions: positions.to_vec(),
        triangles: triangles.to_vec(),
        materials: (materials.iter())
            .map(|id| PhMaterialMgrId {
                id: *id,
                ..Default::default()
            })
            .collect(),
        triangle_materials: triangle_materials.to_vec(),
    };
    build::geometry(&mesh).expect("a mesh a bound can hold")
}

/// A level square of two triangles from `corner`, `side` metres wide, of one material.
pub fn level_square(corner: [f32; 3], side: f32, material: u64) -> PhBoundGeometry {
    let [x, y, z] = corner;
    mesh_bound(
        &[
            [x, y, z],
            [x + side, y, z],
            [x + side, y + side, z],
            [x, y + side, z],
        ],
        &[[0, 1, 2], [0, 2, 3]],
        &[material],
        &[0, 0],
    )
}

/// A `.ybn`'s bytes: `geometry` as the file's one bound.
pub fn mesh_bound_bytes(geometry: PhBoundGeometry) -> Vec<u8> {
    PhBoundAny::PhBoundGeometry(geometry)
        .to_bytes()
        .expect("a bound")
}

/// An archive holding `files`; a path through `x.rpf/` puts the rest of it in an archive nested there.
fn archive_bytes(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut builder = RpfBuilder::default();
    let mut nested: std::collections::BTreeMap<&str, Vec<(&str, Vec<u8>)>> = Default::default();
    for (path, bytes) in files {
        match path.split_once(".rpf/") {
            Some((outer, inner)) => nested
                .entry(outer)
                .or_default()
                .push((inner, bytes.clone())),
            None => {
                builder
                    .insert_with(path, bytes.clone(), false)
                    .unwrap_or_else(|e| panic!("add {path}: {e}"));
            }
        }
    }
    for (outer, inner) in nested {
        builder
            .insert_with(&format!("{outer}.rpf"), archive_bytes(&inner), false)
            .unwrap_or_else(|e| panic!("add {outer}.rpf: {e}"));
    }
    builder.to_bytes().expect("an archive")
}

/// Write the archive `name` of the install at `root`, holding `files`.
pub fn write_archive(root: &Path, name: &str, files: &[(&str, Vec<u8>)]) {
    std::fs::write(root.join(name), archive_bytes(files)).expect("save");
}

/// The install at `root`, mounted with every pack its DLC list names.
pub fn mount_all(root: &Path) -> MountedGame {
    let install = GtaInstall {
        root: root.to_path_buf(),
    };
    build_mounted_game(
        &install,
        Arc::new(NoOpCryptoProvider::new()),
        DlcCutoff::All,
    )
    .expect("the synthetic install mounts")
}

/// The install at `root`, mounted with no DLC.
pub fn mount(root: &Path) -> MountedGame {
    build_mounted_game(
        &GtaInstall {
            root: root.to_path_buf(),
        },
        Arc::new(NoOpCryptoProvider::new()),
        DlcCutoff::BaseGame,
    )
    .expect("the synthetic install mounts")
}

/// A source that counts what an index asks of it, by root archive.
pub struct Counting<'a> {
    pub source: &'a dyn or_depends::WorldSource,
    /// How often each archive's entries were listed.
    pub listed: std::cell::RefCell<Vec<u32>>,
    /// How many files of each archive were opened, whole or by their front.
    pub opened: std::cell::RefCell<Vec<u32>>,
}

impl<'a> Counting<'a> {
    pub fn new(source: &'a dyn or_depends::WorldSource) -> Self {
        let archives = source.archives().len();
        Self {
            source,
            listed: vec![0; archives].into(),
            opened: vec![0; archives].into(),
        }
    }

    /// `(listed, opened)`, each by root archive in mount order.
    pub fn counts(&self) -> (Vec<u32>, Vec<u32>) {
        (self.listed.borrow().clone(), self.opened.borrow().clone())
    }
}

impl or_depends::WorldSource for Counting<'_> {
    fn identity(&self) -> String {
        self.source.identity()
    }

    fn archives(&self) -> Vec<or_depends::world_index::Archive> {
        self.source.archives()
    }

    fn entries(&self, archive: usize, visit: &mut dyn FnMut(&or_title::vfs::EntryFacts)) {
        self.listed.borrow_mut()[archive] += 1;
        self.source.entries(archive, visit);
    }

    fn open(&self, archive: usize, logical: &str) -> Option<Vec<u8>> {
        self.opened.borrow_mut()[archive] += 1;
        self.source.open(archive, logical)
    }

    fn open_head(&self, archive: usize, logical: &str, limit: usize) -> Option<Vec<u8>> {
        self.opened.borrow_mut()[archive] += 1;
        self.source.open_head(archive, logical, limit)
    }

    fn patch_of(&self, archive: usize) -> Option<String> {
        self.source.patch_of(archive)
    }
}

/// The copy the mount would load of each file of `extension`: its lower-case stem, and the last
/// logical path the mount lists it under, by a walk of its root archives in their order.
///
/// ⚠ A file of a folder that patches a root archive lies in the title update, which is walked before every pack, and is stacked
/// over that archive on its device: it is taken after that archive's own files and not where the update is walked.
fn winners(game: &MountedGame, extension: &str) -> std::collections::BTreeMap<String, String> {
    use std::collections::BTreeMap;
    let folders: Vec<String> = (game.roots.iter())
        .filter_map(|root| Some(root.patch.as_ref()?.to_ascii_lowercase()))
        .collect();
    let patching = |logical: &str| -> Option<&String> {
        let lower = logical.to_ascii_lowercase();
        folders
            .iter()
            .find(|folder| lower.starts_with(folder.as_str()))
    };
    let named = |facts: &or_title::vfs::EntryFacts| -> Option<(String, String)> {
        let leaf = facts.leaf.to_ascii_lowercase();
        Some((
            leaf.strip_suffix(extension)?.to_string(),
            facts.logical.clone(),
        ))
    };
    let mut in_a_patch: BTreeMap<&String, Vec<(String, String)>> = BTreeMap::new();
    game.for_each_entry(|facts| {
        if let (Some(folder), Some(file)) = (patching(&facts.logical), named(facts)) {
            in_a_patch.entry(folder).or_default().push(file);
        }
    });
    let mut winners = BTreeMap::new();
    for root in &game.roots {
        root.for_each_entry(|facts| {
            if let (None, Some((stem, logical))) = (patching(&facts.logical), named(facts)) {
                winners.insert(stem, logical);
            }
        });
        let folder = root.patch.as_ref().map(|patch| patch.to_ascii_lowercase());
        let patched = folder.as_ref().and_then(|folder| in_a_patch.get(folder));
        winners.extend(patched.into_iter().flatten().cloned());
    }
    winners
}

/// Hold `named`, what the index names of one kind by stem, to `listed`, what the mount lists, each difference on a line.
#[track_caller]
fn same_files(
    kind: &str,
    named: std::collections::BTreeMap<String, String>,
    listed: &std::collections::BTreeMap<String, String>,
) {
    let stems: std::collections::BTreeSet<&String> = named.keys().chain(listed.keys()).collect();
    let differing: Vec<String> = (stems.into_iter())
        .filter(|stem| named.get(*stem) != listed.get(*stem))
        .map(|stem| {
            format!(
                "{stem}: the index names {:?}, the mount lists {:?}",
                named.get(stem),
                listed.get(stem)
            )
        })
        .collect();
    assert!(
        differing.is_empty(),
        "{} {kind} of {} differ, the first:\n{}",
        differing.len(),
        listed.len(),
        differing[..differing.len().min(12)].join("\n")
    );
}

/// What [`held_to_its_mount`] compared, so a caller can say an empty install agreed with nothing.
#[derive(Debug, PartialEq, Eq)]
pub struct Held {
    pub maps: usize,
    pub type_files: usize,
    pub bounds: usize,
    /// Maps asked at the centre of their streaming extents.
    pub maps_asked: usize,
    /// Carried bounds asked at the centre of their box.
    pub bounds_asked: usize,
}

/// Hold the index of the install at `root`, kept in `file`, to everything a mount of it lists: once
/// built with no kept file, and once from the file by a mount made anew.
pub fn held_to_its_mount(mount: &dyn Fn() -> MountedGame, root: &Path, file: &Path) -> Held {
    use or_depends::world_index::{refresh, KeptState};
    use or_depends::WorldSource as _;
    use std::collections::BTreeMap;

    let game = mount();
    let source = or_depends::MountedWorld::new(&game, root);
    let counting = Counting::new(&source);
    let (index, report) = refresh(&counting, file);
    assert!(
        matches!(report.kept, KeptState::Absent),
        "{:?}",
        report.kept
    );
    assert_eq!(
        report.read.len(),
        report.archives,
        "every archive is read the first time"
    );
    assert_eq!(report.archives, game.roots.len());
    assert!(report.not_kept.is_none(), "{:?}", report.not_kept);

    // Every map, type file and bound the mount lists, at the copy it would load.
    let named = |all: Vec<(&str, &str)>| -> BTreeMap<String, String> {
        (all.into_iter())
            .map(|(name, logical)| (name.to_string(), logical.to_string()))
            .collect()
    };
    let maps = (index.maps().iter()).map(|m| (m.name.as_str(), m.logical.as_str()));
    let types = (index.type_files().iter()).map(|f| (f.name.as_str(), f.logical.as_str()));
    let bounds = (index.bounds().iter()).map(|b| (b.name.as_str(), b.logical.as_str()));
    let (listed_maps, listed_types) = (winners(&game, ".ymap"), winners(&game, ".ytyp"));
    assert!(
        !listed_maps.is_empty() && !listed_types.is_empty(),
        "an install with no world"
    );
    same_files("map(s)", named(maps.collect()), &listed_maps);
    same_files("type file(s)", named(types.collect()), &listed_types);
    same_files("bound(s)", named(bounds.collect()), &winners(&game, ".ybn"));

    // What the index reads of a file's own archive is what the mount opens at the path the index names.
    let maps = (index.maps().iter()).map(|m| (m.archive, m.logical.as_str()));
    let types = (index.type_files().iter()).map(|f| (f.archive, f.logical.as_str()));
    let bounds = (index.bounds().iter()).map(|b| (b.archive, b.logical.as_str()));
    let (mut both_ways, mut patched) = (0usize, 0usize);
    for (archive, logical) in maps.chain(types).chain(bounds) {
        let by_archive = source.open(archive, logical);
        assert!(
            by_archive.is_some() && by_archive == game.vfs.open(logical),
            "{logical} is one file in archive {archive} and another through the mount"
        );
        both_ways += 1;
        let under = |patch: &String| {
            (logical.get(..patch.len())).is_some_and(|front| front.eq_ignore_ascii_case(patch))
        };
        let patches = game.roots.iter().filter_map(|root| root.patch.as_ref());
        patched += usize::from(patches.into_iter().any(under));
    }
    eprintln!("\n=== {both_ways} file(s) read both ways, {patched} of them in a folder that patches a pack ===");

    // A map whose extents are a box answers at their centre, and a carried bound at its own.
    let is_box = |(min, max): (glam::Vec3, glam::Vec3)| {
        min.is_finite() && max.is_finite() && min.cmple(max).all()
    };
    let (mut read, mut maps_asked, mut bounds_asked) = (0usize, 0usize, 0usize);
    for map in index.maps() {
        read += usize::from(map.read);
        let Some(extents) = map.streaming.filter(|extents| is_box(*extents)) else {
            continue;
        };
        let centre = (extents.0 + extents.1) * 0.5;
        let answered = index.maps_at(centre);
        assert!(
            answered.iter().any(|other| other.hash == map.hash),
            "{} is not answered at the centre of its streaming extents, {centre}",
            map.name
        );
        maps_asked += 1;
    }
    for map in index.maps() {
        for carried in index.bounds_of_map(map.hash) {
            let Some(bound) = carried.bound else { continue };
            let Some(span) = bound.span.filter(|span| is_box(*span)) else {
                continue;
            };
            let centre = (span.0 + span.1) * 0.5;
            assert!(
                (index.bounds_at(centre).iter()).any(|found| found.bound.hash == bound.hash),
                "{} is not answered at the centre of its box, {centre}",
                bound.name
            );
            bounds_asked += 1;
        }
    }
    assert!(
        read > 0 && maps_asked > 0,
        "{read} map(s) read, {maps_asked} asked at their centre"
    );
    assert!(index.archetype_count() > 0, "no archetype was read");
    for archetype in index.archetypes() {
        let file = index.type_file(archetype.type_file);
        assert!(
            file.is_some_and(|file| file.read),
            "{:#010X} is declared in no type file the index names",
            archetype.name
        );
    }
    eprintln!(
        "\n=== {} archive(s), {} file(s) opened; {} map(s), {read} read, {maps_asked} answered at \
         their centre; {} type file(s), {} archetype(s); {} bound(s), {} carried, {bounds_asked} \
         answered at their centre; {} cop(ies) shadowed ===",
        report.archives,
        report.opened,
        index.maps().len(),
        index.type_files().len(),
        index.archetype_count(),
        index.bounds().len(),
        index.carried_bounds(),
        index.shadowed()
    );

    // A second start reads the kept index and opens nothing.
    let game = mount();
    let source = or_depends::MountedWorld::new(&game, root);
    let counting = Counting::new(&source);
    let (again, report) = refresh(&counting, file);
    assert!(
        matches!(report.kept, KeptState::Current),
        "{:?}: read again {:?}",
        report.kept,
        report.read
    );
    let (listed, opened) = counting.counts();
    assert!(
        listed.iter().chain(&opened).all(|count| *count == 0),
        "{listed:?} {opened:?}"
    );
    assert_eq!(again.maps(), index.maps());
    assert_eq!(again.type_files(), index.type_files());
    assert_eq!(again.bounds(), index.bounds());
    assert_eq!(again.archetype_count(), index.archetype_count());
    Held {
        maps: index.maps().len(),
        type_files: index.type_files().len(),
        bounds: index.bounds().len(),
        maps_asked,
        bounds_asked,
    }
}

/// What [`models_held_to_their_maps`] compared, so a caller can say a mount with no model agreed with nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelsHeld {
    /// The archetypes a file of the mount names and an entity of a map places.
    pub named: usize,
    /// Those asked for by name.
    pub asked: usize,
    /// The placements held to the entity of their map file.
    pub placements: usize,
    /// Those of them that lie inside the streaming extents of their map.
    pub inside: usize,
}

/// Find up to `handful` models of `source` by their own names, spread over the names its files state, and hold each placement listed to the entity at that index of the map file it names, read again here.
pub fn models_held_to_their_maps(
    source: &dyn or_depends::WorldSource,
    handful: usize,
) -> ModelsHeld {
    use or_depends::find::{
        find, names_of, place_maps, placements_of, Asked, Budget, Detail, Install, Rank,
        SearchNames,
    };

    let (index, _) = or_depends::WorldIndex::build(source, None);
    let (catalogue, names) = names_of(source);
    let search = SearchNames::new(&index, &names);
    let mut placements = or_depends::PlacementIndex::new();
    place_maps(&index, source, &mut placements);
    let mut named: Vec<(String, u32)> = (index.archetypes())
        .filter(|archetype| {
            let mut placed = placements.of(archetype.name).iter();
            placed.any(|placement| placement.placed != or_depends::Placed::GrassBatch)
        })
        .filter_map(|archetype| {
            let flavour = or_io::names::HashFlavour::CaseFolding;
            let name = names.resolve_with(flavour, archetype.name)?;
            Some((name.to_ascii_lowercase(), archetype.name))
        })
        .collect();
    named.sort();
    let step = (named.len() / handful.max(1)).max(1);
    let picked: Vec<&(String, u32)> = named.iter().step_by(step).take(handful).collect();

    let asked = Asked {
        near: None,
        limit: 4,
    };
    let budget = Budget {
        placements: 16,
        bytes: 256 << 20,
    };
    let install = Install {
        world: &index,
        catalogue: &catalogue,
        names: &names,
        search: &search,
        placements: &placements,
    };
    let mut held = ModelsHeld {
        named: named.len(),
        asked: picked.len(),
        placements: 0,
        inside: 0,
    };
    for (name, hash) in picked {
        let found = find(name, &install, &asked, None).expect("a name is text");
        let model = found.models.first();
        let model = model.unwrap_or_else(|| panic!("{name} is not found by its name"));
        assert_eq!(
            (model.name.as_deref(), model.hash, model.rank),
            (Some(name.as_str()), *hash, Rank::Exact)
        );
        assert_eq!(model.placements, placements.of(*hash).len(), "{name}");
        let declared = index.archetype(*hash).expect("an archetype");
        let file = index.type_file(declared.type_file).expect("its type file");
        let said = model.type_file.as_ref().map(|file| &file.logical);
        assert_eq!(said, Some(&file.logical), "{name}");

        let listed = placements_of(*hash, &install, source, None, &budget);
        assert!(
            !listed.listed.is_empty(),
            "{name} is placed and lists nothing"
        );
        for placement in &listed.listed {
            let Detail::Entity {
                index: at, guid, ..
            } = placement.detail
            else {
                continue;
            };
            let map = index.map(placement.map).expect("the map a placement names");
            assert_eq!(placement.logical.as_deref(), Some(map.logical.as_str()));
            let bytes = source.open(map.archive, &map.logical).expect("the file");
            let mut diags = or_io::meta::Diagnostics::new(or_io::meta::ErrorMode::Lenient);
            let library = or_io::meta::default_library();
            let (tree, _) =
                or_io::meta::binary::parse_meta_binary_full(library, &bytes, &mut diags)
                    .expect("the map reads");
            let data = or_world::MapData::new(&tree).expect("a map");
            // The index is a place in the array as the file holds it, where a pointer to nothing has one too.
            let array = or_world::field::array_at(data.instance(), or_world::keys::ENTITIES);
            let entity = array.get(at).and_then(or_world::field::as_struct);
            let entity = entity.map(or_world::Entity::new);
            let entity = entity.unwrap_or_else(|| panic!("{} has no entity {at}", map.logical));
            let which = format!("{name} in {} at {at}", map.logical);
            assert_eq!(entity.archetype(), *hash, "{which}");
            assert_eq!(Some(entity.position()), placement.position, "{which}");
            assert_eq!(entity.guid(), guid, "{which}");
            held.placements += 1;
            let position = entity.position();
            held.inside +=
                usize::from(map.streaming.is_some_and(|(min, max)| {
                    position.cmpge(min).all() && position.cmple(max).all()
                }));
        }
    }
    held
}

/// A `.ynd`'s bytes: a region of `car` vehicle nodes and then `ped` pedestrian ones, each at the
/// three integers a node stores for its place.
pub fn path_region_bytes(car: &[[i16; 3]], ped: &[[i16; 3]]) -> Vec<u8> {
    let node = |[x, y, z]: &[i16; 3]| CPathNode {
        coors_x: *x,
        coors_y: *y,
        as_integer1: u32::from(*z as u16) << 16,
        ..Default::default()
    };
    CPathRegion {
        nodes: car.iter().chain(ped).map(node).collect(),
        num_nodes_car_nodes: car.len() as i32,
        num_nodes_ped_nodes: ped.len() as i32,
        ..Default::default()
    }
    .to_bytes()
    .expect("a path region")
}

/// How many of a path node's stored units make a metre, across the ground and in height.
///
/// ⚠ Nothing in a `.ynd` states either: a node holds integers. Each pair is a reading a suite
/// holds to the ground, the first the one asked to hold and the others its rivals.
pub const NODE_SCALES: [(f32, f32); 5] = [
    (4.0, 32.0),
    (8.0, 32.0),
    (2.0, 32.0),
    (4.0, 16.0),
    (4.0, 64.0),
];

/// How far over or under a road node its ground may be and still be the road, in metres.
pub const STEP: f32 = 1.0;

/// How the ground of a mount stands to the vehicle nodes of its own path regions, under one
/// reading of what a node stores.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NodesHeld {
    /// The `.ynd` files the mount would load, and how many of them did not read.
    pub regions: usize,
    pub unread: usize,
    /// The vehicle nodes asked.
    pub asked: usize,
    /// Those with no static bounds file over or under them.
    pub no_file: usize,
    /// Those with ground within [`STEP`] of them.
    pub within_a_step: usize,
    /// The bounds files that gave no collider, each counted once a node that asked for it.
    pub unread_bounds: usize,
}

/// Ask the ground at every `every`th vehicle node of each path region of `game`, reading a node's
/// place at `scale` units a metre, over the static collision `index` names.
pub fn ground_at_the_road_nodes(
    game: &MountedGame,
    index: &or_depends::WorldIndex,
    source: &dyn or_depends::WorldSource,
    scale: (f32, f32),
    every: usize,
) -> NodesHeld {
    use or_world::collision::GroundProbe;

    let mut world = or_depends::WorldCollision::new(index, source, 512 << 20);
    // A step up and a step down: ground the probe meets is ground within a step of the node.
    let probe = GroundProbe {
        max_rise: STEP,
        max_drop: STEP,
        max_slope: 60f32.to_radians(),
    };
    let mut held = NodesHeld::default();
    for logical in winners(game, ".ynd").values() {
        held.regions += 1;
        let region = (game.vfs.open(logical)).and_then(|file| CPathRegion::from_bytes(&file).ok());
        let Some(region) = region else {
            held.unread += 1;
            continue;
        };
        let car = usize::try_from(region.num_nodes_car_nodes).unwrap_or(0);
        for node in region.nodes.iter().take(car).step_by(every.max(1)) {
            let at = glam::Vec3::new(
                f32::from(node.coors_x) / scale.0,
                f32::from(node.coors_y) / scale.0,
                f32::from(node.coors_z()) / scale.1,
            );
            let answer = world.ground(at, &probe);
            held.asked += 1;
            held.no_file += usize::from(answer.streamed.held.is_empty());
            held.within_a_step += usize::from(answer.ground.is_some());
            held.unread_bounds += answer.streamed.unread.len();
        }
    }
    held
}
