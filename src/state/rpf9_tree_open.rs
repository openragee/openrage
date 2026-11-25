//! A file of a GTA VI archive's tree opened by its table row: a clear row's bytes read through its
//! chunk table on the load worker, under the kind a scan of its start names.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use or_io::container::rpf::{
    open_at, read_rpf9_body, Gta6Mount, GtaViKeys, Rpf9BodyKind, Rpf9Content, Rpf9Place,
    Rpf9ScanError, Rpf9ScanFind, Rpf9ZlibRun, RPF9_SLOT,
};
use or_io::gta6::other_bodies::OtherBody;

use crate::state::document::{DocumentId, HexDocument, MetaDocument, RpfNodeId};
use crate::state::{document_io, AppState, DocPayload, RpfEntryBinding};

/// Where the worker finds the keys the archive's tables open with.
pub(in crate::state) type Keys = fn() -> Result<Arc<GtaViKeys>, String>;

/// The most a row opened from the tree inflates to, as for a body opened from a scan's list.
const LIMIT: u64 = 1 << 30;

/// A clear row of the archive whose header is at `base`, read by its table when its path opens.
#[derive(Clone, Copy)]
struct Row {
    base: u64,
    entry: u32,
    keys: Keys,
}

/// The rows the tree left for the load worker, by the path their body opens under.
// ponytail: one map for the whole app, which no close empties; a row is a few words, and it stays
// so a reload of the body reads by its table again.
static ROWS: Mutex<BTreeMap<PathBuf, Row>> = Mutex::new(BTreeMap::new());

fn rows() -> MutexGuard<'static, BTreeMap<PathBuf, Row>> {
    ROWS.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Leave the read the load worker opens `path`, the body of `place`, with: a ciphered row's decipher, or a clear row's table; a headerless resource's row, whose body starts past the slot at `row_start`, reads as a scan does.
pub(in crate::state) fn leave(path: &Path, place: &Rpf9Place, row_start: u64, keys: Keys) {
    if place.ciphered {
        crate::state::rpf9_scan::remember_ciphered(path, place.base, place.entry);
    } else if place.offset == row_start {
        let row = Row {
            base: place.base,
            entry: place.entry,
            keys,
        };
        rows().insert(path.to_path_buf(), row);
    }
}

/// A row the tree opened, by the path its body opens under: the path its document shows, as a
/// GTA V entry's does, and the row.
// ponytail: one map for the whole app, which no close empties, as the rows above.
static NAMED: Mutex<BTreeMap<PathBuf, (PathBuf, RpfEntryBinding)>> = Mutex::new(BTreeMap::new());

fn named() -> MutexGuard<'static, BTreeMap<PathBuf, (PathBuf, RpfEntryBinding)>> {
    NAMED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Name the body `path` of the tree row `node` of `parent` as a GTA V entry's document is named: the archive's path joined with the row's path in the tree, its hash name where it has no name.
pub(in crate::state) fn name(state: &AppState, path: &Path, parent: DocumentId, node: RpfNodeId) {
    let Some(doc) = state.document(parent) else {
        return;
    };
    let DocPayload::Rpf(rpf) = &doc.payload else {
        return;
    };
    let Some(inner) = rpf.session.path_of(node) else {
        return;
    };
    let shown = crate::state::tabs::entry_display_path(doc.path.clone(), &inner);
    let row = RpfEntryBinding {
        parent_doc_id: parent,
        entry_id: node,
    };
    named().insert(path.to_path_buf(), (shown, row));
}

/// The path the body at `path` shows once the tree opened it; `None` for any other path.
pub(in crate::state) fn shown(path: &Path) -> Option<PathBuf> {
    named().get(path).map(|(shown, _)| shown.clone())
}

/// The tree row the body at `path` was opened from, while the tree holding it is open.
pub(in crate::state) fn row_of(state: &AppState, path: &Path) -> Option<RpfEntryBinding> {
    let row = named().get(path).map(|(_, row)| *row)?;
    state.document(row.parent_doc_id).is_some().then_some(row)
}

/// What a row's read gives the views.
// One is made per open, on the load worker, and taken apart at once.
#[allow(clippy::large_enum_variant)]
pub(in crate::state) enum TableRead {
    /// The body a scan of the row's start names, with the bytes the row states.
    Body(Rpf9ScanFind, Vec<u8>),
    /// Bytes whose start no scan names a body of, opened by their head.
    Plain(DocPayload),
}

