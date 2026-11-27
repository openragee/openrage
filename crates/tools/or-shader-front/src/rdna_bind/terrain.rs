//! The buffers a terrain tile's stages read, as the game binds them for a tile whose own record
//! is not ready yet: a record, one layer and the constants, each built once from three textures
//! of one texel, so that a tile draws its own programs over its own geometry.

use naga::{Block, Expression as E, ImageClass, ScalarKind, Statement as S, TypeInner};
use or_io::gta5::shaders::sgd5::Sgd5Table;

use super::plain::each_block;
use super::shading::{heap_root, Fed, Feed, Source, Unfed};
use super::{join, numbered, Join, Place, Record};
use crate::dxil::ShaderKind;
use crate::rdna::{Index, IndexSource, Resource, ResourceKind};

/// The tile's record a stage reads by the hash of its name: 372 bytes, [`TileRecord`].
pub const RECORD: u32 = 0x5CA7_CB60;
/// The tile's layers: 128 bytes a layer, [`LayerRecord`].
pub const LAYERS: u32 = 0xFC4C_C2EF;
/// The tile's 64 cells, a dword each: where each cell's pages lie in the virtual texture.
pub const CELLS: u32 = 0xFE9D_6ABC;
/// The terrain's constant buffer: 32 bytes, [`Constants`].
pub const CONSTANTS: u32 = 0xE224_0C86;

/// The three textures of one texel the game builds the defaults from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DefaultTexture {
    /// Black, alpha one.
    Black,
    /// The flat normal: half, half, one, alpha one.
    Flat,
    /// Nought in every channel, alpha too.
    Clear,
}

/// The first heap entry the defaults take: past every entry a material's number picks, since a
/// number is a register's low byte and number `i` picks entry `i + 1`.
const FIRST_ENTRY: u32 = 257;

impl DefaultTexture {
    pub const ALL: [Self; 3] = [Self::Black, Self::Flat, Self::Clear];

    /// Its texel as eight-bit red, green, blue and alpha, as the game states it.
    pub fn texel(self) -> [u8; 4] {
        match self {
            Self::Black => [0, 0, 0, 255],
            Self::Flat => [128, 128, 255, 255],
            Self::Clear => [0, 0, 0, 0],
        }
    }

    /// The heap entry the editor puts it at, which the buffers state as its number.
    pub fn entry(self) -> u32 {
        FIRST_ENTRY + self as u32
    }
}

/// What the editor gives a binding of a terrain stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Terrain {
    /// The default [`TileRecord`].
    Record,
    /// One default [`LayerRecord`].
    Layers,
    /// The default [`Constants`].
    Constants,
    /// A texture the stage picks by a number a default buffer states.
    Texture(DefaultTexture),
    /// The feedback texture the stage writes and no stage reads: its writes are taken out.
    Feedback,
}

/// The record of a tile, as its stages read it.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileRecord {
    /// Three rows of four floats, which a tile's pixel stage does not read.
    pub rows: [f32; 12],
    /// The tile's origin in metres, written as whole 32 m steps and a remainder.
    pub origin: [f32; 4],
    /// Texture numbers of the sixteen sub-tiles of each of four kinds, read where a page is missing.
    pub sub_tiles: [[u32; 16]; 4],
    /// Number of the blend-weight map: three weights, the fourth what they leave of one.
    pub weights: u32,
    /// Number of the layer-index map: each channel times 255 is a layer record's index.
    pub indices: u32,
    /// A texture number whose reader is not known.
    pub at_328: u32,
    /// Number of the first tint map, 128 texels, sampled inside a border of one texel.
    pub tint_a: u32,
    /// Number of the second tint map, 256 texels, sampled inside a border of one texel.
    pub tint_b: u32,
    /// Number of a texture the tile program samples, for a purpose not read.
    pub at_340: u32,
    /// A texture number whose reader is not known.
    pub at_344: u32,
    /// Number of a texture the tile program samples, for a purpose not read.
    pub at_348: u32,
    /// A texture number whose reader is not known.
    pub at_352: u32,
    /// Number of an array texture sampled four times at a rounded slice, for a purpose not read.
    pub at_356: u32,
    /// The last sub-tile index, 15 or nought; with nought the tile's coordinates are used whole.
    pub last_sub_tile: u32,
    /// Not read by the tile's programs.
    pub tail: [u32; 2],
}

