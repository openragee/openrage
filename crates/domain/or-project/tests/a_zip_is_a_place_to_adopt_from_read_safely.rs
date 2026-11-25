//! A zip is a place to adopt from: it lists as the folder of the same files does with nothing written, hands over the bytes the folder
//! does, and stages them equal; an archive built to break out of the folder, to be two files in one, to be larger than it says or to
//! hide what it holds is refused whole, by the entry, with nothing written; and a staging folder cut short is removed.
//!
//! Every archive is written here with the crate that reads it. Two things that crate does not write, an entry marked encrypted and
//! an entry that states a false size, are made by changing the bytes it wrote, and each is first read back to state what was meant.

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use or_project::place::{is_archive_name, Limits, Place, Refusal, Wanted, STAGING_PREFIX};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

type Writer = ZipWriter<Cursor<Vec<u8>>>;

fn deflated() -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)
}

/// An archive of `files`, deflated, as bytes.
fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    zip_with(|zip| {
        for (name, bytes) in files {
            zip.start_file(*name, deflated()).unwrap();
            zip.write_all(bytes).unwrap();
        }
    })
}

fn zip_with(fill: impl FnOnce(&mut Writer)) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    fill(&mut zip);
    zip.finish().unwrap().into_inner()
}

/// The names the archive `bytes` states, as the crate reads them back.
fn stated_names(bytes: &[u8]) -> Vec<String> {
    let archive = ZipArchive::new(Cursor::new(bytes.to_vec())).unwrap();
    archive.file_names().map(str::to_string).collect()
}

/// Write `bytes` as `name` in a new scratch folder, and hand back the folder and the file.
fn on_disk(name: &str, bytes: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).unwrap();
    (dir, path)
}

fn put(base: &Path, rel: &str, bytes: &[u8]) {
    let path = base.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// Every file under `base`, by its path with `/`, with its bytes.
fn tree(base: &Path) -> Vec<(String, Vec<u8>)> {
    let mut found = Vec::new();
    let mut stack = vec![base.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path.strip_prefix(base).unwrap().to_string_lossy();
                found.push((rel.replace('\\', "/"), std::fs::read(&path).unwrap()));
            }
        }
    }
    found.sort();
    found
}

const MAPPING: &[(&str, &[u8])] = &[
    (
        "harbour/fxmanifest.lua",
        b"fx_version 'cerulean'\ngame 'gta5'\n",
    ),
    (
        "harbour/stream/harbour.ymap",
        b"a map file, as far as a place knows",
    ),
    ("harbour/stream/props/crate.ydr", b"a model"),
    ("harbour/readme.txt", b""),
];

/// The refusal of the archive `bytes`, opened within `limits`; the folder it is in holds the archive alone afterwards.
fn refused_with(bytes: &[u8], limits: &Limits) -> Refusal {
    let (dir, archive) = on_disk("in.zip", bytes);
    let refusal = Place::open(&archive, limits).expect_err("the archive is refused");
    let left: Vec<String> = tree(dir.path()).into_iter().map(|(name, _)| name).collect();
    assert_eq!(
        left,
        ["in.zip"],
        "a refusal writes nothing beside the archive"
    );
    refusal
}

fn refused(bytes: &[u8]) -> Refusal {
    refused_with(bytes, &Limits::default())
}

