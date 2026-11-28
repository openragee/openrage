//! Every file of the active install and of each install, folder and archive
//! Files shows, nested archives included: what Go to File and the search of
//! every archive read.
//!
//! ⚠ Walked off the UI thread, and what a first walk has found is usable while
//! it runs. A walk again reopens only the archives on disk whose size or time
//! changed, so it is cheap enough to follow every tree edit and save.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::SystemTime;

use or_io::container::rpf::crypto::RpfCryptoProvider;
use or_io::container::rpf::RpfVersion;
use or_io::gta6::hash::name_hash64;
use or_io::hash::string_hash;
use or_title::install::crawl::{self, ControlFlow, LOOSE};

use super::document::DocumentId;
use super::jobs::{file_name, JobKind, Outcome, RunningJob, Then};
use super::memory::hash_table_bytes;
use super::vfs_session::{self, Mounted, DOCUMENT_MOUNT};
use super::{AppState, DocPayload, DocumentKind};

/// How many paths a first walk hands over at a time.
const BATCH: usize = 4096;

/// What an archive on disk was when it was walked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
}

fn stamp_of(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some(Stamp {
        len: meta.len(),
        modified: meta.modified().ok(),
    })
}

/// One root's files.
// ponytail: one string per path, repeating each archive chain; intern the chains if the memory matters.
#[derive(Debug, Clone, Default)]
pub struct Listing {
    /// Every file's path below the root; an archive on disk is followed by what it holds.
    pub files: Vec<Box<str>>,
    /// Each archive on disk the walk opened, by its path below the root.
    stamps: HashMap<Box<str>, Stamp>,
    /// What the walk could not open or read.
    pub warnings: Vec<String>,
    /// The GTA VI archives on disk, by their path below the root; `""` when the root is one.
    rpf9: HashSet<Box<str>>,
}

impl Listing {
    /// Its paths and the stamps of the archives it opened, each string at its capacity.
    fn held_bytes(&self) -> usize {
        let paths: usize = self.files.iter().map(|path| path.len()).sum();
        let stamped: usize = self.stamps.keys().map(|path| path.len()).sum();
        let warnings: usize = self.warnings.iter().map(String::capacity).sum();
        let rpf9: usize = self.rpf9.iter().map(|path| path.len()).sum();
        self.files.capacity() * std::mem::size_of::<Box<str>>()
            + paths
            + hash_table_bytes::<(Box<str>, Stamp)>(self.stamps.capacity())
            + stamped
            + self.warnings.capacity() * std::mem::size_of::<String>()
            + warnings
            + hash_table_bytes::<Box<str>>(self.rpf9.capacity())
            + rpf9
    }
}

/// A root's document as a tree edit or a save leaves it: its edits, whether it
/// is unsaved, its unsaved archives and its rows.
type Seen = (u64, bool, usize, usize);

/// An install, a folder or an archive, and its files.
pub struct Root {
    pub path: PathBuf,
    /// The document Files shows it as, if one is open.
    pub doc: Option<DocumentId>,
    /// An archive opened as a document, walked through the archive it holds.
    archive: bool,
    pub listing: Arc<Listing>,
    /// Bumped each time `listing` is replaced rather than added to.
    pub generation: u64,
    walked: bool,
    walk: Option<Walk>,
    seen: Option<Seen>,
    /// Asked to walk again while a walk was running.
    stale: bool,
}

struct Walk {
    rx: Receiver<Message>,
    cancel: Arc<AtomicBool>,
}

impl Drop for Walk {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

enum Message {
    /// Paths a first walk found, in its order.
    Found(Vec<Box<str>>),
    /// A first walk's end: the archives it opened, what it could not, and which are GTA VI's.
    Finished {
        stamps: HashMap<Box<str>, Stamp>,
        warnings: Vec<String>,
        rpf9: HashSet<Box<str>>,
    },
    /// A walk again's whole listing.
    Replaced(Listing),
}

enum Source {
    Disk(Arc<dyn RpfCryptoProvider>),
    Archive(Mounted),
}

impl Root {
    fn new(path: PathBuf, doc: Option<DocumentId>, archive: bool, seen: Option<Seen>) -> Self {
        Self {
            path,
            doc,
            archive,
            listing: Arc::default(),
            generation: 0,
            walked: false,
            walk: None,
            seen,
            stale: false,
        }
    }

    pub fn is_walking(&self) -> bool {
        self.walk.is_some()
    }

