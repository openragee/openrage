//! An animation resource's virtual section written again: every section-10 channel encoded anew
//! from its steps, the block fields that follow from the constants recomputed, and an edit of a
//! key, a clip's window, a group entry's window or a clip's name made, a DOF held over the whole
//! clip, or a DOF added or removed.
//!
//! ⚠ A block's hash at 0x00 follows from no rule anyone found, so an edited block carries the hash
//! it was read with; whether the game checks it is not known. An edit of a DOF list writes the
//! signature at 0x2C that [`super::dof_signature`] computes from the list; any other keeps it.

mod channels;
mod clips;
mod mapped;
mod relayout;
mod stored;

use std::collections::BTreeSet;
use std::sync::Arc;

use super::{
    constants, delta_ranges, directory, read, read_stored, section10, AnimAt, AnimError,
    AnimResource, Animation, Bad, ClipPlay, Dof, RootKind, Section, DELTA_SECTION,
};
use crate::codec::anim::AnimationSource;
use crate::container::rpf::{Rpf9Form, Rpf9Sections};
use crate::decl::{Aligned, Image};
use crate::gta5::CrBlockStream;

/// Where the virtual section is addressed from.
const VIRTUAL_BASE: u64 = 0x5000_0000;
const BLOCK_HEAD: usize = 0x20;
const MOVER_ENTRY: usize = 6;
/// A block placed past the section's end starts on this many bytes, as every block read does.
const BLOCK_ALIGN: usize = 16;

/// One edit of an animation resource.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnimEdit {
    /// Component `component` of DOF `dof` at `frame`, in every block holding it: in section 10, or
    /// in place in sections 0 to 4.
    Key {
        animation: AnimAt,
        dof: usize,
        component: u8,
        frame: u16,
        value: f32,
    },
    /// The window and rate of a clip of one animation, by its place in [`AnimResource::clips`].
    ClipWindow {
        clip: usize,
        start: f32,
        end: f32,
        rate: f32,
    },
    /// A group entry's window; its rate and a three-point warp list follow from it, the list at
    /// `frame_rate` where another body holds the entry's animation; another list is kept as read.
    EntryWindow {
        clip: usize,
        group: usize,
        entry: usize,
        start: f32,
        end: f32,
        frame_rate: Option<f32>,
    },
    /// DOF `dof` held at `value` over the whole clip, one constant channel in every block.
    Pose {
        animation: AnimAt,
        dof: usize,
        value: [f32; 4],
    },
    /// `dof` added in its place in the DOF list, sorted by track and id, held at `value`.
    AddDof {
        animation: AnimAt,
        dof: Dof,
        value: [f32; 4],
    },
    /// DOF `dof` removed with its channels.
    RemoveDof { animation: AnimAt, dof: usize },
}

/// The key edit a block makes: the step at `frame` of the channel of directory entry `index`.
#[derive(Debug, Clone, Copy)]
struct KeyEdit {
    index: u16,
    frame: usize,
    value: f32,
}

impl Animation {
    /// Component `component` of DOF `dof` at `frame`, where a section-10 channel holds it.
    pub fn delta_value(&self, dof: usize, component: u8, frame: u16) -> Option<f32> {
        let frame = usize::from(frame);
        let block = (frame / usize::from(self.frames_per_chunk.max(1)))
            .min(self.blocks.len().checked_sub(1)?);
        let at = frame - block * usize::from(self.frames_per_chunk);
        let delta = (self.blocks[block].deltas.iter())
            .find(|d| d.dof as usize == dof && d.component == component)?;
        delta.values.get(at).copied()
    }

    /// The `(DOF, component)` pairs a section-10 channel holds in some block, in order.
    pub fn delta_components(&self) -> Vec<(usize, u8)> {
        let pairs: BTreeSet<(usize, u8)> = (self.blocks.iter())
            .flat_map(|b| b.deltas.iter().map(|d| (d.dof as usize, d.component)))
            .collect();
        pairs.into_iter().collect()
    }
}

