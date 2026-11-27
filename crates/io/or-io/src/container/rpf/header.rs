use std::io::{Read, Write};

use crate::decl::*;
use crate::error::{IoError, Result};

pub const RPF7_MAGIC: u32 = 0x5250_4637;

pub const RPF_OPEN_MAGIC: u32 = 0x4E45_504F;

// AES key id used by retail GTA V PC builds.
pub const RPF_AES_MAGIC: u32 = 0x0FFF_FFF9;

// "NG" key id used by retail GTA V PC builds.
pub const RPF_NG_MAGIC: u32 = 0x0FEF_FFFF;

pub const RPF_FILE_ALIGNMENT: u32 = 1 << RPF_FILE_ALIGNMENT_SHIFT;

pub const RPF_FILE_ALIGNMENT_SHIFT: u32 = 9;

pub const RPF_DIRECTORY_OFFSET_MARKER: u32 = (1 << 23) - 1;

pub const RPF_MAX_RESOURCE_CONSUMED_SIZE: u32 = 0xFF_FFFF;

pub const RPF_NAME_OFFSET_MAX_SHIFT: u8 = 3;

pub const RPF_TRAILING_PAD: u32 = 2048;

pub const RPF_HEADER_SIZE: usize = 16;

// The header's encryption word: zero, `OPEN` (modded archives, plaintext TOC)
// or a key id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RpfEncryption {
    None,
    Open,
    // `0x0FFFFFF9` — retail GTA V "AES" mode (block-AES with the key every retail executable carries).
    Aes,
    // `0x0FEFFFFF` — retail GTA V "NG" mode (per-file key derivation).
    Ng,
    Other(u32),
    // The cipher of another title's archive (RPF2 to RPF6, IMG3, RPF8), picked by the header's
    // tag, or for an RPF8 body by the entry's key id with its config in the byte above; a
    // provider that holds none of those keys refuses it, which `Other` would not say.
    Tagged(u32),
}

impl RpfEncryption {
    pub fn from_raw(raw: u32) -> Self {
        match raw {
            0 => RpfEncryption::None,
            RPF_OPEN_MAGIC => RpfEncryption::Open,
            RPF_AES_MAGIC => RpfEncryption::Aes,
            RPF_NG_MAGIC => RpfEncryption::Ng,
            other => RpfEncryption::Other(other),
        }
    }

    pub fn to_raw(self) -> u32 {
        match self {
            RpfEncryption::None => 0,
            RpfEncryption::Open => RPF_OPEN_MAGIC,
            RpfEncryption::Aes => RPF_AES_MAGIC,
            RpfEncryption::Ng => RPF_NG_MAGIC,
            RpfEncryption::Other(raw) | RpfEncryption::Tagged(raw) => raw,
        }
    }

    pub fn needs_crypto(self) -> bool {
        !matches!(self, RpfEncryption::None | RpfEncryption::Open)
    }
}

const NAME_HEAP_SIZE_MASK: u32 = 0x0FFF_FFFF;

/// The sixteen bytes an RPF7 archive starts with, never encrypted whatever `encrypted` says.
#[record(size = 0x10)]
pub struct FiPackHeader {
    pub magic: u32,
    entry_count: u32,
    #[bits(name_shift, 28, 3, "each name offset is shifted left by this")]
    #[bits(is_x_compressed, 31, 1)]
    name_heap_size: u32,
    pub encrypted: u32,
}

impl FiPackHeader {
    fn name_heap_word(&self) -> u32 {
        self.__carry
            .hidden::<u32>(std::mem::offset_of!(Raw<Self>, name_heap_size))
            .copied()
            .unwrap_or(0)
    }

    pub fn name_shift(&self) -> u8 {
        ((self.name_heap_word() >> 28) & 0x7) as u8
    }

