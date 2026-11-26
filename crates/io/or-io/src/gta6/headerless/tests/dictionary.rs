use super::*;
use crate::gta6::hash::name_hash32;

/// Lays out a dictionary's root at offset 0 of `img`, over `entries` (code, record) in code order.
pub(super) fn root_over(img: &mut Image, root: usize, entries: &mut [(u32, usize)]) {
    entries.sort_unstable();
    let code_array = img.alloc(4 * entries.len());
    for (i, (code, _)) in entries.iter().enumerate() {
        img.u32(code_array + 4 * i, *code);
    }
    let records: Vec<usize> = entries.iter().map(|(_, at)| *at).collect();
    let pointers = img.pointers(&records);
    let n = entries.len() as u16;
    img.ptr(root + 0x20, code_array);
    img.u16(root + 0x28, n);
    img.u16(root + 0x2A, n);
    img.ptr(root + 0x30, pointers);
    img.u16(root + 0x38, n);
    img.u16(root + 0x3A, n);
}

fn name_into(img: &mut Image, name: &str) -> usize {
    let at = img.alloc(8 + name.len() + 1);
    img.put(at + 8, name.as_bytes());
    at
}

/// A dictionary of a drawable and an entry of no kind, keyed by `codes`.
fn dictionary(codes: [u32; 2]) -> (Image, Vec<u8>) {
    let (mut img, mut phys) = (Image::default(), Vec::new());
    let root = img.alloc(0x40);
    let drawable = drawable_into(&mut img, &mut phys, [1.0, 1.0, 0.0]).root;
    let other = img.alloc(DRAWABLE_ROOT);
    let code_array = img.alloc(8);
    img.u32(code_array, codes[0]);
    img.u32(code_array + 4, codes[1]);
    let entries = img.pointers(&[other, drawable]);
    img.ptr(root + 0x20, code_array);
    img.u16(root + 0x28, 2);
    img.u16(root + 0x2A, 2);
    img.ptr(root + 0x30, entries);
    img.u16(root + 0x38, 2);
    img.u16(root + 0x3A, 2);
    (img, phys)
}

#[test]
fn a_dictionary_reads_its_drawables_and_refuses_an_entry_of_no_kind() {
    let code = name_hash32(b"box#dr");
    let (img, phys) = dictionary([0, code]);
    let dict = read_dictionary(&img.bytes, Some(&phys)).expect("the dictionary reads");
    let [(0, Err(HeaderlessError::UnknownEntry { .. })), (c, Ok(Entry::Drawable(d)))] =
        dict.entries.as_slice()
    else {
        panic!("a refusal and a drawable, not {:?}", dict.entries);
    };
    assert_eq!((*c, d.name.as_str()), (code, "box#dr"));
}

#[test]
fn the_shaders_of_each_model_are_read_from_the_virtual_section_alone() {
    let keys = |models: &[(String, Vec<Shader>)]| -> Vec<(String, Vec<Option<u64>>)> {
        let of = |(name, shaders): &(String, Vec<Shader>)| {
            let keys = shaders.iter().flat_map(|s| s.texture_keys.clone());
            (name.clone(), keys.collect())
        };
        models.iter().map(of).collect()
    };
    let wanted = vec![("box#dr".to_string(), vec![Some(TEXTURE_KEY), None])];
    let (img, _, _) = drawable([1.0, 1.0, 0.0]);
    let models = model_shaders(Rpf9Role::Drawable, &img.bytes).expect("a drawable");
    assert_eq!(keys(&models), wanted);
    // A dictionary gives each entry that is a drawable, and passes over an entry of no kind.
    let (img, _) = dictionary([0, name_hash32(b"box#dr")]);
    let models = model_shaders(Rpf9Role::Dictionary, &img.bytes).expect("a dictionary");
    assert_eq!(keys(&models), wanted);
    // A body of another role holds no model, and a section cut short is refused.
    let none = model_shaders(Rpf9Role::Bound, &img.bytes).expect("no model");
    assert!(none.is_empty());
    assert!(model_shaders(Rpf9Role::Drawable, &img.bytes[..0x20]).is_err());
}

