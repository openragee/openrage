//! Checks of `hashes census` on an archive laid out by hand and enciphered under a synthetic
//! white-box key, a document and a program built here, so no game byte is involved; and, gated on
//! the install, a run over the build.

use super::*;
use or_io::container::rpf::{
    Rpf9Builder, Rpf9Toc, GROUP_CHUNK_RECORD, GROUP_CONTENT_HASH, GROUP_ENTRY_ROW, GROUP_NAME32,
    GROUP_NAME64,
};
use or_io::gta6::hash::name_hash64;
use or_io::gta6::names::Vocabulary;
use or_io::gta6::script::compiled::{
    check_copy, HEADER_LEN, OPCODE_MULTIPLIER, OPERAND_KEY, OPERAND_MULTIPLIER,
};
use or_io::meta::instance::HashedString;

const REGISTRY_ID: u64 = 0xDEAD_BEEF_0200_0000;
const KEY_ID: u32 = 0xDEAD_BEEF;
const BOOTSTRAP_REGISTRY_ID: u64 = 0x1000_0001_0200_0000;
/// The round tables feeding each output lane of the cipher.
const LANES: [[usize; 4]; 4] = [[0, 13, 10, 7], [1, 14, 4, 11], [5, 2, 8, 15], [9, 6, 12, 3]];
const WORDS: usize = 60;
const MAPS: usize = 16;
const MAP_LEN: usize = 256;
const BLOB_LEN: usize = 4 + WORDS * 4 + MAPS * MAP_LEN;
const NAMES_HEAP: usize = 0x40;
const GAP: usize = 0x20;

