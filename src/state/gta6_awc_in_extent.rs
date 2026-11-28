//! A GTA VI audio container edited in the app saves at the length it was read: the edit is laid back
//! into the extents its chunks were read at, so a copy of its archive can patch the body in place.
//! An edit that does not fit saves as the whole container, and a copy of the archive is then rebuilt.

use std::path::Path;
use std::sync::Arc;

use or_io::awc::{AwcFile, AwcKey};

use crate::state::document::{DocPayload, Document};
use crate::state::gta6_other_bodies_save::Write;
use crate::state::gta6_resource_save::EditedSave;
use crate::state::rpf_archive_open::tree_open::{self, TableRead};

/// The audio container row's `edited_sections`: a GTA VI body's edit laid on the worker in the
/// extents of the container as read, else the whole container; `None` while nothing is edited.
pub(super) fn edited(doc: &Document) -> Option<Result<EditedSave, String>> {
    let DocPayload::Awc(awc) = &doc.payload else {
        return None;
    };
    if !doc.dirty {
        return None;
    }
    let body = (doc.path.as_deref())
        .and_then(|path| Some((path, crate::state::rpf9_scan::scanned_body(path)?)));
    let Some((path, (archive, offset))) = body else {
        return Some(awc.to_bytes().map(EditedSave::Bytes));
    };
    let (path, archive, awc) = (path.to_path_buf(), archive.to_path_buf(), awc.clone());
    let write: Arc<Write> = Arc::new(move || {
        let mut awc = awc.clone();
        let whole = awc.to_bytes()?;
        let fitted = (as_read(&path, &archive, offset))
            .and_then(|source| laid(&source, awc.file(AwcKey::default())?));
        Ok(fitted.unwrap_or_else(|why| {
            tracing::warn!(
                "{}: the edited audio container is saved whole, not in the extents it was read with \
                 ({why}); a copy of its archive that holds it at another length is a rebuilt \
                 archive, which the game refuses because only its maker can sign the table",
                path.display()
            );
            whole
        }))
    });
    Some(Ok(EditedSave::Written(write)))
}

/// The container the body at `offset` of `archive` opened as under `path`, read again by the door
/// it opened through: its tree row's table, else a scan's read.
fn as_read(path: &Path, archive: &Path, offset: u64) -> Result<Vec<u8>, String> {
    match tree_open::read(path, archive, offset) {
        None => crate::state::rpf9_scan::extracted_body(archive, offset),
        Some(read) => match read? {
            TableRead::Body(_, bytes) => Ok(bytes),
            TableRead::Plain(DocPayload::Hex(hex)) => Ok(hex.bytes),
            TableRead::Plain(_) => Err("its row reads as no audio container".to_owned()),
        },
    }
}

