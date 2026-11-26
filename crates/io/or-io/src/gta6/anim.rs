//! GTA VI's animation resources, read only: a clip dictionary, a lone animation set or a motion
//! dictionary at the root of a virtual section, and what each states.
//!
//! ⚠ A block's channel directory holds 11 section counts where GTA V's holds 9, and GTA V's codec
//! reads the 11 without an error and returns the wrong channels. Sections 0 to 5, 7 and 8 hold GTA
//! V's formats and decode through that codec once the directory is rewritten to its 9 slots;
//! section 10 holds a segmented delta code decoded here, and sections 6 and 9, met in no block,
//! are refused.

mod camera_cuts;
mod columns;
mod databases;
mod expressions;
mod expressions_layout;
mod matcher;
mod metadata;
mod motion;
mod section10;
mod signature;
mod write;

use std::cell::Cell;
use std::collections::{HashMap, HashSet};

pub use camera_cuts::{read_camera_cuts, CameraCut};
pub use columns::{column_parameter, ColumnParameter};
pub use databases::{Column, SimplexDatabase, TableDatabase, Tetrahedron};
pub use expressions::{
    read_expressions, write_expression_ids, ExpressionGroup, ExpressionTracks, Expressions,
    EXPRESSIONS_SIGNATURE,
};
pub use expressions_layout::{
    add_expression_track, lay_out_expressions, offered_facial_outputs, remove_expression_track,
    write_edited_expressions,
};
pub use matcher::{
    Matcher, MatcherClass, MatcherClip, MatcherDatabase, MatcherEntry, MatcherFeature, MatcherLevel,
};
pub use metadata::{known_name, Attribute, AttributeValue, Record};
pub use motion::{
    ArrayHead, Database, Grid, Motion, MotionGroup, Parameter, ParameterItem, ParameterObject,
    PhaseCurve, TimeWarp, Typed, Weight,
};
pub use signature::dof_signature;
pub use write::{edit, edit_in_extent, reencode, rename_clip, AnimEdit};

use crate::codec::anim::channel::{
    reconstruct_component, ChannelFormat, DecodedChannel, FORMAT_COUNT,
};
use crate::codec::anim::{AnimationSource, BlockSource, ChannelWalk, DofSamples};
use crate::decl::{Aligned, Image, Raw, View};
use crate::gta5::CrBlockStream;
use crate::gta6::hash::name_hash32;
use crate::IoError;

/// The RSC8 version byte of a stored animation resource.
pub const RSC8_VERSION: u8 = crate::container::rpf::rpf9_scan::RSC8_ANIMATION_VERSION;
/// Sections a block's channel directory counts.
pub const SECTIONS: usize = 11;
/// The section whose channels hold a segmented second-order delta code in place of GTA V's kLinear stream.
pub const DELTA_SECTION: usize = 10;
/// Sections no block was seen to use, whose formats are not known.
const UNKNOWN_SECTIONS: [usize; 2] = [6, 9];
const DICTIONARY_LEN: usize = 0xB0;
const SET_LEN: usize = 0x70;
const ANIMATION_LEN: usize = 0x70;
const CLIP_ONE_LEN: usize = 0xA0;
const CLIP_GROUPS_LEN: usize = 0x80;
const GROUP_LEN: usize = 0x18;
const ENTRY_LEN: usize = 0x38;
const DOF_LEN: usize = 12;
/// The smallest map entry, which bounds how many entries a section can hold.
const MAP_ENTRY_MIN: usize = 0x10;

/// Track names in camelCase, whose hashes GTA VI's DOFs carry as their track word.
const TRACK_NAMES: [&str; 23] = [
    "boneRotation",
    "boneTranslation",
    "boneScale",
    "boneOffset",
    "genericControl",
    "genericTranslation",
    "genericRotation",
    "facialTranslation",
    "cameraTranslation",
    "cameraRotation",
    "cameraFocusDistance",
    "cameraFieldOfView",
    "cameraLimit",
    "firstPersonCameraWeight",
    "shaderSlideU",
    "shaderSlideV",
    "moverAbsoluteTranslation",
    "moverAbsoluteRotation",
    "moverDeltaTranslation",
    "moverDeltaRotation",
    "footCyclePhase",
    "anchorDistance",
    "behavior",
];

/// Why an animation resource, or a value of one, cannot be read.
#[derive(Debug, thiserror::Error)]
pub enum AnimError {
    #[error(
        "the {what} at {at:#X} needs {len} bytes, past the end of the {size}-byte virtual section"
    )]
    PastEnd {
        what: &'static str,
        at: usize,
        len: usize,
        size: usize,
    },
    #[error("the {what} at {at:#X} holds {value:#X}, which is no address in the virtual section ({why}); every object of these resources was measured there")]
    Pointer {
        what: &'static str,
        at: usize,
        value: u64,
        why: String,
    },
    #[error("the root holds a non-zero byte in its first eight or between 0x10 and 0x38, where every animation resource's root holds zero")]
    RootNotZero,
    #[error("the root's word at 0x6C is {0}: 2 or 3 is a clip dictionary and 0 an animation set or a motion dictionary, and no other root is known")]
    RootKind(u32),
    #[error("the {what} {name:?} states key {key:#010X}, which is not its name's hash")]
    Key {
        what: &'static str,
        name: String,
        key: u32,
    },
    #[error("the {what} map at {at:#X} {why}")]
    Map {
        what: &'static str,
        at: usize,
        why: String,
    },
    #[error("entry {index} of the dictionary's animation set list {why}")]
    SetList { index: usize, why: String },
    #[error("the dictionary's animation set list names no set this body stores, and every dictionary was measured to store one")]
    NoSet,
    #[error("clip {clip:?}: its key is not its leaf name's hash low and its folder's hash, or the dictionary's key, high")]
    ClipKey { clip: String },
    #[error("clip {clip:?} has type {value}: only 1, one animation, and 2, groups of animations, are known")]
    ClipType { clip: String, value: u32 },
    #[error("clip {clip:?} plays animation {key:#010X} from a set another body stores, and the dictionary names no such set or its key set lacks that key")]
    External { clip: String, key: u32 },
    #[error("animation {animation:?} is not owned by the animation set whose map holds it")]
    Owner { animation: String },
    #[error("the root states 0 at 0x6C as a lone animation set does, but the objects its map holds are not owned by it, so they are no animations and the root is no animation set; its word at 0x70 is {word:#010X}, a kind no reader here knows")]
    NotAnimationSet { word: u32 },
    #[error("animation {animation:?}: {why}")]
    Blocks { animation: String, why: String },
    #[error("block {block} of animation {animation:?}: its channel directory of 11 sections does not hold to the measured layout: {why}")]
    Directory {
        animation: String,
        block: usize,
        why: String,
    },
    #[error("block {block} of animation {animation:?} has {count} channel(s) in directory section {section}, whose format is not known")]
    Section {
        animation: String,
        block: usize,
        section: usize,
        count: u16,
    },
    #[error("block {block} of animation {animation:?}: DOF {dof}'s channel in section 10 does not decode: {why}")]
    Delta {
        animation: String,
        block: usize,
        dof: u32,
        why: String,
    },
    #[error("motion {motion:?}: {why}")]
    Motion { motion: String, why: String },
    #[error("the tags and properties at {at:#X}: {why}")]
    Metadata { at: usize, why: String },
    #[error("the stored resource's end region {0}")]
    EndRegion(String),
    #[error("animation {animation:?} has {count} DOF(s), so there is no DOF {dof}")]
    NoDof {
        animation: String,
        dof: usize,
        count: usize,
    },
    #[error("decoding a block of animation {animation:?}: {source}")]
    Decode {
        animation: String,
        #[source]
        source: IoError,
    },
    #[error("the body names its lists and blocks more often than twice its {size} bytes could hold them, so it names an object many times")]
    Aliased { size: usize },
    #[error("the expression set {0}")]
    Expressions(String),
    #[error("the edit is not written: {0}")]
    Unwritten(String),
}

