//! Emitter rules, particle rules, events and behaviours copied past a particle effects list's
//! virtual section's end, each from a record of its kind the list holds: each array that grows is
//! laid out anew with its count equal to its capacity, and every word derived from it is derived
//! again. Nothing already in the section moves, so the words no reader types stay valid; a copied
//! rule's GPU buffers get data of their own past the physical section's end.

use super::copier::{dictionary as dictionary_anew, evolution_list as evolution_list_copied};
use super::copier::{kfp as keys_copied, Tail};
use super::*;
use crate::container::rpf::{grow_section, Rpf9Section, Rpf9Sections};

/// The word that addresses physical offset 0.
const PHYSICAL: u64 = 0x6000_0000;
const EMITTER_RULE: usize = 0x300;
const PARTICLE_RULE: usize = 0x430;
const EVENT: usize = 0x88;
/// Where an event points at its object.
const EVENT_OBJECT: usize = 0x60;
/// A domain's size, and a mesh domain's (shape 4).
const DOMAIN: usize = 0x1B0;
const MESH_DOMAIN: usize = 0x1C8;
const MESH: u32 = 4;
/// A GPU buffer object's size, and the alignment of every buffer's data.
const GPU_OBJECT: usize = 0x50;
const GPU_ALIGN: usize = 0x100;
/// No physical object crosses one of these.
const PHYSICAL_BLOCK: usize = 0x1_0000;
/// The buffer slot whose data copies the rule's floats at +0x1E0 to +0x1FB.
const FLOATS_SLOT: u16 = 0x390;
const RULE_FLOATS: (usize, usize) = (0x1E0, 0x1C);
/// A rule's sprite, trail and model class bits, which follow its draw type.
const DRAW_BITS: [u8; 3] = [27, 28, 29];
/// The root's pointers to the emitter and particle rule dictionaries.
const EMITTER_DICTIONARY: usize = 0x30;
const PARTICLE_DICTIONARY: usize = 0x38;

/// What a layout gives: its bytes, and the places of the records it copied, the first record first.
type Laid = (Tail, Vec<usize>);

type Copied = std::result::Result<(), ParticleKeysError>;

fn refused(why: &'static str) -> ParticleKeysError {
    HeaderlessError::Particles(why).into()
}

fn named(name: &str) -> Result<()> {
    need(
        !name.is_empty() && name.len() <= 512 && name.bytes().all(|b| (32..127).contains(&b)),
        "a name is not printable ASCII of at most 512 bytes",
    )
}

/// What `lay` lays out from a base past the virtual section's end of `body`, written there; the
/// list must then read as `expected` makes of it and the places laid out, or `body` is unchanged.
fn append(
    body: &mut Rpf9Sections,
    lay: impl Fn(&Bytes, usize, &ParticleEffects) -> Result<Laid>,
    expected: impl FnOnce(ParticleEffects, &[usize]) -> Result<ParticleEffects>,
) -> Copied {
    let stored = read_particle_effects(&body.virtual_section, None)?;
    let end = body.virtual_section.len().next_multiple_of(16);
    let (dry, _) = sections(&body.virtual_section, None, |v, _| lay(&v, end, &stored))?;
    let len = dry.bytes.len().next_multiple_of(16);
    let mut grown = body.clone();
    grown.virtual_section.resize(end, 0);
    let start = grow_section(&mut grown, &[0x08], Rpf9Section::Virtual, end, len as isize)?;
    // The block map's slots may have grown, which moves what follows them: read the list again.
    let moved = read_particle_effects(&grown.virtual_section, None)?;
    need(
        super::remover::sizes_cleared(moved.clone()) == super::remover::sizes_cleared(stored),
        "growing the section changes how what it holds reads, such as its last behaviour",
    )?;
    let (tail, places) = sections(&grown.virtual_section, None, |v, _| lay(&v, start, &moved))?;
    let v = &mut grown.virtual_section;
    for (at, bytes) in std::iter::once((start, tail.bytes)).chain(tail.patches) {
        match v.get_mut(at..at + bytes.len()) {
            Some(slot) => slot.copy_from_slice(&bytes),
            None => refuse("a copied word lies past the section's end")?,
        }
    }
    let read = read_particle_effects(&grown.virtual_section, None)?;
    let expected = expected(moved, &places)?;
    need(
        super::remover::sizes_cleared(read) == super::remover::sizes_cleared(expected),
        "the copy does not read back as its source, or changes what else the list holds",
    )?;
    *body = grown;
    Ok(())
}

