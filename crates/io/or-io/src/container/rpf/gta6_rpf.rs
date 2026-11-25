//! RPF9, GTA VI's archive, behind [`RpfArchive`]: its enciphered column table read into the entries
//! and names every other generation gives, so one tree, one read and one nested open serve it too.
//!
//! An RPF9 has no directory records and no names: an entry carries the hash of its lower-case path.
//! Where the user's names resolve a hash, the path's folders are made here; where they do not, the
//! entry sits at the root under its hash. Read-only: [`super::gta6_container_write`] writes a copy.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fs::File;
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use super::archive::{RpfArchive, RpfSource};
use super::crypto::TocDecryptOutcome;
use super::entry::{Compressor, RpfBinaryEntry, RpfDirectoryEntry, RpfEntry};
use super::gta6_archive::{Gta6Class, Gta6Entry, NameResolver};
use super::gta6_mount::{open_at_bounded, Gta6Mount};
use super::header::{RpfEncryption, RpfHeader};
use super::layout::{Input, Toc};
use super::reader::{read_at, BodyForm};
use super::rpf9_scan::RPF9_SLOT;
use super::rpf9_toc::GtaViKeys;
use crate::error::{IoError, Result};

/// The environment variable naming a folder of GTA VI names: a vocabulary and name tables.
pub const GTA6_NAMES_ENV: &str = "OPENRAGE_GTA6_NAMES";

/// The longest name a folder or file is given, in bytes.
const MAX_NAME_BYTES: usize = 255;

/// What reading a GTA VI archive needs beyond its bytes: the keys, and the names its hashes stand for.
pub struct Gta6Context {
    pub keys: Arc<GtaViKeys>,
    pub names: Arc<NameResolver>,
}

/// The context a caller gave the process, which archives opened after it are read with.
static GIVEN: RwLock<Option<Arc<Gta6Context>>> = RwLock::new(None);

impl Gta6Context {
    /// Give the process the keys and names GTA VI archives are read with, in place of the environment's.
    pub fn give(context: Gta6Context) {
        *GIVEN.write().unwrap_or_else(|held| held.into_inner()) = Some(Arc::new(context));
    }

    /// The context a caller gave, else the environment's; `None` with no keys.
    pub fn current() -> Result<Option<Arc<Self>>> {
        let given = GIVEN
            .read()
            .unwrap_or_else(|held| held.into_inner())
            .clone();
        match given {
            Some(context) => Ok(Some(context)),
            None => Self::from_environment(),
        }
    }

    /// The keys and names the environment gives, read once for the process; `None` with no keys.
    pub fn from_environment() -> Result<Option<Arc<Self>>> {
        static CONTEXT: OnceLock<std::result::Result<Option<Arc<Gta6Context>>, String>> =
            OnceLock::new();
        CONTEXT
            .get_or_init(|| {
                let keys = GtaViKeys::for_install(None).map_err(|e| e.to_string())?;
                Ok(keys.map(|keys| {
                    let names = std::env::var_os(GTA6_NAMES_ENV)
                        .map(PathBuf::from)
                        .filter(|dir| dir.is_dir())
                        .map_or_else(NameResolver::new, |dir| NameResolver::load(&dir));
                    Arc::new(Gta6Context {
                        keys: Arc::new(keys),
                        names: Arc::new(names),
                    })
                }))
            })
            .clone()
            .map_err(|why| IoError::Invalid {
                what: "the GTA VI key snapshot".into(),
                why,
            })
    }
}

/// One file of an RPF9 as the tree shows it.
struct Leaf {
    /// The folders it sits under, then its own name.
    segments: Vec<String>,
    /// Its start from the archive's header, where its row states one.
    offset: Option<u64>,
    stored: u64,
    real: u64,
    /// A nested archive, which opens as a folder.
    archive: bool,
    /// Its row is the 20-byte one of a headerless resource, whose body follows a 16-byte zero slot.
    slot: bool,
    ciphered: bool,
}

/// An opened RPF9's decrypted table and what its bodies are read with.
pub(super) struct Rpf9State {
    mount: Gta6Mount,
    context: Arc<Gta6Context>,
    /// For each entry of the tree, its leaf; `u32::MAX` for a folder.
    entry_of: Vec<u32>,
    leaves: Vec<Leaf>,
}

