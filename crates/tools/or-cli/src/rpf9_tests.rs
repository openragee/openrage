//! Unit tests for the `rpf9` verbs' pure logic — selector matching, extraction selection, the
//! destination-safe name, the JSON row and the inner-path split — on hand-built entries with no game
//! bytes, so they run in the default tier, and the nested extract, the PSO strings and `find` on a
//! synthetic archive. The listing against a real build is the install-gated `gta6_archive_explorer`
//! test in or-io.

use super::find::{do_find, parse_hash, parse_kind, Kind, Query};
use super::*;
use fixture::Body;
use or_io::container::rpf::Gta6Cipher;
use or_io::gta6::hash::{name_hash32, name_hash64};

#[allow(clippy::duplicate_mod)]
#[path = "rpf9_fixture.rs"]
mod fixture;

/// A hand-built entry: index, name64, optional recovered name, optional cipher.
fn entry(index: u32, name64: u64, name: Option<&str>, cipher: bool) -> Gta6Entry {
    Gta6Entry {
        index,
        name64,
        name32: None,
        name: name.map(str::to_owned),
        start: Some(0x1000 + u64::from(index) * 0x100),
        stored: Some(0x40),
        real: Some(0x30),
        class: Some(0),
        cipher: cipher.then_some(Gta6Cipher {
            key_id: 0x1000_0029,
            cipher_type: 17,
            iv: [0; 16],
            framing: [0, 0x400, 0, 0, 4],
        }),
    }
}

#[test]
fn a_selector_matches_name_hash_and_index() {
    let e = entry(3, 0x092D_F786_29F7_833A, Some("config.dat"), false);
    assert!(entry_matches(&e, "config.dat"));
    assert!(entry_matches(&e, "0x092df78629f7833a"));
    assert!(
        entry_matches(&e, "0x092DF78629F7833A"),
        "case-insensitive hash"
    );
    assert!(entry_matches(&e, "hash_092df78629f7833a"));
    assert!(entry_matches(&e, "3"), "by decimal index");
    assert!(!entry_matches(&e, "install.dat"));
    assert!(!entry_matches(&e, "2"));
}

#[test]
fn an_unnamed_entry_matches_only_its_hash_and_index() {
    let e = entry(0, 0x0123_4567_89AB_CDEF, None, false);
    assert!(entry_matches(&e, "hash_0123456789abcdef"));
    assert!(entry_matches(&e, "0"));
    assert!(!entry_matches(&e, "anything"));
}

#[test]
fn select_takes_the_entry_the_filter_or_everything() {
    let entries = vec![
        entry(0, 1, Some("a/one.dat"), false),
        entry(1, 2, Some("a/two.dat"), false),
        entry(2, 3, Some("b/three.dat"), false),
    ];
    // --entry picks exactly one.
    let one = select_entries(&entries, Some("a/two.dat"), None);
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].index, 1);
    // --filter is a substring over the display name.
    let a = select_entries(&entries, None, Some("a/"));
    assert_eq!(a.len(), 2);
    // Neither flag takes every entry.
    assert_eq!(select_entries(&entries, None, None).len(), 3);
    // --entry wins over --filter when both are given.
    let both = select_entries(&entries, Some("b/three.dat"), Some("a/"));
    assert_eq!(both.len(), 1);
    assert_eq!(both[0].index, 2);
}

#[test]
fn the_destination_name_cannot_escape_the_folder() {
    // A normal name keeps its subdirectories.
    assert_eq!(
        safe_out_name(&entry(0, 1, Some("models/ped.ydr"), false)),
        "models/ped.ydr"
    );
    // Back-slashes become slashes; a traversal or drive prefix is stripped.
    assert_eq!(
        safe_out_name(&entry(0, 1, Some("..\\..\\secret.dat"), false)),
        "secret.dat"
    );
    assert_eq!(
        safe_out_name(&entry(0, 1, Some("C:/windows/evil.dat"), false)),
        "windows/evil.dat"
    );
    // A ':' in a later component (an NTFS alternate-data-stream trick) becomes '_', not just in the
    // first component where a drive prefix is stripped.
    assert_eq!(safe_out_name(&entry(0, 1, Some("a/b:c"), false)), "a/b_c");
    // Other Windows-forbidden characters map to '_' too.
    assert_eq!(
        safe_out_name(&entry(0, 1, Some("a/b?c*d"), false)),
        "a/b_c_d"
    );
    // A wholly-stripped name falls back to the hash, so a file is still written.
    assert_eq!(
        safe_out_name(&entry(0, 0xDEAD_BEEF_0000_0001, Some("../.."), false)),
        "hash_deadbeef00000001"
    );
    // An unnamed entry uses its hash.
    assert_eq!(
        safe_out_name(&entry(0, 0xDEAD_BEEF_0000_0002, None, false)),
        "hash_deadbeef00000002"
    );
}

