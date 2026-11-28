//! One directory behind both envelopes: packs built here with a row table and with none.

use super::*;

/// The directory's file offset, from row 0 of the table.
fn directory_of(bytes: &[u8]) -> usize {
    (u32::from_le_bytes(bytes[16..20].try_into().expect("a row")) & 0xFF_FFFF) as usize * 0x100
}

/// The plain twin of [`build`]'s pack of `specs`: its directory, stating `letter`, then each body
/// as it decodes, so that a file offset is a virtual address.
fn build_plain(specs: &[Spec], letter: u8) -> Vec<u8> {
    let rows = build(specs);
    let at = directory_of(&rows);
    let len = (16 + 32 * specs.len() + 0x40).next_multiple_of(0x1000);
    let mut image = rows[at..at + len].to_vec();
    image[4] = letter;
    for body in specs.iter().flat_map(|spec| &spec.bodies) {
        match body {
            Body::Stored(bytes) | Body::Zlib(bytes) => image.extend(bytes),
            _ => panic!("a body through the coder has no plain form here"),
        }
    }
    image
}

fn plain_of(specs: &[Spec]) -> (tempfile::TempDir, TexturePack) {
    let (dir, pack) = open(&build_plain(specs, b'p'));
    (dir, pack.expect("a file that starts with ETPF opens"))
}

#[test]
fn a_plain_pack_lists_the_entries_of_its_twin_of_rows() {
    let (specs, _) = textures();
    let (_dir, mut rows) = pack_of(&specs);
    let (_dir, mut plain) = plain_of(&specs);
    assert_eq!(
        (rows.envelope(), plain.envelope()),
        (Envelope::Rows, Envelope::Plain)
    );
    assert_eq!(
        (rows.platform(), plain.platform()),
        (Platform::XboxSeries, Platform::Ps5)
    );
    assert!(rows.envelope().has_coder() && !plain.envelope().has_coder());
    assert_eq!(plain.entries.len(), specs.len());
    assert_eq!(plain.entries, rows.entries);
    assert!(plain.rows.is_empty() && plain.is_complete());
    assert_eq!(plain.role(0), None);
    for index in 0..specs.len() {
        assert_eq!(plain.rows_of(index), None);
        assert_eq!(
            plain.range_of(index),
            rows.range_of(index),
            "texture {index}"
        );
    }
    assert_eq!(plain.range_of(specs.len()), None);

    // The coder's tables are for bodies only a pack of rows stores.
    let found = calibrate_packs(&[], &mut |_, _| {}, &|| false).expect("not stopped");
    let tables = Some(std::sync::Arc::new(found.tables));
    rows.set_calibration(tables.clone());
    plain.set_calibration(tables);
    assert!(rows.is_calibrated() && !plain.is_calibrated());
}

#[test]
fn the_image_reads_the_same_bytes_through_both_envelopes() {
    let (specs, _) = textures();
    let image = build_plain(&specs, b'p');
    let (_dir, rows) = pack_of(&specs);
    let (_dir, plain) = plain_of(&specs);
    let size = image.len() as u64;
    assert_eq!(
        (rows.header.virtual_size, plain.header.virtual_size),
        (size, size)
    );
    // Texture 0 is a zlib row then a stored one: a span from inside the first to inside the
    // second, off every page. Bytes 8 to 16 are the directory's count and pool.
    let first = rows.range_of(0).expect("a range");
    let across = (first.start + 0x123, first.end - first.start - 0x456);
    let whole = (0..specs.len()).map(|index| {
        let range = rows.range_of(index).expect("a range");
        (range.start, range.end - range.start)
    });
    for (at, len) in whole.chain([across, (8, 8), (size, 0)]) {
        let expected = &image[at as usize..(at + len) as usize];
        assert!(
            rows.image(at, len).expect("the rows decode") == expected,
            "rows at {at:#x}"
        );
        assert!(
            plain.image(at, len).expect("the file reads") == expected,
            "plain at {at:#x}"
        );
    }
    for pack in [&rows, &plain] {
        assert!(matches!(
            pack.image(size - 1, 2),
            Err(TtpfError::ImageRange { .. })
        ));
    }
}

