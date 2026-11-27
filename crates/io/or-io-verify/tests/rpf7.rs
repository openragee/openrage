//! RPF7 through its declarations: the header and the entry table read typed
//! against the archive reader, each written back, and an archive rebuilt from
//! what it holds.
//!
//! ⚠ **No archive is staged, and every retail archive's table is
//! NG-encrypted**, so the typed reads run over archives the builder writes,
//! with every kind of entry at every name shift. The headers of the archives
//! an install ships, which are never encrypted, are read when
//! `OPENRAGE_GAME_ROOT` names one and skipped with that reason otherwise.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::mem::{offset_of, size_of};
use std::path::PathBuf;

use or_io::container::resource::Rsc7;
use or_io::container::rpf::entry::RpfEntryTag;
use or_io::container::rpf::{
    FiPackEntry, FiPackEntryU, FiPackHeader, RpfArchive, RpfBuilder, RpfDirectoryEntry,
    RpfEncryption, RpfEntry, RpfHeader, RPF7_MAGIC,
};
use or_io::decl::{describe, root, Cx, Owned, Plain, Raw, Rx};
use or_io::desc::{BitRange, FieldRepr, Pack, Value};
use or_test_support::{game_roots, skip_install, GAME_ROOT_ENV};

const HEADER: usize = size_of::<Raw<FiPackHeader>>();
const ENTRY: usize = size_of::<Raw<FiPackEntry>>();

fn resource(system: u32, graphics: u32, body: &[u8]) -> Vec<u8> {
    let seal = Rsc7::header_from_entry_flags(system, graphics);
    let mut blob = seal.header_bytes().to_vec();
    blob.extend_from_slice(body);
    blob
}

/// What goes into each synthetic archive: every entry kind, nested and flat.
fn contents(seed: usize) -> Vec<(String, Vec<u8>, bool)> {
    let mut out = vec![
        ("loose.bin".to_string(), b"loose".repeat(seed + 1), true),
        ("alpha/first.txt".to_string(), b"first".to_vec(), true),
        (
            "alpha/stored.bik".to_string(),
            b"BIK!".repeat(64 * (seed + 1)),
            false,
        ),
        (
            "alpha/sub/leaf.ydr".to_string(),
            resource(0xA000_4000, 0x5000_0000, &b"FAKE".repeat(300)),
            true,
        ),
        ("zeta/last.txt".to_string(), b"last ".repeat(1024), true),
    ];
    for i in 0..(seed * 40) {
        out.push((
            format!("many/d{}/file{i:04}.dat", i % 7),
            vec![i as u8; i % 97 + 1],
            i % 3 != 0,
        ));
    }
    out
}

fn build(files: &[(String, Vec<u8>, bool)], shift: u8) -> Vec<u8> {
    let mut builder = RpfBuilder::default().name_shift(shift);
    for (path, bytes, compressed) in files {
        builder
            .insert_with(path, bytes.clone(), *compressed)
            .unwrap();
    }
    builder.to_bytes().unwrap()
}

/// Every synthetic archive, by name: three sizes at each of the four name shifts.
fn archives() -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for shift in 0..=3u8 {
        for seed in [0, 1, 25] {
            out.push((
                format!("shift{shift}-seed{seed}"),
                build(&contents(seed), shift),
            ));
        }
    }
    out
}

fn words(bytes: &[u8]) -> [u32; 4] {
    std::array::from_fn(|i| u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap()))
}

fn pack() -> Pack {
    describe(
        "gta5",
        "gen8",
        &[root::<Raw<FiPackHeader>>, root::<Raw<FiPackEntry>>],
    )
    .unwrap()
}

/// The owned value of the record `bytes` starts with, and its raw layout.
fn owned<T: Owned>(bytes: &[u8]) -> (T::Raw, T) {
    let raw = T::Raw::read_from(bytes).expect("a whole record");
    let tree = Value::Bytes(Vec::new());
    let value = T::from_raw(&raw, &[], 0, &Rx::new(&tree)).unwrap();
    (raw, value)
}

fn saved<T: Owned>(value: &T, cx: &Cx) -> Vec<u8> {
    Plain::as_bytes(&value.to_raw(&mut Vec::new(), 0, cx).unwrap()).to_vec()
}

