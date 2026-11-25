//! A particle effects list's values written back over its virtual section in place: rule scalars,
//! flags, keyframed values that keep or lose keys, and an event's rules, with the reference counts
//! that follow. Nothing moves, so a rule, an event or a behaviour added is refused; a key added
//! goes through `add_particle_keys` first, and a rule, an event or a behaviour removed through
//! `remove_from_particle_effects`.

use super::*;

/// The word that addresses virtual offset 0.
const VIRTUAL: u64 = 0x5000_0000;

/// `edited` written in place over the section it was read from, or refused, writing nothing.
pub fn write_particle_effects(virtual_section: &mut [u8], edited: &ParticleEffects) -> Result<()> {
    let stored = read_particle_effects(virtual_section, None)?;
    let edited = &derived(&stored, edited);
    let patches = sections(virtual_section, None, |v, _| plan(v, &stored, edited))?;
    let mut out = virtual_section.to_vec();
    for (at, bytes) in patches {
        match out.get_mut(at..at + bytes.len()) {
            Some(slot) => slot.copy_from_slice(&bytes),
            None => return refuse("a value to write is past the section's end"),
        }
    }
    need(
        read_particle_effects(&out, None)? == *edited,
        "an edit changes what is not written in place: a name, a link, a type or a count",
    )?;
    virtual_section.copy_from_slice(&out);
    Ok(())
}

/// Bytes to write, each at its section offset.
type Patches = Vec<(usize, Vec<u8>)>;

/// The writes of one list, each checked to fit the section.
struct Plan<'a> {
    v: Bytes<'a>,
    out: Patches,
}

fn plan(v: Bytes, stored: &ParticleEffects, edited: &ParticleEffects) -> Result<Patches> {
    let rules = stored.effects.len() == edited.effects.len()
        && stored.emitters.len() == edited.emitters.len()
        && stored.particles.len() == edited.particles.len();
    need(
        rules,
        "a rule was added or removed: a dictionary holds exactly as many codes and entries as it has room for, and a new rule's words of unknown meaning have no stated value, so no rule says what the list holds after it",
    )?;
    need(
        stored.behaviours.len() == edited.behaviours.len(),
        "a behaviour was added or removed: a behaviour with no named keyframed value has no stated class, and a sprite rule's last behaviour is known only by its place, so no rule says what the list holds after it",
    )?;
    let mut p = Plan { v, out: vec![] };
    for (s, e) in stored.effects.iter().zip(&edited.effects) {
        p.effect(s, e)?;
        p.relink(s, e, stored)?;
    }
    p.reference_counts(stored, edited)?;
    for (s, e) in stored.emitters.iter().zip(&edited.emitters) {
        p.emitter(s, e)?;
    }
    for (s, e) in stored.particles.iter().zip(&edited.particles) {
        p.particle(s, e)?;
    }
    for (s, e) in stored.behaviours.iter().zip(&edited.behaviours) {
        let places = kfp_places(&p.v, s)?;
        need(
            places.len() == s.kfps.len() && e.kfps.len() == s.kfps.len(),
            "a behaviour's keyframed values were added or removed, which would move the sections",
        )?;
        for ((at, sk), ek) in places.into_iter().zip(&s.kfps).zip(&e.kfps) {
            p.kfp(at, sk, ek)?;
        }
    }
    let counts = super::colour_word::patches(&stored.behaviours, &edited.behaviours);
    p.out.extend(counts);
    let fields = super::fields_word::patches(&stored.behaviours, &edited.behaviours)?;
    p.out.extend(fields);
    Ok(p.out)
}

/// Where a behaviour's keyframed values are, found as the reader finds them.
pub(super) fn kfp_places(v: &Bytes, b: &Behaviour) -> Result<Vec<usize>> {
    let Some(class) = b.class else {
        return Ok(Vec::new());
    };
    let ids: Vec<u32> = (BEHAVIOUR_KFPS.iter().chain(&UNNAMED_KFPS))
        .filter(|(_, name)| name.split(':').next() == Some(class))
        .map(|(id, _)| *id)
        .collect();
    let at = b.at as usize;
    let end = (at + b.size as usize).min(at + BEHAVIOUR_REACH);
    let mut places = Vec::new();
    let mut o = at + 8;
    while o + KFP <= end {
        if ids.contains(&v.u32("behaviour", o + 0x28)?) && kfp(v, o, None).is_ok() {
            places.push(o);
            o += KFP;
        } else {
            o += 8;
        }
    }
    Ok(places)
}

