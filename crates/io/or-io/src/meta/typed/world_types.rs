//! The world family's classes, declared here so a map or a type file built from nothing saves where
//! no schema snapshot is installed, which is every user's machine.
//!
//! Each class states the members its typed record names (`CMapData::MEMBERS` and its siblings), in
//! the engine's order and with the engine's types, and
//! `every_declared_world_class_states_the_members_its_record_types` holds the two together. A class
//! the engine derives from another (`CMloInstanceDef`, `CTimeArchetypeDef`, `CMloArchetypeDef`) is
//! not declared: the members that make one are not typed yet. Nor are the six members of a map that
//! hold its parked vehicles, occluders, time cycle modifiers and far lights, or the classes of those
//! records (`map_records`): a map built from nothing states none of them and refuses a record.
//!
//! ⚠ **A document these types lay out is a PSO, never an RBM.** An RBM class carries a `signature`
//! word that nothing in this tree derives, and a PSO type table states none, so a root built here
//! is written as PSO. A record built in an RBM file whose own table does not state its class is
//! refused with that reason where no snapshot declares the class either; where one does, the class
//! is laid out from it with a zero signature.
//!
//! ⚠ **A record copied from another file is laid out as this file's class, never as its own.** It
//! carries the tree it was read from, and a file whose table states no class for it (a map that holds
//! no entity states none) takes it as a record built here is taken: with the members the class has
//! in this document, each one the copy lacks at its empty value. A member the class lacks would be
//! dropped, so the copy is refused, whether the class is the one declared here or one the file's own
//! table states without that member; and so is a copy into an RBM file with no signature for its
//! class, and one that holds a record (an extension of an entity, say) of a class nothing here states
//! or declares.
//!
//! ⚠ **A class a table only names is no class to lay a record out by.** A table states the classes
//! its document holds, so an interior with no room names the room class in `rooms` and states it
//! nowhere, and the library a save is made against then holds that name with no member. A record of
//! it, built or copied, is refused as one of a class the file lacks: laid out by the name alone it
//! would be written with no member, and in an RBM file with a signature nothing derived.
//!
//! ⚠ **The declaration is the last source and the first.** A class the document's own table states
//! is laid out as that table states it and never as declared here. Where the table states none, the
//! declaration wins over an installed snapshot: a project's source compiles through its base's type
//! table with no snapshot, so a save has to write the same members on every machine.

use std::sync::OnceLock;

use crate::meta::binary::synth::name_hash;
use crate::meta::binary::type_tables;
use crate::meta::binary::{ContainerLayout, MetaLayout};
use crate::meta::instance::{MetaInstance, SourceFormat, Value};
use crate::meta::schema::{parse_psc, ResolvedStruct, SchemaItem, SchemaLibrary};

use super::{ped_types, Arrival};

pub const WORLD_TYPES: &str = r#"<ParserSchema>
  <structdef type="CEntityDef">
    <string name="archetypeName" type="atHashString"/>
    <u32 name="flags"/>
    <u32 name="guid"/>
    <Vector3 name="position"/>
    <Vector4 name="rotation"/>
    <float name="scaleXY"/>
    <float name="scaleZ"/>
    <s32 name="parentIndex"/>
    <float name="lodDist"/>
    <float name="childLodDist"/>
    <u32 name="numChildren"/>
    <s32 name="ambientOcclusionMultiplier" init="255"/>
    <s32 name="artificialAmbientOcclusion" init="255"/>
    <u32 name="tintValue" init="0"/>
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
    <array name="entities" type="atArray"><pointer type="rage::fwEntityDef" policy="owner"/></array>
    <array name="physicsDictionaries" type="atArray"><string type="atHashString"/></array>
  </structdef>
  <structdef type="CBaseArchetypeDef">
    <float name="lodDist"/>
    <u32 name="flags"/>
    <u32 name="specialAttribute"/>
    <Vector3 name="bbMin"/>
    <Vector3 name="bbMax"/>
    <Vector3 name="bsCentre"/>
    <float name="bsRadius"/>
    <float name="hdTextureDist" init="5.0"/>
    <string name="name" type="atHashString"/>
    <string name="textureDictionary" type="atHashString"/>
    <string name="clipDictionary" type="atHashString"/>
    <string name="drawableDictionary" type="atHashString"/>
    <string name="physicsDictionary" type="atHashString"/>
    <string name="assetName" type="atHashString"/>
  </structdef>
  <structdef type="CMapTypes">
    <array name="archetypes" type="atArray"><pointer type="rage::fwArchetypeDef" policy="owner"/></array>
    <string name="name" type="atHashString"/>
    <array name="dependencies" type="atArray"><string type="atHashString"/></array>
  </structdef>
</ParserSchema>"#;

/// [`WORLD_TYPES`] as a library.
pub fn declared() -> &'static SchemaLibrary {
    static LIBRARY: OnceLock<SchemaLibrary> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        let mut library = SchemaLibrary::new();
        let items = parse_psc(WORLD_TYPES.as_bytes(), "world")
            .expect("the world types are well-formed PSC");
        for item in items {
            match item {
                SchemaItem::Struct(def) => library.insert_struct(def),
                SchemaItem::Enum(def) => library.insert_enum(def),
                SchemaItem::Skipped => {}
            }
        }
        library
    })
}

/// Why a `type_name` built in, or copied into, an RBM file whose table states no such class is not saved.
fn no_signature(type_name: &str, how: &str) -> String {
    format!(
        "a `{type_name}` {how} cannot be saved into this file: it is an RBM file whose own type table states no `{type_name}`, and an RBM class carries a signature word that nothing derives, so there is none to write; build the file from nothing, which is written as PSO"
    )
}

/// Why a record laid out by a class that lacks `member` is not saved.
fn would_drop(class: &str, member: &str) -> String {
    format!(
        "a `{class}` here holds `{member}`, a member of the file it came from: this file's own type table does not state the class it is laid out as, and the class as it is built from nothing has no such member, so the save would drop it; build the record here instead of copying it"
    )
}

