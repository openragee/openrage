//! An animation's blocks laid out again from their channels, for an edit that changes which
//! channels a block holds: a DOF held at one value over the whole clip, or a DOF added or removed;
//! and a block whose section-5 channel is keyed, since its table and index width change.
//! Every channel kept keeps its constant words and its frame bits as read; the header, the
//! directory and the mover entries follow from the channels, as every block read states them.
//!
//! ⚠ An edited block keeps the hash it was read with, which follows from no rule anyone found.

use std::sync::Arc;

use super::super::{
    constants, directory, dof_signature, read, read_stored, AnimError, AnimResource, Bad, Dof,
    Section, DELTA_SECTION, DOF_LEN, SECTIONS,
};
use super::{
    mapped, place_blocks, section_of, stored, AnimEdit, BLOCK_ALIGN, BLOCK_HEAD, VIRTUAL_BASE,
};
use crate::container::rpf::rpf9_grow::grow_section_with_data;
use crate::container::rpf::{Rpf9Form, Rpf9Section, Rpf9Sections};
use crate::decl::Aligned;
use crate::gta5::CrBlockStream;
use crate::gta6::hash::name_hash32;

/// The tracks whose channels the block's mover entries name, translation first.
const MOVERS: [u32; 2] = [
    name_hash32(b"moverAbsoluteTranslation"),
    name_hash32(b"moverAbsoluteRotation"),
];
/// The animation flag set exactly when a DOF has a mover track.
const MOVER_FLAG: u16 = 0x10;

/// What a relayout does to one DOF of an animation.
#[derive(Debug, Clone, Copy)]
enum Relayout {
    /// DOF `dof` held at `value`: one constant channel in place of its channels.
    Pose { dof: usize, value: [f32; 4] },
    /// A DOF now at `dof`, held at `value`; the DOFs from there on moved up one place.
    Insert { dof: usize, value: [f32; 4] },
    /// DOF `dof` gone with its channels; the DOFs after it moved down one place.
    Remove { dof: usize },
}

/// A channel of a block: its directory section and entry, its constant words, and where its
/// `width` bits lie in each frame as read, or the value each frame takes in their place.
#[derive(Debug, Clone)]
struct Channel {
    section: usize,
    index: u16,
    words: Vec<u32>,
    width: usize,
    bit: usize,
    frames: Option<Vec<u64>>,
}

fn unwritten(why: impl Into<String>) -> AnimError {
    AnimError::Unwritten(why.into())
}

fn put(out: &mut [u8], at: usize, bytes: &[u8]) {
    out[at..at + bytes.len()].copy_from_slice(bytes);
}