#[test]
fn a_zip_lists_reads_and_stages_as_the_folder_of_the_same_files_does() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("as_a_folder");
    for (name, bytes) in MAPPING {
        put(&folder, name, bytes);
    }
    let archive = dir.path().join("as_a_zip.zip");
    std::fs::write(&archive, zip_of(MAPPING)).unwrap();
    let before = tree(dir.path());

    let limits = Limits::default();
    let mut from_folder = Place::open(&folder, &limits).unwrap();
    let mut from_zip = Place::open(&archive, &limits).unwrap();
    assert!(from_zip.is_archive() && !from_folder.is_archive());
    assert_eq!(from_zip.files(), from_folder.files());
    let mut names: Vec<&str> = MAPPING.iter().map(|(name, _)| *name).collect();
    names.sort_unstable();
    let listed: Vec<&str> = from_zip.files().iter().map(|f| f.path.as_str()).collect();
    assert_eq!(listed, names);

    for (name, bytes) in MAPPING {
        assert_eq!(&from_zip.read(name).unwrap(), bytes, "{name}");
        assert_eq!(&from_folder.read(name).unwrap(), bytes, "{name}");
        let head = &bytes[..bytes.len().min(4)];
        assert_eq!(from_zip.head(name, 4).unwrap(), head, "{name}");
        assert_eq!(from_folder.head(name, 4).unwrap(), head, "{name}");
    }
    assert_eq!(
        tree(dir.path()),
        before,
        "listing and reading a place writes nothing"
    );

    let wanted: Vec<Wanted<'_>> = MAPPING
        .iter()
        .map(|(name, _)| Wanted {
            from: name,
            to: name.strip_prefix("harbour/").unwrap(),
        })
        .collect();
    let (project_a, project_b) = (dir.path().join("a"), dir.path().join("b"));
    std::fs::create_dir_all(&project_a).unwrap();
    std::fs::create_dir_all(&project_b).unwrap();
    let staged_zip = from_zip.stage(&project_a, &wanted).unwrap();
    let staged_folder = from_folder.stage(&project_b, &wanted).unwrap();
    let unpacked = tree(staged_zip.folder());
    assert_eq!(unpacked, tree(staged_folder.folder()));
    assert_eq!(unpacked.len(), MAPPING.len());
    assert_eq!(unpacked[0].0, "fxmanifest.lua");
    let name = staged_zip
        .folder()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(name.starts_with(STAGING_PREFIX), "{name}");
    assert_eq!(staged_zip.folder().parent(), Some(project_a.as_path()));

    // Where a file is staged is held to the staging folder as an entry's own name is.
    for outside in ["../out.lua", "a/../../out.lua", "C:/out.lua"] {
        let climbs = [Wanted {
            from: "harbour/readme.txt",
            to: outside,
        }];
        let refusal = from_zip.stage(&project_b, &climbs).expect_err(outside);
        assert_eq!(refusal.entry(), Some(outside), "{refusal}");
    }
    assert_eq!(
        std::fs::read_dir(&project_b).unwrap().count(),
        1,
        "only the folder staged above"
    );
    assert!(!dir.path().join("out.lua").exists());

    drop(staged_zip);
    assert_eq!(
        tree(&project_a),
        [],
        "a staging folder goes when it is dropped"
    );
    assert_eq!(std::fs::read_dir(&project_a).unwrap().count(), 0);
}

#[test]
fn an_entry_that_leaves_the_folder_is_refused_by_name_whichever_separator_it_uses() {
    for (name, code) in [
        ("../evil.lua", "climbs"),
        ("harbour/../../evil.lua", "climbs"),
        ("harbour\\..\\..\\evil.lua", "climbs"),
        ("..\\evil.lua", "climbs"),
        ("/etc/evil.lua", "absolute"),
        ("\\evil.lua", "absolute"),
        ("\\\\server\\share\\evil.lua", "absolute"),
        ("C:/evil.lua", "drive"),
        ("c:\\evil.lua", "drive"),
        ("harbour/stream/a.ymap:stream", "drive"),
        ("harbour/stream/NUL", "device"),
        ("harbour/stream/con.ymap", "device"),
        ("harbour\\COM1", "device"),
        ("harbour/stream /a.ymap", "not-a-name"),
    ] {
        let bytes = zip_of(&[
            ("harbour/fxmanifest.lua", b"fx_version 'cerulean'"),
            (name, b"print('out')"),
        ]);
        assert!(
            stated_names(&bytes).iter().any(|stated| stated == name),
            "the archive states {name:?}: {:?}",
            stated_names(&bytes)
        );
        let refusal = refused(&bytes);
        assert_eq!(refusal.code(), code, "{name}: {refusal}");
        assert_eq!(refusal.entry(), Some(name), "{refusal}");
        assert!(refusal.to_string().contains(name), "{refusal}");
    }
}

