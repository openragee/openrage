//! An edit laid back into the extents the source's chunks hold: every chunk keeps its offset and the
//! container keeps its length, so an archive holding it changes only bytes inside the body.

use super::Refused;
use crate::awc::{gaps_between, AwcFile};

/// Why an edit is not laid in the source's extents, so its caller rebuilds the container.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NotInExtent {
    #[error(transparent)]
    Refused(#[from] Refused),
    #[error(
        "the edit does not have the source's streams and chunks in the source's order, so it is \
         not laid in extent"
    )]
    Shape,
    #[error(
        "stream {stream}'s {kind} chunk takes {got} bytes and the source holds {room} where it \
         sits, so the edit is not laid in extent: it changes the container's length"
    )]
    DoesNotFit {
        stream: usize,
        kind: String,
        got: usize,
        room: usize,
    },
}

impl AwcFile {
    /// `edited`, an edit of this container, with every chunk at the offset this one holds it at.
    pub fn in_extent(&self, edited: &AwcFile) -> Result<AwcFile, NotInExtent> {
        // ⚠ A changed chunk's room runs from its offset to the next non-empty chunk, or the end, so
        // it may grow into the alignment after it; the bytes a shorter one frees read zero.
        let shape = |f: &AwcFile| -> Vec<(u32, Vec<_>)> {
            (f.streams.iter())
                .map(|s| (s.id, s.chunks.iter().map(|c| c.kind).collect()))
                .collect()
        };
        if shape(self) != shape(edited) {
            return Err(NotInExtent::Shape);
        }
        let spans: Vec<(usize, usize)> = (self.streams.iter().flat_map(|s| &s.chunks))
            .map(|c| (c.offset as usize, c.payload.len()))
            .collect();
        let room = |me: usize| {
            let at = spans[me].0;
            let mut end = self.plaintext_len;
            for (i, &(o, len)) in spans.iter().enumerate() {
                if i == me || len == 0 {
                    continue;
                }
                if o >= at {
                    end = end.min(o);
                } else if o + len > at {
                    return 0;
                }
            }
            end.saturating_sub(at)
        };

        let mut out = self.clone();
        let mut image = vec![0u8; self.plaintext_len];
        for (at, bytes) in &self.padding {
            let at = *at as usize;
            image[at..at + bytes.len()].copy_from_slice(bytes);
        }
        let mut laid = Vec::with_capacity(spans.len());
        let mut changed = Vec::new();
        let mine = (out.streams.iter_mut().enumerate())
            .flat_map(|(s, stream)| stream.chunks.iter_mut().map(move |c| (s, c)));
        let theirs = edited.streams.iter().flat_map(|s| &s.chunks);
        for (i, ((stream, chunk), new)) in mine.zip(theirs).enumerate() {
            let got = new.payload.len();
            let refused = || NotInExtent::DoesNotFit {
                stream,
                kind: chunk.kind.label(),
                got,
                room: room(i),
            };
            if new.payload != chunk.payload {
                if got > room(i) {
                    return Err(refused());
                }
                changed.push((i, refused()));
            }
            let at = chunk.offset as usize;
            chunk.payload.clone_from(&new.payload);
            image[at..at + got].copy_from_slice(&chunk.payload);
            laid.push((at, got));
        }
        // ⚠ A room is bounded by non-empty chunks only, so two changed chunks, one where the source
        // held an empty one, can be laid over each other; that is refused, never written.
        let over = |a: (usize, usize), b: (usize, usize)| {
            a.1 > 0 && b.1 > 0 && a.0 < b.0 + b.1 && b.0 < a.0 + a.1
        };
        for (i, refused) in changed {
            if (laid.iter().enumerate()).any(|(j, &other)| j != i && over(laid[i], other)) {
                return Err(refused);
            }
        }
        out.padding = gaps_between(&image, self.declared_data_offset() as usize, &mut laid);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::opus::tests::{container, mono, single_stream};
    use super::super::opus_peak_windows;
    use super::*;
    use crate::awc::opus::{OpusStream, OPUS_FRAME};
    use crate::awc::{AwcChunkKind, AwcKey};

    /// Two single Opus streams of 40 and 30 packets in GTA VI's layout, its bytes and its parse.
    fn source() -> (Vec<u8>, AwcFile) {
        let streams = [
            (0x0ABC, single_stream(&mono(40, 1), 1)),
            (0x0DEF, single_stream(&mono(30, 2), 1)),
        ];
        let bytes = container(0xFF21, &[0, 0, 0], &streams);
        let awc = AwcFile::parse(&bytes, AwcKey::default()).expect("the container parses");
        (bytes, awc)
    }

    fn replaced(awc: &AwcFile, new: &OpusStream) -> AwcFile {
        let peaks = vec![7; opus_peak_windows(new.samples)];
        (awc.with_opus_replaced(0, new, &peaks, &[])).expect("replaced")
    }

    fn offsets(awc: &AwcFile) -> Vec<u32> {
        (awc.streams.iter().flat_map(|s| &s.chunks))
            .map(|c| c.offset)
            .collect()
    }

    /// `bytes` over stream 1's chunks, as `awc` places them.
    fn second_stream(bytes: &[u8], awc: &AwcFile) -> Vec<u8> {
        (awc.streams[1].chunks.iter())
            .flat_map(|c| &bytes[c.offset as usize..c.offset as usize + c.payload.len()])
            .copied()
            .collect()
    }

    #[test]
    fn a_same_length_replace_keeps_the_containers_length_and_the_other_streams_bytes() {
        let (bytes, awc) = source();
        let mut packets = mono(40, 1);
        for p in &mut packets {
            p[1..].iter_mut().for_each(|b| *b = b.wrapping_add(3));
        }
        let new = OpusStream {
            id: 0x0ABC,
            channels: 1,
            samples: OPUS_FRAME * 40 - 2400,
            packets,
        };
        let edited = replaced(&awc, &new);
        // The peak chunk grows from 4 bytes to 14, which moves everything after it once rebuilt.
        let rebuilt = edited.to_bytes(AwcKey::default()).expect("written");
        assert_ne!(rebuilt.len(), bytes.len());

        let laid = awc.in_extent(&edited).expect("in extent");
        let out = laid.to_bytes(AwcKey::default()).expect("written");
        assert_eq!(out.len(), bytes.len());
        assert_ne!(out, bytes);
        let read = AwcFile::parse(&out, AwcKey::default()).expect("it reads back");
        assert_eq!(offsets(&read), offsets(&awc));
        assert_eq!(second_stream(&out, &read), second_stream(&bytes, &awc));
        assert_eq!(read.opus_streams(0).expect("split"), vec![new]);
        assert_eq!(read.unaccounted_bytes(), 0);
    }

    #[test]
    fn a_shorter_signal_keeps_every_offset_and_frees_zero_bytes() {
        let (bytes, awc) = source();
        let new = OpusStream {
            id: 0x0ABC,
            channels: 1,
            samples: OPUS_FRAME * 20 - 2000,
            packets: mono(20, 9),
        };
        let edited = replaced(&awc, &new);
        let out = (awc.in_extent(&edited).expect("in extent"))
            .to_bytes(AwcKey::default())
            .expect("written");
        assert_eq!(out.len(), bytes.len());
        let read = AwcFile::parse(&out, AwcKey::default()).expect("it reads back");
        assert_eq!(offsets(&read), offsets(&awc));
        assert_eq!(second_stream(&out, &read), second_stream(&bytes, &awc));
        assert_eq!(read.opus_streams(0).expect("split"), vec![new]);
        let was = awc.streams[0].chunk(AwcChunkKind::Data).expect("data");
        let now = read.streams[0].chunk(AwcChunkKind::Data).expect("data");
        assert!(now.payload.len() < was.payload.len());
        let end = now.offset as usize + now.payload.len();
        let freed = &out[end..was.offset as usize + was.payload.len()];
        assert!(freed.iter().all(|b| *b == 0));
    }

    #[test]
    fn two_changed_chunks_that_would_lie_over_each_other_are_refused() {
        let (_, awc) = source();
        // A of 4 bytes, an empty E 10 bytes on and B 16 bytes on; every other chunk empty.
        let base = awc.streams[0].chunks[0].offset;
        let mut src = awc.clone();
        let chunks = src.streams.iter_mut().flat_map(|s| &mut s.chunks);
        for (i, chunk) in chunks.enumerate() {
            let (at, len) = match i {
                0 => (0, 4),
                1 => (10, 0),
                2 => (16, 4),
                _ => (16, 0),
            };
            chunk.offset = base + at;
            chunk.payload = vec![1; len];
        }
        let mut edited = src.clone();
        let mut chunks = edited.streams.iter_mut().flat_map(|s| &mut s.chunks);
        chunks.next().expect("A").payload = vec![2; 14];
        chunks.next().expect("E").payload = vec![3; 4];
        let why = src.in_extent(&edited).expect_err("A and E overlap");
        assert!(matches!(why, NotInExtent::DoesNotFit { .. }), "{why:?}");
    }

    #[test]
    fn a_longer_signal_is_refused_in_extent_by_name() {
        let (_, awc) = source();
        let new = OpusStream {
            id: 0x0ABC,
            channels: 1,
            samples: OPUS_FRAME * 70 - 2000,
            packets: mono(70, 4),
        };
        let why = awc.in_extent(&replaced(&awc, &new)).expect_err("refused");
        assert!(
            matches!(why, NotInExtent::DoesNotFit { stream: 0, .. }),
            "{why:?}"
        );
        assert!(why.to_string().contains("not laid in extent"), "{why}");

        let streams = [(0x0ABC, single_stream(&mono(40, 1), 1))];
        let other = container(0xFF21, &[0, 0], &streams);
        let other = AwcFile::parse(&other, AwcKey::default()).expect("parses");
        assert_eq!(awc.in_extent(&other), Err(NotInExtent::Shape));
    }
}
