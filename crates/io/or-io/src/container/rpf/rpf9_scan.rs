//! A look into an RPF9 archive without its table: a pass over the file's 16-byte slots that finds
//! the bodies stored in the clear by their own bytes, so nothing it finds has a name or a path.
//! A body's end is stated only where its bytes state it or the measured rule has no exception; a
//! headerless resource ends after the blocks its block map counts.

use std::collections::VecDeque;
use std::io::{self, Read, Seek, SeekFrom};
use std::ops::Range;

use byteorder::{ByteOrder, BE, LE};
use flate2::{Decompress, FlushDecompress, Status};

use super::rpf9::{Rpf9Header, RPF9_HEADER_SIZE, RPF9_MAGIC};
use super::version::RSC8_MAGIC;
#[cfg(feature = "awc")]
use crate::awc::{stream_table_offset, AwcChunkKind, AwcFile, AwcKey, AWC_MAGIC_LE};
use crate::gta6::platform::ScanStrategy;

mod kept;
mod names_by_count;
mod raw_deflate;
mod stored;
mod stored_names;
pub use kept::{seal_rpf9_scan, unseal_rpf9_scan};
pub use names_by_count::{rpf9_names_by_count_at, Rpf9NamesByCount};
pub use raw_deflate::rpf9_block_streams;

/// Bodies start on the file's 16-byte boundaries, stored RSC8 resources on any even byte.
pub const RPF9_SLOT: u64 = 16;
/// The piece a chunked body is cut into before each piece is deflated or stored.
const PIECE: u64 = 0x1_0000;
const PAGE: u64 = 0x1000;
/// What an archive of one entry that would otherwise be empty holds; its names follow it.
pub const EMPTY_ARCHIVE_PLACEHOLDER: &[u8; 16] = b"FORCE_EMPTY_RPF\0";
/// How far before a block that starts no resource the start of the resource it belongs to is sought.
const OWNER_REACH: u64 = 64 << 20;
const WINDOW: usize = 8 << 20;
/// What a read of one body takes of the file at a time: most bodies are smaller, and a window of
/// [`WINDOW`] for each would read the archive many times over.
const BODY_WINDOW: usize = 256 << 10;
const INPUT_STEP: usize = 64 << 10;
/// Output one chunk, or a PSO run's section walk, may reach before the scan stops following it.
const CHUNK_LIMIT: u64 = 64 << 20;
/// A resource block's zlib stream inflates to at most this; one that reaches it goes on in the
/// next stream, which is the same block.
pub(super) const STREAM_CAP: u64 = 32 << 20;
/// The caps a block's streams were measured to stop at: 8 MiB in two nested archives, 32 MiB elsewhere.
pub(super) const STREAM_CAPS: [u64; 2] = [8 << 20, STREAM_CAP];
/// Compressed bytes a zlib head takes without an error before its failure counts as partway.
const VALIDATED_INPUT: u64 = 256;
const TEXT_PROBE: usize = 4096;
const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
/// A run of zero bytes no body claims that is at least this long is a piece a download has not
/// delivered: measured, zero gaps between present 4 KiB pages are at most three pages or at
/// least 256, and every gap of 256 pages or more is unallocated in the sparse file.
pub const RPF9_MISSING_RUN: u64 = 1 << 20;
#[cfg(feature = "awc")]
const AWC_PAGE: u64 = 0x1000;
#[cfg(feature = "awc")]
const AWC_EXTENT_LIMIT: u64 = 1 << 28;

/// Why a body cannot be read, or what about it is not known.
#[derive(Debug, thiserror::Error)]
pub enum Rpf9ScanError {
    #[error("reading the archive at {offset:#X}: {source}")]
    Io {
        offset: u64,
        #[source]
        source: io::Error,
    },
    #[error("nothing a scan without the table recognises starts at {offset:#X}")]
    NoBodyAt { offset: u64 },
    #[error("the stored RSC8 at {offset:#X} states config byte 1, which is taken only as a chunk of an audio container of flags 0xFF61: it is read through that container's listing, not at its offset alone")]
    ConfigOneOutsideContainer { offset: u64 },
    #[error("the RPF9 archive at {offset:#X} is not read as one body: its header states no extent, and its table, which lists what it holds, is encrypted")]
    ExtentNotKnown { offset: u64 },
    #[error("the bytes at {offset:#X} are in {section} block {block} of the resource at {owner:#X}, which a read at that offset returns with its other blocks")]
    BlockOfResource {
        offset: u64,
        owner: u64,
        section: &'static str,
        block: usize,
    },
    #[error("the zlib chunk at {offset:#X} fails partway through; what alters such a chunk is not known")]
    ChunkFailsPartway { offset: u64 },
    #[error("the body at {offset:#X} is cut short: the file ends at {file_len:#X} inside it")]
    Truncated { offset: u64, file_len: u64 },
    #[error("the resource that starts the file ends at {end:#X}, and the file goes on to {file_len:#X}: a file of one resource ends where it does")]
    Lengthened { end: u64, file_len: u64 },
    #[error("the body at {offset:#X} states {stated} bytes, which run past the end of the file at {file_len:#X}")]
    LengthPastEnd {
        offset: u64,
        stated: u64,
        file_len: u64,
    },
    #[error("the body at {offset:#X} inflates past the limit of {limit} bytes")]
    InflateLimit { offset: u64, limit: u64 },
    #[error("the resource at {offset:#X} continues at {at:#X} in a piece of the file a download has not delivered")]
    NotOnDisk { offset: u64, at: u64 },
    #[error("the body at {offset:#X} is not read: its zlib stream stops being deflate data at {at:#X} with no piece of the file missing before it, so its tail is replaced or protected, which its bytes do not tell apart")]
    TailNotDeflate { offset: u64, at: u64 },
    #[error("the resource at {offset:#X} is not read: at {at:#X}, {why}")]
    ResourceRefused {
        offset: u64,
        at: u64,
        why: &'static str,
    },
}

/// What a zlib run's first chunk inflates to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Rpf9Content {
    /// Text that opens with an XML declaration, or with an element at its first byte.
    Xml,
    /// Other text: plain text, or XML whose start is not in this run.
    Text,
    /// A PSO document: its first section is `PSIN`.
    Pso,
    /// An RBF metadata document: `RBF0` first.
    Rbf,
    /// Four zero bytes first: a head shape, not an identified format.
    ResourceShaped,
    /// The root of a headerless animation resource, a clip dictionary or a lone animation set.
    ClipRoot,
    /// GTA VI's pipeline cache: `PS`, 0 and version 9 first.
    PipelineCache,
    /// A headerless RBM resource whose class table declares a text table's map.
    TextTable,
    /// Audio game data: a schema word, a data length, and the data block's stamp at 8.
    AudioData,
    /// A body whose head a measured layout names, read as its variant says.
    Identified(crate::gta6::other_bodies::OtherBody),
    Other,
}

/// A run of zlib chunks, and of 64 KiB pieces stored raw between them in a PSO run; no chunk, and
/// pieces only where its end is stated, for a body of an archive that stores bodies as they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rpf9ZlibRun {
    pub content: Rpf9Content,
    pub chunks: u32,
    pub stored_pieces: u32,
    /// The first chunk's compressed bytes, without its zero pad.
    pub first_chunk_len: u64,
    pub first_chunk_inflated: u64,
    /// Every chunk and stored piece of the run, inflated.
    pub inflated: u64,
    /// A resource's signature, over its whole virtual section, where no `0DRP` is at 0x10.
    pub signature: Option<Rpf9Signature>,
    /// The resource the run starts, whose blocks the chunk counts and `inflated` then count.
    pub resource: Option<Rpf9Resource>,
}

/// A headerless resource's blocks: one or two virtual, then up to two physical, back to back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rpf9Resource {
    /// The block counts the block map states; both 0 where no block map was read.
    pub virtual_blocks: u8,
    pub physical_blocks: u8,
    /// The blocks found, in file order.
    pub blocks: [Option<Rpf9Block>; 4],
    /// The sum of the virtual blocks found, inflated.
    pub virtual_len: u64,
    /// The sum of the physical blocks found, inflated.
    pub physical_len: u64,
    pub walk: Rpf9Walk,
}

impl Rpf9Resource {
    /// The blocks after the first, which a scan lists under this resource and not as bodies.
    pub fn continuation_blocks(&self) -> usize {
        self.blocks.iter().flatten().count().saturating_sub(1)
    }

    /// The virtual and the physical section of `bytes`, which a read of this resource returned.
    pub fn sections<'a>(&self, bytes: &'a [u8]) -> (&'a [u8], &'a [u8]) {
        bytes.split_at((self.virtual_len as usize).min(bytes.len()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rpf9Block {
    pub offset: u64,
    /// Bytes in the file, without the zero pad after them.
    pub stored_len: u64,
    pub inflated_len: u64,
    /// False for bytes stored raw, whose end is certain to the 15 zero bytes a pad could hide.
    pub zlib: bool,
    pub physical: bool,
}

/// How far a resource's blocks were followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rpf9Walk {
    /// Every block the block map counts was found.
    Whole,
    /// The block at `at` lies in a piece a download has not delivered.
    NotOnDisk { at: u64 },
    /// What lies at `at` does not hold to the measured layout, for the reason `why` gives.
    Refused { at: u64, why: &'static str },
    /// A zlib stream stops being deflate data at `at` with no piece missing before it: its tail
    /// is replaced or protected, which its bytes do not tell apart.
    TailNotDeflate { at: u64 },
    /// Every block was found, and `targets` pointer words aim just past the `section` section:
    /// values, or a block of another resource, which the bytes do not tell apart.
    PointersPast {
        section: &'static str,
        targets: usize,
    },
}

/// The classes of a headerless resource's first eight u64 slots, and the role its head places.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Rpf9Signature {
    classes: [u8; 8],
    role: Option<Rpf9Role>,
}

impl Rpf9Signature {
    /// The signature of a virtual section that starts with `head` and holds `len` bytes.
    pub fn of(head: &[u8], len: u64) -> Option<Self> {
        // `0` zero, `P` a 0x5000_0000 pointer below `len`, `p` one at or past it, `G` a
        // 0x6000_0000 word, `h` anything else.
        // ⚠ A float that is exactly zero turns an `h` into a `0`, so one layout can have two
        // signatures.
        let slots = head.get(..0x40)?;
        let mut classes = [0u8; 8];
        for (class, slot) in classes.iter_mut().zip(slots.as_chunks::<8>().0) {
            let word = u64::from_le_bytes(*slot);
            let low = word as u32;
            *class = match (word >> 32, low >> 28) {
                _ if word == 0 => b'0',
                (0, 5) if u64::from(low & 0x0FFF_FFFF) < len => b'P',
                (0, 5) => b'p',
                (0, 6) => b'G',
                _ => b'h',
            };
        }
        Some(Self {
            classes,
            role: Rpf9Role::place(&classes, head, len),
        })
    }

    pub fn as_bytes(&self) -> &[u8; 8] {
        &self.classes
    }

    /// The family the measured layouts place this resource in; `None` where none does.
    pub fn role(self) -> Option<Rpf9Role> {
        self.role
    }
}

impl std::fmt::Display for Rpf9Signature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.classes
            .iter()
            .try_for_each(|c| write!(f, "{}", char::from(*c)))
    }
}

/// A family of headerless resources whose layout was measured, by role: no engine name is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Rpf9Role {
    /// A composite of geometry bounds with quantised vertices, primitives and a BVH.
    Bound,
    /// Shaders, levels, models and geometries over a meshlet index list.
    Drawable,
    /// Ascending codes and the entries they key.
    Dictionary,
    /// A fragment: a name, a drawable at 0x18 and, where 0x10 is set, a record with a bound.
    DrawableWithBound,
    /// A navigation mesh of one 128 m cell: polygons, their adjacency and a quadtree.
    NavMesh,
    /// A sparse octree of one 128 m navigation cell, keyed by location code.
    NavOctree,
    /// Ray-tracing instances, whose acceleration structures fill the physical section.
    AccelerationStructure,
    /// A cloth record of type 1 to 5, the type its byte 0x1A states.
    Cloth,
    /// A morph target: a mesh's key and a vertex buffer of quantised deltas with no declaration.
    MorphTarget,
    /// An array of vertex buffers with their declarations, and no index buffer or drawable.
    VertexBuffers,
    /// A blob under a 64-bit key, behind a raw buffer object or in the virtual section.
    KeyedBlob,
    /// The root that names a keyed blob by its key, over a stream whose grammar is not known.
    KeyedBlobDescriptor,
    /// Group hash tables from 64-bit name hashes to asset records and to lists of asset keys.
    AssetIndex,
    /// A raw buffer of elements of four indices, each of another element.
    NeighbourTable,
    /// Effect, emitter and particle rules in three dictionaries, after a property registry.
    ParticleEffects,
}

impl Rpf9Role {
    pub fn label(self) -> &'static str {
        match self {
            Self::Bound => "bound",
            Self::Drawable => "drawable",
            Self::Dictionary => "dictionary",
            Self::DrawableWithBound => "fragment",
            Self::NavMesh => "navigation mesh",
            Self::NavOctree => "navigation octree",
            Self::AccelerationStructure => "acceleration structure",
            Self::Cloth => "cloth",
            Self::MorphTarget => "morph target",
            Self::VertexBuffers => "vertex buffers",
            Self::KeyedBlob => "keyed blob",
            Self::KeyedBlobDescriptor => "keyed-blob descriptor",
            Self::AssetIndex => "asset index",
            Self::NeighbourTable => "neighbour table",
            Self::ParticleEffects => "particle effects",
        }
    }

    /// The family of a virtual section of `len` bytes whose first slots have `classes` and whose
    /// bytes start with `head`.
    fn place(classes: &[u8; 8], head: &[u8], len: u64) -> Option<Self> {
        let u32_at = |at: usize| head.get(at..at + 4).map(LE::read_u32);
        let u64_at = |at: usize| head.get(at..at + 8).map(LE::read_u64);
        let u16_at = |at: usize| head.get(at..at + 2).map(LE::read_u16);
        match classes {
            b"0Ph0hhhh" => return Some(Self::Bound),
            b"0PP0hhhh" | b"0PP00hhh" => return Some(Self::Drawable),
            b"0P0hPhPh" => return Some(Self::Dictionary),
            // The registry and the three dictionaries lie past the 0x70-byte root.
            b"0PP0PPPP"
                if (0x20..0x40)
                    .step_by(8)
                    .all(|at| u64_at(at).is_some_and(|w| w & 0x0FFF_FFFF >= 0x70)) =>
            {
                return Some(Self::ParticleEffects)
            }
            b"0PPPPPh0" => return Some(Self::DrawableWithBound),
            b"0Ph0h0h0" if u32_at(0x14) == Some(0x0001_0015) => return Some(Self::NavMesh),
            b"0PhPPhh0" if u64_at(0x18) == Some(0x5000_0060) => return Some(Self::NavOctree),
            b"0PhPGPPP" if u64_at(0x38) == Some(0x5000_0050) => {
                return Some(Self::AccelerationStructure)
            }
            // Its root points at a vertex buffer at 0x70 that names no declaration.
            b"0PP00000"
                if u64_at(0x10) == Some(0x5000_0070)
                    && u32_at(0x80) == Some(0x1601)
                    && u64_at(0xB0) == Some(0) =>
            {
                return Some(Self::MorphTarget)
            }
            // An array at 0x10 whose u16 count at 0x18 equals its capacity at 0x1A.
            b"0PPh00hh"
                if u16_at(0x18).is_some_and(|n| n != 0)
                    && u16_at(0x18) == u16_at(0x1A)
                    && u32_at(0x1C) == Some(0) =>
            {
                return Some(Self::VertexBuffers)
            }
            // The key at 0x20 and the length at 0x28 follow a raw buffer object at 0x30 or the
            // pointer to the blob there.
            b"0PP0hh0h" if u64_at(0x10) == Some(0x5000_0030) && u32_at(0x40) == Some(0x1380) => {
                return Some(Self::KeyedBlob)
            }
            b"0P0Phhhh" if u64_at(0x18) == Some(0x5000_0030) => return Some(Self::KeyedBlob),
            // An empty dictionary: ref count 1 at 0x18 and null arrays.
            b"0P0h0000" if u64_at(0x18) == Some(1) => return Some(Self::Dictionary),
            // A stream, or table 0, at 0x60, just past the root.
            b"0PhP00hh" if u64_at(0x18) == Some(0x5000_0060) => {
                return Some(Self::KeyedBlobDescriptor)
            }
            b"0PhP0000" | b"0PhPhP00" if u64_at(0x18) == Some(0x5000_0060) => {
                return Some(Self::AssetIndex)
            }
            // A raw buffer object at 0x20 with a vertex buffer's flags.
            b"0PP00hhG" if u64_at(0x10) == Some(0x5000_0020) && u32_at(0x30) == Some(0x1601) => {
                return Some(Self::NeighbourTable)
            }
            _ => {}
        }
        let (flag, kind, owner) = (*head.get(0x18)?, *head.get(0x1A)?, u32_at(0x1C)?);
        let class = |i: usize| classes[i];
        if (1..=5).contains(&kind)
            && class(1) == b'P'
            && matches!(class(2), b'0' | b'P')
            && flag <= 2
            && (matches!(class(3), b'h' | b'G' | b'p') || owner != 0)
            && (matches!(kind, 3 | 5) || class(5) == b'P')
        {
            // Type 4 is one box: all ones at 0x20, then arrays of capacity 16.
            let collider = u32_at(0x20) == Some(u32::MAX) && u16_at(0x32) == Some(16);
            return (kind != 4 || collider).then_some(Self::Cloth);
        }
        // A pointer past the section: a block of it is not read.
        if classes.contains(&b'p') {
            return None;
        }
        let pointer_in = |word: u64| word >> 28 == 5 && word & 0x0FFF_FFFF < len;
        match classes {
            b"0P0P00h0" | b"0PPPPPhP" => Some(Self::DrawableWithBound),
            b"0PPPhhhh" | b"0PP00hh0" | b"0PP0hhh0" | b"0PPP0hhh" | b"0PP0hh0h" | b"0PPPhhh0"
            | b"0PPP0hh0" | b"0PP0hh00" | b"0PP00000"
                if u64_at(0x50).is_some_and(|w| w != 0) && u64_at(0xA8).is_some_and(pointer_in) =>
            {
                Some(Self::Drawable)
            }
            _ => None,
        }
    }
}

/// What a body found by a scan is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rpf9BodyKind {
    /// An RPF9 header and its encrypted table, at offset 0 or nested in the archive.
    Archive(Rpf9Header),
    Zlib(Rpf9ZlibRun),
    /// An RSC8 resource stored unkeyed with no physical section: 16 bytes and its virtual size.
    StoredRsc8 {
        version: u8,
        extent: u64,
    },
    /// A Bink 2 movie whose head states its file length; `revision` is the letter after `KB2`.
    Bink2 {
        revision: u8,
        length: u64,
    },
    /// An `ADAT` audio container at a 4 KiB page that the tree's AWC reader lists with a chunk,
    /// ending at the furthest chunk its table states; found with `awc`.
    Awc {
        flags: u16,
        streams: u32,
        chunks: u32,
    },
    /// GTA VI's `SGD5` effect container, which states no end: its span runs to the next entry.
    Sgd5 {
        head_layouts: u32,
    },
    /// The encrypted names that end the nested archive whose header is at `archive`, placed
    /// after its last entry by the zero bytes from them to the next header.
    Names {
        archive: u64,
    },
    /// The first entry after a table, where the header puts it, in bytes no detector names; only
    /// its first slot is claimed.
    FirstEntry {
        head: [u8; 4],
    },
    /// A raw DEFLATE stream of a first entry that holds one per entry of its archive: PSO or RBF.
    RawDeflate {
        content: Rpf9Content,
    },
}

/// A body a scan found by its bytes alone: it has an offset and no name or path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rpf9ScanFind {
    pub offset: u64,
    pub kind: Rpf9BodyKind,
    /// Bytes from `offset` the scan stepped over as this body's, zero pad included.
    pub span: u64,
    /// The body's stored length, where the format states it or the measured rule has no exception.
    pub stored_len: Option<u64>,
    /// The body's inflated length, on the same terms.
    pub inflated_len: Option<u64>,
    /// The audio container whose chunk table states this body, inside whose span it lies.
    pub container: Option<u64>,
}

impl Rpf9ScanFind {
    /// Said of every find, since the table that names and bounds the entries is encrypted.
    pub const ORIGIN: &'static str =
        "found by a scan without the table: no name, no path, and an end only where the bytes state it";

    /// The name of a file [`extract_rpf9_body`] writes this body to, from its offset and kind alone.
    pub fn file_name(&self) -> String {
        let ext = match self.kind {
            Rpf9BodyKind::Zlib(run) => match (run.content, run.resource) {
                (Rpf9Content::Xml, _) => "xml",
                (Rpf9Content::Text, _) => "txt",
                (Rpf9Content::Pso, _) => "pso",
                // A `.ymt` is read by the metadata reader its first bytes pick.
                (Rpf9Content::Rbf, _) => "ymt",
                (
                    Rpf9Content::ResourceShaped | Rpf9Content::ClipRoot | Rpf9Content::TextTable,
                    Some(_),
                ) => "rsc",
                _ => "bin",
            },
            Rpf9BodyKind::StoredRsc8 { .. } => "rsc8",
            Rpf9BodyKind::Bink2 { .. } => "bik",
            Rpf9BodyKind::Awc { .. } => "awc",
            Rpf9BodyKind::Sgd5 { .. } => "sgd5",
            Rpf9BodyKind::Archive(_) => "rpf",
            Rpf9BodyKind::Names { .. } | Rpf9BodyKind::FirstEntry { .. } => "bin",
            Rpf9BodyKind::RawDeflate {
                content: Rpf9Content::Pso,
            } => "pso",
            Rpf9BodyKind::RawDeflate { .. } => "ymt",
        };
        format!("{:#X}.{ext}", self.offset)
    }
}

/// The version byte of a stored RSC8 that is an animation resource.
pub(crate) const RSC8_ANIMATION_VERSION: u8 = 65;

/// The family a scanned body is listed under, which a command line names by its word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Rpf9Family {
    Models,
    /// Of drawables or of textures: a scan does not read far enough to tell which.
    Dictionaries,
    Collision,
    Navigation,
    Animations,
    Metadata,
    Text,
    Audio,
    Movies,
    Shaders,
    Archives,
    /// A resource of a family with no entry of its own here, or of none the layouts place.
    Other,
    Unknown,
}

impl Rpf9Family {
    pub const ALL: [Self; 13] = [
        Self::Models,
        Self::Dictionaries,
        Self::Collision,
        Self::Navigation,
        Self::Animations,
        Self::Metadata,
        Self::Text,
        Self::Audio,
        Self::Movies,
        Self::Shaders,
        Self::Archives,
        Self::Other,
        Self::Unknown,
    ];

