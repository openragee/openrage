//! Hermetic checks of `rpf9-diff` on archives laid out by hand and enciphered under a synthetic
//! white-box key, so no game byte is involved; the last test is install-gated.

use std::io::Cursor;

use super::*;
use fixture::{archive, keys, Body};
use or_io::container::rpf::{Rpf9ArchiveWriter, GTA6_SNAPSHOT_ENV};
use or_io::gta6::hash::name_hash64;

#[allow(dead_code, clippy::duplicate_mod)]
#[path = "rpf9_fixture.rs"]
mod fixture;

const NAMES: [&str; 11] = [
    "keep.bin",
    "gone.bin",
    "new.txt",
    "old.txt",
    "moved.txt",
    "edit.txt",
    "child.rpf",
    "inner.bin",
    "level.bin",
    "grand.rpf",
    "leaf.txt",
];

fn resolver() -> NameResolver {
    let mut resolver = NameResolver::new();
    for name in NAMES {
        resolver.insert(name_hash64(name.as_bytes()), name);
    }
    resolver
}

fn raw(b: &[u8]) -> Body {
    Body::Raw(b.to_vec())
}

fn diff_with(a: &[u8], b: &[u8], keys: &GtaViKeys, resolver: &NameResolver) -> Vec<Change> {
    let (mut ca, mut cb) = (Cursor::new(a), Cursor::new(b));
    diff_archives(&mut ca, &mut cb, keys, resolver).expect("the archives compare")
}

fn diff(a: &[u8], b: &[u8]) -> Vec<Change> {
    diff_with(a, b, &keys(), &resolver())
}

fn lines(added: &[&str], removed: &[&str]) -> Option<Summary> {
    Some(Summary::Lines {
        added: added.iter().map(|s| s.to_string()).collect(),
        removed: removed.iter().map(|s| s.to_string()).collect(),
    })
}

/// Each change as `(kind, path, summary)`, a rename's path as `from -> to`.
fn outline(changes: &[Change]) -> Vec<(&'static str, String, Option<Summary>)> {
    changes
        .iter()
        .map(|c| match c {
            Change::Added { path, .. } => ("added", path.clone(), None),
            Change::Removed { path, .. } => ("removed", path.clone(), None),
            Change::Renamed { from, to, .. } => ("renamed", format!("{from} -> {to}"), None),
            Change::Changed { path, summary, .. } => ("changed", path.clone(), summary.clone()),
        })
        .collect()
}

/// A and B differ by one entry of each kind; B lists its entries in another order.
fn pair() -> (Vec<u8>, Vec<u8>) {
    let a = archive(&[
        ("keep.bin", raw(b"kept as it was")),
        ("gone.bin", raw(b"gone")),
        ("old.txt", raw(b"renamed body")),
        ("edit.txt", raw(b"line one\nline two\n")),
        (
            "child.rpf",
            Body::Nested(archive(&[
                ("inner.bin", raw(b"x")),
                ("level.bin", raw(b"same inside")),
            ])),
        ),
    ]);
    let b = archive(&[
        (
            "child.rpf",
            Body::Nested(archive(&[
                ("inner.bin", raw(b"y")),
                ("level.bin", raw(b"same inside")),
            ])),
        ),
        ("new.txt", raw(b"fresh")),
        ("edit.txt", raw(b"line one\nline 2\n")),
        ("moved.txt", raw(b"renamed body")),
        ("keep.bin", raw(b"kept as it was")),
    ]);
    (a, b)
}

#[test]
fn an_archive_against_itself_has_no_change() {
    let (a, _) = pair();
    assert!(diff(&a, &a).is_empty());
    assert_eq!(render("a", "b", &[]), "a and b: no entry differs\n");
}

#[test]
fn each_change_kind_is_found_by_name_not_position() {
    let (a, b) = pair();
    let changes = diff(&a, &b);
    let lines = |added: &[&str], removed: &[&str]| {
        Some(Summary::Lines {
            added: added.iter().map(|s| s.to_string()).collect(),
            removed: removed.iter().map(|s| s.to_string()).collect(),
        })
    };
    let summary = |c: &Change| match c {
        Change::Changed { summary, .. } => summary.clone(),
        _ => None,
    };
    let paths: Vec<(&str, String)> = changes
        .iter()
        .map(|c| match c {
            Change::Added { path, .. } => ("added", path.clone()),
            Change::Removed { path, .. } => ("removed", path.clone()),
            Change::Renamed { from, to, .. } => ("renamed", format!("{from} -> {to}")),
            Change::Changed { path, .. } => ("changed", path.clone()),
        })
        .collect();
    assert_eq!(
        paths,
        vec![
            ("changed", "edit.txt".to_string()),
            ("changed", "child.rpf".to_string()),
            ("changed", "child.rpf/inner.bin".to_string()),
            ("removed", "gone.bin".to_string()),
            ("renamed", "old.txt -> moved.txt".to_string()),
            ("added", "new.txt".to_string()),
        ]
    );
    assert_eq!(summary(&changes[0]), lines(&["line 2"], &["line two"]));
    assert_eq!(summary(&changes[1]), Some(Summary::Nested { changes: 1 }));
    assert_eq!(summary(&changes[2]), lines(&["y"], &["x"]));
    assert_eq!(
        changes[5],
        Change::Added {
            path: "new.txt".into(),
            size: Size {
                stored: Some(16),
                real: Some(5),
            },
        }
    );
}

