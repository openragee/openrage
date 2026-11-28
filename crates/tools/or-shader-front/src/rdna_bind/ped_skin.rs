//! The table of skin tone records a ped's pixel program takes its skin's colour from, and the
//! look-up texture the colour is found in.

use or_io::gta6::hash::name_hash32;

use super::shading::Feed;

/// The buffer of skin tone records, [`SkinToneRecord`] after [`SkinToneRecord`]; a part picks
/// its record by a number of its material, nought for a part drawn alone.
pub const SKIN_TONES: u32 = 0x8311_FF80;

/// The binding of the skin albedo look-up table, a volume texture the game binds by its name.
pub const ALBEDO_LUT: u32 = 0x76F8_D61D;

/// The name the game finds the skin albedo look-up table by among its global textures.
pub const ALBEDO_LUT_NAME: u32 = name_hash32(b"skinAlbedoLUT");

/// The name of the tone the game builds where none is loaded, and the tone record nought names.
pub const DEFAULT_TONE: u32 = name_hash32(b"default");

/// A skin tone as the game holds it, of the fields the record is built from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkinTone {
    /// The colour of the skin the albedo is authored for.
    pub colour: [f32; 3],
    /// Where in the look-up table the tone is found: across, down and in depth.
    pub at: [f32; 3],
    /// Three more numbers the record carries; nought for the built-in tone.
    pub extra: [f32; 3],
    /// What a ped wearing it is given as the weight of the per-pixel term of the look-up's
    /// depth ([`super::ped_layers::ObjectData::depth_weight`]); nought for the built-in tone.
    pub depth_weight: f32,
}

impl SkinTone {
    /// The tone the game builds where no tone is loaded under the name a record asks for.
    pub const DEFAULT: Self = Self {
        colour: [
            f32::from_bits(0x3F5F_88B4),
            f32::from_bits(0x3F23_E4BA),
            f32::from_bits(0x3F06_2B88),
        ],
        at: [0.0, 1.0, 0.0],
        extra: [0.0; 3],
        depth_weight: 0.0,
    };
}

/// The tone a preset falls back to where the one it names is not loaded.
pub const FALLBACK_TONE: u32 = 0xC28F_3ECD;
/// The preset a ped takes where the one it names, or none, is not loaded.
pub const FALLBACK_PRESET: u32 = 0xAC23_15DA;

/// A preset of the loaded skin tones: its name, the tone it names, and its entry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkinPreset {
    pub name: u32,
    pub tone: u32,
    pub entry: SkinToneEntry,
}

/// The loaded tones by name, and the presets in the order they are loaded.
#[derive(Debug, Clone, Copy)]
pub struct Loaded<'a> {
    pub tones: &'a [(u32, SkinTone)],
    pub presets: &'a [SkinPreset],
}

impl Loaded<'_> {
    /// The tone named `name`, else the fallback tone, else the built-in one.
    // The built-in tone is held under its own name before the loaded ones, which a loaded tone
    // of that name shadows.
    fn tone(&self, name: u32) -> SkinTone {
        let named = |name| (self.tones.iter()).find_map(|(n, t)| (*n == name).then_some(*t));
        let built_in = (name == DEFAULT_TONE).then_some(SkinTone::DEFAULT);
        (named(name).or(built_in))
            .or_else(|| named(FALLBACK_TONE))
            .unwrap_or(SkinTone::DEFAULT)
    }

    /// The record table: record nought the default entry's, then one for each preset.
    pub fn table(&self) -> Vec<u8> {
        let mut bytes =
            SkinToneRecord::of(&self.tone(DEFAULT_TONE), &SkinToneEntry::DEFAULT).bytes();
        for preset in self.presets {
            bytes.extend(SkinToneRecord::of(&self.tone(preset.tone), &preset.entry).bytes());
        }
        bytes
    }

    /// The number of the record of the preset named `name`: its place plus one, else the
    /// fallback preset's, else nought.
    pub fn slot(&self, name: u32) -> u32 {
        let place = |name| (self.presets.iter()).position(|p| p.name == name);
        (place(name).or_else(|| place(FALLBACK_PRESET))).map_or(0, |at| at as u32 + 1)
    }

    /// The depth weight a ped whose record is number `slot` is given: its preset's tone's.
    pub fn depth_weight(&self, slot: u32) -> f32 {
        let preset = slot
            .checked_sub(1)
            .and_then(|at| self.presets.get(at as usize));
        self.tone(preset.map_or(DEFAULT_TONE, |p| p.tone))
            .depth_weight
    }
}