    fn start(&mut self, source: Source) {
        let previous = self.walked.then(|| Arc::clone(&self.listing));
        let root = self.path.clone();
        self.start_with(move |tx, stop| match source {
            Source::Disk(provider) => walk_disk(&root, provider, previous, tx, stop),
            Source::Archive(mounted) => walk_archive(&root, &mounted, tx),
        });
    }

    fn start_with(&mut self, work: impl FnOnce(&Sender<Message>, &AtomicBool) + Send + 'static) {
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&cancel);
        std::thread::spawn(move || work(&tx, &stop));
        self.walk = Some(Walk { rx, cancel });
    }

    /// A root with nothing to walk, such as an archive never saved.
    fn settle_empty(&mut self) {
        self.listing = Arc::default();
        self.generation += 1;
        self.walked = true;
    }

    /// Take what the walk found; how many files a first walk found, once it has ended.
    fn drain(&mut self) -> Option<usize> {
        let walk = self.walk.as_ref()?;
        let finished = loop {
            match walk.rx.try_recv() {
                Ok(Message::Found(paths)) => Arc::make_mut(&mut self.listing).files.extend(paths),
                Ok(Message::Finished {
                    stamps,
                    warnings,
                    rpf9,
                }) => {
                    let listing = Arc::make_mut(&mut self.listing);
                    listing.stamps = stamps;
                    listing.warnings = warnings;
                    listing.rpf9 = rpf9;
                    break true;
                }
                Ok(Message::Replaced(listing)) => {
                    self.listing = Arc::new(listing);
                    self.generation += 1;
                    break true;
                }
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    // A walk that died before its end must not read as one that looked everywhere.
                    let stopped =
                        format!("{}: the walk stopped before its end", self.path.display());
                    Arc::make_mut(&mut self.listing).warnings.push(stopped);
                    break false;
                }
            }
        };
        self.walk = None;
        let first = !std::mem::replace(&mut self.walked, true);
        (first && finished).then(|| self.listing.files.len())
    }
}

/// What the index should hold this frame: one root, and its document if one is open.
struct Wanted {
    path: PathBuf,
    doc: Option<DocumentId>,
    archive: bool,
    seen: Option<Seen>,
}

#[derive(Default)]
pub struct FileIndex {
    pub roots: Vec<Root>,
    /// Whether the roots were walked with keys, which open archives nothing else can.
    keys: bool,
    /// Walk every root on disk again, for what changed outside the tree.
    pub recheck: bool,
}

impl FileIndex {
    pub fn root_of(&self, doc: DocumentId) -> Option<&Root> {
        self.roots.iter().find(|root| root.doc == Some(doc))
    }

    /// What every root's listing holds, each path once for the root that found it.
    pub fn held_bytes(&self) -> usize {
        let roots = self.roots.capacity() * std::mem::size_of::<Root>();
        let held = |root: &Root| root.path.capacity() + root.listing.held_bytes();
        roots + self.roots.iter().map(held).sum::<usize>()
    }

    pub fn is_walking(&self) -> bool {
        self.roots.iter().any(Root::is_walking)
    }

    pub fn files(&self) -> usize {
        self.roots.iter().map(|root| root.listing.files.len()).sum()
    }

    pub fn generations(&self) -> u64 {
        self.roots.iter().map(|root| root.generation).sum()
    }

    pub fn warnings(&self) -> impl Iterator<Item = &String> {
        self.roots.iter().flat_map(|root| &root.listing.warnings)
    }

    /// A first walk in flight, as the job list draws it; a walk again is too short to list.
    pub fn job(&self) -> Option<RunningJob> {
        let mut first = self.roots.iter().filter(|r| r.is_walking() && !r.walked);
        let root = first.next()?;
        let files =
            root.listing.files.len() + first.map(|other| other.listing.files.len()).sum::<usize>();
        Some(RunningJob {
            kind: JobKind::FileIndex,
            doing: format!("Listing every file of {}", file_name(&root.path)),
            fraction: None,
            detail: format!("{files} files so far"),
            stopping: false,
        })
    }

    /// Stop every walk, keeping what it found; `false` when none runs.
    pub fn stop(&mut self) -> bool {
        let mut stopped = false;
        for root in &mut self.roots {
            if root.walk.take().is_none() {
                continue;
            }
            stopped = true;
            // A walk again leaves the last listing whole; a first walk's is part of the files.
            if !std::mem::replace(&mut root.walked, true) {
                let short = format!("{}: stopped before its end", root.path.display());
                Arc::make_mut(&mut root.listing).warnings.push(short);
            }
        }
        stopped
    }