/// `section` with every block written through the channel encoder; one the reader takes comes back unchanged.
pub fn reencode(section: &[u8], stored: bool) -> Result<Vec<u8>, AnimError> {
    let resource = if stored {
        read_stored(section)?
    } else {
        read(section)?
    };
    let aligned = Aligned::from_bytes(section);
    let s = section_of(&aligned);
    let mut out = section.to_vec();
    let held = stored.then_some(&resource);
    for (set, set_at) in s.set_offsets(&resource)?.into_iter().enumerate() {
        for (a, at) in s.animation_offsets(set_at)?.into_iter().enumerate() {
            let animation = &resource.sets[set].animations[a];
            let blocks = (0..animation.blocks.len()).map(|b| (b, None)).collect();
            write_blocks(&s, &mut out, animation, at, blocks, held)?;
        }
    }
    Ok(out)
}

/// `resource` with clip `clip` named `name`, read again and marked edited.
pub fn rename_clip(
    resource: &AnimResource,
    clip: usize,
    name: &str,
) -> Result<AnimResource, AnimError> {
    let sections = (resource.sections.as_deref()).ok_or_else(|| {
        AnimError::Unwritten("the resource was read with no sections to write".into())
    })?;
    let stored = matches!(sections.form, Rpf9Form::StoredRsc8 { .. });
    let section = sections.virtual_section.as_slice();
    let aligned = Aligned::from_bytes(section);
    let s = section_of(&aligned);
    let no_clip = || AnimError::Unwritten("the resource has no such clip".into());
    let at = *(s.clip_offsets(resource)?.get(clip)).ok_or_else(no_clip)?;
    let mut out = section.to_vec();
    clips::make_room(&s, resource, sections, &mut out, clip, at, name)?;
    let placed = Aligned::from_bytes(&out);
    let s = section_of(&placed);
    let at = *(s.clip_offsets(resource)?.get(clip)).ok_or_else(no_clip)?;
    clips::rename(&s, &mut out, resource, clip, at, name)?;
    let mut edited = if stored {
        read_stored(&out)?
    } else {
        read(&out)?
    };
    edited.sections = Some(Arc::new(Rpf9Sections {
        form: sections.form,
        virtual_section: out,
        physical: sections.physical.clone(),
    }));
    edited.edited = true;
    Ok(edited)
}