#[test]
fn an_entry_that_is_a_link_is_refused_by_name() {
    let bytes = zip_with(|zip| {
        zip.start_file("harbour/fxmanifest.lua", deflated())
            .unwrap();
        zip.add_symlink(
            "harbour/stream/keys",
            "/etc/passwd",
            SimpleFileOptions::default(),
        )
        .unwrap();
    });
    let refusal = refused(&bytes);
    assert_eq!(
        refusal,
        Refusal::Link {
            entry: "harbour/stream/keys".into()
        }
    );
}

#[test]
fn two_entries_that_differ_only_by_case_are_refused_by_both_names() {
    let files = zip_of(&[
        ("harbour/stream/Crate.ydr", b"one"),
        ("harbour/stream/crate.ydr", b"two"),
    ]);
    assert_eq!(
        refused(&files),
        Refusal::CaseTwin {
            entry: "harbour/stream/crate.ydr".into(),
            other: "harbour/stream/Crate.ydr".into(),
        }
    );
    let folders = zip_of(&[
        ("harbour/stream/a.ydr", b"one"),
        ("harbour/Stream/b.ydr", b"two"),
    ]);
    let refusal = refused(&folders);
    assert_eq!(refusal.code(), "case-twin", "{refusal}");
    assert_eq!(refusal.entry(), Some("harbour/Stream/b.ydr"));
    let both = zip_of(&[
        ("harbour/stream", b"a file"),
        ("harbour/stream/b.ydr", b"x"),
    ]);
    assert_eq!(refused(&both).code(), "file-and-folder");
}

#[test]
fn a_path_longer_than_the_destination_holds_is_refused_before_anything_is_written() {
    let leaf = format!("{}.ydr", "n".repeat(60));
    let long = format!("harbour/stream/{leaf}");
    let bytes = zip_of(&[("harbour/a.lua", b"x"), (long.as_str(), b"a model")]);
    let (dir, archive) = on_disk("in.zip", &bytes);
    let project = dir.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let wanted = [
        Wanted {
            from: "harbour/a.lua",
            to: "a.lua",
        },
        Wanted {
            from: &long,
            to: &long,
        },
    ];

    // The path as it is written is the bound: one unit less does not hold it.
    let mut roomy = Place::open(&archive, &Limits::default()).unwrap();
    assert_eq!(roomy.fits(&project, &wanted), Ok(()));
    let written = {
        let staged = roomy.stage(&project, &wanted).unwrap();
        let at = std::path::absolute(staged.path(&long)).unwrap();
        at.to_string_lossy().encode_utf16().count() as u64
    };
    let mut limits = Limits::default();
    limits.set("path", written).unwrap();
    let mut exact = Place::open(&archive, &limits).unwrap();
    assert!(
        exact.stage(&project, &wanted).is_ok(),
        "a path of {written} units fits in {written}"
    );

    limits.set("path", written - 1).unwrap();
    let mut tight = Place::open(&archive, &limits).unwrap();
    let refusal = tight
        .stage(&project, &wanted)
        .expect_err("the path does not fit");
    assert_eq!(
        refusal,
        Refusal::PathTooLong {
            entry: long.clone(),
            units: written,
            limit: written - 1
        }
    );
    assert_eq!(
        std::fs::read_dir(&project).unwrap().count(),
        0,
        "nothing is written"
    );

    let one_name = format!("harbour/{}", "n".repeat(256));
    let mut roomy = Limits::default();
    roomy.set("path", 1_000).unwrap();
    assert_eq!(
        refused_with(&zip_of(&[(one_name.as_str(), b"x")]), &roomy),
        Refusal::NameTooLong {
            entry: one_name.clone(),
            units: 256,
            limit: 255
        },
        "one name is held to what a file system takes, however long a path may be"
    );
}

