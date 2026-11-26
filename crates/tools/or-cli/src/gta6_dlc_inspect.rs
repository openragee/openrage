//! `dlc inspect` of a GTA VI pack: the setup, content definitions and change sets the plain files of
//! an RPF9 archive's own table hold, each known by its root element, printed typed.

use std::fs::File;
use std::path::{Path, PathBuf};

use or_io::container::rpf::gta6_chunk_table::entry_chunks;
use or_io::container::rpf::{Gta6Class, Gta6Mount, GtaViKeys};
use or_title::gta6_mount::{Document, Registry};

/// What a registration document may inflate to; a longer plain file is passed over.
const BODY_LIMIT: u64 = 16 << 20;

/// The registration documents of one archive, each with its bytes, in the table's order.
#[derive(Debug)]
struct Inspected {
    documents: Vec<(Vec<u8>, Document)>,
    /// The plain files read for a document, and how many of them did not read.
    plain: usize,
    not_read: usize,
}

/// `dlc inspect` of the GTA VI archive at `path`, `snapshot` being `--keys` and `rest` the arguments after the path.
pub(crate) fn run(path: &Path, snapshot: Option<&Path>, rest: &[String]) -> Result<(), String> {
    let mut exe = None;
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--exe" => exe = Some(PathBuf::from(it.next().ok_or("--exe needs a path")?)),
            other => return Err(format!("unknown argument {other:?} for a GTA VI pack")),
        }
    }
    let keys = crate::gta6_keys::load(snapshot, exe.as_deref())?;
    let inspected = inspect(path, &keys)?;
    print(path, &inspected)
}

/// Read every plain file of the archive at `path` that may be a registration document; an archive holding no setup is refused.
fn inspect(path: &Path, keys: &GtaViKeys) -> Result<Inspected, String> {
    let at = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let mut file = File::open(path).map_err(|e| at(&e))?;
    let mount = Gta6Mount::open_root(&mut file, keys).map_err(|e| at(&e))?;
    let mut inspected = Inspected {
        documents: Vec::new(),
        plain: 0,
        not_read: 0,
    };
    for entry in &mount.archive.entries {
        // A registration document is deflated under a chunk table; a body stored as it is (a sound, a
        // movie) is passed over unread.
        let plain = entry.kind() == Some(Gta6Class::Media)
            && entry.real.is_some_and(|real| real <= BODY_LIMIT)
            && matches!(
                entry_chunks(&mount.archive, entry.index as usize),
                Ok(Some(_))
            );
        if !plain {
            continue;
        }
        inspected.plain += 1;
        match mount.read_entry_decrypted(&mut file, entry, keys) {
            Ok(body) => (inspected.documents).extend(Document::parse(&body).map(|d| (body, d))),
            Err(e) => {
                tracing::debug!(entry = entry.index, "a plain file is not read: {e}");
                inspected.not_read += 1;
            }
        }
    }
    let setup = |(_, d): &(Vec<u8>, Document)| matches!(d, Document::Setup(_));
    if !inspected.documents.iter().any(setup) {
        return Err(at(&format!(
            "no setup document (SSetupData) among its {} plain file(s), so this is not a DLC pack",
            inspected.plain
        )));
    }
    Ok(inspected)
}

fn print(path: &Path, inspected: &Inspected) -> Result<(), String> {
    let not_read = match inspected.not_read {
        0 => String::new(),
        n => format!("; {n} of them did not read"),
    };
    println!(
        "{}: a GTA VI pack, {} registration document(s) among {} plain file(s){not_read}",
        path.display(),
        inspected.documents.len(),
        inspected.plain
    );
    // The setups first, then the content definitions; the change sets come joined to their groups.
    for setups in [true, false] {
        for (body, document) in &inspected.documents {
            let wanted = match document {
                Document::Setup(_) => setups,
                Document::Content(_) => !setups,
                Document::ChangeSet(_) => false,
            };
            if wanted {
                crate::dlc::inspect_document(body)?;
            }
        }
    }
    for line in set_lines(path, inspected) {
        println!("{line}");
    }
    Ok(())
}

