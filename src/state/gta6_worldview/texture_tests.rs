use or_io::gta6::Format;

use super::*;

/// A texture of one face in DXGI format `dxgi`, its levels' elements `pixels`.
fn texture(dxgi: u8, width: u32, height: u32, levels: u32, pixels: Vec<u8>) -> Texture {
    Texture {
        width,
        height,
        format: Format::of_dxgi(dxgi).expect("a measured format"),
        levels,
        faces: 1,
        pixels,
    }
}

/// `len` bytes, each its place.
fn bytes(len: usize) -> Vec<u8> {
    (0..len).map(|i| i as u8).collect()
}

#[test]
fn the_level_asked_for_is_the_smallest_at_least_as_large_as_the_screen_size() {
    assert_eq!(level_for(1024, 1024, 256), 2);
    assert_eq!(level_for(1024, 512, 300), 1, "256 would be under it");
    assert_eq!(level_for(1024, 1024, 1024), 0);
    assert_eq!(level_for(1024, 1024, 1025), 0);
    assert_eq!(
        level_for(100, 60, 256),
        0,
        "a texture smaller than asked gives its top"
    );
    assert_eq!(level_for(600, 300, 256), 1, "300 by 150");
    assert_eq!(level_for(2048, 4096, 64), 6, "the longer side decides");
    assert_eq!(level_for(1024, 1024, 0), 10);
    assert_eq!(level_for(1, 1, 1), 0);
}

#[test]
fn block_rows_a_device_samples_are_handed_as_they_are_under_their_own_format() {
    // 8 by 8 pixels of 8-byte blocks: four blocks, then one for each of the levels of 4, 2 and 1.
    let stored = bytes(32 + 8 + 8 + 8);
    for (dxgi, format) in [
        (71, TextureFormat::Bc1RgbaUnorm),
        (72, TextureFormat::Bc1RgbaUnormSrgb),
        (80, TextureFormat::Bc4RUnorm),
    ] {
        let read = cpu_texture(&texture(dxgi, 8, 8, 4, stored.clone())).expect("reads");
        assert_eq!((read.width, read.height, read.format), (8, 8, format));
        let levels: Vec<&[u8]> = read.levels.iter().map(Vec::as_slice).collect();
        assert_eq!(
            levels,
            [
                &stored[..32],
                &stored[32..40],
                &stored[40..48],
                &stored[48..]
            ]
        );
        assert_eq!(read.bytes(), 56);
    }
    // 16-byte blocks, and a top that is not square.
    let stored = bytes(2 * 16 + 16);
    for (dxgi, format) in [
        (77, TextureFormat::Bc3RgbaUnorm),
        (78, TextureFormat::Bc3RgbaUnormSrgb),
        (83, TextureFormat::Bc5RgUnorm),
        (98, TextureFormat::Bc7RgbaUnorm),
        (99, TextureFormat::Bc7RgbaUnormSrgb),
    ] {
        let read = cpu_texture(&texture(dxgi, 8, 4, 2, stored.clone())).expect("reads");
        assert_eq!((read.width, read.height, read.format), (8, 4, format));
        assert_eq!(read.levels, [stored[..32].to_vec(), stored[32..].to_vec()]);
    }
    // Levels the payload does not hold are left out.
    let short = cpu_texture(&texture(71, 8, 8, 4, bytes(40))).expect("reads");
    assert_eq!(short.levels.len(), 2);
}

#[test]
fn a_top_level_that_is_not_whole_blocks_and_a_format_no_device_samples_are_decoded_to_rgba() {
    // 6 by 6 pixels are four BC1 blocks, which a device refuses as a first level.
    let read = cpu_texture(&texture(72, 6, 6, 1, bytes(32))).expect("decodes");
    assert_eq!(
        (read.width, read.height, read.format),
        (6, 6, TextureFormat::Rgba8UnormSrgb)
    );
    assert_eq!(read.levels.len(), 1);
    assert_eq!(read.levels[0].len(), 6 * 6 * 4);
    let linear = cpu_texture(&texture(71, 2, 2, 1, bytes(8))).expect("decodes");
    assert_eq!(linear.format, TextureFormat::Rgba8Unorm);
    assert_eq!(linear.levels[0].len(), 2 * 2 * 4);

    // 8-bit RGBA, sRGB: 4 by 4, then 2 by 2 and 1 by 1.
    let read = cpu_texture(&texture(29, 4, 4, 3, bytes(64 + 16 + 4))).expect("decodes");
    assert_eq!(read.format, TextureFormat::Rgba8UnormSrgb);
    let lengths: Vec<usize> = read.levels.iter().map(Vec::len).collect();
    assert_eq!(lengths, [64, 16, 4]);
    assert_eq!(read.bytes(), 84);
}