#[test]
fn a_path_no_folder_could_hold_is_refused_at_the_listing_by_name() {
    let limit = Limits::default().path as usize;
    // Two names, each one a file system takes, that are a path of exactly `units` with the separator between them.
    let of = |units: usize| format!("{}/{}", "a".repeat(100), "b".repeat(units - 101));
    let (held, past) = (of(limit), of(limit + 1));
    assert_eq!(
        (held.encode_utf16().count(), past.encode_utf16().count()),
        (limit, limit + 1)
    );
    let (_dir, archive) = on_disk("in.zip", &zip_of(&[(held.as_str(), b"x")]));
    assert!(Place::open(&archive, &Limits::default()).is_ok());
    assert_eq!(
        refused(&zip_of(&[("a.lua", b"x"), (past.as_str(), b"x")])),
        Refusal::PathTooLongAlone {
            entry: past.clone(),
            units: limit as u64 + 1,
            limit: limit as u64
        }
    );

    // ⚠ Every folder on the way to a file is kept by its whole path, so listing a name of thousands of folders costs its
    // length squared: eight such names, in an archive under a megabyte, are eight seconds and gigabytes of paths unless each
    // is held to the limit first.
    let deep: Vec<String> = (0..8)
        .map(|n| format!("r{n}/{}f.lua", "a/".repeat(30_000)))
        .collect();
    let entries: Vec<(&str, &[u8])> = deep.iter().map(|name| (name.as_str(), &b"x"[..])).collect();
    let refusal = refused(&zip_of(&entries));
    assert_eq!(refusal.code(), "path-too-long", "{refusal}");
    assert_eq!(refusal.entry(), Some(deep[0].as_str()));
    let folder = "d/".repeat(200);
    let bytes = zip_with(|zip| {
        zip.add_directory(folder.as_str(), SimpleFileOptions::default())
            .unwrap();
    });
    assert_eq!(stated_names(&bytes), [folder.as_str()]);
    assert_eq!(
        refused(&bytes),
        Refusal::PathTooLongAlone {
            entry: folder.clone(),
            units: 399,
            limit: limit as u64
        },
        "a folder is a path too"
    );

    // A folder is held to the same bound.
    let dir = tempfile::tempdir().unwrap();
    let mapping = dir.path().join("mapping");
    put(&mapping, "stream/harbour_crate.ydr", b"a model");
    let mut limits = Limits::default();
    limits
        .set("path", "stream/harbour_crate.ydr".len() as u64)
        .unwrap();
    assert!(Place::open(&mapping, &limits).is_ok());
    limits
        .set("path", "stream/harbour_crate.ydr".len() as u64 - 1)
        .unwrap();
    let refusal = Place::open(&mapping, &limits).unwrap_err();
    assert_eq!(refusal.code(), "path-too-long", "{refusal}");
    assert_eq!(refusal.entry(), Some("stream/harbour_crate.ydr"));
}

#[test]
fn an_archive_past_a_limit_is_refused_at_the_first_entry_past_it() {
    let four = zip_of(&[("a", b"1"), ("b", b"2"), ("c", b"3"), ("d", b"4")]);
    let mut limits = Limits::default();
    limits.set("entries", 3).unwrap();
    assert_eq!(
        refused_with(&four, &limits),
        Refusal::TooManyEntries {
            stated: 4,
            limit: 3
        }
    );
    limits.set("entries", 4).unwrap();
    let (_dir, archive) = on_disk("in.zip", &four);
    assert_eq!(Place::open(&archive, &limits).unwrap().files().len(), 4);

    let sixty = [7u8; 60];
    let sized = zip_of(&[("a.ydr", &sixty), ("b.ydr", &sixty), ("c.ydr", &sixty)]);
    let mut limits = Limits::default();
    limits.set("bytes", 119).unwrap();
    assert_eq!(
        refused_with(&sized, &limits),
        Refusal::TooLarge {
            entry: "b.ydr".into(),
            limit: 119
        }
    );
    limits.set("bytes", 180).unwrap();
    let (_dir, archive) = on_disk("in.zip", &sized);
    assert!(Place::open(&archive, &limits).is_ok());

    // Zeros deflate to a thousandth of their size, which is what an archive built to fill a disk is made of.
    let zeros = vec![0u8; 400_000];
    let bomb = zip_of(&[("small.lua", b"print()"), ("stream/zeros.ydr", &zeros)]);
    let packed = bomb.len() as u64;
    assert!(packed * 100 < zeros.len() as u64, "{packed} bytes packed");
    let mut limits = Limits::default();
    limits.set("ratio_floor", 0).unwrap();
    assert_eq!(
        refused_with(&bomb, &limits),
        Refusal::Ratio {
            entry: "stream/zeros.ydr".into(),
            limit: packed * 100,
            ratio: 100,
            packed,
        }
    );
    // Under the floor the ratio is not judged: this archive is small whatever it unpacks to.
    let (_dir, archive) = on_disk("in.zip", &bomb);
    assert!(Place::open(&archive, &Limits::default()).is_ok());
}