/// The word at `at` read, `by` added, and written back.
fn counted(t: &mut Tail, v: &Bytes, at: usize, by: u32) -> Result<()> {
    let Some(n) = v.u32("reference count", at)?.checked_add(by) else {
        return refuse("a rule's reference count would pass its word");
    };
    t.put(at, &n.to_le_bytes())
}

/// The count and capacity of the array whose head is at `head` set to `n`.
fn counts(t: &mut Tail, head: usize, n: usize) -> Result<()> {
    need(
        n <= usize::from(u16::MAX),
        "an array would hold more elements than its count can state",
    )?;
    let n = (n as u16).to_le_bytes();
    t.put(head + 8, &[n, n].concat())
}

/// A copy of the event `event` of the effect rule `effect`, appended to that effect's timeline.
pub fn duplicate_particle_event(body: &mut Rpf9Sections, effect: usize, event: usize) -> Copied {
    let mut grown = body.clone();
    let data = event_object::data_copied(&mut grown, effect, event)?;
    append(
        &mut grown,
        |v, base, list| event_copied(v, base, list, (effect, event), data),
        |mut list, _| {
            let Some(rule) = list.effects.get_mut(effect) else {
                return refuse("the effect rule is not one of the list");
            };
            let Some(copy) = rule.events.get(event).cloned() else {
                return refuse("the event to copy is not one of its effect rule's");
            };
            rule.events.push(copy);
            Ok(list)
        },
    )?;
    event_object::copy_holds(&grown, effect, event)?;
    *body = grown;
    Ok(())
}

/// The event's copy, its object's buffer data at `data`.
fn event_copied(
    v: &Bytes,
    base: usize,
    list: &ParticleEffects,
    (effect, event): (usize, usize),
    data: Option<usize>,
) -> Result<Laid> {
    const WHAT: &str = "event";
    let Some(rule) = list.effects.get(effect) else {
        return refuse("the effect rule is not one of the list");
    };
    let Some(ev) = rule.events.get(event) else {
        return refuse("the event to copy is not one of its effect rule's");
    };
    let head = rule.at as usize + 0x38;
    let (events, count, _) = array(v, WHAT, head, false)?;
    let from = v.to(WHAT, events + 8 * event)?;
    let mut t = Tail::new(base);
    let copy = t.copy(v, WHAT, from, EVENT)?;
    event_object::object_copied(&mut t, v, (from, ev), copy, data)?;
    t.put(copy + 8, &(count as u32).to_le_bytes())?;
    if let Some(l) = v.ptr(WHAT, from + 0x30)? {
        let copied = evolution_list_copied(&mut t, v, l)?;
        t.ptr(copy + 0x30, copied)?;
    }
    let held = t.alloc(8 * (count + 1));
    t.put(held, v.get(WHAT, events, 8 * count)?)?;
    t.ptr(held + 8 * count, copy)?;
    t.ptr(head, held)?;
    counts(&mut t, head, count + 1)?;
    let rules = [
        list.emitters.get(ev.emitter).map(|r| r.at),
        list.particles.get(ev.particle).map(|r| r.at),
    ];
    for at in rules {
        let Some(at) = at else {
            return refuse("an event names a rule the list does not hold");
        };
        counted(&mut t, v, at as usize + 0x10, 1)?;
    }
    Ok((t, vec![copy]))
}

