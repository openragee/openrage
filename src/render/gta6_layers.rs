//! A GTA VI layered material as the game hands it to a pixel program: one record of dwords, eight
//! layers wide, made from the material's rows, its layers' definitions and their tiles.

use std::collections::BTreeMap;

/// The dwords of a record.
pub(crate) const RECORD: usize = 0xFD0 / 4;
/// The layers of a material.
pub(crate) const LAYERS: usize = 8;

/// The index or the layer of a row that is for every one.
const EVERY: u8 = 255;

// The names of the rows read here, as the files state them: hashes whose names are not known.
/// How many times a tile repeats over a part.
const REPEATS: u32 = 0x01AE_45D3;
/// The angle a tile is turned by about its middle, in radians.
const ANGLE: u32 = 0x8BBC_E09B;
/// Where a tile is moved to: index 0 along u, index 1 along v.
const OFFSET: u32 = 0x9DFE_6ABF;
/// A layer's colours, index 0 to 3.
const COLOUR: u32 = 0xF574_261B;
/// A layer definition's flags, and a tile's.
const LAYER_FLAGS: u32 = 0x424E_C6E2;
const TILE_FLAGS: u32 = 0x521A_CAC3;
/// A float a pixel program grows the specular reflectance by: it takes 0.04 and 0.04 times it.
const REFLECTANCE: u32 = 0x27B9_B2FA;
/// A float of a tile, a definition or a material, and one of a tile alone.
const THIRD_FLOAT: u32 = 0x6C75_B6B2;
const TILE_NINTH: u32 = 0xBA09_F178;
/// The floats of a definition or a material the record keeps as they are, each with the row
/// and the dword of the row it is kept in.
const KEPT: [(u32, usize, usize); 6] = [
    (REFLECTANCE, FLAGS, 1),
    (THIRD_FLOAT, FLAGS, 2),
    (0xFB7F_02D8, NINTH, 1),
    (0x5EEB_ED48, NINTH, 2),
    (0x545A_21F2, PAIRS, 0),
    (SCALED, SCALED_ROW, 0),
];
/// A float kept times the first pair's value.
const SCALED: u32 = 0x1B0B_9800;
/// Two pairs of floats, index 0 and 1, between which the model's own byte picks: the record
/// keeps the first pair's value, and the second's squared.
const FIRST_PAIR: u32 = 0x5864_5355;
const SECOND_PAIR: u32 = 0x2830_B00C;
/// A material's two colours for all its layers, index 0 and 1.
const WHOLE_COLOUR: u32 = 0x4A7B_E614;
/// The textures of a layer, in the order the record numbers them: a tile's two, one more, then
/// four by index, or one alone where bit 0 of the material's flags is set.
const TILE_MASK: u32 = 0xC380_E141;
const TILE_SECOND: u32 = 0x1B68_9BA7;
const THIRD: u32 = 0xC78C_DDC8;
const SLOT: u32 = 0xD2B5_98E5;
const ALONE: u32 = 0xF4F6_BFCC;

/// The bit of a layer's flags the record never states.
const NEVER: u32 = 1 << 21;

// Where the rows of a record start, sixteen bytes a layer.
const FLAGS: usize = 0x10;
const NINTH: usize = 0x90;
const PAIRS: usize = 0x110;
const SCALED_ROW: usize = 0xEC0;
const TRANSFORM_U: usize = 0x210;
const TRANSFORM_V: usize = 0x290;
const TEXTURES: usize = 0x790;
const SLOTS: usize = 0x810;
const FIRST_COLOUR: usize = 0x910;
const SECOND_COLOUR: usize = 0x990;
const MATERIAL_FLAGS: usize = 0xE10;
const WHOLE_COLOURS: usize = 0xE34;

/// A row of a list: a value of four bytes by name, for one index and one layer or for every one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Row {
    pub name: u32,
    pub value: u32,
    pub index: u8,
    pub layer: u8,
}

