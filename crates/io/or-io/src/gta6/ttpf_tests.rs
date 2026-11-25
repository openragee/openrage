//! Packs written here from the documented layout, read back.

use std::io::Write;

use super::super::tiling::Tile;
use super::*;
use crate::codec::texture::decode_level;

fn format(code: u16) -> Format {
    Format::of_code(code).expect("a measured format")
}

/// The in-tile element-index bits, lowest first, as the layout states them.
const TILES_64K_8: &str = "x0 y0 x1 x2 y1 y2 x3 y3 x4 y4 x5 y5 x6";
const TILES_64K_16: &str = "x0 y0 x1 y1 y2 x2 y3 x3 y4 x4 y5 x5";
const TILES_4K_8: &str = "x0 y0 y1 x1 x2 y2 x3 y3 x4";
const TILES_4K_16: &str = "y0 y1 x0 x1 y2 x2 y3 x3";
const TILES_4K_1: &str = "x0 x1 x2 x3 y0 y1 y2 y3 y4 x4 y5 x5";
const TILES_64K_1: &str = "x0 x1 x2 y1 y0 y2 x3 y3 y4 x4 y5 x5 y6 x6 y7 x7";
const TILES_4K_2: &str = "x0 x1 x2 y0 y1 y2 x3 y3 x4 y4 x5";
const TILES_64K_2: &str = "x0 x1 x2 y0 y1 y2 x3 y3 x4 y4 x5 y5 x6 y6 x7";
const TILES_4K_4: &str = "x0 x1 y0 y1 y2 x2 y3 x3 y4 x4";
const TILES_64K_4: &str = "x0 x1 y0 y1 x2 y2 y3 x3 y4 x4 y5 x5 y6 x6";

/// Where the i-th level of a packed mip tail starts in its tail tile, as the layout states it.
const SLOTS_4K_8: [usize; 7] = [0x800, 0x600, 0x500, 0x400, 0x300, 0x200, 0x100];
const SLOTS_4K_16: [usize; 6] = [0x800, 0x600, 0x500, 0x400, 0x300, 0x200];
const SLOTS_64K_1: [usize; 6] = [0x8000, 0x4000, 0x2000, 0x1000, 0x800, 0x600];
const SLOTS_64K_8: [usize; 7] = [0x8000, 0x4000, 0x2000, 0x1000, 0x800, 0x600, 0x500];

fn bits(pattern: &str) -> Vec<(bool, usize)> {
    pattern
        .split(' ')
        .map(|bit| {
            (
                bit.starts_with('x'),
                bit[1..].parse().expect("a bit number"),
            )
        })
        .collect()
}

/// A tile's width and height in elements.
fn shape(pattern: &str) -> (usize, usize) {
    let bits = bits(pattern);
    let xs = bits.iter().filter(|(is_x, _)| *is_x).count();
    (1 << xs, 1 << (bits.len() - xs))
}

/// Element (`x`, `y`)'s index inside its tile under `pattern`.
fn index(pattern: &str, x: usize, y: usize) -> usize {
    let (tile_w, tile_h) = shape(pattern);
    bits(pattern)
        .iter()
        .enumerate()
        .map(|(i, &(is_x, bit))| {
            let coordinate = if is_x { x % tile_w } else { y % tile_h };
            ((coordinate >> bit) & 1) << i
        })
        .sum()
}

/// `linear` blocks laid out in tiles, tile by tile row-major, each tile in `pattern`'s order.
fn tile(linear: &[u8], width: usize, height: usize, element: usize, pattern: &str) -> Vec<u8> {
    let (tile_w, tile_h) = shape(pattern);
    let tile_bytes = tile_w * tile_h * element;
    let (across, down) = (width.div_ceil(tile_w), height.div_ceil(tile_h));
    let mut out = vec![0xEE; across * down * tile_bytes];
    for y in 0..height {
        for x in 0..width {
            let at =
                ((y / tile_h) * across + x / tile_w) * tile_bytes + index(pattern, x, y) * element;
            out[at..at + element].copy_from_slice(&linear[(y * width + x) * element..][..element]);
        }
    }
    out
}

/// A zero tail tile holding each level of `levels` (linear elements, width, height) at its slot.
fn tail(
    levels: &[(Vec<u8>, usize, usize)],
    slots: &[usize],
    element: usize,
    pattern: &str,
) -> Vec<u8> {
    let (tile_w, tile_h) = shape(pattern);
    let mut out = vec![0; tile_w * tile_h * element];
    for ((linear, width, height), slot) in levels.iter().zip(slots) {
        for y in 0..*height {
            for x in 0..*width {
                let at = slot + index(pattern, x, y) * element;
                out[at..at + element]
                    .copy_from_slice(&linear[(y * width + x) * element..][..element]);
            }
        }
    }
    out
}

/// Blocks that differ at every position: the texture's seed and the block's coordinates.
fn blocks(width: usize, height: usize, element: usize, seed: u8) -> Vec<u8> {
    let mut out = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let mut block = vec![seed; element];
            block[1..3].copy_from_slice(&(x as u16).to_le_bytes());
            block[3..5].copy_from_slice(&(y as u16).to_le_bytes());
            out.extend(block);
        }
    }
    out
}

/// Single-channel pixels of 1 or 2 bytes, none zero, that differ between neighbours and seeds.
fn pixels(width: usize, height: usize, element: usize, seed: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let value = 1 + (x + 37 * y + 101 * seed) % 251;
            if element == 1 {
                out.push(value as u8);
            } else {
                out.extend((0x3000 + 3 * value as u16).to_le_bytes());
            }
        }
    }
    out
}

/// The colour of BC1 block (`x`, `y`) of `level`: red from x, green from y, blue from the level.
fn rgb565(x: usize, y: usize, level: usize) -> u16 {
    ((x & 31) << 11 | (y & 63) << 5 | (x >> 5 | level << 4)) as u16
}

/// BC1 blocks of one colour each: both endpoints the colour, every index 0.
fn solid_bc1(width: usize, height: usize, level: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let colour = rgb565(x, y, level).to_le_bytes();
            out.extend([colour[0], colour[1], colour[0], colour[1], 0, 0, 0, 0]);
        }
    }
    out
}

enum Body {
    Stored(Vec<u8>),
    Zlib(Vec<u8>),
    Coded {
        kind: u8,
        decoded: u64,
    },
    /// `stored` as the body of codec kind `kind`, decoding to `decoded` bytes.
    Packed {
        kind: u8,
        stored: Vec<u8>,
        decoded: u64,
    },
}

struct Spec {
    width: u32,
    height: u32,
    code: u16,
    tile_mode: u8,
    mips: u8,
    dimension: u8,
    slices: u16,
    /// The row-size code of each of the first four rows.
    codes: [u8; 4],
    bodies: Vec<Body>,
}

/// The directory key texture `k` of a built pack carries.
fn key_of(k: usize) -> u64 {
    0x0123_4567_89AB_0000 + k as u64
}

/// A pack of `specs`, laid out as the container and directory are measured to be.
fn build(specs: &[Spec]) -> Vec<u8> {
    build_padded(specs, 0)
}