#[test]
fn a_file_of_neither_magic_is_refused_naming_both() {
    let mut bytes = build_plain(&textures().0, b'p');
    bytes[..4].copy_from_slice(b"NOPE");
    let (_dir, other) = open(&bytes);
    let refused = other.expect_err("neither magic");
    assert!(matches!(
        refused,
        TtpfError::NotATexturePack([b'N', b'O', b'P', b'E'])
    ));
    let said = refused.to_string();
    assert!(said.contains("RDS5") && said.contains("ETPF"), "{said}");
}

#[test]
fn a_directory_of_a_platform_no_pack_is_measured_on_is_refused_by_name() {
    let (specs, _) = textures();
    // Two platforms no GTA VI build is measured on, and a letter of no platform read.
    for (letter, named) in [(b'y', "PC"), (b'o', "PlayStation 4"), (b'x', "0x78")] {
        let mut rows = build(&specs);
        let at = directory_of(&rows) + 4;
        rows[at] = letter;
        for bytes in [rows, build_plain(&specs, letter)] {
            let (_dir, pack) = open(&bytes);
            let refused = pack.expect_err("no pack of that platform is read");
            assert!(matches!(refused, TtpfError::Platform(_)), "{refused}");
            assert!(refused.to_string().contains(named), "{refused}");
        }
    }
    // The Xbox tiles of a texture lie in rows, so a file with none does not state them.
    let (_dir, pack) = open(&build_plain(&specs, b'z'));
    assert!(matches!(
        &pack,
        Err(TtpfError::Malformed(why)) if why.contains("Xbox Series")
    ));
}

/// The PlayStation 5's 64 KiB tiles: the 4 KiB tile's order, then the Xbox 64 KiB tile's other bits.
const PROSPERO_64K_8: &str = "x0 y0 y1 x1 x2 y2 x3 y3 x4 y4 x5 y5 x6";
const PROSPERO_64K_16: &str = "y0 y1 x0 x1 y2 x2 y3 x3 y4 x4 y5 x5";

#[test]
fn a_playstation_5_pack_reads_its_textures_out_of_its_own_64_kib_tiles() {
    // [`textures`], its two textures of tile mode 0 laid in the PlayStation 5's 64 KiB tiles.
    let (mut specs, expected) = textures();
    let tiled = tile(&expected[1], 128, 64, 16, PROSPERO_64K_16);
    specs[1].bodies = vec![Body::Stored(tiled)];
    let tiled = tile(&expected[3], 150, 75, 8, PROSPERO_64K_8);
    specs[3].bodies = vec![Body::Stored(tiled)];
    let (_dir, plain) = plain_of(&specs);
    for (index, (spec, linear)) in specs.iter().zip(&expected).enumerate() {
        let entry = plain.entries[index];
        assert_eq!(
            (entry.key, entry.width, entry.height),
            (key_of(index), spec.width, spec.height)
        );
        plain.check(index).expect("its entry places it");
        let texture = plain.texture(index).expect("its image reads");
        assert_eq!(texture.format, format(spec.code));
        assert!(texture.pixels == *linear, "texture {index}");
        let (from, whole) = plain.texture_from(index, 1).expect("it reads whole");
        assert!(from == 0 && whole.pixels == *linear, "texture {index}");
    }
    assert!(matches!(
        plain.texture(specs.len()),
        Err(TtpfError::NoSuchTexture { .. })
    ));

    // The Xbox Series's image under the PlayStation 5's letter: a 4 KiB tile is the same on both,
    // and a 64 KiB tile is another picture.
    let (xbox, _) = textures();
    let (_dir, rival) = plain_of(&xbox);
    for (index, linear) in expected.iter().enumerate() {
        let same = rival.texture(index).expect("its image reads").pixels == *linear;
        assert_eq!(same, xbox[index].tile_mode != 0, "texture {index}");
    }
}

#[test]
fn a_playstation_5_64_kib_tile_is_the_4_kib_order_then_the_xbox_tile_s_other_bits() {
    // A8, R16F, RGBA8, BC4 and BC7: an element of 1, 2, 4, 8 and 16 bytes.
    for code in [0x208, 0x1B0, 0x0E0, 0x280, 0x310] {
        let format = format(code);
        let bits = |tile: Tile| tile.bits().collect::<Vec<_>>();
        let small = bits(Tile::of(format, false));
        let rest = bits(Tile::of(format, true))
            .into_iter()
            .filter(|bit| !small.contains(bit));
        let expected: Vec<_> = small.iter().copied().chain(rest).collect();
        assert_eq!(bits(Tile::prospero(format, true)), expected, "{code:#x}");
        assert_eq!(bits(Tile::prospero(format, false)), small, "{code:#x}");
    }
}

