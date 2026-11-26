//! A pack whose BC4 level is stored through the coder under made-up tables, which calibrating
//! the pack finds again from the plain level above it.

use super::*;
use crate::gta6::bcpack::calibrate::tests::{level_above, made_up_body};
use crate::gta6::bcpack::{Checked, Reading, Standing};
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
    assert_eq!(how.reading(pack.standing()), Reading::Calibrated);
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
            &texture.pixels == pixels && how.reading(pack.standing()) == Reading::Exact,
            "texture {index}"
        );
    }
}

#[test]
fn a_bc4_level_through_supplied_tables_is_never_called_exact_and_a_failed_check_stands() {
    use crate::gta6::bcpack::supplied::tests::supplied_body;
    use crate::gta6::ttpf::{coverage, keep_checked, score_packs, Coverage};
    // The texture of the first test, its coded level coded through the tables of a made-up file.
    let (stream, coded, tables) = supplied_body(4, 32 * 16);
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
    assert!(!pack.reads_supplied(4));

    let tables = std::sync::Arc::new(tables);
    pack.set_calibration(Some(tables.clone()));
    assert!(pack.reads_supplied(4) && pack.reads_supplied(7) && !pack.reads_supplied(2));
    let (texture, how) = pack.texture_noted(0).expect("reads");
    assert_eq!(texture.levels, 2);
    assert!(texture.pixels == [top, level].concat());
    let through_the_file = bcpack::Decode {
        supplied: 512,
        kinds: 1 << 4,
        ..bcpack::Decode::default()
    };
    assert_eq!(how, through_the_file, "no estimate");
    // A BC4 block has no rule a wrong table breaks and no second table is known: nothing can
    // confirm the read, so it is not called exact.
    assert_eq!(pack.standing(), Standing::default());
    assert_eq!(how.reading(pack.standing()), Reading::Unruled);

    // A check of the pack: no block speaks against the tables, and its coded level meets the plain one.
    let packs = std::slice::from_ref(&pack);
    let covered = coverage(packs, &|| false).expect("not stopped");
    let [bc4] = covered[..] else {
        panic!("one mode: {covered:?}");
    };
    let counted = (bc4.clean, bc4.disagree, bc4.calibrated, bc4.refused);
    assert_eq!((bc4.kind, counted, bc4.blocks), (4, (1, 0, 0, 0), 512));
    assert_eq!((bc4.fitted, bc4.unknown, bc4.stray), (0, 0, 0));
    let scored = score_packs(packs, &tables, &mut |_, _| {}, &|| false).expect("not stopped");
    let summary = (scored.iter())
        .find(|summary| summary.kind == 4)
        .expect("BC4's");
    assert_eq!(
        (summary.bodies, summary.blocks, summary.unknown),
        (1, 512, 0)
    );
    assert_eq!(summary.mean_mismatch(), Some(0.0));

    // The check kept: it found nothing against the file and held nothing of it either, and
    // the read is no more exact than before.
    let kept = tempfile::tempdir().expect("a scratch folder");
    let stood = keep_checked(kept.path(), &tables, &covered).expect("kept");
    assert_eq!((stood.failed(), stood.held(4)), (None, None));
    assert_eq!(how.reading(stood), Reading::Unruled);
    // A check that found BC7 blocks no encoder writes says the file is not the coder's, and so
    // does every read through it, this BC4 one too.
    let strays = Coverage {
        kind: 7,
        clean: 3,
        disagree: 1,
        blocks: 64,
        stray: 5,
        ..Coverage::default()
    };
    let failed = keep_checked(kept.path(), &tables, &[bc4, strays]).expect("kept");
    let (disagree, blocks) = (5, 64);
    assert_eq!(failed.failed(), Some(Checked { blocks, disagree }));
    assert_eq!(
        how.reading(failed),
        Reading::NotTheCoders { disagree, blocks }
    );
    // A later check of this pack alone, which holds no BC7 block, does not take that back.
    let later = keep_checked(kept.path(), &tables, &covered).expect("kept");
    assert_eq!(later, failed);
    // A check that fails two formats says its own count, over both. One after it that counts
    // fewer, of one of them, neither replaces that count nor is added to it.
    let hdr = Coverage {
        kind: 6,
        disagree: 1,
        blocks: 100,
        stray: 3,
        ..Coverage::default()
    };
    let both = keep_checked(kept.path(), &tables, &[strays, hdr]).expect("kept");
    let of_both = Checked {
        blocks: 164,
        disagree: 8,
    };
    assert_eq!(both.failed(), Some(of_both));
    let fewer = Coverage { stray: 1, ..hdr };
    let after = keep_checked(kept.path(), &tables, &[fewer]).expect("kept");
    assert_eq!(after.failed(), Some(of_both));
}

#[test]
fn a_probe_reads_every_bc1_texture_of_a_few_and_one_in_eight_of_many() {
    use super::calibrate::probed_bc1;
    let read = |count: usize| (0..count).filter(|&nth| probed_bc1(count)(nth)).count();
    assert_eq!((read(0), read(11), read(1024)), (0, 11, 1024));
    // The first, the ninth and so on.
    assert_eq!((read(1025), read(1032), read(1033)), (129, 129, 130));
    assert!(probed_bc1(40_000)(0) && !probed_bc1(40_000)(1) && probed_bc1(40_000)(8));
}