/// `resource` with `edit`, a pose over the whole clip or a DOF added or removed, made in its
/// sections and read again, marked edited.
pub(super) fn edit(resource: &AnimResource, edit: &AnimEdit) -> Result<AnimResource, AnimError> {
    let (place, relayout, added) = match *edit {
        AnimEdit::Pose {
            animation,
            dof,
            value,
        } => (animation, Relayout::Pose { dof, value }, None),
        AnimEdit::AddDof {
            animation,
            dof,
            value,
        } => (animation, Relayout::Insert { dof: 0, value }, Some(dof)),
        AnimEdit::RemoveDof { animation, dof } => (animation, Relayout::Remove { dof }, None),
        _ => return Err(unwritten("the edit changes no block's channels")),
    };
    let sections = (resource.sections.as_deref())
        .ok_or_else(|| unwritten("the resource was read with no sections to write"))?;
    let stored = matches!(sections.form, Rpf9Form::StoredRsc8 { .. });
    let section = sections.virtual_section.as_slice();
    let aligned = Aligned::from_bytes(section);
    let s = section_of(&aligned);
    let animation = (resource.sets.get(place.set))
        .and_then(|set| set.animations.get(place.animation))
        .ok_or_else(|| {
            unwritten(format!(
                "the resource has no animation {}.{}",
                place.set, place.animation
            ))
        })?;
    let no_dof = |dof: usize| AnimError::NoDof {
        animation: animation.name.clone(),
        dof,
        count: animation.dofs.len(),
    };
    let mut dofs = animation.dofs.clone();
    let relayout = match (relayout, added) {
        (Relayout::Insert { value, .. }, Some(new)) => {
            if (dofs.iter()).any(|d| (d.track, d.id) == (new.track, new.id)) {
                return Err(unwritten(format!(
                    "animation {:?} already has a DOF of id {:#010X} and track {}",
                    animation.name,
                    new.id,
                    new.track_label()
                )));
            }
            let at = dofs.partition_point(|d| (d.track, d.id) < (new.track, new.id));
            dofs.insert(at, new);
            Relayout::Insert { dof: at, value }
        }
        (Relayout::Pose { dof, .. }, _) | (Relayout::Remove { dof }, _) if dof >= dofs.len() => {
            return Err(no_dof(dof));
        }
        (Relayout::Remove { dof }, _) => {
            dofs.remove(dof);
            relayout
        }
        (other, _) => other,
    };
    if dofs.len() * 4 > usize::from(u16::MAX) {
        return Err(unwritten(
            "an animation's DOFs number their directory entries in 16 bits, which this many pass",
        ));
    }
    if let Relayout::Pose { dof, value } | Relayout::Insert { dof, value } = relayout {
        let kind = dofs.get(dof).map(|d| d.kind).ok_or_else(|| no_dof(dof))?;
        constant_words(kind, value).map_err(|why| {
            unwritten(format!(
                "DOF {dof} of animation {:?}: {why}",
                animation.name
            ))
        })?;
    }
    let set_at = *(s.set_offsets(resource)?.get(place.set))
        .ok_or_else(|| unwritten("the resource has no such animation set"))?;
    let at = *(s.animation_offsets(set_at)?.get(place.animation))
        .ok_or_else(|| unwritten("the set has no such animation"))?;
    let mut written = Vec::with_capacity(animation.blocks.len());
    for (b, slot) in s.array("block list", at + 0x50, 8)?.into_iter().enumerate() {
        let stream = s.block_stream(s.target("block", slot)?)?;
        let bytes = relaid(&stream, animation.dofs.len(), &dofs, relayout)
            .map_err(|e| e.at(&animation.name, b))?;
        written.push((b, bytes));
    }
    let mut out = section.to_vec();
    place_blocks(&s, &mut out, at, written, stored.then_some(resource))?;
    if dofs.len() != animation.dofs.len() {
        let mut list = Vec::with_capacity(DOF_LEN * dofs.len());
        for d in &dofs {
            list.extend(
                [d.id, d.track, d.kind]
                    .into_iter()
                    .flat_map(u32::to_le_bytes),
            );
        }
        let pointer = match list.is_empty() {
            true => 0,
            false => VIRTUAL_BASE + place_list(&s, resource, sections, &mut out, &list)? as u64,
        };
        // A growth that added block-map slots moved what follows them, perhaps the animation.
        let at = {
            let placed = Aligned::from_bytes(&out);
            let p = section_of(&placed);
            let set_at = *(p.set_offsets(resource)?.get(place.set))
                .ok_or_else(|| unwritten("the resource has no such animation set"))?;
            *(p.animation_offsets(set_at)?.get(place.animation))
                .ok_or_else(|| unwritten("the set has no such animation"))?
        };
        let count = dofs.len() as u16;
        put(&mut out, at + 0x60, &pointer.to_le_bytes());
        put(&mut out, at + 0x68, &count.to_le_bytes());
        put(&mut out, at + 0x6A, &count.to_le_bytes());
        put(&mut out, at + 0x2C, &dof_signature(&dofs).to_le_bytes());
        let moved = dofs.iter().any(|d| MOVERS.contains(&d.track));
        let flags = (animation.flags & !MOVER_FLAG) | if moved { MOVER_FLAG } else { 0 };
        put(&mut out, at + 0x20, &flags.to_le_bytes());
    }
    // Every clip that plays the animation takes its signature at 0x2C into its own at 0x38.
    super::super::signature::resign_clips(section, &mut out)?;
    let mut edited = if stored {
        read_stored(&out)?
    } else {
        read(&out)?
    };
    if stored && resource.end_region.is_some() && edited.end_region != resource.end_region {
        return Err(unwritten(
            "the stored resource's last region does not read again as it was",
        ));
    }
    edited.sections = Some(Arc::new(Rpf9Sections {
        form: sections.form,
        virtual_section: out,
        physical: sections.physical.clone(),
    }));
    edited.edited = true;
    Ok(edited)
}