/// What the root of a resource is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootKind {
    /// Clips and the animation sets they play: 2 or 3 at 0x6C.
    ClipDictionary,
    /// An animation set standing alone, with no clips: 0 at 0x6C.
    AnimationSet,
    /// Motions that draw on clips of other dictionaries by key: 0 at 0x6C, a motion first in its map.
    MotionDictionary,
}

/// An animation resource: its root, its animation sets and clips, or its motions.
#[derive(Debug, Clone)]
pub struct AnimResource {
    pub root: RootKind,
    /// The root's word at 0x6C: 2 or 3 in a clip dictionary, what 3 adds not known; 0 otherwise.
    pub root_word: u32,
    pub name: String,
    /// The name's hash, stated at 0x68.
    pub key: u32,
    /// The virtual and physical blocks the block map at root 0x08 counts.
    pub block_map: Option<[u8; 2]>,
    /// The root's word at 0x60, whose role is not known.
    pub word_at_60: u64,
    /// The word at 0x98 of a root of 0xB0 bytes, whose role is not known; none for a lone set.
    pub word_at_98: Option<u64>,
    pub sets: Vec<AnimSet>,
    /// The keys of the sets a dictionary's set list names and another body stores.
    pub external_sets: Vec<u32>,
    /// A dictionary's second map: the key of every animation its clips play, external ones included.
    pub animation_keys: Vec<u32>,
    /// In the dictionary's map order; none for another root.
    pub clips: Vec<Clip>,
    /// A motion dictionary's motions, in its map order.
    pub motions: Vec<Motion>,
    /// A motion dictionary's motion matchers, in its map order.
    pub matchers: Vec<Matcher>,
    /// The keys of the clip dictionaries a motion dictionary's motions draw on.
    pub dictionaries: Vec<u32>,
    /// What a stored resource holds after its objects; see [`read_stored`].
    pub end_region: Option<EndRegion>,
    /// The sections this was read from, set by whoever has them; none from [`read`].
    pub sections: Option<std::sync::Arc<crate::container::rpf::Rpf9Sections>>,
    /// Whether [`AnimResource::sections`] hold an edit.
    pub edited: bool,
}

/// The region a stored resource's root 0x08 points at, to the end of its section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndRegion {
    /// The names its string area holds, in order: none, one or two.
    pub names: Vec<String>,
    /// The word before the set list, whose role is not known.
    pub word: u32,
}

/// A named map of animations: `<dictionary>-N`, the dictionary's N-th set.
#[derive(Debug, Clone)]
pub struct AnimSet {
    pub name: String,
    pub key: u32,
    pub animations: Vec<Animation>,
}

/// Where an animation is: its set and its place in that set's map.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnimAt {
    pub set: usize,
    pub animation: usize,
}

#[derive(Debug, Clone)]
pub struct Clip {
    pub name: String,
    /// The map key: the leaf name's hash low; high, the folder's hash where the name has one, else the dictionary's key.
    pub key: u64,
    pub play: ClipPlay,
    pub properties: Vec<Record>,
    pub tags: Vec<Record>,
}

/// What a clip plays.
#[derive(Debug, Clone, PartialEq)]
pub enum ClipPlay {
    /// Type 1: one animation, over a window of it at a rate.
    One {
        animation: AnimAt,
        /// The animation's key, stated at 0x70.
        key: u32,
        start: f32,
        end: f32,
        rate: f32,
    },
    /// Type 2: groups of animations, and a duration no group exceeds.
    Groups { total: f32, groups: Vec<ClipGroup> },
    /// An animation pointer of the clip names no animation of the sets, so what it plays is not known.
    Unresolved,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClipGroup {
    pub duration: f32,
    pub entries: Vec<ClipEntry>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClipEntry {
    /// None for an animation of a set another body stores, found by [`ClipEntry::key`].
    pub animation: Option<AnimAt>,
    pub key: u32,
    /// `(x, y)` points from `(0, 0)` to the group's duration twice, whose use is not known.
    pub warp_points: Vec<[f32; 2]>,
    pub start: f32,
    pub end: f32,
    pub rate: f32,
}

/// An animation: its timing, its DOFs and the blocks that hold their channels.
#[derive(Debug, Clone, Default)]
pub struct Animation {
    pub name: String,
    /// The set map's key: the hash of the name without `pack:/` and `.anim`.
    pub key: u32,
    /// The small count at 0x10, a reference count by its values.
    pub references: u32,
    pub flags: u16,
    pub project_flags: u16,
    /// The word at 0x2C, which many animations share.
    pub signature: u32,
    /// The largest block's byte count plus its 0x20-byte header.
    pub max_block_size: u32,
    pub frames: u16,
    pub frames_per_chunk: u16,
    /// Seconds: `(frames - 1) / rate`.
    pub duration: f32,
    pub dofs: Vec<Dof>,
    pub blocks: Vec<Block>,
}

/// One degree of freedom: a 32-bit id, a track word and a type code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dof {
    /// A 32-bit id, a bone name's hash for some DOFs; what the others name is not known.
    pub id: u32,
    /// The hash of a GTA V track name in camelCase where [`Dof::track_name`] finds one.
    pub track: u32,
    /// 0, 2, 3 or 4, with 1, 4 or 5, then 1 or 3, then 2, then 1 directory entries.
    pub kind: u32,
}

/// The track name whose hash `word` is, among those known.
pub fn track_name(word: u32) -> Option<&'static str> {
    TRACK_NAMES
        .into_iter()
        .find(|name| name_hash32(name.as_bytes()) == word)
}

/// The track name whose hash `word` is, or `hash_XXXXXXXX` of the word when no known name hashes to it.
pub fn track_label(word: u32) -> String {
    track_name(word).map_or_else(|| format!("hash_{word:08X}"), str::to_string)
}

