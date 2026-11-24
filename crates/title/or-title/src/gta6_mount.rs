//! A GTA VI install mounted by identity. The table of an RPF9 archive is encrypted, so no path
//! opens a registration document: one is found by scanning an archive for XML bodies, and is
//! known by its root element and its `id`.

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use or_io::container::rpf::{read_rpf9_body, Rpf9BodyKind, Rpf9Content, Rpf9Scan};
use or_io::gta6::hash::name_hash32;

use crate::dlc::{ContentChangeSetDef, ContentDef, RegisteredFile, SetupContentType, SetupData};

/// What a registration document may inflate to; a longer one is passed over.
const BODY_LIMIT: u64 = 16 << 20;
/// The inflated bytes a scan keeps of each body, in which a document's root element is sought.
const HEAD: usize = 4096;
const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
/// The bit of a map's flags that holds it back as script-managed.
const SCRIPT_MANAGED: u32 = 1;

/// One registration document a scan of an archive found, with where it lies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub offset: u64,
    pub document: Document,
}

/// A registration document, by its root element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Document {
    Setup(SetupData),
    Content(ContentDef),
    ChangeSet(ContentChangeSetDef),
}

type Reader = fn(&[u8]) -> Option<Document>;

/// The root element of each registration document, and what reads it.
const READERS: [(&str, Reader); 3] = [
    ("SSetupData", |b| {
        SetupData::parse(b).ok().map(Document::Setup)
    }),
    ("CDataFileMgr__ContentDef", |b| {
        ContentDef::parse(b).ok().map(Document::Content)
    }),
    ("CDataFileMgr__ContentChangeSetDef", |b| {
        ContentChangeSetDef::parse(b).ok().map(Document::ChangeSet)
    }),
];

impl Document {
    /// The registration document `body` is by its root element, `None` for any other body and for one that does not parse.
    pub fn parse(body: &[u8]) -> Option<Self> {
        reader_of(root_name(body)?)?(body)
    }
}

fn reader_of(root: &[u8]) -> Option<Reader> {
    (READERS.iter())
        .find(|(name, _)| root.eq_ignore_ascii_case(name.as_bytes()))
        .map(|(_, read)| *read)
}

/// The name of the root element `head` opens, past a byte order mark, a declaration and comments.
fn root_name(head: &[u8]) -> Option<&[u8]> {
    let mut rest = head.strip_prefix(BOM).unwrap_or(head);
    loop {
        rest = rest.trim_ascii_start().strip_prefix(b"<")?;
        let close: &[u8] = match rest.first()? {
            b'?' => b"?>",
            b'!' => b"-->",
            _ => break,
        };
        let end = rest.windows(close.len()).position(|w| w == close)?;
        rest = &rest[end + close.len()..];
    }
    let len = (rest.iter()).position(|c| c.is_ascii_whitespace() || matches!(c, b'>' | b'/'))?;
    Some(&rest[..len])
}

/// Every registration document the clear XML bodies of the GTA VI archive at `archive` hold; a body that is no such document, or does not parse, is passed over, and `stop` ends the scan with `Interrupted`.
pub fn read_documents(archive: &Path, stop: &dyn Fn() -> bool) -> io::Result<Vec<Found>> {
    let file = File::open(archive)?;
    let mut scan = Rpf9Scan::new(&file, 0..u64::MAX)
        .map_err(io::Error::other)?
        .keeping(HEAD);
    let mut found = Vec::new();
    while let Some(find) = scan.next() {
        if stop() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let find = find.map_err(io::Error::other)?;
        let xml = matches!(find.kind, Rpf9BodyKind::Zlib(run) if run.content == Rpf9Content::Xml);
        // The head the scan kept says which body is worth reading whole.
        if !xml || root_name(scan.kept(&find)).and_then(reader_of).is_none() {
            continue;
        }
        let offset = find.offset;
        match read_rpf9_body(&file, offset, BODY_LIMIT) {
            Ok((_, body)) => {
                found.extend(Document::parse(&body).map(|document| Found { offset, document }))
            }
            Err(e) => tracing::warn!(offset, "a registration document is not read: {e}"),
        }
    }
    Ok(found)
}