/// Where `bytes` now lie in `out`, placed after every object: through the section's growth where
/// it is block-mapped, before the last region of a stored one that ends with one, else at its end.
pub(super) fn place_list(
    s: &Section,
    resource: &AnimResource,
    sections: &Rpf9Sections,
    out: &mut Vec<u8>,
    bytes: &[u8],
) -> Result<usize, AnimError> {
    match sections.form {
        Rpf9Form::BlockMapped => {
            // Read from what is written so far, where a grown block now lies past the end.
            let held = Aligned::from_bytes(out);
            let (pointers, data) = mapped::pointer_fields(&section_of(&held), resource)?;
            let len = out.len().next_multiple_of(BLOCK_ALIGN);
            out.resize(len, 0);
            let mut grown = Rpf9Sections {
                form: sections.form,
                virtual_section: std::mem::take(out),
                physical: sections.physical.clone(),
            };
            let by = bytes.len().next_multiple_of(BLOCK_ALIGN) as isize;
            let grew =
                grow_section_with_data(&mut grown, &pointers, &data, Rpf9Section::Virtual, len, by);
            *out = grown.virtual_section;
            let at = grew.map_err(|e| unwritten(format!("the DOF list is not placed: {e}")))?;
            put(out, at, bytes);
            Ok(at)
        }
        Rpf9Form::StoredRsc8 { .. } if resource.end_region.is_some() => {
            stored::insert_before_end(s, resource, out, bytes)
        }
        Rpf9Form::StoredRsc8 { .. } => {
            let at = out.len().next_multiple_of(BLOCK_ALIGN);
            out.resize(at, 0);
            out.extend_from_slice(bytes);
            Ok(at)
        }
    }
}

/// Block `stream`, read with `read_dofs` DOFs, laid out again for `dofs` with `relayout` made.
fn relaid(
    stream: &CrBlockStream,
    read_dofs: usize,
    dofs: &[Dof],
    relayout: Relayout,
) -> Result<Vec<u8>, Bad> {
    let mut channels = channels_of(stream, read_dofs)?;
    let dof_of = |c: &Channel| usize::from(c.index >> 2);
    let held = |dof: usize, value: [f32; 4]| {
        let kind = dofs.get(dof).map_or(u32::MAX, |d| d.kind);
        let index = u16::try_from(dof * 4)
            .map_err(|_| Bad::Layout("its DOF's directory entry passes 16 bits".into()))?;
        let words = constant_words(kind, value).map_err(Bad::Layout)?;
        Ok::<_, Bad>(
            (words.into_iter())
                .map(|(section, component, words)| Channel {
                    section,
                    index: index | component,
                    words,
                    width: 0,
                    bit: 0,
                    frames: None,
                })
                .collect::<Vec<_>>(),
        )
    };
    match relayout {
        Relayout::Pose { dof, value } => {
            channels.retain(|c| dof_of(c) != dof);
            channels.extend(held(dof, value)?);
        }
        Relayout::Remove { dof } => {
            channels.retain(|c| dof_of(c) != dof);
            for c in channels.iter_mut() {
                if usize::from(c.index >> 2) > dof {
                    c.index -= 4;
                }
            }
        }
        Relayout::Insert { dof, value } => {
            for c in channels.iter_mut() {
                if usize::from(c.index >> 2) >= dof {
                    c.index += 4;
                }
            }
            channels.extend(held(dof, value)?);
        }
    }
    channels.sort_by_key(|c| (c.section, c.index));
    encode(stream, &channels, dofs)
}

