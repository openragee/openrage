//! The table from codec to decoder: one row per codec, and a codec with no row is refused by name.

use std::sync::Arc;

use or_io::awc::codec as awc_codec;
use or_io::awc::{self, Accumulator};

use crate::{
    bik, opus, spectra, xma, AudioInfo, AudioStream, Codec, MediaError, Pcm, Samples, VideoInfo,
    VideoStream,
};

/// Opens a sound track from the bytes its container stored it in.
pub type OpenAudio =
    Arc<dyn Fn(&AudioInfo, &[u8]) -> Result<Box<dyn AudioStream>, MediaError> + Send + Sync>;

/// Opens a picture track from the bytes of the whole movie.
pub type OpenVideo =
    Arc<dyn Fn(&VideoInfo, Arc<[u8]>) -> Result<Box<dyn VideoStream>, MediaError> + Send + Sync>;

/// What a row of the table does for its codec.
#[derive(Clone)]
pub enum Decoder {
    Audio(OpenAudio),
    Video(OpenVideo),
}

impl Decoder {
    pub fn audio(
        open: impl Fn(&AudioInfo, &[u8]) -> Result<Box<dyn AudioStream>, MediaError>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        Decoder::Audio(Arc::new(open))
    }

    pub fn video(
        open: impl Fn(&VideoInfo, Arc<[u8]>) -> Result<Box<dyn VideoStream>, MediaError>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        Decoder::Video(Arc::new(open))
    }
}

/// What the ADPCM decode does with a predictor that runs past `i16`: the clamp the peaks stored in
/// Rockstar's files decide for, as `or-io`'s `Accumulator` documents.
pub(crate) const ACCUMULATOR: Accumulator = Accumulator::Clamped;

/// A row for a codec whose stored bytes `or-io` decodes whole.
fn awc_row(codec: awc::Codec, accumulator: Accumulator) -> Decoder {
    Decoder::audio(move |info, data| {
        let samples = usize::try_from(info.frames)
            .ok()
            .and_then(|frames| frames.checked_mul(usize::from(info.channels)))
            .ok_or_else(|| MediaError::Decode("the stream states more samples than fit".into()))?;
        let pcm = awc_codec::decode(data, samples, codec, accumulator).ok_or_else(|| {
            MediaError::Decode(format!(
                "this stream is {} and its data does not decode to the samples its format states",
                info.codec.label()
            ))
        })?;
        let samples = pcm
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        Ok(Box::new(Pcm::new(Samples::I16(samples), info.channels)))
    })
}

/// The table, and the rows an embedder adds to it.
#[derive(Clone)]
pub struct Decoders {
    rows: Vec<(Codec, Decoder)>,
    pub(crate) accumulator: Accumulator,
}

impl Default for Decoders {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoders {
    /// The codecs OpenRage decodes in its own code. A codec added later is one row here.
    pub fn new() -> Self {
        Self::with_accumulator(ACCUMULATOR)
    }

    /// The same table with ADPCM decoded by `accumulator` rather than the clamp the peaks decide for.
    pub fn with_accumulator(accumulator: Accumulator) -> Self {
        Self {
            rows: vec![
                (Codec::Pcm16, awc_row(awc::Codec::Pcm16, accumulator)),
                (Codec::RageAdpcm, awc_row(awc::Codec::ImaAdpcm, accumulator)),
                (Codec::Mp3, awc_row(awc::Codec::Mp3, accumulator)),
                (Codec::Xma, Decoder::audio(xma::open)),
                (Codec::Opus, Decoder::audio(opus::open)),
                (Codec::Spectra, Decoder::audio(spectra::open)),
                (Codec::Bink1, Decoder::video(bik::open_video)),
                (Codec::Bink2, Decoder::video(bik::open_video)),
                (Codec::BinkAudioDct, Decoder::audio(bik::open_audio)),
                (Codec::BinkAudioRdft, Decoder::audio(bik::open_audio)),
                (Codec::Bink2Audio, Decoder::audio(bik::open_audio)),
            ],
            accumulator,
        }
    }

    /// A table with no row, for an embedder that decodes with rows of its own alone.
    pub fn empty() -> Self {
        Self {
            rows: Vec::new(),
            accumulator: ACCUMULATOR,
        }
    }

    /// One more row, for a codec the table does not decode yet; the first row for a codec decodes it.
    pub fn with(mut self, codec: Codec, decoder: Decoder) -> Self {
        self.rows.push((codec, decoder));
        self
    }

    pub fn audio_decoder(&self, codec: Codec) -> Result<&OpenAudio, MediaError> {
        self.rows
            .iter()
            .find_map(|(row, decoder)| match decoder {
                Decoder::Audio(open) if *row == codec => Some(open),
                _ => None,
            })
            .ok_or(MediaError::NoDecoder(codec))
    }

    pub fn video_decoder(&self, codec: Codec) -> Result<&OpenVideo, MediaError> {
        self.rows
            .iter()
            .find_map(|(row, decoder)| match decoder {
                Decoder::Video(open) if *row == codec => Some(open),
                _ => None,
            })
            .ok_or(MediaError::NoDecoder(codec))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_decodes_what_it_has_rows_for_and_names_what_it_does_not() {
        let table = Decoders::new();
        for codec in [
            Codec::Pcm16,
            Codec::RageAdpcm,
            Codec::Mp3,
            Codec::Xma,
            Codec::Opus,
            Codec::Spectra,
            Codec::BinkAudioDct,
            Codec::BinkAudioRdft,
            Codec::Bink2Audio,
        ] {
            assert!(table.audio_decoder(codec).is_ok(), "{codec:?}");
        }
        assert!(table.video_decoder(Codec::Bink1).is_ok());
        assert!(table.video_decoder(Codec::Bink2).is_ok());
        assert!(
            table.audio_decoder(Codec::Bink1).is_err(),
            "a picture codec is not a sound one"
        );
        let unknown = Codec::Unknown(3);
        assert_eq!(
            table.audio_decoder(unknown).err(),
            Some(MediaError::NoDecoder(unknown))
        );
        assert_eq!(
            table.video_decoder(unknown).err(),
            Some(MediaError::NoDecoder(unknown))
        );
        assert_eq!(
            table.audio_decoder(Codec::Bink2).err(),
            Some(MediaError::NoDecoder(Codec::Bink2)),
            "a picture codec is not a sound one"
        );
        assert!(
            table.video_decoder(Codec::Mp3).is_err(),
            "a sound codec is not a picture one"
        );
    }

    #[test]
    fn a_row_decodes_the_stored_bytes_to_the_samples_the_info_states() {
        let table = Decoders::new();
        let info = AudioInfo {
            codec: Codec::Pcm16,
            channels: 2,
            sample_rate: 8000,
            frames: 3,
        };
        let open = table.audio_decoder(Codec::Pcm16).expect("a row");
        // Three frames of two channels, then a payload that ends early: zero-filled, not shorter.
        let mut audio = open(&info, &[1, 0, 2, 0, 3, 0]).expect("it decodes");
        assert_eq!(
            audio.read_all_i16().expect("samples"),
            vec![1, 2, 3, 0, 0, 0]
        );
    }
}
