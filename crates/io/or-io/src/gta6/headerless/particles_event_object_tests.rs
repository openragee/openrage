//! An event copied with the object its +0x60 names.

use super::*;

/// The words at +0x38 to +0x4F of a buffer object of two 128-byte rows.
const TAIL: [u8; 24] = [
    0x10, 0, 2, 0, 1, 0, 1, 0x0A, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 2, 0, 0, 0,
];

/// Where the body's event object, its buffer object and the buffer's name record are.
struct Object {
    record: usize,
    buffer: usize,
    text: usize,
}

/// The body with an evolution list of one evolution on its event and, at its +0x60, an object
/// whose bit set is `bits` and which points at an evoTexture buffer of two rows where any is set,
/// over a physical section of that buffer's 256 bytes.
fn with_object(bits: &[(usize, u8)]) -> (Rpf9Sections, Places, Object) {
    let (v, at) = body();
    let mut b = Body { v };
    let list = b.alloc(0x38);
    let evolutions = b.alloc(0x18);
    let name = b.text(EVOLUTION);
    b.ptr(evolutions, name);
    b.u32(evolutions + 8, hash(EVOLUTION));
    b.array(list, evolutions, 1, 1);
    b.ptr(at.event + 0x30, list);

    let record = b.alloc(0x90);
    for &(byte, set) in bits {
        b.v[record + byte] = set;
    }
    b.put(record + 0x88, &[1, 0, 0, 1, 1, 1]);
    b.ptr(at.event + 0x60, record);
    let (mut buffer, mut text) = (0, 0);
    if !bits.is_empty() {
        buffer = b.alloc(0x70);
        b.u32(buffer + 8, 64);
        b.u16(buffer + 0xC, 8);
        b.u16(buffer + 0xE, 8);
        b.u32(buffer + 0x10, 0x980);
        b.u64(buffer + 0x18, 0x6000_0000);
        b.ptr(buffer + 0x28, buffer + 0x50);
        text = b.alloc(0x18);
        b.put(text + 8, b"evoTexture");
        b.ptr(buffer + 0x30, text);
        b.put(buffer + 0x38, &TAIL);
        b.put(
            buffer + 0x60,
            &[0x41, 0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
        );
        b.ptr(record + 0x80, buffer);
    }
    let physical: Vec<u8> = (0..=255u8).collect();
    let object = Object {
        record,
        buffer,
        text,
    };
    (mapped(b.v, physical), at, object)
}

fn place(v: &[u8], at: usize) -> usize {
    (u64_at(v, at) & 0x0FFF_FFFF) as usize
}

#[test]
fn an_event_is_duplicated_with_an_object_and_a_buffer_of_its_own() {
    let (mut s, at, o) = with_object(&[(8, 0b101), (0x58, 0x80)]);
    let (old, physical) = (s.virtual_section.clone(), s.physical.clone());
    let effect = read_list(&old).expect("it reads").effects[0].at as usize;
    duplicate_particle_event(&mut s, 0, 0).expect("it copies with its object");

    let v = &s.virtual_section;
    let list = read_list(v).expect("it reads");
    let events = &list.effects[0].events;
    assert_eq!(events.len(), 2);
    assert_eq!(events[1], events[0]);
    let copy = place(v, place(v, effect + 0x38) + 8);
    let record = place(v, copy + 0x60);
    assert!(
        record >= old.len() && record.is_multiple_of(16),
        "its own record"
    );
    assert_eq!(&v[record..record + 0x80], &old[o.record..o.record + 0x80]);
    assert_eq!(&v[record + 0x88..record + 0x90], &[1, 0, 0, 1, 1, 1, 0, 0]);

    // The buffer object differs from its source's in its three pointers only.
    let buffer = place(v, record + 0x80);
    assert!(buffer >= old.len());
    for w in (0..0x70).step_by(8) {
        if ![0x18, 0x28, 0x30].contains(&w) {
            assert_eq!(u64_at(v, buffer + w), u64_at(&old, o.buffer + w), "+{w:#x}");
        }
    }
    assert_eq!(
        u64_at(v, buffer + 0x28),
        0x5000_0000 + (buffer + 0x50) as u64
    );
    let text = place(v, buffer + 0x30);
    assert!(text >= old.len() && text != o.text);
    assert_eq!(&v[text..text + 0x18], &old[o.text..o.text + 0x18]);

    // Its data is its own, 256-aligned past the old physical end, and holds its source's.
    let word = u64_at(v, buffer + 0x18);
    assert_eq!(word >> 28, 6);
    let data = (word & 0x0FFF_FFFF) as usize;
    assert!(
        data >= physical.len() && data.is_multiple_of(0x100),
        "{data:#x}"
    );
    assert_eq!(&s.physical[data..data + 0x100], &physical[..]);
    assert_eq!(&s.physical[..physical.len()], &physical[..]);

    // The source keeps its object.
    assert_eq!(place(v, at.event + 0x60), o.record);
    let source = o.record..o.buffer + 0x70;
    assert_eq!(&v[source.clone()], &old[source]);
}

#[test]
fn an_effect_is_duplicated_with_objects_and_buffers_of_its_own() {
    const COPY: &str = "ent_test_spray_copy";
    let (mut s, at, o) = with_object(&[(8, 0b101), (0x58, 0x80)]);
    let (old, physical) = (s.virtual_section.clone(), s.physical.clone());
    duplicate_particle_effect(&mut s, 0, COPY).expect("it copies with its objects");

    let v = &s.virtual_section;
    let list = read_list(v).expect("it reads");
    let copy = (list.effects.iter().find(|e| e.name == COPY)).expect("the copy");
    let source = (list.effects.iter().find(|e| e.name == EFFECT)).expect("the source");
    assert_eq!(copy.events, source.events);
    let event = place(v, place(v, copy.at as usize + 0x38));
    let record = place(v, event + 0x60);
    assert!(
        record >= old.len() && record.is_multiple_of(16),
        "its own record"
    );
    assert_eq!(&v[record..record + 0x80], &old[o.record..o.record + 0x80]);
    let buffer = place(v, record + 0x80);
    assert!(buffer >= old.len(), "its own buffer object");
    let text = place(v, buffer + 0x30);
    assert!(text >= old.len(), "its own name record");
    let data = (u64_at(v, buffer + 0x18) & 0x0FFF_FFFF) as usize;
    assert!(data >= physical.len(), "{data:#x}");
    assert_eq!(&s.physical[data..data + 0x100], &physical[..]);

    // The source keeps its object, and a second copy is refused for its taken name.
    assert_eq!(place(v, at.event + 0x60), o.record);
    let before = s.clone();
    assert!(duplicate_particle_effect(&mut s, 0, COPY).is_err());
    assert_eq!(s, before);
}

#[test]
fn an_event_whose_object_has_no_buffer_is_duplicated_with_a_record_of_its_own() {
    let (mut s, at, o) = with_object(&[]);
    let (old, physical) = (s.virtual_section.clone(), s.physical.clone());
    duplicate_particle_event(&mut s, 0, 0).expect("it copies with its object");
    let v = &s.virtual_section;
    let effect = read_list(v).expect("it reads").effects[0].at as usize;
    let copy = place(v, place(v, effect + 0x38) + 8);
    let record = place(v, copy + 0x60);
    assert!(record >= old.len() && record != o.record);
    assert_eq!(&v[record..record + 0x90], &old[o.record..o.record + 0x90]);
    assert_eq!(s.physical, physical, "no data is laid out");
    assert_eq!(place(v, at.event + 0x60), o.record);
}

#[test]
fn an_event_whose_object_breaks_a_measured_rule_is_not_duplicated() {
    type Patch = fn(&mut [u8], &Object);
    let wrong: [(&str, Patch); 6] = [
        ("an evolution count", |v, o| v[o.record + 0x88] = 2),
        ("its last bytes", |v, o| v[o.record + 0x8F] = 1),
        ("its bit set emptied", |v, o| {
            v[o.record + 8] = 0;
            v[o.record + 0x58] = 0;
        }),
        ("the row count", |v, o| v[o.buffer + 0x3A] = 3),
        ("the record after it", |v, o| v[o.buffer + 0x50] = 1),
        ("its name", |v, o| v[o.text + 8] = b'k'),
    ];
    for (what, change) in wrong {
        let (mut s, _, o) = with_object(&[(8, 0b101), (0x58, 0x80)]);
        change(&mut s.virtual_section, &o);
        let why = refused_unchanged(&mut s, |s| duplicate_particle_event(s, 0, 0));
        assert!(why.contains("+0x60"), "{what}: {why}");
    }
    // A bit set with no buffer.
    let (mut s, _, o) = with_object(&[]);
    s.virtual_section[o.record + 8] = 1;
    let why = refused_unchanged(&mut s, |s| duplicate_particle_event(s, 0, 0));
    assert!(why.contains("+0x60"), "{why}");
}