/// Read an RPF9's table into a tree of entries, with the state its bodies are read through.
pub(super) fn read(input: &Input) -> Result<(Toc, Rpf9State)> {
    let context = Gta6Context::current()?.ok_or(IoError::Rpf9TocEncrypted)?;
    let mut src = bytes_of(input.source)?;
    let end = input
        .start
        .checked_add(input.size)
        .ok_or_else(|| IoError::RpfBadHeader {
            message: "RPF9 archive extent overflows".into(),
        })?;
    let mut archive = open_at_bounded(&mut src, input.start..end, &context.keys)?;
    archive.resolve_names(&context.names);
    let mount = Gta6Mount {
        base: input.start,
        archive,
    };
    let leaves: Vec<Leaf> = (mount.archive.entries.iter().enumerate())
        .map(|(at, entry)| leaf(&mount, at, entry))
        .collect();
    let (entries, names, entry_of) = lay_out(&leaves);
    let header = RpfHeader {
        entry_count: entries.len() as u32,
        name_heap_size: 0,
        name_shift: 0,
        xcompress: false,
        // The table's cipher is this module's own; no provider is asked for it.
        encryption: RpfEncryption::None,
    };
    let toc = Toc {
        header,
        entries,
        names,
        name_heap: Vec::new(),
        outcome: TocDecryptOutcome::Nominal,
    };
    let state = Rpf9State {
        mount,
        context,
        entry_of,
        leaves,
    };
    Ok((toc, state))
}

fn leaf(mount: &Gta6Mount, at: usize, entry: &Gta6Entry) -> Leaf {
    let row = mount.archive.entry_extra(at).and_then(|extra| extra.nested);
    let stored = (entry.stored)
        .or(row.map(|row| row.stored_size()))
        .unwrap_or(0);
    // A 20-byte row states its two sections' padded sizes, each over a nibble of the version.
    let sections = |row: super::gta6_archive::Gta6NestedRow| {
        u64::from(row.virtual_size & !0xF) + u64::from(row.flags & !0xF)
    };
    let real = entry.real.or(row.map(sections)).unwrap_or(0);
    let archive = entry.kind() == Some(Gta6Class::NestedArchive);
    Leaf {
        segments: segments(entry, archive),
        offset: entry.start,
        stored,
        real,
        archive,
        ciphered: entry.cipher.is_some(),
        slot: entry.stored.is_none() && row.is_some(),
    }
}

/// The folders and file name an entry sits under: its recovered path where that is a plain relative
/// one, else its hash; a nested archive's name ends in `.rpf`, which is what opens it as a folder.
fn segments(entry: &Gta6Entry, archive: bool) -> Vec<String> {
    let mut parts = (entry.name.as_deref())
        .and_then(path_parts)
        .unwrap_or_else(|| vec![hash_name(entry)]);
    if let Some(last) = parts.last_mut().filter(|_| archive) {
        if !last.to_ascii_lowercase().ends_with(".rpf") {
            last.push_str(".rpf");
        }
    }
    parts
}

fn hash_name(entry: &Gta6Entry) -> String {
    match entry.name64 {
        // The few entries with no name row have only their place in the table.
        0 => format!("entry_{}", entry.index),
        hash => format!("hash_{hash:016x}"),
    }
}

/// A recovered path's parts, or `None` when it is not a plain relative path.
fn path_parts(path: &str) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    for part in path.split(['/', '\\']).map(str::trim) {
        if part.is_empty() || part == "." {
            continue;
        }
        // A name is not let out of the archive's tree, whatever the vocabulary holds.
        if part == ".." || part.chars().any(|c| c == ':' || c.is_control()) {
            return None;
        }
        let cut = (0..=part.len().min(MAX_NAME_BYTES))
            .rev()
            .find(|&at| part.is_char_boundary(at))
            .unwrap_or(0);
        parts.push(part[..cut].to_owned());
    }
    (!parts.is_empty()).then_some(parts)
}

#[derive(Default)]
struct Folder {
    folders: BTreeMap<String, Folder>,
    files: Vec<(String, u32)>,
}

