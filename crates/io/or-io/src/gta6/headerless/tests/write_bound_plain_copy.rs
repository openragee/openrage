use super::*;

/// The built type-4 child's shrunk vertices, its two octants' indices and its +0x108 values.
const SHRUNK: [i16; 12] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
const OCTANTS: [(usize, &[u32]); 2] = [(0, &[0, 1]), (3, &[1, 2, 3])];
const SECOND: [u16; 4] = [7, 8, 9, 10];
const MATERIAL: u64 = 0x4000_7646;
/// Each word of the built child that names bytes, and how many it names.
const SPANS: [(usize, usize); 7] = [
    (0x78, 24),
    (0x88, 32),
    (0xB0, 24),
    (0xF0, 8),
    (0x118, 2),
    (0xC0, 96 + 20),
    (0x108, 48),
];

fn word(to: usize) -> [u8; 8] {
    (0x5000_0000 + to as u64).to_le_bytes()
}

fn put(s: &mut Rpf9Sections, at: usize, bytes: &[u8]) {
    s.virtual_section[at..at + bytes.len()].copy_from_slice(bytes);
}

/// `bytes` laid out past the end of `s`'s virtual section, 16-aligned.
fn appended(s: &mut Rpf9Sections, bytes: &[u8]) -> usize {
    let v = &mut s.virtual_section;
    let at = v.len().next_multiple_of(16);
    v.resize(at + bytes.len().next_multiple_of(16), 0);
    v[at..at + bytes.len()].copy_from_slice(bytes);
    at
}

/// [`two_triangles`] as a type-4 child: shrunk vertices, two octants, a +0x108 block whose second
/// slot is set and one material; the sections and where the child lies.
fn plain() -> (Rpf9Sections, usize) {
    let (mut s, child, _) = two_triangles();
    s.virtual_section[child + 0x10] = PLAIN_GEOMETRY;
    let shrunk: Vec<u8> = SHRUNK.iter().flat_map(|q| q.to_le_bytes()).collect();
    let at = appended(&mut s, &shrunk);
    put(&mut s, child + 0x78, &word(at));

    let indices: Vec<u8> = (OCTANTS.iter())
        .flat_map(|(_, list)| list.iter())
        .flat_map(|i| i.to_le_bytes())
        .collect();
    let c = appended(&mut s, &[vec![0; 96], indices].concat());
    let mut next = 96;
    for (i, list) in OCTANTS {
        put(&mut s, c + 4 * i, &(list.len() as u32).to_le_bytes());
        put(&mut s, c + 32 + 8 * i, &word(c + next));
        next += 4 * list.len();
    }
    put(&mut s, child + 0xC0, &word(c));
    put(&mut s, child + 0xC8, &word(c + 32));

    let values: Vec<u8> = SECOND.iter().flat_map(|x| x.to_le_bytes()).collect();
    let array = appended(&mut s, &values);
    let block = appended(&mut s, &[0; 48]);
    let count = (SECOND.len() as u32).to_le_bytes();
    put(&mut s, block + 16, &count);
    put(&mut s, block + 20, &count);
    put(&mut s, block + 24, &word(array));
    put(&mut s, child + 0x108, &word(block));

    let ids = appended(&mut s, &MATERIAL.to_le_bytes());
    let per_primitive = appended(&mut s, &[0, 0]);
    put(&mut s, child + 0xF0, &word(ids));
    put(&mut s, child + 0x118, &word(per_primitive));
    s.virtual_section[child + 0x120] = 1;
    (s, child)
}

/// The bytes each of [`SPANS`] names for the child at `at`.
fn spans(v: &[u8], at: usize) -> Vec<Vec<u8>> {
    (SPANS.iter())
        .map(|(w, len)| {
            let t = target(v, at + w);
            v[t..t + len].to_vec()
        })
        .collect()
}

fn copy_child(s: &mut Rpf9Sections) -> String {
    let kept = s.clone();
    let op = BoundReshape::CopyChild { child: 0 };
    let err = reshape_bound_body(s, BoundHolder::Bound, &op).expect_err("refused");
    assert_eq!(*s, kept, "{err}");
    err.to_string()
}