impl Dof {
    /// The track name whose hash the track word is, among those known.
    pub fn track_name(&self) -> Option<&'static str> {
        track_name(self.track)
    }

    /// The track's name, or `hash_XXXXXXXX` of its word when no known name hashes to it.
    pub fn track_label(&self) -> String {
        track_label(self.track)
    }
}

/// One block of an animation's channels.
#[derive(Debug, Clone)]
pub struct Block {
    /// The channels each of the 11 directory sections holds.
    pub sections: [u16; SECTIONS],
    /// GTA V's block with its directory rewritten to sections 0 to 5, 7 and 8.
    stream: CrBlockStream,
    /// The channels of [`DELTA_SECTION`], each decoded at every frame of the block.
    deltas: Vec<Delta>,
}

impl Block {
    /// Whether the block holds a channel of DOF `dof` in [`DELTA_SECTION`].
    pub fn holds_delta(&self, dof: usize) -> bool {
        self.deltas.iter().any(|d| d.dof as usize == dof)
    }
}

#[derive(Debug, Clone)]
struct Delta {
    dof: u32,
    component: u8,
    values: Vec<f32>,
}

/// One DOF's channels of block `index`, split into those before and after the post pass.
struct Walked<'a> {
    index: usize,
    walk: ChannelWalk<'a>,
    pre: Vec<DecodedChannel>,
    post: Vec<DecodedChannel>,
}

impl Animation {
    /// Frames a second, `(frames - 1) / duration`; `None` for a single frame or no duration.
    pub fn rate(&self) -> Option<f32> {
        (self.frames > 1 && self.duration > 0.0).then(|| f32::from(self.frames - 1) / self.duration)
    }

    /// Whether a block holds a channel of DOF `dof` in [`DELTA_SECTION`].
    pub fn in_delta_section(&self, dof: usize) -> bool {
        self.blocks.iter().any(|b| b.holds_delta(dof))
    }

    /// DOF `dof`'s value at `time` seconds: sections 0 to 5, then 10, then 7 and 8 of its block.
    // A section-10 value between two frames is their linear blend, as GTA V's formats blend.
    pub fn sample(&self, dof: usize, time: f32) -> Result<DofSamples, AnimError> {
        self.sample_walked(dof, time, &mut None)
    }

    /// [`Animation::sample`] at each of `times`, a block's channels walked once for the times in it.
    // ⚠ A walk copies its block's constants, so a walk per sample makes a block's frames quadratic.
    pub fn sample_each(
        &self,
        dof: usize,
        times: impl IntoIterator<Item = f32>,
    ) -> Result<Vec<DofSamples>, AnimError> {
        let mut walked = None;
        (times.into_iter())
            .map(|time| self.sample_walked(dof, time, &mut walked))
            .collect()
    }

    /// [`Animation::sample`], with `walked` reused while `time` lies in the block it walked.
    fn sample_walked<'a>(
        &'a self,
        dof: usize,
        time: f32,
        walked: &mut Option<Walked<'a>>,
    ) -> Result<DofSamples, AnimError> {
        if dof >= self.dofs.len() {
            return Err(AnimError::NoDof {
                animation: self.name.clone(),
                dof,
                count: self.dofs.len(),
            });
        }
        let frame = self.time_to_internal_frame(time);
        let index = self.internal_frame_to_block_index(frame);
        let Some(block) = self.blocks.get(index) else {
            return Err(AnimError::Blocks {
                animation: self.name.clone(),
                why: format!("no block {index}"),
            });
        };
        let at = self.internal_frame_to_block_time(frame, index);
        let tq = (at as usize).min(usize::from(block.stream.num_frames).saturating_sub(2));
        let tr = (at - tq as f32).clamp(0.0, 1.0);
        let decode = |source| AnimError::Decode {
            animation: self.name.clone(),
            source,
        };
        if walked.as_ref().is_none_or(|w| w.index != index) {
            let walk = ChannelWalk::new(&block.stream).map_err(decode)?;
            let (post, pre): (Vec<_>, Vec<_>) = walk
                .channels_for_dof(dof as u32)
                .map_err(decode)?
                .into_iter()
                .partition(|c| c.format.is_post_pass());
            *walked = Some(Walked {
                index,
                walk,
                pre,
                post,
            });
        }
        let Some(Walked {
            walk, pre, post, ..
        }) = walked.as_ref()
        else {
            return Err(AnimError::Blocks {
                animation: self.name.clone(),
                why: format!("block {index} was not walked"),
            });
        };
        let mut samples = walk.evaluate_channels(pre, tq, tr, 1).map_err(decode)?;
        for delta in block.deltas.iter().filter(|d| d.dof as usize == dof) {
            let value = |f: usize| delta.values[f.min(delta.values.len() - 1)];
            let index = usize::from(delta.component).min(3);
            samples.components[index] = value(tq) + (value(tq + 1) - value(tq)) * tr;
            samples.count = samples.count.max(index + 1);
        }
        for channel in post {
            let [a, b, c, d] = samples.components;
            samples.components = if channel.format == ChannelFormat::Reconstruct {
                let w = reconstruct_component(a, b, c);
                match channel.component {
                    0 => [w, a, b, c],
                    1 => [a, w, b, c],
                    2 => [a, b, w, c],
                    _ => [a, b, c, w],
                }
            } else {
                let length = (a * a + b * b + c * c + d * d).sqrt();
                if length > f32::MIN_POSITIVE {
                    [a / length, b / length, c / length, d / length]
                } else {
                    [a, b, c, d]
                }
            };
            samples.count = 4;
        }
        Ok(samples)
    }
}

impl Animation {
    /// [`Animation::sample`] of many DOFs at a time, as a pose asks: a block walked once for them all.
    pub fn sampler(&self) -> Sampler<'_> {
        Sampler {
            animation: self,
            blocks: self.blocks.iter().map(|_| None).collect(),
            words: None,
        }
    }
}

/// What [`Animation::sampler`] returns, a block's channels walked once and the two frames a time
/// lies between read once for every DOF sampled at it.
pub struct Sampler<'a> {
    animation: &'a Animation,
    /// Each block sampled: its walk, and its channels by DOF in the walk's order.
    blocks: Vec<Option<(ChannelWalk<'a>, Vec<DecodedChannel>)>>,
    words: Option<FrameWords>,
}

/// The block and frame last sampled, and that frame's and the next one's words.
struct FrameWords {
    at: (usize, usize),
    prev: Vec<u32>,
    next: Vec<u32>,
}

