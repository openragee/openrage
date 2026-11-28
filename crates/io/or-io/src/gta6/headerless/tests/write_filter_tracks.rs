use super::dictionary::root_over;
use super::write_filters::{bones, filters_and_record, read_back, tracks};
use super::*;
use crate::container::rpf::{Rpf9Form, Rpf9Sections};
use crate::gta6::hash::name_hash32;
use crate::gta6::headerless::write_filter_tracks::filter_signatures;

/// `img` with a block map of one virtual block at its end, which root 0x08 points at.
pub(super) fn mapped(img: Image) -> Rpf9Sections {
    let mut bytes = img.bytes;
    let map = bytes.len().next_multiple_of(16);
    bytes.resize(map + 24, 0);
    bytes[map + 8] = 1;
    bytes[0x08..0x10].copy_from_slice(&(0x5000_0000 + map as u64).to_le_bytes());
    Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section: bytes,
        physical: Vec::new(),
    }
}

/// The word at 0x08 of each of the filter's elements, which no reader names.
fn unnamed(sections: &Rpf9Sections) -> Vec<u32> {
    let v = &sections.virtual_section;
    // The filter is the entry after the 0x40-byte root; its elements' pointer is at +0x28.
    let count = usize::from(u16::from_le_bytes([v[0x70], v[0x71]]));
    let first = (u64_at(v, 0x68) & 0x0FFF_FFFF) as usize;
    (0..count).map(|i| u32_at(v, first + 48 * i + 8)).collect()
}

#[test]
fn a_filter_track_removed_or_copied_moves_what_follows_it_and_reads_back() {
    let (tracks, bones) = (tracks(), bones());
    let sections = mapped(filters_and_record(&tracks, &bones));
    let code = name_hash32(b"edit_filter");
    let len = sections.virtual_section.len();

    let mut same = sections.clone();
    write_filter_tracks(&mut same, code, &tracks).expect("written");
    assert_eq!(same, sections);

    // One fewer: the record after the tracks moves back and still reads.
    let mut fewer = sections.clone();
    let kept = vec![tracks[0], tracks[2]];
    write_filter_tracks(&mut fewer, code, &kept).expect("written");
    assert_eq!(fewer.virtual_section.len(), len - 48);
    assert_eq!(read_back(&fewer.virtual_section), (kept, bones.clone()));
    assert_eq!(unnamed(&fewer), vec![0xA0, 0xA2]);

    // A copy of a track, its weight edited, carries every byte of the track it copies.
    let mut more = sections.clone();
    let mut copy = tracks[1];
    copy.weight = 4.0;
    let grown = vec![tracks[0], tracks[1], copy, tracks[2]];
    write_filter_tracks(&mut more, code, &grown).expect("written");
    assert_eq!(more.virtual_section.len(), len + 48);
    assert_eq!(read_back(&more.virtual_section), (grown, bones.clone()));
    assert_eq!(unnamed(&more), vec![0xA0, 0xA1, 0xA1, 0xA2]);

    // None: the filter holds a null array, as every empty filter read does.
    let mut none = sections.clone();
    write_filter_tracks(&mut none, code, &[]).expect("written");
    assert_eq!(none.virtual_section.len(), len - 3 * 48);
    assert_eq!(u64_at(&none.virtual_section, 0x68), 0);
    assert_eq!(read_back(&none.virtual_section), (Vec::new(), bones));
}

#[test]
fn a_record_bone_removed_or_copied_reads_back() {
    let (tracks, bones) = (tracks(), bones());
    let sections = mapped(filters_and_record(&tracks, &bones));
    let code = name_hash32(b"edit_set");
    let len = sections.virtual_section.len();

    let mut fewer = sections.clone();
    write_record_bones(&mut fewer, code, &bones[1..]).expect("written");
    assert_eq!(fewer.virtual_section.len(), len - 144);
    assert_eq!(
        read_back(&fewer.virtual_section),
        (tracks.clone(), bones[1..].to_vec())
    );

    let mut more = sections.clone();
    let mut copy = bones[0];
    copy.values[3] = -1.0;
    let grown = vec![bones[0], copy, bones[1]];
    write_record_bones(&mut more, code, &grown).expect("written");
    assert_eq!(more.virtual_section.len(), len + 144);
    assert_eq!(read_back(&more.virtual_section), (tracks, grown));
}