/// The entries, names and leaf of each entry of a tree whose folders each hold a run of entries, as
/// every generation's table does: the root at 0, then each folder's children together.
fn lay_out(leaves: &[Leaf]) -> (Vec<RpfEntry>, Vec<String>, Vec<u32>) {
    let mut root = Folder::default();
    for (at, leaf) in leaves.iter().enumerate() {
        let Some((file, folders)) = leaf.segments.split_last() else {
            continue;
        };
        let mut folder = &mut root;
        for name in folders {
            folder = folder.folders.entry(name.clone()).or_default();
        }
        folder.files.push((file.clone(), at as u32));
    }

    let directory = |first: usize, count: usize| {
        RpfEntry::Directory(RpfDirectoryEntry {
            name_offset: 0,
            first_entry_index: first as u32,
            entry_count: count as u32,
        })
    };
    let mut entries = vec![directory(0, 0)];
    let mut names = vec![String::new()];
    let mut entry_of = vec![u32::MAX];
    let mut queue = VecDeque::from([(&root, 0usize)]);
    while let Some((folder, at)) = queue.pop_front() {
        let first = entries.len();
        let mut taken: HashSet<&str> = HashSet::new();
        for (name, sub) in &folder.folders {
            queue.push_back((sub, entries.len()));
            entries.push(directory(0, 0));
            names.push(name.clone());
            entry_of.push(u32::MAX);
            taken.insert(name);
        }
        let mut files: Vec<&(String, u32)> = folder.files.iter().collect();
        files.sort();
        for (name, leaf_at) in files {
            // Two entries under one name, or a file named as a folder beside it, are told apart by
            // their place in the table rather than one hiding the other.
            let name = match taken.insert(name) {
                true => name.clone(),
                false => numbered(name, *leaf_at),
            };
            entries.push(file_entry(&leaves[*leaf_at as usize]));
            names.push(name);
            entry_of.push(*leaf_at);
        }
        entries[at] = directory(first, entries.len() - first);
    }
    (entries, names, entry_of)
}

/// `name` with the entry's place in the table, before the `.rpf` that makes an archive a folder.
fn numbered(name: &str, at: u32) -> String {
    match name.strip_suffix(".rpf") {
        Some(stem) => format!("{stem}#{at}.rpf"),
        None => format!("{name}#{at}"),
    }
}

fn file_entry(leaf: &Leaf) -> RpfEntry {
    // ponytail: the entry's sizes are 32-bit, so one past 4 GiB shows as 4 GiB; a read and a nested
    // open take the table's own 64-bit sizes. Widen the entry if a listing must show them.
    let clamp = |size: u64| u32::try_from(size).unwrap_or(u32::MAX);
    RpfEntry::Binary(RpfBinaryEntry {
        name_offset: 0,
        // A nested archive is kept as it is; any other body states a stored and a real size.
        consumed_size: if leaf.archive { 0 } else { clamp(leaf.stored) },
        file_offset: leaf.offset.unwrap_or(0),
        uncompressed_size: clamp(if leaf.archive { leaf.stored } else { leaf.real }),
        encrypted: leaf.ciphered,
        encrypted_raw: 0,
        compressor: if leaf.archive {
            Compressor::None
        } else {
            Compressor::Deflate
        },
        tag: 0,
    })
}

fn no_body(entry_index: u32) -> IoError {
    IoError::RpfBadEntry {
        message: format!("entry {entry_index} has no body"),
    }
}

impl RpfArchive {
    /// The leaf an RPF9's entry stands for, with its row and the state it is read through.
    fn rpf9_leaf(&self, entry_index: u32) -> Result<(&Rpf9State, &Leaf, &Gta6Entry)> {
        let state = self.rpf9.as_deref().ok_or_else(|| no_body(entry_index))?;
        let at = (state.entry_of.get(entry_index as usize).copied())
            .filter(|at| *at != u32::MAX)
            .ok_or_else(|| no_body(entry_index))? as usize;
        Ok((state, &state.leaves[at], &state.mount.archive.entries[at]))
    }

    /// An RPF9 entry's body: deciphered and inflated, deciphered only, or as the archive holds it.
    pub(super) fn rpf9_form(&self, entry_index: u32, form: BodyForm) -> Result<Vec<u8>> {
        let (state, leaf, entry) = self.rpf9_leaf(entry_index)?;
        let keys = &state.context.keys;
        match form {
            BodyForm::Decoded if !leaf.archive => {
                let mut src = bytes_of(&self.source)?;
                let end = self
                    .start_pos
                    .checked_add(self.size)
                    .ok_or_else(|| no_body(entry_index))?;
                state
                    .mount
                    .read_entry_plan(entry.index, end, u64::MAX)?
                    .read(&mut src, keys)
            }
            BodyForm::Decrypted if leaf.ciphered => {
                let mut src = bytes_of(&self.source)?;
                let end = self
                    .start_pos
                    .checked_add(self.size)
                    .ok_or_else(|| no_body(entry_index))?;
                state
                    .mount
                    .read_entry_deciphered_stored_bounded(&mut src, entry, keys, end)
            }
            _ => self.rpf9_stored(entry_index, leaf),
        }
    }