    /// The family of the body `find` is.
    pub fn of(find: &Rpf9ScanFind) -> Self {
        use Rpf9Role as Role;
        match find.kind {
            Rpf9BodyKind::Archive(_)
            | Rpf9BodyKind::Names { .. }
            | Rpf9BodyKind::FirstEntry { .. } => Self::Archives,
            Rpf9BodyKind::Zlib(run) => match run.content {
                Rpf9Content::Xml | Rpf9Content::Text => Self::Text,
                Rpf9Content::Pso | Rpf9Content::Rbf => Self::Metadata,
                Rpf9Content::ResourceShaped => match run.signature.and_then(|s| s.role()) {
                    Some(
                        Role::Drawable | Role::DrawableWithBound | Role::Cloth | Role::MorphTarget,
                    ) => Self::Models,
                    Some(Role::Dictionary) => Self::Dictionaries,
                    Some(Role::Bound) => Self::Collision,
                    Some(Role::NavMesh | Role::NavOctree) => Self::Navigation,
                    Some(
                        Role::AccelerationStructure
                        | Role::NeighbourTable
                        | Role::VertexBuffers
                        | Role::KeyedBlob
                        | Role::KeyedBlobDescriptor
                        | Role::AssetIndex
                        | Role::ParticleEffects,
                    )
                    | None => Self::Other,
                },
                Rpf9Content::ClipRoot => Self::Animations,
                Rpf9Content::PipelineCache => Self::Shaders,
                Rpf9Content::TextTable => Self::Text,
                Rpf9Content::AudioData => Self::Audio,
                Rpf9Content::Identified(body) => match body {
                    crate::gta6::other_bodies::OtherBody::Gfx => Self::Movies,
                    crate::gta6::other_bodies::OtherBody::Edup => Self::Unknown,
                    _ => Self::Other,
                },
                Rpf9Content::Other => Self::Unknown,
            },
            Rpf9BodyKind::StoredRsc8 {
                version: RSC8_ANIMATION_VERSION,
                ..
            } => Self::Animations,
            Rpf9BodyKind::StoredRsc8 { .. } => Self::Other,
            Rpf9BodyKind::Bink2 { .. } => Self::Movies,
            Rpf9BodyKind::Awc { .. } => Self::Audio,
            Rpf9BodyKind::Sgd5 { .. } => Self::Shaders,
            Rpf9BodyKind::RawDeflate { .. } => Self::Metadata,
        }
    }

    /// The word a command line and a JSON listing name this family by.
    pub fn word(self) -> &'static str {
        match self {
            Self::Models => "models",
            Self::Dictionaries => "dictionaries",
            Self::Collision => "collision",
            Self::Navigation => "navigation",
            Self::Animations => "animations",
            Self::Metadata => "metadata",
            Self::Text => "text",
            Self::Audio => "audio",
            Self::Movies => "movies",
            Self::Shaders => "shaders",
            Self::Archives => "archives",
            Self::Other => "other",
            Self::Unknown => "unknown",
        }
    }

    /// The family `word` names.
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|family| family.word() == word)
    }
}

/// A pass over a range of an RPF9 file, one find at a time; `position` is where a later pass resumes.
pub struct Rpf9Scan<R> {
    reader: Reader<R>,
    pos: u64,
    end: u64,
    unclaimed: u64,
    unclaimed_zero: u64,
    missing: u64,
    missing_runs: u64,
    zero_run: u64,
    broken_chunks: u64,
    tails_not_deflate: u64,
    queued: VecDeque<Rpf9ScanFind>,
    failed: bool,
    carry: Rpf9ScanCarry,
}

/// What a scan of one range hands the scan of the next: what the archive headers found so far say
/// comes after them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rpf9ScanCarry {
    /// The last nested archive header, its names size and entry count, until its names are placed.
    archive: Option<(u64, u32, u32)>,
    /// The table's end, the page after the one holding it and the archive's entry count, until
    /// the first entry is found.
    first_entry: Option<(u64, u64, u32)>,
    /// Whether the last find ends where its bytes state, so names may follow it.
    after_end: bool,
    /// Whether the last find's end is not stated, so names may start at any slot after it.
    end_unknown: bool,
}

impl<R: Read + Seek> Rpf9Scan<R> {
    /// A scan of the bodies that start in `range`, read through bounded buffers; a body may end past it.
    pub fn new(source: R, range: Range<u64>) -> Result<Self, Rpf9ScanError> {
        let reader = Reader::new(source)?;
        Ok(Self {
            reader,
            pos: range.start.next_multiple_of(2),
            end: range.end,
            unclaimed: 0,
            unclaimed_zero: 0,
            missing: 0,
            missing_runs: 0,
            zero_run: 0,
            broken_chunks: 0,
            tails_not_deflate: 0,
            queued: VecDeque::new(),
            failed: false,
            carry: Rpf9ScanCarry::default(),
        })
    }

    /// This scan, going on from where the scan of the range before left `carry`.
    pub fn carrying(mut self, carry: Rpf9ScanCarry) -> Self {
        self.carry = carry;
        self
    }

    /// What the scan of the next range takes from this one.
    pub fn carry(&self) -> Rpf9ScanCarry {
        self.carry
    }

    /// This scan, keeping the first `bytes` inflated bytes of each zlib run and resource it finds,
    /// and of each stored RSC8 its bytes as stored.
    pub fn keeping(mut self, bytes: usize) -> Self {
        self.reader.keep = bytes;
        self
    }

    /// The bytes kept of `find`, where it is the find this scan returned last: what a read of it
    /// starts with. None of a find it kept nothing of, or of an earlier one.
    pub fn kept(&self, find: &Rpf9ScanFind) -> &[u8] {
        let kept = matches!(
            find.kind,
            Rpf9BodyKind::Zlib(_) | Rpf9BodyKind::StoredRsc8 { .. }
        ) && find.container.is_none()
            && self.reader.kept_at == find.offset;
        match kept {
            true => &self.reader.kept,
            false => &[],
        }
    }

    /// Bytes in runs of at least [`RPF9_MISSING_RUN`] zero bytes no body claims, which are not
    /// counted as unclaimed; a run split between two scans of a file is judged in each.
    pub fn missing_bytes(&self) -> u64 {
        self.missing
    }

    /// How many such runs of missing bytes the scan met.
    pub fn missing_runs(&self) -> u64 {
        self.missing_runs
    }

    fn end_zero_run(&mut self) {
        let run = std::mem::take(&mut self.zero_run);
        if run >= RPF9_MISSING_RUN {
            self.unclaimed -= run;
            self.unclaimed_zero -= run;
            self.missing += run;
            self.missing_runs += 1;
        }
    }

    /// The file's length when the scan opened it.
    pub fn file_len(&self) -> u64 {
        self.reader.src.len
    }

    /// The offset the scan has reached, which a scan of the next range starts from.
    pub fn position(&self) -> u64 {
        self.pos
    }

    /// Bytes of the slots this scan examined and claimed nothing about, missing bytes apart.
    pub fn unclaimed_bytes(&self) -> u64 {
        self.unclaimed
    }

    /// How many of the unclaimed bytes are zero, as padding and pieces a sparse file lacks read.
    pub fn unclaimed_zero_bytes(&self) -> u64 {
        self.unclaimed_zero
    }

    /// zlib heads that inflated their first 256 compressed bytes and then failed.
    pub fn broken_chunks(&self) -> u64 {
        self.broken_chunks
    }

    /// How many of those failed with no zero page before the failure: their tails are not deflate.
    // Their bytes from the failure on are replaced or protected, not deflate data.
    pub fn tails_not_deflate(&self) -> u64 {
        self.tails_not_deflate
    }

    /// The bytes from the scan's position to the next slot: a body of any kind at the slot, a
    /// stored RSC8 at any even offset. The first find, with the position moved past it; else
    /// `None`, with the position at the next slot.
    fn step(&mut self) -> Result<Option<Rpf9ScanFind>, Rpf9ScanError> {
        let at = self.pos;
        let slot_end = (at + 1).next_multiple_of(RPF9_SLOT);
        let on_slot = at.is_multiple_of(RPF9_SLOT);
        let after_end = std::mem::take(&mut self.carry.after_end);
        if let (true, true, Some((archive, size, _))) = (on_slot, after_end, self.carry.archive) {
            if let Some(names) = self.reader.names_after(at, size, true)? {
                self.unclaim(at, names - at)?;
                return Ok(Some(self.claim(names_find(
                    names,
                    u64::from(size),
                    archive,
                ))));
            }
        }
        let mut broken = false;
        if on_slot
            && (may_start_body(self.reader.get(at, 4)?) || self.reader.may_start_stored(at)?)
        {
            match self.reader.find_at(at, None, u64::MAX)? {
                Found::Body(find) => return self.claim_before_names(*find).map(Some),
                Found::Broken(tail) => {
                    self.broken_chunks += 1;
                    self.tails_not_deflate += u64::from(tail.is_some());
                    broken = true;
                }
                Found::Nothing => {}
            }
        }
        // Measured: a stored RSC8 starts at an even offset, a 16-byte slot in one of four.
        let first = if on_slot { at + 2 } else { at };
        let window = self.reader.get(first, (slot_end - first) as usize + 3)?;
        let hits: Vec<u64> = (0..slot_end.saturating_sub(first))
            .step_by(2)
            .filter(|k| window.get(*k as usize..*k as usize + 4) == Some(&RSC8_MAGIC.to_le_bytes()))
            .map(|k| first + k)
            .filter(|q| *q < self.end)
            .collect();
        for q in hits {
            if let Found::Body(find) = self.reader.find_at(q, None, u64::MAX)? {
                self.unclaim(at, q - at)?;
                return Ok(Some(self.claim(*find)));
            }
        }
        // Measured: the first non-zero slot after a table is its end or the page after the one
        // holding it, or 16 bytes on behind a zero slot.
        if let (true, Some((table_end, page, entries))) = (on_slot, self.carry.first_entry) {
            let head: [u8; 16] = self.reader.get(at, 16)?.try_into().unwrap_or_default();
            if head != [0; 16] {
                self.carry.first_entry = None;
                if !broken && [table_end, table_end + 16, page, page + 16].contains(&at) {
                    let mut first = [0; 4];
                    first.copy_from_slice(&head[..4]);
                    // An archive of one entry that would be empty holds this placeholder alone.
                    let placeholder = head == *EMPTY_ARCHIVE_PLACEHOLDER && entries == 1;
                    // A first entry of raw DEFLATE streams, one per entry, lists each stream.
                    if !placeholder {
                        if let Some(stream) = self.raw_deflate_entries(at)? {
                            return Ok(Some(stream));
                        }
                    }
                    let stated = placeholder.then_some(RPF9_SLOT);
                    return Ok(Some(self.claim(Rpf9ScanFind {
                        offset: at,
                        kind: Rpf9BodyKind::FirstEntry { head: first },
                        span: RPF9_SLOT,
                        stored_len: stated,
                        inflated_len: stated,
                        container: None,
                    })));
                }
            } else if at >= page + 16 {
                self.carry.first_entry = None;
            }
        }
        // Behind a body whose end is not stated, the names may start at any slot no body claims.
        if let (true, true, None, Some((archive, size, _))) = (
            on_slot,
            self.carry.end_unknown,
            self.carry.first_entry,
            self.carry.archive,
        ) {
            // A blob wider than half the read window would refill the window twice at every slot.
            let narrow = (size as usize) < WINDOW / 2;
            if narrow && self.reader.names_after(at, size, false)?.is_some() {
                return Ok(Some(self.claim(names_find(at, u64::from(size), archive))));
            }
        }
        self.unclaim(at, slot_end - at)?;
        self.pos = slot_end;
        Ok(None)
    }

    /// `find`, with the position moved past it and what it says of the bytes after it kept.
    fn claim(&mut self, find: Rpf9ScanFind) -> Rpf9ScanFind {
        self.end_zero_run();
        self.pos = match find.kind {
            // The next stored resource may start where this one ends.
            Rpf9BodyKind::StoredRsc8 { extent, .. } => find.offset + extent,
            _ => (find.offset + find.span).next_multiple_of(RPF9_SLOT),
        };
        let carry = &mut self.carry;
        carry.first_entry = None;
        carry.after_end = false;
        match find.kind {
            Rpf9BodyKind::Archive(header) => {
                let table_end = find.offset + u64::from(header.toc_end);
                let page = (table_end / PAGE + 1) * PAGE;
                carry.first_entry = Some((table_end, page, header.entry_count));
                // The names of the archive at 0 end the file, so its extent is the file's.
                carry.archive = (find.offset > 0).then_some((
                    find.offset,
                    header.names_size,
                    header.entry_count,
                ));
            }
            Rpf9BodyKind::Names { .. } => carry.archive = None,
            Rpf9BodyKind::Zlib(run) if run.resource.is_none() => {
                // A run that ends on a short chunk ends there unless a stored piece follows.
                carry.after_end = find.stored_len.is_some() || run.inflated % PIECE != 0;
            }
            Rpf9BodyKind::Sgd5 { .. } => {}
            _ => carry.after_end = find.stored_len.is_some(),
        }
        // A short last chunk with no stated end may still be followed by bytes no body claims
        // before the names (story000.rpf 0x3E23DF000; scan-recount rule E).
        let short_unstated =
            matches!(find.kind, Rpf9BodyKind::Zlib(_)) && find.stored_len.is_none();
        carry.end_unknown = (!carry.after_end || short_unstated)
            && !matches!(
                find.kind,
                Rpf9BodyKind::Archive(_) | Rpf9BodyKind::Names { .. }
            );
        self.queued.extend(self.reader.nested.drain(..));
        find
    }

    /// The `len` bytes at `at`, counted as unclaimed.
    fn unclaim(&mut self, at: u64, len: u64) -> Result<(), Rpf9ScanError> {
        if len == 0 {
            return Ok(());
        }
        let bytes = self
            .reader
            .src
            .get(at, len as usize)
            .map_err(|source| Rpf9ScanError::Io { offset: at, source })?;
        let (len, zero) = (
            bytes.len() as u64,
            bytes.iter().filter(|b| **b == 0).count() as u64,
        );
        self.unclaimed += len;
        self.unclaimed_zero += zero;
        if zero == len {
            self.zero_run += len;
        } else {
            self.end_zero_run();
        }
        Ok(())
    }
}

impl<R: Read + Seek> Iterator for Rpf9Scan<R> {
    type Item = Result<Rpf9ScanFind, Rpf9ScanError>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(find) = self.queued.pop_front() {
            return Some(Ok(find));
        }
        if self.failed {
            return None;
        }
        while self.pos < self.end.min(self.reader.src.len) {
            match self.step() {
                Ok(Some(find)) => return Some(Ok(find)),
                Ok(None) => {}
                Err(e) => {
                    self.failed = true;
                    return Some(Err(e));
                }
            }
        }
        self.end_zero_run();
        None
    }
}

/// The body a scan would find at `offset`, and its bytes, inflated; refused past `limit` bytes.
pub fn read_rpf9_body<R: Read + Seek>(
    source: R,
    offset: u64,
    limit: u64,
) -> Result<(Rpf9ScanFind, Vec<u8>), Rpf9ScanError> {
    read_body(&mut Reader::windowed(source, BODY_WINDOW)?, offset, limit)
}

/// Bodies read one after another through one window of the file, as [`read_rpf9_body`] reads
/// one: a body that follows the last closely is read from the window the last one left.
pub struct Rpf9BodyReader<R> {
    reader: Reader<R>,
}

impl<R: Read + Seek> Rpf9BodyReader<R> {
    pub fn new(source: R) -> Result<Self, Rpf9ScanError> {
        Ok(Self {
            reader: Reader::windowed(source, BODY_WINDOW)?,
        })
    }

    /// The body a scan would find at `offset`, and its bytes, inflated; refused past `limit` bytes.
    pub fn read(
        &mut self,
        offset: u64,
        limit: u64,
    ) -> Result<(Rpf9ScanFind, Vec<u8>), Rpf9ScanError> {
        read_body(&mut self.reader, offset, limit)
    }
}

fn read_body<R: Read + Seek>(
    reader: &mut Reader<R>,
    offset: u64,
    limit: u64,
) -> Result<(Rpf9ScanFind, Vec<u8>), Rpf9ScanError> {
    let mut bytes = Vec::new();
    let found = reader.find_at(offset, Some(&mut bytes), limit)?;
    // The placeholder's 16 bytes state its end: read alone, it is the row the scan lists.
    if found == Found::Nothing && offset.is_multiple_of(RPF9_SLOT) {
        let head = reader.get(offset, 16)?;
        if head == EMPTY_ARCHIVE_PLACEHOLDER.as_slice() {
            let find = Rpf9ScanFind {
                offset,
                kind: Rpf9BodyKind::FirstEntry {
                    head: [head[0], head[1], head[2], head[3]],
                },
                span: RPF9_SLOT,
                stored_len: Some(RPF9_SLOT),
                inflated_len: Some(RPF9_SLOT),
                container: None,
            };
            return Ok((find, head.to_vec()));
        }
    }
    // A raw DEFLATE entry of metadata has no head a detector starts on.
    if found == Found::Nothing && offset.is_multiple_of(RPF9_SLOT) {
        if let Some(read) = reader.raw_deflate_body(offset, limit)? {
            return Ok(read);
        }
    }
    // A block of a resource after its first is zlib or stored raw, and starts no resource.
    let may_be_a_block = match &found {
        Found::Body(find) => match &find.kind {
            Rpf9BodyKind::Zlib(run) => {
                run.resource.is_none()
                    && matches!(
                        run.content,
                        Rpf9Content::ResourceShaped | Rpf9Content::ClipRoot | Rpf9Content::Other
                    )
            }
            _ => false,
        },
        Found::Broken(_) | Found::Nothing => true,
    };
    if may_be_a_block && offset.is_multiple_of(RPF9_SLOT) {
        if let Some(block) = reader.owner_of(offset)? {
            return Err(block);
        }
    }
    // A file that starts with a resource holds it alone, ended as [`extract_rpf9_body`] ends it.
    if let Found::Body(find) = &found {
        if let (
            0,
            Rpf9BodyKind::Zlib(Rpf9ZlibRun {
                resource: Some(_), ..
            }),
            Some(stored),
        ) = (offset, &find.kind, find.stored_len)
        {
            let end = stored.next_multiple_of(RPF9_SLOT) + RPF9_SLOT;
            let file_len = reader.src.len;
            if file_len > end {
                return Err(Rpf9ScanError::Lengthened { end, file_len });
            }
        }
    }
    match found {
        Found::Body(find) => Ok((*find, bytes)),
        Found::Broken(Some(at)) => Err(Rpf9ScanError::TailNotDeflate { offset, at }),
        Found::Broken(None) => Err(Rpf9ScanError::ChunkFailsPartway { offset }),
        Found::Nothing => {
            let head = reader.get(offset, 16)?;
            Err(if stored_rsc8(head).is_some_and(|_| head[7] == 1) {
                Rpf9ScanError::ConfigOneOutsideContainer { offset }
            } else {
                Rpf9ScanError::NoBodyAt { offset }
            })
        }
    }
}

/// The body at `offset` as a file of its own that [`read_rpf9_body`] at 0 reads again; refused past `limit` bytes.
// A headerless resource is kept as it is stored, every other body as a read returns it.
pub fn extract_rpf9_body<R: Read + Seek>(
    mut source: R,
    offset: u64,
    limit: u64,
) -> Result<(Rpf9ScanFind, Vec<u8>), Rpf9ScanError> {
    let (find, bytes) = read_rpf9_body(&mut source, offset, limit)?;
    let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
        resource: Some(_), ..
    }) = find.kind
    else {
        return Ok((find, bytes));
    };
    // A read refuses a resource whose blocks were not all followed, so its stored length is known.
    let stored = find.stored_len.ok_or(Rpf9ScanError::NoBodyAt { offset })?;
    if stored > limit {
        return Err(Rpf9ScanError::InflateLimit { offset, limit });
    }
    let mut out = Vec::new();
    Reader::new(source)?.copy(offset, stored, &mut out)?;
    // The pad to 16, and a zero slot where a block a block map counts and does not write would start.
    out.resize((stored.next_multiple_of(RPF9_SLOT) + RPF9_SLOT) as usize, 0);
    Ok((find, out))
}

/// Whether a file that starts with `head` holds a resource on its own, as [`extract_rpf9_body`] writes one.
// A stored RSC8 of `rsc8_version`, or a zlib stream whose first 16 inflated bytes state a block map.
pub fn starts_stored_rpf9_resource(head: &[u8], rsc8_version: u8) -> bool {
    if let Some((version, _)) = stored_rsc8(head) {
        return version == rsc8_version;
    }
    if !head.starts_with(&[0x78, 0xDA]) {
        return false;
    }
    let mut first = [0u8; 16];
    let mut z = Decompress::new(true);
    let _ = z.decompress(head, &mut first, FlushDecompress::None);
    z.total_out() == first.len() as u64 && block_map_at(&first).is_some()
}

#[derive(Debug, PartialEq)]
enum Found {
    Body(Box<Rpf9ScanFind>),
    /// A zlib head that fails partway, which a scan counts and does not claim; the point it
    /// stopped where no zero page starts before it, from which its bytes are not deflate data.
    Broken(Option<u64>),
    Nothing,
}

impl Found {
    fn body(find: Rpf9ScanFind) -> Self {
        Self::Body(Box::new(find))
    }
}

/// The file through one window of at most `WINDOW` bytes.
struct Source<R> {
    inner: R,
    len: u64,
    base: u64,
    buf: Vec<u8>,
    /// The bytes a fill reads where no more are asked for at once.
    window: usize,
}

impl<R: Read + Seek> Source<R> {
    fn new(mut inner: R, window: usize) -> io::Result<Self> {
        let len = inner.seek(SeekFrom::End(0))?;
        Ok(Self {
            inner,
            len,
            base: 0,
            buf: Vec::new(),
            window: window.min(WINDOW),
        })
    }

    /// Up to `n` bytes at `at`, fewer at the end of the file.
    fn get(&mut self, at: u64, n: usize) -> io::Result<&[u8]> {
        if at >= self.len {
            return Ok(&[]);
        }
        let want = (at + n.min(WINDOW) as u64).min(self.len);
        if at < self.base || want > self.base + self.buf.len() as u64 {
            self.fill(at, n.min(WINDOW))?;
        }
        let have = want.min(self.base + self.buf.len() as u64);
        Ok(&self.buf[(at - self.base) as usize..(have - self.base) as usize])
    }

    /// Read the window anew from `at`: the bytes a fill reads, or the `need` asked for at once
    /// where that is more.
    fn fill(&mut self, at: u64, need: usize) -> io::Result<()> {
        let size = (self.len - at).min(self.window.max(need) as u64) as usize;
        self.buf.resize(size, 0);
        self.inner.seek(SeekFrom::Start(at))?;
        let mut got = 0;
        while got < size {
            match self.inner.read(&mut self.buf[got..]) {
                Ok(0) => break,
                Ok(k) => got += k,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        self.buf.truncate(got);
        if got < size {
            // The file shrank since it was measured; what is gone is not there to scan.
            self.len = at + got as u64;
        }
        self.base = at;
        Ok(())
    }
}

/// How one zlib chunk ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Chunk {
    Complete { consumed: u64, produced: u64 },
    NotZlib,
    Broken,
    Truncated,
    OverLimit,
}

/// Why a zlib run stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    /// The last chunk inflated to less than a whole number of 64 KiB.
    Short,
    /// The PSO section walk ended.
    Walked,
    /// After a whole number of 64 KiB, nothing a run continues with.
    NotAChunk,
    Broken(u64),
    Truncated,
    OverLimit,
}

/// The section frames of a PSO document: a 4-letter ident and a big-endian length that counts the
/// frame's own 8 bytes, ending with `CHKS`.
#[derive(Debug, Clone, Copy, Default)]
struct PsoWalk {
    at: u64,
    next_frame: u64,
    header: [u8; 8],
    filled: usize,
    last: bool,
    end: Option<u64>,
    bad: bool,
}

