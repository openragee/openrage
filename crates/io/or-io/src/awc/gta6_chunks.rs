//! Three chunks a GTA VI stream may carry beside its audio: level records, a stored animation
//! resource and markers, one of which states a radio track's id.

use std::io::{Read, Seek, SeekFrom};

use super::{stream_table_offset, AwcChunkKind, AwcStream, AWC_MAGIC_LE, HEADER_BYTES};
use crate::container::rpf::rsc8::Rsc8;
use crate::error::{IoError, Result};

const LOUDNESS_RECORD: usize = 8;
const MARKER_RECORD: usize = 16;
/// The name of the marker whose value is a radio track's id.
const TRACK_ID: u32 = crate::hash::string_hash(b"trackid");
/// The most streams, and the most bytes of one markers chunk, [`read_markers`] reads.
const MAX_STREAMS: usize = 1 << 16;
const MAX_MARKERS: usize = 1 << 20;

/// One record of a stream's [`AwcChunkKind::Markers`] chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AwcMarker {
    /// The hash of the marker's name.
    pub name: u32,
    /// As stated: a float's bits under some names, a name's hash under others.
    pub value: u32,
    /// The sample the marker stands at.
    pub position: u32,
}

impl AwcMarker {
    /// The radio track id the marker states: the value of a `trackid` marker, a whole float.
    pub fn track_id(&self) -> Option<u32> {
        let id = f32::from_bits(self.value);
        let whole = (0.0..16_777_216.0).contains(&id) && id.fract() == 0.0;
        (self.name == TRACK_ID && whole).then_some(id as u32)
    }
}

/// The records of a markers chunk: 16 bytes each, a name, a value, a position and a fourth word.
pub fn markers(payload: &[u8]) -> Result<Vec<AwcMarker>> {
    let (records, rest) = payload.as_chunks::<MARKER_RECORD>();
    if !rest.is_empty() {
        return Err(IoError::Invalid {
            what: "a markers chunk".into(),
            why: format!("its {} bytes are not whole 16-byte records", payload.len()),
        });
    }
    let word = |r: &[u8; MARKER_RECORD], at: usize| {
        u32::from_le_bytes([r[at], r[at + 1], r[at + 2], r[at + 3]])
    };
    Ok(records
        .iter()
        .map(|r| AwcMarker {
            name: word(r, 0),
            value: word(r, 4),
            position: word(r, 8),
        })
        .collect())
}

/// The radio track ids `markers` state, in the order of their positions: one for a track, one
/// for each track of a mix.
pub fn track_ids(markers: &[AwcMarker]) -> Vec<u32> {
    let mut ids: Vec<(u32, u32)> = (markers.iter())
        .filter_map(|m| Some((m.position, m.track_id()?)))
        .collect();
    ids.sort_by_key(|(position, _)| *position);
    ids.into_iter().map(|(_, id)| id).collect()
}

