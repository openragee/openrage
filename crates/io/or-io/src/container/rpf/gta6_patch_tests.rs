//! Hermetic checks of the in-place patch writer, on archives the container writer builds under a
//! synthetic white-box key and on rows laid out by hand; no game byte is involved.

use std::io::Cursor;
use std::path::PathBuf;

use super::*;
use crate::container::rpf::gta6_archive::{Gta6Archive, Gta6Cipher};
use crate::container::rpf::gta6_chunk_table::{author, one_entry_archive, Table};
use crate::container::rpf::gta6_container_write::{build, Rpf9IndexKey, Rpf9NewBody};
use crate::platform::Platform;

const KEY_ID: u32 = 0xDEAD_BEEF;
const REGISTRY_ID: u64 = 0xDEAD_BEEF_0200_0000;
const BOOTSTRAP_REGISTRY_ID: u64 = 0x1000_0001_0200_0000;
const CONTENT_KEY_ID: u32 = 0x1000_0029;
/// The round tables feeding each output lane of the index cipher.
const LANES: [[usize; 4]; 4] = [[0, 13, 10, 7], [1, 14, 4, 11], [5, 2, 8, 15], [9, 6, 12, 3]];
const WORDS: usize = 60;
const MAPS: usize = 16;
const MAP_LEN: usize = 256;
const BLOB_LEN: usize = 4 + WORDS * 4 + MAPS * MAP_LEN;

fn push_perm(out: &mut Vec<u8>, k: usize) {
    for x in 0..256usize {
        out.push(((x + k * 7 + 1) % 256) as u8);
    }
}

fn mix8(b: u8) -> u8 {
    let mut y = 0u8;
    let mut acc = 0u8;
    for i in (0..8).rev() {
        acc ^= (b >> i) & 1;
        y |= acc << i;
    }
    y
}

/// A synthetic snapshot: the bootstrap and one archive index key (byte permutations and invertible
/// affine round tables, the shape the encrypt side inverts), then one content key.
fn snapshot() -> Vec<u8> {
    let mut slot_of = [0usize; 16];
    for lane in LANES.iter() {
        for (slot, &k) in lane.iter().enumerate() {
            slot_of[k] = slot;
        }
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"ORGTAVIK");
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&0x4342u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 32]);
    out.extend_from_slice(&[0x11u8; 16]);
    out.extend_from_slice(&((MAPS * MAP_LEN) as u32).to_le_bytes());
    for k in 0..MAPS {
        push_perm(&mut out, k);
    }
    out.extend_from_slice(&((MAPS * MAP_LEN * 4) as u32).to_le_bytes());
    for (k, &slot) in slot_of.iter().enumerate() {
        let constant = 0x0A0B_0C0Du32.wrapping_mul(k as u32 + 1);
        for x in 0..256u32 {
            let s = (x as u8).rotate_left(3) ^ (k as u8).wrapping_mul(0x5B);
            out.extend_from_slice(&(constant ^ (u32::from(mix8(s)) << (slot * 8))).to_le_bytes());
        }
    }
    out.extend_from_slice(&2u32.to_le_bytes());
    for &id in &[BOOTSTRAP_REGISTRY_ID, REGISTRY_ID] {
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&(BLOB_LEN as u32).to_le_bytes());
        out.extend_from_slice(&[0x8e, 0x9b, 0x3b, 0x4e]);
        for w in 0..WORDS as u32 {
            out.extend_from_slice(&(w.wrapping_mul(2_654_435_761) ^ id as u32).to_le_bytes());
        }
        for k in 0..MAPS {
            push_perm(&mut out, k + 3);
        }
    }
    // The content section: CTR tag, the two shared tables, one key blob.
    out.extend_from_slice(&0x5243u32.to_le_bytes());
    out.extend_from_slice(&4096u32.to_le_bytes());
    out.extend((0..4096).map(|i| (i * 3 + 1) as u8));
    out.extend_from_slice(&16384u32.to_le_bytes());
    out.extend((0..16384).map(|i| (i * 5 + 7) as u8));
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&((u64::from(CONTENT_KEY_ID) << 32) | 0x0200_0000).to_le_bytes());
    out.extend_from_slice(&4340u32.to_le_bytes());
    out.extend_from_slice(&[0x5d, 0xce, 0x18, 0x27]);
    out.extend((0..60u32).flat_map(|w| w.wrapping_mul(0x12345).to_le_bytes()));
    out.extend((0..4096).map(|i| (i * 7 + 3) as u8));
    out
}

fn keys() -> GtaViKeys {
    GtaViKeys::parse(&snapshot()).expect("the synthetic snapshot parses")
}

fn xorshift(n: usize, seed: u32) -> impl Iterator<Item = u32> {
    let mut x = seed.wrapping_mul(0x9E37_79B9) | 1;
    (0..n).map(move |_| {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    })
}

/// Bytes of sixteen letters, which deflate to about half.
fn letters(n: usize, seed: u32) -> Vec<u8> {
    xorshift(n, seed).map(|x| b'a' + (x >> 28) as u8).collect()
}

/// Bytes that do not deflate.
fn noise(n: usize, seed: u32) -> Vec<u8> {
    xorshift(n, seed).map(|x| (x >> 24) as u8).collect()
}

/// A body the writer stores as a zlib chunk, a stored chunk and a zlib tail.
fn body() -> Vec<u8> {
    [letters(0x10000, 1), noise(0x10000, 2), letters(20_000, 3)].concat()
}

fn built(entries: Vec<(&str, Rpf9NewBody)>) -> Vec<u8> {
    let entries: Vec<(String, Rpf9NewBody)> = entries
        .into_iter()
        .map(|(n, b)| (n.to_owned(), b))
        .collect();
    let key = Rpf9IndexKey {
        key_id: KEY_ID,
        iv: [0x33; 16],
    };
    let plan = build(&entries, Platform::XboxSeries, &key, &keys()).expect("built");
    let mut out = Vec::new();
    plan.write_to(&mut Cursor::new(Vec::<u8>::new()), &mut out)
        .expect("written");
    out
}

/// A clear headerless resource's 20-byte row is read past its 16-byte zero slot, as the resource it holds.
#[test]
fn a_clear_headerless_row_reads_past_its_slot_as_its_resource() {
    use crate::container::rpf::{read_rpf9_body, write_rpf9_resource, Rpf9Form, Rpf9Sections};
    let mut virtual_section: Vec<u8> = (0..0x200)
        .map(|i| ((i * 13 + 2) % 251) as u8 | 0x80)
        .collect();
    virtual_section[..8].fill(0);
    virtual_section[8..16].copy_from_slice(&0x5000_0040u64.to_le_bytes());
    virtual_section[0x40..0x70].fill(0);
    let loose = write_rpf9_resource(&Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section,
        physical: Vec::new(),
    })
    .expect("a loose resource");
    let (_, want) = read_rpf9_body(Cursor::new(&loose), 0, u64::MAX).expect("the loose file reads");
    let file = built(vec![(
        "zq/a.ydr",
        Rpf9NewBody::Resource {
            type_code: 0x02,
            loose,
        },
    )]);
    let mount = Gta6Mount::open_root(&mut Cursor::new(&file[..]), &keys()).expect("opens");
    let entry = &mount.archive.entries[0];
    assert!(
        entry.stored.is_none() && entry.cipher.is_none(),
        "a clear 20-byte row"
    );
    let read = mount
        .read_entry(Cursor::new(&file[..]), entry)
        .expect("the row reads");
    assert_eq!(read, want, "the resource past the slot");
}

