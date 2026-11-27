//! Writing an archive of any [`RpfVersion`]: the files by path, then one save.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{BufWriter, Cursor, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use flate2::write::DeflateEncoder;
use flate2::Compression;
use tracing::debug;

use crate::container::resource::{dat_compress_best, ResourceEnvelope, Rsc7};
use crate::error::{IoError, Result};
use crate::hash::string_hash;

use super::archive::RpfArchive;
use super::crypto::{CryptoTarget, NoOpCryptoProvider, RpfCryptoProvider};
use super::entry::{
    hide_big_resource_size, Compressor, RpfBinaryEntry, RpfDirectoryEntry, RpfEntry,
    RpfResourceEntry, RPF_ENTRY_SIZE,
};
use super::header::{
    RpfEncryption, RpfHeader, RPF_FILE_ALIGNMENT, RPF_FILE_ALIGNMENT_SHIFT,
    RPF_MAX_RESOURCE_CONSUMED_SIZE, RPF_NAME_OFFSET_MAX_SHIFT, RPF_TRAILING_PAD,
};
use super::layout::{self, Plan, RPF8_CLEAR};
use super::reader::{build_names, BodyForm};
use super::version::RpfVersion;

/// The longest name a path element may have.
const MAX_NAME_BYTES: usize = 255;

/// A `Write + Seek` that keeps the position and the high-water mark and
/// nothing else.
///
/// ⚠ **The high-water mark, not the final position.** The builder seeks back to
/// patch the header and the entry table after writing the bodies, so the
/// position when it finishes is near the start of the file. A counter that
/// reported `position` would report a few kilobytes for a two-gigabyte archive.
#[derive(Default)]
struct CountingWriter {
    position: u64,
    high_water: u64,
}

impl Write for CountingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.position += buf.len() as u64;
        self.high_water = self.high_water.max(self.position);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Seek for CountingWriter {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let next = match to {
            SeekFrom::Start(n) => n as i128,
            SeekFrom::Current(n) => self.position as i128 + n as i128,
            SeekFrom::End(n) => self.high_water as i128 + n as i128,
        };
        if next < 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "seek before the start of the file",
            ));
        }
        self.position = next as u64;
        // ⚠ A seek past the end does **not** extend a real file, so it must not
        // extend this one either — only a write does. Counting it here would
        // make the preview larger than the save for any builder that seeks to
        // the end before writing.
        Ok(self.position)
    }
}

/// What the archive holds at one path.
enum Item {
    Directory,
    File(Body),
}

struct Body {
    source: Source,
    compress: bool,
    /// The row to write for a body carried as it is stored; see [`RpfBuilder::insert_stored`].
    stored: Option<RpfEntry>,
}

enum Source {
    Memory(Vec<u8>),
    Disk(PathBuf),
    /// An entry of an archive being edited, read when the body is written.
    Carried(Arc<RpfArchive>, u32),
}

impl Body {
    /// The body's bytes, borrowed when they are already in memory.
    fn bytes(&self) -> Result<Cow<'_, [u8]>> {
        match &self.source {
            Source::Memory(b) => Ok(Cow::Borrowed(b)),
            Source::Disk(p) => fs::read(p)
                .map(Cow::Owned)
                .map_err(|e| IoError::FileReadFailed {
                    path: p.clone(),
                    source: e,
                }),
            Source::Carried(from, entry) => {
                let form = if self.stored.is_some() {
                    BodyForm::AsStored
                } else {
                    BodyForm::Decoded
                };
                from.read_form(*entry, form).map(Cow::Owned)
            }
        }
    }
}

/// Builds an archive of any [`RpfVersion`] from paths and bytes, and saves it.
///
/// A path is matched without regard to case or to which slash is used, as the game does, and is
/// kept in lower case. Adding to a path that holds a file replaces the file; the table is laid out
/// from the paths alone, so the same files give the same archive in whatever order they were added.
pub struct RpfBuilder {
    version: RpfVersion,
    name_shift: u8,
    encryption: RpfEncryption,
    keys: Arc<dyn RpfCryptoProvider>,
    items: BTreeMap<String, Item>,
}

impl Default for RpfBuilder {
    /// An RPF7 builder.
    fn default() -> Self {
        Self::new(RpfVersion::Rpf7)
    }
}

impl RpfBuilder {
    /// A builder for `version`, which writes the table that version has and no other, in the clear.
    pub fn new(version: RpfVersion) -> Self {
        Self {
            version,
            name_shift: 0,
            encryption: RpfEncryption::Open,
            keys: Arc::new(NoOpCryptoProvider::new()),
            items: BTreeMap::new(),
        }
    }

    /// An RPF7 whose name heap is aligned to `1 << shift`, which lets it hold `64 KiB << shift` of names.
    pub fn name_shift(mut self, shift: u8) -> Self {
        self.name_shift = shift.min(RPF_NAME_OFFSET_MAX_SHIFT);
        self
    }

    /// Write with `encryption`, which `keys` must be able to apply.
    pub fn encrypted(
        mut self,
        encryption: RpfEncryption,
        keys: Arc<dyn RpfCryptoProvider>,
    ) -> Self {
        self.encryption = encryption;
        self.keys = keys;
        self
    }