/// The markers of each stream that has a markers chunk in the plain container at `offset` of
/// `file`, read from the container's tables and those chunks alone.
pub fn read_markers(mut file: impl Read + Seek, offset: u64) -> Result<Vec<(u32, Vec<AwcMarker>)>> {
    let refuse = |why: &str| IoError::Invalid {
        what: "an audio container".into(),
        why: why.into(),
    };
    let word = |b: &[u8], at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
    let mut head = [0u8; HEADER_BYTES];
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(&mut head)?;
    if word(&head, 0) != AWC_MAGIC_LE {
        return Err(refuse("it does not start as a plain container does"));
    }
    let flags = u16::from_le_bytes([head[6], head[7]]);
    let streams = word(&head, 8) as usize;
    if streams > MAX_STREAMS {
        return Err(refuse("it states more streams than are read"));
    }
    let mut heads = vec![0u8; 4 * streams];
    file.seek(SeekFrom::Start(
        offset + stream_table_offset(flags, streams) as u64,
    ))?;
    file.read_exact(&mut heads)?;
    let heads: Vec<(u32, usize)> = (heads.as_chunks::<4>().0.iter())
        .map(|w| u32::from_le_bytes(*w))
        .map(|w| (w & 0x1FFF_FFFF, (w >> 29) as usize))
        .collect();
    // A stream's chunk count is three bits, so the chunk table is bounded by the stream count.
    let mut table = vec![0u8; 8 * heads.iter().map(|h| h.1).sum::<usize>()];
    file.read_exact(&mut table)?;
    let mut chunks = (table.as_chunks::<8>().0.iter()).map(|w| u64::from_le_bytes(*w));
    let mut out = Vec::new();
    for (id, count) in heads {
        for raw in chunks.by_ref().take(count) {
            if AwcChunkKind::from_byte((raw >> 56) as u8) != AwcChunkKind::Markers {
                continue;
            }
            let size = ((raw >> 28) & 0x0FFF_FFFF) as usize;
            if size > MAX_MARKERS {
                return Err(refuse("a markers chunk of it is longer than are read"));
            }
            let mut payload = vec![0u8; size];
            file.seek(SeekFrom::Start(offset + (raw & 0x0FFF_FFFF)))?;
            file.read_exact(&mut payload)?;
            out.push((id, markers(&payload)?));
        }
    }
    Ok(out)
}

/// One record of a stream's [`AwcChunkKind::Loudness`] chunk: a sample position and a level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AwcLoudness {
    /// As stated: 4,800 times the record's index in nearly every stream, not in all.
    pub position: u32,
    pub value: f32,
}

impl AwcStream {
    /// The records of the stream's markers chunk; `None` when it has none.
    pub fn markers(&self) -> Option<Result<Vec<AwcMarker>>> {
        Some(markers(&self.chunk(AwcChunkKind::Markers)?.payload))
    }

    /// The records of the stream's loudness chunk; `None` when it has none.
    pub fn loudness(&self) -> Option<Result<Vec<AwcLoudness>>> {
        let payload = &self.chunk(AwcChunkKind::Loudness)?.payload;
        if payload.len() % LOUDNESS_RECORD != 0 {
            return Some(Err(IoError::Invalid {
                what: "a loudness chunk".into(),
                why: format!("its {} bytes are not whole 8-byte records", payload.len()),
            }));
        }
        let records = payload
            .as_chunks::<LOUDNESS_RECORD>()
            .0
            .iter()
            .map(|r| AwcLoudness {
                position: u32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                value: f32::from_le_bytes([r[4], r[5], r[6], r[7]]),
            });
        Some(Ok(records.collect()))
    }