/// A row that names a texture by its 64-bit key.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TextureRow {
    pub name: u32,
    pub key: u64,
    pub index: u8,
    pub layer: u8,
}

/// The four lists a tile, a layer definition and a material each hold, sorted by name.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Rows {
    pub floats: Vec<Row>,
    pub colours: Vec<Row>,
    pub words: Vec<Row>,
    pub textures: Vec<TextureRow>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Definition {
    pub tile: u32,
    pub rows: Rows,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Material {
    pub flags: u32,
    /// Each layer's definition, nought for a layer the material has not.
    pub layers: [u32; LAYERS],
    /// The tile a layer takes in place of its definition's, nought for none.
    pub tiles: [u32; LAYERS],
    pub rows: Rows,
}

/// What the materials file holds, each map by its keys.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Layered {
    pub tiles: BTreeMap<u32, Rows>,
    pub definitions: BTreeMap<u32, Definition>,
    pub materials: BTreeMap<u32, Material>,
}

/// Each row of `name` the game takes from `rows`, a layer at a time and an index at a time within
/// it: the row its walk stands on is taken where its index is `indexes[slot]` or every one and its
/// layer is the layer in hand or every one, and the walk then moves on. So a row for every layer
/// is taken once, by the first layer that comes to it.
// ⚠ The game walks the whole sorted list with one cursor, name after name. This walks the rows
// of one name, which is the same wherever a list states no name the game does not take.
fn walk(
    rows: &[Row],
    name: u32,
    layers: usize,
    indexes: &[u8],
    mut take: impl FnMut(usize, usize, u32),
) {
    let mut at = rows.iter().filter(|row| row.name == name).peekable();
    for layer in 0..layers {
        for (slot, index) in indexes.iter().enumerate() {
            let fits = |row: &&Row| {
                (row.index == EVERY || row.index == *index)
                    && (row.layer == EVERY || usize::from(row.layer) == layer)
            };
            if let Some(row) = at.next_if(fits) {
                take(layer, slot, row.value);
            }
        }
    }
}

/// The first row of `name` a list of one layer gives, as [`walk`] takes it.
fn first(rows: &[Row], name: u32) -> Option<u32> {
    let mut found = None;
    walk(rows, name, 1, &[0], |_, _, value| found = Some(value));
    found
}

/// The key of the texture `name` of `index` in `rows`, where a row for that index or every one
/// states it; a list is sorted, so the first such row is the one a search by name comes to.
fn stated(rows: &[TextureRow], name: u32, index: u8) -> Option<u64> {
    (rows.iter())
        .find(|row| row.name == name && (row.index == EVERY || row.index == index))
        .map(|row| row.key)
}

impl Layered {
    /// The tile of layer `layer` of `material`: the one the material states, else its definition's.
    fn tile(&self, material: &Material, layer: usize) -> Option<&Rows> {
        let definition = self.definitions.get(&material.layers[layer]);
        let key = match material.tiles[layer] {
            0 => definition?.tile,
            key => key,
        };
        self.tiles.get(&key)
    }

    /// The key of texture `name` of `index` for layer `layer`: the material's own row for that
    /// very layer and index, else its definition's row, else the tile's; nought where none has.
    fn texture(&self, material: &Material, layer: usize, name: u32, index: u8) -> u64 {
        let own = (material.rows.textures.iter())
            .find(|r| r.name == name && r.index == index && usize::from(r.layer) == layer);
        if let Some(row) = own {
            return row.key;
        }
        let Some(definition) = self.definitions.get(&material.layers[layer]) else {
            return 0;
        };
        stated(&definition.rows.textures, name, index)
            .or_else(|| {
                let tile = self.tile(material, layer)?;
                Some(tile.textures.iter().find(|row| row.name == name)?.key)
            })
            .unwrap_or(0)
    }