#[test]
fn a_key_no_pack_holds_reads_to_nothing() {
    let places = Places {
        packs: Vec::new(),
        keys: HashMap::new(),
        formats: HashMap::new(),
        dictionaries: (PathBuf::new(), HashMap::new()),
    };
    assert!(read_texture(&OpenPacks::default(), &places, 0xC0105, 256).is_none());
    assert!(!places.is_colour(0xC0105));
    let colour = Places {
        formats: HashMap::from([(1, 72 << 3), (2, 71 << 3)]),
        ..places
    };
    assert!(colour.is_colour(1), "BC1 sRGB");
    assert!(!colour.is_colour(2), "BC1 linear");
}

#[test]
fn a_key_no_pack_holds_reads_from_the_texture_archive_s_dictionary_that_holds_it() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (made, _) = crate::state::gta6_edit::tests::dictionary_file(dir.path());
    let root = dir.path().join("install");
    std::fs::create_dir(&root).expect("a folder");
    let key = name_hash64(b"zq_paint");
    let places = crate::state::gta6_texture_index::pack_places(&root);
    let (cancel, progress) = (or_job::Cancel::new(), or_job::Progress::new());
    let (.., (_, bodies)) = places(&cancel, &progress).expect("indexed");
    assert!(
        bodies.is_empty(),
        "a folder with no texture archive holds no dictionary"
    );

    std::fs::copy(&made, root.join("textures.rpf")).expect("copied");
    let places = crate::state::gta6_texture_index::pack_places(&root);
    let (packs, keys, formats, dictionaries) = places(&cancel, &progress).expect("indexed");
    assert_eq!(dictionaries.1, HashMap::from([(key, 0)]));
    let places = Places {
        packs,
        keys,
        formats,
        dictionaries,
    };
    let read = read_texture(&OpenPacks::default(), &places, key, 256).expect("read");
    assert!(read.width > 0 && !read.levels.is_empty());
    assert!(read_texture(&OpenPacks::default(), &places, key ^ 1, 256).is_none());
}

#[test]
fn the_texture_archive_s_keys_are_kept_and_read_back_while_the_archive_is_as_it_was() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (made, _) = crate::state::gta6_edit::tests::dictionary_file(dir.path());
    let root = dir.path().join("install");
    std::fs::create_dir(&root).expect("a folder");
    let archive = root.join("textures.rpf");
    std::fs::copy(&made, &archive).expect("copied");
    let key = name_hash64(b"zq_paint");
    let (cancel, progress) = (or_job::Cancel::new(), or_job::Progress::new());
    let bodies = || {
        let places = crate::state::gta6_texture_index::pack_places(&root);
        let (.., (_, bodies)) = places(&cancel, &progress).expect("indexed");
        bodies
    };
    assert_eq!(bodies(), HashMap::from([(key, 0)]), "scanned");

    // The same size and time with bytes no scan reads: only what was kept can name the key
    let meta = std::fs::metadata(&archive).expect("its size and time");
    let modified = meta.modified().expect("a time");
    std::fs::write(&archive, vec![0u8; meta.len() as usize]).expect("written");
    let file = std::fs::OpenOptions::new().write(true).open(&archive);
    file.and_then(|f| f.set_modified(modified))
        .expect("its time");
    assert_eq!(bodies(), HashMap::from([(key, 0)]), "read back as kept");

    let later = modified + std::time::Duration::from_secs(1);
    let file = std::fs::OpenOptions::new().write(true).open(&archive);
    file.and_then(|f| f.set_modified(later))
        .expect("another time");
    assert!(bodies().is_empty(), "scanned again once its time moved");
}