fn put16(t: &mut [u8], at: usize, v: u16) {
    t[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

fn put32(t: &mut [u8], at: usize, v: u32) {
    t[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn put64(t: &mut [u8], at: usize, v: u64) {
    t[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

fn round8(x: usize) -> usize {
    (x + 7) & !7
}

fn round16(x: usize) -> usize {
    (x + 15) & !15
}

fn push_perm(out: &mut Vec<u8>, k: usize) {
    for x in 0..256usize {
        out.push(((x + k * 7 + 1) % 256) as u8);
    }
}

fn mix8(b: u8) -> u8 {
    let (mut y, mut acc) = (0u8, 0u8);
    for i in (0..8).rev() {
        acc ^= (b >> i) & 1;
        y |= acc << i;
    }
    y
}

/// A synthetic white-box snapshot holding the bootstrap key and one archive key: byte permutations
/// for the maps and invertible affine round tables, the shape the encrypt side inverts.
fn keys() -> GtaViKeys {
    let mut slot_of = [0usize; 16];
    for lane in LANES.iter() {
        for (slot, &k) in lane.iter().enumerate() {
            slot_of[k] = slot;
        }
    }
    let mut out = b"ORGTAVIK".to_vec();
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
    GtaViKeys::parse(&out).expect("the synthetic snapshot parses")
}

/// An archive laid out by hand, every body stored raw: a header, four descriptors in 0x150 bytes
/// of metadata, the columns row24, name64, name32 and an empty `0xe0c1` in the main region and
/// `0xb6f5` external, bodies `GAP` apart, and a names heap.
fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let n = entries.len();
    let stored: Vec<Vec<u8>> = (entries.iter())
        .map(|(_, body)| {
            let mut s = body.to_vec();
            s.resize(round16(s.len()), 0);
            s
        })
        .collect();
    let groups_start = 0x1B0;
    let main_index = groups_start + 5 * 24;
    let main_start = round8(main_index + 4 * n);
    let (name64, name32, e0c1) = (24 * n, 32 * n, round8(36 * n));
    let split = round16(main_start + e0c1);
    let ext_start = round8(split + n);
    let toc_end = round16(ext_start + 8 * n);

    let mut t = vec![0u8; toc_end];
    put32(&mut t, 0x00, RPF9_MAGIC);
    put32(&mut t, 0x04, toc_end as u32);
    put32(&mut t, 0x08, split as u32);
    put32(&mut t, 0x0C, NAMES_HEAP as u32);
    for (i, b) in t[0x10..0x18].iter_mut().enumerate() {
        *b = i as u8 + 1;
    }
    t[0x19] = 0x7A;
    t[0x1A] = 4;
    put32(&mut t, 0x1C, 0x150);
    put16(&mut t, 0x20, 5);
    put16(&mut t, 0x22, 1);
    put32(&mut t, 0x24, n as u32);
    put32(&mut t, 0x28, 0xC0FF_EE11);
    t[0x2C] = 1;
    t[0x2D] = 0x0F;
    t[0x2E] = 0x09;
    put32(&mut t, 0x30, 0x10);
    for (i, (kind, off)) in [(1u16, 0u32), (5, 0x20), (6, 0x40), (9, 0x140)]
        .iter()
        .enumerate()
    {
        put16(&mut t, 0x40 + 8 * i, *kind);
        put32(&mut t, 0x44 + 8 * i, *off);
    }
    put32(&mut t, 0x60, KEY_ID);
    put16(&mut t, 0x64, 16);
    t[0x66..0x76].fill(0x33);
    for (i, b) in t[0x80..0x1B0].iter_mut().enumerate() {
        *b = (i * 7 + 1) as u8;
    }
    let directory = [
        (GROUP_ENTRY_ROW, n, 0, 24, 0),
        (GROUP_NAME64, n, name64, 8, 0),
        (GROUP_NAME32, n, name32, 4, 0),
        (GROUP_CHUNK_RECORD, 0, e0c1, 8, 0),
        (GROUP_CONTENT_HASH, n, 0, 8, 1),
    ];
    for (i, &(hash, count, offset, width, external)) in directory.iter().enumerate() {
        let at = groups_start + 24 * i;
        put64(&mut t, at, hash);
        put32(&mut t, at + 8, count as u32);
        put32(&mut t, at + 12, offset as u32);
        put32(&mut t, at + 16, width | (external << 16));
    }

    let mut at = round16(toc_end + GAP);
    let mut starts = Vec::new();
    for (i, ((name, body), s)) in entries.iter().zip(&stored).enumerate() {
        let row = main_start + 24 * i;
        put64(&mut t, row, (0xFFu64 << 40) | (at as u64 / 16));
        put64(&mut t, row + 8, s.len() as u64 / 16);
        put64(&mut t, row + 16, body.len() as u64);
        put64(
            &mut t,
            main_start + name64 + 8 * i,
            name_hash64(name.as_bytes()),
        );
        put32(
            &mut t,
            main_start + name32 + 4 * i,
            name_hash32(name.as_bytes()),
        );
        put64(&mut t, ext_start + 8 * i, 0x1111_0000 + i as u64);
        t[main_index + 4 * i..main_index + 4 * i + 3].fill(i as u8);
        t[main_index + 4 * i + 3] = 0xFF;
        t[split + i] = i as u8;
        starts.push(at);
        at += s.len() + GAP;
    }

    let toc = Rpf9Toc {
        bytes: t,
        key_id: KEY_ID,
        registry_id: REGISTRY_ID,
        entry_count: n as u32,
        groups: Vec::new(),
    };
    let head = Rpf9Builder::serialize(&toc, &keys()).expect("the synthetic table enciphers");
    let mut file = vec![0u8; at + NAMES_HEAP];
    file[..head.len()].copy_from_slice(&head);
    for (start, s) in starts.iter().zip(&stored) {
        file[*start..*start + s.len()].copy_from_slice(s);
    }
    file[at..].fill(0x5A);
    file
}

/// A folder holding one synthetic archive of three entries.
fn install() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temp dir");
    let file = archive(&[
        ("alpha.bin", &b"alpha body"[..]),
        ("beta.bin", &b"beta body"[..]),
        ("gamma.bin", &[0x47u8; 40][..]),
    ]);
    std::fs::write(dir.path().join("synthetic.rpf"), file).expect("write the archive");
    dir
}

/// What a walk opens and names with, with no lifted listings.
fn with<'a>(keys: &'a GtaViKeys, names: &'a [Source]) -> Gta6<'a> {
    Gta6 {
        keys,
        names,
        natives: None,
    }
}

/// A source of names labelled `label` that knows `names`.
fn source(label: &'static str, names: &[&str]) -> Source {
    let mut vocabulary = Vocabulary::default();
    for name in names {
        vocabulary.add(name, None);
    }
    Source {
        label,
        resolver: NameResolver::from_vocabulary(vocabulary),
    }
}

/// One source that knows the first two entries' names and not the third's.
fn resolver() -> Vec<Source> {
    vec![source("zq vocabulary", &["alpha.bin", "beta.bin"])]
}

#[test]
fn two_named_entries_and_one_unnamed_count_as_two_resolved_and_one_not() {
    let dir = install();
    let archives = or_title::install::gta6_files::list(dir.path()).archive_paths();
    assert_eq!(game_of(&archives), Some(Game::Gta6));
    let options = Options::default();
    let (keys, resolver) = (keys(), resolver());
    let gta6 = with(&keys, &resolver);
    let census = census_gta6(dir.path(), &archives, &options, &Cancel::new(), &gta6);
    assert_eq!((census.archives, census.refused), (1, 0));
    for space in [Space::Name64, Space::Name32] {
        let count = census.count(space);
        let row = (count.seen(), count.resolved(), count.unresolved().len());
        assert_eq!(row, (3, 2, 1), "{}", space.label());
    }
    let gamma = name_hash64(b"gamma.bin");
    let place = census.count(Space::Name64).unresolved().get(&gamma);
    assert_eq!(place.map(String::as_str), Some("synthetic.rpf#2"));
    let columns = census.count(Space::Column);
    assert_eq!((columns.seen(), columns.resolved()), (5, 0));

    let table = report(&census, Game::Gta6, &options);
    assert!(table.contains("1 archives, 0 nested, 0 refused"), "{table}");
    let row = table.lines().find(|l| l.contains("entry name64"));
    let words: Vec<&str> = row
        .expect("an entry name64 row")
        .split_whitespace()
        .collect();
    assert_eq!(words[2..], ["3", "2", "1"], "{table}");
    assert!(table.contains("zq vocabulary 2 (66.7%)"), "{table}");

    let out = dir.path().join("unresolved.tsv");
    assert_eq!(write_unresolved(&census, &out).expect("it writes"), 7);
    let text = std::fs::read_to_string(&out).expect("read it back");
    let line = format!("entry name64\t{gamma:016x}\tsynthetic.rpf#2");
    assert!(text.lines().any(|l| l == line), "{text}");
}

/// A hash two sources name is credited to the first, and the report gives each source's share,
/// a source that named nothing included.
#[test]
fn each_source_is_credited_with_the_hashes_it_named_first() {
    let dir = install();
    let archives = or_title::install::gta6_files::list(dir.path()).archive_paths();
    let names = [
        source("zq first", &["alpha.bin"]),
        source("zq second", &["alpha.bin", "beta.bin", "gamma.bin"]),
        source("zq third", &["beta.bin"]),
    ];
    let options = Options::default();
    let keys = keys();
    let census = census_gta6(
        dir.path(),
        &archives,
        &options,
        &Cancel::new(),
        &with(&keys, &names),
    );
    for space in [Space::Name64, Space::Name32] {
        let by: Vec<(&str, usize)> = census.count(space).by.clone().into_iter().collect();
        assert_eq!(by, [("zq first", 1), ("zq second", 2)], "{}", space.label());
    }
    let table = report(&census, Game::Gta6, &options);
    let row = (table.lines()).find(|l| l.contains("entry name64") && l.contains("zq first"));
    assert_eq!(
        row.map(str::trim),
        Some("entry name64       zq first 1 (33.3%), zq second 2 (66.7%), zq third 0 (0.0%)"),
        "{table}"
    );
}

/// A virtual section the resource writer takes: a zero head, a block-map pointer at 0x08.
fn virtual_section() -> Vec<u8> {
    let mut v: Vec<u8> = (0..0x200)
        .map(|i| ((i * 13 + 2) % 251) as u8 | 0x80)
        .collect();
    v[..8].fill(0);
    v[8..16].copy_from_slice(&0x5000_0040u64.to_le_bytes());
    v[0x40..0x70].fill(0);
    v
}

/// A clear headerless resource stands under a 20-byte row, its body past a 16-byte slot: the row
/// reads past the slot as the resource it holds, and the census counts it read.
#[test]
fn a_clear_headerless_row_is_read_past_its_slot_and_counted() {
    use or_io::container::rpf::gta6_container_write::{build, Rpf9IndexKey, Rpf9NewBody};
    use or_io::container::rpf::{write_rpf9_resource, Rpf9Form, Rpf9Sections};
    use std::io::Cursor;

    let loose = write_rpf9_resource(&Rpf9Sections {
        form: Rpf9Form::BlockMapped,
        virtual_section: virtual_section(),
        physical: Vec::new(),
    })
    .expect("a loose resource");
    let (_, want) = read_rpf9_body(Cursor::new(&loose), 0, u64::MAX).expect("the loose file reads");
    let resource = Rpf9NewBody::Resource {
        type_code: 0x02,
        loose,
    };
    let key = Rpf9IndexKey {
        key_id: KEY_ID,
        iv: [0x33; 16],
    };
    let keys = keys();
    let plan = build(&[("zq/a.ydr".to_owned(), resource)], &key, &keys).expect("it builds");
    let mut bytes = Vec::new();
    (plan.write_to(&mut Cursor::new(Vec::<u8>::new()), &mut bytes)).expect("it is laid out");
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("headerless.rpf");
    std::fs::write(&path, bytes).expect("write the archive");

    let mut file = File::open(&path).expect("the archive");
    let mount = Gta6Mount::open_root(&mut file, &keys).expect("it mounts");
    let entry = &mount.archive.entries[0];
    assert!(
        entry.stored.is_none() && entry.cipher.is_none(),
        "a clear 20-byte row"
    );
    let read = read_row(&mut file, &mount, entry, &keys, DEFAULT_LIMIT);
    assert_eq!(
        read.expect("the row reads"),
        want,
        "the resource past the slot"
    );

    let archives = or_title::install::gta6_files::list(dir.path()).archive_paths();
    let options = Options::default();
    let census = census_gta6(
        dir.path(),
        &archives,
        &options,
        &Cancel::new(),
        &with(&keys, &[]),
    );
    assert_eq!((census.bodies, census.unread), (1, 0));
}

#[test]
fn a_census_with_no_names_resolves_nothing_and_a_cancelled_one_walks_nothing() {
    let dir = install();
    let archives = or_title::install::gta6_files::list(dir.path()).archive_paths();
    let options = Options::default();
    let keys = keys();
    let gta6 = with(&keys, &[]);
    let census = census_gta6(dir.path(), &archives, &options, &Cancel::new(), &gta6);
    assert_eq!(census.count(Space::Name64).resolved(), 0);
    assert_eq!(census.count(Space::Name64).unresolved().len(), 3);

    let cancel = Cancel::new();
    cancel.set();
    let census = census_gta6(dir.path(), &archives, &options, &cancel, &gta6);
    assert!(census.cancelled);
    assert_eq!(census.archives, 0);
    assert!(report(&census, Game::Gta6, &options).contains("CANCELLED"));
}

#[test]
fn the_body_ceiling_stops_reads_and_the_report_says_so() {
    let options = Options {
        every: 2,
        bodies: Cap::at(1, CapUnit::Count("bodies")),
        ..Options::default()
    };
    let cancel = Cancel::new();
    let mut walk = Walk::new(&options, &cancel);
    assert!(walk.admit(), "the first candidate is read");
    assert!(!walk.admit(), "the ceiling of one is reached");
    assert_eq!((walk.census.bodies, walk.census.sampled_out), (1, 0));
    assert!(walk.census.stopped);
    let table = report(&walk.census, Game::Gta6, &options);
    assert!(table.contains("STOPPED SHORT"), "{table}");

    let options = Options {
        every: 2,
        ..Options::default()
    };
    let mut walk = Walk::new(&options, &cancel);
    let read: Vec<bool> = (0..4).map(|_| walk.admit()).collect();
    assert_eq!(read, [true, false, true, false], "one in every two");
    assert_eq!(walk.census.sampled_out, 2);
}

#[test]
fn a_document_counts_its_names_and_resolves_a_hash_value_by_its_own_strings() {
    let (in_blob, absent) = (string_hash(b"zq_in_blob"), string_hash(b"zq_absent"));
    let mut inner = MetaInstance::new("zq_inner_type");
    inner.fields.push((
        "hash_12345678".into(),
        Value::Hash(HashedString::from_hash(in_blob)),
    ));
    let mut root = MetaInstance::new("hash_0BADF00D");
    root.fields
        .push(("zq_named".into(), Value::Hash(HashedString::new("zq_x"))));
    root.fields.push((
        "zq_other".into(),
        Value::Hash(HashedString::from_hash(absent)),
    ));
    root.fields.push((
        "zq_kind".into(),
        Value::Enum {
            type_name: "hash_00C0FFEE".into(),
            variant: "zq_variant".into(),
        },
    ));
    root.fields
        .push(("zq_inner".into(), Value::Struct(Box::new(inner))));
    let strings: HashSet<u32> = [in_blob].into();
    let mut census = Census::default();
    tally_instance(&root, "doc", &strings, &mut census);

    let row = |space| (census.count(space).seen(), census.count(space).resolved());
    assert_eq!(row(Space::Class), (2, 1));
    assert_eq!(row(Space::Member), (5, 4));
    assert_eq!(row(Space::Enum), (2, 1));
    assert_eq!(row(Space::Text), (3, 2));
    let unresolved = census.count(Space::Text).unresolved();
    assert_eq!(
        unresolved.get(&u64::from(absent)).map(String::as_str),
        Some("doc")
    );
    let member = census.count(Space::Member).unresolved();
    assert!(member.contains_key(&0x1234_5678));
}

/// The multiplicative inverse of an odd `m` modulo 2^16.
fn inverse16(m: u16) -> u16 {
    (1..=u16::MAX).find(|x| x.wrapping_mul(m) == 1).unwrap_or(0)
}

fn inverse8(m: u8) -> u8 {
    (1..=u8::MAX).find(|x| x.wrapping_mul(m) == 1).unwrap_or(0)
}

/// One instruction as the game stores it: the opcode word and its operand bytes whitened.
fn encode(op: u16, plain: &[u8]) -> Vec<u8> {
    let mut out = (op.wrapping_mul(inverse16(OPCODE_MULTIPLIER)).to_le_bytes()).to_vec();
    let inv = inverse8(OPERAND_MULTIPLIER);
    out.extend((plain.iter().zip(OPERAND_KEY)).map(|(p, k)| p.wrapping_mul(inv) ^ k));
    out
}

/// A compiled program body holding `code`.
fn program(code: &[u8]) -> Vec<u8> {
    const HASH: u32 = 0x1234_5678;
    /// What a pointer in the body adds to the body offset it names.
    const POINTER_BASE: u64 = 0x5000_0000;
    let mut b = vec![0u8; HEADER_LEN];
    put32(&mut b, 0x10, HASH);
    put32(&mut b, 0x24, check_copy(HASH));
    put32(&mut b, 0x28, 3);
    put32(&mut b, 0x2C, 3);
    put32(&mut b, 0xB8, 2);
    put64(&mut b, 0xE8, POINTER_BASE + HEADER_LEN as u64);
    put32(&mut b, 0xF8, code.len() as u32);
    b.extend_from_slice(code);
    b
}

#[test]
fn a_programs_natives_count_once_each_and_none_is_named() {
    let mut code = encode(NATIVE, &[0x56, 0x34, 0x12, 0x00, 2, 1]);
    code.extend(encode(NATIVE, &[0x56, 0x34, 0x12, 0x00, 2, 1]));
    code.extend(encode(NATIVE, &[0x21, 0x43, 0x65, 0x00, 0, 0]));
    code.extend(encode(0x90, &[]));
    let mut census = Census::default();
    tally_body(&program(&code), "prog", None, None, &mut census);
    assert_eq!(census.scripts, 1);
    let natives = census.count(Space::Native);
    assert_eq!((natives.seen(), natives.resolved()), (2, 0));
    let places: Vec<&str> = natives.unresolved().values().map(String::as_str).collect();
    assert_eq!(places, ["prog+0x0", "prog+0x10"]);
    let table = report(&census, Game::Gta6, &Options::default());
    assert!(table.contains("no lifted listings"), "{table}");

    let mut census = Census::default();
    tally_body(
        b"neither metadata nor a program",
        "x",
        None,
        None,
        &mut census,
    );
    assert_eq!((census.scripts, census.metadata), (0, 0));
    assert_eq!(census.count(Space::Native).seen(), 0);
}

/// A lifted listing declaring a named native at 0x123456 and a hash-only one at 0x654321.
const LISTING: &str = "[VmProgram(0x00000001U)]
public static class Program_00000001
{
    public static class Native
    {
        public static class PLAYER
        {
            public static void SET_THING(VmValue p0) => Vm.Native(0xAAUL, 0x123456U, 0, p0);
            public static void _0x00000000000000BB(VmValue p0) => Vm.Native(0xBBUL, 0x654321U, 0, p0);
        }
    }
}
";

#[test]
fn a_native_whose_address_a_listing_names_is_resolved_and_a_hash_only_one_is_not() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("demo.ysc.cs"), LISTING).expect("write the listing");
    // No configuration folder, so nothing is cached where the user's lives.
    let table = NativeAddresses::load(dir.path(), None).expect("the listing reads");
    assert_eq!(table.len(), 2);

    let mut code = encode(NATIVE, &[0x56, 0x34, 0x12, 0x00, 2, 1]);
    code.extend(encode(NATIVE, &[0x21, 0x43, 0x65, 0x00, 0, 0]));
    code.extend(encode(NATIVE, &[0x01, 0x00, 0x00, 0x00, 0, 0]));
    let mut census = Census {
        native_table: Some(table.len()),
        ..Census::default()
    };
    tally_body(&program(&code), "prog", None, Some(&table), &mut census);
    let natives = census.count(Space::Native);
    assert_eq!((natives.seen(), natives.resolved()), (3, 1));
    let unresolved: Vec<u64> = natives.unresolved().keys().copied().collect();
    assert_eq!(unresolved, [0x1, 0x65_4321], "hash-only and undeclared");

    let table = report(&census, Game::Gta6, &Options::default());
    let row = table.lines().find(|l| l.contains("script native"));
    let row = row.expect("a script native row");
    let words: Vec<&str> = row.split_whitespace().collect();
    assert_eq!(words[2..5], ["3", "1", "2"], "{table}");
    assert!(
        row.contains("the 2 natives the lifted listings declare"),
        "{row}"
    );
}

/// The key snapshot and the build the install-gated checks read; `None`, said as a skip, without.
fn the_build(what: &str) -> Option<(GtaViKeys, PathBuf)> {
    let present = |var: &str| {
        std::env::var_os(var)
            .map(PathBuf::from)
            .filter(|p| p.exists())
    };
    let (Some(snapshot), Some(build)) = (
        present(or_io::container::rpf::GTA6_SNAPSHOT_ENV),
        present("OPENRAGE_GTA6_BUILD"),
    ) else {
        or_test_support::skip_install(
            what,
            format_args!(
                "set {} and OPENRAGE_GTA6_BUILD",
                or_io::container::rpf::GTA6_SNAPSHOT_ENV
            ),
        );
        return None;
    };
    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
    Some((keys, build))
}

/// The clear 20-byte rows met first in the build, which a read at the row's start refused every
/// one of, read past their slot. Install-gated.
#[test]
fn clear_headerless_rows_of_the_build_read_past_their_slot() {
    /// The rows tried, and the deepest nesting walked to find them.
    const TRIES: usize = 16;
    const DEPTH: usize = 3;

    /// Try the clear 20-byte rows of `mount` and of the tables it nests, depth first.
    fn rows(
        file: &mut File,
        mount: &Gta6Mount,
        keys: &GtaViKeys,
        depth: usize,
        tried: &mut Vec<bool>,
    ) {
        for entry in &mount.archive.entries {
            if tried.len() >= TRIES {
                return;
            }
            if mount
                .entry_begins_archive(&mut *file, entry)
                .unwrap_or(false)
            {
                if depth < DEPTH {
                    if let Ok(child) = mount.open_nested(&mut *file, entry, keys) {
                        rows(file, &child, keys, depth + 1, tried);
                    }
                }
                continue;
            }
            let clear_slot = entry.cipher.is_none() && slot_extent(mount, entry).is_some();
            if clear_slot && row_size(mount, entry).is_some_and(|s| s <= 1 << 20) {
                tried.push(read_row(file, mount, entry, keys, DEFAULT_LIMIT).is_ok());
            }
        }
    }

    let what = "clear 20-byte rows of a GTA VI build";
    let Some((keys, build)) = the_build(what) else {
        return;
    };
    let mut tried = Vec::new();
    for path in or_title::install::gta6_files::list(&build).archive_paths() {
        let mut file = File::open(&path).expect("the archive opens");
        if let Ok(root) = Gta6Mount::open_root(&mut file, &keys) {
            rows(&mut file, &root, &keys, 0, &mut tried);
        }
        if tried.len() >= TRIES {
            break;
        }
    }
    if tried.is_empty() {
        or_test_support::skip_install(what, format_args!("no clear 20-byte row was met"));
        return;
    }
    let read = tried.iter().filter(|ok| **ok).count();
    assert!(read > 0, "{read} of {} rows read", tried.len());
}

#[test]
fn the_census_runs_on_the_build_and_prints_its_table() {
    let Some((keys, build)) = the_build("hashes census over a GTA VI build") else {
        return;
    };
    let archives = or_title::install::gta6_files::list(&build).archive_paths();
    assert_eq!(game_of(&archives), Some(Game::Gta6));
    // Top-level tables and a handful of bodies keep the run short.
    let options = Options {
        top_only: true,
        bodies: Cap::at(16, CapUnit::Count("bodies")),
        ..Options::default()
    };
    let census = census_gta6(
        &build,
        &archives,
        &options,
        &Cancel::new(),
        &with(&keys, &[]),
    );
    let table = report(&census, Game::Gta6, &options);
    assert!(census.archives > 0, "{table}");
    assert!(census.count(Space::Name64).seen() > 0, "{table}");
    assert!(census.count(Space::Column).seen() > 0, "{table}");
    for space in SPACES {
        assert!(table.contains(space.label()), "{table}");
    }
}
