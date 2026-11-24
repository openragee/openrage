use super::*;
use crate::container::rpf::Gta6NestedRow;
use crate::gta6::hash::name_hash32;

fn entry() -> Gta6Entry {
    Gta6Entry {
        index: 0,
        name64: 0,
        name32: None,
        name: None,
        start: Some(0),
        stored: None,
        real: None,
        class: None,
        cipher: None,
    }
}

fn extra(kind: u32, virtual_len: u32, physical_len: u32) -> Gta6EntryExtra {
    Gta6EntryExtra {
        nested: Some(Gta6NestedRow {
            resource_type: kind,
            virtual_size: virtual_len,
            flags: physical_len | 3,
            stored_pages: 0,
        }),
        ..Default::default()
    }
}

fn u16_at(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn u32_at(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn u64_at(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

// The two tiled mip levels use different constant blocks, so their decoded order is observable.
fn texture_dictionary() -> (Gta6EntryExtra, Vec<u8>) {
    let mut bytes = vec![0; 0x100 + 0x2000];
    u64_at(&mut bytes, 0x08, 0x5000_0040);
    u64_at(&mut bytes, 0x18, 1);
    u64_at(&mut bytes, 0x20, 0x5000_0040);
    u16_at(&mut bytes, 0x28, 1);
    u16_at(&mut bytes, 0x2A, 1);
    u64_at(&mut bytes, 0x30, 0x5000_0050);
    u16_at(&mut bytes, 0x38, 1);
    u16_at(&mut bytes, 0x3A, 1);
    u32_at(&mut bytes, 0x40, name_hash32(b"sample"));
    u64_at(&mut bytes, 0x50, 0x5000_0060);
    let t = 0x60;
    u32_at(&mut bytes, t + 0x08, 0x2000 / 8);
    u16_at(&mut bytes, t + 0x0C, 8);
    u64_at(&mut bytes, t + 0x18, 0x6000_0000);
    u64_at(&mut bytes, t + 0x28, 0x5000_00B0);
    u64_at(&mut bytes, t + 0x30, 0x5000_00D0);
    u16_at(&mut bytes, t + 0x38, 128);
    u16_at(&mut bytes, t + 0x3A, 64);
    u16_at(&mut bytes, t + 0x3C, 1);
    bytes[t + 0x3E] = 1;
    bytes[t + 0x3F] = 71;
    bytes[t + 0x40] = 5;
    bytes[t + 0x42] = 2;
    u16_at(&mut bytes, t + 0x46, 1);
    u32_at(&mut bytes, t + 0x4C, 2);
    bytes[0xD8..0xDF].copy_from_slice(b"sample\0");
    bytes[0x100..0x1100].fill(0xBB);
    bytes[0x1100..].fill(0xAA);
    (extra(5, 0x100, 0x2000), bytes)
}

#[test]
fn table_hints_do_not_infer_types_from_archive_names_or_extensions() {
    let mut entry = entry();
    entry.name = Some("textures.ytd".into());
    assert_eq!(
        Gta6EntryType::hint(&entry, &Gta6EntryExtra::default()),
        Gta6EntryType::Unknown
    );
    for (code, expected) in [
        (2, Gta6EntryType::Resource(Rpf9Role::Drawable)),
        (3, Gta6EntryType::Resource(Rpf9Role::DrawableWithBound)),
        (4, Gta6EntryType::Resource(Rpf9Role::Dictionary)),
        (5, Gta6EntryType::TextureDictionary),
        (6, Gta6EntryType::Resource(Rpf9Role::Bound)),
        (11, Gta6EntryType::Unknown),
    ] {
        assert_eq!(Gta6EntryType::hint(&entry, &extra(code, 0, 0)), expected);
    }
    entry.class = Some(0);
    assert_eq!(
        Gta6EntryType::hint(&entry, &Gta6EntryExtra::default()),
        Gta6EntryType::Archive
    );
    assert_eq!(
        Gta6EntryType::hint(&entry, &extra(5, 0, 0)),
        Gta6EntryType::Unknown
    );
}

#[test]
fn a_texture_dictionary_is_read_and_untiled_from_the_table_sections_in_memory() {
    let (extra, bytes) = texture_dictionary();
    let detected = Gta6EntryContent::detect(&entry(), &extra, &bytes).expect("valid dictionary");
    assert_eq!(detected.kind, Gta6EntryType::TextureDictionary);
    let dictionary = detected.textures.expect("validated textures");
    let [(code, Ok(Entry::Texture(texture)))] = dictionary.entries.as_slice() else {
        panic!("one complete texture: {dictionary:?}");
    };
    assert_eq!(*code, name_hash32(b"sample"));
    assert_eq!(
        (texture.name.as_str(), texture.width, texture.height),
        ("sample", 128, 64)
    );
    let pixels = texture.texture.as_ref().expect("both mip levels");
    let top = 128 / 4 * (64 / 4) * 8;
    let tail = 64 / 4 * (32 / 4) * 8;
    assert_eq!(pixels.levels, 2);
    assert_eq!(pixels.pixels.len(), top + tail);
    assert!(pixels.pixels[..top].iter().all(|byte| *byte == 0xAA));
    assert!(pixels.pixels[top..].iter().all(|byte| *byte == 0xBB));
}

#[test]
fn a_texture_dictionary_refuses_inconsistent_sizes_flags_and_root() {
    let (extra, bytes) = texture_dictionary();
    let mut wrong = extra;
    wrong.nested.as_mut().expect("row").virtual_size += 16;
    assert!(Gta6EntryContent::detect(&entry(), &wrong, &bytes).is_err());
    wrong = extra;
    wrong.nested.as_mut().expect("row").virtual_size |= 1;
    assert!(Gta6EntryContent::detect(&entry(), &wrong, &bytes).is_err());
    wrong = extra;
    wrong.nested.as_mut().expect("row").flags ^= 1;
    assert!(Gta6EntryContent::detect(&entry(), &wrong, &bytes).is_err());
    assert!(Gta6EntryContent::detect(&entry(), &extra, &bytes[..bytes.len() - 1]).is_err());
    let mut bytes = bytes;
    bytes[..4].copy_from_slice(b"EDUP");
    assert!(Gta6EntryContent::detect(&entry(), &extra, &bytes).is_err());
}

#[test]
fn a_texture_dictionary_never_discards_failed_entries_or_failed_pixels() {
    let (extra, bytes) = texture_dictionary();
    let mut broken_entry = bytes.clone();
    u64_at(&mut broken_entry, 0x50, 0x5000_0400);
    let parsed =
        read_dictionary(&broken_entry[..0x100], Some(&broken_entry[0x100..])).expect("root reads");
    assert!(parsed.entries[0].1.is_err());
    assert!(Gta6EntryContent::detect(&entry(), &extra, &broken_entry).is_err());
    let mut broken_pixels = bytes;
    broken_pixels[0x60 + 0x3F] = 255;
    let parsed = read_dictionary(&broken_pixels[..0x100], Some(&broken_pixels[0x100..]))
        .expect("root reads");
    let Ok(Entry::Texture(texture)) = &parsed.entries[0].1 else {
        panic!("texture structure reads")
    };
    assert!(texture.texture.is_err());
    assert!(Gta6EntryContent::detect(&entry(), &extra, &broken_pixels).is_err());
}

#[test]
fn a_dictionary_signature_does_not_make_nontexture_entries_into_textures() {
    let (mut extra, mut bytes) = texture_dictionary();
    bytes[0x60..0xD0].fill(0);
    u64_at(&mut bytes, 0x60 + 0x10, 7);
    let parsed = read_dictionary(&bytes[..0x100], Some(&bytes[0x100..])).expect("root reads");
    assert!(matches!(&parsed.entries[0].1, Ok(Entry::Word64(7))));
    assert!(Gta6EntryContent::detect(&entry(), &extra, &bytes).is_err());
    extra.nested.as_mut().expect("row").resource_type = 4;
    let detected = Gta6EntryContent::detect(&entry(), &extra, &bytes).expect("ordinary dictionary");
    assert_eq!(detected.kind, Gta6EntryType::Resource(Rpf9Role::Dictionary));
    assert!(detected.textures.is_none());
}

#[test]
fn empty_texture_dictionaries_still_require_a_valid_dictionary_root() {
    let mut bytes = vec![0; 0x40];
    u64_at(&mut bytes, 0x08, 0x5000_0030);
    u64_at(&mut bytes, 0x18, 1);
    let extra = extra(5, bytes.len() as u32, 0);
    let detected = Gta6EntryContent::detect(&entry(), &extra, &bytes).expect("empty dictionary");
    assert_eq!(detected.kind, Gta6EntryType::TextureDictionary);
    assert!(detected.textures.expect("dictionary").entries.is_empty());
    u64_at(&mut bytes, 0x18, 2);
    assert!(Gta6EntryContent::detect(&entry(), &extra, &bytes).is_err());
}

#[test]
fn body_headers_are_recognised_only_at_the_start_of_the_decoded_entry() {
    for (header, expected) in [
        (b"RBF0", Gta6EntryType::Rbf),
        (b"EDUP", Gta6EntryType::Body(OtherBody::Edup)),
    ] {
        let detected =
            Gta6EntryContent::detect(&entry(), &Gta6EntryExtra::default(), header).expect("header");
        assert_eq!(detected.kind, expected);
        let mut later = vec![0xFF; 0x80];
        later[0x20..0x24].copy_from_slice(header);
        let detected = Gta6EntryContent::detect(&entry(), &Gta6EntryExtra::default(), &later)
            .expect("unknown bytes");
        assert_eq!(detected.kind, Gta6EntryType::Unknown);
        assert!(detected.textures.is_none());
    }
}

#[test]
fn an_unrecognised_body_does_not_confirm_the_table_hint() {
    let bytes = vec![0xFF; 0x40];
    let extra = extra(2, bytes.len() as u32, 0);
    assert_eq!(
        Gta6EntryType::hint(&entry(), &extra),
        Gta6EntryType::Resource(Rpf9Role::Drawable)
    );
    let detected = Gta6EntryContent::detect(&entry(), &extra, &bytes).expect("unrecognised body");
    assert_eq!(detected.kind, Gta6EntryType::Unknown);
}

#[test]
fn a_signature_inside_the_body_is_not_searched_for() {
    let (_, dictionary) = texture_dictionary();
    let mut bytes = vec![0xFF; 0x100];
    bytes.extend_from_slice(&dictionary);
    let extra = extra(11, bytes.len() as u32, 0);
    let detected = Gta6EntryContent::detect(&entry(), &extra, &bytes).expect("unknown root");
    assert_eq!(detected.kind, Gta6EntryType::Unknown);
    assert!(detected.textures.is_none());
}

#[test]
fn truncated_or_conflicting_texture_metadata_cannot_offer_a_preview() {
    let (extra, bytes) = texture_dictionary();
    for cut in [0, 16, 32, 48] {
        let mut short = extra;
        let row = short.nested.as_mut().expect("row");
        row.virtual_size = cut as u32;
        row.flags = 3;
        assert!(Gta6EntryContent::detect(&entry(), &short, &bytes[..cut]).is_err());
    }
    let mut conflicting = entry();
    conflicting.class = Some(11);
    assert!(Gta6EntryContent::detect(&conflicting, &extra, &bytes).is_err());
}

#[test]
fn a_nested_archive_hint_cannot_confirm_a_missing_or_incomplete_header() {
    let mut entry = entry();
    entry.class = Some(0);
    for bytes in [b"EDUP".as_slice(), b"9FPR".as_slice(), &[]] {
        assert!(Gta6EntryContent::detect(&entry, &Gta6EntryExtra::default(), bytes).is_err());
    }
}

#[test]
fn a_packed_ten_bit_texture_preserves_tile_coordinates_and_rgba_channels() {
    use crate::codec::texture::{decode_level, DxgiFormat, GrcTextureFormat};

    let (mut extra, mut bytes) = texture_dictionary();
    extra.nested.as_mut().expect("row").flags = 0x1003;
    bytes.truncate(0x1100);
    bytes[0x100..].fill(0);
    let t = 0x60;
    u32_at(&mut bytes, t + 0x08, 32 * 32);
    u16_at(&mut bytes, t + 0x0C, 4);
    u16_at(&mut bytes, t + 0x38, 32);
    u16_at(&mut bytes, t + 0x3A, 32);
    bytes[t + 0x3F] = 24;
    bytes[t + 0x42] = 1;
    let mut linear = Vec::new();
    let mut rgba = Vec::new();
    for y in 0..32u32 {
        for x in 0..32u32 {
            let (r, g, b, a) = (x * 33, y * 33, (x ^ y) * 33, (x + y) % 4);
            let packed = r | (g << 10) | (b << 20) | (a << 30);
            // A four-byte tile's element-index bits are x0 x1 y0 y1 y2 x2 y3 x3 y4 x4.
            let index = (x & 3)
                | ((y & 7) << 2)
                | ((x & 4) << 3)
                | ((y & 8) << 3)
                | ((x & 8) << 4)
                | ((y & 16) << 4)
                | ((x & 16) << 5);
            u32_at(&mut bytes, 0x100 + index as usize * 4, packed);
            linear.extend_from_slice(&packed.to_le_bytes());
            rgba.extend([r as u16 / 4, g as u16 / 4, b as u16 / 4, a as u16 * 85].map(|c| c as u8));
        }
    }
    let detected =
        Gta6EntryContent::detect(&entry(), &extra, &bytes).expect("four-byte packed texels");
    let dictionary = detected.textures.expect("texture dictionary");
    let [(_, Ok(Entry::Texture(texture)))] = dictionary.entries.as_slice() else {
        panic!("one texture: {dictionary:?}");
    };
    let texture = texture.texture.as_ref().expect("pixels");
    assert_eq!(texture.format.dxgi(), DxgiFormat::R10G10B10A2Unorm);
    assert_eq!(texture.format.format(), GrcTextureFormat::A2B10G10R10);
    assert_eq!(texture.pixels, linear);
    let image = decode_level(texture, 0).expect("packed channels decoded");
    assert_eq!((image.width, image.height), (32, 32));
    assert_eq!(image.data, rgba);
}

#[test]
fn a_packed_ten_bit_volume_preserves_each_slice_and_its_channels() {
    use crate::codec::texture::decode_level;

    let (mut extra, mut bytes) = texture_dictionary();
    extra.nested.as_mut().expect("row").flags = 0x1003;
    bytes.truncate(0x1100);
    bytes[0x100..].fill(0);
    let t = 0x60;
    u32_at(&mut bytes, t + 0x08, 8 * 16 * 8);
    u16_at(&mut bytes, t + 0x0C, 4);
    u16_at(&mut bytes, t + 0x38, 8);
    u16_at(&mut bytes, t + 0x3A, 16);
    u16_at(&mut bytes, t + 0x3C, 8);
    bytes[t + 0x3E] = 3;
    bytes[t + 0x3F] = 24;
    bytes[t + 0x42] = 1;
    let mut linear = Vec::new();
    let mut rgba_slices = Vec::new();
    for z in 0..8u32 {
        let mut rgba = Vec::new();
        for y in 0..16u32 {
            for x in 0..8u32 {
                let (r, g, b, a) = (x * 1023 / 7, y * 1023 / 15, z * 1023 / 7, (x + y + z) % 4);
                let packed = r | (g << 10) | (b << 20) | (a << 30);
                // A four-byte volume tile's index bits are x0 y0 z0 y1 x1 z1 y2 x2 z2 y3.
                let index = (x & 1)
                    | ((y & 1) << 1)
                    | ((z & 1) << 2)
                    | ((y & 2) << 2)
                    | ((x & 2) << 3)
                    | ((z & 2) << 4)
                    | ((y & 4) << 4)
                    | ((x & 4) << 5)
                    | ((z & 4) << 6)
                    | ((y & 8) << 6);
                u32_at(&mut bytes, 0x100 + index as usize * 4, packed);
                linear.extend_from_slice(&packed.to_le_bytes());
                rgba.extend([r / 4, g / 4, b / 4, a * 85].map(|c| c as u8));
            }
        }
        rgba_slices.push(rgba);
    }
    let detected =
        Gta6EntryContent::detect(&entry(), &extra, &bytes).expect("packed volume texels");
    let dictionary = detected.textures.expect("texture dictionary");
    let [(_, Ok(Entry::Texture(texture)))] = dictionary.entries.as_slice() else {
        panic!("one texture: {dictionary:?}");
    };
    assert_eq!((texture.dimension, texture.depth), (3, 8));
    let texture = texture.texture.as_ref().expect("volume pixels");
    assert_eq!(texture.faces, 8);
    assert_eq!(texture.pixels, linear);
    for (z, rgba) in rgba_slices.iter().enumerate() {
        let slice = texture.face(z as u32).expect("slice");
        let image = decode_level(&slice, 0).expect("packed channels decoded");
        assert_eq!((image.width, image.height), (8, 16));
        assert_eq!(&image.data, rgba);
    }
}