    /// The seven textures of layer `layer`, in the order the record numbers them.
    pub(crate) fn textures(&self, material: &Material, layer: usize) -> [u64; 7] {
        let of = |name, index| self.texture(material, layer, name, index);
        let alone = material.flags & 1 != 0;
        [
            of(TILE_MASK, EVERY),
            of(TILE_SECOND, EVERY),
            of(THIRD, EVERY),
            if alone { of(ALONE, EVERY) } else { of(SLOT, 0) },
            of(SLOT, 1),
            of(SLOT, 2),
            of(SLOT, 3),
        ]
    }
}

/// What a layer's rows come to before the record is written.
#[derive(Clone, Copy, Default)]
struct Layer {
    repeats: f32,
    angle: f32,
    offset: [f32; 2],
    /// Whether a row states how the tile is laid: its transform is written only then.
    laid: bool,
    tile_flags: u32,
    flags: u32,
    flagged: bool,
    colours: [Option<u32>; 4],
    /// The two pairs, each with whether a row states either of its floats.
    pairs: [([f32; 2], bool); 2],
}

/// The record of `material`. `number` gives the number a pixel program picks the texture of a
/// key by, and is asked for nought too. A value no row states is nought: the game takes it from
/// a table of defaults, which answers nought for a name it has not.
pub(crate) fn record(
    material: &Material,
    layered: &Layered,
    number: &mut dyn FnMut(u64) -> u32,
) -> Vec<u32> {
    let mut record = vec![0u32; RECORD];
    let mut layers = [Layer::default(); LAYERS];
    let row = |at: usize, layer: usize| (at + 16 * layer) / 4;
    for (at, layer) in layers.iter_mut().enumerate() {
        if let Some(tile) = layered.tile(material, at) {
            if let Some(repeats) = first(&tile.floats, REPEATS) {
                (layer.repeats, layer.laid) = (f32::from_bits(repeats), true);
            }
            if let Some(third) = first(&tile.floats, THIRD_FLOAT) {
                record[row(FLAGS, at) + 2] = third;
            }
            if let Some(ninth) = first(&tile.floats, TILE_NINTH) {
                record[row(NINTH, at)] = ninth;
            }
            if let Some(flags) = first(&tile.words, TILE_FLAGS) {
                (layer.tile_flags, layer.flagged) = (flags, true);
            }
        }
        if let Some(definition) = layered.definitions.get(&material.layers[at]) {
            let rows = &definition.rows;
            if let Some(flags) = first(&rows.words, LAYER_FLAGS) {
                (layer.flags, layer.flagged) = (flags, true);
            }
            if let Some(repeats) = first(&rows.floats, REPEATS) {
                (layer.repeats, layer.laid) = (f32::from_bits(repeats), true);
            }
            for (name, to, dword) in KEPT {
                if let Some(kept) = first(&rows.floats, name) {
                    record[row(to, at) + dword] = kept;
                }
            }
            for (pair, name) in layer.pairs.iter_mut().zip([FIRST_PAIR, SECOND_PAIR]) {
                walk(&rows.floats, name, 1, &[0, 1], |_, slot, value| {
                    (pair.0[slot], pair.1) = (f32::from_bits(value), true);
                });
            }
            if let Some(angle) = first(&rows.floats, ANGLE) {
                (layer.angle, layer.laid) = (f32::from_bits(angle), true);
            }
            walk(&rows.floats, OFFSET, 1, &[0, 1], |_, slot, value| {
                (layer.offset[slot], layer.laid) = (f32::from_bits(value), true);
            });
            walk(&rows.colours, COLOUR, 1, &[0, 1, 2, 3], |_, slot, value| {
                layer.colours[slot] = Some(value);
            });
            // A layer with a third colour whose flags leave the bit clear has its second and
            // third change places.
            if layer.colours[2].is_some() && layer.flagged && layer.flags & NEVER == 0 {
                layer.colours.swap(1, 2);
            }
        }
        let textures = layered.textures(material, at).map(&mut *number);
        record[row(TEXTURES, at)..][..3].copy_from_slice(&textures[..3]);
        match material.flags & 1 {
            0 => record[row(SLOTS, at)..][..4].copy_from_slice(&textures[3..]),
            _ => record[row(SLOTS, at)] = textures[3],
        }
    }
    // The material's own rows, over what its layers' definitions gave.
    let rows = &material.rows;
    walk(&rows.floats, REPEATS, LAYERS, &[0], |at, _, value| {
        (layers[at].repeats, layers[at].laid) = (f32::from_bits(value), true);
    });
    for (name, to, dword) in KEPT {
        walk(&rows.floats, name, LAYERS, &[0], |at, _, value| {
            record[row(to, at) + dword] = value;
        });
    }
    for (pair, name) in [FIRST_PAIR, SECOND_PAIR].into_iter().enumerate() {
        walk(&rows.floats, name, LAYERS, &[0, 1], |at, slot, value| {
            let pair = &mut layers[at].pairs[pair];
            (pair.0[slot], pair.1) = (f32::from_bits(value), true);
        });
    }
    walk(&rows.floats, ANGLE, LAYERS, &[0], |at, _, value| {
        (layers[at].angle, layers[at].laid) = (f32::from_bits(value), true);
    });
    walk(&rows.floats, OFFSET, LAYERS, &[0, 1], |at, slot, value| {
        (layers[at].offset[slot], layers[at].laid) = (f32::from_bits(value), true);
    });
    walk(
        &rows.colours,
        COLOUR,
        LAYERS,
        &[0, 1, 2, 3],
        |at, slot, v| {
            layers[at].colours[slot] = Some(v);
        },
    );
    walk(&rows.words, LAYER_FLAGS, LAYERS, &[0], |at, _, value| {
        (layers[at].flags, layers[at].flagged) = (value, true);
    });
    record[MATERIAL_FLAGS / 4] = material.flags;
    walk(&rows.colours, WHOLE_COLOUR, 1, &[0, 1], |_, slot, value| {
        record[WHOLE_COLOURS / 4 + slot] = value;
    });
    for (at, layer) in layers.iter().enumerate() {
        if layer.flagged {
            record[row(FLAGS, at)] = (layer.tile_flags | layer.flags) & !NEVER;
        }
        // ponytail: the byte a model gives to pick between a pair's floats is not read yet, so
        // each pair gives its first; take the byte from the model once what sets it is read.
        let picked = |pair: usize| layer.pairs[pair].0[0];
        if layer.pairs[1].1 {
            record[row(PAIRS, at) + 2] = (picked(1) * picked(1)).to_bits();
        }
        if layer.pairs[0].1 {
            record[row(PAIRS, at) + 1] = picked(0).to_bits();
        }
        let first = f32::from_bits(record[row(PAIRS, at) + 1]);
        let scaled = &mut record[row(SCALED_ROW, at)];
        *scaled = (first * f32::from_bits(*scaled)).to_bits();
        if layer.colours[0].is_some() || layer.colours[1].is_some() {
            let shades = |colour: Option<u32>| shades(colour.unwrap_or(0));
            record[row(FIRST_COLOUR, at)..][..3].copy_from_slice(&shades(layer.colours[0]));
            record[row(SECOND_COLOUR, at)..][..3].copy_from_slice(&shades(layer.colours[1]));
        }
        if layer.laid {
            let [u, v] = transform(layer);
            record[row(TRANSFORM_U, at)..][..4].copy_from_slice(&u.map(f32::to_bits));
            record[row(TRANSFORM_V, at)..][..4].copy_from_slice(&v.map(f32::to_bits));
        }
    }
    record
}

