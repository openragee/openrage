//! The resources of a server read together: every resource under its resources folder, with what each streams and what each declares.
//!
//! [`scan`] walks the folder through its category folders in brackets and reads each file once: a manifest through
//! [`crate::resource::manifest`], a streamed file for its size and the digest of its bytes, and a type file or a map for the names it
//! states. [`findings`] says what two resources hold under one name, [`concerning`] says what of a scan a build is about to write
//! again, and [`config`] reads the order a server's configuration starts its resources in.
//!
//! ⚠ It reads the files and not a server: nothing here says which of two copies a server loads, only which resource a configuration
//! starts last. That a folder in brackets is a category, and that a streamed file is resolved by its name alone, were read from no server.

use std::io::Read;
use std::path::{Path, PathBuf};

use or_io::decl::File;
use or_io::meta::instance::HashedString;
use or_io::meta::typed::{CMapData, CMapTypes};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::paths::relative_inside;
use crate::resource::manifest::{Manifest, NAMES};
use crate::resource::{shown, stream_files, ESCROW_MARKER};

pub mod config;
mod findings;

pub use findings::{
    concerning, findings, placement, rule_of, Finding, GameFiles, Report, Summary, ARCHETYPE_CLAIM,
    RULES,
};

/// The most bytes of a manifest or a configuration that are read: a text file past it is said not to read.
const TEXT_LIMIT: u64 = 8 << 20;

/// The most bytes of a type file or a map that are held while its names are read; a larger one is digested and its names are not known.
const HELD_LIMIT: u64 = 256 << 20;

/// How many bytes of a streamed file are held at once while it is digested.
const CHUNK: usize = 64 << 10;

/// How a scan and a configuration open a file, which is where a test counts the reads.
pub trait Files {
    /// Open the file at `path` at its first byte.
    fn open(&self, path: &Path) -> std::io::Result<Box<dyn Read + '_>>;
}

/// The files of the disk.
pub struct Disk;

impl Files for Disk {
    fn open(&self, path: &Path) -> std::io::Result<Box<dyn Read + '_>> {
        Ok(Box::new(std::fs::File::open(path)?))
    }
}

/// A file under the stream folder of a resource, as a scan keeps it: no byte of it is held.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Streamed {
    /// Inside the resource, with `/`.
    pub path: String,
    /// The file's name in lower case, the key a server resolves a streamed file by.
    pub name: String,
    /// How many bytes were read of it.
    pub bytes: u64,
    /// The SHA-256 of its bytes in hexadecimal; none when it could not be read to its end.
    pub digest: Option<String>,
    /// Whether an asset escrow locks it: its bytes are digested and no reader is handed them.
    pub locked: bool,
}

/// A name a type file or a map of a resource states.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Declared {
    /// The name as a person reads it: its text where the file keeps one, else the hash as `hash_XXXXXXXX`.
    pub name: String,
    /// The hash the game keys the name by.
    pub hash: u32,
    /// The file that states it, inside the resource with `/`.
    pub file: String,
}

/// One resource of a server.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Resource {
    /// Under the server's folder with `/`, through its categories: `[maps]/harbour`.
    pub path: String,
    /// The folder's name, which is what a server calls the resource.
    pub name: String,
    /// The manifest read, by its name.
    pub manifest: String,
    /// Whether the manifest declares the resource a map.
    pub map: bool,
    /// Every file under its stream folder, in path order.
    pub streamed: Vec<Streamed>,
    /// The archetypes its type files declare.
    pub archetypes: Vec<Declared>,
    /// The names its maps state for themselves.
    pub maps: Vec<Declared>,
    /// Whether a type file or a map was locked or did not open, so `archetypes` and `maps` are not all it declares.
    pub names_incomplete: bool,
    /// What was not read of it, one sentence each.
    pub notes: Vec<String>,
}

impl Resource {
    /// How many of its streamed files an asset escrow locks.
    pub fn locked(&self) -> usize {
        self.streamed.iter().filter(|f| f.locked).count()
    }
}