/// Block `block` of `animation`, read as `stream`, with frame `frame` of the section-5 channel of
/// directory entry `index` set to `value` and laid out again; `None` where no section-5 channel
/// has that entry.
// A section-5 channel's frames index a table of distinct steps in the order the frames first use
// them, the smallest 0; its index width is the bit length of the entry count and its entry width
// the bit length of the largest step. Its scale is kept and its offset moved by whole steps.
pub(super) fn indirect_key(
    (stream, animation, block): (&CrBlockStream, &str, usize),
    dofs: &[Dof],
    index: u16,
    frame: usize,
    value: f32,
) -> Result<Option<Vec<u8>>, AnimError> {
    // A key of another section is written in place, so this block's stricter layout is not asked.
    let in_section_5 = directory(stream, dofs.len()).is_ok_and(|d| d.runs[5].contains(&index));
    if !in_section_5 {
        return Ok(None);
    }
    let layout = |e: Bad| e.at(animation, block);
    let mut channels = channels_of(stream, dofs.len()).map_err(layout)?;
    let Some(c) = (channels.iter_mut()).find(|c| c.section == 5 && c.index == index) else {
        return Ok(None);
    };
    let bad = |why: String| {
        unwritten(format!(
            "block {block} of animation {animation:?}: its section-5 channel of DOF {}: {why}",
            index >> 2
        ))
    };
    let frames = usize::from(stream.num_frames);
    let read_size = usize::from(stream.frame_size);
    let start = stream.constant_size as usize;
    let read_frames = (stream.data.get(start..start + read_size * frames))
        .ok_or_else(|| bad("its frames run past its payload".into()))?;
    // The channel read consumed its five words and its table, and its index bits fit its frames.
    let (ib, vb) = (c.width, c.words[1] as usize);
    let (scale, offset) = (f32::from_bits(c.words[3]), f32::from_bits(c.words[4]));
    let table = &c.words[5..];
    if vb > 32 {
        return Err(bad(format!(
            "it states {vb} bits a value, past the 32 a word holds"
        )));
    }
    let mut steps = Vec::with_capacity(frames);
    for f in 0..frames {
        let at = bits_at(&read_frames[f * read_size..(f + 1) * read_size], c.bit, ib) as usize;
        let from = at.checked_mul(vb).filter(|b| b + vb <= 32 * table.len());
        let from = from.ok_or_else(|| {
            bad(format!(
                "frame {f} reads entry {at} past its table of {} words",
                table.len()
            ))
        })?;
        steps.push(bits_in(table, from, vb) as i64);
    }
    if !value.is_finite() || frame >= frames {
        return Err(bad(format!(
            "the value {value} at frame {frame} of {frames} is not a finite value of a frame it holds"
        )));
    }
    if !(scale.is_finite() && scale > 0.0) {
        return Err(bad(format!(
            "its scale {scale} gives no step a new value can take"
        )));
    }
    steps[frame] = ((f64::from(value) - f64::from(offset)) / f64::from(scale)).round() as i64;
    let low = steps.iter().copied().min().unwrap_or(0);
    let offset = (f64::from(offset) + low as f64 * f64::from(scale)) as f32;
    let mut entries: Vec<i64> = Vec::new();
    let mut indices = Vec::with_capacity(frames);
    for step in steps {
        let step = step - low;
        if step > i64::from(u32::MAX) {
            return Err(bad(format!(
                "the value {value} lies more steps of {scale} from the channel's others than 32 bits count"
            )));
        }
        let at = match entries.iter().position(|e| *e == step) {
            Some(at) => at,
            None => {
                entries.push(step);
                entries.len() - 1
            }
        };
        indices.push(at as u64);
    }
    let bit_length = |v: u64| (64 - v.leading_zeros()) as usize;
    let ib = bit_length(entries.len() as u64);
    let vb = bit_length(entries.iter().copied().max().unwrap_or(0) as u64);
    let mut table = vec![0u32; (entries.len() * vb).div_ceil(32)];
    for (i, entry) in entries.iter().enumerate() {
        for b in 0..vb {
            if (entry >> b) & 1 == 1 {
                table[(i * vb + b) / 32] |= 1 << ((i * vb + b) % 32);
            }
        }
    }
    c.words = vec![
        ib as u32,
        vb as u32,
        table.len() as u32,
        scale.to_bits(),
        offset.to_bits(),
    ];
    c.words.extend(table);
    c.width = ib;
    c.frames = Some(indices);
    encode(stream, &channels, dofs).map(Some).map_err(layout)
}

/// Bits `bit .. bit + width` of `words`, least significant first; every bit lies in them.
fn bits_in(words: &[u32], bit: usize, width: usize) -> u64 {
    (0..width).fold(0, |v, b| {
        let at = bit + b;
        v | (u64::from((words[at / 32] >> (at % 32)) & 1) << b)
    })
}

