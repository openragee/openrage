//! A place to adopt from: a folder, or a zip archive read as one.
//!
//! Opening a place lists it and writes nothing. A zip is listed from its directory of entries, so what it holds is known by name and
//! size with no entry unpacked; [`Place::head`] and [`Place::read`] hand over the bytes of one file in memory, and [`Place::stage`]
//! is the one call that writes, into a hidden folder it makes inside the project, which goes again when what it returns is dropped.
//!
//! ⚠ **An archive is somebody else's input.** A name is refused unless it is a path inside the folder it is unpacked into on every
//! host, and the size an entry states is a claim: the listing holds what is stated to the limits, and an unpack counts the bytes it
//! writes and stops at the first one past what was stated. A refusal is of the whole archive, names the entry, and leaves nothing written.

use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};

use zip::{CompressionMethod, ZipArchive};

use crate::paths::is_component;

/// One bound of what a place may hold, with the reason it has the value it has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LimitRow {
    pub code: &'static str,
    pub default: u64,
    pub reason: &'static str,
}

/// Every bound, with its default: the argument of [`Place::open`] starts from these.
pub const LIMITS: &[LimitRow] = &[
    LimitRow {
        code: "entries",
        default: 50_000,
        reason: "how many files and folders a place states: a mapping is some thousands of files, and listing an archive costs memory for each entry before anything is known of it",
    },
    LimitRow {
        code: "bytes",
        default: 8 << 30,
        reason: "the unpacked size of every file together: a mapping people share is megabytes to a few gigabytes, and more than this fills the disk the project is on",
    },
    LimitRow {
        code: "ratio",
        default: 100,
        reason: "how many times its own size an archive may unpack to: the game's files are packed already and text packs to a tenth or a twentieth, while deflate reaches a thousand to one only on bytes built to repeat",
    },
    LimitRow {
        code: "ratio_floor",
        default: 64 << 20,
        reason: "the unpacked size up to which the ratio is not judged: a small archive of very regular text passes any ratio and cannot fill a disk",
    },
    LimitRow {
        code: "path",
        default: 259,
        reason: "the longest path, in UTF-16 units and counted with the folder it is unpacked into, that Windows opens with long paths off; a project is shared between hosts, so the bound is the same on each",
    },
];

/// The longest one name of a path, in bytes and in UTF-16 units: what the file systems of every host take.
pub const NAME_AT_MOST: usize = 255;

const fn default_of(code: &str) -> u64 {
    let mut at = 0;
    while at < LIMITS.len() {
        if const_eq(LIMITS[at].code, code) {
            return LIMITS[at].default;
        }
        at += 1;
    }
    panic!("no such limit")
}

const fn const_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut at = 0;
    while at < a.len() {
        if a[at] != b[at] {
            return false;
        }
        at += 1;
    }
    true
}

/// The bounds one place is opened within, each a row of [`LIMITS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub entries: u64,
    pub bytes: u64,
    pub ratio: u64,
    pub ratio_floor: u64,
    pub path: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            entries: default_of("entries"),
            bytes: default_of("bytes"),
            ratio: default_of("ratio"),
            ratio_floor: default_of("ratio_floor"),
            path: default_of("path"),
        }
    }
}

impl Limits {
    /// Set the bound `code` names; an error names the codes there are.
    pub fn set(&mut self, code: &str, value: u64) -> Result<(), String> {
        let bound = match code {
            "entries" => &mut self.entries,
            "bytes" => &mut self.bytes,
            "ratio" => &mut self.ratio,
            "ratio_floor" => &mut self.ratio_floor,
            "path" => &mut self.path,
            other => {
                let codes: Vec<&str> = LIMITS.iter().map(|row| row.code).collect();
                return Err(format!(
                    "{other} is no limit of a place: {}",
                    codes.join(", ")
                ));
            }
        };
        *bound = value;
        Ok(())
    }

    /// The bound `code` names.
    pub fn get(&self, code: &str) -> Option<u64> {
        match code {
            "entries" => Some(self.entries),
            "bytes" => Some(self.bytes),
            "ratio" => Some(self.ratio),
            "ratio_floor" => Some(self.ratio_floor),
            "path" => Some(self.path),
            _ => None,
        }
    }