/// The two rows that take a part's coordinates to a layer's tile: the tile repeated, moved, then
/// turned about its middle. The fourth of the first is the angle, of the second the repeats.
fn transform(layer: &Layer) -> [[f32; 4]; 2] {
    // The columns of a matrix of three rows.
    let mut columns = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0f32]];
    if layer.repeats != 1.0 {
        columns[0] = [layer.repeats, 0.0, 0.0];
        columns[1] = [0.0, layer.repeats, 0.0];
    }
    let moved = |c: [f32; 3], [u, v]: [f32; 2]| [c[0] + u * c[2], c[1] + v * c[2], c[2]];
    if layer.offset != [0.0, 0.0] {
        columns = columns.map(|c| moved(c, layer.offset));
    }
    if layer.angle != 0.0 {
        let (sin, cos) = layer.angle.sin_cos();
        columns = columns.map(|c| {
            let c = moved(c, [-0.5, -0.5]);
            let turned = [cos * c[0] + sin * c[1], cos * c[1] - sin * c[0], c[2]];
            moved(turned, [0.5, 0.5])
        });
    }
    let line =
        |lane: usize, last: f32| [columns[0][lane], columns[1][lane], columns[2][lane], last];
    [line(0, layer.angle), line(1, layer.repeats)]
}