    /// A walk of `path` that does `work` in place of reading it.
    #[cfg(test)]
    pub fn walk_with(&mut self, path: PathBuf, work: impl FnOnce(&AtomicBool) + Send + 'static) {
        let mut root = Root::new(path, None, false, None);
        root.start_with(move |_, stop| work(stop));
        self.roots.push(root);
    }

    /// Take `wanted` as the roots, and return the ones to walk now.
    fn sync(&mut self, wanted: Vec<Wanted>) -> Vec<usize> {
        let recheck = std::mem::take(&mut self.recheck);
        self.roots
            .retain(|root| wanted.iter().any(|want| want.path == root.path));
        let mut walk = Vec::new();
        for want in wanted {
            let Some(at) = self.roots.iter().position(|root| root.path == want.path) else {
                walk.push(self.roots.len());
                let root = Root::new(want.path, want.doc, want.archive, want.seen);
                self.roots.push(root);
                continue;
            };
            let root = &mut self.roots[at];
            let edited = root.seen.is_some() && want.seen.is_some() && root.seen != want.seen;
            root.doc = want.doc;
            root.seen = want.seen;
            root.stale |= edited || (recheck && !root.archive);
            if root.stale && !root.is_walking() {
                root.stale = false;
                walk.push(at);
            }
        }
        walk
    }

    /// The files `query` names: by the hash of their name first, then by their
    /// name, fuzzily, recent ones first; at most `limit`, and the recent ones
    /// alone for an empty query.
    pub fn find(
        &self,
        query: &str,
        scope: Scope,
        install: Option<&Path>,
        recent: &[Offer],
        limit: usize,
    ) -> Vec<Offer> {
        let in_scope = |root: &Path, path: &str| match scope {
            Scope::All => true,
            Scope::BaseGame | Scope::Overlay => {
                install == Some(root) && in_overlay(path) == (scope == Scope::Overlay)
            }
        };
        let letters = fuzzy_letters(query);
        if letters.is_empty() {
            return recent
                .iter()
                .filter(|offer| in_scope(&offer.root, &offer.path))
                .take(limit)
                .cloned()
                .collect();
        }
        let hashes = hashes_named(query);
        let hash64 = hash64_named(query);
        let recency: HashMap<(&Path, &str), usize> = recent
            .iter()
            .enumerate()
            .map(|(at, offer)| ((offer.root.as_path(), offer.path.as_str()), at))
            .collect();
        let mut found: Vec<(Rank, &Path)> = Vec::new();
        for root in &self.roots {
            for path in root.listing.files.iter().map(|path| &**path) {
                if !in_scope(&root.path, path) {
                    continue;
                }
                let leaf = leaf_of(path);
                let stem = leaf.rsplit_once('.').map_or(leaf, |(stem, _)| stem);
                let named = !hashes.is_empty()
                    && [leaf, stem]
                        .iter()
                        .any(|name| hashes.contains(&string_hash(name.as_bytes())));
                let named = named
                    || hash64.is_some_and(|start| {
                        (names64_of(&root.listing.rpf9, path).into_iter().flatten())
                            .any(|hash| starts(start, hash))
                    });
                let Some(score) = fuzzy_score(&letters, leaf).or(named.then_some(0)) else {
                    continue;
                };
                let recent = recency
                    .get(&(root.path.as_path(), path))
                    .copied()
                    .unwrap_or(usize::MAX);
                found.push(((!named, recent, -score, leaf.len(), leaf, path), &root.path));
            }
        }
        if found.len() > limit {
            found.select_nth_unstable_by(limit, |a, b| a.0.cmp(&b.0));
            found.truncate(limit);
        }
        found.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        found
            .into_iter()
            .map(|((.., path), root)| Offer {
                root: root.to_path_buf(),
                path: path.to_string(),
            })
            .collect()
    }
}

/// Where a file Go to File finds sorts, ascending: a file a hash names, then
/// the recent, the best score, the shortest name, the name and the path.
type Rank<'a> = (bool, usize, i32, usize, &'a str, &'a str);

/// Which files Go to File looks through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    #[default]
    All,
    BaseGame,
    Overlay,
}

impl Scope {
    pub const ALL: [Scope; 3] = [Scope::All, Scope::BaseGame, Scope::Overlay];

