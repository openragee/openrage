//! What the disk says of the sources a module names, gathered once so that every rule and the plan are pure functions of it.
//!
//! A map set names source folders of its module. For each one this looks at the folder as a source does
//! (`or_source::cook::describe`: what it cooks to, how big its inputs are, whether it is the game's own file), and from
//! `Depth::Open` on it cooks a map file or a type file and reads what it places and declares, through the world model of `or-world`.
//!
//! ⚠ **A name is a hash.** A map file states its archetypes and an archetype its model by `atHashString`, and the text is known only
//! where the reader's name index or the file itself gives one that hashes to the number. [`Name`] keeps both and never invents the text.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use or_io::meta::{MetaDocument, MetaInstance, Value};
pub use or_project::content::facts::{stays_inside, Depth};
use or_world::keys;
use or_world::map_types::ArchetypeKind;
use or_world::{field, MapData, MapTypes};

use crate::model::Maps;

/// What a source's extension and kind make of it in a map set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// A `.ymap`: what is placed.
    Map,
    /// A `.ytyp`: what can be placed.
    Types,
    /// A drawable, a drawable dictionary or a fragment: a model an archetype names.
    Model,
    /// A texture dictionary.
    Textures,
}

/// A name as the files state it: the hash, and the text when one is known that hashes to it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Name {
    pub hash: u32,
    pub text: Option<String>,
}

impl Name {
    pub fn of(hash: u32) -> Self {
        Self { hash, text: None }
    }

    /// A name from its text.
    pub fn from_text(text: &str) -> Self {
        Self {
            hash: or_io::string_hash(text.as_bytes()),
            text: Some(text.to_string()),
        }
    }

    /// What to show: the text, or the hash as `hash_XXXXXXXX`, the spelling a source's XML gives an unknown name.
    pub fn shown(&self) -> String {
        match &self.text {
            Some(text) => text.clone(),
            None => format!("hash_{:08X}", self.hash),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.hash == 0
    }
}

/// Where a hash gets its text when the file gives none: a table of the names the caller knows, such as the index of names of the game.
pub type Names<'a> = Option<&'a dyn Fn(u32) -> Option<String>>;

/// The name a value of a hash member states, its text from the file when the file gives one that hashes to it, else from `names`.
fn name_of(value: Option<&Value>, names: Names<'_>) -> Name {
    let hash = match value {
        Some(Value::Hash(h)) => {
            if !h.raw.is_empty() && or_io::string_hash(h.raw.as_bytes()) == h.hash {
                return Name {
                    hash: h.hash,
                    text: Some(h.raw.clone()),
                };
            }
            h.hash
        }
        Some(Value::UInt(v)) => *v as u32,
        Some(Value::String(s)) if !s.is_empty() => return Name::from_text(s),
        _ => return Name::default(),
    };
    Name {
        hash,
        text: names.and_then(|names| names(hash)).filter(|text| {
            // A table that is wrong about a hash is not believed.
            or_io::string_hash(text.as_bytes()) == hash
        }),
    }
}

fn name_at(inst: &MetaInstance, key: u32, names: Names<'_>) -> Name {
    name_of(field::get(inst, key), names)
}

/// What is known of one source a set names.
#[derive(Debug, Clone, PartialEq)]
pub enum Fact {
    /// The name leaves the module's folder, or is no name a host can create.
    Escapes,
    Missing,
    /// There is something there and it is not a source this build can cook, with the reason.
    Unreadable(String),
    Source(Box<SourceFact>),
}

/// A source folder as a build sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceFact {
    /// The word that chooses the compiler: `metadata`, `drawable`, `drawables`, `fragment` or `textures`.
    pub kind: String,
    /// The asset's own name, which names the cooked file.
    pub name: String,
    /// The game extension of the cooked file.
    pub extension: String,
    /// The size of every file the cook reads.
    pub bytes: u64,
    /// The SHA-256 of the game files the source was taken from, which a build that leaves out an unchanged original compares with.
    pub game: Vec<String>,
    /// A digest of every file in the folder, from [`Depth::Digest`] on.
    pub digest: Option<String>,
    /// What the file the source cooks to places or declares, or why it cannot be read; from [`Depth::Open`] on, for a map file and a type file.
    pub inside: Option<Result<Inside, String>>,
}

/// What a source of `kind` that cooks to a file of `extension` is for in a set; none for a kind a maps module does not hold.
pub(crate) fn role_of(kind: &str, extension: &str) -> Option<Role> {
    match (kind, extension) {
        ("metadata", "ymap") => Some(Role::Map),
        ("metadata", "ytyp") => Some(Role::Types),
        ("drawable" | "drawables" | "fragment", _) => Some(Role::Model),
        ("textures", _) => Some(Role::Textures),
        _ => None,
    }
}