    /// The RSC8 resource the stream's animation chunk stores, which fills the chunk exactly; `None`
    /// when it has none.
    pub fn animation(&self) -> Option<Result<Rsc8>> {
        let payload = &self.chunk(AwcChunkKind::Animation)?.payload;
        Some(Rsc8::open(payload).and_then(|rsc| {
            let len = Rsc8::HEADER_LEN + rsc.virtual_size() + rsc.physical_size();
            if len != payload.len() {
                return Err(IoError::Invalid {
                    what: "an animation chunk".into(),
                    why: format!(
                        "its RSC8 states {len} bytes and the chunk holds {}",
                        payload.len()
                    ),
                });
            }
            Ok(rsc)
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::AwcChunk;
    use super::*;

    fn stream(kind: AwcChunkKind, payload: Vec<u8>) -> AwcStream {
        AwcStream {
            id: 1,
            chunks: vec![AwcChunk {
                kind,
                offset: 0,
                payload,
            }],
        }
    }

    #[test]
    fn a_loudness_chunk_reads_as_its_records_and_a_partial_record_is_refused() {
        let records = [(0u32, -20.5f32), (4800, -18.0)];
        let payload: Vec<u8> = records
            .iter()
            .flat_map(|(p, v)| p.to_le_bytes().into_iter().chain(v.to_le_bytes()))
            .collect();
        let read = stream(AwcChunkKind::Loudness, payload.clone())
            .loudness()
            .expect("a chunk")
            .expect("whole records");
        let read: Vec<_> = read.iter().map(|r| (r.position, r.value)).collect();
        assert_eq!(read, records);
        let cut = stream(AwcChunkKind::Loudness, payload[..12].to_vec());
        assert!(cut.loudness().expect("a chunk").is_err());
        assert!(stream(AwcChunkKind::Data, payload).loudness().is_none());
    }

    fn marker(name: &[u8], value: u32, position: u32) -> Vec<u8> {
        [crate::hash::string_hash(name), value, position, 0]
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect()
    }

    #[test]
    fn markers_read_as_their_records_and_state_track_ids_by_position() {
        let payload = [
            marker(b"trackid", 77.0f32.to_bits(), 9_000),
            marker(b"another", 4.0f32.to_bits(), 100),
            marker(b"trackid", 12.0f32.to_bits(), 0),
            marker(b"trackid", 12.5f32.to_bits(), 50),
        ]
        .concat();
        let read = stream(AwcChunkKind::Markers, payload.clone())
            .markers()
            .expect("a chunk")
            .expect("whole records");
        assert_eq!(read.len(), 4);
        assert_eq!((read[1].position, read[1].track_id()), (100, None));
        assert_eq!(track_ids(&read), [12, 77], "a fraction is no id");
        assert!(markers(&payload[..20]).is_err());
        assert!(stream(AwcChunkKind::Data, payload).markers().is_none());
    }

    #[test]
    fn the_markers_of_a_container_are_read_from_its_tables_alone() {
        // A header, two stream words, three chunk words, then the payloads, after 32 bytes
        // that are not the container's.
        let (first, second) = (
            marker(b"trackid", 5.0f32.to_bits(), 0),
            marker(b"trackid", 6.0f32.to_bits(), 0),
        );
        let chunk = |kind: AwcChunkKind, offset: u64, size: u64| {
            (u64::from(kind.as_byte()) << 56) | (size << 28) | offset
        };
        let mut file = vec![0xEEu8; 32];
        for word in [AWC_MAGIC_LE, 1, 2, 48] {
            file.extend(word.to_le_bytes());
        }
        for word in [(1 << 29) | 0x111u32, (2 << 29) | 0x222] {
            file.extend(word.to_le_bytes());
        }
        for word in [
            chunk(AwcChunkKind::Markers, 48, 16),
            chunk(AwcChunkKind::Data, 64, 4),
            chunk(AwcChunkKind::Markers, 68, 16),
        ] {
            file.extend(word.to_le_bytes());
        }
        file.extend(&first);
        file.extend([0; 4]);
        file.extend(&second);
        let read = read_markers(std::io::Cursor::new(&file), 32).expect("reads");
        let ids: Vec<(u32, Vec<u32>)> = (read.iter())
            .map(|(id, markers)| (*id, track_ids(markers)))
            .collect();
        assert_eq!(ids, [(0x111, vec![5]), (0x222, vec![6])]);
        assert!(read_markers(std::io::Cursor::new(&file), 0).is_err());
    }

    #[test]
    fn an_animation_chunk_opens_as_the_rsc8_that_fills_it() {
        const VIRTUAL: u32 = 0x40;
        let mut payload = Vec::new();
        for word in [
            u32::from_le_bytes(*b"RSC8"),
            0x1F << 8 | 0x41,
            VIRTUAL | 4,
            1,
        ] {
            payload.extend(word.to_le_bytes());
        }
        payload.resize(Rsc8::HEADER_LEN + VIRTUAL as usize, 0);
        let rsc = stream(AwcChunkKind::Animation, payload.clone())
            .animation()
            .expect("a chunk")
            .expect("opens");
        assert_eq!(
            (rsc.resource_id(), rsc.virtual_size()),
            (0x41, VIRTUAL as usize)
        );
        payload.extend([0; 16]);
        let padded = stream(AwcChunkKind::Animation, payload);
        assert!(padded.animation().expect("a chunk").is_err());
    }
}
