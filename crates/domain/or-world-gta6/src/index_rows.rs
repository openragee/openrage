//! What one archive gives the index, as its section of the kept file holds it: the rows a scan
//! found, with names unresolved and nothing that another archive decides.

use bytemuck::Pod;
use or_io::gta6::map::stream::grass::slod::SlodClaim;
use or_world::contract::WorldError;
use or_world::kept::{Section, SectionReader};

use super::lights_index::LightRow;
use super::scan::{self, Shard};
use super::{ArchetypeRow, MapRow};

/// A section of an archive the mount names none of the world's: nothing of it was read.
const NOT_READ: u8 = 0;
/// A section that holds the rows of a scan.
const READ: u8 = 1;

/// A map of one archive: its row, the names of its parents' maps, whether it holds something to draw, and the nested archive it lies in.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Map {
    /// Its archive and its run of parents are the link's to give.
    pub(super) row: MapRow,
    pub(super) parents: Vec<u32>,
    pub(super) holds: bool,
    pub(super) container: Option<u64>,
}

/// The rows of one archive.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Rows {
    pub(super) maps: Vec<Map>,
    pub(super) archetypes: Vec<ArchetypeRow>,
    pub(super) lights: Vec<LightRow>,
    /// The group a script-managed map states, with the 64-bit word the same name hashes to.
    pub(super) words: Vec<(u32, u64)>,
    /// The nested archive of each map-types body that lies in one.
    pub(super) types: Vec<u64>,
    /// The word each drawable or fragment body names itself by, with its offset.
    pub(super) models: Vec<(u64, u64)>,
    /// The word of each interior a map-types body states, with the body's offset.
    pub(super) interiors: Vec<(u64, u64)>,
    /// The codes of the drawables each dictionary body holds, with its offset.
    pub(super) keys: Vec<(u64, Vec<u32>)>,
    pub(super) unread: u64,
    pub(super) slod: Vec<SlodClaim>,
}

fn out_of_step(what: &'static str) -> WorldError {
    WorldError::Unread {
        what: "a section of the kept index of a GTA VI install".into(),
        why: format!("{what} are out of step"),
    }
}

pub(super) fn put_pod<T: Pod>(out: &mut Section, rows: &[T]) {
    out.put_bytes(bytemuck::cast_slice(rows));
}

/// The rows of the next run of bytes, refused where the bytes are no whole number of rows.
pub(super) fn pod<T: Pod>(from: &mut SectionReader<'_>) -> Result<Vec<T>, WorldError> {
    let bytes = from.bytes()?;
    match bytes.len() % std::mem::size_of::<T>() {
        // The bytes lie where the section put them, at no alignment: they are copied out.
        0 => Ok(bytemuck::pod_collect_to_vec(bytes)),
        _ => Err(out_of_step("the bytes of a table")),
    }
}

fn put_pairs(out: &mut Section, pairs: &[(u64, u64)]) {
    let flat: Vec<u64> = pairs.iter().flat_map(|(a, b)| [*a, *b]).collect();
    put_pod(out, &flat);
}

fn pairs(from: &mut SectionReader<'_>) -> Result<Vec<(u64, u64)>, WorldError> {
    let flat: Vec<u64> = pod(from)?;
    if !flat.len().is_multiple_of(2) {
        return Err(out_of_step("the pairs of a table"));
    }
    Ok((flat.as_chunks::<2>().0.iter())
        .map(|p| (p[0], p[1]))
        .collect())
}

impl Rows {
    /// What the index keeps of `shard`, each body placed in the nested archive that holds it.
    pub(super) fn of(shard: Shard) -> Self {
        let nested = shard.nested();
        let types = (shard.types.iter())
            .filter_map(|offset| scan::container_of(&nested, *offset))
            .collect();
        let maps = (shard.maps.into_iter())
            .map(|(row, parents, holds)| Map {
                container: scan::container_of(&nested, row.offset),
                row,
                parents,
                holds,
            })
            .collect();
        Self {
            maps,
            archetypes: shard.archetypes,
            lights: shard.lights,
            words: shard.words,
            types,
            models: shard.models,
            interiors: shard.interiors,
            keys: shard.keys,
            unread: shard.unread,
            slod: shard.slod,
        }
    }