impl<'a> Sampler<'a> {
    pub fn sample(&mut self, dof: usize, time: f32) -> Result<DofSamples, AnimError> {
        let animation: &'a Animation = self.animation;
        if dof >= animation.dofs.len() {
            return Err(AnimError::NoDof {
                animation: animation.name.clone(),
                dof,
                count: animation.dofs.len(),
            });
        }
        let frame = animation.time_to_internal_frame(time);
        let index = animation.internal_frame_to_block_index(frame);
        let Some(block) = animation.blocks.get(index) else {
            return Err(AnimError::Blocks {
                animation: animation.name.clone(),
                why: format!("no block {index}"),
            });
        };
        let at = animation.internal_frame_to_block_time(frame, index);
        let tq = (at as usize).min(usize::from(block.stream.num_frames).saturating_sub(2));
        let tr = (at - tq as f32).clamp(0.0, 1.0);
        let decode = |source| AnimError::Decode {
            animation: animation.name.clone(),
            source,
        };
        let (walk, channels) = match &mut self.blocks[index] {
            Some(walked) => walked,
            slot => {
                let walk = ChannelWalk::new(&block.stream).map_err(decode)?;
                let mut channels = walk.channels().map_err(decode)?;
                channels.sort_by_key(|c| c.dof_index);
                slot.insert((walk, channels))
            }
        };
        let from = channels.partition_point(|c| (c.dof_index as usize) < dof);
        let (post, pre): (Vec<_>, Vec<_>) = (channels[from..].iter().copied())
            .take_while(|c| c.dof_index as usize == dof)
            .partition(|c| c.format.is_post_pass());
        let words = match &mut self.words {
            Some(words) if words.at == (index, tq) => words,
            slot => slot.insert(FrameWords {
                at: (index, tq),
                prev: walk.frame_words(tq).map_err(decode)?,
                next: walk.frame_words(tq + 1).map_err(decode)?,
            }),
        };
        let mut samples = walk
            .evaluate_between(&pre, &words.prev, &words.next, tq, tr, 1)
            .map_err(decode)?;
        for delta in block.deltas.iter().filter(|d| d.dof as usize == dof) {
            let value = |f: usize| delta.values[f.min(delta.values.len() - 1)];
            let index = usize::from(delta.component).min(3);
            samples.components[index] = value(tq) + (value(tq + 1) - value(tq)) * tr;
            samples.count = samples.count.max(index + 1);
        }
        for channel in post {
            let [a, b, c, d] = samples.components;
            samples.components = if channel.format == ChannelFormat::Reconstruct {
                let w = reconstruct_component(a, b, c);
                match channel.component {
                    0 => [w, a, b, c],
                    1 => [a, w, b, c],
                    2 => [a, b, w, c],
                    _ => [a, b, c, w],
                }
            } else {
                let length = (a * a + b * b + c * c + d * d).sqrt();
                if length > f32::MIN_POSITIVE {
                    [a / length, b / length, c / length, d / length]
                } else {
                    [a, b, c, d]
                }
            };
            samples.count = 4;
        }
        Ok(samples)
    }
}

impl AnimationSource for Animation {
    type Block = CrBlockStream;

    fn name(&self) -> &str {
        &self.name
    }
    fn num_frames(&self) -> u16 {
        self.frames
    }
    fn frames_per_chunk(&self) -> u16 {
        self.frames_per_chunk
    }
    fn duration(&self) -> f32 {
        self.duration
    }
    fn dof_count(&self) -> usize {
        self.dofs.len()
    }
    /// None: GTA VI's type codes are not GTA V's format types.
    fn dof_format(&self, _: usize) -> Option<u8> {
        None
    }
    /// None: a GTA VI DOF's id is a 32-bit hash and its track a hash, not a tag and a track code.
    fn find_dof(&self, _: u16, _: u8) -> Option<usize> {
        None
    }
    fn block_count(&self) -> usize {
        self.blocks.len()
    }
    fn block(&self, index: usize) -> Option<&CrBlockStream> {
        self.blocks.get(index).map(|b| &b.stream)
    }
}

/// The resource whose virtual section, addressed from 0x5000_0000, is `section`, its root at its start.
pub fn read(section: &[u8]) -> Result<AnimResource, AnimError> {
    let aligned = Aligned::from_bytes(section);
    let s = Section {
        image: Image::new(aligned.bytes(), section.len()),
        left: Cell::new(section.len().saturating_mul(2)),
    };
    s.root()
}

/// [`read`] for a resource stored whole in an RSC8, where a clip dictionary also states the region
/// its root 0x08 points at: a `{0, 1, 0}` record, names, a word, the set list, and a pointer, to
/// the section's end.
pub fn read_stored(section: &[u8]) -> Result<AnimResource, AnimError> {
    let mut resource = read(section)?;
    if resource.root != RootKind::ClipDictionary {
        return Ok(resource);
    }
    let aligned = Aligned::from_bytes(section);
    let s = Section {
        image: Image::new(aligned.bytes(), section.len()),
        left: Cell::new(section.len()),
    };
    resource.end_region = Some(s.end_region()?);
    Ok(resource)
}

/// The virtual section, every read bounded by its end.
struct Section<'a> {
    image: Image<'a>,
    /// Bytes of lists, blocks and records still to read: distinct objects fit the section once,
    /// and records may share their attribute arrays, so a body that names one object many times
    /// cannot multiply the work past twice its own size.
    left: Cell<usize>,
}