/// `bytes` as a source file in a scratch folder, and where its copy goes.
fn on_disk(bytes: &[u8]) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let src = dir.path().join("source.rpf");
    std::fs::write(&src, bytes).expect("the source is written");
    let out = dir.path().join("patched.rpf");
    (dir, src, out)
}

/// Row `entry` of the archive at `base` in `file`, read by the mount's own reader.
fn read_back(file: &Path, base: u64, entry: u32) -> Vec<u8> {
    let mut f = File::open(file).expect("open the copy");
    let mount = Gta6Mount {
        base,
        archive: open_at(&mut f, base, &keys()).expect("the copy's table opens"),
    };
    let e = &mount.archive.entries[entry as usize];
    mount.read_entry(&mut f, e).expect("the row reads back")
}

fn toc_end(file: &[u8], base: usize) -> u64 {
    let header = Rpf9Header::parse(&file[base..base + RPF9_HEADER_SIZE]).expect("a header");
    base as u64 + u64::from(header.toc_end)
}

#[test]
fn a_changed_chunk_that_fits_is_written_and_reads_back_with_every_table_byte_kept() {
    let file = built(vec![
        ("docs/a.txt", Rpf9NewBody::Chunked(body())),
        ("docs/b.txt", Rpf9NewBody::Chunked(letters(30_000, 9))),
    ]);
    let (_dir, src, out) = on_disk(&file);
    let mount = Gta6Mount::open_root(&mut Cursor::new(&file[..]), &keys()).expect("opens");
    let index = |len: usize| {
        mount
            .archive
            .entries
            .iter()
            .position(|e| e.real == Some(len as u64))
            .expect("the entry") as u32
    };
    let (a, b) = (index(body().len()), index(30_000));

    // Letters made runs of one letter deflate shorter than the tail's stored size.
    let mut new = body();
    new[0x20000 + 100..0x20000 + 4100].fill(b'a');
    let patch = BodyPatch {
        base: 0,
        entry: a,
        body: new.clone(),
    };
    let report = write_patched_copy(&src, &out, &keys(), &[patch]).expect("patched");
    assert_eq!(
        std::fs::read(&src).unwrap(),
        file,
        "the source is untouched"
    );

    let plan = &report.plans[0];
    let shape: Vec<(bool, bool)> = plan.chunks.iter().map(|c| (c.zlib, c.changed)).collect();
    assert_eq!(shape, [(true, false), (false, false), (true, true)]);
    assert_eq!(report.chunks_changed, 1);
    let tail = plan.chunks[2];
    assert_eq!(report.bytes_written, u64::from(tail.ceiling));
    assert!(tail.new_stored < tail.ceiling, "{tail:?}");

    // Every byte but the one chunk's is the source's, the table among them.
    let copy = std::fs::read(&out).unwrap();
    assert_eq!(copy.len(), file.len());
    let (at, bytes) = &plan.writes[0];
    let (from, to) = (*at as usize, *at as usize + bytes.len());
    assert!(copy[..from] == file[..from] && copy[to..] == file[to..]);
    assert!(report.table_spans.contains(&(0, toc_end(&file, 0))));
    assert!(from as u64 >= toc_end(&file, 0));

    assert_eq!(read_back(&out, 0, a), new);
    assert_eq!(read_back(&out, 0, b), letters(30_000, 9));
}

#[test]
fn an_unchanged_body_patches_to_a_byte_identical_copy() {
    let file = built(vec![("docs/a.txt", Rpf9NewBody::Chunked(body()))]);
    let (_dir, src, out) = on_disk(&file);
    let patch = BodyPatch {
        base: 0,
        entry: 0,
        body: body(),
    };
    let report = write_patched_copy(&src, &out, &keys(), &[patch]).expect("patched");
    assert_eq!((report.chunks_changed, report.bytes_written), (0, 0));
    assert_eq!(std::fs::read(&out).unwrap(), file);
    let mut partial = out.as_os_str().to_owned();
    partial.push(".partial");
    assert!(
        !std::path::Path::new(&partial).exists(),
        "the partial copy is renamed"
    );
}

#[test]
fn a_stored_chunk_takes_any_content_of_its_length() {
    let file = built(vec![("docs/a.txt", Rpf9NewBody::Chunked(body()))]);
    let (_dir, src, out) = on_disk(&file);
    let mut new = body();
    new[0x10000..0x20000].copy_from_slice(&noise(0x10000, 8));
    let patch = BodyPatch {
        base: 0,
        entry: 0,
        body: new.clone(),
    };
    let report = write_patched_copy(&src, &out, &keys(), &[patch]).expect("patched");
    let stored = report.plans[0].chunks[1];
    assert!(stored.changed && !stored.zlib);
    assert_eq!((stored.new_stored, stored.ceiling), (0x10000, 0x10000));
    assert_eq!(report.chunks_changed, 1);
    assert_eq!(read_back(&out, 0, 0), new);
}

