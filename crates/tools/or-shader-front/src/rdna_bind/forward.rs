//! A forward pass of a GTA VI model: a pixel program that colours or lights a surface itself and
//! is blended over the lit frame, and what the game leaves in the bindings only it fills while
//! the systems that write them are off.

use super::shading::{Feed, Unfed};

/// The colour targets a forward pixel program exports, by what they hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forward {
    /// One colour, already multiplied by its alpha, laid over what is behind it.
    Emissive,
    /// A light the surface adds and a colour what is behind it is multiplied by, in two targets.
    Lit,
}

/// A factor of a blend, as an effect's blend record numbers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Factor {
    Zero,
    One,
    OneMinusSourceAlpha,
}

/// How a forward pass's colour is laid over the frame.
// The colour's source and destination factors, the alpha's, each summed, and the channels
// written, a bit each from red.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Blend {
    pub colour: [Factor; 2],
    pub alpha: [Factor; 2],
    pub writes: u8,
}

/// A target written whole, and the first export target that is no colour target.
const WHOLE: u8 = 0xF;
const COLOUR_TARGETS: u8 = 8;

/// The forward pass `exports` are, where they are one: a colour target written whole, or two.
pub fn forward_of(exports: &[(u8, u8)]) -> Option<Forward> {
    let mut colour: Vec<(u8, u8)> = (exports.iter().copied())
        .filter(|(target, _)| *target < COLOUR_TARGETS)
        .collect();
    colour.sort_unstable();
    colour.dedup();
    match colour.as_slice() {
        [(0, WHOLE)] => Some(Forward::Emissive),
        [(0, WHOLE), (1, WHOLE)] => Some(Forward::Lit),
        _ => None,
    }
}

impl Forward {
    /// The blend its pass draws with, where an effect's own state states it.
    // The one colour target is laid on by the blend record `standard_emissive`'s pass names:
    // ONE and ONE less the source's alpha on the colour, ZERO and ONE on the alpha, red, green and
    // blue written. A lit pass names no blend record: it draws with the engine's own state.
    pub fn blend(self) -> Result<Blend, Unfed> {
        match self {
            Self::Emissive => Ok(Blend {
                colour: [Factor::One, Factor::OneMinusSourceAlpha],
                alpha: [Factor::Zero, Factor::One],
                writes: 0b0111,
            }),
            Self::Lit => Err(Unfed::Blend),
        }
    }
}

/// The draw's fade record: four weights of a two by two stipple from byte 16, and a pixel whose
/// weight is under one is discarded; the rest at nought switches the alpha system's depth test
/// and the water's clip off.
const FADE: u32 = 0xE688_861E;
/// The frame's scale, 16 bytes: what multiplies the colour a pass exports (+4), what it samples
/// of light the frame's size (+0) and the probe's light (+8). One where nothing pre-exposes.
const FRAME_SCALE: u32 = 0xDAAF_2AC8;
/// `g_TransmittanceVolume`, the fog's transmittance, which multiplies the colour.
const TRANSMITTANCE: u32 = 0xBFBD_3BEC;
/// The screen layer's transmittance, which stands in for the fog's where that layer is entered.
const LAYER_TRANSMITTANCE: u32 = 0x8395_00C2;
/// The pools of an entity's records a forward program multiplies its colour by: a light's
/// intensity, colour and two maps (24 bytes a record), and a scale in the high half of +12 (32).
const LIGHT_RECORDS: u32 = 0x7543_300E;
const ENTITY_RECORDS: u32 = 0x598E_AD6B;

/// What the game leaves in a binding only it fills, while the system that writes it is off.
#[derive(Debug, Clone, PartialEq)]
enum Rest {
    /// Bytes, the rest noughts.
    Bytes(Vec<u8>),
    /// The game's one white texel.
    White,
    /// Noughts would change what the program draws, and what the game leaves there is not read.
    Unread,
}

/// Noughts of `len` bytes with `floats` from byte `at`.
fn floats_at(len: usize, at: usize, floats: &[f32]) -> Vec<u8> {
    let mut bytes = vec![0; len];
    for (i, float) in floats.iter().enumerate() {
        bytes[at + 4 * i..at + 4 * i + 4].copy_from_slice(&float.to_le_bytes());
    }
    bytes
}

