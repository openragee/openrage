use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::sync::{Arc, OnceLock};

use tracing::debug;

use crate::container::resource::dat_decompress_bounded;
use crate::container::resource::{ResourceEnvelope, Rsc7};
use crate::error::{IoError, Result};

use super::archive::{RpfArchive, RpfBackend, RpfNode, RpfSource};
use super::crypto::{CryptoTarget, RpfCryptoProvider};
use super::entry::{
    read_big_resource_size as entry_size_from_slot, Compressor, RpfEntry, RpfEntryKind,
    RpfEntryTag, RpfResourceEntry, RPF_ENTRY_SIZE,
};
use super::header::{RpfEncryption, RpfHeader, RPF_HEADER_SIZE, RPF_MAX_RESOURCE_CONSUMED_SIZE};
use super::layout::{self, Input, Toc};
use super::version::RpfVersion;

const MAX_ENTRY_NAME_BYTES: usize = 256;

/// What RPF8 stores past a compressed binary's stream, which its cipher does not count.
const RPF8_COMPRESSED_SLACK: usize = 256;

/// Which of an entry's three forms to hand back; see [`RpfArchive::read_form`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyForm {
    /// Exactly the bytes the archive holds: still encrypted, still deflated.
    ///
    /// ⚠ For a **resource** this is not a loadable file: its sixteen-byte
    /// header slot is filler and the real page flags are in the entry table.
    AsStored,
    /// The key applied, nothing inflated. Meaningful only for a binary entry.
    Decrypted,
    /// What a caller opening the file wants: inflated, or for a resource, with
    /// its RSC7 header reconstructed.
    Decoded,
}

impl BodyForm {
    pub const ALL: [BodyForm; 3] = [Self::AsStored, Self::Decrypted, Self::Decoded];

    pub fn name(self) -> &'static str {
        match self {
            Self::AsStored => "as-stored",
            Self::Decrypted => "decrypted",
            Self::Decoded => "decoded",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "as-stored" | "raw" => Some(Self::AsStored),
            "decrypted" => Some(Self::Decrypted),
            "decoded" => Some(Self::Decoded),
            _ => None,
        }
    }

    /// Does this form produce a file something else can open?
    pub fn is_loadable_for(self, kind: RpfEntryKind) -> bool {
        match self {
            Self::Decoded => true,
            Self::Decrypted => kind == RpfEntryKind::Resource,
            Self::AsStored => false,
        }
    }
}

impl RpfArchive {
    /// Read the table of the archive in `source`, which is `size` bytes from `start_pos`.
    pub(super) fn parse(
        source: RpfSource,
        start_pos: u64,
        size: u64,
        name: Option<String>,
        keys: Arc<dyn RpfCryptoProvider>,
        img1_directory: Option<Vec<u8>>,
    ) -> Result<Self> {
        let input = Input {
            source: &source,
            start: start_pos,
            size,
            name: name.as_deref(),
            crypto: keys.as_ref(),
        };
        #[cfg(feature = "gta6")]
        let mut rpf9 = None;
        let (version, toc) = match img1_directory {
            Some(directory) => (RpfVersion::Img1, layout::read_img1(&directory)?),
            None => {
                let mut head = [0u8; 4];
                read_at(&source, start_pos, &mut head)?;
                let found = u32::from_le_bytes(head);
                let version = RpfVersion::detect(head).ok_or(IoError::RpfBadMagic { found })?;
                let toc = match version {
                    RpfVersion::Rpf7 => read_rpf7(&input)?,
                    #[cfg(feature = "gta6")]
                    RpfVersion::Rpf9 => {
                        let (toc, state) = super::gta6_rpf::read(&input)?;
                        rpf9 = Some(Arc::new(state));
                        toc
                    }
                    // A build without the `gta6` feature has no reader for an RPF9's enciphered
                    // table; with it, the arm above reads the table once a key is given.
                    #[cfg(not(feature = "gta6"))]
                    RpfVersion::Rpf9 => return Err(IoError::Rpf9TocEncrypted),
                    other => layout::read(other, &input)?,
                };
                (version, toc)
            }
        };
        let (nodes, entry_to_node) = build_tree(&toc.entries, &toc.names)?;

        debug!(
            %version,
            entries = toc.entries.len(),
            files = toc
                .entries
                .iter()
                .filter(|e| !matches!(e, RpfEntry::Directory(_)))
                .count(),
            "RPF parsed"
        );

        Ok(Self {
            version,
            header: toc.header,
            entries: toc.entries,
            nodes,
            toc_outcome: toc.outcome,
            names: toc.names,
            name_heap: toc.name_heap,
            entry_to_node,
            source,
            start_pos,
            size,
            keys,
            paths: OnceLock::new(),
            names_platform: OnceLock::new(),
            backend: {
                #[cfg(feature = "gta6")]
                {
                    rpf9.map_or(RpfBackend::Flat, RpfBackend::Rpf9)
                }
                #[cfg(not(feature = "gta6"))]
                {
                    RpfBackend::Flat
                }
            },
        })
    }
}