impl SourceFact {
    /// The file the source cooks to, such as `harbour.ymap`.
    pub fn file_name(&self) -> String {
        format!("{}.{}", self.name, self.extension)
    }

    /// What the source is for in a set.
    pub fn role(&self) -> Option<Role> {
        role_of(&self.kind, &self.extension)
    }

    pub fn map(&self) -> Option<&MapInside> {
        match self.inside.as_ref()?.as_ref().ok()? {
            Inside::Map(map) => Some(map),
            Inside::Types(_) => None,
        }
    }

    pub fn types(&self) -> Option<&TypesInside> {
        match self.inside.as_ref()?.as_ref().ok()? {
            Inside::Types(types) => Some(types),
            Inside::Map(_) => None,
        }
    }
}

/// What the cooked file of a source states.
#[derive(Debug, Clone, PartialEq)]
pub enum Inside {
    Map(Box<MapInside>),
    Types(Box<TypesInside>),
}

/// One placement of an archetype, as the map file states it.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityFact {
    pub archetype: Name,
    pub guid: u32,
    pub position: [f32; 3],
    /// The four floats the file stores, whose meaning depends on [`EntityFact::interior`] (`or_world::Placement`).
    pub rotation: [f32; 4],
    pub scale_xy: f32,
    pub scale_z: f32,
    pub flags: u32,
    pub parent_index: i32,
    /// How many entities it states name it as their LOD parent, in its own map file and in the ones under it.
    pub num_children: u32,
    pub lod_dist: f32,
    /// The level of detail it is for, as the file names it (`LODTYPES_DEPTH_HD`), none where the file states none.
    pub lod_level: Option<String>,
    /// Whether it places an interior, whose rotation is stored as it is and not conjugated.
    pub interior: bool,
}

/// What a `.ymap` places.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MapInside {
    pub name: Name,
    pub parent: Name,
    pub flags: u32,
    pub content_flags: u32,
    pub entities: Vec<EntityFact>,
    /// The archetype each entity and each grass batch places, which is what the manifest's derivation reads (`or_world::manifest::placed`).
    pub placed: Vec<u32>,
    /// The extents the file states, none where it states none.
    pub stored_entities: Option<([f32; 3], [f32; 3])>,
    pub stored_streaming: Option<([f32; 3], [f32; 3])>,
    /// What the engine would misread in the file, one sentence each (`or_world::validate`).
    pub problems: Vec<String>,
}

/// An archetype, as a type file declares it.
#[derive(Debug, Clone, PartialEq)]
pub struct ArchetypeFact {
    pub name: Name,
    pub asset_name: Name,
    pub kind: ArchetypeKind,
    pub bb_min: [f32; 3],
    pub bb_max: [f32; 3],
    pub drawable_dictionary: Name,
    pub texture_dictionary: Name,
    pub lod_dist: f32,
    pub flags: u32,
    pub interior: Option<InteriorFact>,
}

/// An interior archetype's rooms, portals and entity sets, as `or_world` reads them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InteriorFact {
    pub rooms: Vec<RoomFact>,
    pub portals: Vec<PortalFact>,
    pub entity_sets: usize,
    pub entities: usize,
    pub problems: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RoomFact {
    pub name: String,
    pub bb_min: [f32; 3],
    pub bb_max: [f32; 3],
    pub flags: u32,
    pub floor_id: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PortalFact {
    pub room_from: u32,
    pub room_to: u32,
    pub flags: u32,
    pub corners: usize,
}

/// What a `.ytyp` declares.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TypesInside {
    pub name: Name,
    pub archetypes: Vec<ArchetypeFact>,
    pub dependencies: Vec<Name>,
}

/// The sources of a module by the path it names them with.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Facts {
    sources: BTreeMap<String, Fact>,
    /// The source folders of the module's own folder that no set names.
    pub unused: Vec<String>,
    /// The manifests of the game the module keeps, from [`Depth::Open`] on: what its own manifest is typed by and takes its entries from.
    pub manifests: Vec<crate::manifest::Kept>,
}

impl Facts {
    pub fn get(&self, path: &str) -> Option<&Fact> {
        self.sources.get(path)
    }

    pub fn insert(&mut self, path: &str, fact: Fact) {
        self.sources.insert(path.to_string(), fact);
    }

    /// The source `path` names, none when it is missing or not one.
    pub fn source(&self, path: &str) -> Option<&SourceFact> {
        match self.get(path) {
            Some(Fact::Source(source)) => Some(source),
            _ => None,
        }
    }
}

/// Every source a module names, once each.
pub fn named_paths(model: &Maps) -> impl Iterator<Item = &str> {
    let mut seen = BTreeSet::new();
    model
        .maps
        .iter()
        .flat_map(|m| m.sources.iter())
        .map(String::as_str)
        .filter(move |path| seen.insert(*path))
}