/// Each pack's change sets with the setup groups that name them and every file each names, then what a group or an include names that no document holds.
fn set_lines(path: &Path, inspected: &Inspected) -> Vec<String> {
    let documents: Vec<Document> = (inspected.documents.iter())
        .map(|(_, d)| d.clone())
        .collect();
    let registry = Registry::of(vec![(path.to_path_buf(), documents)]);
    let mut lines = Vec::new();
    for pack in &registry.packs {
        lines.push(String::new());
        lines.push(format!(
            "pack {}: {} change set(s)",
            pack.setup.device_name,
            pack.sets.len()
        ));
        for set in &pack.sets {
            let flags = match set.flags.is_empty() {
                true => String::new(),
                false => format!(" [{}]", set.flags.join(" ")),
            };
            let groups = match set.groups.is_empty() {
                true => "named by no group".to_owned(),
                false => format!("group {}", set.groups.join(", ")),
            };
            let level = if set.level {
                ", a set of the level"
            } else {
                ""
            };
            lines.push(format!(
                "change set {}{flags}: {groups}{level}, {} file(s)",
                set.id,
                set.files.len()
            ));
            for file in &set.files {
                let file_type = match file.file_type.as_str() {
                    "" => "(no fileType)",
                    named => named,
                };
                let mut line = format!("  {file_type}  {}", file.filename);
                if !file.flags.is_empty() {
                    line.push_str(&format!(" [{}]", file.flags.join(" ")));
                }
                lines.push(line);
            }
        }
        for id in &pack.unread {
            lines.push(format!(
                "named, and no document holds it: {id} (as ids are compared)"
            ));
        }
    }
    lines
}

