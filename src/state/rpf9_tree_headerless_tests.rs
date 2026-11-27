//! A headerless row edited in a GTA VI tree plans as a patched copy where its body fits its chunks,
//! and the tree becomes the copy its Save as… wrote; hermetic, with one install-gated check.

use or_io::container::rpf::gta6_kinds::kind_label;
use or_io::container::rpf::{open_at, Gta6Class, GTA6_SNAPSHOT_ENV};
use or_io::gta6::text::{read as read_text, TextItem};

use super::tests::{
    body_of, entry_named, files, finished, mount_of, reading_with, showing, source,
    stored_sections, synthetic_keys, toasts, tree, DOC, RESOURCE,
};
use super::*;
use crate::state::opening_an_rpf9_archive::said;
use crate::state::rpf_session::RpfDocument;
use crate::state::ToastType;

/// The row of the tree reading the GTA VI entry `entry` of its top archive.
fn row_of(session: &RpfEditSession, entry: u32) -> RpfNodeId {
    let archive = top_rpf9(session).expect("a GTA VI tree");
    (files(session).into_iter())
        .find(|&id| match session.node(id).map(|n| &n.backing) {
            Some(RpfNodeBacking::ArchiveFile { entry_index, .. }) => {
                (archive.rpf9_place(*entry_index)).is_some_and(|p| p.entry == entry)
            }
            _ => false,
        })
        .expect("a row reads the entry")
}

/// The archive of [`source`] in `dir`, its headerless row's virtual section changed by `edit` and
/// held as the loose form the tree saves into a row; the tree, the row and the edited section.
fn edited(
    dir: &Path,
    edit: impl FnOnce(&mut Vec<u8>),
) -> (PathBuf, RpfDocument, RpfNodeId, Vec<u8>) {
    let path = source(dir);
    let before = std::fs::read(&path).expect("the source");
    let mut sections = stored_sections(&before, RESOURCE);
    edit(&mut sections.virtual_section);
    let loose = write_rpf9_resource(&sections).expect("a loose resource");
    let mut tree = tree(&path);
    let entry = entry_named(&mount_of(&before), RESOURCE).index;
    let row = row_of(&tree.session, entry);
    (tree.session.set_bytes_for(row, loose)).expect("the row takes the bytes");
    (path, tree, row, sections.virtual_section)
}

#[test]
fn a_headerless_row_edited_at_its_length_saves_as_a_patched_copy_and_the_tree_becomes_it() {
    let _serial = crate::ui::theme::serial();
    let keys = synthetic_keys().expect("the synthetic snapshot parses");
    let _reading = reading_with(Arc::clone(&keys));
    let dir = tempfile::tempdir().expect("a scratch directory");
    // The root's u32, 77, zeroed: a body of the section's length that deflates no longer.
    let zero_the_root = |v: &mut Vec<u8>| {
        assert_eq!(v[0xE0], 77, "the root's value");
        v[0xE0..0xE4].fill(0);
    };
    let (path, tree, row, edited_v) = edited(dir.path(), zero_the_root);
    let edits = tree_edits(&tree.session).expect("the tree's edits");
    let planned = plan_tree(&path, &edits, &keys).expect("the save plans");
    assert_eq!(planned.writes(), Writes::Patched, "{:?}", planned.kept);

    let before = std::fs::read(&path).expect("the source");
    let mut state = showing(path.clone(), tree);
    let out = dir.path().join("copy.rpf");
    assert_eq!(
        state.save_gta6_tree_with(DOC, &out, synthetic_keys),
        Some(true)
    );
    finished(&mut state);
    let patched = ["copy.rpf", "is a patched copy", "has not been run"];
    assert!(
        said(&state, ToastType::Success, &patched),
        "{:?}",
        toasts(&state)
    );

    // Every byte outside the row's body is the source's, so each table span is as read.
    let copy = std::fs::read(&out).expect("the copy");
    let mount = mount_of(&before);
    let entry = entry_named(&mount, RESOURCE);
    let start = mount.absolute_start(entry).expect("a start");
    let nested = (mount.archive.entry_extra(entry.index as usize))
        .and_then(|x| x.nested)
        .expect("a 20-byte row");
    let (at, len) = nested.ciphered_body_extent(start);
    let (at, end) = (at as usize, (at + len) as usize);
    assert_eq!(copy.len(), before.len());
    assert_eq!(copy[..at], before[..at], "every byte before the body");
    assert_eq!(copy[end..], before[end..], "every byte after the body");
    assert_ne!(copy[at..end], before[at..end], "the body changed");
    assert_eq!(body_of(&copy, RESOURCE), edited_v);
    assert_eq!(std::fs::read(&path).expect("the source"), before);

    // The tree is the copy now, and nothing in it is marked edited.
    let doc = state.document(DOC).expect("the tree");
    assert_eq!(doc.path.as_deref(), Some(out.as_path()));
    assert_eq!(doc.name, "copy");
    let DocPayload::Rpf(rpf) = &doc.payload else {
        panic!("the tree is still a tree");
    };
    let top = top_rpf9(&rpf.session).expect("a GTA VI tree");
    assert_eq!(top.path(), Some(out.as_path()));
    assert!(!doc.dirty, "the tab is saved");
    let backing = rpf.session.node(row).map(|n| n.backing.clone());
    assert!(
        matches!(
            backing,
            Some(RpfNodeBacking::ArchiveFile {
                override_bytes: None,
                ..
            })
        ),
        "{backing:?}"
    );
}