/// What the game leaves in the binding named `hash` where noughts are not it.
fn rest_of(hash: u32) -> Option<Rest> {
    match hash {
        FADE => Some(Rest::Bytes(floats_at(128, 16, &[1.0; 4]))),
        FRAME_SCALE => Some(Rest::Bytes(floats_at(16, 0, &[1.0; 3]))),
        TRANSMITTANCE | LAYER_TRANSMITTANCE => Some(Rest::White),
        LIGHT_RECORDS | ENTITY_RECORDS => Some(Rest::Unread),
        _ => None,
    }
}

/// `feed`, one binding of a forward stage, as the game leaves it at rest: a binding only the game
/// fills whose noughts are its rest stays noughts. `Err` names one whose rest is not read.
pub(super) fn at_rest(feed: Feed) -> Result<Feed, Unfed> {
    let Feed::Zero(Some(hash)) = feed else {
        return Ok(feed);
    };
    match rest_of(hash) {
        Some(Rest::Bytes(bytes)) => Ok(Feed::Stated(bytes)),
        Some(Rest::White) => Ok(Feed::White),
        Some(Rest::Unread) => Err(Unfed::Unread(hash)),
        None => Ok(feed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forward_pass_is_told_by_its_colour_targets_and_a_deferred_one_is_none() {
        // The emissive program's: target 0 whole, and the export of no target.
        assert_eq!(forward_of(&[(0, 15), (9, 0)]), Some(Forward::Emissive));
        // The lit alpha program's, the null export first and a target exported twice.
        let lit = [(9, 0), (0, 15), (1, 15), (0, 15)];
        assert_eq!(forward_of(&lit), Some(Forward::Lit));
        let deferred = [(0, 15), (1, 15), (2, 15), (3, 15), (4, 3)];
        assert_eq!(forward_of(&deferred), None);
        assert_eq!(forward_of(&[(0, 7)]), None, "a target not written whole");
        assert_eq!(forward_of(&[(9, 0)]), None, "no colour");
    }

    #[test]
    fn only_a_blend_an_effect_states_is_drawn_with() {
        let emissive = Forward::Emissive.blend().expect("stated");
        assert_eq!(emissive.colour, [Factor::One, Factor::OneMinusSourceAlpha]);
        assert_eq!(emissive.alpha, [Factor::Zero, Factor::One]);
        assert_eq!(
            emissive.writes, 0b0111,
            "red, green and blue; the alpha kept"
        );
        assert_eq!(Forward::Lit.blend(), Err(Unfed::Blend));
    }

    #[test]
    fn a_forward_stage_is_given_the_game_s_rest_and_refused_where_that_is_not_read() {
        let float = |bytes: &[u8], at: usize| {
            f32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
        };
        // Every pixel is discarded under a stipple weight of nought: the rest is four ones.
        let Ok(Feed::Stated(fade)) = at_rest(Feed::Zero(Some(FADE))) else {
            panic!("the fade record is not stated");
        };
        assert_eq!(fade.len(), 128);
        assert!((16..32).step_by(4).all(|at| float(&fade, at) == 1.0));
        assert!(fade[..16].iter().chain(&fade[32..]).all(|b| *b == 0));
        let Ok(Feed::Stated(scale)) = at_rest(Feed::Zero(Some(FRAME_SCALE))) else {
            panic!("the frame's scale is not stated");
        };
        assert_eq!(
            [0, 4, 8, 12].map(|at| float(&scale, at)),
            [1.0, 1.0, 1.0, 0.0]
        );
        assert_eq!(at_rest(Feed::Zero(Some(TRANSMITTANCE))), Ok(Feed::White));
        // An entity's records: refused by name, both of them.
        assert_eq!(
            at_rest(Feed::Zero(Some(LIGHT_RECORDS))),
            Err(Unfed::Unread(LIGHT_RECORDS))
        );
        assert_eq!(
            at_rest(Feed::Zero(Some(ENTITY_RECORDS))),
            Err(Unfed::Unread(ENTITY_RECORDS))
        );
        let why = Unfed::Unread(LIGHT_RECORDS).to_string();
        assert!(why.contains("0x7543300E"), "{why}");
        // Noughts that are the rest, and what the editor fills itself, are left as they are.
        assert_eq!(
            at_rest(Feed::Zero(Some(0x4D5A_93EB))),
            Ok(Feed::Zero(Some(0x4D5A_93EB)))
        );
        assert_eq!(at_rest(Feed::Instance), Ok(Feed::Instance));
        assert_eq!(at_rest(Feed::Zero(None)), Ok(Feed::Zero(None)));
    }
}