#[test]
fn a_dictionary_whose_codes_do_not_ascend_is_refused() {
    let (img, phys) = dictionary([0x20, 0x10]);
    assert!(matches!(
        read_dictionary(&img.bytes, Some(&phys)),
        Err(HeaderlessError::CodesNotAscending)
    ));
}

/// A texture object named `name`: `size` (width, height, depth), its dimension, DXGI format,
/// levels and element stride, its pixels at `pixels` bytes into the physical section.
#[allow(clippy::too_many_arguments)]
pub(super) fn texture_into(
    img: &mut Image,
    name: &str,
    size: [u16; 3],
    dimension: u8,
    format: u8,
    mips: u8,
    stride: u16,
    pixels: (usize, usize),
) -> usize {
    let t = img.alloc(0x70);
    img.u32(t + 0x08, (pixels.1 / usize::from(stride)) as u32);
    img.u16(t + 0x0C, stride);
    img.u16(t + 0x0E, 8);
    img.u32(t + 0x10, 0x980);
    img.phys(t + 0x18, pixels.0);
    img.ptr(t + 0x28, t + 0x50);
    let name = name_into(img, name);
    img.ptr(t + 0x30, name);
    for (i, side) in size.iter().enumerate() {
        img.u16(t + 0x38 + 2 * i, *side);
    }
    img.bytes[t + 0x3E] = dimension;
    img.bytes[t + 0x3F] = format;
    img.bytes[t + 0x40] = 5;
    img.bytes[t + 0x42] = mips;
    img.u16(t + 0x46, 1);
    img.u32(t + 0x4C, 2);
    t
}

/// The index bits of a volume tile of 2- and of 8-byte elements, lowest first, as the layout states them.
const VOLUME_2: &str = "x0 z0 y0 z1 y1 x1 z2 y2 x2 z3 y3";
const VOLUME_8: &str = "x0 z0 y0 x1 z1 y1 x2 z2 y2";

/// Element (`x`, `y`, `z`)'s index in a volume tile whose index bits, lowest first, are `pattern`.
fn volume_index(pattern: &str, x: usize, y: usize, z: usize) -> usize {
    (pattern.split(' ').enumerate())
        .map(|(i, bit)| {
            let coordinate = [x, y, z][usize::from(bit.as_bytes()[0] - b'x')];
            ((coordinate >> (bit.as_bytes()[1] - b'0')) & 1) << i
        })
        .sum()
}