#[test]
fn what_cannot_go_in_place_is_refused_by_name_and_writes_nothing() {
    let file = built(vec![("docs/a.txt", Rpf9NewBody::Chunked(body()))]);
    let (_dir, src, out) = on_disk(&file);
    let plan = |body: Vec<u8>, entry: u32| {
        let patch = BodyPatch {
            base: 0,
            entry,
            body,
        };
        plan_patch(&mut Cursor::new(&file[..]), &keys(), &patch)
    };
    let want = body().len() as u64;
    assert!(matches!(
        plan([body(), vec![0]].concat(), 0),
        Err(PatchRefusal::Length { entry: 0, got, want: w }) if got == want + 1 && w == want
    ));
    let mut noisy = body();
    noisy[..0x10000].copy_from_slice(&noise(0x10000, 7));
    let refused = plan(noisy.clone(), 0).expect_err("noise fits no deflated chunk");
    let PatchRefusal::ChunkDoesNotFit {
        entry: 0,
        chunk: 0,
        deflated,
        optimal,
        ceiling,
        ..
    } = &refused
    else {
        panic!("{refused}");
    };
    let ceiling = *ceiling as usize;
    assert!(*deflated > ceiling && *optimal > ceiling, "{refused}");
    let said = refused.to_string();
    assert!(
        said.contains(&format!("{deflated} bytes by the tree's deflater"))
            && said.contains(&format!("to {optimal} by an optimal parse")),
        "{said}"
    );
    assert!(matches!(plan(body(), 5), Err(PatchRefusal::NoRow { .. })));

    let patch = |body: Vec<u8>| BodyPatch {
        base: 0,
        entry: 0,
        body,
    };
    assert!(write_patched_copy(&src, &out, &keys(), &[patch(noisy)]).is_err());
    assert!(!out.exists(), "a refused patch writes no copy");
    assert!(write_patched_copy(&src, &src, &keys(), &[]).is_err());
    std::fs::write(&out, b"kept").unwrap();
    let refused = write_patched_copy(&src, &out, &keys(), &[patch(body())]).unwrap_err();
    assert!(
        refused.to_string().contains("only to a new file"),
        "{refused}"
    );
    assert_eq!(
        std::fs::read(&out).unwrap(),
        b"kept",
        "an existing file is left as it is"
    );
    std::fs::remove_file(&out).unwrap();
    assert!(write_patched_copy(&src, &out, &keys(), &[patch(body()), patch(body())]).is_err());
    assert_eq!(std::fs::read(&src).unwrap(), file);
}

#[test]
fn a_nested_row_is_addressed_by_its_own_archive_base() {
    let child = built(vec![("inner/a.txt", Rpf9NewBody::Chunked(body()))]);
    let file = built(vec![("child.rpf", Rpf9NewBody::Archive(child))]);
    let (_dir, src, out) = on_disk(&file);
    let root = Gta6Mount::open_root(&mut Cursor::new(&file[..]), &keys()).expect("opens");
    let base = root
        .absolute_start(&root.archive.entries[0])
        .expect("a start");
    let refused = plan_patch(
        &mut Cursor::new(&file[..]),
        &keys(),
        &BodyPatch {
            base: 0,
            entry: 0,
            body: Vec::new(),
        },
    );
    assert!(matches!(
        refused,
        Err(PatchRefusal::NestedArchive { entry: 0 })
    ));

    let mut new = body();
    new[0x20000..0x20000 + 4000].fill(b'a');
    let patch = BodyPatch {
        base,
        entry: 0,
        body: new.clone(),
    };
    let report = write_patched_copy(&src, &out, &keys(), &[patch]).expect("patched");
    // The root's table, and the nested archive's, which the root's walk and the chain both name once.
    let tables = [
        (0, toc_end(&file, 0)),
        (base, toc_end(&file, base as usize)),
    ];
    assert!(tables.iter().all(|t| report.table_spans.contains(t)));
    assert_eq!(
        (report.table_spans.iter())
            .filter(|&&(a, _)| a == base)
            .count(),
        1,
        "{:?}",
        report.table_spans
    );
    assert_eq!(report.chunks_changed, 1);
    assert_eq!(read_back(&out, base, 0), new);
}

#[test]
fn empty_blocks_fill_every_length_but_one_to_four_and_eight() {
    let content = letters(5000, 4);
    for n in 0..64 {
        let Some(pad) = padding(n) else {
            assert!([1, 2, 3, 4, 8].contains(&n), "{n} bytes have a padding");
            continue;
        };
        assert_eq!(pad.len(), n);
        for zlib in [true, false] {
            let stream = deflate(&content, 6, zlib).expect("deflates");
            let head = if zlib { 2 } else { 0 };
            let filled = [&stream[..head], &pad[..], &stream[head..]].concat();
            let chunk = Chunk {
                inflated: content.len() as u32,
                stored: filled.len() as u32,
                zlib: true,
            };
            assert_eq!(
                chunk_content(&filled, &chunk),
                Some((zlib, content.clone())),
                "{n} bytes of padding, zlib {zlib}"
            );
        }
    }

    let (shortest, optimal, _) = deflate_to(&content, true, 0, OPTIMAL_CAP, &mut |_| true)
        .expect_err("nothing fits no bytes");
    let least = shortest.min(optimal);
    for len in [least, least + 5, least + 9, least + 100] {
        let (stream, own, _) =
            deflate_to(&content, true, len, OPTIMAL_CAP, &mut |_| true).expect("fills");
        assert_eq!(stream.len(), len);
        assert!(own <= len);
    }
    // A byte short of the first parse, a harder try may still fit, and then fills exactly.
    match deflate_to(&content, true, least - 1, OPTIMAL_CAP, &mut |_| true) {
        Ok((stream, own, optimal)) => assert!(stream.len() == least - 1 && own < least && optimal),
        Err((s, o, _)) => assert!(s == shortest && o <= optimal, "{s} {o}"),
    }
}

/// Words of a small vocabulary in a random order: text whose matches an optimal parse chooses
/// better than the tree's deflater at its best level.
fn words(n: usize, seed: u32) -> Vec<u8> {
    let vocabulary: Vec<Vec<u8>> = xorshift(64, seed)
        .map(|x| letters(3 + (x % 7) as usize, x))
        .collect();
    let mut out = Vec::with_capacity(n + 16);
    for x in xorshift(n, seed ^ 0x5A5A) {
        if out.len() >= n {
            break;
        }
        out.extend_from_slice(&vocabulary[(x >> 26) as usize]);
        out.push(b' ');
    }
    out.truncate(n);
    out
}

#[test]
fn both_encoders_streams_inflate_to_the_same_bytes() {
    let contents = [
        words(0x3000, 31),
        letters(5000, 32),
        noise(3000, 33),
        vec![0u8; 0x12000],
    ];
    for content in &contents {
        for zlib in [true, false] {
            let tree = deflate(content, 9, zlib).expect("the tree's deflater");
            let optimal = optimal_deflate(content, zlib).expect("the optimal parse");
            for stream in [&tree, &optimal] {
                let chunk = Chunk {
                    inflated: content.len() as u32,
                    stored: stream.len() as u32,
                    zlib: true,
                };
                assert_eq!(
                    chunk_content(stream, &chunk),
                    Some((zlib, content.clone())),
                    "{} bytes, zlib {zlib}, {} stored",
                    content.len(),
                    stream.len()
                );
            }
        }
    }
}

