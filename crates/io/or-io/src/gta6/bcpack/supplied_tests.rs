//! A made-up tables file, which no coder holds: read, it has to give the tables it was made from,
//! and a body coded through them has to decode as no estimate.
// Outside a test build the made-up fixtures take a few of these helpers and the rest lie unused.
#![cfg_attr(not(test), allow(dead_code, unused_imports))]

use std::path::Path;

use super::super::bptc::RANKS;
use super::super::calibrate::tests::coded_lanes;
use super::super::calibrate::{
    keep_check, keep_read, keep_supplied, load_cache, member_key, save_cache, supplied_path,
    Checked, Standing,
};
use super::super::planes::blocks;
use super::super::tests::encode;
use super::super::{decode_with, Decode};
use super::*;

/// How many ranks of lane 15 each mode takes: its partitions, or its rotations and selections.
const COUNTS: [usize; 8] = [16, 64, 64, 64, 8, 4, 1, 64];

/// A table of modes no pack holds: each mode's ranks in one run, the modes in the order of their numbers.
fn made_up_modes() -> [u8; RANKS] {
    let mut modes = [UNPLACED; RANKS];
    let mut rank = 0;
    for (mode, count) in COUNTS.into_iter().enumerate() {
        modes[rank..rank + count].fill(mode as u8);
        rank += count;
    }
    modes
}

/// The partition a made-up file gives each place among the ranks of mode 0, and of mode 2.
fn made_up_order0(position: usize) -> usize {
    (position * 7 + 3) % 16
}

fn made_up_order2(position: usize) -> usize {
    (position * 37 + 11) % 64
}

/// The four places a made-up file gives each symbol of a quadrant table, 2 bits each: BC1's,
/// and that of BC3's colour.
fn made_up_quadrant(colour_of_bc3: bool, symbol: usize) -> usize {
    match colour_of_bc3 {
        false => (symbol * 77 + 5) % 256,
        true => (symbol * 33 + 7) % 256,
    }
}

/// The two places a made-up file gives each symbol of pair table `table`, of the file's index
/// tables: every pair of places is some symbol's.
fn made_up_places(table: usize, symbol: usize) -> [usize; 2] {
    let (row, column) = (symbol / 8, symbol % 8);
    [(column + table) % 8, (3 * row + column + table) % 8]
}

/// A file no coder holds, in the layout of a tables file.
pub(in crate::gta6) fn made_up_file() -> Vec<u8> {
    let mut file = Vec::with_capacity(length());
    for model in 0..128 {
        let built_in = &FREQ_TABLE[model / 16][model % 16];
        file.extend(built_in[..9].iter().flat_map(|word| word.to_le_bytes()));
    }
    for (table, symbols) in LENGTHS[1..6].iter().copied().enumerate() {
        file.extend((0..symbols).map(|symbol| match table {
            0 => made_up_quadrant(false, symbol) as u8,
            2 => made_up_quadrant(true, symbol) as u8,
            _ => {
                let [first, second] = made_up_places(table, symbol);
                (first | second << 4) as u8
            }
        }));
    }
    let modes = made_up_modes();
    let mut words = vec![0u8; LENGTHS[6]];
    for first in 0..32u16 {
        for second in 0..16u16 {
            let Some(sig) = Sig::of(first, second, &modes) else {
                continue;
            };
            let partition = match sig.mode {
                0 => made_up_order0(sig.position),
                2 => made_up_order2(sig.position),
                1 | 3 | 7 => 63 - sig.position,
                _ => 0,
            };
            let word = sig.mode
                | sig.rotation() << 3
                | sig.selection() << 5
                | sig.subsets() << 6
                | partition << 8;
            let at = 2 * usize::from(second << 5 | first);
            words[at..at + 2].copy_from_slice(&(word as u16).to_le_bytes());
        }
    }
    file.extend(words);
    let colour = |entry: usize| (0..6).map(move |k| ((entry * 7 + 33 * k + 9) % 32) as u8);
    let ends =
        |entry: usize, count: usize| (0..count).map(move |k| ((entry * 5 + 17 * k) % 256) as u8);
    for entry in 0..ENTRIES {
        file.extend(colour(entry).chain([0; 16]));
    }
    for entry in 0..ENTRIES {
        file.extend(colour(entry).chain(ends(entry, 2)).chain([0; 32]));
    }
    for entry in 0..ENTRIES {
        file.extend(ends(entry, 2).chain([0; 16]));
    }
    for entry in 0..ENTRIES {
        file.extend(ends(entry, 4).chain([0; 32]));
    }
    for entry in 0..ENTRIES {
        file.push(1);
        for field in 0..12 {
            file.extend((((entry * 131 + field * 977 + 5) % 4096) as u16 * 16).to_le_bytes());
        }
        // Indices of 3 bits in the even entries, of 4 in the odd ones.
        file.push(if entry % 2 == 0 { 3 } else { 11 });
        for pixel in 0..16 {
            file.extend((((entry + 5 * pixel) % 8) as i16 - 4).to_le_bytes());
        }
    }
    for entry in 0..ENTRIES {
        // A block of mode 6, which says no subsets of its own.
        file.extend(6u16.to_le_bytes());
        for value in 0..24 {
            let end = if value < 8 {
                (entry * 13 + value * 29 + 5) % 256
            } else {
                0
            };
            file.extend(((end as u16) << 8).to_le_bytes());
        }
        file.extend([4, 0]);
        for pixel in 0..16 {
            file.extend((((entry + 5 * pixel) % 16) as i16 - 8).to_le_bytes());
        }
        file.extend([0; 32]);
    }
    file
}

