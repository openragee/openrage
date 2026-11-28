//! GTA VI's skin tones (a PSO body): the tones a ped's skin is authored against, and the presets
//! a ped picks one of by name, each naming its tone; typed from the value tree.

use super::hash::name_hash32;
use crate::meta::{MetaInstance, Value};

/// The member of the body's root that holds both lists.
pub const SKIN_TONES: u32 = 0x0734_28FF;
const TONES: u32 = 0xFB03_3FB3;
const PRESETS: u32 = name_hash32(b"presets");
const NAME: u32 = name_hash32(b"name");
const TONE_COLOUR: u32 = 0xE7F8_3DE9;
const TONE_PLACE: u32 = 0xD3B0_DC1F;
const TONE_DEPTH_WEIGHT: u32 = 0x7E92_F820;
const TONE_EXTRA: [u32; 3] = [0x12A4_1D6A, 0xED7A_5C54, 0xFFB7_BB60];
const PRESET_COLOUR: u32 = 0xF644_6016;
const PRESET_TONE: u32 = 0xEC99_182D;
const PRESET_OFFSET: u32 = 0xD293_61D0;
/// The three members of a place or an offset.
const AXES: [u32; 3] = [0x56E1_8AA8, 0xCA26_45CC, 0xD0F0_32B0];

/// A skin tone, of the fields the game builds a ped's skin record from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkinTone {
    pub name: u32,
    /// The colour of the skin the albedo is authored for.
    pub colour: [f32; 3],
    /// Where in the skin look-up table the tone is found: across, down and in depth.
    pub place: [f32; 3],
    /// The weight of the per-pixel term of the look-up's depth a ped wearing it is given.
    pub depth_weight: f32,
    /// Three more numbers its skin record carries.
    pub extra: [f32; 3],
}

/// A preset a ped picks by name: a colour, the tone it names, and an offset to the tone's place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkinPreset {
    pub name: u32,
    pub colour: [f32; 3],
    pub tone: u32,
    pub offset: [f32; 3],
}

/// The tones and the presets of a skin tones body, in the order it lists them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SkinTones {
    pub tones: Vec<SkinTone>,
    pub presets: Vec<SkinPreset>,
}

/// Whether `body` may be a skin tones body: it holds the hashes of its tones' list and of a
/// preset's tone.
pub fn is_skin_tones(body: &[u8]) -> bool {
    let holds =
        |hash: u32| (body.windows(4)).any(|w| w == hash.to_le_bytes() || w == hash.to_be_bytes());
    holds(TONES) && holds(PRESET_TONE)
}

fn is_member(name: &str, hash: u32) -> bool {
    name == format!("hash_{hash:08X}") || name_hash32(name.as_bytes()) == hash
}

fn member(of: &MetaInstance, hash: u32) -> Option<&Value> {
    (of.fields.iter()).find_map(|(name, value)| is_member(name, hash).then_some(value))
}

fn structure(value: &Value) -> Option<&MetaInstance> {
    match value {
        Value::Struct(s) | Value::Pointer(Some(s)) => Some(s),
        _ => None,
    }
}

fn hash(of: &MetaInstance, hash: u32) -> u32 {
    match member(of, hash) {
        Some(Value::Hash(h)) => h.hash,
        Some(Value::UInt(word)) => *word as u32,
        _ => 0,
    }
}

fn float(of: &MetaInstance, hash: u32) -> f32 {
    match member(of, hash) {
        Some(Value::Float(value)) => *value as f32,
        _ => 0.0,
    }
}

fn vector(of: &MetaInstance, hash: u32) -> [f32; 3] {
    match member(of, hash) {
        Some(Value::Vec3(v)) => *v,
        Some(Value::Vec4([x, y, z, _])) => [*x, *y, *z],
        _ => [0.0; 3],
    }
}

fn axes(of: &MetaInstance, hash: u32) -> [f32; 3] {
    let Some(of) = member(of, hash).and_then(structure) else {
        return [0.0; 3];
    };
    AXES.map(|axis| float(of, axis))
}

