//! A `.ttpf` texture pack: an `RDS5` header, a table of 8-byte rows, an `ETPF` directory of textures
//! in row 0, then bodies of tiled texture memory. A texture is keyed by the 64-bit hash of its
//! lower-cased name, which the pack does not hold.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::{Path, PathBuf};

use super::bcpack::{self, BcPackError};
use super::tiling::{chain, chain_lengths, volume_chain, Layout, Level, Tile};
use crate::codec::texture::{DxgiFormat, GrcTextureFormat, TextureSource};

const MAGIC: [u8; 4] = *b"RDS5";
const DIRECTORY_MAGIC: [u8; 4] = *b"ETPF";
/// The directory version every measured pack carries.
const DIRECTORY_VERSION: u32 = 0x27A;
const HEADER_LEN: u64 = 0x10;
const ROW_LEN: u64 = 8;
const ENTRY_LEN: u64 = 16;
const KEY_LEN: u64 = 8;
/// The most memory one texture is read into.
const TEXTURE_LIMIT: u64 = 1 << 30;

/// A body's codec kind: the bytes as stored.
pub const KIND_STORED: u8 = 0;
/// A body's codec kind: one zlib stream.
pub const KIND_ZLIB: u8 = 8;

/// Why a pack or one of its textures cannot be read.
#[derive(Debug, thiserror::Error)]
pub enum TtpfError {
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("not a GTA VI texture pack: the file starts {0:02X?}, not RDS5")]
    NotATexturePack([u8; 4]),
    #[error(
        "the file's first bytes are all zero, as a piece a download has not delivered yet reads: \
         the pack opens once its start is on disk"
    )]
    HeadIsZero,
    #[error(
        "the GTA VI texture pack's {what} ends at byte {end}, past the {on_disk} bytes on disk"
    )]
    HeadNotOnDisk {
        what: &'static str,
        end: u64,
        on_disk: u64,
    },
    #[error("the GTA VI texture pack does not hold to its measured layout: {0}")]
    Malformed(String),
    #[error("no texture {index}: the pack holds {count}")]
    NoSuchTexture { index: usize, count: usize },
    #[error(
        "texture {index}'s directory entry, or the one after it, reads as zeros, as a piece a \
         download has not delivered does, so the rows it owns are not known"
    )]
    RangeNotKnown { index: usize },
    #[error(
        "texture {index} has dimension {dimension} and {slices} slice(s): the measured shapes are \
         a 2D texture, a cube of six faces, an array in 64 KiB tiles and a volume"
    )]
    UnknownShape {
        index: usize,
        dimension: u8,
        slices: u16,
    },
    #[error(
        "texture {index} is a volume in tile mode {tile_mode}, and only volumes in tile mode 1's \
         4 KiB tiles are measured"
    )]
    Volume { index: usize, tile_mode: u8 },
    #[error(
        "texture {index} is a cube of {bytes}-byte pixels, and only cubes of BC blocks and of \
         4-byte pixels are measured"
    )]
    CubeNotMeasured { index: usize, bytes: usize },
    #[error(
        "texture {index} is a cube of {rows} rows, and every measured cube has as many as its \
         format code's flags ({flags}) plus one"
    )]
    CubeRows {
        index: usize,
        rows: usize,
        flags: u16,
    },
    #[error(
        "texture {index} has format code {code:#05x}, whose DXGI number has no measured element \
         and tile order"
    )]
    UnknownFormat { index: usize, code: u16 },
    #[error(
        "texture {index}'s top mip sits in the packed mip tail, where its place is not measured \
         for {format}"
    )]
    TopMipInTail { index: usize, format: String },
    #[error("body {row} is stored with codec kind {kind}, {}", codec_state(*kind))]
    UnidentifiedCodec { row: usize, kind: u8 },
    #[error(
        "body {row} holds BC{kind} blocks through the BCPack coder, and does not decode: {source}"
    )]
    BcPack {
        row: usize,
        kind: u8,
        #[source]
        source: BcPackError,
    },
    #[error("body {row} ends at byte {end}, past the {on_disk} bytes on disk")]
    NotOnDisk { row: usize, end: u64, on_disk: u64 },
    #[error(
        "body {row}'s zlib stream does not inflate to exactly its {size} bytes; a piece of the \
         file may be missing"
    )]
    Inflate { row: usize, size: u64 },
    #[error(
        "the pack is not wholly on disk, so the rows a save leaves unchanged cannot be carried \
         as the file stores them"
    )]
    NotWhole,
    #[error(
        "texture {index} is a cube, an array or a volume: a replacement is written as one 2D \
         chain"
    )]
    ReplaceShape { index: usize },
    #[error("texture {index} holds {slices} slice(s), so there is no slice {slice}")]
    NoSuchSlice {
        index: usize,
        slice: u32,
        slices: u32,
    },
    #[error(
        "texture {index} is a volume of {mips} mips, whose z-slabs each hold part of every level, \
         so one slice is not replaced alone"
    )]
    VolumeLevels { index: usize, mips: usize },
    #[error("texture {index} is {format}, which OpenRage has no encoder for in this build")]
    ReplaceFormat { index: usize, format: String },
    #[error(
        "texture {index}'s chain holds mips in its tail tile whose slot is not measured, so a \
         replacement could not place them"
    )]
    TailSlotNotMeasured { index: usize },
    #[error(
        "texture {index} spreads its top mip over {rows} rows that are not the 4 MiB rows a top \
         mip over 4 MiB is cut into, so a replacement could not say where each part goes"
    )]
    SplitTopMip { index: usize, rows: usize },
    #[error(
        "texture {index}'s row-size codes are not those its rows' sizes give, so the rule that \
         writes them does not cover it"
    )]
    CodesNotByRule { index: usize },
    #[error(
        "the image is {}x{}, and texture {index} is {width}x{height}: a replacement keeps the \
         texture's size, format and rows", got.0, got.1
    )]
    ImageSize {
        index: usize,
        width: u32,
        height: u32,
        got: (u32, u32),
    },
    #[error("encoding the replacement failed: {0}")]
    Encode(String),
    #[error(
        "the gap after row {row} is neither 256-byte nor 4 KiB alignment, so a save cannot carry \
         where the row after it starts"
    )]
    GapNotCarried { row: usize },
    #[error("the saved pack would be {0} bytes, past the 4 GiB a row's offset can state")]
    TooLarge(u64),
    #[error(
        "texture {index} cannot hold {mips} mips: from 1 to {most}, the levels its chain's \
         measured tail slots place; where a tail level past them lies is not measured"
    )]
    MipsNotPlaced {
        index: usize,
        mips: u32,
        most: usize,
    },
    #[error(
        "texture {index} cannot be {width}x{height}: a pack states each side from 1 to 16384, \
         and a texture is read into at most 1 GiB"
    )]
    ShapeTooLarge {
        index: usize,
        width: u32,
        height: u32,
    },
    #[error(
        "texture {index} cannot take {format}: it is no format a pack is measured to hold, or \
         OpenRage has no encoder for it in this build"
    )]
    ReshapeFormat { index: usize, format: String },
    #[error(
        "texture {index} is an array in 4 KiB tiles, or of no measured shape: another size, \
         format or mip count is planned for a 2D texture, a cube, an array in 64 KiB tiles and a \
         volume of one level"
    )]
    ReshapeShape { index: usize },
    #[error(
        "texture {index} holds {slices} slice(s), and a reshape takes one image for each, all of \
         one size"
    )]
    SliceImages { index: usize, slices: usize },
    #[error(
        "texture {index} would need a row whose size no row-size code states, or a mip over \
         4 MiB that is not whole 4 MiB rows"
    )]
    RowNotCoded { index: usize },
    #[error("texture {index}'s {mips} mips cannot be stated by its nibble and its rows")]
    NibbleCannotState { index: usize, mips: u32 },
    #[error("the pack's textures would reach page {0:#x}, past what an entry or a row states")]
    PagePastField(u64),
    #[error("writing the pack failed: {0}")]
    Write(#[source] std::io::Error),
}

