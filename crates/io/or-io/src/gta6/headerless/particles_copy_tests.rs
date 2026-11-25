//! An effect rule duplicated past the section's end under a new name.

use super::*;
use crate::container::rpf::{Rpf9Form, Rpf9Sections};

const COPY: &str = "ent_test_spray_copy";

/// The body as block-mapped sections whose map counts the one virtual block it is cut into.
fn mapped(mut v: Vec<u8>) -> Rpf9Sections {
    let map = (u64_at(&v, 8) & 0x0FFF_FFFF) as usize;
    v[map + 8] = 1;
    Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section: v,
        physical: Vec::new(),
    }
}

#[test]
fn an_effect_duplicated_reads_as_its_source_under_its_new_name_and_moves_nothing() {
    let (v, at) = body();
    let mut s = mapped(v);
    let before = s.virtual_section.clone();
    let old = before.len();
    duplicate_particle_effect(&mut s, 0, COPY).expect("it copies");

    let list = read_list(&s.virtual_section).expect("it reads");
    assert_eq!(list.effects.len(), 2);
    let source = (list.effects.iter().find(|e| e.name == EFFECT)).expect("the source");
    let copy = (list.effects.iter().find(|e| e.name == COPY)).expect("the copy");
    assert!(copy.at as usize >= old, "the copy lies past the old end");
    assert_eq!(copy.code, hash(COPY));
    let renamed = EffectRule {
        at: source.at,
        code: source.code,
        name: source.name.clone(),
        ..copy.clone()
    };
    assert_eq!(&renamed, source);
    let codes: Vec<u32> = list.effects.iter().map(|e| e.code).collect();
    assert!(codes.windows(2).all(|w| w[0] < w[1]), "the codes ascend");

    // Each rule the copied event names is named once more.
    assert_eq!(u32_at(&s.virtual_section, at.emitter + 0x10), 3);
    assert_eq!(u32_at(&s.virtual_section, at.particle + 0x10), 3);
    // What was there changed only in the dictionary's heads and the two counts.
    let mut kept = s.virtual_section[..old].to_vec();
    let d = (u64_at(&before, 0x28) & 0x0FFF_FFFF) as usize;
    for (from, len) in [
        (d + 0x20, 0x20),
        (at.emitter + 0x10, 4),
        (at.particle + 0x10, 4),
    ] {
        kept[from..from + len].copy_from_slice(&before[from..from + len]);
    }
    assert_eq!(kept, before);
    // The copy's evolution list is its own.
    let list_of = |rule: u64| u64_at(&s.virtual_section, rule as usize + 0x48);
    assert_ne!(list_of(copy.at), list_of(source.at));
}

#[test]
fn a_duplicated_effect_takes_its_own_edits_and_leaves_its_source_as_it_was() {
    let (v, _) = body();
    let mut s = mapped(v);
    duplicate_particle_effect(&mut s, 0, COPY).expect("it copies");
    let mut list = read_list(&s.virtual_section).expect("it reads");
    let copy = (list.effects.iter().position(|e| e.name == COPY)).expect("the copy");
    let source = (list.effects.iter().position(|e| e.name == EFFECT)).expect("the source");
    list.effects[copy].num_loops = 4;
    list.effects[copy].events[0].start_ratio = 0.3;
    let held = list.effects[source].clone();
    write_particle_effects(&mut s.virtual_section, &list).expect("it writes");
    let back = read_list(&s.virtual_section).expect("it reads again");
    assert_eq!(back.effects[copy].num_loops, 4);
    assert_eq!(back.effects[copy].events[0].start_ratio, 0.3);
    assert_eq!(back.effects[source], held);
}

#[test]
fn a_duplicate_is_refused_for_a_taken_name_a_missing_source_or_an_event_object_of_no_rule() {
    let (v, at) = body();
    let mut s = mapped(v);
    let before = s.clone();
    for (source, name) in [(0, EFFECT), (5, COPY), (0, ""), (0, "a\tname")] {
        let refused = duplicate_particle_effect(&mut s, source, name).expect_err("refused");
        assert!(
            matches!(
                refused,
                ParticleKeysError::Read(HeaderlessError::Particles(_))
            ),
            "{refused}"
        );
        assert_eq!(s, before);
    }

    // An event's +0x60 aimed at a particle rule, which reads as no measured object.
    let to = (0x5000_0000 + at.particle as u64).to_le_bytes();
    s.virtual_section[at.event + 0x60..at.event + 0x68].copy_from_slice(&to);
    let before = s.clone();
    let refused = duplicate_particle_effect(&mut s, 0, COPY).expect_err("an event's object");
    assert!(refused.to_string().contains("+0x60"), "{refused}");
    assert_eq!(s, before);

    let mut stored = mapped(body().0);
    stored.form = Rpf9Form::StoredRsc8 { config: 0 };
    let before = stored.clone();
    let refused = duplicate_particle_effect(&mut stored, 0, COPY).expect_err("no block map");
    assert!(matches!(refused, ParticleKeysError::Grow(_)), "{refused}");
    assert_eq!(stored, before);
}