#[test]
fn every_rpf7_header_reads_typed_as_the_archive_reads() {
    let cx = Cx::new(pack().plans().clone());
    let mut n = 0;
    for (name, bytes) in archives() {
        let (raw, header) = owned::<FiPackHeader>(&bytes);
        assert_eq!(Plain::as_bytes(&raw), &bytes[..HEADER], "{name}");
        let [magic, entry_count, name_heap_size, encrypted] = words(&bytes);
        let archive = RpfArchive::from_bytes(bytes.clone()).unwrap();
        let loaded = archive.header;
        let decoded = RpfHeader::read(&mut Cursor::new(&bytes)).unwrap();
        assert_eq!(decoded, loaded, "{name}");
        assert_eq!(
            (header.magic, header.encrypted),
            (magic, encrypted),
            "{name}"
        );
        assert_eq!(header.magic, RPF7_MAGIC, "{name}");
        assert_eq!(
            RpfEncryption::from_raw(header.encrypted),
            loaded.encryption,
            "{name}"
        );
        assert_eq!(header.name_shift(), loaded.name_shift, "{name}");
        assert_eq!(header.is_x_compressed(), loaded.xcompress, "{name}");
        assert_eq!(loaded.entry_count, entry_count, "{name}");
        assert_eq!(archive.entries.len(), entry_count as usize, "{name}");
        assert_eq!(
            loaded.name_heap_size,
            name_heap_size & 0x0FFF_FFFF,
            "{name}"
        );
        let mut back = Vec::new();
        loaded.write(&mut back).unwrap();
        assert_eq!(back, bytes[..HEADER], "{name}: the header written back");
        assert_eq!(
            saved(&header, &cx),
            bytes[..HEADER],
            "{name}: the owned header saved"
        );
        n += 1;
    }
    let fresh = FiPackHeader::default();
    assert_eq!((fresh.name_shift(), fresh.is_x_compressed()), (0, false));
    println!(
        "{n} archives: every header reads typed as its words and the archive, and writes back"
    );
}

fn kind(e: &RpfEntry) -> &'static str {
    match e {
        RpfEntry::Directory(_) => "directory",
        RpfEntry::Binary(_) => "binary",
        RpfEntry::Resource(_) => "resource",
    }
}

