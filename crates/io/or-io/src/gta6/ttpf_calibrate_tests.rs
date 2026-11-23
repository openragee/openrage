//! A pack whose BC4 level is stored through the coder under made-up tables, which calibrating
//! the pack finds again from the plain level above it.

use super::*;
use crate::gta6::bcpack::calibrate::tests::{level_above, made_up_body};
use crate::gta6::ttpf::calibrate_packs;

#[test]
fn a_coded_level_reads_once_its_pack_is_calibrated_and_says_it_is_an_estimate() {
    // A 256 by 128 BC4 texture: its tail tile, then level 1 as one tile through the coder, then
    // level 0, each pixel of level 1 repeated 2 by 2.
    let (stream, coded) = made_up_body(4, 32 * 16);
    let mut level = vec![0u8; coded.len()];
    for y in 0..16 {
        for x in 0..32 {
            let from = 8 * index(TILES_4K_8, x, y);
            level[8 * (32 * y + x)..][..8].copy_from_slice(&coded[from..from + 8]);
        }
    }
    let top = level_above(4, &level, 32, 16);
    let bodies = vec![
        Body::Zlib(vec![0; 4096]),
        Body::Packed {
            kind: 4,
            stored: stream,
            decoded: 4096,
        },
        Body::Zlib(tile(&top, 64, 32, 8, TILES_4K_8)),
    ];
    let (_dir, mut pack) = pack_of(&[spec(256, 128, 0x281, 1, 0, bodies)]);

    pack.check(0).expect("its top mip is plain");
    let plain = pack.texture(0).expect("reads");
    assert_eq!(plain.levels, 1, "the coded level is left out");
    assert!(plain.pixels == top);
    assert!(!pack.is_calibrated());
    assert_eq!(pack.calibration_textures(&|| false), [0]);
    let refused = TtpfError::UnidentifiedCodec { row: 2, kind: 4 }.to_string();
    assert!(refused.contains("ttpf calibrate"), "{refused}");

    assert!(calibrate_packs(std::slice::from_ref(&pack), &mut |_, _| {}, &|| true).is_none());
    let mut told: Vec<(u64, u64)> = Vec::new();
    let found = calibrate_packs(
        std::slice::from_ref(&pack),
        &mut |done, total| told.push((done, total)),
        &|| false,
    )
    .expect("not stopped");
    assert!(
        (told.windows(2)).all(|pair| pair[0].0 * pair[1].1 <= pair[1].0 * pair[0].1),
        "the progress never goes back: {told:?}"
    );
    assert_eq!(found.members, [pack.member_key()]);
    let summary = (found.summaries.iter())
        .find(|summary| summary.kind == 4)
        .expect("BC4's");
    assert_eq!(
        (summary.bodies, summary.blocks, summary.unknown),
        (1, 512, 0)
    );

    pack.set_calibration(Some(std::sync::Arc::new(found.tables)));
    pack.check(0).expect("reads");
    let (texture, how) = pack.texture_noted(0).expect("reads");
    assert_eq!(texture.levels, 2);
    assert!(
        texture.pixels == [top, level].concat(),
        "level 0, then the level coded"
    );
    assert_eq!((how.calibrated, how.unknown), (512, 0));
    assert!(!how.is_exact());
}

#[test]
fn a_plain_texture_reads_as_exact_under_a_calibration() {
    let (specs, expected) = textures();
    let (_dir, mut pack) = pack_of(&specs);
    assert!(pack.calibration_textures(&|| false).is_empty());
    let found = calibrate_packs(std::slice::from_ref(&pack), &mut |_, _| {}, &|| false)
        .expect("not stopped");
    pack.set_calibration(Some(std::sync::Arc::new(found.tables)));
    for (index, pixels) in expected.iter().enumerate() {
        let (texture, how) = pack.texture_noted(index).expect("reads");
        assert!(
            &texture.pixels == pixels && how.is_exact(),
            "texture {index}"
        );
    }
}
