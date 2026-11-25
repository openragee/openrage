//! GTA VI's particle effects list: three dictionaries of effect, emitter and particle rules keyed
//! by the hash of their names, the keyframed values each holds, and the behaviours a particle rule
//! lists, classed by the ids their keyframed values hold.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use super::bytes::{f32_at, sections, u32_at, u64_at, Bytes};
use super::{HeaderlessError, Result};
use crate::gta6::hash::name_hash32;

/// The root's size: the first record after it starts here.
const ROOT: usize = 0x70;
/// The pointer to the root, which every effect rule holds.
const ROOT_POINTER: u64 = 0x5000_0000;
const KFP: usize = 0x40;
/// How far past a behaviour's start its class is looked for.
const BEHAVIOUR_REACH: usize = 0x800;
const EFFECT_VERSION: u32 = 0x4086_6666;
const EMITTER_VERSION: u32 = 0x4083_3333;
const ZOOM_LEVEL: u32 = 0x42C8_0000;
/// A map's slot counts: it holds the smallest above its key count.
const MAP_SLOTS: [usize; 7] = [11, 29, 53, 103, 211, 409, 821];

const fn hashes<const N: usize>(names: [&str; N]) -> [u32; N] {
    let mut out = [0; N];
    let mut i = 0;
    while i < N {
        out[i] = name_hash32(names[i].as_bytes());
        i += 1;
    }
    out
}

const fn named<const N: usize>(names: [&'static str; N]) -> [(u32, &'static str); N] {
    let mut out = [(0, ""); N];
    let mut i = 0;
    while i < N {
        out[i] = (name_hash32(names[i].as_bytes()), names[i]);
        i += 1;
    }
    out
}

/// The ids of an effect rule's five keyframed values, in the order it holds them.
pub const EFFECT_KFPS: [u32; 5] = hashes([
    "ptxEffectRule:m_colourTintMinKFP",
    "ptxEffectRule:m_colourTintMaxKFP",
    "ptxEffectRule:m_zoomScalarKFP",
    "ptxEffectRule:m_dataSphereKFP",
    "ptxEffectRule:m_dataCapsuleKFP",
]);

/// The ids of an emitter rule's ten keyframed values, in the order it holds them.
pub const EMITTER_KFPS: [u32; 10] = hashes([
    "ptxEmitterRule:m_spawnRateOverTimeKFP",
    "ptxEmitterRule:m_spawnRateOverDistKFP",
    "ptxEmitterRule:m_particleLifeKFP",
    "ptxEmitterRule:m_playbackRateScalarKFP",
    "ptxEmitterRule:m_speedScalarKFP",
    "ptxEmitterRule:m_sizeScalarKFP",
    "ptxEmitterRule:m_accnScalarKFP",
    "ptxEmitterRule:m_dampeningScalarKFP",
    "ptxEmitterRule:m_matrixWeightScalarKFP",
    "ptxEmitterRule:m_inheritVelocityKFP",
]);

/// The ids of a domain's six keyframed values, by type: creation, target, attractor.
// Position, rotation, outer and inner size, then two whose names are not known.
pub const DOMAIN_KFPS: [[u32; 6]; 3] = {
    let creation = hashes([
        "ptxCreationDomain:m_positionKFP",
        "ptxCreationDomain:m_rotationKFP",
        "ptxCreationDomain:m_sizeOuterKFP",
        "ptxCreationDomain:m_sizeInnerKFP",
    ]);
    let target = hashes([
        "ptxTargetDomain:m_positionKFP",
        "ptxTargetDomain:m_rotationKFP",
        "ptxTargetDomain:m_sizeOuterKFP",
        "ptxTargetDomain:m_sizeInnerKFP",
    ]);
    let attractor = hashes([
        "ptxAttractorDomain:m_positionKFP",
        "ptxAttractorDomain:m_rotationKFP",
        "ptxAttractorDomain:m_sizeOuterKFP",
        "ptxAttractorDomain:m_sizeInnerKFP",
    ]);
    [
        [
            creation[0],
            creation[1],
            creation[2],
            creation[3],
            0xB70D_8F2C,
            0x68A5_C8AC,
        ],
        [
            target[0],
            target[1],
            target[2],
            target[3],
            0xEFDE_C010,
            0x8EE3_58E6,
        ],
        [
            attractor[0],
            attractor[1],
            attractor[2],
            attractor[3],
            0xA6F6_EE79,
            0x9AE3_10A7,
        ],
    ]
};

/// The `class:member` names a behaviour's keyframed values are found to hold, by their ids.
pub const BEHAVIOUR_KFPS: [(u32, &str); 46] = named([
    "ptxu_Acceleration:m_xyzMinKFP",
    "ptxu_Acceleration:m_xyzMaxKFP",
    "ptxu_Velocity:m_angleKFP",
    "ptxu_Rotation:m_initialAngleMinKFP",
    "ptxu_Rotation:m_initialAngleMaxKFP",
    "ptxu_Rotation:m_angleMinKFP",
    "ptxu_Rotation:m_angleMaxKFP",
    "ptxu_Size:m_whdMinKFP",
    "ptxu_Size:m_whdMaxKFP",
    "ptxu_Size:m_tblrScalarKFP",
    "ptxu_Size:m_tblrVelScalarKFP",
    "ptxu_Dampening:m_xyzMinKFP",
    "ptxu_Dampening:m_xyzMaxKFP",
    "ptxu_MatrixWeight:m_mtxWeightKFP",
    "ptxu_Collision:m_bouncinessKFP",
    "ptxu_Collision:m_bounceDirVarKFP",
    "ptxu_AnimateTexture:m_animRateKFP",
    "ptxu_Colour:m_emissiveIntensityKFP",
    "ptxu_ZCull:m_heightKFP",
    "ptxu_ZCull:m_fadeDistKFP",
    "ptxu_Decal:m_dimensionsKFP",
    "ptxu_Decal:m_alphaKFP",
    "ptxd_Trail:m_texInfoKFP",
    "ptxu_Attractor:m_strengthKFP",
    "ptxu_CameraEvolution:m_evolutionCurve",
    "ptxu_Wind:m_influenceKFP",
    "ptxu_Noise:m_posNoiseMinKFP",
    "ptxu_Noise:m_posNoiseMaxKFP",
    "ptxu_Noise:m_velNoiseMinKFP",
    "ptxu_Noise:m_velNoiseMaxKFP",
    "ptxu_FogVolume:m_rgbTintMinKFP",
    "ptxu_FogVolume:m_rgbTintMaxKFP",
    "ptxu_FogVolume:m_densityRangeKFP",
    "ptxu_FogVolume:m_scaleMinKFP",
    "ptxu_FogVolume:m_scaleMaxKFP",
    "ptxu_FogVolume:m_rotationMinKFP",
    "ptxu_FogVolume:m_rotationMaxKFP",
    "ptxu_Light:m_rgbMinKFP",
    "ptxu_Light:m_rgbMaxKFP",
    "ptxu_Light:m_intensityKFP",
    "ptxu_Light:m_rangeKFP",
    "ptxu_Light:m_coronaRgbMinKFP",
    "ptxu_Light:m_coronaRgbMaxKFP",
    "ptxu_Light:m_coronaIntensityKFP",
    "ptxu_Light:m_coronaSizeKFP",
    "ptxu_Light:m_coronaFlareKFP",
]);