    /// The builder behind [`RpfArchive::edit`].
    pub(super) fn editing(archive: &RpfArchive) -> Result<Self> {
        // ⚠ RDR2's cipher only deciphers, so an RPF8 is written with a clear table: each carried body
        // keeps the key its own row names, and what is added or changed is stored clear.
        let encryption = if archive.version == RpfVersion::Rpf8 {
            RpfEncryption::None
        } else {
            archive.header.encryption
        };
        let mut builder = Self::new(archive.version)
            .name_shift(archive.header.name_shift)
            .encrypted(encryption, Arc::clone(&archive.keys));
        let from = Arc::new(archive.clone());
        for node in archive.nodes.iter().skip(1) {
            let entry = &archive.entries[node.entry_index() as usize];
            let item = match entry {
                RpfEntry::Directory(_) => Item::Directory,
                // Only RPF7 can be written from the file a body decodes to: an RPF8 body may be Oodle,
                // which cannot be decoded, and an RPF2 to RPF6 resource cannot be written from its file.
                _ => Item::File(Body {
                    source: Source::Carried(Arc::clone(&from), node.entry_index()),
                    compress: matches!(entry, RpfEntry::Binary(b) if b.is_compressed()),
                    stored: (archive.version != RpfVersion::Rpf7).then(|| entry.clone()),
                }),
            };
            builder.put(node.path(), item)?;
        }
        Ok(builder)
    }

    pub fn version(&self) -> RpfVersion {
        self.version
    }

    /// `bytes` at `path`, deflated when the file's extension is one the game deflates.
    pub fn insert(&mut self, path: &str, bytes: impl Into<Vec<u8>>) -> Result<&mut Self> {
        self.insert_with(path, bytes, should_compress_extension(path))
    }

    /// `bytes` at `path`, deflated when `compress` says so and it makes them smaller.
    pub fn insert_with(
        &mut self,
        path: &str,
        bytes: impl Into<Vec<u8>>,
        compress: bool,
    ) -> Result<&mut Self> {
        self.file(path, Source::Memory(bytes.into()), compress, None)
    }

    /// The file at `source` at `path`, read when the archive is written, and deflated by the rule of [`insert`](Self::insert).
    pub fn insert_file(&mut self, path: &str, source: impl AsRef<Path>) -> Result<&mut Self> {
        let source = Source::Disk(source.as_ref().to_path_buf());
        self.file(path, source, should_compress_extension(path), None)
    }

    /// `body` at `path` exactly as `entry` stores it: nothing is decoded, deciphered or recompressed.
    ///
    /// This is how an archive is rewritten around bodies nothing here can decode, such as RPF8's
    /// Oodle, and the one way an RPF2 to RPF6 resource is written. `body` is
    /// [`BodyForm::AsStored`] of `entry`, which must be of this builder's version.
    pub fn insert_stored(
        &mut self,
        path: &str,
        entry: RpfEntry,
        body: Vec<u8>,
    ) -> Result<&mut Self> {
        if matches!(entry, RpfEntry::Directory(_)) {
            return Err(IoError::RpfBadEntry {
                message: "a directory has no body to carry".into(),
            });
        }
        self.file(path, Source::Memory(body), false, Some(entry))
    }

    /// An empty directory at `path`, which a file added under it fills.
    pub fn mkdir(&mut self, path: &str) -> Result<&mut Self> {
        self.put(path, Item::Directory)?;
        Ok(self)
    }

    /// Drop `path`, and everything under it when it is a directory; how many files and directories went.
    pub fn remove(&mut self, path: &str) -> usize {
        let Ok(path) = normalize(path) else { return 0 };
        let below = format!("{path}/");
        let before = self.items.len();
        self.items
            .retain(|at, _| *at != path && !at.starts_with(&below));
        before - self.items.len()
    }

    /// Move `from`, and everything under it when it is a directory, to `to`; how many files and directories moved.
    ///
    /// When it fails the builder is as it was.
    pub fn rename(&mut self, from: &str, to: &str) -> Result<usize> {
        let (from, to) = (normalize(from)?, normalize(to)?);
        let below = format!("{from}/");
        let moving: Vec<String> = self
            .items
            .keys()
            .filter(|at| **at == from || at.starts_with(&below))
            .cloned()
            .collect();
        let taken: Vec<(String, Item)> = moving
            .into_iter()
            .filter_map(|at| self.items.remove(&at).map(|item| (at, item)))
            .collect();
        let renamed = |at: &str| format!("{to}{}", &at[from.len()..]);
        if let Some(error) = taken
            .iter()
            .find_map(|(at, item)| self.claim(&renamed(at), item).err())
        {
            self.items.extend(taken);
            return Err(error);
        }
        let moved = taken.len();
        self.items
            .extend(taken.into_iter().map(|(at, item)| (renamed(&at), item)));
        Ok(moved)
    }

    fn file(
        &mut self,
        path: &str,
        source: Source,
        compress: bool,
        stored: Option<RpfEntry>,
    ) -> Result<&mut Self> {
        let body = Body {
            source,
            compress,
            stored,
        };
        self.put(path, Item::File(body))?;
        Ok(self)
    }