#[test]
fn a_chunk_only_the_optimal_parse_fits_is_patched_and_reads_back() {
    let new = words(0x4000, 21);
    let (shortest, ceiling, _) =
        deflate_to(&new, true, 0, OPTIMAL_CAP, &mut |_| true).expect_err("nothing fits no bytes");
    assert!(
        shortest > ceiling,
        "the tree's deflater makes {shortest} bytes and the optimal parse {ceiling}: the case needs \
         content the optimal parse deflates shorter"
    );
    // The old content deflates short enough to be led by empty blocks to the ceiling.
    let mut old = new.clone();
    old[100..2100].fill(b'a');
    let (stored, _, _) = deflate_to(&old, true, ceiling, OPTIMAL_CAP, &mut |_| true)
        .expect("the old content fills the ceiling");
    let mut held = stored;
    pad16(&mut held);
    let mut archive = tabled(vec![Chunk {
        inflated: new.len() as u32,
        stored: ceiling as u32,
        zlib: true,
    }]);
    let entry = row(0, held.len(), new.len(), None);
    archive.entries = vec![entry.clone()];
    let mount = Gta6Mount { base: 0, archive };
    let file = [vec![0u8; 0x40], held].concat();
    let read = |bytes: &[u8]| mount.read_entry(Cursor::new(bytes), &entry).expect("reads");
    assert_eq!(read(&file), old);

    let patch = BodyPatch {
        base: 0,
        entry: 0,
        body: new.clone(),
    };
    let plan = plan_in(
        &mount,
        &mut Cursor::new(&file[..]),
        &keys(),
        &patch,
        OPTIMAL_CAP,
        &mut |_, _| true,
    )
    .expect("the optimal parse fits");
    let chunk = plan.chunks[0];
    assert!(chunk.changed && chunk.optimal, "{chunk:?}");
    assert_eq!(
        (chunk.new_stored, chunk.ceiling),
        (ceiling as u32, ceiling as u32)
    );
    let mut copy = file.clone();
    for (at, bytes) in &plan.writes {
        copy[*at as usize..][..bytes.len()].copy_from_slice(bytes);
    }
    assert_eq!(copy.len(), file.len());
    assert_eq!(read(&copy), new);
}

fn pad16(v: &mut Vec<u8>) {
    v.resize(v.len().next_multiple_of(16), 0);
}

/// A two-entry archive whose entry 0 has `chunks` for its table (mode 3 where its sizes fit 64 KiB,
/// else mode 4) and entry 1 none.
fn tabled(chunks: Vec<Chunk>) -> Gta6Archive {
    let small = |c: &Chunk| c.inflated <= 0x10000 && c.stored <= 0x10000;
    let mode = if chunks.iter().all(small) { 3 } else { 4 };
    let (u1, words) = author(&Table { mode, chunks }).expect("authored");
    let mut cell_and_heap = 8u32.to_le_bytes().to_vec();
    cell_and_heap.extend_from_slice(&u1.to_le_bytes());
    for word in words {
        cell_and_heap.extend_from_slice(&word.to_le_bytes());
    }
    one_entry_archive(&cell_and_heap)
}

fn row(index: u32, stored: usize, real: usize, cipher: Option<Gta6Cipher>) -> Gta6Entry {
    Gta6Entry {
        index,
        name64: 0,
        name32: None,
        name: None,
        start: Some(0x40),
        stored: Some(stored as u64),
        real: Some(real as u64),
        class: Some(0x0B),
        cipher,
    }
}

#[test]
fn a_ciphered_row_is_re_enciphered_where_its_framing_covers_it() {
    let keys = keys();
    let (a, b, c) = (letters(0x2000, 11), noise(0x3000, 12), letters(0x1800, 13));
    let (za, zc) = (deflate(&a, 9, true).unwrap(), deflate(&c, 9, true).unwrap());
    let mut plain = za.clone();
    pad16(&mut plain);
    let b_off = plain.len();
    plain.extend_from_slice(&b);
    plain.extend_from_slice(&zc);
    pad16(&mut plain);
    let chunk = |inflated: usize, stored: usize, zlib| Chunk {
        inflated: inflated as u32,
        stored: stored as u32,
        zlib,
    };
    let mut archive = tabled(vec![
        chunk(a.len(), za.len(), true),
        chunk(b.len(), b.len(), false),
        chunk(c.len(), zc.len(), true),
    ]);

    // Head and first window enciphered to 0x800, clear to 0x2400, enciphered to 0x2800, …
    let framing = [0x400, 0x400, 0x1C00, 0, 4];
    let iv = [0x11u8; 16];
    let mut enciphered = plain.clone();
    decrypt_extent(&mut enciphered, &framing, &iv, |slice, ctr| {
        keys.content_decrypt(CONTENT_KEY_ID, ctr, slice).unwrap()
    });
    let file = [vec![0u8; 0x40], enciphered.clone()].concat();
    let cipher = Gta6Cipher {
        key_id: CONTENT_KEY_ID,
        cipher_type: 17,
        iv,
        framing,
    };
    let entry = row(
        0,
        enciphered.len(),
        a.len() + b.len() + c.len(),
        Some(cipher),
    );
    archive.entries = vec![entry.clone()];
    let mount = Gta6Mount { base: 0, archive };
    let old = [a.clone(), b.clone(), c.clone()].concat();
    let read = |bytes: &[u8]| {
        mount
            .read_entry_decrypted(Cursor::new(bytes), &entry, &keys)
            .unwrap()
    };
    assert_eq!(read(&file), old);

    let mut new = old.clone();
    new[a.len()..a.len() + b.len()].copy_from_slice(&noise(0x3000, 14));
    let patch = BodyPatch {
        base: 0,
        entry: 0,
        body: new.clone(),
    };
    let plan = plan_in(
        &mount,
        &mut Cursor::new(&file[..]),
        &keys,
        &patch,
        OPTIMAL_CAP,
        &mut |_, _| true,
    )
    .expect("planned");
    assert!(plan.ciphered);
    let changed: Vec<bool> = plan.chunks.iter().map(|c| c.changed).collect();
    assert_eq!(changed, [false, true, false]);
    let mut copy = file.clone();
    for (at, bytes) in &plan.writes {
        copy[*at as usize..][..bytes.len()].copy_from_slice(bytes);
    }
    assert_eq!(read(&copy), new);

    // The stored chunk crosses a clear window, which holds the new plaintext, and an enciphered one.
    assert!(b_off <= 0x2000 && b_off + b.len() >= 0x2500);
    let body_at = |extent: usize| a.len() + extent - b_off;
    assert_eq!(
        copy[0x40 + 0x2000..0x40 + 0x2100],
        new[body_at(0x2000)..body_at(0x2100)]
    );
    assert_ne!(
        copy[0x40 + 0x2400..0x40 + 0x2500],
        new[body_at(0x2400)..body_at(0x2500)]
    );
    // The deflated chunks are the source's bytes.
    assert!(copy[..0x40 + b_off] == file[..0x40 + b_off]);
    assert!(copy[0x40 + b_off + b.len()..] == file[0x40 + b_off + b.len()..]);
}