/// Ids of keyframed values a class holds whose member names are not known.
const UNNAMED_KFPS: [(u32, &str); 6] = [
    (0xE785_1995, "ptxu_Colour"),
    (0xC144_4D14, "ptxu_Colour"),
    (0x6DFB_A680, "ptxu_Colour"),
    (0xA093_0BB2, "ptxu_Colour"),
    (0xD24D_EF27, "ptxu_Colour"),
    (0xCC31_AD40, "ptxu_FogVolume"),
];

/// The property registry's classes, each with the number of its entries.
const REGISTRY: [(&str, usize); 14] = [
    ("ptxu_Acceleration", 2),
    ("ptxu_Rotation", 4),
    ("ptxu_Dampening", 2),
    ("ptxu_MatrixWeight", 1),
    ("ptxu_Size", 4),
    ("ptxu_Colour", 6),
    ("ptxu_AnimateTexture", 1),
    ("ptxu_Attractor", 1),
    ("ptxu_Noise", 1),
    ("ptxu_Collision", 2),
    ("ptxu_ZCull", 2),
    ("ptxu_Wind", 1),
    ("ptxu_Velocity", 1),
    ("ptxu_CameraEvolution", 1),
];

#[derive(Debug, Clone, PartialEq)]
pub struct ParticleEffects {
    pub name: String,
    /// Each entry's id and class hash; what the ids name is not known.
    pub registry: Vec<(u32, u32)>,
    /// In the order of their dictionary's ascending codes, as are the other two.
    pub effects: Vec<EffectRule>,
    pub emitters: Vec<EmitterRule>,
    pub particles: Vec<ParticleRule>,
    /// Every behaviour any particle rule lists, once each, by place in the section.
    pub behaviours: Vec<Behaviour>,
    /// The set at 0x40: the hash of every drawable the particle rules name.
    pub drawable_hashes: BTreeSet<u32>,
    /// The set at 0x58: the hash of every named model a mesh domain spawns on.
    pub mesh_hashes: BTreeSet<u32>,
}

/// A keyframed value: `id` hashes `class:member`.
#[derive(Debug, Clone, PartialEq)]
pub struct Kfp {
    pub id: u32,
    /// `id`, or a target or attractor domain's creation id; a behaviour's may differ, unnamed.
    pub base_id: u32,
    pub entries: Vec<KfpEntry>,
}