fn read_rpf7(input: &Input) -> Result<Toc> {
    let Input {
        source,
        start: start_pos,
        size,
        name: archive_name,
        crypto: crypto_ref,
    } = *input;
    let header = read_header(source, start_pos)?;

    let entries_size = (header.entry_count as usize)
        .checked_mul(RPF_ENTRY_SIZE)
        .ok_or_else(|| IoError::RpfBadHeader {
            message: format!("entry_count {} overflows", header.entry_count),
        })?;
    let toc_total = entries_size + header.name_heap_size as usize;

    let toc_end = (RPF_HEADER_SIZE as u64).saturating_add(toc_total as u64);
    if toc_end > size {
        return Err(IoError::RpfBadHeader {
            message: format!(
                "table of contents needs {} bytes ({} entries + {} byte name heap) \
                     but the archive is only {} bytes",
                toc_total, header.entry_count, header.name_heap_size, size
            ),
        });
    }

    let mut toc_buf = vec![0u8; toc_total];
    read_at(source, start_pos + RPF_HEADER_SIZE as u64, &mut toc_buf)?;

    if !crypto_ref.supports(header.encryption) {
        return Err(IoError::RpfEncryptionUnsupported {
            key_id: header.encryption.to_raw(),
        });
    }
    let archive_size_for_crypto = size.min(u32::MAX as u64) as u32;
    let target = CryptoTarget::Toc {
        archive_name,
        archive_size: archive_size_for_crypto,
    };
    let (entries_buf, names_buf) = toc_buf.split_at_mut(entries_size);
    // A wrong NG key (the archive renamed, so its filename-derived key
    // picks the wrong slot) decrypts entry 0 to bytes that fail this
    // check, and the retail provider then tries every NG key index.
    let entry_count_for_validator = header.entry_count;
    let name_heap_size_for_validator = header.name_heap_size;
    let name_shift_for_validator = header.name_shift;
    let validate_root = move |block: &[u8; 16]| -> bool {
        is_plausible_root_entry(
            block,
            entry_count_for_validator,
            name_heap_size_for_validator,
            name_shift_for_validator,
        )
    };
    let toc_outcome = crypto_ref.decrypt_toc(
        header.encryption,
        target.clone(),
        entries_buf,
        &validate_root,
    )?;
    crypto_ref.decrypt_names(header.encryption, target, names_buf, toc_outcome)?;

    let mut entries: Vec<RpfEntry> = Vec::with_capacity(header.entry_count as usize);
    {
        let mut cursor = Cursor::new(&entries_buf);
        for _ in 0..header.entry_count {
            entries.push(RpfEntry::read(&mut cursor)?);
        }
    }

    let names = build_names(&entries, names_buf, header.name_shift)?;

    let mut big_resource_fixups: Vec<(usize, u32)> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        if let RpfEntry::Resource(r) = entry {
            if r.consumed_size == RPF_MAX_RESOURCE_CONSUMED_SIZE {
                let real = read_big_resource_size(source, start_pos, r)?;
                big_resource_fixups.push((i, real));
            }
        }
    }
    for (i, real) in big_resource_fixups {
        if let RpfEntry::Resource(r_mut) = &mut entries[i] {
            r_mut.consumed_size = real;
        }
    }

    Ok(Toc {
        header,
        entries,
        names,
        name_heap: names_buf.to_vec(),
        outcome: toc_outcome,
    })
}