/// The section and the constant words that hold a DOF of type `kind` at `value`: a rotation's x,
/// y and z with w made positive, a vector's x, y and z, one float, or a type 3's two floats.
// Each channel is its section, the component its directory entry adds to `DOF × 4`, and its words.
fn constant_words(kind: u32, value: [f32; 4]) -> Result<Vec<(usize, u16, Vec<u32>)>, String> {
    if value.iter().any(|v| !v.is_finite()) {
        return Err(format!("the value {value:?} is not finite"));
    }
    let bits = |v: &[f32]| -> Vec<u32> { v.iter().map(|v| v.to_bits()).collect() };
    match kind {
        0 => {
            let length = value.iter().map(|v| v * v).sum::<f32>().sqrt();
            if !length.is_finite() || length <= f32::MIN_POSITIVE {
                return Err("a rotation of no length is no rotation".into());
            }
            let sign = if value[3] < 0.0 { -1.0 } else { 1.0 };
            let q = value.map(|v| sign * v / length);
            Ok(vec![(0, 0, bits(&q[..3]))])
        }
        2 => Ok(vec![(1, 0, bits(&value[..3]))]),
        // A type 3 holds components 0 and 1, each a section-2 float of its own.
        3 => Ok(vec![(2, 0, bits(&value[..1])), (2, 1, bits(&value[1..2]))]),
        4 => Ok(vec![(2, 0, bits(&value[..1]))]),
        _ => Err(format!(
            "a DOF of type {kind} is held by no constant channel OpenRage writes: only types 0, 2, 3 and 4 are"
        )),
    }
}

/// Each channel of block `stream`, in its directory's order, which is the order its constants
/// and frame bits are consumed in.
fn channels_of(stream: &CrBlockStream, dofs: usize) -> Result<Vec<Channel>, Bad> {
    let bad = |why: String| Bad::Layout(why);
    let dir = directory(stream, dofs)?;
    if dir.end != (stream.channel_offset as usize).wrapping_sub(BLOCK_HEAD) {
        return Err(bad(format!(
            "its directory ends at {}, where its channel offset states {}",
            dir.end, stream.channel_offset
        )));
    }
    let frames = usize::from(stream.num_frames) * usize::from(stream.frame_size);
    if stream.data.len() < stream.constant_size as usize + frames {
        return Err(bad("its frames run past its payload".into()));
    }
    let words = constants(stream)?;
    let room = 8 * usize::from(stream.frame_size);
    let word = |at: usize| {
        words.get(at).map(|w| *w as usize).ok_or_else(|| {
            bad(format!(
                "its channels read constant word {at} of {}",
                words.len()
            ))
        })
    };
    let (mut constant, mut bit) = (0usize, 0usize);
    let mut channels = Vec::new();
    for (section, run) in dir.runs.iter().enumerate() {
        for &index in run {
            let (used, width) = match section {
                0 | 1 => (3, 0),
                2 => (1, 0),
                3 => (0, 32),
                4 => (3, word(constant)? & 31),
                5 => (word(constant + 2)?.saturating_add(5), word(constant)?),
                7 | 8 => (0, 0),
                DELTA_SECTION => (word(constant)?, 0),
                _ => return Err(Bad::Section(section, dir.sections[section])),
            };
            let end = constant.checked_add(used).filter(|end| *end <= words.len());
            let fits = bit.checked_add(width).is_some_and(|end| end <= room);
            let (Some(end), true) = (end, fits) else {
                return Err(bad(format!(
                    "its entry {index} in section {section} reads past its constants or its frames"
                )));
            };
            if width > 32 || (section == DELTA_SECTION && used < 4) {
                return Err(bad(format!(
                    "its entry {index} in section {section} states {width} bits a frame and {used} words"
                )));
            }
            channels.push(Channel {
                section,
                index,
                words: words[constant..end].to_vec(),
                width,
                bit,
                frames: None,
            });
            (constant, bit) = (end, bit + width);
        }
    }
    if constant != words.len() {
        return Err(bad(format!(
            "its channels consume {constant} constant words of {}",
            words.len()
        )));
    }
    Ok(channels)
}

