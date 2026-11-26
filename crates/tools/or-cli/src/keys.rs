use std::path::{Path, PathBuf};
use std::sync::Arc;

use or_io::awc::AwcKey;
use or_io::container::rpf::crypto::{extract_keys_with_snapshot, ExtractionMode, SnapshotSource};
use or_io::container::rpf::Rdr2CryptoProvider;
use or_io::container::{
    fingerprint_executable, ExtractionProgress, Gta5RetailCryptoProvider, KeysCache,
    NoOpCryptoProvider, RpfCryptoProvider,
};
use or_title::Game;

pub struct ResolvedCrypto {
    pub provider: Arc<dyn RpfCryptoProvider>,
    /// The XXTEA key an obfuscated `.awc` needs, zero when this key set has
    /// none. ⚠ Zero is a state rather than a failure: a cache an older build
    /// wrote in AES-only mode opens archives fine and cannot open a scrambled
    /// wave container.
    pub awc_key: AwcKey,
    pub notes: Vec<String>,
}

impl ResolvedCrypto {
    fn noop(note: impl Into<String>) -> Self {
        Self {
            provider: Arc::new(NoOpCryptoProvider::new()),
            awc_key: AwcKey::default(),
            notes: vec![note.into()],
        }
    }
}

pub fn shared_keys_cache_path_for(game: Game) -> Option<PathBuf> {
    let name = game.profile().keys_cache_name().unwrap_or("gta5_keys.bin");
    or_title::config::config_dir().map(|dir| dir.join(name))
}

/// The user's own key snapshot beside the keys cache, or else the bundled one.
pub fn user_snapshot() -> SnapshotSource {
    SnapshotSource::discover(or_title::config::config_dir().as_deref()).or_bundled()
}

fn find_game_exe(game_root: &Path) -> Option<PathBuf> {
    Game::detect_from_root(game_root).find_executable(game_root)
}

pub fn resolve_crypto(game_root: &Path, keys_override: Option<&Path>) -> ResolvedCrypto {
    if let Some(path) = keys_override {
        return match load_cache_provider(path) {
            Ok((provider, awc_key)) => ResolvedCrypto {
                provider,
                awc_key,
                notes: vec![format!("using keys from {}", path.display())],
            },
            Err(e) => ResolvedCrypto::noop(format!(
                "--keys {} could not be loaded ({e}); continuing without retail keys",
                path.display()
            )),
        };
    }

    let game = Game::detect_from_root(game_root);
    let exe = find_game_exe(game_root);
    if game == Game::Rdr2 {
        return resolve_rdr2(exe.as_deref());
    }
    let cache_path = shared_keys_cache_path_for(game);

    if let Some(cache_path) = cache_path.as_ref() {
        match KeysCache::read_from(cache_path) {
            Ok(Some(cache)) => {
                let fingerprint = exe
                    .as_deref()
                    .and_then(|exe| fingerprint_executable(exe).ok());
                let stale = fingerprint
                    .as_ref()
                    .is_some_and(|fp| fp.sha1_hex != cache.metadata.gta5_sha1_hex);
                if !stale {
                    // A cache an earlier build wrote without NG keys gains them by one
                    // re-extraction; without an executable to extract from, or if that
                    // fails, its AES keys still serve.
                    let upgrade = fingerprint.is_some() && user_snapshot().would_complete(&cache);
                    match cache.into_keys() {
                        Ok(keys) => {
                            let mut notes =
                                vec![format!("using cached keys ({})", cache_path.display())];
                            if let Some(exe) = exe.as_deref().filter(|_| upgrade) {
                                match try_extract_and_cache(exe, Some(cache_path)) {
                                    Ok(fresh) => return fresh,
                                    Err(e) => notes.push(format!(
                                        "re-extracting to add NG keys failed ({e}); using the \
                                         cached keys"
                                    )),
                                }
                            }
                            notes.extend(provenance_notes(&keys));
                            let awc_key = AwcKey(keys.awc_key);
                            let provider = Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys)));
                            return ResolvedCrypto {
                                provider,
                                awc_key,
                                notes,
                            };
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "cached keys decode failed");
                        }
                    }
                }
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(error = %e, "reading keys cache failed"),
        }
    }

    let Some(exe) = exe else {
        return ResolvedCrypto::noop(format!(
            "no {} found under {} and no keys cache; only OPEN/NONE archives will be searched",
            game.executable_name(),
            game_root.display()
        ));
    };

    extract_and_cache(&exe, cache_path.as_deref())
}

/// RDR2's cipher: the AES key from its executable unlocks a snapshot of the tables.
fn resolve_rdr2(exe: Option<&Path>) -> ResolvedCrypto {
    let Some(exe) = exe else {
        return ResolvedCrypto::noop("RDR2's archives need RDR2.exe beside them to be opened");
    };
    match Rdr2CryptoProvider::for_install(exe, or_title::config::config_dir().as_deref()) {
        Ok(provider) => ResolvedCrypto {
            provider: Arc::new(provider),
            awc_key: AwcKey::default(),
            notes: Vec::new(),
        },
        Err(e) => ResolvedCrypto::noop(format!(
            "RDR2 keys could not be built from {} ({e}); continuing without them",
            exe.display()
        )),
    }
}

/// Mount the install at `root` with every DLC, printing each key note to stderr.
pub(crate) fn mount_all(root: &Path, keys: Option<&Path>) -> Result<or_title::MountedGame, String> {
    let all = or_title::DlcCutoff::All;
    mount_at(root, keys, all, or_title::MountOptions::default()).map(|(game, _)| game)
}