#[test]
fn a_new_element_is_built_from_its_named_values_and_an_edit_in_place_keeps_the_others() {
    let (tracks, bones) = (tracks(), bones());
    let sections = mapped(filters_and_record(&tracks, &bones));
    let code = name_hash32(b"edit_filter");
    let len = sections.virtual_section.len();

    // A track that is no copy: its unnamed bytes are zero.
    let mut new = tracks[0];
    new.group ^= 1;
    new.word_2c = 2;
    let mut added = sections.clone();
    let grown = vec![tracks[0], new];
    write_filter_tracks(&mut added, code, &grown).expect("written");
    assert_eq!(read_back(&added.virtual_section), (grown, bones.clone()));
    assert_eq!(unnamed(&added), vec![0xA0, 0]);

    // A track's id, group and mode set where it lies: its unnamed bytes stay, nothing moves.
    let mut edited = tracks.clone();
    edited[1].track ^= 0x55;
    edited[1].group ^= 0x66;
    edited[1].word_2c = 1;
    let mut in_place = sections.clone();
    write_filter_tracks(&mut in_place, code, &edited).expect("written");
    assert_eq!(in_place.virtual_section.len(), len);
    assert_eq!(
        read_back(&in_place.virtual_section),
        (edited, bones.clone())
    );
    assert_eq!(unnamed(&in_place), vec![0xA0, 0xA1, 0xA2]);

    // A bone that is no copy, and one whose id, kind and flags change where it lies.
    let mut bone = bones[0];
    (bone.bone, bone.kind, bone.flags) = (0x1234, 8, 0x204);
    let mut record = sections.clone();
    let set = name_hash32(b"edit_set");
    write_record_bones(&mut record, set, &[bones[0], bones[1], bone]).expect("written");
    assert_eq!(
        read_back(&record.virtual_section).1,
        vec![bones[0], bones[1], bone]
    );
    let mut record = sections.clone();
    write_record_bones(&mut record, set, &[bone, bones[1]]).expect("written");
    assert_eq!(read_back(&record.virtual_section).1, vec![bone, bones[1]]);
}

/// A dictionary of two frame filters, `a` of signature 0x1111 and `b` of 0x2222, with a block map.
fn two_filters(a: &[FilterTrack], b: &[FilterTrack]) -> Rpf9Sections {
    let mut img = Image::default();
    let root = img.alloc(0x40);
    let mut entries = Vec::new();
    for (name, tracks, signature) in [("a", a, 0x1111), ("b", b, 0x2222)] {
        let filter = img.alloc(0x40);
        img.u32(filter + 0x08, 1);
        img.u32(filter + 0x0C, signature);
        img.u32(filter + 0x10, 14);
        let text = img.alloc(32);
        img.put(text, format!("pack:/{name}.iff\0").as_bytes());
        img.ptr(filter + 0x18, text);
        if !tracks.is_empty() {
            let elements = img.alloc(48 * tracks.len());
            for (i, t) in tracks.iter().enumerate() {
                let e = elements + 48 * i;
                img.u32(e, t.track);
                img.u32(e + 0x04, t.weight.to_bits());
                img.u32(e + 0x28, t.group);
                img.u32(e + 0x2C, t.word_2c);
            }
            img.ptr(filter + 0x28, elements);
        }
        img.u16(filter + 0x30, tracks.len() as u16);
        img.u16(filter + 0x32, tracks.len() as u16);
        entries.push((name_hash32(name.as_bytes()), filter));
    }
    root_over(&mut img, root, &mut entries);
    mapped(img)
}

/// The signature of the filter named `name` in `sections`.
fn signature(sections: &Rpf9Sections, name: &str) -> u32 {
    let v = &sections.virtual_section;
    let count = usize::from(u16::from_le_bytes([v[0x28], v[0x29]]));
    let codes = (u64_at(v, 0x20) & 0x0FFF_FFFF) as usize;
    let entries = (u64_at(v, 0x30) & 0x0FFF_FFFF) as usize;
    let i = (0..count)
        .position(|i| u32_at(v, codes + 4 * i) == name_hash32(name.as_bytes()))
        .expect("the filter");
    let filter = (u64_at(v, entries + 8 * i) & 0x0FFF_FFFF) as usize;
    u32_at(v, filter + 0x0C)
}