#[test]
fn a_tables_file_reads_into_each_mode_s_tables_by_its_layout() {
    let file = made_up_file();
    assert_eq!(file.len(), length());
    let tables = read(&file).expect("reads");
    assert!(KINDS.into_iter().all(|kind| tables.supplied(kind)));
    assert!(!tables.supplied(0) && !tables.supplied(2) && !tables.supplied(8));
    assert!(KINDS.into_iter().all(|kind| tables.covers(kind)));

    // An index table holds places between a block's two ends, not the codes a block stores.
    let (bc1, bc3) = (tables.of(1).expect("BC1"), tables.of(3).expect("BC3"));
    let (bc4, bc5) = (tables.of(4).expect("BC4"), tables.of(5).expect("BC5"));
    // A colour's last place, 3, is a block's second colour, which BC1 stores as code 1.
    let rising = (0..256).find(|&symbol| made_up_quadrant(false, symbol) == 0b11_10_01_00);
    let rising = rising.expect("every pattern is some symbol's");
    assert_eq!(bc1.quadrants[rising], 0b01_11_10_00);
    // An alpha's last place, 7, is a block's second value, which BC4 stores as code 1.
    let far = (0..64).find(|&symbol| made_up_places(3, symbol) == [1, 7]);
    assert_eq!(
        bc4.pairs[far.expect("every pair is some symbol's")],
        2 | 1 << 3
    );
    for symbol in 0..64 {
        let code = |place: usize| match place {
            0 => 0,
            7 => 1,
            between => between as u8 + 1,
        };
        let pair = |table: usize| {
            let [first, second] = made_up_places(table, symbol);
            code(first) | code(second) << 3
        };
        assert_eq!(
            [bc3.pairs[symbol], bc4.pairs[symbol], bc5.pairs[symbol]],
            [pair(1), pair(3), pair(4)]
        );
    }
    assert_ne!(bc1.quadrants, bc3.quadrants);

    // Entry e is slot 255 - e; an alpha end counts from 128, and BC3's alpha comes after its
    // colour in an entry and before it among a block's fields.
    assert_eq!(bc1.preset[255][..6], [9, 10, 11, 12, 13, 14]);
    assert_eq!(bc1.preset[1][..6], [27, 28, 29, 30, 31, 0]);
    assert_eq!(bc3.preset[255][..8], [0x80, 0x91, 9, 10, 11, 12, 13, 14]);
    assert_eq!(bc4.preset[255][..2], [0x80, 0x91]);
    assert_eq!(bc4.preset[254][..2], [0x85, 0x96]);
    assert_eq!(bc5.preset[255][..4], [0x80, 0x91, 0xA2, 0xB3]);

    // A BC6H field counts from the top bit alone and an index from half its range, at the depth
    // of the mode its entry names; an index is held left-aligned in 4 bits.
    let hdr = tables.hdr();
    assert_eq!(hdr.ends[255][0], (5 << 4) + 0x8000);
    assert_eq!(
        hdr.ends[254][11],
        ((((131 + 11 * 977 + 5) % 4096) << 4) as u16).wrapping_add(0x8000)
    );
    assert!(hdr.ends_bits[1..].iter().flatten().all(|&bits| bits == 16));
    assert_eq!(hdr.first[255][..3], [0, 10, 4]);
    assert_eq!(hdr.first[254][..3], [5, 10, 7]);
    assert!(hdr.first_known[1..].iter().all(|&known| known) && !hdr.first_known[0]);

    // A BC7 end is its top byte; the subsets its entry lacks take the first one's values; an
    // index is held as it is, modulo its range.
    let bptc = tables.bptc();
    assert_eq!(bptc.ends[255][..3], [5, 34, 63]);
    assert_eq!(bptc.ends[255][8..11], [5, 34, 63]);
    assert_eq!(bptc.ends[255][16..19], [5, 34, 63]);
    assert_eq!(bptc.first[255][..3], [8, 13, 2]);
    assert_eq!(bptc.first[254][..3], [9, 14, 3]);
    assert!(bptc.second.iter().flatten().all(|&index| index == 0));
    assert!(bptc.ends_known[1..].iter().flatten().all(|&known| known));
    assert!(bptc.first_known[1..].iter().all(|&known| known));

    // Each rank's mode, and the partitions of modes 0 and 2 in the order the file gives them.
    assert_eq!(bptc.modes, made_up_modes());
    assert_eq!(
        bptc.order0,
        std::array::from_fn(|at| made_up_order0(at) as u8)
    );
    assert_eq!(
        bptc.order2,
        std::array::from_fn(|at| made_up_order2(at) as u8)
    );
}

