//! `dlc create-gta6`: a GTA VI pack generated from a folder, its setup, content definition and one
//! change set registering the folder's files, packed with them into a new RPF9.
//!
//! ⚠ Structural only: the game verifies a signature over an archive's table when it mounts it, and
//! only its maker can sign one, so the game refuses the archive this writes.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Cursor, Write};
use std::path::{Path, PathBuf};

use or_io::container::rpf::gta6_container_write::{build, Rpf9IndexKey, Rpf9NewBody};
use or_io::container::rpf::{Gta6Mount, GtaViKeys, RPF9_MAGIC};
use or_io::gta6::names::{vocabulary_file, Vocabulary};
use or_title::dlc::{
    ContentChangeSetDef, ContentChangeSetGroup, ContentDef, IncludedChangeset, RegisteredFile,
    SetupContentType, SetupData,
};

/// The setup's entry: GTA V's name, as no GTA VI document states the setup's own.
const SETUP: &str = "setup2.xml";
/// The content definition's entry, the `datFile` every measured GTA VI setup states.
const CONTENT: &str = "content.xml";
/// The setup group the change set is applied in, the one the build's small packs use.
const GROUP: &str = "GROUP_DEFAULT";
/// The include flags every measured include states.
const INCLUDE_FLAGS: &str = "kDefault";
/// The `fileType` of a registered archive.
const ARCHIVE: &str = "RPF_FILE";
/// The documents a pack adds beside its files: a setup, a content definition and a change set.
const DOCUMENTS: usize = 3;

const USAGE: &str = "\
usage:
  openrage-cli dlc create-gta6 <src_dir> --name <device> --key-from <archive.rpf> --out <new.rpf>
                               [--file-type <ext>=<TYPE>]... [--keys <snapshot>] [--exe <game.exe>]
  Every file under <src_dir> is registered under $(root)/ at its path there; an .rpf is registered
  as an archive, and any other extension needs its fileType named with --file-type.";

const REFUSED: &str = "The game refuses this archive: it verifies a signature over the table when \
it mounts an archive, and only the game's maker can sign one. It is a structural build.";

const UNKNOWN: &str =
    "Not stated by any document of the build, so written as shown or not written: \
the setup's own entry name (written as setup2.xml), what $(root) expands to, and how the game is \
told a pack exists (no pack list is written).";

/// `dlc create-gta6`: parse the arguments, build the pack into a new file, keep its names.
pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let (mut src, mut out, mut name, mut key_from, mut keys_path, mut exe) =
        (None, None, None, None, None, None);
    let mut file_types = BTreeMap::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "-h" | "--help" | "help" => {
                eprintln!("{USAGE}");
                return Ok(());
            }
            "--out" => out = Some(PathBuf::from(value()?)),
            "--name" => name = Some(value()?),
            "--key-from" => key_from = Some(PathBuf::from(value()?)),
            "--keys" => keys_path = Some(PathBuf::from(value()?)),
            "--exe" => exe = Some(PathBuf::from(value()?)),
            "--file-type" => {
                let (ext, named) = file_type_arg(&value()?)?;
                file_types.insert(ext, named);
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown option {other}\n{USAGE}"))
            }
            other if src.is_none() => src = Some(PathBuf::from(other)),
            other => return Err(format!("unexpected argument {other:?}\n{USAGE}")),
        }
    }
    let src: PathBuf =
        src.ok_or_else(|| format!("give the folder of the pack's files\n{USAGE}"))?;
    let out: PathBuf = out.ok_or("--out names the new archive to write")?;
    let device: String = name.ok_or("--name gives the pack's device name")?;
    let like: PathBuf = key_from.ok_or(
        "--key-from names an archive of the same build, whose table key the new one takes",
    )?;
    if !src.is_dir() {
        return Err(format!("{} is not a folder", src.display()));
    }

    let keys = crate::gta6_keys::load(keys_path.as_deref(), exe.as_deref())?;
    let mut source = File::open(&like).map_err(|e| format!("{}: {e}", like.display()))?;
    let mount =
        Gta6Mount::open_root(&mut source, &keys).map_err(|e| format!("{}: {e}", like.display()))?;
    let key = Rpf9IndexKey::of(&mount.archive.toc).map_err(|e| e.to_string())?;

    // `create_new` refuses an existing file, so the cleanup below only deletes a file made here.
    let created = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&out)
        .map_err(|e| format!("{}: {e}", out.display()))?;
    let mut sink = BufWriter::new(created);
    let written = write_pack(&src, &device, &file_types, &key, &keys, &mut sink)
        .and_then(|built| sink.flush().map(|()| built).map_err(|e| e.to_string()));
    let (names, registered, bytes) = match written {
        Ok(built) => built,
        Err(e) => {
            drop(sink);
            let _ = std::fs::remove_file(&out);
            return Err(format!("{}: {e}", out.display()));
        }
    };

    println!(
        "{}: a GTA VI pack of {registered} registered file(s), its setup, content definition and \
         change set, {bytes} bytes written.",
        out.display()
    );
    println!("{REFUSED}");
    println!("{UNKNOWN}");
    if let Some(config) = or_title::config::config_dir() {
        match keep_names(&vocabulary_file(&config), &names) {
            Ok(new) => println!(
                "{new} new name(s) kept in your vocabulary, so the pack's entries are shown by name."
            ),
            Err(e) => println!("The pack's names were not kept in your vocabulary: {e}"),
        }
    }
    Ok(())
}