/// [`build`] with `pad` zero bytes after the directory, which row 0's pad states.
fn build_padded(specs: &[Spec], pad: usize) -> Vec<u8> {
    let rows_n = 2 + specs.iter().map(|s| s.bodies.len()).sum::<usize>();
    let table_end = 16 + 8 * rows_n;
    let directory_at = table_end.next_multiple_of(0x100);
    let pool = 0x40;
    let directory_len = (16 + 32 * specs.len() + pool).next_multiple_of(0x1000);
    let mut file = vec![0u8; directory_at + directory_len + pad];
    file[directory_at..directory_at + 4].copy_from_slice(b"ETPF");
    file[directory_at + 4..directory_at + 8].copy_from_slice(&0x27Au32.to_le_bytes());
    file[directory_at + 8..directory_at + 12].copy_from_slice(&(specs.len() as u32).to_le_bytes());
    file[directory_at + 12..directory_at + 16].copy_from_slice(&(pool as u32).to_le_bytes());
    let keys = directory_at + 16 + 16 * specs.len();
    file[keys + 8 * specs.len()..keys + 16 * specs.len() + pool].fill(0xA5);
    for k in 0..specs.len() {
        file[keys + 8 * k..keys + 8 * k + 8].copy_from_slice(&key_of(k).to_le_bytes());
    }

    let mut rows: Vec<(u64, u8, u64, u64)> = vec![(directory_at as u64, 0, 0, pad as u64)];
    let mut va = directory_len as u64;
    for (k, spec) in specs.iter().enumerate() {
        let entry = directory_at + 16 + 16 * k;
        let packed = (0..4).fold(0u64, |p, j| p | u64::from(spec.codes[j]) << (5 * j));
        let words = [
            (spec.width - 1) | (spec.height - 1) << 14 | u32::from(spec.mips) << 28,
            u32::from(spec.code)
                | u32::from(spec.slices - 1) << 10
                | u32::from(spec.dimension) << 21
                | u32::from(spec.tile_mode) << 24,
            (va >> 12) as u32 | (packed as u32) << 21,
            (packed >> 11) as u32,
        ];
        for (i, word) in words.iter().enumerate() {
            file[entry + 4 * i..entry + 4 * i + 4].copy_from_slice(&word.to_le_bytes());
        }
        for body in &spec.bodies {
            let (kind, stored, decoded) = match body {
                Body::Stored(bytes) => (0, bytes.clone(), bytes.len() as u64),
                Body::Zlib(bytes) => {
                    let mut z =
                        flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
                    z.write_all(bytes).expect("zlib");
                    (8, z.finish().expect("zlib"), bytes.len() as u64)
                }
                Body::Coded { kind, decoded } => (*kind, vec![0x31; 0x234], *decoded),
                Body::Packed {
                    kind,
                    stored,
                    decoded,
                } => (*kind, stored.clone(), *decoded),
            };
            let offset = file.len();
            file.extend(&stored);
            file.resize(file.len().next_multiple_of(0x100), 0);
            let pad = file.len() - offset - stored.len();
            rows.push((offset as u64, kind, va, pad as u64));
            va += decoded;
        }
    }
    let (_, kind, _, pad) = rows[rows.len() - 1];
    rows.push((file.len() as u64, kind, va, pad));

    file[..4].copy_from_slice(b"RDS5");
    file[4..8].copy_from_slice(&(rows_n as u32).to_le_bytes());
    file[8..12].copy_from_slice(&(va as u32).to_le_bytes());
    file[12] = (va >> 32) as u8;
    file[15] = 0x16;
    for (i, &(offset, kind, va, pad)) in rows.iter().enumerate() {
        let page = va >> 12;
        let lo = (offset >> 8) as u32 | u32::from(kind) << 26 | ((page & 3) as u32) << 30;
        let hi = (page >> 2) as u32 | (pad as u32) << 20;
        file[16 + 8 * i..20 + 8 * i].copy_from_slice(&lo.to_le_bytes());
        file[20 + 8 * i..24 + 8 * i].copy_from_slice(&hi.to_le_bytes());
    }
    file
}

fn spec(width: u32, height: u32, code: u16, tile_mode: u8, mips: u8, bodies: Vec<Body>) -> Spec {
    Spec {
        width,
        height,
        code,
        tile_mode,
        mips,
        dimension: 1,
        slices: 1,
        codes: [0; 4],
        bodies,
    }
}

/// `levels` levels of linear elements for a `width` by `height`-element top mip, each from `make`.
fn chain_of(
    levels: usize,
    width: usize,
    height: usize,
    make: impl Fn(usize, usize, usize) -> Vec<u8>,
) -> Vec<(Vec<u8>, usize, usize)> {
    (0..levels)
        .map(|level| {
            let (w, h) = ((width >> level).max(1), (height >> level).max(1));
            (make(w, h, level), w, h)
        })
        .collect()
}

fn concat(levels: &[(Vec<u8>, usize, usize)]) -> Vec<u8> {
    levels
        .iter()
        .flat_map(|(bytes, ..)| bytes)
        .copied()
        .collect()
}

/// Four textures, one per BC tile pattern, and the linear blocks each should read back as.
///
/// Texture 0 is BC1 in 4 KiB tiles: mip0 stored, then one zlib body of a tail tile and mip1. The
/// tail holds mips 2 to 6 at their slots, which its nibble counts, and leaves the slots of mips 7
/// and 8 zero.
fn textures() -> (Vec<Spec>, Vec<Vec<u8>>) {
    let bc1 = chain_of(7, 64, 32, solid_bc1);
    let mut lower = tail(&bc1[2..], &SLOTS_4K_8, 8, TILES_4K_8);
    lower.extend(tile(&bc1[1].0, 32, 16, 8, TILES_4K_8));
    let bc7 = blocks(128, 64, 16, 7);
    let bc5 = blocks(32, 16, 16, 5);
    let bc4 = blocks(150, 75, 8, 4);
    let specs = vec![
        spec(
            256,
            128,
            0x239,
            1,
            6,
            vec![
                Body::Zlib(lower),
                Body::Stored(tile(&bc1[0].0, 64, 32, 8, TILES_4K_8)),
            ],
        ),
        spec(
            512,
            256,
            0x310,
            0,
            1,
            vec![Body::Zlib(tile(&bc7, 128, 64, 16, TILES_64K_16))],
        ),
        spec(
            128,
            64,
            0x298,
            2,
            1,
            vec![Body::Stored(tile(&bc5, 32, 16, 16, TILES_4K_16))],
        ),
        spec(
            600,
            300,
            0x280,
            0,
            1,
            vec![Body::Zlib(tile(&bc4, 150, 75, 8, TILES_64K_8))],
        ),
    ];
    (specs, vec![concat(&bc1), bc7, bc5, bc4])
}

/// Eight 16384×16384 R16F textures of one 512 MiB body each, of a codec nothing reads, then
/// [`textures`]'s BC5 texture past 4 GiB of the virtual space.
fn past_4_gib() -> (Vec<Spec>, Vec<u8>) {
    let mut specs: Vec<Spec> = (0..8)
        .map(|_| {
            let body = Body::Coded {
                kind: 3,
                decoded: 1 << 29,
            };
            spec(16384, 16384, 0x1B1, 0, 1, vec![body])
        })
        .collect();
    let (mut small, mut expected) = textures();
    specs.push(small.remove(2));
    (specs, expected.remove(2))
}

fn written(bytes: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let path = dir.path().join("pack.ttpf");
    std::fs::write(&path, bytes).expect("written");
    (dir, path)
}

fn open(bytes: &[u8]) -> (tempfile::TempDir, Result<TexturePack, TtpfError>) {
    let (dir, path) = written(bytes);
    (dir, TexturePack::open(&path))
}

fn pack_of(specs: &[Spec]) -> (tempfile::TempDir, TexturePack) {
    let (dir, pack) = open(&build(specs));
    (dir, pack.expect("the pack opens"))
}