#[test]
fn a_same_sized_body_with_other_bytes_is_no_rename() {
    // "gone" and "fresh" are both stored in 16 bytes; only the bytes tell them apart.
    let a = archive(&[("gone.bin", raw(b"gone"))]);
    let b = archive(&[("new.txt", raw(b"fresh"))]);
    let changes = diff(&a, &b);
    assert_eq!(changes.len(), 2);
    assert!(matches!(&changes[0], Change::Removed { path, .. } if path == "gone.bin"));
    assert!(matches!(&changes[1], Change::Added { path, .. } if path == "new.txt"));
}

#[test]
fn the_text_report_and_the_json_name_each_change() {
    let (a, b) = pair();
    let changes = diff(&a, &b);
    let text = render("a.rpf", "b.rpf", &changes);
    assert!(text.starts_with("a.rpf -> b.rpf: 1 added, 1 removed, 1 renamed, 3 changed\n"));
    for line in [
        "  + new.txt  stored 16, real 5",
        "  - gone.bin",
        "  > old.txt -> moved.txt",
        "  ~ edit.txt  stored 32, real 18 -> stored 16, real 16",
        "      text: 1 line(s) added, 1 removed",
        "        + line 2",
        "        - line two",
        "      nested archive: 1 change(s) inside",
        "  ~ child.rpf/inner.bin",
    ] {
        assert!(text.contains(line), "{line:?} missing from\n{text}");
    }
    let json = serde_json::to_value(&changes).expect("the changes serialise");
    assert_eq!(json[0]["kind"], "changed");
    assert_eq!(json[0]["summary"]["type"], "lines");
    assert_eq!(json[1]["summary"]["changes"], 1);
    assert_eq!(json[4]["kind"], "renamed");
    assert_eq!(json[4]["to"], "moved.txt");
    assert_eq!(json[5]["size"]["stored"], 16);
}

#[test]
fn lines_are_counted_as_a_multiset() {
    assert_eq!(
        line_diff(b"x\nx\ny\n", b"x\ny\nz\n"),
        Summary::Lines {
            added: vec!["z".into()],
            removed: vec!["x".into()],
        }
    );
}

#[test]
fn fields_are_compared_by_path() {
    let mut a = MetaInstance::new("CThing");
    a.set("count", Value::Int(1));
    a.set("label", Value::String("x".into()));
    a.set("list", Value::Array(vec![Value::UInt(7)]));
    let mut b = MetaInstance::new("CThing");
    b.set("count", Value::Int(2));
    b.set("list", Value::Array(vec![Value::UInt(7), Value::UInt(8)]));
    b.set("flag", Value::Bool(true));
    assert_eq!(
        field_diff("PSO", &a, &b),
        Summary::Fields {
            format: "PSO",
            added: vec!["CThing.flag = true".into(), "CThing.list[1] = 8".into()],
            removed: vec!["CThing.label = \"x\"".into()],
            changed: vec![FieldChange {
                path: "CThing.count".into(),
                before: "1".into(),
                after: "2".into(),
            }],
        }
    );
}

#[test]
fn a_body_is_summarised_by_its_head() {
    assert_eq!(head_kind(b"PSIN...."), Some(Head::Pso));
    assert_eq!(head_kind(b"RBF0...."), Some(Head::Rbf));
    assert_eq!(head_kind(b"<?xml version"), Some(Head::Text));
    assert_eq!(head_kind(b"plain list\n"), Some(Head::Text));
    assert_eq!(head_kind(&[0x00, 0x01, 0xFE]), None);
    assert_eq!(head_kind(b""), None);
    // A metadata head goes to the metadata reader, never to the line diff, even when it is printable.
    let meta = summarize(b"RBF0 one", b"RBF0 two");
    assert!(
        matches!(
            meta,
            Some(Summary::Fields { format: "RBF", .. } | Summary::Unread { .. })
        ),
        "{meta:?}"
    );
    assert_eq!(
        summarize(b"PSIN", b"text"),
        None,
        "two kinds are not compared"
    );
    assert_eq!(
        summarize(&[0, 1], &[0, 2]),
        None,
        "an unknown head is not read"
    );
}