#[test]
fn a_type_4_geometry_child_is_copied_with_arrays_of_its_own() {
    let (mut s, child) = plain();
    let stored = read_bound(&s.virtual_section).expect("the bound reads");
    let mut edited = read_bound(&s.virtual_section).expect("the bound reads");
    assert_eq!(
        copy_bound_child(&mut edited, 0).expect("a type-4 geometry"),
        1
    );
    let before = spans(&s.virtual_section, child);
    let record = s.virtual_section[child..child + 0x130].to_vec();
    let op = BoundReshape::CopyChild { child: 0 };
    reshape_bound_body(&mut s, BoundHolder::Bound, &op).expect("laid out past the end");

    let read = read_bound(&s.virtual_section).expect("it reads back");
    assert_eq!(read.children.len(), 2);
    let shown = |b: &Bound, i: usize| format!("{:?}", b.children[i].bound);
    assert_eq!(
        shown(&read, 1),
        shown(&read, 0),
        "the copy reads as its source"
    );
    assert_eq!(
        shown(&read, 0),
        shown(&stored, 0),
        "the source reads as stored"
    );
    let v = &s.virtual_section;
    assert_eq!(v[child..child + 0x130], record[..], "the source's record");
    assert_eq!(spans(v, child), before, "every array the source names");

    let copy = target(v, target(v, 0x70) + 8);
    assert_ne!(copy, child);
    assert_eq!(v[copy + 0x10], PLAIN_GEOMETRY);
    let (ours, theirs) = (spans(v, copy), spans(v, child));
    for (k, (word, _)) in SPANS.iter().enumerate() {
        assert_ne!(
            target(v, copy + word),
            target(v, child + word),
            "+{word:#X} names bytes of the copy's own"
        );
        if !matches!(word, 0xC0 | 0x108) {
            assert_eq!(ours[k], theirs[k], "+{word:#X} holds the same bytes");
        }
    }
    // The octants: the same counts, then pointers into the copy's own index arrays.
    let (a, b) = (target(v, child + 0xC0), target(v, copy + 0xC0));
    assert_eq!(target(v, copy + 0xC8), b + 32);
    assert_eq!(v[a..a + 32], v[b..b + 32]);
    assert_eq!(v[a + 96..a + 116], v[b + 96..b + 116]);
    for i in 0..8 {
        match u64_at(v, a + 32 + 8 * i) {
            0 => assert_eq!(u64_at(v, b + 32 + 8 * i), 0, "octant {i}"),
            _ => assert_eq!(
                target(v, b + 32 + 8 * i) - b,
                target(v, a + 32 + 8 * i) - a,
                "octant {i}"
            ),
        }
    }
    // The +0x108 block: the same counts, its slot's array the copy's own with the same values.
    let (a, b) = (target(v, child + 0x108), target(v, copy + 0x108));
    assert_eq!(v[a..a + 24], v[b..b + 24]);
    assert_eq!(v[b + 32..b + 48], [0; 16]);
    let (x, y) = (target(v, a + 24), target(v, b + 24));
    assert_ne!(x, y);
    assert_eq!(v[x..x + 8], v[y..y + 8]);
}

/// The count the built +0xF8 record states: one more than the child's vertices, as some measured.
const F8_COUNT: usize = 5;

/// [`plain`] with a +0xF8 record: its count four times, two arrays of 6-byte elements back to
/// back, the first the child's vertices and one more, a record outside the child at +0x20, the
/// child at +0x28 and a word and a float at +0x30; the sections, the child and the record.
fn with_record() -> (Rpf9Sections, usize, usize) {
    let (mut s, child) = plain();
    let vertices = target(&s.virtual_section, child + 0xB0);
    let mut arrays = s.virtual_section[vertices..vertices + 6 * (F8_COUNT - 1)].to_vec();
    // The first array's last element, then the second array.
    arrays.extend((1..=6 + 6 * F8_COUNT).map(|x| x as u8));
    let first = appended(&mut s, &arrays);
    let owner = appended(&mut s, &[[0; 16], [0x11; 16]].concat());
    let count = (F8_COUNT as u32).to_le_bytes();
    let mut record = [count, count].concat();
    record.extend(word(first));
    record.extend([count, count].concat());
    record.extend(word(first + 6 * F8_COUNT));
    record.extend(word(owner));
    record.extend(word(child));
    record.extend(10u32.to_le_bytes());
    record.extend(0.5f32.to_le_bytes());
    record.resize(0x50, 0);
    let at = appended(&mut s, &record);
    put(&mut s, child + 0xF8, &word(at));
    (s, child, at)
}