#[test]
fn the_json_row_carries_name_hash_extent_and_cipher() {
    let e = entry(5, 0x00AB, Some("x.dat"), true);
    let v = entry_json(&e);
    assert_eq!(v["index"], 5);
    assert_eq!(v["name"], "x.dat");
    assert_eq!(v["hash"], "0x00000000000000ab");
    assert_eq!(v["start"], 0x1500);
    assert_eq!(v["stored"], 0x40);
    assert_eq!(v["real"], 0x30);
    assert_eq!(v["cipher"], true);

    // An unnamed, un-extent entry serialises its absent fields as null.
    let bare = Gta6Entry {
        index: 0,
        name64: 1,
        name32: None,
        name: None,
        start: None,
        stored: None,
        real: None,
        class: None,
        cipher: None,
    };
    let b = entry_json(&bare);
    assert!(b["name"].is_null());
    assert!(b["start"].is_null());
    assert_eq!(b["cipher"], false);

    // entries_json wraps them in an array.
    let arr = entries_json(&[e, bare]);
    assert_eq!(arr.as_array().map(|a| a.len()), Some(2));
}

#[test]
fn an_inner_path_splits_on_either_separator() {
    assert_eq!(split_inner("a.rpf/b.rpf"), vec!["a.rpf", "b.rpf"]);
    assert_eq!(split_inner("a.rpf\\b.rpf"), vec!["a.rpf", "b.rpf"]);
    assert_eq!(split_inner("/a.rpf//b.rpf/"), vec!["a.rpf", "b.rpf"]);
    assert!(split_inner("").is_empty());
}

// --- a synthetic archive: the nested extract, the PSO strings, find ---------------------------------

/// `archive` written to `name` in a new temp folder, which the caller keeps alive.
fn write_temp(archive: &[u8], name: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join(name);
    std::fs::write(&path, archive).expect("write the archive");
    (dir, path)
}

/// A resolver that knows `names`, hashed as the fixture names its entries.
fn resolver_of(names: &[&str]) -> NameResolver {
    let mut resolver = NameResolver::new();
    for name in names {
        resolver.insert(name_hash64(name.as_bytes()), *name);
    }
    resolver
}

/// A parent holding a nested archive of two documents, then a leaf of its own.
fn nested_sample() -> (Vec<u8>, Vec<u8>) {
    let child = fixture::archive(&[
        ("inner/a.pso", Body::Zlib(fixture::pso("zq_alpha"))),
        ("inner/b.ydr", Body::Raw(vec![0x42; 40])),
    ]);
    let parent = fixture::archive(&[
        ("child.rpf", Body::Nested(child.clone())),
        ("top.ydr", Body::Raw(vec![0x07; 24])),
    ]);
    (parent, child)
}

const SAMPLE_NAMES: [&str; 4] = ["child.rpf", "inner/a.pso", "inner/b.ydr", "top.ydr"];

/// Before this, a nested archive was skipped and nothing was written.
#[test]
fn extract_writes_a_nested_archive_as_its_row_states_it() {
    let keys = fixture::keys();
    let (parent, child) = nested_sample();
    let (dir, path) = write_temp(&parent, "parent.rpf");
    let dest = dir.path().join("out");
    let pick = Pick {
        entry: Some("child.rpf"),
        filter: None,
        decrypt: false,
        strings: false,
    };
    let resolver = resolver_of(&SAMPLE_NAMES);
    let n = do_extract(&path, &dest, &[], &pick, &keys, &resolver).expect("the extract runs");
    assert_eq!(n, 1, "the nested archive is written");

    let written = std::fs::read(dest.join("child.rpf")).expect("read the written archive");
    assert_eq!(
        written, child,
        "the file is the row's extent, byte for byte"
    );
    let alone = Gta6Mount::open_root(&mut std::io::Cursor::new(&written), &keys)
        .expect("the written file opens as an archive");
    let mut file = File::open(&path).expect("open the parent");
    let mount = Gta6Mount::open_root(&mut file, &keys).expect("mount the parent");
    let nested = mount
        .open_nested(&mut file, &mount.archive.entries[0], &keys)
        .expect("mount the nested archive");
    assert_eq!(alone.archive.entries, nested.archive.entries);
}

