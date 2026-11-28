//! The bodies of an archive that stores them as they are, each from a 16-byte slot: one is named
//! by the bytes it starts with, and its end is stated only where those bytes state it.

use std::io::{self, Read, Seek, SeekFrom};

use super::{
    block_map_at, classify, Found, PsoWalk, Reader, Rpf9BodyKind, Rpf9Content, Rpf9Header,
    Rpf9ScanError, Rpf9ScanFind, Rpf9Signature, Rpf9ZlibRun, ScanStrategy, BOM, PIECE,
    RPF9_HEADER_SIZE, RPF9_SLOT, TEXT_PROBE,
};

/// A section stored as it is states no length, so every pointer of its root counts as inside it.
const ANY_LENGTH: u64 = 1 << 28;
/// How far into a document its XML declaration closes.
const DECLARATION: usize = 128;

/// The strategy the header at the start of `source` states: the zlib one where none reads.
pub(super) fn strategy_of<R: Read + Seek>(source: &mut R) -> io::Result<ScanStrategy> {
    let mut head = [0u8; RPF9_HEADER_SIZE];
    source.seek(SeekFrom::Start(0))?;
    match source.read_exact(&mut head) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(ScanStrategy::ZlibHeads),
        Err(e) => return Err(e),
    }
    let facts = Rpf9Header::read(&head).ok().and_then(|h| h.facts().ok());
    Ok(facts.map_or(ScanStrategy::ZlibHeads, |facts| facts.scan))
}

/// What follows `<?xml` where `head` starts with it, behind a byte-order mark or not.
fn xml_declaration(head: &[u8]) -> Option<&[u8]> {
    head.strip_prefix(&BOM)
        .unwrap_or(head)
        .strip_prefix(b"<?xml")
}

impl<R: Read + Seek> Reader<R> {
    /// Whether a body stored as it is may start at the slot `at`: never in a file of zlib bodies.
    pub(super) fn may_start_stored(&mut self, at: u64) -> Result<bool, Rpf9ScanError> {
        if self.strategy != ScanStrategy::StoredBodies {
            return Ok(false);
        }
        let head = self.get(at, RPF9_SLOT as usize)?;
        Ok(head.starts_with(b"PSIN")
            || head.starts_with(b"RBF0")
            || xml_declaration(head).is_some()
            || block_map_at(head).is_some())
    }

    /// The body stored as it is at `at`; with `out`, its bytes, which only a stated end gives.
    pub(super) fn stored_body(
        &mut self,
        at: u64,
        out: Option<&mut Vec<u8>>,
        limit: u64,
    ) -> Result<Found, Rpf9ScanError> {
        let Some(find) = self.stored_find(at)? else {
            return Ok(Found::Nothing);
        };
        if let Some(out) = out {
            let len = find.stored_len.ok_or(Rpf9ScanError::ResourceRefused {
                offset: at,
                at,
                why: "it is stored as it is, and its bytes do not state where it ends",
            })?;
            if len > limit {
                return Err(Rpf9ScanError::InflateLimit { offset: at, limit });
            }
            self.copy(at, len, out)?;
        }
        Ok(Found::body(find))
    }

    /// The PSO, RBF or XML document at `at`, or the headerless resource behind its zero slot.
    fn stored_find(&mut self, at: u64) -> Result<Option<Rpf9ScanFind>, Rpf9ScanError> {
        let head = self.get(at, BOM.len() + TEXT_PROBE)?.to_vec();
        let left = self.left(at)?;
        let find = |content, signature, len: Option<u64>| Rpf9ScanFind {
            offset: at,
            kind: Rpf9BodyKind::Zlib(Rpf9ZlibRun {
                content,
                chunks: 0,
                stored_pieces: len.map_or(0, |len| len.div_ceil(PIECE) as u32),
                first_chunk_len: 0,
                first_chunk_inflated: 0,
                inflated: len.unwrap_or(0),
                signature,
                resource: None,
            }),
            // A body that states no end claims its first slot alone.
            span: len
                .map_or(RPF9_SLOT, |len| len.next_multiple_of(RPF9_SLOT))
                .min(left),
            stored_len: len,
            inflated_len: len,
            container: None,
        };
        if head.starts_with(b"PSIN") {
            let len = self.pso_len(at)?.filter(|len| *len <= left);
            return Ok(len.map(|len| find(Rpf9Content::Pso, None, Some(len))));
        }
        if head.starts_with(b"RBF0") {
            // Measured: a wide document states FB FF after the magic, any other the 00 00 that
            // opens its first record.
            let opens = matches!(head.get(4..6), Some([0xFB, 0xFF] | [0, 0]));
            return Ok(opens.then(|| find(Rpf9Content::Rbf, None, None)));
        }
        if let Some(declared) = xml_declaration(&head) {
            let close = (declared.windows(2).take(DECLARATION)).position(|pair| pair == b"?>");
            let closes = close.is_some_and(|end| {
                (declared[..end].iter()).all(|c| c.is_ascii_graphic() || *c == b' ')
            });
            return Ok(closes.then(|| find(Rpf9Content::Xml, None, None)));
        }
        // Measured: a resource's section follows a zero slot, where a stored RSC8 has its header,
        // and its map, on a slot of it, states its two counts and nothing else.
        let Some(map) = block_map_at(&head).filter(|map| map.is_multiple_of(RPF9_SLOT)) else {
            return Ok(None);
        };
        let Some(before) = at.checked_sub(RPF9_SLOT) else {
            return Ok(None);
        };
        if self
            .get(before, RPF9_SLOT as usize)?
            .iter()
            .any(|b| *b != 0)
        {
            return Ok(None);
        }
        let counts = self.get(at + map, RPF9_SLOT as usize)?;
        let mapped = counts.len() == RPF9_SLOT as usize
            && (1..=2).contains(&counts[8])
            && counts[9] <= 2
            && counts[..8].iter().chain(&counts[10..]).all(|b| *b == 0);
        if !mapped {
            return Ok(None);
        }
        let content = classify(&head, head.len() as u64);
        let unclassed =
            content == Rpf9Content::ResourceShaped && head.get(0x10..0x14) != Some(b"0DRP");
        let signature = unclassed
            .then(|| Rpf9Signature::of(&head, ANY_LENGTH))
            .flatten();
        Ok(Some(find(content, signature, None)))
    }

    /// The length of the PSO document at `at`, which its section frames state up to `CHKS`.
    fn pso_len(&mut self, at: u64) -> Result<Option<u64>, Rpf9ScanError> {
        let mut walk = PsoWalk::default();
        while !walk.last {
            let frame = self.get(at + walk.next_frame, 8)?;
            walk.at = walk.next_frame;
            if frame.len() < 8 {
                return Ok(None);
            }
            walk.feed(frame);
            if walk.bad {
                return Ok(None);
            }
        }
        Ok(Some(walk.next_frame))
    }
}