#[test]
fn a_playstation_5_texture_whose_entry_states_more_than_its_range_holds_is_refused() {
    // A plain file cut inside its last texture's image.
    let (specs, _) = textures();
    let mut bytes = build_plain(&specs, b'p');
    bytes.truncate(bytes.len() - 0x1000);
    let (_dir, pack) = open(&bytes);
    let pack = pack.expect("the directory still reads");
    let last = specs.len() - 1;
    let refused = pack.check(last).expect_err("its image is cut");
    assert!(matches!(&refused, TtpfError::Malformed(why) if why.contains("its range holds")));
    assert!(pack.texture(last).is_err());
    pack.check(last - 1)
        .expect("the texture before it is whole");
}

#[test]
fn a_plain_pack_saves_as_its_file_and_refuses_by_its_platform_each_edit_written_as_rows() {
    let (specs, _) = textures();
    let bytes = build_plain(&specs, b'p');
    let (_dir, mut plain) = plain_of(&specs);
    let mut saved = Vec::new();
    plain.save_to(&mut saved).expect("a plain pack saves");
    assert!(saved == bytes, "the save is the image the file is");

    let shape = Shape {
        width: 64,
        height: 64,
        format: format(0x238),
        mips: None,
    };
    let edits = [
        plain.reshape_levels(0, shape, &[]).err(),
        plain.save_layout().err(),
    ];
    for refused in edits {
        let refused = refused.expect("no edit of a plain pack is written");
        assert!(matches!(refused, TtpfError::EditPlain { .. }), "{refused}");
        assert!(refused.to_string().contains("PlayStation 5"), "{refused}");
    }
}

/// A texture's pixels cut into its levels, top first.
fn levels_of(texture: &Texture) -> Vec<Vec<u8>> {
    let (side, bytes) = (
        texture.format.element_side(),
        texture.format.element_bytes(),
    );
    let mut rest = texture.pixels.as_slice();
    (0..texture.levels)
        .map(|level| {
            let across = |pixels: u32| (pixels >> level).max(1).div_ceil(side) as usize;
            let (level, after) =
                rest.split_at(across(texture.width) * across(texture.height) * bytes);
            rest = after;
            level.to_vec()
        })
        .collect()
}

#[test]
fn a_replaced_texture_of_a_playstation_5_pack_is_written_over_its_own_bytes_and_reads_back() {
    let (mut specs, expected) = textures();
    let tiled = tile(&expected[1], 128, 64, 16, PROSPERO_64K_16);
    specs[1].bodies = vec![Body::Stored(tiled)];
    let tiled = tile(&expected[3], 150, 75, 8, PROSPERO_64K_8);
    specs[3].bodies = vec![Body::Stored(tiled)];
    let bytes = build_plain(&specs, b'p');
    let (_dir, mut plain) = plain_of(&specs);
    let flat = |pack: &TexturePack, index: usize| {
        let entry = pack.entries[index];
        matches!((entry.dimension, entry.slices), (1 | 5, 1))
    };

    // Replaced by itself, every 2D texture is written as the bytes the file holds; any other shape is refused by name.
    let mut replaced = Vec::new();
    for index in 0..specs.len() {
        let levels = levels_of(&plain.texture(index).expect("its image reads"));
        match plain.replace_levels(index, &levels) {
            Ok(()) => replaced.push(index),
            Err(refused) => {
                assert!(!flat(&plain, index), "texture {index}: {refused}");
                assert!(
                    matches!(refused, TtpfError::ReplaceShape { .. }),
                    "{refused}"
                );
            }
        }
    }
    assert!(
        replaced.contains(&1) && replaced.contains(&3),
        "{replaced:?}"
    );
    let mut saved = Vec::new();
    plain.save_to(&mut saved).expect("it saves");
    assert!(
        saved == bytes,
        "a texture replaced by itself changes no byte"
    );

    // Replaced by other elements, it reads back before a save and after one, and no byte outside it moved.
    let mut levels = levels_of(&plain.texture(1).expect("its image reads"));
    levels.iter_mut().flatten().for_each(|byte| *byte = !*byte);
    assert!(matches!(
        plain.replace_levels(1, &levels[1..]),
        Err(TtpfError::Malformed(_))
    ));
    assert_eq!(plain.replaced_mips(1, 0).ok(), Some(levels.len()));
    plain.replace_levels(1, &levels).expect("it replaces");
    let want = levels.concat();
    assert!(plain.texture(1).expect("it reads").pixels == want);
    let mut saved = Vec::new();
    plain.save_to(&mut saved).expect("it saves");
    let range = plain.range_of(1).expect("a range");
    let (from, to) = (range.start as usize, range.end as usize);
    assert_eq!(saved.len(), bytes.len());
    assert!(saved[..from] == bytes[..from] && saved[to..] == bytes[to..]);
    assert!(saved[from..to] != bytes[from..to]);

    let (_dir, again) = open(&saved);
    let again = again.expect("the saved pack opens");
    for (index, linear) in expected.iter().enumerate() {
        let read = again.texture(index).expect("its image reads").pixels;
        assert!(
            read == *if index == 1 { &want } else { linear },
            "texture {index}"
        );
    }
    let mut twice = Vec::new();
    again.save_to(&mut twice).expect("it saves again");
    assert!(twice == saved, "saving a saved pack is byte-identical");
}