#[test]
fn a_row_added_under_a_name_no_vocabulary_holds_is_found_in_the_copy_by_its_hash() {
    let _serial = crate::ui::theme::serial();
    let keys = synthetic_keys().expect("the synthetic snapshot parses");
    let _reading = reading_with(Arc::clone(&keys));
    let dir = tempfile::tempdir().expect("a scratch directory");
    let path = source(dir.path());
    let mut tree = tree(&path);
    let root = tree.session.root_id();
    let added = b"a body under a name the modder made up. ".repeat(8);
    let row = (tree
        .session
        .add_file_copy(root, "zq_invented_7f3a.bin", added))
    .expect("the row is added");
    let mut state = showing(path, tree);
    let out = dir.path().join("copy.rpf");
    assert_eq!(
        state.save_gta6_tree_with(DOC, &out, synthetic_keys),
        Some(true)
    );
    finished(&mut state);
    let doc = state.document(DOC).expect("the tree");
    assert_eq!(
        doc.path.as_deref(),
        Some(out.as_path()),
        "{:?}",
        toasts(&state)
    );
    assert!(!doc.dirty, "the tree is the copy, which holds its edits");
    let DocPayload::Rpf(rpf) = &doc.payload else {
        panic!("the tree is still a tree");
    };
    let backing = rpf.session.node(row).map(|n| n.backing.clone());
    assert!(
        matches!(
            backing,
            Some(RpfNodeBacking::ArchiveFile {
                override_bytes: None,
                ..
            })
        ),
        "{backing:?}"
    );
}

