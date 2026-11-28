use super::dictionary::root_over;
use super::*;
use crate::gta6::hash::name_hash32;

/// A dictionary of one frame filter of `tracks` and one animation record of `bones`, by name.
pub(super) fn filters_and_record(tracks: &[FilterTrack], bones: &[BoneRecord]) -> Image {
    let mut img = Image::default();
    let root = img.alloc(0x40);
    let filter = img.alloc(0x40);
    img.u32(filter + 0x08, 1);
    img.u32(filter + 0x10, 14);
    let name = img.alloc(32);
    img.put(name, b"pack:/edit_filter.iff\0");
    img.ptr(filter + 0x18, name);
    let elements = img.alloc(48 * tracks.len());
    for (i, t) in tracks.iter().enumerate() {
        let e = elements + 48 * i;
        img.u32(e, t.track);
        img.u32(e + 0x04, t.weight.to_bits());
        // Words the reader does not name, kept by a write.
        img.u32(e + 0x08, 0xA0 + i as u32);
        img.u32(e + 0x28, t.group);
        img.u32(e + 0x2C, t.word_2c);
    }
    img.ptr(filter + 0x28, elements);
    img.u16(filter + 0x30, tracks.len() as u16);
    img.u16(filter + 0x32, tracks.len() as u16);
    let record = img.alloc(0x70);
    img.u32(record + 0x10, 1);
    img.u32(record + 0x18, bones.len() as u32);
    img.u32(record + 0x1C, bones.len() as u32);
    let first = img.alloc(144 * bones.len());
    for (i, b) in bones.iter().enumerate() {
        let e = first + 144 * i;
        for (f, value) in b.values.iter().enumerate() {
            img.u32(e + 0x0C + 4 * f, value.to_bits());
        }
        img.u32(e + 0x78, b.kind);
        img.u32(e + 0x7C, b.flags);
        img.u32(e + 0x80, b.bone);
    }
    img.ptr(record + 0x20, first);
    img.put(record + 0x58, b"edit_set");
    img.bytes[record + 0x67] = 0x80 | 8;
    let mut entries = [
        (name_hash32(b"edit_filter"), filter),
        (name_hash32(b"edit_set"), record),
    ];
    root_over(&mut img, root, &mut entries);
    img
}

pub(super) fn tracks() -> Vec<FilterTrack> {
    (0..3u32)
        .map(|i| FilterTrack {
            track: name_hash32(format!("track_{i}").as_bytes()),
            weight: 0.125 * (i + 1) as f32,
            group: name_hash32(format!("group_{i}").as_bytes()),
            word_2c: 7 + i,
        })
        .collect()
}

pub(super) fn bones() -> Vec<BoneRecord> {
    (0..2u32)
        .map(|i| BoneRecord {
            bone: name_hash32(format!("bone_{i}").as_bytes()),
            kind: 3 + i,
            flags: 0x300 + i,
            values: std::array::from_fn(|f| (100 * i + f as u32) as f32 + 0.25),
        })
        .collect()
}

pub(super) fn read_back(bytes: &[u8]) -> (Vec<FilterTrack>, Vec<BoneRecord>) {
    let dict = read_dictionary(bytes, None).expect("the dictionary reads");
    let (mut filter, mut record) = (Vec::new(), Vec::new());
    for (_, entry) in dict.entries {
        match entry.expect("each entry reads") {
            Entry::FrameFilter { tracks, .. } => filter = tracks,
            Entry::AnimationRecord { bones, .. } => record = bones,
            _ => {}
        }
    }
    (filter, record)
}

#[test]
fn a_filter_weight_and_a_record_value_are_written_in_place_and_read_back() {
    let (tracks, bones) = (tracks(), bones());
    let img = filters_and_record(&tracks, &bones);
    let mut bytes = img.bytes.clone();

    // Unedited values write the bytes as they were.
    write_filter_weights(&mut bytes, name_hash32(b"edit_filter"), &tracks).expect("written");
    write_record_values(&mut bytes, name_hash32(b"edit_set"), &bones).expect("written");
    assert_eq!(bytes, img.bytes);

    let mut edited_tracks = tracks.clone();
    edited_tracks[1].weight = -2.5;
    let mut edited_bones = bones.clone();
    edited_bones[1].values[26] = 9.75;
    write_filter_weights(&mut bytes, name_hash32(b"edit_filter"), &edited_tracks).expect("written");
    write_record_values(&mut bytes, name_hash32(b"edit_set"), &edited_bones).expect("written");
    assert_eq!(
        read_back(&bytes),
        (edited_tracks.clone(), edited_bones.clone())
    );
    let changed = (bytes.iter().zip(&img.bytes))
        .filter(|(a, b)| a != b)
        .count();
    // A weight, a value and the filter's signature.
    assert!((1..=12).contains(&changed), "{changed} bytes changed");

    // A track or bone the entry does not hold, another count, or another kind of entry is refused.
    let mut renamed = edited_tracks.clone();
    renamed[0].group ^= 1;
    let mut fewer = edited_bones.clone();
    fewer.pop();
    let refused = [
        write_filter_weights(&mut bytes, name_hash32(b"edit_filter"), &renamed),
        write_filter_weights(&mut bytes, name_hash32(b"edit_filter"), &edited_tracks[..2]),
        write_filter_weights(&mut bytes, name_hash32(b"edit_set"), &edited_tracks),
        write_record_values(&mut bytes, name_hash32(b"edit_set"), &fewer),
        write_record_values(&mut bytes, name_hash32(b"edit_filter"), &edited_bones),
        write_record_values(&mut bytes, 0x1234, &edited_bones),
    ];
    for (i, result) in refused.into_iter().enumerate() {
        assert!(
            matches!(result, Err(HeaderlessError::NotInPlace(_))),
            "case {i}: {result:?}"
        );
    }
    assert_eq!(read_back(&bytes), (edited_tracks, edited_bones));
}