impl Section<'_> {
    fn root(&self) -> Result<AnimResource, AnimError> {
        let head = self.bytes("root", 0, SET_LEN)?;
        if head[..8].iter().chain(&head[0x10..0x38]).any(|b| *b != 0) {
            return Err(AnimError::RootNotZero);
        }
        let root_word = self.u32("root", 0x6C)?;
        let root = match root_word {
            2 | 3 => RootKind::ClipDictionary,
            0 if self.is_motion_dictionary()? => RootKind::MotionDictionary,
            0 => RootKind::AnimationSet,
            other => return Err(AnimError::RootKind(other)),
        };
        if root != RootKind::AnimationSet {
            self.bytes("root", 0, DICTIONARY_LEN)?;
        }
        let name = self.name("root name", 0x38)?;
        let key = self.u32("root", 0x68)?;
        check_key("root", &name, key)?;
        let mut resource = AnimResource {
            root,
            root_word,
            name,
            key,
            block_map: self.block_map()?,
            word_at_60: self.u64("root", 0x60)?,
            word_at_98: match root {
                RootKind::AnimationSet => None,
                _ => Some(self.u64("root", 0x98)?),
            },
            sets: Vec::new(),
            external_sets: Vec::new(),
            animation_keys: Vec::new(),
            clips: Vec::new(),
            motions: Vec::new(),
            matchers: Vec::new(),
            dictionaries: Vec::new(),
            end_region: None,
            sections: None,
            edited: false,
        };
        match root {
            RootKind::ClipDictionary => self.dictionary(&mut resource)?,
            RootKind::MotionDictionary => self.motion_dictionary(&mut resource)?,
            RootKind::AnimationSet => {
                // A root whose map objects its owner word does not name is another kind of root.
                let set = self.set(0, 0, &mut HashMap::new());
                let set = set.map_err(|e| match (e, self.u32("root", SET_LEN)) {
                    (AnimError::Owner { .. }, Ok(word)) => AnimError::NotAnimationSet { word },
                    (e, _) => e,
                })?;
                resource.sets.push(set);
            }
        }
        Ok(resource)
    }

    /// The virtual and physical block counts of the block map at root 0x08, `{u64 0, u8, u8, ...}`.
    fn block_map(&self) -> Result<Option<[u8; 2]>, AnimError> {
        let Some(at) = self.pointer("block map", 0x08)? else {
            return Ok(None);
        };
        let map = self.bytes("block map", at, 10)?;
        Ok(Some([map[8], map[9]]))
    }

    /// The metadata region root 0xA0 states: a pointer and a byte count.
    fn region(&self) -> Result<metadata::Region, AnimError> {
        let lo = self.target("metadata region", 0xA0)?;
        let len = self.u32("metadata region", 0xA8)? as usize;
        self.bytes("metadata region", lo, len)?;
        Ok(metadata::Region { lo, hi: lo + len })
    }

    fn dictionary(&self, r: &mut AnimResource) -> Result<(), AnimError> {
        self.bytes("root", 0, DICTIONARY_LEN)?;
        let mut found = HashMap::new();
        let list = self.array("animation set list", 0x70, 16)?;
        for (index, at) in list.into_iter().enumerate() {
            let key = self.u32("animation set list", at + 8)?;
            let word = self.u32("animation set list", at + 12)?;
            match self.pointer("animation set", at)? {
                None if word == 0 => r.external_sets.push(key),
                None => {
                    return Err(AnimError::SetList {
                        index,
                        why: format!("names no set and holds {word} at +12, where an external set's entry holds 0"),
                    })
                }
                Some(set) => {
                    let set = self.set(set, r.sets.len(), &mut found)?;
                    if set.key != key || word != 1 {
                        return Err(AnimError::SetList {
                            index,
                            why: format!(
                                "states key {key:#010X} and word {word} for set {:?} of key {:#010X}, where an entry states its set's key and 1",
                                set.name, set.key
                            ),
                        });
                    }
                    r.sets.push(set);
                }
            }
        }
        if r.sets.is_empty() {
            return Err(AnimError::NoSet);
        }
        r.animation_keys = self
            .map("animation key", 0x80, 8, false)?
            .into_iter()
            .map(|(key, _)| key as u32)
            .collect();
        let keys: HashSet<u32> = r.animation_keys.iter().copied().collect();
        for animation in r.sets.iter().flat_map(|s| &s.animations) {
            if !keys.contains(&animation.key) {
                return Err(AnimError::Map {
                    what: "animation key",
                    at: 0x80,
                    why: format!(
                        "does not hold key {:#010X} of animation {:?}",
                        animation.key, animation.name
                    ),
                });
            }
        }
        let external = (!r.external_sets.is_empty()).then_some(&keys);
        let region = self.region()?;
        for (key, entry) in self.map("clip", 0x48, 0x10, true)? {
            let clip = self.target("clip", entry + 8)?;
            r.clips
                .push(self.clip(clip, key, r.key, &found, external, region)?);
        }
        Ok(())
    }

    /// The region root 0x08 points at in a stored resource, laid out back from the set list.
    fn end_region(&self) -> Result<EndRegion, AnimError> {
        let bad = |why: String| AnimError::EndRegion(why);
        let start = self.target("end region", 0x08)?;
        let record = [
            self.u64("end region", start)?,
            self.u64("end region", start + 8)?,
            self.u64("end region", start + 16)?,
        ];
        if record != [0, 1, 0] {
            return Err(bad(format!(
                "at {start:#X} does not open with the record {{0, 1, 0}}"
            )));
        }
        let list = self.target("animation set list", 0x70)?;
        let count = usize::from(self.u16("animation set list", 0x78)?);
        let word_at = list
            .checked_sub(16)
            .filter(|at| *at >= start + 0x18 && at % 16 == 0)
            .ok_or_else(|| {
                bad(format!(
                    "at {start:#X} leaves no room before the set list at {list:#X}"
                ))
            })?;
        let word = self.bytes("end region", word_at, 16)?;
        if word[4..].iter().any(|b| *b != 0) {
            return Err(bad(format!(
                "holds non-zero bytes after its word at {word_at:#X}"
            )));
        }
        // The names lie before the word, or in one variant after the set list.
        let set_end = list + 16 * count;
        let last = self.image.len().saturating_sub(16).max(set_end);
        let earlier = self
            .pointer("end region", last)?
            .is_some_and(|to| to % 16 == 0 && to < start);
        if !earlier || self.u64("end region", last + 8)? != 0 || last + 16 != self.image.len() {
            return Err(bad(format!(
                "does not end with a pointer to an earlier object and a zero at {last:#X}, the section's last 16 bytes"
            )));
        }
        let mut names = Vec::new();
        for (area_at, area_end) in [(start + 0x18, word_at), (set_end, last)] {
            let area = self.bytes("end region", area_at, area_end - area_at)?;
            let mut k = 0;
            while k < area.len() {
                if area[k] == 0 {
                    k += 1;
                    continue;
                }
                let len = area[k..].iter().position(|b| *b == 0).unwrap_or(0);
                let text = &area[k..k + len];
                if len == 0 || (area_at + k) % 8 != 0 || !text.iter().all(|b| (32..127).contains(b))
                {
                    return Err(bad(format!(
                        "holds bytes at {:#X} that are no text ended by a zero and starting on 8 bytes",
                        area_at + k
                    )));
                }
                names.push(String::from_utf8_lossy(text).into_owned());
                k += len;
            }
        }
        Ok(EndRegion {
            names,
            word: u32::from_le_bytes([word[0], word[1], word[2], word[3]]),
        })
    }

    fn spend(&self, len: usize) -> Result<(), AnimError> {
        let left = self.left.get().checked_sub(len).ok_or(AnimError::Aliased {
            size: self.image.len(),
        })?;
        self.left.set(left);
        Ok(())
    }

    fn bytes(&self, what: &'static str, at: usize, len: usize) -> Result<&[u8], AnimError> {
        let size = self.image.len();
        at.checked_add(len)
            .filter(|end| *end <= size)
            .map(|end| &self.image.bytes()[at..end])
            .ok_or(AnimError::PastEnd {
                what,
                at,
                len,
                size,
            })
    }

    fn word<const N: usize>(&self, what: &'static str, at: usize) -> Result<[u8; N], AnimError> {
        let mut word = [0; N];
        word.copy_from_slice(self.bytes(what, at, N)?);
        Ok(word)
    }

    fn u16(&self, what: &'static str, at: usize) -> Result<u16, AnimError> {
        self.word(what, at).map(u16::from_le_bytes)
    }

    fn u32(&self, what: &'static str, at: usize) -> Result<u32, AnimError> {
        self.word(what, at).map(u32::from_le_bytes)
    }

    fn u64(&self, what: &'static str, at: usize) -> Result<u64, AnimError> {
        self.word(what, at).map(u64::from_le_bytes)
    }

    fn f32(&self, what: &'static str, at: usize) -> Result<f32, AnimError> {
        self.word(what, at).map(f32::from_le_bytes)
    }

    /// The offset the pointer at `at` holds; `None` for a null pointer.
    fn pointer(&self, what: &'static str, at: usize) -> Result<Option<usize>, AnimError> {
        let value = self.u64(what, at)?;
        match self.image.resolve(value) {
            Ok(offset) => Ok(Some(offset)),
            Err(crate::desc::Unresolvable::Null) => Ok(None),
            Err(why) => Err(AnimError::Pointer {
                what,
                at,
                value,
                why: why.to_string(),
            }),
        }
    }

    /// The offset the pointer at `at` holds, which must not be null.
    fn target(&self, what: &'static str, at: usize) -> Result<usize, AnimError> {
        self.pointer(what, at)?.ok_or(AnimError::Pointer {
            what,
            at,
            value: 0,
            why: "null".into(),
        })
    }

    /// The elements of the array `{pointer, u16 count, u16 capacity, u32}` at `at`.
    fn array(&self, what: &'static str, at: usize, stride: usize) -> Result<Vec<usize>, AnimError> {
        let count = usize::from(self.u16(what, at + 8)?);
        if count == 0 {
            return Ok(Vec::new());
        }
        let first = self.target(what, at)?;
        self.bytes(what, first, count * stride)?;
        self.spend(count * stride)?;
        Ok((0..count).map(|i| first + i * stride).collect())
    }

    /// The 16-byte name at `at`: inline, its length in byte 15 with the top bit set, or a pointer
    /// and a u16 length.
    fn name(&self, what: &'static str, at: usize) -> Result<String, AnimError> {
        let field = self.bytes(what, at, 16)?;
        let (from, len) = if field[15] & 0x80 != 0 {
            (at, usize::from(field[15] & 0x7F).min(15))
        } else {
            let len = usize::from(self.u16(what, at + 8)?);
            match self.pointer(what, at)? {
                Some(from) => (from, len),
                None => (at, 0),
            }
        };
        Ok(String::from_utf8_lossy(self.bytes(what, from, len)?).into_owned())
    }

    /// The zero-terminated string the pointer at `at` holds; empty for a null pointer.
    fn c_string(&self, what: &'static str, at: usize) -> Result<String, AnimError> {
        let Some(from) = self.pointer(what, at)? else {
            return Ok(String::new());
        };
        let rest = &self.image.bytes()[from..];
        let len = rest
            .iter()
            .position(|b| *b == 0)
            .ok_or(AnimError::PastEnd {
                what,
                at: from,
                len: rest.len() + 1,
                size: self.image.len(),
            })?;
        Ok(String::from_utf8_lossy(&rest[..len]).into_owned())
    }

    /// Every entry of the 0x18-byte map at `at`, bucket by bucket, as its key and its offset; an
    /// entry's key is a u64 when `wide`, else a u32, and its next is at `next_at`.
    fn map(
        &self,
        what: &'static str,
        at: usize,
        next_at: usize,
        wide: bool,
    ) -> Result<Vec<(u64, usize)>, AnimError> {
        let broken = |why: String| AnimError::Map { what, at, why };
        let slots = self.u32(what, at + 8)? as usize;
        let used = self.u32(what, at + 0x10)? as usize;
        if used > self.image.len() / MAP_ENTRY_MIN {
            return Err(broken(format!(
                "states {used} entries, more than the section can hold"
            )));
        }
        self.spend(used * MAP_ENTRY_MIN)?;
        let mut entries = Vec::with_capacity(used);
        if let Some(buckets) = self.pointer(what, at)? {
            self.bytes(what, buckets, slots.saturating_mul(8))?;
            for bucket in 0..slots {
                let mut next = self.pointer(what, buckets + bucket * 8)?;
                while let Some(entry) = next {
                    if entries.len() == used {
                        return Err(broken(format!(
                            "states {used} entries and its chains hold more"
                        )));
                    }
                    let key = if wide {
                        self.u64(what, entry)?
                    } else {
                        u64::from(self.u32(what, entry)?)
                    };
                    if key % slots as u64 != bucket as u64 {
                        return Err(broken(format!(
                            "holds key {key:#X} in bucket {bucket} of {slots}, where a lookup does not walk"
                        )));
                    }
                    entries.push((key, entry));
                    next = self.pointer(what, entry + next_at)?;
                }
            }
        }
        if entries.len() != used {
            return Err(broken(format!(
                "states {used} entries and its buckets hold {}",
                entries.len()
            )));
        }
        Ok(entries)
    }

    /// The animation set at `at`, the `index`-th, recording where each animation is in `found`.
    fn set(
        &self,
        at: usize,
        index: usize,
        found: &mut HashMap<usize, AnimAt>,
    ) -> Result<AnimSet, AnimError> {
        self.bytes("animation set", at, SET_LEN)?;
        let name = self.name("animation set name", at + 0x38)?;
        let key = self.u32("animation set", at + 0x68)?;
        check_key("animation set", &name, key)?;
        let mut animations = Vec::new();
        for (key, entry) in self.map("animation", at + 0x48, 0x10, false)? {
            let animation = self.target("animation", entry + 8)?;
            found.insert(
                animation,
                AnimAt {
                    set: index,
                    animation: animations.len(),
                },
            );
            animations.push(self.animation(animation, at, key as u32)?);
        }
        Ok(AnimSet {
            name,
            key,
            animations,
        })
    }

    fn animation(&self, at: usize, owner: usize, key: u32) -> Result<Animation, AnimError> {
        self.bytes("animation", at, ANIMATION_LEN)?;
        let mut animation = Animation {
            name: self.c_string("animation name", at + 0x30)?,
            key,
            references: self.u32("animation", at + 0x10)?,
            flags: self.u16("animation", at + 0x20)?,
            project_flags: self.u16("animation", at + 0x22)?,
            signature: self.u32("animation", at + 0x2C)?,
            max_block_size: self.u32("animation", at + 0x48)?,
            frames: self.u16("animation", at + 0x24)?,
            frames_per_chunk: self.u16("animation", at + 0x26)?,
            duration: self.f32("animation", at + 0x28)?,
            dofs: Vec::new(),
            blocks: Vec::new(),
        };
        if self.pointer("animation owner", at + 0x18)? != Some(owner) {
            return Err(AnimError::Owner {
                animation: animation.name,
            });
        }
        for dof in self.array("DOF list", at + 0x60, DOF_LEN)? {
            animation.dofs.push(Dof {
                id: self.u32("DOF", dof)?,
                track: self.u32("DOF", dof + 4)?,
                kind: self.u32("DOF", dof + 8)?,
            });
        }
        let pointers = self.array("block list", at + 0x50, 8)?;
        let expected = animation.calc_num_blocks();
        if pointers.len() != expected {
            return Err(AnimError::Blocks {
                animation: animation.name,
                why: format!(
                    "{} blocks, where {} frames of {} a block make {expected}",
                    pointers.len(),
                    animation.frames,
                    animation.frames_per_chunk
                ),
            });
        }
        for (index, pointer) in pointers.into_iter().enumerate() {
            let stream = self.block_stream(self.target("block", pointer)?)?;
            let frames = animation.frames_in_block(index);
            if usize::from(stream.num_frames) != frames {
                return Err(AnimError::Blocks {
                    animation: animation.name,
                    why: format!(
                        "block {index} holds {} frames, where the animation's chunks give it {frames}",
                        stream.num_frames
                    ),
                });
            }
            let block =
                rewrite(stream, &animation.dofs).map_err(|e| e.at(&animation.name, index))?;
            animation.blocks.push(block);
        }
        Ok(animation)
    }

    /// GTA V's block stream at `at`, as its declaration reads it.
    fn block_stream(&self, at: usize) -> Result<CrBlockStream, AnimError> {
        let bad = |e: crate::decl::ViewError| AnimError::Pointer {
            what: "block",
            at,
            value: at as u64,
            why: e.to_string(),
        };
        let view = View::<Raw<CrBlockStream>>::at_offset(self.image, at).map_err(bad)?;
        let data = view.data().map_err(bad)?;
        self.spend(data.as_slice().len())?;
        let data = data.as_slice().to_vec();
        Ok(CrBlockStream {
            hash: view.hash,
            compact_size: view.compact_size,
            constant_size: view.constant_size,
            channel_offset: view.channel_offset,
            slop_size: view.slop_size,
            num_frames: view.num_frames,
            frame_size: view.frame_size,
            indirect_size: view.indirect_size,
            quantize_size: view.quantize_size,
            segment_size: view.segment_size,
            num_mover_channels: view.num_mover_channels,
            data,
            ..CrBlockStream::default()
        })
    }

    /// The clip at `at`; `external` holds the key set when the dictionary names external sets.
    fn clip(
        &self,
        at: usize,
        key: u64,
        root_key: u32,
        found: &HashMap<usize, AnimAt>,
        external: Option<&HashSet<u32>>,
        region: metadata::Region,
    ) -> Result<Clip, AnimError> {
        self.bytes("clip", at, CLIP_GROUPS_LEN)?;
        let name = self.name("clip name", at + 0x28)?;
        let base = name.strip_prefix("pack:/").unwrap_or(&name);
        let base = base.strip_suffix(".clip").unwrap_or(base);
        let (folder, leaf) = base.rsplit_once('/').unwrap_or(("", base));
        let high = if folder.is_empty() {
            root_key
        } else {
            name_hash32(folder.as_bytes())
        };
        if key != u64::from(high) << 32 | u64::from(name_hash32(leaf.as_bytes()))
            || self.u64("clip", at + 0x58)? != key
        {
            return Err(AnimError::ClipKey { clip: name });
        }
        let resolve = |pointer: usize| -> Result<Option<AnimAt>, AnimError> {
            Ok(self
                .pointer("clip animation", pointer)?
                .and_then(|a| found.get(&a).copied()))
        };
        let play = match self.u32("clip", at + 0x20)? {
            1 => {
                self.bytes("clip", at, CLIP_ONE_LEN)?;
                match resolve(at + 0x68)? {
                    Some(animation) => ClipPlay::One {
                        animation,
                        key: self.u32("clip", at + 0x70)?,
                        start: self.f32("clip", at + 0x90)?,
                        end: self.f32("clip", at + 0x94)?,
                        rate: self.f32("clip", at + 0x98)?,
                    },
                    None => ClipPlay::Unresolved,
                }
            }
            2 => {
                let mut groups = Vec::new();
                let mut resolved = true;
                for group in self.array("clip group list", at + 0x68, GROUP_LEN)? {
                    let mut entries = Vec::new();
                    for entry in self.array("clip entry list", group, ENTRY_LEN)? {
                        let key = self.u32("clip entry", entry + 8)?;
                        let animation = match self.pointer("clip animation", entry)? {
                            None if external.is_some_and(|keys| keys.contains(&key)) => None,
                            None => {
                                return Err(AnimError::External {
                                    clip: name.clone(),
                                    key,
                                })
                            }
                            Some(_) => match resolve(entry)? {
                                Some(animation) => Some(animation),
                                None => {
                                    resolved = false;
                                    break;
                                }
                            },
                        };
                        let mut warp_points = Vec::new();
                        for point in self.array("warp point list", entry + 0x10, 16)? {
                            warp_points.push([
                                self.f32("warp point", point)?,
                                self.f32("warp point", point + 4)?,
                            ]);
                        }
                        entries.push(ClipEntry {
                            animation,
                            key,
                            warp_points,
                            start: self.f32("clip entry", entry + 0x28)?,
                            end: self.f32("clip entry", entry + 0x2C)?,
                            rate: self.f32("clip entry", entry + 0x30)?,
                        });
                    }
                    groups.push(ClipGroup {
                        duration: self.f32("clip group", group + 0x10)?,
                        entries,
                    });
                }
                if resolved {
                    ClipPlay::Groups {
                        total: self.f32("clip", at + 0x78)?,
                        groups,
                    }
                } else {
                    ClipPlay::Unresolved
                }
            }
            value => return Err(AnimError::ClipType { clip: name, value }),
        };
        let (properties, tags) = metadata::of_clip(self, region, at)?;
        Ok(Clip {
            name,
            key,
            play,
            properties,
            tags,
        })
    }
}

