use crate::container::rpf::header::RpfEncryption;
use crate::error::Result;

// What is being decrypted: modes such as retail NG key on the per-file name
// and size.
#[derive(Debug, Clone)]
pub enum CryptoTarget<'a> {
    Toc {
        archive_name: Option<&'a str>,
        archive_size: u32,
    },
    Body {
        entry_name: &'a str,
        keyed_size: u32,
        // The unit the game reads an RPF8 body in, which its cipher's ranges depend on; zero elsewhere.
        chunk: u64,
    },
}

// Whether the TOC decrypted under its nominal (filename-derived) NG key or
// needed a brute-force search over the key indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TocDecryptOutcome {
    Nominal,
    RecoveredNg { ng_key_index: usize },
}

pub trait RpfCryptoProvider: Send + Sync {
    fn supports(&self, encryption: RpfEncryption) -> bool;

    fn decrypt(
        &self,
        encryption: RpfEncryption,
        target: CryptoTarget<'_>,
        buffer: &mut [u8],
    ) -> Result<()>;

    fn encrypt(
        &self,
        encryption: RpfEncryption,
        target: CryptoTarget<'_>,
        buffer: &mut [u8],
    ) -> Result<()>;

    // Decrypt the TOC. A provider may fall back to a brute-force NG key
    // search, judged by `validate_first_block` on the first 16 bytes of
    // `buffer` (one NG block, which is exactly one [`super::super::entry::RpfEntry`]).
    fn decrypt_toc(
        &self,
        encryption: RpfEncryption,
        target: CryptoTarget<'_>,
        buffer: &mut [u8],
        _validate_first_block: &dyn Fn(&[u8; 16]) -> bool,
    ) -> Result<TocDecryptOutcome> {
        self.decrypt(encryption, target, buffer)?;
        Ok(TocDecryptOutcome::Nominal)
    }

    /// The key ids archives asked this provider for and it had no cipher for, in order.
    fn missing_keys(&self) -> Vec<u32> {
        Vec::new()
    }

    /// The GTA VI key-and-name session an RPF9 open reads its enciphered table with, where this
    /// provider carries one; `None` for a provider holding no session, so the archive opens by scan.
    #[cfg(feature = "gta6")]
    fn gta6_session(&self) -> Option<std::sync::Arc<crate::container::rpf::Gta6Context>> {
        None
    }

    // Decrypt the name heap under the key the TOC decrypted with. The default
    // ignores `toc_outcome`, which is correct for AES, OPEN and NoOp, where
    // there is only one nominal key.
    fn decrypt_names(
        &self,
        encryption: RpfEncryption,
        target: CryptoTarget<'_>,
        buffer: &mut [u8],
        _toc_outcome: TocDecryptOutcome,
    ) -> Result<()> {
        self.decrypt(encryption, target, buffer)
    }
}

// ⚠ Forwards every method, the overrides included: the retail provider's
// `decrypt_toc` picks the NG key, and the trait default would not.
impl<T: RpfCryptoProvider + ?Sized> RpfCryptoProvider for std::sync::Arc<T> {
    fn missing_keys(&self) -> Vec<u32> {
        (**self).missing_keys()
    }

    #[cfg(feature = "gta6")]
    fn gta6_session(&self) -> Option<std::sync::Arc<crate::container::rpf::Gta6Context>> {
        (**self).gta6_session()
    }

    fn supports(&self, encryption: RpfEncryption) -> bool {
        (**self).supports(encryption)
    }

    fn decrypt(
        &self,
        encryption: RpfEncryption,
        target: CryptoTarget<'_>,
        buffer: &mut [u8],
    ) -> Result<()> {
        (**self).decrypt(encryption, target, buffer)
    }

    fn encrypt(
        &self,
        encryption: RpfEncryption,
        target: CryptoTarget<'_>,
        buffer: &mut [u8],
    ) -> Result<()> {
        (**self).encrypt(encryption, target, buffer)
    }

    fn decrypt_toc(
        &self,
        encryption: RpfEncryption,
        target: CryptoTarget<'_>,
        buffer: &mut [u8],
        validate_first_block: &dyn Fn(&[u8; 16]) -> bool,
    ) -> Result<TocDecryptOutcome> {
        (**self).decrypt_toc(encryption, target, buffer, validate_first_block)
    }

