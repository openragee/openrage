//! The tables of every version but RPF7: what each header and row says, as the entries and names the rest of the module reads.
//!
//! A layout that lists files with no directories gets a root directory at entry 0, so a reader
//! walks every archive the same way.

mod classic;
mod img;
mod rpf6;
mod rpf8;

use std::io::{Seek, Write};

use super::archive::RpfSource;
use super::crypto::{CryptoTarget, RpfCryptoProvider, TocDecryptOutcome};
use super::entry::{RpfDirectoryEntry, RpfEntry};
use super::header::{RpfEncryption, RpfHeader};
use super::reader::read_at;
use super::version::RpfVersion;
use crate::error::{IoError, Result};
use crate::hash::string_hash;

pub(super) use img::{read_img1, write_img1};
pub(super) use rpf8::{
    hash_of as rpf8_hash, is_encrypted as rpf8_is_encrypted,
    resource_from_file as rpf8_resource_from_file, resource_header as rpf8_resource_header,
    CLEAR as RPF8_CLEAR,
};

/// A table read: the entries, each one's name, and the header they came with.
pub(super) struct Toc {
    pub header: RpfHeader,
    pub entries: Vec<RpfEntry>,
    pub names: Vec<String>,
    pub name_heap: Vec<u8>,
    pub outcome: TocDecryptOutcome,
}

/// An archive being read: where its bytes are and what may decrypt them.
pub(super) struct Input<'a> {
    pub source: &'a RpfSource,
    pub start: u64,
    pub size: u64,
    pub name: Option<&'a str>,
    pub crypto: &'a dyn RpfCryptoProvider,
}

impl Input<'_> {
    /// `len` bytes at `at`, refused before anything is allocated when they run past the archive's end.
    pub fn bytes(&self, at: u64, len: u64) -> Result<Vec<u8>> {
        if at.checked_add(len).is_none_or(|end| end > self.size) {
            return Err(IoError::UnexpectedEof {
                offset: at,
                needed: usize::try_from(len).unwrap_or(usize::MAX),
                available: usize::try_from(self.size.saturating_sub(at)).unwrap_or(usize::MAX),
            });
        }
        let mut buf = vec![0u8; len as usize];
        read_at(self.source, self.start + at, &mut buf)?;
        Ok(buf)
    }

    /// Decrypt a table and the names beside it, which the header tags with `tag`: only a provider holding the title's key can.
    fn unseal(
        &self,
        version: RpfVersion,
        tag: u32,
        rows: &mut [u8],
        names: &mut [u8],
    ) -> Result<(RpfEncryption, TocDecryptOutcome)> {
        let mode = version.mode_of(tag);
        if !mode.needs_crypto() {
            return Ok((mode, TocDecryptOutcome::Nominal));
        }
        if !self.crypto.supports(mode) {
            return Err(IoError::RpfEncryptionUnsupported { key_id: tag });
        }
        let target = CryptoTarget::Toc {
            archive_name: self.name,
            archive_size: u32::try_from(self.size).unwrap_or(u32::MAX),
        };
        let outcome = self
            .crypto
            .decrypt_toc(mode, target.clone(), rows, &|_| true)?;
        self.crypto.decrypt_names(mode, target, names, outcome)?;
        Ok((mode, outcome))
    }
}

/// What a table is written from: entries with their final offsets, and the names the builder interned.
pub(super) struct Plan<'a> {
    pub entries: &'a [RpfEntry],
    /// One per entry: the file's name in a tree, its whole path in a flat table.
    pub names: &'a [String],
    /// The builder's name heap, which the text-named tables hold as they are.
    pub name_heap: &'a [u8],
    /// Where the last body ends, which is where anything after the bodies starts.
    pub body_end: u64,
    pub encryption: RpfEncryption,
}

