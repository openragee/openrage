//! An effect rule copied under a new name past a particle effects list's virtual section's end:
//! the rule, its name, keyframed values, events and evolution lists laid out anew, the dictionary's
//! two arrays laid out anew in ascending code order, each event's +0x60 object its own with its
//! buffer's data past the physical section's end, and each rule an event names counted once more.
//! Nothing already in the section moves, so the words no reader types stay valid.

use super::dup::event_object::{data_copied, object_copied, own_copy};
use super::*;
use crate::container::rpf::{grow_section, Rpf9Section, Rpf9Sections};

/// The word that addresses virtual offset 0.
const VIRTUAL: u64 = 0x5000_0000;
const EFFECT_RULE: usize = 0x240;
const EVENT: usize = 0x88;
const EVOLUTION_LIST: usize = 0x38;

/// The list's effect rule `source` copied as `name` past the section's end, or refused unchanged.
pub fn duplicate_particle_effect(
    body: &mut Rpf9Sections,
    source: usize,
    name: &str,
) -> std::result::Result<(), ParticleKeysError> {
    let stored = read_particle_effects(&body.virtual_section, None)?;
    need(
        !name.is_empty() && name.len() <= 512 && name.bytes().all(|b| (32..127).contains(&b)),
        "a name is not printable ASCII of at most 512 bytes",
    )?;
    let events = stored.effects.get(source).map_or(0, |e| e.events.len());
    let mut grown = body.clone();
    // Each event's object's buffer is given data first, which may grow the block map: read again.
    let data = (0..events)
        .map(|event| data_copied(&mut grown, source, event))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let stored = read_particle_effects(&grown.virtual_section, None)?;
    let end = grown.virtual_section.len().next_multiple_of(16);
    let (dry, _) = sections(&grown.virtual_section, None, |v, _| {
        laid_out(&v, end, &stored, (source, &data), name)
    })?;
    let len = dry.bytes.len().next_multiple_of(16);
    grown.virtual_section.resize(end, 0);
    let start = grow_section(&mut grown, &[0x08], Rpf9Section::Virtual, end, len as isize)?;
    // The block map's slots may have grown, which moves what follows them: read the list again.
    let moved = read_particle_effects(&grown.virtual_section, None)?;
    need(
        remover::sizes_cleared(moved.clone()) == remover::sizes_cleared(stored),
        "growing the section changes how what it holds reads, such as its last behaviour",
    )?;
    let (tail, rule) = sections(&grown.virtual_section, None, |v, _| {
        laid_out(&v, start, &moved, (source, &data), name)
    })?;
    let v = &mut grown.virtual_section;
    for (at, bytes) in std::iter::once((start, tail.bytes)).chain(tail.patches) {
        match v.get_mut(at..at + bytes.len()) {
            Some(slot) => slot.copy_from_slice(&bytes),
            None => refuse("a copied word lies past the section's end")?,
        }
    }
    let code = name_hash32(name.as_bytes());
    let mut expected = moved.clone();
    let Some(mut copy) = moved.effects.get(source).cloned() else {
        return Err(
            HeaderlessError::Particles("the effect rule to copy is not one of the list").into(),
        );
    };
    (copy.at, copy.code, copy.name) = (rule as u64, code, name.to_string());
    let place = expected.effects.partition_point(|e| e.code < code);
    expected.effects.insert(place, copy);
    for s in expected
        .particles
        .iter_mut()
        .flat_map(|p| p.spawners.iter_mut())
    {
        if let Some(e) = s.effect.as_mut().filter(|e| **e >= place) {
            *e += 1;
        }
    }
    let read = read_particle_effects(&grown.virtual_section, None)?;
    need(
        remover::sizes_cleared(read.clone()) == remover::sizes_cleared(expected),
        "the copy does not read back as its source under its new name, or changes what else the list holds",
    )?;
    let from = source + usize::from(source >= place);
    for event in 0..events {
        own_copy(&grown, &read, (from, event), (place, event))?;
    }
    *body = grown;
    Ok(())
}

/// Bytes laid out from `base` on, each allocation 16-aligned, and the words written below `base`.
pub(super) struct Tail {
    pub(super) base: usize,
    pub(super) bytes: Vec<u8>,
    pub(super) patches: Vec<(usize, Vec<u8>)>,
}

impl Tail {
    pub(super) fn new(base: usize) -> Self {
        Self {
            base,
            bytes: Vec::new(),
            patches: Vec::new(),
        }
    }

    pub(super) fn alloc(&mut self, len: usize) -> usize {
        let at = self.bytes.len().next_multiple_of(16);
        self.bytes.resize(at + len, 0);
        self.base + at
    }

    pub(super) fn put(&mut self, at: usize, bytes: &[u8]) -> Result<()> {
        let Some(o) = at.checked_sub(self.base) else {
            self.patches.push((at, bytes.to_vec()));
            return Ok(());
        };
        match self.bytes.get_mut(o..o + bytes.len()) {
            Some(slot) => {
                slot.copy_from_slice(bytes);
                Ok(())
            }
            None => refuse("a copied word lies past what was laid out"),
        }
    }