// Whether a freshly decrypted entry 0 looks like the root directory: the
// predicate the brute-force NG key search asks of each candidate key.
fn is_plausible_root_entry(
    block: &[u8; 16],
    total_entries: u32,
    name_heap_size: u32,
    name_shift: u8,
) -> bool {
    // ⚠ The bitfield is decoded by `entry.rs`, not a local copy: a second
    // decoder that drifts here looks like every NG key being wrong.
    let tag = RpfEntryTag::of(block);
    let lo = u32::from_le_bytes(block[8..12].try_into().expect("4 bytes"));
    let hi = u32::from_le_bytes(block[12..16].try_into().expect("4 bytes"));

    if tag.name_offset != 0 || !tag.is_directory() {
        return false;
    }
    let name_offset = tag.name_offset;
    let first_entry_index = lo;
    let entry_count = hi;
    if first_entry_index < 1 || entry_count == 0 {
        return false;
    }
    let Some(end) = first_entry_index.checked_add(entry_count) else {
        return false;
    };
    if end > total_entries {
        return false;
    }
    let abs_name_offset = (name_offset as u64) << name_shift;
    abs_name_offset <= name_heap_size as u64
}

fn read_header(source: &RpfSource, start_pos: u64) -> Result<RpfHeader> {
    let mut buf = [0u8; RPF_HEADER_SIZE];
    read_at(source, start_pos, &mut buf)?;
    let mut cursor = Cursor::new(&buf[..]);
    RpfHeader::read(&mut cursor)
}

pub(super) fn read_at(source: &RpfSource, abs_offset: u64, buf: &mut [u8]) -> Result<()> {
    match source {
        RpfSource::Memory(bytes) => {
            let start = abs_offset as usize;
            let end = start
                .checked_add(buf.len())
                .ok_or_else(|| IoError::RpfBadEntry {
                    message: format!(
                        "RPF read at {} of {} bytes overflows usize",
                        abs_offset,
                        buf.len()
                    ),
                })?;
            if end > bytes.len() {
                return Err(IoError::UnexpectedEof {
                    offset: abs_offset,
                    needed: buf.len(),
                    available: bytes.len().saturating_sub(start),
                });
            }
            buf.copy_from_slice(&bytes[start..end]);
            Ok(())
        }
        RpfSource::File(path) => {
            let mut f = File::open(path).map_err(|e| IoError::FileReadFailed {
                path: path.clone(),
                source: e,
            })?;
            f.seek(SeekFrom::Start(abs_offset))?;
            f.read_exact(buf)?;
            Ok(())
        }
        RpfSource::Nested(parent) => read_at(&parent.source, abs_offset, buf),
    }
}

pub(super) fn build_names(
    entries: &[RpfEntry],
    name_heap: &[u8],
    name_shift: u8,
) -> Result<Vec<String>> {
    let mut names = Vec::with_capacity(entries.len());
    for (i, e) in entries.iter().enumerate() {
        let raw_offset = e.name_offset();
        let absolute = (raw_offset as usize)
            .checked_shl(name_shift as u32)
            .ok_or_else(|| IoError::RpfBadEntry {
                message: format!(
                    "name_offset {} << {} overflows for entry {}",
                    raw_offset, name_shift, i
                ),
            })?;
        if absolute >= name_heap.len() {
            return Err(IoError::RpfBadEntry {
                message: format!(
                    "name_offset {} (>>{}) out of range for entry {}",
                    raw_offset, name_shift, i
                ),
            });
        }
        let end = name_heap[absolute..]
            .iter()
            .position(|&b| b == 0)
            .map(|p| absolute + p)
            .unwrap_or(name_heap.len());
        let s =
            std::str::from_utf8(&name_heap[absolute..end]).map_err(|_| IoError::RpfBadEntry {
                message: format!("entry {} has non-UTF8 name", i),
            })?;
        // Long names can lock up the UI in CodeWalker; we cap the same way.
        //
        // ⚠ On a **character** boundary, not a byte index: `&s[..256]` panics
        // the moment byte 256 lands inside a multi-byte character, which the
        // `from_utf8` above makes possible rather than prevents. Measured on a
        // 320-byte `Open` archive needing no keys at all: `end byte index 256 is
        // not a char boundary; it is inside '€'`.
        //
        // Byte 0 is always a boundary, so the search cannot come back empty.
        let s = if s.len() > MAX_ENTRY_NAME_BYTES {
            let cut = (0..=MAX_ENTRY_NAME_BYTES)
                .rev()
                .find(|&i| s.is_char_boundary(i))
                .unwrap_or(0);
            &s[..cut]
        } else {
            s
        };
        names.push(s.to_string());
    }
    Ok(names)
}