/// 81 printable bytes, more than the head [`head_kind`] reads, so a stored form that opens with them
/// reads as text.
const LONG: &[u8] =
    b"a first chunk of plain text that runs well past the sixty-four byte head checked\n";

#[test]
fn a_body_stored_compressed_or_chunked_is_only_stored_apart_from_its_raw_form() {
    let full = [LONG, &b"tail"[..]].concat();
    let a = archive(&[
        ("edit.txt", Body::Raw(full.clone())),
        ("old.txt", raw(b"one body\nin two lines\n")),
        ("keep.bin", Body::Zlib(full)),
    ]);
    // A stored chunk of text, then a deflated tail that grows: the stored bytes open with the text
    // and the row's real size fits within them, but only the chunk table says where the body is.
    let b = archive(&[
        (
            "edit.txt",
            Body::Chunked(vec![(LONG.to_vec(), false), (b"tail".to_vec(), true)]),
        ),
        ("old.txt", Body::Zlib(b"one body\nin two lines\n".to_vec())),
        (
            "keep.bin",
            Body::Chunked(vec![(LONG.to_vec(), true), (b"tail".to_vec(), true)]),
        ),
    ]);
    let changes = diff(&a, &b);
    assert_eq!(
        outline(&changes),
        vec![
            ("changed", "edit.txt".to_string(), Some(Summary::Stored)),
            ("changed", "old.txt".to_string(), Some(Summary::Stored)),
            ("changed", "keep.bin".to_string(), Some(Summary::Stored)),
        ]
    );
    for change in &changes {
        let Change::Changed { before, after, .. } = change else {
            unreachable!("every change is a change");
        };
        assert_eq!(before.real, after.real, "{change:?}");
    }
    let text = render("a", "b", &changes);
    assert!(
        text.contains("  ~ edit.txt  stored 96, real 85 -> stored 112, real 85\n"),
        "{text}"
    );
    assert!(text.contains("      stored apart only"), "{text}");
    assert!(diff(&b, &b).is_empty(), "a chunked archive against itself");
}

#[test]
fn a_compressed_body_under_a_new_name_is_a_rename_and_an_edited_one_is_not() {
    let a = archive(&[
        ("old.txt", Body::Zlib(b"renamed body\n".to_vec())),
        ("gone.bin", Body::Zlib(b"one\n".to_vec())),
        (
            "edit.txt",
            Body::Chunked(vec![
                (b"line one\n".to_vec(), true),
                (b"line two\n".to_vec(), false),
            ]),
        ),
    ]);
    let b = archive(&[
        ("moved.txt", raw(b"renamed body\n")),
        ("new.txt", Body::Zlib(b"two\n".to_vec())),
        (
            "edit.txt",
            Body::Chunked(vec![
                (b"line one\n".to_vec(), true),
                (b"line 2\n".to_vec(), false),
            ]),
        ),
    ]);
    assert_eq!(
        outline(&diff(&a, &b)),
        vec![
            (
                "changed",
                "edit.txt".to_string(),
                lines(&["line 2"], &["line two"])
            ),
            ("renamed", "old.txt -> moved.txt".to_string(), None),
            ("removed", "gone.bin".to_string(), None),
            ("added", "new.txt".to_string(), None),
        ]
    );
}

#[test]
fn a_nested_archive_changed_two_levels_down_is_followed_to_its_leaves() {
    let side = |leaf: &[u8], keep: Body| {
        archive(&[(
            "child.rpf",
            Body::Nested(archive(&[
                (
                    "grand.rpf",
                    Body::Nested(archive(&[("leaf.txt", raw(leaf)), ("keep.bin", keep)])),
                ),
                ("level.bin", raw(b"same inside")),
            ])),
        )])
    };
    let a = side(b"before\nkept line\n", raw(b"kept as it was"));
    let b = side(
        b"after\nkept line\n",
        Body::Zlib(b"kept as it was".to_vec()),
    );
    let changes = diff(&a, &b);
    assert_eq!(
        outline(&changes),
        vec![
            (
                "changed",
                "child.rpf".to_string(),
                Some(Summary::Nested { changes: 3 })
            ),
            (
                "changed",
                "child.rpf/grand.rpf".to_string(),
                Some(Summary::Nested { changes: 2 })
            ),
            (
                "changed",
                "child.rpf/grand.rpf/leaf.txt".to_string(),
                lines(&["after"], &["before"])
            ),
            (
                "changed",
                "child.rpf/grand.rpf/keep.bin".to_string(),
                Some(Summary::Stored)
            ),
        ]
    );
    let text = render("a", "b", &changes);
    for line in [
        "      nested archive: 3 change(s) inside",
        "      nested archive: 2 change(s) inside",
        "  ~ child.rpf/grand.rpf/leaf.txt",
        "        + after",
        "        - before",
    ] {
        assert!(text.contains(line), "{line:?} missing from\n{text}");
    }
}