#[test]
fn a_type_4_geometry_child_whose_0xf8_is_set_is_copied_with_a_record_of_its_own() {
    let (mut s, child, record) = with_record();
    let source = s.virtual_section[child..child + 0x130].to_vec();
    let arrays = |v: &[u8], r: usize| {
        [0x08, 0x18].map(|w| v[target(v, r + w)..target(v, r + w) + 6 * F8_COUNT].to_vec())
    };
    let held = s.virtual_section[record..record + 0x50].to_vec();
    let held_arrays = arrays(&s.virtual_section, record);
    let op = BoundReshape::CopyChild { child: 0 };
    reshape_bound_body(&mut s, BoundHolder::Bound, &op).expect("laid out past the end");

    let read = read_bound(&s.virtual_section).expect("it reads back");
    assert_eq!(read.children.len(), 2);
    let shown = |i: usize| format!("{:?}", read.children[i].bound);
    assert_eq!(shown(1), shown(0), "the copy reads as its source");
    let v = &s.virtual_section;
    assert_eq!(v[child..child + 0x130], source[..], "the source's record");
    assert_eq!(
        v[record..record + 0x50],
        held[..],
        "the source's +0xF8 record"
    );
    assert_eq!(arrays(v, record), held_arrays, "the source's +0xF8 arrays");

    let copy = target(v, target(v, 0x70) + 8);
    let ours = target(v, copy + 0xF8);
    assert_ne!(ours, record, "the copy's +0xF8 names a record of its own");
    assert_eq!(v[ours..ours + 8], v[record..record + 8]);
    assert_eq!(v[ours + 0x10..ours + 0x18], v[record + 0x10..record + 0x18]);
    for w in [0x08, 0x18] {
        assert_ne!(target(v, ours + w), target(v, record + w), "+{w:#X}");
    }
    assert_eq!(
        arrays(v, ours),
        held_arrays,
        "the same elements in arrays of its own"
    );
    assert_eq!(
        u64_at(v, ours + 0x20),
        u64_at(v, record + 0x20),
        "+0x20 kept"
    );
    assert_eq!(
        target(v, ours + 0x28),
        copy,
        "the copy's record names the copy"
    );
    assert_eq!(
        target(v, record + 0x28),
        child,
        "the source's names the source"
    );
    assert_eq!(v[ours + 0x30..ours + 0x50], v[record + 0x30..record + 0x50]);
}

#[test]
fn a_0xf8_record_off_the_measured_layout_is_refused_by_name() {
    // A record that names another geometry at +0x28.
    let (mut s, _, record) = with_record();
    put(&mut s, record + 0x28, &word(record));
    assert!(copy_child(&mut s).contains("at its +0x28"));
    // A second array whose capacity is not the count.
    let (mut s, _, record) = with_record();
    put(&mut s, record + 0x14, &9u32.to_le_bytes());
    assert!(copy_child(&mut s).contains("one count four times"));
    // A word past the arrays that aims into the section.
    let (mut s, child, record) = with_record();
    put(&mut s, record + 0x40, &word(child));
    assert!(copy_child(&mut s).contains("share them"));
}

#[test]
fn a_type_4_geometry_child_off_the_measured_octant_or_slot_layout_is_refused() {
    // The pointers not just past the counts.
    let (mut s, child) = plain();
    let c = target(&s.virtual_section, child + 0xC0);
    put(&mut s, child + 0xC8, &word(c + 48));
    assert!(copy_child(&mut s).contains("octants"));
    // Octant 3's indices not where octant 0's end.
    let (mut s, child) = plain();
    let c = target(&s.virtual_section, child + 0xC0);
    put(&mut s, c + 32 + 8 * 3, &word(c + 100));
    assert!(copy_child(&mut s).contains("octants"));
    // A slot whose capacity is not its count.
    let (mut s, child) = plain();
    let block = target(&s.virtual_section, child + 0x108);
    put(&mut s, block + 20, &5u32.to_le_bytes());
    assert!(copy_child(&mut s).contains("+0x108"));
}
