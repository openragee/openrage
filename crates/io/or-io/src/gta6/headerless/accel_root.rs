//! What the root of shipped acceleration structures states of the model they were built from:
//! its key and, a record, the level and the model the record's structure is of, with the words
//! of the structure's head the ray tracer's selection is checked against.

use super::instances::read_instances;
use super::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccelRoot {
    /// The key of the selection of record 0's level.
    pub key: u64,
    /// One a model the ray tracer draws, a level after another, in the file's order of models; a
    /// root may hold the records of more levels than the one its key is of.
    pub records: Vec<AccelRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccelRecord {
    /// Bits 22 and 23 of the record's word: the level of detail.
    pub level: u8,
    /// Bits 18 to 21: the low four bits of the model's number among its level's models.
    pub number: u8,
    /// Bits 8 to 17: the geometries described to the builder.
    pub geometries: u16,
    /// `None` for a record of size 0: a model that keeps no geometry.
    pub structure: Option<AccelHead>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccelHead {
    /// The CRC-32C of the build description.
    pub checksum: u32,
    /// The structure's leaves, each one triangle or two.
    pub elements: u32,
}

/// The root of a body of shipped acceleration structures, read as [`read_instances`] reads it.
pub fn read_accel_root(virtual_section: &[u8], physical: Option<&[u8]>) -> Result<AccelRoot> {
    let root = read_instances(virtual_section, physical)?;
    let records = root.instances.iter().map(|instance| AccelRecord {
        level: ((instance.word >> 22) & 3) as u8,
        number: ((instance.word >> 18) & 0xF) as u8,
        geometries: ((instance.word >> 8) & 0x3FF) as u16,
        structure: instance.structure.map(|head| AccelHead {
            checksum: head.word_108,
            elements: head.primitives,
        }),
    });
    Ok(AccelRoot {
        key: root.key,
        records: records.collect(),
    })
}