#[test]
fn a_row_with_no_chunk_table_is_one_stream_or_one_stored_body() {
    let keys = keys();
    let plan_over = |stored: &[u8], real: usize, body: Vec<u8>| {
        let mut held = stored.to_vec();
        pad16(&mut held);
        let mut archive = tabled(vec![Chunk {
            inflated: 16,
            stored: 16,
            zlib: false,
        }]);
        let entry = row(1, held.len(), real, None);
        archive.entries = vec![row(0, 16, 16, None), entry];
        let mount = Gta6Mount { base: 0, archive };
        let file = [vec![0u8; 0x40], held].concat();
        let patch = BodyPatch {
            base: 0,
            entry: 1,
            body,
        };
        let plan = plan_in(
            &mount,
            &mut Cursor::new(&file[..]),
            &keys,
            &patch,
            OPTIMAL_CAP,
            &mut |_, _| true,
        );
        (file, plan)
    };

    // A head that is no stream (0xFF opens neither zlib nor DEFLATE) is a stored body.
    let raw = [vec![0xFF; 16], letters(1000, 5)].concat();
    let other = [vec![0xFF; 16], letters(1000, 6)].concat();
    let (file, plan) = plan_over(&raw, raw.len(), other.clone());
    let plan = plan.expect("a stored body");
    assert_eq!(plan.chunks.len(), 1);
    assert!(!plan.chunks[0].zlib && plan.chunks[0].changed);
    let (at, bytes) = &plan.writes[0];
    assert_eq!((*at, bytes), (0x40, &other));
    assert_eq!(file.len(), 0x40 + raw.len().next_multiple_of(16));

    // One raw DEFLATE stream is one deflated chunk, its stream's length the ceiling.
    let text = letters(5000, 7);
    let stream = deflate(&text, 9, false).unwrap();
    let mut new = text.clone();
    new[1000..2000].fill(b'a');
    let (_, plan) = plan_over(&stream, text.len(), new.clone());
    let plan = plan.expect("one stream");
    assert_eq!(plan.chunks[0].ceiling as usize, stream.len());
    let (_, bytes) = &plan.writes[0];
    let chunk = Chunk {
        inflated: text.len() as u32,
        stored: bytes.len() as u32,
        zlib: true,
    };
    assert_eq!(chunk_content(bytes, &chunk), Some((false, new)));

    // Two streams with no table to state where the first ends are not covered.
    let (first, second) = (letters(3000, 8), letters(2000, 9));
    let mut two = deflate(&first, 9, true).unwrap();
    pad16(&mut two);
    two.extend_from_slice(&deflate(&second, 9, true).unwrap());
    let (_, plan) = plan_over(&two, 5000, [first, second].concat());
    assert!(matches!(plan, Err(PatchRefusal::NoChunkTable { entry: 1 })));
}

/// A one-entry archive whose row holds `chunks`' stored bytes, each laid at a 16-aligned offset.
fn laid(chunks: &[(Vec<u8>, Chunk)]) -> (Gta6Mount, Gta6Entry, Vec<u8>) {
    let mut held = Vec::new();
    for (bytes, _) in chunks {
        held.extend_from_slice(bytes);
        pad16(&mut held);
    }
    let mut archive = tabled(chunks.iter().map(|(_, c)| *c).collect());
    let real = chunks.iter().map(|(_, c)| c.inflated as usize).sum();
    let entry = row(0, held.len(), real, None);
    archive.entries = vec![entry.clone()];
    let file = [vec![0u8; 0x40], held].concat();
    (Gta6Mount { base: 0, archive }, entry, file)
}

#[test]
fn a_chunk_with_filler_after_its_stream_plans_unchanged_and_beside_an_edit_of_another() {
    let (a, b) = (letters(0x2000, 41), letters(0x1800, 42));
    let mut za = deflate(&a, 9, true).unwrap();
    let stored_a = za.len() + 4;
    za.extend_from_slice(&[0xA5; 4]);
    let zb = deflate(&b, 9, true).unwrap();
    let chunk = |inflated: usize, stored: usize| Chunk {
        inflated: inflated as u32,
        stored: stored as u32,
        zlib: true,
    };
    let (mount, entry, file) = laid(&[
        (za, chunk(a.len(), stored_a)),
        (zb.clone(), chunk(b.len(), zb.len())),
    ]);
    let old = [a.clone(), b.clone()].concat();
    let plan = |body: Vec<u8>| {
        let patch = BodyPatch {
            base: 0,
            entry: 0,
            body,
        };
        plan_in(
            &mount,
            &mut Cursor::new(&file[..]),
            &keys(),
            &patch,
            OPTIMAL_CAP,
            &mut |_, _| true,
        )
    };
    let unchanged = plan(old.clone()).expect("an unchanged body plans");
    assert!(unchanged.writes.is_empty());
    let mut new = old.clone();
    new[a.len() + 10..a.len() + 1000].fill(b'a');
    let edited = plan(new.clone()).expect("an edit of the other chunk plans");
    let changed: Vec<bool> = edited.chunks.iter().map(|c| c.changed).collect();
    assert_eq!(changed, [false, true]);
    let mut copy = file.clone();
    for (at, bytes) in &edited.writes {
        copy[*at as usize..][..bytes.len()].copy_from_slice(bytes);
    }
    assert_eq!(mount.read_entry(Cursor::new(&copy), &entry).unwrap(), new);
}

#[test]
fn a_stream_three_bytes_short_of_its_ceiling_is_followed_by_zeros_and_patches_again() {
    let new = letters(0x3000, 51);
    let shortest = (0..=9)
        .filter_map(|level| deflate(&new, level, true))
        .map(|s| s.len())
        .min()
        .unwrap();
    let ceiling = shortest + 3;
    let mut old = new.clone();
    old[100..3000].fill(b'a');
    let (stored, _, _) = deflate_to(&old, true, ceiling, OPTIMAL_CAP, &mut |_| true)
        .expect("the old content fills it");
    let chunk = Chunk {
        inflated: new.len() as u32,
        stored: ceiling as u32,
        zlib: true,
    };
    let (mount, entry, file) = laid(&[(stored, chunk)]);
    let patch = |body: &[u8]| BodyPatch {
        base: 0,
        entry: 0,
        body: body.to_vec(),
    };
    let plan = plan_in(
        &mount,
        &mut Cursor::new(&file[..]),
        &keys(),
        &patch(&new),
        OPTIMAL_CAP,
        &mut |_, _| true,
    )
    .expect("a stream 3 bytes short fits");
    let c = plan.chunks[0];
    assert!(c.changed && !c.optimal, "{c:?}");
    assert_eq!(
        (c.new_stored as usize, c.ceiling as usize),
        (shortest, ceiling)
    );
    let (at, bytes) = &plan.writes[0];
    assert_eq!(bytes[shortest..], [0, 0, 0]);
    let mut copy = file.clone();
    copy[*at as usize..][..bytes.len()].copy_from_slice(bytes);
    assert_eq!(mount.read_entry(Cursor::new(&copy), &entry).unwrap(), new);
    // The patched copy plans again: its zeros after the stream are read past.
    let again = plan_in(
        &mount,
        &mut Cursor::new(&copy[..]),
        &keys(),
        &patch(&old),
        OPTIMAL_CAP,
        &mut |_, _| true,
    )
    .expect("a patched copy patches again");
    assert!(again.chunks[0].changed);
}