/// `edited` laid in the extents the container `source` holds its chunks at, so at `source`'s
/// length; why not where it does not fit them.
fn laid(source: &[u8], edited: &AwcFile) -> Result<Vec<u8>, String> {
    let source = AwcFile::parse(source, AwcKey::default())
        .map_err(|e| format!("the container as read does not parse: {e}"))?;
    let laid = source.in_extent(edited).map_err(|e| e.to_string())?;
    laid.to_bytes(AwcKey::default()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs::File;
    use std::io::{Cursor, Read, Seek, SeekFrom};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use or_io::awc::{AwcChunkKind, AwcFile, AwcKey};
    use or_io::container::rpf::gta6_container_write::{build, Rpf9IndexKey, Rpf9NewBody};
    use or_io::container::rpf::{
        Gta6Context, Gta6Mount, GtaViKeys, NameResolver, Rpf9ArchiveWriter,
    };
    use or_io::container::RpfArchive;
    use or_io::gta6::hash::name_hash64;

    use super::{as_read, laid};
    use crate::state::document::{DocumentId, RpfNodeId};
    use crate::state::opening_a_gta_vi_awc::gta_vi;
    use crate::state::rpf9_scan::{body_path, scanned_body, View};
    use crate::state::rpf_archive_open::gta6_session::{self, Reading};
    use crate::state::rpf_archive_open::tree_open::{leave, name, row_of};
    use crate::state::rpf_archive_open::tree_verbs::headerless_tests::saves_per_kind::overlaid;
    use crate::state::rpf_archive_open::tree_verbs::tests::synthetic_keys;
    use crate::state::rpf_archive_open::tree_verbs::{
        chains, plan_save, write_planned, Planned, Replaced, Saved, Writes,
    };
    use crate::state::rpf_node::ArchiveSource;
    use crate::state::rpf_session::{RpfDocument, RpfEditSession};
    use crate::state::{AppState, DocPayload, Document, RpfNodeBacking, ToastType};

    const DOC: DocumentId = 9_700;
    const AUDIO: &str = "data/voice.awc";
    const DATA: AwcChunkKind = AwcChunkKind::Data;

    /// Archives opened through [`showing`] while this is held are read with `keys` and the given names.
    fn reading(keys: Arc<GtaViKeys>, named: &[&str]) -> Reading {
        let mut names = NameResolver::new();
        for name in named {
            names.insert(name_hash64(name.as_bytes()), *name);
        }
        gta6_session::hold(Gta6Context {
            keys,
            names: Arc::new(names),
        })
    }

    fn pattern(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 97) as u8 + 1).collect()
    }

    /// A data chunk of 200 patterned bytes and 20 zero ones.
    fn data() -> Vec<u8> {
        let mut data = pattern(200);
        data.resize(220, 0);
        data
    }

    /// One GTA VI stream of a data chunk and a seek table after it, on 16-byte steps.
    fn container() -> Vec<u8> {
        gta_vi(
            0xFF61,
            &[(0x0A01, vec![(0x55, data()), (0xA3, pattern(40))])],
        )
    }

    /// `source` with its stream's data chunk replaced by `payload`, which moves the seek table
    /// after it by whole 16-byte steps.
    fn replaced(source: &[u8], payload: Vec<u8>) -> AwcFile {
        let mut awc = AwcFile::parse(source, AwcKey::default()).expect("the container parses");
        awc.set_chunk_payload(0, DATA, payload).expect("replaced");
        awc
    }

    fn offsets(bytes: &[u8]) -> Vec<u32> {
        let awc = AwcFile::parse(bytes, AwcKey::default()).expect("it reads back");
        (awc.streams.iter().flat_map(|s| &s.chunks))
            .map(|c| c.offset)
            .collect()
    }

    #[test]
    fn an_edit_that_fits_keeps_the_containers_length_and_one_that_does_not_is_refused_by_name() {
        let source = container();
        // The same length: one byte changed.
        let mut same = data();
        same[7] ^= 0x40;
        let out = laid(&source, &replaced(&source, same.clone())).expect("in extent");
        assert_eq!(out.len(), source.len());
        let read = AwcFile::parse(&out, AwcKey::default()).expect("it reads back");
        assert_eq!(read.streams[0].chunk(DATA).map(|c| &c.payload), Some(&same));

        // 20 bytes shorter: rewritten whole the seek table moves 16 bytes up; laid, nothing moves.
        let shorter = replaced(&source, pattern(200));
        let whole = shorter.to_bytes(AwcKey::default()).expect("written");
        assert_eq!(whole.len() + 16, source.len());
        let out = laid(&source, &shorter).expect("in extent");
        assert_eq!(out.len(), source.len());
        assert_eq!(offsets(&out), offsets(&source));

        // 64 bytes longer runs past the 4 bytes of padding before the seek table.
        let mut longer = data();
        longer.extend(pattern(64));
        let why = laid(&source, &replaced(&source, longer)).expect_err("refused");
        assert!(why.contains("not laid in extent"), "{why}");
        assert!(why.contains("data chunk"), "{why}");

        let why = laid(b"ADAT", &replaced(&source, pattern(200))).expect_err("refused");
        assert!(why.contains("does not parse"), "{why}");
    }

    /// An archive of the container under a 24-byte row, as `dir/source.rpf`.
    fn archive(dir: &Path) -> PathBuf {
        let key = Rpf9IndexKey {
            key_id: 0xDEAD_BEEF,
            iv: [0x33; 16],
        };
        let keys = synthetic_keys().expect("the synthetic snapshot parses");
        let entries = [(AUDIO.to_owned(), Rpf9NewBody::Chunked(container()))];
        let plan = build(&entries, or_io::platform::Platform::XboxSeries, &key, &keys)
            .expect("the archive builds");
        let mut bytes = Vec::new();
        (plan.write_to(&mut Cursor::new(Vec::<u8>::new()), &mut bytes))
            .expect("the archive is laid out");
        let path = dir.join("source.rpf");
        std::fs::write(&path, bytes).expect("the archive is written");
        path
    }

    /// An app showing the archive at `path` as the tree every archive opens as.
    fn showing(path: &Path) -> Option<AppState> {
        let crypto = gta6_session::crypto();
        let archive = RpfArchive::open_with(path, crypto).ok()?;
        let tree = RpfDocument::new(archive).ok()?;
        let mut state = AppState::new();
        let doc = Document::new(DOC, Some(path.to_path_buf()), DocPayload::Rpf(tree));
        state.documents.push(doc);
        Some(state)
    }

    fn session(state: &AppState) -> &RpfEditSession {
        match state.document(DOC).map(|d| &d.payload) {
            Some(DocPayload::Rpf(rpf)) => &rpf.session,
            _ => panic!("the tree is open"),
        }
    }

    /// The body row `node` of the tree holds once a save put one there.
    fn held(state: &AppState, node: RpfNodeId) -> Option<Vec<u8>> {
        match session(state).node(node).map(|n| &n.backing) {
            Some(RpfNodeBacking::ArchiveFile {
                override_bytes: Some(body),
                ..
            }) => Some(body.to_vec()),
            _ => None,
        }
    }

    /// Save `doc` into its row and wait, until `deadline`, for the row to hold the body it saved.
    fn saved_into_its_row(
        state: &mut AppState,
        doc: DocumentId,
        node: RpfNodeId,
        deadline: Instant,
    ) -> Vec<u8> {
        state.toasts.clear();
        assert_eq!(state.save_into_tree_row(doc), Some(true));
        loop {
            state.poll_save_result();
            if let Some(body) = held(state, node) {
                return body;
            }
            let refused = (state.toasts.iter()).find(|t| t.toast_type == ToastType::Error);
            assert!(refused.is_none(), "{:?}", refused.map(|t| &t.message));
            assert!(Instant::now() < deadline, "the save never ended");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// What a save of the tree plans for the body row `node` holds, read with `keys`.
    fn planned(state: &AppState, disk: &Path, node: RpfNodeId, keys: &GtaViKeys) -> Planned {
        let session = session(state);
        let Some(RpfNodeBacking::ArchiveFile {
            archive_id,
            entry_index,
            override_bytes: Some(body),
        }) = session.node(node).map(|n| &n.backing)
        else {
            panic!("row {node} holds the saved body");
        };
        let archive = &session.archive(*archive_id).expect("its archive").archive;
        let at = (archive.rpf9_place(*entry_index))
            .expect("a GTA VI row")
            .offset;
        let mut file = File::open(disk).expect("the archive");
        let mut found = chains(&mut file, &BTreeSet::from([at]), keys).expect("the row is found");
        drop(file);
        let replaced = Replaced {
            name: (session.node(node))
                .map(|n| n.name.clone())
                .unwrap_or_default(),
            at: found.remove(&at).expect("the row's place"),
            body: body.to_vec(),
        };
        plan_save(
            disk,
            keys,
            &[replaced],
            Vec::new(),
            Rpf9ArchiveWriter::new(),
        )
        .expect("the save plans")
    }

    /// The body of entry `index` of the top-level table of the archive at `path`.
    fn body_of(path: &Path, index: u32, keys: &GtaViKeys) -> Vec<u8> {
        let bytes = std::fs::read(path).expect("the archive");
        let mount = Gta6Mount::open_root(&mut Cursor::new(&bytes), keys).expect("it opens");
        let entry = (mount.archive.entries.iter())
            .find(|e| e.index == index)
            .expect("the entry is kept");
        (mount.read_entry(Cursor::new(&bytes), entry)).expect("the body reads")
    }

    /// The container row `AUDIO` of a new archive in `dir`, its data chunk replaced by `payload`
    /// in a document opened from the tree and saved into its row: the archive, the body the row
    /// holds, what a save of the tree plans and the row's entry.
    fn edited_and_saved(dir: &Path, payload: Vec<u8>) -> (PathBuf, Vec<u8>, Planned, u32) {
        let keys = synthetic_keys().expect("the synthetic snapshot parses");
        let disk = archive(dir);
        let mut state = showing(&disk).expect("the archive opens as a tree");
        let node = (session(&state).all_nodes().into_iter())
            .find(|&id| session(&state).path_of(id).as_deref() == Some(AUDIO))
            .expect("the row is listed");
        let (place, start) = {
            let session = session(&state);
            let Some(RpfNodeBacking::ArchiveFile {
                archive_id,
                entry_index,
                ..
            }) = session.node(node).map(|n| &n.backing)
            else {
                panic!("the row is a file of the archive");
            };
            let archive = &session.archive(*archive_id).expect("its archive").archive;
            let place = archive.rpf9_place(*entry_index).expect("a GTA VI row");
            let row = archive.binary_entry(*entry_index).expect("its row");
            (place.clone(), place.base + row.file_offset)
        };
        // ⚠ A built archive deflates every body, which a scan names no audio container, so the
        // document the tree's door would open is put in place by hand on the row's path.
        let path = body_path(&place.file, place.offset, View::Audio);
        leave(&path, &place, start, synthetic_keys, None);
        name(&state, &path, DOC, node);
        let awc = crate::audio::AwcDocument::open(&container()).expect("an ADAT container");
        let doc = state.insert_document(path.clone(), DocPayload::Awc(awc), None, None);
        assert_eq!(row_of(&state, &path).map(|r| r.entry_id), Some(node));
        let d = state.document_mut(doc).expect("open");
        if let DocPayload::Awc(awc) = &mut d.payload {
            let file = awc.file(AwcKey::default()).expect("it parses");
            file.set_chunk_payload(0, DATA, payload).expect("replaced");
        }
        d.mark_edited();
        let deadline = Instant::now() + Duration::from_secs(60);
        let body = saved_into_its_row(&mut state, doc, node, deadline);
        let plan = planned(&state, &disk, node, &keys);
        (disk, body, plan, place.entry)
    }

    #[test]
    fn a_stream_edited_within_its_extent_saves_into_its_row_and_plans_a_patched_copy() {
        let _serial = crate::ui::theme::serial();
        let keys = synthetic_keys().expect("the synthetic snapshot parses");
        let _reading = reading(Arc::clone(&keys), &[AUDIO]);
        let dir = tempfile::tempdir().expect("a scratch directory");
        // 20 bytes shorter, which the whole container would be 16 bytes shorter for.
        let (disk, body, plan, entry) = edited_and_saved(dir.path(), pattern(200));
        assert_eq!(
            body.len(),
            container().len(),
            "the container keeps its length"
        );
        assert_eq!(offsets(&body), offsets(&container()));
        assert_eq!(plan.writes(), Writes::Patched, "{:?}", plan.kept);

        let out = dir.path().join("copy.rpf");
        let written = write_planned(&disk, &out, &keys, &plan).expect("the copy is written");
        assert_eq!(written, Saved::Patched);
        assert_eq!(
            body_of(&out, entry, &keys),
            body,
            "the copy reads the edit back"
        );
    }

    #[test]
    fn a_stream_edited_past_its_extent_saves_whole_and_plans_a_rebuilt_archive() {
        let _serial = crate::ui::theme::serial();
        let keys = synthetic_keys().expect("the synthetic snapshot parses");
        let _reading = reading(Arc::clone(&keys), &[AUDIO]);
        let dir = tempfile::tempdir().expect("a scratch directory");
        let mut longer = data();
        longer.extend(pattern(64));
        let (_, body, plan, _) = edited_and_saved(dir.path(), longer.clone());
        let whole = (replaced(&container(), longer).to_bytes(AwcKey::default())).expect("written");
        assert_eq!(body, whole, "the whole container is saved");
        assert_eq!(plan.writes(), Writes::Rebuilt);
        let said = plan.kept.join("; ");
        assert!(said.contains("64 bytes over"), "{said}");
    }

    /// The first stream of clear audio containers in the nested archives of the build's largest
    /// archives, each nested archive opened through the tree smallest first, replaced by itself
    /// decoded and encoded again, saved into its row and planned: Patched or the refusal by name is
    /// printed for each, with how many were laid in their extents against how many grew past them.
    /// Install-gated: it needs the key snapshot and the build, and with them a run that plans no
    /// stream fails.
    #[test]
    fn a_stream_of_the_build_replaced_by_itself_re_encoded_saves_as_a_patched_copy_or_is_refused_by_name(
    ) {
        const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
        const WHAT: &str = "gta6 audio stream re-encoded and saved through the tree";
        /// Streams planned before the test stops, and containers tried at most.
        const PLANNED: usize = 3;
        const TRIES: usize = 12;
        /// The largest archives looked in, and the nested archives listed in each at most.
        const ARCHIVES: usize = 6;
        const PER_ARCHIVE: usize = 16;
        /// A container larger than this is passed over: its re-encode would take seconds.
        const CAP: u64 = 1 << 20;
        /// No container is started past this, and every wait ends by `END`, so the test ends
        /// within nextest's 300 s with what it found, a plan's own time past `END` included.
        const BUDGET: Duration = Duration::from_secs(200);
        const END: Duration = Duration::from_secs(240);
        const WAIT: Duration = Duration::from_secs(60);
        let started = Instant::now();
        let wait = || (Instant::now() + WAIT).min(started + END);
        let build = (std::env::var_os(BUILD_ENV).map(PathBuf::from)).filter(|p| p.is_dir());
        let keys = (build.as_ref()).and_then(|_| crate::state::rpf9_scan::rpf9_table::keys().ok());
        let (Some(build), Some(keys)) = (build, keys) else {
            or_test_support::skip_install(
                WHAT,
                format_args!("set OPENRAGE_GTA6_KEYS and {BUILD_ENV}"),
            );
            return;
        };
        let _serial = crate::ui::theme::serial();
        let _reading = reading(Arc::clone(&keys), &[]);
        let mut archives: Vec<(u64, PathBuf)> = Vec::new();
        let mut rpfs_in = |dir: &Path| {
            for path in (std::fs::read_dir(dir).into_iter().flatten().flatten()).map(|e| e.path()) {
                if path.extension().and_then(|x| x.to_str()) == Some("rpf") {
                    if let Ok(meta) = std::fs::metadata(&path) {
                        archives.push((meta.len(), path));
                    }
                }
            }
        };
        rpfs_in(&build);
        for pack in (std::fs::read_dir(build.join("pack")).into_iter().flatten()).flatten() {
            rpfs_in(&pack.path());
        }
        assert!(!archives.is_empty(), "{BUILD_ENV} holds no archive");
        // ⚠ The build's audio is in nested archives of one of its largest archives, which is not
        // the largest of all, so each of the largest is tried, its nested archives spread over sizes.
        archives.sort_by(|a, b| b.cmp(a));
        let over = |planned_n: usize, tries: usize| {
            planned_n >= PLANNED || tries >= TRIES || started.elapsed() > BUDGET
        };
        let (mut said, mut tries, mut planned_n, mut fits, mut grows) = (Vec::new(), 0, 0, 0, 0);
        let mut listed = 0;
        'archives: for (_, disk) in archives.iter().take(ARCHIVES) {
            if over(planned_n, tries) {
                break;
            }
            let file_name = crate::state::jobs::file_name(disk);
            let Some(mut state) = showing(disk) else {
                said.push(format!("{file_name}: does not open as a tree"));
                continue;
            };
            let mut nested: Vec<(u32, RpfNodeId)> = {
                let session = session(&state);
                (session.all_nodes().into_iter())
                    .filter_map(|id| {
                        let RpfNodeBacking::Archive {
                            source:
                                ArchiveSource::Nested {
                                    parent_archive_id,
                                    parent_entry_index,
                                    ..
                                },
                            ..
                        } = &session.node(id)?.backing
                        else {
                            return None;
                        };
                        let parent = &session.archive(*parent_archive_id)?.archive;
                        let row = parent.binary_entry(*parent_entry_index)?;
                        Some((row.uncompressed_size, id))
                    })
                    .collect()
            };
            nested.sort();
            // Spread over their sizes, the smallest first, so audio in a larger one is reached too.
            let take = PER_ARCHIVE.min(nested.len());
            let picked: Vec<RpfNodeId> = (0..take)
                .map(|i| nested[i * nested.len() / take].1)
                .collect();
            for (number, &folder) in picked.iter().enumerate() {
                if over(planned_n, tries) {
                    break 'archives;
                }
                let number = number + 1;
                listed += 1;
                state.request_expand(DOC, folder);
                let deadline = wait();
                while session(&state).needs_expansion(folder) {
                    state.poll_expansions();
                    assert!(
                        Instant::now() < deadline,
                        "nested archive {number} of {file_name} never listed"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                let inner = match session(&state).node(folder).map(|n| &n.backing) {
                    Some(RpfNodeBacking::Archive {
                        archive_id: Some(inner),
                        ..
                    }) => *inner,
                    Some(RpfNodeBacking::Archive { open_error, .. }) => {
                        let why = open_error.as_deref().unwrap_or("no reason given");
                        said.push(format!("nested archive {number} of {file_name}: {why}"));
                        continue;
                    }
                    _ => continue,
                };
                // Clear rows of the nested archive whose body starts at their row and is an
                // audio container.
                let rows: Vec<(RpfNodeId, u32)> = {
                    let session = session(&state);
                    let mut file = File::open(disk).expect("the archive");
                    (session.all_nodes().into_iter())
                        .filter_map(|id| {
                            let RpfNodeBacking::ArchiveFile {
                                archive_id,
                                entry_index,
                                ..
                            } = &session.node(id).filter(|n| !n.is_dir())?.backing
                            else {
                                return None;
                            };
                            if *archive_id != inner {
                                return None;
                            }
                            let archive = &session.archive(*archive_id)?.archive;
                            let place = archive.rpf9_place(*entry_index)?;
                            let row = archive.binary_entry(*entry_index)?;
                            let small = (16..=CAP).contains(&u64::from(row.uncompressed_size));
                            let starts = place.offset == place.base + row.file_offset;
                            let mut head = [0u8; 4];
                            file.seek(SeekFrom::Start(place.offset)).ok()?;
                            file.read_exact(&mut head).ok()?;
                            let clear = small && starts && !place.ciphered && &head == b"ADAT";
                            clear.then_some((id, place.entry))
                        })
                        .collect()
                };
                for (node, entry) in rows {
                    if over(planned_n, tries) {
                        break 'archives;
                    }
                    tries += 1;
                    let what = format!("entry {entry} of nested archive {number} of {file_name}");
                    state.open_rpf_entry(DOC, node);
                    let deadline = wait();
                    while state.load_receiver.is_some() || !state.pending_loads.is_empty() {
                        state.poll_load_result();
                        assert!(Instant::now() < deadline, "the load never ended");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    let opened = (state.documents.iter()).find(|d| {
                        let row = (d.path.as_deref()).and_then(|p| row_of(&state, p));
                        matches!(d.payload, DocPayload::Awc(_))
                            && row.map(|r| r.entry_id) == Some(node)
                    });
                    let found = opened.and_then(|d| Some((d.id, d.path.clone()?)));
                    let Some((doc, path)) = found else {
                        said.push(format!("{what}: the tree opened no audio container"));
                        continue;
                    };
                    let streams = state.awc_streams(doc).ok();
                    let Some(stream) = streams.and_then(|rows| rows.first().map(|r| r.stream))
                    else {
                        said.push(format!("{what}: no stream is listed"));
                        continue;
                    };
                    let replace = (state.awc_wav(doc, stream))
                        .and_then(|wav| state.awc_replace(doc, stream, &wav));
                    if let Err(why) = replace {
                        said.push(format!("{what}: the replace is refused by name: {why}"));
                        continue;
                    }
                    let (archive, offset) = scanned_body(&path).expect("a body of the archive");
                    let source = as_read(&path, archive, offset).expect("the container as read");
                    let in_extent = laid(&source, state.awc_file(doc).expect("its container"));
                    match &in_extent {
                        Ok(_) => fits += 1,
                        Err(_) => grows += 1,
                    }
                    let body = saved_into_its_row(&mut state, doc, node, wait());
                    if let Ok(laid) = &in_extent {
                        assert_eq!(laid.len(), source.len(), "{what}: laid at its length");
                        assert_eq!(
                            &body, laid,
                            "{what}: the row holds the container laid in extent"
                        );
                    }
                    let plan = planned(&state, disk, node, &keys);
                    planned_n += 1;
                    // ⚠ The archive is gigabytes, so the patched copy is not written: the plan's own
                    // bytes laid over the archive read the edit back.
                    let line = match (plan.writes(), &in_extent) {
                        (Writes::Patched, _) => {
                            let read = overlaid(disk, &keys, &plan, offset, &body);
                            if let Err(why) = read {
                                panic!("{what}: Patched, but {why}");
                            }
                            format!("{what}: Patched, and the plan's bytes over the archive read the edit back")
                        }
                        (Writes::AsRead, _) => {
                            format!("{what}: the re-encode wrote the bytes it held")
                        }
                        (Writes::Rebuilt, tried) => {
                            assert!(!plan.kept.is_empty(), "{what}: rebuilt with no reason");
                            let refused =
                                (tried.as_ref().err()).map_or("laid in extent", String::as_str);
                            format!(
                                "{what}: plans a rebuilt archive ({refused}); {}",
                                plan.kept.join("; ")
                            )
                        }
                    };
                    said.push(line);
                }
            }
        }
        for line in &said {
            println!("{line}");
        }
        println!(
            "{} streams re-encoded, {fits} laid in their extents and {grows} grown past them; \
             {planned_n} planned of {tries} tried in {listed} nested archives",
            fits + grows
        );
        assert!(
            planned_n > 0,
            "no stream of the build's audio reached a plan: {}",
            said.join("; ")
        );
    }
}