/// The list's emitter rule `source` copied as `name`, its domains and keyframed values its own.
pub fn duplicate_particle_emitter(body: &mut Rpf9Sections, source: usize, name: &str) -> Copied {
    named(name)?;
    let code = name_hash32(name.as_bytes());
    append(
        body,
        |v, base, list| {
            let Some(from) = list.emitters.get(source) else {
                return refuse("the emitter rule to copy is not one of the list");
            };
            let mut t = Tail::new(base);
            let rule = emitter_copied(&mut t, v, from.at as usize, name, code)?;
            dictionary_anew(&mut t, v, EMITTER_DICTIONARY, code, rule)?;
            Ok((t, vec![rule]))
        },
        |mut list, places| {
            let copy = list.emitters.get(source).cloned();
            let (Some(mut copy), Some(&at)) = (copy, places.first()) else {
                return refuse("the emitter rule to copy is not one of the list");
            };
            (copy.at, copy.code, copy.name) = (at as u64, code, name.to_string());
            let place = list.emitters.partition_point(|e| e.code < code);
            list.emitters.insert(place, copy);
            for ev in list.effects.iter_mut().flat_map(|e| e.events.iter_mut()) {
                if ev.emitter >= place {
                    ev.emitter += 1;
                }
            }
            Ok(list)
        },
    )
}

fn emitter_copied(t: &mut Tail, v: &Bytes, from: usize, name: &str, code: u32) -> Result<usize> {
    const WHAT: &str = "emitter rule";
    let at = t.copy(v, WHAT, from, EMITTER_RULE)?;
    let text = t.alloc(name.len() + 1);
    t.put(text, name.as_bytes())?;
    t.ptr(at + 0x20, text)?;
    t.put(at + 0x1C, &code.to_le_bytes())?;
    // No event names the copy.
    t.put(at + 0x10, &1u32.to_le_bytes())?;
    for i in 0..EMITTER_KFPS.len() {
        keys_copied(t, v, from + 0x48 + KFP * i, at + 0x48 + KFP * i)?;
    }
    keyframe_list(t, v, from + 0x2C8, (from, EMITTER_RULE), at)?;
    for kind in 0..3 {
        if let Some(d) = v.ptr(WHAT, from + 0x30 + 8 * kind)? {
            let copied = domain_copied(t, v, d)?;
            t.ptr(at + 0x30 + 8 * kind, copied)?;
        }
    }
    Ok(at)
}

fn domain_copied(t: &mut Tail, v: &Bytes, from: usize) -> Result<usize> {
    const WHAT: &str = "domain";
    let len = match v.u32(WHAT, from + 0x18C)? {
        MESH => MESH_DOMAIN,
        _ => DOMAIN,
    };
    let at = t.copy(v, WHAT, from, len)?;
    for i in 0..DOMAIN_KFPS[0].len() {
        keys_copied(t, v, from + 8 + KFP * i, at + 8 + KFP * i)?;
    }
    keyframe_list(t, v, from + 0x198, (from, len), at)?;
    Ok(at)
}

/// The keyframe list whose head is at `head` in the record `(from, len)` laid out anew for the
/// record's copy at `to`, its capacity kept and each entry aimed at the copy's own value.
fn keyframe_list(
    t: &mut Tail,
    v: &Bytes,
    head: usize,
    (from, len): (usize, usize),
    to: usize,
) -> Result<()> {
    const WHAT: &str = "keyframe list";
    let (list, count, capacity) = array(v, WHAT, head, false)?;
    let held = t.alloc(8 * capacity);
    for i in 0..count {
        let k = v.to(WHAT, list + 8 * i)?;
        need(
            (from..from + len).contains(&k),
            "a keyframe list names a value its record does not hold",
        )?;
        t.ptr(held + 8 * i, to + (k - from))?;
    }
    t.ptr(to + (head - from), held)
}

/// The bytes of the behaviour `b` a copy takes: those the reader reads it from.
fn extent(list: &ParticleEffects, b: usize) -> Result<usize> {
    match list.behaviours.get(b) {
        Some(held) => Ok((held.size as usize).min(BEHAVIOUR_REACH)),
        None => refuse("a rule lists a behaviour the list does not hold"),
    }
}

/// A copy of the first `n` bytes of the list's behaviour `b`, with keyframed values of its own.
fn behaviour_copied(
    t: &mut Tail,
    v: &Bytes,
    list: &ParticleEffects,
    b: usize,
    n: usize,
) -> Result<usize> {
    let Some(held) = list.behaviours.get(b) else {
        return refuse("a rule lists a behaviour the list does not hold");
    };
    let from = held.at as usize;
    let at = t.copy(v, "behaviour", from, n)?;
    let read = Behaviour {
        size: n as u64,
        ..held.clone()
    };
    for k in super::writer::kfp_places(v, &read)? {
        keys_copied(t, v, k, at + (k - from))?;
    }
    Ok(at)
}