    /// Write the section of an archive that holds none of the world.
    pub(super) fn put_none(out: &mut Section) {
        out.put_u8(NOT_READ);
    }

    /// Write the rows as a section.
    pub(super) fn put(&self, out: &mut Section) {
        out.put_u8(READ);
        self.put_body(out);
    }

    /// Write the rows where a section already says what they are.
    pub(super) fn put_body(&self, out: &mut Section) {
        let rows: Vec<MapRow> = self.maps.iter().map(|m| m.row).collect();
        put_pod(out, &rows);
        let counts: Vec<u32> = (self.maps.iter()).map(|m| m.parents.len() as u32).collect();
        put_pod(out, &counts);
        let parents: Vec<u32> = (self.maps.iter())
            .flat_map(|m| m.parents.iter().copied())
            .collect();
        put_pod(out, &parents);
        let holds: Vec<u8> = self.maps.iter().map(|m| u8::from(m.holds)).collect();
        out.put_bytes(&holds);
        // A body at the archive's own level lies in no nested archive, and none starts at 0.
        let containers: Vec<u64> = (self.maps.iter())
            .map(|m| m.container.unwrap_or(0))
            .collect();
        put_pod(out, &containers);
        put_pod(out, &self.archetypes);
        put_pod(out, &self.lights);
        let words: Vec<(u64, u64)> = (self.words.iter())
            .map(|(group, word)| (u64::from(*group), *word))
            .collect();
        put_pairs(out, &words);
        put_pod(out, &self.types);
        put_pairs(out, &self.models);
        put_pairs(out, &self.interiors);
        out.put_u64(self.keys.len() as u64);
        for (offset, codes) in &self.keys {
            out.put_u64(*offset);
            put_pod(out, codes);
        }
        out.put_u64(self.unread);
        out.put_u64(self.slod.len() as u64);
        for claim in &self.slod {
            put_pod(out, &claim.ids);
            put_pod(out, &claim.words);
        }
    }

    /// The rows `section` holds; `None` for the section of an archive that was not read.
    pub(super) fn read(section: &Section) -> Result<Option<Self>, WorldError> {
        let mut from = section.reader();
        let rows = match from.u8()? {
            NOT_READ => None,
            READ => Some(Self::read_body(&mut from)?),
            _ => return Err(out_of_step("the kinds of section")),
        };
        from.end()?;
        Ok(rows)
    }

    /// The rows [`Self::put_body`] wrote, read from where `from` stands.
    pub(super) fn read_body(from: &mut SectionReader<'_>) -> Result<Self, WorldError> {
        let rows: Vec<MapRow> = pod(from)?;
        let counts: Vec<u32> = pod(from)?;
        let parents: Vec<u32> = pod(from)?;
        let holds = from.bytes()?;
        let containers: Vec<u64> = pod(from)?;
        let stated: u64 = counts.iter().map(|count| u64::from(*count)).sum();
        let each = [counts.len(), holds.len(), containers.len()];
        if each != [rows.len(); 3] || stated != parents.len() as u64 {
            return Err(out_of_step("the maps and their parents"));
        }
        let mut parents = parents.into_iter();
        let maps = (rows.into_iter().zip(counts).zip(holds).zip(containers))
            .map(|(((row, count), holds), container)| Map {
                row,
                parents: parents.by_ref().take(count as usize).collect(),
                holds: *holds != 0,
                container: Some(container).filter(|at| *at != 0),
            })
            .collect();
        let archetypes = pod(from)?;
        let lights = pod(from)?;
        let words = (pairs(from)?.into_iter())
            .map(|(group, word)| Some((u32::try_from(group).ok()?, word)))
            .collect::<Option<_>>()
            .ok_or_else(|| out_of_step("the groups and their words"))?;
        let types = pod(from)?;
        let models = pairs(from)?;
        let interiors = pairs(from)?;
        let mut keys = Vec::new();
        for _ in 0..from.u64()? {
            keys.push((from.u64()?, pod(from)?));
        }
        let unread = from.u64()?;
        let mut slod = Vec::new();
        for _ in 0..from.u64()? {
            slod.push(SlodClaim {
                ids: pod(from)?,
                words: pod(from)?,
            });
        }
        Ok(Self {
            maps,
            archetypes,
            lights,
            words,
            types,
            models,
            interiors,
            keys,
            unread,
            slod,
        })
    }
}