#[test]
fn every_texture_reads_back_to_the_blocks_it_was_tiled_from() {
    let (specs, expected) = textures();
    let (_dir, pack) = pack_of(&specs);
    assert!(pack.is_complete());
    assert_eq!(pack.header.row_count as usize, pack.rows.len());
    assert_eq!(pack.role(0), Some(RowRole::Directory));
    assert_eq!(pack.role(1), Some(RowRole::Body));
    assert_eq!(pack.role(pack.rows.len() - 1), Some(RowRole::Sentinel));
    assert_eq!(pack.role(pack.rows.len()), None);
    assert_eq!(pack.rows_of(0), Some(1..3));

    for (index, expected) in expected.iter().enumerate() {
        let texture = pack.texture(index).expect("the texture reads");
        assert_eq!(pack.entries[index].key, key_of(index));
        assert_eq!(
            (texture.format, texture.faces),
            (format(specs[index].code), 1)
        );
        assert_eq!(
            (texture.width, texture.height),
            (specs[index].width, specs[index].height)
        );
        assert!(texture.pixels == *expected, "texture {index}'s blocks");
    }
    let bc1 = pack.texture(0).expect("reads");
    assert_eq!(
        bc1.levels, 7,
        "mip 1 and the tail's mips 2 to 6 are the six levels row 0's nibble counts"
    );

    let expand = |value: u16, max: u16| {
        ((u32::from(value) * 255 + u32::from(max) / 2) / u32::from(max)) as u8
    };
    for level in 0..7 {
        let image = decode_level(&bc1, level).expect("decodes");
        for (y, x) in [(0, 0), (5, 9), (13, 63)] {
            if x >= image.width.div_ceil(4) as usize || y >= image.height.div_ceil(4) as usize {
                continue;
            }
            let c = rgb565(x, y, level as usize);
            let want = [
                expand(c >> 11, 31),
                expand((c >> 5) & 63, 63),
                expand(c & 31, 31),
                255,
            ];
            let (px, py) = (4 * x as u32, 4 * y as u32);
            assert_eq!(
                image.get_pixel(px, py),
                want,
                "mip {level} block ({x}, {y})"
            );
        }
    }
}

#[test]
fn a_texture_reads_from_a_mip_down_out_of_the_rows_those_mips_lie_in() {
    let (mut specs, expected) = textures();
    let (_dir, pack) = pack_of(&specs);
    // Mip 0 is 64 by 32 blocks of 8 bytes, mip 1 a quarter of that.
    let (top, second) = (64 * 32 * 8, 32 * 16 * 8);
    let (first, texture) = pack.texture_from(0, 1).expect("reads");
    assert_eq!(
        (first, texture.width, texture.height, texture.levels),
        (1, 128, 64, 6)
    );
    assert!(texture.pixels == expected[0][top..]);
    let (first, texture) = pack.texture_from(0, 2).expect("reads");
    assert_eq!(
        (first, texture.width, texture.height, texture.levels),
        (2, 64, 32, 5)
    );
    assert!(texture.pixels == expected[0][top + second..]);
    // A mip past the last is the last.
    let (first, texture) = pack.texture_from(0, 40).expect("reads");
    assert_eq!(
        (first, texture.width, texture.height, texture.levels),
        (6, 4, 2, 1)
    );
    let (first, whole) = pack.texture_from(0, 0).expect("reads");
    assert_eq!(first, 0);
    assert!(whole == pack.texture(0).expect("reads"));

    // The top mip's row through a coder nothing decodes: the texture reads from the mip under it.
    specs[0].bodies[1] = Body::Coded {
        kind: 7,
        decoded: top as u64,
    };
    let (_dir, pack) = pack_of(&specs);
    assert!(matches!(
        pack.texture(0),
        Err(TtpfError::UnidentifiedCodec { kind: 7, .. })
    ));
    let (first, texture) = pack
        .texture_from(0, 0)
        .expect("the mips under the top read");
    assert_eq!((first, texture.width, texture.levels), (1, 128, 6));
    assert!(texture.pixels == expected[0][top..]);
    // And with its first row through one, nothing of it reads.
    specs[0].bodies[0] = Body::Coded {
        kind: 7,
        decoded: 0x2000,
    };
    let (_dir, pack) = pack_of(&specs);
    assert!(matches!(
        pack.texture_from(0, 3),
        Err(TtpfError::UnidentifiedCodec { kind: 7, .. })
    ));
    // With its first row alone through one, the top mip still reads by itself.
    let (mut specs, _) = textures();
    specs[0].bodies[0] = Body::Coded {
        kind: 7,
        decoded: 0x2000,
    };
    let (_dir, pack) = pack_of(&specs);
    let (first, texture) = pack.texture_from(0, 0).expect("the top mip reads");
    assert_eq!((first, texture.levels), (0, 1));
    assert!(texture.pixels == expected[0][..top]);
}

#[test]
fn a_texture_past_4_gib_of_the_virtual_space_reads() {
    let (specs, expected) = past_4_gib();
    let (_dir, pack) = pack_of(&specs);
    assert!(pack.header.virtual_size > u64::from(u32::MAX));
    assert!(
        pack.entries[8].page > 0xF_FFFF,
        "texture 8's page needs bit 20"
    );
    assert!(pack.texture(8).expect("reads").pixels == expected);
    assert!(matches!(
        pack.texture(0),
        Err(TtpfError::UnidentifiedCodec { kind: 3, .. })
    ));
}

#[test]
fn an_unidentified_codec_is_refused_by_its_kind() {
    for kind in [3, 4, 5, 6, 7] {
        let (mut specs, _) = textures();
        specs[2].bodies = vec![Body::Coded {
            kind,
            decoded: 0x2000,
        }];
        let (_dir, pack) = pack_of(&specs);
        let row = pack.rows_of(2).expect("rows").start;
        assert!(
            matches!(pack.texture(2), Err(TtpfError::UnidentifiedCodec { row: r, kind: k }) if (r, k) == (row, kind)),
            "kind {kind}"
        );
        assert!(pack.texture(0).is_ok(), "the other textures read");
    }
}

#[test]
fn a_mip_whose_body_is_unidentified_is_left_out_and_the_top_mip_reads() {
    let (mut specs, expected) = textures();
    specs[0].bodies[0] = Body::Coded {
        kind: 3,
        decoded: 0x2000,
    };
    let (_dir, pack) = pack_of(&specs);
    let texture = pack.texture(0).expect("mip0 reads");
    assert_eq!(texture.levels, 1);
    assert!(texture.pixels == expected[0][..64 * 32 * 8]);
}

#[test]
fn volume_array_and_unmeasured_cube_textures_are_refused_by_name() {
    let refusal = |edit: &dyn Fn(&mut Vec<Spec>), index: usize| {
        let (mut specs, _) = textures();
        edit(&mut specs);
        let (_dir, pack) = pack_of(&specs);
        let (checked, read) = (pack.check(index), pack.texture(index));
        assert_eq!(
            format!("{checked:?}"),
            format!("{:?}", read.as_ref().map(|_| ()))
        );
        read.err()
    };
    for (dimension, slices) in [(5, 16), (1, 2), (2, 5)] {
        let why = refusal(
            &|specs| {
                specs[2].dimension = dimension;
                specs[2].slices = slices;
            },
            2,
        );
        assert!(
            matches!(why, Some(TtpfError::UnknownShape { index: 2, dimension: d, slices: s }) if (d, s) == (dimension, slices)),
            "dimension {dimension}, {slices} slices: {why:?}"
        );
    }
    let why = refusal(&|specs| specs[2].dimension = 3, 2);
    assert!(
        matches!(
            why,
            Some(TtpfError::Volume {
                index: 2,
                tile_mode: 2
            })
        ),
        "{why:?}"
    );

    let why = refusal(
        &|specs| {
            specs[2].dimension = 2;
            specs[2].slices = 6;
        },
        2,
    );
    assert!(
        matches!(why, Some(TtpfError::Malformed(_))),
        "a last row too short for six faces: {why:?}"
    );

    // Only cubes of BC blocks and of 4-byte pixels are measured.
    let body = Body::Coded {
        kind: KIND_ZLIB,
        decoded: 6 * 0x1000,
    };
    let mut cube = spec(64, 64, 0x208, 1, 1, vec![body]);
    (cube.dimension, cube.slices) = (2, 6);
    let (_dir, pack) = pack_of(&[cube]);
    for why in [pack.check(0), pack.texture(0).map(|_| ())] {
        assert!(
            matches!(why, Err(TtpfError::CubeNotMeasured { index: 0, bytes: 1 })),
            "{why:?}"
        );
    }
}