#[test]
fn a_folder_past_a_limit_is_refused_as_an_archive_is() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("mapping");
    for name in ["stream/a.ydr", "stream/b.ydr", "stream/c.ydr"] {
        put(&folder, name, &[7u8; 60]);
    }
    // The folder `stream` is counted with the three files in it.
    let mut limits = Limits::default();
    limits.set("entries", 3).unwrap();
    assert_eq!(
        Place::open(&folder, &limits).unwrap_err(),
        Refusal::TooManyEntries {
            stated: 4,
            limit: 3
        }
    );
    limits.set("entries", 4).unwrap();
    assert_eq!(Place::open(&folder, &limits).unwrap().files().len(), 3);
    limits.set("bytes", 179).unwrap();
    assert_eq!(
        Place::open(&folder, &limits).unwrap_err(),
        Refusal::TooLarge {
            entry: "stream/c.ydr".into(),
            limit: 179
        }
    );
    limits.set("bytes", 180).unwrap();
    assert!(Place::open(&folder, &limits).is_ok());
}

/// The bytes of `value` as an archive stores a 32-bit number.
fn stored(value: u32) -> [u8; 4] {
    value.to_le_bytes()
}

/// `bytes` with every `from` replaced by `to`, and how many there were.
fn replace_all(bytes: &mut [u8], from: &[u8], to: &[u8]) -> usize {
    assert_eq!(from.len(), to.len());
    let mut count = 0;
    let mut at = 0;
    while at + from.len() <= bytes.len() {
        if &bytes[at..at + from.len()] == from {
            bytes[at..at + from.len()].copy_from_slice(to);
            count += 1;
            at += from.len();
        } else {
            at += 1;
        }
    }
    count
}

#[test]
fn an_encrypted_entry_is_refused_by_name() {
    let mut bytes = zip_with(|zip| {
        zip.start_file("harbour/fxmanifest.lua", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"fx_version 'cerulean'").unwrap();
        zip.start_file("harbour/stream/sealed.ydr", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"what a password would open").unwrap();
    });
    // The zip format keeps a word of flags after the two version words of a central header (one, of a local header), and
    // its lowest bit says the entry is encrypted. The name follows the fixed part of either header.
    let name = b"harbour/stream/sealed.ydr";
    let mut marked = 0;
    for (signature, flags_at, name_at) in [(b"PK\x01\x02", 8, 46), (b"PK\x03\x04", 6, 30)] {
        let headers: Vec<usize> = (0..bytes.len().saturating_sub(name_at + name.len()))
            .filter(|at| bytes[*at..].starts_with(signature))
            .filter(|at| bytes[at + name_at..].starts_with(name))
            .collect();
        for at in headers {
            bytes[at + flags_at] |= 1;
            marked += 1;
        }
    }
    assert_eq!(marked, 2, "the entry has a local and a central header");
    let mut archive = ZipArchive::new(Cursor::new(bytes.clone())).unwrap();
    assert!(
        archive.by_index_raw(1).unwrap().encrypted(),
        "the entry reads as encrypted"
    );
    assert!(!archive.by_index_raw(0).unwrap().encrypted());

    assert_eq!(
        refused(&bytes),
        Refusal::Encrypted {
            entry: "harbour/stream/sealed.ydr".into()
        }
    );
}