/// What is known of a codec kind no reader reads.
fn codec_state(kind: u8) -> String {
    match kind {
        3..=7 => format!(
            "BC{kind} blocks through the BCPack coder, whose tables for BC{kind} are not known \
             until `openrage-cli ttpf tables <file>` reads them from a tables file you supply or \
             `openrage-cli ttpf calibrate <folder>` fits them on this machine from the packs of \
             a folder, so it does not decode"
        ),
        _ => "which no measured pack uses".into(),
    }
}

/// The 16-byte file header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Rows in the table, the directory and the sentinel included.
    pub row_count: u32,
    /// The decoded size of the directory and every body, which the sentinel's virtual address repeats.
    pub virtual_size: u64,
}

/// One table row: where a block starts in the file and in the decoded virtual space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// The file offset, a multiple of 0x100.
    pub offset: u64,
    /// The body's codec: [`KIND_STORED`], [`KIND_ZLIB`], the BC number of blocks through BCPack, or one not identified.
    pub kind: u8,
    /// Where the decoded body starts in the virtual space, a multiple of 4 KiB.
    pub virtual_address: u64,
    /// Zero bytes between the end of the stored data and the next row's offset.
    pub pad: u16,
}

/// What a row of the table is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowRole {
    /// Row 0: the `ETPF` directory, stored.
    Directory,
    /// A body: part of one texture's memory.
    Body,
    /// The last row: the file's full length and the virtual size.
    Sentinel,
}

/// One texture's 16-byte directory entry, and its key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// False when its 16 bytes are all zero, as a piece a download has not delivered reads.
    pub on_disk: bool,
    /// The 64-bit hash of the texture's lower-cased name; 0 where the key is not on disk.
    pub key: u64,
    pub width: u32,
    pub height: u32,
    /// The levels row 0 holds, or 0 when the chain has fewer pieces than the format code's flags plus one.
    pub mip_count: u8,
    /// The 10-bit format code: the DXGI number shifted left by 3, and flags.
    pub format_code: u16,
    pub slices: u16,
    /// 1 for a 2D texture, 2 for a cube of 6 slices, 3 for a volume, 5 for an array.
    pub dimension: u8,
    /// 0 for 64 KiB tiles, 1 to 3 for 4 KiB tiles.
    pub tile_mode: u8,
    /// The first 4 KiB page of the texture's virtual range.
    pub page: u32,
    /// The size code of each of the entry's first four rows, 0 where none is written.
    pub row_codes: [u8; 4],
    /// How many leading rows are under 64 KiB, written for three or four rows in 4 KiB tiles.
    pub small_rows: u8,
    /// The pages that would put the first row of 64 KiB or more on a 64 KiB boundary.
    pub align_pages: u16,
}

impl Entry {
    fn parse(bytes: &[u8], key: u64) -> Self {
        let size = u32_at(bytes, 0);
        let layout = u32_at(bytes, 4);
        let (w8, w12) = (u32_at(bytes, 8), u32_at(bytes, 12));
        let packed = u64::from(w8 >> 21) | u64::from(w12 & 0x1FF) << 11;
        Self {
            on_disk: bytes.iter().any(|&b| b != 0),
            key,
            width: (size & 0x3FFF) + 1,
            height: ((size >> 14) & 0x3FFF) + 1,
            mip_count: (size >> 28) as u8,
            format_code: (layout & 0x3FF) as u16,
            slices: ((layout >> 10) & 0x7FF) as u16 + 1,
            dimension: ((layout >> 21) & 7) as u8,
            tile_mode: ((layout >> 24) & 3) as u8,
            page: w8 & 0x1F_FFFF,
            row_codes: std::array::from_fn(|j| ((packed >> (5 * j)) & 0x1F) as u8),
            small_rows: ((w12 >> 19) & 3) as u8,
            align_pages: (w12 >> 21) as u16,
        }
    }