    fn rpf9_stored(&self, entry_index: u32, leaf: &Leaf) -> Result<Vec<u8>> {
        let offset = leaf.offset.ok_or_else(|| no_body(entry_index))?;
        let table_len = self
            .rpf9
            .as_deref()
            .ok_or_else(|| no_body(entry_index))?
            .mount
            .archive
            .toc
            .bytes
            .len() as u64;
        if offset < table_len {
            return Err(no_body(entry_index));
        }
        let len = usize::try_from(leaf.stored)
            .ok()
            .filter(|_| offset.saturating_add(leaf.stored) <= self.size)
            .ok_or(IoError::UnexpectedEof {
                offset,
                needed: usize::try_from(leaf.stored).unwrap_or(usize::MAX),
                available: usize::try_from(self.size.saturating_sub(offset)).unwrap_or(usize::MAX),
            })?;
        let mut out = vec![0u8; len];
        read_at(&self.source, self.start_pos + offset, &mut out)?;
        Ok(out)
    }

    /// The RPF9 nested as file `entry_index` of this one, which reads through it.
    pub(super) fn rpf9_nested(self: &Arc<Self>, entry_index: u32) -> Result<RpfArchive> {
        let (state, leaf, _) = self.rpf9_leaf(entry_index)?;
        let offset =
            (leaf.offset.filter(|_| leaf.archive)).ok_or_else(|| IoError::RpfBadEntry {
                message: format!("entry {entry_index} is not a nested archive"),
            })?;
        if offset < state.mount.archive.toc.bytes.len() as u64
            || offset
                .checked_add(leaf.stored)
                .is_none_or(|end| end > self.size)
        {
            return Err(no_body(entry_index));
        }
        let start = self
            .start_pos
            .checked_add(offset)
            .ok_or_else(|| no_body(entry_index))?;
        let name = (self.node_for_entry(entry_index)).map(|node| node.name().to_string());
        Self::parse(
            RpfSource::Nested(Arc::clone(self)),
            start,
            leaf.stored,
            name,
            Arc::clone(&self.keys),
            None,
        )
    }
}

/// Where an RPF9 entry's body sits in the outermost file, for a reader that takes a file and an offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rpf9Place {
    pub file: PathBuf,
    /// The absolute offset of the header of the archive that holds the entry.
    pub base: u64,
    /// The absolute offset of the entry's body.
    pub offset: u64,
    /// The entry's number in its archive's table.
    pub entry: u32,
    pub ciphered: bool,
}

/// A leaf's original table indices from its outermost file through every containing archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rpf9EntryAddress {
    pub file: PathBuf,
    pub indices: Vec<u32>,
}

impl RpfArchive {
    /// Locate a leaf through already mounted parent tables, without reading any archive bytes.
    pub fn rpf9_entry_address(&self, entry_index: u32) -> Option<Rpf9EntryAddress> {
        let (_, leaf, entry) = self.rpf9_leaf(entry_index).ok()?;
        if leaf.archive || leaf.offset.is_none() {
            return None;
        }
        let mut indices = vec![entry.index];
        let mut current = self;
        loop {
            match &current.source {
                RpfSource::Memory(_) => return None,
                RpfSource::File(file) => {
                    indices.reverse();
                    return Some(Rpf9EntryAddress {
                        file: file.clone(),
                        indices,
                    });
                }
                RpfSource::Nested(parent) => {
                    let state = parent.rpf9.as_deref()?;
                    let offset = current.start_pos.checked_sub(parent.start_pos)?;
                    if offset.checked_add(current.size)? > parent.size {
                        return None;
                    }
                    let mut owners = state.leaves.iter().enumerate().filter(|(_, leaf)| {
                        leaf.archive && leaf.offset == Some(offset) && leaf.stored == current.size
                    });
                    let (at, _) = owners.next()?;
                    // Equal extents can be aliases; the parent pointer does not identify which was opened.
                    if owners.next().is_some() {
                        return None;
                    }
                    indices.push(state.mount.archive.entries.get(at)?.index);
                    current = parent;
                }
            }
        }
    }

    /// Where file `entry_index` of an RPF9 read from disk sits; `None` for a folder, a nested archive, bytes held in memory or any other generation.
    pub fn rpf9_place(&self, entry_index: u32) -> Option<Rpf9Place> {
        let (_, leaf, entry) = self.rpf9_leaf(entry_index).ok()?;
        let offset = leaf.offset.filter(|_| !leaf.archive)?;
        Some(Rpf9Place {
            file: file_of(&self.source)?.to_path_buf(),
            base: self.start_pos,
            offset: self.start_pos + offset + if leaf.slot { RPF9_SLOT } else { 0 },
            entry: entry.index,
            ciphered: leaf.ciphered,
        })
    }
}