/// `edited` with each key's step reciprocal, `time.y`, derived again where the key before it is
/// not the one it followed in `stored`: 0 for the first key and for a step of 0.
pub(super) fn derived(stored: &ParticleEffects, edited: &ParticleEffects) -> ParticleEffects {
    let mut out = edited.clone();
    for (s, e) in stored.effects.iter().zip(&mut out.effects) {
        for (sk, ek) in s.kfps.iter().zip(&mut e.kfps) {
            steps(sk, ek);
        }
    }
    for (s, e) in stored.emitters.iter().zip(&mut out.emitters) {
        for (sk, ek) in s.kfps.iter().zip(&mut e.kfps) {
            steps(sk, ek);
        }
        for (sd, ed) in s.domains.iter().zip(&mut e.domains) {
            if let (Some(sd), Some(ed)) = (sd, ed) {
                for (sk, ek) in sd.kfps.iter().zip(&mut ed.kfps) {
                    steps(sk, ek);
                }
            }
        }
    }
    for (s, e) in stored.behaviours.iter().zip(&mut out.behaviours) {
        for (sk, ek) in s.kfps.iter().zip(&mut e.kfps) {
            steps(sk, ek);
        }
    }
    out
}

// ⚠ A stored reciprocal is not computed from the stored times: one computed matches it bit for
// bit in under half of the keys, so it is kept wherever its two keys are as stored.
fn steps(stored: &Kfp, edited: &mut Kfp) {
    let held: Vec<[u32; 3]> = (stored.entries.windows(2))
        .map(|w| [w[0].time[0], w[1].time[0], w[1].time[1]].map(f32::to_bits))
        .collect();
    let mut before: Option<f32> = None;
    for e in &mut edited.entries {
        let t = e.time[0];
        e.time[1] = match before {
            None => 0.0,
            Some(b) if held.contains(&[b, t, e.time[1]].map(f32::to_bits)) => e.time[1],
            Some(b) if t == b => 0.0,
            Some(b) => 1.0 / (t - b),
        };
        before = Some(t);
    }
}