/// The sources `model` names, looked at in `folder` as deeply as `depth` says.
pub fn inspect(folder: &Path, model: &Maps, depth: Depth) -> Facts {
    inspect_named(folder, model, depth, None)
}

/// [`inspect`], with the text of the names the files give none for taken from `names`.
pub fn inspect_named(folder: &Path, model: &Maps, depth: Depth, names: Names<'_>) -> Facts {
    let mut facts = Facts::default();
    // The names the pack remembers come first: they are the person's own.
    let own = |hash: u32| {
        model
            .name_of(hash)
            .or_else(|| names.and_then(|names| names(hash)))
    };
    for path in named_paths(model) {
        facts.insert(path, look(folder, path, depth, Some(&own)));
    }
    facts.unused = unused(folder, model);
    if depth >= Depth::Open {
        facts.manifests = crate::manifest::kept(folder);
    }
    facts
}

/// The source folders of `folder` that `model` does not name, in order.
fn unused(folder: &Path, model: &Maps) -> Vec<String> {
    let named: BTreeSet<&str> = named_paths(model).collect();
    let mut unused: Vec<String> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| {
            name.to_ascii_lowercase().ends_with(".src") && !named.contains(name.as_str())
        })
        .collect();
    unused.sort();
    unused
}

fn look(folder: &Path, path: &str, depth: Depth, names: Names<'_>) -> Fact {
    if !stays_inside(path) {
        return Fact::Escapes;
    }
    let dir = folder.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
    if !dir.exists() {
        return Fact::Missing;
    }
    if !dir.is_dir() {
        return Fact::Unreadable(
            "is a file, and a map set names the source folders of its module".to_string(),
        );
    }
    let described = match or_source::cook::describe(&dir) {
        Ok(described) => described,
        Err(why) => return Fact::Unreadable(why),
    };
    let mut fact = SourceFact {
        kind: described.kind,
        name: described.name,
        extension: described.extension,
        bytes: described.bytes,
        game: described.game,
        digest: None,
        inside: None,
    };
    if fact.role().is_none() {
        return Fact::Unreadable(format!(
            "is a {} source cooking to a .{} file, and a maps module holds map files, type files, models and textures",
            fact.kind, fact.extension
        ));
    }
    if depth >= Depth::Digest {
        fact.digest = or_source::game_import::tree_digest(&dir).ok();
    }
    if depth >= Depth::Open {
        fact.inside = match fact.role() {
            Some(Role::Map | Role::Types) => Some(read_inside(&dir, &fact.extension, names)),
            _ => None,
        };
    }
    Fact::Source(Box::new(fact))
}

/// The cooked bytes of the metadata source in `dir`.
pub fn cooked(dir: &Path) -> Result<Vec<u8>, String> {
    let source = or_source::asset::Source::open(dir).map_err(|p| p.message)?;
    or_source::metadata::compile(&source)
        .map(|compiled| compiled.bytes)
        .map_err(|problems| {
            problems
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        })
}

/// What the file the source in `dir` cooks to states, of the kind `extension` says.
pub fn read_inside(dir: &Path, extension: &str, names: Names<'_>) -> Result<Inside, String> {
    let bytes = cooked(dir)?;
    inside_of(&bytes, extension, names)
}

/// What a `.ymap` or `.ytyp` file states.
pub fn inside_of(bytes: &[u8], extension: &str, names: Names<'_>) -> Result<Inside, String> {
    let document = MetaDocument::parse(or_io::meta::default_library(), bytes.to_vec())
        .map_err(|e| format!("does not read as a metadata document: {e}"))?;
    match extension {
        "ymap" => {
            let map = MapData::new(&document.instance)
                .ok_or_else(|| "is no map file: its root is not a CMapData".to_string())?;
            Ok(Inside::Map(Box::new(read_map(&map, names))))
        }
        "ytyp" => {
            let types = MapTypes::new(&document.instance)
                .ok_or_else(|| "is no type file: its root is not a CMapTypes".to_string())?;
            Ok(Inside::Types(Box::new(read_types(&types, names))))
        }
        other => Err(format!(
            "a .{other} file is neither a map file nor a type file"
        )),
    }
}