pub(super) fn build_tree(
    entries: &[RpfEntry],
    names: &[String],
) -> Result<(Vec<RpfNode>, Vec<u32>)> {
    let mut nodes: Vec<RpfNode> = Vec::with_capacity(entries.len());
    let mut entry_to_node: Vec<u32> = vec![u32::MAX; entries.len()];
    if entries.is_empty() {
        return Ok((nodes, entry_to_node));
    }
    let root_dir = match &entries[0] {
        RpfEntry::Directory(d) => d,
        _ => {
            return Err(IoError::RpfBadEntry {
                message: "RPF root entry is not a directory".into(),
            })
        }
    };

    nodes.push(RpfNode::Directory {
        entry_index: 0,
        name: String::new(),
        path: String::new(),
        children: Vec::new(),
    });
    entry_to_node[0] = 0;

    let root_first = root_dir.first_entry_index;
    let root_count = root_dir.entry_count;

    fill_children(
        entries,
        names,
        &mut nodes,
        &mut entry_to_node,
        Frame::new(0, root_first, root_count, String::new()),
    )?;

    Ok((nodes, entry_to_node))
}

struct Frame {
    parent_node_idx: u32,
    first: u32,
    count: u32,
    next: u32,
    path: String,
    child_node_indices: Vec<u32>,
}

impl Frame {
    fn new(parent_node_idx: u32, first: u32, count: u32, path: String) -> Self {
        Self {
            parent_node_idx,
            first,
            count,
            next: 0,
            path,
            child_node_indices: Vec::new(),
        }
    }
}

/// Walk the directory table, depth-first, without recursing.
///
/// ⚠ A recursive walk lets the archive decide the stack depth, and a stack
/// overflow is an abort nothing upstream can catch, so nothing gets saved. Two
/// archives did it: a directory listing itself as its own child, and 100,000
/// directories nested one inside the next in 1.6 MB. The visited set below
/// closes only the first; the explicit stack closes both.
///
/// Node indices come out in pre-order — a directory's own node, then its whole
/// subtree, then its next sibling — the order `entry_to_node` and the persisted
/// tree were built against.
fn fill_children(
    entries: &[RpfEntry],
    names: &[String],
    nodes: &mut Vec<RpfNode>,
    entry_to_node: &mut [u32],
    root: Frame,
) -> Result<()> {
    let mut stack: Vec<Frame> = vec![root];

    while let Some(frame) = stack.last_mut() {
        if frame.next >= frame.count {
            let done = stack.pop().expect("last_mut just succeeded");
            if let Some(RpfNode::Directory { children, .. }) =
                nodes.get_mut(done.parent_node_idx as usize)
            {
                *children = done.child_node_indices;
            }
            continue;
        }

        let k = frame.next;
        frame.next += 1;
        let parent_node_idx = frame.parent_node_idx;

        let entry_index = first_plus(frame.first, k, parent_node_idx)?;
        let entry = entries
            .get(entry_index as usize)
            .ok_or_else(|| IoError::RpfBadEntry {
                message: format!(
                    "child entry {} out of range (entry_count={})",
                    entry_index,
                    entries.len()
                ),
            })?;

        let already = entry_to_node
            .get(entry_index as usize)
            .copied()
            .unwrap_or(u32::MAX);
        if already != u32::MAX {
            return Err(IoError::RpfBadEntry {
                message: format!(
                    "entry {} is claimed by two directories (already node {}); \
                     the directory table has a cycle",
                    entry_index, already
                ),
            });
        }

        let name = names.get(entry_index as usize).cloned().unwrap_or_default();
        let child_path = if frame.path.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", frame.path, name)
        };

        let node_idx = nodes.len() as u32;
        let node = match entry.kind() {
            RpfEntryKind::Directory => RpfNode::Directory {
                entry_index,
                name: name.clone(),
                path: child_path.clone(),
                children: Vec::new(),
            },
            RpfEntryKind::Binary => RpfNode::BinaryFile {
                entry_index,
                name,
                path: child_path.clone(),
            },
            RpfEntryKind::Resource => RpfNode::ResourceFile {
                entry_index,
                name,
                path: child_path.clone(),
            },
        };
        nodes.push(node);
        if let Some(slot) = entry_to_node.get_mut(entry_index as usize) {
            *slot = node_idx;
        }
        if let Some(frame) = stack.last_mut() {
            frame.child_node_indices.push(node_idx);
        }

        if let RpfEntry::Directory(d) = entry {
            stack.push(Frame::new(
                node_idx,
                d.first_entry_index,
                d.entry_count,
                child_path,
            ));
        }
    }

    Ok(())
}