/// Refuses `key` unless it is the hash of `name`; an empty name states no key to check.
fn check_key(what: &'static str, name: &str, key: u32) -> Result<(), AnimError> {
    if name.is_empty() || name_hash32(name.as_bytes()) == key {
        return Ok(());
    }
    Err(AnimError::Key {
        what,
        name: name.to_string(),
        key,
    })
}

enum Bad {
    Layout(String),
    Section(usize, u16),
    Delta(u32, String),
}

/// The block with its 11-section directory rewritten to GTA V's 9 slots and its
/// [`DELTA_SECTION`] channels decoded.
///
/// ⚠ Sections 7, 8 and 10 consume no frame bits and sections 7 and 8 no constants, and section
/// 10's constants come last, so leaving it out moves no other channel's constants or bits.
fn rewrite(mut stream: CrBlockStream, dofs: &[Dof]) -> Result<Block, Bad> {
    let bad = |why: String| Bad::Layout(why);
    let start = stream.channel_directory_offset();
    let Directory {
        sections,
        runs: indices,
        ..
    } = directory(&stream, dofs.len())?;
    let limit = dofs.len() * 4;
    for section in UNKNOWN_SECTIONS {
        if sections[section] != 0 {
            return Err(Bad::Section(section, sections[section]));
        }
    }
    let mut entries = vec![0usize; dofs.len()];
    for &index in indices.iter().flatten() {
        entries[usize::from(index) >> 2] += 1;
    }
    for (i, (dof, &count)) in dofs.iter().zip(&entries).enumerate() {
        let measured = match dof.kind {
            0 => [1, 4, 5].contains(&count),
            2 => [1, 3].contains(&count),
            3 => count == 2,
            4 => count == 1,
            _ => false,
        };
        if !measured {
            return Err(bad(format!(
                "DOF {i} has type {} and {count} entries, which is not a measured pairing",
                dof.kind
            )));
        }
    }
    let deltas = deltas(&stream, &indices)?;
    let slot_of = |section: usize| (section < 6 || section == 7 || section == 8).then_some(section);
    let mut counts = [0u16; FORMAT_COUNT];
    let mut runs: Vec<&[u16]> = vec![&[]; FORMAT_COUNT];
    for (section, run) in indices.iter().enumerate() {
        if let Some(slot) = slot_of(section) {
            counts[slot] = sections[section];
            runs[slot] = run.as_slice();
        }
    }
    let mut data = stream.data[..start].to_vec();
    data.extend(counts.iter().flat_map(|c| c.to_le_bytes()));
    let pad = u16::try_from(limit).unwrap_or(u16::MAX);
    for run in runs {
        let padded = (run.len() + 3) & !3;
        let values = run.iter().copied().chain(std::iter::repeat(pad));
        data.extend(values.take(padded).flat_map(u16::to_le_bytes));
    }
    stream.data = data;
    Ok(Block {
        sections,
        stream,
        deltas,
    })
}