#[test]
fn a_headerless_row_that_cannot_go_in_place_plans_a_rebuild_naming_the_row_and_why() {
    let _serial = crate::ui::theme::serial();
    let keys = synthetic_keys().expect("the synthetic snapshot parses");
    let _reading = reading_with(Arc::clone(&keys));
    // 64 bytes no deflater shrinks, between the block map's pointer and the map, at the section's
    // length: far past what a chunk's stored size leaves spare.
    let noisy = |v: &mut Vec<u8>| {
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        for byte in &mut v[0x20..0x60] {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *byte = x as u8;
        }
    };
    let longer = |v: &mut Vec<u8>| v.extend([0u8; 16]);
    type Edit<'a> = &'a dyn Fn(&mut Vec<u8>);
    let cases: [(Edit, &str); 2] = [
        (&noisy, "deflates at best"),
        (&longer, "288 bytes, not the 272"),
    ];
    for (edit, why) in cases {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let (path, tree, row, _) = edited(dir.path(), edit);
        let shown = (tree.session.node(row).map(|n| n.name.clone())).expect("the row");
        let edits = tree_edits(&tree.session).expect("the tree's edits");
        let planned = plan_tree(&path, &edits, &keys).expect("the save plans");
        assert_eq!(planned.writes(), Writes::Rebuilt, "{why}");
        let named = |kept: &String| {
            kept.contains(&shown) && kept.contains(why) && kept.ends_with("bytes over")
        };
        assert!(planned.kept.iter().any(named), "{why}: {:?}", planned.kept);
    }
    // Bytes that read as no resource are named too, never guessed at.
    let dir = tempfile::tempdir().expect("a scratch directory");
    let (path, mut tree, row, _) = edited(dir.path(), |_| {});
    (tree.session.set_bytes_for(row, b"no resource".to_vec())).expect("the row takes the bytes");
    let edits = tree_edits(&tree.session).expect("the tree's edits");
    let planned = plan_tree(&path, &edits, &keys).expect("the save plans");
    let named = |kept: &String| kept.ends_with("does not read as a resource from its start");
    assert!(planned.kept.iter().any(named), "{:?}", planned.kept);
}

/// The span of a build's archive a text table's row ends within.
const SPAN: u64 = 64 << 20;

/// A text table's row of a build archive, nested at most three deep, whose body ends in the first
/// [`SPAN`] bytes of its file: the file, its body's offset and where its body ends.
fn text_table_row(build: &Path, keys: &GtaViKeys) -> Option<(PathBuf, u64, u64)> {
    let mut archives: Vec<PathBuf> = (std::fs::read_dir(build).into_iter().flatten())
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("rpf"))
        .collect();
    archives.sort();
    for path in archives {
        let Ok(mut file) = File::open(&path) else {
            continue;
        };
        let Ok(root) = Gta6Mount::open_root(&mut file, keys) else {
            continue;
        };
        let mut stack = vec![(root, 0)];
        while let Some((mount, depth)) = stack.pop() {
            for e in &mount.archive.entries {
                let Some(start) = mount.absolute_start(e).filter(|&s| s < SPAN) else {
                    continue;
                };
                if e.kind() == Some(Gta6Class::NestedArchive) {
                    let inner = (depth < 3).then(|| mount.open_nested(&mut file, e, keys));
                    if let Some(Ok(inner)) = inner {
                        stack.push((inner, depth + 1));
                    }
                    continue;
                }
                let extra = mount.archive.entry_extra(e.index as usize);
                let Some(row) = extra.and_then(|x| x.nested) else {
                    continue;
                };
                let (at, len) = row.ciphered_body_extent(start);
                if kind_label(None, Some(row.resource_type)) == "text table" && at + len <= SPAN {
                    return Some((path, at, at + len));
                }
            }
        }
    }
    None
}