/// Why a record that holds `member` is not saved into a file whose own table states its class without it.
pub(super) fn would_drop_stated(class: &str, member: &str) -> String {
    format!(
        "a `{class}` here holds `{member}`, a member of the file it came from: this file's own type table states `{class}` without it, so the save would drop it; build the record here instead of copying it"
    )
}

/// Why a `type_name` copied into a file is not saved where nothing states or declares its class.
fn no_class(type_name: &str, rbm: bool) -> String {
    match rbm {
        true => no_signature(type_name, "copied here"),
        false => format!(
            "a `{type_name}` copied here cannot be saved into this file: its own type table states no `{type_name}` and none is declared for a file that states none, so there is no layout to write it with; copy it into a file that holds one"
        ),
    }
}

/// What `judge` first finds in `root` or in a record under it, at any depth.
fn first_in<'a, T>(
    root: &'a MetaInstance,
    judge: &impl Fn(&'a MetaInstance) -> Option<T>,
) -> Option<T> {
    fn in_value<'a, T>(
        value: &'a Value,
        judge: &impl Fn(&'a MetaInstance) -> Option<T>,
    ) -> Option<T> {
        match value {
            Value::Struct(inner) | Value::Pointer(Some(inner)) => first_in(inner, judge),
            Value::Array(items) => items.iter().find_map(|v| in_value(v, judge)),
            Value::Map(pairs) => pairs
                .iter()
                .find_map(|(k, v)| in_value(k, judge).or_else(|| in_value(v, judge))),
            _ => None,
        }
    }
    judge(root).or_else(|| root.fields.iter().find_map(|(_, v)| in_value(v, judge)))
}

/// The first class `root` holds that the type table `layout` was read with does not state.
pub fn first_class_unstated<'a>(layout: &MetaLayout, root: &'a MetaInstance) -> Option<&'a str> {
    // ⚠ A tree as it was read names none: a reader builds each record from an entry of that table. A GTA VI document is
    // laid out by tables of its own form, and none of its classes is judged here.
    let stated = |class: &str| {
        let key = name_hash(class);
        match &layout.container {
            ContainerLayout::Pso(pso) => pso.is_gta6() || pso.schema.struct_for(key).is_some(),
            ContainerLayout::Rbm(rbm) => rbm.model.class_for(key).is_some(),
            ContainerLayout::Rbf(_) => true,
        }
    };
    first_in(root, &|inst| {
        (!stated(&inst.type_name)).then_some(inst.type_name.as_str())
    })
}

/// The class and the name of the first member a record under `root` holds that its class lacks in the type table of `layout`.
pub fn first_member_unstated<'a>(
    layout: &MetaLayout,
    root: &'a MetaInstance,
) -> Option<(&'a str, &'a str)> {
    // ⚠ A tree as it was read holds none: a reader gives a record one member for each entry of its class. A writer drops
    // such a member and says nothing, which is why a save looks for one. A class the table lacks is not judged here.
    fn lacked(inst: &MetaInstance, states: impl Fn(u32) -> bool) -> Option<(&str, &str)> {
        let member = (inst.fields.iter()).find(|(name, _)| !states(name_hash(name)));
        member.map(|(name, _)| (inst.type_name.as_str(), name.as_str()))
    }
    let lacking = |inst: &'a MetaInstance| {
        let key = name_hash(&inst.type_name);
        match &layout.container {
            ContainerLayout::Pso(pso) if !pso.is_gta6() => {
                let class = pso.schema.struct_for(key)?;
                lacked(inst, |member| {
                    class.entries.iter().any(|e| e.name_hash == member)
                })
            }
            ContainerLayout::Rbm(rbm) => {
                let class = rbm.model.class_for(key)?;
                lacked(inst, |member| {
                    class.entries.iter().any(|e| e.name_hash == member)
                })
            }
            _ => None,
        }
    };
    first_in(root, &lacking)
}

/// The types a world document is saved against, and which of them this declaration gave.
pub(super) struct Saving {
    pub(super) library: SchemaLibrary,
    /// The name hash of each class `library` takes from [`declared`].
    declared: Vec<u32>,
    /// Whether the document arrived as RBM, whose classes each carry a signature.
    rbm: bool,
    /// The name hash of each class the document's own table states: none for a root built here, and no list for a text
    /// document, which has no table to lack a class.
    stated: Option<Vec<u32>>,
    /// The name hash of each class `library` holds as the document's own table states it, which is every stated class
    /// where no snapshot is installed: a snapshot's class stands where both state one.
    as_stated: Vec<u32>,
    /// The name hash of each class `library` holds as a name alone, with no member: the table names the class of an
    /// array that holds nothing and states it nowhere, so nothing lays a record of it out.
    named_only: Vec<u32>,
}

impl Saving {
    /// The types of a root that arrived as `arrival`, or was built here when there is none.
    pub(super) fn of(arrival: Option<&Arrival>) -> Self {
        let family = ped_types::library(crate::meta::default_library());
        let own = arrival
            .and_then(|a| a.layout.as_ref())
            .map(type_tables::of_layout);
        let as_stated = (own.iter())
            .flat_map(|tables| tables.structs.values())
            .filter(|def| family.struct_(&def.name).is_none())
            .map(|def| name_hash(&def.name))
            .collect();
        let (mut library, mut named_only) = match &own {
            Some(tables) => {
                let library = type_tables::extend(&family, tables).into_owned();
                let given = |name: &str| {
                    family.struct_(name).is_some() || tables.structs.contains_key(name)
                };
                let named_only: Vec<u32> = (library.structs.keys())
                    .filter(|name| !given(name))
                    .map(|name| name_hash(name))
                    .collect();
                (library, named_only)
            }
            None => (family.into_owned(), Vec::new()),
        };
        let stated = |name: &str| {
            let key = name_hash(name);
            own.as_ref()
                .is_some_and(|tables| tables.structs.keys().any(|k| name_hash(k) == key))
        };
        let mut given = Vec::new();
        if arrival.is_none_or(|a| a.format == SourceFormat::Pso) {
            for (name, def) in &declared().structs {
                if !stated(name) {
                    library.insert_struct(def.clone());
                    given.push(name_hash(name));
                    named_only.retain(|class| *class != name_hash(name));
                }
            }
        }
        let stated = match (arrival, &own) {
            (Some(_), None) => None,
            (_, own) => Some(
                own.iter()
                    .flat_map(|tables| tables.structs.keys())
                    .map(|name| name_hash(name))
                    .collect(),
            ),
        };
        Self {
            library,
            declared: given,
            rbm: arrival.is_some_and(|a| a.format == SourceFormat::Rbm),
            stated,
            as_stated,
            named_only,
        }
    }

