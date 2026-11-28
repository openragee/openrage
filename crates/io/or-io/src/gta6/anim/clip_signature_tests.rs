use std::sync::Arc;

use super::*;
use crate::container::rpf::{Rpf9Form, Rpf9Sections};

/// The standard CRC-32 bit by bit from `state`, with no table: the check the writer's is held to.
fn straight(state: u32, bytes: &[u8]) -> u32 {
    let mut c = !state;
    for b in bytes {
        c ^= u32::from(*b);
        for _ in 0..8 {
            c = if c & 1 == 1 {
                (c >> 1) ^ 0xEDB8_8320
            } else {
                c >> 1
            };
        }
    }
    !c
}

fn le(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// `bytes` with a block map of one virtual block at their end, which root 0x08 points at.
fn mapped(mut bytes: Vec<u8>) -> Vec<u8> {
    let map = bytes.len().next_multiple_of(16);
    bytes.resize(map + 24, 0);
    bytes[map + 8] = 1;
    bytes[0x08..0x10].copy_from_slice(&(VIRTUAL + map as u64).to_le_bytes());
    bytes
}

fn opened(bytes: Vec<u8>) -> AnimResource {
    let mut resource = read(&bytes).expect("reads");
    resource.sections = Some(Arc::new(Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section: bytes,
        physical: Vec::new(),
    }));
    resource
}

fn section(resource: &AnimResource) -> &[u8] {
    &(resource.sections.as_ref())
        .expect("sections")
        .virtual_section
}

/// The one clip's stored signature and the one its fields give.
fn signed(bytes: &[u8]) -> (u32, u32) {
    let signatures = clip_signatures(bytes).expect("the clips read");
    assert_eq!(signatures.len(), 1);
    signatures[0]
}

fn groups() -> Spec {
    Spec {
        clip: 2,
        ..Spec::default()
    }
}

#[test]
fn a_clip_of_one_animation_signs_its_animation_window_and_rate_from_its_type() {
    let built = build(&Spec::default());
    // Type 1, animation signature, start 0, end 1, rate 1, no warp list.
    let bytes = le(&[SIGNATURE, 0, DURATION.to_bits(), 1f32.to_bits(), 0]);
    assert_eq!(signed(&built.bytes), (0, straight(1, &bytes)));

    let mut rate = built.bytes.clone();
    rate[built.clip + 0x98..built.clip + 0x9C].copy_from_slice(&2f32.to_bits().to_le_bytes());
    assert_ne!(signed(&rate).1, signed(&built.bytes).1);

    let renamed = rename_clip(&opened(built.bytes.clone()), 0, "clip_b").expect("renamed");
    assert_eq!(signed(section(&renamed)).1, signed(&built.bytes).1);
}

#[test]
fn a_clip_of_groups_signs_each_entry_from_0_and_its_warp_list_s_crc() {
    let built = build(&groups());
    let points: Vec<u8> = (WARP.iter())
        .flat_map(|[x, y]| le(&[x.to_bits(), y.to_bits(), 0, 0]))
        .collect();
    let warp = straight(0, &points);
    let entry = |start: f32, end: f32, rate: f32, warp: u32| {
        let fields = le(&[
            SIGNATURE,
            start.to_bits(),
            end.to_bits(),
            rate.to_bits(),
            warp,
        ]);
        straight(0, &fields)
    };
    let entries = le(&[entry(0.0, 1.0, 1.0, 0), entry(0.25, 0.75, 2.0, warp)]);
    assert_eq!(signed(&built.bytes), (0, straight(2, &entries)));
}

#[test]
fn a_window_or_rate_edit_writes_the_signature_the_clip_s_fields_give() {
    let one = opened(build(&Spec::default()).bytes);
    let window = AnimEdit::ClipWindow {
        clip: 0,
        start: 0.125,
        end: 0.75,
        rate: 1.5,
    };
    let edited = edit(&one, &window).expect("edits");
    let (stored, computed) = signed(section(&edited));
    assert_eq!(stored, computed);
    assert_ne!(computed, signed(section(&one)).1);
    let fitted = edit_in_extent(&one, &window).expect("fits");
    assert_eq!(section(&fitted), section(&edited));

    let groups = opened(build(&groups()).bytes);
    let entry = AnimEdit::EntryWindow {
        clip: 0,
        group: 0,
        entry: 1,
        start: 0.5,
        end: 1.25,
        frame_rate: None,
    };
    let edited = edit(&groups, &entry).expect("edits");
    let (stored, computed) = signed(section(&edited));
    assert_eq!(stored, computed);
    assert_ne!(computed, signed(section(&groups)).1);
}

#[test]
fn a_dof_list_edit_signs_again_every_clip_that_plays_the_animation() {
    for spec in [Spec::default(), groups()] {
        let resource = opened(mapped(build(&spec).bytes));
        let add = AnimEdit::AddDof {
            animation: AnimAt::default(),
            dof: Dof {
                id: 7,
                track: hash("boneScale"),
                kind: 2,
            },
            value: [1.0, 1.0, 1.0, 0.0],
        };
        let added = edit(&resource, &add).expect("edits");
        assert!(the_animation(&added).signature_holds());
        let (stored, computed) = signed(section(&added));
        assert_eq!(stored, computed, "clip type {}", spec.clip);
        assert_ne!(computed, signed(section(&resource)).1);
    }
}

#[test]
fn an_edit_that_changes_none_of_a_clip_s_fields_keeps_its_word_as_read() {
    let resource = opened(build(&Spec::default()).bytes);
    let prior = (the_animation(&resource).delta_value(2, 0, 29)).expect("a value");
    let key = AnimEdit::Key {
        animation: AnimAt::default(),
        dof: 2,
        component: 0,
        frame: 30,
        value: prior,
    };
    let edited = edit(&resource, &key).expect("edits");
    // The built clip states 0, which its fields do not give, and keeps it.
    let (stored, computed) = signed(section(&edited));
    assert_eq!(stored, 0);
    assert_eq!(computed, signed(section(&resource)).1);
}