#[test]
fn a_body_through_supplied_tables_is_no_estimate_and_bc1_reads_them_in_place_of_its_own() {
    let tables = read(&made_up_file()).expect("reads");
    for kind in [1, 3, 4, 5] {
        let own = tables.of(kind).expect("a mode of halves");
        let made = coded_lanes(kind, 64, own);
        let truth = blocks(kind, &made, own).expect("rebuilds");
        assert_eq!(truth.unknown, 0, "a file gives every slot and symbol");
        let (stream, size) = (encode(kind, &made), truth.bytes.len() as u64);

        let (decoded, how) = decode_with(kind, &stream, size, Some(&tables)).expect("decodes");
        assert!(decoded == truth.bytes, "BC{kind}");
        let through_the_file = Decode {
            supplied: 64,
            kinds: 1 << kind,
            unseconded: how.unseconded,
            ..Decode::default()
        };
        assert_eq!(how, through_the_file, "BC{kind} is no estimate");
        // The made-up BC1 values are not the built-in ones, and its blocks that read one say so.
        assert_eq!(how.unseconded > 0, kind == 1, "BC{kind}");

        // The same values held as a fit holds them are an estimate, and BC1's then stand under
        // its built-in ones, which are not the made-up file's.
        let fitted = Tables {
            supplied: 0,
            ..tables.clone()
        };
        let (decoded, how) = decode_with(kind, &stream, size, Some(&fitted)).expect("decodes");
        assert_eq!((how.calibrated, how.unknown), (64, 0), "BC{kind}");
        assert_eq!(decoded == truth.bytes, kind != 1, "BC{kind}");
    }
}