/// A zstd frame, which an RPF6 body may be in place of deflate.
fn is_zstd(bytes: &[u8]) -> bool {
    matches!(bytes, [first, 0xB5, 0x2F, 0xFD, ..] if first & 0xF0 == 0x20)
}

/// Decode a zstd frame, refusing one that expands past the `limit` bytes the entry declares.
fn zstd_bounded(bytes: &[u8], limit: usize) -> Result<Vec<u8>> {
    let refuse = |e: &dyn std::fmt::Display| IoError::DecompressionFailed(e.to_string());
    let decoder = ruzstd::decoding::StreamingDecoder::new(bytes).map_err(|e| refuse(&e))?;
    let mut out = Vec::new();
    decoder
        .take(limit as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| refuse(&e))?;
    Ok(out)
}

fn first_plus(first: u32, k: u32, parent_node_idx: u32) -> Result<u32> {
    first.checked_add(k).ok_or_else(|| IoError::RpfBadEntry {
        message: format!("child index overflow at parent node {}", parent_node_idx),
    })
}

fn read_big_resource_size(
    source: &RpfSource,
    start_pos: u64,
    entry: &RpfResourceEntry,
) -> Result<u32> {
    let abs_offset = start_pos + entry.file_offset;
    let mut buf = [0u8; Rsc7::HEADER_LEN];
    read_at(source, abs_offset, &mut buf)?;
    Ok(entry_size_from_slot(&buf))
}

impl RpfArchive {
    /// The file at `path`, decrypted and inflated: [`BodyForm::Decoded`].
    pub fn read(&self, path: &str) -> Result<Vec<u8>> {
        let entry = self.find(path).ok_or_else(|| IoError::RpfBadEntry {
            message: format!("no file `{path}` in the archive"),
        })?;
        self.read_entry(entry)
    }

    /// One entry's body, decrypted and inflated: [`BodyForm::Decoded`].
    pub fn read_entry(&self, entry_index: u32) -> Result<Vec<u8>> {
        self.read_form(entry_index, BodyForm::Decoded)
    }

    /// One entry's body, in whichever of the three forms the caller asked for.
    ///
    /// A binary entry has three distinct forms and a resource has **two**:
    ///
    /// | | binary | resource |
    /// |---|---|---|
    /// | [`AsStored`](BodyForm::AsStored) | encrypted and deflated, as on disk | as on disk — ⚠ **and its first sixteen bytes are not an RSC7 header** |
    /// | [`Decrypted`](BodyForm::Decrypted) | deflated, key applied | same as `Decoded`; nothing deflates a resource |
    /// | [`Decoded`](BodyForm::Decoded) | inflated | RSC7 header reconstructed from the entry |
    ///
    /// ⚠ A resource's on-disk header slot holds **random filler** the writer put
    /// there, and the real `system_info` and `graphics_info` live in the *entry
    /// table* — so a decoded read rebuilds the sixteen bytes. `AsStored`
    /// on a `.ydr` gives a file of the right length that nothing, OpenRage
    /// included, can open.
    pub fn read_form(&self, entry_index: u32, form: BodyForm) -> Result<Vec<u8>> {
        #[cfg(feature = "gta6")]
        if let RpfBackend::Rpf9(_) = self.backend {
            return self.rpf9_form(entry_index, form);
        }
        match self.entries.get(entry_index as usize) {
            Some(RpfEntry::Resource(_)) => match form {
                // Nothing inflates a resource, so these two are one answer.
                BodyForm::Decrypted | BodyForm::Decoded => {
                    self.read_prefix(entry_index, usize::MAX)
                }
                BodyForm::AsStored => self.resource_bytes_as_stored(entry_index),
            },
            Some(RpfEntry::Binary(_)) => self.binary_with_form(entry_index, form),
            _ => Err(IoError::RpfBadEntry {
                message: format!("entry {entry_index} has no body"),
            }),
        }
    }