/// Block `stream`'s header and payload holding `channels`, sorted by section and entry, for
/// `dofs`: their constants, their frame bits packed in that order, the directory padded with
/// `4 × DOFs`, and an entry `{section | component << 8, constant word, frame bit}` for each
/// channel of a mover track, translation first.
fn encode(stream: &CrBlockStream, channels: &[Channel], dofs: &[Dof]) -> Result<Vec<u8>, Bad> {
    let bad = |why: &str| Bad::Layout(why.to_string());
    let frames = usize::from(stream.num_frames);
    let read_size = usize::from(stream.frame_size);
    let start = stream.constant_size as usize;
    let read_frames = (stream.data.get(start..start + read_size * frames))
        .ok_or_else(|| bad("its frames run past its payload"))?;
    let frame_size = 4 * channels.iter().map(|c| c.width).sum::<usize>().div_ceil(32);
    let mut counts = [0u16; SECTIONS];
    let mut words = Vec::new();
    let mut mover_entries: [Vec<[u16; 3]>; 2] = [Vec::new(), Vec::new()];
    let mut frame_bytes = vec![0u8; frame_size * frames];
    let (mut indirect, mut quantize, mut bit) = (0usize, 0usize, 0usize);
    for c in channels {
        let count = (counts.get_mut(c.section)).ok_or_else(|| bad("a channel has no section"))?;
        *count = (count.checked_add(1)).ok_or_else(|| bad("a section holds 65,536 channels"))?;
        let track = dofs.get(usize::from(c.index >> 2)).map(|d| d.track);
        if let Some(m) = MOVERS.iter().position(|t| Some(*t) == track) {
            let at = |v: usize| {
                u16::try_from(v)
                    .map_err(|_| bad("a mover channel's constant word or frame bit passes 16 bits"))
            };
            let named = c.section as u16 | (c.index & 3) << 8;
            mover_entries[m].push([named, at(words.len())?, at(bit)?]);
        }
        if c.width > 0 {
            for f in 0..frames {
                let from = &read_frames[f * read_size..(f + 1) * read_size];
                let value = match &c.frames {
                    Some(values) => values.get(f).copied().unwrap_or(0),
                    None => bits_at(from, c.bit, c.width),
                };
                let to = &mut frame_bytes[f * frame_size..(f + 1) * frame_size];
                put_bits(to, bit, c.width, value);
            }
        }
        match c.section {
            4 => quantize += c.width,
            5 => indirect += c.words.len(),
            _ => {}
        }
        words.extend_from_slice(&c.words);
        bit += c.width;
    }
    let pad = u16::try_from(dofs.len() * 4).map_err(|_| bad("its DOFs pass 16 bits"))?;
    let mut dir: Vec<u16> = counts.to_vec();
    for section in 0..SECTIONS {
        let run = channels.iter().filter(|c| c.section == section);
        let from = dir.len();
        dir.extend(run.map(|c| c.index));
        let n = dir.len() - from;
        dir.resize(dir.len() + (4 - n % 4) % 4, pad);
    }
    let [translation, rotation] = &mover_entries;
    if translation.len() > 15 || rotation.len() > 15 {
        return Err(bad(
            "it has more mover channels of a kind than the 15 its nibble counts",
        ));
    }
    let constant_size = 4 * words.len();
    let channel_end = constant_size + frame_size * frames + 2 * dir.len();
    let mover_bytes = 6 * (translation.len() + rotation.len());
    let word = |v: usize| u32::try_from(v).map_err(|_| bad("it passes 4 GiB"));
    let half = |v: usize| u16::try_from(v).map_err(|_| bad("a size of its header passes 16 bits"));
    let mut out = Vec::with_capacity(BLOCK_HEAD + channel_end + mover_bytes);
    for w in [
        stream.hash,
        word(channel_end + mover_bytes)?,
        stream.compact_size,
        word(constant_size)?,
        word(BLOCK_HEAD + channel_end)?,
    ] {
        out.extend(w.to_le_bytes());
    }
    for h in [
        stream.slop_size,
        stream.num_frames,
        half(frame_size)?,
        half(indirect)?,
        half(quantize)?,
    ] {
        out.extend(h.to_le_bytes());
    }
    let nibbles = (translation.len() << 4 | rotation.len()) as u8;
    out.extend([stream.segment_size, nibbles]);
    out.extend(words.iter().flat_map(|w| w.to_le_bytes()));
    out.extend(&frame_bytes);
    out.extend(dir.iter().flat_map(|w| w.to_le_bytes()));
    for entry in translation.iter().chain(rotation) {
        out.extend(entry.iter().flat_map(|h| h.to_le_bytes()));
    }
    Ok(out)
}

/// Bits `bit .. bit + width` of `frame`, least significant first.
fn bits_at(frame: &[u8], bit: usize, width: usize) -> u64 {
    (0..width).fold(0, |v, b| {
        let at = bit + b;
        v | (u64::from((frame[at / 8] >> (at % 8)) & 1) << b)
    })
}

fn put_bits(frame: &mut [u8], bit: usize, width: usize, value: u64) {
    for b in 0..width {
        let (at, one) = (bit + b, ((value >> b) & 1) as u8);
        frame[at / 8] = (frame[at / 8] & !(1 << (at % 8))) | (one << (at % 8));
    }
}