/// The file on disk a source reads from, through any archives it is nested in.
fn file_of(source: &RpfSource) -> Option<&Path> {
    match source {
        RpfSource::File(path) => Some(path),
        RpfSource::Nested(parent) => file_of(&parent.source),
        RpfSource::Memory(_) => None,
    }
}

/// The bytes an archive's source reads from, at the offsets of the outermost file.
enum Bytes<'a> {
    File(File),
    Memory(Cursor<&'a [u8]>),
}

fn bytes_of(source: &RpfSource) -> Result<Bytes<'_>> {
    match source {
        RpfSource::Memory(bytes) => Ok(Bytes::Memory(Cursor::new(bytes.as_slice()))),
        RpfSource::File(path) => {
            (File::open(path).map(Bytes::File)).map_err(|source| IoError::FileReadFailed {
                path: path.clone(),
                source,
            })
        }
        RpfSource::Nested(parent) => bytes_of(&parent.source),
    }
}

impl Read for Bytes<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Bytes::File(file) => file.read(buf),
            Bytes::Memory(bytes) => bytes.read(buf),
        }
    }
}

impl Seek for Bytes<'_> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match self {
            Bytes::File(file) => file.seek(pos),
            Bytes::Memory(bytes) => bytes.seek(pos),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::reader::build_tree;
    use super::*;

    fn entry(index: u32, name: Option<&str>, class: u8) -> Gta6Entry {
        Gta6Entry {
            index,
            name64: 0xAB00 + u64::from(index),
            name32: None,
            name: name.map(str::to_owned),
            start: Some(0x1000 * u64::from(index + 1)),
            stored: Some(0x20),
            real: Some(0x30),
            class: Some(class),
            cipher: None,
        }
    }

    /// The leaves of `entries`, each a nested archive when its class says so.
    fn leaves(entries: &[Gta6Entry]) -> Vec<Leaf> {
        (entries.iter())
            .map(|e| {
                let archive = e.kind() == Some(Gta6Class::NestedArchive);
                Leaf {
                    segments: segments(e, archive),
                    offset: e.start,
                    stored: e.stored.unwrap_or(0),
                    real: e.real.unwrap_or(0),
                    archive,
                    ciphered: false,
                    slot: false,
                }
            })
            .collect()
    }

    /// Every path of the tree `entries` lay out as, through the walk every generation's table takes.
    fn paths(entries: &[Gta6Entry]) -> Vec<String> {
        let (table, names, entry_of) = lay_out(&leaves(entries));
        assert_eq!(table.len(), names.len());
        assert_eq!(table.len(), entry_of.len());
        let (nodes, _) = build_tree(&table, &names).expect("the laid-out table is a tree");
        let mut paths: Vec<String> = (nodes.iter().filter(|n| !n.is_dir()))
            .map(|n| n.path().to_owned())
            .collect();
        paths.sort();
        paths
    }

    #[test]
    fn a_recovered_path_makes_its_folders_and_an_unnamed_entry_sits_at_the_root_under_its_hash() {
        let found = paths(&[
            entry(0, Some("data/cars/one"), 0x0B),
            entry(1, Some("data/cars/two"), 0x0B),
            entry(2, Some("data/readme"), 0xFF),
            entry(3, None, 0x0B),
        ]);
        assert_eq!(
            found,
            [
                "data/cars/one",
                "data/cars/two",
                "data/readme",
                "hash_000000000000ab03"
            ]
        );
    }

    #[test]
    fn a_nested_archive_is_named_as_one_whether_or_not_its_name_resolved() {
        let found = paths(&[entry(0, Some("packs/base"), 0x00), entry(1, None, 0x00)]);
        assert_eq!(found, ["hash_000000000000ab01.rpf", "packs/base.rpf"]);
    }

    #[test]
    fn a_nested_archive_is_listed_from_its_class_before_any_body_exists() {
        let entry = entry(0, None, 0);
        let mount = Gta6Mount {
            base: 0,
            archive: super::super::Gta6Archive {
                toc: super::super::Rpf9Toc {
                    bytes: Vec::new(),
                    key_id: 0,
                    registry_id: 0,
                    entry_count: 1,
                    groups: Vec::new(),
                },
                entries: vec![entry.clone()],
            },
        };
        let listed = leaf(&mount, 0, &entry);
        assert!(listed.archive);
        assert!(listed.segments.last().unwrap().ends_with(".rpf"));
        let keys = GtaViKeys::parse(&or_test_support::rpf9::keys()).unwrap();
        assert!(mount
            .open_nested(&mut Cursor::new(Vec::<u8>::new()), &entry, &keys)
            .is_err());
    }

    #[test]
    fn a_name_that_would_leave_the_tree_falls_back_to_its_hash() {
        for bad in ["../outside", "a/../../b", "c:/windows/x", "a/b\u{7}c"] {
            let found = paths(&[entry(0, Some(bad), 0x0B)]);
            assert_eq!(found, ["hash_000000000000ab00"], "{bad}");
        }
        // Separators of either kind, doubled or leading, are only separators.
        let found = paths(&[entry(0, Some("/a//b\\c"), 0x0B)]);
        assert_eq!(found, ["a/b/c"]);
    }

    #[test]
    fn two_entries_under_one_name_and_a_file_named_as_a_folder_all_stay_listed() {
        let found = paths(&[
            entry(0, Some("a/b"), 0x0B),
            entry(1, Some("a/b"), 0x0B),
            entry(2, Some("a/b/c"), 0x0B),
            entry(3, Some("p"), 0x00),
            entry(4, Some("p"), 0x00),
        ]);
        assert_eq!(found, ["a/b#0", "a/b#1", "a/b/c", "p#4.rpf", "p.rpf"]);
    }

    #[test]
    fn an_entry_of_the_tree_leads_back_to_its_row_and_a_folder_to_none() {
        let entries = [entry(0, Some("x/one"), 0x0B), entry(1, Some("two"), 0x0B)];
        let (table, names, entry_of) = lay_out(&leaves(&entries));
        for (at, name) in names.iter().enumerate() {
            match (&table[at], name.as_str()) {
                (RpfEntry::Directory(_), _) => assert_eq!(entry_of[at], u32::MAX),
                (RpfEntry::Binary(file), "one") => {
                    assert_eq!(entry_of[at], 0);
                    assert_eq!(file.file_offset, 0x1000);
                }
                (RpfEntry::Binary(file), "two") => {
                    assert_eq!(entry_of[at], 1);
                    assert_eq!(file.file_offset, 0x2000);
                }
                other => panic!("an entry no leaf made: {other:?}"),
            }
        }
    }

    #[test]
    fn an_empty_archive_is_a_root_with_nothing_under_it() {
        let (table, names, _) = lay_out(&[]);
        let (nodes, _) = build_tree(&table, &names).expect("a lone root is a tree");
        assert_eq!(nodes.len(), 1);
        assert!(nodes[0].is_dir());
    }

    #[test]
    fn a_size_past_four_gib_is_shown_as_the_most_the_entry_holds() {
        let mut big = entry(0, Some("big"), 0x00);
        big.stored = Some(6 << 30);
        let (table, _, _) = lay_out(&leaves(&[big]));
        let RpfEntry::Binary(file) = &table[1] else {
            panic!("a file")
        };
        assert_eq!(file.uncompressed_size, u32::MAX);
        assert_eq!(file.consumed_size, 0, "a nested archive is kept as it is");
    }
}