    fn put(&mut self, path: &str, item: Item) -> Result<()> {
        let path = normalize(path)?;
        self.claim(&path, &item)?;
        self.items.insert(path, item);
        Ok(())
    }

    /// Whether `item` can stand at the normalised `path`: nothing is under a file, and nothing is a file and a directory at once.
    fn claim(&self, path: &str, item: &Item) -> Result<()> {
        let refuse = |what: String| IoError::RpfBadEntry { message: what };
        for (at, _) in path.match_indices('/') {
            if matches!(self.items.get(&path[..at]), Some(Item::File(_))) {
                return Err(refuse(format!(
                    "`{}` is a file, so nothing can be added under it",
                    &path[..at]
                )));
            }
        }
        match (item, self.items.get(path)) {
            (Item::File(_), Some(Item::Directory)) => {
                return Err(refuse(format!("`{path}` is a directory")))
            }
            (Item::Directory, Some(Item::File(_))) => {
                return Err(refuse(format!("`{path}` is a file")))
            }
            _ => {}
        }
        let below = format!("{path}/");
        let under = self
            .items
            .range(below.clone()..)
            .next()
            .is_some_and(|(at, _)| at.starts_with(&below));
        match item {
            Item::File(_) if under => Err(refuse(format!("`{path}` is a directory"))),
            _ => Ok(()),
        }
    }

