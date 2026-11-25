//! The object an event's +0x60 names, copied with its event: a 0x90-byte record of a 1,024-bit
//! set, the pointer to its `evoTexture` GPU buffer and six counts, the buffer object, its two
//! records and its data each laid out anew.

use super::*;

const WHAT: &str = "event object";
/// The word that addresses virtual offset 0.
const VIRTUAL: u64 = 0x5000_0000;
/// The record's size, the end of its bit set, where it points at its buffer and its counts.
const OBJECT: usize = 0x90;
const BITS: usize = 0x80;
const BUFFER: usize = 0x80;
const COUNTS: usize = 0x88;
/// A buffer object with the 0x20-byte record its +0x28 names right after it.
const BUFFER_OBJECT: usize = 0x70;
/// The u64 at +0x10 of that record; its other 24 bytes are 0.
const RECORD: u64 = 0xFFFF_FFFF_FFFF_0241;
/// The buffer object's words at +0x0C, +0x0E and +0x10.
const WORDS: (u16, u16, u32) = (8, 8, 0x980);
/// Its words at +0x38, +0x40 and +0x48, the row count at +0x3A left 0.
const TAIL: [u64; 3] = [
    0x0A01_0001_0000_0010,
    0x0001_0000_0001_0000,
    0x0002_0000_0000,
];
/// The record its +0x30 names: 8 zero bytes, the name, zeros.
const NAME: &[u8] = b"evoTexture";
const NAME_RECORD: usize = 0x18;
/// The buffer's data is whole rows of this many bytes.
const ROW: usize = 128;

/// An event's object: its place, and its buffer object's place, data place and data length.
#[derive(Clone, Copy)]
struct Object {
    at: usize,
    buffer: Option<(usize, usize, usize)>,
}

fn name_record() -> [u8; NAME_RECORD] {
    let mut r = [0; NAME_RECORD];
    r[8..8 + NAME.len()].copy_from_slice(NAME);
    r
}

/// The 0x50 bytes a buffer object of `n` 4-byte words at `at` holds, with its data and name words.
fn made(n: u32, data: u64, at: usize, name: u64) -> Vec<u8> {
    let words = [
        0,
        u64::from(n) | (u64::from(WORDS.0) << 32) | (u64::from(WORDS.1) << 48),
        u64::from(WORDS.2),
        data,
        0,
        VIRTUAL + (at + 0x50) as u64,
        name,
        TAIL[0] | (u64::from(n / 32) << 16),
        TAIL[1],
        TAIL[2],
    ];
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// The place of the event `event` of the effect rule `effect`, and the event as read.
fn event_at<'a>(
    v: &Bytes,
    list: &'a ParticleEffects,
    effect: usize,
    event: usize,
) -> Result<(usize, &'a Event)> {
    let Some(rule) = list.effects.get(effect) else {
        return refuse("the effect rule is not one of the list");
    };
    let Some(ev) = rule.events.get(event) else {
        return refuse("the event to copy is not one of its effect rule's");
    };
    let (events, _, _) = array(v, WHAT, rule.at as usize + 0x38, false)?;
    Ok((v.to(WHAT, events + 8 * event)?, ev))
}

/// The object the event `ev` at `from` names, refused where it breaks a measured rule.
fn object(v: &Bytes, from: usize, ev: &Event) -> Result<Option<Object>> {
    let Some(at) = v.ptr(WHAT, from + EVENT_OBJECT)? else {
        return Ok(None);
    };
    need(
        at.is_multiple_of(16),
        "an event's +0x60 object is not 16-aligned",
    )?;
    let raw = v.get(WHAT, at, OBJECT)?;
    let evolutions = ev.evolution_list.as_ref().map(|l| l.evolutions.len());
    need(
        evolutions == Some(usize::from(raw[COUNTS])),
        "an event's +0x60 object does not count its event's evolutions",
    )?;
    need(
        raw[OBJECT - 2..].iter().all(|b| *b == 0),
        "an event's +0x60 object sets its last two bytes",
    )?;
    let pointer = |w: &[u8; 8]| {
        let w = u64_at(w, 0);
        w >> 32 == 0 && matches!(w >> 28, 5 | 6)
    };
    let mut words =
        (raw[..BITS].as_chunks::<8>().0.iter()).chain(raw[COUNTS..].as_chunks::<8>().0.iter());
    need(
        !words.any(pointer),
        "an event's +0x60 object holds a pointer other than at +0x80",
    )?;
    let set = raw[..BITS].iter().any(|b| *b != 0);
    let buffer = v.ptr(WHAT, at + BUFFER)?;
    need(
        set == buffer.is_some(),
        "an event's +0x60 object points at a buffer while its bit set is empty, or the reverse",
    )?;
    let Some(b) = buffer else {
        return Ok(Some(Object { at, buffer: None }));
    };
    let n = v.u32(WHAT, b + 8)?;
    let (data, name) = (v.u64(WHAT, b + 0x18)?, v.u64(WHAT, b + 0x30)?);
    need(
        v.get(WHAT, b, 0x50)? == made(n, data, b, name).as_slice(),
        "an event's +0x60 buffer object is not the one its length, data and place make",
    )?;
    let record = v.get(WHAT, b + 0x50, 0x20)?;
    need(
        u64_at(record, 0) | u64_at(record, 8) | u64_at(record, 0x18) == 0
            && u64_at(record, 0x10) == RECORD,
        "an event's +0x60 buffer object is not followed by the record its +0x28 names",
    )?;
    let text = v.to(WHAT, b + 0x30)?;
    need(
        text.is_multiple_of(8) && v.get(WHAT, text, NAME_RECORD)? == name_record().as_slice(),
        "an event's +0x60 buffer object does not name an evoTexture record",
    )?;
    let Some(len) = (n as usize).checked_mul(4) else {
        return refuse("an event's +0x60 buffer's length does not fit");
    };
    let data = v.phys(WHAT, b + 0x18)?;
    need(
        len.is_multiple_of(ROW) && data.is_multiple_of(GPU_ALIGN),
        "an event's +0x60 buffer's data is not 256-aligned whole 128-byte rows",
    )?;
    Ok(Some(Object {
        at,
        buffer: Some((b, data, len)),
    }))
}