    pub fn is_x_compressed(&self) -> bool {
        self.name_heap_word() & 0x8000_0000 != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RpfHeader {
    pub entry_count: u32,
    pub name_heap_size: u32,
    pub name_shift: u8,
    pub xcompress: bool,
    pub encryption: RpfEncryption,
    /// The build-platform letter the archive records in its own header, where its format stores
    /// one — RPF8 keeps it at header offset `0x0A` (`y` PC, `z` Xbox Series, `x` Xbox 360, …).
    /// `None` for every format that does not record it (RPF7 and earlier, RPF9, IMG), for which
    /// the platform has to come from the install the archive was mounted under.
    pub platform: Option<u8>,
}

impl RpfHeader {
    pub fn read<R: Read>(reader: &mut R) -> Result<Self> {
        let mut block = [0u8; RPF_HEADER_SIZE];
        reader.read_exact(&mut block)?;
        let raw = Raw::<FiPackHeader>::read_from(&block).ok_or_else(|| IoError::RpfBadHeader {
            message: "shorter than its sixteen bytes".into(),
        })?;
        if raw.magic != RPF7_MAGIC {
            return Err(IoError::RpfBadMagic { found: raw.magic });
        }
        let name_heap_packed = raw.name_heap_size;

        let name_shift = ((name_heap_packed >> 28) & 0x3) as u8;
        let xcompress = (name_heap_packed & 0x8000_0000) != 0;
        let name_heap_size = name_heap_packed & NAME_HEAP_SIZE_MASK;

        if name_shift > RPF_NAME_OFFSET_MAX_SHIFT {
            return Err(IoError::RpfBadHeader {
                message: format!(
                    "name_shift {} exceeds max {}",
                    name_shift, RPF_NAME_OFFSET_MAX_SHIFT
                ),
            });
        }
        if name_heap_size & 0xF != 0 {
            return Err(IoError::RpfBadHeader {
                message: format!("name_heap_size {} not 16-byte aligned", name_heap_size),
            });
        }

        Ok(Self {
            entry_count: raw.entry_count,
            name_heap_size,
            name_shift,
            xcompress,
            encryption: RpfEncryption::from_raw(raw.encrypted),
            // RPF7 does not record a platform; its reader leaves this `None`.
            platform: None,
        })
    }

    pub fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
        if self.name_shift > RPF_NAME_OFFSET_MAX_SHIFT {
            return Err(IoError::RpfBadHeader {
                message: format!(
                    "name_shift {} exceeds max {}",
                    self.name_shift, RPF_NAME_OFFSET_MAX_SHIFT
                ),
            });
        }
        if self.name_heap_size & 0xF != 0 {
            return Err(IoError::RpfBadHeader {
                message: format!("name_heap_size {} not 16-byte aligned", self.name_heap_size),
            });
        }
        let mut packed = self.name_heap_size & NAME_HEAP_SIZE_MASK;
        packed |= ((self.name_shift as u32) & 0x3) << 28;
        if self.xcompress {
            packed |= 0x8000_0000;
        }

        let mut raw = <Raw<FiPackHeader> as Plain>::zeroed();
        raw.magic = RPF7_MAGIC;
        raw.entry_count = self.entry_count;
        raw.name_heap_size = packed;
        raw.encrypted = self.encryption.to_raw();
        writer.write_all(Plain::as_bytes(&raw))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn header_roundtrip_open() {
        let h = RpfHeader {
            entry_count: 17,
            name_heap_size: 0x40,
            name_shift: 0,
            xcompress: false,
            encryption: RpfEncryption::Open,
            platform: None,
        };
        let mut buf = Vec::new();
        h.write(&mut buf).unwrap();
        assert_eq!(buf.len(), RPF_HEADER_SIZE);

        let mut cursor = Cursor::new(&buf);
        let parsed = RpfHeader::read(&mut cursor).unwrap();
        assert_eq!(parsed, h);
    }

    #[test]
    fn header_roundtrip_with_shift_and_xcompress() {
        let h = RpfHeader {
            entry_count: 1234,
            name_heap_size: 0x4_0000,
            name_shift: 2,
            xcompress: true,
            encryption: RpfEncryption::None,
            platform: None,
        };
        let mut buf = Vec::new();
        h.write(&mut buf).unwrap();

        let mut cursor = Cursor::new(&buf);
        let parsed = RpfHeader::read(&mut cursor).unwrap();
        assert_eq!(parsed, h);
    }

    #[test]
    fn header_rejects_bad_magic() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        let mut cursor = Cursor::new(&buf);
        let err = RpfHeader::read(&mut cursor).unwrap_err();
        assert!(matches!(err, IoError::RpfBadMagic { .. }));
    }

    #[test]
    fn encryption_roundtrip_other() {
        let raw = 0x1234_5678u32;
        let mode = RpfEncryption::from_raw(raw);
        assert!(matches!(mode, RpfEncryption::Other(0x1234_5678)));
        assert_eq!(mode.to_raw(), raw);
    }

    #[test]
    fn encryption_decodes_known_ng_magic() {
        let mode = RpfEncryption::from_raw(RPF_NG_MAGIC);
        assert!(matches!(mode, RpfEncryption::Ng));
        assert_eq!(mode.to_raw(), 0x0FEF_FFFF);
        assert!(mode.needs_crypto());
    }
}