    /// The tree of names every table is written from.
    fn tree(&self) -> Result<Tree<'_>> {
        match self.version {
            RpfVersion::Rpf7 => Tree::of(
                &self.items,
                self.name_shift,
                (64 * 1024usize) << self.name_shift,
            ),
            _ => Tree::of(&self.items, 0, u32::MAX as usize),
        }
    }

    /// Exactly how many bytes [`write_to`](Self::write_to) will write, found by running
    /// the save into a sink that counts, so the alignment, the trailing pad and
    /// the per-entry compression decision cannot drift from a second
    /// calculation (`the_preview_is_the_size_the_save_writes`). It compresses
    /// every body to find out.
    pub fn planned_size(&self) -> Result<u64> {
        self.planned_size_with(|_, _| {})
    }

    /// [`planned_size`](Self::planned_size), calling `on_file` with each file's ordinal and path before reading it.
    pub fn planned_size_with(&self, mut on_file: impl FnMut(usize, &str)) -> Result<u64> {
        let mut counter = CountingWriter::default();
        self.write(&mut counter, &mut on_file)?;
        Ok(counter.high_water)
    }

    /// Write the archive to `path`, which is replaced only once all of it is written.
    ///
    /// An IMG1 is two files, the image at `path` and its directory beside it with the extension `.dir`.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if self.version == RpfVersion::Img1 {
            return replace(path, |img| {
                replace(&path.with_extension("dir"), |dir| self.write_img1(dir, img))
            });
        }
        replace(path, |out| self.write_to(out))
    }

    /// The archive as bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut out = Cursor::new(Vec::new());
        self.write_to(&mut out)?;
        Ok(out.into_inner())
    }

    /// The archive as it would be saved, opened in memory.
    pub fn build(&self) -> Result<RpfArchive> {
        if self.version == RpfVersion::Img1 {
            let (mut directory, mut image) = (Vec::new(), Cursor::new(Vec::new()));
            self.write_img1(&mut directory, &mut image)?;
            return RpfArchive::from_img1(directory, image.into_inner());
        }

        RpfArchive::from_bytes_with(None, self.to_bytes()?, Arc::clone(&self.keys))
    }

    pub fn write_to<W: Write + Seek>(&self, writer: &mut W) -> Result<()> {
        self.write(writer, &mut |_, _| {})
    }

    fn write<W: Write + Seek>(
        &self,
        writer: &mut W,
        on_file: &mut dyn FnMut(usize, &str),
    ) -> Result<()> {
        if !self.keys.supports(self.encryption) {
            return Err(IoError::RpfEncryptionUnsupported {
                key_id: self.encryption.to_raw(),
            });
        }
        let tree = self.tree()?;
        match self.version {
            RpfVersion::Rpf7 => self.write_rpf7(writer, &tree, on_file),
            RpfVersion::Img1 => Err(IoError::Invalid {
                what: "saving an IMG1".into(),
                why: "it is two files, the `.dir` and the `.img`; use `save` or `write_img1`"
                    .into(),
            }),
            version => {
                let laid = self.lay_bodies(writer, &tree, on_file)?;
                let plan = Plan {
                    entries: &laid.entries,
                    names: &laid.names,
                    name_heap: &tree.heap,
                    body_end: laid.body_end,
                    encryption: self.encryption,
                };
                layout::write(version, writer, &plan, self.keys.as_ref())?;
                writer.flush()?;
                Ok(())
            }
        }
    }

    /// An IMG1: the `.img` of bodies to `img`, and the `.dir` that says where they are to `dir`.
    pub fn write_img1<D: Write, I: Write + Seek>(&self, dir: &mut D, img: &mut I) -> Result<()> {
        if self.version != RpfVersion::Img1 {
            return Err(IoError::Invalid {
                what: "saving an IMG1".into(),
                why: format!("this builder is for {}", self.version),
            });
        }
        let tree = self.tree()?;
        let laid = self.lay_bodies(img, &tree, &mut |_, _| {})?;
        let plan = Plan {
            entries: &laid.entries,
            names: &laid.names,
            name_heap: &tree.heap,
            body_end: laid.body_end,
            encryption: RpfEncryption::None,
        };
        layout::write_img1(dir, &plan)?;
        img.flush()?;
        Ok(())
    }

    fn write_rpf7<W: Write + Seek>(
        &self,
        writer: &mut W,
        tree: &Tree,
        on_file: &mut dyn FnMut(usize, &str),
    ) -> Result<()> {
        let (encryption, crypto) = (self.encryption, self.keys.as_ref());
        let mut name_heap = tree.heap.clone();
        let mut rng = 0u64;
        if name_heap.len() & 0xF != 0 {
            let pad = 16 - (name_heap.len() & 0xF);
            for _ in 0..pad {
                name_heap.push(next_random(&mut rng));
            }
        }

        let (mut entries, original_order) = build_tree(tree, self.name_shift, false);

        let header_placeholder = RpfHeader {
            entry_count: entries.len() as u32,
            name_heap_size: name_heap.len() as u32,
            name_shift: self.name_shift,
            xcompress: false,
            encryption,
            platform: None,
        };
        header_placeholder.write(writer)?;
        for e in &entries {
            e.write(writer)?;
        }
        writer.write_all(&name_heap)?;

        write_padding(writer, RPF_FILE_ALIGNMENT, &mut rng)?;

        for (n, (entry_idx, node_idx)) in original_order.iter().enumerate() {
            let entry_idx = *entry_idx;
            let (path, file) = tree.nodes[*node_idx as usize]
                .file
                .expect("non-file in original_order");
            on_file(n, path);

            let body_offset = writer.stream_position()?;
            if body_offset > u32::MAX as u64 {
                return Err(IoError::RpfBadEntry {
                    message: format!("body offset {:#x} exceeds 4 GiB", body_offset),
                });
            }
            if body_offset & ((1u64 << RPF_FILE_ALIGNMENT_SHIFT) - 1) != 0 {
                return Err(IoError::RpfBadEntry {
                    message: format!("body offset {:#x} not 512-aligned", body_offset),
                });
            }

            if let Some(stored) = &file.stored {
                let mut kept = placed(
                    stored,
                    entries[entry_idx as usize].name_offset(),
                    body_offset,
                );
                if let RpfEntry::Resource(r) = &mut kept {
                    r.consumed_size = r.consumed_size.min(RPF_MAX_RESOURCE_CONSUMED_SIZE);
                }
                writer.write_all(&file.bytes()?)?;
                entries[entry_idx as usize] = kept;
                write_padding(writer, RPF_FILE_ALIGNMENT, &mut rng)?;
                continue;
            }

            let mut buffer = file.bytes()?.into_owned();
            let original_size = buffer.len();
            if original_size > u32::MAX as usize {
                return Err(IoError::DataTooLarge {
                    size: original_size,
                    max: u32::MAX as usize,
                });
            }
            let original_size_u32 = original_size as u32;

            let is_resource = buffer.len() >= Rsc7::HEADER_LEN && Rsc7::detect(&buffer);

            let entry_basename = tree.name(*node_idx);

            if is_resource {
                // The flag words move into the entry, and the header slot is
                // filled with random bytes.
                let system_info = u32::from_le_bytes(buffer[8..12].try_into().unwrap());
                let graphics_info = u32::from_le_bytes(buffer[12..16].try_into().unwrap());

                let big = original_size_u32 >= RPF_MAX_RESOURCE_CONSUMED_SIZE;
                let consumed_size = if big {
                    RPF_MAX_RESOURCE_CONSUMED_SIZE
                } else {
                    original_size_u32
                };

                for byte in &mut buffer[..Rsc7::HEADER_LEN] {
                    *byte = next_random(&mut rng);
                }
                if big {
                    let slot: &mut [u8; Rsc7::HEADER_LEN] = (&mut buffer[..Rsc7::HEADER_LEN])
                        .try_into()
                        .expect("a resource body carries a 16-byte RSC7 header");
                    hide_big_resource_size(slot, original_size_u32);
                }

                if encryption.needs_crypto()
                    && entry_basename.to_ascii_lowercase().ends_with(".ysc")
                {
                    crypto.encrypt(
                        encryption,
                        CryptoTarget::Body {
                            entry_name: &entry_basename,
                            keyed_size: consumed_size,
                            chunk: 0,
                        },
                        &mut buffer,
                    )?;
                }

                writer.write_all(&buffer)?;

                let entry = match &mut entries[entry_idx as usize] {
                    RpfEntry::Binary(b) => RpfEntry::Resource(RpfResourceEntry {
                        name_offset: b.name_offset,
                        consumed_size,
                        file_offset: body_offset,
                        system_info,
                        graphics_info,
                        compressor: Compressor::None,
                        tag: 0,
                    }),
                    other => {
                        return Err(IoError::RpfBadEntry {
                            message: format!(
                                "expected placeholder Binary entry at index {}, got {:?}",
                                entry_idx,
                                other.kind()
                            ),
                        })
                    }
                };
                entries[entry_idx as usize] = entry;
            } else {
                let mut consumed_size = 0u32;
                let mut output_buffer = buffer;
                let mut encrypted_after_compress = false;

                if file.compress {
                    let mut encoder =
                        DeflateEncoder::new(Vec::with_capacity(original_size), Compression::best());
                    encoder
                        .write_all(&output_buffer)
                        .map_err(|e| IoError::CompressionFailed(e.to_string()))?;
                    let compressed = encoder
                        .finish()
                        .map_err(|e| IoError::CompressionFailed(e.to_string()))?;
                    if compressed.len() < output_buffer.len() {
                        consumed_size = compressed.len() as u32;
                        output_buffer = compressed;
                        if encryption.needs_crypto() {
                            crypto.encrypt(
                                encryption,
                                CryptoTarget::Body {
                                    entry_name: &entry_basename,
                                    keyed_size: original_size_u32,
                                    chunk: 0,
                                },
                                &mut output_buffer,
                            )?;
                            encrypted_after_compress = true;
                        }
                    }
                }

                writer.write_all(&output_buffer)?;

                let new_entry = match &mut entries[entry_idx as usize] {
                    RpfEntry::Binary(b) => RpfEntry::Binary(RpfBinaryEntry {
                        name_offset: b.name_offset,
                        consumed_size,
                        file_offset: body_offset,
                        uncompressed_size: original_size_u32,
                        encrypted: encrypted_after_compress,
                        encrypted_raw: if encrypted_after_compress { 1 } else { 0 },
                        compressor: if consumed_size != 0 {
                            Compressor::Deflate
                        } else {
                            Compressor::None
                        },
                        tag: 0,
                    }),
                    other => {
                        return Err(IoError::RpfBadEntry {
                            message: format!(
                                "expected placeholder Binary entry at index {}, got {:?}",
                                entry_idx,
                                other.kind()
                            ),
                        })
                    }
                };
                entries[entry_idx as usize] = new_entry;
            }

            write_padding(writer, RPF_FILE_ALIGNMENT, &mut rng)?;
        }

        write_padding(writer, RPF_TRAILING_PAD, &mut rng)?;

        let mut entry_bytes = Vec::with_capacity(entries.len() * RPF_ENTRY_SIZE);
        for e in &entries {
            e.write(&mut entry_bytes)?;
        }
        let mut name_bytes = name_heap.clone();

        let archive_size = writer.stream_position()? as u32;
        if encryption.needs_crypto() {
            let target = CryptoTarget::Toc {
                archive_name: None,
                archive_size,
            };
            crypto.encrypt(encryption, target.clone(), &mut entry_bytes)?;
            crypto.encrypt(encryption, target, &mut name_bytes)?;
        }

        writer.seek(SeekFrom::Start(0))?;
        let final_header = RpfHeader {
            entry_count: entries.len() as u32,
            name_heap_size: name_heap.len() as u32,
            name_shift: self.name_shift,
            xcompress: false,
            encryption,
            platform: None,
        };
        final_header.write(writer)?;
        writer.write_all(&entry_bytes)?;
        writer.write_all(&name_bytes)?;
        writer.flush()?;

        debug!(
            entry_count = entries.len(),
            name_heap_size = name_heap.len(),
            archive_size,
            "RPF7 saved"
        );

        Ok(())
    }
}