/// One of the game's list of tones a record is built for, of the fields the record takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkinToneEntry {
    /// The colour the record states where its look-up falls on [`SkinToneRecord::at`].
    pub colour: [f32; 3],
    /// What is added to the tone's place in the look-up table.
    pub offset: [f32; 3],
}

impl SkinToneEntry {
    /// The entry record nought is built for, which names [`DEFAULT_TONE`].
    pub const DEFAULT: Self = Self {
        colour: [0.0; 3],
        offset: [0.0; 3],
    };
}

/// One 60-byte record of [`SKIN_TONES`], a dword a field.
// The pixel program multiplies the albedo by the colour its look-up gives times
// `colour_scale`, weighted by the part's skin weight; where both coordinates of the look-up
// equal `at` it takes `colour` and samples nothing.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkinToneRecord {
    /// One over the tone's colour, so the tone's own colour leaves the albedo as it is.
    pub colour_scale: [f32; 3],
    /// The entry's colour.
    pub colour: [f32; 3],
    /// Not written.
    pub zeros_18: [f32; 3],
    /// The look-up's first and third coordinates at which `colour` stands in for it.
    pub at: [f32; 2],
    /// The look-up's second coordinate: which tone of the table.
    pub row: f32,
    /// The tone's [`SkinTone::extra`].
    pub extra: [f32; 3],
}

const _: () = assert!(std::mem::size_of::<SkinToneRecord>() == 60);

impl SkinToneRecord {
    /// The record nought: the built-in tone's for the default entry.
    pub const AT_REST: Self = Self::of(&SkinTone::DEFAULT, &SkinToneEntry::DEFAULT);

    /// The record the game builds for `entry` from the tone it names.
    pub const fn of(tone: &SkinTone, entry: &SkinToneEntry) -> Self {
        let [across, down, depth] = [
            tone.at[0] + entry.offset[0],
            tone.at[1] + entry.offset[1],
            tone.at[2] + entry.offset[2],
        ];
        Self {
            colour_scale: [
                1.0 / tone.colour[0],
                1.0 / tone.colour[1],
                1.0 / tone.colour[2],
            ],
            colour: entry.colour,
            zeros_18: [0.0; 3],
            at: [across, depth],
            row: down,
            extra: tone.extra,
        }
    }

    /// The record's bytes.
    pub fn bytes(&self) -> Vec<u8> {
        let floats = [
            self.colour_scale.as_slice(),
            &self.colour,
            &self.zeros_18,
            &self.at,
            &[self.row],
            &self.extra,
        ];
        floats
            .concat()
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect()
    }
}