#[test]
fn a_cube_in_64_kib_tile_mode_reads_its_faces_in_4_kib_tiles_before_a_zero_pad() {
    // 128×128 BC6H: each face a tail tile from mip2 down, mip1 in one 4 KiB tile, mip0 in four;
    // six faces of 0x6000 fill 0x24000, and the row rounds up to 48 pages, 0x30000.
    let faces: Vec<_> = (0..6)
        .map(|face| {
            chain_of(8, 32, 32, |w, h, level| {
                blocks(w, h, 16, (0x10 * face + level + 1) as u8)
            })
        })
        .collect();
    let mut row: Vec<u8> = faces
        .iter()
        .flat_map(|levels| {
            let mut unit = tail(&levels[2..], &SLOTS_4K_16, 16, TILES_4K_16);
            unit.extend(tile(&levels[1].0, 16, 16, 16, TILES_4K_16));
            unit.extend(tile(&levels[0].0, 32, 32, 16, TILES_4K_16));
            unit
        })
        .collect();
    assert_eq!(row.len(), 6 * 0x6000);
    row.resize(48 << 12, 0);
    // Flags 3: three earlier rows, the cube at each smaller size, before the last.
    let cube_of = |earlier: usize, last: Vec<u8>| {
        let mut bodies: Vec<Body> = (0..earlier)
            .map(|j| Body::Coded {
                kind: 3,
                decoded: 0x1000 << j,
            })
            .collect();
        bodies.push(Body::Zlib(last));
        // Nibble 5 and flags 3 hold the whole chain of 8 levels.
        let mut cube = spec(128, 128, 0x2FB, 0, 5, bodies);
        (cube.dimension, cube.slices) = (2, 6);
        cube
    };
    let (_dir, pack) = pack_of(&[cube_of(1, row.clone())]);
    assert!(
        matches!(
            pack.texture(0),
            Err(TtpfError::CubeRows {
                index: 0,
                rows: 2,
                flags: 3
            })
        ),
        "a cube of flags 3 has four rows"
    );
    let mut long = row.clone();
    long.resize(64 << 12, 0);
    let (_dir, pack) = pack_of(&[cube_of(3, long)]);
    assert!(
        matches!(pack.check(0), Err(TtpfError::Malformed(_))),
        "a row past the smallest size a code states that holds six faces"
    );
    let (_dir, pack) = pack_of(&[cube_of(3, row.clone())]);
    let texture = pack.texture(0).expect("the cube reads");
    assert_eq!((texture.faces, texture.levels), (6, 8));
    assert_eq!(texture.format.dxgi(), DxgiFormat::Bc6hUf16);
    for (f, levels) in faces.iter().enumerate() {
        let face = texture.face(f as u32).expect("a face");
        assert!(face.pixels == concat(levels), "face {f}");
    }
    decode_level(&texture, 0).expect("BC6H decodes as GTA V's does");

    row[6 * 0x6000 + 5] = 1;
    let (_dir, pack) = pack_of(&[cube_of(3, row)]);
    assert!(
        matches!(pack.texture(0), Err(TtpfError::Malformed(_))),
        "a byte other than zero past the six faces"
    );
}

#[test]
fn a_format_whose_dxgi_number_is_not_measured_is_refused() {
    // DXGI 2, 34, 49, 81, 84 and 127.
    for code in [0x010, 0x110, 0x188, 0x288, 0x2A0, 0x3FF] {
        let (mut specs, _) = textures();
        specs[2].code = code;
        let (_dir, pack) = pack_of(&specs);
        assert!(
            matches!(pack.texture(2), Err(TtpfError::UnknownFormat { index: 2, code: c }) if c == code),
            "{code:#x}"
        );
    }
}

#[test]
fn a_format_code_is_its_dxgi_number_shifted_left_by_3_and_flags() {
    let cases = [
        (0x0E0, DxgiFormat::R8G8B8A8Unorm, 4, 1),
        (0x0E1, DxgiFormat::R8G8B8A8Unorm, 4, 1),
        (0x2B9, DxgiFormat::B8G8R8A8Unorm, 4, 1),
        (0x2D9, DxgiFormat::B8G8R8A8UnormSrgb, 4, 1),
        (0x2FA, DxgiFormat::Bc6hUf16, 16, 4),
        (0x301, DxgiFormat::Bc6hSf16, 16, 4),
        (0x243, DxgiFormat::Bc1UnormSrgb, 8, 4),
        (0x273, DxgiFormat::Bc3UnormSrgb, 16, 4),
        (0x31B, DxgiFormat::Bc7UnormSrgb, 16, 4),
        (0x208, DxgiFormat::A8Unorm, 1, 1),
        (0x1B1, DxgiFormat::R16Float, 2, 1),
        (0x058, DxgiFormat::R16G16B16A16Unorm, 8, 1),
    ];
    for (code, dxgi, bytes, side) in cases {
        let f = format(code);
        assert_eq!(f.dxgi(), dxgi, "{code:#x}");
        assert_eq!(f.dxgi().to_raw(), u32::from(code >> 3));
        assert_eq!(
            (f.element_bytes(), f.element_side()),
            (bytes, side),
            "{code:#x}"
        );
        assert_eq!(f.is_srgb(), dxgi.is_srgb());
    }
    assert_eq!(format(0x2F9).format(), GrcTextureFormat::BC6);
    assert_eq!(format(0x300).format(), GrcTextureFormat::A32B32G32R32F);
}

#[test]
fn rgba8_and_bgra8_read_their_tail_slots_in_both_tile_sizes_and_a_lone_level_at_0() {
    // Pixels that differ at every position and level.
    let rgba = |levels, w, h| {
        chain_of(levels, w, h, |w, h, level| {
            (0..w * h)
                .flat_map(|i| {
                    [
                        level as u8 + 1,
                        (i % w) as u8,
                        (i / w) as u8,
                        0x80 | (i >> 12) as u8,
                    ]
                })
                .collect()
        })
    };
    // 64×64 in 4 KiB tiles: mip0 in four tiles, mip1 in one, mips 2 to 6 in the tail's slots.
    let small = rgba(7, 64, 64);
    let mut body = tail(&small[2..], &SLOTS_4K_8[..5], 4, TILES_4K_4);
    body.extend(tile(&small[1].0, 32, 32, 4, TILES_4K_4));
    body.extend(tile(&small[0].0, 64, 64, 4, TILES_4K_4));
    // 256×128 in 64 KiB tiles: mip0 in two tiles.
    let large = rgba(1, 256, 128);
    // A lone 16×8 level, at offset 0 of its tile.
    let lone = rgba(1, 16, 8);
    // 256×256 in 64 KiB tiles: mip0 in four tiles, then a row of the tail tile, whose five slots
    // hold mips 2 to 6, and mip1; mips 7 and 8 have no slot.
    let wide = rgba(9, 256, 256);
    let mut lower = tail(&wide[2..], &SLOTS_64K_8[..5], 4, TILES_64K_4);
    lower.extend(tile(&wide[1].0, 128, 128, 4, TILES_64K_4));
    let upper = tile(&wide[0].0, 256, 256, 4, TILES_64K_4);
    let specs = vec![
        spec(64, 64, 0x0E0, 1, 7, vec![Body::Zlib(body)]),
        spec(
            256,
            128,
            0x2B8,
            0,
            1,
            vec![Body::Zlib(tile(&large[0].0, 256, 128, 4, TILES_64K_4))],
        ),
        spec(
            16,
            8,
            0x0E0,
            1,
            1,
            vec![Body::Stored(tile(&lone[0].0, 16, 8, 4, TILES_4K_4))],
        ),
        spec(
            256,
            256,
            0x0E1,
            0,
            6,
            vec![Body::Zlib(lower), Body::Zlib(upper)],
        ),
    ];
    let (_dir, pack) = pack_of(&specs);
    let held = [
        (0, &small[..]),
        (1, &large[..]),
        (2, &lone[..]),
        (3, &wide[..7]),
    ];
    for (index, levels) in held {
        let texture = pack.texture(index).expect("reads");
        assert_eq!(texture.levels as usize, levels.len(), "texture {index}");
        assert!(texture.pixels == concat(levels), "texture {index}'s pixels");
    }
    let bgra = pack.texture(1).expect("reads");
    let image = decode_level(&bgra, 0).expect("decodes");
    let p = &large[0].0[(5 * 256 + 9) * 4..][..4];
    assert_eq!(image.get_pixel(9, 5), [p[2], p[1], p[0], p[3]]);
}