    /// The most a place whose packed size is `packed` may unpack to.
    fn unpacked_at_most(&self, packed: u64) -> u64 {
        self.bytes
            .min(self.ratio_floor.max(self.ratio.saturating_mul(packed)))
    }
}

/// Why a place is not adopted from. Nothing is written when one is returned.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("{place} is not a folder or a zip archive this reads: {why}")]
    NotAPlace { place: String, why: String },
    #[error(
        "`{entry}` is an absolute path, and an entry is unpacked inside the folder of the project"
    )]
    Absolute { entry: String },
    #[error("`{entry}` climbs out of the folder it is unpacked into with `..`")]
    Climbs { entry: String },
    #[error("`{entry}` names a drive or a stream of a file with `:`")]
    Drive { entry: String },
    #[error("`{entry}` names a device of Windows ({device}), which is no file there")]
    Device { entry: String, device: String },
    #[error("`{entry}` holds a name no host can create as it is written: empty, `.`, ending in a dot or a space, or with a character a file name cannot hold")]
    NotAName { entry: String },
    #[error("`{entry}` is a link, and what a link leads to is not in the archive")]
    Link { entry: String },
    #[error("`{entry}` is neither a file nor a folder (mode {mode:o})")]
    Special { entry: String, mode: u32 },
    #[error("`{entry}` and `{other}` differ only by case, and a disk of Windows or macOS makes them one")]
    CaseTwin { entry: String, other: String },
    #[error("`{entry}` is a file where `{other}` needs a folder")]
    FileAndFolder { entry: String, other: String },
    #[error(
        "`{entry}` is {units} long with the folder it is unpacked into, and a path holds {limit}"
    )]
    PathTooLong {
        entry: String,
        units: u64,
        limit: u64,
    },
    #[error("`{entry}` is {units} long before the folder it is unpacked into is counted, and a path holds {limit}")]
    PathTooLongAlone {
        entry: String,
        units: u64,
        limit: u64,
    },
    #[error("`{entry}` holds a name {units} long, and one name of a path holds {limit}")]
    NameTooLong {
        entry: String,
        units: u64,
        limit: u64,
    },
    #[error("the place states {stated} entries, and {limit} are taken")]
    TooManyEntries { stated: u64, limit: u64 },
    #[error("`{entry}` takes the unpacked size past {limit} bytes")]
    TooLarge { entry: String, limit: u64 },
    #[error("`{entry}` takes the unpacked size past {limit} bytes, which is {ratio} times the {packed} bytes of the archive")]
    Ratio {
        entry: String,
        limit: u64,
        ratio: u64,
        packed: u64,
    },
    #[error("`{entry}` is encrypted, and no password is asked for or tried")]
    Encrypted { entry: String },
    #[error("`{entry}` unpacks to more than the {stated} bytes the archive states for it")]
    SizeLies { entry: String, stated: u64 },
    #[error("`{entry}` cannot be read: {why}")]
    Unreadable { entry: String, why: String },
    #[error("`{entry}` is a file the disk already holds under another spelling")]
    OneFile { entry: String },
    #[error("`{entry}` is not in the place")]
    NotHeld { entry: String },
    #[error("{path}: {why}")]
    Io { path: String, why: String },
}

impl Refusal {
    /// The entry the refusal is about, none where it is about the place.
    pub fn entry(&self) -> Option<&str> {
        match self {
            Self::Absolute { entry }
            | Self::Climbs { entry }
            | Self::Drive { entry }
            | Self::Device { entry, .. }
            | Self::NotAName { entry }
            | Self::Link { entry }
            | Self::Special { entry, .. }
            | Self::CaseTwin { entry, .. }
            | Self::FileAndFolder { entry, .. }
            | Self::PathTooLong { entry, .. }
            | Self::PathTooLongAlone { entry, .. }
            | Self::NameTooLong { entry, .. }
            | Self::TooLarge { entry, .. }
            | Self::Ratio { entry, .. }
            | Self::Encrypted { entry }
            | Self::SizeLies { entry, .. }
            | Self::Unreadable { entry, .. }
            | Self::OneFile { entry }
            | Self::NotHeld { entry } => Some(entry),
            Self::NotAPlace { .. } | Self::TooManyEntries { .. } | Self::Io { .. } => None,
        }
    }

