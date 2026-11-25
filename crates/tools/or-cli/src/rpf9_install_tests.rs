//! `rpf9 tree-install`, `rpf9 pack` and `rpf9 extract --kind`/`--dry-run` on synthetic archives, and
//! the install tree over a real build where one is given.

use super::super::{extract_entries, find, Pick, Scope};
use super::*;
use fixture::Body;
use or_io::gta6::hash::name_hash64;

#[allow(clippy::duplicate_mod)]
#[path = "rpf9_fixture.rs"]
mod fixture;

fn resolver_of(names: &[&str]) -> NameResolver {
    let mut resolver = NameResolver::new();
    for name in names {
        resolver.insert(name_hash64(name.as_bytes()), *name);
    }
    resolver
}

const NAMES: [&str; 6] = [
    "common/shared.dat",
    "only/root.dat",
    "child.rpf",
    "inner/a.pso",
    "top.ydr",
    "doc.pso",
];

fn child() -> Vec<u8> {
    fixture::archive(&[("inner/a.pso", Body::Zlib(fixture::pso("zq_alpha")))])
}

/// An archive at the root named to sort after `pack/` by path, and a pack archive naming one of
/// its paths again and holding a nested archive.
fn install() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temp dir");
    let root = fixture::archive(&[
        ("common/shared.dat", Body::Raw(vec![1; 16])),
        ("only/root.dat", Body::Raw(vec![2; 8])),
    ]);
    std::fs::write(dir.path().join("z.rpf"), root).expect("write the root archive");
    let pack = fixture::archive(&[
        ("common/shared.dat", Body::Raw(vec![3; 32])),
        ("child.rpf", Body::Nested(child())),
    ]);
    let folder = dir.path().join("pack").join("p1");
    std::fs::create_dir_all(&folder).expect("make the pack folder");
    std::fs::write(folder.join("a.rpf"), pack).expect("write the pack archive");
    dir
}

#[test]
fn a_path_two_archives_name_is_listed_once_from_the_later_pack() {
    let dir = install();
    let root = dir.path();
    let tree = do_tree_install(root, &fixture::keys(), &resolver_of(&NAMES)).expect("the tree");
    let (at_root, pack) = (
        root.join("z.rpf"),
        root.join("pack").join("p1").join("a.rpf"),
    );
    assert_eq!(tree.archives, vec![at_root.clone(), pack.clone()]);
    assert_eq!(tree.refused, 0);

    let given: Vec<(&str, &Path, Option<u64>, usize)> = (tree.paths.iter())
        .map(|(path, g)| {
            let archive = tree.archives[g.archive].as_path();
            (path.as_str(), archive, g.size, g.shadowed)
        })
        .collect();
    let pso = fixture::pso("zq_alpha").len() as u64;
    assert_eq!(
        given,
        vec![
            ("child.rpf/inner/a.pso", pack.as_path(), Some(pso), 0),
            ("common/shared.dat", pack.as_path(), Some(32), 1),
            ("only/root.dat", at_root.as_path(), Some(8), 0),
        ]
    );

    let lines = tree_lines(root, &tree);
    let shared = (lines.iter())
        .find(|l| l.starts_with("common/shared.dat\t"))
        .expect("the shared path is listed");
    let from = Path::new("pack").join("p1").join("a.rpf");
    assert_eq!(
        shared,
        &format!(
            "common/shared.dat\t{}\tmedia\t32\tover 1 earlier",
            from.display()
        )
    );
}

#[test]
fn the_root_archives_mount_before_the_packs() {
    let root = Path::new("build");
    let order = mount_order(
        root,
        vec![
            root.join("pack").join("b").join("b.rpf"),
            root.join("z.rpf"),
            root.join("pack").join("a").join("a.rpf"),
            root.join("a.rpf"),
        ],
    );
    assert_eq!(
        order,
        vec![
            root.join("a.rpf"),
            root.join("z.rpf"),
            root.join("pack").join("a").join("a.rpf"),
            root.join("pack").join("b").join("b.rpf"),
        ]
    );
}