#[test]
fn a_nested_row_with_no_stored_size_is_refused_and_leaves_no_file() {
    let keys = fixture::keys();
    let (parent, _) = nested_sample();
    let (dir, path) = write_temp(&parent, "parent.rpf");
    let mut file = File::open(&path).expect("open the parent");
    let mount = Gta6Mount::open_root(&mut file, &keys).expect("mount the parent");
    let out = dir.path().join("child.rpf");

    let mut unsized_row = mount.archive.entries[0].clone();
    unsized_row.stored = None;
    assert!(write_nested(&mount, &mut file, &unsized_row, &out).is_err());
    assert!(!out.exists());

    // A row stating more than the file holds is a short copy, refused with no partial file.
    let mut past_end = mount.archive.entries[0].clone();
    past_end.stored = Some(parent.len() as u64);
    assert!(write_nested(&mount, &mut file, &past_end, &out).is_err());
    assert!(!out.exists());
}

#[test]
fn list_and_extract_read_a_pso_documents_strings() {
    let keys = fixture::keys();
    let archive = fixture::archive(&[
        ("doc.pso", Body::Zlib(fixture::pso("zq_named_thing"))),
        ("raw.bin", Body::Raw(b"not a document".to_vec())),
    ]);
    let (dir, path) = write_temp(&archive, "docs.rpf");
    let mut file = File::open(&path).expect("open the archive");
    let mount = Gta6Mount::open_root(&mut file, &keys).expect("mount the archive");

    let strings = mount_strings(&mount, &mut file, &keys);
    assert_eq!(
        strings.get(&0),
        Some(&Ok(vec!["zq_named_thing".to_string()]))
    );
    assert!(!strings.contains_key(&1), "a body that is no PSO has none");
    assert_eq!(body_strings(b"not a document", &keys), Ok(None));

    let mut json = entries_json(&mount.archive.entries);
    add_strings_json(&mut json, &mount.archive.entries, &strings);
    assert_eq!(json[0]["strings"][0], "zq_named_thing");
    assert!(json[1].get("strings").is_none());

    let dest = dir.path().join("out");
    let pick = Pick {
        entry: Some("0"),
        filter: None,
        decrypt: false,
        strings: true,
    };
    let n = do_extract(&path, &dest, &[], &pick, &keys, &NameResolver::new())
        .expect("the extract runs");
    assert_eq!(n, 1);
    let body = dest.join(format!("hash_{:016x}", name_hash64(b"doc.pso")));
    let side =
        std::fs::read_to_string(strings_file(&body)).expect("the strings are written beside it");
    assert_eq!(side, "zq_named_thing\n");
}

#[test]
fn find_walks_nested_archives_by_name_hash_and_kind() {
    let keys = fixture::keys();
    let (parent, _) = nested_sample();
    let (dir, path) = write_temp(&parent, "parent.rpf");
    let resolver = resolver_of(&SAMPLE_NAMES);
    let find = |query: Query| {
        let found = do_find(dir.path(), &query, &keys, &resolver).expect("the search runs");
        assert_eq!((found.archives, found.refused), (1, 0));
        found.hits
    };

    let by_name = find(Query {
        name: Some(".ydr".to_string()),
        ..Query::default()
    });
    let rows: Vec<(String, String, Option<u64>)> = by_name
        .iter()
        .map(|h| (h.inner.join("/"), h.entry.clone(), h.size))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("child.rpf".to_string(), "inner/b.ydr".to_string(), Some(40)),
            (String::new(), "top.ydr".to_string(), Some(24)),
        ]
    );
    assert!(by_name.iter().all(|h| h.archive == path));

    let hex = format!("0x{:016x}", name_hash64(b"inner/a.pso"));
    let by_hash = find(Query {
        hash: Some(parse_hash(&hex).expect("a hex hash")),
        ..Query::default()
    });
    assert_eq!(by_hash.len(), 1);
    assert_eq!(by_hash[0].entry, "inner/a.pso");

    let by_name32 = find(Query {
        hash: Some(u64::from(name_hash32(b"top.ydr"))),
        ..Query::default()
    });
    assert_eq!(by_name32.len(), 1);
    assert_eq!(by_name32[0].entry, "top.ydr");

    let archives = find(Query {
        kind: Some(Kind::Archive),
        ..Query::default()
    });
    assert_eq!(archives.len(), 1);
    assert_eq!(
        (archives[0].inner.is_empty(), archives[0].entry.as_str()),
        (true, "child.rpf")
    );
    let media = find(Query {
        kind: Some(Kind::Class(0xFF)),
        ..Query::default()
    });
    assert_eq!(media.len(), 3, "every raw leaf, nested ones included");

    let none = find(Query {
        name: Some("inner".to_string()),
        kind: Some(Kind::Archive),
        ..Query::default()
    });
    assert!(none.is_empty(), "every criterion must hold");

    let empty = tempfile::tempdir().expect("a temp dir");
    assert!(do_find(empty.path(), &Query::default(), &keys, &resolver).is_err());
}