/// The behaviour `behaviour` copied into the particle rule `into`, at its class bit's place.
// The rule lists none of the behaviour's class; its class word and masks are given the bit.
pub fn copy_particle_behaviour(body: &mut Rpf9Sections, behaviour: usize, into: usize) -> Copied {
    let stored = read_particle_effects(&body.virtual_section, None)?;
    let n = extent(&stored, behaviour)?;
    let Some(bit) = stored.behaviours.get(behaviour).and_then(|b| b.class_bit) else {
        return Err(refused("a behaviour no rule lists has no class bit"));
    };
    append(
        body,
        |v, base, list| behaviour_into(v, base, list, (behaviour, n), into, bit),
        |mut list, places| {
            let copy = list.behaviours.get(behaviour).cloned();
            let (Some(mut copy), Some(&at)) = (copy, places.first()) else {
                return refuse("the behaviour to copy is not one of the list");
            };
            copy.at = at as u64;
            let index = list.behaviours.len();
            list.behaviours.push(copy);
            let Some(rule) = list.particles.get_mut(into) else {
                return refuse("the particle rule to copy into is not one of the list");
            };
            let place = below(rule.class_bits, bit).min(rule.behaviours.len());
            rule.behaviours.insert(place, index);
            rule.class_bits |= 1u32 << bit;
            Ok(list)
        },
    )
}

/// The number of bits of `word` below `bit`: the place a behaviour of that bit takes in a rule.
fn below(word: u32, bit: u8) -> usize {
    (word & ((1u32 << bit) - 1)).count_ones() as usize
}

fn behaviour_into(
    v: &Bytes,
    base: usize,
    list: &ParticleEffects,
    (behaviour, n): (usize, usize),
    into: usize,
    bit: u8,
) -> Result<Laid> {
    const WHAT: &str = "particle rule";
    let Some(rule) = list.particles.get(into) else {
        return refuse("the particle rule to copy into is not one of the list");
    };
    need(
        (rule.class_bits >> bit) & 1 == 0,
        "the rule lists a behaviour of that class bit already, and a rule lists one of each",
    )?;
    need(
        !DRAW_BITS.contains(&bit),
        "a sprite, trail or model behaviour follows its rule's draw type and is not copied",
    )?;
    let at = rule.at as usize;
    let (listed, count, _) = array(v, WHAT, at + 0x188, true)?;
    let place = below(rule.class_bits, bit).min(count);
    let mut t = Tail::new(base);
    // The rule's new list comes before the copy, which then reads up to the section's end.
    let held = t.alloc(8 * (count + 1));
    let (before, after) = v.get(WHAT, listed, 8 * count)?.split_at(8 * place);
    t.put(held, before)?;
    t.put(held + 8 * (place + 1), after)?;
    t.ptr(at + 0x188, held)?;
    counts(&mut t, at + 0x188, count + 1)?;
    for (o, w) in super::bits::class_words(rule.class_bits | (1u32 << bit)) {
        t.put(at + o, &w.to_le_bytes())?;
    }
    let copy = behaviour_copied(&mut t, v, list, behaviour, n)?;
    t.ptr(held + 8 * place, copy)?;
    Ok((t, vec![copy]))
}

/// What a particle rule's copy is laid out from.
struct RuleCopy<'a> {
    source: usize,
    name: &'a str,
    code: u32,
    /// Each listed behaviour's bytes, in the rule's order.
    extents: &'a [usize],
    /// Each GPU buffer's slot and where its copy's data starts in the physical section.
    data: &'a [(u16, usize)],
}