    pub fn label(self) -> &'static str {
        match self {
            Scope::All => "All",
            Scope::BaseGame => "Base game",
            Scope::Overlay => "Mods overlay",
        }
    }

    pub fn hover(self) -> &'static str {
        match self {
            Scope::All => "The install, and every folder and archive open in Files",
            Scope::BaseGame => "The install's own files, outside its mods folder",
            Scope::Overlay => "The install's mods folder, which the game reads first",
        }
    }
}

/// A file Go to File offers: the root it is under and its path there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub root: PathBuf,
    pub path: String,
}

impl Offer {
    pub fn leaf(&self) -> &str {
        leaf_of(&self.path)
    }

    /// The folders and archives it is in, below its root.
    pub fn place(&self) -> &str {
        self.path.rsplit_once('/').map_or("", |(place, _)| place)
    }
}

fn leaf_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn in_overlay(path: &str) -> bool {
    path.split('/')
        .next()
        .is_some_and(|first| first.eq_ignore_ascii_case(or_title::OVERLAY_DIR))
}

/// The hashes `query` spells: `0x` and hex digits, eight hex digits, or a decimal, signed or not.
fn hashes_named(query: &str) -> Vec<u32> {
    let query = query.trim();
    if let Some(hex) = query
        .strip_prefix("0x")
        .or_else(|| query.strip_prefix("0X"))
    {
        return u32::from_str_radix(hex, 16).ok().into_iter().collect();
    }
    let mut hashes = Vec::new();
    if query.len() == 8 {
        hashes.extend(u32::from_str_radix(query, 16).ok());
    }
    hashes.extend(
        query
            .parse::<u32>()
            .ok()
            .or_else(|| query.parse::<i32>().ok().map(|signed| signed as u32)),
    );
    hashes
}

/// The start of a GTA VI name hash `query` spells: 6 to 16 hex digits, with or without `0x`, as
/// their value and how many they are.
fn hash64_named(query: &str) -> Option<(u64, u32)> {
    let query = query.trim();
    let hex = (query.strip_prefix("0x"))
        .or_else(|| query.strip_prefix("0X"))
        .unwrap_or(query);
    if !(6..=16).contains(&hex.len()) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some((u64::from_str_radix(hex, 16).ok()?, hex.len() as u32))
}

/// Whether `hash`, written in 16 hex digits, starts with the digits `start` holds.
fn starts((value, digits): (u64, u32), hash: u64) -> bool {
    hash >> (4 * (16 - digits)) == value
}

/// The name hashes a GTA VI row at `path` may have: the one its `hash_` name states, and its name's
/// inside its innermost archive, with and without the `.rpf` a nested archive's name is given;
/// none for a row of no GTA VI archive, so a GTA V row is found as before.
// ponytail: hashed from the tree's name, so a row whose stated name the tree had to change (a
// numbered duplicate, a `\` or a blank part) is not found; keep the table's hash in the index if that matters.
fn names64_of(rpf9: &HashSet<Box<str>>, path: &str) -> [Option<u64>; 3] {
    let gta6 = rpf9.contains("") || top_archive(path).is_some_and(|top| rpf9.contains(top));
    if !gta6 {
        return [None; 3];
    }
    let inner = inside_archive(path);
    let stated = (leaf_of(inner).strip_prefix("hash_"))
        .and_then(|hex| hex.get(..16))
        .and_then(|hex| u64::from_str_radix(hex, 16).ok());
    let bare = (inner.strip_suffix(".rpf")).map(|name| name_hash64(name.as_bytes()));
    [stated, Some(name_hash64(inner.as_bytes())), bare]
}

fn ends_rpf(segment: &str) -> bool {
    let end = segment.get(segment.len().saturating_sub(4)..);
    end.is_some_and(|end| end.eq_ignore_ascii_case(".rpf"))
}

/// The archive on disk `path` is inside, as its path below the root; `None` for a loose file.
fn top_archive(path: &str) -> Option<&str> {
    let mut end = 0;
    for segment in path.split('/') {
        end += segment.len();
        if end < path.len() && ends_rpf(segment) {
            return Some(&path[..end]);
        }
        end += 1;
    }
    None
}

/// `path` inside the innermost archive that holds it; all of it when none does.
fn inside_archive(path: &str) -> &str {
    let (mut inside, mut rest) = (path, path);
    while let Some((segment, after)) = rest.split_once('/') {
        if ends_rpf(segment) {
            inside = after;
        }
        rest = after;
    }
    inside
}

/// Whether the file at `path` starts as a GTA VI archive does.
fn is_rpf9(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 4];
    let read = std::fs::File::open(path).and_then(|mut file| file.read_exact(&mut head));
    read.is_ok() && RpfVersion::detect(head) == Some(RpfVersion::Rpf9)
}