#[test]
fn a_signed_bc6h_texture_reads_as_the_floats_its_blocks_decode_to() {
    // A 4×4 texture of one block: mode 0 with endpoints of the sign bit set.
    let mut block = [0u8; 16];
    block[0] = 0b0000_0000;
    block[1..9].fill(0xF0);
    let body = tail(&[(block.to_vec(), 1, 1)], &[0], 16, TILES_4K_16);
    let (_dir, pack) = pack_of(&[spec(4, 4, 0x300, 1, 1, vec![Body::Stored(body)])]);
    let texture = pack.texture(0).expect("reads");
    assert_eq!(texture.pixel_format(), GrcTextureFormat::A32B32G32R32F);
    let mut want = [0f32; 48];
    bcdec_rs::bc6h_float(&block, &mut want, 12, true);
    for (k, rgb) in want.as_chunks::<3>().0.iter().enumerate() {
        let got: Vec<f32> = texture.pixels[16 * k..16 * k + 16]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        assert_eq!(got, [rgb[0], rgb[1], rgb[2], 1.0], "pixel {k}");
    }
    decode_level(&texture, 0).expect("the floats decode");
}

#[test]
fn an_array_in_64_kib_tiles_stores_each_piece_of_every_slice_in_turn() {
    // 1024×1024 BC1 × 2: per slice mip0 in eight 64 KiB tiles, mip1 in two, mips 2 to 8 in the
    // tail's seven slots.
    let slices: Vec<_> = (0..2)
        .map(|s| {
            chain_of(11, 256, 256, |w, h, level| {
                blocks(w, h, 8, (0x40 * s + level + 1) as u8)
            })
        })
        .collect();
    let piece = |s: usize, j: usize| match j {
        0 => tail(&slices[s][2..], &SLOTS_64K_8, 8, TILES_64K_8),
        1 => tile(&slices[s][1].0, 128, 128, 8, TILES_64K_8),
        _ => tile(&slices[s][0].0, 256, 256, 8, TILES_64K_8),
    };
    let bodies = (0..3)
        .flat_map(|j| (0..2).map(move |s| (s, j)))
        .map(|(s, j)| Body::Zlib(piece(s, j)))
        .collect();
    let mut array = spec(1024, 1024, 0x239, 0, 11, bodies);
    (array.dimension, array.slices) = (5, 2);
    let (_dir, pack) = pack_of(&[array]);
    let texture = pack.texture(0).expect("the array reads");
    assert_eq!((texture.faces, texture.levels), (2, 9));
    for (s, levels) in slices.iter().enumerate() {
        let slice = texture.face(s as u32).expect("a slice");
        assert!(slice.pixels == concat(&levels[..9]), "slice {s}");
    }
}

#[test]
fn a_pack_whose_directory_row_has_a_pad_opens() {
    let (specs, expected) = textures();
    let (_dir, pack) = open(&build_padded(&specs, 0x200));
    let pack = pack.expect("the pack opens");
    assert_eq!(pack.rows[0].pad, 0x200);
    assert!(pack.texture(2).expect("reads").pixels == expected[2]);
}

#[test]
fn an_entry_that_reads_as_zeros_is_not_on_disk_and_its_neighbour_s_rows_are_not_known() {
    let (specs, expected) = textures();
    let mut bytes = build(&specs);
    let (_dir, pack) = pack_of(&specs);
    let entry = pack.rows[0].offset as usize + 16 + 16 * 2;
    bytes[entry..entry + 16].fill(0);
    let (_dir, holed) = open(&bytes);
    let holed = holed.expect("the pack opens");
    assert!(!holed.entries[2].on_disk && holed.entries[1].on_disk);
    for index in [1, 2] {
        assert!(
            matches!(holed.texture(index), Err(TtpfError::RangeNotKnown { index: i }) if i == index),
            "texture {index}"
        );
    }
    assert!(holed.texture(3).expect("reads").pixels == expected[3]);
}

#[test]
fn a_row_size_code_states_its_row_s_decoded_size() {
    assert_eq!(
        [1, 8, 10, 13, 16, 21, 24, 28].map(row_code_pages),
        [
            Some(1),
            Some(8),
            Some(12),
            Some(32),
            Some(128),
            Some(18),
            Some(144),
            Some(528)
        ]
    );
    assert_eq!([0, 9, 17, 25, 31].map(row_code_pages), [None; 5]);
    // Texture 0's rows decode to 0x2000 and 0x4000 bytes: two pages, then four.
    let (mut specs, _) = textures();
    specs[0].codes = [2, 4, 0, 0];
    let (_dir, pack) = pack_of(&specs);
    assert_eq!(pack.entries[0].row_codes, [2, 4, 0, 0]);
    pack.texture(0).expect("codes that agree read");
    specs[0].codes = [2, 3, 0, 0];
    let (_dir, pack) = pack_of(&specs);
    assert!(matches!(pack.check(0), Err(TtpfError::Malformed(_))));
}

#[test]
fn a_texture_coder_body_is_refused_with_what_is_known_of_it() {
    let refused = TtpfError::UnidentifiedCodec { row: 3, kind: 3 };
    let said = refused.to_string();
    assert!(
        said.contains("BC3 blocks") && said.contains("not known"),
        "{said}"
    );
    let unseen = TtpfError::UnidentifiedCodec { row: 3, kind: 2 };
    assert!(unseen.to_string().contains("no measured pack"), "{unseen}");
}