#[test]
fn a_texture_dictionary_reads_each_texture_by_name_from_the_physical_section() {
    let mut phys = vec![0u8; 0x6000];
    // 128×64 BC1 of two levels: the tail tile, mip1 at its slot 0x800, then mip0's one tile.
    phys[..0x1000].fill(0xBB);
    phys[0x1000..0x2000].fill(0xAA);
    for z in 0..16 {
        for y in 0..16 {
            for x in 0..16 {
                // A 16×16×16 R8G8 volume in two 8×16×16 tiles: a format measured in volumes only.
                let at = 0x2000 + x / 8 * 0x1000 + 2 * volume_index(VOLUME_2, x, y, z);
                phys[at..at + 2].copy_from_slice(&[x as u8, (y << 4 | z) as u8]);
                // An 8×8×8 RGBA16F volume in one tile.
                if x < 8 && y < 8 && z < 8 {
                    let at = 0x4000 + 8 * volume_index(VOLUME_8, x, y, z);
                    phys[at..at + 4].copy_from_slice(&[x as u8, y as u8, z as u8, 1]);
                }
            }
        }
    }
    let mut img = Image::default();
    let root = img.alloc(0x40);
    let bc1 = texture_into(&mut img, "Trophy", [128, 64, 1], 1, 71, 2, 8, (0, 0x2000));
    let r8g8 = texture_into(&mut img, "fog", [16, 16, 16], 3, 49, 1, 2, (0x2000, 0x2000));
    let lut = texture_into(&mut img, "lut", [8, 8, 8], 3, 10, 1, 8, (0x4000, 0x1000));
    let wrong = texture_into(&mut img, "wrong", [4, 4, 1], 1, 71, 1, 8, (0x5000, 0x1000));
    let mut entries = [
        (name_hash32(b"trophy"), bc1),
        (name_hash32(b"fog"), r8g8),
        (name_hash32(b"lut"), lut),
        (name_hash32(b"other"), wrong),
    ];
    root_over(&mut img, root, &mut entries);
    let dict = read_dictionary(&img.bytes, Some(&phys)).expect("the dictionary reads");
    let entry = |code: u32| {
        let (_, entry) = dict
            .entries
            .iter()
            .find(|(c, _)| *c == code)
            .expect("listed");
        entry
    };

    let Ok(Entry::Texture(trophy)) = entry(name_hash32(b"trophy")) else {
        panic!("a texture");
    };
    assert_eq!(trophy.name, "Trophy");
    assert_eq!(
        (trophy.width, trophy.height, trophy.mip_levels),
        (128, 64, 2)
    );
    let texture = trophy.texture.as_ref().expect("its pixels read");
    assert_eq!((texture.levels, texture.faces), (2, 1));
    let mut want = vec![0xAA; 32 * 16 * 8];
    want.extend([0xBB; 16 * 8 * 8]);
    assert!(
        texture.pixels == want,
        "mip0 last, mip1 in the tail tile first"
    );

    let Ok(Entry::Texture(fog)) = entry(name_hash32(b"fog")) else {
        panic!("a texture");
    };
    let fog = fog.texture.as_ref().expect("a 2-byte volume reads");
    assert_eq!(
        (fog.faces, fog.format.dxgi()),
        (16, crate::codec::texture::DxgiFormat::R8G8Unorm)
    );
    for (x, y, z) in [(0, 0, 0), (9, 2, 7), (15, 13, 1), (3, 15, 15)] {
        let at = 2 * ((z * 16 + y) * 16 + x);
        assert_eq!(fog.pixels[at..at + 2], [x as u8, (y << 4 | z) as u8]);
    }

    let Ok(Entry::Texture(lut)) = entry(name_hash32(b"lut")) else {
        panic!("a texture");
    };
    let volume = lut
        .texture
        .as_ref()
        .expect("a one-level 8-byte volume reads");
    assert_eq!(volume.faces, 8);
    for (x, y, z) in [(0, 0, 0), (5, 2, 7), (7, 7, 1)] {
        let at = 8 * ((z * 8 + y) * 8 + x);
        assert_eq!(volume.pixels[at..at + 4], [x as u8, y as u8, z as u8, 1]);
    }

    assert!(matches!(
        entry(name_hash32(b"other")),
        Err(HeaderlessError::Disagree { .. })
    ));
}