/// `x` to the power 2.4 as the game computes it: a logarithm by a polynomial on the mantissa,
/// then a power of two by another on what is left of the exponent.
fn power(x: f32) -> f32 {
    let bits = x.to_bits();
    let exponent = ((bits >> 23) as i32 - 127) as f32;
    let m = f32::from_bits((bits & 0x007F_FFFF) | 0x3F80_0000);
    let log = ((((m * 0.059_651_55 - 0.465_725_63) * m + 1.481_166_5) * m - 2.520_749_6) * m
        + 2.888_270_4)
        * (m - 1.0)
        + exponent;
    let y = (log * 2.4).clamp(-126.999_99, 129.0);
    let whole = (y - 0.5).round_ties_even();
    let f = y - whole;
    let rest = (((f * 0.013_534_167 + 0.052_011_464) * f + 0.241_442_75) * f + 0.693_003_83) * f
        + 1.000_002_6;
    rest * 2f32.powi(whole as i32)
}

/// The three dwords a colour is kept as: a dark shade, the colour and a light shade, each of
/// linear values as bytes, red lowest. A colour with no alpha is three noughts.
pub(crate) fn shades(colour: u32) -> [u32; 3] {
    if colour <= 0x00FF_FFFF {
        return [0; 3];
    }
    let [r, g, b] = [0, 8, 16].map(|shift| {
        let byte = ((colour >> shift) & 255) as f32;
        match byte <= 0.040_45 * 255.0 {
            true => byte * 0.000_303_527,
            false => power(byte * 0.003_717_127 + 0.052_132_703),
        }
    });
    // To hue, saturation and value.
    let p = if g < b {
        [b, g, -1.0, 2.0 / 3.0]
    } else {
        [g, b, 0.0, -1.0 / 3.0]
    };
    let q = if r < p[0] {
        [p[0], p[1], p[3], r]
    } else {
        [r, p[1], p[2], p[0]]
    };
    let spread = q[0] - q[3].min(q[1]);
    let hue = (q[2] + (q[3] - q[1]) / (6.0 * spread + 1e-10)).abs();
    let (saturation, value) = (spread / (q[0] + 1e-10), q[0]);
    let lanes = [
        (6.0 * hue - 3.0).abs() - 1.0,
        2.0 - (6.0 * hue - 2.0).abs(),
        2.0 - (6.0 * hue - 4.0).abs(),
    ]
    .map(|lane| lane.clamp(0.0, 1.0) - 1.0);
    let packed = |saturation: f32, value: f32, alpha: u32| {
        let byte = |lane: f32| ((lane * saturation + 1.0) * value * 255.0 + 0.5).clamp(0.0, 255.0);
        let [r, g, b] = lanes.map(|lane| byte(lane) as u32);
        r | g << 8 | b << 16 | alpha << 24
    };
    [
        packed(saturation, 0.1 * value, 255),
        packed(saturation, value, colour >> 24),
        packed(0.55 * saturation, 0.55 * value + 0.45, 255),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn float(name: u32, value: f32, index: u8, layer: u8) -> Row {
        Row {
            name,
            value: value.to_bits(),
            index,
            layer,
        }
    }

    fn word(name: u32, value: u32, index: u8, layer: u8) -> Row {
        Row {
            name,
            value,
            index,
            layer,
        }
    }

    fn texture(name: u32, key: u64, index: u8, layer: u8) -> TextureRow {
        TextureRow {
            name,
            key,
            index,
            layer,
        }
    }

    /// A material of two layers on one definition, whose tile states two textures.
    fn layered() -> (Material, Layered) {
        let tile = Rows {
            floats: vec![float(REPEATS, 16.0, EVERY, EVERY)],
            words: vec![word(TILE_FLAGS, 0x21_0000, EVERY, EVERY)],
            textures: vec![
                texture(TILE_SECOND, 0xB, EVERY, EVERY),
                texture(TILE_MASK, 0xA, EVERY, EVERY),
            ],
            ..Rows::default()
        };
        let definition = Definition {
            tile: 7,
            rows: Rows {
                floats: vec![
                    float(SCALED, 0.5, EVERY, EVERY),
                    float(REFLECTANCE, 0.5, EVERY, EVERY),
                    float(SECOND_PAIR, 3.0, 0, EVERY),
                    float(FIRST_PAIR, 0.25, 0, EVERY),
                    float(FIRST_PAIR, 0.75, 1, EVERY),
                ],
                colours: vec![
                    word(COLOUR, 0xFF10_2030, 0, EVERY),
                    word(COLOUR, 0xFF40_5060, 1, EVERY),
                ],
                words: vec![word(LAYER_FLAGS, 0x20_0004, EVERY, EVERY)],
                ..Rows::default()
            },
        };
        let material = Material {
            flags: 0x820,
            layers: [9, 9, 0, 0, 0, 0, 0, 0],
            tiles: [0; LAYERS],
            rows: Rows {
                floats: vec![
                    float(REPEATS, 35.0, EVERY, 1),
                    float(REFLECTANCE, 2.0, EVERY, 1),
                ],
                colours: vec![word(COLOUR, 0xFF28_2725, 0, 1)],
                ..Rows::default()
            },
        };
        let layered = Layered {
            tiles: BTreeMap::from([(7, tile)]),
            definitions: BTreeMap::from([(9, definition)]),
            materials: BTreeMap::new(),
        };
        (material, layered)
    }

    #[test]
    fn a_material_s_own_row_is_taken_over_its_layer_s_definition_for_that_layer_alone() {
        let (material, layered) = layered();
        let record = record(&material, &layered, &mut |key| key as u32);
        let at = |at: usize, layer: usize| &record[(at + 16 * layer) / 4..][..4];
        assert_eq!(at(FIRST_COLOUR, 0)[..3], shades(0xFF10_2030));
        assert_eq!(at(FIRST_COLOUR, 1)[..3], shades(0xFF28_2725));
        assert_eq!(at(SECOND_COLOUR, 1)[..3], shades(0xFF40_5060));
        assert_eq!(at(FIRST_COLOUR, 2), [0; 4], "a layer the material has not");
        // The tile's two textures by their numbers, the mask first, and its flags with the
        // definition's less the bit the record never states.
        assert_eq!(at(TEXTURES, 0), [0xA, 0xB, 0, 0]);
        assert_eq!(at(FLAGS, 1)[0], 0x01_0004);
        assert_eq!(at(FLAGS, 0)[1], 0.5f32.to_bits());
        assert_eq!(at(FLAGS, 1)[1], 2f32.to_bits());
        // A pair gives its first float, the second pair's squared, and a float kept times the
        // first pair's.
        assert_eq!(at(PAIRS, 0)[1..3], [0.25f32, 9.0].map(f32::to_bits));
        assert_eq!(at(SCALED_ROW, 0)[0], 0.125f32.to_bits());
        assert_eq!(at(PAIRS, 2), [0; 4]);
        assert_eq!(record[MATERIAL_FLAGS / 4], 0x820);
        // The tile repeats sixteen times, and thirty-five where the material says so.
        let floats = |at: &[u32]| [at[0], at[1], at[2], at[3]].map(f32::from_bits);
        assert_eq!(floats(at(TRANSFORM_U, 0)), [16.0, 0.0, 0.0, 0.0]);
        assert_eq!(floats(at(TRANSFORM_V, 0)), [0.0, 16.0, 0.0, 16.0]);
        assert_eq!(floats(at(TRANSFORM_V, 1)), [0.0, 35.0, 0.0, 35.0]);
        assert_eq!(at(TRANSFORM_U, 2), [0; 4]);
    }

    #[test]
    fn a_row_for_every_layer_is_taken_once_by_the_first_layer_that_comes_to_it() {
        let rows = [
            word(COLOUR, 1, 0, 0),
            word(COLOUR, 2, 0, 4),
            word(COLOUR, 3, 0, EVERY),
        ];
        let mut taken = Vec::new();
        walk(&rows, COLOUR, LAYERS, &[0, 1], |layer, slot, value| {
            taken.push((layer, slot, value));
        });
        assert_eq!(taken, [(0, 0, 1), (4, 0, 2), (5, 0, 3)]);
    }

    #[test]
    fn a_texture_is_the_material_s_for_the_layer_then_the_definition_s_then_the_tile_s() {
        let (mut material, mut layered) = layered();
        material.tiles[1] = 8;
        let other = Rows {
            textures: vec![texture(TILE_MASK, 0xC, EVERY, EVERY)],
            ..Rows::default()
        };
        layered.tiles.insert(8, other);
        material.rows.textures = vec![texture(SLOT, 0xD, 1, 0)];
        let definition = layered.definitions.get_mut(&9).unwrap();
        definition.rows.textures = vec![texture(SLOT, 0xE, 0, EVERY), texture(SLOT, 0xF, 1, EVERY)];
        assert_eq!(
            layered.textures(&material, 0),
            [0xA, 0xB, 0, 0xE, 0xD, 0, 0]
        );
        // The second layer's tile is the one the material states in place of its definition's.
        assert_eq!(layered.textures(&material, 1), [0xC, 0, 0, 0xE, 0xF, 0, 0]);
        assert_eq!(layered.textures(&material, 2), [0; 7]);
    }

    #[test]
    fn a_tile_is_repeated_moved_then_turned_about_its_middle() {
        let layer = Layer {
            repeats: 2.0,
            angle: std::f32::consts::FRAC_PI_2,
            offset: [0.25, 0.0],
            ..Layer::default()
        };
        let [u, v] = transform(&layer);
        // (0.5, 0.5) repeats to (1, 1), moves to (1.25, 1) and turns a quarter about (0.5, 0.5).
        let at = |row: [f32; 4]| row[0] * 0.5 + row[1] * 0.5 + row[2];
        assert!((at(u) - 1.0).abs() < 1e-5 && (at(v) - -0.25).abs() < 1e-5);
        assert_eq!((u[3], v[3]), (layer.angle, 2.0));
    }

    #[test]
    fn a_colour_is_kept_as_a_dark_shade_itself_and_a_light_shade_in_linear_bytes() {
        assert_eq!(shades(0x00FF_FFFF), [0; 3], "no alpha, no colour");
        // A grey of 37, 39, 40 is 5 of 255 linear; its dark shade is a tenth, its light one
        // 0.55 of it and 0.45 more.
        let [dark, own, light] = shades(0xFF28_2725);
        assert_eq!(own, 0xFF05_0505);
        assert_eq!(dark, 0xFF01_0100);
        assert_eq!(light, 0xFF76_736D);
        assert_eq!(
            shades(0x80FF_FFFF)[1],
            0x80FF_FFFF,
            "white stays white, with its alpha"
        );
        // Red keeps its hue in each shade.
        let [dark, own, light] = shades(0xFF00_00FF);
        assert_eq!((dark, own), (0xFF00_001A, 0xFF00_00FF));
        assert_eq!(light, 0xFF73_73FF);
    }
}