/// A block's 11 directory counts, each section's entries, and the payload byte its directory ends at.
struct Directory {
    sections: [u16; SECTIONS],
    runs: Vec<Vec<u16>>,
    end: usize,
}

fn directory(stream: &CrBlockStream, dofs: usize) -> Result<Directory, Bad> {
    let bad = |why: String| Bad::Layout(why);
    if stream.compact_size != 0 {
        return Err(bad(format!(
            "it is compact ({} bytes), so its payload is compressed",
            stream.compact_size
        )));
    }
    let start = stream.channel_directory_offset();
    let words: Vec<u16> = stream
        .data
        .get(start..)
        .ok_or_else(|| {
            bad(format!(
                "its frames end at {start}, past its {} bytes",
                stream.data.len()
            ))
        })?
        .as_chunks::<2>()
        .0
        .iter()
        .map(|w| u16::from_le_bytes(*w))
        .collect();
    let mut sections = [0u16; SECTIONS];
    sections.copy_from_slice(
        words
            .get(..SECTIONS)
            .ok_or_else(|| bad("its directory is shorter than its 11 counts".into()))?,
    );
    let limit = dofs * 4;
    let mut runs = Vec::with_capacity(SECTIONS);
    let mut cursor = SECTIONS;
    for (section, &count) in sections.iter().enumerate() {
        let count = usize::from(count);
        let run = words.get(cursor..cursor + count).ok_or_else(|| {
            bad(format!(
                "section {section}'s {count} entries run past the payload"
            ))
        })?;
        if run.windows(2).any(|w| w[0] >= w[1]) || run.iter().any(|&i| usize::from(i) >= limit) {
            return Err(bad(format!(
                "section {section}'s entries do not rise below 4 times its {dofs} DOFs"
            )));
        }
        runs.push(run.to_vec());
        cursor += (count + 3) & !3;
    }
    Ok(Directory {
        sections,
        runs,
        end: start + 2 * cursor,
    })
}