/// The signature the elements of the filter named `name` in `sections` give.
fn computed(sections: &Rpf9Sections, name: &str) -> u32 {
    let filters = filter_signatures(&sections.virtual_section).expect("the filters read");
    let code = name_hash32(name.as_bytes());
    (filters.into_iter())
        .find_map(|(c, _, computed)| (c == code).then_some(computed))
        .expect("the filter")
}

#[test]
fn a_changed_filter_takes_the_signature_its_elements_give_or_0_when_empty() {
    let tracks = tracks()
        .into_iter()
        .map(|t| FilterTrack { word_2c: 0, ..t })
        .collect::<Vec<_>>();
    let sections = two_filters(&tracks, &tracks[..1]);
    let b = name_hash32(b"b");

    // Unchanged elements keep the signature as read; the other filter keeps its own.
    let mut kept = sections.clone();
    write_filter_tracks(&mut kept, b, &tracks[..1]).expect("written");
    assert_eq!(kept, sections);

    // The same elements as filter a's: the signature they give, not the one a states.
    let mut same = sections.clone();
    write_filter_tracks(&mut same, b, &tracks).expect("written");
    assert_eq!(signature(&same, "b"), computed(&same, "b"));
    assert_eq!(signature(&same, "b"), computed(&same, "a"));
    assert_ne!(signature(&same, "b"), 0x1111);
    assert_eq!(signature(&same, "a"), 0x1111);

    let mut other = sections.clone();
    write_filter_tracks(&mut other, b, &tracks[1..]).expect("written");
    assert_eq!(signature(&other, "b"), computed(&other, "b"));
    assert_ne!(signature(&other, "b"), 0x2222);

    // A weight written in place signs the filter again.
    let mut weighed = sections.clone();
    let mut heavier = tracks[..1].to_vec();
    heavier[0].weight = 3.5;
    write_filter_weights(&mut weighed.virtual_section, b, &heavier).expect("written");
    assert_eq!(signature(&weighed, "b"), computed(&weighed, "b"));
    assert_ne!(signature(&weighed, "b"), 0x2222);

    // None: a null array and signature 0; then a track again, in an array at the section's end.
    let mut none = sections.clone();
    write_filter_tracks(&mut none, b, &[]).expect("written");
    assert_eq!(signature(&none, "b"), 0);
    let mut again = none.clone();
    write_filter_tracks(&mut again, b, &tracks[2..]).expect("written");
    assert_eq!(signature(&again, "b"), computed(&again, "b"));
    let dict = read_dictionary(&again.virtual_section, None).expect("reads");
    let held = (dict.entries.iter()).find_map(|(code, e)| match e {
        Ok(Entry::FrameFilter { tracks, .. }) if *code == b => Some(tracks.clone()),
        _ => None,
    });
    assert_eq!(held, Some(tracks[2..].to_vec()));
}