#[test]
fn a8_and_r16f_read_in_their_own_tiles_and_r16f_leaves_its_tail_unread() {
    let r8 = |levels, w, h| chain_of(levels, w, h, |w, h, level| pixels(w, h, 1, level));
    let r16f = |levels, w, h| chain_of(levels, w, h, |w, h, level| pixels(w, h, 2, level));
    let zlib = |parts: &[Vec<u8>]| Body::Zlib(parts.concat());

    // 128×128 R8 in 4 KiB tiles: mips 0 and 1 in tiles, 2 to 7 in the tail.
    let small_r8 = r8(8, 128, 128);
    let small_r8_bodies = vec![
        zlib(&[
            tail(&small_r8[2..], &SLOTS_4K_8, 1, TILES_4K_1),
            tile(&small_r8[1].0, 64, 64, 1, TILES_4K_1),
        ]),
        zlib(&[tile(&small_r8[0].0, 128, 128, 1, TILES_4K_1)]),
    ];
    // 512×256 R8 in 64 KiB tiles: the tail's six slots hold mips 2 to 7, and mips 8 and 9 have
    // none, so bytes where an eighth slot would be are not read.
    let large_r8 = r8(10, 512, 256);
    let mut large_tail = tail(&large_r8[2..], &SLOTS_64K_1, 1, TILES_64K_1);
    large_tail[0x500..0x600].fill(0xAA);
    let large_r8_body = zlib(&[
        large_tail,
        tile(&large_r8[1].0, 256, 128, 1, TILES_64K_1),
        tile(&large_r8[0].0, 512, 256, 1, TILES_64K_1),
    ]);
    // R16F: two mips in tiles, then a tail tile whose mip 2 the nibble counts and no slot places.
    let small_r16f = r16f(2, 128, 64);
    let small_r16f_body = zlib(&[
        vec![0x55; 0x1000],
        tile(&small_r16f[1].0, 64, 32, 2, TILES_4K_2),
        tile(&small_r16f[0].0, 128, 64, 2, TILES_4K_2),
    ]);
    let large_r16f = r16f(2, 512, 256);
    let large_r16f_body = zlib(&[
        vec![0x55; 0x10000],
        tile(&large_r16f[1].0, 256, 128, 2, TILES_64K_2),
        tile(&large_r16f[0].0, 512, 256, 2, TILES_64K_2),
    ]);
    let specs = vec![
        spec(128, 128, 0x209, 1, 7, small_r8_bodies),
        spec(512, 256, 0x209, 0, 7, vec![large_r8_body]),
        spec(128, 64, 0x1B0, 3, 3, vec![small_r16f_body]),
        spec(512, 256, 0x1B1, 0, 2, vec![large_r16f_body]),
        spec(32, 16, 0x1B0, 1, 3, vec![Body::Zlib(vec![0x55; 0x1000])]),
        // A lone level in one tile lies at its offset 0.
        spec(4, 4, 0x1B0, 1, 1, vec![Body::Zlib(vec![0x55; 0x1000])]),
        spec(4, 4, 0x209, 1, 1, vec![Body::Zlib(vec![0x55; 0x1000])]),
    ];
    let (_dir, pack) = pack_of(&specs);
    let (a8, r16f) = (format(0x209), format(0x1B0));
    let expected = [
        (a8, &small_r8[..]),
        (a8, &large_r8[..8]),
        (r16f, &small_r16f[..]),
        (r16f, &large_r16f[..]),
    ];
    for (index, (format, levels)) in expected.into_iter().enumerate() {
        let texture = pack.texture(index).expect("reads");
        assert_eq!(texture.format, format, "texture {index}");
        assert_eq!(texture.levels as usize, levels.len(), "texture {index}");
        assert!(texture.pixels == concat(levels), "texture {index}'s pixels");
        let last = texture.levels - 1;
        let image = decode_level(&texture, last).expect("decodes");
        let (_, w, h) = levels[last as usize];
        assert_eq!((image.width, image.height), (w as u32, h as u32));
        if format == a8 {
            let value = levels[last as usize].0[0];
            assert_eq!(image.get_pixel(0, 0), [0, 0, 0, value]);
        }
        let dds = crate::codec::texture::to_dds(&texture).expect("a .dds");
        assert!(dds.data == texture.pixels, "texture {index}'s .dds");
    }
    assert!(
        matches!(
            pack.texture(4),
            Err(TtpfError::TopMipInTail { index: 4, format }) if format == "R16Float"
        ),
        "R16F has no tail slot"
    );
    for index in [5, 6] {
        let texture = pack.texture(index).expect("a lone level reads");
        assert_eq!(texture.levels, 1);
        assert!(texture.pixels.iter().all(|&b| b == 0x55), "texture {index}");
    }
}

#[test]
fn a_texture_wholly_in_its_tail_reads_from_its_slots_unless_it_is_a_lone_level() {
    // 8×8 BC1 is 2×2 blocks, then three mips of one block each.
    let bc1 = chain_of(4, 2, 2, solid_bc1);
    let slotted = || vec![Body::Stored(tail(&bc1, &SLOTS_4K_8, 8, TILES_4K_8))];
    let (_dir, pack) = pack_of(&[spec(8, 8, 0x238, 1, 4, slotted())]);
    let texture = pack.texture(0).expect("reads");
    assert_eq!((texture.levels, &texture.pixels), (4, &concat(&bc1)));

    let lone = || vec![Body::Stored(tile(&bc1[0].0, 2, 2, 8, TILES_4K_8))];
    for mips in [0, 1] {
        let (_dir, pack) = pack_of(&[spec(8, 8, 0x238, 1, mips, lone())]);
        let texture = pack.texture(0).expect("a lone level reads");
        assert_eq!((texture.levels, &texture.pixels), (1, &bc1[0].0));
    }

    let bc7 = chain_of(5, 4, 4, |w, h, level| blocks(w, h, 16, 1 + level as u8));
    let body = vec![Body::Zlib(tail(&bc7, &SLOTS_4K_16, 16, TILES_4K_16))];
    let (_dir, pack) = pack_of(&[spec(16, 16, 0x311, 2, 5, body)]);
    let texture = pack.texture(0).expect("reads");
    assert_eq!((texture.levels, &texture.pixels), (5, &concat(&bc7)));

    let two_tiles = slotted().into_iter().chain(lone()).collect();
    let (_dir, pack) = pack_of(&[spec(8, 8, 0x238, 1, 4, two_tiles)]);
    for refused in [pack.texture(0).map(|_| ()), pack.check(0)] {
        assert!(
            matches!(refused, Err(TtpfError::Malformed(_))),
            "two tiles are no chain of an 8×8 texture: {refused:?}"
        );
    }
}

#[test]
fn a_range_that_is_no_mip_chain_is_refused_before_it_is_inflated() {
    // 256×256 BC7 in 64 KiB tiles is one tile of mip0, with or without a tail tile below it.
    let not_zlib = |decoded| {
        vec![Body::Coded {
            kind: KIND_ZLIB,
            decoded,
        }]
    };
    let (_dir, pack) = pack_of(&[spec(256, 256, 0x310, 0, 1, not_zlib(0x20000))]);
    assert!(
        matches!(pack.texture(0), Err(TtpfError::Inflate { .. })),
        "a chain's body is inflated"
    );
    let (_dir, pack) = pack_of(&[spec(256, 256, 0x310, 0, 1, not_zlib(0x30000))]);
    for refused in [pack.texture(0).map(|_| ()), pack.check(0)] {
        assert!(
            matches!(refused, Err(TtpfError::Malformed(_))),
            "{refused:?}"
        );
    }
}