    /// A word for the kind of refusal, which a document names it by.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotAPlace { .. } => "not-a-place",
            Self::Absolute { .. } => "absolute",
            Self::Climbs { .. } => "climbs",
            Self::Drive { .. } => "drive",
            Self::Device { .. } => "device",
            Self::NotAName { .. } => "not-a-name",
            Self::Link { .. } => "link",
            Self::Special { .. } => "special",
            Self::CaseTwin { .. } => "case-twin",
            Self::FileAndFolder { .. } => "file-and-folder",
            Self::PathTooLong { .. } | Self::PathTooLongAlone { .. } | Self::NameTooLong { .. } => {
                "path-too-long"
            }
            Self::TooManyEntries { .. } => "too-many-entries",
            Self::TooLarge { .. } => "too-large",
            Self::Ratio { .. } => "ratio",
            Self::Encrypted { .. } => "encrypted",
            Self::SizeLies { .. } => "size-lies",
            Self::Unreadable { .. } => "unreadable",
            Self::OneFile { .. } => "one-file",
            Self::NotHeld { .. } => "not-held",
            Self::Io { .. } => "io",
        }
    }
}

/// One file a place holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    /// Inside the place, with `/` on every host.
    pub path: String,
    /// The size the place states, which a read holds it to.
    pub bytes: u64,
}

/// Something of a place that is not adopted from, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Left {
    pub path: String,
    pub why: String,
}

/// The extensions of a file that is an archive of other files, which is listed and never opened.
pub const ARCHIVES: &[&str] = &["zip", "7z", "rar", "tar", "gz", "tgz", "oiv", "rpf"];

/// The extension of `path`, lower case, empty where it has none.
pub fn extension_of(path: &str) -> String {
    let leaf = path.rsplit('/').next().unwrap_or(path);
    leaf.rsplit_once('.')
        .map_or(String::new(), |(_, e)| e.to_ascii_lowercase())
}

/// Whether `path` names an archive of other files by its extension.
pub fn is_archive_name(path: &str) -> bool {
    ARCHIVES.contains(&extension_of(path).as_str())
}

/// The device of Windows `name` is, whatever follows its first dot.
fn device_of(name: &str) -> Option<String> {
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    let upper = stem.to_uppercase();
    let numbered = |prefix: &str| {
        upper.strip_prefix(prefix).is_some_and(|rest| {
            let mut digits = rest.chars();
            matches!(
                (digits.next(), digits.next()),
                (Some('0'..='9' | '¹' | '²' | '³'), None)
            )
        })
    };
    let is = matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || numbered("COM")
        || numbered("LPT");
    is.then_some(upper)
}

/// The names of the path `entry` states, or why it is no path inside a folder on every host.
///
/// ⚠ Both separators end a name whatever the host: a `\` is a separator to Windows and an archive written there may state one.
fn names_of(entry: &str) -> Result<Vec<&str>, Refusal> {
    let refused = || entry.to_string();
    let is_separator = |c: char| c == '/' || c == '\\';
    if entry.starts_with(is_separator) {
        return Err(Refusal::Absolute { entry: refused() });
    }
    // A folder is stated with a separator after its name.
    let path = entry.strip_suffix(is_separator).unwrap_or(entry);
    let names: Vec<&str> = path.split(is_separator).collect();
    for name in &names {
        if *name == ".." {
            return Err(Refusal::Climbs { entry: refused() });
        }
        if name.contains(':') {
            return Err(Refusal::Drive { entry: refused() });
        }
        if let Some(device) = device_of(name) {
            return Err(Refusal::Device {
                entry: refused(),
                device,
            });
        }
        if !is_component(name) {
            return Err(Refusal::NotAName { entry: refused() });
        }
        let units = name.encode_utf16().count().max(name.len());
        if units > NAME_AT_MOST {
            return Err(Refusal::NameTooLong {
                entry: refused(),
                units: units as u64,
                limit: NAME_AT_MOST as u64,
            });
        }
    }
    Ok(names)
}

