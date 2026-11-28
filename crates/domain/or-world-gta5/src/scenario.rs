//! Scenario regions as the game reads them: each point's scenario type and
//! model set, which the point holds as indices into its region's look-ups,
//! named against the manifests that declare them.
//!
//! ⚠ `ModelSetId` indexes the vehicle model sets when the point's type spawns
//! a vehicle and the ped model sets otherwise, so a point's model set has no
//! name until the scenario manifest says what its type spawns.

use std::collections::HashMap;
use std::fmt;

use or_io::hash::literal_string_hash;
use or_io::meta::instance::HashedString;
use or_io::meta::{CScenarioPointRegion, MetaInstance, Value};

use crate::field;

/// Where a point sits: the region's own list, or a cluster's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointAt {
    pub cluster: Option<usize>,
    pub index: usize,
}

impl fmt::Display for PointAt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.cluster {
            None => write!(f, "the region's point {}", self.index),
            Some(c) => write!(f, "cluster {c}'s point {}", self.index),
        }
    }
}

/// A point with its scenario type and model set named as the manifests declare them.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedPoint {
    pub at: PointAt,
    pub scenario_type: HashedString,
    pub vehicle: bool,
    /// `None` where the point names the look-ups' `none`.
    pub model_set: Option<HashedString>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ScenarioError {
    #[error("{at} names scenario type {index}, past the region's {len} type names")]
    TypeOutOfRange { at: PointAt, index: u8, len: usize },
    #[error("{at} names scenario type {}, which no scenario manifest declares", .name.to_xml_text())]
    TypeUndeclared { at: PointAt, name: HashedString },
    #[error("{at} names {table} model set {index}, past the region's {len}")]
    ModelSetOutOfRange {
        at: PointAt,
        table: &'static str,
        index: u8,
        len: usize,
    },
    #[error("{at} names model set {}, which no model-set manifest declares", .name.to_xml_text())]
    ModelSetUndeclared { at: PointAt, name: HashedString },
    #[error("the region has no {0}")]
    NoSuchPoint(PointAt),
    #[error("the region already names 256 scenario types, all a point's index can reach")]
    TypeNamesFull,
}

/// A scenario type or type group the scenario manifests declare.
#[derive(Debug, Clone)]
struct Declared {
    name: HashedString,
    vehicle: bool,
    /// A group's member types, whose kind is the group's.
    members: Vec<u32>,
}

/// What the scenario-info and model-set manifests declare, for naming a region's points.
#[derive(Debug, Clone, Default)]
pub struct ScenarioManifests {
    types: HashMap<u32, Declared>,
    model_sets: HashMap<u32, HashedString>,
}

const SCENARIOS: u32 = literal_string_hash(b"Scenarios");
const SCENARIO_TYPE_GROUPS: u32 = literal_string_hash(b"ScenarioTypeGroups");
const TYPES: u32 = literal_string_hash(b"Types");
const SCENARIO_TYPE: u32 = literal_string_hash(b"ScenarioType");
const MODEL_SETS: u32 = literal_string_hash(b"ModelSets");
const NAME: u32 = literal_string_hash(b"Name");

/// The scenario infos whose points take a vehicle model set.
const VEHICLE_INFOS: [&str; 2] = ["CScenarioVehicleInfo", "CScenarioVehicleParkInfo"];

fn hashed(value: &Value) -> Option<HashedString> {
    match value {
        Value::Hash(h) => Some(h.clone()),
        Value::String(s) if !s.is_empty() => Some(HashedString::new(s.as_str())),
        Value::UInt(v) => u32::try_from(*v).ok().map(HashedString::from_hash),
        Value::Pointer(Some(inst)) | Value::Struct(inst) => hashed(field::get(inst, NAME)?),
        _ => None,
    }
}