/// One `--file-type` value, `<extension>=<TYPE>`, as the extension in lower case and the type.
fn file_type_arg(value: &str) -> Result<(String, String), String> {
    match value.split_once('=') {
        Some((extension, file_type)) if !file_type.trim().is_empty() => Ok((
            extension
                .trim()
                .trim_start_matches('.')
                .to_ascii_lowercase(),
            file_type.trim().to_owned(),
        )),
        _ => Err(format!("--file-type {value:?} is not <extension>=<TYPE>")),
    }
}

fn extension(path: &str) -> String {
    Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// The `fileType` `path` is registered under: the one `given` names for its extension, else an archive's for an `.rpf`.
fn file_type(path: &str, given: &BTreeMap<String, String>) -> Result<String, String> {
    let extension = extension(path);
    match given.get(&extension) {
        Some(file_type) => Ok(file_type.clone()),
        None if extension == "rpf" => Ok(ARCHIVE.to_owned()),
        None => Err(format!(
            "{path}: only an archive's fileType is known; name this one's with \
             --file-type {extension}=<TYPE>"
        )),
    }
}

/// Refuse a device name that is not letters, digits and underscores, as it also names the change set's entry.
fn check_device(device: &str) -> Result<(), String> {
    let plain = (device.chars()).all(|c| c.is_ascii_alphanumeric() || c == '_');
    if device.is_empty() || !plain {
        return Err(format!(
            "--name {device:?}: a device name is letters, digits and underscores, as it also names \
             the pack's change set"
        ));
    }
    Ok(())
}

/// The setup, content definition and change set of the pack `device` registering `files`, each `(path in the pack, fileType)`, by the entry each is written at.
// Laid out as the build's small packs lay theirs out: the setup names the content definition as its
// `datFile` and a group of it names the change set, which the content definition includes by path.
fn documents(device: &str, files: &[(String, String)]) -> Vec<(String, String)> {
    let set_path = format!("changesets/ccs_{device}.meta");
    let set = ContentChangeSetDef {
        id: device.to_owned(),
        file_list: (files.iter())
            .map(|(path, file_type)| RegisteredFile {
                filename: format!("$(root)/{path}"),
                file_type: file_type.clone(),
                ..RegisteredFile::default()
            })
            .collect(),
        ..ContentChangeSetDef::default()
    };
    let content = ContentDef {
        included_changesets: vec![IncludedChangeset {
            load_path: format!("$(root)/{set_path}"),
            changeset_include_flags: vec![INCLUDE_FLAGS.to_owned()],
            ..IncludedChangeset::default()
        }],
        ..ContentDef::default()
    };
    let setup = SetupData {
        name_hash: device.to_owned(),
        device_name: device.to_owned(),
        dat_file: CONTENT.to_owned(),
        content_type: SetupContentType::LevelPack,
        order: 1,
        content_change_set_groups: vec![ContentChangeSetGroup {
            name_hash: GROUP.to_owned(),
            content_change_sets: vec![device.into()],
        }],
        platform_data_under_root: Some(false),
        ..SetupData::default()
    };
    vec![
        (SETUP.to_owned(), setup.to_xml()),
        (CONTENT.to_owned(), content.to_xml()),
        (set_path, set.to_xml()),
    ]
}

/// Write the pack `device` of every file under `src` to `out`, its table enciphered under `key`: its entries' names, how many files it registers, and the bytes written.
fn write_pack(
    src: &Path,
    device: &str,
    file_types: &BTreeMap<String, String>,
    key: &Rpf9IndexKey,
    keys: &GtaViKeys,
    out: &mut impl Write,
) -> Result<(Vec<String>, usize, u64), String> {
    check_device(device)?;
    let spec = crate::dlc::dlc_pack_of_folder(src, device)?;
    if spec.files.is_empty() {
        return Err(format!(
            "{} holds no file for the pack to register",
            src.display()
        ));
    }
    let mut registered = Vec::with_capacity(spec.files.len());
    let mut entries = Vec::with_capacity(spec.files.len() + DOCUMENTS);
    for file in &spec.files {
        let path = &file.archive_path;
        let file_type = file_type(path, file_types)?;
        let bytes =
            std::fs::read(&file.source).map_err(|e| format!("{}: {e}", file.source.display()))?;
        let body = if bytes.starts_with(&RPF9_MAGIC.to_le_bytes()) {
            Rpf9NewBody::Archive(bytes)
        } else if extension(path) == "rpf" {
            return Err(format!("{path} is not a GTA VI archive (RPF9)"));
        } else {
            Rpf9NewBody::Chunked(bytes)
        };
        registered.push((path.clone(), file_type));
        entries.push((path.clone(), body));
    }
    for (path, text) in documents(device, &registered) {
        entries.push((path, Rpf9NewBody::Chunked(text.into_bytes())));
    }
    let names = entries.iter().map(|(name, _)| name.clone()).collect();
    let plan = build(&entries, key, keys).map_err(|e| e.to_string())?;
    let written = plan
        .write_to(&mut Cursor::new(Vec::<u8>::new()), out)
        .map_err(|e| e.to_string())?;
    Ok((names, registered.len(), written))
}

/// Add `names` to the vocabulary kept at `file`, refusing one that does not read rather than replacing it; how many were new.
fn keep_names(file: &Path, names: &[String]) -> std::io::Result<usize> {
    let mut vocabulary = match File::open(file) {
        Ok(kept) => Vocabulary::read(BufReader::new(kept))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vocabulary::default(),
        Err(e) => return Err(e),
    };
    let new = (names.iter())
        .filter(|name| vocabulary.add(name, None))
        .count();
    vocabulary.save(file)?;
    Ok(new)
}

// Shared with the `rpf9` tests, which use parts of it this file does not.
#[cfg(test)]
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "rpf9_fixture.rs"]
mod fixture;

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::sync::Arc;

    use or_io::container::rpf::{Gta6Context, NameResolver, RpfArchive, GTA6_SNAPSHOT_ENV};
    use or_title::gta6_mount::{Document, Registry};

    use super::fixture::{self, Body};
    use super::*;

    const DEVICE: &str = "my_pack";
    const CHILD_ENV: &str = "OPENRAGE_GTA6_DLC_REOPEN";
    const CHILD_TEST: &str = "gta6_dlc::tests::reopen_a_built_pack_for_a_parent";
    const SEPARATOR: char = '\u{1f}';
    const MARKER: &str = "the built pack reopened through the archive reader";

    /// The index key of the fixture's archives.
    fn key() -> Rpf9IndexKey {
        let like = fixture::archive(&[("a.txt", Body::Raw(b"a".to_vec()))]);
        let mount = Gta6Mount::open_root(&mut Cursor::new(&like[..]), &fixture::keys())
            .expect("the fixture opens");
        Rpf9IndexKey::of(&mount.archive.toc).expect("its index key")
    }

    /// A plain file's bytes; they shrink as zlib, as the writer needs a chunked body to.
    fn notes() -> Vec<u8> {
        b"notes ".repeat(64)
    }

    fn types() -> BTreeMap<String, String> {
        BTreeMap::from([("meta".to_owned(), "TEST_FILE".to_owned())])
    }

    /// A folder of a nested archive and a plain file, as a pack's files.
    fn source(dir: &Path) -> PathBuf {
        let src = dir.join("src");
        std::fs::create_dir_all(src.join("maps")).expect("a folder");
        std::fs::create_dir_all(src.join("data")).expect("a folder");
        let nested = fixture::archive(&[("inner.txt", Body::Raw(b"inner".to_vec()))]);
        std::fs::write(src.join("maps/inner.rpf"), nested).expect("written");
        std::fs::write(src.join("data/notes.meta"), notes()).expect("written");
        src
    }

    #[test]
    fn the_documents_read_back_typed_and_tie_together_as_the_mount_reads_a_pack() {
        let files = vec![
            ("data/notes.meta".to_owned(), "TEST_FILE".to_owned()),
            ("maps/inner.rpf".to_owned(), ARCHIVE.to_owned()),
        ];
        let docs = documents(DEVICE, &files);
        let paths: Vec<&str> = docs.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, [SETUP, CONTENT, "changesets/ccs_my_pack.meta"]);
        for (path, text) in &docs {
            crate::dlc::inspect_document(text.as_bytes())
                .unwrap_or_else(|e| panic!("dlc inspect reads {path}: {e}"));
        }
        let read: Vec<Document> = (docs.iter())
            .map(|(p, t)| Document::parse(t.as_bytes()).unwrap_or_else(|| panic!("{p} is typed")))
            .collect();
        let [Document::Setup(setup), Document::Content(content), Document::ChangeSet(set)] =
            &read[..]
        else {
            panic!("a setup, a content definition and a change set: {read:?}");
        };
        assert_eq!(setup.device_name, DEVICE);
        assert_eq!(setup.dat_file, CONTENT);
        assert_eq!(setup.content_type, SetupContentType::LevelPack);
        assert_eq!(
            content.included_changesets[0].load_path,
            "$(root)/changesets/ccs_my_pack.meta"
        );
        assert_eq!(set.id, DEVICE);

        let registry = Registry::of(vec![(PathBuf::from("pack.rpf"), read.clone())]);
        let [pack] = &registry.packs[..] else {
            panic!("one pack: {registry:?}");
        };
        assert!(pack.unread.is_empty(), "every named set is found: {pack:?}");
        let [registered] = &pack.sets[..] else {
            panic!("one change set: {pack:?}");
        };
        assert_eq!(registered.groups, [GROUP]);
        let filenames: Vec<&str> = (registered.files.iter())
            .map(|f| f.filename.as_str())
            .collect();
        assert_eq!(
            filenames,
            ["$(root)/data/notes.meta", "$(root)/maps/inner.rpf"]
        );
        assert_eq!(registered.files[1].file_type, ARCHIVE);
    }

    #[test]
    fn what_the_generator_cannot_write_is_refused_by_name() {
        let dir = tempfile::tempdir().expect("a folder");
        let src = dir.path().join("src");
        std::fs::create_dir_all(src.join("maps")).expect("a folder");
        std::fs::create_dir_all(src.join("data")).expect("a folder");
        // The walk is sorted, so the untyped file is met before the false archive.
        std::fs::write(src.join("data/notes.meta"), b"notes").expect("written");
        std::fs::write(src.join("maps/fake.rpf"), b"not an archive").expect("written");
        let refused = |src: &Path, device: &str, types: &BTreeMap<String, String>| {
            write_pack(
                src,
                device,
                types,
                &key(),
                &fixture::keys(),
                &mut Vec::new(),
            )
            .expect_err("refused")
        };
        let none = BTreeMap::new();
        assert!(refused(&src, "my pack", &none).contains("--name"));
        assert!(refused(&src, "", &none).contains("--name"));
        let untyped = refused(&src, DEVICE, &none);
        assert!(untyped.contains("--file-type meta="), "{untyped}");
        assert!(refused(&src, DEVICE, &types()).contains("maps/fake.rpf"));
        let empty = dir.path().join("empty");
        std::fs::create_dir_all(&empty).expect("a folder");
        assert!(refused(&empty, DEVICE, &none).contains("no file"));
        // A body too small to shrink under zlib is stored as it is, which its chunk table reads back.
        let tiny = dir.path().join("tiny");
        std::fs::create_dir_all(tiny.join("data")).expect("a folder");
        std::fs::write(tiny.join("data/x.meta"), b"abcde").expect("written");
        let keys = fixture::keys();
        let written = write_pack(&tiny, DEVICE, &types(), &key(), &keys, &mut Vec::new());
        assert!(written.is_ok(), "{:?}", written.err());

        assert_eq!(
            file_type_arg(".META= X_FILE"),
            Ok(("meta".into(), "X_FILE".into()))
        );
        assert!(file_type_arg("meta").is_err());
        assert!(file_type_arg("meta=").is_err());
    }

    #[test]
    fn a_built_pack_reopens_through_the_archive_reader_with_each_document_typed() {
        let dir = tempfile::tempdir().expect("a folder");
        let src = source(dir.path());
        let mut built = Vec::new();
        let (names, registered, written) =
            write_pack(&src, DEVICE, &types(), &key(), &fixture::keys(), &mut built)
                .expect("built");
        assert_eq!(registered, 2);
        assert_eq!(names.len(), registered + DOCUMENTS);
        assert_eq!(written, built.len() as u64);
        let archive = dir.path().join("pack.rpf");
        std::fs::write(&archive, &built).expect("written");
        let vocabulary = dir.path().join("names").join("vocabulary.txt");
        assert_eq!(keep_names(&vocabulary, &names).expect("kept"), names.len());
        assert_eq!(keep_names(&vocabulary, &names).expect("kept"), 0);

        // The archive reader takes its keys from the process, so it is opened in a child.
        let given = format!("{}{SEPARATOR}{}", archive.display(), vocabulary.display());
        let output = Command::new(std::env::current_exe().expect("this test binary"))
            .args([CHILD_TEST, "--exact", "--nocapture", "--test-threads=1"])
            .env(CHILD_ENV, given)
            .output()
            .expect("this test binary runs as a child");
        let printed = String::from_utf8_lossy(&output.stdout);
        let said = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{printed}\n{said}");
        assert!(printed.contains(MARKER), "the child ran: {printed}\n{said}");
    }

    /// Not a test of its own: the reopen a parent's child process runs.
    #[test]
    fn reopen_a_built_pack_for_a_parent() {
        let Some(given) = std::env::var_os(CHILD_ENV) else {
            return;
        };
        let given = given.to_string_lossy().into_owned();
        let (archive, vocabulary) = given.split_once(SEPARATOR).expect("two paths");
        // The keys and names ride in the provider the open is handed, in place of the deleted global.
        let crypto: Arc<dyn or_io::container::RpfCryptoProvider> = Arc::new(Gta6Context {
            keys: Arc::new(fixture::keys()),
            names: Arc::new(NameResolver::from_vocabulary(Vocabulary::load(Path::new(
                vocabulary,
            )))),
        });
        let archive =
            Arc::new(RpfArchive::open_with(archive, crypto).expect("the built pack opens"));
        let read = |path: &str| {
            (archive.read(path)).unwrap_or_else(|e| panic!("{path} is in the tree and reads: {e}"))
        };
        let setup = SetupData::parse(&read(SETUP)).expect("the setup reads typed");
        assert_eq!(setup.device_name, DEVICE);
        let content = ContentDef::parse(&read(&setup.dat_file)).expect("the content reads typed");
        let load = &content.included_changesets[0].load_path;
        let set_path = load
            .strip_prefix("$(root)/")
            .expect("under the pack's root");
        let set = ContentChangeSetDef::parse(&read(set_path)).expect("the change set reads typed");
        assert_eq!(set.id, DEVICE);
        for file in &set.file_list {
            let path = file.filename.strip_prefix("$(root)/").expect("in the pack");
            assert!(
                archive.find(path).is_some(),
                "{path} is registered and held"
            );
        }
        assert_eq!(read("data/notes.meta"), notes());
        let nested = archive.find("maps/inner.rpf").expect("the nested archive");
        let inner = archive
            .open_nested(nested)
            .expect("a nested archive opens as a folder");
        assert_eq!(inner.file_count(), 1);
        println!("{MARKER}");
    }

    #[test]
    fn a_pack_built_with_a_key_of_the_build_reopens_with_its_documents_typed() {
        let present = |var: &str| {
            std::env::var_os(var)
                .map(PathBuf::from)
                .filter(|p| p.exists())
        };
        let (Some(snapshot), Some(build_root)) =
            (present(GTA6_SNAPSHOT_ENV), present("OPENRAGE_GTA6_BUILD"))
        else {
            or_test_support::skip_install(
                "a GTA VI pack built with a key of the build",
                format_args!("set {GTA6_SNAPSHOT_ENV} and OPENRAGE_GTA6_BUILD"),
            );
            return;
        };
        let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
        // The smallest archive at the root, whose table is read for its key alone.
        let like = (std::fs::read_dir(&build_root).expect("the build root lists"))
            .flatten()
            .map(|e| e.path())
            .filter(|p| extension(&p.to_string_lossy()) == "rpf")
            .min_by_key(|p| std::fs::metadata(p).map_or(u64::MAX, |m| m.len()))
            .expect("an archive at the build root");
        let mut file = File::open(&like).expect("the archive opens");
        let key = Rpf9IndexKey::of(
            &Gta6Mount::open_root(&mut file, &keys)
                .expect("its table deciphers")
                .archive
                .toc,
        )
        .expect("its index key");

        let dir = tempfile::tempdir().expect("a folder");
        let src = dir.path().join("src");
        std::fs::create_dir_all(src.join("data")).expect("a folder");
        std::fs::write(src.join("data/notes.meta"), notes()).expect("written");
        let mut built = Vec::new();
        let (names, _, _) =
            write_pack(&src, DEVICE, &types(), &key, &keys, &mut built).expect("built");

        let mut mount =
            Gta6Mount::open_root(&mut Cursor::new(&built[..]), &keys).expect("the pack reopens");
        let mut resolver = NameResolver::new();
        for name in &names {
            resolver.insert(or_io::gta6::hash::name_hash64(name.as_bytes()), name);
        }
        mount.archive.resolve_names(&resolver);
        let mut documents = Vec::new();
        for entry in &mount.archive.entries {
            let name = entry.name.clone().expect("every entry is named");
            let body = (mount.read_entry_decrypted(Cursor::new(&built[..]), entry, &keys))
                .unwrap_or_else(|e| panic!("{name} reads: {e}"));
            if name == "data/notes.meta" {
                assert_eq!(body, notes());
            } else {
                documents.push(Document::parse(&body).unwrap_or_else(|| panic!("{name} is typed")));
            }
        }
        assert_eq!(documents.len(), DOCUMENTS);
        let registry = Registry::of(vec![(PathBuf::from("pack.rpf"), documents)]);
        let [pack] = &registry.packs[..] else {
            panic!("one pack: {registry:?}");
        };
        assert!(pack.unread.is_empty(), "{pack:?}");
        assert_eq!(pack.sets.len(), 1);
    }
}