#[test]
fn a_nested_archive_is_listed_and_not_opened() {
    // What is inside would be refused if it were ever read as an archive: an entry that climbs out.
    let inner = zip_of(&[("../../evil.lua", b"print('out')")]);
    let bytes = zip_of(&[
        ("harbour/fxmanifest.lua", b"fx_version 'cerulean'"),
        ("harbour/more.zip", &inner),
    ]);
    let (_dir, archive) = on_disk("in.zip", &bytes);
    let place = Place::open(&archive, &Limits::default()).unwrap();
    let listed: Vec<(&str, u64)> = place
        .files()
        .iter()
        .map(|f| (f.path.as_str(), f.bytes))
        .collect();
    assert_eq!(
        listed,
        [
            ("harbour/fxmanifest.lua", 21),
            ("harbour/more.zip", inner.len() as u64)
        ],
        "the archive inside is one file, and nothing in it is listed"
    );
    assert!(is_archive_name("harbour/more.zip") && is_archive_name("x/pack.RPF"));
    assert!(!is_archive_name("harbour/fxmanifest.lua") && !is_archive_name("zip"));
}

#[test]
fn an_entry_larger_than_it_states_ends_the_unpack_and_the_staging_folder_is_removed() {
    const TRUE_SIZE: u32 = 70_001;
    const STATED: u32 = 10;
    let large = vec![b'x'; TRUE_SIZE as usize];
    let mut bytes = zip_of(&[
        ("a.lua", b"ten bytes."),
        ("b.ydr", &large),
        ("c.lua", b"never reached"),
    ]);
    // The size is stated in the local header and in the central one.
    assert_eq!(
        replace_all(&mut bytes, &stored(TRUE_SIZE), &stored(STATED)),
        2
    );
    let mut read_back = ZipArchive::new(Cursor::new(bytes.clone())).unwrap();
    assert_eq!(read_back.by_index_raw(1).unwrap().size(), u64::from(STATED));

    let (dir, archive) = on_disk("in.zip", &bytes);
    let project = dir.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    // The sizes stated are within the limit, so only counting what is written can tell.
    let mut limits = Limits::default();
    limits.set("bytes", 64).unwrap();
    let mut place = Place::open(&archive, &limits).expect("what it states is within the limits");

    assert_eq!(
        place.read("b.ydr"),
        Err(Refusal::SizeLies {
            entry: "b.ydr".into(),
            stated: u64::from(STATED)
        }),
        "a read in memory is held to the size stated"
    );
    let wanted = ["a.lua", "b.ydr", "c.lua"].map(|name| Wanted {
        from: name,
        to: name,
    });
    let refusal = place
        .stage(&project, &wanted)
        .expect_err("the entry is larger than it states");
    assert_eq!(
        refusal,
        Refusal::SizeLies {
            entry: "b.ydr".into(),
            stated: u64::from(STATED)
        }
    );
    assert_eq!(
        std::fs::read_dir(&project).unwrap().count(),
        0,
        "the staging folder that held a.lua is removed"
    );

    // The entries before the false one are sound: staged alone they are written, so the folder above was made and then removed.
    let staged = place.stage(&project, &wanted[..1]).unwrap();
    assert_eq!(std::fs::read(staged.path("a.lua")).unwrap(), b"ten bytes.");
}

#[test]
fn two_names_the_disk_makes_one_file_are_refused_at_the_unpack_and_never_written_over() {
    // One name in the two forms unicode gives an accented letter: no case differs, so only the disk can say they are one.
    let (composed, decomposed) = ("caf\u{e9}.lua", "cafe\u{301}.lua");
    let bytes = zip_of(&[(composed, b"first"), (decomposed, b"second")]);
    let (dir, archive) = on_disk("in.zip", &bytes);
    let project = dir.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let mut place = Place::open(&archive, &Limits::default()).unwrap();
    let wanted = [composed, decomposed].map(|name| Wanted {
        from: name,
        to: name,
    });
    match place.stage(&project, &wanted) {
        // A disk that keeps the two names apart holds both files.
        Ok(staged) => {
            let held = tree(staged.folder());
            assert_eq!(held.len(), 2, "{held:?}");
            assert_eq!(std::fs::read(staged.path(composed)).unwrap(), b"first");
        }
        Err(refusal) => {
            assert_eq!(
                refusal,
                Refusal::OneFile {
                    entry: decomposed.into()
                }
            );
            assert_eq!(std::fs::read_dir(&project).unwrap().count(), 0);
        }
    }
}