/// `query` as [`fuzzy_score`] reads it: lower case, without spaces.
pub fn fuzzy_letters(query: &str) -> Vec<u8> {
    query
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .map(|b| b.to_ascii_lowercase())
        .collect()
}

/// How well `letters` match `text` in order, ignoring case: highest for a run
/// of letters and for the text's own start, then for a word's start, lower for
/// the letters skipped; `None` when they do not all appear.
///
/// ⚠ A run outweighs letters scattered over word starts, and a `.` starts no
/// word: otherwise `addr` ranks `mph3_add.rpf`, whose extension's `r` starts a
/// word, above `adder.yft` in a retail install.
// ponytail: tries each start of the first letter and then matches greedily, not every alignment.
pub fn fuzzy_score(letters: &[u8], text: &str) -> Option<i32> {
    let text = text.as_bytes();
    let Some(&first) = letters.first() else {
        return Some(0);
    };
    (0..text.len())
        .filter(|&start| text[start].to_ascii_lowercase() == first)
        .filter_map(|start| {
            let mut score = 0;
            let mut previous: Option<usize> = None;
            let mut at = start;
            for &letter in letters {
                let found = (at..text.len()).find(|&i| text[i].to_ascii_lowercase() == letter)?;
                let starts_word = found > 0 && {
                    let before = text[found - 1];
                    !before.is_ascii_alphanumeric() && before != b'.'
                };
                score += 1 + match previous {
                    Some(p) if p + 1 == found => 6,
                    _ if found == 0 => 6,
                    _ if starts_word => 4,
                    _ => 0,
                };
                if previous.is_some() {
                    score -= (found - at).min(3) as i32;
                }
                previous = Some(found);
                at = found + 1;
            }
            Some(score)
        })
        .max()
}

fn tree_path(container: &str, path: &str) -> Box<str> {
    if container == LOOSE {
        path.into()
    } else {
        format!("{container}/{path}").into()
    }
}

/// Walk a folder on disk and the archives in it, reopening only those that
/// changed since `previous`; a first walk streams what it finds.
fn walk_disk(
    root: &Path,
    provider: Arc<dyn RpfCryptoProvider>,
    previous: Option<Arc<Listing>>,
    tx: &Sender<Message>,
    cancel: &AtomicBool,
) {
    let mut warnings = Vec::new();
    let mut stamps: HashMap<Box<str>, Stamp> = HashMap::new();
    let mut kept: HashSet<Box<str>> = HashSet::new();
    let mut rpf9: HashSet<Box<str>> = HashSet::new();
    let mut found: Vec<Box<str>> = Vec::new();
    let streams = previous.is_none();
    let outcome = crawl::crawl_where(
        root,
        provider,
        &mut warnings,
        &mut |path| {
            let rel: Box<str> = crawl::display_rel(root, path).into();
            let stamp = stamp_of(path);
            let before = previous.as_ref().and_then(|p| p.stamps.get(&rel));
            let unchanged = stamp.is_some() && before == stamp.as_ref();
            let gta6 = match previous.as_ref().filter(|_| unchanged) {
                Some(previous) => previous.rpf9.contains(&rel),
                None => is_rpf9(path),
            };
            if gta6 {
                rpf9.insert(rel.clone());
            }
            if let Some(stamp) = stamp {
                stamps.insert(rel.clone(), stamp);
            }
            if unchanged {
                kept.insert(rel);
            }
            !unchanged
        },
        &mut |file| {
            if cancel.load(Ordering::Relaxed) {
                return ControlFlow::Break(());
            }
            found.push(tree_path(file.container, file.path));
            if streams && found.len() >= BATCH {
                let _ = tx.send(Message::Found(std::mem::take(&mut found)));
            }
            ControlFlow::Continue(())
        },
    );
    if outcome.is_break() {
        return;
    }
    let Some(previous) = previous else {
        let _ = tx.send(Message::Found(found));
        let _ = tx.send(Message::Finished {
            stamps,
            warnings,
            rpf9,
        });
        return;
    };
    // An archive not reopened keeps what it held, and what could not be read in it.
    warnings.extend(
        previous
            .warnings
            .iter()
            .filter(|warning| kept.iter().any(|archive| warning.contains(&**archive)))
            .cloned(),
    );
    let files = with_kept_archives(found, &previous, &kept);
    let _ = tx.send(Message::Replaced(Listing {
        files,
        stamps,
        warnings,
        rpf9,
    }));
}