    /// A resource's bytes exactly as the archive holds them.
    ///
    /// ⚠ Includes the sixteen-byte header slot **unmodified**, which is filler
    /// rather than an RSC7 header. See [`read_form`](Self::read_form).
    fn resource_bytes_as_stored(&self, entry_index: u32) -> Result<Vec<u8>> {
        let entry = self
            .resource_entry(entry_index)
            .ok_or_else(|| IoError::RpfBadEntry {
                message: format!("entry {entry_index} is not a resource"),
            })?;
        self.body(entry.file_offset, entry.consumed_size as usize)
    }

    /// `len` bytes of the archive at `file_offset`, refused before they are allocated when they lie past its end.
    fn body(&self, file_offset: u64, len: usize) -> Result<Vec<u8>> {
        if file_offset.saturating_add(len as u64) > self.size {
            return Err(IoError::UnexpectedEof {
                offset: file_offset,
                needed: len,
                available: usize::try_from(self.size.saturating_sub(file_offset))
                    .unwrap_or(usize::MAX),
            });
        }
        let mut out = vec![0u8; len];
        read_at(&self.source, self.start_pos + file_offset, &mut out)?;
        Ok(out)
    }

    /// What decrypts an entry's body: the archive's mode, or for RPF8 the entry's own key id with its config above it.
    fn body_mode(&self, tag: u32) -> RpfEncryption {
        match self.version {
            RpfVersion::Rpf8 => RpfEncryption::Tagged(tag & 0xFFFF),
            _ => self.header.encryption,
        }
    }

    fn binary_with_form(&self, entry_index: u32, form: BodyForm) -> Result<Vec<u8>> {
        let crypto = self.keys.as_ref();
        let entry = self
            .binary_entry(entry_index)
            .ok_or_else(|| IoError::RpfBadEntry {
                message: format!("entry {entry_index} is not a binary file"),
            })?;
        let mut on_disk = self.body(entry.file_offset, entry.read_size() as usize)?;
        if form == BodyForm::AsStored {
            return Ok(on_disk);
        }

        let inflate = |body: &[u8]| -> Result<Vec<u8>> {
            let wanted = entry.uncompressed_size as usize;
            let decompressed = match entry.compressor {
                Compressor::Oodle => super::oodle::decompress(body, wanted)?,
                _ if self.version == RpfVersion::Rpf6 && is_zstd(body) => {
                    zstd_bounded(body, wanted)?
                }
                _ => dat_decompress_bounded(body, wanted)?,
            };
            if decompressed.len() != wanted {
                return Err(IoError::DecompressionFailed(format!(
                    "expected {} bytes, got {}",
                    entry.uncompressed_size,
                    decompressed.len()
                )));
            }
            Ok(decompressed)
        };
        let decrypt = |body: &mut [u8]| -> Result<()> {
            // NG key derivation keys on the entry's own name (the basename, not
            // the archive-relative path).
            let name = self.entry_name(entry_index).unwrap_or_default();
            crypto.decrypt(
                self.body_mode(entry.tag),
                CryptoTarget::Body {
                    entry_name: &name,
                    keyed_size: entry.uncompressed_size,
                    // fiPackFile8 reads a deflated body 0x2000 at a time and a stored one 0x1000.
                    chunk: if entry.is_compressed() {
                        0x2000
                    } else {
                        0x1000
                    },
                },
                body,
            )
        };

        if entry.encrypted && self.version == RpfVersion::Rpf8 && entry.is_compressed() {
            // ⚠ The row gives a compressed body's stored size, and the cipher takes the stream's
            // length, which is that or 256 bytes less and is stated nowhere: both are tried and the
            // one that inflates is kept.
            let sealed = on_disk;
            let mut first = None;
            for keyed in [
                sealed.len().saturating_sub(RPF8_COMPRESSED_SLACK),
                sealed.len(),
            ] {
                let mut body = sealed.clone();
                decrypt(&mut body[..keyed])?;
                match inflate(&body) {
                    Ok(inflated) => {
                        return Ok(if form == BodyForm::Decrypted {
                            body
                        } else {
                            inflated
                        })
                    }
                    Err(e) => {
                        first.get_or_insert(e);
                    }
                }
            }
            return Err(first.expect("two attempts were made"));
        }
        if entry.encrypted {
            decrypt(&mut on_disk)?;
        }
        if form == BodyForm::Decrypted || !entry.is_compressed() {
            return Ok(on_disk);
        }
        inflate(&on_disk)
    }