/// Mount the install at `root` up to `cutoff` as `options` say, printing each key note to stderr: the mount, and the keys that opened it.
pub(crate) fn mount_at(
    root: &Path,
    keys: Option<&Path>,
    cutoff: or_title::DlcCutoff,
    options: or_title::MountOptions,
) -> Result<(or_title::MountedGame, Arc<dyn RpfCryptoProvider>), String> {
    let install = or_title::GtaInstall::from_path(root)
        .ok_or_else(|| format!("invalid install root: {}", root.display()))?;
    let resolved = resolve_crypto(root, keys);
    for note in &resolved.notes {
        eprintln!("note: {note}");
    }
    let provider = Arc::clone(&resolved.provider);
    or_title::build_mounted_game_with(&install, resolved.provider, cutoff, options)
        .map(|game| (game, provider))
        .map_err(|e| format!("mount failed: {e}"))
}

fn load_cache_provider(path: &Path) -> Result<(Arc<dyn RpfCryptoProvider>, AwcKey), String> {
    let cache = KeysCache::read_from(path)
        .map_err(|e| format!("read: {e}"))?
        .ok_or_else(|| "cache file not found".to_string())?;
    let keys = cache.into_keys().map_err(|e| format!("decode: {e}"))?;
    let awc_key = AwcKey(keys.awc_key);
    Ok((
        Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys))),
        awc_key,
    ))
}

fn extract_and_cache(exe: &Path, cache_path: Option<&Path>) -> ResolvedCrypto {
    try_extract_and_cache(exe, cache_path).unwrap_or_else(ResolvedCrypto::noop)
}

fn try_extract_and_cache(exe: &Path, cache_path: Option<&Path>) -> Result<ResolvedCrypto, String> {
    eprintln!(
        "openrage-cli: extracting GTA V keys from {} (first run only)...",
        exe.display()
    );
    let progress = |p: ExtractionProgress| {
        if p.total > 0 {
            eprintln!("  [{}] {}/{}", p.stage, p.found, p.total);
        }
    };

    let fingerprint = match fingerprint_executable(exe) {
        Ok(fp) => fp,
        Err(e) => {
            return Err(format!(
                "could not fingerprint {} ({e}); continuing without retail keys",
                exe.display()
            ));
        }
    };

    let snapshot = user_snapshot();
    let mut notes = Vec::new();
    let keys = match extract_keys_with_snapshot(exe, ExtractionMode::Full, &snapshot, &progress) {
        Ok(k) => k,
        Err(full_err) => {
            notes.push(format!(
                "full key extraction failed ({full_err}); retrying AES-only"
            ));
            match extract_keys_with_snapshot(exe, ExtractionMode::AesOnly, &snapshot, &progress) {
                Ok(k) => {
                    notes.push(
                        "extracted AES keys only; NG-encrypted archives will be skipped".into(),
                    );
                    k
                }
                Err(aes_err) => {
                    return Err(format!(
                        "key extraction failed (full: {full_err}; aes-only: {aes_err}); \
                         continuing without retail keys"
                    ));
                }
            }
        }
    };

    notes.extend(provenance_notes(&keys));

    let cache = KeysCache::build(exe.to_path_buf(), fingerprint, &keys);
    if let Some(path) = cache_path {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match cache.write_to(path) {
            Ok(()) => notes.push(format!("wrote keys cache to {}", path.display())),
            Err(e) => notes.push(format!("failed to persist keys cache: {e}")),
        }
    }

    let awc_key = AwcKey(keys.awc_key);
    Ok(ResolvedCrypto {
        provider: Arc::new(Gta5RetailCryptoProvider::new(Arc::new(keys))),
        awc_key,
        notes,
    })
}

/// One note per fact a user could not otherwise learn: that some of their
/// keys came from a snapshot, and whether it describes their build.
fn provenance_notes(keys: &or_io::container::Gta5Keys) -> Vec<String> {
    use or_io::container::rpf::crypto::SnapshotAgreement;

    let provenance = keys.provenance;
    let mut notes = Vec::new();
    if !keys.has_ng() {
        notes.push(or_io::container::rpf::crypto::WITHOUT_A_SNAPSHOT.to_string());
    }
    if provenance.ng_keys.is_snapshot() || provenance.ng_decrypt_tables.is_snapshot() {
        notes.push(format!(
            "NG keys came from {}, not from your executable — no search recovers them \
             from a shipped build",
            provenance.ng_keys.label()
        ));
    }
    match provenance.snapshot_agreement {
        SnapshotAgreement::Agrees => notes.push(
            "the snapshot's hash LUT matches your executable's, so it describes this build".into(),
        ),
        SnapshotAgreement::Disagrees => notes.push(
            "the snapshot's hash LUT does not match your executable's, so its NG keys were \
             refused; NG-encrypted archives will be skipped"
                .into(),
        ),
        SnapshotAgreement::Unchecked => {}
    }
    notes
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// ⚠ The variable is the process's, and a test that sets it is one the others must not overlap.
    static VARIABLE: Mutex<()> = Mutex::new(());

    #[test]
    fn the_keys_cache_and_the_snapshot_follow_openrage_config_dir() {
        let _held = VARIABLE.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("a folder");
        std::env::set_var("OPENRAGE_CONFIG_DIR", dir.path());

        let cache = shared_keys_cache_path_for(Game::GtaVLegacy);
        std::env::remove_var("OPENRAGE_CONFIG_DIR");

        let cache = cache.expect("a keys cache path");
        assert_eq!(
            cache.parent(),
            Some(dir.path()),
            "the CLI kept its keys cache in the real configuration directory"
        );
    }
}
