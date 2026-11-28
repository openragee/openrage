//! One slice of a pack's cube, array or volume replaced in its own size, format and mips: its
//! levels placed where the reader finds them in the rows that hold the slices, every other slice's
//! bytes kept, so the directory, the keys and every row's size stay as they were.

use std::fs::File;

use super::super::{malformed, Plan, TexturePack, TtpfError};
use super::{place, zlib};
use crate::gta6::tiling::{place_planes, Layout, Level};

/// A texture's slices as planned, the memory its rows hold now, and how many levels a slice reads.
type Sliced = (Plan, Vec<u8>, usize);

impl TexturePack {
    /// Slice `slice` of texture `index` replaced from `image` in its own size, format and mips, which it returns: a cube's face, an array's slice, a one-level volume's z-plane, or a 2D texture's only slice.
    #[cfg(feature = "authoring")]
    pub fn replace_slice(
        &mut self,
        index: usize,
        slice: u32,
        image: &crate::codec::texture::RgbaImage,
        options: &crate::authoring::TextureBuildOptions,
    ) -> Result<u32, TtpfError> {
        if self.is_flat(index, slice)? {
            return self.replace(index, image, options);
        }
        let sliced = self.sliced(index, slice)?;
        let (format, kept) = (sliced.0.layout.format, sliced.2);
        if !crate::authoring::is_encodable(format.stored()) {
            return Err(TtpfError::ReplaceFormat {
                index,
                format: format.name(),
            });
        }
        let (width, height) = (sliced.0.layout.width, sliced.0.layout.height);
        if (image.width, image.height) != (width, height) {
            return Err(TtpfError::ImageSize {
                index,
                width,
                height,
                got: (image.width, image.height),
            });
        }
        let encode = |e: crate::Error| TtpfError::Encode(e.to_string());
        let chain = crate::codec::texture::generate_mip_chain_with(
            width,
            height,
            &image.data,
            Some(kept as u32),
            options.filter,
            options.srgb,
            options.alpha,
        )
        .map_err(encode)?;
        let levels = (chain.iter().enumerate())
            .map(|(level, rgba)| {
                let side = |pixels: u32| (pixels >> level).max(1);
                let (w, h) = (side(width), side(height));
                crate::authoring::encode_level(format.stored(), w, h, rgba, options.quality)
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(encode)?;
        self.put_slice(index, slice, sliced, &levels)?;
        Ok(kept as u32)
    }

    /// Slice `slice` of texture `index` replaced by `levels`, its mips top first as linear elements of its format, one for each mip the slice reads.
    pub fn replace_slice_levels(
        &mut self,
        index: usize,
        slice: u32,
        levels: &[Vec<u8>],
    ) -> Result<(), TtpfError> {
        if self.is_flat(index, slice)? {
            return self.replace_levels(index, levels);
        }
        let sliced = self.sliced(index, slice)?;
        self.put_slice(index, slice, sliced, levels)
    }

    /// How many mips a replacement of slice `slice` of texture `index` writes, once the writer covers it.
    pub fn replaced_mips(&self, index: usize, slice: u32) -> Result<usize, TtpfError> {
        if self.is_flat(index, slice)? {
            return Ok(self.writable(index)?.layout.levels.len());
        }
        self.sliced(index, slice).map(|(_, _, kept)| kept)
    }

    /// Whether texture `index` is a 2D texture of one slice, whose only slice is 0.
    fn is_flat(&self, index: usize, slice: u32) -> Result<bool, TtpfError> {
        let count = self.entries.len();
        let entry = (self.entries.get(index)).ok_or(TtpfError::NoSuchTexture { index, count })?;
        let flat = matches!((entry.dimension, entry.slices), (1 | 5, 1));
        if flat && slice > 0 {
            return Err(TtpfError::NoSuchSlice {
                index,
                slice,
                slices: 1,
            });
        }
        Ok(flat)
    }

    /// Where texture `index`'s slices lie, the memory its rows hold now, and how many levels a slice reads.
    fn sliced(&self, index: usize, slice: u32) -> Result<Sliced, TtpfError> {
        self.edits_rows()?;
        if !self.is_complete() {
            return Err(TtpfError::NotWhole);
        }
        let plan = self.plan(index)?;
        let slices = plan.layout.slices;
        if u64::from(slice) >= slices {
            return Err(TtpfError::NoSuchSlice {
                index,
                slice,
                slices: u32::try_from(slices).unwrap_or(u32::MAX),
            });
        }
        let mips = plan.layout.levels.len();
        if plan.layout.depth.is_some() && mips > 1 {
            return Err(TtpfError::VolumeLevels { index, mips });
        }
        let mut memory = vec![0u8; plan.len as usize];
        let mut file = File::open(&self.path).map_err(|source| TtpfError::Io {
            path: self.path.clone(),
            source,
        })?;
        for row in plan.rows.clone() {
            let body = self.body(&mut file, row)?;
            let at = (self.rows[row].virtual_address - plan.start) as usize;
            memory[at..at + body.len()].copy_from_slice(&body);
        }
        let (kept, _) = (plan.layout.read(&memory, 0)).ok_or_else(|| {
            malformed(format!(
                "texture {index}'s top mip does not lie in its range"
            ))
        })?;
        Ok((plan, memory, kept as usize))
    }

    /// Slice `slice`'s `levels` placed into the memory `sliced` holds, and the rows that hold it kept
    /// as zlib streams; the texture's other rows are left as the file stores them.
    fn put_slice(
        &mut self,
        index: usize,
        slice: u32,
        (plan, mut memory, kept): Sliced,
        levels: &[Vec<u8>],
    ) -> Result<(), TtpfError> {
        if levels.len() != kept {
            return Err(malformed(format!(
                "slice {slice} of texture {index} reads {kept} mips, not {}",
                levels.len()
            )));
        }
        let layout = &plan.layout;
        let element = layout.format.element_bytes();
        for (i, (level, elements)) in layout.levels.iter().zip(levels).enumerate() {
            let fits = elements.len() == level.width * level.height * element;
            if !fits || !placed(&mut memory, layout, level, slice.into(), elements) {
                return Err(malformed(format!(
                    "mip {i} of slice {slice} of texture {index}'s replacement is not {}x{} elements of its chain",
                    level.width, level.height
                )));
            }
        }
        let rows = (self.rows_of(index)).ok_or(TtpfError::RangeNotKnown { index })?;
        let bodies = rows
            .map(|row| {
                if !plan.rows.contains(&row) {
                    return Ok(self.replaced_row(row).map_or_else(Vec::new, <[u8]>::to_vec));
                }
                let at = (self.rows[row].virtual_address - plan.start) as usize;
                zlib(&memory[at..at + self.size(row) as usize])
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.replaced.0.insert(index, bodies);
        Ok(())
    }
}

/// `elements` of `level` placed as slice `slice` of `layout` into `memory`, where the reader reads
/// them; `false` where they pass it.
fn placed(memory: &mut [u8], layout: &Layout, level: &Level, slice: u64, elements: &[u8]) -> bool {
    let element = layout.format.element_bytes();
    if layout.depth.is_some() {
        // A one-level volume's slice is one z-plane of its z-slabs.
        let (size, z, tile) = ([level.width, level.height], slice as usize, &layout.tile);
        return place_planes(
            memory,
            level.at,
            size,
            z..z + 1,
            elements,
            tile,
            layout.unit,
        );
    }
    let (start, len) = (level.from(), level.len);
    let base = match layout.piece_major {
        true => layout.slices * start + slice * len,
        false => slice * layout.unit + start,
    };
    let local = Level {
        at: level.at - start,
        ..*level
    };
    let piece = (usize::try_from(base).ok())
        .and_then(|base| memory.get_mut(base..base.checked_add(len as usize)?));
    piece.is_some_and(|piece| place(piece, &local, elements, &layout.tile, element))
}