    /// A resource entry's file, reading no more than `limit` bytes of it from the archive, and so
    /// the front of a file, not the whole.
    ///
    /// ⚠ A stored body is deflated, so what comes back is a file cut short, which `Rsc7::open`
    /// opens to the front of its payload. An entry that is decrypted whole (a `.ysc` in an
    /// encrypted archive) is read whole whatever `limit` says.
    pub fn read_prefix(&self, entry_index: u32, limit: usize) -> Result<Vec<u8>> {
        let crypto = self.keys.as_ref();
        let entry = self
            .resource_entry(entry_index)
            .ok_or_else(|| IoError::RpfBadEntry {
                message: format!("entry {} is not a resource", entry_index),
            })?
            .clone();
        let consumed = entry.consumed_size as usize;
        // RPF7 and RPF8 keep a sixteen-byte slot where the file's header goes and the entry holds
        // what the header says; before them the header is part of the stored file.
        let slot = match self.version {
            RpfVersion::Rpf7 | RpfVersion::Rpf8 => Rsc7::HEADER_LEN,
            _ => 0,
        };
        if consumed < slot {
            return Err(IoError::RpfBadEntry {
                message: format!(
                    "resource entry {} too small ({} bytes; need at least 16 for header slot)",
                    entry_index, consumed
                ),
            });
        }
        // A resource entry has no encrypted bit (its high u32 holds
        // graphics_info), so `.ysc` in an NG/AES archive is taken as encrypted,
        // as CodeWalker does. The NG key derives from the entry's filename, not
        // its full path. An RPF8 entry names its key.
        let name = self.entry_name(entry_index).unwrap_or_default();
        let sealed = match self.version {
            RpfVersion::Rpf7 => {
                self.header.encryption.needs_crypto() && name.to_ascii_lowercase().ends_with(".ysc")
            }
            RpfVersion::Rpf8 => layout::rpf8_is_encrypted(entry.tag),
            _ => false,
        };
        let taken = if sealed {
            consumed
        } else {
            consumed.min(limit.max(slot))
        };
        let mut on_disk = self.body(entry.file_offset, taken)?;

        if sealed {
            // ⚠ RPF8's cipher starts after the slot; RPF7's covers the whole body.
            let from = if self.version == RpfVersion::Rpf8 {
                slot
            } else {
                0
            };
            crypto.decrypt(
                self.body_mode(entry.tag),
                CryptoTarget::Body {
                    entry_name: &name,
                    keyed_size: entry.consumed_size,
                    // pgStreamer reads a compressed resource 0x80000 at a time and a stored one whole.
                    chunk: if entry.compressor != Compressor::None {
                        0x8_0000
                    } else {
                        (on_disk.len() - from) as u64
                    },
                },
                &mut on_disk[from..],
            )?;
        }

        let header = match self.version {
            RpfVersion::Rpf7 => {
                Rsc7::header_from_entry_flags(entry.system_info, entry.graphics_info).header_bytes()
            }
            RpfVersion::Rpf8 => layout::rpf8_resource_header(&entry, sealed).to_vec(),
            _ => return Ok(on_disk),
        };
        let mut out = Vec::with_capacity(taken);
        out.extend_from_slice(&header);
        out.extend_from_slice(&on_disk[slot..]);
        Ok(out)
    }