/// Run `write` on a file beside `path`, and rename it over `path` once it has succeeded.
///
/// So a save can read the archive it replaces, and a save that fails leaves it as it was.
fn replace<T>(path: &Path, write: impl FnOnce(&mut BufWriter<fs::File>) -> Result<T>) -> Result<T> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    let written = fs::File::create(&temp)
        .map_err(IoError::from)
        .and_then(|file| {
            let mut out = BufWriter::new(file);
            let done = write(&mut out)?;
            out.flush()?;
            Ok(done)
        });
    match written {
        Ok(done) => {
            fs::rename(&temp, path)?;
            Ok(done)
        }
        Err(e) => {
            let _ = fs::remove_file(&temp);
            Err(e)
        }
    }
}

/// Where the bodies of every version but RPF7 landed.
struct Laid {
    entries: Vec<RpfEntry>,
    names: Vec<String>,
    body_end: u64,
}

impl RpfBuilder {
    /// The entries to write, each one's name, and the node every file's entry stands for.
    fn plan(&self, tree: &Tree) -> Result<(Vec<RpfEntry>, Vec<String>, Order)> {
        if self.version.is_flat() {
            return Ok(build_flat(tree, self.version));
        }
        let (entries, order) = build_tree(tree, 0, self.version.hashes_names());
        let names = build_names(&entries, &tree.heap, 0)?;
        Ok((entries, names, order))
    }

