use std::collections::HashMap;
use std::ffi::OsStr;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use super::builder::{normalize, RpfBuilder};
use super::crypto::{NoOpCryptoProvider, RpfCryptoProvider, TocDecryptOutcome};
use super::entry::{RpfBinaryEntry, RpfEntry, RpfResourceEntry};
use super::header::RpfHeader;
use super::reader::read_at;
use super::version::RpfVersion;
use crate::error::{IoError, Result};

#[derive(Debug, Clone)]
pub enum RpfNode {
    Directory {
        entry_index: u32,
        name: String,
        path: String,
        children: Vec<u32>,
    },
    BinaryFile {
        entry_index: u32,
        name: String,
        path: String,
    },
    ResourceFile {
        entry_index: u32,
        name: String,
        path: String,
    },
}

impl RpfNode {
    pub fn entry_index(&self) -> u32 {
        match self {
            RpfNode::Directory { entry_index, .. }
            | RpfNode::BinaryFile { entry_index, .. }
            | RpfNode::ResourceFile { entry_index, .. } => *entry_index,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            RpfNode::Directory { name, .. }
            | RpfNode::BinaryFile { name, .. }
            | RpfNode::ResourceFile { name, .. } => name,
        }
    }

    pub fn path(&self) -> &str {
        match self {
            RpfNode::Directory { path, .. }
            | RpfNode::BinaryFile { path, .. }
            | RpfNode::ResourceFile { path, .. } => path,
        }
    }

    pub fn is_dir(&self) -> bool {
        matches!(self, RpfNode::Directory { .. })
    }

    pub fn children(&self) -> &[u32] {
        match self {
            RpfNode::Directory { children, .. } => children.as_slice(),
            _ => &[],
        }
    }
}

/// Where an archive's bytes are.
#[derive(Debug, Clone)]
pub(super) enum RpfSource {
    Memory(Arc<Vec<u8>>),
    File(PathBuf),
    /// A file inside another archive, read through it.
    Nested(Arc<RpfArchive>),
}

/// How an opened archive reads its bodies: every generation but RPF9 has a flat entry table a layout
/// reads; RPF9 keeps its decrypted table and the key-and-name session its bodies and nested archives
/// read through. The open dispatches on this, so no reader asks the version which backend it is.
#[derive(Clone)]
pub(super) enum RpfBackend {
    Flat,
    #[cfg(feature = "gta6")]
    Rpf9(Arc<super::gta6_rpf::Rpf9State>),
}

/// An opened archive of any [`RpfVersion`]: its table, read at once, and its bodies, read when asked for.
///
/// It holds the keys it was opened with, so reading never asks for them again.
#[derive(Clone)]
pub struct RpfArchive {
    pub version: RpfVersion,
    pub header: RpfHeader,
    pub entries: Vec<RpfEntry>,
    pub nodes: Vec<RpfNode>,
    pub toc_outcome: TocDecryptOutcome,
    pub(super) names: Vec<String>,
    pub(super) name_heap: Vec<u8>,
    pub(super) entry_to_node: Vec<u32>,
    pub(super) source: RpfSource,
    pub(super) start_pos: u64,
    pub(super) size: u64,
    pub(super) keys: Arc<dyn RpfCryptoProvider>,
    /// Lower-case path to entry index, built by the first lookup by path.
    pub(super) paths: OnceLock<HashMap<String, u32>>,
    /// The platform letter the stored names spell, worked out once on first ask; `Some(None)` once
    /// the names were scanned and none carried a platform extension (a hash-named table).
    pub(super) names_platform: OnceLock<Option<u8>>,
    /// Which backend reads this archive's bodies and nested archives: a flat table, or RPF9's state.
    pub(super) backend: RpfBackend,
}

impl fmt::Debug for RpfArchive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RpfArchive")
            .field("version", &self.version)
            .field("entries", &self.entries.len())
            .field("size", &self.size)
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

impl RpfArchive {
    /// The build-platform letter the archive records in its own header, where its format keeps one
    /// (RPF8); `None` for a format that does not record it (RPF7 and earlier, RPF9, IMG), whose
    /// platform has to come from the install the archive was mounted under.
    pub fn platform_letter(&self) -> Option<char> {
        self.header.platform.map(char::from)
    }