/// `found`, with what each archive in `kept` held in `previous` put back after it.
fn with_kept_archives(
    found: Vec<Box<str>>,
    previous: &Listing,
    kept: &HashSet<Box<str>>,
) -> Vec<Box<str>> {
    if kept.is_empty() {
        return found;
    }
    let files = &previous.files;
    let mut held: HashMap<&str, &[Box<str>]> = HashMap::new();
    let mut at = 0;
    while at < files.len() {
        let archive = &files[at];
        at += 1;
        if !kept.contains(archive) {
            continue;
        }
        let inside = format!("{archive}/");
        let end = files[at..]
            .iter()
            .position(|file| !file.starts_with(&inside))
            .map_or(files.len(), |n| at + n);
        held.insert(archive, &files[at..end]);
        at = end;
    }
    let mut out = Vec::with_capacity(found.len() + held.values().map(|h| h.len()).sum::<usize>());
    for path in found {
        let inside = held.get(&*path).copied();
        out.push(path);
        out.extend(inside.into_iter().flatten().cloned());
    }
    out
}

/// Walk an archive opened as a document, through the archive the document holds.
fn walk_archive(root: &Path, mounted: &Mounted, tx: &Sender<Message>) {
    let (result, undescended) = vfs_session::find(mounted, "**");
    let files = result
        .matches
        .iter()
        .map(|path| {
            path.strip_prefix(DOCUMENT_MOUNT)
                .unwrap_or(path)
                .trim_start_matches('/')
                .into()
        })
        .collect();
    let warnings = result.unwalkable.into_iter().chain(undescended).collect();
    let rpf9 = match is_rpf9(root) {
        true => HashSet::from([Box::from("")]),
        false => HashSet::new(),
    };
    let _ = tx.send(Message::Replaced(Listing {
        files,
        stamps: HashMap::new(),
        warnings,
        rpf9,
    }));
}

impl AppState {
    /// Index the active install and every install, folder and archive Files shows, and take what the walks found.
    pub fn tick_file_index(&mut self) {
        let keys = self.keys.has_keys();
        if self.file_index.keys != keys {
            // New keys open archives the last walks could not, and a walk again reopens only what changed.
            self.file_index = FileIndex {
                keys,
                ..FileIndex::default()
            };
        }
        let wanted = self.index_roots();
        for at in self.file_index.sync(wanted) {
            let root = &self.file_index.roots[at];
            let source = if root.archive {
                let doc = root.doc.and_then(|doc| self.document(doc));
                match doc.map(|doc| &doc.payload) {
                    Some(DocPayload::Rpf(rpf)) => {
                        vfs_session::mount_document(&rpf.session).map(Source::Archive)
                    }
                    _ => None,
                }
            } else {
                Some(Source::Disk(self.keys.provider()))
            };
            let root = &mut self.file_index.roots[at];
            match source {
                Some(source) => root.start(source),
                None => root.settle_empty(),
            }
        }
        for root in &mut self.file_index.roots {
            if let Some(files) = root.drain() {
                let said = format!("{files} files of {}", file_name(&root.path));
                self.jobs
                    .record(JobKind::FileIndex, Outcome::Done(Then::Nothing), said);
            }
        }
    }