/// The standard CRC-32 bit by bit from 0, with no table: the check the writer's is held to.
fn straight(bytes: &[u8]) -> u32 {
    let mut c = !0u32;
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

/// One frame filter's element as the signature takes it: an id, a weight, two names, a track
/// and a mode.
struct Element {
    id: u32,
    weight: f32,
    inline: &'static [u8],
    pointed: &'static [u8],
    track: u32,
    mode: u32,
}

/// A dictionary of one filter, `filter`, of `elements`: the first name inline, the second through
/// a pointer and a length.
fn one_filter(elements: &[Element]) -> Vec<u8> {
    let mut img = Image::default();
    let root = img.alloc(0x40);
    let filter = img.alloc(0x40);
    img.u32(filter + 0x08, 1);
    img.u32(filter + 0x10, 14);
    let text = img.alloc(32);
    img.put(text, b"pack:/filter.iff\0");
    img.ptr(filter + 0x18, text);
    if !elements.is_empty() {
        let first = img.alloc(48 * elements.len());
        for (i, e) in elements.iter().enumerate() {
            let at = first + 48 * i;
            img.u32(at, e.id);
            img.u32(at + 0x04, e.weight.to_bits());
            if !e.inline.is_empty() {
                img.put(at + 0x08, e.inline);
                img.bytes[at + 0x08 + 15] = 0x80 | e.inline.len() as u8;
            }
            if !e.pointed.is_empty() {
                let name = img.alloc(e.pointed.len() + 1);
                img.put(name, e.pointed);
                img.ptr(at + 0x18, name);
                img.u16(at + 0x18 + 8, e.pointed.len() as u16);
            }
            img.u32(at + 0x28, e.track);
            img.u32(at + 0x2C, e.mode);
        }
        img.ptr(filter + 0x28, first);
    }
    img.u16(filter + 0x30, elements.len() as u16);
    img.u16(filter + 0x32, elements.len() as u16);
    root_over(&mut img, root, &mut [(name_hash32(b"filter"), filter)]);
    img.bytes
}

fn the_signature(bytes: &[u8]) -> u32 {
    let filters = filter_signatures(bytes).expect("the filter reads");
    assert_eq!(filters.len(), 1);
    filters[0].2
}

#[test]
fn a_filter_s_signature_is_one_crc_over_each_element_s_names_weight_id_mode_byte_and_track() {
    let plain = Element {
        id: 0x11,
        weight: 1.0,
        inline: b"",
        pointed: b"",
        track: 0x98E1_750C,
        mode: 0,
    };
    let named = Element {
        id: 0x22,
        weight: 0.5,
        inline: b"SKEL_*",
        pointed: b"SKEL_Spine",
        track: 0xF791_A95F,
        mode: 2,
    };
    let mut expected = Vec::new();
    expected.extend(1f32.to_le_bytes());
    expected.extend(0x11u32.to_le_bytes());
    expected.push(0);
    expected.extend(0x98E1_750Cu32.to_le_bytes());
    expected.extend(b"SKEL_*SKEL_Spine");
    expected.extend(0.5f32.to_le_bytes());
    expected.extend(0x22u32.to_le_bytes());
    expected.push(2);
    expected.extend(0xF791_A95Fu32.to_le_bytes());
    let pair = [plain, named];
    assert_eq!(the_signature(&one_filter(&pair)), straight(&expected));

    let [plain, named] = pair;
    let swapped = [named, plain];
    assert_ne!(the_signature(&one_filter(&swapped)), straight(&expected));
    let [named, plain] = swapped;
    // Only the mode's low byte is taken.
    let wide = Element {
        mode: 0x102,
        ..named
    };
    assert_eq!(
        the_signature(&one_filter(&[plain, wide])),
        straight(&expected)
    );
    assert_eq!(the_signature(&one_filter(&[])), 0);
}

#[test]
fn a_bad_element_or_a_body_with_no_block_map_is_refused_unchanged() {
    let (tracks, bones) = (tracks(), bones());
    let sections = mapped(filters_and_record(&tracks, &bones));
    let mut refused = sections.clone();
    let code = name_hash32(b"edit_filter");
    let mut stranger = tracks[0];
    stranger.group ^= 1;
    let err = write_filter_tracks(&mut refused, code, &[stranger])
        .expect_err("refused")
        .to_string();
    assert!(err.contains("mode at 0x2C"), "{err}");
    let mut infinite = tracks.clone();
    infinite[0].weight = f32::INFINITY;
    let err = write_filter_tracks(&mut refused, code, &infinite)
        .expect_err("refused")
        .to_string();
    assert!(err.contains("finite"), "{err}");
    let err = write_record_bones(&mut refused, name_hash32(b"edit_set"), &[])
        .expect_err("refused")
        .to_string();
    assert!(err.contains("keeps a bone"), "{err}");
    assert_eq!(refused, sections);
    let err = write_record_bones(&mut refused, name_hash32(b"edit_filter"), &bones)
        .expect_err("refused")
        .to_string();
    assert!(err.contains("not of the kind edited"), "{err}");

    let mut stored = sections.clone();
    stored.form = Rpf9Form::StoredRsc8 { config: 0 };
    let err = write_filter_tracks(&mut stored, name_hash32(b"edit_filter"), &tracks[..1])
        .expect_err("refused")
        .to_string();
    assert!(err.contains("stored RSC8"), "{err}");
}