// --- install-gated: a build archive against the container writer's copies ------------------------

/// The build root holding `*.rpf`.
const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
/// A nested archive larger than this is not read whole, so the test stays in memory.
const NESTED_CAP: u64 = 16 << 20;
/// A leaf larger than this is not tried as the replaced entry.
const LEAF_CAP: u64 = 1 << 20;
/// Leaves tried per nested archive before it is passed over.
const TRIES: usize = 20;

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

/// The archive the writer writes from `src` after `edit`, in memory.
fn written(
    src: &[u8],
    keys: &GtaViKeys,
    edit: impl FnOnce(&mut Rpf9ArchiveWriter),
) -> Option<Vec<u8>> {
    let mut writer = Rpf9ArchiveWriter::new();
    edit(&mut writer);
    let mut cursor = Cursor::new(src);
    let plan = writer.plan(&mut cursor, keys).ok()?;
    let mut out = Vec::new();
    plan.write_to(&mut cursor, &mut out).ok()?;
    Some(out)
}

/// A small clear nested archive of the build read whole, one of its leaves as the diff names it, and
/// the archive with that leaf's body replaced by its own body with the last byte flipped.
fn small_archive_and_one_replaced(
    build: &Path,
    keys: &GtaViKeys,
    resolver: &NameResolver,
) -> Option<(Vec<u8>, String, Vec<u8>)> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(build)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("rpf"))
        .collect();
    paths.sort();
    for path in paths {
        let Ok(mut file) = File::open(&path) else {
            continue;
        };
        let Ok(top) = Gta6Mount::open_root(&mut file, keys) else {
            continue;
        };
        for entry in &top.archive.entries {
            let (Some(Gta6Class::NestedArchive), None, Some(start), Some(stored)) = (
                entry.kind(),
                entry.cipher,
                top.absolute_start(entry),
                entry.stored,
            ) else {
                continue;
            };
            if stored > NESTED_CAP {
                continue;
            }
            let mut bytes = vec![0u8; stored as usize];
            if read_exact_at(&mut file, start, &mut bytes).is_err() {
                continue;
            }
            let mut src = Cursor::new(&bytes[..]);
            let Ok(mut mount) = Gta6Mount::open_root(&mut src, keys) else {
                continue;
            };
            mount.archive.resolve_names(resolver);
            let leaves = mount.archive.entries.iter().filter(|e| {
                e.cipher.is_none()
                    && e.kind().is_some_and(|k| k != Gta6Class::NestedArchive)
                    && e.real.is_some_and(|r| r > 0 && r <= LEAF_CAP)
            });
            for leaf in leaves.take(TRIES) {
                let Ok(mut body) = mount.read_entry(&mut src, leaf) else {
                    continue;
                };
                let Some(last) = body.last_mut() else {
                    continue;
                };
                *last ^= 0x01;
                let edited = written(&bytes, keys, |w| {
                    w.replace(&[leaf.index], body);
                });
                if let Some(edited) = edited {
                    return Some((bytes.clone(), leaf.display_name(), edited));
                }
            }
        }
    }
    None
}

/// The writer's copy of a build archive differs in no entry, and the archive with one entry replaced
/// differs in exactly that entry. Install-gated: it needs the key snapshot and the build.
#[test]
fn a_build_archive_against_the_writers_copy_differs_only_where_an_entry_was_replaced() {
    let what = "rpf9-diff against the container writer over a build";
    let (Some(snapshot), Some(build)) = (env_path(GTA6_SNAPSHOT_ENV), env_path(BUILD_ENV)) else {
        or_test_support::skip_install(
            what,
            format_args!("set {GTA6_SNAPSHOT_ENV} and {BUILD_ENV}"),
        );
        return;
    };
    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
    let resolver = NameResolver::new();
    let Some((archive, path, edited)) = small_archive_and_one_replaced(&build, &keys, &resolver)
    else {
        or_test_support::skip_install(
            what,
            format_args!("no small nested archive in the build takes a replaced leaf"),
        );
        return;
    };

    let copy = written(&archive, &keys, |_| {}).expect("the writer copies the archive");
    let none = diff_with(&archive, &copy, &keys, &resolver);
    assert!(none.is_empty(), "the copy differs: {none:?}");

    let changes = diff_with(&archive, &edited, &keys, &resolver);
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert!(
        matches!(&changes[0], Change::Changed { path: p, .. } if *p == path),
        "{changes:?} names {path}"
    );
}