impl PsoWalk {
    /// Takes `bytes` and returns how many of them came before the walk ended.
    fn feed(&mut self, bytes: &[u8]) -> usize {
        let mut i = 0;
        while i < bytes.len() && self.end.is_none() && !self.bad {
            if self.at < self.next_frame {
                let skip = (self.next_frame - self.at).min((bytes.len() - i) as u64);
                self.at += skip;
                i += skip as usize;
            } else {
                self.header[self.filled] = bytes[i];
                self.filled += 1;
                self.at += 1;
                i += 1;
                if self.filled < 8 {
                    continue;
                }
                self.filled = 0;
                let len = u64::from(BE::read_u32(&self.header[4..]));
                let start = self.at - 8;
                if !self.header[..4].iter().all(u8::is_ascii_uppercase)
                    || len < 8
                    || start + len > CHUNK_LIMIT
                {
                    self.bad = true;
                    break;
                }
                self.next_frame = start + len;
                self.last = &self.header[..4] == b"CHKS";
            }
            if self.last && self.at == self.next_frame {
                self.end = Some(self.at);
            }
        }
        i
    }
}

/// Where a run's inflated bytes go: the head that classifies it, the PSO walk, the block map a
/// resource's first 16 bytes point at, and the caller.
struct Sink<'a> {
    head: Vec<u8>,
    walk: PsoWalk,
    out: Option<&'a mut Vec<u8>>,
    limit: u64,
    over: bool,
    /// Bytes taken so far.
    seen: u64,
    /// Where the block map lies, once 16 bytes have come; `NO_MAP` when they state none.
    map_at: Option<u64>,
    map: [u8; 16],
    /// A resource's pointer words with a 16-aligned target, and where each lies: a virtual one
    /// only where it points past itself, since its section holds its own words.
    pointers: Vec<(u64, u64)>,
    /// How many `pointers` held when they were last made distinct.
    distinct: usize,
    /// The bytes of a word a take ended inside.
    part: [u8; 8],
    /// Where the physical blocks' bytes start, whose words are not kept.
    virtual_end: u64,
    /// Where a `0DRP` resource's class records lie and their bytes, once the head states them.
    classes_at: Option<u64>,
    classes: Vec<u8>,
    /// The first `keep` bytes taken, for a scan that keeps them.
    kept: Vec<u8>,
    keep: usize,
}

/// A `map_at` for a run whose first 16 bytes are not a resource's.
const NO_MAP: u64 = u64::MAX;

impl<'a> Sink<'a> {
    fn new(out: Option<&'a mut Vec<u8>>, limit: u64) -> Self {
        Self {
            head: Vec::new(),
            walk: PsoWalk::default(),
            out,
            limit,
            over: false,
            seen: 0,
            map_at: None,
            map: [0; 16],
            pointers: Vec::new(),
            distinct: 0,
            part: [0; 8],
            virtual_end: u64::MAX,
            classes_at: None,
            classes: Vec::new(),
            kept: Vec::new(),
            keep: 0,
        }
    }
}

impl Sink<'_> {
    fn take(&mut self, bytes: &[u8]) {
        let room = (BOM.len() + TEXT_PROBE).saturating_sub(self.head.len());
        self.head.extend_from_slice(&bytes[..room.min(bytes.len())]);
        let start = self.seen;
        self.seen += bytes.len() as u64;
        if self.map_at.is_none() && self.head.len() >= 16 {
            self.map_at = Some(block_map_at(&self.head).unwrap_or(NO_MAP));
        }
        if self.map_at != Some(NO_MAP) && start < self.virtual_end {
            self.keep_pointers(start, bytes);
        }
        if let Some(map) = self.map_at.filter(|at| *at != NO_MAP) {
            let (from, to) = (map.max(start), (map + 16).min(self.seen));
            if from < to {
                self.map[(from - map) as usize..(to - map) as usize]
                    .copy_from_slice(&bytes[(from - start) as usize..(to - start) as usize]);
            }
        }
        self.keep_classes(start, bytes);
        let room = self.keep.saturating_sub(self.kept.len());
        self.kept.extend_from_slice(&bytes[..room.min(bytes.len())]);
        self.walk.feed(bytes);
        if let Some(out) = self.out.as_deref_mut() {
            if out.len() as u64 + bytes.len() as u64 > self.limit {
                self.over = true;
            } else {
                out.extend_from_slice(bytes);
            }
        }
    }

    /// The bytes of a `0DRP` resource's first class records that `bytes`, `start` bytes into
    /// the run, holds.
    fn keep_classes(&mut self, start: u64, bytes: &[u8]) {
        const RECORDS: usize = 8;
        if self.classes_at.is_none() && self.head.len() >= 0x50 {
            let head = &self.head;
            let pointer = LE::read_u64(&head[0x28..]);
            let count = usize::from(LE::read_u16(&head[0x48..])).min(RECORDS);
            let rbm = head[0x10..0x14] == *b"0DRP" && pointer >> 28 == 5;
            self.classes_at = Some(if rbm { pointer & 0x0FFF_FFFF } else { NO_MAP });
            self.classes = vec![0; if rbm { 32 * count } else { 0 }];
        }
        let Some(at) = self.classes_at.filter(|at| *at != NO_MAP) else {
            return;
        };
        let (from, to) = (
            at.max(start),
            (at + self.classes.len() as u64).min(self.seen),
        );
        if from < to {
            self.classes[(from - at) as usize..(to - at) as usize]
                .copy_from_slice(&bytes[(from - start) as usize..(to - start) as usize]);
        }
    }

    /// The pointer words of `bytes`, which start `start` bytes into the run, kept.
    fn keep_pointers(&mut self, start: u64, bytes: &[u8]) {
        let lead = ((8 - start % 8) % 8) as usize;
        let (head, body) = bytes.split_at(lead.min(bytes.len()));
        let filled = (start % 8) as usize;
        self.part[filled..filled + head.len()].copy_from_slice(head);
        if !head.is_empty() && filled + head.len() == 8 {
            self.keep_word(start + lead as u64 - 8, u64::from_le_bytes(self.part));
        }
        let at = start + head.len() as u64;
        let (words, rest) = body.as_chunks::<8>();
        for (i, word) in words.iter().enumerate() {
            let word = u64::from_le_bytes(*word);
            // Nearly every word fails this: a u32 whose top nibble is 5 or 6, 16-aligned.
            if word & 0xFFFF_FFFF_0000_000F == 0 && matches!(word >> 28, 5 | 6) {
                self.keep_word(at + 8 * i as u64, word);
            }
        }
        self.part[..rest.len()].copy_from_slice(rest);
        // ponytail: one entry per distinct forward target; a section of forward pointers to
        // millions of distinct 16-byte slots holds them all, a bitset if that is ever met.
        if self.pointers.len() >= 2 * self.distinct.max(4096) {
            self.pointers.sort_unstable();
            self.pointers.dedup_by_key(|(word, _)| *word);
            self.distinct = self.pointers.len();
        }
    }

    fn keep_word(&mut self, at: u64, word: u64) {
        let target = word & 0x0FFF_FFFF;
        if at < self.virtual_end
            && target.is_multiple_of(16)
            && match word >> 28 {
                5 => target > at,
                6 => true,
                _ => false,
            }
        {
            self.pointers.push((word, at));
        }
    }

    /// Distinct 16-aligned targets of the pointer words of the first `section` bytes whose top
    /// nibble is `base` and that lie from `len` to 1 MiB past it: a missing block leaves several,
    /// a value one.
    fn targets_past(&self, section: u64, base: u64, len: u64) -> usize {
        let mut targets: Vec<u64> = (self.pointers.iter())
            .filter(|(word, at)| word >> 28 == base && *at < section)
            .map(|(word, _)| word & 0x0FFF_FFFF)
            .filter(|t| (len..len + (1 << 20)).contains(t))
            .collect();
        targets.sort_unstable();
        targets.dedup();
        targets.len()
    }

    fn mark(&self) -> (PsoWalk, usize, bool, usize) {
        (
            self.walk,
            self.out.as_ref().map_or(0, |o| o.len()),
            self.over,
            self.kept.len(),
        )
    }

    fn rewind(&mut self, (walk, len, over, kept): (PsoWalk, usize, bool, usize)) {
        self.walk = walk;
        if let Some(out) = self.out.as_deref_mut() {
            out.truncate(len);
        }
        self.over = over;
        // Bytes dropped past `keep` were never kept, so a mark below it is the stream's own length.
        self.kept.truncate(kept);
    }
}

struct Reader<R> {
    src: Source<R>,
    z: Decompress,
    out: Vec<u8>,
    /// Bodies the last container found states inside itself.
    nested: Vec<Rpf9ScanFind>,
    /// How many inflated bytes of each zlib run and stored resource are kept; 0 keeps none.
    keep: usize,
    /// The bytes kept of what `find_at` last looked at, at `kept_at`.
    kept: Vec<u8>,
    kept_at: u64,
    /// How the file's bodies are stored, which the header at its start states by its platform.
    strategy: ScanStrategy,
}

impl<R: Read + Seek> Reader<R> {
    fn new(source: R) -> Result<Self, Rpf9ScanError> {
        Self::windowed(source, WINDOW)
    }

    /// A reader whose fills read `window` bytes of the file, or what is asked for at once.
    fn windowed(mut source: R, window: usize) -> Result<Self, Rpf9ScanError> {
        let strategy = stored::strategy_of(&mut source)
            .map_err(|source| Rpf9ScanError::Io { offset: 0, source })?;
        Ok(Self {
            src: Source::new(source, window)
                .map_err(|source| Rpf9ScanError::Io { offset: 0, source })?,
            z: Decompress::new(true),
            out: vec![0; 32 << 10],
            nested: Vec::new(),
            keep: 0,
            kept: Vec::new(),
            kept_at: u64::MAX,
            strategy,
        })
    }

    /// The `len` bytes at `at` appended to `out`, window by window.
    fn copy(&mut self, at: u64, len: u64, out: &mut Vec<u8>) -> Result<(), Rpf9ScanError> {
        out.reserve_exact(len as usize);
        let mut done = 0;
        while done < len {
            let want = (len - done).min(WINDOW as u64) as usize;
            let piece = self.get(at + done, want)?;
            if piece.is_empty() {
                return Err(Rpf9ScanError::Truncated {
                    offset: at,
                    file_len: self.src.len,
                });
            }
            done += piece.len() as u64;
            out.extend_from_slice(piece);
        }
        Ok(())
    }

    fn get(&mut self, at: u64, n: usize) -> Result<&[u8], Rpf9ScanError> {
        self.src
            .get(at, n)
            .map_err(|source| Rpf9ScanError::Io { offset: at, source })
    }

    /// The bytes the file holds from `at` on.
    // ⚠ A read that comes back short moves the file's end, even to before a body already found.
    fn left(&self, at: u64) -> Result<u64, Rpf9ScanError> {
        let file_len = self.src.len;
        (file_len.checked_sub(at)).ok_or(Rpf9ScanError::Truncated {
            offset: at,
            file_len,
        })
    }

    /// What starts at `at`; with `out`, its bytes too, refused past `limit`.
    fn find_at(
        &mut self,
        at: u64,
        out: Option<&mut Vec<u8>>,
        limit: u64,
    ) -> Result<Found, Rpf9ScanError> {
        self.kept.clear();
        self.kept_at = at;
        let mut head = [0u8; RPF9_HEADER_SIZE];
        let got = self.get(at, RPF9_HEADER_SIZE)?;
        let head = {
            head[..got.len()].copy_from_slice(got);
            &head[..got.len()]
        };
        #[cfg(feature = "awc")]
        if head.len() >= 4 && LE::read_u32(head) == AWC_MAGIC_LE {
            return self.awc(at, out, limit);
        }
        if let Some(head_layouts) = sgd5_head(at, head) {
            return self.sgd5(at, head_layouts, out, limit);
        }
        let stated = if head.len() >= 4 && LE::read_u32(head) == RPF9_MAGIC {
            match archive_head(head) {
                Some(header) => {
                    if out.is_some() {
                        return Err(Rpf9ScanError::ExtentNotKnown { offset: at });
                    }
                    let span = u64::from(header.toc_end).min(self.src.len - at);
                    return Ok(Found::body(Rpf9ScanFind {
                        offset: at,
                        kind: Rpf9BodyKind::Archive(header),
                        span,
                        stored_len: None,
                        inflated_len: None,
                        container: None,
                    }));
                }
                None => None,
            }
        } else if let Some((version, extent)) =
            // Config byte 1 outside an audio container is refused, but at 0, where a file of its
            // own holds what a save of such a body writes.
            stored_rsc8(head).filter(|_| head[7] == 0 || at == 0)
        {
            Some((Rpf9BodyKind::StoredRsc8 { version, extent }, extent))
        } else if let Some((revision, length)) = bink2(head) {
            Some((Rpf9BodyKind::Bink2 { revision, length }, length))
        } else if self.strategy == ScanStrategy::StoredBodies {
            return self.stored_body(at, out, limit);
        } else if head.starts_with(&[0x78, 0xDA]) {
            return self.zlib_run(at, out, limit);
        } else {
            None
        };
        let Some((kind, stated)) = stated else {
            return Ok(Found::Nothing);
        };
        let file_len = self.src.len;
        if let (true, Rpf9BodyKind::StoredRsc8 { .. }) = (self.keep > 0, kind) {
            // At most one window of it, which `get` returns.
            let prefix = stated.min(self.keep as u64) as usize;
            let bytes = self.src.get(at, prefix);
            self.kept = bytes
                .map_err(|source| Rpf9ScanError::Io { offset: at, source })?
                .to_vec();
        }
        if let Some(out) = out {
            if at + stated > file_len {
                return Err(Rpf9ScanError::LengthPastEnd {
                    offset: at,
                    stated,
                    file_len,
                });
            }
            if stated > limit {
                return Err(Rpf9ScanError::InflateLimit { offset: at, limit });
            }
            self.copy(at, stated, out)?;
        }
        Ok(Found::body(Rpf9ScanFind {
            offset: at,
            kind,
            span: stated.next_multiple_of(RPF9_SLOT).min(file_len - at),
            stored_len: Some(stated),
            inflated_len: Some(stated),
            container: None,
        }))
    }

    /// The `SGD5` container at `at`, to the next page that starts `SGD5`, an RPF9 header or a zlib
    /// entry after 16 zero bytes, or to the end of the file.
    fn sgd5(
        &mut self,
        at: u64,
        head_layouts: u32,
        out: Option<&mut Vec<u8>>,
        limit: u64,
    ) -> Result<Found, Rpf9ScanError> {
        let mut end = at + PAGE;
        while end < self.src.len {
            let probe = self.get(end - 16, 20)?;
            let (zeros, page) = probe.split_at(probe.len().min(16));
            if page.starts_with(b"SGD5")
                || page.starts_with(&RPF9_MAGIC.to_le_bytes())
                || (page.starts_with(&[0x78, 0xDA]) && zeros.iter().all(|b| *b == 0))
            {
                break;
            }
            end += PAGE;
        }
        let span = end.min(self.src.len) - at;
        if let Some(out) = out {
            if span > limit {
                return Err(Rpf9ScanError::InflateLimit { offset: at, limit });
            }
            self.copy(at, span, out)?;
        }
        Ok(Found::body(Rpf9ScanFind {
            offset: at,
            kind: Rpf9BodyKind::Sgd5 { head_layouts },
            span,
            stored_len: None,
            inflated_len: None,
            container: None,
        }))
    }