#[test]
fn a_cube_texture_is_a_cube_array_of_six_faces_each_and_refuses_another_view_or_depth() {
    // 32×32 BC7 of one level: each slice one 4 KiB tile, its level at offset 0; slice s all 0x10 + s.
    let mut phys = vec![0u8; 0x18000];
    for (s, tile) in phys.chunks_mut(0x1000).enumerate() {
        tile.fill(0x10 + s as u8);
    }
    let mut img = Image::default();
    let root = img.alloc(0x40);
    let cubes = texture_into(&mut img, "gobo", [32, 32, 12], 2, 98, 1, 16, (0, 0xC000));
    let view = texture_into(&mut img, "view", [32, 32, 12], 2, 98, 1, 16, (0, 0xC000));
    let odd = texture_into(&mut img, "odd", [32, 32, 9], 2, 98, 1, 16, (0xC000, 0x9000));
    img.u16(cubes + 0x60, 0x292);
    img.u16(view + 0x60, 0x241);
    img.u16(odd + 0x60, 0x292);
    let mut entries = [
        (name_hash32(b"gobo"), cubes),
        (name_hash32(b"view"), view),
        (name_hash32(b"odd"), odd),
    ];
    root_over(&mut img, root, &mut entries);
    let dict = read_dictionary(&img.bytes, Some(&phys)).expect("the dictionary reads");
    let texture = |code: u32| {
        (dict.entries.iter())
            .find(|(c, _)| *c == code)
            .and_then(|(_, entry)| match entry {
                Ok(Entry::Texture(t)) => Some(&t.texture),
                _ => None,
            })
            .expect("a texture")
    };

    let gobo = texture(name_hash32(b"gobo"))
        .as_ref()
        .expect("the cube array reads");
    assert_eq!((gobo.faces, gobo.levels), (12, 1));
    assert_eq!(gobo.pixels.len(), 12 * 8 * 8 * 16);
    for (s, face) in gobo.pixels.chunks(8 * 8 * 16).enumerate() {
        assert!(
            face.iter().all(|&b| b == 0x10 + s as u8),
            "slice {s}, face {} of cube {}",
            s % 6,
            s / 6
        );
    }
    assert!(matches!(
        texture(name_hash32(b"view")),
        Err(HeaderlessError::Constant { found: 0x241, .. })
    ));
    assert!(matches!(
        texture(name_hash32(b"odd")),
        Err(HeaderlessError::Disagree {
            first: 9,
            second: 6,
            ..
        })
    ));
}

/// An animation record named `name`, inline when it fits, over `bones`; the bone record at
/// `broken` holds a byte at 0x84, where every measured one holds zero.
fn animation_record_into(
    img: &mut Image,
    name: &str,
    bones: &[BoneRecord],
    broken: Option<usize>,
) -> (usize, usize) {
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
    if let Some(i) = broken {
        img.bytes[first + 144 * i + 0x84] = 1;
    }
    img.ptr(record + 0x20, first);
    if name.len() < 16 {
        img.put(record + 0x58, name.as_bytes());
        img.bytes[record + 0x67] = 0x80 | name.len() as u8;
    } else {
        let text = img.alloc(name.len() + 1);
        img.put(text, name.as_bytes());
        img.ptr(record + 0x58, text);
        img.u16(record + 0x60, name.len() as u16);
        img.u16(record + 0x62, name.len() as u16 + 1);
        img.u32(record + 0x64, 1);
    }
    (record, first)
}

/// Bone records whose every field differs from every other's.
fn bone_records(count: usize, seed: u32) -> Vec<BoneRecord> {
    (0..count as u32)
        .map(|i| BoneRecord {
            bone: name_hash32(format!("skel_{seed}_{i}").as_bytes()),
            kind: 10 * seed + i,
            flags: 0x200 + 10 * seed + i,
            values: std::array::from_fn(|f| (seed * 1000 + 30 * i + f as u32) as f32 + 0.5),
        })
        .collect()
}