#[test]
fn a_chunk_past_the_optimal_cap_is_refused_without_the_optimal_parse() {
    let content = noise(OPTIMAL_CAP + 1, 61);
    let started = std::time::Instant::now();
    let refused = deflate_to(&content, true, 1000, OPTIMAL_CAP, &mut |_| true)
        .expect_err("noise fits no 1000 bytes");
    assert_eq!(refused.1, usize::MAX, "the optimal parse is not tried");
    assert!(refused.0 > content.len());
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    let said = PatchRefusal::ChunkDoesNotFit {
        entry: 0,
        chunk: 0,
        deflated: refused.0,
        optimal: refused.1,
        ceiling: 1000,
        cap: OPTIMAL_CAP,
        harder: None,
    }
    .to_string();
    assert!(said.contains("not tried on a chunk past 8 MiB"), "{said}");
}

/// The words an [`OPTIMAL_PART`] of [`sparse_words`] opens with.
const SPARSE: usize = OPTIMAL_PART / 8;

/// Each part a run of one text's words then zeros: a chunk of many parts whose optimal parse is quick.
fn sparse_words(n: usize, seed: u32) -> Vec<u8> {
    let text = words(n / 8 + SPARSE, seed);
    let mut out = vec![0u8; n];
    for (i, part) in out.chunks_mut(OPTIMAL_PART).enumerate() {
        let k = part.len().min(SPARSE);
        part[..k].copy_from_slice(&text[i * SPARSE..][..k]);
    }
    out
}

/// A one-chunk row whose chunk the optimal parse alone fits, and its old content filling that ceiling.
fn only_optimal_fits(new: &[u8]) -> (Gta6Mount, Gta6Entry, Vec<u8>) {
    let (shortest, ceiling, _) =
        deflate_to(new, true, 0, OPTIMAL_CAP, &mut |_| true).expect_err("nothing fits no bytes");
    assert!(
        shortest > ceiling,
        "the tree's deflater makes {shortest} bytes and the optimal parse {ceiling}: the case needs \
         content the optimal parse deflates shorter"
    );
    let mut old = new.to_vec();
    for part in old.chunks_mut(OPTIMAL_PART) {
        let k = part.len().min(SPARSE);
        part[..k].fill(b'a');
    }
    let (stored, _, _) = deflate_to(&old, true, ceiling, OPTIMAL_CAP, &mut |_| true)
        .expect("the old content fills the ceiling");
    let chunk = Chunk {
        inflated: new.len() as u32,
        stored: ceiling as u32,
        zlib: true,
    };
    let (mount, entry, file) = laid(&[(stored, chunk)]);
    assert_eq!(mount.read_entry(Cursor::new(&file), &entry).unwrap(), old);
    (mount, entry, file)
}

#[test]
fn a_one_mebibyte_chunk_past_the_old_cap_is_patched_by_the_optimal_parse() {
    let new = sparse_words(1 << 20, 71);
    let (mount, entry, file) = only_optimal_fits(&new);
    let patch = BodyPatch {
        base: 0,
        entry: 0,
        body: new.clone(),
    };
    let mut ticks = Vec::new();
    let plan = plan_in(
        &mount,
        &mut Cursor::new(&file[..]),
        &keys(),
        &patch,
        OPTIMAL_CAP,
        &mut |d, t| {
            ticks.push((d, t));
            true
        },
    )
    .expect("the optimal parse fits a 1 MiB chunk");
    let chunk = plan.chunks[0];
    assert!(chunk.changed && chunk.optimal, "{chunk:?}");
    assert_eq!(chunk.new_stored, chunk.ceiling);
    // The count climbs a part at a time to the body's length, which every tick states.
    let total = new.len() as u64;
    assert!(ticks.iter().all(|&(_, t)| t == total), "{ticks:?}");
    assert!(ticks.windows(2).all(|w| w[0].0 <= w[1].0), "{ticks:?}");
    assert_eq!(ticks.last().map(|t| t.0), Some(total));
    let parts = (ticks.iter()).filter(|&&(d, _)| d > 0 && d < total).count();
    assert!(parts >= (1 << 20) / OPTIMAL_PART - 1, "{ticks:?}");
    let mut copy = file.clone();
    for (at, bytes) in &plan.writes {
        copy[*at as usize..][..bytes.len()].copy_from_slice(bytes);
    }
    assert_eq!(mount.read_entry(Cursor::new(&copy), &entry).unwrap(), new);
}

#[test]
fn a_stop_ends_the_plan_inside_the_optimal_parse_and_writes_nothing() {
    let new = sparse_words(1 << 20, 72);
    let (mount, _, file) = only_optimal_fits(&new);
    let patch = BodyPatch {
        base: 0,
        entry: 0,
        body: new.clone(),
    };
    let total = new.len() as u64;
    let (mut asked_after, mut stopped_at) = (0, None);
    let planned = plan_in(
        &mount,
        &mut Cursor::new(&file[..]),
        &keys(),
        &patch,
        OPTIMAL_CAP,
        &mut |d, _| {
            if stopped_at.is_some() {
                asked_after += 1;
            }
            // Stopped after the optimal parse's first part, well short of the chunk's end.
            if d > 0 && d < total && stopped_at.is_none() {
                stopped_at = Some(d);
                return false;
            }
            stopped_at.is_none()
        },
    );
    assert!(
        matches!(planned, Err(PatchRefusal::Stopped { entry: 0 })),
        "{planned:?}"
    );
    assert_eq!(stopped_at, Some(OPTIMAL_PART as u64));
    assert_eq!(asked_after, 0, "a stopped plan is not asked again");

    // A built archive on disk, through the public planner: a stopped plan leaves nothing to write.
    let archive = built(vec![("docs/a.txt", Rpf9NewBody::Chunked(body()))]);
    let (_dir, src, out) = on_disk(&archive);
    let mut edited = body();
    edited[0x20000 + 100..0x20000 + 4100].fill(b'a');
    let patch = BodyPatch {
        base: 0,
        entry: 0,
        body: edited,
    };
    let mut source = File::open(&src).unwrap();
    let stopped = plan_patch_with(&mut source, &keys(), &patch, OPTIMAL_CAP, &mut |_, _| false);
    assert!(matches!(stopped, Err(PatchRefusal::Stopped { entry: 0 })));
    assert!(
        (stopped.unwrap_err().to_string()).contains("nothing was written"),
        "the refusal says so"
    );
    assert!(!out.exists());
    assert_eq!(
        std::fs::read(&src).unwrap(),
        archive,
        "the source is untouched"
    );
}