    /// Where the furthest chunk of the `ADAT` container at `at` ends, as its tables state it in
    /// the layout its flags select; `None` for tables cut short, past the window or past the
    /// data offset.
    #[cfg(feature = "awc")]
    fn awc_extent(&mut self, at: u64) -> Result<Option<u64>, Rpf9ScanError> {
        const HEAD: usize = 16;
        let head = self.get(at, HEAD)?;
        if head.len() < HEAD {
            return Ok(None);
        }
        let flags = LE::read_u16(&head[6..]);
        let streams = LE::read_u32(&head[8..]) as usize;
        let data_offset = LE::read_u32(&head[12..]) as usize;
        if streams > WINDOW {
            return Ok(None);
        }
        let stream_table = stream_table_offset(flags, streams);
        let chunk_table = stream_table + streams * 4;
        if chunk_table > WINDOW {
            return Ok(None);
        }
        let heads = self.get(at, chunk_table)?;
        if heads.len() < chunk_table {
            return Ok(None);
        }
        let chunks: usize = heads[stream_table..]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|w| (u32::from_le_bytes(*w) >> 29) as usize)
            .sum();
        let tables_end = chunk_table + chunks * 8;
        // ⚠ The reader refuses tables that end past the data offset, and only after the whole
        // container is copied: a head on each page would copy the file once per page.
        if tables_end > WINDOW || tables_end > data_offset {
            return Ok(None);
        }
        let tables = self.get(at, tables_end)?;
        if tables.len() < tables_end {
            return Ok(None);
        }
        let furthest = tables[chunk_table..]
            .as_chunks::<8>()
            .0
            .iter()
            .map(|w| {
                let raw = u64::from_le_bytes(*w);
                (raw & 0x0FFF_FFFF) + ((raw >> 28) & 0x0FFF_FFFF)
            })
            .max()
            .unwrap_or(0);
        Ok((furthest > 0).then_some(furthest.max(tables_end as u64)))
    }

    /// The `ADAT` container at `at`, listed by the tree's AWC reader with no key.
    #[cfg(feature = "awc")]
    fn awc(
        &mut self,
        at: u64,
        out: Option<&mut Vec<u8>>,
        limit: u64,
    ) -> Result<Found, Rpf9ScanError> {
        // Measured: a container starts a 4 KiB page, or any slot where bodies are stored as they
        // are, and its table's end, below 256 MiB, is its end.
        if self.strategy == ScanStrategy::ZlibHeads && !at.is_multiple_of(AWC_PAGE) {
            return Ok(Found::Nothing);
        }
        let Some(extent) = self.awc_extent(at)? else {
            return Ok(Found::Nothing);
        };
        if extent >= AWC_EXTENT_LIMIT {
            return Ok(Found::Nothing);
        }
        let file_len = self.src.len;
        if at + extent > file_len {
            return match out {
                Some(_) => Err(Rpf9ScanError::LengthPastEnd {
                    offset: at,
                    stated: extent,
                    file_len,
                }),
                None => Ok(Found::Nothing),
            };
        }
        if out.is_some() && extent > limit {
            return Err(Rpf9ScanError::InflateLimit { offset: at, limit });
        }
        // ponytail: the parse copies the container and each payload, so a scan holds about three
        // times the largest container (329 MiB peak on a 28.5 GB archive); walk the table alone
        // if that matters.
        let mut bytes = Vec::new();
        self.copy(at, extent, &mut bytes)?;
        let Ok(file) = AwcFile::parse(&bytes, AwcKey::default()) else {
            return Ok(Found::Nothing);
        };
        let chunks: Vec<_> = file.streams.iter().flat_map(|s| &s.chunks).collect();
        // A misplaced stream table parses to streams with no chunk.
        if chunks.is_empty() {
            return Ok(Found::Nothing);
        }
        // Config byte 1 is measured only in whole containers of flags 0xFF61 with one animation
        // chunk per stream, whose GTA VI tables the parse holds to end at the data offset.
        let config_1 = file.flags == 0xFF61
            && (file.streams.iter()).all(|s| {
                (s.chunks.iter())
                    .filter(|c| c.kind == AwcChunkKind::Animation)
                    .count()
                    == 1
            });
        self.nested = chunks
            .iter()
            .filter(|c| c.kind == AwcChunkKind::Animation)
            .filter(|c| c.payload.get(7) == Some(&0) || config_1)
            .filter_map(|c| {
                let (version, extent) = stored_rsc8(&c.payload)?;
                (extent == c.payload.len() as u64).then(|| Rpf9ScanFind {
                    offset: at + u64::from(c.offset),
                    kind: Rpf9BodyKind::StoredRsc8 { version, extent },
                    span: extent,
                    stored_len: Some(extent),
                    inflated_len: Some(extent),
                    container: Some(at),
                })
            })
            .collect();
        self.nested.sort_by_key(|f| f.offset);
        let find = Rpf9ScanFind {
            offset: at,
            kind: Rpf9BodyKind::Awc {
                flags: file.flags,
                streams: file.streams.len() as u32,
                chunks: chunks.len() as u32,
            },
            span: extent.next_multiple_of(RPF9_SLOT).min(file_len - at),
            stored_len: Some(extent),
            inflated_len: Some(extent),
            container: None,
        };
        if let Some(out) = out {
            *out = bytes;
        }
        Ok(Found::body(find))
    }

    /// One zlib stream at `at`, inflated into `sink`.
    fn inflate(&mut self, at: u64, sink: &mut Sink) -> Result<Chunk, Rpf9ScanError> {
        if !self.get(at, 2)?.starts_with(&[0x78, 0xDA]) {
            return Ok(Chunk::NotZlib);
        }
        let Self { src, z, out, .. } = self;
        z.reset(true);
        loop {
            let (consumed, produced) = (z.total_in(), z.total_out());
            let input = src
                .get(at + consumed, INPUT_STEP)
                .map_err(|source| Rpf9ScanError::Io {
                    offset: at + consumed,
                    source,
                })?;
            let eof = input.is_empty();
            let status = z.decompress(input, out, FlushDecompress::None);
            let fresh = (z.total_out() - produced) as usize;
            sink.take(&out[..fresh]);
            match status {
                // ⚠ Before the stream's end: a stream of zeros ends in the step that passes the limit.
                Ok(_) if sink.over || z.total_out() > CHUNK_LIMIT => return Ok(Chunk::OverLimit),
                Ok(Status::StreamEnd) => {
                    return Ok(Chunk::Complete {
                        consumed: z.total_in(),
                        produced: z.total_out(),
                    })
                }
                Ok(_) if fresh == 0 && z.total_in() == consumed => {
                    return Ok(if eof {
                        Chunk::Truncated
                    } else if consumed >= VALIDATED_INPUT {
                        Chunk::Broken
                    } else {
                        Chunk::NotZlib
                    });
                }
                Ok(_) => {}
                Err(_) if z.total_in() >= VALIDATED_INPUT => return Ok(Chunk::Broken),
                Err(_) => return Ok(Chunk::NotZlib),
            }
        }
    }

    fn zlib_run(
        &mut self,
        at: u64,
        out: Option<&mut Vec<u8>>,
        limit: u64,
    ) -> Result<Found, Rpf9ScanError> {
        let mut sink = Sink::new(out, limit);
        sink.keep = self.keep;
        sink.kept = std::mem::take(&mut self.kept);
        let found = self.zlib_run_into(at, &mut sink);
        self.kept = std::mem::take(&mut sink.kept);
        found
    }

    fn zlib_run_into(&mut self, at: u64, sink: &mut Sink) -> Result<Found, Rpf9ScanError> {
        let collecting = sink.out.is_some();
        let limit = sink.limit;
        let refuse_limit = |sink: &Sink| -> Result<(), Rpf9ScanError> {
            if sink.over {
                Err(Rpf9ScanError::InflateLimit { offset: at, limit })
            } else {
                Ok(())
            }
        };
        let (consumed, produced) = match self.inflate(at, sink)? {
            Chunk::Complete { consumed, produced } if produced > 0 => (consumed, produced),
            // A stream a piece not delivered cuts fails, runs to the file's end, or inflates past
            // the chunk limit.
            Chunk::Broken | Chunk::NotZlib | Chunk::OverLimit | Chunk::Truncated
                if !sink.over
                    && self.starts_resource(at, sink)?
                    && self.cut_by_missing_piece(at)? =>
            {
                if collecting {
                    return Err(Rpf9ScanError::NotOnDisk { offset: at, at });
                }
                return Ok(Found::body(unread_start(
                    at,
                    sink,
                    Rpf9Walk::NotOnDisk { at },
                )));
            }
            Chunk::Broken if !sink.over && self.starts_resource(at, sink)? => {
                let stopped = self.stop(at);
                if collecting {
                    return Err(Rpf9ScanError::TailNotDeflate {
                        offset: at,
                        at: stopped,
                    });
                }
                return Ok(Found::body(unread_start(
                    at,
                    sink,
                    Rpf9Walk::TailNotDeflate { at: stopped },
                )));
            }
            Chunk::OverLimit if collecting => {
                return Err(Rpf9ScanError::InflateLimit {
                    offset: at,
                    limit: if sink.over { limit } else { CHUNK_LIMIT },
                })
            }
            Chunk::Truncated if collecting => {
                return Err(Rpf9ScanError::Truncated {
                    offset: at,
                    file_len: self.src.len,
                })
            }
            Chunk::Broken => {
                let stopped = self.stop(at);
                let tail = !self.cut_by_missing_piece(at)?;
                return Ok(Found::Broken(tail.then_some(stopped)));
            }
            _ => return Ok(Found::Nothing),
        };
        refuse_limit(&*sink)?;
        let content = classify(&sink.head, produced);
        let pso = content == Rpf9Content::Pso;
        let mut run = Rpf9ZlibRun {
            content,
            chunks: 1,
            stored_pieces: 0,
            first_chunk_len: consumed,
            first_chunk_inflated: produced,
            inflated: produced,
            signature: None,
            resource: None,
        };
        let resource_shaped =
            matches!(content, Rpf9Content::ResourceShaped | Rpf9Content::ClipRoot);
        let mapped = sink.map_at.is_some_and(|map| map != NO_MAP);
        if resource_shaped && mapped && self.starts_entry(at)? {
            return self.resource(at, sink, run);
        }
        let mut end = at + consumed;
        let mut last = produced;
        let stop = loop {
            if pso && sink.walk.end.is_some() {
                break Stop::Walked;
            }
            if last % PIECE != 0 {
                break Stop::Short;
            }
            let next = end.next_multiple_of(RPF9_SLOT);
            let mark = sink.mark();
            match self.inflate(next, sink)? {
                Chunk::Complete { consumed, produced } if produced > 0 => {
                    run.chunks += 1;
                    run.inflated += produced;
                    end = next + consumed;
                    last = produced;
                }
                Chunk::Broken => break Stop::Broken(next),
                Chunk::Truncated => break Stop::Truncated,
                Chunk::OverLimit => {
                    refuse_limit(&*sink)?;
                    // The scan ends the run before a next stream past the chunk limit, so a
                    // read returns the run the scan lists.
                    sink.rewind(mark);
                    break Stop::OverLimit;
                }
                _ if pso && !sink.walk.bad => {
                    sink.rewind(mark);
                    let piece = self.get(next, PIECE as usize)?;
                    if piece.iter().all(|b| *b == 0) {
                        break Stop::NotAChunk;
                    }
                    let mut walk = sink.walk;
                    let took = walk.feed(piece);
                    if walk.end.is_none() && took < PIECE as usize {
                        break Stop::Truncated;
                    }
                    sink.take(&piece[..took]);
                    run.stored_pieces += 1;
                    run.inflated += took as u64;
                    end = next + took as u64;
                    last = took as u64;
                }
                _ if content == Rpf9Content::AudioData => {
                    sink.rewind(mark);
                    let Some(pieces) = self.stored_pieces(next)? else {
                        break Stop::NotAChunk;
                    };
                    for i in 0..pieces {
                        sink.take(self.get(next + i * PIECE, PIECE as usize)?);
                    }
                    run.stored_pieces += pieces as u32;
                    run.inflated += pieces * PIECE;
                    end = next + pieces * PIECE;
                    last = PIECE;
                }
                // A zlib head right after a whole chunk that fails before the inflater trusts it
                // is still the run's next chunk: nothing starts a body there (scan-recount U1).
                Chunk::NotZlib if self.get(next, 2)? == [0x78, 0xDA] => break Stop::Broken(next),
                _ => {
                    sink.rewind(mark);
                    break Stop::NotAChunk;
                }
            }
            refuse_limit(&*sink)?;
        };
        if collecting {
            match stop {
                Stop::Broken(offset) => return Err(Rpf9ScanError::ChunkFailsPartway { offset }),
                Stop::Truncated => {
                    return Err(Rpf9ScanError::Truncated {
                        offset: at,
                        file_len: self.src.len,
                    })
                }
                _ => {}
            }
        }
        // A movie states the whole run's length, which a run of several chunks reaches only here.
        if run.content == Rpf9Content::Other && run.inflated > produced {
            use crate::gta6::other_bodies::OtherBody;
            if let Some(body @ OtherBody::Gfx) = OtherBody::of(&sink.head, run.inflated) {
                run.content = Rpf9Content::Identified(body);
            }
        }
        let certain = match stop {
            Stop::Walked => sink.walk.end == Some(run.inflated),
            // Measured on every XML document of the complete archives.
            Stop::Short => content == Rpf9Content::Xml,
            _ => false,
        };
        let stored = end - at;
        Ok(Found::body(Rpf9ScanFind {
            offset: at,
            kind: Rpf9BodyKind::Zlib(run),
            span: stored.next_multiple_of(RPF9_SLOT).min(self.left(at)?),
            stored_len: certain.then_some(stored),
            inflated_len: certain.then_some(run.inflated),
            container: None,
        }))
    }

    /// Whether the stream at `at` that failed is a resource's first block: its first 16 bytes
    /// state a block map, and an entry may start at `at`.
    fn starts_resource(&mut self, at: u64, sink: &Sink) -> Result<bool, Rpf9ScanError> {
        Ok(sink.map_at.is_some_and(|map| map != NO_MAP) && self.starts_entry(at)?)
    }

    /// Whether a piece not delivered cuts the stream at `at` that the inflater last stopped on:
    /// a zero page starts at or before where it stopped, since an absent piece reads as zeros.
    fn cut_by_missing_piece(&mut self, at: u64) -> Result<bool, Rpf9ScanError> {
        // A slot past the stop allows for the bytes the inflater reads ahead.
        let stopped = self.stop(at) + RPF9_SLOT;
        Ok(self.zero_page_in(at, stopped + PAGE)?.is_some())
    }

    /// How many 64 KiB pieces stored raw from `at` a zlib stream follows, at most 64; `None` where
    /// a piece is short or all zero, or no stream follows them.
    fn stored_pieces(&mut self, at: u64) -> Result<Option<u64>, Rpf9ScanError> {
        for m in 1..=64 {
            let piece = self.get(at + (m - 1) * PIECE, PIECE as usize)?;
            if piece.len() < PIECE as usize || piece.iter().all(|b| *b == 0) {
                return Ok(None);
            }
            if self.get(at + m * PIECE, 2)? == [0x78, 0xDA] {
                return Ok(Some(m));
            }
        }
        Ok(None)
    }

    /// The byte the inflater stopped on in the stream at `at`: the last it consumed.
    fn stop(&self, at: u64) -> u64 {
        at + self.z.total_in().saturating_sub(1)
    }

    /// Whether an entry may start at `at`: at a page, or after a zero 16-byte slot.
    fn starts_entry(&mut self, at: u64) -> Result<bool, Rpf9ScanError> {
        Ok(at.is_multiple_of(PAGE)
            || (at >= RPF9_SLOT
                && self
                    .get(at - RPF9_SLOT, RPF9_SLOT as usize)?
                    .iter()
                    .all(|b| *b == 0)))
    }

    /// The resource whose first block `run` is, its blocks followed as its block map counts them.
    fn resource(
        &mut self,
        at: u64,
        sink: &mut Sink,
        mut run: Rpf9ZlibRun,
    ) -> Result<Found, Rpf9ScanError> {
        let mut blocks = vec![Rpf9Block {
            offset: at,
            stored_len: run.first_chunk_len,
            inflated_len: run.first_chunk_inflated,
            zlib: true,
            physical: false,
        }];
        let mut counts = (0, 0);
        let mut walk = self.walk(sink, &mut blocks, &mut counts)?;
        if sink.over {
            return Err(Rpf9ScanError::InflateLimit {
                offset: at,
                limit: sink.limit,
            });
        }
        let section_len = |physical: bool| -> u64 {
            (blocks.iter())
                .filter(|b| b.physical == physical)
                .map(|b| b.inflated_len)
                .sum()
        };
        let (virtual_len, physical_len) = (section_len(false), section_len(true));
        // ⚠ A navigation mesh's adjacency records and vertices hold words that read as virtual
        // pointers past its section; its reader bounds every pointer it follows.
        let navmesh = Rpf9Signature::of(&sink.head, virtual_len).and_then(Rpf9Signature::role)
            == Some(Rpf9Role::NavMesh);
        if walk == Rpf9Walk::Whole {
            for (base, section, len) in [(5, "virtual", virtual_len), (6, "physical", physical_len)]
            {
                let targets = sink.targets_past(virtual_len, base, len);
                if targets >= 2 && (base == 5 || counts.1 > 0) && !(navmesh && base == 5) {
                    walk = Rpf9Walk::PointersPast { section, targets };
                    break;
                }
            }
        }
        if sink.out.is_some() {
            let offset = at;
            match walk {
                Rpf9Walk::Whole | Rpf9Walk::PointersPast { .. } => {}
                Rpf9Walk::NotOnDisk { at } => return Err(Rpf9ScanError::NotOnDisk { offset, at }),
                Rpf9Walk::TailNotDeflate { at } => {
                    return Err(Rpf9ScanError::TailNotDeflate { offset, at })
                }
                Rpf9Walk::Refused { at, why } => {
                    return Err(Rpf9ScanError::ResourceRefused { offset, at, why })
                }
            }
        }
        if run.content == Rpf9Content::ResourceShaped && sink.head.get(0x10..0x14) != Some(b"0DRP")
        {
            run.signature = Rpf9Signature::of(&sink.head, virtual_len);
        }
        if run.content == Rpf9Content::ResourceShaped
            && crate::gta6::text::declares_map(&sink.classes)
        {
            run.content = Rpf9Content::TextTable;
        }
        run.chunks = blocks.iter().filter(|b| b.zlib).count() as u32;
        run.stored_pieces = blocks.iter().filter(|b| !b.zlib).count() as u32;
        run.inflated = virtual_len + physical_len;
        let mut found = [None; 4];
        for (slot, block) in found.iter_mut().zip(&blocks) {
            *slot = Some(*block);
        }
        run.resource = Some(Rpf9Resource {
            virtual_blocks: counts.0,
            physical_blocks: counts.1,
            blocks: found,
            virtual_len,
            physical_len,
            walk,
        });
        let end = blocks.last().map_or(at, |b| b.offset + b.stored_len);
        let whole = matches!(walk, Rpf9Walk::Whole | Rpf9Walk::PointersPast { .. });
        Ok(Found::body(Rpf9ScanFind {
            offset: at,
            kind: Rpf9BodyKind::Zlib(run),
            span: (end.next_multiple_of(RPF9_SLOT) - at).min(self.left(at)?),
            stored_len: whole.then_some(end - at),
            inflated_len: whole.then_some(virtual_len + physical_len),
            container: None,
        }))
    }

    /// Follows a resource's blocks after `blocks[0]`, back to back at 16-byte ends, into `blocks`,
    /// and its block map's counts into `counts`.
    fn walk(
        &mut self,
        sink: &mut Sink,
        blocks: &mut Vec<Rpf9Block>,
        counts: &mut (u8, u8),
    ) -> Result<Rpf9Walk, Rpf9ScanError> {
        let refused = |at: u64, why: &'static str| Ok(Rpf9Walk::Refused { at, why });
        let start = blocks[0].offset;
        let map_at = sink.map_at.unwrap_or(NO_MAP);
        // ⚠ A one-block virtual section with physical blocks after it never goes on in a next
        // stream: a next one that inflates to whole 64 KiB is the physical head.
        let lone = map_at.saturating_add(16) <= sink.seen && sink.map[8] == 1 && sink.map[9] > 0;
        if !lone {
            let last = blocks[0].inflated_len;
            if let Some(walk) = self.go_on(sink, &mut blocks[0], last)? {
                return Ok(walk);
            }
        }
        // The block map may lie in the virtual section's second block.
        while map_at.saturating_add(16) > sink.seen {
            let at = after(blocks);
            if !self.get(at, 2)?.starts_with(&[0x78, 0xDA]) {
                // A head of whole 64 KiB may go on as a block stored raw that holds the map. Its
                // end is sought from the map on, since a zero run before the map looks like one.
                let map_in_file = (map_at.checked_sub(sink.seen))
                    .filter(|gap| *gap < PIECE)
                    .map(|gap| at + gap);
                if let (1, true, Some(map)) = (
                    blocks.len(),
                    blocks[0].inflated_len.is_multiple_of(PIECE),
                    map_in_file,
                ) {
                    if let Some(rest) = self.stored_len(map, false)? {
                        let slots = self
                            .get(map + 8, 2)?
                            .iter()
                            .map(|n| u64::from(*n))
                            .sum::<u64>();
                        let floor = map_at + 16 + 8 * slots;
                        let block = self.stored(sink, at, map - at + rest, false, floor)?;
                        blocks.push(block);
                        continue;
                    }
                }
                return refused(
                    at,
                    "the block map lies past the zlib blocks of the virtual section",
                );
            }
            match self.zlib_block(at, sink, false)? {
                Block::Read(block) => blocks.push(block),
                Block::Stop(walk) => return Ok(walk),
            }
            if blocks.len() > 2 {
                return refused(
                    start,
                    "the block map lies past the two blocks a virtual section has",
                );
            }
        }
        let map = sink.map;
        let (v, p) = (map[8], map[9]);
        if map[..8].iter().chain(&map[10..]).any(|b| *b != 0) || !(1..=2).contains(&v) || p > 2 {
            return refused(
                start + map_at,
                "the block map is not zero beside its two counts, or counts no virtual block, or more than two in a section",
            );
        }
        *counts = (v, p);
        let (v, p) = (usize::from(v), usize::from(p));
        if blocks.len() > v {
            return refused(
                start,
                "the block map lies past the virtual blocks it counts",
            );
        }
        let whole = |block: &Rpf9Block| block.inflated_len.is_multiple_of(PIECE);
        if blocks.len() < v {
            if !whole(&blocks[0]) {
                return refused(
                    start,
                    "a block before its section's last does not hold a whole number of 64 KiB",
                );
            }
            let floor = map_at + 16 + 8 * (v + p) as u64;
            match self.tail(sink, after(blocks), false, p > 0, floor)? {
                Some(Block::Read(block)) => blocks.push(block),
                Some(Block::Stop(walk)) => return Ok(walk),
                None => {}
            }
        }
        let physical = |blocks: &[Rpf9Block]| blocks.iter().filter(|b| b.physical).count();
        sink.virtual_end = (blocks.iter())
            .filter(|b| !b.physical)
            .map(|b| b.inflated_len)
            .sum();
        if p > 0 && physical(blocks) == 0 {
            let at = after(blocks);
            let head = self.get(at, 16)?.to_vec();
            let block = if head.starts_with(&[0x78, 0xDA]) {
                Some(self.zlib_block(at, sink, true)?)
            } else if head.iter().all(|b| *b == 0) {
                if head.len() == 16 && self.zero_page_in(at, at + 2 * PAGE + RPF9_SLOT)?.is_some() {
                    return Ok(Rpf9Walk::NotOnDisk { at });
                }
                // A virtual section of whole 64 KiB counts a tail it does not write, so the block
                // read as its tail is the physical one.
                let head_whole = whole(&blocks[0]);
                match blocks.get_mut(1) {
                    Some(tail) if p == 1 && v == 2 && head_whole => tail.physical = true,
                    _ => {
                        return refused(
                            at,
                            "zero bytes or the file's end stand where a block the map counts would start",
                        )
                    }
                }
                None
            } else if p == 2 {
                return refused(
                    at,
                    "a block before its section's last is stored raw, as no measured resource has one",
                );
            } else {
                Some(self.stored_block(sink, at, true, false, 0)?)
            };
            match block {
                Some(Block::Read(block)) => blocks.push(block),
                Some(Block::Stop(walk)) => return Ok(walk),
                None => {}
            }
        }
        if p == 2 && physical(blocks) == 1 {
            if !blocks.last().is_some_and(whole) {
                return refused(
                    after(blocks),
                    "a block before its section's last does not hold a whole number of 64 KiB",
                );
            }
            match self.tail(sink, after(blocks), true, false, 0)? {
                Some(Block::Read(block)) => blocks.push(block),
                Some(Block::Stop(walk)) => return Ok(walk),
                None => {}
            }
        }
        // Every block but its section's last holds a whole number of 64 KiB.
        for pair in blocks.windows(2) {
            if pair[0].physical == pair[1].physical && !whole(&pair[0]) {
                return refused(
                    pair[0].offset,
                    "a block before its section's last does not hold a whole number of 64 KiB",
                );
            }
        }
        Ok(Rpf9Walk::Whole)
    }

    /// The block after a section's head at `at`: a zlib or stored tail, or none where zero bytes
    /// or the file's end stand, since a section of whole 64 KiB may count a tail it does not write.
    /// A virtual tail's stream of whole 64 KiB, with physical blocks to come (`more`), is the
    /// physical head, as a section with no tail goes on in the next. `floor` is as `stored` takes.
    fn tail(
        &mut self,
        sink: &mut Sink,
        at: u64,
        physical: bool,
        more: bool,
        floor: u64,
    ) -> Result<Option<Block>, Rpf9ScanError> {
        let head = self.get(at, 16)?.to_vec();
        if head.len() < 16 || head.iter().all(|b| *b == 0) {
            return Ok(None);
        }
        if !head.starts_with(&[0x78, 0xDA]) {
            return self.stored_block(sink, at, physical, more, floor).map(Some);
        }
        let mut block = match self.zlib_block(at, sink, physical)? {
            Block::Read(block) => block,
            stop => return Ok(Some(stop)),
        };
        if block.inflated_len.is_multiple_of(PIECE) {
            if physical || !more {
                return Ok(Some(Block::Stop(Rpf9Walk::Refused {
                    at,
                    why: "a stream after its section's head inflates to a whole number of 64 KiB, as no tail does",
                })));
            }
            block.physical = true;
        }
        Ok(Some(Block::Read(block)))
    }

    /// The block stored raw at `at`, which ends before a zlib block where `more` follow.
    fn stored_block(
        &mut self,
        sink: &mut Sink,
        at: u64,
        physical: bool,
        more: bool,
        floor: u64,
    ) -> Result<Block, Rpf9ScanError> {
        Ok(match self.stored_len(at, more)? {
            Some(len) => Block::Read(self.stored(sink, at, len, physical, floor)?),
            None => Block::Stop(Rpf9Walk::Refused {
                at,
                why: "a block stored raw has no end within 64 KiB: no zero pad before a next item",
            }),
        })
    }

    /// The block stored raw at `at`, `len` bytes in the file, taken into `sink`. Its trailing zero
    /// bytes are not in the file, so it holds a multiple of 16, and a virtual one at least the
    /// bytes to `floor`, the end of the block map's slots.
    fn stored(
        &mut self,
        sink: &mut Sink,
        at: u64,
        len: u64,
        physical: bool,
        floor: u64,
    ) -> Result<Rpf9Block, Rpf9ScanError> {
        // `floor` lies at most the map's 48 bytes past the bytes seen, or in this block.
        let inflated = (len.next_multiple_of(RPF9_SLOT)).max(floor.saturating_sub(sink.seen));
        let mut bytes = self.get(at, len as usize)?.to_vec();
        bytes.resize(inflated as usize, 0);
        sink.take(&bytes);
        Ok(Rpf9Block {
            offset: at,
            stored_len: len,
            inflated_len: inflated,
            zlib: false,
            physical,
        })
    }

    /// The zlib block at `at`, its streams inflated into `sink`.
    fn zlib_block(
        &mut self,
        at: u64,
        sink: &mut Sink,
        physical: bool,
    ) -> Result<Block, Rpf9ScanError> {
        let mut block = Rpf9Block {
            offset: at,
            stored_len: 0,
            inflated_len: 0,
            zlib: true,
            physical,
        };
        let last = match self.inflate(at, sink)? {
            Chunk::Complete { consumed, produced } => {
                block.stored_len = consumed;
                block.inflated_len = produced;
                produced
            }
            other => return Ok(Block::Stop(self.stopped(at, at, other)?)),
        };
        Ok(match self.go_on(sink, &mut block, last)? {
            Some(walk) => Block::Stop(walk),
            None => Block::Read(block),
        })
    }

    /// `block`, gone on in the streams after it while the last, which inflated to `last`,
    /// reached a cap and the next inflates to a whole number of 64 KiB; `None` once it ends.
    fn go_on(
        &mut self,
        sink: &mut Sink,
        block: &mut Rpf9Block,
        mut last: u64,
    ) -> Result<Option<Rpf9Walk>, Rpf9ScanError> {
        loop {
            let next = (block.offset + block.stored_len).next_multiple_of(RPF9_SLOT);
            if !STREAM_CAPS.contains(&last) || !self.goes_on_at(next)? {
                return Ok(None);
            }
            match self.inflate(next, sink)? {
                Chunk::Complete { consumed, produced } => {
                    block.stored_len = next + consumed - block.offset;
                    block.inflated_len += produced;
                    last = produced;
                }
                other => return Ok(Some(self.stopped(block.offset, next, other)?)),
            }
        }
    }

    /// Whether a zlib stream that inflates to a whole number of 64 KiB starts at `at`; the next
    /// entry starts after a zero slot, never there.
    // ⚠ It inflates twice, here and into the sink, which cannot give back what it took.
    fn goes_on_at(&mut self, at: u64) -> Result<bool, Rpf9ScanError> {
        if !self.get(at, 2)?.starts_with(&[0x78, 0xDA]) {
            return Ok(false);
        }
        Ok(matches!(
            self.inflate(at, &mut Sink::new(None, u64::MAX))?,
            Chunk::Complete { produced, .. } if produced > 0 && produced.is_multiple_of(PIECE)
        ))
    }

    /// How a walk stops at the stream at `next` of the block at `at`, which did not end.
    fn stopped(&mut self, at: u64, next: u64, chunk: Chunk) -> Result<Rpf9Walk, Rpf9ScanError> {
        let refused = |why: &'static str| Rpf9Walk::Refused { at, why };
        Ok(match chunk {
            Chunk::Truncated => refused("the file ends inside a block"),
            _ if self.cut_by_missing_piece(next)? => Rpf9Walk::NotOnDisk { at },
            Chunk::OverLimit => refused("a block inflates past 64 MiB"),
            Chunk::Broken => Rpf9Walk::TailNotDeflate {
                at: self.stop(next),
            },
            Chunk::NotZlib | Chunk::Complete { .. } => {
                refused("a block's zlib stream does not inflate")
            }
        })
    }

    /// The length of the block stored raw at `at`: up to the zero bytes before a zlib block
    /// where `to_head`, else up to the zero pad that reaches a page, a page plus one slot, or a
    /// zlib stream after its slot. `None` where no such end lies within 64 KiB.
    fn stored_len(&mut self, at: u64, to_head: bool) -> Result<Option<u64>, Rpf9ScanError> {
        let buf = self.get(at, PIECE as usize + RPF9_SLOT as usize)?.to_vec();
        let zero = |k: usize| {
            buf.get(k..k + 16)
                .is_some_and(|s| s.iter().all(|b| *b == 0))
        };
        let content = |end: usize| {
            buf[..end]
                .iter()
                .rposition(|b| *b != 0)
                .map(|i| i as u64 + 1)
        };
        if to_head {
            for k in (16..buf.len()).step_by(16) {
                if buf[k..].starts_with(&[0x78, 0xDA]) && self.valid_head(at + k as u64)? {
                    return Ok(content(k));
                }
            }
            return Ok(None);
        }
        let (mut k, mut end) = (0, 0);
        while k + 16 <= buf.len() {
            if !zero(k) {
                end = k + 16;
                k += 16;
                continue;
            }
            let mut j = k;
            while zero(j) {
                j += 16;
            }
            let q = at + j as u64;
            if j + 16 > buf.len() || matches!(q % PAGE, 0 | 16) {
                break;
            }
            if self.valid_head(q)? {
                break;
            }
            k = j;
        }
        Ok((end > 0 && end < PIECE as usize)
            .then(|| content(end))
            .flatten())
    }

    /// Whether a zlib stream starts at `at`: its first 4 KiB inflate to 256 bytes or end it.
    fn valid_head(&mut self, at: u64) -> Result<bool, Rpf9ScanError> {
        let Self { src, z, .. } = self;
        let input = src
            .get(at, 4096)
            .map_err(|source| Rpf9ScanError::Io { offset: at, source })?;
        if !input.starts_with(&[0x78, 0xDA]) {
            return Ok(false);
        }
        z.reset(true);
        let mut out = [0u8; 256];
        Ok(match z.decompress(input, &mut out, FlushDecompress::None) {
            Ok(Status::StreamEnd) => true,
            Ok(_) => z.total_out() == out.len() as u64,
            Err(_) => false,
        })
    }

    /// The first 4 KiB page of zero bytes wholly inside `from..to`.
    fn zero_page_in(&mut self, from: u64, to: u64) -> Result<Option<u64>, Rpf9ScanError> {
        let to = to.min(self.src.len);
        let mut page = from.next_multiple_of(PAGE);
        while page + PAGE <= to {
            if self.get(page, PAGE as usize)?.iter().all(|b| *b == 0) {
                return Ok(Some(page));
            }
            page += PAGE;
        }
        Ok(None)
    }

    /// Where a nested archive's names of `size` bytes start after an entry that ends at `at`:
    /// there or, where `next_page`, at the page strictly after the one holding `at`, zero bytes
    /// before them, and the next archive's header at their end or at the page strictly after it.
    fn names_after(
        &mut self,
        at: u64,
        size: u32,
        next_page: bool,
    ) -> Result<Option<u64>, Rpf9ScanError> {
        let size = u64::from(size);
        let paged = (at / PAGE + 1) * PAGE;
        let zero = |bytes: &[u8]| bytes.iter().all(|b| *b == 0);
        for start in std::iter::once(at).chain(next_page.then_some(paged)) {
            if size < RPF9_SLOT || (start > at && !zero(self.get(at, (start - at) as usize)?)) {
                continue;
            }
            let first = self.get(start, 16)?;
            if first.len() < 16 || zero(first) || may_start_body(first) {
                continue;
            }
            let last = self.get(start + size - 16, 16)?;
            if last.len() < 16 || zero(last) {
                continue;
            }
            let end = start + size;
            let after = self.get(end, PAGE as usize + RPF9_HEADER_SIZE)?;
            let pad = (after
                .as_chunks::<16>()
                .0
                .iter()
                .take_while(|s| zero(&s[..]))
                .count()
                * 16) as u64;
            let next = after.get(pad as usize..pad as usize + RPF9_HEADER_SIZE);
            let placed = pad == 0 || end + pad == (end / PAGE + 1) * PAGE;
            if placed && next.and_then(archive_head).is_some() {
                return Ok(Some(start));
            }
        }
        Ok(None)
    }

    /// The first 16 inflated bytes of the zlib stream at `at`, if one starts there.
    fn first_words(&mut self, at: u64) -> Result<Option<[u8; 16]>, Rpf9ScanError> {
        let Self { src, z, .. } = self;
        let input = src
            .get(at, 4096)
            .map_err(|source| Rpf9ScanError::Io { offset: at, source })?;
        if !input.starts_with(&[0x78, 0xDA]) {
            return Ok(None);
        }
        z.reset(true);
        let mut out = [0u8; 16];
        let inflated = z.decompress(input, &mut out, FlushDecompress::None).is_ok();
        Ok((inflated && z.total_out() == 16).then_some(out))
    }

    /// The resource whose blocks include the one at `at`, which starts none: the nearest
    /// resource start within 64 MiB before it, walked, since entries never interleave.
    fn owner_of(&mut self, at: u64) -> Result<Option<Rpf9ScanError>, Rpf9ScanError> {
        let floor = at.saturating_sub(OWNER_REACH);
        let mut hi = at;
        while hi > floor {
            let lo = hi.saturating_sub(WINDOW as u64).max(floor);
            let buf = self.get(lo, (hi - lo) as usize)?.to_vec();
            for k in (0..buf.len()).step_by(RPF9_SLOT as usize).rev() {
                let start = lo + k as u64;
                if !buf[k..].starts_with(&[0x78, 0xDA])
                    || !self.starts_entry(start)?
                    || !self
                        .first_words(start)?
                        .is_some_and(|w| block_map_at(&w).is_some())
                {
                    continue;
                }
                let Found::Body(owner) = self.find_at(start, None, u64::MAX)? else {
                    return Ok(None);
                };
                let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
                    resource: Some(resource),
                    ..
                }) = owner.kind
                else {
                    return Ok(None);
                };
                let blocks: Vec<Rpf9Block> = resource.blocks.iter().flatten().copied().collect();
                let virtual_blocks = blocks.iter().filter(|b| !b.physical).count();
                // A block of several streams holds the later ones too.
                let holds = |b: &Rpf9Block| (b.offset..b.offset + b.stored_len).contains(&at);
                return Ok(blocks.iter().position(holds).map(|i| {
                    let physical = blocks[i].physical;
                    Rpf9ScanError::BlockOfResource {
                        offset: at,
                        owner: start,
                        section: if physical { "physical" } else { "virtual" },
                        block: if physical {
                            i - virtual_blocks + 1
                        } else {
                            i + 1
                        },
                    }
                }));
            }
            hi = lo;
        }
        Ok(None)
    }
}