/// A change set a pack registers: its id, the setup groups that name it, whether a content definition includes it as a set of the level, and its files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredSet {
    pub id: String,
    pub groups: Vec<String>,
    pub level: bool,
    pub flags: Vec<String>,
    pub files: Vec<RegisteredFile>,
}

/// A pack: the archive its setup lies in, its setup, and the change sets its documents give.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pack {
    pub archive: PathBuf,
    pub setup: SetupData,
    pub sets: Vec<RegisteredSet>,
    /// The ids a setup group or an include names whose change set was not found, as they are compared.
    pub unread: Vec<String>,
}

/// The packs of an install, in the order their setups mount.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Registry {
    pub packs: Vec<Pack>,
}

impl Registry {
    /// The registry of the documents found in each archive, an archive a pack where it holds a setup.
    pub fn of(archives: Vec<(PathBuf, Vec<Document>)>) -> Self {
        let mut packs = Vec::new();
        for (archive, documents) in archives {
            let (mut setups, mut contents, mut bodies) = (Vec::new(), Vec::new(), Vec::new());
            for document in documents {
                match document {
                    Document::Setup(setup) => setups.push(setup),
                    Document::Content(content) => contents.push(content),
                    Document::ChangeSet(set) => bodies.push(set),
                }
            }
            // An archive of two setups is two packs over the same documents.
            for setup in setups {
                packs.push(pack(archive.clone(), setup, &contents, &bodies));
            }
        }
        packs.sort_by(|a, b| a.setup.mount_cmp(&b.setup));
        Self { packs }
    }

    /// The pack whose setup states the type of a level pack and whose content definitions include sets, the first by the setups' own order.
    pub fn level_pack(&self) -> Option<&Pack> {
        (self.packs.iter()).find(|pack| {
            pack.setup.content_type == SetupContentType::LevelPack
                && pack.sets.iter().any(|set| set.level)
        })
    }
}

/// A change set's id as it is compared: in lower case, with no underscore.
fn normalised(id: &str) -> String {
    (id.chars().filter(|c| *c != '_'))
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The stem and the extension of the last component of a path a document writes.
fn leaf(path: &str) -> (&str, &str) {
    let name = path.rsplit(['/', '\\', ':']).next().unwrap_or(path);
    name.rsplit_once('.').unwrap_or((name, ""))
}

/// The id of the change set the file at `load_path` holds: its stem with no leading `ccs_`.
fn included_id(load_path: &str) -> String {
    let stem = leaf(load_path).0.to_ascii_lowercase();
    normalised(stem.strip_prefix("ccs_").unwrap_or(&stem))
}

fn pack(
    archive: PathBuf,
    setup: SetupData,
    contents: &[ContentDef],
    bodies: &[ContentChangeSetDef],
) -> Pack {
    let inline = || contents.iter().flat_map(|content| &content.content_sets);
    let included = || contents.iter().flat_map(|content| &content.included_sets);
    let groups = &setup.content_change_set_groups;
    let level: HashSet<String> = (inline().map(|set| normalised(&set.id)))
        .chain(included().map(|id| normalised(id)))
        .collect();
    let mut found = HashSet::new();
    let mut sets = Vec::new();
    // An inline set comes first, so the files of an id stated twice are the inline entry's own.
    for set in inline().chain(bodies) {
        let id = normalised(&set.id);
        if !found.insert(id.clone()) {
            continue;
        }
        sets.push(RegisteredSet {
            id: set.id.clone(),
            groups: (groups.iter())
                .filter(|group| {
                    group
                        .content_change_sets
                        .iter()
                        .any(|n| normalised(n) == id)
                })
                .map(|group| group.name_hash.clone())
                .collect(),
            level: level.contains(&id),
            flags: set.flags.clone(),
            files: set.file_list.clone(),
        });
    }
    let by_path = (contents.iter())
        .flat_map(|content| &content.included_changesets)
        .map(|include| included_id(&include.load_path));
    let named = (groups.iter())
        .flat_map(|group| &group.content_change_sets)
        .chain(included())
        .map(|id| normalised(id))
        .chain(by_path);
    let mut unread = Vec::new();
    for id in named {
        if !found.contains(&id) && !unread.contains(&id) {
            unread.push(id);
        }
    }
    Pack {
        archive,
        setup,
        sets,
        unread,
    }
}

/// What the mount needs to know of one map body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapFact {
    /// Which top-level archive it lies in, an index the caller chose.
    pub archive: u32,
    /// The offset of the nested archive that holds it, `None` at the top level.
    pub container: Option<u64>,
    pub name: u32,
    /// The name of its block map, 0 for none.
    pub block: u32,
    pub flags: u32,
    /// Whether it places an entity or a list.
    pub holds: bool,
}