    /// The element format of the format code, if its DXGI number is a measured one.
    pub fn format(&self) -> Option<Format> {
        Format::of_code(self.format_code)
    }
}

/// The 4 KiB pages a row-size code states; `None` for a code no measured pack writes.
pub fn row_code_pages(code: u8) -> Option<u32> {
    let pages = match code {
        1..=8 => code.into(),
        10 => 12,
        11 => 16,
        12 => 24,
        13 => 32,
        14 => 48,
        15 => 64,
        16 => 128,
        21 => 18,
        22 => 80,
        23 => 96,
        24 => 144,
        26 => 272,
        27 => 384,
        28 => 528,
        _ => return None,
    };
    Some(pages)
}

/// A texel format: a DXGI number whose element is measured, and whose element's size fixes its tile order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format(DxgiFormat);

impl Format {
    /// The format of a pack's format code, whose DXGI number is the code shifted right by 3.
    pub fn of_code(code: u16) -> Option<Self> {
        u8::try_from(code >> 3).ok().and_then(Self::of_dxgi)
    }

    /// The format of DXGI number `number`, if its element is measured.
    pub fn of_dxgi(number: u8) -> Option<Self> {
        use DxgiFormat as D;
        match DxgiFormat::from_raw(number.into()) {
            dxgi @ (D::Bc1Unorm
            | D::Bc1UnormSrgb
            | D::Bc2Unorm
            | D::Bc3Unorm
            | D::Bc3UnormSrgb
            | D::Bc4Unorm
            | D::Bc5Unorm
            | D::Bc6hUf16
            | D::Bc6hSf16
            | D::Bc7Unorm
            | D::Bc7UnormSrgb
            | D::R8G8B8A8Unorm
            | D::R8G8B8A8UnormSrgb
            | D::B8G8R8A8Unorm
            | D::B8G8R8A8UnormSrgb
            | D::R16G16B16A16Float
            | D::R16G16B16A16Unorm
            | D::R32Float
            | D::R16Float
            | D::R16Unorm
            | D::R8Unorm
            | D::A8Unorm) => Some(Self(dxgi)),
            _ => None,
        }
    }

    /// The format of a volume of DXGI number `number`: a 2D texture's, or one measured in volumes only.
    pub(super) fn of_volume_dxgi(number: u8) -> Option<Self> {
        match DxgiFormat::from_raw(number.into()) {
            dxgi @ (DxgiFormat::R16G16Float | DxgiFormat::R8G8Unorm) => Some(Self(dxgi)),
            _ => Self::of_dxgi(number),
        }
    }

    pub fn dxgi(self) -> DxgiFormat {
        self.0
    }

    /// The DXGI name, as `Bc1UnormSrgb`.
    pub fn name(self) -> String {
        format!("{:?}", self.0)
    }

    pub fn is_srgb(self) -> bool {
        self.0.is_srgb()
    }

    /// The tree's format of the pixels read; a signed BC6H texture reads as the floats its blocks decode to.
    pub fn format(self) -> GrcTextureFormat {
        if self.0 == DxgiFormat::Bc6hSf16 {
            return GrcTextureFormat::A32B32G32R32F;
        }
        self.stored()
    }

    fn stored(self) -> GrcTextureFormat {
        GrcTextureFormat::from_dxgi(self.0).0
    }

    pub(super) fn element_bytes(self) -> usize {
        self.stored().bytes_per_block() as usize
    }

    /// Pixels along each side of one element.
    pub(super) fn element_side(self) -> u32 {
        self.stored().block_size()
    }
}

/// A texture read: every mip of its chain that is on disk, in each face of a cube or slice of an array.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub format: Format,
    /// The mips read in each face, top first.
    pub levels: u32,
    /// 6 per cube, in D3D's order +X, −X, +Y, −Y, +Z, −Z, cube after cube; the slices of an array or a volume, a volume's slice s holding slice `s >> m` of level m; 1 otherwise.
    pub faces: u32,
    /// Every mip read, as linear elements, largest first, face after face.
    pub pixels: Vec<u8>,
}

impl Texture {
    /// Face `face` of a cube, or of a 2D texture its only face, as a texture of one face.
    pub fn face(&self, face: u32) -> Option<Texture> {
        if face >= self.faces {
            return None;
        }
        let each = self.pixels.len() / self.faces as usize;
        let at = face as usize * each;
        Some(Texture {
            width: self.width,
            height: self.height,
            format: self.format,
            levels: self.levels,
            faces: 1,
            pixels: self.pixels.get(at..at + each)?.to_vec(),
        })
    }
}

/// A cube reads as its +X face, an array or a volume as its first slice.
impl TextureSource for Texture {
    fn pixel_format(&self) -> GrcTextureFormat {
        self.format.format()
    }

    fn is_srgb(&self) -> bool {
        self.format.is_srgb()
    }

    fn stored_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn stored_levels(&self) -> u32 {
        self.levels
    }

    fn payload(&self) -> &[u8] {
        &self.pixels[..self.pixels.len() / self.faces.max(1) as usize]
    }
}