    pub(super) fn ptr(&mut self, at: usize, to: usize) -> Result<()> {
        self.put(at, &(VIRTUAL + to as u64).to_le_bytes())
    }

    /// The `len` bytes at `from` copied to a place of their own.
    pub(super) fn copy(
        &mut self,
        v: &Bytes,
        what: &'static str,
        from: usize,
        len: usize,
    ) -> Result<usize> {
        let held = v.get(what, from, len)?;
        let at = self.alloc(len);
        self.put(at, held)?;
        Ok(at)
    }

    /// The elements of the array whose head is at `head` copied, the head at `to` aimed at them;
    /// where they are, or `None` for an array of none.
    pub(super) fn elements(
        &mut self,
        v: &Bytes,
        what: &'static str,
        head: usize,
        to: usize,
        stride: usize,
    ) -> Result<Option<usize>> {
        let (from, count, _) = array(v, what, head, true)?;
        if count == 0 {
            return Ok(None);
        }
        let at = self.copy(v, what, from, stride * count)?;
        self.ptr(to, at)?;
        Ok(Some(at))
    }
}

/// The copy of the effect rule at `source` of `list` named `name` laid out from `base`, each
/// event's object's buffer data at the place `data` gives, with the dictionary and reference count
/// words it changes; and where the copy is.
fn laid_out(
    v: &Bytes,
    base: usize,
    list: &ParticleEffects,
    (source, data): (usize, &[Option<usize>]),
    name: &str,
) -> Result<(Tail, usize)> {
    let Some(from) = list.effects.get(source) else {
        return refuse("the effect rule to copy is not one of the list");
    };
    let mut t = Tail {
        base,
        bytes: Vec::new(),
        patches: Vec::new(),
    };
    let rule = effect(&mut t, v, (from, data), name)?;
    dictionary(&mut t, v, 0x28, name_hash32(name.as_bytes()), rule)?;
    let mut named: BTreeMap<u64, u32> = BTreeMap::new();
    for ev in &from.events {
        let rules = [
            list.emitters.get(ev.emitter).map(|r| r.at),
            list.particles.get(ev.particle).map(|r| r.at),
        ];
        for at in rules {
            let Some(at) = at else {
                return refuse("an event names a rule the list does not hold");
            };
            *named.entry(at).or_default() += 1;
        }
    }
    for (at, n) in named {
        let at = at as usize + 0x10;
        let Some(count) = v.u32("reference count", at)?.checked_add(n) else {
            return refuse("a rule's reference count would pass its word");
        };
        t.put(at, &count.to_le_bytes())?;
    }
    Ok((t, rule))
}

/// The keys of the keyframed value at `from` copied for the one at `to`.
pub(super) fn kfp(t: &mut Tail, v: &Bytes, from: usize, to: usize) -> Result<()> {
    t.elements(v, "keyframed value", from + 8, to + 8, 0x20)?;
    Ok(())
}

fn effect(
    t: &mut Tail,
    v: &Bytes,
    (rule, data): (&EffectRule, &[Option<usize>]),
    name: &str,
) -> Result<usize> {
    const WHAT: &str = "effect rule";
    let from = rule.at as usize;
    let at = t.copy(v, WHAT, from, EFFECT_RULE)?;
    let text = t.alloc(name.len() + 1);
    t.put(text, name.as_bytes())?;
    t.ptr(at + 0x20, text)?;
    for i in 0..EFFECT_KFPS.len() {
        kfp(t, v, from + 0xC0 + KFP * i, at + 0xC0 + KFP * i)?;
    }
    let (list, count, capacity) = array(v, WHAT, from + 0x200, false)?;
    let held = t.alloc(8 * capacity);
    for i in 0..count {
        let k = v.to(WHAT, list + 8 * i)?;
        need(
            (from..from + EFFECT_RULE).contains(&k),
            "an effect rule's keyframe list names a value it does not hold",
        )?;
        t.ptr(held + 8 * i, at + (k - from))?;
    }
    t.ptr(at + 0x200, held)?;
    if let Some(l) = v.ptr(WHAT, from + 0x48)? {
        let copied = evolution_list(t, v, l)?;
        t.ptr(at + 0x48, copied)?;
    }
    let (events, count, _) = array(v, WHAT, from + 0x38, false)?;
    // The copy's event array holds exactly its events.
    let n = (count as u16).to_le_bytes();
    t.put(at + 0x38 + 8, &[n, n].concat())?;
    if count == 0 {
        t.put(at + 0x38, &[0; 8])?;
        return Ok(at);
    }
    let held = t.alloc(8 * count);
    t.ptr(at + 0x38, held)?;
    for i in 0..count {
        let e = v.to("event", events + 8 * i)?;
        let Some(ev) = rule.events.get(i) else {
            return refuse("an effect rule's event array holds more events than its list reads");
        };
        let copied = t.copy(v, "event", e, EVENT)?;
        object_copied(t, v, (e, ev), copied, data.get(i).copied().flatten())?;
        t.ptr(held + 8 * i, copied)?;
        if let Some(l) = v.ptr("event", e + 0x30)? {
            let list = evolution_list(t, v, l)?;
            t.ptr(copied + 0x30, list)?;
        }
    }
    Ok(at)
}