/// The first text of a real text table holding a letter, that letter's case changed so the text
/// keeps its length, saved from the table's loose form as the tree saves a row: a patched copy that
/// reads back as the edit. Install-gated: it needs the key snapshot and the build, and copies the
/// first bytes of one archive, up to the row's end and at most 64 MiB.
#[test]
fn a_real_text_tables_first_text_changed_at_its_length_saves_as_a_patched_copy() {
    const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
    const WHAT: &str = "gta6 text table saved through the tree as a patched copy";
    let env = |var: &str| (std::env::var_os(var).map(PathBuf::from)).filter(|p| p.exists());
    let (Some(snapshot), Some(build)) = (env(GTA6_SNAPSHOT_ENV), env(BUILD_ENV)) else {
        let why = format_args!("set {GTA6_SNAPSHOT_ENV} and {BUILD_ENV}");
        or_test_support::skip_install(WHAT, why);
        return;
    };
    let keys = GtaViKeys::from_file(&snapshot).expect("the key snapshot parses");
    let Some((file, offset, end)) = text_table_row(&build, &keys) else {
        or_test_support::skip_install(WHAT, "no text table in the first 64 MiB of an archive");
        return;
    };
    let dir = tempfile::tempdir().expect("a scratch directory");
    let span = dir.path().join("span.rpf");
    let mut from = File::open(&file).expect("the archive opens");
    let mut to = File::create(&span).expect("the span is written");
    std::io::copy(&mut (&mut from).take(end), &mut to).expect("the span is copied");
    drop(to);

    let mut src = File::open(&span).expect("the span opens");
    let mut found = chains(&mut src, &BTreeSet::from([offset]), &keys).expect("the row is found");
    let at = found.remove(&offset).expect("the row");
    assert!(at.headerless, "a text table's row is a 20-byte one");
    let index = *at.chain.last().expect("an entry");
    let mount = Gta6Mount {
        base: at.base,
        archive: open_at(&mut src, at.base, &keys).expect("its archive opens"),
    };
    let e = &mount.archive.entries[index as usize];
    // The stored bytes past the slot, deciphered, read as the tree reads the row's loose form.
    let stored = match e.cipher {
        Some(_) => (mount.read_entry_deciphered_stored(&mut src, e, &keys)).expect("deciphered"),
        None => {
            let mut stored = vec![0u8; (end - offset) as usize];
            (src.seek(std::io::SeekFrom::Start(offset)))
                .and_then(|_| src.read_exact(&mut stored))
                .expect("the stored bytes");
            stored
        }
    };
    let (find, bytes) = read_rpf9_body(Cursor::new(&stored), 0, u64::MAX).expect("a resource");
    let sections = Rpf9Sections::of(&find, &bytes).expect("its sections");
    let (mut table, _) = read_text(&sections.virtual_section).expect("the text table reads");
    let text = (table.entries.iter_mut())
        .find_map(|entry| match &mut entry.item {
            TextItem::Text(text) if text.bytes().any(|b| b.is_ascii_alphabetic()) => Some(text),
            _ => None,
        })
        .expect("a text holding a letter");
    let letter = text
        .bytes()
        .position(|b| b.is_ascii_alphabetic())
        .expect("a letter");
    let mut changed = std::mem::take(text).into_bytes();
    // A letter's case is its 0x20 bit, so the text keeps its byte length.
    changed[letter] ^= 0x20;
    *text = String::from_utf8(changed).expect("an ASCII letter's case keeps UTF-8");
    let virtual_section = (table.to_section_over(&sections.virtual_section)).expect("rewritten");
    let edited = Rpf9Sections {
        virtual_section,
        ..sections.clone()
    };
    let loose = write_rpf9_resource(&edited).expect("a loose form");
    let row = Replaced {
        name: "text table".into(),
        at,
        body: loose.clone(),
    };
    let writer = Rpf9ArchiveWriter::new();
    let planned = plan_save(&span, &keys, &[row], Vec::new(), writer).expect("the save plans");
    assert_eq!(planned.writes(), Writes::Patched, "{:?}", planned.kept);
    let out = dir.path().join("copy.rpf");
    let saved = write_planned(&span, &out, &keys, &planned).expect("the copy is written");
    assert_eq!(saved, Saved::Patched);

    let mut copy = File::open(&out).expect("the copy opens");
    let reopened = Gta6Mount {
        base: mount.base,
        archive: open_at(&mut copy, mount.base, &keys).expect("its archive reopens"),
    };
    let e = &reopened.archive.entries[index as usize];
    let read = (reopened.read_entry_decrypted(&mut copy, e, &keys)).expect("the row reads");
    assert_eq!(Some(read), stored_body(&loose));
}

#[path = "gta6_app_saves_per_kind_tests.rs"]
pub(crate) mod saves_per_kind;