#[test]
fn a_pack_is_printed_whole_with_its_nested_rows_indented() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("parent.rpf");
    let archive = fixture::archive(&[
        ("child.rpf", Body::Nested(child())),
        (
            "top.ydr",
            Body::Chunked(vec![(vec![7; 40], true), (vec![8; 24], false)]),
        ),
    ]);
    std::fs::write(&path, archive).expect("write the archive");
    let lines = pack_lines(&path, &fixture::keys(), &resolver_of(&NAMES)).expect("the pack");

    assert!(lines[0].starts_with("parent.rpf: 2 entries"), "{lines:?}");
    assert_eq!(
        lines.len(),
        5,
        "a count line, two rows, the nested count and its row: {lines:?}"
    );
    assert!(
        lines[1].starts_with("       0  child.rpf  archive  stored "),
        "{}",
        lines[1]
    );
    assert!(
        lines[1].ends_with("  clear  row24 class 0x00"),
        "{}",
        lines[1]
    );
    assert!(
        lines[2].starts_with("    child.rpf: 1 entries"),
        "{}",
        lines[2]
    );
    assert!(
        lines[3].starts_with("         0  inner/a.pso  media  "),
        "{}",
        lines[3]
    );
    assert!(
        lines[4].starts_with("       1  top.ydr  media  stored "),
        "{}",
        lines[4]
    );
    assert!(
        lines[4].ends_with("  real 64  clear  row24 class 0xff, chunk table mode 3 of 2 chunk(s)"),
        "{}",
        lines[4]
    );
}

fn parent() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("parent.rpf");
    let archive = fixture::archive(&[
        ("child.rpf", Body::Nested(child())),
        ("top.ydr", Body::Chunked(vec![(vec![7; 24], false)])),
        ("doc.pso", Body::Zlib(fixture::pso("zq_doc"))),
    ]);
    std::fs::write(&path, archive).expect("write the archive");
    (dir, path)
}

const EVERY: Pick<'static> = Pick {
    entry: None,
    filter: None,
    decrypt: false,
    strings: false,
};

#[test]
fn a_dry_run_counts_what_it_would_write_and_writes_nothing() {
    let (dir, path) = parent();
    let dest = dir.path().join("out");
    let scope = Scope {
        kind: None,
        dry_run: true,
    };
    let (keys, names) = (fixture::keys(), resolver_of(&NAMES));
    let n = extract_entries(&path, &dest, &[], &EVERY, &scope, &keys, &names).expect("dry run");
    assert_eq!(n, 3);
    assert!(!dest.exists(), "a dry run makes no folder");

    let written = extract_entries(&path, &dest, &[], &EVERY, &Scope::default(), &keys, &names)
        .expect("the extract");
    assert_eq!(written, n, "the dry run counts what the extract writes");
}

#[test]
fn a_kind_keeps_only_the_entries_of_that_kind() {
    let (dir, path) = parent();
    let (keys, names) = (fixture::keys(), resolver_of(&NAMES));
    let extract = |kind: &str, out: &str| {
        let scope = Scope {
            kind: Some(find::parse_kind(kind).expect("a kind")),
            dry_run: false,
        };
        let dest = dir.path().join(out);
        let n = extract_entries(&path, &dest, &[], &EVERY, &scope, &keys, &names);
        let mut files: Vec<String> = (std::fs::read_dir(&dest).into_iter().flatten().flatten())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        files.sort();
        (n, files)
    };
    assert_eq!(
        extract("archive", "a"),
        (Ok(1), vec!["child.rpf".to_string()])
    );
    assert_eq!(
        extract("media", "m"),
        (Ok(2), vec!["doc.pso".to_string(), "top.ydr".to_string()])
    );
    let (none, files) = extract("drawable", "d");
    assert!(none.is_err() && files.is_empty(), "no row is a drawable");
}

const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

/// Over a real build, every archive is in the tree, the root's mount before the packs, and every
/// path is given by one of them. Install-gated: it needs the key snapshot and the build.
#[test]
fn the_install_tree_of_the_build_merges_every_archive_in_mount_order() {
    let what = "rpf9 tree-install over a build";
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
    let tree = do_tree_install(&build, &keys, &NameResolver::new()).expect("the tree");
    assert_eq!(tree.archives.len(), build_archives(&build).len());
    let at_root = |p: &PathBuf| p.parent() == Some(build.as_path());
    let packs = tree
        .archives
        .iter()
        .position(|p| !at_root(p))
        .unwrap_or(tree.archives.len());
    assert!(tree.archives[..packs].iter().all(at_root));
    assert!(!tree.archives[packs..].iter().any(at_root));
    assert!(!tree.paths.is_empty());
    assert!(tree.paths.values().all(|g| g.archive < tree.archives.len()));
}