fn items(of: &MetaInstance, hash: u32) -> impl Iterator<Item = &MetaInstance> {
    let items = match member(of, hash) {
        Some(Value::Array(items)) => items.as_slice(),
        _ => &[],
    };
    items.iter().filter_map(structure)
}

/// The skin tones the value tree `root` of a body states; `None` where it holds no such list.
pub fn read_tree(root: &MetaInstance) -> Option<SkinTones> {
    let lists = member(root, SKIN_TONES).and_then(structure)?;
    member(lists, TONES)?;
    let tones = (items(lists, TONES))
        .map(|tone| SkinTone {
            name: hash(tone, NAME),
            colour: vector(tone, TONE_COLOUR),
            place: axes(tone, TONE_PLACE),
            depth_weight: float(tone, TONE_DEPTH_WEIGHT),
            extra: TONE_EXTRA.map(|field| float(tone, field)),
        })
        .collect();
    let presets = (items(lists, PRESETS))
        .map(|preset| SkinPreset {
            name: hash(preset, NAME),
            colour: vector(preset, PRESET_COLOUR),
            tone: hash(preset, PRESET_TONE),
            offset: axes(preset, PRESET_OFFSET),
        })
        .collect();
    Some(SkinTones { tones, presets })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::instance::HashedString;

    fn field(hash: u32, value: Value) -> (String, Value) {
        (format!("hash_{hash:08X}"), value)
    }

    fn instance(fields: Vec<(String, Value)>) -> Value {
        let mut made = MetaInstance::new("made-up");
        made.fields = fields;
        Value::Struct(Box::new(made))
    }

    fn axes_of(values: [f64; 3]) -> Value {
        instance(
            (AXES.iter().zip(values))
                .map(|(axis, value)| field(*axis, Value::Float(value)))
                .collect(),
        )
    }

    #[test]
    fn a_body_s_tones_and_presets_are_read_in_order_with_each_named_field() {
        let tone = instance(vec![
            ("name".to_owned(), Value::Hash(HashedString::from_hash(7))),
            field(TONE_COLOUR, Value::Vec3([0.5, 0.25, 0.125])),
            field(TONE_PLACE, axes_of([0.1, 0.0, 0.2])),
            field(TONE_DEPTH_WEIGHT, Value::Float(0.5)),
            field(TONE_EXTRA[1], Value::Float(0.75)),
        ]);
        let preset = instance(vec![
            ("name".to_owned(), Value::Hash(HashedString::from_hash(9))),
            field(PRESET_COLOUR, Value::Vec3([0.3, 0.2, 0.1])),
            field(PRESET_TONE, Value::Hash(HashedString::from_hash(7))),
            field(PRESET_OFFSET, axes_of([0.0, -0.5, 0.25])),
        ]);
        let lists = instance(vec![
            field(TONES, Value::Array(vec![tone])),
            ("presets".to_owned(), Value::Array(vec![preset])),
        ]);
        let mut root = MetaInstance::new("root");
        root.fields = vec![field(SKIN_TONES, lists)];
        let read = read_tree(&root).expect("skin tones");
        assert_eq!(
            read.tones,
            [SkinTone {
                name: 7,
                colour: [0.5, 0.25, 0.125],
                place: [0.1, 0.0, 0.2],
                depth_weight: 0.5,
                extra: [0.0, 0.75, 0.0],
            }]
        );
        assert_eq!(
            read.presets,
            [SkinPreset {
                name: 9,
                colour: [0.3, 0.2, 0.1],
                tone: 7,
                offset: [0.0, -0.5, 0.25],
            }]
        );
        // A body with no such member is not skin tones.
        assert_eq!(read_tree(&MetaInstance::new("other")), None);
    }

    #[test]
    fn a_skin_tones_body_is_told_by_the_hashes_it_holds() {
        let mut body = TONES.to_le_bytes().to_vec();
        assert!(!is_skin_tones(&body));
        body.extend(PRESET_TONE.to_le_bytes());
        assert!(is_skin_tones(&body));
    }
}