#[test]
fn a_file_that_is_not_the_coder_s_tables_is_refused_by_what_is_wrong_with_it() {
    let file = made_up_file();
    let with = |at: usize, change: fn(u8) -> u8| {
        let mut other = file.clone();
        other[at] = change(other[at]);
        read(&other)
    };
    assert_eq!(
        read(&file[1..]),
        Err(SuppliedError::Length { len: length() - 1 })
    );
    assert_eq!(
        with(18 * 40, |byte| byte ^ 1),
        Err(SuppliedError::Model { model: 40 })
    );

    let refused = |found: Result<Tables, SuppliedError>| match found {
        Err(SuppliedError::Value { table, entry, .. }) => Some((table, entry)),
        _ => None,
    };
    // A third bit in a place of a pair.
    assert_eq!(
        refused(with(start(4) + 9, |byte| byte | 8)),
        Some(("BC4's table of pairs", 9))
    );
    // A colour field wider than 5 bits, in entry 2 of BC1's list and of BC3's.
    assert_eq!(
        refused(with(start(7) + 2 * 22, |_| 32)),
        Some(("BC1's presets", 2))
    );
    assert_eq!(
        refused(with(start(8) + 2 * 40, |_| 32)),
        Some(("BC3's presets", 2))
    );
    // No mode 15, and an index of 4 bits where the entry's mode has 3.
    assert_eq!(
        refused(with(start(11) + 58 + 25, |_| 15)),
        Some(("BC6H's presets", 1))
    );
    assert_eq!(
        refused(with(start(11) + 26, |_| 4)),
        Some(("BC6H's presets", 0))
    );
    // An end past its top byte, a depth no index has, and a second index set.
    assert_eq!(
        refused(with(start(12) + 2, |_| 1)),
        Some(("BC7's presets", 0))
    );
    assert_eq!(
        refused(with(start(12) + 116 + 50, |_| 5)),
        Some(("BC7's presets", 1))
    );
    assert_eq!(
        refused(with(start(12) + 51, |_| 2)),
        Some(("BC7's presets", 0))
    );

    // Lane 15's pairs: the first rank is mode 0's first place, which a file may give any
    // partition under 16, and whose rotation, subsets and mode it may not make up.
    let first = start(6);
    assert!(word(&file, first) & 7 == 0, "symbols 0 and 0 are rank 0");
    let table = "BC7's table of lane 15's pairs";
    assert_eq!(refused(with(first, |byte| byte | 8)), Some((table, 0)));
    assert_eq!(refused(with(first, |byte| byte ^ 0x40)), Some((table, 0)));
    assert_eq!(refused(with(first + 1, |byte| byte | 16)), Some((table, 0)));
    // Two places of mode 0 with one partition leave a partition unnamed.
    assert!(refused(with(first + 1, |byte| byte ^ 1)).is_some());
    // One pair more of mode 0 than it has partitions: the pair of the first rank no mode has.
    let spare = (0..32u16)
        .flat_map(|a| (0..16u16).map(move |b| (a, b)))
        .find(|&(a, b)| bptc::rank(a, b) == Some(285))
        .expect("a pair of every rank");
    let at = first + 2 * usize::from(spare.1 << 5 | spare.0);
    assert_eq!(word(&file, at), 0);
    assert!(refused(with(at + 1, |_| 3)).is_some());
}