    fn index_roots(&self) -> Vec<Wanted> {
        let mut wanted: Vec<Wanted> = Vec::new();
        for doc in self
            .documents
            .iter()
            .filter(|d| d.rpf_entry_binding.is_none())
        {
            let (Some(path), DocPayload::Rpf(rpf)) = (&doc.path, &doc.payload) else {
                continue;
            };
            let unsaved = rpf.session.dirty_archive_ids().len();
            wanted.push(Wanted {
                path: path.clone(),
                doc: Some(doc.id),
                archive: !matches!(doc.kind, DocumentKind::GameRoot | DocumentKind::Folder),
                seen: Some((
                    doc.edit_epoch(),
                    doc.dirty,
                    unsaved,
                    rpf.session.nodes.len(),
                )),
            });
        }
        if let Some(install) = self.prefs.active_game_root() {
            if !wanted.iter().any(|want| want.path == *install) && install.is_dir() {
                wanted.push(Wanted {
                    path: install.clone(),
                    doc: None,
                    archive: false,
                    seen: None,
                });
            }
        }
        wanted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hash_is_read_in_hex_with_or_without_its_prefix_and_in_decimal_signed_or_not() {
        let hash = string_hash(b"adder");
        assert_eq!(hashes_named(&format!("0x{hash:08x}")), [hash]);
        assert!(hashes_named(&format!("{hash:08X}")).contains(&hash));
        assert!(hashes_named(&hash.to_string()).contains(&hash));
        assert!(hashes_named(&(hash as i32).to_string()).contains(&hash));
        assert!(hashes_named("adder").is_empty());
    }

    /// An index of one root on disk whose listing holds `files`, `rpf9` naming its GTA VI archives.
    fn index_of(files: &[&str], rpf9: &[&str]) -> FileIndex {
        let mut root = Root::new(PathBuf::from("install"), None, false, None);
        root.listing = Arc::new(Listing {
            files: files.iter().map(|&file| Box::from(file)).collect(),
            rpf9: rpf9.iter().map(|&archive| Box::from(archive)).collect(),
            ..Listing::default()
        });
        FileIndex {
            roots: vec![root],
            ..FileIndex::default()
        }
    }

    fn found(index: &FileIndex, query: &str) -> Vec<String> {
        let offers = index.find(query, Scope::All, None, &[], 50);
        offers.into_iter().map(|offer| offer.path).collect()
    }

    #[test]
    fn a_gta_vi_row_is_found_by_the_start_of_its_name_hash_in_any_spelling() {
        let hash = name_hash64(b"data/named.bin");
        let nested = name_hash64(b"inner/deep.bin");
        let index = index_of(
            &[
                "six.rpf",
                "six.rpf/data/named.bin",
                "six.rpf/hash_0123456789abcdef",
                "six.rpf/n.rpf/inner/deep.bin",
            ],
            &["six.rpf"],
        );
        let full = format!("{hash:016x}");
        for query in [
            full.clone(),
            format!("0x{full}"),
            full.to_ascii_uppercase(),
            format!("0X{}", full[..6].to_ascii_uppercase()),
            full[..8].to_string(),
        ] {
            assert!(
                found(&index, &query).contains(&"six.rpf/data/named.bin".to_string()),
                "{query}"
            );
        }
        assert!(
            !found(&index, &full[..5]).contains(&"six.rpf/data/named.bin".to_string()),
            "five digits are no hash"
        );
        let inner = format!("0x{:010x}", nested >> 24);
        assert_eq!(found(&index, &inner), ["six.rpf/n.rpf/inner/deep.bin"]);
        assert_eq!(
            found(&index, "0x0123456789AB"),
            ["six.rpf/hash_0123456789abcdef"]
        );
    }

    #[test]
    fn a_gta_v_row_is_not_found_by_a_gta_vi_name_hash() {
        let hash = name_hash64(b"data/named.bin");
        let index = index_of(&["five.rpf", "five.rpf/data/named.bin"], &[]);
        assert!(found(&index, &format!("0x{hash:016x}")).is_empty());
        let named = string_hash(b"named");
        assert_eq!(
            found(&index, &format!("0x{named:08x}")),
            ["five.rpf/data/named.bin"],
            "the hash of its name finds it as before"
        );
    }

    #[test]
    fn a_nested_archives_hash_is_read_with_and_without_the_rpf_its_name_is_given() {
        let set = HashSet::from([Box::from("x/six.rpf")]);
        let [_, full, bare] = names64_of(&set, "x/six.rpf/a/n.rpf");
        assert_eq!(full, Some(name_hash64(b"a/n.rpf")));
        assert_eq!(bare, Some(name_hash64(b"a/n")));
        assert_eq!(
            names64_of(&set, "x/six.rpf"),
            [None; 3],
            "the archive itself is loose"
        );
        assert_eq!(names64_of(&set, "y/five.rpf/a"), [None; 3]);
        let whole = HashSet::from([Box::from("")]);
        assert_eq!(names64_of(&whole, "a/b")[1], Some(name_hash64(b"a/b")));
    }

    #[test]
    fn a_walk_marks_the_gta_vi_archives_on_disk() {
        use or_io::container::rpf::crypto::NoOpCryptoProvider;
        use or_io::container::RpfBuilder;

        let dir = tempfile::tempdir().expect("a scratch directory");
        let magic = RpfVersion::Rpf9.magic().expect("RPF9 has a magic");
        let mut head = magic.to_le_bytes().to_vec();
        head.resize(64, 0);
        std::fs::write(dir.path().join("six.rpf"), head).expect("written");
        RpfBuilder::default()
            .insert_with("a.txt", b"a".to_vec(), false)
            .expect("a file")
            .save(dir.path().join("five.rpf"))
            .expect("saved");
        let (tx, rx) = channel();
        let provider: Arc<dyn RpfCryptoProvider> = Arc::new(NoOpCryptoProvider::new());
        walk_disk(dir.path(), provider, None, &tx, &AtomicBool::new(false));
        drop(tx);
        let rpf9 = rx.iter().find_map(|message| match message {
            Message::Finished { rpf9, .. } => Some(rpf9),
            _ => None,
        });
        assert_eq!(rpf9, Some(HashSet::from([Box::from("six.rpf")])));
    }

    /// Every row of the build's smallest archives is found by the name hash its table states,
    /// save a row the tree renamed.
    #[test]
    fn every_row_of_the_builds_smallest_archives_is_found_by_its_stated_name_hash() {
        use or_io::container::rpf::Gta6Context;
        use or_io::container::RpfArchive;

        const WHAT: &str = "a GTA VI row is found by the name hash its table states";
        const ARCHIVES: usize = 8;
        let build_root = std::env::var_os("OPENRAGE_GTA6_BUILD").map(PathBuf::from);
        let (Some(build_root), Ok(keys)) =
            (build_root, crate::state::rpf9_scan::rpf9_table::keys())
        else {
            or_test_support::skip_install(WHAT, "set OPENRAGE_GTA6_BUILD and OPENRAGE_GTA6_KEYS");
            return;
        };
        let names = crate::state::rpf9_scan::rpf9_table::resolver();
        let provider: Arc<dyn RpfCryptoProvider> = Arc::new(Gta6Context {
            keys,
            names: Arc::clone(&names),
        });
        let mut disks: Vec<(u64, PathBuf)> = std::fs::read_dir(&build_root)
            .expect("the build root lists")
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("rpf")))
            .filter_map(|p| Some((std::fs::metadata(&p).ok()?.len(), p)))
            .collect();
        disks.sort();
        let (mut rows, mut renamed, mut missed) = (0usize, 0usize, Vec::new());
        for (_, disk) in disks.into_iter().take(ARCHIVES) {
            assert!(is_rpf9(&disk), "{} is a GTA VI archive", disk.display());
            let rel = crawl::display_rel(&build_root, &disk);
            let rpf9 = HashSet::from([Box::<str>::from(rel.as_str())]);
            let archive = RpfArchive::open_with(&disk, Arc::clone(&provider))
                .unwrap_or_else(|e| panic!("{} opens: {e}", disk.display()));
            for node in archive.nodes.iter().filter(|node| !node.is_dir()) {
                let Some(row) = archive.rpf9_row(node.entry_index()) else {
                    continue;
                };
                let path = format!("{rel}/{}", node.path());
                // A row whose path in its archive is not the name its hash states is one the tree
                // renamed (numbered, split at a `\`, trimmed), which the index does not find.
                let inner = inside_archive(&path);
                let stated = names.lookup(row.name64);
                if stated.is_some_and(|n| n != inner && Some(n) != inner.strip_suffix(".rpf")) {
                    renamed += 1;
                    continue;
                }
                rows += 1;
                if !names64_of(&rpf9, &path).contains(&Some(row.name64)) {
                    missed.push(path);
                }
            }
        }
        assert!(rows > 0, "the smallest archives list rows");
        assert!(
            missed.is_empty(),
            "{} of {rows} rows ({renamed} renamed rows left out) are not found by their hash, such as {:?}",
            missed.len(),
            &missed[..missed.len().min(5)]
        );
    }

    #[test]
    fn an_archive_not_reopened_keeps_what_it_held() {
        let previous = Listing {
            files: [
                "a.rpf",
                "a.rpf/x.ydr",
                "a.rpf/n.rpf/y.ydr",
                "a.rpfx",
                "b.rpf",
                "b.rpf/old.ydr",
            ]
            .map(Box::from)
            .to_vec(),
            ..Listing::default()
        };
        let kept: HashSet<Box<str>> = HashSet::from([Box::from("a.rpf")]);
        let found: Vec<Box<str>> = ["b.rpf", "b.rpf/new.ydr", "a.rpf", "a.rpfx"]
            .map(Box::from)
            .to_vec();
        let files = with_kept_archives(found, &previous, &kept);
        assert_eq!(
            files.iter().map(|f| &**f).collect::<Vec<_>>(),
            [
                "b.rpf",
                "b.rpf/new.ydr",
                "a.rpf",
                "a.rpf/x.ydr",
                "a.rpf/n.rpf/y.ydr",
                "a.rpfx"
            ]
        );
    }
}