/// A block's constant words.
fn constants(stream: &CrBlockStream) -> Result<Vec<u32>, Bad> {
    Ok(stream
        .data
        .get(..stream.constant_size as usize)
        .ok_or_else(|| {
            Bad::Layout(format!(
                "its {} constant bytes run past its payload",
                stream.constant_size
            ))
        })?
        .as_chunks::<4>()
        .0
        .iter()
        .map(|w| u32::from_le_bytes(*w))
        .collect())
}

/// Each [`DELTA_SECTION`] channel decoded.
fn deltas(stream: &CrBlockStream, indices: &[Vec<u16>]) -> Result<Vec<Delta>, Bad> {
    let constants = constants(stream)?;
    let mut deltas = Vec::with_capacity(indices[DELTA_SECTION].len());
    for (index, words) in delta_ranges(&constants, indices)? {
        let dof = u32::from(index) >> 2;
        let values = section10::decode(&constants[words], usize::from(stream.num_frames))
            .map_err(|why| Bad::Delta(dof, why))?;
        deltas.push(Delta {
            dof,
            component: (index & 3) as u8,
            values,
        });
    }
    Ok(deltas)
}

/// Each [`DELTA_SECTION`] channel's directory entry and the constant words it spans, found by
/// walking sections 0 to 5 first.
///
/// ⚠ The walk must consume the constants exactly: a remainder means a section was misread.
fn delta_ranges(
    constants: &[u32],
    indices: &[Vec<u16>],
) -> Result<Vec<(u16, std::ops::Range<usize>)>, Bad> {
    let bad = |why: String| Bad::Layout(why);
    let word = |at: usize| {
        constants.get(at).copied().ok_or_else(|| {
            bad(format!(
                "its channels read constant word {at} of {}",
                constants.len()
            ))
        })
    };
    let mut at = 0usize;
    for (section, run) in indices.iter().enumerate().take(6) {
        for _ in run.iter() {
            let used = match section {
                0 | 1 | 4 => 3,
                2 => 1,
                3 => 0,
                _ => 5 + word(at + 2)? as usize,
            };
            at = at.saturating_add(used);
        }
    }
    let mut ranges = Vec::with_capacity(indices[DELTA_SECTION].len());
    for &index in &indices[DELTA_SECTION] {
        let dof = u32::from(index) >> 2;
        let stride = word(at)? as usize;
        let range = at..at.saturating_add(stride);
        if constants.get(range.clone()).is_none() || stride < 4 {
            return Err(Bad::Delta(
                dof,
                format!("its {stride} words run past the constants or leave no room for its four header words"),
            ));
        }
        ranges.push((index, range));
        at += stride;
    }
    if at != constants.len() {
        return Err(bad(format!(
            "its channels consume {at} constant words of {}",
            constants.len()
        )));
    }
    Ok(ranges)
}

#[cfg(test)]
#[path = "anim_tests.rs"]
mod tests;