impl Plan<'_> {
    fn put(&mut self, at: usize, bytes: &[u8]) -> Result<()> {
        self.v.get("value written", at, bytes.len())?;
        self.out.push((at, bytes.to_vec()));
        Ok(())
    }

    fn words(&mut self, at: usize, words: &[u32]) -> Result<()> {
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        self.put(at, &bytes)
    }

    fn floats(&mut self, at: usize, values: &[f32]) -> Result<()> {
        let words: Vec<u32> = values.iter().map(|f| f.to_bits()).collect();
        self.words(at, &words)
    }

    /// The keys of the keyframed value at `at`, whose array is written shorter where it stands
    /// when keys were removed, with no pointer once none is left.
    fn kfp(&mut self, at: usize, stored: &Kfp, edited: &Kfp) -> Result<()> {
        need(
            edited.entries.len() <= stored.entries.len(),
            "a key was added to a keyframed value, which add_particle_keys writes past the section's end first",
        )?;
        if edited.entries.len() < stored.entries.len() {
            let n = (edited.entries.len() as u16).to_le_bytes();
            if edited.entries.is_empty() {
                self.put(at + 8, &[0; 8])?;
            }
            self.put(at + 0x10, &[n, n].concat())?;
        }
        if edited.entries.is_empty() {
            return Ok(());
        }
        let to = self.v.to("keyframed value", at + 8)?;
        let values: Vec<f32> = (edited.entries.iter())
            .flat_map(|e| e.time.into_iter().chain(e.value))
            .collect();
        self.floats(to, &values)
    }

    fn effect(&mut self, s: &EffectRule, e: &EffectRule) -> Result<()> {
        let at = s.at as usize;
        self.words(at + 0x50, &[e.num_loops as u32])?;
        self.put(at + 0x54, &e.flags)?;
        self.floats(at + 0x60, &e.rand_offset_pos)?;
        self.floats(at + 0x70, &e.times)?;
        self.put(at + 0x88, &e.culling_modes)?;
        self.floats(at + 0x90, &e.culling_sphere_offset)?;
        self.floats(at + 0xA0, &e.culling)?;
        self.floats(at + 0xB4, &[e.coln_range, e.coln_probe_dist])?;
        self.put(at + 0x210, &e.data_volume)?;
        for (i, (sk, ek)) in s.kfps.iter().zip(&e.kfps).enumerate() {
            self.kfp(at + 0xC0 + KFP * i, sk, ek)?;
        }
        need(
            s.events.len() == e.events.len(),
            "an event was added or removed: a new event's words at +0x18 to +0x24 and its object at +0x60 have no stated value, and what an event list holds past its count is not stated",
        )?;
        let (to, _, _) = array(&self.v, "effect rule", at + 0x38, false)?;
        for (i, ev) in e.events.iter().enumerate() {
            let place = self.v.to("event", to + 8 * i)?;
            self.floats(place + 0x10, &[ev.start_ratio, ev.end_ratio])?;
            let ([rate_min, rate_max], [zoom_min, zoom_max]) =
                (ev.playback_rate_scalar, ev.zoom_scalar);
            self.floats(place + 0x68, &[rate_min, rate_max, zoom_min, zoom_max])?;
            self.words(place + 0x78, &ev.colour_tint)?;
        }
        Ok(())
    }

    /// Each event of `e` that names another rule than the one it named in `s` pointed at that
    /// rule and at its name.
    fn relink(&mut self, s: &EffectRule, e: &EffectRule, stored: &ParticleEffects) -> Result<()> {
        let (to, _, _) = array(&self.v, "effect rule", s.at as usize + 0x38, false)?;
        for (i, (was, ev)) in s.events.iter().zip(&e.events).enumerate() {
            let place = self.v.to("event", to + 8 * i)?;
            if was.emitter != ev.emitter {
                let Some(rule) = stored.emitters.get(ev.emitter) else {
                    return refuse("an event names an emitter rule the list does not hold");
                };
                let at = rule.at as usize;
                self.link(place + 0x50, place + 0x40, at, at + 0x20)?;
            }
            if was.particle != ev.particle {
                let Some(rule) = stored.particles.get(ev.particle) else {
                    return refuse("an event names a particle rule the list does not hold");
                };
                let at = rule.at as usize;
                self.link(place + 0x58, place + 0x48, at, at + 0x178)?;
            }
        }
        Ok(())
    }

    /// The pointer at `at` aimed at the rule at `rule`, and the pointer at `name` given the word
    /// at `held`, the rule's pointer to its name.
    fn link(&mut self, at: usize, name: usize, rule: usize, held: usize) -> Result<()> {
        let word = self.v.u64("rule name", held)?;
        self.put(at, &(VIRTUAL + rule as u64).to_le_bytes())?;
        self.put(name, &word.to_le_bytes())
    }

    /// Each emitter and particle rule's reference count: 1 and the events of `edited` naming it.
    fn reference_counts(
        &mut self,
        stored: &ParticleEffects,
        edited: &ParticleEffects,
    ) -> Result<()> {
        let mut emitters = vec![1u32; stored.emitters.len()];
        let mut particles = vec![1u32; stored.particles.len()];
        for ev in edited.effects.iter().flat_map(|e| &e.events) {
            if let Some(n) = emitters.get_mut(ev.emitter) {
                *n += 1;
            }
            if let Some(n) = particles.get_mut(ev.particle) {
                *n += 1;
            }
        }
        let counts = (stored.emitters.iter().map(|r| r.at).zip(emitters))
            .chain(stored.particles.iter().map(|r| r.at).zip(particles));
        for (at, n) in counts {
            self.words(at as usize + 0x10, &[n])?;
        }
        Ok(())
    }

    fn emitter(&mut self, s: &EmitterRule, e: &EmitterRule) -> Result<()> {
        let at = s.at as usize;
        self.words(at + 0x2F0, &[e.flags])?;
        for (i, (sk, ek)) in s.kfps.iter().zip(&e.kfps).enumerate() {
            self.kfp(at + 0x48 + KFP * i, sk, ek)?;
        }
        for (kind, (sd, ed)) in s.domains.iter().zip(&e.domains).enumerate() {
            // A domain added or removed is refused by the read that follows the write.
            let (Some(sd), Some(ed)) = (sd, ed) else {
                continue;
            };
            let d = self.v.to("domain", at + 0x30 + 8 * kind)?;
            self.put(d + 0x190, &ed.flags)?;
            for (i, (sk, ek)) in sd.kfps.iter().zip(&ed.kfps).enumerate() {
                self.kfp(d + 8 + KFP * i, sk, ek)?;
            }
        }
        Ok(())
    }

    fn particle(&mut self, s: &ParticleRule, e: &ParticleRule) -> Result<()> {
        let at = s.at as usize;
        for (place, sp) in [0x20, 0x90, 0x100].into_iter().zip(&e.spawners) {
            let sp_at = at + place;
            for (o, sc) in [(0x08, &sp.scalars_min), (0x28, &sp.scalars_max)] {
                let words = [
                    sc.duration.to_bits(),
                    sc.playback_rate.to_bits(),
                    sc.colour_tint,
                    sc.zoom.to_bits(),
                    sc.flags,
                ];
                self.words(sp_at + o, &words)?;
            }
            self.floats(sp_at + 0x58, &[sp.trigger_info])?;
            self.put(sp_at + 0x5C, &sp.flags)?;
        }
        self.words(at + 0x170, &e.tex_frame_id.map(|id| id as u32))?;
        self.put(at + 0x410, &[e.sort_type])
    }
}