/// A folder that could not be listed, so what it holds is not known.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unlisted {
    /// Under the server's folder with `/`; empty for the folder itself.
    pub path: String,
    pub why: String,
}

/// What a server's resources folder holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Scan {
    pub folder: PathBuf,
    /// Every resource, in path order.
    pub resources: Vec<Resource>,
    /// The folders that are neither a category in brackets nor a resource with a manifest, which were not walked, in path order.
    pub ignored: Vec<String>,
    /// The category folders that could not be listed.
    pub unlisted: Vec<Unlisted>,
}

impl Scan {
    /// ⚠ A folder that was not listed, or a type file that did not open, makes the names here a short list that looks exhaustive.
    pub fn is_whole(&self) -> bool {
        self.unlisted.is_empty() && !self.resources.iter().any(|r| r.names_incomplete)
    }
}

/// Whether a folder of the name holds resources and is none itself.
pub fn is_category(name: &str) -> bool {
    name.len() >= 2 && name.starts_with('[') && name.ends_with(']')
}

/// Read every resource under `folder`, each file once.
pub fn scan(folder: &Path) -> Scan {
    scan_with(folder, &Disk)
}

/// [`scan`], opening each file through `files`.
pub fn scan_with(folder: &Path, files: &dyn Files) -> Scan {
    let mut scan = Scan {
        folder: folder.to_path_buf(),
        ..Scan::default()
    };
    let mut found: Vec<(String, PathBuf, &'static str)> = Vec::new();
    // ⚠ Categories nest to any depth, so they wait on a list and not on the stack.
    let mut categories = vec![folder.to_path_buf()];
    while let Some(dir) = categories.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(why) => {
                scan.unlisted.push(Unlisted {
                    path: relative_inside(&dir, folder).unwrap_or_default(),
                    why: why.to_string(),
                });
                continue;
            }
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            // A hidden entry and a link are left out, so the walk ends; a file beside the resources is none of them.
            if name.starts_with('.') || !kind.is_dir() {
                continue;
            }
            let at = entry.path();
            let Some(path) = relative_inside(&at, folder) else {
                continue;
            };
            let manifest = NAMES.into_iter().find(|name| is_plain_file(&at.join(name)));
            match manifest {
                _ if is_category(&name) => categories.push(at),
                Some(manifest) => found.push((path, at, manifest)),
                None => scan.ignored.push(path),
            }
        }
    }
    found.sort();
    scan.ignored.sort();
    scan.unlisted.sort_by(|a, b| a.path.cmp(&b.path));
    scan.resources = (found.iter())
        .map(|(path, at, manifest)| read_resource(path, at, manifest, files))
        .collect();
    scan
}

/// Whether `at` is a file and no link to one.
fn is_plain_file(at: &Path) -> bool {
    std::fs::symlink_metadata(at).is_ok_and(|found| found.is_file())
}

