//! The names a document's reference fields can take, from `or-world`'s tables and the game's own.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use or_io::container::RpfCryptoProvider;
use or_io::hash::literal_string_hash;
use or_io::meta::instance::value::HashedString;
use or_io::meta::{MetaInstance, Value};
use or_title::vfs::{build_mounted_game_with, DlcCutoff, MountedGame};
use or_title::{GtaInstall, MountOptions};
use or_world_gta5::{field, keys};

/// What a reference field offers, and how the field stores a choice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reference<'a> {
    /// The field stores a room's position in its interior's `rooms`.
    Room(&'a [String]),
    /// The field stores an entity's position in its interior's `entities`, labelled by archetype.
    Entity(&'a [String]),
    /// The field stores an archetype's name hash.
    Archetype(&'a [HashedString]),
    /// The field stores the name hash of an entity set its interior declares.
    EntitySet(&'a [HashedString]),
    /// The field stores the name hash of a scenario type or type group the install declares.
    ScenarioType(&'a [HashedString]),
    /// The field stores the name hash of a ped or vehicle model set the install declares.
    ModelSet(&'a [HashedString]),
    /// The field stores the name hash of a procedural object or plant tag the install declares.
    ProceduralTag(&'a [HashedString]),
}

/// The tables in scope at one struct of a document: archetypes, an interior's rooms and entities, its sets.
#[derive(Debug, Clone, Default)]
pub struct ReferenceNames {
    archetypes: Arc<[HashedString]>,
    rooms: Arc<[String]>,
    entities: Arc<[String]>,
    entity_sets: Arc<[HashedString]>,
    interiors: Arc<InteriorSets>,
    tables: Arc<GameTables>,
}

impl ReferenceNames {
    /// The tables of a document, given the install's interiors and its own tables.
    pub fn placing(interiors: Arc<InteriorSets>, tables: Arc<GameTables>) -> Self {
        Self {
            interiors,
            tables,
            ..Self::default()
        }
    }

    /// The tables in scope inside `inst`, given those in scope around it.
    pub fn within(&self, inst: &MetaInstance) -> Self {
        let mut names = self.clone();
        if let Some(types) = or_world_gta5::MapTypes::new(inst) {
            names.archetypes = types
                .archetypes()
                .filter_map(|a| hashed(field::get(a.instance(), keys::NAME)?))
                .collect();
        }
        if let Some(interior) = or_world_gta5::Archetype::new(inst).interior() {
            names.rooms = interior
                .rooms()
                .map(|r| r.name().unwrap_or_default())
                .collect();
            names.entities = field::structs_at(inst, keys::ENTITIES)
                .map(|e| {
                    field::get(e, keys::ARCHETYPE_NAME)
                        .and_then(hashed)
                        .map(|h| {
                            if h.raw.is_empty() {
                                h.to_literal()
                            } else {
                                h.raw
                            }
                        })
                        .unwrap_or_default()
                })
                .collect();
        }
        if field::is_type(inst, keys::types::C_MLO_INSTANCE_DEF) {
            names.entity_sets = field::hash_at(inst, keys::ARCHETYPE_NAME)
                .and_then(|name| self.interiors.0.get(&name).cloned())
                .unwrap_or_default();
        }
        names
    }

    /// The names the field called `field_name` can take here, if it is a reference.
    pub fn for_field(&self, field_name: &str) -> Option<Reference<'_>> {
        let key = field::name_hash(field_name);
        if [keys::ROOM_FROM, keys::ROOM_TO, keys::LOCATIONS].contains(&key)
            && !self.rooms.is_empty()
        {
            Some(Reference::Room(&self.rooms))
        } else if key == keys::ATTACHED_OBJECTS && !self.entities.is_empty() {
            Some(Reference::Entity(&self.entities))
        } else if key == keys::ARCHETYPE_NAME && !self.archetypes.is_empty() {
            Some(Reference::Archetype(&self.archetypes))
        } else if key == keys::DEFAULT_ENTITY_SETS && !self.entity_sets.is_empty() {
            Some(Reference::EntitySet(&self.entity_sets))
        } else {
            self.tables.for_field(key)
        }
    }
}

fn hashed(value: &Value) -> Option<HashedString> {
    match value {
        Value::Hash(h) => Some(h.clone()),
        Value::String(s) => Some(HashedString::new(s.as_str())),
        _ => None,
    }
}

/// Every interior's entity-set names, by the interior archetype's name hash.
#[derive(Debug, Clone, Default)]
pub struct InteriorSets(HashMap<u32, Arc<[HashedString]>>);

impl InteriorSets {
    /// Add the interiors one `.ytyp` declares; a later file wins.
    pub fn add(&mut self, types: &MetaInstance) {
        let Some(types) = or_world_gta5::MapTypes::new(types) else {
            return;
        };
        for archetype in types.archetypes() {
            if archetype.interior().is_none() || archetype.name() == 0 {
                continue;
            }
            let sets = field::structs_at(archetype.instance(), keys::ENTITY_SETS)
                .filter_map(|set| hashed(field::get(set, keys::NAME)?))
                .collect();
            self.0.insert(archetype.name(), sets);
        }
    }
}

/// The names the install's own tables declare: scenario types, model sets, procedural tags.
#[derive(Debug, Clone, Default)]
pub struct GameTables {
    scenario_types: Vec<HashedString>,
    // ponytail: ped and vehicle sets share one list, so a ped field also offers vehicle sets;
    // keep them apart by the file's `fileType` if that misleads.
    model_sets: Vec<HashedString>,
    proc_obj_tags: Vec<HashedString>,
    plant_tags: Vec<HashedString>,
}

const SCENARIO_TYPE_FIELDS: [u32; 3] = [
    literal_string_hash(b"spawnType"),
    literal_string_hash(b"ScenarioType"),
    literal_string_hash(b"TypeNames"),
];
const MODEL_SET_FIELDS: [u32; 5] = [
    literal_string_hash(b"pedType"),
    literal_string_hash(b"ModelSet"),
    literal_string_hash(b"PedModelSetNames"),
    literal_string_hash(b"VehicleModelSetNames"),
    literal_string_hash(b"VehicleModelSet"),
];
const PROC_OBJ_TAG_FIELDS: [u32; 1] = [literal_string_hash(b"procObjTag")];
const PLANT_TAG_FIELDS: [u32; 2] = [
    literal_string_hash(b"plantTag"),
    literal_string_hash(b"PlantTag"),
];

/// The files the base game loads its tables from; a DLC pack names its own by `fileType`.
const BASE_TABLES: [&str; 4] = [
    "common:/data/ai/scenarios.meta",
    "common:/data/ai/ambientpedmodelsets.meta",
    "common:/data/ai/vehiclemodelsets.meta",
    "common:/data/materials/procedural.meta",
];
const TABLE_FILE_TYPES: [&str; 4] = [
    "SCENARIO_INFO_FILE",
    "AMBIENT_PED_MODEL_SET_FILE",
    "AMBIENT_VEHICLE_MODEL_SET_FILE",
    "PROCEDURAL_METADATA_FILE",
];

impl GameTables {
    /// Add the names one table file declares; a file of another type adds nothing.
    pub fn add(&mut self, table: &MetaInstance) {
        let names = |array: &[u8], name: &[u8]| {
            let name = literal_string_hash(name);
            field::structs_at(table, literal_string_hash(array))
                .filter_map(move |s| hashed(field::get(s, name)?))
        };
        if field::is_type(table, "CScenarioInfoManager") {
            let types = names(b"Scenarios", b"Name").chain(names(b"ScenarioTypeGroups", b"Name"));
            merge(&mut self.scenario_types, types);
        } else if field::is_type(table, "CAmbientModelSets") {
            merge(&mut self.model_sets, names(b"ModelSets", b"Name"));
        } else if field::is_type(table, "CProceduralInfo") {
            merge(&mut self.proc_obj_tags, names(b"procObjInfos", b"Tag"));
            merge(&mut self.plant_tags, names(b"plantInfos", b"Tag"));
        }
    }

    fn for_field(&self, key: u32) -> Option<Reference<'_>> {
        if SCENARIO_TYPE_FIELDS.contains(&key) && !self.scenario_types.is_empty() {
            Some(Reference::ScenarioType(&self.scenario_types))
        } else if MODEL_SET_FIELDS.contains(&key) && !self.model_sets.is_empty() {
            Some(Reference::ModelSet(&self.model_sets))
        } else if PROC_OBJ_TAG_FIELDS.contains(&key) && !self.proc_obj_tags.is_empty() {
            Some(Reference::ProceduralTag(&self.proc_obj_tags))
        } else if PLANT_TAG_FIELDS.contains(&key) && !self.plant_tags.is_empty() {
            Some(Reference::ProceduralTag(&self.plant_tags))
        } else {
            None
        }
    }
}

/// Sorted by name with one entry per hash, so a name a later file declares again is offered once.
fn merge(list: &mut Vec<HashedString>, names: impl Iterator<Item = HashedString>) {
    list.extend(names.filter(|h| h.hash != 0));
    list.sort_by(|a, b| a.raw.cmp(&b.raw));
    list.dedup_by_key(|h| h.hash);
}

impl ReadFromInstall for GameTables {
    fn read(game: &MountedGame) -> Self {
        let packs = game
            .packs
            .iter()
            .flat_map(|pack| &pack.data_files)
            .filter(|df| {
                TABLE_FILE_TYPES
                    .iter()
                    .any(|t| df.file_type.eq_ignore_ascii_case(t))
            })
            .map(|df| df.filename.as_str());
        let mut tables = Self::default();
        for path in BASE_TABLES.into_iter().chain(packs) {
            let Some(bytes) = game.vfs.open(path) else {
                continue;
            };
            if let Ok(doc) = or_io::meta::MetaDocument::parse(crate::state::meta_schema(), bytes) {
                tables.add(&doc.instance);
            }
        }
        tracing::info!(
            scenario_types = tables.scenario_types.len(),
            model_sets = tables.model_sets.len(),
            procedural_tags = tables.proc_obj_tags.len() + tables.plant_tags.len(),
            "Game tables read"
        );
        tables
    }
}

/// Whether a document places an interior, whose entity sets only an install can name.
pub fn places_an_interior(root: &MetaInstance) -> bool {
    field::structs_at(root, keys::ENTITIES)
        .any(|e| field::is_type(e, keys::types::C_MLO_INSTANCE_DEF))
}

/// What a table read from the mounted install holds; a file it cannot read adds nothing.
pub trait ReadFromInstall: Default + Send + 'static {
    fn read(game: &MountedGame) -> Self;
}

/// The interiors of the install, read once per install off the UI thread.
pub type InstallInteriors = InstallRead<InteriorSets>;
/// The install's scenario types, model sets and procedural tags, read once per install.
pub type InstallTables = InstallRead<GameTables>;

/// What `T` reads from the install, once per install off the UI thread.
#[derive(Default)]
pub struct InstallRead<T> {
    read: Arc<T>,
    rx: Option<Receiver<T>>,
    install: Option<(PathBuf, MountOptions)>,
}

impl<T: ReadFromInstall> InstallRead<T> {
    /// Start reading this install, unless it is read or being read.
    pub fn ensure(
        &mut self,
        install_root: PathBuf,
        crypto: Arc<dyn RpfCryptoProvider>,
        mount: MountOptions,
    ) {
        let install = Some((install_root.clone(), mount));
        if self.install == install {
            return;
        }
        // ponytail: a superseded walk runs to its end and its answer is dropped; cancel it if
        // switching installs mid-walk proves costly.
        self.install = install;
        self.read = Arc::default();
        let (tx, rx) = channel();
        self.rx = Some(rx);
        std::thread::spawn(move || {
            let install = GtaInstall {
                root: install_root.clone(),
            };
            let read = match build_mounted_game_with(&install, crypto, DlcCutoff::All, mount) {
                Ok(game) => T::read(&game),
                Err(e) => {
                    tracing::warn!(
                        install = %install_root.display(),
                        error = %e,
                        "Could not mount the install; the fields it names offer no names"
                    );
                    T::default()
                }
            };
            let _ = tx.send(read);
        });
    }

    /// What is read so far: nothing until the walk finishes.
    pub fn get(&mut self) -> Arc<T> {
        if let Some(read) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.read = Arc::new(read);
            self.rx = None;
        }
        Arc::clone(&self.read)
    }
}

impl ReadFromInstall for InteriorSets {
    /// ⚠ One parsed tree at a time, and one copy per leaf chosen before any is parsed: a retail
    /// install ships many copies of most `.ytyp`.
    fn read(game: &MountedGame) -> Self {
        let mut sets = Self::default();
        let mut winners: BTreeMap<String, String> = BTreeMap::new();
        game.for_each_file(|logical, _container, leaf| {
            let leaf = leaf.to_ascii_lowercase();
            if leaf.ends_with(".ytyp") {
                winners.insert(leaf, logical.to_string());
            }
        });
        for path in winners.values() {
            let Some(bytes) = game.vfs.open(path) else {
                continue;
            };
            if let Ok(doc) = or_io::meta::MetaDocument::parse(crate::state::meta_schema(), bytes) {
                sets.add(&doc.instance);
            }
        }
        tracing::info!(interiors = sets.0.len(), "Interior entity sets read");
        sets
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(name: &str) -> Value {
        let mut inst = MetaInstance::new("CMloRoomDef");
        inst.set("name", Value::String(name.to_string()));
        Value::Struct(Box::new(inst))
    }

    fn archetype(type_name: &str, name: &str) -> MetaInstance {
        let mut inst = MetaInstance::new(type_name);
        inst.set("name", Value::Hash(HashedString::new(name)));
        inst
    }

    fn entity(archetype_name: &str) -> Value {
        let mut inst = MetaInstance::new("CEntityDef");
        inst.set(
            "archetypeName",
            Value::Hash(HashedString::new(archetype_name)),
        );
        Value::Pointer(Some(Box::new(inst)))
    }

    fn scene() -> MetaInstance {
        let mut mlo = archetype("CMloArchetypeDef", "v_int_hall");
        mlo.set(
            "rooms",
            Value::Array(vec![room("limbo"), room("hall"), room("kitchen")]),
        );
        mlo.set(
            "entities",
            Value::Array(vec![entity("prop_chair"), entity("prop_lamp")]),
        );
        let mut set = archetype("CMloEntitySet", "set_party");
        set.set("locations", Value::Array(vec![Value::UInt(1)]));
        set.set("entities", Value::Array(vec![entity("prop_balloon")]));
        mlo.set(
            "entitySets",
            Value::Array(vec![Value::Struct(Box::new(set))]),
        );
        let mut root = MetaInstance::new("CMapTypes");
        root.set(
            "archetypes",
            Value::Array(vec![
                Value::Pointer(Some(Box::new(archetype("CBaseArchetypeDef", "prop_chair")))),
                Value::Pointer(Some(Box::new(mlo))),
            ]),
        );
        root
    }

    fn interior(root: &MetaInstance) -> &MetaInstance {
        field::structs_at(root, keys::ARCHETYPES)
            .nth(1)
            .expect("the scene's interior")
    }

    #[test]
    fn a_room_id_field_offers_the_scenes_room_names() {
        let root = scene();
        let names = ReferenceNames::default()
            .within(&root)
            .within(interior(&root));
        let rooms = ["limbo", "hall", "kitchen"].map(String::from);
        for spelling in ["roomFrom", "roomTo", &format!("0x{:08X}", keys::ROOM_TO)] {
            assert_eq!(
                names.for_field(spelling),
                Some(Reference::Room(&rooms[..])),
                "{spelling}"
            );
        }
    }

    #[test]
    fn an_archetype_name_field_offers_the_documents_archetypes() {
        let root = scene();
        let names = ReferenceNames::default()
            .within(&root)
            .within(interior(&root));
        let Some(Reference::Archetype(offered)) = names.for_field("archetypeName") else {
            panic!("archetypeName is a reference field");
        };
        let offered: Vec<&str> = offered.iter().map(|h| h.raw.as_str()).collect();
        assert_eq!(offered, ["prop_chair", "v_int_hall"]);
    }

    #[test]
    fn an_entity_sets_location_offers_the_interiors_room_names() {
        let root = scene();
        let mlo = interior(&root);
        let set = field::structs_at(mlo, keys::ENTITY_SETS)
            .next()
            .expect("the interior's entity set");
        let names = ReferenceNames::default()
            .within(&root)
            .within(mlo)
            .within(set);
        let rooms = ["limbo", "hall", "kitchen"].map(String::from);
        assert_eq!(
            names.for_field("locations"),
            Some(Reference::Room(&rooms[..]))
        );
    }

    #[test]
    fn an_attached_object_offers_the_interiors_entities() {
        let root = scene();
        let names = ReferenceNames::default()
            .within(&root)
            .within(interior(&root));
        let entities = ["prop_chair", "prop_lamp"].map(String::from);
        assert_eq!(
            names.for_field("attachedObjects"),
            Some(Reference::Entity(&entities[..]))
        );
    }

    /// A `.ymap` placing the scene's interior, whose `.ytyp` it does not hold.
    fn placement() -> MetaInstance {
        let mut placed = MetaInstance::new("CMloInstanceDef");
        placed.set(
            "archetypeName",
            Value::Hash(HashedString::new("v_int_hall")),
        );
        placed.set("defaultEntitySets", Value::Array(Vec::new()));
        let mut map = MetaInstance::new("CMapData");
        map.set(
            "entities",
            Value::Array(vec![Value::Pointer(Some(Box::new(placed)))]),
        );
        map
    }

    fn placed(map: &MetaInstance) -> &MetaInstance {
        field::structs_at(map, keys::ENTITIES)
            .next()
            .expect("the placed interior")
    }

    #[test]
    fn a_default_entity_set_offers_the_sets_another_ytyp_declares() {
        let mut install = InteriorSets::default();
        install.add(&scene());
        let map = placement();
        assert!(places_an_interior(&map));
        let names = ReferenceNames::placing(Arc::new(install), Arc::default())
            .within(&map)
            .within(placed(&map));
        let Some(Reference::EntitySet(offered)) = names.for_field("defaultEntitySets") else {
            panic!("defaultEntitySets is a reference field once the install declares the interior");
        };
        let offered: Vec<&str> = offered.iter().map(|h| h.raw.as_str()).collect();
        assert_eq!(offered, ["set_party"]);
    }

    #[test]
    fn a_default_entity_set_the_install_does_not_declare_offers_nothing() {
        let map = placement();
        let mut elsewhere = scene();
        elsewhere.set("archetypes", Value::Array(Vec::new()));
        let mut install = InteriorSets::default();
        install.add(&elsewhere);
        for names in [
            ReferenceNames::default(),
            ReferenceNames::placing(Arc::new(install), Arc::default()),
        ] {
            let names = names.within(&map).within(placed(&map));
            assert_eq!(names.for_field("defaultEntitySets"), None);
        }
    }

    #[test]
    fn a_room_id_outside_an_interior_offers_nothing() {
        let root = scene();
        let names = ReferenceNames::default().within(&root);
        assert_eq!(names.for_field("roomFrom"), None);
        assert_eq!(names.for_field("lodDist"), None);
    }

    /// A table file of `type_name`, each array holding structs named in a field.
    fn table(type_name: &str, arrays: &[(&str, &str, &[&str])]) -> MetaInstance {
        let mut root = MetaInstance::new(type_name);
        for (array, name_field, names) in arrays {
            let items = names
                .iter()
                .map(|name| {
                    let mut item = MetaInstance::new("Item");
                    item.set(*name_field, Value::String(name.to_string()));
                    Value::Pointer(Some(Box::new(item)))
                })
                .collect();
            root.set(*array, Value::Array(items));
        }
        root
    }

    /// The tables a synthetic install declares, across the files that carry them.
    fn install_tables() -> Arc<GameTables> {
        let mut tables = GameTables::default();
        tables.add(&table(
            "CScenarioInfoManager",
            &[
                (
                    "Scenarios",
                    "Name",
                    &["WORLD_HUMAN_SMOKING", "PROP_HUMAN_SEAT_BENCH"],
                ),
                ("ScenarioTypeGroups", "Name", &["BENCH_GROUP"]),
            ],
        ));
        tables.add(&table(
            "CAmbientModelSets",
            &[("ModelSets", "Name", &["AIRWORKERS"])],
        ));
        tables.add(&table(
            "CAmbientModelSets",
            &[("ModelSets", "Name", &["AIRPORT_BUS", "AIRWORKERS"])],
        ));
        tables.add(&table(
            "CProceduralInfo",
            &[
                ("procObjInfos", "Tag", &["TARMAC"]),
                ("plantInfos", "Tag", &["Proc_Seaweed_Plants_01"]),
            ],
        ));
        Arc::new(tables)
    }

    fn raw(names: &[HashedString]) -> Vec<&str> {
        names.iter().map(|h| h.raw.as_str()).collect()
    }

    #[test]
    fn a_scenario_type_field_offers_the_types_the_install_declares() {
        let names = ReferenceNames::placing(Arc::default(), install_tables());
        let numbered = format!("0x{:08X}", field::name_hash("spawnType"));
        for spelling in ["spawnType", "ScenarioType", "TypeNames", &numbered] {
            let Some(Reference::ScenarioType(types)) = names.for_field(spelling) else {
                panic!("{spelling} is a scenario-type field");
            };
            assert_eq!(
                raw(types),
                [
                    "BENCH_GROUP",
                    "PROP_HUMAN_SEAT_BENCH",
                    "WORLD_HUMAN_SMOKING"
                ],
                "{spelling}"
            );
        }
    }

    #[test]
    fn a_model_set_field_offers_the_sets_the_install_declares() {
        let names = ReferenceNames::placing(Arc::default(), install_tables());
        for spelling in [
            "pedType",
            "ModelSet",
            "PedModelSetNames",
            "VehicleModelSetNames",
        ] {
            let Some(Reference::ModelSet(sets)) = names.for_field(spelling) else {
                panic!("{spelling} is a model-set field");
            };
            assert_eq!(raw(sets), ["AIRPORT_BUS", "AIRWORKERS"], "{spelling}");
        }
    }

    #[test]
    fn a_procedural_tag_field_offers_the_tags_the_install_declares() {
        let names = ReferenceNames::placing(Arc::default(), install_tables());
        let Some(Reference::ProceduralTag(tags)) = names.for_field("procObjTag") else {
            panic!("procObjTag is a procedural-tag field");
        };
        assert_eq!(raw(tags), ["TARMAC"]);
        for spelling in ["plantTag", "PlantTag"] {
            let Some(Reference::ProceduralTag(tags)) = names.for_field(spelling) else {
                panic!("{spelling} is a procedural-tag field");
            };
            assert_eq!(raw(tags), ["Proc_Seaweed_Plants_01"], "{spelling}");
        }
    }

    #[test]
    fn a_table_field_with_no_install_offers_nothing() {
        let names = ReferenceNames::default();
        for spelling in ["spawnType", "pedType", "procObjTag"] {
            assert_eq!(names.for_field(spelling), None, "{spelling}");
        }
    }
}
