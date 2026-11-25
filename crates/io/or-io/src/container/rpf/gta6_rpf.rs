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
use super::gta6_mount::{open_at, Gta6Mount};
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

    /// Put `next` in place of the context a caller gave, `None` leaving the environment's, and return the one it held.
    pub fn swap_given(next: Option<Arc<Gta6Context>>) -> Option<Arc<Gta6Context>> {
        let mut given = GIVEN.write().unwrap_or_else(|held| held.into_inner());
        std::mem::replace(&mut *given, next)
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
                    let names = NameResolver::load_with_defaults(None);
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
    kind: Option<Rpf9Kind>,
}

/// What an RPF9 entry is, as its row's class byte or type code names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rpf9Kind {
    /// The kind in words, as `rpf9 list` prints it.
    pub label: &'static str,
    /// The extension GTA V gives a file of the same format, where it has one.
    pub extension: Option<&'static str>,
}

/// The GTA V extension of each kind label that names the same format.
const EXTENSIONS: [(&str, &str); 11] = [
    ("archive", "rpf"),
    ("drawable", "ydr"),
    ("fragment", "yft"),
    ("texture dictionary", "ytd"),
    ("bound", "ybn"),
    ("clip dictionary", "ycd"),
    ("particles", "ypt"),
    ("map data", "ymap"),
    ("map types", "ytyp"),
    ("navigation mesh", "ynv"),
    ("script program", "ysc"),
];

/// The kind of an entry: a 24-byte row's class byte wins over a 20-byte row's type code, and a row
/// whose class says archive is one only when its body begins one. `None` where no table names it.
fn row_kind(entry: &Gta6Entry, type_code: Option<u32>, archive: bool) -> Option<Rpf9Kind> {
    use super::gta6_kinds::{CLASSES, TYPES};
    if !archive && entry.kind() == Some(Gta6Class::NestedArchive) {
        return None;
    }
    let label = match (entry.class, type_code) {
        (Some(class), _) => CLASSES.iter().find(|(byte, _)| *byte == class)?.1,
        (None, Some(code)) => TYPES.iter().find(|(c, _)| *c == code)?.1,
        (None, None) => return None,
    };
    let extension = (EXTENSIONS.iter().find(|(l, _)| *l == label)).map(|(_, ext)| *ext);
    Some(Rpf9Kind { label, extension })
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
    let mut archive = open_at(&mut src, input.start, &context.keys)?;
    archive.resolve_names(&context.names);
    let mount = Gta6Mount {
        base: input.start,
        archive,
    };
    let leaves: Vec<Leaf> = (mount.archive.entries.iter().enumerate())
        .map(|(at, entry)| leaf(&mount, &mut src, at, entry))
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

fn leaf<R: Read + Seek>(mount: &Gta6Mount, src: &mut R, at: usize, entry: &Gta6Entry) -> Leaf {
    let row = mount.archive.entry_extra(at).and_then(|extra| extra.nested);
    let stored = (entry.stored)
        .or(row.map(|row| row.stored_size()))
        .unwrap_or(0);
    // A 20-byte row states its two sections' padded sizes, each over a nibble of the version.
    let sections = |row: super::gta6_archive::Gta6NestedRow| {
        u64::from(row.virtual_size & !0xF) + u64::from(row.flags & !0xF)
    };
    let real = entry.real.or(row.map(sections)).unwrap_or(0);
    let archive = entry.kind() == Some(Gta6Class::NestedArchive)
        && mount.entry_begins_archive(src, entry).unwrap_or(false);
    Leaf {
        segments: segments(entry, archive),
        offset: entry.start,
        stored,
        real,
        archive,
        ciphered: entry.cipher.is_some(),
        slot: entry.stored.is_none() && row.is_some(),
        kind: row_kind(entry, row.map(|row| row.resource_type), archive),
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
                state.mount.read_entry_decrypted(&mut src, entry, keys)
            }
            BodyForm::Decrypted if leaf.ciphered => {
                let mut src = bytes_of(&self.source)?;
                (state.mount).read_entry_deciphered_stored(&mut src, entry, keys)
            }
            _ => self.rpf9_stored(entry_index, leaf),
        }
    }

    fn rpf9_stored(&self, entry_index: u32, leaf: &Leaf) -> Result<Vec<u8>> {
        let offset = leaf.offset.ok_or_else(|| no_body(entry_index))?;
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
        let (_, leaf, _) = self.rpf9_leaf(entry_index)?;
        let offset =
            (leaf.offset.filter(|_| leaf.archive)).ok_or_else(|| IoError::RpfBadEntry {
                message: format!("entry {entry_index} is not a nested archive"),
            })?;
        let name = (self.node_for_entry(entry_index)).map(|node| node.name().to_string());
        Self::parse(
            RpfSource::Nested(Arc::clone(self)),
            self.start_pos + offset,
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

impl RpfArchive {
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

    /// What entry `entry_index` of an RPF9 is, as its row names it; `None` for a folder, a row no table names or any other generation.
    pub fn rpf9_kind(&self, entry_index: u32) -> Option<Rpf9Kind> {
        // ⚠ Asked of every row a tree draws, so a GTA V row costs no error built and dropped.
        let state = self.rpf9.as_deref()?;
        let at = *state.entry_of.get(entry_index as usize)?;
        state.leaves.get(at as usize)?.kind
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

#[path = "gta6_rpf_row.rs"]
pub mod row;

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
                    kind: row_kind(e, None, archive),
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
    fn a_24_byte_row_is_named_by_its_class_and_a_20_byte_row_by_its_type_code() {
        let media = row_kind(&entry(0, None, 0xFF), None, false);
        assert_eq!(
            media,
            Some(Rpf9Kind {
                label: "media",
                extension: None
            })
        );
        // A 20-byte row has no class byte; its type code names it.
        let mut headerless = entry(1, None, 0);
        headerless.class = None;
        let drawable = row_kind(&headerless, Some(0x02), false);
        assert_eq!(
            drawable,
            Some(Rpf9Kind {
                label: "drawable",
                extension: Some("ydr")
            })
        );
        assert_eq!(
            row_kind(&headerless, Some(0x58), false).map(|k| k.label),
            Some("cloth")
        );
        assert_eq!(
            row_kind(&headerless, Some(0x58), false).and_then(|k| k.extension),
            None
        );
        // The class wins over any code, as `rpf9 list` reads a row.
        let resource = row_kind(&entry(2, None, 0x0B), Some(0x02), false);
        assert_eq!(resource.map(|k| k.label), Some("resource"));
    }

    #[test]
    fn a_row_no_table_names_has_no_kind_and_an_archive_is_one_only_when_its_body_begins_one() {
        assert_eq!(row_kind(&entry(0, None, 0x0E), None, false), None);
        let mut headerless = entry(1, None, 0);
        headerless.class = None;
        assert_eq!(row_kind(&headerless, Some(0x7F), false), None);
        assert_eq!(row_kind(&headerless, None, false), None);
        let nested = row_kind(&entry(2, None, 0x00), None, true);
        assert_eq!(nested.and_then(|k| k.extension), Some("rpf"));
        assert_eq!(row_kind(&entry(3, None, 0x00), None, false), None);
    }

    #[test]
    fn every_extension_given_a_kind_is_one_a_table_names() {
        use super::super::gta6_kinds::{CLASSES, TYPES};
        for (label, _) in EXTENSIONS {
            let named =
                CLASSES.iter().any(|(_, l)| *l == label) || TYPES.iter().any(|(_, l)| *l == label);
            assert!(named, "{label} is no kind a row names");
        }
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