    /// The members `type_name` has in this document, where anything gives them.
    fn class(&self, type_name: &str) -> Option<ResolvedStruct<'_>> {
        let named_only = self.named_only.contains(&name_hash(type_name));
        (self.library.resolve_any(type_name)).filter(|_| !named_only)
    }

    /// Gives `inst` the members its class has in this document.
    pub(super) fn conform(&self, inst: &mut MetaInstance) -> Result<(), String> {
        if self.class(&inst.type_name).is_none() {
            return Err(match self.rbm {
                true => no_signature(&inst.type_name, "built here"),
                false => super::no_class_to_build(&inst.type_name),
            });
        }
        super::conform(&self.library, inst)
    }

    /// Gives `inst`, a record that carries the tree it was read from, the members its class has in this document where the
    /// document's own table states no such class; a record of a class the table states is left as it arrived.
    pub(super) fn conform_carried(&self, inst: &mut MetaInstance) -> Result<(), String> {
        let class = name_hash(&inst.type_name);
        let Some(false) = self.stated.as_ref().map(|stated| stated.contains(&class)) else {
            return Ok(());
        };
        let Some(resolved) = self.class(&inst.type_name) else {
            return Err(no_class(&inst.type_name, self.rbm));
        };
        let held = |member: &str| {
            let key = name_hash(member);
            resolved.fields.iter().any(|f| name_hash(&f.name) == key)
        };
        if let Some((member, _)) = inst.fields.iter().find(|(member, _)| !held(member)) {
            return Err(would_drop(&inst.type_name, member));
        }
        super::conform(&self.library, inst)
    }

    /// Refuses a record no class of this document lays out, and one that holds a member the class it is laid out by lacks.
    ///
    /// ⚠ What a typed record wrote is conformed or refused before this, so a class met here with no layout came inside a
    /// copy: an extension of an entity, say. A class a snapshot gave is not judged for its members.
    pub(super) fn refuse_what_has_no_place(&self, root: &MetaInstance) -> Result<(), String> {
        // A text document has no table: it states what it holds, and nothing lays it out.
        let Some(stated) = &self.stated else {
            return Ok(());
        };
        let refusal = |inst: &MetaInstance| {
            let class = name_hash(&inst.type_name);
            let Some(resolved) = self.class(&inst.type_name) else {
                return Some(no_class(&inst.type_name, self.rbm));
            };
            let declared_here = |class: &str| self.declared.contains(&name_hash(class));
            let as_stated = self.as_stated.contains(&class);
            if !as_stated && !resolved.chain.iter().any(|def| declared_here(&def.name)) {
                return None;
            }
            let held = |member: &str| {
                let key = name_hash(member);
                resolved.fields.iter().any(|f| name_hash(&f.name) == key)
            };
            let (member, _) = inst.fields.iter().find(|(member, _)| !held(member))?;
            Some(match stated.contains(&class) {
                true => would_drop_stated(&inst.type_name, member),
                false => would_drop(&inst.type_name, member),
            })
        };
        first_in(root, &refusal).map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decl::{File, Vec3, Vec4};
    use crate::meta::binary::hash_index::with_public_names_only;
    use crate::meta::instance::HashedString;
    use crate::meta::typed::{
        CBaseArchetypeDef, CBaseArchetypeDefAny, CEntityDef, CEntityDefAny, CMapData, CMapTypes,
    };
    use crate::meta::{
        build_library, canonical, detect_binary_format, write_meta_binary, BinaryFormat,
        MetaDocument,
    };

    fn members(class: &str) -> Vec<String> {
        declared()
            .resolve(class)
            .unwrap_or_else(|| panic!("`{class}` is declared"))
            .fields
            .iter()
            .map(|f| f.name.clone())
            .collect()
    }

    fn sorted(names: impl IntoIterator<Item = impl Into<String>>) -> Vec<String> {
        let mut names: Vec<String> = names.into_iter().map(Into::into).collect();
        names.sort();
        names
    }

    /// The members a map's record types but the ones that hold the records no class is declared for.
    fn map_members_declared() -> Vec<&'static str> {
        let undeclared = crate::meta::typed::map_records::ROOT_MEMBERS;
        let declared = |member: &&str| !undeclared.contains(member);
        (CMapData::MEMBERS.iter().copied())
            .filter(declared)
            .collect()
    }

    #[test]
    fn every_declared_world_class_states_the_members_its_record_types() {
        let of_a_map = map_members_declared();
        assert_eq!(
            of_a_map.len() + crate::meta::typed::map_records::ROOT_MEMBERS.len(),
            CMapData::MEMBERS.len()
        );
        for (class, typed) in [
            ("CEntityDef", CEntityDef::MEMBERS),
            ("CMapData", &of_a_map[..]),
            ("CBaseArchetypeDef", CBaseArchetypeDef::MEMBERS),
            ("CMapTypes", CMapTypes::MEMBERS),
        ] {
            assert_eq!(
                sorted(members(class)),
                sorted(typed.iter().copied()),
                "`{class}` declares what its record types, no member more and none fewer"
            );
        }
        assert_eq!(declared().structs.len(), 4, "and no class besides");
    }

    fn bench(guid: u32) -> CEntityDefAny {
        CEntityDefAny::Base(CEntityDef {
            archetype_name: HashedString::new("prop_bench_01a"),
            flags: 32,
            guid,
            position: Vec3 {
                x: 120.5,
                y: -40.25,
                z: 30.0,
            },
            rotation: Vec4 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            },
            scale_xy: 1.0,
            scale_z: 1.0,
            parent_index: -1,
            lod_dist: 60.0,
            ..Default::default()
        })
    }

    fn hills() -> CMapData {
        CMapData {
            name: HashedString::new("hills"),
            entities: vec![bench(1)],
            ..Default::default()
        }
    }

    /// The members `inst` states, by name hash, in the order it states them.
    fn stated(inst: &MetaInstance) -> Vec<u32> {
        inst.fields.iter().map(|(n, _)| name_hash(n)).collect()
    }

    fn hashes(names: &[&str]) -> Vec<u32> {
        names.iter().map(|n| name_hash(n)).collect()
    }

    fn tree(bytes: &[u8]) -> MetaInstance {
        MetaDocument::parse(&SchemaLibrary::default(), bytes.to_vec())
            .expect("the save reads")
            .instance
    }

    fn elements<'a>(root: &'a MetaInstance, array: &str) -> Vec<&'a MetaInstance> {
        let key = name_hash(array);
        let found = root.fields.iter().find(|(n, _)| name_hash(n) == key);
        match found {
            Some((_, Value::Array(items))) => items
                .iter()
                .filter_map(|v| match v {
                    Value::Struct(inst) | Value::Pointer(Some(inst)) => Some(&**inst),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn a_map_built_from_nothing_saves_as_pso_and_reads_back_as_built() {
        let built = hills();
        let saved = built.to_bytes().expect("a map built from nothing saves");
        assert_eq!(detect_binary_format(&saved), Some(BinaryFormat::Pso));

        let back = CMapData::from_bytes(&saved).expect("and reopens");
        assert!(back.name == "hills");
        assert_eq!(back.entities.len(), 1);
        let entity = &back.entities[0];
        assert!(matches!(entity, CEntityDefAny::Base(_)));
        assert!(entity.archetype_name == "prop_bench_01a");
        assert_eq!(
            (entity.flags, entity.guid, entity.parent_index),
            (32, 1, -1)
        );
        assert_eq!(entity.position, built.entities[0].position);
        assert_eq!(entity.rotation, built.entities[0].rotation);
        assert_eq!(
            (entity.scale_xy, entity.scale_z, entity.lod_dist),
            (1.0, 1.0, 60.0)
        );
        assert_eq!(
            (
                entity.ambient_occlusion_multiplier,
                entity.artificial_ambient_occlusion
            ),
            (255, 255),
            "the engine's inits are stated"
        );
        assert_eq!(back.to_bytes().expect("saves again"), saved);

        let root = tree(&saved);
        assert_eq!(
            sorted_hashes(stated(&root)),
            sorted_hashes(hashes(&map_members_declared()))
        );
        let entities = elements(&root, "entities");
        assert_eq!(entities.len(), 1);
        assert_eq!(
            sorted_hashes(stated(entities[0])),
            sorted_hashes(hashes(CEntityDef::MEMBERS))
        );
    }

    fn sorted_hashes(mut hashes: Vec<u32>) -> Vec<u32> {
        hashes.sort_unstable();
        hashes
    }

    #[test]
    fn a_type_file_built_from_nothing_saves_as_pso_and_reads_back_as_built() {
        let built = CMapTypes {
            name: HashedString::new("hills_types"),
            archetypes: vec![CBaseArchetypeDefAny::Base(CBaseArchetypeDef {
                lod_dist: 80.0,
                bb_min: Vec3 {
                    x: -1.0,
                    y: -2.0,
                    z: 0.0,
                },
                bb_max: Vec3 {
                    x: 1.0,
                    y: 2.0,
                    z: 3.0,
                },
                bs_radius: 2.5,
                name: HashedString::new("prop_bench_01a"),
                asset_name: HashedString::new("prop_bench_01a"),
                ..Default::default()
            })],
            ..Default::default()
        };
        let saved = built
            .to_bytes()
            .expect("a type file built from nothing saves");
        assert_eq!(detect_binary_format(&saved), Some(BinaryFormat::Pso));

        let back = CMapTypes::from_bytes(&saved).expect("and reopens");
        assert!(back.name == "hills_types");
        assert_eq!(back.archetypes.len(), 1);
        let archetype = &back.archetypes[0];
        assert!(matches!(archetype, CBaseArchetypeDefAny::Base(_)));
        assert!(archetype.name == "prop_bench_01a" && archetype.asset_name == "prop_bench_01a");
        assert_eq!(archetype.bb_max, built.archetypes[0].bb_max);
        assert_eq!((archetype.lod_dist, archetype.bs_radius), (80.0, 2.5));
        assert_eq!(
            archetype.hd_texture_dist, 5.0,
            "the engine's init is stated"
        );
        assert_eq!(back.to_bytes().expect("saves again"), saved);
    }

    #[test]
    fn a_map_built_from_nothing_is_the_fixed_point_of_its_own_xml() {
        for saved in [
            hills().to_bytes().expect("saves"),
            CMapData::default().to_bytes().expect("an empty map saves"),
            CMapTypes::default()
                .to_bytes()
                .expect("an empty type file saves"),
        ] {
            let converge = || canonical::converge(&SchemaLibrary::default(), &saved);
            for converged in [converge(), with_public_names_only(converge)] {
                let converged = converged.expect("its XML compiles back");
                assert!(!converged.values_moved, "no value is lost on the way");
                assert_eq!(
                    converged.bytes, saved,
                    "the file its XML compiles to is the file itself"
                );
            }
        }
    }

    #[test]
    fn an_entity_built_into_an_empty_map_saves_what_building_the_map_whole_saves() {
        let empty = CMapData {
            name: HashedString::new("hills"),
            ..Default::default()
        };
        let mut reopened =
            CMapData::from_bytes(&empty.to_bytes().expect("saves")).expect("reopens");
        reopened.entities.push(bench(1));
        assert_eq!(
            reopened.to_bytes().expect("the first entity saves"),
            hills().to_bytes().expect("saves")
        );
    }

    /// The world classes with a member of `CEntityDef` that no record types.
    fn with_a_member_no_record_types() -> SchemaLibrary {
        let tint = r#"<u32 name="tintValue" init="0"/>"#;
        assert!(WORLD_TYPES.contains(tint));
        let psc = WORLD_TYPES.replace(
            tint,
            &format!(r#"{tint}<u32 name="aMemberNoRecordTypes"/>"#),
        );
        build_library([("wider.psc", psc.as_bytes())]).expect("the wider classes parse")
    }

    /// A map of `library`'s classes holding one bench, as `format`.
    fn wider_map(format: BinaryFormat, entities: usize) -> Vec<u8> {
        let library = with_a_member_no_record_types();
        let mut root = MetaInstance::new("CMapData");
        crate::meta::typed::conform(&library, &mut root).expect("conforms");
        let mut entity = MetaInstance::new("CEntityDef");
        crate::meta::typed::conform(&library, &mut entity).expect("conforms");
        // ⚠ By hash: `conform` spells a member as the index of names does, which has no name for this one.
        let mut put = |member: &str, value: Value| {
            let key = name_hash(member);
            let slot = entity.fields.iter_mut().find(|(n, _)| name_hash(n) == key);
            *slot.map(|(_, v)| v).expect("the class has the member") = value;
        };
        put("parentIndex", Value::Int(-1));
        put("aMemberNoRecordTypes", Value::UInt(9));
        root.set(
            "entities",
            Value::Array(
                (0..entities)
                    .map(|_| Value::Pointer(Some(Box::new(entity.clone()))))
                    .collect(),
            ),
        );
        let layout = canonical::layout_for(&library, &root, format, None).expect("lays out");
        write_meta_binary(format, Some(&layout), &root).expect("encodes")
    }

    #[test]
    fn an_entity_built_into_a_file_states_the_members_that_file_states() {
        for format in [BinaryFormat::Pso, BinaryFormat::Rbm] {
            let mut map = CMapData::from_bytes(&wider_map(format, 1)).expect("opens");
            map.entities.push(bench(2));
            let saved = map.to_bytes().expect("an entity built here saves");
            assert_eq!(detect_binary_format(&saved), Some(format));

            let root = tree(&saved);
            let entities = elements(&root, "entities");
            assert_eq!(entities.len(), 2);
            assert_eq!(
                stated(entities[1]),
                stated(entities[0]),
                "the entity built here states what the file's own class states"
            );
            let extra = name_hash("aMemberNoRecordTypes");
            let held = |inst: &MetaInstance| {
                inst.fields
                    .iter()
                    .find(|(n, _)| name_hash(n) == extra)
                    .map(|(_, v)| v.clone())
            };
            assert_eq!(held(entities[0]), Some(Value::UInt(9)));
            assert_eq!(
                held(entities[1]),
                Some(Value::UInt(0)),
                "at its empty value"
            );
            let back = CMapData::from_bytes(&saved).expect("reopens");
            assert_eq!(back.entities[1].guid, 2);
        }
    }

    #[test]
    fn an_entity_built_into_an_rbm_file_that_states_no_entity_class_is_refused_for_its_signature() {
        let mut map = CMapData::from_bytes(&wider_map(BinaryFormat::Rbm, 0)).expect("opens");
        map.entities.push(bench(1));
        let refused = map
            .to_bytes()
            .expect_err("nothing states the signature of the class");
        let why = refused.to_string();
        assert!(
            why.contains("signature") && why.contains("CEntityDef"),
            "{why}"
        );

        // The same file as PSO states no signature, so the class is laid out as it is declared here.
        let mut map = CMapData::from_bytes(&wider_map(BinaryFormat::Pso, 0)).expect("opens");
        map.entities.push(bench(1));
        let saved = map.to_bytes().expect("a PSO takes the declared class");
        assert_eq!(
            CMapData::from_bytes(&saved)
                .expect("reopens")
                .entities
                .len(),
            1
        );
    }

    /// The file `xml` compiles to over `base`, typed as a project's source is: by the base's own table and no snapshot.
    fn compiled(base: &[u8], xml: &str) -> crate::meta::error::Result<Vec<u8>> {
        let empty = SchemaLibrary::default();
        let layout = MetaDocument::parse(&empty, base.to_vec())
            .expect("the base reads")
            .layout
            .expect("a binary document has a layout");
        let format = detect_binary_format(base).expect("the base is binary");
        let typed = canonical::typed_by(&empty, Some(&layout));
        canonical::import(&typed, xml.as_bytes(), format, None)
    }

    #[test]
    fn an_xml_that_adds_the_first_entity_compiles_over_a_pso_base_and_never_over_an_rbm_one() {
        let saved = hills().to_bytes().expect("saves");
        let document = MetaDocument::parse(&SchemaLibrary::default(), saved).expect("reads");
        let xml = canonical::export(
            &document.instance,
            document.layout.as_ref().expect("a layout"),
        )
        .expect("exports");

        let over_pso = compiled(&wider_map(BinaryFormat::Pso, 0), &xml)
            .expect("a PSO base takes the class as it is declared here");
        assert_eq!(
            CMapData::from_bytes(&over_pso)
                .expect("reopens")
                .entities
                .len(),
            1
        );
        // ⚠ An RBM class laid out from the declaration would be written with a zero signature, which no shipped file carries.
        compiled(&wider_map(BinaryFormat::Rbm, 0), &xml)
            .expect_err("an RBM base that states no entity class has no signature for one");
    }

    #[test]
    fn an_entity_built_here_that_is_its_own_parent_is_refused_and_not_saved() {
        let mut map = hills();
        map.entities[0].parent_index = 0;
        let refused = map.to_bytes().expect_err("an index the engine trusts");
        assert!(matches!(refused, crate::Error::Refused { .. }), "{refused}");
        assert!(
            refused.to_string().contains("its own LOD parent"),
            "{refused}"
        );
    }

    #[test]
    fn a_world_save_leaves_its_types_to_no_later_save() {
        let none_left = || super::super::SAVING.with(|saving| saving.borrow().is_none());
        hills().to_bytes().expect("saves");
        assert!(none_left(), "after a save that wrote");
        let mut map = CMapData::from_bytes(&wider_map(BinaryFormat::Rbm, 0)).expect("opens");
        map.entities.push(bench(1));
        map.to_bytes().expect_err("no signature to write");
        assert!(none_left(), "after a save that was refused");
    }

    #[test]
    fn a_member_set_on_an_entity_built_into_a_file_whose_class_lacks_it_is_refused() {
        let tint = r#"<u32 name="tintValue" init="0"/>"#;
        assert!(WORLD_TYPES.contains(tint));
        let narrower = build_library([("narrower.psc", WORLD_TYPES.replace(tint, "").as_bytes())])
            .expect("the narrower classes parse");
        let mut root = MetaInstance::new("CMapData");
        crate::meta::typed::conform(&narrower, &mut root).expect("conforms");
        let mut held = MetaInstance::new("CEntityDef");
        crate::meta::typed::conform(&narrower, &mut held).expect("conforms");
        let parent = name_hash("parentIndex");
        for (name, value) in &mut held.fields {
            if name_hash(name) == parent {
                *value = Value::Int(-1);
            }
        }
        root.set(
            "entities",
            Value::Array(vec![Value::Pointer(Some(Box::new(held)))]),
        );
        let layout =
            canonical::layout_for(&narrower, &root, BinaryFormat::Pso, None).expect("lays out");
        let file = write_meta_binary(BinaryFormat::Pso, Some(&layout), &root).expect("encodes");

        let mut map = CMapData::from_bytes(&file).expect("opens");
        map.entities.push(bench(2));
        map.to_bytes()
            .expect("a tint at the engine's init needs no member to hold it");
        map.entities[1].tint_value = 5;
        let refused = map
            .to_bytes()
            .expect_err("the file's own class has no tint to hold 5");
        let why = refused.to_string();
        assert!(
            why.contains("tintValue") && why.contains("no such member"),
            "{why}"
        );
    }

    #[test]
    fn a_copy_whose_member_the_declared_class_lacks_is_refused_rather_than_dropped() {
        let donor = CMapData::from_bytes(&wider_map(BinaryFormat::Pso, 1)).expect("opens");
        let mut map = hills();
        map.entities.push(donor.entities[0].clone());
        let refused = map.to_bytes().expect_err("the copy holds a member to drop");
        let why = refused.to_string();
        assert!(
            why.contains("would drop") && why.contains("CEntityDef"),
            "{why}"
        );
    }
    /// The world classes with an entity that holds `extensions`, and one class of extension that no record types.
    fn with_an_extension() -> SchemaLibrary {
        let tint = r#"<u32 name="tintValue" init="0"/>"#;
        assert!(WORLD_TYPES.contains(tint));
        let extensions = r#"<array name="extensions" type="atArray"><pointer type="rage::fwExtensionDef" policy="owner"/></array>"#;
        let extension = r#"<structdef type="AnExtensionNoRecordTypes"><u32 name="aMemberNoRecordTypes"/></structdef>"#;
        let psc = WORLD_TYPES
            .replace(tint, &format!("{tint}{extensions}"))
            .replace("</ParserSchema>", &format!("{extension}</ParserSchema>"));
        build_library([("extended.psc", psc.as_bytes())]).expect("the extended classes parse")
    }

    /// A map of `library`'s classes as `format`, holding one entity that `edit` changed.
    fn map_of(
        library: &SchemaLibrary,
        format: BinaryFormat,
        edit: impl FnOnce(&mut dyn FnMut(&str, Value)),
    ) -> Vec<u8> {
        let mut root = MetaInstance::new("CMapData");
        crate::meta::typed::conform(library, &mut root).expect("conforms");
        let mut entity = MetaInstance::new("CEntityDef");
        crate::meta::typed::conform(library, &mut entity).expect("conforms");
        // ⚠ By hash: `conform` spells a member as the index of names does, which lacks some.
        let mut put = |member: &str, value: Value| {
            let key = name_hash(member);
            let slot = entity.fields.iter_mut().find(|(n, _)| name_hash(n) == key);
            *slot.map(|(_, v)| v).expect("the class has the member") = value;
        };
        put("parentIndex", Value::Int(-1));
        edit(&mut put);
        root.set(
            "entities",
            Value::Array(vec![Value::Pointer(Some(Box::new(entity)))]),
        );
        let layout = canonical::layout_for(library, &root, format, None).expect("lays out");
        write_meta_binary(format, Some(&layout), &root).expect("encodes")
    }

    /// A map whose one entity holds `extensions` extensions, as `format`.
    fn extended_map(format: BinaryFormat, extensions: usize) -> Vec<u8> {
        let library = with_an_extension();
        let mut extension = MetaInstance::new("AnExtensionNoRecordTypes");
        crate::meta::typed::conform(&library, &mut extension).expect("conforms");
        let held = (0..extensions)
            .map(|_| Value::Pointer(Some(Box::new(extension.clone()))))
            .collect();
        map_of(&library, format, |put| {
            put("extensions", Value::Array(held))
        })
    }

    /// The name a tree read with no schema gives `name`, which the index of names may lack.
    fn spelt(name: &str) -> String {
        crate::meta::binary::hash_index::default_hash_index().name_or_literal(name_hash(name))
    }

    #[test]
    fn a_copy_that_holds_a_record_of_a_class_this_file_lacks_is_refused_by_name_and_never_by_the_schema(
    ) {
        let extension = spelt("AnExtensionNoRecordTypes");
        for format in [BinaryFormat::Pso, BinaryFormat::Rbm] {
            let donor = CMapData::from_bytes(&extended_map(format, 1)).expect("opens");

            // ⚠ The entity is of a class this file states, and the extension it holds is of one it does not.
            let mut map = CMapData::from_bytes(&extended_map(format, 0)).expect("opens");
            map.entities.push(donor.entities[0].clone());
            let refused = map
                .to_bytes()
                .expect_err("nothing states or declares the class of the extension");
            assert!(matches!(refused, crate::Error::Save { .. }), "{refused}");
            let why = refused.to_string();
            assert!(why.contains(&format!("`{extension}` copied here")), "{why}");
            assert!(!why.contains("synth"), "{format:?}: {why}");
            assert_eq!(
                why.contains("signature"),
                format == BinaryFormat::Rbm,
                "{why}"
            );
            // Beside a record built here, which is conformed and not refused, the copy is still refused by name.
            map.entities.push(bench(7));
            let why = map.to_bytes().expect_err("the same copy").to_string();
            assert!(why.contains(&format!("`{extension}` copied here")), "{why}");

            // A file that holds an extension states its class, and takes the copy with what it holds.
            let mut map = CMapData::from_bytes(&extended_map(format, 1)).expect("opens");
            map.entities.push(donor.entities[0].clone());
            let saved = map
                .to_bytes()
                .expect("the class of the extension is stated");
            let root = tree(&saved);
            let entities = elements(&root, "entities");
            assert_eq!(entities.len(), 2);
            assert_eq!(elements(entities[1], "extensions").len(), 1);
        }
    }

    #[test]
    fn a_copy_whose_member_this_files_own_class_lacks_is_refused_rather_than_dropped() {
        let member = spelt("aMemberNoRecordTypes");
        for format in [BinaryFormat::Pso, BinaryFormat::Rbm] {
            let donor = CMapData::from_bytes(&wider_map(format, 1)).expect("opens");
            // ⚠ This file states an entity class too, without the member the donor's entity holds.
            let narrower = map_of(declared(), format, |_| {});
            let mut map = CMapData::from_bytes(&narrower).expect("opens");
            map.entities.push(donor.entities[0].clone());
            let said = format!("a `CEntityDef` here holds `{member}`");
            for beside_a_record_built_here in [false, true] {
                if beside_a_record_built_here {
                    map.entities.push(bench(7));
                }
                let refused = map.to_bytes().expect_err("the copy holds a member to drop");
                assert!(matches!(refused, crate::Error::Save { .. }), "{refused}");
                let why = refused.to_string();
                assert!(why.contains(&said) && why.contains("would drop"), "{why}");
                assert!(why.contains("states `CEntityDef` without it"), "{why}");
            }
            // The copy taken out, the file saves, with the entity built here as its own class states one.
            map.entities.remove(1);
            let saved = map.to_bytes().expect("nothing is left to drop");
            let root = tree(&saved);
            let entities = elements(&root, "entities");
            assert_eq!(entities.len(), 2);
            assert_eq!(stated(entities[1]), stated(entities[0]));

            // A copy between two files that state the same members is no such copy.
            let mut twin = CMapData::from_bytes(&wider_map(format, 1)).expect("opens");
            twin.entities.push(donor.entities[0].clone());
            let saved = twin.to_bytes().expect("the member has its place");
            let extra = name_hash("aMemberNoRecordTypes");
            for entity in elements(&tree(&saved), "entities") {
                let held = entity.fields.iter().find(|(n, _)| name_hash(n) == extra);
                assert_eq!(held.map(|(_, v)| v), Some(&Value::UInt(9)));
            }
        }
    }

    #[test]
    fn a_class_and_a_member_no_table_states_are_found_in_a_map_as_in_an_array() {
        let saved = hills().to_bytes().expect("saves");
        let document = MetaDocument::parse(&SchemaLibrary::default(), saved).expect("reads");
        let layout = document.layout.as_ref().expect("a layout");
        assert_eq!(first_class_unstated(layout, &document.instance), None);
        assert_eq!(first_member_unstated(layout, &document.instance), None);

        let stranger = || Value::Struct(Box::new(MetaInstance::new("AClassNoTableStates")));
        let entity = |root: &MetaInstance| match root.get("entities") {
            Some(Value::Array(items)) => items[0].clone(),
            other => panic!("the map holds its entity, and `entities` is {other:?}"),
        };
        let found = Some("AClassNoTableStates");
        for held in [
            Value::Map(vec![(Value::UInt(1), stranger())]),
            Value::Map(vec![(stranger(), Value::UInt(1))]),
            Value::Array(vec![stranger()]),
        ] {
            let mut root = document.instance.clone();
            // In a member the class states, so no member is one the table lacks.
            root.set("entities", held);
            assert_eq!(first_class_unstated(layout, &root), found);
            assert_eq!(first_member_unstated(layout, &root), None);
        }

        // A member the table lacks, on a record held in a map and in an array.
        let mut wider = entity(&document.instance);
        let (Value::Pointer(Some(inst)) | Value::Struct(inst)) = &mut wider else {
            panic!("the entity is a record");
        };
        inst.set("aMemberNoTableStates", Value::UInt(9));
        let class = inst.type_name.clone();
        for held in [
            Value::Map(vec![(Value::UInt(1), wider.clone())]),
            Value::Array(vec![entity(&document.instance), wider.clone()]),
        ] {
            let mut root = document.instance.clone();
            root.set("entities", held);
            assert_eq!(first_class_unstated(layout, &root), None);
            assert_eq!(
                first_member_unstated(layout, &root),
                Some((class.as_str(), "aMemberNoTableStates"))
            );
        }
    }

    #[test]
    fn what_a_text_holds_is_judged_by_no_table_and_what_a_file_built_here_holds_is() {
        // ⚠ A text read with no schema calls a record by the element that holds it, a name no class has.
        let mut room = MetaInstance::new("Item");
        room.set("name", Value::String("limbo".into()));
        let mut root = MetaInstance::new("CMapTypes");
        root.set(
            "archetypes",
            Value::Array(vec![Value::Struct(Box::new(room))]),
        );

        let text = Arrival {
            layout: None,
            format: SourceFormat::Xml,
        };
        let judged = Saving::of(Some(&text)).refuse_what_has_no_place(&root);
        assert_eq!(judged, Ok(()), "a text has no table to lack a class");

        let why = (Saving::of(None).refuse_what_has_no_place(&root))
            .expect_err("no class of that name is declared for a file built here");
        assert!(why.contains("`Item` copied here"), "{why}");
    }

    /// The one entity of the file `map` saved as, as a record that carries that file's tree.
    fn carried_entity(map: &CMapData) -> CEntityDefAny {
        let opened = CMapData::from_bytes(&map.to_bytes().expect("saves")).expect("reopens");
        opened.entities[0].clone()
    }

    #[test]
    fn the_last_entity_of_a_map_is_taken_out_and_put_back_as_it_was() {
        let whole = hills().to_bytes().expect("saves");
        let mut map = CMapData::from_bytes(&whole).expect("opens");
        let taken = map.entities.remove(0);
        let without = map.to_bytes().expect("a map with no entity saves");

        // ⚠ The file states no entity class once it holds none, and the record taken out still carries the tree it was read from.
        let mut again = CMapData::from_bytes(&without).expect("reopens");
        assert!(again.entities.is_empty());
        again.entities.push(taken);
        let saved = again.to_bytes().expect("the entity taken out is put back");
        assert_eq!(saved, whole, "and the file is the one it was");
        assert_eq!(
            CMapData::from_bytes(&saved)
                .expect("reopens")
                .to_bytes()
                .expect("saves again"),
            saved
        );
    }

    #[test]
    fn a_map_and_a_type_file_made_from_nothing_take_their_first_record_as_a_copy() {
        let empty = CMapData {
            name: HashedString::new("hills"),
            ..Default::default()
        };
        let mut map = CMapData::from_bytes(&empty.to_bytes().expect("saves")).expect("reopens");
        map.entities.push(carried_entity(&hills()));
        assert_eq!(
            map.to_bytes().expect("the first entity is a copy"),
            hills().to_bytes().expect("saves"),
            "what building the map whole saves"
        );

        let bench = CBaseArchetypeDefAny::Base(CBaseArchetypeDef {
            lod_dist: 80.0,
            name: HashedString::new("prop_bench_01a"),
            ..Default::default()
        });
        let whole = CMapTypes {
            archetypes: vec![bench],
            ..Default::default()
        };
        let whole = whole.to_bytes().expect("saves");
        let copied = CMapTypes::from_bytes(&whole).expect("reopens").archetypes[0].clone();
        let empty = CMapTypes::default().to_bytes().expect("saves");
        let mut types = CMapTypes::from_bytes(&empty).expect("reopens");
        types.archetypes.push(copied);
        assert_eq!(
            types.to_bytes().expect("the first archetype is a copy"),
            whole
        );
    }

    #[test]
    fn a_copy_put_into_an_rbm_file_that_states_no_class_for_it_is_refused_for_its_signature() {
        let donor = CMapData::from_bytes(&wider_map(BinaryFormat::Rbm, 1)).expect("opens");
        let mut map = CMapData::from_bytes(&wider_map(BinaryFormat::Rbm, 0)).expect("opens");
        map.entities.push(donor.entities[0].clone());
        let why = map
            .to_bytes()
            .expect_err("nothing states the signature of the class")
            .to_string();
        assert!(
            why.contains("signature") && why.contains("CEntityDef") && !why.contains("synth"),
            "{why}"
        );
        assert!(
            !why.contains("as a copy"),
            "a copy is what was refused, so the sentence does not advise one: {why}"
        );
    }

    #[test]
    fn a_copy_put_into_a_pso_file_that_states_no_class_for_it_is_conformed_and_never_narrowed() {
        // A copy that holds a member the declared class lacks would lose it.
        let wider = CMapData::from_bytes(&wider_map(BinaryFormat::Pso, 1)).expect("opens");
        let empty = CMapData::default().to_bytes().expect("saves");
        let mut map = CMapData::from_bytes(&empty).expect("reopens");
        map.entities.push(wider.entities[0].clone());
        let why = map
            .to_bytes()
            .expect_err("the copy holds a member to drop")
            .to_string();
        assert!(
            why.contains("would drop") && why.contains("CEntityDef"),
            "{why}"
        );

        // A copy that lacks a member the declared class has states it at the engine's init.
        let ao = r#"<s32 name="artificialAmbientOcclusion" init="255"/>"#;
        assert!(WORLD_TYPES.contains(ao));
        let narrower = build_library([("narrower.psc", WORLD_TYPES.replace(ao, "").as_bytes())])
            .expect("the narrower classes parse");
        let mut root = MetaInstance::new("CMapData");
        crate::meta::typed::conform(&narrower, &mut root).expect("conforms");
        let mut held = MetaInstance::new("CEntityDef");
        crate::meta::typed::conform(&narrower, &mut held).expect("conforms");
        let parent = name_hash("parentIndex");
        for (name, value) in &mut held.fields {
            if name_hash(name) == parent {
                *value = Value::Int(-1);
            }
        }
        root.set(
            "entities",
            Value::Array(vec![Value::Pointer(Some(Box::new(held)))]),
        );
        let layout =
            canonical::layout_for(&narrower, &root, BinaryFormat::Pso, None).expect("lays out");
        let file = write_meta_binary(BinaryFormat::Pso, Some(&layout), &root).expect("encodes");
        let narrow = CMapData::from_bytes(&file).expect("opens");

        let mut map = CMapData::from_bytes(&empty).expect("reopens");
        map.entities.push(narrow.entities[0].clone());
        let saved = map.to_bytes().expect("the copy takes the declared class");
        let root = tree(&saved);
        let entities = elements(&root, "entities");
        assert_eq!(
            sorted_hashes(stated(entities[0])),
            sorted_hashes(hashes(CEntityDef::MEMBERS))
        );
        assert_eq!(
            CMapData::from_bytes(&saved).expect("reopens").entities[0].artificial_ambient_occlusion,
            255
        );
    }
}