/// Data of its own for the buffer of the object the event `event` of the effect rule `effect`
/// names, copied past the physical section's end; where it starts, or `None` with no buffer.
pub(in super::super) fn data_copied(
    body: &mut Rpf9Sections,
    effect: usize,
    event: usize,
) -> std::result::Result<Option<usize>, ParticleKeysError> {
    let list = read_particle_effects(&body.virtual_section, None)?;
    let held = sections(&body.virtual_section, Some(&body.physical[..]), |v, p| {
        let (from, ev) = event_at(&v, &list, effect, event)?;
        match object(&v, from, ev)?.and_then(|o| o.buffer) {
            Some((at, data, len)) => p.get(WHAT, data, len).map(|_| Some((at, data, len))),
            None => Ok(None),
        }
    })?;
    let Some((at, data, len)) = held else {
        return Ok(None);
    };
    let buffer = GpuBuffer {
        slot: BUFFER as u16,
        at: at as u64,
        words: WORDS,
        data_at: data as u64,
        len: len as u64,
    };
    let placed = gpu_data(body, &[buffer], &[])?;
    Ok(placed.first().map(|&(_, to)| to))
}

/// The object the event `ev` at `from` names laid out for its copy at `copy`, the copy's +0x60
/// aimed at it; its buffer's data is at `data`.
pub(in super::super) fn object_copied(
    t: &mut Tail,
    v: &Bytes,
    (from, ev): (usize, &Event),
    copy: usize,
    data: Option<usize>,
) -> Result<()> {
    let Some(o) = object(v, from, ev)? else {
        return Ok(());
    };
    let at = t.copy(v, WHAT, o.at, OBJECT)?;
    t.ptr(copy + EVENT_OBJECT, at)?;
    let Some((b, _, _)) = o.buffer else {
        return Ok(());
    };
    let Some(data) = data else {
        return refuse("an event's +0x60 buffer was given no data of its own");
    };
    let buffer = t.copy(v, WHAT, b, BUFFER_OBJECT)?;
    let text = t.alloc(NAME_RECORD);
    t.put(text, &name_record())?;
    t.put(buffer + 0x18, &(PHYSICAL + data as u64).to_le_bytes())?;
    t.ptr(buffer + 0x28, buffer + 0x50)?;
    t.ptr(buffer + 0x30, text)?;
    t.ptr(at + BUFFER, buffer)
}

/// Refused unless the effect rule `effect`'s last event, a copy of its event `event`, holds an
/// object of its own that reads by the same rules, its buffer's data its own and its source's.
pub(super) fn copy_holds(body: &Rpf9Sections, effect: usize, event: usize) -> Result<()> {
    let list = read_particle_effects(&body.virtual_section, None)?;
    let count = list.effects.get(effect).map_or(0, |r| r.events.len());
    own_copy(
        body,
        &list,
        (effect, event),
        (effect, count.saturating_sub(1)),
    )
}

/// Refused unless the event `copy` of `list`, a copy of the event `event` of the effect rule
/// `effect`, holds an object of its own that reads by the same rules, its buffer's data its own
/// and its source's; `copy` is an effect rule and an event's place in it.
pub(in super::super) fn own_copy(
    body: &Rpf9Sections,
    list: &ParticleEffects,
    (effect, event): (usize, usize),
    copy: (usize, usize),
) -> Result<()> {
    sections(&body.virtual_section, Some(&body.physical[..]), |v, p| {
        let (from, ev) = event_at(&v, list, effect, event)?;
        let (to, copied) = event_at(&v, list, copy.0, copy.1)?;
        let (theirs, ours) = (object(&v, from, ev)?, object(&v, to, copied)?);
        let data = |o: Option<Object>| match o.and_then(|o| o.buffer) {
            Some((_, at, len)) => p.get(WHAT, at, len).map(Some),
            None => Ok(None),
        };
        let own = match (theirs, ours) {
            (Some(a), Some(b)) => match (a.buffer, b.buffer) {
                (Some(x), Some(y)) => a.at != b.at && x.0 != y.0 && x.1 != y.1,
                (None, None) => a.at != b.at,
                _ => false,
            },
            (None, None) => true,
            _ => false,
        };
        need(
            own && data(theirs)? == data(ours)?,
            "an event's copy does not hold a +0x60 object of its own equal to its source's",
        )
    })
}