#[test]
fn parts_of_the_optimal_parse_join_as_one_stream_that_inflates_to_the_content() {
    // Three parts, the third short, the text repeated closer than the window across each seam.
    let text = words(30_000, 81);
    let content = [&text[..], &text, &text, &text, &text, &letters(1000, 82)].concat();
    assert!(content.len() > 2 * OPTIMAL_PART && content.len() < 3 * OPTIMAL_PART);
    for zlib in [true, false] {
        let mut asked = Vec::new();
        let stream = optimal_parse(&content, zlib, &mut |n| {
            asked.push(n);
            true
        })
        .expect("the optimal parse");
        assert_eq!(asked, [0, OPTIMAL_PART, 2 * OPTIMAL_PART]);
        let chunk = Chunk {
            inflated: content.len() as u32,
            stored: stream.len() as u32,
            zlib: true,
        };
        assert_eq!(
            chunk_content(&stream, &chunk),
            Some((zlib, content.clone()))
        );
        // The window crosses each seam: the repeats cost far less than the text, as parts parsed
        // apart would not.
        let first = optimal_deflate(&text, zlib).unwrap();
        assert!(
            stream.len() < 2 * first.len(),
            "{} against {}",
            stream.len(),
            first.len()
        );
    }
}

/// Bytes like a vertex stream: little-endian words that walk by small steps, with noisy low bits.
fn vertices(n: usize, seed: u32) -> Vec<u8> {
    let mut x = 0x4000_0000u32;
    let mut out = Vec::with_capacity(n + 4);
    for r in xorshift(n / 4 + 1, seed) {
        x = x.wrapping_add((r >> 20) & 0x3FF).wrapping_sub(0x1FF);
        out.extend_from_slice(&((x & !0xFF) | (r & 0x3F)).to_le_bytes());
    }
    out.truncate(n);
    out
}

/// A content maker: its length and its seed.
type Content = fn(usize, u32) -> Vec<u8>;

/// The optimal parse's time on 1, 8 and 32 MiB chunks, what a part boundary costs against one
/// whole parse at 256 KiB, and the longest a stop waits.
#[test]
#[ignore = "a measurement of many minutes, run once and read"]
fn the_optimal_parse_on_chunks_of_1_8_and_32_mib() {
    use std::time::Instant;
    let kinds: [(&str, Content); 3] = [
        ("words", words),
        ("letters", letters),
        ("vertices", vertices),
    ];
    for (kind, make) in kinds {
        let content = make(256 << 10, 91);
        let parted = optimal_deflate(&content, true).unwrap();
        let mut whole = Vec::new();
        let options = optimal_options().unwrap();
        zopfli::compress(options, zopfli::Format::Zlib, &content[..], &mut whole).unwrap();
        let level9 = deflate(&content, 9, true).unwrap().len();
        eprintln!(
            "{kind}, 256 KiB: {} bytes in 64 KiB parts, {} in one parse, {level9} at zlib level 9",
            parted.len(),
            whole.len()
        );
    }
    for (kind, make) in kinds {
        for mib in [1usize, 8, 32] {
            if mib == 32 && kind != "words" {
                continue;
            }
            let content = make(mib << 20, 92);
            let began = Instant::now();
            // A stop is seen when the parse next asks, so the longest gap is how long one waits.
            let (mut last, mut gap) = (began, std::time::Duration::ZERO);
            let stream = optimal_parse(&content, true, &mut |_| {
                gap = gap.max(last.elapsed());
                last = Instant::now();
                true
            })
            .unwrap();
            let took = began.elapsed();
            eprintln!(
                "{kind}, {mib} MiB: {:.1} s, {:.2} us a byte, a stop waits at most {:.2} s, {} \
                 bytes ({:.1} %), zlib level 9 {}",
                took.as_secs_f64(),
                took.as_secs_f64() * 1e6 / content.len() as f64,
                gap.max(last.elapsed()).as_secs_f64(),
                stream.len(),
                100.0 * stream.len() as f64 / content.len() as f64,
                deflate(&content, 9, true).unwrap().len()
            );
        }
    }
}

#[test]
fn a_file_cut_short_inside_a_row_protects_no_names_blob_over_that_row() {
    // The archive is given a 64-byte names blob after its last row.
    let mut file = built(vec![("docs/a.txt", Rpf9NewBody::Chunked(body()))]);
    file[0x0C..0x10].copy_from_slice(&0x40u32.to_le_bytes());
    file.extend_from_slice(&[0u8; 0x40]);
    let mount = Gta6Mount::open_root(&mut Cursor::new(&file[..]), &keys()).expect("opens");
    let e = (mount.archive.entries.iter())
        .find(|e| e.real == Some(body().len() as u64))
        .expect("the row");
    let (start, stored) = (mount.absolute_start(e).unwrap(), e.stored.unwrap());
    let whole = protected_spans(&mut Cursor::new(&file[..]), &keys(), 0).expect("spans");
    assert!(whole.iter().any(|&(a, _)| a >= start + stored), "{whole:?}");
    let cut = &file[..(start + stored - 8) as usize];
    let spans = protected_spans(&mut Cursor::new(cut), &keys(), 0).expect("spans");
    assert!(
        spans
            .iter()
            .all(|&(a, b)| b <= start || a >= start + stored),
        "{spans:?} over the row at {start:#x}..{:#x}",
        start + stored
    );
}

/// `new`'s first optimal stream's length, and the stream of the first harder try shorter than it.
fn first_and_harder(new: &[u8]) -> (usize, Vec<u8>) {
    let first = optimal_deflate(new, true).expect("the first parse").len();
    // The plan's own bound, so the try found here is one the plan also reaches.
    let harder =
        harder_deflate(new, true, first - 1, HARDER_BOUND, &mut || true).unwrap_or_else(|miss| {
            panic!(
            "no harder try beat the first parse's {first} bytes ({miss:?}): the case needs content \
             one deflates shorter"
        )
        });
    (first, harder)
}

/// A one-chunk row of `new`'s length stored in `ceiling` bytes, its old content filling them.
fn filled_to(new: &[u8], ceiling: usize) -> (Gta6Mount, Gta6Entry, Vec<u8>) {
    let mut old = new.to_vec();
    old[100..4100].fill(b'a');
    let (stored, _, _) = deflate_to(&old, true, ceiling, OPTIMAL_CAP, &mut |_| true)
        .expect("the old content fills the ceiling");
    let chunk = Chunk {
        inflated: new.len() as u32,
        stored: ceiling as u32,
        zlib: true,
    };
    let (mount, entry, file) = laid(&[(stored, chunk)]);
    assert_eq!(mount.read_entry(Cursor::new(&file), &entry).unwrap(), old);
    (mount, entry, file)
}