fn read_resource(path: &str, at: &Path, manifest: &str, files: &dyn Files) -> Resource {
    let mut resource = Resource {
        path: path.to_string(),
        name: path.rsplit('/').next().unwrap_or(path).to_string(),
        manifest: manifest.to_string(),
        ..Resource::default()
    };
    match text_of(files, &at.join(manifest)) {
        Ok(text) => {
            let read = Manifest::read(&text);
            resource.map = read.is_map();
            if !read.unread.is_empty() {
                resource.notes.push(format!(
                    "{} run(s) of lines of `{manifest}` are not statements this reads, so what they state is not known",
                    read.unread.len()
                ));
            }
        }
        Err(why) => resource.notes.push(format!(
            "`{manifest}` cannot be read, so nothing it states is known: {why}"
        )),
    }

    let (streamed, unlisted) = stream_files(at);
    for (dir, why) in unlisted {
        resource.names_incomplete = true;
        resource.notes.push(format!(
            "`{dir}` cannot be listed, so the files in it are not known: {why}"
        ));
    }
    let mut locked_names = 0usize;
    let mut unopened: Vec<(String, String)> = Vec::new();
    for file in streamed {
        let states = matches!(file.extension.as_str(), "ytyp" | "ymap");
        let mut kept = Streamed {
            path: file.path,
            name: file.name,
            bytes: 0,
            digest: None,
            locked: false,
        };
        match read_once(files, &file.at, states.then_some(HELD_LIMIT)) {
            Ok(read) => {
                kept.bytes = read.bytes;
                kept.digest = Some(read.digest);
                kept.locked = read.locked;
                if states && read.locked {
                    locked_names += 1;
                } else if states {
                    let stated = match read.held {
                        Some(bytes) => resource.names_of(&kept.path, &file.extension, &bytes),
                        None => Err(format!(
                            "is larger than the {HELD_LIMIT} bytes held of a file to read its names"
                        )),
                    };
                    if let Err(why) = stated {
                        unopened.push((kept.path.clone(), why));
                    }
                }
            }
            Err(why) => {
                resource.names_incomplete |= states;
                resource.notes.push(format!(
                    "`{}` cannot be read, so its bytes are not compared: {why}",
                    kept.path
                ));
            }
        }
        resource.streamed.push(kept);
    }
    // One sentence however many do not open: a resource may stream thousands of maps a reader refuses for one reason.
    match unopened.as_slice() {
        [] => {}
        [(path, why)] => resource.notes.push(format!(
            "`{path}` {why}, so the names it states are not known"
        )),
        [(path, why), ..] => resource.notes.push(format!(
            "{} type files or maps do not open, so the names they state are not known; the first, `{path}`, {why}",
            unopened.len()
        )),
    }
    resource.names_incomplete |= !unopened.is_empty();
    if locked_names > 0 {
        resource.names_incomplete = true;
        resource.notes.push(format!(
            "{locked_names} type file(s) or map(s) are locked by an asset escrow and not opened, so the names they state are not known"
        ));
    }
    resource
}

impl Resource {
    /// Keep the names the type file or the map at `path` states, or say why it does not open.
    fn names_of(&mut self, path: &str, extension: &str, bytes: &[u8]) -> Result<(), String> {
        // ⚠ The readers of metadata also take the text a source is kept as, which no game streams.
        if or_io::meta::detect_binary_format(bytes).is_none() {
            return Err("is not metadata in a binary form".to_string());
        }
        let declared = |name: &HashedString| Declared {
            name: shown(name),
            hash: name.hash,
            file: path.to_string(),
        };
        if extension == "ytyp" {
            let types = CMapTypes::from_bytes(bytes).map_err(|e| format!("does not open ({e})"))?;
            let names = types.archetypes.iter().map(|a| declared(&a.name));
            self.archetypes.extend(names);
        } else {
            let map = CMapData::from_bytes(bytes).map_err(|e| format!("does not open ({e})"))?;
            if map.name.hash != 0 {
                self.maps.push(declared(&map.name));
            }
        }
        Ok(())
    }
}

/// What one read of a file gave.
struct Once {
    bytes: u64,
    digest: String,
    locked: bool,
    /// The file's bytes where they were asked for, no escrow locks it and it is within the limit asked.
    held: Option<Vec<u8>>,
}

/// Read the file at `at` once, from its first byte to its last, holding its bytes only when `hold` gives the most to hold and it is not locked.
fn read_once(files: &dyn Files, at: &Path, hold: Option<u64>) -> std::io::Result<Once> {
    let mut reader = files.open(at)?;
    let mut head = Vec::with_capacity(ESCROW_MARKER.len());
    (reader.by_ref().take(ESCROW_MARKER.len() as u64)).read_to_end(&mut head)?;
    let locked = head == *ESCROW_MARKER;
    let mut hasher = Sha256::new();
    hasher.update(&head);
    let mut bytes = head.len() as u64;
    let mut held = (hold.is_some() && !locked).then(|| head.clone());
    let mut chunk = vec![0u8; CHUNK];
    loop {
        let read = match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => read,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        hasher.update(&chunk[..read]);
        bytes += read as u64;
        if hold.is_some_and(|limit| bytes > limit) {
            held = None;
        }
        if let Some(held) = &mut held {
            held.extend_from_slice(&chunk[..read]);
        }
    }
    Ok(Once {
        bytes,
        digest: format!("{:x}", hasher.finalize()),
        locked,
        held,
    })
}