/// An opened `.ttpf`: its header, table and directory, read without reading the bodies.
#[derive(Debug, Clone)]
pub struct TexturePack {
    path: PathBuf,
    on_disk: u64,
    sparse: bool,
    pub header: Header,
    pub rows: Vec<Row>,
    pub entries: Vec<Entry>,
    /// The row each entry's page lands on; `None` for an entry not on disk.
    first_rows: Vec<Option<usize>>,
    replaced: write::Replaced,
    /// The file's own table once a texture changed size, format or mips; `None` until then.
    moved: Option<Box<write::Moved>>,
    /// The coder's tables fitted on this machine, through which a body that does not decode exactly is drawn.
    calibration: Option<std::sync::Arc<bcpack::Tables>>,
}

impl TexturePack {
    /// Reads the header, the table and the directory of the pack at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, TtpfError> {
        let path = path.as_ref().to_path_buf();
        let mut file = File::open(&path).map_err(|source| TtpfError::Io {
            path: path.clone(),
            source,
        })?;
        let metadata = file.metadata().map_err(|source| TtpfError::Io {
            path: path.clone(),
            source,
        })?;
        let (on_disk, sparse) = (metadata.len(), is_sparse(&metadata));
        let present = |what, end| {
            if end > on_disk {
                Err(TtpfError::HeadNotOnDisk { what, end, on_disk })
            } else {
                Ok(())
            }
        };

        present("header", HEADER_LEN)?;
        let head = read_at(&mut file, &path, 0, HEADER_LEN)?;
        let magic = [head[0], head[1], head[2], head[3]];
        if head.iter().all(|&byte| byte == 0) {
            return Err(TtpfError::HeadIsZero);
        }
        if magic != MAGIC {
            return Err(TtpfError::NotATexturePack(magic));
        }
        if head[0x0D] != 0 || head[0x0E] != 0 {
            return Err(malformed(format!(
                "header bytes 0x0D and 0x0E are {:02X?}; only zero is measured, and whether they \
                 widen the virtual size is not known",
                &head[0x0D..0x0F]
            )));
        }
        // Byte 0x0C carries the virtual size from bit 32 up.
        let header = Header {
            row_count: u32_at(&head, 4),
            virtual_size: u64::from(u32_at(&head, 8)) | u64::from(head[0x0C]) << 32,
        };
        if header.row_count < 3 {
            return Err(malformed(format!(
                "{} rows cannot hold a directory, a body and the sentinel",
                header.row_count
            )));
        }

        let table_end = HEADER_LEN + ROW_LEN * u64::from(header.row_count);
        present("table", table_end)?;
        let table = read_at(&mut file, &path, HEADER_LEN, table_end - HEADER_LEN)?;
        let rows = table
            .as_chunks::<{ ROW_LEN as usize }>()
            .0
            .iter()
            .enumerate()
            .map(|(i, row)| parse_row(i, u32_at(row, 0), u32_at(row, 4)))
            .collect::<Result<Vec<_>, _>>()?;
        check_rows(&rows, &header, table_end, on_disk)?;

        let directory = rows[0].offset;
        // Row 0 is stored, so its decoded size is its length in the file less its pad.
        let directory_len = rows[1].virtual_address;
        present("directory", directory + directory_len)?;
        if directory_len < HEADER_LEN {
            return Err(malformed(format!("the directory is {directory_len} bytes")));
        }
        let dir_head = read_at(&mut file, &path, directory, HEADER_LEN)?;
        if dir_head[..4] != DIRECTORY_MAGIC {
            return Err(malformed("row 0 does not start with ETPF".into()));
        }
        let version = u32_at(&dir_head, 4);
        if version != DIRECTORY_VERSION {
            return Err(malformed(format!(
                "the directory's version is {version:#x}; only {DIRECTORY_VERSION:#x} is measured"
            )));
        }
        let count = u64::from(u32_at(&dir_head, 8));
        let pool = u64::from(u32_at(&dir_head, 12));
        if count > rows.len() as u64 - 2
            || HEADER_LEN + 2 * ENTRY_LEN * count + pool > directory_len
        {
            return Err(malformed(format!(
                "{count} entries and a {pool}-byte pool do not fit a {directory_len}-byte directory \
                 over {} bodies",
                rows.len() - 2
            )));
        }
        let listed = read_at(
            &mut file,
            &path,
            directory + HEADER_LEN,
            (ENTRY_LEN + KEY_LEN) * count,
        )?;
        let (entries, keys) = listed.split_at((ENTRY_LEN * count) as usize);
        let entries: Vec<Entry> = entries
            .as_chunks::<{ ENTRY_LEN as usize }>()
            .0
            .iter()
            .zip(keys.as_chunks::<{ KEY_LEN as usize }>().0)
            .map(|(entry, key)| Entry::parse(entry, u64::from_le_bytes(*key)))
            .collect();

        let bodies = &rows[1..rows.len() - 1];
        let mut first_rows: Vec<Option<usize>> = Vec::with_capacity(entries.len());
        let mut previous = 0;
        for (k, entry) in entries.iter().enumerate() {
            if !entry.on_disk {
                first_rows.push(None);
                continue;
            }
            let at = u64::from(entry.page) << 12;
            let row = bodies
                .binary_search_by_key(&at, |r| r.virtual_address)
                .map(|i| i + 1)
                .map_err(|_| {
                    malformed(format!(
                        "texture {k}'s page {:#x} is not where a body starts",
                        entry.page
                    ))
                })?;
            if previous >= row {
                return Err(malformed(format!(
                    "texture {k} does not start after the texture before it"
                )));
            }
            previous = row;
            first_rows.push(Some(row));
        }