    /// The archive stored, uncompressed, as file `entry_index` of this one, which reads through it and holds its keys.
    pub fn open_nested(self: &Arc<RpfArchive>, entry_index: u32) -> Result<RpfArchive> {
        #[cfg(feature = "gta6")]
        if let RpfBackend::Rpf9(_) = self.backend {
            return self.rpf9_nested(entry_index);
        }
        let bin = self
            .binary_entry(entry_index)
            .ok_or_else(|| IoError::RpfBadEntry {
                message: format!(
                    "entry {} is not a binary file (cannot open as nested RPF)",
                    entry_index
                ),
            })?;
        if bin.is_compressed() {
            return Err(IoError::RpfBadEntry {
                message: format!(
                    "entry {} is a compressed nested RPF (unsupported)",
                    entry_index
                ),
            });
        }
        let start = self.start_pos + bin.file_offset;
        let name = self
            .node_for_entry(entry_index)
            .map(|n| n.name().to_string());
        Self::parse(
            RpfSource::Nested(Arc::clone(self)),
            start,
            bin.read_size() as u64,
            name,
            Arc::clone(&self.keys),
            None,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::rpf::entry::RpfDirectoryEntry;

    fn root_block(name_offset: u32, first_entry_index: u32, entry_count: u32) -> [u8; 16] {
        let mut buf = Vec::new();
        RpfEntry::Directory(RpfDirectoryEntry {
            name_offset,
            first_entry_index,
            entry_count,
        })
        .write(&mut buf)
        .expect("writes");
        buf.as_slice().try_into().expect("16 bytes")
    }

    #[test]
    fn the_recovery_predicate_accepts_a_real_root_directory() {
        assert!(is_plausible_root_entry(&root_block(0, 1, 7), 8, 0x40, 0));
    }

    #[test]
    fn the_recovery_predicate_rejects_each_way_a_root_can_be_wrong() {
        let cases: [(&str, [u8; 16], u32, u32, u8); 5] = [
            ("named root", root_block(4, 1, 7), 8, 0x40, 0),
            ("child index 0", root_block(0, 0, 7), 8, 0x40, 0),
            ("no children", root_block(0, 1, 0), 8, 0x40, 0),
            ("range overruns", root_block(0, 1, 7), 4, 0x40, 0),
            (
                "not a directory",
                {
                    let mut buf = Vec::new();
                    RpfEntry::Binary(crate::container::rpf::entry::RpfBinaryEntry {
                        name_offset: 0,
                        consumed_size: 16,
                        file_offset: 512,
                        uncompressed_size: 16,
                        encrypted: false,
                        encrypted_raw: 0,
                        compressor: Compressor::Deflate,
                        tag: 0,
                    })
                    .write(&mut buf)
                    .expect("writes");
                    buf.as_slice().try_into().expect("16 bytes")
                },
                8,
                0x40,
                0,
            ),
        ];

        let mut rejected = 0;
        for (why, block, total, heap, shift) in cases {
            assert!(
                !is_plausible_root_entry(&block, total, heap, shift),
                "{why} should not look like a root"
            );
            rejected += 1;
        }
        assert_eq!(rejected, 5, "every condition was exercised");
    }

    #[test]
    fn the_recovery_predicate_almost_never_accepts_noise() {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };

        let mut accepted = 0usize;
        const TRIALS: usize = 100_000;
        for _ in 0..TRIALS {
            let mut block = [0u8; 16];
            block[0..8].copy_from_slice(&next().to_le_bytes());
            block[8..16].copy_from_slice(&next().to_le_bytes());
            if is_plausible_root_entry(&block, 4096, 0x1000, 0) {
                accepted += 1;
            }
        }
        assert_eq!(
            accepted, 0,
            "{accepted} of {TRIALS} random blocks passed the root test"
        );
    }
}