/// The evolution list at `from` copied with its evolutions, values, keyframes and map; where the
/// copy is.
pub(super) fn evolution_list(t: &mut Tail, v: &Bytes, from: usize) -> Result<usize> {
    const WHAT: &str = "evolution list";
    let at = t.copy(v, WHAT, from, EVOLUTION_LIST)?;
    t.elements(v, WHAT, from, at, 0x18)?;
    let (props, count, _) = array(v, WHAT, from + 0x10, true)?;
    let mut moved = BTreeMap::new();
    if let Some(copied) = t.elements(v, WHAT, from + 0x10, at + 0x10, 0x18)? {
        for k in 0..count {
            let (old, new) = (props + 0x18 * k, copied + 0x18 * k);
            moved.insert(old, new);
            let (keys, n, _) = array(v, WHAT, old, true)?;
            if let Some(kept) = t.elements(v, WHAT, old, new, 0x30)? {
                for m in 0..n {
                    t.elements(v, WHAT, keys + 0x30 * m, kept + 0x30 * m, 0x20)?;
                }
            }
        }
    }
    map(t, v, from + 0x20, at + 0x20, &moved)?;
    Ok(at)
}

/// The map whose head is at `from` copied for the head at `to`, node by node in each chain's
/// order, each node's data aimed at the copy `moved` gives it.
fn map(
    t: &mut Tail,
    v: &Bytes,
    from: usize,
    to: usize,
    moved: &BTreeMap<usize, usize>,
) -> Result<()> {
    const WHAT: &str = "evolution list";
    let (slots, used) = (
        v.u32(WHAT, from + 8)? as usize,
        v.u32(WHAT, from + 0x10)? as usize,
    );
    let Some(buckets) = v.ptr(WHAT, from)? else {
        return Ok(());
    };
    v.get(WHAT, buckets, 8 * slots)?;
    let heads = t.alloc(8 * slots);
    t.ptr(to, heads)?;
    let mut hops = 0;
    for slot in 0..slots {
        let mut node = v.ptr(WHAT, buckets + 8 * slot)?;
        let mut link = heads + 8 * slot;
        while let Some(n) = node {
            hops += 1;
            need(hops <= used, "a map's chains are longer than its key count")?;
            let copy = t.copy(v, WHAT, n, 0x20)?;
            t.ptr(link, copy)?;
            if let Some(d) = v.ptr(WHAT, n + 8)? {
                let Some(&d) = moved.get(&d) else {
                    return refuse("an evolution list's map names a value the list does not hold");
                };
                t.ptr(copy + 8, d)?;
            }
            t.put(copy + 0x10, &[0; 8])?;
            link = copy + 0x10;
            node = v.ptr(WHAT, n + 0x10)?;
        }
    }
    Ok(())
}

/// The codes and entries of the dictionary the root's `slot` points at laid out anew with `code`
/// for the rule at `rule` in its ascending place, each array as long as its count.
pub(super) fn dictionary(
    t: &mut Tail,
    v: &Bytes,
    slot: usize,
    code: u32,
    rule: usize,
) -> Result<()> {
    const WHAT: &str = "rule dictionary";
    let d = v.to(WHAT, slot)?;
    let (codes, n, _) = array(v, WHAT, d + 0x20, true)?;
    let (entries, _, _) = array(v, WHAT, d + 0x30, true)?;
    need(
        n < usize::from(u16::MAX),
        "a dictionary would hold more rules than its count can state",
    )?;
    let mut held: Vec<u32> = (0..n)
        .map(|i| v.u32(WHAT, codes + 4 * i))
        .collect::<Result<_>>()?;
    let mut words: Vec<u64> = (0..n)
        .map(|i| v.u64(WHAT, entries + 8 * i))
        .collect::<Result<_>>()?;
    need(
        !held.contains(&code),
        "a rule of that name's hash is in its dictionary",
    )?;
    let place = held.partition_point(|c| *c < code);
    held.insert(place, code);
    words.insert(place, VIRTUAL + rule as u64);
    let codes = t.alloc(4 * held.len());
    let entries = t.alloc(8 * words.len());
    let codes_bytes: Vec<u8> = held.iter().flat_map(|c| c.to_le_bytes()).collect();
    let entry_bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    t.put(codes, &codes_bytes)?;
    t.put(entries, &entry_bytes)?;
    let count = (held.len() as u16).to_le_bytes();
    for (head, to) in [(d + 0x20, codes), (d + 0x30, entries)] {
        t.ptr(head, to)?;
        t.put(head + 8, &[count, count].concat())?;
    }
    Ok(())
}
