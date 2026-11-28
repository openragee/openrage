//! The vehicles of a mounted install as a list a browser pages through: by text, by class, by type and by pack.
//!
//! ⚠ A class and a type are the text a vehicle data file states (`VC_SPORTS`, `VEHICLE_TYPE_CAR`) and are compared as text:
//! nothing here knows which classes or types the game has.

use or_title::install::vehicles::{MountedVehicle, VehicleFile, Vehicles};

use crate::find::{NameOrder, Rank, Work};

/// One question a browser of vehicles asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VehicleQuery<'q> {
    /// A part of a model's name, matched as a search of the install matches a name, or a part of a game name.
    pub text: Option<&'q str>,
    /// `vehicleClass` as a file states it, in any case.
    pub class: Option<&'q str>,
    /// `type` as a file states it, in any case.
    pub vehicle_type: Option<&'q str>,
    /// The device of the pack whose file states the vehicle, in any case.
    pub pack: Option<&'q str>,
    /// How many vehicles of the answer come before the page.
    pub offset: usize,
    /// The most vehicles of the page.
    pub count: usize,
}

impl Default for VehicleQuery<'_> {
    fn default() -> Self {
        Self {
            text: None,
            class: None,
            vehicle_type: None,
            pack: None,
            offset: 0,
            count: usize::MAX,
        }
    }
}

/// One vehicle of a page, and how it answers the text where there was one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VehicleListed<'a> {
    pub vehicle: &'a MountedVehicle,
    /// The file that states it.
    pub file: &'a VehicleFile,
    pub rank: Option<Rank>,
}

/// A page of the vehicles that answer a question.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VehiclePage<'a> {
    /// How many vehicles answer, of which `rows` holds those of the page.
    pub total: usize,
    pub rows: Vec<VehicleListed<'a>>,
}

/// Every vehicle of an install, by model name.
#[derive(Debug)]
pub struct VehicleCatalogue {
    list: Vehicles,
    /// The model names in lower case, each known by its vehicle's place in the list.
    names: NameOrder,
    /// The places of the vehicles in the list, by model name in lower case.
    by_name: Vec<u32>,
}

/// Each distinct text `of` gives, whatever its case, with how many times it comes, by the text.
fn tally<'a>(of: impl Iterator<Item = &'a str>) -> Vec<(&'a str, usize)> {
    let mut all: Vec<&str> = of.collect();
    all.sort_by_key(|text| text.to_ascii_lowercase());
    let mut tally: Vec<(&str, usize)> = Vec::new();
    for text in all {
        match tally.last_mut() {
            Some((last, count)) if last.eq_ignore_ascii_case(text) => *count += 1,
            _ => tally.push((text, 1)),
        }
    }
    tally
}

impl VehicleCatalogue {
    /// The vehicles of `list`, which a mount gave.
    pub fn new(list: Vehicles) -> Self {
        let lower = |at: usize| list.vehicles[at].vehicle.model_name.to_ascii_lowercase();
        let named = (0..list.vehicles.len()).map(|at| (lower(at), at as u32));
        let mut by_name: Vec<u32> = (0..list.vehicles.len() as u32).collect();
        by_name.sort_by_cached_key(|at| lower(*at as usize));
        Self {
            names: NameOrder::new(named.collect()),
            by_name,
            list,
        }
    }

    /// The vehicles and the data files they were read from, as the mount gave them.
    pub fn list(&self) -> &Vehicles {
        &self.list
    }

    /// Each class a file states, with how many vehicles are of it; a vehicle that states none is under the empty text.
    pub fn classes(&self) -> Vec<(&str, usize)> {
        tally(self.list.vehicles.iter().map(|v| v.vehicle.class.as_str()))
    }

    /// Each type a file states, with how many vehicles are of it.
    pub fn types(&self) -> Vec<(&str, usize)> {
        let types = (self.list.vehicles.iter()).map(|v| v.vehicle.vehicle_type.as_str());
        tally(types)
    }

    /// Each pack whose file states a vehicle of the list, with how many it states.
    pub fn packs(&self) -> Vec<(&str, usize)> {
        let packs = (self.list.vehicles.iter()).map(|v| self.list.file_of(v).pack.as_str());
        tally(packs)
    }

    /// The page of the vehicles that answer `query`, with how many answer: best first where there is a text, and by model name where there is none.
    pub fn page(&self, query: &VehicleQuery<'_>) -> VehiclePage<'_> {
        let same = |asked: Option<&str>, stated: &str| {
            asked.is_none_or(|a| a.eq_ignore_ascii_case(stated))
        };
        let answers = |vehicle: &MountedVehicle| {
            same(query.class, &vehicle.vehicle.class)
                && same(query.vehicle_type, &vehicle.vehicle.vehicle_type)
                && same(query.pack, &self.list.file_of(vehicle).pack)
        };
        let mut page = VehiclePage::default();
        let mut take = |at: u32, rank: Option<Rank>| {
            let vehicle = &self.list.vehicles[at as usize];
            if answers(vehicle) {
                if page.total >= query.offset && page.rows.len() < query.count {
                    page.rows.push(VehicleListed {
                        vehicle,
                        file: self.list.file_of(vehicle),
                        rank,
                    });
                }
                page.total += 1;
            }
        };
        match query.text.map(str::trim).filter(|text| !text.is_empty()) {
            None => self.by_name.iter().for_each(|at| take(*at, None)),
            Some(text) => {
                let text = text.to_ascii_lowercase();
                let matched = (self.names).matching(&text, usize::MAX, &mut Work::default());
                let mut listed = vec![false; self.list.vehicles.len()];
                for (name, rank) in matched {
                    let at = self.names.hash_at(name);
                    listed[at as usize] = true;
                    take(at, Some(rank));
                }
                // ⚠ Several models share a game name, so it is no key of the name order: it is passed over, after the models.
                for at in &self.by_name {
                    let game = &self.list.vehicles[*at as usize].vehicle.game_name;
                    if !listed[*at as usize] && game.to_ascii_lowercase().contains(&text) {
                        take(*at, Some(Rank::Contained));
                    }
                }
            }
        }
        page
    }
}