#[test]
fn a_hidden_name_is_left_out_once_and_nothing_under_it_is_listed() {
    // A resource cloned with git holds a folder of thousands of files that are no part of it.
    let files: &[(&str, &[u8])] = &[
        (".git/config", b"[core]"),
        (".git/objects/1a/2b3c", b"an object"),
        (".git/objects/4d/5e6f", b"another"),
        ("data/.keep", b""),
        ("stream/.DS_Store", b"what a file browser left"),
        ("stream/a.ymap", b"a map"),
    ];
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("mapping");
    for (name, bytes) in files {
        put(&folder, name, bytes);
    }
    let archive = dir.path().join("mapping.zip");
    std::fs::write(&archive, zip_of(files)).unwrap();

    for at in [&folder, &archive] {
        let place = Place::open(at, &Limits::default()).unwrap();
        let listed: Vec<&str> = place.files().iter().map(|f| f.path.as_str()).collect();
        assert_eq!(listed, ["stream/a.ymap"], "{}", at.display());
        let left: Vec<&str> = place.left().iter().map(|l| l.path.as_str()).collect();
        assert_eq!(
            left,
            [".git", "data/.keep", "stream/.DS_Store"],
            "in path order, whatever order the place was walked in: {}",
            at.display()
        );
        assert!(
            place.left().iter().all(|l| l.why.starts_with("hidden")),
            "{:?}",
            place.left()
        );
    }

    // A hidden folder of a folder is not entered, so what it holds is not counted: `.git`, the two folders and their three
    // files are six.
    let mut limits = Limits::default();
    limits.set("entries", 6).unwrap();
    assert!(Place::open(&folder, &limits).is_ok());
    // ⚠ An archive states every entry, and one that is not safe refuses the archive wherever it is.
    limits.set("entries", files.len() as u64 - 1).unwrap();
    assert_eq!(
        Place::open(&archive, &limits).unwrap_err(),
        Refusal::TooManyEntries {
            stated: files.len() as u64,
            limit: files.len() as u64 - 1
        }
    );
    let refusal = refused(&zip_of(&[
        ("stream/a.ymap", b"a map"),
        (".git/../../evil.lua", b"print('out')"),
    ]));
    assert_eq!(refusal.code(), "climbs", "{refusal}");
}

#[test]
fn a_file_that_is_no_archive_is_refused_as_a_place() {
    let (_dir, path) = on_disk("notes.zip", b"this is not a zip archive at all");
    let refusal = Place::open(&path, &Limits::default()).unwrap_err();
    assert_eq!(refusal.code(), "not-a-place", "{refusal}");
    assert_eq!(refusal.entry(), None);
}

#[cfg(unix)]
#[test]
fn a_file_of_a_folder_whose_name_holds_the_other_separator_is_refused_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("mapping");
    put(&folder, "stream/a.ymap", b"a map");
    // One file here, and a file `b.ymap` in a folder `a` on Windows: the project made of it would not be the same on both.
    put(&folder, "stream/a\\b.ymap", b"a map");
    assert_eq!(
        Place::open(&folder, &Limits::default()).unwrap_err(),
        Refusal::NotAName {
            entry: "stream/a\\b.ymap".into()
        }
    );
}

#[cfg(unix)]
#[test]
fn a_link_and_what_is_no_file_in_a_folder_are_left_out_by_name_and_not_read() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("mapping");
    put(&folder, "stream/a.ymap", b"a map");
    put(dir.path(), "outside/secret.txt", b"not part of the mapping");
    std::os::unix::fs::symlink(dir.path().join("outside"), folder.join("stream/linked")).unwrap();
    // A socket is neither a file nor a folder, and reading one as a file never ends.
    let _socket = std::os::unix::net::UnixListener::bind(folder.join("stream/socket")).unwrap();

    let place = Place::open(&folder, &Limits::default()).unwrap();
    let listed: Vec<&str> = place.files().iter().map(|f| f.path.as_str()).collect();
    assert_eq!(listed, ["stream/a.ymap"]);
    let left: Vec<(&str, bool)> = (place.left().iter())
        .map(|left| (left.path.as_str(), left.why.starts_with("a link")))
        .collect();
    assert_eq!(left, [("stream/linked", true), ("stream/socket", false)]);
}