    fn decrypt_names(
        &self,
        encryption: RpfEncryption,
        target: CryptoTarget<'_>,
        buffer: &mut [u8],
        toc_outcome: TocDecryptOutcome,
    ) -> Result<()> {
        (**self).decrypt_names(encryption, target, buffer, toc_outcome)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoOpCryptoProvider;

impl NoOpCryptoProvider {
    pub const fn new() -> Self {
        Self
    }
}

impl RpfCryptoProvider for NoOpCryptoProvider {
    fn supports(&self, encryption: RpfEncryption) -> bool {
        !encryption.needs_crypto()
    }

    fn decrypt(
        &self,
        encryption: RpfEncryption,
        _target: CryptoTarget<'_>,
        _buffer: &mut [u8],
    ) -> Result<()> {
        if !self.supports(encryption) {
            return Err(crate::error::IoError::RpfEncryptionUnsupported {
                key_id: encryption.to_raw(),
            });
        }
        Ok(())
    }

    fn encrypt(
        &self,
        encryption: RpfEncryption,
        _target: CryptoTarget<'_>,
        _buffer: &mut [u8],
    ) -> Result<()> {
        if !self.supports(encryption) {
            return Err(crate::error::IoError::RpfEncryptionUnsupported {
                key_id: encryption.to_raw(),
            });
        }
        Ok(())
    }
}

pub fn default_provider() -> &'static dyn RpfCryptoProvider {
    static PROVIDER: NoOpCryptoProvider = NoOpCryptoProvider::new();
    &PROVIDER
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_supports_open_and_none() {
        let p = NoOpCryptoProvider::new();
        assert!(p.supports(RpfEncryption::None));
        assert!(p.supports(RpfEncryption::Open));
        assert!(!p.supports(RpfEncryption::Aes));
        assert!(!p.supports(RpfEncryption::Other(0xCAFE_BABE)));
    }

    #[test]
    fn noop_passthrough_open() {
        let p = NoOpCryptoProvider::new();
        let original: Vec<u8> = (0..32).collect();
        let mut buf = original.clone();
        p.decrypt(
            RpfEncryption::Open,
            CryptoTarget::Toc {
                archive_name: Some("test.rpf"),
                archive_size: 4096,
            },
            &mut buf,
        )
        .unwrap();
        assert_eq!(buf, original);
    }

    #[test]
    fn an_arc_forwards_an_overridden_decrypt_toc() {
        struct Recovering;
        impl RpfCryptoProvider for Recovering {
            fn supports(&self, _: RpfEncryption) -> bool {
                true
            }
            fn decrypt(&self, _: RpfEncryption, _: CryptoTarget<'_>, _: &mut [u8]) -> Result<()> {
                Ok(())
            }
            fn encrypt(&self, _: RpfEncryption, _: CryptoTarget<'_>, _: &mut [u8]) -> Result<()> {
                Ok(())
            }
            fn decrypt_toc(
                &self,
                _: RpfEncryption,
                _: CryptoTarget<'_>,
                _: &mut [u8],
                _: &dyn Fn(&[u8; 16]) -> bool,
            ) -> Result<TocDecryptOutcome> {
                Ok(TocDecryptOutcome::RecoveredNg { ng_key_index: 7 })
            }
        }

        let shared: std::sync::Arc<dyn RpfCryptoProvider> = std::sync::Arc::new(Recovering);
        let boxed: Box<dyn RpfCryptoProvider> = Box::new(std::sync::Arc::clone(&shared));
        let outcome = boxed
            .decrypt_toc(
                RpfEncryption::Ng,
                CryptoTarget::Toc {
                    archive_name: None,
                    archive_size: 0,
                },
                &mut [0u8; 16],
                &|_| true,
            )
            .unwrap();
        assert_eq!(outcome, TocDecryptOutcome::RecoveredNg { ng_key_index: 7 });
    }

    #[test]
    fn noop_rejects_aes() {
        let p = NoOpCryptoProvider::new();
        let mut buf = vec![0u8; 16];
        let err = p
            .decrypt(
                RpfEncryption::Aes,
                CryptoTarget::Toc {
                    archive_name: None,
                    archive_size: 0,
                },
                &mut buf,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            crate::error::IoError::RpfEncryptionUnsupported { .. }
        ));
    }
}