#[test]
fn find_parses_hashes_and_kinds() {
    assert_eq!(parse_hash("0x00FF"), Ok(0xFF));
    assert_eq!(
        parse_hash("hash_092df78629f7833a"),
        Ok(0x092D_F786_29F7_833A)
    );
    assert_eq!(parse_hash("2a"), Ok(0x2A));
    assert!(parse_hash("zz").is_err());
    assert_eq!(parse_kind("Archive"), Ok(Kind::Archive));
    assert_eq!(parse_kind("resource"), Ok(Kind::Class(0x0B)));
    assert_eq!(parse_kind("media"), Ok(Kind::Class(0xFF)));
    assert_eq!(parse_kind("0x0e"), Ok(Kind::Class(0x0E)));
    assert!(parse_kind("bogus").is_err());
    assert!(parse_kind("0x100").is_err());
}

// --- install-gated: extract --decrypt writes a deciphered body -------------------------------------

/// The build root holding `*.rpf`; set with the key snapshot for the install-gated test below.
const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// The flagged body picked is held small so the decipher-and-inflate stays cheap.
const SAMPLE_CAP: u64 = 0x40000;

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

/// Every file written under `dir`, recursively — the extract lands its output at a name-derived path.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(files_under(&path));
            } else {
                out.push(path);
            }
        }
    }
    out
}

/// `rpf9 extract --decrypt` of one flagged entry writes the same deciphered, inflated body the mount
/// produces, whose head is a recognised kind (PSO `PSIN`, `RBF0`, text, or a headerless script slot).
/// This fails without `--decrypt` wired through, which skips every flagged entry and writes nothing.
/// Install-gated: it needs the content-key snapshot and the build, and skips cleanly without them.
#[test]
fn extract_decrypt_writes_a_deciphered_body() {
    let (Some(snapshot), Some(build)) = (
        env_path(or_io::container::rpf::GTA6_SNAPSHOT_ENV),
        env_path(BUILD_ENV),
    ) else {
        or_test_support::skip_install(
            "rpf9 extract --decrypt over a build",
            format_args!(
                "set {} (with content keys) and {BUILD_ENV}",
                or_io::container::rpf::GTA6_SNAPSHOT_ENV
            ),
        );
        return;
    };
    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
    if !keys.has_content_keys() {
        or_test_support::skip_install(
            "rpf9 extract --decrypt over a build",
            format_args!("the snapshot carries no content section; rebuild it"),
        );
        return;
    }
    let resolver = NameResolver::new();

    // Find one flagged, small content-ciphered entry and the archive it lives in, plus the body the
    // mount deciphers for it — the extract must reproduce exactly these bytes.
    let mut archives: Vec<PathBuf> = std::fs::read_dir(&build)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("rpf"))
        .collect();
    archives.sort();

    let mut found: Option<(PathBuf, u32, Vec<u8>)> = None;
    'outer: for path in &archives {
        let mut file = File::open(path).expect("open the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys).expect("mount the archive");
        for entry in &mount.archive.entries {
            // A flagged entry small enough for the smoke test; `read_entry_decrypted` refuses any
            // cipher it does not handle, so a non-content cipher is skipped by the `Ok` below.
            if entry.cipher.is_none() || entry.stored.is_none_or(|s| s == 0 || s > SAMPLE_CAP) {
                continue;
            }
            if let Ok(body) = mount.read_entry_decrypted(&mut file, entry, &keys) {
                if !body.is_empty() {
                    found = Some((path.clone(), entry.index, body));
                    break 'outer;
                }
            }
        }
    }
    let Some((archive, index, expected)) = found else {
        or_test_support::skip_install(
            "rpf9 extract --decrypt over a build",
            format_args!("no small flagged content entry in the build to extract"),
        );
        return;
    };

    let dest = tempfile::tempdir().expect("a temp dir");
    let selector = index.to_string();
    let pick = Pick {
        entry: Some(&selector),
        filter: None,
        decrypt: true,
        strings: false,
    };
    let n =
        do_extract(&archive, dest.path(), &[], &pick, &keys, &resolver).expect("the extract runs");
    assert_eq!(n, 1, "exactly the one flagged entry is written");

    let written = files_under(dest.path());
    assert_eq!(written.len(), 1, "one file on disk, got {written:?}");
    let on_disk = std::fs::read(&written[0]).expect("read the written body");
    assert_eq!(
        on_disk, expected,
        "the written body matches the mount's deciphered, inflated bytes"
    );

    // The head is a recognised inflated kind, never ciphertext.
    let head_ok = on_disk.starts_with(b"PSIN")
        || on_disk.starts_with(b"RBF0")
        || is_text_head(&on_disk)
        || (on_disk.len() >= 8 && on_disk[..8].iter().all(|&b| b == 0));
    assert!(
        head_ok,
        "head {:02x?} is not a recognised inflated kind",
        &on_disk[..on_disk.len().min(4)]
    );
}