#[test]
fn supplied_tables_are_kept_once_and_read_in_place_of_any_calibration() {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let cache = dir.path().join("bcpack");
    let ours = member_key(Path::new("zq/made_up.ttpf"), 4096);
    save_cache(&cache, &Tables::empty(), &[ours]).expect("written");
    assert_eq!(load_cache(&cache, ours), Some(Tables::empty()));

    let file = made_up_file();
    assert!(keep_supplied(&cache, &file[1..]).is_err());
    assert_eq!(
        load_cache(&cache, ours),
        Some(Tables::empty()),
        "a file that does not read is not kept"
    );

    assert_eq!(keep_supplied(&cache, &file), Ok(supplied_path(&cache)));
    let supplied = read(&file).expect("reads");
    assert_eq!(load_cache(&cache, ours), Some(supplied.clone()));
    // A pack no calibration read reads through them too.
    assert_eq!(load_cache(&cache, ours ^ 1), Some(supplied.clone()));

    // What a whole check found is kept, kind by kind, with the digest of the file it read
    // through. Only a format with a rule a wrong table breaks is held by a count of nothing.
    assert_eq!(supplied.standing(), Standing::default());
    let standing = |cache: &Path| load_cache(cache, ours).map(|tables| tables.standing());
    let found = |blocks: u64, disagree: u64| Checked { blocks, disagree };
    let check = |found: &[(u8, Checked)]| keep_check(&cache, &supplied, found).expect("kept");
    let held = check(&[(6, found(9000, 0)), (5, found(700, 0))]);
    assert_eq!(standing(&cache), Some(held));
    assert_eq!(
        (held.failed(), held.held(6), held.held(7), held.held(5)),
        (None, Some(9000), None, None)
    );
    // A later check of packs with no BC6H block leaves BC6H as it stood, and adds BC7.
    let more = check(&[(7, found(300, 0)), (6, found(0, 0))]);
    assert_eq!((more.held(6), more.held(7)), (Some(9000), Some(300)));
    // What is said to be held of a format is the largest check of it, not the last.
    assert_eq!(check(&[(6, found(40, 0))]).held(6), Some(9000));

    // A listing of one pack that counts blocks against the file fails the file for that pack,
    // and that is kept: every later read of the pack is through a file that is not the coder's
    // for it, and nothing is held for it. Another pack reads as the checks left the file.
    let theirs = ours ^ 2;
    let listed = keep_read(&cache, &supplied, theirs, found(86, 5)).expect("kept");
    assert_eq!(
        (listed.failed(), listed.failed_in_pack(), listed.refuted()),
        (None, Some(found(86, 5)), Some(found(86, 5)))
    );
    assert_eq!((listed.held(6), listed.held(7)), (None, None));
    let of = |member: u64| load_cache(&cache, member).map(|tables| tables.standing());
    assert_eq!(of(theirs), Some(listed));
    assert_eq!(
        of(ours).map(|ours| (ours.refuted(), ours.held(6))),
        Some((None, Some(9000)))
    );
    // A later listing that counts fewer does not lessen it, and a whole check that holds does
    // not take it back.
    assert_eq!(
        keep_read(&cache, &supplied, theirs, found(86, 1)).ok(),
        Some(listed)
    );
    check(&[(6, found(40, 0))]);
    assert_eq!(of(theirs), Some(listed));

    // A check that fails BC6H: the file is not the coder's, and nothing of it is held.
    let failed = check(&[(6, found(500, 7))]);
    assert_eq!(failed.failed(), Some(found(500, 7)));
    assert_eq!((failed.held(6), failed.held(7)), (None, None));
    assert_eq!(standing(&cache), Some(failed));
    // The count said of a failure is one check's own, over the kinds it failed: a check that
    // counts fewer leaves it, and one that counts more replaces it and is not added to it.
    let fewer = check(&[(6, found(800, 2)), (7, found(100, 1))]);
    assert_eq!(fewer.failed(), Some(found(500, 7)));
    let most = check(&[(6, found(800, 9)), (7, found(100, 1)), (5, found(64, 0))]);
    assert_eq!(most.failed(), Some(found(900, 10)));
    let failed = most;
    // A check that finds nothing does not take that back, whether it reads no BC6H block or
    // some: it is of other packs, and what the first one found is still so.
    let later = check(&[(7, found(300, 0)), (5, found(64, 0))]);
    assert_eq!(later.failed(), failed.failed());
    let again = check(&[(6, found(9000, 0))]);
    assert_eq!((again, again.held(7)), (later, None));
    assert_eq!(standing(&cache), Some(again));
    // The record stands for that file alone: one that differs in a byte is not checked.
    let mut other = file.clone();
    other[start(9)] ^= 1;
    assert_ne!(read(&other).expect("reads").digest(), supplied.digest());
    keep_supplied(&cache, &other).expect("kept");
    assert_eq!(standing(&cache), Some(Standing::default()));
}

/// The stream of `count` made-up blocks of codec kind `kind`, a mode of halves, coded through
/// the tables of [`made_up_file`]; the blocks; and those tables.
pub(in crate::gta6) fn supplied_body(kind: u8, count: usize) -> (Vec<u8>, Vec<u8>, Tables) {
    let tables = read(&made_up_file()).expect("reads");
    let own = tables.of(kind).expect("a mode of halves");
    let made = coded_lanes(kind, count, own);
    let truth = blocks(kind, &made, own).expect("rebuilds");
    (encode(kind, &made), truth.bytes, tables)
}