/// Why a map does not stream at rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeldBack {
    NotRegistered,
    ScriptManaged,
    Empty,
}

/// What binding the level's archives to the maps gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// For each map, in the order given: `None` where it streams at rest.
    pub held: Vec<Option<HeldBack>>,
    pub report: Report,
}

/// The counts of a binding.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    pub named_archives: usize,
    pub bound_archives: usize,
    pub maps: usize,
    pub registered: usize,
    pub at_rest: usize,
    pub script_managed: usize,
    pub empty: usize,
    /// The top-level archive taken as the level's, by the caller's index.
    pub level_archive: Option<u32>,
}

/// Bind the archives the sets `level` names to `maps`.
// A container is bound where it holds a map named by the hash of a registered archive's stem. A
// map is tied to the level where its container is bound, or the container of its block map is.
// The top-level archive with the most tied maps is the level's, and each of its maps registered.
pub fn bind(level_files: &[&RegisteredFile], maps: &[MapFact]) -> Binding {
    let named: HashSet<u32> = (level_files.iter())
        .map(|file| leaf(&file.filename))
        .filter(|(_, extension)| extension.eq_ignore_ascii_case("rpf"))
        .map(|(stem, _)| name_hash32(stem.as_bytes()))
        .collect();
    let place = |map: &MapFact| (map.archive, map.container);
    let bound: HashSet<(u32, Option<u64>)> = (maps.iter())
        .filter(|map| named.contains(&map.name))
        .map(place)
        .collect();
    let in_bound: HashSet<u32> = (maps.iter())
        .filter(|map| bound.contains(&place(map)))
        .map(|map| map.name)
        .collect();
    let mut tied = BTreeMap::<u32, usize>::new();
    for map in maps {
        if bound.contains(&place(map)) || (map.block != 0 && in_bound.contains(&map.block)) {
            *tied.entry(map.archive).or_default() += 1;
        }
    }
    // Of two archives with as many tied maps, the lower index.
    let level_archive = (tied.iter())
        .max_by_key(|(archive, count)| (**count, Reverse(**archive)))
        .map(|(archive, _)| *archive);
    let held: Vec<Option<HeldBack>> = (maps.iter())
        .map(|map| {
            if Some(map.archive) != level_archive {
                Some(HeldBack::NotRegistered)
            } else if map.flags & SCRIPT_MANAGED != 0 {
                Some(HeldBack::ScriptManaged)
            } else if !map.holds {
                Some(HeldBack::Empty)
            } else {
                None
            }
        })
        .collect();
    let count = |what: Option<HeldBack>| held.iter().filter(|h| **h == what).count();
    let report = Report {
        named_archives: named.len(),
        bound_archives: bound.len(),
        maps: maps.len(),
        registered: maps.len() - count(Some(HeldBack::NotRegistered)),
        at_rest: count(None),
        script_managed: count(Some(HeldBack::ScriptManaged)),
        empty: count(Some(HeldBack::Empty)),
        level_archive,
    };
    Binding { held, report }
}

#[cfg(test)]
#[path = "gta6_mount_tests.rs"]
mod tests;