// --- install-gated: a nested archive extracted from its row opens on its own -----------------------

/// A nested archive larger than this is not picked, so the copy stays cheap.
const NESTED_CAP: u64 = 64 << 20;

/// `rpf9 extract` of a nested archive writes its row's extent, which opens as an archive listing as
/// many entries as the mount of it inside its parent. Install-gated: it needs the key snapshot and the
/// build, and skips cleanly without them.
#[test]
fn a_nested_archive_extracted_from_its_row_opens_as_the_mount_does() {
    let what = "rpf9 extract of a nested archive over a build";
    let (Some(snapshot), Some(build)) = (
        env_path(or_io::container::rpf::GTA6_SNAPSHOT_ENV),
        env_path(BUILD_ENV),
    ) else {
        or_test_support::skip_install(
            what,
            format_args!(
                "set {} and {BUILD_ENV}",
                or_io::container::rpf::GTA6_SNAPSHOT_ENV
            ),
        );
        return;
    };
    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");

    let mut found: Option<(PathBuf, u32, usize)> = None;
    'outer: for path in build_archives(&build) {
        let mut file = File::open(&path).expect("open the archive");
        let Ok(mount) = Gta6Mount::open_root(&mut file, &keys) else {
            continue;
        };
        for entry in &mount.archive.entries {
            if entry.cipher.is_some() || entry.stored.is_none_or(|s| s > NESTED_CAP) {
                continue;
            }
            if !mount
                .entry_begins_archive(&mut file, entry)
                .unwrap_or(false)
            {
                continue;
            }
            if let Ok(nested) = mount.open_nested(&mut file, entry, &keys) {
                found = Some((path.clone(), entry.index, nested.archive.entries.len()));
                break 'outer;
            }
        }
    }
    let Some((archive, index, mounted)) = found else {
        or_test_support::skip_install(what, format_args!("no small nested archive in the build"));
        return;
    };

    let dest = tempfile::tempdir().expect("a temp dir");
    let selector = index.to_string();
    let pick = Pick {
        entry: Some(&selector),
        filter: None,
        decrypt: false,
        strings: false,
    };
    let n = do_extract(
        &archive,
        dest.path(),
        &[],
        &pick,
        &keys,
        &NameResolver::new(),
    )
    .expect("the extract runs");
    assert_eq!(n, 1, "the nested archive is written");
    let written = files_under(dest.path());
    assert_eq!(written.len(), 1, "one file on disk, got {written:?}");
    let mut alone = File::open(&written[0]).expect("open the written archive");
    let opened = Gta6Mount::open_root(&mut alone, &keys).expect("it opens as an archive");
    assert_eq!(opened.archive.entries.len(), mounted);
}