#[test]
fn a_dictionary_reads_its_frame_filters_animation_records_and_64_bit_words() {
    let mut img = Image::default();
    let root = img.alloc(0x40);
    let filter = img.alloc(0x40);
    img.u32(filter + 0x08, 1);
    img.u32(filter + 0x10, 14);
    let name = img.alloc(32);
    img.put(name, b"pack:/basic_filter.iff\0");
    img.ptr(filter + 0x18, name);
    let tracks = [
        FilterTrack {
            track: name_hash32(b"track_a"),
            weight: 0.25,
            group: name_hash32(b"group_a"),
            word_2c: 2,
        },
        FilterTrack {
            track: 0xFFFF_FFFF,
            weight: 1.0,
            group: name_hash32(b"group_b"),
            word_2c: 1,
        },
    ];
    let elements = img.alloc(48 * tracks.len());
    for (i, t) in tracks.iter().enumerate() {
        let e = elements + 48 * i;
        img.u32(e, t.track);
        img.u32(e + 0x04, t.weight.to_bits());
        img.u32(e + 0x28, t.group);
        img.u32(e + 0x2C, t.word_2c);
    }
    img.ptr(filter + 0x28, elements);
    img.u16(filter + 0x30, 2);
    img.u16(filter + 0x32, 2);
    let empty = img.alloc(0x40);
    img.u32(empty + 0x08, 1);
    img.u32(empty + 0x10, 14);
    let empty_name = img.alloc(32);
    img.put(empty_name, b"pack:/empty_filter.iff\0");
    img.ptr(empty + 0x18, empty_name);
    let (inline, short) = (bone_records(2, 1), bone_records(3, 2));
    let inline_at = animation_record_into(&mut img, "short_set", &inline, None).0;
    let long_at = animation_record_into(&mut img, "a_record_set_of_long_name", &short, None).0;
    let (broken_at, broken_first) =
        animation_record_into(&mut img, "broken_set", &bone_records(2, 3), Some(1));
    let word = img.alloc(0x20);
    img.u64(word + 0x10, 0x0123_4567_89AB_CDEF);
    let mut entries = [
        (name_hash32(b"basic_filter"), filter),
        (name_hash32(b"empty_filter"), empty),
        (name_hash32(b"short_set"), inline_at),
        (name_hash32(b"a_record_set_of_long_name"), long_at),
        (name_hash32(b"broken_set"), broken_at),
        (7, word),
    ];
    root_over(&mut img, root, &mut entries);
    let dict = read_dictionary(&img.bytes, None).expect("the dictionary reads");
    let entry = |code: u32| {
        let (_, entry) = dict
            .entries
            .iter()
            .find(|(c, _)| *c == code)
            .expect("listed");
        entry
    };
    assert!(
        matches!(entry(name_hash32(b"basic_filter")), Ok(Entry::FrameFilter { name, tracks: t }) if name == "pack:/basic_filter.iff" && t[..] == tracks)
    );
    assert!(
        matches!(entry(name_hash32(b"empty_filter")), Ok(Entry::FrameFilter { tracks, .. }) if tracks.is_empty())
    );
    for (name, want) in [
        ("short_set", &inline),
        ("a_record_set_of_long_name", &short),
    ] {
        assert!(
            matches!(entry(name_hash32(name.as_bytes())), Ok(Entry::AnimationRecord { name: n, bones }) if n == name && bones == want),
            "{name}: {:?}",
            entry(name_hash32(name.as_bytes()))
        );
    }
    assert!(matches!(
        entry(name_hash32(b"broken_set")),
        Err(HeaderlessError::Constant { what: "bone record's zero bytes", at, found: 1 })
            if *at == (broken_first + 144 + 0x84) as u64
    ));
    assert!(matches!(entry(7), Ok(Entry::Word64(0x0123_4567_89AB_CDEF))));
}

#[test]
fn a_dictionary_states_the_trace_words_of_each_entry_that_is_a_drawable() {
    let (mut img, _) = dictionary([0, name_hash32(b"box#dr")]);
    let entries = u64_at(&img.bytes, 0x30) as usize & 0x0FFF_FFFF;
    let drawable = u64_at(&img.bytes, entries + 8) as usize & 0x0FFF_FFFF;
    img.u32(drawable + 0xB8, 0x0000_0400);
    // The entry of no kind states none.
    let words = trace_words(Rpf9Role::Dictionary, &img.bytes);
    let [Ok(words)] = words.as_slice() else {
        panic!("one drawable's words, not {words:?}");
    };
    assert_eq!(words.level_word, 0x0000_0400);
    assert_eq!(words.shaders[0].name_hash, SHADER_HASH);
    // An entry is asked for by its code.
    let code = name_hash32(b"box#dr");
    let entry = trace_words_of_entry(&img.bytes, code).expect("the dictionary reads");
    assert_eq!(entry.as_ref(), Some(words));
    assert!(matches!(
        trace_words_of_entry(&img.bytes, code + 1),
        Ok(None)
    ));
    // A dictionary whose entries do not read is one error.
    assert!(matches!(
        trace_words(Rpf9Role::Dictionary, &img.bytes[..0x20]).as_slice(),
        [Err(_)]
    ));
}