/// What the game binds as `hash` for a part drawn alone: record nought of the skin tones, and
/// the look-up table from its own dictionaries.
pub(super) fn stated(hash: u32) -> Option<Feed> {
    match hash {
        SKIN_TONES => Some(Feed::SkinTones),
        ALBEDO_LUT => Some(Feed::GameTexture(ALBEDO_LUT_NAME)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::mem::offset_of;

    use or_io::gta5::shaders::sgd5::Sgd5Table;

    use super::*;
    use crate::rdna::ResourceKind;
    use crate::rdna_bind::shading::feed;
    use crate::rdna_bind::trees::tests::stage_reading;

    fn float(bytes: &[u8], at: usize) -> f32 {
        f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn a_skin_tone_record_holds_each_field_at_the_byte_the_pixel_program_reads() {
        assert_eq!(offset_of!(SkinToneRecord, colour_scale), 0x00);
        assert_eq!(offset_of!(SkinToneRecord, colour), 0x0C);
        assert_eq!(offset_of!(SkinToneRecord, at), 0x24);
        assert_eq!(offset_of!(SkinToneRecord, row), 0x2C);
        assert_eq!(offset_of!(SkinToneRecord, extra), 0x30);
        let tone = SkinTone {
            colour: [0.5, 0.25, 2.0],
            at: [0.125, 0.5, 0.75],
            extra: [3.0, 4.0, 5.0],
            depth_weight: 0.0,
        };
        let entry = SkinToneEntry {
            colour: [0.1, 0.2, 0.3],
            offset: [0.25, 0.125, 0.0625],
        };
        let bytes = SkinToneRecord::of(&tone, &entry).bytes();
        assert_eq!(bytes.len(), 60);
        assert_eq!(
            [float(&bytes, 0), float(&bytes, 4), float(&bytes, 8)],
            [2.0, 4.0, 0.5]
        );
        assert_eq!(float(&bytes, 0x10), 0.2);
        assert_eq!(float(&bytes, 0x18), 0.0);
        // The second coordinate goes last: the record states across, in depth, then down.
        assert_eq!(float(&bytes, 0x24), 0.375);
        assert_eq!(float(&bytes, 0x28), 0.8125);
        assert_eq!(float(&bytes, 0x2C), 0.625);
        assert_eq!(float(&bytes, 0x38), 5.0);
    }

    #[test]
    fn record_nought_undoes_the_built_in_tone_and_looks_up_its_row() {
        let rest = SkinToneRecord::AT_REST;
        for (scale, colour) in rest.colour_scale.iter().zip(SkinTone::DEFAULT.colour) {
            assert!((scale * colour - 1.0).abs() < 1e-6);
        }
        assert_eq!(rest.colour, [0.0; 3]);
        assert_eq!((rest.at, rest.row), ([0.0, 0.0], 1.0));
        assert_eq!(DEFAULT_TONE, 0xE4DF_46D5);
        assert_eq!(ALBEDO_LUT_NAME, 0xBC20_EC5C);
    }

    /// A stage that reads the record a part's number picks, nought.
    const READING: &str = "
@group(0) @binding(0) var<storage> tones: array<vec4<u32>>;
@vertex fn main() -> @builtin(position) vec4<f32> {
    return vec4<f32>(bitcast<f32>(tones[0].x));
}";

    #[test]
    fn a_stage_reading_the_skin_tones_is_fed_the_table_and_the_look_up_by_name() {
        let buffer = ResourceKind::Buffer { stride: None };
        let fed = |hash| {
            let (native, lowered, table) = stage_reading(hash, 4, buffer, READING);
            let stage = feed(&native, lowered, &table, &Sgd5Table::default())
                .unwrap_or_else(|why| panic!("0x{hash:08X}: {why}"));
            stage.feeds[0].feed.clone()
        };
        assert_eq!(fed(SKIN_TONES), Feed::SkinTones);
        // The look-up table is the game's own texture by its name, not noughts.
        assert_eq!(fed(ALBEDO_LUT), Feed::GameTexture(ALBEDO_LUT_NAME));
        assert_eq!(fed(0xA000_0007), Feed::Zero(Some(0xA000_0007)));
    }

    fn tone(colour: f32, depth_weight: f32) -> SkinTone {
        SkinTone {
            colour: [colour; 3],
            at: [0.25, 0.0, 0.5],
            extra: [0.0; 3],
            depth_weight,
        }
    }

    fn preset(name: u32, tone: u32) -> SkinPreset {
        SkinPreset {
            name,
            tone,
            entry: SkinToneEntry {
                colour: [0.5; 3],
                offset: [0.0; 3],
            },
        }
    }

    #[test]
    fn the_loaded_table_holds_record_nought_then_a_record_a_preset_by_its_tone() {
        let tones = [(FALLBACK_TONE, tone(0.5, 0.25)), (7, tone(0.25, 0.75))];
        let presets = [
            preset(FALLBACK_PRESET, FALLBACK_TONE),
            preset(9, 7),
            preset(11, 99),
        ];
        let loaded = Loaded {
            tones: &tones,
            presets: &presets,
        };
        let table = loaded.table();
        assert_eq!(table.len(), 4 * 60);
        // Record nought is the built-in tone's, whatever is loaded.
        assert_eq!(&table[..60], SkinToneRecord::AT_REST.bytes().as_slice());
        // Each preset's record undoes the colour of the tone it names, else the fallback tone's.
        assert_eq!(float(&table, 60), 2.0);
        assert_eq!(float(&table, 120), 4.0);
        assert_eq!(
            float(&table, 180),
            2.0,
            "an unloaded tone takes the fallback"
        );
        // A name is its preset's place plus one; any other, or none, the fallback preset's.
        assert_eq!((loaded.slot(9), loaded.slot(0), loaded.slot(5)), (2, 1, 1));
        assert_eq!(loaded.depth_weight(1), 0.25);
        assert_eq!(loaded.depth_weight(2), 0.75);
        assert_eq!(loaded.depth_weight(0), 0.0, "record nought's built-in tone");
        // With no fallback preset loaded, an unknown name is record nought.
        let none = Loaded {
            tones: &tones,
            presets: &presets[1..],
        };
        assert_eq!(none.slot(0), 0);
    }

    #[test]
    fn a_ped_s_object_record_holds_its_tone_s_number_and_weight_where_the_program_reads_them() {
        use super::super::ped_layers::ObjectData;
        let record = ObjectData {
            skin_tone: 3,
            depth_weight: 0.5,
            ..ObjectData::AT_REST
        };
        let bytes = record.bytes();
        assert_eq!(bytes.len(), 416);
        assert_eq!(
            u32::from_le_bytes(bytes[0x164..0x168].try_into().unwrap()),
            3
        );
        assert_eq!(float(&bytes, 0x19C), 0.5);
    }
}