#[test]
fn a_cube_reads_its_six_faces_from_its_last_row_in_face_order() {
    // 128×128 BC1 in 4 KiB tiles: each face a tail tile from mip1 down, then mip0's two tiles.
    // Nibble 4 and flags 1 hold mips 0 to 4; face 5's tail is zero past them.
    let faces: Vec<_> = (0..6)
        .map(|face| {
            let seed = |level: usize| (0x10 * face + level + 1) as u8;
            chain_of(8, 32, 32, |w, h, level| blocks(w, h, 8, seed(level)))
        })
        .collect();
    let units: Vec<u8> = faces
        .iter()
        .enumerate()
        .flat_map(|(face, levels)| {
            let ends = if face == 5 { 5 } else { 6 };
            let mut unit = tail(&levels[1..ends], &SLOTS_4K_8, 8, TILES_4K_8);
            unit.extend(tile(&levels[0].0, 32, 32, 8, TILES_4K_8));
            unit
        })
        .collect();
    assert_eq!(units.len(), 6 * 0x3000);
    let first_row = Body::Coded {
        kind: 3,
        decoded: 0x6000,
    };
    let mut cube = spec(128, 128, 0x241, 2, 4, vec![first_row, Body::Zlib(units)]);
    (cube.dimension, cube.slices) = (2, 6);

    // 16×16 BC7 in one row: six tail tiles, face t's mip0 at slot 0x800 of tile t.
    let small: Vec<_> = (0..6)
        .map(|face| {
            chain_of(5, 4, 4, |w, h, level| {
                blocks(w, h, 16, (0x20 * face + level + 1) as u8)
            })
        })
        .collect();
    let tails: Vec<u8> = small
        .iter()
        .flat_map(|levels| tail(levels, &SLOTS_4K_16, 16, TILES_4K_16))
        .collect();
    let mut one_row = spec(16, 16, 0x310, 1, 5, vec![Body::Zlib(tails)]);
    (one_row.dimension, one_row.slices) = (2, 6);

    let (_dir, pack) = pack_of(&[cube, one_row]);
    for (index, faces, levels) in [(0, &faces, 5), (1, &small, 5)] {
        assert!(pack.check(index).is_ok());
        let texture = pack.texture(index).expect("the cube reads");
        assert_eq!((texture.faces, texture.levels), (6, levels));
        for (f, chain) in faces.iter().enumerate() {
            let face = texture.face(f as u32).expect("a face");
            assert_eq!((face.faces, face.levels), (1, levels));
            assert!(face.pixels == concat(&chain[..levels as usize]), "face {f}");
        }
        assert!(texture.face(6).is_none());
        assert!(texture.payload() == texture.face(0).expect("+X").pixels);
        decode_level(&texture.face(3).expect("-Y"), 1).expect("decodes");
    }
}

#[test]
fn a_cube_of_4_byte_pixels_reads_each_face_as_the_lone_level_of_one_tile() {
    // 16×16 BGRA8 with nibble 1: face f one 4 KiB tile at 0x1000 × f, its level at offset 0.
    let faces: Vec<Vec<u8>> = (0..6u8)
        .map(|f| {
            (0..16 * 16)
                .flat_map(|i| [f + 1, (i % 16) as u8, (i / 16) as u8, 0xFF])
                .collect()
        })
        .collect();
    let row: Vec<u8> = (faces.iter())
        .flat_map(|face| tail(&[(face.clone(), 16, 16)], &[0], 4, TILES_4K_4))
        .collect();
    let mut cube = spec(16, 16, 0x2B8, 1, 1, vec![Body::Zlib(row.clone())]);
    (cube.dimension, cube.slices) = (2, 6);
    let mut wide = spec(16, 16, 0x050, 1, 1, vec![Body::Zlib(vec![0; 6 << 13])]);
    (wide.dimension, wide.slices) = (2, 6);
    let (_dir, pack) = pack_of(&[cube, wide]);
    let texture = pack.texture(0).expect("the cube reads");
    assert_eq!((texture.faces, texture.levels), (6, 1));
    for (f, face) in faces.iter().enumerate() {
        assert!(
            texture.face(f as u32).expect("a face").pixels == *face,
            "face {f}"
        );
    }
    assert!(
        matches!(
            pack.check(1),
            Err(TtpfError::CubeNotMeasured { index: 1, bytes: 8 })
        ),
        "a cube of 8-byte pixels is not measured"
    );
}

#[test]
fn a_cube_holds_the_levels_its_nibble_and_flags_count_and_reads_a_zero_one_as_black() {
    // 16×16 BC7 in one row, flags 0: face f's five levels in the slots of tail tile f.
    let chains = |zero_face: Option<usize>| -> Vec<Vec<(Vec<u8>, usize, usize)>> {
        (0..6)
            .map(|face| {
                let mut levels = chain_of(5, 4, 4, |w, h, level| {
                    blocks(w, h, 16, (0x30 * face + level + 1) as u8)
                });
                if zero_face == Some(face) {
                    levels[4].0.fill(0);
                }
                levels
            })
            .collect()
    };
    let cube = |nibble: u8, faces: &[Vec<(Vec<u8>, usize, usize)>]| {
        let row = (faces.iter())
            .flat_map(|levels| tail(levels, &SLOTS_4K_16, 16, TILES_4K_16))
            .collect();
        let mut cube = spec(16, 16, 0x310, 1, nibble, vec![Body::Zlib(row)]);
        (cube.dimension, cube.slices) = (2, 6);
        cube
    };
    let (whole, black) = (chains(None), chains(Some(2)));
    let specs = [
        cube(3, &whole[..]),
        cube(5, &black[..]),
        cube(6, &whole[..]),
    ];
    let (_dir, pack) = pack_of(&specs);
    for (index, faces, levels) in [(0, &whole, 3), (1, &black, 5)] {
        let texture = pack.texture(index).expect("the cube reads");
        assert_eq!(
            (texture.faces, texture.levels),
            (6, levels),
            "cube {index}'s faces and levels"
        );
        for (f, chain) in faces.iter().enumerate() {
            let face = texture.face(f as u32).expect("a face");
            assert!(
                face.pixels == concat(&chain[..levels as usize]),
                "cube {index}, face {f}"
            );
        }
    }
    for why in [pack.check(2), pack.texture(2).map(|_| ())] {
        assert!(
            matches!(why, Err(TtpfError::Malformed(_))),
            "six levels where five fit: {why:?}"
        );
    }
}

#[test]
fn r16g16b16a16_unorm_reads_as_8_byte_elements_in_the_8_byte_order() {
    // 128×4 over four tiles, every channel a little-endian u16 and alpha full.
    let texels: Vec<u8> = (0..128 * 4u16)
        .flat_map(|i| [i % 128, i / 128, 3 * i + 1, 0xFFFF])
        .flat_map(u16::to_le_bytes)
        .collect();
    let body = vec![Body::Zlib(tile(&texels, 128, 4, 8, TILES_4K_8))];
    let (_dir, pack) = pack_of(&[spec(128, 4, 0x058, 1, 1, body)]);
    let texture = pack.texture(0).expect("reads");
    assert_eq!(texture.format.dxgi(), DxgiFormat::R16G16B16A16Unorm);
    assert_eq!(texture.pixel_format(), GrcTextureFormat::A16B16G16R16);
    assert_eq!(texture.levels, 1);
    assert!(texture.pixels == texels);
}

#[test]
fn a_2d_texture_holds_the_levels_its_nibble_counts_in_row_0_and_one_per_other_row() {
    // Texture 0's two rows: the tail tile and mip 1, then mip 0; the tail holds mips 2 to 6.
    let (specs, _) = textures();
    let bc1 = chain_of(7, 64, 32, solid_bc1);
    for (nibble, held) in [(0, 2), (2, 3), (4, 5), (6, 7)] {
        let mut specs = textures().0;
        specs[0].mips = nibble;
        let (_dir, pack) = pack_of(&specs);
        let texture = pack.texture(0).expect("reads");
        assert_eq!(texture.levels, held, "nibble {nibble}");
        assert!(
            texture.pixels == concat(&bc1[..held as usize]),
            "nibble {nibble}"
        );
    }
    // Flags 0 put both pieces of mip 1 and mip 0 in row 0, which a nibble of 1 does not count.
    let mut specs = specs;
    (specs[0].code, specs[0].mips) = (0x238, 1);
    let (_dir, pack) = pack_of(&specs);
    assert!(matches!(pack.check(0), Err(TtpfError::Malformed(_))));
}