/// The text of the manifest or the configuration at `at`, read once.
pub(crate) fn text_of(files: &dyn Files, at: &Path) -> Result<String, String> {
    let mut bytes = Vec::new();
    let reader = files.open(at).map_err(|e| e.to_string())?;
    (reader.take(TEXT_LIMIT + 1))
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > TEXT_LIMIT {
        return Err(format!(
            "it is larger than the {TEXT_LIMIT} bytes read of a text file"
        ));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_in_brackets_is_a_category_and_no_other_is() {
        for category in ["[maps]", "[a b]", "[]"] {
            assert!(is_category(category), "{category}");
        }
        for resource in ["maps", "[maps", "maps]", "[", "]", ""] {
            assert!(!is_category(resource), "{resource}");
        }
    }

    #[test]
    fn a_file_is_digested_whole_and_held_only_when_asked_and_not_locked() {
        let dir = tempfile::tempdir().expect("a scratch folder");
        let body: Vec<u8> = (0..3 * CHUNK + 17).map(|i| (i % 251) as u8).collect();
        let plain = dir.path().join("plain.bin");
        std::fs::write(&plain, &body).expect("writes");
        let expected = format!("{:x}", Sha256::digest(&body));

        let unheld = read_once(&Disk, &plain, None).expect("reads");
        assert_eq!((unheld.bytes, unheld.locked), (body.len() as u64, false));
        assert_eq!(unheld.digest, expected);
        assert_eq!(unheld.held, None);
        let held = read_once(&Disk, &plain, Some(HELD_LIMIT)).expect("reads");
        assert_eq!(held.held.as_deref(), Some(body.as_slice()));
        assert_eq!(held.digest, expected);
        // Held to the limit and not a byte past it, and digested whole either way.
        let within = read_once(&Disk, &plain, Some(body.len() as u64)).expect("reads");
        assert_eq!(within.held.as_deref(), Some(body.as_slice()));
        let past = read_once(&Disk, &plain, Some(body.len() as u64 - 1)).expect("reads");
        assert_eq!((past.held, past.bytes), (None, body.len() as u64));
        assert_eq!(past.digest, expected);

        let mut sealed = ESCROW_MARKER.to_vec();
        sealed.extend_from_slice(&body);
        let locked = dir.path().join("locked.bin");
        std::fs::write(&locked, &sealed).expect("writes");
        let read = read_once(&Disk, &locked, Some(HELD_LIMIT)).expect("reads");
        assert!(read.locked);
        assert_eq!(read.held, None, "a locked file is handed to no reader");
        assert_eq!(read.digest, format!("{:x}", Sha256::digest(&sealed)));

        let short = dir.path().join("short.bin");
        std::fs::write(&short, b"FX").expect("writes");
        let read = read_once(&Disk, &short, Some(HELD_LIMIT)).expect("reads");
        assert_eq!((read.bytes, read.locked), (2, false));
        assert_eq!(read.held.as_deref(), Some(b"FX".as_slice()));
    }

    #[test]
    fn a_text_file_is_read_to_the_limit_and_one_past_it_is_said_not_to_read() {
        let dir = tempfile::tempdir().expect("a scratch folder");
        let at = dir.path().join("fxmanifest.lua");
        let mut text = vec![b'-'; TEXT_LIMIT as usize];
        std::fs::write(&at, &text).expect("writes");
        assert_eq!(text_of(&Disk, &at).map(|text| text.len()), Ok(text.len()));
        text.push(b'-');
        std::fs::write(&at, &text).expect("writes");
        let refused = text_of(&Disk, &at).expect_err("one byte past the limit");
        assert!(
            refused.contains(&format!("larger than the {TEXT_LIMIT} bytes")),
            "{refused}"
        );
    }
}
