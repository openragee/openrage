//! Which triangles of a drawable the game hands its ray tracer: one level of detail and, a
//! structure a model the ray tracer's phase draws, the geometries whose shader that phase draws
//! in the opaque or the cutout bucket; with the key and the checksums a shipped acceleration
//! structure states of that choice.

use super::Triangle;
use crate::gta6::headerless::{Drawable, TraceModel, TraceWords};

/// The bit of a model's render mask for the ray tracer's phase; bit 13 of a shader's bucket mask
/// is the same phase.
const TRACED_MODEL: u8 = 0x20;
/// The ray tracer's phase and the opaque bucket.
const OPAQUE: u32 = 0x2001;
/// The ray tracer's phase and the cutout bucket.
const CUTOUT: u32 = 0x2008;
/// What a key adds to the fold of its key words.
const KEY_BIAS: u64 = 6;
/// The index format a build description states: unsigned 16-bit.
const INDEX_FORMAT: u32 = 57;
/// The annotation of an effect whose geometry is described twice.
const TWICE: u32 = 0xD8DB_745C;
/// The annotations of the three kinds of effect the game does not ask [`TWICE`] of.
const NEVER_TWICE: [u32; 3] = [0xC572_8B5C, 0xEEF3_F4AA, 0xA526_95D1];

/// What the ray tracer is given of one level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub level: usize,
    /// The key the root of the shipped structures of this level states.
    pub key: u64,
    /// One bottom-level structure a model the ray tracer draws, in the file's order, which is the
    /// order of a shipped root's records for the level.
    pub structures: Vec<SelectedModel>,
}

/// One model's structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedModel {
    /// An index into the level's models.
    pub model: usize,
    /// In the file's order, which is the order of the structure's geometries; none where the
    /// model keeps no geometry, whose record a shipped root holds with no structure.
    pub geometries: Vec<Selected>,
    /// The checksum the head of a shipped structure of these geometries states.
    pub checksum: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selected {
    /// An index into the model's geometries.
    pub geometry: usize,
    /// False for a cutout geometry, whose alpha a tracing program tests itself.
    pub opaque: bool,
    /// The second of the two descriptions of a geometry described twice, which the game marks.
    pub second: bool,
}

fn traced(model: &TraceModel) -> bool {
    model.render_mask & TRACED_MODEL != 0
}

/// Whether the game describes a geometry of an effect twice, from the annotations of the effect's
/// second variable table: the first word of each is a 30-bit name hash over two bits of type.
pub fn described_twice(annotations: &[(u32, u32)]) -> bool {
    let holds = |key: u32| (annotations.iter()).any(|(word, _)| word >> 2 == key & 0x3FFF_FFFF);
    holds(TWICE) && !NEVER_TWICE.into_iter().any(holds)
}

/// The level the ray tracer is built from: the one the drawable states where it has it, else
/// level 0, or level 1 where `reduced` and a model of it is traced; never one before the first of
/// levels 0 to 2 with a traced model.
pub fn level(words: &TraceWords, reduced: bool) -> Option<usize> {
    // ⚠ `reduced` is the game's answer for the model's archetype under a setting, which no
    // drawable states; the root of a shipped structure states the level that was taken. The game
    // also asks something of each level before it looks for a traced model in it; what is not
    // known, and it is taken here as always granted.
    let traced_in = |l: usize| {
        words.levels[l]
            .as_ref()
            .is_some_and(|m| m.iter().any(traced))
    };
    let first = (0..3).find(|&l| traced_in(l))?;
    let stated = ((words.level_word >> 9) & 7) as usize;
    let stated = (stated.checked_sub(1).map(|l| l.min(3))).filter(|&l| words.levels[l].is_some());
    Some(first.max(stated.unwrap_or(usize::from(reduced && traced_in(1)))))
}

/// What the ray tracer is given of the drawable, `twice` saying of an effect's name hash whether
/// its geometry is [`described_twice`]; `None` where no level has a model it draws.
pub fn select(words: &TraceWords, reduced: bool, twice: impl Fn(u32) -> bool) -> Option<Selection> {
    select_level(words, level(words, reduced)?, twice)
}