    /// The platform letter the archive's own stored names spell, for a format whose table keeps
    /// names as text (RPF7 and earlier, IMG): the most common letter among entries whose extension
    /// is a RAGE platform letter followed by a known stub (`n01.ydr` → `y`). `None` when no entry
    /// name carries one, as in a hash-named table. Scanned once and cached.
    pub fn platform_letter_from_names(&self) -> Option<char> {
        let byte = self.names_platform.get_or_init(|| {
            let mut counts = [0usize; 256];
            for name in &self.names {
                if let Some(letter) = super::platform_letter_of_name(name) {
                    counts[letter as usize] += 1;
                }
            }
            counts
                .iter()
                .enumerate()
                .filter(|(_, c)| **c > 0)
                .max_by_key(|(_, c)| **c)
                .map(|(byte, _)| byte as u8)
        });
        byte.map(char::from)
    }

    /// The archive at `path`, which needs no keys. An IMG1 is named by its `.img` or its `.dir`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path, Arc::new(NoOpCryptoProvider::new()))
    }

    /// The archive at `path`, whose encrypted parts `keys` decrypt.
    pub fn open_with(path: impl AsRef<Path>, keys: Arc<dyn RpfCryptoProvider>) -> Result<Self> {
        let path = path.as_ref();
        let extension = path
            .extension()
            .and_then(OsStr::to_str)
            .map(str::to_ascii_lowercase);
        let (file, mut directory) = match extension.as_deref() {
            Some("dir") => (path.with_extension("img"), Some(path.to_path_buf())),
            _ => (path.to_path_buf(), None),
        };
        let size = std::fs::metadata(&file)
            .map_err(|source| IoError::FileReadFailed {
                path: file.clone(),
                source,
            })?
            .len();
        let name = file.file_name().and_then(OsStr::to_str).map(str::to_owned);
        let source = RpfSource::File(file);

        // An IMG1 has no magic to say what it is: it is an `.img` that opens as nothing else, and
        // its table is the `.dir` beside it.
        let mut head = [0u8; 4];
        let sniffed = read_at(&source, 0, &mut head)
            .ok()
            .and_then(|()| RpfVersion::detect(head));
        if sniffed.is_none() && extension.as_deref() == Some("img") {
            directory = Some(path.with_extension("dir"));
        }
        let directory = directory
            .map(|dir| {
                std::fs::read(&dir).map_err(|source| IoError::FileReadFailed { path: dir, source })
            })
            .transpose()?;
        Self::parse(source, 0, size, name, keys, directory)
    }

    /// An archive held in `bytes`, which needs no keys.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        Self::from_bytes_with(None, bytes, Arc::new(NoOpCryptoProvider::new()))
    }

    /// An archive held in `bytes`, whose encrypted parts `keys` decrypt.
    ///
    /// `name` is the file's name, from which a retail archive's table key is derived.
    pub fn from_bytes_with(
        name: Option<&str>,
        bytes: Vec<u8>,
        keys: Arc<dyn RpfCryptoProvider>,
    ) -> Result<Self> {
        let size = bytes.len() as u64;
        let source = RpfSource::Memory(Arc::new(bytes));
        Self::parse(source, 0, size, name.map(str::to_owned), keys, None)
    }

    /// An IMG1 from its `.dir` and its `.img`.
    pub fn from_img1(directory: Vec<u8>, image: Vec<u8>) -> Result<Self> {
        let size = image.len() as u64;
        let source = RpfSource::Memory(Arc::new(image));
        let keys = Arc::new(NoOpCryptoProvider::new());
        Self::parse(source, 0, size, None, keys, Some(directory))
    }

    /// A builder holding every file of this archive, laid out and encrypted as it is, to add to, remove from and save.
    ///
    /// Nothing is read until the builder writes: each file is carried from here as it is stored,
    /// or for RPF7 decoded and recompressed as it was.
    pub fn edit(&self) -> Result<RpfBuilder> {
        #[cfg(feature = "gta6")]
        if matches!(self.backend, RpfBackend::Rpf9(_)) {
            return Err(IoError::Invalid {
                what: "editing an RPF9 archive".into(),
                why: "it is read-only here; the GTA VI archive writer writes a changed copy".into(),
            });
        }
        RpfBuilder::editing(self)
    }

    pub fn binary_entry(&self, entry_index: u32) -> Option<&RpfBinaryEntry> {
        match self.entries.get(entry_index as usize)? {
            RpfEntry::Binary(b) => Some(b),
            _ => None,
        }
    }

    pub fn resource_entry(&self, entry_index: u32) -> Option<&RpfResourceEntry> {
        match self.entries.get(entry_index as usize)? {
            RpfEntry::Resource(r) => Some(r),
            _ => None,
        }
    }

    pub fn node_for_entry(&self, entry_index: u32) -> Option<&RpfNode> {
        let node_idx = *self.entry_to_node.get(entry_index as usize)?;
        self.nodes.get(node_idx as usize)
    }

    /// The entry at `path`, found without regard to case or to which slash is used.
    pub fn find(&self, path: &str) -> Option<u32> {
        let paths = self.paths.get_or_init(|| {
            self.nodes
                .iter()
                .skip(1)
                .rev()
                .map(|n| (n.path().to_ascii_lowercase(), n.entry_index()))
                .collect()
        });
        paths.get(&normalize(path).ok()?).copied()
    }

    pub fn walk<F: FnMut(&RpfNode, usize)>(&self, mut visit: F) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack: Vec<(u32, usize)> = vec![(0, 0)];
        while let Some((idx, depth)) = stack.pop() {
            let node = &self.nodes[idx as usize];
            visit(node, depth);
            for &child in node.children().iter().rev() {
                stack.push((child, depth + 1));
            }
        }
    }

    pub fn file_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| !matches!(e, RpfEntry::Directory(_)))
            .count()
    }

    pub fn dir_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| matches!(e, RpfEntry::Directory(_)))
            .count()
    }

    pub fn entry_path(&self, entry_index: u32) -> Option<String> {
        let node = self.node_for_entry(entry_index)?;
        Some(node.path().to_string())
    }

    // The basename, which NG key selection hashes: the full path would produce
    // a different `JenkHash` and pick the wrong NG sub-key.
    pub fn entry_name(&self, entry_index: u32) -> Option<String> {
        let node = self.node_for_entry(entry_index)?;
        Some(node.name().to_string())
    }

    /// The archive's size in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The file on disk this archive's own bytes are read from; `None` when they are held in memory or inside another archive.
    pub fn path(&self) -> Option<&Path> {
        match &self.source {
            RpfSource::File(path) => Some(path),
            _ => None,
        }
    }

    /// The archive this one is stored inside; `None` when it is not nested.
    pub fn parent(&self) -> Option<&Arc<RpfArchive>> {
        match &self.source {
            RpfSource::Nested(parent) => Some(parent),
            _ => None,
        }
    }

    /// What the tables of contents hold in memory: the entries, the names they point at and the
    /// tree built over them, each at its capacity; an archive read into memory keeps its bytes besides.
    pub fn table_bytes(&self) -> usize {
        use std::mem::size_of;
        let names: usize = self.names.iter().map(String::capacity).sum();
        let tree: usize = self
            .nodes
            .iter()
            .map(|node| match node {
                RpfNode::Directory {
                    name,
                    path,
                    children,
                    ..
                } => name.capacity() + path.capacity() + children.capacity() * size_of::<u32>(),
                RpfNode::BinaryFile { name, path, .. }
                | RpfNode::ResourceFile { name, path, .. } => name.capacity() + path.capacity(),
            })
            .sum();
        let file = match &self.source {
            RpfSource::File(path) => path.capacity(),
            _ => 0,
        };
        size_of::<Self>()
            + self.entries.capacity() * size_of::<RpfEntry>()
            + self.names.capacity() * size_of::<String>()
            + names
            + self.name_heap.capacity()
            + self.nodes.capacity() * size_of::<RpfNode>()
            + tree
            + self.entry_to_node.capacity() * size_of::<u32>()
            + file
    }

    /// The keys this archive was opened with.
    pub fn keys(&self) -> &Arc<dyn RpfCryptoProvider> {
        &self.keys
    }
}