/// One layer of a tile, as the bake and the tile program read it.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerRecord {
    /// Times the layer repeats across the tile, in u and v.
    pub scale: [f32; 2],
    /// Added to the scaled coordinates, in repeats.
    pub offset: [f32; 2],
    /// Where 1.0 the scale and offset are used; otherwise a scale of 64 and no offset.
    pub own_tiling: f32,
    /// How much of the second tint is laid on.
    pub tint_b_amount: f32,
    /// What the second tint is multiplied by.
    pub tint_b_gain: f32,
    /// The normal map's strength, applied where it is not 1.0.
    pub normal_strength: f32,
    /// What the height above the bias is multiplied by in the blend.
    pub height_scale: f32,
    /// The height taken off before the blend, in the height map's units.
    pub height_bias: f32,
    /// The blend's sharpness: the exponent is `20 − 19·saturate((v − 0.2)·1.7544)`.
    pub sharpness: f32,
    /// The layer's share of the bake's sixth channel.
    pub sixth_share: f32,
    /// How much of the first tint is laid on.
    pub tint_a_amount: f32,
    /// What the first tint is multiplied by.
    pub tint_a_gain: f32,
    /// Not read by the bake.
    pub at_56: [f32; 4],
    /// Two floats the defaults state, whose reader is not known.
    pub at_72: [f32; 2],
    /// Read by the tile program alone, for a purpose not read.
    pub at_80: f32,
    /// Not read by the bake.
    pub at_84: [u32; 7],
    /// Number of the layer's albedo map.
    pub albedo: u32,
    /// Number of the layer's normal map.
    pub normal: u32,
    /// Number of the layer's map of height and parameters.
    pub height: u32,
    /// Not read by the bake.
    pub at_124: u32,
}

/// The terrain's constants.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Constants {
    /// Numbers of the three atlases a page's albedo, normal and height lie in.
    pub atlases: [u32; 3],
    /// Number of the page table.
    pub page_table: u32,
    /// Number of the feedback texture the tile program writes.
    pub feedback: u32,
    /// The bias of the level a page is read at, in levels.
    pub level_bias: f32,
    /// Where its low byte is nought, four constants of the stage's gate are multiplied by nought.
    pub switch: u32,
    /// Not stated by the defaults.
    pub at_28: u32,
}

const _: () = assert!(std::mem::size_of::<TileRecord>() == 372);
const _: () = assert!(std::mem::size_of::<LayerRecord>() == 128);
const _: () = assert!(std::mem::size_of::<Constants>() == 32);

fn bits<const N: usize>(floats: [f32; N]) -> [u32; N] {
    floats.map(f32::to_bits)
}

impl TileRecord {
    /// Its dwords, in the order of its fields.
    pub fn words(&self) -> Vec<u32> {
        let mut out = Vec::with_capacity(93);
        out.extend(bits(self.rows));
        out.extend(bits(self.origin));
        out.extend(self.sub_tiles.iter().flatten());
        out.extend([
            self.weights,
            self.indices,
            self.at_328,
            self.tint_a,
            self.tint_b,
            self.at_340,
            self.at_344,
            self.at_348,
            self.at_352,
            self.at_356,
            self.last_sub_tile,
        ]);
        out.extend(self.tail);
        out
    }
}

impl LayerRecord {
    /// Its dwords, in the order of its fields.
    pub fn words(&self) -> Vec<u32> {
        let mut out = Vec::with_capacity(32);
        out.extend(bits(self.scale));
        out.extend(bits(self.offset));
        out.extend(bits([
            self.own_tiling,
            self.tint_b_amount,
            self.tint_b_gain,
            self.normal_strength,
            self.height_scale,
            self.height_bias,
            self.sharpness,
            self.sixth_share,
            self.tint_a_amount,
            self.tint_a_gain,
        ]));
        out.extend(bits(self.at_56));
        out.extend(bits(self.at_72));
        out.push(self.at_80.to_bits());
        out.extend(self.at_84);
        out.extend([self.albedo, self.normal, self.height, self.at_124]);
        out
    }
}