/// `resource` with `edit` made in its sections and in what it states, marked edited.
pub fn edit(resource: &AnimResource, edit: &AnimEdit) -> Result<AnimResource, AnimError> {
    let sections = (resource.sections.as_deref()).ok_or_else(|| {
        AnimError::Unwritten("the resource was read with no sections to write".into())
    })?;
    let stored = matches!(sections.form, Rpf9Form::StoredRsc8 { .. });
    let section = sections.virtual_section.as_slice();
    let aligned = Aligned::from_bytes(section);
    let s = section_of(&aligned);
    let mut out = section.to_vec();
    let mut edited = resource.clone();
    match *edit {
        AnimEdit::Pose { .. } | AnimEdit::AddDof { .. } | AnimEdit::RemoveDof { .. } => {
            return relayout::edit(resource, edit);
        }
        AnimEdit::Key {
            animation: AnimAt { set, animation: a },
            dof,
            component,
            frame,
            value,
        } => {
            let unwritten = |why: String| AnimError::Unwritten(why);
            let animation = (resource.sets.get(set))
                .and_then(|s| s.animations.get(a))
                .ok_or_else(|| unwritten(format!("the resource has no animation {set}.{a}")))?;
            if dof >= animation.dofs.len() || component > 3 || frame >= animation.frames {
                return Err(unwritten(format!(
                    "animation {:?} has {} DOF(s) of 4 components and {} frame(s), so no component {component} of DOF {dof} at frame {frame}",
                    animation.name,
                    animation.dofs.len(),
                    animation.frames
                )));
            }
            let set_at = s.set_offsets(resource)?[set];
            let at = s.animation_offsets(set_at)?[a];
            let chunk = usize::from(animation.frames_per_chunk);
            let index = (dof * 4) as u16 | u16::from(component);
            let blocks = (0..animation.blocks.len())
                .filter_map(|b| {
                    let first = b * chunk;
                    let frame = usize::from(frame).checked_sub(first)?;
                    (frame < animation.frames_in_block(b)).then_some((
                        b,
                        Some(KeyEdit {
                            index,
                            frame,
                            value,
                        }),
                    ))
                })
                .collect::<Vec<_>>();
            if blocks.is_empty() {
                return Err(unwritten(format!(
                    "no block of animation {:?} holds frame {frame}",
                    animation.name
                )));
            }
            let held = stored.then_some(resource);
            write_blocks(&s, &mut out, animation, at, blocks, held)?;
            let read =
                section_of(&Aligned::from_bytes(&out)).animation(at, set_at, animation.key)?;
            edited.sets[set].animations[a] = read;
            // A block placed before a stored resource's last region moved it: it reads again whole.
            // A stored section with no last region took the block past its end, as a mapped one.
            let moved = stored && resource.end_region.is_some() && out.len() != section.len();
            if moved && read_stored(&out)?.end_region != resource.end_region {
                return Err(AnimError::Unwritten(
                    "the stored resource's last region does not read again as it was".into(),
                ));
            }
        }
        AnimEdit::EntryWindow {
            clip,
            group,
            entry,
            start,
            end,
            frame_rate,
        } => {
            let at = *(s.clip_offsets(resource)?.get(clip))
                .ok_or_else(|| AnimError::Unwritten("the resource has no such clip".into()))?;
            let place = clips::EntryAt { clip, group, entry };
            let window = [start, end];
            clips::entry_window(&s, &mut out, &mut edited, at, place, window, frame_rate)?;
        }
        AnimEdit::ClipWindow {
            clip,
            start,
            end,
            rate,
        } => {
            let unwritten = |why: &str| AnimError::Unwritten(why.to_string());
            let at = *(s.clip_offsets(resource)?.get(clip))
                .ok_or_else(|| unwritten("the resource has no such clip"))?;
            if s.u32("clip", at + 0x20)? != 1 {
                return Err(unwritten(
                    "only a clip of one animation states a window and a rate; a clip of groups states them for each entry",
                ));
            }
            if !(start.is_finite() && end.is_finite() && rate.is_finite())
                || end < start
                || rate <= 0.0
            {
                return Err(unwritten(
                    "a window runs from a finite start to an end no earlier, at a rate above 0",
                ));
            }
            for (field, value) in [(0x90, start), (0x94, end), (0x98, rate)] {
                out[at + field..at + field + 4].copy_from_slice(&value.to_le_bytes());
            }
            if let ClipPlay::One {
                start: s,
                end: e,
                rate: r,
                ..
            } = &mut edited.clips[clip].play
            {
                (*s, *e, *r) = (start, end, rate);
            }
        }
    }
    edited.sections = Some(Arc::new(Rpf9Sections {
        form: sections.form,
        virtual_section: out,
        physical: sections.physical.clone(),
    }));
    edited.edited = true;
    Ok(edited)
}

/// What [`edit_in_extent`] refuses an edit with when it would change the virtual section's length.
pub(super) const PAST_EXTENT: &str = "a copy patched in place keeps every length";

/// [`edit`] where every block it writes keeps within its old extent, so the virtual section keeps its length; refused by name otherwise.
// A block is re-encoded at its tightest, which fits wherever the block's own bit widths and counts
// would, so no looser encoding is tried; a block that comes back shorter is zero-filled to its
// old extent.
pub fn edit_in_extent(
    resource: &AnimResource,
    change: &AnimEdit,
) -> Result<AnimResource, AnimError> {
    let length = |r: &AnimResource| (r.sections.as_deref()).map(|s| s.virtual_section.len());
    let edited = edit(resource, change)?;
    let (was, now) = (length(resource), length(&edited));
    if was != now {
        return Err(AnimError::Unwritten(format!(
            "it moves a block past its old extent, so the virtual section of {} bytes would become {}; {PAST_EXTENT}, so this edit needs a rebuilt archive, which the game refuses because only its maker can sign the table",
            was.unwrap_or(0),
            now.unwrap_or(0)
        )));
    }
    Ok(edited)
}

fn section_of(aligned: &Aligned) -> Section<'_> {
    let len = aligned.bytes().len();
    Section {
        image: Image::new(aligned.bytes(), len),
        left: std::cell::Cell::new(len.saturating_mul(2)),
    }
}