#[test]
fn every_rpf7_entry_reads_typed_as_the_archive_reads() {
    let cx = Cx::new(pack().plans().clone());
    let (mut entries, mut kinds) = (0usize, BTreeMap::<&str, usize>::new());
    for (name, bytes) in archives() {
        let archive = RpfArchive::from_bytes(bytes.clone()).unwrap();
        for (i, entry) in archive.entries.iter().enumerate() {
            let at = HEADER + i * ENTRY;
            let block = &bytes[at..at + ENTRY];
            let (raw, typed) = owned::<FiPackEntry>(block);
            assert_eq!(Plain::as_bytes(&raw), block, "{name} entry {i}");
            let [low, high] = [words(block)[2], words(block)[3]];
            match (&typed.u, entry) {
                (FiPackEntryU::Directory(d), RpfEntry::Directory(e)) => {
                    assert_eq!(d.directory_index, e.first_entry_index, "{name} entry {i}");
                    assert_eq!((low, high), (e.first_entry_index, e.entry_count));
                }
                (FiPackEntryU::File(f), RpfEntry::Binary(e)) => {
                    assert_eq!(
                        (f.uncompressed_size, f.encrypted),
                        (e.uncompressed_size, e.encrypted_raw),
                        "{name} entry {i}"
                    );
                    assert_eq!((low, high), (f.uncompressed_size, f.encrypted));
                }
                (FiPackEntryU::Resource(r), RpfEntry::Resource(e)) => {
                    assert_eq!(
                        (r.info.r#virtual, r.info.physical),
                        (e.system_info, e.graphics_info),
                        "{name} entry {i}"
                    );
                    assert_eq!((low, high), (e.system_info, e.graphics_info));
                }
                (u, e) => panic!("{name} entry {i}: typed as {u:?}, read as {e:?}"),
            }
            let decoded = RpfEntry::read(&mut Cursor::new(block)).unwrap();
            assert_eq!(&decoded, entry, "{name} entry {i}");
            let mut back = Vec::new();
            entry.write(&mut back).unwrap();
            assert_eq!(back, block, "{name} entry {i}: written back");
            assert_eq!(
                saved(&typed, &cx),
                block,
                "{name} entry {i}: the owned entry saved"
            );
            *kinds.entry(kind(entry)).or_default() += 1;
            entries += 1;
        }
    }
    println!("{entries} entries read typed as the archive reads them and write back: {kinds:?}");
    assert_eq!(kinds.len(), 3, "every kind of entry is exercised");
}

fn holdings(archive: &RpfArchive) -> Vec<(String, Vec<u8>, bool)> {
    let mut out = Vec::new();
    for (i, entry) in archive.entries.iter().enumerate() {
        let i = i as u32;
        let path = archive.entry_path(i).unwrap();
        match entry {
            RpfEntry::Directory(_) => {}
            RpfEntry::Binary(b) => {
                out.push((path, archive.read_entry(i).unwrap(), b.is_compressed()))
            }
            RpfEntry::Resource(_) => out.push((path, archive.read_entry(i).unwrap(), true)),
        }
    }
    out
}

/// The property is a fixed point from the first rebuild, as for every declared format.
#[test]
fn an_archive_rebuilt_from_what_it_holds_is_a_fixed_point() {
    let (mut n, mut moved) = (0, 0);
    for (name, bytes) in archives() {
        let first = RpfArchive::from_bytes(bytes.clone()).unwrap();
        let shift = first.header.name_shift;
        let held = holdings(&first);
        let once = build(&held, shift);
        let rebuilt = RpfArchive::from_bytes(once.clone()).unwrap();
        let mut sorted = held.clone();
        sorted.sort();
        let mut again_held = holdings(&rebuilt);
        again_held.sort();
        assert_eq!(
            again_held, sorted,
            "{name}: the rebuild holds what the archive held"
        );
        let twice = build(&holdings(&rebuilt), shift);
        assert_eq!(
            twice, once,
            "{name}: a second rebuild is byte for byte the first"
        );
        moved += usize::from(once != bytes);
        n += 1;
    }
    println!(
        "{n} of {n} archives hold the same files after a rebuild and reach a fixed point; {moved} \
         of them changed bytes on the first"
    );
}

/// Where `marker` lands in `bytes`, found exactly once.
fn landed(bytes: &[u8], marker: &[u8]) -> usize {
    let hits: Vec<usize> = (0..=bytes.len() - marker.len())
        .filter(|&at| &bytes[at..at + marker.len()] == marker)
        .collect();
    assert_eq!(hits.len(), 1, "{marker:02X?} in {bytes:02X?}");
    hits[0]
}

/// Each described range set alone, then every bit of the `width`-bit word outside them, decoded.
fn decodes_as_described(
    what: &str,
    width: u32,
    bits: &[BitRange],
    decode: impl Fn(u64) -> BTreeMap<&'static str, u64>,
) {
    let mut names: Vec<&str> = bits.iter().map(|b| b.name.as_str()).collect();
    names.sort_unstable();
    let read: Vec<&str> = decode(0).into_keys().collect();
    assert_eq!(names, read, "{what}: the ranges the reader decodes");
    for b in bits {
        let got = decode(b.mask());
        for name in &names {
            let want = if *name == b.name {
                b.mask() >> b.low
            } else {
                0
            };
            assert_eq!(got[*name], want, "{what}: {name} with only {} set", b.name);
        }
    }
    let outside = !bits.iter().fold(0u64, |m, b| m | b.mask()) & (u64::MAX >> (64 - width));
    let got = decode(outside);
    assert!(
        got.values().all(|&v| v == 0),
        "{what}: a bit outside every range reads as a field: {got:?}"
    );
}

#[test]
fn the_declarations_describe_themselves_at_the_offsets_they_are_read_at() {
    type Header = Raw<FiPackHeader>;
    type Entry = Raw<FiPackEntry>;
    let pack = pack();
    let described = |ty: &str| -> (usize, Vec<(String, usize)>) {
        let t = pack.type_by_name(ty).unwrap();
        let fields = t.fields.iter().map(|f| (f.name.clone(), f.offset));
        (t.size, fields.collect())
    };
    let named = |pairs: &[(&str, usize)]| -> Vec<(String, usize)> {
        pairs.iter().map(|(n, o)| (n.to_string(), *o)).collect()
    };

    // The count and the heap size are private, so each is found where the writer puts it.
    let (count, heap) = (0xA1B2_C3D4u32, 0x0E5F_6070u32);
    let mut written = Vec::new();
    RpfHeader {
        entry_count: count,
        name_heap_size: heap,
        name_shift: 0,
        xcompress: false,
        encryption: RpfEncryption::None,
        platform: None,
    }
    .write(&mut written)
    .unwrap();
    let heap_at = landed(&written, &heap.to_le_bytes());
    assert_eq!(
        described("FiPackHeader"),
        (
            HEADER,
            named(&[
                ("magic", offset_of!(Header, magic)),
                ("entry_count", landed(&written, &count.to_le_bytes())),
                ("name_heap_size", heap_at),
                ("encrypted", offset_of!(Header, encrypted)),
            ])
        )
    );

    let name_offset = 0xBEEFu16;
    let mut written = Vec::new();
    RpfEntry::Directory(RpfDirectoryEntry {
        name_offset: name_offset.into(),
        first_entry_index: 0,
        entry_count: 0,
    })
    .write(&mut written)
    .unwrap();
    let entry = pack.type_by_name("FiPackEntry").unwrap();
    let word = &entry.fields[0];
    let word_at = landed(&written, &name_offset.to_le_bytes());
    assert_eq!(
        described("FiPackEntry"),
        (
            ENTRY,
            named(&[(word.name.as_str(), word_at), ("u", offset_of!(Entry, u))])
        )
    );
    assert_eq!(
        entry.field_by_name("u").unwrap().repr,
        FieldRepr::Opaque {
            size: size_of::<Raw<FiPackEntryU>>()
        }
    );
    decodes_as_described("FiPackEntry", u64::BITS, &word.bits, |bits| {
        let mut block = [0u8; ENTRY];
        let le = bits.to_le_bytes();
        block[word_at..word_at + le.len()].copy_from_slice(&le);
        let tag = RpfEntryTag::of(&block);
        BTreeMap::from([
            ("name_offset", tag.name_offset.into()),
            ("consumed_size", tag.consumed_size.into()),
            ("file_offset", tag.file_offset_raw.into()),
            ("is_resource", tag.is_resource.into()),
        ])
    });

    let heap_word = pack
        .type_by_name("FiPackHeader")
        .unwrap()
        .field_by_name("name_heap_size")
        .unwrap();
    decodes_as_described("FiPackHeader", u32::BITS, &heap_word.bits, |bits| {
        let mut block = [0u8; HEADER];
        let magic = RPF7_MAGIC.to_le_bytes();
        let at = offset_of!(Header, magic);
        block[at..at + magic.len()].copy_from_slice(&magic);
        let le = u32::try_from(bits).unwrap().to_le_bytes();
        block[heap_at..heap_at + le.len()].copy_from_slice(&le);
        let (_, header) = owned::<FiPackHeader>(&block);
        BTreeMap::from([
            ("name_shift", header.name_shift().into()),
            ("is_x_compressed", header.is_x_compressed().into()),
        ])
    });
}

/// Every archive an install ships starts with a plaintext header, key or no key.
#[test]
fn every_shipped_archive_header_reads_typed() {
    let roots = game_roots();
    if roots.is_empty() {
        skip_install(
            "the shipped RPF7 headers",
            format_args!("no install root; set {GAME_ROOT_ENV}"),
        );
        return;
    }
    let (mut archives, mut by_mode) = (0usize, BTreeMap::<String, usize>::new());
    let mut stack: Vec<PathBuf> = roots;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("rpf"))
            {
                use std::io::Read;
                let mut block = [0u8; HEADER];
                std::fs::File::open(&path)
                    .and_then(|mut f| f.read_exact(&mut block))
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                let (_, header) = owned::<FiPackHeader>(&block);
                assert_eq!(header.magic, RPF7_MAGIC, "{}", path.display());
                let decoded = RpfHeader::read(&mut Cursor::new(&block)).unwrap();
                assert_eq!(
                    decoded.name_shift,
                    header.name_shift(),
                    "{}",
                    path.display()
                );
                assert_eq!(
                    decoded.encryption,
                    RpfEncryption::from_raw(header.encrypted)
                );
                *by_mode
                    .entry(format!("{:?}", decoded.encryption))
                    .or_default() += 1;
                archives += 1;
            }
        }
    }
    println!("{archives} shipped archives: every header reads typed; by encryption {by_mode:?}");
    assert!(archives > 0);
}