#[test]
fn every_tile_takes_each_coordinate_bit_once_and_fills_4_or_64_kib() {
    let tiles = [0x238, 0x310, 0x208, 0x1B0, 0x0E0]
        .into_iter()
        .flat_map(|code| {
            let volume = Tile::volume(format(code)).expect("a volume tile");
            [(true, 0x10000), (false, 0x1000)]
                .map(|(large, bytes)| (Tile::of(format(code), large), bytes))
                .into_iter()
                .chain([(volume, 0x1000)])
        });
    for (tile, bytes) in tiles {
        assert_eq!(tile.bytes(), bytes);
        for (axis, side) in [(b'x', tile.width), (b'y', tile.height), (b'z', tile.depth)] {
            let mut taken: Vec<usize> = tile
                .bits()
                .filter(|&(a, _)| a == axis)
                .map(|(_, b)| b)
                .collect();
            taken.sort_unstable();
            assert_eq!(taken, (0..side.ilog2() as usize).collect::<Vec<_>>());
        }
    }
}

#[test]
fn a_short_file_opens_and_reads_the_textures_that_are_whole() {
    let (specs, expected) = textures();
    let whole = build(&specs);
    let (_dir, pack) = pack_of(&specs);
    let cut = pack.rows[pack.rows_of(3).expect("rows").start].offset as usize + 100;
    let (_dir, short) = open(&whole[..cut]);
    let short = short.expect("a short file opens");
    assert!(!short.is_complete());
    for (index, expected) in expected.iter().enumerate().take(3) {
        assert!(short.texture(index).expect("whole").pixels == *expected);
    }
    assert!(matches!(
        short.texture(3),
        Err(TtpfError::NotOnDisk { on_disk, .. }) if on_disk == cut as u64
    ));
}

#[test]
fn a_zlib_stream_with_a_missing_piece_is_refused() {
    let (specs, _) = textures();
    let mut bytes = build(&specs);
    let (_dir, pack) = pack_of(&specs);
    let row = pack.rows_of(1).expect("rows").start;
    let at = pack.rows[row].offset as usize + 0x400;
    bytes[at..at + 0x800].fill(0);
    let (_dir, holed) = open(&bytes);
    assert!(matches!(
        holed.expect("opens").texture(1),
        Err(TtpfError::Inflate { row: r, .. }) if r == row
    ));
}

#[test]
fn a_file_whose_head_is_not_on_disk_is_refused() {
    let (specs, _) = textures();
    let mut bytes = build(&specs);
    let (_dir, cut) = open(&bytes[..40]);
    assert!(matches!(
        cut,
        Err(TtpfError::HeadNotOnDisk { what: "table", .. })
    ));
    bytes[..16].fill(0);
    let (_dir, zeros) = open(&bytes);
    assert!(matches!(zeros, Err(TtpfError::HeadIsZero)));
    bytes[..4].copy_from_slice(b"NOPE");
    let (_dir, other) = open(&bytes);
    assert!(matches!(
        other,
        Err(TtpfError::NotATexturePack([b'N', b'O', b'P', b'E']))
    ));
}

#[test]
fn the_virtual_size_takes_bit_32_from_header_byte_0x0c() {
    let mut bytes = build(&past_4_gib().0);
    bytes[0x0C] = 0;
    let (_dir, pack) = open(&bytes);
    assert!(matches!(pack, Err(TtpfError::Malformed(_))));
}

#[test]
fn a_texture_past_the_directory_is_refused() {
    let (specs, _) = textures();
    let (_dir, pack) = pack_of(&specs);
    assert!(matches!(
        pack.texture(4),
        Err(TtpfError::NoSuchTexture { index: 4, count: 4 })
    ));
}

#[test]
fn a_mip_split_over_many_bodies_reads_back_whole() {
    let bc7 = blocks(128, 128, 16, 9);
    let tiled = tile(&bc7, 128, 128, 16, TILES_4K_16);
    let bodies = tiled
        .chunks(0x1000)
        .map(|c| Body::Zlib(c.to_vec()))
        .collect();
    let (_dir, pack) = pack_of(&[spec(512, 512, 0x310, 1, 1, bodies)]);
    assert_eq!(pack.rows_of(0).map(|rows| rows.len()), Some(64));
    assert!(pack.texture(0).expect("reads").pixels == bc7);
}

#[test]
fn a_sparse_file_of_full_length_is_not_complete() {
    let zeros = 0x20000;
    let bytes = build(&[spec(
        512,
        256,
        0x238,
        0,
        1,
        vec![Body::Stored(vec![0; zeros])],
    )]);
    let (dir, path) = written(&bytes);
    assert!(TexturePack::open(&path).expect("opens").is_complete());

    // The same bytes with the last body unallocated, as a piece a download has not written is.
    let sparse = dir.path().join("sparse.ttpf");
    let file = std::fs::File::create(&sparse).expect("created");
    (&file)
        .write_all(&bytes[..bytes.len() - zeros])
        .expect("written");
    #[cfg(windows)]
    assert!(
        std::process::Command::new("fsutil")
            .args(["sparse", "setflag"])
            .arg(&sparse)
            .status()
            .is_ok_and(|status| status.success()),
        "fsutil marks the file sparse"
    );
    file.set_len(bytes.len() as u64).expect("lengthened");
    drop(file);
    assert_eq!(std::fs::read(&sparse).expect("read"), bytes);
    let pack = TexturePack::open(&sparse).expect("opens");
    assert_eq!(pack.size_on_disk(), bytes.len() as u64);
    assert!(!pack.is_complete());
}

#[test]
fn check_says_what_texture_refuses_without_reading_a_body() {
    let agree = |pack: &TexturePack, index: usize| {
        let (checked, read) = (pack.check(index), pack.texture(index));
        assert_eq!(
            format!("{:?}", checked.err()),
            format!("{:?}", read.err()),
            "texture {index}"
        );
    };
    let (specs, _) = textures();
    let (_dir, pack) = pack_of(&specs);
    for index in 0..=specs.len() {
        agree(&pack, index);
    }

    let mut refusing = textures().0;
    refusing[1].bodies = vec![Body::Coded {
        kind: 5,
        decoded: 0x40000,
    }];
    refusing[2].dimension = 3;
    refusing[3].code = 0x110;
    let (_dir, refused) = pack_of(&refusing);
    for index in 0..refusing.len() {
        agree(&refused, index);
    }
    assert!(refused.check(1).is_err() && refused.check(0).is_ok());

    let whole = build(&specs);
    let cut = pack.rows[pack.rows_of(3).expect("rows").start].offset as usize + 100;
    let (_dir, short) = open(&whole[..cut]);
    let short = short.expect("a short file opens");
    assert!(matches!(short.check(3), Err(TtpfError::NotOnDisk { .. })));
    agree(&short, 3);

    let mut holed = whole;
    let at = pack.rows[pack.rows_of(1).expect("rows").start].offset as usize + 0x400;
    holed[at..at + 0x800].fill(0);
    let (_dir, holed) = open(&holed);
    let holed = holed.expect("opens");
    assert!(holed.check(1).is_ok(), "only inflating finds a hole");
    assert!(matches!(holed.texture(1), Err(TtpfError::Inflate { .. })));
}

#[path = "ttpf_write_tests.rs"]
mod write;

#[path = "ttpf_volume_tests.rs"]
mod volumes;

#[path = "ttpf_relayout_tests.rs"]
mod relayout;

#[path = "ttpf_slice_tests.rs"]
mod slice_replace;

#[path = "ttpf_bcpack_tests.rs"]
mod coder;

#[path = "ttpf_calibrate_tests.rs"]
mod calibrated;