/// Install-gated: over a PlayStation 5 build's packs, a 2D texture replaced by itself is written as
/// the bytes its file holds, which holds the writer's placing to the reader's on the real layouts.
#[test]
fn a_2d_texture_of_a_playstation_5_build_replaced_by_itself_changes_no_byte() {
    /// The largest range read, and the most textures tried in one pack.
    const LARGEST: u64 = 16 << 20;
    const IN_A_PACK: usize = 48;
    let root = std::env::var_os("OPENRAGE_GTA6_BUILD")
        .map(PathBuf::from)
        .filter(|root| root.join("eboot.bin").is_file());
    let Some(root) = root else {
        or_test_support::skip_install(
            "a PlayStation 5 texture replaced by itself",
            format_args!("set OPENRAGE_GTA6_BUILD to a PlayStation 5 build"),
        );
        return;
    };
    let (mut packs, mut folders) = (Vec::new(), vec![root]);
    while let Some(folder) = folders.pop() {
        for path in (std::fs::read_dir(&folder).into_iter().flatten().flatten()).map(|e| e.path()) {
            if path.is_dir() {
                folders.push(path);
            } else if path.extension().is_some_and(|ext| ext == "ttpf") {
                packs.push(path);
            }
        }
    }
    packs.sort();
    let (mut same, mut differs) = (0usize, Vec::new());
    let mut refused: std::collections::BTreeMap<String, usize> = Default::default();
    for path in &packs {
        let mut pack = TexturePack::open(path).expect("a pack of the build opens");
        assert_eq!(pack.envelope(), Envelope::Plain, "{}", path.display());
        let step = (pack.entries.len() / IN_A_PACK).max(1);
        for index in (0..pack.entries.len()).step_by(step) {
            let entry = pack.entries[index];
            let Some(range) = pack.range_of(index) else {
                continue;
            };
            let len = range.end - range.start;
            if !matches!((entry.dimension, entry.slices), (1 | 5, 1)) || len > LARGEST {
                continue;
            }
            let Ok(texture) = pack.texture(index) else {
                *refused.entry("does not read".into()).or_default() += 1;
                continue;
            };
            // A signed BC6H texture reads as floats, which are not the elements a replacement takes.
            if texture.format.dxgi() == crate::codec::texture::DxgiFormat::Bc6hSf16 {
                continue;
            }
            let was = pack.image(range.start, len).expect("its range reads");
            match pack.replace_levels(index, &levels_of(&texture)) {
                Ok(()) if pack.image(range.start, len).expect("its range reads") == was => {
                    same += 1
                }
                Ok(()) => differs.push(format!("{} texture {index}", path.display())),
                Err(e) => {
                    *refused
                        .entry(e.to_string().chars().take(60).collect())
                        .or_default() += 1
                }
            }
            *pack.replaced_mut() = Replaced::default();
        }
    }
    println!(
        "{} packs: {same} textures written as the file holds them, {} differ, refused {refused:?}",
        packs.len(),
        differs.len()
    );
    assert!(differs.is_empty(), "{:?}", &differs[..differs.len().min(8)]);
    assert!(same > 0, "no 2D texture of the build was replaced");
}