/// The list's particle rule `source` copied as `name`, with its own behaviours and GPU buffers.
// Each GPU buffer's data is copied past the physical section's end.
pub fn duplicate_particle_rule(body: &mut Rpf9Sections, source: usize, name: &str) -> Copied {
    named(name)?;
    let code = name_hash32(name.as_bytes());
    let stored = read_particle_effects(&body.virtual_section, None)?;
    let Some(rule) = stored.particles.get(source) else {
        return Err(refused("the particle rule to copy is not one of the list"));
    };
    let extents = (rule.behaviours.iter())
        .map(|&b| extent(&stored, b))
        .collect::<Result<Vec<_>>>()?;
    let buffers = read_particle_gpu_buffers(&body.virtual_section, Some(&body.physical[..]), rule)?;
    let at = rule.at as usize + RULE_FLOATS.0;
    let floats = body.virtual_section.get(at..at + RULE_FLOATS.1);
    let Some(floats) = floats.map(<[u8]>::to_vec) else {
        return Err(refused("a rule's floats lie past the section's end"));
    };
    let mut grown = body.clone();
    let data = gpu_data(&mut grown, &buffers, &floats)?;
    let copy = RuleCopy {
        source,
        name,
        code,
        extents: &extents,
        data: &data,
    };
    append(
        &mut grown,
        |v, base, list| rule_copied(v, base, list, &copy),
        |list, places| rule_expected(list, places, &copy),
    )?;
    let read = read_particle_effects(&grown.virtual_section, None)?;
    let from = read.particles.iter().find(|p| p.name == rule.name);
    let to = read.particles.iter().find(|p| p.code == code);
    let (Some(from), Some(to)) = (from, to) else {
        return Err(refused("the copied rule does not read back"));
    };
    own_buffers(&grown, from, to)?;
    *body = grown;
    Ok(())
}

fn rule_copied(v: &Bytes, base: usize, list: &ParticleEffects, c: &RuleCopy) -> Result<Laid> {
    const WHAT: &str = "particle rule";
    let Some(rule) = list.particles.get(c.source) else {
        return refuse("the particle rule to copy is not one of the list");
    };
    need(
        c.extents.len() == rule.behaviours.len(),
        "a copied rule's behaviours are not the ones it lists",
    )?;
    let from = rule.at as usize;
    let mut t = Tail::new(base);
    let at = t.copy(v, WHAT, from, PARTICLE_RULE)?;
    let text = t.alloc(c.name.len() + 1);
    t.put(text, c.name.as_bytes())?;
    t.ptr(at + 0x178, text)?;
    // No event names the copy.
    t.put(at + 0x10, &1u32.to_le_bytes())?;
    // The copied record's curve heads still aim at the source's key arrays; give the copy its own.
    super::tables::own_particle_curves(&mut t, v, from, at)?;
    // Shader words, drawable references and the drawables' runtime slots; the drawables' floats
    // are pooled, rules sharing runs of them, so the copy shares its source's.
    for (head, stride) in [(0x370, 8), (0x3B0, 0x20), (0x3C0, 8)] {
        t.elements(v, WHAT, from + head, at + head, stride)?;
    }
    for &(slot, data) in c.data {
        let slot = usize::from(slot);
        let object = v.to(WHAT, from + slot)?;
        let object = t.copy(v, "particle GPU buffer", object, GPU_OBJECT)?;
        t.put(object + 0x18, &(PHYSICAL + data as u64).to_le_bytes())?;
        t.ptr(at + slot, object)?;
    }
    dictionary_anew(&mut t, v, PARTICLE_DICTIONARY, c.code, at)?;
    // The behaviours come last, each reading up to the next or the section's end.
    let held = t.alloc(8 * rule.behaviours.len());
    t.ptr(at + 0x188, held)?;
    let mut places = vec![at];
    for (i, (&b, &n)) in rule.behaviours.iter().zip(c.extents).enumerate() {
        let copied = behaviour_copied(&mut t, v, list, b, n)?;
        t.ptr(held + 8 * i, copied)?;
        places.push(copied);
    }
    Ok((t, places))
}

fn rule_expected(
    mut list: ParticleEffects,
    places: &[usize],
    c: &RuleCopy,
) -> Result<ParticleEffects> {
    let copy = list.particles.get(c.source).cloned();
    let (Some(mut copy), Some((&at, copies))) = (copy, places.split_first()) else {
        return refuse("the particle rule to copy is not one of the list");
    };
    need(
        copies.len() == copy.behaviours.len(),
        "a copied rule's behaviours are not the ones it lists",
    )?;
    for (listed, &b_at) in copy.behaviours.iter_mut().zip(copies) {
        let Some(mut held) = list.behaviours.get(*listed).cloned() else {
            return refuse("a rule lists a behaviour the list does not hold");
        };
        held.at = b_at as u64;
        *listed = list.behaviours.len();
        list.behaviours.push(held);
    }
    (copy.at, copy.code, copy.name) = (at as u64, c.code, c.name.to_string());
    let place = list.particles.partition_point(|p| p.code < c.code);
    list.particles.insert(place, copy);
    for ev in list.effects.iter_mut().flat_map(|e| e.events.iter_mut()) {
        if ev.particle >= place {
            ev.particle += 1;
        }
    }
    Ok(list)
}