#[test]
fn a_chunk_the_first_parse_misses_by_a_few_bytes_is_fitted_by_a_harder_try_and_reads_back() {
    // Two parts of one text: a harder try parses them under fewer blocks or for longer.
    let new = words(0x1C000, 101);
    let (first, harder) = first_and_harder(&new);
    let ceiling = harder.len();
    assert!(
        first - ceiling <= harder_margin(ceiling),
        "the first parse's {first} bytes are past the margin over {ceiling}"
    );
    let (mount, entry, file) = filled_to(&new, ceiling);
    let patch = BodyPatch {
        base: 0,
        entry: 0,
        body: new.clone(),
    };
    let plan = plan_in(
        &mount,
        &mut Cursor::new(&file[..]),
        &keys(),
        &patch,
        OPTIMAL_CAP,
        &mut |_, _| true,
    )
    .expect("a harder try fits the chunk");
    let chunk = plan.chunks[0];
    assert!(chunk.changed && chunk.optimal, "{chunk:?}");
    assert_eq!(
        (chunk.new_stored as usize, chunk.ceiling as usize),
        (ceiling, ceiling)
    );
    assert!(ceiling < first);
    let mut copy = file.clone();
    for (at, bytes) in &plan.writes {
        copy[*at as usize..][..bytes.len()].copy_from_slice(bytes);
    }
    assert_eq!(copy.len(), file.len());
    assert_eq!(mount.read_entry(Cursor::new(&copy), &entry).unwrap(), new);

    // A stop asked while the harder tries run ends the plan, and it is not asked again.
    let total = new.len() as u64;
    let (mut stopped, mut asked_after) = (false, 0);
    let planned = plan_in(
        &mount,
        &mut Cursor::new(&file[..]),
        &keys(),
        &patch,
        OPTIMAL_CAP,
        &mut |done, _| {
            if stopped {
                asked_after += 1;
            }
            // The first parse asks at its parts' starts; only the harder tries ask at the chunk's end.
            stopped = stopped || done == total;
            !stopped
        },
    );
    assert!(
        matches!(planned, Err(PatchRefusal::Stopped { entry: 0 })),
        "{planned:?}"
    );
    assert_eq!(asked_after, 0, "a stopped plan is not asked again");
}

#[test]
fn the_harder_tries_end_within_their_bound_and_at_a_stop() {
    let content = words(0x4000, 102);
    let began = Instant::now();
    optimal_deflate(&content, true).expect("the first parse");
    let once = began.elapsed();
    // No stream is 10 bytes long, so only the bound ends the tries.
    let bound = once * 2;
    let began = Instant::now();
    let miss = harder_deflate(&content, true, 10, bound, &mut || true).expect_err("nothing fits");
    let took = began.elapsed();
    assert!(
        miss.untried.contains(&"250 rounds"),
        "the longest try is begun past what the bound leaves: {miss:?}"
    );
    assert!(
        // A scheduler stall under load is allowed for; 250 rounds alone run about 16 times `once`.
        took < bound + once * 8 + Duration::from_secs(5),
        "{took:?} against a bound of {bound:?}, one parse taking {once:?}"
    );
    assert!(miss.smallest == usize::MAX || miss.smallest > 10);

    let mut asked = 0;
    let began = Instant::now();
    let stop = Duration::from_secs(600);
    let miss = harder_deflate(&content, true, 10, stop, &mut || {
        asked += 1;
        false
    })
    .expect_err("stopped");
    assert_eq!(asked, 1, "a stop is not asked again");
    assert!(
        miss.tried.is_empty() && miss.smallest == usize::MAX,
        "{miss:?}"
    );
    assert!(miss.untried.contains(&"15 rounds with no early stop"));
    assert!(began.elapsed() < once * 8 + Duration::from_secs(5));
}

#[test]
fn a_chunk_within_the_margin_that_no_try_fits_is_refused_saying_what_was_tried() {
    // Noise is stored, at 11 bytes over its own length: within the margin, past every try.
    let content = noise(0x1000, 103);
    let len = content.len();
    let (deflated, optimal, harder) = deflate_to(&content, true, len, OPTIMAL_CAP, &mut |_| true)
        .expect_err("noise deflates to no fewer bytes than it holds");
    let harder = harder.expect("the harder tries ran within the margin");
    assert!(
        optimal > len && optimal - len <= harder_margin(len),
        "{optimal}"
    );
    assert!(harder.untried.is_empty(), "{harder:?}");
    assert!(harder.tried.contains(&"250 rounds"), "{harder:?}");
    assert!(harder.smallest >= optimal);
    let said = PatchRefusal::ChunkDoesNotFit {
        entry: 0,
        chunk: 0,
        deflated,
        optimal,
        ceiling: len as u32,
        cap: OPTIMAL_CAP,
        harder: Some(harder),
    }
    .to_string();
    assert!(
        said.contains(&format!("to {optimal} by an optimal parse, the smallest after the harder tries (15 rounds with no early stop;"))
            && said.contains("250 rounds)"),
        "{said}"
    );

    // Past the margin the harder tries are not run.
    let far = deflate_to(&content, true, len - 100, OPTIMAL_CAP, &mut |_| true)
        .expect_err("noise fits no fewer bytes");
    assert_eq!(far.2, None);
    let cut = HarderMiss {
        smallest: usize::MAX,
        tried: Vec::new(),
        untried: vec!["60 rounds", "250 rounds"],
    };
    let said = optimal_said(optimal, 10, OPTIMAL_CAP, &Some(cut));
    assert!(
        said.contains(
            "parse, with no harder try ended, 60 rounds; 250 rounds cut by the time bound"
        ),
        "{said}"
    );
}

#[test]
fn incompressible_runs_are_cut_into_parts_of_their_own() {
    // Six windows of noise, one alone, then letters past one part.
    let content = [
        letters(0x8000, 111),
        noise(0x6000, 112),
        letters(0x1000, 113),
        noise(0x1000, 114),
        letters(0x13000, 115),
    ]
    .concat();
    let cuts = stored_run_cuts(&content);
    assert_eq!(cuts, [0x8000, 0xE000, 0xE000 + OPTIMAL_PART, 0x23000]);
    let plain = letters(0x20000, 116);
    assert_eq!(
        stored_run_cuts(&plain),
        even_cuts(plain.len(), OPTIMAL_PART)
    );
    assert_eq!(even_cuts(0, OPTIMAL_PART), Vec::<usize>::new());

    let options = optimal_options().expect("options");
    for zlib in [true, false] {
        let stream = parse_with(&content, zlib, options, &cuts, &mut |_| true).expect("parsed");
        let chunk = Chunk {
            inflated: content.len() as u32,
            stored: stream.len() as u32,
            zlib: true,
        };
        assert_eq!(
            chunk_content(&stream, &chunk),
            Some((zlib, content.clone()))
        );
    }
}