    /// Write every body, each at the version's alignment past the room its tables will take.
    fn lay_bodies<W: Write + Seek>(
        &self,
        writer: &mut W,
        tree: &Tree,
        on_file: &mut dyn FnMut(usize, &str),
    ) -> Result<Laid> {
        let version = self.version;
        let (mut entries, names, order) = self.plan(tree)?;
        // The position is counted here and never asked of the writer, which for a file is a
        // system call per body.
        let mut at = layout::prologue(version, &names, tree.heap.len())
            .next_multiple_of(version.alignment());
        writer.seek(SeekFrom::Start(at))?;
        for (n, (entry_idx, node_idx)) in order.into_iter().enumerate() {
            let (path, file) =
                tree.nodes[node_idx as usize]
                    .file
                    .ok_or_else(|| IoError::RpfBadEntry {
                        message: format!("tree node {node_idx} is not a file"),
                    })?;
            on_file(n, path);
            let name_offset = entries[entry_idx as usize].name_offset();
            let (entry, len) = self.place_body(file, name_offset, at, writer)?;
            entries[entry_idx as usize] = entry;
            let end = (at + len as u64).next_multiple_of(version.alignment());
            layout::zeros(writer, end - at - len as u64)?;
            at = end;
        }
        Ok(Laid {
            entries,
            names,
            body_end: at,
        })
    }

    /// Write one body at `at` and return the entry that says where it is, and how long the body is.
    fn place_body<W: Write>(
        &self,
        file: &Body,
        name_offset: u32,
        at: u64,
        writer: &mut W,
    ) -> Result<(RpfEntry, usize)> {
        let version = self.version;
        let bytes = file.bytes()?;
        if let Some(stored) = &file.stored {
            writer.write_all(&bytes)?;
            return Ok((placed(stored, name_offset, at), bytes.len()));
        }
        let size = u32::try_from(bytes.len()).map_err(|_| IoError::DataTooLarge {
            size: bytes.len(),
            max: u32::MAX as usize,
        })?;

        let is_resource = version
            .resource_magic()
            .is_some_and(|magic| bytes.starts_with(&magic.to_le_bytes()));
        if is_resource {
            let row = match version {
                RpfVersion::Rpf8 => layout::rpf8_resource_from_file(&bytes),
                _ => None,
            };
            let Some(mut row) = row else {
                return Err(IoError::Invalid {
                    what: format!("writing a {version} resource"),
                    why: "its header is not one this crate can write from a file; carry it \
                          as stored with `insert_stored`"
                        .into(),
                });
            };
            writer.write_all(&bytes)?;
            (row.name_offset, row.file_offset) = (name_offset, at);
            return Ok((RpfEntry::Resource(row), bytes.len()));
        }

        // RPF8's own compressor is Oodle, which the game reads by the type its stream names.
        let (packed, how) = match version {
            RpfVersion::Rpf8 if file.compress => (
                Some(super::oodle::compress(&bytes)).filter(|d| d.len() < bytes.len()),
                Compressor::Oodle,
            ),
            v if file.compress && v.deflates() => (
                Some(dat_compress_best(&bytes)?).filter(|d| d.len() < bytes.len()),
                Compressor::Deflate,
            ),
            _ => (None, Compressor::None),
        };
        let body: &[u8] = packed.as_deref().unwrap_or(&bytes);
        writer.write_all(body)?;
        let compressed = packed.is_some();
        let entry = RpfEntry::Binary(RpfBinaryEntry {
            name_offset,
            consumed_size: if compressed { body.len() as u32 } else { 0 },
            file_offset: at,
            uncompressed_size: size,
            encrypted: false,
            encrypted_raw: 0,
            compressor: if compressed { how } else { Compressor::None },
            tag: clear_tag(version),
        });
        Ok((entry, body.len()))
    }
}

/// `entry` where it now lies: at `file_offset`, under the name the builder gave it.
fn placed(entry: &RpfEntry, name_offset: u32, file_offset: u64) -> RpfEntry {
    let mut entry = entry.clone();
    entry.set_name_offset(name_offset);
    match &mut entry {
        RpfEntry::Binary(b) => b.file_offset = file_offset,
        RpfEntry::Resource(r) => r.file_offset = file_offset,
        RpfEntry::Directory(_) => {}
    }
    entry
}

/// The row bits of a body in the clear: RPF8's key id says so, and no other version has bits to say it.
fn clear_tag(version: RpfVersion) -> u32 {
    if version == RpfVersion::Rpf8 {
        RPF8_CLEAR
    } else {
        0
    }
}

/// `path` as the archive keeps it: no empty element, forward slashes, lower case.
pub(super) fn normalize(path: &str) -> Result<String> {
    let mut out = String::with_capacity(path.len());
    for name in path.split(['/', '\\']).filter(|name| !name.is_empty()) {
        if name.len() > MAX_NAME_BYTES {
            return Err(IoError::RpfBadEntry {
                message: format!("RPF path element too long ({} bytes): {name:?}", name.len()),
            });
        }
        if !out.is_empty() {
            out.push('/');
        }
        out.extend(name.chars().map(|c| c.to_ascii_lowercase()));
    }
    if out.is_empty() {
        return Err(IoError::RpfBadEntry {
            message: format!("{path:?} names no file or directory"),
        });
    }
    Ok(out)
}

/// The files as a tree of names, which every table is written from.
struct Tree<'a> {
    /// Node 0 is the root.
    nodes: Vec<Node<'a>>,
    /// Every distinct name once, NUL-ended and padded to the name shift; a node's name is an offset into it.
    heap: Vec<u8>,
    interned: HashMap<&'a str, u32>,
    shift: u8,
    heap_max: usize,
}