impl ScenarioManifests {
    /// Adds what a `CScenarioInfoManager` or `CAmbientModelSets` declares; another root adds nothing.
    pub fn add(&mut self, manifest: &MetaInstance) {
        if field::is_type(manifest, "CScenarioInfoManager") {
            for info in field::structs_at(manifest, SCENARIOS) {
                let Some(name) = field::get(info, NAME).and_then(hashed) else {
                    continue;
                };
                let vehicle = VEHICLE_INFOS.iter().any(|t| field::is_type(info, t));
                self.types.insert(
                    name.hash,
                    Declared {
                        name,
                        vehicle,
                        members: Vec::new(),
                    },
                );
            }
            for group in field::structs_at(manifest, SCENARIO_TYPE_GROUPS) {
                let Some(name) = field::get(group, NAME).and_then(hashed) else {
                    continue;
                };
                let members = field::structs_at(group, TYPES)
                    .filter_map(|entry| field::get(entry, SCENARIO_TYPE).and_then(hashed))
                    .map(|h| h.hash)
                    .collect();
                self.types.insert(
                    name.hash,
                    Declared {
                        name,
                        vehicle: false,
                        members,
                    },
                );
            }
        } else if field::is_type(manifest, "CAmbientModelSets") {
            for set in field::structs_at(manifest, MODEL_SETS) {
                if let Some(name) = field::get(set, NAME).and_then(hashed) {
                    self.model_sets.insert(name.hash, name);
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.types.is_empty() && self.model_sets.is_empty()
    }

    /// The declared scenario type or group, and whether its points take a vehicle model set.
    fn scenario_type(&self, hash: u32) -> Option<(&HashedString, bool)> {
        let declared = self.types.get(&hash)?;
        let vehicle = declared.vehicle
            || declared
                .members
                .iter()
                .any(|m| self.types.get(m).is_some_and(|d| d.vehicle));
        Some((&declared.name, vehicle))
    }

    /// Every point of `region`, the region's own then each cluster's, named.
    pub fn name_points(
        &self,
        region: &CScenarioPointRegion,
    ) -> Result<Vec<NamedPoint>, ScenarioError> {
        let look_ups = &region.look_ups;
        let own = region.points.my_points.iter().enumerate().map(|(i, p)| {
            let at = PointAt {
                cluster: None,
                index: i,
            };
            (at, p)
        });
        let clustered = region.clusters.iter().enumerate().flat_map(|(c, cluster)| {
            cluster
                .points
                .my_points
                .iter()
                .enumerate()
                .map(move |(i, p)| {
                    let at = PointAt {
                        cluster: Some(c),
                        index: i,
                    };
                    (at, p)
                })
        });
        own.chain(clustered)
            .map(|(at, point)| {
                let named = region
                    .scenario_type(point)
                    .ok_or(ScenarioError::TypeOutOfRange {
                        at,
                        index: point.i_type,
                        len: look_ups.type_names.len(),
                    })?;
                let (scenario_type, vehicle) = self.scenario_type(named.hash).ok_or_else(|| {
                    ScenarioError::TypeUndeclared {
                        at,
                        name: named.clone(),
                    }
                })?;
                let (table, sets) = if vehicle {
                    ("vehicle", &look_ups.vehicle_model_set_names)
                } else {
                    ("ped", &look_ups.ped_model_set_names)
                };
                let set = sets.get(usize::from(point.model_set_id)).ok_or(
                    ScenarioError::ModelSetOutOfRange {
                        at,
                        table,
                        index: point.model_set_id,
                        len: sets.len(),
                    },
                )?;
                let model_set = if *set == "none" {
                    None
                } else {
                    let declared = self.model_sets.get(&set.hash).ok_or_else(|| {
                        ScenarioError::ModelSetUndeclared {
                            at,
                            name: set.clone(),
                        }
                    })?;
                    Some(declared.clone())
                };
                Ok(NamedPoint {
                    at,
                    scenario_type: scenario_type.clone(),
                    vehicle,
                    model_set,
                })
            })
            .collect()
    }
}

/// Gives the point at `at` the scenario type `name`, adding the name to the region's look-ups
/// when no point names it yet.
///
/// ⚠ A name the look-ups lack grows `TypeNames`, which a save may lay out afresh where the
/// document's own allocation cannot hold it.
pub fn set_scenario_type(
    region: &mut CScenarioPointRegion,
    at: PointAt,
    name: HashedString,
) -> Result<(), ScenarioError> {
    let points = match at.cluster {
        None => Some(&mut region.points.my_points),
        Some(c) => region.clusters.get_mut(c).map(|k| &mut k.points.my_points),
    };
    let point = points
        .and_then(|p| p.get_mut(at.index))
        .ok_or(ScenarioError::NoSuchPoint(at))?;
    let names = &mut region.look_ups.type_names;
    point.i_type = match names.iter().position(|n| n.hash == name.hash) {
        Some(index) => index as u8,
        None if names.len() > usize::from(u8::MAX) => return Err(ScenarioError::TypeNamesFull),
        None => {
            names.push(name);
            (names.len() - 1) as u8
        }
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_io::meta::{CScenarioPoint, CScenarioPointCluster};

    fn named(name: &str) -> Value {
        Value::Hash(HashedString::new(name))
    }

    fn info(type_name: &str, name: &str) -> Value {
        let mut inst = MetaInstance::new(type_name);
        inst.set("Name", named(name));
        Value::Pointer(Some(Box::new(inst)))
    }

    fn manifests() -> ScenarioManifests {
        let mut infos = MetaInstance::new("CScenarioInfoManager");
        infos.set(
            "Scenarios",
            Value::Array(vec![
                info("CScenarioPlayAnimsInfo", "WORLD_HUMAN_SMOKING"),
                info(
                    "CScenarioVehicleParkInfo",
                    "PROP_HUMAN_PARKING_METER_VEHICLE",
                ),
            ]),
        );
        let mut entry = MetaInstance::new("CScenarioTypeGroupEntry");
        entry.set("ScenarioType", named("PROP_HUMAN_PARKING_METER_VEHICLE"));
        let mut group = MetaInstance::new("CScenarioTypeGroup");
        group.set("Name", named("PARKING_GROUP"));
        group.set("Types", Value::Array(vec![Value::Struct(Box::new(entry))]));
        infos.set(
            "ScenarioTypeGroups",
            Value::Array(vec![Value::Pointer(Some(Box::new(group)))]),
        );
        let mut sets = MetaInstance::new("CAmbientModelSets");
        sets.set(
            "ModelSets",
            Value::Array(vec![
                info("CAmbientModelSet", "SMOKERS"),
                info("CAmbientModelSet", "PARKED_CARS"),
            ]),
        );
        let mut m = ScenarioManifests::default();
        m.add(&infos);
        m.add(&sets);
        m.add(&MetaInstance::new("CMapData"));
        m
    }

    fn point(i_type: u8, model_set_id: u8) -> CScenarioPoint {
        let mut p = CScenarioPoint::default();
        p.i_type = i_type;
        p.model_set_id = model_set_id;
        p
    }

    fn region() -> CScenarioPointRegion {
        let mut r = CScenarioPointRegion::default();
        r.look_ups.type_names = ["WORLD_HUMAN_SMOKING", "PARKING_GROUP"]
            .map(HashedString::new)
            .into();
        r.look_ups.ped_model_set_names = ["none", "SMOKERS"].map(HashedString::new).into();
        r.look_ups.vehicle_model_set_names = ["none", "PARKED_CARS"].map(HashedString::new).into();
        r.points.my_points = vec![point(0, 1), point(0, 0)];
        let mut cluster = CScenarioPointCluster::default();
        cluster.points.my_points = vec![point(1, 1)];
        r.clusters = vec![cluster];
        r
    }

    #[test]
    fn a_points_model_set_is_read_from_the_table_its_type_spawns_from() {
        let points = manifests().name_points(&region()).unwrap();
        let summary: Vec<_> = points
            .iter()
            .map(|p| {
                (
                    p.at.cluster,
                    p.scenario_type.raw.as_str(),
                    p.vehicle,
                    p.model_set.as_ref().map(|s| s.raw.as_str()),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (None, "WORLD_HUMAN_SMOKING", false, Some("SMOKERS")),
                (None, "WORLD_HUMAN_SMOKING", false, None),
                (Some(0), "PARKING_GROUP", true, Some("PARKED_CARS")),
            ]
        );
    }

    #[test]
    fn a_name_no_manifest_declares_is_refused() {
        let mut r = region();
        r.look_ups.type_names[0] = HashedString::new("WORLD_HUMAN_NOWHERE");
        let err = manifests().name_points(&r).unwrap_err();
        assert!(matches!(err, ScenarioError::TypeUndeclared { .. }), "{err}");

        let mut r = region();
        r.points.my_points[0].model_set_id = 2;
        let err = manifests().name_points(&r).unwrap_err();
        assert!(
            matches!(err, ScenarioError::ModelSetOutOfRange { table: "ped", .. }),
            "{err}"
        );
    }

    #[test]
    fn setting_a_type_reuses_its_look_up_or_adds_one() {
        let mut r = region();
        let at = PointAt {
            cluster: Some(0),
            index: 0,
        };
        set_scenario_type(&mut r, at, HashedString::new("WORLD_HUMAN_SMOKING")).unwrap();
        assert_eq!(r.clusters[0].points.my_points[0].i_type, 0);
        assert_eq!(r.look_ups.type_names.len(), 2);

        set_scenario_type(&mut r, at, HashedString::new("WORLD_HUMAN_LEANING")).unwrap();
        assert_eq!(r.clusters[0].points.my_points[0].i_type, 2);
        assert_eq!(r.look_ups.type_names.len(), 3);

        let nowhere = PointAt {
            cluster: Some(1),
            index: 0,
        };
        assert_eq!(
            set_scenario_type(&mut r, nowhere, HashedString::new("WORLD_HUMAN_SMOKING")),
            Err(ScenarioError::NoSuchPoint(nowhere))
        );
    }
}
