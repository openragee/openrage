//! The vehicles a mounted game declares: each vehicle data file the mount would load, read for what a list of vehicles shows.
//!
//! ⚠ Only what the file's own text states of a vehicle is read: its model, its game name, its class and its type, each as the
//! file spells it. Nothing here interprets a class or a type, or reads a vehicle's colours, variations or handling.

use crate::dlc::tree::parse_rooted;
use crate::dlc::xml::Result;
use crate::vfs::MountedGame;

/// Where the base game keeps the vehicles of its level; the title update's copy lies over it on the same device.
pub const LEVEL_VEHICLES: &str = "common:/data/levels/gta5/vehicles.meta";

/// A pack registers a vehicle data file with this type.
pub const VEHICLE_FILE_TYPE: &str = "VEHICLE_METADATA_FILE";

/// One vehicle, as a vehicle data file states it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Vehicle {
    /// `modelName`, as the file spells it.
    pub model_name: String,
    /// `gameName`, as the file spells it; empty where the file states none.
    pub game_name: String,
    /// `vehicleClass`, as the file's text states it (`VC_SPORTS`); empty where the file states none.
    pub class: String,
    /// `type`, as the file's text states it (`VEHICLE_TYPE_CAR`); empty where the file states none.
    pub vehicle_type: String,
}

/// What one vehicle data file states.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatedVehicles {
    /// In the file's order.
    pub vehicles: Vec<Vehicle>,
    /// How many of its items state no model, and are no vehicle of the list.
    pub unnamed: usize,
}

/// The vehicles the data file `bytes` states; an error where it is not a list of vehicles.
pub fn vehicles_in(bytes: &[u8]) -> Result<StatedVehicles> {
    let root = parse_rooted(bytes, "vehicles.meta", &["CVehicleModelInfo__InitDataList"])?;
    let mut stated = StatedVehicles::default();
    for item in root.items("InitDatas") {
        let model_name = item.text_of("modelName");
        if model_name.is_empty() {
            stated.unnamed += 1;
            continue;
        }
        stated.vehicles.push(Vehicle {
            model_name,
            game_name: item.text_of("gameName"),
            class: item.text_of("vehicleClass"),
            vehicle_type: item.text_of("type"),
        });
    }
    Ok(stated)
}

/// A vehicle data file of a mount, and what it gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VehicleFile {
    /// The device of the pack it belongs to: the pack that registers it, and for the level's own file the archive whose copy the mount opens.
    pub pack: String,
    /// The path the mount opens it at.
    pub path: String,
    /// How many vehicles it states, or why it gave none: it did not open, or it is no list of vehicles.
    pub read: std::result::Result<usize, String>,
}

/// One vehicle of a mount, as the last data file to state its model does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountedVehicle {
    pub vehicle: Vehicle,
    /// The place in [`Vehicles::files`] of the file that states it.
    pub file: usize,
    /// The places in [`Vehicles::files`] of the earlier files that stated a vehicle of this model, in the mount's order.
    pub replaced: Vec<usize>,
}

/// The vehicles of a mounted game and the data files they were read from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Vehicles {
    /// Each model once, in the order its first statement came in.
    pub vehicles: Vec<MountedVehicle>,
    /// The level's own file, then each file a mounted pack registers and leaves enabled, in the mount's order.
    pub files: Vec<VehicleFile>,
}

impl Vehicles {
    /// The file that states `vehicle`.
    pub fn file_of(&self, vehicle: &MountedVehicle) -> &VehicleFile {
        &self.files[vehicle.file]
    }
}

/// The device of the archive whose copy of the level's vehicles the mount opens: a later title update's lies over an earlier one's and over the base game's.
fn level_pack(game: &MountedGame) -> &'static str {
    let inside = LEVEL_VEHICLES.trim_start_matches("common:/");
    let updates = ["update2", "update"];
    let holds = |device: &&str| game.vfs.exists(&format!("{device}:/common/{inside}"));
    updates.into_iter().find(holds).unwrap_or("common")
}