#[cfg(test)]
#[allow(dead_code, clippy::duplicate_mod)]
#[path = "rpf9_fixture.rs"]
mod fixture;

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use or_io::container::rpf::gta6_container_write::{build, Rpf9IndexKey, Rpf9NewBody};
    use or_io::container::rpf::{GTA6_EXE_ENV, GTA6_SNAPSHOT_ENV};
    use or_title::dlc::{
        ContentChangeSetDef, ContentChangeSetGroup, ContentDef, IncludedChangeset, RegisteredFile,
        SetupContentType, SetupData,
    };

    use super::fixture::{self, Body};
    use super::*;

    const DEVICE: &str = "my_pack";

    /// The index key of the fixture's archives.
    fn key() -> Rpf9IndexKey {
        let like = fixture::archive(&[("a.txt", Body::Raw(b"a".to_vec()))]);
        let mount = Gta6Mount::open_root(&mut Cursor::new(&like[..]), &fixture::keys())
            .expect("the fixture opens");
        Rpf9IndexKey::of(&mount.archive.toc).expect("its index key")
    }

    fn registered(path: &str, file_type: &str) -> RegisteredFile {
        RegisteredFile {
            filename: format!("$(root)/{path}"),
            file_type: file_type.to_owned(),
            ..RegisteredFile::default()
        }
    }

    /// A pack laid out as `dlc create-gta6` lays one out: its files, its setup, the content definition the setup names and the change set that includes, written by the same archive writer.
    fn pack(dir: &Path) -> PathBuf {
        let set = ContentChangeSetDef {
            id: DEVICE.to_owned(),
            file_list: vec![
                registered("data/notes.meta", "TEST_FILE"),
                registered("maps/inner.rpf", "RPF_FILE"),
            ],
            ..ContentChangeSetDef::default()
        };
        let content = ContentDef {
            included_changesets: vec![IncludedChangeset {
                load_path: "$(root)/changesets/ccs_my_pack.meta".to_owned(),
                changeset_include_flags: vec!["kDefault".to_owned()],
                ..IncludedChangeset::default()
            }],
            ..ContentDef::default()
        };
        let setup = SetupData {
            name_hash: DEVICE.to_owned(),
            device_name: DEVICE.to_owned(),
            dat_file: "content.xml".to_owned(),
            content_type: SetupContentType::LevelPack,
            order: 1,
            content_change_set_groups: vec![ContentChangeSetGroup {
                name_hash: "GROUP_DEFAULT".to_owned(),
                content_change_sets: vec![DEVICE.into()],
            }],
            platform_data_under_root: Some(false),
            ..SetupData::default()
        };
        let inner = fixture::archive(&[("inner.txt", Body::Raw(b"inner".to_vec()))]);
        let text = |xml: String| Rpf9NewBody::Chunked(xml.into_bytes());
        let entries = vec![
            (
                "data/notes.meta".to_owned(),
                Rpf9NewBody::Chunked(b"notes ".repeat(64)),
            ),
            ("maps/inner.rpf".to_owned(), Rpf9NewBody::Archive(inner)),
            ("setup2.xml".to_owned(), text(setup.to_xml())),
            ("content.xml".to_owned(), text(content.to_xml())),
            ("changesets/ccs_my_pack.meta".to_owned(), text(set.to_xml())),
        ];
        write(dir, "pack.rpf", &entries)
    }

    fn write(dir: &Path, name: &str, entries: &[(String, Rpf9NewBody)]) -> PathBuf {
        let plan = build(entries, &key(), &fixture::keys()).expect("the pack builds");
        let mut bytes = Vec::new();
        plan.write_to(&mut Cursor::new(Vec::<u8>::new()), &mut bytes)
            .expect("written");
        let path = dir.join(name);
        std::fs::write(&path, bytes).expect("written");
        path
    }

    #[test]
    fn a_built_pack_inspects_with_its_setup_groups_and_content_typed() {
        let dir = tempfile::tempdir().expect("a folder");
        let path = pack(dir.path());
        let inspected = inspect(&path, &fixture::keys()).expect("the pack inspects");
        // The nested archive is not a plain file; the notes are one, and no document.
        assert_eq!((inspected.plain, inspected.not_read), (4, 0));
        // The documents come in the table's order, which the names' hashes set.
        let documents: Vec<Document> = (inspected.documents.iter())
            .map(|(_, d)| d.clone())
            .collect();
        assert_eq!(documents.len(), 3, "{documents:?}");
        let setup = documents.iter().find_map(|d| match d {
            Document::Setup(s) => Some(s),
            _ => None,
        });
        let content = documents.iter().find_map(|d| match d {
            Document::Content(c) => Some(c),
            _ => None,
        });
        let set = documents.iter().find_map(|d| match d {
            Document::ChangeSet(s) => Some(s),
            _ => None,
        });
        let (Some(setup), Some(content), Some(set)) = (setup, content, set) else {
            panic!("a setup, a content definition and a change set: {documents:?}");
        };
        assert_eq!(setup.device_name, DEVICE);
        assert_eq!(setup.order, 1);
        assert_eq!(
            setup.content_change_set_groups[0].name_hash,
            "GROUP_DEFAULT"
        );
        assert_eq!(
            content.included_changesets[0].load_path,
            "$(root)/changesets/ccs_my_pack.meta"
        );
        assert_eq!(set.file_list.len(), 2);

        let lines = set_lines(&path, &inspected);
        assert_eq!(
            lines,
            [
                "",
                "pack my_pack: 1 change set(s)",
                "change set my_pack: group GROUP_DEFAULT, 2 file(s)",
                "  TEST_FILE  $(root)/data/notes.meta",
                "  RPF_FILE  $(root)/maps/inner.rpf",
            ]
        );
        print(&path, &inspected).expect("each document prints typed");
    }

    #[test]
    fn an_archive_with_no_setup_is_refused_by_name() {
        let dir = tempfile::tempdir().expect("a folder");
        let notes = vec![(
            "data/notes.meta".to_owned(),
            Rpf9NewBody::Chunked(b"notes ".repeat(64)),
        )];
        let path = write(dir.path(), "plain.rpf", &notes);
        let err = inspect(&path, &fixture::keys()).expect_err("no pack");
        assert!(err.contains("plain.rpf"), "{err}");
        assert!(err.contains("SSetupData"), "{err}");
        assert!(err.contains("not a DLC pack"), "{err}");

        let err = run(&path, None, &["--bogus".to_owned()]).expect_err("refused");
        assert!(err.contains("--bogus"), "{err}");
        let err = run(&path, None, &["--exe".to_owned()]).expect_err("refused");
        assert!(err.contains("--exe"), "{err}");
    }

    #[test]
    fn every_pack_of_the_build_inspects() {
        let present = |var: &str| {
            std::env::var_os(var)
                .map(PathBuf::from)
                .filter(|p| p.exists())
        };
        let (Some(snapshot), Some(root)) =
            (present(GTA6_SNAPSHOT_ENV), present("OPENRAGE_GTA6_BUILD"))
        else {
            or_test_support::skip_install(
                "dlc inspect of every GTA VI pack",
                format_args!("set {GTA6_SNAPSHOT_ENV} and OPENRAGE_GTA6_BUILD"),
            );
            return;
        };
        let keys =
            crate::gta6_keys::load(Some(snapshot.as_path()), present(GTA6_EXE_ENV).as_deref())
                .expect("the key snapshot opens");
        let (mut archives, mut folders) = (Vec::new(), vec![root]);
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(&folder)
                .expect("the build lists")
                .flatten()
            {
                let path = entry.path();
                if path.is_dir() {
                    folders.push(path);
                } else if path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("rpf"))
                {
                    archives.push(path);
                }
            }
        }
        let (mut packs, mut refused, mut sets, mut not_read) = (0, 0, 0, 0);
        for archive in &archives {
            match inspect(archive, &keys) {
                Ok(inspected) => {
                    packs += 1;
                    not_read += inspected.not_read;
                    let content = (inspected.documents.iter())
                        .any(|(_, d)| matches!(d, Document::Content(_)));
                    assert!(
                        content,
                        "a pack holds the content definition its setup names"
                    );
                    sets += (set_lines(archive, &inspected).iter())
                        .filter(|line| line.starts_with("change set "))
                        .count();
                }
                Err(e) => {
                    assert!(e.contains("SSetupData"), "refused only as no pack: {e}");
                    refused += 1;
                }
            }
        }
        println!(
            "{packs} of {} archives inspect as packs, with {sets} change set(s) and {not_read} \
             plain file(s) not read; {refused} are refused as no pack",
            archives.len()
        );
        // Measured by this suite on build 1.0.2.1, which reads enciphered plain files too: 10 of its
        // 22 archives hold a setup.
        assert_eq!((packs, refused), (10, archives.len() - 10));
    }
}