struct Node<'a> {
    name: u32,
    /// The path and body of a file.
    file: Option<(&'a str, &'a Body)>,
    children: Vec<u32>,
}

/// A file's entry in a table, and the node it was written from.
type Order = Vec<(u32, u32)>;

impl<'a> Tree<'a> {
    fn of(items: &'a BTreeMap<String, Item>, shift: u8, heap_max: usize) -> Result<Self> {
        let root = Node {
            name: 0,
            file: None,
            children: Vec::new(),
        };
        let mut tree = Self {
            nodes: vec![root],
            heap: Vec::new(),
            interned: HashMap::new(),
            shift,
            heap_max,
        };
        tree.intern("")?;
        let mut child_of: HashMap<(u32, u32), u32> = HashMap::new();
        for (path, item) in items {
            let mut at = 0u32;
            for part in path.split('/') {
                let name = tree.intern(part)?;
                at = match child_of.get(&(at, name)) {
                    Some(&node) => node,
                    None => {
                        let node = tree.nodes.len() as u32;
                        tree.nodes.push(Node {
                            name,
                            file: None,
                            children: Vec::new(),
                        });
                        tree.nodes[at as usize].children.push(node);
                        child_of.insert((at, name), node);
                        node
                    }
                };
            }
            if let Item::File(body) = item {
                tree.nodes[at as usize].file = Some((path, body));
            }
        }
        Ok(tree)
    }

    /// Where `name` is in the heap, adding it if it is not.
    fn intern(&mut self, name: &'a str) -> Result<u32> {
        if let Some(&at) = self.interned.get(name) {
            return Ok(at);
        }
        let mask = (1usize << self.shift) - 1;
        let size = (name.len() + 1 + mask) & !mask;
        let at = self.heap.len();
        if at + size > self.heap_max {
            return Err(IoError::RpfNameHeapOverflow {
                size: at + size,
                max: self.heap_max,
            });
        }
        self.heap.extend_from_slice(name.as_bytes());
        self.heap.resize(at + size, 0);
        self.interned.insert(name, at as u32);
        Ok(at as u32)
    }

    fn name_bytes(&self, node: u32) -> &[u8] {
        read_cstr(&self.heap, self.nodes[node as usize].name as usize)
    }

    fn name(&self, node: u32) -> String {
        String::from_utf8_lossy(self.name_bytes(node)).into_owned()
    }
}

/// A table with no directories: a root over every file, each named by its path, sorted as the version sorts.
fn build_flat(tree: &Tree, version: RpfVersion) -> (Vec<RpfEntry>, Vec<String>, Order) {
    // The nodes of files are in path order, which is the order every flat table but RPF8's sorts by.
    let mut files: Vec<(&str, u32)> = tree
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(node, n)| Some((n.file?.0, node as u32)))
        .collect();
    if version == RpfVersion::Rpf8 {
        files.sort_by_cached_key(|(path, _)| (layout::rpf8_hash(path), *path));
    }

    let mut entries = vec![layout::root_over(files.len())];
    let mut names = vec![String::new()];
    let mut order = Order::with_capacity(files.len());
    for (i, (path, node)) in files.into_iter().enumerate() {
        entries.push(RpfEntry::Binary(RpfBinaryEntry {
            name_offset: 0,
            consumed_size: 0,
            file_offset: 0,
            uncompressed_size: 0,
            encrypted: false,
            encrypted_raw: 0,
            compressor: Compressor::None,
            tag: clear_tag(version),
        }));
        // The image formats name a file, not a path.
        names.push(match version {
            RpfVersion::Rpf8 => path.to_owned(),
            _ => path.rsplit('/').next().unwrap_or(path).to_owned(),
        });
        order.push((i as u32 + 1, node));
    }
    (entries, names, order)
}

/// The table of a tree of directories: each directory's children are consecutive rows, sorted by name or by its hash.
fn build_tree(tree: &Tree, name_shift: u8, by_hash: bool) -> (Vec<RpfEntry>, Order) {
    let mut entries = vec![RpfEntry::Directory(RpfDirectoryEntry {
        name_offset: 0,
        first_entry_index: 1,
        entry_count: 0,
    })];
    let mut order = Order::new();
    let count = lay_children(tree, 0, name_shift, by_hash, &mut entries, &mut order);
    if let RpfEntry::Directory(root) = &mut entries[0] {
        root.entry_count = count;
    }
    (entries, order)
}

