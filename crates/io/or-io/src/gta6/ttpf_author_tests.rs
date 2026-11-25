//! Pack textures replaced from image files written here, saved and read back.

use super::*;
use crate::authoring::TextureBuildOptions;
use crate::codec::texture::{save_png, to_dds, RgbaImage};
use crate::gta6::ttpf_author::{replace_from_image, AuthorError, Replace};

/// A 128x64 BC1 texture in one 4 KiB tile, a lone level.
fn bc1() -> Spec {
    spec(128, 64, 0x238, 1, 1, vec![Body::Stored(vec![0x77; 0x1000])])
}

fn linear() -> Replace {
    Replace {
        build: TextureBuildOptions {
            srgb: false,
            ..TextureBuildOptions::default()
        },
        ..Replace::default()
    }
}

/// A solid red PNG of `width` by `height` in `dir`.
fn red_png(dir: &Path, width: u32, height: u32) -> PathBuf {
    let path = dir.join(format!("red_{width}x{height}.png"));
    let image = RgbaImage {
        width,
        height,
        data: [0xFF, 0, 0, 0xFF].repeat((width * height) as usize),
        is_srgb: false,
    };
    save_png(&path, &image).expect("the PNG is written");
    path
}

/// The pack saved to a new file beside it, opened again.
fn saved_and_opened(pack: &TexturePack, dir: &Path) -> TexturePack {
    let path = dir.join("saved.ttpf");
    let mut bytes = Vec::new();
    pack.save_to(&mut bytes).expect("the pack saves");
    std::fs::write(&path, bytes).expect("written");
    TexturePack::open(&path).expect("the saved pack opens")
}

fn is_red(texture: &Texture) -> bool {
    let image = decode_level(texture, 0).expect("decodes");
    (image.data.chunks(4)).all(|p| p[0] > 0xF0 && p[1] < 0x10 && p[2] < 0x10)
}

#[test]
fn a_texture_replaced_from_a_png_reads_back_with_the_new_pixels_under_its_old_key() {
    let (dir, mut pack) = pack_of(&[bc1(), bc1()]);
    let before = pack.texture(0).expect("reads");
    let png = red_png(dir.path(), 128, 64);
    assert!(!is_red(&pack.texture(1).expect("reads")), "not red before");
    assert_eq!(
        replace_from_image(&mut pack, 1, 0, &png, &linear()).expect("replaced"),
        1
    );
    let saved = saved_and_opened(&pack, dir.path());
    assert_eq!(saved.entries.len(), 2);
    assert_eq!(
        saved.entries[1].key,
        key_of(1),
        "the key, the name's hash, is kept"
    );
    assert_eq!(saved.entries[1], pack.entries[1], "the entry is as it was");
    assert!(
        is_red(&saved.texture(1).expect("reads")),
        "the image's pixels"
    );
    assert_eq!(
        saved.texture(0).expect("reads"),
        before,
        "the other texture is untouched"
    );
}

#[test]
fn an_image_of_another_size_is_refused_unless_it_is_resampled_to_the_texture() {
    let (dir, mut pack) = pack_of(&[bc1(), bc1()]);
    let small = red_png(dir.path(), 64, 32);
    let refused = replace_from_image(&mut pack, 1, 0, &small, &linear());
    assert!(
        matches!(
            refused,
            Err(AuthorError::ImageSize {
                index: 1,
                width: 128,
                height: 64,
                got: (64, 32)
            })
        ),
        "{refused:?}"
    );
    let resize = Replace {
        resize: true,
        ..linear()
    };
    replace_from_image(&mut pack, 1, 0, &small, &resize).expect("resampled");
    let saved = saved_and_opened(&pack, dir.path());
    let texture = saved.texture(1).expect("reads");
    assert_eq!(
        (texture.width, texture.height),
        (128, 64),
        "the texture's size"
    );
    assert!(is_red(&texture));
}

#[test]
fn a_bcpack_coded_texture_is_refused_by_name() {
    for kind in [1, 3, 5, 7] {
        let coded = spec(
            128,
            64,
            0x238,
            1,
            1,
            vec![Body::Coded {
                kind,
                decoded: 0x1000,
            }],
        );
        let (dir, mut pack) = pack_of(&[bc1(), coded]);
        let png = red_png(dir.path(), 128, 64);
        let refused = replace_from_image(&mut pack, 1, 0, &png, &linear())
            .expect_err("a BCPack texture is not replaced");
        assert!(
            matches!(refused, AuthorError::BcPack { index: 1, kind: k } if k == kind),
            "{refused:?}"
        );
        assert!(refused.to_string().contains("BCPack"), "{refused}");
        replace_from_image(&mut pack, 0, 0, &png, &linear()).expect("a zlib or stored one is");
    }
}

#[test]
fn a_dds_of_the_texture_s_format_has_its_blocks_copied_and_another_format_is_refused() {
    let (dir, mut pack) = pack_of(&[bc1(), bc1()]);
    let mine = blocks(32, 16, 8, 9);
    let dds = |code: u16, pixels: Vec<u8>, name: &str| {
        let texture = Texture {
            width: 128,
            height: 64,
            format: format(code),
            levels: 1,
            faces: 1,
            pixels,
        };
        let path = dir.path().join(name);
        to_dds(&texture)
            .and_then(|d| d.save(&path))
            .expect("the DDS is written");
        path
    };
    let copy = Replace {
        dds_mips: true,
        ..linear()
    };
    let bc1_dds = dds(0x238, mine.clone(), "bc1.dds");
    assert_eq!(
        replace_from_image(&mut pack, 1, 0, &bc1_dds, &copy).expect("copied"),
        1
    );
    let saved = saved_and_opened(&pack, dir.path());
    assert!(
        saved.texture(1).expect("reads").pixels == mine,
        "block for block"
    );

    let bc3_dds = dds(0x268, blocks(32, 16, 16, 3), "bc3.dds");
    let refused = replace_from_image(&mut pack, 1, 0, &bc3_dds, &copy);
    assert!(
        matches!(refused, Err(AuthorError::DdsMips { index: 1, .. })),
        "{refused:?}"
    );
    let png = red_png(dir.path(), 128, 64);
    let refused = replace_from_image(&mut pack, 1, 0, &png, &copy);
    assert!(
        matches!(refused, Err(AuthorError::DdsMips { index: 1, .. })),
        "{refused:?}"
    );
}