#[cfg(test)]
mod entry_form_tests {
    use super::super::{Gta6Archive, NoOpCryptoProvider, Rpf9Builder, Rpf9Toc, RpfVersion};
    use super::*;
    use std::io::Write;

    fn fixture() -> (RpfArchive, Vec<u8>, Vec<u8>, Vec<u8>) {
        let mut snapshot = or_test_support::rpf9::keys();
        snapshot.extend(0x5243u32.to_le_bytes());
        for size in [4096u32, 16384] {
            snapshot.extend(size.to_le_bytes());
            snapshot.extend(vec![0; size as usize]);
        }
        snapshot.extend(1u32.to_le_bytes());
        snapshot.extend(0x0000_0001_0200_0000u64.to_le_bytes());
        snapshot.extend((4u32 + 60 * 4 + 16 * 256).to_le_bytes());
        snapshot.extend([0x5d, 0xce, 0x18, 0x27]);
        snapshot.extend([0; 60 * 4]);
        for lane in 0..16u8 {
            snapshot.extend([0x80 + lane; 256]);
        }
        let keys = Arc::new(GtaViKeys::parse(&snapshot).unwrap());
        let plain = b"a synthetic body with distinct stored and decoded forms".to_vec();
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&plain).unwrap();
        let mut compressed = encoder.finish().unwrap();
        let table = or_test_support::rpf9::chunked_table(
            11,
            &[(plain.len() as u32, compressed.len() as u32, true)],
        );
        compressed.resize(compressed.len().next_multiple_of(16), 0);
        let mut stored = compressed.clone();
        let iv = [0x22; 16];
        keys.content_decrypt(1, &iv, &mut stored).unwrap();
        let table = Rpf9Builder::serialize(
            &Rpf9Toc {
                bytes: table,
                key_id: 0,
                registry_id: 0,
                entry_count: 0,
                groups: Vec::new(),
            },
            &keys,
        )
        .unwrap();
        let mut archive = Gta6Archive::open(&table, &keys).unwrap();
        archive.entries[0].cipher = Some(super::super::Gta6Cipher {
            key_id: 1,
            cipher_type: 17,
            iv,
            framing: [0, 0x400, 0, 0, 4],
        });
        let base = 0x200;
        let mut bytes = vec![0; base];
        bytes.extend(table);
        bytes.resize(base + 0x1000, 0);
        bytes.extend(&stored);
        let size = bytes.len() as u64 - base as u64;
        let mount = Gta6Mount {
            base: base as u64,
            archive,
        };
        let leaves: Vec<_> = mount
            .archive
            .entries
            .iter()
            .enumerate()
            .map(|(at, entry)| leaf(&mount, at, entry))
            .collect();
        let (entries, names, entry_of) = lay_out(&leaves);
        let (nodes, entry_to_node) = super::super::reader::build_tree(&entries, &names).unwrap();
        let header = RpfHeader {
            entry_count: entries.len() as u32,
            name_heap_size: 0,
            name_shift: 0,
            xcompress: false,
            encryption: RpfEncryption::None,
        };
        let archive = RpfArchive {
            version: RpfVersion::Rpf9,
            header,
            entries,
            nodes,
            toc_outcome: TocDecryptOutcome::Nominal,
            names,
            name_heap: Vec::new(),
            entry_to_node,
            source: RpfSource::Memory(Arc::new(bytes)),
            start_pos: base as u64,
            size,
            keys: Arc::new(NoOpCryptoProvider::new()),
            paths: OnceLock::new(),
            rpf9: Some(Arc::new(Rpf9State {
                mount,
                context: Arc::new(Gta6Context {
                    keys,
                    names: Arc::new(NameResolver::new()),
                }),
                entry_of,
                leaves,
            })),
        };
        // The same absolute offsets are used through a nested source, without changing process keys.
        let mut nested = archive.clone();
        nested.source = RpfSource::Nested(Arc::new(archive));
        (nested, stored, compressed, plain)
    }

    #[test]
    fn rpf9_body_forms_preserve_stored_deciphered_and_decoded_bytes() {
        let (archive, stored, compressed, plain) = fixture();
        assert_eq!(archive.read_form(1, BodyForm::AsStored).unwrap(), stored);
        assert_eq!(
            archive.read_form(1, BodyForm::Decrypted).unwrap(),
            compressed
        );
        assert_eq!(archive.read_form(1, BodyForm::Decoded).unwrap(), plain);
    }

    #[test]
    fn every_rpf9_body_form_stays_inside_its_nested_archive() {
        let (mut archive, _, _, _) = fixture();
        archive.size = 0x1000;
        for form in BodyForm::ALL {
            assert!(
                archive.read_form(1, form).is_err(),
                "{form:?} escaped its parent"
            );
        }
    }

    fn address_archive(
        rows: Vec<Gta6Entry>,
        source: RpfSource,
        start: u64,
        size: u64,
        context: Arc<Gta6Context>,
    ) -> RpfArchive {
        let mount = Gta6Mount {
            base: start,
            archive: Gta6Archive {
                toc: Rpf9Toc {
                    bytes: Vec::new(),
                    key_id: 0,
                    registry_id: 0,
                    entry_count: rows.len() as u32,
                    groups: Vec::new(),
                },
                entries: rows,
            },
        };
        let leaves: Vec<_> = mount
            .archive
            .entries
            .iter()
            .enumerate()
            .map(|(at, entry)| leaf(&mount, at, entry))
            .collect();
        let (entries, names, entry_of) = lay_out(&leaves);
        let (nodes, entry_to_node) = super::super::reader::build_tree(&entries, &names).unwrap();
        RpfArchive {
            version: RpfVersion::Rpf9,
            header: RpfHeader {
                entry_count: entries.len() as u32,
                name_heap_size: 0,
                name_shift: 0,
                xcompress: false,
                encryption: RpfEncryption::None,
            },
            entries,
            nodes,
            toc_outcome: TocDecryptOutcome::Nominal,
            names,
            name_heap: Vec::new(),
            entry_to_node,
            source,
            start_pos: start,
            size,
            keys: Arc::new(NoOpCryptoProvider::new()),
            paths: OnceLock::new(),
            rpf9: Some(Arc::new(Rpf9State {
                mount,
                context,
                entry_of,
                leaves,
            })),
        }
    }

    fn address_row(index: u32, start: u64, stored: u64, nested: bool, name: &str) -> Gta6Entry {
        Gta6Entry {
            index,
            name64: u64::from(index + 1),
            name32: None,
            name: Some(name.into()),
            start: Some(start),
            stored: Some(stored),
            real: Some(stored),
            class: Some(if nested { 0 } else { 255 }),
            cipher: None,
        }
    }

    #[test]
    fn entry_addresses_follow_parent_tables_and_include_the_leaf_index_without_io() {
        let context = Arc::new(Gta6Context {
            keys: Arc::new(GtaViKeys::parse(&or_test_support::rpf9::keys()).unwrap()),
            names: Arc::new(NameResolver::new()),
        });
        let file = PathBuf::from("synthetic-address-file-that-is-never-opened.rpf");
        let root = Arc::new(address_archive(
            vec![
                address_row(0, 0x100, 16, false, "other"),
                address_row(1, 0x1000, 0x3000, true, "outer"),
            ],
            RpfSource::File(file.clone()),
            0,
            0x5000,
            Arc::clone(&context),
        ));
        let middle = Arc::new(address_archive(
            vec![
                address_row(0, 0x100, 16, false, "folder/other"),
                address_row(1, 0x200, 16, false, "another"),
                address_row(2, 0x1000, 0x1000, true, "inner"),
            ],
            RpfSource::Nested(Arc::clone(&root)),
            0x1000,
            0x3000,
            Arc::clone(&context),
        ));
        let leaf = address_archive(
            vec![address_row(0, 0x100, 16, false, "folder/asset.ytd")],
            RpfSource::Nested(middle),
            0x2000,
            0x1000,
            context,
        );
        let tree_index = leaf.find("folder/asset.ytd").unwrap();
        assert_ne!(
            tree_index, 0,
            "folders change the tree index, not the table index"
        );
        assert_eq!(
            leaf.rpf9_entry_address(tree_index),
            Some(Rpf9EntryAddress {
                file: file.clone(),
                indices: vec![1, 2, 0]
            })
        );
        assert_eq!(leaf.rpf9_entry_address(0), None, "the root is a folder");
        assert_eq!(
            root.rpf9_entry_address(root.find("outer.rpf").unwrap()),
            None
        );
        assert_eq!(
            root.rpf9_entry_address(root.find("other").unwrap()),
            Some(Rpf9EntryAddress {
                file,
                indices: vec![0]
            })
        );
    }

    #[test]
    fn entry_addresses_refuse_memory_missing_and_ambiguous_parents() {
        let context = Arc::new(Gta6Context {
            keys: Arc::new(GtaViKeys::parse(&or_test_support::rpf9::keys()).unwrap()),
            names: Arc::new(NameResolver::new()),
        });
        let rows = || vec![address_row(0, 0x100, 16, false, "asset")];
        let memory = address_archive(
            rows(),
            RpfSource::Memory(Arc::new(Vec::new())),
            0,
            0x1000,
            Arc::clone(&context),
        );
        assert_eq!(memory.rpf9_entry_address(1), None);
        for count in [0, 2] {
            let parents = (0..count)
                .map(|i| address_row(i, 0x1000, 0x1000, true, &format!("alias{i}")))
                .collect();
            let parent = Arc::new(address_archive(
                parents,
                RpfSource::File(PathBuf::from("not-opened.rpf")),
                0,
                0x3000,
                Arc::clone(&context),
            ));
            let child = address_archive(
                rows(),
                RpfSource::Nested(parent),
                0x1000,
                0x1000,
                Arc::clone(&context),
            );
            assert_eq!(child.rpf9_entry_address(1), None, "{count} possible owners");
        }
    }

    #[test]
    fn a_nested_archive_cannot_reopen_its_parent_table_or_read_it_as_an_entry() {
        let context = Arc::new(Gta6Context {
            keys: Arc::new(GtaViKeys::parse(&or_test_support::rpf9::keys()).unwrap()),
            names: Arc::new(NameResolver::new()),
        });
        let mut archive = address_archive(
            vec![address_row(0, 0, 64, true, "self")],
            RpfSource::Memory(Arc::new(Vec::new())),
            0,
            64,
            context,
        );
        Arc::get_mut(archive.rpf9.as_mut().unwrap())
            .unwrap()
            .mount
            .archive
            .toc
            .bytes
            .resize(64, 0);
        let archive = Arc::new(archive);
        assert!(matches!(
            archive.rpf9_nested(1),
            Err(IoError::RpfBadEntry { .. })
        ));
        for form in BodyForm::ALL {
            assert!(
                matches!(archive.read_form(1, form), Err(IoError::RpfBadEntry { .. })),
                "{form:?}"
            );
        }
    }
}
