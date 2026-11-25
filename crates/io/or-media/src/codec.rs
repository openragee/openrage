//! The codecs OpenRage knows by name, whether or not the table decodes them.

use or_io::awc;

/// How a stream's samples or pictures are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Codec {
    /// Signed 16-bit little-endian samples.
    Pcm16,
    /// IMA ADPCM in the 2,048-byte blocks RAGE writes.
    RageAdpcm,
    /// MPEG-1 Layer III.
    Mp3,
    /// Xbox Media Audio.
    Xma,
    /// Opus, decoded and encoded at 48 kHz in 20 ms frames.
    Opus,
    /// Bink 1 video, every `BIK` revision.
    Bink1,
    /// Bink audio coded with the DCT.
    BinkAudioDct,
    /// Bink audio coded with the real DFT.
    BinkAudioRdft,
    /// Bink audio coded with the DCT as a `KB2n` movie stores it.
    Bink2Audio,
    /// Bink 2 video, the `KB2g` to `KB2k` revisions.
    Bink2,
    /// GTA VI's codec 2: a mono signal stored as overlapping 2,048-point spectra.
    Spectra,
    /// A code a container states that this does not name.
    Unknown(u32),
}

impl Codec {
    /// What a person reads: a column of the `.awc` panel and the name a refusal gives.
    pub fn label(self) -> String {
        match self {
            Codec::Pcm16 => "PCM16".to_string(),
            Codec::RageAdpcm => "ADPCM".to_string(),
            Codec::Mp3 => "MP3".to_string(),
            Codec::Xma => "XMA".to_string(),
            Codec::Opus => "Opus".to_string(),
            Codec::Bink1 => "Bink 1".to_string(),
            Codec::BinkAudioDct => "Bink audio (DCT)".to_string(),
            Codec::BinkAudioRdft => "Bink audio (RDFT)".to_string(),
            Codec::Bink2Audio => "Bink 2 audio".to_string(),
            Codec::Bink2 => "Bink 2".to_string(),
            Codec::Spectra => "Spectra".to_string(),
            Codec::Unknown(code) => format!("unknown({code})"),
        }
    }
}

impl From<awc::Codec> for Codec {
    fn from(codec: awc::Codec) -> Self {
        match codec {
            awc::Codec::Pcm16 => Codec::Pcm16,
            awc::Codec::ImaAdpcm => Codec::RageAdpcm,
            awc::Codec::Mp3 => Codec::Mp3,
            awc::Codec::Xma => Codec::Xma,
            awc::Codec::Opus => Codec::Opus,
            awc::Codec::Unknown(byte) => Codec::Unknown(u32::from(byte)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_awc_codec_maps_to_the_codec_that_names_it() {
        let pairs = [
            (awc::Codec::Pcm16, Codec::Pcm16),
            (awc::Codec::ImaAdpcm, Codec::RageAdpcm),
            (awc::Codec::Mp3, Codec::Mp3),
            (awc::Codec::Xma, Codec::Xma),
            (awc::Codec::Unknown(9), Codec::Unknown(9)),
        ];
        for (container, named) in pairs {
            assert_eq!(Codec::from(container), named);
        }
        // The labels the `.awc` panel shows are the container's own.
        for byte in [0, 4, 5, 7, 9] {
            let container = awc::Codec::from_byte(byte);
            assert_eq!(Codec::from(container).label(), container.label());
        }
    }
}