/// The read of the row the tree left for `path`, whose body is at `offset` of the file at `archive`; `None` for a path no row was left for.
pub(in crate::state) fn read(
    path: &Path,
    archive: &Path,
    offset: u64,
) -> Option<Result<TableRead, String>> {
    let row = rows().get(path).copied()?;
    Some(read_row(path, archive, offset, row))
}

fn read_row(path: &Path, archive: &Path, offset: u64, row: Row) -> Result<TableRead, String> {
    let keys = (row.keys)()?;
    let mut file = File::open(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let mount = Gta6Mount {
        base: row.base,
        archive: open_at(&mut file, row.base, &keys).map_err(|e| e.to_string())?,
    };
    let entry = (mount.archive.entries.iter())
        .find(|e| e.index == row.entry)
        .ok_or_else(|| tr!("entry {entry} is not in this table", entry = row.entry))?;
    let scanned = read_rpf9_body(&file, offset, LIMIT);
    // A row that states no sizes, and a resource, whose block map ends it, read as a scan does.
    let resource = scanned.as_ref().is_ok_and(|(find, _)| is_resource(find));
    let Some((stored, real)) = (entry.stored.zip(entry.real)).filter(|_| !resource) else {
        let read = scanned.map(|(find, bytes)| TableRead::Body(find, bytes));
        return read.map_err(|e| e.to_string());
    };
    if real > LIMIT {
        return Err(Rpf9ScanError::InflateLimit {
            offset,
            limit: LIMIT,
        }
        .to_string());
    }
    // ⚠ A scan's read ends at a chunk stored raw with no error; the table states every chunk.
    let bytes = mount
        .read_entry(&mut file, entry)
        .map_err(|e| e.to_string())?;
    Ok(match scanned {
        Ok((mut find, _)) => {
            stated(&mut find, stored, &bytes);
            TableRead::Body(find, bytes)
        }
        Err(_) => TableRead::Plain(by_head(path, bytes)),
    })
}

/// Whether `find` is a headerless resource, whose read is its two sections.
fn is_resource(find: &Rpf9ScanFind) -> bool {
    matches!(
        find.kind,
        Rpf9BodyKind::Zlib(Rpf9ZlibRun {
            resource: Some(_),
            ..
        })
    )
}

/// `find` as its row states it: the stored length, and the inflated length of the whole body `bytes`.
fn stated(find: &mut Rpf9ScanFind, stored: u64, bytes: &[u8]) {
    let len = bytes.len() as u64;
    if let Rpf9BodyKind::Zlib(run) = &mut find.kind {
        run.inflated = len;
        // A movie states the whole run's length, which a read cut short of it does not reach.
        if run.content == Rpf9Content::Other {
            if let Some(body @ OtherBody::Gfx) = OtherBody::of(bytes, len) {
                run.content = Rpf9Content::Identified(body);
            }
        }
    }
    find.stored_len = Some(stored);
    find.inflated_len = Some(len);
    find.span = find.span.max(stored.next_multiple_of(RPF9_SLOT));
}

/// `bytes` opened by their head: metadata, text, else its bytes under the reason none reads them.
fn by_head(path: &Path, bytes: Vec<u8>) -> DocPayload {
    if bytes.starts_with(b"PSIN") || bytes.starts_with(b"RBF0") {
        return DocPayload::Meta(MetaDocument::from_bytes(bytes));
    }
    if document_io::is_text(&bytes) {
        return crate::state::rpf9_scan::gta6_data_xml::payload(path, bytes);
    }
    let banner = document_io::no_reader(path);
    DocPayload::Hex(HexDocument { bytes, banner })
}

#[cfg(test)]
#[path = "rpf9_tree_open_names_tests.rs"]
mod names_tests;

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use or_io::container::rpf::gta6_chunk_table::entry_chunks;
    use or_io::container::rpf::gta6_container_write::{build, Rpf9IndexKey, Rpf9NewBody};
    use or_io::container::rpf::{write_rpf9_resource, Rpf9Form, Rpf9Sections};
    use or_io::gta6::hash::name_hash64;

    use super::*;
    use crate::state::opening_an_rpf9_archive::headerless_rbm;
    use crate::state::rpf9_scan::{body_path, read_body, scan_origin, scanned_body, view_of, View};

    const REGISTRY_ID: u64 = 0xDEAD_BEEF_0200_0000;
    const KEY_ID: u32 = 0xDEAD_BEEF;
    const BOOTSTRAP_REGISTRY_ID: u64 = 0x1000_0001_0200_0000;
    /// The round tables feeding each output lane of the cipher.
    const LANES: [[usize; 4]; 4] = [[0, 13, 10, 7], [1, 14, 4, 11], [5, 2, 8, 15], [9, 6, 12, 3]];
    const WORDS: usize = 60;
    const MAPS: usize = 16;
    const MAP_LEN: usize = 256;
    const BLOB_LEN: usize = 4 + WORDS * 4 + MAPS * MAP_LEN;
    /// The bytes a chunk of a body the archive writer chunks inflates to.
    const CHUNK: usize = 64 << 10;
    const BODY: &str = "data/body.bin";

    fn push_perm(out: &mut Vec<u8>, k: usize) {
        out.extend((0..256usize).map(|x| ((x + k * 7 + 1) % 256) as u8));
    }

    fn mix8(b: u8) -> u8 {
        let (mut y, mut acc) = (0u8, 0u8);
        for i in (0..8).rev() {
            acc ^= (b >> i) & 1;
            y |= acc << i;
        }
        y
    }

    /// A synthetic white-box snapshot holding the bootstrap key and one archive key.
    fn snapshot() -> Vec<u8> {
        let mut slot_of = [0usize; 16];
        for lane in LANES.iter() {
            for (slot, &k) in lane.iter().enumerate() {
                slot_of[k] = slot;
            }
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"ORGTAVIK");
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
                let word = constant ^ (u32::from(mix8(s)) << (slot * 8));
                out.extend_from_slice(&word.to_le_bytes());
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
        out
    }

    fn synthetic_keys() -> Result<Arc<GtaViKeys>, String> {
        GtaViKeys::parse(&snapshot())
            .map(Arc::new)
            .map_err(|e| e.to_string())
    }

    /// Bytes that deflate well, start no format a scan names and hold control bytes, so no text.
    fn pattern(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 97) as u8 + 1).collect()
    }

    /// An archive of `entries` built from nothing under the synthetic key, written in `dir`.
    fn archive(dir: &Path, entries: Vec<(String, Rpf9NewBody)>) -> PathBuf {
        let key = Rpf9IndexKey {
            key_id: KEY_ID,
            iv: [0x33; 16],
        };
        let keys = synthetic_keys().expect("the synthetic snapshot parses");
        let plan = build(&entries, &key, &keys).expect("the archive builds");
        let mut bytes = Vec::new();
        plan.write_to(&mut Cursor::new(Vec::<u8>::new()), &mut bytes)
            .expect("the archive is laid out");
        let path = dir.join("source.rpf");
        std::fs::write(&path, bytes).expect("the archive is written");
        path
    }

    /// The top-level table of the archive at `disk` and the place in it of the row named `name`.
    fn row(disk: &Path, name: &str) -> (Gta6Mount, usize) {
        let keys = synthetic_keys().expect("the synthetic snapshot parses");
        let mut file = File::open(disk).expect("the archive");
        let mount = Gta6Mount::open_root(&mut file, &keys).expect("the archive opens");
        let at = (mount.archive.entries.iter())
            .position(|e| e.name64 == name_hash64(name.as_bytes()))
            .unwrap_or_else(|| panic!("no entry {name}"));
        (mount, at)
    }

    /// The row `name` of `disk` as the tree leaves it and the load worker reads it: its path, the
    /// row's start, and what opened.
    fn opened(disk: &Path, name: &str) -> (PathBuf, u64, Result<DocPayload, String>) {
        let (mount, at) = row(disk, name);
        let entry = &mount.archive.entries[at];
        let start = mount.absolute_start(entry).expect("a start");
        let slot = entry.stored.is_none();
        let offset = start + if slot { RPF9_SLOT } else { 0 };
        let place = Rpf9Place {
            file: disk.to_path_buf(),
            base: 0,
            offset,
            entry: entry.index,
            ciphered: false,
        };
        let path = body_path(disk, offset, View::Bytes);
        leave(&path, &place, start, synthetic_keys);
        let payload = read_body(&path, disk, offset);
        (path, start, payload)
    }

    /// A body of several chunks opens whole by its table. The archive writer stores no chunk raw, so
    /// the raw chunk a scan stops at is met only in the install-gated test below.
    #[test]
    fn a_row_of_several_chunks_opens_by_its_table_at_its_whole_real_size() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let body = [pattern(CHUNK), pattern(CHUNK), pattern(4000)].concat();
        let disk = archive(
            dir.path(),
            vec![(BODY.to_owned(), Rpf9NewBody::Chunked(body.clone()))],
        );
        let (mount, at) = row(&disk, BODY);
        let entry = &mount.archive.entries[at];
        let chunks = entry_chunks(&mount.archive, entry.index as usize).expect("a table");
        assert_eq!(
            chunks.expect("its chunks").len(),
            3,
            "the table states three chunks"
        );
        let start = mount.absolute_start(entry).expect("a start");

        let (path, _, payload) = opened(&disk, BODY);
        let hex = match payload {
            Ok(DocPayload::Hex(hex)) => hex,
            Ok(_) => panic!("a body no view reads opens as its bytes"),
            Err(e) => panic!("the row opens: {e}"),
        };
        assert_eq!(hex.bytes, body, "the whole body opens");
        let (find, _) = scan_origin(&path).expect("its origin is kept");
        assert_eq!(find.inflated_len, Some(body.len() as u64));
        assert_eq!(
            scanned_body(&path),
            Some((disk.as_path(), start)),
            "a save finds the archive and the row's body again"
        );
    }

    #[test]
    fn a_headerless_row_opens_as_its_kind_past_its_slot() {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let mut virtual_section = headerless_rbm(0x5000_00E0);
        virtual_section.resize(0x110, 0);
        let loose = write_rpf9_resource(&Rpf9Sections {
            form: Rpf9Form::BlockMapped,
            virtual_section,
            physical: Vec::new(),
        })
        .expect("a loose resource");
        let resource = Rpf9NewBody::Resource {
            type_code: 0x02,
            loose,
        };
        let disk = archive(dir.path(), vec![(BODY.to_owned(), resource)]);
        let (path, start, payload) = opened(&disk, BODY);
        assert!(
            read(&path, &disk, start + RPF9_SLOT).is_none(),
            "a headerless row is read as a scan reads it"
        );
        let payload = payload.unwrap_or_else(|e| panic!("the row opens: {e}"));
        assert!(
            !matches!(payload, DocPayload::Hex(_)),
            "it opens in its view, not as bytes"
        );
        let (find, _) = scan_origin(&path).expect("its origin is kept");
        assert!(view_of(&find).is_ok(), "its kind has a view");
    }

    /// One row of each shape of the build opens through the tree's door at the size its table
    /// states: a clear row whose table stores a chunk raw, a clear headerless resource's row, and a
    /// ciphered row. Install-gated: it needs the key snapshot and the build, and skips without them.
    #[test]
    fn a_row_of_each_shape_of_the_build_opens_at_the_size_its_table_states() {
        use crate::state::rpf9_scan::{decipher_entry, rpf9_table};

        const BUILD_ENV: &str = "OPENRAGE_GTA6_BUILD";
        /// A row larger than this is passed over: a big body would inflate for seconds.
        const CAP: u64 = 1 << 20;
        const MAX_DEPTH: usize = 3;
        const MAX_TRIES: usize = 400;

        let root = std::env::var_os(BUILD_ENV)
            .map(PathBuf::from)
            .filter(|p| p.is_dir());
        let keys = root.as_ref().and_then(|_| rpf9_table::keys().ok());
        let (Some(root), Some(keys)) = (root, keys) else {
            or_test_support::skip_install(
                "gta6 tree row open",
                format_args!("set OPENRAGE_GTA6_KEYS and {BUILD_ENV}"),
            );
            return;
        };
        let mut disks: Vec<PathBuf> = (std::fs::read_dir(&root).into_iter().flatten().flatten())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("rpf"))
            .collect();
        disks.sort();
        // Still to open: a chunked clear row, a headerless clear row, a ciphered row.
        let mut left = [true; 3];
        let mut tries = 0usize;
        'disks: for disk in disks {
            let Ok(mut file) = File::open(&disk) else {
                continue;
            };
            // Each archive's header offset, mounted only when its turn comes.
            let mut stack = vec![(0u64, 0usize)];
            while let Some((base, depth)) = stack.pop() {
                let Ok(archive) = open_at(&mut file, base, &keys) else {
                    continue;
                };
                let mount = Gta6Mount { base, archive };
                for (at, entry) in mount.archive.entries.iter().enumerate() {
                    if !left.contains(&true) || tries >= MAX_TRIES {
                        break 'disks;
                    }
                    let Some(start) = mount.absolute_start(entry) else {
                        continue;
                    };
                    if mount
                        .entry_begins_archive(&mut file, entry)
                        .unwrap_or(false)
                    {
                        if depth + 1 < MAX_DEPTH {
                            stack.push((start, depth + 1));
                        }
                        continue;
                    }
                    let nested = mount.archive.entry_extra(at).and_then(|x| x.nested);
                    let slot = entry.stored.is_none() && nested.is_some();
                    let size = entry.real.or(nested.map(|n| n.stored_size()));
                    if !size.is_some_and(|s| s > 0 && s <= CAP) {
                        continue;
                    }
                    let ciphered = entry.cipher.is_some();
                    let chunked = !ciphered
                        && entry.stored.is_some()
                        && entry_chunks(&mount.archive, entry.index as usize).is_ok_and(|t| {
                            t.is_some_and(|t| t.len() > 1 && t.iter().any(|c| !c.zlib))
                        });
                    let content = entry.cipher.as_ref().is_some_and(|c| c.cipher_type == 17);
                    let shape = match (ciphered, slot, chunked) {
                        (true, _, _) if content => 2,
                        (false, true, _) => 1,
                        (false, false, true) => 0,
                        _ => continue,
                    };
                    if !left[shape] {
                        continue;
                    }
                    tries += 1;
                    let offset = start + if slot { RPF9_SLOT } else { 0 };
                    let place = Rpf9Place {
                        file: disk.clone(),
                        base: mount.base,
                        offset,
                        entry: entry.index,
                        ciphered,
                    };
                    let path = body_path(&disk, offset, View::Bytes);
                    match shape {
                        0 => {
                            let real = entry.real.expect("a chunked row states its size");
                            leave(&path, &place, start, rpf9_table::keys);
                            let len = match read(&path, &disk, offset) {
                                Some(Ok(TableRead::Body(f, _))) if is_resource(&f) => continue,
                                Some(Ok(TableRead::Body(_, bytes))) => Some(bytes.len()),
                                Some(Ok(TableRead::Plain(payload))) => match payload {
                                    DocPayload::Hex(hex) => Some(hex.bytes.len()),
                                    _ => None,
                                },
                                Some(Err(e)) => {
                                    panic!("{}: row {} reads: {e}", disk.display(), entry.index)
                                }
                                None => panic!("the tree left a clear row's read"),
                            };
                            if let Some(len) = len {
                                let at = entry.index;
                                assert_eq!(len as u64, real, "the whole body of row {at}");
                            }
                        }
                        1 => {
                            leave(&path, &place, start, rpf9_table::keys);
                            assert!(read(&path, &disk, offset).is_none());
                            if read_body(&path, &disk, offset).is_err() {
                                continue;
                            }
                            let stated = nested.map_or(0, |n| n.stored_size());
                            if let Some((find, _)) = scan_origin(&path) {
                                assert!(
                                    find.stored_len.is_none_or(|s| s + RPF9_SLOT <= stated),
                                    "row {}'s body lies inside the extent its row states",
                                    entry.index
                                );
                            }
                        }
                        _ => {
                            let Ok(bytes) = decipher_entry(&disk, mount.base, entry.index) else {
                                continue;
                            };
                            if let Some(real) = entry.real {
                                assert_eq!(bytes.len() as u64, real, "row {}", entry.index);
                            }
                        }
                    }
                    // The read works, so the open does.
                    leave(&path, &place, start, rpf9_table::keys);
                    if let Err(e) = read_body(&path, &disk, offset) {
                        panic!("{}: row {} opens: {e}", disk.display(), entry.index);
                    }
                    left[shape] = false;
                }
            }
        }
        if left.contains(&true) {
            or_test_support::skip_install(
                "gta6 tree row open",
                format_args!("no row of the shapes still open {left:?} within {MAX_TRIES} tries"),
            );
        }
    }
}