pub(super) fn read(version: RpfVersion, input: &Input) -> Result<Toc> {
    match version {
        RpfVersion::Rpf0 | RpfVersion::Rpf2 | RpfVersion::Rpf3 | RpfVersion::Rpf4 => {
            classic::read(version, input)
        }
        RpfVersion::Rpf6 => rpf6::read(input),
        RpfVersion::Rpf8 => rpf8::read(input),
        RpfVersion::Img2 | RpfVersion::Img3 => img::read(version, input),
        RpfVersion::Rpf7 | RpfVersion::Rpf9 | RpfVersion::Img1 => Err(IoError::RpfBadHeader {
            message: format!("{version} is not read as a table of this layout"),
        }),
    }
}

/// How many bytes precede the first body of an archive listing `names`, with a name heap of `heap` bytes.
pub(super) fn prologue(version: RpfVersion, names: &[String], heap: usize) -> u64 {
    match version {
        RpfVersion::Rpf0 | RpfVersion::Rpf2 | RpfVersion::Rpf3 | RpfVersion::Rpf4 => {
            classic::prologue(names.len(), heap)
        }
        RpfVersion::Rpf6 => rpf6::prologue(names.len()),
        RpfVersion::Rpf8 => rpf8::prologue(names.len()),
        RpfVersion::Img2 | RpfVersion::Img3 => img::prologue(
            version,
            names.len(),
            names.iter().map(|n| n.len() + 1).sum(),
        ),
        RpfVersion::Rpf7 | RpfVersion::Rpf9 | RpfVersion::Img1 => 0,
    }
}

/// Write everything that is not a body: the tables, and what follows the bodies.
pub(super) fn write<W: Write + Seek>(
    version: RpfVersion,
    w: &mut W,
    plan: &Plan,
    crypto: &dyn RpfCryptoProvider,
) -> Result<()> {
    match version {
        RpfVersion::Rpf0 | RpfVersion::Rpf2 | RpfVersion::Rpf3 | RpfVersion::Rpf4 => {
            classic::write(version, w, plan, crypto)
        }
        RpfVersion::Rpf6 => rpf6::write(w, plan, crypto),
        RpfVersion::Rpf8 => rpf8::write(w, plan, crypto),
        RpfVersion::Img2 | RpfVersion::Img3 => img::write(version, w, plan),
        RpfVersion::Rpf7 | RpfVersion::Rpf9 | RpfVersion::Img1 => Err(IoError::RpfBadHeader {
            message: format!("{version} is not written as a table of this layout"),
        }),
    }
}

/// A hash's name where the archive keeps no text.
pub(super) fn hash_name(hash: u32) -> String {
    format!("hash_{hash:08X}")
}

/// The hash a name stands for: `hash_XXXXXXXX` read back, or the text hashed.
pub(super) fn name_hash(name: &str) -> u32 {
    name.strip_prefix("hash_")
        .and_then(|hex| u32::from_str_radix(hex.split('.').next()?, 16).ok())
        .unwrap_or_else(|| string_hash(name.as_bytes()))
}

/// A root directory over the `files` that follow it in a flat table.
pub(super) fn root_over(files: usize) -> RpfEntry {
    RpfEntry::Directory(RpfDirectoryEntry {
        name_offset: 0,
        first_entry_index: 1,
        entry_count: files as u32,
    })
}

/// `len` zero bytes, with no allocation.
pub(super) fn zeros<W: Write>(w: &mut W, mut len: u64) -> Result<()> {
    const ZEROS: [u8; 4096] = [0; 4096];
    while len > 0 {
        let n = len.min(ZEROS.len() as u64) as usize;
        w.write_all(&ZEROS[..n])?;
        len -= n as u64;
    }
    Ok(())
}

/// Zero bytes from the writer's position up to `at`.
pub(super) fn pad_to<W: Write + Seek>(w: &mut W, at: u64) -> Result<()> {
    let here = w.stream_position()?;
    zeros(w, at.saturating_sub(here))
}

pub(super) fn bad(what: impl Into<String>) -> IoError {
    IoError::RpfBadEntry {
        message: what.into(),
    }
}