/// Writes `blocks` of the animation at `at` into `out`, each with its key edit, then the
/// animation's largest block size; a block that grows moves past the section's end, or before the
/// last region of `stored`, the resource of a stored section that ends with one.
fn write_blocks(
    s: &Section,
    out: &mut Vec<u8>,
    animation: &Animation,
    at: usize,
    blocks: Vec<(usize, Option<KeyEdit>)>,
    stored: Option<&AnimResource>,
) -> Result<(), AnimError> {
    let slots = s.array("block list", at + 0x50, 8)?;
    let dofs = animation.dofs.len();
    let mut written = Vec::with_capacity(blocks.len());
    for (b, edit) in blocks {
        let slot = *(slots.get(b)).ok_or_else(|| AnimError::Unwritten(format!("no block {b}")))?;
        let stream = s.block_stream(s.target("block", slot)?)?;
        // A section-5 channel's table and index width follow from its frames, so its block is
        // laid out again.
        let this = (&stream, animation.name.as_str(), b);
        let indirect = (edit
            .map(|e| relayout::indirect_key(this, &animation.dofs, e.index, e.frame, e.value)))
        .transpose()?;
        if let Some(bytes) = indirect.flatten() {
            written.push((b, bytes));
            continue;
        }
        let (stream, edit) = channels::apply(stream, dofs, edit).map_err(|why| {
            AnimError::Unwritten(format!(
                "block {b} of animation {:?}: {why}",
                animation.name
            ))
        })?;
        let bytes = block_bytes(&stream, dofs, edit).map_err(|e| e.at(&animation.name, b))?;
        written.push((b, bytes));
    }
    place_blocks(s, out, at, written, stored)
}

/// Each of `blocks`, the bytes of a block by its place in the list of the animation at `at`,
/// written into `out`, then the animation's largest block size; a block that grows moves past the
/// section's end, or before the last region of `stored`, the resource of a stored section that
/// ends with one.
fn place_blocks(
    s: &Section,
    out: &mut Vec<u8>,
    at: usize,
    blocks: Vec<(usize, Vec<u8>)>,
    stored: Option<&AnimResource>,
) -> Result<(), AnimError> {
    let slots = s.array("block list", at + 0x50, 8)?;
    let mut sizes = Vec::with_capacity(slots.len());
    for slot in &slots {
        let stream = s.block_stream(s.target("block", *slot)?)?;
        sizes.push(stream.data.len());
    }
    for (b, bytes) in blocks {
        let read_slot =
            *(slots.get(b)).ok_or_else(|| AnimError::Unwritten(format!("no block {b}")))?;
        let old_len = BLOCK_HEAD + sizes[b];
        let ended = stored.filter(|r| r.end_region.is_some());
        // An earlier block of this edit may have moved the last region, which can hold the list.
        let (slot, old_at) = match ended {
            Some(_) => (
                stored::now_at(s, out, read_slot)?,
                stored::now_at(s, out, s.target("block", read_slot)?)?,
            ),
            None => (read_slot, s.target("block", read_slot)?),
        };
        if bytes.len() <= old_len {
            out[old_at..old_at + old_len].fill(0);
            out[old_at..old_at + bytes.len()].copy_from_slice(&bytes);
        } else if let Some(resource) = ended {
            let new_at = stored::insert_before_end(s, resource, out, &bytes)?;
            let slot = stored::now_at(s, out, read_slot)?;
            out[slot..slot + 8].copy_from_slice(&(VIRTUAL_BASE + new_at as u64).to_le_bytes());
        } else {
            let new_at = out.len().next_multiple_of(BLOCK_ALIGN);
            out.resize(new_at, 0);
            out.extend_from_slice(&bytes);
            out[slot..slot + 8].copy_from_slice(&(VIRTUAL_BASE + new_at as u64).to_le_bytes());
        }
        sizes[b] = bytes.len() - BLOCK_HEAD;
    }
    if let Some(largest) = sizes.iter().max() {
        let max = u32::try_from(largest + BLOCK_HEAD)
            .map_err(|_| AnimError::Unwritten("a block passes 4 GiB".into()))?;
        out[at + 0x48..at + 0x4C].copy_from_slice(&max.to_le_bytes());
    }
    Ok(())
}