/// The names of the archive at `archive`, `len` bytes at `offset`.
fn names_find(offset: u64, len: u64, archive: u64) -> Rpf9ScanFind {
    Rpf9ScanFind {
        offset,
        kind: Rpf9BodyKind::Names { archive },
        span: len,
        stored_len: Some(len),
        inflated_len: Some(len),
        container: None,
    }
}

/// A resource whose first block does not read, for the reason `walk` gives: no block read, the
/// slot claimed.
fn unread_start(at: u64, sink: &Sink, walk: Rpf9Walk) -> Rpf9ScanFind {
    let run = Rpf9ZlibRun {
        content: classify(&sink.head, sink.seen),
        chunks: 0,
        stored_pieces: 0,
        first_chunk_len: 0,
        first_chunk_inflated: 0,
        inflated: 0,
        signature: None,
        resource: Some(Rpf9Resource {
            virtual_blocks: 0,
            physical_blocks: 0,
            blocks: [None; 4],
            virtual_len: 0,
            physical_len: 0,
            walk,
        }),
    };
    Rpf9ScanFind {
        offset: at,
        kind: Rpf9BodyKind::Zlib(run),
        span: RPF9_SLOT,
        stored_len: None,
        inflated_len: None,
        container: None,
    }
}

/// How one block of a resource was read: whole, or the walk stopping at it.
enum Block {
    Read(Rpf9Block),
    Stop(Rpf9Walk),
}

/// Where the block after the last of `blocks` starts: at its 16-byte end.
fn after(blocks: &[Rpf9Block]) -> u64 {
    (blocks.last()).map_or(0, |b| (b.offset + b.stored_len).next_multiple_of(RPF9_SLOT))
}

/// The block map a resource's first 16 inflated bytes point at: zero, then a 0x5000_0000 pointer.
fn block_map_at(head: &[u8]) -> Option<u64> {
    let pointer = LE::read_u64(head.get(8..16)?);
    (LE::read_u64(head) == 0 && pointer >> 28 == 5).then_some(pointer & 0x0FFF_FFFF)
}

fn may_start_body(head: &[u8]) -> bool {
    head.starts_with(&[0x78, 0xDA])
        || head.starts_with(b"KB2")
        || head.starts_with(&RPF9_MAGIC.to_le_bytes())
        || head.starts_with(&RSC8_MAGIC.to_le_bytes())
        || (cfg!(feature = "awc") && head.starts_with(b"ADAT"))
        || head.starts_with(b"SGD5")
}

/// The head-layout count of an `SGD5` head at a page start whose first binding layout frames: a
/// size of 24 bytes and 8 per record, the record count stated once and again as class counts.
fn sgd5_head(at: u64, head: &[u8]) -> Option<u32> {
    if !at.is_multiple_of(PAGE) || !head.starts_with(b"SGD5") {
        return None;
    }
    let word = |o: usize| head.get(o..o + 4).map(LE::read_u32);
    let (layouts, size) = (word(0x0C)?, word(0x10)? as usize);
    let records = size.checked_sub(24).filter(|n| n % 8 == 0)? / 8;
    let stated = usize::from(LE::read_u16(head.get(0x1C..0x1E)?));
    let classes: usize = (head.get(0x1E..0x24)?.iter())
        .map(|c| usize::from(*c))
        .sum();
    ((1..=4096).contains(&layouts) && stated == records && classes == records && word(0x28)? == 0)
        .then_some(layouts)
}

/// The RPF9 header at the start of `head` where it holds the model no other bytes measured hold:
/// its table's words in order and 16-aligned, its reserved bytes zero, its index the width and,
/// for index kind 1, the length its count takes. A rule of [`Rpf9Header::parse`] it breaks is
/// named on its row, since its table still bounds what follows.
fn archive_head(head: &[u8]) -> Option<Rpf9Header> {
    let header = Rpf9Header::read(head).ok()?;
    let (raw, index_offset) = (&header.raw, header.index_offset);
    let width = match header.entry_count {
        0..=0xFF => 1,
        0x100..=0xFFFF => 2,
        _ => 3,
    };
    let index_len = u64::from(header.entry_count) * (8 + u64::from(header.index_width));
    ([header.toc_end, index_offset, header.names_size]
        .iter()
        .all(|w| w % 16 == 0)
        && header.toc_end >= 0x60
        && (0x40..header.toc_end).contains(&index_offset)
        && raw[0x34..0x40].iter().all(|b| *b == 0)
        && header.index_width == width
        && (header.index_kind != 1
            || u64::from(header.toc_end - index_offset) == index_len.next_multiple_of(16)))
    .then_some(header)
}

/// The version byte and extent of an RSC8 header in the one form measured: stored, unkeyed, no
/// physical section.
fn stored_rsc8(head: &[u8]) -> Option<(u8, u64)> {
    if head.len() < 16 || LE::read_u32(head) != RSC8_MAGIC {
        return None;
    }
    let version = LE::read_u32(&head[4..]);
    let stored = (version >> 8) & 0x1F == 0x1F;
    let unkeyed = (version >> 16) & 0xFF == 0;
    let physical = LE::read_u32(&head[12..]) & !0xF;
    // The config byte is 0 or 1 in every measured head.
    (stored && unkeyed && physical == 0 && head[7] <= 1).then(|| {
        (
            version as u8,
            16 + u64::from(LE::read_u32(&head[8..]) & !0xF),
        )
    })
}

/// The revision letter and file length of a Bink 2 head that passes the measured header model.
fn bink2(head: &[u8]) -> Option<(u8, u64)> {
    if head.len() < 0x2C || &head[..3] != b"KB2" {
        return None;
    }
    let word = |at: usize| LE::read_u32(&head[at..]);
    let length = u64::from(word(0x04)) + 8;
    let sides = 16..=16_384;
    (head[3].is_ascii_lowercase()
        && (1..=1_000_000).contains(&word(0x08))
        && u64::from(word(0x0C)) <= length
        && sides.contains(&word(0x14))
        && sides.contains(&word(0x18))
        && word(0x1C) > 0
        && word(0x20) > 0
        && length >= 0x2C)
        .then_some((head[3], length))
}

fn classify(head: &[u8], inflated: u64) -> Rpf9Content {
    if head.starts_with(b"PSIN") {
        return Rpf9Content::Pso;
    }
    if head.starts_with(b"PS\x00\x09") {
        return Rpf9Content::PipelineCache;
    }
    if head.starts_with(b"RBF0") {
        return Rpf9Content::Rbf;
    }
    if crate::gta6::audio_data::stamped(head) {
        return Rpf9Content::AudioData;
    }
    if let Some(body) = crate::gta6::other_bodies::OtherBody::of(head, inflated) {
        return Rpf9Content::Identified(body);
    }
    let text = head.strip_prefix(&BOM).unwrap_or(head);
    let probe = &text[..text.len().min(TEXT_PROBE)];
    let share = printable_share(probe);
    if is_text(probe, share) {
        return if opens_xml(text, share) {
            Rpf9Content::Xml
        } else {
            Rpf9Content::Text
        };
    }
    if is_clip_root(head) {
        return Rpf9Content::ClipRoot;
    }
    if inflated >= 0x20 && head.starts_with(&[0; 4]) {
        return Rpf9Content::ResourceShaped;
    }
    Rpf9Content::Other
}

/// The head every headerless animation resource's root has: four zero bytes, zero from 0x10 to
/// 0x38, and its map's flag word at 0x5C.
fn is_clip_root(head: &[u8]) -> bool {
    head.len() >= 0x60
        && head.starts_with(&[0; 4])
        && head[0x10..0x38].iter().all(|b| *b == 0)
        && LE::read_u32(&head[0x5C..]) == 0x0100_0000
}

/// Printable ASCII, tab, line feed, carriage return, and UTF-8 lead bytes.
fn printable_share(probe: &[u8]) -> f64 {
    let printable = probe
        .iter()
        .filter(|&&c| (32..127).contains(&c) || matches!(c, 9 | 10 | 13) || c >= 0xC2)
        .count();
    printable as f64 / probe.len().max(1) as f64
}

fn is_text(probe: &[u8], share: f64) -> bool {
    let utf8 = match std::str::from_utf8(probe) {
        Ok(_) => true,
        // A character cut by the end of the probe.
        Err(e) => e.error_len().is_none(),
    };
    !probe.is_empty()
        && share >= 0.97
        && utf8
        && !probe
            .iter()
            .any(|&c| matches!(c, 0..=8 | 0x0B | 0x0C | 0x0E..=0x1F))
}