/// One key: `time.x` in [0, 1], `time.y` the stored reciprocal of the step from the key before.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KfpEntry {
    pub time: [f32; 4],
    pub value: [f32; 4],
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvolutionList {
    pub evolutions: Vec<Evolution>,
    pub props: Vec<EvolvedProp>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Evolution {
    pub name: String,
    pub override_value: f32,
    pub is_overridden: bool,
}

/// A keyframed value evolutions vary, with its keyframe for each evolution that does.
#[derive(Debug, Clone, PartialEq)]
pub struct EvolvedProp {
    pub id: u32,
    pub blend_mode: u8,
    pub keyframes: Vec<EvolvedKeyframe>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EffectRule {
    pub at: u64,
    pub code: u32,
    pub name: String,
    pub events: Vec<Event>,
    pub evolution_list: Option<EvolutionList>,
    pub num_loops: i32,
    /// Sort events by distance, draw list id, short-lived, no shadows.
    pub flags: [u8; 4],
    pub rand_offset_pos: [f32; 3],
    /// Pre-update time and interval, duration min and max, playback rate min and max.
    pub times: [f32; 6],
    /// Viewport culling mode and its three flags, then distance culling's.
    pub culling_modes: [u8; 8],
    pub culling_sphere_offset: [f32; 3],
    /// Sphere radius, fade and cull distance, LOD evolution distance min and max.
    pub culling: [f32; 5],
    pub coln_range: f32,
    pub coln_probe_dist: f32,
    /// Colour tint min, max, zoom scalar, data sphere, data capsule.
    pub kfps: [Kfp; 5],
    /// Colour tint max enable, use data volume, data volume type.
    pub data_volume: [u8; 3],
}

/// An emitter event of an effect's timeline.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub start_ratio: f32,
    pub end_ratio: f32,
    /// Indices into the list's emitter and particle rules.
    pub emitter: usize,
    pub particle: usize,
    pub evolution_list: Option<EvolutionList>,
    pub playback_rate_scalar: [f32; 2],
    pub zoom_scalar: [f32; 2],
    pub colour_tint: [u32; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmitterRule {
    pub at: u64,
    pub code: u32,
    pub name: String,
    /// Creation, target and attractor; only the attractor may be absent.
    pub domains: [Option<Domain>; 3],
    /// In GTA V's order, spawn rate over time first, inherit velocity last.
    pub kfps: [Kfp; 10],
    /// Byte 0 reads as GTA V's one-shot flag; the rest are not known.
    pub flags: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Domain {
    /// 0 box, 1 sphere, 2 cylinder, 3 attractor, 4 a mesh.
    pub shape: u32,
    /// World space, point relative, creation relative, target relative.
    pub flags: [u8; 4],
    pub file_version: f32,
    /// Position, rotation, size outer, size inner, then two whose names are not known.
    pub kfps: [Kfp; 6],
    /// A mesh domain's model name, which may be empty, and the word beside it.
    pub model: Option<(String, u32)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParticleRule {
    pub at: u64,
    pub code: u32,
    pub name: String,
    /// At a ratio of the particle's life, on collision, and where its `ptxu_ZCull` culls a point.
    pub spawners: [Spawner; 3],
    pub tex_frame_id: [i32; 2],
    /// Indices into the list's behaviours, in the order the rule lists them.
    pub behaviours: Vec<usize>,
    /// One bit per class of its behaviours, in their order: [`class_of_bit`] names each.
    pub class_bits: u32,
    /// `.ttpf` keys, 0 for a slot not used; [`ParticleRule::sprite_slot`] is the sprite's.
    pub texture_keys: [u64; 13],
    /// A variable's hash and its value's, or 0; [`ParticleRule::shading`] names them.
    pub shader_words: Vec<(u32, u32)>,
    pub shader_hash: u32,
    pub drawables: Vec<DrawableRef>,
    pub sort_type: u8,
    /// 0 sprite, 1 model, 2 trail.
    pub draw_type: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Spawner {
    pub scalars_min: SpawnedScalars,
    pub scalars_max: SpawnedScalars,
    /// An index into the list's effect rules.
    pub effect: Option<usize>,
    pub name: String,
    pub trigger_info: f32,
    /// Inherits point life, tracks point position, direction, negative direction.
    pub flags: [u8; 4],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpawnedScalars {
    pub duration: f32,
    pub playback_rate: f32,
    pub colour_tint: u32,
    pub zoom: f32,
    pub flags: u32,
}

/// A drawable a model rule draws, named by `dictionary/drawable`; it is a resource of its own.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawableRef {
    pub name: String,
    /// The hash of the name after the `/`.
    pub hash: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Behaviour {
    pub at: u64,
    /// The class a keyframed value's id names; `None` where none does.
    pub class: Option<&'static str>,
    /// The bit of its class in the class word of each rule listing it; `None` where no rule lists it.
    pub class_bit: Option<u8>,
    /// Bytes to the next behaviour of any class, or to the section's end.
    pub size: u64,
    /// The keyframed values of its class it holds; none for a behaviour of no known class.
    pub kfps: Vec<Kfp>,
    /// Its own words where they are laid out, or the class its place, size or id gives it.
    pub fields: BehaviourFields,
}

fn refuse<T>(why: &'static str) -> Result<T> {
    Err(HeaderlessError::Particles(why))
}

fn need(holds: bool, why: &'static str) -> Result<()> {
    match holds {
        true => Ok(()),
        false => refuse(why),
    }
}

pub fn read_particle_effects(
    virtual_section: &[u8],
    physical: Option<&[u8]>,
) -> Result<ParticleEffects> {
    sections(virtual_section, physical, |v, _| list(v))
}

fn zero(v: &Bytes, what: &'static str, at: usize, len: usize) -> Result<bool> {
    Ok(v.get(what, at, len)?.iter().all(|b| *b == 0))
}

/// An `atArray` at `at`: its target, count and capacity.
fn array(v: &Bytes, what: &'static str, at: usize, equal: bool) -> Result<(usize, usize, usize)> {
    let (count, capacity) = (v.u16(what, at + 8)?, v.u16(what, at + 10)?);
    need(
        v.u32(what, at + 12)? == 0,
        "an array's word at +0xC is not 0",
    )?;
    need(count <= capacity, "an array's count is above its capacity")?;
    need(
        !equal || count == capacity,
        "an array's count is not its capacity",
    )?;
    match v.ptr(what, at)? {
        Some(to) => Ok((to, count.into(), capacity.into())),
        None if count == 0 => Ok((0, 0, capacity.into())),
        None => refuse("an array of elements has no pointer"),
    }
}

/// The printable string the pointer at `at` points at.
fn name(v: &Bytes, what: &'static str, at: usize) -> Result<String> {
    let s = v.c_str(what, v.to(what, at)?)?;
    need(
        s.len() <= 512 && s.bytes().all(|b| (32..127).contains(&b)),
        "a name is not printable ASCII of at most 512 bytes",
    )?;
    Ok(s)
}

fn list(v: Bytes) -> Result<ParticleEffects> {
    need(v.len() >= ROOT, "the section is shorter than the root")?;
    let head = v.u64("root", 0)? | v.u64("root", 0x18)?;
    need(head == 0, "the root's words at 0x00 and 0x18 are not 0")?;
    v.to("block map", 0x08)?;
    let list_name = name(&v, "list name", 0x10)?;
    let registry = registry(&v)?;
    let effect_dict = dictionary(&v, "effect rules", 0x28)?;
    let emitter_dict = dictionary(&v, "emitter rules", 0x30)?;
    let particle_dict = dictionary(&v, "particle rules", 0x38)?;
    let mut seen = HashSet::new();
    let mut rules = effect_dict
        .iter()
        .chain(&emitter_dict)
        .chain(&particle_dict);
    need(
        rules.all(|(_, at)| seen.insert(*at)),
        "two dictionary entries are one rule",
    )?;
    let (emitter_at, particle_at, effect_at) = (
        index(&emitter_dict),
        index(&particle_dict),
        index(&effect_dict),
    );
    let mut refs: HashMap<usize, u32> = HashMap::new();
    let effects = (effect_dict.iter())
        .map(|&(code, at)| effect(&v, at, code, &emitter_at, &particle_at, &mut refs))
        .collect::<Result<Vec<_>>>()?;
    let mut mesh_hashes = BTreeSet::new();
    let emitters = (emitter_dict.iter())
        .map(|&(code, at)| emitter(&v, at, code, &mut mesh_hashes))
        .collect::<Result<Vec<_>>>()?;
    let mut listed_anywhere = BTreeSet::new();
    let mut lists = Vec::with_capacity(particle_dict.len());
    for &(code, at) in &particle_dict {
        let (rule, listed) = particle(&v, at, code, &effect_at)?;
        listed_anywhere.extend(listed.iter().copied());
        lists.push((rule, listed));
    }
    for (_, at) in emitter_dict.iter().chain(&particle_dict) {
        let events = refs.get(at).copied().unwrap_or(0);
        need(
            v.u32("reference count", at + 0x10)? == 1 + events,
            "a rule's reference count is not 1 and the events naming it",
        )?;
    }
    let places: Vec<usize> = listed_anywhere.into_iter().collect();
    let mut behaviours = (places.iter().enumerate())
        .map(|(i, &at)| behaviour(&v, at, places.get(i + 1).copied().unwrap_or(v.len())))
        .collect::<Result<Vec<_>>>()?;
    let mut drawable_hashes = BTreeSet::new();
    let mut particles = Vec::with_capacity(lists.len());
    for (mut rule, listed) in lists {
        let trail = listed.iter().any(|b| is_trail(&v, *b));
        need(
            (rule.draw_type == 2) == trail,
            "a rule draws as a trail without a trail behaviour, or the reverse",
        )?;
        rule.behaviours = (listed.iter())
            .map(|b| places.binary_search(b).unwrap_or_default())
            .collect();
        drawable_hashes.extend(rule.drawables.iter().map(|d| d.hash));
        particles.push(rule);
    }
    bits::assign(&mut behaviours, &particles)?;
    parts::label(&v, &mut behaviours)?;
    parts::event_props_held(&v, &effects, &emitters, &behaviours)?;
    let drawn = atmap(&v, "drawable set", 0x40, false)?;
    need(
        drawn.keys().copied().eq(drawable_hashes.iter().copied()),
        "the set at 0x40 is not the drawables' hashes",
    )?;
    let models = atmap(&v, "model set", 0x58, false)?;
    need(
        models.keys().copied().eq(mesh_hashes.iter().copied()),
        "the set at 0x58 is not the mesh domains' model hashes",
    )?;
    Ok(ParticleEffects {
        name: list_name,
        registry,
        effects,
        emitters,
        particles,
        behaviours,
        drawable_hashes,
        mesh_hashes,
    })
}

/// Each entry's place in the section, to its index in the dictionary and its code.
fn index(dict: &[(u32, usize)]) -> HashMap<usize, (usize, u32)> {
    (dict.iter().enumerate())
        .map(|(i, &(code, at))| (at, (i, code)))
        .collect()
}

fn registry(v: &Bytes) -> Result<Vec<(u32, u32)>> {
    const WHAT: &str = "property registry";
    let r = v.to(WHAT, 0x20)?;
    let head = v.u64(WHAT, r)? | v.u64(WHAT, r + 0x18)?;
    need(head == 0, "the registry's words at 0x00 and 0x18 are not 0")?;
    let (to, count, _) = array(v, WHAT, r + 8, true)?;
    let raw = v.array(WHAT, to, count, 0x10)?;
    let entries: Vec<(u32, u32)> = (raw.as_chunks::<0x10>().0.iter())
        .filter(|e| u64_at(&e[..], 0) == 0)
        .map(|e| (u32_at(e, 8), u32_at(e, 0xC)))
        .collect();
    need(
        entries.len() == count,
        "a registry entry's word at 0x00 is not 0",
    )?;
    for (class, n) in REGISTRY {
        let hash = name_hash32(class.as_bytes());
        let held = entries.iter().filter(|(_, c)| *c == hash).count();
        need(
            held == n,
            "the registry's classes are not the measured ones",
        )?;
    }
    let measured: usize = REGISTRY.iter().map(|(_, n)| n).sum();
    need(
        entries.len() == measured,
        "the registry holds a class not measured",
    )?;
    Ok(entries)
}

/// A dictionary's codes and the entries they key.
fn dictionary(v: &Bytes, what: &'static str, at: usize) -> Result<Vec<(u32, usize)>> {
    let d = v.to(what, at)?;
    let head = zero(v, what, d, 0x18)? && v.u64(what, d + 0x18)? == 1;
    need(head, "a dictionary's head is not (0, 0, 0, 1)")?;
    let (codes_at, codes, _) = array(v, what, d + 0x20, true)?;
    let (entries_at, entries, _) = array(v, what, d + 0x30, true)?;
    need(
        codes == entries,
        "a dictionary's codes and entries differ in count",
    )?;
    let codes = v.array(what, codes_at, codes, 4)?;
    v.array(what, entries_at, entries, 8)?;
    let codes: Vec<u32> = codes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_le_bytes(*c))
        .collect();
    need(
        codes.windows(2).all(|w| w[0] < w[1]),
        "a dictionary's codes do not ascend",
    )?;
    (codes.iter().enumerate())
        .map(|(i, code)| Ok((*code, v.to(what, entries_at + 8 * i)?)))
        .collect()
}

/// A keyframed value at `at`, refused unless it holds `ids`, where given.
fn kfp(v: &Bytes, at: usize, ids: Option<(u32, u32)>) -> Result<Kfp> {
    const WHAT: &str = "keyframed value";
    let (id, base_id) = (v.u32(WHAT, at + 0x28)?, v.u32(WHAT, at + 0x2C)?);
    // ⚠ A colour's first unnamed value may hold a word at +0x30, which stays as stored.
    let tail = match ids.is_none() && id == COLOUR_RGBA_KFPS[0] {
        true => 4,
        false => 0,
    };
    let runtime = v.u64(WHAT, at)? == 0
        && zero(v, WHAT, at + 0x18, 0x10)?
        && zero(v, WHAT, at + 0x30 + tail, 0x10 - tail)?;
    need(runtime, "a keyframed value's runtime words are not 0")?;
    need(
        ids.is_none_or(|ids| ids == (id, base_id)),
        "a keyframed value's ids are not its place's",
    )?;
    let (to, count, _) = array(v, WHAT, at + 8, true)?;
    let raw = v.array("keyframe entries", to, count, 0x20)?;
    let mut entries: Vec<KfpEntry> = Vec::with_capacity(count);
    for e in raw.as_chunks::<0x20>().0 {
        let f: [f32; 8] = std::array::from_fn(|i| f32_at(e, 4 * i));
        need(f.iter().all(|x| x.is_finite()), "a key is not finite")?;
        let time = (0.0..=1.0).contains(&f[0]) && f[2] == 0.0 && f[3] == 0.0;
        need(
            time,
            "a key's time is outside [0, 1] or its lanes z, w are set",
        )?;
        match entries.last() {
            None => need(f[1] == 0.0, "the first key's time.y is not 0")?,
            Some(last) => need(f[0] >= last.time[0], "a key's time goes back")?,
        }
        entries.push(KfpEntry {
            time: [f[0], f[1], f[2], f[3]],
            value: [f[4], f[5], f[6], f[7]],
        });
    }
    Ok(Kfp {
        id,
        base_id,
        entries,
    })
}

/// The keyframed values at `starts` with the ids `ids` and `base`, which the list at `list` holds.
fn kfps<const N: usize>(
    v: &Bytes,
    starts: [usize; N],
    ids: [u32; N],
    base: [u32; N],
    list: usize,
    capacity: usize,
) -> Result<[Kfp; N]> {
    const WRONG: &str = "a keyframe list does not hold its record's values";
    let (to, count, cap) = array(v, "keyframe list", list, false)?;
    need(count == N && cap == capacity, WRONG)?;
    let mut held: Vec<usize> = (0..N)
        .map(|i| v.to("keyframe list", to + 8 * i))
        .collect::<Result<_>>()?;
    let mut own = starts.to_vec();
    held.sort_unstable();
    own.sort_unstable();
    need(held == own, WRONG)?;
    let read: Vec<Kfp> = (0..N)
        .map(|i| kfp(v, starts[i], Some((ids[i], base[i]))))
        .collect::<Result<_>>()?;
    read.try_into().or_else(|_| refuse(WRONG))
}

/// The evolution list at `at`, whose values are of the ids `owner` where given.
fn evolution_list(v: &Bytes, at: usize, owner: Option<&[u32]>) -> Result<EvolutionList> {
    const WHAT: &str = "evolution list";
    let (to, count, _) = array(v, WHAT, at, true)?;
    v.array(WHAT, to, count, 0x18)?;
    let evolutions = (0..count)
        .map(|i| {
            let e = to + 0x18 * i;
            let name = name(v, WHAT, e)?;
            need(
                v.u32(WHAT, e + 8)? == name_hash32(name.as_bytes()),
                "an evolution's hash is not its name's",
            )?;
            Ok(Evolution {
                name,
                override_value: v.f32(WHAT, e + 0xC)?,
                is_overridden: v.u8(WHAT, e + 0x10)? != 0,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let (to, count, _) = array(v, WHAT, at + 0x10, true)?;
    v.array(WHAT, to, count, 0x18)?;
    let mut props = Vec::with_capacity(count);
    let mut by_id = BTreeMap::new();
    for i in 0..count {
        let p = to + 0x18 * i;
        let id = v.u32(WHAT, p + 0x10)?;
        let [blend_mode, pad @ ..] = u8s::<4>(v, WHAT, p + 0x14)?;
        need(
            blend_mode <= 3 && pad == [0; 3],
            "an evolved value's blend mode is above 3 or its pad is set",
        )?;
        need(
            owner.is_none_or(|ids| ids.contains(&id)),
            "an evolved value is no keyframed value of its effect rule",
        )?;
        by_id.insert(id, Some(p));
        props.push(EvolvedProp {
            id,
            blend_mode,
            keyframes: parts::evolved_keyframes(v, p, &evolutions)?,
        });
    }
    let map = atmap(v, WHAT, at + 0x20, true)?;
    need(
        by_id.len() == count && map == by_id,
        "an evolution list's map does not give each value its prop",
    )?;
    Ok(EvolutionList { evolutions, props })
}

/// The keys of the `atMap` at `at` and each node's data, which a set holds none of.
fn atmap(
    v: &Bytes,
    what: &'static str,
    at: usize,
    data: bool,
) -> Result<BTreeMap<u32, Option<usize>>> {
    let (slots, used) = (
        v.u32(what, at + 8)? as usize,
        v.u32(what, at + 0x10)? as usize,
    );
    let pad = v.u32(what, at + 0xC)? | v.u32(what, at + 0x14)?;
    need(pad == 0, "a map's pad words are not 0")?;
    let mut keys = BTreeMap::new();
    let Some(buckets) = v.ptr(what, at)? else {
        need(
            slots == 0 && used == 0,
            "a map with no buckets counts slots or keys",
        )?;
        return Ok(keys);
    };
    need(
        slots != 0 && used <= v.len() / 0x20,
        "a map's slot or key count does not fit the section",
    )?;
    need(
        MAP_SLOTS.into_iter().find(|&n| n > used) == Some(slots),
        "a map's slot count is not the smallest of 11, 29, 53, 103, 211, 409 and 821 above its key count",
    )?;
    v.array(what, buckets, slots, 8)?;
    let mut hops = 0;
    for slot in 0..slots {
        let mut node = v.ptr(what, buckets + 8 * slot)?;
        while let Some(n) = node {
            hops += 1;
            need(hops <= used, "a map's chains are longer than its key count")?;
            let key = v.u32(what, n)?;
            need(
                key as usize % slots == slot,
                "a map's key is in another key's bucket",
            )?;
            let pad = u64::from(v.u32(what, n + 4)?) | v.u64(what, n + 0x18)?;
            need(pad == 0, "a map node's pad words are not 0")?;
            let d = v.ptr(what, n + 8)?;
            need(
                d.is_some() == data,
                "a map node's data is not what its map holds",
            )?;
            keys.insert(key, d);
            node = v.ptr(what, n + 0x10)?;
        }
    }
    need(keys.len() == used, "a map's keys are not its key count")?;
    Ok(keys)
}

fn floats<const N: usize>(v: &Bytes, what: &'static str, at: usize) -> Result<[f32; N]> {
    let b = v.get(what, at, 4 * N)?;
    Ok(std::array::from_fn(|i| f32_at(b, 4 * i)))
}

fn u8s<const N: usize>(v: &Bytes, what: &'static str, at: usize) -> Result<[u8; N]> {
    let b = v.get(what, at, N)?;
    Ok(std::array::from_fn(|i| b[i]))
}

/// The place and index in `dict` of the rule the event points at from `at` and names at `names`.
fn event_rule(
    v: &Bytes,
    e: usize,
    at: usize,
    names: usize,
    dict: &HashMap<usize, (usize, u32)>,
) -> Result<(usize, usize)> {
    let rule_name = name(v, "event", e + names)?;
    let to = v.to("event", e + at)?;
    match dict.get(&to) {
        Some(&(i, code)) if code == name_hash32(rule_name.as_bytes()) => Ok((to, i)),
        _ => refuse("an event's rule is not the one its name names"),
    }
}

fn effect(
    v: &Bytes,
    at: usize,
    code: u32,
    emitters: &HashMap<usize, (usize, u32)>,
    particles: &HashMap<usize, (usize, u32)>,
    refs: &mut HashMap<usize, u32>,
) -> Result<EffectRule> {
    const WHAT: &str = "effect rule";
    let head = zero(v, WHAT, at, 0x10)? && v.u32(WHAT, at + 0x10)? == 1;
    need(head, "an effect rule's head is not (0, 0, 1)")?;
    need(
        v.u32(WHAT, at + 0x18)? == EFFECT_VERSION,
        "an effect rule's file version is not 4.2",
    )?;
    let rule_name = name(v, WHAT, at + 0x20)?;
    need(
        name_hash32(rule_name.as_bytes()) == code,
        "an effect rule's code is not its name's hash",
    )?;
    let root = v.u64(WHAT, at + 0x28)? == ROOT_POINTER && v.u64(WHAT, at + 0x30)? == 0;
    need(root, "an effect rule does not point at its list")?;
    let starts: [usize; 5] = std::array::from_fn(|i| at + 0xC0 + KFP * i);
    let kfps = kfps(v, starts, EFFECT_KFPS, EFFECT_KFPS, at + 0x200, 16)?;
    need(
        v.u32(WHAT, at + 0x23C)? == ZOOM_LEVEL,
        "an effect rule's zoom level is not 100",
    )?;
    let evolution = (v.ptr(WHAT, at + 0x48)?)
        .map(|l| evolution_list(v, l, Some(&EFFECT_KFPS[..])))
        .transpose()?;
    let (to, count, _) = array(v, WHAT, at + 0x38, false)?;
    v.array(WHAT, to, count, 8)?;
    let mut events = Vec::with_capacity(count);
    for i in 0..count {
        let e = v.to("event", to + 8 * i)?;
        let head = v.u64("event", e)? == 0
            && v.u32("event", e + 8)? as usize == i
            && v.u32("event", e + 0xC)? == 0;
        need(head, "an event's head is not (0, its place, 0)")?;
        let (emitter_at, emitter) = event_rule(v, e, 0x50, 0x40, emitters)?;
        let (particle_at, particle) = event_rule(v, e, 0x58, 0x48, particles)?;
        *refs.entry(emitter_at).or_default() += 1;
        *refs.entry(particle_at).or_default() += 1;
        let [start_ratio, end_ratio] = floats(v, "event", e + 0x10)?;
        let [rate_min, rate_max, zoom_min, zoom_max] = floats(v, "event", e + 0x68)?;
        events.push(Event {
            start_ratio,
            end_ratio,
            emitter,
            particle,
            evolution_list: (v.ptr("event", e + 0x30)?)
                .map(|l| evolution_list(v, l, None))
                .transpose()?,
            playback_rate_scalar: [rate_min, rate_max],
            zoom_scalar: [zoom_min, zoom_max],
            colour_tint: [v.u32("event", e + 0x78)?, v.u32("event", e + 0x7C)?],
        });
    }
    Ok(EffectRule {
        at: at as u64,
        code,
        name: rule_name,
        events,
        evolution_list: evolution,
        num_loops: v.u32(WHAT, at + 0x50)? as i32,
        flags: u8s(v, WHAT, at + 0x54)?,
        rand_offset_pos: v.vec3(WHAT, at + 0x60)?,
        times: floats(v, WHAT, at + 0x70)?,
        culling_modes: u8s(v, WHAT, at + 0x88)?,
        culling_sphere_offset: v.vec3(WHAT, at + 0x90)?,
        culling: floats(v, WHAT, at + 0xA0)?,
        coln_range: v.f32(WHAT, at + 0xB4)?,
        coln_probe_dist: v.f32(WHAT, at + 0xB8)?,
        kfps,
        data_volume: u8s(v, WHAT, at + 0x210)?,
    })
}

fn emitter(v: &Bytes, at: usize, code: u32, meshes: &mut BTreeSet<u32>) -> Result<EmitterRule> {
    const WHAT: &str = "emitter rule";
    need(
        zero(v, WHAT, at, 0x10)?,
        "an emitter rule's words at 0x00 and 0x08 are not 0",
    )?;
    need(
        v.u32(WHAT, at + 0x18)? == EMITTER_VERSION,
        "an emitter rule's file version is not 4.1",
    )?;
    let rule_name = name(v, WHAT, at + 0x20)?;
    let coded = v.u32(WHAT, at + 0x1C)? == code && name_hash32(rule_name.as_bytes()) == code;
    need(coded, "an emitter rule's code is not its name's hash")?;
    need(
        v.u64(WHAT, at + 0x28)? == 0,
        "an emitter rule's word at 0x28 is not 0",
    )?;
    let mut domains: [Option<Domain>; 3] = Default::default();
    for (kind, slot) in domains.iter_mut().enumerate() {
        let Some(d) = v.ptr(WHAT, at + 0x30 + 8 * kind)? else {
            need(
                kind == 2,
                "an emitter rule has no creation or target domain",
            )?;
            continue;
        };
        *slot = Some(domain(v, d, kind, meshes)?);
    }
    let starts: [usize; 10] = std::array::from_fn(|i| at + 0x48 + KFP * i);
    Ok(EmitterRule {
        at: at as u64,
        code,
        name: rule_name,
        domains,
        kfps: kfps(v, starts, EMITTER_KFPS, EMITTER_KFPS, at + 0x2C8, 10)?,
        flags: v.u32(WHAT, at + 0x2F0)?,
    })
}

fn domain(v: &Bytes, at: usize, kind: usize, meshes: &mut BTreeSet<u32>) -> Result<Domain> {
    const WHAT: &str = "domain";
    need(v.u64(WHAT, at)? == 0, "a domain's word at 0x00 is not 0")?;
    need(
        v.u32(WHAT, at + 0x188)? as usize == kind,
        "a domain's type is not its emitter slot",
    )?;
    let shape = v.u32(WHAT, at + 0x18C)?;
    need(shape <= 4, "a domain's shape is none of the five measured")?;
    let starts: [usize; 6] = std::array::from_fn(|i| at + 8 + KFP * i);
    let kfps = kfps(v, starts, DOMAIN_KFPS[kind], DOMAIN_KFPS[0], at + 0x198, 16)?;
    let model = match shape {
        4 => {
            let model = name(v, WHAT, at + 0x1B0)?;
            let pad = v.u64(WHAT, at + 0x1B8)? | u64::from(v.u32(WHAT, at + 0x1C4)?);
            need(
                pad == 0,
                "a mesh domain's words at 0x1B8 and 0x1C4 are not 0",
            )?;
            if !model.is_empty() {
                meshes.insert(name_hash32(model.as_bytes()));
            }
            Some((model, v.u32(WHAT, at + 0x1C0)?))
        }
        _ => None,
    };
    Ok(Domain {
        shape,
        flags: u8s(v, WHAT, at + 0x190)?,
        file_version: v.f32(WHAT, at + 0x194)?,
        kfps,
        model,
    })
}

fn scalars(v: &Bytes, at: usize) -> Result<SpawnedScalars> {
    let b = v.get("spawner", at, 0x14)?;
    Ok(SpawnedScalars {
        duration: f32_at(b, 0),
        playback_rate: f32_at(b, 4),
        colour_tint: u32_at(b, 8),
        zoom: f32_at(b, 0xC),
        flags: u32_at(b, 0x10),
    })
}

/// The spawner of `len` bytes at `at`; `effects` gives an effect rule's place its index.
fn spawner(
    v: &Bytes,
    at: usize,
    len: usize,
    effects: &HashMap<usize, (usize, u32)>,
) -> Result<Spawner> {
    const WHAT: &str = "spawner";
    let ends = v.u64(WHAT, at)? == 0 && zero(v, WHAT, at + 0x60, len - 0x60)?;
    need(ends, "a spawner's head or tail is not 0")?;
    let effect = match v.ptr(WHAT, at + 0x48)? {
        None => None,
        Some(e) => match effects.get(&e) {
            Some(&(i, _)) => Some(i),
            None => return refuse("a spawner's effect is not a rule of the list"),
        },
    };
    Ok(Spawner {
        scalars_min: scalars(v, at + 0x08)?,
        scalars_max: scalars(v, at + 0x28)?,
        effect,
        name: name(v, WHAT, at + 0x50)?,
        trigger_info: v.f32(WHAT, at + 0x58)?,
        flags: u8s(v, WHAT, at + 0x5C)?,
    })
}

fn drawable_ref(v: &Bytes, at: usize) -> Result<DrawableRef> {
    const WHAT: &str = "drawable reference";
    let path = name(v, WHAT, at)?;
    let hash = v.u32(WHAT, at + 0x10)?;
    let leaf = path.rsplit('/').next().unwrap_or_default();
    need(
        hash == name_hash32(leaf.as_bytes()),
        "a drawable reference's hash is not its drawable's name's",
    )?;
    let pad = v.u64(WHAT, at + 8)? | u64::from(v.u32(WHAT, at + 0x14)?) | v.u64(WHAT, at + 0x18)?;
    need(pad == 0, "a drawable reference's words are not 0")?;
    Ok(DrawableRef { name: path, hash })
}

/// A particle rule, and the places of the behaviours it lists.
fn particle(
    v: &Bytes,
    at: usize,
    code: u32,
    effects: &HashMap<usize, (usize, u32)>,
) -> Result<(ParticleRule, Vec<usize>)> {
    const WHAT: &str = "particle rule";
    need(
        zero(v, WHAT, at, 0x10)?,
        "a particle rule's words at 0x00 and 0x08 are not 0",
    )?;
    let rule_name = name(v, WHAT, at + 0x178)?;
    need(
        name_hash32(rule_name.as_bytes()) == code,
        "a particle rule's code is not its name's hash",
    )?;
    let spawners = [
        spawner(v, at + 0x20, 0x70, effects)?,
        spawner(v, at + 0x90, 0x70, effects)?,
        spawner(v, at + 0x100, 0x68, effects)?,
    ];
    let (to, count, _) = array(v, WHAT, at + 0x188, true)?;
    v.array(WHAT, to, count, 8)?;
    let listed = (0..count)
        .map(|i| {
            let b = v.to("behaviour", to + 8 * i)?;
            need(
                v.u64("behaviour", b)? == 0,
                "a behaviour's word at 0x00 is not 0",
            )?;
            Ok(b)
        })
        .collect::<Result<Vec<_>>>()?;
    let keys = v.get(WHAT, at + 0x2F0, 13 * 8)?;
    need(
        zero(v, WHAT, at + 0x358, 0x10)?,
        "a particle rule's words after its texture keys are not 0",
    )?;
    let (to, count, _) = array(v, WHAT, at + 0x370, true)?;
    let words = v.array(WHAT, to, count, 8)?;
    let (refs_at, refs, _) = array(v, WHAT, at + 0x3B0, true)?;
    let (slots_at, slots, _) = array(v, WHAT, at + 0x3C0, true)?;
    let (_, weights, _) = array(v, WHAT, at + 0x3D0, true)?;
    need(
        refs == slots && slots == weights,
        "a particle rule's drawable arrays differ in count",
    )?;
    v.array(WHAT, refs_at, refs, 0x20)?;
    let slot_words = v.array(WHAT, slots_at, slots, 8)?;
    need(
        slot_words.iter().all(|b| *b == 0),
        "a drawable's runtime slot is not 0",
    )?;
    let drawables = (0..refs)
        .map(|i| drawable_ref(v, refs_at + 0x20 * i))
        .collect::<Result<Vec<_>>>()?;
    let (sort_type, draw_type) = (v.u8(WHAT, at + 0x410)?, v.u8(WHAT, at + 0x411)?);
    need(
        sort_type <= 4 && draw_type <= 2,
        "a particle rule's sort or draw type is out of range",
    )?;
    need(
        drawables.is_empty() || draw_type == 1,
        "a rule that does not draw models names drawables",
    )?;
    let class_bits = bits::rule_words(v, at, draw_type)?;
    let rule = ParticleRule {
        at: at as u64,
        code,
        name: rule_name,
        spawners,
        tex_frame_id: [
            v.u32(WHAT, at + 0x170)? as i32,
            v.u32(WHAT, at + 0x174)? as i32,
        ],
        behaviours: Vec::new(),
        class_bits,
        texture_keys: std::array::from_fn(|i| u64_at(keys, 8 * i)),
        shader_words: (words.as_chunks::<8>().0.iter())
            .map(|w| (u32_at(w, 0), u32_at(w, 4)))
            .collect(),
        shader_hash: v.u32(WHAT, at + 0x3A8)?,
        drawables,
        sort_type,
        draw_type,
    };
    parts::checked(&rule)?;
    Ok((rule, listed))
}

/// The class the first value at an 8-byte step past `at + 8` names, by an id equal to its base id.
fn class_of(v: &Bytes, at: usize, end: usize) -> Result<Option<&'static str>> {
    let end = end.min(at + BEHAVIOUR_REACH);
    let mut o = at + 8;
    while o + KFP <= end {
        let id = v.u32("behaviour", o + 0x28)?;
        if id != 0 && id == v.u32("behaviour", o + 0x2C)? && v.u64("behaviour", o)? == 0 {
            if let Some(&(_, name)) = BEHAVIOUR_KFPS.iter().find(|(h, _)| *h == id) {
                return Ok(name.split(':').next());
            }
        }
        o += 8;
    }
    Ok(None)
}

fn is_trail(v: &Bytes, at: usize) -> bool {
    class_of(v, at, at + 8 + KFP).ok().flatten() == Some("ptxd_Trail")
}

fn behaviour(v: &Bytes, at: usize, next: usize) -> Result<Behaviour> {
    let class = class_of(v, at, next)?;
    let mut kfps = Vec::new();
    if let Some(class) = class {
        let ids: Vec<u32> = (BEHAVIOUR_KFPS.iter().chain(&UNNAMED_KFPS))
            .filter(|(_, name)| name.split(':').next() == Some(class))
            .map(|(id, _)| *id)
            .collect();
        let end = next.min(at + BEHAVIOUR_REACH);
        let mut o = at + 8;
        while o + KFP <= end {
            // ⚠ A behaviour's base id does not always equal its id, so it is not asked for.
            let found = match ids.contains(&v.u32("behaviour", o + 0x28)?) {
                true => kfp(v, o, None).ok(),
                false => None,
            };
            match found {
                Some(k) => {
                    kfps.push(k);
                    o += KFP;
                }
                None => o += 8,
            }
        }
    }
    let fields = match class {
        Some("ptxu_AnimateTexture") => parts::animate_texture(v, at, next)?,
        Some("ptxu_ZCull") => bits::zcull(v, at, next)?,
        Some("ptxu_Colour") => colour_word::colour(v, at, next)?,
        _ => BehaviourFields::None,
    };
    Ok(Behaviour {
        at: at as u64,
        class,
        class_bit: None,
        size: (next - at) as u64,
        kfps,
        fields,
    })
}

#[path = "particles_open.rs"]
mod parts;
pub use parts::{
    BehaviourFields, DiffuseMode, EvolvedKeyframe, LightingMode, ProjectionMode, ShaderSwitch,
    Shading, COLOUR_RGBA_KFPS,
};

#[path = "particles_write.rs"]
mod writer;
pub use writer::write_particle_effects;

#[path = "particles_remove.rs"]
mod remover;
pub use remover::{remove_from_particle_effects, ParticleRemovals};

#[path = "particles_keys.rs"]
mod keys;
pub use keys::{add_particle_keys, ParticleKeysError};

#[path = "particles_copy.rs"]
mod copier;
pub use copier::duplicate_particle_effect;

#[path = "particles_dup.rs"]
mod dup;
pub use dup::{
    copy_particle_behaviour, duplicate_particle_emitter, duplicate_particle_event,
    duplicate_particle_rule,
};

#[path = "particles_gpu.rs"]
mod gpu;
pub use gpu::{read_particle_gpu_buffers, read_particle_gpu_tables, GpuBuffer, GpuTables};

#[path = "particles_bits.rs"]
mod bits;
pub use bits::{class_of_bit, ClassNamedBy};

#[path = "particles_colour.rs"]
mod colour_word;

#[path = "particles_fields.rs"]
mod fields_word;

#[path = "particles_tables.rs"]
mod tables;
pub use tables::{
    bake_particle_tables, read_particle_curves, write_particle_curves, ParticleCurve,
    ParticleCurves,
};

#[cfg(test)]
#[path = "particles_tests.rs"]
mod tests;