/// The paths a listing has taken so far, by their spelling in lower case, each with the entry that stated it.
#[derive(Default)]
struct Taken {
    files: HashMap<String, String>,
    /// A folder also keeps how it was spelled, since many entries lead through one.
    folders: HashMap<String, (String, String)>,
}

impl Taken {
    /// Take the folders `names` lead through, then the file they end in when `file`.
    fn take(&mut self, entry: &str, names: &[&str], file: bool) -> Result<(), Refusal> {
        let folders = if file { names.len() - 1 } else { names.len() };
        let mut path = String::new();
        for (depth, name) in names.iter().enumerate() {
            if depth > 0 {
                path.push('/');
            }
            path.push_str(name);
            let key = path.to_lowercase();
            let is_folder = depth < folders;
            if is_folder {
                if let Some(other) = self.files.get(&key) {
                    return Err(Refusal::FileAndFolder {
                        entry: other.clone(),
                        other: entry.to_string(),
                    });
                }
                match self.folders.get(&key) {
                    Some((spelled, other)) if *spelled != path => {
                        return Err(Refusal::CaseTwin {
                            entry: entry.to_string(),
                            other: other.clone(),
                        });
                    }
                    Some(_) => {}
                    None => {
                        self.folders.insert(key, (path.clone(), entry.to_string()));
                    }
                }
            } else {
                if let Some((_, other)) = self.folders.get(&key) {
                    return Err(Refusal::FileAndFolder {
                        entry: entry.to_string(),
                        other: other.clone(),
                    });
                }
                if let Some(other) = self.files.insert(key, entry.to_string()) {
                    return Err(Refusal::CaseTwin {
                        entry: entry.to_string(),
                        other,
                    });
                }
            }
        }
        Ok(())
    }
}

/// Why a name that begins with a dot is left out, with everything under it.
const HIDDEN: &str = "hidden: a name that begins with a dot is not read, and nothing under it is";

/// The path up to the first of `names` that begins with a dot, none when none does.
fn hidden_part(names: &[&str]) -> Option<String> {
    let at = names.iter().position(|name| name.starts_with('.'))?;
    Some(names[..=at].join("/"))
}

type Zip = ZipArchive<BufReader<File>>;

enum Kind {
    Folder,
    /// The archive, and the number of each file's entry by its path.
    Zip(Box<Zip>, HashMap<String, usize>),
}

/// A folder or a zip archive, listed.
pub struct Place {
    at: PathBuf,
    kind: Kind,
    files: Vec<Held>,
    left: Vec<Left>,
    limits: Limits,
    /// The size of the archive itself, which no entry of it states; none for a folder.
    packed: Option<u64>,
}

impl std::fmt::Debug for Place {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Place")
            .field("at", &self.at)
            .field("archive", &self.is_archive())
            .field("files", &self.files.len())
            .finish()
    }
}

fn io_refusal(path: &Path, e: &std::io::Error) -> Refusal {
    Refusal::Io {
        path: path.display().to_string(),
        why: e.to_string(),
    }
}

fn units_of(text: &str) -> u64 {
    text.encode_utf16().count() as u64
}

impl Place {
    /// List the folder or the zip archive at `at` within `limits`, writing nothing.
    pub fn open(at: &Path, limits: &Limits) -> Result<Self, Refusal> {
        let mut place = Self {
            at: at.to_path_buf(),
            kind: Kind::Folder,
            files: Vec::new(),
            left: Vec::new(),
            limits: *limits,
            packed: None,
        };
        if at.is_dir() {
            place.list_folder()?;
        } else {
            place.list_zip()?;
        }
        place.files.sort_by(|a, b| a.path.cmp(&b.path));
        place.left.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(place)
    }