fn opens_xml(text: &[u8], share: f64) -> bool {
    let head = &text[..text.len().min(300)];
    let declared = &head[head
        .iter()
        .position(|c| !c.is_ascii_whitespace())
        .unwrap_or(head.len())..];
    if declared.starts_with(b"<?xml") {
        return true;
    }
    // A run that starts mid-document begins with the indentation before an inner element.
    let Some(rest) = head.strip_prefix(b"<") else {
        return false;
    };
    let name = |c: &u8| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b':' | b'-');
    let Some(&first) = rest.first() else {
        return false;
    };
    let after = rest.iter().position(|c| !name(c));
    (first.is_ascii_alphabetic() || first == b'_')
        && after.is_some_and(|i| {
            let c = rest[i];
            c.is_ascii_whitespace() || c == b'>' || c == b'/'
        })
        && share >= 0.98
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::{Cursor, Write};

    mod counted;
    mod raw_streams;
    mod recount;
    mod stored;
    mod stored_names;

    fn deflate(bytes: &[u8]) -> Vec<u8> {
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::best());
        enc.write_all(bytes).expect("deflated");
        let out = enc.finish().expect("finished");
        assert_eq!(out[..2], [0x78, 0xDA], "level 9 writes the measured head");
        out
    }

    /// Bytes no detector starts on: every 16-byte slot begins with a byte no head begins with.
    fn noise(len: usize, seed: u32) -> Vec<u8> {
        let mut x = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
        let mut bytes: Vec<u8> = (0..len)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (x >> 16) as u8
            })
            .collect();
        for slot in bytes.iter_mut().step_by(16) {
            if matches!(*slot, 0x78 | b'9' | b'R' | b'K' | 0) {
                *slot = 0xA5;
            }
        }
        bytes
    }

    /// A table end that leaves room for parts A to C of layout 4 and an index of a few entries.
    const TABLE: u32 = 0x1E0;

    /// An RPF9 head of layout 4 that holds every measured rule, and its table, `toc_end` bytes
    /// in all, with names of `NAMES` bytes.
    fn rpf9_head(entries: u32, toc_end: u32, seed: u32) -> Vec<u8> {
        let index_offset = toc_end - (entries * 9).next_multiple_of(16);
        let mut bytes = noise(toc_end as usize, seed);
        bytes[..RPF9_HEADER_SIZE].fill(0);
        LE::write_u32(&mut bytes[0x00..], RPF9_MAGIC);
        LE::write_u32(&mut bytes[0x04..], toc_end);
        LE::write_u32(&mut bytes[0x08..], index_offset);
        LE::write_u32(&mut bytes[0x0C..], NAMES);
        bytes[0x19] = 0x7A;
        bytes[0x1A] = 4;
        LE::write_u32(&mut bytes[0x1C..], 0x150);
        LE::write_u16(&mut bytes[0x20..], 5);
        LE::write_u16(&mut bytes[0x22..], 1);
        LE::write_u32(&mut bytes[0x24..], entries);
        bytes[0x2C] = 1;
        bytes[0x2D] = 0x0F;
        bytes[0x2E] = 0x09;
        LE::write_u32(&mut bytes[0x30..], 0x10);
        bytes
    }

    const NAMES: u32 = 0x40;

    fn xml(len: usize) -> Vec<u8> {
        let mut text = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Root>\n".to_vec();
        let mut i = 0;
        while text.len() + 8 < len {
            text.extend_from_slice(format!("  <Item value=\"{i}\"/>\n").as_bytes());
            i += 1;
        }
        text.truncate(len - 8);
        text.extend_from_slice(b"</Root>\n");
        text
    }

    /// A PSO document of `len` bytes: `PSIN`, an `LSNS` of noise, and `CHKS` stating the length.
    fn pso(len: usize) -> Vec<u8> {
        let mut body = Vec::new();
        let frame = |body: &mut Vec<u8>, ident: &[u8; 4], content: &[u8]| {
            body.extend_from_slice(ident);
            body.extend_from_slice(&(content.len() as u32 + 8).to_be_bytes());
            body.extend_from_slice(content);
        };
        frame(&mut body, b"PSIN", &[0; 8]);
        let lsns = len - body.len() - 8 - 20;
        frame(&mut body, b"LSNS", &noise(lsns, 7));
        let mut chks = [0u8; 12];
        BE::write_u32(&mut chks, len as u32);
        frame(&mut body, b"CHKS", &chks);
        assert_eq!(body.len(), len);
        body
    }

    /// A file laid out piece by piece, with the finds a scan must report.
    #[derive(Default)]
    struct Layout {
        bytes: Vec<u8>,
        finds: Vec<Rpf9ScanFind>,
        unclaimed: Vec<Range<usize>>,
    }

    impl Layout {
        fn at(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn pad(&mut self) {
            let end = self.bytes.len().next_multiple_of(16);
            self.bytes.resize(end, 0);
        }
        fn unclaimed(&mut self, bytes: &[u8]) {
            let start = self.bytes.len();
            self.bytes.extend_from_slice(bytes);
            self.unclaimed.push(start..self.bytes.len());
        }
        fn claim(&mut self, find: Rpf9ScanFind, bytes: &[u8]) {
            assert_eq!(find.offset, self.at());
            self.bytes.extend_from_slice(bytes);
            self.pad();
            assert_eq!(self.at(), find.offset + find.span);
            self.finds.push(find);
        }
        /// A run of chunks and stored pieces; `stored` names which 64 KiB pieces stay raw.
        fn zlib(&mut self, content: Rpf9Content, body: &[u8], stored: &[usize], certain: bool) {
            let offset = self.at();
            let mut bytes = Vec::new();
            let mut first = (0, 0);
            for (i, piece) in body.chunks(PIECE as usize).enumerate() {
                bytes.resize(bytes.len().next_multiple_of(16), 0);
                if stored.contains(&i) {
                    bytes.extend_from_slice(piece);
                } else {
                    let z = deflate(piece);
                    if i == 0 {
                        first = (z.len() as u64, piece.len() as u64);
                    }
                    bytes.extend_from_slice(&z);
                }
            }
            let pieces = body.len().div_ceil(PIECE as usize);
            let run = Rpf9ZlibRun {
                content,
                chunks: (pieces - stored.len()) as u32,
                stored_pieces: stored.len() as u32,
                first_chunk_len: first.0,
                first_chunk_inflated: first.1,
                inflated: body.len() as u64,
                signature: None,
                resource: None,
            };
            let find = Rpf9ScanFind {
                offset,
                kind: Rpf9BodyKind::Zlib(run),
                span: (bytes.len() as u64).next_multiple_of(16),
                stored_len: certain.then_some(bytes.len() as u64),
                inflated_len: certain.then_some(body.len() as u64),
                container: None,
            };
            self.claim(find, &bytes);
        }
        /// A headerless resource after its zero slot: `blocks` back to back, each its bytes, whether
        /// they are stored raw and whether they are physical; the block map is in the first. A
        /// block stored raw reads zero-extended to 16.
        fn resource(&mut self, blocks: &[(&[u8], bool, bool)], counts: (u8, u8)) {
            self.unclaimed(&[0; 16]);
            let offset = self.at();
            let (mut bytes, mut found) = (Vec::new(), [None; 4]);
            for (slot, (block, stored, physical)) in found.iter_mut().zip(blocks) {
                bytes.resize(bytes.len().next_multiple_of(16), 0);
                let (written, read) = if *stored {
                    (block.to_vec(), block.len().next_multiple_of(16))
                } else {
                    (deflate(block), block.len())
                };
                *slot = Some(Rpf9Block {
                    offset: offset + bytes.len() as u64,
                    stored_len: written.len() as u64,
                    inflated_len: read as u64,
                    zlib: !*stored,
                    physical: *physical,
                });
                bytes.extend(written);
            }
            let section = |physical: bool| -> Vec<u8> {
                (blocks.iter())
                    .filter(|b| b.2 == physical)
                    .flat_map(|b| {
                        let mut read = b.0.to_vec();
                        if b.1 {
                            read.resize(read.len().next_multiple_of(16), 0);
                        }
                        read
                    })
                    .collect()
            };
            let (virtual_section, physical_section) = (section(false), section(true));
            let resource = Rpf9Resource {
                virtual_blocks: counts.0,
                physical_blocks: counts.1,
                blocks: found,
                virtual_len: virtual_section.len() as u64,
                physical_len: physical_section.len() as u64,
                walk: Rpf9Walk::Whole,
            };
            let first = found[0].expect("a first block");
            let inflated = (virtual_section.len() + physical_section.len()) as u64;
            let run = Rpf9ZlibRun {
                content: Rpf9Content::ResourceShaped,
                chunks: blocks.iter().filter(|b| !b.1).count() as u32,
                stored_pieces: blocks.iter().filter(|b| b.1).count() as u32,
                first_chunk_len: first.stored_len,
                first_chunk_inflated: first.inflated_len,
                inflated,
                signature: Rpf9Signature::of(&virtual_section, virtual_section.len() as u64),
                resource: Some(resource),
            };
            let find = Rpf9ScanFind {
                offset,
                kind: Rpf9BodyKind::Zlib(run),
                span: (bytes.len() as u64).next_multiple_of(16),
                stored_len: Some(bytes.len() as u64),
                inflated_len: Some(inflated),
                container: None,
            };
            self.claim(find, &bytes);
        }
        fn archive(&mut self, entries: u32, toc_end: u32, seed: u32) {
            let bytes = rpf9_head(entries, toc_end, seed);
            let header = Rpf9Header::parse(&bytes).expect("a head");
            let find = Rpf9ScanFind {
                offset: self.at(),
                kind: Rpf9BodyKind::Archive(header),
                span: u64::from(toc_end),
                stored_len: None,
                inflated_len: None,
                container: None,
            };
            self.claim(find, &bytes);
        }
        /// Names of `len` bytes, of noise, for the archive whose header is at `archive`.
        fn names(&mut self, len: usize, archive: u64) {
            let find = names_find(self.at(), len as u64, archive);
            self.claim(find, &noise(len, 109));
        }
        fn rsc8(&mut self, virtual_size: u32) {
            let mut bytes = noise(16 + virtual_size as usize, 3);
            bytes[..4].copy_from_slice(&RSC8_MAGIC.to_le_bytes());
            LE::write_u32(&mut bytes[4..], 0x1F41);
            LE::write_u32(&mut bytes[8..], virtual_size | 4);
            LE::write_u32(&mut bytes[12..], 1);
            let extent = 16 + u64::from(virtual_size);
            let find = Rpf9ScanFind {
                offset: self.at(),
                kind: Rpf9BodyKind::StoredRsc8 {
                    version: 0x41,
                    extent,
                },
                span: extent,
                stored_len: Some(extent),
                inflated_len: Some(extent),
                container: None,
            };
            self.claim(find, &bytes);
        }
        fn bink2(&mut self, length: u32) {
            let mut bytes = noise(length as usize, 5);
            bytes[..4].copy_from_slice(b"KB2n");
            for (at, value) in [
                (0x04, length - 8),
                (0x08, 85),
                (0x0C, 4_000),
                (0x10, 85),
                (0x14, 1920),
                (0x18, 1080),
                (0x1C, 30),
                (0x20, 1),
                (0x24, 0x4_0013),
                (0x28, 1),
            ] {
                LE::write_u32(&mut bytes[at..], value);
            }
            let find = Rpf9ScanFind {
                offset: self.at(),
                kind: Rpf9BodyKind::Bink2 {
                    revision: b'n',
                    length: u64::from(length),
                },
                span: u64::from(length).next_multiple_of(16),
                stored_len: Some(u64::from(length)),
                inflated_len: Some(u64::from(length)),
                container: None,
            };
            self.claim(find, &bytes);
        }
        /// Zero bytes to the next 4 KiB page.
        fn page(&mut self) {
            let end = self.bytes.len().next_multiple_of(0x1000);
            let zeros = vec![0; end - self.bytes.len()];
            self.unclaimed(&zeros);
        }
    }

    /// One body of every kind, padding between them, noise, and a nested archive with a body.
    fn every_kind() -> Layout {
        let mut l = Layout::default();
        l.archive(6, TABLE, 1);
        l.zlib(Rpf9Content::Xml, &xml(0x1_0000 + 900), &[], true);
        l.page();
        l.zlib(Rpf9Content::Pso, &pso(0x2_0000 + 300), &[1], true);
        l.zlib(
            Rpf9Content::Text,
            b"a list of names\r\nand more names\r\n",
            &[],
            false,
        );
        let mut resource = vec![0u8; 0x60];
        resource[8..16].copy_from_slice(&0x5000_0040u64.to_le_bytes());
        l.zlib(Rpf9Content::ResourceShaped, &resource, &[], false);
        l.zlib(Rpf9Content::Other, &noise(3000, 11), &[], false);
        l.page();
        l.rsc8(0x2340);
        l.bink2(0x1_2345);
        l.unclaimed(&noise(0x900, 13));
        l.page();
        let virtual_section = virtual_section(0x300, (1, 1));
        l.resource(
            &[
                (&virtual_section, false, false),
                (&[0x22; 0x500], false, true),
            ],
            (1, 1),
        );
        l.page();
        l.archive(2, TABLE, 2);
        l.zlib(Rpf9Content::Other, &noise(0x300, 17), &[], false);
        l.unclaimed(&noise(0x40, 19));
        l
    }

    fn scan_all(bytes: &[u8]) -> (Vec<Rpf9ScanFind>, u64, u64, u64) {
        let mut scan = Rpf9Scan::new(Cursor::new(bytes), 0..u64::MAX).expect("opened");
        let finds = scan
            .by_ref()
            .collect::<Result<Vec<_>, _>>()
            .expect("scanned");
        let (u, z, b) = (
            scan.unclaimed_bytes(),
            scan.unclaimed_zero_bytes(),
            scan.broken_chunks(),
        );
        (finds, u, z, b)
    }

    #[test]
    fn a_file_scans_to_exactly_the_bodies_written() {
        let l = every_kind();
        let (finds, unclaimed, zero, broken) = scan_all(&l.bytes);
        assert_eq!(finds.len(), l.finds.len());
        for (found, written) in finds.iter().zip(&l.finds) {
            assert_eq!(found, written, "at {:#X}", written.offset);
        }
        let claimed: u64 = l.finds.iter().map(|f| f.span).sum();
        assert_eq!(unclaimed, l.bytes.len() as u64 - claimed);
        let zeros = l
            .unclaimed
            .iter()
            .flat_map(|r| &l.bytes[r.clone()])
            .filter(|b| **b == 0)
            .count();
        assert_eq!(zero, zeros as u64);
        assert_eq!(broken, 0);
    }

    #[test]
    fn a_scan_resumed_slice_by_slice_finds_the_same_bodies() {
        let l = every_kind();
        let unclaimed = scan_all(&l.bytes).1;
        assert_eq!(scan_in_slices(&l.bytes, 0x1000), (l.finds, unclaimed));
    }

    /// The finds and the unclaimed bytes of a scan of `bytes` resumed slice by slice.
    fn scan_in_slices(bytes: &[u8], slice: u64) -> (Vec<Rpf9ScanFind>, u64) {
        let (mut finds, mut unclaimed, mut pos) = (Vec::new(), 0, 0);
        let mut carry = Rpf9ScanCarry::default();
        while pos < bytes.len() as u64 {
            let mut scan = Rpf9Scan::new(Cursor::new(bytes), pos..pos + slice)
                .expect("opened")
                .carrying(carry);
            finds.extend(scan.by_ref().map(|f| f.expect("scanned")));
            unclaimed += scan.unclaimed_bytes();
            assert!(scan.position() > pos);
            pos = scan.position();
            carry = scan.carry();
        }
        (finds, unclaimed)
    }

    #[test]
    fn every_found_body_reads_back_inflated() {
        let l = every_kind();
        for find in &l.finds {
            let read = read_rpf9_body(Cursor::new(&l.bytes), find.offset, u64::MAX);
            match find.kind {
                Rpf9BodyKind::Archive(_) => assert!(
                    matches!(read, Err(Rpf9ScanError::ExtentNotKnown { offset }) if offset == find.offset)
                ),
                Rpf9BodyKind::Zlib(run) => {
                    let (again, bytes) = read.expect("read");
                    assert_eq!(&again, find);
                    assert_eq!(bytes.len() as u64, run.inflated);
                }
                Rpf9BodyKind::StoredRsc8 { extent: len, .. }
                | Rpf9BodyKind::Bink2 { length: len, .. } => {
                    let (_, bytes) = read.expect("read");
                    let at = find.offset as usize;
                    assert_eq!(bytes, l.bytes[at..at + len as usize]);
                }
                Rpf9BodyKind::Awc { .. } => unreachable!("every_kind writes no audio container"),
                Rpf9BodyKind::Sgd5 { .. } => unreachable!("every_kind writes no effect container"),
                Rpf9BodyKind::Names { .. } | Rpf9BodyKind::FirstEntry { .. } => {
                    unreachable!("every_kind writes no names followed by a header and no unnamed first entry")
                }
                Rpf9BodyKind::RawDeflate { .. } => {
                    unreachable!("every_kind writes no raw DEFLATE entry")
                }
            }
        }
        let (_, xml_bytes) = read_rpf9_body(Cursor::new(&l.bytes), l.finds[1].offset, u64::MAX)
            .expect("the XML body");
        assert_eq!(xml_bytes, xml(0x1_0000 + 900));
        let (_, pso_bytes) = read_rpf9_body(Cursor::new(&l.bytes), l.finds[2].offset, u64::MAX)
            .expect("the PSO body");
        assert_eq!(pso_bytes, pso(0x2_0000 + 300));
        let noise_at = l.unclaimed.last().expect("names").start as u64;
        assert!(matches!(
            read_rpf9_body(Cursor::new(&l.bytes), noise_at, u64::MAX),
            Err(Rpf9ScanError::NoBodyAt { offset }) if offset == noise_at
        ));
    }

    #[test]
    fn one_reader_reads_every_body_in_turn_as_a_read_of_each_alone_does() {
        let l = every_kind();
        let mut bodies = Rpf9BodyReader::new(Cursor::new(&l.bytes)).expect("a reader");
        // In the file's order and then against it, where the window is read anew before itself.
        for find in l.finds.iter().chain(l.finds.iter().rev()) {
            let alone = read_rpf9_body(Cursor::new(&l.bytes), find.offset, u64::MAX);
            let in_turn = bodies.read(find.offset, u64::MAX);
            match (alone, in_turn) {
                (Ok(alone), Ok(in_turn)) => assert_eq!(alone, in_turn),
                (Err(alone), Err(in_turn)) => assert_eq!(alone.to_string(), in_turn.to_string()),
                (alone, in_turn) => panic!(
                    "{:#X}: read alone {}, in turn {}",
                    find.offset,
                    alone.is_ok(),
                    in_turn.is_ok()
                ),
            }
        }
    }

    #[test]
    fn a_window_smaller_than_what_is_asked_for_at_once_gives_the_same_bytes() {
        let bytes = noise(3 * BODY_WINDOW, 7);
        let mut small = Source::new(Cursor::new(&bytes), 1024).expect("a source");
        let mut whole = Source::new(Cursor::new(&bytes), WINDOW).expect("a source");
        let end = bytes.len() as u64;
        let asked = [
            (0, 16),
            (5, 4096),
            (1000, 2 * BODY_WINDOW),
            (900, 8),
            (end - 10, 64),
            (40, 1024),
        ];
        for (at, n) in asked {
            assert_eq!(
                small.get(at, n).expect("read"),
                whole.get(at, n).expect("read"),
                "{n} bytes at {at}"
            );
        }
        assert_eq!(
            small.get(1000, 2 * BODY_WINDOW).expect("read").len(),
            2 * BODY_WINDOW
        );
        assert!(small.get(end, 1).expect("read").is_empty());
    }

    /// An XML body of two chunks, the second built by `second` from its deflated bytes.
    fn two_chunks(second: impl Fn(Vec<u8>) -> Vec<u8>) -> (Vec<u8>, u64) {
        let text = xml(0x1_0000 + 0x8000);
        let mut bytes = deflate(&text[..PIECE as usize]);
        bytes.resize(bytes.len().next_multiple_of(16), 0);
        let next = bytes.len() as u64;
        // Noise does not shrink, so the chunk holds more than 256 compressed bytes before its end.
        let mut tail = text[PIECE as usize..].to_vec();
        tail.extend(noise(0x4000, 23));
        bytes.extend(second(deflate(&tail)));
        (bytes, next)
    }

    #[test]
    fn a_later_chunk_that_fails_at_its_first_block_is_refused_as_failing_partway() {
        // Block type 3 is reserved: the chunk fails in its first byte after the head.
        let (bytes, next) = two_chunks(|mut z| {
            z[2] = 0xFF;
            z
        });
        assert!(matches!(
            read_rpf9_body(Cursor::new(&bytes), 0, u64::MAX),
            Err(Rpf9ScanError::ChunkFailsPartway { offset }) if offset == next
        ));
    }

    #[test]
    fn a_chunk_that_fails_partway_is_refused_and_ends_the_run_with_its_end_unknown() {
        let (bytes, next) = two_chunks(|mut z| {
            let at = z.len() - 0x200;
            z[at] ^= 0xFF;
            z
        });
        assert!(matches!(
            read_rpf9_body(Cursor::new(&bytes), 0, u64::MAX),
            Err(Rpf9ScanError::ChunkFailsPartway { offset }) if offset == next
        ));
        // Read on its own, the head fails with no zero page before the failure.
        let flipped = bytes.len() as u64 - 0x200;
        assert!(matches!(
            read_rpf9_body(Cursor::new(&bytes), next, u64::MAX),
            Err(Rpf9ScanError::TailNotDeflate { offset, at }) if offset == next && at >= flipped
        ));
        let (finds, _, _, broken) = scan_all(&bytes);
        assert_eq!(finds[0].offset, 0);
        assert_eq!(finds[0].stored_len, None);
        assert!(matches!(finds[0].kind, Rpf9BodyKind::Zlib(run) if run.chunks == 1));
        assert_eq!(broken, 1);
        let mut scan = Rpf9Scan::new(Cursor::new(&bytes), 0..u64::MAX).expect("opened");
        scan.by_ref().for_each(drop);
        assert_eq!(scan.tails_not_deflate(), 1);
        // A stored block whose bytes from a page on read as zeros, as a piece not delivered
        // reads: it fails at its check, past the zero page, and is not a replaced tail.
        let mut cut = vec![0x78, 0xDA, 0x01];
        cut.extend(0x8000u16.to_le_bytes());
        cut.extend((!0x8000u16).to_le_bytes());
        cut.extend(noise(PAGE as usize - cut.len(), 31));
        cut.resize(10 * PAGE as usize, 0);
        let mut scan = Rpf9Scan::new(Cursor::new(&cut), 0..u64::MAX).expect("opened");
        scan.by_ref().for_each(drop);
        assert_eq!((scan.broken_chunks(), scan.tails_not_deflate()), (1, 0));
    }

    #[test]
    fn a_body_the_file_ends_inside_is_refused_as_truncated() {
        let (bytes, _) = two_chunks(|mut z| {
            z.truncate(z.len() / 2);
            z
        });
        let len = bytes.len() as u64;
        assert!(matches!(
            read_rpf9_body(Cursor::new(&bytes), 0, u64::MAX),
            Err(Rpf9ScanError::Truncated { offset: 0, file_len }) if file_len == len
        ));
    }

    fn zeros(l: &mut Layout, len: usize) {
        l.unclaimed(&vec![0; len]);
    }

    #[test]
    fn a_run_of_zeros_a_download_has_not_delivered_is_missing_not_unclaimed() {
        let run = RPF9_MISSING_RUN as usize;
        let mut l = Layout::default();
        l.archive(2, TABLE, 1);
        zeros(&mut l, 0x7A0);
        l.unclaimed(&noise(0x100, 29));
        zeros(&mut l, run + 0x40);
        l.unclaimed(&noise(0x40, 31));
        zeros(&mut l, run - 16);
        l.unclaimed(&noise(0x20, 37));
        zeros(&mut l, 0x30);
        let mut scan = Rpf9Scan::new(Cursor::new(&l.bytes), 0..u64::MAX).expect("opened");
        let finds: Vec<_> = scan.by_ref().map(|f| f.expect("scanned")).collect();
        assert_eq!(finds, l.finds);
        assert_eq!(scan.missing_bytes(), run as u64 + 0x40);
        assert_eq!(scan.missing_runs(), 1);
        let zero = l.bytes[TABLE as usize..]
            .iter()
            .filter(|b| **b == 0)
            .count() as u64;
        assert_eq!(scan.unclaimed_zero_bytes(), zero - scan.missing_bytes());
        assert_eq!(
            scan.unclaimed_bytes(),
            l.bytes.len() as u64 - u64::from(TABLE) - scan.missing_bytes()
        );

        // A missing run that ends the file is closed when the scan ends.
        let mut tail = Layout::default();
        tail.archive(2, TABLE, 1);
        zeros(&mut tail, run);
        let mut scan = Rpf9Scan::new(Cursor::new(&tail.bytes), 0..u64::MAX).expect("opened");
        assert_eq!(scan.by_ref().count(), 1);
        assert_eq!(
            (scan.missing_bytes(), scan.unclaimed_bytes()),
            (run as u64, 0)
        );
    }

    /// An `ADAT` container in the measured layout: a 16-byte head, a u16 per stream when flag bit
    /// 0 is set, a u32 per stream (chunk count in bits 29 to 31, id below), a u64 per chunk (kind
    /// in bits 56 to 63, size in 28 to 55, offset below), then the payloads at 4-byte steps.
    #[cfg(feature = "awc")]
    fn adat(flags: u16, streams: &[Vec<(u8, Vec<u8>)>]) -> (Vec<u8>, Vec<u64>) {
        let index = if flags & 1 != 0 { streams.len() * 2 } else { 0 };
        let chunks: usize = streams.iter().map(Vec::len).sum();
        let tables = 16 + index + streams.len() * 4 + chunks * 8;
        let mut bytes = vec![0u8; tables];
        bytes[..4].copy_from_slice(b"ADAT");
        LE::write_u16(&mut bytes[4..], 1);
        LE::write_u16(&mut bytes[6..], flags);
        LE::write_u32(&mut bytes[8..], streams.len() as u32);
        LE::write_u32(&mut bytes[12..], tables as u32);
        let (mut head, mut word) = (16 + index, 16 + index + streams.len() * 4);
        let mut offsets = Vec::new();
        for (id, stream) in streams.iter().enumerate() {
            LE::write_u32(
                &mut bytes[head..],
                (stream.len() as u32) << 29 | (id as u32 + 1),
            );
            head += 4;
            for (kind, payload) in stream {
                let at = bytes.len().next_multiple_of(4);
                bytes.resize(at, 0);
                bytes.extend_from_slice(payload);
                let raw = u64::from(*kind) << 56 | (payload.len() as u64) << 28 | at as u64;
                LE::write_u64(&mut bytes[word..], raw);
                word += 8;
                offsets.push(at as u64);
            }
        }
        (bytes, offsets)
    }

    #[cfg(feature = "awc")]
    #[test]
    fn a_container_its_reader_refuses_by_its_tables_is_not_copied() {
        /// Counts the bytes read through it.
        struct Counted<'a>(Cursor<&'a [u8]>, u64);
        impl Read for Counted<'_> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                let n = self.0.read(buf)?;
                self.1 += n as u64;
                Ok(n)
            }
        }
        impl Seek for Counted<'_> {
            fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
                self.0.seek(to)
            }
        }
        // A head on each of 64 pages whose one chunk runs to the end of the file, and whose
        // tables end past the data offset it declares.
        let len = 3 * WINDOW;
        let mut bytes = vec![0u8; len];
        for page in 0..64 {
            let at = page * AWC_PAGE as usize;
            let (mut head, _) = adat(0xFF02, &[vec![(0x55, vec![1; 4])]]);
            LE::write_u32(&mut head[12..], 16);
            let size = (len - at - 0x20) as u64;
            LE::write_u64(&mut head[20..], 0x55 << 56 | size << 28 | 0x20);
            bytes[at..at + head.len()].copy_from_slice(&head);
        }
        let mut counted = Counted(Cursor::new(&bytes), 0);
        let mut scan = Rpf9Scan::new(&mut counted, 0..u64::MAX).expect("opened");
        assert_eq!(scan.by_ref().count(), 0);
        drop(scan);
        assert!(
            counted.1 <= 2 * len as u64,
            "{} bytes read of a {len}-byte file",
            counted.1
        );
    }

    #[cfg(feature = "awc")]
    fn stored_rsc8_bytes(virtual_size: u32) -> Vec<u8> {
        let mut l = Layout::default();
        l.rsc8(virtual_size);
        l.bytes
    }

    #[cfg(feature = "awc")]
    #[test]
    fn an_audio_container_is_listed_with_the_resources_its_table_states() {
        let mut l = Layout::default();
        l.archive(2, TABLE, 1);
        l.page();
        let at = l.at();
        // An `animation` chunk off a 16-byte slot, one not yet downloaded, one on a slot, and one
        // longer than the resource it starts with, which the measured size rule does not cover.
        let mut longer = stored_rsc8_bytes(0x80);
        longer.extend([0; 16]);
        let (bytes, offsets) = adat(
            0xFF02,
            &[
                vec![
                    (0x36, vec![7; 6]),
                    (0x55, noise(2048, 43)),
                    (0x76, noise(32, 47)),
                    (0x5C, stored_rsc8_bytes(0x200)),
                ],
                vec![(0x5C, vec![0; 0x110])],
                vec![(0xB2, noise(8, 41)), (0x5C, stored_rsc8_bytes(0x100))],
                vec![(0x5C, longer)],
            ],
        );
        let rsc8_at = [at + offsets[3], at + offsets[6]];
        assert_ne!(rsc8_at[0] % 16, 0, "the first resource sits off a slot");
        assert_eq!(rsc8_at[1] % 16, 0, "the second sits on a slot");
        let extent = bytes.len() as u64;
        l.claim(
            Rpf9ScanFind {
                offset: at,
                kind: Rpf9BodyKind::Awc {
                    flags: 0xFF02,
                    streams: 4,
                    chunks: 8,
                },
                span: extent.next_multiple_of(16),
                stored_len: Some(extent),
                inflated_len: Some(extent),
                container: None,
            },
            &bytes,
        );
        l.page();
        // A GTA VI table is read; one whose first-chunk index is not the running count is not.
        let indexed = crate::awc::opus::tests::container(
            0xFF25,
            &[7, 8],
            &[(1, vec![(0x55, noise(64, 53))])],
        );
        let indexed_len = indexed.len() as u64;
        l.claim(
            Rpf9ScanFind {
                offset: l.at(),
                kind: Rpf9BodyKind::Awc {
                    flags: 0xFF25,
                    streams: 1,
                    chunks: 1,
                },
                span: indexed_len.next_multiple_of(16),
                stored_len: Some(indexed_len),
                inflated_len: Some(indexed_len),
                container: None,
            },
            &indexed,
        );
        l.page();
        let mut miscounted = indexed.clone();
        miscounted[16] = 1;
        l.unclaimed(&miscounted);
        l.page();
        let (empty, _) = adat(0xFF02, &[vec![], vec![]]);
        l.unclaimed(&empty);
        // A table whose chunks end at 0 gives no end.
        l.page();
        let nowhere = l.at();
        let (mut zero_end, _) = adat(0xFF02, &[vec![(0x55, noise(64, 73))]]);
        LE::write_u64(&mut zero_end[20..], 0x55 << 56);
        l.unclaimed(&zero_end);
        // A container off a page start is not one the measured rule places.
        l.unclaimed(&noise(0x40, 59));
        l.pad();
        let off_page = l.at();
        assert_ne!(off_page % 0x1000, 0);
        let (whole, _) = adat(0xFF02, &[vec![(0x55, noise(64, 61))]]);
        l.unclaimed(&whole);
        l.pad();
        // A table that claims a chunk at 256 MiB or past it gives no end.
        l.page();
        let far = l.at();
        let (mut past, _) = adat(0xFF02, &[vec![(0x55, noise(64, 67))]]);
        LE::write_u64(&mut past[20..], 0x55 << 56 | 0x40 << 28 | 0x0FFF_FFF0);
        l.unclaimed(&past);
        l.unclaimed(&noise(0x40, 71));
        for at in [nowhere, off_page, far] {
            assert!(matches!(
                read_rpf9_body(Cursor::new(&l.bytes), at, u64::MAX),
                Err(Rpf9ScanError::NoBodyAt { .. })
            ));
        }

        let (finds, unclaimed, ..) = scan_all(&l.bytes);
        let nested: Vec<_> = rsc8_at
            .iter()
            .zip([0x210, 0x110])
            .map(|(&offset, extent)| Rpf9ScanFind {
                offset,
                kind: Rpf9BodyKind::StoredRsc8 {
                    version: 0x41,
                    extent,
                },
                span: extent,
                stored_len: Some(extent),
                inflated_len: Some(extent),
                container: Some(at),
            })
            .collect();
        let mut expected = l.finds.clone();
        expected.splice(2..2, nested.iter().copied());
        assert_eq!(finds, expected);
        let claimed: u64 = l.finds.iter().map(|f| f.span).sum();
        assert_eq!(unclaimed, l.bytes.len() as u64 - claimed);

        let (find, read) = read_rpf9_body(Cursor::new(&l.bytes), at, u64::MAX).expect("read");
        assert_eq!((find, read.as_slice()), (l.finds[1], bytes.as_slice()));
        let (find, read) =
            read_rpf9_body(Cursor::new(&l.bytes), rsc8_at[0], u64::MAX).expect("read");
        assert_eq!(find.kind, nested[0].kind);
        assert_eq!(read, stored_rsc8_bytes(0x200));
        assert!(matches!(
            read_rpf9_body(Cursor::new(&l.bytes), at, extent - 1),
            Err(Rpf9ScanError::InflateLimit { .. })
        ));
        let cut = &l.bytes[..(at + extent - 1) as usize];
        assert!(matches!(
            read_rpf9_body(Cursor::new(cut), at, u64::MAX),
            Err(Rpf9ScanError::LengthPastEnd { stated, .. }) if stated == extent
        ));
    }

    #[test]
    fn a_stated_length_past_the_end_is_found_and_refused() {
        let mut l = Layout::default();
        l.rsc8(0x4000);
        let mut bytes = l.bytes;
        bytes.truncate(0x1000);
        let extent = super::super::rsc8::Rsc8::HEADER_LEN as u64 + 0x4000;
        let (finds, ..) = scan_all(&bytes);
        assert_eq!(finds.len(), 1);
        assert_eq!(finds[0].stored_len, Some(extent));
        assert_eq!(finds[0].span, 0x1000);
        assert!(matches!(
            read_rpf9_body(Cursor::new(&bytes), 0, u64::MAX),
            Err(Rpf9ScanError::LengthPastEnd {
                offset: 0,
                stated,
                file_len: 0x1000
            }) if stated == extent
        ));
    }

    #[test]
    fn a_next_stream_past_the_chunk_limit_ends_the_run_a_read_returns() {
        let first = vec![0u8; PIECE as usize];
        let mut bytes = deflate(&first);
        bytes.resize(bytes.len().next_multiple_of(RPF9_SLOT as usize), 0);
        bytes.extend(deflate(&vec![0u8; CHUNK_LIMIT as usize + 1]));
        let (finds, ..) = scan_all(&bytes);
        let listed = finds.first().expect("the run at 0");
        let (find, read) = read_rpf9_body(Cursor::new(&bytes), 0, u64::MAX).expect("it reads");
        assert_eq!(&find, listed);
        assert_eq!(read, first);
    }

    #[test]
    fn a_body_that_inflates_past_the_limit_is_refused() {
        let l = every_kind();
        for find in &l.finds[1..3] {
            let inflated = find.inflated_len.expect("stated");
            assert!(read_rpf9_body(Cursor::new(&l.bytes), find.offset, inflated).is_ok());
            assert!(matches!(
                read_rpf9_body(Cursor::new(&l.bytes), find.offset, inflated - 1),
                Err(Rpf9ScanError::InflateLimit { offset, limit }) if offset == find.offset && limit == inflated - 1
            ));
        }
        let rsc8 = l
            .finds
            .iter()
            .find(|f| matches!(f.kind, Rpf9BodyKind::StoredRsc8 { .. }));
        let rsc8 = rsc8.expect("an RSC8 body");
        assert!(matches!(
            read_rpf9_body(Cursor::new(&l.bytes), rsc8.offset, 0x100),
            Err(Rpf9ScanError::InflateLimit { .. })
        ));
    }

    #[test]
    fn a_header_off_the_model_or_an_rsc8_not_stored_is_not_claimed() {
        let mut head = rpf9_head(6, TABLE, 1);
        head[0x34] = 1;
        let (finds, unclaimed, ..) = scan_all(&head);
        assert!(finds.is_empty(), "{finds:?}");
        assert_eq!(unclaimed, head.len() as u64);

        let mut l = Layout::default();
        l.rsc8(0x100);
        LE::write_u32(&mut l.bytes[4..], 0x0041);
        assert!(scan_all(&l.bytes).0.is_empty());
    }

    /// The first 0x100 bytes of a body whose slots have the classes `signature` spells, for a run
    /// of 0x100 bytes.
    fn signed_head(signature: &[u8; 8]) -> Vec<u8> {
        let mut head = vec![0u8; 0x100];
        for (i, class) in signature.iter().enumerate() {
            let word: u64 = match class {
                b'0' => 0,
                b'P' => 0x5000_0040,
                b'p' => 0x5000_0200,
                b'G' => 0x6000_0000,
                _ => 0x4000_0000_3F80_0000,
            };
            head[8 * i..8 * i + 8].copy_from_slice(&word.to_le_bytes());
        }
        head
    }

    #[test]
    fn every_family_signature_reads_back_from_slots_built_to_it() {
        use Rpf9Role::*;
        let families: [(&[u8; 8], Option<Rpf9Role>); 18] = [
            (b"0PhPGPPP", None),
            (b"0Ph0hhhh", Some(Bound)),
            (b"0PP0hhhh", Some(Drawable)),
            (b"0PP00hhh", Some(Drawable)),
            (b"0P0hPhPh", Some(Dictionary)),
            (b"0P0hh0hh", None),
            (b"0PPPPPh0", Some(DrawableWithBound)),
            (b"0Ph0h0h0", None),
            (b"0PhPPhh0", None),
            (b"0PP00000", None),
            (b"0P0hhPhP", None),
            (b"0P0P00h0", Some(DrawableWithBound)),
            (b"0PPPhhhh", None),
            (b"0PPPPPhP", Some(DrawableWithBound)),
            (b"0PPh00hh", None),
            (b"0PP00hh0", None),
            (b"0PP0PPPP", None),
            (b"0pp00hhh", None),
        ];
        for (classes, role) in families {
            let signature = Rpf9Signature::of(&signed_head(classes), 0x100).expect("a signature");
            assert_eq!(signature.as_bytes(), classes);
            assert_eq!(signature.to_string().as_bytes(), classes);
            assert_eq!(signature.role(), role, "{signature}");
        }
        assert_eq!(Rpf9Signature::of(&[0; 0x3F], 0x100), None);
    }

    #[test]
    fn the_families_measured_past_the_first_slots_are_placed_by_their_head() {
        let role = |head: &[u8]| Rpf9Signature::of(head, head.len() as u64).and_then(|s| s.role());
        let word = |head: &mut Vec<u8>, at: usize, value: u64| {
            head[at..at + 8].copy_from_slice(&value.to_le_bytes())
        };
        let mut navmesh = signed_head(b"0Ph0h0h0");
        assert_eq!(role(&navmesh), None);
        navmesh[0x14..0x18].copy_from_slice(&0x0001_0015u32.to_le_bytes());
        assert_eq!(role(&navmesh), Some(Rpf9Role::NavMesh));
        let mut octree = signed_head(b"0PhPPhh0");
        word(&mut octree, 0x18, 0x5000_0060);
        assert_eq!(role(&octree), Some(Rpf9Role::NavOctree));
        let mut pair = signed_head(b"0PhPGPPP");
        word(&mut pair, 0x38, 0x5000_0050);
        assert_eq!(role(&pair), Some(Rpf9Role::AccelerationStructure));
        // A cloth record: its flag, type and owner bytes; type 4 needs its box's arrays.
        let mut cloth = signed_head(b"0P0hh0hh");
        word(&mut cloth, 0x18, 0x1234_5678_0003_0000);
        assert_eq!(role(&cloth), Some(Rpf9Role::Cloth));
        cloth[0x1A] = 4;
        assert_eq!(role(&cloth), None);
        // Type 4: one box, all ones at 0x20 and arrays of capacity 16 at 0x28 and 0x38.
        let mut collider = signed_head(b"0P0hhPhP");
        word(&mut collider, 0x18, 0x1234_5678_0004_0000);
        assert_eq!(role(&collider), None);
        word(&mut collider, 0x20, 0x4000_0000_FFFF_FFFF);
        word(&mut collider, 0x30, 0x0010_0001);
        assert_eq!(role(&collider), Some(Rpf9Role::Cloth));
        // A drawable root of another key, with a level and a name in the section.
        let mut drawable = signed_head(b"0PPPhhhh");
        assert_eq!(role(&drawable), None);
        word(&mut drawable, 0x50, 0x5000_0040);
        word(&mut drawable, 0xA8, 0x5000_0040);
        assert_eq!(role(&drawable), Some(Rpf9Role::Drawable));
        word(&mut drawable, 0xA8, 0x5000_0200);
        assert_eq!(role(&drawable), None);
        // A morph target: a vertex buffer at 0x70 with its flags and no declaration.
        let mut morph = signed_head(b"0PP00000");
        word(&mut morph, 0x10, 0x5000_0070);
        assert_eq!(role(&morph), None);
        word(&mut morph, 0x80, 0x1601);
        assert_eq!(role(&morph), Some(Rpf9Role::MorphTarget));
        word(&mut morph, 0xB0, 0x5000_0040);
        assert_eq!(role(&morph), None);
        // An array of vertex buffers: a u16 count equal to its capacity, then zeros.
        let mut buffers = signed_head(b"0PPh00hh");
        word(&mut buffers, 0x18, 0x0003_0003);
        assert_eq!(role(&buffers), Some(Rpf9Role::VertexBuffers));
        word(&mut buffers, 0x18, 0x0004_0003);
        assert_eq!(role(&buffers), None);
        // A keyed blob behind a raw buffer object at 0x30, or with its bytes there.
        let mut blob = signed_head(b"0PP0hh0h");
        word(&mut blob, 0x10, 0x5000_0030);
        assert_eq!(role(&blob), None);
        word(&mut blob, 0x40, 0x1380);
        assert_eq!(role(&blob), Some(Rpf9Role::KeyedBlob));
        let mut blob = signed_head(b"0P0Phhhh");
        assert_eq!(role(&blob), None);
        word(&mut blob, 0x18, 0x5000_0030);
        assert_eq!(role(&blob), Some(Rpf9Role::KeyedBlob));
    }

    /// A virtual section of `len` bytes whose block map at 0x40 states `counts`, the rest filler
    /// that holds no pointer and no zero.
    fn virtual_section(len: usize, counts: (u8, u8)) -> Vec<u8> {
        let mut v = vec![0x11; len];
        v[..8].fill(0);
        v[8..16].copy_from_slice(&0x5000_0040u64.to_le_bytes());
        v[0x40..0x50].fill(0);
        (v[0x48], v[0x49]) = counts;
        v
    }

    /// The resource the find of a scan is.
    fn resource_of(find: &Rpf9ScanFind) -> Rpf9Resource {
        match find.kind {
            Rpf9BodyKind::Zlib(Rpf9ZlibRun {
                resource: Some(resource),
                ..
            }) => resource,
            other => panic!("a resource, not {other:?}"),
        }
    }

    #[test]
    fn a_resource_is_one_find_whose_blocks_read_as_its_two_sections() {
        let mut l = Layout::default();
        l.archive(2, TABLE, 1);
        l.page();
        // A two-block virtual section whose tail is stored raw, then one zlib physical block.
        let v = virtual_section(PIECE as usize + 0x2F5, (2, 1));
        let (head, tail) = v.split_at(PIECE as usize);
        let physical = vec![0x22u8; 0x500];
        l.resource(
            &[
                (head, false, false),
                (tail, true, false),
                (&physical, false, true),
            ],
            (2, 1),
        );
        l.page();
        // One zlib virtual block, then a physical block stored raw.
        let small = virtual_section(0x180, (1, 1));
        l.resource(
            &[(&small, false, false), (&[0x33; 0x2C3], true, true)],
            (1, 1),
        );
        l.page();
        l.unclaimed(&noise(0x40, 79));
        let (finds, ..) = scan_all(&l.bytes);
        assert_eq!(finds, l.finds);
        let resource = resource_of(&finds[1]);
        assert_eq!(resource.continuation_blocks(), 2);
        let (_, bytes) =
            read_rpf9_body(Cursor::new(&l.bytes), finds[1].offset, u64::MAX).expect("it reads");
        // A block stored raw reads zero-extended to 16, as its trailing zeros are not stored.
        let mut extended = v.clone();
        extended.resize(PIECE as usize + 0x300, 0);
        assert_eq!(resource.sections(&bytes), (&extended[..], &physical[..]));
        let (_, bytes) =
            read_rpf9_body(Cursor::new(&l.bytes), finds[2].offset, u64::MAX).expect("it reads");
        let mut extended = vec![0x33; 0x2C3];
        extended.resize(0x2D0, 0);
        assert_eq!(resource_of(&finds[2]).sections(&bytes).1, extended);
    }

    #[test]
    fn an_extracted_body_reads_again_from_offset_0_of_its_own_file() {
        let mut l = Layout::default();
        l.archive(2, TABLE, 1);
        l.page();
        let v = virtual_section(PIECE as usize + 0x2F5, (2, 1));
        let (head, tail) = v.split_at(PIECE as usize);
        l.resource(
            &[
                (head, false, false),
                (tail, true, false),
                (&[0x22; 0x500], false, true),
            ],
            (2, 1),
        );
        l.page();
        l.resource(
            &[
                (&virtual_section(0x180, (1, 1)), false, false),
                (&[0x33; 0x2C3], true, true),
            ],
            (1, 1),
        );
        l.page();
        // A virtual section of a whole 64 KiB whose map counts a tail block it does not write.
        l.resource(
            &[
                (&virtual_section(PIECE as usize, (2, 1)), false, false),
                (&[0x44; 0x40], false, true),
            ],
            (2, 1),
        );
        l.page();
        l.zlib(Rpf9Content::Xml, &xml(0x900), &[], true);
        l.rsc8(0x230);
        l.page();
        l.unclaimed(&noise(0x40, 89));
        let (finds, ..) = scan_all(&l.bytes);
        assert_eq!(finds, l.finds);
        let resources = finds.iter().filter(|f| f.offset > 0).collect::<Vec<_>>();
        assert_eq!(resources.len(), 5);
        let mut lengthened = 0;
        for find in resources {
            let (_, read) = read_rpf9_body(Cursor::new(&l.bytes), find.offset, u64::MAX)
                .expect("it reads in the archive");
            let (extracted_find, extracted) =
                extract_rpf9_body(Cursor::new(&l.bytes), find.offset, u64::MAX)
                    .expect("it extracts");
            assert_eq!(&extracted_find, find);
            let stored = matches!(
                find.kind,
                Rpf9BodyKind::Zlib(Rpf9ZlibRun {
                    resource: Some(_),
                    ..
                }) | Rpf9BodyKind::StoredRsc8 { .. }
            );
            assert_eq!(starts_stored_rpf9_resource(&extracted, 0x41), stored);
            if !stored {
                assert_eq!(extracted, read, "an XML body leaves inflated");
                continue;
            }
            let (loose, again) =
                read_rpf9_body(Cursor::new(&extracted), 0, u64::MAX).expect("it reads loose");
            assert_eq!(again, read, "{:#X}", find.offset);
            if let Rpf9BodyKind::Zlib(Rpf9ZlibRun {
                resource: Some(resource),
                ..
            }) = find.kind
            {
                // A block stored raw ends at a zero pad, so bytes appended after one are its own.
                if resource.blocks.iter().flatten().all(|b| b.zlib) {
                    let mut longer = extracted.clone();
                    longer.extend([0xAB; 0x40]);
                    let file_len = longer.len() as u64;
                    let refused = read_rpf9_body(Cursor::new(&longer), 0, u64::MAX).map(|(f, _)| f);
                    assert!(
                        matches!(
                            refused,
                            Err(Rpf9ScanError::Lengthened { end, file_len: f })
                                if end == extracted.len() as u64 && f == file_len
                        ),
                        "{refused:?}"
                    );
                    lengthened += 1;
                }
                let start = find.offset as usize;
                let stored = find.stored_len.expect("whole") as usize;
                assert_eq!(&extracted[..stored], &l.bytes[start..start + stored]);
                assert_eq!(
                    resource_of(&loose).sections(&again),
                    resource.sections(&read)
                );
                assert_eq!(resource_of(&loose).walk, Rpf9Walk::Whole);
            }
        }
        assert!(
            lengthened > 0,
            "a resource of zlib blocks alone is lengthened"
        );
        assert!(!starts_stored_rpf9_resource(
            &l.bytes[finds[5].offset as usize..],
            0x40
        ));
        assert_eq!(finds[4].file_name(), format!("{:#X}.xml", finds[4].offset));
        assert_eq!(finds[1].file_name(), format!("{:#X}.rsc", finds[1].offset));
    }

    #[test]
    fn a_whole_number_of_64_kib_counts_a_virtual_tail_it_does_not_write() {
        let mut l = Layout::default();
        l.archive(2, TABLE, 1);
        l.page();
        let v = virtual_section(PIECE as usize, (2, 1));
        l.resource(&[(&v, false, false), (&[0x22; 0x40], false, true)], (2, 1));
        l.page();
        l.unclaimed(&noise(0x40, 83));
        let (finds, ..) = scan_all(&l.bytes);
        assert_eq!(finds, l.finds);
        assert_eq!(resource_of(&finds[1]).virtual_len, PIECE);
    }

    #[test]
    fn a_block_whose_stream_reaches_the_cap_goes_on_in_the_next_stream() {
        let v = virtual_section(0x200, (1, 2));
        let head = vec![0x22u8; STREAM_CAP as usize + PIECE as usize];
        let tail = [0x33u8; 0x500];
        let cap = STREAM_CAP as usize;
        let mut bytes = vec![0u8; 16];
        for part in [&v[..], &head[..cap], &head[cap..], &tail[..]] {
            bytes.resize(bytes.len().next_multiple_of(16), 0);
            bytes.extend(deflate(part));
        }
        bytes.resize(
            bytes.len().next_multiple_of(PAGE as usize) + PAGE as usize,
            0,
        );
        let (finds, ..) = scan_all(&bytes);
        let [find] = finds.as_slice() else {
            panic!("one find, not {finds:?}")
        };
        let resource = resource_of(find);
        assert_eq!(resource.walk, Rpf9Walk::Whole);
        assert_eq!(resource.physical_len, (head.len() + tail.len()) as u64);
        assert_eq!(resource.continuation_blocks(), 2);
        let (_, read) = read_rpf9_body(Cursor::new(&bytes), 16, u64::MAX).expect("it reads");
        let (_, physical) = resource.sections(&read);
        assert_eq!(physical, [&head[..], &tail[..]].concat());
    }

    /// The physical length, block count and walk of the one resource `bytes` holds.
    fn walked(bytes: &[u8]) -> (u64, usize, Rpf9Walk) {
        let (finds, ..) = scan_all(bytes);
        let resource = resource_of(finds.first().expect("a find"));
        (
            resource.physical_len,
            resource.blocks.iter().flatten().count(),
            resource.walk,
        )
    }

    #[test]
    fn a_physical_block_goes_on_past_an_8_mib_stream_only_into_one_of_whole_64_kib() {
        let cap = 8 << 20;
        let v = virtual_section(0x200, (1, 1));
        let physical = noise(cap + 2 * PIECE as usize, 113);
        let split = lone_resource(&[&v, &physical[..cap], &physical[cap..]]);
        assert_eq!(walked(&split), (physical.len() as u64, 2, Rpf9Walk::Whole));
        let (_, read) = read_rpf9_body(Cursor::new(&split), 16, u64::MAX).expect("it reads");
        assert_eq!(&read[0x200..], &physical[..]);
        // A read at the second stream names the block it goes on.
        let first = (16 + deflate(&v).len()).next_multiple_of(16);
        let second = (first + deflate(&physical[..cap]).len()).next_multiple_of(16) as u64;
        assert!(matches!(
            read_rpf9_body(Cursor::new(&split), second, u64::MAX),
            Err(Rpf9ScanError::BlockOfResource { offset, owner: 16, section: "physical", block: 1 })
                if offset == second
        ));
        // A stream of 0x200 bytes at the next 16-byte end is no part of the block.
        let next = lone_resource(&[&v, &physical[..cap], &[0x5A; 0x200]]);
        assert_eq!(walked(&next), (cap as u64, 2, Rpf9Walk::Whole));
    }

    #[test]
    fn a_physical_section_of_whole_64_kib_counts_a_tail_it_does_not_write() {
        let v = virtual_section(0x200, (1, 2));
        let physical = noise(2 * PIECE as usize, 127);
        assert_eq!(
            walked(&lone_resource(&[&v, &physical])),
            (physical.len() as u64, 2, Rpf9Walk::Whole)
        );
    }

    #[test]
    fn an_empty_virtual_tail_leaves_the_next_stream_of_whole_64_kib_to_the_physical_head() {
        let v = virtual_section(3 * PIECE as usize, (2, 2));
        let physical = noise(2 * PIECE as usize + 0x500, 131);
        let (head, tail) = physical.split_at(2 * PIECE as usize);
        let bytes = lone_resource(&[&v, head, tail]);
        assert_eq!(walked(&bytes), (physical.len() as u64, 3, Rpf9Walk::Whole));
        let (find, read) = read_rpf9_body(Cursor::new(&bytes), 16, u64::MAX).expect("it reads");
        assert_eq!(resource_of(&find).sections(&read), (&v[..], &physical[..]));
    }

    #[test]
    fn a_stored_tail_that_ends_in_the_block_map_reads_with_its_slots() {
        let map = PIECE as usize + 0x100;
        let mut v = vec![0x11u8; map + 0x20];
        v[..8].fill(0);
        v[8..16].copy_from_slice(&(0x5000_0000 + map as u64).to_le_bytes());
        v[map..].fill(0);
        v[map + 8] = 2;
        let mut bytes = vec![0u8; 16];
        bytes.extend(deflate(&v[..PIECE as usize]));
        bytes.resize(bytes.len().next_multiple_of(16), 0);
        bytes.extend(&v[PIECE as usize..map + 9]);
        bytes.resize(
            bytes.len().next_multiple_of(PAGE as usize) + PAGE as usize,
            0,
        );
        let (_, read) = read_rpf9_body(Cursor::new(&bytes), 16, u64::MAX).expect("it reads");
        assert_eq!(read, v);
    }

    #[test]
    fn a_resource_whose_block_lies_in_a_piece_not_delivered_is_named_so() {
        let mut bytes = vec![0u8; 16];
        bytes.extend(deflate(&virtual_section(0x200, (1, 1))));
        let physical_at = (bytes.len() as u64).next_multiple_of(16);
        bytes.resize(bytes.len() + 3 * PAGE as usize, 0);
        let (finds, ..) = scan_all(&bytes);
        let [find] = finds.as_slice() else {
            panic!("one find, not {finds:?}")
        };
        let resource = resource_of(find);
        assert_eq!(resource.walk, Rpf9Walk::NotOnDisk { at: physical_at });
        assert_eq!((find.stored_len, find.inflated_len), (None, None));
        assert!(matches!(
            read_rpf9_body(Cursor::new(&bytes), 16, u64::MAX),
            Err(Rpf9ScanError::NotOnDisk { offset: 16, at }) if at == physical_at
        ));
    }

    #[test]
    fn a_resource_whose_first_block_a_missing_piece_cuts_is_named_so() {
        let mut v = virtual_section(0x8000, (1, 0));
        v[0x50..].copy_from_slice(&noise(0x8000 - 0x50, 97));
        let z = deflate(&v);
        let mut bytes = vec![0u8; 16];
        bytes.extend(&z[..z.len() / 2]);
        bytes.resize(bytes.len() + 3 * PAGE as usize, 0);
        let (finds, ..) = scan_all(&bytes);
        let first = finds.first().expect("a find");
        assert_eq!(first.offset, 16);
        assert_eq!(resource_of(first).walk, Rpf9Walk::NotOnDisk { at: 16 });
        assert!(matches!(
            read_rpf9_body(Cursor::new(&bytes), 16, u64::MAX),
            Err(Rpf9ScanError::NotOnDisk { offset: 16, at: 16 })
        ));
    }

    #[test]
    fn a_stream_whose_tail_is_not_deflate_data_with_no_piece_missing_is_named_so() {
        // The stream's bytes from a page + 0x400 on are replaced; a zero page lies past its end.
        let tail_replaced = |stream: &[u8], from: usize| {
            assert!(
                stream.len() > from + 0x100,
                "the replaced tail is inside the stream"
            );
            let mut bytes = stream[..from].to_vec();
            bytes.extend(noise(0x2000, 101));
            bytes.resize(
                bytes.len().next_multiple_of(PAGE as usize) + 3 * PAGE as usize,
                0,
            );
            bytes
        };
        // Numbers as text, which deflate codes with Huffman codes rather than storing raw.
        let numbers = |len: usize| -> Vec<u8> {
            let text = (0..len).flat_map(|i| format!("{} ", i * 7919 % 10007).into_bytes());
            text.take(len).collect()
        };
        let mut v = virtual_section(0x8000, (1, 0));
        v[0x50..].copy_from_slice(&numbers(0x8000 - 0x50));
        let mut first = vec![0u8; 16];
        first.extend(tail_replaced(&deflate(&v), 0x1400 - 16));
        let mut later = vec![0u8; 16];
        later.extend(deflate(&virtual_section(0x200, (1, 1))));
        later.resize(later.len().next_multiple_of(16), 0);
        let block = later.len();
        later = [
            &later[..],
            &tail_replaced(&deflate(&numbers(0x4000)), 0x800)[..],
        ]
        .concat();
        for (bytes, from) in [(first, 0x1400), (later, block + 0x800)] {
            let (finds, ..) = scan_all(&bytes);
            let find = finds.first().expect("a find");
            let Rpf9Walk::TailNotDeflate { at } = resource_of(find).walk else {
                panic!("a tail not deflate data, not {:?}", resource_of(find).walk)
            };
            assert!((from as u64..from as u64 + 0x2000).contains(&at), "{at:#X}");
            assert!(matches!(
                read_rpf9_body(Cursor::new(&bytes), 16, u64::MAX),
                Err(Rpf9ScanError::TailNotDeflate { offset: 16, at: a }) if a == at
            ));
        }
    }

    #[test]
    fn pointers_just_past_the_virtual_section_are_reported_and_one_is_a_value() {
        let read = |targets: &[u64]| {
            let mut v = virtual_section(0x200, (1, 0));
            for (i, t) in targets.iter().enumerate() {
                v[0x100 + 8 * i..0x108 + 8 * i].copy_from_slice(&(0x5000_0000 + t).to_le_bytes());
            }
            let mut bytes = vec![0u8; 16];
            bytes.extend(deflate(&v));
            let (find, read) = read_rpf9_body(Cursor::new(bytes), 16, u64::MAX).expect("it reads");
            assert_eq!(read, v);
            resource_of(&find).walk
        };
        assert_eq!(read(&[0x200]), Rpf9Walk::Whole);
        assert_eq!(
            read(&[0x200, 0x210, 0x210]),
            Rpf9Walk::PointersPast {
                section: "virtual",
                targets: 2
            }
        );
    }

    #[test]
    fn a_first_entry_is_found_at_the_table_s_end_or_the_page_after_behind_its_slot() {
        let mut l = Layout::default();
        let v = virtual_section(0x100, (1, 0));
        l.archive(2, TABLE, 1);
        l.resource(&[(&v, false, false)], (1, 0));
        l.page();
        l.archive(2, TABLE, 2);
        l.page();
        l.resource(&[(&v, false, false)], (1, 0));
        let (finds, ..) = scan_all(&l.bytes);
        assert_eq!(finds, l.finds);
    }

    #[test]
    fn a_stored_rsc8_is_found_at_any_even_offset() {
        let mut one = Layout::default();
        one.rsc8(0x100);
        let mut bytes = noise(4, 89);
        let at = bytes.len() as u64;
        bytes.extend(&one.bytes);
        bytes.extend(&one.bytes);
        let (finds, ..) = scan_all(&bytes);
        let offsets: Vec<u64> = finds.iter().map(|f| f.offset).collect();
        assert_eq!(offsets, [at, at + one.bytes.len() as u64]);
    }

    #[test]
    fn content_is_told_by_the_first_chunk() {
        assert_eq!(
            classify(b"<?xml version=\"1.0\"?><a/>", 24),
            Rpf9Content::Xml
        );
        assert_eq!(
            classify(b"\xEF\xBB\xBF<CDataFileMgr__ContentDef>\n", 30),
            Rpf9Content::Xml
        );
        assert_eq!(
            classify(b"\t<Item value=\"1\"/>\n</Items>", 26),
            Rpf9Content::Text
        );
        assert_eq!(
            classify(b"value=\"16.0\"/>\n</Item>", 22),
            Rpf9Content::Text
        );
        assert_eq!(classify(b"PSIN\0\0\0\x10", 16), Rpf9Content::Pso);
        assert_eq!(classify(b"RBF0\xFB\xFF\x01\0", 8), Rpf9Content::Rbf);
        assert_eq!(classify(&[0; 0x20], 0x20), Rpf9Content::ResourceShaped);
        assert_eq!(classify(&[0; 0x10], 0x10), Rpf9Content::Other);
        assert_eq!(classify(b"<a\x01b>", 5), Rpf9Content::Other);
        assert_eq!(
            classify(b"MoVE\x13\0\0\0", 8),
            Rpf9Content::Identified(crate::gta6::other_bodies::OtherBody::MoveNetwork)
        );
    }

    #[test]
    fn a_head_with_the_animation_root_s_zeros_and_map_flag_is_a_clip_root() {
        let mut root = [0u8; 0x60];
        root[0x5F] = 1;
        assert_eq!(classify(&root, 0x60), Rpf9Content::ClipRoot);
        root[0x20] = 1;
        assert_eq!(classify(&root, 0x60), Rpf9Content::ResourceShaped);
        assert_eq!(classify(&root[..0x5F], 0x5F), Rpf9Content::ResourceShaped);
    }

    #[test]
    fn a_head_of_whole_64_kib_goes_on_as_a_stored_tail_that_holds_the_block_map() {
        let map = PIECE as usize + 0x100;
        let mut v = vec![0x11u8; PIECE as usize + 0x2F0];
        v[..8].fill(0);
        v[8..16].copy_from_slice(&(0x5000_0000 + map as u64).to_le_bytes());
        v[map..map + 16].fill(0);
        v[map + 8] = 2;
        let mut bytes = vec![0u8; 16];
        bytes.extend(deflate(&v[..PIECE as usize]));
        bytes.resize(bytes.len().next_multiple_of(16), 0);
        bytes.extend(&v[PIECE as usize..]);
        bytes.resize(
            bytes.len().next_multiple_of(PAGE as usize) + PAGE as usize,
            0,
        );
        let (find, read) = read_rpf9_body(Cursor::new(&bytes), 16, u64::MAX).expect("it reads");
        assert_eq!(read, v);
        assert_eq!(resource_of(&find).virtual_blocks, 2);
    }

    #[test]
    fn a_navigation_mesh_s_words_that_read_as_pointers_past_its_section_do_not_refuse_it() {
        let mut v = virtual_section(0x200, (1, 0));
        v[0x14..0x18].copy_from_slice(&0x0001_0015u32.to_le_bytes());
        for at in [0x18, 0x28, 0x38] {
            v[at..at + 8].fill(0);
        }
        for (i, target) in [0x200u64, 0x210].iter().enumerate() {
            v[0x100 + 8 * i..0x108 + 8 * i].copy_from_slice(&(0x5000_0000 + target).to_le_bytes());
        }
        let mut bytes = vec![0u8; 16];
        bytes.extend(deflate(&v));
        let (find, read) = read_rpf9_body(Cursor::new(&bytes), 16, u64::MAX).expect("it reads");
        assert_eq!(read, v);
        let Rpf9BodyKind::Zlib(run) = find.kind else {
            panic!("a zlib run");
        };
        assert_eq!(
            run.signature.and_then(Rpf9Signature::role),
            Some(Rpf9Role::NavMesh)
        );
    }

    /// An `SGD5` head of one binding layout of `records` records, then noise.
    fn sgd5(records: u8, len: usize) -> Vec<u8> {
        let mut bytes = noise(len, 23);
        bytes[..4].copy_from_slice(b"SGD5");
        LE::write_u32(&mut bytes[0x0C..], 1);
        LE::write_u32(&mut bytes[0x10..], 24 + 8 * u32::from(records));
        LE::write_u16(&mut bytes[0x1C..], u16::from(records));
        bytes[0x1E..0x24].copy_from_slice(&[0, records, 0, 0, 0, 0]);
        LE::write_u32(&mut bytes[0x28..], 0);
        bytes
    }

    #[test]
    fn an_effect_container_spans_to_the_next_entry_and_a_pipeline_cache_is_named() {
        let mut l = Layout::default();
        l.archive(3, TABLE, 1);
        l.page();
        let start = l.at();
        let mut container = sgd5(3, 0x1800);
        container.resize(0x2000, 0);
        l.claim(
            Rpf9ScanFind {
                offset: start,
                kind: Rpf9BodyKind::Sgd5 { head_layouts: 1 },
                span: 0x2000,
                stored_len: None,
                inflated_len: None,
                container: None,
            },
            &container,
        );
        let mut cache = noise(0x900, 29);
        cache[..4].copy_from_slice(b"PS\x00\x09");
        l.zlib(Rpf9Content::PipelineCache, &cache, &[], true);
        let (finds, _, _, _) = scan_all(&l.bytes);
        assert_eq!(
            finds[1], l.finds[1],
            "the container ends at the zlib entry's page"
        );
        assert!(matches!(
            finds[2].kind,
            Rpf9BodyKind::Zlib(run) if run.content == Rpf9Content::PipelineCache
        ));
        let (_, bytes) = read_rpf9_body(Cursor::new(&l.bytes), start, u64::MAX).expect("read");
        assert_eq!(bytes, container);

        // A head whose first layout does not frame, or that is off a page, is no container.
        let mut broken = sgd5(3, 0x40);
        broken[0x1F] = 2;
        assert_eq!(sgd5_head(0, &broken), None);
        assert_eq!(sgd5_head(0x10, &sgd5(3, 0x40)), None);
        assert_eq!(sgd5_head(0x1000, &sgd5(3, 0x40)), Some(1));
    }

    /// A resource after its zero slot, then zero bytes to the page after the next: `blocks` are
    /// its sections' bytes, the virtual one first.
    fn lone_resource(blocks: &[&[u8]]) -> Vec<u8> {
        let mut bytes = vec![0u8; 16];
        for block in blocks {
            bytes.resize(bytes.len().next_multiple_of(16), 0);
            bytes.extend(deflate(block));
        }
        bytes.resize(
            bytes.len().next_multiple_of(PAGE as usize) + PAGE as usize,
            0,
        );
        bytes
    }

    #[test]
    fn a_resource_the_file_no_longer_reaches_once_read_is_refused_as_truncated() {
        /// A file that reads as ended wherever a read starts before `1`, as one cut down since
        /// it was measured does.
        struct Shrunk<'a>(Cursor<&'a [u8]>, u64);
        impl Read for Shrunk<'_> {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.0.position() < self.1 {
                    return Ok(0);
                }
                self.0.read(buf)
            }
        }
        impl Seek for Shrunk<'_> {
            fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
                self.0.seek(to)
            }
        }
        // The zero slot before the resource is read after its first block, and reads as the end.
        let bytes = lone_resource(&[&virtual_section(0x200, (1, 0))]);
        let read = read_rpf9_body(Shrunk(Cursor::new(&bytes), 16), 16, u64::MAX);
        assert!(
            matches!(
                read,
                Err(Rpf9ScanError::Truncated {
                    offset: 16,
                    file_len: 0
                })
            ),
            "{read:?}"
        );
    }

    #[test]
    fn a_resource_whose_pointers_reach_just_past_a_section_is_listed_whole_with_them_named() {
        let with = |v: &mut Vec<u8>, base: u64, targets: &[u64]| {
            for (i, t) in targets.iter().enumerate() {
                v[0x100 + 8 * i..0x108 + 8 * i].copy_from_slice(&(base + t).to_le_bytes());
            }
        };
        let mut v = virtual_section(0x200, (1, 0));
        with(&mut v, 0x5000_0000, &[0x200, 0x210]);
        let physical = vec![0x22u8; 0x500];
        let mut w = virtual_section(0x200, (1, 1));
        with(&mut w, 0x6000_0000, &[0x500, 0x510]);
        for (bytes, section, len) in [
            (lone_resource(&[&v]), "virtual", 0x200),
            (lone_resource(&[&w, &physical]), "physical", 0x500),
        ] {
            let (finds, ..) = scan_all(&bytes);
            let [find] = finds.as_slice() else {
                panic!("one find, not {finds:?}")
            };
            assert_eq!(
                resource_of(find).walk,
                Rpf9Walk::PointersPast {
                    section,
                    targets: 2
                }
            );
            assert!(find.stored_len.is_some());
            let (_, read) = read_rpf9_body(Cursor::new(&bytes), 16, u64::MAX).expect("it reads");
            let (virtual_read, physical_read) = resource_of(find).sections(&read);
            assert_eq!(
                [virtual_read.len(), physical_read.len()],
                if section == "virtual" {
                    [len, 0]
                } else {
                    [0x200, len]
                }
            );
        }
        // One word past the section is a value: the resource is whole.
        let mut one = virtual_section(0x200, (1, 0));
        with(&mut one, 0x5000_0000, &[0x200]);
        let (finds, ..) = scan_all(&lone_resource(&[&one]));
        assert_eq!(resource_of(&finds[0]).walk, Rpf9Walk::Whole);
    }

    #[test]
    fn a_pointer_word_two_takes_split_is_kept_once_and_a_backward_one_not_at_all() {
        let mut words = [0u8; 0x40];
        for (at, word) in [
            (0x08, 0x5000_0020u64),
            (0x18, 0x5000_0100),
            (0x28, 0x6000_0020),
            (0x30, 0x5000_0010),
            (0x38, 0x5000_0108),
        ] {
            words[at..at + 8].copy_from_slice(&word.to_le_bytes());
        }
        for cut in [1, 0x0C, 0x1B, 0x20, 0x2D] {
            let mut sink = Sink::new(None, u64::MAX);
            sink.take(&words[..cut]);
            sink.take(&words[cut..]);
            assert_eq!(
                sink.pointers,
                [
                    (0x5000_0020, 0x08),
                    (0x5000_0100, 0x18),
                    (0x6000_0020, 0x28)
                ],
                "cut at {cut:#X}"
            );
        }
    }

    #[test]
    fn a_read_at_a_block_after_a_resource_s_first_names_the_resource_it_belongs_to() {
        let mut l = Layout::default();
        l.archive(2, TABLE, 1);
        l.page();
        let v = virtual_section(0x300, (1, 2));
        let (head, tail) = (noise(PIECE as usize, 103), noise(0x500, 107));
        l.resource(
            &[
                (&v, false, false),
                (&head, false, true),
                (&tail, true, true),
            ],
            (1, 2),
        );
        l.page();
        l.unclaimed(&noise(0x40, 101));
        let owner = l.finds[1].offset;
        let blocks = resource_of(&l.finds[1]).blocks;
        for (block, index) in [(1, 1), (2, 2)] {
            let at = blocks[block].expect("a block").offset;
            assert!(matches!(
                read_rpf9_body(Cursor::new(&l.bytes), at, u64::MAX),
                Err(Rpf9ScanError::BlockOfResource { offset, owner: o, section: "physical", block: b })
                    if offset == at && o == owner && b == index
            ));
        }
        // Past the resource's last block, the nearest start before is no owner.
        let noise_at = l.unclaimed.last().expect("noise").start as u64;
        assert!(matches!(
            read_rpf9_body(Cursor::new(&l.bytes), noise_at, u64::MAX),
            Err(Rpf9ScanError::NoBodyAt { .. })
        ));
    }

    #[test]
    fn a_nested_archive_s_names_are_placed_packed_or_on_the_next_page_before_the_next_header() {
        // Packed, paged, and a blob longer than the header states, which is not placed.
        for (paged, extra) in [(false, 0), (true, 0), (false, 16)] {
            let mut l = Layout::default();
            l.archive(2, TABLE, 1);
            l.page();
            l.archive(2, TABLE, 2);
            let nested = l.finds[1].offset;
            l.zlib(Rpf9Content::Xml, &xml(0x900), &[], true);
            if paged {
                l.page();
            }
            match extra {
                0 => l.names(NAMES as usize, nested),
                _ => l.unclaimed(&noise(NAMES as usize + extra, 109)),
            }
            l.page();
            l.archive(2, TABLE, 3);
            l.zlib(Rpf9Content::Xml, &xml(0x700), &[], true);
            let case = format!("paged {paged}, {extra} bytes over");
            assert_eq!(scan_all(&l.bytes).0, l.finds, "{case}");
            assert_eq!(scan_in_slices(&l.bytes, 0x800).0, l.finds, "{case}");
        }
    }

    #[test]
    fn a_nested_header_off_a_measured_rule_is_listed_with_the_rule_named() {
        let mut l = Layout::default();
        l.archive(2, TABLE, 1);
        l.page();
        let mut head = rpf9_head(2, TABLE, 2);
        head[0x1A] = 9;
        assert!(Rpf9Header::parse(&head).is_err());
        let header = Rpf9Header::read(&head).expect("an RPF9 head");
        assert!(header.broken_rule().is_some_and(|why| why.contains("0x1A")));
        let find = Rpf9ScanFind {
            offset: l.at(),
            kind: Rpf9BodyKind::Archive(header),
            span: u64::from(TABLE),
            stored_len: None,
            inflated_len: None,
            container: None,
        };
        l.claim(find, &head);
        l.zlib(Rpf9Content::Xml, &xml(0x500), &[], true);
        assert_eq!(scan_all(&l.bytes).0, l.finds);
    }

    #[test]
    fn a_first_entry_no_detector_names_is_listed_where_the_header_puts_it() {
        // At the table's end, behind a zero slot on the page after it, and off both.
        let after_page = 0x1000 - TABLE as usize + 16;
        for (gap, listed) in [(0, true), (after_page, true), (0x40, false)] {
            let mut l = Layout::default();
            l.archive(2, TABLE, 1);
            l.unclaimed(&vec![0; gap]);
            let entry = noise(0x300, 113);
            if listed {
                let head = <[u8; 4]>::try_from(&entry[..4]).expect("four bytes");
                let find = Rpf9ScanFind {
                    offset: l.at(),
                    kind: Rpf9BodyKind::FirstEntry { head },
                    span: 16,
                    stored_len: None,
                    inflated_len: None,
                    container: None,
                };
                l.claim(find, &entry[..16]);
                l.unclaimed(&entry[16..]);
            } else {
                l.unclaimed(&entry);
            }
            assert_eq!(
                scan_all(&l.bytes).0,
                l.finds,
                "{gap:#X} bytes after the table"
            );
        }
    }

    #[test]
    fn a_text_table_and_audio_game_data_are_named_by_their_content() {
        use crate::gta6::text::{TextEntry, TextItem, TextTable};
        let table = TextTable {
            groups: 2,
            entries: vec![TextEntry {
                key: 0x0000_0081,
                group: 1,
                slot: 0,
                item: TextItem::Text("a made-up text".into()),
            }],
        };
        let sections = super::super::Rpf9Sections {
            form: super::super::Rpf9Form::BlockMapped,
            virtual_section: table.to_section().expect("written"),
            physical: Vec::new(),
        };
        let file = super::super::write_rpf9_resource(&sections).expect("a loose resource");
        let content = |bytes: &[u8]| match scan_all(bytes).0.first().map(|f| f.kind) {
            Some(Rpf9BodyKind::Zlib(run)) => Some(run.content),
            _ => None,
        };
        assert_eq!(content(&file), Some(Rpf9Content::TextTable));
        // The same resource with another class first is no text table.
        let mut other = sections.clone();
        let at = (LE::read_u64(&other.virtual_section[0x28..]) & 0x0FFF_FFFF) as usize;
        LE::write_u32(&mut other.virtual_section[at..], 0x1234_5678);
        let file = super::super::write_rpf9_resource(&other).expect("a loose resource");
        assert_eq!(content(&file), Some(Rpf9Content::ResourceShaped));

        // Audio game data whose second 64 KiB piece is stored raw between two streams.
        let mut body = noise(3 * PIECE as usize + 0x123, 83);
        LE::write_u32(&mut body, 166);
        LE::write_u32(&mut body[8..], crate::gta6::audio_data::STAMP);
        let mut l = Layout::default();
        l.zlib(Rpf9Content::AudioData, &body, &[1], false);
        let (finds, ..) = scan_all(&l.bytes);
        assert_eq!(finds, l.finds);
        let (find, read) = read_rpf9_body(Cursor::new(&l.bytes), 0, u64::MAX).expect("it reads");
        assert_eq!((find, read), (l.finds[0], body));
    }

    #[cfg(feature = "awc")]
    #[test]
    fn config_byte_1_is_listed_only_in_a_container_of_the_measured_variant() {
        use crate::awc::opus::tests::{container, Chunks};
        let rsc8 = |config: u8| {
            let mut bytes = stored_rsc8_bytes(0x100);
            bytes[7] = config;
            bytes
        };
        let listed = |flags: u16, streams: &[(u32, Chunks)]| -> Vec<u8> {
            let mut l = Layout::default();
            l.archive(2, TABLE, 1);
            l.page();
            let words = vec![9; streams.len() + 1];
            l.unclaimed(&container(flags, &words, streams));
            l.page();
            (scan_all(&l.bytes).0.iter())
                .filter_map(|f| match f.kind {
                    Rpf9BodyKind::StoredRsc8 { .. } => {
                        let at = f.offset as usize;
                        Some(l.bytes[at + 7])
                    }
                    _ => None,
                })
                .collect()
        };
        let stream = |config: u8| vec![(0x55, noise(64, 89)), (0x5C, rsc8(config))];
        let one_each = [(1, stream(1)), (2, stream(1))];
        assert_eq!(listed(0xFF61, &one_each), [1, 1]);
        assert_eq!(listed(0xFF61, &[(1, stream(0)), (2, stream(0))]), [0, 0]);
        // Another flag word, a stream with two animation chunks, or a config byte past 1.
        assert_eq!(listed(0xFF25, &one_each), Vec::<u8>::new());
        let mut twice = stream(1);
        twice.push((0x5C, rsc8(1)));
        assert_eq!(
            listed(0xFF61, &[(1, twice), (2, stream(1))]),
            Vec::<u8>::new()
        );
        assert_eq!(
            listed(0xFF61, &[(1, stream(2)), (2, stream(2))]),
            Vec::<u8>::new()
        );

        // A stored RSC8 of config 1 is refused in an archive and read where a file of its own holds it.
        let in_archive = |config: u8| {
            let mut l = Layout::default();
            l.archive(2, TABLE, 1);
            l.page();
            let at = l.at();
            l.unclaimed(&rsc8(config));
            l.page();
            (scan_all(&l.bytes).0.iter())
                .any(|f| f.offset == at && matches!(f.kind, Rpf9BodyKind::StoredRsc8 { .. }))
        };
        assert!(in_archive(0));
        assert!(!in_archive(1));
        assert!(read_rpf9_body(Cursor::new(&rsc8(1)), 0, u64::MAX).is_ok());
        assert!(read_rpf9_body(Cursor::new(&rsc8(2)), 0, u64::MAX).is_err());
    }
}