/// Block `stream`'s header and payload, every section-10 channel encoded anew from its steps
/// with `edit` made, and the constant size, channel offset, data size and the mover entries'
/// constant words recomputed; the frame fields follow from sections 3 to 5, which no edit moves.
fn block_bytes(stream: &CrBlockStream, dofs: usize, edit: Option<KeyEdit>) -> Result<Vec<u8>, Bad> {
    let bad = |why: String| Bad::Layout(why);
    let dir = directory(stream, dofs)?;
    if dir.end != (stream.channel_offset as usize).wrapping_sub(BLOCK_HEAD) {
        return Err(bad(format!(
            "its directory ends at {}, where its channel offset states {}",
            dir.end, stream.channel_offset
        )));
    }
    let constants = constants(stream)?;
    let ranges = delta_ranges(&constants, &dir.runs)?;
    if let Some(edit) = edit {
        if !ranges.iter().any(|(index, _)| *index == edit.index) {
            let dof = u32::from(edit.index >> 2);
            let section = (dir.runs.iter()).position(|run| run.contains(&edit.index));
            return Err(Bad::Delta(dof, match section {
                Some(section) => format!("its component {} is in section {section}, whose channels are not written here: only section 10's are", edit.index & 3),
                None => format!("it has no component {}", edit.index & 3),
            }));
        }
    }
    let first = ranges.first().map_or(constants.len(), |(_, r)| r.start);
    let mut words = constants[..first].to_vec();
    let mut starts = Vec::with_capacity(ranges.len());
    for (index, range) in &ranges {
        let dof = u32::from(*index) >> 2;
        let channel = &constants[range.clone()];
        let mut q = section10::quantised(channel, usize::from(stream.num_frames))
            .map_err(|why| Bad::Delta(dof, why))?;
        let (mut scale, mut offset) = (channel[2], channel[3]);
        if let Some(edit) = edit.filter(|e| e.index == *index) {
            (scale, offset) = requantise(&mut q, scale, offset, edit.frame, edit.value)
                .map_err(|why| Bad::Delta(dof, why))?;
        }
        starts.push((range.start, words.len()));
        words.extend(section10::encode(&q, scale, offset).map_err(|why| Bad::Delta(dof, why))?);
    }
    let movers = &stream.data[dir.end..];
    let nibbles = (stream.num_mover_channels >> 4) + (stream.num_mover_channels & 15);
    if movers.len() != MOVER_ENTRY * usize::from(nibbles) {
        return Err(bad(format!(
            "it states {nibbles} mover channel(s) and holds {} bytes after its directory, not 6 for each",
            movers.len()
        )));
    }
    let mut entries = movers.to_vec();
    for entry in entries.chunks_mut(MOVER_ENTRY) {
        if usize::from(entry[0]) != DELTA_SECTION {
            continue;
        }
        let old = usize::from(u16::from_le_bytes([entry[2], entry[3]]));
        let new = (starts.iter().find(|(from, _)| *from == old))
            .and_then(|(_, to)| u16::try_from(*to).ok())
            .ok_or_else(|| {
                bad(format!(
                    "a section-10 mover entry names constant word {old}, where no section-10 channel starts or a new start passes 16 bits"
                ))
            })?;
        entry[2..4].copy_from_slice(&new.to_le_bytes());
    }
    let constant_size = 4 * words.len();
    let frames_end = constant_size + (dir.end - stream.constant_size as usize);
    let size = |v: usize| u32::try_from(v).map_err(|_| bad("it passes 4 GiB".into()));
    let mut out = Vec::with_capacity(BLOCK_HEAD + frames_end + entries.len());
    for word in [
        stream.hash,
        size(frames_end + entries.len())?,
        stream.compact_size,
        size(constant_size)?,
        size(BLOCK_HEAD + frames_end)?,
    ] {
        out.extend(word.to_le_bytes());
    }
    for half in [
        stream.slop_size,
        stream.num_frames,
        stream.frame_size,
        stream.indirect_size,
        stream.quantize_size,
    ] {
        out.extend(half.to_le_bytes());
    }
    out.extend([stream.segment_size, stream.num_mover_channels]);
    out.extend(words.iter().flat_map(|w| w.to_le_bytes()));
    out.extend(&stream.data[stream.constant_size as usize..dir.end]);
    out.extend(entries);
    Ok(out)
}