        Ok(Self {
            path,
            on_disk,
            sparse,
            header,
            rows,
            entries,
            first_rows,
            replaced: write::Replaced::default(),
            moved: None,
            calibration: None,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The file's length when it was opened.
    pub fn size_on_disk(&self) -> u64 {
        self.on_disk
    }

    /// Whether the file is as long as its sentinel row states and not sparse; a download in progress can be shorter, or full length with pieces still missing.
    pub fn is_complete(&self) -> bool {
        let sentinel = self.file_rows().last();
        sentinel.is_some_and(|s| s.offset == self.on_disk) && !self.sparse
    }

    /// What row `row` is, or `None` past the table.
    pub fn role(&self, row: usize) -> Option<RowRole> {
        let last = self.rows.len() - 1;
        match row {
            0 => Some(RowRole::Directory),
            r if r < last => Some(RowRole::Body),
            r if r == last => Some(RowRole::Sentinel),
            _ => None,
        }
    }

    /// The rows that hold texture `index`'s memory, in virtual order; `None` where its entry or the next is not on disk.
    pub fn rows_of(&self, index: usize) -> Option<Range<usize>> {
        let first = (*self.first_rows.get(index)?)?;
        let end = match self.first_rows.get(index + 1) {
            Some(next) => (*next)?,
            None => self.rows.len() - 1,
        };
        Some(first..end)
    }

    /// Why texture `index` cannot be read, as far as the directory and the table tell without inflating a body.
    pub fn check(&self, index: usize) -> Result<(), TtpfError> {
        self.plan_as(index, true).map(|_| ())
    }

    /// The row of `rows` that holds byte `at` of a texture whose range starts at `start`.
    fn row_holding(&self, rows: &Range<usize>, start: u64, at: u64) -> usize {
        rows.start + self.rows[rows.clone()].partition_point(|r| r.virtual_address - start <= at)
            - 1
    }

    /// The decoded size of body `row`.
    fn size(&self, row: usize) -> u64 {
        self.rows[row + 1].virtual_address - self.rows[row].virtual_address
    }

    /// What reading texture `index` takes, once the rows its top mip needs are known to be readable.
    fn plan(&self, index: usize) -> Result<Plan, TtpfError> {
        self.plan_as(index, false)
    }

    /// [`Self::plan`], a row the pack's calibration covers being readable too when `fitted`.
    fn plan_as(&self, index: usize, fitted: bool) -> Result<Plan, TtpfError> {
        let plan = self.layout_of(index)?;
        let first = if plan.layout.slices > 1 || plan.layout.depth.is_some() {
            plan.rows.start
        } else {
            self.row_holding(&plan.rows, plan.start, plan.layout.levels[0].from())
        };
        for row in first..plan.rows.end {
            self.readable_as(row, fitted)?;
        }
        Ok(plan)
    }

    /// Where texture `index`'s levels lie in the rows of its range, whatever those rows' codecs.
    fn layout_of(&self, index: usize) -> Result<Plan, TtpfError> {
        let count = self.entries.len();
        let Some(entry) = self.entries.get(index) else {
            return Err(TtpfError::NoSuchTexture { index, count });
        };
        let rows = self
            .rows_of(index)
            .ok_or(TtpfError::RangeNotKnown { index })?;
        let format = entry.format().ok_or(TtpfError::UnknownFormat {
            index,
            code: entry.format_code,
        })?;
        let (cube, slices) = match (entry.dimension, entry.slices) {
            (1 | 5, 1) => (false, 1),
            (2, 6) => (true, 6),
            (5, slices) if entry.tile_mode == 0 => (false, slices),
            (3, depth) => (false, depth),
            (dimension, slices) => {
                return Err(TtpfError::UnknownShape {
                    index,
                    dimension,
                    slices,
                })
            }
        };
        let flags = entry.format_code & 7;
        if cube && format.element_side() == 1 && format.element_bytes() != 4 {
            let bytes = format.element_bytes();
            return Err(TtpfError::CubeNotMeasured { index, bytes });
        }
        if cube && rows.len() != usize::from(flags) + 1 {
            let rows = rows.len();
            return Err(TtpfError::CubeRows { index, rows, flags });
        }
        for (j, &code) in entry.row_codes.iter().enumerate() {
            let stated = row_code_pages(code).map(|pages| u64::from(pages) << 12);
            if code != 0
                && (j >= rows.len() || stated.is_some_and(|s| s != self.size(rows.start + j)))
            {
                return Err(malformed(format!(
                    "texture {index}'s row-size code {code} for its row {j} is not that row's size"
                )));
            }
        }
        // A cube's faces lie in its last row, in 4 KiB tiles whatever its tile mode.
        let rows = if cube { rows.end - 1..rows.end } else { rows };
        let tile = Tile::of(format, entry.tile_mode == 0 && !cube);
        let start = self.rows[rows.start].virtual_address;
        let len = self.rows[rows.end].virtual_address - start;
        // A signed BC6H block of 16 bytes reads as 16 pixels of four floats.
        let grows = if format.dxgi() == DxgiFormat::Bc6hSf16 {
            16
        } else {
            1
        };
        if grows * len > TEXTURE_LIMIT {
            return Err(malformed(format!(
                "texture {index}'s range of {len} bytes is more than the {TEXTURE_LIMIT} a texture \
                 is read into"
            )));
        }
        if entry.dimension == 3 {
            return self.volume(index, entry, rows, format);
        }
        let slices = u64::from(slices);
        let lone = entry.mip_count <= 1;
        // A cube's row holds its six faces, level 0 alone when its nibble is at most 1, else the
        // whole chain, then zero bytes up to the smallest size a row code states.
        let unit = if cube {
            let lengths = chain_lengths(entry.width, entry.height, format, &tile);
            let unit = if lone {
                lengths.first()
            } else {
                lengths.last()
            };
            unit.copied()
                .filter(|&unit| smallest_row_holding(6 * unit) == Some(len))
        } else {
            Some(len / slices).filter(|unit| unit * slices == len)
        };
        let mut levels = unit
            .and_then(|unit| chain(entry.width, entry.height, format, &tile, lone, unit))
            .ok_or_else(|| {
                malformed(format!(
                    "texture {index}'s {len} bytes are no {slices} mip chain(s) of its size"
                ))
            })?;
        let unit = unit.unwrap_or_default();
        if levels.is_empty() {
            return Err(TtpfError::TopMipInTail {
                index,
                format: format.name(),
            });
        }
        let stated = (!cube && slices == 1).then(|| held_levels(entry, flags, &levels, unit));
        if let Some(held) = stated {
            let explicit = levels.iter().filter(|l| !l.tail).count();
            if held < explicit {
                return Err(malformed(format!(
                    "texture {index}'s nibble counts {held} levels, and its range holds {explicit} \
                     outside its tail"
                )));
            }
            levels.truncate(held);
        }
        if cube {
            // Each face holds its lone level 0, or as many levels as its nibble and one more for
            // each row after the first; a zero tail slot among them is a black level.
            let held = if lone {
                1
            } else {
                usize::from(entry.mip_count) + usize::from(flags)
            };
            let explicit = levels.iter().filter(|l| !l.tail).count();
            if held < explicit || held > levels.len() {
                return Err(malformed(format!(
                    "texture {index}'s nibble and flags count {held} levels in each face, and its \
                     last row places {explicit} outside its tail and {} in all",
                    levels.len()
                )));
            }
            levels.truncate(held);
        }
        Ok(Plan {
            rows,
            start,
            len,
            pad_from: if cube { 6 * unit } else { len },
            held: stated,
            layout: Layout {
                format,
                width: entry.width,
                height: entry.height,
                tile,
                levels,
                slices,
                unit,
                piece_major: slices > 1 && !cube,
                stated: stated.is_some() || cube,
                depth: None,
            },
        })
    }

    /// Where volume `index`'s levels lie: its range is z-slabs of 4 KiB tiles, each holding the chain.
    fn volume(
        &self,
        index: usize,
        entry: &Entry,
        rows: Range<usize>,
        format: Format,
    ) -> Result<Plan, TtpfError> {
        let tile = Tile::volume(format)
            .filter(|_| entry.tile_mode == 1)
            .ok_or(TtpfError::Volume {
                index,
                tile_mode: entry.tile_mode,
            })?;
        let start = self.rows[rows.start].virtual_address;
        let len = self.rows[rows.end].virtual_address - start;
        let (width, height, depth) = (entry.width, entry.height, u32::from(entry.slices));
        let mips = entry.mip_count.max(1);
        let (levels, unit) = volume_chain([width, height, depth], format, &tile, mips.into(), len)
            .ok_or_else(|| {
                malformed(format!(
                    "texture {index}'s {len} bytes are not the z-slabs of a {width}x{height}x{depth} \
                     volume of {mips} level(s)"
                ))
            })?;
        Ok(Plan {
            rows,
            start,
            len,
            pad_from: len,
            held: None,
            layout: Layout {
                format,
                width,
                height,
                tile,
                levels,
                slices: depth.into(),
                unit,
                piece_major: false,
                stated: true,
                depth: Some(depth),
            },
        })
    }

    /// Texture `index`: every mip of its chain whose rows are readable, top first, in each face.
    pub fn texture(&self, index: usize) -> Result<Texture, TtpfError> {
        self.texture_noted(index).map(|(texture, _)| texture)
    }

    /// [`Self::texture`], and how its coded bodies were rebuilt: exactly, or through the pack's calibration.
    pub fn texture_noted(&self, index: usize) -> Result<(Texture, bcpack::Decode), TtpfError> {
        let Plan {
            rows,
            start,
            len,
            pad_from,
            layout,
            ..
        } = self.plan_as(index, true)?;
        let mut decode = bcpack::Decode::default();
        let mut memory = vec![0u8; len as usize];
        let mut file = File::open(&self.path).map_err(|source| TtpfError::Io {
            path: self.path.clone(),
            source,
        })?;
        // The bytes before this were not read.
        let mut read_from = 0;
        for row in rows.clone().rev() {
            let signed = layout.format.dxgi() == DxgiFormat::Bc6hSf16;
            match self.body_fitted(&mut file, row, signed, &mut decode) {
                Ok(body) => {
                    let at = (self.rows[row].virtual_address - start) as usize;
                    memory[at..at + body.len()].copy_from_slice(&body);
                }
                Err(e) => {
                    read_from = self.rows[row + 1].virtual_address - start;
                    if layout.slices > 1
                        || layout.depth.is_some()
                        || layout.levels[0].from() < read_from
                    {
                        return Err(e);
                    }
                    // The levels under the ones read are not said to be exact where a body of
                    // them is stored through the coder.
                    let coded = |row: &usize| {
                        let kind = self.rows[*row].kind;
                        kind != KIND_STORED
                            && kind != KIND_ZLIB
                            && self.replaced_row(*row).is_none()
                    };
                    decode.unread = (rows.start..=row).filter(coded).count() as u64;
                    break;
                }
            }
        }
        if memory[pad_from as usize..].iter().any(|&b| b != 0) {
            return Err(malformed(format!(
                "texture {index}'s last row holds bytes other than zero past its six faces"
            )));
        }
        let (levels, pixels) = layout.read(&memory, read_from).ok_or_else(|| {
            malformed(format!(
                "texture {index}'s top mip does not lie in its range"
            ))
        })?;
        let texture = Texture {
            width: layout.width,
            height: layout.height,
            format: layout.format,
            levels,
            faces: layout.slices as u32,
            pixels,
        };
        Ok((texture, decode))
    }

    /// Texture `index` from mip `from` down, or the largest mip under it whose rows decode, with which mip that is.
    pub fn texture_from(&self, index: usize, from: u32) -> Result<(u32, Texture), TtpfError> {
        let Plan {
            rows,
            start,
            layout,
            ..
        } = self.layout_of(index)?;
        // Only the rows the mips asked for lie in are read; a cube, an array or a volume reads whole.
        if layout.slices > 1 || layout.depth.is_some() {
            return self.texture(index).map(|texture| (0, texture));
        }
        // The chain holds its small levels first: a level and those under it are a prefix of it.
        let end_of = |level: &Level| level.from() + level.len;
        let mut readable = 0;
        for row in rows.clone() {
            if self.readable(row).is_err() {
                break;
            }
            readable = self.rows[row + 1].virtual_address - start;
        }
        let asked = (from as usize).min(layout.levels.len().saturating_sub(1));
        let first = (asked..layout.levels.len()).find(|&l| end_of(&layout.levels[l]) <= readable);
        let Some(first) = first else {
            // The rows of the small mips do not read, but those of the large ones may.
            return self.texture(index).map(|texture| (0, texture));
        };
        let last = self.row_holding(&rows, start, end_of(&layout.levels[first]) - 1);
        let mut memory = vec![0u8; (self.rows[last + 1].virtual_address - start) as usize];
        let mut file = File::open(&self.path).map_err(|source| TtpfError::Io {
            path: self.path.clone(),
            source,
        })?;
        for row in rows.start..=last {
            let body = self.body(&mut file, row)?;
            let at = (self.rows[row].virtual_address - start) as usize;
            memory[at..at + body.len()].copy_from_slice(&body);
        }
        let shift = |side: u32| (side >> first).max(1);
        let part = Layout {
            width: shift(layout.width),
            height: shift(layout.height),
            levels: layout.levels[first..].to_vec(),
            ..layout
        };
        let (levels, pixels) = part.read(&memory, 0).ok_or_else(|| {
            malformed(format!(
                "texture {index}'s mip {first} does not lie in its rows"
            ))
        })?;
        Ok((
            first as u32,
            Texture {
                width: part.width,
                height: part.height,
                format: part.format,
                levels,
                faces: 1,
                pixels,
            },
        ))
    }

    /// Whether body `row`'s codec is identified and its bytes are on disk.
    fn readable(&self, row: usize) -> Result<(), TtpfError> {
        self.readable_as(row, false)
    }

    /// [`Self::readable`], a codec the pack's calibration covers being identified too when `fitted`.
    fn readable_as(&self, row: usize, fitted: bool) -> Result<(), TtpfError> {
        if self.replaced_row(row).is_some() {
            return Ok(());
        }
        let this = self.rows[row];
        let covered =
            fitted && (self.calibration.as_deref()).is_some_and(|tables| tables.covers(this.kind));
        if this.kind != KIND_STORED
            && this.kind != KIND_ZLIB
            && !bcpack::decodes(this.kind)
            && !covered
        {
            return Err(TtpfError::UnidentifiedCodec {
                row,
                kind: this.kind,
            });
        }
        let (_, next) = self.file_span(row)?;
        if next.offset > self.on_disk {
            return Err(TtpfError::NotOnDisk {
                row,
                end: next.offset,
                on_disk: self.on_disk,
            });
        }
        Ok(())
    }

    /// Body `row`, decoded to its virtual size.
    fn body(&self, file: &mut File, row: usize) -> Result<Vec<u8>, TtpfError> {
        self.readable(row)?;
        let size = self.size(row);
        let read;
        let (kind, bytes) = match self.replaced_row(row) {
            Some(bytes) => (KIND_ZLIB, bytes),
            None => {
                read = self.stored(file, row)?;
                (self.rows[row].kind, read.as_slice())
            }
        };
        let stored = bytes.len() as u64;
        if kind == KIND_STORED {
            if stored != size {
                return Err(malformed(format!(
                    "stored body {row} holds {stored} bytes for a decoded size of {size}"
                )));
            }
            return Ok(bytes.to_vec());
        }
        if kind != KIND_ZLIB {
            return bcpack::decode(kind, bytes, size).map_err(|source| TtpfError::BcPack {
                row,
                kind,
                source,
            });
        }
        let mut decoder = flate2::read::ZlibDecoder::new(bytes);
        let mut out = Vec::new();
        let read = Read::by_ref(&mut decoder)
            .take(size + 1)
            .read_to_end(&mut out);
        if read.is_err() || out.len() as u64 != size || decoder.total_in() != stored {
            return Err(TtpfError::Inflate { row, size });
        }
        Ok(out)
    }

    /// [`Self::body`], a coded body through the tables the pack has set where it reads through
    /// them, and how its blocks were rebuilt counted in `decode`.
    ///
    /// ⚠ Only a read for showing or exporting takes this: what a save writes back goes through
    /// [`Self::body`], so an estimate is never written as if the pack stored it.
    fn body_fitted(
        &self,
        file: &mut File,
        row: usize,
        signed: bool,
        decode: &mut bcpack::Decode,
    ) -> Result<Vec<u8>, TtpfError> {
        let kind = self.rows[row].kind;
        if kind == KIND_STORED || kind == KIND_ZLIB || self.replaced_row(row).is_some() {
            return self.body(file, row);
        }
        self.readable_as(row, true)?;
        let stored = self.stored(file, row)?;
        let size = self.size(row);
        let tables = self.calibration.as_deref();
        let (blocks, how) = bcpack::decode_checked(kind, &stored, size, tables, signed)
            .map_err(|source| TtpfError::BcPack { row, kind, source })?;
        decode.add(how);
        Ok(blocks)
    }

    /// Row `row`'s bytes as the file stores them, whatever its codec.
    fn stored(&self, file: &mut File, row: usize) -> Result<Vec<u8>, TtpfError> {
        let (this, next) = self.file_span(row)?;
        let end = next.offset;
        if end > self.on_disk {
            return Err(TtpfError::NotOnDisk {
                row,
                end,
                on_disk: self.on_disk,
            });
        }
        read_at(file, &self.path, this.offset, self.stored_len(row)?)
    }

    /// How many bytes the file stores for row `row`: up to the next row, less the pad.
    fn stored_len(&self, row: usize) -> Result<u64, TtpfError> {
        let (this, next) = self.file_span(row)?;
        (next.offset - this.offset)
            .checked_sub(u64::from(this.pad))
            .filter(|&n| n > 0)
            .ok_or_else(|| {
                malformed(format!(
                    "body {row}'s pad of {} leaves it no stored bytes",
                    this.pad
                ))
            })
    }
}

fn malformed(why: String) -> TtpfError {
    TtpfError::Malformed(why)
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// `len` bytes at `at`; the caller has checked that they are on disk, which bounds `len`.
fn read_at(file: &mut File, path: &Path, at: u64, len: u64) -> Result<Vec<u8>, TtpfError> {
    let mut bytes = vec![0; len as usize];
    file.seek(SeekFrom::Start(at))
        .and_then(|_| file.read_exact(&mut bytes))
        .map_err(|source| TtpfError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(bytes)
}

fn parse_row(index: usize, lo: u32, hi: u32) -> Result<Row, TtpfError> {
    if lo & 0x0300_0000 != 0 {
        return Err(malformed(format!(
            "row {index} sets bits 24 and 25 of its first word, which are zero in every row measured"
        )));
    }
    let page = u64::from(hi & 0x000F_FFFF) << 2 | u64::from(lo >> 30);
    Ok(Row {
        offset: u64::from(lo & 0x00FF_FFFF) << 8,
        kind: ((lo >> 26) & 0xF) as u8,
        virtual_address: page << 12,
        pad: (hi >> 20) as u16,
    })
}

fn check_rows(
    rows: &[Row],
    header: &Header,
    table_end: u64,
    on_disk: u64,
) -> Result<(), TtpfError> {
    let (directory, sentinel, last_body) = (rows[0], rows[rows.len() - 1], rows[rows.len() - 2]);
    if directory.offset != table_end.next_multiple_of(0x100) {
        return Err(malformed(format!(
            "row 0 starts at {:#x}, not at the first 0x100 boundary after the table",
            directory.offset
        )));
    }
    if directory.kind != KIND_STORED || directory.virtual_address != 0 {
        return Err(malformed(
            "row 0 is not a stored directory at virtual address 0".into(),
        ));
    }
    if let Some(i) = rows.windows(2).position(|pair| {
        pair[1].offset <= pair[0].offset || pair[1].virtual_address <= pair[0].virtual_address
    }) {
        return Err(malformed(format!(
            "row {} does not start after row {i} in the file and in the virtual space",
            i + 1
        )));
    }
    if Some(rows[1].virtual_address)
        != (rows[1].offset - directory.offset).checked_sub(u64::from(directory.pad))
    {
        return Err(malformed(
            "the directory's virtual size is not its length in the file less its pad".into(),
        ));
    }
    if sentinel.virtual_address != header.virtual_size {
        return Err(malformed(format!(
            "the sentinel's virtual address {:#x} is not the header's virtual size {:#x}",
            sentinel.virtual_address, header.virtual_size
        )));
    }
    if (sentinel.kind, sentinel.pad) != (last_body.kind, last_body.pad) {
        return Err(malformed(
            "the sentinel does not repeat the last body's kind and pad".into(),
        ));
    }
    if on_disk > sentinel.offset {
        return Err(malformed(format!(
            "the file is {on_disk} bytes, longer than the {} its sentinel states",
            sentinel.offset
        )));
    }
    Ok(())
}

/// What a read of one texture takes: the rows of its range and where its slices lie in it.
struct Plan {
    rows: Range<usize>,
    start: u64,
    len: u64,
    /// Where the zero bytes after a cube's six faces start; the range's end for any other shape.
    pad_from: u64,
    /// The levels a 2D texture's nibble says its chain holds, which can be more than its tile's measured slots place.
    held: Option<usize>,
    layout: Layout,
}

/// The levels a 2D chain holds: row 0's, the nibble, and one for each other row, the rows being
/// as many as its pieces (its explicit levels and its tail tile), at most `flags` plus one.
fn held_levels(entry: &Entry, flags: u16, levels: &[Level], unit: u64) -> usize {
    let explicit: Vec<&Level> = levels.iter().filter(|l| !l.tail).collect();
    let tail = explicit.iter().map(|l| l.len).sum::<u64>() < unit;
    let pieces = explicit.len() + usize::from(tail);
    let rows = pieces.min(usize::from(flags) + 1);
    match usize::from(entry.mip_count) {
        0 => rows,
        nibble => nibble + rows - 1,
    }
}

/// The bytes of the smallest row a row-size code states that holds `bytes`.
fn smallest_row_holding(bytes: u64) -> Option<u64> {
    (1..32)
        .filter_map(row_code_pages)
        .map(|pages| u64::from(pages) << 12)
        .filter(|&row| row >= bytes)
        .min()
}

/// Whether the file is sparse, so that ranges of it may be unallocated and read as zeros.
fn is_sparse(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_SPARSE_FILE: u32 = 0x200;
        metadata.file_attributes() & FILE_ATTRIBUTE_SPARSE_FILE != 0
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.blocks().saturating_mul(512) < metadata.len()
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = metadata;
        false
    }
}

#[path = "ttpf_calibrate.rs"]
mod calibrate;
pub use calibrate::{
    calibrate_packs, calibrate_packs_from, coverage, keep_checked, probe_packs, score_packs,
    Calibrated, Coverage,
};

#[path = "ttpf_write.rs"]
mod write;
pub(super) use write::place;
pub use write::{Replaced, SaveLayout, Shape};

#[cfg(test)]
#[path = "ttpf_tests.rs"]
mod tests;