/// Data of its own for each of `buffers`, copied past the physical section's end, each 256-aligned
/// and all in one 64 KiB block, the `+0x390` copy derived again from the rule's `floats`: each
/// buffer's slot and where its copy's data starts.
fn gpu_data(
    body: &mut Rpf9Sections,
    buffers: &[GpuBuffer],
    floats: &[u8],
) -> std::result::Result<Vec<(u16, usize)>, ParticleKeysError> {
    if buffers.is_empty() {
        return Ok(Vec::new());
    }
    let laid = |from: usize| {
        let mut end = from;
        let places: Vec<usize> = (buffers.iter())
            .map(|b| {
                let at = end.next_multiple_of(GPU_ALIGN);
                end = at + b.len as usize;
                at
            })
            .collect();
        (places, end.next_multiple_of(16))
    };
    let mut at = body.physical.len();
    let (mut places, mut end) = laid(at);
    if at / PHYSICAL_BLOCK != (end.max(at + 1) - 1) / PHYSICAL_BLOCK {
        let block = at.next_multiple_of(PHYSICAL_BLOCK);
        let pad = (block - at) as isize;
        grow_section(body, &[0x08], Rpf9Section::Physical, at, pad)?;
        at = block;
        (places, end) = laid(at);
        need(
            end - at <= PHYSICAL_BLOCK,
            "a copied rule's GPU buffers do not fit one 64 KiB physical block",
        )?;
    }
    let by = (end - at) as isize;
    grow_section(body, &[0x08], Rpf9Section::Physical, at, by)?;
    for (b, &to) in buffers.iter().zip(&places) {
        let (from, len) = (b.data_at as usize, b.len as usize);
        let held = body.physical.get(from..from + len).map(<[u8]>::to_vec);
        let (Some(held), Some(slot)) = (held, body.physical.get_mut(to..to + len)) else {
            return Err(refused("a GPU buffer's data lies past its section"));
        };
        slot.copy_from_slice(&held);
        if b.slot == FLOATS_SLOT {
            match slot.get_mut(..floats.len()) {
                Some(copied) => copied.copy_from_slice(floats),
                None => refuse("a rule's +0x390 GPU buffer is shorter than its floats")?,
            }
        }
    }
    Ok(buffers.iter().map(|b| b.slot).zip(places).collect())
}

/// Refused unless the rule `to`, a copy of `from`, points at GPU buffers of its own whose data is
/// its source's.
fn own_buffers(s: &Rpf9Sections, from: &ParticleRule, to: &ParticleRule) -> Result<()> {
    let ours = read_particle_gpu_buffers(&s.virtual_section, Some(&s.physical[..]), from)?;
    let theirs = read_particle_gpu_buffers(&s.virtual_section, Some(&s.physical[..]), to)?;
    let data = |b: &GpuBuffer| {
        let at = b.data_at as usize;
        s.physical.get(at..at + b.len as usize)
    };
    need(
        ours.len() == theirs.len(),
        "a copied rule does not point at as many GPU buffers as its source",
    )?;
    for (a, b) in ours.iter().zip(&theirs) {
        let own = (a.slot, a.words, a.len) == (b.slot, b.words, b.len)
            && a.at != b.at
            && a.data_at != b.data_at
            && (b.data_at as usize).is_multiple_of(GPU_ALIGN);
        need(
            own,
            "a copied rule's GPU buffer is not its own, or its data is not 256-aligned",
        )?;
        need(
            data(a).is_some() && data(a) == data(b),
            "a copied rule's GPU buffer does not hold its source's data",
        )?;
    }
    Ok(())
}

#[path = "particles_event_object.rs"]
pub(super) mod event_object;
