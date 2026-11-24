//! A pack texture replaced from an image file: any import format decoded, resampled to the
//! texture's size only when asked, and encoded into its own format with its mips made from the top
//! one; or a DDS of the texture's own format and size whose mips are copied block for block. The
//! entry, its key and every other row stay as they were, so a save is the same dictionary.

use std::path::{Path, PathBuf};

use super::bcpack;
use super::ttpf::{Format, TexturePack, TtpfError};
use crate::authoring::texture_import::{self, SourceFormat};
use crate::authoring::TextureBuildOptions;
use crate::codec::texture::{format_of_dds, resize_rgba, MipFilter, RgbaImage};
use crate::dds::Dds;

/// How an image replaces a texture.
#[derive(Debug, Clone, Default)]
pub struct Replace {
    pub build: TextureBuildOptions,
    /// Resample an image of another size to the texture's, rather than refuse it.
    pub resize: bool,
    /// Copy a DDS's own mips block for block, rather than make them from its top mip.
    pub dds_mips: bool,
}

/// Why an image does not replace a texture.
#[derive(Debug, thiserror::Error)]
pub enum AuthorError {
    #[error(
        "texture {index} is stored through the BCPack coder as BC{kind} blocks (codec kind \
         {kind}), which OpenRage does not encode, so it is not replaced"
    )]
    BcPack { index: usize, kind: u8 },
    #[error(
        "the image is {}x{}, and texture {index} is {width}x{height}: a replacement keeps the \
         texture's size unless the image is resampled to it", got.0, got.1
    )]
    ImageSize {
        index: usize,
        width: u32,
        height: u32,
        got: (u32, u32),
    },
    #[error("texture {index}'s mips are not copied from this image: {why}")]
    DdsMips { index: usize, why: String },
    #[error("{}: {source}", path.display())]
    Image {
        path: PathBuf,
        #[source]
        source: crate::Error,
    },
    #[error(transparent)]
    Ttpf(#[from] TtpfError),
}

/// The BCPack codec kind of the first row of texture `index` stored through that coder, if any.
pub fn bcpack_kind(pack: &TexturePack, index: usize) -> Option<u8> {
    let rows = pack.rows_of(index)?;
    (pack.rows[rows].iter())
        .map(|row| row.kind)
        .find(|&kind| bcpack::lane_widths(kind).is_some())
}

/// Slice `slice` of texture `index` replaced from the image at `image` as `how` says; the mips it holds.
pub fn replace_from_image(
    pack: &mut TexturePack,
    index: usize,
    slice: u32,
    image: &Path,
    how: &Replace,
) -> Result<u32, AuthorError> {
    let count = pack.entries.len();
    let entry = *(pack.entries.get(index)).ok_or(TtpfError::NoSuchTexture { index, count })?;
    if let Some(kind) = bcpack_kind(pack, index) {
        return Err(AuthorError::BcPack { index, kind });
    }
    let (width, height) = (entry.width, entry.height);
    let failed = |source| AuthorError::Image {
        path: image.to_path_buf(),
        source,
    };
    if how.dds_mips {
        let format = entry.format().ok_or(TtpfError::UnknownFormat {
            index,
            code: entry.format_code,
        })?;
        let mips = pack.replaced_mips(index, slice)?;
        let levels = dds_levels(image, (width, height), format, mips)
            .map_err(|why| AuthorError::DdsMips { index, why })?;
        pack.replace_slice_levels(index, slice, &levels)?;
        return Ok(mips as u32);
    }
    let mut rgba = texture_import::load(image).map_err(failed)?;
    if (rgba.width, rgba.height) != (width, height) {
        if !how.resize {
            return Err(AuthorError::ImageSize {
                index,
                width,
                height,
                got: (rgba.width, rgba.height),
            });
        }
        let from = (rgba.width, rgba.height);
        let data = resize_rgba(
            &rgba.data,
            from,
            (width, height),
            MipFilter::Lanczos3,
            how.build.srgb,
        )
        .map_err(failed)?;
        rgba = RgbaImage {
            width,
            height,
            data,
            ..rgba
        };
    }
    Ok(pack.replace_slice(index, slice, &rgba, &how.build)?)
}

/// The first `mips` levels of the DDS at `path`, as linear elements, once it is one surface of `size` in `format`.
fn dds_levels(
    path: &Path,
    size: (u32, u32),
    format: Format,
    mips: usize,
) -> Result<Vec<Vec<u8>>, String> {
    if SourceFormat::of_path(path) != Some(SourceFormat::Dds) {
        return Err("only a DDS holds mips of its own to copy".into());
    }
    let dds = Dds::open(path).map_err(|e| e.to_string())?;
    if !dds.is_one_surface() {
        return Err("the DDS is a cube, a volume or an array, not one surface".into());
    }
    let (grc, _) = format_of_dds(&dds).map_err(|e| e.to_string())?;
    // A legacy header states no colour space, and the blocks are the same either way.
    if Format::of_grc(grc, format.is_srgb()) != Some(format) {
        return Err(format!(
            "the DDS is {grc:?}, and the texture is {}",
            format.name()
        ));
    }
    let got = (dds.header.width, dds.header.height);
    if got != size {
        return Err(format!(
            "the DDS is {}x{}, and the texture is {}x{}",
            got.0, got.1, size.0, size.1
        ));
    }
    let held = dds.header.mip_map_count.max(1) as usize;
    if held < mips {
        return Err(format!(
            "the DDS holds {held} mip(s), and the texture's chain {mips}"
        ));
    }
    let (side, bytes) = (grc.block_size(), grc.bytes_per_block() as usize);
    let mut at = 0;
    let mut levels = Vec::with_capacity(mips);
    for level in 0..mips {
        let elements = |pixels: u32| (pixels >> level).max(1).div_ceil(side) as usize;
        let len = elements(size.0) * elements(size.1) * bytes;
        let data = (dds.data.get(at..at + len))
            .ok_or_else(|| format!("the DDS ends inside mip {level}"))?;
        levels.push(data.to_vec());
        at += len;
    }
    Ok(levels)
}