/// `q` with step `frame` set to `value`, and the scale and offset words they read with: the
/// channel's own while `value` lies on its range and the steps still span `0 ..= 2^Q - 1`, else
/// the steps quantised afresh over the values' range in the channel's bits, one at least.
fn requantise(
    q: &mut [i64],
    scale: u32,
    offset: u32,
    frame: usize,
    value: f32,
) -> Result<(u32, u32), String> {
    if !value.is_finite() || frame >= q.len() {
        return Err(format!(
            "the value {value} at frame {frame} of {} is not a finite value of a frame it holds",
            q.len()
        ));
    }
    let (s, o) = (f32::from_bits(scale), f32::from_bits(offset));
    let top = q.iter().copied().max().unwrap_or(0).max(0) as u64;
    let bits = 64 - top.leading_zeros();
    let span = |bits: u32| (1i64 << bits) - 1;
    if s > 0.0 {
        let step = ((f64::from(value) - f64::from(o)) / f64::from(s)).round();
        if (0.0..=span(bits) as f64).contains(&step) {
            let was = std::mem::replace(&mut q[frame], step as i64);
            let (lo, hi) = (q.iter().min(), q.iter().max());
            if lo == Some(&0) && hi == Some(&span(bits)) {
                return Ok((scale, offset));
            }
            q[frame] = was;
        }
    }
    let mut values: Vec<f64> = (q.iter())
        .map(|x| f64::from(section10::value(s, o, *x)))
        .collect();
    values[frame] = f64::from(value);
    let lo = values.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if hi == lo {
        q.fill(0);
        return Ok((0f32.to_bits(), (lo as f32).to_bits()));
    }
    let top = span(bits.max(1));
    let (scale, offset) = (((hi - lo) / top as f64) as f32, lo as f32);
    if !(scale.is_finite() && scale > 0.0) {
        return Err(format!("its range {lo} to {hi} has no finite step"));
    }
    for (x, v) in q.iter_mut().zip(values) {
        *x = ((v - f64::from(offset)) / f64::from(scale))
            .round()
            .clamp(0.0, top as f64) as i64;
    }
    Ok((scale.to_bits(), offset.to_bits()))
}

impl Bad {
    /// The error of block `block` of animation `animation`.
    pub(super) fn at(self, animation: &str, block: usize) -> AnimError {
        let animation = animation.to_string();
        match self {
            Bad::Layout(why) => AnimError::Directory {
                animation,
                block,
                why,
            },
            Bad::Section(section, count) => AnimError::Section {
                animation,
                block,
                section,
                count,
            },
            Bad::Delta(dof, why) => AnimError::Delta {
                animation,
                block,
                dof,
                why,
            },
        }
    }
}

impl Section<'_> {
    /// The offsets of `resource`'s sets, in its order.
    fn set_offsets(&self, resource: &AnimResource) -> Result<Vec<usize>, AnimError> {
        match resource.root {
            RootKind::AnimationSet => Ok(vec![0]),
            RootKind::MotionDictionary => Ok(Vec::new()),
            RootKind::ClipDictionary => {
                let mut sets = Vec::new();
                for at in self.array("animation set list", 0x70, 16)? {
                    sets.extend(self.pointer("animation set", at)?);
                }
                Ok(sets)
            }
        }
    }

    /// The offsets of the animations of the set at `set`, in its map's order.
    fn animation_offsets(&self, set: usize) -> Result<Vec<usize>, AnimError> {
        (self.map("animation", set + 0x48, 0x10, false)?.into_iter())
            .map(|(_, entry)| self.target("animation", entry + 8))
            .collect()
    }

    /// The offsets of `resource`'s clips, in its order.
    fn clip_offsets(&self, resource: &AnimResource) -> Result<Vec<usize>, AnimError> {
        if resource.root != RootKind::ClipDictionary {
            return Ok(Vec::new());
        }
        (self.map("clip", 0x48, 0x10, true)?.into_iter())
            .map(|(_, entry)| self.target("clip", entry + 8))
            .collect()
    }
}