#[test]
fn a_probe_holds_the_bc1_blocks_two_tables_give_differently_through_each() {
    use super::super::calibrate::tests::level_above;
    use super::super::calibrate::{probe_tables, LevelData, LevelIn, TextureIn};
    use super::super::lanes;
    // A level coded through the made-up file's BC1 tables, under the level above it.
    let (width, height) = (16, 16);
    let (stream, truth, tables) = supplied_body(1, width * height);
    let coded = lanes(1, &stream, truth.len() as u64).expect("the lanes decode");
    let texture = |_: usize, _: bool| {
        vec![TextureIn {
            signed: false,
            kind: 1,
            levels: vec![
                LevelIn {
                    width: 2 * width,
                    height: 2 * height,
                    data: LevelData::Plain(level_above(1, &truth, width, height)),
                },
                LevelIn {
                    width,
                    height,
                    data: LevelData::Coded {
                        lanes: coded.clone(),
                        at: (0..(width * height) as u32).collect(),
                    },
                },
            ],
        }]
    };
    let found = probe_tables(1, &texture, &tables, &mut |_, _| {}, &|| false);
    let found = found.expect("not stopped");
    // The made-up file's BC1 values are not the built-in ones. Through the file the blocks that
    // read one are the level's own pixels, 16 of 3 channels a block; through the built-in
    // tables they are not.
    assert!(found.bc1_blocks > 0);
    assert_eq!(found.bc1_compared, 48 * found.bc1_blocks);
    assert!(found.bc1_through == 0.0 && found.bc1_built_in > 0.0);
}

/// `file` with entries `a` and `b` of preset list `list`, of the six, changed over.
pub(in crate::gta6) fn with_entries_swapped(
    file: &[u8],
    list: usize,
    a: usize,
    b: usize,
) -> Vec<u8> {
    let (from, size) = (start(7 + list), ENTRY[list]);
    let mut out = file.to_vec();
    let entry = |e: usize| from + e * size..from + (e + 1) * size;
    out[entry(a)].copy_from_slice(&file[entry(b)]);
    out[entry(b)].copy_from_slice(&file[entry(a)]);
    out
}

/// `file` with each of the preset lists `lists`, of the six, moved round by one entry, each
/// entry taking the next one's place.
fn with_lists_rotated(file: &[u8], lists: std::ops::Range<usize>) -> Vec<u8> {
    let mut out = file.to_vec();
    for list in lists {
        out[start(7 + list)..start(8 + list)].rotate_left(ENTRY[list]);
    }
    out
}

