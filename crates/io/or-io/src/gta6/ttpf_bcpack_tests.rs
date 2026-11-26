//! A pack whose BC1 body is stored through the BCPack coder, coded here from made-up blocks.

use super::*;
use crate::gta6::bcpack::tests::{bc1_lanes, encode};
use crate::gta6::bcpack::BcPackError;

/// [`textures`] with texture 0's stored top mip coded as a body of codec kind 1 instead.
fn coded() -> (Vec<Spec>, Vec<Vec<u8>>) {
    let (mut specs, expected) = textures();
    let Body::Stored(tiled) = &specs[0].bodies[1] else {
        panic!("texture 0's second body is its stored top mip");
    };
    let packed = Body::Packed {
        kind: 1,
        stored: encode(1, &bc1_lanes(tiled)),
        decoded: tiled.len() as u64,
    };
    specs[0].bodies[1] = packed;
    (specs, expected)
}

#[test]
fn a_bc1_body_through_the_coder_reads_as_the_blocks_it_codes() {
    let (specs, expected) = coded();
    let (_dir, pack) = pack_of(&specs);
    let row = pack.rows_of(0).expect("rows").start + 1;
    assert_eq!(pack.rows[row].kind, 1);
    pack.check(0)
        .expect("a BC1 coder body is one the pack reads");
    let texture = pack.texture(0).expect("reads");
    assert_eq!(texture.levels, 7);
    assert!(
        texture.pixels == expected[0],
        "every level, the top mip decoded"
    );
}

#[test]
fn a_bc1_body_that_does_not_decode_is_refused_by_its_row_and_the_coder_says_why() {
    let (mut specs, _) = coded();
    let Body::Packed { stored, .. } = &mut specs[0].bodies[1] else {
        panic!("coded");
    };
    stored.truncate(stored.len() / 2);
    let (_dir, pack) = pack_of(&specs);
    let row = pack.rows_of(0).expect("rows").start + 1;
    let refused = pack.texture(0).expect_err("its top mip is cut short");
    assert!(
        matches!(
            refused,
            TtpfError::BcPack { row: r, kind: 1, source: BcPackError::Truncated { .. } } if r == row
        ),
        "{refused}"
    );
    assert!(pack.texture(1).is_ok(), "the other textures read");
}

#[test]
fn a_texture_read_above_a_coder_body_that_does_not_read_says_the_body_was_not_read() {
    use crate::gta6::bcpack::{Reading, Standing};
    // Texture 0's lower levels through the coder as a BC5 body, which reads through no table kept.
    let (mut specs, _) = textures();
    let Body::Zlib(lower) = &specs[0].bodies[0] else {
        panic!("texture 0's first body holds its lower levels as zlib");
    };
    specs[0].bodies[0] = Body::Coded {
        kind: 5,
        decoded: lower.len() as u64,
    };
    let (_dir, pack) = pack_of(&specs);
    let (texture, how) = pack.texture_noted(0).expect("its plain top level reads");
    assert_eq!((texture.levels, how.unread), (1, 1));
    assert_eq!(how.reading(Standing::default()), Reading::CodedUnread);
    // A texture every body of which read says nothing of the kind.
    let (_, how) = pack.texture_noted(1).expect("reads");
    assert_eq!(
        (how.unread, how.reading(Standing::default())),
        (0, Reading::Exact)
    );
}