impl Constants {
    /// Its dwords, in the order of its fields.
    pub fn words(&self) -> Vec<u32> {
        let mut out = self.atlases.to_vec();
        out.extend([
            self.page_table,
            self.feedback,
            self.level_bias.to_bits(),
            self.switch,
            self.at_28,
        ]);
        out
    }
}

/// The record the game binds for a tile with no record of its own: no sub-tiles, and an index
/// map and a weight map of [`DefaultTexture::Clear`], so every pixel is the first layer whole.
/// `number` gives each texture's number.
pub fn default_record(number: impl Fn(DefaultTexture) -> u32) -> TileRecord {
    let (black, flat, clear) = (
        number(DefaultTexture::Black),
        number(DefaultTexture::Flat),
        number(DefaultTexture::Clear),
    );
    TileRecord {
        rows: [0.0; 12],
        origin: [0.0; 4],
        sub_tiles: [[black; 16], [flat; 16], [black; 16], [flat; 16]],
        weights: clear,
        indices: clear,
        at_328: black,
        tint_a: black,
        tint_b: black,
        at_340: black,
        at_344: black,
        at_348: black,
        at_352: black,
        at_356: black,
        last_sub_tile: 0,
        tail: [0; 2],
    }
}

/// The one layer the game binds with that record.
pub fn default_layer(number: impl Fn(DefaultTexture) -> u32) -> LayerRecord {
    LayerRecord {
        scale: [128.0; 2],
        offset: [0.0; 2],
        own_tiling: 1.0,
        tint_b_amount: 1.0,
        tint_b_gain: 1.0,
        normal_strength: 1.0,
        height_scale: 1.0,
        height_bias: 0.0,
        sharpness: 0.0,
        sixth_share: 0.0,
        tint_a_amount: 0.0,
        tint_a_gain: 1.0,
        at_56: [0.0; 4],
        at_72: [24.0; 2],
        at_80: 0.2,
        at_84: [0; 7],
        albedo: number(DefaultTexture::Black),
        normal: number(DefaultTexture::Flat),
        height: number(DefaultTexture::Black),
        at_124: 0,
    }
}

/// The constants the game binds with that record.
pub fn default_constants(number: impl Fn(DefaultTexture) -> u32) -> Constants {
    let black = number(DefaultTexture::Black);
    Constants {
        atlases: [black; 3],
        page_table: black,
        feedback: black,
        level_bias: 2.0,
        switch: 0,
        at_28: 0,
    }
}

/// What the editor gives a buffer only the game fills, by the hash of its name, where it is one of
/// the terrain's whose default is read.
pub(super) fn buffer(hash: u32) -> Option<Feed> {
    let part = match hash {
        RECORD => Terrain::Record,
        LAYERS => Terrain::Layers,
        CONSTANTS => Terrain::Constants,
        _ => return None,
    };
    Some(Feed::Terrain(part))
}

/// The default texture whose number the default buffer of `hash` states at byte `offset`.
fn texture_at(hash: u32, offset: u32) -> Option<DefaultTexture> {
    let words = match hash {
        RECORD => default_record(DefaultTexture::entry).words(),
        CONSTANTS => default_constants(DefaultTexture::entry).words(),
        _ => return None,
    };
    if !offset.is_multiple_of(4) {
        return None;
    }
    let word = *words.get((offset / 4) as usize)?;
    DefaultTexture::ALL.into_iter().find(|t| t.entry() == word)
}

/// What fills `resource`, an entry of the heap a stage picks by the number at a stated place of
/// the terrain's record or constants; `None` where it is no such entry.
pub(super) fn picked(
    resource: &Resource,
    kind: ShaderKind,
    records: &[Record],
    table: &Sgd5Table,
) -> Option<Feed> {
    if !heap_root(resource, kind, records) {
        return None;
    }
    let Some(Index {
        source:
            IndexSource::Data {
                buffer,
                offset,
                plus: None,
            },
        shift: 5,
    }) = resource.at.as_ref()?.index.as_ref()
    else {
        return None;
    };
    if !buffer.path.is_empty() || buffer.index.is_some() {
        return None;
    }
    let place = numbered(
        Place::Table {
            root: buffer.root,
            offset: buffer.offset,
        },
        kind,
    );
    let Join::Named { named, .. } = join(place, records, table) else {
        return None;
    };
    let texture = texture_at(named.name_hash, *offset)?;
    Some(Feed::Terrain(match resource.kind {
        ResourceKind::StorageTexture { .. } => Terrain::Feedback,
        _ => Terrain::Texture(texture),
    }))
}