/// Every vehicle `game` declares: the level's own file, then each vehicle data file a mounted pack registers and leaves enabled, a later file's vehicle taking the place of an earlier one of the same model.
// ⚠ A model is one whatever its case, as the game hashes a name. A file that does not open or does not read is listed with
// why and adds no vehicle.
pub fn vehicles(game: &MountedGame) -> Vehicles {
    let mut paths: Vec<(String, String)> = Vec::new();
    if game.vfs.exists(LEVEL_VEHICLES) {
        paths.push((level_pack(game).to_string(), LEVEL_VEHICLES.to_string()));
    }
    for pack in &game.packs {
        let registered = (pack.data_files.iter())
            .filter(|file| file.file_type == VEHICLE_FILE_TYPE && !file.disabled);
        for file in registered {
            // A pack that registers the level's own file again adds nothing the level did not.
            let listed = |(_, path): &(String, String)| path.eq_ignore_ascii_case(&file.filename);
            if !paths.iter().any(listed) {
                paths.push((pack.device_name.clone(), file.filename.clone()));
            }
        }
    }

    let mut all = Vehicles::default();
    let mut by_model: std::collections::HashMap<String, usize> = Default::default();
    for (pack, path) in paths {
        let stated = match game.vfs.open(&path) {
            None => Err("the mount holds no such file".to_string()),
            Some(bytes) => vehicles_in(&bytes).map_err(|e| e.to_string()),
        };
        let file = all.files.len();
        let read = stated.as_ref().map(|stated| stated.vehicles.len());
        all.files.push(VehicleFile {
            pack,
            path,
            read: read.map_err(Clone::clone),
        });
        for vehicle in stated.map(|stated| stated.vehicles).unwrap_or_default() {
            match by_model.get(&vehicle.model_name.to_ascii_lowercase()) {
                Some(at) => {
                    let earlier = &mut all.vehicles[*at];
                    earlier.replaced.push(earlier.file);
                    earlier.file = file;
                    earlier.vehicle = vehicle;
                }
                None => {
                    by_model.insert(vehicle.model_name.to_ascii_lowercase(), all.vehicles.len());
                    all.vehicles.push(MountedVehicle {
                        vehicle,
                        file,
                        replaced: Vec::new(),
                    });
                }
            }
        }
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A vehicle data file stating `vehicles` as `(model, game name, class, type)`, each with lists of its own whose items are no vehicle.
    fn file(vehicles: &[(&str, &str, &str, &str)]) -> String {
        let items: String = (vehicles.iter())
            .map(|(model, game, class, kind)| {
                format!(
                    "<Item><modelName>{model}</modelName><gameName>{game}</gameName>\
                     <type>{kind}</type><vehicleClass>{class}</vehicleClass>\
                     <drivers><Item><driverName>someone</driverName></Item></drivers></Item>"
                )
            })
            .collect();
        format!(
            "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<CVehicleModelInfo__InitDataList>\
             <residentTxd>shared</residentTxd><InitDatas>{items}</InitDatas>\
             <txdRelationships><Item><parent>a</parent><child>b</child></Item></txdRelationships>\
             </CVehicleModelInfo__InitDataList>"
        )
    }

    #[test]
    fn a_vehicle_file_states_each_vehicle_once_with_its_own_text_and_no_item_of_another_list() {
        let text = file(&[
            ("carone", "CARONE", "VC_SPORTS", "VEHICLE_TYPE_CAR"),
            ("boatone", "BOATONE", "VC_BOAT", "VEHICLE_TYPE_BOAT"),
        ]);
        let stated = vehicles_in(text.as_bytes()).expect("a list of vehicles");
        let vehicle = |model: &str, game: &str, class: &str, kind: &str| Vehicle {
            model_name: model.into(),
            game_name: game.into(),
            class: class.into(),
            vehicle_type: kind.into(),
        };
        assert_eq!(
            stated.vehicles,
            vec![
                vehicle("carone", "CARONE", "VC_SPORTS", "VEHICLE_TYPE_CAR"),
                vehicle("boatone", "BOATONE", "VC_BOAT", "VEHICLE_TYPE_BOAT"),
            ],
            "the driver of a vehicle and a texture relationship are items of other lists"
        );
        assert_eq!(stated.unnamed, 0);
    }

    #[test]
    fn an_item_with_no_model_is_counted_and_a_document_of_another_kind_is_refused() {
        let text = file(&[
            ("", "NOBODY", "VC_SPORTS", "VEHICLE_TYPE_CAR"),
            ("bare", "", "", ""),
        ]);
        let stated = vehicles_in(text.as_bytes()).expect("a list of vehicles");
        assert_eq!(stated.unnamed, 1);
        assert_eq!(stated.vehicles.len(), 1);
        assert_eq!(
            stated.vehicles[0],
            Vehicle {
                model_name: "bare".into(),
                ..Default::default()
            },
            "what the file does not state is empty and not guessed"
        );

        let other = "<CPedModelInfo__InitDataList><InitDatas><Item><modelName>x</modelName>\
                     </Item></InitDatas></CPedModelInfo__InitDataList>";
        let refused = vehicles_in(other.as_bytes()).expect_err("a list of peds");
        assert!(
            refused
                .to_string()
                .contains("CVehicleModelInfo__InitDataList"),
            "{refused}"
        );
        assert!(vehicles_in(b"<CVehicleModelInfo__InitDataList><InitDatas>").is_err());
    }
}