fn read_map(map: &MapData<'_>, names: Names<'_>) -> MapInside {
    let root = map.instance();
    MapInside {
        name: name_at(root, keys::NAME, names),
        parent: name_at(root, keys::PARENT, names),
        flags: map.flags(),
        content_flags: map.content_flags(),
        entities: map
            .entities()
            .map(|entity| {
                let inst = entity.instance();
                EntityFact {
                    archetype: name_at(inst, keys::ARCHETYPE_NAME, names),
                    guid: entity.guid(),
                    position: entity.position().to_array(),
                    rotation: field::vec4_at(inst, keys::ROTATION).unwrap_or([0.0, 0.0, 0.0, 1.0]),
                    scale_xy: field::f32_at(inst, keys::SCALE_XY).unwrap_or(1.0),
                    scale_z: field::f32_at(inst, keys::SCALE_Z).unwrap_or(1.0),
                    flags: entity.flags(),
                    parent_index: field::i32_at(inst, keys::PARENT_INDEX).unwrap_or(-1),
                    num_children: entity.num_children(),
                    lod_dist: entity.lod_dist(),
                    lod_level: entity.lod_level().map(str::to_string),
                    interior: entity.is_interior_instance(),
                }
            })
            .collect(),
        placed: or_world::manifest::placed(map),
        stored_entities: map
            .stored_entities_extents()
            .map(|(min, max)| (min.to_array(), max.to_array())),
        stored_streaming: map
            .stored_streaming_extents()
            .map(|(min, max)| (min.to_array(), max.to_array())),
        problems: or_world::validate::check(map)
            .iter()
            .map(ToString::to_string)
            .collect(),
    }
}

fn read_types(types: &MapTypes<'_>, names: Names<'_>) -> TypesInside {
    TypesInside {
        name: name_at(types.instance(), keys::NAME, names),
        archetypes: types
            .archetypes()
            .map(|archetype| {
                let inst = archetype.instance();
                let bounds = archetype.bounds();
                ArchetypeFact {
                    name: name_at(inst, keys::NAME, names),
                    asset_name: name_at(inst, keys::ASSET_NAME, names),
                    kind: archetype.kind(),
                    bb_min: bounds.map_or([0.0; 3], |b| b.min.to_array()),
                    bb_max: bounds.map_or([0.0; 3], |b| b.max.to_array()),
                    drawable_dictionary: name_at(inst, keys::DRAWABLE_DICTIONARY, names),
                    texture_dictionary: name_at(inst, keys::TEXTURE_DICTIONARY, names),
                    lod_dist: archetype.lod_dist(),
                    flags: archetype.flags(),
                    interior: archetype
                        .interior()
                        .map(|interior| read_interior(&interior)),
                }
            })
            .collect(),
        dependencies: field::array_at(types.instance(), keys::DEPENDENCIES)
            .iter()
            .map(|value| name_of(Some(value), names))
            .collect(),
    }
}

fn read_interior(interior: &or_world::Interior<'_>) -> InteriorFact {
    InteriorFact {
        rooms: interior
            .rooms()
            .map(|room| {
                let (bb_min, bb_max) = room
                    .bounds()
                    .map_or(([0.0; 3], [0.0; 3]), |(a, b)| (a.to_array(), b.to_array()));
                RoomFact {
                    name: room.name().unwrap_or_default(),
                    bb_min,
                    bb_max,
                    flags: room.flags(),
                    floor_id: room.floor_id(),
                }
            })
            .collect(),
        portals: interior
            .portals()
            .map(|portal| PortalFact {
                room_from: portal.room_from(),
                room_to: portal.room_to(),
                flags: portal.flags(),
                corners: portal.corners().len(),
            })
            .collect(),
        entity_sets: interior.entity_set_count(),
        entities: interior.entity_count(),
        problems: or_world::validate::check_interior(interior)
            .iter()
            .map(ToString::to_string)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MapSet;

    #[test]
    fn the_sources_a_model_names_are_looked_at_to_the_depth_asked() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("nothing.metadata.src")).unwrap();
        std::fs::write(dir.path().join("a.txt"), b"a file").unwrap();
        let mut model = Maps::new();
        let mut set = MapSet::new("harbour");
        for path in [
            "missing.metadata.src",
            "../out.metadata.src",
            "a.txt",
            "nothing.metadata.src",
        ] {
            set.sources.push(path.into());
        }
        model.maps.push(set);
        let facts = inspect(dir.path(), &model, Depth::Stat);
        assert_eq!(facts.get("missing.metadata.src"), Some(&Fact::Missing));
        assert_eq!(facts.get("../out.metadata.src"), Some(&Fact::Escapes));
        assert!(matches!(facts.get("a.txt"), Some(Fact::Unreadable(_))));
        assert!(matches!(
            facts.get("nothing.metadata.src"),
            Some(Fact::Unreadable(_))
        ));
    }

    #[test]
    fn a_name_shows_its_text_or_the_spelling_a_source_gives_an_unknown_one() {
        assert_eq!(Name::from_text("prop_bench_01a").shown(), "prop_bench_01a");
        assert_eq!(Name::of(0xAB).shown(), "hash_000000AB");
        assert!(Name::default().is_empty());
    }
}
