//! The variables of GTA VI's timecycle frame its lights read, each by its index in the variable table
//! of the user's executable: the global intensity, the range scales and the level-of-detail lights'
//! two sets of tint, override and intensity.

use glam::Vec3;
use or_io::gta5::data::timecycle::Keyframe;
use or_title::gta6_day_frame::name_hash;
use or_title::gta6_day_vars::VarTable;

/// The indices the lights read.
const LOD_GAIN: usize = 237;
const INTENSITY: usize = 240;
const LOD_RANGE: usize = 241;
const MAP_RANGE_EXTERIOR: usize = 242;
const MAP_RANGE_INTERIOR: usize = 243;
/// The first of a set's intensity, tint, override colour and override amount.
const SET_ZERO: usize = 286;
const SET_ONE: usize = 374;
const CATEGORIES: usize = 343;

/// One set of the level-of-detail lights' globals.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LodSet {
    pub intensity: f32,
    pub tint: Vec3,
    pub colour: Vec3,
    pub amount: f32,
}

/// What the frame gives the lights.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightVars {
    pub intensity: f32,
    pub lod_range: f32,
    /// A map light's range scales outdoors and in.
    pub map_range: [f32; 2],
    pub lod_gain: Vec3,
    /// Set 0, the map's own lights, and set 1, a vehicle's.
    pub sets: [LodSet; 2],
    /// The six category multipliers, which set 1 alone takes.
    pub categories: [f32; 6],
}

impl Default for LightVars {
    /// The table's defaults: every scale one and every override amount nought.
    fn default() -> Self {
        let set = LodSet {
            intensity: 1.0,
            tint: Vec3::ONE,
            colour: Vec3::ONE,
            amount: 0.0,
        };
        Self {
            intensity: 1.0,
            lod_range: 1.0,
            map_range: [1.0; 2],
            lod_gain: Vec3::ONE,
            sets: [set; 2],
            categories: [1.0; 6],
        }
    }
}

impl LightVars {
    /// The variables `own` states, found by the hash `table` gives each index, the table's default where it states none.
    pub fn of(table: &VarTable, own: &Keyframe) -> Self {
        let mut stated = std::collections::HashMap::new();
        for (name, value) in own.iter() {
            stated.insert(name_hash(name), value);
        }
        let v = |i: usize| {
            (table.entries.get(i))
                .map_or(1.0, |e| stated.get(&e.hash).copied().unwrap_or(e.default))
        };
        let triple = |i: usize| Vec3::new(v(i), v(i + 1), v(i + 2));
        let set = |i: usize| LodSet {
            intensity: v(i),
            tint: triple(i + 1),
            colour: triple(i + 4),
            amount: v(i + 7),
        };
        Self {
            intensity: v(INTENSITY),
            lod_range: v(LOD_RANGE),
            map_range: [v(MAP_RANGE_EXTERIOR), v(MAP_RANGE_INTERIOR)],
            lod_gain: triple(LOD_GAIN),
            sets: [set(SET_ZERO), set(SET_ONE)],
            categories: std::array::from_fn(|k| v(CATEGORIES + k)),
        }
    }

    /// The bits of every value, which a choice of lamps made for other variables is told apart by.
    pub fn bits(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let s = |set: &LodSet| {
            [set.intensity, set.tint.x, set.tint.y, set.tint.z]
                .into_iter()
                .chain([set.colour.x, set.colour.y, set.colour.z, set.amount])
        };
        let all = [
            self.intensity,
            self.lod_range,
            self.map_range[0],
            self.map_range[1],
        ]
        .into_iter()
        .chain(self.lod_gain.to_array())
        .chain(s(&self.sets[0]))
        .chain(s(&self.sets[1]))
        .chain(self.categories);
        for x in all {
            x.to_bits().hash(&mut hasher);
        }
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use or_title::gta6_day_vars::VarEntry;

    /// A table of 400 variables, each hashed from its own index's name and defaulting to its index.
    fn table() -> VarTable {
        let mut bytes = Vec::new();
        for i in 0..1407u32 {
            let default = if i < 400 { i as f32 } else { 0.0 };
            for word in [i, name_hash(&format!("v{i}")), default.to_bits(), 1, 0, 2] {
                bytes.extend(word.to_le_bytes());
            }
        }
        VarTable::parse(&bytes).expect("a table")
    }

    #[test]
    fn a_variable_is_read_by_the_hash_its_index_states_and_takes_its_default_where_unstated() {
        let table = table();
        assert_eq!(
            table.entries[240],
            VarEntry {
                hash: name_hash("v240"),
                default: 240.0,
                kind: 1,
                mode: 2
            }
        );
        let mut own = Keyframe::default();
        own.set("v240", 0.5);
        own.set("v293", 0.25);
        own.set("v345", 3.0);
        let vars = LightVars::of(&table, &own);
        assert_eq!(vars.intensity, 0.5);
        assert_eq!(vars.lod_range, 241.0, "unstated: the table's default");
        assert_eq!(vars.map_range, [242.0, 243.0]);
        assert_eq!(vars.sets[0].intensity, 286.0);
        assert_eq!(vars.sets[0].tint, Vec3::new(287.0, 288.0, 289.0));
        assert_eq!(vars.sets[0].colour, Vec3::new(290.0, 291.0, 292.0));
        assert_eq!(vars.sets[0].amount, 0.25);
        assert_eq!(vars.sets[1].intensity, 374.0);
        assert_eq!(vars.sets[1].amount, 381.0);
        assert_eq!(vars.categories[2], 3.0);
        assert_ne!(vars.bits(), LightVars::default().bits());
    }
}