    fn list_zip(&mut self) -> Result<(), Refusal> {
        let not_a_place = |why: String| Refusal::NotAPlace {
            place: self.at.display().to_string(),
            why,
        };
        let file = File::open(&self.at).map_err(|e| not_a_place(e.to_string()))?;
        let packed = file
            .metadata()
            .map_err(|e| not_a_place(e.to_string()))?
            .len();
        let mut archive =
            ZipArchive::new(BufReader::new(file)).map_err(|e| not_a_place(e.to_string()))?;
        let stated = archive.len() as u64;
        if stated > self.limits.entries {
            return Err(Refusal::TooManyEntries {
                stated,
                limit: self.limits.entries,
            });
        }
        let at_most = self.limits.unpacked_at_most(packed);
        let mut taken = Taken::default();
        let mut index = HashMap::new();
        let mut hidden = BTreeSet::new();
        let mut total = 0u64;
        for number in 0..archive.len() {
            // ⚠ Read raw: nothing of the entry is unpacked or decrypted to learn what it states.
            let entry = archive
                .by_index_raw(number)
                .map_err(|e| Refusal::Unreadable {
                    entry: format!("entry {number}"),
                    why: e.to_string(),
                })?;
            let name = entry.name().to_string();
            let names = names_of(&name)?;
            self.holds_alone(&name, &names)?;
            if let Some(mode) = entry.unix_mode() {
                // The type of a file is the four bits above its permissions: a folder, a plain file and a link are these three.
                match mode & 0o170000 {
                    0 | 0o040000 | 0o100000 => {}
                    0o120000 => return Err(Refusal::Link { entry: name }),
                    _ => return Err(Refusal::Special { entry: name, mode }),
                }
            }
            if entry.encrypted() {
                return Err(Refusal::Encrypted { entry: name });
            }
            let folder = entry.is_dir();
            taken.take(&name, &names, !folder)?;
            // ⚠ A hidden entry is left out only here, after every refusal above: an archive is refused for an entry wherever it is.
            let hidden_at = hidden_part(&names);
            if folder {
                hidden.extend(hidden_at);
                continue;
            }
            match entry.compression() {
                CompressionMethod::Stored | CompressionMethod::Deflated => {}
                other => {
                    return Err(Refusal::Unreadable {
                        entry: name,
                        why: format!(
                            "it is packed by a method this build does not unpack ({other})"
                        ),
                    })
                }
            }
            total = total.saturating_add(entry.size());
            if total > at_most {
                return Err(self.past(name, packed));
            }
            if let Some(hidden_at) = hidden_at {
                hidden.insert(hidden_at);
                continue;
            }
            let path = names.join("/");
            index.insert(path.clone(), number);
            self.files.push(Held {
                path,
                bytes: entry.size(),
            });
        }
        self.left.extend(hidden.into_iter().map(|path| Left {
            path,
            why: HIDDEN.into(),
        }));
        self.packed = Some(packed);
        self.kind = Kind::Zip(Box::new(archive), index);
        Ok(())
    }

    /// Refuses a path longer by itself than a path holds, which no folder could take.
    ///
    /// ⚠ This also bounds the listing: every folder on the way to a file is kept by its whole path, which costs the square of a
    /// path's length, and an archive states names of tens of thousands of bytes.
    fn holds_alone(&self, entry: &str, names: &[&str]) -> Result<(), Refusal> {
        let separators = names.len().saturating_sub(1) as u64;
        let units = names.iter().map(|name| units_of(name)).sum::<u64>() + separators;
        if units > self.limits.path {
            return Err(Refusal::PathTooLongAlone {
                entry: entry.to_string(),
                units,
                limit: self.limits.path,
            });
        }
        Ok(())
    }

    /// The refusal of the entry that took the unpacked size past what the limits allow an archive of `packed` bytes.
    fn past(&self, entry: String, packed: u64) -> Refusal {
        let limit = self.limits.unpacked_at_most(packed);
        if limit < self.limits.bytes {
            Refusal::Ratio {
                entry,
                limit,
                ratio: self.limits.ratio,
                packed,
            }
        } else {
            Refusal::TooLarge { entry, limit }
        }
    }