#[test]
fn a_file_with_two_entries_swapped_or_its_lists_rotated_reads_and_is_not_called_exact() {
    use super::super::bptc::tests::body_over_the_slots;
    use super::super::Reading;
    let file = made_up_file();
    let right = read(&file).expect("reads");
    let (stream, size) = body_over_the_slots(right.bptc(), 100);
    let (bc4_stream, bc4_blocks, _) = supplied_body(4, 64);
    let through = |file: &[u8]| {
        let tables = read(file).expect("a file of the right layout reads, whatever it holds");
        let (_, bc7) = decode_with(7, &stream, size, Some(&tables)).expect("decodes");
        let (blocks, bc4) = decode_with(4, &bc4_stream, 512, Some(&tables)).expect("decodes");
        (bc7, bc4, blocks == bc4_blocks)
    };
    let unchecked = Standing::default();
    let found = |how: &Decode| Checked {
        blocks: how.supplied,
        disagree: how.disagree(),
    };
    // How a file stands after a whole check that read the BC7 body and the BC4 one.
    let checked = |bc7: &Decode, bc4: &Decode| unchecked.after(&[(7, found(bc7)), (4, found(bc4))]);

    // The file the bodies were coded through: no block speaks against it. The BC7 read is
    // exact once a check has held BC7, and not before: one texture reads few of the entries,
    // and a wrong entry it reads need not break a rule in it.
    let (bc7, bc4, same) = through(&file);
    assert_eq!((bc7.supplied, bc7.kinds, bc7.disagree()), (100, 1 << 7, 0));
    assert_eq!(bc7.reading(unchecked), Reading::Unchecked);
    let held = checked(&bc7, &bc4);
    assert_eq!((held.held(7), held.held(4)), (Some(100), None));
    assert_eq!(bc7.reading(held), Reading::Exact);
    // A check of BC6H blocks says nothing of BC7's tables.
    assert_eq!(
        bc7.reading(unchecked.after(&[(6, found(&bc7))])),
        Reading::Unchecked
    );
    // A body of the texture that was not read: the levels read are not all of it, and it is
    // not called exact, with tables or without.
    let partly = |how: Decode| Decode { unread: 1, ..how };
    assert_eq!(partly(bc7).reading(held), Reading::CodedUnread);
    assert_eq!(
        partly(Decode::default()).reading(unchecked),
        Reading::CodedUnread
    );
    assert_eq!(Decode::default().reading(unchecked), Reading::Exact);
    let clean = bc7;
    // A BC4 block has no rule a wrong table breaks: right as these blocks are, nothing can
    // show it, and no check makes the read exact.
    assert!(same && bc4.disagree() == 0);
    assert_eq!(bc4.reading(unchecked), Reading::Unruled);
    assert_eq!(bc4.reading(held), Reading::Unruled);

    // Entries 0 and 1 of BC7's list changed over: the two blocks that read them hold an
    // anchor's index no block stores, and the read says how many blocks disagree.
    let swapped = with_entries_swapped(&file, 5, 0, 1);
    let (bc7, _, _) = through(&swapped);
    assert_eq!((bc7.unknown, bc7.stray), (0, 2));
    let two_of_them = Reading::Disagrees {
        disagree: 2,
        blocks: 100,
    };
    assert_eq!(bc7.reading(unchecked), two_of_them);
    // A whole check that held the file does not make a read that disagrees exact.
    assert_eq!(bc7.reading(held), two_of_them);
    // Once a listing of its pack has counted those two, the texture beside it that read clean
    // through the same file is not called exact either.
    let here = held.in_pack(found(&bc7));
    assert_eq!(bc7.reading(here), two_of_them);
    assert_eq!(clean.reading(held), Reading::Exact);
    assert_eq!(
        clean.reading(here),
        Reading::NotTheCoders {
            disagree: 2,
            blocks: 100
        }
    );

    // The lists of BC1, BC3, BC4 and BC5 moved round by one entry, BC6H's and BC7's as they
    // were. A whole check holds the file, for no rule sees those four lists; the BC4 body
    // decodes to other blocks, and is not called exact.
    let (bc7, bc4, same) = through(&with_lists_rotated(&file, 0..4));
    let held = checked(&bc7, &bc4);
    assert_eq!((held.failed(), held.held(7)), (None, Some(100)));
    assert!(!same && bc4.disagree() == 0);
    assert_eq!(bc4.reading(held), Reading::Unruled);
    assert_eq!(bc7.reading(held), Reading::Exact);

    // Every list moved round by one entry: half the BC7 blocks say so, a whole check fails, and
    // every read through the file says the file is not the coder's, the BC4 one too.
    let (bc7, bc4, same) = through(&with_lists_rotated(&file, 0..6));
    assert_eq!(bc7.stray, 50);
    assert!(matches!(bc7.reading(unchecked), Reading::Disagrees { .. }));
    assert!(!same && bc4.disagree() == 0);
    let failed = checked(&bc7, &bc4);
    assert_eq!(failed.failed(), Some(found(&bc7)));
    assert!(matches!(bc4.reading(failed), Reading::NotTheCoders { .. }));
    // A later check that reads BC7 blocks and finds nothing against the file leaves it failed.
    let nothing = Checked {
        blocks: 4096,
        disagree: 0,
    };
    let later = failed.after(&[(7, nothing)]);
    assert_eq!((later.failed(), later.held(7)), (failed.failed(), None));
}