/// Whether a stage of these bindings reads the terrain's layers, whose textures it picks from
/// the heap by the numbers the layer states.
pub(super) fn layered(filled: &[Source]) -> bool {
    filled.contains(&Source::Game(LAYERS))
}

/// The heap entries a layered stage's heap holds besides the material's, and what fills each.
pub(super) fn heap(layered: bool) -> Vec<(u32, Feed)> {
    if !layered {
        return Vec::new();
    }
    (DefaultTexture::ALL.into_iter())
        .map(|texture| (texture.entry(), Feed::Terrain(Terrain::Texture(texture))))
        .collect()
}

/// `module`, fed `feeds`, held to what the terrain's defaults establish: refused where it reads
/// the tile's cells, which the game binds only with a tile's own record, unless `unguarded`; the
/// feedback texture's writes taken out.
pub(super) fn held(module: &mut naga::Module, feeds: &[Fed], unguarded: bool) -> Result<(), Unfed> {
    if !unguarded && feeds.iter().any(|fed| fed.feed == Feed::Zero(Some(CELLS))) {
        return Err(Unfed::TerrainCells);
    }
    for fed in feeds.iter() {
        if fed.feed == Feed::Terrain(Terrain::Feedback) {
            unwritten(module, (fed.group, fed.binding))?;
        }
    }
    Ok(())
}

/// Take out every store to the texture `module` binds at `at` and bind it as a texture that is
/// sampled, which nothing then reads; refused where the module reads the texture back.
pub(super) fn unwritten(module: &mut naga::Module, at: (u32, u32)) -> Result<(), Unfed> {
    let found = module
        .global_variables
        .iter()
        .find(|(_, global)| (global.binding.as_ref()).is_some_and(|b| (b.group, b.binding) == at));
    let Some((global, _)) = found else {
        return Ok(());
    };
    let TypeInner::Image {
        dim,
        arrayed,
        class: ImageClass::Storage { .. },
    } = module.types[module.global_variables[global].ty].inner
    else {
        return Ok(());
    };
    let functions = (module.functions.iter_mut().map(|(_, f)| f))
        .chain(module.entry_points.iter_mut().map(|e| &mut e.function));
    for function in functions {
        let is_it = |expressions: &naga::Arena<E>, image: naga::Handle<E>| {
            expressions[image] == E::GlobalVariable(global)
        };
        let read = (function.expressions.iter()).any(|(_, e)| match *e {
            E::ImageLoad { image, .. } => is_it(&function.expressions, image),
            _ => false,
        });
        let mut updated = false;
        let expressions = &function.expressions;
        each_block(&mut function.body, &mut |block| {
            let stores: Vec<usize> = (block.iter().enumerate())
                .filter(|(_, statement)| match statement {
                    S::ImageStore { image, .. } => is_it(expressions, *image),
                    S::ImageAtomic { image, .. } => {
                        updated |= is_it(expressions, *image);
                        false
                    }
                    _ => false,
                })
                .map(|(place, _)| place)
                .collect();
            for place in stores.into_iter().rev() {
                block.splice(place..=place, Block::new());
            }
        });
        if read || updated {
            return Err(Unfed::TerrainFeedback);
        }
    }
    let sampled = module.types.insert(
        naga::Type {
            name: None,
            inner: TypeInner::Image {
                dim,
                arrayed,
                class: ImageClass::Sampled {
                    kind: ScalarKind::Float,
                    multi: false,
                },
            },
        },
        naga::Span::UNDEFINED,
    );
    module.global_variables.get_mut(global).ty = sampled;
    Ok(())
}

#[cfg(test)]
mod tests;