    fn list_folder(&mut self) -> Result<(), Refusal> {
        let mut taken = Taken::default();
        let mut total = 0u64;
        let mut stated = 0u64;
        let mut stack = vec![(self.at.clone(), String::new())];
        while let Some((dir, inside)) = stack.pop() {
            let mut entries: Vec<_> = std::fs::read_dir(&dir)
                .map_err(|e| io_refusal(&dir, &e))?
                .collect::<Result<_, _>>()
                .map_err(|e| io_refusal(&dir, &e))?;
            entries.sort_by_key(std::fs::DirEntry::file_name);
            for entry in entries {
                stated += 1;
                if stated > self.limits.entries {
                    return Err(Refusal::TooManyEntries {
                        stated,
                        limit: self.limits.entries,
                    });
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                let hidden = name.starts_with('.');
                let path = if inside.is_empty() {
                    name
                } else {
                    format!("{inside}/{name}")
                };
                // ⚠ Not entered: the folder a version control keeps is thousands of files that are no part of what is adopted.
                if hidden {
                    self.left.push(Left {
                        path,
                        why: HIDDEN.into(),
                    });
                    continue;
                }
                let kind = entry
                    .file_type()
                    .map_err(|e| io_refusal(&entry.path(), &e))?;
                if kind.is_symlink() {
                    self.left.push(Left {
                        path,
                        why: "a link: it is not followed, so nothing behind it is adopted".into(),
                    });
                    continue;
                }
                if !kind.is_dir() && !kind.is_file() {
                    self.left.push(Left {
                        path,
                        why: "neither a file nor a folder, so it is not read".into(),
                    });
                    continue;
                }
                // ⚠ One name here and a folder and a file on Windows, so the project made of it would differ between hosts.
                if path.contains('\\') {
                    return Err(Refusal::NotAName { entry: path });
                }
                let names = names_of(&path)?;
                self.holds_alone(&path, &names)?;
                taken.take(&path, &names, !kind.is_dir())?;
                if kind.is_dir() {
                    stack.push((entry.path(), path));
                    continue;
                }
                let bytes = entry
                    .metadata()
                    .map_err(|e| io_refusal(&entry.path(), &e))?
                    .len();
                total = total.saturating_add(bytes);
                if total > self.limits.bytes {
                    return Err(Refusal::TooLarge {
                        entry: path,
                        limit: self.limits.bytes,
                    });
                }
                self.files.push(Held { path, bytes });
            }
        }
        Ok(())
    }

    /// Where the place is.
    pub fn at(&self) -> &Path {
        &self.at
    }

    /// Whether the place is an archive, read without unpacking it.
    pub fn is_archive(&self) -> bool {
        matches!(self.kind, Kind::Zip(..))
    }

    /// Every file of the place, in path order.
    pub fn files(&self) -> &[Held] {
        &self.files
    }

    /// What of the place is not adopted from, in path order: a hidden name with what is under it, the links of a folder, and what in it is neither a file nor a folder.
    pub fn left(&self) -> &[Left] {
        &self.left
    }

    /// The bounds the place was opened within.
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    fn held(&self, path: &str) -> Result<&Held, Refusal> {
        self.files
            .binary_search_by(|held| held.path.as_str().cmp(path))
            .map(|at| &self.files[at])
            .map_err(|_| Refusal::NotHeld {
                entry: path.to_string(),
            })
    }

    /// The reader of the file `path`, and the size it states.
    fn reader(&mut self, path: &str) -> Result<(Box<dyn Read + '_>, u64), Refusal> {
        let stated = self.held(path)?.bytes;
        let unreadable = |why: String| Refusal::Unreadable {
            entry: path.to_string(),
            why,
        };
        match &mut self.kind {
            Kind::Folder => {
                let at = self
                    .at
                    .join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
                let file = File::open(at).map_err(|e| unreadable(e.to_string()))?;
                Ok((Box::new(file), stated))
            }
            Kind::Zip(archive, index) => {
                let number = *index.get(path).ok_or_else(|| Refusal::NotHeld {
                    entry: path.to_string(),
                })?;
                let entry = archive
                    .by_index(number)
                    .map_err(|e| unreadable(e.to_string()))?;
                Ok((Box::new(entry), stated))
            }
        }
    }

    /// The first `at_most` bytes of the file `path`, fewer when it is shorter; nothing is written.
    pub fn head(&mut self, path: &str, at_most: usize) -> Result<Vec<u8>, Refusal> {
        let (reader, _) = self.reader(path)?;
        let mut head = Vec::with_capacity(at_most);
        reader
            .take(at_most as u64)
            .read_to_end(&mut head)
            .map_err(|e| Refusal::Unreadable {
                entry: path.to_string(),
                why: e.to_string(),
            })?;
        Ok(head)
    }

    /// The bytes of the file `path`, held to the size the place states for it; nothing is written.
    pub fn read(&mut self, path: &str) -> Result<Vec<u8>, Refusal> {
        let (reader, stated) = self.reader(path)?;
        let mut bytes = Vec::new();
        // One byte past what was stated is enough to know the statement is false.
        reader
            .take(stated.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|e| Refusal::Unreadable {
                entry: path.to_string(),
                why: e.to_string(),
            })?;
        if bytes.len() as u64 > stated {
            return Err(Refusal::SizeLies {
                entry: path.to_string(),
                stated,
            });
        }
        Ok(bytes)
    }

    /// Whether every path of `wanted` is one inside a staging folder of `project` that a path can hold: the check [`Place::stage`] makes first.
    pub fn fits(&self, project: &Path, wanted: &[Wanted<'_>]) -> Result<(), Refusal> {
        let folder = std::path::absolute(project).unwrap_or_else(|_| project.to_path_buf());
        // The folder, a separator, the staging folder's name and another separator come before the path.
        let before = units_of(&folder.to_string_lossy()) + 2 + STAGING_NAME_UNITS;
        for want in wanted {
            // Where a file is written is a path inside the staging folder, whoever names it.
            names_of(want.to)?;
            let units = before + units_of(want.to);
            if units > self.limits.path {
                return Err(Refusal::PathTooLong {
                    entry: want.from.to_string(),
                    units,
                    limit: self.limits.path,
                });
            }
        }
        Ok(())
    }

    /// Unpack or copy the files `wanted` into a new hidden folder inside `project`, which goes again when what is returned is dropped.
    ///
    /// ⚠ A file is made new and never written over, and the bytes written are counted: the first past what an entry states ends the
    /// unpack and removes the folder with everything in it.
    pub fn stage(&mut self, project: &Path, wanted: &[Wanted<'_>]) -> Result<Staged, Refusal> {
        self.fits(project, wanted)?;
        let folder = project.join(format!("{STAGING_PREFIX}{}", staging_id()));
        std::fs::create_dir(&folder).map_err(|e| io_refusal(&folder, &e))?;
        let staged = Staged { folder };
        let mut buffer = vec![0u8; 64 * 1024];
        for want in wanted {
            let to = staged.path(want.to);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(|e| io_refusal(parent, &e))?;
            }
            let (mut reader, stated) = self.reader(want.from)?;
            let mut file = File::options()
                .write(true)
                .create_new(true)
                .open(&to)
                .map_err(|e| match e.kind() {
                    std::io::ErrorKind::AlreadyExists => Refusal::OneFile {
                        entry: want.from.to_string(),
                    },
                    _ => io_refusal(&to, &e),
                })?;
            let mut written = 0u64;
            loop {
                let read = reader.read(&mut buffer).map_err(|e| Refusal::Unreadable {
                    entry: want.from.to_string(),
                    why: e.to_string(),
                })?;
                if read == 0 {
                    break;
                }
                written += read as u64;
                // ⚠ The listing held the sizes stated to the limits, so holding each file to its own keeps the whole within them.
                if written > stated {
                    return Err(Refusal::SizeLies {
                        entry: want.from.to_string(),
                        stated,
                    });
                }
                file.write_all(&buffer[..read])
                    .map_err(|e| io_refusal(&to, &e))?;
            }
        }
        Ok(staged)
    }
}

/// One file to stage: where it is in the place, and where under the staging folder it is written, both with `/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wanted<'a> {
    pub from: &'a str,
    pub to: &'a str,
}

/// How the name of a staging folder begins: hidden, so no walk of the project reads it.
pub const STAGING_PREFIX: &str = ".adopting-";
const STAGING_ID_UNITS: u64 = 8;
const STAGING_NAME_UNITS: u64 = STAGING_PREFIX.len() as u64 + STAGING_ID_UNITS;

fn staging_id() -> String {
    let id = crate::content::folder::new_id();
    id[..STAGING_ID_UNITS as usize].to_string()
}

/// The folder a place was staged into, removed with everything in it when this is dropped.
#[derive(Debug)]
pub struct Staged {
    folder: PathBuf,
}

impl Staged {
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Where the file staged as `to` is.
    pub fn path(&self, to: &str) -> PathBuf {
        self.folder
            .join(to.replace('/', std::path::MAIN_SEPARATOR_STR))
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_limit_has_a_reason_and_is_the_default_of_the_argument() {
        let limits = Limits::default();
        for row in LIMITS {
            assert_eq!(limits.get(row.code), Some(row.default), "{}", row.code);
            assert!(
                row.reason.len() > 40,
                "{} says why: {}",
                row.code,
                row.reason
            );
        }
        let mut changed = limits;
        for row in LIMITS {
            changed.set(row.code, 7).unwrap();
            assert_eq!(changed.get(row.code), Some(7));
        }
        let refused = changed.set("depth", 1).unwrap_err();
        assert!(
            refused.contains("entries") && refused.contains("ratio_floor"),
            "{refused}"
        );
    }

    #[test]
    fn a_name_is_a_path_inside_a_folder_on_every_host_or_it_is_refused_for_what_it_is() {
        assert_eq!(names_of("stream/a.ymap").unwrap(), ["stream", "a.ymap"]);
        assert_eq!(
            names_of("stream\\sub\\a.ymap").unwrap(),
            ["stream", "sub", "a.ymap"]
        );
        assert_eq!(names_of("stream/").unwrap(), ["stream"]);
        assert_eq!(names_of("console.lua").unwrap(), ["console.lua"]);
        assert_eq!(names_of("com10.txt").unwrap(), ["com10.txt"]);

        let code = |entry: &str| names_of(entry).unwrap_err().code();
        for entry in [
            "/etc/passwd",
            "\\windows\\x",
            "//server/share/x",
            "\\\\?\\C:\\x",
        ] {
            assert_eq!(code(entry), "absolute", "{entry}");
        }
        for entry in ["../x", "a/../../x", "a\\..\\x", "stream/..", "..\\x"] {
            assert_eq!(code(entry), "climbs", "{entry}");
        }
        for entry in ["C:/x", "c:\\x", "C:x", "stream/a.ymap:hidden"] {
            assert_eq!(code(entry), "drive", "{entry}");
        }
        for entry in [
            "NUL",
            "stream/con",
            "aux.ymap",
            "stream\\COM1.txt",
            "lpt9",
            "Prn .txt",
            "CONIN$",
        ] {
            assert_eq!(code(entry), "device", "{entry}");
        }
        for entry in [
            "", "a//b", "./a", "a/./b", "a /b", "a./b", "a\u{0}b", "a/b?", "a|b",
        ] {
            assert_eq!(code(entry), "not-a-name", "{entry:?}");
        }
        let long = "n".repeat(NAME_AT_MOST + 1);
        assert_eq!(code(&format!("stream/{long}")), "path-too-long");
        assert!(names_of(&"n".repeat(NAME_AT_MOST)).is_ok());
    }

    #[test]
    fn two_spellings_of_one_path_and_a_file_where_a_folder_is_are_told_apart() {
        let mut taken = Taken::default();
        let mut take = |entry: &str, file: bool| {
            let names = names_of(entry).unwrap();
            taken.take(entry, &names, file)
        };
        take("stream/a.ymap", true).unwrap();
        take("stream/b.ymap", true).unwrap();
        take("stream/", false).unwrap();
        assert_eq!(
            take("stream/A.ymap", true),
            Err(Refusal::CaseTwin {
                entry: "stream/A.ymap".into(),
                other: "stream/a.ymap".into()
            })
        );
        assert_eq!(
            take("Stream/c.ymap", true),
            Err(Refusal::CaseTwin {
                entry: "Stream/c.ymap".into(),
                other: "stream/a.ymap".into()
            })
        );
        assert_eq!(take("stream", true).unwrap_err().code(), "file-and-folder");
        assert_eq!(
            take("stream/a.ymap/inside", true).unwrap_err().code(),
            "file-and-folder"
        );
    }
}