/// Every archive an install ships opens with the keys the default path
/// resolves: the executable's own, and NG keys from `OPENRAGE_KEY_SNAPSHOT`
/// or else the bundled snapshot (D18).
#[test]
fn every_shipped_archive_opens_with_the_default_keys() {
    use std::sync::Arc;

    use or_io::container::rpf::crypto::{extract_keys, Gta5RetailCryptoProvider};

    let roots = game_roots();
    if roots.is_empty() {
        skip_install(
            "the shipped RPF7 archives, opened",
            format_args!("no install root; set {GAME_ROOT_ENV}"),
        );
        return;
    }
    for root in roots {
        let exe = ["GTA5_Enhanced.exe", "GTA5.exe"]
            .iter()
            .map(|name| root.join(name))
            .find(|path| path.is_file())
            .unwrap_or_else(|| panic!("no game executable under {}", root.display()));
        let keys = Arc::new(extract_keys(&exe, &|_| {}).unwrap());
        let (mut opened, mut entries, mut by_mode) = (0usize, 0usize, BTreeMap::new());
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("rpf"))
                {
                    let provider = Arc::new(Gta5RetailCryptoProvider::new(Arc::clone(&keys)));
                    let archive = RpfArchive::open_with(&path, provider)
                        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                    *by_mode
                        .entry(format!("{:?}", archive.header.encryption))
                        .or_insert(0usize) += 1;
                    entries += archive.entries.len();
                    opened += 1;
                }
            }
        }
        println!(
            "{}: {opened} archives opened with the default keys, {entries} entries; \
             by encryption {by_mode:?}",
            root.display()
        );
        assert!(opened > 0, "{}", root.display());
    }
}
