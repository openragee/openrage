//! Unit tests for the `rpf9` verbs' pure logic — selector matching, extraction selection, the
//! destination-safe name, the JSON row and the inner-path split — on hand-built entries with no game
//! bytes, so they run in the default tier. The listing against a real build is the install-gated
//! `gta6_archive_explorer` test in or-io.

use super::*;
use or_io::container::rpf::Gta6Cipher;

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
    let n = do_extract(
        &archive,
        dest.path(),
        &[],
        Some(&index.to_string()),
        None,
        true,
        &keys,
        &resolver,
    )
    .expect("the extract runs");
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