/// Add the rows of `parent`'s children, then of theirs, and say how many rows are `parent`'s own.
fn lay_children(
    tree: &Tree,
    parent: u32,
    name_shift: u8,
    by_hash: bool,
    entries: &mut Vec<RpfEntry>,
    order: &mut Order,
) -> u32 {
    let mut children = tree.nodes[parent as usize].children.clone();
    if by_hash {
        children.sort_by_cached_key(|&c| string_hash(tree.name_bytes(c)));
    } else {
        children.sort_by(|&a, &b| tree.name_bytes(a).cmp(tree.name_bytes(b)));
    }

    let first = entries.len();
    for &child in &children {
        let node = &tree.nodes[child as usize];
        let name_offset = node.name >> name_shift;
        entries.push(match node.file {
            None => RpfEntry::Directory(RpfDirectoryEntry {
                name_offset,
                first_entry_index: 0,
                entry_count: 0,
            }),
            Some(_) => RpfEntry::Binary(RpfBinaryEntry {
                name_offset,
                consumed_size: 0,
                file_offset: 0,
                uncompressed_size: 0,
                encrypted: false,
                encrypted_raw: 0,
                compressor: Compressor::None,
                tag: 0,
            }),
        });
    }
    for (i, &child) in children.iter().enumerate() {
        let at = first + i;
        if tree.nodes[child as usize].file.is_some() {
            order.push((at as u32, child));
            continue;
        }
        let first_child = entries.len() as u32;
        let count = lay_children(tree, child, name_shift, by_hash, entries, order);
        if let RpfEntry::Directory(d) = &mut entries[at] {
            d.first_entry_index = first_child;
            d.entry_count = count;
        }
    }
    children.len() as u32
}

fn read_cstr(heap: &[u8], offset: usize) -> &[u8] {
    let nul = heap[offset..]
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(heap.len() - offset);
    &heap[offset..offset + nul]
}

fn should_compress_extension(dest_path: &str) -> bool {
    if let Some(dot) = dest_path.rfind('.') {
        let ext = &dest_path[dot..];
        if ext.eq_ignore_ascii_case(".bik")
            || ext.eq_ignore_ascii_case(".awc")
            || ext.eq_ignore_ascii_case(".gxt")
            || ext.eq_ignore_ascii_case(".rpf")
        {
            return false;
        }
    }
    true
}

fn write_padding<W: Write + Seek>(writer: &mut W, alignment: u32, rng: &mut u64) -> Result<()> {
    let pos = writer.stream_position()?;
    let aligned = (pos + alignment as u64 - 1) & !(alignment as u64 - 1);
    if aligned == pos {
        return Ok(());
    }
    let pad = (aligned - pos) as usize;
    let mut buf = vec![0u8; pad];
    for b in &mut buf {
        *b = next_random(rng);
    }
    writer.write_all(&buf)?;
    Ok(())
}

fn next_random(state: &mut u64) -> u8 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (*state >> 56) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_blacklist_matches_engine() {
        assert!(!should_compress_extension("foo/bar.RPF"));
        assert!(!should_compress_extension("foo/bar.bik"));
        assert!(!should_compress_extension("foo/bar.AWC"));
        assert!(!should_compress_extension("foo/bar.gxt"));
        assert!(should_compress_extension("foo/bar.txt"));
        assert!(should_compress_extension("foo/bar.ydr"));
        assert!(should_compress_extension("noextension"));
    }

    #[test]
    fn a_path_is_the_same_whatever_its_case_or_slashes() {
        assert_eq!(
            normalize("\\Models//Props\\A.YDR").unwrap(),
            "models/props/a.ydr"
        );
        assert!(normalize("//").is_err());
    }

    #[test]
    fn adding_to_a_path_replaces_what_was_there() {
        let mut b = RpfBuilder::default();
        b.insert_with("a/b.txt", b"first".to_vec(), false).unwrap();
        b.insert_with("A\\B.TXT", b"second".to_vec(), false)
            .unwrap();
        let archive = b.build().unwrap();
        assert_eq!(archive.file_count(), 1);
        assert_eq!(archive.read("a/b.txt").unwrap(), b"second");
    }

    #[test]
    fn a_file_and_a_directory_cannot_share_a_path() {
        let mut b = RpfBuilder::default();
        b.insert("a/b", b"x".to_vec()).unwrap();
        assert!(b.insert("a/b/c", b"y".to_vec()).is_err());
        assert!(b.mkdir("a/b").is_err());
        b.insert("d/e", b"x".to_vec()).unwrap();
        assert!(b.insert("d", b"y".to_vec()).is_err());
    }

    #[test]
    fn the_same_files_make_the_same_archive_in_whatever_order() {
        let files = [("b/z.txt", "1"), ("a.txt", "2"), ("b/y.txt", "3")];
        let bytes = |order: &[usize]| {
            let mut b = RpfBuilder::default();
            for &i in order {
                b.insert_with(files[i].0, files[i].1, false).unwrap();
            }
            b.to_bytes().unwrap()
        };
        assert_eq!(bytes(&[0, 1, 2]), bytes(&[2, 1, 0]));
    }

    #[test]
    fn a_rename_moves_a_directory_and_a_refused_one_changes_nothing() {
        let mut b = RpfBuilder::default();
        b.insert("d/x.bin", b"1".to_vec()).unwrap();
        b.insert("d/sub/y.bin", b"2".to_vec()).unwrap();
        b.insert("f", b"3".to_vec()).unwrap();
        assert!(b.rename("d", "f/g").is_err());
        assert_eq!(b.rename("d", "e").unwrap(), 2);
        let archive = b.build().unwrap();
        assert!(archive.find("e/sub/y.bin").is_some());
        assert!(archive.find("d/x.bin").is_none());
        assert_eq!(b.remove("e"), 2);
        assert_eq!(b.remove("e"), 0);
    }
}