/// What the ray tracer is given of level `level`; `None` where the drawable has no such level or
/// a geometry names a shader the drawable lacks.
pub fn select_level(
    words: &TraceWords,
    level: usize,
    twice: impl Fn(u32) -> bool,
) -> Option<Selection> {
    let models = words.levels.get(level)?.as_ref()?;
    let mut structures = Vec::new();
    let mut key = 0;
    for (m, model) in models.iter().enumerate().filter(|(_, m)| traced(m)) {
        let mut geometries = Vec::new();
        let mut description = vec![0u32];
        let mut folded = 0;
        for (g, geometry) in model.geometries.iter().enumerate() {
            let shader = words.shaders.get(usize::from(geometry.shader))?;
            let mask = shader.bucket_mask;
            let (opaque, cutout) = (mask & OPAQUE == OPAQUE, mask & CUTOUT == CUTOUT);
            if !opaque && !cutout {
                continue;
            }
            folded = fold(folded, geometry.key_word >> 9);
            let copies = 1 + usize::from(twice(shader.name_hash));
            for second in [false, true].into_iter().take(copies) {
                geometries.push(Selected {
                    geometry: g,
                    opaque: !cutout,
                    second,
                });
                description.extend([
                    u32::from(!cutout),
                    INDEX_FORMAT,
                    3 * geometry.triangle_count,
                    geometry.vertex_count,
                ]);
            }
        }
        description[0] = geometries.len() as u32;
        key = fold(key, folded);
        structures.push(SelectedModel {
            model: m,
            geometries,
            checksum: crc32c(description.iter().flat_map(|word| word.to_le_bytes())),
        });
    }
    Some(Selection {
        level,
        key: key.wrapping_add(KEY_BIAS),
        structures,
    })
}

/// `next` folded into `folded`; the first word folded is itself.
fn fold(folded: u64, next: u64) -> u64 {
    const MUL: u64 = 0x9DDF_EA08_EB38_2D69;
    if folded == 0 {
        return next;
    }
    let a = (folded.wrapping_add(0x651E_95C4_D06F_BFB1) ^ next).wrapping_mul(MUL);
    let b = ((a >> 47) ^ next ^ a).wrapping_mul(MUL);
    ((b >> 47) ^ b).wrapping_mul(MUL)
}

/// CRC-32C from a register of zero, with no final inversion.
fn crc32c(bytes: impl Iterator<Item = u8>) -> u32 {
    bytes.fold(0, |crc, byte| {
        (0..8).fold(crc ^ u32::from(byte), |c, _| {
            (c >> 1) ^ (0x82F6_3B78 & (c & 1).wrapping_neg())
        })
    })
}

/// The triangles of one structure of a selection, as its bottom level is built from them.
#[derive(Debug, Clone, PartialEq)]
pub struct Traced {
    /// An index into the level's models.
    pub model: usize,
    /// A geometry is numbered by its description's place in the structure and a primitive by
    /// its triangle's place in its geometry: the two numbers a hit states.
    pub triangles: Vec<Triangle>,
    /// The descriptions whose geometry, or its positions, did not read.
    pub unread: usize,
    /// The triangles that name a vertex their geometry lacks or one that is no finite number.
    pub left_out: usize,
}

/// The triangles of each structure of `selection` that keeps geometry, from the `drawable` the
/// selection's words were read of; `None` where the drawable lacks a level, a model or a geometry
/// the selection names.
pub fn triangles(selection: &Selection, drawable: &Drawable) -> Option<Vec<Traced>> {
    // ⚠ A geometry described twice gives its triangles twice, under two geometry numbers, both
    // in the model's winding: whether the game's second description is the other winding is not
    // known.
    let models = drawable.levels.get(selection.level)?.as_ref()?;
    let kept = (selection.structures.iter()).filter(|s| !s.geometries.is_empty());
    kept.map(|structure| {
        let model = models.get(structure.model)?;
        let mut traced = Traced {
            model: structure.model,
            triangles: Vec::new(),
            unread: 0,
            left_out: 0,
        };
        for (number, selected) in structure.geometries.iter().enumerate() {
            let geometry = model.geometries.get(selected.geometry)?.as_ref().ok();
            let Some((geometry, positions)) =
                geometry.and_then(|g| Some((g, g.positions.as_ref().ok()?)))
            else {
                traced.unread += 1;
                continue;
            };
            for (primitive, triangle) in geometry.triangles.iter().enumerate() {
                let corners = triangle.map(|vertex| positions.get(vertex as usize).copied());
                match corners {
                    [Some(a), Some(b), Some(c)]
                        if [a, b, c].iter().flatten().all(|x| x.is_finite()) =>
                    {
                        traced.triangles.push(Triangle {
                            corners: [a, b, c],
                            geometry: number as u32,
                            primitive: primitive as u32,
                            opaque: selected.opaque,
                        });
                    }
                    _ => traced.left_out += 1,
                }
            }
        }
        Some(traced)
    })
    .collect()
}

/// The name hashes of the effects of `container` whose geometry is [`described_twice`].
#[cfg(feature = "fxc")]
pub fn twice_described(
    container: &crate::gta5::shaders::sgd5::Sgd5File,
) -> std::collections::HashSet<u32> {
    let twice = (container.effects.iter()).filter(|e| described_twice(&e.second.annotations));
    twice
        .map(|e| crate::gta6::hash::name_hash32(e.name.as_bytes()))
        .collect()
}

#[cfg(test)]
#[path = "select_tests.rs"]
mod tests;
