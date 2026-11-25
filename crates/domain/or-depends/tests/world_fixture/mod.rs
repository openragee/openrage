//! A GTA V install built by hand: maps, type files, a manifest and bounds written as the bytes the
//! game ships them in, put in archives and mounted with no game present.

#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

use or_io::container::rpf::crypto::NoOpCryptoProvider;
use or_io::container::rpf::RpfBuilder;
use or_io::decl::File as _;
use or_io::gta5::shared::Vec4V;
use or_io::gta5::{PhBoundAny, PhBoundSphere};
use or_io::meta::canonical;
use or_io::meta::instance::HashedString;
use or_io::meta::{
    build_library, write_meta_binary, BinaryFormat, MetaInstance, SchemaLibrary, Value,
};
use or_title::vfs::{build_mounted_game, DlcCutoff};
use or_title::{GtaInstall, MountedGame};

/// The members the world index reads, and no other: a file written from this reads back to the
/// tree it was written from, whatever schema snapshot the machine has.
const PSC: &str = r#"<?xml version="1.0"?>
<ParserSchema>
  <structdef type="CEntityDef">
    <string name="archetypeName" type="atHashString"/>
    <Vector3 name="position"/>
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

/// A `.ymap`'s bytes: a `CMapData` named `name`, as an RSC7 resource.
pub fn map_bytes(name: &str, spec: &MapSpec) -> Vec<u8> {
    let mut entity = MetaInstance::new("CEntityDef");
    entity.set("archetypeName", Value::Hash(HashedString::new("prop_a")));
    entity.set("position", Value::Vec3(spec.entities.0));
    let mut entities = vec![Value::Pointer(Some(Box::new(entity)))];
    for interior in &spec.interiors {
        let mut placed = MetaInstance::new("CMloInstanceDef");
        placed.set("archetypeName", Value::Hash(HashedString::new(*interior)));
        placed.set("position", Value::Vec3(spec.entities.0));
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
    let (min, max) = span;
    let mut sphere = PhBoundSphere::default();
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
}

/// The copy the mount would load of each file of `extension`: its lower-case stem, and the last
/// logical path the mount lists it under.
fn winners(game: &MountedGame, extension: &str) -> std::collections::BTreeMap<String, String> {
    let mut winners = std::collections::BTreeMap::new();
    game.for_each_file(|logical, _container, leaf| {
        let leaf = leaf.to_ascii_lowercase();
        if let Some(stem) = leaf.strip_suffix(extension) {
            winners.insert(stem.to_string(), logical.to_string());
        }
    });
    winners
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
    assert_eq!(named(maps.collect()), listed_maps);
    assert_eq!(named(types.collect()), listed_types);
    assert_eq!(named(bounds.collect()), winners(&game, ".ybn"));

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
        "{:?}",
        report.kept
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
